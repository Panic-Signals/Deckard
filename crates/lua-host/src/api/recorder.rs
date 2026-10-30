//! The focused sound's recorder, as the Sampling page's RECORD screen reads it
//! (`NI.DATA.StateHelper.getFocusRecorder`): its mode, the Detect threshold, whether it
//! waits or records, and the input levels its meter shows. The app's recorder fills
//! [`State::record`] every frame. START / STOP and CANCEL (`NI.DATA.RecorderAccess`)
//! leave a [`RecordPress`] for the app. Its knobs are the Sampling page's
//! (`api::sampling`).

use super::data::set_fn;
use super::{methods, object, state};
use crate::state::{ObjRef, Prop, RecordPress, State};
use mlua::{Lua, MultiValue, Table, Value};

/// The Detect threshold's range, in dB (as the app's RECORD tab).
pub(crate) const THRESHOLD_MIN: f64 = -60.0;
const THRESHOLD: &str = "DetectThreshold";
const MODE: &str = "RecordingMode";
const FLAG: &str = "RecordingFlag";

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let sh = methods(lua, impl_t, "NI.DATA.StateHelper")?;
    set_fn(lua, &sh, "getFocusRecorder", |lua, _: MultiValue| {
        let has_sound = {
            let st = state(lua);
            let st = st.borrow();
            st.project().is_some_and(|p| p.groups.get(st.focus.group).is_some_and(|g| st.focus.sound < g.sounds.len()))
        };
        if has_sound { object(lua, ObjRef::Recorder) } else { Ok(Value::Nil) }
    })?;
    let rec = methods(lua, impl_t, "Recorder")?;
    set_fn(lua, &rec, "isWaiting", |lua, _: MultiValue| Ok(state(lua).borrow().record.waiting))?;
    set_fn(lua, &rec, "isRecording", |lua, _: MultiValue| Ok(state(lua).borrow().record.recording))?;
    // The meter's levels (left, right; linear): `RecordMeter:setLevels(...)`.
    set_fn(lua, &rec, "getLevelGUIReadout", |lua, _: MultiValue| {
        let (l, r) = state(lua).borrow().record.levels;
        Ok((f64::from(l), f64::from(r)))
    })?;
    for (method, name) in [("getDetectThreshold", THRESHOLD), ("getRecordingFlag", FLAG)] {
        set_fn(lua, &rec, method, move |lua, _: MultiValue| {
            let owner = state(lua).borrow_mut().objects.intern(ObjRef::Recorder);
            object(lua, ObjRef::Param { owner, name: name.to_string() })
        })?;
    }
    let access = methods(lua, impl_t, "NI.DATA.RecorderAccess")?;
    for (method, press) in [("toggleRecording", RecordPress::Toggle), ("requestCancelRecording", RecordPress::Cancel)] {
        set_fn(lua, &access, method, move |lua, _: MultiValue| {
            state(lua).borrow_mut().record_press = Some(press);
            Ok(())
        })?;
    }

    Ok(())
}

pub(super) fn live(st: &State, owner: &ObjRef, name: &str) -> Option<Prop> {
    let r = &st.record;
    Some(match (owner, name) {
        (ObjRef::Recorder, MODE) => Prop::Int(r.mode),
        (ObjRef::Recorder, THRESHOLD) => Prop::Num(r.threshold),
        (ObjRef::Recorder, FLAG) => Prop::Bool(r.recording),
        _ => return None,
    })
}

/// The smallest value of a recorder parameter (others start at 0).
pub(super) fn min(owner: &ObjRef, name: &str) -> Option<f64> {
    matches!((owner, name), (ObjRef::Recorder, THRESHOLD)).then_some(THRESHOLD_MIN)
}

/// The largest value of a recorder parameter.
pub(super) fn max(owner: &ObjRef, name: &str) -> Option<i64> {
    match (owner, name) {
        (ObjRef::Recorder, THRESHOLD) => Some(0),
        (ObjRef::Recorder, MODE) => Some(3),
        _ => None,
    }
}
