use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

pub type Controls = Arc<AtomicBool>;

// Stereo-linked adaptive leveling followed by a fast peak limiter. Measurements
// ignore silence; gain changes are smoothed rather than changing the user's volume.
pub struct Processor {
    controls: Controls,
    frames: usize,
    power: f64,
    fast_power: f64,
    slow_power: f64,
    measured: bool,
    target: f64,
    gain: f64,
    wet: f64,
    limiter: f64,
}
impl Processor {
    pub fn new(controls: Controls) -> Self {
        Self { controls, frames: 0, power: 0., fast_power: 0., slow_power: 0.,
            measured: false, target: 1., gain: 1., wet: 0., limiter: 1. }
    }
    pub fn new_track(&mut self) {
        self.frames = 0;
        self.power = 0.;
        self.measured = false;
    }
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let enabled = self.controls.load(Ordering::Relaxed);
        let left = if left.is_finite() { left as f64 } else { 0. };
        let right = if right.is_finite() { right as f64 } else { 0. };
        self.power += (left * left + right * right) * 0.5;
        self.frames += 1;
        if self.frames == 441 {
            let power = self.power / 441.;
            self.frames = 0;
            self.power = 0.;
            // Do not raise background noise or chase silent sections.
            if power > 0.003f64.powi(2) {
                if !self.measured {
                    self.fast_power = power;
                    self.slow_power = power;
                    self.measured = true;
                } else {
                    self.fast_power += 0.0952 * (power - self.fast_power);
                    self.slow_power += 0.00664 * (power - self.slow_power);
                }
                let rms = self.fast_power.max(self.slow_power).sqrt();
                self.target = (0.12 / rms).clamp(0.126, 4.);
            }
        }
        let speed = if self.target < self.gain { 1. / (44100. * 0.08) } else { 1. / (44100. * 1.5) };
        self.gain += speed * (self.target - self.gain);
        self.wet += (if enabled { 1. } else { 0. } - self.wet) / (44100. * 0.02);
        if !enabled && self.wet < 0.000001 {
            self.wet = 0.;
            self.limiter = 1.;
            return (left as f32, right as f32);
        }
        let level = 1. + self.wet * (self.gain - 1.);
        let peak = left.abs().max(right.abs()) * level;
        let limit = if peak > 0.95 { 0.95 / peak } else { 1. };
        if limit < self.limiter { self.limiter = limit; }
        else { self.limiter += (limit - self.limiter) / (44100. * 0.05); }
        let level = level * self.limiter;
        ((left * level) as f32, (right * level) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone_rms(amplitude: f32) -> f64 {
        let mut processor = Processor::new(Arc::new(AtomicBool::new(true)));
        let mut power = 0.;
        for frame in 0..44100 * 8 {
            let input = (std::f32::consts::TAU * 440. * frame as f32 / 44100.).sin() * amplitude;
            let (left, right) = processor.process(input, input);
            assert_eq!(left, right);
            if frame >= 44100 * 7 { power += (left as f64).powi(2); }
        }
        (power / 44100.).sqrt()
    }
    #[test]
    fn quiet_and_loud_tracks_converge_to_similar_levels() {
        let quiet = tone_rms(0.06);
        let loud = tone_rms(0.6);
        assert!((quiet / loud - 1.).abs() < 0.03, "{quiet} vs {loud}");
        assert!((loud - 0.12).abs() < 0.005);
    }
    #[test]
    fn switching_tracks_relearns_level_without_resetting_gain_abruptly() {
        let mut processor = Processor::new(Arc::new(AtomicBool::new(true)));
        let mut levels = Vec::new();
        for amplitude in [0.6, 0.06, 0.6] {
            let gain = processor.gain;
            processor.new_track();
            assert_eq!(processor.gain, gain);
            let mut power = 0.;
            for frame in 0..44100 * 8 {
                let input = (std::f32::consts::TAU * 440. * frame as f32 / 44100.).sin() * amplitude;
                let (left, _) = processor.process(input, input);
                if frame >= 44100 * 7 { power += (left as f64).powi(2); }
            }
            levels.push((power / 44100.).sqrt());
        }
        for level in levels { assert!((level - 0.12).abs() < 0.005); }
    }
    #[test]
    fn disabled_is_exact_bypass() {
        let mut processor = Processor::new(Arc::new(AtomicBool::new(false)));
        for sample in [0., -0.7, 0.2, 1.2].into_iter().cycle().take(10000) {
            assert_eq!(processor.process(sample, -sample), (sample, -sample));
        }
    }
    #[test]
    fn silence_stays_silent_and_limiter_preserves_stereo_balance() {
        let mut processor = Processor::new(Arc::new(AtomicBool::new(true)));
        for _ in 0..44100 { assert_eq!(processor.process(0., 0.), (0., 0.)); }
        for _ in 0..44100 {
            let (left, right) = processor.process(2., 1.);
            assert!(left.is_finite() && left <= 0.950001);
            assert!((left - 2. * right).abs() < 0.000001);
        }
    }
    #[test]
    fn live_disable_returns_to_original_volume() {
        let controls = Arc::new(AtomicBool::new(true));
        let mut processor = Processor::new(controls.clone());
        for _ in 0..44100 { processor.process(0.5, 0.5); }
        controls.store(false, Ordering::Relaxed);
        let mut output = (0., 0.);
        for _ in 0..44100 { output = processor.process(0.5, 0.5); }
        assert_eq!(output, (0.5, 0.5));
    }
}
