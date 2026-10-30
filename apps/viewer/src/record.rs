//! REC: notes played live (pads, MIDI, the computer keyboard) written into the
//! patterns playing (docs/superpowers/plans/2026-10-22-phase2a-recording-notes.md).
//! The engine notes each live note on and release with the song position it played at
//! ([`deckard_engine::Recorded`]); here they become notes in the group's pattern.

use deckard_doc::Document;
use deckard_engine::{Mode, Recorded, RecordedControl, RecordedExpr};
use deckard_model::expression::thin;
use deckard_model::{Curve, Dim, ExprKind, Expression, LaneRef, MidiEvent, MidiKind, Note, Project};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Where song position `tick` (in what `mode` plays) falls in group `g`'s pattern:
/// (the pattern, the position in it, from its start and wrapping at its length); `None`
/// when the group plays no pattern there.
pub fn pattern_at(p: &Project, mode: Mode, tick: f64, g: usize) -> Option<(usize, i64)> {
    let (pi, t) = match mode {
        Mode::Pattern(pg, pi) => (pg == g).then_some((pi, tick))?,
        Mode::Scene(s) => (p.scenes.get(s)?.patterns.iter().find(|x| x.0 == g)?.1, tick),
        Mode::Arrangement => {
            let si = p.section_at(tick as i64)?;
            let scene = p.sections[si].scene?;
            (p.scenes.get(scene)?.patterns.iter().find(|x| x.0 == g)?.1, tick - p.section_start(si) as f64)
        }
    };
    let pat = p.groups.get(g)?.patterns.get(pi)?;
    let len = pat.length.max(1);
    Some((pi, pat.start + (t.round() as i64).rem_euclid(len)))
}

/// A take being recorded: the notes held (by group, sound, pitch), each written into its
/// pattern at note-on and grown while held, as Maschine does
/// (docs/superpowers/specs/2026-10-25-recording-capture-design.md).
#[derive(Default)]
pub struct Take {
    held: HashMap<(usize, usize, i64), Held>,
}

/// How a played note is placed: Input Quantize's grid (ticks), if it's on.
#[derive(Debug, Clone, Copy, Default)]
pub struct Placing {
    pub quantize: Option<i64>,
}

/// An edit of group `g`'s pattern `pi` the take asks for: `remove` out, `add` in, with
/// `expr` (the expression played on it) put on `add` once the note is done.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub g: usize,
    pub pi: usize,
    pub remove: Vec<Note>,
    pub add: Note,
    pub expr: Option<Expression>,
}

struct Held {
    on: Recorded,
    pattern: usize,
    /// The note as it's written in the pattern now (or will be).
    note: Note,
    /// Snapped ahead of the song: not written yet; the clock time it's due, and the
    /// doubles it replaces then.
    due: Option<(f64, Vec<Note>)>,
    /// (ticks from the note's start, value) per [`Dim::ALL`].
    samples: [Vec<(i64, f32)>; 3],
    release: Option<u8>,
}

impl Held {
    /// The note ended at `played`: its length the time it was held.
    fn ended(self, played: f64) -> Edit {
        let mut e = Expression { release_velocity: self.release, ..Expression::default() };
        for (d, s) in Dim::ALL.into_iter().zip(&self.samples) {
            if !s.is_empty() {
                *e.curve_mut(d) = Some(Curve { points: thin(s, d.tolerance()) });
            }
        }
        let length = ((played - self.on.played).round() as i64).max(1);
        Edit {
            g: self.on.group,
            pi: self.pattern,
            remove: self.due.map_or(vec![self.note], |(_, doubles)| doubles),
            add: Note { length, ..self.note },
            expr: (!e.is_empty()).then_some(e),
        }
    }
}

