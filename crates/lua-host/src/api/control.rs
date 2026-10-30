//! The Control page's view of the app's Control area (`State::control`): the focused
//! chain's slots (`NI.DATA.StateHelper.getFocusSlot` …), the focused slot's pages and
//! knobs (`NI.DATA.ParameterCache`), and the parameter text helpers
//! (`NI.GUI.ParameterWidgetHelper`). What the scripts change becomes a
//! [`ControlRequest`] for the app to carry out with its own edits.

use super::{methods, object, object_ref, state};
use crate::state::{ControlRequest, ListKind, ObjRef, Prop};
use mlua::{Lua, MultiValue, Table, Value};

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let push = |lua: &Lua, r: ControlRequest| state(lua).borrow_mut().control_requests.push(r);

    // VARIATION on the MK3: the focused sound's Groove page `n` (0 Swing, 1 Variation,
    // 2 Generate) in Control mode.
    let ppa = methods(lua, impl_t, "NI.DATA.ParameterPageAccess")?;
    set(lua, &ppa, "setFocusPageOnGroovePageGroup", move |lua, (_app, n): (Value, i64)| {
        push(lua, ControlRequest::GroovePage(n.clamp(0, 2) as usize));
        Ok(())
    })?;

    // Focus.
    let sh = methods(lua, impl_t, "NI.DATA.StateHelper")?;
    set(lua, &sh, "getFocusSlot", |lua, _: MultiValue| {
        let focus = state(lua).borrow().control.focus_slot;
        match focus {
            Some(i) => object(lua, ObjRef::Slot(i)),
            None => Ok(Value::Nil),
        }
    })?;
    set(lua, &sh, "getFocusSlotIndex", |lua, _: MultiValue| Ok(state(lua).borrow().control.focus_slot.map_or(-1, |i| i as i64)))?;
    set(lua, &sh, "getFocusChainSlots", |lua, _: MultiValue| object(lua, ObjRef::List(ListKind::Slots)))?;

    // The focused sound (or group) whose chain the Control page shows.
    set(lua, &sh, "getFocusMixingLayer", |lua, _: MultiValue| {
        let (level, g, s) = {
            let st = state(lua);
            let st = st.borrow();
            (st.control.level, st.focus.group, st.focus.sound)
        };
        match level {
            2 => object(lua, ObjRef::Sound(g, s)),
            1 => object(lua, ObjRef::Group(g)),
            _ => object(lua, ObjRef::Song),
        }
    })?;
    // A slot's name as shown (SlotAlgorithms.getDisplayName, …getSampleOrPresetName).
    let algos = methods(lua, impl_t, "NI.DATA.SlotAlgorithms")?;
    for name in ["getDisplayName", "getSampleOrPresetName"] {
        set(lua, &algos, name, |lua, slot: Value| {
            Ok(match object_ref(lua, &slot) {
                Some(ObjRef::Slot(i)) => state(lua).borrow().control.slots.get(i).map_or(String::new(), |s| s.name.clone()),
                _ => String::new(),
            })
        })?;
    }

    // What changed in the chain since the last controller timer: the focused slot (or
    // what's in it), the slots, one slot's module. The Control page redraws on these.
    let sc = methods(lua, impl_t, "StateCache")?;
    set(lua, &sc, "isFocusSlotChanged", |lua, _: MultiValue| {
        let st = state(lua);
        let st = st.borrow();
        let c = &st.control;
        Ok(st.chain_seen.as_ref().is_none_or(|(focus, slots)| {
            *focus != c.focus_slot || focus.and_then(|i| slots.get(i)) != c.focus_slot.and_then(|i| c.slots.get(i))
        }))
    })?;
    set(lua, &sc, "isFocusChainChanged", |lua, _: MultiValue| Ok(chain_changed(lua)))?;
    for ty in ["Sound", "Group", "Song"] {
        let t = methods(lua, impl_t, ty)?;
        set(lua, &t, "getChain", |lua, _: Value| object(lua, ObjRef::Chain))?;
    }
    let chain_t = methods(lua, impl_t, "Chain")?;
    for name in ["isChanged", "isAnySlotChanged"] {
        set(lua, &chain_t, name, |lua, _: Value| Ok(chain_changed(lua)))?;
    }
    set(lua, &chain_t, "getSlots", |lua, _: Value| object(lua, ObjRef::List(ListKind::Slots)))?;

    // Slots, their modules and the modules' info.
    let slot = methods(lua, impl_t, "Slot")?;
    set(lua, &slot, "getModule", |lua, this: Value| match object_ref(lua, &this) {
        Some(ObjRef::Slot(i)) => object(lua, ObjRef::Module(i)),
        _ => Ok(Value::Nil),
    })?;
    // getResourceLocator(Name) / getBankResourceLocator(Name): what Maschine finds a
    // product's artwork by; DECKARD's are their names.
    for name in ["getResourceLocator", "getBankResourceLocator"] {
        set(lua, &slot, name, |_, (_, n): (Value, Option<String>)| Ok(n.unwrap_or_default()))?;
    }
    set(lua, &slot, "isModuleChanged", |lua, this: Value| {
        let Some(ObjRef::Slot(i)) = object_ref(lua, &this) else { return Ok(false) };
        let st = state(lua);
        let st = st.borrow();
        Ok(st.chain_seen.as_ref().is_none_or(|(_, slots)| slots.get(i).map(|s| &s.name) != st.control.slots.get(i).map(|s| &s.name)))
    })?;
    set(lua, &slot, "getPluginHost", |_, _: Value| Ok(Value::Nil))?;
    set(lua, &slot, "getActiveParameter", |lua, this: Value| {
        let Some(r @ ObjRef::Slot(_)) = object_ref(lua, &this) else { return Ok(Value::Nil) };
        let owner = state(lua).borrow_mut().objects.intern(r);
        object(lua, ObjRef::Param { owner, name: "Active".into() })
    })?;
    let module = methods(lua, impl_t, "Module")?;
    set(lua, &module, "getInfo", |lua, this: Value| match object_ref(lua, &this) {
        Some(ObjRef::Module(i)) => object(lua, ObjRef::ModuleInfo(i)),
        _ => Ok(Value::Nil),
    })?;
    set(lua, &module, "getPluginPopupOpenParameter", |lua, this: Value| {
        let Some(r @ ObjRef::Module(_)) = object_ref(lua, &this) else { return Ok(Value::Nil) };
        let owner = state(lua).borrow_mut().objects.intern(r);
        object(lua, ObjRef::Param { owner, name: "PluginPopupOpen".into() })
    })?;
    let info = methods(lua, impl_t, "ModuleInfo")?;
    set(lua, &info, "getId", |lua, this: Value| {
        if let Some(ObjRef::PluginEntry(i)) = object_ref(lua, &this) {
            return Ok(super::plugins::entry_id(lua, i));
        }
        let Some(ObjRef::ModuleInfo(i)) = object_ref(lua, &this) else { return Ok(-1) };
        let plugin = state(lua).borrow().control.slots.get(i).is_some_and(|s| s.plugin);
        // NI.DATA.ModuleInfo.ID_PLUGINHOST for a plug-in; another id for internal modules.
        let id: i64 = lua.load("return NI.DATA.ModuleInfo.ID_PLUGINHOST").eval().unwrap_or(38);
        Ok(if plugin { id } else { id + 1000 })
    })?;
    // An instrument's or an effect's (the chain's type icons).
    set(lua, &info, "getType", |lua, this: Value| {
        Ok(match object_ref(lua, &this) {
            Some(ObjRef::PluginEntry(i)) => super::plugins::entry_type(lua, i),
            Some(ObjRef::ModuleInfo(i)) => {
                let instrument = state(lua).borrow().control.slots.get(i).is_some_and(|s| s.instrument);
                super::plugins::type_of(lua, instrument)
            }
            _ => -1,
        })
    })?;
    for ty in ["Slot", "Module", "ModuleInfo"] {
        let t = methods(lua, impl_t, ty)?;
        for name in ["getName", "getDisplayName"] {
            set(lua, &t, name, |lua, this: Value| {
                let i = match object_ref(lua, &this) {
                    Some(ObjRef::PluginEntry(i)) => return Ok(super::plugins::entry_name(lua, i)),
                    Some(ObjRef::Slot(i) | ObjRef::Module(i) | ObjRef::ModuleInfo(i)) => i,
                    _ => return Ok(String::new()),
                };
                Ok(state(lua).borrow().control.slots.get(i).map_or(String::new(), |s| s.name.clone()))
            })?;
        }
    }

    // The knobs of the shown page.
    let pc = methods(lua, impl_t, "NI.DATA.ParameterCache")?;
    set(lua, &pc, "getParameterByPosition", |lua, (_app, pos): (Value, i64)| {
        let has = state(lua).borrow().control.knobs.get(pos.max(0) as usize).is_some_and(Option::is_some);
        if !has {
            return Ok(Value::Nil);
        }
        let owner = state(lua).borrow_mut().objects.intern(ObjRef::ControlKnobs);
        object(lua, ObjRef::Param { owner, name: pos.to_string() })
    })?;
    set(lua, &pc, "getNumPages", |lua, _: MultiValue| Ok(state(lua).borrow().control.pages.len() as i64))?;
    set(lua, &pc, "getFocusPage", |lua, _: MultiValue| Ok(state(lua).borrow().control.page as i64))?;
    set(lua, &pc, "advanceFocusPage", move |lua, (_app, next): (Value, bool)| {
        push(lua, ControlRequest::Page(next));
        Ok(())
    })?;
    // The cache is never mid-update here (Maschine's flag for that); the pages redraw
    // whenever they're asked, a plug-in focused or not.
    set(lua, &pc, "isValid", |_, _: MultiValue| Ok(true))?;
    set(lua, &pc, "getActualParameter", |_, p: Value| Ok(p))?;

    // Chains and levels.
    let chain = methods(lua, impl_t, "NI.DATA.ChainAccess")?;
    set(lua, &chain, "shiftSlotFocus", move |lua, (_app, _slots, d): (Value, Value, i64)| {
        push(lua, ControlRequest::Slot(d));
        Ok(())
    })?;
    set(lua, &chain, "removeSlot", move |lua, (_app, _slots, slot): (Value, Value, Value)| {
        if let Some(ObjRef::Slot(i)) = object_ref(lua, &slot) {
            push(lua, ControlRequest::Remove(i));
        }
        Ok(())
    })?;
    // moveSlot(App, Slots, Slot, Before): the slot goes before position `Before` (the
    // scripts pass the focus + 2 to move right, − 1 to move left).
    set(lua, &chain, "moveSlot", move |lua, (_app, _slots, slot, before): (Value, Value, Value, i64)| {
        if let Some(ObjRef::Slot(from)) = object_ref(lua, &slot) {
            let before = before.max(0) as usize;
            let to = if before > from { before - 1 } else { before };
            if to != from {
                push(lua, ControlRequest::Move(from, to));
            }
        }
        Ok(())
    })?;
    let song = methods(lua, impl_t, "NI.DATA.SongAccess")?;
    set(lua, &song, "setLevelTab", move |lua, (_app, _song, level): (Value, Value, i64)| {
        let level = level.clamp(0, 2) as usize;
        state(lua).borrow_mut().control.level = level;
        // The Mixer follows: a group's sounds at the sound level, the groups at the group
        // level (pushing the 4-D encoder down or up in Mix mode).
        if level > 0 {
            let st = state(lua);
            super::data::set_live_param(&mut st.borrow_mut(), &ObjRef::Song, "SoundsVisibleMixer", &Prop::Bool(level == 2));
        }
        push(lua, ControlRequest::Level(level));
        Ok(())
    })?;
    let song_t = methods(lua, impl_t, "Song")?;
    set(lua, &song_t, "getLevelTab", |lua, _: Value| Ok(state(lua).borrow().control.level as i64))?;
    set(lua, &song_t, "isLevelTabChanged", |lua, _: Value| {
        let st = state(lua);
        let st = st.borrow();
        Ok(st.level_seen != Some(st.control.level))
    })?;

    // Text of parameters.
    let helper = methods(lua, impl_t, "NI.GUI.ParameterWidgetHelper")?;
    set(lua, &helper, "getParameterName", |lua, p: Value| {
        Ok(match (knob(lua, &p), super::sampling::knob(lua, &p)) {
            (Some(k), _) => k.name,
            (_, Some(k)) => k.name,
            _ => plain_name(lua, &p),
        })
    })?;
    set(lua, &helper, "getValueString", |lua, (p, _): (Value, MultiValue)| {
        Ok(match (knob(lua, &p), super::sampling::knob(lua, &p)) {
            (Some(k), _) => k.text,
            (_, Some(k)) => k.text,
            _ => {
                let f: mlua::Function = lua.load("return function(p) return p:getValueString() end").eval()?;
                f.call::<String>(p).unwrap_or_default()
            }
        })
    })?;
    // "12.0 %" → ("12.0", "%"); "1.20 kHz" → ("1.20", "kHz"); text alone otherwise
    // ("Chd Set", "Wh Tone").
    set(lua, &helper, "getValueUnitPair", |_, text: Option<String>| {
        let text = text.unwrap_or_default();
        Ok(match text.rsplit_once(' ') {
            Some((v, u)) if v.parse::<f64>().is_ok() && !u.is_empty() && u.chars().all(|c| c.is_alphabetic() || c == '%') => {
                (v.to_string(), u.to_string())
            }
            _ => (text, String::new()),
        })
    })?;
    let param = methods(lua, impl_t, "Parameter")?;
    set(lua, &param, "getSectionName", |lua, p: Value| Ok(knob(lua, &p).map_or(String::new(), |k| k.section)))?;
    set(lua, &param, "isAssigned", |_, _: Value| Ok(true))?;
    // The largest value: the last page group, a note's 127; otherwise 1 (switches, 0..1 knobs).
    set(lua, &param, "getMin", |lua, p: Value| {
        let Some(ObjRef::Param { owner, name }) = object_ref(lua, &p) else { return Ok(0.0) };
        let st = state(lua);
        let st = st.borrow();
        Ok(st.objects.get(owner).and_then(|o| super::recorder::min(o, &name).or_else(|| super::sampling::min(o, &name))).unwrap_or(0.0))
    })?;
    set(lua, &param, "getMax", |lua, p: Value| {
        let Some(ObjRef::Param { owner, name }) = object_ref(lua, &p) else { return Ok(1) };
        let st = state(lua);
        let st = st.borrow();
        Ok(match (st.objects.get(owner), name.as_str()) {
            (Some(ObjRef::Workspace), "PageGroup") => st.control.page_groups.saturating_sub(1) as i64,
            (Some(o), n) => super::scales::max(&st, o, n)
                .or_else(|| super::pad_mode::max(o, n))
                .or_else(|| super::pad_touch::max(o, n))
                .or_else(|| super::pattern_editor::max(n))
                .or_else(|| super::lock::max(o, n))
                .or_else(|| super::arranger::max(n))
                .or_else(|| super::recorder::max(o, n))
                .or_else(|| super::sampling::max(&st, o, n))
                .or_else(|| super::arp::max(&st, o, n))
                .or_else(|| (n == "SwingAmount").then_some(100))
                .unwrap_or(1),
            _ => 1,
        })
    })?;
    Ok(())
}

