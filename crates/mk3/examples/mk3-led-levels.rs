//! Compare low button brightnesses: the first ten button LEDs (CHANNEL, PLUGIN, ARRANGE,
//! MIX, BROWSE, SAMPLE, LEFT, RIGHT, FILE, SETTINGS…) at levels 1..=10, for a while.
//! `cargo run -p deckard-mk3 --example mk3-led-levels -- [seconds] [first level] [step]`

use deckard_mk3::leds::{self, Leds};
use std::time::Duration;

fn main() {
    let args: Vec<u32> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let secs = args.first().copied().unwrap_or(60);
    let first = args.get(1).copied().unwrap_or(1);
    let step = args.get(2).copied().unwrap_or(1);
    let dev = deckard_mk3::Device::open().expect("open the MK3");
    let mut l = Leds::default();
    for (i, name) in leds::BUTTON_LEDS.iter().take(10).enumerate() {
        let level = (first + step * i as u32).min(0x7f) as u8;
        l.set(name, level);
        println!("{name}: 0x{level:02x}");
    }
    for _ in 0..secs * 2 {
        dev.write_leds(&l).expect("write the LEDs");
        std::thread::sleep(Duration::from_millis(500));
    }
    dev.write_leds(&Leds::default()).ok();
}
