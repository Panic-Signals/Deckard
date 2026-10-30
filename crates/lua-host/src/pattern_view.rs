//! What Maschine's native pattern editor widget (`NI.GUI.insertEventPatternEditor`)
//! shows on the controller's displays, drawn as shapes over its box (the Step, Events
//! and Pattern pages; MK3 manual, "Working with Patterns and Clips").
//!
//! - The overview (left display, the editor with an overview source) shows the whole
//!   pattern: a row per sound of the group, or with the pads in Keyboard mode a row per
//!   pitch of the focused sound; a frame around what the right display shows.
//! - The detail (right display) shows part of it, zoomed and scrolled as the scripts
//!   set it (`api::pattern_editor`): in Step mode the focused sound's notes in a row at
//!   the top with their velocities (or with PROBABILITY on, their probabilities)
//!   underneath, each a thin stem with a dot on top; else a lane per sound (doubled in height
//!   by knob 7); in Keyboard mode the focused sound's notes by pitch, on a piano roll
//!   drawn as the app's pattern editor draws it (pitches outside the scale in red).
//!
//! Both have a timeline at the top and the playhead as a white line. A note's velocity
//! is its opacity; selected notes have a white border. Sizes and colours are the style
//! sheet's (`HW/MaschineStudio/ArrangerPage.txt`): 16-pixel headers, 8-pixel overview
//! rows, a 24-pixel note row, notes in palette 1 between alpha 68 and 216.

use crate::state::State;
use deckard_model::{Color, Note};
use deckard_nwl::draw::{At, Fill, Shape};

/// What the editors show, as the scripts set it up.
#[derive(Debug, Clone, Copy, Default)]
pub struct View {
    /// Step mode: the focused sound's steps (the pads hold 16 of them).
    pub step: bool,
    /// The pads play the focused sound's pitches (Keyboard mode).
    pub keyboard: bool,
    /// Step mode shows the steps' probabilities, not their velocities.
    pub probability: bool,
    /// The focused group's root note (the scale overlay's).
    pub root: i64,
}

const HEADER: i32 = 16;
const ROW: i32 = 8;
const LANE: i32 = 24;
/// The dot on a velocity or probability stem.
const DOT: i32 = 7;
/// A sound's lane on the right display when it shows them all (Events page), and with
/// the lanes doubled (knob 7).
const LANE_ALL: i32 = 12;
const LANE_TALL: i32 = 24;
/// The right display's lanes area.
const LANES_HEIGHT: i32 = 192;
const NOTE_PALETTE: u32 = 1;
const ALPHA: (f32, f32) = (68.0, 216.0);
const DEFAULT_SOUND_COLOR: u32 = 15;

/// The small pixel font of the displays' labels (the sheet's timeline font is one of
/// Maschine's built-in bitmap fonts).
pub(crate) const TIMELINE_FONT: (&str, f32) = ("Shared/prg65_j.ttf", 11.0);
const TIMELINE: [u8; 4] = [38, 38, 38, 255];
const TIMELINE_TEXT: [u8; 4] = [170, 170, 170, 255];
const STEP_LINE: [u8; 4] = [255, 255, 255, 12];
const BEAT_LINE: [u8; 4] = [255, 255, 255, 26];
const BAR_LINE: [u8; 4] = [255, 255, 255, 56];
const ROW_LINE: [u8; 4] = [255, 255, 255, 10];
/// The piano roll, as the app's pattern editor draws it (`apps/viewer` layout.rs): white
/// and black keys' rows apart, a line under each C's, and the pitches outside the scale
/// tinted red, their keys more strongly than their rows.
const WHITE_ROW: [u8; 4] = [0x21, 0x21, 0x21, 255];
const BLACK_ROW: [u8; 4] = [0x18, 0x18, 0x18, 255];
const C_LINE: [u8; 4] = [255, 255, 255, 24];
const OUT_OF_SCALE_ROW: [u8; 4] = [220, 40, 40, 28];
const OUT_OF_SCALE_KEY: [u8; 4] = [220, 40, 40, 110];
const KEY_GAP: [u8; 4] = [0x18, 0x18, 0x18, 255];
/// The keys beside it.
const WHITE_KEY: [u8; 4] = [0x9b, 0x9b, 0x9b, 255];
const BLACK_KEY: [u8; 4] = [0x18, 0x18, 0x18, 255];
const KEY_LABEL: [u8; 4] = [0, 0, 0, 255];
/// The keys' box starts this far below the editor's rows (its `outer-margin-top` 18
/// against the editor's 16-pixel timeline).
const KEYS_BELOW_ROWS: i32 = 2;
const WHITE: [u8; 4] = [255, 255, 255, 255];
const FOCUS_ROW: [u8; 4] = [255, 255, 255, 14];