fn set<A, R>(lua: &Lua, t: &Table, name: &str, f: impl Fn(&Lua, A) -> mlua::Result<R> + 'static) -> mlua::Result<()>
where
    A: mlua::FromLuaMulti,
    R: mlua::IntoLuaMulti,
{
    t.set(name, lua.create_function(f)?)
}

/// The Control-area knob behind a parameter proxy, if it is one.
fn knob(lua: &Lua, p: &Value) -> Option<crate::state::ControlKnob> {
    let pos = control_pos(lua, p)?;
    state(lua).borrow().control.knobs.get(pos)?.clone()
}

/// The knob position of a Control-area parameter.
/// The CHANNEL page's groups, as Maschine names them.
const PAGE_GROUPS: [&str; 4] = ["Input", "Output", "Groove", "Macro"];

/// Item `i` of a list parameter here (`getAsString(i)`): a page group's name.
pub(super) fn list_string(owner: &ObjRef, name: &str, i: i64) -> Option<String> {
    match (owner, name) {
        (ObjRef::Workspace, "PageGroup") => PAGE_GROUPS.get(usize::try_from(i).ok()?).map(|s| s.to_string()),
        _ => None,
    }
}

pub(super) fn control_pos(lua: &Lua, p: &Value) -> Option<usize> {
    let ObjRef::Param { owner, name } = object_ref(lua, p)? else { return None };
    let st = state(lua);
    let st = st.borrow();
    matches!(st.objects.get(owner), Some(ObjRef::ControlKnobs)).then(|| name.parse().ok()).flatten()
}

fn plain_name(lua: &Lua, p: &Value) -> String {
    match object_ref(lua, p) {
        // The Chords page's TYPE knob picks a set in User Chord mode.
        Some(ObjRef::Param { name, .. }) if name == "CurrentChordType" && state(lua).borrow().scale.screen_mode == 2 => "Set".into(),
        Some(ObjRef::Param { name, .. }) => label(&name).map_or(name, str::to_string),
        _ => String::new(),
    }
}

/// Parameters' names as Maschine labels them on the hardware (seen in captures).
fn label(name: &str) -> Option<&'static str> {
    Some(match name {
        "ScaleBank" => "Bank",
        "Scale" | "CurrentChordType" => "Type",
        "ChordScreenChordMode" | "ChordMode" | "ChokeMode" | "LinkMode" => "Mode",
        "ChokeGroup" | "LinkGroup" => "Group",
        // Settings ▸ MIDI's Link (as Preferences ▸ General ▸ Link names it).
        "LinkEnabled" => "Enabled",
        _ => return None,
    })
}

