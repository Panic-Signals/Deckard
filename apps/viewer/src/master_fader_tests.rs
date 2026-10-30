//! The header's master fader, at its right end: dragging its level line sets the
//! master level, a double-click sets 0 dB; undoable.

use eframe::egui::{Event, PointerButton, Pos2};
use egui_kittest::Harness;

fn level(h: &Harness<'static, crate::App>) -> f64 {
    h.state().doc.as_ref().unwrap().borrow().project().master_level
}

/// A point along the fader's meters (0 left … 1 right).
fn track(h: &Harness<'static, crate::App>, x: f32) -> Pos2 {
    let r: eframe::egui::Rect = h.ctx.data(|d| d.get_temp(crate::layout::header_rect("master"))).expect("master drawn");
    Pos2::new(r.left() + r.width() * x, r.center().y)
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

#[test]
fn the_header_fader_sets_the_master_level() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    let start = level(&h);
    let (a, b) = (track(&h, 0.5), track(&h, 0.3));
    drag(&mut h, a, b);
    let lower = level(&h);
    assert!(lower < start - 0.1, "dragged left: {start} → {lower}");
    let (a, b) = (track(&h, 0.3), track(&h, 0.6));
    drag(&mut h, a, b);
    assert!(level(&h) > lower + 0.2, "dragged right");
    // Undoable, as the Mixer's fader (moves close together are one step).
    h.state().doc.as_ref().unwrap().borrow_mut().undo();
    assert!((level(&h) - start).abs() < 1e-9, "undo takes the drags back");
    // Double-click: 0 dB.
    let at = track(&h, 0.2);
    h.hover_at(at);
    h.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() });
            h.step();
        }
    }
    h.run_steps(3);
    assert!((level(&h) - 0.75).abs() < 1e-9, "0 dB: {}", level(&h));
}
