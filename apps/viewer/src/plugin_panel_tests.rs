//! The plug-in strip's panels: every hosted plug-in shows its own pages and knobs, not
//! just the focused one's, each on a page of its own; views of panels gone are dropped.

use crate::App;
use crate::layout::{self, Act, ChainTab};
use deckard_doc::Chain;
use eframe::egui;
use egui_kittest::Harness;

const CHAIN: Chain = Chain::Sound(0, 0);

/// A new project, its first sound holding DECKARD's two plug-ins (slots 0 and 1), both
/// running (as playing loads them), the first focused, in the Mixer (the strip below
/// shows the sound's chain).
fn strip_with_two_plugins() -> Harness<'static, App> {
    strip_with(true)
}

/// [`strip_with_two_plugins`], the second running if `running`.
fn strip_with(running: bool) -> Harness<'static, App> {
    use deckard_plugins::native::CLASSES;
    let mut app = App { settings: crate::settings::Settings::default(), model: "M3".into(), ..App::default() };
    app.new_project();
    let doc = app.doc.clone().unwrap();
    for (i, class) in CLASSES.iter().take(2).enumerate() {
        let new = deckard_doc::NewModule::Plugin { id: class.plugin_id(), name: class.name.into(), chunk: Vec::new() };
        doc.borrow_mut().add_slot(0, 0, i, &new).unwrap();
    }
    (app.group, app.sound, app.chain_tab, app.channel, app.slot) = (0, 0, ChainTab::Sound, false, Some(0));
    app.mixer = true;
    if running {
        app.slot_plugin_in(CHAIN, 1).unwrap();
    }
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0 + layout::CONTROLLER_WIDTH, 930.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    h.run_steps(4);
    h
}

/// What the strip's hosted plug-in panels show: (slot, page, knob labels).
fn shown(h: &Harness<'static, App>) -> Vec<(usize, usize, Vec<String>)> {
    h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-plugins"))).unwrap_or_default()
}

/// The labels of page `page` of slot `slot`'s view, as the strip draws them.
fn labels(h: &Harness<'static, App>, slot: usize, page: usize) -> Vec<String> {
    let v = h.state().plugin_views.get(&(CHAIN, slot)).expect("a view of the slot");
    v.pages[page].knobs.iter().map(|(_, l, _)| l.to_uppercase()).collect()
}

#[test]
fn every_plugin_panel_shows_its_knobs() {
    let h = strip_with_two_plugins();
    let shown = shown(&h);
    let one = shown.iter().find(|x| x.0 == 1).expect("the unfocused plug-in's panel shows its knobs");
    assert!(!one.2.is_empty(), "{shown:?}");
    assert_eq!(one.2, labels(&h, 1, 0));
    let zero = shown.iter().find(|x| x.0 == 0).expect("the focused one's too");
    assert_eq!(zero.2, labels(&h, 0, 0));
}

#[test]
fn each_plugin_panel_keeps_its_own_page() {
    let mut h = strip_with_two_plugins();
    let at = |h: &Harness<'static, App>, slot: usize| shown(h).into_iter().find(|x| x.0 == slot).map(|x| (x.1, x.2));
    // The unfocused one's page turns; the focused one's stays.
    layout::apply_acts(h.state_mut(), vec![Act::PluginPageOf(CHAIN, 1, 1)]);
    h.run_steps(3);
    assert_eq!(at(&h, 1), Some((1, labels(&h, 1, 1))));
    assert_eq!(at(&h, 0), Some((0, labels(&h, 0, 0))));
    assert_eq!(h.state().plugin_page, 0, "the focused slot's page");
    // The focused one's (as the SOUND tab and the controller turn it); the other stays.
    layout::apply_acts(h.state_mut(), vec![Act::PluginPage(2)]);
    h.run_steps(3);
    assert_eq!(at(&h, 0), Some((2, labels(&h, 0, 2))));
    assert_eq!(at(&h, 1), Some((1, labels(&h, 1, 1))));
    // Focused, a slot shows the page it was on.
    layout::apply_acts(h.state_mut(), vec![Act::FocusSlot(0, 0, 1)]);
    h.run_steps(3);
    assert_eq!(h.state().plugin_page, 1, "slot 1's own page");
    assert_eq!(at(&h, 0), Some((2, labels(&h, 0, 2))));
    layout::apply_acts(h.state_mut(), vec![Act::FocusSlot(0, 0, 0)]);
    h.run_steps(3);
    assert_eq!(h.state().plugin_page, 2, "slot 0's own page");
    assert_eq!(h.state().plugin_view().map(|v| v.key), Some((CHAIN, 0)), "the focused slot's view drives the controller");
}

