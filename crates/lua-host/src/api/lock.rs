//! Lock on the controller (docs/superpowers/plans/2026-10-22-phase6-lock.md): the
//! scripts' `ParameterSnapshotsAccess` calls, answered from the project's Lock and
//! passed to the app as [`ControlRequest::Lock`]; the song's snapshot manager and its
//! morph settings (read from and written to the project). A pad of the Snapshots page
//! that holds a snapshot recalls it natively ([`native_pad`]).

use super::data::set_fn;
use super::{methods, object, state};
use crate::state::{ControlRequest, LockOp, ObjRef, Prop, State};
use mlua::{Lua, MultiValue, Table, Value};

/// The manager's parameters: the scripts' getter and the project's field.
const PARAMS: [(&str, &str); 6] = [
    ("getSnapshotMorphParameter", "morph"),
    ("getSnapshotMorphModeParameter", "morph_mode"),
    ("getSnapshotMorphTimeParameter", "morph_time"),
    ("getSnapshotMorphDestTimeParameter", "morph_grid"),
    ("getNumSnapshotBanksParameter", "banks"),
    ("getFocusSnapshotBankIndexParameter", "focus_bank"),
];

fn lock(st: &State) -> deckard_model::Lock {
    st.project().map(|p| p.lock.clone()).unwrap_or_default()
}

fn push(lua: &Lua, op: LockOp) {
    state(lua).borrow_mut().control_requests.push(ControlRequest::Lock(op));
}

/// The manager's settings, read.
pub(super) fn live(st: &State, owner: &ObjRef, name: &str) -> Option<Prop> {
    if !matches!(owner, ObjRef::LockManager) {
        return None;
    }
    let l = lock(st);
    Some(match name {
        "morph" => Prop::Bool(l.morph),
        "morph_mode" => Prop::Int(l.mode as i64),
        "morph_time" => Prop::Int(l.time as i64),
        "morph_grid" => Prop::Int(l.grid as i64),
        "banks" => Prop::Int(l.banks as i64),
        "focus_bank" => Prop::Int(l.focus_bank as i64),
        _ => return None,
    })
}

/// …written (to the project); false if it isn't one.
pub(super) fn write(st: &mut State, owner: &ObjRef, name: &str, v: &Prop) -> bool {
    if !matches!(owner, ObjRef::LockManager) || !PARAMS.iter().any(|(_, f)| *f == name) {
        return false;
    }
    let n = match v {
        Prop::Bool(b) => f64::from(u8::from(*b)),
        Prop::Int(i) => *i as f64,
        Prop::Num(n) => *n,
        _ => return true,
    };
    if let Some(doc) = st.doc.clone() {
        let field = format!("parameter_snapshots/{name}");
        let _ = doc.borrow_mut().set("Lock", deckard_doc::Target::Song, &field, mxprj::decode::Value::Float(n));
    }
    true
}

/// Item `i` of a setting's choices.
pub(super) fn list_string(owner: &ObjRef, name: &str, i: i64) -> Option<String> {
    if !matches!(owner, ObjRef::LockManager) {
        return None;
    }
    let i = usize::try_from(i).ok()?;
    let list: &[&str] = match name {
        "morph_mode" => &deckard_model::LOCK_MODES,
        "morph_time" => &deckard_model::LOCK_TIMES,
        "morph_grid" => &deckard_model::LOCK_GRIDS,
        _ => return None,
    };
    list.get(i).map(|s| s.to_string())
}

pub(super) fn is_enum(owner: &ObjRef, name: &str) -> bool {
    matches!(owner, ObjRef::LockManager) && matches!(name, "morph_mode" | "morph_time" | "morph_grid")
}

pub(super) fn max(owner: &ObjRef, name: &str) -> Option<i64> {
    if !matches!(owner, ObjRef::LockManager) {
        return None;
    }
    Some(match name {
        "morph_mode" => 1,
        "morph_time" => 5,
        "morph_grid" => 3,
        "banks" => 4,
        "focus_bank" => 3,
        _ => return None,
    })
}

