//! Group and master plug-in chains in the window: the chain area's GROUP and MASTER
//! tabs edit them as the SOUND tab edits a sound's, and the controller shows them.

use crate::layout::{self, Act, AddWhat, ChainTab};
use crate::tests::harness;
use deckard_doc::{Chain, Target};

fn slots(h: &egui_kittest::Harness<'static, crate::App>, c: Chain) -> Vec<(String, bool)> {
    let doc = h.state().doc.clone().unwrap();
    let d = doc.borrow();
    d.chain_slots(c).unwrap_or(&[]).iter().map(|x| (x.module.clone(), x.active)).collect()
}

fn act(h: &mut egui_kittest::Harness<'static, crate::App>, a: Act) {
    layout::apply_acts(h.state_mut(), vec![a]);
    h.run_steps(2);
}

#[test]
fn the_group_tab_edits_the_groups_chain() {
    let Some(mut h) = harness(false, false) else { return };
    let g = h.state().group;
    h.state_mut().chain_tab = ChainTab::Group;
    let n = slots(&h, Chain::Group(g)).len();
    act(&mut h, Act::AddSlot(AddWhat::Effect("Filter")));
    assert_eq!(slots(&h, Chain::Group(g))[n], ("Filter".to_string(), true));
    assert_eq!(h.state().slot, Some(n));
    act(&mut h, Act::SlotActive(n, false));
    assert!(!slots(&h, Chain::Group(g))[n].1);
    act(&mut h, Act::SetParam(Target::ChainModule(Chain::Group(g), n), "cutoff", 0.3));
    let doc = h.state().doc.clone().unwrap();
    assert_eq!(doc.borrow().chain_slots(Chain::Group(g)).unwrap()[n].param("cutoff"), Some(0.3));
    act(&mut h, Act::RemoveSlot(n));
    assert_eq!(slots(&h, Chain::Group(g)).len(), n);
    // An instrument doesn't go on a group.
    act(&mut h, Act::AddSlot(AddWhat::Sampler));
    assert_eq!(slots(&h, Chain::Group(g)).len(), n);
}

#[test]
fn the_master_tab_edits_the_masters_chain() {
    let Some(mut h) = harness(false, false) else { return };
    h.state_mut().chain_tab = ChainTab::Master;
    let n = slots(&h, Chain::Master).len();
    act(&mut h, Act::AddSlot(AddWhat::Effect("Reverb")));
    assert_eq!(slots(&h, Chain::Master)[n].0, "Reverb");
    if n > 0 {
        act(&mut h, Act::MoveSlot(n, 0));
        assert_eq!(slots(&h, Chain::Master)[0].0, "Reverb");
    }
}

/// The controller's PLUG-IN page is given the chain of the level shown (MASTER, GROUP).
#[test]
fn the_controller_is_shown_the_groups_chain() {
    let Some(mut h) = harness(false, false) else { return };
    let g = h.state().group;
    h.state_mut().chain_tab = ChainTab::Group;
    act(&mut h, Act::AddSlot(AddWhat::Effect("Filter")));
    let c = h.state().control_snapshot([false; 8]);
    assert_eq!(c.level, 1);
    assert!(c.slots.iter().any(|s| s.name == "Filter"), "{:?}", c.slots.iter().map(|s| &s.name).collect::<Vec<_>>());
    let _ = g;
}

/// On the controller: GROUP, then an effect loaded from its browser goes on the group's
/// chain; an instrument doesn't.
#[test]
fn the_controller_loads_effects_onto_the_group() {
    use deckard_lua_host::state::ControlRequest as R;
    let Some(mut h) = harness(false, false) else { return };
    let g = h.state().group;
    h.state_mut().control_request(R::Level(1));
    h.run_steps(2);
    assert_eq!(h.state().chain_tab, ChainTab::Group);
    let choices = h.state().plugin_choices();
    let filter = choices.iter().position(|(e, _)| e.name == "Filter").expect("Filter offered");
    let sampler = choices.iter().position(|(e, _)| e.name == "Sampler").unwrap();
    let n = slots(&h, Chain::Group(g)).len();
    h.state_mut().control_request(R::Load { entry: filter, replace: None });
    h.run_steps(2);
    assert_eq!(slots(&h, Chain::Group(g)).len(), n + 1);
    assert!(slots(&h, Chain::Group(g)).iter().any(|s| s.0 == "Filter"));
    h.state_mut().control_request(R::Load { entry: sampler, replace: None });
    h.run_steps(2);
    assert_eq!(slots(&h, Chain::Group(g)).len(), n + 1, "no instrument on a group");
}

