//! Lua host for Maschine's hardware-controller scripts.
//!
//! The scripts are NI's, loaded at run time from the user's own Maschine install (see
//! `ni-res-extract`) plus an optional mod folder that overrides them file by file; none
//! are bundled. The `NI.*`, `App` and `NHLController` APIs they call are implemented in
//! Rust ([`api`]). Anything not yet implemented is a recorded stub (see `prelude.lua`),
//! so every script loads and [`Host::report`] lists the API still to build, most-used
//! first.

pub mod api;
mod arranger_view;
mod mixer_view;
mod pattern_view;
mod record_meter;
mod sample_view;
pub mod screen;
pub mod scripts;
pub mod state;

use mlua::{Function, Lua, LuaOptions, MultiValue, StdLib, Table, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub use scripts::Scripts;
pub use state::State;

/// How often each stubbed API path was touched.
/// How long one script call may run before it's stopped.
pub const SCRIPT_TIME_LIMIT: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Debug, Default)]
pub struct Recorder {
    pub hits: HashMap<String, (&'static str, u64)>,
}

pub struct Host {
    pub lua: Lua,
    pub state: Rc<RefCell<State>>,
    pub recorder: Rc<RefCell<Recorder>>,
    /// Script errors raised by callbacks.
    pub errors: Vec<String>,
    /// When each knob (1–8) was last touched, for double taps.
    knob_taps: [Option<std::time::Instant>; 8],
}

/// Two touches of a knob this close together are a double tap.
const DOUBLE_TAP: std::time::Duration = std::time::Duration::from_millis(400);

impl Host {
    /// A Lua state with the prelude and API installed, and `require` resolving through
    /// `scripts`. Only the safe standard libraries are opened: scripts and mods get no
    /// file or process access.
    pub fn new(scripts: Scripts, state: State) -> mlua::Result<Host> {
        let libs = StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH | StdLib::PACKAGE;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        let state = Rc::new(RefCell::new(state));
        let recorder = Rc::new(RefCell::new(Recorder::default()));
        lua.set_app_data(state.clone());

        let g = lua.globals();
        let start = std::time::Instant::now();
        g.set("__host_clock", lua.create_function(move |_, ()| Ok(start.elapsed().as_secs_f64()))?)?;
        // The parts of `os` and `debug` the scripts use, without file or process access.
        lua.load(
            r#"
            os = { clock = __host_clock, time = function() return math.floor(__host_clock()) end }
            debug = { traceback = function(m) return m end, getinfo = function() return {} end,
                      sethook = function() end }
            "#,
        )
        .exec()?;

        let host_t = lua.create_table()?;
        let rec = recorder.clone();
        host_t.set(
            "record",
            lua.create_function(move |_, (path, kind): (String, String)| {
                let kind: &'static str = match kind.as_str() {
                    "call" => "call",
                    "const" => "const",
                    "field" => "field",
                    _ => "index",
                };
                rec.borrow_mut().hits.entry(path).or_insert((kind, 0)).1 += 1;
                Ok(())
            })?,
        )?;
        let impl_t = lua.create_table()?;
        host_t.set("impl", impl_t.clone())?;
        host_t.set("namespaces", lua.create_table_from(scripts.namespaces().into_iter().map(|n| (n, true)))?)?;
        host_t.set("known", known_constants(&lua, &scripts, &state.borrow().controller)?)?;
        lua.load(include_str!("prelude.lua")).set_name("@prelude.lua").call::<()>(host_t.clone())?;
        g.set("__host", host_t)?;

        install_require(&lua, scripts)?;
        api::install(&lua, &impl_t)?;
        g.set(
            "print",
            lua.create_function(|lua, args: MultiValue| {
                let parts: Vec<String> = args.iter().map(|v| v.to_string().unwrap_or_default()).collect();
                if let Some(state) = lua.app_data_ref::<Rc<RefCell<State>>>() {
                    state.borrow_mut().log.push(parts.join("\t"));
                }
                Ok(())
            })?,
        )?;
        Ok(Host { lua, state, recorder, errors: Vec::new(), knob_taps: [None; 8] })
    }

    /// Load `Scripts/Main.lua` and the controller scripts for the state's controller
    /// model, the way the app does at start-up.
    pub fn boot(&mut self) -> mlua::Result<()> {
        let require: Function = self.lua.globals().get("require")?;
        require.call::<()>("Scripts/Main")?;
        // The app starts controllers on their main page: Control, or Main on Jam.
        let known: Table = self.lua.globals().get::<Table>("__host")?.get::<Table>("known")?.get("hw")?;
        let first = ["PAGE_CONTROL", "PAGE_MAIN"].into_iter().find_map(|p| known.get::<Option<i64>>(p).ok().flatten());
        self.state.borrow_mut().hw.page_stack = first.into_iter().collect();
        // The 4-D encoder starts in its default mode (the page's own use of it).
        if let Some(mode) = known.get::<Option<i64>>("JOGWHEEL_MODE_DEFAULT").ok().flatten() {
            self.state.borrow_mut().hw.jog_wheel_mode = mode;
        }
        let load: Function = self.lua.globals().get("loadControllerScripts")?;
        load.call::<()>("Maschine")?;
        self.lua.load(api::plugins::OVERRIDES).set_name("deckard-overrides").exec()?;
        let csi: Value = self.lua.globals().get("ControllerScriptInterface")?;
        if csi.is_nil() {
            return Err(mlua::Error::runtime(format!("no controller script for model {:?}", self.state.borrow().controller)));
        }
        Ok(())
    }

