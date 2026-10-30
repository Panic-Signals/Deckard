//! The controller's Sampling page beyond RECORD (`record_knobs`): what its EDIT, SLICE
//! and ZONE screens show of the focused zone's sample (`State::sample`: the waveform's
//! peaks, play range, loop, selection, slices, zones), their knobs, and what turning
//! each sets, as the Sampling view's footers have them:
//!
//! - EDIT: PLAY RANGE (START, END), SELECTION RANGE (START, END); the MK3 manual's two
//!   pages.
//! - ZONE: PLAY RANGE, LOOP (ACTIVE, START, END, XFADE), TUNE / MIX (TUNE, GAIN, PAN,
//!   ROOT KEY), ENVELOPE (ATTACK, DECAY), MAP (KEY LO, KEY HI, VEL LO, VEL HI); the
//!   manual's five pages.
//! - SLICE: the knobs NI's script asks its slice container for: the slicer's MODE and
//!   its setting (SLICES, SENSITIVITY, ZERO-X), the tempo it slices to, and how APPLY
//!   makes a pattern.
//!
//! Frame knobs move across the part of the sample shown in a turn (a hundredth a step).

use crate::layout::sampling::{GRID_DIVS, PATTERN_MODES, SLICE_MODES, SPLIT_COUNTS, SamplingState, frames_text};
use crate::record_knobs::{self, Knobs};
use deckard_lua_host::state::{KnobPage, SampleView, SamplingKnob};
use deckard_model::{Color, Project};

/// Peaks per display: a column per pixel of the MK3's 480-pixel displays.
const COLUMNS: usize = 480;
const TURN: f64 = 1.0 / 32.0;
/// A frame knob: a hundredth of the part shown per step.
const FRAME_STEPS: f64 = 100.0;
/// NI's slicer modes (`NI.DATA.MODE_DETECT` 0, SPLIT 1, GRID 2, MANUAL 3) as the app's
/// (`SLICE_MODES`: Split, Grid, Detect, Manual), and back.
const NI_TO_APP_SLICE_MODE: [usize; 4] = [2, 0, 1, 3];
const APP_TO_NI_SLICE_MODE: [usize; 4] = [1, 2, 0, 3];

