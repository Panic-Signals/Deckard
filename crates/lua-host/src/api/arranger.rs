//! The Song view on the controller (the Arranger page's SECTION tab, MK3 manual
//! "Arranging Sections in the Song View"): the sections NI's scripts focus, move,
//! lengthen and point at scenes, and how the displays' arranger is zoomed and
//! scrolled (the song's `ArrangerZoomHW` and `ArrangerOffsetHW`, knobs 5 and 6).
//!
//! Zoom is how many times the unzoomed view (four 4/4 bars across the right display,
//! as the pattern editor's) is magnified; the offset is in ticks from the song's start.
//! Until the scripts zoom, the whole song shows.

use super::data::{edit, find_arg, set_fn};
use super::{methods, state};
use crate::state::{ObjRef, Prop, State};
use deckard_model::PPQ;
use mlua::{Lua, MultiValue, Table, Value};

/// What the unzoomed view spans.
const FULL_VIEW: f64 = (16 * PPQ) as f64;
/// Zoomed in at most this far: a beat across the display.
const MAX_ZOOM: i64 = 16;

const ZOOM: &str = "ArrangerZoomHW";
const OFFSET: &str = "ArrangerOffsetHW";

/// The song's length as the arranger shows it: its sections, at least a bar.
pub(crate) fn song_length(st: &State) -> i64 {
    st.project().map_or(FULL_VIEW as i64, |p| p.arrangement_length().max(p.bar_length()))
}

/// The zoom and offset in effect.
fn zoom_offset(st: &State) -> (f64, i64) {
    let len = song_length(st);
    let fit = FULL_VIEW / len as f64;
    if st.arranger.zoom <= 0.0 {
        return (fit, 0);
    }
    let zoom = st.arranger.zoom.clamp(fit.min(MAX_ZOOM as f64), MAX_ZOOM as f64);
    let shown = (FULL_VIEW / zoom).round() as i64;
    (zoom, st.arranger.offset.clamp(0, (len - shown).max(0)))
}

/// What the right display shows of the song, in ticks.
pub(crate) fn window(st: &State) -> (i64, i64) {
    let (zoom, offset) = zoom_offset(st);
    (offset, offset + (FULL_VIEW / zoom).round() as i64)
}

/// The view's parameters, read.
pub(super) fn live(st: &State, owner: &ObjRef, name: &str) -> Option<Prop> {
    match (owner, name) {
        (ObjRef::Song, ZOOM) => Some(Prop::Num(zoom_offset(st).0)),
        (ObjRef::Song, OFFSET) => Some(Prop::Int(zoom_offset(st).1)),
        (ObjRef::Section(i), "AutoLength") => Some(Prop::Bool(st.project()?.sections.get(*i)?.auto)),
        _ => None,
    }
}

/// The view's parameters, written; false if `name` isn't one.
pub(super) fn write(st: &mut State, owner: &ObjRef, name: &str, v: &Prop) -> bool {
    let num = match v {
        Prop::Int(i) => *i as f64,
        Prop::Num(n) => *n,
        _ => return false,
    };
    match (owner, name) {
        (ObjRef::Song, ZOOM | OFFSET) => {
            // Start from what's shown, so setting one keeps the other.
            let (zoom, offset) = zoom_offset(st);
            st.arranger.zoom = zoom;
            st.arranger.offset = offset;
            if name == ZOOM {
                st.arranger.zoom = num.clamp(0.0001, MAX_ZOOM as f64);
            } else {
                st.arranger.offset = num.round().max(0.0) as i64;
            }
            true
        }
        _ => false,
    }
}

/// The largest value of a view parameter.
pub(super) fn max(name: &str) -> Option<i64> {
    (name == ZOOM).then_some(MAX_ZOOM)
}

/// Focus section `i` and the scene it plays.
fn focus_section(st: &mut State, i: usize) {
    let scene = st.project().and_then(|p| p.sections.get(i).map(|s| s.scene));
    if let Some(scene) = scene {
        st.focus.section = Some(i);
        if scene.is_some() {
            st.focus.scene = scene;
        }
    }
}

fn section_arg(lua: &Lua, args: &MultiValue) -> Option<usize> {
    find_arg(lua, args, |r| if let ObjRef::Section(s) = r { Some(s) } else { None })
}

