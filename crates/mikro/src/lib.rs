//! Maschine Mikro MK3 over USB HID (docs/mikro-usb.md, to be written).
//!
//! VID 0x17cc, PID 0x1700. All communication — controls and the OLED screen — goes
//! through the HID interface (Interface 4). There is no bulk USB display interface on
//! the Mikro; it is a fundamentally different device from the Maschine MK3.
//!
//! Protocol learned from open-source community work (maschine-mikro-mk3-driver by
//! r00tman; cabl; pymikro). No Native Instruments code.
//!
//! # Input reports
//!
//! ## Report 0x01 (64 bytes) — buttons, encoder, touch strip
//!
//! | bytes | content                                                       |
//! |-------|---------------------------------------------------------------|
//! | 1–6   | 48 button bits: bit n = byte[1 + n/8], mask 1 << (n%8)        |
//! | 7     | encoder relative delta, treat as `i8` (positive = clockwise)  |
//! | 10    | touch strip: 0 = untouched, 1–200 = finger position left→right |
//!
//! Button bit assignments are in [`BUTTONS`].
//!
//! ## Report 0x02 (64 bytes) — pad events
//!
//! Three-byte entries, terminated by an all-zero entry:
//! - byte 0: pad HID index (0–15)
//! - byte 1 high nibble: event type (0x0/0x1 = press/NoteOn, 0x2/0x3 = release/NoteOff,
//!   0x4 = aftertouch)
//! - byte 1 low nibble + byte 2: 12-bit pressure (0–4095)
//!
//! # Output reports
//!
//! ## Report 0x80 (81 bytes) — all LEDs
//!
//! One write for buttons, pads, and touch-strip segments combined:
//! - byte 0:    report id 0x80
//! - bytes 1–39:  39 button LEDs (single-channel brightness)
//! - bytes 40–55: 16 pad LEDs (colour palette byte)
//! - bytes 56–80: 25 touch-strip segment LEDs (single-channel brightness)
//!
//! ## Reports 0xe0 × 2 (265 bytes each) — 128×64 OLED screen
//!
//! Two writes push the full 512-byte framebuffer:
//! - header (9 bytes) + 256 bytes of framebuffer data
//! - Pixels are 1-bit monochrome, column-major chunked (see [`Screen`])

pub mod leds;
pub mod screen;

pub use screen::Screen;

/// Native Instruments' USB vendor id (shared with MK3, Jam, etc.).
pub const VENDOR: u16 = 0x17cc;

/// Maschine Mikro MK3 USB product id.
pub const PRODUCT: u16 = 0x1700;

/// Report 0x01 button bit-index → name pairs.
///
/// Indices are the bit position within bytes 1–6 of the input report (0 = LSB of byte 1).
/// Names follow the `NI.HW.BUTTON_*` convention used in the rest of Deckard's surface layer.
/// Derived from open-source maschine-mikro-mk3-driver (controls.rs).
pub const BUTTONS: &[(usize, &str)] = &[
    (0,  "BUTTON_MASCHINE"),
    (1,  "BUTTON_STAR"),
    (2,  "BUTTON_BROWSE"),
    (3,  "BUTTON_VOLUME"),
    (4,  "BUTTON_SWING"),
    (5,  "BUTTON_TEMPO"),
    (6,  "BUTTON_PLUGIN"),
    (7,  "BUTTON_SAMPLING"),
    (8,  "BUTTON_LEFT"),
    (9,  "BUTTON_RIGHT"),
    (10, "BUTTON_PITCH"),
    (11, "BUTTON_MOD"),
    (12, "BUTTON_PERFORM"),
    (13, "BUTTON_NOTES"),
    (14, "BUTTON_GROUP"),
    (15, "BUTTON_AUTO"),
    (16, "BUTTON_LOCK"),
    (17, "BUTTON_NOTE_REPEAT"),
    (18, "BUTTON_TRANSPORT_LOOP"),
    (19, "BUTTON_TRANSPORT_ERASE"),
    (20, "BUTTON_TAP"),
    (21, "BUTTON_FOLLOW"),
    (22, "BUTTON_TRANSPORT_PLAY"),
    (23, "BUTTON_TRANSPORT_RECORD"),
    (24, "BUTTON_TRANSPORT_STOP"),
    (25, "BUTTON_SHIFT"),
    (26, "BUTTON_FIX_VELOCITY"),
    (27, "BUTTON_PAD_MODE"),
    (28, "BUTTON_KEYBOARD"),
    (29, "BUTTON_CHORDS"),
    (30, "BUTTON_STEP"),
    (31, "BUTTON_SCENE"),
    (32, "BUTTON_PATTERN"),
    (33, "BUTTON_EVENTS"),
    (34, "BUTTON_VARIATION"),
    (35, "BUTTON_DUPLICATE"),
    (36, "BUTTON_SELECT"),
    (37, "BUTTON_SOLO"),
    (38, "BUTTON_MUTE"),
    // Encoder: press and capacitive touch are inputs only — no LEDs behind them.
    (39, "BUTTON_ENCODER_PRESS"),
    (40, "BUTTON_ENCODER_TOUCH"),
];

