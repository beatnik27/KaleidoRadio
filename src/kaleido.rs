//! Native mirrored petals and the procedural Prism patterns from index.html.
use crate::audio::Metrics;
use crate::hover_hints::HoverHint;
use crate::icons::{self, Icon, IconButton};
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use rand::Rng;
use std::{f32::consts::TAU, sync::Arc};

#[path = "musical_motion.rs"]
mod musical_motion;
#[path = "prism_renderer.rs"]
mod prism_renderer;
#[path = "projectm.rs"]
mod projectm;

const TRANSITION_SECONDS: f32 = 3.5;
const MILKDROP_DEFAULT_SECONDS: u32 = 28;
const MILKDROP_MAX_SECONDS: u32 = 300;

const PATTERNS: [&str; 11] = [
    "Neon Petals",
    "Chromatic Bloom",
    "Stained Glass",
    "Crystal Garden",
    "Prismatic Mosaic",
    "Neon Filigree",
    "Opal Currents",
    "Cosmic Lace",
    "Liquid Petals",
    "Mirror Labyrinth",
    "Solar Embroidery",
];

// base, spin, layer step, split, radial shift, sector variation, saturation
const PALETTES: [[f32; 7]; 5] = [
    [0., 44., 34., 55., 70., 36., 0.96],
    [305., 30., 47., 70., 50., 24., 1.],
    [262., 20., 26., 42., 90., 48., 0.94],
    [8., 36., 30., 60., 60., 30., 1.],
    [190., 28., 38., 50., 80., 40., 0.96],
];

#[derive(Clone, Copy)]
struct MusicSettings {
    folds: u32,
    motion: f32,
    complexity: f32,
    chroma: f32,
}

fn default_milkdrop_auto() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub auto: bool,
    pub folds: u32,
    pub motion: f32,
    pub clockwise: bool,
    pub complexity: f32,
    pub chroma: f32,
    pub palette: usize,
    pub pattern: usize,
    pub hue: f32,
    pub seed: f32,
    pub milkdrop: bool,
    pub milkdrop_preset: usize,
    #[serde(default = "default_milkdrop_auto")]
    pub milkdrop_auto: bool,
    #[serde(default)]
    pub milkdrop_seconds: Option<u32>,
    pub milkdrop_random: bool,
    pub milkdrop_disabled: Vec<String>,
    pub milkdrop_favorites: Vec<String>,
    pub milkdrop_favorites_only: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Kaleido::default().settings()
    }
}

#[derive(Clone)]
pub struct Kaleido {
    previous: Option<Box<Kaleido>>,
    renderer: Option<Arc<prism_renderer::Renderer>>,
    projectm: Option<Arc<projectm::Renderer>>,
    projectm_error: Option<String>,
    projectm_presets: Vec<String>,
    milkdrop: bool,
    milkdrop_preset: usize,
    milkdrop_seconds: u32,
    milkdrop_random: bool,
    milkdrop_disabled: Vec<String>,
    milkdrop_favorites: Vec<String>,
    milkdrop_favorites_only: bool,
    milkdrop_timer: f32,
    transition: f32,
    frame_dt: f32,
    pending_pattern: Option<usize>,
    pending_folds: Option<u32>,
    pending_remix: bool,
    pub auto: bool,
    folds: u32,
    motion: f32,
    clockwise: bool,
    choreography: musical_motion::Motion,
    rotation: f32,
    complexity: f32,
    chroma: f32,
    palette: usize,
    pattern: usize,
    hue: f32,
    seed: f32,
    phase: f32,
    color_phase: f32,
    music_hue: f32,
    music_symmetry: f32,
    last_time: Option<f64>,
    since_remix: f32,
    since_beat: f32,
    beats: u32,
    bass_average: f32,
    previous_bass: f32,
    energy: f32,
    level_average: [f32; 3],
    meter_bands: [f32; 2],
    meter_baseline: [f32; 2],
    beat_pulse: f32,
    bass: f32,
    treble: f32,
    pointer: Vec2,
}
impl Default for Kaleido {
    fn default() -> Self {
        Self {
            previous: None,
            renderer: None,
            projectm: None,
            projectm_error: None,
            projectm_presets: Vec::new(),
            milkdrop: false,
            milkdrop_preset: 0,
            milkdrop_seconds: MILKDROP_DEFAULT_SECONDS,
            milkdrop_random: false,
            milkdrop_disabled: Vec::new(),
            milkdrop_favorites: Vec::new(),
            milkdrop_favorites_only: false,
            milkdrop_timer: 0.,
            transition: 1.,
            frame_dt: 0.,
            pending_pattern: None,
            pending_folds: None,
            pending_remix: false,
            auto: true,
            folds: 14,
            motion: 0.6,
            clockwise: true,
            choreography: musical_motion::Motion::default(),
            rotation: 0.,
            complexity: 0.72,
            chroma: 1.,
            palette: 0,
            pattern: 0,
            hue: 0.,
            seed: 1.7,
            phase: 0.,
            color_phase: 0.,
            music_hue: 0.,
            music_symmetry: 0.,
            last_time: None,
            since_remix: 0.,
            since_beat: 1.,
            beats: 0,
            bass_average: 0.,
            previous_bass: 0.,
            energy: 0.,
            level_average: [0.02, 0.006, 0.008],
            meter_bands: [0.; 2],
            meter_baseline: [0.42; 2],
            beat_pulse: 0.,
            bass: 0.,
            treble: 0.,
            pointer: Vec2::ZERO,
        }
    }
}
impl Kaleido {
    pub fn initialize_renderer(&mut self, gl: &eframe::glow::Context) {
        match prism_renderer::Renderer::new(gl) {
            Ok(renderer) => self.renderer = Some(Arc::new(renderer)),
            Err(error) => eprintln!("Visualizer shader unavailable: {error}"),
        }
    }

    pub fn initialize_projectm(&mut self) {
        match projectm::Renderer::new() {
            Ok(renderer) => {
                let renderer = Arc::new(renderer);
                self.projectm_presets = renderer.preset_names();
                self.projectm = Some(renderer);
                if self.milkdrop_preset_available(self.milkdrop_preset) {
                    self.set_milkdrop_preset(self.milkdrop_preset);
                } else {
                    self.advance_milkdrop_preset();
                }
            }
            Err(error) => {
                eprintln!("projectM unavailable: {error}");
                self.projectm_error = Some(error);
            }
        }
    }

    pub fn set_milkdrop_preset(&mut self, index: usize) {
        self.milkdrop_preset = index;
        self.milkdrop_timer = 0.;
        if let Some(renderer) = &self.projectm {
            renderer.set_preset(index);
        }
    }

    fn milkdrop_preset_enabled(&self, index: usize) -> bool {
        self.projectm_presets
            .get(index)
            .is_some_and(|name| !self.milkdrop_disabled.contains(name))
    }

    fn milkdrop_preset_available(&self, index: usize) -> bool {
        self.milkdrop_preset_enabled(index)
            && (!self.milkdrop_favorites_only
                || self.milkdrop_favorites.contains(&self.projectm_presets[index]))
    }

    fn advance_milkdrop_preset(&mut self) {
        self.milkdrop_timer = 0.0;
        let count = self.projectm_presets.len();
        let next = if self.milkdrop_random {
            let candidates: Vec<_> = (0..count)
                .filter(|&index| index != self.milkdrop_preset && self.milkdrop_preset_available(index))
                .collect();
            use rand::seq::IndexedRandom;
            candidates.choose(&mut rand::rng()).copied()
        } else {
            self.adjacent_milkdrop_preset(false)
        };
        if let Some(index) = next {
            self.set_milkdrop_preset(index);
        } else if !self.milkdrop_preset_available(self.milkdrop_preset) {
            if let Some(index) = (0..count).find(|&index| self.milkdrop_preset_available(index)) {
                self.set_milkdrop_preset(index);
            }
        }
    }

    fn adjacent_milkdrop_preset(&self, previous: bool) -> Option<usize> {
        let count = self.projectm_presets.len();
        let start = self.milkdrop_preset.min(count.saturating_sub(1));
        (1..count)
            .map(|offset| if previous { count - offset } else { offset })
            .map(|offset| (start + offset) % count)
            .find(|&index| self.milkdrop_preset_available(index))
    }

    fn milkdrop_navigation(&mut self, ui: &mut egui::Ui) -> bool {
        for (previous, icon, label) in [
            (true, Icon::Previous, "Previous Preset"),
            (false, Icon::Next, "Next Preset"),
        ] {
            let adjacent = self.adjacent_milkdrop_preset(previous);
            let response = ui.add_enabled(
                adjacent.is_some(),
                IconButton::new(icon, label).left_aligned(),
            );
            if response.clicked() && let Some(index) = adjacent {
                self.set_milkdrop_preset(index);
                return true;
            }
        }
        false
    }

    fn toggle_milkdrop_favorite(&mut self) {
        let was_available = self.milkdrop_preset_available(self.milkdrop_preset);
        let name = &self.projectm_presets[self.milkdrop_preset];
        if self.milkdrop_favorites.contains(name) {
            self.milkdrop_favorites.retain(|favorite| favorite != name);
        } else {
            self.milkdrop_favorites.push(name.clone());
        }
        if !self.milkdrop_preset_available(self.milkdrop_preset) {
            self.advance_milkdrop_preset();
        } else if !was_available {
            self.set_milkdrop_preset(self.milkdrop_preset);
        }
    }

    fn milkdrop_pool_menu(&mut self, ui: &mut egui::Ui) -> bool {
        let previous = self.milkdrop_favorites_only;
        ui.label("Use MilkDrop Presets");
        icons::menu_choice(ui, &mut self.milkdrop_favorites_only, false, Icon::Library, "All Presets");
        icons::menu_choice(ui, &mut self.milkdrop_favorites_only, true, Icon::Star, "Favorite Presets");
        if previous == self.milkdrop_favorites_only {
            return false;
        }
        self.milkdrop_timer = 0.0;
        if !self.milkdrop_preset_available(self.milkdrop_preset) {
            self.advance_milkdrop_preset();
        }
        true
    }

