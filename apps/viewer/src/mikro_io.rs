//! The Maschine Mikro MK3 over USB (`deckard-mikro`): a dedicated thread reads the controller.
//! Pads play straight from that thread, as the MK3's and JAM's do, so a note never waits for a
//! frame; every event also goes to the app, which hands it to the controller script
//! (`crates/surface/lua/devices/maschine_mikro_mk3`).
//!
//! Preferences ▸ MIDI switches it off.

use crate::App;
use deckard_engine::Live;
use deckard_engine::output::Sender;
use deckard_mikro::{Event, Mikro, Screen, leds::Leds};
use deckard_surface::device::{Device, Out, PadMap, PadNotes};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The script's id: `devices/<id>/`.
pub const DEVICE: &str = "maschine_mikro_mk3";

/// How often the lights and OLED screen want redrawing when nothing else asks.
const FRAME: Duration = Duration::from_millis(16);

#[derive(Default)]
struct Shared {
    /// Preferences' switch (until told, on: the app sets it as it starts).
    enabled: bool,
    connected: bool,
    status: String,
    events: Vec<Event>,
    /// Pad hits that came while no engine played: (group, sound, pitch, velocity).
    pending: Vec<(usize, usize, i64, i64)>,
    output: Option<Sender>,
    route: Option<PadMap>,
    velocity_scaling: i8,
    leds: Option<Leds>,
    screen: Option<Screen>,
    repaint: Option<eframe::egui::Context>,
    /// DECKARD is closing: lights and screen off, then stop.
    closing: bool,
    closed: bool,
}

pub struct MikroIo {
    shared: Arc<Mutex<Shared>>,
}

