//! Which bit is which control, and which byte which light.
//!
//! **Provisional.** The layout comes from the open-source cabl driver (MIT,
//! `src/devices/ni/MaschineJam.cpp`, github.com/shaduzlabs/cabl), checked against what one
//! unit reports: its 120 button bits, 37 + 16 + 80 + 88 LED bytes and two unused button LEDs
//! all match the measured descriptor (docs/jam-usb.md). Nobody has yet pressed each control
//! against this table. `examples/jam-check.rs` is how to do that, and a wrong name is a
//! one-line fix here.

/// The bits of report 0x01 before the pads, from the first after the encoder's byte.
const BEFORE_PADS: [&str; 18] = [
    "SONG",
    "SCENE_1",
    "SCENE_2",
    "SCENE_3",
    "SCENE_4",
    "SCENE_5",
    "SCENE_6",
    "SCENE_7",
    "SCENE_8",
    "STEP",
    "PAD_MODE",
    "CLEAR",
    "DUPLICATE",
    "UP",
    "LEFT",
    "RIGHT",
    "DOWN",
    "NOTE_REPEAT",
];

/// The first pad's bit. Then 64 pads by rows from the top left.
pub const FIRST_PAD_BIT: usize = BEFORE_PADS.len();

/// The bits after the pads. (The footswitch's two are a guess: cabl leaves the last three
/// unnamed, and the manual's count leaves room for exactly these and one spare.)
const AFTER_PADS: [&str; 38] = [
    "GROUP_A",
    "GROUP_B",
    "GROUP_C",
    "GROUP_D",
    "GROUP_E",
    "GROUP_F",
    "GROUP_G",
    "GROUP_H",
    "MST",
    "GRP",
    "IN1",
    "CUE",
    "BROWSE",
    "MACRO",
    "LEVEL",
    "AUX",
    "CONTROL",
    "AUTO",
    "PERFORM",
    "NOTES",
    "LOCK",
    "TUNE",
    "SWING",
    "SHIFT",
    "PLAY",
    "REC",
    "PAGE_LEFT",
    "PAGE_RIGHT",
    "TEMPO",
    "GRID",
    "SOLO",
    "MUTE",
    "SELECT",
    "ENCODER_TOUCH",
    "ENCODER",
    "FOOT_TIP",
    "FOOT_RING",
    "",
];

/// What bit `n` (from 0) of the button bytes is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Button(&'static str),
    /// Column and row from 0, the top left first.
    Pad(u8, u8),
    Nothing,
}

/// Bit `n` of the button bytes.
pub fn control(n: usize) -> Control {
    if n < FIRST_PAD_BIT {
        return Control::Button(BEFORE_PADS[n]);
    }
    let pad = n - FIRST_PAD_BIT;
    if pad < 64 {
        return Control::Pad((pad % 8) as u8, (pad / 8) as u8);
    }
    match AFTER_PADS.get(pad - 64) {
        Some(&"") | None => Control::Nothing,
        Some(&name) => Control::Button(name),
    }
}

/// Report 0x80's first 37 bytes, in order. `IN2` and `MIDI` are the two the manual doesn't
/// list: they sit between `IN1` and `CUE`, where the panel prints those SHIFT labels, so
/// they're probably lights for them.
pub const BUTTON_LEDS: [&str; 37] = [
    "SONG",
    "STEP",
    "PAD_MODE",
    "CLEAR",
    "DUPLICATE",
    "UP",
    "LEFT",
    "RIGHT",
    "DOWN",
    "NOTE_REPEAT",
    "MST",
    "GRP",
    "IN1",
    "IN2",
    "CUE",
    "MIDI",
    "BROWSE",
    "MACRO",
    "LEVEL",
    "AUX",
    "CONTROL",
    "AUTO",
    "PERFORM",
    "NOTES",
    "LOCK",
    "TUNE",
    "SWING",
    "SHIFT",
    "PLAY",
    "REC",
    "PAGE_LEFT",
    "PAGE_RIGHT",
    "TEMPO",
    "GRID",
    "SOLO",
    "MUTE",
    "SELECT",
];

/// Report 0x80: where the two meter columns' 8 LEDs each start.
pub const METER_LEDS: usize = BUTTON_LEDS.len();

/// Whether LED 1 of a meter column and of a strip is the bottom one. A guess to confirm by
/// eye: flipping it here turns every meter and strip the right way up.
pub const BOTTOM_FIRST: bool = true;

/// Report 0x81: 8 scene buttons, the pads by rows from the top left, 8 group buttons.
pub const SCENE_LEDS: usize = 0;
pub const PAD_LEDS: usize = 8;
pub const GROUP_LEDS: usize = 72;

/// LEDs on each Smart Strip.
pub const STRIP_LEDS: usize = 11;
/// LEDs in each meter column.
pub const METER_HEIGHT: usize = 8;

/// Report 0x02: a strip's two fingers' full travel.
pub const STRIP_RANGE: f64 = 1024.0;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_button_bit_has_one_meaning() {
        let names: Vec<&str> = (0..120)
            .filter_map(|n| match control(n) {
                Control::Button(b) => Some(b),
                _ => None,
            })
            .collect();
        assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len(), "no name twice");
        // 8 scenes, 8 groups, 35 others, the encoder's two, the footswitch's two.
        assert_eq!(names.len(), 8 + 8 + 35 + 2 + 2);
        assert_eq!(control(FIRST_PAD_BIT), Control::Pad(0, 0));
        assert_eq!(control(FIRST_PAD_BIT + 63), Control::Pad(7, 7));
        assert_eq!(control(119), Control::Nothing);
        assert_eq!(control(500), Control::Nothing);
    }

    /// Every lit button has a bit too: the two tables name the same panel. (`IN2` and `MIDI`
    /// are SHIFT labels, with lights and no bits of their own.)
    #[test]
    fn the_lights_name_the_buttons() {
        let bits: HashSet<&str> = (0..120)
            .filter_map(|n| match control(n) {
                Control::Button(b) => Some(b),
                _ => None,
            })
            .collect();
        for led in BUTTON_LEDS {
            assert!(bits.contains(led) || led == "IN2" || led == "MIDI", "{led}");
        }
    }

    #[test]
    fn the_light_counts_are_what_the_device_reports() {
        assert_eq!(METER_LEDS + 2 * METER_HEIGHT, 53);
        assert_eq!(GROUP_LEDS + 8, 80);
        assert_eq!(8 * STRIP_LEDS, 88);
    }
}
