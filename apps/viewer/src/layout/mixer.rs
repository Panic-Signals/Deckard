//! The Mixer view (docs/mixer-layout.md): a strip per group (or per sound of the focused
//! group), each with its header, plug-in slots, pan, headroom, meters, fader, cue and
//! mute, then the output; the master (or, at sound level, the focused group) fixed at
//! the right. It replaces the work view; the chain area stays below it.

use super::*;

/// Strip pitch (71 + a 1px stroke) and the master column's width.
const PITCH: f32 = 72.0;
const MASTER_W: f32 = 81.0;
const HEAD: f32 = 40.0;
const LEVEL_H: f32 = 186.0;
const SLOT_ROW: f32 = 20.0;

/// Whose channel a strip is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Group(usize),
    Sound(usize, usize),
    Master,
}

impl Channel {
    fn target(self) -> Target {
        match self {
            Channel::Group(g) => Target::Group(g),
            Channel::Sound(g, s) => Target::Sound(g, s),
            Channel::Master => Target::Song,
        }
    }
}

/// What a strip shows.
struct Strip {
    ch: Channel,
    id: String,
    name: String,
    color: Color32,
    level: f64,
    pan: f64,
    muted: bool,
    cue: bool,
    focused: bool,
    meter: (f32, f32),
    /// Held peaks (left, right).
    peak: (f32, f32),
    slots: Vec<deckard_model::SlotInfo>,
    /// Routing selectors: audio in, MIDI in, main output, aux 1, aux 2, MIDI out.
    io: [Option<IoField>; 6],
    /// Aux 1 and 2 send levels.
    aux: [f64; 2],
}

