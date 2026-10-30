//! Prehear: the browser's preview of a sample, played once through the master output
//! (beside the metronome, not tempo-synced), resampled to the output's rate.

use crate::sample::Sample;
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct Prehear {
    sample: Option<Arc<Sample>>,
    /// Where it is, in the sample's frames; `None` once it has played.
    at: Option<f64>,
    gain: f32,
}

impl Prehear {
    /// Play `sample` from its start at `gain` (`None` stops). Returns the one it
    /// replaces, to be freed off the audio thread.
    pub(crate) fn set(&mut self, sample: Option<Arc<Sample>>, gain: f32) -> Option<Arc<Sample>> {
        self.at = sample.as_ref().map(|_| 0.0);
        self.gain = gain;
        std::mem::replace(&mut self.sample, sample)
    }

    /// Add it to `out` (interleaved stereo) at output rate `sr`.
    pub(crate) fn mix(&mut self, out: &mut [f32], sr: f64) {
        let (Some(s), Some(mut at)) = (&self.sample, self.at) else { return };
        let step = f64::from(s.rate) / sr;
        let n = s.frames.len();
        for pair in out.chunks_exact_mut(2) {
            let i = at as usize;
            if i >= n {
                self.at = None;
                return;
            }
            let frac = (at - i as f64) as f32;
            let a = s.frames[i];
            let b = s.frames[(i + 1).min(n - 1)];
            pair[0] += (a[0] + (b[0] - a[0]) * frac) * self.gain;
            pair[1] += (a[1] + (b[1] - a[1]) * frac) * self.gain;
            at += step;
        }
        self.at = Some(at);
    }
}

#[cfg(test)]
mod tests {
    use super::Prehear;
    use crate::sample::Sample;
    use std::sync::Arc;

    fn ones(frames: usize, rate: u32) -> Arc<Sample> {
        Arc::new(Sample { rate, frames: vec![[1.0, 1.0]; frames] })
    }

    fn render(p: &mut Prehear, frames: usize, sr: f64) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        p.mix(&mut out, sr);
        out
    }

    #[test]
    fn prehear_plays_once_then_stops() {
        let mut p = Prehear::default();
        assert!(p.set(Some(ones(100, 48_000)), 1.0).is_none());
        let out = render(&mut p, 200, 48_000.0);
        assert!(out[..180].iter().all(|x| *x > 0.9), "plays");
        assert!(out[202..].iter().all(|x| *x == 0.0), "then stops");
        assert!(render(&mut p, 50, 48_000.0).iter().all(|x| *x == 0.0));
    }

    #[test]
    fn none_stops_it() {
        let mut p = Prehear::default();
        p.set(Some(ones(1000, 48_000)), 1.0);
        render(&mut p, 10, 48_000.0);
        let old = p.set(None, 1.0);
        assert!(old.is_some(), "the old sample comes back to be freed");
        assert!(render(&mut p, 10, 48_000.0).iter().all(|x| *x == 0.0));
    }

    #[test]
    fn gain_scales_it() {
        let mut p = Prehear::default();
        p.set(Some(ones(1000, 48_000)), 0.5);
        let out = render(&mut p, 10, 48_000.0);
        assert!(out.iter().all(|x| (*x - 0.5).abs() < 1e-6));
    }

    #[test]
    fn another_rate_is_resampled() {
        let mut p = Prehear::default();
        // 100 frames at 24 kHz last 200 frames at 48 kHz.
        p.set(Some(ones(100, 24_000)), 1.0);
        let out = render(&mut p, 300, 48_000.0);
        assert!(out[..390].iter().all(|x| *x > 0.9));
        assert!(out[404..].iter().all(|x| *x == 0.0));
    }

    #[test]
    fn it_adds_to_what_is_there() {
        let mut p = Prehear::default();
        p.set(Some(ones(10, 48_000)), 1.0);
        let mut out = vec![0.25f32; 4];
        p.mix(&mut out, 48_000.0);
        assert!(out.iter().all(|x| (*x - 1.25).abs() < 1e-6));
    }
}
