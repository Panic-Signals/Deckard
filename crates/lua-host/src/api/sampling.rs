//! The Sampling page's data as the app has it: the knobs of each tab and the focused
//! zone's sample, its slices and the Sampler's zones ([`State::sample`]).
//!
//! - Knobs: `App:getSamplingParameters():getPagesForTab(tab)` gives the tab's pages of
//!   up to four ([`State::sampling_pages`]); LEFT / RIGHT page through them
//!   (`SamplingParametersAccess.focusPageOffset`). The SLICE tab's script picks its own
//!   (`getSliceModeParameter`, …: [`State::slice_knobs`]). Turning one leaves the new
//!   value in [`State::sampling_edits`] for the app.
//! - The sample: `getFocusSample`, `getFocusTransactionSample` and its slice container,
//!   the Sampler's zones (`getZones`, `getFocusZone`), and the wave display's zoom and
//!   offset (`SampleOwnerViewHelper.makeFocusView`, knobs 5 and 6).
//! - What the page asks of them (`FocusSampleAccess`, `TransactionSampleAccess`,
//!   `SamplerAccess`, `ZoneAccess`, `SamplePrehearAccess`) is left in
//!   [`State::sampling_calls`].

use super::data::set_fn;
use super::{methods, object, object_ref, state};
use crate::state::{ObjRef, Prop, SamplingCall, SamplingKnob, State};
use mlua::{Lua, MultiValue, Table, Value};

const FOCUS_PAGE: &str = "FocusPageIndex";
const ZOOM: &str = "Zoom";
const OFFSET: &str = "Offset";
const FOCUS_FRAME: &str = "FocusFrame";
const STRETCH_VISIBLE: &str = "TimeStretchVisible";
/// The shortest stretch of a sample shown (frames), as the app's Sampling view.
const MIN_SPAN: f64 = 64.0;
/// `SamplingScreenMode.RECORD`.
const RECORD_TAB: usize = 0;
/// `SamplingHelper.EditFunc…`: the functions the app has.
const EDIT_FUNCTIONS: [(&str, usize); 11] = [
    ("truncate", 1),
    ("normalize", 2),
    ("reverse", 3),
    ("fadeIn", 4),
    ("fadeOut", 5),
    ("removeDC", 6),
    ("silence", 7),
    ("cut", 8),
    ("copy", 9),
    ("paste", 10),
    ("duplicate", 11),
];

fn tab(t: i64) -> Option<usize> {
    usize::try_from(t).ok().filter(|&t| t < 4)
}

fn call(lua: &Lua, c: SamplingCall) {
    state(lua).borrow_mut().sampling_calls.push(c);
}

fn has_sample(lua: &Lua) -> bool {
    state(lua).borrow().sample.is_some()
}

/// `obj` while there's a sample, else nil.
fn with_sample(lua: &Lua, obj: ObjRef) -> mlua::Result<Value> {
    if has_sample(lua) { object(lua, obj) } else { Ok(Value::Nil) }
}

/// A knob's parameter, by key (nil if the app gave no such knob).
fn knob_param(lua: &Lua, key: &str) -> mlua::Result<Value> {
    let st = state(lua);
    let found = knob_by_key(&st.borrow(), key).is_some();
    if !found {
        return Ok(Value::Nil);
    }
    let owner = st.borrow_mut().objects.intern(ObjRef::SamplingParameters);
    object(lua, ObjRef::Param { owner, name: key.to_string() })
}

fn param(lua: &Lua, owner: ObjRef, name: &str) -> mlua::Result<Value> {
    let owner = state(lua).borrow_mut().objects.intern(owner);
    object(lua, ObjRef::Param { owner, name: name.to_string() })
}

/// The focused slice (`SliceContainer:getFocusSliceIndex`), `None` without one.
fn focus_slice(st: &State) -> Option<usize> {
    st.sample.as_ref().and_then(|s| s.focus_slice.filter(|&i| i < s.slices.len()))
}

/// Slice `i`'s frames (start, end).
fn slice_range(st: &State, i: usize) -> Option<(usize, usize)> {
    let s = st.sample.as_ref()?;
    let a = *s.slices.get(i)?;
    Some((a, s.slices.get(i + 1).copied().unwrap_or(s.frames)))
}