impl Take {
    /// Take in a recorded on or release: the edits it makes. A note goes in at once, a
    /// placeholder long (a tick, a grid step quantized, a repeat note its own length),
    /// replacing notes of its sound and pitch starting in that span; a release sets
    /// the length it was held. A pad pressed again while held ends its earlier note.
    pub fn add(&mut self, p: &Project, r: Recorded, at: Placing) -> Vec<Edit> {
        let key = (r.group, r.sound, r.pitch);
        let mut out = Vec::new();
        let earlier = self.held.remove(&key);
        let skip = earlier.as_ref().map(|h| h.note);
        if let Some(h) = earlier {
            out.push(h.ended(r.played));
        }
        if r.velocity <= 0 {
            return out;
        }
        let Some((pattern, mut position)) = pattern_at(p, r.mode, r.tick, r.group) else { return out };
        let Some(pat) = p.groups.get(r.group).and_then(|g| g.patterns.get(pattern)) else { return out };
        // Played with its length (Note Repeat): as it is. Else snapped, then wrapped.
        let repeat = r.length != i64::MAX;
        let quantize = at.quantize.filter(|q| *q > 0 && !repeat);
        // How far ahead of where it was played the grid puts it (ticks).
        let mut ahead = 0;
        if let Some(q) = quantize {
            let raw = position - pat.start;
            let rel = (raw as f64 / q as f64).round() as i64 * q;
            ahead = rel - raw;
            position = pat.start + rel.rem_euclid(pat.length.max(1));
        }
        let length = if repeat { r.length.max(1) } else { quantize.unwrap_or(1) };
        let add = note(&r, position, length);
        let remove: Vec<Note> = pat
            .notes
            .iter()
            .filter(|n| n.sound == r.sound && n.pitch == r.pitch && (position..position + length).contains(&n.position))
            .filter(|n| Some(**n) != skip)
            .copied()
            .collect();
        // Ahead of the song: written when the song gets there, so the playing pattern
        // doesn't play it again just after the pad.
        let due = (ahead > 0 && !repeat).then(|| (r.played + ahead as f64, remove.clone()));
        if due.is_none() {
            out.push(Edit { g: r.group, pi: pattern, remove, add, expr: None });
        }
        if !repeat {
            self.held.insert(key, Held { on: r, pattern, note: add, due, samples: Default::default(), release: None });
        }
        out
    }

    /// Expression played on a held note, at its offset on the clock that never wraps;
    /// one from before the note, or before the last, is dropped.
    pub fn add_expr(&mut self, r: RecordedExpr) {
        let Some(h) = self.held.get_mut(&(r.group, r.sound, r.pitch)) else { return };
        let at = (r.played - h.on.played).round() as i64;
        match r.kind {
            ExprKind::Release => h.release = Some((r.value * 127.0).round().clamp(1.0, 127.0) as u8),
            ExprKind::Dim(d) => {
                let s = &mut h.samples[Dim::ALL.iter().position(|x| *x == d).unwrap_or(0)];
                if at >= 0 && s.last().is_none_or(|l| l.0 <= at) {
                    s.push((at, r.value));
                }
            }
        }
    }

    /// The clock at `played`: held notes grown to the time they've been held.
    pub fn grow(&mut self, played: f64) -> Vec<Edit> {
        let mut out = Vec::new();
        for h in self.held.values_mut() {
            let length = (played - h.on.played).round() as i64;
            if let Some((when, doubles)) = &h.due {
                if played < *when {
                    continue;
                }
                let add = Note { length: length.max(h.note.length), ..h.note };
                out.push(Edit { g: h.on.group, pi: h.pattern, remove: doubles.clone(), add, expr: None });
                (h.note, h.due) = (add, None);
            } else if length > h.note.length {
                let add = Note { length, ..h.note };
                out.push(Edit { g: h.on.group, pi: h.pattern, remove: vec![h.note], add, expr: None });
                h.note = add;
            }
        }
        out.sort_by_key(|e| (e.g, e.pi, e.add.position, e.add.sound, e.add.pitch));
        out
    }

    /// Whether notes are held.
    pub fn holding(&self) -> bool {
        !self.held.is_empty()
    }

    /// End the take at `played`: the notes still held end there.
    pub fn close(&mut self, played: f64) -> Vec<Edit> {
        let mut out: Vec<Edit> = self.held.drain().map(|(_, h)| h.ended(played)).collect();
        out.sort_by_key(|e| (e.g, e.pi, e.add.position, e.add.sound, e.add.pitch));
        out
    }
}

/// Everything the engine recorded, back in the order it was played.
pub enum Rec {
    Note(Recorded),
    Expr(RecordedExpr),
    Control(RecordedControl),
}

