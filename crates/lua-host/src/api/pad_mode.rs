//! PAD MODE's sound settings (`PadModePageMK3`): a sound's Choke and Link groups (None,
//! 1–8) and modes (Send, Receive), kept in the project, and the pads' Focus mode (On
//! Trigger, Pinned), the workspace's.

use crate::state::{ObjRef, Prop, State};

/// A sound's choke and link settings, by the scripts' name: the project's field and the
/// undo label.
fn field(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "ChokeGroup" => ("choke_group", "Choke Group"),
        "ChokeMode" => ("choke_mode", "Choke Mode"),
        "LinkGroup" => ("link_group", "Link Group"),
        "LinkMode" => ("link_mode", "Link Mode"),
        _ => return None,
    })
}

const GROUPS: [&str; 9] = ["None", "1", "2", "3", "4", "5", "6", "7", "8"];
const MODES: [&str; 2] = ["Send", "Receive"];
/// `NI.DATA.Workspace.FOCUS_SOUND_ON_PAD_EVENT_ON_TRIGGER` and `_PINNED`: DECKARD's
/// values (they aren't among the recovered ones).
pub const FOCUS: [(&str, i64); 2] = [("FOCUS_SOUND_ON_PAD_EVENT_ON_TRIGGER", 0), ("FOCUS_SOUND_ON_PAD_EVENT_PINNED", 1)];
const FOCUS_NAMES: [&str; 2] = ["On Trigger", "Pinned"];

fn list(owner: &ObjRef, name: &str) -> Option<&'static [&'static str]> {
    match (owner, name) {
        (ObjRef::Sound(..), "ChokeGroup" | "LinkGroup") => Some(&GROUPS),
        (ObjRef::Sound(..), "ChokeMode" | "LinkMode") => Some(&MODES),
        (ObjRef::Workspace, "FocusSoundOnPadEvent") => Some(&FOCUS_NAMES),
        _ => None,
    }
}

pub(super) fn is_enum(owner: &ObjRef, name: &str) -> bool {
    list(owner, name).is_some()
}

pub(super) fn max(owner: &ObjRef, name: &str) -> Option<i64> {
    list(owner, name).map(|l| l.len() as i64 - 1)
}

/// Item `i` of a setting's list.
pub(super) fn list_string(owner: &ObjRef, name: &str, i: i64) -> Option<String> {
    list(owner, name)?.get(usize::try_from(i).ok()?).map(|s| s.to_string())
}

/// How a value shows under its knob.
pub(super) fn value_string(owner: &ObjRef, name: &str, v: &Prop) -> Option<String> {
    let i = match v {
        Prop::Int(i) => *i,
        Prop::Num(n) => n.round() as i64,
        _ => return None,
    };
    list_string(owner, name, i)
}

/// A sound's setting, as the project has it.
pub(super) fn live(st: &State, owner: &ObjRef, name: &str) -> Option<Prop> {
    let ObjRef::Sound(g, s) = owner else { return None };
    field(name)?;
    let p = st.project()?;
    let c = &p.groups.get(*g)?.sounds.get(*s)?.channel;
    Some(Prop::Int(match name {
        "ChokeGroup" => c.choke_group,
        "ChokeMode" => c.choke_mode,
        "LinkGroup" => c.link_group,
        _ => c.link_mode,
    }))
}

/// Set a sound's setting in the project (one undo step), kept within its list; the
/// Focus mode within its two.
pub(super) fn write(st: &mut State, owner: &ObjRef, name: &str, v: &Prop) -> bool {
    let Some(max) = max(owner, name) else { return false };
    let n = match v {
        Prop::Int(i) => *i,
        Prop::Num(n) => n.round() as i64,
        Prop::Bool(b) => i64::from(*b),
        _ => return false,
    }
    .clamp(0, max);
    let ObjRef::Sound(g, s) = owner else {
        // The Focus mode: the workspace's, kept as a parameter (not handled here).
        return false;
    };
    let Some((f, label)) = field(name) else { return false };
    let Some(doc) = st.doc.clone() else { return false };
    let done = doc.borrow_mut().set(label, deckard_doc::Target::Sound(*g, *s), f, mxprj::decode::Value::Int(n));
    if let Err(e) = done {
        st.log.push(format!("edit {owner:?}.{name}: {e}"));
    }
    true
}
