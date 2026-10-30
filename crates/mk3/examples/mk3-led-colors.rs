//! Check the MK3's LED colours: the pads show the 16 palette colours (top left red,
//! across, then down), the groups colours 1..8, the touch strip all 25 in turn, and
//! every few seconds the shade steps through dim, dim flash, bright, flash. The
//! transport buttons show brightness 0x10, 0x40 and 0x7f (play, record, stop).
//! `cargo run -p deckard-mk3 --example mk3-led-colors`

use deckard_mk3::leds::{self, Leds};
use std::time::Duration;

fn main() {
    let api = hidapi::HidApi::new().expect("hidapi");
    // Shared with NI's agent (macOS; elsewhere HID devices open shared).
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3");
    let device_write = |l: &Leds| {
        dev.write(&l.buttons).ok();
        dev.write(&l.pads).ok();
    };
    let shades = ["dim", "dim flash", "bright", "flash"];
    for round in 0..3 {
        for (shade, name) in shades.iter().enumerate() {
            println!("round {}: {name}", round + 1);
            let mut l = Leds::default();
            // Pads in the order of LEDColors: the top row is PAD_13..16.
            let order = [13, 14, 15, 16, 9, 10, 11, 12, 5, 6, 7, 8, 1, 2, 3, 4];
            for (colour, pad) in order.iter().enumerate() {
                l.set(&format!("LED_PAD_{pad}"), ((colour as u8 + 1) << 2) | shade as u8);
            }
            for g in 0..8u8 {
                l.set(&format!("LED_GROUP_{}", (b'A' + g) as char), ((g + 1) << 2) | shade as u8);
            }
            for t in 0..25 {
                l.set(leds::PAD_LEDS[t], (((t % 17) as u8 + 1) << 2) | shade as u8);
            }
            for w in &leds::BUTTON_LEDS[58..62] {
                l.set(w, (17 << 2) | shade as u8);
            }
            l.set("LED_TRANSPORT_PLAY", 0x10);
            l.set("LED_TRANSPORT_RECORD", 0x40);
            l.set("LED_TRANSPORT_STOP", 0x7f);
            device_write(&l);
            std::thread::sleep(Duration::from_secs(5));
        }
    }
    device_write(&Leds::default());
}
