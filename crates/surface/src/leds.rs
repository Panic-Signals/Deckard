//! Light bytes for the scripts' `deckard.led`.

use deckard_mk3::leds::{self, BACKLIGHT, Leds};

/// Full brightness for a one-colour button.
pub const ON: u8 = 0x7f;

/// Before the scripts set any: buttons at their faint backlight (so their labels read),
/// pads, groups and the strip dark.
pub fn blank() -> Leds {
    let mut out = Leds::default();
    for name in leds::BUTTON_LEDS {
        if !leds::is_colour(name) {
            out.set(name, BACKLIGHT);
        }
    }
    out
}

/// The palette byte nearest a colour (0..255 a channel), bright.
pub fn bright(rgb: [u8; 3]) -> u8 {
    // A full shade of the colour, whatever its brightness in the software.
    let max = rgb.iter().copied().max().unwrap_or(0).max(1) as u16;
    let [r, g, b] = rgb.map(|c| (c as u16 * 127 / max) as u8);
    if rgb == [0, 0, 0] { leds::palette(16, 4) } else { leds::palette_rgb(r, g, b) }
}

/// The same colour, dim.
pub fn dim(rgb: [u8; 3]) -> u8 {
    bright(rgb) & !3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Surface;
    use crate::tests_support::view;
    use deckard_mk3::Event;
    use std::time::Instant;

    fn byte(l: &Leds, name: &str) -> u8 {
        if let Some(i) = leds::BUTTON_LEDS.iter().position(|n| *n == name) {
            l.buttons[1 + i]
        } else {
            l.pads[1 + leds::PAD_LEDS.iter().position(|n| *n == name).expect(name)]
        }
    }

    #[test]
    fn transport_lights() {
        let (s, mut v) = (Surface::new(), view());
        let l = s.leds(&v);
        assert_eq!(s.error(), None);
        // (Stopped, PLAY and REC keep their backlight, as Maschine's.)
        assert_eq!(byte(&l, "LED_TRANSPORT_PLAY"), BACKLIGHT);
        assert_eq!(byte(&l, "LED_TRANSPORT_RECORD"), BACKLIGHT);
        v.playing = true;
        v.recording = true;
        v.auto_write = true;
        let l = s.leds(&v);
        assert_eq!(byte(&l, "LED_TRANSPORT_PLAY"), ON);
        assert_eq!(byte(&l, "LED_TRANSPORT_RECORD"), ON);
        assert_eq!(byte(&l, "LED_AUTO_WRITE"), ON);
    }

    #[test]
    fn groups_light_in_their_colours() {
        let (s, v) = (Surface::new(), view());
        let l = s.leds(&v);
        assert_eq!(byte(&l, "LED_GROUP_A"), leds::palette(1, 4), "focused: bright, its colour");
        assert_eq!(byte(&l, "LED_GROUP_B"), leds::palette(1, 2), "the other: dim");
        assert_eq!(byte(&l, "LED_GROUP_C"), leds::palette(16, 2), "after the last: dim white, a new group");
        assert_eq!(byte(&l, "LED_GROUP_D"), 0, "no group");
    }

    #[test]
    fn pads_light_by_sound() {
        let (mut s, v) = (Surface::new(), view());
        let l = s.leds(&v);
        assert_eq!(byte(&l, "LED_PAD_1"), leds::palette(0, 4), "focused sound bright");
        assert_eq!(byte(&l, "LED_PAD_2"), leds::palette(0, 2), "others dim");
        assert_eq!(byte(&l, "LED_PAD_9"), 0, "empty sounds off");
        s.event(&Event::PadOn(2, 1.0), &v, Instant::now());
        assert_eq!(byte(&s.leds(&v), "LED_PAD_2"), leds::palette(0, 4), "held: bright");
    }

    #[test]
    fn held_mute_shows_muted_pads() {
        let (mut s, mut v) = (Surface::new(), view());
        v.groups[0].sounds[1].muted = true;
        s.event(&Event::Button("BUTTON_MUTE", true), &v, Instant::now());
        let l = s.leds(&v);
        assert_eq!(byte(&l, "LED_PAD_2"), leds::palette(0, 2), "muted: dim");
        assert_eq!(byte(&l, "LED_PAD_3"), leds::palette(0, 4), "playing: bright");
        assert_eq!(byte(&l, "LED_MUTE"), ON);
    }

    #[test]
    fn keyboard_lights_the_roots() {
        let (mut s, v) = (Surface::new(), view());
        s.event(&Event::Button("BUTTON_KEYBOARD", true), &v, Instant::now());
        let l = s.leds(&v);
        assert_eq!(byte(&l, "LED_PAD_1"), leds::palette(0, 4), "C3");
        assert_eq!(byte(&l, "LED_PAD_13"), leds::palette(0, 4), "C4");
        assert_eq!(byte(&l, "LED_PAD_2"), leds::palette(0, 2));
        assert_eq!(byte(&l, "LED_KEYBOARD"), ON);
    }
}
