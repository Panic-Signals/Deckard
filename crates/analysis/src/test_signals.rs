//! Made-up audio with known answers, for the tests.

use std::f64::consts::TAU;

/// A deterministic noise source in -1..1.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

/// `bars` bars of a drum loop at `bpm`: kicks on beats 1 and 3, snares on 2 and 4,
/// closed hats on the 8ths, the off-beat 8ths at `swing` of the beat (0.5 is straight).
/// Exactly `bars` × 4 beats long; tails past the end are cut.
pub fn drums(bpm: f64, bars: usize, rate: u32, swing: f64) -> Vec<[f32; 2]> {
    let beat = 60.0 / bpm * f64::from(rate);
    let len = (beat * 4.0 * bars as f64).round() as usize;
    let mut out = vec![[0f32; 2]; len];
    let mut noise = Noise(1);
    let mut add = |at: f64, f: &mut dyn FnMut(f64) -> f32, secs: f64| {
        let start = at.round() as usize;
        let n = (secs * f64::from(rate)) as usize;
        for i in 0..n {
            if start + i >= len {
                break;
            }
            let v = f(i as f64 / f64::from(rate));
            out[start + i][0] += v;
            out[start + i][1] += v;
        }
    };
    for b in 0..bars * 4 {
        let at = b as f64 * beat;
        if b % 2 == 0 {
            add(at, &mut |t| (0.9 * (TAU * 55.0 * t).sin() * (-t / 0.04).exp()) as f32, 0.12);
        } else {
            add(at, &mut |t| 0.5 * noise.next() * (-t / 0.05).exp() as f32, 0.15);
        }
        for (k, frac) in [0.0, swing].into_iter().enumerate() {
            let mut n2 = Noise(7 + b as u32 * 2 + k as u32);
            add(at + frac * beat, &mut |t| 0.15 * n2.next() * (-t / 0.003).exp() as f32, 0.008);
        }
    }
    out
}

/// `seconds` of a tone at `hz`: `harmonics` partials falling as 1/k.
pub fn tone(hz: f64, seconds: f64, rate: u32, harmonics: usize) -> Vec<[f32; 2]> {
    let n = (seconds * f64::from(rate)) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            let v: f64 = (1..=harmonics).map(|k| (TAU * hz * k as f64 * t).sin() / k as f64).sum();
            let v = (0.3 * v) as f32;
            [v, v]
        })
        .collect()
}
