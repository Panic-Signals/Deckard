//! The Mixer page's strips (`MixerBar`, one per group or sound), drawn by the host from
//! what NI's script sets on them (`MixerVector.loadItem`, `updateLevels`), laid out as
//! Maschine's are (the manual's Mix mode pictures; sizes, fonts and greys from the style
//! sheet's `MixerPage.txt`):
//! - the header: the short name in the channel's colour and the long name over two
//!   lines, the whole header in the colour when selected (a selected strip's body is
//!   lit grey);
//! - the pan bar (the channel's colour from the middle to its side) over a mark at the
//!   middle, and the headroom (peak) under it;
//! - the scale (10, 0, 10, 20, 30, ∞), the two meters in the colour with their peak,
//!   the 0 dB line, and the fader: a white line up to the level, a small arrow at its
//!   top (smaller than Maschine's, so the meters are wider);
//! - the plug-in icons (FX, or an instrument's bars) at the right, from the bottom up;
//! - the value (level or pan, whichever the knobs turn) under it all.
//!
//! Muted strips are grey.

use crate::state::{Prop, Widgets};
use deckard_nwl::draw::{At, Fill, Shape};

/// The style sheet's palettes: the channel colours as backgrounds, and as text.
const BG_PALETTE: u32 = 2;
const FG_PALETTE: u32 = 1;
const FONT: &str = "Shared/prg65_j.ttf";
/// The meters' foot, up from the strip's bottom (the value is under it).
const FOOT: i32 = 37;
/// The header's foot, up from the strip's bottom (a sound's is 40 high, a group's 72).
const HEADER_FOOT: i32 = 198;
/// The meters' scale: dB, and how far above the foot it is drawn (Maschine's scale
/// isn't linear: the top 10 dB are spread wide).
const SCALE: [(f64, f64); 6] = [(-60.0, 0.0), (-30.0, 22.0), (-20.0, 39.0), (-10.0, 64.0), (0.0, 91.0), (10.0, 119.0)];
/// What the meters' values (0..1000) span, in dB (`NI.UTILS.LevelScale`).
const LOW_DB: f64 = -60.0;
const HIGH_DB: f64 = 10.0;
/// The meters, the fader and its arrow (rows), across the strip.
const METERS: [(i32, i32); 2] = [(28, 54), (57, 83)];
const FADER_X: i32 = 88;
const ARROW: i32 = 5;

const WHITE: [u8; 4] = [255, 255, 255, 255];
const NAME: [u8; 4] = [220, 220, 220, 255];
/// The headroom, the scale and the pan's mark.
const HEADROOM: [u8; 4] = [80, 80, 80, 255];
const SCALE_TEXT: [u8; 4] = [95, 95, 95, 255];
const TRACK: [u8; 4] = [48, 48, 48, 255];
const ZERO_LINE: [u8; 4] = [85, 85, 85, 255];
/// A selected strip's body, and a muted strip's colour (the sheet's inactive grey).
const SELECTED_BODY: [u8; 4] = [13, 13, 13, 255];
const MUTED: [u8; 4] = [71, 71, 71, 255];

/// The height a strip takes: a sound's (the style sheet's 238); a group's the whole
/// display, or its header alone (the room the sounds leave) when they're shown under it.
pub fn height(w: &Widgets, id: usize) -> i32 {
    let bar = w.get(id).and_then(|x| x.parent);
    let vector = bar.and_then(|b| w.get(b)?.parent).and_then(|v| w.get(v));
    if !vector.is_some_and(|v| v.name == "GroupVector") {
        return 238;
    }
    // The sounds shown under the groups (any of the row says so; empty slots and the
    // + don't): the groups are a row of headers.
    let row = bar.and_then(|b| w.get(b)).map_or(&[][..], |b| b.children.as_slice());
    if row.iter().any(|&c| attr(w, c, "SoundsVisible") == Some("true")) { 28 } else { 270 }
}

