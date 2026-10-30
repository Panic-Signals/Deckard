//! `NHLController`: the hardware the scripts drive (LEDs, page stack, modes, display).

use super::{methods, new_object, object, state};
use crate::state::ObjRef;
use mlua::{Lua, MultiValue, Table, Value};

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let model = state(lua).borrow().controller.clone();
    let has = |names: &[&str]| names.contains(&model.as_str());
    let keyboards = ["KKS2_49", "KKS2_61", "KKS2_88", "KKA", "KKM"];
    // Controllers with RGB pads/buttons driven through a colour lookup table.
    let color_lut = has(&["M3", "MH1071", "MM3", "MJ1", "KKA", "KKM"]);
    let rgb = color_lut || has(&["MS1", "KKS2_49", "KKS2_61", "KKS2_88", "M2", "MM2"]);

    let c = methods(lua, impl_t, "NHLController")?;
    c.set("getControllerAcronym", lua.create_function(|lua, _: Value| Ok(state(lua).borrow().controller.clone()))?)?;
    c.set(
        "onTapTempo",
        lua.create_function(|lua, _: Value| {
            let st = state(lua);
            let mut st = st.borrow_mut();
            if let Some(bpm) = tap_tempo(&mut st.taps, std::time::Instant::now()) {
                super::data::set_live_param(&mut st, &ObjRef::Song, "Tempo", &crate::state::Prop::Num(bpm));
            }
            Ok(())
        })?,
    )?;
    c.set("hasColorLUT", lua.create_function(move |_, _: Value| Ok(color_lut))?)?;
    c.set("hasRGBLEDs", lua.create_function(move |_, _: Value| Ok(rgb))?)?;
    c.set("isMikro", lua.create_function(move |_, _: Value| Ok(model.starts_with("MM")))?)?;
    c.set(
        "setLEDLevel",
        lua.create_function(|lua, (_, id, level): (Value, i64, Option<f64>)| {
            state(lua).borrow_mut().hw.leds.entry(id).or_default().level = level.unwrap_or(0.0);
            Ok(())
        })?,
    )?;
    c.set(
        "setLEDColorLUT",
        lua.create_function(|lua, (_, id, color, led_state): (Value, i64, Value, Value)| {
            // Unimplemented API upstream can hand us a stub instead of a number.
            let int = |v: &Value, default| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).unwrap_or(default);
            state(lua).borrow_mut().hw.leds.entry(id).or_default().lut = Some((int(&color, -1), int(&led_state, 0)));
            Ok(())
        })?,
    )?;
    c.set(
        "setLEDColor",
        lua.create_function(|lua, (_, id, r, g, b): (Value, i64, f64, f64, f64)| {
            let c = |v: f64| v.clamp(0.0, 127.0) as u8;
            state(lua).borrow_mut().hw.leds.entry(id).or_default().rgb = Some((c(r), c(g), c(b)));
            Ok(())
        })?,
    )?;
    c.set(
        "resetAllLEDs",
        lua.create_function(|lua, _: Value| {
            state(lua).borrow_mut().hw.leds.clear();
            Ok(())
        })?,
    )?;
    c.set("updateLEDs", lua.create_function(|_, _: MultiValue| Ok(()))?)?;
    // The 4-D encoder pushed in (its turns are then fine).
    c.set(
        "getWheelPressed",
        lua.create_function(|lua, _: Value| {
            let wheel: Option<i64> = lua.load("return NI.HW.BUTTON_WHEEL").eval().ok().flatten();
            Ok(wheel.is_some_and(|w| state(lua).borrow().hw.held.contains(&w)))
        })?,
    )?;
    for (get, set, field) in
        [("getPadMode", "setPadMode", 0), ("getJogWheelMode", "setJogWheelMode", 1), ("getEncoderMode", "setEncoderMode", 2)]
    {
        c.set(
            get,
            lua.create_function(move |lua, _: Value| {
                let mode = {
                    let st = state(lua);
                    let hw = &st.borrow().hw;
                    [hw.pad_mode, hw.jog_wheel_mode, hw.encoder_mode][field]
                };
                // A page's default pad mode, as Maschine resolves it: the Step pages step
                // through the pattern, the Pattern page picks patterns; elsewhere the pads
                // play what the focus group does, its keyboard (Keyboard and Chords modes)
                // or its sounds.
                if field == 0 && mode == 0 {
                    return lua
                        .load(
                            "local top = NHLController:getPageStack():getTopPage()
                             if top == NI.HW.PAGE_STEP or top == NI.HW.PAGE_STEP_STUDIO or top == NI.HW.PAGE_STEP_MOD then
                                 return NI.HW.PAD_MODE_STEP
                             elseif top == NI.HW.PAGE_PATTERN then return NI.HW.PAD_MODE_PATTERN end
                             local g = NI.DATA.StateHelper.getFocusGroup(App)
                             if g and g:getKeyboardModeEnabledParameter():getValue() then return NI.HW.PAD_MODE_KEYBOARD end
                             return 0",
                        )
                        .eval();
                }
                Ok(mode)
            })?,
        )?;
        c.set(
            set,
            lua.create_function(move |lua, (_, v): (Value, i64)| {
                let st = state(lua);
                let hw = &mut st.borrow_mut().hw;
                *[&mut hw.pad_mode, &mut hw.jog_wheel_mode, &mut hw.encoder_mode][field] = v;
                Ok(())
            })?,
        )?;
    }
    c.set(
        "resetJogWheelMode",
        lua.create_function(|lua, _: Value| {
            let mode: Option<i64> = lua.load("return NI.HW.JOGWHEEL_MODE_DEFAULT").eval().ok().flatten();
            if let Some(m) = mode {
                state(lua).borrow_mut().hw.jog_wheel_mode = m;
            }
            Ok(())
        })?,
    )?;
    c.set("getContext", lua.create_function(|lua, _: Value| object(lua, ObjRef::Context))?)?;
    c.set("getScreenButtonMode", lua.create_function(|lua, _: Value| Ok(state(lua).borrow().hw.screen_button_mode))?)?;
    c.set(
        "setScreenButtonMode",
        lua.create_function(|lua, (_, mode): (Value, i64)| {
            state(lua).borrow_mut().hw.screen_button_mode = mode;
            Ok(())
        })?,
    )?;
    let context = methods(lua, impl_t, "Context")?;
    // The touch strip's mode (NI.HW.TS_MODE_*: PITCH, MOD, PERFORM, NOTES), set by the
    // buttons beside it; 0, none of them, shows the play position.
    context.set(
        "getTouchstripModeParameter",
        lua.create_function(|lua, _: Value| {
            let owner = state(lua).borrow_mut().objects.intern(ObjRef::Context);
            object(lua, ObjRef::Param { owner, name: "TouchstripMode".into() })
        })?,
    )?;
    // The mod wheel the strip last left a sound at (MOD's lights while untouched).
    context.set(
        "getLastTouchstripValueForSound",
        lua.create_function(|lua, (_, sound): (Value, Value)| {
            let Some(ObjRef::Sound(g, s)) = super::object_ref(lua, &sound) else { return Ok(0.0) };
            Ok(state(lua).borrow().strip_mod.get(&(g, s)).copied().unwrap_or(0.0))
        })?,
    )?;
    context.set("getSceneBank8", lua.create_function(|lua, _: Value| Ok(state(lua).borrow().hw.scene_bank8))?)?;
    context.set(
        "setSceneBank8",
        lua.create_function(|lua, (_, bank): (Value, i64)| {
            state(lua).borrow_mut().hw.scene_bank8 = bank.max(0);
            Ok(())
        })?,
    )?;
    c.set("getPageStack", lua.create_function(|lua, _: Value| object(lua, ObjRef::PageStack))?)?;
    c.set("getTopPage", lua.create_function(|lua, _: Value| Ok(state(lua).borrow().hw.page_stack.last().copied()))?)?;
    c.set("getHardwareDisplay", lua.create_function(|lua, _: Value| object(lua, ObjRef::HardwareDisplay))?)?;
    page_stack(lua, impl_t)?;

    // The display hands out root widgets: getOverlayRoot(), getPageStack(), ...
    let d = methods(lua, impl_t, "HardwareDisplay")?;
    let meta = lua.create_table()?;
    meta.set(
        "__index",
        lua.create_function(|lua, (_, key): (Table, String)| {
            let Some(root) = key.strip_prefix("get").map(str::to_string) else { return Ok(Value::Nil) };
            let f = lua.create_function(move |lua, _: MultiValue| {
                let st = state(lua);
                let existing = st.borrow().hw.display_roots.get(&root).copied();
                let id = existing.unwrap_or_else(|| {
                    let mut st = st.borrow_mut();
                    let id = st.widgets.insert(None, &root, &root);
                    st.hw.display_roots.insert(root.clone(), id);
                    // The overlay spans both displays.
                    if root == "OverlayRoot" {
                        st.widgets.set(id, "Width", crate::state::Prop::Int(960));
                        st.widgets.set(id, "Height", crate::state::Prop::Int(272));
                    }
                    id
                });
                new_object(lua, "Widget", id)
            })?;
            Ok(Value::Function(f))
        })?,
    )?;
    d.set_metatable(Some(meta))?;

    // Feature flags. Best guesses from the hardware; not yet confirmed against the app.
    let model = state(lua).borrow().controller.clone();
    let has = |names: &[&str]| names.contains(&model.as_str());
    let hw = methods(lua, impl_t, "NI.HW.FEATURE")?;
    hw.set("MK3", has(&["M3", "MH1071"]))?;
    hw.set("PADS", !has(&keyboards))?;
    hw.set("KEYBOARD", has(&keyboards))?;
    hw.set("JOGWHEEL", has(&["MS1", "M3", "MH1071"]))?;
    hw.set("JOYCODER", has(&["MM3", "KKS2_49", "KKS2_61", "KKS2_88", "KKA", "KKM"]))?;
    hw.set("CHANNEL", has(&["M3", "MH1071", "MS1"]))?;
    hw.set("SCREEN_TYPE_STUDIO", has(&["MS1", "M3", "MH1071"]))?;
    hw.set("SCREEN_TYPE_CLASSIC", has(&["M1", "M2"]))?;
    hw.set("SCREEN_TYPE_MIKRO", has(&["MM1", "MM2"]))?;
    let app = methods(lua, impl_t, "NI.APP.FEATURE")?;
    for f in ["ACCESSIBILITY", "ARRANGER", "EFFECTS_CHAIN", "MODULATION", "SAMPLER", "UNDO_REDO"] {
        app.set(f, true)?;
    }
    for f in ["DEV_TOOLS", "NTK_SUPPORT"] {
        app.set(f, false)?;
    }
    Ok(())
}

