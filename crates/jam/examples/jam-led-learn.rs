//! Learn the JAM's LED reports: each byte of 0x80, 0x81 and 0x82 is lit alone. Press what
//! lit up and its input bit is recorded; or, for a light with nothing to press (a strip's or
//! a meter's), type what you see and Enter ("strip C 4", "left meter 2"). Enter alone means
//! nothing visible lit. Lights go dark when it ends, and stay as they are if you Ctrl-C.
//! `cargo run -p deckard-jam --example jam-led-learn [report] [from] [value]`
//! `report` is 80, 81 or 82 (default all), `from` the first byte (from 1), `value` the byte
//! to light with in hex (default 7f). Close Maschine first. Writes $JAM_LEARN_OUT (default
//! ./jam-leds.txt), appending.

mod support;

use deckard_jam::{LEDS, all_dark, changes, dark, open, rising};
use std::io::Write;
use support::{refuse_if_maschine_runs, say, typed};

fn main() {
    refuse_if_maschine_runs();
    let args: Vec<String> = std::env::args().collect();
    let reports: Vec<(u8, usize)> = match args.get(1).and_then(|s| u8::from_str_radix(s.trim_start_matches("0x"), 16).ok()) {
        Some(id) => LEDS.iter().copied().filter(|(i, _)| *i == id).collect(),
        None => LEDS.to_vec(),
    };
    let from: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
    let value = args.get(3).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(0x7f);
    let path = std::env::var("JAM_LEARN_OUT").unwrap_or_else(|_| "jam-leds.txt".into());
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path).expect("output file");
    let dev = open().expect("open the JAM");
    let input = typed();
    all_dark(&dev).expect("write");
    // The first report sets the state to compare with: what's at rest isn't a press.
    println!("Press and let go of any button to start …");
    say("Press and let go of any button to start");
    let mut last = {
        let mut buf = [0u8; 256];
        loop {
            let n = dev.read_timeout(&mut buf, 100).expect("read");
            if n > 0 && buf[0] == 0x01 {
                break buf[..n].to_vec();
            }
        }
    };
    println!("Press what lights up, or type what you see and Enter; Enter alone if nothing lit.");
    say("Press what lights up");
    for (id, len) in reports {
        for i in (from - 1)..len {
            let mut r = dark(id, len);
            r[1 + i] = value;
            dev.write(&r).expect("write");
            print!("report {id:#04x} byte {} … ", i + 1);
            std::io::stdout().flush().ok();
            let mut buf = [0u8; 256];
            let what = loop {
                if let Ok(text) = input.try_recv() {
                    break if text.is_empty() { "nothing".to_string() } else { format!("seen: {text}") };
                }
                let n = dev.read_timeout(&mut buf, 20).expect("read");
                if n > 0 && buf[0] == 0x01 {
                    let bits = rising(&changes(&last, &buf[..n]));
                    last = buf[..n].to_vec();
                    if !bits.is_empty() {
                        let list: Vec<String> = bits.iter().map(|(b, n)| format!("{b}.{n}")).collect();
                        break format!("input {}", list.join(" "));
                    }
                }
            };
            println!("{what}");
            writeln!(file, "{id:#04x}\t{}\t{what}", i + 1).expect("write");
            file.flush().ok();
        }
    }
    all_dark(&dev).expect("write");
    println!("Done: {path}");
    say("Done");
}
