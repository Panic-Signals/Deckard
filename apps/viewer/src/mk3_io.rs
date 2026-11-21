//! The Maschine MK3 over USB (`deckard-mk3`): a thread reads the controller. Pads
//! play the focused group's sounds straight from that thread (stamped, like MIDI in,
//! so they don't wait for a frame); every event also goes to the app, which hands it to
//! the controller scripts.

use crate::pad_play::Pads;
use deckard_engine::Live;
use deckard_engine::output::Sender;
use deckard_mk3::Event;
use deckard_model::pad_touch::PadTouch;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use crate::pad_play::{Touching, pad_velocity};
// (The pad core's tests moved with it; these are theirs.)
#[cfg(test)]
pub use crate::pad_play::touch_value;

/// Where pads play: the scripts' focus, kept up to date by the app.
#[derive(Clone, Copy)]
pub struct PadRoute {
    /// Pads play sounds (not picking scenes, sections, …).
    pub play: bool,
    pub group: usize,
    /// Keyboard mode: every pad plays this sound, at its own note (128: none).
    pub keys: Option<(usize, [i64; 16])>,
    /// Chords mode: the keys' sound plays each pad's chord instead (128: none).
    pub chords: Option<[[i64; 16]; 16]>,
    /// Fixed Velocity: every hit plays at this velocity, however hard.
    pub velocity: Option<i64>,
    /// Velocity scaling, -3 (Soft 3) … 3 (Hard 3) (see [`pad_velocity`]). The pad
    /// sensitivity isn't here: the controller applies it (`Mk3Io::set_pad_sensitivity`).
    pub scaling: i8,
    /// The song's Fixed Velocity value: what hits play at in the pressure pad modes.
    pub fixed: i64,
    /// The group's sounds' Pad Touch (what pressing a held pad does for each).
    pub touches: [PadTouch; 16],
}

impl PadRoute {
    /// A sound's Pad Touch, if it's in the route's group.
    pub(crate) fn touch(&self, group: usize, sound: usize) -> Option<PadTouch> {
        (group == self.group).then(|| self.touches.get(sound).copied()).flatten()
    }
}

/// The notes the hits queued while no engine ran play (`take_pending`'s (group, sound,
/// pitch, velocity); velocity 0, a release, plays nothing): each at its velocity, a
/// beat long.
pub fn pending_notes(pending: &[(usize, usize, i64, i64)]) -> Vec<Live> {
    pending
        .iter()
        .filter(|p| p.3 > 0)
        .map(|&(group, sound, pitch, velocity)| Live::NoteOn { group, sound, pitch, velocity, length: deckard_model::PPQ })
        .collect()
}

#[derive(Clone)]
pub enum ScreenFrame {
    Picture(deckard_surface::Picture),
    Pixels(Arc<Vec<[u8; 4]>>),
}

impl ScreenFrame {
    pub fn into_pixels(self) -> Vec<[u8; 4]> {
        match self {
            ScreenFrame::Picture(p) => p.into_pixels(),
            ScreenFrame::Pixels(px) => Arc::unwrap_or_clone(px),
        }
    }
}

/// What a pad's pressure (or release) sends: every `Live` to the engine, and each
/// pad-touch event to the monitor as well (`pad_row` says which).
pub(crate) fn forward_pad_lives(lives: Vec<Live>, mut engine: impl FnMut(Live), mut row: impl FnMut(crate::midi_monitor::Pad)) {
    for l in lives {
        let ev = pad_row(&l);
        engine(l);
        if let Some(ev) = ev {
            row(ev);
        }
    }
}

