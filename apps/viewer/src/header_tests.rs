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

/// SWING shows the master swing; dragging it up raises it, double-click sets 0 %.
#[test]
fn the_header_swing_sets_the_master_swing() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    let swing = |h: &Harness<'static, crate::App>| h.state().doc.as_ref().unwrap().borrow().project().swing.amount;
    let r = at(&h, "swing");
    drag(&mut h, r.center(), r.center() - eframe::egui::vec2(0.0, 40.0));
    assert!(swing(&h) > 5.0, "dragged up: {}", swing(&h));
    let r = at(&h, "swing");
    click(&mut h, r.center(), true);
    assert_eq!(swing(&h), 0.0);
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