impl Rec {
    pub fn merged(notes: Vec<Recorded>, exprs: Vec<RecordedExpr>, controls: Vec<RecordedControl>) -> Vec<Rec> {
        let mut all: Vec<Rec> = notes.into_iter().map(Rec::Note).collect();
        all.extend(exprs.into_iter().map(Rec::Expr));
        all.extend(controls.into_iter().map(Rec::Control));
        all.sort_by_key(|r| match r {
            Rec::Note(x) => x.seq,
            Rec::Expr(x) => x.seq,
            Rec::Control(x) => x.seq,
        });
        all
    }
}

/// A controller that stops moving this long stops overwriting its lane (touch mode).
pub(crate) const IDLE: Duration = Duration::from_millis(250);

/// A lane being written is written into the document this often (each write is an
/// edit), and when it wraps, goes idle or recording stops.
const COMMIT: Duration = Duration::from_millis(100);

/// Controllers played while recording, written into their lanes over what was there:
/// while a controller moves, the lane is cleared as the playhead passes and its values
/// written in; idle for [`IDLE`], it leaves the rest. Each lane is (group, pattern,
/// sound, lane).
#[derive(Default)]
pub struct ControlWriter {
    lanes: HashMap<(usize, usize, usize, LaneRef), Writing>,
}

struct Writing {
    last: Instant,
    /// When it was last written into the document.
    written: Instant,
    /// Where in the pattern the playhead was last seen.
    seen: Option<i64>,
    /// The lane is cleared from where writing began up to here (a pattern position).
    cleared_to: i64,
    /// Values taken in since the lane was last written.
    pending: Vec<MidiEvent>,
}

fn lane_of(kind: MidiKind) -> LaneRef {
    match kind {
        MidiKind::Controller { number, .. } => LaneRef { kind: 4, number: number as i64 },
        MidiKind::PitchBend(_) => LaneRef { kind: 3, number: -1 },
        MidiKind::ChannelPressure(_) => LaneRef { kind: 6, number: -1 },
    }
}

impl ControlWriter {
    /// A controller value played: the lane goes into write mode (from here, if it
    /// wasn't).
    pub fn add(&mut self, doc: &mut Document, r: RecordedControl, now: Instant) {
        let Some((pi, pos)) = pattern_at(doc.project(), r.mode, r.tick, r.group) else { return };
        let lane = lane_of(r.kind);
        let key = (r.group, pi, r.sound, lane);
        if let Some(w) = self.lanes.get_mut(&key)
            && now.duration_since(w.last) >= IDLE
        {
            let end = w.pending.last().map_or(w.cleared_to, |e| e.position + 1);
            write(doc, key, w, end);
            self.lanes.remove(&key);
        }
        let w = self.lanes.entry(key).or_insert_with(|| {
            let _ = doc.add_lane(r.group, r.sound, lane);
            Writing { last: now, written: now, seen: None, cleared_to: pos, pending: Vec::new() }
        });
        w.last = now;
        w.pending.retain(|e| e.position != pos);
        w.pending.push(MidiEvent { sound: r.sound, position: pos, kind: r.kind });
    }

    /// The song at `tick` (what `mode` plays): lanes still being written are cleared up
    /// to the playhead and their values written; idle ones leave write mode.
    pub fn tick(&mut self, doc: &mut Document, mode: Mode, tick: f64, now: Instant) {
        let keys: Vec<_> = self.lanes.keys().copied().collect();
        for key in keys {
            let w = self.lanes.get_mut(&key).expect("a lane");
            let writing = now.duration_since(w.last) < IDLE;
            let at = pattern_at(doc.project(), mode, tick, key.0).filter(|x| x.0 == key.1).map(|x| x.1);
            let Some(at) = at.filter(|_| writing) else {
                let end = w.pending.last().map_or(w.cleared_to, |e| e.position + 1);
                write(doc, key, w, end);
                self.lanes.remove(&key);
                continue;
            };
            // Round the loop: written to the pattern's end, then on from its start (the
            // values played since the wrap are behind where it was cleared to).
            if w.seen.is_some_and(|s| at < s)
                && let Some(pat) = doc.project().groups.get(key.0).and_then(|x| x.patterns.get(key.1))
            {
                let (start, end) = (pat.start, pat.start + pat.length.max(1));
                let from = w.cleared_to;
                let (after, before): (Vec<MidiEvent>, Vec<MidiEvent>) = w.pending.drain(..).partition(|e| e.position < from);
                w.pending = before;
                write(doc, key, w, end);
                (w.cleared_to, w.pending, w.written) = (start, after, now);
            }
            w.seen = Some(at);
            if now.duration_since(w.written) >= COMMIT {
                write(doc, key, w, at);
                w.written = now;
            }
        }
    }