/// Whether a button name has a backlit LED. The encoder's press and touch sensors do not.
pub fn has_led(name: &str) -> bool {
    !matches!(name, "BUTTON_ENCODER_PRESS" | "BUTTON_ENCODER_TOUCH")
}

/// Convert a HID pad index (0–15) to the 1-based pad number printed on the hardware.
///
/// The Mikro MK3 uses the same layout as the full MK3: index 12 = pad 1 (bottom left),
/// index 15 = pad 4 (bottom right), index 0 = pad 13 (top left).
pub fn pad_number(hid_index: u8) -> usize {
    let row_from_top = hid_index as usize / 4;
    let col = hid_index as usize % 4;
    (3 - row_from_top) * 4 + col + 1
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Everything the Mikro MK3 can tell us.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// A button changed state. `name` is from [`BUTTONS`]; `pressed` = true on down.
    Button(&'static str, bool),
    /// The encoder turned. `delta` is signed: positive = clockwise, negative = counter.
    EncoderTurn(i8),
    /// The touch strip. `kind`: 0 = first touch, 1 = move, 2 = release. `pos`: 0.0–1.0.
    Strip(u8, f64),
    /// A pad was pressed. `pad` = 1–16 (printed number). `pressure` = 0.0–1.0 (12-bit).
    PadOn(usize, f64),
    /// Pressure update on a held pad.
    PadPressure(usize, f64),
    /// A pad was released.
    PadOff(usize),
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Stateful decoder: call [`Decoder::decode`] with each raw HID report.
#[derive(Default)]
pub struct Decoder {
    /// Last 6 button bytes from report 0x01, for edge detection.
    buttons: [u8; 6],
    /// Previous encoder detent counter (4-bit 0..=15 wrapping), for delta calculation.
    encoder: Option<u8>,
    /// Previous raw strip position (0 = untouched), for touch/move/release detection.
    strip: u8,
    /// Which pads are currently held, indexed by HID index (0–15).
    held: [bool; 16],
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode one HID report and append any resulting [`Event`]s to `out`.
    pub fn decode(&mut self, r: &[u8], out: &mut Vec<Event>) {
        match r.first() {
            Some(0x01) if r.len() >= 11 => self.controls(r, out),
            Some(0x02)                  => self.pads(&r[1..], out),
            _                           => {}
        }
    }

    fn controls(&mut self, r: &[u8], out: &mut Vec<Event>) {
        // ---- buttons (bytes 1–6, 48 bit-positions, 41 assigned) ----
        let bits: [u8; 6] = r[1..7].try_into().unwrap();
        let old = self.buttons;
        for &(bit, name) in BUTTONS {
            let was = (old[bit / 8] >> (bit % 8)) & 1;
            let is  = (bits[bit / 8] >> (bit % 8)) & 1;
            if was != is {
                out.push(Event::Button(name, is == 1));
            }
        }
        self.buttons = bits;

        // ---- encoder (byte 7 low nibble: 4-bit counter 0..=15, wrapping) ----
        let raw = r[7] & 0x0f;
        if let Some(prev) = self.encoder {
            let delta = (raw as i8 - prev as i8 + 8).rem_euclid(16) - 8;
            if delta != 0 {
                out.push(Event::EncoderTurn(delta));
            }
        }
        self.encoder = Some(raw);

        // ---- touch strip (byte 10: 0 = off, 1–200 = position) ----
        let pos_raw = r[10];
        let prev    = self.strip;
        let pos     = if pos_raw == 0 { 0.0 } else { (pos_raw as f64 - 1.0) / 199.0 };
        match (prev, pos_raw) {
            (0, 0) => {}
            (0, _) => out.push(Event::Strip(0, pos)),
            (_, 0) => out.push(Event::Strip(2, (prev as f64 - 1.0) / 199.0)),
            (a, b) if a != b => out.push(Event::Strip(1, pos)),
            _ => {}
        }
        self.strip = pos_raw;
    }

    fn pads(&mut self, b: &[u8], out: &mut Vec<Event>) {
        for e in b.chunks_exact(3) {
            // All-zero entry terminates the list.
            if e[0] == 0 && e[1] == 0 && e[2] == 0 {
                break;
            }
            let Some(slot) = (e[0] < 16).then_some(e[0] as usize) else { continue };
            let pad      = pad_number(e[0]);
            let pressure = (((e[1] & 0x0f) as u16) << 8) | e[2] as u16;
            let p_norm   = pressure as f64 / 4095.0;
            match (e[1] & 0xf0) >> 4 {
                0x0 | 0x1 => {
                    // PressOn or NoteOn
                    out.push(Event::PadOn(pad, p_norm));
                    self.held[slot] = true;
                }
                0x4 if self.held[slot] => {
                    // Aftertouch on a held pad
                    out.push(Event::PadPressure(pad, p_norm));
                }
                0x2 | 0x3 => {
                    // PressOff or NoteOff
                    if std::mem::take(&mut self.held[slot]) {
                        out.push(Event::PadOff(pad));
                    }
                }
                _ => {}
            }
        }
    }
}



// ---------------------------------------------------------------------------
// Mikro — the top-level device handle
// ---------------------------------------------------------------------------

/// An open Maschine Mikro MK3. Mirrors the `Jam` type in `deckard-jam`.
pub struct Mikro {
    dev:     hidapi::HidDevice,
    decoder: Decoder,
    /// Last flushed LED state; only the report is sent when it differs.
    shown:   Option<leds::Leds>,
}

impl Mikro {
    /// Open the first Maschine Mikro MK3 found.
    ///
    /// On macOS, the HID device is opened non-exclusive so it can coexist with
    /// NIHostIntegrationAgent while the display (which is HID-only on the Mikro)
    /// is not in use by NI software. On Linux, the udev rule
    /// `scripts/linux/50-deckard-maschine-mikro-mk3.rules` grants access.
    pub fn open() -> Result<Self, String> {
        Ok(Mikro {
            dev:     open()?,
            decoder: Decoder::default(),
            shown:   None,
        })
    }

    /// Poll for events. Waits up to `timeout_ms` for the first report, then drains
    /// whatever else has arrived without blocking.
    pub fn poll(&mut self, timeout_ms: i32, out: &mut Vec<Event>) -> Result<(), String> {
        let mut buf = [0u8; 64];
        let mut wait = timeout_ms;
        loop {
            let n = self.dev.read_timeout(&mut buf, wait).map_err(|e| e.to_string())?;
            if n == 0 {
                return Ok(());
            }
            self.decoder.decode(&buf[..n], out);
            wait = 0;
        }
    }

    /// Push LED state to the device. Sends the 81-byte report only if it differs
    /// from the last write (matches the deckard-jam `show` convention).
    pub fn show(&mut self, leds: &leds::Leds) -> Result<(), String> {
        let changed = self.shown.as_ref().is_none_or(|s| s != leds);
        if changed {
            self.dev.write(&leds.buf).map_err(|e| e.to_string())?;
            self.shown = Some(leds.clone());
        }
        Ok(())
    }

    /// Turn every LED off and forget the last-shown state.
    pub fn dark(&mut self) -> Result<(), String> {
        self.shown = None;
        self.show(&leds::Leds::default())
    }

    /// Push a [`Screen`] framebuffer to the OLED (two 265-byte HID writes).
    pub fn write_screen(&self, screen: &Screen) -> Result<(), String> {
        let (hi, lo) = screen.packets();
        self.dev.write(&hi).map_err(|e| e.to_string())?;
        self.dev.write(&lo).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Low-level HID write; prefer [`show`](Self::show) and [`write_screen`](Self::write_screen).
    pub fn write(&self, report: &[u8]) -> Result<(), String> {
        self.dev.write(report).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Manufacturer and product strings, for connection logging.
    pub fn device_strings(&self) -> (Option<String>, Option<String>) {
        (
            self.dev.get_manufacturer_string().ok().flatten(),
            self.dev.get_product_string().ok().flatten(),
        )
    }
}

// ---------------------------------------------------------------------------
// open() — shared by Mikro::open() and the examples
// ---------------------------------------------------------------------------

/// Open the Maschine Mikro MK3's HID interface.
///
/// Exported so examples can call it directly (matching the deckard-jam pattern of
/// `deckard_jam::open()`).
pub fn open() -> Result<hidapi::HidDevice, String> {
    let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;

    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);

    if !api.device_list().any(|d| d.vendor_id() == VENDOR && d.product_id() == PRODUCT) {
        return Err(format!(
            "no Maschine Mikro MK3 found \
             (looking for USB {:04x}:{:04x}; is the udev rule installed?)",
            VENDOR, PRODUCT
        ));
    }

    api.open(VENDOR, PRODUCT)
        .map_err(|e| format!("couldn't open Maschine Mikro MK3: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_numbering() {
        assert_eq!(pad_number(0), 13, "top left is pad 13");
        assert_eq!(pad_number(3), 16, "top right is pad 16");
        assert_eq!(pad_number(12), 1, "bottom left is pad 1");
        assert_eq!(pad_number(15), 4, "bottom right is pad 4");

        let mut seen = [false; 17];
        for i in 0..16u8 {
            let p = pad_number(i);
            assert!((1..=16).contains(&p));
            assert!(!seen[p], "duplicate pad number {p}");
            seen[p] = true;
        }
    }

    #[test]
    fn decoder_buttons() {
        let mut d = Decoder::new();
        let mut out = Vec::new();

        // Press BUTTON_MASCHINE (bit 0 of byte 1)
        let mut r = [0u8; 64];
        r[0] = 0x01;
        r[1] = 0x01;
        d.decode(&r, &mut out);
        assert_eq!(out, vec![Event::Button("BUTTON_MASCHINE", true)]);

        // Release BUTTON_MASCHINE and press BUTTON_TRANSPORT_PLAY (bit 22 = bit 6 of byte 3)
        // bit 22: byte 1 + 22/8 = byte 3. 22 % 8 = 6. mask = 1 << 6 = 0x40.
        out.clear();
        r[1] = 0x00;
        r[3] = 0x40;
        d.decode(&r, &mut out);
        assert_eq!(
            out,
            vec![
                Event::Button("BUTTON_MASCHINE", false),
                Event::Button("BUTTON_TRANSPORT_PLAY", true),
            ]
        );
    }

    #[test]
    fn decoder_encoder() {
        let mut d = Decoder::new();
        let mut out = Vec::new();

        let mut r = [0u8; 64];
        r[0] = 0x01;
        r[7] = 5;
        d.decode(&r, &mut out);
        assert!(out.is_empty(), "first encoder reading establishes baseline");

        // Turned clockwise by 2
        r[7] = 7;
        d.decode(&r, &mut out);
        assert_eq!(out, vec![Event::EncoderTurn(2)]);

        // Turned counter-clockwise by 3
        out.clear();
        r[7] = 4;
        d.decode(&r, &mut out);
        assert_eq!(out, vec![Event::EncoderTurn(-3)]);

        // Clockwise across 4-bit wrap (15 -> 0 -> 1)
        out.clear();
        r[7] = 15;
        d.decode(&r, &mut out);
        out.clear();
        r[7] = 0;
        d.decode(&r, &mut out);
        assert_eq!(out, vec![Event::EncoderTurn(1)], "15 -> 0 must wrap as +1, not -15");

        // Counter-clockwise across 4-bit wrap (0 -> 15)
        out.clear();
        r[7] = 15;
        d.decode(&r, &mut out);
        assert_eq!(out, vec![Event::EncoderTurn(-1)], "0 -> 15 must wrap as -1");

        // Multi-step clockwise across wrap (14 -> 1)
        out.clear();
        r[7] = 14;
        d.decode(&r, &mut out);
        out.clear();
        r[7] = 1;
        d.decode(&r, &mut out);
        assert_eq!(out, vec![Event::EncoderTurn(3)]);
    }

    #[test]
    fn decoder_strip() {
        let mut d = Decoder::new();
        let mut out = Vec::new();

        let mut r = [0u8; 64];
        r[0] = 0x01;

        // Touch at position 101 (middle of 1..200 range)
        r[10] = 101;
        d.decode(&r, &mut out);
        assert_eq!(out.len(), 1);
        match out[0] {
            Event::Strip(kind, pos) => {
                assert_eq!(kind, 0, "touch start");
                assert!((pos - 0.5025).abs() < 0.01);
            }
            _ => panic!("expected Strip event"),
        }

        // Move to position 150
        out.clear();
        r[10] = 150;
        d.decode(&r, &mut out);
        assert_eq!(out.len(), 1);
        match out[0] {
            Event::Strip(kind, ..) => assert_eq!(kind, 1, "strip move"),
            _ => panic!("expected Strip event"),
        }

        // Release (position 0)
        out.clear();
        r[10] = 0;
        d.decode(&r, &mut out);
        assert_eq!(out.len(), 1);
        match out[0] {
            Event::Strip(kind, ..) => assert_eq!(kind, 2, "strip release"),
            _ => panic!("expected Strip event"),
        }
    }

    #[test]
    fn decoder_pads() {
        let mut d = Decoder::new();
        let mut out = Vec::new();

        let mut r = [0u8; 64];
        r[0] = 0x02;
        // Pad HID index 12 (Pad 1), event 0x1 (NoteOn), pressure 2048 (0x800)
        r[1] = 12;
        r[2] = 0x18; // event 0x1, high pressure nibble 0x8
        r[3] = 0x00; // low pressure 0x00 -> pressure = 0x800 = 2048
        d.decode(&r, &mut out);
        assert_eq!(out.len(), 1);
        match out[0] {
            Event::PadOn(pad, p) => {
                assert_eq!(pad, 1);
                assert!((p - (2048.0 / 4095.0)).abs() < 0.001);
            }
            _ => panic!("expected PadOn event"),
        }

        // Pressure update (event 0x4)
        out.clear();
        r[2] = 0x4c; // event 0x4, pressure 0xc00 = 3072
        r[3] = 0x00;
        d.decode(&r, &mut out);
        assert_eq!(out.len(), 1);
        match out[0] {
            Event::PadPressure(pad, p) => {
                assert_eq!(pad, 1);
                assert!((p - (3072.0 / 4095.0)).abs() < 0.001);
            }
            _ => panic!("expected PadPressure event"),
        }

        // Release (event 0x3)
        out.clear();
        r[2] = 0x30;
        r[3] = 0x00;
        d.decode(&r, &mut out);
        assert_eq!(out, vec![Event::PadOff(1)]);
    }

    #[test]
    fn screen_buffer_and_packets() {
        let mut s = Screen::new();
        assert_eq!(s.buf, [0xff; 512], "initially blank / dark");

        // Pixel (0, 0): chunk 0, col 0, bit 0 -> buf[0] bit 0 cleared
        s.set(0, 0, true);
        assert_eq!(s.buf[0], 0xfe);
        assert!(s.get(0, 0));
        assert!(!s.get(0, 1));

        s.set(0, 0, false);
        assert_eq!(s.buf[0], 0xff);
        assert!(!s.get(0, 0));

        let (hi, lo) = s.packets();
        assert_eq!(&hi[..9], &screen::HEADER_HI);
        assert_eq!(&hi[9..], &s.buf[..256]);
        assert_eq!(&lo[..9], &screen::HEADER_LO);
        assert_eq!(&lo[9..], &s.buf[256..]);
    }
}
