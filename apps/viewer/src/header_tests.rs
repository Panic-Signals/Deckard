//! The header, in Maschine's order: the transport, then the metronome, BPM, SWING, SIG,
//! the position and scene, follow and the computer keyboard; on the right the controller
//! view, the master fader and the CPU meter.

use eframe::egui::{Event, PointerButton, Pos2, Rect};
use egui_kittest::Harness;

/// Where the header drew `name` (`crate::layout::header_rect`).
fn at(h: &Harness<'static, crate::App>, name: &str) -> Rect {
    h.ctx.data(|d| d.get_temp(crate::layout::header_rect(name))).unwrap_or_else(|| panic!("{name} drawn"))
}

fn click(h: &mut Harness<'static, crate::App>, pos: Pos2, double: bool) {
    h.hover_at(pos);
    h.step();
    for _ in 0..if double { 2 } else { 1 } {
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() });
            h.step();
        }
    }
    h.run_steps(2);
}

fn drag(h: &mut Harness<'static, crate::App>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for k in 1..=6 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 6.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
}

/// Left to right as Maschine's: transport, LINK, metronome, the display (BPM, SWING,
/// SIG), follow, keyboard; the master fader then the CPU meter at the right end.
#[test]
fn the_header_is_in_maschines_order() {
    let Some(h) = crate::tests::harness(false, false) else { return };
    let order = ["play", "record", "loop", "link", "metronome", "bpm", "swing", "sig", "follow", "keys"];
    for w in order.windows(2) {
        assert!(at(&h, w[0]).center().x < at(&h, w[1]).center().x, "{} before {}", w[0], w[1]);
    }
    assert!(at(&h, "master").right() < at(&h, "cpu").left() + 1.0, "the CPU meter last");
    // The transport group is centred.
    let (l, r) = (at(&h, "restart").left(), at(&h, "keys").right());
    let mid = h.ctx.content_rect().center().x;
    assert!(((l + r) / 2.0 - mid).abs() < 2.0, "centred: {l}..{r} about {mid}");
}

/// SWING shows the master swing; dragging it up raises it, a double-click types it, ⌘-click
/// sets 0 %. BPM types the same way.
#[test]
fn the_header_swing_sets_the_master_swing() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    let swing = |h: &Harness<'static, crate::App>| h.state().doc.as_ref().unwrap().borrow().project().swing.amount;
    let r = at(&h, "swing");
    drag(&mut h, r.center(), r.center() - eframe::egui::vec2(0.0, 40.0));
    assert!(swing(&h) > 5.0, "dragged up: {}", swing(&h));
    let enter = |h: &mut Harness<'static, crate::App>, typed: &str| {
        h.event(eframe::egui::Event::Text(typed.into()));
        h.step();
        h.event(eframe::egui::Event::Key {
            key: eframe::egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        });
        h.run_steps(3);
    };
    let r = at(&h, "swing");
    h.run_steps(40);
    click(&mut h, r.center(), true);
    h.run_steps(2);
    enter(&mut h, "12.5");
    assert_eq!(swing(&h), 12.5);
    h.run_steps(40);
    h.event(eframe::egui::Event::ModifiersChanged(eframe::egui::Modifiers::COMMAND));
    click(&mut h, r.center(), false);
    h.event(eframe::egui::Event::ModifiersChanged(Default::default()));
    h.run_steps(2);
    assert_eq!(swing(&h), 0.0);
    let r = at(&h, "bpm");
    h.run_steps(40);
    click(&mut h, r.center(), true);
    h.run_steps(2);
    enter(&mut h, "98.5");
    assert_eq!(h.state().doc.as_ref().unwrap().borrow().project().tempo, 98.5);
}

/// The keyboard button turns the computer keyboard (⌘K) on and off.
#[test]
fn the_header_keyboard_button() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    let r = at(&h, "keys");
    click(&mut h, r.center(), false);
    assert!(h.state().keys.on);
    click(&mut h, r.center(), false);
    assert!(!h.state().keys.on);
}

