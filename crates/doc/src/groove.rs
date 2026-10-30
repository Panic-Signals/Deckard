//! The Groove pages' Variation and Generate (docs/channel-software.md §5): edits of a
//! sound's notes in a pattern, from the settings saved with the sound.

use super::{Document, Target};
use deckard_model::Note;

/// A small random source (xorshift), seeded from the clock: like Maschine, each
/// press gives a different result.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new() -> Rng {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Rng(t | 1)
    }

    #[cfg(test)]
    pub(crate) fn seeded(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn normal(&mut self) -> f64 {
        let (u, v) = (self.unit().max(1e-12), self.unit());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }

    /// A value in [lo, hi] with Maschine's spreads: 0 Even (uniform), 1 Natural (normal
    /// around the middle, σ = width/4), 2 Skewed (normal from the low end); draws
    /// outside are drawn again.
    pub(crate) fn spread(&mut self, lo: f64, hi: f64, distribution: i64) -> f64 {
        if hi <= lo {
            return lo;
        }
        let sigma = (hi - lo) / 4.0;
        for _ in 0..64 {
            let x = match distribution {
                0 => lo + self.unit() * (hi - lo),
                1 => (lo + hi) / 2.0 + self.normal() * sigma,
                _ => lo + self.normal().abs() * sigma,
            };
            if (lo..=hi).contains(&x) {
                return x;
            }
        }
        (lo + hi) / 2.0
    }
}

/// Step length in ticks of a Euclidean division (`grid` for Grid; Fit Bar: the bar
/// split into the steps).
pub fn division_ticks(division: i64, grid: i64, bar: i64, steps: i64) -> i64 {
    match division {
        1 => 960,
        2 => 480,
        3 => 240,
        4 => 120,
        5 => 320,
        6 => 160,
        7 => 768,
        8 => 384,
        9 => 192,
        10 => (bar / steps.max(1)).max(1),
        _ => grid.max(1),
    }
}

/// Which of `n` steps get a note: `k` spread as evenly as can be, turned by `r`.
pub fn euclid(n: usize, k: usize, r: usize) -> Vec<bool> {
    let k = k.min(n);
    let mut acc = n - k;
    let mut bits: Vec<bool> = (0..n)
        .map(|_| {
            acc += k;
            if acc >= n {
                acc -= n;
                true
            } else {
                false
            }
        })
        .collect();
    if n > 0 {
        bits.rotate_left(r % n);
    }
    bits
}

/// Where Variation and Generate settings come from: a field (`sequence_variation/…`,
/// `sequence_generation/…`) to its value, if set.
pub type Settings<'a> = &'a dyn Fn(&str) -> Option<f64>;

impl Document {
    /// The settings saved with sound `s` of group `g` (projects from Maschine 3 keep
    /// them; older ones have none).
    pub fn groove_setting(&self, g: usize, s: usize, field: &str) -> Option<f64> {
        self.num(Target::Sound(g, s), field)
    }

    /// Whether sound `s` of group `g` keeps Variation and Generate settings.
    pub fn has_groove_settings(&self, g: usize, s: usize) -> bool {
        self.groove_setting(g, s, "sequence_variation/velocity_amount").is_some()
    }

    /// Humanize sound `s`'s notes in pattern `p` of group `g` with its Variation
    /// settings (`grid`: the step, in ticks, the time and length shifts are measured in).
    pub fn apply_variation(&mut self, g: usize, p: usize, s: usize, grid: i64, settings: Settings) -> Result<(), String> {
        self.apply_variation_with(g, p, s, grid, settings, None, &mut Rng::new())
    }