    /// Call `ControllerScriptInterface:<method>(args...)`, as the app does for hardware
    /// events and timers. Script errors are collected in `errors`, not returned.
    pub fn event(&mut self, method: &str, args: impl mlua::IntoLuaMulti) -> bool {
        // (A page the timer pushes or pops is still to be shown: Step mode's hold.)
        let stack_before = (method == "onControllerTimer").then(|| self.state.borrow().hw.page_stack.clone());
        // A watchdog: a script call that runs away (a loop that never ends) is stopped
        // with where it was, rather than freezing the app.
        let deadline = std::time::Instant::now() + SCRIPT_TIME_LIMIT;
        self.lua
            .set_hook(mlua::HookTriggers::new().every_nth_instruction(100_000), move |lua, _| {
                if std::time::Instant::now() < deadline {
                    return Ok(mlua::VmState::Continue);
                }
                let mut at = Vec::new();
                for level in 0..12 {
                    let Some(frame) = lua.inspect_stack(level, |d| {
                        let src = d.source();
                        format!("{}:{}", src.short_src.as_deref().unwrap_or("?"), d.current_line().map_or(-1, |l| l as i64))
                    }) else {
                        break;
                    };
                    at.push(frame);
                }
                Err(mlua::Error::runtime(format!("script ran over {SCRIPT_TIME_LIMIT:?}, stopped at {}", at.join(" < "))))
            })
            .ok();
        let r = (|| -> mlua::Result<()> {
            let csi: Table = self.lua.globals().get("ControllerScriptInterface")?;
            let f: Option<Function> = csi.get(method)?;
            let Some(f) = f else { return Err(mlua::Error::runtime(format!("controller has no {method}"))) };
            let mut a = args.into_lua_multi(&self.lua)?;
            a.push_front(Value::Table(csi));
            f.call::<()>(a)
        })();
        self.lua.remove_hook();
        if let Some(before) = stack_before {
            let mut st = self.state.borrow_mut();
            st.hw.page_stack_changed = st.hw.page_stack != before;
            st.changed_params.clear();
            st.level_seen = Some(st.control.level);
            st.arranger_seen = Some((st.ideas_view, st.focus.scene, st.focus.section));
            st.focus_seen = Some(st.focus);
            st.chain_seen = Some((st.control.focus_slot, st.control.slots.clone()));
        }
        match r {
            Ok(()) => true,
            Err(e) => {
                let e = format!("{method}: {e}");
                if e.contains("script ran over") {
                    eprintln!("{e}");
                }
                self.errors.push(e);
                false
            }
        }
    }

    /// One app update: `onCustomProcess` (page sync and screens), then the controller
    /// timer (LEDs), skipping either if the controller script doesn't define it.
    pub fn tick(&mut self) {
        // LEDHelper keeps an LED flashing for two timers after its note ends; with
        // Maschine's faster timer that's unseen, with ours it lags. Flashes end with
        // their notes instead.
        let _ =
            self.lua.load("local c = LEDHelper and LEDHelper.FlashStateCountdown; if c then for k in pairs(c) do c[k] = 0 end end").exec();
        // The Song view always has a section in focus while there are any (the app may
        // have switched to it, or deleted the focused one).
        {
            let mut st = self.state.borrow_mut();
            if !st.ideas_view {
                api::arranger::focus_a_section(&mut st);
            }
            flag_color_changes(&mut st);
        }
        for (method, args) in [("onCustomProcess", Some(false)), ("onControllerTimer", None)] {
            let has = self
                .lua
                .globals()
                .get::<Table>("ControllerScriptInterface")
                .and_then(|c| c.get::<Option<Function>>(method))
                .is_ok_and(|f| f.is_some());
            if has {
                self.event(method, args);
            }
        }
        // A new plug-in list has been seen by the scripts (they rebuild their browser).
        self.state.borrow_mut().plugins_changed = false;
        // Lists (connectVector) and the Ideas view show what the scripts hold now.
        let screen = self.active_screen();
        for realize in ["__realize_vectors", "__realize_idea_spaces"] {
            if let Ok(f) = self.lua.globals().get::<Function>(realize)
                && let Err(e) = f.call::<()>(screen)
            {
                self.errors.push(e.to_string());
            }
        }
    }

    /// Give the controller's plug-in browser the plug-ins the app can load; the scripts
    /// rebuild their lists on the next tick if it changed.
    /// Link and MIDI sync as the app has them (Settings ▸ MIDI): what the scripts
    /// change there shows in `state.sync`.
    pub fn set_sync(&mut self, view: state::SyncView) {
        let mut st = self.state.borrow_mut();
        st.sync = view;
        api::sync::refresh(&mut st);
    }

    /// The meters, for the Mixer page (see [`state::Levels`]).
    pub fn set_levels(&mut self, levels: state::Levels) {
        self.state.borrow_mut().levels = levels;
    }

    /// The browser as the app has it (BROWSE); the scripts are told what changed.
    pub fn set_browser(&mut self, view: state::BrowserView) {
        let old = std::mem::replace(&mut self.state.borrow_mut().browser, view.clone());
        if old == view {
            return;
        }
        let mut models = Vec::new();
        if old.products != view.products {
            models.push(api::browser::MODEL_PRODUCTS);
        }
        models.extend([api::browser::MODEL_BANKCHAIN, api::browser::MODEL_ATTRIBUTES, api::browser::MODEL_RESULTS]);
        let has = self
            .lua
            .globals()
            .get::<Table>("ControllerScriptInterface")
            .and_then(|c| c.get::<Option<Function>>("onDB3ModelChanged"))
            .is_ok_and(|f| f.is_some());
        if has {
            for m in models {
                self.event("onDB3ModelChanged", m);
            }
        }
    }

    pub fn set_plugins(&mut self, plugins: Vec<state::PluginEntry>) {
        let mut st = self.state.borrow_mut();
        if st.plugins != plugins {
            st.plugins = plugins;
            st.plugins_changed = true;
        }
    }

