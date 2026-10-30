//! Write one copy of a project per kind of note edit (and one with all of them), for
//! checking that Maschine opens what we write.
//!
//! Usage: cargo run --release -p deckard-doc --example note-edits -- in.mxprj out-dir

use deckard_doc::Document;
use deckard_model::Note;
use std::path::PathBuf;

type Edit = fn(&mut Document) -> Result<(), String>;

/// Group 0's first pattern with notes, and a sound without notes in it.
fn target(d: &Document) -> (usize, usize) {
    let g = &d.project().groups[0];
    let p = g.patterns.iter().position(|p| !p.notes.is_empty()).unwrap();
    let s = (0..g.sounds.len()).find(|s| g.patterns[p].notes.iter().all(|n| n.sound != *s)).unwrap();
    (p, s)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, out) = (PathBuf::from(&args[0]), PathBuf::from(&args[1]));
    std::fs::create_dir_all(&out).unwrap();
    let edits: Vec<(&str, Edit)> = vec![
        ("add", |d| {
            let (p, _) = target(d);
            let n = d.project().groups[0].patterns[p].notes[0];
            d.add_note(0, p, Note { position: n.position + 480, ..n })
        }),
        ("new-sound", |d| {
            let (p, s) = target(d);
            let notes: Vec<Note> = (0..4)
                .map(|i| Note { expr: None, sound: s, position: i * 960, length: 240, pitch: 60, velocity: 90, probability: 100 })
                .collect();
            d.edit_notes("Draw Notes", 0, p, &[], &notes)
        }),
        ("delete-all", |d| {
            let (p, _) = target(d);
            let s = d.project().groups[0].patterns[p].notes[0].sound;
            let all: Vec<Note> = d.project().groups[0].patterns[p].notes.iter().filter(|n| n.sound == s).copied().collect();
            d.delete_notes(0, p, &all)
        }),
        ("change", |d| {
            let (p, _) = target(d);
            let n = d.project().groups[0].patterns[p].notes[0];
            d.change_note(0, p, n, Note { position: n.position + 240, length: 480, velocity: 50, ..n })
        }),
        ("probability", |d| {
            let (p, _) = target(d);
            let n = d.project().groups[0].patterns[p].notes[0];
            d.change_note(0, p, n, Note { probability: 50, ..n })
        }),
    ];
    let mut all = Document::open(&input).unwrap();
    for (name, edit) in &edits {
        let mut d = Document::open(&input).unwrap();
        edit(&mut d).unwrap_or_else(|e| panic!("{name}: {e}"));
        d.save_as(&out.join(format!("n-{name}.mxprj"))).unwrap();
        edit(&mut all).unwrap_or_else(|e| panic!("all/{name}: {e}"));
    }
    all.save_as(&out.join("n-all.mxprj")).unwrap();
    println!("wrote {} files to {}", edits.len() + 1, out.display());
}