/// The Mix view's master strip: bypassing an effect, and showing it in the MASTER tab.
#[test]
fn the_mix_views_master_strip_bypasses_and_shows_effects() {
    let Some(mut h) = harness(false, false) else { return };
    let n = slots(&h, Chain::Master).len();
    if n == 0 {
        return;
    }
    act(&mut h, Act::ChainSlotActive(Chain::Master, 0, false));
    assert!(!slots(&h, Chain::Master)[0].1);
    act(&mut h, Act::FocusChainSlot(Chain::Master, n - 1));
    assert_eq!(h.state().chain_tab, ChainTab::Master);
    assert_eq!(h.state().slot, Some(n - 1));
}

/// A sound's effects reorder with Move Up / Move Down.
#[test]
fn a_sounds_effects_move() {
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
    doc.borrow_mut().add_slot(0, 0, 1, &deckard_doc::NewModule::Effect("Filter")).unwrap();
    (app.group, app.sound, app.chain_tab) = (0, 0, ChainTab::Sound);
    let names = || doc.borrow().project().groups[0].sounds[0].slots.iter().map(|s| s.module.clone()).collect::<Vec<_>>();
    assert_eq!(names(), ["Compressor", "Filter"]);
    crate::layout::apply_acts(&mut app, vec![Act::MoveSlot(1, 0)]);
    assert_eq!(names(), ["Filter", "Compressor"], "{}", app.status);
    crate::layout::apply_acts(&mut app, vec![Act::MoveSlot(0, 1)]);
    assert_eq!(names(), ["Compressor", "Filter"], "{}", app.status);
}

