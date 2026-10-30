//! Maschine 3's drum synth editors in the strip (`DrumSynthModuleScaffold.qml`): a MAIN
//! tab (`DrumSynthMainTabBase.qml`: Tune with its Note knob, the engine, Velocity and
//! Decay) and a MODULATION tab with the engine's own controls (`synth_editors.rs`). The
//! positions are the QML's, in its 632×216 panel; the drawing is DECKARD's own. The Poly
//! Synth's editor (`PolySynthEditor.qml`) is here too: four tabs, Osc A, Osc B, LFO, Mod.

use super::*;
use crate::drum_ui;

const W: f32 = 632.0;
const H: f32 = 216.0;
const TABS: [&str; 2] = ["MAIN", "MODULATION"];
const POLY_TABS: [&str; 4] = ["OSC A", "OSC B", "LFO", "MOD"];

/// A drum panel's width.
pub fn width() -> f32 {
    W * editor::BODY_H / H + 8.0
}

/// The engine's Modulation tab.
fn modulation(module: &str) -> Option<&'static editor::Editor> {
    super::synth_editors::MODULATION.iter().find(|e| e.module == module)
}

/// The Poly Synth panel's width.
pub fn poly_width() -> f32 {
    863.0 * editor::BODY_H / H + 8.0
}

/// The panel of Poly Synth slot `i` in `body`: its tabs, and the one shown.
#[allow(clippy::too_many_arguments)]
pub fn poly_draw(
    ui: &mut Ui,
    painter: &Painter,
    body: Rect,
    slot: &deckard_model::SlotInfo,
    (g, s, i): (usize, usize, usize),
    color: Color32,
    acts: &mut Vec<Act>,
) {
    let id = ui.id().with(("poly", g, s, i));
    let tab = tabs(ui, painter, body, id.with("tab"), &POLY_TABS);
    if let Some(ed) = super::synth_editors::POLY_SYNTH.get(tab) {
        editor::draw(ui, painter, body, ed, slot, (g, s, i), (0.0, 0.0, 0.0), color, acts);
    }
}

/// The tabs right of the title; returns the one shown (kept per panel).
fn tabs(ui: &mut Ui, painter: &Painter, body: Rect, id: egui::Id, names: &[&str]) -> usize {
    let cur = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0);
    let mut x = body.right() - 12.0;
    let mut rects = vec![Rect::NOTHING; names.len()];
    for (k, name) in names.iter().enumerate().rev() {
        let galley = painter.layout_no_wrap(name.to_string(), font(11.0), white(200));
        let r = Rect::from_min_size(Pos2::new(x - galley.size().x - 8.0, body.top() + 8.0), Vec2::new(galley.size().x + 8.0, 20.0));
        x = r.left() - 6.0;
        let resp = ui.interact(r, id.with(k), Sense::click());
        let ink = if k == cur {
            Color32::WHITE
        } else if resp.hovered() {
            white(200)
        } else {
            white(110)
        };
        painter.text(r.center(), Align2::CENTER_CENTER, *name, font(11.0), ink);
        if k == cur {
            painter.hline(r.x_range().shrink(4.0), r.bottom(), Stroke::new(1.5, white(200)));
        }
        if resp.clicked() {
            ui.data_mut(|d| d.insert_temp(id, k));
        }
        rects[k] = r;
    }
    // Where the tabs are, for tests.
    let key = if names.len() == 4 { "deckard-poly-tabs" } else { "deckard-drum-tabs" };
    ui.data_mut(|d| d.insert_temp(egui::Id::new(key), rects));
    ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0)
}

/// The panel of drum synth slot `i` in `body`.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ui: &mut Ui,
    painter: &Painter,
    body: Rect,
    slot: &deckard_model::SlotInfo,
    (g, s, i): (usize, usize, usize),
    color: Color32,
    acts: &mut Vec<Act>,
) {
    let id = ui.id().with(("drum", g, s, i));
    if tabs(ui, painter, body, id.with("tab"), &TABS) == 1 {
        if let Some(ed) = modulation(&slot.module) {
            editor::draw(ui, painter, body, ed, slot, (g, s, i), (0.0, 0.0, 0.0), color, acts);
        }
        return;
    }
    main_tab(ui, painter, body, slot, (g, s, i), color, id, acts);
}

