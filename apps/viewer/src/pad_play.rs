//! What a pad hit plays, out of the device thread: the pad core every device (the
//! MK3, the on-screen panel, a browser client) plays its pads through.
//!
//! A hit plays the focused sound's pitch for its pad — its key, in Keyboard mode,
//! or its chord, in Chords mode — held until the pad is let go, and what it started
//! is remembered per pad, so a release stops the same note however the route moved
//! meanwhile. A held pad's pressure becomes what its sound's Pad Touch asks for
//! (Note Repeat's velocity, poly aftertouch, channel aftertouch or a CC), and only
//! changes are sent. With no engine running, a hit waits in the pending queue for
//! one instead.

use crate::mk3_io::{PadRoute, pad_notes};
use deckard_engine::Live;
use deckard_model::pad_touch::{PadTouch, TouchMode};

/// Each velocity scaling's curve, Soft 3 … Hard 3: (m, n) in [`pad_velocity`].
const VELOCITY_CURVES: [(f64, f64); 7] = [(1.71, 2.5), (1.50, 2.5), (1.26, 2.5), (1.0, 1.0), (0.82, 4.0), (0.60, 4.0), (0.32, 4.0)];

/// A pad's pressure (0..1, as the pad was hit) as velocity (0..1), by the velocity
/// scaling (-3 Soft 3 … 3 Hard 3), as Maschine plays it. With u = √pressure, the
/// velocity is min(1, m·u + (1 − m)·uⁿ): Linear is u itself, and m is the slope at the
/// light end (Soft above 1, Hard below), n how late it bends to full. Measured against
/// Maschine 3 on an MK3, within a velocity step at every setting (docs/mk3-usb.md).
/// The sensitivity isn't in it: the controller applies it (`Mk3Io::set_pad_sensitivity`).
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
    /// sound's settings as last seen. (A release needs neither the slot nor the route:
    /// this is what the MK3's own tests call.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn release(&mut self, pad: usize, _slot: (usize, usize, i64), _route: &PadRoute) -> Vec<Live> {
        self.let_go(pad)
    }

    /// Pad `pad` (1–16) let go, whatever it was sounding: what it was sending, by its
    /// sound's settings as last seen (a release needs no route).
    fn let_go(&mut self, pad: usize) -> Vec<Live> {
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

/// The pads as one device plays them: what each hit started (so a release stops the
/// same note however the route moved), what the held pads' pressure sends, and the
/// hits that came while no engine was running, for the app to play.
pub struct Pads {
    /// What each pad (0–15) started, so it stops the same note however the route
    /// changed (a chord's first note; its others in `chord_rest`).
    sounding: [Option<(usize, usize, i64)>; 16],
    chord_rest: [Vec<(usize, usize, i64)>; 16],
    /// What the held pads' pressure sends (by each sound's Pad Touch).
    touching: Touching,
    /// Pad hits that came while no engine played: (group, sound, pitch, velocity).
    pending: Vec<(usize, usize, i64, i64)>,
    /// An engine is running: hits play now. None, they wait in `pending` for one.
    engine: bool,
}

impl Default for Pads {
    /// Pads ready to play: a device's pads wait for no engine until it says so.
    fn default() -> Pads {
        Pads { sounding: [None; 16], chord_rest: Default::default(), touching: Touching::default(), pending: Vec::new(), engine: true }
    }
}

impl Pads {
    pub fn new() -> Pads {
        Pads::default()
    }

    /// Whether an engine is running: hits play now, or wait in `pending` for one.
    pub(crate) fn set_engine(&mut self, running: bool) {
        self.engine = running;
    }

    /// What pad `pad` (1–16) is sounding now: its group, sound and pitch, if it is.
    pub fn sounding(&self, pad: usize) -> Option<(usize, usize, i64)> {
        pad.checked_sub(1).filter(|i| *i < 16).and_then(|i| self.sounding[i])
    }

    /// Pad `pad` (1–16) hit at `velocity` (1–127): the notes to play for it, held
    /// until it is let go. With no engine running, they wait in `pending` instead.
    pub fn hit(&mut self, route: &PadRoute, pad: usize, velocity: i64) -> Vec<Live> {
        if !route.play {
            return Vec::new();
        }
        let notes = pad_notes(route, pad);
        let Some(&(sound, pitch)) = notes.first() else { return Vec::new() };
        let group = route.group;
        let velocity = Touching::hit_velocity(route, group, sound, velocity);
        if !self.engine {
            // No engine yet: the notes, for the app to play.
            self.pending.extend(notes.iter().map(|&(sound, pitch)| (group, sound, pitch, velocity)));
            return Vec::new();
        }
        let mut out = Vec::new();
        // Held until the pad is let go (Note Repeat / the Arp take held notes).
        out.push(Live::NoteOn { group, sound, pitch, velocity, length: i64::MAX });
        self.touching.hit(pad, (group, sound, pitch), route);
        if let Some(slot) = pad.checked_sub(1).and_then(|i| self.sounding.get_mut(i)) {
            *slot = Some((group, sound, pitch));
        }
        // (A chord's other notes sound with it; its pressure goes to the first.)
        let others: Vec<_> = notes[1..].iter().map(|&(sound, pitch)| (group, sound, pitch)).collect();
        for &(group, sound, pitch) in &others {
            out.push(Live::NoteOn { group, sound, pitch, velocity, length: i64::MAX });
        }
        if let Some(rest) = pad.checked_sub(1).and_then(|i| self.chord_rest.get_mut(i)) {
            *rest = others;
        }
        out
    }

    /// Pad `pad` (1–16) pressed at `pressure` (0..1), `slot` what it is sounding
    /// (see [`Pads::sounding`]): what its sound's Pad Touch sends now (only changes).
    pub fn pressure(&mut self, route: &PadRoute, pad: usize, pressure: f64, slot: Option<(usize, usize, i64)>) -> Vec<Live> {
        self.touching.pressure(pad, slot, pressure, route)
    }

    /// Pad `pad` (1–16) let go: the notes it started, and what its pressure was
    /// sending, all stopped.
    pub fn release(&mut self, pad: usize) -> Vec<Live> {
        let Some(i) = pad.checked_sub(1).filter(|i| *i < 16) else { return Vec::new() };
        let rest = std::mem::take(&mut self.chord_rest[i]);
        let Some((group, sound, pitch)) = self.sounding[i].take() else { return Vec::new() };
        let mut out: Vec<Live> = rest.into_iter().map(|(group, sound, pitch)| Live::NoteRelease { group, sound, pitch }).collect();
        out.push(Live::NoteRelease { group, sound, pitch });
        out.extend(self.touching.let_go(pad));
        out
    }

    /// The hits taken while no engine was running, and forgotten: (group, sound,
    /// pitch, velocity; 0 for a release).
    pub fn pending(&mut self) -> Vec<(usize, usize, i64, i64)> {
        std::mem::take(&mut self.pending)
    }
}
