//! Link and MIDI sync on the controller (Settings ▸ MIDI;
//! docs/superpowers/plans/2026-10-23-phase9-link-midi-clock.md): the scripts' `NSA`
//! MIDI sync and device calls, the hardware preferences' receive offset, the
//! Workspace's Link parameters, and the Link status and phase widgets, all answered
//! from (and written to) [`SyncView`], which the app keeps in step with its own.

use super::data::set_fn;
use super::{methods, state};
use crate::state::{ObjRef, Prop, State, SyncView};
use mlua::{Lua, Table, Value};

/// The settings page's right column (pixels), which the Link widgets fill; the phase
/// meter's bar within it (less its outer margins), and its indicator at a full bar
/// (less the bar's margins).
const COLUMN: i64 = 240;
const PHASE_BAR: i64 = COLUMN - 2 * 44;
const PHASE_WIDTH: f64 = (PHASE_BAR - 2 * 4) as f64;

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let nsa = methods(lua, impl_t, "NSA")?;
    set_fn(lua, &nsa, "getMidiSyncStatus", |lua, _: Value| Ok(state(lua).borrow().sync.midi_sync))?;
    // (Receive Clock isn't offered while Link is on: the scripts keep within their list.)
    set_fn(lua, &nsa, "setMidiSyncStatus", |lua, (_, v): (Value, i64)| {
        state(lua).borrow_mut().sync.midi_sync = v.clamp(0, 2);
        Ok(())
    })?;
    set_fn(lua, &nsa, "getCurrentClockOffset", |lua, _: Value| Ok(state(lua).borrow().sync.send_offset))?;
    set_fn(lua, &nsa, "setClockOffset", |lua, (_, v): (Value, i64)| {
        state(lua).borrow_mut().sync.send_offset = v.clamp(0, 200);
        Ok(())
    })?;
    // MIDI devices (`Input`: inputs, else outputs), from 0.
    set_fn(lua, &nsa, "getMidiDevicesCount", |lua, (_, input): (Value, bool)| Ok(ports(&state(lua).borrow().sync, input).len()))?;
    set_fn(lua, &nsa, "getMidiDeviceString", |lua, (_, i, input): (Value, usize, bool)| {
        Ok(ports(&state(lua).borrow().sync, input).get(i).map(|p| p.0.clone()).unwrap_or_default())
    })?;
    set_fn(lua, &nsa, "getMidiDeviceState", |lua, (_, i, input): (Value, usize, bool)| {
        Ok(ports(&state(lua).borrow().sync, input).get(i).is_some_and(|p| p.1))
    })?;
    set_fn(lua, &nsa, "toggleMidiDevice", |lua, (_, i, input): (Value, usize, bool)| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        let list = if input { &mut st.sync.inputs } else { &mut st.sync.outputs };
        if let Some(p) = list.get_mut(i) {
            p.1 = !p.1;
        }
        Ok(())
    })?;
    // Leaving the tab stores the setup, and toggling a device reopens the ports: the app
    // does both when its settings change.
    set_fn(lua, &nsa, "storeSetup", |_, _: Value| Ok(()))?;
    set_fn(lua, &methods(lua, impl_t, "App")?, "updateMidiDevices", |_, _: Value| Ok(()))?;
    let prefs = methods(lua, impl_t, "NI.HW.getMaschinePreferences()")?;
    set_fn(lua, &prefs, "getMidiClockSyncOffset", |lua, _: Value| Ok(state(lua).borrow().sync.receive_offset))?;
    set_fn(lua, &prefs, "setMidiClockSyncOffset", |lua, (_, _app, v): (Value, Value, i64)| {
        state(lua).borrow_mut().sync.receive_offset = v.clamp(0, 200);
        Ok(())
    })?;
    // DEFAULT (MIDI input of sounds set to Default): NI.DATA.DEFAULT_MIDI_INPUT_FOCUS 0,
    // _NONE 1.
    let general = methods(lua, impl_t, "NI.APP.Preferences.getGeneral()")?;
    set_fn(lua, &general, "getDefaultMidiInput", |lua, _: Value| Ok(if state(lua).borrow().sync.default_focus { 0 } else { 1 }))?;
    set_fn(lua, &general, "setDefaultMidiInput", |lua, (_, v): (Value, i64)| {
        state(lua).borrow_mut().sync.default_focus = v == 0;
        Ok(())
    })?;
    Ok(())
}

