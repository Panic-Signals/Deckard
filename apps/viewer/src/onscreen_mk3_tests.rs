//! The panel core's contract, with no device and no window: pointer contact in,
//! `deckard_mk3::Event`s out. Every test drives `Panel` directly — no egui context exists here.

use crate::mk3_layout;
use crate::mk3_sink::Mk3Sink;
use crate::onscreen_mk3::Panel;
use deckard_mk3::Event;
use eframe::egui;

/// The middle of the named control: every gesture below starts from a known control.
fn center(id: &str) -> egui::Pos2 {
    mk3_layout::control(id).unwrap_or_else(|| panic!("{id} is in the table")).rect.center()
}

#[test]
fn a_pad_press_emits_pad_on_with_velocity() {
    let mut panel = Panel::default();
    panel.pointer_down(0, center("pad1"), None);
    let events = panel.take();
    assert_eq!(events.len(), 1, "one event for the press");
    let Event::PadOn(pad, velocity) = events[0] else { panic!("a pad press is a PadOn, got {:?}", events[0]) };
    assert_eq!(pad, 1, "the middle of pad 1 is pad 1");
    assert!((0.0..=1.0).contains(&velocity), "velocity {velocity} is inside 0..=1");
    // A caller with pressure decides the velocity.
    let mut panel = Panel::default();
    panel.pointer_down(0, center("pad2"), Some(0.25));
    assert_eq!(panel.take(), vec![Event::PadOn(2, 0.25)], "the caller's pressure is the velocity");
}

#[test]
fn a_knob_drag_emits_turns_and_a_dead_zone() {
    let mut panel = Panel::default();
    panel.pointer_down(0, center("knob1"), None);
    assert_eq!(panel.take(), Vec::new(), "touching a knob turns nothing");
    // 1 px is inside the dead zone: still nothing.
    panel.pointer_move(0, center("knob1") + egui::vec2(0.0, -1.0));
    assert_eq!(panel.take(), Vec::new(), "1 px moves nothing");
    // 6 px past the dead zone's 3 still earns no step: the first step lands at 3 + 6 = 9 px.
    panel.pointer_move(0, center("knob1") + egui::vec2(0.0, -6.0));
    assert_eq!(panel.take(), Vec::new(), "6 px is still no step");
    // Past the threshold: one step, and up reads clockwise (positive).
    panel.pointer_move(0, center("knob1") + egui::vec2(0.0, -9.0));
    let events = panel.take();
    assert_eq!(events.len(), 1, "one step past the dead zone is one turn event");
    let Event::Knob(index, inc) = events[0] else { panic!("a knob drag is a Knob, got {:?}", events[0]) };
    assert_eq!(index, 0, "knob 1 is index 0");
    assert!(inc > 0.0, "an up drag turns clockwise (inc {inc})");
    // Back down past the threshold turns the other way.
    panel.pointer_move(0, center("knob1") + egui::vec2(0.0, 10.0));
    let events = panel.take();
    assert_eq!(events.len(), 1, "the return drag is one turn event");
    let Event::Knob(index, inc) = events[0] else { panic!("a knob drag is a Knob, got {:?}", events[0]) };
    assert_eq!(index, 0, "still knob 1");
    assert!(inc < 0.0, "a down drag turns counter-clockwise (inc {inc})");
}

#[test]
fn wheel_ticks_and_jog_drags_turn() {
    let mut panel = Panel::default();
    panel.wheel(0);
    assert_eq!(panel.take(), Vec::new(), "zero ticks are silent");
    panel.wheel(3);
    assert_eq!(panel.take(), vec![Event::Wheel(3)], "ticks report clockwise");
    // The jog turns like a knob, from its own surface: the wheel's push sits at its
    // centre and the four directions ring its pips, so the probe takes a point the ring
    // leaves free (the disc's lower right).
    let jog = mk3_layout::control("jog").expect("jog is in the table");
    let at_jog = jog.rect.center() + egui::vec2(jog.rect.width() * 0.3, jog.rect.height() * 0.3);
    assert_eq!(mk3_layout::at(at_jog).map(|c| c.id), Some("jog"), "the probe point lands on the jog, not on the ring drawn over it");
    panel.pointer_down(0, at_jog, None);
    assert_eq!(panel.take(), Vec::new(), "touching the jog turns nothing");
    panel.pointer_move(0, at_jog + egui::vec2(0.0, -6.0));
    assert_eq!(panel.take(), Vec::new(), "6 px is still no click");
    panel.pointer_move(0, at_jog + egui::vec2(0.0, -9.0));
    assert_eq!(panel.take(), vec![Event::Wheel(1)], "past the threshold: one clockwise click");
    panel.pointer_move(0, at_jog + egui::vec2(0.0, 9.0));
    assert_eq!(panel.take(), vec![Event::Wheel(-2)], "the return drag clicks back twice");
}