impl MikroIo {
    /// Start looking for a Maschine Mikro MK3 (and keep looking: it may be plugged in later).
    pub fn start() -> MikroIo {
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let s = shared.clone();
        std::thread::Builder::new().name("DECKARD MIKRO".into()).spawn(move || run(&s)).ok();
        MikroIo { shared }
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

    pub fn set_velocity_scaling(&self, scaling: i8) {
        self.shared.lock().unwrap().velocity_scaling = scaling;
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

    /// Update the 128×32 monochrome OLED screen.
    pub fn set_screen(&self, screen: Screen) {
        self.shared.lock().unwrap().screen = Some(screen);
    }

    /// Leave the lights and screen dark. Waits (up to a second) for the thread to do it.
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

/// Convert pad 1..16 to PadMap 0..63 index.
/// Pad 1 is bottom-left (col 0, row 3).
/// In PadMap: idx = col + row * 8.
fn pad_map_index(pad: usize) -> usize {
    if !(1..=16).contains(&pad) {
        return 0;
    }
    let col = (pad - 1) % 4;
    let row = 3 - (pad - 1) / 4;
    col + row * 8
}

/// What a pad's hit does: the notes it started and velocity.
/// Respects per-pad velocity overrides (e.g. 16 Velocities), Fixed Velocity mode,
/// or falls back to pressure-sensitive velocity with the user's velocity scaling curve.
pub(crate) fn hit(route: Option<&PadMap>, pad: usize, pressure: f64, scaling: i8) -> (PadNotes, i64) {
    let pressure_vel = ((crate::mk3_io::pad_velocity(pressure, scaling) * 127.0).round() as i64).clamp(1, 127);
    match route {
        Some(r) => {
            let idx = pad_map_index(pad);
            let notes = r.pads.get(idx).cloned().unwrap_or_default();
            let vel = r
                .pad_velocities
                .get(idx)
                .copied()
                .flatten()
                .or_else(|| r.fixed_velocity.then_some(r.velocity.clamp(1, 127)))
                .unwrap_or(pressure_vel);
            (notes, vel)
        }
        None => (Vec::new(), pressure_vel),
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
        let mut mikro = match Mikro::open() {
            Ok(m) => m,
            Err(e) => {
                shared.lock().unwrap().status = e;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        let _polling = crate::controller_timing::Polling::start();
        {
            let mut s = shared.lock().unwrap();
            s.connected = true;
            s.status = "Maschine Mikro MK3 connected".into();
        }
        let mut events = Vec::new();
        // What each pad (1..16) started, so it stops the same notes however the route changed.
        let mut sounding: Vec<PadNotes> = vec![Vec::new(); 16];
        loop {
            let (closing, enabled, leds, screen, scaling) = {
                let s = shared.lock().unwrap();
                (s.closing, s.enabled, s.leds.clone(), s.screen.clone(), s.velocity_scaling)
            };
            if closing || !enabled {
                let _ = mikro.dark();
                let mut s = shared.lock().unwrap();
                s.closed = closing;
                s.connected = false;
                release(&s, &mut sounding);
                if closing {
                    return;
                }
                break;
            }

            // Write LEDs (only changes sent over HID report 0x80)
            let wrote_leds = leds.as_ref().map_or(Ok(()), |l| mikro.show(l));

            // Write Screen (report 0xe0)
            let wrote_screen = if let Some(scr) = screen.as_ref().filter(|s| s.is_dirty()) {
                mikro.write_screen(scr)
            } else {
                Ok(())
            };

            let read = wrote_leds.and(wrote_screen).and_then(|()| mikro.poll(2, &mut events));
            if let Err(e) = read {
                let mut s = shared.lock().unwrap();
                s.status = format!("Maschine Mikro MK3: {e}");
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
                match *e {
                    Event::PadOn(pad, pressure) if (1..=16).contains(&pad) => {
                        let (notes, velocity) = hit(route.as_ref(), pad, pressure, scaling);
                        match &out {
                            Some(o) => {
                                for &(group, sound, pitch) in &notes {
                                    o.live(Live::NoteOn { group, sound, pitch, velocity, length: i64::MAX });
                                }
                                sounding[pad - 1] = notes;
                            }
                            None => s.pending.extend(notes.iter().map(|&(g, n, p)| (g, n, p, velocity))),
                        }
                    }
                    Event::PadOff(pad) if (1..=16).contains(&pad) => {
                        if let Some(o) = &out {
                            for (group, sound, pitch) in std::mem::take(&mut sounding[pad - 1]) {
                                o.live(Live::NoteRelease { group, sound, pitch });
                            }
                        }
                    }
                    _ => {}
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

/// Coalesce consecutive touch strip moves in one frame to prevent overshoot.
pub(crate) fn coalesced(events: Vec<Event>) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    for e in events {
        if let Event::Strip(1, _) = e
            && let Some(last) = out.last_mut()
            && matches!(last, Event::Strip(1, _))
        {
            *last = e;
            continue;
        }
        out.push(e);
    }
    out
}

/// Release any sounding notes when the Mikro MK3 disconnects or is turned off.
fn release(s: &Shared, sounding: &mut [PadNotes]) {
    let Some(o) = s.output.as_ref().filter(|o| o.has_engine()) else { return };
    for notes in sounding {
        for (group, sound, pitch) in std::mem::take(notes) {
            o.live(Live::NoteRelease { group, sound, pitch });
        }
    }
}

impl App {
    /// Synchronize the Maschine Mikro MK3 controller state with the app and surface scripts.
    pub(crate) fn mikro_sync(&mut self, ctx: &eframe::egui::Context, covered: bool) {
        let Some(m) = &self.mikro else { return };
        m.set_context(ctx);
        m.set_enabled(self.settings.mikro_enabled);
        m.set_velocity_scaling(self.settings.velocity_scaling);
        let connected = m.connected();
        let (events, pending) = (m.take_events(), m.take_pending());
        let now = Instant::now();

        if !connected {
            if let Some(mut d) = self.mikro_device.take() {
                let asked = d.close();
                self.mikro_asked(asked);
                self.status = "Maschine Mikro MK3 disconnected".into();
            }
            return;
        }

        if self.mikro_device.is_none() {
            let user = crate::menu::settings_dir().map(|d| d.join("controller"));
            let mut d = Device::new(DEVICE, user);
            let view = self.surface_view();
            let asked = d.open(&view, now);
            self.mikro_device = Some(d);
            self.mikro_asked(asked);
            self.status = "Maschine Mikro MK3 connected".into();
            eprintln!("Maschine Mikro MK3 connected");
        }

        let (leds, screen, route) = self.mikro_events(events, now);
        if let Some(m) = &self.mikro {
            m.set_leds(leds.scaled(self.settings.led_brightness));
            m.set_screen(screen);
            m.set_route(route);
        }

        // Pad hits that came while no engine ran
        for (group, sound, pitch, velocity) in pending {
            self.play_note(Live::NoteOn { group, sound, pitch, velocity, length: deckard_model::PPQ });
        }

        self.input_sync();
        ctx.request_repaint_after(if covered { crate::undrawn(ctx, FRAME) } else { FRAME });
    }

    /// Process Mikro events through its Lua device script: produce LEDs, OLED screen, and pad route.
    pub(crate) fn mikro_events(&mut self, events: Vec<Event>, now: Instant) -> (Leds, Screen, PadMap) {
        self.update_slot_view();
        let view = self.surface_view();
        let Some(mut d) = self.mikro_device.take() else {
            return (Leds::default(), Screen::default(), PadMap::default());
        };

        let mut asked = d.poll(now);
        for e in coalesced(events) {
            asked.extend(match e {
                Event::Button(name, down) => d.button(name, down, &view, now),
                Event::EncoderTurn(delta) => d.encoder(delta as i32, &view, now),
                Event::Strip(kind, pos) => {
                    let kind_str = match kind {
                        0 => "down",
                        1 => "move",
                        _ => "up",
                    };
                    d.strip(0, 0, kind_str, pos, &view, now)
                }
                Event::PadOn(pad, _) => {
                    let col = ((pad - 1) % 4) as u8;
                    let row = (3 - (pad - 1) / 4) as u8;
                    d.pad(col, row, true, &view, now)
                }
                Event::PadOff(pad) => {
                    let col = ((pad - 1) % 4) as u8;
                    let row = (3 - (pad - 1) / 4) as u8;
                    d.pad(col, row, false, &view, now)
                }
                Event::PadPressure(_, _) => Vec::new(),
            });
        }
        asked.extend(d.tick(&view, now));
        let view_stale = !asked.is_empty();
        self.mikro_asked(asked);
        // Restore `mikro_device` before calling `surface_view()` so `self.mikro_device.is_some()`
        // remains true when populating `v.browser` and `v.control.modules` for LEDs and OLED.
        self.mikro_device = Some(d);
        let view = if view_stale {
            self.update_slot_view();
            self.surface_view()
        } else {
            view
        };
        let d = self.mikro_device.as_mut().expect("restored above");
        let (leds, screen, route) = (d.mikro_leds(&view, now), d.mikro_screen(&view, now), d.pad_map(&view, now));

        let error = d.error();
        if error != self.mikro_error {
            if let Some(e) = &error {
                self.status = format!("controller script: {e}");
            }
            self.mikro_error = error;
        }
        (leds, screen, route)
    }

    /// Handle actions requested by the Mikro MK3 Lua script.
    fn mikro_asked(&mut self, asked: Vec<Out>) {
        for a in asked {
            match a {
                Out::App(a) => self.surface_act(a),
                Out::Print(p) => self.print_notes(&p),
                Out::Message(m) => self.status = m,
                Out::Midi { .. } => {}
            }
        }
    }
}
