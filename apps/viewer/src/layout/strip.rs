//! The plug-in strip of the Mix view: under the faders, a panel for each plug-in of the
//! focused sound, side by side, as Maschine 3 shows them. Maschine's own effects have
//! their own layouts (the Compressor with its input, gain-reduction and output meters, the
//! EQ by band); other effects, the Sampler and hosted plug-ins show their parameter pages.

use super::*;

mod drum;
pub(crate) mod editor;
mod editors;
mod sampler;
mod synth_editors;

/// The strip's height, under the Mixer.
pub const STRIP_H: f32 = 250.0;
/// A panel's header bar (Maschine's search field), and the body below it.
const BAR_H: f32 = 22.0;
const PANEL_TOP: f32 = 8.0;
/// Space between panels, and the width of the "+" at the end.
const PANEL_GAP: f32 = 12.0;
const ADD_W: f32 = 26.0;
/// The GR meter's green.
const GR_GREEN: Color32 = Color32::from_rgb(0x0f, 0xd8, 0x8a);

/// A title as Maschine draws its effects' names: in the logo font when there is one
/// (lower case: its capitals are logos), else wide-set capitals.
fn title(painter: &Painter, at: Pos2, text: &str) {
    if painter.ctx().data(|d| d.get_temp::<bool>(egui::Id::new(LOGO_FONT))).unwrap_or(false) {
        let font = FontId::new(18.0, egui::FontFamily::Name(LOGO_FONT.into()));
        painter.text(at, Align2::LEFT_CENTER, text.to_lowercase(), font, white(200));
        return;
    }
    let mut x = at.x;
    for ch in text.to_uppercase().chars() {
        let r = painter.text(Pos2::new(x, at.y), Align2::LEFT_CENTER, ch, font(15.0), white(200));
        x = r.right() + 2.5;
    }
}

/// A knob label in Maschine's strip style ("FREQ" → "Freq"; abbreviations such as "SR"
/// stay as they are).
fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            if w.len() <= 2 {
                return w.to_string();
            }
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| f.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a knob's range is centred on zero (its arc fills from the top).
fn bipolar(edit: &Option<Edit>) -> bool {
    matches!(edit, Some(Edit::Param(_, _, lo, hi, _) | Edit::Int(_, _, lo, hi, _)) if *lo < 0.0 && *hi > 0.0)
}

/// Drag `edit` from `amount` with `resp` (vertical, `px` for the whole range; Shift:
/// fine), double-click: its default.
fn drag(ui: &Ui, resp: &Response, id: egui::Id, amount: f32, px: f32, edit: Edit, acts: &mut Vec<Act>) {
    // From its own value when a route moves it on screen.
    let amount = super::mod_routes::ui_edit_from(ui, &edit, amount);
    if let Some(v) = turned(ui, resp, id, amount, px) {
        acts.push(edit.to_act(v));
    }
    if resp.double_clicked() {
        acts.push(edit.reset());
    }
}

/// The drag half of [`drag`]: the amount (0..1) `resp`'s drag has turned `amount` to.
pub(super) fn turned(ui: &Ui, resp: &Response, id: egui::Id, amount: f32, px: f32) -> Option<f32> {
    if resp.drag_started() {
        ui.data_mut(|d| d.insert_temp(id, (amount, 0f32)));
    }
    let fine = ui.input(|i| i.modifiers.shift);
    let dy = -resp.drag_delta().y / if fine { px * 5.0 } else { px };
    if resp.dragged() && dy != 0.0 {
        let (from, moved) = ui.data(|d| d.get_temp::<(f32, f32)>(id)).unwrap_or((amount, 0.0));
        let moved = moved + dy;
        ui.data_mut(|d| d.insert_temp(id, (from, moved)));
        return Some((from + moved).clamp(0.0, 1.0));
    }
    None
}

/// A knob at `c` (radius `r`) with its label below; hovering or dragging shows its
/// value instead.
#[allow(clippy::too_many_arguments)]
fn knob_at(ui: &mut Ui, painter: &Painter, c: Pos2, r: f32, k: &Knob, id: egui::Id, color: Color32, acts: &mut Vec<Act>) {
    knob_with(ui, painter, c, r, k, bipolar(&k.3), id, color, acts)
}

