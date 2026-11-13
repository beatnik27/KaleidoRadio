//! MilkDrop presets via libprojectM, loaded at runtime like the Swift app.
//!
//! The `.milk` files in `presets/` are not interpreted here: projectM
//! parses the preset parameters and per-frame/per-pixel equations itself.
//! This module only:
//!   - dlopens `libprojectM-4.dylib` (bundled in the app, or next to the
//!     project for a plain `cargo run`),
//!   - renders projectM into its own framebuffer object at the callback
//!     rect's physical size, then blits that texture over the rect,
//!   - feeds synthetic PCM built from the Kaleido meter bands, exactly
//!     as the Swift app does (no audio tap is required),
//!   - loads presets from `presets/` (bundled, or in the project root).
//!
//! If the library is missing, the renderer reports an error and the
//! app keeps working with Prism/petals. Nothing here panics on a
//! missing library, a missing symbol, or a failed preset load.

use eframe::{
    egui, egui_glow,
    glow::{self, HasContext},
};
use std::{
    collections::HashSet,
    ffi::CString,
    os::raw::{c_char, c_void},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// Meter snapshot handed to the renderer each frame. Mirrors the
/// Swift app's `VisualizerMetrics`: bass/mid/treble bands, beat pulse,
/// and overall level. Mid is approximated from the overall energy,
/// since the Rust audio path only exposes bass and treble bands.
#[derive(Clone, Copy)]
pub struct Metrics {
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub beat: f32,
    pub level: f32,
}

type Handle = *mut c_void;
type CreateFn = unsafe extern "C" fn() -> Handle;
type DestroyFn = unsafe extern "C" fn(Handle);
type LoadPresetFileFn = unsafe extern "C" fn(Handle, *const c_char, bool);
type PcmAddFloatFn = unsafe extern "C" fn(Handle, *const f32, u32, i32);
type RenderFrameFn = unsafe extern "C" fn(Handle);
type RenderFrameFboFn = unsafe extern "C" fn(Handle, u32);
type SetFpsFn = unsafe extern "C" fn(Handle, i32);
type SetMeshSizeFn = unsafe extern "C" fn(Handle, usize, usize);
type SetBeatSensitivityFn = unsafe extern "C" fn(Handle, f32);
type SetPresetDurationFn = unsafe extern "C" fn(Handle, f64);
type SetSoftCutDurationFn = unsafe extern "C" fn(Handle, f64);
type SetWindowSizeFn = unsafe extern "C" fn(Handle, usize, usize);
type SetTextureSearchPathsFn = unsafe extern "C" fn(Handle, *const *const c_char, usize);

struct Fns {
    create: CreateFn,
    destroy: DestroyFn,
    load_preset_file: LoadPresetFileFn,
    pcm_add_float: PcmAddFloatFn,
    render_frame: RenderFrameFn,
    render_frame_fbo: Option<RenderFrameFboFn>,
    set_fps: SetFpsFn,
    set_mesh_size: SetMeshSizeFn,
    set_beat_sensitivity: SetBeatSensitivityFn,
    set_preset_duration: SetPresetDurationFn,
    set_soft_cut_duration: SetSoftCutDurationFn,
    set_window_size: SetWindowSizeFn,
    set_texture_search_paths: Option<SetTextureSearchPathsFn>,
}

unsafe impl Send for Inner {}

struct Inner {
    _lib: libloading::Library,
    fns: Fns,
    handle: Handle,
    presets: Vec<PathBuf>,
    names: Vec<String>,
    current: Option<usize>,
    pending: Option<usize>,
    phase: f32,
    fbo: Option<glow::Framebuffer>,
    texture: Option<glow::Texture>,
    size: (u32, u32),
    blit_program: Option<glow::Program>,
    blit_tex_loc: Option<glow::UniformLocation>,
    blit_origin_loc: Option<glow::UniformLocation>,
    blit_size_loc: Option<glow::UniformLocation>,
    logged_sizes: HashSet<(u32, u32)>,
    fbo_failed_for: Option<(u32, u32)>,
    blit_failed: bool,
}

pub struct Renderer {
    inner: Mutex<Inner>,
}

fn dylib_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("../Resources/libprojectM-4.dylib"));
            out.push(dir.join("libprojectM-4.dylib"));
        }
    }
    out.push(PathBuf::from("libprojectM-4.dylib"));
    out.push(PathBuf::from("/opt/homebrew/lib/libprojectM-4.dylib"));
    out.push(PathBuf::from("/usr/local/lib/libprojectM-4.dylib"));
    out
}

