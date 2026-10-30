//! How many notes of each group move with 100 % swing, per swing cycle.
//! Usage: cargo run --release -p deckard-doc --example swing-check -- in.mxprj

use deckard_doc::{Document, Target};
use deckard_engine::{Mode, timeline};
use deckard_model::Swing;

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let d = Document::open(std::path::Path::new(&path)).unwrap();
    let base = timeline(d.project(), Mode::Arrangement);
    for g in 0..d.project().groups.len() {
        let mut row = format!("{:>2} {:<14}", g, d.project().groups[g].name);
        for cycle in [0, 2] {
            let mut e = Document::open(std::path::Path::new(&path)).unwrap();
            e.set_swing(Target::Group(g), Swing { amount: 100.0, cycle, invert: false }).unwrap();
            let t = timeline(e.project(), Mode::Arrangement);
            let notes = base.events.iter().filter(|x| x.group == g).count();
            let moved = base.events.iter().zip(&t.events).filter(|(a, b)| a.group == g && a.tick != b.tick).count();
            row += &format!("  cycle {}: {moved}/{notes} moved", deckard_model::SWING_CYCLES[cycle as usize].0);
        }
        println!("{row}");
    }
}