pub(crate) fn rect(x0: At, y0: At, x1: At, y1: At, c: [u8; 4]) -> Shape {
    Shape::rect(x0, y0, x1, y1, Fill::Rgba(c))
}

/// A note of a sound coloured `colour` (palette entry 0 is white, the colours follow).
fn note_fill(colour: u32, velocity: i64) -> Fill {
    let v = velocity.clamp(1, 127) as f32 / 127.0;
    Fill::Palette { palette: NOTE_PALETTE, index: colour + 1, alpha: (ALPHA.0 + (ALPHA.1 - ALPHA.0) * v) as u8 }
}

/// A one-pixel white frame from (x0, y0) to (x1, y1).
pub(crate) fn frame(out: &mut Vec<Shape>, x0: At, y0: At, x1: At, y1: At) {
    let one = |a: At, d: i32| At { frac: a.frac, px: a.px + d };
    out.push(rect(x0, y0, x1, one(y0, 1), WHITE));
    out.push(rect(x0, one(y1, -1), x1, y1, WHITE));
    out.push(rect(x0, y0, one(x0, 1), y1, WHITE));
    out.push(rect(one(x1, -1), y0, x1, y1, WHITE));
}

/// The shapes for one editor: the overview or the detail.
pub fn shapes(st: &State, overview: bool, view: View) -> Vec<Shape> {
    let Some(pattern) = focused(st) else { return Vec::new() };
    let (start, end) = (pattern.start, pattern.end.max(pattern.start + 1));
    let grid = st.step_grid.max(1);
    // What the right display shows: as the scripts zoomed and scrolled it.
    let shown = crate::api::pattern_editor::window(st).unwrap_or((start, end));
    let (t0, t1) = if overview { (start, end) } else { shown };
    let x = |t: i64| At::frac((t - t0) as f32 / (t1 - t0).max(1) as f32);
    let mut out = Vec::new();

    // The timeline: beats marked, bars numbered (beats too when there's room).
    let bar = pattern.bar.max(1);
    let beat = pattern.beat.max(1);
    out.push(rect(At::frac(0.0), At::px(0), At::frac(1.0), At::px(HEADER), TIMELINE));
    let beats_numbered = (t1 - t0) <= 2 * bar;
    let mut t = t0.div_euclid(beat) * beat;
    while t < t1 {
        if t >= t0 {
            let on_bar = t.rem_euclid(bar) == 0;
            let tick_top = if on_bar { 3 } else { HEADER - 4 };
            out.push(rect(x(t), At::px(tick_top), At { px: 1, ..x(t) }, At::px(HEADER), TIMELINE_TEXT));
            if on_bar || beats_numbered {
                let label = if on_bar && !beats_numbered {
                    format!("{}", t.div_euclid(bar) + 1)
                } else {
                    format!("{}.{}", t.div_euclid(bar) + 1, t.rem_euclid(bar) / beat + 1)
                };
                let mut s = rect(At { px: 3, ..x(t) }, At::px(0), At { px: 40, ..x(t) }, At::px(HEADER), TIMELINE_TEXT);
                s.text = Some((label, TIMELINE_FONT.0.into(), TIMELINE_FONT.1));
                out.push(s);
            }
        }
        t += beat;
    }

    // Keyboard mode's piano roll: a row per pitch, as the app draws it.
    let pitch_lo = crate::api::pattern_editor::pitch_low(st, view.root);
    let rows = crate::api::pattern_editor::PITCH_ROWS;
    let row_y = |p: i64| {
        let k = (p - pitch_lo + 1) as f32 / rows as f32;
        At { frac: 1.0 - k, px: (HEADER as f32 * k) as i32 }
    };
    if view.keyboard && !overview {
        for p in pitch_lo..pitch_lo + rows {
            let (y0, y1) = (row_y(p), row_y(p - 1));
            out.push(rect(At::frac(0.0), y0, At::frac(1.0), y1, if is_black(p) { BLACK_ROW } else { WHITE_ROW }));
            if !crate::api::scales::in_scale(st.scale.bank, st.scale.scale, view.root, p) {
                out.push(rect(At::frac(0.0), y0, At::frac(1.0), y1, OUT_OF_SCALE_ROW));
            }
            if p.rem_euclid(12) == 0 {
                out.push(rect(At::frac(0.0), At { px: y1.px - 1, ..y1 }, At::frac(1.0), y1, C_LINE));
            }
        }
    }

    // The grid under the notes: steps (when there's room), beats, bars.
    let steps_drawn = (t1 - t0) / grid <= 64;
    let mut t = t0.div_euclid(grid) * grid;
    while t < t1 {
        if t > t0 {
            let c = if t.rem_euclid(bar) == 0 {
                BAR_LINE
            } else if t.rem_euclid(beat) == 0 {
                BEAT_LINE
            } else {
                STEP_LINE
            };
            if steps_drawn || c != STEP_LINE {
                out.push(rect(x(t), At::px(HEADER), At { px: 1, ..x(t) }, At::frac(1.0), c));
            }
        }
        t += grid;
    }
    // Past the pattern's end: dimmed.
    if t1 > end {
        out.push(rect(x(end), At::px(HEADER), At::frac(1.0), At::frac(1.0), [0, 0, 0, 150]));
    }

    let sel = |n: &Note| pattern.selected.contains(n);
    let visible = |n: &Note| n.position < t1 && n.position + n.length > t0;
    let colour = |sound: usize| pattern.colours.get(sound).copied().unwrap_or(DEFAULT_SOUND_COLOR);
    let focus = st.focus.sound;

    if !overview && !view.keyboard && !view.step {
        // Every sound of the group in a lane, as the sound list beside it; with tall
        // lanes the ones around the focused sound.
        let lane = if crate::api::pattern_editor::tall_lanes(st) { LANE_TALL } else { LANE_ALL };
        let rows = (LANES_HEIGHT / lane) as usize;
        let first = (focus + 1).saturating_sub(rows).min(16 - rows);
        let row = |s: usize| (s >= first && s < first + rows).then(|| HEADER + (s - first) as i32 * lane);
        if let Some(y) = row(focus) {
            out.push(rect(At::frac(0.0), At::px(y), At::frac(1.0), At::px(y + lane), FOCUS_ROW));
        }
        // Audio plug-ins' loops in their lanes (as Maschine: 10 pixels of a 12-pixel
        // lane, 18 of 24).
        let (h, top) = if lane == LANE_TALL { (18, 3) } else { (10, 1) };
        for s in 0..16 {
            if let Some(y) = row(s) {
                audio_lane(&mut out, st, s, &pattern, (t0, t1), &x, (At::px(y + top), At::px(y + top + h)), false);
            }
        }
        for n in pattern.notes.iter().filter(|n| visible(n)) {
            let Some(y) = row(n.sound) else { continue };
            note(&mut out, x(n.position), At::px(y + 1), x(n.position + n.length), At::px(y + lane), colour(n.sound), n.velocity, sel(n));
        }
    } else if view.keyboard || !overview {
        // One sound: by pitch (Keyboard mode), else a row and velocities below.
        let mine: Vec<&Note> = pattern.notes.iter().filter(|n| n.sound == focus).collect();
        if view.keyboard && !overview {
            audio_lane(&mut out, st, focus, &pattern, (t0, t1), &x, (At::px(HEADER), At::frac(1.0)), true);
            // The piano roll's two octaves (knob 8 scrolls them).
            for n in mine.iter().filter(|n| visible(n)).filter(|n| (pitch_lo..pitch_lo + rows).contains(&n.pitch)) {
                let (y0, y1) = (row_y(n.pitch), row_y(n.pitch - 1));
                note(
                    &mut out,
                    x(n.position),
                    y0,
                    x(n.position + n.length),
                    At { px: y1.px - 1, ..y1 },
                    colour(n.sound),
                    n.velocity,
                    sel(n),
                );
            }
        } else if view.keyboard {
            // The overview: every pitch the sound plays, two octaves at least.
            let lo = mine.iter().map(|n| n.pitch).min().unwrap_or(60);
            let hi = mine.iter().map(|n| n.pitch).max().unwrap_or(60);
            let span = (hi - lo + 1).max(24);
            let lo = (lo - (span - (hi - lo + 1)) / 2).clamp(0, 128 - span);
            let y = |p: i64| At {
                frac: 1.0 - (p - lo + 1) as f32 / span as f32,
                px: (HEADER as f32 * (p - lo + 1) as f32 / span as f32) as i32,
            };
            for p in lo..lo + span {
                if p.rem_euclid(12) == 0 {
                    out.push(rect(At::frac(0.0), y(p), At::frac(1.0), At { px: 1, ..y(p) }, ROW_LINE));
                }
            }
            for n in mine.iter().filter(|n| visible(n)) {
                let (y0, y1) = (y(n.pitch), y(n.pitch - 1));
                note(&mut out, x(n.position), y0, x(n.position + n.length), y1, colour(n.sound), n.velocity, sel(n));
            }
        } else {
            let lane = (At::px(HEADER + 1), At::px(HEADER + 1 + LANE));
            audio_lane(&mut out, st, focus, &pattern, (t0, t1), &x, (At::px(HEADER + 4), At::px(HEADER + LANE - 2)), false);
            for n in mine.iter().filter(|n| visible(n)) {
                let (x0, x1) = (x(n.position), x(n.position + n.length));
                note(&mut out, x0, lane.0, x1, lane.1, colour(n.sound), n.velocity, sel(n));
                // Below, its velocity (or probability) as a stem up from the bottom with a
                // dot on top, at the note's start; the area runs from below the row
                // (with room for the dot) to the bottom.
                let v = if view.probability { n.probability.clamp(0, 100) as f32 / 100.0 } else { n.velocity.clamp(0, 127) as f32 / 127.0 };
                let top = (HEADER + LANE + 8 + DOT / 2) as f32;
                let bottom = DOT / 2;
                let y = At { frac: 1.0 - v, px: (v * top + (1.0 - v) * -(bottom as f32)).round() as i32 };
                let fill = if sel(n) {
                    Fill::Rgba(WHITE)
                } else {
                    Fill::Palette { palette: NOTE_PALETTE, index: colour(n.sound) + 1, alpha: 255 }
                };
                let at = At { px: x0.px + DOT / 2, ..x0 };
                out.push(Shape::rect(At { px: at.px - 1, ..at }, y, At { px: at.px + 1, ..at }, At::frac(1.0), fill));
                let mut dot = Shape::rect(
                    At { px: at.px - DOT / 2, ..at },
                    At { px: y.px - DOT / 2, ..y },
                    At { px: at.px - DOT / 2 + DOT, ..at },
                    At { px: y.px - DOT / 2 + DOT, ..y },
                    fill,
                );
                dot.round = true;
                out.push(dot);
            }
        }
    } else {
        // Every sound of the group, a row each.
        for s in 0..16 {
            let y0 = At::px(HEADER + s as i32 * ROW);
            if s == focus {
                out.push(rect(At::frac(0.0), y0, At::frac(1.0), At::px(HEADER + (s as i32 + 1) * ROW), FOCUS_ROW));
            }
        }
        for s in 0..16 {
            let y = HEADER + s as i32 * ROW;
            audio_lane(&mut out, st, s, &pattern, (t0, t1), &x, (At::px(y + 1), At::px(y + ROW - 1)), false);
        }
        for n in pattern.notes.iter().filter(|n| visible(n)).filter(|n| n.sound < 16) {
            let y0 = At::px(HEADER + n.sound as i32 * ROW + 1);
            let y1 = At::px(HEADER + (n.sound as i32 + 1) * ROW);
            note(&mut out, x(n.position), y0, x(n.position + n.length), y1, colour(n.sound), n.velocity, sel(n));
        }
    }

    // Where the right display is, on the overview.
    if overview && shown != (start, end) {
        frame(&mut out, x(shown.0), At::px(HEADER), x(shown.1), At::frac(1.0));
    }
    // The playhead.
    if let Some(p) = crate::api::events::play_tick(st).filter(|p| (t0..t1).contains(p)) {
        out.push(rect(x(p), At::px(0), At { px: 1, ..x(p) }, At::frac(1.0), WHITE));
    }
    out
}

