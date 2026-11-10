//! Pitch detection with YIN (de Cheveigne and Kawahara, 2002).

/// Samples one detection reads.
pub const WINDOW: usize = 4096;

/// The highest aperiodicity a detection may have.
const THRESHOLD: f64 = 0.15;
/// The lowest pitch reported, in Hz.
const MIN_HZ: f64 = 20.0;

/// A detected pitch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pitch {
    pub hz: f32,
    /// `1 - aperiodicity` at the chosen dip; at least 0.85.
    pub clarity: f32,
}

/// The pitch of `samples` (the first `WINDOW` of them are used), or `None` when they
/// aren't periodic enough, are silent, or sound below 20 Hz.
pub fn detect(samples: &[f32], sample_rate: f32) -> Option<Pitch> {
    let n = samples.len().min(WINDOW);
    let half = n / 2;
    if half < 4 || sample_rate <= 0.0 {
        return None;
    }
    let x: Vec<f64> = samples[..n].iter().map(|&s| s as f64).collect();
    let rate = sample_rate as f64;
    // Longest lag that is still a period of 20 Hz or more.
    let max_tau = ((rate / MIN_HZ) as usize).min(half);
    if max_tau < 3 {
        return None;
    }
    // Silence: nothing to measure.
    if x[..half].iter().all(|s| s.abs() < 1e-9) {
        return None;
    }
    // Difference function over a fixed integration window of `half` samples, and its
    // cumulative mean normalised form, worked out only as far as the search needs them.
    let mut d = vec![0.0f64; max_tau + 1];
    let mut cmnd = vec![1.0f64; max_tau + 1];
    let mut sum = 0.0;
    let mut done = 0;
    let mut upto = |to: usize, d: &mut [f64], cmnd: &mut [f64]| {
        while done < to.min(max_tau) {
            done += 1;
            let tau = done;
            d[tau] = squared_difference(&x[..half], &x[tau..tau + half]);
            sum += d[tau];
            cmnd[tau] = if sum > 0.0 { d[tau] * tau as f64 / sum } else { 1.0 };
        }
    };
    // The first dip under the threshold, followed down to its bottom.
    let mut tau = 2;
    upto(3, &mut d, &mut cmnd);
    while tau < max_tau {
        upto(tau + 1, &mut d, &mut cmnd);
        if cmnd[tau] < THRESHOLD {
            loop {
                upto(tau + 2, &mut d, &mut cmnd);
                if !(tau + 1 < max_tau && cmnd[tau + 1] < cmnd[tau]) {
                    break;
                }
                tau += 1;
            }
            break;
        }
        tau += 1;
    }
    if tau >= max_tau || cmnd[tau] >= THRESHOLD {
        return None;
    }
    // Parabola through the raw difference for a first sub-sample lag, then a search on
    // the difference measured at fractional lags, which a parabola can't follow on
    // harmonic-rich waves at high pitches.
    let (a, b, c) = (d[tau - 1], d[tau], d[tau + 1]);
    let denom = a - 2.0 * b + c;
    let shift = if denom > 0.0 { ((a - c) / (2.0 * denom)).clamp(-1.0, 1.0) } else { 0.0 };
    let lag = refine(&x, tau as f64 + shift);
    let hz = rate / lag;
    if hz < MIN_HZ {
        return None;
    }
    Some(Pitch { hz: hz as f32, clarity: (1.0 - cmnd[tau]) as f32 })
}

/// The sum of `(a - b)²` over two equal slices, in four lanes so it vectorises.
fn squared_difference(a: &[f64], b: &[f64]) -> f64 {
    let mut lanes = [0.0f64; 4];
    let (ca, cb) = (a.chunks_exact(4), b.chunks_exact(4));
    let tail: f64 = ca.remainder().iter().zip(cb.remainder()).map(|(a, b)| (a - b) * (a - b)).sum();
    for (a, b) in ca.zip(cb) {
        for k in 0..4 {
            let e = a[k] - b[k];
            lanes[k] += e * e;
        }
    }
    lanes.iter().sum::<f64>() + tail
}

/// Half the taps of the interpolating filter.
const TAPS: usize = 8;

/// Golden-section steps: they narrow the two-sample bracket to under 1e-4 of a sample.
const REFINE_STEPS: usize = 20;

/// The Hann-windowed sinc taps that read a signal `frac` (0 to 1) of a sample past a point:
/// tap `i` weighs the sample `i - TAPS + 1` places on.
fn taps(frac: f64) -> [f64; 2 * TAPS] {
    std::array::from_fn(|i| {
        let dist = (i as isize - TAPS as isize + 1) as f64 - frac;
        let sinc = if dist.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * dist).sin() / (std::f64::consts::PI * dist) };
        sinc * (0.5 + 0.5 * (std::f64::consts::PI * dist / TAPS as f64).cos())
    })
}

