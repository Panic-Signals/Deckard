//! Macros on the controller: MACRO shows the channel's macros (none until one is
//! made), and SHIFT + MACRO makes a touched knob's parameter one, plug-ins' too; MIDI
//! learn of plug-ins' parameters; and plug-ins' parameters in a macro's target menu.

use crate::layout;
use deckard_doc::Target;
use deckard_lua_host::state::ControlRequest as R;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app
}

/// The names of the knobs the controller shows.
fn shown(app: &crate::App) -> Vec<Option<String>> {
    app.control_snapshot([false; 8]).knobs.iter().map(|k| k.as_ref().map(|k| k.name.clone())).collect()
}

/// CHANNEL ▸ Macro of the focused sound.
fn macros(app: &mut crate::App) {
    (app.channel, app.chain_tab, app.prop_set, app.prop_page) = (true, layout::ChainTab::Sound, 3, 0);
}

#[test]
fn no_macros_show_no_knobs() {
    let mut app = app();
    macros(&mut app);
    assert_eq!(shown(&app), vec![None; 8]);
}

#[test]
fn an_assigned_macro_shows_alone() {
    let mut app = app();
    // Output ▸ Audio's LEVEL (knob 3) made the sound's macro.
    (app.channel, app.chain_tab, app.prop_set, app.prop_page) = (true, layout::ChainTab::Sound, 1, 0);
    app.control_request(R::SetMacro { pos: 2, level: 2 });
    macros(&mut app);
    let names = shown(&app);
    assert_eq!(names[0].as_deref(), Some("LEVEL"));
    assert!(names[1..].iter().all(Option::is_none), "{names:?}");
    let ch = Target::Sound(app.group, app.sound);
    assert!(app.doc.as_ref().unwrap().borrow().macro_pages(ch)[0].macros[0].target.is_some());
}

/// One of DECKARD's own plug-ins in the focused sound's first slot, its page on the
/// controller.
fn with_plugin() -> crate::App {
    use deckard_plugins::native::CLASSES;
    let mut app = app();
    let doc = app.doc.clone().unwrap();
    let class = &CLASSES[0];
    let new = deckard_doc::NewModule::Plugin { id: class.plugin_id(), name: class.name.into(), chunk: Vec::new() };
    doc.borrow_mut().add_slot(0, 0, 0, &new).unwrap();
    (app.group, app.sound, app.chain_tab, app.channel, app.slot) = (0, 0, layout::ChainTab::Sound, false, Some(0));
    app.update_slot_view();
    app
}

/// SHIFT + MACRO on a plug-in's page, a knob touched: its parameter is a macro of the
/// sound, shown on the sound's Macro page as on the plug-in's page, and turning it
/// there turns the plug-in. Touched again, it's taken off.
#[test]
fn a_plugin_knob_becomes_a_macro() {
    let mut app = with_plugin();
    let page = app.control_snapshot([false; 8]);
    let knob = page.knobs[1].clone().expect("the plug-in's second knob");
    app.control_request(R::SetMacro { pos: 1, level: 2 });
    let look = |knobs: &[Option<deckard_lua_host::state::ControlKnob>]| {
        knobs.iter().map(|k| k.as_ref().map(|k| (k.name.clone(), k.section.clone(), k.text.clone()))).collect::<Vec<_>>()
    };
    assert_eq!(look(&app.control_snapshot([false; 8]).knobs), look(&page.knobs), "the plug-in's page stays as it was");

    macros(&mut app);
    let m = app.control_snapshot([false; 8]).knobs[0].clone().expect("the macro");
    assert_eq!((&m.name, &m.text), (&knob.name, &knob.text), "{}", app.status);
    assert!(shown(&app)[1..].iter().all(Option::is_none));
    app.control_request(R::Turn { pos: 0, inc: 0.25, fine: false });
    let turned = app.control_snapshot([false; 8]).knobs[0].clone().unwrap();
    assert_ne!(turned.value, m.value, "the macro turns the plug-in's parameter");

    // The plug-in's own parameter (its page reads it again within 100 ms).
    let id = app.plugin_view.as_ref().unwrap().pages[0].knobs[1].0.id;
    let plugin = app.slot_plugin_in(deckard_doc::Chain::Sound(0, 0), 0).unwrap();
    assert_eq!(plugin.lock().unwrap().param_value(id) as f32, turned.value as f32, "the plug-in's parameter");
    (app.channel, app.slot) = (false, Some(0));
    app.control_request(R::SetMacro { pos: 1, level: 2 });
    macros(&mut app);
    assert_eq!(shown(&app), vec![None; 8], "taken off");
}

