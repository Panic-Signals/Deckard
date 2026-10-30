//! Maschine MK3 over USB (docs/mk3-usb.md): the controller's HID reports decoded into
//! the events NI's controller scripts take (`onSwitchEvent`, `onPadEvent`,
//! `onEncoderEvent`, `onWheelEvent`, `onTouchEvent`).
//!
//! Learned from the hardware (tools: `examples/mk3-learn*.rs`):
//! - report 0x01 (42 bytes): bytes 1..11 = 80 button bits ([`BUTTONS`]); byte 11 low
//!   nibble = the 4-D encoder's position (one step a click, wrapping 0..15); bytes 12..28
//!   = the eight knobs, u16 LE, 0..999 a turn, wrapping; bytes 28..30 a timestamp, 30..32
//!   the touch strip's first finger (0..1023, 0 = not touched), 32..34 a second finger.
//! - report 0x02: pad events, three bytes each — HID pad index, then a u16 BE whose top
//!   nibble is the event (1 press, 4 pressure, 3 release; 0 and 2 the press and release
//!   of a pad pressed slowly, starting light) and low 12 bits the pressure.

pub mod display;
pub mod leds;

use std::time::{Duration, Instant};

/// Native Instruments' USB vendor id.
pub const VENDOR: u16 = 0x17cc;
/// The Maschine MK3.
pub const PRODUCT: u16 = 0x1600;

/// Report 0x01's button bits and the scripts' names for them (`NI.HW.<name>`).
pub const BUTTONS: &[(usize, &str)] = &[
    (0, "BUTTON_WHEEL"),
    (2, "BUTTON_WHEEL_UP"),
    (3, "BUTTON_WHEEL_RIGHT"),
    (4, "BUTTON_WHEEL_DOWN"),
    (5, "BUTTON_WHEEL_LEFT"),
    (6, "BUTTON_SHIFT"),
    (7, "BUTTON_SCREEN_8"),
    (8, "BUTTON_GROUP_A"),
    (9, "BUTTON_GROUP_B"),
    (10, "BUTTON_GROUP_C"),
    (11, "BUTTON_GROUP_D"),
    (12, "BUTTON_GROUP_E"),
    (13, "BUTTON_GROUP_F"),
    (14, "BUTTON_GROUP_G"),
    (15, "BUTTON_GROUP_H"),
    (16, "BUTTON_NOTES"),
    (17, "BUTTON_VOLUME"),
    (18, "BUTTON_SWING"),
    (19, "BUTTON_TEMPO"),
    (20, "BUTTON_NOTE_REPEAT"),
    (21, "BUTTON_LOCK"),
    (24, "BUTTON_PAD_MODE"),
    (25, "BUTTON_KEYBOARD"),
    (26, "BUTTON_CHORDS"),
    (27, "BUTTON_STEP"),
    (28, "BUTTON_FIX_VELOCITY"),
    (29, "BUTTON_SCENE"),
    (30, "BUTTON_PATTERN"),
    (31, "BUTTON_EVENTS"),
    (33, "BUTTON_VARIATION"),
    (34, "BUTTON_DUPLICATE"),
    (35, "BUTTON_SELECT"),
    (36, "BUTTON_SOLO"),
    (37, "BUTTON_MUTE"),
    (38, "BUTTON_PITCH"),
    (39, "BUTTON_MOD"),
    (40, "BUTTON_PERFORM"),
    (41, "BUTTON_TRANSPORT_LOOP"),
    (42, "BUTTON_TRANSPORT_ERASE"),
    (43, "BUTTON_TAP"),
    (44, "BUTTON_FOLLOW"),
    (45, "BUTTON_TRANSPORT_PLAY"),
    (46, "BUTTON_TRANSPORT_RECORD"),
    (47, "BUTTON_TRANSPORT_STOP"),
    (48, "BUTTON_MACRO"),
    (49, "BUTTON_SETTINGS"),
    (50, "BUTTON_RIGHT"),
    (51, "BUTTON_SAMPLE"),
    (52, "BUTTON_MIX"),
    (53, "BUTTON_PLUGIN"),
    (56, "BUTTON_CHANNEL"),
    (57, "BUTTON_ARRANGE"),
    (58, "BUTTON_BROWSE"),
    (59, "BUTTON_LEFT"),
    (60, "BUTTON_FILE"),
    (61, "BUTTON_AUTO"),
    (64, "BUTTON_SCREEN_1"),
    (65, "BUTTON_SCREEN_2"),
    (66, "BUTTON_SCREEN_3"),
    (67, "BUTTON_SCREEN_4"),
    (68, "BUTTON_SCREEN_5"),
    (69, "BUTTON_SCREEN_6"),
    (70, "BUTTON_SCREEN_7"),
    (71, "BUTTON_WHEEL_TOUCH"),
    (72, "BUTTON_CAP_8"),
    (73, "BUTTON_CAP_7"),
    (74, "BUTTON_CAP_6"),
    (75, "BUTTON_CAP_5"),
    (76, "BUTTON_CAP_4"),
    (77, "BUTTON_CAP_3"),
    (78, "BUTTON_CAP_2"),
    (79, "BUTTON_CAP_1"),
];

