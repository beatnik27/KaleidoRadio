//! Physical-pixel procedural rendering for the Prism variations.
use eframe::{
    egui, egui_glow,
    glow::{self, HasContext},
};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(super) struct Scene {
    pub mode: f32,
    pub seed: f32,
    pub folds: f32,
    pub rotation: f32,
    pub hue: f32,
    pub opacity: f32,
}

pub(super) struct Renderer {
    program: glow::Program,
    resolution: Option<glow::UniformLocation>,
    origin: Option<glow::UniformLocation>,
    rotation: Option<glow::UniformLocation>,
    hue: Option<glow::UniformLocation>,
    scene: Option<glow::UniformLocation>,
    opacity: Option<glow::UniformLocation>,
}

impl Renderer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        // Called on the UI thread while eframe's OpenGL context is current.
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, source) in [
                (
                    glow::VERTEX_SHADER,
                    "#version 150 core\nvoid main() { vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)); gl_Position = vec4(p * 2. - 1., 0., 1.); }",
                ),
                (glow::FRAGMENT_SHADER, include_str!("prism.frag")),
            ] {
                let shader = match gl.create_shader(kind) {
                    Ok(shader) => shader,
                    Err(error) => {
                        for shader in shaders {
                            gl.delete_shader(shader);
                        }
                        gl.delete_program(program);
                        return Err(error);
                    }
                };
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    let error = gl.get_shader_info_log(shader);
                    gl.delete_shader(shader);
                    for shader in shaders {
                        gl.delete_shader(shader);
                    }
                    gl.delete_program(program);
                    return Err(error);
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
                let error = gl.get_program_info_log(program);
                gl.delete_program(program);
                return Err(error);
            }
            Ok(Self {
                program,
                resolution: gl.get_uniform_location(program, "resolution"),
                origin: gl.get_uniform_location(program, "origin"),
                rotation: gl.get_uniform_location(program, "rotation"),
                hue: gl.get_uniform_location(program, "hue"),
                scene: gl.get_uniform_location(program, "scene"),
                opacity: gl.get_uniform_location(program, "opacity"),
            })
        }
    }

    pub fn paint(self: &Arc<Self>, painter: &egui::Painter, rect: egui::Rect, scene: Scene) {
        let renderer = self.clone();
        painter.add(egui::PaintCallback {
            rect,
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                let gl = painter.gl();
                let viewport = info.viewport_in_pixels();
                if viewport.width_px <= 0 || viewport.height_px <= 0 {
                    return;
                }
                // eframe sets viewport/scissor and restores its own GL state after
                // the callback. VAOs are context-local, unlike the shared program:
                // create one here so detached viewports work on every context.
                unsafe {
                    let Ok(vao) = gl.create_vertex_array() else {
                        return;
                    };
                    gl.bind_vertex_array(Some(vao));
                    gl.use_program(Some(renderer.program));
                    gl.enable(glow::BLEND);
                    gl.blend_equation(glow::FUNC_ADD);
                    gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                    gl.uniform_2_f32(
                        renderer.resolution.as_ref(),
                        viewport.width_px as f32,
                        viewport.height_px as f32,
                    );
                    gl.uniform_2_f32(
                        renderer.origin.as_ref(),
                        viewport.left_px as f32,
                        viewport.from_bottom_px as f32,
                    );
                    gl.uniform_1_f32(renderer.rotation.as_ref(), scene.rotation);
                    gl.uniform_1_f32(renderer.hue.as_ref(), scene.hue);
                    gl.uniform_1_f32(renderer.opacity.as_ref(), scene.opacity);
                    gl.uniform_3_f32(renderer.scene.as_ref(), scene.mode, scene.seed, scene.folds);
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                    gl.bind_vertex_array(None);
                    gl.delete_vertex_array(vao);
                }
            })),
        });
    }

    pub fn destroy(&self, gl: &glow::Context) {
        // App::on_exit runs before eframe tears down the current context.
        unsafe {
            gl.delete_program(self.program);
        }
    }
}