/// [`knob_at`], its arc filling from the top when `centred`.
#[allow(clippy::too_many_arguments)]
fn knob_with(ui: &mut Ui, painter: &Painter, c: Pos2, r: f32, k: &Knob, centred: bool, id: egui::Id, color: Color32, acts: &mut Vec<Act>) {
    let (label, text, amount, edit) = k;
    let resp = knob_face(ui, painter, c, r, label, text, *amount, centred, id, color);
    if let Some(edit) = edit {
        drag(ui, &resp, id.with("drag"), *amount, 150.0, *edit, acts);
    }
}

/// A knob's look and hit area, without what a drag does.
#[allow(clippy::too_many_arguments)]
pub(super) fn knob_face(
    ui: &mut Ui,
    painter: &Painter,
    c: Pos2,
    r: f32,
    label: &str,
    text: &str,
    amount: f32,
    centred: bool,
    id: egui::Id,
    color: Color32,
) -> Response {
    let area = Rect::from_center_size(c + Vec2::new(0.0, 6.0), Vec2::new(r * 2.0 + 24.0, r * 2.0 + 26.0));
    let resp = ui.interact(area, id, Sense::click_and_drag()).on_hover_text("Drag (Shift: fine); double-click: default");
    let start = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    let arc = |from: f32, to: f32, col: Color32, w: f32| {
        let n = 32;
        let pts: Vec<Pos2> = (0..=n).map(|i| c + Vec2::angled(from + (to - from) * i as f32 / n as f32) * r).collect();
        painter.add(egui::Shape::line(pts, Stroke::new(w, col)));
    };
    let w = if r > 18.0 { 5.0 } else { 4.0 };
    arc(start, start + sweep, Color32::from_rgb(0x3a, 0x3a, 0x3a), w);
    let a = amount.clamp(0.0, 1.0);
    if centred {
        if (a - 0.5).abs() > 0.004 {
            arc(start + sweep * 0.5, start + sweep * a, color, w);
        }
    } else if a > 0.004 {
        arc(start, start + sweep * a, color, w);
    }
    // The pointer.
    let ang = start + sweep * a;
    painter.line_segment([c + Vec2::angled(ang) * (r * 0.25), c + Vec2::angled(ang) * (r * 0.72)], Stroke::new(3.0, white(170)));
    let live = resp.hovered() || resp.dragged();
    let below = Pos2::new(c.x, c.y + r + 12.0);
    if live && !text.is_empty() {
        painter.text(below, Align2::CENTER_CENTER, text, font(12.0), white(245));
    } else {
        painter.text(below, Align2::CENTER_CENTER, title_case(label), font(12.0), white(160));
    }
    resp
}

/// A choice as Maschine's strip shows it: "◂ Name ▸"; the arrows step, a click on the
/// name lists them all.
fn choice_at(ui: &mut Ui, painter: &Painter, center: Pos2, k: &Knob, id: egui::Id, acts: &mut Vec<Act>) {
    choice_sized(ui, painter, center, 78.0, k, id, acts)
}

/// [`choice_at`] with the name `width` wide.
fn choice_sized(ui: &mut Ui, painter: &Painter, center: Pos2, width: f32, k: &Knob, id: egui::Id, acts: &mut Vec<Act>) {
    let Some(Edit::Choice(t, f, options, cur, _)) = k.3 else { return };
    if let Some(i) = choice_pick(ui, painter, center, width, options, cur, id) {
        acts.push(Act::SetParam(t, f, i as f64));
    }
}