/// The lag near `start` (within a sample) where the signal best matches itself.
fn refine(x: &[f64], start: f64) -> f64 {
    let n = x.len();
    let reach = start.ceil() as usize + 1 + TAPS;
    if reach + 2 * TAPS + 2 >= n {
        return start;
    }
    let end = n - reach - TAPS - 1;
    // Every sample is read at the same fraction past a whole one, so one set of taps serves.
    let diff = |lag: f64| -> f64 {
        let whole = lag.floor();
        let w = taps(lag - whole);
        let first = whole as isize - TAPS as isize + 1;
        (TAPS..end.max(TAPS))
            .map(|j| {
                let at = (j as isize + first) as usize;
                let shifted: f64 = x[at..at + 2 * TAPS].iter().zip(&w).map(|(s, c)| s * c).sum();
                let e = x[j] - shifted;
                e * e
            })
            .sum()
    };
    // Golden-section search over a one-sample bracket.
    let ratio = 0.618_033_988_749_895;
    let (mut lo, mut hi) = (start - 1.0, start + 1.0);
    let mut m1 = hi - ratio * (hi - lo);
    let mut m2 = lo + ratio * (hi - lo);
    let (mut f1, mut f2) = (diff(m1), diff(m2));
    for _ in 0..REFINE_STEPS {
        if f1 < f2 {
            hi = m2;
            m2 = m1;
            f2 = f1;
            m1 = hi - ratio * (hi - lo);
            f1 = diff(m1);
        } else {
            lo = m1;
            m1 = m2;
            f1 = f2;
            m2 = lo + ratio * (hi - lo);
            f2 = diff(m2);
        }
    }
    (lo + hi) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48000.0;

    fn cents(a: f32, b: f32) -> f32 {
        1200.0 * (a / b).log2()
    }

    /// Band-limited harmonics `amp(k)` of a wave at `hz`, with a random-ish phase.
    fn wave(hz: f32, amp: impl Fn(usize) -> f32) -> Vec<f32> {
        let top = ((RATE / 2.0 * 0.98) / hz) as usize;
        (0..WINDOW)
            .map(|i| {
                let t = i as f64 / RATE as f64;
                (1..=top.max(1))
                    .map(|k| amp(k) as f64 * (std::f64::consts::TAU * hz as f64 * k as f64 * t + 0.3 * k as f64).sin())
                    .sum::<f64>() as f32
            })
            .collect()
    }

    #[test]
    fn detects_a_sine_saw_and_filtered_saw_within_one_cent() {
        for hz in [32.7f32, 110.0, 440.0, 2093.0] {
            let waves = [
                ("sine", wave(hz, |k| if k == 1 { 0.5 } else { 0.0 })),
                ("saw", wave(hz, |k| 0.4 / k as f32)),
                ("filtered saw", wave(hz, |k| 0.4 / (k * k) as f32)),
            ];
            for (name, samples) in waves {
                let p = detect(&samples, RATE).unwrap_or_else(|| panic!("{name} at {hz} Hz: no pitch"));
                assert!(cents(p.hz, hz).abs() < 1.0, "{name} at {hz} Hz read {} ({} cents)", p.hz, cents(p.hz, hz));
                assert!(p.clarity >= 0.85, "{name} at {hz} Hz clarity {}", p.clarity);
            }
        }
    }

    /// A detection costs well under a hop, so the tuner keeps up at any rate. Timed only in
    /// release builds: `cargo test --release -p deckard-plugins --lib zhora::pitch`.
    #[cfg(not(debug_assertions))]
    #[test]
    fn a_detection_takes_under_a_quarter_of_a_hop() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};
        let budget = Duration::from_secs_f64(super::super::HOP as f64 / RATE as f64 / 4.0);
        for hz in [32.7f32, 110.0, 440.0, 2093.0] {
            for samples in [wave(hz, |k| if k == 1 { 0.5 } else { 0.0 }), wave(hz, |k| 0.4 / k as f32)] {
                // The middle of several, so another test running at once doesn't decide it.
                let mut times: Vec<Duration> = (0..9)
                    .map(|_| {
                        let t = Instant::now();
                        black_box(detect(black_box(&samples), RATE));
                        t.elapsed()
                    })
                    .collect();
                times.sort();
                assert!(times[4] < budget, "{hz} Hz took {:?}, over {budget:?}", times[4]);
            }
        }
    }

    #[test]
    fn noise_and_silence_give_no_pitch() {
        assert_eq!(detect(&vec![0.0; WINDOW], RATE), None);
        let mut seed = 12345u32;
        let noise: Vec<f32> = (0..WINDOW)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
            })
            .collect();
        assert_eq!(detect(&noise, RATE), None);
        // Below 20 Hz is not a pitch either.
        assert_eq!(detect(&wave(15.0, |k| if k == 1 { 0.5 } else { 0.0 }), RATE), None);
    }
}
