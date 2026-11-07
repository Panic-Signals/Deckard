//! The Maschine JAM over USB (`deckard-jam`): a thread reads the controller. Pads play straight
//! from that thread, as the MK3's do, so a note never waits for a frame; every event also goes
//! to the app, which hands it to the script (`crates/surface/lua/devices/maschine_jam`).
//!
//! Deckard opens the JAM whether or not Maschine is running (docs/superpowers/specs/
//! 2026-11-05-maschine-jam-design.md, "Sharing the JAM"): the HID interface opens shared, so
//! with both running each acts on every press and they overwrite each other's lights.
//! Preferences ▸ MIDI switches it off.

use crate::App;
use deckard_engine::Live;
use deckard_engine::output::Sender;
use deckard_jam::{Event, Jam, Leds, Touch};
use deckard_surface::device::{Device, Out, PadMap, PadNotes};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The script's id: `devices/<id>/`.
pub const DEVICE: &str = "maschine_jam";

/// How often the lights want redrawing, for what pulses and chases, when nothing else asks.
const FRAME: Duration = Duration::from_millis(40);

#[derive(Default)]
struct Shared {
    /// Preferences' switch (until told, off: the app sets it as it starts).
    enabled: bool,
    connected: bool,
    status: String,
    events: Vec<Event>,
    /// Pad hits that came while no engine played: (group, sound, pitch, velocity).
    pending: Vec<(usize, usize, i64, i64)>,
    output: Option<Sender>,
    route: Option<PadMap>,
    leds: Option<Leds>,
    repaint: Option<eframe::egui::Context>,
    /// DECKARD is closing: lights off, then stop.
    closing: bool,
    closed: bool,
}

pub struct JamIo {
    shared: Arc<Mutex<Shared>>,
}

impl JamIo {
    /// Start looking for a JAM (and keep looking: it may be plugged in later).
    pub fn start() -> JamIo {
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let s = shared.clone();
        std::thread::Builder::new().name("DECKARD JAM".into()).spawn(move || run(&s)).ok();
        JamIo { shared }
    }

    pub fn set_enabled(&self, on: bool) {
        self.shared.lock().unwrap().enabled = on;
    }

    pub fn set_context(&self, ctx: &eframe::egui::Context) {
        let mut s = self.shared.lock().unwrap();
        if s.repaint.is_none() {
            s.repaint = Some(ctx.clone());
        }
    }

    /// The output pads play into.
    pub fn set_output(&self, output: Sender) {
        self.shared.lock().unwrap().output = Some(output);
    }

    pub fn connected(&self) -> bool {
        self.shared.lock().unwrap().connected
    }

