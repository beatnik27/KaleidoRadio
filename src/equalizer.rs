use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub const FREQUENCIES: [f64; 10] = [31.25, 62.5, 125., 250., 500., 1000., 2000., 4000., 8000., 16000.];
pub const LABELS: [&str; 10] = ["31", "63", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];

// Gains in dB, ordered from the lowest to the highest band.
pub const PRESETS: [(&str, [f32; 10]); 10] = [
    ("Flat", [0.; 10]),
    ("Bass Boost", [5., 5., 4., 2., 0., 0., 0., 0., 0., 0.]),
    ("Bass Cut", [-6., -5., -4., -2., 0., 0., 0., 0., 0., 0.]),
    ("Treble Boost", [0., 0., 0., 0., 0., 0., 1., 2., 4., 5.]),
    ("Vocal Boost", [-3., -3., -2., -1., 1., 3., 4., 3., 1., 0.]),
    ("Rock", [4., 3., 2., 0., -2., -1., 1., 3., 4., 3.]),
    ("Pop", [-1., 1., 2., 3., 3., 2., 1., 0., -1., -2.]),
    ("Electronic", [5., 4., 2., 0., -2., 0., 1., 2., 4., 4.]),
    ("Jazz", [3., 2., 1., 1., -1., 0., 1., 2., 3., 2.]),
    ("Classical", [2., 2., 1., 0., 0., 0., 0., 1., 2., 3.]),
];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub bands: [f32; 10],
    pub preamp: f32,
}
impl Default for Settings {
    fn default() -> Self { Self { enabled: false, bands: [0.; 10], preamp: 0. } }
}
impl Settings {
    fn normalized(mut self) -> Self {
        for gain in &mut self.bands {
            *gain = if gain.is_finite() { gain.clamp(-12., 12.) } else { 0. };
        }
        self.preamp = if self.preamp.is_finite() { self.preamp.clamp(-12., 12.) } else { 0. };
        self
    }
}
pub type Controls = Arc<Mutex<Settings>>;

#[derive(Clone, Copy)]
struct Band {
    b: [f64; 3],
    a: [f64; 2],
    state: [[f64; 2]; 2],
}
impl Default for Band {
    fn default() -> Self { Self { b: [1., 0., 0.], a: [0., 0.], state: [[0.; 2]; 2] } }
}
impl Band {
    fn configure(&mut self, frequency: f64, gain: f64) {
        // Peaking biquad, with independent delay state for left and right.
        let a = 10f64.powf(gain / 40.);
        let omega = std::f64::consts::TAU * frequency / 44100.;
        let alpha = omega.sin() / (2. * std::f64::consts::SQRT_2);
        let a0 = 1. + alpha / a;
        self.b = [(1. + alpha * a) / a0, -2. * omega.cos() / a0, (1. - alpha * a) / a0];
        self.a = [-2. * omega.cos() / a0, (1. - alpha / a) / a0];
    }
    fn process(&mut self, input: f64, channel: usize) -> f64 {
        let state = &mut self.state[channel];
        let output = self.b[0] * input + state[0];
        state[0] = self.b[1] * input - self.a[0] * output + state[1];
        state[1] = self.b[2] * input - self.a[1] * output;
        output
    }
}

