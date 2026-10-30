//! The DECKARD block: what DECKARD keeps in a project that Maschine has no place for,
//! saved as a root-level `DKRD`/`EXPR` item of the project file (Maschine loads a
//! project with it and ignores it). Format: `docs/deckard-block.md`.

use super::Document;
use super::expression::set_expr_field;
use deckard_model::pad_touch::PadTouch;
use deckard_model::{Curve, Dim, ExprId, Expression, MidiKind, Note, Point, Project};
use mxprj::decode::{Obj, Value};

pub(crate) const DOMAIN: [u8; 4] = *b"DKRD";
pub(crate) const ITEM: [u8; 4] = *b"EXPR";
pub(crate) const FRAME: u32 = u32::from_be_bytes(*b"DKRD");
/// The payload version written; versions 1 (before Pad Touch), 2 (before modulation
/// routes), 3 (before PROB LOCK), 4 (before the screen), 5 (before LFOs) and 6 (before
/// CC routes) still read.
const VERSION: u32 = 7;

/// Where the project was left when saved: the focus, and the controller's page and pad
/// mode, so it reopens there (BUGS #6). Not part of the song: setting it is no edit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SavedScreen {
    pub group: usize,
    pub sound: usize,
    pub scene: usize,
    pub pattern: usize,
    /// The focused plug-in slot.
    pub slot: Option<usize>,
    /// The Channel page shown rather than the plug-ins.
    pub channel: bool,
    /// The controller's page and pad mode (DECKARD's layer).
    pub page: String,
    pub mode: String,
}

/// A note's expression as the block keys it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    pub sound: usize,
    pub position: i64,
    pub pitch: i64,
    /// Among the pattern's notes with this sound, position and pitch, in note order.
    pub ordinal: usize,
    pub expr: Expression,
}

/// A sound's channel-pressure events in a pattern.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PressureLane {
    pub group: usize,
    pub pattern: usize,
    pub sound: usize,
    pub events: Vec<(i64, u8)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Block {
    /// Per pattern (group, pattern; clips as `CLIP + k`).
    pub patterns: Vec<(usize, usize, Vec<Entry>)>,
    pub pressure: Vec<PressureLane>,
    /// Sounds' Pad Touch (group, sound), those not at the default.
    pub touch: Vec<(usize, usize, PadTouch)>,
    /// Sounds' modulation routes (group, sound, as text), those with any (version 3).
    pub routes: Vec<(usize, usize, String)>,
    /// The sounds (group, sound) with PROB LOCK on (version 4).
    pub prob_locked: Vec<(usize, usize)>,
    /// Where the project was left (version 5).
    pub screen: Option<SavedScreen>,
    /// The project's LFOs as [`deckard_model::lfo::to_text`] (version 6).
    pub lfos: String,
    /// The project's CC routes as [`deckard_model::mod_matrix::cc_to_text`] (version 7).
    pub cc_routes: String,
}

/// Every pattern of the project with its index (clips as `CLIP + k`).
fn patterns(p: &Project) -> impl Iterator<Item = (usize, usize, &deckard_model::Pattern)> {
    p.groups.iter().enumerate().flat_map(|(g, grp)| {
        grp.patterns
            .iter()
            .enumerate()
            .map(move |(i, pt)| (g, i, pt))
            .chain(grp.clip_patterns.iter().enumerate().map(move |(k, pt)| (g, crate::CLIP + k, pt)))
    })
}

/// Each note with its ordinal among the notes before it with the same key.
fn with_ordinals(notes: &[Note]) -> Vec<(usize, &Note)> {
    notes
        .iter()
        .enumerate()
        .map(|(i, n)| (notes[..i].iter().filter(|m| (m.sound, m.position, m.pitch) == (n.sound, n.position, n.pitch)).count(), n))
        .collect()
}

/// `c` without what lies past `len` (a value at `len` where it was cut).
fn clipped(c: &Curve, len: i64) -> Curve {
    let mut points: Vec<Point> = c.points.iter().filter(|p| p.at <= len).copied().collect();
    if c.points.iter().any(|p| p.at > len) && points.last().is_none_or(|p| p.at < len) {
        points.push(Point { at: len, value: c.value_at(len), curve: 0.0 });
    }
    Curve { points }
}