/// What the controller did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// A button (`NI.HW.<name>`) went down or up.
    Button(&'static str, bool),
    /// Knob `index` (0..8) turned by `inc` (a fraction of a turn, + clockwise).
    Knob(usize, f64),
    /// The 4-D encoder turned by `steps` clicks (+ clockwise).
    Wheel(i32),
    /// Pad `pad` (1..=16, as printed: 1 bottom left) was hit with `velocity` (0..1).
    PadOn(usize, f64),
    /// Pressure on a held pad (0..1).
    PadPressure(usize, f64),
    PadOff(usize),
    /// Touch-strip finger `finger` (0, 1): `kind` 0 touched, 1 moved, 2 let go, at
    /// `pos` (0..1).
    Touch(usize, u8, f64),
}

/// The pad printed `n` (1..=16, rows from the bottom) for HID pad index `i`.
pub fn pad_number(i: u8) -> usize {
    let (row_from_top, col) = (i as usize / 4, i as usize % 4);
    (3 - row_from_top) * 4 + col + 1
}

/// Turns reports into [`Event`]s, keeping the last state.
#[derive(Default)]
pub struct Decoder {
    /// The buttons held (all released to begin with: the MK3 reports only changes, so
    /// its first report may be the first press).
    buttons: [u8; 10],
    wheel: Option<u8>,
    knobs: Option<[u16; 8]>,
    /// Each knob's last direction moved (+1, −1, 0 none yet) and the counts back against
    /// it not yet passed on.
    knob_dir: [i32; 8],
    knob_back: [i32; 8],
    strip: [u16; 2],
    /// Pads waiting for their velocity: the highest pressure so far, the reports seen,
    /// and when they were pressed.
    pending: [Option<(u16, u8, Instant)>; 16],
    held: [bool; 16],
}

/// Counts a knob must turn back against its last direction before that counts: a knob
/// resting between two counts flickers by one, which would otherwise wobble whatever it
/// turns (the Sampling page's zoom re-centres the waveform on every change).
const KNOB_REVERSE: i32 = 2;

/// Pressure reports a hit's velocity is taken over (its peak; ~1 ms each).
const VELOCITY_REPORTS: u8 = 3;
/// The longest a hit waits for them: the MK3 reports a pad's pressure only as it changes,
/// so a pad pressed and held steady (often, with others held) may send no more. Then
/// the hit plays with the peak so far.
const VELOCITY_WAIT: Duration = Duration::from_millis(4);

impl Decoder {
    pub fn new() -> Decoder {
        Decoder::default()
    }

    /// Knob `k` moved `d` counts: what to pass on. Counts its way go at once; counts
    /// back go once there are `KNOB_REVERSE` of them (one alone is flicker, cancelled
    /// when the knob comes its way again).
    fn knob_moved(&mut self, k: usize, d: i32) -> Option<i32> {
        if d == 0 {
            return None;
        }
        let dir = self.knob_dir[k];
        if dir == 0 || d.signum() == dir {
            // Back where it was first: only what's past that moves.
            let net = d + std::mem::take(&mut self.knob_back[k]);
            self.knob_dir[k] = d.signum();
            if net.signum() == d.signum() {
                return Some(net);
            }
            self.knob_back[k] = net;
            return None;
        }
        self.knob_back[k] += d;
        if self.knob_back[k].abs() < KNOB_REVERSE {
            return None;
        }
        let d = std::mem::take(&mut self.knob_back[k]);
        self.knob_dir[k] = d.signum();
        Some(d)
    }

    /// The events in one HID input report (report id first).
    pub fn decode(&mut self, r: &[u8], out: &mut Vec<Event>) {
        self.decode_at(r, Instant::now(), out);
    }

