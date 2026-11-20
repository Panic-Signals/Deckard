//! The Groove pages' Variation and Generate (docs/channel-software.md §5): edits of a
//! sound's notes in a pattern, from the settings saved with the sound.

use super::{Document, Op, Target};
use deckard_model::{GROOVE, Note};
use mxprj::decode::Value;

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

/// The pitches a chord of `count` notes drawn from `pool`: distinct, so a chord is not
/// one note twice.
fn chord_pitches(rng: &mut Rng, pool: &[i64], count: i64, dist: i64) -> Vec<i64> {
    let mut pitches: Vec<i64> = Vec::new();
    if pool.is_empty() {
        return pitches;
    }
    for _ in 0..count.clamp(0, 6) {
        for _ in 0..16 {
            let i = rng.spread(0.0, (pool.len() - 1) as f64, dist).round() as usize;
            let pitch = pool[i.min(pool.len() - 1)];
            if !pitches.contains(&pitch) {
                pitches.push(pitch);
                break;
            }
        }
    }
    pitches
}

/// How many notes a chord gets: the CHORDS value itself when the distribution is Fixed
/// (3), else drawn from the top half up to it.
fn chord_count(rng: &mut Rng, count: i64, dist: i64) -> i64 {
    if dist == 3 {
        count
    } else {
        let least = (count - (count - 1) / 2).max(1);
        rng.spread(least as f64, count as f64, dist).round() as i64
    }
}

/// The Variation page's settings (work-item 35): Maschine's, with DECKARD's defaults
/// where the sound saves nothing. They live under `sequence_variation/…`, which
/// [`Sound::groove`] keeps when the project's format has no place for them (its
/// `SequenceVariationModule` is four floats, an enum and a bool).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Variation {
    /// 0 Humanize, 1 Random.
    pub mode: i64,
    pub velocity_lo: i64,
    pub velocity_hi: i64,
    /// Per cent of one grid step, 0–50.
    pub time_shift: f64,
    /// Per cent, 10–100.
    pub probability: i64,
    pub note_lo: i64,
    pub note_hi: i64,
    /// Notes a chord has per step, 1–6.
    pub note_count: i64,
    /// A note's length in grid steps, 1–6.
    pub steps: f64,
    /// Distributions (0 Equal, 1 Gauss, 2 ½ Gauss, 3 Fixed): the chord's size, the
    /// pitches, the lengths.
    pub dist_note_count: i64,
    pub dist_notes: i64,
    pub dist_note_length: i64,
}

impl Default for Variation {
    fn default() -> Variation {
        Variation {
            mode: 0,
            velocity_lo: 100,
            velocity_hi: 127,
            time_shift: 0.0,
            probability: 50,
            note_lo: 60,
            note_hi: 72,
            note_count: 1,
            steps: 1.0,
            dist_note_count: 3,
            dist_notes: 0,
            dist_note_length: 3,
        }
    }
}