    pub fn milkdrop_preset_menu(&mut self, ui: &mut egui::Ui) -> bool {
        if !self.milkdrop {
            return false;
        }
        let pool_changed = self.milkdrop_pool_menu(ui);
        ui.separator();
        let navigated = self.milkdrop_navigation(ui);
        let actions_changed = self.milkdrop_preset_actions(ui);
        pool_changed || navigated || actions_changed
    }

    fn milkdrop_preset_actions(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        if !self.milkdrop {
            return false;
        }
        if self.milkdrop_preset_enabled(self.milkdrop_preset) {
            let name = &self.projectm_presets[self.milkdrop_preset];
            let label = if self.milkdrop_favorites.contains(name) {
                "Remove from Favorites"
            } else {
                "Add to Favorites"
            };
            if icons::menu_item(ui, Icon::Star, label).clicked() {
                self.toggle_milkdrop_favorite();
                return true;
            }
            if icons::menu_item(ui, Icon::Stop, "Disable Current MilkDrop Preset").clicked() {
                self.milkdrop_disabled.push(self.projectm_presets[self.milkdrop_preset].clone());
                self.advance_milkdrop_preset();
                changed = true;
            }
        }
        if !self.milkdrop_disabled.is_empty() && icons::menu_item(ui, Icon::Refresh, "Re-enable All MilkDrop Presets").clicked() {
            self.milkdrop_disabled.clear();
            if self.milkdrop_preset_available(self.milkdrop_preset) {
                self.set_milkdrop_preset(self.milkdrop_preset);
            } else {
                self.advance_milkdrop_preset();
            }
            changed = true;
        }
        changed
    }

    pub fn destroy_renderer(&self, gl: &eframe::glow::Context) {
        if let Some(renderer) = &self.renderer {
            renderer.destroy(gl);
        }
        if let Some(renderer) = &self.projectm {
            renderer.destroy(gl);
        }
    }

    fn direction(&self) -> f32 {
        if self.clockwise { 1. } else { -1. }
    }

    fn effective_hue(&self) -> f32 {
        self.hue + self.color_phase * PALETTES[self.palette][1] + self.music_hue
    }

