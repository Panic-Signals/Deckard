//! The Maschine MK3 over USB (`deckard-mk3`): a thread reads the controller. Pads
//! play the focused group's sounds straight from that thread (stamped, like MIDI in,
//! so they don't wait for a frame); every event also goes to the app, which hands it to
//! the controller scripts.

use deckard_engine::Live;
use deckard_engine::output::Sender;
use deckard_mk3::Event;
use deckard_model::pad_touch::{PadTouch, TouchMode};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    fn touch(&self, group: usize, sound: usize) -> Option<PadTouch> {
        (group == self.group).then(|| self.touches.get(sound).copied()).flatten()
    }
}

/// Each velocity scaling's curve, Soft 3 … Hard 3: (m, n) in [`pad_velocity`].
const VELOCITY_CURVES: [(f64, f64); 7] = [(1.71, 2.5), (1.50, 2.5), (1.26, 2.5), (1.0, 1.0), (0.82, 4.0), (0.60, 4.0), (0.32, 4.0)];

/// A pad's pressure (0..1, the MK3's report as the pad was hit) as velocity (0..1), by
/// the velocity scaling (-3 Soft 3 … 3 Hard 3), as Maschine plays it. With u = √pressure,
/// the velocity is min(1, m·u + (1 − m)·uⁿ): Linear is u itself, and m is the slope at
/// the light end (Soft above 1, Hard below), n how late it bends to full. Measured
/// against Maschine 3 on an MK3, within a velocity step at every setting
/// (docs/mk3-usb.md).
pub fn pad_velocity(pressure: f64, scaling: i8) -> f64 {
    let u = pressure.clamp(0.0, 1.0).sqrt();
    let (m, n) = VELOCITY_CURVES[(scaling.clamp(-3, 3) + 3) as usize];
    (m * u + (1.0 - m) * u.powf(n)).clamp(0.0, 1.0)
}

/// A held pad's pressure (0..1) shaped by a curve (0..1, 0.5 straight): above 0.5 light
/// presses count for more (an exponent down to 1/4), below less (up to 4).
pub fn pressure_shape(pressure: f64, curve: f64) -> f64 {
    deckard_model::pad_touch::shape(pressure, curve)
}

/// What a held pad's pressure (0..1) sends by a sound's Pad Touch (0–127): from Min to
/// Max, along its curve.
pub fn touch_value(pressure: f64, t: &PadTouch) -> u8 {
    let pct = |n: u8| f64::from(n.min(100)) / 100.0;
    let (lo, hi) = (pct(t.min), pct(t.max).max(pct(t.min)));
    ((lo + (hi - lo) * pressure_shape(pressure, pct(t.curve))) * 127.0).round() as u8
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

/// A held pad, as [`Touching`] keeps it: the sound it plays, that sound's Pad Touch as
/// last seen, and the value it last sent (or counts for, in the channel-wide modes).
#[derive(Clone, Copy)]
struct Held {
    sound: (usize, usize),
    touch: PadTouch,
    value: Option<u8>,
}

/// What a channel-wide mode sends on: channel aftertouch, or a CC by number.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Wide {
    Aftertouch,
    Cc(u8),
}

impl Wide {
    fn of(t: &PadTouch) -> Option<Wide> {
        match t.mode {
            TouchMode::Aftertouch => Some(Wide::Aftertouch),
            TouchMode::Cc => Some(Wide::Cc(t.cc.min(127))),
            _ => None,
        }
    }
}

/// The pads' pressure, turned into what each sound's Pad Touch sends: Note Repeat's
/// velocity (Velocity), poly aftertouch on the pad's note (Poly AT), or channel
/// aftertouch or a CC for the sound (Aftertouch, CC: the hardest held pad counts, and
/// letting go of the last sends Min). Only changes are sent. When a held pad's sound
/// changes mode or CC number, what it sent before goes back to Min once no held pad
/// sends it.
#[derive(Default)]
pub struct Touching {
    /// By pad (0–15).
    held: [Option<Held>; 16],
    /// The value each sound last sent on each channel-wide target.
    sent: std::collections::HashMap<((usize, usize), Wide), u8>,
}

