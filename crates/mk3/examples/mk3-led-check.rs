//! Check the MK3's LED colours by asking: each step lights the controller, a spoken
//! question asks about it, and you answer on the controller (PLAY yes, STOP no).
//! Writes $MK3_LEARN_OUT (default ./mk3-led-check.txt).
//! `cargo run -p deckard-mk3 --example mk3-led-check`

use deckard_mk3::leds::{self, Leds};
use deckard_mk3::{Decoder, Event};
use std::io::Write;
use std::time::Duration;

fn say(s: &str) {
    let _ = std::process::Command::new("say").arg(s).status();
}

/// The pads in `LEDColors` order: top left first, across, then down.
const PADS: [usize; 16] = [13, 14, 15, 16, 9, 10, 11, 12, 5, 6, 7, 8, 1, 2, 3, 4];

fn rainbow(shade: u8) -> Leds {
    let mut l = Leds::default();
    for (colour, pad) in PADS.iter().enumerate() {
        l.set(&format!("LED_PAD_{pad}"), ((colour as u8 + 1) << 2) | shade);
    }
    l
}

fn all_pads(byte: u8) -> Leds {
    let mut l = Leds::default();
    for pad in 1..=16 {
        l.set(&format!("LED_PAD_{pad}"), byte);
    }
    l
}

fn main() {
    let api = hidapi::HidApi::new().expect("hidapi");
    // Shared with NI's agent (macOS; elsewhere HID devices open shared).
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3");
    let out = std::env::var("MK3_LEARN_OUT").unwrap_or_else(|_| "mk3-led-check.txt".into());
    let mut file = std::fs::File::create(&out).expect("output file");
    let show = |l: &Leds| {
        dev.write(&l.buttons).ok();
        dev.write(&l.pads).ok();
    };
    let mut dec = Decoder::new();
    let mut buf = [0u8; 64];
    let mut events = Vec::new();
    // Answer lights: PLAY and STOP lit so they're easy to find.
    let with_answers = |mut l: Leds| {
        l.set("LED_TRANSPORT_PLAY", 0x7f);
        l.set("LED_TRANSPORT_STOP", 0x7f);
        l
    };

    let mut questions: Vec<(&str, Leds)> = vec![
        (
            "Are the pads lit in sixteen different colours, starting red at the top left and going through orange, yellow, green, cyan, blue, to purple and pink at the bottom right?",
            rainbow(2),
        ),
        ("Is the top left pad red?", rainbow(2)),
        ("Is the bottom right pad pink or reddish pink?", rainbow(2)),
        ("Now the pads are dimmer. Are they the same colours as before, only darker?", rainbow(0)),
        ("Are the pads all white?", all_pads((17 << 2) | 2)),
        ("Are the pads all green?", all_pads((7 << 2) | 2)),
        ("Are the pads all blue?", all_pads((11 << 2) | 2)),
    ];
    let mut groups = Leds::default();
    for g in 0..8u8 {
        groups.set(&format!("LED_GROUP_{}", (b'A' + g) as char), ((g + 1) << 2) | 2);
    }
    questions.push(("Are the group buttons A to H lit in different colours, A red?", groups));
    let mut strip = Leds::default();
    for t in 0..25 {
        strip.set(leds::PAD_LEDS[t], ((t % 16) as u8 + 1) << 2 | 2);
    }
    questions.push(("Is the touch strip lit?", strip.clone()));
    questions.push(("Is the touch strip lit in several colours, not just white?", strip));
    let mut arrows = Leds::default();
    for w in &leds::BUTTON_LEDS[58..62] {
        arrows.set(w, (17 << 2) | 2);
    }
    questions.push(("Are the four arrow lights around the big encoder lit?", arrows));
    let mut levels = Leds::default();
    levels.set("LED_TRANSPORT_LOOP", 0x10);
    levels.set("LED_TRANSPORT_ERASE", 0x40);
    levels.set("LED_TAP", 0x7f);
    questions.push(("Do the restart, erase and tap buttons go from faint to bright, left to right?", levels));

    println!("Answer on the controller: PLAY = yes, STOP = no. Touch any knob to start.");
    say("Touch any knob to start.");
    // Prime the decoder with a first report 0x01.
    loop {
        let n = dev.read_timeout(&mut buf, 100).unwrap_or(0);
        if n > 0 && buf[0] == 1 {
            dec.decode(&buf[..n], &mut events);
            break;
        }
    }
    for (q, l) in &questions {
        let l = with_answers(l.clone());
        show(&l);
        std::thread::sleep(Duration::from_millis(400));
        // Drop anything pressed while the question was set up.
        loop {
            let n = dev.read_timeout(&mut buf, 0).unwrap_or(0);
            if n == 0 {
                break;
            }
            dec.decode(&buf[..n], &mut Vec::new());
        }
        println!("{q}");
        say(q);
        let answer = 'wait: loop {
            show(&l);
            let n = dev.read_timeout(&mut buf, 100).unwrap_or(0);
            if n == 0 {
                continue;
            }
            events.clear();
            dec.decode(&buf[..n], &mut events);
            for e in &events {
                match e {
                    Event::Button("BUTTON_TRANSPORT_PLAY", true) => break 'wait "yes",
                    Event::Button("BUTTON_TRANSPORT_STOP", true) => break 'wait "no",
                    _ => {}
                }
            }
        };
        println!("  {answer}");
        say(answer);
        writeln!(file, "{answer}\t{q}").ok();
    }
    show(&Leds::default());
    say("Done, thank you.");
    println!("wrote {out}");
}
