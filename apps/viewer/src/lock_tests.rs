//! Lock in the app: LOCK, Extended Lock's pads and buttons, morphing.

use deckard_doc::Target;
use egui_kittest::kittest::Queryable;
use mxprj::decode::Value;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app
}

fn level(app: &crate::App) -> f64 {
    app.doc.as_ref().unwrap().borrow().num(Target::Sound(0, 0), "level").unwrap()
}

fn set_level(app: &crate::App, v: f64) {
    app.doc.as_ref().unwrap().borrow_mut().set("Level", Target::Sound(0, 0), "level", Value::Float(v)).unwrap();
}

/// LOCK takes a snapshot and lights; changes are free; LOCK again recalls it and goes out.
#[test]
fn lock_takes_and_recalls() {
    let mut app = app();
    set_level(&app, 0.4);
    app.lock_button();
    assert!(app.lock_active);
    let doc = app.doc.clone().unwrap();
    assert_eq!(doc.borrow().project().lock.slots, [0], "the first free slot");
    set_level(&app, 0.9);
    app.lock_button();
    assert!(!app.lock_active);
    assert!((level(&app) - 0.4).abs() < 1e-6);
    // Again: the focused snapshot is updated, not a new one.
    set_level(&app, 0.6);
    app.lock_button();
    assert_eq!(doc.borrow().project().lock.slots, [0]);
    set_level(&app, 0.1);
    app.lock_button();
    assert!((level(&app) - 0.6).abs() < 1e-6);
}

/// Extended Lock: a free pad takes a snapshot there; a pad with one recalls it; DELETE;
/// the banks (▶ past the last adds one).
#[test]
fn extended_lock_pads_and_banks() {
    let mut app = app();
    let doc = app.doc.clone().unwrap();
    set_level(&app, 0.3);
    app.lock_pad(4);
    set_level(&app, 0.7);
    app.lock_pad(9);
    assert_eq!(doc.borrow().project().lock.slots, [4, 9]);
    app.lock_pad(4);
    assert!((level(&app) - 0.3).abs() < 1e-6, "recalled");
    assert_eq!(doc.borrow().project().lock.focus, Some(4));
    app.lock_delete();
    assert_eq!(doc.borrow().project().lock.slots, [9]);
    app.lock_bank(true);
    assert_eq!((doc.borrow().project().lock.banks, doc.borrow().project().lock.focus_bank), (2, 1), "a new bank");
    app.lock_pad(0);
    assert_eq!(doc.borrow().project().lock.slots, [9, 16]);
    app.lock_bank(false);
    assert_eq!(doc.borrow().project().lock.focus_bank, 0);
}

/// Morph on, Travel over 1 Bar (2 s at 120 BPM): halfway, the level is halfway; at the
/// end, the snapshot's.
#[test]
fn travel_morphs_over_the_time() {
    let mut app = app();
    set_level(&app, 0.2);
    app.lock_pad(0);
    set_level(&app, 0.8);
    app.lock_setting("morph", 1.0);
    app.lock_setting("morph_mode", 0.0);
    app.lock_setting("morph_time", 1.0);
    app.lock_recall_at(0, 10.0, None);
    assert!((level(&app) - 0.8).abs() < 1e-6, "not yet");
    app.lock_tick(11.0, None);
    assert!((level(&app) - 0.5).abs() < 0.02, "halfway: {}", level(&app));
    app.lock_tick(12.5, None);
    assert!((level(&app) - 0.2).abs() < 1e-6, "arrived");
    assert!(app.lock_morph.is_none());
}

/// Target, Bar: the morph arrives at the next downbeat of the playing song.
#[test]
fn target_morphs_to_the_next_bar() {
    let mut app = app();
    set_level(&app, 0.0);
    app.lock_pad(0);
    set_level(&app, 1.0);
    app.lock_setting("morph", 1.0);
    app.lock_setting("morph_mode", 1.0);
    app.lock_setting("morph_grid", 3.0);
    // Playing at tick 960 of a 3840-tick bar: 3 beats (1.5 s) to the downbeat.
    app.lock_recall_at(0, 0.0, Some(960.0));
    app.lock_tick(0.75, Some(960.0 + 1440.0));
    assert!((level(&app) - 0.5).abs() < 0.02, "halfway: {}", level(&app));
    app.lock_tick(1.6, Some(3840.0 + 100.0));
    assert!(level(&app).abs() < 1e-6);
}

