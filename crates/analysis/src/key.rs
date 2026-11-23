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

/// The bass is heard lower and closer: C0 to C3 (MIDI 24 to 60), at a quarter of the
/// rate (a window is 1.5 s there: 0.67 Hz a bin, three bins a semitone at the lowest
/// note), with what's above it filtered out first.
const BASS_LOW: f32 = 24.0;
const BASS_HIGH: f32 = 60.0;
const BASS_RATE: u32 = RATE / 4;
const BASS_WINDOW: usize = 4096;
const BASS_CUT: f32 = 260.0;
/// Keys this near the best fit (and no worse than `GUESS_FIT`) are the ones the bass
/// chooses between when none stands out.
const CLOSE: f32 = 0.1;
const GUESS_FIT: f32 = 0.45;
/// The least the bass must fit its choice better than the next note's key.
const BASS_LEAD: f32 = 0.08;
/// The least of a mix's power that must be in its low end for that to be its bass.
const BASS_SHARE: f32 = 0.05;

/// What's heard in a recording: how much of each pitch class, how far from concert pitch
/// it's tuned (semitones), and how much of it is notes (0 to 1).
struct Heard {
    chroma: [f32; 12],
    tuning: f32,
    tonal: f32,
}

/// `x` (mono, at `rate`) heard in windows of `window` samples, over the notes `low` to
/// `high` (MIDI). `None`: too short, or silence.
fn hear(x: &[f32], rate: f32, window: usize, low: f32, high: f32) -> Option<Heard> {
    if x.len() < window {
        return None;
    }
    let hann: Vec<f32> = (0..window).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / window as f32).cos()).collect();
    // Every spectral peak as (its note, exact; its weight), and how tonal each window is.
    let mut peaks: Vec<(f32, f32)> = Vec::new();
    let (mut tonal, mut windows) = (0f32, 0usize);
    let (mut re, mut im) = (vec![0f32; window], vec![0f32; window]);
    let mut at = 0;
    while at + window <= x.len() {
        let w = &x[at..at + window];
        at += window / 2;
        let rms = (w.iter().map(|s| s * s).sum::<f32>() / window as f32).sqrt();
        if rms < MIN_RMS {
            continue;
        }
        for i in 0..window {
            re[i] = w[i] * hann[i];
            im[i] = 0.0;
        }
        fft(&mut re, &mut im);
        let mag: Vec<f32> = (0..window / 2).map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt()).collect();
        let note_of = |bin: f32| 69.0 + 12.0 * (bin * rate / window as f32 / 440.0).log2();
        let bin_of = |note: f32| 440.0 * 2f32.powf((note - 69.0) / 12.0) * window as f32 / rate;
        let (lo, hi) = (bin_of(low).floor().max(2.0) as usize, (bin_of(high).ceil() as usize).min(window / 2 - 2));
        if hi <= lo {
            continue;
        }
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
    Some(Heard { chroma, tuning, tonal: tonal / windows as f32 })
}

/// How well `chroma` fits the key of `tonic` (`minor` or major): their correlation.
fn fit(chroma: &[f32; 12], tonic: usize, minor: bool) -> f32 {
    let profile = if minor { &MINOR } else { &MAJOR };
    let mean = |v: &[f32; 12]| v.iter().sum::<f32>() / 12.0;
    let (mc, mp) = (mean(chroma), mean(profile));
    let (mut num, mut dc, mut dp) = (0f32, 0f32, 0f32);
    for (i, p) in profile.iter().enumerate() {
        let (a, b) = (chroma[(i + tonic) % 12] - mc, p - mp);
        num += a * b;
        dc += a * a;
        dp += b * b;
    }
    if dc > 0.0 { num / (dc * dp).sqrt() } else { 0.0 }
}

/// Every key's fit to `chroma`, the best first: (fit, tonic, minor).
fn fits(chroma: &[f32; 12]) -> Vec<(f32, u8, bool)> {
    let mut fits: Vec<(f32, u8, bool)> = Vec::new();
    for t in 0..12 {
        fits.push((fit(chroma, t, false), t as u8, false));
        fits.push((fit(chroma, t, true), t as u8, true));
    }
    fits.sort_by(|a, b| b.0.total_cmp(&a.0));
    fits
}

/// The best fit's lead over the keys of other notes: a major key and its relative minor
/// are the same seven, and which of the two is the lesser question.
fn lead(fits: &[(f32, u8, bool)]) -> f32 {
    let best = fits[0];
    let relative = if best.2 { (best.1 + 3) % 12 } else { (best.1 + 9) % 12 };
    best.0 - fits.iter().find(|f| f.1 != best.1 && !(f.1 == relative && f.2 != best.2)).map_or(0.0, |f| f.0)
}

