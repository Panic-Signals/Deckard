//! Check the map (`src/map.rs`) by hand: press anything, and it says what the map calls it and
//! lights that control. A strip shows its finger on its LEDs, the encoder walks the meters.
//! `cargo run -p deckard-jam --example jam-check [dark]`. Close Maschine first. Ctrl-C ends it,
//! with the lights left as they are; `dark` only turns them all off.

mod support;

use deckard_jam::leds::{ON, SHADE_BRIGHT, colour};
use deckard_jam::map::{METER_HEIGHT, STRIP_LEDS};
use deckard_jam::{Event, Jam, Leds};
use support::{refuse_if_maschine_runs, say};

fn main() {
    refuse_if_maschine_runs();
    let mut jam = Jam::open().expect("open the JAM");
    // `jam-check dark`: every light off, and stop.
    if std::env::args().nth(1).as_deref() == Some("dark") {
        jam.dark().expect("dark");
        return;
    }
    let mut leds = Leds::blank();
    let mut level = 0i32;
    jam.show(&leds).expect("show");
    println!("Press anything: its name should be what's printed on the panel, and it should light.");
    say("Press anything");
    let mut events = Vec::new();
    loop {
        events.clear();
        jam.poll(50, &mut events).expect("read");
        for e in &events {
            println!("{e:?}");
            match *e {
                Event::Button(name, down) => {
                    if let Some(n) = name.strip_prefix("SCENE_").and_then(|n| n.parse::<usize>().ok()) {
                        leds.scene(n - 1, if down { colour(n as i64, SHADE_BRIGHT) } else { 0 });
                    } else if let Some(g) = name.strip_prefix("GROUP_").and_then(|g| g.bytes().next()) {
                        let n = usize::from(g - b'A');
                        leds.group(n, if down { colour(n as i64 + 8, SHADE_BRIGHT) } else { 0 });
                    } else {
                        leds.button(name, if down { ON } else { 0x09 });
                    }
                    if down {
                        say(&name.replace('_', " ").to_lowercase());
                    }
                }
                Event::Pad { column, row, down } => {
                    leds.pad(usize::from(column), usize::from(row), if down { colour(i64::from(row * 2), SHADE_BRIGHT) } else { 0 });
                    if down {
                        say(&format!("column {} row {}", (b'A' + column) as char, row + 1));
                    }
                }
                Event::Strip { strip, kind, pos, .. } => {
                    let upto = if matches!(kind, deckard_jam::Touch::Up) { 0 } else { (pos * STRIP_LEDS as f64).ceil() as usize };
                    for led in 0..STRIP_LEDS {
                        leds.strip(usize::from(strip), led, if led < upto { colour(i64::from(strip) * 2, SHADE_BRIGHT) } else { 0 });
                    }
                }
                Event::Encoder(steps) => {
                    level = (level + steps).clamp(0, 2 * METER_HEIGHT as i32);
                    for led in 0..METER_HEIGHT as i32 {
                        leds.meter(0, led as usize, if led < level { ON } else { 0 });
                        leds.meter(1, led as usize, if led + (METER_HEIGHT as i32) < level { ON } else { 0 });
                    }
                }
            }
        }
        jam.show(&leds).expect("show");
    }
}
