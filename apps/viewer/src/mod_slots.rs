//! The MK3's mod matrix page (SHIFT + SOLO): every route in the project as a slot of
//! six rows (Source, Group, Sound, Module, Destination, Depth), and their edits
//! ([`deckard_surface::Action::ModSlot`]). A slot without a Destination is a draft,
//! kept on the app until it has one. Past the last group, Group is MIDI: the slot is a
//! CC route ([`CcRoute`]), its rows Port, Channel and CC, made on its first CC.

use crate::App;
use crate::layout::{self, channel, mod_routes};
use deckard_doc::Chain;
use deckard_model::Project;
use deckard_model::lfo::Lfo;
use deckard_model::mod_matrix::{CC_MAX, CcRoute, ModParam, ModRoute, ModSource, choices};
use deckard_surface::ModSlotView;

/// How far a Depth step moves High.
const DEPTH_STEP: f32 = 0.05;
/// What an unset row shows.
const NONE: &str = "—";
/// The Group row of a CC route.
const MIDI: &str = "MIDI";

/// A slot being made: its source, and the group, sound and chain slot chosen so far
/// (or `midi`: a CC route's port and channel so far). `from`: the route (on sound `.0`,
/// `.1`) it was, or `from_cc` the CC route, which stays until a Destination (or CC) is
/// chosen, the draft shown in its place meanwhile.
#[derive(Debug, Clone, PartialEq)]
pub struct ModDraft {
    pub source: ModSource,
    pub group: Option<usize>,
    pub sound: Option<usize>,
    pub slot: Option<usize>,
    pub from: Option<(usize, usize, ModRoute)>,
    pub midi: Option<CcDraft>,
    pub from_cc: Option<CcRoute>,
}

impl ModDraft {
    fn new(source: ModSource) -> ModDraft {
        ModDraft { source, group: None, sound: None, slot: None, from: None, midi: None, from_cc: None }
    }
}

/// A CC route being made: the port and channel (1–16) chosen so far.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CcDraft {
    pub port: Option<String>,
    pub channel: Option<u8>,
}

/// What a slot is: route `.2` of sound (`.0`, `.1`), CC route `.0`, the draft, or the
/// empty one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModSlot {
    Route(usize, usize, usize),
    Cc(usize),
    Draft,
    Empty,
}

/// A route as the columns know it: a sound's (group, sound, chain slot and parameter:
/// one route a parameter), or a CC route's (source, port, channel, CC). Kept on the
/// app, not saved.
#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Sound(usize, usize, usize, ModParam),
    Cc(ModSource, String, u8, u8),
}

fn key(g: usize, s: usize, r: &ModRoute) -> Key {
    Key::Sound(g, s, r.slot, r.param.clone())
}

fn cc_key(r: &CcRoute) -> Key {
    Key::Cc(r.source, r.port.clone(), r.channel, r.cc)
}

/// Where a route is: (group, sound, place on the sound), or its place among the CC
/// routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    Sound(usize, usize, usize),
    Cc(usize),
}

/// The columns' order, kept on the app (not saved): each column's route, and its place
/// on its sound when last seen; and the project's chains then (`shape`).
#[derive(Debug, Default)]
pub struct ModOrder {
    cols: Vec<(Key, usize)>,
    shape: Vec<Vec<Vec<String>>>,
}

/// Every sound's chain: the module (class, or plug-in id) in each slot.
fn shape(p: &Project) -> Vec<Vec<Vec<String>>> {
    p.groups
        .iter()
        .map(|g| {
            g.sounds
                .iter()
                .map(|s| s.slots.iter().map(|x| x.plugin.as_ref().map_or_else(|| x.module.clone(), |pr| pr.id.clone())).collect())
                .collect()
        })
        .collect()
}

/// Whether `new` is `old` with only groups, sounds or slots added at the ends (none
/// renumbered).
fn extends(old: &[Vec<Vec<String>>], new: &[Vec<Vec<String>>]) -> bool {
    old.len() <= new.len() && old.iter().zip(new).all(|(o, n)| o.len() <= n.len() && o.iter().zip(n).all(|(o, n)| n.starts_with(o)))
}

/// Every route in the project as (its key, where it is): those in `order` in that
/// order; a column whose route is gone (an undo, a redo) takes the route now at its
/// place (on its sound, or among the CC routes); then the rest by group, sound and
/// place, then the CC routes.
fn ordered(p: &Project, order: &[(Key, usize)]) -> Vec<(Key, At)> {
    let mut all: Vec<Option<(Key, At)>> = Vec::new();
    for (g, grp) in p.groups.iter().enumerate() {
        for (s, snd) in grp.sounds.iter().enumerate() {
            all.extend(snd.mod_routes.iter().enumerate().map(|(i, r)| Some((key(g, s, r), At::Sound(g, s, i)))));
        }
    }
    all.extend(p.cc_routes.iter().enumerate().map(|(i, r)| Some((cc_key(r), At::Cc(i)))));
    let mut take = |f: &dyn Fn(&(Key, At)) -> bool| all.iter_mut().find(|x| x.as_ref().is_some_and(f)).and_then(Option::take);
    let mut got: Vec<_> = order.iter().map(|(k, _)| take(&|(kk, _)| kk == k)).collect();
    for (x, (k, i)) in got.iter_mut().zip(order) {
        if x.is_none() {
            let was = match k {
                Key::Sound(g, s, ..) => At::Sound(*g, *s, *i),
                Key::Cc(..) => At::Cc(*i),
            };
            *x = take(&|(_, at)| *at == was);
        }
    }
    got.into_iter().flatten().chain(all.into_iter().flatten()).collect()
}

/// The slots for routes `routes` in column order (the draft in place of the route it
/// came from), then a new draft or an empty slot.
pub fn slots(p: &Project, draft: Option<&ModDraft>, routes: &[At]) -> Vec<ModSlot> {
    let origin = draft.and_then(|d| match (&d.from, &d.from_cc) {
        (Some((g, s, r)), _) => Some(At::Sound(*g, *s, from_at(p, *g, *s, r)?)),
        (_, Some(r)) => Some(At::Cc(p.cc_routes.iter().position(|x| x == r)?)),
        _ => None,
    });
    let mut out: Vec<ModSlot> = routes
        .iter()
        .map(|&at| match at {
            _ if origin == Some(at) => ModSlot::Draft,
            At::Sound(g, s, i) => ModSlot::Route(g, s, i),
            At::Cc(i) => ModSlot::Cc(i),
        })
        .collect();
    // A draft from a route is in its place (each slot keeps its column); a new one last.
    out.push(if draft.is_some() && origin.is_none() { ModSlot::Draft } else { ModSlot::Empty });
    out
}

/// Where route `r` is on sound (`g`, `s`) now.
fn from_at(p: &Project, g: usize, s: usize, r: &ModRoute) -> Option<usize> {
    p.groups.get(g)?.sounds.get(s)?.mod_routes.iter().position(|x| x == r)
}

/// A parameter a route can drive: the module (class, or plug-in id) and parameter, its
/// name as the knob's, and its knob's travel now (0..1).
#[derive(Debug, Clone, PartialEq)]
pub struct Destination {
    pub module: String,
    pub param: ModParam,
    pub name: String,
    pub travel: f32,
}

/// The parameters of slot `i` of sound (`g`, `s`) a route can drive, in the Control
/// page's order.
pub fn destinations(app: &App, g: usize, s: usize, i: usize) -> Vec<Destination> {
    let Some(doc) = &app.doc else { return Vec::new() };
    let d = doc.borrow();
    let mut out: Vec<Destination> = Vec::new();
    for k in layout::slot_knobs(app, &d, g, s, i) {
        let Some(e) = &k.3 else { continue };
        let Some((_, _, slot, module, param)) = mod_routes::knob_route_target(d.project(), Chain::Sound(g, s), e) else { continue };
        if slot == i && !out.iter().any(|x| x.param == param) {
            out.push(Destination { module, param, name: channel::title(&k.0), travel: k.2.clamp(0.0, 1.0) });
        }
    }
    out
}