/// A frame range object (`getMin`, `getMax`), or a slice (`getStartFrame`, `getEndFrame`).
fn range(lua: &Lua, (a, b): (usize, usize)) -> mlua::Result<Value> {
    let f: mlua::Function = lua
        .load(
            "return function(a, b) return { getMin = function() return a end, getMax = function() return b end, \
             getStartFrame = function() return a end, getEndFrame = function() return b end } end",
        )
        .eval()?;
    f.call((a, b))
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    // Knob pages.
    let app = methods(lua, impl_t, "App")?;
    set_fn(lua, &app, "getSamplingParameters", |lua, _: MultiValue| object(lua, ObjRef::SamplingParameters))?;
    let sp = methods(lua, impl_t, "SamplingParameters")?;
    set_fn(lua, &sp, "getPagesForTab", |lua, (_, t): (Value, i64)| match tab(t) {
        Some(t) if !state(lua).borrow().sampling_pages[t].is_empty() => object(lua, ObjRef::KnobPages(t)),
        _ => Ok(Value::Nil),
    })?;
    let pages = methods(lua, impl_t, "SamplingParameterPages")?;
    set_fn(lua, &pages, "getNumPages", |lua, this: Value| {
        let Some(ObjRef::KnobPages(t)) = object_ref(lua, &this) else { return Ok(0) };
        Ok(state(lua).borrow().sampling_pages[t].len() as i64)
    })?;
    set_fn(lua, &pages, "getFocusPage", |lua, this: Value| {
        let Some(ObjRef::KnobPages(t)) = object_ref(lua, &this) else { return Ok(Value::Nil) };
        let page = {
            let st = state(lua);
            let st = st.borrow();
            (st.sampling_page[t] < st.sampling_pages[t].len()).then_some(st.sampling_page[t])
        };
        page.map_or(Ok(Value::Nil), |i| object(lua, ObjRef::KnobPage(t, i)))
    })?;
    set_fn(lua, &pages, "getFocusPageIndex", |lua, this: Value| {
        let Some(ObjRef::KnobPages(t)) = object_ref(lua, &this) else { return Ok(Value::Nil) };
        param(lua, ObjRef::KnobPages(t), FOCUS_PAGE)
    })?;
    let page = methods(lua, impl_t, "SamplingParameterPage")?;
    set_fn(lua, &page, "getPageName", |lua, this: Value| {
        let st = state(lua);
        let st = st.borrow();
        Ok(match object_ref(lua, &this) {
            Some(ObjRef::KnobPage(t, i)) => st.sampling_pages[t].get(i).map_or(String::new(), |p| p.name.clone()),
            _ => String::new(),
        })
    })?;
    set_fn(lua, &page, "getParameter", |lua, (this, k): (Value, i64)| {
        let key = {
            let st = state(lua);
            let st = st.borrow();
            match object_ref(lua, &this) {
                Some(ObjRef::KnobPage(t, i)) => {
                    st.sampling_pages[t].get(i).and_then(|p| p.knobs.get(usize::try_from(k).ok()?)).map(|k| k.key.clone())
                }
                _ => None,
            }
        };
        key.map_or(Ok(Value::Nil), |key| knob_param(lua, &key))
    })?;
    // The app leaves its recorder's settings alone while it records.
    set_fn(lua, &page, "isDisabledWhenRecording", |lua, this: Value| {
        Ok(matches!(object_ref(lua, &this), Some(ObjRef::KnobPage(RECORD_TAB, _))))
    })?;
    let spa = methods(lua, impl_t, "NI.DATA.SamplingParametersAccess")?;
    set_fn(lua, &spa, "focusPageOffset", |lua, (_, t, by): (Value, i64, i64)| {
        if let Some(t) = tab(t) {
            let st = state(lua);
            let mut st = st.borrow_mut();
            let last = st.sampling_pages[t].len().saturating_sub(1) as i64;
            st.sampling_page[t] = (st.sampling_page[t] as i64 + by).clamp(0, last) as usize;
        }
        Ok(())
    })?;
    set_fn(lua, &spa, "showTimeStretchSettings", |_, _: MultiValue| Ok(()))?;

    // The focused sample.
    let sh = methods(lua, impl_t, "NI.DATA.StateHelper")?;
    set_fn(lua, &sh, "getFocusSample", |lua, _: MultiValue| with_sample(lua, ObjRef::Sample))?;
    set_fn(lua, &sh, "getFocusTransactionSample", |lua, _: MultiValue| with_sample(lua, ObjRef::TransactionSample))?;
    set_fn(lua, &sh, "getFocusSampleLength", |lua, _: MultiValue| Ok(state(lua).borrow().sample.as_ref().map_or(0, |s| s.frames as i64)))?;
    set_fn(lua, &sh, "getFocusSampleLengthAsString", |lua, _: MultiValue| {
        Ok(state(lua).borrow().sample.as_ref().map_or(String::new(), |s| length_text(s.frames, s.rate)))
    })?;
    set_fn(lua, &sh, "getFocusZone", |lua, _: MultiValue| {
        let any = state(lua).borrow().sample.as_ref().is_some_and(|s| !s.zones.is_empty());
        if any { object(lua, ObjRef::Zone) } else { Ok(Value::Nil) }
    })?;
    let tsa = methods(lua, impl_t, "NI.DATA.TransactionSampleAlgorithms")?;
    set_fn(lua, &tsa, "getName", |lua, _: MultiValue| Ok(state(lua).borrow().sample.as_ref().map_or(String::new(), |s| s.name.clone())))?;
    let ts = methods(lua, impl_t, "TransactionSample")?;
    set_fn(lua, &ts, "getSliceContainer", |lua, _: MultiValue| with_sample(lua, ObjRef::Slices))?;
    set_fn(lua, &ts, "getFileLocation", |lua, _: MultiValue| {
        Ok(state(lua).borrow().sample.as_ref().map_or(String::new(), |s| s.path.clone()))
    })?;

    // Its slices.
    let slices = methods(lua, impl_t, "SliceContainer")?;
    set_fn(lua, &slices, "getNumSlices", |lua, _: MultiValue| {
        Ok(state(lua).borrow().sample.as_ref().map_or(0, |s| s.slices.len() as i64))
    })?;
    set_fn(lua, &slices, "getFocusSliceIndex", |lua, _: MultiValue| Ok(focus_slice(&state(lua).borrow()).map_or(-1, |i| i as i64)))?;
    set_fn(lua, &slices, "isFocusSliceChanged", |_, _: MultiValue| Ok(true))?;
    set_fn(lua, &slices, "getFocusSliceFrameRange", |lua, _: MultiValue| {
        let r = {
            let st = state(lua);
            let st = st.borrow();
            focus_slice(&st).and_then(|i| slice_range(&st, i))
        };
        range(lua, r.unwrap_or((0, 0)))
    })?;
    set_fn(lua, &slices, "getSliceByIndex", |lua, (_, i): (Value, i64)| {
        let r = usize::try_from(i).ok().and_then(|i| slice_range(&state(lua).borrow(), i));
        r.map_or(Ok(Value::Nil), |r| range(lua, r))
    })?;
    // The analysis is done: Detect's density and zero-crossing knobs show.
    set_fn(lua, &slices, "getAnalysisState", |lua, _: MultiValue| lua.load("return NI.DATA.ANALYSIS_PARAMETERS_SWAPPED").eval::<Value>())?;
    for (method, key) in [
        ("getSliceModeParameter", "Slice.Mode"),
        ("getSplitCountParameter", "Slice.Split"),
        ("getGridNoteLengthParameter", "Slice.Grid"),
        ("getDensityParameter", "Slice.Density"),
        ("getInProgressParameter", "Slice.Density"),
        ("getZeroCrossingParameter", "Slice.ZeroX"),
        ("getBpmModeParameter", "Slice.BpmMode"),
        ("getAutoBpmParameter", "Slice.Bpm"),
        ("getManualBpmParameter", "Slice.Bpm"),
    ] {
        set_fn(lua, &slices, method, move |lua, _: MultiValue| {
            // The script reads these two whatever the app gave: a plain one (0) without.
            match knob_param(lua, key)? {
                Value::Nil if matches!(key, "Slice.Mode" | "Slice.BpmMode") => param(lua, ObjRef::Slices, key),
                v => Ok(v),
            }
        })?;
    }
    let ws = methods(lua, impl_t, "Workspace")?;
    for (method, key) in [
        ("getSlicerApplyModeParameter", "Slice.Apply"),
        ("getSamplingSliceToMonoParameter", "Slice.Mono"),
        ("getAutoSnapParameter", "Slice.Snap"),
    ] {
        set_fn(lua, &ws, method, move |lua, _: MultiValue| knob_param(lua, key))?;
    }

    // The Sampler's zones.
    let module = methods(lua, impl_t, "Module")?;
    set_fn(lua, &module, "getZones", |lua, _: MultiValue| object(lua, ObjRef::Zones))?;
    let zones = methods(lua, impl_t, "ZoneVector")?;
    set_fn(lua, &zones, "size", |lua, _: MultiValue| Ok(state(lua).borrow().sample.as_ref().map_or(0, |s| s.zones.len() as i64)))?;
    set_fn(lua, &zones, "getFocusObject", |lua, _: MultiValue| {
        let any = state(lua).borrow().sample.as_ref().is_some_and(|s| !s.zones.is_empty());
        if any { object(lua, ObjRef::Zone) } else { Ok(Value::Nil) }
    })?;
    set_fn(lua, &zones, "calcIndex", |lua, _: MultiValue| Ok(state(lua).borrow().sample.as_ref().map_or(0, |s| s.focus_zone as i64)))?;
    let zone = methods(lua, impl_t, "Zone")?;
    set_fn(lua, &zone, "getTransactionSample", |lua, _: MultiValue| with_sample(lua, ObjRef::TransactionSample))?;
    set_fn(lua, &zone, "getZoomFactorParameter", |lua, _: MultiValue| param(lua, ObjRef::WaveView, ZOOM))?;
    // The stretch settings (EDIT ▸ STRETCH ▸ SETTINGS) aren't on the controller: hidden.
    set_fn(lua, &zone, "getTimeStretchVisibleParameter", |lua, _: MultiValue| param(lua, ObjRef::Zone, STRETCH_VISIBLE))?;

    // The wave display's view (knobs 5 and 6).
    let views = methods(lua, impl_t, "NI.DATA.SampleOwnerViewHelper")?;
    for name in ["makeFocusView", "makeFocusTakeView"] {
        set_fn(lua, &views, name, |lua, _: MultiValue| with_sample(lua, ObjRef::WaveView))?;
    }
    let view = methods(lua, impl_t, "SampleOwnerView")?;
    for (method, name) in [("waveZoomParameter", ZOOM), ("waveOffsetParameter", OFFSET), ("waveFocusFrameParameter", FOCUS_FRAME)] {
        set_fn(lua, &view, method, move |lua, _: MultiValue| param(lua, ObjRef::WaveView, name))?;
    }

    // What the page asks of the app.
    let fsa = methods(lua, impl_t, "NI.DATA.FocusSampleAccess")?;
    for (name, f) in EDIT_FUNCTIONS {
        set_fn(lua, &fsa, name, move |lua, _: MultiValue| {
            call(lua, SamplingCall::Edit(f));
            Ok(())
        })?;
    }
    let tsa = methods(lua, impl_t, "NI.DATA.TransactionSampleAccess")?;
    set_fn(lua, &tsa, "setFocusSliceByIndex", |lua, (_, _, i): (Value, Value, i64)| {
        if let Some(s) = state(lua).borrow_mut().sample.as_mut() {
            s.focus_slice = usize::try_from(i).ok();
        }
        Ok(())
    })?;
    set_fn(lua, &tsa, "moveSliceStart", |lua, (_, _, i, frame): (Value, Value, i64, f64)| {
        if let Ok(i) = usize::try_from(i) {
            call(lua, SamplingCall::MoveSlice(i, frame.max(0.0) as usize));
        }
        Ok(())
    })?;
    // A slice ends where the next starts: moving its end moves that.
    set_fn(lua, &tsa, "moveSliceEnd", |lua, (_, _, i, frame): (Value, Value, i64, f64)| {
        let n = state(lua).borrow().sample.as_ref().map_or(0, |s| s.slices.len());
        if let Ok(i) = usize::try_from(i)
            && i + 1 < n
        {
            call(lua, SamplingCall::MoveSlice(i + 1, frame.max(0.0) as usize));
        }
        Ok(())
    })?;
    set_fn(lua, &tsa, "splitSlice", |lua, (_, _, i): (Value, Value, i64)| {
        if let Ok(i) = usize::try_from(i) {
            call(lua, SamplingCall::SplitSlice(i));
        }
        Ok(())
    })?;
    set_fn(lua, &tsa, "removeSliceMarker", |lua, (_, _, i): (Value, Value, i64)| {
        if let Ok(i) = usize::try_from(i) {
            call(lua, SamplingCall::RemoveSlice(i));
        }
        Ok(())
    })?;
    set_fn(lua, &tsa, "removeAllSlices", |lua, _: MultiValue| {
        call(lua, SamplingCall::ClearSlices);
        Ok(())
    })?;
    for name in ["sliceNoUndo", "updateAnalysisProgress"] {
        set_fn(lua, &tsa, name, |_, _: MultiValue| Ok(()))?;
    }
    let sa = methods(lua, impl_t, "NI.DATA.SamplerAccess")?;
    for name in ["applySlicingToSound", "applySlicingToGroup", "applySingleSliceToSound"] {
        set_fn(lua, &sa, name, |lua, _: MultiValue| {
            call(lua, SamplingCall::ApplySlices);
            Ok(())
        })?;
    }
    set_fn(lua, &sa, "removeZone", |lua, _: MultiValue| {
        let z = state(lua).borrow().sample.as_ref().filter(|s| !s.zones.is_empty()).map(|s| s.focus_zone);
        if let Some(z) = z {
            call(lua, SamplingCall::RemoveZone(z));
        }
        Ok(())
    })?;
    set_fn(lua, &sa, "cycleZoneSelection", |lua, (_, _, next): (Value, Value, bool)| {
        let to = {
            let st = state(lua);
            let mut st = st.borrow_mut();
            st.sample.as_mut().and_then(|s| {
                let to = if next { s.focus_zone + 1 } else { s.focus_zone.checked_sub(1)? };
                (to < s.zones.len()).then(|| {
                    s.focus_zone = to;
                    to
                })
            })
        };
        if let Some(to) = to {
            call(lua, SamplingCall::FocusZone(to));
        }
        Ok(())
    })?;
    // STEMS: the app's separation (one at a time), on the focused sample or another.
    set_fn(lua, &fsa, "getStemSeparationSource", |lua, _: MultiValue| {
        let run = state(lua).borrow().stems;
        match run {
            Some(r) if r.focused => with_sample(lua, ObjRef::TransactionSample),
            Some(_) => object(lua, ObjRef::StemSource),
            None => Ok(Value::Nil),
        }
    })?;
    set_fn(lua, &fsa, "getStemSeparationProgress", |lua, _: MultiValue| {
        Ok(state(lua).borrow().stems.map_or(0.0, |r| f64::from(r.progress)))
    })?;
    set_fn(lua, &fsa, "executeStemSeparation", |lua, _: MultiValue| {
        call(lua, SamplingCall::Stems);
        Ok(())
    })?;
    set_fn(lua, &fsa, "cancelStemSeparation", |lua, _: MultiValue| {
        call(lua, SamplingCall::CancelStems);
        Ok(())
    })?;
    let za = methods(lua, impl_t, "NI.DATA.ZoneAccess")?;
    set_fn(lua, &za, "findAndSetLoopPoints", |lua, _: MultiValue| {
        call(lua, SamplingCall::AutoLoop);
        Ok(())
    })?;
    let prehear = methods(lua, impl_t, "NI.DATA.SamplePrehearAccess")?;
    set_fn(lua, &prehear, "playSample", |lua, (_, _, r): (Value, Value, Value)| {
        if let Value::Table(r) = r {
            let (a, b): (f64, f64) = (r.get::<mlua::Function>("getMin")?.call(())?, r.get::<mlua::Function>("getMax")?.call(())?);
            call(lua, SamplingCall::Prehear(a as usize, b as usize));
        }
        Ok(())
    })?;
    set_fn(lua, &prehear, "isFocusedZonePlaying", |lua, _: MultiValue| {
        Ok(state(lua).borrow().sample.as_ref().is_some_and(|s| s.playhead.is_some()))
    })?;
    Ok(())
}

