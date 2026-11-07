//! List the JAM's HID reports from its report descriptor: each report id, whether it is
//! input, output or feature, its size and how its fields are cut. Sends nothing.
//! `cargo run -p deckard-jam --example jam-hid-reports`

use std::collections::BTreeMap;

fn main() {
    let dev = deckard_jam::open().expect("open the JAM");
    println!("{:?}, {:?}", dev.get_manufacturer_string(), dev.get_product_string());
    let mut buf = [0u8; hidapi::MAX_REPORT_DESCRIPTOR_SIZE];
    let n = dev.get_report_descriptor(&mut buf).expect("report descriptor");
    let d = &buf[..n];
    println!("descriptor: {n} bytes");
    let (mut page, mut id, mut size, mut count) = (0u32, 0u32, 0u32, 0u32);
    let mut usages: Vec<u32> = Vec::new();
    // (kind, id) -> each field: bits a value, how many, the usages.
    let mut reports: BTreeMap<(&str, u32), Vec<String>> = BTreeMap::new();
    let mut bytes: BTreeMap<(&str, u32), u32> = BTreeMap::new();
    let mut i = 0;
    while i < d.len() {
        let p = d[i];
        let len = match p & 3 {
            3 => 4,
            x => x as usize,
        };
        let mut v = 0u32;
        for k in 0..len {
            v |= u32::from(*d.get(i + 1 + k).unwrap_or(&0)) << (8 * k);
        }
        match p & 0xfc {
            0x04 => page = v,
            0x74 => size = v,
            0x94 => count = v,
            0x84 => id = v,
            0x08 => usages.push(v),
            0x80 | 0x90 | 0xb0 => {
                let kind = match p & 0xfc {
                    0x80 => "input",
                    0x90 => "output",
                    _ => "feature",
                };
                *bytes.entry((kind, id)).or_default() += size * count;
                let list: Vec<String> = usages.iter().map(|u| format!("{u:#x}")).collect();
                reports.entry((kind, id)).or_default().push(format!("{count} x {size} bits, page {page:#x}, usages [{}]", list.join(" ")));
                usages.clear();
            }
            _ => {}
        }
        i += 1 + len;
    }
    for ((kind, id), fields) in &reports {
        println!("{kind:<8}{id:#04x}: {} bytes", bytes[&(*kind, *id)] / 8);
        for f in fields {
            println!("    {f}");
        }
    }
}