/// What a pick in a routing list does.
#[derive(Clone)]
enum Pick {
    Route(RouteSet),
    Param(Target, &'static str, f64),
}

/// A routing selector: what it shows, and its list.
struct IoField {
    text: String,
    names: Vec<String>,
    picks: Vec<Pick>,
    cur: usize,
}

/// A page's routing selector in slot `slot`, showing `none` when it's off.
fn route_field(pg: &channel::Page, slot: usize, none: &str) -> Option<IoField> {
    let (_, names, sets) = pg.routes.iter().find(|(i, ..)| *i == slot)?;
    let cur = match pg.slots.get(slot) {
        Some(Some((_, _, _, Some(Edit::Route(c))))) => *c,
        _ => usize::MAX,
    };
    let shown = names.get(cur).map_or("?", String::as_str);
    let text = if shown == "None" { none.to_string() } else { channel::abbrev(shown.split(" - ").next().unwrap_or(shown)) };
    Some(IoField { text, names: names.clone(), picks: sets.iter().cloned().map(Pick::Route).collect(), cur })
}

/// The routing selectors of a channel (see [`Strip::io`]).
fn io_fields(c: &channel::Ctx, level: Level) -> [Option<IoField>; 6] {
    let pg = |set, page| channel::page(c, level, set, page);
    match level {
        Level::Sound(..) => {
            let aux = pg(1, 1);
            [
                route_field(&pg(0, 0), 0, "AUDIO IN"),
                route_field(&pg(0, 1), 0, "MIDI IN"),
                route_field(&pg(1, 0), 0, "None"),
                route_field(&aux, 0, "AUX 1"),
                route_field(&aux, 4, "AUX 2"),
                route_field(&pg(1, 2), 0, "MIDI OUT"),
            ]
        }
        Level::Group(g) => {
            let aux = pg(1, 1);
            let ch = &c.p.groups[g].channel;
            // MIDI in: the source in Manual key mode, else the key mode itself.
            let midi_in = if ch.key_mode == 2 {
                route_field(&pg(0, 0), 1, "MIDI IN")
            } else {
                let cur = ch.key_mode.clamp(0, 2) as usize;
                Some(IoField {
                    text: if cur == 0 { "MIDI IN".into() } else { channel::KEY_MODES[cur].into() },
                    names: channel::KEY_MODES.iter().map(|s| format!("Key Mode: {s}")).collect(),
                    picks: (0..3).map(|k| Pick::Param(Target::Group(g), "key_mode", k as f64)).collect(),
                    cur,
                })
            };
            [None, midi_in, route_field(&pg(1, 0), 0, "None"), route_field(&aux, 0, "AUX 1"), route_field(&aux, 4, "AUX 2"), None]
        }
        Level::Master => [None, None, route_field(&pg(1, 0), 0, "Ext. 1"), None, None, None],
    }
}

/// A routing selector: a click lists the choices.
fn io_button(ui: &Ui, painter: &Painter, b: Rect, id: egui::Id, f: &IoField, acts: &mut Vec<Act>) {
    let resp = ui.interact(b, id, Sense::click()).on_hover_text(f.names.get(f.cur).cloned().unwrap_or_default());
    painter.rect_filled(b, 2.0, if resp.hovered() { S03 } else { S02 });
    let text = painter.layout_no_wrap(f.text.clone(), font(10.0), white(200));
    let w = b.width() - 10.0;
    let shown = if text.size().x > w {
        let mut t = f.text.clone();
        while !t.is_empty() && painter.layout_no_wrap(format!("{t}…"), font(10.0), white(200)).size().x > w {
            t.pop();
        }
        format!("{t}…")
    } else {
        f.text.clone()
    };
    painter.text(
        Pos2::new(b.left() + 5.0, b.center().y),
        Align2::LEFT_CENTER,
        shown,
        font(10.0),
        white(if f.cur == 0 { 142 } else { 220 }),
    );
    if let Some(k) = choice_list(&resp, f.names.iter().map(String::as_str), f.cur)
        && k != f.cur
    {
        acts.push(match f.picks[k].clone() {
            Pick::Route(r) => Act::SetRoute(r),
            Pick::Param(t, field, v) => Act::SetParam(t, field, v),
        });
    }
}

/// The meters' width and their spacing (the fader's arrow is beside them).
const METER_W: f32 = 13.0;
const METER_STEP: f32 = 14.0;

/// dB to the y offset in the 99-high scale/meter area: +10, 0, −10, −20, −30 dB and −∞
/// at 3, 25, 47, 63, 79 and 95 (`MIX_MTR_LevelScale`).
fn db_y(db: f32) -> f32 {
    const PTS: [(f32, f32); 5] = [(10.0, 3.0), (0.0, 25.0), (-10.0, 47.0), (-20.0, 63.0), (-30.0, 79.0)];
    if db >= 10.0 {
        return 3.0;
    }
    for w in PTS.windows(2) {
        let ((d0, y0), (d1, y1)) = (w[0], w[1]);
        if db >= d1 {
            return y0 + (d0 - db) / (d0 - d1) * (y1 - y0);
        }
    }
    // Below −30 dB: the last 16 px fall to −∞ (at −60 dB).
    (79.0 + (-30.0 - db) / 30.0 * 16.0).min(95.0)
}

fn y_db(y: f32) -> f32 {
    // Invert db_y by bisection (it is monotonic).
    let (mut lo, mut hi) = (-90.0f32, 10.0f32);
    if y <= 3.0 {
        return 10.0;
    }
    if y >= 95.0 {
        return f32::NEG_INFINITY;
    }
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if db_y(mid) > y {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

fn to_db(level: f32) -> f32 {
    if level <= 0.0 { f32::NEG_INFINITY } else { 20.0 * level.log10() }
}

/// The Mixer, in `rect`.
pub fn mixer(ui: &mut Ui, app: &App, doc: &deckard_doc::Document, rect: Rect, acts: &mut Vec<Act>) {
    let p = doc.project();
    let groove = |g: usize, s: usize, f: &str| app.groove_setting(g, s, f);
    let on = |prefix: &str, v: &Vec<String>| -> Vec<String> {
        v.iter().filter(|n| !app.settings.midi_off.contains(&format!("{prefix}{n}"))).cloned().collect()
    };
    let (midi_ins, midi_outs) = (on("in:", &app.midi_ports.0), on("out:", &app.midi_ports.1));
    let ctx = channel::Ctx {
        doc,
        has_midi_macro: false,
        p,
        midi_ins: &midi_ins,
        midi_outs: &midi_outs,
        polyphonic: false,
        groove: &groove,
        io: &app.settings,
        plugin_knob: &|_| None,
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, S00);
    // Sidebar: extend/collapse, IO, plug-ins, aux.
    let side = Rect::from_min_size(rect.min, Vec2::new(STRIP, rect.height()));
    painter.rect_filled(side, 0.0, S05);
    painter.vline(side.right() - 0.5, side.y_range(), Stroke::new(1.0, Color32::BLACK));
    let toggles: [(bool, &str, &str, MixerSection); 4] = [
        (app.mixer_sections.extended, "⇕", "Extend or collapse the strips", MixerSection::Extended),
        (app.mixer_sections.io, "IO", "Inputs and outputs", MixerSection::Io),
        (app.mixer_sections.plugins, "FX", "Plug-in slots", MixerSection::Plugins),
        (app.mixer_sections.aux, "AUX", "Aux sends", MixerSection::Aux),
    ];
    for (i, (on, text, tip, sec)) in toggles.into_iter().enumerate() {
        let y = rect.top() + 25.0 + i as f32 * 24.0 + if i > 0 { 8.0 } else { 0.0 };
        let r = Rect::from_min_size(Pos2::new(side.left() + 3.0, y), Vec2::new(STRIP - 6.0, 20.0));
        let resp = ui.interact(r, ui.id().with(("mix-side", i)), Sense::click()).on_hover_text(tip);
        if on {
            painter.rect_filled(r, 3.0, S03);
        } else if resp.hovered() {
            painter.rect_filled(r, 3.0, white(20));
        }
        let ink = if on { white(245) } else { white(142) };
        if i == 0 {
            // Extend/collapse: a triangle pointing to what a click does.
            let c = r.center();
            let pts = if on {
                vec![c + Vec2::new(-5.0, 3.0), c + Vec2::new(5.0, 3.0), c + Vec2::new(0.0, -3.0)]
            } else {
                vec![c + Vec2::new(-5.0, -3.0), c + Vec2::new(5.0, -3.0), c + Vec2::new(0.0, 3.0)]
            };
            painter.add(egui::Shape::convex_polygon(pts, ink, Stroke::NONE));
        } else {
            painter.text(r.center(), Align2::CENTER_CENTER, text, font(10.0), ink);
        }
        if resp.clicked() {
            acts.push(Act::MixerSection(sec));
        }
    }

    // The strips: groups, or the focused group's sounds.
    let sounds = app.mixer_sounds;
    let held = |ch: Channel| app.peaks.get(&ch).map_or((0.0, 0.0), |p| (p[0].0, p[1].0));
    let gmeter = |g: usize| app.meters.get(g).map_or((0.0, 0.0), |row| row.iter().fold((0f32, 0f32), |m, v| (m.0.max(v.0), m.1.max(v.1))));
    let mut strips: Vec<Strip> = Vec::new();
    if sounds {
        if let Some(gr) = p.groups.get(app.group) {
            for (s, snd) in gr.sounds.iter().enumerate() {
                strips.push(Strip {
                    ch: Channel::Sound(app.group, s),
                    id: (s + 1).to_string(),
                    name: if snd.name.is_empty() { format!("Sound {}", s + 1) } else { snd.name.clone() },
                    color: sound_color(p, app.group, s),
                    level: snd.level,
                    pan: snd.pan,
                    muted: snd.muted,
                    cue: snd.cue,
                    focused: app.sound == s,
                    meter: app.meters.get(app.group).and_then(|r| r.get(s)).copied().unwrap_or((0.0, 0.0)),
                    peak: held(Channel::Sound(app.group, s)),
                    slots: snd.slots.clone(),
                    io: io_fields(&ctx, Level::Sound(app.group, s)),
                    aux: snd.aux,
                });
            }
        }
    } else {
        for (g, gr) in p.groups.iter().enumerate() {
            strips.push(Strip {
                ch: Channel::Group(g),
                id: group_id(g),
                name: group_name(p, g),
                color: group_color(p, g),
                level: gr.level,
                pan: gr.pan,
                muted: gr.muted,
                cue: gr.cue,
                focused: app.group == g,
                meter: gmeter(g),
                peak: held(Channel::Group(g)),
                slots: gr.slots.clone(),
                io: io_fields(&ctx, Level::Group(g)),
                aux: gr.aux,
            });
        }
    }
    // The fixed right column: the master, or the focused group at sound level.
    let right = Rect::from_min_max(Pos2::new(rect.right() - MASTER_W, rect.top()), rect.max);
    let fixed = if sounds && let Some(gr) = p.groups.get(app.group) {
        Strip {
            ch: Channel::Group(app.group),
            id: group_id(app.group),
            name: group_name(p, app.group),
            color: group_color(p, app.group),
            level: gr.level,
            pan: gr.pan,
            muted: gr.muted,
            cue: gr.cue,
            focused: true,
            meter: gmeter(app.group),
            peak: held(Channel::Group(app.group)),
            slots: gr.slots.clone(),
            io: io_fields(&ctx, Level::Group(app.group)),
            aux: gr.aux,
        }
    } else {
        Strip {
            ch: Channel::Master,
            id: String::new(),
            name: "Master".into(),
            color: Color32::WHITE,
            level: p.master_level,
            pan: p.master_pan,
            muted: false,
            cue: false,
            focused: false,
            meter: app.master_meter,
            peak: held(Channel::Master),
            slots: p.master_slots.clone(),
            io: io_fields(&ctx, Level::Master),
            aux: [0.0; 2],
        }
    };

    let area = Rect::from_min_max(Pos2::new(side.right() + 2.0, rect.top()), Pos2::new(right.left() - 8.0, rect.bottom() - 19.0));
    let off = hscroll(ui, "mixer-strips", area, strips.len() as f32 * PITCH);
    let clip = painter.with_clip_rect(area);
    let mut rects = Vec::new();
    for (i, st) in strips.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(area.left() + i as f32 * PITCH - off, area.top()), Vec2::new(PITCH - 1.0, area.height()));
        if r.right() < area.left() || r.left() > area.right() {
            continue;
        }
        strip(ui, app, &clip, r, area, st, sounds, acts);
        rects.push((st.ch, r));
    }
    // Scrollbar track under the strips.
    let bar = Rect::from_min_max(Pos2::new(area.left() + 5.0, area.bottom() + 1.0), Pos2::new(area.right() - 5.0, rect.bottom() - 1.0));
    painter.rect_filled(bar, 2.0, Color32::BLACK);
    let content = strips.len() as f32 * PITCH;
    if content > area.width() {
        let w = bar.width() * area.width() / content;
        let x = bar.left() + (bar.width() - w) * off / (content - area.width());
        painter.rect_filled(Rect::from_min_size(Pos2::new(x, bar.top() + 3.0), Vec2::new(w, bar.height() - 6.0)), 3.0, S03);
    }
    // The master column, with its shadow over the strips.
    for k in 0..8 {
        painter.vline(right.left() - 8.0 + k as f32 + 0.5, area.y_range(), Stroke::new(1.0, Color32::from_black_alpha((k * 12) as u8)));
    }
    let inner = Rect::from_min_max(Pos2::new(right.left() + 5.0, right.top()), Pos2::new(right.right() - 5.0, rect.bottom() - 19.0));
    strip(ui, app, &painter, inner, inner, &fixed, sounds, acts);
    rects.push((fixed.ch, inner));
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-mixer-strips"), rects));
}

/// One channel strip in `r` (clipped to `clip`).
#[allow(clippy::too_many_arguments)]
fn strip(ui: &mut Ui, app: &App, painter: &Painter, r: Rect, clip: Rect, st: &Strip, sounds: bool, acts: &mut Vec<Act>) {
    let master = st.ch == Channel::Master;
    let key = st.ch;
    let id = ui.id().with(("mix", format!("{key:?}")));
    let vis = |x: Rect| x.intersect(clip);
    // Body.
    let body = Rect::from_min_max(Pos2::new(r.left(), r.top() + HEAD + 4.0), r.max);
    painter.rect_filled(body, 0.0, if st.focused { Color32::from_rgb(0x14, 0x14, 0x14) } else { S05 });
    painter.vline(r.right() + 0.5, r.y_range(), Stroke::new(1.0, Color32::BLACK));

    // Header: ID and mini meter, then the name.
    let head = Rect::from_min_size(r.min, Vec2::new(r.width(), HEAD));
    let mute_col = MUTE_COL;
    let (bg, id_ink, name_ink) = match (st.focused && !master, st.muted) {
        (true, false) => (st.color, Color32::from_black_alpha(245), Color32::from_black_alpha(245)),
        (true, true) => (mute_col, Color32::from_black_alpha(245), Color32::from_black_alpha(245)),
        (false, true) => (if sounds { S00 } else { S05 }, mute_col, white(142)),
        (false, false) => (if matches!(st.ch, Channel::Sound(..)) { S00 } else { S05 }, st.color, white(245)),
    };
    painter.rect_filled(head, 0.0, bg);
    let row1 = Rect::from_min_size(Pos2::new(head.left() + 4.0, head.top()), Vec2::new(37.0, 20.0));
    if master {
        painter.text(Pos2::new(head.left() + 4.0, head.top() + 10.0), Align2::LEFT_CENTER, "MASTER", font(11.0), white(245));
    } else {
        painter.text(row1.left_center(), Align2::LEFT_CENTER, &st.id, font(14.0), id_ink);
    }
    // Mini meter.
    let mm = Rect::from_min_size(Pos2::new(head.right() - 8.0, head.top() + 4.0), Vec2::new(2.0, 14.0));
    painter.rect_filled(mm, 0.0, Color32::BLACK);
    let lv = st.meter.0.max(st.meter.1);
    let fill = (95.0 - db_y(to_db(lv))) / 92.0;
    if fill > 0.0 {
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(mm.left(), mm.bottom() - mm.height() * fill.clamp(0.0, 1.0)), mm.max),
            0.0,
            name_ink,
        );
    }
    let name_r = Rect::from_min_size(Pos2::new(head.left() + 4.0, head.top() + 21.0), Vec2::new(r.width() - 6.0, 18.0));
    let renaming = app.renaming == Some(st.ch.target()) && !master;
    if renaming {
        let rt = match st.ch {
            Channel::Group(g) => Target::Group(g),
            ch => ch.target(),
        };
        rename_editor(ui, name_r, rt, &st.name, acts);
    } else {
        let p = painter.with_clip_rect(vis(name_r));
        p.text(name_r.left_center(), Align2::LEFT_CENTER, &st.name, font(12.0), name_ink);
    }
    // Header clicks: focus; double-click the ID: mute; the name: rename; elsewhere:
    // switch between groups and sounds.
    if !master {
        let resp = ui
            .interact(vis(head), id.with("head"), Sense::click())
            .on_hover_text("Click: focus. Double-click the ID: mute, the name: rename, elsewhere: show the group's sounds (or the groups)");
        if resp.clicked() {
            match st.ch {
                Channel::Group(g) => acts.push(Act::FocusGroup(g)),
                Channel::Sound(_, s) => acts.push(Act::FocusSound(s)),
                Channel::Master => {}
            }
        }
        if resp.double_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            if row1.contains(pos) {
                acts.push(Act::ToggleMute(st.ch.target()));
            } else if name_r.contains(pos) {
                acts.push(Act::StartRename(match st.ch {
                    Channel::Group(g) => Target::Group(g),
                    ch => ch.target(),
                }));
            } else if let Channel::Group(g) = st.ch {
                acts.push(Act::FocusGroup(g));
                acts.push(Act::MixerSounds(!sounds));
            }
        }
    }
    // The group-colour bar.
    let bar = Rect::from_min_size(Pos2::new(r.left(), head.bottom()), Vec2::new(r.width(), 3.0));
    painter.rect_filled(
        bar,
        0.0,
        if st.muted {
            mute_col
        } else if master {
            white(200)
        } else {
            st.color
        },
    );

    // Plug-in slots (sounds only for now: their chains are editable).
    let mut y = bar.bottom() + 4.0;
    // Inputs (IO): audio in (sounds) and MIDI in. Routing isn't editable yet: the
    // projects store the defaults.
    if app.mixer_sections.extended && app.mixer_sections.io && !master {
        let rows: &[usize] = if matches!(st.ch, Channel::Sound(..)) { &[0, 1] } else { &[1] };
        for &k in rows {
            let b = Rect::from_min_size(Pos2::new(r.left() + 3.0, y + 2.0), Vec2::new(r.width() - 6.0, 17.0));
            if let Some(f) = &st.io[k] {
                io_button(ui, painter, b.intersect(clip), ui.id().with(("mixer-io", st.ch, k)), f, acts);
            }
            y += 21.0;
        }
        y += 4.0;
    }
    if app.mixer_sections.extended
        && app.mixer_sections.plugins
        && !master
        && let Channel::Sound(g, s) = st.ch
    {
        {
            for (i, slot) in st.slots.iter().enumerate() {
                let row = Rect::from_min_size(Pos2::new(r.left(), y), Vec2::new(r.width(), SLOT_ROW - 1.0));
                let sq = Rect::from_min_size(row.min + Vec2::new(2.0, 4.0), Vec2::new(10.0, 11.0));
                painter.rect_filled(row, 0.0, S01);
                painter.rect_filled(sq, 1.0, if slot.active { st.color } else { S03 });
                let p = painter.with_clip_rect(vis(row));
                p.text(
                    Pos2::new(sq.right() + 4.0, row.center().y),
                    Align2::LEFT_CENTER,
                    &slot.name,
                    font(11.0),
                    if slot.active { white(220) } else { white(110) },
                );
                if ui.interact(vis(sq), id.with(("bypass", i)), Sense::click()).on_hover_text("Bypass").clicked() {
                    acts.push(Act::SlotActiveOf(g, s, i, !slot.active));
                }
                let name = Rect::from_min_max(Pos2::new(sq.right(), row.top()), row.max);
                if ui.interact(vis(name), id.with(("slot", i)), Sense::click()).on_hover_text("Show its pages").clicked() {
                    acts.push(Act::FocusSlot(g, s, i));
                }
                y += SLOT_ROW;
            }
            let add = Rect::from_min_size(Pos2::new(r.left(), y), Vec2::new(r.width(), SLOT_ROW - 1.0));
            let resp =
                ui.interact(vis(add), id.with("add"), Sense::click()).on_hover_text("Add a module or plug-in (in the SOUND tab below)");
            painter.rect_stroke(add, 2.0, Stroke::new(1.0, if resp.hovered() { white(120) } else { white(40) }), StrokeKind::Inside);
            painter.text(add.center(), Align2::CENTER_CENTER, "+", font(13.0), white(142));
            if resp.clicked() {
                acts.push(Act::FocusSlot(g, s, usize::MAX));
            }
        }
    }
    // A group's and the master's effects: bypassed with the square, shown (in the
    // GROUP or MASTER tab) with the name.
    let chain = match st.ch {
        Channel::Group(g) => Some(deckard_doc::Chain::Group(g)),
        Channel::Master => Some(deckard_doc::Chain::Master),
        _ => None,
    };
    if app.mixer_sections.extended
        && app.mixer_sections.plugins
        && let Some(chain) = chain
    {
        for (i, slot) in st.slots.iter().enumerate() {
            let row = Rect::from_min_size(Pos2::new(r.left(), y), Vec2::new(r.width(), SLOT_ROW - 1.0));
            let sq = Rect::from_min_size(row.min + Vec2::new(2.0, 4.0), Vec2::new(10.0, 11.0));
            painter.rect_filled(row, 0.0, S01);
            painter.rect_filled(sq, 1.0, if slot.active { st.color } else { S03 });
            let p = painter.with_clip_rect(vis(row));
            p.text(
                Pos2::new(sq.right() + 4.0, row.center().y),
                Align2::LEFT_CENTER,
                &slot.name,
                font(11.0),
                if slot.active { white(220) } else { white(110) },
            );
            if ui.interact(vis(sq), id.with(("cbypass", i)), Sense::click()).on_hover_text("Bypass").clicked() {
                acts.push(Act::ChainSlotActive(chain, i, !slot.active));
            }
            let name = Rect::from_min_max(Pos2::new(sq.right(), row.top()), row.max);
            if ui.interact(vis(name), id.with(("cslot", i)), Sense::click()).on_hover_text("Show its pages").clicked() {
                acts.push(Act::FocusChainSlot(chain, i));
            }
            y += SLOT_ROW;
        }
    }
    let _ = y;
    if !app.mixer_sections.extended {
        return;
    }

    // The level module, at the bottom.
    // From the bottom: MIDI out (IO, sounds), the aux sends (AUX), then the level module.
    let mut bottom = r.bottom();
    if app.mixer_sections.io && matches!(st.ch, Channel::Sound(..)) {
        let b = Rect::from_min_size(Pos2::new(r.left() + 3.0, bottom - 20.0), Vec2::new(r.width() - 6.0, 17.0));
        if let Some(f) = &st.io[5] {
            io_button(ui, painter, b.intersect(clip), ui.id().with(("mixer-io", st.ch, 5)), f, acts);
        }
        bottom -= 22.0;
    }
    if app.mixer_sections.aux && !master {
        let aux_top = bottom - 62.0;
        let t = match st.ch {
            Channel::Group(g) => Target::GroupStrip(g),
            ch => ch.target(),
        };
        for k in 0..2 {
            let row = Rect::from_min_size(Pos2::new(r.left() + 1.0, aux_top + 3.0 + k as f32 * 30.0), Vec2::new(r.width() - 2.0, 17.0));
            let dest = Rect::from_min_max(row.min, Pos2::new(row.right() - 20.0, row.bottom()));
            match &st.io[3 + k] {
                Some(f) => io_button(ui, painter, dest.intersect(clip), ui.id().with(("mixer-io", st.ch, 3 + k)), f, acts),
                None => {
                    painter.rect_filled(row, 2.0, S02);
                }
            }
            // The send level: a small knob.
            let kc = Pos2::new(row.right() - 10.0, row.center().y);
            let v = st.aux[k];
            let start = std::f32::consts::PI * 0.75;
            let sweep = std::f32::consts::PI * 1.5;
            let arc = |from: f32, to: f32, col: Color32| {
                let pts: Vec<Pos2> = (0..=12).map(|i| kc + Vec2::angled(from + (to - from) * i as f32 / 12.0) * 6.5).collect();
                painter.add(egui::Shape::line(pts, Stroke::new(2.0, col)));
            };
            arc(start, start + sweep, white(40));
            arc(start, start + sweep * v.clamp(0.0, 1.0) as f32, st.color);
            let kr = Rect::from_center_size(kc, Vec2::splat(17.0));
            let resp = ui.interact(vis(kr), id.with(("aux", k)), Sense::click_and_drag()).on_hover_text(format!(
                "Aux {} send {} (drag; double-click: 0 dB)",
                k + 1,
                db(v)
            ));
            if resp.dragged() {
                let fine = ui.input(|i| i.modifiers.shift);
                let d = -resp.drag_delta().y as f64 / if fine { 1000.0 } else { 100.0 };
                if d != 0.0 {
                    acts.push(Act::SetAux(t, k, (v + d).clamp(0.0, 1.0)));
                }
            }
            if resp.double_clicked() {
                acts.push(Act::SetAux(t, k, 0.75));
            }
        }
        bottom = aux_top;
    }
    let lm = Rect::from_min_size(Pos2::new(r.left(), bottom - LEVEL_H), Vec2::new(r.width(), LEVEL_H));
    painter.hline(lm.x_range(), lm.top() + 0.5, Stroke::new(1.0, Color32::BLACK));
    let target = st.ch.target();
    // Pan: a bipolar bar.
    let pan_r = Rect::from_min_size(Pos2::new(lm.left() + 5.0, lm.top() + 6.0), Vec2::new(61.0, 5.0));
    painter.rect_filled(pan_r, 0.0, S01);
    let cx = pan_r.center().x;
    let px = cx + st.pan as f32 * pan_r.width() / 2.0;
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(cx.min(px), pan_r.top()), Pos2::new(cx.max(px).max(cx.min(px) + 1.0), pan_r.bottom())),
        0.0,
        st.color,
    );
    painter.vline(cx, pan_r.y_range(), Stroke::new(1.0, S00));
    let pan_resp = ui
        .interact(vis(pan_r.expand2(Vec2::new(0.0, 4.0))), id.with("pan"), Sense::click_and_drag())
        .on_hover_text(format!("Pan {} (drag; double-click: centre)", pan(st.pan)));
    if pan_resp.hovered() || pan_resp.dragged() {
        painter.rect_filled(pan_r, 0.0, white(20));
    }
    if pan_resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let d = pan_resp.drag_delta().x / if fine { 300.0 } else { 30.5 };
        if d != 0.0 {
            acts.push(Act::SetPan(target, (st.pan + d as f64).clamp(-1.0, 1.0)));
        }
    }
    if pan_resp.double_clicked() {
        acts.push(Act::SetPan(target, 0.0));
    }
    // Headroom: the peak since the last click.
    let hr = Rect::from_min_size(Pos2::new(lm.left(), lm.top() + 13.0), Vec2::new(r.width(), 11.0));
    let peak = app.headroom.get(&key).copied().unwrap_or(0.0);
    let clipped = peak > 1.0;
    let text = if peak <= 1e-6 { "-inf".to_string() } else { format!("{:.1}", to_db(peak)) };
    painter.text(hr.center(), Align2::CENTER_CENTER, text, font(9.0), if clipped { Color32::WHITE } else { white(61) });
    if ui.interact(vis(hr), id.with("headroom"), Sense::click()).on_hover_text("Peak level since the last click; click to reset").clicked()
    {
        acts.push(Act::ResetHeadroom(key));
    }
    // Scale, meters, fader.
    let sm = Rect::from_min_size(Pos2::new(lm.left(), lm.top() + 26.0), Vec2::new(r.width(), 99.0));
    for (db, label) in [(10.0, "10"), (0.0, "0"), (-10.0, "10"), (-20.0, "20"), (-30.0, "30"), (f32::NEG_INFINITY, "∞")] {
        let yy = sm.top() + if db.is_finite() { db_y(db) } else { 95.0 };
        painter.text(Pos2::new(sm.left() + 15.0, yy), Align2::RIGHT_CENTER, label, font(8.0), white(70));
    }
    painter.hline((sm.left() + 18.0)..=(sm.left() + 22.0 + METER_STEP + METER_W + 2.0), sm.top() + 25.0, Stroke::new(1.0, white(41)));
    let meter_col = if master { Color32::WHITE } else { st.color };
    for (k, v) in [st.meter.0, st.meter.1].into_iter().enumerate() {
        let m = Rect::from_min_size(Pos2::new(sm.left() + 22.0 + k as f32 * METER_STEP, sm.top() + 1.0), Vec2::new(METER_W, 96.0));
        painter.rect_filled(m, 0.0, S00);
        let top = sm.top() + db_y(to_db(v)).min(95.0);
        if v > 1e-5 {
            painter.rect_filled(Rect::from_min_max(Pos2::new(m.left(), top), m.max), 0.0, meter_col);
        }
        // The held peak: a line where the level last reached.
        let peak = if k == 0 { st.peak.0 } else { st.peak.1 };
        if peak > 1e-5 {
            let y = sm.top() + db_y(to_db(peak)).min(95.0);
            painter.hline(m.x_range(), y, Stroke::new(1.5, meter_col));
        }
        if st.muted {
            painter.rect_filled(m, 0.0, Color32::from_black_alpha(102));
        }
    }
    // The fader handle: a small left-pointing arrow at the level, beside the meters.
    let level_db = deckard_model::level_db(st.level) as f32;
    let hy = sm.top() + if level_db.is_finite() { db_y(level_db) } else { 95.0 };
    let hx = sm.left() + 22.0 + METER_STEP + METER_W + 2.0;
    painter.add(egui::Shape::convex_polygon(
        vec![Pos2::new(hx, hy), Pos2::new(hx + 6.0, hy - 4.0), Pos2::new(hx + 6.0, hy + 4.0)],
        Color32::from_rgb(0xF2, 0xF2, 0xF2),
        Stroke::NONE,
    ));
    let fader = Rect::from_min_size(Pos2::new(sm.left() + 20.0, sm.top() - 2.0), Vec2::new(r.width() - 20.0, 103.0));
    let fr = ui
        .interact(vis(fader), id.with("fader"), Sense::click_and_drag())
        .on_hover_text(format!("{} (drag; Shift: fine; double-click: 0 dB)", db(st.level)));
    if fr.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let dy = fr.drag_delta().y / if fine { 10.0 } else { 1.0 };
        if dy != 0.0 {
            let cur = if level_db.is_finite() { db_y(level_db) } else { 95.0 };
            let d = y_db((cur + dy).clamp(3.0, 95.0));
            acts.push(Act::SetLevel(target, deckard_model::level_from_db(d as f64)));
        }
    }
    if fr.double_clicked() {
        acts.push(Act::SetLevel(target, 0.75));
    }
    painter.hline(lm.x_range(), lm.top() + 142.5, Stroke::new(1.0, Color32::BLACK));
    // Cue and mute (right-click mute: solo).
    let row = Rect::from_min_size(Pos2::new(lm.left(), lm.top() + 143.0), Vec2::new(r.width(), 20.0));
    if !master {
        let cue = Rect::from_min_size(Pos2::new(row.left() + 4.0, row.top()), Vec2::new(20.0, 20.0));
        let cr = ui.interact(vis(cue), id.with("cue"), Sense::click()).on_hover_text("Cue: send to the headphones bus");
        headphones(
            painter,
            cue.center(),
            if st.cue {
                Color32::WHITE
            } else if cr.hovered() {
                white(200)
            } else {
                white(110)
            },
        );
        if cr.clicked() {
            acts.push(Act::Cue(target, !st.cue));
        }
        let m = Rect::from_min_size(Pos2::new(row.right() - 40.0, row.top() + 2.0), Vec2::new(34.0, 16.0));
        let mr = ui.interact(vis(m), id.with("mute"), Sense::click()).on_hover_text("Mute (right-click: solo)");
        mute_face(painter, m, st.muted, mr.hovered());
        if mr.clicked() {
            acts.push(Act::ToggleMute(target));
        }
        if mr.secondary_clicked() {
            acts.push(Act::Solo(st.ch));
        }
    }
    // Output.
    let out = Rect::from_min_size(Pos2::new(lm.left() + 3.0, lm.top() + 166.0), Vec2::new(65.0, 17.0));
    if let Some(f) = &st.io[2] {
        io_button(ui, painter, out.intersect(clip), ui.id().with(("mixer-io", st.ch, 2)), f, acts);
    }
}

