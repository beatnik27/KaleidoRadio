//! YouTube search and just-in-time playback resolution through yt-dlp.
pub mod preload;
pub mod streaming;
use crate::model::Station;
use serde_json::Value;
use std::{
    io::{BufRead, Read},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Quality {
    P360,
    P480,
    #[default]
    P720,
    P1080,
    Best,
}
impl Quality {
    pub const ALL: [Self; 5] = [Self::P360, Self::P480, Self::P720, Self::P1080, Self::Best];
    pub fn label(self) -> &'static str {
        match self {
            Self::P360 => "360p",
            Self::P480 => "480p",
            Self::P720 => "720p",
            Self::P1080 => "1080p",
            Self::Best => "Best Available",
        }
    }
    pub(super) fn format(self, frame_rate: FrameRate) -> String {
        let cap = match self {
            Self::P360 => "[height<=360]",
            Self::P480 => "[height<=480]",
            Self::P720 => "[height<=720]",
            Self::P1080 => "[height<=1080]",
            Self::Best => "",
        };
        let fps = frame_rate.filter();
        // Native playback needs H.264 video and AAC audio in an MP4 container.
        // The fps cap, when set, applies to both the split and the
        // single-file fallback, so the fallback cannot pick a faster video.
        format!(
            "bestvideo[ext=mp4][vcodec^=avc1]{cap}{fps}+bestaudio[ext=m4a]/best[ext=mp4][vcodec^=avc1]{cap}{fps}"
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FrameRate {
    #[default]
    Best,
    Fps30,
    Fps60,
}
impl FrameRate {
    pub const ALL: [Self; 3] = [Self::Best, Self::Fps30, Self::Fps60];
    pub fn label(self) -> &'static str {
        match self {
            Self::Best => "Best Available",
            Self::Fps30 => "30 FPS",
            Self::Fps60 => "60 FPS",
        }
    }
    fn filter(self) -> &'static str {
        match self {
            Self::Best => "",
            Self::Fps30 => "[fps<=30]",
            Self::Fps60 => "[fps<=60]",
        }
    }
}

pub fn is_url(address: &str) -> bool {
    url::Url::parse(address).ok().is_some_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && matches!(
                url.host_str(),
                Some(
                    "youtube.com"
                        | "www.youtube.com"
                        | "m.youtube.com"
                        | "youtu.be"
                        | "music.youtube.com"
                )
            )
    })
}

/// A YouTube channel returned by a channel search.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub handle: Option<String>,
    pub url: String,
    pub description: Option<String>,
    pub follower_count: Option<u64>,
    pub thumbnail_url: Option<String>,
}

/// A channel as a favorite station: it lands in YouTube Favorites
/// (media_source `youtubeChannel`) and, when clicked there, lists
/// the channel's videos instead of playing a stream.
/// `artist` is the handle and `tags` the subscriber label, so the
/// Favorites card can show both, and `favicon_url` is the avatar.
pub fn channel_station(channel: &Channel) -> crate::model::Station {
    let mut station = crate::model::Station::new(&channel.name, &channel.url);
    station.artist = channel.handle.clone();
    station.media_source = Some("youtubeChannel".into());
    station.youtube_channel_id = Some(channel.id.clone());
    station.homepage = Some(channel.url.clone());
    station.favicon_url = channel.thumbnail_url.clone();
    station.tags = subscriber_label(channel.follower_count);
    station
}
/// The reverse of `channel_station`: a favorite station that is a
/// channel, back to a `Channel` that can be opened. The id falls
/// back to the channel URL itself, which `channel_videos_target`
/// also accepts, so an edited URL still opens something.
pub fn channel_for_station(station: &crate::model::Station) -> Option<Channel> {
    if !station.is_youtube_channel() {
        return None;
    }
    let id = station
        .youtube_channel_id
        .clone()
        .or_else(|| {
            station
                .stream_url
                .split("/channel/")
                .nth(1)
                .map(|rest| rest.trim_end_matches('/').to_owned())
        })
        .unwrap_or_else(|| station.stream_url.clone());
    Some(Channel {
        id,
        name: station.name.clone(),
        handle: station.artist.clone(),
        url: station.stream_url.clone(),
        description: None,
        follower_count: None,
        thumbnail_url: station.favicon_url.clone(),
    })
}
pub fn subscriber_label(count: Option<u64>) -> Option<String> {
    let count = count?;
    if count >= 1_000_000 {
        Some(format!("{:.1}M subscribers", count as f64 / 1_000_000.0))
    } else if count >= 1_000 {
        Some(format!("{:.1}K subscribers", count as f64 / 1_000.0))
    } else {
        Some(format!("{count} subscribers"))
    }
}

/// YouTube's channel-only search filter (`sp=EgIQAg==`).
pub fn channel_search_target(query: &str) -> String {
    let mut target = url::Url::parse("https://www.youtube.com/results").unwrap();
    target
        .query_pairs_mut()
        .append_pair("search_query", query)
        .append_pair("sp", "EgIQAg==");
    target.into()
}