/// "1.234 s", or minutes and seconds past a minute.
fn length_text(frames: usize, rate: u32) -> String {
    let s = frames as f64 / f64::from(rate.max(1));
    if s < 60.0 { format!("{s:.3} s") } else { format!("{}:{:06.3}", (s / 60.0) as u64, s % 60.0) }
}

/// The Sampling knob behind a parameter, if it is one.
pub(super) fn knob(lua: &Lua, p: &Value) -> Option<SamplingKnob> {
    let ObjRef::Param { owner, name } = object_ref(lua, p)? else { return None };
    let st = state(lua);
    let st = st.borrow();
    knob_of(&st, st.objects.get(owner)?, &name).cloned()
}

fn knob_of<'a>(st: &'a State, owner: &ObjRef, name: &str) -> Option<&'a SamplingKnob> {
    match owner {
        ObjRef::SamplingParameters => knob_by_key(st, name),
        // Note Repeat / Arp's settings (`api::arp`).
        ObjRef::Arpeggiator => st.arp.knobs.iter().find(|k| k.key == name),
        _ => None,
    }
}

fn knob_by_key<'a>(st: &'a State, key: &str) -> Option<&'a SamplingKnob> {
    st.sampling_pages.iter().flatten().flat_map(|p| &p.knobs).chain(&st.slice_knobs).find(|k| k.key == key)
}

