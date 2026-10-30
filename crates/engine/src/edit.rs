//! The Sample Editor's edits (docs/sampling-layout.md §4) and slicing (§5), on decoded
//! samples. Edits never change their input: they return a new sample, which the caller
//! writes to a new file and points the zone at (so undo restores the old one).

use crate::sample::Sample;
use std::path::{Path, PathBuf};

type Frames = Vec<[f32; 2]>;

fn clamp_range(len: usize, (a, b): (usize, usize)) -> (usize, usize) {
    let a = a.min(len);
    (a, b.clamp(a, len))
}

fn with_frames(s: &Sample, frames: Frames) -> Sample {
    Sample { rate: s.rate, frames }
}

/// Keep only `[a, b)`.
pub fn truncate(s: &Sample, r: (usize, usize)) -> Sample {
    let (a, b) = clamp_range(s.frames.len(), r);
    with_frames(s, s.frames[a..b].to_vec())
}

/// Scale `[a, b)` so its peak is 0 dBFS (nothing if it is silent).
pub fn normalize(s: &Sample, r: (usize, usize)) -> Sample {
    let (a, b) = clamp_range(s.frames.len(), r);
    let peak = s.frames[a..b].iter().fold(0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    let mut out = s.frames.clone();
    if peak > 0.0 {
        let g = 1.0 / peak;
        out[a..b].iter_mut().for_each(|f| *f = [f[0] * g, f[1] * g]);
    }
    with_frames(s, out)
}

/// Reverse `[a, b)` in place.
pub fn reverse(s: &Sample, r: (usize, usize)) -> Sample {
    let (a, b) = clamp_range(s.frames.len(), r);
    let mut out = s.frames.clone();
    out[a..b].reverse();
    with_frames(s, out)
}

/// A linear fade over `[a, b)`: in (0 → 1) or out (1 → 0).
pub fn fade(s: &Sample, r: (usize, usize), fade_in: bool) -> Sample {
    let (a, b) = clamp_range(s.frames.len(), r);
    let n = (b - a).max(1) as f32;
    let mut out = s.frames.clone();
    for (i, f) in out[a..b].iter_mut().enumerate() {
        let t = i as f32 / n;
        let g = if fade_in { t } else { 1.0 - t };
        *f = [f[0] * g, f[1] * g];
    }
    with_frames(s, out)
}

/// Remove DC: subtract each channel's mean over `[a, b)`.
pub fn remove_dc(s: &Sample, r: (usize, usize)) -> Sample {
    let (a, b) = clamp_range(s.frames.len(), r);
    let n = (b - a).max(1) as f64;
    let mean = s.frames[a..b].iter().fold([0f64; 2], |m, f| [m[0] + f[0] as f64, m[1] + f[1] as f64]);
    let mean = [(mean[0] / n) as f32, (mean[1] / n) as f32];
    let mut out = s.frames.clone();
    out[a..b].iter_mut().for_each(|f| *f = [f[0] - mean[0], f[1] - mean[1]]);
    with_frames(s, out)
}

/// Zero `[a, b)`.
pub fn silence(s: &Sample, r: (usize, usize)) -> Sample {
    let (a, b) = clamp_range(s.frames.len(), r);
    let mut out = s.frames.clone();
    out[a..b].iter_mut().for_each(|f| *f = [0.0; 2]);
    with_frames(s, out)
}

/// Short linear ramps at the start and end of `frames`, so joins don't click.
fn declick(frames: &mut [[f32; 2]], n: usize) {
    let n = n.min(frames.len() / 2);
    let len = frames.len();
    for i in 0..n {
        let g = i as f32 / n as f32;
        frames[i] = [frames[i][0] * g, frames[i][1] * g];
        frames[len - 1 - i] = [frames[len - 1 - i][0] * g, frames[len - 1 - i][1] * g];
    }
}

/// Copy `[a, b)` (to the clipboard).
pub fn copy(s: &Sample, r: (usize, usize)) -> Sample {
    truncate(s, r)
}

/// Remove `[a, b)`, joining the rest; returns (the sample, what was cut).
pub fn cut(s: &Sample, r: (usize, usize)) -> (Sample, Sample) {
    let (a, b) = clamp_range(s.frames.len(), r);
    let mut out = s.frames[..a].to_vec();
    out.extend_from_slice(&s.frames[b..]);
    (with_frames(s, out), copy(s, (a, b)))
}

/// Insert `clip` at `at` (resampled if its rate differs; mono and stereo mix freely).
pub fn paste(s: &Sample, at: usize, clip: &Sample) -> Sample {
    let at = at.min(s.frames.len());
    let mut ins =
        if clip.rate == s.rate || clip.rate == 0 { clip.frames.clone() } else { resample(&clip.frames, clip.rate as f64 / s.rate as f64) };
    declick(&mut ins, 32);
    let mut out = s.frames[..at].to_vec();
    out.extend(ins);
    out.extend_from_slice(&s.frames[at..]);
    with_frames(s, out)
}

/// Insert a copy of `[a, b)` right after it.
pub fn duplicate(s: &Sample, r: (usize, usize)) -> Sample {
    let (a, b) = clamp_range(s.frames.len(), r);
    let mut out = s.frames[..b].to_vec();
    out.extend_from_slice(&s.frames[a..b]);
    out.extend_from_slice(&s.frames[b..]);
    with_frames(s, out)
}

/// Linear-interpolation resampling by `ratio` (source frames per output frame).
fn resample(x: &[[f32; 2]], ratio: f64) -> Frames {
    if x.is_empty() || ratio <= 0.0 {
        return Vec::new();
    }
    let n = (x.len() as f64 / ratio).round() as usize;
    (0..n)
        .map(|i| {
            let p = i as f64 * ratio;
            let k = p as usize;
            let t = (p - k as f64) as f32;
            let a = x[k.min(x.len() - 1)];
            let b = x[(k + 1).min(x.len() - 1)];
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
        })
        .collect()
}

/// Time-stretch `[a, b)` by `time` (new length / old: 2 = twice as long) and pitch-shift
/// it by `semitones`, with WSOLA (waveform-similarity overlap-add). Maschine uses
/// élastique here, which is proprietary; this keeps the same controls.
pub fn stretch(s: &Sample, r: (usize, usize), time: f64, semitones: f64) -> Sample {
    let (a, b) = clamp_range(s.frames.len(), r);
    let pitch = 2f64.powf(semitones / 12.0);
    // Stretch by time·pitch, then resample by pitch: the length becomes time·len and
    // the pitch moves by `pitch`.
    let stretched = wsola(&s.frames[a..b], (time * pitch).clamp(0.05, 20.0), s.rate.max(1) as f64);
    let mid = if (pitch - 1.0).abs() > 1e-9 { resample(&stretched, pitch) } else { stretched };
    let mut out = s.frames[..a].to_vec();
    out.extend(mid);
    out.extend_from_slice(&s.frames[b..]);
    with_frames(s, out)
}

/// WSOLA: overlap-add windows taken from the input at `1/factor` of the output's pace,
/// each nudged (within a tolerance) to where it best continues the previous one.
fn wsola(x: &[[f32; 2]], factor: f64, sr: f64) -> Frames {
    let win = ((sr * 0.04) as usize).max(64); // 40 ms
    let hop = win / 2;
    let tol = win / 4;
    if x.len() < win * 2 {
        return resample(x, 1.0 / factor);
    }
    let out_len = (x.len() as f64 * factor) as usize;
    let mut out = vec![[0f32; 2]; out_len + win];
    let mut norm = vec![0f32; out_len + win];
    let hann: Vec<f32> = (0..win).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / win as f32).cos()).collect();
    let mono = |i: usize| x.get(i).map_or(0.0, |f| f[0] + f[1]);
    let mut prev_end: Option<usize> = None; // where the last window's continuation is in x
    let mut o = 0;
    while o < out_len {
        let nominal = ((o as f64 / factor) as usize).min(x.len().saturating_sub(win));
        // Best offset within ±tol: most similar to the natural continuation.
        let start = match prev_end {
            Some(cont) => {
                let lo = nominal.saturating_sub(tol);
                let hi = (nominal + tol).min(x.len().saturating_sub(win));
                (lo..=hi)
                    .step_by(2)
                    .max_by(|&p, &q| {
                        let score = |s: usize| (0..hop).step_by(4).map(|k| mono(cont + k) * mono(s + k)).sum::<f32>();
                        score(p).total_cmp(&score(q))
                    })
                    .unwrap_or(nominal)
            }
            None => nominal,
        };
        for k in 0..win {
            if let (Some(f), Some(d)) = (x.get(start + k), out.get_mut(o + k)) {
                d[0] += f[0] * hann[k];
                d[1] += f[1] * hann[k];
                norm[o + k] += hann[k];
            }
        }
        prev_end = Some(start + hop);
        o += hop;
    }
    out.truncate(out_len);
    for (f, n) in out.iter_mut().zip(&norm) {
        if *n > 1e-3 {
            *f = [f[0] / n, f[1] / n];
        }
    }
    out
}

