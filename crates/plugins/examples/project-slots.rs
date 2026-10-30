//! Every plug-in slot in a project: its plug-in, its saved pages, and how many of the
//! saved knobs name a parameter the loaded plug-in has.
//!
//! Usage: cargo run --release -p deckard-plugins --example project-slots <file.mxprj>

use deckard_plugins::{Host, Slot};

fn main() {
    let path = std::env::args().nth(1).expect("usage: project-slots <file.mxprj>");
    let bytes = std::fs::read(&path).expect("read file");
    let types = mxprj::types::Types::new();
    let root = mxprj::Project::parse(&bytes).expect("parse").decode(&types).expect("decode");
    let project = deckard_model::Project::from_decoded(root);
    let mut host = Host::default();
    for (g, group) in project.groups.iter().enumerate() {
        for (s, sound) in group.sounds.iter().enumerate() {
            for (i, slot) in sound.slots.iter().enumerate() {
                let Some(p) = &slot.plugin else { continue };
                let saved: usize = p.pages.iter().map(|pg| pg.params.len()).sum();
                print!("{g}/{s} slot {i} {} ({}): {} saved pages, {saved} knobs", p.name, p.id, p.pages.len());
                match host.instance(Slot::Sound { group: g, sound: s }, &p.id, &p.name, &p.chunk, 44_100.0) {
                    Ok(inst) => {
                        let inst = inst.lock().unwrap();
                        let all = inst.parameter_list(false);
                        let found = p.pages.iter().flat_map(|pg| &pg.params).filter(|k| all.iter().any(|i| i.id == k.id)).count();
                        println!("; loaded, {} parameters, {found} saved knobs found", all.len());
                        for pg in p.pages.iter().take(2) {
                            println!("    page {:?}: ids {:?}", pg.name, pg.params.iter().map(|k| k.id).collect::<Vec<_>>());
                        }
                    }
                    Err(e) => println!("; not loaded: {e}"),
                }
            }
        }
    }
    deckard_plugins::exit(0)
}
