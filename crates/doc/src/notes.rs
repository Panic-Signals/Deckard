//! Note editing. A pattern maps each sound to a `SequenceReference`, whose
//! `sequence` (null if the sound has no notes there) is defined in the sound's own
//! `sequences` list. A sequence's events are a boost `multi_index_container`:
//! `count`, then per event a `TimedEvent` (position and a pointer to the `NoteEvent`:
//! length, note, velocity, and from Maschine 3 a probability) followed by a node
//! marker, then a header marker and a reorder list that is always just null, since
//! events are saved in index (position) order. Node markers are empty; Maschine's
//! files often refer them back to unrelated earlier objects (reused addresses), and
//! new ones are written as fresh objects.

use super::arrange::px_mut;
use super::{Document, Op};
use deckard_model::{MidiEvent, MidiKind, Note};
use mxprj::decode::{Obj, Path, Value};
use std::collections::HashMap;

const SEQUENCE: &str = "NI::MASCOMMON::DATA::Sequence";
const TIMED_EVENT: &str = "NI::MASCOMMON::DATA::TimedEvent";
const NOTE_EVENT: &str = "NI::MASCOMMON::DATA::NoteEvent";
const MIDI_EVENT: &str = "NI::MASCHINE::DATA::MidiEvent";
const AUDIO_EVENT: &str = "NI::MASCHINE::DATA::AudioEvent";
const LENGTH_BASE: &str = "<NI::MASCOMMON::DATA::LengthEvent>";
const POLICY: &str = "NI::MASCOMMON::DATA::ProbabilityEventPolicy<NI::MASCOMMON::DATA::NoteEvent>";

/// Bring note events to the format that has a probability (NoteEvent v1 with a
/// ProbabilityEventPolicy v1), as Maschine 3 does when it re-saves an older project.
/// Returns how many objects changed.
pub(crate) fn upgrade_notes(v: &mut Value) -> usize {
    let mut n = 0;
    v.walk_mut(&mut |o| {
        if o.type_name == NOTE_EVENT && o.version == 0 {
            let at = o.fields.iter().position(|(f, _)| f == LENGTH_BASE).map_or(0, |i| i + 1);
            let policy =
                Obj { type_name: POLICY.into(), version: 1, fields: vec![("probability".into(), Value::Int(100))], ..Obj::default() };
            o.fields.insert(at, (format!("<{POLICY}>"), Value::Obj(policy)));
            o.version = 1;
            n += 1;
        } else if o.type_name == POLICY && o.version == 0 {
            let muted = o.get("muted").and_then(Value::as_int).unwrap_or(0) != 0;
            o.fields = vec![("probability".into(), Value::Int(if muted { 0 } else { 100 }))];
            o.version = 1;
            n += 1;
        }
    });
    n
}

/// The note fields of a `NoteEvent` (length sits on its `LengthEvent` base).
fn note_fields(ev: &Obj) -> Option<(i64, i64, i64)> {
    Some((ev.find("length")?.as_int()?, ev.find("note")?.as_int()?, ev.find("velocity")?.as_int()?))
}

fn set_int(o: &mut Obj, field: &str, v: i64) {
    if let Some(slot) = o.find_mut(field) {
        *slot = Value::Int(v);
    }
}

/// What a `MidiEvent` holds: `i14` the kind (4 = control change, 3 = pitch bend),
/// `i18` the controller number (−1 for pitch bend), `i20` the value.
fn midi_kind(ev: &Obj) -> Option<MidiKind> {
    let int = |k: &str| ev.get(k).and_then(Value::as_int);
    match int("i14")? {
        4 => Some(MidiKind::Controller { number: int("i18")?.clamp(0, 127) as u8, value: int("i20")?.clamp(0, 127) as u8 }),
        3 => Some(MidiKind::PitchBend(int("i20")?.clamp(0, 16383) as u16)),
        6 => Some(MidiKind::ChannelPressure(int("i20")?.clamp(0, 127) as u8)),
        _ => None,
    }
}

fn midi_fields(kind: MidiKind) -> (i64, i64, i64) {
    match kind {
        MidiKind::Controller { number, value } => (4, number as i64, value as i64),
        MidiKind::PitchBend(v) => (3, -1, v as i64),
        MidiKind::ChannelPressure(v) => (6, -1, v as i64),
    }
}

