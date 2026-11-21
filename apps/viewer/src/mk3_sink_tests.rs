//! The sink seam's contract, with no device and no window: every enabled sink sees the
//! same frame, and every sink's queued input comes back merged in sink order.

use crate::Mk3Out;
use crate::mk3_io::{Mk3Io, PadRoute, ScreenFrame};
use crate::mk3_sink::{DeviceSink, Mk3Sink, NullSink, Sinks};
use deckard_mk3::Event;
use deckard_mk3::leds::Leds;
use deckard_model::pad_touch::PadTouch;
use std::cell::RefCell;
use std::rc::Rc;

/// A frame the tests hand the sinks: fixed LEDs, group 3's route.
fn test_frame(screens: Option<(u64, ScreenFrame)>) -> Mk3Out {
    Mk3Out {
        route: PadRoute {
            play: true,
            group: 3,
            keys: None,
            chords: None,
            velocity: None,
            scaling: 0,
            fixed: 100,
            touches: [PadTouch::default(); 16],
        },
        leds: Leds { buttons: [0x11; 63], pads: [0x22; 42] },
        screens,
    }
}

/// What a recording sink was shown: the whole frame, as the sink received it.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    leds: Leds,
    screens: Option<(u64, Vec<[u8; 4]>)>,
}

/// A sink that keeps what it was shown and replays queued input: the seam's tests assert
/// on what each sink received, not just that `Sinks::show` returned.
#[derive(Debug, Default)]
struct Recording {
    shown: Rc<RefCell<Vec<Shown>>>,
    queued: Vec<Event>,
}

impl Recording {
    fn with_queued(events: Vec<Event>) -> (Self, Rc<RefCell<Vec<Shown>>>) {
        let shown: Rc<RefCell<Vec<Shown>>> = Rc::default();
        (Self { shown: shown.clone(), queued: events }, shown)
    }
}

impl Mk3Sink for Recording {
    fn show(&mut self, out: &Mk3Out) {
        self.shown
            .borrow_mut()
            .push(Shown { leds: out.leds.clone(), screens: out.screens.as_ref().map(|(n, f)| (*n, f.clone().into_pixels())) });
    }

    fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.queued)
    }
}

#[test]
fn every_sink_sees_the_same_frame() {
    let frame = test_frame(Some((7, ScreenFrame::Pixels(std::sync::Arc::new(vec![[1, 2, 3, 255]; 4])))));
    let (first, first_shown) = Recording::with_queued(Vec::new());
    let (second, second_shown) = Recording::with_queued(Vec::new());
    let mut sinks = Sinks::new();
    sinks.add(first);
    sinks.add(second);
    sinks.show(&frame);
    let first = first_shown.borrow();
    let second = second_shown.borrow();
    assert_eq!(first.len(), 1, "one frame shown once");
    assert_eq!(second.len(), 1, "one frame shown once");
    assert_eq!(first[0].leds, frame.leds, "the first sink got the frame's LEDs");
    assert_eq!(
        first[0].screens,
        frame.screens.as_ref().map(|(n, f)| (*n, f.clone().into_pixels())),
        "the first sink got the frame's screens"
    );
    assert_eq!(first[0], second[0], "both sinks captured the same frame");
}

#[test]
fn events_from_every_sink_are_merged() {
    let (first, _) = Recording::with_queued(vec![Event::Button("PLAY", true)]);
    let (second, _) = Recording::with_queued(vec![Event::Button("PLAY", true), Event::Knob(0, 0.5)]);
    let mut sinks = Sinks::new();
    sinks.add(first);
    sinks.add(second);
    assert_eq!(
        sinks.events(),
        vec![Event::Button("PLAY", true), Event::Button("PLAY", true), Event::Knob(0, 0.5)],
        "in sink order, and the same button twice is two events"
    );
}