/// The picking half of [`choice_sized`]: the option a click or arrow chose, if it moved.
pub(super) fn choice_pick(
    ui: &mut Ui,
    painter: &Painter,
    center: Pos2,
    width: f32,
    options: &[&str],
    cur: usize,
    id: egui::Id,
) -> Option<usize> {
    let mut picked = None;
    let name = options.get(cur).copied().unwrap_or("");
    let name_r = Rect::from_center_size(center, Vec2::new(width, 20.0));
    let resp = ui.interact(name_r, id, Sense::click());
    painter.text(center, Align2::CENTER_CENTER, name, font(13.0), if resp.hovered() { Color32::WHITE } else { white(220) });
    if let Some(i) = choice_list(&resp, options.iter().copied(), cur)
        && i != cur
    {
        picked = Some(i);
    }
    for (dir, x) in [(-1i64, name_r.left() - 6.0), (1, name_r.right() + 6.0)] {
        let b = Rect::from_center_size(Pos2::new(x, center.y), Vec2::new(14.0, 18.0));
        let next = cur as i64 + dir;
        let ok = (0..options.len() as i64).contains(&next);
        let r = ui.interact(b, id.with(dir), Sense::click());
        let ink = if !ok {
            white(40)
        } else if r.hovered() {
            Color32::WHITE
        } else {
            white(150)
        };
        let c = b.center();
        let pts = if dir < 0 {
            vec![c + Vec2::new(3.0, -4.0), c + Vec2::new(3.0, 4.0), c + Vec2::new(-3.0, 0.0)]
        } else {
            vec![c + Vec2::new(-3.0, -4.0), c + Vec2::new(-3.0, 4.0), c + Vec2::new(3.0, 0.0)]
        };
        painter.add(egui::Shape::convex_polygon(pts, ink, Stroke::NONE));
        if ok && r.clicked() {
            picked = Some(next as usize);
        }
    }
    picked
}

/// An on/off switch with its label below.
fn toggle_at(ui: &mut Ui, painter: &Painter, c: Pos2, k: &Knob, id: egui::Id, color: Color32, acts: &mut Vec<Act>) {
    let Some(Edit::Toggle(t, f, on)) = k.3 else { return };
    let b = Rect::from_center_size(c, Vec2::new(48.0, 20.0));
    let resp = ui.interact(b, id, Sense::click());
    painter.rect_filled(
        b,
        3.0,
        if on {
            color
        } else if resp.hovered() {
            S03
        } else {
            S02
        },
    );
    painter.text(
        b.center(),
        Align2::CENTER_CENTER,
        if on { "ON" } else { "OFF" },
        font(11.0),
        if on { Color32::BLACK } else { white(200) },
    );
    painter.text(Pos2::new(c.x, c.y + 28.0), Align2::CENTER_CENTER, title_case(&k.0), font(12.0), white(160));
    if resp.clicked() {
        acts.push(Act::SetParam(t, f, if on { 0.0 } else { 1.0 }));
    }
}

/// One cell of a parameter grid: a knob, a choice or a switch.
#[allow(clippy::too_many_arguments)]
fn cell(ui: &mut Ui, painter: &Painter, c: Pos2, r: f32, k: &Knob, id: egui::Id, color: Color32, acts: &mut Vec<Act>) {
    match k.3 {
        Some(Edit::Choice(..)) => {
            choice_at(ui, painter, c, k, id, acts);
            painter.text(Pos2::new(c.x, c.y + r + 12.0), Align2::CENTER_CENTER, title_case(&k.0), font(12.0), white(160));
        }
        Some(Edit::Toggle(..)) => toggle_at(ui, painter, c, k, id, color, acts),
        _ => knob_at(ui, painter, c, r, k, id, color, acts),
    }
}

