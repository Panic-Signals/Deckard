//! SHIFT + a pad on DECKARD's own controller layer: the edits printed on the pads
//! ([`deckard_surface::Action::Edit`]) and SHIFT + ERASE + a pad
//! ([`deckard_surface::Action::ResetSound`]).

use deckard_surface::Action as A;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app
}

fn notes(app: &crate::App) -> usize {
    let d = app.doc.as_ref().unwrap().borrow();
    d.project().groups[app.group].patterns.get(app.pattern).map_or(0, |p| p.notes.len())
}

#[test]
fn the_shift_pads_undo_redo_copy_and_paste() {
    let mut app = app();
    app.surface_step(0, 240, 100);
    app.surface_step(240, 240, 100);
    assert_eq!(notes(&app), 2);
    app.surface_act(A::Edit("undo"));
    assert_eq!(notes(&app), 1);
    app.surface_act(A::Edit("redo"));
    assert_eq!(notes(&app), 2);
    app.surface_act(A::Edit("undo_step"));
    assert_eq!(notes(&app), 1);
    app.surface_act(A::Edit("redo_step"));
    assert_eq!(notes(&app), 2);
    // (The step's note is selected: copied, and pasted in.)
    app.surface_act(A::Edit("copy"));
    app.surface_act(A::Edit("paste"));
    assert_eq!(notes(&app), 3);
    app.surface_act(A::Edit("clear_modulation"));
    assert_eq!(notes(&app), 3, "the notes stay");
}

#[test]
fn shift_erase_and_a_pad_removes_its_sound() {
    let mut app = app();
    let name = |app: &crate::App| app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[1].name.clone();
    app.doc.as_ref().unwrap().borrow_mut().rename(deckard_doc::Target::Sound(0, 1), "Kick").unwrap();
    assert_eq!(name(&app), "Kick");
    app.surface_act(A::ResetSound(1));
    assert_eq!(name(&app), "Sound 2", "Maschine's Reset");
}

#[test]
fn prob_lock_locks_the_focused_sounds_chances() {
    let mut app = app();
    app.sound = 2;
    app.surface_act(A::ProbLock(true));
    assert!(app.surface_view().groups[0].sounds[2].prob_locked);
    let p = app.doc.as_ref().unwrap().borrow().project().clone();
    let t = deckard_engine::timeline(&p, deckard_engine::Mode::Arrangement);
    assert_eq!(t.prob_locked, [(0, 2)], "the engine keeps its chances");
    app.surface_act(A::ProbLock(false));
    assert!(!app.surface_view().groups[0].sounds[2].prob_locked);
}

#[test]
fn a_double_tapped_knob_sets_the_selected_notes_to_100() {
    let mut app = app();
    app.surface_step(0, 240, 40);
    let note = |app: &crate::App| app.doc.as_ref().unwrap().borrow().project().groups[app.group].patterns[app.pattern].notes[0];
    app.surface_act(A::Events("probability", -30));
    assert_eq!((note(&app).velocity, note(&app).probability), (40, 70));
    app.surface_act(A::Events("velocity_to", 100));
    app.surface_act(A::Events("probability_to", 100));
    assert_eq!((note(&app).velocity, note(&app).probability), (100, 100));
}

/// The Settings page's metronome VOLUME and AUTOENABLE (C32): stepped, shown and kept.
#[test]
fn the_metronome_volume_and_autoenable_are_settings() {
    let mut app = app();
    assert_eq!((app.settings.metronome_volume, app.settings.metronome_auto), (100, false));
    app.surface_act(A::Setting("metronome_volume", 0, -10));
    app.surface_act(A::Setting("metronome_auto", 0, 1));
    assert_eq!((app.settings.metronome_volume, app.settings.metronome_auto), (90, true));
    let v = app.settings_view();
    assert_eq!(v.values.get("metronome_volume").map(String::as_str), Some("90%"));
    assert_eq!(v.values.get("metronome_auto").map(String::as_str), Some("On"));
    let kept = crate::settings::Settings::parse(&app.settings.to_text());
    assert_eq!((kept.metronome_volume, kept.metronome_auto), (90, true));
}

#[test]
fn the_hardware_settings_are_settings() {
    let mut app = app();
    let s = &app.settings;
    assert_eq!((s.velocity_scaling, s.led_brightness, s.strip_position, s.touch_auto_write), (0, 100, 2, false));
    app.surface_act(A::Setting("velocity_scaling", 0, -2));
    app.surface_act(A::Setting("led_brightness", 0, -50));
    app.surface_act(A::Setting("strip_position", 0, -1));
    app.surface_act(A::Setting("touch_auto_write", 0, 1));
    let v = app.settings_view();
    let shown = |k: &str| v.values.get(k).cloned().unwrap_or_default();
    assert_eq!(
        [shown("velocity_scaling"), shown("led_brightness"), shown("strip_position"), shown("touch_auto_write")],
        ["Soft 2", "50%", "Rec. Only", "On"]
    );
    let kept = crate::settings::Settings::parse(&app.settings.to_text());
    assert_eq!((kept.velocity_scaling, kept.led_brightness, kept.strip_position, kept.touch_auto_write), (-2, 50, 1, true));
    // What the pads play and the strip and lights show follow them.
    assert_eq!(app.surface_view().strip_position, 1);
    assert!(app.surface_view().touch_auto_write);
}

/// C50: SHIFT + a group button from the MK3, through the app: bank 2's first group.
#[test]
fn shift_and_a_group_button_pick_the_bank_from_the_mk3() {
    use deckard_mk3::Event as E;
    let mut app = app();
    while app.doc.as_ref().unwrap().borrow().project().groups.len() < 10 {
        app.surface_act(A::AddGroup);
    }
    app.focus_group(2);
    let mut requests = Vec::new();
    for e in [
        E::Button("BUTTON_SHIFT", true),
        E::Button("BUTTON_GROUP_B", true),
        E::Button("BUTTON_GROUP_B", false),
        E::Button("BUTTON_SHIFT", false),
    ] {
        app.mk3_events(vec![e], &mut requests);
    }
    assert_eq!(app.group, 8);
    // All at once, as a fast press comes in one frame.
    app.focus_group(2);
    app.mk3_events(
        vec![
            E::Button("BUTTON_SHIFT", true),
            E::Button("BUTTON_GROUP_B", true),
            E::Button("BUTTON_GROUP_B", false),
            E::Button("BUTTON_SHIFT", false),
        ],
        &mut requests,
    );
    assert_eq!(app.group, 8, "in one frame");
}
