//! The controller's Sampling page beyond RECORD (`record_knobs`): what its EDIT, SLICE
//! and ZONE screens show of the focused zone's sample (`State::sample`: the waveform's
//! peaks, play range, loop, selection, slices, zones), their knobs, and what turning
//! each sets, as the Sampling view's footers have them:
//!
//! - EDIT: PLAY RANGE (START, END), SELECTION RANGE (START, END); the MK3 manual's two
//!   pages. A New Sampler zone's also TEMPO / PITCH (MODE, BPM, BEATS, ROOT) and
//!   PLAYBACK (SPEED, REVERSE, KEY).
//! - ZONE: PLAY RANGE, LOOP (ACTIVE, START, END, XFADE), TUNE / MIX (TUNE, GAIN, PAN,
//!   ROOT KEY), ENVELOPE (ATTACK, DECAY), MAP (KEY LO, KEY HI, VEL LO, VEL HI); the
//!   manual's five pages.
//! - SLICE: the knobs NI's script asks its slice container for: the slicer's MODE and
//!   its setting (SLICES, SENSITIVITY, ZERO-X), the tempo it slices to, and how APPLY
//!   makes a pattern.
//!
//! Frame knobs move across the part of the sample shown in a turn (a hundredth a step).

use crate::layout::sampling::{APPLY_TO, FIND_COUNTS, GRID_DIVS, NOTE_ORDERS, PATTERN_MODES, SLICE_MODES, SPLIT_COUNTS, SamplingState};
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
/// DECKARD's own Find comes after them.
const NI_TO_APP_SLICE_MODE: [usize; 6] = [2, 0, 1, 3, 4, 5];
const APP_TO_NI_SLICE_MODE: [usize; 6] = [1, 2, 0, 3, 4, 5];

/// What turning one of these knobs sets.
#[derive(Debug, Clone, PartialEq)]
pub enum Apply {
    Record(record_knobs::Apply),
    /// A parameter of the focused zone (`Zone::set`'s path).
    Zone(&'static str),
    /// The selection's start (false) or end.
    Selection(bool),
    SliceMode,
    /// A New Sampler zone's own speed (percent), and whether it plays backwards.
    Speed,
    Reverse,
    /// Held, a one-shot or a loop.
    Plays,
    /// A New Sampler zone's tail (ms), and its sound's Slice Motion.
    Tail,
    Motion,
    /// The loop's length: sixteenths of the sample's tempo, or (no tempo) milliseconds.
    LoopLength,
    FindCount,
    RandomCount,
    SplitCount,
    GridDiv,
    Sensitivity,
    ZeroX,
    PatternMode,
    ApplyTo,
    NoteOrder,
    /// A New Sampler zone's play mode (0 Repitch, 1 Sync, 2 Keys), tempo, length in
    /// beats (whole ones: its tempo follows), root note, and the note it plays at (its
    /// Tune).
    PlayMode,
    Bpm,
    Beats,
    Root,
    PlaysAt,
    /// A Sync zone's start grid (`StartGrid::ALL`).
    StartGrid,
    /// Stem lane `.0`'s level, in dB (−60: silent).
    LaneGain(usize),
    /// QUANTIZE's grid (`beat::WARP_GRIDS`) and strength (0..1).
    WarpGrid,
    WarpStrength,
    /// A setting of the focused layer; of the sound's morph.
    Layer(LayerKnob),
    Morph(MorphKnob),
    /// Shown, not set.
    None,
}

/// The focused layer's settings on LAYERS' knobs. Its envelope's times are steps of
/// [`time_at`]; a window point is A, B, C or D (0 to 3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayerKnob {
    Pan,
    Tune,
    Root,
    Attack,
    Decay,
    Sustain,
    Release,
    Window(usize),
}

/// The morph's settings on LAYERS' knobs. RATE is a step of [`rate_at`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MorphKnob {
    On,
    Position,
    Rate,
    Shape,
    Depth,
    Reset,
    Fade,
}

/// The envelope times a knob steps through: none, then from a millisecond a tenth
/// longer each step, to the longest.
const TIME_STEPS: usize = 110;

/// The time at step `i` (seconds).
pub fn time_at(i: usize) -> f64 {
    if i == 0 { 0.0 } else { (0.001 * 1.1f64.powi(i as i32 - 1)).min(deckard_model::layers::TIME_MAX) }
}