/// Making a plug-in's parameter a macro is one undo step.
#[test]
fn a_plugin_macro_undoes_in_one_step() {
    let mut app = with_plugin();
    app.control_request(R::SetMacro { pos: 0, level: 2 });
    let ch = Target::Sound(0, 0);
    let doc = app.doc.clone().unwrap();
    assert!(doc.borrow().macro_pages(ch)[0].macros[0].target.is_some());
    assert!(doc.borrow_mut().undo());
    assert!(doc.borrow().macro_pages(ch).iter().all(|p| p.macros.iter().all(|m| m.target.is_none())));
    assert!(doc.borrow().project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap().pages.is_empty(), "the pages too");
}

/// Which knobs the controller marks with the macro icon.
fn marked(app: &crate::App) -> Vec<bool> {
    app.surface_view().control.knobs.iter().map(|k| k.as_ref().is_some_and(|k| k.in_macro)).collect()
}

/// A parameter some macro controls shows the macro icon on its page: a channel's,
/// a plug-in's. The Macro page's own knobs don't (they are all macros).
#[test]
fn knobs_a_macro_controls_show_the_icon() {
    let mut app = with_plugin();
    assert_eq!(marked(&app), vec![false; 8]);
    app.control_request(R::SetMacro { pos: 1, level: 1 });
    assert_eq!(marked(&app)[..3], [false, true, false], "{}", app.status);

    (app.channel, app.prop_set, app.prop_page) = (true, 1, 0);
    app.control_request(R::SetMacro { pos: 2, level: 0 });
    assert_eq!(marked(&app)[..4], [false, false, true, false], "Output ▸ Audio's LEVEL, the master's macro");

    macros(&mut app);
    assert_eq!(marked(&app), vec![false; 8], "the Macro page");
}

/// A group's macro on one of its sounds' plug-ins turns it from the group's Macro page.
#[test]
fn a_group_macro_turns_a_sounds_plugin() {
    let mut app = with_plugin();
    app.control_request(R::SetMacro { pos: 1, level: 1 });
    let id = app.plugin_view.as_ref().unwrap().pages[0].knobs[1].0.id;
    (app.channel, app.chain_tab, app.prop_set, app.prop_page) = (true, layout::ChainTab::Group, 3, 0);
    let before = app.control_snapshot([false; 8]).knobs[0].clone().expect("the group's macro");
    app.control_request(R::Turn { pos: 0, inc: 0.25, fine: false });
    let plugin = app.slot_plugin_in(deckard_doc::Chain::Sound(0, 0), 0).unwrap();
    let now = plugin.lock().unwrap().param_value(id);
    assert_ne!(now as f32, before.value as f32, "the sound's plug-in turned");
    assert_eq!(app.control_snapshot([false; 8]).knobs[0].as_ref().map(|k| k.value as f32), Some(now as f32));
}