/// Waveform columns across a display.
const WAVE_COLUMNS: i64 = 240;
/// The style sheet's loop colours (`ArrangerPage.txt`, "CLIP EDITOR - … WAVEDISPLAY"):
/// drum lanes the sound's colour (palette 1), repeats palette 19, greys muted or off;
/// the piano roll palettes 7 and 6, white over it muted.
const WAVE_MUTED: [u8; 4] = [102, 102, 102, 255];
const WAVE_MUTED_REPEAT: [u8; 4] = [79, 79, 79, 255];
const WAVE_MUTED_GATE: [u8; 4] = [118, 118, 118, 255];
const WAVE_PIANO_MUTED: [u8; 4] = [255, 255, 255, 51];
const WAVE_PIANO_MUTED_REPEAT: [u8; 4] = [255, 255, 255, 26];

/// Sound `s`'s Audio plug-in loop between `y0` and `y1` (Maschine's `ClipAudioLane`):
/// from the pattern's start, a loop's length in beats at a time (a repeat after the
/// first), on the same ticks as the steps whatever the tempo. Loop mode: in colour
/// where it's on in the pattern; Gate mode: in colour under the sound's notes.
#[allow(clippy::too_many_arguments)]
fn audio_lane(
    out: &mut Vec<Shape>,
    st: &State,
    s: usize,
    pattern: &Pattern,
    (t0, t1): (i64, i64),
    x: &dyn Fn(i64) -> At,
    (y0, y1): (At, At),
    piano: bool,
) {
    let Some(p) = st.project() else { return };
    let g = st.focus.group;
    let Some(snd) = p.groups.get(g).and_then(|gr| gr.sounds.get(s)) else { return };
    let Some(a) = snd.audio.as_ref().filter(|_| snd.instrument == "AudioModule") else { return };
    let cue = st.focus.pattern.and_then(|pi| p.groups[g].patterns.get(pi)).and_then(|pt| pt.audio.iter().find(|c| c.sound == s).copied());
    let k = cue.map_or(a.focused, |c| c.take);
    let (Some(take), Some(peaks)) = (a.takes.get(k), st.loop_peaks.get(&(g, s, k))) else { return };
    let loop_ticks = (take.beats * deckard_model::PPQ as f64) as i64;
    if loop_ticks <= 0 || peaks.is_empty() {
        return;
    }
    let gate = a.mode == deckard_model::AudioMode::Gate;
    let muted = snd.muted || cue.is_none();
    if piano && muted && !gate {
        return;
    }
    let colour = pattern.colours.get(s).copied().unwrap_or(DEFAULT_SOUND_COLOR);
    let under_note = |t: i64| pattern.notes.iter().any(|n| n.sound == s && n.position <= t && t < n.position + n.length);
    let fill = |t: i64, repeat: bool| -> Fill {
        let lit = if gate { under_note(t) } else { true };
        match (piano, muted, lit, repeat) {
            (false, false, true, false) => Fill::Palette { palette: 1, index: colour + 1, alpha: 255 },
            (false, false, true, true) => Fill::Palette { palette: if gate { 1 } else { 19 }, index: colour + 1, alpha: 255 },
            (false, true, true, _) if gate => Fill::Rgba(WAVE_MUTED_GATE),
            (false, true, _, true) => Fill::Rgba(WAVE_MUTED_REPEAT),
            (false, ..) => Fill::Rgba(WAVE_MUTED),
            (true, false, true, false) => Fill::Palette { palette: 7, index: colour + 1, alpha: 255 },
            (true, false, true, true) => Fill::Palette { palette: if gate { 7 } else { 6 }, index: colour + 1, alpha: 255 },
            (true, _, _, true) => Fill::Rgba(WAVE_PIANO_MUTED_REPEAT),
            (true, ..) => Fill::Rgba(WAVE_PIANO_MUTED),
        }
    };
    let mid = At { frac: (y0.frac + y1.frac) / 2.0, px: (y0.px + y1.px) / 2 };
    let half = ((y1.frac - y0.frac) / 2.0, (y1.px - y0.px) as f32 / 2.0);
    let at = |v: f32| At { frac: mid.frac - v * half.0, px: mid.px - (v * half.1).round() as i32 };
    let (from, to) = (t0.max(pattern.start), t1.min(pattern.end));
    let step = ((t1 - t0) / WAVE_COLUMNS).max(1);
    let mut t = from;
    while t < to {
        let rel = t - pattern.start;
        let (rep, within) = (rel / loop_ticks, rel % loop_ticks);
        let i = ((within as f64 / loop_ticks as f64 * peaks.len() as f64) as usize).min(peaks.len() - 1);
        let (lo, hi) = peaks[i];
        let (top, mut bottom) = (at(hi.clamp(-1.0, 1.0)), at(lo.clamp(-1.0, 1.0)));
        // At least a pixel high (silence a line).
        if bottom.frac == top.frac && bottom.px <= top.px {
            bottom.px = top.px + 1;
        }
        out.push(Shape::rect(x(t), top, x((t + step).min(to)), bottom, fill(t, rep > 0)));
        t += step;
    }
}

