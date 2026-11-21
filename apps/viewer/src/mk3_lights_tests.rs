//! [`crate::mk3_lights`] paints what the scripts' LED bytes say, with no device and no
//! window: palette bytes resolve to DECKARD's own colours, dim is a darker scale of
//! bright, flash alternates with off on the scripts' cadence, and unknown ids stay dark.

use crate::mk3_lights;
use deckard_mk3::leds::Leds;
use std::time::{Duration, Instant};

/// The strip's segments light from the same palette as every other colour LED, and they
/// are not pads or group buttons, so a flash bit still pulses there.
#[test]
fn the_strip_lights_like_any_other_colour_led() {
    let from = Instant::now();
    let leds = leds_with("LED_TS_05", 2 * 4 + 2);
    let group = leds_with("LED_GROUP_A", 2 * 4 + 2);
    assert_eq!(
        mk3_lights::strip_segment(&leds, 4, from),
        mk3_lights::color(&group, "BUTTON_GROUP_A", from),
        "segment 5 is LED_TS_05, in the same palette colour as any other LED"
    );
    assert_eq!(mk3_lights::strip_segment(&leds, 0, from), None, "an unset segment is dark");
    assert_eq!(mk3_lights::strip_segment(&leds, mk3_lights::STRIP_SEGMENTS, from), None, "there is no segment past the last");
}

/// The strip's count must match the driver's own list, not a number typed by hand:
/// `PAD_LEDS` is 25 strip LEDs followed by 16 pads.
#[test]
fn the_strip_segment_count_is_the_drivers_count() {
    let strip = deckard_mk3::leds::PAD_LEDS.iter().filter(|n| n.starts_with("LED_TS_")).count();
    assert_eq!(strip, mk3_lights::STRIP_SEGMENTS, "the painter, the page and the driver agree on the strip");
}
/// `LED_PAD_1` holding `byte`, everything else dark.
fn leds_with_pad(byte: u8) -> Leds {
    leds_with("LED_PAD_1", byte)
}

/// The named LED holding `byte`, everything else dark.
fn leds_with(name: &str, byte: u8) -> Leds {
    let mut leds = Leds::default();
    assert!(leds.set(name, byte));
    leds
}

/// A colour LED whose flash bit still pulses: the wheel's arrow, which the panel does
/// not hold steady (pads and group buttons do).
const PULSING: (&str, &str) = ("BUTTON_WHEEL_UP", "LED_WHEEL_BUTTON_UP");

/// A flashing LED is lit somewhere in the 250 ms after `from`: the first lit sample.
/// A flashing LED is lit somewhere in the 250 ms after now: the first lit sample. Warms
/// up first, so the flash epoch predates every probe time and the phase reads exact
/// instead of saturating to zero.
fn first_lit(leds: &Leds, id: &str) -> Instant {
    mk3_lights::color(leds, id, Instant::now());
    let from = Instant::now();
    (0..500)
        .map(|ms| from + Duration::from_millis(ms))
        .find(|t| mk3_lights::color(leds, id, *t).is_some())
        .expect("a flashing LED is lit somewhere in its period")
}

#[test]
fn a_bright_colour_led_is_its_palette_colour() {
    let leds = leds_with_pad(1 * 4 + 2);
    assert_eq!(mk3_lights::color(&leds, "pad1", Instant::now()), Some([0xff, 0x13, 0x0f]), "colour 1, bright: the palette's red, full");
}

#[test]
fn a_flash_led_resolves_to_on_and_off() {
    let leds = leds_with(PULSING.1, 1 * 4 + 3);
    let on = first_lit(&leds, PULSING.0);
    assert_eq!(mk3_lights::color(&leds, PULSING.0, on), Some([0xff, 0x13, 0x0f]), "lit: the same red as the steady byte");
    assert_eq!(mk3_lights::color(&leds, PULSING.0, on + mk3_lights::FLASH_PERIOD / 2), None, "half a period later: dark");
}

/// The scripts pass `flash` for a selection and for a sounding pad (`global.lua:516`,
/// `:553`), but the hardware holds those lit: a pad that strobes under a finger reads as
/// broken, and so does a group button that never stops blinking.
#[test]
fn a_flashing_pad_or_group_is_steady() {
    let from = Instant::now();
    for (id, name) in [("pad1", "LED_PAD_1"), ("BUTTON_GROUP_A", "LED_GROUP_A")] {
        let leds = leds_with(name, 1 * 4 + 3);
        for ms in [0u64, 125, 250, 375, 499] {
            assert!(
                mk3_lights::color(&leds, id, from + Duration::from_millis(ms)).is_some(),
                "{id} stays lit {ms} ms into the flash period"
            );
        }
    }
}