    /// Recording ended: what's taken in is written (cleared to where the playhead was
    /// last seen), and every lane leaves write mode.
    pub fn finish(&mut self, doc: &mut Document) {
        for (key, mut w) in std::mem::take(&mut self.lanes) {
            let end = w.seen.or(w.pending.last().map(|e| e.position + 1)).unwrap_or(w.cleared_to);
            write(doc, key, &mut w, end);
        }
    }
}

/// Clear lane `key` from `w.cleared_to` to `end` (wrapping at the pattern's loop end)
/// and write its pending values.
fn write(doc: &mut Document, (g, pi, sound, lane): (usize, usize, usize, LaneRef), w: &mut Writing, end: i64) {
    let Some(pat) = doc.project().groups.get(g).and_then(|x| x.patterns.get(pi)) else { return };
    let (start, len) = (pat.start, pat.length.max(1));
    let from = w.cleared_to;
    // Behind where it was cleared to: round the loop when that's most of it, else not
    // moved (a value from a moment ago).
    let end = if end < from && from - end <= len / 2 { from } else { end };
    let cleared = |p: i64| {
        if end >= from { p >= from && p < end } else { (p >= from && p < start + len) || (p >= start && p < end) }
    };
    let remove: Vec<MidiEvent> = pat
        .midi
        .iter()
        .filter(|e| e.sound == sound && lane_of(e.kind) == lane)
        .filter(|e| cleared(e.position) || w.pending.iter().any(|x| x.position == e.position))
        .copied()
        .collect();
    let add = std::mem::take(&mut w.pending);
    w.cleared_to = end;
    if (remove.is_empty() && add.is_empty()) || remove == add {
        return;
    }
    let _ = doc.edit_midi("Record", g, pi, &remove, &add);
}

/// Auto length (docs/superpowers/plans/2026-10-23-auto-length-recording.md): an empty
/// pattern recorded into starts as one bar, looping; a note in its last bar adds the
/// next; coming round to its start with the last bar empty, it settles on the bars
/// played and grows no more.
#[derive(Debug, Clone)]
pub struct Grow {
    pub group: usize,
    pub pattern: usize,
    /// Its length before.
    pub before: i64,
    bar: i64,
    bars: i64,
    /// The last bar a note was played in.
    played: Option<i64>,
    settled: bool,
    /// Where in the pattern the song was last seen.
    last: i64,
}

impl Grow {
    pub fn new(group: usize, pattern: usize, bar: i64) -> Grow {
        Grow { group, pattern, before: bar, bar: bar.max(1), bars: 1, played: None, settled: false, last: 0 }
    }

    pub fn length(&self) -> i64 {
        self.bars * self.bar
    }

    pub fn settled(&self) -> bool {
        self.settled
    }

    /// A note started `at` ticks into the pattern: its new length, if it grows.
    pub fn hit(&mut self, at: i64) -> Option<i64> {
        let b = at.max(0) / self.bar;
        self.played = Some(self.played.map_or(b, |p| p.max(b)));
        if self.settled || b + 1 < self.bars {
            return None;
        }
        self.bars = b + 2;
        Some(self.length())
    }

    /// The song `at` ticks into the pattern: its new length, if it settles.
    pub fn at(&mut self, at: i64) -> Option<i64> {
        let wrapped = at < self.last;
        self.last = at;
        let played = self.played.filter(|_| wrapped && !self.settled)?;
        self.settled = true;
        self.bars = played + 1;
        Some(self.length())
    }
}

fn note(on: &Recorded, position: i64, length: i64) -> Note {
    Note {
        expr: None,
        sound: on.sound,
        position,
        length: length.max(1),
        pitch: on.pitch,
        velocity: on.velocity.clamp(1, 127),
        probability: 100,
    }
}