/// The step nearest `seconds`.
pub fn time_step(seconds: f64) -> usize {
    if seconds < 0.0005 {
        return 0;
    }
    (((seconds / 0.001).ln() / 1.1f64.ln()).round().max(0.0) as usize + 1).min(TIME_STEPS)
}

/// How many free rates follow the synced ones on the RATE knob.
const FREE_STEPS: usize = 98;

/// The morph's rate at step `i` of its knob: a synced cycle in bars (`true`), then free
/// seconds, a tenth longer each step.
pub fn rate_at(i: usize) -> (bool, f64) {
    use deckard_model::layers::{FREE, SYNCED};
    match SYNCED.get(i) {
        Some(&bars) => (true, bars),
        None => (false, (FREE.0 * 1.1f64.powi((i - SYNCED.len()) as i32)).min(FREE.1)),
    }
}

/// The step of the RATE knob nearest the morph's rate.
pub fn rate_step(m: &deckard_model::layers::Morph) -> usize {
    use deckard_model::layers::{FREE, SYNCED};
    if m.sync {
        return SYNCED.iter().position(|b| *b >= m.bars).unwrap_or(SYNCED.len() - 1);
    }
    SYNCED.len() + (((m.seconds.max(FREE.0) / FREE.0).ln() / 1.1f64.ln()).round() as usize).min(FREE_STEPS - 1)
}

/// LAYERS' pages for sound (`g`, `s`), its layer `focus` focused: LAYER, ENVELOPE, MORPH
/// and WINDOW. None for a sound without layers.
fn layer_pages(out: &mut Pages, p: &Project, (g, s): (usize, usize), focus: usize) {
    use crate::layout::sampling::layers::{rate_text, time_text};
    use deckard_model::layers::{Fade, Shape};
    let layers = crate::layers::layers_in(p, g, s);
    let Some(&(zi, l)) = layers.get(focus.min(layers.len().saturating_sub(1))) else { return };
    let Some(snd) = p.groups.get(g).and_then(|x| x.sounds.get(s)) else { return };
    let Some(z) = snd.sampler.as_ref().and_then(|x| x.zones.get(zi)) else { return };
    let m = snd.new_sampler.map(|n| n.morph).unwrap_or_default();
    let names = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let fine = 1.0 / 200.0;
    page(
        out,
        4,
        "LAYER",
        vec![
            (knob("Layer.Pan", "PAN", crate::layout::pan(z.pan), z.pan, (-1.0, 1.0), 0.01, fine), Apply::Layer(LayerKnob::Pan)),
            (knob("Layer.Tune", "TUNE", format!("{:+.2}", z.tune), z.tune, (-36.0, 36.0), 0.01, fine), Apply::Layer(LayerKnob::Tune)),
            (
                knob("Layer.Root", "ROOT", crate::keys::note_name(z.root_key), z.root_key as f64, (0.0, 127.0), 1.0, TURN),
                Apply::Layer(LayerKnob::Root),
            ),
        ],
    );
    let time = |key: &str, name: &str, v: f64, which: LayerKnob| {
        (knob(key, name, time_text(v), time_step(v) as f64, (0.0, TIME_STEPS as f64), 1.0, fine * 2.0), Apply::Layer(which))
    };
    page(
        out,
        4,
        "ENVELOPE",
        vec![
            time("Layer.Attack", "ATTACK", l.attack, LayerKnob::Attack),
            time("Layer.Decay", "DECAY", l.decay, LayerKnob::Decay),
            (
                knob("Layer.Sustain", "SUSTAIN", format!("{:.0} %", l.sustain * 100.0), l.sustain, (0.0, 1.0), 0.01, fine),
                Apply::Layer(LayerKnob::Sustain),
            ),
            time("Layer.Release", "RELEASE", l.release, LayerKnob::Release),
        ],
    );
    let round = m.shape == Shape::Round;
    // (Round always goes all the way: DEPTH is There and back's, and doesn't turn.)
    let depth = if round {
        knob("Morph.Depth", "DEPTH", "-".into(), m.depth, (m.depth, m.depth), 0.01, fine)
    } else {
        knob("Morph.Depth", "DEPTH", format!("{:.0} %", m.depth * 100.0), m.depth, (0.0, 1.0), 0.01, fine)
    };
    let last = (deckard_model::layers::SYNCED.len() + FREE_STEPS - 1) as f64;
    page(
        out,
        4,
        "MORPH",
        vec![
            (on_off("Morph.On", "MORPH", m.on), Apply::Morph(MorphKnob::On)),
            (
                knob("Morph.Position", "POSITION", format!("{:.0} %", m.position * 100.0), m.position, (0.0, 1.0), 0.005, fine),
                Apply::Morph(MorphKnob::Position),
            ),
            (knob("Morph.Rate", "RATE", rate_text(&m), rate_step(&m) as f64, (0.0, last), 1.0, fine * 4.0), Apply::Morph(MorphKnob::Rate)),
            (choice("Morph.Shape", "SHAPE", &names(&["Round", "There and back"]), usize::from(!round)), Apply::Morph(MorphKnob::Shape)),
            (depth, Apply::Morph(MorphKnob::Depth)),
            (on_off("Morph.Reset", "RESET", m.reset), Apply::Morph(MorphKnob::Reset)),
        ],
    );
    // (Round: a window may reach a turn past either end.)
    let range = if round { (-1.0, 2.0) } else { (0.0, 1.0) };
    let point = |k: usize| {
        let name = ["A", "B", "C", "D"][k];
        (
            knob(&format!("Window.{name}"), name, format!("{:.0} %", l.window[k] * 100.0), l.window[k], range, 0.005, fine),
            Apply::Layer(LayerKnob::Window(k)),
        )
    };
    page(
        out,
        4,
        "WINDOW",
        vec![
            point(0),
            point(1),
            point(2),
            point(3),
            (
                choice("Window.Fade", "FADE", &names(&["Linear", "Equal power"]), usize::from(m.fade == Fade::Power)),
                Apply::Morph(MorphKnob::Fade),
            ),
        ],
    );
}

