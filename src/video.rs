//! Native video playback, owned and polled on the application's UI thread.
use eframe::egui;

/// Fit inline media to both viewport dimensions without stretching or cropping.
pub fn inline_size(source: egui::Vec2, bounds: egui::Vec2) -> egui::Vec2 {
    let source = if source.x.is_finite() && source.y.is_finite() && source.x > 0.0 && source.y > 0.0 {
        source
    } else { egui::vec2(16.0, 9.0) };
    let bounds = bounds.max(egui::Vec2::splat(1.0));
    source * (bounds.x / source.x).min(bounds.y / source.y)
}

pub fn is_video_url(address: &str) -> bool {
    if crate::youtube::is_url(address) { return true; }
    url::Url::parse(address).ok().is_some_and(|url| {
        let path = std::path::Path::new(url.path());
        path.extension().is_some_and(|ext| {
            ["mp4", "mov", "m4v", "m3u8"].contains(&ext.to_string_lossy().to_lowercase().as_str())
        })
    })
}

/// A video file from Local or Remote always plays in video mode, through
/// the same player YouTube uses. The audio-only preference exists for
/// YouTube and Music streams, not for files from the user's own folders
/// and servers.
pub fn is_file_video(station: &crate::model::Station) -> bool {
    !crate::youtube::is_url(&station.stream_url)
        && (station.stream_url.starts_with("file:")
            || station.stream_url.starts_with("smb:")
            || station.stream_url.starts_with("sftp:")
            || station.media_source.as_deref() == Some("remoteFile"))
        && is_video_url(&station.stream_url)
}

/// Decode a single poster frame on an artwork worker, with bounded output and runtime.
pub fn thumbnail(address: &str, auth: Option<&(String, String)>) -> Result<Vec<u8>, String> {
    if address.starts_with("smb:") {
        let local = crate::smb::resolve(address, auth)?;
        return thumbnail(&local, None);
    }
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let url = url::Url::parse(address).map_err(|_| "Invalid video URL")?;
    let input = if url.scheme() == "file" {
        url.to_file_path()
            .map_err(|_| "Invalid video file")?
            .to_string_lossy()
            .into_owned()
    } else {
        address.to_owned()
    };
    let mut command = Command::new(crate::audio::executable("ffmpeg"));
    command.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
    if matches!(url.scheme(), "http" | "https") {
        command.args(["-rw_timeout", "5000000"]);
        if let Some((user, password)) = auth {
            use base64::Engine;
            let token =
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
            command.args(["-headers", &format!("Authorization: Basic {token}\r\n")]);
        }
    }
    command
        .args([
            "-i",
            &input,
            "-map",
            "0:V:0",
            "-frames:v",
            "1",
            "-an",
            "-vf",
            "scale=640:360:force_original_aspect_ratio=decrease",
            "-c:v",
            "png",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| "FFmpeg is required for video thumbnails")?;
    let output = child.stdout.take().ok_or("Thumbnail output unavailable")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        output
            .take(4 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "Thumbnail decoding failed")?
        .map_err(|_| "Thumbnail output could not be read")?;
    if success && !bytes.is_empty() && bytes.len() < 4 * 1024 * 1024 {
        Ok(bytes)
    } else {
        Err("No video thumbnail available".into())
    }
}

pub fn format_time(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.).floor() as u64
    } else {
        0
    };
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Time under a horizontal seek bar position, clamped to the track.
/// Used for the hover preview and for click/drag seeks.
pub fn seek_time_for_x(x: f32, left: f32, width: f32, duration: f64) -> f64 {
    if width <= 0.0 || !duration.is_finite() || duration <= 0.0 {
        return 0.0;
    }
    (((x as f64 - left as f64) / width as f64).clamp(0.0, 1.0)) * duration
}

#[derive(Default)]
pub struct ControlsFade {
    last_activity: Option<f64>,
}
impl ControlsFade {
    pub fn reset(&mut self) {
        self.last_activity = None;
    }
    pub fn opacity(&mut self, now: f64, interaction: bool) -> f32 {
        if interaction || self.last_activity.is_none() {
            self.last_activity = Some(now);
        }
        let idle = (now - self.last_activity.unwrap_or(now)).max(0.);
        ((3.25 - idle) / 0.25).clamp(0., 1.) as f32
    }
}