/// Row `row` of slot `slot` turned `steps` (0: erase the slot).
pub fn step(app: &mut App, slot: usize, row: usize, steps: i64) {
    let Some(doc) = app.doc.clone() else { return };
    let (at, p) = {
        let d = doc.borrow();
        let p = d.project().clone();
        (app.mod_columns(&p).get(slot).copied(), p)
    };
    match at {
        Some(ModSlot::Route(g, s, i)) => route_step(app, &p, (g, s, i), row, steps),
        Some(ModSlot::Cc(i)) => cc_step(app, &p, i, row, steps),
        Some(ModSlot::Draft) => draft_step(app, &p, row, steps),
        Some(ModSlot::Empty) if row == 0 && steps > 0 => {
            if let Some(source) = source_step(&p.lfos, None, steps) {
                app.mod_draft = Some(ModDraft::new(source));
            }
        }
        _ => {}
    }
}

/// Give sound (`g`, `s`) these routes; a failure goes to the status line.
/// Whether it did.
fn set(app: &mut App, g: usize, s: usize, routes: Vec<ModRoute>) -> bool {
    let Some(doc) = app.doc.clone() else { return false };
    let r = doc.borrow_mut().set_mod_routes(g, s, routes);
    if let Err(e) = &r {
        app.status = format!("can't change that: {e}");
    }
    r.is_ok()
}

fn route_step(app: &mut App, p: &Project, (g, s, i): (usize, usize, usize), row: usize, steps: i64) {
    let snd = &p.groups[g].sounds[s];
    let mut list = snd.mod_routes.clone();
    let r = list[i].clone();
    if steps == 0 {
        list.remove(i);
        set(app, g, s, list);
        return;
    }
    // A new place: (group, sound, slot), each kept while it still holds the route's module.
    let draft = |group: usize, sound: Option<usize>, slot: Option<usize>| ModDraft {
        group: Some(group),
        sound,
        slot,
        from: Some((g, s, r.clone())),
        ..ModDraft::new(r.source)
    };
    let keep_slot = |gg: usize, ss: usize| holds(p, gg, ss, r.slot, &r.module).then_some(r.slot);
    match row {
        0 => match source_step(&p.lfos, Some(r.source), steps) {
            Some(src) => {
                list[i].source = src;
                set(app, g, s, list);
            }
            None => {
                list.remove(i);
                set(app, g, s, list);
            }
        },
        1 if g as i64 + steps >= p.groups.len() as i64 => {
            if let Some(source) = cc_source(&p.lfos, r.source) {
                app.mod_draft = Some(ModDraft { midi: Some(CcDraft::default()), from: Some((g, s, r.clone())), ..ModDraft::new(source) });
            }
        }
        1 => {
            let to = clamp_step(g, steps, p.groups.len());
            if to != g {
                let sound = (s < p.groups[to].sounds.len()).then_some(s);
                app.mod_draft = Some(draft(to, sound, sound.and_then(|ss| keep_slot(to, ss))));
            }
        }
        2 => {
            let to = clamp_step(s, steps, p.groups[g].sounds.len());
            if to != s {
                app.mod_draft = Some(draft(g, Some(to), keep_slot(g, to)));
            }
        }
        3 => {
            let to = clamp_step(r.slot, steps, snd.slots.len());
            if to != r.slot && !snd.slots.is_empty() {
                app.mod_draft = Some(draft(g, Some(s), Some(to)));
            }
        }
        4 => {
            let dests = free(destinations(app, g, s, r.slot), &list, r.slot, Some(i));
            let cur = dests.iter().position(|d| d.param == r.param && d.module == r.module);
            let Some(to) = index_step(cur, steps, dests.len()) else { return };
            if Some(to) == cur {
                return;
            }
            let d = &dests[to];
            let moved = ModRoute { module: d.module.clone(), param: d.param.clone(), ..r };
            let new = key(g, s, &moved);
            place(&mut list, moved, Some(i));
            if set(app, g, s, list) {
                app.mod_rekey(Some(key(g, s, &r)), new, i);
            }
        }
        5 => {
            list[i].high = round2((r.high + DEPTH_STEP * steps as f32).clamp(0.0, 1.0));
            set(app, g, s, list);
        }
        _ => {}
    }
}

fn draft_step(app: &mut App, p: &Project, row: usize, steps: i64) {
    let Some(mut d) = app.mod_draft.clone() else { return };
    if steps == 0 {
        return erase_draft(app, p, d);
    }
    if let Some(mut m) = d.midi.clone() {
        match row {
            0 => match cc_source_step(&p.lfos, d.source, steps) {
                Some(src) => d.source = src,
                None => return erase_draft(app, p, d),
            },
            // Back from MIDI: the last group, a draft again.
            1 if steps < 0 && !p.groups.is_empty() => {
                d.midi = None;
                d.group = Some(clamp_step(p.groups.len(), steps, p.groups.len()));
            }
            2 => {
                let ports = out_ports(app);
                let cur = m.port.as_ref().and_then(|x| ports.iter().position(|q| q == x));
                if let Some(i) = index_step(cur, steps, ports.len()) {
                    m.port = Some(ports[i].clone());
                }
                d.midi = Some(m);
            }
            3 => {
                m.channel = Some(m.channel.map_or(steps, |c| i64::from(c) + steps).clamp(1, 16) as u8);
                d.midi = Some(m);
            }
            4 => return cc_destine(app, p, d, m, steps),
            _ => return,
        }
        app.mod_draft = Some(d);
        return;
    }
    let sounds = |g: Option<usize>| g.and_then(|g| p.groups.get(g)).map_or(0, |x| x.sounds.len());
    let chain =
        |g: Option<usize>, s: Option<usize>| g.zip(s).and_then(|(g, s)| p.groups.get(g)?.sounds.get(s)).map_or(0, |x| x.slots.len());
    match row {
        0 => match source_step(&p.lfos, Some(d.source), steps) {
            Some(src) => d.source = src,
            None => return erase_draft(app, p, d),
        },
        1 if steps > 0 && d.group.map_or(steps - 1, |g| g as i64 + steps) >= p.groups.len() as i64 => {
            // Past the last group: MIDI (an LFO's). A CC route's draft is the route
            // again, untouched.
            if d.from_cc.is_some() && d.from.is_none() {
                app.mod_draft = None;
                return;
            }
            let Some(source) = cc_source(&p.lfos, d.source) else { return };
            d = ModDraft { midi: Some(CcDraft::default()), from: d.from, from_cc: d.from_cc, ..ModDraft::new(source) };
        }
        1 => d.group = index_step(d.group, steps, p.groups.len()),
        2 if d.group.is_some() => d.sound = index_step(d.sound, steps, sounds(d.group)),
        3 if d.sound.is_some() => d.slot = index_step(d.slot, steps, chain(d.group, d.sound)),
        4 => return destine(app, p, d, steps),
        _ => return,
    }
    // (What was chosen below a changed row stays while it's there.)
    d.sound = d.sound.filter(|s| *s < sounds(d.group));
    d.slot = d.slot.filter(|i| *i < chain(d.group, d.sound));
    app.mod_draft = Some(d);
}