/// The focused pattern as the editors draw it.
struct Pattern {
    start: i64,
    end: i64,
    bar: i64,
    beat: i64,
    notes: Vec<Note>,
    selected: Vec<Note>,
    /// Each sound's colour index.
    colours: Vec<u32>,
}

fn focused(st: &State) -> Option<Pattern> {
    let (g, p) = (st.focus.group, st.focus.pattern?);
    let pr = st.project()?;
    let group = pr.groups.get(g)?;
    let pat = group.patterns.get(p)?;
    let len = if pat.length > 0 { pat.length } else { pr.bar_length() };
    let selected = match &st.selection {
        Some((sg, sp, sel)) if (*sg, *sp) == (g, p) => sel.clone(),
        _ => Vec::new(),
    };
    let colours = group
        .sounds
        .iter()
        .map(|s| match s.color {
            Color::Palette(i) => i as u32,
            Color::Default => DEFAULT_SOUND_COLOR,
        })
        .collect();
    Some(Pattern {
        start: pat.start,
        end: pat.start + len,
        bar: pr.bar_length(),
        beat: deckard_model::PPQ * 4 / pr.denominator.max(1),
        notes: pat.notes.clone(),
        selected,
        colours,
    })
}

#[allow(clippy::too_many_arguments)]
fn note(out: &mut Vec<Shape>, x0: At, y0: At, x1: At, y1: At, colour: u32, velocity: i64, selected: bool) {
    let x1 = At { px: x1.px - 1, ..x1 };
    out.push(Shape::rect(x0, y0, x1, y1, note_fill(colour, velocity)));
    if selected {
        frame(out, x0, y0, x1, y1);
    }
}