/// The middle of `frames` that's listened to, mono, at about `to` Hz (by averaging runs
/// of samples), and the rate it's at.
fn listened(frames: &[[f32; 2]], rate: u32, to: u32) -> (Vec<f32>, f32) {
    let most = (LISTEN * f64::from(rate)) as usize;
    let from = frames.len().saturating_sub(most) / 2;
    let part = &frames[from..(from + most).min(frames.len())];
    let by = (rate / to).max(1) as usize;
    (mono(part).chunks(by).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect(), rate as f32 / by as f32)
}

/// The mix heard for its key.
fn heard(frames: &[[f32; 2]], rate: u32) -> Option<Heard> {
    if rate == 0 {
        return None;
    }
    let (x, rate) = listened(frames, rate, RATE);
    hear(&x, rate, WINDOW, LOW, HIGH)
}

/// The bass of `frames` heard: what's below `BASS_CUT`, at a quarter of the rate.
///
/// `of_mix`: `frames` is the whole mix, which may have no bass to speak of: `None` then
/// (what's left under the filter is the bottom of other things, not a bass line).
fn heard_bass(frames: &[[f32; 2]], rate: u32, of_mix: bool) -> Option<Heard> {
    if rate == 0 {
        return None;
    }
    let most = (LISTEN * f64::from(rate)) as usize;
    let from = frames.len().saturating_sub(most) / 2;
    let mut x = mono(&frames[from..(from + most).min(frames.len())]);
    let power = |x: &[f32]| x.iter().map(|s| s * s).sum::<f32>();
    let whole = power(&x);
    // Four one-pole low-passes (24 dB an octave): what's above would fold into the bass.
    let k = 1.0 - (-2.0 * PI * BASS_CUT / rate as f32).exp();
    for _ in 0..4 {
        let mut y = 0f32;
        for s in &mut x {
            y += k * (*s - y);
            *s = y;
        }
    }
    if of_mix && power(&x) < BASS_SHARE * whole {
        return None;
    }
    let by = (rate / BASS_RATE).max(1) as usize;
    let x: Vec<f32> = x.chunks(by).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    hear(&x, rate as f32 / by as f32, BASS_WINDOW, BASS_LOW, BASS_HIGH)
}

/// The key of `frames` at `rate`, or `None` when none stands out (drums, noise, silence,
/// a single note).
pub fn key(frames: &[[f32; 2]], rate: u32) -> Option<Key> {
    let h = heard(frames, rate)?;
    let fits = fits(&h.chroma);
    let (best, lead) = (fits[0], lead(&fits));
    if best.0 < MIN_FIT || lead < MIN_LEAD || h.tonal < MIN_TONAL {
        return None;
    }
    Some(Key { tonic: best.1, minor: best.2, cents: 100.0 * h.tuning, confidence: (lead / 0.3).clamp(0.0, 1.0) })
}

/// The key of `frames` as [`key`] hears it, and where no key stands out, the one its
/// bass chooses among those close to the best: a key and the key a fifth away share six
/// of their seven notes, and the bass plays the tonic. `bass`: the bass alone (a stem),
/// else it's heard in the low end of `frames`. The flag: it's a guess (nothing stood
/// out in the bass either: the best fit, for what it's worth). `None`: drums, noise,
/// silence.
pub fn key_by_bass(frames: &[[f32; 2]], rate: u32, bass: Option<&[[f32; 2]]>) -> Option<(Key, bool)> {
    let h = heard(frames, rate)?;
    let b = match bass {
        Some(stem) => heard_bass(stem, rate, false),
        None => heard_bass(frames, rate, true),
    };
    choose(&h, b.as_ref())
}

/// [`key_by_bass`] of what's heard in the mix and (if there's one) in its bass.
fn choose(h: &Heard, bass: Option<&Heard>) -> Option<(Key, bool)> {
    let fits = fits(&h.chroma);
    let (best, led) = (fits[0], lead(&fits));
    if h.tonal < MIN_TONAL || best.0 < GUESS_FIT {
        return None;
    }
    let key = |f: (f32, u8, bool), confidence: f32| Key { tonic: f.1, minor: f.2, cents: 100.0 * h.tuning, confidence };
    // A key well ahead is the mix's. One only just ahead stands unless the bass says
    // otherwise (the key a fifth up fits a mix nearly as well, sometimes better); with
    // none ahead it's a guess unless the bass settles it.
    let clear = best.0 >= MIN_FIT && led >= MIN_LEAD;
    let guess = Some(if clear { (key(best, (led / 0.3).clamp(0.0, 1.0)), false) } else { (key(best, 0.0), true) });
    if clear && led >= CLOSE {
        return guess;
    }
    let Some(b) = bass else { return guess };
    // The candidates by how well the bass fits each.
    let mut by_bass: Vec<(f32, u8, bool)> =
        fits.iter().filter(|f| f.0 >= best.0 - CLOSE && f.0 >= GUESS_FIT).map(|f| (fit(&b.chroma, f.1 as usize, f.2), f.1, f.2)).collect();
    by_bass.sort_by(|a, b| b.0.total_cmp(&a.0));
    // Its note is the bass's choice (among keys the mix already fits, so how well the
    // bass alone fits a key matters less than which it fits better); major or minor is
    // the mix's: the bass seldom plays the third.
    let chosen = by_bass[0];
    let over = chosen.0 - by_bass.iter().find(|f| f.1 != chosen.1).map_or(0.0, |f| f.0);
    if over < BASS_LEAD {
        return guess;
    }
    let of_mix = fits.iter().find(|f| f.1 == chosen.1).copied().unwrap_or(chosen);
    Some((key(of_mix, (over / 0.3).clamp(0.0, 1.0)), false))
}