/// Every tab's pages and the SLICE tab's knobs, with what each sets.
#[derive(Default)]
pub struct Pages {
    pub tabs: [Vec<KnobPage>; 5],
    pub slice: Vec<SamplingKnob>,
    pub apply: std::collections::HashMap<String, Apply>,
}

fn knob(key: &str, name: &str, text: String, value: f64, (min, max): (f64, f64), step: f64, turn: f64) -> SamplingKnob {
    SamplingKnob { key: key.into(), name: name.into(), text, value, min, max, step, turn }
}

/// A sample's length in beats as its knob shows it: whole bars of four as bars too ("8
/// (2 bars)"), a length between beats to two places ("8.60").
pub fn beats_text(beats: Option<f64>) -> String {
    match beats {
        None => "-".into(),
        Some(b) if (b - b.round()).abs() < 0.005 => {
            let n = b.round() as i64;
            match (n % 4, n / 4) {
                (0, 1) => format!("{n} (1 bar)"),
                (0, bars) => format!("{n} ({bars} bars)"),
                _ => format!("{n}"),
            }
        }
        Some(b) => format!("{b:.2}"),
    }
}

/// The whole beats a turn of the BEATS knob from `now` to `asked` means: from a length
/// between beats, the next whole one that way.
pub fn beats_turned(now: Option<f64>, asked: f64) -> f64 {
    let Some(now) = now else { return asked.round().max(1.0) };
    let steps = (asked - now).round();
    let whole = (now - now.round()).abs() < 0.005;
    let from = if whole {
        now.round()
    } else if steps > 0.0 {
        now.floor()
    } else {
        now.ceil()
    };
    (from + steps).max(1.0)
}

fn choice(key: &str, name: &str, options: &[String], at: usize) -> SamplingKnob {
    let text = options.get(at).cloned().unwrap_or_default();
    knob(key, name, text, at as f64, (0.0, options.len().saturating_sub(1) as f64), 1.0, TURN)
}

fn on_off(key: &str, name: &str, on: bool) -> SamplingKnob {
    choice(key, name, &["Off".into(), "On".into()], usize::from(on))
}