    /// The events in one HID input report that came at `now`.
    pub fn decode_at(&mut self, r: &[u8], now: Instant, out: &mut Vec<Event>) {
        match r.first() {
            Some(1) if r.len() >= 34 => self.controls(r, out),
            Some(2) => self.pads(&r[1..], now, out),
            _ => {}
        }
    }

    fn controls(&mut self, r: &[u8], out: &mut Vec<Event>) {
        let bits: [u8; 10] = r[1..11].try_into().unwrap();
        let old = self.buttons;
        for &(bit, name) in BUTTONS {
            let (was, is) = (old[bit / 8] >> (bit % 8) & 1, bits[bit / 8] >> (bit % 8) & 1);
            if was != is {
                out.push(Event::Button(name, is == 1));
            }
        }
        self.buttons = bits;
        let wheel = r[11] & 0x0f;
        if let Some(old) = self.wheel {
            let d = ((wheel as i32 - old as i32 + 8).rem_euclid(16)) - 8;
            if d != 0 {
                out.push(Event::Wheel(d));
            }
        }
        self.wheel = Some(wheel);
        let knobs: [u16; 8] = std::array::from_fn(|k| u16::from_le_bytes([r[12 + 2 * k], r[13 + 2 * k]]));
        if let Some(old) = self.knobs {
            for k in 0..8 {
                let d = ((knobs[k] as i32 - old[k] as i32 + 500).rem_euclid(1000)) - 500;
                if let Some(d) = self.knob_moved(k, d) {
                    out.push(Event::Knob(k, d as f64 / 1000.0));
                }
            }
        }
        self.knobs = Some(knobs);
        for f in 0..2 {
            let v = u16::from_le_bytes([r[30 + 2 * f], r[31 + 2 * f]]) & 0x3ff;
            let old = self.strip[f];
            let pos = v as f64 / 1023.0;
            match (old, v) {
                (0, 0) => {}
                (0, _) => out.push(Event::Touch(f, 0, pos)),
                (_, 0) => out.push(Event::Touch(f, 2, old as f64 / 1023.0)),
                (a, b) if a != b => out.push(Event::Touch(f, 1, pos)),
                _ => {}
            }
            self.strip[f] = v;
        }
    }

    fn pads(&mut self, b: &[u8], now: Instant, out: &mut Vec<Event>) {
        for e in b.chunks_exact(3) {
            let word = u16::from_be_bytes([e[1], e[2]]);
            if word == 0 && e[0] == 0 {
                break;
            }
            let Some(slot) = (e[0] < 16).then_some(e[0] as usize) else { continue };
            let pad = pad_number(e[0]);
            let pressure = word & 0x0fff;
            match word >> 12 {
                // A press: 1 struck, 0 pressed slowly (starting light).
                0 | 1 => {
                    self.pending[slot] = Some((pressure, 1, now));
                    self.held[slot] = true;
                }
                // Pressure only from held pads: the MK3 keeps reporting a released pad's
                // pressure as it springs back, which isn't a press.
                4 if self.held[slot] => match &mut self.pending[slot] {
                    Some((peak, n, _)) => {
                        *peak = (*peak).max(pressure);
                        *n += 1;
                        if *n >= VELOCITY_REPORTS {
                            out.push(Event::PadOn(pad, *peak as f64 / 4095.0));
                            self.pending[slot] = None;
                        }
                    }
                    None => out.push(Event::PadPressure(pad, pressure as f64 / 4095.0)),
                },
                // A release: 3 for a struck pad, 2 for one pressed slowly.
                2 | 3 => {
                    if let Some((peak, ..)) = self.pending[slot].take() {
                        out.push(Event::PadOn(pad, peak as f64 / 4095.0));
                    }
                    if std::mem::take(&mut self.held[slot]) {
                        out.push(Event::PadOff(pad));
                    }
                }
                _ => {}
            }
        }
        self.flush_at(now, out);
    }

    /// Play the hits that have waited [`VELOCITY_WAIT`] for their pressure reports.
    pub fn flush(&mut self, out: &mut Vec<Event>) {
        self.flush_at(Instant::now(), out);
    }

    fn flush_at(&mut self, now: Instant, out: &mut Vec<Event>) {
        for slot in 0..16 {
            if let Some((peak, _, at)) = self.pending[slot]
                && now.duration_since(at) >= VELOCITY_WAIT
            {
                out.push(Event::PadOn(pad_number(slot as u8), peak as f64 / 4095.0));
                self.pending[slot] = None;
            }
        }
    }
}

