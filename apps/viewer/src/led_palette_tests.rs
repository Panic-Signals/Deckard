//! One LED colour table, both panels.
//!
//! The MK3 and the Jam name their colours from the same 17 `LEDColors` indices (1 red … 17 white),
//! and the two panels stand side by side in one window: the same index must be the same red on
//! both. It is also the theme's table (`crates/surface/lua/themes/maschine.lua`), parsed here from
//! the Lua the surface ships rather than retyped, so a transcription that drifts fails.

use crate::led_palette::PALETTE;
use crate::mk3_lights;
use std::time::Instant;

/// The table is the theme's `PALETTE`, entry for entry: the scripts run this Lua, so it is the
/// copy the hardware's colours actually come from.
#[test]
fn the_table_is_the_themes_palette_verbatim() {
    assert_eq!(PALETTE.to_vec(), theme_palette(), "the LED table is the theme's, verbatim");
    assert_eq!(PALETTE.len(), 17, "16 colours and white");
}

/// The theme's `PALETTE`, parsed out of the shipped Lua: the first `{` after the declaration to the
/// `}` that closes it, and every `0xNN` inside in threes.
fn theme_palette() -> Vec<[u8; 3]> {
    const THEME: &str = include_str!("../../../crates/surface/lua/themes/maschine.lua");
    let start = THEME.find("local PALETTE = {").expect("the theme declares the palette");
    let mut depth = 0usize;
    let mut end = THEME.len();
    for (at, c) in THEME[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + at;
                    break;
                }
            }
            _ => {}
        }
    }
    let block = THEME[start..end].as_bytes();
    let mut bytes = Vec::new();
    let mut at = 0;
    while at + 3 < block.len() {
        if &block[at..at + 2] == b"0x" {
            let hex = std::str::from_utf8(&block[at + 2..at + 4]).expect("two hex digits");
            bytes.push(u8::from_str_radix(hex, 16).expect("a byte in hex"));
        }
        at += 1;
    }
    assert_eq!(bytes.len() % 3, 0, "the palette is triples: {bytes:?}");
    bytes.chunks(3).map(|rgb| [rgb[0], rgb[1], rgb[2]]).collect()
}
