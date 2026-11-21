//! One seam for MK3 output: a frame goes to every enabled sink, input comes back merged.
//!
//! The hardware driver is one sink among several; an on-screen panel and a browser client
//! join later without touching the frame's producers. `Sinks::show` hands the same frame
//! to every enabled sink in the order they were enabled, and `Sinks::events` merges every
//! sink's queued input in that order without de-duplicating (two devices pressing the same
//! button is two events).

use crate::Mk3Out;
use crate::mk3_io::Mk3Io;
use crate::onscreen_mk3::Host;
use eframe::egui;
use std::sync::{Arc, Mutex, MutexGuard};

/// A destination for a frame of MK3 output, and a source of MK3 input.
///
/// A sink that joins late (the panel, a browser client connecting between redraws)
/// gets the last frame through `joined`, so it never sits on blank displays waiting
/// for the script to draw again.
pub trait Mk3Sink {
    /// A frame of script output: what the hardware would show right now. A sink that needs
    /// the frame later (a panel opened or a client joined after the draw) keeps its own copy.
    fn show(&mut self, out: &Mk3Out);
    /// The last frame shown, handed to a sink that joined after it was fanned out.
    /// The default is `show`: sinks that keep nothing need no special case.
    fn joined(&mut self, out: &Mk3Out) {
        self.show(out);
    }
    /// Input to feed into `App::mk3_events` next frame.
    fn events(&mut self) -> Vec<deckard_mk3::Event>;
    /// Whether this sink is the hardware driver: its input already played on the
    /// device thread, so the app thread must never play it again. Only the
    /// [`DeviceSink`] says yes; the panel and the browser clients play through
    /// the app-owned [`crate::pad_play::Pads`] instead.
    fn is_device(&self) -> bool {
        false
    }
    /// Whether the app must redraw without new input (animated pages, lights flashing).
    fn wants_frames(&self) -> bool {
        false
    }
}

/// A sink that keeps nothing and queues nothing: tests and headless runs.
///
/// Test-only until the panel and the server sink here (hence the allow).
#[allow(dead_code)]
pub struct NullSink;

#[allow(dead_code)]
impl Mk3Sink for NullSink {
    fn show(&mut self, _out: &Mk3Out) {}

    fn events(&mut self) -> Vec<deckard_mk3::Event> {
        Vec::new()
    }
}

/// The hardware driver as a sink: today's `set_route`/`set_leds`/`set_screens` behind
/// `show`, the driver's queue behind `events`. The device threads keep their semantics;
/// `show` only stores into the shared state the threads already poll.
pub struct DeviceSink {
    mk3: Mk3Io,
}

impl DeviceSink {
    pub fn new(mk3: Mk3Io) -> Self {
        Self { mk3 }
    }
}

impl Mk3Sink for DeviceSink {
    fn show(&mut self, out: &Mk3Out) {
        self.mk3.set_route(out.route);
        self.mk3.set_leds(out.leds.clone());
        if let Some((n, px)) = &out.screens {
            self.mk3.set_screens(*n, px.clone());
        }
    }

    fn events(&mut self) -> Vec<deckard_mk3::Event> {
        self.mk3.take_events()
    }

    fn is_device(&self) -> bool {
        true
    }
}

/// One frame's input, split by origin: `device` already played on the hardware's
/// own thread (the app hands it straight to the scripts), `virtual_` queued from
/// the panel and the browser clients (the app plays it first, then hands it on).
/// `virtual_` is not a Rust keyword, so it carries the trailing underscore.
#[derive(Default)]
pub struct Input {
    /// What the hardware driver queued: already sounding, never replayed.
    pub device: Vec<deckard_mk3::Event>,
    /// What the panel and the browser clients queued: silent until played.
    pub virtual_: Vec<deckard_mk3::Event>,
}

