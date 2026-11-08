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
/// The group tabs over the strips: the row's height and a tab's width.
const TABS_H: f32 = 28.0;
const TAB_W: f32 = 96.0;
/// An extra-output strip's mode menu (stereo, mono, split), under its label: its height.
const MODE_H: f32 = 17.0;

/// A plug-in's extra-output strip: its tint, its dashed edge, and the plug-in's name.
const EXTRA_BG: Color32 = Color32::from_rgb(0x1F, 0x2A, 0x33);
const EXTRA_EDGE: Color32 = Color32::from_rgb(0x4A, 0x90, 0xC2);
const EXTRA_INK: Color32 = Color32::from_rgb(0x7F, 0xC0, 0xEE);

/// Whose channel a strip is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Group(usize),
    Sound(usize, usize),
    Master,
    /// A plug-in's extra output: strip `.0` of `project.extra_outs`.
    Extra(usize),
}

impl Channel {
    /// Its document target (an extra output's strip has none of its own: the project,
    /// which no channel edit takes; its edits are [`Act::SetExtraOut`]).
    fn target(self) -> Target {
        match self {
            Channel::Group(g) => Target::Group(g),
            Channel::Sound(g, s) => Target::Sound(g, s),
            Channel::Master => Target::Song,
            Channel::Extra(_) => Target::Project,
        }
    }
}

/// What a strip shows.
struct Strip {
    ch: Channel,
    id: String,
    name: String,
    /// The plug-in's name, over `name` (an extra output's strip; else empty).
    plugin: String,
    /// The owner's full name (an extra output's strip; else empty): its ID is `id`.
    owner: String,
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
    /// An extra output's mode menu (see [`mode_field`]).
    mode: Option<IoField>,
}