/// The software's Lock panel (View ▸ Lock): LOCK, the bank's pads, UPDATE, DELETE, Morph.
#[test]
fn the_lock_panel() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    h.state_mut().menu_commands.push(crate::menu::Command::LockPanel);
    h.run_steps(3);
    assert!(h.state().lock_panel);
    let doc = h.state().doc.clone().unwrap();
    let click = |h: &mut egui_kittest::Harness<'static, crate::App>, label: &str| {
        h.get_by_label(label).click();
        h.run_steps(3);
    };
    click(&mut h, "LOCK");
    assert!(h.state().lock_active);
    let first = doc.borrow().project().lock.slots.clone();
    assert_eq!(first.len(), 1);
    // A free pad (the fifth) takes one there.
    click(&mut h, "Lock pad 5");
    assert!(doc.borrow().project().lock.slots.contains(&4));
    click(&mut h, "Lock pad 5");
    assert_eq!(doc.borrow().project().lock.focus, Some(4), "recalled and focused");
    click(&mut h, "DELETE");
    assert!(!doc.borrow().project().lock.slots.contains(&4));
    click(&mut h, "Morph");
    assert!(doc.borrow().project().lock.morph);
}

/// The MK3's requests reach the app's Lock.
#[test]
fn the_mk3_drives_lock() {
    use deckard_lua_host::state::{ControlRequest as R, LockOp};
    let mut app = app();
    let doc = app.doc.clone().unwrap();
    app.control_request(R::Lock(LockOp::Button));
    assert!(app.lock_active);
    app.control_request(R::Lock(LockOp::Pad(5)));
    assert!(doc.borrow().project().lock.slots.contains(&5));
    app.control_request(R::Lock(LockOp::Pad(5)));
    assert_eq!(doc.borrow().project().lock.focus, Some(5));
    app.control_request(R::Lock(LockOp::Update));
    app.control_request(R::Lock(LockOp::DeleteSlot(5)));
    assert!(!doc.borrow().project().lock.slots.contains(&5));
    app.control_request(R::Lock(LockOp::Bank(true)));
    assert_eq!(doc.borrow().project().lock.banks, 2);
}

/// DECKARD's own controller layer drives Lock, and sees it.
#[test]
fn the_own_layer_drives_lock() {
    use deckard_surface::Action as A;
    let mut app = app();
    let doc = app.doc.clone().unwrap();
    app.surface_act(A::Lock("button", 0));
    assert!(app.surface_view().lock.active);
    app.surface_act(A::Lock("pad", 5));
    app.surface_act(A::Lock("pad", 5));
    let v = app.surface_view().lock;
    assert!(v.slots.contains(&5));
    assert_eq!(v.focus, Some(5));
    app.surface_act(A::Lock("delete_slot", 5));
    assert!(!doc.borrow().project().lock.slots.contains(&5));
    app.surface_act(A::Lock("bank", 1));
    assert_eq!((app.surface_view().lock.bank, app.surface_view().lock.banks), (1, 2));
    app.surface_act(A::Lock("morph", 1));
    app.surface_act(A::Lock("morph_time", 3));
    let l = doc.borrow().project().lock.clone();
    assert!(l.morph);
    assert_eq!(l.time, 3);
}

/// Edit ▸ MIDI Change… sets Lock's MIDI change in the project.
#[test]
fn the_midi_change_dialog_sets_lock() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    h.state_mut().menu_commands.push(crate::menu::Command::MidiChange);
    h.run_steps(3);
    assert!(h.state().midi_change);
    h.get_by_label("Lock trigger").click();
    h.run_steps(2);
    h.get_by_label("Program Change").click();
    h.run_steps(3);
    let doc = h.state().doc.clone().unwrap();
    assert_eq!(doc.borrow().project().lock.midi_trigger, 2, "{}", h.state().status);
}
