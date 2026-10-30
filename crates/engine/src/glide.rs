//! Knob moves, glided. Settings reach the engine at most once a UI frame (~16 ms apart:
//! the MK3's knobs are read then, and each change is an edit), so an effect or synth that
//! took each at once would step: a filter sweep zippers instead of moving as Maschine's
//! do. A [`Glide`] takes the continuous settings to their new values over [`GLIDE_MS`],
//! a little longer than a frame, in steps of [`STEP`] frames: one movement however the
//! edits are spaced. Choices and switches (whole numbers before and after) change at once.

/// How long a setting takes to reach a new value.
pub const GLIDE_MS: f64 = 30.0;
/// Frames between steps of a glide.
pub const STEP: usize = 64;

pub struct Glide {
    /// The values the effect or synth has now (its last `set`).
    now: Vec<(String, f64)>,
    target: Vec<f64>,
    step: Vec<f64>,
    /// Steps still to go, and how many a glide takes.
    left: u32,
    steps: u32,
}

impl Glide {
    /// Starting at `params` (allocates: made with the effect, off the audio thread).
    pub fn new(params: &[(String, f64)], sr: f64) -> Glide {
        Glide {
            now: params.to_vec(),
            target: params.iter().map(|(_, v)| *v).collect(),
            step: vec![0.0; params.len()],
            left: 0,
            steps: ((sr * GLIDE_MS / 1000.0) / STEP as f64).round().max(1.0) as u32,
        }
    }

    /// The values to `set` now.
    pub fn now(&self) -> &[(String, f64)] {
        &self.now
    }

    pub fn gliding(&self) -> bool {
        self.left > 0
    }

    /// Head for `params`: choices and switches take theirs at once, the rest glide from
    /// where they are. Then `set` [`Glide::now`]. False (nothing changed here) when
    /// `params` aren't the same settings, which are then taken at once. No allocating.
    pub fn retarget(&mut self, params: &[(String, f64)]) -> bool {
        if params.len() != self.now.len() || params.iter().zip(&self.now).any(|((a, _), (b, _))| a != b) {
            return false;
        }
        let steps = f64::from(self.steps);
        for (i, (_, v)) in params.iter().enumerate() {
            let now = self.now[i].1;
            self.target[i] = *v;
            if now.fract() == 0.0 && v.fract() == 0.0 {
                self.now[i].1 = *v;
                self.step[i] = 0.0;
            } else {
                self.step[i] = (*v - now) / steps;
            }
        }
        self.left = self.steps;
        true
    }

    /// Whether `params` are the values it is at or heading for.
    pub fn heading_for(&self, params: &[(String, f64)]) -> bool {
        params.len() == self.target.len() && params.iter().zip(&self.target).all(|((_, a), b)| a == b)
    }

    /// One step: true when the values moved (`set` [`Glide::now`] again).
    pub fn tick(&mut self) -> bool {
        if self.left == 0 {
            return false;
        }
        self.left -= 1;
        for (i, (_, v)) in self.now.iter_mut().enumerate() {
            *v = if self.left == 0 { self.target[i] } else { *v + self.step[i] };
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(cutoff: f64, mode: f64) -> Vec<(String, f64)> {
        vec![("cutoff".into(), cutoff), ("mode".into(), mode)]
    }

    /// A new cutoff is reached evenly over the glide; a new mode at once.
    #[test]
    fn continuous_settings_glide_and_choices_jump() {
        let sr = 48_000.0;
        let mut g = Glide::new(&params(0.25, 0.0), sr);
        assert!(g.retarget(&params(0.75, 2.0)));
        assert_eq!(g.now()[1].1, 2.0, "mode at once");
        assert_eq!(g.now()[0].1, 0.25, "cutoff from where it was");
        let steps = (sr * GLIDE_MS / 1000.0 / STEP as f64).round() as usize;
        let mut last = 0.25;
        for i in 0..steps {
            assert!(g.tick());
            let c = g.now()[0].1;
            assert!(c > last && c <= 0.75 + 1e-12, "step {i}: {c}");
            last = c;
        }
        assert_eq!(g.now()[0].1, 0.75);
        assert!(!g.tick() && !g.gliding());
    }

    /// A new value mid-glide carries on from where it got to (no jump back).
    #[test]
    fn a_new_value_mid_glide_starts_from_where_it_is() {
        let mut g = Glide::new(&params(0.0, 1.0), 48_000.0);
        g.retarget(&params(0.5, 1.0));
        for _ in 0..5 {
            g.tick();
        }
        let mid = g.now()[0].1;
        g.retarget(&params(0.6, 1.0));
        assert_eq!(g.now()[0].1, mid);
        g.tick();
        assert!(g.now()[0].1 > mid);
    }

    /// Other settings (another module's) are taken at once.
    #[test]
    fn other_settings_dont_glide() {
        let mut g = Glide::new(&params(0.1, 0.0), 48_000.0);
        assert!(!g.retarget(&[("drive".into(), 0.3)]));
    }
}
