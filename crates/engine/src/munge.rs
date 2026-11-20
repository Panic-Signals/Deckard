//! Munge (sampler roadmap, phase 3 part 3): a loop cut into eighth notes and built again
//! with another swing, with eighths taken out, on an even grid. Parts made shorter are
//! cut and faded; parts made longer are filled by repeating their ends.

use crate::sample::Sample;

type Frames = Vec<[f32; 2]>;

/// What MUNGE does to a loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Munge {
    /// −1..=1: how much later (or earlier) each second eighth or sixteenth comes; 1 is a
    /// triplet.
    pub swing: f64,
    /// Swing moves the sixteenths (else the eighths).
    pub sixteenths: bool,
    /// The eighths of a bar that are taken out.
    pub mutes: [bool; 8],
    /// The fade where a part is cut or joined, ms.
    pub grunge_ms: f64,
    /// How much of a part's end is repeated to make it longer, ms.
    pub comb_ms: f64,
}

pub const GRUNGE_MS: (f64, f64) = (1.0, 30.0);
pub const COMB_MS: (f64, f64) = (2.0, 100.0);

impl Default for Munge {
    fn default() -> Munge {
        Munge { swing: 0.0, sixteenths: true, mutes: [false; 8], grunge_ms: 5.0, comb_ms: 30.0 }
    }
}

impl Munge {
    /// Whether it changes anything (the splice settings alone don't).
    pub fn changes(&self) -> bool {
        self.swing != 0.0 || self.mutes.iter().any(|m| *m)
    }
}

/// `src` made `target` frames long: cut and faded out over `grunge` frames, or filled by
/// repeating its last `comb` frames (their joins crossfaded over `grunge`), dying away.
fn fit(src: &[[f32; 2]], target: usize, grunge: usize, comb: usize) -> Frames {
    let len = src.len();
    let mut out = src[..target.min(len)].to_vec();
    if target < len {
        let g = grunge.min(target / 2);
        for i in 0..g {
            let w = i as f32 / g as f32;
            let f = &mut out[target - 1 - i];
            *f = [f[0] * w, f[1] * w];
        }
    } else if target > len && len > 0 {
        let c = comb.clamp(1, (len / 2).max(1));
        let x = grunge.min(c / 2).min(len - c);
        // The end of what repeats fades into the audio before it, so each time round
        // joins on.
        for i in 0..x {
            let w = (i + 1) as f32 / x as f32;
            let (at, pre) = (len - x + i, len - c - x + i);
            out[at] = [out[at][0] * (1.0 - w) + src[pre][0] * w, out[at][1] * (1.0 - w) + src[pre][1] * w];
        }
        let fill = target - len;
        for i in 0..fill {
            let g = 1.0 - (i + 1) as f32 / fill as f32;
            let f = out[len - c + i % c];
            out.push([f[0] * g, f[1] * g]);
        }
    }
    out
}