    /// The style sheet the displays are drawn with; widgets' style properties
    /// (`getIntProperty`, `getStringWidth`) come from it.
    pub fn set_look(&mut self, look: Rc<deckard_nwl::draw::Look>) {
        self.lua.set_app_data(look);
    }

    /// What the displays show: the active page's screen and, over it, the overlay
    /// (touched-knob lists …) when it shows anything.
    pub fn display(&self) -> Option<(deckard_nwl::draw::Element, Option<deckard_nwl::draw::Element>)> {
        let view = self.pattern_view();
        let st = self.state.borrow();
        let page = screen::element(&st, self.active_screen()?, view)?;
        let overlay = st
            .hw
            .display_roots
            .get("OverlayRoot")
            .and_then(|&id| screen::element(&st, id, view))
            .filter(|e| e.children.iter().any(|c| !c.hidden));
        Some((page, overlay))
    }

    /// The pads play the focused sound's pitches (Keyboard mode) rather than the
    /// group's sounds (Pad mode).
    pub fn keyboard_mode(&self) -> bool {
        self.pattern_view().keyboard
    }

    /// What the pattern editors show: the focused sound's steps in Step mode, its
    /// pitches in Keyboard mode (with the group's root, for the scale overlay).
    fn pattern_view(&self) -> pattern_view::View {
        let (step, keyboard, root): (bool, bool, i64) = self
            .lua
            .load(
                "local g = NI.DATA.StateHelper.getFocusGroup(App)
                 return NHLController:getPadMode() == NI.HW.PAD_MODE_STEP,
                        PadModeHelper and PadModeHelper.getKeyboardMode() or false,
                        g and g:getRootNoteParameter():getValue() or 60",
            )
            .eval()
            .unwrap_or((false, false, 60));
        pattern_view::View { step, keyboard, probability: false, root }
    }

    /// The integer the host uses for a constant such as `NI.HW.BUTTON_PLAY`.
    pub fn constant(&self, path: &str) -> mlua::Result<i64> {
        let mut v: Value = Value::Table(self.lua.globals());
        for part in path.split('.') {
            v = match v {
                Value::Table(t) => t.get(part)?,
                _ => return Err(mlua::Error::runtime(format!("{path}: not a namespace"))),
            };
        }
        match v {
            Value::Integer(i) => Ok(i),
            Value::Number(n) => Ok(n as i64),
            other => Err(mlua::Error::runtime(format!("{path} is {}", other.type_name()))),
        }
    }

    /// The constant's name for a host-assigned value, e.g. `NI.HW.LED_TRANSPORT_PLAY`.
    pub fn constant_name(&self, prefix: &str, value: i64) -> Option<String> {
        let host: Table = self.lua.globals().get("__host").ok()?;
        let constants: Table = host.get("constants").ok()?;
        constants.pairs::<String, i64>().filter_map(Result::ok).find(|(k, v)| *v == value && k.starts_with(prefix)).map(|(k, _)| k)
    }

    /// Press and release a hardware button (`BUTTON_TRANSPORT_PLAY`, ...).
    pub fn press(&mut self, button: &str) -> mlua::Result<()> {
        self.button(button, true)?;
        self.button(button, false)
    }