/// A black key's pitch.
fn is_black(p: i64) -> bool {
    matches!(p.rem_euclid(12), 1 | 3 | 6 | 8 | 10)
}

/// The keys beside the piano roll (`NI.GUI.insertPianorollKeyboard`), row for row with
/// it and as the app draws them: white and black keys, each C named with its octave, the
/// pitches outside the scale tinted red, the keys sounding now lit.
pub fn keyboard(st: &State, view: View) -> Vec<Shape> {
    let lo = crate::api::pattern_editor::pitch_low(st, view.root);
    let rows = crate::api::pattern_editor::PITCH_ROWS;
    let sounding: Vec<i64> =
        st.notes_on.all().filter(|((g, s, _), _)| (*g, *s) == (st.focus.group, st.focus.sound)).map(|((_, _, p), _)| p).collect();
    // Row for row with the editor, whose rows start `KEYS_BELOW_ROWS` above this box.
    let y = |p: i64| {
        let k = (p - lo + 1) as f32 / rows as f32;
        At { frac: 1.0 - k, px: (-(KEYS_BELOW_ROWS as f32) * k).round() as i32 }
    };
    let mut out = Vec::new();
    for p in lo..lo + rows {
        let (y0, y1) = (y(p), y(p - 1));
        let bottom = At { px: y1.px - 1, ..y1 };
        let fill = if sounding.contains(&p) {
            WHITE
        } else if is_black(p) {
            BLACK_KEY
        } else {
            WHITE_KEY
        };
        out.push(rect(At::frac(0.0), y0, At::frac(1.0), bottom, fill));
        if !crate::api::scales::in_scale(st.scale.bank, st.scale.scale, view.root, p) {
            out.push(rect(At::frac(0.0), y0, At::frac(1.0), bottom, OUT_OF_SCALE_KEY));
        }
        out.push(rect(At::frac(0.0), bottom, At::frac(1.0), y1, KEY_GAP));
        if p.rem_euclid(12) == 0 {
            let mut label = rect(At::px(2), y0, At::frac(1.0), bottom, KEY_LABEL);
            label.text = Some((format!("C{}", p.div_euclid(12) - 2), TIMELINE_FONT.0.into(), 8.0));
            out.push(label);
        }
    }
    out
}

/// Where DECKARD's own picture of a plug-in is (its tile in the chain, or its logo on
/// the right display), by the style sheet's key (its name in small letters, without
/// spaces or parentheses).
pub(crate) fn plugin_picture(key: &str, thumb: bool) -> String {
    format!("DECKARD/Plugins/{key}_{}.png", if thumb { "Thumb" } else { "Logo" })
}