/// The MIDI Monitor's row for a pad's live event, if it gets one: the pad-touch
/// events (pressure, poly aftertouch, a CC or channel aftertouch). A note's row is
/// the note's own (`Pad::On`, `Pad::Off`).
fn pad_row(l: &Live) -> Option<crate::midi_monitor::Pad> {
    use crate::midi_monitor::Pad;
    match *l {
        Live::RepeatPressure { velocity, .. } => Some(Pad::Pressure(velocity.clamp(0, 127) as u8)),
        Live::Expression { value, .. } => Some(Pad::PolyAt((value * 127.0).round() as u8)),
        Live::ControlIn { kind, .. } => Some(Pad::Control(kind)),
        _ => None,
    }
}
#[derive(Default)]
struct Shared {
    events: Vec<Event>,
    /// The pads, playing: what each hit started, what a held pad's pressure sends,
    /// and the hits that came while no engine played.
    pads: Pads,
    output: Option<Sender>,
    route: Option<PadRoute>,
    status: String,
    repaint: Option<eframe::egui::Context>,
    /// What the LEDs should show.
    leds: Option<deckard_mk3::leds::Leds>,
    #[cfg(test)]
    leds_applied: Option<(deckard_mk3::leds::Leds, Instant)>,
    /// What the displays should show: both side by side, 960 × 272 RGBA, and its number.
    screens: Option<(u64, ScreenFrame)>,
    display_status: String,
    /// An MK3 is plugged in, and NI's software is why its screens won't open.
    agent_holding: bool,
    /// Leave the screens alone, as they're being given back to NI's driver.
    displays_released: bool,
    /// The displays' backlight, 0..=100 (`set_display_brightness`).
    display_brightness: Option<u8>,
    /// The LEDs' global level, 0..=100 % (`set_led_brightness`): output report `0xf3`.
    led_brightness: Option<u8>,
    /// Bumped whenever that level must be written again: it changed, or the screens
    /// came back (a driver change leaves the panels at their own 20%).
    brightness_stamp: u64,
    /// The same for the LEDs' level, which a replug also resets.
    led_stamp: u64,
    /// The pads' sensitivity, 0..=100 % (`set_pad_sensitivity`), and a stamp bumped only
    /// when it changes: it's a feature report read and written back, not worth every frame.
    pad_sensitivity: Option<u8>,
    sensitivity_stamp: u64,
    /// DECKARD is closing: dark LEDs, the goodbye picture on the displays, then stop
    /// (`shut_down`).
    closing: bool,
    goodbye: Option<Arc<Vec<[u8; 4]>>>,
    /// Whether the controller and the displays are open, and whether each has been left
    /// dark since `closing`.
    connected: bool,
    displays_connected: bool,
    leds_closed: bool,
    displays_closed: bool,
    /// The MIDI Monitor, shown what the pads play.
    monitor: Option<crate::midi_monitor::Monitor>,
}

#[derive(Clone)]
pub struct Mk3Io {
    shared: Arc<Mutex<Shared>>,
    display_wake: Arc<std::sync::Condvar>,
}

/// The pitch pads play: a sound's base key (Maschine's default, C3).
const PAD_PITCH: i64 = 60;

/// The notes (sound, pitch) a hit on `pad` (1–16) plays: its sound at C3, its key in
/// Keyboard mode, or its chord in Chords mode; none for a pad with nothing to play.
pub fn pad_notes(route: &PadRoute, pad: usize) -> Vec<(usize, i64)> {
    let Some(i) = pad.checked_sub(1).filter(|&i| i < 16) else { return Vec::new() };
    match (route.keys, route.chords) {
        (Some((sound, _)), Some(chords)) => chords[i].iter().filter(|&&n| n < 128).map(|&n| (sound, n)).collect(),
        (Some((sound, notes)), None) => notes.get(i).filter(|&&n| n < 128).map(|&n| vec![(sound, n)]).unwrap_or_default(),
        (None, _) => vec![(i, PAD_PITCH)],
    }
}

impl Mk3Io {
    /// Start looking for an MK3 (and keep looking: it may be plugged in later).
    pub fn start() -> Mk3Io {
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let display_wake = Arc::new(std::sync::Condvar::new());
        let s = shared.clone();
        std::thread::Builder::new().name("DECKARD MK3".into()).spawn(move || run(&s)).ok();
        let s = shared.clone();
        let w = display_wake.clone();
        std::thread::Builder::new().name("DECKARD MK3 displays".into()).spawn(move || run_displays(&s, &w)).ok();
        Mk3Io { shared, display_wake }
    }

