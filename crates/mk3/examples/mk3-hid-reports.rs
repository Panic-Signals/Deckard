//! List the MK3's HID reports from its report descriptor: each report id, whether it is
//! input, output or feature, its size in bytes and the usages in it. Opened shared, so
//! it runs next to DECKARD or NI's agent.
//! `cargo run -p deckard-mk3 --example mk3-hid-reports`

use std::collections::BTreeMap;

#[derive(Default)]
struct Report {
    bits: u32,
    usages: Vec<String>,
}

fn main() {
    let api = hidapi::HidApi::new().expect("hidapi");
    // Shared with NI's agent (macOS; elsewhere HID devices open shared).
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3's HID interface");
    let mut buf = [0u8; hidapi::MAX_REPORT_DESCRIPTOR_SIZE];
    let n = dev.get_report_descriptor(&mut buf).expect("report descriptor");
    let d = &buf[..n];
    println!("descriptor: {n} bytes");

    // Global and local items as the HID spec defines them; main items close a field.
    let (mut page, mut id, mut size, mut count) = (0u32, 0u32, 0u32, 0u32);
    let mut usages: Vec<u32> = Vec::new();
    let mut reports: BTreeMap<(&str, u32), Report> = BTreeMap::new();
    let mut i = 0;
    while i < d.len() {
        let prefix = d[i];
        let len = [0, 1, 2, 4][(prefix & 3) as usize];
        let mut v = 0u32;
        for k in 0..len {
            v |= (*d.get(i + 1 + k).unwrap_or(&0) as u32) << (8 * k);
        }
        i += 1 + len;
        match prefix & 0xfc {
            0x04 => page = v,
            0x84 => id = v,
            0x74 => size = v,
            0x94 => count = v,
            0x08 => usages.push(if len == 4 { v } else { (page << 16) | v }),
            0x80 | 0x90 | 0xb0 => {
                let kind = match prefix & 0xfc {
                    0x80 => "input",
                    0x90 => "output",
                    _ => "feature",
                };
                let r = reports.entry((kind, id)).or_default();
                r.bits += size * count;
                for u in usages.drain(..) {
                    r.usages.push(format!("{:04x}:{:04x}", u >> 16, u & 0xffff));
                }
            }
            0xa0 | 0xc0 => usages.clear(),
            _ => {}
        }
    }
    for ((kind, id), r) in &reports {
        let shown: Vec<_> = r.usages.iter().take(8).cloned().collect();
        let more = if r.usages.len() > 8 { format!(" … ({} usages)", r.usages.len()) } else { String::new() };
        println!("{kind:7} 0x{id:02x}: {:4} bytes  {}{more}", r.bits.div_ceil(8), shown.join(" "));
    }
}
