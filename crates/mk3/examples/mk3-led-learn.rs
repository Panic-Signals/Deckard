//! Learn the MK3's LED reports: each byte of output report 0x80 (buttons) and 0x81
//! (pads, touch strip) is lit alone, and you press what lit up (spoken prompts, no time
//! limit). Touch the touch strip to skip a light you can't press (or none you can see).
//! `cargo run -p deckard-mk3 --example mk3-led-learn [report] [from]`
//! (report 0x80 or 0x81, default both; from = first byte, 1-based). Writes
//! $MK3_LEARN_OUT (default ./mk3-leds.txt), appending.

use deckard_mk3::{Decoder, Event};
use std::io::Write;
use std::time::Duration;

fn say(s: &str) {
    let _ = std::process::Command::new("say").arg(s).spawn();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let reports: Vec<(u8, usize)> = match args.get(1).map(String::as_str) {
        Some("0x80") | Some("80") => vec![(0x80, 62)],
        Some("0x81") | Some("81") => vec![(0x81, 41)],
        _ => vec![(0x80, 62), (0x81, 41)],
    };
    let from: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
    let api = hidapi::HidApi::new().expect("hidapi");
    // Shared with NI's agent (macOS; elsewhere HID devices open shared).
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3");
    let out = std::env::var("MK3_LEARN_OUT").unwrap_or_else(|_| "mk3-leds.txt".into());
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&out).expect("output file");
    let mut dec = Decoder::new();
    let mut buf = [0u8; 64];
    let mut events = Vec::new();
    let dark = |id: u8, n: usize| {
        let mut r = vec![0u8; n + 1];
        r[0] = id;
        r
    };
    for &(id, n) in &reports {
        dev.write(&dark(id, n)).ok();
    }
    println!("For each light: press the button or hit the pad that is lit; touch the strip to skip.");
    // The decoder needs one report to know the state; touching a knob sends one.
    say("touch any knob to start");
    println!("touch any knob to start …");
    loop {
        let k = dev.read_timeout(&mut buf, 100).unwrap_or(0);
        if k > 0 && buf[0] == 1 {
            dec.decode(&buf[..k], &mut events);
            break;
        }
    }
    events.clear();
    for &(id, n) in &reports {
        for i in (if reports.len() == 1 { from - 1 } else { 0 })..n {
            let mut r = dark(id, n);
            r[1 + i] = 0x7f;
            // Keep it lit: the controller (or NI's agent) may overwrite it.
            print!("report {id:#04x} byte {} … ", i + 1);
            std::io::stdout().flush().ok();
            say(if id == 0x80 { "press the lit button" } else { "hit the lit pad" });
            loop {
                let k = dev.read_timeout(&mut buf, 5).unwrap_or(0);
                if k == 0 {
                    break;
                }
                dec.decode(&buf[..k], &mut events);
            }
            events.clear();
            let found = loop {
                dev.write(&r).ok();
                let k = dev.read_timeout(&mut buf, 100).unwrap_or(0);
                if k == 0 {
                    continue;
                }
                dec.decode(&buf[..k], &mut events);
                if let Some(e) = events.iter().find_map(|e| match e {
                    Event::Button(name, true) => Some(name.to_string()),
                    Event::PadOn(p, _) => Some(format!("PAD_{p}")),
                    Event::Touch(_, 0, _) => Some("-".into()),
                    _ => None,
                }) {
                    break e;
                }
                events.clear();
            };
            println!("{found}");
            writeln!(file, "{id:#04x}\t{}\t{found}", i + 1).ok();
            dev.write(&dark(id, n)).ok();
            std::thread::sleep(Duration::from_millis(700));
            // Let the strip go / release before the next one.
            loop {
                let k = dev.read_timeout(&mut buf, 20).unwrap_or(0);
                if k == 0 {
                    break;
                }
                dec.decode(&buf[..k], &mut events);
            }
        }
    }
    say("done");
    println!("wrote {out}");
}