#[derive(Default)]
pub struct State {
    pub position: f64,
    pub duration: f64,
    pub seekable: bool,
    pub status: i32,
    pub stalled: bool,
    pub buffered: f64,
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use std::ffi::{CString, c_char, c_void};
    unsafe extern "C" {
        fn KRVideoCreate(
            url: *const c_char,
            user: *const c_char,
            password: *const c_char,
            offset: f64,
            volume: f32,
        ) -> *mut c_void;
        fn KRVideoDestroy(handle: *mut c_void);
        fn KRVideoState(
            handle: *mut c_void,
            position: *mut f64,
            duration: *mut f64,
            seekable: *mut bool,
        ) -> i32;
        fn KRVideoPause(handle: *mut c_void, paused: bool);
        fn KRVideoSeek(handle: *mut c_void, position: f64);
        fn KRVideoStalled(handle: *mut c_void) -> bool;
        fn KRVideoBuffered(handle: *mut c_void) -> f64;
        fn KRVideoRate(handle: *mut c_void, rate: f32);
        fn KRVideoVolume(handle: *mut c_void, volume: f32);
        fn KRVideoError(handle: *mut c_void, buffer: *mut c_char, capacity: usize);
        fn KRVideoFrame(
            handle: *mut c_void,
            rgba: *mut u8,
            capacity: usize,
            width: *mut u32,
            height: *mut u32,
        ) -> bool;
    }
    pub struct Player {
        handle: std::ptr::NonNull<c_void>,
        pixels: Vec<u8>,
        // AVFoundation calls and destruction must remain on the creating thread.
        _ui_thread: std::marker::PhantomData<std::rc::Rc<()>>,
    }
    impl Player {
        pub fn new(
            url: &str,
            offset: f64,
            volume: f32,
            auth: Option<(&str, &str)>,
        ) -> Result<Self, String> {
            let address = CString::new(url).map_err(|_| "Invalid video URL")?;
            let user = CString::new(auth.map_or("", |a| a.0)).map_err(|_| "Invalid username")?;
            let password =
                CString::new(auth.map_or("", |a| a.1)).map_err(|_| "Invalid password")?;
            // SAFETY: strings remain alive for this call; native code copies them.
            let handle = unsafe {
                KRVideoCreate(
                    address.as_ptr(),
                    user.as_ptr(),
                    password.as_ptr(),
                    offset,
                    volume,
                )
            };
            Ok(Self {
                handle: std::ptr::NonNull::new(handle)
                    .ok_or("Could not initialize video playback")?,
                pixels: vec![0; 1280 * 720 * 4],
                _ui_thread: std::marker::PhantomData,
            })
        }
        pub fn state(&self) -> State {
            let mut state = State::default();
            // SAFETY: the retained object is owned by self and all pointers are valid out parameters.
            state.status = unsafe {
                KRVideoState(
                    self.handle.as_ptr(),
                    &mut state.position,
                    &mut state.duration,
                    &mut state.seekable,
                )
            };
            if !state.position.is_finite() {
                state.position = 0.;
            }
            if !state.duration.is_finite() {
                state.duration = 0.;
            }
            // SAFETY: same owned handle as the state call above.
            state.stalled = unsafe { KRVideoStalled(self.handle.as_ptr()) };
            // SAFETY: same owned handle as the state call above.
            state.buffered = unsafe { KRVideoBuffered(self.handle.as_ptr()) };
            if !state.buffered.is_finite() {
                state.buffered = 0.;
            }
            state
        }
        pub fn frame(&mut self) -> Option<egui::ColorImage> {
            let (mut width, mut height) = (0, 0);
            // SAFETY: native code checks the buffer capacity before rendering.
            let fresh = unsafe {
                KRVideoFrame(
                    self.handle.as_ptr(),
                    self.pixels.as_mut_ptr(),
                    self.pixels.len(),
                    &mut width,
                    &mut height,
                )
            };
            fresh.then(|| {
                egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &self.pixels[..width as usize * height as usize * 4],
                )
            })
        }
        pub fn seek(&self, position: f64) {
            unsafe {
                KRVideoSeek(self.handle.as_ptr(), position);
            }
        }
        pub fn pause(&self, paused: bool) {
            unsafe {
                KRVideoPause(self.handle.as_ptr(), paused);
            }
        }
        pub fn volume(&self, volume: f32) {
            unsafe {
                KRVideoVolume(self.handle.as_ptr(), volume);
            }
        }
        pub fn rate(&self, rate: f32) {
            unsafe {
                KRVideoRate(self.handle.as_ptr(), rate.clamp(0.25, 2.0));
            }
        }
        pub fn error(&self) -> String {
            let mut buffer = [0u8; 1024];
            unsafe {
                KRVideoError(
                    self.handle.as_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                );
            }
            String::from_utf8_lossy(
                &buffer[..buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len())],
            )
            .into_owned()
        }
    }
    impl Drop for Player {
        fn drop(&mut self) {
            unsafe {
                KRVideoDestroy(self.handle.as_ptr());
            }
        }
    }
}
#[cfg(target_os = "macos")]
pub use native::Player;

