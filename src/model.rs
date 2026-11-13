use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Station {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(rename = "streamURL")]
    pub stream_url: String,
    #[serde(default, rename = "stationUUID")]
    pub station_uuid: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default, rename = "faviconURL")]
    pub favicon_url: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub bitrate: Option<u32>,
    #[serde(default)]
    pub media_source: Option<String>,
    #[serde(default)]
    pub youtube_channel_id: Option<String>,
    #[serde(default)]
    pub youtube_music_album_params: Option<String>,
    #[serde(default)]
    pub modified_at: Option<i64>,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub disc_number: Option<u32>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FavoriteCategory {
    #[default]
    Radio,
    YouTube,
    YouTubeMusic,
}
impl FavoriteCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Radio => "Radio",
            Self::YouTube => "YouTube",
            Self::YouTubeMusic => "YouTube Music",
        }
    }
}
impl Station {
    pub fn new(name: impl Into<String>, stream_url: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            artist: None,
            stream_url: stream_url.into(),
            station_uuid: None,
            homepage: None,
            favicon_url: None,
            tags: None,
            country_code: None,
            codec: None,
            bitrate: None,
            media_source: None,
            youtube_channel_id: None,
            youtube_music_album_params: None,
            modified_at: None,
            track_number: None,
            disc_number: None,
        }
    }
    pub fn is_file(&self) -> bool {
        self.stream_url.starts_with("file:") || self.stream_url.starts_with("smb:") || self.stream_url.starts_with("sftp:") || matches!(self.media_source.as_deref(), Some("remoteFile" | "youtube" | "youtubeMusic"))
    }
    /// A favorite that is a whole channel, not a single video: clicking
    /// it lists the channel's videos instead of playing a stream.
    pub fn is_youtube_channel(&self) -> bool {
        self.media_source.as_deref() == Some("youtubeChannel") || self.youtube_channel_id.is_some()
    }
    /// A favorite that is a whole YouTube Music artist or album, not a
    /// single track: clicking it opens that artist or album instead.
    pub fn is_music_artist(&self) -> bool {
        self.media_source.as_deref() == Some("youtubeMusicArtist")
    }
    pub fn is_music_album(&self) -> bool {
        self.media_source.as_deref() == Some("youtubeMusicAlbum")
    }
    pub fn favorite_category(&self) -> FavoriteCategory {
        match self.media_source.as_deref() {
            Some("youtubeMusic") | Some("youtubeMusicArtist") | Some("youtubeMusicAlbum") => return FavoriteCategory::YouTubeMusic,
            Some("youtube") | Some("youtubeChannel") => return FavoriteCategory::YouTube,
            _ => {}
        }
        let host = url::Url::parse(self.stream_url.trim())
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default()
            .to_lowercase();
        if host == "music.youtube.com" {
            FavoriteCategory::YouTubeMusic
        } else if host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be" {
            FavoriteCategory::YouTube
        } else {
            FavoriteCategory::Radio
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Enter a station name.".into());
        }
        let url = url::Url::parse(self.stream_url.trim()).map_err(|_| "Enter a valid URL.")?;
        if !["http", "https", "file", "smb", "sftp"].contains(&url.scheme()) {
            return Err("Use an HTTP, HTTPS, SMB, SFTP, or file URL.".into());
        }
        if url.scheme() == "smb" && !crate::smb::valid_url(url.as_str()) {
            return Err("Include an SMB share, for example smb://server/Music.".into());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("Use the Remote tab for authenticated URLs.".into());
        }
        Ok(())
    }
    pub fn normalize(&mut self) {
        self.name = self.name.trim().to_owned();
        self.stream_url = self.stream_url.trim().to_owned();
        for field in [&mut self.homepage, &mut self.favicon_url, &mut self.artist] {
            if let Some(value) = field {
                let trimmed = value.trim().to_owned();
                *field = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                };
            }
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Folder {
    pub path: PathBuf,
    pub tracks: Vec<Station>,
}
impl Folder {
    pub fn scan(path: PathBuf, previous: &[Station]) -> Result<Self, String> {
        Self::scan_depth(path, previous, usize::MAX)
    }
    fn scan_depth(path: PathBuf, previous: &[Station], depth: usize) -> Result<Self, String> {
        if !path.is_dir() {
            return Err("Music folder is unavailable.".into());
        }
        let mut tracks = Vec::new();
        for entry in walkdir::WalkDir::new(&path)
            .max_depth(depth)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
        {
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.file_type().is_file() || !is_media(entry.path()) {
                continue;
            }
            let url = url::Url::from_file_path(entry.path())
                .map_err(|_| "Invalid file path")?
                .to_string();
            let mut track = Station::new(
                entry
                    .path()
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy(),
                &url,
            );
            if let Some(old) = previous.iter().find(|s| s.stream_url == url) {
                track.id = old.id;
            }
            track.modified_at = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            if let Ok(metadata) = crate::file_metadata::read_summary(&url, None) {
                track.track_number = metadata.track_number;
                track.disc_number = metadata.disc_number;
                if let Some(title) = metadata.title { track.name = title; }
            }
            tracks.push(track);
        }
        tracks.sort_by_key(|s| s.name.to_lowercase());
        Ok(Self { path, tracks })
    }
}
pub struct LocalDirectory {
    pub path: PathBuf,
    pub folders: Vec<PathBuf>,
    pub tracks: Vec<Station>,
}
impl LocalDirectory {
    pub fn read(path: PathBuf, previous: &[Station]) -> Result<Self, String> {
        let entries = fs::read_dir(&path)
            .map_err(|_| "Music folder is unavailable or unreadable.".to_owned())?;
        let mut folders = Vec::new();
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                folders.push(entry.path());
            }
        }
        folders.sort_by_key(|path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
        });
        let tracks = Folder::scan_depth(path.clone(), previous, 1)?.tracks;
        Ok(Self {
            path,
            folders,
            tracks,
        })
    }
}
pub fn is_media(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        [
            "aac", "aif", "aifc", "aiff", "alac", "caf", "flac", "m4a", "m4b", "mp3", "mp4", "oga",
            "ogg", "opus", "wav", "mov", "m4v", "m3u8",
        ]
        .contains(&e.to_string_lossy().to_lowercase().as_str())
    })
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RemoteConnection {
    pub name: String,
    pub url: String,
    pub username: String,
}
#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Repeat {
    #[default]
    All,
    One,
    Once,
}
// A shuffle pass visits each queued file once before repeating.
pub fn shuffle_candidates(queue: &[Station], current: usize, history: &[Uuid]) -> Vec<usize> {
    queue.iter().enumerate()
        .filter(|(index, station)| *index != current && !history.contains(&station.id))
        .map(|(index, _)| index)
        .collect()
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum SortOrder {
    #[default]
    DateDescending,
    DateAscending,
    NameAscending,
    NameDescending,
    TrackDiscAscending,
}
impl SortOrder {
    pub fn label(self) -> &'static str {
        match self {
            Self::DateDescending => "Descending by Date",
            Self::DateAscending => "Ascending by Date",
            Self::NameAscending => "Ascending by Name",
            Self::NameDescending => "Descending by Name",
            Self::TrackDiscAscending => "Ascending by Track/Disc",
        }
    }
    pub fn compare_track_disc(self, a: (Option<u32>, Option<u32>), b: (Option<u32>, Option<u32>)) -> std::cmp::Ordering {
        if self != Self::TrackDiscAscending { return std::cmp::Ordering::Equal; }
        let key = |(disc, track): (Option<u32>, Option<u32>)| {
            let disc = disc.filter(|n| *n > 0);
            let track = track.filter(|n| *n > 0);
            (disc.is_none() && track.is_none(), disc.unwrap_or(1), track.unwrap_or(u32::MAX))
        };
        key(a).cmp(&key(b))
    }
    pub fn compare(
        self,
        a: &str,
        a_date: Option<i64>,
        b: &str,
        b_date: Option<i64>,
    ) -> std::cmp::Ordering {
        let name = a
            .to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b));
        match self {
            Self::DateDescending => b_date.cmp(&a_date).then(name),
            Self::DateAscending => a_date.is_none().cmp(&b_date.is_none())
                .then_with(|| a_date.cmp(&b_date)).then(name),
            Self::NameAscending | Self::TrackDiscAscending => name,
            Self::NameDescending => name.reverse(),
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub local_sort: SortOrder,
    pub remote_sort: SortOrder,
    pub stations: Vec<Station>,
    pub folders: Vec<Folder>,
    pub connections: Vec<RemoteConnection>,
    pub show_remote_path: bool,
    pub show_local_path: bool,
    pub show_hover_hints: bool,
    pub show_local_tab: bool,
    pub show_remote_tab: bool,
    pub show_youtube_tab: bool,
    pub show_youtube_music_tab: bool,
    pub download_directory: Option<PathBuf>,
    pub youtube_quality: crate::youtube::Quality,
    pub youtube_frame_rate: crate::youtube::FrameRate,
    pub youtube_hide_title: bool,
    pub local_remote_hide_title: bool,
    pub video_audio_only: bool,
    pub youtube_audio_only: bool,
    pub youtube_search_limit: u32,
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub sensitivity: f32,
    pub kaleido: crate::kaleido::Settings,
    pub selected_tab: usize,
    pub folder_index: usize,
    pub local_directory: Option<PathBuf>,
    pub compact_library: bool,
    pub visualizer_open: bool,
    pub visualizer_track_overlay: bool,
    pub show_meters: bool,
    pub show_meters_compact: bool,
    pub show_playback_visuals: bool,
    pub show_recent_tracks: bool,
    pub show_recent_tracks_compact: bool,
    pub auto_volume: bool,
    pub gapless_playback: bool,
    pub crossfade_playback: bool,
    pub crossfade_seconds: f64,
    pub equalizer: crate::equalizer::Settings,
    pub last_remote: RemoteConnection,
    pub last_station: Option<Station>,
    pub local_positions: std::collections::HashMap<String, f64>,
}
impl Default for Library {
    fn default() -> Self {
        let mut stations = [
            ("Groove Salad", "groovesalad"),
            ("Drone Zone", "dronezone"),
            ("Secret Agent", "secretagent"),
            ("Lush", "lush"),
        ]
        .into_iter()
        .map(|(name, slug)| Station::new(name, format!("https://ice2.somafm.com/{slug}-128-mp3")))
        .collect::<Vec<_>>();
        stations.sort_by_key(|s| s.name.to_lowercase());
        Self {
            local_sort: SortOrder::default(),
            remote_sort: SortOrder::default(),
            stations,
            folders: vec![],
            connections: vec![],
            show_remote_path: true,
            show_local_path: true,
            show_hover_hints: true,
            show_local_tab: true,
            show_remote_tab: true,
            show_youtube_tab: true,
            show_youtube_music_tab: true,
            download_directory: None,
            youtube_quality: crate::youtube::Quality::default(),
            youtube_frame_rate: crate::youtube::FrameRate::default(),
            youtube_hide_title: false,
            local_remote_hide_title: false,
            video_audio_only: false,
            youtube_audio_only: false,
            youtube_search_limit: 20,
            volume: 0.7,
            shuffle: false,
            repeat: Repeat::All,
            sensitivity: 1.0,
            kaleido: crate::kaleido::Settings::default(),
            selected_tab: 0,
            folder_index: 0,
            local_directory: None,
            compact_library: false,
            visualizer_open: false,
            visualizer_track_overlay: false,
            show_meters: true,
            show_meters_compact: true,
            show_playback_visuals: true,
            show_recent_tracks: true,
            show_recent_tracks_compact: true,
            auto_volume: false,
            gapless_playback: false,
            crossfade_playback: false,
            crossfade_seconds: 5.0,
            equalizer: crate::equalizer::Settings::default(),
            last_remote: RemoteConnection::default(),
            last_station: None,
            local_positions: std::collections::HashMap::new(),
        }
    }
}
impl Library {
    pub fn audio_only_for(&self, station: &Station) -> bool {
        if station.favorite_category() == FavoriteCategory::YouTube {
            self.youtube_audio_only
        } else {
            self.video_audio_only
        }
    }
    pub fn path() -> PathBuf {
        if let Some(path) = std::env::var_os("KALEIDO_DATA_DIR") {
            return PathBuf::from(path).join("library.json");
        }
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        #[cfg(target_os = "macos")]
        let dir = home.join("Library/Application Support/KaleidoRadio-Rust");
        #[cfg(not(target_os = "macos"))]
        let dir = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or(home.join(".local/share"))
            .join("KaleidoRadio-Rust");
        dir.join("library.json")
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Library could not be read: {e}. Existing file preserved."))
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        fs::create_dir_all(path.parent().ok_or("Invalid library path")?)
            .map_err(|e| e.to_string())?;
        let temp = path.with_extension("json.tmp");
        fs::write(
            &temp,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(temp, path).map_err(|e| e.to_string())
    }
    pub fn add(&mut self, station: Station) {
        if !self
            .stations
            .iter()
            .any(|s| s.stream_url == station.stream_url)
        {
            self.stations.push(station);
        }
        self.stations.sort_by_key(|s| s.name.to_lowercase());
    }
    pub fn import(&mut self, bytes: &[u8], replace: bool) -> Result<usize, String> {
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let mut incoming: Vec<Station> =
            serde_json::from_value(value.get("stations").unwrap_or(&value).clone())
                .map_err(|e| e.to_string())?;
        for station in &mut incoming {
            station.normalize();
            station.validate()?;
        }
        let count = incoming.len();
        if replace {
            self.stations.clear();
        }
        for s in incoming {
            self.add(s);
        }
        Ok(count)
    }
    pub fn export(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec_pretty(&serde_json::json!({"version":1,"exportedAt":chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs,true),"stations":self.stations}))
            .map_err(|e| e.to_string())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn swift_exchange_round_trip_and_merge() {
        let mut library = Library::default();
        let data = br#"{"version":1,"exportedAt":"2026-10-13T10:00:00Z","stations":[{"id":"12345678-1234-1234-1234-123456789012","name":"Test","streamURL":"https://example.org/live","faviconURL":null,"createdAt":"2026-01-01T00:00:00Z"}]}"#;
        assert_eq!(library.import(data, true).unwrap(), 1);
        library.import(data, false).unwrap();
        assert_eq!(library.stations.len(), 1);
        let mut second = Library::default();
        let exported = library.export().unwrap();
        let value: serde_json::Value = serde_json::from_slice(&exported).unwrap();
        assert!(
            chrono::DateTime::parse_from_rfc3339(value["exportedAt"].as_str().unwrap()).is_ok()
        );
        second.import(&exported, true).unwrap();
        assert_eq!(second.stations[0].id, library.stations[0].id);
    }
    #[test]
    fn invalid_import_is_atomic() {
        let mut library = Library::default();
        let count = library.stations.len();
        assert!(
            library
                .import(br#"[{"name":"bad","streamURL":"ftp://host/music"}]"#, true)
                .is_err()
        );
        assert_eq!(library.stations.len(), count);
    }
    #[test]
    fn settings_and_playback_selection_survive_restart() {
        let path = std::env::temp_dir()
            .join(format!("kaleido-settings-{}", Uuid::new_v4()))
            .join("library.json");
        let mut station = Station::new("Track", "file:///tmp/song.wav");
        station.artist = Some("Artist".into());
        let mut library = Library {
            volume: 0.23,
            sensitivity: 2.4,
            local_sort: SortOrder::TrackDiscAscending,
            remote_sort: SortOrder::TrackDiscAscending,
            show_remote_path: false,
            show_local_path: false,
            show_local_tab: false,
            show_remote_tab: false,
            show_youtube_tab: false,
            show_youtube_music_tab: false,
            youtube_quality: crate::youtube::Quality::P1080,
            video_audio_only: true,
            youtube_search_limit: 0,
            auto_volume: true,
            gapless_playback: true,
            crossfade_playback: true,
            crossfade_seconds: 3.5,
            equalizer: crate::equalizer::Settings { enabled: true, bands: [3.; 10], preamp: -2. },
            selected_tab: 1,
            folder_index: 2,
            local_directory: Some(PathBuf::from("/Music/Album")),
            compact_library: true,
            visualizer_open: true,
            last_station: Some(station.clone()),
            ..Default::default()
        };
        library.kaleido = crate::kaleido::Settings {
            auto: false,
            folds: 22,
            motion: 0.4,
            clockwise: false,
            complexity: 0.45,
            chroma: 1.7,
            palette: 3,
            pattern: 8,
            hue: 210.,
            seed: 2.,
            milkdrop: false,
            milkdrop_preset: 0,
            ..crate::kaleido::Settings::default()
        };
        library
            .local_positions
            .insert(station.stream_url.clone(), 42.5);
        library.save(&path).unwrap();
        let restored = Library::load(&path).unwrap();
        assert_eq!(restored.kaleido, library.kaleido);
        assert_eq!(restored.last_station.as_ref().unwrap().artist.as_deref(), Some("Artist"));
        assert_eq!(restored.last_station.unwrap().id, station.id);
        assert_eq!(restored.local_positions[&station.stream_url], 42.5);
        assert_eq!(restored.selected_tab, 1);
        assert_eq!(restored.folder_index, 2);
        assert_eq!(
            restored.local_directory,
            Some(PathBuf::from("/Music/Album"))
        );
        assert!(restored.visualizer_open && restored.compact_library);
        assert_eq!(restored.volume, 0.23);
        assert_eq!(restored.sensitivity, 2.4);
        assert!(restored.gapless_playback && restored.crossfade_playback);
        assert!(restored.auto_volume);
        assert!(!restored.show_remote_path);
        assert!(!restored.show_local_path);
        assert_eq!(restored.youtube_quality, crate::youtube::Quality::P1080);
        assert!(restored.video_audio_only);
        assert_eq!(restored.youtube_search_limit, 0);
        assert!(!restored.show_youtube_music_tab);
        assert!(!restored.show_local_tab && !restored.show_remote_tab && !restored.show_youtube_tab);
        assert_eq!(restored.crossfade_seconds, 3.5);
        assert_eq!(restored.equalizer, library.equalizer);
        assert!(restored.local_sort == SortOrder::TrackDiscAscending);
        assert!(restored.remote_sort == SortOrder::TrackDiscAscending);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn shuffle_pass_excludes_current_and_previously_played_tracks() {
        let queue = vec![Station::new("A", "file:///a"), Station::new("B", "file:///b"), Station::new("C", "file:///c")];
        assert_eq!(shuffle_candidates(&queue, 0, &[]), vec![1, 2]);
        assert_eq!(shuffle_candidates(&queue, 1, &[queue[0].id]), vec![2]);
        assert!(shuffle_candidates(&queue, 2, &[queue[0].id, queue[1].id]).is_empty());
        assert_eq!(shuffle_candidates(&queue, 2, &[]), vec![0, 1]);
    }

    #[test]
    fn old_libraries_get_new_settings_without_losing_favorites() {
        let restored: Library = serde_json::from_value(serde_json::json!({
            "stations": [{"name": "Station", "streamURL": "https://example.org/live"}],
            "visualizer": 2, "volume": 0.5
        }))
        .unwrap();
        assert_eq!(restored.stations.len(), 1);
        assert_eq!(restored.volume, 0.5);
        assert_eq!(restored.kaleido, crate::kaleido::Settings::default());
        assert!(restored.last_station.is_none());
        assert!(restored.show_remote_path);
        assert!(restored.show_local_path);
        assert_eq!(restored.youtube_quality, crate::youtube::Quality::P720);
        assert_eq!(restored.youtube_frame_rate, crate::youtube::FrameRate::Best);
        assert!(!restored.youtube_hide_title);
        assert!(!restored.local_remote_hide_title);
        assert!(!restored.video_audio_only);
        assert_eq!(restored.youtube_search_limit, 20);
        assert!(restored.show_youtube_music_tab);
        assert!(restored.show_local_tab && restored.show_remote_tab && restored.show_youtube_tab);
    }

    #[test]
    fn favorite_category_uses_media_source_then_url_host() {
        let mut youtube = Station::new("Video", "https://www.youtube.com/watch?v=abc");
        youtube.media_source = Some("youtube".into());
        assert_eq!(youtube.favorite_category(), FavoriteCategory::YouTube);
        let mut music = Station::new("Song", "https://music.youtube.com/watch?v=abc");
        music.media_source = Some("youtubeMusic".into());
        assert_eq!(music.favorite_category(), FavoriteCategory::YouTubeMusic);
        let plain_youtube = Station::new("Video", "https://youtu.be/abc");
        assert_eq!(plain_youtube.favorite_category(), FavoriteCategory::YouTube);
        let plain_music = Station::new("Song", "https://music.youtube.com/watch?v=abc");
        assert_eq!(plain_music.favorite_category(), FavoriteCategory::YouTubeMusic);
        let radio = Station::new("Radio", "https://example.org/live");
        assert_eq!(radio.favorite_category(), FavoriteCategory::Radio);
        let mut channel = Station::new("Channel", "https://www.youtube.com/channel/UCabc");
        channel.media_source = Some("youtubeChannel".into());
        channel.youtube_channel_id = Some("UCabc".into());
        assert!(channel.is_youtube_channel());
        assert_eq!(channel.favorite_category(), FavoriteCategory::YouTube);
        assert!(!youtube.is_youtube_channel());
    }
    #[test]
    fn audio_only_mode_is_independent_for_youtube_and_music() {
        let mut library = Library::default();
        library.youtube_audio_only = true;
        library.video_audio_only = false;
        let youtube = Station::new("Video", "https://www.youtube.com/watch?v=abc");
        let music = Station::new("Song", "https://music.youtube.com/watch?v=abc");
        let radio = Station::new("Radio", "https://example.org/live");
        assert!(library.audio_only_for(&youtube));
        assert!(!library.audio_only_for(&music));
        assert!(!library.audio_only_for(&radio));
    }
    #[test]
    fn normalize_trims_and_clears_empty_option_fields() {
        let mut station = Station::new("  Groove Salad ", " https://example.org/live ");
        station.homepage = Some("  ".into());
        station.favicon_url = Some(" https://example.org/art.png ".into());
        station.normalize();
        assert_eq!(station.name, "Groove Salad");
        assert_eq!(station.stream_url, "https://example.org/live");
        assert_eq!(station.homepage, None);
        assert_eq!(
            station.favicon_url.as_deref(),
            Some("https://example.org/art.png")
        );
    }
    #[test]
    fn local_navigation_lists_only_direct_files_and_includes_empty_folders() {
        let path = std::env::temp_dir().join(format!("kaleido-navigation-{}", Uuid::new_v4()));
        fs::create_dir_all(path.join("Album/Disc 1")).unwrap();
        fs::create_dir_all(path.join("Empty")).unwrap();
        fs::create_dir_all(path.join(".hidden")).unwrap();
        fs::write(path.join("root.mp3"), []).unwrap();
        fs::write(path.join("notes.txt"), []).unwrap();
        fs::write(path.join("Album/song.flac"), []).unwrap();
        fs::write(path.join("Album/Disc 1/nested.m4a"), []).unwrap();
        let scanned = Folder::scan(path.clone(), &[]).unwrap();
        let root = LocalDirectory::read(path.clone(), &scanned.tracks).unwrap();
        assert_eq!(root.folders, [path.join("Album"), path.join("Empty")]);
        assert_eq!(root.tracks.len(), 1);
        assert_eq!(root.tracks[0].name, "root");
        let album = LocalDirectory::read(path.join("Album"), &scanned.tracks).unwrap();
        assert_eq!(album.folders, [path.join("Album/Disc 1")]);
        assert_eq!(album.tracks.len(), 1);
        assert_eq!(album.tracks[0].name, "song");
        assert_eq!(
            album.tracks[0].id,
            scanned.tracks.iter().find(|s| s.name == "song").unwrap().id
        );
        let empty = LocalDirectory::read(path.join("Empty"), &scanned.tracks).unwrap();
        assert!(empty.folders.is_empty() && empty.tracks.is_empty());
        fs::remove_dir_all(&path).unwrap();
        assert!(LocalDirectory::read(path, &[]).is_err());
    }

    #[test]
    fn scan_skips_unreadable_entries() {
        let path = std::env::temp_dir().join(format!("kaleido-test-{}", Uuid::new_v4()));
        fs::create_dir_all(path.join("locked")).unwrap();
        fs::write(path.join("song.mp3"), []).unwrap();
        fs::write(path.join("locked/hidden.mp3"), []).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path.join("locked"), fs::Permissions::from_mode(0o000));
        }
        let folder = Folder::scan(path.clone(), &[]).unwrap();
        assert!(folder.tracks.iter().any(|t| t.name == "song"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path.join("locked"), fs::Permissions::from_mode(0o755));
        }
        fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod sort_tests {
    use super::*;

    #[test]
    fn track_disc_sort_is_numeric_disc_first_and_untagged_last() {
        let mut entries = [(Some(2), Some(1)), (None, None), (Some(1), Some(10)),
            (Some(1), Some(2)), (None, Some(1)), (Some(1), None)];
        entries.sort_by(|a, b| SortOrder::TrackDiscAscending.compare_track_disc(*a, *b));
        assert_eq!(entries, [(None, Some(1)), (Some(1), Some(2)), (Some(1), Some(10)),
            (Some(1), None), (Some(2), Some(1)), (None, None)]);
        assert_eq!(SortOrder::NameAscending.compare_track_disc((Some(2), Some(1)), (Some(1), Some(2))), std::cmp::Ordering::Equal);
    }

    #[test]
    fn sort_orders_and_missing_dates() {
        let mut entries = [("Zulu", Some(10)), ("alpha", Some(20)), ("Beta", None)];
        entries.sort_by(|a, b| SortOrder::default().compare(a.0, a.1, b.0, b.1));
        assert_eq!(entries.map(|e| e.0), ["alpha", "Zulu", "Beta"]);
        entries.sort_by(|a, b| SortOrder::NameAscending.compare(a.0, a.1, b.0, b.1));
        assert_eq!(entries.map(|e| e.0), ["alpha", "Beta", "Zulu"]);
        entries.sort_by(|a, b| SortOrder::NameDescending.compare(a.0, a.1, b.0, b.1));
        assert_eq!(entries.map(|e| e.0), ["Zulu", "Beta", "alpha"]);
    }

    #[test]
    fn ascending_date_sorts_oldest_first_with_missing_dates_last() {
        let mut entries = [("Zulu", Some(20)), ("missing", None), ("Beta", Some(10)), ("alpha", Some(10))];
        entries.sort_by(|a, b| SortOrder::DateAscending.compare(a.0, a.1, b.0, b.1));
        assert_eq!(entries.map(|e| e.0), ["alpha", "Beta", "Zulu", "missing"]);
        let library = Library { local_sort: SortOrder::DateAscending, remote_sort: SortOrder::DateAscending, ..Default::default() };
        let restored: Library = serde_json::from_str(&serde_json::to_string(&library).unwrap()).unwrap();
        assert!(restored.local_sort == SortOrder::DateAscending);
        assert!(restored.remote_sort == SortOrder::DateAscending);
    }

    #[test]
    fn old_library_defaults_to_newest_first() {
        let library: Library = serde_json::from_str("{}").unwrap();
        assert!(library.local_sort == SortOrder::DateDescending);
        assert!(library.remote_sort == SortOrder::DateDescending);
    }
}

#[cfg(test)]
mod video_discovery_tests {
    use super::*;
    #[test]
    fn local_directories_include_video_and_audio() {
        let path = std::env::temp_dir().join(format!("kaleido-video-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        for name in ["clip.mp4", "portrait.MOV", "movie.m4v", "stream.m3u8", "song.mp3", "notes.txt"] {
            fs::write(path.join(name), []).unwrap();
        }
        let folder = LocalDirectory::read(path.clone(), &[]).unwrap();
        assert_eq!(folder.tracks.len(), 5);
        assert!(folder.tracks.iter().all(Station::is_file));
        fs::remove_dir_all(path).unwrap();
    }
}
