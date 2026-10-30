//! Check that a sound's effect plug-ins process it: render a sound of a project dry,
//! then with an effect plug-in added after its instrument, and compare.
//!
//! `cargo run --release -p deckard --example fx-check -- <project> <effect.vst3> [group] [sound]`
//! (runs on the main thread, as plug-ins need).

use deckard_doc::{Document, NewModule};
use deckard_engine::{Engine, Mode, SampleCache};
use std::path::Path;

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn render(d: &Document, g: usize, s: usize, cache: &mut SampleCache) -> Vec<f32> {
    let (mut e, report) = Engine::new(d.project(), d.path(), 48_000, Mode::Arrangement, cache);
    if !report.plugin_errors.is_empty() {
        eprintln!("plug-in errors: {:?}", report.plugin_errors);
    }
    e.sequencing = false;
    let mut out = vec![0f32; 2 * 48_000];
    e.note_on(g, s, 60, 110, 960);
    for chunk in out.chunks_mut(1024) {
        e.process(chunk);
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (project, bundle) = (Path::new(&args[1]), Path::new(&args[2]));
    let g: usize = args.get(3).and_then(|x| x.parse().ok()).unwrap_or(0);
    let s: usize = args.get(4).and_then(|x| x.parse().ok()).unwrap_or(0);
    let mut d = Document::open(project).expect("project");
    let entries = deckard_plugins::scan::vst3_from_moduleinfo(bundle)
        .or_else(|| deckard_plugins::scan::vst3_by_loading(bundle).ok())
        .expect("plug-in classes");
    let fx = entries.iter().find(|e| !e.instrument).expect("an effect class");
    println!("effect: {} ({})", fx.name, fx.maschine_id().unwrap_or_default());
    let mut cache = SampleCache::default();
    let dry = render(&d, g, s, &mut cache);
    let module = NewModule::Plugin { id: fx.maschine_id().unwrap(), name: fx.name.clone(), chunk: Vec::new() };
    let at = d.project().groups[g].sounds[s].slots.len();
    d.add_slot(g, s, at, &module).expect("add the effect");
    let wet = render(&d, g, s, &mut cache);
    let diff: Vec<f32> = dry.iter().zip(&wet).map(|(a, b)| a - b).collect();
    println!("dry rms {:.5}, with effect {:.5}, difference {:.5}", rms(&dry), rms(&wet), rms(&diff));
    // Bypassed: as dry.
    d.set_slot_active(g, s, at, false).expect("bypass");
    let bypassed = render(&d, g, s, &mut cache);
    let diff2: Vec<f32> = dry.iter().zip(&bypassed).map(|(a, b)| a - b).collect();
    println!("bypassed: difference from dry {:.6}", rms(&diff2));
    let processed = rms(&diff) > 1e-4;
    println!("{}", if processed { "OK: the effect changes the sound" } else { "FAIL: the effect did nothing" });
    // Without running plug-ins' exit handlers (some crash in them, as DECKARD avoids too).
    deckard_plugins::exit(if processed { 0 } else { 1 });
}