/// The block for `p`: its notes' expression (`exprs` looks it up) and channel pressure.
pub(crate) fn block_of(p: &Project, exprs: &dyn Fn(ExprId) -> Option<Expression>) -> Block {
    let mut b = Block::default();
    for (g, i, pt) in patterns(p) {
        let entries: Vec<Entry> = with_ordinals(&pt.notes)
            .into_iter()
            .filter_map(|(ordinal, n)| {
                let mut expr = exprs(n.expr?)?;
                for d in Dim::ALL {
                    if let Some(c) = expr.curve_mut(d) {
                        *c = clipped(c, n.length.max(1));
                    }
                }
                (!expr.is_empty()).then_some(Entry { sound: n.sound, position: n.position, pitch: n.pitch, ordinal, expr })
            })
            .collect();
        if !entries.is_empty() {
            b.patterns.push((g, i, entries));
        }
        let mut lanes: Vec<PressureLane> = Vec::new();
        for m in &pt.midi {
            let MidiKind::ChannelPressure(v) = m.kind else { continue };
            match lanes.iter_mut().find(|l| l.sound == m.sound) {
                Some(l) => l.events.push((m.position, v)),
                None => lanes.push(PressureLane { group: g, pattern: i, sound: m.sound, events: vec![(m.position, v)] }),
            }
        }
        b.pressure.extend(lanes);
    }
    b
}

// ---- bytes --------------------------------------------------------------------------

#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: usize) {
        self.0.extend_from_slice(&(v as u32).to_le_bytes());
    }
    fn i32(&mut self, v: i64) {
        self.0.extend_from_slice(&(v as i32).to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
}

struct R<'a>(&'a [u8]);

impl R<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        if self.0.len() < N {
            return Err("the DECKARD block ends early".into());
        }
        let (a, rest) = self.0.split_at(N);
        self.0 = rest;
        Ok(a.try_into().unwrap())
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take::<1>()?[0])
    }
    fn u32(&mut self) -> Result<usize, String> {
        Ok(u32::from_le_bytes(self.take()?) as usize)
    }
    /// A count of things at least `each` bytes long: no more than the bytes left.
    fn count(&mut self, each: usize) -> Result<usize, String> {
        let n = self.u32()?;
        if n.saturating_mul(each) > self.0.len() {
            return Err("a count in the DECKARD block is too large".into());
        }
        Ok(n)
    }
    fn i32(&mut self) -> Result<i64, String> {
        Ok(i32::from_le_bytes(self.take()?) as i64)
    }
    fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_le_bytes(self.take()?))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take()?))
    }
}