/// Turn a Sampling knob (`inc` of a turn; a tenth with `fine`): a step each time it has
/// gone the knob's `turn`. `false` if `p` isn't one.
pub(super) fn turn(lua: &Lua, p: &Value, inc: f64, fine: bool) -> bool {
    let Some(k) = knob(lua, p) else { return false };
    let st = state(lua);
    let mut st = st.borrow_mut();
    let acc = st.sampling_turns.entry(k.key.clone()).or_default();
    *acc += inc * if fine { 0.1 } else { 1.0 };
    let n = (*acc / k.turn.max(1e-6)).trunc();
    if n == 0.0 {
        return true;
    }
    *acc -= n * k.turn;
    set(&mut st, &k, k.value + n * k.step);
    true
}

/// Set a knob's value (within its range) for the app to apply.
fn set(st: &mut State, k: &SamplingKnob, to: f64) {
    let to = to.clamp(k.min, k.max);
    if to == k.value {
        return;
    }
    // Shown at once; the app's next knobs replace it.
    let State { sampling_pages, slice_knobs, arp, .. } = st;
    if let Some(mine) = sampling_pages
        .iter_mut()
        .flatten()
        .flat_map(|p| &mut p.knobs)
        .chain(slice_knobs.iter_mut())
        .chain(arp.knobs.iter_mut())
        .find(|m| m.key == k.key)
    {
        mine.value = to;
    }
    if k.key.starts_with("arp/") {
        st.arp_edits.push((k.key.clone(), to));
    } else {
        st.sampling_edits.push((k.key.clone(), to));
    }
}

