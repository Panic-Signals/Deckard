//! One light function for every MK3 surface, and nothing else.
//!
//! The controller scripts report each LED as one hardware byte ([`deckard_mk3::leds`]),
//! and the on-screen panel and the browser page must paint the same colour from it, so
//! neither keeps a palette of its own: both call [`color`]. `None` means off — the
//! painter draws an unlit control, never a panic.
//!
//! The palette is DECKARD's own, not the user's theme, and not this module's: [`crate::led_palette`]
//! holds the theme's `LEDColors` transcription for both panels, because the MK3's glass and the
//! Jam's must show one colour for one index. A themed palette is a later concern, not this task's.

use crate::led_palette::PALETTE;
use crate::mk3_layout;
use deckard_mk3::leds::{self, Leds, OFF};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How much of the bright colour a dim LED paints (0..1): clearly darker per channel,
/// still legible beside the bright one.
pub const DIM_SCALE: f32 = 0.35;

/// The scripts flash at two pulses a second: `sampling.lua` (WAITING) tests
/// `(deckard.now() * 4) % 2 < 1`, and `deckard.now()` is seconds, so the period is
/// half a second — lit the first 250 ms, dark the second 250 ms.
pub const FLASH_PERIOD: Duration = Duration::from_millis(500);

/// A single-colour LED's colour: the buttons are white-backlit, the scripts' default
/// colour (index 16) for what has none of its own.
const SINGLE: [u8; 3] = [0xff, 0xff, 0xff];

/// Whether a flashing LED is lit now: the first half of each [`FLASH_PERIOD`] after a
/// fixed epoch, so both painters animate identically and neither can drift.
fn flash_on(now: Instant) -> bool {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = *EPOCH.get_or_init(Instant::now);
    let half = FLASH_PERIOD.as_millis() / 2;
    (now.saturating_duration_since(epoch).as_millis() % FLASH_PERIOD.as_millis()) < half
}

/// How many LEDs the touch strip has: `LED_TS_01`..`LED_TS_25`, the first of the 41
/// entries in [`deckard_mk3::leds::PAD_LEDS`]. Pinned to that list by a test rather than
/// counted by hand: an off-by-one leaves the last segment permanently dark.
pub const STRIP_SEGMENTS: usize = 25;

/// The colour to paint control `id` now: the LED's RGB, or [`None`] for off.
pub fn color(leds: &Leds, id: &str, now: Instant) -> Option<[u8; 3]> {
    mk3_layout::control(id)?;
    let name = mk3_layout::led(id)?;
    decode(&name, leds.get(&name)?, now, id)
}

/// One segment of the touch strip, lit or not. The strip is 26 colour LEDs and the scripts
/// light them as a level or a sweep, so the surface has to show them: a strip that never
/// lights is a dead control under a finger.
pub fn strip_segment(leds: &Leds, index: usize, now: Instant) -> Option<[u8; 3]> {
    if index >= STRIP_SEGMENTS {
        return None;
    }
    let name = format!("LED_TS_{:02}", index + 1);
    decode(&name, leds.get(&name)?, now, &name)
}

/// One LED byte as paint: a palette byte for the colour LEDs, a level for the rest. `id`
/// names the control for the steady-shade rule (see [`steady_shade`]).
fn decode(name: &str, byte: u8, now: Instant, id: &str) -> Option<[u8; 3]> {
    if leds::is_colour(name) {
        let (colour, shade) = (byte >> 2, byte & 3);
        if !(1..=17).contains(&colour) {
            return None;
        }
        let mut rgb = PALETTE[colour as usize - 1];
        if shade & 2 == 0 {
            rgb = rgb.map(|c| (c as f32 * DIM_SCALE) as u8);
        }
        // Pads and group buttons never blink: the scripts pass `flash` for a selection and
        // for a sounding pad (`global.lua:516`, `:553`), but the hardware holds those
        // steady, and a pad that strobes under a finger reads as broken. Everything else
        // keeps the pulse its script asked for (sampling's waiting cue, page hints).
        if shade & 1 != 0 && !steady_shade(id) && !flash_on(now) {
            return None;
        }
        Some(rgb)
    } else {
        // A one-colour LED's byte is a level, higher is brighter: main's model in
        // `crates/mk3/src/leds.rs` (BACKLIGHT 0x09 < DIM 0x22 < ON 0x7f, all under 0x80).
        // The older NI duty this file first read was active-low and never below 0x80.
        if byte == OFF {
            return None;
        }
        if byte >= leds::ON || byte == 0xff { Some(SINGLE) } else { Some(SINGLE.map(|c| (c as f32 * DIM_SCALE) as u8)) }
    }
}

/// Whether this control's flash bit means "steady": the pads and the group buttons, whose
/// selected and sounding states the hardware shows lit rather than pulsing.
fn steady_shade(id: &str) -> bool {
    id.starts_with("pad") || id.starts_with("BUTTON_GROUP_")
}
