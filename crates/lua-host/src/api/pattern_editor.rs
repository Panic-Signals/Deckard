//! The controller's pattern editor view: how far it's zoomed and scrolled, and how tall
//! its sound lanes are. NI's scripts zoom and scroll it through the song's pattern-editor
//! parameters (`getPatternEditorZoomParameter`, `…OffsetParameter`, the Events page's
//! knobs 5 and 6, the Step page's 16-step pages) and size its lanes with the group's
//! `PatternEditorVerticalZoomHW` (knob 7), using the widget's time grid
//! (`getInnerWidth`, `getTimeGrid():getTicksPerPixelUnscaled()`) for the arithmetic.
//!
//! Zoom is how many times the unzoomed view (four 4/4 bars across the right display)
//! is magnified; the offset is in ticks from the pattern's start. A pattern newly in
//! focus shows whole until the scripts zoom.
//!
//! The song's `PatternEditorSnapEnabled` (the Events page's GRID) lives here too: with it
//! off, edits move by the smallest step. So does the piano roll's pitch scroll (Keyboard
//! mode's knob 8, `scrollPianorollHW`): two octaves shown, from `pitch_low`.

use super::data::set_fn;
use super::{methods, object, state};
use crate::state::{ObjRef, Prop, State};
use deckard_model::PPQ;
use mlua::{Lua, MultiValue, Table, Value};

/// The detail (right) display's editor width and the overview's, in pixels (the style
/// sheet's `$PatternEditorWidthR` and `L`).
pub(crate) const DETAIL_WIDTH: i64 = 453;
pub(crate) const OVERVIEW_WIDTH: i64 = 480;
/// What the unzoomed view spans.
const FULL_VIEW: f64 = (16 * PPQ) as f64;
/// Zoomed in at most this far (a beat across the display at 16).
const MAX_ZOOM: i64 = 64;

const ZOOM: &str = "PatternEditorZoomHW";
const OFFSET: &str = "PatternEditorOffsetHW";
const FOCUS_OFFSET: &str = "PatternEditorFocusOffsetHW";
const LANES: &str = "PatternEditorVerticalZoomHW";
const SNAP: &str = "PatternEditorSnapEnabled";
const PITCH_SCROLL: &str = "PianorollOffsetYHW";
/// The Step Grid and the Nudge Grid (the Grid page's STEP: its pads and knob 4).
const STEP_SNAP: &str = "PatternEditorSnap";
const NUDGE_SNAP: &str = "PatternEditorNudgeSnap";

/// Maschine's Step Grid values, in its order, with their length in ticks of 4/4 (a bar
/// is the song's).
pub const STEP_GRIDS: [(&str, i64); 15] = [
    ("1 Bar", 4 * PPQ),
    ("1/2", 2 * PPQ),
    ("1/4", PPQ),
    ("1/8", PPQ / 2),
    ("1/16", PPQ / 4),
    ("1/32", PPQ / 8),
    ("1/64", PPQ / 16),
    ("1/128", PPQ / 32),
    ("1 Bar T", 8 * PPQ / 3),
    ("1/2T", 4 * PPQ / 3),
    ("1/4T", 2 * PPQ / 3),
    ("1/8T", PPQ / 3),
    ("1/16T", PPQ / 6),
    ("1/32T", PPQ / 12),
    ("1/64T", PPQ / 24),
];

/// The Nudge Grid's values: the step grid over 1, 2, 4, 8, 16.
pub const NUDGE_GRIDS: [&str; 5] = ["Step", "Step/2", "Step/4", "Step/8", "Step/16"];

/// The song's Perform Grid (scene and section changes) and Arrange Grid (pattern and
/// section lengths), with the Arrange Grid's switch and Quick lengths: the Grid page's
/// PERFORM and ARRANGE. Kept in the project (`perform_grid`, …).
const PERFORM_GRID: &str = "PerformGrid";
const ARRANGE_GRID: &str = "ArrangeGrid";
const ARRANGE_SNAP: &str = "ArrangeGridSnap";
const QUICK: &str = "PatternLengthQuick";

/// The Perform Grid's values, as Maschine's header lists them.
pub const PERFORM_GRIDS: [&str; 7] = ["Off", "1 Bar", "1/2", "1/4", "1/8", "1/16", "Section"];
/// The Arrange Grid's.
pub const ARRANGE_GRIDS: [&str; 5] = ["1 Bar", "1/2", "1/4", "1/8", "1/16"];

/// The project field of a song grid parameter, its default, and whether it's a switch.
fn song_grid(name: &str) -> Option<(&'static str, i64, bool)> {
    Some(match name {
        PERFORM_GRID => ("perform_grid", 5, false),
        ARRANGE_GRID => ("arrange_grid", 0, false),
        ARRANGE_SNAP => ("arrange_grid_snap", 1, true),
        QUICK => ("pattern_length_quick", 1, true),
        _ => return None,
    })
}