/// A number dragged up and down (Maschine's NumericArea): `label` over `value` text.
#[allow(clippy::too_many_arguments)]
fn numeric(ui: &mut Ui, painter: &Painter, r: Rect, label: Option<&str>, size: f32, k: &Knob, id: egui::Id, acts: &mut Vec<Act>) {
    let resp = ui.interact(r, id, Sense::click_and_drag()).on_hover_text("Drag (Shift: fine); double-click: default");
    let mut y = r.top();
    if let Some(label) = label {
        painter.text(Pos2::new(r.center().x, y + 7.0), Align2::CENTER_CENTER, label, font(12.0), white(160));
        y += 18.0;
    }
    let ink = if resp.hovered() || resp.dragged() { Color32::WHITE } else { white(230) };
    painter.text(Pos2::new(r.center().x, y + size * 0.6), Align2::CENTER_CENTER, &k.1, font(size), ink);
    if let Some(edit) = k.3 {
        drag(ui, &resp, id.with("drag"), k.2, 300.0, edit, acts);
    }
}

/// Maschine's BipolarSlider: a horizontal bar filled from its centre, `label` below.
#[allow(clippy::too_many_arguments)]
fn bipolar_slider(ui: &mut Ui, painter: &Painter, r: Rect, label: &str, k: &Knob, id: egui::Id, color: Color32, acts: &mut Vec<Act>) {
    let track = Rect::from_center_size(Pos2::new(r.center().x, r.top() + 6.0), Vec2::new(r.width(), 4.0));
    let resp = ui.interact(r, id, Sense::click_and_drag()).on_hover_text("Drag (Shift: fine); double-click: default");
    painter.rect_filled(track, 2.0, Color32::from_rgb(0x3a, 0x3a, 0x3a));
    let a = k.2.clamp(0.0, 1.0);
    let (x0, x1) = (track.center().x, track.left() + track.width() * a);
    painter.rect_filled(Rect::from_x_y_ranges(x0.min(x1)..=x0.max(x1), track.y_range()), 2.0, color);
    painter.vline(track.center().x, track.y_range().expand(3.0), Stroke::new(1.0, white(90)));
    let live = resp.hovered() || resp.dragged();
    let text = if live { k.1.as_str() } else { label };
    painter.text(Pos2::new(r.center().x, r.top() + 24.0), Align2::CENTER_CENTER, text, font(12.0), white(if live { 245 } else { 160 }));
    if let Some(edit) = k.3 {
        // From its own value when a route moves it on screen.
        let a = super::super::mod_routes::ui_edit_from(ui, &edit, a);
        if resp.drag_started() {
            ui.data_mut(|d| d.insert_temp(id.with("from"), (a, 0f32)));
        }
        let fine = ui.input(|i| i.modifiers.shift);
        let dx = resp.drag_delta().x / track.width() / if fine { 5.0 } else { 1.0 };
        if resp.dragged() && dx != 0.0 {
            let (from, moved) = ui.data(|d| d.get_temp::<(f32, f32)>(id.with("from"))).unwrap_or((a, 0.0));
            ui.data_mut(|d| d.insert_temp(id.with("from"), (from, moved + dx)));
            acts.push(edit.to_act((from + moved + dx).clamp(0.0, 1.0)));
        }
        if resp.double_clicked() {
            acts.push(edit.reset());
        }
    }
}

/// The decay's shape (Maschine's DecayVisualizer): a falling curve whose length follows
/// Decay.
fn decay_curve(painter: &Painter, r: Rect, decay: f32, color: Color32) {
    painter.rect_filled(r, 2.0, Color32::from_rgb(0x1a, 0x1a, 0x1a));
    let tau = 0.04 + 0.5 * decay.clamp(0.0, 1.0).powi(2);
    let n = 48;
    let pts: Vec<Pos2> = (0..=n)
        .map(|k| {
            let t = k as f32 / n as f32;
            Pos2::new(r.left() + r.width() * t, r.bottom() - (r.height() - 4.0) * (-t / tau).exp())
        })
        .collect();
    // Under the curve (concave, so in strips).
    for w in pts.windows(2) {
        let strip = Rect::from_min_max(Pos2::new(w[0].x, w[0].y.min(w[1].y)), Pos2::new(w[1].x, r.bottom()));
        painter.rect_filled(strip, 0.0, color.gamma_multiply(0.18));
    }
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, color)));
}