/// The Fourier transform of `re` + i `im` in place; their length is a power of two.
pub(crate) fn fft(re: &mut [f32], im: &mut [f32]) {
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

    /// E minor and B minor chords, turn about: the two keys a fifth apart fit alike, so
    /// no key stands out. The bass settles it: the one on E makes it E, the one on B, B
    /// (a stem of its own, or heard under the chords). With no bass it's a guess.
    #[test]
    fn the_bass_settles_a_close_key() {
        use super::key_by_bass;
        // An E minor cadence, then the same in B minor, all above the bass: the two keys
        // fit nearly alike.
        let cadence = |up: u8| -> Vec<Vec<u8>> {
            [[45u8, 57, 60, 64], [50, 57, 62, 65], [52, 56, 59, 64], [45, 57, 60, 64]]
                .iter()
                .map(|c| c.iter().map(|n| n + up).collect())
                .collect()
        };
        for up in [19u8, 31] {
            let both: Vec<Vec<u8>> = cadence(up).into_iter().chain(cadence(up + 7)).collect();
            let both: Vec<&[u8]> = both.iter().map(Vec::as_slice).collect();
            let chord = chords(&both, 1.0);
            // No bass in it: the mix's own answer stands (E minor, only just).
            let (k, guess) = key_by_bass(&chord, SR, None).expect("a key");
            assert_eq!((k.tonic, k.minor, guess), (4, true, false), "{k:?}");
            for (hz, tonic) in [(41.2, 4), (61.74, 11)] {
                let bass = tone(hz, 8.0, SR, 3);
                let (k, guess) = key_by_bass(&chord, SR, Some(&bass)).expect("a key");
                assert_eq!((k.tonic, guess), (tonic, false), "the bass alone on {hz} Hz: {k:?}");
                // The same bass under the chords, and no stem.
                let mix: Vec<[f32; 2]> = chord.iter().zip(&bass).map(|(c, b)| [c[0] * 0.5 + b[0] * 0.5, c[1] * 0.5 + b[1] * 0.5]).collect();
                let (k, guess) = key_by_bass(&mix, SR, None).expect("a key");
                assert_eq!((k.tonic, guess), (tonic, false), "the bass in the mix on {hz} Hz: {k:?}");
            }
        }
        // Two keys a fifth apart heard exactly alike: a guess, till a bass is heard.
        let heard = |chroma: [f32; 12]| super::Heard { chroma, tuning: 0.0, tonal: 1.0 };
        let both = heard(std::array::from_fn(|i| super::MINOR[(i + 8) % 12] + super::MINOR[(i + 1) % 12]));
        let (k, guess) = super::choose(&both, None).expect("a guess");
        assert!(guess && k.confidence == 0.0 && (k.tonic == 4 || k.tonic == 11), "{k:?}");
        let on = |class: usize| heard(std::array::from_fn(|i| f32::from(u8::from(i == class))));
        assert_eq!(super::choose(&both, Some(&on(4))).map(|(k, g)| (k.tonic, g)), Some((4, false)));
        assert_eq!(super::choose(&both, Some(&on(11))).map(|(k, g)| (k.tonic, g)), Some((11, false)));
        // A bass that says nothing either way (every note alike) leaves it a guess.
        assert!(super::choose(&both, Some(&heard([1.0; 12]))).is_some_and(|(_, g)| g));
        // A key that stands out is the mix's, whatever the bass plays.
        let clear = chords(&[&[48, 60, 64, 67], &[53, 60, 65, 69], &[55, 59, 62, 67], &[48, 60, 64, 67]], 1.5);
        let (k, guess) = key_by_bass(&clear, SR, Some(&tone(61.74, 6.0, SR, 3))).expect("a key");
        assert_eq!((k.tonic, k.minor, guess), (0, false, false));
        // And drums have none, guessed or not.
        assert_eq!(key_by_bass(&drums(120.0, 4, SR, 0.5), SR, None), None);
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