impl Touching {
    /// The velocity a hit plays at: in the pressure modes the song's Fixed Velocity;
    /// else the pads' Fixed Velocity if on, or the hit's own (`from_pressure`).
    pub fn hit_velocity(route: &PadRoute, group: usize, sound: usize, from_pressure: i64) -> i64 {
        match route.touch(group, sound).map(|t| t.mode) {
            Some(TouchMode::PolyAt | TouchMode::Aftertouch | TouchMode::Cc) => route.fixed.clamp(1, 127),
            _ => route.velocity.unwrap_or(from_pressure),
        }
    }

    /// Pad `pad` (1–16) started `slot` (group, sound, pitch).
    pub fn hit(&mut self, pad: usize, slot: (usize, usize, i64), route: &PadRoute) {
        let (g, s, _) = slot;
        if let Some(h) = pad.checked_sub(1).and_then(|i| self.held.get_mut(i)) {
            *h = Some(Held { sound: (g, s), touch: route.touch(g, s).unwrap_or_default(), value: None });
        }
    }

    /// Pad `pad` (1–16), sounding `slot`, pressed at `pressure` (0..1).
    pub fn pressure(&mut self, pad: usize, slot: Option<(usize, usize, i64)>, pressure: f64, route: &PadRoute) -> Vec<Live> {
        let (Some((g, s, pitch)), Some(i)) = (slot, pad.checked_sub(1).filter(|i| *i < 16)) else { return Vec::new() };
        let Some(mut h) = self.held[i] else { return Vec::new() };
        let mut out = Vec::new();
        let now = route.touch(g, s).unwrap_or(h.touch);
        if (now.mode, Wide::of(&now)) != (h.touch.mode, Wide::of(&h.touch)) {
            // A new mode or CC: this pad starts over there, and stops counting where it was.
            let was = h.touch;
            self.held[i] = Some(Held { touch: now, value: None, ..h });
            if let Some(w) = Wide::of(&was) {
                out.extend(self.channel_wide((g, s), w, was));
            }
            h = self.held[i].unwrap();
        }
        h.touch = now;
        let (value, live) = match h.touch.mode {
            TouchMode::Velocity => {
                let v = ((pad_velocity(pressure, route.scaling) * 127.0).round() as i64).clamp(1, 127);
                // (Fixed Velocity: the repeats stay at the hit's, however the pad is pressed.)
                let live = route.velocity.is_none().then_some(Live::RepeatPressure { group: g, sound: s, pitch, velocity: v });
                (v as u8, live)
            }
            TouchMode::PolyAt => {
                let v = touch_value(pressure, &h.touch);
                let kind = deckard_model::ExprKind::Dim(deckard_model::Dim::Pressure);
                (v, Some(Live::Expression { group: g, sound: s, pitch, kind, value: f32::from(v) / 127.0 }))
            }
            TouchMode::Aftertouch | TouchMode::Cc => (touch_value(pressure, &h.touch), None),
        };
        let changed = h.value != Some(value);
        h.value = Some(value);
        self.held[i] = Some(h);
        match (live, Wide::of(&h.touch)) {
            (Some(l), _) if changed => out.push(l),
            (None, Some(w)) => out.extend(self.channel_wide((g, s), w, h.touch)),
            _ => {}
        }
        out
    }

    /// Pad `pad` (1–16), which sounded `slot`, let go: what it was sending, by its
    /// sound's settings as last seen.
    pub fn release(&mut self, pad: usize, _slot: (usize, usize, i64), _route: &PadRoute) -> Vec<Live> {
        let Some(h) = pad.checked_sub(1).and_then(|i| self.held.get_mut(i)).and_then(Option::take) else { return Vec::new() };
        match Wide::of(&h.touch) {
            Some(w) => self.channel_wide(h.sound, w, h.touch),
            None => Vec::new(),
        }
    }

