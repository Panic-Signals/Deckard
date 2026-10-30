//! How evenly live notes come out: notes sent from another thread at known moments
//! (as pads or MIDI would) play DECKARD's Replicant through the real output (master
//! turned down; the sound is recorded before it), and each note's onset is compared
//! with when it was sent. Run with sample-accurate timing on, then off.
//!
//! ```text
//! cargo run --release -p deckard --example timing-test -- [--buffer FRAMES] [--notes N]
//! ```

use deckard_engine::output::Output;
use deckard_engine::{Engine, Live, Mode, SampleCache, Tap, TapSource};
use deckard_model::{Group, PluginRef, Project, SlotInfo, Sound};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn arg<T: std::str::FromStr>(args: &[String], name: &str) -> Option<T> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok())
}

fn project() -> Project {
    let class = deckard_plugins::native::CLASSES[0];
    let plugin = PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
    Project {
        tempo: 120.0,
        numerator: 4,
        denominator: 4,
        groups: vec![Group {
            level: 0.75,
            sounds: vec![Sound {
                level: 0.75,
                instrument: "PluginHost".into(),
                plugin: Some(plugin.clone()),
                slots: vec![SlotInfo {
                    module: "PluginHost".into(),
                    name: class.name.into(),
                    active: true,
                    plugin: Some(plugin),
                    params: Vec::new(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Onsets (frames) in `x` (interleaved stereo): the first loud sample after a silence.
fn onsets(x: &[f32], rate: f64) -> Vec<usize> {
    let quiet_needed = (0.03 * rate) as usize;
    let mut quiet = quiet_needed;
    let mut out = Vec::new();
    for (f, v) in x.chunks(2).enumerate() {
        if v[0].abs() > 0.02 {
            if quiet >= quiet_needed {
                out.push(f);
            }
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
    out
}

fn run(out: &Output, exact: bool, notes: usize) -> Vec<f64> {
    out.set_sample_accurate(exact);
    // As in the app: the group played has the focus (it plays live, not ahead).
    out.set_render_ahead(std::env::var("AHEAD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.02));
    out.set_focus(Some(0));
    let (mut engine, _) = Engine::new(&project(), None, out.rate, Mode::Arrangement, &mut SampleCache::default());
    engine.sequencing = false;
    engine.master = 0.0;
    out.play(engine);
    std::thread::sleep(Duration::from_millis(300));
    let buffer: Arc<Mutex<Vec<f32>>> = Arc::default();
    out.set_tap(Some(Tap { source: TapSource::Sound(0, 0), buffer: buffer.clone() }));
    std::thread::sleep(Duration::from_millis(100));
    // Drain what came before, and note when recording "starts" by the tap's count.
    buffer.lock().unwrap().clear();
    let t0 = Instant::now();
    let mut sent = Vec::new();
    let mut recorded: Vec<f32> = Vec::new();
    // Notes 180 ms apart (plus an odd 0–7 ms so they fall anywhere in a block).
    let sender = out.sender();
    for i in 0..notes {
        let due = t0 + Duration::from_micros(200_000 + i as u64 * 180_000 + (i as u64 * 2_713) % 7_000);
        while Instant::now() < due {
            std::hint::spin_loop();
        }
        sent.push(Instant::now().duration_since(t0).as_secs_f64());
        sender.live(Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 110, length: 60 });
        recorded.extend(buffer.lock().unwrap().drain(..));
    }
    std::thread::sleep(Duration::from_millis(400));
    recorded.extend(buffer.lock().unwrap().drain(..));
    out.set_tap(None);
    eprintln!("sounds ahead during the run: {}", out.telemetry(|t| t.sounds_ahead));
    out.stop();
    let rate = out.rate as f64;
    let on = onsets(&recorded, rate);
    // Each onset relative to its send time; the spread of that is the jitter.
    on.iter().zip(&sent).map(|(&f, &s)| f as f64 / rate - s).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let buffer = arg(&args, "--buffer").unwrap_or(256u32);
    let notes = arg(&args, "--notes").unwrap_or(30usize);
    let out = Output::with_options(buffer, true).unwrap_or_else(|e| {
        eprintln!("{e}");
        deckard_plugins::exit(1)
    });
    println!(
        "buffer {} frames at {} Hz ({:.2} ms)",
        out.block_frames().max(buffer as usize),
        out.rate,
        buffer as f64 / out.rate as f64 * 1000.0
    );
    for exact in [true, false] {
        let d = run(&out, exact, notes);
        if d.len() < notes / 2 {
            println!("{}: only {} onsets found", if exact { "sample-accurate" } else { "next block" }, d.len());
            continue;
        }
        let mean = d.iter().sum::<f64>() / d.len() as f64;
        let (lo, hi) = d.iter().fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x - mean), b.max(x - mean)));
        let sd = (d.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / d.len() as f64).sqrt();
        println!(
            "{:>16}: {} notes, timing spread {:.2} ms (min {:+.2} / max {:+.2} ms, sd {:.3} ms)",
            if exact { "sample-accurate" } else { "next block" },
            d.len(),
            (hi - lo) * 1000.0,
            lo * 1000.0,
            hi * 1000.0,
            sd * 1000.0
        );
    }
    deckard_plugins::exit(0)
}
