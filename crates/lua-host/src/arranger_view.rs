//! What Maschine's native arranger widget (`NI.GUI.insertArrangerPanel`) shows on the
//! controller's displays in the Song view (MK3 manual, "Arranging Sections in the Song
//! View"), drawn as shapes over its box.
//!
//! - The overview (left display, the panel marked `setHWScreen(true)`) shows the whole
//!   song: a timeline, a line in each section's scene colour, a thin lane per group, and
//!   a frame around what the right display shows.
//! - The detail (right display) shows part of it, zoomed and scrolled by knobs 5 and 6
//!   (`api::arranger`): the sections' names along the top, the focused one lit and
//!   underlined in its colour, a timeline, and a lane per group of the focused bank of
//!   eight, the focused group's lit.
//!
//! In the lanes each section shows the patterns its scene plays, in the pattern's
//! colour, with a line where the pattern repeats. The playhead is a white line. Sizes
//! and colours are the style sheet's (`HW/MaschineStudio/ArrangerPage.txt`): a 22-pixel
//! section header, an 18-pixel timeline (14 on the overview), 26-pixel lanes
//! overlapping by 2 (10 on the overview), lanes rgb(26,26,26) apart by rgb(16,16,16).

use crate::pattern_view::{TIMELINE_FONT, frame, rect};
use crate::state::{Playing, State};
use deckard_model::{Color, Project};
use deckard_nwl::draw::{At, Fill, Shape};

const HEADER: i32 = 22;
const TIMELINE: i32 = 18;
const TIMELINE_OVERVIEW: i32 = 14;
const LANE: i32 = 24;
const LANE_OVERVIEW: i32 = 8;
/// The overview's scene colour line: 2 pixels, a pixel below the timeline.
const SCENE_LINE: (i32, i32) = (1, 2);
const LANES_SHOWN: usize = 8;
const FOCUS_LINE: i32 = 3;

const HEADER_BG: [u8; 4] = [26, 26, 26, 255];
const SECTION_BG: [u8; 4] = [51, 51, 51, 255];
const SECTION_FOCUS_BG: [u8; 4] = [90, 90, 90, 255];
const SECTION_TEXT: [u8; 4] = [170, 170, 170, 255];
const PATTERN_TEXT: [u8; 4] = [0, 0, 0, 220];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const TIMELINE_BG: [u8; 4] = [16, 16, 16, 255];
const TIMELINE_TEXT: [u8; 4] = [132, 132, 132, 255];
const LANE_BG: [u8; 4] = [26, 26, 26, 255];
const LANE_GAP: [u8; 4] = [16, 16, 16, 255];
const BAR_GAP: [u8; 4] = [17, 17, 17, 255];
const FOCUS_LANE: [u8; 4] = [255, 255, 255, 8];
const REPEAT_LINE: [u8; 4] = [0, 0, 0, 90];
/// Past the song's end.
const AFTER_END: [u8; 4] = [0, 0, 0, 120];
const PALETTE: u32 = 1;
const DEFAULT_GROUP_COLOR: u32 = 1;
const DEFAULT_SCENE_COLOR: u32 = 4;

fn index(c: Color, default: u32) -> u32 {
    match c {
        Color::Palette(i) => i as u32,
        Color::Default => default,
    }
}

fn colour(index: u32, alpha: u8) -> Fill {
    Fill::Palette { palette: PALETTE, index: index + 1, alpha }
}