/// A vertical level meter with a dB scale, and a handle on it for a parameter (the
/// Compressor's Threshold and Gain). `span`: the scale's dB (0 at the top down to
/// `-span`); `level`: what the meter shows (linear); `handle`: the parameter (0..1, top to
/// bottom when `down`); `marks`: the scale's labels.
#[allow(clippy::too_many_arguments)]
fn meter_fader(
    ui: &mut Ui,
    painter: &Painter,
    col: Rect,
    span: f32,
    level: f32,
    handle: Option<(&Knob, bool)>,
    marks: &[f32],
    id: egui::Id,
    color: Color32,
    acts: &mut Vec<Act>,
) {
    let bar = Rect::from_min_size(Pos2::new(col.left() + 30.0, col.top()), Vec2::new(10.0, col.height()));
    painter.rect_filled(bar, 0.0, Color32::from_rgb(0x2c, 0x2c, 0x2c));
    let y_of = |db: f32| bar.top() + (-db / span).clamp(0.0, 1.0) * bar.height();
    for &m in marks {
        painter.text(Pos2::new(bar.left() - 6.0, y_of(m)), Align2::RIGHT_CENTER, format!("{m:.0}"), font(10.0), white(90));
    }
    if level > 1e-6 {
        let db = 20.0 * level.log10();
        if db > -span {
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(bar.left() + 2.0, y_of(db)), Pos2::new(bar.right() - 2.0, bar.bottom())),
                0.0,
                white(200),
            );
        }
    }
    let Some((k, down)) = handle else { return };
    let v = k.2.clamp(0.0, 1.0);
    let hy = bar.top() + if down { v } else { 1.0 - v } * bar.height();
    let hx = bar.right() + 3.0;
    painter.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(hx, hy),
            Pos2::new(hx + 6.0, hy - 6.0),
            Pos2::new(hx + 18.0, hy - 6.0),
            Pos2::new(hx + 18.0, hy + 6.0),
            Pos2::new(hx + 6.0, hy + 6.0),
        ],
        color,
        Stroke::NONE,
    ));
    let area = Rect::from_min_max(Pos2::new(bar.left() - 4.0, col.top() - 6.0), Pos2::new(hx + 20.0, col.bottom() + 6.0));
    let resp =
        ui.interact(area, id, Sense::click_and_drag()).on_hover_text(format!("{} {} (drag; double-click: default)", title_case(&k.0), k.1));
    if (resp.hovered() || resp.dragged()) && !k.1.is_empty() {
        painter.text(Pos2::new(hx + 22.0, hy), Align2::LEFT_CENTER, &k.1, font(11.0), white(245));
    }
    if let Some(edit) = k.3 {
        // Dragging down lowers a Gain (up the scale) but raises a Threshold's 0..1.
        let px = if down { -bar.height() } else { bar.height() };
        drag(ui, &resp, id.with("drag"), k.2, px, edit, acts);
    }
}

/// The Compressor, as Maschine 3's strip shows it: Threshold over the input meter, the GR
/// meter, Knee and Amount over Attack and Release, Gain over the output meter.
#[allow(clippy::too_many_arguments)]
fn compressor(
    ui: &mut Ui,
    painter: &Painter,
    body: Rect,
    slot: &deckard_model::SlotInfo,
    g: usize,
    s: usize,
    i: usize,
    meters: (f32, f32, f32),
    color: Color32,
    acts: &mut Vec<Act>,
) {
    let pg = crate::module_ui::page(slot, g, s, i, 0);
    let id = ui.id().with(("strip-comp", g, s, i));
    if let Some(k) = &pg.slots[0] {
        choice_at(ui, painter, Pos2::new(body.right() - 60.0, body.top() + 18.0), k, id.with("mode"), acts);
    }
    let top = body.top() + 46.0;
    let h = body.height() - 82.0;
    // Threshold (0 to −60 dB) over the input level.
    let thr = Rect::from_min_size(Pos2::new(body.left() + 10.0, top), Vec2::new(60.0, h));
    let marks60 = [0.0, -10.0, -20.0, -30.0, -40.0, -50.0, -60.0];
    meter_fader(ui, painter, thr, 60.0, meters.0, pg.slots[1].as_ref().map(|k| (k, true)), &marks60, id.with("thr"), color, acts);
    painter.text(Pos2::new(thr.left() + 34.0, body.bottom() - 18.0), Align2::CENTER_CENTER, "Threshold", font(12.0), white(160));
    // Gain reduction: down from the top, over 60 dB.
    let gr = Rect::from_min_size(Pos2::new(thr.right() + 22.0, top), Vec2::new(10.0, h));
    painter.rect_filled(gr, 0.0, Color32::from_rgb(0x2c, 0x2c, 0x2c));
    let red = (meters.1 / 60.0).clamp(0.0, 1.0);
    if red > 0.002 {
        painter.rect_filled(
            Rect::from_min_size(gr.min + Vec2::new(2.0, 0.0), Vec2::new(gr.width() - 4.0, gr.height() * red)),
            0.0,
            GR_GREEN,
        );
    }
    let gr_resp = ui.interact(gr, id.with("gr"), Sense::hover());
    gr_resp.on_hover_text(format!("Gain reduction {:.1} dB", meters.1));
    painter.text(Pos2::new(gr.center().x, body.bottom() - 18.0), Align2::CENTER_CENTER, "GR", font(12.0), white(160));
    // The knobs: Knee and Amount, then Attack and Release.
    let kx = gr.right() + 50.0;
    for (n, (slot_ix, r, dx, dy)) in
        [(2usize, 23.0, 0.0, 30.0), (3, 23.0, 86.0, 30.0), (4, 17.0, 0.0, 108.0), (5, 17.0, 86.0, 108.0)].into_iter().enumerate()
    {
        if let Some(k) = &pg.slots[slot_ix] {
            knob_at(ui, painter, Pos2::new(kx + dx, body.top() + dy + 20.0), r, k, id.with(("k", n)), color, acts);
        }
    }
    // Gain (−36 to 0 dB) over the output level.
    let gain = Rect::from_min_size(Pos2::new(kx + 120.0, top), Vec2::new(60.0, h));
    let marks36 = [0.0, -6.0, -12.0, -18.0, -24.0, -30.0, -36.0];
    meter_fader(ui, painter, gain, 36.0, meters.2, pg.slots[6].as_ref().map(|k| (k, false)), &marks36, id.with("gain"), color, acts);
    painter.text(Pos2::new(gain.left() + 34.0, body.bottom() - 18.0), Align2::CENTER_CENTER, "Gain", font(12.0), white(160));
}

