//! The lanes listed under a sound's notes: the sound's `midi_automators`, an
//! `ObjectVector` of `MidiAutomator`s (`mode` a MIDI event type: 2 velocity, 8
//! probability, 4 a control change with `controller` its number, 3 pitch bend), whose
//! `current` is the lane shown.

use crate::{Document, Op};
use deckard_model::{LaneRef, MidiEvent};
use mxprj::decode::{Obj, Path, Value};

const AUTOMATOR: &str = "NI::MASCHINE::DATA::MidiAutomator";

/// The automator a list item (`shared_ptr{px}`) holds.
fn automator(item: &Value) -> Option<&Obj> {
    match item {
        Value::Obj(o) => match o.get("px") {
            Some(Value::Obj(a)) if a.type_name == AUTOMATOR => Some(a),
            _ => None,
        },
        _ => None,
    }
}

fn param(a: &Obj, field: &str) -> Option<i64> {
    match a.get(field)? {
        Value::Obj(p) => p.find("value")?.as_int(),
        _ => None,
    }
}

fn lane_of(item: &Value) -> Option<LaneRef> {
    let a = automator(item)?;
    Some(LaneRef { kind: param(a, "mode")?, number: param(a, "controller").unwrap_or(0) })
}

/// Pitch bend lanes are the same lane whatever their number (Maschine saved −1 or 0).
fn same(a: LaneRef, b: LaneRef) -> bool {
    a.kind == b.kind && (a.kind == 3 || a.number == b.number)
}

/// Set a parameter's value (and `t0`, which mirrors it).
fn set_param(a: &mut Obj, field: &str, v: i64) {
    if let Some(Value::Obj(p)) = a.get_mut(field) {
        for k in ["value", "t0"] {
            if let Some(slot) = p.find_mut(k) {
                *slot = Value::Int(v);
            }
        }
    }
}

impl Document {
    /// The sound's `midi_automators` vector: (its items list, its `current`).
    fn lane_paths(&self, g: usize, s: usize) -> Result<(Path, Path), String> {
        let sound = self.addresses.groups.get(g).and_then(|ga| ga.sounds.get(s)).ok_or("no such sound")?.clone();
        let v = self.field_path(&sound, &["midi_automators"])?;
        Ok((self.field_path(&v, &["objects", "items"])?, self.field_path(&v, &["current"])?))
    }

    /// Add `lane` to sound `s`'s lane list and show it, as one undo step. Nothing
    /// happens if the sound has it already.
    pub fn add_lane(&mut self, g: usize, s: usize, lane: LaneRef) -> Result<(), String> {
        let (items_path, current_path) = self.lane_paths(g, s)?;
        let mut items = self.items(&items_path)?;
        if items.iter().filter_map(lane_of).any(|l| same(l, lane)) {
            return Ok(());
        }
        let mut item = match items.iter().find(|i| automator(i).is_some()) {
            Some(t) => {
                let t = t.clone();
                self.fresh_copy(&t)
            }
            None => self.build(|b| b.item(AUTOMATOR))?,
        };
        let Value::Obj(wrapper) = &mut item else { unreachable!() };
        let Some(Value::Obj(a)) = wrapper.get_mut("px") else { unreachable!() };
        set_param(a, "mode", lane.kind);
        set_param(a, "controller", lane.number);
        let oid = a.oid.ok_or("lane without id")?;
        items.push(item);
        let ops = vec![Op::Set { path: items_path, value: Value::List(items) }, Op::Set { path: current_path, value: Value::Ref(oid) }];
        self.apply("Add MIDI Lane", ops)
    }

    /// Take `lane` off sound `s`'s list and delete `events` (its events in pattern
    /// `p`), as one undo step.
    pub fn remove_lane(&mut self, g: usize, p: usize, s: usize, lane: LaneRef, events: &[MidiEvent]) -> Result<(), String> {
        let mut ops = if events.is_empty() { Vec::new() } else { self.event_ops(g, p, &[], &[], events, &[])? };
        let (items_path, current_path) = self.lane_paths(g, s)?;
        let mut items = self.items(&items_path)?;
        let before = items.len();
        let removed: Vec<u64> =
            items.iter().filter(|i| lane_of(i).is_some_and(|l| same(l, lane))).filter_map(|i| automator(i)?.oid).collect();
        items.retain(|i| !lane_of(i).is_some_and(|l| same(l, lane)));
        if items.len() != before {
            if let Some(Value::Ref(cur)) = self.tree.at(&current_path)
                && removed.contains(cur)
            {
                let first = items.iter().find_map(|i| automator(i)?.oid);
                ops.push(Op::Set { path: current_path, value: first.map_or(Value::Null, Value::Ref) });
            }
            ops.push(Op::Set { path: items_path, value: Value::List(items) });
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.apply("Remove MIDI Lane", ops)
    }
}
