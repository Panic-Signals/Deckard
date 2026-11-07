//! The three LED reports. One-colour lights take a brightness, 0..0x7f. Colour lights take a
//! byte from the JAM's own palette (read from its feature reports, docs/jam-usb.md):
//! `(colour + 1) × 4 + shade`, colour 0 red … 15 fuchsia, 16 white.

use crate::map::{BOTTOM_FIRST, BUTTON_LEDS, GROUP_LEDS, METER_HEIGHT, METER_LEDS, PAD_LEDS, SCENE_LEDS, STRIP_LEDS};

/// Full brightness of a one-colour light.
pub const ON: u8 = 0x7f;
/// A button that can be pressed but isn't selected.
pub const DIM: u8 = 0x22;
/// A button that's off keeps a faint light, so its label can be read in the dark.
pub const BACKLIGHT: u8 = 0x09;

/// The palette's shades: dimmest, half-lit (the manual's pattern not playing), full, and a
/// paler full (the JAM's own table has it as the brightest).
pub const SHADE_FAINT: u8 = 0;
pub const SHADE_DIM: u8 = 1;
pub const SHADE_BRIGHT: u8 = 2;
pub const SHADE_PALE: u8 = 3;

/// The colour numbers there are: 0..=15, and white.
pub const WHITE: i64 = 16;

/// A colour LED's byte.
pub fn colour(palette: i64, shade: u8) -> u8 {
    let palette = if (0..=WHITE).contains(&palette) { palette } else { WHITE };
    ((palette as u8 + 1) << 2) | (shade & 3)
}

/// What every LED should show, as the reports go out (each with its id first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leds {
    pub buttons: [u8; 54],
    pub pads: [u8; 81],
    pub strips: [u8; 89],
}

impl Default for Leds {
    /// All dark.
    fn default() -> Leds {
        let mut l = Leds { buttons: [0; 54], pads: [0; 81], strips: [0; 89] };
        l.buttons[0] = 0x80;
        l.pads[0] = 0x81;
        l.strips[0] = 0x82;
        l
    }
}

impl Leds {
    /// Every button at its backlight, the colours dark: what a frame starts from.
    pub fn blank() -> Leds {
        let mut l = Leds::default();
        l.buttons[1..=BUTTON_LEDS.len()].fill(BACKLIGHT);
        l
    }

    /// One-colour button `name`'s brightness; `false` if there's no such light.
    pub fn button(&mut self, name: &str, brightness: u8) -> bool {
        let Some(i) = BUTTON_LEDS.iter().position(|b| *b == name) else { return false };
        self.buttons[1 + i] = brightness.min(ON);
        true
    }

    /// Scene button `n` (from 0).
    pub fn scene(&mut self, n: usize, byte: u8) {
        if n < 8 {
            self.pads[1 + SCENE_LEDS + n] = byte;
        }
    }

    /// Group button `n` (from 0).
    pub fn group(&mut self, n: usize, byte: u8) {
        if n < 8 {
            self.pads[1 + GROUP_LEDS + n] = byte;
        }
    }

    /// The pad at `column`, `row` (from 0, the top left first).
    pub fn pad(&mut self, column: usize, row: usize, byte: u8) {
        if column < 8 && row < 8 {
            self.pads[1 + PAD_LEDS + row * 8 + column] = byte;
        }
    }

    /// LED `led` (from 0, the bottom one first) of strip `strip` (from 0).
    pub fn strip(&mut self, strip: usize, led: usize, byte: u8) {
        if strip < 8 && led < STRIP_LEDS {
            let led = if BOTTOM_FIRST { led } else { STRIP_LEDS - 1 - led };
            self.strips[1 + strip * STRIP_LEDS + led] = byte;
        }
    }

    /// LED `led` (from 0, the bottom one first) of meter column `column` (0 left, 1 right).
    pub fn meter(&mut self, column: usize, led: usize, brightness: u8) {
        if column < 2 && led < METER_HEIGHT {
            let led = if BOTTOM_FIRST { led } else { METER_HEIGHT - 1 - led };
            self.buttons[1 + METER_LEDS + column * METER_HEIGHT + led] = brightness.min(ON);
        }
    }

    /// The one-colour lights at `level` percent, as Preferences' LED brightness has them;
    /// what's lit stays lit, and the colours stay as they are (the palette has no dimmer).
    pub fn scaled(&self, level: u8) -> Leds {
        let mut out = self.clone();
        if level >= 100 {
            return out;
        }
        for b in &mut out.buttons[1..] {
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
    fn colours_are_the_palette_bytes() {
        assert_eq!(colour(0, SHADE_FAINT), 4, "red, as the JAM's table starts");
        assert_eq!(colour(0, SHADE_BRIGHT), 6);
        assert_eq!(colour(WHITE, SHADE_BRIGHT), 70);
        assert_eq!(colour(99, SHADE_BRIGHT), 70, "out of range is white");
    }

    #[test]
    fn each_light_lands_on_its_byte() {
        let mut l = Leds::default();
        l.scene(1, 9);
        l.pad(0, 0, 8);
        l.pad(7, 7, 7);
        l.group(7, 6);
        assert_eq!((l.pads[2], l.pads[9], l.pads[72], l.pads[80]), (9, 8, 7, 6));
        assert!(l.button("SONG", 0x40));
        assert_eq!(l.buttons[1], 0x40);
        assert!(!l.button("NOPE", 1));
        l.strip(1, 0, 5);
        assert_eq!(l.strips[1 + 11], 5);
        l.meter(1, 7, 3);
        assert_eq!(l.buttons[53], 3);
        assert_eq!(l.buttons.len() - 1, 53);
        assert_eq!(l.pads.len() - 1, 80);
        assert_eq!(l.strips.len() - 1, 88);
    }

    #[test]
    fn places_off_the_controller_are_ignored() {
        let mut l = Leds::default();
        let dark = l.clone();
        l.scene(8, 1);
        l.group(8, 1);
        l.pad(8, 0, 1);
        l.strip(8, 0, 1);
        l.strip(0, 11, 1);
        l.meter(2, 0, 1);
        assert_eq!(l, dark);
    }

    #[test]
    fn blank_backlights_the_buttons_only() {
        let l = Leds::blank();
        assert_eq!(l.buttons[1], BACKLIGHT);
        assert_eq!(l.buttons[1 + METER_LEDS], 0, "meters dark");
        assert!(l.pads[1..].iter().all(|b| *b == 0));
    }

    #[test]
    fn scaling_dims_buttons_and_keeps_colours() {
        let mut l = Leds::blank();
        l.button("PLAY", ON);
        l.pad(0, 0, colour(1, SHADE_BRIGHT));
        let half = l.scaled(50);
        assert_eq!(half.buttons[1 + BUTTON_LEDS.iter().position(|b| *b == "PLAY").unwrap()], 64);
        assert_eq!(half.pads, l.pads);
        assert_eq!(l.scaled(100), l);
        assert!(l.scaled(1).buttons[1] >= 1, "lit stays lit");
    }
}