/// The notes and MIDI events to take out of a pattern and put in, per sound.
#[derive(Default)]
pub(crate) struct Edits<'a> {
    remove: Vec<&'a Note>,
    add: Vec<&'a Note>,
    remove_midi: Vec<&'a MidiEvent>,
    add_midi: Vec<&'a MidiEvent>,
}

/// One event of a sequence: the `TimedEvent` and its node marker.
#[derive(Clone)]
pub(crate) struct Item {
    pub(crate) event: Value,
    pub(crate) node: Value,
}

impl Item {
    pub(crate) fn position(&self) -> i64 {
        match &self.event {
            Value::Obj(o) => o.get("position").and_then(Value::as_int).unwrap_or(0),
            _ => 0,
        }
    }
}

impl Document {
    /// Every object by id (first occurrence) with its path.
    pub(crate) fn object_paths(&self) -> HashMap<u64, Path> {
        let mut out = HashMap::new();
        self.root().walk_paths(&mut |o, path| {
            if let Some(id) = o.oid {
                out.entry(id).or_insert_with(|| path.to_vec());
            }
        });
        out
    }

    pub(crate) fn obj_by_path(&self, path: &Path) -> Option<&Obj> {
        match self.tree.at(path)? {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }

    /// The event a `TimedEvent` points to, following a back-reference.
    pub(crate) fn event_obj<'a>(&'a self, te: &'a Value, paths: &HashMap<u64, Path>) -> Option<&'a Obj> {
        let Value::Obj(te) = te else { return None };
        match te.get("event")? {
            Value::Obj(e) => Some(e),
            Value::Ref(id) => self.obj_by_path(paths.get(id)?),
            _ => None,
        }
    }

    /// A first object of `type_name` (to copy as a template).
    pub(crate) fn find_type(&self, pred: impl Fn(&Obj) -> bool) -> Option<Obj> {
        let mut found = None;
        self.root().walk_paths(&mut |o, _| {
            if found.is_none() && pred(o) {
                found = Some(o.clone());
            }
        });
        found
    }

    /// A new note event (`TimedEvent` holding its `NoteEvent`), copied from
    /// `template` (or any note in the project) with fresh ids.
    fn new_event(&mut self, n: &Note, template: Option<&Value>, paths: &HashMap<u64, Path>) -> Result<Value, String> {
        let te = match template {
            Some(t) => t.clone(),
            None => self.build(|b| b.note_event())?,
        };
        // The event itself (not a reference to someone else's).
        let ev = self.event_obj(&te, paths).ok_or("note event not found")?.clone();
        let Value::Obj(mut te) = te else { unreachable!() };
        *te.get_mut("event").ok_or("TimedEvent without event")? = Value::Obj(ev);
        let mut te = self.fresh_copy(&Value::Obj(te));
        let Value::Obj(t) = &mut te else { unreachable!() };
        *t.get_mut("position").ok_or("TimedEvent without position")? = Value::Int(n.position);
        super::expression::set_expr_field(t, n.expr);
        let Some(Value::Obj(ev)) = t.get_mut("event") else { unreachable!() };
        set_int(ev, "length", n.length.max(1));
        set_int(ev, "note", n.pitch.clamp(0, 127));
        set_int(ev, "velocity", n.velocity.clamp(1, 127));
        set_int(ev, "probability", n.probability.clamp(0, 100));
        Ok(te)
    }

    /// A new MIDI event (`TimedEvent` holding a `MidiEvent`), copied from one in the
    /// project if there is one, else built from the classes every note uses.
    fn new_midi_event(&mut self, m: &MidiEvent) -> Result<Value, String> {
        let template =
            self.find_type(|o| o.type_name == TIMED_EVENT && matches!(o.get("event"), Some(Value::Obj(e)) if e.type_name == MIDI_EVENT));
        let mut te = match template {
            Some(t) => self.fresh_copy(&Value::Obj(t)),
            None => self.build(|b| b.midi_event())?,
        };
        let Value::Obj(t) = &mut te else { unreachable!() };
        *t.get_mut("position").ok_or("TimedEvent without position")? = Value::Int(m.position);
        crate::block::mark_made(t, matches!(m.kind, MidiKind::ChannelPressure(_)));
        let Some(Value::Obj(ev)) = t.get_mut("event") else { return Err("TimedEvent without event".into()) };
        let (kind, number, value) = midi_fields(m.kind);
        set_int(ev, "i14", kind);
        set_int(ev, "i18", number);
        set_int(ev, "i20", value);
        Ok(te)
    }

    pub(crate) fn new_node(&mut self) -> Result<Value, String> {
        self.build(|b| b.node())
    }

    /// The pattern's entry for `sound` (index in the entries list) and where its
    /// sequence is defined (`None` if it has no notes there).
    pub(crate) fn sequence_of(
        &self,
        g: usize,
        p: usize,
        sound: usize,
        paths: &HashMap<u64, Path>,
    ) -> Result<(Path, usize, Option<Path>), String> {
        let ga = self.addresses.groups.get(g).ok_or("no such group")?;
        let pattern = if p >= crate::CLIP { ga.clip_patterns.get(p - crate::CLIP) } else { ga.patterns.get(p) }.ok_or("no such pattern")?;
        let sound_oid = self.oid_at(ga.sounds.get(sound).ok_or("no such sound")?)?;
        let entries_path = self.field_path(pattern, &["sound_sequences", "objects", "items"])?;
        let entries = self.items(&entries_path)?;
        let is_sound = |e: &Value| match e {
            Value::Obj(o) => match o.get("sound") {
                Some(Value::Ref(r)) => *r == sound_oid,
                Some(Value::Obj(s)) => s.oid == Some(sound_oid),
                _ => false,
            },
            _ => false,
        };
        let i = entries.iter().position(is_sound).ok_or("the pattern has no slot for this sound")?;
        // `reference` is a shared_ptr{px: SequenceReference}.
        let seq_ref = match &entries[i] {
            Value::Obj(o) => match o.get("reference") {
                Some(Value::Obj(r)) => match r.get("px") {
                    Some(Value::Obj(sr)) => sr.get("sequence").cloned(),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };
        let seq_path = match seq_ref {
            Some(Value::Ref(id)) => Some(paths.get(&id).cloned().ok_or("sequence not found")?),
            Some(Value::Obj(_)) => {
                let entry = [entries_path.as_slice(), &[i as u32]].concat();
                Some(self.field_path(&entry, &["reference", "px", "sequence"])?)
            }
            _ => None,
        };
        Ok((entries_path, i, seq_path))
    }

    /// Remove `remove` and add `add` in pattern `p` of group `g`, as one undo step.
    /// Notes are matched by sound, position and pitch.
    pub fn edit_notes(&mut self, label: &str, g: usize, p: usize, remove: &[Note], add: &[Note]) -> Result<(), String> {
        self.edit_events(label, g, p, remove, add, &[], &[])
    }

    /// Remove and add MIDI controller and pitch-bend events (matched exactly), as one
    /// undo step.
    pub fn edit_midi(&mut self, label: &str, g: usize, p: usize, remove: &[MidiEvent], add: &[MidiEvent]) -> Result<(), String> {
        self.edit_events(label, g, p, &[], &[], remove, add)
    }

    #[allow(clippy::too_many_arguments)]
    fn edit_events(
        &mut self,
        label: &str,
        g: usize,
        p: usize,
        remove: &[Note],
        add: &[Note],
        remove_midi: &[MidiEvent],
        add_midi: &[MidiEvent],
    ) -> Result<(), String> {
        let ops = self.event_ops(g, p, remove, add, remove_midi, add_midi)?;
        if ops.is_empty() {
            return Ok(());
        }
        self.apply(label, ops)
    }

    /// The ops of an [`edit_events`](Self::edit_events), to apply with others.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn event_ops(
        &mut self,
        g: usize,
        p: usize,
        remove: &[Note],
        add: &[Note],
        remove_midi: &[MidiEvent],
        add_midi: &[MidiEvent],
    ) -> Result<Vec<Op>, String> {
        if add.iter().any(|n| n.probability != 100) {
            self.upgrade_note_format();
        }
        let paths = self.object_paths();
        let mut sounds: Vec<usize> =
            remove.iter().chain(add).map(|n| n.sound).chain(remove_midi.iter().chain(add_midi).map(|m| m.sound)).collect();
        sounds.sort_unstable();
        sounds.dedup();
        let mut ops = Vec::new();
        for sound in sounds {
            let (entries_path, entry, seq) = self.sequence_of(g, p, sound, &paths)?;
            let e = Edits {
                remove: remove.iter().filter(|n| n.sound == sound).collect(),
                add: add.iter().filter(|n| n.sound == sound).collect(),
                remove_midi: remove_midi.iter().filter(|m| m.sound == sound).collect(),
                add_midi: add_midi.iter().filter(|m| m.sound == sound).collect(),
            };
            match seq {
                Some(seq_path) => ops.extend(self.rewrite_sequence(&seq_path, &e, &paths)?),
                None if !e.remove.is_empty() || !e.remove_midi.is_empty() => return Err("no such event".into()),
                None => ops.extend(self.create_sequence(g, sound, &entries_path, entry, &e, &paths)?),
            }
        }
        Ok(ops)
    }

    /// Ops that rewrite the sequence at `seq_path` without `removes` and with `adds`.
    fn rewrite_sequence(&mut self, seq_path: &Path, e: &Edits, paths: &HashMap<u64, Path>) -> Result<Vec<Op>, String> {
        let (removes, adds) = (&e.remove, &e.add);
        let seq = self.obj_by_path(seq_path).ok_or("sequence")?.clone();
        let events_path = self.field_path(seq_path, &["events"])?;
        let Some(Value::Obj(events)) = self.tree.at(&events_path).cloned() else { return Err("sequence without events".into()) };
        let mut items = items_of(&events)?;
        // Remove: first event with the same position and pitch (closest in length and
        // velocity among ties).
        let mut removed = Vec::new();
        for n in removes {
            let pos = items
                .iter()
                .enumerate()
                .filter(|(_, it)| it.position() == n.position)
                .filter_map(|(i, it)| {
                    let (len, pitch, vel) = note_fields(self.event_obj(&it.event, paths)?)?;
                    (pitch == n.pitch).then_some((i, (len - n.length).abs() + (vel - n.velocity).abs()))
                })
                .min_by_key(|(_, d)| *d)
                .map(|(i, _)| i)
                .ok_or_else(|| format!("no note at {} with pitch {}", n.position, n.pitch))?;
            removed.push(items.remove(pos));
        }
        for m in &e.remove_midi {
            let pos = items
                .iter()
                .position(|it| {
                    it.position() == m.position
                        && self.event_obj(&it.event, paths).is_some_and(|ev| ev.type_name == MIDI_EVENT && midi_kind(ev) == Some(m.kind))
                })
                .ok_or_else(|| format!("no MIDI event at {}", m.position))?;
            removed.push(items.remove(pos));
        }
        // Add, copying the format of a removed note of this sound if there is one. Only
        // notes are templates: sequences also hold other events (MIDI controllers such
        // as a sustain pedal, recorded with the notes), which stay when every note goes.
        let is_note = |it: &Item| self.event_obj(&it.event, paths).is_some_and(|e| e.type_name == NOTE_EVENT);
        let removed_notes: Vec<Item> = removed.iter().filter(|it| is_note(it)).cloned().collect();
        let kept_note = items.iter().find(|it| is_note(it)).map(|i| i.event.clone());
        for (k, n) in adds.iter().enumerate() {
            let template = removed_notes.get(k).or(removed_notes.first()).map(|i| i.event.clone()).or_else(|| kept_note.clone());
            let event = self.new_event(n, template.as_ref(), paths)?;
            let node = self.new_node()?;
            // After existing events at the same position, as an insert would go.
            let at = items.iter().position(|i| i.position() > n.position).unwrap_or(items.len());
            items.insert(at, Item { event, node });
        }
        for m in &e.add_midi {
            let event = self.new_midi_event(m)?;
            let node = self.new_node()?;
            let at = items.iter().position(|i| i.position() > m.position).unwrap_or(items.len());
            items.insert(at, Item { event, node });
        }
        let mut ops = vec![Op::Set { path: events_path, value: Value::Obj(events_with(&events, &items)) }];
        // The sequence's `current` event, if it was removed.
        let removed_ids: Vec<u64> = removed.iter().filter_map(|i| self.event_obj(&i.event, paths).and_then(|e| e.oid)).collect();
        if let Some(Value::Ref(cur)) = seq.get("current")
            && removed_ids.contains(cur)
        {
            ops.push(Op::Set { path: self.field_path(seq_path, &["current"])?, value: Value::Null });
        }
        Ok(ops)
    }

    /// The Audio plug-in of sound `sound` plays take object `take` in pattern `p` (its
    /// `AudioEvent`, at the start, for ever), or doesn't (none): ops for that.
    pub(crate) fn audio_event_ops(&mut self, g: usize, p: usize, sound: usize, take: Option<u64>) -> Result<Vec<Op>, String> {
        let paths = self.object_paths();
        let (entries_path, entry, seq) = self.sequence_of(g, p, sound, &paths)?;
        let item = match take {
            Some(t) => Some(Item { event: self.build(|b| b.audio_event(t))?, node: self.new_node()? }),
            None => None,
        };
        let Some(seq_path) = seq else {
            return match item {
                Some(it) => self.create_sequence_with(g, sound, &entries_path, entry, &Edits::default(), vec![it], &paths),
                None => Ok(Vec::new()),
            };
        };
        let events_path = self.field_path(&seq_path, &["events"])?;
        let Some(Value::Obj(events)) = self.tree.at(&events_path).cloned() else { return Err("sequence without events".into()) };
        let mut items = items_of(&events)?;
        let before = items.len();
        items.retain(|it| !self.event_obj(&it.event, &paths).is_some_and(|e| e.type_name == AUDIO_EVENT));
        if before == items.len() && item.is_none() {
            return Ok(Vec::new());
        }
        if let Some(it) = item {
            items.insert(0, it);
        }
        let mut ops = vec![Op::Set { path: events_path, value: Value::Obj(events_with(&events, &items)) }];
        // The sequence's `current` event, if it was an audio event now gone.
        ops.push(Op::Set { path: self.field_path(&seq_path, &["current"])?, value: Value::Null });
        Ok(ops)
    }

    /// Ops that give `sound` a new sequence (in its `sequences` list) holding `adds`,
    /// and point the pattern's entry at it.
    fn create_sequence(
        &mut self,
        g: usize,
        sound: usize,
        entries_path: &Path,
        entry: usize,
        e: &Edits,
        paths: &HashMap<u64, Path>,
    ) -> Result<Vec<Op>, String> {
        self.create_sequence_with(g, sound, entries_path, entry, e, Vec::new(), paths)
    }

    /// As [`create_sequence`](Self::create_sequence), with `first` (made already)
    /// before the notes.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create_sequence_with(
        &mut self,
        g: usize,
        sound: usize,
        entries_path: &Path,
        entry: usize,
        e: &Edits,
        first: Vec<Item>,
        paths: &HashMap<u64, Path>,
    ) -> Result<Vec<Op>, String> {
        let sound_path = self.addresses.groups[g].sounds[sound].clone();
        let list_path = self.field_path(&sound_path, &["sequences", "objects", "items"])?;
        let mut list = self.items(&list_path)?;
        // A sequence item (shared_ptr{px: Sequence}) to copy the format from.
        let mut item = match list.first() {
            Some(t) => {
                let t = t.clone();
                self.fresh_copy(&t)
            }
            None => self.build(|b| b.item(SEQUENCE))?,
        };
        let header = self.new_node()?;
        let mut new_items = first;
        let mut sorted: Vec<(i64, Option<&Note>, Option<&MidiEvent>)> = e.add.iter().map(|n| (n.position, Some(*n), None)).collect();
        sorted.extend(e.add_midi.iter().map(|m| (m.position, None, Some(*m))));
        // Controllers first at the same position, as recorded.
        sorted.sort_by_key(|(at, n, _)| (*at, n.is_some()));
        for (_, n, m) in sorted {
            let event = match (n, m) {
                (Some(n), _) => self.new_event(n, None, paths)?,
                (_, Some(m)) => self.new_midi_event(m)?,
                _ => unreachable!(),
            };
            new_items.push(Item { event, node: self.new_node()? });
        }
        let seq = px_mut(&mut item).ok_or("sequence item")?;
        let seq_oid = seq.oid.ok_or("sequence id")?;
        let Some(Value::Obj(events)) = seq.get("events").cloned() else { return Err("sequence without events".into()) };
        let mut events = events_with(&events, &new_items);
        if let Some(h) = events.get_mut("header") {
            *h = header;
        }
        if let Some(r) = events.get_mut("reorder") {
            *r = Value::Null;
        }
        *seq.get_mut("events").unwrap() = Value::Obj(events);
        if let Some(c) = seq.get_mut("current") {
            *c = Value::Null;
        }
        list.push(item);
        let reference = self.field_path(&[entries_path.as_slice(), &[entry as u32]].concat(), &["reference", "px", "sequence"])?;
        Ok(vec![Op::Set { path: list_path, value: Value::List(list) }, Op::Set { path: reference, value: Value::Ref(seq_oid) }])
    }

    /// Whether notes can store a probability (Maschine 3's format).
    pub fn notes_have_probability(&self) -> bool {
        let mut old = false;
        self.root().walk_paths(&mut |o, _| old |= (o.type_name == NOTE_EVENT || o.type_name == POLICY) && o.version == 0);
        !old
    }

    /// Upgrade every note to the format with a probability (see [`upgrade_notes`]).
    /// A class has one version per archive, so this covers the whole project and the
    /// undo history. It is not an undo step of its own; the project counts as changed.
    pub fn upgrade_note_format(&mut self) -> bool {
        if upgrade_notes(&mut self.tree) == 0 {
            return false;
        }
        for step in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            for op in step.undo.iter_mut().chain(step.redo.iter_mut()) {
                if let Op::Set { value, .. } = op {
                    upgrade_notes(value);
                }
            }
        }
        self.refresh();
        true
    }