/// `[a, b)` of `s` munged. `cuts`: where its eighth notes start (frames, rising; those
/// outside the range are passed over, and the range's start is always one); `eighth`:
/// the length an eighth gets, frames. The rest of the sample is kept. `None`: nothing
/// would be left of the range.
pub fn munge(s: &Sample, r: (usize, usize), cuts: &[usize], eighth: f64, m: &Munge) -> Option<Sample> {
    if !m.changes() {
        return Some(s.clone());
    }
    let len = s.frames.len();
    let a = r.0.min(len);
    let b = r.1.clamp(a, len);
    if b <= a || !(eighth.is_finite() && eighth >= 2.0) {
        return None;
    }
    let mut starts = vec![a];
    for &c in cuts {
        if c > *starts.last().unwrap() && c < b {
            starts.push(c);
        }
    }
    // The pieces left, in order: (from, to, whether it keeps its length, whether what
    // came before it was taken out).
    let mut pieces: Vec<(usize, usize, bool, bool)> = Vec::new();
    let mut gap = false;
    for (k, &from) in starts.iter().enumerate() {
        let to = starts.get(k + 1).copied().unwrap_or(b);
        if m.mutes[k % 8] {
            gap = true;
            continue;
        }
        // (A last part that isn't a whole eighth stays as long as it is.)
        let odd_end = k + 1 == starts.len() && ((to - from) as f64) < 0.75 * eighth;
        if m.sixteenths && !odd_end {
            let mid = from + (to - from) / 2;
            pieces.push((from, mid, false, gap));
            pieces.push((mid, to, false, false));
        } else {
            pieces.push((from, to, odd_end, gap));
        }
        gap = false;
    }
    if pieces.is_empty() {
        return None;
    }
    let ms =
        |v: f64, (lo, hi): (f64, f64)| ((if v.is_finite() { v } else { lo }).clamp(lo, hi) / 1000.0 * f64::from(s.rate)).round() as usize;
    let (grunge, comb) = (ms(m.grunge_ms, GRUNGE_MS).max(1), ms(m.comb_ms, COMB_MS).max(1));
    // (Short: a hit starts there.)
    let rise = (s.rate as usize / 3000).max(1);
    let swing = if m.swing.is_finite() { m.swing.clamp(-1.0, 1.0) } else { 0.0 } / 3.0;
    let unit = if m.sixteenths { eighth / 2.0 } else { eighth };
    let mut out: Frames = s.frames[..a].to_vec();
    let mut at = 0f64;
    let mut changed = false;
    for (j, &(from, to, as_is, after_gap)) in pieces.iter().enumerate() {
        let src = &s.frames[from..to];
        let target = if as_is {
            src.len()
        } else {
            let end = at + unit * if j % 2 == 0 { 1.0 + swing } else { 1.0 - swing };
            let n = (end.round() - at.round()).max(1.0) as usize;
            at = end;
            n
        };
        let mut piece = fit(src, target, grunge, comb);
        // After a part that was cut, filled or taken out, it comes in over a third of a
        // millisecond.
        if changed || after_gap {
            let n = rise.min(piece.len() / 2);
            for (i, f) in piece.iter_mut().take(n).enumerate() {
                let w = i as f32 / n as f32;
                *f = [f[0] * w, f[1] * w];
            }
        }
        changed = target != src.len();
        out.extend(piece);
    }
    out.extend_from_slice(&s.frames[b..]);
    Some(Sample { rate: s.rate, frames: out })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EIGHTH: usize = 6000;

    /// A bar at 48 kHz: a click on each eighth (louder each time), silence between.
    fn bar() -> Sample {
        let mut frames = vec![[0.0f32; 2]; 8 * EIGHTH];
        for k in 0..8 {
            for i in 0..200 {
                let v = (0.3 + 0.08 * k as f32) * (1.0 - i as f32 / 200.0) * if i % 2 == 0 { 1.0 } else { -1.0 };
                frames[k * EIGHTH + i] = [v, v];
            }
        }
        Sample { rate: 48_000, frames }
    }

    fn cuts() -> Vec<usize> {
        (0..8).map(|k| k * EIGHTH).collect()
    }

    /// Where the clicks start: (frame, level).
    fn clicks(s: &Sample) -> Vec<(usize, f32)> {
        let mut out: Vec<(usize, f32)> = Vec::new();
        let mut i = 0;
        while i < s.frames.len() {
            if s.frames[i][0].abs() > 0.02 {
                let peak = s.frames[i..(i + 60).min(s.frames.len())].iter().fold(0f32, |m, f| m.max(f[0].abs()));
                out.push((i, peak));
                i += 1000;
            } else {
                i += 1;
            }
        }
        out
    }

    #[test]
    fn nothing_changed_is_the_input() {
        let s = bar();
        let m = Munge { grunge_ms: 30.0, comb_ms: 2.0, ..Munge::default() };
        assert!(!m.changes());
        assert_eq!(munge(&s, (0, s.frames.len()), &cuts(), EIGHTH as f64, &m).unwrap().frames, s.frames);
    }

    #[test]
    fn swing_moves_the_off_beat() {
        let s = bar();
        let all = (0, s.frames.len());
        let swung = munge(&s, all, &cuts(), EIGHTH as f64, &Munge { swing: 1.0, sixteenths: false, ..Munge::default() }).unwrap();
        assert_eq!(swung.frames.len(), s.frames.len(), "as long as it was");
        let at: Vec<usize> = clicks(&swung).iter().map(|c| c.0).collect();
        assert_eq!(at.len(), 8, "{at:?}");
        for (k, x) in at.iter().enumerate() {
            let want = (k / 2) * 2 * EIGHTH + (k % 2) * (EIGHTH * 4 / 3);
            assert!(x.abs_diff(want) < 60, "click {k} at {x}, not {want}");
        }
        // The other way takes it out again.
        let straight = munge(&swung, all, &cuts(), EIGHTH as f64, &Munge { swing: -1.0, sixteenths: false, ..Munge::default() }).unwrap();
        let at: Vec<usize> = clicks(&straight).iter().map(|c| c.0).collect();
        assert_eq!(at.len(), 8, "{at:?}");
        for (k, x) in at.iter().enumerate() {
            assert!(x.abs_diff(k * EIGHTH) < 60, "click {k} at {x}");
        }
        // On sixteenths the eighths stay where they are.
        let sixteenths = munge(&s, all, &cuts(), EIGHTH as f64, &Munge { swing: 0.5, ..Munge::default() }).unwrap();
        let at: Vec<usize> = clicks(&sixteenths).iter().map(|c| c.0).collect();
        assert_eq!(at.len(), 8, "{at:?}");
        for (k, x) in at.iter().enumerate() {
            assert!(x.abs_diff(k * EIGHTH) < 60, "click {k} at {x}");
        }
    }

    #[test]
    fn a_muted_eighth_shortens_the_bar() {
        let s = bar();
        let mut m = Munge::default();
        m.mutes[3] = true;
        assert!(m.changes());
        let out = munge(&s, (0, s.frames.len()), &cuts(), EIGHTH as f64, &m).unwrap();
        assert_eq!(out.frames.len(), 7 * EIGHTH, "seven eighths");
        let got = clicks(&out);
        assert_eq!(got.len(), 7, "{got:?}");
        for (k, (x, level)) in got.iter().enumerate() {
            assert!(x.abs_diff(k * EIGHTH) < 60, "click {k} at {x}");
            let from = if k < 3 { k } else { k + 1 };
            assert!((level - (0.3 + 0.08 * from as f32)).abs() < 0.06, "click {k} is the old {from}: {level}");
        }
        assert!(munge(&s, (0, s.frames.len()), &cuts(), EIGHTH as f64, &Munge { mutes: [true; 8], ..m }).is_none(), "nothing left");
        assert!(munge(&s, (0, 0), &cuts(), EIGHTH as f64, &m).is_none());
        assert!(munge(&s, (0, s.frames.len()), &cuts(), f64::NAN, &m).is_none());
    }

    /// A part made longer goes on with its own end, no louder than it was, down to nothing.
    #[test]
    fn a_longer_piece_is_filled_and_dies_away() {
        let frames: Frames = (0..2 * EIGHTH).map(|i| [(i as f32 * 0.05).sin() * 0.5; 2]).collect();
        let s = Sample { rate: 48_000, frames };
        let m = Munge { swing: 1.0, sixteenths: false, comb_ms: 10.0, ..Munge::default() };
        let out = munge(&s, (0, 2 * EIGHTH), &[0, EIGHTH], EIGHTH as f64, &m).unwrap();
        assert_eq!(out.frames.len(), 2 * EIGHTH);
        let first = &out.frames[..8000];
        assert!(first.iter().all(|f| f[0].abs() <= 0.501), "no louder");
        assert!(first[6500][0].abs() + first[6510][0].abs() + first[6520][0].abs() > 0.05, "the fill sounds");
        assert!(first[7990..].iter().all(|f| f[0].abs() < 0.01), "and dies away");
        // No jump where the repeats join.
        let jump = first.windows(2).map(|w| (w[1][0] - w[0][0]).abs()).fold(0f32, f32::max);
        assert!(jump < 0.06, "{jump}");
        // The second half was cut to 4000 frames, faded at its end.
        assert!(out.frames[2 * EIGHTH - 1][0].abs() < 0.01);
    }

    #[test]
    fn outside_the_range_is_kept() {
        let s = bar();
        let r = (EIGHTH + 500, 5 * EIGHTH + 500);
        let mut m = Munge { sixteenths: false, ..Munge::default() };
        m.mutes[0] = true;
        let cuts: Vec<usize> = (1..5).map(|k| k * EIGHTH + 500).collect();
        let out = munge(&s, r, &cuts, EIGHTH as f64, &m).unwrap();
        assert_eq!(out.frames.len(), s.frames.len() - EIGHTH);
        assert_eq!(out.frames[..r.0], s.frames[..r.0]);
        assert_eq!(out.frames[r.1 - EIGHTH..], s.frames[r.1..]);
        // A last part that is no whole eighth stays as long as it is.
        let short = munge(&s, (0, 2 * EIGHTH + 1000), &[0, EIGHTH, 2 * EIGHTH], EIGHTH as f64, &m).unwrap();
        assert_eq!(short.frames.len(), s.frames.len() - EIGHTH);
    }
}