/// The window's right-click menu on a plug-in knob's name: Create Group Macro, then
/// Remove Group Macro.
#[test]
fn the_windows_menu_makes_a_plugin_knob_a_macro() {
    use crate::layout::{Act, MacroAct, apply_acts};
    let mut app = with_plugin();
    let id = app.plugin_view.as_ref().unwrap().pages[0].knobs[2].0.id;
    let chain = deckard_doc::Chain::Sound(0, 0);
    let toggle = || Act::Macro(MacroAct::Plugin { ch: Target::Group(0), chain, slot: 0, id });
    apply_acts(&mut app, vec![toggle()]);
    let doc = app.doc.clone().unwrap();
    let target = doc.borrow().macro_pages(Target::Group(0))[0].macros[0].target;
    assert_eq!(target.and_then(|t| doc.borrow().plugin_param_at(t)), Some((chain, 0, id)), "{}", app.status);
    assert!(marked(&app)[2], "the knob shows the icon");
    apply_acts(&mut app, vec![toggle()]);
    assert_eq!(doc.borrow().macro_pages(Target::Group(0))[0].macros[0].target, None);
}

/// MIDI Learn on a plug-in knob: the next control moved is learned (the plug-in's pages
/// saved so the parameter has an object, one undo step) and sets nothing; then it turns
/// the plug-in's parameter.
#[test]
fn a_plugin_knob_learns_a_midi_control() {
    let mut app = with_plugin();
    let chain = deckard_doc::Chain::Sound(0, 0);
    let id = app.plugin_view.as_ref().unwrap().pages[0].knobs[1].0.id;
    let plugin = app.slot_plugin_in(chain, 0).unwrap();
    let before = plugin.lock().unwrap().param_value(id);
    app.toggle_plugin_midi_learn(chain, 0, id);
    app.midi_remote(74, 0.3);
    let doc = app.doc.clone().unwrap();
    let param = doc.borrow().plugin_param_oid(chain, 0, id).expect("the parameter's object");
    assert_eq!(doc.borrow().midi_key_for(param), Some(74), "{}", app.status);
    assert_eq!(app.remote.plugin_learn, None, "learned once");
    assert_eq!(plugin.lock().unwrap().param_value(id), before, "learning sets nothing");
    assert!(doc.borrow_mut().undo());
    assert_eq!(doc.borrow().plugin_param_oid(chain, 0, id), None, "one undo step, the pages too");
    assert!(doc.borrow_mut().redo());
    // (The ends: the parameter may be stepped.)
    for v in [1.0, 0.0] {
        app.midi_remote(74, v);
        assert_eq!(plugin.lock().unwrap().param_value(id), v, "the control turns it");
    }
}

/// A MIDI control learned by a macro of a plug-in's parameter turns the plug-in.
#[test]
fn midi_turns_a_plugin_macro() {
    let mut app = with_plugin();
    let chain = deckard_doc::Chain::Sound(0, 0);
    let id = app.plugin_view.as_ref().unwrap().pages[0].knobs[1].0.id;
    app.control_request(R::SetMacro { pos: 1, level: 1 });
    let doc = app.doc.clone().unwrap();
    let m = doc.borrow().macro_oid(Target::Group(0), 0, 0).unwrap();
    doc.borrow_mut().learn_midi(m, 20).unwrap();
    let plugin = app.slot_plugin_in(chain, 0).unwrap();
    for v in [1.0, 0.0] {
        app.midi_remote(20, v);
        assert_eq!(plugin.lock().unwrap().param_value(id), v);
    }
}

/// The window's Assignment area on a plug-in's page: a knob's Learn waits for a
/// control, then shows it.
#[test]
fn the_assignment_area_learns_a_plugin_knob() {
    use egui_kittest::kittest::Queryable;
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    let chain = deckard_doc::Chain::Sound(0, 0);
    {
        use deckard_plugins::native::CLASSES;
        let app = h.state_mut();
        let class = &CLASSES[0];
        let new = deckard_doc::NewModule::Plugin { id: class.plugin_id(), name: class.name.into(), chunk: Vec::new() };
        app.doc.clone().unwrap().borrow_mut().add_slot(0, 0, 0, &new).unwrap();
        (app.group, app.sound, app.chain_tab, app.channel, app.slot) = (0, 0, layout::ChainTab::Sound, false, Some(0));
        app.update_slot_view();
    }
    h.run_steps(3);
    let id = h.state().plugin_view.as_ref().unwrap().pages[0].knobs[1].0.id;
    h.get_by_label("Assignment area").click();
    h.run_steps(3);
    h.get_all_by_label("Learn").nth(1).unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().remote.plugin_learn, Some((chain, 0, id)));
    h.state_mut().midi_remote(21, 0.5);
    h.run_steps(3);
    assert!(h.query_by_label("CC 21").is_some());
}

