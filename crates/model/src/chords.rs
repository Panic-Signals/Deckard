//! What a pad plays in Chords mode: the Harmonizer's chords, the chord sets and the user
//! chords, at a fixed position or Auto (docs/scales.md, "Chord voicings"). A scale is its
//! place in the tables: bank * 15 + its place in the bank.

use crate::scale_tables as t;

/// The settings a chord is made from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chords<'a> {
    /// 1 Harmonizer, 2 Chord Set, 3 User Chord (`ChordMode`).
    pub mode: i64,
    pub scale: usize,
    /// The root note (any octave; only its pitch class counts).
    pub root: i64,
    /// The chord type in the list for the mode (from 0): the Harmonizer's (the chromatic
    /// list in the chromatic scale, else the scale list), the chord set.
    pub kind: usize,
    /// The Harmonizer's chromatic chord type, whatever the mode (Auto checks it).
    pub chromatic: usize,
    /// The place in `CHORD_POSITIONS` (0..=18; 9 Root, 18 Auto).
    pub position: usize,
    /// User Chord mode's set: 12 chords, semitones above the root.
    pub user: &'a [Vec<i64>],
}

/// Auto's state: the anchor chord the others lead towards, the last note played, and
/// the settings they belong to (a change clears them).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Auto {
    pub anchor: Vec<i64>,
    pub last: i64,
    pub of: String,
}

/// Auto gives up leading after this many steps.
const MAX_STEPS: usize = 64;

/// The last position entry: Auto.
const AUTO: usize = 18;

/// Chromatic Harmonizer types too wide for Auto: Dominant 9 and Quartal.
const WIDE: [usize; 2] = [9, 13];

impl Chords<'_> {
    fn key(&self) -> i64 {
        self.root.rem_euclid(12)
    }

    fn chromatic_scale(&self) -> bool {
        self.scale == 0
    }

    /// The pattern for note `n`, its octave and how many entries (a chord set's bass)
    /// positions leave alone.
    fn pattern(&self, n: i64) -> (Vec<i64>, i64, usize) {
        let pad = (n - self.key()).rem_euclid(12) as usize;
        let ints = |r: &[i8]| r.iter().map(|&x| x as i64).collect::<Vec<_>>();
        match self.mode {
            1 if self.chromatic_scale() => (t::HARMONIZER_CHROMATIC.get(self.kind).map_or_else(Vec::new, |r| ints(r)), 12, 0),
            1 => {
                let size = scale_size(self.scale);
                let p = match self.kind {
                    0 => vec![0, size],
                    k => t::HARMONIZER_SCALE.get(k - 1).map_or_else(Vec::new, |r| ints(r)),
                };
                (p, size, 0)
            }
            2 => (t::CHORD_SET_CHORDS.get(self.kind).map_or_else(Vec::new, |s| ints(s[pad])), 12, 1),
            3 => (self.user.get(pad).cloned().unwrap_or_default(), 12, 0),
            _ => (Vec::new(), 12, 0),
        }
    }

    /// Pattern entry `x` as a note, for note `n`.
    fn note(&self, n: i64, x: i64) -> i64 {
        match self.mode {
            1 if !self.chromatic_scale() => n + scale_step(self.scale, self.key(), n, x),
            3 => n - (n - self.key()).rem_euclid(12) + x,
            _ => n + x,
        }
    }

    fn notes(&self, n: i64, pattern: &[i64]) -> Vec<i64> {
        pattern.iter().map(|&x| self.note(n, x)).collect()
    }

    /// What the engine gets for note `n`: the Harmonizer moves it onto the scale.
    fn input(&self, n: i64) -> i64 {
        if self.mode == 1 { n + t::SNAP[self.scale][(n - self.key()).rem_euclid(12) as usize] as i64 } else { n }
    }

    /// The chord for note `n` at position `p` (Root 0), notes outside MIDI dropped.
    pub fn fixed(&self, n: i64, p: i64) -> Vec<i64> {
        let n = self.input(n);
        let (mut pattern, octave, skip) = self.pattern(n);
        if pattern.len() > skip {
            shift(&mut pattern[skip..], p, octave);
        }
        midi(self.notes(n, &pattern))
    }

    /// Settings Auto's state belongs to.
    fn fingerprint(&self) -> String {
        format!("{}/{}/{}/{}/{}/{}", self.mode, self.scale, self.key(), self.kind, self.chromatic, self.position)
    }

    /// The chord a hit sending note `n` plays, moving Auto's state on.
    pub fn play(&self, n: i64, auto: &mut Auto) -> Vec<i64> {
        if !(0..=127).contains(&n) || !(1..=3).contains(&self.mode) {
            return Vec::new();
        }
        let auto_on = self.position == AUTO && !(self.chromatic_scale() && WIDE.contains(&self.chromatic));
        if !auto_on {
            let p = if self.position == AUTO { 0 } else { self.position as i64 - 9 };
            return self.fixed(n, p);
        }
        let fp = self.fingerprint();
        if auto.of != fp {
            *auto = Auto { of: fp, ..Auto::default() };
        }
        let n = self.input(n);
        let (mut pattern, octave, skip) = self.pattern(n);
        let mut chord = self.notes(n, &pattern);
        let jump = (n - auto.last).abs() >= 12;
        auto.last = n;
        if jump || auto.anchor.is_empty() {
            auto.anchor = chord.clone();
            return midi(chord);
        }
        if pattern.len() < 3 {
            return midi(chord);
        }
        let target = mean(&auto.anchor[skip.min(auto.anchor.len())..]);
        let step = if mean(&chord[skip..]) <= target { 1 } else { -1 };
        let mut dist = (mean(&chord[skip..]) - target).abs();
        for _ in 0..MAX_STEPS {
            if dist < 1.0 {
                break;
            }
            let best = chord.clone();
            shift(&mut pattern[skip..], step, octave);
            chord = self.notes(n, &pattern);
            let next = (mean(&chord[skip..]) - target).abs();
            if next > dist {
                return midi(best);
            }
            dist = next;
        }
        midi(chord)
    }
}