/// What a loop's length is counted in for a sample at `bpm` and `rate`: frames a unit,
/// the knob's step in units, and the unit's name. Beats (a sixteenth a step), or
/// milliseconds (10 a step) without a tempo.
pub fn loop_unit(bpm: Option<f64>, rate: u32) -> (f64, f64, &'static str) {
    match bpm.filter(|b| b.is_finite() && *b > 0.0) {
        Some(b) => (f64::from(rate) * 60.0 / b, 0.25, "beats"),
        None => (f64::from(rate) / 1000.0, 10.0, "ms"),
    }
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
    let (zi, z, _, take) = crate::layout::sampling::edited(p, (g, s, zi))?;
    let zones =
        p.groups[g].sounds[s].sampler.as_ref().map_or(vec![], |smp| smp.zones.iter().map(|z| (z.key_range, z.velocity_range)).collect());
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
    // Its stem lanes, those whose waveforms are loaded.
    let lanes = z.meta.as_ref().filter(|_| !take).map_or_else(Vec::new, |m| {
        let levels = deckard_model::new_sampler::lane_levels(&m.stems);
        (0..m.stems.len())
            .filter_map(|k| {
                let l = m.stems[k].as_ref().filter(|_| st.lanes[k].is_some())?;
                let detail = (0..COLUMNS)
                    .map(|c| {
                        let f0 = (v0 + (v1 - v0) * c as f64 / COLUMNS as f64) as usize;
                        let f1 = ((v0 + (v1 - v0) * (c + 1) as f64 / COLUMNS as f64) as usize).max(f0 + 1);
                        let [a, b] = st.lane_column(k, f0, f1);
                        (a.0.min(b.0).min(0.0), a.1.max(b.1).max(0.0))
                    })
                    .collect();
                Some(deckard_model::sampling_view::LaneView {
                    lane: k,
                    name: deckard_model::new_sampler::LANE_NAMES[k].to_uppercase(),
                    mute: l.mute,
                    solo: l.solo,
                    heard: levels[k] > 0.0,
                    detail,
                })
            })
            .collect()
    });
    // Its layers (a layered sound's), their waveforms as LAYERS has them loaded.
    let found = crate::layers::layers_in(p, g, s);
    let heard = deckard_model::layers::audible(&found.iter().map(|(_, l)| *l).collect::<Vec<_>>());
    let all = p.groups[g].sounds[s].sampler.as_ref().map(|x| x.zones.as_slice()).unwrap_or_default();
    let layers: Vec<deckard_model::sampling_view::LayerView> = found
        .iter()
        .enumerate()
        .filter_map(|(k, &(i, l))| {
            let lz = all.get(i)?;
            let detail = st.layers.get(k).and_then(Option::as_ref).map_or_else(Vec::new, |view| {
                let n = view.sample.frames.len();
                (0..COLUMNS)
                    .map(|c| {
                        let (f0, f1) = (c * n / COLUMNS, ((c + 1) * n / COLUMNS).max(c * n / COLUMNS + 1));
                        let [a, b] = view.column(f0, f1);
                        (a.0.min(b.0).min(0.0), a.1.max(b.1).max(0.0))
                    })
                    .collect()
            });
            Some(deckard_model::sampling_view::LayerView {
                name: lz.sample.path.rsplit(['/', '\\']).next().unwrap_or("").to_string(),
                mute: l.mute,
                solo: l.solo,
                heard: heard.get(k).copied().unwrap_or(true),
                gain: lz.gain,
                level: crate::layout::sampling::layers::level_text(lz.gain),
                window: l.window,
                envelope: [l.attack, l.decay, l.sustain, l.release],
                detail,
            })
        })
        .collect();
    let morph = p.groups[g].sounds[s].new_sampler.filter(|_| !layers.is_empty()).map(|n| deckard_model::sampling_view::MorphView {
        on: n.morph.on,
        position: st.morph_at.map(f64::from).filter(|_| n.morph.on).unwrap_or(n.morph.position),
        round: n.morph.shape == deckard_model::layers::Shape::Round,
        power: n.morph.fade == deckard_model::layers::Fade::Power,
    });
    let layer = zi.min(layers.len().saturating_sub(1));
    let group = &p.groups[g];
    let color = match (group.sounds[s].color, group.color) {
        (Color::Palette(i), _) | (Color::Default, Color::Palette(i)) => i as u32,
        (Color::Default, Color::Default) => (g * 2 % 16) as u32,
    };
    let slices = if st.tab == 2 && st.slices.is_empty() { vec![0] } else { st.slices.clone() };
    let focus_slice = focus_slice.filter(|&i| i < slices.len());
    let kept = slices.iter().enumerate().filter(|(_, f)| st.kept.contains(f)).map(|(i, _)| i).collect();
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
        kept,
        focus_slice,
        playhead: st.playhead,
        zones,
        take,
        focus_zone: zi,
        color: Some(color),
        lanes,
        layers,
        layer,
        morph,
        layered: crate::layers::layered(p, g, s),
    })
}