/// What turning one of these knobs sets.
#[derive(Debug, Clone, PartialEq)]
pub enum Apply {
    Record(record_knobs::Apply),
    /// A parameter of the focused zone (`Zone::set`'s path).
    Zone(&'static str),
    /// The selection's start (false) or end.
    Selection(bool),
    SliceMode,
    SplitCount,
    GridDiv,
    Sensitivity,
    ZeroX,
    PatternMode,
    /// Shown, not set.
    None,
}

/// Every tab's pages and the SLICE tab's knobs, with what each sets.
#[derive(Default)]
pub struct Pages {
    pub tabs: [Vec<KnobPage>; 4],
    pub slice: Vec<SamplingKnob>,
    pub apply: std::collections::HashMap<String, Apply>,
}

fn knob(key: &str, name: &str, text: String, value: f64, (min, max): (f64, f64), step: f64, turn: f64) -> SamplingKnob {
    SamplingKnob { key: key.into(), name: name.into(), text, value, min, max, step, turn }
}

fn choice(key: &str, name: &str, options: &[String], at: usize) -> SamplingKnob {
    let text = options.get(at).cloned().unwrap_or_default();
    knob(key, name, text, at as f64, (0.0, options.len().saturating_sub(1) as f64), 1.0, TURN)
}

fn on_off(key: &str, name: &str, on: bool) -> SamplingKnob {
    choice(key, name, &["Off".into(), "On".into()], usize::from(on))
}

fn page(pages: &mut Pages, tab: usize, name: &str, knobs: Vec<(SamplingKnob, Apply)>) {
    let knobs = knobs
        .into_iter()
        .map(|(k, a)| {
            pages.apply.insert(k.key.clone(), a);
            k
        })
        .collect();
    pages.tabs[tab].push(KnobPage { name: name.into(), knobs });
}

/// The focused zone's sample as the controller shows it (`None` without one).
/// `focus_slice`: the scripts' own, kept while there is such a slice.
pub fn sample_view(st: &SamplingState, p: &Project, (g, s, zi): (usize, usize, usize), focus_slice: Option<usize>) -> Option<SampleView> {
    let sample = st.sample.as_ref()?;
    let smp = p.groups.get(g)?.sounds.get(s)?.sampler.as_ref()?;
    let z = smp.zones.get(zi)?;
    let frames = sample.frames.len();
    let (v0, v1) = st.view();
    let peaks = |a: f64, b: f64| -> Vec<(f32, f32)> {
        (0..COLUMNS)
            .map(|c| {
                let f0 = (a + (b - a) * c as f64 / COLUMNS as f64) as usize;
                let f1 = ((a + (b - a) * (c + 1) as f64 / COLUMNS as f64) as usize).max(f0 + 1);
                let [l, r] = st.column(f0, f1);
                // Both channels together.
                (l.0.min(r.0).min(0.0), l.1.max(r.1).max(0.0))
            })
            .collect()
    };
    let play = if z.play_range.1 > z.play_range.0 { (z.play_range.0 as usize, z.play_range.1 as usize + 1) } else { (0, frames) };
    let group = &p.groups[g];
    let color = match (group.sounds[s].color, group.color) {
        (Color::Palette(i), _) | (Color::Default, Color::Palette(i)) => i as u32,
        (Color::Default, Color::Default) => (g * 2 % 16) as u32,
    };
    let slices = if st.tab == 2 && st.slices.is_empty() { vec![0] } else { st.slices.clone() };
    let focus_slice = focus_slice.filter(|&i| i < slices.len());
    Some(SampleView {
        name: z.sample.path.rsplit(['/', '\\']).next().unwrap_or("").to_string(),
        path: z.sample.path.clone(),
        frames,
        rate: sample.rate,
        overview: peaks(0.0, frames as f64),
        detail: peaks(v0, v1),
        view: (v0, v1),
        play,
        looped: (z.loop_active && z.loop_range.1 > z.loop_range.0).then_some((z.loop_range.0 as usize, z.loop_range.1 as usize + 1)),
        selection: st.selection,
        slices,
        focus_slice,
        playhead: st.playhead,
        zones: smp.zones.iter().map(|z| (z.key_range, z.velocity_range)).collect(),
        focus_zone: zi,
        color: Some(color),
    })
}

/// The knobs: RECORD's (`record_knobs`), and with a sample, EDIT's, SLICE's and ZONE's.
pub fn pages(record: Knobs, st: &SamplingState, p: &Project, (g, s, zi): (usize, usize, usize)) -> Pages {
    let mut out = Pages::default();
    out.tabs[0] = record.pages;
    out.apply.extend(record.apply.into_iter().map(|(k, a)| (k, Apply::Record(a))));
    let Some(z) = p.groups.get(g).and_then(|x| x.sounds.get(s)).and_then(|x| x.sampler.as_ref()).and_then(|x| x.zones.get(zi)) else {
        return out;
    };
    let Some(sample) = st.sample.as_ref() else { return out };
    let (len, rate) = (sample.frames.len().max(1) as f64, sample.rate);
    let (v0, v1) = st.view();
    let step = ((v1 - v0) / FRAME_STEPS).max(1.0).round();
    let last = len - 1.0;
    let frames = |key: &str, name: &str, v: i64, apply: Apply| {
        (knob(key, name, frames_text(v.max(0) as usize, rate), v as f64, (0.0, last), step, 1.0 / FRAME_STEPS), apply)
    };
    let (lo, hi) = if z.play_range.1 > z.play_range.0 { z.play_range } else { (0, last as i64) };
    let (sa, sb) = st.selection.map_or((lo, hi), |(a, b)| (a as i64, b as i64));

    // EDIT.
    page(
        &mut out,
        1,
        "PLAY RANGE",
        vec![
            frames("Edit.Start", "START", lo, Apply::Zone("play_range/low")),
            frames("Edit.End", "END", hi, Apply::Zone("play_range/high")),
        ],
    );
    page(
        &mut out,
        1,
        "SELECTION RANGE",
        vec![frames("Sel.Start", "START", sa, Apply::Selection(false)), frames("Sel.End", "END", sb, Apply::Selection(true))],
    );

    // ZONE.
    page(
        &mut out,
        3,
        "PLAY RANGE",
        vec![
            frames("Zone.Start", "START", lo, Apply::Zone("play_range/low")),
            frames("Zone.End", "END", hi, Apply::Zone("play_range/high")),
        ],
    );
    let (la, lb) = z.loop_range;
    let xfade_max = (lb - la).max(0) as f64;
    page(
        &mut out,
        3,
        "LOOP",
        vec![
            (on_off("Zone.Loop", "ACTIVE", z.loop_active), Apply::Zone("loop_active")),
            frames("Zone.LoopStart", "START", la, Apply::Zone("loop_range/low")),
            frames("Zone.LoopEnd", "END", lb, Apply::Zone("loop_range/high")),
            (
                knob(
                    "Zone.XFade",
                    "XFADE",
                    frames_text(z.x_fade.max(0) as usize, rate),
                    z.x_fade as f64,
                    (0.0, xfade_max),
                    step,
                    1.0 / FRAME_STEPS,
                ),
                Apply::Zone("x_fade"),
            ),
        ],
    );
    let gain_db = 20.0 * (2.3948107 * z.gain.powi(5) + 0.767467 * z.gain * z.gain).max(1e-9).log10();
    let note = |k: i64| crate::recorder::note_name(k.clamp(0, 127) as u8);
    page(
        &mut out,
        3,
        "TUNE / MIX",
        vec![
            (knob("Zone.Tune", "TUNE", format!("{:+.2}", z.tune), z.tune, (-36.0, 36.0), 0.01, 1.0 / 1200.0), Apply::Zone("tune")),
            (knob("Zone.Gain", "GAIN", format!("{gain_db:.1} dB"), z.gain, (0.0, 1.0), 0.005, 1.0 / 200.0), Apply::Zone("gain")),
            (knob("Zone.Pan", "PAN", crate::layout::pan(z.pan), z.pan, (-1.0, 1.0), 0.01, 1.0 / 200.0), Apply::Zone("pan")),
            (knob("Zone.Root", "ROOT KEY", note(z.root_key), z.root_key as f64, (0.0, 127.0), 1.0, TURN), Apply::Zone("root_key")),
        ],
    );
    // Attack and decay in frames, a millisecond a step.
    let ms = (f64::from(rate) / 1000.0).max(1.0);
    let ms_text = |f: i64| format!("{:.0} ms", f as f64 / ms);
    page(
        &mut out,
        3,
        "ENVELOPE",
        vec![
            (knob("Zone.Attack", "ATTACK", ms_text(z.fade_in), z.fade_in as f64, (0.0, last), ms, 1.0 / 200.0), Apply::Zone("attack")),
            (knob("Zone.Decay", "DECAY", ms_text(z.fade_out), z.fade_out as f64, (0.0, last), ms, 1.0 / 200.0), Apply::Zone("decay")),
        ],
    );
    let key = |k: &str, n: &str, v: i64, text: String, path: &'static str| {
        (knob(k, n, text, v as f64, (0.0, 127.0), 1.0, TURN), Apply::Zone(path))
    };
    let (kl, kh, vl, vh) = (z.key_range.0, z.key_range.1, z.velocity_range.0, z.velocity_range.1);
    page(
        &mut out,
        3,
        "MAP",
        vec![
            key("Zone.KeyLo", "KEY LO", kl, note(kl), "key_range/low"),
            key("Zone.KeyHi", "KEY HI", kh, note(kh), "key_range/high"),
            key("Zone.VelLo", "VEL LO", vl, vl.to_string(), "velocity_range/low"),
            key("Zone.VelHi", "VEL HI", vh, vh.to_string(), "velocity_range/high"),
        ],
    );

