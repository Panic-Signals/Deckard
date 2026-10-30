//! LFOs on DECKARD's own controller layer ([`deckard_surface::Action::Lfo`]).

use deckard_surface::Action as A;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app
}

#[test]
fn create_then_shape_then_remove() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 2, 0.0));
    let v = app.surface_view();
    assert_eq!(v.lfos[0].pad, 2);
    assert_eq!(v.lfo_focus, 2);
    app.surface_act(A::Lfo("shape", 2, 5.0));
    assert_eq!(app.surface_view().lfos[0].shape, "Random");
    app.surface_act(A::Lfo("remove", 2, 0.0));
    assert!(app.surface_view().lfos.is_empty());
}

#[test]
fn mute_toggles() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    assert!(!app.surface_view().lfos[0].muted);
    app.surface_act(A::Lfo("mute", 1, 0.0));
    assert!(app.surface_view().lfos[0].muted);
    app.surface_act(A::Lfo("mute", 1, 0.0));
    assert!(!app.surface_view().lfos[0].muted);
    app.surface_act(A::Lfo("mute", 4, 0.0));
    assert_eq!(app.surface_view().lfos.len(), 1, "no LFO on pad 4: nothing made");
}

#[test]
fn knob_steps_clamp() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    app.surface_act(A::Lfo("phase", 1, 10.0));
    app.surface_act(A::Lfo("phase", 1, 1000.0));
    assert!(app.surface_view().lfos[0].phase <= 360.0);
    app.surface_act(A::Lfo("quantize", 1, -1.0));
    assert_eq!(app.surface_view().lfos[0].quantize, 0, "Off stays Off");
    app.surface_act(A::Lfo("quantize", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].quantize, 2);
    assert_eq!(app.surface_view().lfos[0].rate, "1/4");
    app.surface_act(A::Lfo("rate", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].rate, "1/4.");
    app.surface_act(A::Lfo("level", 1, -500.0));
    assert_eq!(app.surface_view().lfos[0].level, 0.0);
    app.surface_act(A::Lfo("fade", 1, 1000.0));
    assert_eq!(app.surface_view().lfos[0].fade, 16.0);
}

#[test]
fn trigger_focused_uses_the_focused_sound() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    app.surface_act(A::FocusSound(2));
    app.surface_act(A::Lfo("trigger_focused", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S3");
    app.surface_act(A::Lfo("trigger_focused", 1, 0.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "Off");
    app.surface_act(A::Lfo("trigger", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S1");
}

#[test]
fn points_follow_phase() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    let before = app.lfo_views(true)[0].points.clone();
    assert_eq!(before.len(), 240);
    app.surface_act(A::Lfo("phase", 1, 18.0));
    assert_ne!(app.lfo_views(true)[0].points[0], before[0]);
}

/// The waves' pictures and the mod matrix's slots are made only while their MK3 pages
/// show.
#[test]
fn pictures_and_slots_only_on_their_pages() {
    use deckard_mk3::Event;
    use std::time::Instant;
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    let v = app.surface_view();
    assert!(v.lfos[0].points.is_empty() && v.mod_slots.is_empty(), "the Control page");
    for (name, down) in [("BUTTON_SHIFT", true), ("BUTTON_SELECT", true), ("BUTTON_SELECT", false)] {
        app.surface.event(&Event::Button(name, down), &v, Instant::now());
    }
    assert_eq!(app.surface_view().lfos[0].points.len(), 240, "the LFO page");
    app.surface.event(&Event::Button("BUTTON_SOLO", true), &v, Instant::now());
    let v = app.surface_view();
    assert!(v.lfos[0].points.is_empty(), "the mod matrix page");
    assert_eq!(v.mod_slots.len(), 1, "its empty slot");
}

/// "env" toggles Envelope mode; with no trigger it takes the focused sound (and keeps
/// it back in LFO mode); the name reads "ENV n"; "retrigger" toggles.
#[test]
fn env_mode_takes_the_focused_sound() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 3, 0.0));
    app.surface_act(A::FocusSound(1));
    app.surface_act(A::Lfo("env", 3, 0.0));
    let l = &app.surface_view().lfos[0];
    assert!(l.env);
    assert_eq!((l.name.as_str(), l.trigger.as_str()), ("ENV 3", "A1 / S2"));
    assert_eq!((l.attack.as_str(), l.decay.as_str(), l.release.as_str(), l.sustain), ("10 ms", "300 ms", "300 ms", 0.5));
    assert!(l.retrigger);
    app.surface_act(A::Lfo("retrigger", 3, 0.0));
    assert!(!app.surface_view().lfos[0].retrigger);
    app.surface_act(A::Lfo("env", 3, 0.0));
    let l = &app.surface_view().lfos[0];
    assert!(!l.env);
    assert_eq!((l.name.as_str(), l.trigger.as_str()), ("LFO 3", "A1 / S2"), "the trigger stays");
    assert!(!l.retrigger, "the envelope's settings kept");
    assert_eq!(l.note_off, None);
    app.surface_act(A::Lfo("env", 3, 0.0));
    assert!(app.lfo_views(true)[0].note_off.is_some());
}

