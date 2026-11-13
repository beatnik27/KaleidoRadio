//! Local HLS relay: yt-dlp owns remote requests, FFmpeg muxes incoming A/V,
//! and AVFoundation consumes completed segments before the download finishes.
use super::{FrameRate, Quality, error_message, is_url};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub struct Session {
    pub address: String,
    pub start_time: f64,
    pub duration: Option<f64>,
    pub artist: Option<String>,
    buffered_seconds: Arc<AtomicU64>,
    cancelled: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    #[cfg(test)]
    cache: std::path::PathBuf,
}
impl Session {
    pub fn can_seek(&self, position: f64) -> bool {
        buffered_contains(
            self.start_time,
            f64::from_bits(self.buffered_seconds.load(Ordering::Relaxed)),
            position,
        )
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok()?.clone()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

// Every subprocess (including yt-dlp's FFmpeg child) belongs to an owned group.
// Cleanup runs on a worker, never waits for external processes on the UI thread.
pub(super) struct Process(pub(super) Child);
impl Process {
    pub(super) fn terminate(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            #[cfg(unix)]
            unsafe {
                // SAFETY: a still-owned live child was spawned in a new process
                // group with its PID as group ID; no other session is targeted.
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.terminate();
    }
}
pub(super) fn spawn(command: &mut Command) -> Result<Process, String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .map(Process)
        .map_err(|e| format!("Could not start YouTube streaming: {e}"))
}

fn logs(directory: &Path) -> String {
    let mut result = String::new();
    for name in ["youtube.log", "mux.log"] {
        if let Ok(file) = File::open(directory.join(name)) {
            let _ = file.take(64 * 1024).read_to_string(&mut result);
        }
    }
    // Keep the actual downloader/muxer failure, rather than truncating the
    // initial extraction progress before the error text.
    let tail = result
        .chars()
        .rev()
        .take(3000)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    error_message(&tail)
}
fn cache_size(directory: &Path) -> u64 {
    fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}
fn failed(process: &mut Process) -> bool {
    matches!(process.0.try_wait(), Ok(Some(status)) if !status.success())
}

pub fn start(
    address: &str,
    quality: Quality,
    frame_rate: FrameRate,
    offset: f64,
    cancelled: Arc<AtomicBool>,
) -> Result<Session, String> {
    start_mode(address, quality, frame_rate, offset, cancelled, false)
}
pub fn start_audio(
    address: &str,
    quality: Quality,
    frame_rate: FrameRate,
    offset: f64,
    cancelled: Arc<AtomicBool>,
) -> Result<Session, String> {
    start_mode(address, quality, frame_rate, offset, cancelled, true)
}
fn start_mode(
    address: &str,
    quality: Quality,
    frame_rate: FrameRate,
    offset: f64,
    cancelled: Arc<AtomicBool>,
    audio_only: bool,
) -> Result<Session, String> {
    if cancelled.load(Ordering::Relaxed) {
        return Err("YouTube playback cancelled.".into());
    }
    if !is_url(address) {
        return Err("Enter a YouTube video URL.".into());
    }
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let token = uuid::Uuid::new_v4().to_string();
    let local_address = format!(
        "http://{}/{token}/stream.m3u8",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    // A short pre-roll lets stream-copy output reach a clean keyframe before
    // the requested position, without downloading the earlier part of the video.
    let start_time = (offset.max(0.0) - 8.0).max(0.0);
    let local_offset = offset.max(0.0) - start_time;
    let duration_file = directory.path().join("duration.json");
    let artist_file = directory.path().join("artist.json");
    let ffmpeg = crate::audio::executable("ffmpeg");
    let deno = crate::audio::executable("deno");
    let mut command = Command::new(crate::audio::executable("yt-dlp"));
    command
        .args([
            "--ignore-config",
            "--no-playlist",
            "--no-warnings",
            "--no-progress",
            "--socket-timeout",
            "15",
            "--retries",
            "1",
            "--js-runtimes",
            &format!("deno:{deno}"),
            "--ffmpeg-location",
            &ffmpeg,
            "--downloader",
            "ffmpeg",
            "--downloader-args",
            "ffmpeg:-loglevel error -nostats",
            "--format",
            &if audio_only {
                "bestaudio[ext=m4a]".to_owned()
            } else {
                quality.format(frame_rate)
            },
            "--output",
            "-",
            "--print-to-file",
            "before_dl:%(duration)j",
        ])
        .arg(&duration_file)
        .args(["--print-to-file", "before_dl:%(artists,artist,creator)j"])
        .arg(&artist_file);
    if audio_only {
        // A single M4A format would otherwise use a non-streamable MP4 muxer
        // on stdout. MPEG-TS can flow incrementally into the HLS remuxer.
        command.args(["--downloader-args", "ffmpeg_o:-f mpegts"]);
    }
    if start_time > 0.0 {
        command.args(["--download-sections", &format!("*{start_time:.3}-inf")]);
    }
    command
        .arg("--")
        .arg(address)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(File::create(directory.path().join("youtube.log")).map_err(|e| e.to_string())?);
    let mut download = spawn(&mut command)?;
    let pipe = download
        .0
        .stdout
        .take()
        .ok_or("YouTube stream output unavailable")?;
    let mut command = Command::new(&ffmpeg);
    command.current_dir(directory.path()).args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        "pipe:0",
    ]);
    if !audio_only {
        command.args(["-map", "0:v:0"]);
    }
    command
        .args([
            "-map",
            "0:a:0",
            "-c",
            "copy",
            "-f",
            "hls",
            "-hls_time",
            "2",
            "-hls_list_size",
            "0",
            "-hls_playlist_type",
            "event",
            "-hls_flags",
            "independent_segments+temp_file",
            "stream.m3u8",
        ])
        .stdin(Stdio::from(pipe))
        .stdout(Stdio::null())
        .stderr(File::create(directory.path().join("mux.log")).map_err(|e| e.to_string())?);
    let mut mux = spawn(&mut command)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err("YouTube playback cancelled.".into());
        }
        if failed(&mut download) || failed(&mut mux) {
            return Err(logs(directory.path()));
        }
        if cache_size(directory.path()) > 512 * 1024 * 1024 {
            return Err("YouTube stream cache exceeded 512 MB.".into());
        }
        let playlist = fs::read_to_string(directory.path().join("stream.m3u8")).unwrap_or_default();
        if playlist_ready(&playlist, local_offset) {
            break;
        }
        if Instant::now() >= deadline {
            return Err("YouTube streaming timed out before playback started.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let duration = fs::read_to_string(&duration_file)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text.trim()).ok())
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite() && *value > 0.0);
    let artist = fs::read_to_string(&artist_file)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text.trim()).ok())
        .and_then(|value| super::artist_value(&value));
    let buffered_seconds = Arc::new(AtomicU64::new(
        playlist_duration(
            &fs::read_to_string(directory.path().join("stream.m3u8")).unwrap_or_default(),
        )
        .to_bits(),
    ));
    let worker_buffered = buffered_seconds.clone();
    let error = Arc::new(Mutex::new(None));
    let worker_error = error.clone();
    let worker_cancel = cancelled.clone();
    #[cfg(test)]
    let cache = directory.path().to_path_buf();
    std::thread::spawn(move || {
        while !worker_cancel.load(Ordering::Relaxed) {
            if let Ok(playlist) = fs::read_to_string(directory.path().join("stream.m3u8")) {
                worker_buffered.store(playlist_duration(&playlist).to_bits(), Ordering::Relaxed);
            }
            let failure = if failed(&mut download) || failed(&mut mux) {
                Some(logs(directory.path()))
            } else if cache_size(directory.path()) > 512 * 1024 * 1024 {
                Some("YouTube stream cache exceeded 512 MB.".into())
            } else {
                None
            };
            if let Some(failure) = failure {
                if let Ok(mut error) = worker_error.lock() {
                    *error = Some(failure);
                }
                break;
            }
            match listener.accept() {
                Ok((connection, _)) => {
                    let _ = serve(connection, directory.path(), &token);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(_) => break,
            }
        }
        drop(mux);
        drop(download);
        drop(listener);
        drop(directory);
    });
    Ok(Session {
        address: local_address,
        start_time,
        duration,
        artist,
        buffered_seconds,
        cancelled,
        error,
        #[cfg(test)]
        cache,
    })
}