#[cfg(test)]
mod grow_tests {
    use super::Grow;

    const BAR: i64 = 3840;

    /// Nothing played: one bar, looping.
    #[test]
    fn nothing_played_loops_a_bar() {
        let mut g = Grow::new(0, 0, BAR);
        assert_eq!(g.length(), BAR);
        for at in [0, 1000, 3000, 10, 2000] {
            assert_eq!(g.at(at), None);
        }
        assert_eq!(g.length(), BAR);
    }

    /// Bar 1 played, bar 2 not: it goes on into bar 2, then comes round to loop bar 1.
    #[test]
    fn an_empty_bar_settles_it() {
        let mut g = Grow::new(0, 0, BAR);
        assert_eq!(g.at(500), None);
        assert_eq!(g.hit(600), Some(2 * BAR), "the next bar added");
        assert_eq!(g.at(BAR + 100), None, "on into bar 2");
        assert_eq!(g.at(2 * BAR - 10), None);
        assert_eq!(g.at(20), Some(BAR), "round again, bar 2 empty: bar 1 only");
        assert!(g.settled());
        assert_eq!(g.hit(700), None, "settled: no more growing");
        assert_eq!(g.length(), BAR);
    }

    /// Each bar played into adds the next.
    #[test]
    fn each_bar_played_adds_the_next() {
        let mut g = Grow::new(0, 0, BAR);
        assert_eq!(g.hit(100), Some(2 * BAR));
        assert_eq!(g.at(BAR + 50), None);
        assert_eq!(g.hit(BAR + 200), Some(3 * BAR));
        assert_eq!(g.at(2 * BAR + 50), None);
        assert_eq!(g.hit(BAR + 900), None, "an earlier bar: no change");
        assert_eq!(g.at(10), Some(2 * BAR), "bar 3 empty: two bars");
    }

    /// Notes first played on a later pass of the one-bar loop still start it.
    #[test]
    fn the_first_note_may_come_late() {
        let mut g = Grow::new(0, 0, BAR);
        for at in [100, 3000, 50, 3500, 20] {
            assert_eq!(g.at(at), None);
        }
        assert_eq!(g.hit(2000), Some(2 * BAR));
    }
}

#[cfg(test)]
mod tests {
    use super::{Edit, Placing, Take, pattern_at};
    use deckard_engine::{Mode, Recorded};
    use deckard_model::Note;
    use deckard_model::{Pattern, Project, Scene, Section};

    /// Two groups; group 0's pattern 0 is 1 bar starting at tick 960, pattern 1 is 2 bars;
    /// scene 0 plays (0, 0), scene 1 plays (0, 1); the arrangement is scene 0 then 1.
    fn project() -> Project {
        let mut p = Project::default();
        let bar = p.bar_length();
        let mut g = deckard_model::Group::default();
        g.patterns = vec![
            Pattern { name: "A".into(), start: 960, length: bar, ..Pattern::default() },
            Pattern { name: "B".into(), start: 0, length: 2 * bar, ..Pattern::default() },
        ];
        p.groups = vec![g.clone(), g];
        p.scenes = vec![
            Scene { name: "1".into(), patterns: vec![(0, 0)], ..Scene::default() },
            Scene { name: "2".into(), patterns: vec![(0, 1)], ..Scene::default() },
        ];
        p.sections = vec![Section { scene: Some(0), length: bar, auto: true }, Section { scene: Some(1), length: 2 * bar, auto: true }];
        p
    }

    #[test]
    fn a_song_position_is_a_place_in_the_groups_pattern() {
        let p = project();
        let bar = p.bar_length();
        // Scene 0: pattern 0 loops each bar, from its start (960).
        assert_eq!(pattern_at(&p, Mode::Scene(0), 100.0, 0), Some((0, 960 + 100)));
        assert_eq!(pattern_at(&p, Mode::Scene(0), (bar + 100) as f64, 0), Some((0, 960 + 100)));
        // A group with nothing in the scene records nowhere.
        assert_eq!(pattern_at(&p, Mode::Scene(0), 100.0, 1), None);
        // The pattern played alone.
        assert_eq!(pattern_at(&p, Mode::Pattern(0, 1), 300.0, 0), Some((1, 300)));
        assert_eq!(pattern_at(&p, Mode::Pattern(0, 1), 300.0, 1), None);
        // The arrangement: the second section (scene 1, pattern 1) from its start.
        assert_eq!(pattern_at(&p, Mode::Arrangement, (bar + 50) as f64, 0), Some((1, 50)));
    }