fn ports(s: &SyncView, input: bool) -> &[(String, bool)] {
    if input { &s.inputs } else { &s.outputs }
}

/// The Workspace's Link parameters, read.
pub(super) fn live(st: &State, owner: &ObjRef, name: &str) -> Option<Prop> {
    match (owner, name) {
        (ObjRef::Workspace, "LinkEnabled") => Some(Prop::Bool(st.sync.link)),
        (ObjRef::Workspace, "LinkStartStopSyncEnabled") => Some(Prop::Bool(st.sync.link_start_stop)),
        _ => None,
    }
}

/// …written; false if it isn't one.
pub(super) fn write(st: &mut State, owner: &ObjRef, name: &str, v: &Prop) -> bool {
    let on = matches!(v, Prop::Bool(true)) || matches!(v, Prop::Int(n) if *n != 0);
    match (owner, name) {
        (ObjRef::Workspace, "LinkEnabled") => {
            st.sync.link = on;
            // Link takes priority over receiving clock.
            if on && st.sync.midi_sync == 2 {
                st.sync.midi_sync = 0;
            }
        }
        (ObjRef::Workspace, "LinkStartStopSyncEnabled") => st.sync.link_start_stop = on,
        _ => return false,
    }
    refresh(st);
    true
}

/// Give a Link status or phase meter widget its parts as the style sheet has them: a
/// label; a progress bar with its indicator.
pub(super) fn link_widget(st: &mut State, kind: &str, id: usize) {
    match kind {
        "HWLinkStatus" => {
            // (The label as wide as the column, so its text centres.)
            let label = st.widgets.insert(Some(id), "Label", "");
            st.widgets.set(label, "Width", Prop::Int(COLUMN));
        }
        "HWLinkPhaseMeter" => {
            let bar = st.widgets.insert(Some(id), "ProgressBar", "");
            st.widgets.set(bar, "Width", Prop::Int(PHASE_BAR));
            st.widgets.insert(Some(bar), "Label", "indicator");
        }
        _ => return,
    }
    st.widgets.set(id, "Width", Prop::Int(COLUMN));
    refresh(st);
}

/// The Link widgets show the app's Link: "LINK OFF", or how many peers ("1 LINK", "2
/// LINKS"); the phase meter fills over the bar.
pub(crate) fn refresh(st: &mut State) {
    let s = &st.sync;
    let text = match (s.link, s.peers) {
        (false, _) => "LINK OFF".to_string(),
        (true, 1) => "1 LINK".to_string(),
        (true, n) => format!("{n} LINKS"),
    };
    let fill = if s.link { (s.phase.clamp(0.0, 1.0) * PHASE_WIDTH).round() as i64 } else { 0 };
    let child = |st: &State, parent: usize| st.widgets.all.iter().position(|w| w.parent == Some(parent));
    let statuses: Vec<usize> = st.widgets.all.iter().enumerate().filter(|(_, w)| w.kind == "HWLinkStatus").map(|(i, _)| i).collect();
    for id in statuses {
        if let Some(label) = child(st, id) {
            st.widgets.set(label, "Text", Prop::Str(text.clone()));
        }
    }
    let meters: Vec<usize> = st.widgets.all.iter().enumerate().filter(|(_, w)| w.kind == "HWLinkPhaseMeter").map(|(i, _)| i).collect();
    for id in meters {
        if let Some(indicator) = child(st, id).and_then(|bar| child(st, bar)) {
            st.widgets.set(indicator, "Width", Prop::Int(fill));
        }
    }
}