/// The EQ by band, as Maschine's strip shows it: the output Gain, then per band its Gain
/// above its Freq (and Width).
#[allow(clippy::too_many_arguments)]
fn eq(
    ui: &mut Ui,
    painter: &Painter,
    body: Rect,
    slot: &deckard_model::SlotInfo,
    g: usize,
    s: usize,
    i: usize,
    color: Color32,
    acts: &mut Vec<Act>,
) {
    let (a, b) = (crate::module_ui::page(slot, g, s, i, 0), crate::module_ui::page(slot, g, s, i, 1));
    let id = ui.id().with(("strip-eq", g, s, i));
    let (top_y, low_y) = (body.top() + 72.0, body.top() + 150.0);
    if let Some(k) = &b.slots[7] {
        knob_at(ui, painter, Pos2::new(body.left() + 40.0, low_y), 16.0, k, id.with("out"), color, acts);
    }
    // (column x, heading, gain slot, freq slot, width slot on page 2)
    let bands =
        [(130.0, "LOW", 1, 0, None), (230.0, "LOW-MID", 3, 2, Some(2)), (360.0, "HIGH-MID", 5, 4, Some(5)), (470.0, "HIGH", 7, 6, None)];
    for (n, (x, head, gain, freq, width)) in bands.into_iter().enumerate() {
        let cx = body.left() + x;
        painter.text(Pos2::new(cx, body.top() + 32.0), Align2::CENTER_CENTER, head, font(12.0), white(200));
        if let Some(k) = &a.slots[gain] {
            knob_at(ui, painter, Pos2::new(cx, top_y), 22.0, k, id.with(("g", n)), color, acts);
        }
        let (fx, wx) = if width.is_some() { (cx - 30.0, cx + 30.0) } else { (cx, cx) };
        if let Some(k) = &a.slots[freq] {
            knob_at(ui, painter, Pos2::new(fx, low_y), 16.0, k, id.with(("f", n)), color, acts);
        }
        if let Some(w) = width
            && let Some(k) = &b.slots[w]
        {
            knob_at(ui, painter, Pos2::new(wx, low_y), 16.0, k, id.with(("w", n)), color, acts);
        }
    }
}

/// Up to eight parameters in two rows of four (a page of Maschine's own effects, the
/// Sampler's or a plug-in's); a choice in the first slot labelled MODE goes in the title
/// row instead.
fn grid(ui: &mut Ui, painter: &Painter, body: Rect, slots: &[Option<Knob>], id: egui::Id, color: Color32, acts: &mut Vec<Act>) {
    let mode_first = matches!(slots.first(), Some(Some(k)) if k.0 == "MODE" && matches!(k.3, Some(Edit::Choice(..))));
    if mode_first {
        choice_at(ui, painter, Pos2::new(body.right() - 60.0, body.top() + 18.0), slots[0].as_ref().unwrap(), id.with("mode"), acts);
    }
    let items: Vec<&Knob> = slots.iter().enumerate().filter(|(k, _)| !(mode_first && *k == 0)).filter_map(|(_, k)| k.as_ref()).collect();
    for (n, k) in items.iter().enumerate().take(8) {
        let (row, colm) = (n / 4, n % 4);
        let c = Pos2::new(body.left() + 50.0 + colm as f32 * 76.0, body.top() + 68.0 + row as f32 * 78.0);
        cell(ui, painter, c, 18.0, k, id.with(("cell", n)), color, acts);
    }
}

