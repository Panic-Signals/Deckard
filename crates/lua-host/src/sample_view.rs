//! What the Sampling page's native displays show of the focused zone's sample
//! ([`State::sample`], from the app), drawn as shapes over their boxes:
//!
//! - The wave editors (`insertSampleOwnerWaveEditor` on EDIT and ZONE,
//!   `insertSliceWaveEditor` on SLICE): the waveform, in the sound's colour inside the
//!   play range and grey outside it, the selection (EDIT), the loop, the slices with the
//!   focused one lit (SLICE), the start and end of the play range and the playhead. The
//!   one marked `setHWOverview()` (left display) shows the whole sample with a frame
//!   around the part the right display shows.
//! - The zone map (`insertZoneMapEditor`, ZONE's left display): keys across, velocities
//!   up, each zone a box, the focused one lit.

use crate::pattern_view::rect;
use crate::state::{SampleView, State};
use deckard_nwl::draw::{At, Fill, Shape};

const BG: [u8; 4] = [0, 0, 0, 255];
const OUTSIDE: [u8; 4] = [80, 80, 80, 255];
const UNCOLOURED: [u8; 4] = [200, 200, 200, 255];
const MIDLINE: [u8; 4] = [40, 40, 40, 255];
const MARKER: [u8; 4] = [255, 255, 255, 255];
const SELECTION: [u8; 4] = [255, 255, 255, 46];
const FOCUS_SLICE: [u8; 4] = [255, 255, 255, 40];
const SLICE_LINE: [u8; 4] = [255, 255, 255, 150];
const LOOP_LINE: [u8; 4] = [255, 255, 255, 200];
const KEYS_BG: [u8; 4] = [26, 26, 26, 255];
const OCTAVE: [u8; 4] = [10, 10, 10, 255];
const ZONE: [u8; 4] = [120, 120, 120, 160];
/// The waveform's half height, as a part of the box's.
const HALF: f32 = 0.46;

fn colour(s: &SampleView, alpha: u8) -> Fill {
    match s.color {
        Some(i) => Fill::Palette { palette: 1, index: i + 1, alpha },
        None => Fill::Rgba([UNCOLOURED[0], UNCOLOURED[1], UNCOLOURED[2], alpha]),
    }
}

fn vline(x: At, w: i32, c: [u8; 4]) -> Shape {
    rect(x, At::frac(0.0), At { px: x.px + w, ..x }, At::frac(1.0), c)
}

/// A wave editor: the overview (`overview`) or the zoomed part; with the slices (`slices`).
pub fn wave(st: &State, overview: bool, slices: bool) -> Vec<Shape> {
    let Some(s) = st.sample.as_ref().filter(|s| s.frames > 0) else { return Vec::new() };
    let (peaks, (v0, v1)) = if overview { (&s.overview, (0.0, s.frames as f64)) } else { (&s.detail, s.view) };
    let span = (v1 - v0).max(1.0);
    let x = |f: f64| At::frac(((f - v0) / span) as f32);
    let shown = |f: usize| (f as f64) >= v0 && (f as f64) <= v1;
    let mut out = vec![rect(At::frac(0.0), At::frac(0.0), At::frac(1.0), At::frac(1.0), BG)];
    // Behind the wave: the selection, the focused slice.
    if !slices && let Some((a, b)) = s.selection {
        out.push(rect(x(a as f64), At::frac(0.0), x(b as f64), At::frac(1.0), SELECTION));
    }
    if slices && let Some(i) = s.focus_slice.filter(|&i| i < s.slices.len()) {
        let end = s.slices.get(i + 1).copied().unwrap_or(s.frames);
        out.push(rect(x(s.slices[i] as f64), At::frac(0.0), x(end as f64), At::frac(1.0), FOCUS_SLICE));
    }
    out.push(rect(At::frac(0.0), At::frac(0.5), At::frac(1.0), At { px: 1, ..At::frac(0.5) }, MIDLINE));
    let n = peaks.len().max(1);
    for (i, &(mn, mx)) in peaks.iter().enumerate() {
        let f = v0 + span * (i as f64 + 0.5) / n as f64;
        let inside = f >= s.play.0 as f64 && f < s.play.1 as f64;
        let fill = if inside { colour(s, 255) } else { Fill::Rgba(OUTSIDE) };
        let (top, bottom) = (0.5 - mx.clamp(-1.0, 1.0) * HALF, 0.5 - mn.clamp(-1.0, 1.0) * HALF);
        // At least a pixel, so silence shows as a line.
        out.push(Shape::rect(
            At::frac(i as f32 / n as f32),
            At::frac(top),
            At { px: 1, ..At::frac(i as f32 / n as f32) },
            At { px: 1, ..At::frac(bottom) },
            fill,
        ));
    }
    if slices {
        for &f in s.slices.iter().skip(1).filter(|&&f| shown(f)) {
            out.push(vline(x(f as f64), 1, SLICE_LINE));
        }
    }
    if let Some((a, b)) = s.looped {
        for f in [a, b].into_iter().filter(|&f| shown(f)) {
            out.push(vline(x(f as f64), 1, LOOP_LINE));
        }
    }
    for f in [s.play.0, s.play.1].into_iter().filter(|&f| shown(f)) {
        out.push(vline(x(f as f64), 2, MARKER));
    }
    // Where the right display is, on the overview.
    if overview && (s.view.1 - s.view.0) < s.frames as f64 - 1.0 {
        let (a, b) = (x(s.view.0), x(s.view.1));
        crate::pattern_view::frame(&mut out, a, At::frac(0.0), b, At::frac(1.0));
    }
    if let Some(p) = s.playhead.filter(|&p| shown(p)) {
        out.push(vline(x(p as f64), 1, MARKER));
    }
    out
}

/// The zone map: 128 keys across, velocities 0–127 up.
pub fn zone_map(st: &State) -> Vec<Shape> {
    let Some(s) = st.sample.as_ref() else { return Vec::new() };
    let kx = |k: i64| At::frac(k as f32 / 128.0);
    let vy = |v: i64| At::frac(1.0 - v as f32 / 128.0);
    let mut out = vec![rect(At::frac(0.0), At::frac(0.0), At::frac(1.0), At::frac(1.0), KEYS_BG)];
    for c in (12..128).step_by(12) {
        out.push(vline(kx(c), 1, OCTAVE));
    }
    // The focused zone last, over the others.
    let order = (0..s.zones.len()).filter(|&i| i != s.focus_zone).chain((s.focus_zone < s.zones.len()).then_some(s.focus_zone));
    for i in order {
        let ((k0, k1), (v0, v1)) = s.zones[i];
        let (x0, x1, y0, y1) = (kx(k0), kx(k1 + 1), vy(v1 + 1), vy(v0));
        if i == s.focus_zone {
            out.push(Shape::rect(x0, y0, x1, y1, colour(s, 220)));
            crate::pattern_view::frame(&mut out, x0, y0, x1, y1);
        } else {
            out.push(rect(x0, y0, x1, y1, ZONE));
        }
    }
    out
}
