//! Prehear: the browser's preview of a sample, played once through the master output
//! (beside the metronome, not tempo-synced), resampled to the output's rate.

use crate::sample::Sample;
use std::sync::Arc;

/// How long a sample that's cut off (by the next, or a stop) takes to fade out: a cut
/// mid-waveform clicks, as scrolling through the browser does many times a second.
const FADE_SECONDS: f64 = 0.01;

#[derive(Default)]
pub(crate) struct Prehear {
    sample: Option<Arc<Sample>>,
    /// Where it is, in the sample's frames; `None` once it has played.
    at: Option<f64>,
    gain: f32,
    /// The one cut off, fading out under the next: its place, gain and the part of the
    /// fade left (1 to 0).
    fading: Option<Fading>,
}

struct Fading {
    sample: Arc<Sample>,
    at: f64,
    gain: f32,
    left: f32,
}

impl Prehear {
    /// Play `sample` from its start at `gain` (`None` stops), the one playing fading
    /// out. Returns one it no longer holds, to be freed off the audio thread: the one
    /// replaced if it had finished, or the one that was fading.
    pub(crate) fn set(&mut self, sample: Option<Arc<Sample>>, gain: f32) -> Option<Arc<Sample>> {
        let playing = self.at.take();
        self.at = sample.as_ref().map(|_| 0.0);
        let old = std::mem::replace(&mut self.sample, sample);
        let old_gain = std::mem::replace(&mut self.gain, gain);
        match (old, playing) {
            (Some(sample), Some(at)) => self.fading.replace(Fading { sample, at, gain: old_gain, left: 1.0 }).map(|f| f.sample),
            (Some(finished), None) => Some(finished),
            (None, _) => self.fading.take_if(|f| f.left <= 0.0).map(|f| f.sample),
        }
    }

    /// Add it to `out` (interleaved stereo) at output rate `sr`.
    pub(crate) fn mix(&mut self, out: &mut [f32], sr: f64) {
        if let Some(f) = self.fading.as_mut().filter(|f| f.left > 0.0) {
            let step = f64::from(f.sample.rate) / sr;
            let fall = (1.0 / (FADE_SECONDS * sr)).min(1.0) as f32;
            let n = f.sample.frames.len();
            for pair in out.chunks_exact_mut(2) {
                let i = f.at as usize;
                f.left = (f.left - fall).max(0.0);
                if i >= n || f.left <= 0.0 {
                    f.left = 0.0;
                    break;
                }
                let frac = (f.at - i as f64) as f32;
                let (a, b) = (f.sample.frames[i], f.sample.frames[(i + 1).min(n - 1)]);
                let g = f.gain * f.left;
                pair[0] += (a[0] + (b[0] - a[0]) * frac) * g;
                pair[1] += (a[1] + (b[1] - a[1]) * frac) * g;
                f.at += step;
            }
        }
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
    fn none_stops_it_fading_out() {
        let mut p = Prehear::default();
        p.set(Some(ones(10_000, 48_000)), 1.0);
        render(&mut p, 10, 48_000.0);
        assert!(p.set(None, 1.0).is_none(), "kept while it fades");
        // 10 ms at 48 kHz: falling from where it was, never a jump, then silence.
        let out = render(&mut p, 600, 48_000.0);
        assert!(out[0] > 0.99, "no cut: {}", out[0]);
        assert!(out.chunks(2).collect::<Vec<_>>().windows(2).all(|w| w[1][0] <= w[0][0] && w[0][0] - w[1][0] < 0.01));
        assert!(out[2 * 480..].iter().all(|x| *x == 0.0), "gone in 10 ms");
        assert!(p.set(None, 1.0).is_some(), "then it comes back to be freed");
    }

    /// Scrolling: each next sample starts from its start at once, the one before fading
    /// under it instead of stopping dead (a click).
    #[test]
    fn the_next_starts_at_once_and_the_last_fades_under_it() {
        let mut p = Prehear::default();
        p.set(Some(ones(10_000, 48_000)), 1.0);
        render(&mut p, 100, 48_000.0);
        let zeros = Arc::new(Sample { rate: 48_000, frames: vec![[0.0, 0.0]; 10_000] });
        assert!(p.set(Some(zeros), 1.0).is_none());
        let out = render(&mut p, 600, 48_000.0);
        assert!(out[0] > 0.99 && out[2 * 240] > 0.4 && out[2 * 240] < 0.6, "the last fading: {} {}", out[0], out[2 * 240]);
        assert!(out[2 * 480..].iter().all(|x| *x == 0.0));
        // A third: the first goes back to be freed, the second fades.
        assert!(p.set(Some(ones(10, 48_000)), 1.0).is_some());
    }

    #[test]
    fn a_finished_one_comes_back_at_once() {
        let mut p = Prehear::default();
        p.set(Some(ones(10, 48_000)), 1.0);
        render(&mut p, 100, 48_000.0);
        assert!(p.set(None, 1.0).is_some(), "nothing to fade");
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