// ---- slicing --------------------------------------------------------------------------

/// Split: `n` equal slices of `[a, b)`; their start frames.
pub fn split_positions((a, b): (usize, usize), n: usize) -> Vec<usize> {
    let len = b.saturating_sub(a);
    (0..n.max(1)).map(|k| a + ((k * len) as f64 / n.max(1) as f64).round() as usize).collect()
}

/// Grid: one slice per `division`th note at `bpm` (4 = quarters).
pub fn grid_positions(rate: u32, (a, b): (usize, usize), bpm: f64, division: u32) -> Vec<usize> {
    let per = rate as f64 * 60.0 * 4.0 / bpm.max(1.0) / division.max(1) as f64;
    let count = ((b.saturating_sub(a)) as f64 / per).round().max(1.0) as usize;
    (0..count).map(|k| a + (k as f64 * per).round() as usize).filter(|&p| p < b).collect()
}

/// Detect: transients of `[a, b)` (energy-flux onsets), strongest first, keeping the
/// strongest `sensitivity` (0..1) share of them; returned in time order, `a` first.
/// `zero_x` moves each to the nearest zero crossing before it.
pub fn detect_positions(s: &Sample, (a, b): (usize, usize), sensitivity: f64, zero_x: bool) -> Vec<usize> {
    let (a, b) = clamp_range(s.frames.len(), (a, b));
    let hop = ((s.rate as f64 * 0.005) as usize).max(32); // 5 ms
    let energy: Vec<f32> = (a..b)
        .step_by(hop)
        .map(|i| s.frames[i..(i + hop).min(b)].iter().map(|f| (f[0] + f[1]) * (f[0] + f[1])).sum::<f32>().sqrt())
        .collect();
    // Rises in energy (log-compressed), locally maximal.
    let le: Vec<f32> = energy.iter().map(|e| (1.0 + 100.0 * e).ln()).collect();
    let mut cands: Vec<(f32, usize)> = Vec::new();
    for k in 1..le.len() {
        let flux = le[k] - le[k - 1];
        let before = if k >= 2 { le[k - 1] - le[k - 2] } else { 0.0 };
        let after = if k + 1 < le.len() { le[k + 1] - le[k] } else { 0.0 };
        if flux > 0.05 && flux >= before && flux >= after {
            cands.push((flux, a + k * hop));
        }
    }
    cands.sort_by(|x, y| y.0.total_cmp(&x.0));
    // Two onsets closer than 50 ms are one.
    let min_gap = (s.rate as f64 * 0.05) as usize;
    let mut kept: Vec<usize> = Vec::new();
    let take = ((cands.len() as f64) * sensitivity.clamp(0.0, 1.0)).round() as usize;
    for &(_, p) in cands.iter().take(take) {
        if kept.iter().all(|&q| p.abs_diff(q) >= min_gap) && p.abs_diff(a) >= min_gap {
            kept.push(p);
        }
    }
    if zero_x {
        for p in &mut kept {
            let mut q = *p;
            while q > a + 1
                && (s.frames[q][0] + s.frames[q][1]).signum() == (s.frames[q - 1][0] + s.frames[q - 1][1]).signum()
                && *p - q < hop
            {
                q -= 1;
            }
            *p = q;
        }
    }
    kept.push(a);
    kept.sort_unstable();
    kept.dedup();
    kept
}