#[test]
fn a_dim_led_is_darker_than_the_bright_one() {
    let bright = mk3_lights::color(&leds_with_pad(1 * 4 + 2), "pad1", Instant::now()).expect("bright red");
    let dim = mk3_lights::color(&leds_with_pad(1 * 4 + 0), "pad1", Instant::now()).expect("dim red");
    assert!(
        dim.iter().map(|c| *c as u16).sum::<u16>() < bright.iter().map(|c| *c as u16).sum::<u16>(),
        "dim {dim:?} is darker over all channels than bright {bright:?}"
    );
}

#[test]
fn a_dim_single_colour_led_is_dimmer_than_its_bright_counterpart() {
    let mut leds = Leds::default();
    assert!(leds.set("LED_TRANSPORT_PLAY", deckard_mk3::leds::ON));
    let bright = mk3_lights::color(&leds, "BUTTON_TRANSPORT_PLAY", Instant::now()).expect("ON is lit");
    assert!(leds.set("LED_TRANSPORT_PLAY", deckard_mk3::leds::DIM));
    let dim = mk3_lights::color(&leds, "BUTTON_TRANSPORT_PLAY", Instant::now()).expect("DIM is lit, not black");
    for (d, b) in dim.iter().zip(bright.iter()) {
        assert!(d < b, "dim {dim:?} is strictly darker per channel than bright {bright:?}");
    }
}

#[test]
fn an_unknown_id_returns_none() {
    let leds = leds_with_pad(1 * 4 + 2);
    assert_eq!(mk3_lights::color(&leds, "no_such_control", Instant::now()), None, "not in the table: unlit, never a panic");
    assert_eq!(mk3_lights::color(&leds, "strip", Instant::now()), None, "the table has the strip but no single LED does");
    assert_eq!(mk3_lights::color(&Leds::default(), "pad1", Instant::now()), None, "a dark byte paints nothing");
}

#[test]
fn the_flash_period_is_500ms() {
    // Pins FLASH_PERIOD against literals: halve the constant and this fails (at
    // +250 ms the shorter period would read lit again, not dark). The scripts test
    // `(deckard.now() * 4) % 2 < 1` in seconds, so a flash is lit at t, dark at
    // t + 250 ms, lit again at t + 500 ms.
    // The pulsing control is the wheel's arrow: pads and group buttons hold their flash
    // steady now (`a_flashing_pad_or_group_is_steady`), so the cadence is pinned on a
    // colour LED that still pulses.
    let leds = leds_with(PULSING.1, 1 * 4 + 3);
    let on = first_lit(&leds, PULSING.0);
    assert_eq!(mk3_lights::color(&leds, PULSING.0, on + Duration::from_millis(250)), None, "half a 500 ms period later: dark");
    assert_eq!(
        mk3_lights::color(&leds, PULSING.0, on + Duration::from_millis(500)),
        Some([0xff, 0x13, 0x0f]),
        "a full 500 ms period later: lit again"
    );
}

#[test]
fn led_names_follow_the_scripts() {
    use crate::mk3_layout::led;
    assert_eq!(led("pad1").as_deref(), Some("LED_PAD_1"));
    assert_eq!(led("pad16").as_deref(), Some("LED_PAD_16"));
    assert_eq!(led("BUTTON_TRANSPORT_PLAY").as_deref(), Some("LED_TRANSPORT_PLAY"));
    assert_eq!(led("BUTTON_SCREEN_4").as_deref(), Some("LED_SCREEN_BUTTON_4"));
    assert_eq!(led("BUTTON_WHEEL_UP").as_deref(), Some("LED_WHEEL_BUTTON_UP"));
    assert_eq!(led("BUTTON_AUTO").as_deref(), Some("LED_AUTO_WRITE"));
    assert_eq!(led("BUTTON_WHEEL"), None, "the press has no light; its arrows do");
    assert_eq!(led("strip"), None, "25 segments are not one light");
    assert_eq!(led("knob1"), None, "knobs have no light");
    assert_eq!(led("nope"), None, "unknown ids have none either");
}