/// A pad of the Snapshots page holding a snapshot recalls it (Maschine does it natively;
/// the scripts only take and delete). `index` is 1-based.
pub(crate) fn native_pad(st: &mut State, on_snapshots_page: bool, shift_or_erase: bool, index: i64) {
    if !on_snapshots_page || shift_or_erase {
        return;
    }
    let Some(pad) = index.checked_sub(1).and_then(|i| usize::try_from(i).ok()).filter(|i| *i < 16) else { return };
    let l = lock(st);
    if l.slots.contains(&(l.focus_bank * 16 + pad)) {
        st.control_requests.push(ControlRequest::Lock(LockOp::Pad(pad)));
    }
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let song = methods(lua, impl_t, "Song")?;
    set_fn(lua, &song, "getParameterSnapshotsManager", |lua, _: Value| object(lua, ObjRef::LockManager))?;
    let m = methods(lua, impl_t, "ParameterSnapshotsManager")?;
    for (f, name) in PARAMS {
        set_fn(lua, &m, f, move |lua, _: Value| {
            let owner = state(lua).borrow_mut().objects.intern(ObjRef::LockManager);
            object(lua, ObjRef::Param { owner, name: name.to_string() })
        })?;
    }
    let psa = methods(lua, impl_t, "NI.DATA.ParameterSnapshotsAccess")?;
    let pad_of = |args: &MultiValue| args.get(1).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))).unwrap_or(-1);
    set_fn(lua, &psa, "isSnapshotActiveHW", |lua, _: MultiValue| Ok(state(lua).borrow().lock_active))?;
    set_fn(lua, &psa, "setSnapshotActiveHW", |lua, _: MultiValue| {
        push(lua, LockOp::Button);
        Ok(())
    })?;
    set_fn(lua, &psa, "getFocusSnapshotBankIndex", |lua, _: MultiValue| Ok(lock(&state(lua).borrow()).focus_bank as i64))?;
    set_fn(lua, &psa, "getFocusSnapshotIndex", |lua, _: MultiValue| Ok(lock(&state(lua).borrow()).focus.map_or(-1, |f| f as i64)))?;
    set_fn(lua, &psa, "hasFocusSnapshot", |lua, _: MultiValue| Ok(lock(&state(lua).borrow()).focus.is_some()))?;
    set_fn(lua, &psa, "isSnapshot", move |lua, args: MultiValue| {
        let i = pad_of(&args);
        Ok(i >= 0 && lock(&state(lua).borrow()).slots.contains(&(i as usize)))
    })?;
    set_fn(lua, &psa, "isFocusSnapshot", move |lua, args: MultiValue| {
        let i = pad_of(&args);
        Ok(i >= 0 && lock(&state(lua).borrow()).focus == Some(i as usize))
    })?;
    set_fn(lua, &psa, "isSnapshotForPad", move |lua, args: MultiValue| {
        let l = lock(&state(lua).borrow());
        let i = pad_of(&args);
        Ok(i >= 0 && l.slots.contains(&(l.focus_bank * 16 + i as usize)))
    })?;
    set_fn(lua, &psa, "isFocusSnapshotForPad", move |lua, args: MultiValue| {
        let l = lock(&state(lua).borrow());
        let i = pad_of(&args);
        Ok(i >= 0 && l.focus == Some(l.focus_bank * 16 + i as usize))
    })?;
    set_fn(lua, &psa, "createParameterSnapshotFromPad", move |lua, args: MultiValue| {
        let i = pad_of(&args);
        if (0..16).contains(&i) {
            push(lua, LockOp::Pad(i as usize));
        }
        Ok(())
    })?;
    set_fn(lua, &psa, "deleteParameterSnapshotFromPad", move |lua, args: MultiValue| {
        let i = pad_of(&args);
        let bank = lock(&state(lua).borrow()).focus_bank;
        if (0..16).contains(&i) {
            push(lua, LockOp::DeleteSlot(bank * 16 + i as usize));
        }
        Ok(())
    })?;
    set_fn(lua, &psa, "applyParameterSnapshot", |lua, _: MultiValue| {
        push(lua, LockOp::Update);
        Ok(())
    })?;
    set_fn(lua, &psa, "deleteFocusParameterSnapshot", |lua, _: MultiValue| {
        push(lua, LockOp::Delete);
        Ok(())
    })?;
    set_fn(lua, &psa, "addSnapshotBank", |lua, _: MultiValue| {
        push(lua, LockOp::Bank(true));
        Ok(())
    })?;
    Ok(())
}