/// Set a knob's parameter or the wave view (`setValue`, `ParameterAccess.set…`):
/// `true` if it was one.
pub(super) fn write(st: &mut State, owner: &ObjRef, name: &str, v: &Prop) -> bool {
    let num = match v {
        Prop::Int(i) => *i as f64,
        Prop::Num(n) => *n,
        Prop::Bool(b) => f64::from(u8::from(*b)),
        _ => return matches!(owner, ObjRef::WaveView) || knob_of(st, owner, name).is_some(),
    };
    if let ObjRef::WaveView = owner {
        let Some(s) = st.sample.as_mut() else { return true };
        let len = s.frames as f64;
        let span = (s.view.1 - s.view.0).max(1.0);
        let (a, span) = match name {
            OFFSET => (num, span),
            ZOOM => (s.view.0, (len / num.max(1.0)).max(MIN_SPAN.min(len))),
            _ => return true,
        };
        let a = a.clamp(0.0, (len - span).max(0.0));
        s.view = (a, a + span);
        st.sampling_calls.push(SamplingCall::View(a, a + span));
        return true;
    }
    let Some(k) = knob_of(st, owner, name).cloned() else { return false };
    set(st, &k, num);
    true
}

pub(super) fn live(st: &State, owner: &ObjRef, name: &str) -> Option<Prop> {
    if let Some(k) = knob_of(st, owner, name) {
        // (Note Repeat's HOLD is a switch to the scripts: 0 would read as on.)
        if let (ObjRef::Arpeggiator, "arp/hold") = (owner, name) {
            return Some(Prop::Bool(k.value > 0.5));
        }
        return Some(Prop::Num(k.value));
    }
    match (owner, name) {
        (ObjRef::KnobPages(t), FOCUS_PAGE) => Some(Prop::Int(st.sampling_page[*t] as i64)),
        (ObjRef::Zone, STRETCH_VISIBLE) => Some(Prop::Bool(false)),
        (ObjRef::WaveView, _) => {
            let s = st.sample.as_ref()?;
            let span = (s.view.1 - s.view.0).max(1.0);
            Some(Prop::Num(match name {
                ZOOM => s.frames as f64 / span,
                OFFSET => s.view.0,
                FOCUS_FRAME => s.view.0 + span / 2.0,
                _ => return None,
            }))
        }
        _ => None,
    }
}

/// The wave view's zoom: up to the shortest stretch shown.
pub(super) fn max(st: &State, owner: &ObjRef, name: &str) -> Option<i64> {
    match (owner, name) {
        (ObjRef::WaveView, ZOOM) => Some(st.sample.as_ref().map_or(1.0, |s| (s.frames as f64 / MIN_SPAN).max(1.0)) as i64),
        _ => None,
    }
}

pub(super) fn min(owner: &ObjRef, name: &str) -> Option<f64> {
    matches!((owner, name), (ObjRef::WaveView, ZOOM)).then_some(1.0)
}
