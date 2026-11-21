//! The one LED colour table: what a script's `LEDColors` index looks like on both panels.
//!
//! The MK3 and the Jam name their colours from the same seventeen indices — 1 red … 17 white, the
//! theme's `LEDColors` order — so on a controller with both plugged in, `colour 3` has to be the
//! same orange on both glasses. One table, then: [`crate::mk3_lights`] reads it for the MK3's
//! packed palette byte and [`crate::jam_lights`] for the Jam's, and neither keeps a copy of its own.
//!
//! This is deliberately *not* [`crate::layout::PALETTE`]. That is the interface's group-colour table
//! (`docs/ui-layout.md` §8: Maschine's `Colors.qml`, palette 1), the brighter set the project view
//! draws scenes and groups with, and it is a screen table. The Jam used to read it, so the same
//! index drew a paler colour on the panel than the hardware lights — the one visual change of the
//! dual-stack work.
//!
//! The values are `crates/surface/lua/themes/maschine.lua`'s `PALETTE`, transcribed verbatim:
//! that file is the copy the scripts run, and [`crate::led_palette_tests`] parses it to keep this
//! one honest. A themed palette is a later concern, not this module's.

/// The scripts' `LEDColors` index + 1 (17 is white), as the theme's `PALETTE`.
pub(crate) const PALETTE: [[u8; 3]; 17] = [
    [0xff, 0x13, 0x0f],
    [0xff, 0x3c, 0x14],
    [0xff, 0x78, 0x00],
    [0xff, 0xb9, 0x00],
    [0xff, 0xff, 0x00],
    [0x92, 0xff, 0x00],
    [0x26, 0xff, 0x26],
    [0x00, 0xd1, 0x81],
    [0x00, 0xb4, 0xe9],
    [0x00, 0x78, 0xff],
    [0x00, 0x48, 0xff],
    [0x82, 0x00, 0xff],
    [0xa2, 0x00, 0xc8],
    [0xf5, 0x00, 0xc8],
    [0xff, 0x00, 0x78],
    [0xf7, 0x07, 0x3e],
    [0xff, 0xff, 0xff],
];
