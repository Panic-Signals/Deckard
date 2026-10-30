//! A hosted VST3's saved state, with an installed plug-in (skipped without one). Run
//! on the main thread, where plug-ins load (no test harness).

use deckard_plugins::{Host, Slot, Transport, scan};

/// The bundle to test with: `$DECKARD_TEST_VST3`, else Dexed in the system's folder.
fn bundle() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("DECKARD_TEST_VST3") {
        return p.into();
    }
    let dir = if cfg!(target_os = "macos") {
        "/Library/Audio/Plug-Ins/VST3"
    } else if cfg!(windows) {
        r"C:\Program Files\Common Files\VST3"
    } else {
        "/usr/lib/vst3"
    };
    std::path::Path::new(dir).join("Dexed.vst3")
}

fn main() {
    a_routed_parameter_saves_the_knobs_value();
    println!("vst3_state: ok");
    deckard_plugins::exit(0)
}

/// BUGS #16: a parameter a modulation route holds is saved at the knob's value, not
/// the route's, and goes on playing the route's.
fn a_routed_parameter_saves_the_knobs_value() {
    let bundle = bundle();
    let bundle = bundle.as_path();
    if !bundle.exists() {
        eprintln!("no {}; skipping", bundle.display());
        return;
    }
    let entries = scan::vst3_from_moduleinfo(bundle).map(Ok).unwrap_or_else(|| scan::vst3_by_loading(bundle)).expect("scan");
    let e = entries.first().expect("a class");
    let id = e.maschine_id().unwrap();
    let mut host = Host::default();
    let t = Transport::default();
    let a = host.instance(Slot::Sound { group: 0, sound: 0 }, &id, &e.name, &[], 44_100.0).expect("load");
    let (k, chunk) = {
        let mut p = a.lock().unwrap();
        let k = p.parameters().iter().find(|i| i.steps == 0).expect("a continuous parameter").id;
        p.set_param(k, 0.2);
        for _ in 0..4 {
            p.process(256, &t);
        }
        p.modulate(k, 0.9);
        for _ in 0..4 {
            p.process(256, &t);
        }
        let chunk = p.save_chunk();
        assert_eq!(p.modulated(k), Some(0.9), "still routed after saving");
        p.unmodulate(k);
        p.process(256, &t);
        assert!((p.param_value(k) - 0.2).abs() < 0.01, "the route gone, the knob's value is back: {}", p.param_value(k));
        (k, chunk)
    };
    let b = host.instance(Slot::Sound { group: 0, sound: 1 }, &id, &e.name, &chunk, 44_100.0).expect("reload");
    let v = b.lock().unwrap().param_value(k);
    assert!((v - 0.2).abs() < 0.01, "saved at the knob's 0.2, not the route's 0.9: {v}");
}