/// The `/videos` tab for a channel id, handle, or channel URL.
pub fn channel_videos_target(reference: &str) -> String {
    let reference = reference.trim().trim_end_matches('/');
    if let Some(handle) = reference.strip_prefix('@') {
        return format!("https://www.youtube.com/@{handle}/videos");
    }
    if reference.starts_with("UC")
        && reference
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return format!("https://www.youtube.com/channel/{reference}/videos");
    }
    if reference.starts_with("http://") || reference.starts_with("https://") {
        return if reference.ends_with("/videos") {
            reference.to_owned()
        } else {
            format!("{reference}/videos")
        };
    }
    format!("https://www.youtube.com/@{reference}/videos")
}

fn error_message(error: &str) -> String {
    if error.contains("403") || error.contains("Forbidden") {
        return "YouTube refused the video download (403 Forbidden). Try another video or update yt-dlp.".into();
    }
    let safe = error
        .split_whitespace()
        .map(|word| {
            if word.contains("http://") || word.contains("https://") {
                "[URL omitted]"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if safe.is_empty() {
        "YouTube request failed.".into()
    } else {
        format!("YouTube: {}", safe.chars().take(600).collect::<String>())
    }
}

pub type DownloadProgress = std::sync::Arc<dyn Fn(f32) + Send + Sync>;

fn download_progress(line: &str) -> Option<f32> {
    if let Some(processor) = line.trim().strip_prefix("KALEIDO_POSTPROCESS:") {
        return matches!(processor, "Merger" | "Metadata" | "EmbedThumbnail" | "VideoRemuxer")
            .then_some(1.0);
    }
    let percent = line.strip_prefix("KALEIDO_PROGRESS:")?.trim().trim_end_matches('%');
    let percent = percent.parse::<f32>().ok()?;
    percent.is_finite().then(|| (percent / 100.0).clamp(0.0, 0.99))
}

fn run_download(
    args: &[&str],
    timeout: Option<Duration>,
    download: Option<&std::path::Path>,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
    progress: Option<DownloadProgress>,
) -> Result<String, String> {
    let mut command = Command::new(crate::audio::executable("yt-dlp"));
    command.args([
        "--ignore-config",
        "--no-playlist",
        "--no-warnings",
        "--socket-timeout",
        "15",
        "--retries",
        "1",
    ]);
    let deno = crate::audio::executable("deno");
    command.args(["--js-runtimes", &format!("deno:{deno}")]);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = streaming::spawn(&mut command)?;
    let stdout = child.0.stdout.take().ok_or("yt-dlp output unavailable")?;
    let stderr = child.0.stderr.take().ok_or("yt-dlp errors unavailable")?;
    let read = |stream: Box<dyn Read + Send>, limit| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stream.take(limit).read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let output = std::thread::spawn(move || {
        let mut output = String::new();
        for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(value) = download_progress(&line)
                && let Some(progress) = &progress
            {
                progress(value);
            }
            if output.len() < 8 * 1024 * 1024 {
                output.push_str(&line);
                output.push('\n');
            }
        }
        output
    });
    let errors = read(Box::new(stderr), 64 * 1024);
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    let mut oversized = false;
    let status = loop {
        if cancelled.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed)) {
            child.terminate();
            break None;
        }
        if download.is_some_and(|path| {
            let bytes = std::fs::read_dir(path)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|entry| entry.metadata().ok())
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len())
                .sum::<u64>();
            bytes > 512 * 1024 * 1024
        }) {
            oversized = true;
            child.terminate();
            break None;
        }
        match child.0.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if deadline.is_none_or(|deadline| Instant::now() < deadline) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                child.terminate();
                break None;
            }
        }
    };
    let output = output.join().unwrap_or_default();
    let error = errors.join().unwrap_or_default();
    match status {
        Some(status) if status.success() => Ok(output),
        Some(_) => Err(error_message(&error)),
        None if oversized => Err("Video exceeds the 512 MB fallback buffer limit.".into()),
        None => Err("YouTube request timed out. Try again.".into()),
    }
}

pub(super) fn artist_value(value: &Value) -> Option<String> {
    let name = if let Some(values) = value.as_array() {
        values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        value.as_str()?.to_owned()
    };
    let name = name.trim().trim_end_matches(" - Topic").trim();
    (!name.is_empty() && name != "NA").then(|| name.to_owned())
}
fn artist_name(value: &Value) -> Option<String> {
    ["artists", "artist", "creator"]
        .iter()
        .find_map(|key| value.get(key).and_then(artist_value))
}

fn station(value: &Value) -> Option<Station> {
    let id = value.get("id")?.as_str()?;
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    let mut station = Station::new(
        value.get("title")?.as_str()?,
        format!("https://www.youtube.com/watch?v={id}"),
    );
    station.artist = artist_name(value);
    station.media_source = Some("youtube".into());
    station.homepage = Some(station.stream_url.clone());
    station.favicon_url = Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"));
    let channel = value
        .get("channel")
        .or_else(|| value.get("uploader"))
        .or_else(|| value.get("playlist_channel"))
        .or_else(|| value.get("playlist_uploader"))
        .and_then(Value::as_str)
        .unwrap_or("YouTube");
    let duration = value
        .get("duration")
        .and_then(Value::as_f64)
        .map(crate::video::format_time);
    station.tags = Some(match duration {
        Some(duration) => format!("{channel} · {duration}"),
        None => channel.into(),
    });
    Some(station)
}

