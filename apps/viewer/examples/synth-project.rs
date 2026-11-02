//! A new project whose sound 0/0 holds one internal instrument at its defaults: the source
//! for `maschine-ref make` when comparing an engine with Maschine's.
//!
//! `cargo run --release -p deckard --example synth-project -- <class> <out.mxprj>`

use deckard_doc::{Document, NewModule};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let [_, class, out] = &a[..] else {
        eprintln!("synth-project <class> <out.mxprj>");
        std::process::exit(2)
    };
    let mut d = Document::new_project().expect("a project");
    let params = deckard_engine::synth::defaults(class).expect("an internal instrument");
    let module = NewModule::Instrument { class: class.clone(), params: params.into_iter().map(|(k, v)| (k.to_string(), v)).collect() };
    d.add_slot(0, 0, 0, &module).expect("add it");
    d.save_as(std::path::Path::new(out)).expect("save");
}
