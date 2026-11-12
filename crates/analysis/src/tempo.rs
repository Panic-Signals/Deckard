//! A sample's tempo.

use crate::{mono, onset};

/// Where a tempo came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempoSource {
    /// Given (the file's name).
    Hint,
    /// Heard, then made exact: the range holds a whole number of bars (or 2ⁿ beats).
    Loop,
    /// Heard.
    Audio,
}

/// A sample's tempo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tempo {
    /// Beats per minute: folded into 85–170, or the hint.
    pub bpm: f64,
    /// The sample's length in beats at `bpm`.
    pub beats: f64,
    /// The frame of the first beat: where the beat grid falls, within one beat.
    pub offset: usize,
    /// How sure: 0 to 1; 1 for a hint.
    pub confidence: f32,
    pub source: TempoSource,
    /// The tempo ×2 and ÷2, the usual mistakes, for the ×2 and ½ buttons.
    pub candidates: [f64; 2],
}

/// The range searched, and the one tempos are folded into.
const SEARCH: (f64, f64) = (60.0, 200.0);
const FOLD: (f64, f64) = (85.0, 170.0);
/// How near a whole number of bars the length must be for the tempo to snap to it.
const LOOP_TOLERANCE: f64 = 0.02;

/// The tempo of `frames` at `rate`, or `None` for one-shots, silence and audio without a
/// beat. `hint` (the tempo in the file's name) wins over what's heard; the audio still
/// places the beat grid.
pub fn tempo(frames: &[[f32; 2]], rate: u32, hint: Option<f64>) -> Option<Tempo> {
    if rate == 0 {
        return None;
    }
    let seconds = frames.len() as f64 / f64::from(rate);
    let make = |bpm: f64, offset: usize, confidence: f32, source: TempoSource| Tempo {
        bpm,
        beats: seconds * bpm / 60.0,
        offset,
        confidence,
        source,
        candidates: [bpm * 2.0, bpm / 2.0],
    };
    let (raw, hop) = onset::envelope(&mono(frames), rate);
    // Smoothed over a few hops, so an onset's hop (where it falls in the grid of hops)
    // doesn't favour tempos whose beat is a whole number of hops.
    let env = smooth(&raw);
    let lag_of = |bpm: f64| 60.0 * f64::from(rate) / (bpm * hop);
    if let Some(h) = hint.filter(|h| h.is_finite() && *h > 0.0) {
        return Some(make(h, offset(&env, lag_of(h), hop), 1.0, TempoSource::Hint));
    }
    if seconds < 1.0 || onsets(&env) < 4 {
        return None;
    }
    // Coarse: every 0.1 BPM, the lags of a beat, two beats and half a beat, weighed by
    // the prior. The half beat keeps 2:3 relatives (a dotted beat) from winning: their
    // halves fall between the 8ths.
    let prior = |bpm: f64| (-0.5 * (bpm / 120.0).log2().powi(2)).exp();
    let mut scores = Vec::new();
    let mut bpm = SEARCH.0;
    while bpm <= SEARCH.1 {
        let l = lag_of(bpm);
        scores.push((bpm, prior(bpm) * (ac(&env, l) + 0.5 * ac(&env, 2.0 * l) + 0.5 * ac(&env, 0.5 * l))));
        bpm += 0.1;
    }
    let (best, top) = scores.iter().copied().fold((0.0, f64::MIN), |a, b| if b.1 > a.1 { b } else { a });
    let mean = scores.iter().map(|s| s.1.max(0.0)).sum::<f64>() / scores.len() as f64;
    if top <= 0.0 || mean <= 0.0 {
        return None;
    }
    let confidence = ((top / mean - 1.0) / 3.0).clamp(0.0, 1.0) as f32;
    if confidence < 0.1 {
        return None;
    }
    // Fine: every 0.01 BPM within 1 BPM, over as many beats' lags as fit in half the
    // sample, so the error doesn't grow with the hop.
    let fine = |bpm: f64| {
        let l = lag_of(bpm);
        let k = ((env.len() as f64 / 2.0) / l).floor().max(1.0) as usize;
        (1..=k).map(|i| ac(&env, l * i as f64)).sum::<f64>()
    };
    let mut bpm = best;
    let mut top = fine(best);
    for i in -100..=100 {
        let b = best + f64::from(i) * 0.01;
        let s = fine(b);
        if s > top {
            (bpm, top) = (b, s);
        }
    }
    while bpm < FOLD.0 {
        bpm *= 2.0;
    }
    while bpm > FOLD.1 {
        bpm /= 2.0;
    }
    let mut source = TempoSource::Audio;
    let n = seconds * bpm / 60.0;
    let whole = (0..=8).map(|k| f64::from(1u32 << k)).chain((1..=64).map(|m| 4.0 * f64::from(m)));
    if let Some(c) = whole.min_by(|a, b| (n - a).abs().total_cmp(&(n - b).abs()))
        && ((n - c) / c).abs() < LOOP_TOLERANCE
    {
        bpm = 60.0 * c / seconds;
        source = TempoSource::Loop;
    }
    Some(make(bpm, offset(&env, lag_of(bpm), hop), confidence, source))
}