fn channel(value: &Value) -> Option<Channel> {
    let id = value
        .get("channel_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .or_else(|| value.get("id").and_then(Value::as_str))?;
    if !id.starts_with("UC")
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    let name = value
        .get("title")
        .or_else(|| value.get("channel"))
        .or_else(|| value.get("uploader"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    let url = value
        .get("channel_url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("https://www.youtube.com/channel/{id}"));
    let handle = value
        .get("uploader_id")
        .and_then(Value::as_str)
        .filter(|handle| handle.starts_with('@'))
        .map(str::to_owned);
    let description = value
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_owned);
    let thumbnail_url = value
        .get("thumbnails")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|thumbnail| {
            let url = thumbnail.get("url").and_then(Value::as_str)?;
            let width = thumbnail.get("width").and_then(Value::as_u64).unwrap_or(0);
            Some((width, url))
        })
        .max_by_key(|(width, _)| *width)
        .map(|(_, url)| url)
        .filter(|url| !url.is_empty())
        .map(|url| {
            if let Some(rest) = url.strip_prefix("//") {
                format!("https://{rest}")
            } else {
                url.to_owned()
            }
        });
    Some(Channel {
        id: id.to_owned(),
        name: name.to_owned(),
        handle,
        url,
        description,
        follower_count: value.get("channel_follower_count").and_then(Value::as_u64),
        thumbnail_url,
    })
}

fn search_target(query: &str, limit: u32) -> String {
    if is_url(query) {
        query.to_owned()
    } else if limit == 0 {
        format!("ytsearchall:{query}")
    } else {
        format!("ytsearch{limit}:{query}")
    }
}

fn music_search_target(query: &str) -> String {
    if is_url(query) {
        return query.to_owned();
    }
    let mut target = url::Url::parse("https://music.youtube.com/search").unwrap();
    target.query_pairs_mut().append_pair("q", query);
    target.set_fragment(Some("songs"));
    target.into()
}
fn music_station(value: &Value) -> Option<Station> {
    let mut station = station(value)?;
    station.stream_url = station
        .stream_url
        .replace("www.youtube.com", "music.youtube.com");
    station.homepage = Some(station.stream_url.clone());
    station.media_source = Some("youtubeMusic".into());
    if station.tags.as_deref() == Some("YouTube") {
        station.tags = Some("YouTube Music".into());
    }
    Some(station)
}

pub fn search_music(
    query: &str,
    limit: u32,
    cancelled: &std::sync::atomic::AtomicBool,
    on_result: impl FnMut(Station),
) -> Result<(), String> {
    search_source(query, limit, cancelled, true, on_result)
}

/// Emit results as they arrive so long searches remain useful and cancellable.
pub fn search(
    query: &str,
    limit: u32,
    cancelled: &std::sync::atomic::AtomicBool,
    on_result: impl FnMut(Station),
) -> Result<(), String> {
    search_source(query, limit, cancelled, false, on_result)
}
fn search_source(
    query: &str,
    limit: u32,
    cancelled: &std::sync::atomic::AtomicBool,
    music: bool,
    mut on_result: impl FnMut(Station),
) -> Result<(), String> {
    use std::io::BufRead;
    use std::sync::{atomic::Ordering, mpsc};
    if query.trim().is_empty() || cancelled.load(Ordering::Relaxed) {
        return Ok(());
    }
    let mut command = Command::new(crate::audio::executable("yt-dlp"));
    command.args([
        "--ignore-config",
        "--no-playlist",
        "--no-warnings",
        "--socket-timeout",
        "15",
        "--retries",
        "1",
        "--flat-playlist",
        "--lazy-playlist",
        "--dump-json",
        "--skip-download",
    ]);
    if limit > 0 {
        command.args(["--playlist-end", &limit.to_string()]);
    }
    command
        .args([
            "--",
            &if music {
                music_search_target(query.trim())
            } else {
                search_target(query.trim(), limit)
            },
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = streaming::spawn(&mut command)?;
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or("YouTube search output unavailable")?;
    let stderr = child
        .0
        .stderr
        .take()
        .ok_or("YouTube search errors unavailable")?;
    let (tx, rx) = mpsc::sync_channel(32);
    let reader = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            let count = match reader
                .by_ref()
                .take(1024 * 1024)
                .read_until(b'\n', &mut line)
            {
                Ok(count) => count,
                Err(error) => {
                    let _ = tx.send(Err(error.to_string()));
                    break;
                }
            };
            if count == 0 {
                break;
            }
            if line.last() != Some(&b'\n') && count >= 1024 * 1024 {
                let _ = tx.send(Err("YouTube returned an oversized search result.".into()));
                break;
            }
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let result = serde_json::from_slice::<Value>(&line)
                .map_err(|e| format!("Invalid YouTube result: {e}"));
            match result {
                Ok(value) => {
                    if let Some(station) = if music {
                        music_station(&value)
                    } else {
                        station(&value)
                    } {
                        if tx.send(Ok(station)).is_err() {
                            break;
                        }
                    }
                }
                Err(error) => {
                    let _ = tx.send(Err(error));
                    break;
                }
            }
        }
    });
    let errors = std::thread::spawn(move || {
        let mut error = String::new();
        let _ = stderr.take(64 * 1024).read_to_string(&mut error);
        error
    });
    let mut last_result = Instant::now();
    let result = loop {
        if cancelled.load(Ordering::Relaxed) {
            break Ok(());
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(station)) => {
                last_result = Instant::now();
                on_result(station);
            }
            Ok(Err(error)) => break Err(error),
            Err(mpsc::RecvTimeoutError::Disconnected) => match child.0.try_wait() {
                Ok(Some(status)) => {
                    break if status.success() {
                        Ok(())
                    } else {
                        Err(String::new())
                    };
                }
                Ok(None) if last_result.elapsed() < Duration::from_secs(60) => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                _ => break Err("YouTube search timed out.".into()),
            },
            Err(mpsc::RecvTimeoutError::Timeout)
                if last_result.elapsed() >= Duration::from_secs(60) =>
            {
                break Err("YouTube search timed out waiting for more results.".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    child.terminate();
    drop(rx);
    let _ = reader.join();
    let error = errors.join().unwrap_or_default();
    result.map_err(|message| {
        if message.is_empty() {
            error_message(&error)
        } else {
            message
        }
    })
}

/// Run a yt-dlp `--flat-playlist --dump-json` target and hand each parsed
/// JSON line to `on_value`, mirroring `search_source`'s cancellation and
/// timeout handling. Channel search and channel videos use this instead
/// of the video search path because they parse different shapes.
fn flat_playlist(
    target: &str,
    limit: u32,
    cancelled: &std::sync::atomic::AtomicBool,
    mut on_value: impl FnMut(Value),
) -> Result<(), String> {
    use std::io::BufRead;
    use std::sync::{atomic::Ordering, mpsc};
    if target.trim().is_empty() || cancelled.load(Ordering::Relaxed) {
        return Ok(());
    }
    let mut command = Command::new(crate::audio::executable("yt-dlp"));
    command.args([
        "--ignore-config",
        "--no-playlist",
        "--no-warnings",
        "--socket-timeout",
        "15",
        "--retries",
        "1",
        "--flat-playlist",
        "--lazy-playlist",
        "--dump-json",
        "--skip-download",
    ]);
    if limit > 0 {
        command.args(["--playlist-end", &limit.to_string()]);
    }
    command
        .args(["--", target])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = streaming::spawn(&mut command)?;
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or("YouTube output unavailable")?;
    let stderr = child
        .0
        .stderr
        .take()
        .ok_or("YouTube errors unavailable")?;
    let (tx, rx) = mpsc::sync_channel(32);
    let reader = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            let count = match reader
                .by_ref()
                .take(1024 * 1024)
                .read_until(b'\n', &mut line)
            {
                Ok(count) => count,
                Err(error) => {
                    let _ = tx.send(Err(error.to_string()));
                    break;
                }
            };
            if count == 0 {
                break;
            }
            if line.last() != Some(&b'\n') && count >= 1024 * 1024 {
                let _ = tx.send(Err("YouTube returned an oversized result.".into()));
                break;
            }
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let result = serde_json::from_slice::<Value>(&line)
                .map_err(|e| format!("Invalid YouTube result: {e}"));
            if tx.send(result).is_err() {
                break;
            }
        }
    });
    let errors = std::thread::spawn(move || {
        let mut error = String::new();
        let _ = stderr.take(64 * 1024).read_to_string(&mut error);
        error
    });
    let mut last_result = Instant::now();
    let result = loop {
        if cancelled.load(Ordering::Relaxed) {
            break Ok(());
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(value)) => {
                last_result = Instant::now();
                on_value(value);
            }
            Ok(Err(error)) => break Err(error),
            Err(mpsc::RecvTimeoutError::Disconnected) => match child.0.try_wait() {
                Ok(Some(status)) => {
                    break if status.success() {
                        Ok(())
                    } else {
                        Err(String::new())
                    };
                }
                Ok(None) if last_result.elapsed() < Duration::from_secs(60) => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                _ => break Err("YouTube search timed out.".into()),
            },
            Err(mpsc::RecvTimeoutError::Timeout)
                if last_result.elapsed() >= Duration::from_secs(60) =>
            {
                break Err("YouTube search timed out waiting for more results.".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    child.terminate();
    drop(rx);
    let _ = reader.join();
    let error = errors.join().unwrap_or_default();
    result.map_err(|message| {
        if message.is_empty() {
            error_message(&error)
        } else {
            message
        }
    })
}

/// Search for channels by name, emitting each channel as it arrives.
pub fn search_channels(
    query: &str,
    limit: u32,
    cancelled: &std::sync::atomic::AtomicBool,
    mut on_result: impl FnMut(Channel),
) -> Result<(), String> {
    if query.trim().is_empty() {
        return Ok(());
    }
    let target = channel_search_target(query.trim());
    flat_playlist(&target, limit, cancelled, |value| {
        if let Some(channel) = channel(&value) {
            on_result(channel);
        }
    })
}

/// List a channel's videos, newest first.
///
/// The channel's `/videos` tab already returns entries newest first,
/// and that order is kept. When yt-dlp does supply upload dates they
/// are used as a safety net: dated videos sort newest first and
/// undated ones keep their incoming order at the end.
pub fn search_channel_videos(
    reference: &str,
    limit: u32,
    cancelled: &std::sync::atomic::AtomicBool,
    mut on_result: impl FnMut(Station),
) -> Result<(), String> {
    if reference.trim().is_empty() {
        return Ok(());
    }
    let target = channel_videos_target(reference.trim());
    let mut videos: Vec<(Option<String>, Station)> = Vec::new();
    flat_playlist(&target, limit, cancelled, |value| {
        if let Some(station) = station(&value) {
            let date = value
                .get("upload_date")
                .and_then(Value::as_str)
                .map(str::to_owned);
            videos.push((date, station));
        }
    })?;
    sort_newest_first(&mut videos);
    for (_, station) in videos {
        on_result(station);
    }
    Ok(())
}

fn sort_newest_first(videos: &mut [(Option<String>, Station)]) {
    // Upload dates are YYYYMMDD, so string order is date order.
    // `sort_by` is stable, so equal or missing dates keep their
    // incoming (playlist) order.
    videos.sort_by(|(a, _), (b, _)| b.cmp(a));
}

/// Download and combine the preferred native-compatible streams before playback.
pub fn buffer(
    address: &str,
    quality: Quality,
    frame_rate: FrameRate,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
) -> Result<tempfile::NamedTempFile, String> {
    buffer_mode(address, quality, frame_rate, cancelled, false)
}
pub fn buffer_audio(
    address: &str,
    quality: Quality,
    frame_rate: FrameRate,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
) -> Result<tempfile::NamedTempFile, String> {
    buffer_mode(address, quality, frame_rate, cancelled, true)
}
pub fn save_download(
    address: &str,
    quality: Quality,
    frame_rate: FrameRate,
    cancelled: &std::sync::atomic::AtomicBool,
    audio_only: bool,
    destination: &std::path::Path,
    automatic: bool,
    progress: DownloadProgress,
) -> Result<(), String> {
    let parent = destination.parent().filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let directory = tempfile::tempdir_in(parent).map_err(|error| error.to_string())?;
    let staged = directory.path().join(if audio_only { "audio.m4a" } else { "video.mp4" });
    download_media(address, quality, frame_rate, Some(cancelled), audio_only, true, &staged, Some(progress))?;
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Download cancelled.".into());
    }
    if automatic {
        save_without_overwriting(staged, destination).map_err(|error| error.to_string())
    } else {
        std::fs::rename(staged, destination).map_err(|error| error.to_string())
    }
}

fn save_without_overwriting(
    staged: std::path::PathBuf,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    let mut staged = tempfile::TempPath::try_from_path(staged)?;
    let mut candidate = destination.to_owned();
    let stem = destination.file_stem().unwrap_or_default().to_string_lossy();
    let extension = destination.extension().unwrap_or_default().to_string_lossy();
    for number in 2.. {
        match staged.persist_noclobber(&candidate) {
            Ok(()) => return Ok(()),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                staged = error.path;
                candidate.set_file_name(format!("{stem} ({number}).{extension}"));
            }
            Err(error) => return Err(error.error),
        }
    }
    unreachable!("download filename suffixes exhausted");
}

fn buffer_mode(
    address: &str,
    quality: Quality,
    frame_rate: FrameRate,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
    audio_only: bool,
) -> Result<tempfile::NamedTempFile, String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let staged = directory.path().join("video.mp4");
    download_media(address, quality, frame_rate, cancelled, audio_only, false, &staged, None)?;
    let file = tempfile::Builder::new()
        .prefix("kaleido-youtube-")
        .suffix(".mp4")
        .tempfile()
        .map_err(|error| error.to_string())?;
    std::fs::rename(staged, file.path()).map_err(|error| error.to_string())?;
    Ok(file)
}

fn download_media(
    address: &str,
    quality: Quality,
    frame_rate: FrameRate,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
    audio_only: bool,
    manual: bool,
    destination: &std::path::Path,
    progress: Option<DownloadProgress>,
) -> Result<(), String> {
    let bounded = !manual || audio_only;
    if !is_url(address) {
        return Err("Enter a YouTube video URL.".into());
    }
    let output = destination.to_str().ok_or("Invalid YouTube download path")?.replace('%', "%%");
    let format = if audio_only {
        "bestaudio[ext=m4a]".to_owned()
    } else {
        quality.format(frame_rate)
    };
    let ffmpeg = crate::audio::executable("ffmpeg");
    let mut args = vec![
        "--no-part",
        "--no-continue",
        "--force-overwrites",
        "--format",
        &format,
        "--merge-output-format",
        "mp4",
        "--ffmpeg-location",
        &ffmpeg,
        "--output",
        &output,
    ];
    if manual {
        args.extend([
            "--embed-thumbnail",
            "--convert-thumbnails", "jpg",
            "--embed-metadata",
            "--progress",
            "--newline",
            "--no-colors",
            "--progress-delta", "0.25",
            "--progress-template", "download:KALEIDO_PROGRESS:%(progress._percent_str)s",
            "--progress-template", "postprocess:KALEIDO_POSTPROCESS:%(progress.postprocessor)s",
        ]);
    } else {
        args.push("--no-progress");
    }
    if bounded {
        args.extend(["--max-filesize", "512M"]);
    }
    args.extend(["--", address]);
    run_download(
        &args,
        bounded.then_some(Duration::from_secs(120)),
        destination.parent().filter(|_| bounded),
        cancelled,
        progress,
    )?;
    let size = std::fs::metadata(destination).map_or(0, |metadata| metadata.len());
    if bounded && size > 512 * 1024 * 1024 {
        return Err("Video exceeds the 512 MB fallback buffer limit.".into());
    }
    if size == 0 {
        return Err(if bounded {
            "YouTube returned no compatible media, or it exceeds the 512 MB download limit.".into()
        } else {
            "YouTube returned no compatible video.".into()
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_download_preserves_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("Video.mp4");
        let staged = directory.path().join("staged.mp4");
        let second = directory.path().join("Video (2).mp4");
        std::fs::write(&destination, b"original").unwrap();
        std::fs::write(&second, b"second").unwrap();
        std::fs::write(&staged, b"new download").unwrap();

        save_without_overwriting(staged, &destination).unwrap();

        assert_eq!(std::fs::read(destination).unwrap(), b"original");
        assert_eq!(std::fs::read(second).unwrap(), b"second");
        assert_eq!(std::fs::read(directory.path().join("Video (3).mp4")).unwrap(), b"new download");
    }

    #[test]
    fn download_progress_parses_stream_percentages_and_finishing() {
        assert_eq!(download_progress("KALEIDO_PROGRESS: 42.5%"), Some(0.425));
        assert_eq!(download_progress("KALEIDO_PROGRESS:100.0%"), Some(0.99));
        assert_eq!(download_progress("KALEIDO_POSTPROCESS:EmbedThumbnail"), Some(1.0));
        assert_eq!(download_progress("KALEIDO_POSTPROCESS:Merger"), Some(1.0));
        assert_eq!(download_progress("KALEIDO_POSTPROCESS:ThumbnailsConvertor"), None);
        for line in ["[download] Fetching video", "KALEIDO_PROGRESS:N/A", "KALEIDO_PROGRESS:NaN%"] {
            assert_eq!(download_progress(line), None);
        }
    }

    #[test]
    #[ignore = "requires yt-dlp, Deno, internet access, and Xcode command line tools"]
    #[cfg(target_os = "macos")]
    fn live_search_and_buffered_native_playback() {
        let mut results = Vec::new();
        search(
            "Big Buck Bunny",
            20,
            &std::sync::atomic::AtomicBool::new(false),
            |station| results.push(station),
        )
        .unwrap();
        assert!(!results.is_empty());
        let address = std::env::var("KALEIDO_YOUTUBE_TEST_URL")
            .unwrap_or_else(|_| "https://www.youtube.com/watch?v=aqz-KE-bpKQ".into());
        let file = buffer(&address, Quality::default(), FrameRate::default(), None).unwrap();
        let probe_output = Command::new(crate::audio::executable("ffprobe"))
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,height",
                "-of",
                "json",
            ])
            .arg(file.path())
            .output()
            .unwrap();
        assert!(probe_output.status.success());
        let metadata: Value = serde_json::from_slice(&probe_output.stdout).unwrap();
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
        let address = url::Url::from_file_path(file.path()).unwrap();
        let output = Command::new(probe)
            .args([address.as_str(), "--preview"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    #[ignore = "requires yt-dlp and internet access"]
    fn live_custom_and_unlimited_search_with_cancellation() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let cancelled = AtomicBool::new(false);
        let mut count = 0;
        search("Big Buck Bunny", 25, &cancelled, |_| count += 1).unwrap();
        assert_eq!(count, 25);
        count = 0;
        search("Big Buck Bunny", 0, &cancelled, |_| {
            count += 1;
            if count == 25 {
                cancelled.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
        assert_eq!(count, 25);
        assert!(cancelled.load(Ordering::Relaxed));
    }
    #[test]
    #[ignore = "requires yt-dlp, FFmpeg, internet access, and Xcode command line tools"]
    #[cfg(target_os = "macos")]
    fn live_music_search_and_playback() {
        let mut results = Vec::new();
        search_music(
            "Luttrell",
            3,
            &std::sync::atomic::AtomicBool::new(false),
            |station| results.push(station),
        )
        .unwrap();
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|station| {
            station
                .stream_url
                .starts_with("https://music.youtube.com/watch?v=")
        }));
        let stream = streaming::start(
            &results[0].stream_url,
            Quality::default(),
            FrameRate::default(),
            0.0,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();
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
            .args([&stream.address, "--preview"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn music_search_uses_songs_and_keeps_music_links() {
        let target = url::Url::parse(&music_search_target("Luttrell & friends")).unwrap();
        assert_eq!(target.host_str(), Some("music.youtube.com"));
        assert_eq!(target.fragment(), Some("songs"));
        assert_eq!(
            target.query_pairs().find(|(key, _)| key == "q").unwrap().1,
            "Luttrell & friends"
        );
        let station = music_station(&serde_json::json!({"id":"abc_123", "title":"Song"})).unwrap();
        assert_eq!(
            station.stream_url,
            "https://music.youtube.com/watch?v=abc_123"
        );
        assert!(station.is_file());
        assert!(crate::video::is_video_url(&station.stream_url));
    }
    #[test]
    fn custom_and_unlimited_search_targets() {
        assert_eq!(search_target("music", 75), "ytsearch75:music");
        assert_eq!(search_target("music", 0), "ytsearchall:music");
        assert_eq!(
            search_target("https://youtu.be/abc", 0),
            "https://youtu.be/abc"
        );
    }
    #[test]
    fn cancelled_search_does_not_start_a_process() {
        search(
            "unused",
            0,
            &std::sync::atomic::AtomicBool::new(true),
            |_| panic!("No results expected"),
        )
        .unwrap();
    }
    #[test]
    fn artist_metadata_prefers_performers_over_uploader() {
        assert_eq!(
            artist_name(
                &serde_json::json!({"artists":["Artist A", "Artist B"],"uploader":"Label"})
            )
            .as_deref(),
            Some("Artist A, Artist B")
        );
        assert!(artist_name(&serde_json::json!({"uploader":"Label"})).is_none());
    }
    #[test]
    fn preferred_quality_defaults_to_720p_and_sets_resolution_caps() {
        assert_eq!(Quality::default(), Quality::P720);
        assert_eq!(FrameRate::default(), FrameRate::Best);
        for (quality, height) in [
            (Quality::P360, 360),
            (Quality::P480, 480),
            (Quality::P720, 720),
            (Quality::P1080, 1080),
        ] {
            let format = quality.format(FrameRate::Best);
            assert_eq!(format.matches(&format!("[height<={height}]")).count(), 2);
            assert!(format.contains("+bestaudio[ext=m4a]"));
        }
        assert!(!Quality::Best.format(FrameRate::Best).contains("height<="));
    }
    #[test]
    fn frame_rate_caps_apply_to_both_format_parts() {
        let format = Quality::P720.format(FrameRate::Fps30);
        assert_eq!(format.matches("[fps<=30]").count(), 2);
        assert!(format.contains("[height<=720]"));
        let format = Quality::Best.format(FrameRate::Fps60);
        assert_eq!(format.matches("[fps<=60]").count(), 2);
        assert!(!format.contains("height<="));
        assert!(!Quality::P720.format(FrameRate::Best).contains("fps<="));
    }
    #[test]
    fn errors_do_not_expose_temporary_stream_urls() {
        let message = error_message(
            "HTTP Error 403: Forbidden for https://cdn.googlevideo.com/video?sig=secret",
        );
        assert!(message.contains("403"));
        assert!(!message.contains("secret"));
        let message =
            error_message("Unable to fetch https://example.org/video?token=private network failed");
        assert!(!message.contains("private"));
        assert!(message.contains("network failed"));
    }
    #[test]
    fn accepts_only_youtube_hosts() {
        assert!(is_url("https://youtu.be/abc"));
        assert!(is_url("https://www.youtube.com/watch?v=abc"));
        assert!(!is_url("https://youtube.com.evil.org/watch?v=abc"));
        assert!(!is_url("file:///youtube.com"));
    }
    #[test]
    fn results_keep_watch_urls_and_metadata() {
        let s = station(&serde_json::json!({"id":"abc_123", "title":"Video", "channel":"Channel", "duration":125.0})).unwrap();
        assert_eq!(s.stream_url, "https://www.youtube.com/watch?v=abc_123");
        assert_eq!(s.tags.as_deref(), Some("Channel · 2:05"));
        assert_eq!(s.media_source.as_deref(), Some("youtube"));
        assert!(s.is_file());
        assert!(crate::video::is_video_url(&s.stream_url));
        assert!(matches!(
            crate::card_artwork::Source::station(&s),
            Some(crate::card_artwork::Source::Image(_))
        ));
        assert!(station(&serde_json::json!({"id":"../bad", "title":"Bad"})).is_none());
    }
    #[test]
    fn channel_search_target_uses_channel_filter() {
        let target = url::Url::parse(&channel_search_target("Blender & friends")).unwrap();
        assert_eq!(target.host_str(), Some("www.youtube.com"));
        assert_eq!(
            target.query_pairs().find(|(key, _)| key == "search_query").unwrap().1,
            "Blender & friends"
        );
        assert_eq!(
            target.query_pairs().find(|(key, _)| key == "sp").unwrap().1,
            "EgIQAg=="
        );
    }
    #[test]
    fn channel_station_round_trips_to_a_channel_favorite() {
        let channel = Channel {
            id: "UCabc".into(),
            name: "Example Channel".into(),
            handle: Some("@example".into()),
            url: "https://www.youtube.com/channel/UCabc".into(),
            description: Some("About".into()),
            follower_count: Some(1_250_000),
            thumbnail_url: Some("https://example.com/avatar.jpg".into()),
        };
        let station = channel_station(&channel);
        assert!(station.is_youtube_channel());
        assert_eq!(station.favorite_category(), crate::model::FavoriteCategory::YouTube);
        assert_eq!(station.favicon_url.as_deref(), Some("https://example.com/avatar.jpg"));
        let back = channel_for_station(&station).unwrap();
        assert_eq!(back.id, "UCabc");
        assert_eq!(back.name, "Example Channel");
        assert_eq!(back.handle.as_deref(), Some("@example"));
        assert_eq!(channel_videos_target(&back.id), "https://www.youtube.com/channel/UCabc/videos");
        let video = crate::model::Station::new("Video", "https://www.youtube.com/watch?v=abc");
        assert!(channel_for_station(&video).is_none());
    }
    #[test]
    fn channel_videos_target_for_id_handle_and_url() {
        assert_eq!(
            channel_videos_target("UCSMOQeBJ2RAnuFungnQOxLg"),
            "https://www.youtube.com/channel/UCSMOQeBJ2RAnuFungnQOxLg/videos"
        );
        assert_eq!(
            channel_videos_target("@BlenderOfficial"),
            "https://www.youtube.com/@BlenderOfficial/videos"
        );
        assert_eq!(
            channel_videos_target("https://www.youtube.com/channel/UCSMOQeBJ2RAnuFungnQOxLg"),
            "https://www.youtube.com/channel/UCSMOQeBJ2RAnuFungnQOxLg/videos"
        );
        assert_eq!(
            channel_videos_target("https://www.youtube.com/channel/UCSMOQeBJ2RAnuFungnQOxLg/videos"),
            "https://www.youtube.com/channel/UCSMOQeBJ2RAnuFungnQOxLg/videos"
        );
    }
    #[test]
    fn channel_parses_search_result_and_rejects_videos() {
        let parsed = channel(&serde_json::json!({
            "id": "UCSMOQeBJ2RAnuFungnQOxLg",
            "channel_id": "UCSMOQeBJ2RAnuFungnQOxLg",
            "title": "Blender",
            "uploader_id": "@BlenderOfficial",
            "channel_url": "https://www.youtube.com/channel/UCSMOQeBJ2RAnuFungnQOxLg",
            "description": "Official YouTube channel for Blender.",
            "channel_follower_count": 1_250_000,
            "thumbnails": [{"url": "//yt3.example/avatar=s88", "width": 88}, {"url": "//yt3.example/avatar=s176", "width": 176}]
        }))
        .unwrap();
        assert_eq!(parsed.name, "Blender");
        assert_eq!(parsed.handle.as_deref(), Some("@BlenderOfficial"));
        assert_eq!(parsed.follower_count, Some(1_250_000));
        assert_eq!(parsed.thumbnail_url.as_deref(), Some("https://yt3.example/avatar=s176"));
        assert!(channel(&serde_json::json!({"id": "dVrELpmfcEQ", "title": "A video"})).is_none());
    }
    #[test]
    fn subscriber_label_formats_counts() {
        assert_eq!(subscriber_label(Some(1_250_000)).as_deref(), Some("1.2M subscribers"));
        assert_eq!(subscriber_label(Some(57_700)).as_deref(), Some("57.7K subscribers"));
        assert_eq!(subscriber_label(Some(42)).as_deref(), Some("42 subscribers"));
        assert!(subscriber_label(None).is_none());
    }
    #[test]
    fn station_uses_playlist_channel_when_uploader_missing() {
        let s = station(&serde_json::json!({
            "id": "dVrELpmfcEQ",
            "title": "A video",
            "duration": 60.0,
            "playlist_channel": "Blender"
        }))
        .unwrap();
        assert_eq!(s.tags.as_deref(), Some("Blender · 1:00"));
    }
    #[test]
    fn channel_videos_sort_newest_first_with_missing_dates_last() {
        let mut videos: Vec<(Option<String>, Station)> = [
            (None, station(&serde_json::json!({"id": "aaa111", "title": "Undated"})).unwrap()),
            (Some("20240101".to_owned()), station(&serde_json::json!({"id": "bbb222", "title": "Older"})).unwrap()),
            (Some("20250101".to_owned()), station(&serde_json::json!({"id": "ccc333", "title": "Newer"})).unwrap()),
        ]
        .into_iter()
        .collect();
        sort_newest_first(&mut videos);
        let titles: Vec<_> = videos.iter().map(|(_, s)| s.name.as_str()).collect();
        assert_eq!(titles, ["Newer", "Older", "Undated"]);
    }
}
