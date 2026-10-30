//! The Sampler's panel in the plug-in strip, as Maschine 3 draws it (SamplerEditor.qml,
//! SamplerMainPage.qml, SamplerFXPage.qml): a Main page with the zone's waveform and its
//! play, loop and voice settings, and an FX page with Tune, Saturation, Lo-fi, the
//! filter's response and the amplitude envelope, each with handles to drag.

use super::*;
use deckard_engine::sampler as dsp;
use deckard_model::{Sampler, Zone};

/// The panel's width (Maschine's is 871).
pub const WIDTH: f32 = 880.0;
/// Separator and wave-box border grey (#2A2A2A); the displays' background (#181818).
const LINE: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
const DISPLAY: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
/// Section titles on the FX page (#a3FFFFFF).
const HEAD: Color32 = Color32::from_rgba_premultiplied(0xa3, 0xa3, 0xa3, 0xa3);

const POLY: [&str; 8] = ["Legato", "1", "2", "4", "8", "16", "32", "64"];
const MODEL: [&str; 7] = ["Standard", "MP60", "S1200", "S1200 L", "S1200 LM", "S1200 HM", "S1200 H"];
const ENVELOPE: [&str; 3] = ["One-shot", "AHD", "ADSR"];
const FILTER: [&str; 5] = ["Off", "LP2", "BP2", "HP2", "EQ"];

fn bold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("bold".into()))
}

/// The bold font if the style has one, else the regular.
fn heading_font(ui: &Ui, size: f32) -> FontId {
    let has = ui.ctx().fonts(|f| f.families().iter().any(|x| *x == egui::FontFamily::Name("bold".into())));
    if has { bold(size) } else { font(size) }
}

/// The panel's body (below the header bar and title), page tabs included.
#[allow(clippy::too_many_arguments)]
pub fn panel(
    ui: &mut Ui,
    app: &App,
    painter: &Painter,
    body: Rect,
    smp: &Sampler,
    g: usize,
    s: usize,
    color: Color32,
    acts: &mut Vec<Act>,
) {
    let id = ui.id().with(("strip-sampler", g, s));
    // "Main | FX", top right.
    let page_id = id.with("page");
    let mut page = ui.data(|d| d.get_temp::<usize>(page_id)).unwrap_or(0);
    let mut x = body.right() - 14.0;
    for (k, name) in ["FX", "Main"].into_iter().enumerate().map(|(k, n)| (1 - k, n)) {
        let galley = painter.layout_no_wrap(name.into(), font(15.0), Color32::WHITE);
        let r = Rect::from_min_size(Pos2::new(x - galley.size().x, body.top() + 8.0), galley.size());
        let resp = ui.interact(r.expand(3.0), id.with(("tab", k)), Sense::click());
        let ink = if k == page {
            Color32::WHITE
        } else if resp.hovered() {
            white(200)
        } else {
            white(120)
        };
        painter.galley_with_override_text_color(r.min, galley, ink);
        if resp.clicked() {
            page = k;
            ui.data_mut(|d| d.insert_temp(page_id, k));
        }
        x = r.left() - 10.0;
        if k == 1 {
            painter.vline(x + 5.0, Rect::from_min_size(r.min, r.size()).y_range(), Stroke::new(1.0, white(60)));
        }
    }
    let content = Rect::from_min_max(Pos2::new(body.left() + 16.0, body.top() + 36.0), Pos2::new(body.right() - 16.0, body.bottom() - 6.0));
    if page == 0 {
        main_page(ui, app, painter, content, smp, g, s, color, id, acts);
    } else {
        fx_page(ui, painter, content, smp, g, s, color, id, acts);
    }
}

