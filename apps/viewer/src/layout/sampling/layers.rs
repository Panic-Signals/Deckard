//! The LAYERS tab: a New Sampler sound's layers as lanes (a sample each, its envelope
//! drawn over it), the morph strip with every layer's window above them, and the
//! focused layer's and the morph's settings below.

use super::*;
use deckard_model::layers::{FREE, Fade, Layer, MAX_LAYERS, Morph, SYNCED, Shape, TIME_MAX, window_gain};

/// A layer's colour, by its lane.
pub const LAYER_COLOURS: [Color32; MAX_LAYERS] = [
    Color32::from_rgb(0xf2, 0xa2, 0x3c),
    Color32::from_rgb(0xe8, 0x58, 0x8a),
    Color32::from_rgb(0x5a, 0x9c, 0xf2),
    Color32::from_rgb(0x62, 0xc7, 0x8a),
    Color32::from_rgb(0xb0, 0x7c, 0xf0),
    Color32::from_rgb(0x4c, 0xc9, 0xd6),
    Color32::from_rgb(0xe6, 0xd2, 0x4a),
    Color32::from_rgb(0xf0, 0x74, 0x52),
];

/// Where the lanes are this frame (a file dropped on one lands there).
pub const LANE_RECTS: &str = "deckard-layer-lanes";

/// A time as shown: "5 ms", "1.50 s".
pub fn time_text(seconds: f64) -> String {
    if seconds < 1.0 { format!("{:.0} ms", seconds * 1000.0) } else { format!("{seconds:.2} s") }
}

/// A time `n` drag steps on: finer the shorter it is.
pub fn time_stepped(seconds: f64, n: f64) -> f64 {
    (seconds + n * (seconds * 0.02).max(0.001)).clamp(0.0, TIME_MAX)
}

/// A synced cycle as shown: "1/16", "8 bars".
pub fn bars_text(bars: f64) -> String {
    if bars < 1.0 {
        format!("1/{:.0}", 1.0 / bars)
    } else if bars == 1.0 {
        "1 bar".into()
    } else {
        format!("{bars:.0} bars")
    }
}

/// The morph's rate as shown.
pub fn rate_text(m: &Morph) -> String {
    if m.sync {
        bars_text(m.bars)
    } else if m.seconds < 10.0 {
        format!("{:.2} s", m.seconds)
    } else {
        format!("{:.0} s", m.seconds)
    }
}

/// The morph's rate `n` steps on: the next synced cycle, or the free one a little longer
/// or shorter.
pub fn rate_stepped(m: &Morph, n: f64) -> MorphAct {
    if m.sync {
        let at = SYNCED.iter().position(|b| *b >= m.bars).unwrap_or(SYNCED.len() - 1) as f64;
        let step = if n > 0.0 { n.ceil() } else { n.floor() };
        MorphAct::Bars(SYNCED[(at + step).clamp(0.0, (SYNCED.len() - 1) as f64) as usize])
    } else {
        MorphAct::Seconds((m.seconds * 1.03f64.powf(n)).clamp(FREE.0, FREE.1))
    }
}

/// The morph's rate as typed: a synced cycle ("1/8", "4", "4 bars": the nearest there
/// is), or the free one in seconds.
pub fn rate_typed(m: &Morph, typed: &str) -> Option<MorphAct> {
    if !m.sync {
        return value_entry::time(typed, "s", false).filter(|s| *s > 0.0).map(|s| MorphAct::Seconds(s.clamp(FREE.0, FREE.1)));
    }
    let t = typed.trim().to_ascii_lowercase();
    let t = t.trim_end_matches("bars").trim_end_matches("bar").trim();
    let bars = match t.split_once('/') {
        Some((a, b)) => a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok()?,
        None => t.parse::<f64>().ok()?,
    };
    if !(bars.is_finite() && bars > 0.0) {
        return None;
    }
    let far = |b: &f64| (b / bars).ln().abs();
    SYNCED.iter().copied().min_by(|a, b| far(a).total_cmp(&far(b))).map(MorphAct::Bars)
}