/// After the instrument, effects move; a side-chain stays with its effect.
#[test]
fn effects_move_after_the_instrument_with_their_side_chain() {
    let Some(mut h) = harness(false, false) else { return };
    let (g, s) = (h.state().group, h.state().sound);
    let doc = h.state().doc.clone().unwrap();
    let n = doc.borrow().project().groups[g].sounds[s].slots.len();
    doc.borrow_mut().add_slot(g, s, n, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
    doc.borrow_mut().add_slot(g, s, n + 1, &deckard_doc::NewModule::Effect("Filter")).unwrap();
    let other = if s == 0 { 1 } else { 0 };
    doc.borrow_mut().set_sidechain(g, s, n, Some(deckard_model::ScSource::Sound(g, other))).unwrap();
    h.state_mut().chain_tab = ChainTab::Sound;
    h.run_steps(2);
    let names = || doc.borrow().project().groups[g].sounds[s].slots.iter().map(|x| x.module.clone()).collect::<Vec<_>>();
    act(&mut h, Act::MoveSlot(n + 1, n));
    let after = names();
    assert_eq!(&after[n..], ["Filter", "Compressor"], "{}", h.state().status);
    assert_eq!(
        doc.borrow().project().groups[g].sounds[s].channel.sidechain,
        [(n + 1, deckard_model::ScSource::Sound(g, other))],
        "the side-chain moved with the Compressor"
    );
}

/// Side-chain sources are a sound's effects' only: a group's or the master's Compressor
/// offers none (choosing one used to set the focused sound's slot instead).
#[test]
fn only_a_sounds_effects_take_side_chains() {
    use crate::layout::channel::sidechain_choices;
    use deckard_doc::Chain;
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    let d = doc.borrow();
    let p = d.project();
    assert!(sidechain_choices(p, Chain::Sound(0, 0), 0).is_some());
    assert!(sidechain_choices(p, Chain::Group(0), 0).is_none());
    assert!(sidechain_choices(p, Chain::Master, 0).is_none());
}

/// The MK3's move (SHIFT + << >>) reaches the chain shown.
#[test]
fn the_mk3_moves_effects() {
    use deckard_lua_host::state::ControlRequest as R;
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
    doc.borrow_mut().add_slot(0, 0, 1, &deckard_doc::NewModule::Effect("Filter")).unwrap();
    (app.group, app.sound, app.chain_tab) = (0, 0, ChainTab::Sound);
    app.control_request(R::Move(1, 0));
    let names: Vec<String> = doc.borrow().project().groups[0].sounds[0].slots.iter().map(|s| s.module.clone()).collect();
    assert_eq!(names, ["Filter", "Compressor"], "{}", app.status);
    assert_eq!(app.slot, Some(0), "the moved effect stays focused");
}

/// The controller shows a plug-in's knobs as soon as it focuses another sound's (or
/// slot's) plug-in, not a frame later: the screens it sends are drawn then.
#[test]
fn the_controller_shows_the_knobs_of_the_plugin_it_moves_to() {
    use deckard_plugins::native::CLASSES;
    let mut app = crate::App::default();
    app.new_project();
    app.settings.controller = crate::settings::ControllerChoice::Deckard;
    let doc = app.doc.clone().unwrap();
    for (s, class) in CLASSES.iter().take(2).enumerate() {
        let new = deckard_doc::NewModule::Plugin { id: class.plugin_id(), name: class.name.into(), chunk: Vec::new() };
        doc.borrow_mut().add_slot(0, s, 0, &new).unwrap();
    }
    (app.group, app.sound, app.chain_tab, app.channel) = (0, 0, ChainTab::Sound, false);
    app.update_slot_view();
    let shown = |app: &crate::App| app.surface_view().control.knobs.iter().flatten().map(|k| k.name.clone()).collect::<Vec<_>>();
    assert!(!shown(&app).is_empty(), "{}", app.status);
    app.sound = 1;
    app.mk3_events(Vec::new(), &mut Vec::new());
    let names = shown(&app);
    assert!(!names.is_empty(), "the second sound's plug-in shows its knobs");
    assert_eq!(app.plugin_view.as_ref().map(|v| v.key), Some((Chain::Sound(0, 1), 0)));
}

/// The controller reaches an effect's Side-Chain Input page (RIGHT), its SOURCE a
/// routing choice the knob turns, as the Control area shows it.
#[test]
fn the_controller_shows_a_compressors_side_chain_page() {
    use deckard_lua_host::state::ControlRequest as R;
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    doc.borrow_mut().add_slot(0, 0, 1, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
    (app.group, app.sound, app.chain_tab, app.channel, app.slot) = (0, 0, ChainTab::Sound, false, Some(1));
    let c = app.control_snapshot([false; 8]);
    assert_eq!(c.pages, ["Main", "Side-Chain Input"]);
    app.control_request(R::Page(true));
    let c = app.control_snapshot([false; 8]);
    assert_eq!(c.page, 1, "RIGHT reaches the side-chain page");
    let names: Vec<Option<String>> = c.knobs.iter().map(|k| k.as_ref().map(|k| k.name.clone())).collect();
    assert_eq!(names[0].as_deref(), Some("SOURCE"), "{names:?}");
    assert_eq!(names[1].as_deref(), Some("GAIN"));
    let before = c.knobs[0].as_ref().unwrap().text.clone();
    // A turn past a routing step picks the next source.
    for _ in 0..4 {
        app.control_request(R::Turn { pos: 0, inc: 0.2, fine: false });
    }
    let after = app.control_snapshot([false; 8]).knobs[0].as_ref().unwrap().text.clone();
    assert_ne!(before, after, "the SOURCE knob picks another source");
}

/// DECKARD's own layer: the Plug-in page's SHIFT layer moves, bypasses and removes the
/// focused slot.
#[test]
fn the_own_layers_shift_layer_edits_the_focused_slot() {
    use deckard_surface::Action as A;
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
    doc.borrow_mut().add_slot(0, 0, 1, &deckard_doc::NewModule::Effect("Filter")).unwrap();
    (app.group, app.sound, app.chain_tab, app.slot) = (0, 0, ChainTab::Sound, Some(1));
    let slots = || doc.borrow().project().groups[0].sounds[0].slots.iter().map(|s| (s.module.clone(), s.active)).collect::<Vec<_>>();
    app.surface_act(A::SlotEdit("move_left"));
    assert_eq!(slots()[0].0, "Filter", "{}", app.status);
    assert_eq!(app.slot, Some(0), "it stays focused");
    app.surface_act(A::SlotEdit("move_right"));
    assert_eq!((slots()[1].0.as_str(), app.slot), ("Filter", Some(1)));
    app.surface_act(A::SlotEdit("bypass"));
    assert!(!slots()[1].1, "bypassed");
    app.surface_act(A::SlotEdit("bypass"));
    assert!(slots()[1].1, "on again");
    app.surface_act(A::SlotEdit("remove"));
    assert_eq!(slots().iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["Compressor"]);
}

/// The controller's browser offers a group the Perform FX (C40), one a group, first in
/// its chain as the software's "+" menu puts it; not a sound.
#[test]
fn the_controller_adds_a_groups_perform_fx() {
    use deckard_lua_host::state::ControlRequest as R;
    let mut app = crate::App::default();
    app.new_project();
    app.chain_tab = ChainTab::Group;
    let offered = |app: &crate::App| app.plugin_choices().into_iter().filter(|(e, _)| e.name.starts_with("Perform ")).count();
    assert_eq!(offered(&app), 8, "the eight modes");
    let stutter = app.plugin_choices().iter().position(|(e, _)| e.name == "Perform Stutter").unwrap();
    app.control_request(R::Load { entry: stutter, replace: None });
    let at = app.perform_slot(0).expect("added");
    assert_eq!(at, 0, "first in the chain");
    let mode = app.doc.as_ref().unwrap().borrow().project().groups[0].slots[at].param("mode");
    assert_eq!(mode, Some(5.0), "Stutter");
    assert_eq!(offered(&app), 0, "a group has one");
    app.chain_tab = ChainTab::Sound;
    app.doc.as_ref().unwrap().borrow_mut().remove_slot_in(Chain::Group(0), 0).unwrap();
    assert_eq!(offered(&app), 0, "not for a sound");
}