// ---- Main ------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn main_page(
    ui: &mut Ui,
    app: &App,
    painter: &Painter,
    area: Rect,
    smp: &Sampler,
    g: usize,
    s: usize,
    color: Color32,
    id: egui::Id,
    acts: &mut Vec<Act>,
) {
    painter.rect_filled(area, 3.0, DISPLAY);
    let wave = Rect::from_min_size(area.min, Vec2::new(area.width(), (area.height() - 43.0).max(40.0)));
    painter.rect_stroke(wave, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
    let zi = app.zone.min(smp.zones.len().saturating_sub(1));
    let Some(zone) = smp.zones.get(zi) else {
        painter.text(wave.center(), Align2::CENTER_CENTER, "No sample", font(12.0), white(110));
        return;
    };
    let wf = app.waveform.as_ref().filter(|w| (w.key.0, w.key.1, w.key.2) == (g, s, zi));
    let zt = Target::Zone(g, s, zi);
    let len = wf.map_or(zone.play_range.1.max(1) + 1, |w| w.len as i64);
    match wf {
        Some(w) => waveform(ui, painter, wave.shrink2(Vec2::new(1.0, 0.0)), w, zone, zt, color, app.zone_playhead, id, acts),
        None => {
            painter.text(wave.center(), Align2::CENTER_CENTER, "Loading…", font(12.0), white(110));
        }
    }
    // The controls row: PLAY | LOOP | VOICE / ENGINE.
    let row = Rect::from_min_max(Pos2::new(area.left(), wave.bottom()), area.max);
    let base = row.bottom() - 9.0; // the values' and section labels' baseline
    let label_y = row.top() + 12.0;
    let st = Target::Sampler(g, s);
    let mut x = row.left() + 12.0;
    let hf = heading_font(ui, 13.0);
    let section = |painter: &Painter, x: f32, text: &str| -> f32 {
        let r = painter.text(Pos2::new(x, base), Align2::LEFT_BOTTOM, text, hf.clone(), Color32::WHITE);
        r.right()
    };
    let separator = |painter: &Painter, x: f32| {
        painter.vline(x, Rect::from_min_size(row.min, Vec2::new(0.0, row.height() - 4.0)).y_range(), Stroke::new(1.0, LINE))
    };
    let frames = |v: i64| v.to_string();
    let (lo, hi) = zone.play_range;
    x = section(painter, x, "PLAY") + 14.0;
    let span = (len - 1).max(1);
    if let Some(v) =
        numeric(ui, painter, Pos2::new(x, label_y), base, "START", &frames(lo), color, true, id.with("play-lo"), lo as f64, span as f64)
    {
        acts.push(Act::SetParam(zt, "play_range/low", v.clamp(0.0, (hi - 1).max(0) as f64).round()));
    }
    x += 64.0;
    if let Some(v) =
        numeric(ui, painter, Pos2::new(x, label_y), base, "END", &frames(hi), color, true, id.with("play-hi"), hi as f64, span as f64)
    {
        acts.push(Act::SetParam(zt, "play_range/high", v.clamp((lo + 1) as f64, span as f64).round()));
    }
    x += 76.0;
    separator(painter, x);
    x = section(painter, x + 12.0, "LOOP") + 12.0;
    if power(ui, painter, Pos2::new(x + 6.0, base - 7.0), zone.loop_active, color, id.with("loop")) {
        acts.push(Act::SetParam(zt, "loop_active", if zone.loop_active { 0.0 } else { 1.0 }));
    }
    x += 30.0;
    let (llo, lhi) = zone.loop_range;
    let on = zone.loop_active;
    if let Some(v) =
        numeric(ui, painter, Pos2::new(x, label_y), base, "START", &frames(llo), color, on, id.with("loop-lo"), llo as f64, span as f64)
    {
        acts.push(Act::SetParam(zt, "loop_range/low", v.clamp(0.0, (lhi - 1).max(0) as f64).round()));
    }
    x += 64.0;
    if let Some(v) =
        numeric(ui, painter, Pos2::new(x, label_y), base, "END", &frames(lhi), color, on, id.with("loop-hi"), lhi as f64, span as f64)
    {
        acts.push(Act::SetParam(zt, "loop_range/high", v.clamp((llo + 1) as f64, span as f64).round()));
    }
    x += 64.0;
    let xf_max = (lhi - llo).max(0) as f64;
    if let Some(v) = numeric(
        ui,
        painter,
        Pos2::new(x, label_y),
        base,
        "X-FADE",
        &frames(zone.x_fade),
        color,
        on,
        id.with("xfade"),
        zone.x_fade as f64,
        xf_max.max(1.0),
    ) {
        acts.push(Act::SetParam(zt, "x_fade", v.clamp(0.0, xf_max).round()));
    }
    x += 56.0;
    separator(painter, x);
    x = section(painter, x + 12.0, "VOICE / ENGINE") + 26.0;
    painter.text(Pos2::new(x, label_y), Align2::LEFT_CENTER, "REVERSE", font(11.0), white(150));
    if power(ui, painter, Pos2::new(x + 22.0, base - 7.0), smp.reverse, color, id.with("reverse")) {
        acts.push(Act::SetParam(st, "reverse", if smp.reverse { 0.0 } else { 1.0 }));
    }
    x += 66.0;
    let poly = (smp.get("polyphony").max(0.0) as usize).min(POLY.len() - 1);
    if let Some(i) = dropdown(ui, painter, Pos2::new(x, label_y), base, "POLY", &POLY, poly, color, id.with("poly")) {
        acts.push(Act::SetParam(st, "polyphony", i as f64));
    }
    x += 58.0;
    let legato = poly == 0;
    let gl = smp.get("glide");
    let glide = if legato { format!("{:.2}", gl * gl) } else { "0.00".into() };
    // (Dragged from its own value when a route moves it.)
    let own = super::super::mod_routes::ui_base(ui, st, "glide", gl);
    if let Some(v) = numeric(ui, painter, Pos2::new(x, label_y), base, "GLIDE", &glide, Color32::WHITE, legato, id.with("glide"), own, 1.0)
    {
        acts.push(Act::SetParam(st, "glide", v.clamp(0.0, 1.0)));
    }
    x += 46.0;
    let model = (smp.get("mode").max(0.0) as usize).min(MODEL.len() - 1);
    if let Some(i) = dropdown(ui, painter, Pos2::new(x, label_y), base, "MODEL", &MODEL, model, color, id.with("model")) {
        acts.push(Act::SetParam(st, "mode", i as f64));
    }
}

/// A value under its label (Maschine's NumericArea): drag up or down to change it, over
/// `range` for 300 px (Shift: fine). Returns the new value.
#[allow(clippy::too_many_arguments)]
fn numeric(
    ui: &mut Ui,
    painter: &Painter,
    label_at: Pos2,
    base: f32,
    label: &str,
    text: &str,
    color: Color32,
    enabled: bool,
    id: egui::Id,
    value: f64,
    range: f64,
) -> Option<f64> {
    painter.text(label_at, Align2::LEFT_CENTER, label, font(11.0), white(if enabled { 150 } else { 70 }));
    let r = painter.text(Pos2::new(label_at.x, base), Align2::LEFT_BOTTOM, text, font(15.0), if enabled { color } else { white(70) });
    if !enabled {
        return None;
    }
    let hit =
        Rect::from_min_max(Pos2::new(label_at.x - 2.0, label_at.y - 7.0), Pos2::new(r.right().max(label_at.x + 40.0) + 4.0, base + 2.0));
    let resp = ui.interact(hit, id, Sense::drag()).on_hover_text("Drag up or down (Shift: fine)");
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if resp.drag_started() {
        ui.data_mut(|d| d.insert_temp(id, (value, 0f64)));
    }
    if resp.dragged() && resp.drag_delta().y != 0.0 {
        let fine = ui.input(|i| i.modifiers.shift);
        let (from, moved) = ui.data(|d| d.get_temp::<(f64, f64)>(id)).unwrap_or((value, 0.0));
        let moved = moved - resp.drag_delta().y as f64 / if fine { 3000.0 } else { 300.0 };
        ui.data_mut(|d| d.insert_temp(id, (from, moved)));
        return Some(from + moved * range);
    }
    None
}

/// A power switch (Maschine's Toggle) centred at `c`; true when clicked.
fn power(ui: &mut Ui, painter: &Painter, c: Pos2, on: bool, color: Color32, id: egui::Id) -> bool {
    let resp = ui.interact(Rect::from_center_size(c, Vec2::splat(18.0)), id, Sense::click());
    let ink = if on {
        color
    } else if resp.hovered() {
        white(200)
    } else {
        white(110)
    };
    let r = 6.0;
    let gap = 0.9f32;
    let n = 24;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + gap + (std::f32::consts::TAU - 2.0 * gap) * i as f32 / n as f32;
            c + Vec2::angled(a) * r
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, ink)));
    painter.line_segment([c + Vec2::new(0.0, -r - 1.0), c + Vec2::new(0.0, -1.0)], Stroke::new(1.5, ink));
    resp.clicked()
}