    fn on(sound: usize, pitch: i64, tick: f64, played: f64) -> Recorded {
        Recorded { group: 0, sound, pitch, velocity: 90, length: i64::MAX, tick, mode: Mode::Scene(0), seq: 0, played }
    }

    fn off(sound: usize, pitch: i64, tick: f64, played: f64) -> Recorded {
        Recorded { velocity: 0, length: 0, ..on(sound, pitch, tick, played) }
    }

    const FREE: Placing = Placing { quantize: None };

    fn note(sound: usize, pitch: i64, position: i64, length: i64) -> Note {
        Note { sound, pitch, position, length, velocity: 90, probability: 100, expr: None }
    }

    /// Pattern 0 of group 0 (1 bar from tick 960) holding `notes`.
    fn with_notes(notes: Vec<Note>) -> Project {
        let mut p = project();
        p.groups[0].patterns[0].notes = notes;
        p
    }

    #[test]
    fn a_note_is_written_at_note_on() {
        let p = project();
        let mut take = Take::default();
        let e = take.add(&p, on(3, 62, 120.0, 500.0), FREE);
        assert_eq!(e, vec![Edit { g: 0, pi: 0, remove: vec![], add: note(3, 62, 960 + 120, 1), expr: None }]);
    }

    #[test]
    fn a_held_note_grows_and_its_release_sets_its_length() {
        let p = project();
        let mut take = Take::default();
        take.add(&p, on(3, 62, 120.0, 500.0), FREE);
        let g = take.grow(800.0);
        assert_eq!(g.len(), 1);
        assert_eq!((g[0].remove.clone(), g[0].add), (vec![note(3, 62, 1080, 1)], note(3, 62, 1080, 300)));
        assert!(take.grow(700.0).is_empty(), "never shrinks");
        let r = take.add(&p, off(3, 62, 600.0, 980.0), FREE);
        assert_eq!((r[0].remove.clone(), r[0].add), (vec![note(3, 62, 1080, 300)], note(3, 62, 1080, 480)));
        assert!(take.grow(5000.0).is_empty(), "released");
    }

    /// The user's chords: held over the loop end, a note keeps growing past it.
    #[test]
    fn a_note_held_over_the_loop_end_keeps_growing() {
        let p = project();
        let bar = p.bar_length() as f64;
        let mut take = Take::default();
        take.add(&p, on(1, 60, 0.0, 100.0), FREE);
        // Released a bar and a half later: the song wrapped, the clock didn't.
        let r = take.add(&p, off(1, 60, bar * 0.5, 100.0 + bar * 1.5), FREE);
        assert_eq!(r[0].add, note(1, 60, 960, (bar * 1.5) as i64));
    }

    #[test]
    fn expression_over_the_wrap() {
        use deckard_engine::RecordedExpr;
        use deckard_model::{Dim, ExprKind};
        let p = project();
        let bar = p.bar_length() as f64;
        let mut take = Take::default();
        take.add(&p, on(1, 60, bar - 100.0, 1000.0), FREE);
        let x = |tick: f64, played: f64, value: f32| RecordedExpr {
            group: 0,
            sound: 1,
            pitch: 60,
            kind: ExprKind::Dim(Dim::Pressure),
            value,
            tick,
            mode: Mode::Scene(0),
            seq: 0,
            played,
        };
        take.add_expr(x(bar - 50.0, 1050.0, 0.2));
        take.add_expr(x(50.0, 1150.0, 0.8));
        let r = take.add(&p, off(1, 60, 100.0, 1200.0), FREE);
        let pts = r[0].expr.as_ref().unwrap().pressure.as_ref().unwrap().points.clone();
        assert_eq!(pts.iter().map(|p| (p.at, p.value)).collect::<Vec<_>>(), vec![(50, 0.2), (150, 0.8)]);
    }

