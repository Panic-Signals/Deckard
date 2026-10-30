//! The LFO panel (docs/superpowers/specs/2026-10-28-lfos-design.md, "Software"): the
//! project's LFOs listed on the left, the picked one's wave on the right, its controls
//! under that. It takes the Mixer's place.

use super::*;
use deckard_model::lfo::{Lfo, RATES, Shape, env_knob, env_time};
use deckard_model::mod_matrix::{CC_MAX, CcRoute, ModSource};

/// The list's width.
const LIST_W: f32 = 170.0;
/// The Mode cell's width (first in the controls row).
const MODE_W: f32 = 120.0;

/// The lowest pad (1-16) no LFO has; none when all 16 are made.
pub fn next_free_pad(lfos: &[Lfo]) -> Option<u8> {
    (1..=16).find(|p| !lfos.iter().any(|l| l.pad == *p))
}

/// The LFO the panel shows: the focused pad's, or the first when that one is gone.
pub fn focused(lfos: &[Lfo], focus: u8) -> Option<&Lfo> {
    lfos.iter().find(|l| l.pad == focus).or(lfos.first())
}

/// A sound as the trigger list names it: "A1 / S3 – name".
fn sound_label(p: &Project, g: usize, s: usize) -> String {
    let name = p.groups.get(g).and_then(|x| x.sounds.get(s)).map_or("", |x| x.name.as_str());
    format!("{} / S{} \u{2013} {name}", group_id(g), s + 1)
}