/// Where the backlight level sits in a display's settings report.
const DISPLAY_BRIGHTNESS_BYTE: usize = 8;

/// An open MK3 (its HID interface, shared with NI's agent if that runs).
pub struct Device {
    hid: hidapi::HidDevice,
    decoder: Decoder,
}

impl Device {
    /// The first MK3 connected, if any.
    pub fn open() -> Result<Device, String> {
        let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
        // Shared with NI's agent (macOS; elsewhere HID devices open shared).
        #[cfg(target_os = "macos")]
        api.set_open_exclusive(false);
        let hid = api.open(VENDOR, PRODUCT).map_err(|e| format!("no Maschine MK3: {e}"))?;
        Ok(Device { hid, decoder: Decoder::new() })
    }

    /// Wait up to `timeout_ms` for input; the events of every report that came.
    pub fn poll(&mut self, timeout_ms: i32, out: &mut Vec<Event>) -> Result<(), String> {
        let mut buf = [0u8; 64];
        let mut wait = timeout_ms;
        loop {
            let n = self.hid.read_timeout(&mut buf, wait).map_err(|e| e.to_string())?;
            if n == 0 {
                // No more reports: hits still waiting for theirs play now if it's time.
                self.decoder.flush(out);
                return Ok(());
            }
            self.decoder.decode(&buf[..n], out);
            wait = 0;
        }
    }