/// Slices (start, end) from start positions within `[a, b)`.
pub fn slices_from(positions: &[usize], b: usize) -> Vec<(usize, usize)> {
    let mut p: Vec<usize> = positions.iter().copied().filter(|&x| x < b).collect();
    p.sort_unstable();
    p.dedup();
    p.iter().enumerate().map(|(i, &s)| (s, p.get(i + 1).copied().unwrap_or(b))).collect()
}

// ---- files ----------------------------------------------------------------------------

/// Write `s` as a 24-bit stereo WAV.
pub fn write_wav(path: &Path, s: &Sample) -> Result<(), String> {
    write_wav_channels(path, s, 2)
}

/// [`write_wav`] with 1 channel (the left) or 2.
pub fn write_wav_channels(path: &Path, s: &Sample, channels: u16) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let channels = channels.clamp(1, 2);
    let spec = hound::WavSpec { channels, sample_rate: s.rate.max(1), bits_per_sample: 24, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| format!("{}: {e}", path.display()))?;
    for f in &s.frames {
        for c in &f[..channels as usize] {
            let v = (c.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
            w.write_sample(v).map_err(|e| e.to_string())?;
        }
    }
    w.finalize().map_err(|e| e.to_string())
}

/// Level below which audio counts as silence (−60 dBFS).
const SILENCE: f32 = 0.001;