    pub fn add_note(&mut self, g: usize, p: usize, note: Note) -> Result<(), String> {
        self.edit_notes("Add Note", g, p, &[], &[note])
    }

    pub fn delete_notes(&mut self, g: usize, p: usize, notes: &[Note]) -> Result<(), String> {
        self.edit_notes(if notes.len() == 1 { "Delete Note" } else { "Delete Notes" }, g, p, notes, &[])
    }

    /// Replace `old` with `new` (moved, resized, other pitch, velocity or sound).
    pub fn change_note(&mut self, g: usize, p: usize, old: Note, new: Note) -> Result<(), String> {
        if old == new {
            return Ok(());
        }
        self.edit_notes("Edit Note", g, p, &[old], &[new])
    }
}

/// The events of an `Events` object, in order.
pub(crate) fn items_of(events: &Obj) -> Result<Vec<Item>, String> {
    let Some(items) = events.get("items") else { return Err("events without items".into()) };
    let fields: Vec<(String, Value)> = match items {
        Value::Obj(o) if o.type_name.is_empty() => o.fields.clone(),
        Value::Null | Value::Obj(_) => Vec::new(),
        _ => return Err("unexpected events layout".into()),
    };
    if !fields.len().is_multiple_of(2) {
        return Err("events not in event/node pairs".into());
    }
    Ok(fields.chunks(2).map(|c| Item { event: c[0].1.clone(), node: c[1].1.clone() }).collect())
}

/// `events` with its items (and count) replaced.
pub(crate) fn events_with(events: &Obj, items: &[Item]) -> Obj {
    let mut out = events.clone();
    let mut fields = Vec::with_capacity(items.len() * 2);
    for (i, it) in items.iter().enumerate() {
        fields.push((format!("e{i}"), it.event.clone()));
        fields.push((format!("n{i}"), it.node.clone()));
    }
    if let Some(c) = out.get_mut("count") {
        *c = Value::Int(items.len() as i64);
    }
    if let Some(it) = out.get_mut("items") {
        *it = Value::Obj(Obj { fields, ..Obj::default() });
    }
    out
}