impl Variation {
    /// What `settings` gives (`sequence_variation/…` names) over DECKARD's defaults,
    /// each clamped to what the manual allows; a range given the wrong way round is
    /// read low to high.
    pub fn from_settings(settings: Settings) -> Variation {
        let d = Variation::default();
        let v = |f: &str, d: f64| settings(&format!("sequence_variation/{f}")).unwrap_or(d);
        let (lo, hi) = (v("velocity_range/low", d.velocity_lo as f64), v("velocity_range/high", d.velocity_hi as f64));
        // The generator's own half of the page: Random and Euclid are the generator, so
        // its probability, note range and chords are the generator's settings, where its
        // module keeps them (`sequence_generation/…`).
        let g = |f: &str| settings(&format!("sequence_generation/{f}"));
        let (nlo, nhi) = (g("note_range/low").unwrap_or(d.note_lo as f64), g("note_range/high").unwrap_or(d.note_hi as f64));
        Variation {
            // Humanize, Random, Euclid: the page's three, in the order it lists them.
            mode: (v("mode", d.mode as f64) as i64).clamp(0, 2),
            velocity_lo: lo.min(hi).round().clamp(0.0, 127.0) as i64,
            velocity_hi: lo.max(hi).round().clamp(0.0, 127.0) as i64,
            time_shift: v("time_shift", d.time_shift).clamp(0.0, 50.0),
            probability: (g("probability").unwrap_or(d.probability as f64) as i64).clamp(10, 100),
            note_lo: nlo.min(nhi).round().clamp(0.0, 127.0) as i64,
            note_hi: nlo.max(nhi).round().clamp(0.0, 127.0) as i64,
            note_count: (g("note_count").unwrap_or(d.note_count as f64) as i64).clamp(1, 6),
            steps: v("steps", d.steps).clamp(1.0, 6.0),
            dist_note_count: (v("dist/note_count", d.dist_note_count as f64) as i64).clamp(0, 3),
            dist_notes: (v("dist/notes", d.dist_notes as f64) as i64).clamp(0, 2),
            dist_note_length: (v("dist/note_length", d.dist_note_length as f64) as i64).clamp(0, 3),
        }
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
    /// Variation or Generate setting `field` of sound `s` of group `g`: the project's
    /// own (projects from Maschine 3 keep them), else DECKARD's, kept with the sound.
    pub fn groove_setting(&self, g: usize, s: usize, field: &str) -> Option<f64> {
        // The project's own where the sound's module holds the field — and DECKARD's
        // otherwise: the format's Variation module is four floats and an enum, so the
        // page's other settings (its mode, ranges, chords, distributions) have no place
        // there and live with the sound.
        if self.has_groove_settings(g, s) {
            if let Some(v) = self.num(Target::Sound(g, s), field) {
                return Some(v);
            }
        }
        self.kept_groove(g, s, field)
    }

    /// Set Variation or Generate setting `field` of sound `s` of group `g`, where
    /// [`Document::groove_setting`] reads it (one undo step; a knob turn is one).
    pub fn set_groove_setting(&mut self, g: usize, s: usize, field: &str, v: f64) -> Result<(), String> {
        // Where the sound's module holds the field, there; else in DECKARD's field, where
        // `groove_setting` reads it back. (Writing the format's path blind is what made
        // the page's settings vanish: the module has no `mode`, and the write errored.)
        if self.has_groove_settings(g, s) && self.num(Target::Sound(g, s), field).is_some() {
            return self.set("Channel Property", Target::Sound(g, s), field, Value::Float(v));
        }
        let ops = self.groove_ops(g, s, field, v)?;
        if ops.is_empty() {
            return Ok(());
        }
        self.apply_with("Channel Property", ops, vec![Some((Target::Sound(g, s), GROOVE.to_string()))])
    }

    /// The Groove page (0 Swing, 1 Variation, 2 Generate) sound `s` of group `g` was
    /// left on, if it has been shown.
    pub fn groove_page(&self, g: usize, s: usize) -> Option<usize> {
        self.kept_groove(g, s, "page").map(|p| (p.max(0.0) as usize).min(2))
    }

    /// Keep the Groove page sound `s` of group `g` shows: saved and copied with the
    /// sound, but no edit (no undo step, and an unchanged project stays unchanged).
    pub fn set_groove_page(&mut self, g: usize, s: usize, page: usize) {
        if self.groove_page(g, s) == Some(page) {
            return;
        }
        let Ok(ops) = self.groove_ops(g, s, "page", page as f64) else { return };
        let clean = !self.is_dirty();
        if self.run(&ops).is_ok() {
            self.update(&[Some((Target::Sound(g, s), GROOVE.to_string()))]);
            if clean {
                self.saved_revision = self.revision;
            }
        }
    }

    fn kept_groove(&self, g: usize, s: usize, field: &str) -> Option<f64> {
        let snd = self.project.groups.get(g)?.sounds.get(s)?;
        snd.groove.iter().find(|(k, _)| k == field).map(|(_, v)| *v)
    }

    /// The op that sets `field` of sound (`g`, `s`)'s DECKARD Groove field to `v`
    /// (none if it is).
    pub(crate) fn groove_ops(&self, g: usize, s: usize, field: &str, v: f64) -> Result<Vec<Op>, String> {
        let snd = self.project.groups.get(g).and_then(|x| x.sounds.get(s)).ok_or("no such sound")?;
        let mut kept = snd.groove.clone();
        match kept.iter_mut().find(|(k, _)| k == field) {
            Some((_, x)) if *x == v => return Ok(Vec::new()),
            Some((_, x)) => *x = v,
            None => kept.push((field.to_string(), v)),
        }
        kept.sort_by(|a, b| a.0.cmp(&b.0));
        self.groove_text_ops(g, s, &deckard_model::groove_to_text(&kept))
    }

    /// The op that sets sound (`g`, `s`)'s DECKARD Groove field to `text`.
    pub(crate) fn groove_text_ops(&self, g: usize, s: usize, text: &str) -> Result<Vec<Op>, String> {
        let path = self.owner(Target::Sound(g, s)).ok_or_else(|| format!("no sound {} in group {}", s + 1, g + 1))?;
        let value = (!text.is_empty()).then(|| Value::Str(text.to_string()));
        Ok(vec![Op::SetField { path, field: GROOVE.to_string(), value }])
    }

    /// Copy sound `target`'s DECKARD Groove field from the tree into the model.
    pub(crate) fn patch_groove(&mut self, target: Target) -> bool {
        let Target::Sound(g, s) = target else { return false };
        let Some(path) = self.owner(target) else { return false };
        let groove = match self.tree.at(&path) {
            Some(Value::Obj(o)) => match o.get(GROOVE) {
                Some(Value::Str(t)) => deckard_model::groove_from_text(t),
                _ => Vec::new(),
            },
            _ => return false,
        };
        let Some(snd) = self.project.groups.get_mut(g).and_then(|x| x.sounds.get_mut(s)) else { return false };
        snd.groove = groove;
        true
    }

    /// Whether sound `s` of group `g` keeps Variation and Generate settings.
    pub fn has_groove_settings(&self, g: usize, s: usize) -> bool {
        self.num(Target::Sound(g, s), "sequence_variation/velocity_amount").is_some()
    }

    /// Sound `s`'s Variation settings (group `g`), DECKARD's defaults where the sound
    /// saves none.
    pub fn variation(&self, g: usize, s: usize) -> Variation {
        Variation::from_settings(&|f| self.groove_setting(g, s, f))
    }

    /// The Groove page's one action, chosen by its Mode: 0 Humanize varies the notes that
    /// are there, 1 Random and 2 Euclid run the generator with its own settings
    /// (`sequence_generation/…`), which reads the page's velocity band, length and time
    /// shift where they are set. One engine, two surfaces — the in-app Groove page and the
    /// Jam's (work-item 35).
    #[allow(clippy::too_many_arguments)]
    pub fn apply_groove_page(
        &mut self,
        g: usize,
        p: usize,
        s: usize,
        grid: i64,
        polyphonic: bool,
        scale: Option<&dyn Fn(i64) -> bool>,
        settings: Settings,
    ) -> Result<(), String> {
        self.apply_groove_page_with(g, p, s, grid, polyphonic, scale, settings, None, &mut Rng::new())
    }

    /// [`Document::apply_groove_page`] starting from `base` (the notes as they were before
    /// a run of live changes): Humanize re-applies from it, the generator replaces.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_groove_page_from(
        &mut self,
        g: usize,
        p: usize,
        s: usize,
        grid: i64,
        polyphonic: bool,
        scale: Option<&dyn Fn(i64) -> bool>,
        settings: Settings,
        base: &[Note],
    ) -> Result<(), String> {
        self.apply_groove_page_with(g, p, s, grid, polyphonic, scale, settings, Some(base), &mut Rng::new())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_groove_page_with(
        &mut self,
        g: usize,
        p: usize,
        s: usize,
        grid: i64,
        polyphonic: bool,
        scale: Option<&dyn Fn(i64) -> bool>,
        settings: Settings,
        base: Option<&[Note]>,
        rng: &mut Rng,
    ) -> Result<(), String> {
        let mode = settings("sequence_variation/mode").unwrap_or(0.0) as i64;
        if mode == 0 {
            return self.apply_variation_with(g, p, s, grid, scale, settings, base, rng);
        }
        // Random is the generator's Random (0), Euclid its Euclidean (1).
        let generator = if mode == 1 { 0.0 } else { 1.0 };
        let forced = |f: &str| if f == "sequence_generation/mode" { Some(generator) } else { settings(f) };
        self.generate_with(g, p, s, grid, polyphonic, scale, &forced, rng)
    }

    /// Humanize (the Variation page's mode 0): move sound `s`'s notes in pattern `p` of
    /// group `g` where they are — velocity into the band, the time shift, the length's
    /// scale (`grid`: the step, in ticks). Random and Euclid are the generator's, through
    /// [`Document::apply_groove_page`].
    pub fn apply_variation(
        &mut self,
        g: usize,
        p: usize,
        s: usize,
        grid: i64,
        scale: Option<&dyn Fn(i64) -> bool>,
        settings: Settings,
    ) -> Result<(), String> {
        self.apply_variation_with(g, p, s, grid, scale, settings, None, &mut Rng::new())
    }

    /// [`Document::apply_variation`] starting from `base` (the notes as they were
    /// before a run of live changes) instead of the notes now.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_variation_from(
        &mut self,
        g: usize,
        p: usize,
        s: usize,
        grid: i64,
        scale: Option<&dyn Fn(i64) -> bool>,
        settings: Settings,
        base: &[Note],
    ) -> Result<(), String> {
        self.apply_variation_with(g, p, s, grid, scale, settings, Some(base), &mut Rng::new())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_variation_with(
        &mut self,
        g: usize,
        p: usize,
        s: usize,
        grid: i64,
        // Humanize keeps the pitches it has; the dispatcher passes the scale on to the
        // generator (Random, Euclid), which is the only half that snaps to it.
        _scale: Option<&dyn Fn(i64) -> bool>,
        settings: Settings,
        base: Option<&[Note]>,
        rng: &mut Rng,
    ) -> Result<(), String> {
        let pat = self.project.groups.get(g).and_then(|x| x.patterns.get(p)).ok_or("no such pattern")?.clone();
        // The page's own settings, and the two the in-app Variation page has always read:
        // the velocity's deviation and the length's scale.
        let v = |f: &str, d: f64| settings(&format!("sequence_variation/{f}")).unwrap_or(d);
        let st = Variation::from_settings(settings);
        let (amount, length_scale, length_variation) =
            (v("velocity_amount", 0.0) / 100.0, 4f64.powf((v("length_scale", 0.5) - 0.5) * 2.0), v("length_variation", 0.0) / 100.0);
        let old: Vec<Note> = pat.notes.iter().filter(|n| n.sound == s).copied().collect();
        if old.is_empty() {
            return Err("this sound has no notes in the pattern".into());
        }
        let step = grid.max(1) as f64;
        let end = pat.start + if pat.length > 0 { pat.length } else { self.project.bar_length() };
        let time = st.time_shift / 100.0;
        // Where a note may land: inside the pattern, and at least 10 ticks long.
        let place = |wanted: f64| (wanted.round() as i64).clamp(pat.start, end - 1);
        let fit = |wanted: f64, at: i64| (wanted.round() as i64).clamp(10, (end - at).max(10));
        let mut new: Vec<Note> = Vec::new();
        // Humanize: nothing is made or dropped. A note's velocity stays where it is
        // inside the band, and its deviation (`velocity_amount`, the in-app page's
        // knob) may take it out, so the band clamps it back in.
        let dist = v("distribution", 1.0) as i64;
        for n in base.unwrap_or(&old) {
            let mut m = *n;
            let dv = rng.spread(-amount * 63.5, amount * 63.5, dist);
            m.velocity = (n.velocity as f64 + dv).round().clamp(st.velocity_lo as f64, st.velocity_hi as f64) as i64;
            m.position = place(n.position as f64 + rng.spread(-time * step, time * step, dist));
            let dl = rng.spread(-length_variation * step, length_variation * step, dist);
            m.length = fit(n.length as f64 * length_scale + dl, m.position);
            if !new.iter().any(|x| x.position == m.position && x.pitch == m.pitch) {
                new.push(m);
            }
        }
        self.edit_notes("Apply Variation", g, p, &old, &new)
    }

    /// Fill sound `s`'s part of pattern `p` with new notes from its Generate settings
    /// (Random or Euclidean), replacing its notes there. `polyphonic`: chords in the
    /// note range (else the sound's root note only: its base key, as its pad plays). `scale`: whether a pitch is in the
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
        // The root note: the sound's base key (what its pad plays), up into the scale.
        let base = self.project.groups.get(g).and_then(|x| x.sounds.get(s)).map_or(60, |x| x.channel.base_key).clamp(0, 127);
        let root = (base..(base + 12).min(128)).find(|&p| in_scale(p)).unwrap_or(base);
        let velocity = 100.0;
        let grid = grid.max(1);
        let note = |position: i64, pitch: i64, vel: i64, length: i64| Note {
            ratchet: Default::default(),
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
            // The Variation page's slots each do something here: where the page has set
            // one, the Random engine reads it, so the Jam's Apply and the app's Generate
            // page run this one piece of code. Unset, the older behaviour stands.
            let sv = |f: &str| settings(&format!("sequence_variation/{f}"));
            let band = match (sv("velocity_range/low"), sv("velocity_range/high")) {
                (None, None) => None,
                (lo, hi) => {
                    let (lo, hi) = (lo.unwrap_or(0.0), hi.unwrap_or(127.0));
                    Some((lo.min(hi).clamp(0.0, 127.0), lo.max(hi).clamp(0.0, 127.0)))
                }
            };
            let step = grid.max(1) as f64;
            let time_shift = sv("time_shift").unwrap_or(0.0).clamp(0.0, 50.0) / 100.0;
            let length = sv("steps").map_or(grid as f64, |x| x.clamp(1.0, 6.0) * step);
            let dist_count = sv("dist/note_count").map_or(dist, |d| d as i64);
            let dist_notes = sv("dist/notes").map_or(dist, |d| d as i64);
            // A note's length had no distribution before the page existed: unset, it
            // stays fixed rather than picking up the chord's.
            let dist_length = sv("dist/note_length").map_or(3, |d| d as i64);
            let (lo, hi) = (v("note_range/low", 60.0) as i64, v("note_range/high", 72.0) as i64);
            let (lo, hi) = (lo.min(hi).clamp(0, 127), lo.max(hi).clamp(0, 127));
            let count = if polyphonic { v("note_count", 1.0).clamp(1.0, 6.0) as i64 } else { 1 };
            // The pitches a chord's notes are drawn from: the range's, in the scale.
            let pool: Vec<i64> = (lo..=hi).filter(|&p| in_scale(p)).collect();
            let mut t = start;
            while t < end {
                if rng.unit() < prob {
                    // The chord's size and its pitches come from the same helpers the
                    // Variation page uses, so the two pages cannot drift apart.
                    let k = chord_count(rng, count, dist_count);
                    let pitches = if polyphonic { chord_pitches(rng, &pool, k, dist_notes) } else { vec![root] };
                    let at =
                        ((t as f64 + rng.spread(-time_shift * step, time_shift * step, dist_notes)).round() as i64).clamp(start, end - 1);
                    let want = if dist_length == 3 { length } else { rng.spread(length * 0.5, length * 1.5, dist_length) };
                    let len = (want.round() as i64).clamp(10, (end - at).max(10));
                    for pitch in pitches {
                        let vel = match band {
                            Some((band_lo, band_hi)) => rng.spread(band_lo, band_hi, 0),
                            None => velocity + rng.spread(-amount * 63.5, amount * 63.5, dist),
                        };
                        if new.iter().any(|x: &Note| x.position == at && x.pitch == pitch) {
                            continue;
                        }
                        new.push(note(at, pitch, vel.round().clamp(1.0, 127.0) as i64, len));
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

    /// Euclidean (and Random's single notes) go on the sound's base key, the pitch its
    /// pad plays, moved up into the scale.
    #[test]
    fn generate_writes_the_sounds_base_key() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "P").unwrap();
        d.project.groups[0].sounds[0].channel.base_key = 50;
        let euclid: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_generation/mode" => Some(1.0),
            "sequence_generation/steps" => Some(4.0),
            "sequence_generation/fills" => Some(4.0),
            "sequence_generation/division" => Some(1.0),
            _ => None,
        };
        d.generate_with(0, 0, 0, 240, false, None, euclid, &mut Rng::seeded(1)).unwrap();
        let pitches = |d: &Document| d.project.groups[0].patterns[0].notes.iter().map(|n| n.pitch).collect::<Vec<_>>();
        assert!(!pitches(&d).is_empty() && pitches(&d).iter().all(|&p| p == 50), "{:?}", pitches(&d));
        // D is out of C major: up to E.
        let major = |p: i64| [0, 2, 4, 5, 7, 9, 11].contains(&p.rem_euclid(12));
        d.project.groups[0].sounds[0].channel.base_key = 51;
        d.generate_with(0, 0, 0, 240, false, Some(&major), euclid, &mut Rng::seeded(1)).unwrap();
        assert!(pitches(&d).iter().all(|&p| p == 52), "{:?}", pitches(&d));
    }

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
        d.apply_variation_with(0, 0, 0, 240, None, vary, None, &mut Rng::seeded(3)).unwrap();
        let after: Vec<Note> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect();
        assert_eq!(after.len(), before.len());
        assert!(after.iter().zip(&before).all(|(a, b)| a.position == b.position && a.length == b.length));
        assert!(after.iter().zip(&before).any(|(a, b)| a.velocity != b.velocity), "velocities varied");
        // One undo step each.
        d.undo();
        assert_eq!(d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect::<Vec<_>>(), before);
        assert!(!d.has_groove_settings(0, 0), "this project predates them");
    }

    /// A sound's settings (here the project's own) and its Groove page are kept with
    /// the sound: undone, saved, and copied with it.
    #[test]
    fn groove_settings_are_kept_with_the_sound() {
        let mut d = Document::new_project().unwrap();
        d.set_groove_setting(0, 0, "sequence_generation/mode", 1.0).unwrap();
        d.set_groove_setting(0, 0, "sequence_generation/fills", 3.0).unwrap();
        assert_eq!(d.groove_setting(0, 0, "sequence_generation/fills"), Some(3.0));
        assert_ne!(d.groove_setting(0, 1, "sequence_generation/fills"), Some(3.0), "only that sound's");
        // The page is no edit.
        let dirty = d.is_dirty();
        let steps = d.undo_labels().len();
        d.set_groove_page(0, 0, 2);
        assert_eq!((d.groove_page(0, 0), d.groove_page(0, 1)), (Some(2), None));
        assert_eq!((d.is_dirty(), d.undo_labels().len()), (dirty, steps));
        // Saved.
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(back.groove_setting(0, 0, "sequence_generation/fills"), Some(3.0));
        assert_eq!(back.groove_page(0, 0), Some(2));
        // Copied.
        let clip = d.copy_sound(0, 0).unwrap();
        d.paste_sounds(0, 5, &[clip]).unwrap();
        assert_eq!(d.groove_setting(0, 5, "sequence_generation/mode"), Some(1.0));
        assert_eq!(d.groove_setting(0, 5, "sequence_generation/fills"), Some(3.0));
        assert_eq!(d.groove_page(0, 5), Some(2));
    }

    /// A sound from a format with no place for the settings (a kit from before
    /// Maschine 3) keeps them in DECKARD's field: undone, copied, and not left
    /// on its pad for a sound pasted over it.
    #[test]
    fn a_sound_without_a_place_keeps_its_settings_in_deckards_field() {
        let mut d = Document::new_project().unwrap();
        d.tree.walk_mut(&mut |o| o.fields.retain(|(f, _)| f != "sequence_variation" && f != "sequence_generation"));
        d.refresh();
        assert!(!d.has_groove_settings(0, 0));
        d.set_groove_setting(0, 0, "sequence_generation/mode", 1.0).unwrap();
        d.set_groove_setting(0, 0, "sequence_generation/fills", 3.0).unwrap();
        assert_eq!(d.groove_setting(0, 0, "sequence_generation/fills"), Some(3.0));
        assert_eq!(d.undo_labels().len(), 1, "one knob session");
        // (Saving is `groove_settings_are_kept_with_the_sound`'s page: the same block.)
        let clip = d.copy_sound(0, 0).unwrap();
        d.paste_sounds(0, 5, &[clip]).unwrap();
        assert_eq!(d.groove_setting(0, 5, "sequence_generation/fills"), Some(3.0));
        let empty = d.copy_sound(0, 1).unwrap();
        d.paste_sounds(0, 0, &[empty]).unwrap();
        assert_eq!(d.groove_setting(0, 0, "sequence_generation/fills"), None, "the pasted sound's own");
        d.undo();
        d.undo();
        d.undo();
        assert_eq!(d.groove_setting(0, 0, "sequence_generation/fills"), None);
    }

    /// Humanize moves the notes it has and only what the page says: the velocity band
    /// bounds them (the in-app knob's deviation may push one out, so it is clamped back),
    /// and nothing is created, dropped or re-pitched.
    #[test]
    fn humanize_bounds_velocity_and_leaves_the_rest() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "P").unwrap();
        let euclid: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_generation/mode" => Some(1.0),
            "sequence_generation/steps" => Some(4.0),
            "sequence_generation/fills" => Some(4.0),
            "sequence_generation/division" => Some(1.0),
            _ => None,
        };
        d.generate_with(0, 0, 0, 240, false, None, euclid, &mut Rng::seeded(1)).unwrap();
        let band: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_variation/velocity_range/low" => Some(40.0),
            "sequence_variation/velocity_range/high" => Some(60.0),
            _ => None,
        };
        let before: Vec<Note> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect();
        assert!(!before.is_empty());
        d.apply_variation_with(0, 0, 0, 240, None, band, None, &mut Rng::seeded(3)).unwrap();
        let after: Vec<Note> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect();
        assert_eq!(after.len(), before.len(), "nothing made or dropped");
        assert!(after.iter().all(|n| (40..=60).contains(&n.velocity)), "{:?}", after.iter().map(|n| n.velocity).collect::<Vec<_>>());
        assert!(after.iter().zip(&before).all(|(a, b)| a.position == b.position && a.pitch == b.pitch && a.length == b.length));
    }

    /// Random varies what is there: what the probability keeps is re-pitched into the
    /// note range as a chord of the Note Count, in the velocity band, at the step length.
    #[test]
    fn random_keeps_the_ranges_the_chords_and_the_length() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "P").unwrap();
        let euclid: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_generation/mode" => Some(1.0),
            "sequence_generation/steps" => Some(4.0),
            "sequence_generation/fills" => Some(4.0),
            "sequence_generation/division" => Some(1.0),
            _ => None,
        };
        d.generate_with(0, 0, 0, 240, false, None, euclid, &mut Rng::seeded(1)).unwrap();
        let page: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_variation/mode" => Some(1.0),
            "sequence_generation/probability" => Some(100.0),
            "sequence_generation/note_range/low" => Some(60.0),
            "sequence_generation/note_range/high" => Some(64.0),
            "sequence_generation/note_count" => Some(3.0),
            "sequence_variation/velocity_range/low" => Some(70.0),
            "sequence_variation/velocity_range/high" => Some(80.0),
            _ => None,
        };
        d.apply_groove_page_with(0, 0, 0, 240, true, None, page, None, &mut Rng::seeded(4)).unwrap();
        let after: Vec<Note> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect();
        assert!(!after.is_empty());
        assert!(after.iter().all(|n| (60..=64).contains(&n.pitch)), "{:?}", after.iter().map(|n| n.pitch).collect::<Vec<_>>());
        assert!(after.iter().all(|n| (70..=80).contains(&n.velocity)));
        assert!(after.iter().all(|n| n.length == 240), "a step long");
        let chord = |at: i64| after.iter().filter(|n| n.position == at).count();
        let most = after.iter().map(|n| chord(n.position)).max().unwrap_or(0);
        assert_eq!(most, 3, "the chord is Note Count notes (the range holds five pitches)");
    }

    /// Every setting the Variation page shows survives a save into the sound and reads
    /// back: the format's module holds four floats and an enum, so the rest ride on
    /// DECKARD's field — and a write that lands nowhere would make Apply read its
    /// defaults and look like it did nothing.
    #[test]
    fn every_variation_setting_round_trips() {
        let mut d = Document::new_project().unwrap();
        // Each setting where its engine keeps it: the generator's own half in its module,
        // DECKARD's extension of the page in the sound's field.
        for (name, value) in [
            ("sequence_variation/mode", 2.0),
            ("sequence_variation/velocity_range/low", 40.0),
            ("sequence_variation/velocity_range/high", 60.0),
            ("sequence_variation/time_shift", 25.0),
            ("sequence_variation/steps", 2.0),
            ("sequence_variation/dist/note_count", 1.0),
            ("sequence_variation/dist/notes", 2.0),
            ("sequence_variation/dist/note_length", 3.0),
            ("sequence_generation/probability", 70.0),
            ("sequence_generation/note_range/low", 58.0),
            ("sequence_generation/note_range/high", 74.0),
            ("sequence_generation/note_count", 3.0),
        ] {
            d.set_groove_setting(0, 0, name, value).unwrap();
            assert_eq!(d.groove_setting(0, 0, name), Some(value), "{name}");
        }
        let v = d.variation(0, 0);
        assert_eq!((v.mode, v.velocity_lo, v.velocity_hi, v.probability), (2, 40, 60, 70), "Euclid reads back as 2");
        assert_eq!((v.note_count, v.steps, v.dist_notes), (3, 2.0, 2));
    }

    /// The app's own path: the page's settings saved into the sound, read back through
    /// `groove_setting` as `GROOVE_FIELDS` reads them, then Apply. (Writing the format's
    /// path blind is what once made every turn vanish and Apply a no-op.)
    #[test]
    fn the_apps_path_reads_its_settings_and_varies_the_notes() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "P").unwrap();
        let euclid: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_generation/mode" => Some(1.0),
            "sequence_generation/steps" => Some(4.0),
            "sequence_generation/fills" => Some(4.0),
            "sequence_generation/division" => Some(1.0),
            _ => None,
        };
        d.generate_with(0, 0, 0, 240, false, None, euclid, &mut Rng::seeded(1)).unwrap();
        for (field, value) in [
            ("sequence_variation/mode", 1.0),
            ("sequence_generation/probability", 100.0),
            ("sequence_generation/note_range/low", 60.0),
            ("sequence_generation/note_range/high", 64.0),
        ] {
            d.set_groove_setting(0, 0, field, value).unwrap();
        }
        // The viewer's list, as `GROOVE_FIELDS` has it.
        let fields = [
            "sequence_variation/mode",
            "sequence_variation/velocity_amount",
            "sequence_variation/time_shift",
            "sequence_variation/length_scale",
            "sequence_variation/length_variation",
            "sequence_variation/distribution",
            "sequence_variation/velocity_range/low",
            "sequence_variation/velocity_range/high",
            "sequence_variation/steps",
            "sequence_generation/probability",
            "sequence_generation/note_range/low",
            "sequence_generation/note_range/high",
            "sequence_generation/note_count",
        ];
        let settings: Vec<(String, f64)> = fields.iter().filter_map(|f| d.groove_setting(0, 0, f).map(|v| (f.to_string(), v))).collect();
        let get = |f: &str| settings.iter().find(|(k, _)| k == f).map(|(_, v)| *v);
        let before: Vec<Note> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect();
        d.apply_groove_page(0, 0, 0, 240, true, None, &get).unwrap();
        let after: Vec<Note> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect();
        assert_ne!(after, before, "Apply varies the notes");
        assert!(after.iter().all(|n| (60..=64).contains(&n.pitch)), "{:?}", after.iter().map(|n| n.pitch).collect::<Vec<_>>());
        assert_eq!(d.variation(0, 0).mode, 1, "the page reads its own mode back");
    }

    /// Random makes notes where the sound had none: the probability is per *step* of the
    /// pattern ("at 50 % a note may or may not be created at each step"), so a sound with
    /// four notes in a bar of sixteen fills it when the probability is 100.
    #[test]
    fn random_makes_notes_on_the_steps_the_sound_had_none() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "P").unwrap();
        let euclid: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_generation/mode" => Some(1.0),
            "sequence_generation/steps" => Some(4.0),
            "sequence_generation/fills" => Some(4.0),
            "sequence_generation/division" => Some(1.0),
            _ => None,
        };
        d.generate_with(0, 0, 0, 240, false, None, euclid, &mut Rng::seeded(1)).unwrap();
        let bar = d.project().bar_length();
        assert_eq!(d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).count(), 4, "four to the bar");
        let page: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_variation/mode" => Some(1.0),
            "sequence_generation/probability" => Some(100.0),
            "sequence_generation/note_range/low" => Some(60.0),
            "sequence_generation/note_range/high" => Some(64.0),
            _ => None,
        };
        d.apply_groove_page_with(0, 0, 0, 240, true, None, page, None, &mut Rng::seeded(7)).unwrap();
        let mut at: Vec<i64> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).map(|n| n.position).collect();
        at.sort_unstable();
        at.dedup();
        assert_eq!(at.len() as i64, bar / 240, "a note on every step: {at:?}");
    }

    /// Random makes notes for a sound that has none at all: the steps it has nothing on
    /// are exactly where the probability puts them, and Humanize still refuses (there is
    /// nothing to move).
    #[test]
    fn random_makes_notes_for_a_sound_with_none() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "P").unwrap();
        let bar = d.project().bar_length();
        let random: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_variation/mode" => Some(1.0),
            "sequence_generation/probability" => Some(100.0),
            "sequence_generation/note_range/low" => Some(60.0),
            "sequence_generation/note_range/high" => Some(64.0),
            _ => None,
        };
        d.apply_groove_page_with(0, 0, 0, 240, true, None, random, None, &mut Rng::seeded(2)).unwrap();
        let mut at: Vec<i64> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).map(|n| n.position).collect();
        at.sort_unstable();
        at.dedup();
        assert_eq!(at.len() as i64, bar / 240, "a note on every step: {at:?}");
        // Humanize on the same (now full) sound still refuses another sound with none.
        let humanize: &dyn Fn(&str) -> Option<f64> = &|f| (f == "sequence_variation/mode").then_some(0.0);
        let err = d.apply_variation_with(0, 0, 1, 240, None, humanize, None, &mut Rng::seeded(3)).unwrap_err();
        assert!(err.contains("no notes"), "{err}");
    }

    /// The Random engine reads the Variation page's slots where the page has set them:
    /// the velocity band, the note length and the time shift each do something, so no
    /// slot on the Jam's page is a number nobody reads. Unset, the older behaviour stands.
    #[test]
    fn random_generate_reads_the_variation_pages_settings() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "P").unwrap();
        let page: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_generation/mode" => Some(0.0),
            "sequence_generation/probability" => Some(100.0),
            "sequence_generation/note_range/low" => Some(60.0),
            "sequence_generation/note_range/high" => Some(64.0),
            "sequence_variation/velocity_range/low" => Some(70.0),
            "sequence_variation/velocity_range/high" => Some(80.0),
            "sequence_variation/steps" => Some(2.0),
            _ => None,
        };
        d.generate_with(0, 0, 0, 240, false, None, page, &mut Rng::seeded(5)).unwrap();
        let notes: Vec<Note> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect();
        assert!(!notes.is_empty());
        assert!(notes.iter().all(|n| (60..=64).contains(&n.pitch)), "{:?}", notes.iter().map(|n| n.pitch).collect::<Vec<_>>());
        assert!(notes.iter().all(|n| (70..=80).contains(&n.velocity)), "{:?}", notes.iter().map(|n| n.velocity).collect::<Vec<_>>());
        let bar = d.project().bar_length();
        assert!(
            notes.iter().all(|n| n.length == 480 || n.position + 480 > bar),
            "two steps long, cut at the pattern's end: {:?}",
            notes.iter().map(|n| n.length).collect::<Vec<_>>()
        );
        // The same call without the page's settings keeps the deviation and one step.
        d.generate_with(
            0,
            0,
            0,
            240,
            false,
            None,
            &|f: &str| (f == "sequence_generation/probability").then_some(100.0),
            &mut Rng::seeded(5),
        )
        .unwrap();
        let plain: Vec<Note> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect();
        assert!(plain.iter().all(|n| n.length == 240), "one step, as before");
        assert!(plain.iter().any(|n| n.velocity != 100), "the deviation around 100, as before");
    }

    /// One engine, two surfaces: with the same seed the Jam's Apply (mode Random) and the
    /// app's Generate button write the identical notes. Random output differs press to
    /// press by design, so a seed is the only way to say "same code" and mean it.
    #[test]
    fn the_jams_apply_and_the_generate_button_are_one_engine() {
        let page: &dyn Fn(&str) -> Option<f64> = &|f| match f {
            "sequence_variation/mode" => Some(1.0),
            "sequence_generation/probability" => Some(80.0),
            "sequence_generation/note_range/low" => Some(60.0),
            "sequence_generation/note_range/high" => Some(71.0),
            "sequence_generation/note_count" => Some(2.0),
            "sequence_variation/velocity_range/low" => Some(70.0),
            "sequence_variation/velocity_range/high" => Some(90.0),
            "sequence_variation/steps" => Some(1.0),
            _ => None,
        };
        let mut apply = Document::new_project().unwrap();
        apply.add_pattern(0, 0, "P").unwrap();
        apply.apply_groove_page_with(0, 0, 0, 240, true, None, page, None, &mut Rng::seeded(9)).unwrap();
        let mut button = Document::new_project().unwrap();
        button.add_pattern(0, 0, "P").unwrap();
        button
            .generate_with(0, 0, 0, 240, true, None, &|f: &str| (f == "sequence_generation/mode").then_some(0.0).or_else(|| page(f)), &mut Rng::seeded(9))
            .unwrap();
        let notes = |d: &Document| -> Vec<Note> {
            d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).copied().collect()
        };
        assert!(!notes(&apply).is_empty(), "something was made");
        assert_eq!(notes(&apply), notes(&button), "the Jam's Apply and the Generate button are the same call");
    }
}
