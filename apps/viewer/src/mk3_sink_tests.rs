//! The sink seam: one frame reaches every sink, and every sink's events come back.

use crate::Mk3Out;
use crate::mk3_io;
use crate::mk3_sink::{Mk3Sink, NullSink, Sinks};
use deckard_mk3::Event;
use deckard_mk3::leds::Leds;
use std::cell::RefCell;
use std::rc::Rc;

fn route() -> mk3_io::PadRoute {
    mk3_io::PadRoute {
        play: true,
        group: 2,
        keys: None,
        chords: None,
        velocity: None,
        scaling: 0,
        fixed: 100,
        touches: [Default::default(); 16],
    }
}

fn frame() -> Mk3Out {
    Mk3Out { route: route(), leds: Leds::default(), screens: None }
}

/// A frame that sends new screens: what the app hands out when the display changed.
fn frame_with_screens(n: u64) -> Mk3Out {
    let frame = mk3_io::ScreenFrame::Pixels(std::sync::Arc::new(vec![[1, 2, 3, 255]; 4]));
    Mk3Out { route: route(), leds: Leds::default(), screens: Some((n, frame)) }
}

/// What a recording sink saw and was given: shared, so a test can read it after the sink
/// has gone into `Sinks` as a box.
#[derive(Default)]
struct Seen {
    shown: Vec<(mk3_io::PadRoute, Leds, Option<u64>)>,
    queued: Vec<Event>,
}

struct Recording(Rc<RefCell<Seen>>);

impl Mk3Sink for Recording {
    fn show(&mut self, out: &Mk3Out) {
        self.0.borrow_mut().shown.push((out.route, out.leds.clone(), out.screens.as_ref().map(|(n, _)| *n)));
    }

    fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.0.borrow_mut().queued)
    }
}

/// A recording sink, and the handle a test reads it through.
fn recording() -> (Recording, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    (Recording(Rc::clone(&seen)), seen)
}

#[test]
fn every_sink_sees_the_same_frame() {
    let (first_sink, first) = recording();
    let (second_sink, second) = recording();
    let mut sinks = Sinks::new();
    sinks.add(Box::new(first_sink));
    sinks.add(Box::new(second_sink));
    sinks.add(Box::new(NullSink));
    assert_eq!(sinks.len(), 3, "three sinks, one of them silent");

    let out = frame();
    sinks.show(&out);

    assert_eq!(first.borrow().shown.len(), 1, "the first sink was shown the frame");
    assert_eq!(second.borrow().shown.len(), 1, "and so was the second");
    assert_eq!(first.borrow().shown[0].0.group, 2, "with the route the app built");
    assert_eq!(second.borrow().shown[0].0.group, 2, "the same route");
    assert!(first.borrow().shown[0].1 == second.borrow().shown[0].1, "and the same lights");

    sinks.show(&frame());
    assert_eq!(first.borrow().shown.len(), 2, "the next frame reaches it too");
    assert_eq!(second.borrow().shown.len(), 2);
}

#[test]
fn events_from_every_sink_are_merged() {
    let (first_sink, first) = recording();
    let (second_sink, second) = recording();
    first.borrow_mut().queued = vec![Event::Button("BUTTON_MIX", true)];
    second.borrow_mut().queued = vec![Event::Button("BUTTON_MIX", false), Event::Button("BUTTON_SHIFT", true)];

    let mut sinks = Sinks::new();
    sinks.add(Box::new(first_sink));
    sinks.add(Box::new(second_sink));

    assert_eq!(
        sinks.events(),
        vec![Event::Button("BUTTON_MIX", true), Event::Button("BUTTON_MIX", false), Event::Button("BUTTON_SHIFT", true)],
        "every sink's events, in sink order"
    );
    assert!(sinks.events().is_empty(), "and each event is taken once");
}

#[test]
fn no_sink_wants_frames_by_default() {
    let (sink, _) = recording();
    let mut sinks = Sinks::new();
    sinks.add(Box::new(sink));
    sinks.add(Box::new(NullSink));
    assert!(!sinks.wants_frames(), "a sink that shows what it is given asks for no frames");
    assert!(!Sinks::new().wants_frames(), "and no sinks at all asks for none");
}

#[test]
fn a_sink_joining_late_receives_the_current_screen() {
    let mut sinks = Sinks::new();
    sinks.show(&frame_with_screens(7));

    let (late_sink, late) = recording();
    sinks.add(Box::new(late_sink));
    assert!(late.borrow().shown.is_empty(), "nothing yet: it has not been replayed");
    sinks.replay_latest();

    let seen = late.borrow();
    assert_eq!(seen.shown.len(), 1, "the frame in hand, once");
    assert_eq!(seen.shown[0].2, Some(7), "with the screens the device is showing");
    assert!(seen.shown[0].1 == Leds::default(), "the lights that are on the device");
}

#[test]
fn a_frame_without_screens_still_replays_the_last_one() {
    let mut sinks = Sinks::new();
    sinks.show(&frame_with_screens(7));
    sinks.show(&frame()); // nothing changed on the display
    let (late_sink, late) = recording();
    sinks.add(Box::new(late_sink));
    sinks.replay_latest();

    let shown = late.borrow().shown.len();
    assert_eq!(shown, 1, "a sink that arrives between redraws still gets a frame");
}

#[test]
fn a_sink_that_was_there_for_the_frame_is_not_shown_it_twice() {
    let (early_sink, early) = recording();
    let mut sinks = Sinks::new();
    sinks.add(Box::new(early_sink));
    sinks.show(&frame_with_screens(7));
    assert_eq!(early.borrow().shown.len(), 1);

    sinks.replay_latest();
    assert_eq!(early.borrow().shown.len(), 1, "it already had that frame");
}