#[test]
fn two_fingers_on_two_pads_are_independent() {
    let mut panel = Panel::default();
    panel.pointer_down(0, center("pad1"), None);
    panel.pointer_down(1, center("pad2"), None);
    assert_eq!(panel.take().len(), 2, "each finger presses its own pad");
    panel.pointer_up(0);
    assert_eq!(panel.take(), vec![Event::PadOff(1)], "lifting finger 0 lifts only pad 1");
    // Finger 1 still holds pad 2: a move still tracks it, and its lift still releases it.
    panel.pointer_move(1, center("pad2") + egui::vec2(0.0, 1.0));
    assert_eq!(panel.take(), Vec::new(), "1 px moves no pressure (a pixel is ~0.013, well under the eps)");
    panel.pointer_move(1, center("pad2") + egui::vec2(0.0, 20.0));
    assert!(matches!(panel.take().as_slice(), [Event::PadPressure(2, _)]), "finger 1 still holds pad 2 after finger 0 lifted");
    panel.pointer_up(1);
    assert_eq!(panel.take(), vec![Event::PadOff(2)], "lifting finger 1 lifts pad 2");
}

#[test]
fn release_all_lifts_everything_held() {
    let mut panel = Panel::default();
    panel.pointer_down(0, center("pad3"), None);
    panel.pointer_down(1, center("BUTTON_SHIFT"), None);
    panel.pointer_down(2, center("pad1"), None);
    panel.pointer_down(3, center("strip"), None);
    let down = panel.take();
    assert_eq!(down.len(), 4, "down queues the press for every held control");
    assert_eq!(
        &down[..3],
        &[Event::PadOn(3, 0.8), Event::Button("BUTTON_SHIFT", true), Event::PadOn(1, 0.8)],
        "down queues the press for every held control",
    );
    // The strip reports the pointer's horizontal fraction: the table's centre
    // is 0.5 up to f32 rounding, so the middle reads approximately.
    let Event::Touch(slot, kind, pos) = down[3] else { panic!("the strip down is a Touch, got {:?}", down[3]) };
    assert_eq!((slot, kind), (0, 0), "the first strip touch is finger 0, touched");
    assert!((pos - 0.5).abs() < 1e-6, "the strip down reports the middle: {pos}");
    panel.release_all();
    panel.release_all();
    let up = panel.take();
    assert_eq!(up.len(), 4, "release_all drains every held gesture");
    assert_eq!(
        &up[..3],
        &[Event::PadOff(1), Event::PadOff(3), Event::Button("BUTTON_SHIFT", false)],
        "pads by number, then buttons in press order, then strip fingers by slot",
    );
    let Event::Touch(slot, kind, pos) = up[3] else { panic!("the strip release is a Touch, got {:?}", up[3]) };
    assert_eq!((slot, kind), (0, 2), "the strip finger releases");
    assert!((pos - 0.5).abs() < 1e-6, "the release reports where the finger was: {pos}");
    panel.release_all();
    assert_eq!(panel.take(), Vec::new(), "release_all clears: a second call emits nothing");
}