/// What a pick in a routing list does.
#[derive(Clone)]
enum Pick {
    Route(RouteSet),
    Param(Target, &'static str, f64),
    /// Where an extra output's strip goes.
    Extra(usize, deckard_model::AudioDest),
    /// An extra output's strip: stereo or mono, split, or joined (a split half).
    ExtraChannels(usize, deckard_model::extra_out::Channels),
    Split(usize),
    Join(usize),
}

/// What a pick asks for.
fn pick_act(p: Pick) -> Act {
    match p {
        Pick::Route(r) => Act::SetRoute(r),
        Pick::Param(t, field, v) => Act::SetParam(t, field, v),
        Pick::Extra(i, d) => Act::SetExtraOut(i, ExtraOutEdit::Dest(d)),
        Pick::ExtraChannels(i, ch) => Act::SetExtraOut(i, ExtraOutEdit::Channels(ch)),
        Pick::Split(i) => Act::SplitExtraOut(i),
        Pick::Join(i) => Act::JoinExtraOut(i),
    }
}

/// Extra-output strip `i` (`x`)'s mode menu: Stereo, Mono or Split; on a split half,
/// Join (back into one stereo strip, with the left half's settings).
fn mode_field(x: &deckard_model::extra_out::ExtraOut, i: usize) -> IoField {
    use deckard_model::extra_out::Channels;
    match x.channels {
        Channels::Stereo | Channels::Mono => IoField {
            text: if x.channels == Channels::Mono { "MONO" } else { "STEREO" }.into(),
            names: vec!["Stereo".into(), "Mono".into(), "Split".into()],
            picks: vec![Pick::ExtraChannels(i, Channels::Stereo), Pick::ExtraChannels(i, Channels::Mono), Pick::Split(i)],
            cur: usize::from(x.channels == Channels::Mono),
        },
        Channels::Left | Channels::Right => IoField {
            text: if x.channels == Channels::Left { "SPLIT L" } else { "SPLIT R" }.into(),
            names: vec!["Join".into()],
            picks: vec![Pick::Join(i)],
            cur: usize::MAX,
        },
    }
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

/// Extra-output strip `i`'s output selector. One on an output that isn't connected
/// says where it plays instead: its parent (a sound's plug-in's, the group; else the
/// master).
fn strip_out_field(c: &channel::Ctx, i: usize) -> IoField {
    use deckard_model::AudioDest;
    use deckard_model::extra_out::Owner;
    let x = &c.p.extra_outs[i];
    let short = |n: &str| channel::abbrev(n.split(" - ").next().unwrap_or(n));
    let mut options = channel::strip_destinations(c, i);
    let cur = options.iter().position(|(_, d)| *d == x.dest);
    let fallback = match x.dest {
        // (Past the engine's outputs is never connected, nor asked about: a huge pair
        // would overflow the routing's sums.)
        AudioDest::Ext(n) if n < 0 || n as usize >= deckard_engine::OUTPUTS || !c.io.output_pair_on(n as usize) => {
            let parent = match x.owner {
                Owner::Sound(g, _) | Owner::Chain(g, _, _) if g < c.p.groups.len() => group_name(c.p, g),
                _ => "Master".to_string(),
            };
            Some(format!("Out {} (not connected) → {parent}", n.saturating_add(1)))
        }
        _ => None,
    };
    let text = match (&fallback, cur) {
        (Some(f), _) => f.clone(),
        (None, Some(k)) => short(&options[k].0),
        // Its pick leads back into it now: shown, not offered.
        (None, None) => channel::strip_dest_name(c, i, x.dest).map_or_else(|| "?".to_string(), |n| short(&n)),
    };
    if let (Some(f), Some(k)) = (fallback, cur) {
        options[k].0 = f;
    }
    let (names, dests): (Vec<String>, Vec<AudioDest>) = options.into_iter().unzip();
    IoField { text, names, picks: dests.into_iter().map(|d| Pick::Extra(i, d)).collect(), cur: cur.unwrap_or(usize::MAX) }
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
        acts.push(pick_act(f.picks[k].clone()));
    }
}

/// The meters' width and their spacing (the fader's arrow is beside them).
const METER_W: f32 = 13.0;
const METER_STEP: f32 = 14.0;
/// Where modulation has a fader now (the colour of a knob's mark).
const MODULATED: Color32 = Color32::from_rgb(0x9a, 0xd0, 0xff);

/// ⌃-click (right-click) on a fader: its menu, to remove its modulation.
fn remove_modulation(resp: &egui::Response, edit: super::Edit, acts: &mut Vec<Act>) {
    if modulation::mod_target(edit).is_none() {
        return;
    }
    resp.context_menu(|ui| {
        if ui.button("Remove Modulation").on_hover_text("Remove this fader's modulation from every pattern").clicked() {
            acts.push(Act::ClearModulation(edit));
            ui.close();
        }
    });
}

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
        midi_input: &|g| app.midi_input(g),
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
        let y = rect.top() + TABS_H + 25.0 + i as f32 * 24.0 + if i > 0 { 8.0 } else { 0.0 };
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
    group_tabs(
        ui,
        app,
        p,
        &painter,
        Rect::from_min_max(Pos2::new(side.right(), rect.top()), Pos2::new(rect.right(), rect.top() + TABS_H)),
        acts,
    );
    // Under the tabs: the strips and the fixed column.
    let rect = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + TABS_H), rect.max);
    let held = |ch: Channel| app.peaks.get(&ch).map_or((0.0, 0.0), |p| (p[0].0, p[1].0));
    let gmeter = |g: usize| app.meters.get(g).map_or((0.0, 0.0), |row| row.iter().fold((0f32, 0f32), |m, v| (m.0.max(v.0), m.1.max(v.1))));
    // A plug-in's extra outputs: each strip after its channel's (the master's before the
    // master), shown where that channel is.
    use deckard_model::extra_out::Owner;
    let extra_strip = |i: usize| -> Strip {
        let x = &p.extra_outs[i];
        let plugin = deckard_model::extra_out::plugin_at(p, x.owner)
            .filter(|pr| pr.id == x.plugin && !pr.name.is_empty())
            .map_or_else(|| x.plugin.clone(), |pr| pr.name.clone());
        let lin = if x.level.is_finite() { x.level.clamp(0.0, 4.0) } else { 0.0 };
        // Its owner: the group's ID as its header shows it (A1), a sound's (A1/3), or M
        // for the master.
        let letter = group_id;
        let sound = |g: usize, s: usize| {
            let n = p.groups.get(g).and_then(|gr| gr.sounds.get(s)).map(|sn| sn.name.clone()).filter(|n| !n.is_empty());
            n.unwrap_or_else(|| format!("Sound {}", s + 1))
        };
        let (oid, oname) = match x.owner {
            Owner::Group(g, _) => (letter(g), if g < p.groups.len() { group_name(p, g) } else { letter(g) }),
            Owner::Sound(g, s) | Owner::Chain(g, s, _) => (format!("{}/{}", letter(g), s + 1), sound(g, s)),
            Owner::Master(_) => ("M".to_string(), "Master".to_string()),
        };
        Strip {
            ch: Channel::Extra(i),
            id: oid,
            name: crate::extra_outs::label(x),
            plugin,
            owner: oname,
            color: EXTRA_EDGE,
            level: if lin > 0.0 { deckard_model::level_from_db(20.0 * lin.log10()) } else { 0.0 },
            pan: if x.pan.is_finite() { x.pan.clamp(-1.0, 1.0) } else { 0.0 },
            muted: x.muted,
            cue: false,
            focused: false,
            meter: app.extra_meters.get(i).copied().unwrap_or((0.0, 0.0)),
            peak: held(Channel::Extra(i)),
            slots: Vec::new(),
            io: [None, None, Some(strip_out_field(&ctx, i)), None, None, None],
            aux: [0.0; 2],
            mode: Some(mode_field(x, i)),
        }
    };
    let mut strips: Vec<Strip> = Vec::new();
    if sounds {
        if let Some(gr) = p.groups.get(app.group) {
            for (s, snd) in gr.sounds.iter().enumerate() {
                strips.push(Strip {
                    ch: Channel::Sound(app.group, s),
                    id: (s + 1).to_string(),
                    name: if snd.name.is_empty() { format!("Sound {}", s + 1) } else { snd.name.clone() },
                    plugin: String::new(),
                    owner: String::new(),
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
                    mode: None,
                });
            }
        }
    } else {
        for (g, gr) in p.groups.iter().enumerate() {
            strips.push(Strip {
                ch: Channel::Group(g),
                id: group_id(g),
                name: group_name(p, g),
                plugin: String::new(),
                owner: String::new(),
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
                mode: None,
            });
        }
    }
    if !sounds {
        // Every extra output, just left of the master: by owner in mixer order (a group's,
        // then its sounds'; the master's last), each owner's in `extra_outs` order.
        let rank = |o: Owner| match o {
            Owner::Group(g, _) => (g, 0),
            Owner::Sound(g, s) | Owner::Chain(g, s, _) => (g, s.saturating_add(1)),
            Owner::Master(_) => (usize::MAX, 0),
        };
        let mut all: Vec<usize> = (0..p.extra_outs.len()).collect();
        all.sort_by_key(|&i| rank(p.extra_outs[i].owner));
        strips.extend(all.into_iter().map(extra_strip));
    }
    // The fixed right column: the master, or the focused group at sound level.
    let right = Rect::from_min_max(Pos2::new(rect.right() - MASTER_W, rect.top()), rect.max);
    let fixed = if sounds && let Some(gr) = p.groups.get(app.group) {
        Strip {
            ch: Channel::Group(app.group),
            id: group_id(app.group),
            name: group_name(p, app.group),
            plugin: String::new(),
            owner: String::new(),
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
            mode: None,
        }
    } else {
        Strip {
            ch: Channel::Master,
            id: String::new(),
            name: "Master".into(),
            plugin: String::new(),
            owner: String::new(),
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
            mode: None,
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

/// The group tabs, in `row`: a tab per group (its ID and name, in its colour), the
/// focused group's selected at sound level. Click: that group's sounds; double-click:
/// the groups. The row scrolls on its own (wheel over it) and brings the focused tab
/// into view when the focus moves.
fn group_tabs(ui: &mut Ui, app: &App, p: &deckard_model::Project, painter: &Painter, row: Rect, acts: &mut Vec<Act>) {
    painter.rect_filled(row, 0.0, S05);
    painter.hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, Color32::BLACK));
    let n = p.groups.len();
    let tabs = row.shrink2(Vec2::new(4.0, 0.0));
    let mut off = hscroll(ui, "mixer-tabs", tabs, n as f32 * TAB_W);
    // The focus moved (or the level did): its tab into view.
    let seen_id = ui.id().with("mixer-tabs-seen");
    let now = (app.group, app.mixer_sounds);
    if ui.data(|d| d.get_temp::<(usize, bool)>(seen_id)) != Some(now) && app.group < n {
        let (l, r) = (app.group as f32 * TAB_W, (app.group + 1) as f32 * TAB_W);
        off = off.min(l).max(r - tabs.width()).clamp(0.0, (n as f32 * TAB_W - tabs.width()).max(0.0));
        ui.data_mut(|d| {
            d.insert_temp(ui.id().with("mixer-tabs"), off);
            d.insert_temp(seen_id, now);
        });
    }
    let clip = painter.with_clip_rect(tabs.intersect(painter.clip_rect()));
    let mut drawn = Vec::new();
    for g in 0..n {
        let r = Rect::from_min_size(Pos2::new(tabs.left() + g as f32 * TAB_W - off, row.top()), Vec2::new(TAB_W, TABS_H));
        if r.right() < tabs.left() || r.left() > tabs.right() {
            continue;
        }
        let selected = app.mixer_sounds && g == app.group;
        let resp = ui
            .interact(r.intersect(tabs), ui.id().with(("mixer-tab", g)), Sense::click())
            .on_hover_text(format!("{}\nClick: show the group's sounds. Double-click: show the groups", group_name(p, g)));
        let col = group_color(p, g);
        let inner = r.shrink2(Vec2::new(1.0, 4.0));
        let fill = if selected {
            col
        } else if resp.hovered() {
            S03
        } else {
            S01
        };
        clip.rect_filled(inner, 2.0, fill);
        let (id_ink, name_ink) = if selected { (black(245), black(245)) } else { (col, white(200)) };
        let text = clip.with_clip_rect(inner.intersect(tabs));
        text.text(Pos2::new(inner.left() + 6.0, inner.center().y), Align2::LEFT_CENTER, group_id(g), font(12.0), id_ink);
        let full = group_name(p, g);
        let name = elide_middle(painter, &full, 11.0, inner.right() - 4.0 - (inner.left() + 28.0));
        text.text(Pos2::new(inner.left() + 28.0, inner.center().y), Align2::LEFT_CENTER, &name, font(11.0), name_ink);
        if resp.double_clicked() {
            acts.push(Act::MixerSounds(false));
        } else if resp.clicked() {
            acts.push(Act::FocusGroup(g));
            acts.push(Act::MixerSounds(true));
        }
        drawn.push((g, r, selected));
    }
    ui.data_mut(|d| d.insert_temp(egui::Id::new("deckard-mixer-tabs"), drawn));
}