    /// What the controller did since the last call.
    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut self.shared.lock().unwrap().events)
    }

    /// Pad hits for the app to play itself (no engine was running).
    pub fn take_pending(&self) -> Vec<(usize, usize, i64, i64)> {
        std::mem::take(&mut self.shared.lock().unwrap().pending)
    }

    pub fn set_route(&self, route: PadMap) {
        self.shared.lock().unwrap().route = Some(route);
    }

    /// Light the LEDs so (only what changed is sent).
    pub fn set_leds(&self, leds: Leds) {
        self.shared.lock().unwrap().leds = Some(leds);
    }

    /// Leave the lights off. Waits (up to a second) for the thread to do it.
    pub fn shut_down(&self) {
        self.shared.lock().unwrap().closing = true;
        let until = Instant::now() + Duration::from_secs(1);
        while Instant::now() < until {
            {
                let s = self.shared.lock().unwrap();
                if s.closed || !s.connected {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// What a pad's hit does: the notes it started, or for no engine, the notes to play later.
fn hit(route: Option<&PadMap>, pad: usize) -> (PadNotes, i64) {
    match route {
        Some(r) => (r.pads.get(pad).cloned().unwrap_or_default(), r.velocity),
        None => (Vec::new(), 100),
    }
}

fn run(shared: &Mutex<Shared>) {
    loop {
        {
            let s = shared.lock().unwrap();
            if s.closing {
                return;
            }
            if !s.enabled {
                drop(s);
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        }
        let mut jam = match Jam::open() {
            Ok(j) => j,
            Err(e) => {
                shared.lock().unwrap().status = e;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        {
            let mut s = shared.lock().unwrap();
            s.connected = true;
            s.status = "Maschine JAM connected".into();
        }
        let mut events = Vec::new();
        // What each pad started, so it stops the same notes however the route changed.
        let mut sounding: Vec<PadNotes> = vec![Vec::new(); 64];
        loop {
            let (closing, enabled, leds) = {
                let s = shared.lock().unwrap();
                (s.closing, s.enabled, s.leds.clone())
            };
            if closing || !enabled {
                let _ = jam.dark();
                let mut s = shared.lock().unwrap();
                s.closed = closing;
                s.connected = false;
                release(&s, &mut sounding);
                if closing {
                    return;
                }
                break;
            }
            let wrote = leds.as_ref().map_or(Ok(()), |l| jam.show(l));
            let read = wrote.and_then(|()| jam.poll(2, &mut events));
            if let Err(e) = read {
                let mut s = shared.lock().unwrap();
                s.status = format!("Maschine JAM: {e}");
                s.connected = false;
                release(&s, &mut sounding);
                break;
            }
            if events.is_empty() {
                continue;
            }
            let mut s = shared.lock().unwrap();
            let out = s.output.clone().filter(Sender::has_engine);
            let route = s.route.clone();
            for e in &events {
                let Event::Pad { column, row, down } = *e else { continue };
                let pad = usize::from(column) + usize::from(row) * 8;
                if down {
                    let (notes, velocity) = hit(route.as_ref(), pad);
                    match &out {
                        Some(o) => {
                            for &(group, sound, pitch) in &notes {
                                // Held until the pad is let go (Note Repeat and the Arp take held notes).
                                o.live(Live::NoteOn { group, sound, pitch, velocity, length: i64::MAX });
                            }
                            sounding[pad] = notes;
                        }
                        None => s.pending.extend(notes.iter().map(|&(g, n, p)| (g, n, p, velocity))),
                    }
                } else if let Some(o) = &out {
                    for (group, sound, pitch) in std::mem::take(&mut sounding[pad]) {
                        o.live(Live::NoteRelease { group, sound, pitch });
                    }
                }
            }
            s.events.append(&mut events);
            if let Some(ctx) = &s.repaint {
                ctx.request_repaint();
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// A frame's events with only the last of a finger's run of moves: scripts work from one view
/// a frame, so a strip turning a knob by the difference from its value would count each move
/// from the same place and overshoot.
pub(crate) fn coalesced(events: Vec<Event>) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    for e in events {
        if let Event::Strip { strip, finger, kind: Touch::Move, .. } = e
            && let Some(last) =
                out.iter_mut().rev().find(|x| matches!(x, Event::Strip { strip: s, finger: f, .. } if *s == strip && *f == finger))
            && matches!(last, Event::Strip { kind: Touch::Move, .. })
        {
            *last = e;
            continue;
        }
        out.push(e);
    }
    out
}

/// Whatever pads were holding ends: the JAM went, or was switched off.
fn release(s: &Shared, sounding: &mut [PadNotes]) {
    let Some(o) = s.output.as_ref().filter(|o| o.has_engine()) else { return };
    for notes in sounding {
        for (group, sound, pitch) in std::mem::take(notes) {
            o.live(Live::NoteRelease { group, sound, pitch });
        }
    }
}

impl App {
    /// The JAM's events since the last frame, to its script; its lights and where its pads
    /// play kept current. As `mk3_sync` is, called with the window shown or not (`covered`), so
    /// the lights keep time with it minimized.
    pub(crate) fn jam_sync(&mut self, ctx: &eframe::egui::Context, covered: bool) {
        let Some(j) = &self.jam else { return };
        j.set_context(ctx);
        j.set_enabled(self.settings.jam_enabled);
        let connected = j.connected();
        let (events, pending) = (j.take_events(), j.take_pending());
        let now = Instant::now();
        if !connected {
            if let Some(mut d) = self.jam_device.take() {
                let asked = d.close();
                self.jam_asked(asked);
                self.jam_settings_wanted = false;
                self.input_watched = false;
                self.status = "Maschine JAM disconnected".into();
            }
            return;
        }
        if self.jam_device.is_none() {
            let user = crate::menu::settings_dir().map(|d| d.join("controller"));
            let mut d = Device::new(DEVICE, user);
            let view = self.surface_view();
            let asked = d.open(&view, now);
            self.jam_device = Some(d);
            self.jam_asked(asked);
            self.status = "Maschine JAM connected".into();
            eprintln!("Maschine JAM connected");
        }
        let (leds, route) = self.jam_events(events, now);
        if let Some(j) = &self.jam {
            j.set_leds(leds.scaled(self.settings.led_brightness));
            j.set_route(route);
        }
        // Pad hits that came while no engine ran (it starts with the first).
        for (group, sound, pitch, velocity) in pending {
            self.play_note(Live::NoteOn { group, sound, pitch, velocity, length: deckard_model::PPQ });
        }
        // (The window may be covered, with no frame to do it.)
        self.input_sync();
        ctx.request_repaint_after(if covered { crate::undrawn(ctx, FRAME) } else { FRAME });
    }

    /// The JAM's events, to its script: the lights and the pads' route after them.
    /// Hardware-free, so tests drive it with made-up events.
    pub(crate) fn jam_events(&mut self, events: Vec<Event>, now: Instant) -> (Leds, PadMap) {
        self.update_slot_view();
        if self.jam_settings_wanted {
            self.refresh_surface_settings();
        }
        let view = self.surface_view();
        let Some(mut d) = self.jam_device.take() else { return (Leds::default(), PadMap::default()) };
        let mut asked = d.poll(now);
        for e in coalesced(events) {
            asked.extend(match e {
                Event::Button(name, down) => d.button(name, down, &view, now),
                Event::Pad { column, row, down } => d.pad(column, row, down, &view, now),
                Event::Strip { strip, finger, kind, pos } => d.strip(strip, finger, kind.name(), pos, &view, now),
                Event::Encoder(steps) => d.encoder(steps, &view, now),
            });
        }
        asked.extend(d.tick(&view, now));
        self.jam_asked(asked);
        self.update_slot_view();
        let view = self.surface_view();
        let (leds, route) = (d.leds(&view, now), d.pad_map(&view, now));
        let error = d.error();
        if error != self.jam_error {
            if let Some(e) = &error {
                self.status = format!("controller script: {e}");
            }
            self.jam_error = error;
        }
        self.jam_device = Some(d);
        (leds, route)
    }

    /// What the script asked of the app.
    fn jam_asked(&mut self, asked: Vec<Out>) {
        for a in asked {
            match a {
                Out::App(a) => self.surface_act(a),
                Out::Print(p) => self.print_notes(&p),
                Out::Message(m) => self.status = m,
                Out::Midi { .. } => {}
            }
        }
    }

    /// Group `g`'s Perform FX's mode: one is added if the group has none.
    pub(crate) fn jam_perform_mode(&mut self, g: usize, mode: usize) {
        use deckard_doc::{Chain, Target};
        let Some(doc) = self.doc.clone() else { return };
        if self.perform_slot(g).is_none()
            && let Err(e) = doc.borrow_mut().add_perform_fx(g)
        {
            self.status = format!("can't add a Perform FX: {e}");
            return;
        }
        if let Some(i) = self.perform_slot(g) {
            crate::layout::apply_acts(
                self,
                vec![crate::layout::Act::SetParam(Target::ChainModule(Chain::Group(g), i), "mode", mode.min(7) as f64)],
            );
        }
    }

    /// A sound's Sampler's tune, or every Sampler's in a group moved together so that the
    /// first is at `semitones`. Sounds with no Sampler have no tune to set.
    pub(crate) fn jam_tune(&mut self, g: usize, sound: Option<usize>, semitones: f64) {
        use crate::layout::Act;
        use deckard_doc::Target;
        let Some(doc) = self.doc.clone() else { return };
        let tunes: Vec<(usize, f64)> = {
            let d = doc.borrow();
            let Some(group) = d.project().groups.get(g) else { return };
            group.sounds.iter().enumerate().filter_map(|(s, snd)| snd.sampler.as_ref().map(|x| (s, x.tune))).collect()
        };
        let acts = match sound {
            Some(s) => tunes.iter().filter(|(x, _)| *x == s).map(|_| Act::SetParam(Target::Sampler(g, s), "tune", semitones)).collect(),
            None => {
                let Some(&(_, first)) = tunes.first() else { return };
                let by = semitones - first;
                tunes.iter().map(|&(s, t)| Act::SetParam(Target::Sampler(g, s), "tune", (t + by).clamp(-36.0, 36.0))).collect()
            }
        };
        crate::layout::apply_acts(self, acts);
    }

    /// Control knob `pos` back to its default.
    pub(crate) fn jam_reset_knob(&mut self, pos: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let knobs = if self.channel {
            let Some((_, _, pg)) = crate::layout::channel_page(self, &doc.borrow()) else { return };
            pg.slots
        } else {
            let Some(pg) = self.slot.and_then(|i| crate::layout::slot_page(self, doc.borrow().project(), i)) else { return };
            pg.knobs
        };
        if let Some(Some((_, _, _, Some(edit)))) = knobs.get(pos).cloned() {
            crate::layout::apply_acts(self, vec![edit.reset()]);
        }
    }
}