    #[test]
    fn quantized_onto_the_end_wraps_to_the_start() {
        let p = project();
        let bar = p.bar_length() as f64;
        let mut take = Take::default();
        assert!(take.add(&p, on(2, 60, bar - 20.0, 0.0), Placing { quantize: Some(240) }).is_empty(), "due when the song gets there");
        let e = take.grow(20.0);
        assert_eq!(e[0].add, note(2, 60, 960, 240), "on step 1, a step long while held");
        let e = take.add(&p, on(3, 60, 100.0, 100.0), Placing { quantize: Some(240) });
        assert_eq!(e[0].add.position, 960, "nearest step, behind: at once");
    }

    /// A pad hit twice on one step (quantized) leaves one note; other pitches and sounds
    /// stay.
    #[test]
    fn doubles_are_removed() {
        let kept = [note(2, 61, 960, 100), note(4, 60, 960, 100), note(2, 60, 960 + 480, 100)];
        let p = with_notes([vec![note(2, 60, 960 + 10, 100)], kept.to_vec()].concat());
        let mut take = Take::default();
        let e = take.add(&p, on(2, 60, 5.0, 0.0), Placing { quantize: Some(240) });
        assert_eq!(e[0].remove, vec![note(2, 60, 970, 100)]);
        let e = take.add(&p, on(2, 61, 300.0, 0.0), FREE);
        assert!(e[0].remove.is_empty(), "unquantized: only one on the very tick");
    }

    #[test]
    fn the_same_pad_again_ends_the_earlier_note() {
        let p = project();
        let mut take = Take::default();
        take.add(&p, on(1, 60, 0.0, 0.0), FREE);
        let e = take.add(&p, on(1, 60, 200.0, 200.0), FREE);
        assert_eq!(e.len(), 2, "{e:?}");
        assert_eq!(e[0].add, note(1, 60, 960, 200), "the first, ended");
        assert_eq!(e[1].add, note(1, 60, 960 + 200, 1), "the second, started");
    }

    /// Note Repeat's notes come with their length: written at once, not input-quantized.
    #[test]
    fn repeat_notes_are_not_quantized() {
        let p = project();
        let mut take = Take::default();
        let hit = Recorded { length: 120, ..on(2, 60, 50.0, 0.0) };
        let e = take.add(&p, hit, Placing { quantize: Some(240) });
        assert_eq!(e[0].add, note(2, 60, 960 + 50, 120));
        assert!(take.grow(9999.0).is_empty(), "not held");
    }

    #[test]
    fn notes_held_at_the_end_keep_their_length() {
        let p = project();
        let mut take = Take::default();
        take.add(&p, on(1, 60, 0.0, 0.0), FREE);
        take.add(&p, on(2, 64, 0.0, 0.0), FREE);
        let c = take.close(300.0);
        assert_eq!(c.iter().map(|e| e.add.length).collect::<Vec<_>>(), vec![300, 300]);
        assert!(take.close(400.0).is_empty());
    }

    /// Played early and snapped later by Input Quantize: written only once the song
    /// reaches it, so the playing pattern doesn't play it again just after the pad.
    #[test]
    fn a_note_snapped_ahead_waits_for_the_song() {
        let p = with_notes(vec![note(2, 60, 960 + 240, 50)]);
        let mut take = Take::default();
        let e = take.add(&p, on(2, 60, 230.0, 1000.0), Placing { quantize: Some(240) });
        assert!(e.is_empty(), "not yet: {e:?}");
        assert!(take.grow(1005.0).is_empty(), "still ahead");
        let e = take.grow(1012.0);
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].remove.clone(), e[0].add), (vec![note(2, 60, 1200, 50)], note(2, 60, 1200, 240)), "in, replacing the double");
        // Released before it was due: written once, finished.
        let e = take.add(&p, on(3, 60, 235.0, 2000.0), Placing { quantize: Some(240) });
        assert!(e.is_empty());
        let e = take.add(&p, off(3, 60, 236.0, 2001.0), Placing { quantize: Some(240) });
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].remove.clone(), e[0].add), (vec![], note(3, 60, 1200, 1)));
    }
}
