//! An installed Audio Unit as DECKARD hosts it: its parameters, a block of audio, its state
//! saved and loaded into a new instance, and its window (for `seconds`, 0 for none).
//!
//! Usage: cargo run -p deckard-plugins --example au-check <type> <subtype> <vendor> [seconds]
//! e.g.   cargo run -p deckard-plugins --example au-check aufx PRTL Output 5

#[cfg(target_os = "macos")]
fn main() {
    use deckard_plugins::{Host, MAX_BLOCK, Slot, Transport, editor::Editor, state::au_id};
    let args: Vec<String> = std::env::args().collect();
    let code = |s: &str| u32::from_be_bytes(s.as_bytes().try_into().expect("a four-character code"));
    let (kind, sub, vendor) = (code(&args[1]), code(&args[2]), &args[3]);
    let seconds: f64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let id = au_id(vendor, kind, sub).expect("an instrument or effect");
    println!("id {id}");
    let mut host = Host::default();
    let inst = host.instance(Slot::Master { slot: 0 }, &id, &args[2], &[], 48_000.0).expect("load");
    let t = Transport { tempo: 120.0, playing: true, numerator: 4, denominator: 4, ..Transport::default() };
    let tone: Vec<f32> = (0..MAX_BLOCK).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    let chunk = {
        let mut p = inst.lock().unwrap();
        let shown = p.parameters();
        println!("{} parameters, {} shown", p.parameter_list(false).len(), shown.len());
        for i in shown.iter().take(12) {
            let v = p.param_value(i.id);
            println!("  {:10} {:28} steps {:3} = {:.3} {:?}", i.id, i.title, i.steps, v, p.param_text(i.id, v));
        }
        p.note_on(60, 100);
        let mut peak = 0f32;
        for _ in 0..100 {
            let (l, _) = p.process_input(MAX_BLOCK, &t, Some((&tone, &tone)));
            peak = l.iter().fold(peak, |m, s| m.max(s.abs()));
        }
        p.all_notes_off();
        println!("output peak {peak:.4}");
        p.save_chunk()
    };
    println!(
        "chunk: type {}, {} bytes",
        u32::from_le_bytes(chunk[4..8].try_into().unwrap()),
        u32::from_le_bytes(chunk[8..12].try_into().unwrap())
    );
    let again = host.instance(Slot::Master { slot: 1 }, &id, &args[2], &chunk, 48_000.0).expect("load with the chunk");
    let same = again.lock().unwrap().save_chunk() == chunk;
    println!("a new instance saves the same state: {same}");
    // Settings: turn every 3rd parameter, save, load into a third instance, compare.
    let turned = {
        let mut p = inst.lock().unwrap();
        for i in p.parameters().iter().step_by(3) {
            let v = p.param_value(i.id);
            p.set_param(i.id, if v > 0.5 { v - 0.3 } else { v + 0.3 });
        }
        p.save_chunk()
    };
    let third = host.instance(Slot::Master { slot: 2 }, &id, &args[2], &turned, 48_000.0).expect("load with the chunk");
    {
        let (a, b) = (inst.lock().unwrap(), third.lock().unwrap());
        let list = a.parameters();
        let wrong: Vec<_> =
            list.iter().filter(|i| (a.param_value(i.id) - b.param_value(i.id)).abs() > 1e-3).map(|i| i.title.clone()).collect();
        println!("restored {}/{} parameters; not: {wrong:?}", list.len() - wrong.len(), list.len());
    }
    drop(third);
    if seconds > 0.0 {
        let editor = Editor::open(&inst, &args[2]).expect("the window");
        println!("window open");
        let start = std::time::Instant::now();
        while editor.is_open() && start.elapsed().as_secs_f64() < seconds {
            editor.tick();
            inst.lock().unwrap().process_input(512, &t, Some((&tone[..512], &tone[..512])));
            let until = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.016);
            objc2_foundation::NSRunLoop::currentRunLoop().runUntilDate(&until);
        }
        drop(editor);
        println!("window closed");
    }
    drop(again);
    drop(inst);
    host.retain(|_| false);
    println!("released");
    deckard_plugins::exit(0)
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Audio Units are on macOS only");
}
