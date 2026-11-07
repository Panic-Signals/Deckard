//! An installed Audio Unit's output pairs as DECKARD hosts them: how many it reports, what
//! each is called, and the peak on each while a note plays (or a tone goes in, for an
//! effect).
//!
//! Usage: cargo run -p deckard-plugins --example au-outputs <type> <subtype> <vendor> [chunk-file]
//! e.g.   cargo run -p deckard-plugins --example au-outputs aumu NiK8 "Native Instruments"

#[cfg(target_os = "macos")]
fn main() {
    use deckard_plugins::{Host, MAX_BLOCK, Slot, Transport, state::au_id};
    let args: Vec<String> = std::env::args().collect();
    let code = |s: &str| u32::from_be_bytes(s.as_bytes().try_into().expect("a four-character code"));
    let (kind, sub, vendor) = (code(&args[1]), code(&args[2]), &args[3]);
    let chunk = args.get(4).map(|p| std::fs::read(p).expect("the chunk file")).unwrap_or_default();
    let id = au_id(vendor, kind, sub).expect("an instrument or effect");
    let mut host = Host::default();
    let inst = host.instance(Slot::Master { slot: 0 }, &id, &args[2], &chunk, 48_000.0).expect("load");
    let t = Transport { tempo: 120.0, playing: true, numerator: 4, denominator: 4, ..Transport::default() };
    let tone: Vec<f32> = (0..MAX_BLOCK).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    let mut p = inst.lock().unwrap();
    let n = p.output_count();
    println!("{id}: {n} output pair(s)");
    for b in 0..n {
        println!("  bus {b}: {:?} (present: {})", p.output_name(b), p.output_bus(b, MAX_BLOCK).is_some());
    }
    let mut peaks = vec![0f32; n];
    for pitch in [36, 38, 42, 48, 60, 72] {
        p.note_on(pitch, 110);
    }
    for _ in 0..200 {
        p.process_input(MAX_BLOCK, &t, Some((&tone, &tone)));
        for (b, peak) in peaks.iter_mut().enumerate() {
            if let Some((l, r)) = p.output_bus(b, MAX_BLOCK) {
                *peak = l.iter().chain(r).fold(*peak, |m, s| m.max(s.abs()));
            }
        }
    }
    p.all_notes_off();
    for (b, peak) in peaks.iter().enumerate() {
        println!("  bus {b} peak {peak:.4}");
    }
    drop(p);
    drop(inst);
    host.retain(|_| false);
    deckard_plugins::exit(0)
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Audio Units are on macOS only");
}
