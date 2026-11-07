//! Light exactly what's named and nothing else, for checking the map by eye or by camera:
//! `jam-lights b:SONG g:0 s:3 p:2,7 st:0,10 m:1,7`. With no arguments: every light off.
//! Buttons, then group, scene, pad (column, row, from the top left), strip and meter (LEDs
//! from the bottom). `:c` after a colour light's place picks its palette colour (`p:0,0:5`).
//! `=n` after a colour light's place sends that byte as it is (`p:0,0=4`), and `blank` starts
//! from the buttons' backlight. The lights stay as they are after it exits. Close Maschine first.

mod support;

use deckard_jam::leds::{ON, SHADE_BRIGHT, WHITE, colour};
use deckard_jam::{Jam, Leds};
use support::refuse_if_maschine_runs;

fn main() {
    refuse_if_maschine_runs();
    let mut jam = Jam::open().expect("open the JAM");
    let mut leds = Leds::default();
    for token in std::env::args().skip(1) {
        if token == "blank" {
            leds = Leds::blank();
            continue;
        }
        let (kind, rest) = token.split_once(':').expect("kind:place");
        let (rest, raw) = match rest.split_once('=') {
            Some((rest, n)) => (rest, Some(n.parse::<u8>().expect("a byte"))),
            None => (rest, None),
        };
        let (place, palette) = match rest.split_once(':') {
            Some((place, c)) => (place, c.parse().expect("a colour")),
            None => (rest, WHITE),
        };
        let byte = raw.unwrap_or_else(|| colour(palette, SHADE_BRIGHT));
        let nums: Vec<usize> = place.split(',').filter_map(|n| n.parse().ok()).collect();
        match (kind, nums.as_slice()) {
            ("b", _) => assert!(leds.button(place, ON), "no light {place}"),
            ("g", [n]) => leds.group(*n, byte),
            ("s", [n]) => leds.scene(*n, byte),
            ("p", [c, r]) => leds.pad(*c, *r, byte),
            ("st", [s, l]) => leds.strip(*s, *l, byte),
            ("m", [c, l]) => leds.meter(*c, *l, ON),
            _ => panic!("what is {token}?"),
        }
    }
    jam.show(&leds).expect("show");
}