    // SLICE: the knobs NI's script picks (by its slicer mode's numbers).
    let names = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let ni_names: Vec<String> = (0..4).map(|m| SLICE_MODES[NI_TO_APP_SLICE_MODE[m]].to_string()).collect();
    let tempo = p.tempo;
    let slice = [
        (choice("Slice.Mode", "MODE", &ni_names, APP_TO_NI_SLICE_MODE[st.slice_mode.min(3)]), Apply::SliceMode),
        (choice("Slice.Split", "SLICES", &SPLIT_COUNTS.map(|c| c.to_string()), st.split), Apply::SplitCount),
        (choice("Slice.Grid", "SLICES", &GRID_DIVS.map(|d| d.1.to_string()), st.grid), Apply::GridDiv),
        (
            knob("Slice.Density", "SENSITIVITY", format!("{:.0} %", st.sensitivity * 100.0), st.sensitivity, (0.0, 1.0), 0.01, 1.0 / 100.0),
            Apply::Sensitivity,
        ),
        (on_off("Slice.ZeroX", "ZERO-X", st.zero_x), Apply::ZeroX),
        (choice("Slice.Apply", "PATTERN", &names(&PATTERN_MODES), st.pattern_mode), Apply::PatternMode),
        // Slices follow the song's tempo.
        (knob("Slice.BpmMode", "BPM", "Song".into(), 0.0, (0.0, 0.0), 1.0, TURN), Apply::None),
        (knob("Slice.Bpm", "TEMPO", format!("{tempo:.2}"), tempo, (tempo, tempo), 1.0, TURN), Apply::None),
    ];
    for (k, a) in slice {
        out.apply.insert(k.key.clone(), a);
        out.slice.push(k);
    }
    out
}

/// What NI's slicer mode `ni` is in the app (`SLICE_MODES`).
pub fn app_slice_mode(ni: f64) -> usize {
    NI_TO_APP_SLICE_MODE[(ni.round().max(0.0) as usize).min(3)]
}