/// The step grid's place in [`STEP_GRIDS`] (the nearest).
pub fn step_index(ticks: i64) -> usize {
    (0..STEP_GRIDS.len()).min_by_key(|&i| (STEP_GRIDS[i].1 - ticks).abs()).unwrap_or(4)
}

/// The length of [`STEP_GRIDS`] value `i` in a song whose bar is `bar` ticks (1 Bar and
/// 1 Bar T are the song's bar).
pub fn step_ticks(i: usize, bar: i64) -> i64 {
    match i.min(STEP_GRIDS.len() - 1) {
        0 => bar,
        8 => bar * 2 / 3,
        i => STEP_GRIDS[i].1,
    }
}

/// Item `i` of the Step or Nudge Grid's values (`getAsString(i)`).
pub(super) fn list_string(owner: &ObjRef, name: &str, i: i64) -> Option<String> {
    let i = usize::try_from(i).ok()?;
    match (owner, name) {
        (ObjRef::Song, STEP_SNAP) => STEP_GRIDS.get(i).map(|g| g.0.to_string()),
        (ObjRef::Song, NUDGE_SNAP) => NUDGE_GRIDS.get(i).map(|g| g.to_string()),
        (ObjRef::Song, PERFORM_GRID) => PERFORM_GRIDS.get(i).map(|g| g.to_string()),
        (ObjRef::Song, ARRANGE_GRID) => ARRANGE_GRIDS.get(i).map(|g| g.to_string()),
        _ => None,
    }
}

/// Whether a parameter is the Step or Nudge Grid (choices).
pub(super) fn is_enum(owner: &ObjRef, name: &str) -> bool {
    matches!((owner, name), (ObjRef::Song, STEP_SNAP | NUDGE_SNAP | PERFORM_GRID | ARRANGE_GRID))
}
/// The piano roll's rows: two octaves.
pub(crate) const PITCH_ROWS: i64 = 24;

/// Ticks per pixel of the unzoomed view.
fn ticks_per_pixel() -> f64 {
    FULL_VIEW / DETAIL_WIDTH as f64
}

/// The focused pattern's start and length.
fn pattern_span(st: &State) -> Option<(i64, i64)> {
    let (g, p) = (st.focus.group, st.focus.pattern?);
    let pr = st.project()?;
    let pat = pr.groups.get(g)?.patterns.get(p)?;
    let len = if pat.length > 0 { pat.length } else { pr.bar_length() };
    Some((pat.start, len.max(1)))
}

/// The zoom and offset in effect: the view as the scripts set it for the focused
/// pattern, else the whole pattern.
fn zoom_offset(st: &State) -> (f64, i64) {
    let len = pattern_span(st).map_or(FULL_VIEW as i64, |(_, l)| l);
    let fit = FULL_VIEW / len as f64;
    let focus = st.focus.pattern.map(|p| (st.focus.group, p));
    if st.editor.pattern != focus || st.editor.zoom <= 0.0 {
        return (fit, 0);
    }
    let zoom = st.editor.zoom.clamp(fit, MAX_ZOOM as f64);
    let shown = (FULL_VIEW / zoom).round() as i64;
    (zoom, st.editor.offset.clamp(0, (len - shown).max(0)))
}

/// What the right display shows of the focused pattern, in pattern time.
pub(crate) fn window(st: &State) -> Option<(i64, i64)> {
    let (start, _) = pattern_span(st)?;
    let (zoom, offset) = zoom_offset(st);
    let t0 = start + offset;
    Some((t0, t0 + (FULL_VIEW / zoom).round() as i64))
}

/// Whether the sound lanes are doubled in height (knob 7).
pub(crate) fn tall_lanes(st: &State) -> bool {
    st.editor.tall_lanes
}

/// The focused sound's piano-roll scroll, if it was scrolled.
fn scrolled(st: &State) -> Option<f64> {
    match st.editor.pitch_offset {
        Some((g, s, off)) if (g, s) == (st.focus.group, st.focus.sound) => Some(off),
        _ => None,
    }
}

