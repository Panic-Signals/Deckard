//! Watch the JAM: prints every byte that changes in its input reports, with the time.
//! Writes nothing to the controller, so it's safe next to Maschine. Press things and watch
//! what moves.
//! `cargo run -p deckard-jam --example jam-probe [seconds] [report]`
//! With `seconds` it stops by itself and says how often each report came; `report` (say
//! 0x02) shows that report alone. Prints the feature reports too with `features` in place
//! of `seconds`: reading them changes nothing that's been seen on the MK3's.

use deckard_jam::{FEATURES, changes, open};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dev = open().expect("open the JAM");
    if args.get(1).is_some_and(|a| a == "features") {
        for (id, len) in FEATURES {
            let mut buf = vec![0u8; len + 1];
            buf[0] = id;
            match dev.get_feature_report(&mut buf) {
                Ok(n) => println!("feature {id:#04x} ({n} bytes): {}", hex(&buf[..n])),
                Err(e) => println!("feature {id:#04x}: {e}"),
            }
        }
        return;
    }
    let seconds = args.get(1).and_then(|s| s.parse::<u64>().ok());
    let only = args.get(2).and_then(|s| u8::from_str_radix(s.trim_start_matches("0x"), 16).ok());

    let mut last: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
    println!("watching … (the first report of each id prints whole)");
    let start = Instant::now();
    let mut count: BTreeMap<u8, u64> = BTreeMap::new();
    let mut buf = [0u8; 256];
    while seconds.is_none_or(|s| start.elapsed() < Duration::from_secs(s)) {
        let n = dev.read_timeout(&mut buf, 100).expect("read");
        if n == 0 {
            continue;
        }
        let id = buf[0];
        *count.entry(id).or_default() += 1;
        if !last.contains_key(&id) && only.is_none_or(|o| o == id) {
            println!("{:8.3}s {id:#04x} ({n} bytes): {}", start.elapsed().as_secs_f64(), hex(&buf[..n]));
            last.insert(id, buf[..n].to_vec());
            continue;
        }
        let was = last.get(&id).cloned().unwrap_or_default();
        let c = changes(&was, &buf[..n]);
        if !c.is_empty() && only.is_none_or(|o| o == id) {
            let list: Vec<String> = c.iter().map(|x| format!("[{}] {:02x}->{:02x}", x.byte, x.was, x.now)).collect();
            println!("{:8.3}s {id:#04x}: {}", start.elapsed().as_secs_f64(), list.join("  "));
        }
        last.insert(id, buf[..n].to_vec());
    }
    println!("in {} s, reports by id: {count:02x?}", seconds.unwrap_or(0));
}
