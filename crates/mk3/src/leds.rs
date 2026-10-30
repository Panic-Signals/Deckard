//! The MK3's LEDs (learned with `examples/mk3-led-learn.rs`): output report 0x80 holds
//! the buttons, 0x81 the touch strip and pads, one byte an LED, named here as the
//! scripts name them (`NI.HW.<name>`).
//!
//! Single-colour LEDs take a brightness, 0..0x7f. Colour LEDs (groups, pads, touch
//! strip, the 4-D encoder's arrows) take a palette byte: colour 1..17 (the scripts'
//! `LEDColors` index + 1, 17 white) times four, plus 0 dim, 1 dim flash, 2 bright,
//! 3 flash.

/// Report 0x80, bytes 1..=62.
pub const BUTTON_LEDS: [&str; 62] = [
    "LED_CHANNEL",
    "LED_PLUGIN",
    "LED_ARRANGE",
    "LED_MIX",
    "LED_BROWSE",
    "LED_SAMPLE",
    "LED_LEFT",
    "LED_RIGHT",
    "LED_FILE",
    "LED_SETTINGS",
    "LED_AUTO_WRITE",
    "LED_MACRO",
    "LED_SCREEN_BUTTON_1",
    "LED_SCREEN_BUTTON_2",
    "LED_SCREEN_BUTTON_3",
    "LED_SCREEN_BUTTON_4",
    "LED_SCREEN_BUTTON_5",
    "LED_SCREEN_BUTTON_6",
    "LED_SCREEN_BUTTON_7",
    "LED_SCREEN_BUTTON_8",
    "LED_VOLUME",
    "LED_SWING",
    "LED_NOTE_REPEAT",
    "LED_TEMPO",
    "LED_LOCK",
    "LED_PITCH",
    "LED_MOD",
    "LED_PERFORM",
    "LED_NOTES",
    "LED_GROUP_A",
    "LED_GROUP_B",
    "LED_GROUP_C",
    "LED_GROUP_D",
    "LED_GROUP_E",
    "LED_GROUP_F",
    "LED_GROUP_G",
    "LED_GROUP_H",
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
    // The 4-D encoder's arrows: up, left, right, down (checked on the hardware, lighting
    // them through the Mixer's channels).
    "LED_WHEEL_BUTTON_UP",
    "LED_WHEEL_BUTTON_LEFT",
    "LED_WHEEL_BUTTON_RIGHT",
    "LED_WHEEL_BUTTON_DOWN",
];

/// Report 0x81, bytes 1..=41: the touch strip left to right, then the pads from the
/// top row.
pub const PAD_LEDS: [&str; 41] = [
    "LED_TS_01",
    "LED_TS_02",
    "LED_TS_03",
    "LED_TS_04",
    "LED_TS_05",
    "LED_TS_06",
    "LED_TS_07",
    "LED_TS_08",
    "LED_TS_09",
    "LED_TS_10",
    "LED_TS_11",
    "LED_TS_12",
    "LED_TS_13",
    "LED_TS_14",
    "LED_TS_15",
    "LED_TS_16",
    "LED_TS_17",
    "LED_TS_18",
    "LED_TS_19",
    "LED_TS_20",
    "LED_TS_21",
    "LED_TS_22",
    "LED_TS_23",
    "LED_TS_24",
    "LED_TS_25",
    "LED_PAD_13",
    "LED_PAD_14",
    "LED_PAD_15",
    "LED_PAD_16",
    "LED_PAD_9",
    "LED_PAD_10",
    "LED_PAD_11",
    "LED_PAD_12",
    "LED_PAD_5",
    "LED_PAD_6",
    "LED_PAD_7",
    "LED_PAD_8",
    "LED_PAD_1",
    "LED_PAD_2",
    "LED_PAD_3",
    "LED_PAD_4",
];

/// Whether an LED takes a palette byte rather than a brightness.
pub fn is_colour(name: &str) -> bool {
    // The pads are LED_PAD_1..16; LED_PAD_MODE is a one-colour button.
    let pad = name.strip_prefix("LED_PAD_").is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()));
    pad || name.starts_with("LED_GROUP_") || name.starts_with("LED_TS_") || name.starts_with("LED_WHEEL_BUTTON_")
}

/// The palette byte of a scripts' colour (`LEDColors` index, 16 white) and LED state
/// (`LEDHelper.LS_*`: 1 off, 2 dim, 3 dim flash, 4 bright, 5 flash). The low two bits
/// are the shade, 0 dimmest to 3 brightest; Maschine's MK3 table gives bright, dim flash
/// and flash the full level (`MaschineStudioController.LEDValues`), so they take 3.
pub fn palette(colour: i64, state: i64) -> u8 {
    if state <= 1 {
        return 0;
    }
    let colour = if (0..=16).contains(&colour) { colour } else { 16 };
    // LS_DIM and LS_DIM_FLASH (dim until something flashes it) are dim.
    let shade = if state <= 3 { 0 } else { 3 };
    ((colour as u8 + 1) << 2) | shade
}

/// A button LED's brightness for an LED state (`LEDHelper.LS_*`): off buttons keep a
/// faint backlight so their labels can be read, dim ones (LS_DIM, and LS_DIM_FLASH, dim
/// until something flashes it: SAMPLE while its page is hidden) a little more, the
/// rest full.
pub fn button_level(state: i64) -> u8 {
    match state {
        ..=1 => BACKLIGHT,
        2 | 3 => DIM,
        _ => 0x7f,
    }
}

/// The faint light of a button that's off.
pub const BACKLIGHT: u8 = 0x09;

