//! Look at what the MK3's button LEDs actually render: hold bytes side by side, or flash a
//! candidate against a steady level (easier to judge than two steady lights).
//!
//! `cargo run -p deckard-mk3 --example mk3-led-probe -- [-s seconds] [-d on-ms off-ms] [byte …]`
//!
//! `cargo run -p deckard-mk3 --example mk3-led-probe -- 9e b0 c0 d0 e0 f0 fc ff`
//!
//! Buttons 1–8 take up to eight hex bytes, all at once; the ones left over stay dark. The
//! bytes are printed against the buttons they land on. With none given it probes both
//! families: the 2-bit indices under `0x80` and the duties over it.
//!
//! `--flash <steady> [candidate …]` holds button 1 at the steady byte and flashes buttons
//! 5–8, each between its candidate and the steady byte — a visible flicker means that
//! candidate is a different level. `--pairs` shows every state at once: bright and dim
//! steady, each flashing, plus dark and `0xff`.
//!
//! `-d` shapes the blink: `-d 900 120` is nearly steady with a tick, the default a square
//! wave. (A blink always reads as a blink — measured; no ratio passes for a steady level.)
//!
//! `--ramp <from> <to> [ms]` walks one byte through every value between the two on buttons
//! 2–8, with button 1 held at `0x9e` as the reference, printing each step: watch the row
//! move, and see whether any value lands between the two levels.
//!
//! `--ramp-feature <index> <from> <to> [ms]` does the same to byte `index` of the
//! controller's settings report (`0xf8`), with buttons 1 and 3 held bright and dim as
//! anchors — how a device-wide brightness would show itself.
//!
//! `--ramp-report <id> <index> <from> <to> [ms]` walks a byte of an output report
//! (`0xf3`, one byte, or `0xf4`, 32) with the same anchors: the reports the driver has
//! never written, tried for a device-wide level. A controller that starts behaving oddly
//! wants a replug.
//!
//! What the panel does with the bytes, and which levels DECKARD sends, is in
//! docs/mk3-usb.md.
use deckard_mk3::leds::Leds;
use std::time::Duration;

/// What to draw.
enum Mode {
    /// One byte per button, 1–8.
    Bytes(Vec<u8>),
    /// A steady byte on button 1, each candidate flashing against it on buttons 5–8.
    Flash { steady: u8, candidates: Vec<u8> },
    /// Every state the panel has, side by side.
    Pairs,
    /// One byte walked through every value, on buttons 2–8, button 1 as the reference.
    Ramp { from: u8, to: u8, ms: u64 },
    /// A byte of the settings report walked through every value, anchors lit.
    RampFeature { index: usize, from: u8, to: u8, ms: u64 },
    /// A byte of an output report walked through every value, anchors lit.
    RampReport { id: u8, index: usize, from: u8, to: u8, ms: u64 },
}

fn hex(a: &str) -> Option<u8> {
    u8::from_str_radix(a.trim_start_matches("0x"), 16).ok()
}

