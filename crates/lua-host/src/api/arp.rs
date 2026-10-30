//! NOTE REPEAT: Note Repeat / Arp's settings for NI's RepeatPage scripts. The app hands
//! them over as knobs ([`State::arp`], keys `arp/<field>`, their texts and choice
//! lists); turning one leaves the new value in [`State::arp_edits`]. Whether it plays
//! (`NI.HW.getArpeggiatorActiveParameter`) and LOCK (`SongAccess.isArpRepeatLocked` /
//! `setArpRepeatLockState`) are the app's too.

use super::data::set_fn;
use super::{methods, object, state};
use crate::state::{ObjRef, State};
use mlua::{Lua, MultiValue, Table, Value};

/// The Arpeggiator's parameters, by the scripts' getter and DECKARD's key.
const PARAMS: [(&str, &str); 12] = [
    ("getTypeParameter", "arp/type"),
    ("getSequenceParameter", "arp/sequence"),
    ("getOctavesParameter", "arp/octaves"),
    ("getDynamicParameter", "arp/dynamic"),
    ("getGateParameter", "arp/gate"),
    ("getHoldParameter", "arp/hold"),
    ("getRetriggerParameter", "arp/retrigger"),
    ("getRepeatParameter", "arp/repeat"),
    ("getOffsetParameter", "arp/offset"),
    ("getInversionParameter", "arp/inversion"),
    ("getMinKeyParameter", "arp/min_key"),
    ("getMaxKeyParameter", "arp/max_key"),
];

fn param(lua: &Lua, owner: ObjRef, name: &str) -> mlua::Result<Value> {
    let owner = state(lua).borrow_mut().objects.intern(owner);
    object(lua, ObjRef::Param { owner, name: name.to_string() })
}

/// Item `i` of a setting's choices (`getAsString(i)`).
pub(super) fn list_string(st: &State, owner: &ObjRef, name: &str, i: i64) -> Option<String> {
    if !matches!(owner, ObjRef::Arpeggiator) {
        return None;
    }
    let list = &st.arp.lists.iter().find(|(k, _)| k == name)?.1;
    list.get(usize::try_from(i).ok()?).cloned()
}

/// A choice's largest value, its last item (NI's touched-knob list is `getMax() + 1` long).
pub(super) fn max(st: &State, owner: &ObjRef, name: &str) -> Option<i64> {
    if !matches!(owner, ObjRef::Arpeggiator) {
        return None;
    }
    let list = &st.arp.lists.iter().find(|(k, _)| k == name)?.1;
    Some(list.len().saturating_sub(1) as i64)
}

/// Whether a setting is a choice (its knob lists its values when touched).
pub(super) fn is_enum(st: &State, owner: &ObjRef, name: &str) -> bool {
    matches!(owner, ObjRef::Arpeggiator) && st.arp.lists.iter().any(|(k, _)| k == name)
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let data = methods(lua, impl_t, "NI.DATA")?;
    set_fn(lua, &data, "getArpeggiator", |lua, _: MultiValue| object(lua, ObjRef::Arpeggiator))?;
    for (f, key) in [
        ("getArpeggiatorRateParameter", "arp/rate"),
        ("getArpeggiatorRateUnitParameter", "arp/rate_unit"),
        ("getArpeggiatorPresetParameter", "arp/preset"),
        ("getArpeggiatorPresetParameterJam", "arp/preset"),
    ] {
        set_fn(lua, &data, f, move |lua, _: MultiValue| param(lua, ObjRef::Arpeggiator, key))?;
    }
    let arp = methods(lua, impl_t, "Arpeggiator")?;
    for (f, key) in PARAMS {
        set_fn(lua, &arp, f, move |lua, _: MultiValue| param(lua, ObjRef::Arpeggiator, key))?;
    }
    let hw = methods(lua, impl_t, "NI.HW")?;
    set_fn(lua, &hw, "getArpeggiatorActiveParameter", |lua, _: MultiValue| param(lua, ObjRef::Workspace, "ArpActive"))?;
    let song = methods(lua, impl_t, "NI.DATA.SongAccess")?;
    set_fn(lua, &song, "isArpRepeatLocked", |lua, _: MultiValue| Ok(state(lua).borrow().arp.locked))?;
    set_fn(lua, &song, "setArpRepeatLockState", |lua, (_, on): (Value, bool)| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        st.arp.locked = on;
        st.arp_edits.push(("arp/locked".into(), f64::from(u8::from(on))));
        Ok(())
    })?;
    Ok(())
}