    /// Press or release a button by name. Modifier pages (SCENE, PATTERN, ...) show
    /// only while their button is held.
    pub fn button(&mut self, button: &str, pressed: bool) -> mlua::Result<()> {
        // (The MK3's AUTO is Maschine's AUTO_WRITE; its BUTTON_AUTO is the Jam's.)
        let button = if button == "BUTTON_AUTO" { "BUTTON_AUTO_WRITE" } else { button };
        let id = self.constant(&format!("NI.HW.{button}"))?;
        {
            let held = &mut self.state.borrow_mut().hw.held;
            if pressed {
                held.insert(id);
            } else {
                held.remove(&id);
            }
        }
        if pressed {
            self.native_screen_button(id);
        }
        self.event("onSwitchEvent", (id, pressed));
        if pressed && let Some(n) = button.strip_prefix("BUTTON_CAP_").and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=8).contains(n))
        {
            let now = std::time::Instant::now();
            let double = self.knob_taps[n - 1].is_some_and(|t| now.duration_since(t) < DOUBLE_TAP);
            self.knob_taps[n - 1] = if double { None } else { Some(now) };
            if double {
                self.knob_double_tap(n);
            }
        }
        Ok(())
    }

    /// A knob tapped twice: on the Step and Events pages, VELOCITY sets the selected
    /// notes' velocity to 100 and PROBABILITY their probability to 100 %.
    fn knob_double_tap(&mut self, knob: usize) {
        // The Mixer: the strip under the knob back to 0 dB, or its pan to the middle
        // (whichever the knobs turn).
        let mixer = format!(
            "local p = ControllerScriptInterface and ControllerScriptInterface.ActivePage \
             if not (p and p.getLevelParameter and p.getFocusVector and p.EncoderMode) then return false end \
             local param = p:getLevelParameter({knob} + p:getFocusVector():getOffset()) \
             if not param then return true end \
             local pan = p.EncoderMode ~= MixerVector.MODE_VOLUME \
             NI.DATA.ParameterAccess.setFloatParameter(App, param, pan and 0.0 or {zero}) \
             return true",
            zero = deckard_model::level_from_db(0.0)
        );
        if self.lua.load(mixer).eval::<bool>().unwrap_or(false) {
            self.tick();
            return;
        }
        let name: Option<String> = self
            .lua
            .load(format!(
                "local c = ControllerScriptInterface; local p = c and c.ActivePage \
                 while p and not p.Screen and p.CurrentPage and p.CurrentPage ~= p do p = p.CurrentPage end \
                 local w = p and p.Screen and p.Screen.ParameterWidgets and p.Screen.ParameterWidgets[{knob}] \
                 return w and w.Name and w.Name:getText() or nil"
            ))
            .eval()
            .ok()
            .flatten();
        let reset = match name.as_deref() {
            Some("VELOCITY") => api::events::Reset::Velocity,
            Some("PROBABILITY") => api::events::Reset::Probability,
            _ => return,
        };
        api::events::reset_selected(&self.lua, reset);
        self.tick();
    }

    /// In the Ideas view (`SCREENBUTTON_MODE_SCENES`) the buttons above the displays
    /// pick scene 1–8 of the bank, launching it as SCENE + pad does; the one after the
    /// last scene ("+") makes a new one.
    fn native_screen_button(&mut self, id: i64) {
        let Ok(first) = self.constant("NI.HW.BUTTON_SCREEN_1") else { return };
        let Some(k) = (0..8).find(|k| id == first + k) else { return };
        let scenes_mode = self.constant("NI.HW.SCREENBUTTON_MODE_SCENES").ok();
        let (mode, bank) = {
            let st = self.state.borrow();
            (st.hw.screen_button_mode, st.hw.scene_bank8)
        };
        if Some(mode) != scenes_mode {
            return;
        }
        let i = (bank * 8 + k) as usize;
        let n = self.state.borrow().project().map_or(0, |p| p.scenes.len());
        if i < n {
            let mut st = self.state.borrow_mut();
            st.focus.scene = Some(i);
            st.launched_scene = Some(i);
        } else if i == n {
            let _ = self.lua.load("NI.DATA.IdeaSpaceAccess.createScene(App, NI.DATA.StateHelper.getFocusSong(App))").exec();
        }
    }

    /// Hit pad `index` (1-based, as the scripts count) with `value` (0..1), then release it.
    pub fn pad(&mut self, index: i64, value: f64) {
        // The app acts on the hardware event before the scripts see it (in section
        // mode the scripts switch pads off while one is held).
        self.native_pad(index);
        self.hold_pad(index, true, false);
        self.event("onPadEvent", (index, true, value));
        self.event("onPadEvent", (index, false, 0.0));
        // Pressed and released at once: it stays lit a moment, so the tap shows.
        self.hold_pad(index, false, true);
    }

    /// A pad pressed (`trigger`, with `value` 0..1) or released, as the hardware sends
    /// it (1-based index).
    pub fn pad_event(&mut self, index: i64, trigger: bool, value: f64) {
        if trigger {
            self.native_pad(index);
            // (Chords mode's Auto moves on as the pad's chord played.)
            let mut st = self.state.borrow_mut();
            let (auto, after) = &mut st.chord_auto;
            if let Some(next) = usize::try_from(index - 1).ok().and_then(|i| after.get_mut(i)).and_then(Option::take) {
                *auto = next;
            }
        }
        self.hold_pad(index, trigger, false);
        self.event("onPadEvent", (index, trigger, value));
    }

    /// Note what pad `index` (1-based) plays while it's down, so it lights (see
    /// [`state::NotesOn`]): in Keyboard mode the focus sound at the pad's note, otherwise
    /// the pad's sound. Released, it goes dark at once, or after
    /// [`state::NotesOn::AFTER_RELEASE`] if it `lingers`.
    fn hold_pad(&mut self, index: i64, down: bool, lingers: bool) {
        let Some(i) = index.checked_sub(1).and_then(|i| usize::try_from(i).ok()).filter(|&i| i < 16) else { return };
        if !down {
            let pad = &mut self.state.borrow_mut().notes_on.pads[i];
            match pad {
                Some((_, released)) if lingers => *released = Some(std::time::Instant::now()),
                _ => *pad = None,
            }
            return;
        }
        let note = if !self.pads_play_sounds() {
            None
        } else if let Some((sound, notes)) = self.keyboard_pads() {
            (notes[i] < 128).then(|| (self.state.borrow().focus.group, sound, notes[i]))
        } else {
            Some((self.state.borrow().focus.group, i, 60))
        };
        self.state.borrow_mut().notes_on.pads[i] = note.map(|n| (n, None));
    }

    /// The notes the sequencer plays now, (group, sound, pitch), for the LEDs.
    pub fn set_notes_played(&mut self, played: std::collections::HashSet<(usize, usize, i64)>) {
        self.state.borrow_mut().notes_on.played = played;
    }

    /// The scale the pads play in Keyboard and Chords modes: (root note, MIDI 60 = C3;
    /// bank; scale in it). The root is the focused group's, the scale the scale engine's.
    pub fn scale(&self) -> (i64, i64, i64) {
        let root: i64 = self
            .lua
            .load("local g = NI.DATA.StateHelper.getFocusGroup(App); return g and g:getRootNoteParameter():getValue() or 60")
            .eval()
            .unwrap_or(60);
        let s = self.state.borrow().scale;
        (root, s.bank, s.scale)
    }

    /// Set that scale (as the Keyboard page's knobs do).
    pub fn set_scale(&mut self, root: i64, bank: i64, scale: i64) {
        {
            let mut st = self.state.borrow_mut();
            st.scale.bank = bank.clamp(0, 7);
            st.scale.scale = scale.clamp(0, 14);
        }
        let set = format!(
            "local g = NI.DATA.StateHelper.getFocusGroup(App); if g then g:getRootNoteParameter():setValue({}) end",
            root.clamp(0, 127)
        );
        if let Err(e) = self.lua.load(set).exec() {
            self.errors.push(e.to_string());
        }
    }

    /// Whether knob `n` (1..=8) is touched (the MK3's knobs sense a finger).
    pub fn knob_touched(&self, n: usize) -> bool {
        self.constant(&format!("NI.HW.BUTTON_CAP_{n}")).is_ok_and(|id| self.state.borrow().hw.held.contains(&id))
    }

    /// The velocity every pad plays with, in Fixed Velocity (`NI.HW.PAD_VELOCITY_FIXED`,
    /// the FIXED VEL button); `None` while pads play as hit.
    pub fn fixed_velocity(&self) -> Option<i64> {
        use deckard_doc::Target;
        let doc = self.state.borrow().doc.clone()?;
        let d = doc.borrow();
        // NI.HW.PAD_VELOCITY_FIXED is 1; the project keeps the mode and the velocity.
        (d.num(Target::Song, "pad_velocity_mode")? as i64 == 1)
            .then(|| d.num(Target::Song, "fixed_velocity").map_or(100, |v| v as i64).clamp(0, 127))
    }

    /// The song's Fixed Velocity (0–127, Maschine's default 100), whatever the pads'
    /// velocity mode: what the pads play at in the Pad Touch pressure modes.
    pub fn fixed_velocity_value(&self) -> i64 {
        use deckard_doc::Target;
        let Some(doc) = self.state.borrow().doc.clone() else { return 100 };
        let d = doc.borrow();
        d.num(Target::Song, "fixed_velocity").map_or(100, |v| v as i64).clamp(0, 127)
    }

    /// Where the app's playhead is (ticks into what plays) and what plays, for the
    /// touch strip's play position; stopped, where STOP left it (`None`: nothing yet).
    pub fn set_playhead(&mut self, position: i64, what: Option<state::Playing>) {
        let mut st = self.state.borrow_mut();
        st.transport.what = what;
        if what.is_some() {
            st.transport.position = position;
        }
    }

    /// Each group's progress through its pattern in the focused scene, (bars, how far
    /// through them in bars), for the Ideas view; groups without a pattern there are left
    /// out.
    pub fn set_group_progress(&mut self, progress: std::collections::HashMap<usize, (usize, f64)>) {
        self.state.borrow_mut().group_progress = progress;
    }

    /// An encoder turned (`ENCODER_SCREEN_1`… or `ENCODER_VOLUME`…) by `inc` (a fraction
    /// of a turn).
    pub fn encoder(&mut self, encoder: &str, inc: f64) -> mlua::Result<()> {
        let id = self.constant(&format!("NI.HW.{encoder}"))?;
        // In Control mode the app itself turns the shown page's parameters (the scripts
        // leave the screen encoders alone then); SHIFT turns them finely.
        let control = self.constant("NI.HW.ENC_MODE_CONTROL").ok();
        if let Some(n) = encoder.strip_prefix("ENCODER_SCREEN_").and_then(|n| n.parse::<usize>().ok())
            && control.is_some()
            && self.state.borrow().hw.encoder_mode == control.unwrap()
        {
            let held = |name: &str| self.constant(name).ok().is_some_and(|b| self.state.borrow().hw.held.contains(&b));
            let (fine, erase) = (held("NI.HW.BUTTON_SHIFT"), held("NI.HW.BUTTON_ERASE"));
            let r = if erase {
                crate::state::ControlRequest::Erase { pos: n - 1 }
            } else {
                crate::state::ControlRequest::Turn { pos: n - 1, inc, fine }
            };
            self.state.borrow_mut().control_requests.push(r);
        }
        self.event("onEncoderEvent", (id, inc));
        Ok(())
    }

    /// Run a chunk of Lua in the scripts' world (tests, `lua-host-run --eval`); an error
    /// goes in `errors`.
    pub fn eval_lua(&mut self, code: &str) {
        if let Err(e) = self.lua.load(code).exec() {
            self.errors.push(e.to_string());
        }
    }

    /// The focused group's root note (the scale's key).
    pub fn root_note(&self) -> i64 {
        api::scales::root_note(&self.lua)
    }

    /// A global the scripts set (tests: values read back after [`Host::eval_lua`]).
    pub fn lua_global<T: mlua::FromLua>(&self, name: &str) -> T {
        self.lua.globals().get(name).expect("a global of that type")
    }

    /// The jog wheel (4-D encoder) turned by `steps` clicks.
    pub fn wheel(&mut self, steps: i64) {
        self.event("onWheelEvent", (0, steps));
    }

    /// The touch strip: finger `id` (0-based), `kind` 0 touched, 1 moved, 2 released,
    /// at `value` (0..1).
    pub fn touch(&mut self, id: i64, kind: i64, value: f64) {
        // In PERFORM mode the strip plays the focused group's Perform FX (natively, in
        // Maschine): the app engages and moves it.
        let mode: Option<i64> = self.lua.load("return NHLController:getContext():getTouchstripModeParameter():getValue()").eval().ok();
        // PITCH bends the focused sound, back to the middle once let go; MOD is its mod
        // wheel, kept where it's let go (natively too).
        let (pitch, modulate) = (self.constant("NI.HW.TS_MODE_PITCH").ok(), self.constant("NI.HW.TS_MODE_MOD").ok());
        if id == 0 && mode.is_some() && (mode == pitch || mode == modulate) {
            let mut st = self.state.borrow_mut();
            let (group, sound) = (st.focus.group, st.focus.sound);
            let value = value.clamp(0.0, 1.0);
            if mode == pitch {
                let bend = if kind == 2 { 0.0 } else { value * 2.0 - 1.0 };
                st.control_requests.push(state::ControlRequest::StripControl { group, sound, pitch: true, value: bend });
            } else if kind != 2 {
                st.strip_mod.insert((group, sound), value);
                st.control_requests.push(state::ControlRequest::StripControl { group, sound, pitch: false, value });
            }
        }
        if mode.is_some() && mode == self.constant("NI.HW.TS_MODE_NOTES").ok() {
            self.strum(id, kind, value);
        }
        if id == 0 && mode.is_some() && mode == self.constant("NI.HW.TS_MODE_PERFORM").ok() {
            let mut st = self.state.borrow_mut();
            let group = st.focus.group;
            if api::perform::has(&st, group) {
                let touch = match kind {
                    0 => Some(true),
                    2 => Some(false),
                    _ => None,
                };
                st.control_requests.push(state::ControlRequest::PerformStrip { group, touch, pos: value.clamp(0.0, 1.0) });
            }
        }
        self.event("onTouchEvent", (id, kind, value));
    }

    /// NOTES mode (native in Maschine): finger `finger` strums the pads held, else all
    /// 16, along the strip (`pos` 0..1), holding the note of the pad under it till it
    /// moves on to another or lifts (`kind` 2).
    fn strum(&mut self, finger: i64, kind: i64, pos: f64) {
        let Some(f) = usize::try_from(finger).ok().filter(|&f| f < 2) else { return };
        let stop = |st: &mut state::State| {
            if let Some((_, group, sound, pitches, velocity)) = st.strummed[f].take() {
                for pitch in pitches {
                    st.control_requests.push(state::ControlRequest::StripNote { group, sound, pitch, velocity, on: false });
                }
            }
        };
        if kind == 2 {
            stop(&mut self.state.borrow_mut());
            return;
        }
        let held: Vec<usize> =
            self.state.borrow().notes_on.pads.iter().enumerate().filter(|(_, p)| matches!(p, Some((_, None)))).map(|(i, _)| i).collect();
        let list = if held.is_empty() { (0..16).collect() } else { held };
        let pad = list[((pos.clamp(0.0, 1.0) * list.len() as f64) as usize).min(list.len() - 1)];
        if self.state.borrow().strummed[f].as_ref().is_some_and(|s| s.0 == pad) {
            return;
        }
        // What the pad plays: in Keyboard (or Chords) mode the focused sound's note (or
        // chord), else the pad's sound at its base key.
        let group = self.state.borrow().focus.group;
        let (sound, pitches) = if let Some(chords) = self.keyboard_chords() {
            (self.state.borrow().focus.sound, chords[pad].iter().copied().filter(|&n| n < 128).collect())
        } else if let Some((sound, notes)) = self.keyboard_pads() {
            (sound, if notes[pad] < 128 { vec![notes[pad]] } else { Vec::new() })
        } else {
            let doc = self.state.borrow().doc.clone();
            let base = doc.and_then(|d| Some(d.borrow().project().groups.get(group)?.sounds.get(pad)?.channel.base_key));
            (pad, vec![base.unwrap_or(60)])
        };
        let velocity = self.fixed_velocity().unwrap_or(100);
        let mut st = self.state.borrow_mut();
        stop(&mut st);
        for &pitch in &pitches {
            st.control_requests.push(state::ControlRequest::StripNote { group, sound, pitch, velocity, on: true });
        }
        st.strummed[f] = Some((pad, group, sound, pitches, velocity));
    }

    /// What the app itself does with a pad hit, before the scripts see it. In
    /// scene mode the scripts leave the switch to the app ("done via NHL event -> RT
    /// event"): pad N picks scene N of the focused bank of 16.
    fn native_pad(&mut self, index: i64) {
        // The Snapshots page: a pad holding a snapshot recalls it.
        let top = self.state.borrow().hw.page_stack.last().copied();
        let snapshots = top.is_some() && top == self.constant("NI.HW.PAGE_SNAPSHOTS").ok();
        let mods = ["NI.HW.BUTTON_SHIFT", "NI.HW.BUTTON_ERASE"].map(|b| self.constant(b).ok());
        {
            let mut st = self.state.borrow_mut();
            let held = mods.iter().flatten().any(|b| st.hw.held.contains(b));
            api::lock::native_pad(&mut st, snapshots, held, index);
        }
        let mode = self.state.borrow().hw.pad_mode;
        let scenes = self.constant("NI.HW.PAD_MODE_SCENE").ok() == Some(mode);
        let sections = self.constant("NI.HW.PAD_MODE_SECTION").ok() == Some(mode);
        if !scenes && !sections {
            return;
        }
        // Pad N picks item N of the focused bank of 16 (scenes in the Ideas view,
        // sections in the Song view).
        let bank_param = if scenes { "getFocusSceneBankIndexParameter" } else { "getFocusSectionBankIndexParameter" };
        let bank: i64 = self
            .lua
            .load(format!("local s = NI.DATA.StateHelper.getFocusSong(App); return s and s:{bank_param}():getValue() or 0"))
            .eval::<Option<f64>>()
            .ok()
            .flatten()
            .map_or(0, |v| v as i64);
        let i = bank * 16 + index - 1;
        let mut st = self.state.borrow_mut();
        let (n_scenes, section_scene) = match st.project() {
            Some(p) => (p.scenes.len(), usize::try_from(i).ok().and_then(|i| p.sections.get(i)).map(|s| s.scene)),
            None => (0, None),
        };
        if scenes && (0..n_scenes as i64).contains(&i) {
            st.focus.scene = Some(i as usize);
            st.launched_scene = Some(i as usize);
        } else if let Some(scene) = section_scene.filter(|_| sections) {
            st.focus.section = Some(i as usize);
            if scene.is_some() {
                st.focus.scene = scene;
            }
            st.launched_section = Some(i as usize);
        }
    }

    /// Whether pads currently play sounds (as opposed to picking scenes, sections, ...).
    /// In Keyboard (and Chords) mode, what the pads play: the focused sound, at each pad's
    /// note of the scale (pads 1–16; 128 for none).
    pub fn keyboard_pads(&self) -> Option<(usize, [i64; 16])> {
        let on: bool = self
            .lua
            .load("local g = NI.DATA.StateHelper.getFocusGroup(App); return g ~= nil and g:getKeyboardModeEnabledParameter():getValue() == true")
            .eval()
            .ok()?;
        if !on {
            return None;
        }
        let sound = self.state.borrow().focus.sound;
        let mut notes = [128; 16];
        for (i, n) in notes.iter_mut().enumerate() {
            *n = self
                .lua
                .load(format!("local s = NI.DATA.StateHelper.getFocusSong(App); return s and s:getNoteForPad({i}) or 128"))
                .eval()
                .unwrap_or(128);
        }
        Some((sound, notes))
    }

    /// In Chords mode, the notes each pad plays (pads 1–16; 128 for none), as Maschine's
    /// scale engine voices them (`api::scales::pad_chords`); None otherwise.
    pub fn keyboard_chords(&self) -> Option<[[i64; 16]; 16]> {
        let (_, keys) = self.keyboard_pads()?;
        let root = self.root_note();
        let mut st = self.state.borrow_mut();
        let scale = st.scale;
        let (auto, after) = &mut st.chord_auto;
        api::scales::pad_chords(&scale, root, &keys, auto, after)
    }

    /// Whether pad hits play sounds: not while they pick scenes, sections or patterns,
    /// enter steps or select notes, or do something else of the page's (no pad mode:
    /// muting, picking pages, selecting events).
    pub fn pads_play_sounds(&self) -> bool {
        // (With SHIFT held the pads run the shortcuts printed under them, natively in
        // Maschine: they don't play.)
        let shift = self.constant("NI.HW.BUTTON_SHIFT").ok();
        if shift.is_some_and(|b| self.state.borrow().hw.held.contains(&b)) {
            return false;
        }
        let Ok(mode) = self.lua.load("return NHLController:getPadMode()").eval::<i64>() else { return true };
        ![
            "NI.HW.PAD_MODE_SCENE",
            "NI.HW.PAD_MODE_SECTION",
            "NI.HW.PAD_MODE_PATTERN",
            "NI.HW.PAD_MODE_STEP",
            "NI.HW.PAD_MODE_SELECT_NOTE_EVENTS",
            "NI.HW.PAD_MODE_NONE",
        ]
        .iter()
        .any(|c| self.constant(c).ok() == Some(mode))
    }

    /// An LED as the scripts last set it (`LED_PAD_1`, ...); None if they never did.
    pub fn led(&self, led: &str) -> Option<state::Led> {
        let id = self.constant(&format!("NI.HW.{led}")).ok()?;
        self.state.borrow().hw.leds.get(&id).copied()
    }

    /// What an LED shows, as RGB 0..255, or None if it's off. Colour-table LEDs are
    /// looked up in the scripts' own `LEDColors` table (index × LED state).
    pub fn led_rgb(&self, led: &str) -> Option<[u8; 3]> {
        let id = self.constant(&format!("NI.HW.{led}")).ok()?;
        let l = *self.state.borrow().hw.leds.get(&id)?;
        if !l.is_lit() {
            return None;
        }
        let scale = |v: f64| (v.clamp(0.0, 127.0) * 2.0) as u8;
        if let Some((color, led_state)) = l.lut {
            let colors: Table = self.lua.globals().get("LEDColors").ok()?;
            let white: i64 = colors.get("WHITE").unwrap_or(16);
            let row: Table = colors.get(if color < 0 { white } else { color }).ok()?;
            let rgb: Table = row.get(led_state).ok()?;
            return Some([scale(rgb.get(1).ok()?), scale(rgb.get(2).ok()?), scale(rgb.get(3).ok()?)]);
        }
        if let Some((r, g, b)) = l.rgb {
            return Some([scale(r as f64), scale(g as f64), scale(b as f64)]);
        }
        let v = (l.level.clamp(0.0, 1.0) * 255.0) as u8;
        Some([v, v, v])
    }

    /// Widget id of the active page's screen root, if the controller has one.
    pub fn active_screen(&self) -> Option<usize> {
        let v: Value = self
            .lua
            // Forward pages (SCENE, ...) keep the screen on their current sub-page.
            .load(
                "local c = ControllerScriptInterface; local p = c and c.ActivePage \
                 while p and not p.Screen and p.CurrentPage and p.CurrentPage ~= p do p = p.CurrentPage end \
                 local s = p and p.Screen; return s and s.RootBar",
            )
            .eval()
            .ok()?;
        api::object_id(&v)
    }

    /// The active page's screen buttons (1–8, above the displays): label, selected,
    /// enabled, visible, as the scripts set them.
    pub fn screen_buttons(&self) -> Vec<ScreenButton> {
        let ids: Vec<Value> = self
            .lua
            .load(
                "local c = ControllerScriptInterface; local p = c and c.ActivePage \
                 while p and not p.Screen and p.CurrentPage and p.CurrentPage ~= p do p = p.CurrentPage end \
                 local b = p and p.Screen and p.Screen.ScreenButton; local out = {} \
                 if type(b) == 'table' then for i = 1, 8 do out[i] = b[i] or false end end \
                 return out",
            )
            .eval::<Table>()
            .map(|t| t.sequence_values::<Value>().filter_map(Result::ok).collect())
            .unwrap_or_default();
        let st = self.state.borrow();
        // Picking scenes (the Ideas view): each button is its scene, then "+".
        if Some(st.hw.screen_button_mode) == self.constant("NI.HW.SCREENBUTTON_MODE_SCENES").ok()
            && let Some(p) = st.project()
        {
            let first = st.hw.scene_bank8.max(0) as usize * 8;
            return (first..first + 8)
                .map(|i| match p.scenes.get(i) {
                    Some(s) => ScreenButton { label: s.name.clone(), selected: st.focus.scene == Some(i), enabled: true, visible: true },
                    None if i == p.scenes.len() => ScreenButton { label: "+".into(), selected: false, enabled: true, visible: true },
                    None => ScreenButton { visible: false, ..Default::default() },
                })
                .collect();
        }
        ids.iter()
            .map(|v| {
                let Some(w) = api::object_id(v).and_then(|id| st.widgets.all.get(id)) else { return ScreenButton::default() };
                let flag = |k: &str, default: bool| match w.props.get(k) {
                    Some(state::Prop::Bool(b)) => *b,
                    _ => default,
                };
                let label = match (w.props.get("Text"), w.props.get("style")) {
                    (Some(state::Prop::Str(t)), _) if !t.is_empty() => t.clone(),
                    (_, Some(state::Prop::Str(t))) => t.clone(),
                    _ => String::new(),
                };
                ScreenButton { label, selected: flag("Selected", false), enabled: flag("Enabled", true), visible: flag("Visible", true) }
            })
            .collect()
    }

    /// Stubbed API paths, most-touched first.
    pub fn report(&self) -> Vec<(String, &'static str, u64)> {
        let mut v: Vec<_> = self.recorder.borrow().hits.iter().map(|(k, (kind, n))| (k.clone(), *kind, *n)).collect();
        v.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
        v
    }
}

