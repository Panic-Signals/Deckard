//! Maschine's own effects in the strip, laid out as Maschine 3's editors place them
//! (`editors.rs`, read from its QML): knobs, selectors, switches, meters with faders. The
//! values, ranges and edits come from the module's parameter pages (`module_ui`), found by
//! field.

use super::*;

/// One control of an editor: its top-left in QML pixels, its label, the model property it
/// shows, and when it is shown (all of: the field's value is (`true`) / isn't one of these).
pub struct W {
    pub x: f32,
    pub y: f32,
    pub label: &'static str,
    pub prop: &'static str,
    pub kind: Kind,
    pub when: &'static [(&'static str, bool, &'static [i32])],
}

pub enum Kind {
    /// A knob `size` across.
    Knob { size: f32, bipolar: bool },
    /// "◂ value ▸" with its label (if any) below.
    Cycler { label_w: f32 },
    /// A power switch over its label.
    Toggle { label_on_top: bool },
    /// A level meter (`meter`: "inputMeter", "outputMeter") with the parameter's handle on
    /// it and a dB scale.
    Fader { min_db: f32, ticks: &'static [f32], invert: bool, meter: &'static str },
    /// A meter alone (gain reduction).
    Bar { w: f32, h: f32, from_top: bool },
    /// A heading.
    Label,
    /// A connecting line.
    Line { w: f32, h: f32 },
}

pub struct Editor {
    pub module: &'static str,
    /// As Maschine titles it ("LEGACY REVERB").
    pub name: &'static str,
    pub width: f32,
    pub height: f32,
    pub widgets: &'static [W],
}

/// Maschine's editor of a module class, if it has one DECKARD lays out this way.
pub fn editor(module: &str) -> Option<&'static Editor> {
    super::editors::EDITORS.iter().chain(super::synth_editors::INSTRUMENTS).find(|e| e.module == module)
}

/// The body's height in a strip (QML editors are 216 high; they are scaled to it).
pub const BODY_H: f32 = STRIP_H - PANEL_TOP - 10.0 - BAR_H - 4.0;

/// A panel's width for `ed`.
pub fn width(ed: &Editor) -> f32 {
    ed.width * BODY_H / ed.height + 8.0
}