/// Without the silence at its start and end (all of it silent: one frame).
pub fn trim_silence(s: &Sample) -> Sample {
    let loud = |f: &[f32; 2]| f[0].abs().max(f[1].abs()) > SILENCE;
    let a = s.frames.iter().position(loud).unwrap_or(0);
    let b = s.frames.iter().rposition(loud).map_or(a + 1, |i| i + 1);
    Sample { rate: s.rate, frames: s.frames[a.min(s.frames.len())..b.max(a + 1).min(s.frames.len())].to_vec() }
}

/// A loop in frames `a..b` (the part where a held note sustains) whose end joins
/// smoothly back to its start: both ends on rising zero crossings, the start chosen
/// where the audio before it best matches the audio before the end. `None` if the
/// region is too short or nothing matches well.
pub fn find_loop(s: &Sample, (a, b): (usize, usize)) -> Option<(usize, usize)> {
    const WINDOW: usize = 256;
    let mono: Vec<f32> = s.frames.iter().map(|f| (f[0] + f[1]) * 0.5).collect();
    let b = b.min(mono.len());
    let rising = |i: usize| i > 0 && i < mono.len() && mono[i - 1] <= 0.0 && mono[i] > 0.0;
    // The end: the last rising zero crossing before `b`.
    let end = (a + WINDOW..b).rev().find(|&i| rising(i))?;
    let min_len = (s.rate as usize / 20).max(WINDOW * 2); // at least 50 ms
    if end < a + min_len + WINDOW {
        return None;
    }
    let tail = &mono[end - WINDOW..end];
    let energy = |w: &[f32]| w.iter().map(|v| v * v).sum::<f32>();
    let e_tail = energy(tail);
    if e_tail < SILENCE * SILENCE * WINDOW as f32 {
        return None;
    }
    let mut best: Option<(f32, usize)> = None;
    // Starts past the attack (the region's first quarter).
    for st in (a + (end - a) / 4).max(a + WINDOW)..end - min_len {
        if !rising(st) {
            continue;
        }
        let w = &mono[st - WINDOW..st];
        let dot: f32 = w.iter().zip(tail).map(|(x, y)| x * y).sum();
        let score = dot / (energy(w) * e_tail).sqrt().max(1e-12);
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, st));
        }
    }
    let (score, start) = best?;
    (score > 0.9).then_some((start, end))
}

