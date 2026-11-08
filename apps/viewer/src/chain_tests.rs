//! Group and master plug-in chains in the window: the chain area's GROUP and MASTER
//! tabs edit them as the SOUND tab edits a sound's, and the controller shows them.

use crate::layout::{self, Act, AddWhat, ChainTab};
use crate::tests::harness;
use deckard_doc::{Chain, Target};

/// A double-click on a plug-in's row asks for its window. This plug-in isn't installed, so
/// the answer is in the status line, and no window opens; no audio device opens either.
#[test]
fn double_clicking_a_plugins_row_asks_for_its_window() {
    use eframe::egui::{self, Event, PointerButton};
    let mut app = crate::App::default();
    app.new_project();
    app.output = Some(Err("no audio in this test".into()));
    let new = deckard_doc::NewModule::Plugin { id: "Native Instruments:VST3:FX:00000000".into(), name: "Absent".into(), chunk: Vec::new() };
    app.doc.as_ref().unwrap().borrow_mut().add_slot(0, 0, 0, &new).unwrap();
    (app.group, app.sound, app.chain_tab, app.channel) = (0, 0, ChainTab::Sound, false);
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1500.0 + layout::CONTROLLER_WIDTH, 930.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut crate::App| app.frame(ui), app);
    h.run_steps(3);
    let at = h.ctx.data(|d| d.get_temp::<egui::Rect>(layout::header_rect("chain slot 0"))).expect("the plug-in's row is drawn").center();
    let click = |h: &mut egui_kittest::Harness<'static, crate::App>, times: usize| {
        h.run_steps(40);
        h.hover_at(at);
        h.step();
        for _ in 0..times {
            for pressed in [true, false] {
                h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() });
                h.step();
            }
        }
        h.run_steps(2);
    };
    click(&mut h, 1);
    assert!(!h.state().status.starts_with("plug-in window"), "one click only selects it: {}", h.state().status);
    assert_eq!(h.state().slot, Some(0));
    click(&mut h, 2);
    assert!(h.state().status.starts_with("plug-in window"), "{}", h.state().status);
}

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

