//! Print the MK3's HID input reports as they change (opened shared, next to NI's agent).
//! `cargo run -p deckard-mk3 --example mk3-probe -- [seconds]`

use std::time::{Duration, Instant};

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let api = hidapi::HidApi::new().expect("hidapi");
    // Shared with NI's agent (macOS; elsewhere HID devices open shared).
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    for d in api.device_list().filter(|d| d.vendor_id() == deckard_mk3::VENDOR) {
        println!(
            "{:04x}:{:04x} iface {} usage {:04x}/{:04x} {:?} {}",
            d.vendor_id(),
            d.product_id(),
            d.interface_number(),
            d.usage_page(),
            d.usage(),
            d.product_string(),
            d.path().to_string_lossy()
        );
    }
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3's HID interface");
    let mut last: std::collections::HashMap<u8, Vec<u8>> = Default::default();
    let mut buf = [0u8; 256];
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        let n = dev.read_timeout(&mut buf, 50).unwrap_or(0);
        if n == 0 {
            continue;
        }
        let r = buf[..n].to_vec();
        let id = r[0];
        let prev = last.get(&id);
        if prev != Some(&r) {
            let diff: Vec<String> = r
                .iter()
                .enumerate()
                .filter(|(i, b)| prev.is_none_or(|p| p.get(*i) != Some(*b)))
                .map(|(i, b)| format!("{i}={b:02x}"))
                .collect();
            println!("report {id:02x} len {n}: {}", if prev.is_none() { hex(&r) } else { diff.join(" ") });
            last.insert(id, r);
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}
