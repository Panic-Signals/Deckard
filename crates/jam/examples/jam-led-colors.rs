//! See what values the JAM's lights take. Lights one kind of LED at a value and waits: type
//! what you see and Enter, or Enter alone for the next. Ctrl-C leaves the lights as they
//! are; the end turns them off.
//! `cargo run -p deckard-jam --example jam-led-colors [walk]`
//! By default it fills the pads with a chart, a colour a column and a shade a row, by the
//! MK3's formula `(colour + 1) * 4 + shade` (docs/mk3-usb.md), then shows the same values on
//! the scene, group and strip LEDs. `walk` steps one pad, one scene button, one group
//! button, one strip LED and one plain button through every byte 0..255 instead, which needs
//! the byte positions from jam-led-learn (it assumes byte 1 of each report).
//! Close Maschine first. Writes $JAM_LEARN_OUT (default ./jam-colors.txt), appending.

mod support;

use deckard_jam::{LEDS, all_dark, dark, open};
use std::io::Write;
use support::{refuse_if_maschine_runs, say, typed};

fn main() {
    refuse_if_maschine_runs();
    let walk = std::env::args().nth(1).is_some_and(|a| a == "walk");
    let path = std::env::var("JAM_LEARN_OUT").unwrap_or_else(|_| "jam-colors.txt".into());
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path).expect("output file");
    let dev = open().expect("open the JAM");
    let input = typed();
    let mut ask = |what: String| {
        println!("{what}: what do you see? (Enter for next)");
        let seen = input.recv().unwrap_or_default();
        writeln!(file, "{what}\t{seen}").expect("write");
        file.flush().ok();
    };
    all_dark(&dev).expect("write");
    if walk {
        for v in 0..=255u8 {
            for (id, len) in LEDS {
                let mut r = dark(id, len);
                r[1] = v;
                dev.write(&r).expect("write");
            }
            ask(format!("byte value {v:#04x}"));
        }
    } else {
        say("Look at the pads");
        // Pads: colour by byte position across, shade down. 16 colours don't fit 8 columns, so
        // two charts.
        for first in [0u8, 8] {
            let mut r = dark(0x81, LEDS[1].1);
            for row in 0..8u8 {
                for col in 0..8u8 {
                    // Assumes bytes run by row from the first pad; jam-led-learn says if not.
                    let at = 1 + usize::from(row) * 8 + usize::from(col);
                    r[at] = (first + col + 1) * 4 + (row % 4);
                }
            }
            dev.write(&r).expect("write");
            ask(format!("pad chart, colours {first}.. across, shades 0-3 down (repeating)"));
        }
        // White, by the same formula (17th colour), at each shade.
        for shade in 0..4u8 {
            let mut r = dark(0x81, LEDS[1].1);
            r[1..].fill(17 * 4 + shade);
            dev.write(&r).expect("write");
            ask(format!("every colour LED white, shade {shade}"));
        }
        for level in [0x10u8, 0x20, 0x40, 0x7f] {
            let mut r = dark(0x80, LEDS[0].1);
            r[1..].fill(level);
            dev.write(&r).expect("write");
            ask(format!("every plain LED at {level:#04x}"));
            dev.write(&dark(0x80, LEDS[0].1)).expect("write");
        }
        for v in [17u8 * 4 + 3, 5 * 4 + 3, 0x7f] {
            let mut r = dark(0x82, LEDS[2].1);
            r[1..].fill(v);
            dev.write(&r).expect("write");
            ask(format!("every strip LED at {v:#04x}"));
        }
    }
    all_dark(&dev).expect("write");
    println!("Done: {path}");
}