/// The shapes for one panel: the overview or the detail.
pub fn shapes(st: &State, overview: bool) -> Vec<Shape> {
    let Some(p) = st.project() else { return Vec::new() };
    let song = crate::api::arranger::song_length(st);
    let shown = crate::api::arranger::window(st);
    let (t0, t1) = if overview { (0, song) } else { shown };
    let x = |t: i64| At::frac((t - t0) as f32 / (t1 - t0).max(1) as f32);
    let px = |a: At, d: i32| At { px: a.px + d, ..a };
    let mut out = Vec::new();
    let bar = p.bar_length().max(1);
    let end = p.arrangement_length();
    let focus = st.focus.section.filter(|&s| s < p.sections.len());

    // The sections' names (the detail), each from its start to its end.
    let mut y = 0;
    if !overview {
        out.push(rect(At::frac(0.0), At::px(0), At::frac(1.0), At::px(HEADER), HEADER_BG));
        for (i, s) in p.sections.iter().enumerate() {
            let (a, b) = (p.section_start(i), p.section_start(i) + s.length);
            if b <= t0 || a >= t1 {
                continue;
            }
            let focused = focus == Some(i);
            out.push(rect(x(a), At::px(0), px(x(b), -1), At::px(HEADER), if focused { SECTION_FOCUS_BG } else { SECTION_BG }));
            let scene = s.scene.and_then(|k| p.scenes.get(k));
            if let Some(sc) = scene {
                let line = if focused { FOCUS_LINE } else { 1 };
                out.push(Shape::rect(
                    x(a),
                    At::px(HEADER - line),
                    px(x(b), -1),
                    At::px(HEADER),
                    colour(index(sc.color, DEFAULT_SCENE_COLOR), 255),
                ));
            }
            // The name, from the section's start or the display's left edge.
            let left = if a < t0 { At::px(0) } else { x(a) };
            let mut name =
                rect(px(left, 7), At::px(0), px(x(b), -2), At::px(HEADER - FOCUS_LINE), if focused { WHITE } else { SECTION_TEXT });
            let text = scene.map_or_else(|| "None".to_string(), |sc| sc.name.clone());
            name.text = Some((text, TIMELINE_FONT.0.into(), 13.0));
            name.clip = true;
            out.push(name);
        }
        y = HEADER;
    }

    // The timeline: bars marked, numbered where there's room.
    let tl = if overview { TIMELINE_OVERVIEW } else { TIMELINE };
    out.push(rect(At::frac(0.0), At::px(y), At::frac(1.0), At::px(y + tl), TIMELINE_BG));
    let px_per_bar = if overview { 480.0 } else { 453.0 } * bar as f32 / (t1 - t0).max(1) as f32;
    let every = [1, 2, 4, 8, 16, 32, 64, 128].into_iter().find(|&n| px_per_bar * n as f32 >= 28.0).unwrap_or(256);
    let mut t = t0.div_euclid(bar) * bar;
    while t < t1 {
        let n = t.div_euclid(bar);
        if t >= t0 && n % every == 0 {
            out.push(rect(x(t), At::px(y + 3), px(x(t), 1), At::px(y + tl), TIMELINE_TEXT));
            let mut label = rect(px(x(t), 3), At::px(y - 1), px(x(t), 40), At::px(y + tl), TIMELINE_TEXT);
            label.text = Some(((n + 1).to_string(), TIMELINE_FONT.0.into(), TIMELINE_FONT.1));
            out.push(label);
        }
        t += bar;
    }
    y += tl;

    // The overview's line of scene colours.
    if overview {
        let (gap, h) = SCENE_LINE;
        for (i, s) in p.sections.iter().enumerate() {
            let Some(sc) = s.scene.and_then(|k| p.scenes.get(k)) else { continue };
            let a = p.section_start(i);
            let alpha = if focus == Some(i) { 255 } else { 160 };
            out.push(Shape::rect(
                x(a),
                At::px(y + gap),
                px(x(a + s.length), -1),
                At::px(y + gap + h),
                colour(index(sc.color, DEFAULT_SCENE_COLOR), alpha),
            ));
        }
        y += gap + h + 1;
    }

    // A lane per group: on the detail the focused bank of eight, on the overview all.
    let (lane, first, count) = if overview {
        (LANE_OVERVIEW, 0, p.groups.len())
    } else {
        let first = st.focus.group / LANES_SHOWN * LANES_SHOWN;
        (LANE, first, p.groups.len().saturating_sub(first).min(LANES_SHOWN))
    };
    let lanes_top = y;
    out.push(rect(At::frac(0.0), At::px(y), At::frac(1.0), At::frac(1.0), LANE_GAP));
    for k in 0..count {
        let g = first + k;
        let (y0, y1) = (y + k as i32 * lane, y + (k as i32 + 1) * lane - if overview { 1 } else { 2 });
        out.push(rect(At::frac(0.0), At::px(y0), At::frac(1.0), At::px(y1), LANE_BG));
        // The gaps between bars, where they're far enough apart to see.
        if px_per_bar >= 6.0 {
            let mut t = t0.div_euclid(bar) * bar;
            while t < t1 {
                if t > t0 {
                    out.push(rect(x(t), At::px(y0), px(x(t), 1), At::px(y1), BAR_GAP));
                }
                t += bar;
            }
        }
        if g == st.focus.group {
            out.push(rect(At::frac(0.0), At::px(y0), At::frac(1.0), At::px(y1), FOCUS_LANE));
        }
        lane_patterns(&mut out, &p, g, (t0, t1), &x, (y0, y1), overview);
    }

    // Past the song's end: dimmed.
    if end < t1 {
        out.push(rect(x(end.max(t0)), At::px(lanes_top), At::frac(1.0), At::frac(1.0), AFTER_END));
    }
    // Where the right display is, on the overview.
    if overview && shown != (0, song) {
        frame(&mut out, x(shown.0), At::px(TIMELINE_OVERVIEW), x(shown.1), At::frac(1.0));
    }
    // The playhead, while the song plays.
    if st.transport.playing && st.transport.what == Some(Playing::Arrangement) {
        let t = st.transport.position;
        if (t0..t1).contains(&t) {
            out.push(rect(x(t), At::px(0), px(x(t), 1), At::frac(1.0), WHITE));
        }
    }
    out
}