fn preset_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("../Resources/presets"));
            out.push(dir.join("../Resources/ProjectMPresets"));
            out.push(dir.join("presets"));
        }
    }
    out.push(PathBuf::from("presets"));
    out.push(PathBuf::from("Resources/presets"));
    out.push(PathBuf::from("Resources/ProjectMPresets"));
    out
}

fn discover_presets() -> Vec<PathBuf> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in preset_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_milk = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("milk"));
            if !is_milk {
                continue;
            }
            let key = path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if seen.insert(key) {
                out.push(path);
            }
        }
    }
    out.sort_by(|a, b| {
        a.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
            .cmp(&b.file_name().unwrap_or_default().to_string_lossy().to_lowercase())
    });
    out
}

fn preset_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn synthetic_pcm(metrics: Metrics, phase: &mut f32) -> Vec<f32> {
    let bass = metrics.bass.clamp(0.02, 1.);
    let mid = metrics.mid.clamp(0.02, 1.);
    let treble = metrics.treble.clamp(0.02, 1.);
    let beat = metrics.beat.clamp(0., 1.);
    let level = metrics.level.clamp(0.04, 1.);
    let mut samples = Vec::with_capacity(512 * 2);
    for index in 0..512 {
        let t = *phase + index as f32 / 512.;
        let left =
            (t * 36.).sin() * bass + (t * 97.).sin() * mid * 0.45 + (t * 181.).sin() * treble * 0.2;
        let right = (t * 31. + 0.7).sin() * bass
            + (t * 83. + 0.2).sin() * mid * 0.45
            + (t * 173.).sin() * treble * 0.2;
        let pulse = 1. + beat * 1.8;
        samples.push((left * level * pulse).clamp(-1., 1.));
        samples.push((right * level * pulse).clamp(-1., 1.));
    }
    *phase += 0.018 + level * 0.012;
    samples
}