/// Every enabled sink, in the order they were enabled: one `show` fans the same frame out
/// to all of them, one `take_input` drains what they queued, split by origin.
#[derive(Default)]
pub struct Sinks {
    sinks: Vec<Box<dyn Mk3Sink>>,
    /// The hardware's handle on the same state the [`DeviceSink`] in `sinks` drives (`Mk3Io`
    /// is a lock around shared state, so this never drifts from the sink). `None` until
    /// [`Sinks::enable_device`]; the pass-throughs below no-op without it.
    device: Option<Mk3Io>,
    /// The last frame `show` fanned out, with the last screens the source drew: most
    /// frames carry `screens: None` (nothing changed), so the retained screens stay
    /// until the source draws again. `None` until the first `show`.
    last: Option<Mk3Out>,
    /// How many of `sinks` have seen `last`: the ones at and after this index joined
    /// later and are still owed it through [`Sinks::replay_latest`].
    shown: usize,
    /// The on-screen panel (Tasks 7-8): one more sink in the same seam, kept beside
    /// the boxed sinks so the app can paint it without a downcast, and shared so the
    /// panel's own window (a deferred viewport with a `'static` closure) draws the
    /// same host. `None` until [`Sinks::set_panel_open`] opens it.
    panel: Option<Arc<Mutex<Host>>>,
}

impl Sinks {
    pub fn new() -> Self {
        Self::default()
    }

    /// One more sink, at the back: frames fan out and input merges in the order sinks
    /// joined. The hardware joins through here too (see `enable_device`).
    pub fn add(&mut self, sink: impl Mk3Sink + 'static) {
        self.sinks.push(Box::new(sink));
        self.replay_latest();
    }

    /// Open the on-screen panel, or close it: opening replays the retained frame
    /// like any late join, so the panel shows the current picture at once; closing
    /// keeps the host (and its retained frame) but draws nothing. A closed host
    /// still sinks frames, so reopening never shows a stale picture.
    pub fn set_panel_open(&mut self, open: bool) {
        if open {
            if self.panel.is_none() {
                let mut host = Host::default();
                host.open = true;
                if let Some(last) = self.last.clone() {
                    host.joined(&last);
                }
                self.panel = Some(Arc::new(Mutex::new(host)));
            } else if let Some(shared) = self.panel.clone() {
                // A poisoned lock recovers onto the data it holds: an open/close
                // toggle must never silently diverge the menu from reality.
                shared.lock().unwrap_or_else(|e| e.into_inner()).open = true;
            }
        } else if let Some(shared) = self.panel.clone() {
            shared.lock().unwrap_or_else(|e| e.into_inner()).open = false;
        }
    }

    /// Whether the panel is shown (docked or in its own window).
    pub fn panel_open(&self) -> bool {
        self.panel.as_ref().is_some_and(|p| p.lock().unwrap_or_else(|e| e.into_inner()).open)
    }