/// Group `g`'s patterns, section by section, between `y0` and `y1`.
fn lane_patterns(
    out: &mut Vec<Shape>,
    p: &Project,
    g: usize,
    (t0, t1): (i64, i64),
    x: &dyn Fn(i64) -> At,
    (y0, y1): (i32, i32),
    overview: bool,
) {
    let group = &p.groups[g];
    let alpha = if group.muted { 90 } else { 255 };
    for (i, s) in p.sections.iter().enumerate() {
        let Some(sc) = s.scene.and_then(|k| p.scenes.get(k)) else { continue };
        let Some(&(_, pi)) = sc.patterns.iter().find(|(pg, _)| *pg == g) else { continue };
        let Some(pat) = group.patterns.get(pi) else { continue };
        let (a, b) = (p.section_start(i), p.section_start(i) + s.length);
        if b <= t0 || a >= t1 {
            continue;
        }
        let c = index(pat.color, index(group.color, DEFAULT_GROUP_COLOR));
        let right = At { px: x(b).px - 1, ..x(b) };
        out.push(Shape::rect(x(a), At::px(y0), right, At::px(y1), colour(c, alpha)));
        // Where the pattern starts over within the section.
        let len = if pat.length > 0 { pat.length } else { p.bar_length() };
        let mut t = a + len;
        while t < b && t < t1 && len > 0 {
            if t > t0 {
                out.push(rect(x(t), At::px(y0), At { px: x(t).px + 1, ..x(t) }, At::px(y1), REPEAT_LINE));
            }
            t += len;
        }
        if !overview {
            let left = if a < t0 { At::px(0) } else { x(a) };
            let mut name = rect(At { px: left.px + 4, ..left }, At::px(y0), right, At::px(y1), PATTERN_TEXT);
            name.text = Some((pat.name.clone(), TIMELINE_FONT.0.into(), TIMELINE_FONT.1));
            name.clip = true;
            out.push(name);
        }
    }
}