#[test]
fn a_touch_strip_drag_emits_touch_positions() {
    let strip = mk3_layout::control("strip").expect("strip is in the table");
    let at = |frac: f32| egui::pos2(strip.rect.min.x + strip.rect.width() * frac, strip.rect.center().y);
    let mut panel = Panel::default();
    panel.pointer_down(0, at(0.25), None);
    let down = panel.take();
    assert_eq!(down.len(), 1, "one event for the touch");
    let Event::Touch(finger, kind, pos) = down[0] else { panic!("a strip touch is a Touch, got {:?}", down[0]) };
    assert_eq!((finger, kind), (0, 0), "the first strip touch is finger 0, touched");
    assert!((pos - 0.25).abs() < 1e-6, "pos {pos} is where the finger landed");
    panel.pointer_move(0, at(0.75));
    let moved = panel.take();
    assert_eq!(moved.len(), 1, "one event for the move");
    let Event::Touch(finger, kind, pos) = moved[0] else { panic!("a strip move is a Touch, got {:?}", moved[0]) };
    assert_eq!((finger, kind), (0, 1), "the same finger, moved");
    assert!((pos - 0.75).abs() < 1e-6, "pos {pos} follows the finger");
    panel.pointer_up(0);
    let up = panel.take();
    assert_eq!(up.len(), 1, "one event for the lift");
    let Event::Touch(finger, kind, pos) = up[0] else { panic!("a strip lift is a Touch, got {:?}", up[0]) };
    assert_eq!((finger, kind), (0, 2), "the same finger, released");
    assert!((pos - 0.75).abs() < 1e-6, "the release reports where the finger was");
}

#[test]
fn a_finger_on_a_held_control_does_not_disturb_the_first() {
    let mut panel = Panel::default();
    panel.pointer_down(0, center("pad1"), None);
    panel.pointer_down(1, center("pad1"), None);
    assert!(
        matches!(panel.take().as_slice(), [Event::PadOn(1, _), Event::PadOn(1, _)]),
        "both fingers press: two PadOns, and the second neither clears nor replaces the first"
    );
    panel.pointer_up(0);
    assert_eq!(panel.take(), vec![Event::PadOff(1)], "lifting the first releases once");
    // The second finger still holds pad 1.
    panel.pointer_move(1, center("pad1") + egui::vec2(0.0, 20.0));
    assert!(matches!(panel.take().as_slice(), [Event::PadPressure(1, _)]), "the second finger still tracks pad 1 after the first lifted");
    panel.pointer_up(1);
    assert_eq!(panel.take(), vec![Event::PadOff(1)], "the second lift releases again");
}

#[test]
fn gestures_keep_the_down_control() {
    let mut panel = Panel::default();
    // A pad drag that wanders onto another pad still reports the pad it started on.
    panel.pointer_down(0, center("pad1"), None);
    panel.take();
    panel.pointer_move(0, center("pad16"));
    assert!(matches!(panel.take().as_slice(), [Event::PadPressure(1, _)]), "the wandered drag reports pad 1, not the pad it moved onto");
    // A knob drag far past its own rect still turns the knob it started on.
    panel.pointer_down(1, center("knob2"), None);
    panel.take();
    panel.pointer_move(1, center("knob2") + egui::vec2(0.0, -100.0));
    let events = panel.take();
    assert!(!events.is_empty(), "the drag turns something");
    assert!(
        events.iter().all(|e| matches!(e, Event::Knob(1, _))),
        "every turn names knob 2 (index 1), not whatever is under the finger now: {events:?}"
    );
    assert!(events.iter().all(|e| matches!(e, Event::Knob(_, inc) if *inc > 0.0)), "up is clockwise: {events:?}");
    // A button released nowhere still releases the button it started on.
    panel.pointer_down(2, center("BUTTON_SHIFT"), None);
    assert_eq!(panel.take(), vec![Event::Button("BUTTON_SHIFT", true)], "down presses the button it landed on");
    panel.pointer_move(2, egui::pos2(mk3_layout::WIDTH - 8.0, mk3_layout::HEIGHT - 8.0));
    panel.pointer_up(2);
    assert_eq!(panel.take(), vec![Event::Button("BUTTON_SHIFT", false)], "up names the down button, not the gap the finger moved to");
}

