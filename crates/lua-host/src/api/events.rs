//! Note events for the controller's Step, Events and Pattern pages: what NI's scripts
//! call to find, add, remove, select and edit the notes of the focused pattern
//! (`NI.DATA.EventPatternAccess`, `EventPatternTools`, `SequenceAlgorithms`, …), on the
//! shared document as undoable edits. The selection lives in the host's state and is
//! shared with the app's pattern editor.
//!
//! Positions are pattern time (the pattern plays `start .. start + length`), as in the
//! document. Calls on a pattern or sound other than the focused one act on the one
//! given where they name it.

use super::data::{INVALID_TICK, edit, find_arg, focus_and_assign_pattern, set_fn};
use super::{methods, object, object_ref, state};
use crate::state::{ObjRef, Playing, State};
use deckard_model::{Note, PPQ};
use mlua::{Lua, MultiValue, Table, Value};

/// The smallest nudge or length step (SHIFT on the knobs): a 128th note.
pub(crate) const MIN_SNAP: i64 = PPQ / 32;

/// Maschine's comparison conditions for `getNoteEventByNote`.
const COND_EQUAL: i64 = 0;
const COND_GREATER: i64 = 1;
const COND_GREATER_EQUAL: i64 = 2;
const COND_LESS: i64 = 3;
const COND_LESS_EQUAL: i64 = 4;

fn int(args: &MultiValue, i: usize) -> Option<i64> {
    match args.get(i)? {
        Value::Integer(n) => Some(*n),
        Value::Number(n) => Some(n.round() as i64),
        _ => None,
    }
}

fn boolean(args: &MultiValue, i: usize) -> bool {
    matches!(args.get(i), Some(Value::Boolean(true)))
}

/// The focused pattern and sound: (group, pattern, sound).
fn focus(st: &State) -> Option<(usize, usize, usize)> {
    Some((st.focus.group, st.focus.pattern?, st.focus.sound))
}

/// A pattern's notes, and where it plays (start, end).
fn notes(st: &State, g: usize, p: usize) -> (Vec<Note>, i64, i64) {
    let Some(pr) = st.project() else { return (Vec::new(), 0, 0) };
    let Some(pat) = pr.groups.get(g).and_then(|g| g.patterns.get(p)) else { return (Vec::new(), 0, 0) };
    let len = if pat.length > 0 { pat.length } else { pr.bar_length() };
    (pat.notes.clone(), pat.start, pat.start + len)
}

/// The selected notes of pattern (g, p) still in it.
fn selected(st: &State, g: usize, p: usize) -> Vec<Note> {
    let Some((sg, sp, sel)) = &st.selection else { return Vec::new() };
    if (*sg, *sp) != (g, p) {
        return Vec::new();
    }
    let (all, ..) = notes(st, g, p);
    sel.iter().filter(|n| all.contains(n)).copied().collect()
}

fn select(lua: &Lua, g: usize, p: usize, sel: Vec<Note>) {
    state(lua).borrow_mut().selection = Some((g, p, sel));
}

/// Replace `remove` with `add` in pattern (g, p) as one undo step named `label`; the
/// selection becomes `select_after` (if given).
fn change(lua: &Lua, label: &str, g: usize, p: usize, remove: &[Note], add: &[Note], select_after: Option<Vec<Note>>) {
    if remove.is_empty() && add.is_empty() {
        return;
    }
    // (While recording: a step of its own, not part of the take.)
    if edit(lua, |d| d.outside_take(|d| d.edit_notes(label, g, p, remove, add))).is_some()
        && let Some(sel) = select_after
    {
        select(lua, g, p, sel);
    }
}

/// Whether the pads (and the displays) are in Keyboard view: the focused sound only.
fn keyboard_view(lua: &Lua) -> bool {
    lua.load("return PadModeHelper and PadModeHelper.getKeyboardMode() or false").eval().unwrap_or(false)
}

/// The notes a command works on: the selection; with none, what the displays show (the
/// manual: in Keyboard view the focused sound's notes, else the group's).
fn targets(st: &State, g: usize, p: usize, keyboard: bool) -> Vec<Note> {
    let sel = selected(st, g, p);
    if !sel.is_empty() {
        return sel;
    }
    let all = notes(st, g, p).0;
    if keyboard { all.into_iter().filter(|n| n.sound == st.focus.sound).collect() } else { all }
}

/// Edit the selection, or with `shown_if_none` and nothing selected what the displays
/// show (`targets`): `f` maps each note (with the pattern's start and end); the edited
/// notes stay selected.
fn modify(lua: &Lua, label: &str, shown_if_none: bool, f: impl Fn(Note, i64, i64) -> Note) {
    let keyboard = shown_if_none && keyboard_view(lua);
    let (g, p, olds, start, end) = {
        let st = state(lua);
        let st = st.borrow();
        let Some((g, p, _)) = focus(&st) else { return };
        let (_, start, end) = notes(&st, g, p);
        let olds = if shown_if_none { targets(&st, g, p, keyboard) } else { selected(&st, g, p) };
        (g, p, olds, start, end)
    };
    let news: Vec<Note> = olds.iter().map(|n| f(*n, start, end)).collect();
    let pairs: Vec<(Note, Note)> = olds.into_iter().zip(news).filter(|(a, b)| a != b).collect();
    if pairs.is_empty() {
        return;
    }
    let (remove, add): (Vec<Note>, Vec<Note>) = pairs.into_iter().unzip();
    let keep: Vec<Note> = {
        let st = state(lua);
        let st = st.borrow();
        let had = selected(&st, g, p);
        if had.is_empty() { Vec::new() } else { add.clone() }
    };
    change(lua, label, g, p, &remove, &add, Some(keep));
}

/// What a double-tapped knob resets the selected notes to.
pub(crate) enum Reset {
    /// Velocity 100.
    Velocity,
    /// Probability 100 %.
    Probability,
}

/// Reset the selected notes' velocity or probability (one undo step).
pub(crate) fn reset_selected(lua: &Lua, what: Reset) {
    match what {
        Reset::Velocity => modify(lua, "Velocity", false, |n, _, _| Note { velocity: 100, ..n }),
        Reset::Probability => modify(lua, "Probability", false, |n, _, _| Note { probability: 100, ..n }),
    }
}