/// The draft given a Destination, the first free one on: it becomes a route (moved from
/// the one it was, in one undo step).
fn destine(app: &mut App, p: &Project, d: ModDraft, steps: i64) {
    let (Some(g), Some(s), Some(slot)) = (d.group, d.sound, d.slot) else { return };
    let from = d.from.as_ref().and_then(|(fg, fs, r)| Some((*fg, *fs, from_at(p, *fg, *fs, r)?)));
    let routes = |g: usize, s: usize| p.groups[g].sounds[s].mod_routes.clone();
    let mut list = routes(g, s);
    let old_at = from.filter(|(fg, fs, _)| (*fg, *fs) == (g, s)).map(|(.., i)| i);
    let dests = free(destinations(app, g, s, slot), &list, slot, old_at);
    let Some(to) = index_step(None, steps, dests.len()) else { return };
    let dest = &dests[to];
    let made = ModRoute::new(d.source, slot, &dest.module, dest.param.clone());
    // A new route starts at the knob, Depth 0; a moved one keeps its range.
    let made = match (&d.from, &d.from_cc) {
        (Some((_, _, r)), _) => ModRoute { low: r.low, high: r.high, curve: r.curve, range: r.range, ..made },
        (_, Some(r)) => ModRoute { low: r.low, high: r.high, curve: r.curve, ..made },
        _ => ModRoute { low: dest.travel, high: dest.travel, ..made },
    };
    let new = key(g, s, &made);
    place(&mut list, made, old_at);
    let at = list.iter().position(|x| key(g, s, x) == new).unwrap_or(0);
    app.mod_draft = None;
    let cc_at = d.from_cc.as_ref().and_then(|r| p.cc_routes.iter().position(|x| x == r));
    let done = match from {
        _ if cc_at.is_some() => {
            let Some(doc) = app.doc.clone() else { return };
            let mut ccs = p.cc_routes.clone();
            ccs.remove(cc_at.unwrap_or(0));
            let r = doc.borrow_mut().transaction("Modulation", |doc| {
                doc.set_cc_routes(ccs)?;
                doc.set_mod_routes(g, s, list)
            });
            if let Err(e) = &r {
                app.status = format!("can't change that: {e}");
            }
            r.is_ok()
        }
        Some((fg, fs, i)) if (fg, fs) != (g, s) => {
            let Some(doc) = app.doc.clone() else { return };
            let mut left = routes(fg, fs);
            left.remove(i);
            let r = doc.borrow_mut().transaction("Modulation", |doc| {
                doc.set_mod_routes(fg, fs, left)?;
                doc.set_mod_routes(g, s, list)
            });
            if let Err(e) = &r {
                app.status = format!("can't change that: {e}");
            }
            r.is_ok()
        }
        _ => set(app, g, s, list),
    };
    // (It takes the draft's column: the route's it was, or the last.)
    if done {
        let old = match cc_at {
            Some(_) => d.from_cc.as_ref().map(cc_key),
            None => d.from.as_ref().filter(|_| from.is_some()).map(|(fg, fs, r)| key(*fg, *fs, r)),
        };
        app.mod_rekey(old, new, at);
    }
}

/// The MIDI draft given a CC (`steps` on from none: CC 1 for the first), the port and
/// channel the first ones when not chosen: it becomes a CC route (moved from the route
/// it was, in one undo step, keeping its range); a new one at Depth 0.
fn cc_destine(app: &mut App, p: &Project, d: ModDraft, m: CcDraft, steps: i64) {
    if steps < 0 {
        return;
    }
    let Some(port) = m.port.or_else(|| out_ports(app).into_iter().next()) else {
        app.status = "no MIDI output port to send a CC to".into();
        return;
    };
    let made = CcRoute::new(d.source, &port, m.channel.unwrap_or(1), steps.clamp(0, i64::from(CC_MAX)) as u8);
    let made = match &d.from {
        Some((_, _, r)) => CcRoute { low: r.low, high: r.high, curve: r.curve, ..made },
        None => made,
    };
    let new = cc_key(&made);
    let mut ccs = p.cc_routes.clone();
    ccs.push(made);
    let at = ccs.len() - 1;
    app.mod_draft = None;
    let from = d.from.as_ref().and_then(|(g, s, r)| Some((*g, *s, from_at(p, *g, *s, r)?)));
    let done = match from {
        Some((g, s, i)) => {
            let Some(doc) = app.doc.clone() else { return };
            let mut left = p.groups[g].sounds[s].mod_routes.clone();
            left.remove(i);
            let r = doc.borrow_mut().transaction("Modulation", |doc| {
                doc.set_mod_routes(g, s, left)?;
                doc.set_cc_routes(ccs)
            });
            if let Err(e) = &r {
                app.status = format!("can't change that: {e}");
            }
            r.is_ok()
        }
        None => set_cc(app, ccs),
    };
    if done {
        app.mod_rekey(d.from.as_ref().filter(|_| from.is_some()).map(|(g, s, r)| key(*g, *s, r)), new, at);
    }
}

/// Give the project these CC routes; a failure goes to the status line. Whether it did.
fn set_cc(app: &mut App, routes: Vec<CcRoute>) -> bool {
    let Some(doc) = app.doc.clone() else { return false };
    let r = doc.borrow_mut().set_cc_routes(routes);
    if let Err(e) = &r {
        app.status = format!("can't change that: {e}");
    }
    r.is_ok()
}

/// Row `row` of CC route `i` turned `steps` (0: erased): Source over the LFOs, Group
/// back to the last group (a draft), Port, Channel, CC, Depth.
fn cc_step(app: &mut App, p: &Project, i: usize, row: usize, steps: i64) {
    let mut list = p.cc_routes.clone();
    let r = list[i].clone();
    if steps == 0 {
        list.remove(i);
        set_cc(app, list);
        return;
    }
    let c = &mut list[i];
    match row {
        0 => match cc_source_step(&p.lfos, r.source, steps) {
            Some(src) => c.source = src,
            None => {
                list.remove(i);
                set_cc(app, list);
                return;
            }
        },
        1 if steps < 0 && !p.groups.is_empty() => {
            let group = clamp_step(p.groups.len(), steps, p.groups.len());
            app.mod_draft = Some(ModDraft { group: Some(group), from_cc: Some(r.clone()), ..ModDraft::new(r.source) });
            return;
        }
        2 => {
            let ports = out_ports(app);
            let cur = ports.iter().position(|q| *q == r.port);
            let Some(to) = index_step(cur, steps, ports.len()) else { return };
            c.port = ports[to].clone();
        }
        3 => c.channel = (i64::from(r.channel) + steps).clamp(1, 16) as u8,
        4 => c.cc = (i64::from(r.cc) + steps).clamp(0, i64::from(CC_MAX)) as u8,
        5 => c.high = round2((r.high + DEPTH_STEP * steps as f32).clamp(0.0, 1.0)),
        _ => return,
    }
    let new = cc_key(c);
    if set_cc(app, list) {
        app.mod_rekey(Some(cc_key(&r)), new, i);
    }
}

/// The output ports a CC can go to (those not turned off in Settings).
pub(crate) fn out_ports(app: &App) -> Vec<String> {
    app.midi_ports.1.iter().filter(|n| !app.settings.midi_off.contains(&format!("out:{n}"))).cloned().collect()
}

/// A CC route's source for `cur`: it, if an LFO; else the first LFO; none without one.
fn cc_source(lfos: &[Lfo], cur: ModSource) -> Option<ModSource> {
    match cur {
        ModSource::Lfo(_) => Some(cur),
        _ => lfos.first().map(|l| ModSource::Lfo(l.pad)),
    }
}

/// A CC route's source `steps` on from `cur` in None, then the LFOs (the expressions
/// passed over); `None` is None.
fn cc_source_step(lfos: &[Lfo], cur: ModSource, steps: i64) -> Option<ModSource> {
    let list: Vec<ModSource> = lfos.iter().map(|l| ModSource::Lfo(l.pad)).collect();
    step_in(&list, Some(cur), steps)
}

