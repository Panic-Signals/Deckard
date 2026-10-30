//! Macros on the controller (docs/superpowers/plans/2026-10-23-phase7-macros.md): MACRO
//! shows the channel's Macro properties in the Control area (Maschine switches it
//! natively when the page shows, [`page_pushed`]); SHIFT + MACRO's Set Macro page asks
//! the app to make a knob's parameter a macro ([`ControlRequest::SetMacro`]) and shows
//! which knobs are ones, read from the project's macro pages.

use super::{methods, new_object, object, state};
use crate::state::{ControlRequest, MACRO_PAGE_GROUP, ObjRef, State};
use deckard_doc::Target;
use mlua::{Lua, MultiValue, Table, Value};

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let ppa = methods(lua, impl_t, "NI.DATA.ParameterPageAccess")?;
    ppa.set("isMacroPageActive", lua.create_function(|lua, _: MultiValue| Ok(macro_page_active(&state(lua).borrow())))?)?;
    ppa.set(
        "setMacroPageActive",
        lua.create_function(|lua, _: MultiValue| {
            show_macro_properties(&mut state(lua).borrow_mut());
            Ok(())
        })?,
    )?;
    // anyMacrosForParameter(App, Param, Level): a macro of the master, the focused group
    // or the focused sound controls the knob's parameter.
    ppa.set(
        "anyMacrosForParameter",
        lua.create_function(|lua, (_app, p, level): (Value, Value, i64)| {
            let Some(pos) = super::control::control_pos(lua, &p) else { return Ok(false) };
            let st = state(lua);
            let st = st.borrow();
            let Some(param) = st.control.knobs.get(pos).and_then(|k| k.as_ref()?.param) else { return Ok(false) };
            let Some(doc) = st.doc.clone() else { return Ok(false) };
            let channel = channel(&st, level);
            Ok(doc.borrow().macro_pages(channel).iter().any(|pg| pg.macros.iter().any(|m| m.target == Some(param))))
        })?,
    )?;
    ppa.set(
        "setOrRemoveMacroForParameter",
        lua.create_function(|lua, (_app, p, level): (Value, Value, i64)| {
            if let Some(pos) = super::control::control_pos(lua, &p) {
                state(lua).borrow_mut().control_requests.push(ControlRequest::SetMacro { pos, level: level.clamp(0, 2) as usize });
            }
            Ok(())
        })?,
    )?;

    // The state cache's parameter cache: whether a knob's parameter is modulated.
    let sc = methods(lua, impl_t, "StateCache")?;
    sc.set("getParameterCache", lua.create_function(|lua, _: Value| new_object(lua, "ParameterCache", 0))?)?;
    let pc = methods(lua, impl_t, "ParameterCache")?;
    // The shown page of the Control area.
    pc.set(
        "getPageParameter",
        lua.create_function(|lua, _: Value| {
            let owner = state(lua).borrow_mut().objects.intern(ObjRef::ControlKnobs);
            object(lua, ObjRef::Param { owner, name: "Page".into() })
        })?,
    )?;
    pc.set(
        "isAutomated",
        lua.create_function(|lua, (_, p): (Value, Value)| {
            let Some(pos) = super::control::control_pos(lua, &p) else { return Ok(false) };
            Ok(state(lua).borrow().control.knobs.get(pos).is_some_and(|k| k.as_ref().is_some_and(|k| k.modulated)))
        })?,
    )?;
    Ok(())
}

/// The Control area shows the channel's Macro properties.
fn macro_page_active(st: &State) -> bool {
    !st.control.modules_visible && st.control.page_group == MACRO_PAGE_GROUP
}

/// The channel of a level (`NI.DATA.LEVEL_TAB_SONG`, `_GROUP`, `_SOUND`) at the focus.
fn channel(st: &State, level: i64) -> Target {
    match level {
        0 => Target::Song,
        1 => Target::Group(st.focus.group),
        _ => Target::Sound(st.focus.group, st.focus.sound),
    }
}

/// Show the channel's Macro properties (asking the app for what changes).
fn show_macro_properties(st: &mut State) {
    if st.control.modules_visible {
        st.control.modules_visible = false;
        st.control_requests.push(ControlRequest::ModulesVisible(false));
    }
    if st.control.page_group != MACRO_PAGE_GROUP {
        st.control.page_group = MACRO_PAGE_GROUP;
        st.control_requests.push(ControlRequest::PageGroup(MACRO_PAGE_GROUP));
    }
}

/// A page was pushed onto the stack: the MACRO page shows the Macro properties.
pub(crate) fn page_pushed(lua: &Lua, page: i64) {
    let macro_page: Option<i64> = lua.load("return NI.HW.PAGE_MACRO").eval().ok();
    if macro_page == Some(page) {
        show_macro_properties(&mut state(lua).borrow_mut());
    }
}