/// Move `pattern` to position `p` (`ChordPosition`), with `octave` steps to an octave.
pub fn shift(pattern: &mut [i64], p: i64, octave: i64) {
    if p == 0 || pattern.len() < 2 {
        return;
    }
    let span = pattern[pattern.len() - 1] - pattern[0];
    if span > octave {
        return;
    }
    // A top doubling the bottom is set aside and put back an octave over the new bottom.
    let doubled = span == octave;
    let body = if doubled { pattern.len() - 1 } else { pattern.len() };
    let m = body as i64;
    pattern[..body].rotate_left(p.rem_euclid(m) as usize);
    for (j, x) in pattern[..body].iter_mut().enumerate() {
        *x += (p + j as i64).div_euclid(m) * octave;
    }
    if doubled {
        pattern[body] = pattern[0] + octave;
    }
}

/// How many notes scale `scale` has in an octave.
pub fn scale_size(scale: usize) -> i64 {
    t::SNAP[scale].iter().filter(|&&x| x == 0).count() as i64
}

/// Semitones from `n` to the note `step` scale notes away, in `scale` played from `key`
/// (0 when `n` is off the scale).
fn scale_step(scale: usize, key: i64, n: i64, step: i64) -> i64 {
    if step == 0 {
        return 0;
    }
    // The scale's pitch classes, lowest first.
    let mut pcs: Vec<i64> = (0..12).filter(|&k| t::SNAP[scale][k as usize] == 0).map(|k| (k + key) % 12).collect();
    pcs.sort_unstable();
    let Some(i) = pcs.iter().position(|&pc| pc == n.rem_euclid(12)) else { return 0 };
    let size = pcs.len() as i64;
    let to = i as i64 + step;
    pcs[to.rem_euclid(size) as usize] - pcs[i] + 12 * to.div_euclid(size)
}

fn mean(xs: &[i64]) -> f64 {
    if xs.is_empty() { 0.0 } else { xs.iter().sum::<i64>() as f64 / xs.len() as f64 }
}

