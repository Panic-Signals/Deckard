//! The places in a recording worth putting on pads: where something starts. Each candidate
//! (a bar or beat line, else a hit) is weighed by how much the sound changes there, how
//! cleanly it begins and how open the passage after it is (neither near silence nor
//! everything at once); the best are taken, apart from each other and unlike each other.

use crate::key::fft;
use crate::mono;

/// A window and the step between windows (23 ms and 12 ms at 44.1 kHz).
const WINDOW: usize = 1024;
const HOP: usize = 512;
/// The bands a window's energy is taken in: from 0, split at 170 Hz and each octave up.
const BANDS: usize = 8;
const FIRST_SPLIT: f32 = 170.0;
/// Passages quieter than this below the loudest don't count (in nepers: 46 dB).
const FLOOR: f32 = 5.3;
/// What a candidate is weighed by: the change there, its attack, how open it is.
const W_CHANGE: f32 = 0.45;
const W_ATTACK: f32 = 0.3;
const W_OPEN: f32 = 0.25;
/// A passage is weighed over this long after its start at most, and at least (seconds).
const SPAN: (f64, f64) = (1.0, 4.0);
/// Without lines to start on, hits this far apart at least are the candidates (seconds).
const HIT_GAP: f64 = 0.5;
/// A start moves to a hit this near it (seconds).
const SNAP: f64 = 0.04;

/// Up to `count` starts (frames, in order) in `range` of `frames`, among `lines` (bar or
/// beat lines in frames; none: among its hits).
pub fn find_samples(frames: &[[f32; 2]], rate: u32, range: (usize, usize), lines: &[usize], count: usize) -> Vec<usize> {
    find_samples_keeping(frames, rate, range, lines, count, &[])
}

