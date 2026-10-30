//! Set one byte of a display's settings (feature report 0xf8 left, 0xf9 right) and read
//! it back, to learn which sets the backlight. The report reads
//! `e0 01 10 01 60 01 14 32 00`: 480, 272, then single bytes.
//! `cargo run -p deckard-mk3 --example mk3-display-brightness -- <0|1> <byte index 1..9> <value>`
//! (no value: just print both reports).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let api = hidapi::HidApi::new().expect("hidapi");
    // Shared with NI's agent (macOS; elsewhere HID devices open shared).
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3's HID interface");
    let read = |id: u8| {
        let mut buf = [0u8; 64];
        buf[0] = id;
        let n = dev.get_feature_report(&mut buf).expect("read the display settings");
        buf[..n].to_vec()
    };
    if let [display, index, value] = &args[..] {
        let id = if display == "0" { 0xf8 } else { 0xf9 };
        let index: usize = index.parse().expect("byte index");
        let value = u8::from_str_radix(value.trim_start_matches("0x"), if value.starts_with("0x") { 16 } else { 10 }).expect("value");
        let mut report = read(id);
        println!("before 0x{id:02x}: {report:02x?}");
        report[index] = value;
        dev.send_feature_report(&report).expect("write the display settings");
    }
    for id in [0xf8, 0xf9] {
        println!("0x{id:02x}: {:02x?}", read(id));
    }
}