/// A drum, drawn simply where Maschine shows the engine's picture.
fn drum_picture(painter: &Painter, c: Pos2, r: f32, color: Color32) {
    let ink = color.gamma_multiply(0.55);
    painter.circle_stroke(c, r, Stroke::new(2.0, ink));
    painter.circle_stroke(c, r * 0.72, Stroke::new(1.0, ink.gamma_multiply(0.6)));
    painter.circle_filled(c, r * 0.12, ink);
}

#[allow(clippy::too_many_arguments)]
fn main_tab(
    ui: &mut Ui,
    painter: &Painter,
    body: Rect,
    slot: &deckard_model::SlotInfo,
    (g, s, i): (usize, usize, usize),
    color: Color32,
    id: egui::Id,
    acts: &mut Vec<Act>,
) {
    let module = slot.module.as_str();
    let k = body.height() / H;
    let at = |x: f32, y: f32| body.min + Vec2::new(x, y) * k;
    let rect = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(at(x, y), Vec2::new(w, h) * k);
    let ctl = |field: &str| drum_ui::control(slot, g, s, i, field);
    let engine = slot.param(drum_ui::engine_field(module)).unwrap_or(0.0).round() as i32;
    let line = Color32::from_rgb(0x32, 0x32, 0x32);
    for r in [rect(174.0, 43.0, 1.0, 157.0), rect(459.0, 43.0, 1.0, 157.0), rect(21.0, 158.0, 145.0, 1.0), rect(468.0, 158.0, 145.0, 1.0)] {
        painter.rect_filled(r, 0.0, line);
    }
    // Tune, and under it its small knob (NOTE when it shows a note).
    if let Some(tune) = ctl("tune") {
        numeric(ui, painter, rect(31.0, 43.0, 125.0, 80.0), Some("TUNE"), 26.0 * k.max(0.8), &tune, id.with("tune"), acts);
        let hold = module == "Perc" && engine == 0;
        if hold && let Some(h) = ctl("fractal_tune_hold") {
            let mut h = h;
            h.0 = Cow::Borrowed("HOLD");
            editor::power_at(ui, painter, at(51.0, 190.0), at(51.0, 172.0), &h, id.with("hold"), color, acts);
        }
        let mut note = tune;
        note.0 = Cow::Borrowed(if drum_ui::tune_is_note(module, engine) { "NOTE" } else { "" });
        let x = if hold { 24.0 + 55.0 } else { 24.0 + 32.0 } + 13.0;
        knob_with(ui, painter, at(x + 30.0, 176.0 + 13.0), 13.0 * k - 1.5, &note, false, id.with("note"), color, acts);
    }
    // The engine, and its picture.
    if let Some(e) = ctl(drum_ui::engine_field(module)) {
        choice_sized(ui, painter, at(233.0 + 82.0, 47.0), 147.0 * k, &e, id.with("engine"), acts);
    }
    drum_picture(painter, at(317.0, 112.0), 40.0 * k, color);
    // Velocity.
    let vel = if module == "Hihat" { "velocity_scaling" } else { "velocity" };
    if let Some(v) = ctl(vel) {
        bipolar_slider(ui, painter, rect(253.0, 174.0, 128.0, 30.0), "Velocity", &v, id.with("velocity"), color, acts);
    }
    // Decay: its curve, its percentage, and Gate where the engine has one.
    painter.text(at(482.0 + 58.0, 47.0), Align2::CENTER_CENTER, "DECAY", font(12.0), white(160));
    if let Some(d) = ctl("decay") {
        decay_curve(painter, rect(482.0, 62.0, 117.0, 60.0), d.2, color);
        let gate = match (module, engine) {
            ("Snare", 0 | 1) | ("Hihat", 0..=2) => ctl("gate"),
            _ => None,
        };
        let x = if gate.is_some() { 482.0 + 17.0 } else { 482.0 + 36.0 };
        numeric(ui, painter, rect(x, 132.0, 50.0, 48.0), None, 14.0, &d, id.with("decay"), acts);
        painter.text(at(x + 25.0, 182.0), Align2::CENTER_CENTER, "Percent", font(11.0), white(140));
        if let Some(mut gk) = gate {
            gk.0 = Cow::Borrowed("GATE");
            editor::power_at(
                ui,
                painter,
                at(x + 50.0 + 8.0 + 14.0, 160.0),
                at(x + 50.0 + 8.0 + 14.0, 140.0),
                &gk,
                id.with("gate"),
                color,
                acts,
            );
        }
    }
}