#[test]
fn views_of_slots_no_longer_drawn_are_dropped() {
    let mut h = strip_with_two_plugins();
    let keys = |h: &Harness<'static, App>| {
        let mut k: Vec<_> = h.state().plugin_views.keys().copied().collect();
        k.sort_by_key(|k| k.1);
        k
    };
    assert_eq!(keys(&h), [(CHAIN, 0), (CHAIN, 1)]);
    // (Their last sight a while ago.)
    let age = |h: &mut Harness<'static, App>| {
        for v in h.state_mut().plugin_views.values_mut() {
            v.seen -= crate::VIEW_KEEP;
        }
    };
    // Still drawn: kept.
    age(&mut h);
    h.run_steps(2);
    assert_eq!(keys(&h), [(CHAIN, 0), (CHAIN, 1)]);
    // The Mixer closes: the strip isn't drawn; only the focused slot's view stays.
    h.state_mut().mixer = false;
    h.run_steps(2);
    age(&mut h);
    h.run_steps(2);
    assert_eq!(keys(&h), [(CHAIN, 0)]);
    // Another sound, no plug-ins: none.
    h.state_mut().sound = 1;
    h.run_steps(2);
    age(&mut h);
    h.run_steps(2);
    assert_eq!(keys(&h), []);
}

/// Drawing a panel never loads its plug-in (that waits on the audio thread, and may
/// take seconds): a plug-in not running yet shows no view until it's focused.
#[test]
fn a_panel_drawn_never_loads_its_plugin() {
    let mut h = strip_with(false);
    let key = h.state().plugin_slot_key(CHAIN, 1);
    assert!(h.state().samples.plugins.existing(key).is_none(), "not loaded by drawing");
    assert!(!h.state().plugin_views.contains_key(&(CHAIN, 1)));
    assert!(shown(&h).iter().all(|x| x.0 != 1));
    // Focused, it loads and shows its knobs.
    layout::apply_acts(h.state_mut(), vec![Act::FocusSlot(0, 0, 1)]);
    h.run_steps(3);
    assert!(h.state().samples.plugins.existing(key).is_some());
    assert_eq!(shown(&h).iter().find(|x| x.0 == 1).map(|x| x.2.clone()), Some(labels(&h, 1, 0)));
}

/// Another plug-in in a slot (an undo, a replace) gets a view of its own, not the old
/// one's pages.
#[test]
fn a_plugin_replaced_in_a_slot_gets_its_own_view() {
    use deckard_plugins::native::CLASSES;
    let mut h = strip_with_two_plugins();
    let names = |h: &Harness<'static, App>, slot: usize| -> Vec<String> {
        h.state().plugin_views.get(&(CHAIN, slot)).map_or(Vec::new(), |v| v.pages.iter().map(|p| p.name.clone()).collect())
    };
    let plaits = names(&h, 1);
    let replicant = names(&h, 0);
    assert_ne!(plaits, replicant);
    // Slot 1 becomes a Replicant (behind the views' back, as an undo would).
    let doc = h.state().doc.clone().unwrap();
    doc.borrow_mut().remove_slot_in(CHAIN, 1).unwrap();
    let new = deckard_doc::NewModule::Plugin { id: CLASSES[0].plugin_id(), name: CLASSES[0].name.into(), chunk: Vec::new() };
    doc.borrow_mut().add_slot(0, 0, 1, &new).unwrap();
    h.run_steps(1);
    assert_ne!(names(&h, 1), plaits, "the old view went");
    // Running (as playing loads it): its own pages.
    h.state_mut().slot_plugin_in(CHAIN, 1).unwrap();
    h.run_steps(3);
    assert_eq!(names(&h, 1), replicant);
    assert_eq!(shown(&h).iter().find(|x| x.0 == 1).map(|x| x.2.clone()), Some(labels(&h, 1, 0)));
}

/// Before any engine runs (nothing played yet), an undo of a plug-in's state reaches
/// the focused plug-in: its view refreshes it from the project, so it shows (and later
/// saves) the state undone to.
#[test]
fn undo_reloads_the_focused_plugins_state_before_any_engine() {
    let mut h = strip_with_two_plugins();
    assert!(!h.state().output.as_ref().is_some_and(|o| o.as_ref().is_ok_and(|o| o.has_engine())), "no engine");
    let key = h.state().plugin_slot_key(CHAIN, 0);
    let inst = h.state().samples.plugins.existing(key).expect("the focused plug-in runs");
    let (id, before) = {
        let v = h.state().plugin_views.get(&(CHAIN, 0)).unwrap();
        let id = v.pages[0].knobs.iter().find(|(i, ..)| i.steps == 0).expect("a continuous knob").0.id;
        (id, inst.lock().unwrap().param_value(id))
    };
    let to = if before > 0.5 { 0.1 } else { 0.9 };
    // Turned: DECKARD's own plug-ins save into the project as they change (an undo step).
    layout::apply_acts(h.state_mut(), vec![Act::PluginParam(0, id, to)]);
    h.run_steps(2);
    assert!((inst.lock().unwrap().param_value(id) - to).abs() < 1e-6);
    let doc = h.state().doc.clone().unwrap();
    assert!(doc.borrow_mut().undo());
    // (Its next refresh.)
    for v in h.state_mut().plugin_views.values_mut() {
        v.refreshed -= crate::VIEW_REFRESH;
    }
    h.run_steps(2);
    assert!((inst.lock().unwrap().param_value(id) - before).abs() < 1e-6, "the instance took the undone state");
    let v = h.state().plugin_views.get(&(CHAIN, 0)).unwrap();
    let k = v.pages[0].knobs.iter().position(|(i, ..)| i.id == id).unwrap();
    assert!((v.values[k].0 - before).abs() < 1e-6, "and its panel shows it");
}
