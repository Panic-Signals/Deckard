//! An installed VST3's parameters as DECKARD sees them: every one, and those the
//! automatic pages show; then turns the first shown one.
//!
//! Usage: cargo run --release -p deckard-plugins --example plugin-params <bundle.vst3>

use deckard_plugins::{Host, Slot, scan};

fn main() {
    let bundle = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: plugin-params <bundle.vst3>"));
    let entries = scan::vst3_from_moduleinfo(&bundle).map(Ok).unwrap_or_else(|| scan::vst3_by_loading(&bundle)).expect("scan");
    let e = entries.first().expect("a class");
    let id = e.maschine_id().unwrap();
    println!("{} ({id})", e.name);
    let mut host = Host::default();
    let inst = host.instance(Slot::Sound { group: 0, sound: 0 }, &id, &e.name, &[], 44_100.0).expect("load");
    let mut p = inst.lock().unwrap();
    let all = p.parameter_list(false);
    let shown = p.parameters();
    println!("{} parameters, {} shown, own pages {}", all.len(), shown.len(), p.pages().len());
    for i in all.iter().take(40) {
        let auto = shown.iter().any(|s| s.id == i.id);
        println!("  id {:10} {:28} short {:12} steps {} shown {auto}", i.id, i.title, i.short, i.steps);
    }
    let handle = p.params();
    if let Some(k) = shown.first() {
        let before = p.param_value(k.id);
        p.set_param(k.id, if before > 0.5 { 0.1 } else { 0.9 });
        println!("set {} {before:.3} -> {:.3} (controller handle: {})", k.title, p.param_value(k.id), handle.is_some());
        let t = deckard_plugins::Transport::default();
        for block in 1..=40 {
            p.process(512, &t);
            if block % 10 == 0 {
                println!("  after {block} blocks: {:.3}", p.param_value(k.id));
            }
        }
    }
    drop(p);
    deckard_plugins::exit(0)
}
