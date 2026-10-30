//! The `deckard` table the scripts call (docs/controller-api.md). Groups, sounds, knobs,
//! pages and display buttons count from 1 in Lua, from 0 in the app.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

use crate::{Action, Browse, ColorOf, Shared, View, leds, names};

/// Tempos the app takes: 10–400 BPM.
pub fn tempo_in_range(bpm: f64) -> f64 {
    bpm.clamp(10.0, 400.0)
}

pub fn install(lua: &Lua, shared: &Rc<RefCell<Shared>>) -> mlua::Result<()> {
    let d = lua.create_table()?;

    // Reading.
    let sh = shared.clone();
    d.set("state", lua.create_function(move |lua, ()| state(lua, &sh.borrow().view))?)?;
    d.set("scales", scales(lua)?)?;
    {
        // User chord `slot` of set `set` (from 1): the semitones above the root.
        let sh = shared.clone();
        d.set(
            "user_chord",
            lua.create_function(move |lua, (set, slot): (usize, usize)| {
                let v = &sh.borrow().view;
                let notes = v.user_chords.get(set.wrapping_sub(1)).and_then(|s| s.get(slot.wrapping_sub(1))).cloned().unwrap_or_default();
                lua.create_sequence_from(notes)
            })?,
        )?;
    }
    {
        // A note's name in the scale chosen played from `root` (none off it).
        let sh = shared.clone();
        d.set(
            "note_name",
            lua.create_function(move |_, (root, note): (i64, i64)| {
                let v = &sh.borrow().view;
                let scale = (v.scale_bank.clamp(0, 7) * 15 + v.scale_kind.clamp(0, 14)) as usize;
                Ok(deckard_model::scales::note_name(scale, root, note))
            })?,
        )?;
    }
    {
        // The chord a pad sending `note` plays in Chords mode (docs/scales.md), and Auto's
        // state after it.
        let sh = shared.clone();
        d.set(
            "chord",
            lua.create_function(move |lua, (note, spec, auto): (i64, Table, Option<Table>)| {
                let v = &sh.borrow().view;
                let mode: i64 = spec.get("mode")?;
                let kind = spec.get::<usize>("type")?.saturating_sub(1);
                let user = if mode == 3 { v.user_chords.get(kind).map(Vec::as_slice).unwrap_or_default() } else { &[] };
                let chords = deckard_model::chords::Chords {
                    mode,
                    scale: (v.scale_bank.clamp(0, 7) * 15 + v.scale_kind.clamp(0, 14)) as usize,
                    root: spec.get("root")?,
                    kind,
                    chromatic: spec.get::<usize>("chromatic")?.saturating_sub(1),
                    position: spec.get::<usize>("position")?.saturating_sub(1),
                    user,
                };
                let mut state = deckard_model::chords::Auto::default();
                if let Some(a) = &auto {
                    state.anchor = a.get::<Option<Vec<i64>>>("anchor")?.unwrap_or_default();
                    state.last = a.get::<Option<i64>>("last")?.unwrap_or(0);
                    state.of = a.get::<Option<String>>("of")?.unwrap_or_default();
                }
                let notes = chords.play(note, &mut state);
                let next = lua.create_table()?;
                next.set("anchor", lua.create_sequence_from(state.anchor)?)?;
                next.set("last", state.last)?;
                next.set("of", state.of)?;
                Ok((lua.create_sequence_from(notes)?, next))
            })?,
        )?;
    }
    let sh = shared.clone();
    d.set("held", lua.create_function(move |_, name: String| Ok(sh.borrow().held.contains(&name)))?)?;
    let sh = shared.clone();
    d.set("pad_held", lua.create_function(move |_, n: usize| Ok(n >= 1 && n <= 16 && sh.borrow().pads[n - 1]))?)?;
    d.set("device", lua.create_function(|_, ()| Ok(crate::sources::DEVICE))?)?;
    let sh = shared.clone();
    d.set("now", lua.create_function(move |_, ()| Ok(sh.borrow().now))?)?;
    let sh = shared.clone();
    d.set("page", lua.create_function(move |_, ()| Ok(sh.borrow().page.clone()))?)?;
    let sh = shared.clone();
    // (The mod matrix left for a page shown while a button is held: it's coming back.)
    let parked = std::cell::Cell::new(false);
    d.set(
        "show_page",
        lua.create_function(move |_, name: String| {
            // The mod matrix gone for good (not just under a held page): the slot being
            // made goes with it.
            const HELD: [&str; 6] = ["hold", "pattern", "scene", "grid", "events", "note_repeat"];
            let from_matrix = sh.borrow().page == "mod_matrix";
            if name == "mod_matrix" {
                parked.set(false);
            } else if from_matrix && HELD.contains(&name.as_str()) {
                parked.set(true);
            } else if from_matrix || (parked.get() && !HELD.contains(&name.as_str())) {
                parked.set(false);
                sh.borrow_mut().actions.push(Action::ModSlotsClosed);
            }
            sh.borrow_mut().page = name;
            Ok(())
        })?,
    )?;

    // Lights.
    let sh = shared.clone();
    d.set(
        "led",
        lua.create_function(move |_, (name, how, bright): (String, Value, Option<bool>)| {
            let Some(hw) = names::light(&name) else { return Err(mlua::Error::runtime(format!("no light {name}"))) };
            let byte = match &how {
                Value::String(s) => match (s.to_str()?.as_ref(), deckard_mk3::leds::is_colour(&hw)) {
                    ("off", true) => 0,
                    ("off", false) => deckard_mk3::leds::BACKLIGHT,
                    ("dark", _) => 0,
                    ("dim", false) => deckard_mk3::leds::DIM,
                    ("on", false) => leds::ON,
                    ("dim", true) => leds::dim([255; 3]),
                    ("on", true) => leds::bright([255; 3]),
                    (other, _) => return Err(mlua::Error::runtime(format!("light {name}: \"{other}\"? (off, dim, on)"))),
                },
                Value::Table(t) => {
                    let rgb = crate::screen::rgb(t)?;
                    if bright.unwrap_or(true) { leds::bright(rgb) } else { leds::dim(rgb) }
                }
                // A palette index (0..17, 16 white), as the project's colours are.
                Value::Integer(i) => deckard_mk3::leds::palette(*i, if bright.unwrap_or(true) { 4 } else { 2 }),
                Value::Nil => 0,
                _ => return Err(mlua::Error::runtime(format!("light {name}: a level or a colour"))),
            };
            sh.borrow_mut().leds.set(&hw, byte);
            Ok(())
        })?,
    )?;

    // Requests.
    let act = |sh: &Rc<RefCell<Shared>>, a: Action| sh.borrow_mut().actions.push(a);
    macro_rules! request {
        ($name:literal, ($($arg:ident : $t:ty),*) => $make:expr) => {{
            let sh = shared.clone();
            d.set(
                $name,
                lua.create_function(move |_, ($($arg,)*): ($($t,)*)| {
                    act(&sh, $make);
                    Ok(())
                })?,
            )?;
        }};
    }
    /// A count from 1 in Lua as an index from 0.
    fn index(n: usize, what: &str) -> mlua::Result<usize> {
        n.checked_sub(1).ok_or_else(|| mlua::Error::runtime(format!("{what} counts from 1")))
    }
    macro_rules! indexed {
        ($name:literal, $what:literal, $variant:path) => {{
            let sh = shared.clone();
            d.set(
                $name,
                lua.create_function(move |_, n: usize| {
                    act(&sh, $variant(index(n, $what)?));
                    Ok(())
                })?,
            )?;
        }};
    }
    request!("play", () => Action::Play);
    request!("stop", () => Action::Stop);
    request!("record", (on: bool, count_in: Option<bool>) => Action::Record { on, count_in: count_in.unwrap_or(false) });
    request!("restart", () => Action::Restart);
    request!("set_loop", (on: bool) => Action::Loop(on));
    request!("metronome", (on: bool) => Action::Metronome(on));
    request!("auto_write", (on: bool) => Action::AutoWrite(on));
    request!("tempo", (bpm: f64) => Action::Tempo(tempo_in_range(bpm)));
    request!("swing", (amount: f64) => Action::Swing(amount.clamp(0.0, 100.0)));
    request!("volume", (level: f64) => Action::Volume(level.clamp(0.0, 1.0)));
    request!("volume_db", (db: f64) => Action::VolumeDb(db.min(MAX_DB)));
    request!("fixed_velocity", (on: bool) => Action::FixedVelocity(on));
    request!("prob_lock", (on: bool) => Action::ProbLock(on));
    request!("velocity", (v: i64) => Action::VelocityValue(v.clamp(0, 127)));
    request!("add_group", () => Action::AddGroup);
    // (Sets 1–16 and slots 1–12, as the scripts count.)
    let set = |n: i64| (n.clamp(1, 16) - 1) as usize;
    let slot = |n: i64| (n.clamp(1, 12) - 1) as usize;
    request!("user_chord_note", (s: i64, k: i64, note: i64) => Action::UserChordNote { set: set(s), slot: slot(k), note: note.clamp(-127, 127) });
    request!("user_chord_preview", (s: i64, k: i64, root: i64, on: bool) => Action::UserChordPreview {
        set: set(s),
        slot: slot(k),
        root: root.clamp(0, 127),
        on,
    });
    request!("user_chord_move", (s: i64, k: i64, next: bool) => Action::UserChordMove { set: set(s), slot: slot(k), next });
    request!("user_chord_delete", (s: i64, k: i64) => Action::UserChordDelete { set: set(s), slot: slot(k) });
    request!("scale", (root: i64, bank: i64, kind: i64) => Action::Scale {
        root: root.clamp(0, 127),
        bank: (bank - 1).clamp(0, 7),
        kind: (kind - 1).clamp(0, 14),
    });
    request!("keyboard_mode", (on: bool) => Action::KeyboardMode(on));
    request!("next_page", () => Action::Page(true));
    request!("previous_page", () => Action::Page(false));
    request!("slot", (delta: i64) => Action::Slot(delta));
    request!("pattern_span", (start: i64, length: i64) => Action::PatternSpan { start: start.max(0), length: length.max(1) });
    request!("pattern_length", (length: i64) => Action::PatternLength(length.max(1)));
    {
        let sh = shared.clone();
        d.set(
            "set_color",
            lua.create_function(move |_, (what, color): (String, Option<i64>)| {
                let of = match what.as_str() {
                    "pattern" => ColorOf::Pattern,
                    "scene" => ColorOf::Scene,
                    "group" => ColorOf::Group,
                    "sound" => ColorOf::Sound,
                    _ => return Err(mlua::Error::runtime(format!("colour what? \"{what}\" (pattern, scene, group, sound)"))),
                };
                act(&sh, Action::Color { of, color: color.map(|c| c.clamp(0, 15) as u8) });
                Ok(())
            })?,
        )?;
    }
    {
        let sh = shared.clone();
        d.set(
            "pad_touch",
            lua.create_function(move |_, (name, value): (String, i64)| {
                let (field, max) = match name.as_str() {
                    "mode" => ("mode", 3),
                    "cc" => ("cc", 127),
                    "curve" => ("curve", 100),
                    "min" => ("min", 100),
                    "max" => ("max", 100),
                    _ => return Err(mlua::Error::runtime(format!("no Pad Touch setting \"{name}\" (mode, cc, curve, min, max)"))),
                };
                act(&sh, Action::PadTouch(field, value.clamp(0, max)));
                Ok(())
            })?,
        )?;
    }
    {
        let sh = shared.clone();
        d.set(
            "grid",
            lua.create_function(move |_, (name, value): (String, mlua::Value)| {
                // (Each value's place in its list, from 0; the switches true / false.)
                let (field, max) = match name.as_str() {
                    "perform" => ("perform", 6),
                    "arrange" => ("arrange", 4),
                    "arrange_snap" => ("arrange_snap", 1),
                    "quick" => ("quick", 1),
                    "step" => ("step", 14),
                    "step_snap" => ("step_snap", 1),
                    "nudge" => ("nudge", 4),
                    _ => {
                        return Err(mlua::Error::runtime(format!(
                            "no grid \"{name}\" (perform, arrange, arrange_snap, quick, step, step_snap, nudge)"
                        )));
                    }
                };
                let value = match value {
                    mlua::Value::Boolean(on) => i64::from(on),
                    mlua::Value::Integer(i) => i,
                    mlua::Value::Number(n) => n.round() as i64,
                    _ => return Err(mlua::Error::runtime(format!("grid \"{name}\": a number or true / false"))),
                };
                act(&sh, Action::Grid(field, value.clamp(0, max)));
                Ok(())
            })?,
        )?;
    }
    request!("follow", (on: bool) => Action::Follow(on));
    indexed!("groove_page", "a Groove page", Action::GroovePage);
    {
        let sh = shared.clone();
        d.set(
            "file",
            lua.create_function(move |_, (what, n): (String, Option<usize>)| {
                const WHAT: [&str; 7] = ["new", "save", "save_copy", "export_audio", "save_group", "save_sound", "load"];
                let Some(&what) = WHAT.iter().find(|w| **w == what) else {
                    return Err(mlua::Error::runtime(format!("no file command \"{what}\" ({})", WHAT.join(", "))));
                };
                // (Recent projects count from 1, as the scripts count.)
                let n = if what == "load" {
                    match n {
                        Some(n) if n >= 1 => n - 1,
                        _ => return Err(mlua::Error::runtime("file(\"load\", n): a recent project, from 1")),
                    }
                } else {
                    0
                };
                act(&sh, Action::File(what, n));
                Ok(())
            })?,
        )?;
    }
    {
        let sh = shared.clone();
        d.set(
            "setting",
            lua.create_function(move |_, (name, step, index): (String, i64, Option<usize>)| {
                const NAMES: [&str; 31] = [
                    "count_in",
                    "numerator",
                    "denominator",
                    "quantize",
                    "device",
                    "buffer_size",
                    "in_route",
                    "out_route",
                    "midi_sync",
                    "clock_offset",
                    "default_input",
                    "midi_in",
                    "midi_out",
                    "link",
                    "link_start_stop",
                    "sensitivity",
                    "display_brightness",
                    "metronome_volume",
                    "metronome_auto",
                    "velocity_scaling",
                    "led_brightness",
                    "strip_position",
                    "touch_auto_write",
                    "metronome_ticks",
                    "reload_last",
                    "auto_backup",
                    "duplicate",
                    "link_duplicates",
                    "pattern_length",
                    "pattern_length_fine",
                    "pattern_grow",
                ];
                let Some(&name) = NAMES.iter().find(|n| **n == name) else {
                    return Err(mlua::Error::runtime(format!("no setting \"{name}\" ({})", NAMES.join(", "))));
                };
                // (Channels and ports count from 1, as the pages count.)
                let index = match index {
                    Some(i) if i >= 1 => i - 1,
                    None => 0,
                    _ => return Err(mlua::Error::runtime("setting(name, step, index): index from 1")),
                };
                act(&sh, Action::Setting(name, index, step.signum()));
                Ok(())
            })?,
        )?;
    }
    request!("show_plugins", (on: bool) => Action::ModulesVisible(on));
    {
        let sh = shared.clone();
        d.set(
            "sound_setting",
            lua.create_function(move |_, (name, value): (String, i64)| {
                let (field, max) = match name.as_str() {
                    "choke_group" => ("choke_group", 8),
                    "link_group" => ("link_group", 8),
                    "choke_mode" => ("choke_mode", 1),
                    "link_mode" => ("link_mode", 1),
                    _ => {
                        return Err(mlua::Error::runtime(format!(
                            "no sound setting \"{name}\" (choke_group, choke_mode, link_group, link_mode)"
                        )));
                    }
                };
                act(&sh, Action::SoundSetting(field, value.clamp(0, max)));
                Ok(())
            })?,
        )?;
    }
    // A group's (sound `nil`) or a sound's level (dB) and pan (−1..1).
    let channel = |group: usize, sound: Option<usize>| -> mlua::Result<(usize, Option<usize>)> {
        Ok((index(group, "a group")?, sound.map(|s| index(s, "a sound")).transpose()?))
    };
    {
        let sh = shared.clone();
        d.set(
            "set_level",
            lua.create_function(move |_, (group, sound, db): (usize, Option<usize>, f64)| {
                let (group, sound) = channel(group, sound)?;
                act(&sh, Action::ChannelLevel { group, sound, db: db.min(MAX_DB) });
                Ok(())
            })?,
        )?;
        let sh = shared.clone();
        d.set(
            "set_pan",
            lua.create_function(move |_, (group, sound, pan): (usize, Option<usize>, f64)| {
                let (group, sound) = channel(group, sound)?;
                act(&sh, Action::ChannelPan { group, sound, pan: pan.clamp(-1.0, 1.0) });
                Ok(())
            })?,
        )?;
    }
    {
        let sh = shared.clone();
        d.set(
            "pattern_edit",
            lua.create_function(move |_, what: String| {
                let Some(&w) = ["create", "double", "duplicate", "delete"].iter().find(|w| **w == what) else {
                    return Err(mlua::Error::runtime(format!("no pattern edit \"{what}\" (create, double, duplicate, delete)")));
                };
                act(&sh, Action::PatternEdit(w));
                Ok(())
            })?,
        )?;
        let sh = shared.clone();
        d.set(
            "scene_edit",
            lua.create_function(move |_, what: String| {
                let Some(&w) = ["create", "insert", "duplicate", "delete", "clear", "append"].iter().find(|w| **w == what) else {
                    return Err(mlua::Error::runtime(format!(
                        "no scene edit \"{what}\" (create, insert, duplicate, delete, clear, append)"
                    )));
                };
                act(&sh, Action::SceneEdit(w));
                Ok(())
            })?,
        )?;
        let sh = shared.clone();
        d.set(
            "section_edit",
            lua.create_function(move |_, what: String| {
                let Some(&w) = ["create", "duplicate", "delete"].iter().find(|w| **w == what) else {
                    return Err(mlua::Error::runtime(format!("no section edit \"{what}\" (create, duplicate, delete)")));
                };
                act(&sh, Action::SectionEdit(w));
                Ok(())
            })?,
        )?;
    }
    // The library (BROWSE): the content kind (1–7), user content (else factory), a
    // product, an attribute (1 Type, 2 Sub-type, 3 Character; `nil` for all), the
    // result focus moved; the focused result loaded, preheard or made a favourite (or
    // not); favourites only.
    request!("browse_kind", (n: usize) => Action::Browse(Browse::Kind(index(n, "a content kind")?.min(6))));
    request!("browse_user", (on: bool) => Action::Browse(Browse::User(on)));
    request!("browse_product", (n: Option<usize>) => Action::Browse(Browse::Product(n.map(|n| index(n, "a product")).transpose()?)));
    request!("browse_attribute", (which: usize, n: Option<usize>) => {
        let which = index(which, "an attribute")?;
        if which > 2 {
            return Err(mlua::Error::runtime("an attribute is 1 (Type), 2 (Sub-type) or 3 (Character)"));
        }
        Action::Browse(Browse::Attribute(which, n.map(|n| index(n, "a value")).transpose()?))
    });
    request!("browse_focus", (by: i64) => Action::Browse(Browse::Focus(by)));
    request!("browse_load", () => Action::Browse(Browse::Load));
    request!("browse_favorites", () => Action::Browse(Browse::Favorites));
    request!("browse_set_favorite", () => Action::Browse(Browse::SetFavorite));
    request!("browse_prehear", () => Action::Browse(Browse::Prehear));
    request!("song_view", (on: bool) => Action::SongView(on));
    request!("move_section", (next: bool) => Action::SectionMove(next));
    request!("section_scene", (next: bool) => Action::SectionScene(next));
    request!("section_length", (ticks: i64) => Action::SectionLength(ticks.max(1)));
    request!("load_module", (n: Option<usize>) => Action::LoadModule(n.map(|n| index(n, "a module")).transpose()?));
    indexed!("focus_group", "a group", Action::FocusGroup);
    indexed!("focus_scene", "a scene", Action::FocusScene);
    indexed!("launch_scene", "a scene", Action::LaunchScene);
    indexed!("focus_section", "a section", Action::FocusSection);
    indexed!("pick_pattern", "a pattern", Action::PickPattern);
    let sh = shared.clone();
    d.set(
        "scene_pattern",
        lua.create_function(move |_, (group, pattern): (usize, Option<usize>)| {
            let group = index(group, "a group")?;
            let pattern = pattern.map(|p| index(p, "a pattern")).transpose()?;
            act(&sh, Action::ScenePattern { group, pattern });
            Ok(())
        })?,
    )?;
    indexed!("touch_write", "a knob", Action::TouchWrite);
    // The Sampling page (docs/controller-api.md "Sampling").
    request!("sampling", (open: bool) => Action::Sampling(open));
    {
        let sh = shared.clone();
        d.set(
            "sampling_tab",
            lua.create_function(move |_, tab: usize| {
                let tab = index(tab, "a tab")?;
                if tab > 3 {
                    return Err(mlua::Error::runtime("sampling_tab: 1 RECORD … 4 ZONE"));
                }
                act(&sh, Action::SamplingTab(tab));
                Ok(())
            })?,
        )?;
    }
    {
        let sh = shared.clone();
        d.set(
            "sampling_knob",
            lua.create_function(move |_, (page, knob, value): (usize, usize, f64)| {
                act(&sh, Action::SamplingKnob { page: index(page, "a page")?, knob: index(knob, "a knob")?, value });
                Ok(())
            })?,
        )?;
    }
    {
        // A sample's slices and zones count from 1, its frames from 0.
        let sh = shared.clone();
        d.set(
            "sampling_call",
            lua.create_function(move |_, (what, a, b): (String, Option<f64>, Option<f64>)| {
                use crate::SamplingCall as C;
                let n = |x: Option<f64>, what: &str| -> mlua::Result<usize> {
                    index(x.ok_or_else(|| mlua::Error::runtime(format!("sampling_call: {what} wanted")))?.round().max(0.0) as usize, what)
                };
                let frame = |x: Option<f64>| x.unwrap_or(0.0).max(0.0);
                let call = match what.as_str() {
                    "edit" => C::Edit(n(a, "an edit function")? + 1),
                    "auto_loop" => C::AutoLoop,
                    "stems" => C::Stems,
                    "cancel_stems" => C::CancelStems,
                    "view" => C::View(frame(a), frame(b)),
                    "split_slice" => C::SplitSlice(n(a, "a slice")?),
                    "remove_slice" => C::RemoveSlice(n(a, "a slice")?),
                    "clear_slices" => C::ClearSlices,
                    "move_slice" => C::MoveSlice(n(a, "a slice")?, frame(b) as usize),
                    "apply_slices" => C::ApplySlices,
                    "remove_zone" => C::RemoveZone(n(a, "a zone")?),
                    "focus_zone" => C::FocusZone(n(a, "a zone")?),
                    "prehear" => C::Prehear(frame(a) as usize, frame(b) as usize),
                    _ => return Err(mlua::Error::runtime(format!("no sampling call \"{what}\""))),
                };
                act(&sh, Action::SamplingCall(call));
                Ok(())
            })?,
        )?;
    }
    request!("record_sample", (cancel: Option<bool>) => Action::RecordSample(if cancel == Some(true) { crate::RecordPress::Cancel } else { crate::RecordPress::Toggle }));
    {
        let sh = shared.clone();
        d.set(
            "take",
            lua.create_function(move |_, (n, delete): (usize, Option<bool>)| {
                act(&sh, Action::Take { index: index(n, "a take")?, delete: delete == Some(true) });
                Ok(())
            })?,
        )?;
    }
    {
        // The focused sample's peaks, a column of the display each: `overview` (the
        // whole sample) or `detail` (the frames shown), as min, max, min, max …
        let sh = shared.clone();
        d.set(
            "peaks",
            lua.create_function(move |lua, which: String| {
                let v = &sh.borrow().view;
                let Some(s) = &v.sampling.sample else { return lua.create_table() };
                let peaks = if which == "detail" { &s.detail } else { &s.overview };
                lua.create_sequence_from(peaks.iter().flat_map(|&(lo, hi)| [lo, hi]))
            })?,
        )?;
    }
    indexed!("focus_sound", "a sound", Action::FocusSound);
    indexed!("mute_group", "a group", Action::MuteGroup);
    indexed!("solo_group", "a group", Action::SoloGroup);
    indexed!("mute_sound", "a sound", Action::MuteSound);
    indexed!("solo_sound", "a sound", Action::SoloSound);
    indexed!("erase_sound", "a sound", Action::EraseSound);
    indexed!("reset_sound", "a sound", Action::ResetSound);
    indexed!("delete_group", "a group", Action::DeleteGroup);
    indexed!("level", "a level", Action::Level);
    indexed!("page_group", "a page group", Action::PageGroup);
    {
        let sh = shared.clone();
        d.set(
            "lock",
            lua.create_function(move |_, (what, n): (String, Option<mlua::Value>)| {
                const WHAT: [&str; 10] =
                    ["button", "pad", "update", "delete", "delete_slot", "bank", "morph", "morph_mode", "morph_time", "morph_grid"];
                let Some(&what) = WHAT.iter().find(|w| **w == what) else {
                    return Err(mlua::Error::runtime(format!("no lock command \"{what}\" ({})", WHAT.join(", "))));
                };
                let n = match (what, n) {
                    // (Pads and slots count from 1, as the pages count.)
                    ("pad" | "delete_slot", Some(mlua::Value::Integer(n))) if n >= 1 => n as usize - 1,
                    ("pad" | "delete_slot", _) => return Err(mlua::Error::runtime(format!("lock(\"{what}\", n): n from 1"))),
                    ("bank", Some(mlua::Value::Integer(n))) => usize::from(n > 0),
                    (_, Some(mlua::Value::Boolean(on))) => usize::from(on),
                    (_, Some(mlua::Value::Integer(n))) if n >= 0 => n as usize,
                    (_, _) => 0,
                };
                act(&sh, Action::Lock(what, n));
                Ok(())
            })?,
        )?;
    }
    let sh = shared.clone();
    d.set(
        "strip",
        lua.create_function(move |_, (what, value): (String, f64)| {
            const WHAT: [&str; 5] = ["pitch", "mod", "perform_touch", "perform", "perform_release"];
            let Some(&what) = WHAT.iter().find(|w| **w == what) else {
                return Err(mlua::Error::runtime(format!("no strip \"{what}\" ({})", WHAT.join(", "))));
            };
            let value = if what == "pitch" { value.clamp(-1.0, 1.0) } else { value.clamp(0.0, 1.0) };
            act(&sh, Action::Strip(what, value));
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "strip_note",
        lua.create_function(move |_, (sound, pitch, velocity, on): (usize, i64, i64, bool)| {
            if sound < 1 {
                return Err(mlua::Error::runtime("strip_note(sound, ...): sound from 1"));
            }
            act(&sh, Action::StripNote { sound: sound - 1, pitch: pitch.clamp(0, 127), velocity: velocity.clamp(1, 127), on });
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "edit",
        lua.create_function(move |_, what: String| {
            const WHAT: [&str; 7] = ["undo", "redo", "undo_step", "redo_step", "copy", "paste", "clear_modulation"];
            let Some(&what) = WHAT.iter().find(|w| **w == what) else {
                return Err(mlua::Error::runtime(format!("no edit \"{what}\" ({})", WHAT.join(", "))));
            };
            act(&sh, Action::Edit(what));
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "perform_fx",
        lua.create_function(move |_, ()| {
            act(&sh, Action::PerformFx);
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "note_repeat",
        lua.create_function(move |_, (what, value): (String, mlua::Value)| {
            const WHAT: [&str; 16] = [
                "type",
                "rate",
                "rate_unit",
                "sequence",
                "octaves",
                "dynamic",
                "gate",
                "retrigger",
                "repeat",
                "offset",
                "inversion",
                "min_key",
                "max_key",
                "hold",
                "locked",
                "preset",
            ];
            let Some(&what) = WHAT.iter().find(|w| **w == what) else {
                return Err(mlua::Error::runtime(format!("no Note Repeat setting \"{what}\" ({})", WHAT.join(", "))));
            };
            let value = match value {
                mlua::Value::Boolean(on) => f64::from(u8::from(on)),
                mlua::Value::Integer(n) => n as f64,
                mlua::Value::Number(n) => n,
                _ => return Err(mlua::Error::runtime(format!("note_repeat(\"{what}\", value): a number or a boolean"))),
            };
            act(&sh, Action::Repeat(what, value));
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "slot_edit",
        lua.create_function(move |_, what: String| {
            const WHAT: [&str; 5] = ["edit", "move_left", "move_right", "bypass", "remove"];
            let Some(&what) = WHAT.iter().find(|w| **w == what) else {
                return Err(mlua::Error::runtime(format!("no slot command \"{what}\" ({})", WHAT.join(", "))));
            };
            act(&sh, Action::SlotEdit(what));
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "events",
        lua.create_function(move |_, (what, n): (String, Option<i64>)| {
            const WHAT: [&str; 22] = [
                "nudge",
                "nudge_fine",
                "pitch",
                "velocity",
                "length",
                "length_fine",
                "probability",
                "nudge_shown",
                "nudge_shown_fine",
                "transpose",
                "quantize",
                "quantize_half",
                "clear",
                "clear_all",
                "velocity_to",
                "probability_to",
                "select_all",
                "select_none",
                "select_sound",
                "select_pitch",
                "erase_pitch",
                "select_step",
            ];
            let Some(&what) = WHAT.iter().find(|w| **w == what) else {
                return Err(mlua::Error::runtime(format!("no events command \"{what}\" ({})", WHAT.join(", "))));
            };
            let n = n.unwrap_or(0);
            let n = match what {
                // (Sounds count from 1, as the pages count.)
                "select_sound" => index(usize::try_from(n).unwrap_or(0), "a sound")? as i64,
                "transpose" | "select_pitch" | "erase_pitch" | "clear_all" | "velocity_to" | "probability_to" => n,
                _ => n.signum(),
            };
            act(&sh, Action::Events(what, n));
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "step",
        lua.create_function(move |_, (at, length, velocity): (i64, i64, i64)| {
            act(&sh, Action::StepToggle { at: at.max(0), length: length.max(1), velocity: velocity.clamp(0, 127) });
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "step_mod_at",
        lua.create_function(move |_, (at, length): (Option<i64>, Option<i64>)| {
            act(&sh, Action::StepModAt(at.map(|at| (at.max(0), length.unwrap_or(1).max(1)))));
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "step_mod",
        lua.create_function(move |_, (from, length, held, knob, inc): (i64, i64, Vec<usize>, usize, f64)| {
            let pads = held.iter().filter(|p| (1..=16).contains(*p)).fold(0u16, |m, p| m | 1 << (p - 1));
            if !(1..=8).contains(&knob) {
                return Err(mlua::Error::runtime(format!("no knob {knob} (1-8)")));
            }
            act(&sh, Action::StepMod { from, length: length.max(1), pads, knob: knob - 1, inc });
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "select_events",
        lua.create_function(move |_, (start, end, low, high): (i64, i64, Option<i64>, Option<i64>)| {
            act(&sh, Action::EventsRange { start, end, low: low.unwrap_or(0).clamp(0, 127), high: high.unwrap_or(127).clamp(0, 127) });
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "lfo",
        lua.create_function(move |_, (what, pad, value): (String, usize, Option<f64>)| {
            const WHAT: [&str; 19] = [
                "focus",
                "create",
                "remove",
                "mute",
                "shape",
                "sync",
                "trigger_focused",
                "rate",
                "phase",
                "level",
                "quantize",
                "trigger",
                "fade",
                "env",
                "retrigger",
                "attack",
                "decay",
                "sustain",
                "release",
            ];
            let Some(what) = WHAT.iter().find(|w| **w == what) else {
                return Err(mlua::Error::runtime(format!("lfo(what, pad, value): what is one of {}", WHAT.join(", "))));
            };
            if !(1..=16).contains(&pad) {
                return Err(mlua::Error::runtime("lfo pads count from 1 to 16"));
            }
            act(&sh, Action::Lfo(what, pad, value.unwrap_or(0.0)));
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "mod_slot",
        lua.create_function(move |_, (slot, row, steps): (usize, usize, i64)| {
            if slot < 1 || !(1..=6).contains(&row) {
                return Err(mlua::Error::runtime("mod_slot(slot, row, steps): slots count from 1, rows from 1 to 6"));
            }
            act(&sh, Action::ModSlot(slot - 1, row - 1, steps));
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "mod_slots_closed",
        lua.create_function(move |_, ()| {
            act(&sh, Action::ModSlotsClosed);
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "set_macro",
        lua.create_function(move |_, (knob, level): (usize, usize)| {
            if knob > 8 || level > 2 {
                return Err(mlua::Error::runtime("set_macro(knob 1-8, level 0 master, 1 group, 2 sound)"));
            }
            act(&sh, Action::SetMacro { knob: index(knob, "a knob")?, level });
            Ok(())
        })?,
    )?;
    let sh = shared.clone();
    d.set(
        "turn_knob",
        lua.create_function(move |_, (knob, amount, fine): (usize, f64, Option<bool>)| {
            if knob > 8 {
                return Err(mlua::Error::runtime("there are 8 knobs"));
            }
            act(&sh, Action::Turn { knob: index(knob, "a knob")?, inc: amount, fine: fine.unwrap_or(false) });
            Ok(())
        })?,
    )?;

    lua.globals().set("deckard", d)
}

/// The loudest a channel's level goes (dB).
const MAX_DB: f64 = 10.0;

/// An amplitude in dB (−∞ for silence).
fn db(amplitude: f32) -> f64 {
    if amplitude > 0.0 { 20.0 * f64::from(amplitude).log10() } else { f64::NEG_INFINITY }
}

/// A channel's Mixer fields on its table.
fn mix(lua: &Lua, t: &Table, m: &crate::MixView) -> mlua::Result<()> {
    t.set("level", m.level)?;
    t.set("pan", m.pan)?;
    t.set("meter", lua.create_sequence_from([db(m.meter.0), db(m.meter.1)])?)?;
    t.set("headroom", db(m.headroom))?;
    let plugins = lua.create_table()?;
    for (i, &(instrument, active)) in m.plugins.iter().enumerate() {
        let p = lua.create_table()?;
        p.set("instrument", instrument)?;
        p.set("active", active)?;
        plugins.set(i + 1, p)?;
    }
    t.set("plugins", plugins)
}

/// The app as the scripts see it (docs/controller-api.md, `deckard.state()`).
/// The library as BROWSE shows it (counts from 1).
fn browser(lua: &Lua, b: &crate::BrowserView) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("kind", b.kind + 1)?;
    t.set("user", b.user)?;
    t.set("products", lua.create_sequence_from(b.products.iter().map(String::as_str))?)?;
    t.set("product", b.product.map(|p| p + 1))?;
    let attributes = lua.create_table()?;
    for (i, (values, chosen)) in b.attributes.iter().enumerate() {
        let a = lua.create_table()?;
        a.set("values", lua.create_sequence_from(values.iter().map(String::as_str))?)?;
        a.set("chosen", chosen.map(|c| c + 1))?;
        attributes.set(i + 1, a)?;
    }
    t.set("attributes", attributes)?;
    let results = lua.create_table()?;
    for (i, (name, favorite)) in b.results.iter().enumerate() {
        let r = lua.create_table()?;
        r.set("name", name.as_str())?;
        r.set("favorite", *favorite)?;
        results.set(i + 1, r)?;
    }
    t.set("results", results)?;
    t.set("focus", b.focus + 1)?;
    t.set("busy", b.busy)?;
    t.set("favorites", b.favorites)?;
    Ok(t)
}

/// `deckard.scales`: Keyboard and Chords mode's lists, as Maschine has them
/// (`deckard_model::scale_tables`): the banks; each bank's scales by full and short
/// name, and the semitones above the root each one plays; the chord modes, the chord
/// types for each, and the chord positions.
fn scales(lua: &Lua) -> mlua::Result<Table> {
    use deckard_model::scale_tables as st;
    let t = lua.create_table()?;
    t.set("banks", lua.create_sequence_from(st::BANKS)?)?;
    let (names, short, steps) = (lua.create_table()?, lua.create_table()?, lua.create_table()?);
    for b in 0..st::BANKS.len() {
        names.set(b + 1, lua.create_sequence_from(st::SCALES[b])?)?;
        short.set(b + 1, lua.create_sequence_from(st::SCALES_SHORT[b])?)?;
        let bank = lua.create_table()?;
        for k in 0..15 {
            let snap = &st::SNAP[b * 15 + k];
            bank.set(k + 1, lua.create_sequence_from((0..12).filter(|&i| snap[i] == 0))?)?;
        }
        steps.set(b + 1, bank)?;
    }
    t.set("names", names)?;
    t.set("short", short)?;
    t.set("steps", steps)?;
    t.set("chord_modes", lua.create_sequence_from(st::CHORD_SCREEN_MODES)?)?;
    t.set("chord_modes_short", lua.create_sequence_from(st::CHORD_SCREEN_MODES_SHORT)?)?;
    t.set("chords_chromatic", lua.create_sequence_from(st::CHORDS_CHROMATIC)?)?;
    t.set("chords_chromatic_short", lua.create_sequence_from(st::CHORDS_CHROMATIC_SHORT)?)?;
    t.set("chords_scale", lua.create_sequence_from(st::CHORDS_SCALE)?)?;
    t.set("chord_sets", lua.create_sequence_from(st::CHORD_SETS)?)?;
    t.set("chord_sets_short", lua.create_sequence_from(st::CHORD_SETS_SHORT)?)?;
    t.set("user_chord_sets", lua.create_sequence_from(st::USER_CHORD_SETS)?)?;
    t.set("positions", lua.create_sequence_from(st::CHORD_POSITIONS)?)?;
    Ok(t)
}

fn state(lua: &Lua, v: &View) -> mlua::Result<Table> {
    let rgb = |c: [u8; 3]| lua.create_sequence_from(c);
    let t = lua.create_table()?;
    t.set("project", v.project.as_str())?;
    t.set("scene_name", v.scene_name.as_str())?;
    t.set("scene_palette", v.scene_color)?;
    let scenes = lua.create_table()?;
    for (i, sc) in v.scenes.iter().enumerate() {
        let st = lua.create_table()?;
        st.set("name", sc.name.as_str())?;
        st.set("palette", sc.color)?;
        st.set("color", rgb(sc.rgb)?)?;
        st.set("default_color", sc.default_color)?;
        // (By group: the pattern, or `false`.)
        let patterns = lua.create_table()?;
        for (g, p) in sc.patterns.iter().enumerate() {
            patterns.set(g + 1, p.map_or(Value::Boolean(false), |p| Value::Integer(p as i64 + 1)))?;
        }
        st.set("patterns", patterns)?;
        scenes.set(i + 1, st)?;
    }
    t.set("scenes", scenes)?;
    let lfos = lua.create_table()?;
    for (i, l) in v.lfos.iter().enumerate() {
        let lt = lua.create_table()?;
        lt.set("pad", l.pad)?;
        lt.set("shape", l.shape.as_str())?;
        lt.set("sync", l.sync)?;
        lt.set("rate", l.rate.as_str())?;
        lt.set("phase", l.phase)?;
        lt.set("level", l.level)?;
        lt.set("quantize", l.quantize)?;
        lt.set("trigger", l.trigger.as_str())?;
        lt.set("fade", l.fade)?;
        lt.set("muted", l.muted)?;
        lt.set("env", l.env)?;
        lt.set("attack", l.attack.as_str())?;
        lt.set("decay", l.decay.as_str())?;
        lt.set("release", l.release.as_str())?;
        lt.set("sustain", l.sustain)?;
        lt.set("retrigger", l.retrigger)?;
        lt.set("name", l.name.as_str())?;
        lt.set("points", lua.create_sequence_from(l.points.iter().copied())?)?;
        lt.set("note_off", l.note_off)?;
        lfos.set(i + 1, lt)?;
    }
    t.set("lfos", lfos)?;
    t.set("lfo_focus", v.lfo_focus)?;
    let slots = lua.create_table()?;
    for (i, m) in v.mod_slots.iter().enumerate() {
        let rows = lua.create_sequence_from(m.rows.iter().map(String::as_str))?;
        rows.set("midi", m.midi)?;
        slots.set(i + 1, rows)?;
    }
    t.set("mod_slots", slots)?;
    t.set("scene", v.scene + 1)?;
    t.set("playing_scene", v.playing_scene.map(|s| s + 1))?;
    t.set("queued_scene", v.queued_scene.map(|s| s + 1))?;
    t.set("song_view", v.song_view)?;
    let sections = lua.create_table()?;
    for (i, sec) in v.sections.iter().enumerate() {
        let st = lua.create_table()?;
        st.set("scene", sec.scene.map(|s| s + 1))?;
        st.set("start", sec.start)?;
        st.set("length", sec.length)?;
        st.set("auto", sec.auto)?;
        sections.set(i + 1, st)?;
    }
    t.set("sections", sections)?;
    t.set("section", v.section.map(|s| s + 1))?;
    t.set("pattern", v.pattern + 1)?;
    t.set("pattern_start", v.pattern_start)?;
    t.set("pattern_length", v.pattern_length)?;
    t.set("pattern_auto", v.pattern_auto)?;
    t.set("pattern_position", v.pattern_position)?;
    let notes = lua.create_table()?;
    for (i, n) in v.notes.iter().enumerate() {
        let nt = lua.create_table()?;
        nt.set("sound", n.sound + 1)?;
        nt.set("position", n.position)?;
        nt.set("length", n.length)?;
        nt.set("pitch", n.pitch)?;
        nt.set("velocity", n.velocity)?;
        nt.set("probability", n.probability)?;
        nt.set("selected", n.selected)?;
        notes.set(i + 1, nt)?;
    }
    t.set("notes", notes)?;
    t.set("browser", browser(lua, &v.browser)?)?;
    t.set("tempo", v.tempo)?;
    t.set("swing", v.swing)?;
    t.set("volume", v.master_volume)?;
    t.set("volume_db", v.master_db)?;
    t.set("playing", v.playing)?;
    t.set("recording", v.recording)?;
    t.set("looping", v.looping)?;
    t.set("metronome", v.metronome)?;
    t.set("auto_write", v.auto_write)?;
    t.set("strip_position", v.strip_position)?;
    t.set("touch_auto_write", v.touch_auto_write)?;
    t.set("fixed_velocity", v.fixed_velocity)?;
    t.set("velocity", v.velocity)?;
    let scale = lua.create_table()?;
    scale.set("bank", v.scale_bank + 1)?;
    scale.set("kind", v.scale_kind + 1)?;
    t.set("scale", scale)?;
    t.set("follow", v.follow)?;
    t.set("recent", lua.create_sequence_from(v.recent.iter().map(String::as_str))?)?;
    let settings = lua.create_table()?;
    settings.set("values", lua.create_table_from(v.settings.values.iter().map(|(k, v)| (k.as_str(), v.as_str())))?)?;
    for (key, ports) in [("midi_inputs", &v.settings.midi_inputs), ("midi_outputs", &v.settings.midi_outputs)] {
        let list = lua.create_table()?;
        for (name, on) in ports {
            let p = lua.create_table()?;
            p.set("name", name.as_str())?;
            p.set("on", *on)?;
            list.push(p)?;
        }
        settings.set(key, list)?;
    }
    settings.set("audio_status", lua.create_sequence_from(v.settings.audio_status.iter().map(String::as_str))?)?;
    settings.set("link", v.settings.link)?;
    settings.set("link_peers", v.settings.link_peers)?;
    t.set("settings", settings)?;
    let lock = lua.create_table()?;
    lock.set("active", v.lock.active)?;
    // (Slots and the bank from 1, as the pages count.)
    lock.set("slots", lua.create_table_from(v.lock.slots.iter().map(|s| (s + 1, true)))?)?;
    lock.set("focus", v.lock.focus.map(|f| f + 1))?;
    lock.set("bank", v.lock.bank + 1)?;
    lock.set("banks", v.lock.banks)?;
    lock.set("morph", v.lock.morph)?;
    lock.set("mode", v.lock.mode)?;
    lock.set("time", v.lock.time)?;
    lock.set("grid", v.lock.grid)?;
    t.set("lock", lock)?;
    t.set("sampling", sampling(lua, &v.sampling)?)?;
    let nr = lua.create_table()?;
    let settings = lua.create_table()?;
    for k in &v.note_repeat.settings {
        let st = lua.create_table()?;
        st.set("name", k.name.as_str())?;
        st.set("text", k.text.as_str())?;
        st.set("value", k.value)?;
        st.set("min", k.min)?;
        st.set("max", k.max)?;
        st.set("step", k.step)?;
        st.set("turn", k.turn)?;
        settings.set(k.key.as_str(), st)?;
    }
    nr.set("settings", settings)?;
    nr.set("presets", lua.create_sequence_from(v.note_repeat.presets.iter().map(String::as_str))?)?;
    // (The preset from 1, as the display buttons count.)
    nr.set("preset", v.note_repeat.preset + 1)?;
    nr.set("hold", v.note_repeat.hold)?;
    nr.set("locked", v.note_repeat.locked)?;
    nr.set("active", v.note_repeat.active)?;
    t.set("note_repeat", nr)?;
    let grid = lua.create_table()?;
    grid.set("perform", v.grid.perform)?;
    grid.set("arrange", v.grid.arrange)?;
    grid.set("arrange_snap", v.grid.arrange_snap)?;
    grid.set("quick", v.grid.quick)?;
    grid.set("step", v.grid.step)?;
    grid.set("step_snap", v.grid.step_snap)?;
    grid.set("step_ticks", v.grid.step_ticks)?;
    grid.set("nudge", v.grid.nudge)?;
    t.set("grid", grid)?;
    t.set("position", v.position)?;
    t.set("progress", v.progress)?;
    t.set("numerator", v.numerator)?;
    t.set("denominator", v.denominator)?;
    t.set("group", v.group + 1)?;
    t.set("sound", v.sound + 1)?;
    let groups = lua.create_table()?;
    for (i, g) in v.groups.iter().enumerate() {
        let gt = lua.create_table()?;
        gt.set("name", g.name.as_str())?;
        gt.set("color", rgb(g.rgb)?)?;
        gt.set("palette", g.color)?;
        gt.set("muted", g.muted)?;
        gt.set("default_color", g.default_color)?;
        gt.set("playing", g.playing)?;
        if let Some((touched, pos)) = g.perform {
            let pt = lua.create_table()?;
            pt.set("touched", touched)?;
            pt.set("pos", pos)?;
            gt.set("perform", pt)?;
        }
        mix(lua, &gt, &g.mix)?;
        let patterns = lua.create_table()?;
        for (i, pt) in g.patterns.iter().enumerate() {
            let t = lua.create_table()?;
            t.set("name", pt.name.as_str())?;
            t.set("palette", pt.color)?;
            t.set("color", rgb(pt.rgb)?)?;
            t.set("default_color", pt.default_color)?;
            t.set("bars", pt.bars)?;
            patterns.set(i + 1, t)?;
        }
        gt.set("patterns", patterns)?;
        gt.set("scene_pattern", g.scene_pattern.map(|i| i + 1))?;
        gt.set("progress", g.progress)?;
        let sounds = lua.create_table()?;
        for (j, s) in g.sounds.iter().enumerate() {
            let st = lua.create_table()?;
            st.set("name", s.name.as_str())?;
            st.set("color", rgb(s.rgb)?)?;
            st.set("palette", s.color)?;
            st.set("muted", s.muted)?;
            st.set("used", s.used)?;
            st.set("playing", s.playing)?;
            st.set("choke_group", s.choke_group)?;
            st.set("choke_mode", s.choke_mode)?;
            st.set("link_group", s.link_group)?;
            st.set("link_mode", s.link_mode)?;
            st.set("base_key", s.base_key)?;
            st.set("default_color", s.default_color)?;
            st.set("prob_locked", s.prob_locked)?;
            let touch = lua.create_table()?;
            touch.set("mode", s.touch_mode)?;
            touch.set("cc", s.touch_cc)?;
            touch.set("curve", s.touch_curve)?;
            touch.set("min", s.touch_min)?;
            touch.set("max", s.touch_max)?;
            st.set("touch", touch)?;
            mix(lua, &st, &s.mix)?;
            sounds.set(j + 1, st)?;
        }
        gt.set("sounds", sounds)?;
        groups.set(i + 1, gt)?;
    }
    t.set("groups", groups)?;
    let c = &v.control;
    let ct = lua.create_table()?;
    ct.set("level", c.level + 1)?;
    ct.set("plugins", c.modules_visible)?;
    ct.set("page_group", c.page_group + 1)?;
    ct.set("page_groups", c.page_groups)?;
    ct.set("slot", c.slot.as_deref())?;
    ct.set("slots", c.slot_count)?;
    ct.set("slot_index", c.slot_index.map(|i| i + 1))?;
    let chain = lua.create_table()?;
    for (i, slot) in c.chain.iter().enumerate() {
        let st = lua.create_table()?;
        st.set("name", slot.name.as_str())?;
        st.set("instrument", slot.instrument)?;
        st.set("active", slot.active)?;
        st.set("plugin", slot.plugin)?;
        st.set("picture", slot.picture.as_deref())?;
        chain.set(i + 1, st)?;
    }
    ct.set("chain", chain)?;
    ct.set("pages", lua.create_sequence_from(c.pages.iter().map(String::as_str))?)?;
    ct.set("page", c.page + 1)?;
    let knobs = lua.create_table()?;
    for k in 0..8 {
        let kt = match c.knobs.get(k) {
            Some(Some(knob)) => {
                let kt = lua.create_table()?;
                kt.set("name", knob.name.as_str())?;
                kt.set("section", knob.section.as_str())?;
                kt.set("text", knob.text.as_str())?;
                kt.set("value", knob.value)?;
                kt.set("modulatable", knob.modulatable)?;
                kt.set("step_mod", knob.step_mod.as_deref())?;
                kt.set("auto_mod", knob.auto_mod.as_deref())?;
                kt.set("macro", knob.in_macro)?;
                Value::Table(kt)
            }
            _ => Value::Boolean(false),
        };
        knobs.set(k + 1, kt)?;
    }
    ct.set("knobs", knobs)?;
    let modules = lua.create_table()?;
    for (i, m) in c.modules.iter().enumerate() {
        let mt = lua.create_table()?;
        mt.set("name", m.name.as_str())?;
        mt.set("instrument", m.instrument)?;
        mt.set("format", m.format.as_str())?;
        mt.set("vendor", m.vendor.as_str())?;
        modules.set(i + 1, mt)?;
    }
    ct.set("modules", modules)?;
    t.set("control", ct)?;
    Ok(t)
}

/// The Sampling page's view (tabs, pages, slices and zones from 1; frames from 0).
fn sampling(lua: &Lua, v: &crate::SamplingView) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("tab", v.tab + 1)?;
    let knob = |k: &crate::SamplingKnob| -> mlua::Result<Table> {
        let kt = lua.create_table()?;
        kt.set("key", k.key.as_str())?;
        kt.set("name", k.name.as_str())?;
        kt.set("text", k.text.as_str())?;
        kt.set("value", k.value)?;
        kt.set("min", k.min)?;
        kt.set("max", k.max)?;
        kt.set("step", k.step)?;
        kt.set("turn", k.turn)?;
        Ok(kt)
    };
    let tabs = lua.create_table()?;
    for (i, pages) in v.pages.iter().enumerate() {
        let pt = lua.create_table()?;
        for (j, p) in pages.iter().enumerate() {
            let page = lua.create_table()?;
            page.set("name", p.name.as_str())?;
            page.set("knobs", lua.create_sequence_from(p.knobs.iter().map(knob).collect::<mlua::Result<Vec<_>>>()?)?)?;
            pt.set(j + 1, page)?;
        }
        tabs.set(i + 1, pt)?;
    }
    t.set("pages", tabs)?;
    let slice = lua.create_table()?;
    for (i, k) in v.slice.iter().enumerate() {
        let kt = knob(k)?;
        kt.set("index", i + 1)?;
        slice.set(k.key.as_str(), kt)?;
    }
    t.set("slice", slice)?;
    if let Some(s) = &v.sample {
        let st = lua.create_table()?;
        st.set("name", s.name.as_str())?;
        st.set("frames", s.frames)?;
        st.set("rate", s.rate)?;
        st.set("view", lua.create_sequence_from([s.view.0, s.view.1])?)?;
        st.set("play", lua.create_sequence_from([s.play.0, s.play.1])?)?;
        st.set("looped", s.looped.map(|(a, b)| lua.create_sequence_from([a, b])).transpose()?)?;
        st.set("selection", s.selection.map(|(a, b)| lua.create_sequence_from([a, b])).transpose()?)?;
        st.set("slices", lua.create_sequence_from(s.slices.iter().copied())?)?;
        st.set("focus_slice", s.focus_slice.map(|i| i + 1))?;
        st.set("playhead", s.playhead)?;
        let zones = lua.create_table()?;
        for (i, ((kl, kh), (vl, vh))) in s.zones.iter().enumerate() {
            zones.set(i + 1, lua.create_sequence_from([*kl, *kh, *vl, *vh])?)?;
        }
        st.set("zones", zones)?;
        st.set("focus_zone", s.focus_zone + 1)?;
        st.set("palette", s.color)?;
        t.set("sample", st)?;
    }
    let r = lua.create_table()?;
    r.set("levels", lua.create_sequence_from([v.record.levels.0, v.record.levels.1])?)?;
    r.set("mode", v.record.mode)?;
    r.set("threshold", v.record.threshold)?;
    r.set("waiting", v.record.waiting)?;
    r.set("recording", v.record.recording)?;
    t.set("record", r)?;
    t.set("takes", v.takes)?;
    t.set("take", v.take.map(|i| i + 1))?;
    if let Some(s) = v.stems {
        let st = lua.create_table()?;
        st.set("focused", s.focused)?;
        st.set("progress", s.progress)?;
        t.set("stems", st)?;
    }
    Ok(t)
}