/// A panel's width.
fn panel_width(slot: &deckard_model::SlotInfo) -> f32 {
    match slot.module.as_str() {
        "Compressor" => 360.0,
        "EQ" => 520.0,
        "Sampler" if slot.plugin.is_none() => sampler::WIDTH,
        m if slot.plugin.is_none() && crate::drum_ui::is_drum(m) => drum::width(),
        "PolySynth" if slot.plugin.is_none() => drum::poly_width(),
        m if slot.plugin.is_none() && editor::editor(m).is_some() => editor::width(editor::editor(m).unwrap()),
        _ => 330.0,
    }
}

/// The page shown by a panel that has several (Sampler, plug-ins), kept per panel.
fn panel_page(ui: &Ui, id: egui::Id) -> usize {
    ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0)
}

/// "◂ name ▸" stepping through `names`, kept per panel.
fn page_stepper(ui: &mut Ui, painter: &Painter, center: Pos2, names: &[String], id: egui::Id) -> usize {
    let n = names.len().max(1);
    let cur = panel_page(ui, id).min(n - 1);
    let name_r = Rect::from_center_size(center, Vec2::new(150.0, 20.0));
    painter.text(center, Align2::CENTER_CENTER, names.get(cur).map_or("", String::as_str), font(12.0), white(220));
    for (dir, x) in [(-1i64, name_r.left()), (1, name_r.right())] {
        let b = Rect::from_center_size(Pos2::new(x, center.y), Vec2::new(14.0, 18.0));
        let next = cur as i64 + dir;
        let ok = (0..n as i64).contains(&next);
        let r = ui.interact(b, id.with(("step", dir)), Sense::click());
        let c = b.center();
        let pts = if dir < 0 {
            vec![c + Vec2::new(3.0, -4.0), c + Vec2::new(3.0, 4.0), c + Vec2::new(-3.0, 0.0)]
        } else {
            vec![c + Vec2::new(-3.0, -4.0), c + Vec2::new(-3.0, 4.0), c + Vec2::new(3.0, 0.0)]
        };
        painter.add(egui::Shape::convex_polygon(
            pts,
            if !ok {
                white(40)
            } else if r.hovered() {
                Color32::WHITE
            } else {
                white(150)
            },
            Stroke::NONE,
        ));
        if ok && r.clicked() {
            ui.data_mut(|d| d.insert_temp(id, next as usize));
        }
    }
    panel_page(ui, id).min(n - 1)
}