/// `env` smoothed by a short triangle (1 2 3 2 1).
fn smooth(env: &[f32]) -> Vec<f32> {
    const W: [f32; 5] = [1.0 / 9.0, 2.0 / 9.0, 3.0 / 9.0, 2.0 / 9.0, 1.0 / 9.0];
    (0..env.len())
        .map(|i| W.iter().enumerate().filter_map(|(k, w)| (i + k).checked_sub(2).and_then(|j| env.get(j)).map(|v| v * w)).sum())
        .collect()
}

/// How many distinct onsets stand out (local peaks over the mean plus one deviation).
fn onsets(env: &[f32]) -> usize {
    let n = env.len().max(1) as f32;
    let sd = (env.iter().map(|v| v * v).sum::<f32>() / n).sqrt(); // the mean is 0
    (1..env.len().saturating_sub(1)).filter(|&i| env[i] > sd && env[i] >= env[i - 1] && env[i] > env[i + 1]).count()
}

/// The envelope's autocorrelation at a fractional `lag` (in hops), per overlapping hop.
fn ac(env: &[f32], lag: f64) -> f64 {
    let whole = lag.floor() as usize;
    let frac = lag - whole as f64;
    if whole + 1 >= env.len() {
        return 0.0;
    }
    let n = env.len() - whole - 1;
    let sum: f64 = (0..n)
        .map(|i| f64::from(env[i]) * ((1.0 - frac) * f64::from(env[i + whole]) + frac * f64::from(env[i + whole + 1])))
        .sum();
    sum / n as f64
}

/// The frame where the beat grid (one beat every `lag` hops) best meets the onsets.
fn offset(env: &[f32], lag: f64, hop: f64) -> usize {
    if env.is_empty() || !(lag >= 1.0) {
        return 0;
    }
    let phases = lag.ceil() as usize;
    let score = |p: usize| {
        let mut s = 0.0;
        let mut at = p as f64;
        while (at.round() as usize) < env.len() {
            s += f64::from(env[at.round() as usize]);
            at += lag;
        }
        s
    };
    let best = (0..phases.min(env.len())).max_by(|&a, &b| score(a).total_cmp(&score(b))).unwrap_or(0);
    (best as f64 * hop).round() as usize
}

#[cfg(test)]
mod tests {
    use super::{TempoSource, tempo};
    use crate::test_signals::drums;

    const SR: u32 = 44_100;

    #[test]
    fn finds_the_tempo_of_a_drum_loop() {
        for bpm in [93.0, 120.0, 140.0] {
            let t = tempo(&drums(bpm, 4, SR, 0.5), SR, None).expect("a tempo");
            assert!((t.bpm - bpm).abs() < 0.05, "{bpm}: {t:?}");
            assert_eq!(t.source, TempoSource::Loop);
            assert!((t.beats - 16.0).abs() < 0.01);
        }
    }