#[cfg(not(target_os = "macos"))]
pub struct Player;
#[cfg(not(target_os = "macos"))]
impl Player {
    pub fn new(_: &str, _: f64, _: f32, _: Option<(&str, &str)>) -> Result<Self, String> {
        Err("Video playback currently requires macOS.".into())
    }
    pub fn state(&self) -> State {
        State::default()
    }
    pub fn frame(&mut self) -> Option<egui::ColorImage> {
        None
    }
    pub fn pause(&self, _: bool) {}
    pub fn seek(&self, _: f64) {}
    pub fn volume(&self, _: f32) {}
    pub fn rate(&self, _: f32) {}
    pub fn error(&self) -> String {
        "Video playback requires macOS.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_media_fits_wide_square_four_three_and_portrait_sources() {
        let bounds = egui::vec2(960.0, 560.0);
        for source in [egui::vec2(1920.0, 1080.0), egui::vec2(1440.0, 1080.0), egui::vec2(1000.0, 1000.0), egui::vec2(1080.0, 1920.0)] {
            let fitted = inline_size(source, bounds);
            assert!(fitted.x <= bounds.x + 0.01 && fitted.y <= bounds.y + 0.01);
            assert!((fitted.x / fitted.y - source.x / source.y).abs() < 0.001);
            let short = inline_size(source, egui::vec2(500.0, 180.0));
            assert!(short.y <= 180.01 && short.x <= 500.01);
        }
        assert!((inline_size(egui::vec2(4.0, 3.0), bounds).x - 746.6667).abs() < 0.01);
    }

    #[test]
    fn identifies_video_files_and_direct_streams_without_query_confusion() {
        for url in [
            "file:///Movies/Test.MOV",
            "https://example.com/movie.mp4?token=abc",
            "https://example.com/live/index.m3u8",
            "https://example.com/movie.m4v",
        ] {
            assert!(is_video_url(url), "{url}");
        }
        for url in [
            "https://example.com/song.mp3?video=movie.mp4",
            "file:///Music/song.m4a",
            "https://example.com/radio",
        ] {
            assert!(!is_video_url(url), "{url}");
        }
    }
}

#[cfg(test)]
mod controls_tests {
    use super::*;
    #[test]
    fn time_uses_minutes_and_zero_padded_seconds() {
        for (seconds, expected) in [
            (0., "0:00"),
            (9.9, "0:09"),
            (60., "1:00"),
            (3661., "61:01"),
            (-1., "0:00"),
            (f64::NAN, "0:00"),
        ] {
            assert_eq!(format_time(seconds), expected);
        }
    }
    #[test]
    fn seek_time_maps_bar_positions_and_clamps_outside() {
        assert_eq!(seek_time_for_x(50.0, 0.0, 100.0, 200.0), 100.0);
        assert_eq!(seek_time_for_x(-10.0, 0.0, 100.0, 200.0), 0.0);
        assert_eq!(seek_time_for_x(150.0, 0.0, 100.0, 200.0), 200.0);
        assert_eq!(seek_time_for_x(25.0, 10.0, 100.0, 80.0), 12.0);
        assert_eq!(seek_time_for_x(50.0, 0.0, 0.0, 200.0), 0.0);
        assert_eq!(seek_time_for_x(50.0, 0.0, 100.0, 0.0), 0.0);
    }
    #[test]
    fn file_videos_force_video_mode_but_streams_do_not() {
        let mut station = crate::model::Station::new("Movie", "file:///Movies/Movie.mp4");
        assert!(is_file_video(&station));
        station.stream_url = "smb://server/Share/Movie.mov".into();
        assert!(is_file_video(&station));
        station.stream_url = "https://example.com/Movie.mp4".into();
        station.media_source = Some("remoteFile".into());
        assert!(is_file_video(&station));
        station.stream_url = "https://example.com/Song.mp3".into();
        assert!(!is_file_video(&station));
        station.media_source = None;
        station.stream_url = "https://www.youtube.com/watch?v=abc".into();
        assert!(!is_file_video(&station));
        station.stream_url = "file:///Music/Song.mp3".into();
        assert!(!is_file_video(&station));
    }
    #[test]
    fn controls_fade_after_idle_and_reappear_on_interaction() {
        let mut controls = ControlsFade::default();
        assert_eq!(controls.opacity(10., false), 1.);
        assert_eq!(controls.opacity(12.99, false), 1.);
        assert!((controls.opacity(13.125, false) - 0.5).abs() < 0.01);
        assert_eq!(controls.opacity(13.3, false), 0.);
        assert_eq!(controls.opacity(20., true), 1.);
        assert_eq!(controls.opacity(24., true), 1.);
        assert_eq!(controls.opacity(28., false), 0.);
        controls.reset();
        assert_eq!(controls.opacity(30., false), 1.);
    }
    #[test]
    #[ignore = "requires FFmpeg; run cargo test video_thumbnail -- --ignored"]
    fn video_thumbnail_decodes_a_real_frame_and_metadata_uses_it() {
        let directory =
            std::env::temp_dir().join(format!("kaleido-thumbnail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("poster.mp4");
        let status = std::process::Command::new(crate::audio::executable("ffmpeg"))
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=10",
                "-t",
                "0.3",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let url = url::Url::from_file_path(&path).unwrap();
        let bytes = thumbnail(url.as_str(), None).unwrap();
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (640, 360));
        let metadata = crate::file_metadata::read(url.as_str(), None).unwrap();
        assert!(image::load_from_memory(&metadata.artwork.unwrap()).is_ok());
        assert!(
            thumbnail(
                &url::Url::from_file_path(directory.join("missing.mp4"))
                    .unwrap()
                    .to_string(),
                None
            )
            .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