/// One channel strip in `r` (clipped to `clip`).
#[allow(clippy::too_many_arguments)]
fn strip(ui: &mut Ui, app: &App, painter: &Painter, r: Rect, clip: Rect, st: &Strip, sounds: bool, acts: &mut Vec<Act>) {
    let master = st.ch == Channel::Master;
    // A plug-in's extra output: tinted, with a dashed edge.
    let extra = match st.ch {
        Channel::Extra(i) => Some(i),
        _ => None,
    };
    let key = st.ch;
    let id = ui.id().with(("mix", format!("{key:?}")));
    let vis = |x: Rect| x.intersect(clip);
    // Body.
    let body = Rect::from_min_max(Pos2::new(r.left(), r.top() + HEAD + 4.0), r.max);
    let body_fill = match (extra, st.focused) {
        (Some(_), _) => EXTRA_BG,
        (None, true) => Color32::from_rgb(0x14, 0x14, 0x14),
        (None, false) => S05,
    };
    painter.rect_filled(if extra.is_some() { r } else { body }, 0.0, body_fill);
    painter.vline(r.right() + 0.5, r.y_range(), Stroke::new(1.0, Color32::BLACK));

    // Header: ID and mini meter, then the name.
    let head = Rect::from_min_size(r.min, Vec2::new(r.width(), HEAD));
    let mute_col = MUTE_COL;
    let (bg, id_ink, name_ink) = match (st.focused && !master, st.muted) {
        _ if extra.is_some() => (EXTRA_BG, EXTRA_INK, if st.muted { white(142) } else { white(245) }),
        (true, false) => (st.color, Color32::from_black_alpha(245), Color32::from_black_alpha(245)),
        (true, true) => (mute_col, Color32::from_black_alpha(245), Color32::from_black_alpha(245)),
        (false, true) => (if sounds { S00 } else { S05 }, mute_col, white(142)),
        (false, false) => (if matches!(st.ch, Channel::Sound(..)) { S00 } else { S05 }, st.color, white(245)),
    };
    painter.rect_filled(head, 0.0, bg);
    let row1 = Rect::from_min_size(Pos2::new(head.left() + 4.0, head.top()), Vec2::new(37.0, 20.0));
    if master {
        painter.text(Pos2::new(head.left() + 4.0, head.top() + 10.0), Align2::LEFT_CENTER, "MASTER", font(11.0), white(245));
    } else if extra.is_some() {
        // The plug-in's name, small, over the output's.
        let pr = Rect::from_min_size(row1.min, Vec2::new(r.width() - 14.0, row1.height()));
        let top = elide_middle(painter, &format!("{}\u{b7}{}", st.id, st.plugin), 9.0, pr.width());
        painter.with_clip_rect(vis(pr)).text(pr.left_center(), Align2::LEFT_CENTER, &top, font(9.0), id_ink);
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
    let renaming = app.renaming == Some(st.ch.target()) && !master && extra.is_none();
    if renaming {
        let rt = match st.ch {
            Channel::Group(g) => Target::Group(g),
            ch => ch.target(),
        };
        rename_editor(ui, name_r, rt, &st.name, acts);
    } else {
        // A group's name: shortened in the middle (its start and end stay), whole on
        // hover.
        let name =
            if matches!(st.ch, Channel::Group(_)) { elide_middle(painter, &st.name, 12.0, name_r.width() - 2.0) } else { st.name.clone() };
        let p = painter.with_clip_rect(vis(name_r));
        p.text(name_r.left_center(), Align2::LEFT_CENTER, &name, font(12.0), name_ink);
        if name != st.name {
            ui.interact(vis(name_r), id.with("full-name"), Sense::hover()).on_hover_text(&st.name);
        }
    }
    if extra.is_some() {
        ui.interact(vis(head), id.with("head"), Sense::hover())
            .on_hover_text(format!("{}: {}, {} (an extra output of the plug-in)", st.owner, st.plugin, st.name));
    }
    // Header clicks: focus; double-click the ID: mute; the name: rename; elsewhere:
    // switch between groups and sounds.
    if !master && extra.is_none() {
        let resp = ui
            .interact(vis(head), id.with("head"), Sense::click())
            .on_hover_text("Click: focus. Double-click the ID: mute, the name: rename, elsewhere: show the group's sounds (or the groups)");
        if resp.clicked() {
            match st.ch {
                Channel::Group(g) => acts.push(Act::FocusGroup(g)),
                Channel::Sound(_, s) => acts.push(Act::FocusSound(s)),
                Channel::Master | Channel::Extra(_) => {}
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
    // An extra output's mode menu, under its label: in the row its neighbours' MIDI in
    // takes (it has none), so the rows below stay level with theirs.
    if let Some(f) = &st.mode {
        let b = Rect::from_min_size(Pos2::new(r.left() + 3.0, y + 2.0), Vec2::new(r.width() - 6.0, MODE_H));
        io_button(ui, painter, b.intersect(clip), id.with("mode"), f, acts);
    }
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
                let chain = deckard_doc::Chain::Sound(g, s);
                let bypass = ui.interact(vis(sq), id.with(("bypass", i)), Sense::click()).on_hover_text("Bypass");
                if bypass.clicked() {
                    acts.push(Act::SlotActiveOf(g, s, i, !slot.active));
                }
                bypass.context_menu(|ui| super::slot_menu(ui, app, chain, i, slot.is_instrument(), acts));
                let name = Rect::from_min_max(Pos2::new(sq.right(), row.top()), row.max);
                let shown = ui.interact(vis(name), id.with(("slot", i)), Sense::click()).on_hover_text("Show its pages");
                if shown.clicked() {
                    acts.push(Act::FocusSlot(g, s, i));
                }
                shown.context_menu(|ui| super::slot_menu(ui, app, chain, i, slot.is_instrument(), acts));
                y += SLOT_ROW;
            }
            let add = Rect::from_min_size(Pos2::new(r.left(), y), Vec2::new(r.width(), SLOT_ROW - 1.0));
            let resp = ui.interact(vis(add), id.with("add"), Sense::click()).on_hover_text("Add an instrument, effect or plug-in");
            painter.rect_stroke(add, 2.0, Stroke::new(1.0, if resp.hovered() { white(120) } else { white(40) }), StrokeKind::Inside);
            super::plus_glyph(painter, add.center(), 13.0, white(142));
            // The "+" menu adds to the focused chain: focus this sound's.
            let chain = deckard_doc::Chain::Sound(g, s);
            if resp.clicked() && app.chain() != chain {
                acts.push(Act::FocusChainSlot(chain, usize::MAX));
            }
            // A sound has one instrument, in its first slot.
            let instruments = st.slots.is_empty();
            egui::Popup::menu(&resp).show(|ui| super::add_menu(ui, app, instruments, acts));
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
            let bypass = ui.interact(vis(sq), id.with(("cbypass", i)), Sense::click()).on_hover_text("Bypass");
            if bypass.clicked() {
                acts.push(Act::ChainSlotActive(chain, i, !slot.active));
            }
            bypass.context_menu(|ui| super::slot_menu(ui, app, chain, i, slot.is_instrument(), acts));
            let name = Rect::from_min_max(Pos2::new(sq.right(), row.top()), row.max);
            let shown = ui.interact(vis(name), id.with(("cslot", i)), Sense::click()).on_hover_text("Show its pages");
            if shown.clicked() {
                acts.push(Act::FocusChainSlot(chain, i));
            }
            shown.context_menu(|ui| super::slot_menu(ui, app, chain, i, slot.is_instrument(), acts));
            y += SLOT_ROW;
        }
        // "+": add an effect at the end of this chain (the "+" menu, with this chain
        // focused, so the menu offers what it takes and adds to it).
        let add = Rect::from_min_size(Pos2::new(r.left(), y), Vec2::new(r.width(), SLOT_ROW - 1.0));
        let resp = ui.interact(vis(add), id.with("cadd"), Sense::click()).on_hover_text("Add an effect");
        painter.rect_stroke(add, 2.0, Stroke::new(1.0, if resp.hovered() { white(120) } else { white(40) }), StrokeKind::Inside);
        painter.text(add.center(), Align2::CENTER_CENTER, "+", font(13.0), white(142));
        if resp.clicked() && app.chain() != chain {
            acts.push(Act::FocusChainSlot(chain, usize::MAX));
        }
        egui::Popup::menu(&resp).show(|ui| super::add_menu(ui, app, false, acts));
    }
    let _ = y;
    if !app.mixer_sections.extended {
        if extra.is_some() {
            dashed_edge(painter, r.shrink(0.5), EXTRA_EDGE);
        }
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
    // (An extra output's strip has no sends: the space stays, its level module level
    // with its neighbours'.)
    if app.mixer_sections.aux && extra.is_some() {
        bottom -= 62.0;
    }
    if app.mixer_sections.aux && !master && extra.is_none() {
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
            super::value_entry::gestures(
                ui,
                &resp,
                super::value_entry::What::Aux(t, k),
                &format!("{:.1}", deckard_model::level_db(v)),
                acts,
            );
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
    // Modulated: where it has the pan now, a mark.
    if let Some(m) = app.mod_amount(super::Edit::Pan(target)) {
        painter.vline(cx + (m * 2.0 - 1.0) * pan_r.width() / 2.0, pan_r.y_range(), Stroke::new(1.5, MODULATED));
    }
    let pan_resp = ui
        .interact(vis(pan_r.expand2(Vec2::new(0.0, 4.0))), id.with("pan"), Sense::click_and_drag())
        .on_hover_text(format!("Pan {} (drag; double-click: centre)", pan(st.pan)));
    if pan_resp.hovered() || pan_resp.dragged() {
        painter.rect_filled(pan_r, 0.0, white(20));
    }
    // With Auto-write on while the song plays, a drag writes modulation.
    let pan_edit = super::Edit::Pan(target);
    let pan_writes = extra.is_none() && super::knob_drag_writes(app, pan_edit);
    let pan_now = if pan_writes { app.mod_position(pan_edit).map_or(st.pan, |a| a * 2.0 - 1.0) } else { st.pan };
    if pan_resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let d = pan_resp.drag_delta().x / if fine { 300.0 } else { 30.5 };
        if d != 0.0 {
            let to = (pan_now + d as f64).clamp(-1.0, 1.0);
            acts.push(match extra {
                Some(i) => Act::SetExtraOut(i, ExtraOutEdit::Pan(to)),
                None if pan_writes => Act::Modulate(pan_edit, ((to + 1.0) / 2.0) as f32, false),
                None => Act::SetPan(target, to),
            });
        }
    }
    if let Some(i) = extra {
        if pan_resp.double_clicked() || (pan_resp.clicked() && ui.input(|i| i.modifiers.command)) {
            acts.push(Act::SetExtraOut(i, ExtraOutEdit::Pan(0.0)));
        }
    } else {
        if pan_resp.drag_stopped() && pan_writes {
            acts.push(Act::Modulate(pan_edit, ((pan_now + 1.0) / 2.0) as f32, true));
        }
        remove_modulation(&pan_resp, pan_edit, acts);
        super::value_entry::gestures(
            ui,
            &pan_resp,
            super::value_entry::What::Edit(super::Edit::Pan(target)),
            &format!("{:.0}", st.pan * 100.0),
            acts,
        );
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
    // Modulated: where it has the level now, a mark beside the handle.
    let level_y = |level: f64| {
        let d = deckard_model::level_db(level) as f32;
        if d.is_finite() { db_y(d) } else { 95.0 }
    };
    let level_edit = super::Edit::Level(target);
    if let Some(m) = app.mod_amount(level_edit) {
        let my = sm.top() + level_y(f64::from(m));
        painter.hline((hx - 2.0)..=(hx + 6.0), my, Stroke::new(1.5, MODULATED));
    }
    let fader = Rect::from_min_size(Pos2::new(sm.left() + 20.0, sm.top() - 2.0), Vec2::new(r.width() - 20.0, 103.0));
    let fr = ui
        .interact(vis(fader), id.with("fader"), Sense::click_and_drag())
        .on_hover_text(format!("{} (drag; Shift: fine; double-click: 0 dB)", db(st.level)));
    // With Auto-write on while the song plays, a drag writes modulation.
    let level_writes = extra.is_none() && super::knob_drag_writes(app, level_edit);
    let level_now = if level_writes { app.mod_position(level_edit).unwrap_or(st.level) } else { st.level };
    if fr.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let dy = fr.drag_delta().y / if fine { 10.0 } else { 1.0 };
        if dy != 0.0 {
            let d = y_db((level_y(level_now) + dy).clamp(3.0, 95.0));
            acts.push(match extra {
                // A strip's level is linear.
                Some(i) => Act::SetExtraOut(i, ExtraOutEdit::Level(if d.is_finite() { 10f64.powf(d as f64 / 20.0) } else { 0.0 })),
                None if level_writes => Act::Modulate(level_edit, deckard_model::level_from_db(d as f64) as f32, false),
                None => Act::SetLevel(target, deckard_model::level_from_db(d as f64)),
            });
        }
    }
    if let Some(i) = extra {
        if fr.double_clicked() || (fr.clicked() && ui.input(|i| i.modifiers.command)) {
            acts.push(Act::SetExtraOut(i, ExtraOutEdit::Level(1.0)));
        }
    } else {
        if fr.drag_stopped() && level_writes {
            acts.push(Act::Modulate(level_edit, level_now as f32, true));
        }
        remove_modulation(&fr, level_edit, acts);
        super::value_entry::gestures(
            ui,
            &fr,
            super::value_entry::What::Edit(super::Edit::Level(target)),
            &format!("{:.1}", deckard_model::level_db(st.level)),
            acts,
        );
    }
    painter.hline(lm.x_range(), lm.top() + 142.5, Stroke::new(1.0, Color32::BLACK));
    // Cue and mute (right-click mute: solo).
    let row = Rect::from_min_size(Pos2::new(lm.left(), lm.top() + 143.0), Vec2::new(r.width(), 20.0));
    if !master && extra.is_none() {
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
    if let Some(i) = extra {
        let m = Rect::from_min_size(Pos2::new(row.right() - 40.0, row.top() + 2.0), Vec2::new(34.0, 16.0));
        let mr = ui.interact(vis(m), id.with("mute"), Sense::click()).on_hover_text("Mute (right-click: solo)");
        mute_face(painter, m, st.muted, mr.hovered());
        if mr.clicked() {
            acts.push(Act::SetExtraOut(i, ExtraOutEdit::Mute(!st.muted)));
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
    if extra.is_some() {
        dashed_edge(painter, r.shrink(0.5), EXTRA_EDGE);
    }
}

/// A 1px dashed outline of `r`.
fn dashed_edge(painter: &Painter, r: Rect, col: Color32) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, col), 4.0, 3.0));
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

    /// Strip `ch`'s "+" under its `slots` plug-in rows, in the whole app's Mixer.
    fn add_row(h: &egui_kittest::Harness<'static, crate::App>, ch: Channel, slots: usize) -> egui::Pos2 {
        let strips: Vec<(Channel, Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-mixer-strips"))).expect("mixer drawn");
        let r = strips.into_iter().find(|(c, _)| *c == ch).unwrap().1;
        egui::pos2(r.center().x, r.top() + HEAD + 3.0 + 4.0 + slots as f32 * SLOT_ROW + SLOT_ROW / 2.0)
    }

    fn click_at(h: &mut egui_kittest::Harness<'static, crate::App>, pos: egui::Pos2) {
        h.hover_at(pos);
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.run_steps(3);
    }

    /// Every strip's "+" opens the add menu, as the master's always did.
    #[test]
    fn each_strips_plus_opens_the_add_menu() {
        use egui_kittest::kittest::Queryable;
        for (sounds, ch) in [(false, Channel::Master), (false, Channel::Group(1)), (true, Channel::Sound(0, 1)), (true, Channel::Group(0))]
        {
            let Some(mut h) = crate::tests::harness(false, false) else { return };
            h.state_mut().group = 0;
            (h.state_mut().mixer, h.state_mut().mixer_sounds) = (true, sounds);
            h.run_steps(3);
            let n = {
                let doc = h.state().doc.clone().unwrap();
                let d = doc.borrow();
                let p = d.project();
                match ch {
                    Channel::Master => p.master_slots.len(),
                    Channel::Group(g) => p.groups[g].slots.len(),
                    Channel::Sound(g, s) => p.groups[g].sounds[s].slots.len(),
                    Channel::Extra(_) => unreachable!(),
                }
            };
            let at = add_row(&h, ch, n);
            click_at(&mut h, at);
            assert!(h.query_by_label("Internal ⏵").is_some(), "{ch:?}: no menu");
        }
    }

    /// Pick "+" ▸ Internal ▸ Compressor in the open menu.
    fn pick_compressor(h: &mut egui_kittest::Harness<'static, crate::App>) {
        use egui_kittest::kittest::Queryable;
        h.get_by_label("Internal ⏵").click();
        h.run_steps(3);
        h.get_by_label("Compressor").click();
        h.run_steps(3);
    }

    /// A sound strip's "+" adds to that sound, a group strip's to that group, whatever
    /// was focused before.
    #[test]
    fn a_strips_plus_adds_to_its_own_chain() {
        let Some(mut h) = crate::tests::harness(false, false) else { return };
        (h.state_mut().mixer, h.state_mut().mixer_sounds) = (true, true);
        h.run_steps(3);
        let doc = h.state().doc.clone().unwrap();
        let n = doc.borrow().project().groups[0].sounds[2].slots.len();
        let at = add_row(&h, Channel::Sound(0, 2), n);
        click_at(&mut h, at);
        pick_compressor(&mut h);
        assert_eq!(doc.borrow().project().groups[0].sounds[2].slots.len(), n + 1, "{}", h.state().status);
        assert_eq!(h.state().sound, 2, "that sound focused");
        assert!(!h.state().channel, "its plug-ins shown");

        h.state_mut().mixer_sounds = false;
        h.run_steps(3);
        let n = doc.borrow().project().groups[3].slots.len();
        let at = add_row(&h, Channel::Group(3), n);
        click_at(&mut h, at);
        pick_compressor(&mut h);
        assert_eq!(doc.borrow().project().groups[3].slots.len(), n + 1, "{}", h.state().status);
    }

    /// An empty sound's "+" offers instruments.
    #[test]
    fn an_empty_sounds_plus_offers_instruments() {
        use egui_kittest::kittest::Queryable;
        let mut app = crate::App::default();
        app.new_project();
        (app.mixer, app.mixer_sounds) = (true, true);
        let doc = app.doc.clone().unwrap();
        assert!(doc.borrow().project().groups[0].sounds[1].slots.is_empty());
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1500.0, 930.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app: &mut crate::App| app.frame(ui), app);
        h.run_steps(3);
        let at = add_row(&h, Channel::Sound(0, 1), 0);
        click_at(&mut h, at);
        h.get_by_label("Internal ⏵").click();
        h.run_steps(3);
        h.get_by_label("Bass Synth").click();
        h.run_steps(3);
        assert_eq!(doc.borrow().project().groups[0].sounds[1].slots[0].module, "BassSynth", "{}", h.state().status);
    }

    fn right_click_at(h: &mut egui_kittest::Harness<'static, crate::App>, pos: egui::Pos2) {
        h.hover_at(pos);
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.run_steps(3);
    }

    /// Right-click ▸ Remove takes the row's own effect out, whichever chain is focused.
    #[test]
    fn a_rows_right_click_removes_it_from_its_own_chain() {
        use egui_kittest::kittest::Queryable;
        let Some(mut h) = crate::tests::harness(false, false) else { return };
        (h.state_mut().mixer, h.state_mut().mixer_sounds) = (true, false);
        h.run_steps(3);
        let doc = h.state().doc.clone().unwrap();
        let (focused, master) = {
            let d = doc.borrow();
            (d.project().groups[0].sounds[0].slots.len(), d.project().master_slots.iter().map(|x| x.name.clone()).collect::<Vec<_>>())
        };
        assert!(master.len() >= 2, "{master:?}");
        let at = add_row(&h, Channel::Master, 0);
        right_click_at(&mut h, at);
        h.get_by_label("Remove").click();
        h.run_steps(3);
        let d = doc.borrow();
        let left: Vec<_> = d.project().master_slots.iter().map(|x| x.name.clone()).collect();
        assert_eq!(left, master[1..], "{}", h.state().status);
        assert_eq!(d.project().groups[0].sounds[0].slots.len(), focused, "the focused chain is untouched");
    }

    /// Right-click ▸ Change Effect swaps the row's module, in place.
    #[test]
    fn a_rows_right_click_changes_its_effect_in_place() {
        use egui_kittest::kittest::Queryable;
        let Some(mut h) = crate::tests::harness(false, false) else { return };
        (h.state_mut().mixer, h.state_mut().mixer_sounds, h.state_mut().group, h.state_mut().sound) = (true, true, 0, 0);
        h.state_mut().chain_tab = crate::layout::ChainTab::Master;
        h.run_steps(3);
        let doc = h.state().doc.clone().unwrap();
        let modules = |doc: &std::rc::Rc<std::cell::RefCell<deckard_doc::Document>>| -> Vec<String> {
            doc.borrow().project().groups[0].sounds[0].slots.iter().map(|x| x.module.clone()).collect()
        };
        assert_eq!(modules(&doc), ["Sampler", "EQ", "Compressor"]);
        let at = add_row(&h, Channel::Sound(0, 0), 1);
        right_click_at(&mut h, at);
        h.get_by_label("Change Effect ⏵").click();
        h.run_steps(3);
        pick_compressor(&mut h);
        assert_eq!(modules(&doc), ["Sampler", "Compressor", "Compressor"], "{}", h.state().status);
        assert_eq!(h.state().slot, Some(1), "the new one shown");
        doc.borrow_mut().undo();
        assert_eq!(modules(&doc), ["Sampler", "EQ", "Compressor"], "one undo step restores it");
    }

    #[test]
    fn scale_matches_maschine() {
        for (db, y) in [(10.0, 3.0), (0.0, 25.0), (-10.0, 47.0), (-20.0, 63.0), (-30.0, 79.0)] {
            assert!((db_y(db) - y).abs() < 1e-3, "{db} dB");
            assert!((y_db(y) - db).abs() < 0.01, "y {y}");
        }
        assert_eq!(y_db(95.0), f32::NEG_INFINITY);
    }

    use deckard_model::AudioDest;
    use deckard_model::extra_out::{ExtraOut, Owner};

    fn out(owner: Owner, name: &str, dest: AudioDest) -> ExtraOut {
        ExtraOut {
            owner,
            bus: 1,
            plugin: "Portal".into(),
            name: name.into(),
            level: 1.0,
            pan: 0.0,
            muted: false,
            dest,
            channels: Default::default(),
        }
    }

    /// A new project with three groups and the strips `outs`.
    fn app_with(outs: Vec<ExtraOut>) -> crate::App {
        let mut app = crate::App { settings: crate::settings::Settings::default(), ..crate::App::default() };
        app.new_project();
        let doc = app.doc.clone().unwrap();
        while doc.borrow().project().groups.len() < 3 {
            doc.borrow_mut().add_group().unwrap();
        }
        doc.borrow_mut().set_extra_outs(outs).unwrap();
        app
    }

    /// The Mixer alone, drawn in a wide window: what it asked for, the strips it drew
    /// (in order) and the text it painted.
    struct St {
        app: crate::App,
        acts: Vec<Act>,
        strips: Vec<(Channel, Rect)>,
        /// The group tabs: group, where, whether selected.
        tabs: Vec<(usize, Rect, bool)>,
        text: String,
        /// Apply what it asks for after each frame (as the app does), noting whether all
        /// of it only set strip 0's level.
        apply: bool,
        applied: usize,
        only_level: bool,
    }

    fn harness(app: crate::App) -> egui_kittest::Harness<'static, St> {
        let st = St {
            app,
            acts: Vec::new(),
            strips: Vec::new(),
            tabs: Vec::new(),
            text: String::new(),
            apply: false,
            applied: 0,
            only_level: true,
        };
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(2200.0, 760.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, st: &mut St| {
                let doc = st.app.doc.clone().unwrap();
                let rect = ui.max_rect();
                mixer(ui, &st.app, &doc.borrow(), rect, &mut st.acts);
                if st.apply {
                    let acts = std::mem::take(&mut st.acts);
                    st.applied += acts.len();
                    st.only_level &= acts.iter().all(|a| matches!(a, Act::SetExtraOut(0, ExtraOutEdit::Level(_))));
                    apply_acts(&mut st.app, acts);
                }
                st.strips = ui.data(|d| d.get_temp(egui::Id::new("deckard-mixer-strips"))).unwrap_or_default();
                st.tabs = ui.data(|d| d.get_temp(egui::Id::new("deckard-mixer-tabs"))).unwrap_or_default();
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
        h
    }

    fn order(h: &egui_kittest::Harness<'static, St>) -> Vec<Channel> {
        h.state().strips.iter().map(|(c, _)| *c).collect()
    }

    #[test]
    fn all_strips_sit_before_the_master_in_group_order() {
        let mut app = app_with(vec![
            out(Owner::Master(0), "Bus 0", AudioDest::Ext(1)),
            out(Owner::Chain(1, 0, 0), "Bus 1", AudioDest::Parent),
            out(Owner::Sound(0, 1), "Bus 2", AudioDest::Parent),
            out(Owner::Group(1, 0), "Bus 3", AudioDest::Parent),
            out(Owner::Group(0, 0), "Bus 4", AudioDest::Parent),
            out(Owner::Group(0, 0), "Bus 5", AudioDest::Parent),
            out(Owner::Sound(0, 0), "Bus 6", AudioDest::Parent),
        ]);
        app.mixer_sounds = false;
        let h = harness(app);
        let o = order(&h);
        let e = |i| Channel::Extra(i);
        assert_eq!(
            o,
            vec![Channel::Group(0), Channel::Group(1), Channel::Group(2), e(4), e(5), e(6), e(2), e(3), e(1), e(0), Channel::Master],
            "{o:?}"
        );
    }

    #[test]
    fn no_strips_in_the_sounds_mixer() {
        let mut app = app_with(vec![
            out(Owner::Chain(0, 1, 0), "Bus 1", AudioDest::Parent),
            out(Owner::Group(0, 0), "Bus 2", AudioDest::Parent),
            out(Owner::Master(0), "Bus 3", AudioDest::Parent),
        ]);
        (app.mixer_sounds, app.group) = (true, 0);
        let h = harness(app);
        let o = order(&h);
        assert!(o.iter().all(|c| !matches!(c, Channel::Extra(_))), "{o:?}");
    }

    #[test]
    fn the_strip_names_its_owner() {
        let mut app = app_with(vec![
            out(Owner::Group(1, 0), "Bus 1", AudioDest::Parent),
            out(Owner::Sound(0, 2), "Bus 2", AudioDest::Parent),
            out(Owner::Master(0), "Bus 3", AudioDest::Parent),
        ]);
        app.mixer_sounds = false;
        let h = harness(app);
        let text = &h.state().text;
        assert!(text.contains("|B1\u{b7}Portal|") && text.contains("|A1/3\u{b7}Portal|") && text.contains("|M\u{b7}Portal|"), "{text}");
    }

    #[test]
    fn dragging_a_strip_fader_sets_only_its_level() {
        let mut app = app_with(vec![out(Owner::Group(0, 0), "Bus 1", AudioDest::Ext(1))]);
        app.mixer_sounds = false;
        let mut h = harness(app);
        let before = h.state().app.doc.clone().unwrap().borrow().project().clone();
        h.state_mut().apply = true;
        let r = h.state().strips.iter().find(|(c, _)| *c == Channel::Extra(0)).unwrap().1;
        // The 0 dB line of the fader (as the group strips' test drags it).
        let at = egui::pos2(r.left() + 48.0, r.bottom() - LEVEL_H + 26.0 + 25.0);
        h.hover_at(at);
        h.run_steps(1);
        h.drag_at(at);
        h.run_steps(1);
        for k in 1..=4 {
            h.hover_at(at + Vec2::new(0.0, 5.0 * k as f32));
            h.run_steps(1);
        }
        h.drop_at(at + Vec2::new(0.0, 20.0));
        h.run_steps(1);
        assert!(h.state().applied > 0, "the drag asked for something");
        assert!(h.state().only_level, "only its level");
        let after = h.state().app.doc.clone().unwrap().borrow().project().clone();
        let x = &after.extra_outs[0];
        assert!(x.level < 0.5 && x.level > 0.0, "about -10 dB: {}", x.level);
        assert_eq!((x.pan, x.muted, x.dest), (0.0, false, AudioDest::Ext(1)));
        assert_eq!(before.groups.iter().map(|g| g.level).collect::<Vec<_>>(), after.groups.iter().map(|g| g.level).collect::<Vec<_>>());
    }

    /// The output picker's choices for strip `i` of `p` (connected outputs: pair 0 and
    /// 1).
    fn choices(doc: &deckard_doc::Document, p: &deckard_model::Project, i: usize) -> Vec<AudioDest> {
        let mut io = crate::settings::Settings::default();
        io.output_routing[2..4].copy_from_slice(&[Some(2), Some(3)]);
        let groove = |_: usize, _: usize, _: &str| None;
        let c = channel::Ctx {
            doc,
            has_midi_macro: false,
            p,
            midi_ins: &[],
            midi_outs: &[],
            polyphonic: false,
            groove: &groove,
            io: &io,
            plugin_knob: &|_| None,
            midi_input: &|_| None,
        };
        channel::strip_destinations(&c, i).into_iter().map(|(_, d)| d).collect()
    }

    #[test]
    fn a_master_strip_offers_only_outputs() {
        let app = app_with(vec![out(Owner::Master(0), "Bus 1", AudioDest::Ext(1))]);
        let doc = app.doc.clone().unwrap();
        let d = doc.borrow();
        let all = choices(&d, d.project(), 0);
        assert_eq!(all, vec![AudioDest::None, AudioDest::Ext(0), AudioDest::Ext(1)], "{all:?}");
    }

    #[test]
    fn a_strip_cannot_route_back_into_its_owner() {
        let app = app_with(vec![out(Owner::Group(0, 0), "Bus 1", AudioDest::Ext(1))]);
        let doc = app.doc.clone().unwrap();
        let d = doc.borrow();
        let mut p = d.project().clone();
        // Group B goes into Group A: A's strip can't go to B (nor A itself).
        p.groups[1].channel.dest = AudioDest::Group(0);
        let all = choices(&d, &p, 0);
        assert!(!all.contains(&AudioDest::Group(0)) && !all.contains(&AudioDest::Group(1)), "{all:?}");
        assert!(all.contains(&AudioDest::Group(2)) && all.contains(&AudioDest::Parent) && all.contains(&AudioDest::Ext(1)), "{all:?}");
        // Through an aux send and another strip, too: C sends to B's input, and a strip
        // of C's goes... C's own aux into A.
        p.groups[1].channel.dest = AudioDest::Parent;
        p.groups[2].channel.aux_dest[0] = AudioDest::Group(0);
        p.extra_outs.push(out(Owner::Group(1, 0), "Bus 2", AudioDest::Group(2)));
        let all = choices(&d, &p, 0);
        assert!(!all.contains(&AudioDest::Group(1)) && !all.contains(&AudioDest::Group(2)), "{all:?}");
        // A sound's strip: not into a bus sound that leads back to that sound.
        p.extra_outs = vec![out(Owner::Sound(1, 0), "Bus 1", AudioDest::Parent)];
        p.groups[2].channel.aux_dest[0] = AudioDest::None;
        p.groups[1].channel.dest = AudioDest::Sound(1, 0);
        let all = choices(&d, &p, 0);
        assert!(!all.contains(&AudioDest::Parent) && !all.contains(&AudioDest::Group(1)), "its group leads back: {all:?}");
    }

    #[test]
    fn an_unconnected_output_shows_the_fallback() {
        let app = app_with(vec![
            out(Owner::Group(0, 0), "Bus 1", AudioDest::Ext(5)),
            out(Owner::Sound(1, 0), "Bus 2", AudioDest::Ext(7)),
            out(Owner::Group(0, 0), "Bus 3", AudioDest::Ext(0)),
        ]);
        let doc = app.doc.clone().unwrap();
        let d = doc.borrow();
        let io = crate::settings::Settings::default();
        let groove = |_: usize, _: usize, _: &str| None;
        let c = channel::Ctx {
            doc: &d,
            has_midi_macro: false,
            p: d.project(),
            midi_ins: &[],
            midi_outs: &[],
            polyphonic: false,
            groove: &groove,
            io: &io,
            plugin_knob: &|_| None,
            midi_input: &|_| None,
        };
        let f = strip_out_field(&c, 0);
        assert_eq!(f.text, "Out 6 (not connected) → Master");
        assert_eq!(f.names[f.cur], f.text, "the list says so too");
        let group = group_name(d.project(), 1);
        assert_eq!(strip_out_field(&c, 1).text, format!("Out 8 (not connected) → {group}"));
        assert_eq!(strip_out_field(&c, 2).text, "Ext. 1");
        // An output past any there is (a damaged file): not connected, no overflow.
        let mut p = d.project().clone();
        p.extra_outs[2].dest = AudioDest::Ext(i64::MAX);
        let c = channel::Ctx { p: &p, ..c };
        assert_eq!(strip_out_field(&c, 2).text, format!("Out {} (not connected) → Master", i64::MAX));
    }

    /// Where strip `ch`'s mode menu is (under its label).
    fn menu_at(h: &egui_kittest::Harness<'static, St>, ch: Channel) -> egui::Pos2 {
        let r = h.state().strips.iter().find(|(c, _)| *c == ch).unwrap().1;
        egui::pos2(r.left() + 20.0, r.top() + HEAD + 3.0 + 4.0 + 2.0 + MODE_H / 2.0)
    }

    #[test]
    fn splitting_a_strip_shows_two_strips_next_to_each_other() {
        // Real plug-ins in the owners' slots: undo drops strips whose plug-in isn't there.
        const FX: &str = "T:VST3:FX:Portal";
        let mut app = app_with(Vec::new());
        let doc = app.doc.clone().unwrap();
        for g in [0, 1] {
            let fx = deckard_doc::NewModule::Plugin { id: FX.into(), name: "Portal".into(), chunk: Vec::new() };
            doc.borrow_mut().add_slot_in(deckard_doc::Chain::Group(g), 0, &fx).unwrap();
        }
        let mut outs = vec![out(Owner::Group(0, 0), "Kick", AudioDest::Ext(1)), out(Owner::Group(1, 0), "Aux 2", AudioDest::Ext(2))];
        outs.iter_mut().for_each(|x| x.plugin = FX.into());
        doc.borrow_mut().set_extra_outs(outs).unwrap();
        app.mixer_sounds = false;
        let mut h = harness(app);
        assert!(h.state().text.contains("|Out 2 Kick|") && h.state().text.contains("|STEREO|"), "{}", h.state().text);
        h.state_mut().apply = true;
        // The menu under the label: Split.
        use egui_kittest::kittest::Queryable;
        let at = menu_at(&h, Channel::Extra(0));
        h.hover_at(at);
        h.run_steps(1);
        h.drag_at(at);
        h.run_steps(1);
        h.drop_at(at);
        h.run_steps(2);
        h.get_by_label("Split").click();
        h.run_steps(3);
        let o = order(&h);
        assert_eq!(
            &o[..6],
            &[Channel::Group(0), Channel::Group(1), Channel::Group(2), Channel::Extra(0), Channel::Extra(1), Channel::Extra(2)],
            "{o:?}"
        );
        let text = &h.state().text;
        assert!(text.contains("|Out 2 Kick L|") && text.contains("|Out 2 Kick R|") && text.contains("|Out 2|"), "{text}");
        let doc = h.state().app.doc.clone().unwrap();
        let outs = doc.borrow().project().extra_outs.clone();
        use deckard_model::extra_out::Channels;
        assert_eq!(outs.iter().map(|x| x.channels).collect::<Vec<_>>(), vec![Channels::Left, Channels::Right, Channels::Stereo]);
        assert_eq!((outs[1].dest, outs[1].level, outs[1].name.as_str()), (AudioDest::Ext(1), 1.0, "Kick"), "a copy of its settings");
        // One step: undone, one stereo strip again.
        doc.borrow_mut().undo();
        assert_eq!(doc.borrow().project().extra_outs.len(), 2);
        assert_eq!(doc.borrow().project().extra_outs[0].channels, Channels::Stereo);
    }

    #[test]
    fn join_is_offered_only_on_a_split_half() {
        use deckard_model::extra_out::Channels;
        let x = out(Owner::Group(0, 0), "Kick", AudioDest::Ext(1));
        let names = |ch: Channels| mode_field(&ExtraOut { channels: ch, ..x.clone() }, 3).names;
        assert_eq!(names(Channels::Stereo), vec!["Stereo", "Mono", "Split"]);
        assert_eq!(names(Channels::Mono), vec!["Stereo", "Mono", "Split"]);
        assert_eq!(names(Channels::Left), vec!["Join"]);
        assert_eq!(names(Channels::Right), vec!["Join"]);
        // What each pick asks for.
        let acts =
            |ch: Channels| -> Vec<Act> { mode_field(&ExtraOut { channels: ch, ..x.clone() }, 3).picks.into_iter().map(pick_act).collect() };
        let a = acts(Channels::Stereo);
        assert!(matches!(a[1], Act::SetExtraOut(3, ExtraOutEdit::Channels(Channels::Mono))));
        assert!(matches!(a[2], Act::SplitExtraOut(3)));
        assert!(matches!(acts(Channels::Right)[0], Act::JoinExtraOut(3)));
        // Joined from the right half, in the app: the left half's settings, one strip.
        let mut app = app_with(vec![x.clone()]);
        apply_acts(&mut app, vec![Act::SplitExtraOut(0), Act::SetExtraOut(0, ExtraOutEdit::Level(0.5))]);
        apply_acts(&mut app, vec![Act::JoinExtraOut(1)]);
        let outs = app.doc.clone().unwrap().borrow().project().extra_outs.clone();
        assert_eq!(outs.len(), 1);
        assert_eq!((outs[0].channels, outs[0].level), (Channels::Stereo, 0.5));
        // A half's mode doesn't change but by joining.
        apply_acts(&mut app, vec![Act::SplitExtraOut(0), Act::SetExtraOut(1, ExtraOutEdit::Channels(Channels::Mono))]);
        let outs = app.doc.clone().unwrap().borrow().project().extra_outs.clone();
        assert_eq!(outs[1].channels, Channels::Right);
    }

    /// `n` clicks at `pos` (2: a double-click), after a pause (not part of the clicks
    /// before), then what they ask for applied.
    fn clicks(h: &mut egui_kittest::Harness<'static, St>, pos: egui::Pos2, n: usize) {
        use egui::{Event, PointerButton};
        h.state_mut().apply = true;
        h.run_steps(40);
        h.hover_at(pos);
        h.step();
        for _ in 0..n {
            for pressed in [true, false] {
                h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() });
                h.step();
            }
        }
        h.run_steps(2);
    }

    fn tab(h: &egui_kittest::Harness<'static, St>, g: usize) -> Rect {
        h.state().tabs.iter().find(|t| t.0 == g).unwrap_or_else(|| panic!("tab {g} drawn")).1
    }

    #[test]
    fn clicking_a_group_tab_shows_its_sounds() {
        let mut app = app_with(Vec::new());
        (app.mixer_sounds, app.group) = (false, 0);
        let mut h = harness(app);
        let at = tab(&h, 2).center();
        clicks(&mut h, at, 1);
        assert!(h.state().app.mixer_sounds, "its sounds");
        assert_eq!(h.state().app.group, 2);
        assert!(order(&h).contains(&Channel::Sound(2, 0)), "{:?}", order(&h));
        // Another group's tab, at sound level: that group's sounds.
        let at = tab(&h, 1).center();
        clicks(&mut h, at, 1);
        assert!(h.state().app.mixer_sounds);
        assert_eq!(h.state().app.group, 1);
    }

    #[test]
    fn double_clicking_a_tab_shows_the_groups() {
        let mut app = app_with(Vec::new());
        (app.mixer_sounds, app.group) = (true, 1);
        let mut h = harness(app);
        let at = tab(&h, 2).center();
        clicks(&mut h, at, 2);
        assert!(!h.state().app.mixer_sounds, "the groups");
        assert!(order(&h).contains(&Channel::Master), "{:?}", order(&h));
    }

    #[test]
    fn double_clicking_the_fixed_group_column_shows_the_groups() {
        let mut app = app_with(Vec::new());
        (app.mixer_sounds, app.group) = (true, 1);
        let mut h = harness(app);
        let r = h.state().strips.last().filter(|(c, _)| *c == Channel::Group(1)).expect("the group on the right").1;
        // Its header's background (right of the ID).
        clicks(&mut h, egui::pos2(r.left() + 52.0, r.top() + 10.0), 2);
        assert!(!h.state().app.mixer_sounds, "the groups");
        assert_eq!(h.state().app.group, 1, "still focused");
        assert!(!h.state().app.doc.clone().unwrap().borrow().project().groups[1].muted, "not muted");
    }

    #[test]
    fn the_focused_groups_tab_is_selected_at_sound_level() {
        let mut app = app_with(Vec::new());
        (app.mixer_sounds, app.group) = (true, 2);
        let mut h = harness(app);
        let tabs = h.state().tabs.clone();
        assert_eq!(tabs.iter().map(|t| t.0).collect::<Vec<_>>(), vec![0, 1, 2], "a tab per group");
        assert_eq!(tabs.iter().filter(|t| t.2).map(|t| t.0).collect::<Vec<_>>(), vec![2]);
        let text = &h.state().text;
        assert!(text.contains("|A1|") && text.contains("|C1|"), "{text}");
        // The strips start under the tabs.
        let row = tabs[0].1;
        assert!(h.state().strips.iter().all(|(_, r)| r.top() >= row.bottom()), "{:?} under {row:?}", h.state().strips);
        // At group level, none.
        h.state_mut().app.mixer_sounds = false;
        h.run_steps(2);
        assert!(h.state().tabs.iter().all(|t| !t.2), "{:?}", h.state().tabs);
    }

    #[test]
    fn a_long_group_name_is_shortened_in_the_middle() {
        let long = "Drums Acoustic Kit Number 2";
        for sounds in [false, true] {
            let mut app = app_with(Vec::new());
            apply_acts(&mut app, vec![Act::Rename(Target::Group(1), long.into())]);
            (app.mixer_sounds, app.group) = (sounds, 1);
            let h = harness(app);
            let text = &h.state().text;
            assert!(!text.contains(long), "{text}");
            // In the tab, and in the strip's header (the groups) or the fixed column.
            let short: Vec<&str> = text.split('|').filter(|t| t.contains('…')).collect();
            assert!(short.len() >= 2, "{text}");
            for t in short {
                assert!(t.starts_with('D') && t.ends_with('2') && t.matches('…').count() == 1, "{t}");
            }
        }
    }
}