/// A zone's gain as stored, in dB.
pub fn level_text(gain: f64) -> String {
    let lin = 2.3948107 * gain.powi(5) + 0.767467 * gain * gain;
    let db = 20.0 * lin.max(1e-12).log10();
    if lin < 1e-6 {
        "-inf dB".into()
    } else if db.abs() < 0.05 {
        "0.0 dB".into()
    } else {
        format!("{db:.1} dB")
    }
}

fn lact(lane: usize, a: LayerAct) -> Act {
    sact(SamplingAct::Layer(lane, a))
}

fn mact(a: MorphAct) -> Act {
    sact(SamplingAct::Morph(a))
}

/// The LAYERS tab in `body`.
pub(super) fn layers_tab(ui: &mut Ui, app: &App, p: &Project, body: Rect, gcol: Color32, acts: &mut Vec<Act>) {
    let painter = ui.painter_at(body);
    let (g, s) = (app.group, app.sound);
    let layers = crate::layers::layers_in(p, g, s);
    let Some(snd) = p.groups.get(g).and_then(|x| x.sounds.get(s)) else { return };
    let morph = snd.new_sampler.map(|n| n.morph).unwrap_or_default();
    let zones = snd.sampler.as_ref().map(|x| x.zones.as_slice()).unwrap_or_default();
    let focus = app.zone.min(layers.len().saturating_sub(1));
    let st = &app.sampling;
    let footer_y = body.bottom() - 60.0;

    // The morph strip: every layer's window, and where the morph is.
    let strip = Rect::from_min_max(Pos2::new(body.left() + 12.0, body.top() + 8.0), Pos2::new(body.right() - 12.0, body.top() + 72.0));
    painter.rect_filled(strip, 2.0, s01());
    painter.text(Pos2::new(strip.left() + 6.0, strip.top() + 9.0), Align2::LEFT_CENTER, "MORPH", font(10.0), white(142));
    let plot = Rect::from_min_max(Pos2::new(strip.left() + 6.0, strip.top() + 18.0), Pos2::new(strip.right() - 6.0, strip.bottom() - 4.0));
    let px = |v: f64| plot.left() + v as f32 * plot.width();
    let dim = if morph.on { 1.0 } else { 0.35 };
    let clip = ui.painter_at(plot);
    for (k, (_, l)) in layers.iter().enumerate() {
        let col = LAYER_COLOURS[k].gamma_multiply(dim);
        let w = l.window;
        let shifts: &[f64] = if morph.shape == Shape::Round { &[-1.0, 0.0, 1.0] } else { &[0.0] };
        for &o in shifts {
            let pts = vec![
                Pos2::new(px(w[0] + o), plot.bottom()),
                Pos2::new(px(w[1] + o), plot.top()),
                Pos2::new(px(w[2] + o), plot.top()),
                Pos2::new(px(w[3] + o), plot.bottom()),
            ];
            clip.add(egui::Shape::convex_polygon(pts.clone(), col.gamma_multiply(if k == focus { 0.45 } else { 0.2 }), Stroke::NONE));
            clip.add(egui::Shape::line(pts, Stroke::new(if k == focus { 2.0 } else { 1.0 }, col)));
        }
    }
    // The focused layer's four points, dragged sideways.
    if let Some((_, l)) = layers.get(focus) {
        for point in 0..4 {
            let v = l.window[point];
            let shown = if morph.shape == Shape::Round { v.rem_euclid(1.0) } else { v.clamp(0.0, 1.0) };
            let y = if point == 1 || point == 2 { plot.top() + 4.0 } else { plot.bottom() - 4.0 };
            let c = Pos2::new(px(shown), y);
            let r = Rect::from_center_size(c, Vec2::splat(14.0));
            ui.data_mut(|d| d.insert_temp(egui::Id::new(("deckard-layer-window", point)), r));
            let resp = ui
                .interact(r, ui.id().with(("layer-window", point)), Sense::drag())
                .on_hover_text("Drag: where this layer fades in and out");
            painter.circle_filled(c, if resp.hovered() || resp.dragged() { 5.0 } else { 3.5 }, LAYER_COLOURS[focus]);
            painter.circle_stroke(c, 5.0, Stroke::new(1.0, Color32::BLACK));
            if resp.dragged()
                && let Some(at) = resp.interact_pointer_pos()
            {
                let at = f64::from(((at.x - plot.left()) / plot.width()).clamp(0.0, 1.0));
                // (Round: the place nearest where the point was, a turn either way.)
                let to = if morph.shape == Shape::Round { at + (v - at).round() } else { at };
                if (to - v).abs() > 1e-4 {
                    acts.push(lact(focus, LayerAct::Window(point, to)));
                }
            }
        }
    }
    let at = st.morph_at.map(f64::from).filter(|_| morph.on).unwrap_or(morph.position);
    painter.vline(px(at.clamp(0.0, 1.0)), plot.y_range(), Stroke::new(1.5, white(if morph.on { 245 } else { 110 })));

    // The lanes.
    let top = strip.bottom() + 8.0;
    let lane_h = ((footer_y - 8.0 - top) / MAX_LAYERS as f32).clamp(18.0, 44.0);
    let mut rects = Vec::with_capacity(MAX_LAYERS);
    let audible = deckard_model::layers::audible(&layers.iter().map(|(_, l)| *l).collect::<Vec<Layer>>());
    for k in 0..MAX_LAYERS {
        let r = Rect::from_min_size(Pos2::new(body.left() + 12.0, top + k as f32 * lane_h), Vec2::new(body.width() - 24.0, lane_h - 2.0));
        rects.push(r);
        let Some(&(zi, layer)) = layers.get(k) else {
            painter.rect_filled(r, 2.0, s00());
            if k == layers.len() {
                painter.text(Pos2::new(r.left() + 30.0, r.center().y), Align2::LEFT_CENTER, "drop a sample here", font(11.0), white(110));
                let b = Rect::from_min_size(Pos2::new(r.left() + 160.0, r.center().y - 9.0), Vec2::new(64.0, 18.0));
                let resp = ui.interact(b, ui.id().with("layer-add"), Sense::click()).on_hover_text("Choose samples for the next layers");
                painter.rect_filled(b, 2.0, if resp.hovered() { s03() } else { s01() });
                painter.text(b.center(), Align2::CENTER_CENTER, "+ LAYER", font(10.0), white(if resp.hovered() { 245 } else { 200 }));
                if resp.clicked() {
                    acts.push(mact(MorphAct::Add));
                }
            }
            painter.text(Pos2::new(r.left() + 10.0, r.center().y), Align2::CENTER_CENTER, (k + 1).to_string(), font(11.0), white(70));
            continue;
        };
        let Some(z) = zones.get(zi) else { continue };
        let col = LAYER_COLOURS[k];
        let on = k == focus;
        painter.rect_filled(r, 2.0, if on { s02() } else { s01() });
        painter.rect_filled(Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, col);
        // Its number and name: click to focus, drag up or down to reorder.
        let head = Rect::from_min_size(r.min, Vec2::new(170.0, r.height()));
        let resp = ui
            .interact(head, ui.id().with(("layer-head", k)), Sense::click_and_drag())
            .on_hover_text("Click to edit this layer; drag up or down to move it");
        painter.text(
            Pos2::new(r.left() + 14.0, r.center().y),
            Align2::CENTER_CENTER,
            (k + 1).to_string(),
            font(11.0),
            white(if on { 245 } else { 142 }),
        );
        let name = z.sample.path.rsplit(['/', '\\']).next().unwrap_or("");
        painter.text(
            Pos2::new(r.left() + 28.0, r.center().y),
            Align2::LEFT_CENTER,
            elide(&painter, name, 11.0, 136.0),
            font(11.0),
            if on { col } else { white(200) },
        );
        if resp.clicked() {
            acts.push(lact(k, LayerAct::Focus));
        }
        if resp.drag_stopped()
            && let Some(at) = ui.input(|i| i.pointer.interact_pos())
        {
            let to = (((at.y - top) / lane_h).floor().max(0.0) as usize).min(layers.len() - 1);
            if to != k {
                acts.push(lact(k, LayerAct::MoveTo(to)));
            }
        }
        if resp.dragged() {
            painter.rect_stroke(r, 2.0, Stroke::new(1.0, col), egui::StrokeKind::Inside);
        }
        resp.context_menu(|ui| {
            if ui.button("Duplicate layer").clicked() {
                acts.push(lact(k, LayerAct::Duplicate));
                ui.close();
            }
            if ui.button("Remove layer").clicked() {
                acts.push(lact(k, LayerAct::Remove));
                ui.close();
            }
        });
        // Mute and solo.
        for (j, (label, lit, act)) in [("M", layer.mute, LayerAct::Mute), ("S", layer.solo, LayerAct::Solo)].into_iter().enumerate() {
            let b = Rect::from_min_size(Pos2::new(head.right() + 4.0 + j as f32 * 22.0, r.center().y - 9.0), Vec2::new(18.0, 18.0));
            let resp = ui.interact(b, ui.id().with(("layer-ms", k, j)), Sense::click());
            painter.rect_filled(
                b,
                2.0,
                if lit {
                    col
                } else if resp.hovered() {
                    s03()
                } else {
                    s00()
                },
            );
            painter.text(b.center(), Align2::CENTER_CENTER, label, font(10.0), if lit { Color32::BLACK } else { white(164) });
            if resp.clicked() {
                acts.push(lact(k, act));
            }
        }
        // Its level, and its waveform between.
        let level = Rect::from_min_max(Pos2::new(r.right() - 70.0, r.top()), r.max);
        painter.text(level.center(), Align2::CENTER_CENTER, level_text(z.gain), font(11.0), white(if on { 245 } else { 164 }));
        let gain = z.gain;
        let lr = ui.interact(level, ui.id().with(("layer-level", k)), Sense::drag()).on_hover_text("Drag up or down: this layer's level");
        if lr.dragged() {
            let fine = if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1.0 };
            let n = f64::from(-lr.drag_delta().y) * fine;
            if n != 0.0 {
                acts.push(lact(k, LayerAct::Level((gain + n * 0.005).clamp(0.0, 1.0))));
            }
        }
        let wave = Rect::from_min_max(Pos2::new(head.right() + 52.0, r.top() + 2.0), Pos2::new(level.left() - 4.0, r.bottom() - 2.0));
        painter.rect_filled(wave, 0.0, Color32::BLACK);
        // Dimmed by how far shut its window is now, and when it's not heard.
        let open = if morph.on { window_gain(layer.window, at, morph.shape, Fade::Linear) } else { 1.0 };
        let heard = audible.get(k).copied().unwrap_or(true);
        let wcol = col.gamma_multiply(if heard { 0.35 + 0.65 * open } else { 0.2 });
        if let Some(Some(view)) = st.layers.get(k) {
            let n = view.sample.frames.len();
            let cols = wave.width().max(1.0) as usize;
            for c in 0..cols {
                let (a, b) = (c * n / cols, ((c + 1) * n / cols).max(c * n / cols + 1));
                let p = column_of(&view.sample.frames, &view.peaks, a, b);
                let (lo, hi) = (p[0].0.min(p[1].0).clamp(-1.0, 1.0), p[0].1.max(p[1].1).clamp(-1.0, 1.0));
                let x = wave.left() + c as f32 + 0.5;
                let y = |v: f32| wave.center().y - v * wave.height() / 2.0;
                painter.vline(x, y(hi)..=y(lo).max(y(hi) + 1.0), Stroke::new(1.0, wcol));
            }
            if let Some(Some(head)) = st.layer_heads.get(k) {
                let x = wave.left() + *head as f32 / n.max(1) as f32 * wave.width();
                painter.vline(x, wave.y_range(), Stroke::new(1.0, white(245)));
            }
        }
        // Its envelope over it: attack, decay to the sustain level, held, release.
        let hold = (layer.attack + layer.decay + layer.release).max(0.25);
        let total = layer.attack + layer.decay + hold + layer.release;
        let ex = |t: f64| wave.left() + (t / total) as f32 * wave.width();
        let ey = |v: f64| wave.bottom() - v as f32 * wave.height();
        let pts = vec![
            Pos2::new(ex(0.0), ey(0.0)),
            Pos2::new(ex(layer.attack), ey(1.0)),
            Pos2::new(ex(layer.attack + layer.decay), ey(layer.sustain)),
            Pos2::new(ex(layer.attack + layer.decay + hold), ey(layer.sustain)),
            Pos2::new(ex(total), ey(0.0)),
        ];
        painter.add(egui::Shape::line(pts, Stroke::new(1.0, white(if on { 220 } else { 110 }))));
    }
    ui.data_mut(|d| d.insert_temp(egui::Id::new(LANE_RECTS), rects));

    // Below: the focused layer's settings, the morph's, the sound's keys.
    let mut x = body.left() + 12.0;
    if let Some(&(zi, layer)) = layers.get(focus)
        && let Some(z) = zones.get(zi)
    {
        let zt = Target::Zone(g, s, zi);
        let (tune, gain, pan_v, root) = (z.tune, z.gain, z.pan, z.root_key);
        let title = format!("LAYER {}", focus + 1);
        x = group_box(
            ui,
            &painter,
            x,
            footer_y,
            &title,
            &[
                Field {
                    enter: read(move |t| value_entry::level(t).map(|v| lact(focus, LayerAct::Level(v)))),
                    ..field("LEVEL", level_text(gain), move |n| lact(focus, LayerAct::Level((gain + n * 0.005).clamp(0.0, 1.0))))
                },
                Field {
                    enter: read(move |t| value_entry::pan(t).map(|v| Act::SetParam(zt, "pan", v))),
                    ..field("PAN", pan(pan_v), move |n| Act::SetParam(zt, "pan", (pan_v + n * 0.01).clamp(-1.0, 1.0)))
                },
                Field {
                    enter: num(move |v| Act::SetParam(zt, "tune", v.clamp(-36.0, 36.0))),
                    ..field("TUNE", format!("{tune:+.2}"), move |n| Act::SetParam(zt, "tune", (tune + n * 0.01).clamp(-36.0, 36.0)))
                },
                Field {
                    enter: note_in(move |k| Act::SetParam(zt, "root_key", k as f64)),
                    ..field("ROOT", note_name(root), move |n| Act::SetParam(zt, "root_key", (root as f64 + n).clamp(0.0, 127.0)))
                },
            ],
            acts,
        );
        x = group_box(
            ui,
            &painter,
            x,
            footer_y,
            "ENVELOPE",
            &[
                Field {
                    enter: secs(false, move |s| lact(focus, LayerAct::Attack(s.clamp(0.0, TIME_MAX)))),
                    ..field("ATTACK", time_text(layer.attack), move |n| lact(focus, LayerAct::Attack(time_stepped(layer.attack, n))))
                },
                Field {
                    enter: secs(false, move |s| lact(focus, LayerAct::Decay(s.clamp(0.0, TIME_MAX)))),
                    ..field("DECAY", time_text(layer.decay), move |n| lact(focus, LayerAct::Decay(time_stepped(layer.decay, n))))
                },
                Field {
                    enter: num(move |v| lact(focus, LayerAct::Sustain((v / 100.0).clamp(0.0, 1.0)))),
                    ..field("SUSTAIN", format!("{:.0} %", layer.sustain * 100.0), move |n| {
                        lact(focus, LayerAct::Sustain((layer.sustain + n * 0.01).clamp(0.0, 1.0)))
                    })
                },
                Field {
                    enter: secs(false, move |s| lact(focus, LayerAct::Release(s.clamp(0.0, TIME_MAX)))),
                    ..field("RELEASE", time_text(layer.release), move |n| lact(focus, LayerAct::Release(time_stepped(layer.release, n))))
                },
            ],
            acts,
        );
    }
    let m = morph;
    let shapes = vec!["Round".to_string(), "There and back".to_string()];
    let fades = vec!["Linear".to_string(), "Equal power".to_string()];
    let mut depth = field("DEPTH", if m.shape == Shape::Round { "-".into() } else { format!("{:.0} %", m.depth * 100.0) }, move |n| {
        mact(MorphAct::Depth(if m.shape == Shape::Round { m.depth } else { (m.depth + n * 0.01).clamp(0.0, 1.0) }))
    });
    if m.shape == Shape::Round {
        depth.tip = "Round always goes all the way: depth is There and back's";
    } else {
        depth.enter = num(|v| mact(MorphAct::Depth((v / 100.0).clamp(0.0, 1.0))));
    }
    let mut spread = field("WINDOWS", if m.spread { "Even".into() } else { "By hand".into() }, move |_| mact(MorphAct::Spread));
    spread.tip = "Click: spread the layers' windows evenly again";
    spread.click = Some(Box::new(move || mact(MorphAct::Spread)));
    x = group_box(
        ui,
        &painter,
        x,
        footer_y,
        "MORPH",
        &[
            toggle_field("ON", m.on, move || mact(MorphAct::On(!m.on))),
            Field {
                enter: num(|v| mact(MorphAct::Position((v / 100.0).clamp(0.0, 1.0)))),
                ..field("POSITION", format!("{:.0} %", m.position * 100.0), move |n| {
                    mact(MorphAct::Position((m.position + n * 0.005).clamp(0.0, 1.0)))
                })
            },
            Field { enter: read(move |t| rate_typed(&m, t).map(mact)), ..field("RATE", rate_text(&m), move |n| mact(rate_stepped(&m, n))) },
            toggle_field("SYNC", m.sync, move || mact(MorphAct::Sync(!m.sync))),
            choice_field("SHAPE", shapes, usize::from(m.shape == Shape::Back), |k| {
                mact(MorphAct::Shape(if k == 1 { Shape::Back } else { Shape::Round }))
            }),
            depth,
            toggle_field("RESET", m.reset, move || mact(MorphAct::Reset(!m.reset))),
            choice_field("FADE", fades, usize::from(m.fade == Fade::Power), |k| {
                mact(MorphAct::Fade(if k == 1 { Fade::Power } else { Fade::Linear }))
            }),
            spread,
        ],
        acts,
    );
    if let Some(z) = zones.first().filter(|_| !layers.is_empty()) {
        let (lo, hi) = z.key_range;
        group_box(
            ui,
            &painter,
            x,
            footer_y,
            "KEYS",
            &[
                Field {
                    enter: note_in(move |k| mact(MorphAct::Keys(k.clamp(0, hi), hi))),
                    ..field("LOW", note_name(lo), move |n| mact(MorphAct::Keys((lo as f64 + n).clamp(0.0, hi as f64) as i64, hi)))
                },
                Field {
                    enter: note_in(move |k| mact(MorphAct::Keys(lo, k.clamp(lo, 127)))),
                    ..field("HIGH", note_name(hi), move |n| mact(MorphAct::Keys(lo, (hi as f64 + n).clamp(lo as f64, 127.0) as i64)))
                },
            ],
            acts,
        );
    }
    let _ = gcol;
}