/// Side-chain sources are offered for a sound's and a group's effects (excluding self),
/// while the master's effects offer none.
#[test]
fn sounds_and_groups_take_side_chains_not_master() {
    use crate::layout::channel::sidechain_choices;
    use deckard_doc::Chain;
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    let g1 = doc.borrow_mut().add_group().unwrap();
    doc.borrow_mut().add_slot_in(Chain::Group(0), 0, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
    doc.borrow_mut().add_slot_in(Chain::Master, 0, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
    {
        let d = doc.borrow();
        let p = d.project();
        assert!(sidechain_choices(p, Chain::Sound(0, 0), 0).is_some());
        let (choices, _, _) = sidechain_choices(p, Chain::Group(0), 0).expect("group offers side-chain choices");
        assert!(!choices.iter().any(|c| c.starts_with("A1 ")), "a group cannot side-chain itself: {choices:?}");
        assert!(choices.iter().any(|c| c.starts_with("B1 ")), "another group is offered: {choices:?}");
        assert!(sidechain_choices(p, Chain::Master, 0).is_none());
    }
    (app.group, app.chain_tab, app.channel, app.slot) = (0, ChainTab::Group, false, Some(0));
    assert_eq!(app.control_snapshot([false; 8]).pages, ["Main", "Side-Chain Input"]);
    doc.borrow_mut().set_chain_sidechain(Chain::Group(0), 0, Some(deckard_model::ScSource::Group(g1))).unwrap();
    assert_eq!(doc.borrow().project().groups[0].channel.sidechain, [(0, deckard_model::ScSource::Group(g1))]);

    (app.chain_tab, app.channel, app.slot) = (ChainTab::Master, false, Some(0));
    assert_eq!(app.control_snapshot([false; 8]).pages, ["Main"], "master omits Side-Chain Input");
}

/// The 4-D encoder's left and right, and SHIFT's slot edits, stay on the chain
/// shown. Master does not become Group, and Group does not become Sound. An empty
/// chain does not change tab even when the sound has inserts.
#[test]
fn the_controller_stays_on_the_master_and_group_chains() {
    use deckard_lua_host::state::ControlRequest as R;
    use deckard_surface::Action as A;
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_slot_in(Chain::Master, 0, &deckard_doc::NewModule::Effect("EQ")).unwrap();
    doc.borrow_mut().add_slot_in(Chain::Master, 1, &deckard_doc::NewModule::Effect("Filter")).unwrap();
    doc.borrow_mut().add_slot_in(Chain::Group(0), 0, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
    doc.borrow_mut().add_slot_in(Chain::Group(0), 1, &deckard_doc::NewModule::Effect("Reverb")).unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Effect("Saturator")).unwrap();
    doc.borrow_mut().add_slot(0, 0, 1, &deckard_doc::NewModule::Effect("Limiter")).unwrap();
    let names = |c: Chain| doc.borrow().chain_slots(c).unwrap_or(&[]).iter().map(|s| (s.module.clone(), s.active)).collect::<Vec<_>>();

    (app.chain_tab, app.slot, app.slot_of) = (ChainTab::Master, Some(0), Some(Chain::Master));
    app.control_request(R::Slot(1));
    assert_eq!((app.chain_tab, app.slot), (ChainTab::Master, Some(1)), "right moves to the next master insert");
    app.control_request(R::Slot(1));
    assert_eq!((app.chain_tab, app.slot), (ChainTab::Master, None), "past the last is the +, still on Master");
    app.control_request(R::Slot(1));
    assert_eq!(app.chain_tab, ChainTab::Master, "nothing past the +, so the tab stays");
    app.control_request(R::Slot(-1));
    assert_eq!((app.chain_tab, app.slot), (ChainTab::Master, Some(1)), "left comes back onto the last insert");

    (app.chain_tab, app.slot, app.slot_of) = (ChainTab::Group, Some(1), Some(Chain::Group(0)));
    app.control_request(R::Slot(-1));
    assert_eq!((app.chain_tab, app.slot), (ChainTab::Group, Some(0)), "left stays on Group");
    app.control_request(R::Slot(1));
    assert_eq!((app.chain_tab, app.slot), (ChainTab::Group, Some(1)));
    app.surface_act(A::SlotEdit("bypass"));
    assert!(!names(Chain::Group(0))[1].1, "SHIFT bypasses the group's insert: {}", app.status);
    assert!(names(Chain::Sound(0, 0)).iter().all(|s| s.1), "the sound's inserts stay on");
    app.surface_act(A::SlotEdit("move_left"));
    assert_eq!(names(Chain::Group(0))[0].0, "Reverb");
    assert_eq!(app.chain_tab, ChainTab::Group);
    app.surface_act(A::SlotEdit("remove"));
    assert_eq!(names(Chain::Group(0)).iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["Compressor"]);
    assert_eq!(names(Chain::Sound(0, 0)).len(), 2, "remove doesn't take a sound insert");

    let eq = app.plugin_choices().iter().position(|(_, w)| matches!(w, AddWhat::Effect("EQ"))).expect("EQ offered");
    (app.chain_tab, app.slot, app.slot_of) = (ChainTab::Group, None, Some(Chain::Group(0)));
    app.control_request(R::Load { entry: eq, replace: None });
    assert_eq!(app.chain_tab, ChainTab::Group, "inserting stays on Group");
    assert!(names(Chain::Group(0)).iter().any(|s| s.0 == "EQ"), "the effect lands on the group");
    assert_eq!(names(Chain::Sound(0, 0)).len(), 2);

    doc.borrow_mut().remove_slot_in(Chain::Master, 0).unwrap();
    doc.borrow_mut().remove_slot_in(Chain::Master, 0).unwrap();
    (app.chain_tab, app.slot, app.slot_of) = (ChainTab::Master, None, Some(Chain::Master));
    app.control_request(R::Slot(1));
    assert_eq!((app.chain_tab, app.slot), (ChainTab::Master, None), "an empty chain does not change tab");
    assert_eq!(names(Chain::Sound(0, 0)).len(), 2);

    (app.chain_tab, app.slot, app.slot_of) = (ChainTab::Sound, Some(0), Some(Chain::Sound(0, 0)));
    app.control_request(R::Slot(1));
    assert_eq!((app.chain_tab, app.slot), (ChainTab::Sound, Some(1)), "Sound still steps its own inserts");
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
    assert_eq!(app.plugin_view().map(|v| v.key), Some((Chain::Sound(0, 1), 0)));
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

#[test]
fn a_side_chain_plugin_shows_its_side_chain_page_on_sound_and_group() {
    use deckard_doc::Chain;
    use deckard_lua_host::state::ControlRequest as R;
    struct Duck;
    impl deckard_plugins::native::Engine for Duck {
        fn set_param(&mut self, _: u32, _: f64) {}
        fn note_on(&mut self, _: u8, _: u8) {}
        fn note_off(&mut self, _: u8) {}
        fn reset(&mut self) {}
        fn has_sidechain(&self) -> bool {
            true
        }
        fn process(&mut self, _: &mut [f32], _: &mut [f32], _: &deckard_plugins::Transport) {}
    }
    static PARAMS: [deckard_plugins::native::Param; 1] =
        [deckard_plugins::native::Param { id: 0, name: "Mix", short: "MIX", default: 1.0, steps: 0 }];
    static PAGES: [deckard_plugins::native::Page; 1] = [deckard_plugins::native::Page { name: "Main", knobs: &[(0, "")] }];
    static DUCK_CLASS: deckard_plugins::native::Class = deckard_plugins::native::Class {
        cid: *b"DKRDVIEWERDUCK01",
        name: "Duck",
        instrument: false,
        params: &PARAMS,
        pages: &PAGES,
        make: |_| Box::new(Duck),
    };

    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    let plugin_id = DUCK_CLASS.plugin_id();
    let new = deckard_doc::NewModule::Plugin { id: plugin_id.clone(), name: "Duck".into(), chunk: Vec::new() };
    doc.borrow_mut().add_slot_in(Chain::Sound(0, 0), 0, &new).unwrap();
    doc.borrow_mut().add_slot_in(Chain::Group(0), 0, &new).unwrap();

    for (tab, chain) in [(ChainTab::Sound, Chain::Sound(0, 0)), (ChainTab::Group, Chain::Group(0))] {
        let id = deckard_plugins::state::PluginId::parse(&plugin_id, "Duck").unwrap();
        let host_slot = app.plugin_slot_key(chain, 0);
        let rate = app.output_rate();
        app.samples.plugins.insert(host_slot, deckard_plugins::Plugin::native(&DUCK_CLASS, id, "Duck", rate));
        (app.group, app.sound, app.chain_tab, app.channel, app.slot, app.plugin_page) = (0, 0, tab, false, Some(0), 0);
        app.update_slot_view();
        let c = app.control_snapshot([false; 8]);
        assert_eq!(c.pages, ["Main", "Side-Chain Input"], "{tab:?}");
        app.control_request(R::Page(true));
        let c = app.control_snapshot([false; 8]);
        assert_eq!(c.page, 1, "{tab:?}: RIGHT reaches the side-chain page");
        let names: Vec<Option<String>> = c.knobs.iter().map(|k| k.as_ref().map(|k| k.name.clone())).collect();
        assert_eq!(names[0].as_deref(), Some("SOURCE"), "{tab:?}: {names:?}");
        assert_eq!(names[1].as_deref(), Some("GAIN"));
        assert_eq!(names[2].as_deref(), Some("FILTER"));
        assert_eq!(names[3].as_deref(), Some("CENTER FREQ"));
        assert_eq!(names[4].as_deref(), Some("WIDTH"));
        for _ in 0..4 {
            app.control_request(R::Turn { pos: 0, inc: 0.2, fine: false });
        }
        let sc = match tab {
            ChainTab::Sound => doc.borrow().project().groups[0].sounds[0].channel.sidechain.clone(),
            ChainTab::Group => doc.borrow().project().groups[0].channel.sidechain.clone(),
            ChainTab::Master => unreachable!(),
        };
        assert!(!sc.is_empty(), "{tab:?}: turning SOURCE chooses a side-chain source");
    }
}