    /// `sound`'s value on `w` now: its hardest held pad's there, or, with none held,
    /// Min (`t`'s) if something else was sent, and then forgotten.
    fn channel_wide(&mut self, sound: (usize, usize), w: Wide, t: PadTouch) -> Vec<Live> {
        let key = (sound, w);
        let feeding = self.held.iter().flatten().filter(|h| h.sound == sound && Wide::of(&h.touch) == Some(w));
        let v = match (feeding.clone().filter_map(|h| h.value).max(), feeding.count() > 0) {
            (Some(v), _) => {
                if self.sent.insert(key, v) == Some(v) {
                    return Vec::new();
                }
                v
            }
            (None, true) => return Vec::new(),
            (None, false) => {
                let min = touch_value(0.0, &PadTouch { curve: 50, ..t });
                match self.sent.remove(&key) {
                    Some(last) if last != min => min,
                    _ => return Vec::new(),
                }
            }
        };
        let kind = match w {
            Wide::Cc(number) => deckard_model::MidiKind::Controller { number, value: v },
            Wide::Aftertouch => deckard_model::MidiKind::ChannelPressure(v),
        };
        vec![Live::ControlIn { group: sound.0, sound: sound.1, kind, thru: true, record: true }]
    }
}

#[derive(Clone)]
pub enum ScreenFrame {
    Picture(Arc<deckard_surface::Picture>),
    Pixels(Arc<Vec<[u8; 4]>>),
}

impl ScreenFrame {
    #[cfg(test)]
    pub fn into_pixels(self) -> Vec<[u8; 4]> {
        let mut out = Vec::new();
        self.write_pixels(&mut out);
        out
    }

    pub fn write_pixels(self, out: &mut Vec<[u8; 4]>) {
        match self {
            ScreenFrame::Picture(p) => match Arc::try_unwrap(p) {
                Ok(mut pic) => pic.rasterize_into(out),
                Err(shared) => {
                    let mut pic = (*shared).clone();
                    pic.rasterize_into(out);
                }
            },
            ScreenFrame::Pixels(px) => {
                out.clear();
                out.extend_from_slice(&px);
            }
        }
    }
}

#[derive(Default)]
struct Shared {
    events: Vec<Event>,
    /// Pad hits that came while no engine played: (group, sound, pitch, velocity; 0 for
    /// a release).
    pending: Vec<(usize, usize, i64, i64)>,
    output: Option<Sender>,
    route: Option<PadRoute>,
    status: String,
    repaint: Option<eframe::egui::Context>,
    /// What the LEDs should show.
    leds: Option<deckard_mk3::leds::Leds>,
    leds_seq: u64,
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
    /// DECKARD is closing: dark LEDs, a black frame on the displays, then stop
    /// (`shut_down`). The frame is black so a still picture is not left to burn in.
    closing: bool,
    goodbye: Option<Arc<Vec<[u8; 4]>>>,
    /// Whether the controller and the displays are open, and whether each has been left
    /// dark since `closing`.
    connected: bool,
    displays_connected: bool,
    /// The first look for the controller is over (found or not).
    probed: bool,
    leds_closed: bool,
    displays_closed: bool,
    /// The MIDI Monitor, shown what the pads play.
    monitor: Option<crate::midi_monitor::Monitor>,
}

pub struct Mk3Io {
    shared: Arc<Mutex<Shared>>,
    display_wake: Arc<std::sync::Condvar>,
}

/// The pitch pads play: a sound's base key (Maschine's default, C3).
const PAD_PITCH: i64 = 60;