/// A muted strip's grey.
const MUTE_COL: Color32 = Color32::from_rgb(0x84, 0x84, 0x84);

/// A mute button's face in `m`: "M", grey when `muted`.
pub(super) fn mute_face(painter: &Painter, m: Rect, muted: bool, hovered: bool) {
    let fill = if muted {
        MUTE_COL
    } else if hovered {
        S03
    } else {
        S02
    };
    painter.rect_filled(m, 2.0, fill);
    painter.text(m.center(), Align2::CENTER_CENTER, "M", font(11.0), if muted { Color32::BLACK } else { white(142) });
}

/// A small headphones icon.
fn headphones(p: &Painter, c: Pos2, ink: Color32) {
    let pts: Vec<Pos2> = (0..=12).map(|i| c + Vec2::angled(std::f32::consts::PI + std::f32::consts::PI * i as f32 / 12.0) * 6.0).collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.5, ink)));
    for dx in [-6.0, 4.0] {
        p.rect_filled(Rect::from_min_size(c + Vec2::new(dx, -1.0), Vec2::new(2.5, 6.0)), 1.0, ink);
    }
}

/// Which optional parts of the strips the Mixer shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerSection {
    Extended,
    Io,
    Plugins,
    Aux,
}

#[derive(Debug, Clone, Copy)]
pub struct MixerSections {
    pub extended: bool,
    pub io: bool,
    pub plugins: bool,
    pub aux: bool,
}

impl Default for MixerSections {
    fn default() -> Self {
        MixerSections { extended: true, io: false, plugins: true, aux: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_matches_maschine() {
        for (db, y) in [(10.0, 3.0), (0.0, 25.0), (-10.0, 47.0), (-20.0, 63.0), (-30.0, 79.0)] {
            assert!((db_y(db) - y).abs() < 1e-3, "{db} dB");
            assert!((y_db(y) - db).abs() < 0.01, "y {y}");
        }
        assert_eq!(y_db(95.0), f32::NEG_INFINITY);
    }
}