#[test]
fn a_re_down_releases_the_old_gesture() {
    let mut panel = Panel::default();
    panel.pointer_down(0, center("pad1"), None);
    assert!(matches!(panel.take().as_slice(), [Event::PadOn(1, _)]), "pad 1 presses");
    // No lift: the finger lands on pad 2. Pad 1 releases first, then pad 2 presses.
    panel.pointer_down(0, center("pad2"), None);
    assert!(
        matches!(panel.take().as_slice(), [Event::PadOff(1), Event::PadOn(2, _)]),
        "the old pad releases before the new one presses: no orphaned PadOn"
    );
    panel.pointer_up(0);
    assert_eq!(panel.take(), vec![Event::PadOff(2)], "the lift releases pad 2; pad 1 is not orphaned");
    // Same for a button: a re-down elsewhere releases the held button first.
    panel.pointer_down(1, center("BUTTON_SHIFT"), None);
    assert_eq!(panel.take(), vec![Event::Button("BUTTON_SHIFT", true)], "shift presses");
    panel.pointer_down(1, center("pad3"), None);
    assert!(
        matches!(panel.take().as_slice(), [Event::Button("BUTTON_SHIFT", false), Event::PadOn(3, _)]),
        "shift releases before pad 3 presses: no stuck light"
    );
    panel.pointer_up(1);
    panel.pointer_up(1);
    assert_eq!(panel.take(), vec![Event::PadOff(3)], "the lift releases pad 3");
}

/// A frame the docked host is shown: fixed LEDs, group 3's route.
fn host_frame(leds: deckard_mk3::leds::Leds, screens: Option<(u64, crate::mk3_io::ScreenFrame)>) -> crate::Mk3Out {
    crate::Mk3Out {
        route: crate::mk3_io::PadRoute {
            play: true,
            group: 3,
            keys: None,
            chords: None,
            velocity: None,
            scaling: 0,
            fixed: 100,
            touches: [deckard_model::pad_touch::PadTouch::default(); 16],
        },
        leds,
        screens,
    }
}

#[test]
fn the_docked_host_returns_pointer_events_as_mk3_events() {
    let mut host = crate::onscreen_mk3::Host::default();
    host.panel.pointer_down(0, center("pad1"), None);
    host.show(&host_frame(deckard_mk3::leds::Leds::default(), None));
    let events = host.events();
    assert!(matches!(events.as_slice(), [Event::PadOn(1, _)]), "pointer down in pad 1's rect drains as one PadOn(1, _): {events:?}");
    assert_eq!(host.events(), Vec::new(), "a second drain is empty");
}

#[test]
fn the_docked_host_retains_the_last_frame() {
    let mut host = crate::onscreen_mk3::Host::default();
    assert!(host.last.is_none(), "no frame yet");
    let px = vec![[1, 2, 3, 255]; 4];
    host.show(&host_frame(
        deckard_mk3::leds::Leds::default(),
        Some((7, crate::mk3_io::ScreenFrame::Pixels(std::sync::Arc::new(px.clone())))),
    ));
    assert_eq!(
        host.last.as_ref().and_then(|o| o.screens.as_ref().map(|(n, f)| (*n, f.clone().into_pixels()))),
        Some((7, px.clone())),
        "show retains the frame's screens"
    );
    host.show(&host_frame(deckard_mk3::leds::Leds::default(), None));
    assert_eq!(
        host.last.as_ref().and_then(|o| o.screens.as_ref().map(|(n, f)| (*n, f.clone().into_pixels()))),
        Some((7, px)),
        "a frame without screens keeps the retained screens"
    );
}

#[test]
fn the_docked_host_wants_frames_only_while_a_led_flashes() {
    let mut host = crate::onscreen_mk3::Host::default();
    assert!(!host.wants_frames(), "no frame yet: no repaint asked");
    let mut steady = deckard_mk3::leds::Leds::default();
    assert!(steady.set("LED_PAD_1", 17 * 4 + 2), "white, bright, steady");
    host.show(&host_frame(steady, None));
    assert!(!host.wants_frames(), "a steady LED needs no repaint");
    let mut flashing = deckard_mk3::leds::Leds::default();
    assert!(flashing.set("LED_PAD_1", 17 * 4 + 3), "white, dim flash");
    host.show(&host_frame(flashing, None));
    assert!(host.wants_frames(), "a flashing LED asks for frames");
}