    pub fn settings(&self) -> Settings {
        Settings {
            auto: self.auto,
            folds: self.folds,
            motion: self.motion,
            clockwise: self.clockwise,
            complexity: self.complexity,
            chroma: self.chroma,
            palette: self.palette,
            pattern: self.pattern,
            hue: self.hue,
            seed: self.seed,
            milkdrop: self.milkdrop,
            milkdrop_preset: self.milkdrop_preset,
            milkdrop_auto: self.milkdrop_seconds != 0,
            milkdrop_seconds: Some(self.milkdrop_seconds),
            milkdrop_random: self.milkdrop_random,
            milkdrop_disabled: self.milkdrop_disabled.clone(),
            milkdrop_favorites: self.milkdrop_favorites.clone(),
            milkdrop_favorites_only: self.milkdrop_favorites_only,
        }
    }
    pub fn restore(settings: &Settings) -> Self {
        let clean = |value: f32, low: f32, high: f32, fallback: f32| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                fallback
            }
        };
        Self {
            auto: settings.auto,
            folds: settings.folds.clamp(4, 28) / 2 * 2,
            motion: clean(settings.motion, 0., 2., 0.6),
            clockwise: settings.clockwise,
            complexity: clean(settings.complexity, 0., 1., 0.72),
            chroma: clean(settings.chroma, 0., 2., 1.),
            palette: settings.palette.min(4),
            pattern: settings.pattern.min(PATTERNS.len() - 1),
            hue: clean(settings.hue, 0., 360., 0.),
            seed: clean(settings.seed, 0., TAU, 1.7),
            milkdrop: settings.milkdrop,
            milkdrop_preset: settings.milkdrop_preset,
            milkdrop_seconds: settings.milkdrop_seconds
                .unwrap_or(if settings.milkdrop_auto { MILKDROP_DEFAULT_SECONDS } else { 0 })
                .min(MILKDROP_MAX_SECONDS),
            milkdrop_random: settings.milkdrop_random,
            milkdrop_disabled: settings.milkdrop_disabled.clone(),
            milkdrop_favorites: settings.milkdrop_favorites.clone(),
            milkdrop_favorites_only: settings.milkdrop_favorites_only,
            ..Default::default()
        }
    }

    fn begin_transition(&mut self) {
        if self.previous.is_none() {
            let mut previous = self.clone();
            previous.previous = None;
            self.previous = Some(Box::new(previous));
            self.transition = 0.;
        }
    }

    fn select_pattern(&mut self, pattern: usize) {
        if self.previous.is_some() {
            self.pending_pattern = Some(pattern);
            return;
        }
        self.begin_transition();
        self.pattern = pattern;
    }

    pub fn remix(&mut self) {
        if self.previous.is_some() {
            self.pending_remix = true;
            return;
        }
        self.begin_transition();
        let mut rng = rand::rng();
        self.folds = rng.random_range(4..=10) * 2;
        self.motion = rng.random_range(0.3..=1.3);
        self.hue = (self.hue + rng.random_range(45.0..315.0)).rem_euclid(360.);
        self.palette = (self.palette + rng.random_range(1..5)) % 5;
        self.pattern = (self.pattern + rng.random_range(1..PATTERNS.len())) % PATTERNS.len();
        self.seed = rng.random_range(0.0..TAU);
        self.since_remix = 0.;
        self.beats = 0;
    }

    pub fn update(&mut self, metrics: Metrics, time: f64, sensitivity: f32, playing: bool) {
        let dt = self
            .last_time
            .map_or(0., |last| (time - last).clamp(0., 0.1) as f32);
        self.last_time = Some(time);
        self.frame_dt = dt;
        if self.milkdrop && self.milkdrop_seconds > 0 {
            self.milkdrop_timer += dt;
            if self.milkdrop_timer >= self.milkdrop_seconds as f32 {
                self.advance_milkdrop_preset();
            }
        }
        self.transition = (self.transition + dt / TRANSITION_SECONDS).min(1.);
        if self.transition >= 1. {
            self.previous = None;
            if let Some(pattern) = self.pending_pattern.take() {
                self.select_pattern(pattern);
            } else if let Some(folds) = self.pending_folds.take() {
                self.begin_transition();
                self.folds = folds;
            } else if std::mem::take(&mut self.pending_remix) {
                self.remix();
            }
        }
        let mut levels = [metrics.rms, metrics.bass, metrics.treble];
        for (i, level) in levels.iter_mut().enumerate() {
            let raw = if playing && level.is_finite() {
                level.max(0.)
            } else {
                0.
            };
            self.level_average[i] += (raw - self.level_average[i]) * (1. - (-dt * 0.8).exp());
            // Soft compression preserves changes in loud music instead of pinning
            // RMS/bass/treble to 1. Adapt to quieter recordings without amplifying silence.
            let reference = self.level_average[i].max([0.02, 0.006, 0.008][i]);
            let signal = (raw - 0.0005).max(0.) * sensitivity;
            *level = signal / (signal + reference * 1.4);
        }
        let [energy, bass, treble] = levels;
        // Magnify small deviations from each band's recent level, rather than
        // boosting loudness until all LEDs saturate. All motion comes from audio.
        for ((envelope, baseline), band) in self
            .meter_bands
            .iter_mut()
            .zip(self.meter_baseline.iter_mut())
            .zip([bass, treble])
        {
            let target = if playing && band > 0.02 {
                (0.5 + 0.5 * ((band - *baseline) * 30.).tanh()) * (band / 0.08).clamp(0., 1.)
            } else {
                0.
            };
            if playing && band > 0.02 {
                *baseline += (band - *baseline) * (1. - (-dt * 1.2).exp());
            }
            let rate = if target > *envelope { 90. } else { 55. };
            *envelope += (target - *envelope) * (1. - (-dt * rate).exp());
        }
        let smoothing = 1. - (-dt * 10.).exp();
        self.energy += (energy - self.energy) * smoothing;
        self.bass += (bass - self.bass) * smoothing;
        self.treble += (treble - self.treble) * smoothing;
        self.beat_pulse *= (-dt * 6.).exp();
        self.since_beat += dt;
        if self.auto && playing && energy > 0.015 {
            self.since_remix += dt;
        }
        let threshold = (self.bass_average * 1.18).max(0.12);
        let beat = self.auto
            && playing
            && bass > threshold
            && self.previous_bass <= threshold
            && self.since_beat > 0.28;
        self.bass_average += (bass - self.bass_average) * (1. - (-dt * 1.5).exp());
        self.previous_bass = bass;
        if beat {
            self.beats += 1;
            self.beat_pulse = 1.;
            self.since_beat = 0.;
        }
        if self.auto
            && playing
            && energy > 0.015
            && self.since_remix >= 4.
            && ((beat && self.beats >= 8) || self.since_remix >= 16.)
        {
            self.remix();
        }
        // A fast independent envelope lets fine band changes alter symmetry
        // immediately, even during a slow transition between pattern styles.
        let target_symmetry = if self.auto && playing && energy > 0.015 {
            let [bass, treble] = self.meter_bands;
            14. * ((bass - treble) * 3.).tanh() + 4. * (bass + treble - 1.) + 8. * self.beat_pulse
        } else {
            0.
        };
        if self.auto {
            let rate = if target_symmetry.abs() > self.music_symmetry.abs() {
                26.
            } else {
                16.
            };
            self.music_symmetry +=
                (target_symmetry - self.music_symmetry) * (1. - (-dt * rate).exp());
        } else {
            self.music_symmetry = 0.;
        }
        let direction = self.direction();
        let angle_delta = self.choreography.update(
            dt,
            playing && energy > 0.015,
            self.auto,
            self.motion,
            direction,
            self.energy,
            self.meter_bands,
            beat,
        );
        self.rotation += angle_delta;
        let settings = self.music_settings();
        self.phase += dt * settings.motion * direction;
        self.color_phase += dt * settings.chroma;
        let target_hue = if self.auto && playing && energy > 0.015 {
            let [bass, treble] = self.meter_bands;
            (120. * (treble - bass) + 80. * self.beat_pulse + 30. * self.energy).clamp(-160., 160.)
        } else {
            0.
        };
        if self.auto {
            self.music_hue += (target_hue - self.music_hue) * (1. - (-dt * 6.).exp());
        } else {
            self.music_hue = 0.;
        }
        if let Some(previous) = &mut self.previous {
            // Both images turn together throughout a pattern crossfade.
            previous.rotation += angle_delta;
            previous.phase += dt * previous.music_settings().motion * direction;
            previous.color_phase += dt * previous.music_settings().chroma;
            previous.music_hue = self.music_hue;
            previous.music_symmetry = self.music_symmetry;
        }
    }

    fn music_settings(&self) -> MusicSettings {
        if !self.auto || self.energy < 0.015 {
            return MusicSettings {
                folds: ((self.visual_folds() / 2.).round() as u32 * 2).clamp(4, 28),
                motion: self.motion,
                complexity: self.complexity,
                chroma: self.chroma,
            };
        }
        // Reuse the LEDs' fast, high-contrast band envelopes for visible motion.
        let [bass, treble] = self.meter_bands;
        MusicSettings {
            folds: ((self.visual_folds() / 2.).round() as u32 * 2).clamp(4, 28),
            motion: (self.motion * (0.15 + self.energy * 0.5 + bass * 2.2 + self.beat_pulse * 0.8))
                .clamp(0., 2.),
            // Density and shape scale stay fixed; audio drives motion and hue.
            complexity: self.complexity,
            chroma: (self.chroma * (0.15 + treble * 2.2 + self.beat_pulse * 0.5)).clamp(0., 2.),
        }
    }

    fn visual_folds(&self) -> f32 {
        (self.folds as f32 + if self.auto { self.music_symmetry } else { 0. }).clamp(4., 28.)
    }

    fn symmetry_blend(&self) -> (u32, u32, f32) {
        let folds = self.visual_folds();
        let low = (folds / 2.).floor() as u32 * 2;
        (low, (low + 2).min(28), (folds - low as f32) * 0.5)
    }

    fn radius(&self, size: Vec2) -> f32 {
        size.x.min(size.y) * 0.43
    }

    pub fn visualizer_choices(&mut self, ui: &mut egui::Ui) -> bool {
        let previous = self.milkdrop;
        icons::menu_choice(ui, &mut self.milkdrop, false, Icon::Meter, "Kaleido Visualizer");
        icons::menu_choice(ui, &mut self.milkdrop, true, Icon::Music, "MilkDrop Visualizer");
        self.milkdrop != previous
    }

    pub fn controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Visualizer");
            self.visualizer_choices(ui);
        });
        ui.separator();
        if self.milkdrop {
            self.milkdrop_controls(ui);
        } else {
            self.kaleidoradio_controls(ui);
        }
    }

    fn kaleidoradio_controls(&mut self, ui: &mut egui::Ui) {
        if ui.checkbox(&mut self.auto, "React to Music").changed() {
            self.previous = None;
            self.transition = 1.;
            self.choreography = musical_motion::Motion::default();
            self.music_hue = 0.;
            self.pending_pattern = None;
            self.pending_folds = None;
            self.pending_remix = false;
            self.music_symmetry = 0.;
            self.beat_pulse = 0.;
            self.beats = 0;
            self.since_remix = 0.;
        }
        ui.horizontal(|ui| {
            for (label, level) in [
                ("Level", self.energy),
                ("Bass", self.bass),
                ("Treble", self.treble),
            ] {
                ui.add(egui::ProgressBar::new(level).desired_width(90.).text(label));
            }
        });
        let live = self.music_settings();
        ui.label(format!(
            "Live: {} folds · motion {:.2} · hue {:.0}° · {} {}",
            live.folds,
            self.choreography.velocity.abs(),
            self.effective_hue().rem_euclid(360.),
            if self.auto {
                self.choreography.name()
            } else {
                "Steady"
            },
            if self.choreography.velocity.abs() < 0.05 {
                "Turning"
            } else if self.choreography.velocity > 0. {
                "Clockwise"
            } else {
                "Counterclockwise"
            }
        ));
        ui.small(if self.auto {
            "Music drives spins, swings, hue, and fast symmetry changes with smooth motion."
        } else {
            "Steady animation using your settings. Remix manually whenever you like."
        });
        if icons::menu_item(ui, Icon::Refresh, "Remix Now").clicked() {
            self.remix();
        }
        egui::Grid::new("visualizer-controls")
            .spacing(egui::vec2(16.0, 8.0))
            .show(ui, |ui| {
                ui.label("Pattern");
                let mut pattern = self.pattern;
                egui::ComboBox::from_id_salt("kaleido-pattern")
                    .selected_text(PATTERNS[pattern])
                    .show_ui(ui, |ui| {
                        for (i, name) in PATTERNS.iter().enumerate() {
                            icons::menu_choice(ui, &mut pattern, i, Icon::Music, name);
                        }
                    });
                if pattern != self.pattern {
                    self.select_pattern(pattern);
                }
                ui.end_row();
                ui.label(if self.auto { "Base Hue" } else { "Hue" });
                ui.add(egui::Slider::new(&mut self.hue, 0.0..=360.0).suffix("°"));
                ui.end_row();
                ui.label(if self.auto {
                    "Base Symmetry"
                } else {
                    "Symmetry"
                });
                let mut folds = self.pending_folds.unwrap_or(self.folds);
                if ui
                    .add(egui::Slider::new(&mut folds, 4..=28).step_by(2.))
                    .changed()
                {
                    if self.previous.is_none() {
                        self.begin_transition();
                        self.folds = folds;
                    } else {
                        self.pending_folds = Some(folds);
                    }
                }
                ui.end_row();
                ui.label(if self.auto {
                    "Base Direction"
                } else {
                    "Direction"
                });
                egui::ComboBox::from_id_salt("kaleido-direction")
                    .selected_text(if self.clockwise {
                        "Clockwise"
                    } else {
                        "Counterclockwise"
                    })
                    .show_ui(ui, |ui| {
                        icons::menu_choice(ui, &mut self.clockwise, true, Icon::Refresh, "Clockwise");
                        icons::menu_choice(ui, &mut self.clockwise, false, Icon::RepeatOne, "Counterclockwise");
                    });
                ui.end_row();
                ui.label("Motion");
                ui.add(egui::Slider::new(&mut self.motion, 0.0..=2.0).fixed_decimals(2));
                ui.end_row();
            });
    }

    fn milkdrop_controls(&mut self, ui: &mut egui::Ui) {
        self.milkdrop_pool_menu(ui);
        ui.separator();
        if self.projectm.is_some() {
            if self.milkdrop_preset_available(self.milkdrop_preset) {
                self.milkdrop_picker(ui);
                ui.horizontal(|ui| self.milkdrop_navigation(ui));
            } else if self.projectm_presets.is_empty() {
                ui.label("No .milk presets found in presets/.");
            } else if self.milkdrop_favorites_only {
                ui.label("No enabled favorite presets. Choose All Presets to add favorites.");
            } else {
                ui.label("All MilkDrop presets are disabled.");
            }
            ui.checkbox(&mut self.milkdrop_random, "Randomize Auto-Advance");
            if ui.add(
                egui::Slider::new(&mut self.milkdrop_seconds, 0..=MILKDROP_MAX_SECONDS)
                    .text("Auto-Advance")
                    .step_by(1.0)
                    .custom_formatter(|value, _| {
                        let seconds = value as u32;
                        if seconds == 0 {
                            "Off".to_owned()
                        } else {
                            format!("{}:{:02}", seconds / 60, seconds % 60)
                        }
                    })
                    .custom_parser(|value| {
                        if value.eq_ignore_ascii_case("off") {
                            Some(0.0)
                        } else if let Some((minutes, seconds)) = value.split_once(':') {
                            Some(minutes.parse::<f64>().ok()? * 60.0 + seconds.parse::<f64>().ok()?)
                        } else {
                            value.parse().ok()
                        }
                    }),
            ).changed() {
                self.milkdrop_timer = 0.0;
            }
            self.milkdrop_preset_actions(ui);
            self.disabled_milkdrop_presets_ui(ui);
        } else if let Some(error) = &self.projectm_error {
            ui.label(format!("projectM unavailable: {error}"));
        } else {
            ui.label("projectM not initialised.");
        }
    }

    fn disabled_milkdrop_presets_ui(&mut self, ui: &mut egui::Ui) {
        if self.milkdrop_disabled.is_empty() {
            return;
        }
        let mut reenable = None;
        let label = format!("Disabled Presets ({})", self.milkdrop_disabled.len());
        egui::CollapsingHeader::new(label)
            .id_salt("disabled-milkdrop-presets")
            .show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    for (index, name) in self.milkdrop_disabled.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if icons::menu_item(ui, Icon::Refresh, "Re-enable").clicked() {
                                reenable = Some(index);
                            }
                            ui.label(name);
                        });
                    }
                });
            });
        if let Some(index) = reenable {
            let was_available = self.milkdrop_preset_available(self.milkdrop_preset);
            self.milkdrop_disabled.remove(index);
            if !self.milkdrop_preset_available(self.milkdrop_preset) {
                self.advance_milkdrop_preset();
            } else if !was_available {
                self.set_milkdrop_preset(self.milkdrop_preset);
            }
        }
    }

    fn milkdrop_picker(&mut self, ui: &mut egui::Ui) {
        let mut preset = self.milkdrop_preset;
        let label = "MilkDrop Preset";
        let selected = &self.projectm_presets[preset];
        ui.label(label);
        egui::ComboBox::from_id_salt(label)
            .width(ui.available_width().min(360.))
            .truncate()
            .selected_text(selected)
            .show_ui(ui, |ui| {
                let mut has_presets = false;
                for (index, name) in self.projectm_presets.iter().enumerate() {
                    if self.milkdrop_preset_available(index) {
                        let icon = if self.milkdrop_favorites.contains(name) { Icon::Star } else { Icon::Music };
                        icons::menu_choice(ui, &mut preset, index, icon, name);
                        has_presets = true;
                    }
                }
                if !has_presets {
                    ui.label("No enabled favorites yet.");
                }
            });
        if preset != self.milkdrop_preset {
            self.set_milkdrop_preset(preset);
        }
    }

    fn meter_lit_segments(level: f32, volume: f32, segments: usize) -> usize {
        let level = level.clamp(0., 1.);
        (level * volume.clamp(0., 1.) * segments as f32).round() as usize
    }

    /// Two LED rows: bass above treble, with fast envelopes from the live audio analysis.
    pub fn draw_band_meter(&self, ui: &mut egui::Ui, width: f32, volume: f32) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 64.), egui::Sense::hover());
        response.on_hover_hint("Bass (top) · Treble (bottom) · Lit length follows volume");
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6., Color32::from_rgb(8, 7, 8));
        let segments = 42;
        let padding = 8.;
        let pitch = (width - padding * 2.) / segments as f32;
        let bar_width = (pitch * 0.64).max(1.);
        let bar_height = 21.;
        for (row, level) in self.meter_bands.into_iter().enumerate() {
            // Expand ordinary band levels with headroom instead of clipping
            // stronger signals to a constant full row.
            let lit = Self::meter_lit_segments(level, volume, segments);
            for segment in 0..segments {
                let color = if segment >= 36 {
                    Color32::from_rgb(255, 35, 16)
                } else if segment >= 22 {
                    Color32::from_rgb(239, 255, 25)
                } else {
                    Color32::from_rgb(57, 255, 20)
                };
                let bar = egui::Rect::from_min_size(
                    rect.min + Vec2::new(padding + segment as f32 * pitch, 8. + row as f32 * 27.),
                    Vec2::new(bar_width, bar_height),
                );
                let active = segment < lit;
                if active {
                    painter.rect_filled(bar.expand(1.5), 2., color.gamma_multiply(0.12));
                }
                painter.rect_filled(
                    bar,
                    1.2,
                    color.gamma_multiply(if active { 1. } else { 0.10 }),
                );
            }
        }
    }

    pub fn reset_and_remix(&mut self) {
        self.remix();
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) -> egui::Response {
        self.draw_interactive(ui, true)
    }

    pub fn draw_inline(&mut self, ui: &mut egui::Ui, backdrop: Color32) -> egui::Response {
        let response = self.draw_interactive(ui, false);
        // egui clips painters to rectangles. Mask the four corners with the
        // surrounding card color so every animated layer follows the artwork edge.
        let rect = response.rect;
        let radius = crate::theme::MEDIA_CORNER_RADIUS
            .min(rect.width() * 0.5)
            .min(rect.height() * 0.5);
        let fringe = (1.0 / ui.ctx().pixels_per_point()).min(radius);
        // Scissors round to physical pixels. Extend the opaque mask beyond
        // its rectangular bounds so fractional edges cannot leave a bright seam.
        let bleed = 2.0 / ui.ctx().pixels_per_point();
        let mut mesh = egui::Mesh::default();
        for (corner, direction) in [
            (rect.left_top(), Vec2::new(1., 1.)),
            (rect.right_top(), Vec2::new(-1., 1.)),
            (rect.right_bottom(), Vec2::new(-1., -1.)),
            (rect.left_bottom(), Vec2::new(1., -1.)),
        ] {
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(corner - direction * bleed, backdrop);
            for step in 0..=24 {
                let angle = step as f32 / 24. * std::f32::consts::FRAC_PI_2;
                let arc = Vec2::new(angle.cos(), angle.sin());
                let center = corner + direction * radius;
                mesh.colored_vertex(center - direction * arc * radius, backdrop);
                mesh.colored_vertex(
                    center - direction * arc * (radius - fringe),
                    Color32::TRANSPARENT,
                );
                if step > 0 {
                    let previous = base + 1 + (step - 1) * 2;
                    let current = base + 1 + step * 2;
                    mesh.add_triangle(base, previous, current);
                    mesh.add_triangle(previous, previous + 1, current);
                    mesh.add_triangle(previous + 1, current + 1, current);
                }
            }
            let outer_start = mesh.vertices.len() as u32;
            mesh.colored_vertex(corner + direction * Vec2::new(-bleed, radius), backdrop);
            mesh.add_triangle(base, outer_start, base + 1);
            let outer_end = mesh.vertices.len() as u32;
            mesh.colored_vertex(corner + direction * Vec2::new(radius, -bleed), backdrop);
            mesh.add_triangle(base, base + 1 + 24 * 2, outer_end);
        }
        ui.painter_at(rect).add(egui::Shape::mesh(mesh));
        response
    }
    fn draw_interactive(
        &mut self,
        ui: &mut egui::Ui,
        remix_on_double_click: bool,
    ) -> egui::Response {
        let size = ui.available_size().max(Vec2::splat(80.));
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        if remix_on_double_click && response.double_clicked() {
            self.reset_and_remix();
        }
        let target = response
            .hover_pos()
            .map_or(Vec2::ZERO, |p| (p - rect.center()) / (size * 0.5));
        self.pointer += (target - self.pointer) * (1. - (-self.frame_dt * 5.).exp());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0., Color32::from_rgb(6, 4, 9));
        let blend = self.transition * self.transition * (3. - 2. * self.transition);
        if let Some(previous) = &self.previous {
            let mut old_painter = painter.clone();
            // Opaque procedural meshes blend over the outgoing image directly.
            // Fading both would darken the middle of the transition.
            if previous.pattern == 0 || self.pattern == 0 {
                old_painter.multiply_opacity(1. - blend);
            }
            previous.paint(&old_painter, rect, size);
        }
        let mut painter = painter;
        painter.multiply_opacity(if self.previous.is_some() { blend } else { 1. });
        self.paint(&painter, rect, size);
        response
    }

    /// Render at physical pixel resolution on the GPU. The mesh path remains
    /// available for headless tests or systems where shader setup fails.
    fn paint_prism(&self, painter: &egui::Painter, rect: egui::Rect, size: Vec2) {
        if let Some(renderer) = &self.renderer {
            let palette = PALETTES[self.palette];
            renderer.paint(
                painter,
                rect,
                prism_renderer::Scene {
                    mode: self.pattern as f32 - 1.,
                    seed: self.seed,
                    folds: self.visual_folds(),
                    rotation: self.rotation,
                    hue: (self.effective_hue() + palette[0]) / 360.,
                    opacity: painter.opacity(),
                },
            );
            return;
        }
        let scale = size.x.min(size.y) / 2.15;
        let columns = (size.x / 3.).clamp(32., 256.) as u32;
        let rows = (size.y / 3.).clamp(32., 256.) as u32;
        let mut mesh = egui::Mesh::default();
        mesh.vertices.reserve(((columns + 1) * (rows + 1)) as usize);
        mesh.indices.reserve((columns * rows * 6) as usize);
        let settings = self.music_settings();
        for y in 0..=rows {
            for x in 0..=columns {
                let pos =
                    rect.min + size * Vec2::new(x as f32 / columns as f32, y as f32 / rows as f32);
                mesh.colored_vertex(
                    pos,
                    self.prism_color((pos - rect.center()) / scale, settings),
                );
                if x < columns && y < rows {
                    let i = y * (columns + 1) + x;
                    mesh.add_triangle(i, i + 1, i + columns + 1);
                    mesh.add_triangle(i + 1, i + columns + 2, i + columns + 1);
                }
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    fn prism_color(&self, uv: Vec2, mut settings: MusicSettings) -> Color32 {
        let (low, high, blend) = self.symmetry_blend();
        settings.folds = low;
        let a = self.prism_color_at_folds(uv, settings);
        if blend <= 0.001 {
            return a;
        }
        settings.folds = high;
        let b = self.prism_color_at_folds(uv, settings);
        let mix = |a: u8, b: u8| (a as f32 * (1. - blend) + b as f32 * blend).round() as u8;
        Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
    }

    fn prism_color_at_folds(&self, uv: Vec2, settings: MusicSettings) -> Color32 {
        let r = uv.length();
        let n = settings.folds as f32;
        let wedge = TAU / n;
        let a = ((uv.y.atan2(uv.x) - self.rotation + self.seed + wedge * 0.5).rem_euclid(wedge)
            - wedge * 0.5)
            .abs();
        let p = Vec2::angled(a) * r;
        // Animate the angular coordinate above, not the radial field. Time in
        // radial wave phases reads as a zoom even when the canvas scale is fixed.
        let t = self.seed * 0.16;
        let palette = PALETTES[self.palette];
        let h = (self.effective_hue() + palette[0]) / 360. + self.seed * 0.13;
        let detail = 1.;
        let palette_color = |v: f32| -> [f32; 3] {
            [0., 0.33, 0.67].map(|offset| 0.52 + 0.48 * (TAU * (offset + v) + self.seed).cos())
        };
        let mut col = [0.; 3];
        let mut add = |color: [f32; 3], strength: f32| {
            for i in 0..3 {
                col[i] += color[i] * strength;
            }
        };
        match self.pattern {
            1 => {
                let petals = (a * n).cos() * 0.5 + 0.5;
                let wave =
                    (r * 24. * detail - t * 2. + (a * n).sin() * 2.8 + (r * 9. + t).sin()).sin();
                let veins = (r * 36. * detail + petals * 5. + t).sin().abs();
                let bead = (r * 19. - t + petals * 3.).sin().max(0.).powi(10);
                add(
                    palette_color(r * 0.48 + wave * 0.16 + h),
                    0.16 + 0.85 * wave.abs().powi(2),
                );
                add(palette_color(r * 0.3 + h + 0.3), veins.powi(24) * 0.9);
                add([0.6, 0.85, 1.], bead * petals.powi(7) * 0.7);
            }
            2 | 4 => {
                let mut q = p * if self.pattern == 2 { 11. } else { 18. } * detail;
                q += Vec2::new((r * 4. + t).sin() * 0.55, t * 0.4);
                q.x += 0.65 * (q.y * 0.55 + self.seed).sin();
                let cell = q.floor();
                let f = q - cell;
                let diagonal = if f.x < f.y { 13. } else { 0. };
                let rnd = prism_hash(cell + Vec2::splat(self.seed + diagonal));
                let edge =
                    f.x.min(1. - f.x)
                        .min(f.y.min(1. - f.y))
                        .min((f.x - f.y).abs() * 0.707);
                let lead = smoothstep(0.015, 0.055, edge);
                add(
                    palette_color(rnd * 0.9 + h),
                    (0.55 + rnd * 0.55)
                        * if self.pattern == 2 {
                            lead
                        } else {
                            0.7 + 0.3 * lead
                        },
                );
            }
            3 => {
                let petal = (a * n * 0.5).cos().abs().powi(3);
                let bands = (r * 22. * detail - petal * 5. - t).sin();
                let gem = (1. - bands.abs()).max(0.).powi(5);
                let fade = 1. - smoothstep(0.65, 1.32, r);
                add(palette_color(r * 0.8 + petal * 0.25 + h), gem * 1.4 * fade);
                add([0.65, 0.9, 1.], gem.powi(12) * 0.65 * fade);
                add(
                    palette_color(r + h + 0.4),
                    (r * 33. + petal * 6. + t).sin().abs().powi(40) * 0.65 * fade,
                );
            }
            5 => {
                let warp = (r * 12. - t).sin() * 0.13;
                let lace = (a * n * 1.5 + (r * 18. * detail + t).sin() * 2.)
                    .sin()
                    .abs();
                let ring = (r * 28. * detail + (a * n).sin() * 2. + t).sin().abs();
                add(
                    palette_color(r * 0.6 + h),
                    0.08 + 0.9 * lace.powi(20) + 0.75 * ring.powi(26),
                );
                add(
                    palette_color(r + h + 0.45),
                    (r * 16. + warp * 12. + (a * n).cos() * 4.)
                        .sin()
                        .abs()
                        .powi(30),
                );
            }
            6 => {
                let flow = (p.x * 15. * detail + (p.y * 12. + t).sin() * 2.).sin()
                    + (r * 19. - t * 1.3).cos();
                let ripple = (flow * 2.4 + (a * n).cos() * 2. + t).sin().abs();
                add(
                    palette_color(flow * 0.12 + r * 0.25 + h),
                    0.35 + 0.65 * ripple.powi(3),
                );
                add([0.7, 0.85, 1.], ripple.powi(32) * 0.7);
            }
            7 => {
                let depth = -(r + 0.055).ln();
                let web = (depth * 14. * detail + (a * n).sin() * 2. - t * 2.)
                    .sin()
                    .abs();
                let spoke = (a * n * 2. + depth * 3. + t).sin().abs();
                add(
                    palette_color(depth * 0.22 + h),
                    0.8 * web.powi(35) + 0.65 * spoke.powi(45),
                );
                add(
                    [0.8, 0.9, 1.],
                    ((depth * 31. - t).sin() * (a * n * 3.).cos())
                        .max(0.)
                        .powi(24)
                        * 1.8,
                );
            }
            8 => {
                let lobe = (a * n).cos();
                let fold = (r * 17. * detail - lobe * 3.5 + (r * 5. - t).sin() * 2. - t).sin();
                let inner = (r * 29. * detail + lobe * 5. + t * 0.7).sin().abs();
                add(
                    palette_color(r * 0.65 + fold * 0.15 + h),
                    (0.15 + 0.85 * fold.abs().powf(0.65))
                        * (0.3 + 0.7 * smoothstep(0.03, 0.3, inner)),
                );
                add(palette_color(h + r + 0.35), inner.powi(48) * 0.9);
            }
            9 => {
                let q = p * 16. * detail + Vec2::new(t * 0.35, (t * 0.4).sin());
                let cell = q.floor();
                let f = q - cell - Vec2::splat(0.5);
                let path = if prism_hash(cell + Vec2::splat(self.seed)) > 0.5 {
                    (f.x.abs().max(f.y.abs()) - 0.32).abs()
                } else {
                    f.x.abs().min(f.y.abs())
                };
                add(
                    palette_color(prism_hash(cell + Vec2::splat(self.seed)) * 0.3 + r * 0.4 + h),
                    (1. - smoothstep(0.035, 0.09, path)) * 0.95 + 0.035,
                );
                add([0.65, 0.85, 1.], (1. - smoothstep(0., 0.018, path)) * 0.45);
            }
            _ => {
                let thread = (r * 48. * detail + (a * n).cos() * 5. + (r * 8. - t).sin() * 2.)
                    .sin()
                    .abs();
                let braid = (a * n * 3. + r * 12. + t).sin().abs();
                // Rotate the golden reference palette with the user hue as well.
                let gold = neon(35. + self.effective_hue(), 1., 0.6, 1.);
                let gold = [gold.r(), gold.g(), gold.b()].map(|c| c as f32 / 255.);
                let tint = palette_color(r * 0.3 + h);
                let mix = 0.3 + 0.2 * (t * 0.3).sin();
                add(
                    std::array::from_fn(|i| gold[i] * (1. - mix) + tint[i] * mix),
                    0.08 + 0.85 * thread.powi(28) + 0.6 * braid.powi(36),
                );
                add(palette_color(h + 0.12), (thread * braid).powi(20) * 0.8);
            }
        }
        add(palette_color(h + 0.2), (-r * 35.).exp() * 0.9);
        let fade = 1. - 0.22 * smoothstep(0.65, 1.6, r);
        let rgb =
            col.map(|c| ((1. - (-c * fade * 1.35).exp()).powf(0.88) * 255.).clamp(0., 255.) as u8);
        Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }

    fn paint(&self, painter: &egui::Painter, rect: egui::Rect, size: Vec2) {
        if self.milkdrop && self.milkdrop_favorites_only
            && !self.milkdrop_preset_available(self.milkdrop_preset)
        {
            painter.rect_filled(rect, 0.0, Color32::BLACK);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No Enabled Favorite Presets",
                egui::FontId::proportional(16.0),
                Color32::WHITE,
            );
            return;
        }

        if self.milkdrop && self.milkdrop_preset_available(self.milkdrop_preset) {
            if let Some(renderer) = &self.projectm {
                renderer.paint(
                    painter,
                    rect,
                    projectm::Metrics {
                        bass: self.bass,
                        mid: self.energy,
                        treble: self.treble,
                        beat: self.beat_pulse,
                        level: self.energy,
                    },
                );
                return;
            }
        }
        if self.pattern != 0 {
            self.paint_prism(painter, rect, size);
            return;
        }
        let (low, high, blend) = self.symmetry_blend();
        let mut settings = self.music_settings();
        settings.folds = low;
        let mut low_painter = painter.clone();
        low_painter.multiply_opacity(1. - blend);
        self.paint_petals(&low_painter, rect, size, settings);
        if blend > 0.001 {
            settings.folds = high;
            let mut high_painter = painter.clone();
            high_painter.multiply_opacity(blend);
            self.paint_petals(&high_painter, rect, size, settings);
        }
    }

    fn paint_petals(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        size: Vec2,
        settings: MusicSettings,
    ) {
        let treble = if self.auto { self.meter_bands[1] } else { 0. };
        let center = rect.center();
        let radius = self.radius(size);
        let t = self.phase;
        let mut p = PALETTES[self.palette];
        p[0] += self.effective_hue();
        let angle = TAU / settings.folds as f32;
        for n in 0..3 {
            let n = n as f32;
            let point = center
                + Vec2::new(
                    (t * 0.1 + n * 2.1).cos() * size.x * 0.18,
                    (t * 0.13 + n * 1.7).sin() * size.y * 0.16,
                );
            glow(
                &painter,
                point,
                radius * 1.6,
                p[0] + t * p[1] * 0.12 + n * 120.,
                0.09,
            );
        }
        // Cover the corners even on wide windows, at a fixed visual scale.
        let background_radius = size.length() * 0.5 / (TAU / 192.).cos();
        let mut swirl = egui::Mesh::default();
        swirl.colored_vertex(center, Color32::TRANSPARENT);
        for i in 0..=96 {
            let a = i as f32 / 96. * TAU;
            swirl.colored_vertex(
                center + Vec2::angled(a + t * 0.06) * background_radius,
                neon(
                    p[0] + i as f32 / 96. * 360. + t * p[1] * 0.25,
                    p[6],
                    0.42,
                    0.06,
                ),
            );
            if i > 0 {
                swirl.add_triangle(0, i, i + 1);
            }
        }
        painter.add(egui::Shape::mesh(swirl));
        // Keep topology stable as audio changes; complexity still bends and colors petals.
        let layers = 16;
        for sector in 0..settings.folds {
            let rotation = sector as f32 * angle + self.rotation;
            let mirror = if sector % 2 == 0 { 1. } else { -1. };
            let transform = |v: Vec2| center + rotate(Vec2::new(v.x, v.y * mirror), rotation);
            for j in (0..=layers).rev() {
                let progress = (j + 1) as f32 / (layers + 1) as f32;
                let j = j as f32;
                let r = radius * progress;
                let pulse = (t * 0.6 + j * 0.85 + self.seed).sin();
                let bend = (self.pointer.x * 0.13
                    + (t * 0.3 + j * 0.7 + self.seed).sin() * 0.16
                    + (t * 0.9 + j * 1.3).sin() * 0.08)
                    * settings.complexity;
                let spread = angle
                    * (0.26 + 0.16 * (j * 0.8 + t * 0.25).sin())
                    * (0.85 + settings.complexity * 0.3);
                let hue = p[0]
                    + j * p[2]
                    + sector as f32 / settings.folds as f32 * p[5]
                    + progress * p[4];
                let start = Vec2::new(r * 0.18, 0.);
                let tip = Vec2::new(r, 0.);
                let a = Vec2::new(r * 0.42, -r * spread * (1.5 + pulse * 0.3));
                let b = Vec2::new(r * 0.85, -r * spread * 0.8);
                let mut boundary = Vec::with_capacity(40);
                for side in 0..2 {
                    for step in 0..20 {
                        let u = step as f32 / 19.;
                        let v = if side == 0 {
                            cubic(start, a, b, tip, u)
                        } else {
                            cubic(tip, Vec2::new(b.x, -b.y), Vec2::new(a.x, -a.y), start, u)
                        };
                        boundary.push(rotate(v, bend));
                    }
                }
                let clipped = clip_wedge(boundary, angle * 0.5);
                if clipped.len() < 3 {
                    continue;
                }
                let centroid = clipped.iter().copied().fold(Vec2::ZERO, |sum, v| sum + v)
                    / clipped.len() as f32;
                let mut fill = egui::Mesh::default();
                let base = 0;
                let shade = |v: Vec2| {
                    let fraction = (v.length() / r).clamp(0., 1.);
                    let mix = (1. - ((fraction - 0.6) / 0.6).abs()).max(0.);
                    petal_color(
                        hue + p[3] * mix,
                        p[6],
                        0.5,
                        0.1 + mix * (0.22 + settings.complexity * 0.3),
                    )
                };
                fill.colored_vertex(transform(centroid), shade(centroid));
                for v in &clipped {
                    fill.colored_vertex(transform(*v), shade(*v));
                }
                for k in 0..clipped.len() as u32 {
                    fill.add_triangle(
                        base,
                        base + 1 + k,
                        base + 1 + (k + 1) % clipped.len() as u32,
                    );
                }
                painter.add(egui::Shape::mesh(fill));
                // Clip strokes too, so alternating sectors keep their mirrored seams.
                for edge in clipped.windows(2) {
                    let points = [transform(edge[0]), transform(edge[1])];
                    painter.line_segment(
                        points,
                        Stroke::new(5. - progress * 2., neon(hue, p[6], 0.55, 0.08)),
                    );
                    painter.line_segment(
                        points,
                        Stroke::new(
                            0.8 + (1. - progress) * 1.1 + treble * 0.7,
                            neon(hue, p[6], 0.6, 0.75),
                        ),
                    );
                }
                let filament: Vec<_> = (0..16)
                    .map(|k| {
                        let u = k as f32 / 15.;
                        let a = Vec2::new(r * 0.23, 0.);
                        let b = Vec2::new(r * 0.6, r * spread * 0.4 * (t * 0.4 + j).sin());
                        let c = Vec2::new(r * 0.97, 0.);
                        rotate(
                            a * (1. - u).powi(2) + b * (2. * (1. - u) * u) + c * u * u,
                            bend,
                        )
                    })
                    .collect();
                for edge in filament.windows(2) {
                    if inside(edge[0], angle * 0.5) && inside(edge[1], angle * 0.5) {
                        painter.line_segment(
                            [transform(edge[0]), transform(edge[1])],
                            Stroke::new(0.7, neon(hue + p[3], 1., 0.88, 0.38)),
                        );
                    }
                }
                if (j as u32).is_multiple_of(3) {
                    let ring_radius = (r * angle * 0.13 * (1. + self.pointer.y * 0.2)).max(1.);
                    for k in 0..32 {
                        let point = |k: u32| {
                            rotate(
                                Vec2::new(r * 0.72, 0.)
                                    + Vec2::angled(k as f32 / 32. * TAU) * ring_radius,
                                bend,
                            )
                        };
                        let a = point(k);
                        let b = point(k + 1);
                        if inside(a, angle * 0.5) && inside(b, angle * 0.5) {
                            painter.line_segment(
                                [transform(a), transform(b)],
                                Stroke::new(1., neon(hue + p[3], p[6], 0.7, 0.38)),
                            );
                        }
                    }
                }
                for d in 0..2 {
                    let u = (d + 1) as f32 / 3.;
                    let v = rotate(
                        Vec2::new(
                            r * (0.25 + 0.62 * u),
                            (t * 3. + j + d as f32 * 2.1).sin() * r * 0.12 * u,
                        ),
                        bend,
                    );
                    if inside(v, angle * 0.5) {
                        painter.circle_filled(
                            transform(v),
                            1.1 + (1. - progress) * 1.6,
                            neon(hue + p[3] + d as f32 * 50., 1., 0.8, 0.65 + treble * 0.2),
                        );
                    }
                }
            }
        }
        let core = p[0];
        glow(&painter, center, 64., core, 0.5);
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(center, Color32::WHITE);
        for i in 0..=64 {
            let a = i as f32 / 64. * TAU;
            mesh.colored_vertex(
                center + Vec2::angled(a) * 22.,
                neon(core + i as f32 / 64. * 360., 1., 0.62, 1.),
            );
            if i > 0 {
                mesh.add_triangle(0, i, i + 1);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
        painter.circle_filled(center, 4.5, Color32::WHITE);
    }
}
fn smoothstep(low: f32, high: f32, value: f32) -> f32 {
    let t = ((value - low) / (high - low)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
fn prism_hash(p: Vec2) -> f32 {
    ((p.x * 127.1 + p.y * 311.7).sin() * 43758.547).rem_euclid(1.)
}
fn rotate(v: Vec2, a: f32) -> Vec2 {
    Vec2::new(v.x * a.cos() - v.y * a.sin(), v.x * a.sin() + v.y * a.cos())
}
fn cubic(a: Vec2, b: Vec2, c: Vec2, d: Vec2, t: f32) -> Vec2 {
    a * (1. - t).powi(3)
        + b * (3. * (1. - t).powi(2) * t)
        + c * (3. * (1. - t) * t * t)
        + d * t.powi(3)
}
fn inside(v: Vec2, half: f32) -> bool {
    v.x >= 0. && v.y.abs() <= v.x * half.tan()
}
fn clip_wedge(mut points: Vec<Vec2>, half: f32) -> Vec<Vec2> {
    for sign in [-1., 1.] {
        let distance = |v: Vec2| v.x * half.sin() + sign * v.y * half.cos();
        let input = std::mem::take(&mut points);
        if input.is_empty() {
            break;
        }
        let mut previous = *input.last().unwrap();
        for current in input {
            let a = distance(previous);
            let b = distance(current);
            if (a >= 0.) != (b >= 0.) {
                points.push(previous + (current - previous) * (a / (a - b)));
            }
            if b >= 0. {
                points.push(current);
            }
            previous = current;
        }
    }
    points
}
// HSL like the HTML source, emitted with additive blending for canvas 'screen'.
fn neon(hue: f32, saturation: f32, light: f32, alpha: f32) -> Color32 {
    let h = hue.rem_euclid(360.) / 60.;
    let c = (1. - (2. * light - 1.).abs()) * saturation;
    let x = c * (1. - (h.rem_euclid(2.) - 1.).abs());
    let rgb = match h as u32 {
        0 => [c, x, 0.],
        1 => [x, c, 0.],
        2 => [0., c, x],
        3 => [0., x, c],
        4 => [x, 0., c],
        _ => [c, 0., x],
    };
    let m = light - c * 0.5;
    let byte = |v: f32| ((v + m) * alpha * 255.).clamp(0., 255.) as u8;
    Color32::from_rgba_premultiplied(byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), 0)
}
// Translucent fills keep overlapping petals colorful rather than clipping to white.
fn petal_color(hue: f32, saturation: f32, light: f32, alpha: f32) -> Color32 {
    let color = neon(hue, saturation, light, 1.);
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (alpha * 255.) as u8)
}
fn glow(painter: &egui::Painter, center: Pos2, radius: f32, hue: f32, opacity: f32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, neon(hue, 1., 0.6, opacity));
    for i in 0..=48 {
        mesh.colored_vertex(
            center + Vec2::angled(i as f32 / 48. * TAU) * radius,
            Color32::TRANSPARENT,
        );
        if i > 0 {
            mesh.add_triangle(0, i, i + 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    #[test]
    fn small_band_changes_move_symmetry_immediately_during_pattern_transitions() {
        let mut state = Kaleido::default();
        let steady = Metrics {
            rms: 0.08,
            bass: 0.03,
            treble: 0.03,
        };
        for frame in 0..=100 {
            state.update(steady, frame as f64 * 0.02, 1., true);
        }
        state.select_pattern(1);
        assert!(state.previous.is_some());
        let before = state.visual_folds();
        let beats = state.beats;
        for frame in 101..=105 {
            state.update(
                Metrics {
                    bass: 0.032,
                    ..steady
                },
                frame as f64 * 0.02,
                1.,
                true,
            );
        }
        assert_eq!(state.beats, beats, "sub-beat changes should be enough");
        assert!(
            (state.visual_folds() - before).abs() > 2.,
            "a small change must move multiple folds within 100ms"
        );
        assert!(state.previous.is_some());
        assert!(
            state.transition < 0.05,
            "symmetry must not wait for the style transition"
        );
        let (low, high, blend) = state.symmetry_blend();
        assert_eq!(low % 2, 0);
        assert_eq!(high % 2, 0);
        assert!(
            blend > 0. && blend < 1.,
            "render smooth blends between mirrored even fold counts"
        );
        assert_eq!(state.folds, 14);
        for frame in 106..=155 {
            state.update(Metrics::default(), frame as f64 * 0.02, 1., false);
        }
        assert!((state.visual_folds() - 14.).abs() < 0.01);
        state.auto = false;
        state.music_symmetry = 10.;
        assert_eq!(state.visual_folds(), 14.);
    }

    #[test]
    fn audio_shifts_visible_hue_even_without_color_drift_or_motion() {
        let mut bass = Kaleido {
            hue: 145.,
            motion: 0.,
            chroma: 0.,
            ..Default::default()
        };
        let mut treble = bass.clone();
        for state in [&mut bass, &mut treble] {
            state.update(Metrics::default(), 0., 1., false);
        }
        for frame in 1..=4 {
            bass.update(
                Metrics {
                    rms: 0.1,
                    bass: 0.3,
                    treble: 0.,
                },
                frame as f64 * 0.1,
                1.,
                true,
            );
            treble.update(
                Metrics {
                    rms: 0.1,
                    bass: 0.,
                    treble: 0.3,
                },
                frame as f64 * 0.1,
                1.,
                true,
            );
        }
        assert!((bass.effective_hue() - treble.effective_hue()).abs() > 60.);
        for state in [&bass, &treble] {
            assert_eq!(state.hue, 145.);
            assert_eq!(state.phase, 0.);
            assert_eq!(state.color_phase, 0.);
            assert!((state.effective_hue() - state.hue).abs() > 15.);
            if let Some(previous) = &state.previous {
                assert_eq!(previous.music_hue, state.music_hue);
            }
        }
        for pattern in 1..PATTERNS.len() {
            bass.pattern = pattern;
            treble.pattern = pattern;
            let colors = |state: &Kaleido| -> Vec<_> {
                (1..20)
                    .map(|i| {
                        state.prism_color(Vec2::new(i as f32 * 0.04, 0.17), state.music_settings())
                    })
                    .collect()
            };
            assert_ne!(
                colors(&bass),
                colors(&treble),
                "{} must visibly respond to audio hue",
                PATTERNS[pattern]
            );
        }
        for frame in 5..=44 {
            bass.update(Metrics::default(), frame as f64 * 0.1, 1., false);
        }
        assert!(
            bass.music_hue.abs() < 0.01,
            "paused hue should settle back to its base"
        );
        treble.auto = false;
        treble.update(Metrics::default(), 0.5, 1., false);
        assert_eq!(treble.effective_hue(), treble.hue);
    }

    #[test]
    fn music_choreography_moves_rendered_orientation_and_both_crossfade_layers() {
        let mut state = Kaleido::default();
        let mut travel = 0.;
        for frame in 0..1600 {
            let rotation = state.rotation;
            state.update(
                Metrics {
                    rms: 0.1,
                    bass: if frame % 25 == 0 { 0.1 } else { 0.002 },
                    treble: 0.03,
                },
                frame as f64 * 0.02,
                1.,
                true,
            );
            let delta = state.rotation - rotation;
            travel += delta.abs();
            assert!(delta.abs() < 0.15, "orientation must never jump");
            if let Some(previous) = &state.previous {
                assert!((previous.rotation - state.rotation).abs() < 0.0001);
            }
        }
        assert!(travel > TAU * 4.);
        assert!(state.clockwise);
    }

    #[test]
    fn motion_reverses_without_jumping_and_remix_preserves_direction() {
        let mut state = Kaleido {
            auto: false,
            ..Default::default()
        };
        state.update(Metrics::default(), 0., 1., false);
        state.update(Metrics::default(), 0.1, 1., false);
        let phase = state.phase;
        assert!(phase > 0.);
        state.clockwise = false;
        assert_eq!(state.phase, phase);
        state.update(Metrics::default(), 0.2, 1., false);
        assert!(state.phase.abs() < 0.00001);
        state.remix();
        assert!(!state.clockwise);
        state.update(Metrics::default(), 0.3, 1., false);
        assert!(state.phase < 0.);
        assert!(state.previous.as_ref().unwrap().phase < 0.);
        let restored = Kaleido::restore(&state.settings());
        assert!(!restored.clockwise);
    }

    #[test]
    fn led_envelopes_follow_transients_faster_than_visualizer() {
        let mut state = super::Kaleido::default();
        state.update(super::Metrics::default(), 0., 1., false);
        state.update(
            super::Metrics {
                rms: 0.1,
                bass: 0.1,
                treble: 0.,
            },
            0.02,
            1.,
            true,
        );
        assert!(state.meter_bands[0] > state.bass * 2.);
        assert_eq!(state.meter_bands[1], 0.);
        let peak = state.meter_bands[0];
        state.update(super::Metrics::default(), 0.07, 1., false);
        assert!(state.meter_bands[0] < peak * 0.5);
    }

    #[test]
    fn kaleido_responds_strongly_to_sensitive_band_envelopes() {
        let mut state = super::Kaleido {
            energy: 0.4,
            ..Default::default()
        };
        state.meter_bands = [0.2, 0.2];
        let low = state.music_settings();
        state.meter_bands = [0.8, 0.8];
        let high = state.music_settings();
        assert!(high.motion - low.motion > 0.7);
        assert_eq!(high.complexity, low.complexity);
        assert!(high.chroma - low.chroma > 1.);
    }

    #[test]
    fn led_meter_magnifies_small_band_changes() {
        let mut state = super::Kaleido::default();
        state.update(super::Metrics::default(), 0., 1., false);
        state.update(
            super::Metrics {
                rms: 0.01,
                bass: 0.009,
                treble: 0.007,
            },
            0.1,
            1.,
            true,
        );
        let before = state.meter_bands;
        state.update(
            super::Metrics {
                rms: 0.01,
                bass: 0.007,
                treble: 0.009,
            },
            0.2,
            1.,
            true,
        );
        assert!(before[0] - state.meter_bands[0] > 0.3);
        assert!(state.meter_bands[1] - before[1] > 0.3);
    }

    #[test]
    fn led_meter_preserves_band_changes_and_volume() {
        // These stronger signals used to all clip to the same full row.
        let counts =
            [0.6, 0.8, 0.95].map(|level| super::Kaleido::meter_lit_segments(level, 1., 42));
        assert!(counts[0] < counts[1] && counts[1] < counts[2]);
        assert!(counts[2] > 36, "strong peaks should reach red");
        assert!(super::Kaleido::meter_lit_segments(0.95, 1., 42) > 36);
        assert!(
            super::Kaleido::meter_lit_segments(0.3, 1., 42)
                < super::Kaleido::meter_lit_segments(0.42, 1., 42)
        );
        assert_eq!(super::Kaleido::meter_lit_segments(1., 0., 42), 0);
        assert_eq!(super::Kaleido::meter_lit_segments(0., 1., 42), 0);
        assert_eq!(super::Kaleido::meter_lit_segments(1., 0.5, 42), 21);
    }

    use super::*;
    #[test]
    fn remix_blends_from_previous_geometry_then_releases_it() {
        let mut state = Kaleido {
            auto: false,
            ..Default::default()
        };
        state.update(Metrics::default(), 0., 1., false);
        let original = state.settings();
        state.remix();
        assert_eq!(state.previous.as_ref().unwrap().settings(), original);
        assert_eq!(state.transition, 0.);
        for frame in 1..=45 {
            state.update(
                Metrics::default(),
                frame as f64 * TRANSITION_SECONDS as f64 / 90.,
                1.,
                false,
            );
        }
        assert!((state.transition - 0.5).abs() < 0.001);
        assert!(state.previous.is_some());
        assert!(state.previous.as_ref().unwrap().phase > 0.);
        for frame in 46..=100 {
            state.update(
                Metrics::default(),
                frame as f64 * TRANSITION_SECONDS as f64 / 90.,
                1.,
                false,
            );
        }
        assert_eq!(state.transition, 1.);
        assert!(state.previous.is_none());
    }

    #[test]
    fn changes_wait_for_the_current_transition_to_finish() {
        let mut state = Kaleido {
            auto: false,
            ..Default::default()
        };
        state.update(Metrics::default(), 0., 1., false);
        state.select_pattern(1);
        for frame in 1..=10 {
            state.update(Metrics::default(), frame as f64 * 0.05, 1., false);
        }
        let progress = state.transition;
        state.select_pattern(2);
        state.remix();
        assert_eq!(state.pattern, 1);
        assert_eq!(state.transition, progress);
        assert_eq!(state.previous.as_ref().unwrap().pattern, 0);
        for frame in 11..=72 {
            state.update(Metrics::default(), frame as f64 * 0.05, 1., false);
        }
        assert_eq!(state.pattern, 2);
        assert_eq!(state.previous.as_ref().unwrap().pattern, 1);
        assert!(state.transition < 0.1);
    }

    #[test]
    fn wedge_clipping_preserves_mirror_boundaries() {
        let clipped = clip_wedge(
            vec![Vec2::new(1., -2.), Vec2::new(3., 0.), Vec2::new(1., 2.)],
            0.2,
        );
        assert!(clipped.len() >= 3);
        assert!(
            clipped
                .iter()
                .all(|v| v.y.abs() <= v.x * 0.2_f32.tan() + 0.00001)
        );
    }
    #[test]
    fn restores_controls_without_restoring_audio_envelopes() {
        let settings = Settings {
            auto: false,
            folds: 24,
            motion: 1.2,
            clockwise: false,
            complexity: 0.4,
            chroma: 1.8,
            palette: 3,
            pattern: 7,
            hue: 145.,
            seed: 2.,
            milkdrop: false,
            milkdrop_preset: 0,
            ..Settings::default()
        };
        let serialized = serde_json::to_string(&settings).unwrap();
        let saved: Settings = serde_json::from_str(&serialized).unwrap();
        let state = Kaleido::restore(&saved);
        assert_eq!(state.settings(), settings);
        assert_eq!(state.energy, 0.);
        assert_eq!(state.beats, 0);
        assert_eq!(state.phase, 0.);
        let invalid = Settings {
            folds: u32::MAX,
            palette: usize::MAX,
            motion: f32::NAN,
            pattern: usize::MAX,
            hue: f32::NAN,
            ..settings
        };
        let restored = Kaleido::restore(&invalid);
        assert_eq!(restored.folds, 28);
        assert_eq!(restored.palette, 4);
        assert_eq!(restored.pattern, PATTERNS.len() - 1);
        assert_eq!(restored.hue, 0.);
        assert!(restored.motion.is_finite());
    }

    #[test]
    fn legacy_settings_default_to_original_pattern_and_hue() {
        let settings: Settings = serde_json::from_str(r#"{"palette":2,"folds":12}"#).unwrap();
        assert_eq!(settings.pattern, 0);
        assert_eq!(settings.hue, 0.);
        assert!(settings.milkdrop_favorites.is_empty());
        assert!(!settings.milkdrop_favorites_only);
        assert_eq!(
            Kaleido::restore(&settings).settings(),
            Settings { milkdrop_seconds: Some(28), ..settings }
        );
        let disabled: Settings = serde_json::from_str(r#"{"milkdrop_auto":false}"#).unwrap();
        assert_eq!(Kaleido::restore(&disabled).milkdrop_seconds, 0);
    }

    #[test]
    fn milkdrop_advance_skips_disabled_presets_in_both_orders() {
        let mut state = Kaleido {
            projectm_presets: vec!["First".into(), "Black".into(), "Last".into()],
            milkdrop_disabled: vec!["Black".into()],
            ..Default::default()
        };
        for random in [false, true] {
            state.milkdrop_random = random;
            state.advance_milkdrop_preset();
            assert_eq!(state.milkdrop_preset, 2);
            state.advance_milkdrop_preset();
            assert_eq!(state.milkdrop_preset, 0);
        }
        state.milkdrop_disabled.extend(["First".into(), "Last".into()]);
        state.advance_milkdrop_preset();
        assert!(!state.milkdrop_preset_available(state.milkdrop_preset));
    }

    #[test]
    fn milkdrop_manual_navigation_wraps_and_skips_disabled_presets() {
        let mut state = Kaleido {
            projectm_presets: vec!["First".into(), "Disabled".into(), "Middle".into(), "Last".into()],
            milkdrop_disabled: vec!["Disabled".into()],
            milkdrop_random: true,
            ..Default::default()
        };
        assert_eq!(state.adjacent_milkdrop_preset(true), Some(3));
        assert_eq!(state.adjacent_milkdrop_preset(false), Some(2));
        state.set_milkdrop_preset(3);
        assert_eq!(state.adjacent_milkdrop_preset(true), Some(2));
        assert_eq!(state.adjacent_milkdrop_preset(false), Some(0));
        state.milkdrop_disabled.extend(["First".into(), "Middle".into()]);
        assert_eq!(state.adjacent_milkdrop_preset(true), None);
        assert_eq!(state.adjacent_milkdrop_preset(false), None);
    }

    #[test]
    fn milkdrop_favorites_only_filters_navigation_and_auto_advance() {
        let mut state = Kaleido {
            projectm_presets: vec!["First".into(), "Other".into(), "Disabled".into(), "Last".into()],
            milkdrop_favorites: vec!["First".into(), "Disabled".into(), "Last".into()],
            milkdrop_disabled: vec!["Disabled".into()],
            milkdrop_favorites_only: true,
            ..Default::default()
        };
        assert!(!state.milkdrop_preset_available(1));
        assert!(!state.milkdrop_preset_available(2));
        assert_eq!(state.adjacent_milkdrop_preset(true), Some(3));
        assert_eq!(state.adjacent_milkdrop_preset(false), Some(3));
        for random in [false, true] {
            state.milkdrop_random = random;
            state.advance_milkdrop_preset();
            assert_eq!(state.milkdrop_preset, 3);
            state.advance_milkdrop_preset();
            assert_eq!(state.milkdrop_preset, 0);
        }
        state.toggle_milkdrop_favorite();
        assert_eq!(state.milkdrop_preset, 3);
        state.toggle_milkdrop_favorite();
        assert!(!state.milkdrop_preset_available(state.milkdrop_preset));
        state.advance_milkdrop_preset();
        assert!(!state.milkdrop_preset_available(state.milkdrop_preset));
        assert_eq!(state.adjacent_milkdrop_preset(false), None);

        let restored = Kaleido::restore(&serde_json::from_str(
            &serde_json::to_string(&state.settings()).unwrap()
        ).unwrap());
        assert!(restored.milkdrop_favorites_only);
        assert_eq!(restored.milkdrop_favorites, state.milkdrop_favorites);
    }

    #[test]
    fn milkdrop_favorites_survive_reload_by_name() {
        let mut state = Kaleido {
            projectm_presets: vec!["First".into(), "Favorite".into()],
            milkdrop_preset: 1,
            ..Default::default()
        };
        state.toggle_milkdrop_favorite();
        let saved = serde_json::to_string(&state.settings()).unwrap();
        let mut restored = Kaleido::restore(&serde_json::from_str(&saved).unwrap());
        assert_eq!(restored.milkdrop_favorites, ["Favorite"]);
        restored.projectm_presets = vec!["Favorite".into(), "First".into()];
        restored.set_milkdrop_preset(0);
        restored.toggle_milkdrop_favorite();
        assert!(restored.milkdrop_favorites.is_empty());
    }

    #[test]
    fn procedural_motion_rotates_without_changing_radial_geometry() {
        for pattern in 1..PATTERNS.len() {
            let mut state = Kaleido {
                pattern,
                auto: false,
                ..Default::default()
            };
            let samples: Vec<_> = (1..40)
                .map(|i| {
                    let point = Vec2::new(i as f32 * 0.027, 0.19);
                    (point, state.prism_color(point, state.music_settings()))
                })
                .collect();
            state.rotation = 0.24;
            for (point, expected) in samples {
                // Undo rotation: unchanged colors imply unchanged ring positions,
                // cell sizes, and density throughout the motion cycle.
                let actual =
                    state.prism_color(rotate(point, state.rotation), state.music_settings());
                for (a, b) in actual.to_array().into_iter().zip(expected.to_array()) {
                    assert!(
                        a.abs_diff(b) <= 2,
                        "{} changed radial geometry",
                        PATTERNS[pattern]
                    );
                }
            }
        }
    }

    #[test]
    fn prism_patterns_are_distinct_and_hue_rotates_each_pattern() {
        let mut signatures = std::collections::HashSet::new();
        for pattern in 1..PATTERNS.len() {
            let mut state = Kaleido {
                pattern,
                auto: false,
                ..Default::default()
            };
            let sample = |state: &Kaleido| -> Vec<_> {
                (0..32)
                    .map(|i| {
                        state.prism_color(Vec2::new(i as f32 * 0.037, 0.17), state.music_settings())
                    })
                    .collect()
            };
            let original = sample(&state);
            state.hue = 120.;
            assert_ne!(
                original,
                sample(&state),
                "{} must respond to hue",
                PATTERNS[pattern]
            );
            assert!(
                signatures.insert(original),
                "patterns must have distinct geometry/colors"
            );
            state.remix();
            assert_ne!(state.pattern, pattern);
            let hue_change = (state.hue - 120.).rem_euclid(360.);
            assert!((45.0..315.0).contains(&hue_change));
            assert_eq!(state.previous.as_ref().unwrap().hue, 120.);
        }
    }

    #[test]
    fn reaction_disabled_uses_base_settings_and_keeps_animating() {
        let mut state = Kaleido {
            auto: false,
            ..Default::default()
        };
        let original = state.settings();
        for frame in 0..1200 {
            state.update(
                Metrics {
                    rms: 0.4,
                    bass: if frame % 30 == 0 { 0.5 } else { 0.002 },
                    treble: 0.3,
                },
                frame as f64 / 60.,
                1.,
                true,
            );
            let live = state.music_settings();
            assert_eq!(live.folds, original.folds);
            assert_eq!(live.motion, original.motion);
            assert_eq!(live.complexity, original.complexity);
            assert_eq!(live.chroma, original.chroma);
        }
        assert_eq!(state.settings(), original);
        assert_eq!(state.beats, 0);
        assert!(state.phase > 0. && state.color_phase > 0.);
        state.remix();
        assert_ne!(state.palette, original.palette);
    }

    #[test]
    fn silence_and_pause_do_not_remix() {
        let mut state = Kaleido::default();
        for i in 0..1000 {
            state.update(Metrics::default(), i as f64 * 0.05, 1., true);
        }
        assert_eq!(state.palette, 0);
        for i in 1000..2000 {
            state.update(
                Metrics {
                    rms: 1.,
                    bass: 1.,
                    treble: 1.,
                },
                i as f64 * 0.05,
                1.,
                false,
            );
        }
        assert_eq!(state.palette, 0);
    }
    #[test]
    fn bass_accents_remix_with_cooldown() {
        let mut state = Kaleido::default();
        let mut changes = Vec::new();
        let mut symmetries = std::collections::HashSet::new();
        for i in 0..300 {
            let previous = state.palette;
            let metrics = Metrics {
                rms: 0.1,
                bass: if i % 10 == 0 { 0.08 } else { 0.002 },
                treble: 0.03,
            };
            state.update(metrics, i as f64 * 0.05, 1., true);
            symmetries.insert(state.music_settings().folds);
            if state.palette != previous {
                changes.push(i as f32 * 0.05);
            }
        }
        assert!(!changes.is_empty());
        assert!(symmetries.len() > 1);
        assert!(
            symmetries
                .iter()
                .all(|folds| (4..=28).contains(folds) && folds % 2 == 0)
        );
        assert!(changes[0] >= 4. && changes[0] < 10.);
        assert!(changes.windows(2).all(|times| times[1] - times[0] >= 4.));
    }

    #[test]
    fn loud_music_keeps_dynamic_range_and_beats_change_geometry() {
        let mut state = Kaleido::default();
        let quiet = Metrics {
            rms: 0.3,
            bass: 0.15,
            treble: 0.25,
        };
        for i in 0..200 {
            state.update(quiet, i as f64 * 0.02, 1., true);
        }
        let before = (
            state.energy,
            state.bass,
            state.treble,
            state.music_settings(),
        );
        let radius = state.radius(Vec2::new(900., 700.));
        let loud = Metrics {
            rms: 0.6,
            bass: 0.3,
            treble: 0.5,
        };
        for i in 200..205 {
            state.update(loud, i as f64 * 0.02, 1., true);
        }
        assert!(state.energy > before.0 + 0.08);
        assert!(state.bass > before.1 + 0.08);
        assert!(state.treble > before.2 + 0.08);
        let live = state.music_settings();
        assert!(live.motion > before.3.motion + 0.1);
        assert_eq!(live.complexity, before.3.complexity);
        assert!(live.chroma > before.3.chroma + 0.1);
        assert_eq!(state.radius(Vec2::new(900., 700.)), radius);
        assert!(state.beat_pulse > 0.4);
        for i in 205..300 {
            state.update(Metrics::default(), i as f64 * 0.02, 1., true);
        }
        assert!(state.energy < 0.001 && state.bass < 0.001 && state.beat_pulse < 0.001);
    }

    #[test]
    fn canvas_emits_valid_meshes_at_extreme_settings() {
        let ctx = egui::Context::default();
        let mut state = Kaleido {
            folds: 28,
            complexity: 1.,
            ..Default::default()
        };
        for pattern in 0..PATTERNS.len() {
            state.pattern = pattern;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                state.draw(ui);
            });
            output.textures_delta.clear();
            let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
            assert!(!primitives.is_empty());
            for primitive in primitives {
                if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                    assert!(mesh.is_valid());
                    assert!(
                        mesh.vertices
                            .iter()
                            .all(|v| v.pos.x.is_finite() && v.pos.y.is_finite())
                    );
                }
            }
        }
    }

    #[test]
    fn sustained_music_remixes_and_manual_mode_stays_fixed() {
        let mut state = Kaleido::default();
        let metrics = Metrics {
            rms: 0.1,
            bass: 0.05,
            treble: 0.1,
        };
        for i in 0..340 {
            state.update(metrics, i as f64 * 0.05, 1., true);
        }
        assert_ne!(state.palette, 0);
        state.auto = false;
        let palette = state.palette;
        for i in 340..1000 {
            state.update(metrics, i as f64 * 0.05, 1., true);
        }
        assert_eq!(state.palette, palette);
    }
}