/// The focused section, if there is one.
fn focused(lua: &Lua) -> Option<usize> {
    let st = state(lua);
    let st = st.borrow();
    let n = st.project()?.sections.len();
    st.focus.section.filter(|&s| s < n)
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let sh = methods(lua, impl_t, "NI.DATA.StateHelper")?;
    set_fn(lua, &sh, "getSongOverviewLength", |lua, _: MultiValue| Ok(song_length(&state(lua).borrow())))?;
    let song = methods(lua, impl_t, "Song")?;
    // The scripts redraw on these; the shapes are drawn fresh every frame anyway.
    set_fn(lua, &song, "isSongLengthChanged", |_, _: MultiValue| Ok(false))?;
    for (ty, name) in [("StateCache", "isGroupsChanged"), ("List", "isSortOrderChanged")] {
        set_fn(lua, &methods(lua, impl_t, ty)?, name, |_, _: MultiValue| Ok(false))?;
    }

    // scrollArrangerTimeHW(App, Song, Delta): the view moves `Delta` ticks (knob 6).
    let scrolling = methods(lua, impl_t, "NI.DATA.ScrollingAccess")?;
    set_fn(lua, &scrolling, "scrollArrangerTimeHW", |lua, (_, _, delta): (Value, Value, f64)| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        let offset = zoom_offset(&st).1 as f64 + delta;
        write(&mut st, &ObjRef::Song, OFFSET, &Prop::Num(offset));
        Ok(())
    })?;

    let algos = methods(lua, impl_t, "NI.DATA.SongAlgorithms")?;
    // No loop over sections, and no clips to convert sections into yet.
    set_fn(lua, &algos, "isSectionPartOfLoop", |_, _: MultiValue| Ok(false))?;
    set_fn(lua, &algos, "canConvertSectionToClips", |_, _: MultiValue| Ok(false))?;
    set_fn(lua, &algos, "getSectionLength", |lua, args: MultiValue| {
        let Some(i) = section_arg(lua, &args) else { return Ok(0) };
        Ok(state(lua).borrow().project().and_then(|p| p.sections.get(i).map(|s| s.length)).unwrap_or(0))
    })?;
    // A section's colour is its scene's.
    let section_algos = methods(lua, impl_t, "NI.DATA.SectionAlgorithms")?;
    set_fn(lua, &section_algos, "getColor", |lua, args: MultiValue| {
        let Some(i) = section_arg(lua, &args) else { return Ok(None) };
        let st = state(lua);
        let st = st.borrow();
        let Some(p) = st.project() else { return Ok(None) };
        let scene = p.sections.get(i).and_then(|s| s.scene).and_then(|s| p.scenes.get(s));
        Ok(scene.map(|s| super::data::scene_color(s.color)))
    })?;

    let menu = methods(lua, impl_t, "NI.GUI.MenuHelper")?;
    // getSceneNames(Song): a VectorString of the scenes' names (knob 2's list).
    set_fn(lua, &menu, "getSceneNames", |lua, _: MultiValue| {
        let names: Vec<String> = state(lua).borrow().project().map_or(Vec::new(), |p| p.scenes.iter().map(|s| s.name.clone()).collect());
        let n = names.len();
        let v = lua.create_table()?;
        v.set("size", lua.create_function(move |_, _: Value| Ok(n))?)?;
        v.set("at", lua.create_function(move |_, (_, i): (Value, usize)| Ok(names.get(i).cloned().unwrap_or_default()))?)?;
        Ok(v)
    })?;

    let arranger = methods(lua, impl_t, "NI.DATA.ArrangerAccess")?;
    set_fn(lua, &arranger, "getFocusedSectionSongPos", |lua, _: MultiValue| Ok(focused(lua).map_or(-1, |s| s as i64)))?;
    // createSectionIfKeyUnused(App, Index): a pad past the last section adds one, playing
    // the focused scene.
    set_fn(lua, &arranger, "createSectionIfKeyUnused", |lua, (_, index): (Value, i64)| {
        let (n, scene) = {
            let st = state(lua);
            let st = st.borrow();
            (st.project().map_or(0, |p| p.sections.len()), st.focus.scene.unwrap_or(0))
        };
        if index >= n as i64
            && let Some(at) = edit(lua, |d| d.add_section(scene))
        {
            focus_section(&mut state(lua).borrow_mut(), at);
        }
        Ok(())
    })?;

    // CREATE (button 5): a section after the focused one, playing its scene (the focused
    // scene without one).
    set_fn(lua, &arranger, "insertSection", |lua, args: MultiValue| {
        let after = section_arg(lua, &args);
        let scene = {
            let st = state(lua);
            let st = st.borrow();
            let from = after.and_then(|i| st.project()?.sections.get(i)?.scene);
            from.or(st.focus.scene).unwrap_or(0)
        };
        let Some(at) = edit(lua, |d| d.add_section(scene)) else { return Ok(()) };
        let to = after.map_or(at, |i| i + 1);
        if to != at {
            edit(lua, |d| d.move_section(at, to));
        }
        focus_section(&mut state(lua).borrow_mut(), to);
        Ok(())
    })?;

    let song_access = methods(lua, impl_t, "NI.DATA.SongAccess")?;
    // createGroup(App, Song, ...): the dim white group button after the last group, or
    // the software's "+" — a new empty group at the end, focused.
    set_fn(lua, &song_access, "createGroup", |lua, _: MultiValue| {
        if let Some(g) = edit(lua, |d| d.add_group()) {
            let st = state(lua);
            let mut st = st.borrow_mut();
            st.focus.group = g;
            st.focus.sound = 0;
            st.focus.pattern = None;
        }
        Ok(())
    })?;
    // DUPLICATE (button 4): a copy after it, with a copy of its scene (as Maschine 3).
    set_fn(lua, &song_access, "duplicateSection", |lua, args: MultiValue| {
        let Some(i) = section_arg(lua, &args) else { return Ok(()) };
        if let Some(at) = edit(lua, |d| d.duplicate_section(i)) {
            focus_section(&mut state(lua).borrow_mut(), at);
        }
        Ok(())
    })?;
    // LEFT / RIGHT: the section before or after.
    set_fn(lua, &song_access, "shiftSectionFocus", |lua, (_, _, next): (Value, Value, bool)| {
        if let Some(f) = focused(lua) {
            let to = if next { f + 1 } else { f.saturating_sub(1) };
            focus_section(&mut state(lua).borrow_mut(), to);
        }
        Ok(())
    })?;
    // Knob 1 (POSITION): the focused section swaps places with its neighbour.
    set_fn(lua, &song_access, "swapFocusedSection", |lua, (_, next): (Value, bool)| {
        let Some(f) = focused(lua) else { return Ok(()) };
        let to = if next { f + 1 } else { f.wrapping_sub(1) };
        if edit(lua, |d| d.move_section(f, to)).is_some() {
            state(lua).borrow_mut().focus.section = Some(to);
        }
        Ok(())
    })?;

    let section_access = methods(lua, impl_t, "NI.DATA.SectionAccess")?;
    // Knob 2 (SCENE): the scene the section plays, the next or previous one.
    set_fn(lua, &section_access, "shiftSceneOfSection", |lua, args: MultiValue| {
        let Some(i) = section_arg(lua, &args) else { return Ok(()) };
        let next = args.iter().rev().find_map(|v| if let Value::Boolean(b) = v { Some(*b) } else { None }).unwrap_or(true);
        let (scene, scenes) = {
            let st = state(lua);
            let st = st.borrow();
            let Some(p) = st.project() else { return Ok(()) };
            (p.sections.get(i).and_then(|s| s.scene), p.scenes.len())
        };
        let to = match (scene, next) {
            (Some(s), true) if s + 1 < scenes => s + 1,
            (Some(s), false) if s > 0 => s - 1,
            (None, _) if scenes > 0 => 0,
            _ => return Ok(()),
        };
        if edit(lua, |d| d.set_section_scene(i, to)).is_some() && focused(lua) == Some(i) {
            state(lua).borrow_mut().focus.scene = Some(to);
        }
        Ok(())
    })?;
    // Knob 4 (LENGTH): a bar longer or shorter, a beat with SHIFT.
    set_fn(lua, &section_access, "incrementFocusSectionLength", |lua, (_, dir, fine): (Value, i64, Option<bool>)| {
        let Some(i) = focused(lua) else { return Ok(()) };
        let (len, step) = {
            let st = state(lua);
            let st = st.borrow();
            let Some(p) = st.project() else { return Ok(()) };
            let bar = p.bar_length();
            (p.sections[i].length, if fine.unwrap_or(false) { (bar / p.numerator.max(1)).max(1) } else { bar })
        };
        // From a length off the grid, to the next step of it.
        let to = if dir > 0 { (len / step + 1) * step } else { ((len - 1) / step) * step };
        if to >= step {
            edit(lua, |d| d.set_section_length(i, Some(to)));
        }
        Ok(())
    })?;
    Ok(())
}

/// The Song view focuses a section (the one playing the focused scene, else the first)
/// and keeps it on the right display: a newly focused section out of view scrolls in.
pub(crate) fn focus_a_section(st: &mut State) {
    let Some(p) = st.project() else { return };
    let n = p.sections.len();
    if n > 0 && !st.focus.section.is_some_and(|s| s < n) {
        let i = p.sections.iter().position(|s| s.scene.is_some() && s.scene == st.focus.scene).unwrap_or(0);
        drop(p);
        focus_section(st, i);
    } else {
        drop(p);
    }
    let Some(f) = st.focus.section else { return };
    if st.arranger.shown_section == Some(f) {
        return;
    }
    st.arranger.shown_section = Some(f);
    let Some((a, b)) = st.project().and_then(|p| p.sections.get(f).map(|s| (p.section_start(f), p.section_start(f) + s.length))) else {
        return;
    };
    let (t0, t1) = window(st);
    if a < t0 || (b > t1 && a > t0) {
        write(st, &ObjRef::Song, OFFSET, &Prop::Int(a));
    }
}
