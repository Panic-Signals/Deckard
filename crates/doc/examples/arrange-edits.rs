//! Write one copy of a project per scene/arrangement edit (and one with all of them),
//! for checking that Maschine opens what we write.
//!
//! Usage: cargo run --release -p deckard-doc --example arrange-edits -- in.mxprj out-dir

use deckard_doc::Document;
use std::path::PathBuf;

type Edit = fn(&mut Document) -> Result<(), String>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, out) = (PathBuf::from(&args[0]), PathBuf::from(&args[1]));
    std::fs::create_dir_all(&out).unwrap();
    let edits: Vec<(&str, Edit)> = vec![
        ("assign", |d| d.assign_pattern(0, 1, Some(d.project().groups[1].patterns.len() - 1))),
        ("clear-slot", |d| d.assign_pattern(1, 0, None)),
        ("add-scene", |d| d.add_scene().map(drop)),
        ("dup-scene", |d| d.duplicate_scene(0).map(drop)),
        ("del-scene", |d| d.delete_scene(d.project().scenes.len() - 1)),
        ("dup-section", |d| d.duplicate_section(0).map(drop)),
        ("del-section", |d| d.delete_section(1)),
        ("section-length", |d| d.set_section_length(0, Some(12 * 3840))),
        ("move-section", |d| d.move_section(0, 2)),
        ("section-scene", |d| d.set_section_scene(1, 0)),
        ("add-section", |d| d.add_section(0).map(drop)),
    ];
    let mut all = Document::open(&input).unwrap();
    for (name, edit) in &edits {
        let mut d = Document::open(&input).unwrap();
        edit(&mut d).unwrap_or_else(|e| panic!("{name}: {e}"));
        d.save_as(&out.join(format!("e-{name}.mxprj"))).unwrap();
        edit(&mut all).unwrap_or_else(|e| panic!("all/{name}: {e}"));
    }
    all.save_as(&out.join("e-all.mxprj")).unwrap();
    println!("wrote {} files to {}", edits.len() + 1, out.display());
}