    /// Leave the controller dark but for `picture` on the displays (both, 960 × 272
    /// RGBA): LEDs off. Waits (up to a second) for both threads to do it.
    pub fn shut_down(&self, picture: Vec<[u8; 4]>) {
        {
            let mut s = self.shared.lock().unwrap();
            s.goodbye = Some(Arc::new(picture));
            s.closing = true;
        }
        self.display_wake.notify_all();
        let until = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < until {
            let s = self.shared.lock().unwrap();
            if (s.leds_closed || !s.connected) && (s.displays_closed || !s.displays_connected) {
                return;
            }
            drop(s);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Set the displays' backlight (0..=100), now and whenever an MK3 connects.
    pub fn set_display_brightness(&self, level: u8) {
        let mut s = self.shared.lock().unwrap();
        s.display_brightness = Some(level.min(100));
        s.brightness_stamp = s.brightness_stamp.wrapping_add(1);
    }

    /// Set the LEDs' global level (0..=100 %), now and whenever an MK3 connects.
    pub fn set_led_brightness(&self, level: u8) {
        let mut s = self.shared.lock().unwrap();
        s.led_brightness = Some(level.min(100));
        s.led_stamp = s.led_stamp.wrapping_add(1);
    }

    /// Set the pads' sensitivity (0..=100 %), now if it changed and whenever an MK3
    /// connects. The controller applies it to the pressure it reports, as with Maschine.
    pub fn set_pad_sensitivity(&self, level: u8) {
        let mut s = self.shared.lock().unwrap();
        let level = Some(level.min(100));
        if s.pad_sensitivity != level {
            s.pad_sensitivity = level;
            s.sensitivity_stamp = s.sensitivity_stamp.wrapping_add(1);
        }
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

    /// Show what the pads play in the MIDI Monitor.
    pub fn set_monitor(&self, m: crate::midi_monitor::Monitor) {
        let mut s = self.shared.lock().unwrap();
        if s.monitor.is_none() {
            s.monitor = Some(m);
        }
    }

    pub fn set_route(&self, route: PadRoute) {
        self.shared.lock().unwrap().route = Some(route);
    }

    /// Light the LEDs so (sent when they change).
    pub fn set_leds(&self, leds: deckard_mk3::leds::Leds) {
        self.shared.lock().unwrap().leds = Some(leds);
    }

    #[cfg(test)]
    pub(crate) fn leds_applied_at(&self, leds: &deckard_mk3::leds::Leds) -> Option<Instant> {
        self.shared.lock().unwrap().leds_applied.as_ref().filter(|(l, _)| l == leds).map(|(_, t)| *t)
    }

    /// Show these on the displays (both side by side, 960 × 272); `n` changes when
    /// the picture does.
    pub fn set_screens(&self, n: u64, frame: ScreenFrame) {
        let mut s = self.shared.lock().unwrap();
        if s.screens.as_ref().is_none_or(|(m, _)| *m != n) {
            s.screens = Some((n, frame));
            self.display_wake.notify_one();
        }
    }

    /// What the controller did since the last call.
    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut self.shared.lock().unwrap().events)
    }

    /// Whether NI's software is keeping the MK3's screens from opening.
    pub fn agent_holding(&self) -> bool {
        self.shared.lock().unwrap().agent_holding
    }

    /// Whether the MK3's screens are open.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn displays_connected(&self) -> bool {
        self.shared.lock().unwrap().displays_connected
    }

    /// Let go of the screens and leave them alone (they're being given back to NI's
    /// driver), or take them up again.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn release_displays(&self, released: bool) {
        self.shared.lock().unwrap().displays_released = released;
        self.display_wake.notify_one();
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn displays_released(&self) -> bool {
        self.shared.lock().unwrap().displays_released
    }

    /// Pad hits for the app to play itself (no engine was running).
    pub fn take_pending(&self) -> Vec<(usize, usize, i64, i64)> {
        self.shared.lock().unwrap().pads.pending()
    }
}

#[cfg(test)]
impl Mk3Io {
    /// A driver that never touches USB: no threads, no device. What it stored is then
    /// exactly what the hardware sink forwarded.
    pub(crate) fn detached() -> Mk3Io {
        Mk3Io { shared: Arc::default(), display_wake: Arc::new(std::sync::Condvar::new()) }
    }

    /// What the last `show` stored for the device threads: the route, the LEDs, the screens.
    pub(crate) fn stored_route(&self) -> Option<PadRoute> {
        self.shared.lock().unwrap().route
    }

    pub(crate) fn stored_leds(&self) -> Option<deckard_mk3::leds::Leds> {
        self.shared.lock().unwrap().leds.clone()
    }

    pub(crate) fn stored_screens(&self) -> Option<(u64, Vec<[u8; 4]>)> {
        self.shared.lock().unwrap().screens.clone().map(|(n, px)| (n, px.into_pixels()))
    }
}

