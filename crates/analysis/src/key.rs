//! A recording's key: its tonic and whether it's minor, from which pitch classes sound
//! most (a chromagram) set against how often each degree of a key is heard. This is the
//! root of what has no single note to find: chords, a band, a whole song.
//!
//! The method is the published one: C. Krumhansl's key profiles (Cognitive Foundations of
//! Musical Pitch, 1990) in I. Sha'ath's revision for recorded music (Estimation of Key in
//! Digital Music Recordings, 2011), correlated with the chromagram at each of 24 keys.

use std::f32::consts::PI;

use crate::mono;

/// A recording's key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key {
    /// The tonic's pitch class: 0 is C, 9 is A.
    pub tonic: u8,
    pub minor: bool,
    /// How far the recording is tuned from concert pitch: −50 to +50.
    pub cents: f32,
    /// How sure: 0 to 1.
    pub confidence: f32,
}

/// The rate the spectrum is taken at, about: the notes that name a key are below 2 kHz.
const RATE: u32 = 11_025;
/// A window (0.74 s at 11,025 Hz: 1.35 Hz a bin, three bins a semitone at the lowest
/// note) and the step between windows.
const WINDOW: usize = 8192;
const HOP: usize = 4096;
/// The notes heard: C1 to C6 (MIDI 36 to 96).
const LOW: f32 = 36.0;
const HIGH: f32 = 96.0;
/// How much of a long recording is listened to (seconds), from its middle.
const LISTEN: f64 = 120.0;
/// Windows quieter than this (RMS) don't count: −50 dBFS.
const MIN_RMS: f32 = 0.003_162;
/// The least a key must stand out to be named: its correlation with the chromagram, and
/// its lead over the best key of other notes.
const MIN_FIT: f32 = 0.55;
const MIN_LEAD: f32 = 0.04;
/// The least share of the spectrum's energy in peaks: below it it's noise or drums.
const MIN_TONAL: f32 = 0.2;

const MAJOR: [f32; 12] = [6.6, 2.0, 3.5, 2.3, 4.6, 4.0, 2.5, 5.2, 2.4, 3.7, 2.3, 3.4];
const MINOR: [f32; 12] = [6.5, 2.7, 3.5, 5.4, 2.6, 3.5, 2.5, 5.2, 4.0, 2.7, 4.3, 3.2];

/// The key of `frames` at `rate`, or `None` when none stands out (drums, noise, silence,
/// a single note).
pub fn key(frames: &[[f32; 2]], rate: u32) -> Option<Key> {
    if rate == 0 {
        return None;
    }
    let most = (LISTEN * f64::from(rate)) as usize;
    let from = frames.len().saturating_sub(most) / 2;
    let part = &frames[from..(from + most).min(frames.len())];
    // Down to about RATE by averaging runs of samples.
    let by = (rate / RATE).max(1) as usize;
    let rate = rate as f32 / by as f32;
    let x: Vec<f32> = mono(part).chunks(by).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    if x.len() < WINDOW {
        return None;
    }
    let hann: Vec<f32> = (0..WINDOW).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / WINDOW as f32).cos()).collect();
    // Every spectral peak as (its note, exact; its weight), and how tonal each window is.
    let mut peaks: Vec<(f32, f32)> = Vec::new();
    let (mut tonal, mut windows) = (0f32, 0usize);
    let (mut re, mut im) = (vec![0f32; WINDOW], vec![0f32; WINDOW]);
    let mut at = 0;
    while at + WINDOW <= x.len() {
        let w = &x[at..at + WINDOW];
        at += HOP;
        let rms = (w.iter().map(|s| s * s).sum::<f32>() / WINDOW as f32).sqrt();
        if rms < MIN_RMS {
            continue;
        }
        for i in 0..WINDOW {
            re[i] = w[i] * hann[i];
            im[i] = 0.0;
        }
        fft(&mut re, &mut im);
        let mag: Vec<f32> = (0..WINDOW / 2).map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt()).collect();
        let note_of = |bin: f32| 69.0 + 12.0 * (bin * rate / WINDOW as f32 / 440.0).log2();
        let bin_of = |note: f32| 440.0 * 2f32.powf((note - 69.0) / 12.0) * WINDOW as f32 / rate;
        let (lo, hi) = (bin_of(LOW).floor().max(2.0) as usize, (bin_of(HIGH).ceil() as usize).min(WINDOW / 2 - 2));
        let total: f32 = mag[lo..hi].iter().map(|m| m * m).sum();
        let mut in_peaks = 0f32;
        let mut found: Vec<(f32, f32)> = Vec::new();
        for i in lo..hi {
            let (a, b, c) = (mag[i - 1], mag[i], mag[i + 1]);
            if b > a && b >= c && b > mag[i - 2] && b > mag[i + 2] {
                // The peak's place between bins, by a parabola through its log heights.
                let (la, lb, lc) = (a.max(1e-12).ln(), b.ln(), c.max(1e-12).ln());
                let d = 0.5 * (la - lc) / (la - 2.0 * lb + lc);
                found.push((note_of(i as f32 + d.clamp(-0.5, 0.5)), b));
                in_peaks += a * a + b * b + c * c;
            }
        }
        // The loudest peaks of the window only, the rest is the noise between them.
        let top = found.iter().map(|p| p.1).fold(0.0, f32::max);
        // Weighed by the square root, so one loud note doesn't drown a chord.
        peaks.extend(found.iter().filter(|p| p.1 > 0.05 * top).map(|p| (p.0, (p.1 / top).sqrt() * rms.sqrt())));
        let strong: f32 = found.iter().filter(|p| p.1 > 0.05 * top).map(|p| p.1 * p.1).sum();
        let all: f32 = found.iter().map(|p| p.1 * p.1).sum();
        if total > 0.0 && all > 0.0 {
            tonal += strong / all * (in_peaks / total).min(1.0);
        }
        windows += 1;
    }
    if windows == 0 || peaks.is_empty() {
        return None;
    }
    // The tuning: where the peaks sit between semitones, as the angle of their mean.
    let (mut s, mut c) = (0f32, 0f32);
    for &(n, w) in &peaks {
        let a = 2.0 * PI * (n - n.round());
        s += w * a.sin();
        c += w * a.cos();
    }
    let tuning = s.atan2(c) / (2.0 * PI);
    // The chromagram: each peak to its pitch class, less the further it is from one.
    let mut chroma = [0f32; 12];
    for &(n, w) in &peaks {
        let n = n - tuning;
        let near = n.round();
        let off = (n - near).abs();
        chroma[(near as i32).rem_euclid(12) as usize] += w * (1.0 - 2.0 * off).max(0.0);
    }
    // Each key's fit: the correlation of the chromagram with its profile.
    let fit = |profile: &[f32; 12], tonic: usize| {
        let mean = |v: &[f32; 12]| v.iter().sum::<f32>() / 12.0;
        let (mc, mp) = (mean(&chroma), mean(profile));
        let (mut num, mut dc, mut dp) = (0f32, 0f32, 0f32);
        for (i, p) in profile.iter().enumerate() {
            let (a, b) = (chroma[(i + tonic) % 12] - mc, p - mp);
            num += a * b;
            dc += a * a;
            dp += b * b;
        }
        if dc > 0.0 { num / (dc * dp).sqrt() } else { 0.0 }
    };
    let mut fits: Vec<(f32, u8, bool)> = Vec::new();
    for t in 0..12 {
        fits.push((fit(&MAJOR, t), t as u8, false));
        fits.push((fit(&MINOR, t), t as u8, true));
    }
    fits.sort_by(|a, b| b.0.total_cmp(&a.0));
    let best = fits[0];
    // Its lead is over the keys of other notes: a major key and its relative minor are
    // the same seven, and which of the two is the lesser question.
    let relative = if best.2 { (best.1 + 3) % 12 } else { (best.1 + 9) % 12 };
    let next = fits.iter().find(|f| f.1 != best.1 && !(f.1 == relative && f.2 != best.2)).map_or(0.0, |f| f.0);
    let tonal = tonal / windows as f32;
    if best.0 < MIN_FIT || best.0 - next < MIN_LEAD || tonal < MIN_TONAL {
        return None;
    }
    let confidence = ((best.0 - next) / 0.3).clamp(0.0, 1.0);
    Some(Key { tonic: best.1, minor: best.2, cents: 100.0 * tuning, confidence })
}