impl Renderer {
    pub fn new() -> Result<Self, String> {
        let mut errors = Vec::new();
        let mut lib = None;
        for path in dylib_candidates() {
            match unsafe { libloading::Library::new(&path) } {
                Ok(l) => {
                    lib = Some(l);
                    break;
                }
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }
        let lib = lib.ok_or_else(|| {
            format!(
                "libprojectM-4.dylib not found. Tried:\n{}",
                errors.join("\n")
            )
        })?;
        unsafe {
            let get = |name: &[u8]| -> Result<libloading::Symbol<'_, CreateFn>, String> {
                lib.get(name)
                    .map_err(|_| format!("libprojectM is missing symbol {}", String::from_utf8_lossy(name)))
            };
            let fns = Fns {
                create: *get(b"projectm_create\0")?,
                destroy: *lib
                    .get::<DestroyFn>(b"projectm_destroy\0")
                    .map_err(|_| "libprojectM is missing symbol projectm_destroy".to_string())?,
                load_preset_file: *lib
                    .get::<LoadPresetFileFn>(b"projectm_load_preset_file\0")
                    .map_err(|_| "libprojectM is missing symbol projectm_load_preset_file".to_string())?,
                pcm_add_float: *lib
                    .get::<PcmAddFloatFn>(b"projectm_pcm_add_float\0")
                    .map_err(|_| "libprojectM is missing symbol projectm_pcm_add_float".to_string())?,
                render_frame: *lib
                    .get::<RenderFrameFn>(b"projectm_opengl_render_frame\0")
                    .map_err(|_| "libprojectM is missing symbol projectm_opengl_render_frame".to_string())?,
                render_frame_fbo: lib
                    .get::<RenderFrameFboFn>(b"projectm_opengl_render_frame_fbo\0")
                    .ok()
                    .map(|s| *s),
                set_fps: *lib
                    .get::<SetFpsFn>(b"projectm_set_fps\0")
                    .map_err(|_| "libprojectM is missing symbol projectm_set_fps".to_string())?,
                set_mesh_size: *lib
                    .get::<SetMeshSizeFn>(b"projectm_set_mesh_size\0")
                    .map_err(|_| "libprojectM is missing symbol projectm_set_mesh_size".to_string())?,
                set_beat_sensitivity: *lib
                    .get::<SetBeatSensitivityFn>(b"projectm_set_beat_sensitivity\0")
                    .map_err(|_| "libprojectM is missing symbol projectm_set_beat_sensitivity".to_string())?,
                set_preset_duration: *lib
                    .get::<SetPresetDurationFn>(b"projectm_set_preset_duration\0")
                    .map_err(|_| "libprojectM is missing symbol projectm_set_preset_duration".to_string())?,
                set_soft_cut_duration: *lib
                    .get::<SetSoftCutDurationFn>(b"projectm_set_soft_cut_duration\0")
                    .map_err(|_| "libprojectM is missing symbol projectm_set_soft_cut_duration".to_string())?,
                set_window_size: *lib
                    .get::<SetWindowSizeFn>(b"projectm_set_window_size\0")
                    .map_err(|_| "libprojectM is missing symbol projectm_set_window_size".to_string())?,
                set_texture_search_paths: lib
                    .get::<SetTextureSearchPathsFn>(b"projectm_set_texture_search_paths\0")
                    .ok()
                    .map(|s| *s),
            };
            let handle = (fns.create)();
            if handle.is_null() {
                return Err("projectm_create() returned null (no current OpenGL context?)".to_string());
            }
            (fns.set_fps)(handle, 60);
            (fns.set_mesh_size)(handle, 48, 36);
            (fns.set_beat_sensitivity)(handle, 1.35);
            (fns.set_preset_duration)(handle, 28.0);
            (fns.set_soft_cut_duration)(handle, 3.0);
            let presets = discover_presets();
            let names = presets.iter().map(|p| preset_name(p)).collect();
            if let Some(set_paths) = fns.set_texture_search_paths {
                let dirs: Vec<CString> = preset_dirs()
                    .into_iter()
                    .filter(|d| d.is_dir())
                    .filter_map(|d| CString::new(d.to_string_lossy().into_owned()).ok())
                    .collect();
                let ptrs: Vec<*const c_char> = dirs.iter().map(|d| d.as_ptr()).collect();
                if !ptrs.is_empty() {
                    set_paths(handle, ptrs.as_ptr(), ptrs.len());
                }
            }
            Ok(Self {
                inner: Mutex::new(Inner {
                    _lib: lib,
                    fns,
                    handle,
                    presets,
                    names,
                    current: None,
                    pending: None,
                    phase: 0.,
                    fbo: None,
                    texture: None,
                    size: (0, 0),
                    blit_program: None,
                    blit_tex_loc: None,
                    blit_origin_loc: None,
                    blit_size_loc: None,
                    logged_sizes: HashSet::new(),
                    fbo_failed_for: None,
                    blit_failed: false,
                }),
            })
        }
    }

    pub fn preset_names(&self) -> Vec<String> {
        self.inner.lock().map(|i| i.names.clone()).unwrap_or_default()
    }

    pub fn preset_count(&self) -> usize {
        self.inner.lock().map(|i| i.presets.len()).unwrap_or(0)
    }

    pub fn set_preset(&self, index: usize) {
        if let Ok(mut inner) = self.inner.lock() {
            if index < inner.presets.len() {
                inner.pending = Some(index);
            }
        }
    }

    pub fn current_preset(&self) -> Option<usize> {
        self.inner.lock().ok().and_then(|i| i.current)
    }