fn run(shared: &Mutex<Shared>) {
    loop {
        let mut dev = match deckard_mk3::Device::open() {
            Ok(d) => d,
            Err(e) => {
                shared.lock().unwrap().status = e;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        // Clear the preceding host/session before waiting for the first UI state.
        if let Err(e) = dev.write_leds(&deckard_mk3::leds::Leds::default()) {
            shared.lock().unwrap().status = format!("Maschine MK3: {e}");
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        let _polling = crate::controller_timing::Polling::start();
        {
            let mut s = shared.lock().unwrap();
            s.status = format!("{} connected", dev.model().name);
            s.connected = true;
            // A fresh connection: nothing is held.
            s.pads = Pads::new();
        }
        let mut events = Vec::new();
        // Shown on the LEDs: none yet, so the first state is sent.
        let mut shown = None;
        // The backlight write that succeeded, so a failed one is tried again and a
        // screen reconnect (a new stamp) is sent even when the level did not change.
        let mut applied_stamp = 0u64;
        let mut brightness_retry = Instant::now();
        // The same for the LEDs' level (report `0xf3`), which is not read back.
        let mut applied_led_stamp = 0u64;
        let mut led_retry = Instant::now();
        // The same for the pads' sensitivity: written on connecting, then as it changes.
        let mut applied_sensitivity_stamp = 0u64;
        let mut sensitivity_retry = Instant::now();
        loop {
            if shared.lock().unwrap().closing {
                let _ = dev.write_leds(&deckard_mk3::leds::Leds::default());
                shared.lock().unwrap().leds_closed = true;
                return;
            }
            if let Err(e) = dev.poll(2, &mut events) {
                let mut s = shared.lock().unwrap();
                s.status = format!("Maschine MK3: {e}");
                s.connected = false;
                break;
            }
            let (wanted, stamp) = {
                let s = shared.lock().unwrap();
                (s.display_brightness, s.brightness_stamp)
            };
            if let Some(level) = wanted
                && stamp != applied_stamp
                && Instant::now() >= brightness_retry
            {
                match dev.set_display_brightness(level) {
                    Ok(()) => applied_stamp = stamp,
                    Err(e) => {
                        shared.lock().unwrap().display_status = format!("display brightness: {e}");
                        brightness_retry = Instant::now() + Duration::from_secs(1);
                    }
                }
            }
            let (wanted_leds, led_stamp) = {
                let s = shared.lock().unwrap();
                (s.led_brightness, s.led_stamp)
            };
            if let Some(level) = wanted_leds
                && led_stamp != applied_led_stamp
                && Instant::now() >= led_retry
            {
                match dev.set_led_brightness(level) {
                    Ok(()) => applied_led_stamp = led_stamp,
                    Err(e) => {
                        shared.lock().unwrap().status = format!("LED brightness: {e}");
                        led_retry = Instant::now() + Duration::from_secs(1);
                    }
                }
            }
            let (wanted_sensitivity, sensitivity_stamp) = {
                let s = shared.lock().unwrap();
                (s.pad_sensitivity, s.sensitivity_stamp)
            };
            if let Some(level) = wanted_sensitivity
                && sensitivity_stamp != applied_sensitivity_stamp
                && Instant::now() >= sensitivity_retry
            {
                match dev.set_pad_sensitivity(level) {
                    Ok(()) => applied_sensitivity_stamp = sensitivity_stamp,
                    Err(e) => {
                        shared.lock().unwrap().status = format!("pad sensitivity: {e}");
                        sensitivity_retry = Instant::now() + Duration::from_secs(1);
                    }
                }
            }
            let leds = shared.lock().unwrap().leds.clone();
            if leds.is_some() && leds != shown {
                if let Some(l) = &leds
                    && let Err(e) = dev.write_leds(l)
                {
                    let mut s = shared.lock().unwrap();
                    s.status = format!("Maschine MK3: {e}");
                    s.connected = false;
                    break;
                }
                #[cfg(test)]
                if let Some(l) = &leds {
                    shared.lock().unwrap().leds_applied = Some((l.clone(), Instant::now()));
                }
                shown = leds;
            }
            if events.is_empty() {
                continue;
            }
            let mut s = shared.lock().unwrap();
            let route = s.route.unwrap_or(PadRoute {
                play: true,
                group: 0,
                keys: None,
                chords: None,
                velocity: None,
                scaling: 0,
                fixed: 100,
                touches: [PadTouch::default(); 16],
            });
            let out = s.output.clone().filter(Sender::has_engine);
            // The pads play now, or wait for the engine that isn't running yet.
            s.pads.set_engine(out.is_some());
            let monitor = s.monitor.clone().filter(crate::midi_monitor::Monitor::is_open);
            let shown = |pad: usize, slot: Option<(usize, usize, i64)>, ev| {
                if let Some(m) = &monitor {
                    m.pad(pad, slot.map(|x| x.2), ev);
                }
            };
            // What a pad's pressure (or release) sends, to the engine and the monitor.
            let send = |pad: usize, slot: Option<(usize, usize, i64)>, lives: Vec<Live>| {
                forward_pad_lives(
                    lives,
                    |l| {
                        if let Some(o) = &out {
                            o.live(l);
                        }
                    },
                    |ev| shown(pad, slot, ev),
                );
            };
            for e in &events {
                if let Event::PadPressure(p, v) = *e {
                    let slot = s.pads.sounding(p);
                    if out.is_some() {
                        send(p, slot, s.pads.pressure(&route, p, v, slot));
                    }
                    continue;
                }
                let (pad, from_pressure) = match *e {
                    Event::PadOn(p, v) => (p, ((pad_velocity(v, route.scaling) * 127.0).round() as i64).clamp(1, 127)),
                    Event::PadOff(p) => (p, 0),
                    _ => continue,
                };
                if from_pressure == 0 {
                    let slot = s.pads.sounding(pad);
                    let lives = s.pads.release(pad);
                    if out.is_some() && slot.is_some() {
                        shown(pad, slot, crate::midi_monitor::Pad::Off);
                        send(pad, slot, lives);
                    }
                    continue;
                }
                // The notes the hit plays, or the ones it queues for the engine that
                // isn't running yet.
                let lives = s.pads.hit(&route, pad, from_pressure);
                if let (Some(o), Some(&Live::NoteOn { velocity, .. })) = (&out, lives.first()) {
                    for l in &lives {
                        o.live(*l);
                    }
                    shown(pad, s.pads.sounding(pad), crate::midi_monitor::Pad::On(velocity));
                }
            }
            s.events.append(&mut events);
            if let Some(ctx) = &s.repaint {
                ctx.request_repaint();
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn set_agent_holding(s: &mut Shared, holding: bool) {
    if s.agent_holding != holding {
        s.agent_holding = holding;
        if let Some(ctx) = &s.repaint {
            ctx.request_repaint();
        }
    }
}

/// Keep the displays showing the latest picture (the displays are their own USB
/// interface, written apart from the buttons and LEDs).
fn run_displays(shared: &Mutex<Shared>, wake: &std::sync::Condvar) {
    use deckard_mk3::display::{Displays, HEIGHT, WIDTH, rgb565};
    let mut left = vec![0u16; WIDTH * HEIGHT];
    let mut right = vec![0u16; WIDTH * HEIGHT];
    // Fill `out` with one 480×272 half of a 960-wide RGBA picture.
    let fill_half = |px: &[[u8; 4]], x0: usize, out: &mut [u16]| {
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let [r, g, b, _] = px.get(y * WIDTH * 2 + x0 + x).copied().unwrap_or([0, 0, 0, 255]);
                out[y * WIDTH + x] = rgb565(r, g, b);
            }
        }
    };
    loop {
        {
            let mut s = shared.lock().unwrap();
            if s.closing {
                s.displays_closed = true;
                return;
            }
            if s.displays_released {
                set_agent_holding(&mut s, false);
                drop(s);
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        }
        let mut d = match Displays::open() {
            Ok(d) => d,
            Err(e) => {
                let holding = crate::ni_agent::holding(&e);
                let mut s = shared.lock().unwrap();
                s.display_status = e;
                set_agent_holding(&mut s, holding);
                drop(s);
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        {
            let mut s = shared.lock().unwrap();
            s.display_status.clear();
            s.displays_connected = true;
            // The panels forget their backlight when the screen driver changes.
            s.brightness_stamp = s.brightness_stamp.wrapping_add(1);
            set_agent_holding(&mut s, false);
        }
        let mut shown = None;
        let mut last_px: Vec<[u8; 4]> = Vec::new();
        loop {
            let (closing, released, next_screen) = {
                let mut s = shared.lock().unwrap();
                loop {
                    if s.closing {
                        break (Some(s.goodbye.clone()), s.displays_released, None);
                    }
                    if s.displays_released {
                        break (None, true, None);
                    }
                    if let Some((n, frame)) = &s.screens
                        && shown != Some(*n)
                    {
                        break (None, false, Some((*n, frame.clone())));
                    }
                    s = wake.wait(s).unwrap();
                }
            };
            if let Some(goodbye) = closing {
                let px = goodbye.map_or_else(Vec::new, |p| p.to_vec());
                fill_half(&px, 0, &mut left);
                fill_half(&px, WIDTH, &mut right);
                let _ = d.show(0, &left).and_then(|_| d.show(1, &right));
                shared.lock().unwrap().displays_closed = true;
                return;
            }
            if released {
                // Clear both screens before letting go. Otherwise the panels keep the
                // last picture after the driver changes.
                left.fill(0);
                let _ = d.show(0, &left).and_then(|_| d.show(1, &left));
                shared.lock().unwrap().displays_connected = false;
                break;
            }
            if let Some((n, frame)) = next_screen {
                let px = frame.into_pixels();
                if px != last_px {
                    fill_half(&px, 0, &mut left);
                    fill_half(&px, WIDTH, &mut right);
                    if let Err(e) = d.show(0, &left).and_then(|_| d.show(1, &right)) {
                        let mut s = shared.lock().unwrap();
                        s.display_status = e;
                        s.displays_connected = false;
                        break;
                    }
                    last_px = px;
                }
                shown = Some(n);
            }
        }
        // Let go now: the screens' driver may be about to change.
        drop(d);
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::pad_touch::{PadTouch, TouchMode};
    use deckard_model::{Dim, ExprKind, MidiKind};

    fn route(touch: PadTouch) -> PadRoute {
        let mut touches = [PadTouch::default(); 16];
        touches[0] = touch;
        PadRoute { play: true, group: 0, keys: None, chords: None, velocity: None, scaling: 0, fixed: 90, touches }
    }

    fn mode(mode: TouchMode) -> PadTouch {
        PadTouch { mode, cc: 74, ..PadTouch::default() }
    }

    /// Keyboard mode: every pad plays sound 0, pad 1 at C3, pad 2 at E3.
    fn keys(touch: PadTouch) -> PadRoute {
        let mut notes = [128; 16];
        (notes[0], notes[1]) = (60, 64);
        PadRoute { keys: Some((0, notes)), ..route(touch) }
    }

    #[test]
    fn pads_play_a_sound_a_key_or_a_chord() {
        let plain = route(PadTouch::default());
        assert_eq!(pad_notes(&plain, 3), [(2, PAD_PITCH)]);
        let k = keys(PadTouch::default());
        assert_eq!(pad_notes(&k, 2), [(0, 64)]);
        assert!(pad_notes(&k, 3).is_empty(), "a pad with no note plays nothing");
        let mut chords = [[128; 16]; 16];
        chords[0][..3].copy_from_slice(&[60, 64, 67]);
        let c = PadRoute { chords: Some(chords), ..k };
        assert_eq!(pad_notes(&c, 1), [(0, 60), (0, 64), (0, 67)]);
        assert!(pad_notes(&c, 2).is_empty(), "an empty chord plays nothing");
    }

    #[test]
    fn touch_value_maps_into_min_and_max() {
        let t = PadTouch { min: 20, max: 90, ..PadTouch::default() };
        assert_eq!(touch_value(0.0, &t), 25);
        assert_eq!(touch_value(1.0, &t), 114);
        assert_eq!(touch_value(0.5, &t), 70);
        let flat = PadTouch { min: 40, max: 40, ..PadTouch::default() };
        assert_eq!((touch_value(0.0, &flat), touch_value(1.0, &flat)), (51, 51));
    }

    /// The curve: above the middle, light presses count for more; below, less; the
    /// ends stay put.
    #[test]
    fn the_curve_bends_the_middle() {
        let c = |curve| PadTouch { curve, ..PadTouch::default() };
        assert!(touch_value(0.5, &c(80)) > 76);
        assert!(touch_value(0.5, &c(20)) < 51);
        for curve in [0, 50, 100] {
            assert_eq!((touch_value(0.0, &c(curve)), touch_value(1.0, &c(curve))), (0, 127));
        }
    }

    #[test]
    fn each_mode_sends_its_kind() {
        let pressure = |m: TouchMode| {
            let r = route(mode(m));
            let mut t = Touching::default();
            t.hit(1, (0, 0, 60), &r);
            t.pressure(1, Some((0, 0, 60)), 1.0, &r)
        };
        assert_eq!(pressure(TouchMode::Velocity), vec![Live::RepeatPressure { group: 0, sound: 0, pitch: 60, velocity: 127 }]);
        let at = ExprKind::Dim(Dim::Pressure);
        assert_eq!(pressure(TouchMode::PolyAt), vec![Live::Expression { group: 0, sound: 0, pitch: 60, kind: at, value: 1.0 }]);
        let control = |kind| vec![Live::ControlIn { group: 0, sound: 0, kind, thru: true, record: true }];
        assert_eq!(pressure(TouchMode::Aftertouch), control(MidiKind::ChannelPressure(127)));
        assert_eq!(pressure(TouchMode::Cc), control(MidiKind::Controller { number: 74, value: 127 }));
    }

    #[test]
    fn pressure_modes_hit_at_fixed_velocity() {
        for m in [TouchMode::PolyAt, TouchMode::Aftertouch, TouchMode::Cc] {
            assert_eq!(Touching::hit_velocity(&route(mode(m)), 0, 0, 30), 90, "{m:?}");
        }
        assert_eq!(Touching::hit_velocity(&route(mode(TouchMode::Velocity)), 0, 0, 30), 30);
        // The pads' own Fixed Velocity still wins in the Velocity mode.
        let fixed = PadRoute { velocity: Some(110), ..route(PadTouch::default()) };
        assert_eq!(Touching::hit_velocity(&fixed, 0, 0, 30), 110);
    }

    /// Aftertouch from two held pads of a sound: the harder one counts; letting go of
    /// it leaves the other's; letting go of the last sends Min.
    #[test]
    fn the_hardest_pad_wins_and_release_sends_min() {
        let r = keys(PadTouch { mode: TouchMode::Aftertouch, min: 10, ..PadTouch::default() });
        let mut t = Touching::default();
        let at = |v| vec![Live::ControlIn { group: 0, sound: 0, kind: MidiKind::ChannelPressure(v), thru: true, record: true }];
        t.hit(1, (0, 0, 60), &r);
        t.hit(2, (0, 0, 64), &r);
        assert_eq!(t.pressure(1, Some((0, 0, 60)), 1.0, &r), at(127));
        assert_eq!(t.pressure(2, Some((0, 0, 64)), 0.5, &r), vec![], "lighter: nothing new");
        let half = touch_value(0.5, &r.touches[0]);
        assert_eq!(t.release(1, (0, 0, 60), &r), at(half), "pad 2's");
        assert_eq!(t.release(2, (0, 0, 64), &r), at(13), "Min (10 %)");
    }

    #[test]
    fn poly_at_goes_on_each_pads_pitch() {
        let r = keys(mode(TouchMode::PolyAt));
        let mut t = Touching::default();
        t.hit(1, (0, 0, 60), &r);
        t.hit(2, (0, 0, 64), &r);
        let pitch = |l: Vec<Live>| match l.as_slice() {
            [Live::Expression { pitch, .. }] => *pitch,
            other => panic!("{other:?}"),
        };
        assert_eq!(pitch(t.pressure(1, Some((0, 0, 60)), 0.7, &r)), 60);
        assert_eq!(pitch(t.pressure(2, Some((0, 0, 64)), 0.7, &r)), 64);
        assert_eq!(t.release(1, (0, 0, 60), &r), vec![], "Poly AT: the note ends, nothing more");
    }

    /// A pad held on sound 0 while the route moves on: its pressure follows sound 0's
    /// mode as it is now, and its release sends sound 0's Min.
    #[test]
    fn a_held_pad_keeps_its_sound() {
        let mut r = route(PadTouch { mode: TouchMode::Aftertouch, min: 20, ..PadTouch::default() });
        r.touches[3] = PadTouch { mode: TouchMode::Cc, min: 50, ..PadTouch::default() };
        let mut t = Touching::default();
        t.hit(1, (0, 0, 60), &r);
        // Sound 0 becomes CC 74 (Min 20) while held.
        r.touches[0] = PadTouch { mode: TouchMode::Cc, cc: 74, min: 20, ..PadTouch::default() };
        let cc =
            |v| vec![Live::ControlIn { group: 0, sound: 0, kind: MidiKind::Controller { number: 74, value: v }, thru: true, record: true }];
        assert_eq!(t.pressure(1, Some((0, 0, 60)), 1.0, &r), cc(127));
        // The route goes to another group: sound 0's settings as last seen.
        r.group = 1;
        assert_eq!(t.release(1, (0, 0, 60), &r), cc(25));
    }

    fn control(kind: MidiKind) -> Live {
        Live::ControlIn { group: 0, sound: 0, kind, thru: true, record: true }
    }

    fn cc(number: u8, value: u8) -> Live {
        control(MidiKind::Controller { number, value })
    }

    /// A held pad's sound goes from CC to Velocity: letting go still takes the CC back
    /// to Min, so the synth isn't left bent.
    #[test]
    fn leaving_cc_mode_while_held_returns_the_cc_to_min() {
        let mut r = route(PadTouch { mode: TouchMode::Cc, cc: 74, min: 20, ..PadTouch::default() });
        let mut t = Touching::default();
        t.hit(1, (0, 0, 60), &r);
        assert_eq!(t.pressure(1, Some((0, 0, 60)), 1.0, &r), vec![cc(74, 127)]);
        r.touches[0].mode = TouchMode::Velocity;
        assert_eq!(t.release(1, (0, 0, 60), &r), vec![cc(74, 25)]);
        // And pressed again after the change, it's the new mode's from the start.
        r.touches[0].mode = TouchMode::Cc;
        t.hit(1, (0, 0, 60), &r);
        assert_eq!(t.pressure(1, Some((0, 0, 60)), 1.0, &r), vec![cc(74, 127)]);
    }

    /// The CC number changes while a pad is held: the old CC goes back to Min as the
    /// new one takes over, even at the same value.
    #[test]
    fn a_new_cc_number_frees_the_old_one() {
        let mut r = route(PadTouch { mode: TouchMode::Cc, cc: 1, ..PadTouch::default() });
        let mut t = Touching::default();
        t.hit(1, (0, 0, 60), &r);
        assert_eq!(t.pressure(1, Some((0, 0, 60)), 1.0, &r), vec![cc(1, 127)]);
        r.touches[0].cc = 74;
        assert_eq!(t.pressure(1, Some((0, 0, 60)), 1.0, &r), vec![cc(1, 0), cc(74, 127)]);
        assert_eq!(t.release(1, (0, 0, 60), &r), vec![cc(74, 0)]);
    }

    /// From Aftertouch to CC while held: aftertouch goes back to Min, the CC starts.
    #[test]
    fn aftertouch_to_cc_while_held_frees_the_aftertouch() {
        let mut r = route(PadTouch { mode: TouchMode::Aftertouch, ..PadTouch::default() });
        let mut t = Touching::default();
        t.hit(1, (0, 0, 60), &r);
        assert_eq!(t.pressure(1, Some((0, 0, 60)), 1.0, &r), vec![control(MidiKind::ChannelPressure(127))]);
        r.touches[0] = PadTouch { mode: TouchMode::Cc, cc: 74, ..PadTouch::default() };
        let got = t.pressure(1, Some((0, 0, 60)), 0.5, &r);
        assert_eq!(got, vec![control(MidiKind::ChannelPressure(0)), cc(74, 64)]);
    }

    /// From CC to Poly AT while held: the CC goes back to Min, and the note gets its
    /// pressure as poly aftertouch.
    #[test]
    fn cc_to_poly_at_while_held_frees_the_cc() {
        let mut r = route(PadTouch { mode: TouchMode::Cc, cc: 74, ..PadTouch::default() });
        let mut t = Touching::default();
        t.hit(1, (0, 0, 60), &r);
        t.pressure(1, Some((0, 0, 60)), 1.0, &r);
        r.touches[0].mode = TouchMode::PolyAt;
        let got = t.pressure(1, Some((0, 0, 60)), 1.0, &r);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0], cc(74, 0));
        assert!(matches!(got[1], Live::Expression { pitch: 60, .. }), "{got:?}");
        assert_eq!(t.release(1, (0, 0, 60), &r), vec![]);
    }

    /// Hits queued while no engine ran play at the velocity they were given (Fixed
    /// Velocity in the pressure modes), each at its own pitch; releases play nothing.
    #[test]
    fn pending_hits_keep_their_velocity() {
        let r = route(mode(TouchMode::PolyAt));
        let fixed = Touching::hit_velocity(&r, 0, 0, 30);
        let notes = pending_notes(&[(0, 0, 60, fixed), (0, 0, 60, 0), (0, 1, 64, 45)]);
        let on = |sound, pitch, velocity| Live::NoteOn { group: 0, sound, pitch, velocity, length: deckard_model::PPQ };
        assert_eq!(notes, vec![on(0, 60, 90), on(1, 64, 45)]);
    }

    #[test]
    fn repeated_values_send_nothing() {
        for m in TouchMode::ALL {
            let r = route(mode(m));
            let mut t = Touching::default();
            t.hit(1, (0, 0, 60), &r);
            assert_eq!(t.pressure(1, Some((0, 0, 60)), 0.6, &r).len(), 1, "{m:?}");
            assert_eq!(t.pressure(1, Some((0, 0, 60)), 0.6, &r), vec![], "{m:?}");
        }
    }

    #[test]
    fn pressure_on_a_silent_pad_is_dropped() {
        let r = route(mode(TouchMode::PolyAt));
        assert_eq!(Touching::default().pressure(1, None, 0.5, &r), vec![]);
    }

    /// A pad let go stops the note it started at the engine: the releases go straight
    /// there, not through the pad-touch filter (the monitor's own row is `Pad::Off`).
    #[test]
    fn a_pad_release_reaches_the_engine() {
        let r = route(PadTouch::default());
        let mut pads = Pads::new();
        pads.hit(&r, 1, 100);
        let (mut engine, mut rows) = (Vec::new(), Vec::new());
        forward_pad_lives(pads.release(1), |l| engine.push(l), |ev| rows.push(ev));
        assert_eq!(engine, vec![Live::NoteRelease { group: 0, sound: 0, pitch: PAD_PITCH }], "the note the pad started, stopped");
        assert_eq!(rows, vec![], "no pad-touch row: a note's row is its own `Pad::Off`");
    }

    /// A chord's pads stop every note they started, not just the first.
    #[test]
    fn a_chord_release_stops_every_note_it_started() {
        let mut chords = [[128; 16]; 16];
        chords[0][..3].copy_from_slice(&[60, 64, 67]);
        let r = PadRoute { chords: Some(chords), ..keys(PadTouch::default()) };
        let mut pads = Pads::new();
        assert_eq!(pads.hit(&r, 1, 100).len(), 3, "the whole chord sounds");
        let mut engine = Vec::new();
        forward_pad_lives(pads.release(1), |l| engine.push(l), |_| {});
        let release = |pitch| Live::NoteRelease { group: 0, sound: 0, pitch };
        assert_eq!(engine, vec![release(64), release(67), release(60)], "the chord's other notes, then the first");
    }
}
