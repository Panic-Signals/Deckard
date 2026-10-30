//! Print what the MK3 sends for its pads, entry by entry, with the time: the raw event
//! (1 press, 4 pressure, 3 release, or anything else), the pressure, and what DECKARD
//! makes of it. Opened shared, so it runs next to DECKARD.
//! `cargo run -p deckard-mk3 --release --example mk3-pad-log`

use std::time::Instant;

fn main() {
    let api = hidapi::HidApi::new().expect("hidapi");
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3's HID interface");
    let mut decoder = deckard_mk3::Decoder::new();
    let start = Instant::now();
    let mut buf = [0u8; 64];
    println!("play the pads; ctrl-c to stop");
    loop {
        let n = dev.read_timeout(&mut buf, 2).expect("read");
        let mut events = Vec::new();
        if n == 0 {
            decoder.flush(&mut events);
        } else if buf[0] == 2 {
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            for e in buf[1..n].chunks_exact(3) {
                let word = u16::from_be_bytes([e[1], e[2]]);
                if word == 0 && e[0] == 0 {
                    break;
                }
                let pad = if e[0] < 16 { deckard_mk3::pad_number(e[0]).to_string() } else { format!("index {}", e[0]) };
                println!("{ms:10.1} ms  pad {pad:>2}  event {}  pressure {:4}", word >> 12, word & 0x0fff);
            }
            decoder.decode(&buf[..n], &mut events);
        }
        for e in events {
            if matches!(e, deckard_mk3::Event::PadOn(..) | deckard_mk3::Event::PadOff(..)) {
                println!("{:10.1} ms  -> {e:?}", start.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }
}
