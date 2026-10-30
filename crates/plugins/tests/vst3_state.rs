//! A hosted VST3's saved state, with an installed plug-in (skipped without one). Run
//! on the main thread, where plug-ins load (no test harness).

use deckard_plugins::{Host, Slot, Transport, scan};

const BUNDLE: &str = "/Library/Audio/Plug-Ins/VST3/Dexed.vst3";

fn main() {
    a_routed_parameter_saves_the_knobs_value();
    println!("vst3_state: ok");
    deckard_plugins::exit(0)
}

/// BUGS #16: a parameter a modulation route holds is saved at the knob's value, not
/// the route's, and goes on playing the route's.
fn a_routed_parameter_saves_the_knobs_value() {
    let bundle = std::path::Path::new(BUNDLE);
    if !bundle.exists() {
        eprintln!("no {BUNDLE}; skipping");
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