/// The Task 1 lesson at this seam: the hardware adapter must forward the whole frame —
/// route, LEDs and screens — exactly as the direct calls did, so this asserts on what
/// the driver received, not just that `show` returned.
#[test]
fn device_sink_forwards_the_whole_frame() {
    let mk3 = Mk3Io::detached();
    let mut sink = DeviceSink::new(mk3.clone());
    let frame = test_frame(Some((7, ScreenFrame::Pixels(std::sync::Arc::new(vec![[1, 2, 3, 255]; 2])))));
    sink.show(&frame);
    let route = mk3.stored_route().expect("the route reached the driver");
    assert_eq!(route.play, frame.route.play, "route.play");
    assert_eq!(route.group, frame.route.group, "route.group");
    assert_eq!(route.keys, frame.route.keys, "route.keys");
    assert_eq!(route.chords, frame.route.chords, "route.chords");
    assert_eq!(route.velocity, frame.route.velocity, "route.velocity");
    assert_eq!(route.scaling, frame.route.scaling, "route.scaling");
    assert_eq!(route.fixed, frame.route.fixed, "route.fixed");
    assert_eq!(route.touches, frame.route.touches, "route.touches");
    assert_eq!(mk3.stored_leds(), Some(frame.leds.clone()), "the LEDs reached the driver");
    assert_eq!(mk3.stored_screens(), Some((7, vec![[1, 2, 3, 255]; 2])), "the screens reached the driver");
    // A frame without screens leaves the last picture alone: the driver's `set_screens`
    // only stores a new count.
    sink.show(&test_frame(None));
    assert_eq!(mk3.stored_route().map(|r| r.group), Some(3), "the route is still forwarded");
    assert_eq!(mk3.stored_screens(), Some((7, vec![[1, 2, 3, 255]; 2])), "no screens sent: the last picture stays");
}

#[test]
fn null_sink_does_nothing() {
    let mut null = NullSink;
    null.show(&test_frame(None));
    assert_eq!(null.events(), Vec::new(), "no input queued");
    assert!(!null.wants_frames(), "never asks for frames");
}

#[test]
fn a_sink_joining_late_receives_the_current_screen() {
    let px = vec![[1, 2, 3, 255]; 4];
    let (first, _) = Recording::with_queued(Vec::new());
    let mut sinks = Sinks::new();
    sinks.add(first);
    sinks.show(&test_frame(Some((7, ScreenFrame::Pixels(std::sync::Arc::new(px.clone()))))));
    let (late, late_shown) = Recording::with_queued(Vec::new());
    sinks.add(late);
    let late = late_shown.borrow();
    assert_eq!(late.len(), 1, "the late sink got the retained frame once, with no re-render");
    assert_eq!(late[0].screens, Some((7, px)), "the late sink received the current screen (count, px)");
}

#[test]
fn a_frame_without_screens_still_replays_the_last_one() {
    let px = vec![[9, 9, 9, 255]; 2];
    let (first, _) = Recording::with_queued(Vec::new());
    let mut sinks = Sinks::new();
    sinks.add(first);
    sinks.show(&test_frame(Some((7, ScreenFrame::Pixels(std::sync::Arc::new(px.clone()))))));
    sinks.show(&test_frame(None));
    let (late, late_shown) = Recording::with_queued(Vec::new());
    sinks.add(late);
    let late = late_shown.borrow();
    assert_eq!(late.len(), 1, "the late sink got one replayed frame");
    assert_eq!(late[0].screens, Some((7, px)), "screens: None does not clear the retained frame");
}

#[test]
fn panel_opens_with_the_retained_frame() {
    let px = vec![[1, 2, 3, 255]; 4];
    let mut sinks = Sinks::new();
    assert!(!sinks.panel_open(), "closed until opened");
    sinks.show(&test_frame(Some((7, ScreenFrame::Pixels(std::sync::Arc::new(px.clone()))))));
    sinks.set_panel_open(true);
    assert!(sinks.panel_open(), "open after the toggle");
    let panel = sinks.panel_mut().expect("the open panel paints through here");
    assert_eq!(
        panel.last.as_ref().and_then(|o| o.screens.as_ref().map(|(n, f)| (*n, f.clone().into_pixels()))),
        Some((7, px)),
        "opening replays the retained frame like any late join"
    );
}