/// A note as the scripts see it.
fn note_event(lua: &Lua, n: &Note) -> mlua::Result<Value> {
    let t = lua.create_table()?;
    t.set("sound", n.sound)?;
    t.set("position", n.position)?;
    t.set("length", n.length)?;
    t.set("pitch", n.pitch)?;
    t.set("velocity", n.velocity)?;
    t.set("probability", n.probability)?;
    let host: Table = lua.globals().get("__host")?;
    host.get::<mlua::Function>("note_event")?.call(t)
}

fn note_of(t: &Table) -> Option<Note> {
    Some(Note {
        expr: None,
        sound: t.get("sound").ok()?,
        position: t.get("position").ok()?,
        length: t.get("length").ok()?,
        pitch: t.get("pitch").ok()?,
        velocity: t.get("velocity").ok()?,
        probability: t.get("probability").ok()?,
    })
}

fn vector(lua: &Lua, items: &[Note]) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (i, n) in items.iter().enumerate() {
        list.set(i + 1, note_event(lua, n)?)?;
    }
    let host: Table = lua.globals().get("__host")?;
    host.get::<mlua::Function>("vector")?.call(list)
}

/// The notes in a vector the host handed out.
fn vector_notes(v: Option<&Value>) -> Vec<Note> {
    let Some(Value::Table(t)) = v else { return Vec::new() };
    let Ok(items) = t.get::<Table>("items") else { return Vec::new() };
    items.sequence_values::<Table>().filter_map(|e| e.ok().and_then(|e| note_of(&e))).collect()
}

/// The integers in a `VectorInt` the scripts filled.
fn ints(v: Option<&Value>) -> Vec<i64> {
    let Some(Value::Table(t)) = v else { return Vec::new() };
    let Ok(items) = t.get::<Table>("items") else { return Vec::new() };
    items.sequence_values::<i64>().filter_map(Result::ok).collect()
}

/// Where the focused pattern plays now (pattern time), if it does.
pub(crate) fn play_tick(st: &State) -> Option<i64> {
    let (g, p, _) = focus(st)?;
    if !st.transport.playing {
        return None;
    }
    let pr = st.project()?;
    let playing = match st.transport.what? {
        Playing::Pattern(pg, pp) => (pg, pp) == (g, p),
        Playing::Scene(s) => pr.scenes.get(s).is_some_and(|sc| sc.patterns.contains(&(g, p))),
        Playing::Arrangement => false,
    };
    let pat = pr.groups.get(g)?.patterns.get(p)?;
    let len = if pat.length > 0 { pat.length } else { pr.bar_length() };
    playing.then(|| pat.start + st.transport.position.rem_euclid(len.max(1)))
}

/// `(g, p, sound)` from a `Sequence` argument, else the focus.
fn sequence_arg(lua: &Lua, args: &MultiValue) -> Option<(usize, usize, usize)> {
    find_arg(lua, args, |r| if let ObjRef::Sequence(g, p, s) = r { Some((g, p, s)) } else { None }).or_else(|| focus(&state(lua).borrow()))
}

fn pattern_arg(lua: &Lua, args: &MultiValue) -> Option<(usize, usize)> {
    find_arg(lua, args, |r| if let ObjRef::Pattern(g, p) = r { Some((g, p)) } else { None })
        .or_else(|| focus(&state(lua).borrow()).map(|(g, p, _)| (g, p)))
}

fn sound_arg(lua: &Lua, args: &MultiValue) -> Option<usize> {
    find_arg(lua, args, |r| if let ObjRef::Sound(_, s) = r { Some(s) } else { None })
}

/// Make a pattern for the focused group if it has none in focus (into the scene).
fn ensure_pattern(lua: &Lua) {
    let g = {
        let st = state(lua);
        let st = st.borrow();
        if st.focus.pattern.is_some() || st.doc.is_none() {
            return;
        }
        st.focus.group
    };
    let n = state(lua).borrow().project().map_or(0, |p| p.groups.get(g).map_or(0, |g| g.patterns.len()));
    if let Some(made) = edit(lua, |d| d.insert_pattern(g, 0, n, &format!("Pattern {}", n + 1))) {
        focus_and_assign_pattern(lua, g, made);
    }
}

