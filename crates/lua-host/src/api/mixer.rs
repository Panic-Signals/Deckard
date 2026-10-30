//! The controller's Mixer page: what its strips meter. The app gives the host each
//! sound's level (its meters, peak since the last frame, falling) and each channel's
//! headroom (the loudest since it was reset) with [`crate::Host::set_levels`];
//! `Group`/`Sound`/`Song` `getLevel(channel)` and `getHeadRoom()` read them, and
//! `NI.UTILS.LevelScale` turns them into the meters' scale (−60 dB … +10 dB) and text.

use super::data::set_fn;
use super::{methods, object_ref, state};
use crate::state::{ObjRef, State};
use mlua::{Lua, Table, Value};

/// The meters' range in dB.
const LOW_DB: f64 = -60.0;
const HIGH_DB: f64 = 10.0;

/// A channel's level (left, right): a sound's, a group's (its sounds' loudest), the
/// master's.
fn level(st: &State, owner: &ObjRef) -> (f32, f32) {
    let l = &st.levels;
    let loudest = |row: &[(f32, f32)]| row.iter().fold((0f32, 0f32), |m, v| (m.0.max(v.0), m.1.max(v.1)));
    match owner {
        ObjRef::Sound(g, s) => l.sounds.get(*g).and_then(|r| r.get(*s)).copied().unwrap_or_default(),
        ObjRef::Group(g) => l.sounds.get(*g).map(|r| loudest(r)).unwrap_or_default(),
        ObjRef::Song => l.master,
        _ => (0.0, 0.0),
    }
}

/// A channel's headroom: its loudest since reset (amplitude).
fn headroom(st: &State, owner: &ObjRef) -> f32 {
    let key = match owner {
        ObjRef::Sound(g, s) => (Some(*g), Some(*s)),
        ObjRef::Group(g) => (Some(*g), None),
        _ => (None, None),
    };
    st.levels.headroom.get(&key).copied().unwrap_or(0.0)
}

fn db_of(amplitude: f64) -> f64 {
    if amplitude > 0.0 { 20.0 * amplitude.log10() } else { f64::NEG_INFINITY }
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    for kind in ["Group", "Sound", "Song"] {
        let t = methods(lua, impl_t, kind)?;
        // getLevel(channel): 0 left, 1 right (amplitude).
        set_fn(lua, &t, "getLevel", |lua, (this, ch): (Value, Option<i64>)| {
            let Some(owner) = object_ref(lua, &this) else { return Ok(0.0) };
            let (l, r) = level(&state(lua).borrow(), &owner);
            Ok(f64::from(if ch == Some(1) { r } else { l }))
        })?;
        set_fn(lua, &t, "getHeadRoom", |lua, this: Value| {
            let Some(owner) = object_ref(lua, &this) else { return Ok(0.0) };
            Ok(f64::from(headroom(&state(lua).borrow(), &owner)))
        })?;
    }
    let scale = methods(lua, impl_t, "NI.UTILS.LevelScale")?;
    // amplitudeScaling2LevelApprox(amplitude, floor): where it sits on the meter, 0..1
    // over −60 … +10 dB (0 at or under `floor`).
    scale.set(
        "amplitudeScaling2LevelApprox",
        lua.create_function(|_, (amp, floor): (f64, Option<f64>)| {
            if amp <= floor.unwrap_or(0.0) {
                return Ok(0.0);
            }
            Ok(((db_of(amp) - LOW_DB) / (HIGH_DB - LOW_DB)).clamp(0.0, 1.0))
        })?,
    )?;
    // convertLevelTodB(amplitude, low, high): in dB, within low..high.
    scale.set(
        "convertLevelTodB",
        lua.create_function(|_, (amp, lo, hi): (f64, Option<f64>, Option<f64>)| {
            let (lo, hi) = (lo.unwrap_or(LOW_DB), hi.unwrap_or(HIGH_DB));
            Ok(db_of(amp).clamp(lo, hi))
        })?,
    )?;
    // dBValueToString(dB, low): "-3.2", "-inf" at or under `low`.
    scale.set(
        "dBValueToString",
        lua.create_function(|_, (db, lo): (f64, Option<f64>)| {
            Ok(if db <= lo.unwrap_or(LOW_DB) { "-inf".to_string() } else { format!("{:.1}", db + 0.0) })
        })?,
    )?;
    Ok(())
}