fn midi(notes: Vec<i64>) -> Vec<i64> {
    notes.into_iter().filter(|n| (0..=127).contains(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Main bank: 0 Chromatic, 1 Major.
    const CHROMATIC: usize = 0;
    const MAJOR: usize = 1;
    const ROOT: usize = 9;
    const AUTO_POS: usize = 18;

    fn harmonizer(scale: usize, kind: usize, position: usize) -> Chords<'static> {
        Chords { mode: 1, scale, root: 60, kind, chromatic: 3, position, user: &[] }
    }

    fn play(c: &Chords, n: i64) -> Vec<i64> {
        c.play(n, &mut Auto::default())
    }

    #[test]
    fn the_harmonizer_stacks_the_scale() {
        // C major, 1-3-5: C, Dm, Bdim; Octave.
        let c = harmonizer(MAJOR, 3, ROOT);
        assert_eq!(play(&c, 60), [60, 64, 67]);
        assert_eq!(play(&c, 62), [62, 65, 69]);
        assert_eq!(play(&c, 71), [71, 74, 77]);
        assert_eq!(play(&harmonizer(MAJOR, 0, ROOT), 64), [64, 76]);
        // 1-3-5-7 on A: Am7.
        assert_eq!(play(&harmonizer(MAJOR, 5, ROOT), 69), [69, 72, 76, 79]);
    }

    #[test]
    fn the_chromatic_harmonizer_adds_semitones() {
        assert_eq!(play(&harmonizer(CHROMATIC, 3, ROOT), 61), [61, 65, 68]);
        assert_eq!(play(&harmonizer(CHROMATIC, 9, ROOT), 60), [60, 64, 67, 70, 74]);
        assert_eq!(play(&harmonizer(CHROMATIC, 0, ROOT), 60), [60, 72]);
    }

    #[test]
    fn positions_invert_the_chord() {
        // C major 1-3-5 on C: first inversion, then down one.
        assert_eq!(play(&harmonizer(MAJOR, 3, ROOT + 1), 60), [64, 67, 72]);
        assert_eq!(play(&harmonizer(MAJOR, 3, ROOT - 1), 60), [55, 60, 64]);
        assert_eq!(play(&harmonizer(CHROMATIC, 3, ROOT + 3), 60), [72, 76, 79]);
        // An octave on top is kept over the new bottom.
        assert_eq!(play(&harmonizer(CHROMATIC, 0, ROOT + 1), 60), [72, 84]);
        // The first "Auto" is position -9.
        assert_eq!(play(&harmonizer(CHROMATIC, 3, 0), 60), [24, 28, 31]);
        let mut p = [0, 4, 7, 11];
        shift(&mut p, 2, 12);
        assert_eq!(p, [7, 11, 12, 16]);
    }

    #[test]
    fn chord_sets_play_their_voicings() {
        let set = |kind, position| Chords { mode: 2, scale: MAJOR, root: 60, kind, chromatic: 3, position, user: &[] };
        // Major 1: C over C2, Em over E2.
        assert_eq!(play(&set(0, ROOT), 60), [48, 64, 67, 72]);
        assert_eq!(play(&set(0, ROOT), 61), [52, 64, 67, 71]);
        // Minor 3: Cm, Ab.
        assert_eq!(play(&set(10, ROOT), 60), [48, 60, 63, 67]);
        assert_eq!(play(&set(10, ROOT), 61), [44, 60, 63, 68]);
        // Positions leave the bass.
        assert_eq!(play(&set(10, ROOT + 1), 60), [48, 63, 67, 72]);
        // In D the same pads are a tone up.
        assert_eq!(play(&Chords { root: 62, ..set(10, ROOT) }, 62), [50, 62, 65, 69]);
    }

    #[test]
    fn user_chords_sit_on_the_root() {
        let chords = vec![vec![0, 4, 7], vec![]];
        let c = Chords { mode: 3, scale: MAJOR, root: 60, kind: 0, chromatic: 3, position: ROOT, user: &chords };
        assert_eq!(play(&c, 60), [60, 64, 67]);
        assert!(play(&c, 61).is_empty());
        assert_eq!(play(&Chords { position: ROOT + 1, ..c }, 60), [64, 67, 72]);
    }

    #[test]
    fn auto_leads_towards_the_anchor() {
        let c = harmonizer(MAJOR, 3, AUTO_POS);
        let mut auto = Auto::default();
        assert_eq!(c.play(60, &mut auto), [60, 64, 67], "the first chord is the anchor");
        assert_eq!(c.play(65, &mut auto), [60, 65, 69], "F moves down to C-F-A");
        assert_eq!(c.play(67, &mut auto), [59, 62, 67], "G to B-D-G");
        assert_eq!(c.play(84, &mut auto), [84, 88, 91], "an octave's jump starts again");
        assert_eq!(auto.anchor, [84, 88, 91]);
        // A setting changed: no anchor, the last note 0.
        let sus = Chords { kind: 4, ..c };
        assert_eq!(sus.play(67, &mut auto), [67, 72, 74], "1-4-5 on G, from Root again");
        assert_eq!(auto.last, 67);
        // Dominant 9 in the chromatic scale is too wide for Auto: Root.
        let wide = Chords { chromatic: 9, ..harmonizer(CHROMATIC, 9, AUTO_POS) };
        let mut auto = Auto::default();
        wide.play(60, &mut auto);
        assert_eq!(wide.play(65, &mut auto), [65, 69, 72, 75, 79]);
    }
}
