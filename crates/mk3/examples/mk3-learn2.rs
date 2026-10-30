//! Second learning pass (spoken prompts, no time limit): pad numbering, the main
//! encoder, a knob and the touch strip. Prints and writes what changed.
//! `cargo run -p deckard-mk3 --example mk3-learn2` ($MK3_LEARN_OUT, default ./mk3-learn2.txt)

use std::io::Write;
use std::time::{Duration, Instant};

fn say(s: &str) {
    let _ = std::process::Command::new("say").arg(s).status();
}

fn main() {
    let api = hidapi::HidApi::new().expect("hidapi");
    // Shared with NI's agent (macOS; elsewhere HID devices open shared).
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3");
    let out = std::env::var("MK3_LEARN_OUT").unwrap_or_else(|_| "mk3-learn2.txt".into());
    let mut file = std::fs::File::create(&out).expect("output file");
    let mut buf = [0u8; 64];
    let mut log = |s: String| {
        println!("{s}");
        writeln!(file, "{s}").ok();
    };
    // Pads: the number printed on each pad, 1 (bottom left) … 16 (top right).
    for n in 1..=16 {
        print!("hit pad {n} (the number printed on it) … ");
        std::io::stdout().flush().ok();
        say(&format!("pad {n}"));
        while dev.read_timeout(&mut buf, 20).unwrap_or(0) > 0 {}
        let idx = loop {
            if dev.read_timeout(&mut buf, 50).unwrap_or(0) > 3 && buf[0] == 2 && buf[2] >> 4 == 1 {
                break buf[1];
            }
        };
        log(format!("pad {n} = hid index {idx}"));
        std::thread::sleep(Duration::from_millis(400));
    }
    // Continuous controls: record report 1's bytes 11..42 before and while it moves.
    let mut grab = |prompt: &str, secs: u64| {
        println!("{prompt} ({secs} s)");
        say(prompt);
        while dev.read_timeout(&mut buf, 20).unwrap_or(0) > 0 {}
        let end = Instant::now() + Duration::from_secs(secs);
        let mut lines = Vec::new();
        let mut last = [0u8; 42];
        while Instant::now() < end {
            let n = dev.read_timeout(&mut buf, 20).unwrap_or(0);
            if n >= 42 && buf[0] == 1 && buf[..42] != last[..] {
                last.copy_from_slice(&buf[..42]);
                lines.push(buf[11..42].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
            }
        }
        (prompt.to_string(), lines)
    };
    let tests = [
        ("turn the big encoder clockwise, slowly, five clicks", 12),
        ("turn the big encoder counter clockwise, five clicks", 12),
        ("turn knob 1 clockwise, a full turn, slowly", 12),
        ("slide one finger along the touch strip, left to right", 10),
    ];
    for (p, s) in tests {
        let (prompt, lines) = grab(p, s);
        log(format!("== {prompt}: bytes 11..41 of report 1"));
        for l in lines {
            log(l);
        }
    }
    say("done");
    println!("wrote {out}");
}