/// The knobs: RECORD's (`record_knobs`), and with a sample, EDIT's, SLICE's and ZONE's.
pub fn pages(record: Knobs, st: &SamplingState, p: &Project, (g, s, zi): (usize, usize, usize)) -> Pages {
    let mut out = Pages::default();
    out.tabs[0] = record.pages;
    out.apply.extend(record.apply.into_iter().map(|(k, a)| (k, Apply::Record(a))));
    layer_pages(&mut out, p, (g, s), zi);
    let Some((_, z, _, take)) = crate::layout::sampling::edited(p, (g, s, zi)) else {
        return out;
    };
    let Some(sample) = st.sample.as_ref() else { return out };
    let (len, rate) = (sample.frames.len().max(1) as f64, sample.rate);
    let (v0, v1) = st.view();
    let frame_turn = 1.0 / (v1 - v0).max(FRAME_STEPS);
    let last = len - 1.0;
    let frames = |key: &str, name: &str, v: i64, (min, max): (f64, f64), apply: Apply| {
        (knob(key, name, v.max(0).to_string(), v as f64, (min, max), 1.0, frame_turn), apply)
    };
    let (lo, hi) = if z.play_range.1 > z.play_range.0 { z.play_range } else { (0, last as i64) };
    let start_max = (hi - 1).max(0) as f64;
    let end_min = (lo + 1).min(last as i64) as f64;
    let (sa, sb) = st.selection.map_or((lo, hi), |(a, b)| (a as i64, b as i64));

    // EDIT.
    page(
        &mut out,
        1,
        "PLAY RANGE",
        vec![
            frames("Edit.Start", "START", lo, (0.0, start_max), Apply::Zone("play_range/low")),
            frames("Edit.End", "END", hi, (end_min, last), Apply::Zone("play_range/high")),
        ],
    );
    page(
        &mut out,
        1,
        "SELECTION RANGE",
        vec![
            frames("Sel.Start", "START", sa, (0.0, last), Apply::Selection(false)),
            frames("Sel.End", "END", sb, (0.0, last), Apply::Selection(true)),
        ],
    );

    // An Audio plug-in's take has no zones or slices.
    if take {
        return out;
    }

    // A New Sampler zone: how it plays, its tempo and its pitch (roadmap 1d; the
    // display buttons have ×2, ½ and TO PROJECT under SHIFT).
    let new_sampler = p.groups.get(g).and_then(|x| x.sounds.get(s)).is_some_and(|x| x.new_sampler.is_some());
    if let (true, Some(m)) = (new_sampler, z.meta.as_ref()) {
        use deckard_model::new_sampler::PlayMode;
        let modes = ["Repitch".to_string(), "Sync".into(), "Keys".into()];
        let mode = match m.mode {
            PlayMode::Repitch => 0,
            PlayMode::Sync => 1,
            PlayMode::Keys => 2,
        };
        let bpm = m.bpm.unwrap_or(120.0);
        let bpm_text = m.bpm.map_or("-".into(), |b| format!("{b:.2}"));
        let root = m.root.unwrap_or(z.root_key.clamp(0, 127) as u8);
        let root_text = match m.root {
            // (A key's tonic: "E3 min", a guess "E3 min?".)
            Some(r) if m.key_minor.is_some() || m.root_guess => format!(
                "{}{}{}",
                crate::analysis::note_name(r),
                m.key_minor.map_or("", |minor| if minor { " min" } else { " maj" }),
                if m.root_guess { "?" } else { "" }
            ),
            Some(r) if m.cents.abs() >= 1.0 => format!("{} {:+.0}", crate::analysis::note_name(r), m.cents),
            Some(r) => crate::analysis::note_name(r),
            None => "-".into(),
        };
        // (The note it plays at is PLAYBACK's KEY.)
        let knobs = vec![
            (choice("Beat.Mode", "MODE", &modes, mode), Apply::PlayMode),
            (knob("Beat.Bpm", "BPM", bpm_text, bpm, (20.0, 999.0), 0.01, 1.0 / 400.0), Apply::Bpm),
            (knob("Beat.Beats", "BEATS", beats_text(m.beats), m.beats.unwrap_or(4.0), (1.0, 999.0), 1.0, TURN), Apply::Beats),
            (knob("Beat.Root", "ROOT", root_text, f64::from(root), (0.0, 127.0), 1.0, TURN), Apply::Root),
        ];
        page(&mut out, 1, "TEMPO / PITCH", knobs);
        // How this slice plays: its own speed against the song (Sync and Keys), backwards,
        // and the note it plays at.
        let mut knobs = Vec::new();
        if m.mode != PlayMode::Repitch && m.bpm.is_some() {
            let sp = m.speed_factor() * 100.0;
            knobs.push((knob("Play.Speed", "SPEED", format!("{sp:.0} %"), sp, (25.0, 400.0), 1.0, 1.0 / 375.0), Apply::Speed));
        }
        knobs.push((on_off("Play.Reverse", "REVERSE", m.reverse), Apply::Reverse));
        {
            use deckard_model::new_sampler::Plays;
            let names: Vec<String> = Plays::ALL.iter().map(|p| p.label().to_string()).collect();
            let cur = Plays::ALL.iter().position(|p| *p == Plays::of(&z)).unwrap_or(0);
            knobs.push((choice("Play.Plays", "PLAYS", &names, cur), Apply::Plays));
            // A loop's length, from its start: a sixteenth a step where the sample has a
            // tempo, else 10 ms.
            if Plays::of(&z) == Plays::Loop {
                let (la, lb) = z.loop_range;
                let (unit, step, name) = loop_unit(m.bpm, rate);
                let room = ((if z.play_range.1 > z.play_range.0 { z.play_range.1 } else { len as i64 - 1 }) - la + 1).max(2) as f64 / unit;
                let now = (lb - la + 1).max(1) as f64 / unit;
                let text = if name == "ms" { format!("{now:.0} ms") } else { format!("{:.2} {name}", now) };
                knobs.push((knob("Play.Loop", "LOOP LENGTH", text, now, (step.min(room), room), step, 1.0 / 64.0), Apply::LoopLength));
            }
        }
        if let Some(r) = m.root {
            let at = crate::beat::target_of(r, m.cents, z.tune).clamp(0, 127);
            knobs.push((knob("Play.Key", "KEY", crate::analysis::note_name(at as u8), at as f64, (0.0, 127.0), 1.0, TURN), Apply::PlaysAt));
        }
        page(&mut out, 1, "PLAYBACK", knobs);
        // The slice's tail and the sound's Slice Motion (a page of their own: the MK3
        // reaches a page's first four knobs).
        {
            use deckard_model::new_sampler::{Motion, TAIL_MAX};
            let tail = m.tail_ms();
            let text = if tail > 0.0 { format!("{tail:.0} ms") } else { "Off".into() };
            let motion = p.groups.get(g).and_then(|x| x.sounds.get(s)).and_then(|x| x.new_sampler).map_or(Motion::Off, |n| n.motion);
            let names: Vec<String> = Motion::ALL.iter().map(|x| x.label().to_string()).collect();
            let at = Motion::ALL.iter().position(|x| *x == motion).unwrap_or(0);
            let knobs = vec![
                (knob("Play.Tail", "TAIL", text, tail, (0.0, TAIL_MAX), 10.0, 1.0 / 200.0), Apply::Tail),
                (choice("Play.Motion", "MOTION", &names, at), Apply::Motion),
            ];
            page(&mut out, 1, "TAIL / MOTION", knobs);
        }
        if m.mode == deckard_model::new_sampler::PlayMode::Sync {
            use deckard_model::new_sampler::StartGrid;
            let grids: Vec<String> = StartGrid::ALL.iter().map(|g| g.label().to_string()).collect();
            let cur = StartGrid::ALL.iter().position(|g| *g == m.start).unwrap_or(0);
            page(&mut out, 1, "SYNC", vec![(choice("Beat.Start", "START", &grids, cur), Apply::StartGrid)]);
        }
        // Its stem lanes' levels (the display buttons mute them while this page shows;
        // SHIFT: solo).
        if m.has_stems() {
            let knobs = (0..m.stems.len())
                .filter_map(|k| {
                    let l = m.stems[k].as_ref()?;
                    let db = if l.gain > 0.0005 { (20.0 * l.gain.log10()).max(-60.0) } else { -60.0 };
                    let text = if db <= -60.0 { "-inf".into() } else { format!("{db:+.1} dB") };
                    let name = deckard_model::new_sampler::LANE_NAMES[k].to_uppercase();
                    Some((knob(&format!("Stem.Gain{k}"), &name, text, db, (-60.0, 6.0), 0.5, 1.0 / 132.0), Apply::LaneGain(k)))
                })
                .collect();
            page(&mut out, 1, "STEMS", knobs);
        }
        // Warp markers (the EDIT functions put them on the hits, quantize and clear them).
        if crate::beat::WARPS && m.bpm.is_some() {
            let grids: Vec<String> = crate::beat::WARP_GRIDS.iter().map(|g| g.1.to_string()).collect();
            let strength = st.warp_strength;
            page(
                &mut out,
                1,
                "WARP",
                vec![
                    (choice("Warp.Grid", "GRID", &grids, st.warp_grid.min(grids.len() - 1)), Apply::WarpGrid),
                    (
                        knob("Warp.Strength", "STRENGTH", format!("{:.0} %", strength * 100.0), strength, (0.0, 1.0), 0.05, 1.0 / 40.0),
                        Apply::WarpStrength,
                    ),
                    (knob("Warp.Count", "MARKERS", m.warps.len().to_string(), 0.0, (0.0, 0.0), 1.0, TURN), Apply::None),
                ],
            );
        }
    }

    // ZONE.
    page(
        &mut out,
        3,
        "PLAY RANGE",
        vec![
            frames("Zone.Start", "START", lo, (0.0, start_max), Apply::Zone("play_range/low")),
            frames("Zone.End", "END", hi, (end_min, last), Apply::Zone("play_range/high")),
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
            frames("Zone.LoopStart", "START", la, (0.0, last), Apply::Zone("loop_range/low")),
            frames("Zone.LoopEnd", "END", lb, (0.0, last), Apply::Zone("loop_range/high")),
            (
                knob("Zone.XFade", "XFADE", z.x_fade.max(0).to_string(), z.x_fade as f64, (0.0, xfade_max), 1.0, frame_turn),
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
    let ni_names: Vec<String> = (0..6).map(|m| SLICE_MODES[NI_TO_APP_SLICE_MODE[m]].to_string()).collect();
    let own_tempo = z.meta.as_ref().filter(|_| new_sampler).and_then(|m| m.bpm);
    let tempo = own_tempo.unwrap_or(p.tempo);
    let slice = [
        (choice("Slice.Mode", "MODE", &ni_names, APP_TO_NI_SLICE_MODE[st.slice_mode.min(5)]), Apply::SliceMode),
        (choice("Slice.Split", "SLICES", &SPLIT_COUNTS.map(|c| c.to_string()), st.split), Apply::SplitCount),
        (choice("Slice.Find", "SAMPLES", &FIND_COUNTS.map(|c| c.to_string()), st.find), Apply::FindCount),
        (choice("Slice.Random", "SAMPLES", &FIND_COUNTS.map(|c| c.to_string()), st.random), Apply::RandomCount),
        (choice("Slice.Grid", "SLICES", &GRID_DIVS.map(|d| d.1.to_string()), st.grid), Apply::GridDiv),
        (
            knob("Slice.Density", "SENSITIVITY", format!("{:.0} %", st.sensitivity * 100.0), st.sensitivity, (0.0, 1.0), 0.01, 1.0 / 100.0),
            Apply::Sensitivity,
        ),
        (on_off("Slice.ZeroX", "ZERO-X", st.zero_x), Apply::ZeroX),
        (choice("Slice.Apply", "PATTERN", &names(&PATTERN_MODES), st.pattern_mode), Apply::PatternMode),
        (choice("Slice.To", "TO", &names(&APPLY_TO), st.apply_to.min(1)), Apply::ApplyTo),
        (choice("Slice.Order", "ORDER", &names(&NOTE_ORDERS), usize::from(st.shuffle)), Apply::NoteOrder),
        // Slices follow the song's tempo; a New Sampler zone's with a tempo, its own.
        (knob("Slice.BpmMode", "BPM", if own_tempo.is_some() { "Sample" } else { "Song" }.into(), 0.0, (0.0, 0.0), 1.0, TURN), Apply::None),
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
    NI_TO_APP_SLICE_MODE[(ni.round().max(0.0) as usize).min(5)]
}
