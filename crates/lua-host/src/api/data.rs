//! `NI.DATA` and `App`: the project (song, groups, sounds, patterns, scenes), focus,
//! parameters and transport, backed by [`deckard_model::Project`].
//!
//! Any `getXParameter()` (or `...ParameterHW`/`...ParameterSW`) on a data object returns
//! a `Parameter` handle. Parameters that exist in the model (tempo, names, colors,
//! levels, mute, pattern length) and in the transport (playing, recording, loop,
//! metronome) read and write that state; the rest are kept in [`State::params`].

use super::{methods, object, object_ref, state};
use crate::State;
use crate::state::{ListKind, ObjRef, Playing, Prop};
use deckard_model::{Color, PPQ};
use mlua::{Function, Lua, MultiValue, Table, Value};

/// Types whose `getXParameter` methods produce parameters.
const DATA_TYPES: &[&str] =
    &["Song", "Group", "Sound", "EventPattern", "Scene", "Section", "Workspace", "Metronome", "ScaleEngine", "Recorder"];

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    for ty in DATA_TYPES {
        let t = methods(lua, impl_t, ty)?;
        let meta = lua.create_table()?;
        meta.set(
            "__index",
            lua.create_function(|lua, (_, key): (Table, String)| {
                let Some(name) = parameter_name(&key) else { return Ok(Value::Nil) };
                let name = name.to_string();
                let f = lua.create_function(move |lua, (this, _): (Value, MultiValue)| {
                    let owner_ref = object_ref(lua, &this);
                    let Some(owner_ref) = owner_ref else { return Ok(Value::Nil) };
                    let owner = state(lua).borrow_mut().objects.intern(owner_ref);
                    object(lua, ObjRef::Param { owner, name: name.clone() })
                })?;
                Ok(Value::Function(f))
            })?,
        )?;
        t.set_metatable(Some(meta))?;
    }

    // App
    let app = methods(lua, impl_t, "App")?;
    set_fn(lua, &app, "getWorkspace", |lua, _: MultiValue| object(lua, ObjRef::Workspace))?;
    set_fn(lua, &app, "getMetronome", |lua, _: MultiValue| object(lua, ObjRef::Metronome))?;
    set_fn(lua, &app, "getCPULevel", |_, _: MultiValue| Ok(Value::Number(0.0)))?;
    set_fn(lua, &app, "getStateCache", |lua, _: MultiValue| object(lua, ObjRef::StateCache))?;
    let ni_app = methods(lua, impl_t, "NI.APP")?;
    set_fn(lua, &ni_app, "isHeadless", |_, _: MultiValue| Ok(false))?;
    set_fn(lua, &ni_app, "isStandalone", |_, _: MultiValue| Ok(true))?;
    set_fn(lua, &ni_app, "isKompleteKontrol", |_, _: MultiValue| Ok(false))?;

    // Per-update caches the scripts poll. Notes aren't reported as sounding yet.
    let sc = methods(lua, impl_t, "StateCache")?;
    // isNoteOn(index, kind): kind 0 sound `index` of the focus group, 1 pitch `index` of
    // the focus sound, 2 pad `index` in 16 Velocities, 3 any sound of group `index`.
    set_fn(lua, &sc, "isNoteOn", |lua, (_this, index, kind): (Value, i64, i64)| {
        let st = state(lua);
        let st = st.borrow();
        let (g, s) = (st.focus.group, st.focus.sound);
        let i = index.max(0) as usize;
        Ok(st.notes_on.all().any(|((ng, ns, np), pad)| match kind {
            0 => ng == g && ns == i,
            1 => ng == g && ns == s && np == index,
            2 => pad == Some(i),
            3 => ng == i,
            _ => false,
        }))
    })?;
    // The level (master/group/sound) the mixer and Control area address.
    set_fn(lua, &sc, "isMixingLayerChanged", |lua, _: MultiValue| {
        let st = state(lua);
        let st = st.borrow();
        Ok(st.level_seen != Some(st.control.level))
    })?;
    // What the focus was at the last controller timer: the pages redraw what changed.
    for name in ["isFocusSceneChanged", "isFocusGroupChanged", "isFocusSoundChanged", "isFocusPatternChanged"] {
        set_fn(lua, &sc, name, move |lua, _: MultiValue| {
            let st = state(lua);
            let st = st.borrow();
            let (now, seen) = (st.focus, st.focus_seen);
            Ok(seen.is_none_or(|f| match name {
                "isFocusSceneChanged" => f.scene != now.scene,
                "isFocusGroupChanged" => f.group != now.group,
                "isFocusSoundChanged" => (f.group, f.sound) != (now.group, now.sound),
                _ => (f.group, f.pattern) != (now.group, now.pattern),
            }))
        })?;
    }
    set_fn(lua, &sc, "getObjectCache", |lua, _: MultiValue| object(lua, ObjRef::ObjectCache))?;
    let oc = methods(lua, impl_t, "ObjectCache")?;
    // A sound is "enabled" (lit on its pad) when it has something loaded.
    set_fn(lua, &oc, "isSoundEnabled", |lua, (_, sound): (Value, Value)| {
        let Some(ObjRef::Sound(g, s)) = object_ref(lua, &sound) else { return Ok(false) };
        let st = state(lua);
        let st = st.borrow();
        let enabled = st
            .project()
            .as_deref()
            .and_then(|p| p.groups.get(g)?.sounds.get(s))
            .is_some_and(|s| !s.instrument.is_empty() || !s.name.is_empty());
        Ok(enabled)
    })?;

    // StateHelper: focus.
    let sh = methods(lua, impl_t, "NI.DATA.StateHelper")?;
    set_fn(lua, &sh, "getFocusSong", |lua, _: MultiValue| {
        let has = state(lua).borrow().doc.is_some();
        if has { object(lua, ObjRef::Song) } else { Ok(Value::Nil) }
    })?;
    set_fn(lua, &sh, "getFocusGroup", |lua, _: MultiValue| {
        let g = focus_group(&state(lua).borrow());
        g.map_or(Ok(Value::Nil), |g| object(lua, ObjRef::Group(g)))
    })?;
    set_fn(lua, &sh, "getFocusGroupIndex", |lua, _: MultiValue| {
        Ok(focus_group(&state(lua).borrow()).map_or(Value::Nil, |g| Value::Integer(g as i64)))
    })?;
    set_fn(lua, &sh, "getFocusSound", |lua, _: MultiValue| {
        let s = focus_sound(&state(lua).borrow());
        s.map_or(Ok(Value::Nil), |(g, s)| object(lua, ObjRef::Sound(g, s)))
    })?;
    set_fn(lua, &sh, "getFocusSoundIndex", |lua, _: MultiValue| {
        Ok(focus_sound(&state(lua).borrow()).map_or(Value::Nil, |(_, s)| Value::Integer(s as i64)))
    })?;
    set_fn(lua, &sh, "getFocusEventPattern", |lua, _: MultiValue| {
        let p = focus_pattern(&state(lua).borrow());
        p.map_or(Ok(Value::Nil), |(g, p)| object(lua, ObjRef::Pattern(g, p)))
    })?;
    set_fn(lua, &sh, "getFocusSection", |lua, _: MultiValue| {
        let st = state(lua);
        let n = list_len(&st.borrow(), ListKind::Sections);
        let sec = st.borrow().focus.section.filter(|&s| s < n);
        sec.map_or(Ok(Value::Nil), |s| object(lua, ObjRef::Section(s)))
    })?;
    // Recorded takes: none yet (the hardware's recording isn't connected to the app's).
    let sound_algos = methods(lua, impl_t, "NI.DATA.SoundAlgorithms")?;
    set_fn(lua, &sound_algos, "getTakes", |_, _: MultiValue| Ok(Value::Nil))?;
    // The focused sound's Sampler / Audio module: its first module, when it is one.
    for (name, kind) in [("getFocusSampler", "Sampler"), ("getFocusAudioModule", "AudioModule")] {
        set_fn(lua, &sh, name, move |lua, _: MultiValue| {
            let is = {
                let st = state(lua);
                let st = st.borrow();
                focus_sound(&st).is_some_and(|(g, s)| st.project().as_deref().is_some_and(|p| p.groups[g].sounds[s].instrument == kind))
            };
            if is { object(lua, ObjRef::Module(0)) } else { Ok(Value::Nil) }
        })?;
    }
    set_fn(lua, &sh, "getFocusScene", |lua, _: MultiValue| {
        let st = state(lua);
        let sc = st.borrow().focus.scene.filter(|&s| s < st.borrow().project().as_deref().map_or(0, |p| p.scenes.len()));
        sc.map_or(Ok(Value::Nil), |s| object(lua, ObjRef::Scene(s)))
    })?;
    set_fn(lua, &sh, "getFocusPatternLength", |lua, _: MultiValue| {
        let st = state(lua);
        let st = st.borrow();
        Ok(focus_pattern(&st).map_or(0, |(g, p)| st.project().as_deref().unwrap().groups[g].patterns[p].length))
    })?;

    // Focus changes.
    let song_access = methods(lua, impl_t, "NI.DATA.SongAccess")?;
    set_fn(lua, &song_access, "setFocusGroup", |lua, (_app, _song, group): (Value, Value, Value)| {
        if let Some(ObjRef::Group(g)) = object_ref(lua, &group) {
            let st = state(lua);
            let mut st = st.borrow_mut();
            st.focus.group = g;
            st.focus.sound = 0;
            // The group's pattern in the focused scene, as Maschine focuses it.
            let scene = st.focus.scene;
            let pattern = st.project().and_then(|p| {
                let in_scene = scene.and_then(|s| p.scenes.get(s)?.patterns.iter().find(|(pg, _)| *pg == g).map(|&(_, pi)| pi));
                in_scene.or_else(|| (!p.groups[g].patterns.is_empty()).then_some(0))
            });
            st.focus.pattern = pattern;
        }
        Ok(())
    })?;
    // The Pattern page's knobs (EventPatternAccess::increment*): steps of the arrange
    // grid (a bar), or with SHIFT the pattern editor's (a sixteenth), snapping to them.
    // POSITION moves the pattern over its notes (start and end); START moves its start
    // with the end held; LENGTH moves its end.
    let pattern_access = methods(lua, impl_t, "NI.DATA.EventPatternAccess")?;
    for (name, kind) in [("incrementPosition", 0), ("incrementStart", 1), ("incrementExplicitLength", 2)] {
        set_fn(lua, &pattern_access, name, move |lua, args: MultiValue| {
            let Some((g, p)) = find_arg(lua, &args, |r| if let ObjRef::Pattern(g, p) = r { Some((g, p)) } else { None }) else {
                return Ok(());
            };
            // (App, Pattern, Increment, Fine[, Quick])
            let up = match args.get(2) {
                Some(Value::Integer(i)) => *i > 0,
                Some(Value::Number(n)) => *n > 0.0,
                _ => false,
            };
            let fine = matches!(args.get(3), Some(Value::Boolean(true)));
            let span = state(lua).borrow().project().and_then(|pr| {
                let pat = pr.groups.get(g)?.patterns.get(p)?;
                Some((pat.start, if pat.length > 0 { pat.length } else { pr.bar_length() }, pr.bar_length()))
            });
            let Some((start, length, bar)) = span else { return Ok(()) };
            let step = if fine { PPQ / 4 } else { bar };
            let snap = |t: i64| if up { t.div_euclid(step) * step + step } else { -(-t).div_euclid(step) * step - step };
            let (start, length) = match kind {
                0 => (snap(start).max(0), length),
                1 => {
                    let s = snap(start).max(0);
                    if s >= start + length {
                        return Ok(());
                    }
                    (s, start + length - s)
                }
                _ => (start, snap(length).max(step)),
            };
            edit(lua, |d| {
                // LENGTH (Record Prepare's knob): a length chosen, no longer Auto.
                if kind == 2 {
                    d.set_length_auto(g, p, false);
                }
                d.set_pattern_span(g, p, start, length)
            });
            Ok(())
        })?;
    }
    // Color names by palette index, then Auto, Use Group, White and Default
    // (NI.DATA.COLOR_DEFAULT, 19): Maschine's `DefaultColorInfo` table.
    const COLOR_NAMES: [&str; 20] = [
        "Red",
        "Orange",
        "Light Orange",
        "Warm Yellow",
        "Yellow",
        "Lime",
        "Green",
        "Mint",
        "Turquoise",
        "Cyan",
        "Blue",
        "Plum",
        "Violet",
        "Purple",
        "Magenta",
        "Fuchsia",
        "Auto",
        "Use Group",
        "White",
        "Default",
    ];
    let color_info = methods(lua, impl_t, "NI.DATA.DefaultColorInfo")?;
    set_fn(lua, &color_info, "getColorName", |_, args: MultiValue| {
        let i = args.iter().find_map(|v| v.as_i64()).unwrap_or(-1);
        Ok(usize::try_from(i).ok().and_then(|i| COLOR_NAMES.get(i)).copied().unwrap_or(""))
    })?;
    // The COLOR knobs: (App, Object, Color, UseDefaultColor) for a pattern, scene, group
    // or sound; setSelectedSoundsColor's object is the group, and colors its focused
    // sound (the only selection here).
    for (module, name) in [
        ("NI.DATA.EventPatternAccess", "setColor"),
        ("NI.DATA.SceneAccess", "setSceneColor"),
        ("NI.DATA.GroupAccess", "setGroupColor"),
        ("NI.DATA.SoundAccess", "setSoundColor"),
        ("NI.DATA.GroupAccess", "setSelectedSoundsColor"),
    ] {
        let selected_sounds = name == "setSelectedSoundsColor";
        let access = methods(lua, impl_t, module)?;
        set_fn(lua, &access, name, move |lua, args: MultiValue| {
            use deckard_doc::Target;
            let target = match args.get(1).and_then(|v| object_ref(lua, v)) {
                Some(ObjRef::Pattern(g, p)) => Target::Pattern(g, p),
                Some(ObjRef::Scene(s)) => Target::Scene(s),
                Some(ObjRef::Group(g)) if selected_sounds => Target::Sound(g, state(lua).borrow().focus.sound),
                Some(ObjRef::Group(g)) => Target::Group(g),
                Some(ObjRef::Sound(g, s)) => Target::Sound(g, s),
                _ => return Ok(()),
            };
            let color = match (args.get(2).and_then(Value::as_i64), args.get(3)) {
                (_, Some(Value::Boolean(true))) | (None, _) => deckard_model::Color::Default,
                (Some(c), _) => deckard_model::Color::Palette(c.clamp(0, 15) as u8),
            };
            edit(lua, |d| d.set_color(&[target], color));
            Ok(())
        })?;
    }
    // setPadVelocityMode(App, Song, Group, Mode): the FIXED VEL button.
    set_fn(lua, &song_access, "setPadVelocityMode", |lua, args: MultiValue| {
        let mode = match args.get(3) {
            Some(Value::Integer(i)) => *i,
            Some(Value::Number(n)) => *n as i64,
            _ => return Ok(()),
        };
        let owner = state(lua).borrow_mut().objects.intern(ObjRef::Song);
        let param = object(lua, ObjRef::Param { owner, name: "PadVelocityMode".into() })?;
        write_param(lua, &param, Prop::Int(mode));
        Ok(())
    })?;
    let group_access = methods(lua, impl_t, "NI.DATA.GroupAccess")?;
    // Keyboard mode (pads play notes of the focused sound) for the focused group.
    set_fn(lua, &group_access, "setKeyboardMode", |lua, (_app, on): (Value, bool)| {
        let Some(g) = focus_group(&state(lua).borrow()) else { return Ok(()) };
        let owner = state(lua).borrow_mut().objects.intern(ObjRef::Group(g));
        let param = object(lua, ObjRef::Param { owner, name: "KeyboardModeEnabled".into() })?;
        write_param(lua, &param, Prop::Bool(on));
        Ok(())
    })?;
    // transposeRootNote(App, Group, Inc, _): Keyboard mode's OCT and SEMI buttons move
    // the group's root note, and so the pads' notes. transposeNoteView (chord edit) the same.
    for name in ["transposeRootNote", "transposeNoteView"] {
        set_fn(lua, &group_access, name, |lua, args: MultiValue| {
            let g = match args.get(1).and_then(|v| object_ref(lua, v)) {
                Some(ObjRef::Group(g)) => g,
                _ => match focus_group(&state(lua).borrow()) {
                    Some(g) => g,
                    None => return Ok(()),
                },
            };
            let inc = match args.get(2) {
                Some(Value::Integer(i)) => *i,
                Some(Value::Number(n)) => *n as i64,
                _ => return Ok(()),
            };
            let owner = state(lua).borrow_mut().objects.intern(ObjRef::Group(g));
            let param = object(lua, ObjRef::Param { owner, name: "RootNote".into() })?;
            let root = match read_param(lua, &param) {
                Some(Prop::Int(n)) => n,
                Some(Prop::Num(n)) => n as i64,
                _ => 60,
            };
            write_param(lua, &param, Prop::Int((root + inc).clamp(0, 127)));
            Ok(())
        })?;
    }
    // bounceSelectedSounds(App, Group): PAD MODE's SHIFT + BOUNCE; the app bounces the
    // group's focused sound (its only selection here), after asking.
    set_fn(lua, &group_access, "bounceSelectedSounds", |lua, args: MultiValue| {
        let g = match args.get(1).and_then(|v| object_ref(lua, v)) {
            Some(ObjRef::Group(g)) => Some(g),
            _ => focus_group(&state(lua).borrow()),
        };
        let st = state(lua);
        let mut st = st.borrow_mut();
        if let Some(g) = g {
            st.bounce_request = Some((g, st.focus.sound));
        }
        Ok(())
    })?;
    // toggleKeyboardMode(App): the Step page's KEYBOARD.
    set_fn(lua, &group_access, "toggleKeyboardMode", |lua, _: MultiValue| {
        let Some(g) = focus_group(&state(lua).borrow()) else { return Ok(()) };
        let owner = state(lua).borrow_mut().objects.intern(ObjRef::Group(g));
        let param = object(lua, ObjRef::Param { owner, name: "KeyboardModeEnabled".into() })?;
        let on = matches!(read_param(lua, &param), Some(Prop::Bool(true)));
        write_param(lua, &param, Prop::Bool(!on));
        Ok(())
    })?;
    // Pattern mode: a pad with a pattern focuses it, an empty one makes a new pattern.
    // Either way the pattern goes into the focused scene, as Maschine does. Patterns
    // have no gaps here, so a new one goes after the group's others.
    set_fn(lua, &group_access, "createPattern", |lua, args: MultiValue| {
        let Some(g) = find_arg(lua, &args, |r| if let ObjRef::Group(g) = r { Some(g) } else { None }) else { return Ok(()) };
        let from = state(lua).borrow().focus.pattern.filter(|_| state(lua).borrow().focus.group == g).unwrap_or(0);
        let n = state(lua).borrow().project().map_or(0, |p| p.groups.get(g).map_or(0, |g| g.patterns.len()));
        if let Some(made) = edit(lua, |d| d.add_pattern(g, from, &format!("Pattern {}", n + 1))) {
            focus_and_assign_pattern(lua, g, made);
        }
        Ok(())
    })?;
    // insertPattern(App, Group, Pattern): a new empty pattern right after `Pattern` (after
    // the group's others without one), focused and in the scene; the Pattern page's CREATE.
    set_fn(lua, &group_access, "insertPattern", |lua, args: MultiValue| {
        let Some(g) = find_arg(lua, &args, |r| if let ObjRef::Group(g) = r { Some(g) } else { None }) else { return Ok(()) };
        let after = find_arg(lua, &args, |r| if let ObjRef::Pattern(pg, p) = r { (pg == g).then_some(p) } else { None });
        let n = state(lua).borrow().project().map_or(0, |p| p.groups.get(g).map_or(0, |g| g.patterns.len()));
        let at = after.map_or(n, |p| p + 1);
        if let Some(made) = edit(lua, |d| d.insert_pattern(g, after.unwrap_or(0), at, &format!("Pattern {}", n + 1))) {
            focus_and_assign_pattern(lua, g, made);
        }
        Ok(())
    })?;
    // shiftPatternFocus(App, Group, Next, Create): the group's next or previous pattern
    // (the first, from none), into the scene.
    set_fn(lua, &group_access, "shiftPatternFocus", |lua, (_app, group, next): (Value, Value, bool)| {
        let Some(ObjRef::Group(g)) = object_ref(lua, &group) else { return Ok(()) };
        let (n, at) = {
            let st = state(lua);
            let st = st.borrow();
            let n = st.project().map_or(0, |p| p.groups.get(g).map_or(0, |g| g.patterns.len()));
            (n, st.focus.pattern.filter(|_| st.focus.group == g))
        };
        let to = match at {
            Some(p) if next => Some(p + 1).filter(|&q| q < n),
            Some(p) => p.checked_sub(1),
            None => (n > 0 && next).then_some(0),
        };
        if let Some(p) = to {
            focus_and_assign_pattern(lua, g, p);
        }
        Ok(())
    })?;
    set_fn(lua, &group_access, "insertPatternAndFocus", |lua, args: MultiValue| {
        if let Some((g, p)) = find_arg(lua, &args, |r| if let ObjRef::Pattern(g, p) = r { Some((g, p)) } else { None }) {
            focus_and_assign_pattern(lua, g, p);
        }
        Ok(())
    })?;
    // setFocusSound(App, Group or Sounds, Sound), on GroupAccess and ObjectVectorAccess.
    let vector_access = methods(lua, impl_t, "NI.DATA.ObjectVectorAccess")?;
    for (table, name) in [(&group_access, "setFocusSound"), (&group_access, "setFocusSoundNonRepeating"), (&vector_access, "setFocusSound")]
    {
        set_fn(lua, table, name, |lua, (_app, _group, sound): (Value, Value, Value)| {
            if let Some(ObjRef::Sound(g, s)) = object_ref(lua, &sound) {
                let st = state(lua);
                let mut st = st.borrow_mut();
                st.focus.group = g;
                st.focus.sound = s;
            }
            Ok(())
        })?;
    }
    // removeGroupResetLast(App, Groups, Group): SHIFT + ERASE + a group button; the
    // group deleted, or reset if it's the last.
    set_fn(lua, &vector_access, "removeGroupResetLast", |lua, args: MultiValue| {
        let Some(g) = find_arg(lua, &args, |r| if let ObjRef::Group(g) = r { Some(g) } else { None }) else { return Ok(()) };
        if let Some(n) = edit(lua, |d| d.delete_group(g).map(|_| d.project().groups.len())) {
            let st = state(lua);
            let mut st = st.borrow_mut();
            let focus = st.focus.group;
            if focus > g {
                st.focus.group = focus - 1;
            } else if focus == g {
                (st.focus.group, st.focus.sound, st.focus.pattern) = (g.min(n.saturating_sub(1)), 0, None);
            }
        }
        Ok(())
    })?;

    // Transport.
    let tr = methods(lua, impl_t, "NI.DATA.TransportAccess")?;
    set_fn(lua, &tr, "canControlTransport", |_, _: MultiValue| Ok(true))?;
    set_fn(lua, &tr, "togglePlay", |lua, _: MultiValue| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        st.transport.playing = !st.transport.playing;
        if !st.transport.playing {
            st.transport.recording = false;
        }
        Ok(())
    })?;
    set_fn(lua, &tr, "restartTransport", |lua, _: MultiValue| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        st.transport.position = 0;
        st.transport.playing = true;
        st.transport.restarted = true;
        Ok(())
    })?;
    // startEventRecording(App, count_in, erase): REC (SHIFT: with a count-in). The song
    // is the app's to start: stopped, REC arms (PLAY starts the take); with the count-in
    // the app counts in and plays (docs/superpowers/plans/2026-10-23-auto-length-recording.md).
    set_fn(lua, &tr, "startEventRecording", |lua, (_, count_in): (Value, Option<bool>)| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        st.transport.recording = true;
        st.transport.count_in = count_in.unwrap_or(false);
        Ok(())
    })?;
    set_fn(lua, &tr, "stopEventRecording", |lua, _: MultiValue| {
        state(lua).borrow_mut().transport.recording = false;
        Ok(())
    })?;
    set_fn(lua, &tr, "toggleLoop", |lua, _: MultiValue| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        st.transport.looping = !st.transport.looping;
        Ok(())
    })?;
    set_fn(lua, &tr, "isLoopActive", |lua, _: MultiValue| Ok(state(lua).borrow().transport.looping))?;
    set_fn(lua, &tr, "getPlayPosition", |lua, _: MultiValue| Ok(state(lua).borrow().transport.position))?;

    // Parameter access: (App, Param, Value, ...).
    let pa = methods(lua, impl_t, "NI.DATA.ParameterAccess")?;
    for name in [
        "setBoolParameter",
        "setBoolParameterNoUndo",
        "setEnumParameter",
        "setEnumParameterNoUndo",
        "setSizeTParameter",
        "setSizeTParameterNoUndo",
        "setDoubleParameterNoUndo",
        "setFloatParameter",
        "setFloatParameterNoUndo",
        "setTickParameterNoUndo",
        "setWStringParameter",
        "setIntParameter",
        "setIntParameterNoUndo",
    ] {
        set_fn(lua, &pa, name, |lua, (_app, param, v): (Value, Value, Value)| {
            write_param(lua, &param, Prop::from_lua(&v));
            Ok(())
        })?;
    }
    for name in ["toggleBoolParameter", "toggleBoolParameterNoUndo"] {
        set_fn(lua, &pa, name, |lua, (_app, param): (Value, Value)| {
            let on = matches!(read_param(lua, &param), Some(Prop::Bool(true)));
            write_param(lua, &param, Prop::Bool(!on));
            Ok(())
        })?;
    }
    // addLevelPanEncoderDelta(App, Param, Inc, Fine) (a knob, `Inc` turns) and
    // addLevelPanWheelDelta(App, Param, Inc, Fine, Model) (the 4-D encoder, `Inc` clicks):
    // a channel's level (a turn over the meter's −60 … +10 dB, a click 0.5 dB) or pan
    // (a turn across, a click 2 %); finely with SHIFT, a tenth.
    for (name, wheel) in [("addLevelPanEncoderDelta", false), ("addLevelPanWheelDelta", true)] {
        set_fn(lua, &pa, name, move |lua, args: MultiValue| {
            let inc = match args.get(2) {
                Some(Value::Integer(i)) => Some(*i as f64),
                Some(Value::Number(n)) => Some(*n),
                _ => None,
            };
            let (Some(param), Some(inc)) = (args.get(1).cloned(), inc) else { return Ok(()) };
            let fine = matches!(args.get(3), Some(Value::Boolean(true)));
            let inc = if fine { inc * 0.1 } else { inc };
            let Some((_, _, name)) = param_parts(lua, &param) else { return Ok(()) };
            let Some(Prop::Num(v)) = read_param(lua, &param) else { return Ok(()) };
            let to = match name.as_str() {
                "Level" => level_step(v, inc, wheel),
                "Pan" => (v + if wheel { inc * 0.02 } else { inc * 2.0 }).clamp(-1.0, 1.0),
                _ => return Ok(()),
            };
            write_param(lua, &param, Prop::Num(to));
            Ok(())
        })?;
    }
    for (name, wheel) in [("addParameterEncoderDelta", false), ("addParameterWheelDelta", true)] {
        set_fn(lua, &pa, name, move |lua, (_app, param, inc, fine): (Value, Value, f64, Option<bool>)| {
            // A Control-area knob: the app turns it, on its own scale.
            if let Some(pos) = super::control::control_pos(lua, &param) {
                state(lua).borrow_mut().control_requests.push(crate::state::ControlRequest::Turn { pos, inc, fine: fine == Some(true) });
                return Ok(());
            }
            // A RECORD screen knob: a step every so far.
            if super::sampling::turn(lua, &param, inc, fine == Some(true)) {
                return Ok(());
            }
            // A channel's level (the 4-D encoder's VOLUME) and swing (SWING): a click 0.5 dB
            // or 1 %, a knob's turn across their range; a tenth with SHIFT.
            if let Some((_, owner, name)) = param_parts(lua, &param)
                && matches!(owner, ObjRef::Song | ObjRef::Group(_) | ObjRef::Sound(..))
                && let Some(Prop::Num(v)) = read_param(lua, &param)
            {
                let inc = if fine == Some(true) { inc * 0.1 } else { inc };
                let to = match name.as_str() {
                    "Level" => Some(level_step(v, inc, wheel)),
                    "SwingAmount" => Some((v + if wheel { inc } else { inc * 100.0 }).clamp(0.0, 100.0)),
                    _ => None,
                };
                if let Some(to) = to {
                    write_param(lua, &param, Prop::Num(to));
                    return Ok(());
                }
            }
            // Approximate: one step per wheel click, or the encoder delta scaled to the
            // value's magnitude. The app's real per-parameter ranges are not recovered yet.
            let new = match read_param(lua, &param) {
                Some(Prop::Int(i)) => Prop::Int(i + inc.signum() as i64),
                Some(Prop::Num(n)) => Prop::Num(n + inc * if fine == Some(true) { 0.1 } else { 1.0 }),
                Some(Prop::Bool(_)) if inc != 0.0 => Prop::Bool(inc > 0.0),
                other => other.unwrap_or(Prop::Nil),
            };
            write_param(lua, &param, new);
            Ok(())
        })?;
    }

    // Parameter objects.
    let p = methods(lua, impl_t, "Parameter")?;
    set_fn(lua, &p, "getValue", |lua, this: Value| match read_param(lua, &this) {
        Some(v) => v.into_lua(lua),
        None => Ok(Value::Nil),
    })?;
    set_fn(lua, &p, "setValue", |lua, (this, v): (Value, Value)| {
        write_param(lua, &this, Prop::from_lua(&v));
        Ok(())
    })?;
    set_fn(lua, &p, "getValueString", |lua, this: Value| {
        // A setting the app shows as it likes (Sampling, Note Repeat / Arp).
        if let Some(k) = super::sampling::knob(lua, &this) {
            return Ok(k.text);
        }
        let value = read_param(lua, &this);
        if let (Some((_, owner, name)), Some(v)) = (param_parts(lua, &this), &value)
            && let Some(s) = super::scales::value_string(&state(lua).borrow(), &owner, &name, v)
                .or_else(|| super::pad_mode::value_string(&owner, &name, v))
                .or_else(|| super::pad_touch::value_string(&owner, &name, v))
        {
            return Ok(s);
        }
        if let (Some((_, owner, name)), Some(Prop::Num(n))) = (param_parts(lua, &this), &value)
            && let Some(s) = mix_string(&owner, &name, *n)
        {
            return Ok(s);
        }
        Ok(match value {
            Some(Prop::Bool(b)) => if b { "On" } else { "Off" }.to_string(),
            Some(Prop::Int(i)) => i.to_string(),
            Some(Prop::Num(n)) => format!("{n:.2}"),
            Some(Prop::Str(s)) => s,
            _ => String::new(),
        })
    })?;
    set_fn(lua, &p, "getName", |lua, this: Value| {
        Ok(match object_ref(lua, &this) {
            Some(ObjRef::Param { name, .. }) => name,
            _ => String::new(),
        })
    })?;
    // Scale engine settings are enums: touching their knob lists their values.
    set_fn(lua, &p, "getTag", |lua, this: Value| {
        Ok(match param_parts(lua, &this) {
            Some((_, owner, name))
                if super::scales::is_enum(&owner, &name)
                    || super::pad_mode::is_enum(&owner, &name)
                    || super::pad_touch::is_enum(&owner, &name)
                    || super::arp::is_enum(&state(lua).borrow(), &owner, &name)
                    || super::pattern_editor::is_enum(&owner, &name)
                    || super::lock::is_enum(&owner, &name) =>
            {
                lua.load("return NI.DATA.MaschineParameter.TAG_ENUM").eval()?
            }
            _ => 0,
        })
    })?;
    // A color parameter left at the object's default color (a pattern's: its group's).
    set_fn(lua, &p, "isUseDefaultColor", |lua, this: Value| {
        use deckard_model::Color;
        let Some((_, owner, name)) = param_parts(lua, &this) else { return Ok(false) };
        let st = state(lua);
        let st = st.borrow();
        let Some(pr) = st.project() else { return Ok(false) };
        let color = match owner {
            _ if name != "Color" => None,
            ObjRef::Group(g) => pr.groups.get(g).map(|g| g.color),
            ObjRef::Sound(g, s) => pr.groups.get(g).and_then(|g| g.sounds.get(s)).map(|s| s.color),
            ObjRef::Pattern(g, p) => pr.groups.get(g).and_then(|g| g.patterns.get(p)).map(|p| p.color),
            ObjRef::Scene(s) => pr.scenes.get(s).map(|s| s.color),
            _ => None,
        };
        Ok(color == Some(Color::Default))
    })?;
    set_fn(lua, &p, "isParameterEqual", |lua, (this, other): (Value, Value)| {
        Ok(matches!((param_parts(lua, &this), param_parts(lua, &other)), (Some((a, ..)), Some((b, ..))) if a == b))
    })?;
    set_fn(lua, &p, "isChanged", |lua, this: Value| {
        let Some((pid, ..)) = param_parts(lua, &this) else { return Ok(false) };
        Ok(state(lua).borrow().changed_params.contains(&pid))
    })?;
    // getAsString(i): item i of an enum's list (its full name); else the value string.
    let value_string: Function = p.get("getValueString")?;
    // getAsString(value): a value of the parameter as shown (a list entry, a level, a
    // pan); else the current one's.
    set_fn(lua, &p, "getAsString", move |lua, (this, v): (Value, Value)| {
        let parts = param_parts(lua, &this);
        if let (Some(i), Some((_, owner, name))) =
            (v.as_i64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)), &parts)
            && let Some(s) = super::scales::list_string(&state(lua).borrow(), owner, name, i)
                .or_else(|| super::pad_mode::list_string(owner, name, i))
                .or_else(|| super::pad_touch::list_string(owner, name, i))
                .or_else(|| super::arp::list_string(&state(lua).borrow(), owner, name, i))
                .or_else(|| super::pattern_editor::list_string(owner, name, i))
                .or_else(|| super::lock::list_string(owner, name, i))
                .or_else(|| super::control::list_string(owner, name, i))
        {
            return Ok(s);
        }
        if let (Some(n), Some((_, owner, name))) = (v.as_f64(), &parts)
            && let Some(s) = mix_string(owner, name, n)
        {
            return Ok(s);
        }
        value_string.call::<String>(this)
    })?;

    // Object methods.
    let song = methods(lua, impl_t, "Song")?;
    set_fn(lua, &song, "getGroups", |lua, _: Value| object(lua, ObjRef::List(ListKind::Groups)))?;
    set_fn(lua, &song, "getScenes", |lua, _: Value| object(lua, ObjRef::List(ListKind::Scenes)))?;
    set_fn(lua, &song, "getSections", |lua, _: Value| object(lua, ObjRef::List(ListKind::Sections)))?;
    set_fn(lua, &song, "getTicksPerBeat", |_, _: Value| Ok(PPQ))?;
    // Which level the mixer/encoders address: NI.DATA.LEVEL_TAB_SONG/GROUP/SOUND.
    set_fn(lua, &song, "getLevelTab", |lua, _: Value| {
        let h: Table = lua.globals().get("NI")?;
        let data: Table = h.get("DATA")?;
        data.get::<Value>("LEVEL_TAB_GROUP")
    })?;
    for (name, duration) in [("getTickPositionToString", false), ("getTickToString", true)] {
        set_fn(lua, &song, name, move |lua, (_, tick): (Value, Option<i64>)| {
            let st = state(lua);
            let st = st.borrow();
            let tick = tick.unwrap_or(st.transport.position);
            let text = st.project().as_deref().map_or(String::new(), |p| bar_beat_tick(p, tick, duration));
            Ok(text)
        })?;
    }
    let section = methods(lua, impl_t, "Section")?;
    set_fn(lua, &section, "getScene", |lua, this: Value| {
        let Some(ObjRef::Section(i)) = object_ref(lua, &this) else { return Ok(Value::Nil) };
        let scene = state(lua).borrow().project().as_deref().and_then(|p| p.sections.get(i)?.scene);
        scene.map_or(Ok(Value::Nil), |s| object(lua, ObjRef::Scene(s)))
    })?;
    // Scene and arrangement edits: undoable edits of the shared document.
    let ideas = methods(lua, impl_t, "NI.DATA.IdeaSpaceAccess")?;
    set_fn(lua, &ideas, "createScene", |lua, _: MultiValue| {
        let at = edit(lua, |d| d.add_scene());
        focus_scene(lua, at);
        Ok(())
    })?;
    set_fn(lua, &ideas, "insertSceneAfterFocusScene", |lua, _: MultiValue| {
        let after = state(lua).borrow().focus.scene.map_or(usize::MAX, |s| s + 1);
        let at = edit(lua, |d| d.insert_scene(after));
        focus_scene(lua, at);
        Ok(())
    })?;
    set_fn(lua, &ideas, "duplicateScene", |lua, args: MultiValue| {
        let Some(scene) = find_arg(lua, &args, |r| if let ObjRef::Scene(s) = r { Some(s) } else { None }) else { return Ok(()) };
        let at = edit(lua, |d| d.duplicate_scene(scene));
        focus_scene(lua, at);
        Ok(())
    })?;
    set_fn(lua, &ideas, "removeScene", |lua, args: MultiValue| {
        let Some(scene) = find_arg(lua, &args, |r| if let ObjRef::Scene(s) = r { Some(s) } else { None }) else { return Ok(()) };
        edit(lua, |d| d.delete_scene(scene));
        let n = list_len(&state(lua).borrow(), ListKind::Scenes);
        let st = state(lua);
        let mut st = st.borrow_mut();
        st.focus.scene = (n > 0).then(|| scene.min(n - 1));
        Ok(())
    })?;
    set_fn(lua, &song_access, "createSectionWithScene", |lua, args: MultiValue| {
        let Some(scene) = find_arg(lua, &args, |r| if let ObjRef::Scene(s) = r { Some(s) } else { None }) else { return Ok(()) };
        let at = edit(lua, |d| d.add_section(scene));
        state(lua).borrow_mut().focus.section = at;
        Ok(())
    })?;
    set_fn(lua, &song_access, "removeSection", |lua, args: MultiValue| {
        let Some(section) = find_arg(lua, &args, |r| if let ObjRef::Section(s) = r { Some(s) } else { None }) else { return Ok(()) };
        edit(lua, |d| d.delete_section(section));
        Ok(())
    })?;
    // The arranger's view (Ideas or Song) and what changed since the last timer.
    set_fn(lua, &song, "getArrangerState", |lua, _: Value| object(lua, ObjRef::ArrangerState))?;
    let arranger = methods(lua, impl_t, "ArrangerState")?;
    set_fn(lua, &arranger, "isViewInIdeaSpace", |lua, _: Value| Ok(state(lua).borrow().ideas_view))?;
    set_fn(lua, &arranger, "isViewChanged", |lua, _: Value| {
        let st = state(lua);
        let st = st.borrow();
        Ok(st.arranger_seen.is_none_or(|(ideas, ..)| ideas != st.ideas_view))
    })?;
    set_fn(lua, &arranger, "isFocusChanged", |lua, _: Value| {
        let st = state(lua);
        let st = st.borrow();
        Ok(st.arranger_seen.is_none_or(|(_, scene, section)| (scene, section) != (st.focus.scene, st.focus.section)))
    })?;
    for (name, ideas) in [("focusIdeas", true), ("focusSongTimeline", false)] {
        set_fn(lua, &song_access, name, move |lua, _: MultiValue| {
            let st = state(lua);
            let mut st = st.borrow_mut();
            st.ideas_view = ideas;
            if !ideas {
                super::arranger::focus_a_section(&mut st);
            }
            Ok(())
        })?;
    }
    let scene_access = methods(lua, impl_t, "NI.DATA.SceneAccess")?;
    // clearScene(App, Song, Scene): no patterns left in it.
    set_fn(lua, &scene_access, "clearScene", |lua, args: MultiValue| {
        let Some(scene) = find_arg(lua, &args, |r| if let ObjRef::Scene(s) = r { Some(s) } else { None }) else { return Ok(()) };
        let groups: Vec<usize> = state(lua)
            .borrow()
            .project()
            .and_then(|p| Some(p.scenes.get(scene)?.patterns.iter().map(|&(g, _)| g).collect()))
            .unwrap_or_default();
        for g in groups {
            edit(lua, |d| d.assign_pattern(scene, g, None));
        }
        Ok(())
    })?;
    // removePattern(App, Scene, Group): the group plays nothing in the scene.
    set_fn(lua, &scene_access, "removePattern", |lua, args: MultiValue| {
        let scene = find_arg(lua, &args, |r| if let ObjRef::Scene(s) = r { Some(s) } else { None });
        let group = find_arg(lua, &args, |r| if let ObjRef::Group(g) = r { Some(g) } else { None });
        if let (Some(scene), Some(group)) = (scene, group) {
            edit(lua, |d| d.assign_pattern(scene, group, None));
        }
        Ok(())
    })?;
    set_fn(lua, &scene_access, "getPattern", |lua, args: MultiValue| {
        let scene = find_arg(lua, &args, |r| if let ObjRef::Scene(s) = r { Some(s) } else { None });
        let group = find_arg(lua, &args, |r| if let ObjRef::Group(g) = r { Some(g) } else { None });
        let (Some(scene), Some(group)) = (scene, group) else { return Ok(Value::Nil) };
        let pattern =
            state(lua).borrow().project().and_then(|p| p.scenes.get(scene)?.patterns.iter().find(|(g, _)| *g == group).map(|&(_, pi)| pi));
        pattern.map_or(Ok(Value::Nil), |pi| object(lua, ObjRef::Pattern(group, pi)))
    })?;
    let group_ns = methods(lua, impl_t, "NI.DATA.Group")?;
    set_fn(lua, &group_ns, "getLabel", |_, i: i64| Ok(group_label(i.max(0) as usize)))?;
    // Key switches (instruments that change articulation by key): none sounding, which
    // the scripts read as 128 (`InfoBarHelper.INVALID_NOTE`).
    let ni_data = methods(lua, impl_t, "NI.DATA")?;
    set_fn(lua, &ni_data, "getKeySwitchNote", |_, _: MultiValue| Ok(128))?;
    // The Sampling page's tab (RECORD, EDIT, SLICE, ZONE): scripts compare it with the
    // mode they show, so it must read back what was set.
    let workspace = methods(lua, impl_t, "NI.DATA.WORKSPACE")?;
    // Auto-write (modulation; AUTO): the app's switch, and AUTO's requests to it.
    for name in ["isAutoWriteEnabledFromHW", "isAutoWriteEnabledFromHWModel"] {
        set_fn(lua, &workspace, name, |lua, _: MultiValue| Ok(state(lua).borrow().auto_write))?;
    }
    set_fn(lua, &workspace, "setAutoWriteEnabled", |lua, (_app, _sw, on): (Value, Value, bool)| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        st.auto_write = on;
        st.auto_write_set = Some(on);
        Ok(())
    })?;
    set_fn(lua, &workspace, "getSamplingTab", |lua, _app: Value| Ok(state(lua).borrow().sampling_tab))?;
    set_fn(lua, &workspace, "setSamplingTab", |lua, (_app, tab): (Value, i64)| {
        state(lua).borrow_mut().sampling_tab = tab.clamp(0, 3);
        Ok(())
    })?;
    let algos = methods(lua, impl_t, "NI.DATA.SongAlgorithms")?;
    set_fn(lua, &algos, "isIdeaSpaceFocused", |lua, _song: Value| Ok(state(lua).borrow().ideas_view))?;
    // What plays: a scene or pattern of the Ideas view, or the arrangement. Stopped, the
    // Ideas view's scenes play next while it's shown.
    set_fn(lua, &algos, "isIdeaSpacePlaying", |lua, _song: Value| {
        let st = state(lua);
        let st = st.borrow();
        Ok(match st.transport.what {
            Some(w) => w != Playing::Arrangement,
            None => st.ideas_view,
        })
    })?;
    set_fn(lua, &song, "getIdeaSpacePlayingScene", |lua, _: Value| {
        let st = state(lua);
        let (what, focus, n) = {
            let st = st.borrow();
            (st.transport.what, st.focus.scene, st.project().as_deref().map_or(0, |p| p.scenes.len()))
        };
        let sc = match what {
            Some(Playing::Scene(s)) => Some(s),
            Some(_) => None,
            None => focus.or(Some(0)),
        };
        match sc.filter(|&s| s < n) {
            Some(s) => object(lua, ObjRef::Scene(s)),
            None => Ok(Value::Nil),
        }
    })?;
    // The touch strip's play position (TouchstripControllerMK3): where the scene playing,
    // or a group's pattern in it, is in its loop; the song's length for the arrangement.
    set_fn(lua, &algos, "scenePlayPositionInIdeaSpace", |lua, (_song, pos): (Value, i64)| {
        let st = state(lua);
        let st = st.borrow();
        Ok(match (st.transport.what, st.project()) {
            (Some(Playing::Scene(s)), Some(p)) if s < p.scenes.len() => pos.rem_euclid(p.scene_length(s).max(1)),
            _ => INVALID_TICK,
        })
    })?;
    set_fn(lua, &algos, "getLastSectionEnd", |lua, _song: Value| {
        Ok(state(lua).borrow().project().map_or(0, |p| p.sections.iter().map(|s| s.length).sum::<i64>()))
    })?;
    let sequence_algos = methods(lua, impl_t, "NI.DATA.SequenceAlgorithms")?;
    set_fn(lua, &sequence_algos, "invalidTick", |_, _: MultiValue| Ok(INVALID_TICK))?;
    let idea_algos = methods(lua, impl_t, "NI.DATA.IdeaSpaceAlgorithms")?;
    set_fn(lua, &idea_algos, "getPlayingPattern", |lua, args: MultiValue| {
        let Some(g) = find_arg(lua, &args, |r| if let ObjRef::Group(g) = r { Some(g) } else { None }) else { return Ok(Value::Nil) };
        let playing = {
            let st = state(lua);
            let st = st.borrow();
            match (st.transport.what, st.project()) {
                (Some(Playing::Pattern(pg, pi)), _) if pg == g => Some(pi),
                (Some(Playing::Scene(s)), Some(p)) => {
                    p.scenes.get(s).and_then(|s| s.patterns.iter().find(|(pg, _)| *pg == g)).map(|&(_, pi)| pi)
                }
                _ => None,
            }
        };
        playing.map_or(Ok(Value::Nil), |pi| object(lua, ObjRef::Pattern(g, pi)))
    })?;
    // In the pattern's own ticks: its start plus how far into its loop.
    set_fn(lua, &idea_algos, "patternCurrentPlayPosition", |lua, args: MultiValue| {
        let Some((g, pi)) = find_arg(lua, &args, |r| if let ObjRef::Pattern(g, p) = r { Some((g, p)) } else { None }) else {
            return Ok(INVALID_TICK);
        };
        let pos = match args.get(2) {
            Some(Value::Integer(i)) => *i,
            Some(Value::Number(n)) => *n as i64,
            _ => return Ok(INVALID_TICK),
        };
        let st = state(lua);
        let st = st.borrow();
        let Some(p) = st.project() else { return Ok(INVALID_TICK) };
        let Some(pat) = p.groups.get(g).and_then(|g| g.patterns.get(pi)) else { return Ok(INVALID_TICK) };
        let len = if pat.length > 0 { pat.length } else { p.bar_length() };
        Ok(pat.start + pos.rem_euclid(len))
    })?;
    // No loop range here: the arrangement's play position spans the song.
    set_fn(lua, &song, "getLoopParameter", |lua, _: Value| {
        let t = lua.create_table()?;
        t.set("getLoopBeginInTicks", lua.create_function(|_, _: MultiValue| Ok(0))?)?;
        t.set("getLoopLengthInTicks", lua.create_function(|_, _: MultiValue| Ok(0))?)?;
        Ok(t)
    })?;
    let scene_t = methods(lua, impl_t, "Scene")?;
    set_fn(lua, &scene_t, "getLongestPatternLength", |lua, this: Value| {
        let Some(ObjRef::Scene(s)) = object_ref(lua, &this) else { return Ok(0) };
        Ok(state(lua).borrow().project().filter(|p| s < p.scenes.len()).map_or(0, |p| p.scene_length(s)))
    })?;
    // Show the play position on the touch strip (the hardware settings' "On"; 0 Off,
    // 1 Rec. Only, 2 On).
    for (name, v) in [("PLAYPOSITION_TOUCHSTRIP_OFF", 0), ("PLAYPOSITION_TOUCHSTRIP_REC_ONLY", 1), ("PLAYPOSITION_TOUCHSTRIP_ON", 2)] {
        ni_data.set(name, v)?;
    }
    let prefs = methods(lua, impl_t, "NI.HW.getMaschinePreferences()")?;
    set_fn(lua, &prefs, "getShowPlayPositionOnTouchstrip", |_, _: MultiValue| Ok(2))?;
    set_fn(lua, &song_access, "toggleIdeasView", |lua, _: MultiValue| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        st.ideas_view = !st.ideas_view;
        Ok(())
    })?;
    set_fn(lua, &algos, "getSectionFromSectionPosition", |lua, (_song, pos): (Value, i64)| {
        let n = list_len(&state(lua).borrow(), ListKind::Sections);
        if pos < 0 || pos as usize >= n {
            return Ok(Value::Nil);
        }
        object(lua, ObjRef::Section(pos as usize))
    })?;
    let group = methods(lua, impl_t, "Group")?;
    set_fn(lua, &group, "getSounds", |lua, this: Value| match object_ref(lua, &this) {
        Some(ObjRef::Group(g)) => object(lua, ObjRef::List(ListKind::Sounds(g))),
        _ => Ok(Value::Nil),
    })?;
    set_fn(lua, &group, "getPatterns", |lua, this: Value| match object_ref(lua, &this) {
        Some(ObjRef::Group(g)) => object(lua, ObjRef::List(ListKind::Patterns(g))),
        _ => Ok(Value::Nil),
    })?;
    set_fn(lua, &group, "getFirstPattern", |lua, this: Value| match object_ref(lua, &this) {
        Some(ObjRef::Group(g)) if list_len(&state(lua).borrow(), ListKind::Patterns(g)) > 0 => object(lua, ObjRef::Pattern(g, 0)),
        _ => Ok(Value::Nil),
    })?;
    for ty in ["Group", "Sound", "EventPattern", "Scene"] {
        let t = methods(lua, impl_t, ty)?;
        set_fn(lua, &t, "getDisplayName", |lua, this: Value| {
            let r = object_ref(lua, &this);
            Ok(r.map_or(String::new(), |r| display_name(&state(lua).borrow(), &r)))
        })?;
    }

    // Lists (std::vector-like containers, 0-based).
    let list = methods(lua, impl_t, "List")?;
    set_fn(lua, &list, "size", |lua, this: Value| Ok(list_of(lua, &this).map_or(0, |k| list_len(&state(lua).borrow(), k))))?;
    set_fn(lua, &list, "empty", |lua, this: Value| Ok(list_of(lua, &this).map_or(0, |k| list_len(&state(lua).borrow(), k)) == 0))?;
    for name in ["at", "find"] {
        set_fn(lua, &list, name, |lua, (this, i): (Value, i64)| {
            let Some(k) = list_of(lua, &this) else { return Ok(Value::Nil) };
            if i < 0 || i as usize >= list_len(&state(lua).borrow(), k) {
                return Ok(Value::Nil);
            }
            object(lua, element(k, i as usize))
        })?;
    }
    // getNext(item)/getPrev(item): the item after or before it, or nil.
    for (name, step) in [("getNext", 1i64), ("getPrev", -1)] {
        set_fn(lua, &list, name, move |lua, (this, item): (Value, Value)| {
            let (Some(k), Some(r)) = (list_of(lua, &this), object_ref(lua, &item)) else { return Ok(Value::Nil) };
            let n = list_len(&state(lua).borrow(), k);
            let Some(i) = (0..n).find(|&i| element(k, i) == r) else { return Ok(Value::Nil) };
            match usize::try_from(i as i64 + step).ok().filter(|&j| j < n) {
                Some(j) => object(lua, element(k, j)),
                None => Ok(Value::Nil),
            }
        })?;
    }
    set_fn(lua, &list, "getFocusObject", |lua, this: Value| {
        let Some(k) = list_of(lua, &this) else { return Ok(Value::Nil) };
        let st = state(lua);
        let f = st.borrow().focus;
        let i = match k {
            ListKind::Groups => focus_group(&st.borrow()),
            ListKind::Sounds(g) => (f.group == g).then_some(f.sound),
            ListKind::Patterns(g) => (f.group == g).then_some(f.pattern).flatten(),
            ListKind::Scenes => f.scene,
            ListKind::Sections => f.section,
            ListKind::Slots => st.borrow().control.focus_slot,
        };
        match i.filter(|&i| i < list_len(&st.borrow(), k)) {
            Some(i) => object(lua, element(k, i)),
            None => Ok(Value::Nil),
        }
    })?;
    set_fn(lua, &list, "calcIndex", |lua, (this, item): (Value, Value)| {
        let (Some(k), Some(r)) = (list_of(lua, &this), object_ref(lua, &item)) else { return Ok(-1) };
        let n = list_len(&state(lua).borrow(), k);
        Ok((0..n).find(|&i| element(k, i) == r).map_or(-1, |i| i as i64))
    })?;
    set_fn(lua, &list, "isInSelection", |lua, (this, item): (Value, Value)| {
        let focus: Function = lua.load("return function(l) return l:getFocusObject() end").eval()?;
        let f: Value = focus.call(this)?;
        Ok(object_ref(lua, &f).is_some() && object_ref(lua, &f) == object_ref(lua, &item))
    })?;
    Ok(())
}

