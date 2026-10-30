//! Print how far knob 1 has been turned, in the raw counts report 0x01 carries, to
//! measure the counts in a turn: turn it one full turn clockwise (a mark on the cap
//! back to where it started) and read the total. Also prints each raw value, to show
//! where it wraps. Opened shared, so it runs next to DECKARD.
//! `cargo run -p deckard-mk3 --release --example mk3-knob-log`

fn main() {
    let api = hidapi::HidApi::new().expect("hidapi");
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3's HID interface");
    let mut buf = [0u8; 64];
    let mut last: Option<u16> = None;
    let (mut total, mut lo, mut hi) = (0i64, u16::MAX, 0u16);
    println!("turn knob 1 one full turn clockwise; ctrl-c to stop");
    loop {
        let n = dev.read_timeout(&mut buf, 50).expect("read");
        if n < 14 || buf[0] != 1 {
            continue;
        }
        let v = u16::from_le_bytes([buf[12], buf[13]]);
        (lo, hi) = (lo.min(v), hi.max(v));
        if let Some(old) = last
            && old != v
        {
            // The smaller way round, whatever the wrap point (assumed under 4096).
            let d = (v as i64 - old as i64 + 2048).rem_euclid(4096) - 2048;
            let d = if d.abs() > 500 { (v as i64 - old as i64 + 500).rem_euclid(1000) - 500 } else { d };
            total += d;
            println!("raw {v:5}  step {d:+4}  total {total:+6}   (lowest {lo}, highest {hi})");
        }
        last = Some(v);
    }
}
