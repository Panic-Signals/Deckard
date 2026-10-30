//! The Maschine MK3 over USB (`deckard-mk3`): a thread reads the controller. Pads
//! play the focused group's sounds straight from that thread (stamped, like MIDI in,
//! so they don't wait for a frame); every event also goes to the app, which hands it to
//! the controller scripts.

use deckard_engine::Live;
use deckard_engine::output::Sender;
use deckard_mk3::Event;
use deckard_model::pad_touch::{PadTouch, TouchMode};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    /// Pad sensitivity, 0..1, and velocity scaling, -3 (Soft 3) … 3 (Hard 3) (see
    /// [`pad_velocity`]).
    pub sensitivity: f64,
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

/// A pad's pressure (0..1, from the MK3) as velocity (0..1), by the sensitivity (0..1): at
/// 0 as it comes; higher, light presses count for more (a gain up to ×4, and a curve that
/// lifts the low end), so a gentle press isn't too quiet to hear. Then the velocity
/// scaling (-3 … 3) bends it: Soft lifts what's short of full, Hard holds it back (a
/// power of 1.5 a step).
pub fn pad_velocity(pressure: f64, sensitivity: f64, scaling: i8) -> f64 {
    let s = sensitivity.clamp(0.0, 1.0);
    let v = (pressure.max(0.0) * (1.0 + 3.0 * s)).min(1.0).powf(1.0 - 0.5 * s);
    v.powf(1.5f64.powi(i32::from(scaling.clamp(-3, 3))))
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
                let v = ((pad_velocity(pressure, route.sensitivity, route.scaling) * 127.0).round() as i64).clamp(1, 127);
                (v as u8, Some(Live::RepeatPressure { group: g, sound: s, pitch, velocity: v }))
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
    /// What the displays should show: both side by side, 960 × 272 RGBA, and its number.
    screens: Option<(u64, Arc<Vec<[u8; 4]>>)>,
    display_status: String,
    /// The displays' backlight, 0..=100 (`set_display_brightness`).
    display_brightness: Option<u8>,
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

pub struct Mk3Io {
    shared: Arc<Mutex<Shared>>,
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
        let s = shared.clone();
        std::thread::Builder::new().name("DECKARD MK3".into()).spawn(move || run(&s)).ok();
        let s = shared.clone();
        std::thread::Builder::new().name("DECKARD MK3 displays".into()).spawn(move || run_displays(&s)).ok();
        Mk3Io { shared }
    }

    /// Leave the controller dark but for `picture` on the displays (both, 960 × 272
    /// RGBA): LEDs off. Waits (up to a second) for both threads to do it.
    pub fn shut_down(&self, picture: Vec<[u8; 4]>) {
        {
            let mut s = self.shared.lock().unwrap();
            s.goodbye = Some(Arc::new(picture));
            s.closing = true;
        }
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
        self.shared.lock().unwrap().display_brightness = Some(level.min(100));
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

    /// Show these on the displays (both side by side, 960 × 272); `n` changes when
    /// the picture does.
    pub fn set_screens(&self, n: u64, px: &[[u8; 4]]) {
        let mut s = self.shared.lock().unwrap();
        if s.screens.as_ref().is_none_or(|(m, _)| *m != n) {
            s.screens = Some((n, Arc::new(px.to_vec())));
        }
    }

    /// What the controller did since the last call.
    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut self.shared.lock().unwrap().events)
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
                shared.lock().unwrap().status = e;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        {
            let mut s = shared.lock().unwrap();
            s.status = "Maschine MK3 connected".into();
            s.connected = true;
        }
        let mut events = Vec::new();
        // Shown on the LEDs: none yet, so the first state is sent.
        let mut shown = None;
        // The backlight as last set on this connection.
        let mut brightness = None;
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
            if let Err(e) = dev.poll(2, &mut events) {
                let mut s = shared.lock().unwrap();
                s.status = format!("Maschine MK3: {e}");
                s.connected = false;
                break;
            }
            let wanted = shared.lock().unwrap().display_brightness;
            if wanted.is_some() && wanted != brightness {
                if let Some(level) = wanted
                    && let Err(e) = dev.set_display_brightness(level)
                {
                    shared.lock().unwrap().display_status = format!("display brightness: {e}");
                }
                brightness = wanted;
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
                sensitivity: 0.5,
                scaling: 0,
                fixed: 100,
                touches: [PadTouch::default(); 16],
            });
            let out = s.output.clone().filter(Sender::has_engine);
            let monitor = s.monitor.clone().filter(crate::midi_monitor::Monitor::is_open);
            let shown = |pad: usize, slot: Option<(usize, usize, i64)>, ev| {
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
                    shown(pad, slot, ev);
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
                    Event::PadOn(p, v) => (p, ((pad_velocity(v, route.sensitivity, route.scaling) * 127.0).round() as i64).clamp(1, 127)),
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
                        shown(pad, Some((group, sound, pitch)), crate::midi_monitor::Pad::Off);
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
                        shown(pad, Some((group, sound, pitch)), crate::midi_monitor::Pad::On(velocity));
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
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Keep the displays showing the latest picture (the displays are their own USB
/// interface, written apart from the buttons and LEDs).
fn run_displays(shared: &Mutex<Shared>) {
    use deckard_mk3::display::{Displays, HEIGHT, WIDTH, rgb565};
    loop {
        let mut d = match Displays::open() {
            Ok(d) => d,
            Err(e) => {
                shared.lock().unwrap().display_status = e;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        {
            let mut s = shared.lock().unwrap();
            s.display_status.clear();
            s.displays_connected = true;
        }
        let mut shown = None;
        loop {
            // One display's half of a 960-wide picture, as the display takes it.
            let half = |px: &[[u8; 4]], x0: usize| -> Vec<u16> {
                let mut out = Vec::with_capacity(WIDTH * HEIGHT);
                for y in 0..HEIGHT {
                    for x in x0..x0 + WIDTH {
                        let [r, g, b, _] = px.get(y * WIDTH * 2 + x).copied().unwrap_or([0, 0, 0, 255]);
                        out.push(rgb565(r, g, b));
                    }
                }
                out
            };
            let closing = {
                let s = shared.lock().unwrap();
                s.closing.then(|| s.goodbye.clone())
            };
            if let Some(goodbye) = closing {
                let px = goodbye.map_or_else(Vec::new, |p| p.to_vec());
                let _ = d.show(0, &half(&px, 0)).and_then(|_| d.show(1, &half(&px, WIDTH)));
                shared.lock().unwrap().displays_closed = true;
                return;
            }
            let latest = shared.lock().unwrap().screens.clone();
            match latest {
                Some((n, px)) if shown != Some(n) => {
                    if let Err(e) = d.show(0, &half(&px, 0)).and_then(|_| d.show(1, &half(&px, WIDTH))) {
                        let mut s = shared.lock().unwrap();
                        s.display_status = e;
                        s.displays_connected = false;
                        break;
                    }
                    shown = Some(n);
                }
                _ => std::thread::sleep(Duration::from_millis(8)),
            }
        }
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
        PadRoute { play: true, group: 0, keys: None, chords: None, velocity: None, sensitivity: 0.5, scaling: 0, fixed: 90, touches }
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
}
