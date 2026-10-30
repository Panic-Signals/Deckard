//! Learn which bit of HID report 0x01 each MK3 button sets: it names a button, you
//! press it, it records the bit. Run it yourself (it prompts):
//! `cargo run -p deckard-mk3 --example mk3-learn [from]`: no time limit; each prompt
//! is also spoken (macOS `say`). Touch the touch strip to skip a control. `from` (1..)
//! resumes at that control, appending to the file.
//! Writes the map to $MK3_LEARN_OUT (default ./mk3-buttons.txt).

use std::io::Write;
use std::time::{Duration, Instant};

const BUTTONS: &[&str] = &[
    "CHANNEL (MIDI)",
    "PLUG-IN (Instance)",
    "ARRANGER",
    "MIXER",
    "BROWSER (+Plug-in)",
    "SAMPLING",
    "page LEFT arrow (left of the screens)",
    "page RIGHT arrow (left of the screens)",
    "FILE (Save)",
    "SETTINGS",
    "AUTO",
    "MACRO (Set)",
    "display button 1",
    "display button 2",
    "display button 3",
    "display button 4",
    "display button 5",
    "display button 6",
    "display button 7",
    "display button 8",
    "4-D encoder PUSH",
    "4-D encoder tilt UP",
    "4-D encoder tilt RIGHT",
    "4-D encoder tilt DOWN",
    "4-D encoder tilt LEFT",
    "VOLUME",
    "SWING",
    "NOTE REPEAT (Arp)",
    "TEMPO",
    "LOCK",
    "PITCH",
    "MOD",
    "PERFORM (FX Select)",
    "NOTES",
    "group A",
    "group B",
    "group C",
    "group D",
    "group E",
    "group F",
    "group G",
    "group H",
    "RESTART (Loop)",
    "ERASE (Replace)",
    "TAP (Metro)",
    "FOLLOW (Grid)",
    "PLAY",
    "REC (Count-in)",
    "STOP",
    "SHIFT",
    "FIXED VEL (16 Vel)",
    "PAD MODE",
    "KEYBOARD",
    "CHORDS",
    "STEP",
    "SCENE (Section)",
    "PATTERN",
    "EVENTS",
    "VARIATION (Navigate)",
    "DUPLICATE (Double)",
    "SELECT",
    "SOLO",
    "MUTE (Choke)",
    "TOUCH knob 1 (just rest a finger on it)",
    "TOUCH knob 2",
    "TOUCH knob 3",
    "TOUCH knob 4",
    "TOUCH knob 5",
    "TOUCH knob 6",
    "TOUCH knob 7",
    "TOUCH knob 8",
    "TOUCH the 4-D encoder top (rest a finger on it)",
];

fn main() {
    let api = hidapi::HidApi::new().expect("hidapi");
    // Shared with NI's agent (macOS; elsewhere HID devices open shared).
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3");
    let out = std::env::var("MK3_LEARN_OUT").unwrap_or_else(|_| "mk3-buttons.txt".into());
    let from: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
    // Bits already learned (a resumed run): never taken again, so a touch sensor that
    // fires along with a press (the 4-D encoder's) doesn't get credited.
    let mut assigned = [false; 80];
    let mut done: Vec<String> = Vec::new();
    if from > 1 {
        for line in std::fs::read_to_string(&out).unwrap_or_default().lines() {
            let (bit, name) = line.split_once('\t').unwrap_or(("-", line));
            if let Ok(b) = bit.parse::<usize>() {
                assigned[b.min(79)] = true;
            }
            done.push(name.to_string());
        }
    }
    let mut file =
        std::fs::OpenOptions::new().create(true).append(from > 1).write(true).truncate(from == 1).open(&out).expect("output file");
    let mut buf = [0u8; 64];
    let mut held = [0u8; 10];
    let read = |buf: &mut [u8; 64]| dev.read_timeout(buf, 20).unwrap_or(0);
    println!("Press each control when asked; touch the touch strip to skip one. Release before the next.");
    // The touch strip's bytes (28..34 of report 1): any change while waiting is a skip.
    let mut strip: Option<[u8; 6]> = None;
    for (i, name) in BUTTONS.iter().enumerate().skip(from - 1) {
        if done.iter().any(|d| d == name) {
            continue;
        }
        print!("[{}/{}] {name} … ", i + 1, BUTTONS.len());
        std::io::stdout().flush().ok();
        let spoken = name.split(" (").next().unwrap_or(name).replace("4-D", "four D");
        let _ = std::process::Command::new("say").arg(&spoken).spawn();
        let mut found = None;
        let mut skip = false;
        while found.is_none() && !skip {
            if read(&mut buf) > 34 && buf[0] == 1 {
                let s: [u8; 6] = buf[28..34].try_into().unwrap();
                if strip.is_some_and(|p| p[2..] != s[2..]) && s[2..] != [0, 0, 0, 0] {
                    skip = true;
                }
                strip = Some(s);
                for byte in 0..10 {
                    let free = (0..8).filter(|k| !assigned[byte * 8 + k]).fold(0u8, |m, k| m | 1 << k);
                    let new = buf[1 + byte] & !held[byte] & free;
                    if new != 0 {
                        found = Some(byte * 8 + new.trailing_zeros() as usize);
                    }
                    held[byte] = buf[1 + byte];
                }
            }
        }
        match found {
            Some(bit) => {
                println!("bit {bit} (byte {}, mask {:#04x})", 1 + bit / 8, 1u8 << (bit % 8));
                writeln!(file, "{bit}\t{name}").ok();
                assigned[bit] = true;
                // Wait for the release.
                let end = Instant::now() + Duration::from_secs(5);
                while Instant::now() < end {
                    if read(&mut buf) > 10 && buf[0] == 1 {
                        held.copy_from_slice(&buf[1..11]);
                        if held[bit / 8] & (1 << (bit % 8)) == 0 {
                            break;
                        }
                    }
                }
            }
            None => {
                println!("skipped");
                // Let go of the strip first.
                std::thread::sleep(Duration::from_millis(1500));
                while read(&mut buf) > 0 {}
                strip = None;
                writeln!(file, "-\t{name}").ok();
            }
        }
    }
    println!("wrote {out}");
}
