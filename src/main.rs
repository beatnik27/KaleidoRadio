mod audio;
mod auto_volume;
mod equalizer;
mod smb;
mod sftp;
mod video;
mod youtube;
mod artwork_cache;
mod card_artwork;
mod credentials;
mod file_metadata;
mod hover_hints;
mod icons;
mod kaleido;
mod media;
mod model;
mod music;
mod network;
mod theme;
mod visualizer_window;
use eframe::egui;
use crate::hover_hints::HoverHint;
use icons::{Icon, IconButton};
use notify::Watcher;
use model::{FavoriteCategory, Folder, Library, LocalDirectory, RemoteConnection, Repeat, SortOrder, Station};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Favorites,
    Local,
    Discover,
    Remote,
    YouTube,
    YouTubeMusic,
}
enum TaskResult {
    YouTubeDownloaded(Result<(), String>),
    YouTubeDownloadProgress(f32),
    YouTubePreloaded(u64, Result<YouTubeReady, String>),
    Music(u64, Result<music::ResultPage, String>),
    RadioMore(u64, Result<music::ResultPage, String>),
    YouTubeSearch(bool, u64, Result<(), String>),
    YouTubeSearchItem(bool, u64, Station),
    YouTubeChannelItem(u64, youtube::Channel),
    YouTubeChannels(u64, Result<(), String>),
    YouTubeChannelVideos(u64, Result<Vec<Station>, String>),
    YouTubeStreaming(u64, f64, Result<youtube::streaming::Session, String>),
    YouTubeFallback(u64),
    YouTubeBuffered(u64, f64, Result<tempfile::NamedTempFile, String>),
    Search(u64, Result<Vec<Station>, String>),
    PreparedTransition(u64, Uuid, Result<audio::Prepared, String>),
    SMBPlayback(u64, f64, Result<String, String>),
    Browse(u64, Result<Vec<network::RemoteEntry>, String>),
    Artwork(u64, Result<Vec<u8>, String>),
    FileMetadata(u64, Result<file_metadata::Metadata, String>),
    RemoteEntryMetadata(u64, String, file_metadata::Metadata),
    Folder(u64, Result<Folder, String>),
    LocalDirectory(u64, Result<LocalDirectory, String>),
}
struct RecentTrack {
    title: String,
    artwork: Option<egui::TextureHandle>,
}
#[derive(Default)]
struct YouTubeSearchState {
    query: String,
    results: Vec<Station>,
    searching: bool,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    generation: u64,
    error: Option<String>,
    channel_mode: bool,
    channels: Vec<youtube::Channel>,
    active_channel: Option<youtube::Channel>,
    channel_results: Vec<Station>,
}
#[derive(Clone, Copy, Default, PartialEq)]
enum MusicView { #[default] Artists, Albums, Album, Shuffle, Radio }
#[derive(Default)]
struct MusicBrowser {
    query: String,
    artists: Vec<music::Artist>,
    profile: Option<music::Profile>,
    album: Option<music::Album>,
    tracks: Vec<Station>,
    view: MusicView,
    loading: bool,
    error: Option<String>,
    generation: u64,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
struct MusicRadio {
    epoch: u64,
    page: music::RadioPage,
    loading: bool,
    waiting: bool,
    ids: std::collections::HashSet<Uuid>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    artist: String,
}
impl Drop for MusicRadio {
    fn drop(&mut self) { self.cancel.store(true, std::sync::atomic::Ordering::Relaxed); }
}
enum YouTubeMedia {
    Stream(youtube::streaming::Session),
    Buffered(tempfile::NamedTempFile),
}
impl YouTubeMedia {
    fn address(&self) -> Result<String, String> {
        match self {
            Self::Stream(stream) => Ok(stream.address.clone()),
            Self::Buffered(file) => url::Url::from_file_path(file.path()).map(String::from).map_err(|_| "Invalid preload path".into()),
        }
    }
}
struct YouTubeReady { media: YouTubeMedia, prepared: Option<audio::Prepared> }
#[derive(Clone, PartialEq)]
struct PreloadKey {
    owner: Uuid,
    audio: bool,
    quality: youtube::Quality,
    frame_rate: youtube::FrameRate,
    shuffle: bool,
    repeat: Repeat,
    radio: Option<u64>,
    transitions: (bool, bool, u64),
}
struct YouTubePreload {
    epoch: u64,
    key: PreloadKey,
    target: Uuid,
    reset_history: bool,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ready: Option<YouTubeReady>,
}
struct App {
    library: Library,
    path: PathBuf,
    writable: bool,
    player: audio::Player,
    playback_generation: u64,
    youtube_preload: Option<YouTubePreload>,
    retired_youtube_preloads: Vec<(Instant, YouTubePreload)>,
    preload_epoch: u64,
    transition_generation: u64,
    transition_next: Option<Station>,
    transition_settings: Option<(bool, bool, f64, bool, Repeat)>,
    smb_connecting: bool,
    tab: Tab,
    current: Option<Station>,
    queue: Vec<Station>,
    queue_from_discover: bool,
    auth: Option<(String, String)>,
    status: String,
    title: String,
    album: Option<String>,
    history: Vec<RecentTrack>,
    duration: f64,
    metrics: audio::Metrics,
    filter: String,
    compact_library: bool,
    youtube_download_progress: f32,
    youtube_buffer: Option<tempfile::NamedTempFile>,
    youtube_download: Option<(
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        std::thread::JoinHandle<()>,
    )>,
    youtube_stream: Option<youtube::streaming::Session>,
    youtube_resume_paused: bool,
    youtube_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    youtube_search: YouTubeSearchState,
    youtube_music_search: YouTubeSearchState,
    music_browser: MusicBrowser,
    music_artist_mode: bool,
    music_radio: Option<MusicRadio>,
    music_radio_epoch: u64,
    query: String,
    results: Vec<Station>,
    searching: bool,
    generation: u64,
    tx: Sender<TaskResult>,
    rx: Receiver<TaskResult>,
    artwork: Option<egui::TextureHandle>,
    video_texture: Option<egui::TextureHandle>,
    video_fullscreen: bool,
    video_controls: video::ControlsFade,
    video_seekable: bool,
    video_buffered: f64,
    video_muted: bool,
    video_toggle_at: Option<f64>,
    current_has_video: bool,
    source_has_video: bool,
    seek_position: Option<f64>,
    keep_video_frame: bool,
    audio_starved: bool,
    card_artwork: card_artwork::Cache,
    show_inline_visualizer: bool,
    inline_toggle_at: Option<f64>,
    artwork_generation: u64,
    artwork_color: egui::Color32,
    remote: RemoteConnection,
    remote_settings_open: bool,
    settings_open: bool,
    reveal_equalizer: bool,
    favorite_category: FavoriteCategory,
    password: String,
    remote_address: String,
    remote_filter: String,
    entries: Vec<network::RemoteEntry>,
    browsing: bool,
    remote_background_refresh: bool,
    remote_error: Option<String>,
    remote_generation: u64,
    remote_metadata_cache: std::collections::HashMap<(String, String), file_metadata::Metadata>,
    remote_metadata_pending: usize,
    remote_metadata_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    edit: Option<Station>,
    edit_existing: bool,
    visualizer_open: bool,
    popup_track_card_drawn: bool,
    kaleido: kaleido::Kaleido,
    folder_index: usize,
    local_directory: Option<LocalDirectory>,
    local_directory_path: Option<PathBuf>,
    local_directory_generation: u64,
    local_directory_loading: bool,
    local_directory_error: Option<String>,
    local_watcher: Option<notify::RecommendedWatcher>,
    local_watch_path: Option<PathBuf>,
    local_changes_tx: Sender<()>,
    local_changes_rx: Receiver<()>,
    local_change_at: Option<Instant>,
    folder_generation: u64,
    started: Instant,
    retry_at: Option<Instant>,
    retries: u8,
    shuffle_history: Vec<Uuid>,
    scanning: bool,
    media: media::Bridge,
    media_updated: Instant,
    settings_checked: Instant,
    last_saved: String,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // macOS fullscreen normally only auto-hides the menu bar, so it
        // slides back in and shrinks the video (a 24px menu bar costs
        // a 16:9 video about 42px of width). Borderless-game mode hides
        // the menu bar and dock for the whole fullscreen session, and
        // winit restores them on exit, so fullscreen video gets the
        // full monitor, including its width.
        #[cfg(target_os = "macos")]
        if let Some(window) = cc.winit_window() {
            use winit::platform::macos::WindowExtMacOS;
            window.set_borderless_game(true);
        }
        theme::apply(&cc.egui_ctx);
        let path = Library::path();
        let loaded = Library::load(&path);
        let writable = loaded.is_ok();
        let status = loaded
            .as_ref()
            .err()
            .cloned()
            .unwrap_or_else(|| "Choose a station or open your music folder.".into());
        let (tx, rx) = mpsc::channel();
        let (local_changes_tx, local_changes_rx) = mpsc::channel();
        let mut library = loaded.unwrap_or_default();
        hover_hints::set_enabled(&cc.egui_ctx, library.show_hover_hints);
        for track in library
            .folders
            .iter_mut()
            .flat_map(|folder| &mut folder.tracks)
        {
            if track.modified_at.is_none() {
                track.modified_at = url::Url::parse(&track.stream_url)
                    .ok()
                    .and_then(|url| url.to_file_path().ok())
                    .and_then(|path| std::fs::metadata(path).ok())
                    .and_then(|metadata| metadata.modified().ok())
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs() as i64);
            }
        }
        let current = library.last_station.clone();
        let queue = current
            .as_ref()
            .and_then(|station| {
                library
                    .folders
                    .iter()
                    .find(|folder| folder.tracks.iter().any(|track| track.id == station.id))
                    .map(|folder| {
                        let parent = url::Url::parse(&station.stream_url).ok()
                            .and_then(|url| url.to_file_path().ok())
                            .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
                        folder.tracks.iter().filter(|track| url::Url::parse(&track.stream_url).ok()
                            .and_then(|url| url.to_file_path().ok())
                            .is_some_and(|path| path.parent() == parent.as_deref()))
                            .cloned().collect()
                    })
            })
            .unwrap_or_else(|| library.stations.clone());
        let mut kaleido = kaleido::Kaleido::restore(&library.kaleido);
        if let Some(gl) = &cc.gl { kaleido.initialize_renderer(gl); }
        kaleido.initialize_projectm();
        let tab = match library.selected_tab {
            1 => Tab::Local,
            2 => Tab::Discover,
            3 => Tab::Remote,
            4 => Tab::YouTube,
            5 => Tab::YouTubeMusic,
            _ => Tab::Favorites,
        };
        let compact_library = library.compact_library;
        let visualizer_open = library.visualizer_open;
        let remote = library.last_remote.clone();
        let password = credentials::load(&remote);
        let folder_index = library
            .folder_index
            .min(library.folders.len().saturating_sub(1));
        let title = current.as_ref().map(|s| s.name.clone()).unwrap_or_default();
        let last_saved = serde_json::to_string(&library).unwrap_or_default();
        let source_has_video = current.as_ref().is_some_and(|station| video::is_video_url(&station.stream_url));
        let current_has_video = current.as_ref().is_some_and(|station| source_has_video && (video::is_file_video(station) || !library.audio_only_for(station)));
        let mut app = Self {
            library,
            path,
            writable,
            player: audio::Player::new(),
            playback_generation: 0,
            youtube_preload: None,
            retired_youtube_preloads: Vec::new(),
            preload_epoch: 0,
            transition_generation: 0,
            transition_next: None,
            transition_settings: None,
            smb_connecting: false,
            tab,
            current,
            queue,
            queue_from_discover: false,
            auth: None,
            status,
            title,
            album: None,
            history: vec![],
            duration: 0.0,
            metrics: audio::Metrics::default(),
            filter: String::new(),
            compact_library,
            youtube_download_progress: 0.0,
            youtube_buffer: None,
            youtube_download: None,
            youtube_stream: None,
            youtube_resume_paused: false,
            youtube_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            youtube_search: YouTubeSearchState::default(),
            youtube_music_search: YouTubeSearchState::default(),
            music_browser: MusicBrowser::default(),
            music_artist_mode: true,
            music_radio: None,
            music_radio_epoch: 0,
            query: String::new(),
            results: vec![],
            searching: false,
            generation: 0,
            tx,
            rx,
            artwork: None,
            video_texture: None,
            video_fullscreen: false,
            video_controls: Default::default(),
            video_seekable: false,
            video_buffered: 0.0,
            video_muted: false,
            video_toggle_at: None,
            current_has_video,
            source_has_video,
            seek_position: None,
            keep_video_frame: false,
            audio_starved: false,
            card_artwork: Default::default(),
            show_inline_visualizer: false,
            inline_toggle_at: None,
            artwork_generation: 0,
            artwork_color: theme::DEFAULT_ARTWORK_COLOR,
            remote,
            remote_settings_open: false,
            settings_open: false,
            reveal_equalizer: false,
            favorite_category: FavoriteCategory::Radio,
            password,
            remote_address: String::new(),
            remote_filter: String::new(),
            entries: vec![],
            browsing: false,
            remote_background_refresh: false,
            remote_error: None,
            remote_generation: 0,
            remote_metadata_cache: Default::default(),
            remote_metadata_pending: 0,
            remote_metadata_cancel: Default::default(),
            edit: None,
            edit_existing: false,
            visualizer_open,
            popup_track_card_drawn: false,
            kaleido,
            folder_index,
            local_directory: None,
            local_directory_path: None,
            local_directory_generation: 0,
            local_directory_loading: false,
            local_directory_error: None,
            local_watcher: None,
            local_watch_path: None,
            local_changes_tx,
            local_changes_rx,
            local_change_at: None,
            folder_generation: 0,
            started: Instant::now(),
            retry_at: None,
            retries: 0,
            shuffle_history: vec![],
            scanning: false,
            media: media::Bridge::new(),
            media_updated: Instant::now(),
            settings_checked: Instant::now(),
            last_saved,
        };
        app.player.set_equalizer(app.library.equalizer);
        app.player.set_auto_volume(app.library.auto_volume);
        if app.current.is_some() {
            if app.writable {
                app.status = "Stopped. Press Play to resume.".into();
            }
            app.fetch_artwork(String::new());
        }
        app
    }
    fn playback_position(&self) -> f64 {
        if self.smb_connecting && self.current.as_ref().is_some_and(|station| youtube::is_url(&station.stream_url)) {
            return self.current.as_ref().and_then(|station| self.library.local_positions.get(&station.stream_url)).copied().unwrap_or(0.0);
        }
        self.player.position() + self.youtube_stream.as_ref().map_or(0.0, |stream| stream.start_time)
    }
    fn restore_youtube_pause(&mut self) {
        if self.youtube_resume_paused && self.player.active() && !self.player.paused {
            self.player.toggle();
        }
        self.youtube_resume_paused = false;
    }
    fn snapshot_position(&mut self) {
        if self.player.active()
            && !self.player.finished()
            && let Some(station) = self.current.as_ref().filter(|s| s.is_file() || (self.current_has_video && self.duration > 0.))
        {
            let position = self.playback_position();
            if position.is_finite() && position >= 0. {
                self.library
                    .local_positions
                    .insert(station.stream_url.clone(), position);
            }
        }
    }
    fn save(&mut self) {
        self.snapshot_position();
        self.library.kaleido = self.kaleido.settings();
        self.library.selected_tab = self.tab as usize;
        self.library.folder_index = self.folder_index;
        self.library.compact_library = self.compact_library;
        self.library.visualizer_open = self.visualizer_open;
        self.library.last_remote = self.remote.clone();
        self.library.last_station = self.current.clone();
        let serialized = serde_json::to_string(&self.library).unwrap_or_default();
        if serialized == self.last_saved {
            return;
        }
        if self.writable
            && let Err(e) = self.library.save(&self.path)
        {
            self.status = format!("Could not save library: {e}");
        } else if self.writable {
            self.last_saved = serialized;
        }
    }
    fn play(
        &mut self,
        station: Station,
        queue: Vec<Station>,
        auth: Option<(String, String)>,
        offset: f64,
    ) {
        self.queue = queue;
        self.queue_from_discover = false;
        self.play_queued(station, auth, offset);
    }
    fn play_queued(&mut self, station: Station, auth: Option<(String, String)>, offset: f64) {
        if self.music_radio.as_ref().is_some_and(|radio| !radio.ids.contains(&station.id)) { self.music_radio = None; }
        self.snapshot_position();
        self.compact_library = false;
        self.retry_at = None;
        self.retries = 0;
        self.source_has_video = video::is_video_url(&station.stream_url);
        self.current_has_video = self.source_has_video
            && (video::is_file_video(&station) || !self.library.audio_only_for(&station));
        self.video_texture = None;
        self.current = Some(station.clone());
        self.auth = auth;
        self.duration = 0.0;
        self.youtube_resume_paused = false;
        self.seek_position = None;
        self.video_toggle_at = None;
        self.title = station.name.clone();
        self.album = None;
        self.artwork = None;
        self.artwork_color = theme::DEFAULT_ARTWORK_COLOR;
        self.metrics = audio::Metrics::default();
        // A video clicked again — after navigating away, or after the
        // app was closed and reopened — resumes from its saved scrub
        // location instead of restarting at 0. Audio and radio are
        // untouched: they have no video position to resume from, and
        // an explicit non-zero offset (a seek, a mode change) wins.
        let offset = video_resume_position(&self.library.local_positions, &station, self.current_has_video, offset);
        self.start_playback(offset);
        self.save();
        self.fetch_artwork(String::new());
    }
    fn seek_playback(&mut self, position: f64) {
        if let Some(station) = &self.current {
            self.library.local_positions.insert(station.stream_url.clone(), position);
        }
        if let Some(stream) = &self.youtube_stream {
            if stream.can_seek(position) {
                let local_position = (position - stream.start_time).max(0.0);
                if let Some(video) = &self.player.video { video.seek(local_position); }
                else {
                    let address = stream.address.clone();
                    let paused = self.player.paused;
                    self.start_player(&address, local_position);
                    self.youtube_resume_paused = paused;
                    self.restore_youtube_pause();
                }
            } else {
                let paused = self.player.paused;
                // A restart seek keeps the last frame on screen instead
                // of going black while the new stream is extracted.
                self.keep_video_frame = true;
                self.start_playback(position);
                self.youtube_resume_paused = paused;
                self.status = format!("Seeking to {}…", video::format_time(position));
            }
        } else if self.smb_connecting && self.current.as_ref().is_some_and(|s| youtube::is_url(&s.stream_url)) {
            let paused = self.youtube_resume_paused;
            self.keep_video_frame = true;
            self.start_playback(position);
            self.youtube_resume_paused = paused;
            self.status = format!("Seeking to {}…", video::format_time(position));
        } else if let Some(video) = &self.player.video {
            video.seek(position);
        } else if self.player.active() {
            self.start_playback(position);
        }
    }
    fn cancel_transition(&mut self) {
        self.transition_generation += 1;
        self.transition_next = None;
        self.transition_settings = None;
        self.player.clear_prepared();
    }
    fn clear_youtube_preload(&mut self) {
        if let Some(preload) = self.youtube_preload.take() {
            self.player.clear_prepared();
            // The audio callback may have just promoted this decoder. Keep its
            // media alive until the queued Transition event can transfer ownership.
            if preload.key.audio && preload.ready.is_some() && self.player.active() {
                self.retired_youtube_preloads.push((Instant::now() + Duration::from_secs(1), preload));
            } else { preload.cancel.store(true, std::sync::atomic::Ordering::Relaxed); }
            self.preload_epoch += 1;
        }
    }
    fn clear_retired_preloads(&mut self) {
        for (_, preload) in self.retired_youtube_preloads.drain(..) {
            preload.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    fn preload_key(&self) -> Option<PreloadKey> {
        Some(PreloadKey {
            owner: self.current.as_ref()?.id,
            audio: self.current.as_ref().map(|station| self.library.audio_only_for(station)).unwrap_or(false),
            quality: self.library.youtube_quality,
            frame_rate: self.library.youtube_frame_rate,
            shuffle: self.library.shuffle,
            repeat: self.library.repeat,
            radio: self.music_radio.as_ref().map(|radio| radio.epoch),
            transitions: (self.library.gapless_playback, self.library.crossfade_playback, self.library.crossfade_seconds.to_bits()),
        })
    }
    fn prepare_youtube_preload(&mut self) {
        if !self.player.active() || self.smb_connecting { return; }
        let Some(key) = self.preload_key() else { return; };
        if self.youtube_preload.as_ref().is_some_and(|preload| preload.key != key) { self.clear_youtube_preload(); }
        let Some(index) = self.queue.iter().position(|station| station.id == key.owner) else { return; };
        let preferred = self.youtube_preload.as_ref().map(|preload| preload.target);
        let Some((next, reset_history)) = youtube::preload::next_index(&self.queue, index, self.library.repeat, self.library.shuffle, &self.shuffle_history, self.music_radio.is_some(), preferred) else { self.clear_youtube_preload(); return; };
        let station = self.queue[next].clone();
        if !youtube::is_url(&station.stream_url) { self.clear_youtube_preload(); return; }
        if self.youtube_preload.as_ref().is_some_and(|preload| preload.target == station.id) { return; }
        self.clear_youtube_preload();
        self.preload_epoch += 1;
        let epoch = self.preload_epoch;
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.youtube_preload = Some(YouTubePreload { epoch, key: key.clone(), target: station.id, reset_history, cancel: cancel.clone(), ready: None });
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let stream = if key.audio { youtube::streaming::start_audio(&station.stream_url, key.quality, key.frame_rate, 0.0, cancel.clone()) }
                    else { youtube::streaming::start(&station.stream_url, key.quality, key.frame_rate, 0.0, cancel.clone()) };
                if cancel.load(std::sync::atomic::Ordering::Relaxed) { return Err("Preload cancelled.".into()); }
                let media = match stream {
                    Ok(stream) => YouTubeMedia::Stream(stream),
                    Err(_) => YouTubeMedia::Buffered(if key.audio { youtube::buffer_audio(&station.stream_url, key.quality, key.frame_rate, Some(&cancel))? }
                        else { youtube::buffer(&station.stream_url, key.quality, key.frame_rate, Some(&cancel))? }),
                };
                let prepared = if key.audio {
                    audio::Prepared::open(station.id, &media.address()?, None, 0.0).and_then(|prepared| prepared.warm(&cancel)).ok()
                } else { None };
                Ok(YouTubeReady { media, prepared })
            })();
            let _ = tx.send(TaskResult::YouTubePreloaded(epoch, result));
        });
    }
    fn accept_youtube_media(&mut self, media: YouTubeMedia, offset: f64, start: bool) {
        match media {
            YouTubeMedia::Stream(stream) => {
                let address = stream.address.clone();
                let local_offset = (offset - stream.start_time).max(0.0);
                if let Some(duration) = stream.duration { self.duration = duration; }
                if let Some(artist) = &stream.artist {
                    if let Some(current) = self.current.as_mut().filter(|station| station.artist.is_none()) {
                        current.artist = Some(artist.clone());
                        for station in &mut self.queue { if station.stream_url == current.stream_url { station.artist = Some(artist.clone()); } }
                    }
                }
                self.youtube_buffer = None;
                self.youtube_stream = Some(stream);
                if start { self.start_player(&address, local_offset); }
            }
            YouTubeMedia::Buffered(file) => {
                if let Ok(address) = url::Url::from_file_path(file.path()) {
                    self.youtube_stream = None;
                    self.youtube_buffer = Some(file);
                    if start { self.start_player(address.as_str(), offset); }
                }
            }
        }
        self.video_seekable = self.duration > 0.0;
        if start { self.restore_youtube_pause(); }
        self.update_media();
    }
    fn start_playback(&mut self, offset: f64) {
        let warmed = self.youtube_preload.take().and_then(|mut preload| {
            let usable = offset <= 0.001 && self.current.as_ref().is_some_and(|station| station.id == preload.target)
                && preload.key.audio == self.current.as_ref().map(|station| self.library.audio_only_for(station)).unwrap_or(false) && preload.key.quality == self.library.youtube_quality && preload.key.frame_rate == self.library.youtube_frame_rate;
            if usable {
                if let Some(ready) = preload.ready.take() { return Some((ready, preload.cancel)); }
            }
            preload.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            None
        });
        self.preload_epoch += 1;
        self.player.stop();
        self.clear_retired_preloads();
        self.youtube_buffer = None;
        self.youtube_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.youtube_stream = None;
        self.youtube_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.cancel_transition();
        self.playback_generation += 1;
        self.smb_connecting = false;
        if !std::mem::take(&mut self.keep_video_frame) { self.video_texture = None; }
        self.video_seekable = false;
        let Some(station) = &self.current else { return; };
        let address = station.stream_url.clone();
        self.source_has_video |= video::is_video_url(&address);
        self.current_has_video = self.source_has_video
            && (video::is_file_video(station) || !self.library.audio_only_for(station));
        if youtube::is_url(&address) {
            if let Some((ready, cancel)) = warmed {
                self.youtube_cancel = cancel;
                self.accept_youtube_media(ready.media, offset, true);
                return;
            }
            self.library.local_positions.insert(address.clone(), offset.max(0.0));
            self.video_seekable = self.duration > 0.0;
            self.smb_connecting = true;
            self.status = if self.library.audio_only_for(station) { "Starting YouTube audio…".into() }
                else { format!("Starting YouTube stream · {}…", self.library.youtube_quality.label()) };
            let quality = self.library.youtube_quality;
            let frame_rate = self.library.youtube_frame_rate;
            let audio_only = self.library.audio_only_for(station);
            let generation = self.playback_generation;
            let tx = self.tx.clone();
            let cancelled = self.youtube_cancel.clone();
            std::thread::spawn(move || {
                let stream = if audio_only {
                    youtube::streaming::start_audio(&address, quality, frame_rate, offset, cancelled.clone())
                } else { youtube::streaming::start(&address, quality, frame_rate, offset, cancelled.clone()) };
                match stream {
                    Ok(stream) => { let _ = tx.send(TaskResult::YouTubeStreaming(generation, offset, Ok(stream))); }
                    Err(_) if !cancelled.load(std::sync::atomic::Ordering::Relaxed) => {
                        let _ = tx.send(TaskResult::YouTubeFallback(generation));
                        let buffered = if audio_only { youtube::buffer_audio(&address, quality, frame_rate, Some(&cancelled)) }
                            else { youtube::buffer(&address, quality, frame_rate, Some(&cancelled)) };
                        let _ = tx.send(TaskResult::YouTubeBuffered(generation, offset, buffered));
                    }
                    Err(_) => {}
                }
            });
            self.update_media();
        } else if address.starts_with("smb:") || address.starts_with("sftp:") {
            self.smb_connecting = true;
            if self.auth.is_none() {
                if let Some(connection) = self.library.connections.iter().find(|connection| smb::same_share(&connection.url, &address) || sftp::same_server(&connection.url, &address)) {
                    self.auth = Some((connection.username.clone(), credentials::load(connection)));
                }
            }
            self.player.stop();
            self.status = "Connecting…".into();
            let generation = self.playback_generation;
            let auth = self.auth.clone();
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let result = if address.starts_with("sftp:") { sftp::resolve(&address, auth.as_ref()) } else { smb::resolve(&address, auth.as_ref()) };
                let _ = tx.send(TaskResult::SMBPlayback(generation, offset, result));
            });
            self.update_media();
        } else {
            self.start_player(&address, offset);
        }
    }
    fn start_player(&mut self, address: &str, offset: f64) {
        // A new player is a fresh decoder source, unstarved until its own
        // decoder reports otherwise.
        self.audio_starved = false;
        let auth = self.auth.as_ref().map(|(user, password)| (user.as_str(), password.as_str()));
        let result = if self.current.as_ref().is_some_and(|station| self.library.audio_only_for(station) && !video::is_file_video(station)) && self.source_has_video {
            self.player.play_audio(address, offset, self.library.volume, auth)
        } else if self.current_has_video {
            let volume = if self.video_muted { 0.0 } else { self.library.volume };
            self.player.play_video(address, offset, volume, auth)
        } else {
            self.player.play(address, offset, self.library.volume, auth)
        };
        match result {
            Ok(()) => self.status = "Connecting…".into(),
            Err(error) => self.status = error,
        }
        self.update_media();
    }
    fn fetch_artwork(&mut self, title: String) {
        let title = if self
            .current
            .as_ref()
            .is_some_and(|station| network::should_lookup_artwork(&title, station))
        {
            title
        } else {
            String::new()
        };
        self.artwork_generation += 1;
        let id = self.artwork_generation;
        let fallback = self.current.as_ref().and_then(|s| s.favicon_url.clone());
        let file_url = self.current.as_ref().filter(|s| !youtube::is_url(&s.stream_url) && (s.is_file() || video::is_video_url(&s.stream_url))).map(|s| s.stream_url.clone());
        let auth = self.auth.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = match file_url {
                Some(url) => TaskResult::FileMetadata(id, file_metadata::read(&url, auth.as_ref())),
                None => TaskResult::Artwork(id, network::artwork(&title, fallback.as_deref())),
            };
            let _ = tx.send(result);
        });
    }
    fn search(&mut self) {
        self.generation += 1;
        let id = self.generation;
        self.searching = true;
        let query = self.query.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(TaskResult::Search(id, network::search(&query)));
        });
    }
    fn browse(&mut self, address: String) {
        self.browse_directory(address, false);
    }
    fn browse_directory(&mut self, address: String, background: bool) {
        if !network::valid_remote_url(&self.remote.url) {
            self.status = "Enter an HTTP(S), smb://server/share, or sftp://server/path URL without embedded credentials.".into();
            return;
        }
        self.remote_metadata_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.remote_metadata_pending = 0;
        self.remote_generation += 1;
        let id = self.remote_generation;
        self.browsing = true;
        self.remote_error = None;
        self.remote_background_refresh = background;
        self.remote_address = address.clone();
        if !background {
            self.remote_filter.clear();
            self.entries.clear();
        }
        let connection = self.remote.clone();
        let password = self.password.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(TaskResult::Browse(
                id,
                network::browse(&connection, &password, &address),
            ));
        });
    }
    fn add_folder(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.scan_folder(path);
        }
    }
    fn scan_folder(&mut self, path: PathBuf) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        let old = self
            .library
            .folders
            .iter()
            .find(|f| f.path == path)
            .map_or(vec![], |f| f.tracks.clone());
        self.folder_generation += 1;
        let id = self.folder_generation;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(TaskResult::Folder(id, Folder::scan(path, &old)));
        });
    }
    fn open_local_directory(&mut self, path: PathBuf) {
        self.read_local_directory(path, false);
    }
    fn read_local_directory(&mut self, path: PathBuf, background: bool) {
        let Some(root) = self.library.folders.get(self.folder_index) else {
            return;
        };
        if !path.starts_with(&root.path) {
            return;
        }
        let previous = root.tracks.clone();
        self.local_directory_generation += 1;
        let generation = self.local_directory_generation;
        self.local_directory_path = Some(path.clone());
        if !background {
            self.local_directory = None;
            self.filter.clear();
        }
        self.local_directory_error = None;
        self.local_directory_loading = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(TaskResult::LocalDirectory(
                generation,
                LocalDirectory::read(path, &previous),
            ));
        });
    }
    fn reset_local_directory(&mut self) {
        self.local_directory_generation += 1;
        self.local_directory_path = None;
        self.local_directory = None;
        self.local_directory_error = None;
        self.local_directory_loading = false;
        self.library.local_directory = None;
    }
    fn next_shuffle_index(&mut self, current: usize) -> Option<usize> {
        let mut candidates = model::shuffle_candidates(&self.queue, current, &self.shuffle_history);
        if candidates.is_empty() && self.library.repeat == Repeat::All {
            self.shuffle_history.clear();
            candidates = model::shuffle_candidates(&self.queue, current, &self.shuffle_history);
        }
        use rand::seq::IndexedRandom;
        candidates.choose(&mut rand::rng()).copied()
    }
    fn advance(&mut self, previous: bool, ended: bool) {
        // From the Favorites tab, Previous/Next stay inside the highlighted
        // category, even when the play queue predates a category switch or
        // came from elsewhere (cold-start Play queues the whole library).
        // Discover results and music radio sessions keep their own queues.
        if self.tab == Tab::Favorites && !self.queue_from_discover && self.music_radio.is_none() {
            if let Some(current) = self.current.clone() {
                if self.library.stations.iter().any(|s| s.id == current.id) {
                    let list: Vec<Station> = self.library.stations.iter()
                        .filter(|s| s.favorite_category() == self.favorite_category)
                        .cloned()
                        .collect();
                    if list.is_empty() {
                        self.status = "No favorites in this category.".into();
                        return;
                    }
                    match list.iter().position(|s| s.id == current.id) {
                        None => {
                            let station = if previous { list.last().unwrap().clone() } else { list.first().unwrap().clone() };
                            self.play(station, list, self.auth.clone(), 0.0);
                            return;
                        }
                        Some(_) => self.queue = list,
                    }
                }
            }
        }
        if self.queue.is_empty() {
            return;
        }
        let index = self
            .current
            .as_ref()
            .and_then(|s| self.queue.iter().position(|q| q.id == s.id))
            .unwrap_or(0);
        if self.music_radio.is_some() {
            if previous || index + 1 < self.queue.len() {
                let next = if previous { index.saturating_sub(1) } else { index + 1 };
                self.play(self.queue[next].clone(), self.queue.clone(), None, 0.0);
            } else if self.music_radio.as_ref().is_some_and(|radio| radio.page.continuation.is_some() || radio.loading) {
                self.player.stop();
                if let Some(radio) = self.music_radio.as_mut() { radio.waiting = true; }
                self.status = "Loading more radio songs…".into();
                self.pump_radio();
            } else {
                self.stop();
                self.status = "Radio queue finished.".into();
            }
            return;
        }
        let is_file = self.current.as_ref().is_some_and(Station::is_file);
        let next = if is_file && ended && self.library.repeat == Repeat::One {
            index
        } else if is_file && self.library.shuffle && self.queue.len() > 1 {
            if previous {
                self.shuffle_history
                    .pop()
                    .and_then(|id| self.queue.iter().position(|s| s.id == id))
                    .unwrap_or(index)
            } else {
                let planned = self.youtube_preload.as_ref().filter(|preload| self.preload_key().as_ref() == Some(&preload.key) && (ended || preload.target != self.queue[index].id))
                    .and_then(|preload| self.queue.iter().position(|station| station.id == preload.target).map(|next| (next, preload.reset_history)));
                let next = if let Some((next, reset)) = planned {
                    if reset { self.shuffle_history.clear(); }
                    Some(next)
                } else { self.next_shuffle_index(index) };
                let Some(next) = next else {
                    self.player.stop();
                    self.status = "Queue finished.".into();
                    return;
                };
                self.shuffle_history.push(self.queue[index].id);
                next
            }
        } else if previous {
            if index == 0 {
                if is_file && self.library.repeat != Repeat::All {
                    0
                } else {
                    self.queue.len() - 1
                }
            } else {
                index - 1
            }
        } else if index + 1 == self.queue.len() {
            if is_file && self.library.repeat != Repeat::All {
                self.player.stop();
                self.status = "Queue finished.".into();
                return;
            }
            0
        } else {
            index + 1
        };
        self.play_queued(
            self.queue[next].clone(),
            self.auth.clone(),
            0.0,
        );
    }
    fn poll_folder_changes(&mut self) {
        if self.local_watch_path != self.local_directory_path {
            self.local_watcher = None;
            self.local_watch_path = self.local_directory_path.clone();
            self.local_change_at = None;
            while self.local_changes_rx.try_recv().is_ok() {}
            if let Some(path) = &self.local_watch_path {
                let tx = self.local_changes_tx.clone();
                let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if event.as_ref().map_or(true, |event| !matches!(event.kind, notify::EventKind::Access(_))) {
                        let _ = tx.send(());
                    }
                });
                match watcher.and_then(|mut watcher| {
                    watcher.watch(path, notify::RecursiveMode::NonRecursive)?;
                    Ok(watcher)
                }) {
                    Ok(watcher) => self.local_watcher = Some(watcher),
                    Err(error) => self.local_directory_error = Some(format!("Cannot watch folder: {error}")),
                }
            }
        }
        while self.local_changes_rx.try_recv().is_ok() {
            self.local_change_at = Some(Instant::now());
        }
        if !self.local_directory_loading && self.local_change_at.is_some_and(|at| at.elapsed() >= Duration::from_millis(600)) {
            self.local_change_at = None;
            if let Some(path) = self.local_directory_path.clone() {
                self.read_local_directory(path, true);
            }
        }

    }
    fn poll(&mut self, ctx: &egui::Context) {
        self.poll_folder_changes();
        self.card_artwork.poll(ctx);
        for command in self.media.commands() {
            match command.resolve(self.player.active() || self.retry_at.is_some()) {
                media::Command::Play => self.resume(),
                media::Command::Stop => self.stop(),
                media::Command::Toggle => unreachable!("toggle is resolved above"),
                media::Command::Next => self.advance(false, false),
                media::Command::Previous => self.advance(true, false),
                media::Command::OutputChanged => {
                    if self.player.active() {
                        let offset = self.playback_position();
                        self.snapshot_position();
                        self.player.reset_output();
                        self.start_playback(offset);
                    }
                }
                media::Command::Disconnected => {
                    self.stop();
                    self.player.reset_output();
                    self.status = "Headphones disconnected. Playback stopped.".into();
                }
            }
        }
        while let Ok(result) = self.rx.try_recv() {
            match result {
                TaskResult::YouTubeDownloadProgress(progress) => {
                    self.youtube_download_progress = progress;
                }
                TaskResult::YouTubeDownloaded(result) => {
                    self.youtube_download = None;
                    self.status = match result {
                        Ok(()) => "Download Saved.".into(),
                        Err(error) => format!("Download failed: {error}"),
                    };
                }
                TaskResult::YouTubePreloaded(epoch, result) => {
                    if let Some(preload) = self.youtube_preload.as_mut().filter(|preload| preload.epoch == epoch) {
                        if let Ok(mut ready) = result {
                            if let Some(prepared) = ready.prepared.take() { self.player.queue_prepared(prepared); }
                            preload.ready = Some(ready);
                        }
                    }
                }
                TaskResult::Music(id, result) if id == self.music_browser.generation => {
                    self.music_browser.loading = false;
                    match result {
                        Ok(page) => self.apply_music_page(page),
                        Err(error) => self.music_browser.error = Some(error),
                    }
                }
                TaskResult::RadioMore(epoch, result) => self.apply_radio_more(epoch, result),
                TaskResult::SMBPlayback(generation, offset, result) if generation == self.playback_generation => {
                    self.smb_connecting = false;
                    match result {
                        Ok(address) => { self.start_player(&address, offset); self.restore_youtube_pause(); },
                        Err(error) => self.status = error,
                    }
                }
                TaskResult::YouTubeFallback(id) if id == self.playback_generation => {
                    self.status = if self.current.as_ref().is_some_and(|station| self.library.audio_only_for(station)) { "Streaming unavailable; downloading YouTube audio…".into() }
                        else { "Direct streaming unavailable; downloading YouTube video…".into() };
                }
                TaskResult::YouTubeStreaming(id, offset, result) if id == self.playback_generation => {
                    self.smb_connecting = false;
                    match result {
                        Ok(stream) => self.accept_youtube_media(YouTubeMedia::Stream(stream), offset, true),
                        Err(error) => self.status = error,
                    }
                }
                TaskResult::YouTubeBuffered(id, offset, result) if id == self.playback_generation => {
                    self.smb_connecting = false;
                    match result {
                        Ok(file) => {
                            match url::Url::from_file_path(file.path()) {
                                Ok(address) => {
                                    self.youtube_buffer = Some(file);
                                    self.start_player(address.as_str(), offset);
                                    self.restore_youtube_pause();
                                }
                                Err(_) => self.status = "Invalid YouTube buffer path.".into(),
                            }
                        }
                        Err(error) => self.status = error,
                    }
                }
                TaskResult::YouTubeSearchItem(music, id, station) => {
                    let state = if music { &mut self.youtube_music_search } else { &mut self.youtube_search };
                    if id == state.generation { state.results.push(station); }
                }
                TaskResult::YouTubeSearch(music, id, result) => {
                    let state = if music { &mut self.youtube_music_search } else { &mut self.youtube_search };
                    if id == state.generation {
                        state.searching = false;
                        match result {
                            Ok(()) => self.status = format!("Found {} {} results.", state.results.len(), if music { "YouTube Music" } else { "YouTube" }),
                            Err(error) => state.error = Some(error),
                        }
                    }
                }
                TaskResult::YouTubeChannelItem(id, channel) => {
                    let state = &mut self.youtube_search;
                    if id == state.generation && !state.channels.iter().any(|c| c.id == channel.id) {
                        state.channels.push(channel);
                    }
                }
                TaskResult::YouTubeChannels(id, result) => {
                    let state = &mut self.youtube_search;
                    if id == state.generation {
                        state.searching = false;
                        match result {
                            Ok(()) => self.status = format!("Found {} channels.", state.channels.len()),
                            Err(error) => state.error = Some(error),
                        }
                    }
                }
                TaskResult::YouTubeChannelVideos(id, result) => {
                    let state = &mut self.youtube_search;
                    if id == state.generation {
                        state.searching = false;
                        match result {
                            Ok(videos) => {
                                let name = state
                                    .active_channel
                                    .as_ref()
                                    .map(|c| c.name.clone())
                                    .unwrap_or_else(|| "Channel".into());
                                self.status = format!("Found {} videos from {}.", videos.len(), name);
                                state.channel_results = videos;
                            }
                            Err(error) => state.error = Some(error),
                        }
                    }
                }
                TaskResult::Search(id, result) if id == self.generation => {
                    self.searching = false;
                    match result {
                        Ok(stations) => {
                            self.status = format!("Found {} stations.", stations.len());
                            self.results = stations;
                        }
                        Err(e) => self.status = e,
                    }
                }
                TaskResult::Browse(id, result) if id == self.remote_generation => {
                    self.browsing = false;
                    match result {
                        Ok(entries) => {
                            self.remote_error = None;
                            if !self.remote_background_refresh {
                                self.status = format!("{} remote entries.", entries.len());
                            }
                            self.entries = entries;
                            self.load_remote_entry_metadata();
                        }
                        Err(e) => {
                            self.remote_error = Some(e.clone());
                            if !self.remote_background_refresh { self.status = e; }
                        }
                    }
                }
                TaskResult::RemoteEntryMetadata(id, url, metadata) if id == self.remote_generation => {
                    self.remote_metadata_pending = self.remote_metadata_pending.saturating_sub(1);
                    for track in &mut self.queue {
                        if track.stream_url == url {
                            if let Some(title) = &metadata.title { track.name = title.clone(); }
                            track.track_number = metadata.track_number;
                            track.disc_number = metadata.disc_number;
                        }
                    }
                    if let Some(current) = self.current.as_mut().filter(|s| s.stream_url == url) {
                        current.track_number = metadata.track_number;
                        current.disc_number = metadata.disc_number;
                    }
                    self.remote_metadata_cache.insert((self.remote.username.clone(), url), metadata);
                }
                TaskResult::PreparedTransition(generation, id, result)
                    if generation == self.transition_generation && self.transition_next.as_ref().is_some_and(|s| s.id == id) => {
                    if let Ok(prepared) = result {
                        self.player.queue_prepared(prepared);
                    }
                }
                TaskResult::FileMetadata(id, Ok(metadata)) if id == self.artwork_generation => {
                    if let Some(current) = &mut self.current {
                        current.track_number = metadata.track_number;
                        current.disc_number = metadata.disc_number;
                    }
                    if let Some(title) = metadata.title {
                        self.title = title;
                    }
                    self.album = metadata.album;
                    self.update_media();
                    if let Some(bytes) = metadata.artwork {
                        let _ = self.tx.send(TaskResult::Artwork(id, Ok(bytes)));
                    }
                }
                TaskResult::Artwork(id, Ok(bytes)) if id == self.artwork_generation => {
                    if let Ok(image) = image::load_from_memory(&bytes) {
                        self.artwork_color = theme::artwork_color(&image.to_rgba8());
                        let image = image.thumbnail(1024, 1024).to_rgba8();
                        let size = [image.width() as usize, image.height() as usize];
                        self.artwork = Some(ctx.load_texture(
                            "now-playing-artwork",
                            egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                            egui::TextureOptions::LINEAR,
                        ));
                        if !self.current_has_video && self.current.as_ref().is_some_and(|s| !s.is_file()) {
                            if let Some(track) = self.history.first_mut().filter(|track| track.title == self.title) {
                                let thumbnail = image::imageops::thumbnail(&image, 72, 72);
                                track.artwork = Some(ctx.load_texture(
                                    format!("recent-track-artwork-{id}"),
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [thumbnail.width() as usize, thumbnail.height() as usize],
                                        thumbnail.as_raw(),
                                    ),
                                    egui::TextureOptions::LINEAR,
                                ));
                            }
                        }
                    }
                }
                TaskResult::LocalDirectory(id, result) if id == self.local_directory_generation => {
                    self.local_directory_loading = false;
                    match result {
                        Ok(directory) => {
                            if let Some(root) = self.library.folders.get_mut(self.folder_index) {
                                root.tracks.retain(|track| url::Url::parse(&track.stream_url).ok()
                                    .and_then(|url| url.to_file_path().ok())
                                    .is_none_or(|path| path.parent() != Some(directory.path.as_path())));
                                root.tracks.extend(directory.tracks.clone());
                            }
                            self.library.local_directory = Some(directory.path.clone());
                            self.local_directory = Some(directory);
                            self.save();
                        }
                        Err(error) => self.local_directory_error = Some(error),
                    }
                }
                TaskResult::Folder(id, result) if id == self.folder_generation => {
                    self.scanning = false;
                    match result {
                        Ok(folder) => {
                            self.status = format!("Loaded {} tracks.", folder.tracks.len());
                            if let Some(i) = self
                                .library
                                .folders
                                .iter()
                                .position(|f| f.path == folder.path)
                            {
                                self.library.folders[i] = folder;
                                self.folder_index = i;
                            } else {
                                self.library.folders.push(folder);
                                self.folder_index = self.library.folders.len() - 1;
                            }
                            self.tab = Tab::Local;
                            let directory = self.local_directory_path.clone()
                                .filter(|path| path.starts_with(&self.library.folders[self.folder_index].path) && path.is_dir())
                                .unwrap_or_else(|| self.library.folders[self.folder_index].path.clone());
                            self.open_local_directory(directory);
                            self.save();
                        }
                        Err(e) => self.status = e,
                    }
                }
                _ => {}
            }
        }
        while let Ok(event) = self.player.events.try_recv() {
            match event {
                audio::Event::Metrics(metrics) => {
                    self.metrics = metrics;
                    if !self.player.paused {
                        self.status = if self.audio_starved { "Buffering…" } else { "Playing" }.into();
                    }
                }
                audio::Event::Starved(starved) => {
                    self.audio_starved = starved;
                    if self.player.video.is_none() && self.player.active() && !self.player.paused {
                        self.status = if starved { "Buffering…" } else { "Playing" }.into();
                    }
                }
                audio::Event::Transition(id, duration) => {
                    // A new track is a fresh decoder source, unstarved
                    // until its own decoder reports otherwise.
                    self.audio_starved = false;
                    let mut promoted = self.youtube_preload.take();
                    if promoted.as_ref().is_some_and(|preload| preload.target != id) {
                        self.youtube_preload = promoted.take();
                    }
                    if promoted.is_none() {
                        if let Some(index) = self.retired_youtube_preloads.iter().position(|(_, preload)| preload.target == id) {
                            promoted = Some(self.retired_youtube_preloads.remove(index).1);
                        }
                    }
                    if promoted.as_ref().is_some_and(|preload| preload.reset_history) { self.shuffle_history.clear(); }
                    if let Some(station) = self.queue.iter().find(|s| s.id == id).cloned() {
                        if let Some(previous) = &self.current {
                            self.library.local_positions.remove(&previous.stream_url);
                            if self.library.shuffle && self.library.repeat != Repeat::One && self.music_radio.is_none() { self.shuffle_history.push(previous.id); }
                        }
                        self.current = Some(station.clone());
                        self.title = station.name;
                        self.album = None;
                        self.duration = duration;
                        if let Some(mut preload) = promoted {
                            if let Some(ready) = preload.ready.take() {
                                self.youtube_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                self.youtube_cancel = preload.cancel;
                                self.source_has_video = true;
                                self.current_has_video = false;
                                self.accept_youtube_media(ready.media, 0.0, false);
                            }
                        }
                        self.seek_position = None;
                        self.artwork = None;
                        self.artwork_color = theme::DEFAULT_ARTWORK_COLOR;
                        self.transition_next = None;
                        self.transition_settings = None;
                        self.status = "Playing".into();
                        self.fetch_artwork(String::new());
                        self.update_media();
                        self.save();
                    }
                }
                audio::Event::Metadata(title) => {
                    if self.current.as_ref().is_some_and(Station::is_file) {
                        continue;
                    }
                    if self.title != title {
                        self.title = if title.trim().is_empty() {
                            self.current
                                .as_ref()
                                .map(|s| s.name.clone())
                                .unwrap_or_default()
                        } else {
                            title.clone()
                        };
                        if !title.trim().is_empty() {
                            self.history.insert(0, RecentTrack { title: title.clone(), artwork: None });
                            self.history.truncate(40);
                        }
                        self.fetch_artwork(title);
                    }
                }
                audio::Event::Duration(duration) => self.duration = self.youtube_stream.as_ref().and_then(|stream| stream.duration).unwrap_or(duration),
                audio::Event::Error(e) => self.status = e,
                audio::Event::VideoDetected => {
                    self.source_has_video = true;
                    if self.current.as_ref().is_some_and(|station| self.library.audio_only_for(station) && !video::is_file_video(station)) { continue; }
                    self.current_has_video = true;
                    let offset = self.playback_position();
                    self.start_playback(offset);
                    break;
                }
            }
        }
        if let Some(error) = self.youtube_stream.as_ref().and_then(|stream| stream.error()) {
            self.stop();
            self.status = error;
        }
        if let Some(video) = self.player.video.as_mut() {
            let state = video.state();
            let full_duration = self.youtube_stream.as_ref().and_then(|stream| stream.duration);
            self.duration = full_duration.unwrap_or(state.duration);
            self.video_seekable = (full_duration.is_some() || state.seekable) && self.duration > 0.;
            // AVPlayer reports on its own timeline; the YouTube relay
            // starts that timeline at the stream's start offset.
            self.video_buffered = state.buffered + self.youtube_stream.as_ref().map_or(0.0, |stream| stream.start_time);
            if state.status == -1 {
                let error = video.error();
                self.stop();
                self.status = format!("Video unavailable: {error}");
            } else if state.status == 3 {
                self.current_has_video = false;
                self.video_texture = None;
                self.video_seekable = false;
                self.audio_starved = false;
                if let Some(station) = &self.current {
                    if let Err(error) = self.player.play_audio(&station.stream_url, state.position, self.library.volume,
                        self.auth.as_ref().map(|(user, password)| (user.as_str(), password.as_str()))) {
                        self.status = error;
                    }
                }
            } else {
                if state.status == 1 {
                    self.status = if state.stalled { "Buffering…" } else if self.player.paused { "Paused" } else { "Playing" }.into();
                }
                if let Some(frame) = video.frame() {
                    if let Some(texture) = self.video_texture.as_mut() {
                        texture.set(frame, egui::TextureOptions::LINEAR);
                    } else {
                        self.video_texture = Some(ctx.load_texture("video-frame", frame, egui::TextureOptions::LINEAR));
                    }
                }
            }
        }
        self.retired_youtube_preloads.retain(|(deadline, preload)| {
            if Instant::now() >= *deadline { preload.cancel.store(true, std::sync::atomic::Ordering::Relaxed); false }
            else { true }
        });
        self.prepare_transition();
        self.pump_radio();
        if self.player.finished() {
            if self.current.as_ref().is_some_and(Station::is_file) {
                if let Some(station) = &self.current {
                    self.library.local_positions.remove(&station.stream_url);
                }
                self.advance(false, true);
            } else {
                // A finished video starts from the beginning next time,
                // instead of resuming from its final seconds.
                if self.current_has_video {
                    if let Some(station) = &self.current {
                        self.library.local_positions.remove(&station.stream_url);
                    }
                }
                self.player.stop();
                if self.current_has_video && self.duration > 0. {
                    self.status = "Video finished.".into();
                } else if self.retries < 3 {
                    self.retries += 1;
                    self.retry_at =
                        Some(Instant::now() + Duration::from_secs(u64::from(self.retries) * 2));
                    self.status = format!("Stream disconnected. Retry {}/3…", self.retries);
                } else {
                    self.status = "Stream unavailable. Press Play to reconnect.".into();
                }
            }
        }
        if self.retry_at.is_some_and(|at| Instant::now() >= at) {
            self.retry_at = None;
            self.start_playback(0.0);
        }
        if self.player.paused {
            self.metrics = audio::Metrics::default();
        }
    }
    fn list(
        &mut self,
        ui: &mut egui::Ui,
        stations: Vec<Station>,
        favorite_actions: bool,
        folders: &[PathBuf],
    ) {
        let mut open_folder = None;
        let mut action = None;
        let mut remove = None;
        let mut edit = None;
        let mut add = None;
        let filter = if self.tab == Tab::Discover {
            String::new()
        } else {
            self.filter.trim().to_lowercase()
        };
        let visible: Vec<_> = stations
            .iter()
            .filter(|s| filter.is_empty() || s.name.to_lowercase().contains(&filter))
            .collect();
        let folder_count = folders
            .iter()
            .filter(|path| {
                filter.is_empty()
                    || path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&filter)
            })
            .count();
        ui.label(
            egui::RichText::new(format!(
                "{}{} {}",
                if self.tab == Tab::Local {
                    format!("{folder_count} folders · ")
                } else {
                    String::new()
                },
                visible.len(),
                match (self.tab == Tab::Local, visible.len() == 1) {
                    (true, true) => "track",
                    (true, false) => "tracks",
                    (false, true) => "station",
                    (false, false) => "stations",
                }
            ))
            .small()
            .color(theme::MUTED),
        );
        let scroll_key = (self.tab as usize, self.local_directory_path.clone().filter(|_| self.tab == Tab::Local));
        egui::ScrollArea::vertical()
            .id_salt(("station-list", scroll_key))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for folder in folders {
                    let name = folder.file_name().unwrap_or_default().to_string_lossy();
                    if !filter.is_empty() && !name.to_lowercase().contains(&filter) { continue; }
                    egui::Frame::new().fill(theme::SURFACE).corner_radius(10).inner_margin(7.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let artwork = self.card_artwork.show(ui, None, Icon::Folder);
                                let width = ui.available_width();
                                ui.vertical(|ui| {
                                    ui.set_width(width);
                                    if ui.add(egui::Button::new("").left_text(egui::RichText::new(name.as_ref()).strong())
                                        .frame(false).wrap().min_size(egui::vec2(width, 20.0))).clicked() || artwork.clicked() {
                                        open_folder = Some(folder.clone());
                                    }
                                    ui.label(egui::RichText::new("Folder").small().color(theme::MUTED));
                                });
                            });
                        });
                    ui.add_space(2.0);
                }
                if visible.is_empty() && folder_count == 0 {
                    ui.add_space(20.0);
                    ui.label(
                        egui::RichText::new(if stations.is_empty() && folders.is_empty() {
                            if self.tab == Tab::Local { "No media files or subfolders in this folder." }
                            else if self.tab == Tab::Favorites { "No favorites in this category yet. Use the star on a station, channel, artist, or album to save it here." }
                            else { "Your library is empty. Add a station or music folder to get started." }
                        } else {
                            "No matches. Try another name."
                        })
                        .color(theme::MUTED),
                    );
                }
                for station in visible {
                    let selected = self.current.as_ref().is_some_and(|s| s.id == station.id);
                    let fill = if selected {
                        egui::Color32::from_rgb(45, 37, 70)
                    } else {
                        theme::SURFACE
                    };
                    egui::Frame::new()
                        .fill(fill)
                        .corner_radius(10)
                        .inner_margin(7.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                if self
                                    .card_artwork
                                    .show(
                                        ui,
                                        card_artwork::Source::station(station),
                                        if station.is_file() {
                                            Icon::Position
                                        } else {
                                            Icon::Server
                                        },
                                    )
                                    .clicked()
                                {
                                    action = Some(station.clone());
                                }
                                let tools = if favorite_actions {
                                    40.0
                                } else if matches!(self.tab, Tab::Discover | Tab::YouTube | Tab::YouTubeMusic) {
                                    44.0
                                } else {
                                    0.0
                                };
                                let width = (ui.available_width() - tools - 10.0).max(80.0);
                                ui.vertical(|ui| {
                                    ui.set_width(width);
                                    let title = numbered_track_title(&station.name, station.track_number,
                                        (self.tab == Tab::Local && self.library.local_sort == SortOrder::TrackDiscAscending)
                                            || (self.tab == Tab::YouTubeMusic && self.music_artist_mode && self.music_browser.view == MusicView::Album));
                                    let text = egui::RichText::new(title).strong().color(
                                        if selected {
                                            theme::ACCENT
                                        } else {
                                            egui::Color32::from_rgb(232, 235, 247)
                                        },
                                    );
                                    if ui
                                        .add(
                                            egui::Button::new("")
                                                .left_text(text)
                                                .frame(false)
                                                .wrap()
                                                .min_size(egui::vec2(width, 20.0)),
                                        )
                                        .on_hover_hint("Play")
                                        .clicked()
                                    {
                                        action = Some(station.clone());
                                    }
                                    let subtitle = if station.is_youtube_channel() {
                                        [Some("Channel".to_owned()), station.artist.clone(), station.tags.clone()].into_iter().flatten().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join(" · ")
                                    } else if station.is_music_artist() {
                                        station.tags.clone().filter(|tags| !tags.trim().is_empty()).unwrap_or_else(|| "Artist".into())
                                    } else if station.is_music_album() {
                                        station.tags.clone().filter(|tags| !tags.trim().is_empty()).unwrap_or_else(|| "Album".into())
                                    } else if selected && self.player.active() {
                                        if self.player.paused {
                                            "Paused".to_owned()
                                        } else {
                                            "Now playing".to_owned()
                                        }
                                    } else if youtube::is_url(&station.stream_url) {
                                        station.artist.clone().or_else(|| station.tags.clone().filter(|tags| !tags.trim().is_empty())).unwrap_or_else(|| "YouTube".into())
                                    } else if self.tab == Tab::Local {
                                        "Media file".to_owned()
                                    } else {
                                        [
                                            station.country_code.clone(),
                                            station.codec.clone(),
                                            station.bitrate.map(|b| format!("{b} kbps")),
                                        ]
                                        .into_iter()
                                        .flatten()
                                        .filter(|s| !s.is_empty())
                                        .collect::<Vec<_>>()
                                        .join(" · ")
                                    };
                                    ui.label(
                                        egui::RichText::new(if subtitle.is_empty() {
                                            "Internet radio"
                                        } else {
                                            &subtitle
                                        })
                                        .small()
                                        .color(theme::MUTED),
                                    );
                                });
                                if favorite_actions {
                                    let menu = ui.menu_button("   ", |ui| {
                                        if icons::menu_item(ui, Icon::Settings, "Edit Station")
                                            .clicked()
                                        {
                                            edit = Some(station.clone());
                                            ui.close();
                                        }
                                        if icons::menu_item(ui, Icon::Trash, "Remove Favorite")
                                            .clicked()
                                        {
                                            remove = Some(station.id);
                                            ui.close();
                                        }
                                    });
                                    menu.response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            ui.is_enabled(),
                                            "Station actions",
                                        )
                                    });
                                    icons::draw(
                                        ui.painter(),
                                        Icon::More,
                                        egui::Rect::from_center_size(
                                            menu.response.rect.center(),
                                            egui::vec2(20.0, 20.0),
                                        ),
                                        theme::MUTED,
                                    );
                                } else if matches!(self.tab, Tab::Discover | Tab::YouTube | Tab::YouTubeMusic) {
                                    let favorite_id = self
                                        .library
                                        .stations
                                        .iter()
                                        .find(|s| s.stream_url == station.stream_url)
                                        .map(|s| s.id);
                                    let saved = favorite_id.is_some();
                                    let can_remove = self.tab == Tab::Discover;
                                    if ui
                                        .add_enabled(
                                            !saved || can_remove,
                                            IconButton::new(Icon::Star, "").selected(saved),
                                        )
                                        .on_hover_hint(if saved && can_remove {
                                            "Remove favorite"
                                        } else if saved {
                                            "Already saved"
                                        } else {
                                            "Save favorite"
                                        })
                                        .clicked()
                                    {
                                        if let Some(id) = favorite_id {
                                            remove = Some(id);
                                        } else {
                                            add = Some(station.clone());
                                        }
                                    }
                                }
                            });
                        });
                    ui.add_space(2.0);
                }
            });
        if let Some(folder) = open_folder {
            self.open_local_directory(folder);
        }
        if let Some(station) = action {
            if let Some(channel) = youtube::channel_for_station(&station) {
                self.open_channel_videos(channel);
            } else if let Some(artist) = music::artist_for_station(&station) {
                self.open_music_artist(artist);
            } else if let Some(album) = music::album_for_station(&station) {
                self.open_music_album(album);
            } else {
                self.shuffle_history.clear();
                self.queue = stations;
                self.queue_from_discover = self.tab == Tab::Discover;
                self.play_queued(station, None, 0.0);
            }
        }
        if let Some(id) = remove {
            self.library.stations.retain(|s| s.id != id);
            self.save();
        }
        if let Some(station) = edit {
            self.edit = Some(station);
            self.edit_existing = true;
        }
        if let Some(station) = add {
            // Land Favorites on the category this star just saved to,
            // so a YouTube star shows up under YouTube, and so on.
            self.favorite_category = station.favorite_category();
            self.library.add(station);
            self.save();
        }
    }
    fn library_ui(&mut self, ui: &mut egui::Ui) {
        if (self.tab == Tab::Local && !self.library.show_local_tab)
            || (self.tab == Tab::Remote && !self.library.show_remote_tab)
            || (self.tab == Tab::YouTube && !self.library.show_youtube_tab)
            || (self.tab == Tab::YouTubeMusic && !self.library.show_youtube_music_tab)
        {
            self.tab = Tab::Favorites;
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("KaleidoRadio").size(18.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(IconButton::new(Icon::Gear, "").selected(self.settings_open))
                    .on_hover_hint(if self.settings_open { "Close settings" } else { "Open settings" })
                    .clicked()
                {
                    self.settings_open = !self.settings_open;
                }
            });
        });
        ui.label(
            egui::RichText::new("YOUR SOUND, IN COLOR")
                .small()
                .color(theme::MUTED),
        );
        ui.add_space(12.0);
        let navigation_width = (ui.available_width() - 8.0) / 2.0;
        egui::Grid::new("library-navigation")
            .num_columns(2)
            .spacing(egui::vec2(8.0, 8.0))
            .show(ui, |ui| {
                let width = navigation_width;
                let show_local = self.library.show_local_tab;
                let show_remote = self.library.show_remote_tab;
                let show_youtube = self.library.show_youtube_tab;
                let show_music = self.library.show_youtube_music_tab;
                for (index, (tab, label, icon)) in [
                    (Tab::Favorites, "Favorites", Icon::Star),
                    (Tab::Local, "Local", Icon::Folder),
                    (Tab::Discover, "Discover", Icon::Search),
                    (Tab::Remote, "Remote", Icon::Server),
                    (Tab::YouTube, "YouTube", Icon::Play),
                    (Tab::YouTubeMusic, "YouTube Music", Icon::Music),
                ]
                .into_iter()
                .filter(|(tab, _, _)| match tab {
                    Tab::Local => show_local,
                    Tab::Remote => show_remote,
                    Tab::YouTube => show_youtube,
                    Tab::YouTubeMusic => show_music,
                    _ => true,
                })
                .enumerate()
                {
                    if ui
                        .add(
                            IconButton::new(icon, label)
                                .selected(self.tab == tab)
                                .min_size(egui::vec2(width, 30.0)),
                        )
                        .clicked()
                    {
                        self.tab = tab;
                        if tab == Tab::Discover && self.results.is_empty() && !self.searching {
                            self.search();
                        }
                    }
                    if index % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
        if matches!(self.tab, Tab::Favorites | Tab::Local) {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter by name…")
                    .desired_width(f32::INFINITY),
            );
        }
        match self.tab {
            Tab::Favorites => {
                ui.horizontal_wrapped(|ui| {
                    if icons::button(ui, Icon::Plus, "Station").clicked() {
                        self.edit = Some(Station::new("", ""));
                        self.edit_existing = false;
                    }
                    ui.menu_button("Manage", |ui| {
                        if icons::menu_item(ui, Icon::Import, "Import Favorites…").clicked() {
                            self.import(false);
                            ui.close();
                        }
                        if icons::menu_item(ui, Icon::Import, "Replace from JSON…").clicked() {
                            self.import(true);
                            ui.close();
                        }
                        if icons::menu_item(ui, Icon::Export, "Export Favorites…").clicked() {
                            self.export();
                            ui.close();
                        }
                    });
                });
                ui.horizontal(|ui| {
                    for category in [FavoriteCategory::Radio, FavoriteCategory::YouTube, FavoriteCategory::YouTubeMusic] {
                        ui.selectable_value(&mut self.favorite_category, category, category.label());
                    }
                });
                let favorites: Vec<Station> = self
                    .library
                    .stations
                    .iter()
                    .filter(|station| station.favorite_category() == self.favorite_category)
                    .cloned()
                    .collect();
                self.list(ui, favorites, true, &[]);
            }
            Tab::Local => {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.scanning, IconButton::new(Icon::Folder, "Folder"))
                        .clicked()
                    {
                        self.add_folder();
                    }
                    if ui
                        .add_enabled(!self.scanning, IconButton::new(Icon::Refresh, "Refresh"))
                        .clicked()
                        && let Some(folder) = self.library.folders.get(self.folder_index)
                    {
                        self.scan_folder(folder.path.clone());
                    }
                    if icons::button(ui, Icon::Trash, "Remove").clicked()
                        && self.folder_index < self.library.folders.len()
                    {
                        self.library.folders.remove(self.folder_index);
                        self.folder_index = 0;
                        self.reset_local_directory();
                        self.save();
                    }
                });
                if self.scanning {
                    ui.spinner();
                }
                let previous_folder_index = self.folder_index;
                egui::ComboBox::from_id_salt("folder")
                    .selected_text(
                        self.library
                            .folders
                            .get(self.folder_index)
                            .map_or("Choose Folder".into(), |f| {
                                f.path.file_name().unwrap_or_default().to_string_lossy()
                            }),
                    )
                    .show_ui(ui, |ui| {
                        for (index, folder) in self.library.folders.iter().enumerate() {
                            let name = folder.path.file_name().unwrap_or_default().to_string_lossy();
                            icons::menu_choice(ui, &mut self.folder_index, index, Icon::Folder, &name);
                        }
                    });
                if self.folder_index != previous_folder_index {
                    self.reset_local_directory();
                }
                if self.local_directory_path.is_none()
                    && let Some(root) = self.library.folders.get(self.folder_index)
                {
                    let path = self
                        .library
                        .local_directory
                        .clone()
                        .filter(|path| path.starts_with(&root.path))
                        .unwrap_or_else(|| root.path.clone());
                    self.open_local_directory(path);
                }
                if let Some(path) = self.local_directory_path.clone()
                    && let Some(root) = self.library.folders.get(self.folder_index)
                {
                    let root_path = root.path.clone();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(path != root_path, IconButton::new(Icon::Up, "Up"))
                            .clicked()
                            && let Some(parent) = path.parent()
                        {
                            self.open_local_directory(parent.to_path_buf());
                        }
                        if ui
                            .add_enabled(path != root_path, IconButton::new(Icon::Folder, "Root"))
                            .clicked()
                        {
                            self.open_local_directory(root_path.clone());
                        }
                    });
                    if self.library.show_local_path {
                        ui.label(
                            egui::RichText::new(
                                path.strip_prefix(&root_path)
                                    .ok()
                                    .filter(|p| !p.as_os_str().is_empty())
                                    .unwrap_or(&root_path)
                                    .to_string_lossy(),
                            )
                            .small()
                            .color(theme::MUTED),
                        );
                    }
                }
                if self.local_directory_loading {
                    ui.spinner();
                }
                if let Some(error) = &self.local_directory_error {
                    ui.label(error);
                }
                self.queue_controls_ui(ui);
                if sort_menu(ui, "local-sort", &mut self.library.local_sort) {
                    self.save();
                }
                if let Some(directory) = &self.local_directory {
                    let folders = directory.folders.clone();
                    let mut tracks = directory.tracks.clone();
                    tracks.sort_by(|a, b| {
                        self.library.local_sort.compare_track_disc(
                            (a.disc_number, a.track_number), (b.disc_number, b.track_number),
                        ).then_with(|| self.library.local_sort.compare(
                            &a.name,
                            a.modified_at,
                            &b.name,
                            b.modified_at,
                        ))
                    });
                    self.list(ui, tracks, false, &folders);
                }
            }
            Tab::Discover => {
                let enter = ui.text_edit_singleline(&mut self.query).lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui
                    .add_enabled(!self.searching, IconButton::new(Icon::Search, "Search"))
                    .clicked()
                    || enter
                {
                    self.search();
                }
                if self.searching {
                    ui.spinner();
                }
                self.list(ui, self.results.clone(), false, &[]);
            }
            Tab::Remote => self.remote_ui(ui),
            Tab::YouTube => self.youtube_ui(ui, false),
            Tab::YouTubeMusic => self.youtube_music_ui(ui),
        }
    }
    fn cancel_music_request(&mut self) {
        self.music_browser.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.music_browser.generation += 1;
        self.music_browser.loading = false;
    }
    fn request_music(&mut self, request: music::Request) {
        self.cancel_music_request();
        self.music_browser.error = None;
        self.music_browser.loading = true;
        self.music_browser.cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = self.music_browser.cancel.clone();
        let generation = self.music_browser.generation;
        let tx = self.tx.clone();
        std::thread::spawn(move || { let _ = tx.send(TaskResult::Music(generation, music::execute(request, &cancel))); });
    }
    fn apply_music_page(&mut self, page: music::ResultPage) {
        match page {
            music::ResultPage::Artists(artists) => {
                self.music_browser.artists = artists;
                self.music_browser.profile = None;
                self.music_browser.album = None;
                self.music_browser.tracks.clear();
                self.music_browser.view = MusicView::Artists;
            }
            music::ResultPage::Artist(profile) => {
                self.music_browser.profile = Some(profile);
                self.music_browser.view = MusicView::Albums;
            }
            music::ResultPage::Album(album, tracks) => {
                self.music_browser.album = Some(album);
                self.music_browser.tracks = tracks;
                self.music_browser.view = MusicView::Album;
            }
            music::ResultPage::Mix(page, mix) => {
                self.music_radio = None;
                self.music_browser.tracks = page.tracks.clone();
                self.music_browser.view = if mix == music::Mix::Radio { MusicView::Radio } else { MusicView::Shuffle };
                if mix == music::Mix::Radio {
                    self.music_radio_epoch += 1;
                    self.music_radio = Some(MusicRadio {
                        epoch: self.music_radio_epoch,
                        ids: page.tracks.iter().map(|track| track.id).collect(), page,
                        loading: false, waiting: false,
                        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                        artist: self.music_browser.profile.as_ref().map(|profile| profile.artist.id.clone()).unwrap_or_default(),
                    });
                }
                self.shuffle_history.clear();
                if let Some(track) = self.music_browser.tracks.first().cloned() {
                    self.play(track, self.music_browser.tracks.clone(), None, 0.0);
                }
            }
            music::ResultPage::More(_) => {}
        }
    }
    fn pump_radio(&mut self) {
        let Some(radio) = self.music_radio.as_mut() else { return; };
        if radio.loading || radio.page.continuation.is_none() { return; }
        let near_end = self.current.as_ref().and_then(|track| self.queue.iter().position(|item| item.id == track.id)).is_some_and(|index| index + 4 >= self.queue.len());
        if !near_end && !radio.waiting { return; }
        radio.loading = true;
        let epoch = radio.epoch;
        let request = music::Request::More(radio.page.clone());
        let cancel = radio.cancel.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || { let _ = tx.send(TaskResult::RadioMore(epoch, music::execute(request, &cancel))); });
    }
    fn apply_radio_more(&mut self, epoch: u64, result: Result<music::ResultPage, String>) {
        let Some(radio) = self.music_radio.as_mut().filter(|radio| radio.epoch == epoch) else { return; };
        radio.loading = false;
        match result {
            Ok(music::ResultPage::More(page)) => {
                let added = music::fresh_tracks(&self.queue, &page.tracks);
                radio.ids.extend(added.iter().map(|track| track.id));
                radio.page = page;
                // Stop repeated requests if the service returns an overlapping page only.
                if added.is_empty() { radio.page.continuation = None; }
                let resume = radio.waiting;
                radio.waiting = false;
                let next = added.first().cloned();
                self.queue.extend(added.iter().cloned());
                if self.music_browser.view == MusicView::Radio && self.music_browser.profile.as_ref().is_some_and(|profile| profile.artist.id == radio.artist) {
                    self.music_browser.tracks.extend(added);
                }
                if resume {
                    if let Some(track) = next { self.play(track, self.queue.clone(), None, 0.0); }
                    else { self.stop(); self.status = "No more radio songs are available.".into(); }
                }
            }
            Err(error) => {
                radio.page.continuation = None;
                if self.music_browser.view == MusicView::Radio { self.music_browser.error = Some(error.clone()); }
                self.status = format!("Radio: {error}");
            }
            _ => {}
        }
    }
    fn music_catalog_card(&mut self, ui: &mut egui::Ui, title: &str, subtitle: &str, thumbnail: Option<&String>, favorite: &Station) -> (bool, bool) {
        let mut clicked = false;
        let mut starred = false;
        let saved = self.library.stations.iter().any(|s| s.stream_url == favorite.stream_url);
        egui::Frame::new().fill(theme::SURFACE).corner_radius(10).inner_margin(8.0).show(ui, |ui| {
            ui.set_min_width((ui.available_width() - 1.0).max(0.0));
            ui.horizontal(|ui| {
                clicked |= self.card_artwork.show(ui, thumbnail.cloned().map(card_artwork::Source::Image), Icon::Music).clicked();
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - 44.0).max(60.0));
                    clicked |= ui.add(egui::Button::new("").left_text(egui::RichText::new(title).strong()).frame(false).wrap()).clicked();
                    if !subtitle.is_empty() { ui.label(egui::RichText::new(subtitle).small().color(theme::MUTED)); }
                });
                if ui.add_enabled(!saved, IconButton::new(Icon::Star, "").selected(saved)).on_hover_hint(if saved { "Already saved" } else { "Save favorite" }).clicked() {
                    starred = true;
                }
            });
        });
        ui.add_space(4.0);
        (clicked, starred)
    }
    fn youtube_music_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.music_artist_mode, true, "Artists & albums");
            ui.selectable_value(&mut self.music_artist_mode, false, "Songs");
        });
        if !self.music_artist_mode { self.youtube_ui(ui, true); return; }
        let enter = ui.add(egui::TextEdit::singleline(&mut self.music_browser.query).hint_text("Search for an artist").desired_width(f32::INFINITY)).lost_focus()
            && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if (icons::button(ui, Icon::Search, "Find artist").clicked() || enter) && !self.music_browser.query.trim().is_empty() {
            self.request_music(music::Request::Search(self.music_browser.query.clone(), self.library.youtube_search_limit));
        }
        if self.music_browser.loading {
            ui.horizontal(|ui| { ui.spinner(); ui.label("Loading YouTube Music…"); if ui.button("Cancel").clicked() { self.cancel_music_request(); } });
        }
        if let Some(error) = &self.music_browser.error { ui.label(error); }
        if self.music_browser.view != MusicView::Artists {
            ui.horizontal_wrapped(|ui| {
                if ui.add(IconButton::new(Icon::Back, "Artists")).on_hover_hint("Back to artists").clicked() { self.cancel_music_request(); self.music_browser.view = MusicView::Artists; }
                if self.music_browser.profile.is_some() && ui.button("Albums").clicked() { self.cancel_music_request(); self.music_browser.view = MusicView::Albums; }
            });
        }
        if let Some(profile) = self.music_browser.profile.clone().filter(|_| self.music_browser.view != MusicView::Artists) {
            ui.label(egui::RichText::new(&profile.artist.name).strong().size(17.0));
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(profile.shuffle.is_some() && !self.music_browser.loading, IconButton::new(Icon::Music, "Random songs")).clicked() {
                    self.request_music(music::Request::Mix(profile.shuffle.unwrap(), music::Mix::Shuffle));
                }
                if ui.add_enabled(profile.radio.is_some() && !self.music_browser.loading, IconButton::new(Icon::Server, "Start radio")).clicked() {
                    self.request_music(music::Request::Mix(profile.radio.unwrap(), music::Mix::Radio));
                }
                let favorite = music::artist_station(&profile.artist);
                let saved = self.library.stations.iter().any(|s| s.stream_url == favorite.stream_url);
                if ui.add_enabled(!saved, IconButton::new(Icon::Star, "Save artist").selected(saved)).on_hover_hint(if saved { "Already saved" } else { "Save favorite" }).clicked() {
                    self.favorite_category = FavoriteCategory::YouTubeMusic;
                    self.library.add(favorite);
                    self.save();
                }
            });
        }
        // Give catalogue results a bounded viewport.
        match self.music_browser.view {
            MusicView::Artists => {
                let artists = self.music_browser.artists.clone();
                let query = self.music_browser.query.clone();
                let mut starred = Vec::new();
                egui::ScrollArea::vertical()
                    .id_salt(("music-artists", query))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for artist in artists {
                            let favorite = music::artist_station(&artist);
                            let (clicked, starred_card) = self.music_catalog_card(ui, &artist.name, &artist.subtitle, artist.thumbnail.as_ref(), &favorite);
                            if starred_card { starred.push(favorite); }
                            if clicked {
                                self.request_music(music::Request::Artist(artist));
                            }
                        }
                        if self.music_browser.artists.is_empty() && !self.music_browser.loading {
                            ui.small("Find an artist to browse albums, shuffle their songs, or start radio.");
                        }
                    });
                for favorite in starred {
                    self.favorite_category = FavoriteCategory::YouTubeMusic;
                    self.library.add(favorite);
                    self.save();
                }
            }
            MusicView::Albums => {
                if let Some(profile) = self.music_browser.profile.clone() {
                    let mut starred = Vec::new();
                    egui::ScrollArea::vertical()
                        .id_salt(("music-albums", &profile.artist.id))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if profile.albums.is_empty() { ui.small("No albums are available for this artist."); }
                            let mut previous_section = "";
                            for album in profile.albums {
                                let section = if album.subtitle.starts_with("Album") { "Albums" } else { "Singles & EPs" };
                                if section != previous_section { ui.label(egui::RichText::new(section).strong()); previous_section = section; }
                                let favorite = music::album_station(&album);
                                let (clicked, starred_card) = self.music_catalog_card(ui, &album.title, &album.subtitle, album.thumbnail.as_ref(), &favorite);
                                if starred_card { starred.push(favorite); }
                                if clicked {
                                    self.request_music(music::Request::Album(album));
                                }
                            }
                        });
                    for favorite in starred {
                        self.favorite_category = FavoriteCategory::YouTubeMusic;
                        self.library.add(favorite);
                        self.save();
                    }
                }
            }
            MusicView::Album | MusicView::Shuffle | MusicView::Radio => {
                let title = match self.music_browser.view {
                    MusicView::Album => self.music_browser.album.as_ref().map(|album| album.title.as_str()).unwrap_or("Album"),
                    MusicView::Shuffle => "Random artist songs",
                    _ => "Artist radio · includes related artists",
                };
                ui.label(egui::RichText::new(title).strong());
                if self.music_browser.view != MusicView::Radio {
                    ui.horizontal(|ui| {
                        if self.music_browser.view == MusicView::Album {
                            if let Some(album) = self.music_browser.album.clone() {
                                let favorite = music::album_station(&album);
                                let saved = self.library.stations.iter().any(|s| s.stream_url == favorite.stream_url);
                                if ui.add_enabled(!saved, IconButton::new(Icon::Star, "Save album").selected(saved)).on_hover_hint(if saved { "Already saved" } else { "Save favorite" }).clicked() {
                                    self.favorite_category = FavoriteCategory::YouTubeMusic;
                                    self.library.add(favorite);
                                    self.save();
                                }
                            }
                        }
                        if ui.add_enabled(!self.music_browser.loading && !self.music_browser.tracks.is_empty(), IconButton::new(Icon::Play, if self.music_browser.view == MusicView::Album { "Play album" } else { "Play songs" })).clicked() {
                            self.shuffle_history.clear();
                            if let Some(track) = self.music_browser.tracks.first().cloned() { self.play(track, self.music_browser.tracks.clone(), None, 0.0); }
                        }
                    });
                    self.queue_controls_ui(ui);
                } else { ui.small("Radio follows YouTube Music's mix order and loads more songs automatically."); }
                self.list(ui, self.music_browser.tracks.clone(), false, &[]);
            }
        }
    }
    /// Channel results as scrollable cards, mirroring `list`'s video cards:
    /// a `SURFACE` frame, artwork on the left, a strong title button, a
    /// small muted subtitle, and 2.0 of space between cards. Returns the
    /// channel whose artwork or title was clicked, if any.
    /// Open a channel's videos, newest first, in the YouTube tab —
    /// whether the channel came from a search card or a favorite.
    /// Open a saved artist or album in YouTube Music, whether it came
    /// from a catalogue card or a favorite.
    fn open_music_artist(&mut self, artist: music::Artist) {
        self.tab = Tab::YouTubeMusic;
        self.music_artist_mode = true;
        self.request_music(music::Request::Artist(artist));
    }
    fn open_music_album(&mut self, album: music::Album) {
        self.tab = Tab::YouTubeMusic;
        self.music_artist_mode = true;
        self.request_music(music::Request::Album(album));
    }
    fn open_channel_videos(&mut self, channel: youtube::Channel) {
        self.tab = Tab::YouTube;
        let state = &mut self.youtube_search;
        state.channel_mode = true;
        state.generation += 1;
        state.searching = true;
        state.error = None;
        state.active_channel = Some(channel.clone());
        state.channel_results.clear();
        state.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        state.cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled = state.cancel.clone();
        let limit = self.library.youtube_search_limit;
        let generation = state.generation;
        let reference = channel.id.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut videos = Vec::new();
            let result = youtube::search_channel_videos(&reference, limit, &cancelled, |station| videos.push(station));
            let result = result.map(|()| videos);
            let _ = tx.send(TaskResult::YouTubeChannelVideos(generation, result));
        });
    }
    fn channel_list(&mut self, ui: &mut egui::Ui, channels: &[youtube::Channel]) -> Option<youtube::Channel> {
        ui.label(
            egui::RichText::new(format!(
                "{} {}",
                channels.len(),
                if channels.len() == 1 { "channel" } else { "channels" }
            ))
            .small()
            .color(theme::MUTED),
        );
        let mut clicked = None;
        let mut starred = None;
        egui::ScrollArea::vertical()
            .id_salt("channel-list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if channels.is_empty() {
                    ui.add_space(20.0);
                    ui.label(
                        egui::RichText::new("Search for a channel to see results.").color(theme::MUTED),
                    );
                    return;
                }
                for channel in channels {
                    let mut card_clicked = false;
                    let station = youtube::channel_station(channel);
                    let saved = self.library.stations.iter().any(|s| s.stream_url == station.stream_url || (s.youtube_channel_id.is_some() && s.youtube_channel_id == station.youtube_channel_id));
                    egui::Frame::new()
                        .fill(theme::SURFACE)
                        .corner_radius(10)
                        .inner_margin(7.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                if self
                                    .card_artwork
                                    .show(
                                        ui,
                                        channel.thumbnail_url.clone().map(card_artwork::Source::Image),
                                        Icon::Server,
                                    )
                                    .clicked()
                                {
                                    card_clicked = true;
                                }
                                let width = (ui.available_width() - 44.0 - 10.0).max(80.0);
                                ui.vertical(|ui| {
                                    ui.set_width(width);
                                    if ui
                                        .add(
                                            egui::Button::new("")
                                                .left_text(
                                                    egui::RichText::new(&channel.name)
                                                        .strong()
                                                        .color(egui::Color32::from_rgb(232, 235, 247)),
                                                )
                                                .frame(false)
                                                .wrap()
                                                .min_size(egui::vec2(width, 20.0)),
                                        )
                                        .on_hover_hint("Show channel videos")
                                        .clicked()
                                    {
                                        card_clicked = true;
                                    }
                                    let subtitle = [
                                        channel.handle.clone(),
                                        youtube::subscriber_label(channel.follower_count),
                                    ]
                                    .into_iter()
                                    .flatten()
                                    .collect::<Vec<_>>()
                                    .join(" · ");
                                    ui.label(
                                        egui::RichText::new(if subtitle.is_empty() {
                                            "YouTube channel"
                                        } else {
                                            &subtitle
                                        })
                                        .small()
                                        .color(theme::MUTED),
                                    );
                                    if let Some(description) = &channel.description {
                                        let short: String = description.chars().take(140).collect();
                                        ui.label(egui::RichText::new(short).small().color(theme::MUTED));
                                    }
                                });
                                if ui
                                    .add_enabled(!saved, IconButton::new(Icon::Star, "").selected(saved))
                                    .on_hover_hint(if saved { "Already saved" } else { "Save favorite" })
                                    .clicked()
                                {
                                    starred = Some(channel.clone());
                                }
                            });
                        });
                    if card_clicked {
                        clicked = Some(channel.clone());
                    }
                    ui.add_space(2.0);
                }
            });
        if let Some(channel) = starred {
            self.favorite_category = FavoriteCategory::YouTube;
            self.library.add(youtube::channel_station(&channel));
            self.save();
        }
        clicked
    }
    fn youtube_ui(&mut self, ui: &mut egui::Ui, music: bool) {
        let name = if music { "YouTube Music" } else { "YouTube" };
        let state = if music { &mut self.youtube_music_search } else { &mut self.youtube_search };
        if !music {
            ui.horizontal(|ui| {
                if ui.selectable_label(!state.channel_mode, "Videos").clicked() {
                    state.channel_mode = false;
                }
                if ui.selectable_label(state.channel_mode, "Channels").clicked() {
                    state.channel_mode = true;
                }
            });
        }
        let hint = if !music && state.channel_mode {
            "Search for a channel".to_owned()
        } else {
            format!("Search {name} or paste a link")
        };
        let enter = ui.add(egui::TextEdit::singleline(&mut state.query)
            .hint_text(hint)
            .desired_width(f32::INFINITY)).lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.add_enabled(!state.searching, IconButton::new(Icon::Search, &format!("Search {name}"))).clicked() || enter)
            && !state.query.trim().is_empty()
        {
            // Channels mode searches for channel names. A pasted link in
            // Channels mode falls back to the regular video search, so
            // paste-a-link keeps working exactly as before.
            let channel_search = !music && state.channel_mode && !youtube::is_url(&state.query);
            if !channel_search && !music && state.channel_mode {
                state.channel_mode = false;
            }
            state.generation += 1;
            state.searching = true;
            state.error = None;
            if channel_search {
                state.channels.clear();
                state.active_channel = None;
                state.channel_results.clear();
            } else {
                state.results.clear();
            }
            state.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            state.cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cancelled = state.cancel.clone();
            let limit = self.library.youtube_search_limit;
            let generation = state.generation;
            let query = state.query.clone();
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                if channel_search {
                    let mut on_result = |channel| { let _ = tx.send(TaskResult::YouTubeChannelItem(generation, channel)); };
                    let result = youtube::search_channels(&query, limit, &cancelled, &mut on_result);
                    let _ = tx.send(TaskResult::YouTubeChannels(generation, result));
                } else {
                    let mut on_result = |station| { let _ = tx.send(TaskResult::YouTubeSearchItem(music, generation, station)); };
                    let result = if music {
                        youtube::search_music(&query, limit, &cancelled, &mut on_result)
                    } else {
                        youtube::search(&query, limit, &cancelled, &mut on_result)
                    };
                    let _ = tx.send(TaskResult::YouTubeSearch(music, generation, result));
                }
            });
        }
        if state.searching {
            let count = if !music && state.channel_mode {
                if state.active_channel.is_some() { state.channel_results.len() } else { state.channels.len() }
            } else {
                state.results.len()
            };
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("{} results", count));
                if ui.button("Stop search").clicked() { state.cancel.store(true, std::sync::atomic::Ordering::Relaxed); }
            });
        }
        if let Some(error) = &state.error { ui.label(error); }
        if !music && state.channel_mode {
            let channels = state.channels.clone();
            let active_channel = state.active_channel.clone();
            let mut back_clicked = false;
            let mut channel_starred = false;
            if let Some(channel) = &active_channel {
                let station = youtube::channel_station(channel);
                let saved = self.library.stations.iter().any(|s| s.stream_url == station.stream_url || (s.youtube_channel_id.is_some() && s.youtube_channel_id == station.youtube_channel_id));
                ui.horizontal(|ui| {
                    if ui.add(IconButton::new(Icon::Back, "Back to channels")).clicked() { back_clicked = true; }
                    ui.heading(&channel.name);
                    if ui.add_enabled(!saved, IconButton::new(Icon::Star, "").selected(saved)).on_hover_hint(if saved { "Already saved" } else { "Save favorite" }).clicked() { channel_starred = true; }
                });
                if let Some(handle) = &channel.handle { ui.label(handle); }
            }
            if channel_starred {
                if let Some(channel) = &active_channel {
                    self.favorite_category = FavoriteCategory::YouTube;
                    self.library.add(youtube::channel_station(channel));
                    // Saved below, after the search-state borrow ends.
                }
            }
            if back_clicked {
                state.active_channel = None;
                state.channel_results.clear();
                state.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                if channel_starred { self.save(); }
                return;
            }
            if state.active_channel.is_some() {
                let results = state.channel_results.clone();
                self.queue_controls_ui(ui);
                self.list(ui, results, false, &[]);
                if channel_starred { self.save(); }
                return;
            }
            let clicked_channel = self.channel_list(ui, &channels);
            if let Some(channel) = clicked_channel {
                self.open_channel_videos(channel);
            }
            return;
        }
        let results = state.results.clone();
        self.queue_controls_ui(ui);
        self.list(ui, results, false, &[]);
    }
    fn queue_controls_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let shuffle_changed = ui.checkbox(&mut self.library.shuffle, "Shuffle").changed();
            let mut changed = shuffle_changed;
            for (mode, icon, label, description) in [
                (Repeat::Once, Icon::Stop, "Repeat: Off", "Play the remaining queue, then stop."),
                (Repeat::All, Icon::RepeatAll, "Repeat: All", "Repeat the entire queue."),
                (Repeat::One, Icon::RepeatOne, "Repeat: One", "Repeat the current track."),
            ] {
                if ui.add(IconButton::new(icon, label).selected(self.library.repeat == mode))
                    .on_hover_hint(description)
                    .clicked()
                {
                    self.library.repeat = mode;
                    changed = true;
                }
            }
            if shuffle_changed {
                self.shuffle_history.clear();
            }
            if changed {
                self.clear_youtube_preload();
                self.cancel_transition();
                self.save();
            }
        });
    }
    fn load_remote_entry_metadata(&mut self) {
        self.remote_metadata_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.remote_metadata_cancel = cancel.clone();
        let urls: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| !entry.directory)
            .filter(|entry| !entry.url.starts_with("sftp:"))
            .filter(|entry| {
                !self
                    .remote_metadata_cache
                    .contains_key(&(self.remote.username.clone(), entry.url.clone()))
            })
            .map(|entry| entry.url.clone())
            .collect();
        self.remote_metadata_pending = urls.len();
        let auth = (self.remote.username.clone(), self.password.clone());
        let tx = self.tx.clone();
        let generation = self.remote_generation;
        std::thread::spawn(move || {
            for url in urls {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let metadata = file_metadata::read_summary(&url, Some(&auth)).unwrap_or_default();
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                if tx
                    .send(TaskResult::RemoteEntryMetadata(generation, url, metadata))
                    .is_err()
                {
                    break;
                }
            }
        });
    }
    fn remote_entry_metadata(
        &self,
        entry: &network::RemoteEntry,
    ) -> Option<&file_metadata::Metadata> {
        self.remote_metadata_cache
            .get(&(self.remote.username.clone(), entry.url.clone()))
    }
    fn remote_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut select = None;
            ui.label(egui::RichText::new("Saved servers").strong());
            for (i, connection) in self.library.connections.iter().enumerate() {
                if ui
                    .add(
                        IconButton::new(Icon::Server, &connection.name)
                            .selected(
                                !self.remote_address.is_empty()
                                    && self.remote.url == connection.url,
                            )
                            .min_size(egui::vec2(ui.available_width(), 30.0)),
                    )
                    .on_hover_hint(&connection.url)
                    .clicked()
                {
                    select = Some(i);
                }
            }
            if self.library.connections.is_empty() {
                ui.label(
                    egui::RichText::new("No saved servers yet.")
                        .small()
                        .color(theme::MUTED),
                );
            }
            ui.horizontal(|ui| {
                if icons::button(ui, Icon::Plus, "New Server").clicked() {
                    self.remote = RemoteConnection::default();
                    self.password.clear();
                    self.remote_address.clear();
                    self.entries.clear();
                    self.remote_generation += 1;
                    self.remote_metadata_cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    self.remote_metadata_pending = 0;
                    self.browsing = false;
                    self.remote_settings_open = true;
                }
                if !self.remote_address.is_empty()
                    && icons::button(ui, Icon::Settings, "Server settings").clicked()
                {
                    self.remote_settings_open = true;
                }
            });
            if let Some(i) = select {
                self.remote = self.library.connections[i].clone();
                self.password = credentials::load(&self.remote);
                self.remote_settings_open = false;
                self.browse(format!("{}/", self.remote.url.trim_end_matches('/')));
            }
            if self.remote_settings_open {
                ui.separator();
                ui.label("Name");
                ui.text_edit_singleline(&mut self.remote.name);
                ui.label("Server URL");
                ui.add(egui::TextEdit::singleline(&mut self.remote.url).hint_text("https://server/path, smb://server/share, or sftp://server/path"));
                ui.small("SFTP: verify the server with ssh first. Leave Password empty to use your SSH agent.");
                ui.label("Username");
                ui.text_edit_singleline(&mut self.remote.username);
                ui.label("Password");
                ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
                ui.horizontal(|ui| {
                    if ui.button("Connect").clicked() {
                        if !network::valid_remote_url(&self.remote.url) {
                            self.status = "Enter an HTTP(S), SMB, or SFTP URL without embedded credentials.".into();
                        } else {
                            self.remote_settings_open = false;
                            self.remote_metadata_cache.clear();
                            self.browse(format!("{}/", self.remote.url.trim_end_matches('/')));
                        }
                    }
                    if icons::button(ui, Icon::Save, "Save server").clicked() {
                        if self.remote.name.trim().is_empty()
                            || !network::valid_remote_url(&self.remote.url)
                        {
                            self.status =
                                "Enter a name and an HTTP(S), SMB, or SFTP URL without embedded credentials."
                                    .into();
                        } else if let Err(error) = credentials::save(&self.remote, &self.password) {
                            self.status = error;
                        } else {
                            self.library
                                .connections
                                .retain(|c| c.url != self.remote.url);
                            self.library.connections.push(self.remote.clone());
                            self.remote_metadata_cache.clear();
                            self.remote_settings_open = false;
                            self.save();
                            self.browse(format!("{}/", self.remote.url.trim_end_matches('/')));
                        }
                    }
                    if self
                        .library
                        .connections
                        .iter()
                        .any(|c| c.url == self.remote.url)
                        && icons::button(ui, Icon::Trash, "Remove server").clicked()
                    {
                        if let Err(error) = credentials::delete(&self.remote) {
                            self.status = error;
                            return;
                        }
                        self.library
                            .connections
                            .retain(|c| c.url != self.remote.url);
                        self.remote_settings_open = false;
                        self.remote_address.clear();
                        self.entries.clear();
                        self.remote_generation += 1;
                        self.remote_metadata_cancel
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        self.remote_metadata_pending = 0;
                        self.browsing = false;
                        self.save();
                    }
                    if ui.button("Cancel").clicked() {
                        self.remote_settings_open = false;
                    }
                });
                return;
            }
            if self.remote_address.is_empty() {
                return;
            }
            ui.separator();
            if self.browsing {
                ui.spinner();
            }
            if self.library.show_remote_path {
                ui.small(&self.remote_address);
            }
            let parent = if self.remote_address.starts_with("smb:") {
                smb::parent(&self.remote_address)
            } else {
                url::Url::parse(&self.remote_address).and_then(|url| url.join("../")).ok().map(String::from)
            };
            ui.horizontal(|ui| {
                if ui.add_enabled(parent.is_some(), IconButton::new(Icon::Up, "Up one folder")).clicked()
                    && let Some(parent) = parent {
                    self.browse(parent);
                }
                if ui.add_enabled(!self.browsing, IconButton::new(Icon::Refresh, "Refresh")).clicked() {
                    self.remote_metadata_cache.clear();
                    self.browse_directory(self.remote_address.clone(), true);
                }
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.remote_filter)
                    .hint_text("Filter by name…")
                    .desired_width(f32::INFINITY),
            );
            self.queue_controls_ui(ui);
            if sort_menu(ui, "remote-sort", &mut self.library.remote_sort) {
                self.save();
            }
            let username = &self.remote.username;
            let metadata = &self.remote_metadata_cache;
            self.entries.sort_by(|a, b| {
                let title = |entry: &network::RemoteEntry| {
                    metadata
                        .get(&(username.clone(), entry.url.clone()))
                        .and_then(|m| m.title.clone())
                        .unwrap_or_else(|| entry.name.clone())
                };
                b.directory.cmp(&a.directory).then_with(|| {
                    let numbers = |entry: &network::RemoteEntry| metadata
                        .get(&(username.clone(), entry.url.clone()))
                        .map_or((None, None), |m| (m.disc_number, m.track_number));
                    self.library.remote_sort.compare_track_disc(numbers(a), numbers(b))
                        .then_with(|| self.library.remote_sort.compare(
                        &title(a),
                        a.modified_at,
                        &title(b),
                        b.modified_at,
                    ))
                })
            });
            let filter = self.remote_filter.trim().to_lowercase();
            let visible: Vec<_> = self
                .entries
                .iter()
                .filter(|entry| {
                    filter.is_empty()
                        || entry.name.to_lowercase().contains(&filter)
                        || self
                            .remote_entry_metadata(entry)
                            .and_then(|m| m.title.as_deref())
                            .is_some_and(|title| title.to_lowercase().contains(&filter))
                })
                .collect();
            if let Some(error) = &self.remote_error {
                ui.colored_label(theme::MUTED, error);
            }
            if visible.is_empty() && !self.browsing && self.remote_error.is_none() {
                ui.label(
                    egui::RichText::new(if filter.is_empty() {
                        "No media files or folders here."
                    } else {
                        "No matches. Try another name."
                    })
                    .small()
                    .color(theme::MUTED),
                );
            }
            ui.label(
                egui::RichText::new(format!(
                    "{} folders · {} tracks",
                    visible.iter().filter(|e| e.directory).count(),
                    visible.iter().filter(|e| !e.directory).count(),
                ))
                .small()
                .color(theme::MUTED),
            );
            if self.remote_metadata_pending > 0 {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.small(format!(
                        "Reading metadata · {} remaining",
                        self.remote_metadata_pending
                    ));
                });
            }
            let mut open = None;
            for &entry in &visible {
                let metadata = self.remote_entry_metadata(entry);
                let title = metadata
                    .and_then(|m| m.title.as_deref())
                    .unwrap_or(&entry.name)
                    .to_owned();
                let title = numbered_track_title(&title, metadata.and_then(|m| m.track_number),
                    !entry.directory && self.library.remote_sort == SortOrder::TrackDiscAscending);
                let selected = !entry.directory
                    && self
                        .current
                        .as_ref()
                        .is_some_and(|s| s.stream_url == entry.url);
                let subtitle = if entry.directory {
                    "Folder"
                } else if selected && self.player.active() {
                    if self.player.paused {
                        "Paused"
                    } else {
                        "Now playing"
                    }
                } else {
                    metadata
                        .and_then(|m| m.album.as_deref())
                        .unwrap_or("Media file")
                };
                let subtitle = subtitle.to_owned();
                egui::Frame::new()
                    .fill(if selected {
                        egui::Color32::from_rgb(45, 37, 70)
                    } else {
                        theme::SURFACE
                    })
                    .corner_radius(10)
                    .inner_margin(7.0)
                    .show(ui, |ui| {
                        let width = ui.available_width();
                        ui.set_min_width(width);
                        ui.horizontal(|ui| {
                            let source = (!entry.directory).then(|| card_artwork::Source::File {
                                address: entry.url.clone(),
                                auth: Some((self.remote.username.clone(), self.password.clone())),
                            });
                            if self
                                .card_artwork
                                .show(
                                    ui,
                                    source,
                                    if entry.directory {
                                        Icon::Folder
                                    } else {
                                        Icon::Position
                                    },
                                )
                                .clicked()
                            {
                                open = Some(entry.clone());
                            }
                            let text_width = ui.available_width();
                            ui.vertical(|ui| {
                                ui.set_width(text_width);
                                if ui
                                    .add(
                                        egui::Button::new("")
                                            .left_text(egui::RichText::new(&title).strong().color(
                                                if selected {
                                                    theme::ACCENT
                                                } else {
                                                    ui.visuals().text_color()
                                                },
                                            ))
                                            .frame(false)
                                            .wrap()
                                            .min_size(egui::vec2(text_width, 20.0)),
                                    )
                                    .on_hover_hint(&entry.name)
                                    .clicked()
                                {
                                    open = Some(entry.clone());
                                }
                                ui.label(
                                    egui::RichText::new(&subtitle).small().color(theme::MUTED),
                                );
                            });
                        });
                    });
                ui.add_space(2.0);
            }
            if let Some(entry) = open {
                if entry.directory {
                    self.browse(entry.url);
                } else {
                    let tracks = visible
                        .iter()
                        .copied()
                        .filter(|e| !e.directory)
                        .map(|e| {
                            let title = self
                                .remote_entry_metadata(e)
                                .and_then(|m| m.title.as_deref())
                                .unwrap_or(&e.name);
                            let mut s = Station::new(title, &e.url);
                            s.media_source = Some("remoteFile".into());
                            if let Some(metadata) = self.remote_entry_metadata(e) {
                                s.track_number = metadata.track_number;
                                s.disc_number = metadata.disc_number;
                            }
                            s
                        })
                        .collect::<Vec<_>>();
                    if let Some(station) =
                        tracks.iter().find(|s| s.stream_url == entry.url).cloned()
                    {
                        self.shuffle_history.clear();
                        self.play(
                            station,
                            tracks,
                            Some((self.remote.username.clone(), self.password.clone())),
                            0.0,
                        );
                    }
                }
            }
        });
    }
    fn import(&mut self, replace: bool) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Favorites JSON", &["json"])
            .pick_file()
        {
            match std::fs::read(path)
                .map_err(|e| e.to_string())
                .and_then(|b| self.library.import(&b, replace))
            {
                Ok(n) => {
                    self.status = format!("Imported {n} favorites.");
                    self.save();
                }
                Err(e) => self.status = format!("Import failed: {e}"),
            }
        }
    }
    fn export(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name("KaleidoRadio-favorites.json")
            .save_file()
        {
            match self
                .library
                .export()
                .and_then(|b| std::fs::write(path, b).map_err(|e| e.to_string()))
            {
                Ok(()) => self.status = "Favorites exported.".into(),
                Err(e) => self.status = e,
            }
        }
    }
    fn playback_mode_ui(&mut self, ui: &mut egui::Ui, category: FavoriteCategory, in_menu: bool) {
        let previous = match category {
            FavoriteCategory::YouTube => self.library.youtube_audio_only,
            _ => self.library.video_audio_only,
        };
        let mut audio_only = previous;
        let mut choices = |ui: &mut egui::Ui| {
            icons::menu_choice(ui, &mut audio_only, false, Icon::Fullscreen, "Video");
            icons::menu_choice(ui, &mut audio_only, true, Icon::Music, "Audio");
        };
        if in_menu {
            choices(ui);
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.label("Playback Mode");
                choices(ui);
            });
        }
        if previous != audio_only {
            match category {
                FavoriteCategory::YouTube => self.library.youtube_audio_only = audio_only,
                _ => self.library.video_audio_only = audio_only,
            }
            let applies_to_current = self.current.as_ref().is_some_and(|station| !video::is_file_video(station) && match category {
                FavoriteCategory::YouTube => station.favorite_category() == FavoriteCategory::YouTube,
                _ => station.favorite_category() != FavoriteCategory::YouTube,
            });
            if applies_to_current {
                let position = self.playback_position();
                let paused = if self.smb_connecting { self.youtube_resume_paused } else { self.player.paused };
                let playing = self.player.active() || self.smb_connecting;
                if let Some(station) = self.current.clone() {
                    self.current_has_video = self.source_has_video
                        && (video::is_file_video(&station) || !self.library.audio_only_for(&station));
                }
                self.video_texture = None;
                if playing {
                    self.start_playback(position);
                    self.youtube_resume_paused = paused;
                    // Local/Remote playback starts synchronously; YouTube restores on ready.
                    if !self.smb_connecting { self.restore_youtube_pause(); }
                }
            }
            self.save();
            if in_menu {
                ui.close();
            }
            ui.ctx().request_repaint();
        }
    }
    fn video_controls_ui(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let volume_width = (width * 0.14).clamp(45.0, 110.0);
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.horizontal_wrapped(|ui| {
            let playing = self.player.active() && !self.player.paused;
            if ui.add(IconButton::new(if playing { Icon::Pause } else { Icon::Play }, ""))
                .on_hover_hint(if playing { "Pause" } else { "Play" }).clicked() {
                if self.player.active() { self.player.toggle(); } else { self.resume(); }
                self.update_media();
            }
            if ui.add(IconButton::new(Icon::Stop, "")).on_hover_hint("Stop").clicked() { self.stop(); }
            if ui.add(icons::IconButton::new(Icon::Fullscreen, ""))
                .on_hover_hint(if self.video_fullscreen { "Exit fullscreen (Esc)" } else { "Fullscreen" }).clicked() {
                self.video_fullscreen = !self.video_fullscreen;
                self.video_controls.reset();
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.video_fullscreen));
            }
            let mut position = self.seek_position.unwrap_or_else(|| if self.player.active() {
                self.playback_position()
            } else {
                self.current.as_ref().and_then(|s| self.library.local_positions.get(&s.stream_url)).copied().unwrap_or(0.)
            });
            if self.video_seekable {
                let scrubber_gap = 8.0;
                ui.add_space(scrubber_gap);
                // Shaka-style scrub bar: played fill, buffered fill, and
                // a hover preview of the time under the pointer.
                let bar_width = (width - volume_width - 382.0 - scrubber_gap).max(30.0);
                let (bar_rect, seek) = ui.allocate_exact_size(egui::vec2(bar_width, 28.0), egui::Sense::click_and_drag());
                seek.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Slider,
                        ui.is_enabled(),
                        "Playback position",
                    )
                });
                let track = egui::Rect::from_center_size(bar_rect.center(), egui::vec2(bar_rect.width(), 6.0));
                let painter = ui.painter();
                painter.rect_filled(track, 3.0, egui::Color32::from_gray(70));
                let frac = |seconds: f64| (seconds / self.duration).clamp(0.0, 1.0) as f32;
                let buffered_x = track.left() + track.width() * frac(self.video_buffered);
                if buffered_x > track.left() {
                    painter.rect_filled(egui::Rect::from_min_max(track.left_top(), egui::pos2(buffered_x, track.bottom())), 3.0, egui::Color32::from_gray(140));
                }
                let played_x = track.left() + track.width() * frac(position);
                if played_x > track.left() {
                    painter.rect_filled(egui::Rect::from_min_max(track.left_top(), egui::pos2(played_x, track.bottom())), 3.0, theme::ACCENT);
                }
                painter.circle_filled(egui::pos2(played_x, track.center().y), 6.0, egui::Color32::WHITE);
                if let Some(hover) = seek.hover_pos() {
                    let preview = video::seek_time_for_x(hover.x, bar_rect.left(), bar_rect.width(), self.duration);
                    let x = hover.x.clamp(bar_rect.left() + 18.0, (bar_rect.right() - 18.0).max(bar_rect.left() + 18.0));
                    painter.text(egui::pos2(x, bar_rect.top()), egui::Align2::CENTER_TOP, video::format_time(preview), egui::FontId::proportional(12.0), egui::Color32::WHITE);
                }
                if seek.dragged() {
                    if let Some(pointer) = seek.interact_pointer_pos() {
                        position = video::seek_time_for_x(pointer.x, bar_rect.left(), bar_rect.width(), self.duration);
                        self.seek_position = Some(position);
                    }
                }
                if seek.drag_stopped() || seek.clicked() {
                    if let Some(pointer) = seek.interact_pointer_pos() {
                        position = video::seek_time_for_x(pointer.x, bar_rect.left(), bar_rect.width(), self.duration);
                    }
                    self.seek_position = None;
                    self.seek_playback(position);
                }
            }
            if self.duration > 0. {
                ui.label(format!("{} / {}", video::format_time(position), video::format_time(self.duration)));
            } else { ui.label("LIVE"); }
            icons::inline(ui, Icon::Volume, theme::MUTED);
            ui.spacing_mut().slider_width = volume_width;
            let volume = ui.add(egui::Slider::new(&mut self.library.volume, 0.0..=1.0)
                .show_value(false)).on_hover_hint("Volume");
            if volume.changed() { self.video_muted = false; self.player.volume(self.library.volume); }
            if volume.drag_stopped() || (volume.changed() && !volume.dragged()) { self.save(); }
            ui.label(format!("{}%", (self.library.volume * 100.0).round() as u32));
            if ui.add(IconButton::new(if self.video_muted { Icon::Mute } else { Icon::Volume }, ""))
                .on_hover_hint(if self.video_muted { "Unmute" } else { "Mute" }).clicked() {
                self.video_muted = !self.video_muted;
                let volume = if self.video_muted { 0.0 } else { self.library.volume };
                self.player.volume(volume);
            }
            let rate = self.player.video_rate();
            let rate_label = if rate.fract() == 0.0 { format!("{}×", rate as i32) } else { format!("{rate}×") };
            ui.menu_button(rate_label, |ui| {
                for option in [0.5f32, 0.75, 1.0, 1.25, 1.5, 2.0] {
                    let label = if option.fract() == 0.0 { format!("{}×", option as i32) } else { format!("{option}×") };
                    if ui.add(IconButton::new(Icon::Meter, &label).left_aligned().selected(rate == option)).clicked() {
                        self.player.set_video_rate(option);
                        ui.close();
                    }
                }
            }).response.on_hover_hint("Playback speed");
        });
    }

    fn playback_buttons_ui(&mut self, ui: &mut egui::Ui, compact: bool) {
                            if ui
                                .add_enabled(
                                    !self.queue.is_empty(),
                                    IconButton::new(Icon::Previous, "")
                                        .min_size(egui::vec2(if compact { 32.0 } else { 52.0 }, 32.0)),
                                )
                                .on_shortcut_hint("Previous", &["⌘", "←"])
                                .clicked()
                            {
                                self.advance(true, false);
                            }
                            let playing = (self.player.active() && !self.player.paused) || self.smb_connecting;
                            if ui
                                .add(
                                    IconButton::new(
                                        if playing { Icon::Stop } else { Icon::Play },
                                        "",
                                    )
                                    .color(egui::Color32::from_rgb(20, 16, 33))
                                    .fill(theme::ACCENT)
                                    .min_size(egui::vec2(if compact { 44.0 } else { 84.0 }, 40.0)),
                                )
                                .on_hover_hint("Play / stop · Space")
                                .clicked()
                            {
                                self.toggle();
                            }
                            if ui
                                .add_enabled(
                                    !self.queue.is_empty(),
                                    IconButton::new(Icon::Next, "")
                                        .min_size(egui::vec2(if compact { 32.0 } else { 52.0 }, 32.0)),
                                )
                                .on_shortcut_hint("Next", &["⌘", "→"])
                                .clicked()
                            {
                                self.advance(false, false);
                            }
    }

    fn volume_ui(&mut self, ui: &mut egui::Ui, width: f32) {
        ui.spacing_mut().slider_width = width;
        icons::inline(ui, Icon::Volume, theme::MUTED);
        let volume = ui.add(egui::Slider::new(&mut self.library.volume, 0.0..=1.0).show_value(false))
            .on_hover_hint("Volume");
        if volume.changed() { self.video_muted = false; self.player.volume(self.library.volume); }
        if volume.drag_stopped() || (volume.changed() && !volume.dragged()) { self.save(); }
        ui.label(format!("{}%", (self.library.volume * 100.0).round() as u32));
    }

    fn settings_card_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(theme::darker(self.artwork_color, 0.46))
            .corner_radius(12)
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    icons::inline(ui, Icon::Gear, theme::ACCENT);
                    ui.label(egui::RichText::new("Settings").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() { self.settings_open = false; }
                    });
                });
                ui.separator();
                ui.strong("Interface");
                if ui
                    .checkbox(&mut self.library.show_hover_hints, "Show Hover Hints")
                    .changed()
                {
                    hover_hints::set_enabled(ui.ctx(), self.library.show_hover_hints);
                    self.save();
                }
                self.widget_settings_ui(ui);
                self.library_path_settings_ui(ui);

                ui.separator();
                ui.strong("Playback");
                self.playback_settings_ui(ui);
                self.equalizer_settings_ui(ui);
                self.video_title_settings_ui(ui);
                self.visualizer_settings_ui(ui);

                ui.separator();
                ui.strong("YouTube & Downloads");
                self.youtube_settings_ui(ui);
                self.download_settings_ui(ui);

                if let Some(link) = self.current.as_ref().and_then(|s| s.homepage.as_ref()) {
                    ui.separator();
                    ui.hyperlink_to("Current station website ↗", link);
                }
            });
    }
    fn player_ui(&mut self, ui: &mut egui::Ui, compact: bool) {
        // Settings-only view: when the gear icon is clicked, the central
        // area shows nothing but the Settings card — no player, meters,
        // or recent-tracks cards. The left panel stays so the gear icon
        // (and the card's Close button) can turn it off again.
        if self.settings_open {
            egui::ScrollArea::vertical()
                .id_salt("settings-scroll")
                .show(ui, |ui| {
                    let width = ui.available_width().min(960.0);
                    let inset = ((ui.available_width() - width) * 0.5).max(0.0);
                    ui.horizontal(|ui| {
                        ui.add_space(inset);
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| self.settings_card_ui(ui),
                        );
                    });
                });
            return;
        }
        let height = ui.available_height();
        let reserved_height_id = ui.id().with("compact-player-reserved-height");
        let reserved_height = ui.ctx().data_mut(|data| {
            data.get_temp::<f32>(reserved_height_id).unwrap_or(400.0)
        });
        let mut media_height = 0.0;
        let mut inline_track_card_drawn = false;
        let video_panel = self.current_has_video;
        let mut render_player = |ui: &mut egui::Ui| {
                ui.add_space(12.0);
                let meter_height = if if compact { self.library.show_meters_compact } else { self.library.show_meters } { 132.0 } else { 0.0 };
                let size = (ui.available_width() - if compact { 16.0 } else { 36.0 })
                    .min(400.0)
                    .min(if compact {
                        // Reserve the measured space needed by metadata, meters and controls.
                        (height - reserved_height - 16.0).max(48.0)
                    } else {
                        (height - 299.0 - meter_height).max(170.0)
                    });
                ui.vertical_centered(|ui| {
                    let background_width = ui.available_width();
                    // The bars under the video are exactly as wide as the
                    // video itself, centred under it, never wider. This
                    // width must come from the same inline_size fit the
                    // video itself uses, not from the media frame's
                    // response rect: in a centred layout egui expands
                    // that rect to the full frame width, so measuring
                    // it always gave the stack width and an inset of 0.
                    let video_width = if self.current_has_video {
                        let source_size = self.video_texture.as_ref().or(self.artwork.as_ref())
                            .map_or(egui::vec2(16.0, 9.0), |texture| texture.size_vec2());
                        video::inline_size(source_size, video_bounds(background_width, height, compact, reserved_height)).x
                    } else { 0.0 };
                    let video_inset = if self.current_has_video { centred_inset(background_width, video_width) } else { 0.0 };
                    let media_frame = if self.current_has_video {
                        egui::Frame::NONE
                    } else {
                        egui::Frame::new()
                            .fill(theme::darker(self.artwork_color, 0.46))
                            .corner_radius(20)
                            .inner_margin(8.0)
                    };
                    let media_response = media_frame.show(ui, |ui| {
                            let source_size = self.video_texture.as_ref().or(self.artwork.as_ref())
                                .map_or(egui::vec2(16.0, 9.0), |texture| texture.size_vec2());
                            let video_size = video::inline_size(source_size, video_bounds(background_width, height, compact, reserved_height));
                            // Height-limited media keeps playback controls and Settings reachable.
                            ui.set_min_size(if self.current_has_video {
                                video_size
                            } else {
                                egui::vec2((background_width - 16.0).max(size), size)
                            });
                            let now = ui.input(|i| i.time);
                            if self.inline_toggle_at.is_some_and(|deadline| now > deadline) {
                                self.show_inline_visualizer = !self.show_inline_visualizer;
                                self.inline_toggle_at = None;
                            }
                            if self.video_toggle_at.is_some_and(|deadline| now > deadline) {
                                self.video_toggle_at = None;
                                if self.player.active() { self.player.toggle(); }
                                self.update_media();
                            }
                            let is_video = self.current_has_video;
                            let response = if is_video {
                                let (rect, response) = ui.allocate_exact_size(video_size, egui::Sense::click());
                                ui.painter().rect_filled(rect, theme::MEDIA_CORNER_RADIUS, egui::Color32::BLACK);
                                if let Some(texture) = self.video_texture.as_ref().or(self.artwork.as_ref()) {
                                    let fitted = texture.size_vec2() * (rect.width() / texture.size_vec2().x).min(rect.height() / texture.size_vec2().y);
                                    let image_rect = egui::Rect::from_center_size(rect.center(), fitted);
                                    ui.painter().image(texture.id(), image_rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)), egui::Color32::WHITE);
                                } else {
                                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Loading video…", egui::FontId::proportional(16.), egui::Color32::WHITE);
                                }
                                response
                            } else if let Some(texture) = self.artwork.as_ref()
                                .filter(|_| !self.show_inline_visualizer)
                            {
                                // Reserve the full artwork area so non-square covers are
                                // centered vertically as well as horizontally in the card.
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(size, size), egui::Sense::hover(),
                                );
                                ui.put(
                                    rect,
                                    egui::Image::new(texture)
                                        .fit_to_exact_size(egui::vec2(size, size))
                                        .corner_radius(theme::MEDIA_CORNER_RADIUS)
                                        .sense(egui::Sense::click_and_drag()),
                                )
                            } else if !self.show_inline_visualizer {
                                let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click_and_drag());
                                ui.painter().rect_filled(rect, theme::MEDIA_CORNER_RADIUS, theme::darker(self.artwork_color, 0.7));
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "No artwork", egui::FontId::proportional(16.), ui.visuals().weak_text_color());
                                response
                            } else {
                                // The inline visualizer fills the card's inner width;
                                // artwork above stays a centred size x size square.
                                let vis_width = (background_width - 16.0).max(size);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(vis_width, size),
                                    egui::Layout::top_down(egui::Align::Center),
                                    |ui| self.kaleido.draw_inline(ui, theme::darker(self.artwork_color, 0.46)),
                                ).inner
                            };
                            if !is_video {
                                inline_track_card_drawn = self.paint_now_playing_overlay(ui, response.rect);
                            }
                            if is_video {
                                self.inline_toggle_at = None;
                                if response.double_clicked() {
                                    self.video_fullscreen = true;
                                    self.video_controls.reset();
                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                                    self.video_toggle_at = None;
                                } else if response.clicked() {
                                    let delay = ui.ctx().options(|o| o.input_options.max_double_click_delay);
                                    self.video_toggle_at = Some(now + delay);
                                }
                            } else if response.double_clicked() {
                                self.show_inline_visualizer = false;
                                self.visualizer_open = true;
                                self.inline_toggle_at = None;
                            } else if response.clicked() {
                                let delay = ui.ctx().options(|o| o.input_options.max_double_click_delay);
                                self.inline_toggle_at = Some(now + delay);
                            }
                            if let Some(deadline) = self.inline_toggle_at {
                                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(
                                    (deadline - now).max(0.0),
                                ));
                            }
                            if let Some(deadline) = self.video_toggle_at {
                                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(
                                    (deadline - now).max(0.0),
                                ));
                            }
                            let inline_visualizer = !is_video && self.show_inline_visualizer;
                            response.context_menu(|ui| {
                                if is_video {
                                    self.video_context_menu(ui);
                                } else if inline_visualizer {
                                    self.visualizer_context_menu(ui);
                                } else {
                                    self.visualizer_overlay_option(ui);
                                    self.youtube_context_actions(ui);
                                }
                                if compact && icons::menu_item(ui, Icon::Library, "Library · ⌘L").clicked() {
                                    self.compact_library = true;
                                    ui.close();
                                }
                            });
                            response.on_hover_hint(
                                if is_video { "Video playback" } else { "Click to switch artwork/visualizer; double-click to open the visualizer window" },
                            );
                        });
                    media_height = media_response.response.rect.height();
                    if self.current_has_video {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.add_space(video_inset);
                            ui.allocate_ui_with_layout(
                                egui::vec2(video_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    egui::Frame::new().fill(egui::Color32::BLACK).inner_margin(8.0).show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        self.video_controls_ui(ui);
                                    });
                                },
                            );
                        });
                    }
                    if !self.current_has_video && if compact { self.library.show_meters_compact } else { self.library.show_meters } {
                    ui.add_space(12.0);
                    egui::Frame::new()
                        .fill(theme::darker(self.artwork_color, 0.46))
                        .corner_radius(12)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.horizontal(|ui| {
                                    icons::inline(ui, Icon::Meter, theme::ACCENT);
                                    ui.label(egui::RichText::new("Audio Meters").strong());
                                });
                                {
                                    ui.add_space(8.0);
                                    self.kaleido.draw_band_meter(ui, ui.available_width(), self.library.volume);
                                }
                            });
                        });
                    }
                    // Hide title for YouTube and for Local/Remote: for
                    // video the title card is only title and artist, and
                    // playback controls live in the black bar above it,
                    // so skipping the card hides the title with nothing
                    // lost. Audio keeps its card, with only its title
                    // lines hidden below.
                    let hide_youtube_title = self.library.youtube_hide_title
                        && self.current.as_ref().is_some_and(|station| station.favorite_category() == FavoriteCategory::YouTube);
                    if self.current_has_video && self.video_title_hidden() {
                        return;
                    }
                    ui.add_space(12.0);
                    // Same width rule as the video controls above: the
                    // title card is video-wide for video, full width for audio.
                    let card_width = if self.current_has_video { video_width } else { ui.available_width() };
                    let card_inset = if self.current_has_video { video_inset } else { 0.0 };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(card_inset);
                        ui.allocate_ui_with_layout(
                            egui::vec2(card_width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                    egui::Frame::new()
                        .fill(if self.current_has_video { theme::SURFACE } else { theme::darker(self.artwork_color, 0.46) })
                        .corner_radius(12)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.vertical_centered(|ui| {
                    let popup_track_card_visible = self.visualizer_open
                        && self.popup_track_card_drawn
                        && self.library.visualizer_track_overlay
                        && !self.title.trim().is_empty();
                    if !hide_youtube_title && !inline_track_card_drawn && !popup_track_card_visible {
                        self.draw_now_playing_card(ui, None);
                    }
                    if let Some(station) = self.current.as_ref().filter(|s| s.is_file()) {
                        let order = if station.media_source.as_deref() == Some("remoteFile") || station.stream_url.starts_with("smb:") {
                            self.library.remote_sort
                        } else { self.library.local_sort };
                        if order == SortOrder::TrackDiscAscending {
                            let details = [
                                station.track_number.filter(|n| *n > 0).map(|n| format!("Track {n}")),
                                station.disc_number.filter(|n| *n > 0).map(|n| format!("Disc {n}")),
                            ].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                            if !details.is_empty() {
                                ui.label(egui::RichText::new(details).small().color(theme::MUTED));
                            }
                        }
                    }
                    if !self.current_has_video {
                    ui.add_space(12.0);
                    let controls_width = if compact {
                        ui.available_width()
                    } else {
                        ui.available_width().min(480.0)
                    };
                    let controls = ui.allocate_ui_with_layout(
                        egui::vec2(controls_width, 40.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            self.playback_buttons_ui(ui, compact);
                            let equalizer_button_width = 28.0_f32.max(
                                16.0 + 2.0 * ui.spacing().button_padding.x,
                            );
                            let width = (ui.available_width() - 64.0
                                - equalizer_button_width - ui.spacing().item_spacing.x)
                                .max(24.0);
                            self.volume_ui(ui, width);
                            let eq_enabled = self.library.equalizer.enabled;
                            if ui
                                .add(IconButton::new(Icon::Equalizer, "")
                                    .selected(eq_enabled)
                                    .min_size(egui::vec2(equalizer_button_width, 28.0)))
                                .on_hover_hint(if eq_enabled { "Equalizer is on" } else { "Equalizer is off" })
                                .clicked()
                            {
                                self.settings_open = true;
                                self.reveal_equalizer = true;
                                ui.ctx().request_repaint();
                            }
                            ui.min_rect()
                        },
                    );
                    ui.add_space(8.0);
                    let slider_row_width = controls.inner.width();
                    if self.duration > 0.0 && (self.video_seekable || (!self.current_has_video && self.current.as_ref().is_some_and(Station::is_file))) {
                        let mut position = self.seek_position.unwrap_or_else(|| if self.player.active() {
                            self.playback_position()
                        } else {
                            self.current
                                .as_ref()
                                .and_then(|s| self.library.local_positions.get(&s.stream_url))
                                .copied()
                                .unwrap_or(0.)
                        });
                        let seek = ui
                            .allocate_ui_with_layout(
                                egui::vec2(controls_width, 28.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().slider_width = slider_row_width;
                                    ui.add(
                                        egui::Slider::new(&mut position, 0.0..=self.duration)
                                            .show_value(false),
                                    )
                                    .on_hover_hint("Playback position")
                                },
                            )
                            .inner;
                        ui.allocate_ui_with_layout(
                            egui::vec2(controls_width, 16.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_width(slider_row_width);
                                let time = |seconds: f64| format!("{}:{:02}", (seconds / 60.0) as u32, (seconds % 60.0) as u32);
                                ui.label(egui::RichText::new(time(position)).small().color(theme::MUTED));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(egui::RichText::new(time(self.duration)).small().color(theme::MUTED));
                                });
                            },
                        );
                        if seek.dragged() { self.seek_position = Some(position); }
                        if seek.drag_stopped() || (seek.changed() && !seek.dragged()) {
                            self.seek_position = None;
                            self.library.local_positions.insert(
                                self.current.as_ref().unwrap().stream_url.clone(),
                                position,
                            );
                            if self.player.active() {
                                self.seek_playback(position);
                            }
                            self.save();
                        }
                    } else if self.player.video.is_some() {
                        ui.label(egui::RichText::new("Video · seeking unavailable").small().color(theme::MUTED));
                    } else if let Some(station) = self.current.as_ref().filter(|s| !s.is_file()) {
                        let favorite = self.library.stations.iter()
                            .find(|saved| saved.stream_url == station.stream_url);
                        live_radio_badge(
                            ui,
                            station,
                            favorite,
                            &mut self.card_artwork,
                            self.player.active() && !self.player.paused,
                        );
                    }
                    }
                            });
                        });
                            },
                        );
                    });
                });
                if !self.current_has_video && (if compact { self.library.show_recent_tracks_compact } else { self.library.show_recent_tracks }) && self.current.as_ref().is_some_and(|s| !s.is_file()) && self.player.active() {
                    ui.add_space(12.0);
                    egui::Frame::new()
                        .fill(theme::darker(self.artwork_color, 0.46))
                        .corner_radius(12)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                icons::inline(ui, Icon::History, theme::ACCENT);
                                ui.label(egui::RichText::new("Recent Tracks").strong());
                            });
                            if self.history.is_empty() {
                                ui.label(
                                    egui::RichText::new("Track titles will appear here as you listen.")
                                        .color(theme::MUTED),
                                );
                            } else {
                                egui::ScrollArea::vertical()
                                    .id_salt("recent-tracks-scroll")
                                    .max_height(recent_tracks_max_height(compact))
                                    .min_scrolled_height(recent_tracks_max_height(compact))
                                    .auto_shrink([false, false])
                                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                                    .scroll_source(egui::scroll_area::ScrollSource::ALL)
                                    .show(ui, |ui| {
                                        for track in &self.history {
                                            ui.horizontal(|ui| {
                                                if let Some(texture) = &track.artwork {
                                                    ui.add(egui::Image::new((texture.id(), egui::vec2(36.0, 36.0))).corner_radius(4));
                                                } else {
                                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
                                                    ui.painter().rect_filled(rect, 4, theme::SURFACE);
                                                    icons::draw(ui.painter(), Icon::History, rect.shrink(8.0), theme::MUTED);
                                                }
                                                ui.add(egui::Label::new(&track.title).wrap());
                                            });
                                        }
                                    });
                            }
                        });
                }
        };
        let content_height = if compact {
            // Compact playback is centred the same way, with the top
            // space kept out of the stack height used for media sizing.
            let stack_height_id = ui.id().with("player-stack-height");
            let previous_height = ui.ctx().data_mut(|data| data.get_temp::<f32>(stack_height_id));
            let top_inset = vertical_top_inset(height, previous_height);
            let mut stack_height = 0.0;
            ui.vertical(|ui| {
                if top_inset > 0.0 {
                    ui.add_space(top_inset);
                }
                stack_height = ui.scope(|ui| render_player(ui)).response.rect.height();
            });
            ui.ctx().data_mut(|data| data.insert_temp(stack_height_id, stack_height));
            if previous_height.is_none_or(|previous| (previous - stack_height).abs() > 0.5) {
                ui.ctx().request_repaint();
            }
            stack_height
        } else {
            let stack_height_id = ui.id().with("player-stack-height");
            let previous_height = ui.ctx().data_mut(|data| data.get_temp::<f32>(stack_height_id));
            let top_inset = vertical_top_inset(height, previous_height);
            let mut measured_height = 0.0;
            let output = egui::ScrollArea::vertical()
                .id_salt("player-scroll")
                .show(ui, |ui| {
                    if top_inset > 0.0 {
                        ui.add_space(top_inset);
                    }
                    // Keep audio cards readable on wide windows and full-screen displays.
                    // Video stretches to the panel width instead, so its
                    // controls below it line up with the video.
                    let width = if video_panel { ui.available_width() } else { ui.available_width().min(960.0) };
                    let inset = ((ui.available_width() - width) * 0.5).max(0.0);
                    let content_spacing = ui.spacing().item_spacing;
                    let stack = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(inset);
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.spacing_mut().item_spacing = content_spacing;
                                render_player(ui);
                            },
                        );
                    });
                    measured_height = stack.response.rect.height();
                });
            ui.ctx().data_mut(|data| data.insert_temp(stack_height_id, measured_height));
            if previous_height.is_none_or(|previous| (previous - measured_height).abs() > 0.5) {
                ui.ctx().request_repaint();
            }
            output.content_size.y
        };
        if compact {
            let measured = (content_height - media_height).max(0.0);
            ui.ctx().data_mut(|data| data.insert_temp(reserved_height_id, measured));
            if (measured - reserved_height).abs() > 0.5 {
                ui.ctx().request_repaint();
            }
        }
    }
    fn youtube_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("YouTube & YouTube Music", |ui| {
            let previous_quality = self.library.youtube_quality;
            let previous_frame_rate = self.library.youtube_frame_rate;
            let mut changed = false;

            ui.strong("Search");
            ui.horizontal_wrapped(|ui| {
                ui.label("Result Limit");
                let mut unlimited = self.library.youtube_search_limit == 0;
                if ui.checkbox(&mut unlimited, "Unlimited").changed() {
                    self.library.youtube_search_limit = if unlimited { 0 } else { 20 };
                    changed = true;
                }
                if !unlimited {
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.library.youtube_search_limit).range(1..=u32::MAX))
                        .changed();
                }
            });
            ui.small("Applies to YouTube and YouTube Music. Unlimited collects all available results.");

            ui.separator();
            ui.strong("YouTube Playback");
            self.playback_mode_ui(ui, FavoriteCategory::YouTube, false);
            ui.horizontal_wrapped(|ui| {
                ui.label("Preferred Quality");
                egui::ComboBox::from_id_salt("preferred-youtube-quality")
                    .selected_text(self.library.youtube_quality.label())
                    .show_ui(ui, |ui| {
                        for quality in youtube::Quality::ALL {
                            icons::menu_choice(ui, &mut self.library.youtube_quality, quality, Icon::Fullscreen, quality.label());
                        }
                    });
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Frame Rate");
                egui::ComboBox::from_id_salt("preferred-youtube-frame-rate")
                    .selected_text(self.library.youtube_frame_rate.label())
                    .show_ui(ui, |ui| {
                        for frame_rate in youtube::FrameRate::ALL {
                            icons::menu_choice(ui, &mut self.library.youtube_frame_rate, frame_rate, Icon::Meter, frame_rate.label());
                        }
                    });
            });
            ui.small("Quality and frame rate cap the best available stream and apply to the next video.");
            changed |= ui
                .checkbox(&mut self.library.youtube_hide_title, "Hide the Title Under the Video Player")
                .changed();

            ui.separator();
            ui.strong("YouTube Music Playback");
            self.playback_mode_ui(ui, FavoriteCategory::YouTubeMusic, false);
            ui.small("Also applies to other video streams. Audio mode shows artwork, meters, and audio controls.");

            if previous_quality != self.library.youtube_quality || previous_frame_rate != self.library.youtube_frame_rate {
                self.clear_youtube_preload();
                changed = true;
            }
            if changed {
                self.save();
            }
        });
    }
    fn download_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Downloads", |ui| {
            if let Some(directory) = &self.library.download_directory {
                ui.label(directory.display().to_string());
            } else {
                ui.label("Ask Where to Save Each Download");
            }
            ui.horizontal(|ui| {
                if icons::menu_item(ui, Icon::Folder, "Choose Folder…").clicked() {
                    let mut dialog = rfd::FileDialog::new();
                    if let Some(directory) = &self.library.download_directory {
                        dialog = dialog.set_directory(directory);
                    }
                    if let Some(directory) = dialog.pick_folder() {
                        self.library.download_directory = Some(directory);
                        self.save();
                    }
                }
                if self.library.download_directory.is_some()
                    && icons::menu_item(ui, Icon::Refresh, "Ask Each Time").clicked()
                {
                    self.library.download_directory = None;
                    self.save();
                }
            });
        });
    }

    fn video_title_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Local & Remote Video", |ui| {
            if ui.checkbox(&mut self.library.local_remote_hide_title, "Hide the Title Under the Video Player").changed() {
                self.save();
            }
            ui.small("Hide title keeps playback controls in the black bar above the title. Applies to video files from Local and Remote only.");
        });
    }
    fn library_path_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Library Paths", |ui| {
            if ui.checkbox(&mut self.library.show_local_path, "Show Current Local Path")
                .on_hover_hint("Display the path of the local folder being browsed.")
                .changed()
            {
                self.save();
            }
            if ui.checkbox(&mut self.library.show_remote_path, "Show Current Server Path")
                .on_hover_hint("Display the full URL of the folder being browsed.")
                .changed()
            {
                self.save();
            }
        });
    }
    fn equalizer_settings_ui(&mut self, ui: &mut egui::Ui) {
        let reveal = std::mem::take(&mut self.reveal_equalizer);
        let header = egui::CollapsingHeader::new("Equalizer").open(reveal.then_some(true));
        let section = header.show(ui, |ui| {
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("Enable Equalizer");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    changed |= icons::slide_toggle(ui, &mut self.library.equalizer.enabled, "Equalizer").changed();
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Preset");
                let settings = &mut self.library.equalizer;
                let selected = equalizer::PRESETS
                    .iter()
                    .position(|(_, bands)| settings.bands == *bands && settings.preamp == 0.);
                let label = selected.map_or("Custom", |index| equalizer::PRESETS[index].0);
                egui::ComboBox::from_id_salt("equalizer-preset")
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for (index, (name, bands)) in equalizer::PRESETS.iter().enumerate() {
                            if ui.selectable_label(selected == Some(index), *name).clicked() {
                                settings.bands = *bands;
                                settings.preamp = 0.;
                                changed = true;
                                ui.close();
                            }
                        }
                    });
            });
            ui.add_enabled_ui(self.library.equalizer.enabled, |ui| {
                egui::ScrollArea::horizontal().id_salt("equalizer-bands").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (index, label) in equalizer::LABELS.iter().enumerate() {
                            ui.allocate_ui_with_layout(egui::vec2(30., 148.), egui::Layout::top_down(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(format!("{:+.0}", self.library.equalizer.bands[index])).small());
                                ui.spacing_mut().slider_width = 100.;
                                changed |= ui.add(egui::Slider::new(&mut self.library.equalizer.bands[index], -12.0..=12.0)
                                    .vertical().show_value(false)).on_hover_hint(format!("{label} Hz · gain in dB")).changed();
                                ui.label(egui::RichText::new(*label).small());
                            });
                        }
                    });
                });
                changed |= ui.add(egui::Slider::new(&mut self.library.equalizer.preamp, -12.0..=12.0).text("Preamp").suffix(" dB").fixed_decimals(2)).changed();
            });
            if icons::menu_item(ui, Icon::Refresh, "Reset to Flat").clicked() {
                self.library.equalizer.bands = [0.; 10];
                self.library.equalizer.preamp = 0.;
                changed = true;
            }
            ui.label(egui::RichText::new("Audio playback only · automatic headroom for boosts").small().color(theme::MUTED));
            if changed {
                self.player.set_equalizer(self.library.equalizer);
                self.save();
            }
        });
        if reveal {
            section.header_response.scroll_to_me(Some(egui::Align::TOP));
        }
    }
    fn widget_settings_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.collapsing("Player Widgets & Library Tabs", |ui| {
            egui::Grid::new("widget-visibility")
                .spacing(egui::vec2(20.0, 8.0))
                .show(ui, |ui| {
                ui.label("Widget");
                ui.label("Compact View");
                ui.label("Full View");
                ui.end_row();
                ui.label("Recent Tracks");
                changed |= ui.checkbox(&mut self.library.show_recent_tracks_compact, "Show").changed();
                changed |= ui.checkbox(&mut self.library.show_recent_tracks, "Show").changed();
                ui.end_row();
                ui.label("Audio Meters");
                changed |= ui.checkbox(&mut self.library.show_meters_compact, "Show").changed();
                changed |= ui.checkbox(&mut self.library.show_meters, "Show").changed();
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.label("Library Tabs");
            changed |= ui.checkbox(&mut self.library.show_local_tab, "Show Local").changed();
            changed |= ui.checkbox(&mut self.library.show_remote_tab, "Show Remote").changed();
            changed |= ui.checkbox(&mut self.library.show_youtube_tab, "Show YouTube").changed();
            changed |= ui.checkbox(&mut self.library.show_youtube_music_tab, "Show YouTube Music").changed();
        });
        if changed {
            if (self.tab == Tab::Local && !self.library.show_local_tab)
                || (self.tab == Tab::Remote && !self.library.show_remote_tab)
            || (self.tab == Tab::YouTube && !self.library.show_youtube_tab)
            || (self.tab == Tab::YouTubeMusic && !self.library.show_youtube_music_tab)
            {
                self.tab = Tab::Favorites;
            }
            self.save();
        }
    }
    fn playback_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Audio Playback", |ui| {
            ui.horizontal(|ui| {
                ui.label("Auto Volume");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::slide_toggle(ui, &mut self.library.auto_volume, "Auto Volume").changed() {
                        self.player.set_auto_volume(self.library.auto_volume);
                        self.save();
                    }
                });
            });
            ui.label(egui::RichText::new("Smoothly levels radio and audio tracks as they play.").small().color(theme::MUTED));
            ui.separator();
            let mut changed = false;
            for (label, crossfade) in [("Gapless Playback", false), ("Crossfade Playback", true)] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let enabled = if crossfade { &mut self.library.crossfade_playback } else { &mut self.library.gapless_playback };
                        changed |= icons::slide_toggle(ui, enabled, label).changed();
                    });
                });
            }
            if self.library.crossfade_playback {
                changed |= ui.add(egui::Slider::new(&mut self.library.crossfade_seconds, 1.0..=12.0).text("Fade Duration").suffix(" s")).changed();
            }
            ui.label(egui::RichText::new("Applies between queued audio files. Crossfade takes priority when both are on.").small().color(theme::MUTED));
            if changed {
                self.clear_youtube_preload();
                self.cancel_transition();
                self.save();
            }
        });
    }
    fn prepare_transition(&mut self) {
        if self.current.as_ref().is_some_and(|station| youtube::is_url(&station.stream_url)) {
            self.prepare_youtube_preload();
            return;
        }
        self.clear_youtube_preload();
        let settings = (self.library.gapless_playback, self.library.crossfade_playback,
            self.library.crossfade_seconds, self.library.shuffle, self.library.repeat);
        if self.transition_settings.as_ref() != Some(&settings) {
            self.cancel_transition();
            self.transition_settings = Some(settings);
        }
        if self.transition_next.is_some() { return; }
        if (!settings.0 && !settings.1) || self.current_has_video || !self.player.active() { return; }
        let fade = if settings.1 { settings.2.clamp(1., 12.) } else { 0. };
        if self.duration <= 0. { return; }
        let Some(current) = self.current.as_ref().filter(|s| s.is_file()) else { return; };
        let Some(index) = self.queue.iter().position(|s| s.id == current.id) else { return; };
        let next = if self.library.repeat == Repeat::One {
            index
        } else if self.library.shuffle && self.queue.len() > 1 {
            let Some(next) = self.next_shuffle_index(index) else { return; };
            next
        } else if index + 1 < self.queue.len() {
            index + 1
        } else if self.library.repeat == Repeat::All {
            0
        } else { return; };
        let station = self.queue[next].clone();
        if !station.is_file() || video::is_video_url(&station.stream_url) { return; }
        let generation = self.transition_generation;
        self.transition_next = Some(station.clone());
        let tx = self.tx.clone();
        let auth = self.auth.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let address = if station.stream_url.starts_with("smb:") {
                    smb::resolve(&station.stream_url, auth.as_ref())?
                } else if station.stream_url.starts_with("sftp:") {
                    sftp::resolve(&station.stream_url, auth.as_ref())?
                } else { station.stream_url };
                audio::Prepared::open(station.id, &address,
                    auth.as_ref().map(|(user, password)| (user.as_str(), password.as_str())), fade)
            })();
            let _ = tx.send(TaskResult::PreparedTransition(generation, station.id, result));
        });
    }
    fn now_playing_text(&self) -> (Option<&str>, &str) {
        if self.title.is_empty() {
            (None, "Ready to listen")
        } else if let Some(artist) = self.current.as_ref().and_then(|station| station.artist.as_deref()) {
            let title = self.title.strip_prefix(&format!("{artist} - ")).unwrap_or(&self.title);
            (Some(artist), title)
        } else if let Some((artist, title)) = self.title.split_once(" - ") {
            (Some(artist), title)
        } else {
            (None, self.title.as_str())
        }
    }

    fn paint_now_playing_overlay(&self, ui: &mut egui::Ui, rect: egui::Rect) -> bool {
        if !self.library.visualizer_track_overlay || self.title.trim().is_empty() {
            return false;
        }
        let bounds = rect.intersect(ui.clip_rect()).shrink(12.0);
        if bounds.width() < 96.0 || bounds.height() < 32.0 {
            return false;
        }
        self.draw_now_playing_card(ui, Some(bounds));
        true
    }

    fn draw_now_playing_card(&self, ui: &mut egui::Ui, overlay_bounds: Option<egui::Rect>) {
        let available_width = overlay_bounds.map_or_else(|| ui.available_width(), |rect| rect.width());
        let (artist, title) = self.now_playing_text();
        let padding = 8.0;
        let artwork_size = if self.artwork.is_some() {
            let max_height = overlay_bounds.map_or(44.0, |rect| rect.height() - 2.0 * padding);
            max_height.min(44.0).min((available_width - 48.0).max(0.0))
        } else {
            0.0
        };
        let text_offset = if artwork_size > 0.0 { artwork_size + padding } else { 0.0 };
        let painter = ui.painter();
        let text_width = (available_width - 2.0 * padding - text_offset).max(1.0);
        let title = painter.layout(
            title.to_owned(),
            egui::FontId::proportional(16.0),
            egui::Color32::WHITE,
            text_width,
        );
        let artist = artist.filter(|artist| !artist.trim().is_empty()).map(|artist| {
            painter.layout(
                artist.to_owned(),
                egui::FontId::proportional(13.0),
                theme::MUTED,
                text_width,
            )
        });
        let gap = 2.0;
        let text_height = title.size().y + artist.as_ref().map_or(0.0, |line| gap + line.size().y);
        let text_width = title.size().x.max(artist.as_ref().map_or(0.0, |line| line.size().x));
        let size = egui::vec2(
            text_width + text_offset + 2.0 * padding,
            text_height.max(artwork_size) + 2.0 * padding,
        );
        let card = if let Some(bounds) = overlay_bounds {
            egui::Rect::from_min_size(bounds.left_bottom() - egui::vec2(0.0, size.y), size)
        } else {
            ui.allocate_exact_size(size, egui::Sense::hover()).0
        };
        let painter = overlay_bounds.map_or_else(|| ui.painter().clone(), |bounds| ui.painter_at(bounds));
        painter.rect_filled(card, 10.0, egui::Color32::from_black_alpha(215));
        if let Some(artwork) = self.artwork.as_ref().filter(|_| artwork_size > 0.0) {
            let source = artwork.size_vec2();
            let fitted = source * (artwork_size / source.x.max(source.y));
            let center = egui::pos2(card.left() + padding + artwork_size * 0.5, card.center().y);
            painter.image(
                artwork.id(),
                egui::Rect::from_center_size(center, fitted),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        let text_pos = egui::pos2(
            card.left() + padding + text_offset,
            card.center().y - text_height * 0.5,
        );
        let artist_pos = text_pos + egui::vec2(0.0, title.size().y + gap);
        painter.galley(text_pos, title, egui::Color32::WHITE);
        if let Some(artist) = artist {
            painter.galley(artist_pos, artist, theme::MUTED);
        }
    }

    fn current_is_youtube(&self) -> bool {
        self.current.as_ref().is_some_and(|station| youtube::is_url(&station.stream_url))
    }

    fn video_title_hidden(&self) -> bool {
        if self.current_is_youtube() {
            self.library.youtube_hide_title
        } else {
            self.library.local_remote_hide_title
        }
    }

    fn video_context_menu(&mut self, ui: &mut egui::Ui) {
        let hidden = self.video_title_hidden();
        let label = if hidden { "Show Title" } else { "Hide Title" };
        if icons::menu_item(ui, Icon::Music, label).clicked() {
            if self.current_is_youtube() {
                self.library.youtube_hide_title = !hidden;
            } else {
                self.library.local_remote_hide_title = !hidden;
            }
            self.save();
            ui.close();
        }
        self.youtube_context_actions(ui);
    }

    fn youtube_context_actions(&mut self, ui: &mut egui::Ui) {
        if !self.current_is_youtube() {
            return;
        }
        let Some(category) = self.current.as_ref().map(Station::favorite_category) else { return };
        ui.menu_button("Playback Mode", |ui| self.playback_mode_ui(ui, category, true));
        let busy = self.youtube_download.is_some();
        let label = if busy { "Download in Progress…" } else { "Download" };
        if ui.add_enabled(!busy, IconButton::new(Icon::Export, label).left_aligned()).clicked() {
            ui.close();
            self.download_current_youtube(ui.ctx());
        }
    }

    fn download_progress_ui(&self, ui: &mut egui::Ui) {
        if self.youtube_download.is_none() {
            return;
        }
        let progress = self.youtube_download_progress;
        let label = if progress >= 1.0 {
            "Finishing Download · Merging and Embedding Artwork".to_owned()
        } else {
            format!("Downloading Stream · {:.0}%", progress * 100.0)
        };
        ui.add(egui::ProgressBar::new(progress)
            .desired_width(280.0_f32.min(ui.available_width()))
            .animate(true)
            .text(label));
    }

    fn download_current_youtube(&mut self, ctx: &egui::Context) {
        let Some(station) = &self.current else { return };
        let audio_only = station.favorite_category() == FavoriteCategory::YouTubeMusic;
        let extension = if audio_only { "m4a" } else { "mp4" };
        let title: String = station.name.chars()
            .filter(|c| !c.is_control() && !matches!(c, '/' | ':' | '\\'))
            .take(60)
            .collect();
        let title = title.trim().trim_matches('.');
        let title = if title.is_empty() { "YouTube Download" } else { title };
        let file_name = format!("{title}.{extension}");
        let automatic = self.library.download_directory.is_some();
        let destination = if let Some(directory) = &self.library.download_directory {
            directory.join(file_name)
        } else {
            let Some(path) = rfd::FileDialog::new()
                .set_file_name(file_name)
                .add_filter(if audio_only { "M4A Audio" } else { "MP4 Video" }, &[extension])
                .save_file()
            else { return };
            path
        };

        let address = station.stream_url.clone();
        let quality = self.library.youtube_quality;
        let frame_rate = self.library.youtube_frame_rate;
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        self.youtube_download_progress = 0.0;
        self.status = "Downloading from YouTube…".into();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let progress_tx = tx.clone();
        let progress_ctx = ctx.clone();
        let progress = std::sync::Arc::new(move |value| {
            let _ = progress_tx.send(TaskResult::YouTubeDownloadProgress(value));
            progress_ctx.request_repaint();
        });
        let worker = std::thread::spawn(move || {
            let result = youtube::save_download(&address, quality, frame_rate, &worker_cancel, audio_only, &destination, automatic, progress);
            let _ = tx.send(TaskResult::YouTubeDownloaded(result));
            ctx.request_repaint();
        });
        self.youtube_download = Some((cancel, worker));
    }

    fn visualizer_overlay_option(&mut self, ui: &mut egui::Ui) {
        if ui.add(
            IconButton::new(Icon::Music, "Show Now-Playing Card")
                .left_aligned()
                .selected(self.library.visualizer_track_overlay),
        ).clicked() {
            self.library.visualizer_track_overlay = !self.library.visualizer_track_overlay;
            self.save();
        }
    }

    fn visualizer_context_menu(&mut self, ui: &mut egui::Ui) {
        if self.kaleido.visualizer_choices(ui) {
            self.save();
            ui.close();
            return;
        }
        ui.separator();
        self.visualizer_overlay_option(ui);
        if self.kaleido.milkdrop_preset_menu(ui) {
            self.save();
            ui.close();
        }
        self.youtube_context_actions(ui);
    }

    fn visualizer_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Visualizer", |ui| {
            self.visualizer_overlay_option(ui);
            if ui
                .add(
                    egui::Slider::new(&mut self.library.sensitivity, 0.2..=4.0)
                        .text("Audio Sensitivity"),
                )
                .changed()
            {
                self.save();
            }
            ui.separator();
            self.kaleido.controls(ui);
        });
    }

    fn update_media(&self) {
        self.media.update(
            self.player.active() && !self.player.paused,
            &self.title,
            self.current.as_ref().map(|s| s.name.as_str()).unwrap_or(""),
            self.current.as_ref().is_none_or(|s| !s.is_file()) && !(self.current_has_video && self.duration > 0.),
            self.playback_position(),
            self.duration,
        );
    }
    fn stop(&mut self) {
        self.clear_youtube_preload();
        self.clear_retired_preloads();
        self.music_radio = None;
        self.cancel_transition();
        self.smb_connecting = false;
        self.playback_generation += 1;
        self.snapshot_position();
        self.retry_at = None;
        self.player.stop();
        self.youtube_buffer = None;
        self.youtube_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.youtube_stream = None;
        self.youtube_resume_paused = false;
        self.video_toggle_at = None;
        self.metrics = audio::Metrics::default();
        self.status = "Stopped".into();
        self.update_media();
        self.save();
    }
    fn resume(&mut self) {
        if self.player.active() || self.smb_connecting {
            return;
        }
        self.retry_at = None;
        self.retries = 0;
        if let Some(station) = self.current.as_ref() {
            let offset = if station.is_file() || self.current_has_video {
                self.library
                    .local_positions
                    .get(&station.stream_url)
                    .copied()
                    .unwrap_or(0.)
            } else {
                0.
            };
            self.start_playback(offset);
        } else if let Some(station) = self.library.stations.first().cloned() {
            self.play(station, self.library.stations.clone(), None, 0.0);
        }
        self.update_media();
    }
    fn toggle(&mut self) {
        if self.player.video.is_some() && self.player.paused {
            self.player.toggle();
            self.update_media();
            return;
        }
        if self.player.active() || self.retry_at.is_some() || self.smb_connecting {
            self.stop();
        } else {
            self.resume();
        }
    }
    fn editor(&mut self, ctx: &egui::Context) {
        let Some(mut station) = self.edit.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        egui::Window::new(if self.edit_existing {
            "Edit station"
        } else {
            "Add station"
        })
        .open(&mut open)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut station.name);
            ui.label("Stream URL");
            ui.text_edit_singleline(&mut station.stream_url);
            ui.label("Homepage");
            ui.text_edit_singleline(station.homepage.get_or_insert_default());
            ui.label("Artwork URL");
            ui.text_edit_singleline(station.favicon_url.get_or_insert_default());
            if icons::button(ui, Icon::Save, "Save").clicked() {
                save = true;
            }
        });
        if save {
            station.normalize();
            match station.validate() {
                Ok(()) => {
                    self.library.stations.retain(|s| s.id != station.id);
                    self.library.add(station);
                    self.save();
                }
                Err(e) => {
                    self.status = e;
                    self.edit = Some(station);
                }
            }
        } else if open {
            self.edit = Some(station);
        }
    }
}
impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if visualizer_window::handle_close(ctx, &mut self.visualizer_open) {
            self.show_inline_visualizer = false;
            self.popup_track_card_drawn = false;
            self.inline_toggle_at = None;
        }
    }
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Some((cancel, worker)) = self.youtube_download.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = worker.join();
        }
        if let Some(gl) = gl { self.kaleido.destroy_renderer(gl); }
        self.clear_youtube_preload();
        self.clear_retired_preloads();
        self.save();
        self.player.stop();
        self.youtube_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.youtube_search.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.youtube_music_search.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.music_browser.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.music_radio = None;
        self.youtube_stream = None;
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        if self.media_updated.elapsed() >= Duration::from_millis(250) {
            self.update_media();
            self.media_updated = Instant::now();
        }
        if self.settings_checked.elapsed() >= Duration::from_secs(2) {
            self.save();
            self.settings_checked = Instant::now();
        }
        self.kaleido.update(
            self.metrics,
            self.started.elapsed().as_secs_f64(),
            self.library.sensitivity,
            self.player.active() && !self.player.paused,
        );
        if !ctx.egui_wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                self.toggle();
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::ArrowRight)) {
                self.advance(false, false);
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::ArrowLeft)) {
                self.advance(true, false);
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::L)) {
                self.compact_library = !self.compact_library;
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Num1)) {
                self.tab = Tab::Favorites;
                self.compact_library = true;
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Num2)) {
                self.tab = Tab::Discover;
                self.compact_library = true;
                if self.results.is_empty() {
                    self.search();
                }
            }
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
            self.add_folder();
        }
        if self.video_fullscreen && (ctx.input(|i| i.key_pressed(egui::Key::Escape)) || !self.current_has_video) {
            self.video_fullscreen = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        // Esc leaves fullscreen or maximized (zoomed) for the main window.
        // Both commands are no-ops when windowed, and the maximized flag
        // is not updated at runtime on macOS, so this is deliberately not
        // gated on reported state.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
        }
        // Plain arrows scrub the video by 5 seconds. Command-arrows stay
        // on Previous/Next track, and typing keeps the keys for editing.
        if !ctx.egui_wants_keyboard_input()
            && !ctx.input(|i| i.modifiers.command)
            && self.player.video.is_some()
            && self.video_seekable
            && self.seek_position.is_none()
        {
            let delta = if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                Some(-5.0)
            } else if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                Some(5.0)
            } else {
                None
            };
            if let Some(delta) = delta {
                let target = (self.playback_position() + delta).clamp(0.0, self.duration);
                self.seek_playback(target);
            }
        }
        if self.video_fullscreen {
            let now = ctx.input(|i| i.time);
            let interaction = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO
                || i.pointer.any_down() || i.events.iter().any(|event|
                    matches!(event, egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }
                        | egui::Event::Key { .. } | egui::Event::MouseWheel { .. })));
            let opacity = self.video_controls.opacity(now, interaction || self.youtube_download.is_some());
            let video_rect = ui.available_rect_before_wrap();
            // Fit to the full window rect, not to whatever the panel
            // happens to offer, so a 16:9 video on a 16:9 monitor
            // stretches to the monitor's full width, with no side bars
            // left over from a menu bar or panel margin.
            let fullscreen_size = self.video_texture.as_ref().or(self.artwork.as_ref())
                .map(|texture| video::inline_size(texture.size_vec2(), video_rect.size()))
                .unwrap_or(video_rect.size());
            let fullscreen_left = video_rect.left() + (video_rect.width() - fullscreen_size.x) * 0.5;
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(egui::Color32::BLACK)).show(ui, |ui| {
                let response = ui.centered_and_justified(|ui| {
                    if let Some(texture) = self.video_texture.as_ref().or(self.artwork.as_ref()) {
                        ui.add(egui::Image::new(texture).fit_to_exact_size(fullscreen_size).sense(egui::Sense::click()))
                    } else {
                        ui.add(egui::Label::new("Loading video…").sense(egui::Sense::click()))
                    }
                }).inner;
                response.context_menu(|ui| self.video_context_menu(ui));
            });
            if opacity > 0.0 {
                egui::Area::new(egui::Id::new("fullscreen-video-controls"))
                    .order(egui::Order::Foreground)
                    .pivot(egui::Align2::LEFT_BOTTOM)
                    .fixed_pos(egui::pos2(fullscreen_left, video_rect.bottom()))
                    .fade_in(false)
                    .show(&ctx, |ui| {
                        ui.set_opacity(opacity);
                        egui::Frame::new().fill(egui::Color32::from_black_alpha(220))
                            .inner_margin(8.0).show(ui, |ui| {
                                // Inner margin adds 16 to the total, so
                                // this makes the bar exactly video-wide.
                                ui.set_width((fullscreen_size.x - 16.0).max(1.0));
                                self.video_controls_ui(ui);
                                if !self.video_title_hidden() {
                                    ui.label(&self.title);
                                }
                                self.download_progress_ui(ui);
                            });
                    });
            }
            ctx.request_repaint_after(Duration::from_millis(16));
            return;
        }
        egui::Panel::bottom("status").show(ui, |ui| {
            self.download_progress_ui(ui);
            ui.horizontal_wrapped(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::Vec2::splat(8.0), egui::Sense::hover());
                let color = if self.player.active() && !self.player.paused {
                    theme::ACCENT
                } else {
                    theme::MUTED
                };
                ui.painter().circle_filled(dot.center(), 3.0, color);
                ui.label(
                    egui::RichText::new(&self.status)
                        .small()
                        .color(theme::MUTED),
                );
                if self.status == "Connecting…" {
                    ui.spinner();
                }
            });
        });
        if ui.available_width() > 820.0 {
            egui::Panel::left("library-v2")
                .resizable(true)
                .default_size(350.0)
                .size_range(300.0..=520.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::darker(self.artwork_color, 0.56))
                        .inner_margin(16.0),
                )
                .show(ui, |ui| {
                    theme::background(ui, self.artwork_color, true, 16.0);
                    self.library_ui(ui);
                });
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(if self.current_has_video { theme::SURFACE } else { theme::darker(self.artwork_color, 0.32) })
                        .inner_margin(24.0),
                )
                .show(ui, |ui| {
                    if !self.current_has_video { theme::background(ui, self.artwork_color, false, 24.0); }
                    self.player_ui(ui, false);
                });
        } else {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(if self.current_has_video { theme::SURFACE } else { theme::darker(self.artwork_color, 0.32) })
                        .inner_margin(16.0),
                )
                .show(ui, |ui| {
                    if self.compact_library || !self.current_has_video {
                        theme::background(ui, self.artwork_color, self.compact_library, 16.0);
                    }
                    if self.compact_library {
                        self.library_ui(ui);
                    } else {
                        self.player_ui(ui, true);
                    }
                });
        }
        self.editor(&ctx);
        if self.visualizer_open {
            ctx.show_viewport_immediate(
                visualizer_window::id(),
                egui::ViewportBuilder::default()
                    .with_title("Kaleido Visualizer")
                    .with_inner_size([900.0, 700.0]),
                |ctx, _class| {
                    if visualizer_window::handle_close(ctx, &mut self.visualizer_open) {
                        self.show_inline_visualizer = false;
                        self.popup_track_card_drawn = false;
                        self.inline_toggle_at = None;
                        return;
                    }
                    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            let response = self.kaleido.draw(ui);
                            self.popup_track_card_drawn = self.paint_now_playing_overlay(ui, response.rect);
                            response.context_menu(|ui| self.visualizer_context_menu(ui));
                        });
                },
            );
        }
        if self.visualizer_open {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(if self.player.video.is_some() { 16 } else { 50 }));
        }
    }
}
fn live_radio_badge(
    ui: &mut egui::Ui,
    station: &Station,
    favorite: Option<&Station>,
    artwork: &mut card_artwork::Cache,
    playing: bool,
) {
    let dot_id = ui.id().with("live-radio-dot");
    let chevron_id = ui.id().with("live-radio-chevron");
    let badge = egui::Button::new((
        egui::Atom::custom(dot_id, egui::Vec2::splat(12.0)),
        egui::RichText::new("LIVE RADIO").small().strong().color(theme::ACCENT),
        egui::Atom::custom(chevron_id, egui::Vec2::splat(10.0)),
    ))
    .fill(egui::Color32::from_rgb(44, 37, 66))
    .stroke(egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.55)))
    .min_size(egui::vec2(112.0, 26.0))
    .corner_radius(13)
    .atom_ui(ui);

    if let Some(dot) = badge.rect(dot_id) {
        let color = if playing { theme::ACCENT } else { theme::MUTED };
        let pulse = if playing && ui.is_rect_visible(dot) {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
            let seconds = ui.input(|input| input.time);
            (0.5 - 0.5 * (seconds * std::f64::consts::TAU / 1.8).cos()) as f32
        } else {
            0.0
        };
        ui.painter().circle_filled(
            dot.center(),
            4.0 + 2.0 * pulse,
            color.gamma_multiply(0.14 + 0.18 * pulse),
        );
        ui.painter().circle_filled(dot.center(), 2.5, color);
    }
    if let Some(chevron) = badge.rect(chevron_id) {
        let center = chevron.center();
        ui.painter().add(egui::Shape::line(
            vec![
                center + egui::vec2(-3.0, -1.5),
                center + egui::vec2(0.0, 1.5),
                center + egui::vec2(3.0, -1.5),
            ],
            egui::Stroke::new(1.25, theme::ACCENT),
        ));
    }
    let response = badge.response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_hint("Show station details");
    egui::Popup::from_toggle_button_response(&response)
        .gap(6.0)
        .show(|ui| {
            ui.set_max_width(280.0);
            ui.horizontal(|ui| {
                let source = favorite
                    .and_then(card_artwork::Source::station)
                    .or_else(|| card_artwork::Source::station(station));
                artwork.show(ui, source, Icon::Server);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Current station").small().color(theme::MUTED));
                    ui.add(egui::Label::new(egui::RichText::new(&station.name).strong()).wrap());
                });
            });
        });
}

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--audio-smoke-test") {
        return smoke_test();
    }
    let icon = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .ok()
        .map(|i| {
            let i = i.to_rgba8();
            #[cfg(target_os = "macos")]
            let i = {
                let size = 1024;
                let inset = 100;
                let artwork_size = size - 2 * inset;
                let radius = 184.0_f32;
                let mut artwork = image::imageops::resize(
                    &i,
                    artwork_size,
                    artwork_size,
                    image::imageops::FilterType::Lanczos3,
                );
                for (column, row, pixel) in artwork.enumerate_pixels_mut() {
                    let horizontal = (column as f32 + 0.5 - artwork_size as f32 / 2.0)
                        .abs()
                        - (artwork_size as f32 / 2.0 - radius);
                    let vertical = (row as f32 + 0.5 - artwork_size as f32 / 2.0)
                        .abs()
                        - (artwork_size as f32 / 2.0 - radius);
                    let distance = horizontal.max(0.0).hypot(vertical.max(0.0)) - radius;
                    let coverage = (0.5 - distance).clamp(0.0, 1.0);
                    pixel[3] = (pixel[3] as f32 * coverage).round() as u8;
                }
                let mut canvas = image::RgbaImage::new(size, size);
                image::imageops::overlay(&mut canvas, &artwork, inset.into(), inset.into());
                canvas
            };
            egui::IconData {
                width: i.width(),
                height: i.height(),
                rgba: i.into_raw(),
            }
        });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title_shown(false)
        .with_inner_size([1080.0, 760.0])
        .with_min_inner_size([420.0, 560.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    eframe::run_native(
        "KaleidoRadio",
        eframe::NativeOptions {
            viewport,
            persistence_path: Library::path().parent().map(|p| p.join("window.ron")),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
fn smoke_test() -> eframe::Result {
    let url = std::env::args()
        .nth(2)
        .expect("Usage: kaleido-radio --audio-smoke-test URL");
    let mut player = audio::Player::new();
    player
        .play(&url, 0.0, 0.0, None)
        .expect("Audio initialization failed");
    let start = Instant::now();
    let mut samples = 0;
    while start.elapsed() < Duration::from_secs(8) {
        if let Ok(event) = player.events.recv_timeout(Duration::from_millis(100)) {
            println!("{event:?}");
            if matches!(event, audio::Event::Metrics(_)) {
                samples += 1;
            }
        }
        if samples >= 10 {
            break;
        }
    }
    player.toggle();
    assert!(player.paused);
    player.toggle();
    assert!(!player.paused);
    player.stop();
    assert!(!player.active());
    assert!(samples >= 10, "No decoded audio reached the output source");
    println!("Audio smoke test passed.");
    Ok(())
}

/// Space above the player stack so it sits vertically centred in its
/// panel, for audio and video alike. Zero on the first frame, before
/// the stack is measured, and zero when the stack already fills or
/// overflows the panel, so the scroll area still works then.
fn vertical_top_inset(height: f32, measured: Option<f32>) -> f32 {
    measured.map_or(0.0, |measured| ((height - measured) * 0.5).max(0.0))
}
/// Bounds for the video player: the full panel width, and the panel
/// height minus the space the controls and title card need below it.
/// No fixed 960x560 cap, so `inline_size` stretches the video
/// proportionally to the panel, shrinking it only when the panel
/// height, not its width, is the limiting side.
/// Left inset that centres `content` in `available`, zero when it
/// already fills or overflows the space.
fn centred_inset(available: f32, content: f32) -> f32 {
    ((available - content) * 0.5).max(0.0)
}
fn video_bounds(background_width: f32, height: f32, compact: bool, reserved_height: f32) -> egui::Vec2 {
    let max_height = if compact {
        (height - reserved_height - 16.0).max(48.0)
    } else {
        (height - 300.0).max(48.0)
    };
    egui::vec2(background_width, max_height)
}
/// Height of the Recent tracks scroll area, in both layouts: three
/// rows of 36px artwork plus the 3px gaps between them is 114px,
/// plus 10px of headroom so the third row is never clipped by
/// rounding or a two-line title. This must be used as both the
/// `max_height` and the `min_scrolled_height` of the scroll area:
/// `max_height` alone is only a cap, and `auto_shrink` off alone
/// still leaves the area at whatever height the surrounding layout
/// offers it — inside the player stack, allocated with height 0,
/// that offer collapses, so the area stayed two rows tall even
/// with a 124px cap. The minimum forces it to 124px, so the card
/// is three rows tall even with fewer tracks, three tracks show
/// before scrolling, and a fourth track scrolls. Scroll-bar
/// behaviour is unchanged.
fn recent_tracks_max_height(_compact: bool) -> f32 {
    124.0
}
/// Offset to start a video at: its saved scrub location, when the
/// caller asked for the start and a usable saved location exists.
/// Audio, radio, and explicit seeks keep the requested offset.
fn video_resume_position(positions: &std::collections::HashMap<String, f64>, station: &Station, is_video: bool, requested: f64) -> f64 {
    if requested > 0.001 || !is_video {
        return requested;
    }
    positions.get(&station.stream_url).copied().filter(|position| position.is_finite() && *position > 0.0).unwrap_or(requested)
}
fn numbered_track_title(title: &str, track: Option<u32>, numbered: bool) -> String {
    if numbered {
        if let Some(track) = track.filter(|n| *n > 0) { return format!("{track:02}  {title}"); }
    }
    title.to_owned()
}

fn sort_menu(ui: &mut egui::Ui, id: &str, order: &mut SortOrder) -> bool {
    let previous = *order;
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("Sort: {}", order.label()))
        .show_ui(ui, |ui| {
            for mode in [
                SortOrder::DateDescending,
                SortOrder::DateAscending,
                SortOrder::NameAscending,
                SortOrder::NameDescending,
                SortOrder::TrackDiscAscending,
            ] {
                let icon = match mode {
                    SortOrder::DateDescending | SortOrder::DateAscending => Icon::History,
                    SortOrder::NameAscending | SortOrder::NameDescending => Icon::Library,
                    SortOrder::TrackDiscAscending => Icon::Music,
                };
                icons::menu_choice(ui, order, mode, icon, mode.label());
            }
        });
    previous != *order
}

#[cfg(test)]
mod folder_watch_tests {
    use super::*;
    #[test]
    fn vertical_top_inset_centres_short_stacks_and_not_tall_ones() {
        assert_eq!(vertical_top_inset(760.0, None), 0.0);
        assert_eq!(vertical_top_inset(760.0, Some(560.0)), 100.0);
        assert_eq!(vertical_top_inset(760.0, Some(760.0)), 0.0);
        assert_eq!(vertical_top_inset(760.0, Some(900.0)), 0.0);
    }
    #[test]
    fn video_bounds_use_panel_width_not_a_fixed_cap() {
        assert_eq!(video_bounds(1522.0, 1000.0, false, 0.0), egui::vec2(1522.0, 700.0));
        assert_eq!(video_bounds(682.0, 760.0, false, 0.0), egui::vec2(682.0, 460.0));
        assert_eq!(video_bounds(600.0, 200.0, true, 100.0), egui::vec2(600.0, 84.0));
    }
    #[test]
    fn centred_inset_splits_the_leftover_space() {
        assert_eq!(centred_inset(1522.0, 1000.0), 261.0);
        assert_eq!(centred_inset(1000.0, 1000.0), 0.0);
        assert_eq!(centred_inset(800.0, 1000.0), 0.0);
    }
    #[test]
    fn height_limited_video_is_narrower_than_its_panel() {
        // The screenshot case: a wide panel with a height-limited
        // 16:9 video, so the controls must be this width, not the
        // panel width, and centred with this inset on each side.
        let bounds = video_bounds(1522.0, 1000.0, false, 0.0);
        let video_width = video::inline_size(egui::vec2(16.0, 9.0), bounds).x;
        assert!(video_width < 1522.0);
        assert!((video_width - 1244.4445).abs() < 0.01);
        assert!((centred_inset(1522.0, video_width) - 138.7778).abs() < 0.01);
    }
    #[test]
    fn recent_tracks_show_three_rows_before_scrolling() {
        // Three 36px rows plus the 3px gaps between them, in both
        // layouts, and short of four full rows, so the fourth scrolls.
        let three_rows = 3.0 * 36.0 + 2.0 * 3.0;
        let four_rows = 4.0 * 36.0 + 3.0 * 3.0;
        assert_eq!(recent_tracks_max_height(true), 124.0);
        assert_eq!(recent_tracks_max_height(false), 124.0);
        assert!(recent_tracks_max_height(true) >= three_rows);
        assert!(recent_tracks_max_height(true) < four_rows);
    }
    #[test]
    fn video_resume_uses_saved_position_only_for_video() {
        let video = Station::new("Video", "https://www.youtube.com/watch?v=abc");
        let radio = Station::new("Radio", "https://example.org/live");
        let mut positions = std::collections::HashMap::new();
        positions.insert(video.stream_url.clone(), 123.4);
        positions.insert(radio.stream_url.clone(), 45.0);
        assert_eq!(video_resume_position(&positions, &video, true, 0.0), 123.4);
        assert_eq!(video_resume_position(&positions, &radio, false, 0.0), 0.0);
        assert_eq!(video_resume_position(&positions, &video, true, 10.0), 10.0);
        let unknown = Station::new("New", "https://www.youtube.com/watch?v=xyz");
        assert_eq!(video_resume_position(&positions, &unknown, true, 0.0), 0.0);
    }

    #[test]
    fn filesystem_events_refresh_directory_and_preserve_track_ids() {
        let path = std::env::temp_dir().join(format!("kaleido-watch-{}", Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("first.mp3"), []).unwrap();
        let before = LocalDirectory::read(path.clone(), &[]).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if event.is_ok() { let _ = tx.send(()); }
        }).unwrap();
        watcher.watch(&path, notify::RecursiveMode::NonRecursive).unwrap();
        std::fs::write(path.join("second.mp3"), []).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).expect("new file should trigger a folder event");
        let after = LocalDirectory::read(path.clone(), &before.tracks).unwrap();
        assert_eq!(after.tracks.len(), 2);
        assert_eq!(after.tracks.iter().find(|track| track.stream_url == before.tracks[0].stream_url).unwrap().id, before.tracks[0].id);
        drop(watcher);
        std::fs::remove_dir_all(path).unwrap();
    }
}