fn attr<'a>(w: &'a Widgets, id: usize, k: &str) -> Option<&'a str> {
    match w.get(id)?.props.get(&format!("attr:{k}")) {
        Some(Prop::Str(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// A part the script reached through `getX()` (a child widget), by name.
fn part(w: &Widgets, id: usize, name: &str) -> Option<usize> {
    match w.get(id)?.props.get(name) {
        Some(Prop::Widget(c)) => Some(*c),
        _ => None,
    }
}

fn prop<'a>(w: &'a Widgets, id: usize, part_name: &str, key: &str) -> Option<&'a Prop> {
    w.get(part(w, id, part_name)?)?.props.get(key)
}

fn text_of(w: &Widgets, id: usize, part_name: &str) -> String {
    match prop(w, id, part_name, "Text") {
        Some(Prop::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn num(p: Option<&Prop>) -> f64 {
    match p {
        Some(Prop::Num(n)) => *n,
        Some(Prop::Int(i)) => *i as f64,
        _ => 0.0,
    }
}

/// `px` pixels down from the top.
fn y(px: i32) -> At {
    At::px(px)
}

/// `px` pixels up from the bottom.
fn up(px: i32) -> At {
    At { frac: 1.0, px: -px }
}

fn rect(x0: i32, y0: At, x1: i32, y1: At, fill: Fill) -> Shape {
    Shape::rect(At::px(x0), y0, At::px(x1), y1, fill)
}

/// Text from `x` (or centred between `x` and `x1`), its box from `top`.
fn text(s: &str, x: i32, x1: i32, top: At, size: f32, fill: Fill, centred: bool) -> Shape {
    let bottom = At { px: top.px + size as i32 + 4, ..top };
    Shape { text: Some((s.to_string(), FONT.to_string(), size)), centred, ..Shape::rect(At::px(x), top, At::px(x1), bottom, fill) }
}

/// How far above the meters' foot `db` is drawn.
fn scale_height(db: f64) -> f64 {
    if !db.is_finite() || db <= SCALE[0].0 {
        return 0.0;
    }
    for pair in SCALE.windows(2) {
        let ((d0, h0), (d1, h1)) = (pair[0], pair[1]);
        if db <= d1 {
            return h0 + (db - d0) / (d1 - d0) * (h1 - h0);
        }
    }
    SCALE[SCALE.len() - 1].1
}

/// Where `db` is drawn: up from the bottom.
fn level_y(db: f64) -> i32 {
    FOOT + scale_height(db).round() as i32
}

/// A triangle pointing left with its tip at (`x`, `cy`), `rows` high (odd), as rows.
fn arrow_left(x: i32, cy: At, rows: i32, fill: Fill, out: &mut Vec<Shape>) {
    let half = rows / 2;
    for i in -half..=half {
        let width = half - i.abs() + 1;
        out.push(rect(x + i.abs(), At { px: cy.px + i, ..cy }, x + i.abs() + width, At { px: cy.px + i + 1, ..cy }, fill));
    }
}

/// A triangle pointing up with its tip at (`cx`, `top`), `rows` high.
fn arrow_up(cx: i32, top: At, rows: i32, fill: Fill, out: &mut Vec<Shape>) {
    for i in 0..rows {
        out.push(rect(cx - i, At { px: top.px + i, ..top }, cx + i + 1, At { px: top.px + i + 1, ..top }, fill));
    }
}

/// The long name in up to two lines of about `width` characters (broken at a space or
/// after a hyphen where it can).
fn two_lines(name: &str, width: usize) -> (String, String) {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= width {
        return (name.to_string(), String::new());
    }
    let cut = (1..=width).rev().find(|&i| chars[i - 1] == '-' || chars.get(i) == Some(&' ')).unwrap_or(width);
    let first: String = chars[..cut].iter().collect();
    let rest: String = chars[cut..].iter().collect::<String>().trim_start().to_string();
    let second: String = rest.chars().take(width).collect();
    (first.trim_end().to_string(), second)
}

/// The strip `id` as shapes in its box.
pub fn bar(w: &Widgets, id: usize) -> Vec<Shape> {
    let mut out = Vec::new();
    let is = |k: &str| attr(w, id, k) == Some("true");
    if is("IsEmpty") {
        return out;
    }
    if is("IsAdd") {
        let fill = Fill::Rgba(if is("IsSelected") { WHITE } else { [110, 110, 110, 255] });
        out.push(text("+", 0, 120, y(4), 18.0, fill, true));
        return out;
    }
    let colour = match prop(w, id, "Header", "PaletteColorIndex") {
        Some(Prop::Int(i)) if *i >= 0 => Some(*i as u32),
        _ => None,
    };
    let muted = is("IsMuted");
    let selected = is("IsSelected");
    let tint = |alpha: u8| match colour {
        Some(index) if !muted => Fill::Palette { palette: BG_PALETTE, index, alpha },
        _ => Fill::Rgba([MUTED[0], MUTED[1], MUTED[2], alpha]),
    };
    let tint_text = match colour {
        Some(index) if !muted => Fill::Palette { palette: FG_PALETTE, index, alpha: 255 },
        _ => Fill::Rgba(MUTED),
    };
    let header_row = height(w, id) <= 30;
    // The header: on the colour when selected; a selected strip's body lit.
    let header_bottom = if header_row { up(0) } else { up(HEADER_FOOT) };
    if selected && !header_row {
        out.push(rect(0, header_bottom, 120, up(0), Fill::Rgba(SELECTED_BODY)));
    }
    if selected {
        out.push(rect(0, y(0), 120, header_bottom, tint(255)));
    }
    let (short_fill, name_fill) = match (selected, muted) {
        (true, _) => (Fill::Rgba(WHITE), Fill::Rgba(WHITE)),
        (false, true) => (Fill::Rgba(MUTED), Fill::Rgba(MUTED)),
        (false, false) => (tint_text, Fill::Rgba(NAME)),
    };
    out.push(text(&text_of(w, id, "NameShort"), 6, 30, y(7), 13.0, short_fill, false));
    let long = text_of(w, id, "NameLong");
    // (One line in a row of headers, cut to fit.)
    let (first, second) = if header_row { (long.chars().take(10).collect(), String::new()) } else { two_lines(&long, 10) };
    out.push(text(&first, 28, 119, y(7), 13.0, name_fill, false));
    if !second.is_empty() {
        out.push(text(&second, 28, 119, y(22), 13.0, name_fill, false));
    }
    // A group row over the sounds: the focused group's colour along its foot.
    if header_row {
        // (The focused group's colour, which the script gives the separator.)
        if is("IsSeparatorVisible")
            && let Some(Prop::Int(index)) = prop(w, id, "Separator", "PaletteColorIndex")
        {
            out.push(rect(0, up(2), 120, up(0), Fill::Palette { palette: BG_PALETTE, index: *index as u32, alpha: 255 }));
        }
        return out;
    }
    if attr(w, id, "ControlsVisible") == Some("false") {
        return out;
    }
    // Pan: the track, the channel's colour from the middle to its side, the middle's
    // mark, and the headroom under it.
    let pan = num(w.get(id).and_then(|x| x.props.get("PanControlToValue"))).clamp(-1.0, 1.0);
    let pan_top = 189;
    out.push(rect(15, up(pan_top), 105, up(pan_top - 4), Fill::Rgba(TRACK)));
    let px = 60 + (pan * 45.0).round() as i32;
    if px != 60 {
        out.push(rect(px.min(60), up(pan_top), px.max(60), up(pan_top - 4), tint(if is("IsPanControlEnabled") { 255 } else { 190 })));
    }
    arrow_up(60, up(pan_top - 8), 4, Fill::Rgba(HEADROOM), &mut out);
    let clipping = is("IsClipping");
    let headroom = text_of(w, id, "HeadRoom");
    out.push(text(&headroom, 0, 120, up(pan_top - 13), 11.0, Fill::Rgba(if clipping && !muted { WHITE } else { HEADROOM }), true));
    // The scale.
    for (db, label) in [(10.0, "10 -"), (0.0, "0 -"), (-10.0, "10 -"), (-20.0, "20 -"), (-30.0, "30 -"), (f64::NEG_INFINITY, "∞ -")] {
        let at = level_y(db);
        out.push(text(label, 3, 26, up(at + 6), 9.0, Fill::Rgba(SCALE_TEXT), false));
    }
    // The 0 dB line, then the meters with their peak.
    let zero = level_y(0.0);
    out.push(rect(METERS[0].0 + 4, up(zero), FADER_X, up(zero - 1), Fill::Rgba(ZERO_LINE)));
    // (The peak each meter holds a moment, then lets fall: `gui::hold_peak`.)
    let db_of = |v: f64| LOW_DB + v.min(1.0) * (HIGH_DB - LOW_DB);
    for (k, name) in ["LeftMeter", "RightMeter"].into_iter().enumerate() {
        let (x0, x1) = METERS[k];
        let v = num(prop(w, id, name, "Value")) / 1000.0;
        if v > 0.0 {
            out.push(rect(x0, up(level_y(db_of(v))), x1, up(FOOT), tint(255)));
        }
        let peak = num(prop(w, id, name, "Peak")) / 1000.0;
        if peak > 0.0 {
            let at = level_y(db_of(peak));
            out.push(rect(x0, up(at + 1), x1, up(at), tint(255)));
        }
    }
    // The fader: up to the level, with its arrow.
    let level = num(w.get(id).and_then(|x| x.props.get("VolumeControlToValue")));
    let at = level_y(deckard_model::level_db(level));
    let fader = Fill::Rgba(if muted { MUTED } else { WHITE });
    out.push(rect(FADER_X, up(at), FADER_X + 1, up(FOOT), fader));
    arrow_left(FADER_X - ARROW / 2 - 1, up(at), ARROW, fader, &mut out);
    // The plug-ins, from the bottom up: an instrument's bars, effects' FX.
    let mut slot_at = FOOT;
    for i in 0..7 {
        let name = format!("SlotIcon{i}");
        if !matches!(prop(w, id, &name, "Visible"), Some(Prop::Bool(true))) {
            continue;
        }
        let icon = part(w, id, &name).unwrap_or(id);
        let instrument = attr(w, icon, "IsInstrument") == Some("true");
        let bypassed = attr(w, icon, "IsBypassed") == Some("true");
        let fill = if bypassed { Fill::Rgba(MUTED) } else { tint(255) };
        let (top, bottom) = (up(slot_at + 9), up(slot_at));
        if instrument {
            for b in 0..3 {
                out.push(rect(103 + b * 3, top, 104 + b * 3, bottom, fill));
            }
        } else {
            // A framed "FX".
            out.push(rect(102, top, 113, At { px: top.px + 1, ..top }, fill));
            out.push(rect(102, At { px: bottom.px - 1, ..bottom }, 113, bottom, fill));
            out.push(rect(102, top, 103, bottom, fill));
            out.push(rect(112, top, 113, bottom, fill));
            out.push(text("FX", 102, 113, At { px: top.px + 1, ..top }, 7.0, fill, true));
        }
        slot_at += 11;
    }
    // The value.
    let value_fill = Fill::Rgba(match (muted, selected) {
        (false, _) => WHITE,
        (true, true) => MUTED,
        (true, false) => [90, 90, 90, 255],
    });
    out.push(text(&text_of(w, id, "ValueLabel"), 0, 120, up(31), 20.0, value_fill, true));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Long names go over two lines, broken where they can be.
    #[test]
    fn long_names_wrap_onto_two_lines() {
        assert_eq!(two_lines("Beat", 12), ("Beat".into(), String::new()));
        assert_eq!(two_lines("String-Stab", 7), ("String-".into(), "Stab".into()));
        assert_eq!(two_lines("Mono Chord 2", 8), ("Mono".into(), "Chord 2".into()));
    }

    /// The scale: 0 dB well up the meter, −∞ at its foot, louder higher.
    #[test]
    fn the_scale_rises_with_the_level() {
        assert_eq!(level_y(f64::NEG_INFINITY), FOOT);
        assert!(level_y(-30.0) < level_y(-10.0) && level_y(-10.0) < level_y(0.0) && level_y(0.0) < level_y(10.0));
        assert_eq!(level_y(0.0), FOOT + 91);
    }
}
