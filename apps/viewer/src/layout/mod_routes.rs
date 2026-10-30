//! The Assignment area's MOD tab (docs/superpowers/specs/2026-10-24-expression-out-design.md,
//! 2b): under each knob of a sound's chain, its modulation route (or "Assign"), edited
//! in a menu that stays open until clicked outside; and the ROUTES list, every route of
//! the sound in one place.

use super::*;
use deckard_doc::Chain;
use deckard_model::lfo::Lfo;
use deckard_model::mod_matrix::{ModParam, ModRoute, ModSource, choices};

/// What a route from knob `edit` would drive, in chain `c`: the sound (group, sound),
/// the slot, the slot's module (or plug-in id) and the parameter. Continuous knobs of a
/// sound's own chain only.
pub fn knob_route_target(p: &Project, c: Chain, edit: &Edit) -> Option<(usize, usize, usize, String, ModParam)> {
    let slots = |g: usize, s: usize| p.groups.get(g).and_then(|x| x.sounds.get(s)).map(|x| &x.slots);
    match *edit {
        Edit::Param(t, f, ..) | Edit::Log(t, f, ..) | Edit::Int(t, f, ..) => match t {
            Target::Sampler(g, s) => {
                let slot = slots(g, s)?.iter().position(|x| x.module == "Sampler")?;
                Some((g, s, slot, "Sampler".into(), ModParam::Field(f.into())))
            }
            Target::Module(g, s, i) | Target::ChainModule(Chain::Sound(g, s), i) => {
                let info = slots(g, s)?.get(i)?;
                info.plugin.is_none().then(|| (g, s, i, info.module.clone(), ModParam::Field(f.into())))
            }
            _ => None,
        },
        Edit::Plugin(i, id, _) | Edit::PluginIn(Chain::Sound(..), i, id, _) => {
            let (g, s) = match (*edit, c) {
                (Edit::PluginIn(Chain::Sound(g, s), ..), _) | (Edit::Plugin(..), Chain::Sound(g, s)) => (g, s),
                _ => return None,
            };
            let pr = slots(g, s)?.get(i)?.plugin.as_ref()?;
            Some((g, s, i, pr.id.clone(), ModParam::Plugin(id)))
        }
        _ => None,
    }
}

/// A source's name: "LFO 3" ("ENV 3" in Envelope mode, "LFO 3 (none)" when the project
/// has no such LFO), or what a note gives.
pub fn lfo_name(lfos: &[Lfo], s: ModSource) -> String {
    match s {
        ModSource::Lfo(n) => match lfos.iter().find(|l| l.pad == n) {
            Some(l) => l.name(),
            None => format!("{} (none)", s.name()),
        },
        _ => s.name(),
    }
}

/// A route's range: "10–90".
fn route_range(r: &ModRoute) -> String {
    let pct = |x: f32| (x * 100.0).round() as i32;
    format!("{}–{}", pct(r.low), pct(r.high))
}

/// How a route reads in its field: "Pressure 10–90".
pub fn route_text(lfos: &[Lfo], r: &ModRoute) -> String {
    format!("{} {}", lfo_name(lfos, r.source), route_range(r))
}