    /// Every tempo from 70 to 180 BPM, straight and swung, 2 and 4 bars: right, or off by
    /// exactly ×2 / ÷2 (folding), never by anything else.
    #[test]
    fn tempos_across_the_range() {
        let mut wrong = Vec::new();
        for bpm in (70..=180).step_by(3).map(f64::from) {
            for (bars, swing) in [(4, 0.5), (2, 0.5), (4, 0.62)] {
                let t = tempo(&drums(bpm, bars, SR, swing), SR, None);
                let ok = t.is_some_and(|t| [bpm, bpm * 2.0, bpm / 2.0].iter().any(|b| (t.bpm - b).abs() < 0.05));
                if !ok {
                    wrong.push((bpm, bars, swing, t.map(|t| t.bpm)));
                }
            }
        }
        assert!(wrong.is_empty(), "{wrong:?}");
    }

    #[test]
    fn the_tempo_of_an_unlooped_range() {
        let mut f = drums(120.0, 4, SR, 0.5);
        f.extend(std::iter::repeat_n([0.0; 2], (0.3 * f64::from(SR)) as usize));
        let t = tempo(&f, SR, None).expect("a tempo");
        assert!((t.bpm - 120.0).abs() < 1.0, "{t:?}");
        assert_eq!(t.source, TempoSource::Audio);
    }

    #[test]
    fn folds_fast_tempos_and_keeps_the_other() {
        let t = tempo(&drums(174.0, 4, SR, 0.5), SR, None).expect("a tempo");
        assert!((t.bpm - 87.0).abs() < 0.1, "{t:?}");
        assert!((t.candidates[0] - 174.0).abs() < 0.2);
    }

    #[test]
    fn swing_doesnt_throw_it() {
        let t = tempo(&drums(120.0, 4, SR, 0.6), SR, None).expect("a tempo");
        assert!((t.bpm - 120.0).abs() < 0.5, "{t:?}");
    }

    #[test]
    fn the_hint_wins() {
        let t = tempo(&drums(120.0, 4, SR, 0.5), SR, Some(100.0)).unwrap();
        assert_eq!((t.bpm, t.source, t.confidence), (100.0, TempoSource::Hint, 1.0));
    }

    #[test]
    fn a_single_hit_has_no_tempo() {
        let one_kick: Vec<[f32; 2]> = drums(60.0, 1, SR, 0.5)[..(0.5 * f64::from(SR)) as usize].to_vec();
        let mut f = one_kick;
        f.extend(std::iter::repeat_n([0.0; 2], 2 * SR as usize));
        assert_eq!(tempo(&f, SR, None), None);
    }

    #[test]
    fn silence_and_tiny_inputs() {
        assert_eq!(tempo(&[], SR, None), None);
        assert_eq!(tempo(&[[0.1; 2]; 10], SR, None), None);
        assert_eq!(tempo(&vec![[0.0; 2]; 2 * SR as usize], SR, None), None);
        assert_eq!(tempo(&drums(120.0, 4, SR, 0.5), 0, None), None);
    }

    #[test]
    fn other_rates_agree() {
        for rate in [22_050, 96_000] {
            let t = tempo(&drums(120.0, 4, rate, 0.5), rate, None).expect("a tempo");
            assert!((t.bpm - 120.0).abs() < 0.05, "{rate}: {t:?}");
        }
    }

    #[test]
    fn the_offset_finds_the_first_beat() {
        let f = drums(120.0, 4, SR, 0.5);
        let quarter = (0.25 * 0.5 * f64::from(SR)) as usize; // ¼ of a 0.5 s beat
        let mut rotated = f[f.len() - quarter..].to_vec();
        rotated.extend_from_slice(&f[..f.len() - quarter]);
        let t = tempo(&rotated, SR, None).expect("a tempo");
        let hop = 256;
        assert!(t.offset.abs_diff(quarter) <= hop * 3 / 2, "offset {} vs {quarter}", t.offset);
    }
}