/// Attack steps move the knob a hundredth of its exponential 1 ms–10 s travel, and stay
/// inside it; Sustain steps 1%.
#[test]
fn env_time_steps_are_exponential_and_clamped() {
    use deckard_model::lfo::{ENV_MAX, ENV_MIN, env_knob};
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    app.surface_act(A::Lfo("env", 1, 0.0));
    let attack = |app: &crate::App| app.doc.as_ref().unwrap().borrow().project().lfos[0].attack;
    let at = env_knob(attack(&app));
    app.surface_act(A::Lfo("attack", 1, 10.0));
    assert!((env_knob(attack(&app)) - (at + 0.1)).abs() < 1e-4);
    let a = attack(&app);
    app.surface_act(A::Lfo("attack", 1, 10.0));
    let b = attack(&app);
    app.surface_act(A::Lfo("attack", 1, 10.0));
    assert!(((b / a) - (attack(&app) / b)).abs() < 1e-3, "each step the same factor");
    app.surface_act(A::Lfo("attack", 1, 1000.0));
    assert_eq!(attack(&app), ENV_MAX);
    app.surface_act(A::Lfo("attack", 1, -1000.0));
    assert!((attack(&app) - ENV_MIN).abs() < 1e-7);
    assert_eq!(app.surface_view().lfos[0].attack, "1 ms");
    app.surface_act(A::Lfo("release", 1, 100.0));
    assert_eq!(app.surface_view().lfos[0].release, "10.00 s");
    app.surface_act(A::Lfo("sustain", 1, -3.0));
    assert!((app.surface_view().lfos[0].sustain - 0.47).abs() < 1e-6);
    app.surface_act(A::Lfo("sustain", 1, 500.0));
    assert_eq!(app.surface_view().lfos[0].sustain, 1.0);
}

/// In Envelope mode the trigger is never turned off: TRIG with 0 keeps it, and stepping
/// the Trigger knob skips Off.
#[test]
fn an_envelopes_trigger_never_goes_off() {
    let mut app = app();
    app.surface_act(A::Lfo("create", 1, 0.0));
    app.surface_act(A::Lfo("env", 1, 0.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S1");
    app.surface_act(A::Lfo("trigger_focused", 1, 0.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S1", "kept");
    app.surface_act(A::Lfo("trigger", 1, -5.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S1", "no Off");
    app.surface_act(A::Lfo("trigger", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S2");
    app.surface_act(A::FocusSound(4));
    app.surface_act(A::Lfo("trigger_focused", 1, 1.0));
    assert_eq!(app.surface_view().lfos[0].trigger, "A1 / S5");
}

/// An envelope whose trigger is gone shows "None".
#[test]
fn an_envelope_without_a_trigger_shows_none() {
    use deckard_model::lfo::Lfo;
    let mut app = app();
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::SetLfo(Lfo { env: true, ..Lfo::new(1) })]);
    assert_eq!(app.surface_view().lfos[0].trigger, "None");
}
