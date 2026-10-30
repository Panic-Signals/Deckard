//! A plug-in's parameter pages as DECKARD shows them: the pages saved in the project
//! (or the automatic ones), with each knob's parameter, value and text.
//!
//! Usage: cargo run --release -p deckard-plugins --example plugin-pages <file.mxprj> <group> [page]

use deckard_plugins::{Host, Slot};

fn main() {
    let path = std::env::args().nth(1).expect("usage: plugin-pages <file.mxprj> <group> [page]");
    let g: usize = std::env::args().nth(2).and_then(|g| g.parse().ok()).expect("group");
    let page: usize = std::env::args().nth(3).and_then(|p| p.parse().ok()).unwrap_or(0);
    let bytes = std::fs::read(&path).expect("read file");
    let types = mxprj::types::Types::new();
    let root = mxprj::Project::parse(&bytes).expect("parse").decode(&types).expect("decode");
    let project = deckard_model::Project::from_decoded(root);
    let mut host = Host::default();
    let sound = project.groups[g].sounds.iter().position(|s| s.plugin.is_some()).expect("a plug-in sound");
    let p = project.groups[g].sounds[sound].plugin.clone().unwrap();
    let inst = host.instance(Slot::Sound { group: g, sound }, &p.id, &p.name, &p.chunk, 44_100.0).expect("load");
    let mut inst = inst.lock().unwrap();
    let all = inst.parameter_list(false);
    println!("{}: {} parameters ({} automatable), {} saved pages", p.name, all.len(), inst.parameters().len(), p.pages.len());
    if let Some(pg) = p.pages.get(page) {
        println!("page {} {:?}:", page + 1, pg.name);
        for k in &pg.params {
            let info = all.iter().find(|i| i.id == k.id);
            let v = inst.param_value(k.id);
            println!(
                "  id {:6} {:24} {:16} section {:?} value {:.3} = {}",
                k.id,
                info.map_or("?", |i| i.title.as_str()),
                if k.name.is_empty() { "-" } else { &k.name },
                k.section,
                v,
                inst.param_text(k.id, v)
            );
        }
    }
    // Turn the page's first knob, as the SOUND tab does.
    if let Some(k) = p.pages.get(page).and_then(|pg| pg.params.first()) {
        let before = inst.param_value(k.id);
        inst.set_param(k.id, 0.25);
        let chunk_changed = inst.save_chunk() != p.chunk;
        println!(
            "set id {} {before:.3} -> {:.3}; edited: {}; saved state changes: {chunk_changed}",
            k.id,
            inst.param_value(k.id),
            inst.take_dirty()
        );
    }
    deckard_plugins::exit(0)
}