/// A list choice under its label (Maschine's DropDown: "⌄ value"); returns the pick.
#[allow(clippy::too_many_arguments)]
fn dropdown(
    ui: &mut Ui,
    painter: &Painter,
    label_at: Pos2,
    base: f32,
    label: &str,
    options: &[&str],
    cur: usize,
    color: Color32,
    id: egui::Id,
) -> Option<usize> {
    painter.text(label_at, Align2::LEFT_CENTER, label, font(11.0), white(150));
    let chevron = Pos2::new(label_at.x + 5.0, base - 8.0);
    painter.add(egui::Shape::line(
        vec![chevron + Vec2::new(-4.0, -2.0), chevron + Vec2::new(0.0, 2.0), chevron + Vec2::new(4.0, -2.0)],
        Stroke::new(1.5, white(200)),
    ));
    let r =
        painter.text(Pos2::new(label_at.x + 14.0, base), Align2::LEFT_BOTTOM, options.get(cur).copied().unwrap_or(""), font(15.0), color);
    let hit = Rect::from_min_max(Pos2::new(label_at.x - 2.0, label_at.y - 7.0), Pos2::new(r.right() + 4.0, base + 2.0));
    let resp = ui.interact(hit, id, Sense::click()).on_hover_text(format!("{label}: click to choose"));
    choice_list(&resp, options.iter().copied(), cur).filter(|&i| i != cur)
}