/// The focused sound's notes in pattern order (position, then pitch).
fn sound_notes(st: &State, g: usize, p: usize, sound: usize) -> Vec<Note> {
    let mut v: Vec<Note> = notes(st, g, p).0.into_iter().filter(|n| n.sound == sound).collect();
    v.sort_by_key(|n| (n.position, n.pitch));
    v
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    // The focus.
    let sh = methods(lua, impl_t, "NI.DATA.StateHelper")?;
    set_fn(lua, &sh, "getFocusSoundSequence", |lua, _: MultiValue| {
        let f = focus(&state(lua).borrow());
        f.map_or(Ok(Value::Nil), |(g, p, s)| object(lua, ObjRef::Sequence(g, p, s)))
    })?;
    set_fn(lua, &sh, "getFocusPatternActiveRangeBegin", |lua, _: MultiValue| {
        let st = state(lua);
        let st = st.borrow();
        Ok(focus(&st).map_or(0, |(g, p, _)| notes(&st, g, p).1))
    })?;
    set_fn(lua, &sh, "getFocusPatternActiveRangeEnd", |lua, _: MultiValue| {
        let st = state(lua);
        let st = st.borrow();
        Ok(focus(&st).map_or(0, |(g, p, _)| notes(&st, g, p).2))
    })?;
    // isSongEntityFocused(App, Entity): a pattern is in focus whenever one is (DECKARD
    // has no clips), which shows the Step page's DOUBLE.
    set_fn(lua, &sh, "isSongEntityFocused", |lua, args: MultiValue| {
        let pattern: i64 = lua.load("return NI.DATA.FOCUS_ENTITY_PATTERN").eval().unwrap_or(0);
        Ok(int(&args, 1) == Some(pattern) && state(lua).borrow().focus.pattern.is_some())
    })?;
    set_fn(lua, &sh, "getFocusClipEvent", |_, _: MultiValue| Ok(Value::Nil))?;

    // A sound's notes in a pattern.
    let seq = methods(lua, impl_t, "Sequence")?;
    set_fn(lua, &seq, "empty", |lua, this: Value| {
        let Some(ObjRef::Sequence(g, p, s)) = super::object_ref(lua, &this) else { return Ok(true) };
        Ok(sound_notes(&state(lua).borrow(), g, p, s).is_empty())
    })?;
    set_fn(lua, &seq, "isChanged", |_, _: Value| Ok(true))?;
    let pattern = methods(lua, impl_t, "EventPattern")?;
    set_fn(lua, &pattern, "getSoundSequence", |lua, (this, sound): (Value, Value)| {
        let (Some(ObjRef::Pattern(g, p)), Some(ObjRef::Sound(_, s))) = (super::object_ref(lua, &this), super::object_ref(lua, &sound))
        else {
            return Ok(Value::Nil);
        };
        object(lua, ObjRef::Sequence(g, p, s))
    })?;
    set_fn(lua, &pattern, "isEmpty", |lua, this: Value| {
        let Some(ObjRef::Pattern(g, p)) = super::object_ref(lua, &this) else { return Ok(true) };
        Ok(notes(&state(lua).borrow(), g, p).0.is_empty())
    })?;

    // The step grid and the play position in steps.
    let song = methods(lua, impl_t, "Song")?;
    // getPatternEditorSnapInTicks(UseSnapEnabled): the step grid, or with the grid off
    // (GRID) and UseSnapEnabled the smallest step.
    set_fn(lua, &song, "getPatternEditorSnapInTicks", |lua, args: MultiValue| {
        let st = state(lua);
        let st = st.borrow();
        Ok(if boolean(&args, 1) && st.editor.grid_off { MIN_SNAP } else { st.step_grid })
    })?;
    // A note's name, middle C (60) as C3.
    set_fn(lua, &song, "getNoteToString", |_, args: MultiValue| Ok(super::scales::note_name(int(&args, 1).unwrap_or(60))))?;
    let cache = methods(lua, impl_t, "StateCache")?;
    set_fn(lua, &cache, "playPositionStepEditor", |lua, _: Value| {
        let st = state(lua);
        let st = st.borrow();
        let Some(tick) = play_tick(&st) else { return Ok(-1) };
        let (_, start, _) = focus(&st).map_or((Vec::new(), 0, 0), |(g, p, _)| notes(&st, g, p));
        let grid = st.step_grid.max(1);
        let first = -(-start).div_euclid(grid) * grid;
        Ok((tick - first).div_euclid(grid))
    })?;

    // Audio plug-ins' loops (docs/audio-plugin.md).
    let sound_algos = methods(lua, impl_t, "NI.DATA.SoundAlgorithms")?;
    // An Audio plug-in in Loop mode (its events pages hide what only notes have).
    set_fn(lua, &sound_algos, "hasAudioModuleInLoopMode", |lua, sound: Value| {
        let Some(ObjRef::Sound(g, s)) = object_ref(lua, &sound) else { return Ok(false) };
        let st = state(lua);
        let st = st.borrow();
        Ok(st
            .project()
            .and_then(|p| p.groups.get(g)?.sounds.get(s).cloned())
            .is_some_and(|snd| snd.instrument == "AudioModule" && snd.audio.is_some_and(|a| a.mode == deckard_model::AudioMode::Loop)))
    })?;
    let pattern_algos = methods(lua, impl_t, "NI.DATA.EventPatternAlgorithms")?;
    // The sound's Audio plug-in plays in the pattern (it has its AudioEvent).
    set_fn(lua, &pattern_algos, "isAudioLoopEnabled", |lua, (pattern, sound): (Value, Value)| {
        let (Some(ObjRef::Pattern(g, p)), Some(ObjRef::Sound(_, s))) = (object_ref(lua, &pattern), object_ref(lua, &sound)) else {
            return Ok(false);
        };
        let st = state(lua);
        let st = st.borrow();
        Ok(st.project().and_then(|pr| pr.groups.get(g)?.patterns.get(p).cloned()).is_some_and(|pt| pt.audio.iter().any(|c| c.sound == s)))
    })?;
    // How far a loop reaches in the pattern (ticks): it fills it, repeating.
    let audio_algos = methods(lua, impl_t, "NI.DATA.AudioModuleAlgorithms")?;
    set_fn(lua, &audio_algos, "getAudioLoopLength", |lua, (_, pattern): (Value, Value)| {
        let Some(ObjRef::Pattern(g, p)) = object_ref(lua, &pattern) else { return Ok(0) };
        let st = state(lua);
        let st = st.borrow();
        Ok(st
            .project()
            .and_then(|pr| {
                let pt = pr.groups.get(g)?.patterns.get(p)?;
                Some(if pt.length > 0 { pt.length } else { pr.bar_length() })
            })
            .unwrap_or(0))
    })?;
    // A step pressed on a Loop-mode sound: its loop on (its take in focus) or off in the
    // pattern, as one undo step.
    let pattern_access = methods(lua, impl_t, "NI.DATA.EventPatternAccess")?;
    set_fn(lua, &pattern_access, "toggleAudioLoop", |lua, args: MultiValue| {
        let Some((g, p)) = find_arg(lua, &args, |r| if let ObjRef::Pattern(g, p) = r { Some((g, p)) } else { None }) else { return Ok(()) };
        let Some(s) = find_arg(lua, &args, |r| if let ObjRef::Sound(_, s) = r { Some(s) } else { None }) else { return Ok(()) };
        edit(lua, |d| {
            let pr = d.project();
            let on = pr.groups.get(g).and_then(|gr| gr.patterns.get(p)).is_some_and(|pt| pt.audio.iter().any(|c| c.sound == s));
            let take = pr.groups.get(g).and_then(|gr| gr.sounds.get(s)).and_then(|x| x.audio.as_ref()).map_or(0, |a| a.focused);
            let (take, label) = if on { (None, "Audio Off") } else { (Some(take), "Audio On") };
            d.set_audio_in(g, s, &[p], take, label)
        });
        Ok(())
    })?;
    // A sound's Audio plug-in (its first module, when it's one).
    let sound_t = methods(lua, impl_t, "Sound")?;
    set_fn(lua, &sound_t, "getAudioModule", |lua, this: Value| {
        let Some(ObjRef::Sound(g, s)) = object_ref(lua, &this) else { return Ok(Value::Nil) };
        let is = state(lua)
            .borrow()
            .project()
            .and_then(|pr| pr.groups.get(g)?.sounds.get(s).map(|x| x.instrument == "AudioModule"))
            .unwrap_or(false);
        if is { object(lua, ObjRef::Module(0)) } else { Ok(Value::Nil) }
    })?;
    let workspace = methods(lua, impl_t, "NI.DATA.WORKSPACE")?;
    set_fn(lua, &workspace, "createPatternIfNeeded", |lua, _: MultiValue| {
        ensure_pattern(lua);
        Ok(())
    })?;

    // Step mode.
    let access = methods(lua, impl_t, "NI.DATA.EventPatternAccess")?;
    // getSoundNoteEvents(Pattern, Sound, Time, Length, Note): the sound's notes starting
    // in [Time, Time + Length) at that pitch (any, for CONST_INVALID_NOTE).
    set_fn(lua, &access, "getSoundNoteEvents", |lua, args: MultiValue| {
        let (Some((g, p)), Some(s)) = (pattern_arg(lua, &args), sound_arg(lua, &args)) else { return vector(lua, &[]) };
        let (t, len, pitch) = (int(&args, 2).unwrap_or(0), int(&args, 3).unwrap_or(0), int(&args, 4).unwrap_or(128));
        let found: Vec<Note> = sound_notes(&state(lua).borrow(), g, p, s)
            .into_iter()
            .filter(|n| n.position >= t && n.position < t + len && (!(0..128).contains(&pitch) || n.pitch == pitch))
            .collect();
        vector(lua, &found)
    })?;
    // insertSoundEventStepMode(App, Pattern, Sound, Time, Pitch, Velocity, Length): the
    // new note is the selection, as Maschine selects the last step created.
    set_fn(lua, &access, "insertSoundEventStepMode", |lua, args: MultiValue| {
        ensure_pattern(lua);
        let Some(s) = sound_arg(lua, &args) else { return Ok(()) };
        let Some((g, p)) = pattern_arg(lua, &args) else { return Ok(()) };
        let n = Note {
            expr: None,
            sound: s,
            position: int(&args, 3).unwrap_or(0),
            pitch: int(&args, 4).unwrap_or(60).clamp(0, 127),
            velocity: int(&args, 5).unwrap_or(100).clamp(0, 127),
            length: int(&args, 6).unwrap_or(PPQ / 4).max(1),
            probability: 100,
        };
        change(lua, "Add Step", g, p, &[], &[n], Some(vec![n]));
        Ok(())
    })?;
    set_fn(lua, &access, "removeSoundEventsStepMode", |lua, args: MultiValue| {
        let Some((g, p)) = pattern_arg(lua, &args) else { return Ok(()) };
        let gone = vector_notes(args.get(3));
        let keep: Vec<Note> = {
            let st = state(lua);
            let st = st.borrow();
            selected(&st, g, p).into_iter().filter(|n| !gone.contains(n)).collect()
        };
        change(lua, "Remove Step", g, p, &gone, &[], Some(keep));
        Ok(())
    })?;
    // selectNoteEventsByHWPadIndexes(App, Song, Sound, Indexes): the notes on those
    // steps (held pads), selected.
    set_fn(lua, &access, "selectNoteEventsByHWPadIndexes", |lua, args: MultiValue| {
        let steps = ints(args.get(3));
        let st = state(lua);
        let mut st = st.borrow_mut();
        let Some((g, p, s)) = focus(&st) else { return Ok(()) };
        let (_, start, _) = notes(&st, g, p);
        let grid = st.step_grid.max(1);
        let first = -(-start).div_euclid(grid) * grid;
        let sel: Vec<Note> = sound_notes(&st, g, p, s)
            .into_iter()
            .filter(|n| steps.iter().any(|&i| n.position >= first + i * grid && n.position < first + (i + 1) * grid))
            .collect();
        st.selection = Some((g, p, sel));
        Ok(())
    })?;
    // removeEventsByPadIndex(App, Index, UseKeyboardMode): EVENTS + ERASE + pad: that
    // sound's notes (or, when UseKeyboardMode and the pads are in Keyboard mode, the
    // focused sound's at the pad's pitch).
    set_fn(lua, &access, "removeEventsByPadIndex", |lua, args: MultiValue| {
        let (idx, keyboard) = (int(&args, 1).unwrap_or(0), boolean(&args, 2) && keyboard_view(lua));
        // The root comes from the scripts: read it before borrowing the state.
        let root = super::scales::root_note(lua);
        let pitch = keyboard.then(|| super::scales::pad_note(&state(lua).borrow().scale, root, idx));
        let (g, p, gone) = {
            let st = state(lua);
            let st = st.borrow();
            let Some((g, p, s)) = focus(&st) else { return Ok(()) };
            let all = notes(&st, g, p).0;
            let gone: Vec<Note> = if keyboard {
                all.into_iter().filter(|n| n.sound == s && Some(n.pitch) == pitch).collect()
            } else {
                all.into_iter().filter(|n| n.sound as i64 == idx).collect()
            };
            (g, p, gone)
        };
        change(lua, "Delete Events", g, p, &gone, &[], None);
        Ok(())
    })?;
    set_fn(lua, &access, "canDoublePattern", |lua, _: MultiValue| Ok(focus(&state(lua).borrow()).is_some()))?;
    for name in ["doublePattern", "doubleFocusPatternOrClipEvent"] {
        set_fn(lua, &access, name, |lua, args: MultiValue| {
            let Some((g, p)) = pattern_arg(lua, &args) else { return Ok(()) };
            let (all, start, end) = notes(&state(lua).borrow(), g, p);
            let len = end - start;
            let copies: Vec<Note> =
                all.iter().filter(|n| n.position >= start && n.position < end).map(|n| Note { position: n.position + len, ..*n }).collect();
            if edit(lua, |d| d.set_pattern_span(g, p, start, len * 2)).is_some() {
                change(lua, "Double Pattern", g, p, &[], &copies, None);
            }
            Ok(())
        })?;
    }
    // Auto length: no length chosen for the focused pattern (docs/superpowers/plans/
    // 2026-10-23-auto-length-recording.md); Record Prepare shows "Auto".
    set_fn(lua, &access, "hasFocusedPatternAutomaticLength", |lua, _: MultiValue| {
        let st = state(lua);
        let st = st.borrow();
        let (g, p) = (st.focus.group, st.focus.pattern);
        Ok(p.zip(st.doc.as_ref()).is_some_and(|(p, d)| d.borrow().length_is_auto(g, p)))
    })?;
    set_fn(lua, &access, "setExplicitLength", |lua, args: MultiValue| {
        let Some((g, p)) = pattern_arg(lua, &args) else { return Ok(()) };
        edit(lua, |d| {
            d.set_length_auto(g, p, false);
            Ok(())
        });
        let start = notes(&state(lua).borrow(), g, p).1;
        if let Some(len) = int(&args, 2).filter(|&l| l > 0) {
            edit(lua, |d| d.set_pattern_span(g, p, start, len));
        }
        Ok(())
    })?;
    set_fn(lua, &access, "setLengthInBars", |lua, args: MultiValue| {
        let Some((g, p)) = pattern_arg(lua, &args) else { return Ok(()) };
        edit(lua, |d| {
            d.set_length_auto(g, p, false);
            Ok(())
        });
        let (start, bar) = {
            let st = state(lua);
            let st = st.borrow();
            (notes(&st, g, p).1, st.project().map_or(PPQ * 4, |pr| pr.bar_length()))
        };
        if let Some(bars) = int(&args, 2).filter(|&b| b > 0) {
            edit(lua, |d| d.set_pattern_span(g, p, start, bars * bar));
        }
        Ok(())
    })?;

    // Selecting.
    let tools = methods(lua, impl_t, "NI.DATA.EventPatternTools")?;
    for (name, v) in [
        ("COND_EQUAL", COND_EQUAL),
        ("COND_GREATER", COND_GREATER),
        ("COND_GREATER_EQUAL", COND_GREATER_EQUAL),
        ("COND_LESS", COND_LESS),
        ("COND_LESS_EQUAL", COND_LESS_EQUAL),
    ] {
        tools.set(name, v)?;
    }
    // getNoteEventByNote(Sequence, From, To, Pitch, _, Condition): the note starting in
    // [From, To] whose pitch meets the condition, nearest to Pitch.
    set_fn(lua, &tools, "getNoteEventByNote", |lua, args: MultiValue| {
        let Some((g, p, s)) = sequence_arg(lua, &args) else { return Ok(Value::Nil) };
        let (a, b, pitch, cond) =
            (int(&args, 1).unwrap_or(0), int(&args, 2).unwrap_or(0), int(&args, 3).unwrap_or(0), int(&args, 5).unwrap_or(0));
        let found = sound_notes(&state(lua).borrow(), g, p, s)
            .into_iter()
            .filter(|n| n.position >= a && n.position <= b)
            .filter(|n| match cond {
                COND_GREATER => n.pitch > pitch,
                COND_GREATER_EQUAL => n.pitch >= pitch,
                COND_LESS => n.pitch < pitch,
                COND_LESS_EQUAL => n.pitch <= pitch,
                _ => n.pitch == pitch,
            })
            .min_by_key(|n| (n.pitch - pitch).abs());
        found.map_or(Ok(Value::Nil), |n| note_event(lua, &n))
    })?;
    set_fn(lua, &tools, "anyNoteEventHasLessVelocity", |lua, args: MultiValue| {
        let (Some((g, p)), Some(s)) = (pattern_arg(lua, &args), sound_arg(lua, &args)) else { return Ok(false) };
        let (t, len, pitch, vel) =
            (int(&args, 2).unwrap_or(0), int(&args, 3).unwrap_or(0), int(&args, 4).unwrap_or(128), int(&args, 5).unwrap_or(127));
        Ok(sound_notes(&state(lua).borrow(), g, p, s)
            .iter()
            .any(|n| n.position >= t && n.position <= t + len && (!(0..128).contains(&pitch) || n.pitch == pitch) && n.velocity < vel))
    })?;
    set_fn(lua, &tools, "setNoteEventsVelocity", |lua, args: MultiValue| {
        let (Some((g, p)), Some(s)) = (pattern_arg(lua, &args), sound_arg(lua, &args)) else { return Ok(()) };
        let (t, len, pitch, vel) =
            (int(&args, 3).unwrap_or(0), int(&args, 4).unwrap_or(0), int(&args, 5).unwrap_or(128), int(&args, 6).unwrap_or(127));
        let olds: Vec<Note> = sound_notes(&state(lua).borrow(), g, p, s)
            .into_iter()
            .filter(|n| n.position >= t && n.position <= t + len && (!(0..128).contains(&pitch) || n.pitch == pitch))
            .collect();
        let news: Vec<Note> = olds.iter().map(|n| Note { velocity: vel.clamp(1, 127), ..*n }).collect();
        change(lua, "Accent", g, p, &olds, &news, None);
        Ok(())
    })?;
    set_fn(lua, &tools, "selectAllEvents", |lua, _: MultiValue| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        if let Some((g, p, _)) = focus(&st) {
            let all = notes(&st, g, p).0;
            st.selection = Some((g, p, all));
        }
        Ok(())
    })?;
    set_fn(lua, &tools, "deselectAllEvents", |lua, _: MultiValue| {
        state(lua).borrow_mut().selection = None;
        Ok(())
    })?;
    set_fn(lua, &tools, "hasNoteEditEvents", |lua, _: MultiValue| {
        let st = state(lua);
        let st = st.borrow();
        Ok(focus(&st).is_some_and(|(g, p, _)| !notes(&st, g, p).0.is_empty()))
    })?;
    set_fn(lua, &tools, "hasEventsForSound", |lua, args: MultiValue| {
        let st = state(lua);
        let Some(s) = sound_arg(lua, &args) else { return Ok(false) };
        let st = st.borrow();
        Ok(focus(&st).is_some_and(|(g, p, _)| !sound_notes(&st, g, p, s).is_empty()))
    })?;
    set_fn(lua, &tools, "allEventsSelectedForSound", |lua, args: MultiValue| {
        let Some(s) = sound_arg(lua, &args) else { return Ok(false) };
        let st = state(lua);
        let st = st.borrow();
        let Some((g, p, _)) = focus(&st) else { return Ok(false) };
        let mine = sound_notes(&st, g, p, s);
        let sel = selected(&st, g, p);
        Ok(!mine.is_empty() && mine.iter().all(|n| sel.contains(n)))
    })?;
    set_fn(lua, &tools, "hasEventsForNote", |lua, args: MultiValue| {
        let pitch = int(&args, 1).unwrap_or(-1);
        let st = state(lua);
        let st = st.borrow();
        Ok(focus(&st).is_some_and(|(g, p, s)| sound_notes(&st, g, p, s).iter().any(|n| n.pitch == pitch)))
    })?;
    set_fn(lua, &tools, "allEventsSelectedForNote", |lua, args: MultiValue| {
        let pitch = int(&args, 1).unwrap_or(-1);
        let st = state(lua);
        let st = st.borrow();
        let Some((g, p, s)) = focus(&st) else { return Ok(false) };
        let mine: Vec<Note> = sound_notes(&st, g, p, s).into_iter().filter(|n| n.pitch == pitch).collect();
        let sel = selected(&st, g, p);
        Ok(!mine.is_empty() && mine.iter().all(|n| sel.contains(n)))
    })?;
    // toggleEventSelectionBySound(App, Group, Sound): add all its notes, or (all
    // selected already) take them out; the sound gets the focus.
    set_fn(lua, &tools, "toggleEventSelectionBySound", |lua, args: MultiValue| {
        let Some(s) = sound_arg(lua, &args) else { return Ok(()) };
        let st = state(lua);
        let mut st = st.borrow_mut();
        let Some((g, p, _)) = focus(&st) else { return Ok(()) };
        let mine = sound_notes(&st, g, p, s);
        let mut sel = selected(&st, g, p);
        if !mine.is_empty() && mine.iter().all(|n| sel.contains(n)) {
            sel.retain(|n| n.sound != s);
        } else {
            sel.extend(mine.into_iter().filter(|n| !sel.contains(n)).collect::<Vec<_>>());
        }
        st.selection = Some((g, p, sel));
        st.focus.sound = s;
        Ok(())
    })?;
    set_fn(lua, &tools, "toggleEventSelectionByNote", |lua, args: MultiValue| {
        let pitch = int(&args, 1).unwrap_or(-1);
        let st = state(lua);
        let mut st = st.borrow_mut();
        let Some((g, p, s)) = focus(&st) else { return Ok(()) };
        let mine: Vec<Note> = sound_notes(&st, g, p, s).into_iter().filter(|n| n.pitch == pitch).collect();
        let mut sel = selected(&st, g, p);
        if !mine.is_empty() && mine.iter().all(|n| sel.contains(n)) {
            sel.retain(|n| !(n.sound == s && n.pitch == pitch));
        } else {
            sel.extend(mine.into_iter().filter(|n| !sel.contains(n)).collect::<Vec<_>>());
        }
        st.selection = Some((g, p, sel));
        Ok(())
    })?;
    // selectNoteEventsInTime(App, Sequence, From, To) and selectNoteEvents(…, Low,
    // High): the sound's notes starting from From to To (the last selected note's start,
    // so inclusive) become the selection.
    set_fn(lua, &tools, "selectNoteEventsInTime", |lua, args: MultiValue| {
        let Some((g, p, s)) = sequence_arg(lua, &args) else { return Ok(()) };
        let (a, b) = (int(&args, 2).unwrap_or(0), int(&args, 3).unwrap_or(i64::MAX));
        let sel: Vec<Note> =
            sound_notes(&state(lua).borrow(), g, p, s).into_iter().filter(|n| n.position >= a && n.position <= b).collect();
        select(lua, g, p, sel);
        Ok(())
    })?;
    set_fn(lua, &tools, "selectNoteEvents", |lua, args: MultiValue| {
        let Some((g, p, s)) = sequence_arg(lua, &args) else { return Ok(()) };
        let (a, b, lo, hi) =
            (int(&args, 2).unwrap_or(0), int(&args, 3).unwrap_or(i64::MAX), int(&args, 4).unwrap_or(0), int(&args, 5).unwrap_or(127));
        let sel: Vec<Note> = sound_notes(&state(lua).borrow(), g, p, s)
            .into_iter()
            .filter(|n| n.position >= a && n.position <= b && n.pitch >= lo && n.pitch <= hi)
            .collect();
        select(lua, g, p, sel);
        Ok(())
    })?;
    // The previous or next note of the focused sound after the selection (one note).
    fn step_to(st: &State, next: bool) -> Option<(usize, usize, Note)> {
        let (g, p, s) = focus(st)?;
        let mine = sound_notes(st, g, p, s);
        let sel = selected(st, g, p);
        let at = sel.iter().filter(|n| n.sound == s).filter_map(|n| mine.iter().position(|m| m == n));
        let i = if next { at.max().map_or(0, |i| i + 1) } else { at.min().and_then(|i| i.checked_sub(1)).unwrap_or(usize::MAX) };
        let i = if sel.is_empty() && !next { mine.len().checked_sub(1)? } else { i };
        mine.get(i).map(|n| (g, p, *n))
    }
    set_fn(lua, &tools, "canSelectPrevNextEvent", |lua, args: MultiValue| Ok(step_to(&state(lua).borrow(), boolean(&args, 1)).is_some()))?;
    set_fn(lua, &tools, "selectPrevNextEvent", |lua, args: MultiValue| {
        let to = step_to(&state(lua).borrow(), boolean(&args, 1));
        if let Some((g, p, n)) = to {
            select(lua, g, p, vec![n]);
        }
        Ok(())
    })?;
    set_fn(lua, &tools, "getNumSelectedNoteEventsInPatternRange", |lua, _: MultiValue| {
        let st = state(lua);
        let st = st.borrow();
        Ok(focus(&st).map_or(0, |(g, p, _)| {
            let (_, a, b) = notes(&st, g, p);
            selected(&st, g, p).iter().filter(|n| n.position >= a && n.position < b).count()
        }))
    })?;
    // The selection's position, length, pitch, velocity or probability; −1 when they
    // differ ("*").
    for (name, field) in [
        ("getSelectedNoteEventPosition", 0),
        ("getSelectedNoteEventsLength", 1),
        ("getSelectedNoteEventsPitch", 2),
        ("getSelectedNoteEventsVelocity", 3),
        ("getSelectedNoteEventsProbability", 4),
    ] {
        set_fn(lua, &tools, name, move |lua, _: MultiValue| {
            let st = state(lua);
            let st = st.borrow();
            let Some((g, p, _)) = focus(&st) else { return Ok(-1) };
            let vals: Vec<i64> = selected(&st, g, p)
                .iter()
                .map(|n| match field {
                    0 => n.position,
                    1 => n.length,
                    2 => n.pitch,
                    3 => n.velocity,
                    _ => n.probability,
                })
                .collect();
            Ok(match vals.first() {
                Some(&v) if vals.iter().all(|&x| x == v) => v,
                _ => -1,
            })
        })?;
    }

    // The knobs edit the selection only.
    set_fn(lua, &tools, "modifySelectedNoteEventsVelocity", |lua, args: MultiValue| {
        let d = int(&args, 1).unwrap_or(0);
        modify(lua, "Velocity", false, |n, _, _| Note { velocity: (n.velocity + d).clamp(1, 127), ..n });
        Ok(())
    })?;
    set_fn(lua, &tools, "modifySelectedNoteEventsPitch", |lua, args: MultiValue| {
        let d = int(&args, 1).unwrap_or(0);
        modify(lua, "Pitch", false, |n, _, _| Note { pitch: (n.pitch + d).clamp(0, 127), ..n });
        Ok(())
    })?;
    // Transposing, nudging, quantizing, clearing and copying (SHIFT + pads, the 4-D
    // encoder) work on what's shown when nothing is selected.
    set_fn(lua, &tools, "transposeNoteEvents", |lua, args: MultiValue| {
        let d = int(&args, 1).unwrap_or(0);
        modify(lua, "Transpose", true, |n, _, _| Note { pitch: (n.pitch + d).clamp(0, 127), ..n });
        Ok(())
    })?;
    set_fn(lua, &tools, "modifySelectedNoteEventsLength", |lua, args: MultiValue| {
        let d = int(&args, 1).unwrap_or(0);
        // No shorter than one step of the change.
        modify(lua, "Length", false, |n, _, _| Note { length: (n.length + d).max(d.abs().max(MIN_SNAP)), ..n });
        Ok(())
    })?;
    set_fn(lua, &tools, "modifySelectedNoteEventsProbability", |lua, args: MultiValue| {
        let d = int(&args, 1).unwrap_or(0);
        modify(lua, "Probability", false, |n, _, _| Note { probability: (n.probability + d).clamp(0, 100), ..n });
        Ok(())
    })?;
    // nudgeEventsInPatternRange(App, Direction, Fine, SelectedOnly): by the Nudge Grid
    // (Fine: the smallest step); notes pushed past an end come round from the other (the
    // knob passes SelectedOnly; SHIFT + pads and the 4-D encoder don't).
    set_fn(lua, &tools, "nudgeEventsInPatternRange", |lua, args: MultiValue| {
        let dir = int(&args, 1).unwrap_or(0).signum();
        let step = {
            let st = state(lua);
            let st = st.borrow();
            if boolean(&args, 2) || st.editor.grid_off { MIN_SNAP } else { st.nudge_grid.max(1) }
        };
        modify(lua, "Nudge", !boolean(&args, 3), |n, a, b| Note {
            position: a + (n.position + dir * step - a).rem_euclid((b - a).max(1)),
            ..n
        });
        Ok(())
    })?;
    // quantizeNoteEvents(App, Fifty): to the step grid, all the way or half of it.
    for name in ["quantizeNoteEvents", "quantizeNoteEventsInFocusedSound"] {
        let only_focus = name.ends_with("Sound");
        set_fn(lua, &tools, name, move |lua, args: MultiValue| {
            let half = boolean(&args, 1);
            let (grid, sound) = {
                let st = state(lua);
                let st = st.borrow();
                (st.step_grid.max(1), st.focus.sound)
            };
            modify(lua, "Quantize", true, |n, _, _| {
                if only_focus && n.sound != sound {
                    return n;
                }
                let target = (n.position + grid / 2).div_euclid(grid) * grid;
                let moved = if half { n.position + (target - n.position) / 2 } else { target };
                Note { position: moved, ..n }
            });
            Ok(())
        })?;
    }
    // Deleting: the selection (or every note), a sound's, the group's.
    set_fn(lua, &tools, "removeNoteAndAudioEvents", |lua, _: MultiValue| {
        let keyboard = keyboard_view(lua);
        let (g, p, gone) = {
            let st = state(lua);
            let st = st.borrow();
            let Some((g, p, _)) = focus(&st) else { return Ok(()) };
            (g, p, targets(&st, g, p, keyboard))
        };
        change(lua, "Clear", g, p, &gone, &[], Some(Vec::new()));
        Ok(())
    })?;
    // Clear Auto (SHIFT + pad 10): the focused sound's modulation in the focused pattern.
    set_fn(lua, &tools, "removeModulationEvents", |lua, _: MultiValue| {
        let Some((g, p, s)) = focus(&state(lua).borrow()) else { return Ok(()) };
        edit(lua, |d| d.clear_pattern_modulation(deckard_doc::Chain::Sound(g, s), p));
        Ok(())
    })?;
    set_fn(lua, &tools, "removeAllEventsFromFocusedSound", |lua, _: MultiValue| {
        let (g, p, gone) = {
            let st = state(lua);
            let st = st.borrow();
            let Some((g, p, s)) = focus(&st) else { return Ok(()) };
            (g, p, sound_notes(&st, g, p, s))
        };
        change(lua, "Clear Sound", g, p, &gone, &[], Some(Vec::new()));
        Ok(())
    })?;
    set_fn(lua, &tools, "removeAllEventsFromGroup", |lua, _: MultiValue| {
        let (g, p, gone) = {
            let st = state(lua);
            let st = st.borrow();
            let Some((g, p, _)) = focus(&st) else { return Ok(()) };
            (g, p, notes(&st, g, p).0)
        };
        change(lua, "Clear Group", g, p, &gone, &[], Some(Vec::new()));
        Ok(())
    })?;

    // Selected notes of a sequence in a time range.
    let seq_algos = methods(lua, impl_t, "NI.DATA.SequenceAlgorithms")?;
    fn in_range(lua: &Lua, args: &MultiValue) -> Vec<Note> {
        let Some((g, p, s)) = sequence_arg(lua, args) else { return Vec::new() };
        let (a, b) = (int(args, 1).unwrap_or(0), int(args, 2).unwrap_or(i64::MAX));
        let st = state(lua);
        let st = st.borrow();
        let mut v: Vec<Note> = selected(&st, g, p).into_iter().filter(|n| n.sound == s && n.position >= a && n.position < b).collect();
        v.sort_by_key(|n| (n.position, n.pitch));
        v
    }
    set_fn(lua, &seq_algos, "getNumSelectedNoteEventsInRange", |lua, args: MultiValue| Ok(in_range(lua, &args).len()))?;
    set_fn(lua, &seq_algos, "getNumSelectedAudioEventsInRange", |_, _: MultiValue| Ok(0))?;
    set_fn(lua, &seq_algos, "getFirstSelectedNoteEventInRangeTime", |lua, args: MultiValue| {
        Ok(in_range(lua, &args).first().map_or(INVALID_TICK, |n| n.position))
    })?;
    set_fn(lua, &seq_algos, "getFirstSelectedNoteEventInRangeLength", |lua, args: MultiValue| {
        Ok(in_range(lua, &args).first().map_or(INVALID_TICK, |n| n.length))
    })?;
    set_fn(lua, &seq_algos, "getLastSelectedNoteEventInRangeTime", |lua, args: MultiValue| {
        Ok(in_range(lua, &args).last().map_or(INVALID_TICK, |n| n.position))
    })?;
    // The index (in the sound's notes) of the one selected note in range.
    set_fn(lua, &seq_algos, "getSelectedNoteEventIndexInRange", |lua, args: MultiValue| {
        let Some(first) = in_range(lua, &args).into_iter().next() else { return Ok(-1) };
        let Some((g, p, s)) = sequence_arg(lua, &args) else { return Ok(-1) };
        Ok(sound_notes(&state(lua).borrow(), g, p, s).iter().position(|n| *n == first).map_or(-1, |i| i as i64))
    })?;

    // Undo and redo of the shared document: the `…Marker` calls a take (SHIFT + pads 1
    // and 2), the plain ones a single edit (pads 3 and 4). Its other calls (transaction
    // sequences) do nothing: each edit is its own undo step.
    let app = methods(lua, impl_t, "App")?;
    set_fn(lua, &app, "getTransactionManager", |lua, _: MultiValue| {
        let t = lua.create_table()?;
        for name in ["undoTransaction", "undoTransactionMarker", "redoTransaction", "redoTransactionMarker"] {
            let (redo, take) = (name.starts_with("redo"), name.ends_with("Marker"));
            t.set(
                name,
                lua.create_function(move |lua, _: MultiValue| {
                    let doc = state(lua).borrow().doc.clone();
                    if let Some(doc) = doc {
                        let mut d = doc.borrow_mut();
                        match (redo, take) {
                            (false, true) => d.undo(),
                            (true, true) => d.redo(),
                            (false, false) => d.undo_step(),
                            (true, false) => d.redo_step(),
                        };
                    }
                    Ok(())
                })?,
            )?;
        }
        for name in ["canUndo", "canUndoMarker", "canRedo", "canRedoMarker"] {
            let redo = name.starts_with("canRedo");
            t.set(
                name,
                lua.create_function(move |lua, _: MultiValue| {
                    let doc = state(lua).borrow().doc.clone();
                    Ok(doc.is_some_and(|d| {
                        let d = d.borrow();
                        if redo { d.redo_label().is_some() } else { d.undo_label().is_some() }
                    }))
                })?,
            )?;
        }
        let rest = lua.create_table()?;
        rest.set("__index", lua.create_function(|lua, _: MultiValue| lua.create_function(|_, _: MultiValue| Ok(())))?)?;
        t.set_metatable(Some(rest))?;
        Ok(t)
    })?;

    // Copy and paste notes (SHIFT + pads 11 and 12): the selection (or every note),
    // pasted at the same place in the focused pattern, selected.
    let clipboard = methods(lua, impl_t, "NI.DATA.NoteEventClipboardAccess")?;
    set_fn(lua, &clipboard, "copySelectedNoteEvents", |lua, _: MultiValue| {
        let keyboard = keyboard_view(lua);
        let st = state(lua);
        let mut st = st.borrow_mut();
        let Some((g, p, _)) = focus(&st) else { return Ok(()) };
        let start = notes(&st, g, p).1;
        st.note_clipboard = targets(&st, g, p, keyboard).into_iter().map(|n| Note { position: n.position - start, ..n }).collect();
        Ok(())
    })?;
    set_fn(lua, &clipboard, "pasteNoteEvents", |lua, _: MultiValue| {
        let (g, p, add) = {
            let st = state(lua);
            let st = st.borrow();
            let Some((g, p, _)) = focus(&st) else { return Ok(()) };
            let (have, start, _) = notes(&st, g, p);
            let add: Vec<Note> =
                st.note_clipboard.iter().map(|n| Note { position: n.position + start, ..*n }).filter(|n| !have.contains(n)).collect();
            (g, p, add)
        };
        change(lua, "Paste", g, p, &[], &add, Some(add.clone()));
        Ok(())
    })?;

    // Patterns: delete and duplicate.
    let group_access = methods(lua, impl_t, "NI.DATA.GroupAccess")?;
    for name in ["deletePattern", "deleteFocusPattern"] {
        set_fn(lua, &group_access, name, |lua, args: MultiValue| {
            let Some((g, p)) = pattern_arg(lua, &args) else { return Ok(()) };
            if edit(lua, |d| d.delete_pattern(g, p)).is_some() {
                let st = state(lua);
                let mut st = st.borrow_mut();
                let left = st.project().map_or(0, |pr| pr.groups.get(g).map_or(0, |g| g.patterns.len()));
                if st.focus.group == g {
                    st.focus.pattern = (left > 0).then(|| p.min(left - 1));
                }
                st.selection = None;
            }
            Ok(())
        })?;
    }
    set_fn(lua, &group_access, "duplicatePattern", |lua, args: MultiValue| {
        let Some((g, p)) = pattern_arg(lua, &args) else { return Ok(()) };
        if let Some(made) = edit(lua, |d| d.duplicate_pattern(g, p)) {
            focus_and_assign_pattern(lua, g, made);
        }
        Ok(())
    })?;
    Ok(())
}
