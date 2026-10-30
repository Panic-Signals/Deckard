//! Read the MK3's feature reports (only reads; nothing is written to the controller).
//! `cargo run -p deckard-mk3 --example mk3-hid-features -- [id ...]` (hex ids; default
//! the small ones the descriptor lists: d0 d8 d9 da db f0 f2 f8 f9).

fn main() {
    let api = hidapi::HidApi::new().expect("hidapi");
    // Shared with NI's agent (macOS; elsewhere HID devices open shared).
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3's HID interface");
    let ids: Vec<u8> = std::env::args().skip(1).filter_map(|a| u8::from_str_radix(a.trim_start_matches("0x"), 16).ok()).collect();
    let ids = if ids.is_empty() { vec![0xd0, 0xd8, 0xd9, 0xda, 0xdb, 0xf0, 0xf2, 0xf8, 0xf9] } else { ids };
    for id in ids {
        let mut buf = [0u8; 256];
        buf[0] = id;
        match dev.get_feature_report(&mut buf) {
            Ok(n) => println!("0x{id:02x}: {:02x?}", &buf[..n]),
            Err(e) => println!("0x{id:02x}: {e}"),
        }
    }
}
