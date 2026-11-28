//! LED output for the Maschine Mikro MK3.
//!
//! All LEDs are written in a single HID output report 0x80 (81 bytes):
//!
//! ```text
//! byte  0:       report id 0x80
//! bytes  1– 39:  39 button LEDs   (single-channel brightness byte each)
//! bytes 40– 55:  16 pad LEDs      (colour palette byte each, row-by-row top to bottom: 13..16, 9..12, 5..8, 1..4)
//! bytes 56– 80:  25 touch-strip segment LEDs (single-channel brightness byte each, left to right)
//! ```
//!
//! # Brightness levels (single-colour LEDs)
//!
//! The hardware renders three visible levels, matching the full MK3:
//!
//! | constant    | value  | appearance                              |
//! |-------------|--------|-----------------------------------------|
//! | [`BACKLIGHT`] | `0x09` | Faint — label readable in the dark     |
//! | [`DIM`]       | `0x22` | Medium — a pressable-but-unselected button |
//! | [`ON`]        | `0x7f` | Bright — the selected / active state    |
//! | [`OFF`]       | `0x00` | Dark — no light at all                  |
//!
//! # Colour LEDs (pads)
//!
//! On the Mikro MK3, only the 16 pads are RGB LEDs. The touch-strip segments and
//! buttons are single-colour white LEDs taking brightness bytes.
//!
//! Pad LEDs take a palette byte: `(colour + 1) << 2 | shade`.
//! - `colour` 0 = Red … 15 = Fuchsia, 16 = White (matches the MK3/Jam LEDColors table).
//! - `shade`  0 = dim, 2 = bright, 3 = flash (pale bright).
//!
//! Use [`palette`] or [`colour`] to compute the byte, or [`palette_rgb`] to pick the nearest
//! palette entry to an RGB colour.

/// Faint backlight — button label readable in the dark, button logically off.
/// Matches [`deckard_mk3::leds::BACKLIGHT`] and [`deckard_jam::leds::BACKLIGHT`].
pub const BACKLIGHT: u8 = 0x09;

/// Medium brightness — a button that can be pressed but is not selected.
pub const DIM: u8 = 0x22;

/// Full brightness — the selected or active state.
pub const ON: u8 = 0x7f;

/// No light at all. Use when a button should be completely dark.
pub const OFF: u8 = 0x00;

// Pad colour shade values (low 2 bits of palette byte).
pub const SHADE_DIM:    u8 = 0;
pub const SHADE_BRIGHT: u8 = 2;
pub const SHADE_FLASH:  u8 = 3;

/// White palette index (same value as in deckard-mk3 and deckard-jam).
pub const WHITE: i64 = 16;

/// Encode a colour LED byte from palette index and shade.
///
/// - `palette`: 0 = Red … 15 = Fuchsia, 16 = White. Out-of-range → White.
/// - `shade`:   use [`SHADE_DIM`], [`SHADE_BRIGHT`], or [`SHADE_FLASH`].
///
/// Returns `0x00` (off) when `palette < 0`.
pub fn colour(palette: i64, shade: u8) -> u8 {
    if palette < 0 {
        return 0;
    }
    let p = if (0..=WHITE).contains(&palette) { palette } else { WHITE };
    ((p as u8 + 1) << 2) | (shade & 3)
}

/// The palette byte of a scripts' colour (`LEDColors` index, 16 white) and LED state
/// (`LEDHelper.LS_*`: 1 off, 2 dim, 3 dim flash, 4 bright, 5 flash).
///
/// Fully compatible with [`deckard_mk3::leds::palette`].
pub fn palette(colour: i64, state: i64) -> u8 {
    if state <= 1 {
        return 0;
    }
    let colour = if (0..=WHITE).contains(&colour) { colour } else { WHITE };
    let shade = match state {
        2 | 3 => SHADE_DIM,
        4     => SHADE_BRIGHT,
        _     => SHADE_FLASH,
    };
    ((colour as u8 + 1) << 2) | shade
}

/// A button LED's byte for an LED state (`LEDHelper.LS_*`): off buttons keep a faint
/// [`BACKLIGHT`] so their labels can be read, dim ones take [`DIM`], active ones take [`ON`].
pub fn button_level(state: i64) -> u8 {
    match state {
        ..=1 => BACKLIGHT,
        2 | 3 => DIM,
        _ => ON,
    }
}