/// The controls row at the top of `ctl`: the plug-in strip's knobs and choices, Mode
/// first; an envelope's Attack, Decay, Sustain and Release in place of Shape, Sync, Rate
/// and Phase, and Retrigger in place of Fade in. `focus`: the focused sound (an
/// envelope's trigger when it's made with none). Gives the LFO as they leave it.
fn controls(ui: &mut Ui, painter: &Painter, p: &Project, lfo: &Lfo, ctl: Rect, focus: (usize, usize)) -> Lfo {
    let mut n = lfo.clone();
    let cy = ctl.top() + 28.0;
    let id = egui::Id::new(("lfo-controls", lfo.pad));
    let x = std::cell::Cell::new(ctl.left());
    // Each cell is `w` wide, its control centred in it.
    let cell = |w: f32| {
        x.set(x.get() + w);
        Pos2::new(x.get() - w / 2.0, cy)
    };
    let caption = |c: Pos2, t: &str| {
        painter.text(Pos2::new(c.x, c.y + 28.0), Align2::CENTER_CENTER, t, font(12.0), white(160));
    };
    let c = cell(MODE_W);
    if let Some(i) = strip::choice_pick(ui, painter, c, 70.0, &["LFO", "Envelope"], usize::from(n.env), id.with("mode")) {
        n.env = i == 1;
        // (An envelope needs a trigger: the focused sound's, if it has none.)
        if n.env && n.trigger.is_none() {
            n.trigger = Some(focus);
        }
    }
    caption(c, "Mode");
    let fresh = Lfo::new(n.pad);
    let turn = |ui: &mut Ui, w: f32, name: &str, text: String, amount: f32, key: &str| -> (Option<f32>, bool) {
        let c = cell(w);
        let resp = strip::knob_face(ui, painter, c, 16.0, name, &text, amount, false, id.with(key), PLAY_ON);
        (strip::turned(ui, &resp, id.with((key, "drag")), amount, 150.0), resp.double_clicked())
    };
    if lfo.env {
        // Attack, Decay, Release: 1 ms-10 s on an exponential knob.
        for (name, key) in [("Attack", "attack"), ("Decay", "decay"), ("Sustain", "sustain"), ("Release", "release")] {
            let (now, reset) = match key {
                "attack" => (&mut n.attack, fresh.attack),
                "decay" => (&mut n.decay, fresh.decay),
                "sustain" => (&mut n.sustain, fresh.sustain),
                _ => (&mut n.release, fresh.release),
            };
            if key == "sustain" {
                let (v, r) = turn(ui, 90.0, name, format!("{:.0} %", *now * 100.0), *now, key);
                if let Some(v) = v {
                    *now = (v * 100.0).round() / 100.0;
                }
                if r {
                    *now = reset;
                }
                continue;
            }
            let (v, r) = turn(ui, 90.0, name, crate::surface_lfo::env_time_text(*now), env_knob(*now), key);
            if let Some(v) = v {
                *now = env_time(v);
            }
            if r {
                *now = reset;
            }
        }
    } else {
        let shapes: Vec<&str> = Shape::ALL.iter().map(|s| s.name()).collect();
        let c = cell(130.0);
        let at = Shape::ALL.iter().position(|s| *s == n.shape).unwrap_or(0);
        if let Some(i) = strip::choice_pick(ui, painter, c, 70.0, &shapes, at, id.with("shape")) {
            n.shape = Shape::ALL[i];
        }
        caption(c, "Shape");
        let c = cell(110.0);
        if let Some(i) = strip::choice_pick(ui, painter, c, 40.0, &["Off", "On"], usize::from(n.sync), id.with("sync")) {
            n.sync = i == 1;
        }
        caption(c, "Sync");
        if n.sync {
            let c = cell(130.0);
            let names: Vec<&str> = RATES.iter().map(|(name, _)| *name).collect();
            if let Some(i) = strip::choice_pick(ui, painter, c, 70.0, &names, n.rate.min(RATES.len() - 1), id.with("rate")) {
                n.rate = i;
            }
            caption(c, "Rate");
        } else {
            // Hz: 0.01-20 on a log scale, so each octave takes the same turn.
            let span = (20.0f32 / 0.01).ln();
            let c = cell(90.0);
            let amount = (n.hz / 0.01).ln() / span;
            let resp = strip::knob_face(ui, painter, c, 16.0, "Rate", &format!("{:.2} Hz", n.hz), amount, false, id.with("hz"), PLAY_ON);
            if let Some(v) = strip::turned(ui, &resp, id.with("hz-drag"), amount, 150.0) {
                n.hz = (0.01 * (v * span).exp()).clamp(0.01, 20.0);
            }
            if resp.double_clicked() {
                n.hz = fresh.hz;
            }
        }
        let (v, reset) = turn(ui, 90.0, "Phase", format!("{:.0}\u{b0}", n.phase), n.phase / 360.0, "phase");
        if let Some(v) = v {
            n.phase = (v * 360.0).round();
        }
        if reset {
            n.phase = fresh.phase;
        }
    }
    let (v, reset) = turn(ui, 90.0, "Level", format!("{:.0} %", n.level * 100.0), n.level, "level");
    if let Some(v) = v {
        n.level = (v * 100.0).round() / 100.0;
    }
    if reset {
        n.level = fresh.level;
    }
    let text = if n.quantize < 2 { "Off".to_string() } else { format!("{} steps", n.quantize) };
    let (v, reset) = turn(ui, 90.0, "Quantize", text, f32::from(n.quantize) / 32.0, "quantize");
    if let Some(v) = v {
        // (One step per cycle is no quantizing: the knob rests at Off, then 2.)
        let q = (v * 32.0).round() as u8;
        n.quantize = if q < 2 { 0 } else { q };
    }
    if reset {
        n.quantize = fresh.quantize;
    }
    // Trigger: Off, then every sound; an envelope's has no Off ("None" when unset).
    let c = cell(230.0);
    let sounds: Vec<(usize, usize)> = p.groups.iter().enumerate().flat_map(|(g, grp)| (0..grp.sounds.len()).map(move |s| (g, s))).collect();
    let none = if lfo.env { "None" } else { "Off" };
    let labels: Vec<String> = std::iter::once(none.to_string()).chain(sounds.iter().map(|&(g, s)| sound_label(p, g, s))).collect();
    let names: Vec<&str> = labels.iter().map(String::as_str).collect();
    // (A trigger naming no sound here, as hand-edited text may, shows as unset.)
    let found = n.trigger.and_then(|t| sounds.iter().position(|x| *x == t));
    let set = lfo.env && found.is_some();
    let names = if set { &names[1..] } else { &names[..] };
    let at = if set { found.unwrap_or(0) } else { found.map_or(0, |i| i + 1) };
    if let Some(i) = strip::choice_pick(ui, painter, c, 170.0, names, at, id.with("trigger")) {
        let i = if set { Some(i) } else { i.checked_sub(1) };
        // (An envelope's "None" picked again stays None.)
        if let Some(i) = i {
            n.trigger = Some(sounds[i]);
        } else if !lfo.env {
            n.trigger = None;
        }
    }
    caption(c, "Trigger");
    if lfo.env {
        let c = cell(110.0);
        if let Some(i) = strip::choice_pick(ui, painter, c, 40.0, &["Off", "On"], usize::from(n.retrigger), id.with("retrigger")) {
            n.retrigger = i == 1;
        }
        caption(c, "Retrigger");
    } else if n.trigger.is_some() {
        let (v, reset) = turn(ui, 90.0, "Fade in", format!("{:.2} beats", n.fade), n.fade / 16.0, "fade");
        if let Some(v) = v {
            n.fade = (v * 16.0 * 4.0).round() / 4.0;
        }
        if reset {
            n.fade = fresh.fade;
        }
    }
    n
}

