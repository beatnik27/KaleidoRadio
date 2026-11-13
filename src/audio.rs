use rodio::{OutputStream, OutputStreamBuilder, Sink, Source};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
    },
    time::Duration,
};

#[derive(Clone, Copy, Default, Debug)]
pub struct Metrics {
    pub rms: f32,
    pub bass: f32,
    pub treble: f32,
}
#[derive(Debug)]
pub enum Event {
    Metadata(String),
    Duration(f64),
    Metrics(Metrics),
    // The decoder has no data behind the silence it is emitting: the
    // download has fallen behind playback. Sent only on transitions.
    Starved(bool),
    Error(String),
    VideoDetected,
    Transition(uuid::Uuid, f64),
}
pub(crate) fn executable(name: &str) -> String {
    let key = format!("KALEIDO_{}", name.to_uppercase());
    if let Ok(path) = std::env::var(key) {
        return path;
    }
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        let path = format!("{dir}/{name}");
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }
    name.to_owned()
}
pub struct Player {
    output: Option<OutputStream>,
    sink: Option<Sink>,
    transition: Arc<Mutex<Option<Prepared>>>,
    clock: Arc<AtomicU64>,
    equalizer: crate::equalizer::Controls,
    auto_volume: crate::auto_volume::Controls,
    pub events: Receiver<Event>,
    pub paused: bool,
    pub offset: f64,
    pub video: Option<crate::video::Player>,
    video_rate: f32,
}
impl Player {
    pub fn new() -> Self {
        Self {
            output: None,
            sink: None,
            transition: Arc::new(Mutex::new(None)),
            clock: Arc::new(AtomicU64::new(0)),
            equalizer: Arc::new(Mutex::new(crate::equalizer::Settings::default())),
            auto_volume: Arc::new(AtomicBool::new(false)),
            events: mpsc::channel().1,
            paused: false,
            offset: 0.0,
            video: None,
            video_rate: 1.0,
        }
    }
    pub fn set_auto_volume(&mut self, enabled: bool) {
        self.auto_volume.store(enabled, Ordering::Relaxed);
    }
    pub fn set_equalizer(&mut self, settings: crate::equalizer::Settings) {
        *self.equalizer.lock().unwrap() = settings;
    }
    pub fn play(
        &mut self,
        url: &str,
        offset: f64,
        volume: f32,
        auth: Option<(&str, &str)>,
    ) -> Result<(), String> {
        if crate::video::is_video_url(url) {
            self.play_video(url, offset, volume, auth)
        } else {
            self.play_audio(url, offset, volume, auth)
        }
    }
    pub fn play_audio(&mut self, url: &str, offset: f64, volume: f32, auth: Option<(&str, &str)>) -> Result<(), String> {
        self.stop();
        if self.output.is_none() {
            self.output = Some(
                OutputStreamBuilder::open_default_stream()
                    .map_err(|e| format!("Audio output unavailable: {e}"))?,
            );
        }
        let decoder = Decoder::open(url, offset, auth)?;
        let (event_tx, event_rx) = mpsc::channel();
        self.events = event_rx;
        self.clock = Arc::new(AtomicU64::new(offset.to_bits()));
        self.transition = Arc::new(Mutex::new(None));
        let sink = Sink::connect_new(self.output.as_ref().unwrap().mixer());
        sink.set_volume(volume);
        sink.append(TransitionSource {
            current: decoder,
            pending: self.transition.clone(),
            clock: self.clock.clone(),
            events: event_tx,
            position: 0,
            offset,
            fading: None,
            fade_position: 0,
            fade_samples: 0,
            equalizer: crate::equalizer::Processor::new(self.equalizer.clone()),
            auto_volume: crate::auto_volume::Processor::new(self.auto_volume.clone()),
            right_sample: None,
            leveling_title: String::new(),
        });
        self.sink = Some(sink);
        self.paused = false;
        self.offset = offset;
        Ok(())
    }
    pub fn queue_prepared(&mut self, prepared: Prepared) {
        *self.transition.lock().unwrap() = Some(prepared);
    }
    pub fn clear_prepared(&mut self) {
        self.transition.lock().unwrap().take();
    }
    pub fn play_video(&mut self, url: &str, offset: f64, volume: f32, auth: Option<(&str, &str)>) -> Result<(), String> {
        self.stop();
        self.video = Some(crate::video::Player::new(url, offset, volume, auth)?);
        self.offset = offset;
        if let Some(video) = &self.video { video.rate(self.video_rate); }
        Ok(())
    }
    pub fn video_rate(&self) -> f32 {
        self.video_rate
    }
    pub fn set_video_rate(&mut self, rate: f32) {
        self.video_rate = rate.clamp(0.25, 2.0);
        if let Some(video) = &self.video { video.rate(self.video_rate); }
    }
    pub fn stop(&mut self) {
        self.video = None;
        self.clear_prepared();
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.paused = false;
        self.events = mpsc::channel().1;
        self.output = None;
    }
    pub fn reset_output(&mut self) {
        self.stop();
        self.output = None;
    }
    pub fn toggle(&mut self) {
        if let Some(video) = &self.video {
            self.paused = !self.paused;
            video.pause(self.paused);
            return;
        }
        if let Some(sink) = &self.sink {
            self.paused = !self.paused;
            if self.paused {
                sink.pause();
            } else {
                sink.play();
            }
        }
    }
    pub fn volume(&self, volume: f32) {
        if let Some(video) = &self.video { video.volume(volume); }
        if let Some(sink) = &self.sink {
            sink.set_volume(volume);
        }
    }
    pub fn position(&self) -> f64 {
        if let Some(video) = &self.video { return video.state().position; }
        f64::from_bits(self.clock.load(Ordering::Relaxed))
    }
    pub fn finished(&self) -> bool {
        self.video.as_ref().map_or_else(|| self.sink.as_ref().is_some_and(Sink::empty), |video| video.state().status == 2)
    }
    pub fn active(&self) -> bool {
        self.sink.is_some() || self.video.is_some()
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}
// Process cleanup stays off the audio callback, including at track boundaries.
fn decoder_reaper() -> &'static Sender<Child> {
    static REAPER: OnceLock<Sender<Child>> = OnceLock::new();
    REAPER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Child>();
        std::thread::spawn(move || {
            for mut child in rx {
                let _ = child.kill();
                let _ = child.wait();
            }
        });
        tx
    })
}
// A decoder owns its process, so cancelling a preload also closes its pipes.
// PCM chunks between the decoder thread and the audio callback. Each chunk
// holds up to 2048 stereo samples, so 96 chunks bank roughly two seconds
// of audio — enough to ride out download jitter on streamed sources,
// where the previous 8-chunk buffer underran at about 0.2 seconds.
const PCM_CHANNEL_CHUNKS: usize = 96;
struct Decoder {
    source: PcmSource,
    child: Option<Child>,
    events: Receiver<Event>,
    duration: Arc<AtomicU64>,
}
impl Decoder {
    fn open(url: &str, offset: f64, auth: Option<(&str, &str)>) -> Result<Self, String> {
        let _ = decoder_reaper();
        let mut command = Command::new(executable("ffmpeg"));
        command.args([
            "-nostdin",
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "verbose",
        ]);
        if offset > 0.0 {
            command.args(["-ss", &offset.to_string()]);
        }
        if url.starts_with("http") {
            command.args(["-rw_timeout", "15000000", "-icy", "1"]);
            if let Some((user, password)) = auth {
                use base64::Engine;
                let token =
                    base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
                command.args(["-headers", &format!("Authorization: Basic {token}\r\n")]);
            }
        }
        let input = if url.starts_with("file:") {
            url::Url::parse(url)
                .ok()
                .and_then(|u| u.to_file_path().ok())
                .map(|p| p.to_string_lossy().into_owned())
                .ok_or("Invalid local file URL")?
        } else {
            url.to_owned()
        };
        command
            .args([
                "-i",
                &input,
                "-vn",
                "-f",
                "f32le",
                "-acodec",
                "pcm_f32le",
                "-ac",
                "2",
                "-ar",
                "44100",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|_| {
            "FFmpeg is required. Install it with brew install ffmpeg, or set KALEIDO_FFMPEG."
                .to_owned()
        })?;
        let stdout = child.stdout.take().ok_or("Decoder output unavailable")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Decoder diagnostics unavailable")?;
        let stopped = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(PCM_CHANNEL_CHUNKS);
        let (event_tx, event_rx) = mpsc::channel();
        let stop_reader = stopped.clone();
        std::thread::spawn(move || decode(stdout, tx, stop_reader));
        let metadata_tx = event_tx.clone();
        let duration = Arc::new(AtomicU64::new(0));
        let decoder_duration = duration.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line.contains("Video:") && !line.contains("attached pic") {
                    let _ = metadata_tx.send(Event::VideoDetected);
                }
                if let Some(title) = metadata_title(&line) {
                    let _ = metadata_tx.send(Event::Metadata(title));
                }
                if let Some(duration) = parse_duration(&line) {
                    decoder_duration.store(duration.to_bits(), Ordering::Relaxed);
                    let _ = metadata_tx.send(Event::Duration(duration));
                }
                if line.contains("Error opening input")
                    || line.contains("HTTP error")
                    || line.contains("Invalid data found")
                {
                    let _ = metadata_tx.send(Event::Error("Could not decode this source. Check its address, credentials, or availability.".into()));
                }
            }
        });
        Ok(Decoder {
            source: PcmSource { rx, queue: VecDeque::new(), stopped, events: event_tx, analyzer: AudioAnalyzer::default(), starved: false },
            child: Some(child),
            events: event_rx,
            duration,
        })
    }
    fn ready(&mut self) -> bool {
        if self.source.queue.is_empty() {
            if let Ok(chunk) = self.source.rx.try_recv() {
                self.source.queue.extend(chunk);
            }
        }
        !self.source.queue.is_empty()
    }
    fn duration(&self) -> f64 {
        f64::from_bits(self.duration.load(Ordering::Relaxed))
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        self.source.stopped.store(true, Ordering::Relaxed);
        if let Some(child) = self.child.take() {
            let _ = decoder_reaper().send(child);
        }
    }
}
pub struct Prepared {
    id: uuid::Uuid,
    decoder: Decoder,
    fade: f64,
}
impl Prepared {
    pub fn warm(mut self, cancelled: &AtomicBool) -> Result<Self, String> {
        if cancelled.load(Ordering::Relaxed) { return Err("Preload cancelled.".into()); }
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !self.decoder.ready() {
            if cancelled.load(Ordering::Relaxed) { return Err("Preload cancelled.".into()); }
            if std::time::Instant::now() >= deadline { return Err("Next track did not become ready.".into()); }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(self)
    }
    pub fn open(id: uuid::Uuid, url: &str, auth: Option<(&str, &str)>, fade: f64) -> Result<Self, String> {
        Ok(Self { id, decoder: Decoder::open(url, 0., auth)?, fade })
    }
}
struct TransitionSource {
    current: Decoder,
    pending: Arc<Mutex<Option<Prepared>>>,
    clock: Arc<AtomicU64>,
    events: Sender<Event>,
    position: u64,
    offset: f64,
    fading: Option<Decoder>,
    fade_position: u64,
    fade_samples: u64,
    equalizer: crate::equalizer::Processor,
    auto_volume: crate::auto_volume::Processor,
    right_sample: Option<f32>,
    leveling_title: String,
}
impl TransitionSource {
    fn promote(&mut self, ended: bool) -> bool {
        let mut pending = self.pending.lock().unwrap();
        let Some(next) = pending.as_mut() else { return false; };
        let duration = self.current.duration();
        let remaining = duration - self.offset - self.position as f64 / 88200.;
        let fade = next.fade.min((duration - self.offset).max(0.) / 2.).min(next.decoder.duration() / 2.);
        // Keep stereo frames aligned and never consume a next track before it is ready.
        if self.position % 2 != 0 || (!ended && (fade <= 0. || duration <= 0. || remaining > fade)) || !next.decoder.ready() {
            return false;
        }
        let next = pending.take().unwrap();
        drop(pending);
        let duration = next.decoder.duration();
        let previous = std::mem::replace(&mut self.current, next.decoder);
        self.fade_samples = if ended { 0 } else { (remaining.max(0.).min(fade) * 44100.) as u64 * 2 };
        self.fading = (self.fade_samples > 0).then_some(previous);
        self.fade_position = 0;
        self.position = 0;
        self.offset = 0.;
        self.clock.store(0f64.to_bits(), Ordering::Relaxed);
        self.auto_volume.new_track();
        self.leveling_title.clear();
        let _ = self.events.send(Event::Transition(next.id, duration));
        true
    }
}
impl TransitionSource {
    fn forward_events(&mut self) {
        for event in self.current.events.try_iter() {
            if let Event::Metadata(title) = &event {
                if !title.trim().is_empty() && *title != self.leveling_title {
                    self.auto_volume.new_track();
                    self.leveling_title.clone_from(title);
                }
            }
            let _ = self.events.send(event);
        }
    }
    fn next_mixed_sample(&mut self) -> Option<f32> {
        if self.position % 882 == 0 { self.promote(false); }
        let sample = match self.current.source.next() {
            Some(sample) => sample,
            None => {
                self.forward_events();
                if !self.promote(true) { return None; }
                self.current.source.next()?
            }
        };
        if self.current.source.starved {
            // No data behind this silence: hold the position clock so the
            // displayed position and the saved resume point do not run
            // ahead of what was actually heard.
            self.forward_events();
            return Some(sample);
        }
        let sample = if let Some(previous) = &mut self.fading {
            let t = (self.fade_position / 2) as f32 / (self.fade_samples / 2).max(1) as f32;
            let old = previous.source.next().unwrap_or(0.);
            self.fade_position += 1;
            let mixed = old * (1. - t) + sample * t;
            if self.fade_position >= self.fade_samples { self.fading = None; }
            mixed
        } else { sample };
        self.position += 1;
        if self.position % 882 == 0 {
            self.clock.store((self.offset + self.position as f64 / 88200.).to_bits(), Ordering::Relaxed);
            self.forward_events();
            if let Some(previous) = &self.fading {
                // Old metadata and meter events must not replace the new track's state.
                for _ in previous.events.try_iter() {}
            }
        }
        Some(self.equalizer.process(sample))
    }
}
impl Iterator for TransitionSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if let Some(right) = self.right_sample.take() { return Some(right); }
        let left = self.next_mixed_sample()?;
        let right = self.next_mixed_sample().unwrap_or(0.);
        let (left, right) = self.auto_volume.process(left, right);
        self.right_sample = Some(right);
        Some(left)
    }
}
impl Source for TransitionSource {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> u16 { 2 }
    fn sample_rate(&self) -> u32 { 44100 }
    fn total_duration(&self) -> Option<Duration> { None }
}