/// Both displays side by side, every pixel black. Closing sends this so the panels
/// are not left showing a still picture.
fn black_displays() -> Vec<[u8; 4]> {
    vec![[0, 0, 0, 255]; deckard_mk3::display::WIDTH * 2 * deckard_mk3::display::HEIGHT]
}

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

    /// Leave the controller dark: LEDs off and both displays black. Waits (up to a
    /// second) for both threads to do it.
    pub fn shut_down(&self) {
        {
            let mut s = self.shared.lock().unwrap();
            s.goodbye = Some(Arc::new(black_displays()));
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

    /// An MK3 handle with no USB threads, for tests that only check the close request.
    #[cfg(test)]
    pub(crate) fn detached() -> Mk3Io {
        Mk3Io { shared: Arc::default(), display_wake: Arc::new(std::sync::Condvar::new()) }
    }

    /// The frame `shut_down` queued, if it has.
    #[cfg(test)]
    pub(crate) fn queued_frame(&self) -> Option<Vec<[u8; 4]>> {
        self.shared.lock().unwrap().goodbye.as_deref().cloned()
    }

    /// Set the displays' backlight (0..=100), now if it changed and whenever an MK3
    /// connects or its displays reopen.
    pub fn set_display_brightness(&self, level: u8) {
        let mut s = self.shared.lock().unwrap();
        let level = Some(level.min(100));
        if s.display_brightness != level {
            s.display_brightness = level;
            s.brightness_stamp = s.brightness_stamp.wrapping_add(1);
        }
    }

    /// Set the LEDs' global level (0..=100 %), now if it changed and whenever an MK3
    /// connects.
    pub fn set_led_brightness(&self, level: u8) {
        let mut s = self.shared.lock().unwrap();
        let level = Some(level.min(100));
        if s.led_brightness != level {
            s.led_brightness = level;
            s.led_stamp = s.led_stamp.wrapping_add(1);
        }
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
        let mut s = self.shared.lock().unwrap();
        s.leds = Some(leds);
        s.leds_seq = s.leds_seq.wrapping_add(1);
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

    /// Whether the controller is there, once the first look for it is over: it's looked
    /// for on a thread of its own, so asking as that starts always said no. Waits at
    /// most `patience` for that first look.
    pub fn found(&self, patience: Duration) -> bool {
        let until = std::time::Instant::now() + patience;
        loop {
            {
                let s = self.shared.lock().unwrap();
                if s.probed || s.connected {
                    return s.connected;
                }
            }
            if std::time::Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
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
        std::mem::take(&mut self.shared.lock().unwrap().pending)
    }
}

fn run(shared: &Mutex<Shared>) {
    loop {
        let mut dev = match deckard_mk3::Device::open() {
            Ok(d) => d,
            Err(e) => {
                let mut s = shared.lock().unwrap();
                s.status = e;
                s.probed = true;
                drop(s);
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        // Clear the preceding host/session before waiting for the first UI state.
        if let Err(e) = dev.write_leds(&deckard_mk3::leds::Leds::default()) {
            let mut s = shared.lock().unwrap();
            s.status = format!("Maschine MK3: {e}");
            s.probed = true;
            drop(s);
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        let _polling = crate::controller_timing::Polling::start();
        {
            let mut s = shared.lock().unwrap();
            s.status = format!("{} connected", dev.model().name);
            s.connected = true;
            s.probed = true;
        }
        let mut events = Vec::new();
        // Shown on the LEDs: none yet, so the first state is sent.
        let mut shown = None;
        let mut checked_leds_seq = 0u64;
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
        // What each pad started, so it stops the same note however the route changed
        // (a chord's first note; its others in `chord_rest`).
        let mut sounding: [Option<(usize, usize, i64)>; 16] = [None; 16];
        let mut chord_rest: [Vec<(usize, usize, i64)>; 16] = Default::default();
        // What the held pads' pressure sends (by each sound's Pad Touch).
        let mut touching = Touching::default();
        loop {
            if shared.lock().unwrap().closing {
                let _ = dev.write_leds(&deckard_mk3::leds::Leds::default());
                shared.lock().unwrap().leds_closed = true;
                return;
            }
            if let Err(e) = dev.poll(1, &mut events) {
                let mut s = shared.lock().unwrap();
                s.status = format!("Maschine MK3: {e}");
                s.connected = false;
                break;
            }
            // Dispatch arrived pad/button events immediately before any LED/feature writes.
            if !events.is_empty() {
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
                let monitor = s.monitor.clone().filter(crate::midi_monitor::Monitor::is_open);
                let shown_mon = |pad: usize, slot: Option<(usize, usize, i64)>, ev| {
                    if let Some(m) = &monitor {
                        m.pad(pad, slot.map(|x| x.2), ev);
                    }
                };
                // What a pad's pressure (or release) sends, to the engine and the monitor.
                let send = |pad: usize, slot: Option<(usize, usize, i64)>, lives: Vec<Live>| {
                    use crate::midi_monitor::Pad;
                    for l in lives {
                        let ev = match l {
                            Live::RepeatPressure { velocity, .. } => Pad::Pressure(velocity.clamp(0, 127) as u8),
                            Live::Expression { value, .. } => Pad::PolyAt((value * 127.0).round() as u8),
                            Live::ControlIn { kind, .. } => Pad::Control(kind),
                            _ => continue,
                        };
                        if let Some(o) = &out {
                            o.live(l);
                        }
                        shown_mon(pad, slot, ev);
                    }
                };
                for e in &events {
                    if let Event::PadPressure(p, v) = *e {
                        let slot = p.checked_sub(1).and_then(|i| sounding.get(i)).copied().flatten();
                        if out.is_some() {
                            send(p, slot, touching.pressure(p, slot, v, &route));
                        }
                        continue;
                    }
                    let (pad, from_pressure) = match *e {
                        Event::PadOn(p, v) => (p, ((pad_velocity(v, route.scaling) * 127.0).round() as i64).clamp(1, 127)),
                        Event::PadOff(p) => (p, 0),
                        _ => continue,
                    };
                    let slot = pad.checked_sub(1).and_then(|i| sounding.get_mut(i));
                    let rest = pad.checked_sub(1).and_then(|i| chord_rest.get_mut(i));
                    if from_pressure == 0 {
                        let rest = rest.map(std::mem::take).unwrap_or_default();
                        if let (Some(o), Some(Some((group, sound, pitch)))) = (&out, slot.map(Option::take)) {
                            for &(group, sound, pitch) in &rest {
                                o.live(Live::NoteRelease { group, sound, pitch });
                            }
                            o.live(Live::NoteRelease { group, sound, pitch });
                            shown_mon(pad, Some((group, sound, pitch)), crate::midi_monitor::Pad::Off);
                            send(pad, Some((group, sound, pitch)), touching.release(pad, (group, sound, pitch), &route));
                        } else if out.is_none() && route.play && route.keys.is_none() {
                            s.pending.push((route.group, pad - 1, PAD_PITCH, 0));
                        }
                        continue;
                    }
                    if !route.play {
                        continue;
                    }
                    let notes = pad_notes(&route, pad);
                    let Some(&(sound, pitch)) = notes.first() else { continue };
                    let group = route.group;
                    let velocity = Touching::hit_velocity(&route, group, sound, from_pressure);
                    match &out {
                        Some(o) => {
                            // Held until the pad is let go (Note Repeat / the Arp take held notes).
                            o.live(Live::NoteOn { group, sound, pitch, velocity, length: i64::MAX });
                            shown_mon(pad, Some((group, sound, pitch)), crate::midi_monitor::Pad::On(velocity));
                            touching.hit(pad, (group, sound, pitch), &route);
                            if let Some(slot) = slot {
                                *slot = Some((group, sound, pitch));
                            }
                            // (A chord's other notes sound with it; its pressure goes to the first.)
                            let others: Vec<_> = notes[1..].iter().map(|&(sound, pitch)| (group, sound, pitch)).collect();
                            for &(group, sound, pitch) in &others {
                                o.live(Live::NoteOn { group, sound, pitch, velocity, length: i64::MAX });
                            }
                            if let Some(rest) = rest {
                                *rest = others;
                            }
                        }
                        None => s.pending.extend(notes.iter().map(|&(sound, pitch)| (group, sound, pitch, velocity))),
                    }
                }
                s.events.append(&mut events);
                if let Some(ctx) = &s.repaint {
                    ctx.request_repaint();
                }
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
            let leds = {
                let s = shared.lock().unwrap();
                if s.leds_seq != checked_leds_seq {
                    checked_leds_seq = s.leds_seq;
                    s.leds.clone()
                } else {
                    None
                }
            };
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
    use deckard_mk3::display::{Displays, HEIGHT, WIDTH, rgb565_dithered, rgb565_rounded};
    // (A picture's pixels, marked by the surface, rounded rather than dithered.)
    let to565 = |x: usize, y: usize, [r, g, b, a]: [u8; 4]| {
        if a == deckard_surface::PICTURE_ALPHA { rgb565_rounded(r, g, b) } else { rgb565_dithered(x, y, r, g, b) }
    };
    let mut left = vec![0u16; WIDTH * HEIGHT];
    let mut right = vec![0u16; WIDTH * HEIGHT];
    let mut px: Vec<[u8; 4]> = Vec::with_capacity(WIDTH * 2 * HEIGHT);
    let mut last_px: Vec<[u8; 4]> = Vec::with_capacity(WIDTH * 2 * HEIGHT);
    // Fill `out` with one 480×272 half of a 960-wide RGBA picture, dithered to the
    // displays' 5-6-5 levels (the one place the picture loses its low bits).
    let fill_half = |px: &[[u8; 4]], x0: usize, out: &mut [u16]| {
        let stride = WIDTH * 2;
        if px.len() >= stride * HEIGHT && x0 + WIDTH <= stride {
            for y in 0..HEIGHT {
                let src = &px[y * stride + x0..y * stride + x0 + WIDTH];
                let dst = &mut out[y * WIDTH..(y + 1) * WIDTH];
                for (x, (o, &p)) in dst.iter_mut().zip(src).enumerate() {
                    *o = to565(x, y, p);
                }
            }
        } else {
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    out[y * WIDTH + x] = to565(x, y, px.get(y * stride + x0 + x).copied().unwrap_or([0, 0, 0, 255]));
                }
            }
        }
    };
    let half_changed = |a: &[[u8; 4]], b: &[[u8; 4]], x0: usize| -> bool {
        let stride = WIDTH * 2;
        if a.len() != b.len() || a.len() < stride * HEIGHT {
            return true;
        }
        (0..HEIGHT).any(|y| a[y * stride + x0..y * stride + x0 + WIDTH] != b[y * stride + x0..y * stride + x0 + WIDTH])
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
        last_px.clear();
        let mut previous: Option<deckard_surface::Picture> = None;
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
                let gpx = goodbye.as_deref().map_or(&[][..], Vec::as_slice);
                fill_half(gpx, 0, &mut left);
                fill_half(gpx, WIDTH, &mut right);
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
                let region = match frame {
                    ScreenFrame::Picture(picture) => {
                        let mut picture = Arc::try_unwrap(picture).unwrap_or_else(|shared| (*shared).clone());
                        let region = previous.as_ref().and_then(|old| picture.rasterize_spectrum_update(old, &mut px));
                        if region.is_none() {
                            picture.rasterize_into(&mut px);
                        }
                        previous = Some(picture);
                        region
                    }
                    ScreenFrame::Pixels(pixels) => {
                        previous = None;
                        px.clear();
                        px.extend_from_slice(&pixels);
                        None
                    }
                };
                let mut res = Ok(());
                if let Some((x0, y0, x1, y1)) = region {
                    for (panel, rgb) in [&mut left, &mut right].into_iter().enumerate() {
                        let start = x0.max(panel * WIDTH);
                        let end = x1.min((panel + 1) * WIDTH);
                        if start >= end {
                            continue;
                        }
                        let mut changed = false;
                        for y in y0..y1 {
                            let wide_row = y * (2 * WIDTH);
                            for x in start..end {
                                let pixel = px[wide_row + x];
                                if last_px[wide_row + x] != pixel {
                                    last_px[wide_row + x] = pixel;
                                    let x = x - panel * WIDTH;
                                    let dst = &mut rgb[y * WIDTH + x];
                                    let value = to565(x, y, pixel);
                                    changed |= *dst != value;
                                    *dst = value;
                                }
                            }
                        }
                        if changed {
                            res = d.show(panel, rgb);
                            if res.is_err() {
                                break;
                            }
                        }
                    }
                } else {
                    let left_dirty = half_changed(&px, &last_px, 0);
                    let right_dirty = half_changed(&px, &last_px, WIDTH);
                    if left_dirty {
                        fill_half(&px, 0, &mut left);
                        res = d.show(0, &left);
                    }
                    if res.is_ok() && right_dirty {
                        fill_half(&px, WIDTH, &mut right);
                        res = d.show(1, &right);
                    }
                    std::mem::swap(&mut last_px, &mut px);
                }
                if let Err(e) = res {
                    let mut s = shared.lock().unwrap();
                    s.display_status = e;
                    s.displays_connected = false;
                    break;
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

    /// The splash asks whether the MK3 is there as the look for it starts: the answer
    /// waits for that first look, rather than saying no at once.
    #[test]
    fn found_waits_for_the_first_look() {
        let io = Mk3Io { shared: Arc::default(), display_wake: Arc::new(std::sync::Condvar::new()) };
        assert!(!io.found(Duration::from_millis(20)), "never looked: no, after the wait");
        let s = io.shared.clone();
        let looking = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            let mut s = s.lock().unwrap();
            s.connected = true;
            s.probed = true;
        });
        assert!(io.found(Duration::from_secs(5)), "found once the look is over");
        looking.join().unwrap();
        let io = Mk3Io { shared: Arc::default(), display_wake: Arc::new(std::sync::Condvar::new()) };
        io.shared.lock().unwrap().probed = true;
        let t = std::time::Instant::now();
        assert!(!io.found(Duration::from_secs(5)) && t.elapsed() < Duration::from_secs(1), "looked, not there: no at once");
    }

    /// Closing queues a black frame for both displays. No controller is opened.
    #[test]
    fn closing_queues_a_black_frame() {
        let io = Mk3Io { shared: Arc::default(), display_wake: Arc::new(std::sync::Condvar::new()) };
        io.shut_down();
        let s = io.shared.lock().unwrap();
        assert!(s.closing);
        let px = s.goodbye.as_deref().expect("a frame is queued");
        assert_eq!(px.len(), black_displays().len());
        assert!(px.iter().all(|p| *p == [0, 0, 0, 255]), "every pixel is black");
    }

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

    /// Fixed Velocity: a held pad's pressure doesn't change what Note Repeat plays at
    /// (it dropped as the pad was let go); without it, pressure sets the repeats'.
    #[test]
    fn fixed_velocity_holds_note_repeat_against_pad_pressure() {
        let slot = (0, 0, 60);
        let fixed = PadRoute { velocity: Some(100), ..route(mode(TouchMode::Velocity)) };
        let mut t = Touching::default();
        t.hit(1, slot, &fixed);
        assert!(t.pressure(1, Some(slot), 0.9, &fixed).is_empty(), "pressed harder");
        assert!(t.pressure(1, Some(slot), 0.05, &fixed).is_empty(), "being let go");
        let free = route(mode(TouchMode::Velocity));
        let mut t = Touching::default();
        t.hit(1, slot, &free);
        assert!(matches!(t.pressure(1, Some(slot), 0.05, &free).as_slice(), [Live::RepeatPressure { .. }]));
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

    #[test]
    fn unchanged_brightness_levels_do_not_bump_feature_stamps() {
        let io = Mk3Io { shared: Arc::new(Mutex::new(Shared::default())), display_wake: Arc::new(std::sync::Condvar::new()) };
        io.set_display_brightness(80);
        io.set_led_brightness(75);
        assert_eq!(io.shared.lock().unwrap().brightness_stamp, 1);
        assert_eq!(io.shared.lock().unwrap().led_stamp, 1);
        io.set_display_brightness(80);
        io.set_led_brightness(75);
        assert_eq!(io.shared.lock().unwrap().brightness_stamp, 1);
        assert_eq!(io.shared.lock().unwrap().led_stamp, 1);
        io.set_display_brightness(120);
        io.set_led_brightness(60);
        assert_eq!(io.shared.lock().unwrap().brightness_stamp, 2);
        assert_eq!(io.shared.lock().unwrap().led_stamp, 2);
        io.set_display_brightness(100);
        assert_eq!(io.shared.lock().unwrap().brightness_stamp, 2);
    }
}