fn buffered_contains(start: f64, length: f64, position: f64) -> bool {
    position.is_finite() && position >= start && position < start + length - 0.25
}
fn playlist_duration(playlist: &str) -> f64 {
    playlist
        .lines()
        .filter_map(|line| {
            line.strip_prefix("#EXTINF:")?
                .split(',')
                .next()?
                .parse::<f64>()
                .ok()
        })
        .sum()
}

fn playlist_ready(playlist: &str, offset: f64) -> bool {
    let durations: Vec<f64> = playlist
        .lines()
        .filter_map(|line| {
            line.strip_prefix("#EXTINF:")?
                .split(',')
                .next()?
                .parse()
                .ok()
        })
        .collect();
    !durations.is_empty()
        && (playlist.contains("#EXT-X-ENDLIST")
            || (durations.len() >= 2 && durations.iter().sum::<f64>() >= offset.max(0.0) + 4.0))
}

fn requested_file<'a>(path: &'a str, token: &str) -> Option<&'a str> {
    let path = path.strip_prefix('/')?;
    let (prefix, file) = path.split_once('/')?;
    if prefix != token
        || file.is_empty()
        || !file
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    {
        return None;
    }
    if file == "stream.m3u8" || (file.starts_with("stream") && file.ends_with(".ts")) {
        Some(file)
    } else {
        None
    }
}
fn serve(mut connection: TcpStream, directory: &Path, token: &str) -> std::io::Result<()> {
    // Accepted sockets can inherit the listener's nonblocking mode on macOS.
    connection.set_nonblocking(false)?;
    connection.set_read_timeout(Some(Duration::from_secs(2)))?;
    connection.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut header = Vec::new();
    let mut bytes = [0; 1024];
    while header.len() < 8192 {
        let count = connection.read(&mut bytes)?;
        if count == 0 {
            return Ok(());
        }
        header.extend_from_slice(&bytes[..count]);
        if header.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8_lossy(&header);
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let file = requested_file(path, token).filter(|_| matches!(method, "GET" | "HEAD"));
    let Some(name) = file else {
        connection.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
        return Ok(());
    };
    let Ok(mut file) = File::open(directory.join(name)) else {
        connection.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
        return Ok(());
    };
    let length = file.metadata()?.len();
    let range = request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if !key.eq_ignore_ascii_case("range") {
            return None;
        }
        let (start, end) = value.trim().strip_prefix("bytes=")?.split_once('-')?;
        let start = start.parse::<u64>().ok()?;
        let end = if end.is_empty() {
            length.saturating_sub(1)
        } else {
            end.parse::<u64>().ok()?
        };
        Some((start, end))
    });
    let (start, end) = range.unwrap_or((0, length.saturating_sub(1)));
    if range.is_some() && (start > end || start >= length) {
        write!(
            connection,
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{length}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )?;
        return Ok(());
    }
    let end = end.min(length.saturating_sub(1));
    let count = if length == 0 { 0 } else { end - start + 1 };
    let mime = if name.ends_with(".m3u8") {
        "application/vnd.apple.mpegurl"
    } else {
        "video/mp2t"
    };
    write!(
        connection,
        "HTTP/1.1 {}\r\nContent-Type: {mime}\r\nContent-Length: {count}\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n",
        if range.is_some() {
            "206 Partial Content"
        } else {
            "200 OK"
        }
    )?;
    if range.is_some() {
        write!(
            connection,
            "Content-Range: bytes {start}-{end}/{length}\r\n"
        )?;
    }
    connection.write_all(b"\r\n")?;
    if method == "GET" {
        file.seek(SeekFrom::Start(start))?;
        std::io::copy(&mut file.take(count), &mut connection)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seek_routing_uses_original_video_timestamps() {
        assert!(buffered_contains(292.0, 14.0, 300.0));
        assert!(!buffered_contains(292.0, 14.0, 100.0));
        assert!(!buffered_contains(292.0, 14.0, 400.0));
        assert!(!buffered_contains(292.0, 14.0, f64::NAN));
        assert!(!buffered_contains(292.0, 14.0, 306.0));
    }
    #[test]
    #[ignore = "requires yt-dlp, FFmpeg, internet access, and Xcode command line tools"]
    #[cfg(target_os = "macos")]
    fn live_unbuffered_forward_and_backward_seek() {
        let directory = tempfile::tempdir().unwrap();
        let probe = directory.path().join("video-probe");
        let mut compiler = Command::new("xcrun");
        compiler.args([
            "clang",
            "-w",
            "-fobjc-arc",
            "-fblocks",
            "tests/native_video.m",
        ]);
        for framework in [
            "Foundation",
            "AVFoundation",
            "CoreVideo",
            "CoreImage",
            "CoreMedia",
            "QuartzCore",
            "CoreGraphics",
        ] {
            compiler.args(["-framework", framework]);
        }
        assert!(compiler.arg("-o").arg(&probe).status().unwrap().success());
        for target in [300.0_f64, 20.0] {
            let started = Instant::now();
            let stream = start(
                "https://www.youtube.com/watch?v=Kl12yfoHaLg",
                Quality::default(),
                FrameRate::default(),
                target,
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            assert_eq!(stream.start_time, target - 8.0);
            assert!(
                stream
                    .duration
                    .is_some_and(|duration| duration > 430.0 && duration < 432.0)
            );
            assert!(stream.can_seek(target));
            assert!(!stream.can_seek(0.0));
            assert!(!stream.can_seek(400.0));
            let playlist = reqwest::blocking::get(&stream.address)
                .unwrap()
                .text()
                .unwrap();
            assert!(!playlist.contains("#EXT-X-ENDLIST"));
            assert!(
                playlist_duration(&playlist) < 60.0,
                "Must fetch near the target rather than the earlier prefix"
            );
            let output = Command::new(&probe)
                .args([&stream.address, "--stream-offset", "8.0"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            eprintln!(
                "Seek to {target}s played after {:.1}s, full duration {:.1}s",
                started.elapsed().as_secs_f64(),
                stream.duration.unwrap()
            );
            drop(stream);
        }
    }
    #[test]
    fn resume_waits_until_requested_position_is_buffered() {
        let playlist = "#EXTM3U\n#EXTINF:7.0,\nstream0.ts\n#EXTINF:7.0,\nstream1.ts\n";
        assert!(playlist_ready(playlist, 0.0));
        assert!(!playlist_ready(playlist, 20.0));
        assert!(playlist_ready(&format!("{playlist}#EXT-X-ENDLIST\n"), 12.0));
        assert!(!playlist_ready("#EXTM3U\n", 0.0));
    }
    #[test]
    fn cancellation_prevents_starting_external_processes() {
        let cancelled = Arc::new(AtomicBool::new(true));
        assert!(
            start(
                "https://www.youtube.com/watch?v=unused",
                Quality::default(),
                FrameRate::default(),
                0.0,
                cancelled
            )
            .err()
            .unwrap()
            .contains("cancelled")
        );
    }
    #[test]
    #[ignore = "requires yt-dlp, FFmpeg, internet access, and Xcode command line tools"]
    #[cfg(target_os = "macos")]
    fn live_stream_starts_before_download_finishes_and_cleans_up() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        let stream = start(
            "https://www.youtube.com/watch?v=Kl12yfoHaLg",
            Quality::default(),
            FrameRate::default(),
            0.0,
            cancelled.clone(),
        )
        .unwrap();
        let cache = stream.cache.clone();
        let playlist = reqwest::blocking::get(&stream.address)
            .unwrap()
            .text()
            .unwrap();
        assert!(
            !playlist.contains("#EXT-X-ENDLIST"),
            "Video should still be arriving when playback starts"
        );
        eprintln!(
            "Streaming started in {:.1}s before the complete download",
            started.elapsed().as_secs_f64()
        );
        let output = Command::new(crate::audio::executable("ffprobe"))
            .args([
                "-v",
                "error",
                "-rw_timeout",
                "5000000",
                "-read_intervals",
                "%+1",
                "-show_entries",
                "stream=codec_type,height",
                "-of",
                "json",
                &stream.address,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}; stream error: {:?}",
            String::from_utf8_lossy(&output.stderr),
            stream.error()
        );
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let streams = metadata["streams"].as_array().unwrap();
        assert!(streams.iter().any(|s| s["codec_type"] == "audio"));
        assert!(
            streams
                .iter()
                .any(|s| s["codec_type"] == "video" && s["height"] == 720)
        );
        let directory = tempfile::tempdir().unwrap();
        let probe = directory.path().join("video-probe");
        let mut compiler = Command::new("xcrun");
        compiler.args([
            "clang",
            "-w",
            "-fobjc-arc",
            "-fblocks",
            "tests/native_video.m",
        ]);
        for framework in [
            "Foundation",
            "AVFoundation",
            "CoreVideo",
            "CoreImage",
            "CoreMedia",
            "QuartzCore",
            "CoreGraphics",
        ] {
            compiler.args(["-framework", framework]);
        }
        assert!(compiler.arg("-o").arg(&probe).status().unwrap().success());
        let output = Command::new(probe)
            .args([&stream.address, "--stream-preview"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}; stream error: {:?}",
            String::from_utf8_lossy(&output.stderr),
            stream.error()
        );
        assert!(stream.error().is_none());
        drop(stream);
        assert!(cancelled.load(Ordering::Relaxed));
        let deadline = Instant::now() + Duration::from_secs(5);
        while cache.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !cache.exists(),
            "Streaming processes and temporary segments must be cleaned up"
        );
    }
    #[test]
    fn relay_paths_require_session_token_and_media_files() {
        assert_eq!(
            requested_file("/secret/stream.m3u8", "secret"),
            Some("stream.m3u8")
        );
        for path in [
            "/other/stream.m3u8",
            "/secret/../youtube.log",
            "/secret/stream%2f..ts",
            "/secret/youtube.log",
        ] {
            assert!(requested_file(path, "secret").is_none());
        }
    }
    #[test]
    fn relay_waits_for_headers_on_nonblocking_accepted_sockets() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("stream.m3u8"), b"#EXTM3U\n").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut connection = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        server.set_nonblocking(true).unwrap();
        let client = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            connection
                .write_all(b"GET /token/stream.m3u8 HTTP/1.1\r\n\r\n")
                .unwrap();
            let mut response = String::new();
            connection.read_to_string(&mut response).unwrap();
            response
        });
        serve(server, directory.path(), "token").unwrap();
        assert!(client.join().unwrap().ends_with("#EXTM3U\n"));
    }
    #[test]
    fn relay_serves_byte_ranges_and_head_requests() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("stream0.ts"), b"0123456789").unwrap();
        for (method, range, expected_status, expected_body) in [
            ("GET", "Range: bytes=2-5\r\n", "206 Partial Content", "2345"),
            ("HEAD", "", "200 OK", ""),
            (
                "GET",
                "Range: bytes=20-\r\n",
                "416 Range Not Satisfiable",
                "",
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let mut connection = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            write!(
                connection,
                "{method} /token/stream0.ts HTTP/1.1\r\n{range}\r\n"
            )
            .unwrap();
            let (server, _) = listener.accept().unwrap();
            serve(server, directory.path(), "token").unwrap();
            let mut response = String::new();
            connection.read_to_string(&mut response).unwrap();
            assert!(response.starts_with(&format!("HTTP/1.1 {expected_status}")));
            assert_eq!(response.split_once("\r\n\r\n").unwrap().1, expected_body);
        }
    }
}