    /// Send an output report (LEDs: 0x80 buttons, 0x81 pads and touch strip).
    pub fn write(&self, report: &[u8]) -> Result<(), String> {
        self.hid.write(report).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Show these LEDs (both reports).
    pub fn write_leds(&self, l: &leds::Leds) -> Result<(), String> {
        self.write(&l.buttons)?;
        self.write(&l.pads)
    }

    /// The displays' backlight, 0..=100. Each display's settings are a feature report
    /// (0xf8 left, 0xf9 right: width, height, then single bytes); the brightness is
    /// byte 8, which the controller starts at 20 (`examples/mk3-display-brightness.rs`).
    pub fn set_display_brightness(&self, level: u8) -> Result<(), String> {
        for id in [0xf8, 0xf9] {
            let mut buf = [0u8; 64];
            buf[0] = id;
            let n = self.hid.get_feature_report(&mut buf).map_err(|e| e.to_string())?;
            if n <= DISPLAY_BRIGHTNESS_BYTE {
                return Err(format!("display settings 0x{id:02x}: {n} bytes"));
            }
            buf[DISPLAY_BRIGHTNESS_BYTE] = level.min(100);
            self.hid.send_feature_report(&buf[..n]).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report1(bits: &[usize], wheel: u8, knob0: u16, strip: u16) -> Vec<u8> {
        let mut r = vec![0u8; 42];
        r[0] = 1;
        for &b in bits {
            r[1 + b / 8] |= 1 << (b % 8);
        }
        r[11] = wheel;
        r[12..14].copy_from_slice(&knob0.to_le_bytes());
        r[30..32].copy_from_slice(&strip.to_le_bytes());
        r
    }

    /// The MK3 reports only changes, so its first report may be the first press: buttons
    /// start released, not from that report.
    #[test]
    fn the_first_press_counts() {
        let mut d = Decoder::new();
        let mut ev = Vec::new();
        d.decode(&report1(&[45], 15, 995, 0), &mut ev);
        assert_eq!(ev, [Event::Button("BUTTON_TRANSPORT_PLAY", true)]);
    }

    #[test]
    fn buttons_wheel_knobs_and_strip() {
        let mut d = Decoder::new();
        let mut ev = Vec::new();
        d.decode(&report1(&[], 15, 995, 0), &mut ev);
        assert!(ev.is_empty(), "the first report only sets the state");
        d.decode(&report1(&[45], 1, 5, 512), &mut ev);
        assert_eq!(ev[0], Event::Button("BUTTON_TRANSPORT_PLAY", true));
        assert_eq!(ev[1], Event::Wheel(2), "15 → 1 wraps: two clicks");
        assert_eq!(ev[2], Event::Knob(0, 0.01), "995 → 5 wraps: +10");
        assert!(matches!(ev[3], Event::Touch(0, 0, _)));
        ev.clear();
        d.decode(&report1(&[], 1, 5, 0), &mut ev);
        assert_eq!(ev[0], Event::Button("BUTTON_TRANSPORT_PLAY", false));
        assert!(matches!(ev[1], Event::Touch(0, 2, _)));
    }

    /// A knob resting between two counts (5, 4, 5, 4, …) moves nothing; turned back
    /// for real it moves at once after the second count.
    #[test]
    fn a_knob_flickering_by_one_count_is_still() {
        let mut d = Decoder::new();
        let mut ev = Vec::new();
        for v in [0, 3, 5] {
            d.decode(&report1(&[], 0, v, 0), &mut ev);
        }
        assert_eq!(ev, [Event::Knob(0, 0.003), Event::Knob(0, 0.002)]);
        ev.clear();
        for v in [4, 5, 4, 5, 4, 5] {
            d.decode(&report1(&[], 0, v, 0), &mut ev);
        }
        assert!(ev.is_empty(), "flicker: {ev:?}");
        for v in [4, 3, 2] {
            d.decode(&report1(&[], 0, v, 0), &mut ev);
        }
        assert_eq!(ev, [Event::Knob(0, -0.002), Event::Knob(0, -0.001)], "turned back");
    }

    #[test]
    fn pads_take_their_peak_as_velocity() {
        assert_eq!(pad_number(12), 1);
        assert_eq!(pad_number(3), 16);
        let mut d = Decoder::new();
        let mut ev = Vec::new();
        // HID pad 12 (pad 1): press at 0x0ba, pressure 0x287, 0xc13, then 0x400, release,
        // all at one moment (the reports' own times, however slowly the test runs).
        let now = Instant::now();
        for w in [0x10ba_u16, 0x4287, 0x4c13, 0x4400, 0x3000] {
            let mut r = vec![2u8, 12];
            r.extend_from_slice(&w.to_be_bytes());
            d.decode_at(&r, now, &mut ev);
            std::thread::sleep(VELOCITY_WAIT + Duration::from_millis(1));
        }
        assert_eq!(ev[0], Event::PadOn(1, 0xc13 as f64 / 4095.0));
        assert_eq!(ev[1], Event::PadPressure(1, 0x400 as f64 / 4095.0));
        assert_eq!(ev[2], Event::PadOff(1));
    }

    /// A pad pressed slowly comes as event 0 and goes as event 2 (seen on the MK3, with
    /// other pads held).
    #[test]
    fn a_slow_press_plays_and_releases() {
        let mut d = Decoder::new();
        let mut ev = Vec::new();
        // Pad 1 (HID 12): press at 60, then the pressure rising.
        d.decode(&[2, 12, 0x00, 60, 12, 0x43, 0x09], &mut ev);
        d.flush_at(Instant::now() + VELOCITY_WAIT, &mut ev);
        assert_eq!(ev, [Event::PadOn(1, 0x309 as f64 / 4095.0)]);
        ev.clear();
        d.decode(&[2, 12, 0x20, 0x00], &mut ev);
        assert_eq!(ev, [Event::PadOff(1)]);
    }

    /// Pressure after a release (the pad springing back) doesn't press it again.
    #[test]
    fn pressure_after_a_release_is_not_a_press() {
        let mut d = Decoder::new();
        let mut ev = Vec::new();
        d.decode(&[2, 12, 0x18, 0x00, 12, 0x30, 0x00], &mut ev);
        assert_eq!(ev, [Event::PadOn(1, 0x800 as f64 / 4095.0), Event::PadOff(1)]);
        ev.clear();
        d.decode(&[2, 12, 0x41, 0x00], &mut ev);
        d.flush_at(Instant::now() + VELOCITY_WAIT, &mut ev);
        assert!(ev.is_empty(), "no hanging note: {ev:?}");
    }

    /// A pad pressed and held steady (no more pressure reports, as with other pads held)
    /// still plays, once it has waited for them, with the pressure it came with.
    #[test]
    fn a_held_pad_plays_without_more_reports() {
        let mut d = Decoder::new();
        let mut ev = Vec::new();
        // Pad 1 (HID 12) pressed at 0x800 and nothing more.
        d.decode(&[2, 12, 0x18, 0x00], &mut ev);
        assert!(ev.is_empty(), "waits for its pressure");
        d.flush_at(Instant::now() + VELOCITY_WAIT, &mut ev);
        assert_eq!(ev, [Event::PadOn(1, 0x800 as f64 / 4095.0)]);
        // Released: off only (it played already).
        ev.clear();
        d.decode(&[2, 12, 0x30, 0x00], &mut ev);
        assert_eq!(ev, [Event::PadOff(1)]);
    }
}