/// `dests` (of chain slot `slot`) less those another route in `list` (all but route
/// `own`) drives: a Destination step passes over them, never taking one from its route.
fn free(dests: Vec<Destination>, list: &[ModRoute], slot: usize, own: Option<usize>) -> Vec<Destination> {
    let taken = |d: &Destination| list.iter().enumerate().any(|(k, x)| Some(k) != own && x.slot == slot && x.param == d.param);
    dests.into_iter().filter(|d| !taken(d)).collect()
}

/// The draft erased, and the route it came from with it.
fn erase_draft(app: &mut App, p: &Project, d: ModDraft) {
    app.mod_draft = None;
    if let Some((g, s, r)) = &d.from
        && let Some(i) = from_at(p, *g, *s, r)
    {
        let mut list = p.groups[*g].sounds[*s].mod_routes.clone();
        list.remove(i);
        set(app, *g, *s, list);
    }
    if let Some(r) = &d.from_cc
        && let Some(i) = p.cc_routes.iter().position(|x| x == r)
    {
        let mut list = p.cc_routes.clone();
        list.remove(i);
        set_cc(app, list);
    }
}

/// Put route `r` in `list`, one route a parameter: at `at` (the route it was, which
/// keeps its place; another on the same parameter goes), else in place of the one on
/// its parameter, else last.
fn place(list: &mut Vec<ModRoute>, r: ModRoute, at: Option<usize>) {
    let same = |x: &ModRoute| x.slot == r.slot && x.param == r.param;
    match at {
        Some(i) => {
            // (Those before it that go move it down.)
            let gone = list[..i].iter().filter(|x| same(x)).count();
            let mut k = 0;
            list.retain(|x| {
                let keep = k == i || !same(x);
                k += 1;
                keep
            });
            list[i - gone] = r;
        }
        None => match list.iter().position(same) {
            Some(k) => list[k] = r,
            None => list.push(r),
        },
    }
}

/// Whether slot `slot` of sound (`g`, `s`) holds `module` (a class, or a plug-in's id).
fn holds(p: &Project, g: usize, s: usize, slot: usize, module: &str) -> bool {
    p.groups.get(g).and_then(|x| x.sounds.get(s)).and_then(|x| x.slots.get(slot)).is_some_and(|x| match &x.plugin {
        Some(pr) => pr.id == module,
        None => x.module == module,
    })
}

/// The source `steps` on from `cur` in None, then [`choices`]; `None` is None. A source
/// that isn't a choice (a removed LFO) steps from where it would be.
fn source_step(lfos: &[Lfo], cur: Option<ModSource>, steps: i64) -> Option<ModSource> {
    step_in(&choices(lfos), cur, steps)
}

/// The source `steps` on from `cur` in None, then `list`.
fn step_in(list: &[ModSource], cur: Option<ModSource>, steps: i64) -> Option<ModSource> {
    let rank = |s: &ModSource| match s {
        ModSource::Lfo(n) => 3 + i64::from(*n),
        s => ModSource::ALL.iter().position(|x| x == s).unwrap_or(0) as i64,
    };
    let to = match cur {
        None => steps,
        Some(c) => match list.iter().position(|x| *x == c) {
            Some(k) => k as i64 + 1 + steps,
            None => {
                let before = list.iter().filter(|x| rank(x) < rank(&c)).count() as i64;
                if steps > 0 { before + steps } else { before + 1 + steps }
            }
        },
    };
    let to = to.clamp(0, list.len() as i64) as usize;
    (to > 0).then(|| list[to - 1])
}

/// Index `cur` of `n` moved `steps`, stopping at the ends; from none, a step forward
/// is the first.
fn index_step(cur: Option<usize>, steps: i64, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    match cur {
        Some(c) => Some(clamp_step(c, steps, n)),
        None if steps > 0 => Some((steps as usize - 1).min(n - 1)),
        None => None,
    }
}

fn clamp_step(cur: usize, steps: i64, n: usize) -> usize {
    (cur as i64 + steps).clamp(0, n.saturating_sub(1) as i64) as usize
}

fn round2(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}

/// Depth (`high` − `low`) as a signed percent: "+15%", "−40%", "0%".
fn depth_text(low: f32, high: f32) -> String {
    let d = ((high - low) * 100.0).round() as i32;
    match d {
        0 => "0%".into(),
        d if d > 0 => format!("+{d}%"),
        d => format!("−{}%", -d),
    }
}

/// A group as the Group row names it: "A1 – name", or "A1" when it has none of its
/// own (none, or Maschine's "Group A1").
fn group_text(p: &Project, g: usize) -> Option<String> {
    let grp = p.groups.get(g)?;
    let id = layout::group_id(g);
    Some(if grp.name.is_empty() || grp.name == format!("Group {id}") { id } else { format!("{id} – {}", grp.name) })
}

fn sound_text(p: &Project, g: usize, s: usize) -> Option<String> {
    let snd = p.groups.get(g)?.sounds.get(s)?;
    let name = if snd.name.is_empty() { format!("Sound {}", s + 1) } else { snd.name.clone() };
    Some(format!("S{} – {name}", s + 1))
}

impl App {
    /// The mod matrix's columns now: in the order they had (routes new to it after), which
    /// is kept for next time.
    /// A group, sound or chain slot removed or moved (the routes' keys renumbered)
    /// starts it again, by sound.
    fn mod_columns(&self, p: &Project) -> Vec<ModSlot> {
        let mut o = self.mod_order.borrow_mut();
        let now = shape(p);
        if !extends(&o.shape, &now) {
            o.cols.clear();
        }
        o.shape = now;
        let routes = ordered(p, &o.cols);
        o.cols = routes
            .iter()
            .map(|(k, at)| {
                let i = match at {
                    At::Sound(.., i) | At::Cc(i) => *i,
                };
                (k.clone(), i)
            })
            .collect();
        let at: Vec<_> = routes.into_iter().map(|(_, at)| at).collect();
        slots(p, self.mod_draft.as_ref(), &at)
    }

    /// The route `old` (none: a new one, last) is now `new`, at place `i` on its sound,
    /// in the same column.
    fn mod_rekey(&self, old: Option<Key>, new: Key, i: usize) {
        let cols = &mut self.mod_order.borrow_mut().cols;
        match old.and_then(|o| cols.iter().position(|(k, _)| *k == o)) {
            Some(at) => cols[at] = (new, i),
            None => cols.push((new, i)),
        }
    }

