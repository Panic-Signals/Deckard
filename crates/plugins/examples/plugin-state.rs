//! Load every plug-in instrument of a project with its saved state, and check the
//! state the plug-in gives back is what Maschine saved.
//!
//! Usage: cargo run --release -p deckard-plugins --example plugin-state <file.mxprj> [group]

use deckard_plugins::{Host, Slot};

fn main() {
    let path = std::env::args().nth(1).expect("usage: plugin-state <file.mxprj> [group]");
    let only: Option<usize> = std::env::args().nth(2).and_then(|g| g.parse().ok());
    let bytes = std::fs::read(&path).expect("read file");
    let types = mxprj::types::Types::new();
    let root = mxprj::Project::parse(&bytes).expect("parse").decode(&types).expect("decode");
    let project = deckard_model::Project::from_decoded(root);
    let mut host = Host::default();
    for (g, group) in project.groups.iter().enumerate().filter(|(g, _)| only.is_none_or(|o| o == *g)) {
        for (s, sound) in group.sounds.iter().enumerate() {
            let Some(p) = &sound.plugin else { continue };
            let t = std::time::Instant::now();
            match host.instance(Slot::Sound { group: g, sound: s }, &p.id, &p.name, &p.chunk, 44_100.0) {
                Ok(inst) => {
                    let mut inst = inst.lock().unwrap();
                    println!(
                        "  takes sustain (CC 64): {}, mod wheel (CC 1): {}, pitch bend: {}",
                        inst.takes_controller(64),
                        inst.takes_controller(1),
                        inst.takes_controller(129)
                    );
                    let again = inst.save_chunk();
                    let same = again == p.chunk;
                    let first_diff = again.iter().zip(&p.chunk).position(|(a, b)| a != b);
                    println!(
                        "{} / {}: {} ({:?}) loaded in {:.2}s; state {} bytes, saved {} bytes, {}",
                        group.name,
                        sound.name,
                        p.name,
                        inst.id.format,
                        t.elapsed().as_secs_f64(),
                        p.chunk.len(),
                        again.len(),
                        if same { "identical".to_string() } else { format!("differs from byte {first_diff:?}") }
                    );
                }
                Err(e) => println!("{} / {}: {e}", group.name, sound.name),
            }
        }
    }
    deckard_plugins::exit(0)
}