/// The piano roll's lowest pitch: as scrolled (knob 8), else two octaves centred on
/// the focused sound's notes (around `root` when it has none).
pub(crate) fn pitch_low(st: &State, root: i64) -> i64 {
    let top = 128 - PITCH_ROWS;
    if let Some(off) = scrolled(st) {
        return ((off / 100.0).round() as i64).clamp(0, top);
    }
    let pitches: Vec<i64> = match (st.focus.pattern, st.project()) {
        (Some(p), Some(pr)) => pr
            .groups
            .get(st.focus.group)
            .and_then(|g| g.patterns.get(p))
            .map(|pat| pat.notes.iter().filter(|n| n.sound == st.focus.sound).map(|n| n.pitch).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let (lo, hi) = match (pitches.iter().min(), pitches.iter().max()) {
        (Some(&lo), Some(&hi)) => (lo, hi),
        _ => (root, root + 12),
    };
    (lo - (PITCH_ROWS - (hi - lo + 1)).max(0) / 2).clamp(0, top)
}

/// The view's parameters, read.
pub(super) fn live(st: &State, owner: &ObjRef, name: &str) -> Option<Prop> {
    if let (ObjRef::Song, Some((field, default, switch))) = (owner, song_grid(name)) {
        let v = st.doc.as_ref().and_then(|d| d.borrow().num(deckard_doc::Target::Song, field)).map_or(default, |v| v as i64);
        return Some(if switch { Prop::Bool(v != 0) } else { Prop::Int(v) });
    }
    match (owner, name) {
        (ObjRef::Song, ZOOM) => Some(Prop::Num(zoom_offset(st).0)),
        (ObjRef::Song, OFFSET | FOCUS_OFFSET) => Some(Prop::Int(zoom_offset(st).1)),
        (ObjRef::Group(_), LANES) => Some(Prop::Int(i64::from(st.editor.tall_lanes))),
        (ObjRef::Song, SNAP) => Some(Prop::Bool(!st.editor.grid_off)),
        (ObjRef::Song, STEP_SNAP) => Some(Prop::Int(step_index(st.step_grid) as i64)),
        (ObjRef::Song, NUDGE_SNAP) => Some(Prop::Int(st.nudge_div as i64)),
        (ObjRef::Sound(..), PITCH_SCROLL) => Some(Prop::Num(scrolled(st).unwrap_or_else(|| pitch_low(st, 60) as f64 * 100.0))),
        _ => None,
    }
}

/// The view's parameters, written; false if `name` isn't one.
pub(super) fn write(st: &mut State, owner: &ObjRef, name: &str, v: &Prop) -> bool {
    if let (ObjRef::Song, SNAP, Prop::Bool(on)) = (owner, name, v) {
        st.editor.grid_off = !on;
        return true;
    }
    let num = match v {
        Prop::Int(i) => *i as f64,
        Prop::Num(n) => *n,
        Prop::Bool(b) => f64::from(u8::from(*b)),
        _ => return false,
    };
    if let (ObjRef::Song, Some((field, ..))) = (owner, song_grid(name)) {
        let Some(doc) = st.doc.clone() else { return false };
        let max = self::max(name).unwrap_or(1);
        let _ =
            doc.borrow_mut().set("Grid", deckard_doc::Target::Song, field, mxprj::decode::Value::Int((num.round() as i64).clamp(0, max)));
        return true;
    }
    match (owner, name) {
        (ObjRef::Song, STEP_SNAP) => {
            let bar = st.project().map_or(4 * PPQ, |p| p.bar_length());
            st.step_grid = step_ticks(num.round().max(0.0) as usize, bar);
            st.editor.grid_off = false;
            st.nudge_grid = (st.step_grid >> st.nudge_div).max(1);
            st.grid_changed = true;
            true
        }
        (ObjRef::Song, NUDGE_SNAP) => {
            st.nudge_div = (num.round().max(0.0) as usize).min(NUDGE_GRIDS.len() - 1);
            st.nudge_grid = (st.step_grid >> st.nudge_div).max(1);
            st.grid_changed = true;
            true
        }
        (ObjRef::Song, ZOOM | OFFSET | FOCUS_OFFSET) => {
            // Start from what's shown, so setting one keeps the other.
            let (zoom, offset) = zoom_offset(st);
            st.editor.pattern = st.focus.pattern.map(|p| (st.focus.group, p));
            st.editor.zoom = zoom;
            st.editor.offset = offset;
            if name == ZOOM {
                st.editor.zoom = num.clamp(0.01, MAX_ZOOM as f64);
            } else {
                st.editor.offset = num.round().max(0.0) as i64;
            }
            true
        }
        (ObjRef::Group(_), LANES) => {
            st.editor.tall_lanes = num >= 0.5;
            true
        }
        _ => false,
    }
}

/// The largest value of a view parameter.
pub(super) fn max(name: &str) -> Option<i64> {
    match name {
        ZOOM => Some(MAX_ZOOM),
        LANES => Some(1),
        STEP_SNAP => Some(STEP_GRIDS.len() as i64 - 1),
        NUDGE_SNAP => Some(NUDGE_GRIDS.len() as i64 - 1),
        PERFORM_GRID => Some(PERFORM_GRIDS.len() as i64 - 1),
        ARRANGE_GRID => Some(ARRANGE_GRIDS.len() as i64 - 1),
        _ => None,
    }
}

fn song_param(lua: &Lua, name: &str) -> mlua::Result<Value> {
    let owner = state(lua).borrow_mut().objects.intern(ObjRef::Song);
    object(lua, ObjRef::Param { owner, name: name.to_string() })
}

/// Whether a widget is the overview editor (the one given an overview source), and
/// whether it's the Song view's arranger (whose overview is the one marked
/// `setHWScreen(true)`) rather than the pattern editor.
fn kind(lua: &Lua, this: &Value) -> (bool, bool) {
    let Some(id) = super::object_id(this) else { return (false, false) };
    let st = state(lua);
    let st = st.borrow();
    let Some(w) = st.widgets.get(id) else { return (false, false) };
    if w.kind == "ArrangerPanel" {
        return (matches!(w.props.get("HWScreen"), Some(Prop::Bool(true))), true);
    }
    (matches!(w.props.get("OverviewSource"), Some(Prop::Widget(_))), false)
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let sh = methods(lua, impl_t, "NI.DATA.StateHelper")?;
    set_fn(lua, &sh, "getPatternEditorZoomParameter", |lua, _: MultiValue| song_param(lua, ZOOM))?;
    set_fn(lua, &sh, "getPatternEditorOffsetParameter", |lua, _: MultiValue| song_param(lua, OFFSET))?;
    let song = methods(lua, impl_t, "Song")?;
    set_fn(lua, &song, "getPatternEditorFocusOffsetParameterHW", |lua, _: MultiValue| song_param(lua, FOCUS_OFFSET))?;
    let scrolling = methods(lua, impl_t, "NI.DATA.ScrollingAccess")?;
    // scrollPianorollHW(App, Sound, Delta): knob 8 in Keyboard mode moves the piano roll
    // `Delta` hundredths of a semitone up (500 a knob turn's unit).
    set_fn(lua, &scrolling, "scrollPianorollHW", |lua, (_, _, delta): (Value, Value, f64)| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        let from = scrolled(&st).unwrap_or_else(|| pitch_low(&st, 60) as f64 * 100.0);
        let to = (from + delta).clamp(0.0, (128 - PITCH_ROWS) as f64 * 100.0);
        st.editor.pitch_offset = Some((st.focus.group, st.focus.sound, to));
        Ok(())
    })?;
    // scrollPatternEditorTimeHW(App, Delta): the view moves `Delta` ticks.
    set_fn(lua, &scrolling, "scrollPatternEditorTimeHW", |lua, (_, delta): (Value, f64)| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        let offset = zoom_offset(&st).1 as f64 + delta;
        write(&mut st, &ObjRef::Song, OFFSET, &Prop::Num(offset));
        Ok(())
    })?;

    // The editor widget's size and time grid, for the scripts' zoom arithmetic.
    let widget = methods(lua, impl_t, "Widget")?;
    set_fn(lua, &widget, "getInnerWidth", |lua, this: Value| Ok(if kind(lua, &this).0 { OVERVIEW_WIDTH } else { DETAIL_WIDTH }))?;
    set_fn(lua, &widget, "getTimeGrid", |lua, this: Value| {
        let (overview, arranger) = kind(lua, &this);
        let t = lua.create_table()?;
        t.set("getTicksPerPixelUnscaled", lua.create_function(|_, _: MultiValue| Ok(ticks_per_pixel()))?)?;
        // calcTicksForPixels(px): ticks across `px` pixels as zoomed (the overview: the
        // whole pattern across it).
        t.set(
            "calcTicksForPixels",
            lua.create_function(move |lua, (_, px): (Value, f64)| {
                let st = state(lua);
                let st = st.borrow();
                Ok(if arranger {
                    if overview {
                        px * super::arranger::song_length(&st) as f64 / OVERVIEW_WIDTH as f64
                    } else {
                        let (t0, t1) = super::arranger::window(&st);
                        px * (t1 - t0) as f64 / DETAIL_WIDTH as f64
                    }
                } else if overview {
                    let len = pattern_span(&st).map_or(FULL_VIEW as i64, |(_, l)| l);
                    px * len as f64 / OVERVIEW_WIDTH as f64
                } else {
                    px * ticks_per_pixel() / zoom_offset(&st).0
                })
            })?,
        )?;
        t.set(
            "getPlayPosition",
            lua.create_function(|lua, _: MultiValue| {
                let st = state(lua);
                let st = st.borrow();
                Ok(super::events::play_tick(&st).unwrap_or(0))
            })?,
        )?;
        t.set("isPlayInfoValid", lua.create_function(|lua, _: MultiValue| Ok(super::events::play_tick(&state(lua).borrow()).is_some()))?)?;
        Ok(t)
    })?;
    for name in ["updateOverviewTimeGrid", "resetViewport", "showScrollbar", "showTimeline"] {
        set_fn(lua, &widget, name, |_, this: Value| Ok(this))?;
    }
    Ok(())
}