/// The Fourier transform of `re` + i `im` in place; their length is a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1f64, 0f64);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (xr, xi) = (re[b] * cr as f32 - im[b] * ci as f32, re[b] * ci as f32 + im[b] * cr as f32);
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
                (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::key;
    use crate::test_signals::{drums, tone};

    const SR: u32 = 44_100;

    /// `chords` (MIDI notes each) one after another, `seconds` each.
    fn chords(chords: &[&[u8]], seconds: f64) -> Vec<[f32; 2]> {
        let mut out = Vec::new();
        for c in chords {
            let mut mix = vec![[0f32; 2]; (seconds * f64::from(SR)) as usize];
            for &n in *c {
                let hz = 440.0 * 2f64.powf((f64::from(n) - 69.0) / 12.0);
                for (m, t) in mix.iter_mut().zip(tone(hz, seconds, SR, 4)) {
                    m[0] += t[0] / c.len() as f32;
                    m[1] += t[1] / c.len() as f32;
                }
            }
            out.extend(mix);
        }
        out
    }

    #[test]
    fn a_major_cadence() {
        // C, F, G, C.
        let f = chords(&[&[48, 60, 64, 67], &[53, 60, 65, 69], &[55, 59, 62, 67], &[48, 60, 64, 67]], 1.5);
        let k = key(&f, SR).expect("a key");
        assert_eq!((k.tonic, k.minor), (0, false), "{k:?}");
        assert!(k.cents.abs() < 5.0, "{k:?}");
    }

    #[test]
    fn a_minor_cadence() {
        // A minor, D minor, E, A minor.
        let f = chords(&[&[45, 57, 60, 64], &[50, 57, 62, 65], &[52, 56, 59, 64], &[45, 57, 60, 64]], 1.5);
        let k = key(&f, SR).expect("a key");
        assert_eq!((k.tonic, k.minor), (9, true), "{k:?}");
    }

    #[test]
    fn drums_and_silence_have_no_key() {
        assert_eq!(key(&drums(120.0, 4, SR, 0.5), SR), None);
        assert_eq!(key(&vec![[0.0; 2]; 4 * SR as usize], SR), None);
        assert_eq!(key(&[], SR), None);
        assert_eq!(key(&[[0.1; 2]; 100], 0), None);
    }

    #[test]
    fn other_rates_agree() {
        let f: Vec<[f32; 2]> = chords(&[&[48, 60, 64, 67], &[53, 60, 65, 69], &[55, 59, 62, 67], &[48, 60, 64, 67]], 1.5);
        // The same frames called 48 kHz are 147 cents sharp: a semitone and a half up.
        let k = key(&f, 48_000).expect("a key");
        assert!(k.tonic == 1 || k.tonic == 2, "{k:?}");
    }
}
