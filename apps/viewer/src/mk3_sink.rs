//! Where an MK3 frame goes: the device, and anything else that shows what it shows.
//!
//! The app builds one frame's worth of what the controller should show (`Mk3Out`) and hands
//! it to every sink; each sink's events come back merged. The device is one sink among
//! them, which is what lets an on-screen panel be another.
//!
//! `show` takes `&Mk3Out` rather than taking it: a sink that wants to keep the frame (a
//! panel redrawing between frames) clones what it keeps, and the others pay nothing.

use crate::Mk3Out;
use crate::mk3_io;
use deckard_mk3::Event;
use std::sync::Arc;

/// Something that shows an MK3's frame and answers what its own surface did.
pub trait Mk3Sink {
    /// Show one frame: the pads' route, the LEDs, and the screens if they changed.
    fn show(&mut self, out: &Mk3Out);
    /// What this surface did since the last call, oldest first.
    fn events(&mut self) -> Vec<Event>;
    /// This sink draws every frame rather than only when something changed.
    fn wants_frames(&self) -> bool {
        false
    }
}

/// A sink that shows nothing and answers nothing: headless runs, and tests of the others.
pub struct NullSink;

impl Mk3Sink for NullSink {
    fn show(&mut self, _out: &Mk3Out) {}
    fn events(&mut self) -> Vec<Event> {
        Vec::new()
    }
}

/// The controller itself, as one sink: the driver's thread takes the frame and queues its
/// own events.
pub struct DeviceSink {
    io: Arc<mk3_io::Mk3Io>,
}

impl DeviceSink {
    pub fn new(io: Arc<mk3_io::Mk3Io>) -> DeviceSink {
        DeviceSink { io }
    }
}

impl Mk3Sink for DeviceSink {
    fn show(&mut self, out: &Mk3Out) {
        self.io.set_route(out.route);
        self.io.set_leds(out.leds.clone());
        if let Some((n, frame)) = out.screens.clone() {
            self.io.set_screens(n, frame);
        }
    }

    fn events(&mut self) -> Vec<Event> {
        self.io.take_events()
    }
}

/// Every sink, in the order they were added, and the frame each of them gets.
#[derive(Default)]
pub struct Sinks {
    sinks: Vec<Box<dyn Mk3Sink>>,
}

impl Sinks {
    pub fn new() -> Sinks {
        Sinks::default()
    }

    pub fn add(&mut self, sink: Box<dyn Mk3Sink>) {
        self.sinks.push(sink);
    }

    /// How many sinks there are (the device is one, when it started).
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }

    /// Show one frame on every sink: the same route, the same LEDs, the same screens.
    pub fn show(&mut self, out: &Mk3Out) {
        for sink in &mut self.sinks {
            sink.show(out);
        }
    }

    /// Every sink's events, in sink order: the controller's first, then a panel's.
    pub fn events(&mut self) -> Vec<Event> {
        let mut all = Vec::new();
        for sink in &mut self.sinks {
            all.extend(sink.events());
        }
        all
    }

    /// Any sink that redraws every frame asks the app for one.
    pub fn wants_frames(&self) -> bool {
        self.sinks.iter().any(|s| s.wants_frames())
    }
}