/// The plug-in strip of the focused sound, in `rect`.
pub fn plugin_strip(ui: &mut Ui, app: &App, doc: &deckard_doc::Document, rect: Rect, acts: &mut Vec<Act>) {
    let p = doc.project();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_rgb(0x1b, 0x1b, 0x1b));
    let (g, s) = (app.group, app.sound);
    let Some(snd) = p.groups.get(g).and_then(|x| x.sounds.get(s)) else { return };
    let color = group_color(p, g);
    let widths: Vec<f32> = snd.slots.iter().map(panel_width).collect();
    let content = widths.iter().map(|w| w + PANEL_GAP).sum::<f32>() + ADD_W + 2.0 * PANEL_GAP;
    let area = rect.shrink2(Vec2::new(4.0, 0.0));
    let off = hscroll(ui, "plugin-strip", area, content);
    let clip = painter.with_clip_rect(area);
    let mut x = area.left() + PANEL_GAP - off;
    let mut panels = Vec::new();
    for (i, slot) in snd.slots.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(x, area.top() + PANEL_TOP), Vec2::new(widths[i], area.height() - PANEL_TOP - 10.0));
        x += widths[i] + PANEL_GAP;
        if r.right() < area.left() || r.left() > area.right() {
            continue;
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.intersect(area)));
        panel(&mut child, app, &clip, r, snd, slot, g, s, i, color, acts);
        panels.push((i, r));
    }
    // "+": add a plug-in at the end of the chain.
    let add = Rect::from_min_size(Pos2::new(x, area.top() + PANEL_TOP), Vec2::new(ADD_W, area.height() - PANEL_TOP - 10.0));
    let resp = ui.interact(add.intersect(area), ui.id().with("strip-add"), Sense::click()).on_hover_text("Add a plug-in");
    clip.rect_stroke(add, 3.0, Stroke::new(1.0, if resp.hovered() { white(120) } else { white(50) }), StrokeKind::Inside);
    clip.text(add.center(), Align2::CENTER_CENTER, "+", font(20.0), white(if resp.hovered() { 230 } else { 150 }));
    // A sound has one instrument, in its first slot.
    let instruments = snd.slots.is_empty();
    egui::Popup::menu(&resp).show(|ui| add_menu(ui, app, instruments, acts));
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-strip-panels"), panels));
}