#[test]
fn panel_input_merges_and_close_gates_repaint() {
    let mut sinks = Sinks::new();
    sinks.set_panel_open(true);
    let pad = crate::mk3_layout::control("pad1").expect("pad1 is in the table").rect.center();
    sinks.panel_mut().expect("open").panel.pointer_down(0, pad, None);
    assert!(matches!(sinks.events().as_slice(), [Event::PadOn(1, _)]), "panel contact drains through the same seam");
    assert_eq!(sinks.events(), Vec::new(), "a second drain is empty");
    let mut leds = Leds::default();
    assert!(leds.set("LED_PAD_1", 17 * 4 + 3), "white, dim flash");
    let mut frame = test_frame(None);
    frame.leds = leds;
    sinks.show(&frame);
    assert!(sinks.wants_frames(), "a flashing LED asks for frames while the panel is open");
    sinks.set_panel_open(false);
    assert!(!sinks.panel_open(), "closed after the toggle");
    assert!(!sinks.wants_frames(), "a closed panel never asks for frames");
}

#[test]
fn a_poisoned_panel_lock_recovers_instead_of_freezing() {
    let mut sinks = Sinks::new();
    sinks.set_panel_open(true);
    // Poison the panel lock deliberately: a thread panics while holding the guard.
    let shared = sinks.panel_shared().expect("the open panel is shared");
    let holder = std::thread::spawn(move || {
        let _held = shared.lock().unwrap();
        panic!("deliberate: poison the panel lock");
    });
    assert!(holder.join().is_err(), "the holder panicked, so the lock is poisoned");
    // Recovery, not a silent freeze: open still reports, frames still retain, input still drains.
    assert!(sinks.panel_open(), "a poisoned-but-open panel still reports open");
    let px = vec![[1, 2, 3, 255]; 4];
    sinks.show(&test_frame(Some((7, ScreenFrame::Pixels(std::sync::Arc::new(px.clone()))))));
    assert_eq!(
        sinks
            .panel_mut()
            .expect("the guard recovers")
            .last
            .as_ref()
            .and_then(|o| o.screens.as_ref().map(|(n, f)| (*n, f.clone().into_pixels()))),
        Some((7, px)),
        "a poisoned lock still retains the frame"
    );
    let pad = crate::mk3_layout::control("pad1").expect("pad1 is in the table").rect.center();
    sinks.panel_mut().expect("the guard recovers").panel.pointer_down(0, pad, None);
    assert!(matches!(sinks.events().as_slice(), [Event::PadOn(1, _)]), "a poisoned lock still drains input");
    assert!(sinks.panel_open(), "recovery holds: still open afterwards");
}

/// A stub that claims the hardware's identity: its input already played.
struct DeviceStub {
    queued: Vec<Event>,
}

impl Mk3Sink for DeviceStub {
    fn show(&mut self, _out: &Mk3Out) {}

    fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.queued)
    }

    fn is_device(&self) -> bool {
        true
    }
}

#[test]
fn input_splits_by_origin_so_hardware_never_replays() {
    let mut sinks = Sinks::new();
    sinks.add(DeviceStub { queued: vec![Event::PadOn(1, 0.8)] });
    sinks.set_panel_open(true);
    let pad = crate::mk3_layout::control("pad2").expect("pad2 is in the table").rect.center();
    sinks.panel_mut().expect("open").panel.pointer_down(0, pad, None);
    let input = sinks.take_input();
    assert_eq!(input.device, vec![Event::PadOn(1, 0.8)], "the hardware half: already played");
    assert!(matches!(input.virtual_.as_slice(), [Event::PadOn(2, _)]), "the virtual half: still to play, got {:?}", input.virtual_);
    assert_eq!(sinks.take_input().device, Vec::new(), "a second drain is empty");
    assert_eq!(sinks.take_input().virtual_, Vec::new(), "a second drain is empty");
}
