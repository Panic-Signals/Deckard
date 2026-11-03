//! An Audio Unit through DECKARD's engine: after an instrument in a new project (an effect),
//! or as the instrument; rendered, and rendered again from the project saved and reopened.
//!
//! `cargo run -p deckard --example au-engine-check -- <vendor> <type> <subtype>`
//! (runs on the main thread, as plug-ins need).

#[cfg(target_os = "macos")]
fn main() {
    use deckard_doc::{Document, NewModule};
    use deckard_engine::{Engine, Mode, SampleCache};
    let args: Vec<String> = std::env::args().collect();
    let code = |s: &str| u32::from_be_bytes(s.as_bytes().try_into().expect("a four-character code"));
    let (kind, sub) = (code(&args[2]), code(&args[3]));
    let id = deckard_plugins::state::au_id(&args[1], kind, sub).expect("an instrument or effect");
    println!("{id}");
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt();
    let render = |d: &Document, cache: &mut SampleCache| {
        let (mut e, report) = Engine::new(d.project(), d.path(), 48_000, Mode::Arrangement, cache);
        assert!(report.plugin_errors.is_empty(), "plug-in errors: {:?}", report.plugin_errors);
        e.sequencing = false;
        let mut out = vec![0f32; 2 * 48_000];
        e.note_on(0, 0, 60, 110, 960);
        for chunk in out.chunks_mut(1024) {
            e.process(chunk);
        }
        out
    };
    let mut d = Document::new_project().expect("a project");
    let mut cache = SampleCache::default();
    let module = NewModule::Plugin { id: id.clone(), name: args[3].clone(), chunk: Vec::new() };
    let instrument = module.is_instrument();
    if !instrument {
        let class = deckard_plugins::native::CLASSES.iter().find(|c| c.instrument).expect("an instrument of DECKARD's");
        d.add_slot(0, 0, 0, &NewModule::Plugin { id: class.plugin_id(), name: class.name.into(), chunk: Vec::new() }).expect("instrument");
    }
    let dry = render(&d, &mut cache);
    let at = d.project().groups[0].sounds[0].slots.len();
    d.add_slot(0, 0, at, &module).expect("add the Audio Unit");
    let wet = render(&d, &mut cache);
    let diff: Vec<f32> = dry.iter().zip(&wet).map(|(a, b)| a - b).collect();
    println!("without {:.5}, with {:.5}, difference {:.5}", rms(&dry), rms(&wet), rms(&diff));
    let ok = rms(&diff) > 1e-4;
    println!("{}", if ok { "OK: it changes the sound" } else { "FAIL: it did nothing" });
    deckard_plugins::exit(if ok { 0 } else { 1 });
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Audio Units are on macOS only");
}