/// One plug-in's panel in `r`: its header bar (bypass, name), its title, its controls.
#[allow(clippy::too_many_arguments)]
fn panel(
    ui: &mut Ui,
    app: &App,
    painter: &Painter,
    r: Rect,
    snd: &deckard_model::Sound,
    slot: &deckard_model::SlotInfo,
    g: usize,
    s: usize,
    i: usize,
    color: Color32,
    acts: &mut Vec<Act>,
) {
    let id = ui.id().with(("strip-panel", g, s, i));
    let focused = app.slot == Some(i);
    // The header bar.
    let bar = Rect::from_min_size(r.min, Vec2::new(r.width(), BAR_H));
    painter.rect_filled(bar, 2.0, Color32::from_rgb(0x2a, 0x2a, 0x2a));
    painter.rect_stroke(bar, 2.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Inside);
    let power = Rect::from_center_size(Pos2::new(bar.left() + 12.0, bar.center().y), Vec2::splat(14.0));
    let pr = ui.interact(power, id.with("bypass"), Sense::click()).on_hover_text(if slot.active { "Bypass" } else { "Turn on" });
    let ink = if slot.active { color } else { white(90) };
    painter.circle_stroke(power.center(), 5.0, Stroke::new(1.5, ink));
    painter.line_segment([power.center() + Vec2::new(0.0, -7.0), power.center() + Vec2::new(0.0, -1.0)], Stroke::new(1.5, ink));
    if pr.clicked() {
        acts.push(Act::SlotActiveOf(g, s, i, !slot.active));
    }
    let name_r = Rect::from_min_max(Pos2::new(power.right() + 6.0, bar.top()), bar.max);
    let nr = ui.interact(name_r, id.with("name"), Sense::click()).on_hover_text("Focus (its pages in the SOUND tab)");
    painter.text(
        Pos2::new(name_r.left(), bar.center().y),
        Align2::LEFT_CENTER,
        &slot.name,
        font(11.0),
        white(if nr.hovered() { 245 } else { 180 }),
    );
    if nr.clicked() {
        acts.push(Act::FocusSlot(g, s, i));
    }
    // The body.
    let body = Rect::from_min_max(Pos2::new(r.left(), bar.bottom() + 4.0), r.max);
    painter.rect_filled(body, 3.0, Color32::from_rgb(0x22, 0x22, 0x22));
    if focused {
        painter.rect_stroke(body, 3.0, Stroke::new(1.0, white(40)), StrokeKind::Inside);
    }
    let shown = match &slot.plugin {
        Some(pr) => pr.name.clone(),
        None => match editor::editor(&slot.module) {
            // As Maschine titles its own ("LEGACY REVERB", "DISTORTION").
            Some(ed) => ed.name.to_string(),
            None => deckard_model::module_label(&slot.module).to_string(),
        },
    };
    title(painter, Pos2::new(body.left() + 14.0, body.top() + 18.0), &shown);
    let inner = body.shrink2(Vec2::new(4.0, 0.0));
    if !slot.active {
        painter.rect_filled(body, 3.0, Color32::from_black_alpha(90));
    }
    // Its knobs where its routes have them now (a plug-in's show that themselves); their
    // edits start from their own values.
    let (live, mut bases) = app.live_slot_bases((g, s, i), snd, slot, Target::Module(g, s, i));
    if slot.module == "Sampler"
        && let Some(smp) = &snd.sampler
    {
        bases = app.live_sampler_bases(g, s, snd, smp).1;
    }
    super::mod_routes::set_bases(ui, bases);
    let slot: &deckard_model::SlotInfo = &live;
    match (slot.module.as_str(), &slot.plugin) {
        ("Compressor", None) => {
            let m = app.fx_meter_view.get(&(g, s, i)).copied().unwrap_or_default();
            compressor(ui, painter, inner, slot, g, s, i, m, color, acts)
        }
        ("EQ", None) => eq(ui, painter, inner, slot, g, s, i, color, acts),
        (m, None) if crate::drum_ui::is_drum(m) => drum::draw(ui, painter, inner, slot, (g, s, i), color, acts),
        ("PolySynth", None) => drum::poly_draw(ui, painter, inner, slot, (g, s, i), color, acts),
        (m, None) if editor::editor(m).is_some() => {
            let m = app.fx_meter_view.get(&(g, s, i)).copied().unwrap_or_default();
            editor::draw(ui, painter, inner, editor::editor(&slot.module).unwrap(), slot, (g, s, i), m, color, acts)
        }
        (m, None) if crate::module_ui::pages(m).is_some() => {
            let pg = crate::module_ui::page(slot, g, s, i, 0);
            grid(ui, painter, inner, &pg.slots, id, color, acts);
        }
        ("Sampler", None) => {
            if let Some(smp) = &snd.sampler {
                let live = app.live_sampler(g, s, snd, smp);
                sampler::panel(ui, app, painter, body, &live, g, s, color, acts);
            }
        }
        (_, Some(_)) => {
            // A hosted plug-in: its pages when it's the focused slot (its values come
            // from the instance), else a click to show them.
            match app.plugin_view.as_ref().filter(|v| v.key == (deckard_doc::Chain::Sound(g, s), i) && v.error.is_none()) {
                Some(v) => {
                    let names: Vec<String> = v
                        .pages
                        .iter()
                        .enumerate()
                        .map(|(k, p)| if p.name.is_empty() { format!("Page {}", k + 1) } else { p.name.clone() })
                        .collect();
                    let cur = app.plugin_page.min(names.len().saturating_sub(1));
                    let pid = id.with("page");
                    if panel_page(ui, pid) != cur {
                        ui.data_mut(|d| d.insert_temp(pid, cur));
                    }
                    let page = page_stepper(ui, painter, Pos2::new(body.right() - 100.0, body.top() + 18.0), &names, pid);
                    if page != cur {
                        acts.push(Act::PluginPage(page));
                    }
                    if page == v.page
                        && let Some(pg) = v.pages.get(page)
                    {
                        let knobs: Vec<Option<Knob>> = pg
                            .knobs
                            .iter()
                            .enumerate()
                            .map(|(k, (info, label, _))| {
                                let (value, text) = v.values.get(k).cloned().unwrap_or((info.default, String::new()));
                                Some((Cow::Owned(label.to_uppercase()), text, value as f32, Some(Edit::Plugin(i, info.id, info.default))))
                            })
                            .collect();
                        grid(ui, painter, inner, &knobs, id.with(("plugin", page)), color, acts);
                    }
                }
                None => {
                    let b = Rect::from_center_size(body.center(), Vec2::new(150.0, 26.0));
                    let resp = ui.interact(b, id.with("show"), Sense::click());
                    painter.rect_filled(b, 3.0, if resp.hovered() { S03 } else { S02 });
                    painter.text(b.center(), Align2::CENTER_CENTER, "Show parameters", font(12.0), white(220));
                    if resp.clicked() {
                        acts.push(Act::FocusSlot(g, s, i));
                    }
                }
            }
        }
        _ => {
            painter.text(body.center(), Align2::CENTER_CENTER, "No controls yet", font(12.0), white(120));
        }
    }
}