pub(crate) fn encode(b: &Block) -> Vec<u8> {
    let mut w = W::default();
    w.u32(VERSION as usize);
    w.u32(b.patterns.len());
    for (g, p, entries) in &b.patterns {
        w.u32(*g);
        w.u32(*p);
        w.u32(entries.len());
        for e in entries {
            w.u32(e.sound);
            w.i64(e.position);
            w.i32(e.pitch);
            w.u32(e.ordinal);
            w.u8(e.expr.release_velocity.unwrap_or(0));
            let mask = Dim::ALL.iter().enumerate().filter(|(_, d)| e.expr.curve(**d).is_some()).fold(0u8, |m, (i, _)| m | 1 << i);
            w.u8(mask);
            for c in Dim::ALL.iter().filter_map(|d| e.expr.curve(*d)) {
                w.u32(c.points.len());
                for p in &c.points {
                    w.i64(p.at);
                    w.f32(p.value);
                    w.f32(p.curve);
                }
            }
        }
    }
    w.u32(b.pressure.len());
    for l in &b.pressure {
        w.u32(l.group);
        w.u32(l.pattern);
        w.u32(l.sound);
        w.u32(l.events.len());
        for &(at, v) in &l.events {
            w.i64(at);
            w.u8(v);
        }
    }
    w.u32(b.touch.len());
    for (g, s, t) in &b.touch {
        w.u32(*g);
        w.u32(*s);
        for v in t.values() {
            w.u8(v as u8);
        }
    }
    w.u32(b.routes.len());
    for (g, s, text) in &b.routes {
        w.u32(*g);
        w.u32(*s);
        w.u32(text.len());
        for &c in text.as_bytes() {
            w.u8(c);
        }
    }
    w.u32(b.prob_locked.len());
    for (g, s) in &b.prob_locked {
        w.u32(*g);
        w.u32(*s);
    }
    w.u8(b.screen.is_some() as u8);
    if let Some(s) = &b.screen {
        for v in [s.group, s.sound, s.scene, s.pattern] {
            w.u32(v);
        }
        w.u32(s.slot.map_or(0, |i| i + 1));
        w.u8(s.channel as u8);
        for text in [&s.page, &s.mode] {
            w.u32(text.len());
            for &c in text.as_bytes() {
                w.u8(c);
            }
        }
    }
    for text in [&b.lfos, &b.cc_routes] {
        w.u32(text.len());
        for &c in text.as_bytes() {
            w.u8(c);
        }
    }
    w.0
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Block, String> {
    let mut r = R(bytes);
    let version = r.u32()?;
    if !(1..=VERSION as usize).contains(&version) {
        return Err(format!("DECKARD block version {version} (this DECKARD reads {VERSION})"));
    }
    let mut b = Block::default();
    for _ in 0..r.count(12)? {
        let (g, p) = (r.u32()?, r.u32()?);
        let mut entries = Vec::new();
        for _ in 0..r.count(22)? {
            let (sound, position, pitch, ordinal) = (r.u32()?, r.i64()?, r.i32()?, r.u32()?);
            let release = r.u8()?;
            let mask = r.u8()?;
            let mut expr = Expression { release_velocity: (release > 0).then_some(release.min(127)), ..Expression::default() };
            for (i, d) in Dim::ALL.iter().enumerate() {
                if mask & 1 << i == 0 {
                    continue;
                }
                let mut points = Vec::new();
                for _ in 0..r.count(16)? {
                    points.push(Point { at: r.i64()?, value: r.f32()?, curve: r.f32()? });
                }
                *expr.curve_mut(*d) = Some(Curve { points });
            }
            entries.push(Entry { sound, position, pitch, ordinal, expr });
        }
        b.patterns.push((g, p, entries));
    }
    for _ in 0..r.count(16)? {
        let (group, pattern, sound) = (r.u32()?, r.u32()?, r.u32()?);
        let mut events = Vec::new();
        for _ in 0..r.count(9)? {
            events.push((r.i64()?, r.u8()?));
        }
        b.pressure.push(PressureLane { group, pattern, sound, events });
    }
    if version >= 2 {
        for _ in 0..r.count(13)? {
            let (g, s) = (r.u32()?, r.u32()?);
            let mut v = [None; 5];
            for x in &mut v {
                *x = Some(r.u8()? as i64);
            }
            b.touch.push((g, s, PadTouch::from_values(v)));
        }
    }
    if version >= 3 {
        for _ in 0..r.count(12)? {
            let (g, s) = (r.u32()?, r.u32()?);
            let mut text = Vec::new();
            for _ in 0..r.count(1)? {
                text.push(r.u8()?);
            }
            b.routes.push((g, s, String::from_utf8_lossy(&text).into_owned()));
        }
    }
    if version >= 4 {
        for _ in 0..r.count(8)? {
            b.prob_locked.push((r.u32()?, r.u32()?));
        }
    }
    if version >= 5 && r.u8()? != 0 {
        let (group, sound, scene, pattern) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
        let slot = r.u32()?.checked_sub(1);
        let channel = r.u8()? != 0;
        let mut text = || -> Result<String, String> {
            let mut t = Vec::new();
            for _ in 0..r.count(1)? {
                t.push(r.u8()?);
            }
            Ok(String::from_utf8_lossy(&t).into_owned())
        };
        let (page, mode) = (text()?, text()?);
        b.screen = Some(SavedScreen { group, sound, scene, pattern, slot, channel, page, mode });
    }
    if version >= 6 {
        let mut text = Vec::new();
        for _ in 0..r.count(1)? {
            text.push(r.u8()?);
        }
        b.lfos = String::from_utf8_lossy(&text).into_owned();
    }
    if version >= 7 {
        let mut text = Vec::new();
        for _ in 0..r.count(1)? {
            text.push(r.u8()?);
        }
        b.cc_routes = String::from_utf8_lossy(&text).into_owned();
    }
    Ok(b)
}

/// Marks the `TimedEvent` of channel pressure DECKARD made; Maschine's own (read from
/// its archive) has none and stays there.
const MADE: &str = "deckard:made";

/// Mark (or unmark) a new `TimedEvent` as channel pressure DECKARD made.
pub(crate) fn mark_made(te: &mut Obj, made: bool) {
    te.fields.retain(|(f, _)| f != MADE);
    if made {
        te.fields.push((MADE.into(), Value::Int(1)));
    }
}

fn is_made(te: &Obj) -> bool {
    te.fields.iter().any(|(f, _)| f == MADE)
}

/// Take the channel-pressure events DECKARD made (inline `MidiEvent`s of kind 6,
/// marked) out of every sequence in `v`: they go in the block instead.
pub(crate) fn strip_channel_pressure(v: &mut Value) {
    const SEQUENCE: &str = "NI::MASCOMMON::DATA::Sequence";
    v.walk_mut(&mut |o| {
        if o.type_name != SEQUENCE {
            return;
        }
        let Some(Value::Obj(events)) = o.get("events") else { return };
        let Ok(items) = super::notes::items_of(events) else { return };
        let pressure = |it: &super::notes::Item| match &it.event {
            Value::Obj(te) if is_made(te) => is_pressure(te),
            _ => false,
        };
        if !items.iter().any(pressure) {
            return;
        }
        let gone: Vec<u64> = items
            .iter()
            .filter(|it| pressure(it))
            .filter_map(|it| match &it.event {
                Value::Obj(te) => match te.get("event") {
                    Some(Value::Obj(ev)) => ev.oid,
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let kept: Vec<super::notes::Item> = items.into_iter().filter(|it| !pressure(it)).collect();
        let events = super::notes::events_with(events, &kept);
        *o.get_mut("events").expect("events") = Value::Obj(events);
        if let Some(cur) = o.get_mut("current")
            && matches!(cur, Value::Ref(id) if gone.contains(id))
        {
            *cur = Value::Null;
        }
    });
}

fn is_pressure(te: &Obj) -> bool {
    match te.get("event") {
        Some(Value::Obj(ev)) => ev.type_name == "NI::MASCHINE::DATA::MidiEvent" && ev.get("i14").and_then(Value::as_int) == Some(6),
        _ => false,
    }
}

impl Document {
    /// Only the events of `lanes` DECKARD made: Maschine's own stay in its archive.
    fn made_pressure(&self, lanes: Vec<PressureLane>) -> Vec<PressureLane> {
        if lanes.is_empty() {
            return lanes;
        }
        let paths = self.object_paths();
        lanes
            .into_iter()
            .filter_map(|mut l| {
                let seq = self.sequence_of(l.group, l.pattern, l.sound, &paths).ok()?.2?;
                let Some(Value::Obj(seq)) = self.tree.at(&seq) else { return None };
                let Some(Value::Obj(events)) = seq.get("events") else { return None };
                let mut made: Vec<(i64, u8)> = super::notes::items_of(events)
                    .ok()?
                    .iter()
                    .filter_map(|it| match &it.event {
                        Value::Obj(te) if is_made(te) && is_pressure(te) => {
                            let Some(Value::Obj(ev)) = te.get("event") else { return None };
                            Some((te.get("position")?.as_int()?, ev.get("i20")?.as_int()?.clamp(0, 127) as u8))
                        }
                        _ => None,
                    })
                    .collect();
                l.events.retain(|e| match made.iter().position(|m| m == e) {
                    Some(i) => {
                        made.swap_remove(i);
                        true
                    }
                    None => false,
                });
                (!l.events.is_empty()).then_some(l)
            })
            .collect()
    }

    /// Save channel pressure in Maschine's archive (kind-6 MIDI events) rather than the
    /// DECKARD block. For checking Maschine reads it; off by default.
    pub fn set_native_pressure(&mut self, on: bool) {
        self.native_pressure = on;
    }

    /// The file bytes `archived` (Maschine's part, saved) with this document's DECKARD
    /// block, or without one if there's nothing to keep.
    pub(crate) fn with_block(&self, archived: &[u8]) -> Result<Vec<u8>, String> {
        let mut b = block_of(&self.project, &|id| self.expression(id).cloned());
        b.pressure = if self.native_pressure { Vec::new() } else { self.made_pressure(std::mem::take(&mut b.pressure)) };
        b.touch = self
            .project
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, grp)| {
                grp.sounds.iter().enumerate().filter(|(_, s)| !s.pad_touch.is_default()).map(move |(s, snd)| (g, s, snd.pad_touch))
            })
            .collect();
        b.routes = self
            .project
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, grp)| {
                grp.sounds
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !s.mod_routes.is_empty())
                    .map(move |(s, snd)| (g, s, deckard_model::mod_matrix::to_text(&snd.mod_routes)))
            })
            .collect();
        b.prob_locked = self
            .project
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, grp)| grp.sounds.iter().enumerate().filter(|(_, s)| s.probability_locked).map(move |(s, _)| (g, s)))
            .collect();
        b.screen = self.screen.clone();
        b.lfos = deckard_model::lfo::to_text(&self.project.lfos);
        b.cc_routes = deckard_model::mod_matrix::cc_to_text(&self.project.cc_routes);
        let payload = (!b.patterns.is_empty()
            || !b.pressure.is_empty()
            || !b.touch.is_empty()
            || !b.routes.is_empty()
            || !b.prob_locked.is_empty()
            || b.screen.is_some()
            || !b.lfos.is_empty()
            || !b.cc_routes.is_empty())
        .then(|| encode(&b));
        mxprj::container::set_root_child(archived, DOMAIN, ITEM, FRAME, payload.as_deref())
    }

    /// Where the project was left, as saved ([`SavedScreen`]).
    pub fn screen(&self) -> Option<&SavedScreen> {
        self.screen.as_ref()
    }

    /// Keep where the project is left, for the next save (no edit).
    pub fn set_screen(&mut self, screen: Option<SavedScreen>) {
        self.screen = screen;
    }

    /// Take in the DECKARD block of the file the document was read from (at open: not an
    /// undo step). A block that doesn't read is left out, and the project opens anyway.
    pub(crate) fn read_block(&mut self) {
        let Some(bytes) = mxprj::container::root_child(&self.file, DOMAIN, ITEM) else { return };
        let b = match decode(&bytes) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("deckard: the project's DECKARD block was left out: {e}");
                return;
            }
        };
        // Setting a field moves no object: the paths hold for every note.
        let paths = self.object_paths();
        for (g, p, entries) in b.patterns {
            for e in entries {
                let id = self.add_expression(e.expr.clone());
                if self.set_note_expr(g, p, &e, id, &paths).is_err() {
                    // Its note isn't there: dropped.
                }
            }
        }
        for l in b.pressure {
            // Not those Maschine's own data has already (read as channel pressure too).
            let have = self.project.groups.get(l.group).and_then(|g| {
                if l.pattern >= crate::CLIP { g.clip_patterns.get(l.pattern - crate::CLIP) } else { g.patterns.get(l.pattern) }
            });
            let have: Vec<deckard_model::MidiEvent> = have.map(|p| p.midi.clone()).unwrap_or_default();
            let events: Vec<deckard_model::MidiEvent> = l
                .events
                .iter()
                .map(|&(position, v)| deckard_model::MidiEvent { sound: l.sound, position, kind: MidiKind::ChannelPressure(v) })
                .filter(|m| !have.contains(m))
                .collect();
            if let Ok(ops) = self.event_ops(l.group, l.pattern, &[], &[], &[], &events) {
                let _ = self.run(&ops);
            }
        }
        for (g, s, t) in b.touch {
            // A sound that isn't there: dropped.
            if let Ok(ops) = self.pad_touch_ops(g, s, t) {
                let _ = self.run(&ops);
            }
        }
        for (g, s, text) in b.routes {
            if let Ok(ops) = self.mod_route_ops(g, s, &deckard_model::mod_matrix::from_text(&text)) {
                let _ = self.run(&ops);
            }
        }
        for (g, s) in b.prob_locked {
            if let Ok(ops) = self.prob_lock_ops(g, s, true) {
                let _ = self.run(&ops);
            }
        }
        let ops = self.lfo_ops(&deckard_model::lfo::from_text(&b.lfos));
        let _ = self.run(&ops);
        let ops = self.cc_route_ops(&deckard_model::mod_matrix::cc_from_text(&b.cc_routes));
        let _ = self.run(&ops);
        self.screen = b.screen;
        self.refresh();
        self.revision = 0;
    }

    /// Give the note the block's `e` names its expression id, straight in the tree.
    fn set_note_expr(
        &mut self,
        g: usize,
        p: usize,
        e: &Entry,
        id: ExprId,
        paths: &std::collections::HashMap<u64, mxprj::decode::Path>,
    ) -> Result<(), String> {
        let (_, _, seq) = self.sequence_of(g, p, e.sound, paths)?;
        let events_path = self.field_path(&seq.ok_or("no notes")?, &["events"])?;
        let Some(Value::Obj(events)) = self.tree.at(&events_path).cloned() else { return Err("no events".into()) };
        let mut items = super::notes::items_of(&events)?;
        let mut seen = 0;
        let mut target = None;
        for (i, it) in items.iter().enumerate() {
            let is_it = it.position() == e.position
                && self
                    .event_obj(&it.event, paths)
                    .is_some_and(|ev| ev.type_name.ends_with("NoteEvent") && ev.find("note").and_then(Value::as_int) == Some(e.pitch));
            if is_it {
                if seen == e.ordinal {
                    target = Some(i);
                    break;
                }
                seen += 1;
            }
        }
        let i = target.ok_or("no such note")?;
        let Value::Obj(te) = &mut items[i].event else { return Err("not an event".into()) };
        set_expr_field(te, Some(id));
        let events = super::notes::events_with(&events, &items);
        *self.tree.at_mut(&events_path).ok_or("events")? = Value::Obj(events);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::lfo::Lfo;
    use mxprj::container::{root_child, set_root_child};

    fn curve(points: &[(i64, f32)]) -> Curve {
        Curve { points: points.iter().map(|&(at, value)| Point { at, value, curve: 0.0 }).collect() }
    }

    fn full() -> Expression {
        Expression {
            pitch: Some(curve(&[(0, 0.0), (480, 2.0)])),
            slide: Some(curve(&[(0, 0.5)])),
            pressure: Some(curve(&[(0, 0.1), (240, 0.9), (960, 0.3)])),
            release_velocity: Some(64),
        }
    }

    fn note(position: i64, expr: Option<ExprId>) -> Note {
        Note { sound: 0, position, length: 960, pitch: 60, velocity: 100, probability: 100, expr }
    }

    fn doc_with(exprs: &[Expression]) -> Document {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "Pattern 1").unwrap();
        let notes: Vec<Note> = exprs.iter().map(|e| note(0, Some(d.add_expression(e.clone())))).collect();
        d.edit_notes("t", 0, 0, &[], &notes).unwrap();
        d
    }

    fn reopen(d: &Document) -> Document {
        Document::from_bytes(d.to_bytes().unwrap(), None).unwrap()
    }

    fn exprs_of(d: &Document) -> Vec<Option<Expression>> {
        d.project().groups[0].patterns[0].notes.iter().map(|n| n.expr.and_then(|i| d.expression(i)).cloned()).collect()
    }

    /// BUGS #6: where the project was left (focus, the controller's page) comes back.
    #[test]
    fn the_screen_round_trips_through_a_save() {
        let mut d = doc_with(&[]);
        let screen = SavedScreen {
            group: 0,
            sound: 3,
            scene: 2,
            pattern: 4,
            slot: Some(1),
            channel: true,
            page: "mixer".into(),
            mode: "keyboard".into(),
        };
        d.set_screen(Some(screen.clone()));
        assert_eq!(reopen(&d).screen(), Some(&screen));
        assert_eq!(reopen(&doc_with(&[])).screen(), None);
    }

    #[test]
    fn version_4_blocks_still_read() {
        let b = Block { prob_locked: vec![(0, 1)], ..Block::default() };
        let mut bytes = encode(&b);
        // Version 4: no screen or LFOs at the end.
        bytes[..4].copy_from_slice(&4u32.to_le_bytes());
        bytes.truncate(bytes.len() - 9);
        assert_eq!(decode(&bytes).unwrap(), b);
    }

    #[test]
    fn version_5_blocks_still_read() {
        let b = Block { prob_locked: vec![(0, 1)], ..Block::default() };
        let full = Block { lfos: "lfo".into(), ..b.clone() };
        let mut bytes = encode(&full);
        // Version 5: no LFO text (a length and 3 bytes) at the end.
        bytes[..4].copy_from_slice(&5u32.to_le_bytes());
        bytes.truncate(bytes.len() - 11);
        assert_eq!(decode(&bytes).unwrap(), b);
    }

    #[test]
    fn lfos_round_trip_in_the_block() {
        let mut d = doc_with(&[]);
        d.set_lfo(Lfo { phase: 90.0, trigger: Some((0, 1)), ..Lfo::new(4) }).unwrap();
        let b = Block { lfos: "x".into(), ..Block::default() };
        assert_eq!(decode(&encode(&b)).unwrap(), b);
        assert_eq!(reopen(&d).project().lfos, d.project().lfos);
        assert!(!reopen(&d).is_dirty());
    }

    #[test]
    fn version_6_blocks_still_read() {
        let b = Block { lfos: "lfo".into(), ..Block::default() };
        let full = Block { cc_routes: "cc".into(), ..b.clone() };
        let mut bytes = encode(&full);
        // Version 6: no CC route text (a length and 2 bytes) at the end.
        bytes[..4].copy_from_slice(&6u32.to_le_bytes());
        bytes.truncate(bytes.len() - 6);
        assert_eq!(decode(&bytes).unwrap(), b);
        assert_eq!(decode(&encode(&full)).unwrap(), full);
    }

    #[test]
    fn version_3_blocks_still_read() {
        let b = Block { routes: vec![(0, 1, "x".into())], ..Block::default() };
        let mut bytes = encode(&b);
        // Version 3: no PROB LOCK count or screen at the end.
        bytes[..4].copy_from_slice(&3u32.to_le_bytes());
        bytes.truncate(bytes.len() - 13);
        assert_eq!(decode(&bytes).unwrap(), b);
    }

    #[test]
    fn version_2_blocks_still_read() {
        let b = Block { touch: vec![(0, 1, PadTouch { cc: 74, ..PadTouch::default() })], ..Block::default() };
        let mut bytes = encode(&b);
        // Version 2: no routes or PROB LOCK counts, nor a screen, at the end.
        bytes[..4].copy_from_slice(&2u32.to_le_bytes());
        bytes.truncate(bytes.len() - 17);
        assert_eq!(decode(&bytes).unwrap(), b);
    }

    #[test]
    fn expression_round_trips_through_a_save() {
        let back = reopen(&doc_with(&[full()]));
        assert_eq!(exprs_of(&back), vec![Some(full())]);
        assert!(!back.is_dirty(), "reading the block is no edit");
    }

    #[test]
    fn the_block_is_a_dkrd_root_child() {
        let saved = doc_with(&[full()]).to_bytes().unwrap();
        assert!(root_child(&saved, DOMAIN, ITEM).is_some());
    }

    #[test]
    fn resave_keeps_one_block() {
        let again = reopen(&doc_with(&[full()])).to_bytes().unwrap();
        let root = mxprj::container::Item::parse(&again).unwrap();
        assert_eq!(root.children.iter().filter(|c| c.domain == DOMAIN).count(), 1);
    }

    #[test]
    fn no_expression_no_block() {
        let mut d = reopen(&doc_with(&[full()]));
        let n = d.project().groups[0].patterns[0].notes[0];
        d.edit_notes("Delete", 0, 0, &[n], &[]).unwrap();
        assert_eq!(root_child(&d.to_bytes().unwrap(), DOMAIN, ITEM), None);
    }

    #[test]
    fn undone_take_writes_nothing() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "Pattern 1").unwrap();
        d.begin_take("Record");
        let id = d.add_expression(full());
        d.edit_notes("Record", 0, 0, &[], &[note(0, Some(id))]).unwrap();
        d.end_take();
        assert!(d.undo());
        assert_eq!(root_child(&d.to_bytes().unwrap(), DOMAIN, ITEM), None);
    }

    #[test]
    fn duplicate_keys_use_the_ordinal() {
        let other = Expression { pressure: Some(curve(&[(0, 0.7)])), ..Expression::default() };
        let back = reopen(&doc_with(&[full(), other.clone()]));
        let mut got = exprs_of(&back);
        got.sort_by_key(|e| e.as_ref().map(|e| e.release_velocity));
        assert_eq!(got, vec![Some(other), Some(full())]);
    }

    #[test]
    fn a_bad_block_is_ignored() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "Pattern 1").unwrap();
        d.edit_notes("t", 0, 0, &[], &[note(0, None)]).unwrap();
        let junk = set_root_child(&d.to_bytes().unwrap(), DOMAIN, ITEM, FRAME, Some(b"junk")).unwrap();
        let back = Document::from_bytes(junk, None).unwrap();
        assert_eq!(exprs_of(&back), vec![None]);
    }

    #[test]
    fn orphan_entries_are_dropped() {
        let d = doc_with(&[full()]);
        let mut b = block_of(d.project(), &|id| d.expression(id).cloned());
        let mut ghost = b.patterns[0].2[0].clone();
        ghost.position = 3840;
        b.patterns[0].2.insert(0, ghost);
        let file = set_root_child(&d.to_bytes().unwrap(), DOMAIN, ITEM, FRAME, Some(&encode(&b))).unwrap();
        let back = Document::from_bytes(file, None).unwrap();
        assert_eq!(exprs_of(&back), vec![Some(full())]);
    }

    #[test]
    fn curves_are_cut_at_the_notes_end() {
        let long = Expression { pressure: Some(curve(&[(0, 0.0), (1920, 1.0)])), ..Expression::default() };
        let back = reopen(&doc_with(&[long]));
        let c = exprs_of(&back)[0].clone().unwrap().pressure.unwrap();
        assert_eq!(c.points.last().map(|p| (p.at, p.value)), Some((960, 0.5)));
    }

    fn pressure_ramp() -> Vec<deckard_model::MidiEvent> {
        (0..3)
            .map(|i| deckard_model::MidiEvent { sound: 0, position: i * 240, kind: MidiKind::ChannelPressure(20 + i as u8 * 30) })
            .collect()
    }

    fn midi_of(d: &Document) -> Vec<deckard_model::MidiEvent> {
        d.project().groups[0].patterns[0].midi.clone()
    }

    /// Final review #2: channel pressure Maschine made (in its archive when opened)
    /// stays there, and needs no block.
    #[test]
    fn maschine_made_pressure_stays_native() {
        let mut d = doc_with(&[]);
        d.edit_midi("Pressure", 0, 0, &[], &pressure_ramp()).unwrap();
        // As Maschine saved it: no DECKARD fields.
        let mut root = Value::Obj(d.root().clone());
        crate::expression::strip_deckard_fields(&mut root);
        let Value::Obj(root) = root else { unreachable!() };
        let doc = mxprj::Project::parse(&d.file).unwrap();
        let archived = doc.encode(&d.types, root).unwrap();
        let file = set_root_child(&archived, DOMAIN, ITEM, FRAME, None).unwrap();
        let opened = Document::from_bytes(file.clone(), None).unwrap();
        assert_eq!(midi_of(&opened), pressure_ramp());
        let saved = opened.to_bytes().unwrap();
        assert!(root_child(&saved, DOMAIN, ITEM).is_none(), "no block for Maschine's own events");
        assert_eq!(saved, file, "saved as it was");
    }

    #[test]
    fn channel_pressure_round_trips_in_the_block() {
        let mut d = doc_with(&[]);
        d.edit_midi("Pressure", 0, 0, &[], &pressure_ramp()).unwrap();
        assert_eq!(midi_of(&d), pressure_ramp());
        let saved = d.to_bytes().unwrap();
        // Not in Maschine's archive: in the block.
        let archive = mxprj::Project::parse(&saved).unwrap().decode(&mxprj::types::Types::new()).unwrap();
        let mut kind6 = 0;
        archive.walk_paths(&mut |o, _| {
            kind6 += (o.type_name == "NI::MASCHINE::DATA::MidiEvent" && o.get("i14").and_then(Value::as_int) == Some(6)) as usize
        });
        assert_eq!(kind6, 0);
        let back = Document::from_bytes(saved, None).unwrap();
        assert_eq!(midi_of(&back), pressure_ramp());
        assert!(!back.is_dirty());
        // And saved again, still once.
        assert_eq!(midi_of(&reopen(&back)), pressure_ramp());
    }

    #[test]
    fn native_pressure_stays_in_the_archive() {
        let mut d = doc_with(&[]);
        d.edit_midi("Pressure", 0, 0, &[], &pressure_ramp()).unwrap();
        d.set_native_pressure(true);
        let saved = d.to_bytes().unwrap();
        assert_eq!(root_child(&saved, DOMAIN, ITEM), None);
        assert_eq!(midi_of(&Document::from_bytes(saved, None).unwrap()), pressure_ramp());
    }

    #[test]
    fn a_truncated_block_is_an_error() {
        let bytes = encode(&block_of(doc_with(&[full()]).project(), &|_| Some(full())));
        assert!(decode(&bytes[..bytes.len() - 3]).is_err());
        let mut huge = bytes.clone();
        huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&huge).is_err());
    }
}