/// A screen button as the scripts last set it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScreenButton {
    pub label: String,
    pub selected: bool,
    pub enabled: bool,
    pub visible: bool,
}

/// The style sheet's key for a plug-in's pictures (`image-<key>`): its name in small
/// letters, without spaces or parenthesised parts ("Bass Synth" → "basssynth").
pub fn plugin_picture_key(name: &str) -> String {
    api::utils::remove_parens_and_whitespace(&name.to_lowercase())
}

/// Where the host keeps DECKARD's own picture of a plug-in: its chain tile, or its logo.
pub fn plugin_picture_path(key: &str, thumb: bool) -> String {
    pattern_view::plugin_picture(key, thumb)
}

/// Recovered constant values, as `{ hw = {...}, hosting = {...}, nsa = {...}, data = {...} }`
/// name -> value tables. `NI.HW` values depend on the controller family.
fn known_constants(lua: &Lua, scripts: &Scripts, controller: &str) -> mlua::Result<Table> {
    let family = match controller {
        "MJ1" => "registerJamHW",
        c if c.starts_with("KK") => "registerKompleteHW",
        _ => "registerMaschineHW",
    };
    let pick = |funcs: &[&str]| -> mlua::Result<Table> {
        let t = lua.create_table()?;
        // Later functions in `funcs` win.
        for f in funcs {
            for (_, name, v) in scripts.constants.iter().filter(|c| c.0 == *f) {
                t.set(name.as_str(), *v)?;
            }
        }
        Ok(t)
    };
    let t = lua.create_table()?;
    t.set("hw", pick(&["registerHW", family])?)?;
    let hosting = pick(&["registerHosting"])?;
    // Not among the recovered values: DECKARD's own bit for "Audio Units only" (a filter
    // only the scripts and this host pass between them).
    if !hosting.contains_key("SKIP_NOT_AUv2_PLUGINS")? {
        hosting.set("SKIP_NOT_AUv2_PLUGINS", 4096)?;
    }
    t.set("hosting", hosting)?;
    t.set("nsa", pick(&["registerNSA"])?)?;
    let data = pick(&["setConstants", "registerDataObjectsAccess", "registerDataParameters", "registerDATA"])?;
    for (name, v) in api::pad_mode::FOCUS {
        if !data.contains_key(name)? {
            data.set(name, v)?;
        }
    }
    // `NI.DB3` (the browser) isn't among the recovered values: DECKARD's own.
    for (name, v) in api::browser::CONSTANTS {
        if !data.contains_key(name)? {
            data.set(name, v)?;
        }
    }
    t.set("data", data)?;
    Ok(t)
}