/// The stored field a QML model property shows, where the names differ.
fn field_of(module: &str, prop: &'static str) -> Cow<'static, str> {
    if let Some(f) = crate::drum_ui::qml_field(module, prop) {
        return Cow::Borrowed(f);
    }
    let f = match (module, prop) {
        ("BeatDelay", "amount") => "feedback",
        ("BassSynth", "shape2") => "shape_2",
        ("BassSynth", "octave2") => "octave_2",
        ("BassSynth", "modAmt") => "mod_amount",
        ("Filter", "shapeLfo") => "lfo_shape",
        ("Filter", "shapeEnv") => "env_shape",
        ("Flanger", "modSource") => "source",
        ("Flanger", "freq") => "frequency",
        ("Flanger", "modulation") => "amount",
        ("Flanger", "fbAmount") => "feedback",
        ("Flanger", "fbInvert") => "invert",
        ("Flanger", "lfoSpeed") => "speed",
        ("Flanger", "envShape") => "shape",
        ("TransientDesigner", "gain") => "input_gain",
        ("FreqShifter", "amount") => "feedback",
        ("Phaser", "modSource") => "source",
        ("Phaser", "freqCenter") => "frequency",
        ("Phaser", "modAmount") => "amount",
        ("Phaser", "feedbackAmount") => "feedback",
        ("Phaser", "poleEnabled8") => "eight_pole",
        ("Phaser", "modSpeedLfo") => "speed",
        ("Phaser", "modEnvShapeAmount") => "shape",
        ("MullhollandDrive", "toneMod") => "tone_mod",
        ("MullhollandDrive", "saturation") => "analog_saturation",
        ("MullhollandDrive", "bass") => "analog_bass",
        ("MullhollandDrive", "analogTone") => "analog_tone",
        ("MullhollandDrive", "analogMix") => "analog_mix",
        ("MullhollandDrive", "focus") => "analog_focus",
        ("MullhollandDrive", "definition") => "analog_definition",
        ("Resochord", "transpose") => "tune",
        ("GrainStretch", "enabled") => "on",
        ("GrainStretch", "loop") => "loop_length",
        ("Saturator", "compress") => "classic_input",
        ("Saturator", "contour") => "classic_contour",
        ("Saturator", "drive") => "classic_drive",
        ("Saturator", "tapeGain") => "tape_input",
        ("Saturator", "tapeTone") => "tape_contour",
        ("Saturator", "tapeWarmth") => "tape_drive",
        ("Saturator", "tubeBassOverload") => "tube_overload",
        ("Saturator", "tubeEQLow") => "tube_bass",
        ("Saturator", "tubeEQBypass") => "tube_eq_bypass",
        ("Saturator", "tubeEQHigh") => "tube_treble",
        _ => return Cow::Owned(snake(prop)),
    };
    Cow::Borrowed(f)
}

/// "tubeDrive" → "tube_drive", "poleEnabled8" → "pole_enabled8".
fn snake(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The field an edit changes.
fn edit_field(e: &Edit) -> Option<&'static str> {
    match e {
        Edit::Param(_, f, ..) | Edit::Log(_, f, ..) | Edit::Int(_, f, ..) | Edit::Toggle(_, f, _) | Edit::Choice(_, f, ..) => Some(f),
        _ => None,
    }
}

/// The control for `field` on any of the module's pages.
fn control(slot: &deckard_model::SlotInfo, g: usize, s: usize, i: usize, field: &str) -> Option<Knob> {
    let n = crate::module_ui::pages(&slot.module)?.len();
    (0..n).find_map(|p| {
        crate::module_ui::page(slot, g, s, i, p).slots.into_iter().flatten().find(|k| k.3.as_ref().and_then(edit_field) == Some(field))
    })
}

/// Whether `w` shows with the slot's settings.
fn shown(slot: &deckard_model::SlotInfo, module: &str, w: &W) -> bool {
    w.when.iter().all(|(prop, is, vals)| {
        let f = field_of(module, prop);
        // A Plate Reverb from before its Mode (class version 0) is a Plate.
        let unset = match (module, *prop) {
            ("PlateReverb", _) if slot.param("wet_gain").is_none() => 2.0,
            ("Resochord", "mode") => 1.0,
            _ => 0.0,
        };
        let v = slot.param(&f).unwrap_or(unset).round() as i32;
        vals.contains(&v) == *is
    })
}

/// Draws `ed` for slot `i` in `body` (the panel under its header bar). `meters`: input
/// level and output level (linear) and gain reduction (dB), for its meters.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ui: &mut Ui,
    painter: &Painter,
    body: Rect,
    ed: &Editor,
    slot: &deckard_model::SlotInfo,
    (g, s, i): (usize, usize, usize),
    meters: (f32, f32, f32),
    color: Color32,
    acts: &mut Vec<Act>,
) {
    let k = body.height() / ed.height;
    let at = |x: f32, y: f32| body.min + Vec2::new(x, y) * k;
    let id = ui.id().with(("strip-editor", g, s, i));
    let span = ed.widgets.iter().find_map(|w| if let Kind::Fader { min_db, .. } = w.kind { Some(-min_db) } else { None }).unwrap_or(40.0);
    for (n, w) in ed.widgets.iter().enumerate() {
        if !shown(slot, ed.module, w) {
            continue;
        }
        let wid = id.with(n);
        let ctl = if w.prop.is_empty() { None } else { control(slot, g, s, i, &field_of(ed.module, w.prop)) };
        // A control with no page yet shows, inert, as Maschine places it.
        let fallback = || -> Knob { (Cow::Borrowed(w.label), String::new(), 0.0, None) };
        match &w.kind {
            Kind::Knob { size, bipolar } => {
                let r = (size / 2.0 - if *size > 40.0 { 2.5 } else { 1.5 }) * k;
                let mut knob = ctl.unwrap_or_else(fallback);
                knob.0 = Cow::Borrowed(w.label);
                knob_with(ui, painter, at(w.x + size / 2.0, w.y + size / 2.0), r, &knob, *bipolar, wid, color, acts);
            }
            Kind::Cycler { label_w } => {
                let wd = 28.0 + label_w;
                let c = at(w.x + wd / 2.0, w.y + 8.0);
                match &ctl {
                    Some(knob) => choice_sized(ui, painter, c, (label_w + 10.0) * k, knob, wid, acts),
                    None => {
                        painter.text(c, Align2::CENTER_CENTER, "—", font(13.0), white(90));
                    }
                }
                if !w.label.is_empty() {
                    painter.text(at(w.x + wd / 2.0, w.y + 28.0), Align2::CENTER_CENTER, w.label, font(12.0), white(160));
                }
            }
            Kind::Toggle { label_on_top } => {
                let tw = (w.label.len() as f32 * 7.0).max(18.0);
                let (icon_y, label_y) = if *label_on_top { (w.y + 30.0, w.y + 10.0) } else { (w.y + 19.0, w.y + 38.0) };
                let mut knob = ctl.unwrap_or_else(fallback);
                knob.0 = Cow::Borrowed(w.label);
                power_at(ui, painter, at(w.x + tw / 2.0, icon_y), at(w.x + tw / 2.0, label_y), &knob, wid, color, acts);
            }
            Kind::Fader { min_db, ticks, invert, meter } => {
                let col = Rect::from_min_size(at(w.x - 6.0, w.y), Vec2::new(60.0 * k, 115.0 * k));
                let level = if *meter == "outputMeter" { meters.2 } else { meters.0 };
                let knob = ctl.as_ref().map(|c| (c, *invert));
                meter_fader(ui, painter, col, -min_db, level, knob, ticks, wid, color, acts);
                painter.text(at(w.x + 28.0, w.y + 136.0), Align2::CENTER_CENTER, w.label, font(12.0), white(160));
            }
            Kind::Bar { w: bw, h, from_top } => {
                let bar = Rect::from_min_size(at(w.x, w.y), Vec2::new(bw * k, h * k));
                painter.rect_filled(bar, 0.0, Color32::from_rgb(0x2c, 0x2c, 0x2c));
                let red = (meters.1 / span).clamp(0.0, 1.0);
                if red > 0.002 {
                    let fill = if *from_top {
                        Rect::from_min_size(bar.min, Vec2::new(bar.width(), bar.height() * red))
                    } else {
                        Rect::from_min_max(Pos2::new(bar.left(), bar.bottom() - bar.height() * red), bar.max)
                    };
                    painter.rect_filled(fill, 0.0, GR_GREEN);
                }
                ui.interact(bar, wid, Sense::hover()).on_hover_text(format!("Gain reduction {:.1} dB", meters.1));
                painter.text(at(w.x + bw / 2.0, w.y + h + 22.0), Align2::CENTER_CENTER, "GR", font(12.0), white(160));
            }
            Kind::Label => {
                painter.text(at(w.x, w.y + 8.0), Align2::LEFT_CENTER, w.label, font(12.0), white(110));
            }
            Kind::Line { w: lw, h } => {
                painter.rect_filled(
                    Rect::from_min_size(at(w.x, w.y), Vec2::new(lw * k, (h * k).max(1.0))),
                    0.0,
                    Color32::from_rgb(0x32, 0x32, 0x32),
                );
            }
        }
    }
}