/// Seconds between the ruler's labels for `secs` over `px` pixels: labels at least
/// 180 px apart (Maschine's 1.8 s sample, 0.5 s apart).
fn ruler_step(secs: f32, px: f32) -> f32 {
    const STEPS: [f32; 14] = [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0];
    STEPS.into_iter().find(|s| s / secs.max(1e-6) * px >= 180.0).unwrap_or(120.0)
}

/// The zone's two channels, the seconds ruler, the S and E markers (drag them; the loop's
/// too while it's on) and the playhead.
#[allow(clippy::too_many_arguments)]
fn waveform(
    ui: &mut Ui,
    painter: &Painter,
    rect: Rect,
    wf: &sampler_ui::Waveform,
    zone: &Zone,
    target: Target,
    color: Color32,
    playhead: Option<usize>,
    id: egui::Id,
    acts: &mut Vec<Act>,
) {
    let clip = painter.with_clip_rect(rect);
    let len = wf.len.max(1) as f32;
    let secs = len / wf.rate.max(1) as f32;
    // The ruler.
    let ruler_h = 20.0;
    let step = ruler_step(secs, rect.width());
    let minor = step / 5.0;
    let mut k = 0;
    loop {
        let t = k as f32 * minor;
        if t > secs {
            break;
        }
        let x = rect.left() + rect.width() * t / secs;
        let major = k % 5 == 0;
        clip.vline(x, rect.top() + 3.0..=rect.top() + if major { 18.0 } else { 9.0 }, Stroke::new(1.0, white(if major { 90 } else { 45 })));
        if major {
            let text = if step < 0.01 {
                format!("{:.3}s", t)
            } else if step < 0.1 {
                format!("{:.2}s", t)
            } else if step < 1.0 {
                format!("{:.1}s", t)
            } else {
                format!("{:.0}s", t)
            };
            clip.text(Pos2::new(x + 3.0, rect.top() + 12.0), Align2::LEFT_CENTER, text, font(10.0), white(150));
        }
        k += 1;
    }
    // Two lanes, one per channel.
    let lanes = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + ruler_h + 4.0), Pos2::new(rect.right(), rect.bottom() - 14.0));
    let x_of = |f: i64| rect.left() + rect.width() * (f as f32 / len).clamp(0.0, 1.0);
    let (lo, hi) = if zone.play_range.1 > zone.play_range.0 { zone.play_range } else { (0, wf.len as i64 - 1) };
    let lane_h = lanes.height() / 2.0;
    let px = rect.width().max(1.0) as usize;
    for (c, peaks) in wf.channels.iter().enumerate() {
        let mid = lanes.top() + lane_h * (c as f32 + 0.5);
        let h = lane_h * 0.45;
        let n = peaks.len();
        let cols: Vec<sampler_ui::WaveCol> = (0..px)
            .filter_map(|col| {
                let (a, b) = (col * n / px, ((col + 1) * n / px).max(col * n / px + 1).min(n));
                if a >= n {
                    return None;
                }
                let (mn, mx) = peaks[a..b].iter().fold((f32::MAX, f32::MIN), |(l, h), p| (l.min(p.0), h.max(p.1)));
                let frame = (col as f32 / px as f32 * len) as i64;
                let inside = (lo..=hi).contains(&frame);
                Some(sampler_ui::WaveCol {
                    x: rect.left() + col as f32 + 0.5,
                    top: mid - mx.clamp(-1.0, 1.0) * h,
                    bottom: mid - mn.clamp(-1.0, 1.0) * h,
                    color: if inside { color } else { color.gamma_multiply(0.35) },
                })
            })
            .collect();
        sampler_ui::paint_wave(&clip, &cols);
    }
    // The loop.
    if zone.loop_active && zone.loop_range.1 > zone.loop_range.0 {
        let r = Rect::from_min_max(Pos2::new(x_of(zone.loop_range.0), lanes.top()), Pos2::new(x_of(zone.loop_range.1), lanes.bottom()));
        clip.rect_filled(r, 0.0, Color32::from_white_alpha(14));
        for x in [r.left(), r.right()] {
            clip.vline(x, lanes.y_range(), Stroke::new(1.0, white(150)));
        }
    }
    // Start and end: white lines with their S and E.
    for (x, letter) in [(x_of(lo), "S"), (x_of(hi), "E")] {
        clip.vline(x, rect.top() + ruler_h..=rect.bottom() - 6.0, Stroke::new(1.5, Color32::WHITE));
        let c = Pos2::new(x.clamp(rect.left() + 8.0, rect.right() - 8.0), rect.bottom() - 9.0);
        clip.circle_filled(c, 8.0, Color32::WHITE);
        clip.text(c, Align2::CENTER_CENTER, letter, font(11.0), Color32::BLACK);
    }
    if let Some(f) = playhead {
        clip.vline(x_of(f as i64), lanes.y_range(), Stroke::new(1.0, Color32::WHITE));
        ui.ctx().request_repaint();
    }
    // Drag the nearest marker.
    let resp = ui.interact(rect, id.with("wave"), Sense::drag()).on_hover_text("Drag the S and E markers (and the loop's, when it's on)");
    let marker = id.with("marker");
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
    {
        let mut marks = vec![("play_range/low", x_of(lo)), ("play_range/high", x_of(hi))];
        if zone.loop_active {
            marks.push(("loop_range/low", x_of(zone.loop_range.0)));
            marks.push(("loop_range/high", x_of(zone.loop_range.1)));
        }
        let near = marks.into_iter().min_by(|a, b| (a.1 - p.x).abs().total_cmp(&(b.1 - p.x).abs())).filter(|m| (m.1 - p.x).abs() < 12.0);
        ui.data_mut(|d| d.insert_temp::<Option<&'static str>>(marker, near.map(|m| m.0)));
    }
    if resp.dragged()
        && let Some(Some(field)) = ui.data(|d| d.get_temp::<Option<&'static str>>(marker))
        && let Some(p) = resp.interact_pointer_pos()
    {
        let frame = (((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * (len - 1.0)).round() as f64;
        acts.push(Act::SetParam(target, field, frame));
    }
}

// ---- FX --------------------------------------------------------------------------

/// A knob from the Sampler's pages with Maschine's strip label.
fn relabel(k: Option<Knob>, label: &'static str) -> Option<Knob> {
    k.map(|(_, text, v, e)| (Cow::Borrowed(label), text, v, e))
}

#[allow(clippy::too_many_arguments)]
fn fx_page(
    ui: &mut Ui,
    painter: &Painter,
    area: Rect,
    smp: &Sampler,
    g: usize,
    s: usize,
    color: Color32,
    id: egui::Id,
    acts: &mut Vec<Act>,
) {
    let pitch = sampler_ui::page(smp, g, s, 1);
    let fx = sampler_ui::page(smp, g, s, 2);
    let st = Target::Sampler(g, s);
    let top = area.top();
    let sep = |painter: &Painter, x: f32| painter.vline(x, top + 2.0..=area.bottom() - 4.0, Stroke::new(1.0, LINE));
    let hf = heading_font(ui, 12.0);
    let title = |painter: &Painter, x: f32, text: &str| {
        painter.text(Pos2::new(x, top + 6.0), Align2::CENTER_CENTER, text, hf.clone(), HEAD);
    };
    // TUNE (130).
    let mut x = area.left();
    title(painter, x + 65.0, "TUNE");
    if let Some(k) = relabel(pitch.slots[0].clone(), "Tune") {
        knob_at(ui, painter, Pos2::new(x + 65.0, top + 72.0), 25.0, &k, id.with("tune"), color, acts);
    }
    x += 130.0;
    sep(painter, x);
    // SATURATION and LO-FI (102 each): two knobs stacked.
    for (n, (name, a, b)) in [("SATURATION", (0, "Comp"), (1, "Drive")), ("LO-FI", (2, "SR"), (3, "Bits"))].into_iter().enumerate() {
        title(painter, x + 51.0, name);
        for (row, (slot, label)) in [a, b].into_iter().enumerate() {
            if let Some(k) = relabel(fx.slots[slot].clone(), label) {
                knob_at(ui, painter, Pos2::new(x + 51.0, top + 42.0 + row as f32 * 74.0), 17.0, &k, id.with(("fx", n, row)), color, acts);
            }
        }
        x += 102.0;
        sep(painter, x);
    }
    // FILTER (230): mode, response, its knobs.
    let mode = (smp.get("filter").max(0.0) as usize).min(FILTER.len() - 1);
    painter.text(Pos2::new(x + 12.0, top + 6.0), Align2::LEFT_CENTER, "FILTER", hf.clone(), HEAD);
    if let Some(i) = cycler(ui, painter, Pos2::new(x + 230.0 - 50.0, top + 6.0), &FILTER, mode, id.with("filter-mode")) {
        acts.push(Act::SetParam(st, "filter", i as f64));
    }
    let disp = Rect::from_min_size(Pos2::new(x + 12.0, top + 20.0), Vec2::new(206.0, 70.0));
    filter_display(ui, painter, disp, smp, mode as i64, st, color, id.with("filter"), acts);
    let knobs: Vec<Option<Knob>> = match mode {
        1 | 3 => vec![relabel(fx.slots[5].clone(), "Cutoff"), relabel(fx.slots[6].clone(), "Res")],
        2 => vec![relabel(fx.slots[5].clone(), "Cutoff")],
        4 => vec![relabel(fx.slots[5].clone(), "Freq"), relabel(fx.slots[6].clone(), "Width"), relabel(fx.slots[7].clone(), "Gain")],
        _ => Vec::new(),
    };
    knob_row(ui, painter, disp.center().x, disp.bottom() + 25.0, &knobs, id.with("filter-knobs"), color, acts);
    x += 230.0;
    sep(painter, x);
    // AMP ENVELOPE (the rest, 303 in Maschine).
    let env = (smp.get("envelope_type").max(0.0) as usize).min(ENVELOPE.len() - 1);
    let w = area.right() - x;
    painter.text(Pos2::new(x + 12.0, top + 6.0), Align2::LEFT_CENTER, "AMP ENVELOPE", hf.clone(), HEAD);
    if let Some(i) = cycler(ui, painter, Pos2::new(x + w - 56.0, top + 6.0), &ENVELOPE, env, id.with("env-mode")) {
        acts.push(Act::SetParam(st, "envelope_type", i as f64));
    }
    let disp = Rect::from_min_size(Pos2::new(x + 12.0, top + 20.0), Vec2::new(w - 33.0, 70.0));
    envelope_display(ui, painter, disp, smp, env as i64, st, color, id.with("env"), acts);
    let k = &pitch.slots;
    let knobs: Vec<Option<Knob>> = match env {
        1 => vec![relabel(k[4].clone(), "Attack"), relabel(k[5].clone(), "Hold"), relabel(k[6].clone(), "Decay")],
        2 => vec![
            relabel(k[4].clone(), "Attack"),
            relabel(k[5].clone(), "Decay"),
            relabel(k[6].clone(), "Sustain"),
            relabel(k[7].clone(), "Release"),
        ],
        _ => Vec::new(),
    };
    knob_row(ui, painter, disp.center().x, disp.bottom() + 25.0, &knobs, id.with("env-knobs"), color, acts);
}

/// Knobs in a row centred on `cx` (24 px apart, as Maschine's Row).
#[allow(clippy::too_many_arguments)]
fn knob_row(ui: &mut Ui, painter: &Painter, cx: f32, cy: f32, knobs: &[Option<Knob>], id: egui::Id, color: Color32, acts: &mut Vec<Act>) {
    let pitch = 34.0 + 24.0;
    let x0 = cx - (knobs.len().saturating_sub(1)) as f32 * pitch / 2.0;
    for (n, k) in knobs.iter().enumerate() {
        if let Some(k) = k {
            knob_at(ui, painter, Pos2::new(x0 + n as f32 * pitch, cy), 17.0, k, id.with(n), color, acts);
        }
    }
}

/// "◂ name ▸" right-aligned at `right_center`: the arrows step through `options`; a click
/// on the name lists them.
fn cycler(ui: &mut Ui, painter: &Painter, center: Pos2, options: &[&str], cur: usize, id: egui::Id) -> Option<usize> {
    let name_r = Rect::from_center_size(center, Vec2::new(60.0, 16.0));
    let resp = ui.interact(name_r, id, Sense::click());
    painter.text(center, Align2::CENTER_CENTER, options[cur], font(12.0), if resp.hovered() { Color32::WHITE } else { HEAD });
    let mut pick = choice_list(&resp, options.iter().copied(), cur).filter(|&i| i != cur);
    for (dir, x) in [(-1i64, name_r.left() - 4.0), (1, name_r.right() + 4.0)] {
        let b = Rect::from_center_size(Pos2::new(x, center.y), Vec2::new(12.0, 16.0));
        let next = cur as i64 + dir;
        let ok = (0..options.len() as i64).contains(&next);
        let r = ui.interact(b, id.with(dir), Sense::click());
        let c = b.center();
        let pts = if dir < 0 {
            vec![c + Vec2::new(3.0, -4.0), c + Vec2::new(3.0, 4.0), c + Vec2::new(-3.0, 0.0)]
        } else {
            vec![c + Vec2::new(-3.0, -4.0), c + Vec2::new(-3.0, 4.0), c + Vec2::new(3.0, 0.0)]
        };
        let ink = if !ok {
            white(40)
        } else if r.hovered() {
            Color32::WHITE
        } else {
            HEAD
        };
        painter.add(egui::Shape::convex_polygon(pts, ink, Stroke::NONE));
        if ok && r.clicked() {
            pick = Some(next as usize);
        }
    }
    pick
}

/// A curve (points x, y in 0..1, y up) filled under, in `r`.
fn curve(painter: &Painter, r: Rect, pts: &[(f32, f32)], color: Color32) {
    let to = |(x, y): (f32, f32)| Pos2::new(r.left() + x * r.width(), (r.bottom() - y * r.height()).clamp(r.top() + 1.0, r.bottom() - 1.0));
    let line: Vec<Pos2> = pts.iter().copied().map(to).collect();
    // The fill, as vertical strips under the line.
    let mut mesh = egui::Mesh::default();
    let fill = color.gamma_multiply(0.2);
    for (i, p) in line.iter().enumerate() {
        mesh.colored_vertex(*p, fill);
        mesh.colored_vertex(Pos2::new(p.x, r.bottom()), fill);
        if i > 0 {
            let (a, b) = ((i as u32 - 1) * 2, i as u32 * 2);
            mesh.add_triangle(a, a + 1, b);
            mesh.add_triangle(b, a + 1, b + 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.add(egui::Shape::line(line, Stroke::new(2.0, color)));
}

/// A display's handle (accent ring, black, accent dot).
fn handle(painter: &Painter, c: Pos2, color: Color32) {
    painter.circle_filled(c, 8.0, color);
    painter.circle_filled(c, 6.0, Color32::BLACK);
    painter.circle_filled(c, 2.8, color);
}

/// The filter's response (from the engine's own coefficients) with a handle at its
/// cutoff and resonance (the EQ: frequency and gain); drag it.
#[allow(clippy::too_many_arguments)]
fn filter_display(
    ui: &mut Ui,
    painter: &Painter,
    r: Rect,
    smp: &Sampler,
    mode: i64,
    st: Target,
    color: Color32,
    id: egui::Id,
    acts: &mut Vec<Act>,
) {
    painter.rect_filled(r, 2.0, DISPLAY);
    let clip = painter.with_clip_rect(r);
    if mode == 0 {
        curve(&clip, r, &[(0.0, 0.5), (1.0, 0.5)], color);
        return;
    }
    let eq = (smp.get("eq_frequency"), smp.get("eq_width"), smp.get("eq_gain"));
    let range = if mode == 4 { 80.0 } else { 60.0 };
    let pts: Vec<(f32, f32)> = dsp::filter_response(mode, smp.get("cutoff"), smp.get("resonance"), eq, 160)
        .into_iter()
        .map(|(x, db)| (x, 0.5 + db / range))
        .collect();
    curve(&clip, r, &pts, color);
    let (hx, hy) = match mode {
        1 | 3 => (smp.get("cutoff"), smp.get("resonance")),
        2 => (smp.get("cutoff"), 0.5),
        _ => (eq.0, (eq.2 + 1.0) / 2.0),
    };
    let at = Pos2::new(r.left() + hx as f32 * r.width(), r.bottom() - hy as f32 * r.height());
    handle(painter, at, color);
    let resp = ui.interact(r, id, Sense::click_and_drag()).on_hover_text("Drag: cutoff (left/right) and resonance (up/down)");
    if (resp.dragged() || resp.drag_started())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let nx = ((p.x - r.left()) / r.width()).clamp(0.0, 1.0) as f64;
        let ny = ((r.bottom() - p.y) / r.height()).clamp(0.0, 1.0) as f64;
        match mode {
            1 | 3 => {
                acts.push(Act::SetParam(st, "cutoff", nx));
                acts.push(Act::SetParam(st, "resonance", ny));
            }
            2 => acts.push(Act::SetParam(st, "cutoff", nx)),
            _ => {
                acts.push(Act::SetParam(st, "eq_frequency", nx));
                acts.push(Act::SetParam(st, "eq_gain", ny * 2.0 - 1.0));
            }
        }
    }
}

/// The amplitude envelope's points as Maschine draws them (`getAmpEnvelopeResponse`):
/// AHD over three equal spans; ADSR with attack and decay a quarter each, release
/// from the end over up to a quarter, sustain between; one-shot a flat line.
pub fn envelope_points(mode: i64, a: f64, h: f64, d: f64, s: f64, r: f64) -> Vec<(f32, f32)> {
    let p = |x: f64, y: f64| (x as f32, y as f32);
    match mode {
        1 => vec![p(0.0, 0.0), p(a / 3.0, 1.0), p((a + h) / 3.0, 1.0), p((a + h + d) / 3.0, 0.0), p(1.0, 0.0)],
        2 => vec![p(0.0, 0.0), p(a / 4.0, 1.0), p((a + d) / 4.0, s), p(1.0 - r / 4.0, s), p(1.0, 0.0)],
        _ => vec![p(0.0, 0.5), p(1.0, 0.5)],
    }
}

#[allow(clippy::too_many_arguments)]
fn envelope_display(
    ui: &mut Ui,
    painter: &Painter,
    r: Rect,
    smp: &Sampler,
    mode: i64,
    st: Target,
    color: Color32,
    id: egui::Id,
    acts: &mut Vec<Act>,
) {
    painter.rect_filled(r, 2.0, DISPLAY);
    let inner = r.shrink(10.0);
    let v = |k: &str| smp.get(&format!("volume_envelope/{k}"));
    let (a, h, d, s, rel) = (v("attack"), v("hold"), v("decay"), v("sustain"), v("release"));
    let pts = envelope_points(mode, a, h, d, s, rel);
    curve(&painter.with_clip_rect(r), inner, &pts, color);
    if mode == 0 {
        return;
    }
    let to = |(x, y): (f32, f32)| Pos2::new(inner.left() + x * inner.width(), inner.bottom() - y * inner.height());
    let handles = [to(pts[1]), to(pts[2]), to(pts[3])];
    for c in handles {
        handle(painter, c, color);
    }
    // Drag a handle: which one is chosen on press.
    let resp = ui.interact(r, id, Sense::drag()).on_hover_text("Drag the handles");
    let which = id.with("which");
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
    {
        let near = handles
            .iter()
            .enumerate()
            .min_by(|x, y| x.1.distance(p).total_cmp(&y.1.distance(p)))
            .filter(|(_, c)| c.distance(p) < 18.0)
            .map(|(i, _)| i);
        ui.data_mut(|dd| dd.insert_temp::<Option<usize>>(which, near));
    }
    if resp.dragged()
        && let Some(Some(k)) = ui.data(|dd| dd.get_temp::<Option<usize>>(which))
        && let Some(p) = resp.interact_pointer_pos()
    {
        let gx = ((p.x - inner.left()) / inner.width()).clamp(0.0, 1.0) as f64;
        let gy = ((inner.bottom() - p.y) / inner.height()).clamp(0.0, 1.0) as f64;
        let set = |acts: &mut Vec<Act>, k: &'static str, v: f64| acts.push(Act::SetParam(st, k, v.clamp(0.0, 1.0)));
        // The later handles sit after the attack (and hold) as set, not as a route
        // shows them.
        let own = |k: &str, shown: f64| super::super::mod_routes::ui_base(ui, st, &format!("volume_envelope/{k}"), shown);
        let (a, h) = (own("attack", a), own("hold", h));
        match (mode, k) {
            (1, 0) => set(acts, "volume_envelope/attack", gx * 3.0),
            (1, 1) => set(acts, "volume_envelope/hold", gx * 3.0 - a),
            (1, 2) => set(acts, "volume_envelope/decay", gx * 3.0 - a - h),
            (2, 0) => set(acts, "volume_envelope/attack", gx * 4.0),
            (2, 1) => {
                set(acts, "volume_envelope/decay", gx * 4.0 - a);
                set(acts, "volume_envelope/sustain", gy);
            }
            (2, 2) => {
                set(acts, "volume_envelope/release", (1.0 - gx) * 4.0);
                set(acts, "volume_envelope/sustain", gy);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_points_follow_maschines_layout() {
        assert_eq!(envelope_points(1, 0.3, 0.3, 0.3, 0.0, 0.0)[3], (0.3, 0.0));
        let adsr = envelope_points(2, 1.0, 0.0, 1.0, 0.5, 1.0);
        assert_eq!(adsr[1], (0.25, 1.0));
        assert_eq!(adsr[2], (0.5, 0.5));
        assert_eq!(adsr[3], (0.75, 0.5));
        assert_eq!(envelope_points(0, 0.0, 0.0, 0.0, 0.0, 0.0), vec![(0.0, 0.5), (1.0, 0.5)]);
    }

    #[test]
    fn ruler_labels_stay_apart() {
        assert_eq!(ruler_step(1.8, 820.0), 0.5); // Maschine's 1.8 s kick: 0.0s 0.5s 1.0s 1.5s
        assert_eq!(ruler_step(0.05, 800.0), 0.025);
    }
}