/// `require "Scripts/..."` loads from the script set; standard modules (`math`, ...)
/// come from `package.loaded` as usual. C modules are never loaded.
fn install_require(lua: &Lua, scripts: Scripts) -> mlua::Result<()> {
    let package: Table = lua.globals().get("package")?;
    let preload: Function = lua.load("return package.searchers[1]").eval()?;
    for k in ["loadlib", "cpath", "path", "searchpath"] {
        package.set(k, Value::Nil)?;
    }
    let scripts = Rc::new(scripts);
    let searcher = lua.create_function(move |lua, name: String| match scripts.source(&name) {
        Some((path, src)) => {
            let chunk = lua.load(src).set_name(format!("@{}", path.display())).into_function()?;
            Ok(Value::Function(chunk))
        }
        None => Ok(Value::String(lua.create_string(format!("\n\tno script '{name}' in the script set"))?)),
    })?;
    let searchers = lua.create_sequence_from([preload, searcher])?;
    package.set("searchers", searchers)?;
    Ok(())
}

/// Colours changed since the scripts last looked (from the app, or by the scripts'
/// colour calls, which don't go through a parameter) are flagged as their `Color`
/// parameter changed: the screens redraw a colour only when it is.
fn flag_color_changes(st: &mut State) {
    use state::ObjRef;
    let Some(pr) = st.project() else { return };
    let now: Vec<(ObjRef, deckard_model::Color)> = pr
        .groups
        .iter()
        .enumerate()
        .flat_map(|(g, group)| {
            std::iter::once((ObjRef::Group(g), group.color))
                .chain(group.sounds.iter().enumerate().map(move |(s, sound)| (ObjRef::Sound(g, s), sound.color)))
        })
        .collect();
    drop(pr);
    let changed: Vec<ObjRef> =
        now.iter().filter(|c| !st.colors_seen.is_empty() && !st.colors_seen.contains(c)).map(|(o, _)| o.clone()).collect();
    for owner in changed {
        let owner = st.objects.intern(owner);
        let pid = st.objects.intern(ObjRef::Param { owner, name: "Color".into() });
        st.changed_params.insert(pid);
    }
    st.colors_seen = now;
}