#[test]
fn portrait_swaps_the_surface_axes_in_hit_testing() {
    use crate::onscreen_mk3::{surface_size, to_landscape, to_portrait, to_surface};
    // The portrait surface is the whole table turned 90° clockwise: TAP's portrait
    // image sits on File in landscape, and pressing that same surface point in
    // portrait is TAP. The pair is derived from the table, never hardcoded.
    let tap = center("BUTTON_TAP");
    let q = to_portrait(tap, true);
    assert_eq!(mk3_layout::at(q).map(|c| c.id), Some("BUTTON_FILE"), "TAP's portrait image sits on File in landscape");
    // The aspect flips: a 1518 x 1382 surface becomes 1382 x 1518.
    assert_eq!(surface_size(false), egui::vec2(mk3_layout::WIDTH, mk3_layout::HEIGHT), "landscape is the table itself");
    assert_eq!(surface_size(true), egui::vec2(mk3_layout::HEIGHT, mk3_layout::WIDTH), "portrait flips the axes");
    assert_eq!(to_landscape(q, false), q, "landscape maps nothing");
    assert_eq!(to_portrait(q, false), q, "landscape maps nothing, either way");
    let back = to_landscape(q, true);
    assert_eq!(mk3_layout::at(back).map(|c| c.id), Some("BUTTON_TAP"), "portrait: the same point is Tap");
    assert_eq!(to_portrait(back, true), q, "the transform undoes itself");
    // Painter and hit test share the one transform: TAP's portrait placement
    // contains TAP's portrait image.
    let tapp = mk3_layout::control("BUTTON_TAP").expect("BUTTON_TAP is in the table");
    assert_eq!(to_surface(tapp.rect, false), tapp.rect, "landscape places nothing");
    assert!(to_surface(tapp.rect, true).contains(q), "portrait places Tap over its own image");
    // End-to-end through the panel: the same surface point presses File in
    // landscape and Tap in portrait.
    let mut panel = Panel::default();
    panel.pointer_down(0, q, None);
    assert_eq!(panel.take(), vec![Event::Button("BUTTON_FILE", true)], "landscape presses File");
    let mut panel = Panel::default();
    panel.pointer_down(0, back, None);
    assert_eq!(panel.take(), vec![Event::Button("BUTTON_TAP", true)], "portrait presses Tap");
}

/// Two touch fingers hold two pads: per-finger identity survives the painter's
/// forwarding, so the second finger holds its own control instead of re-downing
/// finger 0 (which would release the first pad). Screen coords are table coords
/// here: scale 1 from the origin, landscape.
#[test]
fn two_touch_fingers_hold_two_pads() {
    use crate::onscreen_mk3::Host;
    use eframe::egui::{TouchId, TouchPhase};
    let mut host = Host::default();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(mk3_layout::WIDTH, mk3_layout::HEIGHT));
    let pad = |id| mk3_layout::control(id).unwrap_or_else(|| panic!("{id} is in the table")).rect.center();
    host.forward_touches(rect, 1.0, false, &[(TouchId(1), TouchPhase::Start, pad("pad1"), None)]);
    assert_eq!(host.events(), vec![Event::PadOn(1, 0.8)], "the first finger holds pad 1");
    host.forward_touches(rect, 1.0, false, &[(TouchId(2), TouchPhase::Start, pad("pad2"), None)]);
    assert_eq!(host.events(), vec![Event::PadOn(2, 0.8)], "the second finger holds its own pad, not finger 0's re-down");
    host.forward_touches(rect, 1.0, false, &[(TouchId(1), TouchPhase::End, pad("pad1"), None)]);
    assert_eq!(host.events(), vec![Event::PadOff(1)], "lifting one finger releases only its pad");
    host.forward_touches(rect, 1.0, false, &[(TouchId(2), TouchPhase::End, pad("pad2"), None)]);
    assert_eq!(host.events(), vec![Event::PadOff(2)], "the held pad releases on its own lift");
}