/// The stack of hardware pages (bottom first); the top page is the one shown.
fn page_stack(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let p = methods(lua, impl_t, "PageStack")?;
    fn edit(lua: &Lua, f: impl FnOnce(&mut Vec<i64>)) {
        let st = state(lua);
        let mut st = st.borrow_mut();
        let before = st.hw.page_stack.clone();
        f(&mut st.hw.page_stack);
        if st.hw.page_stack != before {
            st.hw.page_stack_changed = true;
        }
    }
    fn read<R>(lua: &Lua, f: impl FnOnce(&[i64]) -> R) -> R {
        f(&state(lua).borrow().hw.page_stack)
    }
    p.set("getTopPage", lua.create_function(|lua, _: Value| Ok(read(lua, |s| s.last().copied())))?)?;
    p.set("getNumPages", lua.create_function(|lua, _: Value| Ok(read(lua, |s| s.len())))?)?;
    p.set("getPageAt", lua.create_function(|lua, (_, i): (Value, usize)| Ok(read(lua, |s| s.get(i).copied())))?)?;
    p.set("isPageInStack", lua.create_function(|lua, (_, id): (Value, i64)| Ok(read(lua, |s| s.contains(&id))))?)?;
    p.set("isTopPage", lua.create_function(|lua, (_, id): (Value, i64)| Ok(read(lua, |s| s.last() == Some(&id))))?)?;
    p.set("isPageStackChanged", lua.create_function(|lua, _: Value| Ok(state(lua).borrow().hw.page_stack_changed))?)?;
    p.set(
        "pushPage",
        lua.create_function(|lua, (_, id): (Value, i64)| {
            edit(lua, |s| {
                s.retain(|p| *p != id);
                s.push(id)
            });
            super::macros::page_pushed(lua, id);
            Ok(())
        })?,
    )?;
    p.set(
        "popPage",
        lua.create_function(|lua, _: Value| {
            edit(lua, |s| {
                if s.len() > 1 {
                    s.pop();
                }
            });
            Ok(())
        })?,
    )?;
    p.set(
        "popToBottomPage",
        lua.create_function(|lua, _: Value| {
            edit(lua, |s| s.truncate(1));
            Ok(())
        })?,
    )?;
    p.set(
        "removePage",
        lua.create_function(|lua, (_, id): (Value, i64)| {
            edit(lua, |s| s.retain(|p| *p != id));
            Ok(())
        })?,
    )?;
    p.set(
        "replacePage",
        lua.create_function(|lua, (_, old, new): (Value, i64, i64)| {
            edit(lua, |s| {
                if let Some(p) = s.iter_mut().find(|p| **p == old) {
                    *p = new;
                }
            });
            Ok(())
        })?,
    )?;
    p.set(
        "replaceBottomPage",
        lua.create_function(|lua, (_, id): (Value, i64)| {
            edit(lua, |s| match s.first_mut() {
                Some(p) => *p = id,
                None => s.push(id),
            });
            Ok(())
        })?,
    )?;
    p.set(
        "setTop",
        lua.create_function(|lua, (_, id): (Value, i64)| {
            edit(lua, |s| {
                s.retain(|p| *p != id);
                s.push(id)
            });
            Ok(())
        })?,
    )?;
    p.set(
        "getPagePinState",
        lua.create_function(|lua, (_, id): (Value, i64)| Ok(state(lua).borrow().hw.pinned.get(&id).copied().unwrap_or(false)))?,
    )?;
    p.set(
        "setPagePinState",
        lua.create_function(|lua, (_, id, pinned): (Value, i64, bool)| {
            state(lua).borrow_mut().hw.pinned.insert(id, pinned);
            Ok(())
        })?,
    )?;
    Ok(())
}

