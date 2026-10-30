//! Write copies of a project with timing edits (time signature, pattern length,
//! swing), for checking that Maschine opens them.
//!
//! Usage: cargo run --release -p deckard-doc --example timing-edits -- in.mxprj out-dir

use deckard_doc::{Document, Target};
use deckard_model::Swing;
use std::path::PathBuf;

type Edit = fn(&mut Document) -> Result<(), String>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, out) = (PathBuf::from(&args[0]), PathBuf::from(&args[1]));
    std::fs::create_dir_all(&out).unwrap();
    let edits: Vec<(&str, Edit)> = vec![
        ("time-3-4", |d| d.set_time_signature(3, 4)),
        ("time-6-8", |d| d.set_time_signature(6, 8)),
        ("pattern-length", |d| d.set_pattern_length(0, 0, 2 * 3840)),
        ("swing-group", |d| d.set_swing(Target::Group(0), Swing { amount: 40.0, cycle: 2, invert: false })),
        ("swing-master", |d| d.set_swing(Target::Song, Swing { amount: 25.0, cycle: 0, invert: true })),
        ("swing-sound", |d| d.set_swing(Target::Sound(0, 0), Swing { amount: 60.0, cycle: 1, invert: false })),
    ];
    let mut all = Document::open(&input).unwrap();
    for (name, edit) in &edits {
        let mut d = Document::open(&input).unwrap();
        edit(&mut d).unwrap_or_else(|e| panic!("{name}: {e}"));
        d.save_as(&out.join(format!("t-{name}.mxprj"))).unwrap();
        edit(&mut all).unwrap_or_else(|e| panic!("all/{name}: {e}"));
    }
    all.save_as(&out.join("t-all.mxprj")).unwrap();
    println!("wrote {} files to {}", edits.len() + 1, out.display());
}