    /// [`Document::apply_variation`] starting from `base` (the notes as they were
    /// before a run of live changes) instead of the notes now.
    pub fn apply_variation_from(
        &mut self,
        g: usize,
        p: usize,
        s: usize,
        grid: i64,
        settings: Settings,
        base: &[Note],
    ) -> Result<(), String> {
        self.apply_variation_with(g, p, s, grid, settings, Some(base), &mut Rng::new())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_variation_with(
        &mut self,
        g: usize,
        p: usize,
        s: usize,
        grid: i64,
        settings: Settings,
        base: Option<&[Note]>,
        rng: &mut Rng,
    ) -> Result<(), String> {
        let pat = self.project.groups.get(g).and_then(|x| x.patterns.get(p)).ok_or("no such pattern")?.clone();
        let v = |f: &str, d: f64| settings(&format!("sequence_variation/{f}")).unwrap_or(d);
        let (vel, time, scale, var, dist) = (
            v("velocity_amount", 0.0) / 100.0,
            v("time_shift", 0.0) / 100.0,
            4f64.powf((v("length_scale", 0.5) - 0.5) * 2.0),
            v("length_variation", 0.0) / 100.0,
            v("distribution", 1.0) as i64,
        );
        let old: Vec<Note> = pat.notes.iter().filter(|n| n.sound == s).copied().collect();
        if old.is_empty() {
            return Err("this sound has no notes in the pattern".into());
        }
        let step = grid.max(1) as f64;
        let end = pat.start + if pat.length > 0 { pat.length } else { self.project.bar_length() };
        let mut new: Vec<Note> = Vec::new();
        for n in base.unwrap_or(&old) {
            let mut m = *n;
            let dv = rng.spread(-vel * 63.5, vel * 63.5, dist);
            m.velocity = (n.velocity as f64 + dv).round().clamp(1.0, 127.0) as i64;
            let dt = rng.spread(-time * step, time * step, dist);
            m.position = ((n.position as f64 + dt).round() as i64).clamp(pat.start, end - 1);
            let dl = rng.spread(-var * step, var * step, dist);
            m.length = ((n.length as f64 * scale + dl).round() as i64).max(10);
            if !new.iter().any(|x| x.position == m.position && x.pitch == m.pitch) {
                new.push(m);
            }
        }
        self.edit_notes("Apply Variation", g, p, &old, &new)
    }

    /// Fill sound `s`'s part of pattern `p` with new notes from its Generate settings
    /// (Random or Euclidean), replacing its notes there. `polyphonic`: chords in the
    /// note range (else the sound's root note only). `scale`: whether a pitch is in the
    /// selected scale (none selected: `None`); DECKARD's rule, stricter than Maschine's,
    /// is that every note is then in it.
    #[allow(clippy::too_many_arguments)]
    pub fn generate(
        &mut self,
        g: usize,
        p: usize,
        s: usize,
        grid: i64,
        polyphonic: bool,
        scale: Option<&dyn Fn(i64) -> bool>,
        settings: Settings,
    ) -> Result<(), String> {
        self.generate_with(g, p, s, grid, polyphonic, scale, settings, &mut Rng::new())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn generate_with(
        &mut self,
        g: usize,
        p: usize,
        s: usize,
        grid: i64,
        polyphonic: bool,
        scale: Option<&dyn Fn(i64) -> bool>,
        settings: Settings,
        rng: &mut Rng,
    ) -> Result<(), String> {
        let pat = self.project.groups.get(g).and_then(|x| x.patterns.get(p)).ok_or("no such pattern")?.clone();
        let v = |f: &str, d: f64| settings(&format!("sequence_generation/{f}")).unwrap_or(d);
        let len = if pat.length > 0 { pat.length } else { self.project.bar_length() };
        let (start, end) = (pat.start, pat.start + len);
        let in_scale = |p: i64| scale.is_none_or(|f| f(p));
        // The root note, up into the scale.
        let root = (60..72).find(|&p| in_scale(p)).unwrap_or(60);
        let velocity = 100.0;
        let grid = grid.max(1);
        let note = |position: i64, pitch: i64, vel: i64, length: i64| Note {
            expr: None,
            sound: s,
            position,
            length,
            pitch,
            velocity: vel,
            probability: 100,
        };
        let mut new = Vec::new();
        if v("mode", 0.0) as i64 == 0 {
            let prob = v("probability", 55.0) / 100.0;
            let amount = v("velocity_amount", 10.0) / 100.0;
            let dist = v("distribution", 1.0) as i64;
            let (lo, hi) = (v("note_range/low", 60.0) as i64, v("note_range/high", 72.0) as i64);
            let (lo, hi) = (lo.min(hi).clamp(0, 127), lo.max(hi).clamp(0, 127));
            let count = if polyphonic { v("note_count", 1.0).clamp(1.0, 6.0) as i64 } else { 1 };
            // The pitches a chord's notes are drawn from: the range's, in the scale.
            let pool: Vec<i64> = (lo..=hi).filter(|&p| in_scale(p)).collect();
            let mut t = start;
            while t < end {
                if rng.unit() < prob {
                    let least = (count - (count - 1) / 2).max(1);
                    let k = rng.spread(least as f64, count as f64, dist).round() as i64;
                    let mut pitches: Vec<i64> = Vec::new();
                    for _ in 0..k {
                        if !polyphonic {
                            pitches = vec![root];
                            break;
                        }
                        if pool.is_empty() {
                            break;
                        }
                        for _ in 0..16 {
                            let pitch = pool[((rng.unit() * pool.len() as f64) as usize).min(pool.len() - 1)];
                            if !pitches.contains(&pitch) {
                                pitches.push(pitch);
                                break;
                            }
                        }
                    }
                    for pitch in pitches {
                        let vel = velocity + rng.spread(-amount * 63.5, amount * 63.5, dist);
                        new.push(note(t, pitch, vel.round().clamp(1.0, 127.0) as i64, grid.min(end - t)));
                    }
                }
                t += grid;
            }
        } else {
            let division = v("division", 0.0) as i64;
            let steps = v("steps", 8.0).max(1.0) as usize;
            let fills = v("fills", 5.0) as usize;
            let rotate = v("rotate", 0.0) as usize;
            let step = division_ticks(division, grid, self.project.bar_length(), steps as i64);
            let bits = euclid(steps, fills, rotate);
            let mut t = start;
            let mut i = 0;
            while t < end {
                if bits[i % steps] {
                    new.push(note(t, root, velocity as i64, step.min(end - t)));
                }
                t += step;
                i += 1;
            }
        }
        let old: Vec<Note> = pat.notes.iter().filter(|n| n.sound == s).copied().collect();
        self.edit_notes("Generate", g, p, &old, &new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euclidean_rhythms() {
        let x = |b: Vec<bool>| b.iter().map(|&on| if on { 'x' } else { '.' }).collect::<String>();
        assert_eq!(x(euclid(8, 3, 0)), "x..x..x.");
        assert_eq!(x(euclid(8, 5, 0)), "x.x.xx.x");
        assert_eq!(x(euclid(4, 4, 0)), "xxxx");
        assert_eq!(x(euclid(8, 3, 2)), ".x..x.x.");
    }

    /// DECKARD's rule for Generate's Random with a scale selected (not Maschine's, which
    /// can leave a pitch outside): every note is in the scale and the range; a range
    /// with no note of the scale gets none.
    #[test]
    fn random_keeps_to_the_scale() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "P").unwrap();
        let major = |p: i64| [0, 2, 4, 5, 7, 9, 11].contains(&p.rem_euclid(12));
        // 61..70 (C#3 to A#3, both outside C major), four-note chords on every step.
        let chords: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_generation/probability" => Some(100.0),
            "sequence_generation/note_range/low" => Some(61.0),
            "sequence_generation/note_range/high" => Some(70.0),
            "sequence_generation/note_count" => Some(4.0),
            _ => None,
        };
        for seed in 0..5 {
            d.generate_with(0, 0, 0, 240, true, Some(&major), chords, &mut Rng::seeded(seed)).unwrap();
            let n: Vec<i64> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).map(|n| n.pitch).collect();
            assert!(!n.is_empty());
            assert!(n.iter().all(|&p| major(p) && (61..=70).contains(&p)), "{n:?}");
        }
        // The root note on pads, out of the scale (C# major-ish: no C): snapped up into it.
        let no_c = |p: i64| p.rem_euclid(12) != 0;
        let every: &dyn Fn(&str) -> Option<f64> = &|f| (f == "sequence_generation/probability").then_some(100.0);
        d.generate_with(0, 0, 0, 240, false, Some(&no_c), every, &mut Rng::seeded(1)).unwrap();
        assert!(d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).all(|n| n.pitch == 61));
        // A range with none of the scale's notes: nothing.
        let none: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_generation/probability" => Some(100.0),
            "sequence_generation/note_range/low" => Some(61.0),
            "sequence_generation/note_range/high" => Some(61.0),
            _ => None,
        };
        d.generate_with(0, 0, 0, 240, true, Some(&major), none, &mut Rng::seeded(1)).unwrap();
        assert_eq!(d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).count(), 0);
    }

    #[test]
    fn spreads_stay_in_range() {
        let mut r = Rng::seeded(7);
        for d in 0..3 {
            let xs: Vec<f64> = (0..2000).map(|_| r.spread(-10.0, 10.0, d)).collect();
            assert!(xs.iter().all(|x| (-10.0..=10.0).contains(x)));
            let mean = xs.iter().sum::<f64>() / xs.len() as f64;
            match d {
                2 => assert!(mean < -3.0, "skewed low: {mean}"),
                _ => assert!(mean.abs() < 1.0, "centred: {mean}"),
            }
        }
    }

    fn cloud9() -> Option<Document> {
        Document::open(std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj")).ok()
    }

    #[test]
    fn generate_and_vary_a_sound() {
        let Some(mut d) = cloud9() else { return };
        // Euclidean 4 of 8 in eighths: a note every quarter, root note, whole pattern.
        let euclid: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_generation/mode" => Some(1.0),
            "sequence_generation/steps" => Some(8.0),
            "sequence_generation/fills" => Some(4.0),
            "sequence_generation/division" => Some(2.0),
            _ => None,
        };
        d.generate_with(0, 0, 0, 240, false, None, euclid, &mut Rng::seeded(1)).unwrap();
        let pat = d.project().groups[0].patterns[0].clone();
        let mine: Vec<Note> = pat.notes.iter().filter(|n| n.sound == 0).copied().collect();
        let len = if pat.length > 0 { pat.length } else { d.project().bar_length() };
        assert_eq!(mine.len() as i64, len / 960, "a note per quarter");
        assert!(mine.iter().all(|n| n.pitch == 60 && n.length == 480 && (n.position - pat.start) % 960 == 0));
        // Random: about the probability's share of the steps get a note.
        let random: &dyn Fn(&str) -> Option<f64> = &|f| (f == "sequence_generation/probability").then_some(50.0);
        d.generate_with(0, 0, 0, 120, false, None, random, &mut Rng::seeded(5)).unwrap();
        let n = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).count() as f64;
        let steps = (len / 120) as f64;
        assert!((n / steps - 0.5).abs() < 0.2, "{n} of {steps}");
        // Variation: velocities move, nothing else when only velocity is on.
        let before: Vec<Note> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect();
        let vary: &dyn Fn(&str) -> Option<f64> = &|f| (f == "sequence_variation/velocity_amount").then_some(100.0);
        d.apply_variation_with(0, 0, 0, 240, vary, None, &mut Rng::seeded(3)).unwrap();
        let after: Vec<Note> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect();
        assert_eq!(after.len(), before.len());
        assert!(after.iter().zip(&before).all(|(a, b)| a.position == b.position && a.length == b.length));
        assert!(after.iter().zip(&before).any(|(a, b)| a.velocity != b.velocity), "velocities varied");
        // One undo step each.
        d.undo();
        assert_eq!(d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect::<Vec<_>>(), before);
        assert!(!d.has_groove_settings(0, 0), "this project predates them");
    }
}