/// Maschine's on/off switch: a power icon (lit in the group's colour when on), its label
/// at `label`.
#[allow(clippy::too_many_arguments)]
pub(super) fn power_at(
    ui: &mut Ui,
    painter: &Painter,
    icon: Pos2,
    label: Pos2,
    k: &Knob,
    id: egui::Id,
    color: Color32,
    acts: &mut Vec<Act>,
) {
    let on = matches!(k.3, Some(Edit::Toggle(_, _, true)));
    let area = Rect::from_two_pos(icon - Vec2::new(18.0, 11.0), label + Vec2::new(18.0, 8.0));
    let resp = ui.interact(area, id, Sense::click()).on_hover_text(if on { "On (click: off)" } else { "Off (click: on)" });
    let ink = if on {
        color
    } else if resp.hovered() {
        white(150)
    } else {
        Color32::from_rgb(0x4f, 0x4f, 0x4f)
    };
    painter.circle_stroke(icon + Vec2::new(0.0, 1.0), 5.0, Stroke::new(1.6, ink));
    painter.line_segment([icon + Vec2::new(0.0, -6.0), icon + Vec2::new(0.0, 0.0)], Stroke::new(1.6, ink));
    painter.text(label, Align2::CENTER_CENTER, title_case(&k.0), font(12.0), white(160));
    if let Some(Edit::Toggle(t, f, on)) = k.3
        && resp.clicked()
    {
        acts.push(Act::SetParam(t, f, if on { 0.0 } else { 1.0 }));
    }
}

#[cfg(test)]
/// Where the shown knobs of slot `i`'s editor sit in its panel (offsets from the panel's
/// top-left), with the fields they turn: for tests.
pub(crate) fn knob_spots(slot: &deckard_model::SlotInfo, (g, s, i): (usize, usize, usize)) -> Vec<(Vec2, String)> {
    // A drum synth's: its Modulation tab.
    let drum = || super::synth_editors::MODULATION.iter().find(|e| e.module == slot.module);
    let Some(ed) = editor(&slot.module).or_else(drum) else { return Vec::new() };
    let k = BODY_H / ed.height;
    let top = Vec2::new(4.0, BAR_H + 4.0);
    ed.widgets
        .iter()
        .filter(|w| shown(slot, ed.module, w))
        .filter_map(|w| {
            let Kind::Knob { size, .. } = w.kind else { return None };
            let f = field_of(ed.module, w.prop);
            control(slot, g, s, i, &f)?;
            Some((top + Vec2::new(w.x + size / 2.0, w.y + size / 2.0) * k, f.into_owned()))
        })
        .collect()
}
