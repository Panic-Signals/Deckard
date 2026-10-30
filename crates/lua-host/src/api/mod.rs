//! Rust implementations of the API the scripts call, registered by type name in the
//! prelude's dispatch table (`IMPL[type][key]`). Types are the proxy names: `NI.HW`,
//! `NHLController`, `PageStack`, and object types such as `Song` or `Group`.

mod arp;
pub(crate) mod arranger;
pub(crate) mod browser;
mod control;
mod data;
pub(crate) mod events;
mod gui;
mod hw;
pub(crate) mod lock;
mod macros;
mod mixer;
pub mod nsa;
pub(crate) mod pad_mode;
pub(crate) mod pad_touch;
pub mod pattern_editor;
pub(crate) mod perform;
pub(crate) mod plugins;
mod recorder;
mod sampling;
use deckard_model::scale_tables;
pub mod scales;
pub(crate) mod sync;
pub mod utils;

use crate::State;
use crate::state::ObjRef;
use mlua::{Function, Lua, Table, Value};
use std::cell::RefCell;
use std::rc::Rc;

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    hw::install(lua, impl_t)?;
    data::install(lua, impl_t)?;
    events::install(lua, impl_t)?;
    pattern_editor::install(lua, impl_t)?;
    arranger::install(lua, impl_t)?;
    recorder::install(lua, impl_t)?;
    sampling::install(lua, impl_t)?;
    browser::install(lua, impl_t)?;
    arp::install(lua, impl_t)?;
    control::install(lua, impl_t)?;
    plugins::install(lua, impl_t)?;
    scales::install(lua, impl_t)?;
    gui::install(lua, impl_t)?;
    utils::install(lua, impl_t)?;
    mixer::install(lua, impl_t)?;
    perform::install(lua, impl_t)?;
    lock::install(lua, impl_t)?;
    nsa::install(lua, impl_t)?;
    sync::install(lua, impl_t)?;
    macros::install(lua, impl_t)?;
    Ok(())
}

/// The `IMPL[ty]` table, created on first use.
pub(crate) fn methods(lua: &Lua, impl_t: &Table, ty: &str) -> mlua::Result<Table> {
    match impl_t.get::<Option<Table>>(ty)? {
        Some(t) => Ok(t),
        None => {
            let t = lua.create_table()?;
            impl_t.set(ty, t.clone())?;
            Ok(t)
        }
    }
}

pub(crate) fn state(lua: &Lua) -> Rc<RefCell<State>> {
    lua.app_data_ref::<Rc<RefCell<State>>>().expect("host state").clone()
}

/// The proxy object of type `ty` carrying engine id `id` (see `prelude.lua`). The same
/// (type, id) always yields the same table, so scripts can compare objects with `==`.
pub fn new_object(lua: &Lua, ty: &str, id: usize) -> mlua::Result<Value> {
    let host: Table = lua.globals().get("__host")?;
    let cache: Table = match host.raw_get::<Option<Table>>("objects")? {
        Some(c) => c,
        None => {
            let c = lua.create_table()?;
            host.raw_set("objects", c.clone())?;
            c
        }
    };
    let key = format!("{ty}:{id}");
    if let Some(t) = cache.raw_get::<Option<Table>>(key.as_str())? {
        return Ok(Value::Table(t));
    }
    let new_proxy: Function = host.get("new_proxy")?;
    let fields = lua.create_table()?;
    fields.raw_set("__id", id)?;
    let v: Value = new_proxy.call((ty, fields))?;
    cache.raw_set(key, v.clone())?;
    Ok(v)
}

/// The proxy for an engine object.
pub fn object(lua: &Lua, r: ObjRef) -> mlua::Result<Value> {
    let ty = r.type_name();
    let id = state(lua).borrow_mut().objects.intern(r);
    new_object(lua, ty, id)
}

/// The engine object behind a proxy argument.
pub fn object_ref(lua: &Lua, v: &Value) -> Option<ObjRef> {
    let id = object_id(v)?;
    let ty = object_type(v)?;
    let st = state(lua);
    let st = st.borrow();
    st.objects.get(id).filter(|r| r.type_name() == ty).cloned()
}

/// Engine id of a proxy object made by [`new_object`].
pub fn object_id(v: &Value) -> Option<usize> {
    match v {
        Value::Table(t) => t.raw_get::<Option<usize>>("__id").ok().flatten(),
        _ => None,
    }
}

pub fn object_type(v: &Value) -> Option<String> {
    match v {
        Value::Table(t) => t.raw_get::<Option<String>>("__type").ok().flatten(),
        _ => None,
    }
}