/// The field under a knob: its route (click: edit it) or "Assign" (click: pick a source).
pub(super) fn knob_field(
    ui: &mut Ui,
    r: Rect,
    routes: &[ModRoute],
    lfos: &[Lfo],
    target: (usize, usize, usize, String, ModParam),
    acts: &mut Vec<Act>,
) {
    let (g, s, slot, module, param) = target;
    let (at, cur) = knob_route(routes, slot, &module, &param);
    let text = cur.as_ref().map_or("Assign".to_string(), |r| route_text(lfos, r));
    let mut child = ui.new_child(UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().button_padding = Vec2::new(4.0, 1.0);
    let config = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    egui::containers::menu::MenuButton::new(egui::RichText::new(text).size(10.0)).config(config).ui(&mut child, |ui| {
        let made = ModRoute::new(ModSource::Pressure, slot, &module, param.clone());
        route_editor(ui, g, s, routes, lfos, at, cur.clone(), made, acts);
    });
}

/// The route at knob (`slot`, `module`, `param`): where it is in `routes`, and the
/// route when it's the knob's. One left from another module the slot held is where an
/// assignment goes (replacing it), but isn't the knob's.
pub fn knob_route(routes: &[ModRoute], slot: usize, module: &str, param: &ModParam) -> (Option<usize>, Option<ModRoute>) {
    let at = routes.iter().position(|x| x.slot == slot && x.param == *param);
    (at, at.map(|i| &routes[i]).filter(|x| x.module == module).cloned())
}

/// The values sound `snd`'s routes give its built-in knobs now: (slot, field, stored
/// value), from each route's travel as the engine published it (`travel(k)`: route
/// `k`'s; `None`: not driving). Plug-ins show theirs themselves.
pub fn routed_values(snd: &deckard_model::Sound, travel: impl Fn(usize) -> Option<f32>) -> Vec<(usize, String, f64)> {
    let mut out = Vec::new();
    for (k, r) in snd.mod_routes.iter().enumerate() {
        let ModParam::Field(f) = &r.param else { continue };
        if !route_live(snd, r) {
            continue;
        }
        if let Some(t) = travel(k) {
            out.push((r.slot, f.clone(), deckard_engine::modscale::scale_of(&r.module, f).at(f64::from(t))));
        }
    }
    out
}

/// Slot `i` as its knobs show it: at the values routes give it ([`routed_values`]).
pub fn live_slot<'a>(slot: &'a deckard_model::SlotInfo, i: usize, vals: &[(usize, String, f64)]) -> Cow<'a, deckard_model::SlotInfo> {
    let mut vals = vals.iter().filter(|v| v.0 == i).peekable();
    if vals.peek().is_none() {
        return Cow::Borrowed(slot);
    }
    let mut out = slot.clone();
    for (_, f, v) in vals {
        if let Some(x) = out.params.iter_mut().find(|(k, _)| k == f) {
            x.1 = *v;
        }
    }
    Cow::Owned(out)
}

/// A Sampler (in slot `i`) as its knobs show it ([`live_slot`]).
pub fn live_sampler<'a>(smp: &'a deckard_model::Sampler, i: usize, vals: &[(usize, String, f64)]) -> Cow<'a, deckard_model::Sampler> {
    let mut vals = vals.iter().filter(|v| v.0 == i).peekable();
    if vals.peek().is_none() {
        return Cow::Borrowed(smp);
    }
    let mut out = smp.clone();
    for (_, f, v) in vals {
        out.set(f, *v);
    }
    Cow::Owned(out)
}

/// The own (stored) values of the knobs routes move on screen: (target, field, value).
/// Edits start from these, not from where the knob shows.
pub type Bases = Vec<(Target, String, f64)>;

/// [`Bases`] of slot `i` (its knobs' target `t`).
pub fn slot_bases(slot: &deckard_model::SlotInfo, t: Target, i: usize, vals: &[(usize, String, f64)]) -> Bases {
    vals.iter().filter(|v| v.0 == i).filter_map(|(_, f, _)| Some((t, f.clone(), slot.param(f)?))).collect()
}

/// [`Bases`] of a Sampler in slot `i` (its knobs' target `t`).
pub fn sampler_bases(smp: &deckard_model::Sampler, t: Target, i: usize, vals: &[(usize, String, f64)]) -> Bases {
    vals.iter().filter(|v| v.0 == i).map(|(_, f, _)| (t, f.clone(), smp.get(f))).collect()
}

/// Field `f` of `t`'s own value: its base when a route moves it, else `shown`.
pub fn base_of(bases: &[(Target, String, f64)], t: Target, f: &str, shown: f64) -> f64 {
    bases.iter().find(|b| b.0 == t && b.1 == f).map_or(shown, |b| b.2)
}

