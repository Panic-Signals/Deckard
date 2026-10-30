//! Play a project through the real audio output (master turned all the way down)
//! and report how the audio callback keeps up, in three phases: playback alone;
//! with the UI's polling (meters, playhead: 60 times a second); and with an edit
//! (the engine rebuilt and adopted, as after every change in the app) every
//! `--edit-ms` milliseconds.
//!
//! ```text
//! cargo run --release -p deckard --example audio-stress -- <project.mxprj> [--seconds S] [--edit-ms MS] [--scene N] [--buffer FRAMES] [--one-core] [--ahead MS]
//! ```

use deckard_engine::output::Output;
use deckard_engine::{Engine, Mode, SampleCache};
use std::path::Path;
use std::time::{Duration, Instant};

fn arg<T: std::str::FromStr>(args: &[String], name: &str) -> Option<T> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("audio-stress <project.mxprj> [--seconds S] [--edit-ms MS] [--scene N]");
        deckard_plugins::exit(2)
    };
    let seconds: f64 = arg(&args, "--seconds").unwrap_or(8.0);
    let edit_ms: u64 = arg(&args, "--edit-ms").unwrap_or(250);
    let mode = arg::<usize>(&args, "--scene").map_or(Mode::Arrangement, Mode::Scene);
    let doc = deckard_doc::Document::open(Path::new(path)).unwrap_or_else(|e| {
        eprintln!("{e}");
        deckard_plugins::exit(1)
    });
    let multicore = !args.iter().any(|a| a == "--one-core");
    let ahead_ms: f64 = arg(&args, "--ahead").unwrap_or(0.0);
    let out =
        Output::with_options(arg(&args, "--buffer").unwrap_or(deckard_engine::output::DEFAULT_BUFFER), multicore).unwrap_or_else(|e| {
            eprintln!("{e}");
            deckard_plugins::exit(1)
        });
    let mut cache = SampleCache::default();
    let t = Instant::now();
    let (mut engine, report) = Engine::new(doc.project(), doc.path(), out.rate, mode, &mut cache);
    println!(
        "{} at {} Hz: built in {:.0} ms, {} plug-ins ({} failed)",
        Path::new(path).file_name().unwrap().to_string_lossy(),
        out.rate,
        t.elapsed().as_secs_f64() * 1000.0,
        report.plugins.len(),
        report.plugin_errors.len()
    );
    engine.master = 0.0;
    engine.looping = true;
    out.set_render_ahead(ahead_ms / 1000.0);
    // What the playing engine was built from, for incremental edits.
    let (mut edited, mut chains) = (doc.project().clone(), report.chains.clone());
    out.play(engine);
    std::thread::sleep(Duration::from_millis(500));
    let (slow_us, who) = deckard_engine::take_slowest();
    let name = cache
        .plugins
        .instances()
        .find(|(_, p)| std::sync::Arc::as_ptr(p) as usize == who)
        .map_or("?".into(), |(_, p)| p.lock().unwrap().name.clone());
    println!("warm-up (first 0.5 s): {}, slowest plug-in call {slow_us} µs ({name})", out.stats.summary());

    let snapshot = |out: &Output| {
        use std::sync::atomic::Ordering::Relaxed;
        let s = &out.stats;
        (s.callbacks.load(Relaxed), s.missed.load(Relaxed), s.late.load(Relaxed))
    };
    let skips = || deckard_engine::PLUGIN_SKIPS.load(std::sync::atomic::Ordering::Relaxed);
    let instances: Vec<_> = cache.plugins.instances().map(|(_, p)| p.clone()).collect();
    // Opening a plug-in's page lists its parameters (once).
    let t = Instant::now();
    let page_ids: Vec<u32> =
        instances.first().map_or(Vec::new(), |p| p.lock().unwrap().parameters().iter().take(8).map(|i| i.id).collect());
    println!("opening a page lists the parameters in {:.0} µs", t.elapsed().as_secs_f64() * 1e6);
    for phase in ["playback alone", "with UI polling", "with a plug-in page open", "turning a plug-in knob", "with UI polling and edits"] {
        let s0 = skips();
        let mut last_page = Instant::now();
        let mut page_us = 0f64;
        let (c0, m0, l0) = snapshot(&out);
        out.stats.peak_load.store(0, std::sync::atomic::Ordering::Relaxed);
        let start = Instant::now();
        let mut last_edit = Instant::now();
        let mut rebuild_ms: Vec<f64> = Vec::new();
        let mut full_ms: Vec<f64> = Vec::new();
        let mut adopt_us: Vec<f64> = Vec::new();
        while start.elapsed().as_secs_f64() < seconds {
            if phase != "playback alone" {
                // As the app reads meters and the playhead each frame.
                out.take_meters();
                out.telemetry(|t| (t.position, t.sequencing));
                out.zone_position(0, 0, 0);
            }
            // A plug-in's page refreshes its eight values every 100 ms (as the app does).
            if phase == "with a plug-in page open"
                && last_page.elapsed() >= Duration::from_millis(100)
                && let Some(p) = instances.first()
                && let Ok(p) = p.try_lock()
            {
                last_page = Instant::now();
                let t = Instant::now();
                // As the app does: through the controller, not holding the instance.
                if let Some(h) = p.params() {
                    drop(p);
                    page_us = page_us.max(t.elapsed().as_secs_f64() * 1e6);
                    for &id in &page_ids {
                        let _ = h.text(id, h.value(id));
                    }
                }
            }
            if phase == "turning a plug-in knob"
                && let (Some(p), Some(&id)) = (instances.first(), page_ids.first())
                && let Some(h) = p.lock().unwrap().params()
            {
                h.set(id, 0.5 + 0.3 * (start.elapsed().as_secs_f64() * 3.0).sin());
            }
            if phase.ends_with("edits") && last_edit.elapsed() >= Duration::from_millis(edit_ms) {
                last_edit = Instant::now();
                // A real edit: a sound's level, then an effect's setting, alternately.
                let mode = out.telemetry(|t| t.mode).unwrap_or(mode);
                let mut to = edited.clone();
                let n = rebuild_ms.len();
                if let Some(snd) = to.groups.iter_mut().flat_map(|g| g.sounds.iter_mut()).find(|s| !s.slots.is_empty()) {
                    if n.is_multiple_of(2) {
                        snd.level = if n.is_multiple_of(4) { 0.6 } else { 0.7 };
                    } else if let Some((_, v)) = snd.slots.iter_mut().skip(1).flat_map(|x| x.params.iter_mut()).next() {
                        *v = if n % 4 == 1 { 0.3 } else { 0.4 };
                    }
                }
                // Built in full (measured only), then incrementally (played).
                let t = Instant::now();
                let (full, _) = Engine::new(&to, doc.path(), out.rate, mode, &mut cache);
                full_ms.push(t.elapsed().as_secs_f64() * 1000.0);
                drop(full);
                let t = Instant::now();
                let (mut fresh, report) = Engine::new_after(&to, Some((&edited, &chains)), doc.path(), out.rate, mode, &mut cache);
                fresh.master = 0.0;
                rebuild_ms.push(t.elapsed().as_secs_f64() * 1000.0);
                (edited, chains) = (to, report.chains);
                let t = Instant::now();
                out.adopt(fresh, doc.project());
                adopt_us.push(t.elapsed().as_secs_f64() * 1e6);
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        let (c1, m1, l1) = snapshot(&out);
        let (slow_us, who) = deckard_engine::take_slowest();
        let slow_name = cache
            .plugins
            .instances()
            .find(|(_, p)| std::sync::Arc::as_ptr(p) as usize == who)
            .map_or("?".to_string(), |(slot, p)| format!("{} ({slot:?})", p.lock().map(|p| p.name.clone()).unwrap_or_default()));
        let peak = out.stats.peak_load.load(std::sync::atomic::Ordering::Relaxed) as f64 / 10.0;
        println!(
            "{phase:>28}: {} blocks ({} frames each), {} missed, {} late, {} plug-in blocks skipped, peak load {peak:.0} %",
            c1 - c0,
            out.block_frames(),
            m1 - m0,
            l1 - l0,
            skips() - s0
        );
        if slow_us > 0 {
            println!("{:>28}  slowest plug-in call {slow_us} µs: {slow_name}", "");
        }
        if page_us > 0.0 {
            println!("{:>28}  a page refresh holds the plug-in for up to {page_us:.0} µs", "");
        }
        if !rebuild_ms.is_empty() {
            let max = |v: &[f64]| v.iter().cloned().fold(0.0, f64::max);
            println!(
                "{:>28}  {} edits: full rebuild avg {:.2} / max {:.2} ms, incremental avg {:.3} / max {:.3} ms (UI thread), sending max {:.0} µs",
                "",
                rebuild_ms.len(),
                full_ms.iter().sum::<f64>() / full_ms.len() as f64,
                max(&full_ms),
                rebuild_ms.iter().sum::<f64>() / rebuild_ms.len() as f64,
                max(&rebuild_ms),
                max(&adopt_us)
            );
        }
    }
    println!("overall: {}", out.stats.summary());
    let (ahead, late) = out.telemetry(|t| (t.sounds_ahead, t.ahead_underruns));
    if ahead_ms > 0.0 {
        println!("rendered ahead ({ahead_ms} ms): {ahead} sounds, {late} times not ready");
    }
    if let Some((n, helped)) = out.render_threads() {
        println!("render threads: {n}, sounds they rendered: {helped}");
    }
    out.stop();
    std::thread::sleep(Duration::from_millis(200));
    deckard_plugins::exit(0)
}
