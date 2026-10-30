//! The Step Mod page of DECKARD's own controller layer (a step's pad held in Step
//! mode): the knobs write the held steps' modulation, a step's for that step alone
//! ([`deckard_surface::Action::StepMod`]).

use crate::layout::ChainTab;
use deckard_surface::Action as A;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    doc.borrow_mut().add_slot(0, 0, 1, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
    (app.group, app.sound, app.chain_tab, app.channel, app.slot) = (0, 0, ChainTab::Sound, false, Some(1));
    // (A pattern: Step mode's first step.)
    app.surface_step(0, 240, 100);
    app
}

/// The first knob shown that can be modulated.
fn knob(app: &mut crate::App) -> usize {
    let v = app.surface_view();
    v.control.knobs.iter().position(|k| k.as_ref().is_some_and(|k| k.modulatable)).expect("a knob to modulate")
}

fn points(app: &crate::App) -> Vec<(i64, f64)> {
    let d = app.doc.as_ref().unwrap().borrow();
    d.project().groups[0].patterns[0].mods.iter().map(|m| (m.position, (m.value * 1000.0).round() / 1000.0)).collect()
}

#[test]
fn a_held_steps_knob_writes_its_modulation() {
    let mut app = app();
    let k = knob(&mut app);
    let undo = app.doc.as_ref().unwrap().borrow().undo_labels().len();
    app.surface_act(A::StepModAt(Some((480, 240))));
    assert_eq!(app.surface_view().control.knobs[k].as_ref().unwrap().step_mod, None, "none yet");
    app.surface_act(A::StepMod { from: 0, length: 240, pads: 1 << 2, knob: k, inc: 0.25 });
    assert_eq!(app.surface_view().control.knobs[k].as_ref().unwrap().step_mod.as_deref(), Some("+25%"));
    // That step alone: back to none either side.
    assert_eq!(points(&app), [(479, 0.0), (480, 0.25), (719, 0.25), (720, 0.0)]);
    app.surface_act(A::StepMod { from: 0, length: 240, pads: 1 << 2 | 1 << 3, knob: k, inc: 0.25 });
    // (Step 4 starts where step 3's ended: at none.)
    assert_eq!(points(&app), [(479, 0.0), (480, 0.5), (719, 0.5), (720, 0.25), (959, 0.25), (960, 0.0)]);
    assert_eq!(app.surface_view().control.knobs[k].as_ref().unwrap().step_mod.as_deref(), Some("+50%"));
    // Let go: one undo step for it all.
    app.surface_act(A::StepModAt(None));
    assert_eq!(app.surface_view().control.knobs[k].as_ref().unwrap().step_mod, None);
    assert_eq!(app.doc.as_ref().unwrap().borrow().undo_labels().len(), undo + 1);
    assert!(app.doc.as_ref().unwrap().borrow_mut().undo());
    assert!(points(&app).is_empty());
}

/// With Auto-write on, each knob that can be modulated shows its modulation where the
/// song is (stopped: the focused pattern's start), as a percentage (C31).
#[test]
fn auto_write_shows_each_knobs_modulation() {
    let mut app = app();
    let k = knob(&mut app);
    let auto_mod = |app: &mut crate::App| app.surface_view().control.knobs[k].as_ref().unwrap().auto_mod.clone();
    assert_eq!(auto_mod(&mut app), None, "AUTO off");
    app.auto_write = true;
    assert_eq!(auto_mod(&mut app).as_deref(), Some("0%"), "none written");
    app.surface_act(A::StepModAt(Some((0, 240))));
    app.surface_act(A::StepMod { from: 0, length: 240, pads: 1, knob: k, inc: 0.25 });
    app.surface_act(A::StepModAt(None));
    assert_eq!(auto_mod(&mut app).as_deref(), Some("+25%"));
    let v = app.surface_view();
    assert!(v.control.knobs.iter().flatten().filter(|k| !k.modulatable).all(|k| k.auto_mod.is_none()));
}

/// Touch Auto-write (HARDWARE ▸ AUTO-WRITE): a knob held with Auto-write on while the song
/// plays writes its modulation, though it isn't turned.
#[test]
fn a_touched_knob_writes_with_touch_auto_write() {
    let mut app = app();
    let k = knob(&mut app);
    app.auto_write = true;
    let head = Some((deckard_engine::Mode::Scene(0), 100.0));
    app.touch_write_at(k, head);
    assert!(app.mod_take.is_none(), "Touch Auto-write off");
    app.settings.touch_auto_write = true;
    app.touch_write_at(k, None);
    assert!(app.mod_take.is_none(), "stopped");
    app.touch_write_at(k, head);
    assert!(app.mod_take.is_some(), "writing");
}