    /// The mod matrix slots for the surface.
    pub(crate) fn mod_slot_views(&self) -> Vec<ModSlotView> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let shown = doc.borrow();
        let p = shown.project();
        let none = || NONE.to_string();
        // (Each slot's destinations once.)
        let mut dests: Vec<((usize, usize, usize), Vec<Destination>)> = Vec::new();
        self.mod_columns(p)
            .into_iter()
            .map(|at| {
                let rows = match at {
                    ModSlot::Route(g, s, i) => {
                        let snd = &p.groups[g].sounds[s];
                        let r = &snd.mod_routes[i];
                        let live = mod_routes::route_live(snd, r);
                        let module = match (&r.param, live) {
                            (_, true) => snd.slots[r.slot].name.clone(),
                            (ModParam::Field(_), false) => format!("{} (none)", r.module),
                            (ModParam::Plugin(_), false) => "Plug-in (none)".into(),
                        };
                        let key = (g, s, r.slot);
                        if live && !dests.iter().any(|(k, _)| *k == key) {
                            dests.push((key, destinations(self, g, s, r.slot)));
                        }
                        let named = dests.iter().find(|(k, _)| *k == key).and_then(|(_, d)| d.iter().find(|d| d.param == r.param));
                        let dest = named.map_or_else(|| channel::title(&mod_routes::param_name(r)), |d| d.name.clone());
                        [
                            mod_routes::lfo_name(&p.lfos, r.source),
                            group_text(p, g).unwrap_or_else(none),
                            sound_text(p, g, s).unwrap_or_else(none),
                            module,
                            dest,
                            depth_text(r.low, r.high),
                        ]
                    }
                    ModSlot::Cc(i) => {
                        let r = &p.cc_routes[i];
                        let rows = [
                            mod_routes::lfo_name(&p.lfos, r.source),
                            MIDI.into(),
                            r.port.clone(),
                            r.channel.to_string(),
                            r.cc.to_string(),
                            depth_text(r.low, r.high),
                        ];
                        return ModSlotView { rows, midi: true };
                    }
                    ModSlot::Draft if self.mod_draft.as_ref().is_some_and(|d| d.midi.is_some()) => {
                        let d = self.mod_draft.as_ref().expect("a draft slot has a draft");
                        let m = d.midi.clone().unwrap_or_default();
                        let rows = [
                            mod_routes::lfo_name(&p.lfos, d.source),
                            MIDI.into(),
                            m.port.unwrap_or_else(none),
                            m.channel.map_or_else(none, |c| c.to_string()),
                            none(),
                            none(),
                        ];
                        return ModSlotView { rows, midi: true };
                    }
                    ModSlot::Draft => {
                        let d = self.mod_draft.as_ref().expect("a draft slot has a draft");
                        let module = d
                            .group
                            .zip(d.sound)
                            .zip(d.slot)
                            .and_then(|((g, s), i)| p.groups.get(g)?.sounds.get(s)?.slots.get(i))
                            .map(|x| x.name.clone());
                        [
                            mod_routes::lfo_name(&p.lfos, d.source),
                            d.group.and_then(|g| group_text(p, g)).unwrap_or_else(none),
                            d.group.zip(d.sound).and_then(|(g, s)| sound_text(p, g, s)).unwrap_or_else(none),
                            module.unwrap_or_else(none),
                            none(),
                            none(),
                        ]
                    }
                    ModSlot::Empty => std::array::from_fn(|_| none()),
                };
                ModSlotView { rows, midi: false }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_surface::Action as A;

    /// A new project whose first sound has a Sampler.
    fn app() -> App {
        let mut app = App::default();
        app.new_project();
        app.doc.as_ref().unwrap().borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
        app
    }

    fn routes(app: &App) -> Vec<ModRoute> {
        app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].mod_routes.clone()
    }

    fn set_routes(app: &App, routes: Vec<ModRoute>) {
        app.doc.as_ref().unwrap().borrow_mut().set_mod_routes(0, 0, routes).unwrap();
    }

    fn rows(app: &App, slot: usize) -> [String; 6] {
        app.mod_slot_views()[slot].rows.clone()
    }

    fn cutoff(source: ModSource) -> ModRoute {
        ModRoute::new(source, 0, "Sampler", ModParam::Field("cutoff".into()))
    }

    #[test]
    fn the_last_slot_is_empty() {
        let app = app();
        let slots = app.mod_slot_views();
        assert_eq!(slots.len(), 1);
        assert!(slots[0].rows.iter().all(|r| r == NONE), "{:?}", slots[0].rows);
    }

    /// Groups as Maschine names them: the ninth is A2; an unnamed one is just its ID.
    #[test]
    fn the_group_row_names_groups_by_letter_and_bank() {
        let mut app = app();
        let doc = app.doc.clone().unwrap();
        for _ in 0..8 {
            doc.borrow_mut().add_group().unwrap();
        }
        set_routes(&app, vec![cutoff(ModSource::Pressure)]);
        assert_eq!(rows(&app, 0)[1], "A1");
        app.surface_act(A::ModSlot(0, 1, 8));
        assert_eq!(rows(&app, 0)[1], "A2", "Maschine's \"Group A2\": not \"A2 – Group A2\"");
        let mut q = doc.borrow().project().clone();
        q.groups[8].name.clear();
        assert_eq!(group_text(&q, 8).unwrap(), "A2");
        q.groups[8].name = "Bass".into();
        assert_eq!(group_text(&q, 8).unwrap(), "A2 – Bass");
    }

    /// An LFO in Envelope mode is named "ENV n" in the Source row.
    #[test]
    fn an_envelope_source_reads_env() {
        let mut app = app();
        app.surface_act(A::Lfo("create", 1, 0.0));
        app.surface_act(A::ModSlot(0, 0, 4));
        assert_eq!(rows(&app, 0)[0], "LFO 1");
        app.surface_act(A::Lfo("env", 1, 0.0));
        assert_eq!(rows(&app, 0)[0], "ENV 1");
    }

    #[test]
    fn a_slot_becomes_a_route_with_a_destination() {
        let mut app = app();
        app.surface_act(A::Lfo("create", 1, 0.0));
        // None, Pressure, Slide, Pitch, LFO 1.
        app.surface_act(A::ModSlot(0, 0, 4));
        assert_eq!(rows(&app, 0)[0], "LFO 1");
        app.surface_act(A::ModSlot(0, 1, 1));
        app.surface_act(A::ModSlot(0, 2, 1));
        app.surface_act(A::ModSlot(0, 3, 1));
        let r = rows(&app, 0);
        assert_eq!(r[1], "A1", "{r:?}");
        assert!(r[2].starts_with("S1 – "), "{r:?}");
        assert_eq!(r[3], "Sampler");
        assert!(routes(&app).is_empty(), "no route without a destination");
        app.surface_act(A::ModSlot(0, 4, 1));
        let first = destinations(&app, 0, 0, 0)[0].clone();
        let made = routes(&app);
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].source, ModSource::Lfo(1));
        assert_eq!(made[0].param, first.param);
        assert_eq!(made[0].low, first.travel, "starts at the knob");
        assert_eq!(made[0].high, made[0].low, "Depth 0");
        assert_eq!(made[0].curve, 50);
        let r = rows(&app, 0);
        assert_eq!(r[4], first.name);
        assert_eq!(r[5], "0%");
        assert_eq!(app.mod_slot_views().len(), 2);
    }

    #[test]
    fn depth_moves_high_from_low() {
        let mut app = app();
        set_routes(&app, vec![ModRoute { low: 0.4, high: 0.4, ..cutoff(ModSource::Pressure) }]);
        app.surface_act(A::ModSlot(0, 5, 3));
        assert!((routes(&app)[0].high - 0.55).abs() < 1e-6, "{}", routes(&app)[0].high);
        assert_eq!(routes(&app)[0].low, 0.4);
        assert_eq!(rows(&app, 0)[5], "+15%");
        app.surface_act(A::ModSlot(0, 5, -100));
        assert_eq!(routes(&app)[0].high, 0.0);
        assert_eq!(rows(&app, 0)[5], "−40%");
    }

    #[test]
    fn changing_the_destination_moves_the_route() {
        let mut app = app();
        // (CUTOFF shows with a filter on.)
        let filter = layout::Act::SetParam(deckard_doc::Target::Sampler(0, 0), "filter", 1.0);
        layout::apply_acts(&mut app, vec![filter]);
        let d = destinations(&app, 0, 0, 0);
        let at = d.iter().position(|x| x.param == ModParam::Field("cutoff".into())).expect("cutoff is a destination");
        let next = d[at + 1].param.clone();
        let drive = ModParam::Field("drive".into());
        let free = d[at + 2..].iter().find(|x| x.param != drive).expect("a free one after").param.clone();
        let moved = ModRoute { low: 0.1, high: 0.9, ..cutoff(ModSource::Slide) };
        let there = ModRoute { param: next.clone(), ..cutoff(ModSource::Pressure) };
        let other = ModRoute { param: drive, ..cutoff(ModSource::Pitch) };
        set_routes(&app, vec![moved.clone(), other.clone(), there.clone()]);
        app.surface_act(A::ModSlot(0, 4, 1));
        assert_eq!(routes(&app), vec![ModRoute { param: free, ..moved }, other, there], "moved in its place, past the taken one");
    }

    /// The user's report: a new slot (Source, Group, Sound, Module set) turned slowly
    /// through its Destinations, over a parameter another route has. The slot keeps all
    /// it has, skips the taken one, and the other route stays as it was.
    #[test]
    fn a_new_slot_skips_a_taken_destination() {
        let mut app = app();
        layout::apply_acts(&mut app, vec![layout::Act::SetParam(deckard_doc::Target::Sampler(0, 0), "filter", 1.0)]);
        app.surface_act(A::Lfo("create", 1, 0.0));
        let d = destinations(&app, 0, 0, 0);
        assert!(d.len() >= 5, "{d:?}");
        let taken = ModRoute { module: d[1].module.clone(), param: d[1].param.clone(), ..cutoff(ModSource::Pressure) };
        set_routes(&app, vec![taken.clone()]);
        // Slot 2 (from 0: 1): LFO 1, A1, S1, the Sampler.
        app.surface_act(A::ModSlot(1, 0, 4));
        app.surface_act(A::ModSlot(1, 1, 1));
        app.surface_act(A::ModSlot(1, 2, 1));
        app.surface_act(A::ModSlot(1, 3, 1));
        let kept = |app: &App| {
            let r = rows(app, 1);
            assert_eq!((r[0].as_str(), r[1].as_str(), r[3].as_str()), ("LFO 1", "A1", "Sampler"), "{r:?}");
            assert!(r[2].starts_with("S1 – "), "{r:?}");
            assert_eq!(routes(app)[0], taken, "the other route untouched");
            assert_eq!(rows(app, 0)[0], "Pressure");
            r[4].clone()
        };
        app.surface_act(A::ModSlot(1, 4, 1));
        assert_eq!(kept(&app), d[0].name);
        // A slow turn: one step a call.
        app.surface_act(A::ModSlot(1, 4, 1));
        assert_eq!(kept(&app), d[2].name, "the taken one skipped");
        for k in 3..d.len() {
            app.surface_act(A::ModSlot(1, 4, 1));
            assert_eq!(kept(&app), d[k].name);
        }
        app.surface_act(A::ModSlot(1, 4, 1));
        assert_eq!(kept(&app), d[d.len() - 1].name, "stops at the end");
        for k in (2..d.len() - 1).rev() {
            app.surface_act(A::ModSlot(1, 4, -1));
            assert_eq!(kept(&app), d[k].name);
        }
        app.surface_act(A::ModSlot(1, 4, -1));
        assert_eq!(kept(&app), d[0].name, "back over it too");
        app.surface_act(A::ModSlot(1, 4, 3));
        assert_eq!(kept(&app), d[4].name, "three free ones on: d[2], d[3], d[4]");
        assert_eq!(routes(&app).len(), 2);
        assert_eq!(app.mod_slot_views().len(), 3);
    }

    /// A draft's first Destination, when the first parameter is taken: the next free one.
    #[test]
    fn a_draft_skips_a_taken_first_destination() {
        let mut app = app();
        app.surface_act(A::Lfo("create", 1, 0.0));
        let d = destinations(&app, 0, 0, 0);
        let taken = ModRoute { module: d[0].module.clone(), param: d[0].param.clone(), ..cutoff(ModSource::Pressure) };
        set_routes(&app, vec![taken.clone()]);
        app.surface_act(A::ModSlot(1, 0, 4));
        app.surface_act(A::ModSlot(1, 1, 1));
        app.surface_act(A::ModSlot(1, 2, 1));
        app.surface_act(A::ModSlot(1, 3, 1));
        app.surface_act(A::ModSlot(1, 4, 1));
        let made = routes(&app);
        assert_eq!(made.len(), 2, "{made:?}");
        assert_eq!(made[0], taken, "the other route untouched");
        assert_eq!((made[1].source, &made[1].param), (ModSource::Lfo(1), &d[1].param));
        assert_eq!(rows(&app, 1)[4], d[1].name);
    }

    /// Every parameter taken: a Destination step does nothing, and the draft stays.
    #[test]
    fn a_draft_with_every_destination_taken_stays() {
        let mut app = app();
        app.surface_act(A::Lfo("create", 1, 0.0));
        let d = destinations(&app, 0, 0, 0);
        let all: Vec<ModRoute> =
            d.iter().map(|x| ModRoute { module: x.module.clone(), param: x.param.clone(), ..cutoff(ModSource::Pressure) }).collect();
        set_routes(&app, all.clone());
        let n = all.len();
        app.surface_act(A::ModSlot(n, 0, 4));
        app.surface_act(A::ModSlot(n, 1, 1));
        app.surface_act(A::ModSlot(n, 2, 1));
        app.surface_act(A::ModSlot(n, 3, 1));
        app.surface_act(A::ModSlot(n, 4, 1));
        assert_eq!(routes(&app), all);
        assert!(app.mod_draft.is_some());
        assert_eq!(rows(&app, n)[3], "Sampler");
    }

    /// A draft left alone while the app carries on (views built, the surface ticked)
    /// is still there: only closing the page or another document drops it.
    #[test]
    fn a_draft_survives_a_pause() {
        let mut app = app();
        app.surface_act(A::ModSlot(0, 0, 1));
        app.surface_act(A::ModSlot(0, 1, 1));
        let draft = app.mod_draft.clone();
        assert!(draft.is_some());
        for _ in 0..3 {
            let _ = app.mod_slot_views();
            let v = app.surface_view();
            let later = std::time::Instant::now() + std::time::Duration::from_secs(30);
            for a in app.surface.tick(&v, later) {
                app.surface_act(a);
            }
        }
        assert_eq!(app.mod_draft, draft);
    }

    /// Every slot's rows, by column.
    fn columns(app: &App) -> Vec<[String; 6]> {
        app.mod_slot_views().into_iter().map(|v| v.rows).collect()
    }

    fn sound_routes(app: &App, s: usize) -> Vec<ModRoute> {
        app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[s].mod_routes.clone()
    }

    /// A new slot made on a sound before another sound's route stays in its column, and
    /// that route in its own (the review's case).
    #[test]
    fn a_new_slot_on_an_earlier_sound_keeps_its_column() {
        let mut app = app();
        let doc = app.doc.clone().unwrap();
        doc.borrow_mut().add_slot(0, 1, 0, &deckard_doc::NewModule::Sampler).unwrap();
        let theirs = vec![ModRoute { low: 0.0, high: 1.0, ..cutoff(ModSource::Pressure) }];
        doc.borrow_mut().set_mod_routes(0, 1, theirs.clone()).unwrap();
        app.surface_act(A::Lfo("create", 1, 0.0));
        app.surface_act(A::ModSlot(1, 0, 4));
        app.surface_act(A::ModSlot(1, 1, 1));
        app.surface_act(A::ModSlot(1, 2, 1));
        app.surface_act(A::ModSlot(1, 3, 1));
        let d = destinations(&app, 0, 0, 0);
        let before = columns(&app)[0].clone();
        assert!(before[2].starts_with("S2 – "), "{before:?}");
        for k in 0..2 {
            app.surface_act(A::ModSlot(1, 4, 1));
            let c = columns(&app);
            let r = &c[1];
            assert_eq!((r[0].as_str(), r[1].as_str(), r[3].as_str()), ("LFO 1", "A1", "Sampler"), "step {k}: {r:?}");
            assert!(r[2].starts_with("S1 – "), "step {k}: {r:?}");
            assert_eq!(r[4], d[k].name, "step {k}");
            assert_eq!(c[0], before, "step {k}: S2's route in its column");
            assert_eq!(sound_routes(&app, 1), theirs, "step {k}: S2's route untouched");
        }
    }

    /// A route moved to another sound keeps its column; every other column shows what
    /// it did. ERASE closes the columns up after the one erased.
    #[test]
    fn a_route_moved_to_another_sound_keeps_its_column() {
        let mut app = app();
        let doc = app.doc.clone().unwrap();
        doc.borrow_mut().add_slot(0, 1, 0, &deckard_doc::NewModule::Sampler).unwrap();
        doc.borrow_mut().add_slot(0, 2, 0, &deckard_doc::NewModule::Sampler).unwrap();
        let drive = ModRoute { param: ModParam::Field("drive".into()), ..cutoff(ModSource::Pitch) };
        set_routes(&app, vec![cutoff(ModSource::Slide), drive]);
        doc.borrow_mut().set_mod_routes(0, 1, vec![cutoff(ModSource::Pressure)]).unwrap();
        let before = columns(&app);
        assert_eq!(before.len(), 4);
        // Column 0 (S1's Slide route) to S3, then a Destination.
        app.surface_act(A::ModSlot(0, 2, 2));
        app.surface_act(A::ModSlot(0, 4, 1));
        assert_eq!(sound_routes(&app, 2).len(), 1, "moved to S3");
        let after = columns(&app);
        assert_eq!(after.len(), 4);
        assert_eq!(after[0][0], "Slide");
        assert!(after[0][2].starts_with("S3 – "), "{:?}", after[0]);
        assert_eq!(after[1..], before[1..], "the other columns as they were");
        // Its Destination stepped: still column 0.
        app.surface_act(A::ModSlot(0, 4, 1));
        let stepped = columns(&app);
        assert_eq!(stepped[0][..4], after[0][..4]);
        assert_eq!(stepped[1..], before[1..]);
        // ERASE on column 0: the later ones close up, in their order.
        app.surface_act(A::ModSlot(0, 0, 0));
        assert_eq!(columns(&app)[..3], before[1..]);
    }

    /// Undo and redo of a Destination change: the route stays in its column.
    #[test]
    fn undo_of_a_destination_keeps_the_column() {
        let mut app = app();
        let filter = layout::Act::SetParam(deckard_doc::Target::Sampler(0, 0), "filter", 1.0);
        layout::apply_acts(&mut app, vec![filter]);
        let drive = ModRoute { param: ModParam::Field("drive".into()), ..cutoff(ModSource::Pitch) };
        set_routes(&app, vec![cutoff(ModSource::Slide), drive]);
        let before = columns(&app);
        app.surface_act(A::ModSlot(0, 4, 1));
        let after = columns(&app);
        assert_eq!(after[0][0], "Slide");
        assert_ne!(after[0][4], before[0][4], "moved");
        let doc = app.doc.clone().unwrap();
        assert!(doc.borrow_mut().undo());
        assert_eq!(columns(&app), before, "undone: in their columns");
        assert!(doc.borrow_mut().redo());
        assert_eq!(columns(&app), after, "redone");
    }

    /// A group deleted renumbers the ones after it: the columns keep their routes.
    #[test]
    fn deleting_a_group_keeps_the_columns_routes() {
        let app = app();
        let doc = app.doc.clone().unwrap();
        for g in 1..3 {
            doc.borrow_mut().add_group().unwrap();
            doc.borrow_mut().add_slot(g, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
        }
        doc.borrow_mut().set_mod_routes(1, 0, vec![cutoff(ModSource::Slide)]).unwrap();
        doc.borrow_mut().set_mod_routes(2, 0, vec![cutoff(ModSource::Pressure)]).unwrap();
        assert_eq!((columns(&app)[0][0].as_str(), columns(&app)[1][0].as_str()), ("Slide", "Pressure"));
        doc.borrow_mut().delete_group(0).unwrap();
        let c = columns(&app);
        assert!(c[0][0] == "Slide" && c[0][1].starts_with("A1"), "{c:?}");
        assert!(c[1][0] == "Pressure" && c[1][1].starts_with("B1"), "{c:?}");
    }

    /// Another sound: a draft (the route drives until then), then one move, one undo step.
    #[test]
    fn changing_the_sound_makes_a_draft_until_a_destination() {
        let mut app = app();
        app.doc.as_ref().unwrap().borrow_mut().add_slot(0, 1, 0, &deckard_doc::NewModule::Sampler).unwrap();
        set_routes(&app, vec![ModRoute { low: 0.2, high: 0.7, ..cutoff(ModSource::Slide) }]);
        app.surface_act(A::ModSlot(0, 2, 1));
        let r = rows(&app, 0);
        assert_eq!(r[0], "Slide");
        assert!(r[2].starts_with("S2 – "), "{r:?}");
        assert_eq!((r[3].as_str(), r[4].as_str()), ("Sampler", NONE), "the module kept, the destination cleared");
        assert_eq!(app.mod_slot_views().len(), 2, "the draft in the route's place, then the empty slot");
        assert_eq!(routes(&app).len(), 1, "the route drives until it has a new destination");
        app.surface_act(A::ModSlot(0, 4, 1));
        assert!(routes(&app).is_empty());
        let doc = app.doc.clone().unwrap();
        let moved = doc.borrow().project().groups[0].sounds[1].mod_routes.clone();
        let first = destinations(&app, 0, 1, 0)[0].param.clone();
        assert_eq!(moved, vec![ModRoute { low: 0.2, high: 0.7, param: first, ..cutoff(ModSource::Slide) }], "its range kept");
        assert!(doc.borrow_mut().undo());
        assert_eq!(routes(&app).len(), 1, "one undo step");
        assert!(doc.borrow().project().groups[0].sounds[1].mod_routes.is_empty());
    }

    /// A draft stays in its route's column: the next turns act on it, not the next route.
    #[test]
    fn slots_keep_their_place() {
        let mut app = app();
        let doc = app.doc.clone().unwrap();
        doc.borrow_mut().add_group().unwrap();
        doc.borrow_mut().add_group().unwrap();
        let second = ModRoute { param: ModParam::Field("drive".into()), ..cutoff(ModSource::Pressure) };
        set_routes(&app, vec![cutoff(ModSource::Slide), second.clone()]);
        app.surface_act(A::ModSlot(0, 1, 1));
        assert_eq!(rows(&app, 0)[1], "B1");
        app.surface_act(A::ModSlot(0, 1, 1));
        assert_eq!(rows(&app, 0)[1], "C1");
        let r = rows(&app, 1);
        assert_eq!((r[0].as_str(), r[1].as_str()), ("Pressure", "A1"), "{r:?}");
        assert_eq!(routes(&app), vec![cutoff(ModSource::Slide), second], "both still there");
        assert_eq!(app.mod_slot_views().len(), 3);
    }

    /// Closing the page drops the draft; the route it came from stays.
    #[test]
    fn a_draft_is_dropped_on_close_and_new_project() {
        let mut app = app();
        set_routes(&app, vec![cutoff(ModSource::Slide)]);
        app.surface_act(A::ModSlot(0, 2, 1));
        assert!(app.mod_draft.is_some());
        app.surface_act(A::ModSlotsClosed);
        assert!(app.mod_draft.is_none());
        assert_eq!(routes(&app), vec![cutoff(ModSource::Slide)]);
        assert!(rows(&app, 0)[2].starts_with("S1 – "));
        app.surface_act(A::ModSlot(0, 2, 1));
        assert!(app.mod_draft.is_some());
        app.new_project();
        assert!(app.mod_draft.is_none());
    }

    #[test]
    fn erase_removes_the_slot_route() {
        let mut app = app();
        set_routes(&app, vec![cutoff(ModSource::Pressure)]);
        assert_eq!(app.mod_slot_views().len(), 2);
        app.surface_act(A::ModSlot(0, 0, 0));
        assert!(routes(&app).is_empty());
        assert_eq!(app.mod_slot_views().len(), 1);
        // A source stepped back to None erases too.
        set_routes(&app, vec![cutoff(ModSource::Pressure)]);
        app.surface_act(A::ModSlot(0, 0, -1));
        assert!(routes(&app).is_empty());
    }

    #[test]
    fn missing_lfo_shows_none() {
        let app = app();
        set_routes(&app, vec![cutoff(ModSource::Lfo(3))]);
        assert_eq!(rows(&app, 0)[0], "LFO 3 (none)");
    }

    fn ccs(app: &App) -> Vec<CcRoute> {
        app.doc.as_ref().unwrap().borrow().project().cc_routes.clone()
    }

    /// [`app`] with LFO 1 and two output ports.
    fn midi_app() -> App {
        let mut app = app();
        app.midi_ports.1 = vec!["Port A".into(), "Port B".into()];
        app.surface_act(A::Lfo("create", 1, 0.0));
        app
    }

    /// Group stepped past the last group is MIDI: the rows read Port, Channel, CC; the
    /// first CC step makes the route (the first port, channel 1); Depth, then ERASE.
    /// The slot keeps its column throughout.
    #[test]
    fn a_group_step_past_the_last_group_is_midi() {
        let mut app = midi_app();
        set_routes(&app, vec![cutoff(ModSource::Pressure)]);
        // Pressure, then past A1: MIDI takes an LFO.
        app.surface_act(A::ModSlot(1, 0, 1));
        app.surface_act(A::ModSlot(1, 1, 2));
        let v = app.mod_slot_views();
        assert!(v[1].midi && !v[0].midi);
        assert_eq!(v[1].rows, ["LFO 1", "MIDI", NONE, NONE, NONE, NONE].map(String::from));
        assert!(ccs(&app).is_empty(), "no route without a CC");
        app.surface_act(A::ModSlot(1, 4, 1));
        assert_eq!(ccs(&app), vec![CcRoute::new(ModSource::Lfo(1), "Port A", 1, 1)]);
        assert_eq!(rows(&app, 1), ["LFO 1", "MIDI", "Port A", "1", "1", "0%"].map(String::from));
        app.surface_act(A::ModSlot(1, 2, 1));
        app.surface_act(A::ModSlot(1, 3, 9));
        app.surface_act(A::ModSlot(1, 4, 73));
        app.surface_act(A::ModSlot(1, 5, 2));
        assert_eq!(ccs(&app), vec![CcRoute { high: 0.1, ..CcRoute::new(ModSource::Lfo(1), "Port B", 10, 74) }]);
        let c = columns(&app);
        assert_eq!(c[0][0], "Pressure", "{c:?}");
        assert_eq!(c[1], ["LFO 1", "MIDI", "Port B", "10", "74", "+10%"].map(String::from));
        // A sound route made after comes after it.
        app.surface_act(A::ModSlot(2, 0, 2));
        app.surface_act(A::ModSlot(2, 1, 1));
        app.surface_act(A::ModSlot(2, 2, 1));
        app.surface_act(A::ModSlot(2, 3, 1));
        app.surface_act(A::ModSlot(2, 4, 3));
        let c = columns(&app);
        assert_eq!((c[0][0].as_str(), c[1][1].as_str(), c[2][0].as_str()), ("Pressure", "MIDI", "Slide"), "{c:?}");
        app.surface_act(A::ModSlot(1, 0, 0));
        assert!(ccs(&app).is_empty());
        let c = columns(&app);
        assert_eq!((c.len(), c[0][0].as_str(), c[1][0].as_str()), (3, "Pressure", "Slide"), "closed up: {c:?}");
    }

    /// A CC route's Source steps over the LFOs only; Group back from MIDI is a draft on
    /// the last group, which takes the CC route's place once it has a Destination.
    #[test]
    fn a_cc_route_steps_back_to_a_group() {
        let mut app = midi_app();
        app.surface_act(A::Lfo("create", 2, 0.0));
        let made = CcRoute { low: 0.2, high: 0.6, ..CcRoute::new(ModSource::Lfo(2), "Port A", 2, 7) };
        app.doc.as_ref().unwrap().borrow_mut().set_cc_routes(vec![made.clone()]).unwrap();
        app.surface_act(A::ModSlot(0, 0, -1));
        assert_eq!(ccs(&app)[0].source, ModSource::Lfo(1));
        app.surface_act(A::ModSlot(0, 0, 1));
        assert_eq!(ccs(&app), vec![made.clone()]);
        app.surface_act(A::ModSlot(0, 1, -1));
        let v = app.mod_slot_views();
        assert_eq!(v.len(), 2, "the draft in the route's place");
        assert!(!v[0].midi);
        assert_eq!(v[0].rows[..2], ["LFO 2", "A1"].map(String::from));
        assert_eq!(ccs(&app), vec![made.clone()], "kept until a Destination");
        app.surface_act(A::ModSlot(0, 2, 1));
        app.surface_act(A::ModSlot(0, 3, 1));
        app.surface_act(A::ModSlot(0, 4, 1));
        assert!(ccs(&app).is_empty());
        let r = routes(&app);
        assert_eq!((r.len(), r[0].source, r[0].low, r[0].high), (1, ModSource::Lfo(2), 0.2, 0.6));
        assert_eq!(columns(&app).len(), 2);
        assert!(app.doc.as_ref().unwrap().borrow_mut().undo());
        assert_eq!(ccs(&app), vec![made], "one undo step");
        assert!(routes(&app).is_empty());
    }

    /// A sound route's Group stepped past the last is a MIDI draft; its first CC moves
    /// it there, with its range, in its column.
    #[test]
    fn a_sound_route_moves_to_a_cc() {
        let mut app = midi_app();
        let r = ModRoute { low: 0.1, high: 0.9, ..cutoff(ModSource::Lfo(1)) };
        set_routes(&app, vec![r, cutoff(ModSource::Pressure)]);
        app.surface_act(A::ModSlot(0, 1, 1));
        assert!(app.mod_slot_views()[0].midi);
        app.surface_act(A::ModSlot(0, 4, 5));
        assert_eq!(ccs(&app), vec![CcRoute { low: 0.1, high: 0.9, ..CcRoute::new(ModSource::Lfo(1), "Port A", 1, 5) }]);
        assert_eq!(routes(&app), vec![cutoff(ModSource::Pressure)]);
        let c = columns(&app);
        assert_eq!((c[0][1].as_str(), c[1][0].as_str()), ("MIDI", "Pressure"), "{c:?}");
    }

    /// A CC route's Group turned back and forward again (MIDI) is the CC route again,
    /// untouched: a CC step edits it, no second route.
    #[test]
    fn a_cc_route_back_and_forward_is_itself() {
        let mut app = midi_app();
        app.surface_act(A::Lfo("create", 2, 0.0));
        set_routes(&app, vec![cutoff(ModSource::Pressure)]);
        let made = CcRoute { high: 0.5, ..CcRoute::new(ModSource::Lfo(2), "Port B", 10, 74) };
        app.doc.as_ref().unwrap().borrow_mut().set_cc_routes(vec![made.clone()]).unwrap();
        app.surface_act(A::ModSlot(1, 1, -1));
        assert!(!app.mod_slot_views()[1].midi);
        app.surface_act(A::ModSlot(1, 1, 1));
        assert_eq!(rows(&app, 1), ["LFO 2", "MIDI", "Port B", "10", "74", "+50%"].map(String::from));
        app.surface_act(A::ModSlot(1, 4, 1));
        assert_eq!(ccs(&app), vec![CcRoute { cc: 75, ..made.clone() }]);
        assert_eq!(columns(&app).len(), 3);
        // CC stops at 119 (120-127 are channel mode).
        app.surface_act(A::ModSlot(1, 4, 100));
        assert_eq!(ccs(&app)[0].cc, 119);
        assert_eq!(rows(&app, 1)[4], "119");
    }
}
