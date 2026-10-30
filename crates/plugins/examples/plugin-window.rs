//! Open an installed VST3's own window, as DECKARD does, and keep it served until it
//! is closed (or `seconds` pass). The way to try a plug-in's editor on a system without
//! a project that uses it.
//!
//! Usage: cargo run --release -p deckard-plugins --example plugin-window <bundle.vst3> [seconds]

use deckard_plugins::{Host, Slot, editor::Editor, module, scan};
use std::time::{Duration, Instant};

fn main() {
    let bundle = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: plugin-window <bundle.vst3> [seconds]"));
    let seconds: f64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(60.0);
    if let Some(dir) = bundle.parent() {
        let mut dirs = module::search_paths();
        dirs.insert(0, dir.to_path_buf());
        module::set_search_paths(dirs);
    }
    let entries = scan::vst3_from_moduleinfo(&bundle).map(Ok).unwrap_or_else(|| scan::vst3_by_loading(&bundle)).expect("scan");
    let e = entries.first().expect("a class");
    let id = e.maschine_id().unwrap();
    println!("{} ({id})", e.name);
    let mut host = Host::default();
    let inst = host.instance(Slot::Sound { group: 0, sound: 0 }, &id, &e.name, &[], 44_100.0).expect("load");
    let editor = match Editor::open(&inst, &format!("{} — plugin-window", e.name)) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("could not open the window: {err}");
            deckard_plugins::exit(1)
        }
    };
    println!("window open; close it to end (or wait {seconds}s)");
    let start = Instant::now();
    let t = deckard_plugins::Transport::default();
    while editor.is_open() && start.elapsed().as_secs_f64() < seconds {
        // As the app does every frame: serve the editor, then run some audio.
        #[cfg(windows)]
        editor.pump_messages();
        editor.tick();
        inst.lock().unwrap().process(512, &t);
        std::thread::sleep(Duration::from_millis(16));
    }
    println!("{}", if editor.is_open() { "time's up" } else { "closed" });
    drop(editor);
    deckard_plugins::exit(0)
}