    pub fn paint(self: &Arc<Self>, painter: &egui::Painter, rect: egui::Rect, metrics: Metrics) {
        let renderer = self.clone();
        painter.add(egui::PaintCallback {
            rect,
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                let gl = painter.gl();
                let viewport = info.viewport_in_pixels();
                if viewport.width_px <= 0 || viewport.height_px <= 0 {
                    return;
                }
                let Ok(mut inner) = renderer.inner.lock() else {
                    return;
                };
                if inner.handle.is_null() {
                    return;
                }
                unsafe {
                    if let Some(index) = inner.pending.take() {
                        if let Some(path) = inner.presets.get(index) {
                            if let Ok(cpath) = CString::new(path.to_string_lossy().into_owned()) {
                                (inner.fns.load_preset_file)(inner.handle, cpath.as_ptr(), true);
                                inner.current = Some(index);
                            }
                        }
                    }
                    if inner.current.is_none() && !inner.presets.is_empty() {
                        let path = inner.presets[0].clone();
                        if let Ok(cpath) = CString::new(path.to_string_lossy().into_owned()) {
                            (inner.fns.load_preset_file)(inner.handle, cpath.as_ptr(), true);
                            inner.current = Some(0);
                        }
                    }
                    let samples = synthetic_pcm(metrics, &mut inner.phase);
                    (inner.fns.pcm_add_float)(inner.handle, samples.as_ptr(), 512, 2);
                    let width = viewport.width_px as u32;
                    let height = viewport.height_px as u32;
                    if inner.logged_sizes.insert((width, height)) {
                        eprintln!("[projectM] paint {}x{} px, clip {:?} points, screen {:?} px", width, height, info.clip_rect, info.screen_size_px);
                    }
                    (inner.fns.set_window_size)(inner.handle, width as usize, height as usize);
                    if let Some(render_fbo) = inner.fns.render_frame_fbo {
                        let mut rendered = false;
                        if ensure_fbo(gl, &mut inner, width, height) {
                            let fbo = inner.fbo.unwrap();
                            // egui's scissor is in window coordinates. It can
                            // exclude the entire offscreen target when the
                            // visualizer is embedded in the player.
                            gl.disable(glow::SCISSOR_TEST);
                            render_fbo(inner.handle, fbo.0.get());
                            rendered = true;
                        } else if inner.fbo_failed_for != Some((width, height)) {
                            inner.fbo_failed_for = Some((width, height));
                            eprintln!("[projectM] FBO creation failed for {}x{}, falling back to direct render", width, height);
                        }
                        gl.bind_framebuffer(glow::FRAMEBUFFER, painter.intermediate_fbo());
                        gl.viewport(viewport.left_px, viewport.from_bottom_px, viewport.width_px, viewport.height_px);
                        // Scissor to egui's clip rect, not the full callback
                        // rect: the inline vis is nested inside the media
                        // card, so only part of the callback rect may be
                        // visible, and drawing outside the clip is wrong.
                        let clip = info.clip_rect_in_pixels();
                        gl.enable(glow::SCISSOR_TEST);
                        gl.scissor(clip.left_px, clip.from_bottom_px, clip.width_px, clip.height_px);
                        if rendered && ensure_blit(gl, &mut inner) {
                            let program = inner.blit_program.unwrap();
                            let texture = inner.texture.unwrap();
                            let Ok(vao) = gl.create_vertex_array() else {
                                return;
                            };
                            gl.bind_vertex_array(Some(vao));
                            gl.use_program(Some(program));
                            gl.disable(glow::BLEND);
                            gl.disable(glow::DEPTH_TEST);
                            gl.active_texture(glow::TEXTURE0);
                            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                            gl.uniform_1_i32(inner.blit_tex_loc.as_ref(), 0);
                            gl.uniform_2_f32(inner.blit_origin_loc.as_ref(), viewport.left_px as f32, viewport.from_bottom_px as f32);
                            gl.uniform_2_f32(inner.blit_size_loc.as_ref(), viewport.width_px as f32, viewport.height_px as f32);
                            gl.draw_arrays(glow::TRIANGLES, 0, 3);
                            gl.bind_texture(glow::TEXTURE_2D, None);
                            gl.bind_vertex_array(None);
                            gl.delete_vertex_array(vao);
                        } else {
                            if !inner.blit_failed {
                                inner.blit_failed = true;
                                eprintln!("[projectM] blit unavailable, falling back to direct render");
                            }
                            (inner.fns.render_frame)(inner.handle);
                        }
                    } else {
                        (inner.fns.render_frame)(inner.handle);
                    }
                }
            })),
        });
    }

    pub fn destroy(&self, gl: &glow::Context) {
        if let Ok(mut inner) = self.inner.lock() {
            unsafe {
                if let Some(fbo) = inner.fbo.take() {
                    gl.delete_framebuffer(fbo);
                }
                if let Some(texture) = inner.texture.take() {
                    gl.delete_texture(texture);
                }
                if let Some(program) = inner.blit_program.take() {
                    gl.delete_program(program);
                }
                if !inner.handle.is_null() {
                    (inner.fns.destroy)(inner.handle);
                    inner.handle = std::ptr::null_mut();
                }
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                (self.fns.destroy)(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

fn ensure_fbo(gl: &glow::Context, inner: &mut Inner, width: u32, height: u32) -> bool {
    if inner.fbo.is_some() && inner.size == (width, height) {
        return true;
    }
    unsafe {
        if let Some(fbo) = inner.fbo.take() {
            gl.delete_framebuffer(fbo);
        }
        if let Some(texture) = inner.texture.take() {
            gl.delete_texture(texture);
        }
        // The blit program was created in the same GL context as the
        // FBO. The main window and the visualizer window can use
        // different contexts, so a program cached from one context
        // must not be reused in another: drop it with the FBO and
        // recreate it in the context that paints next.
        if let Some(program) = inner.blit_program.take() {
            gl.delete_program(program);
        }
        inner.blit_tex_loc = None;
        inner.blit_origin_loc = None;
        inner.blit_size_loc = None;
        inner.blit_failed = false;
        inner.fbo_failed_for = None;
        let Ok(texture) = gl.create_texture() else {
            return false;
        };
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, width as i32, height as i32, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
        gl.bind_texture(glow::TEXTURE_2D, None);
        let Ok(fbo) = gl.create_framebuffer() else {
            gl.delete_texture(texture);
            return false;
        };
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(texture), 0);
        let complete = gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        if !complete {
            gl.delete_framebuffer(fbo);
            gl.delete_texture(texture);
            return false;
        }
        inner.fbo = Some(fbo);
        inner.texture = Some(texture);
        inner.size = (width, height);
        true
    }
}

fn ensure_blit(gl: &glow::Context, inner: &mut Inner) -> bool {
    if inner.blit_program.is_some() {
        return true;
    }
    unsafe {
        let Ok(program) = gl.create_program() else {
            return false;
        };
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, "#version 150 core\nvoid main() { vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)); gl_Position = vec4(p * 2. - 1., 0., 1.); }"),
            (glow::FRAGMENT_SHADER, "#version 150 core\nuniform sampler2D u_tex;\nuniform vec2 u_origin;\nuniform vec2 u_size;\nout vec4 out_color;\nvoid main() { vec2 uv = (gl_FragCoord.xy - u_origin) / u_size; out_color = texture(u_tex, uv); }"),
        ] {
            let Ok(shader) = gl.create_shader(kind) else {
                gl.delete_program(program);
                return false;
            };
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                gl.delete_shader(shader);
                for s in shaders {
                    gl.delete_shader(s);
                }
                gl.delete_program(program);
                return false;
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            gl.delete_program(program);
            return false;
        }
        inner.blit_tex_loc = gl.get_uniform_location(program, "u_tex");
        inner.blit_origin_loc = gl.get_uniform_location(program, "u_origin");
        inner.blit_size_loc = gl.get_uniform_location(program, "u_size");
        inner.blit_program = Some(program);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_pcm_stays_in_range_and_is_stereo() {
        let mut phase = 0.;
        let samples = synthetic_pcm(Metrics { bass: 1., mid: 1., treble: 1., beat: 1., level: 1. }, &mut phase);
        assert_eq!(samples.len(), 1024);
        assert!(samples.iter().all(|s| (-1. ..=1.).contains(s)));
        assert!(phase > 0.);
    }

    #[test]
    fn preset_name_uses_file_stem() {
        assert_eq!(preset_name(Path::new("/x/Geiss - Foo.milk")), "Geiss - Foo");
    }
}