/// A Control-area parameter's or slot's live value (`None`: not one of them).
pub(super) fn live(st: &crate::State, owner: &ObjRef, name: &str) -> Option<Prop> {
    match (owner, name) {
        (ObjRef::ControlKnobs, "Page") => Some(Prop::Int(st.control.page as i64)),
        (ObjRef::ControlKnobs, pos) => st.control.knobs.get(pos.parse::<usize>().ok()?)?.as_ref().map(|k| Prop::Num(k.value)),
        (ObjRef::Slot(i), "Active") => Some(Prop::Bool(st.control.slots.get(*i)?.active)),
        (ObjRef::Workspace, "ModulesVisible") => Some(Prop::Bool(st.control.modules_visible)),
        (ObjRef::Workspace, "PageGroup") => Some(Prop::Int(st.control.page_group as i64)),
        _ => None,
    }
}

/// Write one of them: a request for the app (`true` if it was one).
pub(super) fn write(st: &mut crate::State, owner: &ObjRef, name: &str, v: &Prop) -> bool {
    match (owner, name) {
        (ObjRef::Slot(i), "Active") => {
            let on = matches!(v, Prop::Bool(true)) || matches!(v, Prop::Int(1)) || matches!(v, Prop::Num(x) if *x > 0.5);
            if let Some(s) = st.control.slots.get_mut(*i) {
                s.active = on;
            }
            st.control_requests.push(ControlRequest::Active(*i, on));
            true
        }
        (ObjRef::Workspace, "ModulesVisible") => {
            let on = matches!(v, Prop::Bool(true)) || matches!(v, Prop::Int(1));
            st.control.modules_visible = on;
            st.control_requests.push(ControlRequest::ModulesVisible(on));
            true
        }
        (ObjRef::Workspace, "PageGroup") => {
            let n = match v {
                Prop::Int(i) => *i,
                Prop::Num(x) => *x as i64,
                _ => return true,
            };
            let n = n.clamp(0, st.control.page_groups.saturating_sub(1) as i64) as usize;
            st.control.page_group = n;
            st.control_requests.push(ControlRequest::PageGroup(n));
            true
        }
        (ObjRef::ControlKnobs, pos) => {
            if let (Ok(pos), Prop::Num(x)) = (pos.parse::<usize>(), v)
                && let Some(Some(k)) = st.control.knobs.get(pos)
            {
                let inc = x - k.value;
                st.control_requests.push(ControlRequest::Turn { pos, inc, fine: false });
            }
            true
        }
        _ => false,
    }
}

/// The chain's slots changed since the last controller timer.
fn chain_changed(lua: &Lua) -> bool {
    let st = state(lua);
    let st = st.borrow();
    st.chain_seen.as_ref().is_none_or(|(_, slots)| *slots != st.control.slots)
}