/// The CPU meter's load: the audio thread's busy time over the audio's time since the
/// last look.
#[test]
fn cpu_load_is_busy_over_audio_time() {
    use crate::layout::cpu_load;
    assert_eq!(cpu_load((0, 0), (0, 0)), None, "nothing played");
    assert_eq!(cpu_load((100, 1_000), (350, 2_000)), Some(0.25));
    assert_eq!(cpu_load((0, 0), (3_000, 1_000)), Some(1.0), "late blocks: full");
}

/// A new release shows a green "Update available" button between save and the
/// transport (no dot by the menus); clicking it opens About, where it updates.
#[test]
fn a_new_release_shows_an_update_button_before_the_transport() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    h.ctx.data_mut(|d| d.remove::<Rect>(crate::layout::header_rect("update")));
    h.run_steps(2);
    assert!(h.ctx.data(|d| d.get_temp::<Rect>(crate::layout::header_rect("update"))).is_none(), "none without an update");
    h.state_mut().update.check = crate::update::Check::Available(crate::github::Release {
        version: (9, 0, 0),
        tag: "9.0.0".into(),
        url: "https://github.com/Panic-Signals/Deckard/releases/tag/9.0.0".into(),
        assets: Vec::new(),
    });
    h.run_steps(2);
    let b = at(&h, "update");
    assert!(b.left() > 200.0 && b.right() < at(&h, "restart").left(), "after the menus and save, before restart: {b:?}");
    assert!(h.state().about.is_none());
    click(&mut h, b.center(), false);
    assert!(h.state().about.is_some(), "About opens, with its Update button");
}

/// Launched on a newer release than the last one run: it asks to open the release notes
/// (once); the same release, or a first run, doesn't.
#[test]
fn a_new_release_offers_its_release_notes_once() {
    let mut app = crate::App::default();
    app.after_update("0.2.0");
    assert_eq!(app.whats_new, None, "a first run");
    assert_eq!(app.settings.last_version, "0.2.0");
    app.after_update("0.2.0");
    assert_eq!(app.whats_new, None, "the same release");
    app.after_update("0.3.0");
    assert_eq!(app.whats_new.as_deref(), Some("0.3.0"));
    assert_eq!(app.settings.last_version, "0.3.0");
}

/// NEW on the MK3 with unsaved changes: the question is asked on its displays too, and
/// answered there (DON'T SAVE starts the new project; CANCEL keeps this one).
#[test]
fn the_mk3_asks_about_unsaved_changes_and_takes_the_answer() {
    use deckard_mk3::Event as Mk3;
    let press = |h: &mut Harness<'static, crate::App>, name: &'static str| {
        let mut reqs = Vec::new();
        h.state_mut().mk3_events(vec![Mk3::Button(name, true), Mk3::Button(name, false)], &mut reqs);
        h.run_steps(3);
    };
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    let was = h.state().path.clone();
    assert!(was.is_some());
    h.state().doc.as_ref().unwrap().borrow_mut().mark_unsaved();
    press(&mut h, "BUTTON_FILE");
    press(&mut h, "BUTTON_SCREEN_2");
    assert!(h.state().confirm.is_some(), "NEW asks");
    let ask = h.state().surface_view().ask.expect("the MK3 is told the question");
    assert!(ask.question.starts_with("Save changes to") && !ask.revert, "{ask:?}");
    // CANCEL: this project stays, the question goes.
    press(&mut h, "BUTTON_SCREEN_8");
    assert!(h.state().confirm.is_none() && h.state().path == was);
    assert!(h.state().surface_view().ask.is_none());
    // DON'T SAVE: the new project.
    press(&mut h, "BUTTON_SCREEN_2");
    assert!(h.state().confirm.is_some());
    press(&mut h, "BUTTON_SCREEN_6");
    assert!(h.state().confirm.is_none());
    assert!(h.state().path.is_none() && !h.state().dirty(), "an untitled new project");
}