/// A one-colour button's byte for a level the scripts set (0.0..=1.0): full stays full;
/// anything less is dim, scaled but never below the backlight.
pub fn button_brightness(level: f64) -> u8 {
    let b = (level.clamp(0.0, 1.0) * 127.0).round() as u8;
    if b >= ON { b } else { ((b as f64 * 0.7).round() as u8).max(BACKLIGHT) }
}

/// The palette byte nearest an RGB colour (0..127 per channel): its hue picks the colour,
/// its brightness dim or bright.
pub fn palette_rgb(r: u8, g: u8, b: u8) -> u8 {
    let (r, g, b) = (r as f64 / 127.0, g as f64 / 127.0, b as f64 / 127.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max < 0.05 {
        return 0;
    }
    let state = if max > 0.6 { 4 } else { 2 };
    if max - min < 0.2 * max {
        return palette(WHITE, state);
    }
    let d = max - min;
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    // The hues of LEDColors 0..15, red round to fuchsia.
    const HUES: [f64; 16] = [
        0.0, 6.0, 23.0, 35.0, 52.0, 78.0, 120.0, 135.0, 177.0, 212.0, 240.0, 258.0, 280.0, 331.0, 346.0, 355.0,
    ];
    let dist = |a: f64| (h - a).abs().min(360.0 - (h - a).abs());
    let colour = (0..16).min_by(|&a, &b| dist(HUES[a]).total_cmp(&dist(HUES[b]))).unwrap_or(0);
    palette(colour as i64, state)
}

/// Named LED slots in the button array (byte offsets within [`Leds::buf`] bytes 1–39).
/// Order matches the hardware register order (status bytes 0..=38).
pub const BUTTON_LEDS: [&str; 39] = [
    "LED_MASCHINE",
    "LED_STAR",
    "LED_BROWSE",
    "LED_VOLUME",
    "LED_SWING",
    "LED_TEMPO",
    "LED_PLUGIN",
    "LED_SAMPLING",
    "LED_LEFT",
    "LED_RIGHT",
    "LED_PITCH",
    "LED_MOD",
    "LED_PERFORM",
    "LED_NOTES",
    "LED_GROUP",
    "LED_AUTO",
    "LED_LOCK",
    "LED_NOTE_REPEAT",
    "LED_TRANSPORT_LOOP",
    "LED_TRANSPORT_ERASE",
    "LED_TAP",
    "LED_FOLLOW",
    "LED_TRANSPORT_PLAY",
    "LED_TRANSPORT_RECORD",
    "LED_TRANSPORT_STOP",
    "LED_SHIFT",
    "LED_FIX_VELOCITY",
    "LED_PAD_MODE",
    "LED_KEYBOARD",
    "LED_CHORDS",
    "LED_STEP",
    "LED_SCENE",
    "LED_PATTERN",
    "LED_EVENTS",
    "LED_VARIATION",
    "LED_DUPLICATE",
    "LED_SELECT",
    "LED_SOLO",
    "LED_MUTE",
    // Note: ENCODER_PRESS and ENCODER_TOUCH are inputs only — no LED slots.
];

/// Named LED slots for the 16 pads (bytes 40–55 of the report).
///
/// Native Instruments hardware registers pads row-by-row, top to bottom:
/// pads 13–16 (top row), 9–12, 5–8, 1–4 (bottom row).
/// Matches [`deckard_mk3::leds::PAD_LEDS`].
pub const PAD_LEDS: [&str; 16] = [
    "LED_PAD_13", "LED_PAD_14", "LED_PAD_15", "LED_PAD_16",
    "LED_PAD_9",  "LED_PAD_10", "LED_PAD_11", "LED_PAD_12",
    "LED_PAD_5",  "LED_PAD_6",  "LED_PAD_7",  "LED_PAD_8",
    "LED_PAD_1",  "LED_PAD_2",  "LED_PAD_3",  "LED_PAD_4",
];

/// Named LED slots for the 25 touch-strip segments (bytes 56–80 of the report), left to right.
pub const STRIP_LEDS: [&str; 25] = [
    "LED_TS_01", "LED_TS_02", "LED_TS_03", "LED_TS_04", "LED_TS_05",
    "LED_TS_06", "LED_TS_07", "LED_TS_08", "LED_TS_09", "LED_TS_10",
    "LED_TS_11", "LED_TS_12", "LED_TS_13", "LED_TS_14", "LED_TS_15",
    "LED_TS_16", "LED_TS_17", "LED_TS_18", "LED_TS_19", "LED_TS_20",
    "LED_TS_21", "LED_TS_22", "LED_TS_23", "LED_TS_24", "LED_TS_25",
];

/// Normalize alias names to canonical names used in BUTTON_LEDS.
fn normalize_button_name(name: &str) -> &str {
    match name {
        "LED_RESTART" => "LED_TRANSPORT_LOOP",
        "LED_ERASE" => "LED_TRANSPORT_ERASE",
        "LED_AUTO_WRITE" => "LED_AUTO",
        "LED_SAMPLE" => "LED_SAMPLING",
        "LED_FIXED_VEL" => "LED_FIX_VELOCITY",
        "LED_PLAY" => "LED_TRANSPORT_PLAY",
        "LED_REC" => "LED_TRANSPORT_RECORD",
        "LED_STOP" => "LED_TRANSPORT_STOP",
        other => other,
    }
}

/// Returns true if the LED name is an RGB pad LED.
///
/// On the Mikro MK3, only the 16 pads (`LED_PAD_1`..=16) are colour LEDs.
/// `LED_PAD_MODE` is a one-colour button LED. Touch strip segments are single-colour white LEDs.
pub fn is_colour(name: &str) -> bool {
    name.strip_prefix("LED_PAD_").is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
}

/// Convert a 1-based pad number (1–16, bottom-left first) to its 0-based offset in PAD_LEDS (0–15).
pub fn pad_number_to_index(pad: usize) -> Option<usize> {
    if (1..=16).contains(&pad) {
        let row_from_bottom = (pad - 1) / 4;
        let col = (pad - 1) % 4;
        let row_from_top = 3 - row_from_bottom;
        Some(row_from_top * 4 + col)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Leds
// ---------------------------------------------------------------------------

/// The full LED state for one frame, stored as the raw 81-byte HID report 0x80.
///
/// Build a frame with the setter methods, then call [`Mikro::show`](crate::Mikro::show) to send it.
/// Only reports that differ from the last flush go over USB (same as deckard-jam).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leds {
    /// buf[0] = 0x80 (report id); buf[1..=80] = LED data.
    pub buf: [u8; 81],
}

impl Default for Leds {
    /// All LEDs off (dark). The report id byte is preset to 0x80.
    fn default() -> Self {
        let mut l = Leds { buf: [0u8; 81] };
        l.buf[0] = 0x80;
        l
    }
}

impl Leds {
    /// Every button at its backlight level, pads and strip dark.
    ///
    /// A good starting point for a frame: buttons are faintly lit so their labels
    /// read in the dark, matching the MK3 and Jam conventions.
    pub fn blank() -> Self {
        let mut l = Leds::default();
        l.buf[1..=BUTTON_LEDS.len()].fill(BACKLIGHT);
        l
    }

    // ---- Button LEDs (bytes 1–39) ----------------------------------------

    /// Set a button LED by name. Returns `false` if the name is not found.
    ///
    /// `brightness`: use [`OFF`], [`BACKLIGHT`], [`DIM`], or [`ON`].
    pub fn button(&mut self, name: &str, brightness: u8) -> bool {
        let norm = normalize_button_name(name);
        let Some(i) = BUTTON_LEDS.iter().position(|n| *n == norm) else {
            return false;
        };
        self.buf[1 + i] = brightness;
        true
    }

    /// Read a button LED's current brightness byte.
    pub fn get_button(&self, name: &str) -> Option<u8> {
        let norm = normalize_button_name(name);
        BUTTON_LEDS.iter().position(|n| *n == norm).map(|i| self.buf[1 + i])
    }

    // ---- Pad LEDs (bytes 40–55) ------------------------------------------

    /// Set pad `n` (1-based, 1–16, bottom-left first) to a colour byte.
    /// Use [`palette`], [`colour`], or [`palette_rgb`] to build the byte.
    pub fn pad(&mut self, n: usize, byte: u8) {
        if let Some(idx) = pad_number_to_index(n) {
            self.buf[40 + idx] = byte;
        }
    }

    /// Set pad `n` to off (dark).
    pub fn pad_off(&mut self, n: usize) {
        self.pad(n, 0);
    }

    // ---- Touch-strip segment LEDs (bytes 56–80) --------------------------

    /// Set strip segment `seg` (0-based, 0–24, left to right) to `brightness`.
    pub fn strip_segment(&mut self, seg: usize, brightness: u8) {
        if seg < 25 {
            self.buf[56 + seg] = brightness;
        }
    }

    /// Light strip segments 0..`count` at `brightness`; turn the rest off.
    ///
    /// Useful for displaying a bar (e.g. encoder position, volume meter, strip touch point).
    pub fn strip_level(&mut self, count: usize, brightness: u8) {
        for seg in 0..25 {
            self.buf[56 + seg] = if seg < count { brightness } else { OFF };
        }
    }

    /// Light a single strip segment (0..24) at `brightness`; turn the rest off.
    ///
    /// Useful for displaying a dot indicator.
    pub fn strip_dot(&mut self, index: usize, brightness: u8) {
        for seg in 0..25 {
            self.buf[56 + seg] = if seg == index { brightness } else { OFF };
        }
    }

    // ---- Generic set/get by name -----------------------------------------

    /// Set any LED by its named constant (`LED_PLAY`, `LED_PAD_1`, `LED_TS_01`, etc.).
    /// Returns `false` if the name is unrecognised.
    pub fn set(&mut self, name: &str, byte: u8) -> bool {
        let norm = normalize_button_name(name);
        if let Some(i) = BUTTON_LEDS.iter().position(|n| *n == norm) {
            self.buf[1 + i] = byte;
        } else if let Some(i) = PAD_LEDS.iter().position(|n| *n == name) {
            self.buf[40 + i] = byte;
        } else if let Some(i) = STRIP_LEDS.iter().position(|n| *n == name) {
            self.buf[56 + i] = byte;
        } else {
            return false;
        }
        true
    }

    /// Read any LED by name.
    pub fn get(&self, name: &str) -> Option<u8> {
        let norm = normalize_button_name(name);
        if let Some(i) = BUTTON_LEDS.iter().position(|n| *n == norm) {
            Some(self.buf[1 + i])
        } else if let Some(i) = PAD_LEDS.iter().position(|n| *n == name) {
            Some(self.buf[40 + i])
        } else {
            STRIP_LEDS.iter().position(|n| *n == name).map(|i| self.buf[56 + i])
        }
    }

    /// Return a copy of these LEDs with button and strip brightness scaled to `level` percent (0..=100).
    /// RGB pad LEDs are left untouched.
    pub fn scaled(&self, level: u8) -> Leds {
        let mut out = self.clone();
        if level >= 100 {
            return out;
        }
        for b in &mut out.buf[1..=39] {
            if *b != 0 {
                *b = ((u16::from(*b) * u16::from(level) + 99) / 100).max(1) as u8;
            }
        }
        for b in &mut out.buf[56..=80] {
            if *b != 0 {
                *b = ((u16::from(*b) * u16::from(level) + 99) / 100).max(1) as u8;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_bytes_match_mk3() {
        assert_eq!(palette(0, 1), 0, "off");
        assert_eq!(palette(0, 2), 0x04, "red dim");
        assert_eq!(palette(0, 4), 0x06, "red bright");
        assert_eq!(palette(WHITE, 4), 0x46, "white bright");
        assert_eq!(palette(99, 4), 0x46, "out of range is white");
        assert_eq!(palette_rgb(127, 0, 0), palette(0, 4));
        assert_eq!(palette_rgb(0, 0, 127), palette(10, 4));
        assert_eq!(palette_rgb(32, 32, 32), palette(WHITE, 2));
    }

    #[test]
    fn button_levels() {
        assert_eq!(button_level(1), BACKLIGHT);
        assert_eq!(button_level(2), DIM);
        assert_eq!(button_level(4), ON);
        assert_eq!(button_brightness(1.0), ON);
        assert_eq!(button_brightness(0.0), BACKLIGHT);
    }

    #[test]
    fn is_colour_only_for_pads() {
        assert!(is_colour("LED_PAD_1"));
        assert!(is_colour("LED_PAD_16"));
        assert!(!is_colour("LED_PAD_MODE"), "PAD MODE is a button");
        assert!(!is_colour("LED_TRANSPORT_PLAY"));
        assert!(!is_colour("LED_TS_01"), "Mikro MK3 strip LEDs are white");
    }

    #[test]
    fn each_light_lands_on_its_byte() {
        let mut l = Leds::default();

        // First button
        assert!(l.set("LED_MASCHINE", ON));
        assert_eq!(l.buf[1], ON);

        // Last button
        assert!(l.set("LED_MUTE", DIM));
        assert_eq!(l.buf[39], DIM);

        // Pad 13 (top-left) -> report byte 40 (first pad slot)
        assert!(l.set("LED_PAD_13", 0x12));
        assert_eq!(l.buf[40], 0x12);
        l.pad(13, 0x34);
        assert_eq!(l.buf[40], 0x34);

        // Pad 16 (top-right) -> report byte 43
        l.pad(16, 0x56);
        assert_eq!(l.buf[43], 0x56);
        assert_eq!(l.get("LED_PAD_16"), Some(0x56));

        // Pad 1 (bottom-left) -> report byte 52
        assert!(l.set("LED_PAD_1", 0x78));
        assert_eq!(l.buf[52], 0x78);
        l.pad(1, 0x9a);
        assert_eq!(l.buf[52], 0x9a);
        assert_eq!(l.get("LED_PAD_1"), Some(0x9a));

        // Pad 4 (bottom-right) -> report byte 55 (last pad slot)
        l.pad(4, 0xbc);
        assert_eq!(l.buf[55], 0xbc);
        assert_eq!(l.get("LED_PAD_4"), Some(0xbc));

        // Strip first segment (leftmost) -> report byte 56
        assert!(l.set("LED_TS_01", ON));
        assert_eq!(l.buf[56], ON);
        l.strip_segment(0, DIM);
        assert_eq!(l.buf[56], DIM);

        // Strip last segment (rightmost) -> report byte 80
        assert!(l.set("LED_TS_25", ON));
        assert_eq!(l.buf[80], ON);
        l.strip_segment(24, DIM);
        assert_eq!(l.buf[80], DIM);

        // Aliases work
        assert!(l.set("LED_RESTART", ON));
        assert_eq!(l.get("LED_TRANSPORT_LOOP"), Some(ON));
        assert!(l.set("LED_ERASE", DIM));
        assert_eq!(l.get("LED_TRANSPORT_ERASE"), Some(DIM));
        assert!(l.set("LED_SAMPLE", ON));
        assert_eq!(l.get("LED_SAMPLING"), Some(ON));

        // Unrecognized name
        assert!(!l.set("LED_NONEXISTENT", 1));
        assert_eq!(l.get("LED_NONEXISTENT"), None);
    }

    #[test]
    fn strip_helpers() {
        let mut l = Leds::default();
        l.strip_level(5, ON);
        for i in 0..5 {
            assert_eq!(l.buf[56 + i], ON);
        }
        for i in 5..25 {
            assert_eq!(l.buf[56 + i], OFF);
        }

        l.strip_dot(10, DIM);
        for i in 0..25 {
            assert_eq!(l.buf[56 + i], if i == 10 { DIM } else { OFF });
        }
    }

    #[test]
    fn scaling_dims_single_colour_leaves_pads() {
        let mut l = Leds::default();
        l.set("LED_TRANSPORT_PLAY", 100);
        l.strip_segment(0, 100);
        l.pad(1, 0x46); // white bright

        let scaled = l.scaled(50);
        assert_eq!(scaled.buf[1 + 22], 50, "play button dimmed to 50%");
        assert_eq!(scaled.buf[56], 50, "strip segment dimmed to 50%");
        assert_eq!(scaled.buf[52], 0x46, "pad colour unaffected by brightness scaling");
    }
}
