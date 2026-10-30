//! DECKARD's Pad Touch settings on a sound (docs/superpowers/specs/2026-10-24-pad-touch-
//! design.md), for the Keyboard page's built-in mod: `PadTouchMode` (a list),
//! `PadTouchCC`, and `PadTouchCurve`, `PadTouchMin`, `PadTouchMax` in percent, kept in
//! the project (`Document::set_pad_touch`).

use crate::state::{ObjRef, Prop, State};
use deckard_model::pad_touch::{PadTouch, TouchMode, cc_label};

fn is_ours(owner: &ObjRef, name: &str) -> bool {
    matches!(owner, ObjRef::Sound(..)) && matches!(name, "PadTouchMode" | "PadTouchCC" | "PadTouchCurve" | "PadTouchMin" | "PadTouchMax")
}

pub(super) fn is_enum(owner: &ObjRef, name: &str) -> bool {
    is_ours(owner, name) && name == "PadTouchMode"
}

pub(super) fn max(owner: &ObjRef, name: &str) -> Option<i64> {
    is_ours(owner, name).then_some(match name {
        "PadTouchMode" => 3,
        "PadTouchCC" => 127,
        _ => 100,
    })
}

/// Item `i` of the Mode list.
pub(super) fn list_string(owner: &ObjRef, name: &str, i: i64) -> Option<String> {
    is_enum(owner, name)
        .then(|| deckard_model::pad_touch::TouchMode::ALL.get(usize::try_from(i).ok()?).map(|m| m.name().to_string()))
        .flatten()
}

/// How a value shows under its knob.
pub(super) fn value_string(owner: &ObjRef, name: &str, v: &Prop) -> Option<String> {
    if !is_ours(owner, name) {
        return None;
    }
    let i = match v {
        Prop::Int(i) => *i,
        Prop::Num(n) => n.round() as i64,
        _ => return None,
    };
    Some(match name {
        "PadTouchMode" => list_string(owner, name, i)?,
        "PadTouchCC" => cc_label(i.clamp(0, 127) as u8),
        _ => format!("{i} %"),
    })
}

fn touch(st: &State, g: usize, s: usize) -> Option<PadTouch> {
    Some(st.project()?.groups.get(g)?.sounds.get(s)?.pad_touch)
}

/// The sound's setting, as the project has it.
pub(super) fn live(st: &State, owner: &ObjRef, name: &str) -> Option<Prop> {
    let ObjRef::Sound(g, s) = owner else { return None };
    if !is_ours(owner, name) {
        return None;
    }
    let t = touch(st, *g, *s)?;
    Some(Prop::Int(match name {
        "PadTouchMode" => t.mode.index(),
        "PadTouchCC" => t.cc as i64,
        "PadTouchCurve" => t.curve as i64,
        "PadTouchMin" => t.min as i64,
        _ => t.max as i64,
    }))
}

/// Set the sound's setting (one undo step), within its range; Min above Max takes Max
/// with it, and Max below Min takes Min.
pub(super) fn write(st: &mut State, owner: &ObjRef, name: &str, v: &Prop) -> bool {
    let (ObjRef::Sound(g, s), Some(max)) = (owner, max(owner, name)) else { return false };
    let n = match v {
        Prop::Int(i) => *i,
        Prop::Num(n) => n.round() as i64,
        Prop::Bool(b) => i64::from(*b),
        _ => return false,
    }
    .clamp(0, max);
    let Some(mut t) = touch(st, *g, *s) else { return true };
    let byte = n as u8;
    match name {
        "PadTouchMode" => t.mode = TouchMode::from_index(n),
        "PadTouchCC" => t.cc = byte,
        "PadTouchCurve" => t.curve = byte,
        "PadTouchMin" => {
            t.min = byte;
            t.max = t.max.max(byte);
        }
        _ => {
            t.max = byte;
            t.min = t.min.min(byte);
        }
    }
    let Some(doc) = st.doc.clone() else { return true };
    let done = doc.borrow_mut().set_pad_touch(*g, *s, t);
    if let Err(e) = done {
        st.log.push(format!("edit {owner:?}.{name}: {e}"));
    }
    true
}