// Runs after track mixing, so changes apply live to radio, files, and crossfades.
pub struct Processor {
    controls: Controls,
    settings: Settings,
    bands: [Band; 10],
    gains: [f64; 10],
    samples: u64,
    wet: f64,
    level: f64,
    target_level: f64,
}
impl Processor {
    pub fn new(controls: Controls) -> Self {
        Self { controls, settings: Settings::default(), bands: [Band::default(); 10],
            gains: [0.; 10], samples: 0, wet: 0., level: 1., target_level: 1. }
    }
    pub fn process(&mut self, input: f32) -> f32 {
        if self.samples % 256 == 0 {
            if let Ok(settings) = self.controls.try_lock() { self.settings = settings.normalized(); }
            for (index, band) in self.bands.iter_mut().enumerate() {
                self.gains[index] += 0.135 * (self.settings.bands[index] as f64 - self.gains[index]);
                band.configure(FREQUENCIES[index], self.gains[index]);
            }
            // Reserve headroom for boosts, independently of the volume control.
            let boost = self.gains.iter().copied().fold(0f64, f64::max);
            self.target_level = 10f64.powf((self.settings.preamp as f64 - boost) / 20.);
        }
        let channel = self.samples as usize % 2;
        self.samples += 1;
        let input = if input.is_finite() { input as f64 } else { 0. };
        // Smooth bypass, gain, and coefficient changes without restarting playback.
        self.wet += 0.001 * (if self.settings.enabled { 1. } else { 0. } - self.wet);
        self.level += 0.001 * (self.target_level - self.level);
        let mut filtered = input;
        for band in &mut self.bands { filtered = band.process(filtered, channel); }
        (input * (1. - self.wet) + filtered * self.level * self.wet) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn amplitude(settings: Settings, frequency: f64, channel: usize) -> f64 {
        let mut processor = Processor::new(Arc::new(Mutex::new(settings)));
        let mut power = 0.;
        for frame in 0..44100 {
            let input = (std::f64::consts::TAU * frequency * frame as f64 / 44100.).sin() as f32 * 0.1;
            for side in 0..2 {
                let sample = processor.process(if side == channel { input } else { 0. });
                if frame >= 22050 && side == channel { power += (sample as f64).powi(2); }
                if side != channel { assert_eq!(sample, 0.); }
            }
        }
        (power / 22050.).sqrt()
    }
    #[test]
    fn positive_preamp_boosts_audio_and_defaults_to_zero() {
        assert_eq!(Settings::default().preamp, 0.);
        let reference = amplitude(Settings::default(), 1000., 0);
        let boosted = amplitude(Settings { enabled: true, preamp: 12., ..Settings::default() }, 1000., 0);
        assert!((boosted / reference - 10f64.powf(12. / 20.)).abs() < 0.01);
    }
    #[test]
    fn disabled_equalizer_is_exact_bypass() {
        let mut processor = Processor::new(Arc::new(Mutex::new(Settings { bands: [12.; 10], ..Settings::default() })));
        for sample in [0.1, -0.2, 0., 0.75].into_iter().cycle().take(10000) {
            assert_eq!(processor.process(sample), sample);
        }
    }
    #[test]
    fn flat_equalizer_preserves_audio_and_channels() {
        let reference = amplitude(Settings::default(), 1000., 0);
        let actual = amplitude(Settings { enabled: true, ..Settings::default() }, 1000., 1);
        assert!((actual / reference - 1.).abs() < 0.001);
    }
    #[test]
    fn band_cut_attenuates_its_center_frequency() {
        let mut settings = Settings { enabled: true, ..Settings::default() };
        settings.bands[5] = -12.;
        let cut = amplitude(settings, 1000., 0);
        let reference = amplitude(Settings::default(), 1000., 0);
        assert!((cut / reference - 10f64.powf(-12. / 20.)).abs() < 0.01);
        assert!(amplitude(settings, 100., 0) / amplitude(Settings::default(), 100., 0) > 0.98);
    }
    #[test]
    fn controls_update_during_playback_and_extreme_gains_stay_finite() {
        let controls = Arc::new(Mutex::new(Settings::default()));
        let mut processor = Processor::new(controls.clone());
        for _ in 0..10000 { assert_eq!(processor.process(0.1), 0.1); }
        *controls.lock().unwrap() = Settings { enabled: true, bands: [12.; 10], preamp: -12. };
        let mut changed = false;
        for i in 0..88200 {
            let input = (i as f32 * 0.1).sin() * 0.1;
            let output = processor.process(input);
            assert!(output.is_finite() && output.abs() < 1.);
            changed |= (input - output).abs() > 0.01;
        }
        assert!(changed);
    }
}