/// [`find_samples`] with some starts decided already: the `kept` ones in `range` as they
/// are, and as many found beside them as make `count` in all, apart from them and unlike
/// them.
pub fn find_samples_keeping(
    frames: &[[f32; 2]],
    rate: u32,
    range: (usize, usize),
    lines: &[usize],
    count: usize,
    kept: &[usize],
) -> Vec<usize> {
    let (a, b) = (range.0.min(frames.len()), range.1.min(frames.len()));
    let mut kept: Vec<usize> = kept.iter().copied().filter(|&f| f >= a && f < b).collect();
    kept.sort_unstable();
    kept.dedup();
    if rate == 0 || count <= kept.len() || b < a + 4 * WINDOW {
        return kept;
    }
    let x = mono(&frames[a..b]);
    let hann: Vec<f32> = (0..WINDOW).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WINDOW as f32).cos()).collect();
    // Each window's log energy by band, and its rise over the window before (the flux).
    let split: Vec<usize> = (0..BANDS - 1)
        .map(|k| ((FIRST_SPLIT * 2f32.powi(k as i32) * WINDOW as f32 / rate as f32).ceil() as usize).clamp(1, WINDOW / 2))
        .collect();
    let n = (x.len() - WINDOW) / HOP + 1;
    let mut bands: Vec<[f32; BANDS]> = Vec::with_capacity(n);
    let mut flux = vec![0f32; n];
    let (mut re, mut im) = (vec![0f32; WINDOW], vec![0f32; WINDOW]);
    for k in 0..n {
        let w = &x[k * HOP..k * HOP + WINDOW];
        for i in 0..WINDOW {
            re[i] = w[i] * hann[i];
            im[i] = 0.0;
        }
        fft(&mut re, &mut im);
        let mut e = [0f32; BANDS];
        let mut band = 0;
        for i in 1..WINDOW / 2 {
            while band < BANDS - 1 && i >= split[band] {
                band += 1;
            }
            e[band] += re[i] * re[i] + im[i] * im[i];
        }
        let e = e.map(|v| (v / (WINDOW * WINDOW) as f32 + 1e-10).ln());
        if let Some(prev) = bands.last() {
            flux[k] = e.iter().zip(prev).map(|(now, was)| (now - was).max(0.0)).sum();
        }
        bands.push(e);
    }
    // What counts as a hit: a local peak of the flux, well over what's usual.
    let mut sorted = flux.clone();
    sorted.sort_by(f32::total_cmp);
    let strong = sorted[(n * 9 / 10).min(n - 1)].max(1e-3);
    let hit = |k: usize| flux[k] >= strong && flux[k] >= flux[k.saturating_sub(1)] && flux[k] >= flux[(k + 1).min(n - 1)];
    let window_of = |f: usize| (f.saturating_sub(a) / HOP).min(n - 1);
    // The candidates, as windows.
    let mut cands: Vec<usize> = lines.iter().filter(|&&f| f >= a && f < b).map(|&f| window_of(f)).collect();
    if cands.is_empty() {
        let gap = (HIT_GAP * f64::from(rate) / HOP as f64) as usize;
        let mut last: Option<usize> = None;
        for k in 0..n {
            if hit(k) && last.is_none_or(|l| k - l >= gap) {
                cands.push(k);
                last = Some(k);
            }
        }
    }
    // (The kept ones are candidates too: what's found is weighed against them.)
    let held: Vec<usize> = kept.iter().map(|&f| window_of(f)).collect();
    cands.extend(&held);
    cands.sort_unstable();
    cands.dedup();
    if cands.is_empty() {
        return kept;
    }
    // Each candidate's passage: the band levels after it and before it.
    let per = |seconds: f64| (seconds * f64::from(rate) / HOP as f64) as usize;
    let mean = |from: usize, to: usize| -> [f32; BANDS] {
        let (from, to) = (from.min(n), to.min(n));
        let mut m = [0f32; BANDS];
        if to > from {
            for e in &bands[from..to] {
                for (s, v) in m.iter_mut().zip(e) {
                    *s += v;
                }
            }
            m = m.map(|v| v / (to - from) as f32);
        }
        m
    };
    let level = |m: &[f32; BANDS]| m.iter().sum::<f32>() / BANDS as f32;
    let apart = |p: &[f32; BANDS], q: &[f32; BANDS]| p.iter().zip(q).map(|(u, v)| (u - v) * (u - v)).sum::<f32>().sqrt();
    struct Cand {
        at: usize,
        after: [f32; BANDS],
        level: f32,
        change: f32,
        attack: f32,
    }
    let mut of: Vec<Cand> = Vec::new();
    for (i, &k) in cands.iter().enumerate() {
        let next = cands.get(i + 1).copied().unwrap_or(n);
        let len = (next - k).clamp(per(SPAN.0).max(1), per(SPAN.1).max(1));
        // (A window in: the hit itself is in both otherwise.)
        let after = mean(k + 1, k + 1 + len);
        let before = if k > 1 { mean(k.saturating_sub(len), k - 1) } else { [(1e-10f32).ln(); BANDS] };
        let near = per(SNAP).max(1);
        let attack = (k.saturating_sub(near)..=(k + near).min(n - 1)).map(|j| flux[j]).fold(0.0, f32::max);
        of.push(Cand { at: k, level: level(&after), change: apart(&after, &before), attack: (attack / strong).min(2.0) / 2.0, after });
    }
    let top = of.iter().map(|c| c.level).fold(f32::MIN, f32::max);
    of.retain(|c| c.level > top - FLOOR || held.contains(&c.at));
    if of.is_empty() {
        return kept;
    }
    // How open: the middle of the levels heard is best, the ends (thin, or everything) least.
    let mut levels: Vec<f32> = of.iter().map(|c| c.level).collect();
    levels.sort_by(f32::total_cmp);
    let rank = |l: f32| levels.partition_point(|v| *v < l) as f32 / levels.len().max(1) as f32;
    let most = of.iter().map(|c| c.change).fold(1e-6, f32::max);
    let scores: Vec<f32> =
        of.iter().map(|c| W_CHANGE * c.change / most + W_ATTACK * c.attack + W_OPEN * (1.0 - 2.0 * (rank(c.level) - 0.5).abs())).collect();
    // The best first, each later one worth less the more it sounds like one taken, and
    // none within a passage's length of one taken (while others are left).
    let gap = per(SPAN.0).max(1);
    let mut taken: Vec<usize> = (0..of.len()).filter(|&i| held.contains(&of[i].at)).collect();
    let had = taken.len();
    // (Kept ones sharing a window are one candidate, and each still a start.)
    let want = (count - kept.len() + had).min(of.len());
    for spaced in [true, false] {
        while taken.len() < want {
            let best = (0..of.len())
                .filter(|i| !taken.contains(i))
                .filter(|&i| !spaced || taken.iter().all(|&t| of[i].at.abs_diff(of[t].at) >= gap))
                .map(|i| {
                    let like = taken.iter().map(|&t| (-apart(&of[i].after, &of[t].after)).exp()).fold(0.0, f32::max);
                    (scores[i] * (1.0 - 0.5 * like), i)
                })
                .max_by(|p, q| p.0.total_cmp(&q.0));
            match best {
                Some((_, i)) => taken.push(i),
                None => break,
            }
        }
    }
    // Each on the hit nearest it, if one is near, and then on where that hit begins.
    let near = per(SNAP).max(1);
    let mut starts: Vec<usize> = taken[had..]
        .iter()
        .map(|&i| {
            let k = of[i].at;
            let on = (k.saturating_sub(near)..=(k + near).min(n - 1)).filter(|&j| hit(j)).min_by_key(|j| j.abs_diff(k));
            match on {
                Some(j) => a + onset(&x, j * HOP),
                None => a + k * HOP,
            }
        })
        .collect();
    starts.extend(&kept);
    starts.sort_unstable();
    starts.dedup();
    starts
}