pub(super) fn set_fn<A, R>(lua: &Lua, t: &Table, name: &str, f: impl Fn(&Lua, A) -> mlua::Result<R> + 'static) -> mlua::Result<()>
where
    A: mlua::FromLuaMulti,
    R: mlua::IntoLuaMulti,
{
    t.set(name, lua.create_function(f)?)
}

/// `getTempoParameter` -> `Tempo`; `getArrangerZoomParameterHW` -> `ArrangerZoomHW`.
fn parameter_name(key: &str) -> Option<String> {
    let rest = key.strip_prefix("get")?;
    for suffix in ["Parameter", "ParameterHW", "ParameterSW"] {
        if let Some(name) = rest.strip_suffix(suffix).filter(|n| !n.is_empty()) {
            return Some(format!("{name}{}", &suffix[9..]));
        }
    }
    None
}

/// Make an edit to the shared document; failures go to the script log.
pub(super) fn edit<R>(lua: &Lua, f: impl FnOnce(&mut deckard_doc::Document) -> Result<R, String>) -> Option<R> {
    let doc = state(lua).borrow().doc.clone()?;
    let result = f(&mut doc.borrow_mut());
    match result {
        Ok(r) => Some(r),
        Err(e) => {
            state(lua).borrow_mut().log.push(format!("edit: {e}"));
            None
        }
    }
}

fn focus_scene(lua: &Lua, scene: Option<usize>) {
    if let Some(s) = scene {
        state(lua).borrow_mut().focus.scene = Some(s);
    }
}

/// The first argument that is an engine object `pick` accepts.
pub(super) fn find_arg<T>(lua: &Lua, args: &MultiValue, pick: impl Fn(ObjRef) -> Option<T>) -> Option<T> {
    args.iter().find_map(|v| object_ref(lua, v).and_then(&pick))
}

/// Focus pattern `p` of group `g` and put it in the focused scene.
pub(super) fn focus_and_assign_pattern(lua: &Lua, g: usize, p: usize) {
    let scene = {
        let st = state(lua);
        let mut st = st.borrow_mut();
        st.focus.group = g;
        st.focus.pattern = Some(p);
        st.focus.scene
    };
    if let Some(s) = scene {
        edit(lua, |d| d.assign_pattern(s, g, Some(p)));
    }
}

fn focus_group(st: &State) -> Option<usize> {
    let p = st.project()?;
    (st.focus.group < p.groups.len()).then_some(st.focus.group)
}

fn focus_sound(st: &State) -> Option<(usize, usize)> {
    let g = focus_group(st)?;
    (st.focus.sound < st.project().as_deref()?.groups[g].sounds.len()).then_some((g, st.focus.sound))
}

fn focus_pattern(st: &State) -> Option<(usize, usize)> {
    let g = focus_group(st)?;
    let p = st.focus.pattern?;
    (p < st.project().as_deref()?.groups[g].patterns.len()).then_some((g, p))
}

fn list_of(lua: &Lua, v: &Value) -> Option<ListKind> {
    match object_ref(lua, v)? {
        ObjRef::List(k) => Some(k),
        _ => None,
    }
}

fn list_len(st: &State, k: ListKind) -> usize {
    if k == ListKind::Slots {
        return st.control.slots.len();
    }
    let Some(p) = st.project() else { return 0 };
    match k {
        ListKind::Groups => p.groups.len(),
        ListKind::Sounds(g) => p.groups.get(g).map_or(0, |g| g.sounds.len()),
        ListKind::Patterns(g) => p.groups.get(g).map_or(0, |g| g.patterns.len()),
        ListKind::Scenes => p.scenes.len(),
        ListKind::Sections => p.sections.len(),
        ListKind::Slots => unreachable!(),
    }
}

fn element(k: ListKind, i: usize) -> ObjRef {
    match k {
        ListKind::Groups => ObjRef::Group(i),
        ListKind::Sounds(g) => ObjRef::Sound(g, i),
        ListKind::Patterns(g) => ObjRef::Pattern(g, i),
        ListKind::Scenes => ObjRef::Scene(i),
        ListKind::Sections => ObjRef::Section(i),
        ListKind::Slots => ObjRef::Slot(i),
    }
}

fn display_name(st: &State, r: &ObjRef) -> String {
    let Some(p) = st.project() else { return String::new() };
    let named = |name: &str, fallback: String| if name.is_empty() { fallback } else { name.to_string() };
    match *r {
        ObjRef::Group(g) => named(&p.groups[g].name, format!("Group {}", group_label(g))),
        ObjRef::Sound(g, s) => named(&p.groups[g].sounds[s].name, format!("Sound {}", s + 1)),
        ObjRef::Pattern(g, i) => named(&p.groups[g].patterns[i].name, format!("Pattern {}", i + 1)),
        ObjRef::Scene(s) => named(&p.scenes[s].name, format!("Scene {}", s + 1)),
        _ => String::new(),
    }
}

/// Group A..Z, then AA, AB, ...
/// A group's label as Maschine shows it: banks of eight, A1–H1, A2–H2, ...
fn group_label(i: usize) -> String {
    format!("{}{}", (b'A' + (i % 8) as u8) as char, i / 8 + 1)
}

/// 1-based `bar.beat.sixteenth` like the hardware's position display.
/// A position as bar.beat.sixteenth from 1.1.1, or (`duration`) a length as
/// bars:beats:sixteenths from 0:0:0, as Maschine shows pattern lengths.
/// `SequenceAlgorithms::invalidTick()`: no position.
pub(super) const INVALID_TICK: i64 = i64::MIN;

fn bar_beat_tick(p: &deckard_model::Project, tick: i64, duration: bool) -> String {
    let bar = p.bar_length().max(1);
    let beat = (PPQ * 4 / p.denominator.max(1)).max(1);
    let sixteenth = PPQ / 4;
    let in_bar = tick.rem_euclid(bar);
    let (bars, beats, sixteenths) = (tick.div_euclid(bar), in_bar / beat, (in_bar % beat) / sixteenth);
    if duration { format!("{bars}:{beats}:{sixteenths}") } else { format!("{}.{}.{}", bars + 1, beats + 1, sixteenths + 1) }
}

/// A color as the scripts read it: a "default" one as Maschine 3 shows it in a new
/// project (seen on the hardware: sounds 15, the first group 1, the first scene 4; how
/// its automatic group and scene colors go on from there isn't known yet).
fn color_prop(c: Color, default: i64) -> Prop {
    match c {
        Color::Palette(i) => Prop::Int(i as i64),
        Color::Default => Prop::Int(default),
    }
}

const DEFAULT_SOUND_COLOR: i64 = 15;
const DEFAULT_GROUP_COLOR: i64 = 1;
const DEFAULT_SCENE_COLOR: i64 = 4;

/// A scene's colour index as the scripts read it.
pub(super) fn scene_color(c: Color) -> i64 {
    match color_prop(c, DEFAULT_SCENE_COLOR) {
        Prop::Int(i) => i,
        _ => DEFAULT_SCENE_COLOR,
    }
}

/// The owner and name of a parameter handle.
fn param_parts(lua: &Lua, v: &Value) -> Option<(usize, ObjRef, String)> {
    let ObjRef::Param { owner, name } = object_ref(lua, v)? else { return None };
    let pid = state(lua).borrow_mut().objects.intern(ObjRef::Param { owner, name: name.clone() });
    let owner_ref = state(lua).borrow().objects.get(owner)?.clone();
    Some((pid, owner_ref, name))
}

pub(super) fn read_param(lua: &Lua, v: &Value) -> Option<Prop> {
    let (pid, owner, name) = param_parts(lua, v)?;
    let st = state(lua);
    let found = {
        let st = st.borrow();
        live_param(&st, &owner, &name).or_else(|| st.params.get(&pid).cloned())
    };
    Some(match found {
        Some(v) => v,
        None => app_default(lua, &owner, &name).or_else(|| super::scales::default(&owner, &name)).unwrap_or_else(|| default_for(&name)),
    })
}

/// App preferences whose defaults matter to the scripts, as Lua expressions.
fn app_default(lua: &Lua, owner: &ObjRef, name: &str) -> Option<Prop> {
    let expr = match (owner, name) {
        (ObjRef::Workspace, "FocusSoundOnPadEvent") => "NI.DATA.Workspace.FOCUS_SOUND_ON_PAD_EVENT_ON_TRIGGER",
        _ => return None,
    };
    lua.load(format!("return {expr}")).eval::<Value>().ok().map(|v| Prop::from_lua(&v))
}

pub(super) fn write_param(lua: &Lua, v: &Value, value: Prop) {
    let Some((pid, owner, name)) = param_parts(lua, v) else { return };
    // A list's value stays within it.
    let value = match (super::pad_mode::max(&owner, &name), value) {
        (Some(m), Prop::Int(i)) => Prop::Int(i.clamp(0, m)),
        (Some(m), Prop::Num(n)) => Prop::Int((n.round() as i64).clamp(0, m)),
        (_, v) => v,
    };
    let st = state(lua);
    let mut st = st.borrow_mut();
    if !set_live_param(&mut st, &owner, &name, &value) {
        st.params.insert(pid, value);
    }
    st.changed_params.insert(pid);
}

/// Parameters backed by the transport or the project model.
fn live_param(st: &State, owner: &ObjRef, name: &str) -> Option<Prop> {
    if let Some(v) = super::control::live(st, owner, name)
        .or_else(|| super::scales::live(st, owner, name))
        .or_else(|| super::pad_mode::live(st, owner, name))
        .or_else(|| super::pad_touch::live(st, owner, name))
        .or_else(|| super::pattern_editor::live(st, owner, name))
        .or_else(|| super::arranger::live(st, owner, name))
        .or_else(|| super::recorder::live(st, owner, name))
        .or_else(|| super::sampling::live(st, owner, name))
        .or_else(|| super::perform::live(st, owner, name))
        .or_else(|| super::lock::live(st, owner, name))
        .or_else(|| super::sync::live(st, owner, name))
    {
        return Some(v);
    }
    let t = &st.transport;
    match (owner, name) {
        (ObjRef::Workspace, "Playing") => return Some(Prop::Bool(t.playing)),
        (ObjRef::Workspace, "Recording") => return Some(Prop::Bool(t.recording)),
        (ObjRef::Workspace, "ArpActive") => return Some(Prop::Bool(st.arp.active)),
        (ObjRef::Metronome, "Enabled") => return Some(Prop::Bool(t.metronome)),
        (ObjRef::Song, "Loop") => return Some(Prop::Bool(t.looping)),
        _ => {}
    }
    // Step mode's Fixed Velocity is the pads' (the MK3's FIXED VEL button).
    // The Mixer shows the focused group's sounds (else the groups), as the app's does.
    if let (ObjRef::Song, "SoundsVisibleMixer") = (owner, name) {
        return Some(Prop::Bool(st.project().is_some_and(|p| p.mixer_sounds)));
    }
    if let (ObjRef::Song, STEP_FIXED_VELOCITY) = (owner, name) {
        return Some(Prop::Bool(matches!(live_param(st, owner, "PadVelocityMode"), Some(Prop::Int(PAD_VELOCITY_FIXED)))));
    }
    let p = st.project()?;
    Some(match (owner, name) {
        (ObjRef::Song, "Tempo") => Prop::Num(p.tempo),
        (ObjRef::Song, "Level") => Prop::Num(p.master_level),
        (ObjRef::Song, "SwingAmount") => Prop::Num(p.swing.amount),
        (ObjRef::Song, n) if song_field(n).is_some() => {
            let (field, _, lo, _) = song_field(n).unwrap_or_default();
            let stored = st.doc.as_ref().and_then(|d| d.borrow().num(deckard_doc::Target::Song, field));
            Prop::Int(stored.map_or(if field == "fixed_velocity" { 100 } else { lo }, |v| v as i64))
        }
        (ObjRef::Song, "Numerator") => Prop::Int(p.numerator),
        (ObjRef::Song, "Denominator") => Prop::Int(p.denominator),
        (ObjRef::Group(g), n) => {
            let g = p.groups.get(*g)?;
            match n {
                "Name" => Prop::Str(g.name.clone()),
                "Color" => color_prop(g.color, DEFAULT_GROUP_COLOR),
                "Level" => Prop::Num(g.level),
                "SwingAmount" => Prop::Num(g.swing.amount),
                "Pan" => Prop::Num(g.pan),
                "Mute" => Prop::Bool(g.muted),
                _ => return None,
            }
        }
        (ObjRef::Sound(g, s), n) => {
            let s = p.groups.get(*g)?.sounds.get(*s)?;
            match n {
                "Name" => Prop::Str(s.name.clone()),
                "Color" => color_prop(s.color, DEFAULT_SOUND_COLOR),
                "Level" => Prop::Num(s.level),
                "SwingAmount" => Prop::Num(s.swing.amount),
                "Pan" => Prop::Num(s.pan),
                "Mute" => Prop::Bool(s.muted),
                "ProbabilityLocked" => Prop::Bool(s.probability_locked),
                _ => return None,
            }
        }
        (ObjRef::Pattern(g, i), n) => {
            let group = p.groups.get(*g)?;
            let pat = group.patterns.get(*i)?;
            match n {
                "Name" => Prop::Str(pat.name.clone()),
                // A pattern's default color is its group's.
                "Color" => match color_prop(group.color, DEFAULT_GROUP_COLOR) {
                    Prop::Int(gc) => color_prop(pat.color, gc),
                    other => other,
                },
                "Length" => Prop::Int(if pat.length > 0 { pat.length } else { p.bar_length() }),
                "Start" => Prop::Int(pat.start),
                _ => return None,
            }
        }
        (ObjRef::Scene(i), n) => {
            let s = p.scenes.get(*i)?;
            match n {
                "Name" => Prop::Str(if s.name.is_empty() { format!("Scene {}", i + 1) } else { s.name.clone() }),
                "Color" => color_prop(s.color, DEFAULT_SCENE_COLOR),
                _ => return None,
            }
        }
        _ => return None,
    })
}

pub(super) fn set_live_param(st: &mut State, owner: &ObjRef, name: &str, v: &Prop) -> bool {
    if super::control::write(st, owner, name, v)
        || super::scales::write(st, owner, name, v)
        || super::pad_mode::write(st, owner, name, v)
        || super::pad_touch::write(st, owner, name, v)
        || super::pattern_editor::write(st, owner, name, v)
        || super::arranger::write(st, owner, name, v)
        || super::sampling::write(st, owner, name, v)
        || super::lock::write(st, owner, name, v)
        || super::sync::write(st, owner, name, v)
    {
        return true;
    }
    let b = matches!(v, Prop::Bool(true));
    if let (ObjRef::Song, STEP_FIXED_VELOCITY) = (owner, name) {
        return set_live_param(st, owner, "PadVelocityMode", &Prop::Int(if b { PAD_VELOCITY_FIXED } else { 0 }));
    }
    let num = match v {
        Prop::Int(i) => Some(*i as f64),
        Prop::Num(n) => Some(*n),
        _ => None,
    };
    match (owner, name) {
        (ObjRef::Workspace, "Playing") => st.transport.playing = b,
        (ObjRef::Workspace, "Recording") => st.transport.recording = b,
        (ObjRef::Metronome, "Enabled") => st.transport.metronome = b,
        (ObjRef::Song, "Loop") => st.transport.looping = b,
        _ => {
            use deckard_doc::Target;
            let Some(doc) = st.doc.clone() else { return false };
            let mut d = doc.borrow_mut();
            let groups = d.project().groups.len();
            let sounds = |g: usize| d.project().groups.get(g).map_or(0, |g| g.sounds.len());
            let target = match owner {
                ObjRef::Group(g) if *g < groups => Target::Group(*g),
                ObjRef::Sound(g, s) if *s < sounds(*g) => Target::Sound(*g, *s),
                ObjRef::Song => Target::Song,
                _ => return false,
            };
            let done = match (target, name, num, v) {
                (Target::Song, "Tempo", Some(n), _) => d.set_tempo(n),
                (Target::Song, "SoundsVisibleMixer", _, _) => {
                    d.set("Mixer Sounds Visible", target, "sounds_visible_mixer", mxprj::decode::Value::Int(i64::from(b)))
                }
                (Target::Song, name, Some(n), _) if song_field(name).is_some() => {
                    let (field, label, lo, hi) = song_field(name).unwrap_or_default();
                    d.set(label, target, field, mxprj::decode::Value::Int((n.round() as i64).clamp(lo, hi)))
                }
                (_, "Level", Some(n), _) => d.set_level(target, n),
                (_, "SwingAmount", Some(n), _) => {
                    let p = d.project();
                    let swing = match target {
                        Target::Group(g) => p.groups[g].swing,
                        Target::Sound(g, s) => p.groups[g].sounds[s].swing,
                        _ => p.swing,
                    };
                    d.set_swing(target, deckard_model::Swing { amount: n.clamp(0.0, 100.0), ..swing })
                }
                (Target::Group(_) | Target::Sound(..), "Pan", Some(n), _) => d.set_pan(target, n),
                (Target::Group(_) | Target::Sound(..), "Mute", _, _) => d.set_mute(target, b),
                (Target::Sound(g, s), "ProbabilityLocked", _, _) => d.set_probability_locked(g, s, b),
                (Target::Group(_) | Target::Sound(..), "Name", _, Prop::Str(s)) => d.rename(target, s),
                _ => return false,
            };
            if let Err(e) = done {
                st.log.push(format!("edit {owner:?}.{name}: {e}"));
                return false;
            }
        }
    }
    true
}

/// Step mode's Fixed Velocity switch, and the pad velocity mode it stands for
/// (`NI.HW.PAD_VELOCITY_FIXED`).
const STEP_FIXED_VELOCITY: &str = "StepModeFixedVelocityEnabled";
const PAD_VELOCITY_FIXED: i64 = 1;

/// Song settings kept in the project as they are, by the scripts' parameter name: the
/// project's field, the undo label, and the range.
fn song_field(name: &str) -> Option<(&'static str, &'static str, i64, i64)> {
    Some(match name {
        // Pad velocity: NI.HW.PAD_VELOCITY_NORMAL, _FIXED or _16_LEVELS.
        "PadVelocityMode" => ("pad_velocity_mode", "Pad Velocity Mode", 0, 2),
        // What pads (and new steps) play with in Fixed Velocity: 0–127, Maschine's
        // default 100.
        "FixedVelocity" => ("fixed_velocity", "Fixed Velocity", 0, 127),
        _ => return None,
    })
}

/// Unknown parameters start as `false` if the name reads like a switch, else 0.
fn default_for(name: &str) -> Prop {
    const SWITCHES: &[&str] =
        &["Enabled", "Playing", "Recording", "Visible", "Active", "Mute", "Solo", "Lock", "Loop", "InProgress", "On", "Locked", "Pinned"];
    if SWITCHES.iter().any(|s| name.ends_with(s) || name.contains(&format!("{s}HW"))) { Prop::Bool(false) } else { Prop::Int(0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_names() {
        assert_eq!(parameter_name("getTempoParameter").as_deref(), Some("Tempo"));
        assert_eq!(parameter_name("getArrangerZoomParameterHW").as_deref(), Some("ArrangerZoomHW"));
        assert_eq!(parameter_name("getParameter"), None);
        assert_eq!(parameter_name("getSounds"), None);
    }

    #[test]
    fn group_labels() {
        assert_eq!(group_label(0), "A1");
        assert_eq!(group_label(7), "H1");
        assert_eq!(group_label(8), "A2");
    }
}

/// A channel's level (in dB) or pan as the mixer shows it: "-6.0 dB", "-inf dB"; "C",
/// "L 50", "R 25".
/// A channel's level moved by `inc`: a wheel click 0.5 dB, a knob's turn the meter's
/// −60 … +10 dB; below −60 dB, silent.
fn level_step(v: f64, inc: f64, wheel: bool) -> f64 {
    const LOW: f64 = -60.0;
    let db = deckard_model::level_db(v);
    let db = if db.is_finite() { db.max(LOW - 0.5) } else { LOW - 0.5 };
    let db = db + if wheel { inc * 0.5 } else { inc * 70.0 };
    if db < LOW { 0.0 } else { deckard_model::level_from_db(db.min(10.0)) }
}

fn mix_string(owner: &ObjRef, name: &str, v: f64) -> Option<String> {
    if !matches!(owner, ObjRef::Group(_) | ObjRef::Sound(..) | ObjRef::Song) {
        return None;
    }
    Some(match name {
        "Level" => {
            let db = deckard_model::level_db(v);
            if db.is_finite() && db > -100.0 { format!("{:.1} dB", (db * 10.0).round() / 10.0 + 0.0) } else { "-inf dB".into() }
        }
        "SwingAmount" => format!("{v:.1} %"),
        "Pan" => match (v * 100.0).round() as i64 {
            0 => "C".into(),
            p if p < 0 => format!("L {}", -p),
            p => format!("R {p}"),
        },
        _ => return None,
    })
}