fn decode(mut reader: impl Read, tx: SyncSender<Vec<f32>>, stop: Arc<AtomicBool>) {
    let mut bytes = [0u8; 8192];
    let mut pending = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        let n = match reader.read(&mut bytes) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        pending.extend_from_slice(&bytes[..n]);
        let end = pending.len() / 4 * 4;
        let mut samples = pending[..end]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect::<Vec<_>>();
        pending.drain(..end);
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            match tx.try_send(samples) {
                Ok(()) => break,
                Err(TrySendError::Disconnected(_)) => return,
                Err(TrySendError::Full(s)) => {
                    samples = s;
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }
}
struct PcmSource {
    rx: Receiver<Vec<f32>>,
    queue: VecDeque<f32>,
    stopped: Arc<AtomicBool>,
    events: Sender<Event>,
    analyzer: AudioAnalyzer,
    starved: bool,
}

// Keep filter history per channel: filtering interleaved samples mixes stereo
// channels together and can erase bass when the channels have opposite phase.
#[derive(Default)]
struct AudioAnalyzer {
    count: usize,
    power: f32,
    low_power: f32,
    high_power: f32,
    low: [f32; 2],
    mid: [f32; 2],
}
impl AudioAnalyzer {
    fn push(&mut self, sample: f32) -> Option<Metrics> {
        let sample = if sample.is_finite() { sample } else { 0. };
        let channel = self.count % 2;
        self.low[channel] += 0.025 * (sample - self.low[channel]);
        self.mid[channel] += 0.3 * (sample - self.mid[channel]);
        self.power += sample * sample;
        self.low_power += self.low[channel].powi(2);
        self.high_power += (sample - self.mid[channel]).powi(2);
        self.count += 1;
        // 20 ms of 44.1 kHz stereo audio, measured as it is played.
        if self.count < 1764 {
            return None;
        }
        let metrics = Metrics {
            rms: (self.power / self.count as f32).sqrt(),
            bass: (self.low_power / self.count as f32).sqrt(),
            treble: (self.high_power / self.count as f32).sqrt(),
        };
        self.count = 0;
        self.power = 0.;
        self.low_power = 0.;
        self.high_power = 0.;
        Some(metrics)
    }
}

impl Iterator for PcmSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.stopped.load(Ordering::Relaxed) {
            return None;
        }
        if self.queue.is_empty() {
            match self.rx.recv_timeout(Duration::from_millis(5)) {
                Ok(chunk) => {
                    self.queue.extend(chunk);
                    self.set_starved(false);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.set_starved(false);
                    return None;
                }
                Err(_) => self.set_starved(true),
            }
        } else if self.starved {
            // Data arrived through ready() while a previous pull timed out.
            self.set_starved(false);
        }
        let sample = self.queue.pop_front().unwrap_or(0.);
        if let Some(metrics) = self.analyzer.push(sample) {
            let _ = self.events.send(Event::Metrics(metrics));
        }
        Some(sample)
    }
}
impl PcmSource {
    fn set_starved(&mut self, starved: bool) {
        if self.starved != starved {
            self.starved = starved;
            let _ = self.events.send(Event::Starved(starved));
        }
    }
}
impl Source for PcmSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        44100
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
pub fn metadata_title(line: &str) -> Option<String> {
    if let Some((_, title)) = line.split_once("Metadata update for StreamTitle: ") {
        return Some(title.trim().to_owned());
    }
    if let Some((_, title)) = line.split_once("StreamTitle='") {
        return Some(title.split("';").next()?.trim().to_owned());
    }
    let (key, value) = line.trim().split_once(':')?;
    if ["title", "icy-title", "StreamTitle"].contains(&key.trim()) && !value.trim().is_empty() {
        Some(value.trim().to_owned())
    } else {
        None
    }
}
fn parse_duration(line: &str) -> Option<f64> {
    let time = line.split_once("Duration: ")?.1.split(',').next()?;
    let mut parts = time.split(':');
    Some(
        parts.next()?.parse::<f64>().ok()? * 3600.0
            + parts.next()?.parse::<f64>().ok()? * 60.0
            + parts.next()?.parse::<f64>().ok()?,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn test_decoder(samples: Vec<f32>) -> Decoder {
        let duration = samples.len() as f64 / 88200.;
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(samples).unwrap();
        let (events, received) = mpsc::channel();
        Decoder {
            source: PcmSource { rx, queue: VecDeque::new(), stopped: Arc::new(AtomicBool::new(false)), events, analyzer: AudioAnalyzer::default(), starved: false },
            child: None,
            events: received,
            duration: Arc::new(AtomicU64::new(duration.to_bits())),
        }
    }
    fn test_transition(current: Vec<f32>, next: Vec<f32>, fade: f64) -> (TransitionSource, Receiver<Event>, uuid::Uuid) {
        let id = uuid::Uuid::new_v4();
        let (tx, rx) = mpsc::channel();
        (TransitionSource {
            current: test_decoder(current),
            pending: Arc::new(Mutex::new(Some(Prepared { id, decoder: test_decoder(next), fade }))),
            clock: Arc::new(AtomicU64::new(0)), events: tx, position: 0, offset: 0.,
            fading: None, fade_position: 0, fade_samples: 0,
            equalizer: crate::equalizer::Processor::new(Arc::new(Mutex::new(crate::equalizer::Settings::default()))),
            auto_volume: crate::auto_volume::Processor::new(Arc::new(AtomicBool::new(false))),
            right_sample: None,
            leveling_title: String::new(),
        }, rx, id)
    }
    #[test]
    fn warming_a_cancelled_preload_does_not_queue_it() {
        let prepared = Prepared { id: uuid::Uuid::new_v4(), decoder: test_decoder(vec![0.5; 1764]), fade: 0.0 };
        assert!(prepared.warm(&AtomicBool::new(true)).is_err());
    }
    #[test]
    #[ignore = "requires yt-dlp, FFmpeg, and internet access"]
    fn live_preloaded_youtube_audio_handoff_has_no_startup_wait() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stream = crate::youtube::streaming::start_audio("https://www.youtube.com/watch?v=Kl12yfoHaLg", crate::youtube::Quality::default(), crate::youtube::FrameRate::default(), 0.0, cancelled.clone()).unwrap();
        let id = uuid::Uuid::new_v4();
        let prepared = Prepared::open(id, &stream.address, None, 0.0).unwrap().warm(&cancelled).unwrap();
        let (mut source, events, _) = test_transition(vec![0.25; 1764], vec![], 0.0);
        *source.pending.lock().unwrap() = Some(prepared);
        let started = std::time::Instant::now();
        let samples: Vec<_> = source.by_ref().take(3528).collect();
        assert_eq!(&samples[..1764], vec![0.25; 1764]);
        assert_eq!(samples.len(), 3528);
        assert!(started.elapsed() < Duration::from_millis(500), "Track boundary waited for network/decoder startup");
        let transitions: Vec<_> = events.try_iter().filter_map(|event| if let Event::Transition(id, _) = event { Some(id) } else { None }).collect();
        assert_eq!(transitions, vec![id]);
        eprintln!("Preloaded audio track boundary completed in {:?}", started.elapsed());
        drop(source); drop(stream);
    }
    #[test]
    fn gapless_preserves_every_sample_and_emits_one_track_boundary() {
        let (source, events, id) = test_transition(vec![0.25; 1764], vec![0.75; 1764], 0.);
        let samples = source.collect::<Vec<_>>();
        assert_eq!(&samples[..1764], vec![0.25; 1764]);
        assert_eq!(&samples[1764..], vec![0.75; 1764]);
        let transitions = events.try_iter().filter_map(|event| match event {
            Event::Transition(id, duration) => Some((id, duration)), _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(transitions, vec![(id, 0.02)]);
    }
    #[test]
    fn crossfade_overlaps_tracks_with_stereo_aligned_linear_gains() {
        let (source, _, _) = test_transition(vec![1.; 3528], vec![-1.; 3528], 0.02);
        let samples = source.collect::<Vec<_>>();
        assert_eq!(samples.len(), 5292);
        assert!(samples[..1764].iter().all(|s| *s == 1.));
        assert!(samples[2646].abs() < 0.002);
        assert!(samples[3528..].iter().all(|s| *s == -1.));
        assert!(samples.chunks_exact(2).all(|frame| frame[0] == frame[1]));
    }
    #[test]
    fn cancelling_preload_does_not_play_the_next_track() {
        let (source, events, _) = test_transition(vec![0.25; 1764], vec![0.75; 1764], 0.);
        source.pending.lock().unwrap().take();
        assert_eq!(source.collect::<Vec<_>>(), vec![0.25; 1764]);
        assert!(!events.try_iter().any(|event| matches!(event, Event::Transition(..))));
    }
    #[test]
    fn crossfade_is_limited_for_short_tracks() {
        let (source, _, _) = test_transition(vec![1.; 3528], vec![-1.; 3528], 12.);
        assert_eq!(source.count(), 5292);
    }

    #[test]
    #[ignore = "requires yt-dlp, FFmpeg, and internet access"]
    fn live_youtube_audio_mode_decodes_and_reports_meters() {
        let address = "https://www.youtube.com/watch?v=Kl12yfoHaLg";
        let stream = crate::youtube::streaming::start_audio(address, crate::youtube::Quality::default(), crate::youtube::FrameRate::default(), 300.0, Arc::new(AtomicBool::new(false))).unwrap();
        let probe = |url: &str| {
            let output = Command::new(executable("ffprobe"))
                .args(["-v", "error", "-rw_timeout", "5000000", "-read_intervals", "%+1", "-show_entries", "stream=codec_type", "-of", "json", url])
                .output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
        };
        assert!(stream.artist.as_deref().is_some_and(|artist| artist.contains("Luttrell")), "Expected artist metadata, got {:?}", stream.artist);
        let metadata = probe(&stream.address);
        let streams = metadata["streams"].as_array().unwrap();
        assert!(streams.iter().any(|stream| stream["codec_type"] == "audio"));
        assert!(!streams.iter().any(|stream| stream["codec_type"] == "video"));
        let duration = stream.duration;
        let mut decoder = Decoder::open(&stream.address, 8.0, None).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !decoder.ready() {
            assert!(std::time::Instant::now() < deadline, "Audio did not begin decoding");
            std::thread::sleep(Duration::from_millis(20));
        }
        let samples: Vec<_> = decoder.source.by_ref().take(88200).collect();
        assert!(samples.iter().any(|sample| sample.abs() > 0.001));
        let events: Vec<_> = decoder.events.try_iter().collect();
        assert!(events.iter().any(|event| matches!(event, Event::Metrics(metrics) if metrics.rms > 0.0)));
        assert!(!events.iter().any(|event| matches!(event, Event::VideoDetected)));
        drop(decoder); drop(stream);
        let video = crate::youtube::streaming::start(address, crate::youtube::Quality::default(), crate::youtube::FrameRate::default(), 300.0, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(video.start_time, 292.0);
        assert_eq!(video.duration, duration);
        let metadata = probe(&video.address);
        assert!(metadata["streams"].as_array().unwrap().iter().any(|stream| stream["codec_type"] == "video"));
    }

    #[test]
    #[ignore = "requires FFmpeg"]
    fn real_decoders_join_without_added_silence() {
        let directory = std::env::temp_dir().join(format!("kaleido-gapless-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut decoders = Vec::new();
        for (index, amplitude) in [0.25, 0.75].into_iter().enumerate() {
            let path = directory.join(format!("{index}.wav"));
            assert!(Command::new(executable("ffmpeg"))
                .args(["-v", "error", "-f", "lavfi", "-i", &format!("aevalsrc={amplitude}:s=44100:d=0.1"), "-ac", "2", "-c:a", "pcm_f32le"])
                .arg(&path).status().unwrap().success());
            let mut decoder = Decoder::open(path.to_str().unwrap(), 0., None).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while decoder.child.as_mut().unwrap().try_wait().unwrap().is_none() {
                assert!(std::time::Instant::now() < deadline, "decoder did not finish buffering");
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(decoder.ready());
            decoders.push(decoder);
        }
        let (mut source, events, _) = test_transition(vec![], vec![], 0.);
        let next = decoders.pop().unwrap();
        source.current = decoders.pop().unwrap();
        source.pending.lock().unwrap().as_mut().unwrap().decoder = next;
        let samples = source.collect::<Vec<_>>();
        assert_eq!(samples.len(), 17640);
        // FFmpeg's mono-to-stereo conversion uses equal-power panning.
        assert!(samples[..8820].iter().all(|sample| *sample > 0.17 && *sample < 0.18));
        assert!(samples[8820..].iter().all(|sample| *sample > 0.53 && *sample < 0.54));
        assert_eq!(events.try_iter().filter(|event| matches!(event, Event::Transition(..))).count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn tone_metrics(frequency: f32, inverted_right: bool) -> Metrics {
        let mut analyzer = AudioAnalyzer::default();
        let mut latest = Metrics::default();
        for frame in 0..44100 {
            let sample = (std::f32::consts::TAU * frequency * frame as f32 / 44100.).sin() * 0.4;
            for channel in 0..2 {
                if let Some(metrics) = analyzer.push(if inverted_right && channel == 1 {
                    -sample
                } else {
                    sample
                }) {
                    latest = metrics;
                }
            }
        }
        latest
    }

    #[test]
    fn stereo_phase_does_not_cancel_bass_analysis() {
        let normal = tone_metrics(80., false);
        let inverted = tone_metrics(80., true);
        assert!(normal.bass > 0.2);
        assert!((normal.bass - inverted.bass).abs() < 0.001);
    }

    #[test]
    fn audio_bands_distinguish_bass_and_treble() {
        let bass = tone_metrics(80., false);
        let treble = tone_metrics(8000., false);
        assert!(bass.bass > treble.bass * 10.);
        assert!(treble.treble > bass.treble * 10.);
    }

    #[test]
    fn pcm_playback_emits_levels_every_twenty_milliseconds() {
        let (tx, rx) = mpsc::sync_channel(1);
        let (events, received) = mpsc::channel();
        let samples: Vec<_> = (0..1764 * 3)
            .map(|i| (std::f32::consts::TAU * 80. * (i / 2) as f32 / 44100.).sin() * 0.4)
            .collect();
        tx.send(samples.clone()).unwrap();
        drop(tx);
        let source = PcmSource {
            rx,
            queue: VecDeque::new(),
            stopped: Arc::new(AtomicBool::new(false)),
            events,
            analyzer: AudioAnalyzer::default(),
            starved: false,
        };
        assert_eq!(source.collect::<Vec<_>>(), samples);
        let metrics: Vec<_> = received
            .try_iter()
            .filter_map(|event| {
                if let Event::Metrics(m) = event {
                    Some(m)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(metrics.len(), 3);
        assert!(metrics.iter().all(|m| m.rms > 0.2 && m.bass > 0.2));
    }

    #[test]
    fn empty_stream_metadata_clears_song_artwork() {
        assert_eq!(metadata_title("StreamTitle='';"), Some(String::new()));
        assert_eq!(
            metadata_title("Metadata update for StreamTitle: "),
            Some(String::new())
        );
    }

    #[test]
    fn parses_decoder_metadata() {
        assert_eq!(
            metadata_title("[https @ 0x123] Metadata update for StreamTitle: Artist - Song"),
            Some("Artist - Song".into())
        );
        assert_eq!(
            metadata_title("  title : Artist - Song"),
            Some("Artist - Song".into())
        );
        assert_eq!(
            metadata_title("Metadata update for StreamTitle='Artist - Song';"),
            Some("Artist - Song".into())
        );
        assert_eq!(
            parse_duration("  Duration: 01:02:03.50, start: 0"),
            Some(3723.5)
        );
    }

    #[test]
    fn pcm_source_reports_starvation_once_and_resumes_on_data() {
        let (tx, rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let mut source = PcmSource {
            rx,
            queue: VecDeque::new(),
            stopped: Arc::new(AtomicBool::new(false)),
            events: event_tx,
            analyzer: AudioAnalyzer::default(),
            starved: false,
        };
        // No data yet: silence, and the starvation event fires exactly once.
        assert_eq!(source.next(), Some(0.));
        assert_eq!(source.next(), Some(0.));
        assert!(source.starved);
        assert!(matches!(event_rx.try_recv(), Ok(Event::Starved(true))));
        assert!(event_rx.try_recv().is_err());
        // Data arrives: the real sample follows and starvation clears once.
        tx.send(vec![0.5, -0.5]).unwrap();
        assert_eq!(source.next(), Some(0.5));
        assert!(!source.starved);
        assert!(matches!(event_rx.try_recv(), Ok(Event::Starved(false))));
        assert_eq!(source.next(), Some(-0.5));
        // Sender dropped: the source ends instead of looping silence.
        drop(tx);
        assert_eq!(source.next(), None);
    }
}