/// Where the hit whose rise shows in the window at `at` begins: the steepest rise of the
/// level's change, by blocks of 64 samples, in the window and the step before it.
fn onset(x: &[f32], at: usize) -> usize {
    const BLOCK: usize = 64;
    let (from, to) = (at.saturating_sub(HOP), (at + WINDOW).min(x.len()));
    // (Of the sample-to-sample change: a bass note's own swing isn't a rise.)
    let level =
        |i: usize| (x[i.max(1) - 1..(i + BLOCK).min(x.len())].windows(2).map(|s| (s[1] - s[0]) * (s[1] - s[0])).sum::<f32>() + 1e-9).ln();
    let mut best = (0f32, at);
    let mut i = from + BLOCK;
    while i + BLOCK <= to {
        let rise = level(i) - level(i - BLOCK);
        if rise > best.0 {
            best = (rise, i);
        }
        i += BLOCK;
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::find_samples;
    use crate::test_signals::{drums, tone};

    const SR: u32 = 44_100;

    /// 24 bars of drums at 120: a bass tone comes in at bar 8, a high one replaces it at
    /// bar 16, and bars 20 and 21 are silent.
    fn song() -> (Vec<[f32; 2]>, usize) {
        let bar = 2 * SR as usize;
        let mut f = drums(120.0, 24, SR, 0.5);
        for (from, to, hz) in [(8, 16, 82.41), (16, 20, 659.25)] {
            for (m, t) in f[from * bar..to * bar].iter_mut().zip(tone(hz, (to - from) as f64 * 2.0, SR, 4)) {
                m[0] += t[0];
                m[1] += t[1];
            }
        }
        for m in &mut f[20 * bar..22 * bar] {
            *m = [0.0; 2];
        }
        (f, bar)
    }

    #[test]
    fn the_places_things_start() {
        let (f, bar) = song();
        let lines: Vec<usize> = (0..24).map(|k| k * bar).collect();
        let found = find_samples(&f, SR, (0, f.len()), &lines, 4);
        assert_eq!(found.len(), 4, "{found:?}");
        let near = |want: usize| found.iter().any(|&s| s.abs_diff(want) < SR as usize / 50);
        assert!(near(8 * bar) && near(16 * bar), "where the bass and the lead come in: {found:?}");
        assert!(found.iter().all(|&s| !(20 * bar..22 * bar - SR as usize / 10).contains(&s)), "none in the silence: {found:?}");
        assert!(found.windows(2).all(|w| w[0] < w[1]), "in order");
    }

    #[test]
    fn as_many_as_asked_for_apart() {
        let (f, bar) = song();
        let lines: Vec<usize> = (0..96).map(|k| k * bar / 4).collect();
        let found = find_samples(&f, SR, (0, f.len()), &lines, 16);
        assert_eq!(found.len(), 16);
        assert!(found.windows(2).all(|w| w[1] - w[0] > SR as usize * 9 / 10), "a second apart: {found:?}");
        // Asked for more than there are lines: each line once.
        let few: Vec<usize> = (0..4).map(|k| k * bar).collect();
        assert_eq!(find_samples(&f, SR, (0, 4 * bar), &few, 16).len(), 4);
    }

    #[test]
    fn without_lines_it_starts_on_hits() {
        let (f, _) = song();
        let found = find_samples(&f, SR, (0, f.len()), &[], 8);
        assert_eq!(found.len(), 8);
        // Every start is on a drum hit: an 8th of 120 BPM is 11,025 frames.
        assert!(found.iter().all(|&s| (s + 300) % 11_025 < 900), "{:?}", found.iter().map(|s| s % 11_025).collect::<Vec<_>>());
    }

    #[test]
    fn a_range_and_nothing_to_find() {
        let (f, bar) = song();
        let lines: Vec<usize> = (0..24).map(|k| k * bar).collect();
        let found = find_samples(&f, SR, (4 * bar, 12 * bar), &lines, 3);
        assert!(found.iter().all(|&s| s >= 4 * bar - 600 && s < 12 * bar), "{found:?}");
        assert!(found.iter().any(|&s| s.abs_diff(8 * bar) < 900));
        assert!(find_samples(&vec![[0.0; 2]; 10 * SR as usize], SR, (0, 10 * SR as usize), &[], 8).is_empty());
        assert!(find_samples(&f, SR, (0, 100), &[], 8).is_empty());
        assert!(find_samples(&f, 0, (0, f.len()), &[], 8).is_empty());
    }

    #[test]
    fn kept_starts_stay_and_the_rest_are_found() {
        use super::find_samples_keeping;
        let (f, bar) = song();
        let lines: Vec<usize> = (0..24).map(|k| k * bar).collect();
        let kept = [3 * bar + 100];
        let found = find_samples_keeping(&f, SR, (0, f.len()), &lines, 4, &kept);
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(found.contains(&kept[0]), "kept where it was: {found:?}");
        assert!(found.iter().all(|&s| s == kept[0] || s.abs_diff(kept[0]) > SR as usize), "the rest apart from it: {found:?}");
        assert!(found.windows(2).all(|w| w[0] < w[1]), "in order");
        // One kept on a line: there once.
        let found = find_samples_keeping(&f, SR, (0, f.len()), &lines, 4, &[8 * bar]);
        assert_eq!(found.len(), 4, "{found:?}");
        assert_eq!(found.iter().filter(|&&s| s.abs_diff(8 * bar) < SR as usize).count(), 1, "{found:?}");
        // Outside the range: not kept. More kept than asked for: those.
        let found = find_samples_keeping(&f, SR, (4 * bar, 12 * bar), &lines, 3, &[bar + 7, 5 * bar + 7]);
        assert!(found.contains(&(5 * bar + 7)) && !found.contains(&(bar + 7)), "{found:?}");
        assert_eq!(found.len(), 3);
        let many: Vec<usize> = (1..6).map(|k| k * bar + 50).collect();
        assert_eq!(find_samples_keeping(&f, SR, (0, f.len()), &lines, 4, &many), many);
        // Nothing to find: the kept ones still.
        assert_eq!(find_samples_keeping(&f, SR, (0, 100), &[], 8, &[40]), [40]);
    }
}