/// A dim button: one that can be pressed but isn't selected.
pub const DIM: u8 = 0x22;

/// A one-colour button's brightness for a level the scripts set (0..1): full stays
/// full; anything less is a dim button, 30% under the level (dim is 0.25 on the MK3),
/// but never below the backlight.
pub fn button_brightness(level: f64) -> u8 {
    let b = brightness(level);
    if b >= 0x7f { b } else { ((b as f64 * 0.7).round() as u8).max(BACKLIGHT) }
}

/// The palette byte nearest an RGB colour (0..127 a channel): its hue picks the
/// colour, its brightness dim or bright.
pub fn palette_rgb(r: u8, g: u8, b: u8) -> u8 {
    let (r, g, b) = (r as f64 / 127.0, g as f64 / 127.0, b as f64 / 127.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max < 0.05 {
        return 0;
    }
    let state = if max > 0.6 { 4 } else { 2 };
    if max - min < 0.2 * max {
        return palette(16, state);
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
    const HUES: [f64; 16] = [0.0, 6.0, 23.0, 35.0, 52.0, 78.0, 120.0, 135.0, 177.0, 212.0, 240.0, 258.0, 280.0, 331.0, 346.0, 355.0];
    let dist = |a: f64| (h - a).abs().min(360.0 - (h - a).abs());
    let colour = (0..16).min_by(|&a, &b| dist(HUES[a]).total_cmp(&dist(HUES[b]))).unwrap_or(0);
    palette(colour as i64, state)
}

/// The brightness byte of a level 0..1.
pub fn brightness(level: f64) -> u8 {
    (level.clamp(0.0, 1.0) * 127.0).round() as u8
}

/// Both LED reports.
#[derive(Clone, PartialEq, Eq)]
pub struct Leds {
    pub buttons: [u8; 63],
    pub pads: [u8; 42],
}

impl Default for Leds {
    fn default() -> Leds {
        let mut l = Leds { buttons: [0; 63], pads: [0; 42] };
        l.buttons[0] = 0x80;
        l.pads[0] = 0x81;
        l
    }
}

impl Leds {
    /// Set an LED by name (its byte: brightness or palette, see [`is_colour`]).
    pub fn set(&mut self, name: &str, byte: u8) -> bool {
        if let Some(i) = BUTTON_LEDS.iter().position(|n| *n == name) {
            self.buttons[1 + i] = byte;
        } else if let Some(i) = PAD_LEDS.iter().position(|n| *n == name) {
            self.pads[1 + i] = byte;
        } else {
            return false;
        }
        true
    }

    /// The one-colour LEDs at `level` % of their brightness (a lit one stays lit); the
    /// colour LEDs, which take a palette byte, as they are.
    pub fn scaled(&self, level: u8) -> Leds {
        let mut out = self.clone();
        if level >= 100 {
            return out;
        }
        let dim = |b: u8| if b == 0 { 0 } else { ((u16::from(b) * u16::from(level) + 99) / 100).max(1) as u8 };
        for (i, name) in BUTTON_LEDS.iter().enumerate() {
            if !is_colour(name) {
                out.buttons[1 + i] = dim(self.buttons[1 + i]);
            }
        }
        out
    }

    /// An LED's byte by name.
    pub fn get(&self, name: &str) -> Option<u8> {
        if let Some(i) = BUTTON_LEDS.iter().position(|n| *n == name) {
            Some(self.buttons[1 + i])
        } else {
            PAD_LEDS.iter().position(|n| *n == name).map(|i| self.pads[1 + i])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_bytes() {
        assert_eq!(palette(0, 1), 0, "off");
        assert!(is_colour("LED_PAD_16") && !is_colour("LED_PAD_MODE"), "PAD MODE is a button, not a pad");
        assert_eq!(palette(0, 2), 0x04, "red dim");
        assert_eq!(palette(16, 3), palette(16, 2), "dim until flashed: dim");
        assert_eq!(button_level(3), DIM);
        assert_eq!(palette(0, 4), 0x07, "red bright");
        assert_eq!(palette(16, 4), 0x47, "white bright");
        assert_eq!(palette_rgb(127, 0, 0), palette(0, 4));
        assert_eq!(palette_rgb(0, 0, 127), palette(10, 4));
        assert_eq!(palette_rgb(32, 32, 32), palette(16, 2));
    }

    #[test]
    fn scaled_dims_the_one_colour_leds() {
        let mut l = Leds::default();
        l.set("LED_TRANSPORT_PLAY", 0x7f);
        l.set("LED_SHIFT", BACKLIGHT);
        l.set("LED_GROUP_A", palette(0, 4));
        l.set("LED_PAD_1", palette(3, 4));
        let half = l.scaled(50);
        assert_eq!(half.get("LED_TRANSPORT_PLAY"), Some(0x40));
        assert_eq!(half.get("LED_SHIFT"), Some(5), "the backlight dims too");
        assert_eq!((half.get("LED_GROUP_A"), half.get("LED_PAD_1")), (l.get("LED_GROUP_A"), l.get("LED_PAD_1")), "colours kept");
        assert_eq!(l.scaled(1).get("LED_SHIFT"), Some(1), "lit stays lit");
        assert!(l.scaled(100) == l);
    }

    #[test]
    fn every_led_has_a_byte() {
        let mut l = Leds::default();
        assert!(l.set("LED_PAD_1", 5));
        assert_eq!(l.pads[38], 5, "pad 1 is bottom left: byte 38");
        assert!(l.set("LED_TRANSPORT_PLAY", 0x7f));
        assert_eq!(l.buttons[42], 0x7f);
        assert!(!l.set("LED_NOPE", 1));
    }
}
