//! Music-driven angular choreography. Only orientation changes, never scale.
use std::f32::consts::TAU;

#[derive(Clone)]
pub(super) struct Motion {
    pub velocity: f32,
    phrase: u32,
    progress: f32,
    beat_age: f32,
    beat_period: f32,
    duration: f32,
}

impl Default for Motion {
    fn default() -> Self {
        Self {
            velocity: 0.,
            phrase: 0,
            progress: 0.,
            beat_age: 1.,
            beat_period: 0.5,
            duration: 8.,
        }
    }
}

impl Motion {
    pub fn name(&self) -> &'static str {
        match self.phrase % 3 {
            0 => "Spin",
            1 => "Coast",
            _ => "Swing",
        }
    }

    /// Return an angular displacement in radians. Phrase progress follows audio
    /// energy and tempo; acceleration is bounded so accents never jump orientation.
    pub fn update(
        &mut self,
        dt: f32,
        active: bool,
        reacting: bool,
        speed: f32,
        direction: f32,
        energy: f32,
        bands: [f32; 2],
        beat: bool,
    ) -> f32 {
        self.beat_age += dt;
        if beat {
            if (0.28..=1.5).contains(&self.beat_age) {
                self.beat_period += (self.beat_age - self.beat_period) * 0.25;
            }
            self.beat_age = 0.;
        }
        let [bass, treble] = bands;
        let previous_velocity = self.velocity;
        let target = if !reacting {
            direction * speed * 0.35
        } else if !active || speed == 0. {
            0.
        } else {
            let pace = (speed / 0.6) * (0.7 + energy * 0.45 + bass * 0.25);
            self.progress = (self.progress + dt * pace / self.duration).min(1.);
            let u = self.progress;
            // Alternate long spins with rests and shorter oscillating gestures.
            // A spin's sin² envelope integrates to two or three full turns.
            let sign = direction * if (self.phrase / 3) % 2 == 0 { 1. } else { -1. };
            let target = match self.phrase % 3 {
                0 => {
                    let turns = 2. + ((self.phrase / 3) % 2) as f32;
                    sign * TAU * turns / self.duration * pace * (1. - (TAU * u).cos())
                }
                1 => sign * 0.16 * (1. - u) * (0.4 + energy),
                _ => {
                    let envelope = (std::f32::consts::PI * u).sin().powi(2);
                    sign * (TAU * u * 2.).sin()
                        * envelope
                        * (0.8 + bass * 0.8 + treble * 0.4)
                        * pace
                }
            };
            if self.progress >= 1. {
                self.phrase = self.phrase.wrapping_add(1);
                self.progress = 0.;
                self.duration = match self.phrase % 3 {
                    0 => (self.beat_period * 16.).clamp(6., 12.),
                    1 => (self.beat_period * 4.).clamp(1.5, 3.),
                    _ => (self.beat_period * 12.).clamp(4., 8.),
                };
            }
            target
        };
        // Smooth velocity and limit acceleration in radians/s². Integrating the
        // average velocity keeps position continuous through stops and reversals.
        let target = target.clamp(-5., 5.);
        let change = (target - self.velocity) * (1. - (-dt * 5.).exp());
        self.velocity += change.clamp(-dt * 4., dt * 4.);
        (previous_velocity + self.velocity) * 0.5 * dt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phrases_include_full_turns_coasting_and_bidirectional_swings() {
        let mut motion = Motion::default();
        let mut angle = 0.;
        let mut spin_angle = 0.;
        let mut swing_signs = std::collections::HashSet::new();
        let mut peak: f32 = 0.;
        let mut coast_min: f32 = 10.;
        for frame in 0..3000 {
            let name = motion.name();
            let last_velocity = motion.velocity;
            let delta = motion.update(0.02, true, true, 0.6, 1., 0.5, [0.5, 0.4], frame % 25 == 0);
            angle += delta;
            assert!((motion.velocity - last_velocity).abs() <= 0.081);
            match name {
                "Spin" => {
                    spin_angle += delta.abs();
                    peak = peak.max(motion.velocity.abs());
                }
                "Coast" => coast_min = coast_min.min(motion.velocity.abs()),
                _ => {
                    if motion.velocity.abs() > 0.1 {
                        swing_signs.insert(motion.velocity > 0.);
                    }
                }
            }
        }
        assert!(angle.is_finite());
        assert!(
            spin_angle > TAU * 8.,
            "spins must complete multiple full rotations"
        );
        assert!(peak > 2.);
        assert!(coast_min < 0.2);
        assert_eq!(swing_signs.len(), 2);
    }

    #[test]
    fn audio_changes_pace_and_silence_coasts_to_a_stop() {
        let mut quiet = Motion::default();
        let mut loud = quiet.clone();
        let mut quiet_angle = 0.;
        let mut loud_angle = 0.;
        for frame in 0..100 {
            quiet_angle +=
                quiet.update(0.02, true, true, 0.6, 1., 0.15, [0.1, 0.1], frame % 25 == 0);
            loud_angle += loud.update(0.02, true, true, 0.6, 1., 0.8, [0.9, 0.7], frame % 25 == 0);
        }
        assert!(loud_angle > quiet_angle * 1.5);
        for _ in 0..200 {
            loud.update(0.02, false, true, 0.6, 1., 0., [0., 0.], false);
        }
        assert!(loud.velocity.abs() < 0.001);
    }

    #[test]
    fn base_direction_mirrors_choreography_and_zero_motion_stays_still() {
        let mut cw = Motion::default();
        let mut ccw = cw.clone();
        let mut stopped = cw.clone();
        for frame in 0..1000 {
            let a = cw.update(0.02, true, true, 0.6, 1., 0.5, [0.5, 0.4], frame % 25 == 0);
            let b = ccw.update(0.02, true, true, 0.6, -1., 0.5, [0.5, 0.4], frame % 25 == 0);
            assert!((a + b).abs() < 0.0001);
            assert_eq!(
                stopped.update(0.02, true, true, 0., 1., 0.5, [0.5, 0.4], true),
                0.
            );
        }
    }
}