/// Taps further apart than this start counting again.
const TAP_GAP: std::time::Duration = std::time::Duration::from_secs(2);
/// The taps averaged.
const TAPS: usize = 5;

/// TAP pressed at `now`: the tempo of the last few taps (from the second tap on),
/// to a tenth of a BPM.
pub(crate) fn tap_tempo(taps: &mut Vec<std::time::Instant>, now: std::time::Instant) -> Option<f64> {
    if taps.last().is_some_and(|&t| now.duration_since(t) > TAP_GAP) {
        taps.clear();
    }
    taps.push(now);
    if taps.len() > TAPS {
        taps.remove(0);
    }
    let (first, last) = (*taps.first()?, *taps.last()?);
    let beat = last.duration_since(first).as_secs_f64() / (taps.len() - 1) as f64;
    (taps.len() > 1 && beat > 0.0).then(|| (600.0 / beat).round() / 10.0)
}

#[cfg(test)]
mod tests {
    use super::tap_tempo;
    use std::time::{Duration, Instant};

    /// Taps half a second apart make 120 BPM; a long pause starts over.
    #[test]
    fn taps_set_the_tempo() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut taps = Vec::new();
        assert_eq!(tap_tempo(&mut taps, at(0)), None);
        assert_eq!(tap_tempo(&mut taps, at(500)), Some(120.0));
        assert_eq!(tap_tempo(&mut taps, at(1000)), Some(120.0));
        assert_eq!(tap_tempo(&mut taps, at(1490)), Some(120.8));
        assert_eq!(tap_tempo(&mut taps, at(9000)), None, "a pause starts over");
        assert_eq!(tap_tempo(&mut taps, at(9600)), Some(100.0));
    }
}