/// Where an edit of knob `edit`, shown at `shown`, starts (0..1): its own value's
/// place when a route moves it on screen.
pub fn edit_from(bases: &[(Target, String, f64)], edit: &Edit, shown: f32) -> f32 {
    let (Edit::Param(t, f, ..) | Edit::Log(t, f, ..) | Edit::Int(t, f, ..)) = edit else { return shown };
    let Some((_, scale)) = crate::modulation::scale_of(*edit) else { return shown };
    match bases.iter().find(|b| b.0 == *t && b.1 == *f) {
        Some(b) => scale.travel(b.2) as f32,
        None => shown,
    }
}

fn bases_id() -> egui::Id {
    egui::Id::new("deckard-knob-bases")
}

/// The knobs drawn next take their edits' starting values from `bases`.
pub fn set_bases(ui: &Ui, bases: Bases) {
    ui.data_mut(|d| d.insert_temp(bases_id(), bases));
}

/// [`edit_from`] with the bases [`set_bases`] gave.
pub fn ui_edit_from(ui: &Ui, edit: &Edit, shown: f32) -> f32 {
    let bases: Bases = ui.data(|d| d.get_temp(bases_id())).unwrap_or_default();
    edit_from(&bases, edit, shown)
}

/// [`base_of`] with the bases [`set_bases`] gave.
pub fn ui_base(ui: &Ui, t: Target, f: &str, shown: f64) -> f64 {
    let bases: Bases = ui.data(|d| d.get_temp(bases_id())).unwrap_or_default();
    base_of(&bases, t, f, shown)
}

/// Whether route `r` drives something: its slot holds the module it names.
pub fn route_live(sound: &deckard_model::Sound, r: &ModRoute) -> bool {
    sound.slots.get(r.slot).is_some_and(|x| match (&r.param, &x.plugin) {
        (ModParam::Plugin(_), Some(pr)) => pr.id == r.module,
        (ModParam::Field(_), None) => x.module == r.module,
        _ => false,
    })
}

/// Edit route `cur` (at `at` in `routes`; none: a source picked makes `made`, at `at`
/// if given, else added); every change is pushed as the sound's whole list.
#[allow(clippy::too_many_arguments)]
fn route_editor(
    ui: &mut Ui,
    g: usize,
    s: usize,
    routes: &[ModRoute],
    lfos: &[Lfo],
    at: Option<usize>,
    cur: Option<ModRoute>,
    made: ModRoute,
    acts: &mut Vec<Act>,
) {
    let mut set = |r: Option<ModRoute>| {
        let mut list = routes.to_vec();
        match (at, r) {
            (Some(i), Some(r)) => list[i] = r,
            (Some(i), None) => {
                list.remove(i);
            }
            (None, Some(r)) => list.push(r),
            (None, None) => return,
        }
        acts.push(Act::ModRoutes(g, s, list));
    };
    ui.horizontal(|ui| {
        ui.label("Source");
        for src in choices(lfos) {
            let on = cur.as_ref().is_some_and(|r| r.source == src);
            if ui.selectable_label(on, lfo_name(lfos, src)).clicked() && !on {
                let base = cur.clone().unwrap_or_else(|| made.clone());
                set(Some(ModRoute { source: src, ..base }));
            }
        }
    });
    let Some(r) = cur else {
        ui.label(egui::RichText::new("Pick what drives this knob: its sound's notes' pressure, slide or pitch.").weak());
        return;
    };
    let mut n = r.clone();
    let (mut low, mut high) = ((n.low * 100.0).round() as i32, (n.high * 100.0).round() as i32);
    egui::Grid::new("mod-route").num_columns(2).show(ui, |ui| {
        ui.label("Low");
        ui.add(egui::Slider::new(&mut low, 0..=100).suffix(" %"));
        ui.end_row();
        ui.label("High");
        ui.add(egui::Slider::new(&mut high, 0..=100).suffix(" %"));
        ui.end_row();
        ui.label("Curve");
        ui.add(egui::Slider::new(&mut n.curve, 0..=100).suffix(" %"));
        ui.end_row();
        if n.source == ModSource::Pitch {
            ui.label("Range");
            ui.add(egui::Slider::new(&mut n.range, 1..=48).suffix(" st"));
            ui.end_row();
        }
    });
    if low != (r.low * 100.0).round() as i32 {
        n.low = low as f32 / 100.0;
    }
    if high != (r.high * 100.0).round() as i32 {
        n.high = high as f32 / 100.0;
    }
    if n != r {
        set(Some(n));
    }
    ui.label(egui::RichText::new("Low above High inverts. Pitch rests halfway.").weak());
    if ui.button("Remove").clicked() {
        set(None);
        ui.close();
    }
}