/// A macro's target menu lists the plug-ins' parameters of the channels it reaches,
/// under the plug-in (slot and name), its page and the knob's name: a sound's own, and
/// a group's its sounds'.
#[test]
fn the_target_menu_lists_plugin_parameters() {
    use layout::channel::Level;
    let app = with_plugin();
    let page = &app.plugin_view.as_ref().unwrap().pages[0];
    let (info, name, _) = &page.knobs[1];
    let plugin = app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap().name.clone();
    let sound = app.plugin_targets(Level::Sound(0, 0));
    let knob = if name.is_empty() { info.title.clone() } else { name.clone() };
    let at = sound.iter().find(|t| t.1 == (deckard_doc::Chain::Sound(0, 0), 0, info.id)).expect("the knob's parameter");
    assert_eq!(at.0, vec![format!("1 - {plugin}"), page.name.clone(), knob.clone()]);
    let group = app.plugin_targets(Level::Group(0));
    let at = group.iter().find(|t| t.1 == (deckard_doc::Chain::Sound(0, 0), 0, info.id)).expect("the sound's, from the group");
    assert_eq!(at.0[..2], ["Sounds".to_string(), "S1 - Sound 1".to_string()]);
    assert!(
        app.plugin_targets(Level::Master).iter().all(|t| t.1.0 != deckard_doc::Chain::Sound(0, 0)),
        "the master reaches groups, not sounds"
    );
}

/// Picking a plug-in's parameter for an empty macro points it there (the pages saved),
/// one undo step.
#[test]
fn an_empty_macro_picks_a_plugin_parameter() {
    use crate::layout::{Act, MacroAct, apply_acts};
    let mut app = with_plugin();
    let chain = deckard_doc::Chain::Sound(0, 0);
    let id = app.plugin_view.as_ref().unwrap().pages[0].knobs[1].0.id;
    let ch = Target::Sound(0, 0);
    apply_acts(&mut app, vec![Act::Macro(MacroAct::AssignPlugin { ch, page: 0, slot: 3, chain, plugin_slot: 0, id })]);
    let doc = app.doc.clone().unwrap();
    let target = doc.borrow().macro_pages(ch)[0].macros[3].target;
    assert_eq!(target.and_then(|t| doc.borrow().plugin_param_at(t)), Some((chain, 0, id)), "{}", app.status);
    assert!(doc.borrow_mut().undo());
    assert_eq!(doc.borrow().macro_pages(ch)[0].macros[3].target, None);
    assert_eq!(doc.borrow().plugin_param_oid(chain, 0, id), None, "one undo step, the pages too");
}

/// A plug-in that isn't the one shown still offers its parameters, and can be picked.
#[test]
fn a_plugin_not_shown_is_offered_too() {
    use crate::layout::{Act, MacroAct, apply_acts};
    let mut app = with_plugin();
    let chain = deckard_doc::Chain::Sound(0, 0);
    let id = app.plugin_view.as_ref().unwrap().pages[0].knobs[1].0.id;
    // The sound's Macro properties: no plug-in page shown.
    macros(&mut app);
    app.plugin_view = None;
    assert!(app.plugin_targets(layout::channel::Level::Sound(0, 0)).iter().any(|t| t.1 == (chain, 0, id)));
    let ch = Target::Sound(0, 0);
    apply_acts(&mut app, vec![Act::Macro(MacroAct::AssignPlugin { ch, page: 0, slot: 0, chain, plugin_slot: 0, id })]);
    let doc = app.doc.clone().unwrap();
    let target = doc.borrow().macro_pages(ch)[0].macros[0].target;
    assert_eq!(target.and_then(|t| doc.borrow().plugin_param_at(t)), Some((chain, 0, id)), "{}", app.status);
}