fn main() {
    let mut secs = 60u32;
    let mut duty = (250u64, 250u64);
    let mut bytes = Vec::new();
    let mut mode = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-s" | "--secs" => secs = args.next().and_then(|a| a.parse().ok()).unwrap_or(60),
            "-d" | "--duty" => {
                duty = (args.next().and_then(|a| a.parse().ok()).unwrap_or(250), args.next().and_then(|a| a.parse().ok()).unwrap_or(250));
            }
            "--pairs" => mode = Some(Mode::Pairs),
            "--ramp" => {
                let from = args.next().and_then(|a| hex(&a)).unwrap_or(0x80);
                let to = args.next().and_then(|a| hex(&a)).unwrap_or(0xff);
                let ms = args.next().and_then(|a| a.parse().ok()).unwrap_or(120);
                mode = Some(Mode::Ramp { from, to, ms });
            }
            "--ramp-report" => {
                let id = args.next().and_then(|a| hex(&a)).unwrap_or(0xf3);
                let index = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
                let from = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
                let to = args.next().and_then(|a| a.parse().ok()).unwrap_or(127);
                let ms = args.next().and_then(|a| a.parse().ok()).unwrap_or(120);
                mode = Some(Mode::RampReport { id, index, from, to, ms });
            }
            "--ramp-feature" => {
                let index = args.next().and_then(|a| a.parse().ok()).unwrap_or(9);
                let from = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
                let to = args.next().and_then(|a| a.parse().ok()).unwrap_or(50);
                let ms = args.next().and_then(|a| a.parse().ok()).unwrap_or(120);
                mode = Some(Mode::RampFeature { index, from, to, ms });
            }
            "--flash" => {
                let steady = args.next().and_then(|a| hex(&a)).unwrap_or(0x9e);
                let candidates: Vec<u8> = args.by_ref().map_while(|a| hex(&a)).take(4).collect();
                mode = Some(Mode::Flash { steady, candidates });
            }
            other => match hex(other) {
                Some(b) => bytes.push(b),
                None => println!("(ignoring {other}: a byte is hex, a flag is -s, -d, --flash or --pairs)"),
            },
        }
    }
    let mode = mode.unwrap_or_else(|| {
        if bytes.is_empty() { Mode::Bytes(vec![0x01, 0x09, 0x22, 0x7f, 0x9e, 0xc0, 0xd0, 0xf8]) } else { Mode::Bytes(bytes) }
    });
    // The blink's phases, and how many of them fit in the hold: `-d` can be slower than the
    // default, so counting `secs * 2` cycles would overrun the printed time.
    let cycle = (duty.0 + duty.1).max(1);
    let cycles = secs as u64 * 1000 / cycle;
    let dev = deckard_mk3::Device::open().expect("open the MK3");
    let leds = |phase: &[(usize, u8)]| {
        let mut l = Leds::default();
        for (button, byte) in phase {
            l.set(&format!("LED_SCREEN_BUTTON_{button}"), *byte);
        }
        l
    };
    let hold = |steady: &[(usize, u8)], flash: &[(usize, u8)]| {
        println!("(holding {secs} s)");
        for _ in 0..cycles {
            dev.write_leds(&leds(steady)).expect("write the LEDs");
            std::thread::sleep(Duration::from_millis(duty.0));
            dev.write_leds(&leds(flash)).expect("write the LEDs");
            std::thread::sleep(Duration::from_millis(duty.1));
        }
    };
    match &mode {
        Mode::Bytes(list) => {
            let mut phase = Vec::new();
            for (i, byte) in list.iter().enumerate() {
                println!("button {}: 0x{byte:02x}", i + 1);
                phase.push((i + 1, *byte));
            }
            println!("(holding {secs} s)");
            for _ in 0..secs * 2 {
                dev.write_leds(&leds(&phase)).expect("write the LEDs");
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        Mode::Flash { steady, candidates } => {
            let mut on = vec![(1, *steady)];
            let mut off = vec![(1, *steady)];
            println!("button 1: 0x{steady:02x} steady");
            for (k, candidate) in candidates.iter().take(4).enumerate() {
                println!("button {}: flashing 0x{steady:02x} / 0x{candidate:02x}", 5 + k);
                on.push((5 + k, *candidate));
                off.push((5 + k, *steady));
            }
            if candidates.is_empty() {
                println!("(no candidates: give one to four bytes after --flash)");
            }
            hold(&off, &on);
        }
        Mode::Ramp { from, to, ms } => {
            let (from, to, ms) = (*from, *to, *ms);
            println!("button 1: 0x9e (reference); buttons 2-8 ramping 0x{from:02x} -> 0x{to:02x}, {ms} ms a step");
            let range: Vec<u8> = if from <= to { (from..=to).collect() } else { (to..=from).rev().collect() };
            for byte in range {
                let mut phase = vec![(1, 0x9e)];
                for button in 2..=8 {
                    phase.push((button, byte));
                }
                dev.write_leds(&leds(&phase)).expect("write the LEDs");
                println!("  0x{byte:02x}");
                std::thread::sleep(Duration::from_millis(ms));
            }
        }
        Mode::RampFeature { index, from, to, ms } => {
            let (index, from, to, ms) = (*index, *from, *to, *ms);
            let mut report = dev.read_feature(0xf8).expect("read the controller's settings");
            let Some(was) = report.get(index).copied() else {
                println!("byte index {index} is past the report's {} bytes", report.len());
                return;
            };
            println!("0xf8 byte {index} was 0x{was:02x}; ramping {from} -> {to}, {ms} ms a step");
            println!("button 1: 0x9e (bright anchor), button 3: 0xd0 (dim anchor)");
            let range: Vec<u8> = if from <= to { (from..=to).collect() } else { (to..=from).rev().collect() };
            for value in range {
                let mut anchors = Leds::default();
                anchors.set("LED_SCREEN_BUTTON_1", 0x9e);
                anchors.set("LED_SCREEN_BUTTON_3", 0xd0);
                dev.write_leds(&anchors).expect("write the LEDs");
                report[index] = value;
                dev.write_feature(&report).expect("write the controller's settings");
                println!("  byte {index} = {value} (0x{value:02x})");
                std::thread::sleep(Duration::from_millis(ms));
            }
            // Put the byte back as it was, so a sweep leaves no trace.
            report[index] = was;
            dev.write_feature(&report).ok();
            println!("byte {index} back to 0x{was:02x}");
        }
        Mode::RampReport { id, index, from, to, ms } => {
            let (id, index, from, to, ms) = (*id, *index, *from, *to, *ms);
            // `0xf4` is 32 bytes of payload, everything else one: the id is byte 0.
            let len = if id == 0xf4 { 33 } else { 2 };
            println!("0x{id:02x} byte {index} ramping {from} -> {to}, {ms} ms a step (a {len}-byte report)");
            println!("button 1: 0x9e (bright anchor), button 3: 0xd0 (dim anchor)");
            let range: Vec<u8> = if from <= to { (from..=to).collect() } else { (to..=from).rev().collect() };
            for value in range {
                let mut anchors = Leds::default();
                anchors.set("LED_SCREEN_BUTTON_1", 0x9e);
                anchors.set("LED_SCREEN_BUTTON_3", 0xd0);
                dev.write_leds(&anchors).expect("write the LEDs");
                let mut report = vec![0u8; len];
                report[0] = id;
                if let Some(byte) = report.get_mut(1 + index) {
                    *byte = value;
                }
                dev.write(&report).expect("write the report");
                println!("  byte {index} = {value} (0x{value:02x})");
                std::thread::sleep(Duration::from_millis(ms));
            }
            let mut off = vec![0u8; len];
            off[0] = id;
            dev.write(&off).ok();
        }
        Mode::Pairs => {
            let steps: [(u8, u8, &str); 8] = [
                (0x9e, 0x9e, "0x9e steady (bright)"),
                (0x9e, 0x00, "0x9e / off flashing (bright + flash)"),
                (0xd0, 0xd0, "0xd0 steady (dim)"),
                (0xd0, 0x9e, "0xd0 / 0x9e flashing (dim + flash)"),
                (0x00, 0x00, "off steady"),
                (0x00, 0x9e, "off / 0x9e flashing (off + flash)"),
                (0xff, 0xff, "0xff steady (max drive: bright)"),
                (0xff, 0x00, "0xff / off flashing"),
            ];
            let mut first = Vec::new();
            let mut second = Vec::new();
            for (k, (steady_byte, flash_byte, what)) in steps.iter().enumerate() {
                println!("button {}: {what}", k + 1);
                first.push((k + 1, *steady_byte));
                second.push((k + 1, *flash_byte));
            }
            hold(&first, &second);
        }
    }
    dev.write_leds(&Leds::default()).ok();
}