    /// The panel, to paint it: `None` until [`Sinks::set_panel_open`] opens it. The
    /// guard borrows the shared host, so the docked surface and the panel's own
    /// window never see two hosts. A poisoned lock recovers onto the data it holds
    /// instead of freezing the panel with no signal.
    pub fn panel_mut(&mut self) -> Option<MutexGuard<'_, Host>> {
        self.panel.as_ref().map(|p| p.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The shared panel host, for the panel's own window: the deferred viewport's
    /// `'static` closure draws through this, never a second host.
    pub fn panel_shared(&self) -> Option<Arc<Mutex<Host>>> {
        self.panel.clone()
    }

    /// Start the hardware driver, exactly as start-up always has (`Mk3Io::start` opens the
    /// device lazily, so a missing MK3 is not an error), and sink it first.
    pub fn enable_device(&mut self, mk3: Mk3Io) {
        if self.device.is_some() {
            return;
        }
        // One device, two handles: `Mk3Io` is a lock around shared state, so the app's handle
        // and the sink's never drift, and the device is never opened twice.
        self.device = Some(mk3.clone());
        self.add(DeviceSink::new(mk3));
    }

    /// The same frame to every enabled sink, in the order they were enabled.
    pub fn show(&mut self, out: &Mk3Out) {
        // The retained frame keeps the last screens the source drew: most frames carry
        // `screens: None` (nothing changed), and a sink joining between redraws must
        // still see the current screen. Enabled sinks get the frame verbatim, as before.
        let mut frame = out.clone();
        if frame.screens.is_none() {
            frame.screens = self.last.as_ref().and_then(|prev| prev.screens.clone());
        }
        self.last = Some(frame);
        for sink in &mut self.sinks {
            sink.show(out);
        }
        self.shown = self.sinks.len();
        if let Some(shared) = self.panel.clone() {
            shared.lock().unwrap_or_else(|e| e.into_inner()).show(out);
        }
    }

    /// The retained frame to every sink that joined since it was shown, in join order.
    /// A frame without screens does not clear the retained one: the screens the source
    /// last drew are still what a late sink should see. Nothing happens when no frame
    /// has been shown yet; the next `show` covers the new sink.
    pub fn replay_latest(&mut self) {
        let Some(last) = self.last.clone() else { return };
        for sink in &mut self.sinks[self.shown..] {
            sink.joined(&last);
        }
        self.shown = self.sinks.len();
    }

    /// Every sink's queued input, split by origin: what the hardware already played
    /// on the device thread, and what the panel and browser clients queued for the
    /// app thread to play. Chose one method returning both halves over two drain
    /// methods, so a caller cannot drain one half and forget the other.
    pub fn take_input(&mut self) -> Input {
        let mut input = Input::default();
        for sink in &mut self.sinks {
            if sink.is_device() {
                input.device.extend(sink.events());
            } else {
                input.virtual_.extend(sink.events());
            }
        }
        if let Some(shared) = self.panel.clone() {
            input.virtual_.extend(shared.lock().unwrap_or_else(|e| e.into_inner()).events());
        }
        input
    }

    /// Every sink's queued input, in sink order: the device half first, then the
    /// virtual half. Not de-duplicated: two devices pressing the same button is
    /// two events. The app drains halves separately (`take_input`); this merged
    /// view is what the tests assert on.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn events(&mut self) -> Vec<deckard_mk3::Event> {
        let input = self.take_input();
        input.device.into_iter().chain(input.virtual_).collect()
    }

    /// The last frame `show` fanned out, if one has: the pads play virtual input
    /// from this route (see `App::play_virtual_pads`).
    pub fn last_route(&self) -> Option<crate::mk3_io::PadRoute> {
        self.last.as_ref().map(|out| out.route)
    }
    /// Whether any sink wants frames without new input.
    pub fn wants_frames(&self) -> bool {
        self.sinks.iter().any(|sink| sink.wants_frames())
            || self.panel.as_ref().is_some_and(|p| {
                let host = p.lock().unwrap_or_else(|e| e.into_inner());
                host.open && host.wants_frames()
            })
    }

    /// Pad hits that came while no engine ran (the device's queue, not a sink's).
    pub fn take_pending(&self) -> Vec<(usize, usize, i64, i64)> {
        self.device.as_ref().map_or(Vec::new(), Mk3Io::take_pending)
    }

    /// What wakes the device threads when the app draws (the repaint handle).
    pub fn set_context(&self, ctx: &egui::Context) {
        if let Some(m) = &self.device {
            m.set_context(ctx);
        }
    }

    /// Show what the pads play in the MIDI Monitor.
    pub fn set_monitor(&self, monitor: crate::midi_monitor::Monitor) {
        if let Some(m) = &self.device {
            m.set_monitor(monitor);
        }
    }

    /// The output the pads play into.
    pub fn set_output(&self, output: deckard_engine::output::Sender) {
        if let Some(m) = &self.device {
            m.set_output(output);
        }
    }

    /// The LEDs' global level (0..=100 %), now and whenever an MK3 connects.
    pub fn set_led_brightness(&self, level: u8) {
        if let Some(m) = &self.device {
            m.set_led_brightness(level);
        }
    }

    /// The displays' backlight (0..=100), now and whenever an MK3 connects.
    pub fn set_display_brightness(&self, level: u8) {
        if let Some(m) = &self.device {
            m.set_display_brightness(level);
        }
    }

    /// The pads' sensitivity (0..=100 %), now if it changed and whenever an MK3 connects.
    pub fn set_pad_sensitivity(&self, level: u8) {
        if let Some(m) = &self.device {
            m.set_pad_sensitivity(level);
        }
    }
}