/// ROUTES (n): every route of sound (`g`, `s`): source, target, range; × removes one.
pub(super) fn routes_list(ui: &mut Ui, r: Rect, sound: &deckard_model::Sound, lfos: &[Lfo], g: usize, s: usize, acts: &mut Vec<Act>) {
    let routes = &sound.mod_routes;
    let mut child = ui.new_child(UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().button_padding = Vec2::new(4.0, 1.0);
    let config = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    let label = format!("ROUTES ({})", routes.len());
    egui::containers::menu::MenuButton::new(egui::RichText::new(label).size(11.0)).config(config).ui(&mut child, |ui| {
        if routes.is_empty() {
            ui.label(egui::RichText::new("No routes: assign one under a knob.").weak());
        }
        for (i, r) in routes.iter().enumerate() {
            ui.horizontal(|ui| {
                let off = if route_live(sound, r) { "" } else { "  (inactive: another module)" };
                ui.label(format!("{}: {} / {}  {}{off}", lfo_name(lfos, r.source), slot_name(sound, r), param_name(r), route_range(r)));
                if ui.small_button("×").on_hover_text("Remove this route").clicked() {
                    let mut list = routes.clone();
                    list.remove(i);
                    acts.push(Act::ModRoutes(g, s, list));
                }
            });
        }
    });
}

fn slot_name(sound: &deckard_model::Sound, r: &ModRoute) -> String {
    sound.slots.get(r.slot).map_or_else(|| format!("Slot {}", r.slot + 1), |x| format!("{} {}", r.slot + 1, x.name))
}

pub fn param_name(r: &ModRoute) -> String {
    match &r.param {
        ModParam::Field(f) => f.replace('/', " ").replace('_', " "),
        ModParam::Plugin(id) => format!("parameter {id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        let mut d = deckard_doc::Document::new_project().unwrap();
        if d.project().groups[0].sounds[0].slots.is_empty() {
            d.add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
        }
        let n = d.project().groups[0].sounds[0].slots.len();
        d.add_slot(0, 0, n, &deckard_doc::NewModule::Effect("Filter")).unwrap();
        let mut p = d.project().clone();
        let pr = deckard_model::PluginRef { id: "X:VST3:FX:1".into(), name: "Fx".into(), ..Default::default() };
        p.groups[0].sounds[0].slots.push(deckard_model::SlotInfo {
            module: "PluginHost".into(),
            name: "Fx".into(),
            active: true,
            plugin: Some(pr),
            params: vec![],
        });
        p
    }

    #[test]
    fn knob_route_targets() {
        let p = project();
        let snd = Chain::Sound(0, 0);
        let smp = Edit::Param(Target::Sampler(0, 0), "cutoff", 0.0, 1.0, 1.0);
        assert_eq!(knob_route_target(&p, snd, &smp), Some((0, 0, 0, "Sampler".into(), ModParam::Field("cutoff".into()))));
        let n = p.groups[0].sounds[0].slots.len();
        let fx = Edit::Param(Target::Module(0, 0, n - 2), "cutoff", 0.0, 1.0, 0.5);
        assert_eq!(knob_route_target(&p, snd, &fx), Some((0, 0, n - 2, "Filter".into(), ModParam::Field("cutoff".into()))));
        let plug = Edit::Plugin(n - 1, 42, 0.0);
        assert_eq!(knob_route_target(&p, snd, &plug), Some((0, 0, n - 1, "X:VST3:FX:1".into(), ModParam::Plugin(42))));
        assert_eq!(knob_route_target(&p, Chain::Group(0), &plug), None, "a group's plug-in");
        let choice = Edit::Choice(Target::Sampler(0, 0), "filter", &["a", "b"], 0, 0.0);
        assert_eq!(knob_route_target(&p, snd, &choice), None, "a selector");
        assert_eq!(knob_route_target(&p, snd, &Edit::Level(Target::Sound(0, 0))), None);
    }

    /// A route left from a module the slot no longer holds isn't shown as the knob's:
    /// assigning there replaces it; the ROUTES list marks it inactive.
    #[test]
    fn a_route_to_another_module_is_inactive() {
        let p = project();
        let n = p.groups[0].sounds[0].slots.len();
        let dead = ModRoute::new(ModSource::Pressure, n - 2, "Compressor", ModParam::Field("cutoff".into()));
        let routes = vec![dead.clone()];
        assert_eq!(knob_route(&routes, n - 2, "Filter", &ModParam::Field("cutoff".into())), (Some(0), None));
        assert!(!route_live(&p.groups[0].sounds[0], &dead));
        let live = ModRoute { module: "Filter".into(), ..dead };
        assert_eq!(knob_route(&[live.clone()], n - 2, "Filter", &ModParam::Field("cutoff".into())), (Some(0), Some(live.clone())));
        assert!(route_live(&p.groups[0].sounds[0], &live));
    }

    /// Knob `field` of slot `n` as shown (value, position) with route 1 of the sound at
    /// travel 0.25 (as published) and no other.
    fn shown(snd: &deckard_model::Sound, n: usize, field: &str) -> (String, f32) {
        let vals = routed_values(snd, |k| (k == 1).then_some(0.25));
        let knobs: Vec<Option<Knob>> = if snd.slots[n].module == "Sampler" {
            let smp = live_sampler(snd.sampler.as_ref().unwrap(), n, &vals);
            (0..crate::sampler_ui::PAGES.len()).flat_map(|pg| crate::sampler_ui::page(&smp, 0, 0, pg).slots).collect()
        } else {
            let slot = live_slot(&snd.slots[n], n, &vals);
            let pages = crate::module_ui::pages(&slot.module).unwrap().len();
            (0..pages).flat_map(|pg| crate::module_ui::page_for(&slot, Target::Module(0, 0, n), pg).slots).collect()
        };
        let k = knobs.into_iter().flatten().find(|k| matches!(k.3, Some(Edit::Param(_, f, ..) | Edit::Log(_, f, ..)) if f == field));
        k.map(|k| (k.1, k.2)).expect("the knob")
    }

    /// A built-in knob a route drives shows where the route has it now (its published
    /// travel), value and position, as a plug-in's does; with the route gone (or one
    /// that isn't the knob's), its own value again.
    #[test]
    fn a_routed_knob_shows_its_travel() {
        crate::modulation::register();
        let mut p = project();
        let n = p.groups[0].sounds[0].slots.len() - 2;
        let snd = &mut p.groups[0].sounds[0];
        let own = shown(snd, n, "cutoff");
        assert!((own.1 - 0.25).abs() > 0.1, "{own:?}");
        let cutoff = ModParam::Field("cutoff".into());
        // Route 0 another module's (not driving), route 1 the knob's.
        let dead = ModRoute::new(ModSource::Lfo(1), n, "Compressor", cutoff.clone());
        let live = ModRoute::new(ModSource::Lfo(1), n, "Filter", cutoff.clone());
        snd.mod_routes = vec![dead.clone(), live];
        let moved = shown(snd, n, "cutoff");
        assert!((moved.1 - 0.25).abs() < 1e-6, "{moved:?}");
        assert_ne!(moved.0, own.0, "the value shown moves too");
        assert_eq!(shown(snd, n, "resonance"), {
            let keep = std::mem::take(&mut snd.mod_routes);
            let r = shown(snd, n, "resonance");
            snd.mod_routes = keep;
            r
        });
        snd.mod_routes = vec![dead];
        assert_eq!(shown(snd, n, "cutoff"), own, "route gone");
        // The Sampler's.
        let smp_slot = snd.slots.iter().position(|x| x.module == "Sampler").unwrap();
        let own = shown(snd, smp_slot, "tune");
        let tune = ModParam::Field("tune".into());
        snd.mod_routes = vec![
            ModRoute::new(ModSource::Pressure, smp_slot, "Sampler", cutoff),
            ModRoute::new(ModSource::Pressure, smp_slot, "Sampler", tune),
        ];
        let moved = shown(snd, smp_slot, "tune");
        assert!((moved.1 - 0.25).abs() < 1e-3, "{moved:?} (own {own:?})");
    }

    /// A routed knob shows where its route has it, but a drag (or a turn, a ring, a
    /// handle) starts from its own value: stored 0.20, shown 0.93, dragged +0.01 sets
    /// 0.21. The Sampler's envelope too.
    #[test]
    fn edits_start_from_the_stored_value() {
        crate::modulation::register();
        let mut p = project();
        let n = p.groups[0].sounds[0].slots.len() - 2;
        let snd = &mut p.groups[0].sounds[0];
        snd.slots[n].params.iter_mut().find(|x| x.0 == "cutoff").unwrap().1 = 0.20;
        let t = Target::Module(0, 0, n);
        let page = crate::module_ui::page_for(&snd.slots[n], t, 0);
        let edit = page.slots.iter().flatten().find_map(|k| k.3.filter(|e| matches!(e, Edit::Param(_, "cutoff", ..)))).unwrap();
        assert_eq!(edit_from(&[], &edit, 0.5), 0.5, "no route: from where it shows");
        snd.mod_routes = vec![ModRoute::new(ModSource::Lfo(1), n, "Filter", ModParam::Field("cutoff".into()))];
        let vals = routed_values(snd, |_| Some(0.93));
        let bases = slot_bases(&snd.slots[n], t, n, &vals);
        let from = edit_from(&bases, &edit, 0.93);
        assert!((from - 0.20).abs() < 1e-6, "{from}");
        match edit.to_act(from + 0.01) {
            Act::SetParam(_, "cutoff", v) => assert!((v - 0.21).abs() < 1e-6, "{v}"),
            _ => panic!("not the cutoff"),
        }
        // The Sampler's envelope handles: from the stored attack.
        let smp_slot = snd.slots.iter().position(|x| x.module == "Sampler").unwrap();
        snd.sampler.as_mut().unwrap().set("volume_envelope/attack", 0.1);
        let attack = ModParam::Field("volume_envelope/attack".into());
        snd.mod_routes = vec![ModRoute::new(ModSource::Lfo(1), smp_slot, "Sampler", attack)];
        let vals = routed_values(snd, |_| Some(0.9));
        let smp = snd.sampler.as_ref().unwrap();
        let st = Target::Sampler(0, 0);
        let bases = sampler_bases(smp, st, smp_slot, &vals);
        let live = live_sampler(smp, smp_slot, &vals).get("volume_envelope/attack");
        assert!((live - 0.1).abs() > 0.1, "shown elsewhere: {live}");
        assert_eq!(base_of(&bases, st, "volume_envelope/attack", live), 0.1);
        assert_eq!(base_of(&bases, st, "volume_envelope/hold", 0.4), 0.4, "not routed: as shown");
    }

    #[test]
    fn routes_read_as_source_and_range() {
        let r = ModRoute { low: 0.1, high: 0.9, ..ModRoute::new(ModSource::Slide, 0, "Sampler", ModParam::Field("cutoff".into())) };
        assert_eq!(route_text(&[], &r), "Slide 10–90");
    }

    #[test]
    fn missing_lfo_reads_none() {
        assert_eq!(lfo_name(&[], ModSource::Lfo(3)), "LFO 3 (none)");
        assert_eq!(lfo_name(&[Lfo::new(3)], ModSource::Lfo(3)), "LFO 3");
        assert_eq!(lfo_name(&[], ModSource::Pitch), "Pitch");
        let r = ModRoute { low: 0.1, high: 0.9, ..ModRoute::new(ModSource::Lfo(3), 0, "Sampler", ModParam::Field("cutoff".into())) };
        assert_eq!(route_text(&[], &r), "LFO 3 (none) 10–90");
    }
}