/// Where an edited copy of `original` goes: `<dir>/<name> - N.wav` with the first free
/// N (an existing " - N" suffix is replaced), as Maschine names them.
pub fn edited_path(dir: &Path, original: &Path) -> PathBuf {
    let stem = original.file_stem().map_or("Sample".to_string(), |s| s.to_string_lossy().into_owned());
    let base = match stem.rsplit_once(" - ") {
        Some((b, n)) if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() => b.to_string(),
        _ => stem,
    };
    let ext =
        original.extension().and_then(|e| e.to_str()).filter(|e| ["wav", "aif", "aiff", "aifc"].contains(&e.to_ascii_lowercase().as_str()));
    // We always write WAV.
    let _ = ext;
    (2..).map(|n| dir.join(format!("{base} - {n}.wav"))).find(|p| !p.exists()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_silence_and_finds_a_loop() {
        let rate = 48_000;
        // 0.1 s silence, a 220 Hz tone for 1 s, 0.1 s silence.
        let mut frames = vec![[0.0f32; 2]; 4800];
        frames.extend((0..48_000).map(|i| {
            let v = (i as f32 / rate as f32 * 220.0 * std::f32::consts::TAU).sin() * 0.5;
            [v, v]
        }));
        frames.extend(vec![[0.0f32; 2]; 4800]);
        let s = Sample { rate, frames };
        let t = trim_silence(&s);
        assert!((t.frames.len() as i64 - 48_000).abs() < 300, "{}", t.frames.len());
        let (a, b) = find_loop(&t, (0, t.frames.len())).expect("a loop in a steady tone");
        assert!(b > a + 2400);
        // A whole number of periods (218.18 frames each), give or take a frame.
        let periods = (b - a) as f64 / (rate as f64 / 220.0);
        assert!((periods - periods.round()).abs() * (rate as f64 / 220.0) < 1.5, "{periods}");
        assert!(find_loop(&Sample { rate, frames: vec![[0.0; 2]; 48_000] }, (0, 48_000)).is_none());
    }

    fn ramp(n: usize) -> Sample {
        Sample { rate: 44_100, frames: (0..n).map(|i| [i as f32 / n as f32, -(i as f32) / n as f32]).collect() }
    }

    #[test]
    fn edits() {
        let s = ramp(100);
        assert_eq!(truncate(&s, (10, 20)).frames.len(), 10);
        let n = normalize(&s, (0, 50));
        assert!((n.frames[49][0] - 1.0).abs() < 1e-6 && n.frames[60] == s.frames[60]);
        let r = reverse(&s, (0, 100));
        assert_eq!(r.frames[0], s.frames[99]);
        let f = fade(&s, (0, 100), true);
        assert_eq!(f.frames[0], [0.0, 0.0]);
        let (c, clip) = cut(&s, (10, 30));
        assert_eq!((c.frames.len(), clip.frames.len()), (80, 20));
        assert_eq!(c.frames[10], s.frames[30]);
        assert_eq!(paste(&c, 10, &clip).frames.len(), 100);
        assert_eq!(duplicate(&s, (0, 10)).frames.len(), 110);
        let sil = silence(&s, (5, 10));
        assert!(sil.frames[5..10].iter().all(|f| *f == [0.0, 0.0]));
        let dc = remove_dc(&Sample { rate: 1, frames: vec![[0.5, 0.5]; 10] }, (0, 10));
        assert!(dc.frames.iter().all(|f| f[0].abs() < 1e-6));
    }

    #[test]
    fn stretch_changes_length_and_keeps_pitch() {
        // 1 s of 440 Hz.
        let sr = 44_100;
        let s = Sample {
            rate: sr,
            frames: (0..sr as usize).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / sr as f32).sin()).map(|x| [x, x]).collect(),
        };
        let t = stretch(&s, (0, s.frames.len()), 2.0, 0.0);
        assert!((t.frames.len() as f64 / s.frames.len() as f64 - 2.0).abs() < 0.01, "{}", t.frames.len());
        // Zero crossings per second stay about 880 (440 Hz).
        let zc = |x: &[[f32; 2]]| x.windows(2).filter(|w| (w[0][0] >= 0.0) != (w[1][0] >= 0.0)).count() as f64;
        let rate = zc(&t.frames[4410..t.frames.len() - 4410]) / ((t.frames.len() - 8820) as f64 / sr as f64);
        assert!((rate - 880.0).abs() < 40.0, "{rate}");
        let up = stretch(&s, (0, s.frames.len()), 1.0, 12.0);
        assert!((up.frames.len() as f64 / s.frames.len() as f64 - 1.0).abs() < 0.02);
        let rate = zc(&up.frames[4410..up.frames.len() - 4410]) / ((up.frames.len() - 8820) as f64 / sr as f64);
        assert!((rate - 1760.0).abs() < 80.0, "an octave up: {rate}");
    }

    #[test]
    fn slicing() {
        assert_eq!(split_positions((0, 100), 4), [0, 25, 50, 75]);
        // 120 BPM, quarters, 2 s at 1 kHz: 4 slices of 500.
        assert_eq!(grid_positions(1000, (0, 2000), 120.0, 4), [0, 500, 1000, 1500]);
        // Four clicks: Detect finds them.
        let mut s = Sample { rate: 44_100, frames: vec![[0.0; 2]; 44_100] };
        for k in 0..4 {
            for i in 0..2000 {
                let v = (-(i as f32) / 300.0).exp() * if i % 2 == 0 { 0.8 } else { -0.8 };
                s.frames[5000 + k * 10_000 + i] = [v, v];
            }
        }
        let p = detect_positions(&s, (0, 44_100), 1.0, false);
        assert_eq!(p.len(), 5, "the start and four onsets: {p:?}");
        for (k, x) in p[1..].iter().enumerate() {
            assert!(x.abs_diff(5000 + k * 10_000) < 300, "{p:?}");
        }
        assert_eq!(slices_from(&[0, 50], 100), [(0, 50), (50, 100)]);
    }

    #[test]
    fn edited_file_names() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("edit");
        let p = edited_path(&dir, Path::new("/x/Kick - 3.wav"));
        assert_eq!(p.file_name().unwrap(), "Kick - 2.wav");
        write_wav(&p, &ramp(10)).unwrap();
        assert_eq!(edited_path(&dir, Path::new("/x/Kick.aif")).file_name().unwrap(), "Kick - 3.wav");
        let back = crate::sample::Sample::load(&p).unwrap();
        assert_eq!(back.frames.len(), 10);
    }
}