/// The MIDI CC section's heading and row heights, and its columns' widths.
const CC_TOP: f32 = 24.0;
const CC_ROW: f32 = 64.0;
const CC_SOURCE: f32 = 130.0;
const CC_PORT: f32 = 220.0;
const CC_CHANNEL: f32 = 90.0;
const CC_CC: f32 = 90.0;
const CC_DEPTH: f32 = 90.0;

/// The MIDI CC section in `area`: the project's CC routes (`routes`), one row each
/// (Source, Port, Channel, CC, Depth, and × to remove it), then "+ Add CC" (off with no
/// LFOs or no `ports`). An edit gives the whole list, as [`Act::SetCcRoutes`].
fn cc_section(ui: &mut Ui, painter: &Painter, lfos: &[Lfo], ports: &[String], routes: &[CcRoute], area: Rect, acts: &mut Vec<Act>) {
    let head = Rect::from_min_size(area.min, Vec2::new(area.width(), CC_TOP - 4.0));
    ui.put(head, egui::Label::new(egui::RichText::new("MIDI CC").size(11.0).color(white(140))));
    let mut list = routes.to_vec();
    let mut changed = false;
    let mut gone = None;
    for (i, r) in routes.iter().enumerate() {
        let id = egui::Id::new(("cc-route", i));
        let top = area.top() + CC_TOP + CC_ROW * i as f32;
        let cy = top + CC_ROW / 2.0 - 8.0;
        let x = std::cell::Cell::new(area.left());
        let cell = |w: f32| {
            x.set(x.get() + w);
            Pos2::new(x.get() - w / 2.0, cy)
        };
        let caption = |c: Pos2, t: &str| {
            painter.text(Pos2::new(c.x, c.y + 24.0), Align2::CENTER_CENTER, t, font(12.0), white(160));
        };
        let n = &mut list[i];
        // Source: the LFOs (one that's gone shown as such, last).
        let mut sources: Vec<ModSource> = lfos.iter().map(|l| ModSource::Lfo(l.pad)).collect();
        if !sources.contains(&r.source) {
            sources.push(r.source);
        }
        let names: Vec<String> = sources.iter().map(|s| mod_routes::lfo_name(lfos, *s)).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let c = cell(CC_SOURCE);
        let at = sources.iter().position(|s| *s == r.source).unwrap_or(0);
        if let Some(k) = strip::choice_pick(ui, painter, c, CC_SOURCE - 30.0, &names, at, id.with("source")) {
            n.source = sources[k];
        }
        caption(c, "Source");
        // Port: the output ports (one not there now kept, last).
        let mut names: Vec<&str> = ports.iter().map(String::as_str).collect();
        if !names.contains(&r.port.as_str()) {
            names.push(&r.port);
        }
        let c = cell(CC_PORT);
        let at = names.iter().position(|p| *p == r.port).unwrap_or(0);
        if let Some(k) = strip::choice_pick(ui, painter, c, CC_PORT - 30.0, &names, at, id.with("port")) {
            n.port = names[k].to_string();
        }
        caption(c, "Port");
        let channels: Vec<String> = (1..=16).map(|c| c.to_string()).collect();
        let channels: Vec<&str> = channels.iter().map(String::as_str).collect();
        let c = cell(CC_CHANNEL);
        if let Some(k) = strip::choice_pick(ui, painter, c, 40.0, &channels, usize::from(r.channel.clamp(1, 16) - 1), id.with("channel")) {
            n.channel = k as u8 + 1;
        }
        caption(c, "Channel");
        let c = cell(CC_CC);
        // (0-119: 120-127 are channel mode, not parameters.)
        let top = f32::from(CC_MAX);
        let amount = f32::from(r.cc) / top;
        let resp = strip::knob_face(ui, painter, c, 16.0, "CC", &r.cc.to_string(), amount, false, id.with("cc"), PLAY_ON);
        if let Some(v) = strip::turned(ui, &resp, id.with("cc-drag"), amount, 300.0) {
            n.cc = (v * top).round().clamp(0.0, top) as u8;
        }
        // Depth: High from Low, either way (centred at 0).
        let c = cell(CC_DEPTH);
        let depth = r.high - r.low;
        let amount = (depth + 1.0) / 2.0;
        let pct = (depth * 100.0).round() as i32;
        let text = if pct > 0 {
            format!("+{pct}%")
        } else if pct < 0 {
            format!("\u{2212}{}%", -pct)
        } else {
            "0%".into()
        };
        let resp = strip::knob_face(ui, painter, c, 16.0, "Depth", &text, amount, true, id.with("depth"), PLAY_ON);
        if let Some(v) = strip::turned(ui, &resp, id.with("depth-drag"), amount, 150.0) {
            n.high = ((r.low + v * 2.0 - 1.0).clamp(0.0, 1.0) * 100.0).round() / 100.0;
        }
        if resp.double_clicked() {
            n.high = n.low;
        }
        changed |= *n != *r;
        // ×: remove it.
        let c = cell(40.0);
        let x_rect = Rect::from_center_size(c, Vec2::splat(22.0));
        let resp = ui.interact(x_rect, id.with("remove"), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Remove CC route {}", i + 1)));
        painter.text(c, Align2::CENTER_CENTER, "\u{d7}", font(16.0), if resp.hovered() { white(230) } else { white(140) });
        if resp.on_hover_text("Remove").clicked() {
            gone = Some(i);
        }
    }
    if let Some(i) = gone {
        list.remove(i);
        changed = true;
    }
    let add = Rect::from_min_size(Pos2::new(area.left(), area.top() + CC_TOP + CC_ROW * routes.len() as f32), Vec2::new(90.0, 22.0));
    let first = lfos.first().zip(ports.first());
    let clicked = ui
        .scope_builder(UiBuilder::new().max_rect(add), |ui| ui.add_enabled(first.is_some(), egui::Button::new("+ Add CC")).clicked())
        .inner;
    if clicked && let Some((l, port)) = first {
        list.push(CcRoute::new(ModSource::Lfo(l.pad), port, 1, 1));
        changed = true;
    }
    if changed {
        acts.push(Act::SetCcRoutes(list));
    }
}

/// The LFO panel, in `r`.
pub fn lfo_panel(ui: &mut Ui, app: &App, d: &deckard_doc::Document, r: Rect, acts: &mut Vec<Act>) {
    let p = d.project();
    let painter = ui.painter_at(r);
    painter.rect_filled(r, 0.0, S00);
    let list = Rect::from_min_size(r.min, Vec2::new(LIST_W, r.height()));
    painter.rect_filled(list, 0.0, S05);
    painter.vline(list.right() - 0.5, list.y_range(), Stroke::new(1.0, Color32::BLACK));
    let mut side = ui.new_child(UiBuilder::new().max_rect(list.shrink(8.0)).layout(egui::Layout::top_down(egui::Align::Min)));
    side.label(egui::RichText::new("LFOs").size(11.0).color(white(140)));
    side.add_space(4.0);
    let shown = focused(&p.lfos, app.lfo_focus).map(|l| l.pad);
    for l in &p.lfos {
        // The name (click: show it) and its M (mute), like the Mixer's.
        side.horizontal(|ui| {
            let name = egui::RichText::new(l.name());
            let name = if l.muted { name.color(white(110)) } else { name };
            let resp = ui.add_sized(Vec2::new(list.width() - 16.0 - 34.0, 22.0), egui::Button::new(name).selected(shown == Some(l.pad)));
            if resp.clicked() {
                acts.push(Act::FocusLfo(l.pad));
            }
            resp.context_menu(|ui| {
                if ui.button("Remove").clicked() {
                    acts.push(Act::RemoveLfo(l.pad));
                    ui.close();
                }
            });
            let (m, mr) = ui.allocate_exact_size(Vec2::new(26.0, 16.0), Sense::click());
            mr.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Mute {}", l.name())));
            mixer::mute_face(ui.painter(), m, l.muted, mr.hovered());
            if mr.on_hover_text("Mute").clicked() {
                acts.push(Act::SetLfo(Lfo { muted: !l.muted, ..l.clone() }));
            }
        });
    }
    if let Some(pad) = next_free_pad(&p.lfos) {
        side.add_space(4.0);
        if side.button("+ Add LFO").clicked() {
            acts.push(Act::SetLfo(Lfo::new(pad)));
            acts.push(Act::FocusLfo(pad));
        }
    }
    let body = Rect::from_min_max(Pos2::new(list.right() + 14.0, r.top() + 12.0), Pos2::new(r.right() - 14.0, r.bottom() - 8.0));
    let pic = Rect::from_min_size(body.min, Vec2::new(body.width(), (body.height() * 0.4).clamp(70.0, 180.0)));
    // The MIDI CC section, under the controls.
    let cc = Rect::from_min_max(Pos2::new(body.left(), pic.bottom() + 100.0), body.max);
    cc_section(ui, &painter, &p.lfos, &crate::mod_slots::out_ports(app), &p.cc_routes, cc, acts);
    let Some(lfo) = focused(&p.lfos, app.lfo_focus) else {
        painter.text(body.left_top(), Align2::LEFT_TOP, "No LFOs yet: add one.", font(12.0), white(110));
        return;
    };
    // The wave.
    painter.rect_filled(pic, 3.0, S01);
    painter.hline(pic.x_range(), pic.center().y, Stroke::new(1.0, white(20)));
    let pts: Vec<Pos2> = lfo
        .picture(480)
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = pic.left() + 6.0 + (pic.width() - 12.0) * i as f32 / 479.0;
            Pos2::new(x, pic.bottom() - 8.0 - (pic.height() - 16.0) * v.clamp(0.0, 1.0))
        })
        .collect();
    // (Muted: dimmed, its shape still shown.)
    let ink = if lfo.muted { PLAY_ON.gamma_multiply(0.3) } else { PLAY_ON };
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, ink)));
    // An envelope's note off: a dashed line where the release begins.
    if let Some(off) = lfo.picture_note_off() {
        let x = pic.left() + 6.0 + (pic.width() - 12.0) * off;
        let line = [Pos2::new(x, pic.top() + 4.0), Pos2::new(x, pic.bottom() - 4.0)];
        painter.extend(egui::Shape::dashed_line(&line, Stroke::new(1.0, white(70)), 4.0, 4.0));
        painter.text(Pos2::new(x + 4.0, pic.top() + 6.0), Align2::LEFT_TOP, "note off", font(11.0), white(90));
    }
    // The controls.
    let ctl = Rect::from_min_max(Pos2::new(body.left(), pic.bottom() + 12.0), body.max);
    let n = controls(ui, &painter, p, lfo, ctl, (app.group, app.sound));
    if n != *lfo {
        acts.push(Act::SetLfo(n));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::mod_matrix::{ModSource, choices};

    fn app() -> crate::App {
        let mut app = crate::App::default();
        app.new_project();
        app
    }

    #[test]
    fn toggling_lfos_hides_the_mixer() {
        let mut app = app();
        apply_acts(&mut app, vec![Act::ToggleMixer]);
        assert!(app.mixer && !app.lfos);
        apply_acts(&mut app, vec![Act::ToggleLfos]);
        assert!(app.lfos && !app.mixer);
        apply_acts(&mut app, vec![Act::ToggleMixer]);
        assert!(app.mixer && !app.lfos);
    }

    #[test]
    fn add_lfo_takes_the_lowest_free_pad() {
        let l = |p| Lfo::new(p);
        assert_eq!(next_free_pad(&[]), Some(1));
        assert_eq!(next_free_pad(&[l(1), l(3)]), Some(2));
        let all: Vec<Lfo> = (1..=16).map(l).collect();
        assert_eq!(next_free_pad(&all), None);
    }

    #[test]
    fn the_picker_lists_existing_lfos() {
        let mut app = app();
        apply_acts(&mut app, vec![Act::SetLfo(Lfo::new(4))]);
        let doc = app.doc.clone().unwrap();
        assert!(choices(&doc.borrow().project().lfos).contains(&ModSource::Lfo(4)));
        apply_acts(&mut app, vec![Act::FocusLfo(4), Act::RemoveLfo(4)]);
        assert!(doc.borrow().project().lfos.is_empty());
    }

    /// Removing the focused LFO moves the focus to the first one left (0 when none).
    #[test]
    fn removing_the_focused_lfo_moves_the_focus() {
        let mut app = app();
        apply_acts(&mut app, vec![Act::SetLfo(Lfo::new(2)), Act::SetLfo(Lfo::new(5)), Act::FocusLfo(5)]);
        apply_acts(&mut app, vec![Act::RemoveLfo(2)]);
        assert_eq!(app.lfo_focus, 5, "another one's removal leaves it");
        apply_acts(&mut app, vec![Act::SetLfo(Lfo::new(3)), Act::RemoveLfo(5)]);
        assert_eq!(app.lfo_focus, 3);
        apply_acts(&mut app, vec![Act::RemoveLfo(3)]);
        assert_eq!(app.lfo_focus, 0);
    }

    /// The panel drawn in a window of its own project: empty, with a triggered LFO, and with all 16 (no "+ Add LFO").
    #[test]
    fn the_panel_draws() {
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1500.0, 930.0))
            .build_ui_state(|ui, app: &mut crate::App| app.frame(ui), app());
        h.state_mut().lfos = true;
        h.run_steps(3);
        assert!(h.query_by_label("+ Add LFO").is_some());
        let mut lfo = Lfo::new(2);
        lfo.trigger = Some((0, 0));
        h.state().doc.clone().unwrap().borrow_mut().set_lfo(lfo).unwrap();
        h.state_mut().lfo_focus = 2;
        h.run_steps(3);
        assert!(h.query_by_label("LFO 2").is_some());
        for pad in 1..=16 {
            h.state().doc.clone().unwrap().borrow_mut().set_lfo(Lfo::new(pad)).unwrap();
        }
        h.run_steps(3);
        assert!(h.query_by_label("LFO 16").is_some());
        assert!(h.query_by_label("+ Add LFO").is_none(), "none to add at 16");
    }

    /// A row's M mutes and unmutes its LFO, and leaves the focus where it is.
    #[test]
    fn the_m_button_mutes_without_focusing() {
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1500.0, 930.0))
            .build_ui_state(|ui, app: &mut crate::App| app.frame(ui), app());
        h.state_mut().lfos = true;
        let doc = h.state().doc.clone().unwrap();
        doc.borrow_mut().set_lfo(Lfo::new(1)).unwrap();
        doc.borrow_mut().set_lfo(Lfo::new(2)).unwrap();
        h.state_mut().lfo_focus = 1;
        h.run_steps(3);
        h.get_by_label("Mute LFO 2").click();
        h.run_steps(3);
        assert!(doc.borrow().project().lfos[1].muted);
        assert!(!doc.borrow().project().lfos[0].muted);
        assert_eq!(h.state().lfo_focus, 1, "the focus stays");
        h.get_by_label("Mute LFO 2").click();
        h.run_steps(3);
        assert!(!doc.borrow().project().lfos[1].muted);
    }

    /// The controls row alone, at a known place: the knobs' names are painted, Fade in
    /// only for a triggered LFO, and dragging the Phase knob up turns the phase.
    #[test]
    fn the_knobs_are_drawn_and_turn() {
        struct St {
            app: crate::App,
            lfo: Lfo,
            origin: Pos2,
            text: String,
        }
        let mut lfo = Lfo::new(1);
        lfo.trigger = Some((0, 0));
        let st = St { app: app(), lfo, origin: Pos2::ZERO, text: String::new() };
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 200.0)).build_ui_state(
            |ui, st: &mut St| {
                st.origin = ui.max_rect().min;
                let ctl = Rect::from_min_size(st.origin, Vec2::new(1100.0, 120.0));
                let painter = ui.painter().clone();
                let doc = st.app.doc.clone().unwrap();
                st.lfo = controls(ui, &painter, doc.borrow().project(), &st.lfo, ctl, (0, 0));
                st.text = ui.ctx().graphics(|g| {
                    let mut all = String::new();
                    for c in g.get(ui.layer_id()).into_iter().flat_map(|l| l.all_entries()) {
                        if let egui::Shape::Text(t) = &c.shape {
                            all += &format!("{}|", t.galley.text());
                        }
                    }
                    all
                });
            },
            st,
        );
        h.run_steps(2);
        for name in ["Shape", "Sync", "Rate", "Phase", "Level", "Quantize", "Trigger", "Fade"] {
            assert!(h.state().text.to_lowercase().contains(&name.to_lowercase()), "{name} in {}", h.state().text);
        }
        h.state_mut().lfo.trigger = None;
        h.run_steps(2);
        assert!(!h.state().text.contains("Fade in"), "no fade without a trigger: {}", h.state().text);
        // Phase: the fifth cell (Mode, 130 + 110 + 130 wide, then half of 90), 28 down.
        let at = h.state().origin + Vec2::new(MODE_W + 130.0 + 110.0 + 130.0 + 45.0, 28.0);
        h.hover_at(at);
        h.run_steps(1);
        h.drag_at(at);
        h.run_steps(1);
        for k in 1..=6 {
            h.hover_at(at - Vec2::new(0.0, 5.0 * k as f32));
            h.run_steps(1);
        }
        h.drop_at(at - Vec2::new(0.0, 30.0));
        h.run_steps(2);
        let phase = h.state().lfo.phase;
        assert!(phase > 30.0 && phase <= 90.0, "30px up is about 72 degrees, got {phase}");
    }

    /// The controls row in Envelope mode: Mode, Attack, Decay, Sustain, Release, Level,
    /// Quantize, Trigger, Retrigger, no Shape or Phase; in LFO mode, Phase again. The
    /// list names it "ENV n".
    #[test]
    fn the_envelope_controls_are_drawn() {
        struct St {
            app: crate::App,
            lfo: Lfo,
            text: String,
        }
        let lfo = Lfo { env: true, trigger: Some((0, 0)), ..Lfo::new(1) };
        let st = St { app: app(), lfo, text: String::new() };
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 200.0)).build_ui_state(
            |ui, st: &mut St| {
                let ctl = Rect::from_min_size(ui.max_rect().min, Vec2::new(1350.0, 120.0));
                let painter = ui.painter().clone();
                let doc = st.app.doc.clone().unwrap();
                st.lfo = controls(ui, &painter, doc.borrow().project(), &st.lfo, ctl, (0, 0));
                st.text = ui.ctx().graphics(|g| {
                    let mut all = String::new();
                    for c in g.get(ui.layer_id()).into_iter().flat_map(|l| l.all_entries()) {
                        if let egui::Shape::Text(t) = &c.shape {
                            all += &format!("{}|", t.galley.text());
                        }
                    }
                    all
                });
            },
            st,
        );
        h.run_steps(2);
        let text = h.state().text.clone();
        for name in ["Mode", "Attack", "Decay", "Sustain", "Release", "Level", "Quantize", "Trigger", "Retrigger"] {
            assert!(text.contains(name), "{name} in {text}");
        }
        for name in ["Phase", "Shape", "Fade"] {
            assert!(!text.contains(name), "{name} in {text}");
        }
        // A trigger naming no sound (hand-edited text): "None".
        h.state_mut().lfo.trigger = Some((9, 9));
        h.run_steps(2);
        assert!(h.state().text.contains("None"), "{}", h.state().text);
        h.state_mut().lfo.env = false;
        h.run_steps(2);
        assert!(h.state().text.contains("Phase") && !h.state().text.contains("Attack"), "{}", h.state().text);
    }

    /// In the app, an envelope is listed as "ENV n".
    #[test]
    fn the_list_names_an_envelope() {
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1500.0, 930.0))
            .build_ui_state(|ui, app: &mut crate::App| app.frame(ui), app());
        h.state_mut().lfos = true;
        let doc = h.state().doc.clone().unwrap();
        doc.borrow_mut().set_lfo(Lfo { env: true, trigger: Some((0, 0)), ..Lfo::new(2) }).unwrap();
        h.state_mut().lfo_focus = 2;
        h.run_steps(3);
        assert!(h.query_by_label("ENV 2").is_some());
        assert!(h.query_by_label("Mute ENV 2").is_some());
    }

    /// The MIDI CC section alone: "+ Add CC" adds a route (the first LFO and port,
    /// channel 1, CC 1, Depth 0); a row's controls edit it; its × removes it. No LFOs or
    /// no ports: nothing to add.
    #[test]
    fn the_cc_section_adds_edits_and_removes() {
        use deckard_model::mod_matrix::{CcRoute, ModSource};
        use egui_kittest::kittest::Queryable;
        struct St {
            lfos: Vec<Lfo>,
            ports: Vec<String>,
            routes: Vec<CcRoute>,
            origin: Pos2,
            acts: Vec<Act>,
        }
        let st = St {
            lfos: vec![Lfo::new(3)],
            ports: vec!["Port A".into(), "Port B".into()],
            routes: Vec::new(),
            origin: Pos2::ZERO,
            acts: Vec::new(),
        };
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 300.0)).build_ui_state(
            |ui, st: &mut St| {
                st.origin = ui.max_rect().min;
                let area = Rect::from_min_size(st.origin, Vec2::new(880.0, 280.0));
                let painter = ui.painter().clone();
                cc_section(ui, &painter, &st.lfos, &st.ports, &st.routes, area, &mut st.acts);
            },
            st,
        );
        h.run_steps(2);
        h.get_by_label("+ Add CC").click();
        h.run_steps(2);
        let made = CcRoute::new(ModSource::Lfo(3), "Port A", 1, 1);
        assert!(matches!(&h.state().acts[..], [Act::SetCcRoutes(r)] if *r == vec![made.clone()]), "Add");
        h.state_mut().acts.clear();
        h.state_mut().routes = vec![made.clone()];
        h.run_steps(2);
        // Depth: the row's last knob, dragged up.
        let at = h.state().origin + Vec2::new(CC_SOURCE + CC_PORT + CC_CHANNEL + CC_CC + CC_DEPTH / 2.0, CC_TOP + CC_ROW / 2.0);
        h.hover_at(at);
        h.run_steps(1);
        h.drag_at(at);
        h.run_steps(1);
        for k in 1..=6 {
            h.hover_at(at - Vec2::new(0.0, 5.0 * k as f32));
            h.run_steps(1);
        }
        h.drop_at(at - Vec2::new(0.0, 30.0));
        h.run_steps(2);
        let Some(Act::SetCcRoutes(r)) = h.state().acts.last() else { panic!("{} acts", h.state().acts.len()) };
        assert!(r[0].high > 0.1 && r[0].low == 0.0, "{r:?}");
        h.state_mut().acts.clear();
        // CC: dragged far up, it stops at 119 (120-127 are channel mode).
        h.state_mut().routes = vec![CcRoute { cc: 119, ..made.clone() }];
        h.run_steps(2);
        let at = h.state().origin + Vec2::new(CC_SOURCE + CC_PORT + CC_CHANNEL + CC_CC / 2.0, CC_TOP + CC_ROW / 2.0);
        h.hover_at(at);
        h.run_steps(1);
        h.drag_at(at);
        h.run_steps(1);
        for k in 1..=6 {
            h.hover_at(at - Vec2::new(0.0, 20.0 * k as f32));
            h.run_steps(1);
        }
        h.drop_at(at - Vec2::new(0.0, 120.0));
        h.run_steps(2);
        let ccs: Vec<u8> = h.state().acts.iter().filter_map(|a| if let Act::SetCcRoutes(r) = a { Some(r[0].cc) } else { None }).collect();
        assert!(ccs.iter().all(|c| *c <= 119), "{ccs:?}");
        h.state_mut().routes = vec![made.clone()];
        h.run_steps(2);
        h.state_mut().acts.clear();
        h.get_by_label("Remove CC route 1").click();
        h.run_steps(2);
        assert!(matches!(&h.state().acts[..], [Act::SetCcRoutes(r)] if r.is_empty()), "×");
        h.state_mut().acts.clear();
        h.state_mut().ports.clear();
        h.run_steps(2);
        h.get_by_label("+ Add CC").click();
        h.run_steps(2);
        assert!(h.state().acts.is_empty(), "no ports: nothing to add");
    }

    /// In the app: the panel shows the section, and its Add goes through to the doc.
    #[test]
    fn add_cc_in_the_panel() {
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1500.0, 930.0))
            .build_ui_state(|ui, app: &mut crate::App| app.frame(ui), app());
        h.state_mut().lfos = true;
        let doc = h.state().doc.clone().unwrap();
        doc.borrow_mut().set_lfo(Lfo::new(1)).unwrap();
        h.state_mut().midi_ports.1 = vec!["Port A".into()];
        h.state_mut().midi_ports_at = Some(std::time::Instant::now());
        h.run_steps(3);
        assert!(h.query_by_label("MIDI CC").is_some());
        h.get_by_label("+ Add CC").click();
        h.run_steps(3);
        assert_eq!(doc.borrow().project().cc_routes.len(), 1);
        assert_eq!(doc.borrow().project().cc_routes[0].port, "Port A");
    }
}
