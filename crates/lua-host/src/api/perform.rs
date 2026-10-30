//! Perform FX on the controller (docs/superpowers/plans/2026-10-22-phase5-perform-fx.md):
//! a group's Perform FX for the scripts (`Group:getPerformanceFX()`, its strip
//! parameters, which the touch strip's LEDs show), adding one (SHIFT + PERFORM,
//! `SongAccess.loadPerformanceFX`) and the strip playing it in PERFORM mode, which
//! Maschine does natively: the app engages and moves it ([`ControlRequest::PerformStrip`]).

use super::data::set_fn;
use super::{methods, object, state};
use crate::state::{ControlRequest, ObjRef, Prop, State};
use mlua::{Lua, MultiValue, Table, Value};

const CLASS: &str = "PerformanceFX";

/// Group `g`'s Perform FX's parameters, if it has one.
fn params(st: &State, g: usize) -> Option<Vec<(String, f64)>> {
    let p = st.project()?;
    p.groups.get(g)?.slots.iter().find(|s| s.module == CLASS).map(|s| s.params.clone())
}

/// Whether group `g` has a Perform FX.
pub(crate) fn has(st: &State, g: usize) -> bool {
    params(st, g).is_some()
}

/// Its parameters, read.
pub(super) fn live(st: &State, owner: &ObjRef, name: &str) -> Option<Prop> {
    let ObjRef::PerformFx(g) = owner else { return None };
    let v = params(st, *g)?.into_iter().find(|(k, _)| k == name)?.1;
    Some(if name == "touchstrip_on_off" { Prop::Bool(v >= 0.5) } else { Prop::Num(v) })
}

fn param(lua: &Lua, g: usize, name: &str) -> mlua::Result<Value> {
    let owner = state(lua).borrow_mut().objects.intern(ObjRef::PerformFx(g));
    object(lua, ObjRef::Param { owner, name: name.to_string() })
}

/// The focused group.
fn focus_group(lua: &Lua) -> usize {
    state(lua).borrow().focus.group
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let group = methods(lua, impl_t, "Group")?;
    set_fn(lua, &group, "getPerformanceFX", |lua, this: Value| {
        let Some(ObjRef::Group(g)) = super::object_ref(lua, &this) else { return Ok(Value::Nil) };
        if has(&state(lua).borrow(), g) { object(lua, ObjRef::PerformFx(g)) } else { Ok(Value::Nil) }
    })?;
    let fx = methods(lua, impl_t, CLASS)?;
    for (f, name) in [
        ("getTouchstripOnOffParameter", "touchstrip_on_off"),
        ("getTouchstripPosParameter", "touchstrip_pos"),
        ("getModeParameter", "mode"),
    ] {
        set_fn(lua, &fx, f, move |lua, this: Value| {
            let Some(ObjRef::PerformFx(g)) = super::object_ref(lua, &this) else { return Ok(Value::Nil) };
            param(lua, g, name)
        })?;
    }
    // Its parameters are all 0–1 (Mode aside): normalized, they're themselves. The
    // touch strip's LEDs place the strip position by it.
    let p = methods(lua, impl_t, "Parameter")?;
    let before: Option<mlua::Function> = p.get("getNormalizedValue").ok();
    set_fn(lua, &p, "getNormalizedValue", move |lua, this: Value| {
        if let Some(ObjRef::Param { owner, name }) = super::object_ref(lua, &this)
            && let Some(owner) = state(lua).borrow().objects.get(owner).cloned()
            && let Some(Prop::Num(v)) = live(&state(lua).borrow(), &owner, &name)
        {
            return Ok(Value::Number(v));
        }
        if let Some(f) = &before {
            return f.call::<Value>(this);
        }
        // Others: the value within its range (0 for one that has none).
        let t = match &this {
            Value::Table(t) => t,
            _ => return Ok(Value::Number(0.0)),
        };
        let num = |m: &str| -> Option<f64> {
            let v: Value = t.get::<mlua::Function>(m).ok()?.call(this.clone()).ok()?;
            match v {
                Value::Number(n) => Some(n),
                Value::Integer(i) => Some(i as f64),
                Value::Boolean(b) => Some(f64::from(u8::from(b))),
                _ => None,
            }
        };
        Ok(Value::Number(match (num("getValue"), num("getMin"), num("getMax")) {
            (Some(v), Some(lo), Some(hi)) if hi > lo => ((v - lo) / (hi - lo)).clamp(0.0, 1.0),
            _ => 0.0,
        }))
    })?;
    // canAddPerformanceFX(MixingLayer): a group without one.
    let sh = methods(lua, impl_t, "NI.DATA.StateHelper")?;
    set_fn(lua, &sh, "canAddPerformanceFX", |lua, layer: MultiValue| {
        let g = match layer.front().and_then(|v| super::object_ref(lua, v)) {
            Some(ObjRef::Group(g)) => g,
            _ => focus_group(lua),
        };
        let st = state(lua);
        let st = st.borrow();
        Ok(st.project().is_some_and(|p| g < p.groups.len()) && !has(&st, g))
    })?;
    // loadPerformanceFX(App, …): one on the focused group (if it has none), shown.
    let song = methods(lua, impl_t, "NI.DATA.SongAccess")?;
    set_fn(lua, &song, "loadPerformanceFX", |lua, _: MultiValue| {
        let g = focus_group(lua);
        let doc = state(lua).borrow().doc.clone();
        if let Some(doc) = doc
            && !has(&state(lua).borrow(), g)
            && doc.borrow_mut().add_perform_fx(g).is_err()
        {
            return Ok(());
        }
        state(lua).borrow_mut().control_requests.push(ControlRequest::ShowPerformFx(g));
        Ok(())
    })?;
    Ok(())
}
