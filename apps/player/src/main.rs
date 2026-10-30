//! deckard-player: play a Maschine project through the default audio output, or
//! render it to a WAV file.
//!
//! Usage: deckard-player <file.mxprj> [--render out.wav] [--scene N | --pattern G P]
//!                        [--seconds S] [--loop] [--rate HZ] [--events]
//!                        [--solo G[:S]]
//!
//! `--solo` plays only group G (or only its sound S); indices are 0-based.
//! `--events` prints the note events (seconds, group, sound, pitch, velocity) and exits.

use deckard_engine::output::Output;
use deckard_engine::{Engine, Mode, SampleCache};
use std::path::Path;

fn arg<T: std::str::FromStr>(args: &[String], flag: &str, n: usize) -> Option<T> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1 + n)).and_then(|v| v.parse().ok())
}

fn main() {
    run();
    // Skip loaded plug-ins' exit-time destructors (Massive's crash in them).
    deckard_plugins::exit(0)
}

fn run() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1).filter(|a| !a.starts_with("--")) else {
        eprintln!(
            "usage: {} <file.mxprj> [--render out.wav] [--scene N | --pattern G P] [--seconds S] [--loop] [--rate HZ] [--events] [--solo G[:S]] [--threads N] [--ahead MS]",
            args[0]
        );
        std::process::exit(2);
    };
    let mode = if let Some(n) = arg::<usize>(&args, "--scene", 0) {
        Mode::Scene(n)
    } else if let (Some(g), Some(p)) = (arg::<usize>(&args, "--pattern", 0), arg::<usize>(&args, "--pattern", 1)) {
        Mode::Pattern(g, p)
    } else {
        Mode::Arrangement
    };
    let looping = args.iter().any(|a| a == "--loop");
    let render_to: Option<String> = arg(&args, "--render", 0);
    let seconds: Option<f64> = arg(&args, "--seconds", 0);
    let solo: Option<(usize, Option<usize>)> = arg::<String>(&args, "--solo", 0).map(|v| {
        let mut it = v.split(':').map(|n| n.parse::<usize>().unwrap_or_else(|_| fail("--solo takes G or G:S")));
        (it.next().unwrap(), it.next())
    });

    let bytes = std::fs::read(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
    let doc = mxprj::Project::parse(&bytes).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
    let types = mxprj::types::Types::new();
    let root = doc.decode(&types).unwrap_or_else(|(e, at)| fail(&format!("{path}: {e} in {at}")));
    let project = deckard_model::Project::from_decoded(root);

    if args.iter().any(|a| a == "--events") {
        let spt = 60.0 / (project.tempo * deckard_model::PPQ as f64);
        println!("seconds\tgroup\tsound\tpitch\tvelocity");
        for e in deckard_engine::timeline(&project, mode).events {
            let what = match e.midi {
                None => format!("{}\t{}", e.pitch, e.velocity),
                Some(deckard_model::MidiKind::Controller { number, value }) => format!("cc{number}\t{value}"),
                Some(deckard_model::MidiKind::PitchBend(v)) => format!("bend\t{v}"),
                Some(deckard_model::MidiKind::ChannelPressure(v)) => format!("pressure\t{v}"),
            };
            println!("{:.6}\t{}\t{}\t{what}", e.tick as f64 * spt, e.group, e.sound);
        }
        return;
    }
    if let Some(out) = render_to {
        let rate = arg(&args, "--rate", 0).unwrap_or(44_100);
        let (mut engine, report) = Engine::new(&project, Some(Path::new(path)), rate, mode, &mut SampleCache::default());
        print_report(&project, &engine, &report);
        apply_solo(&mut engine, solo);
        engine.looping = looping && seconds.is_some();
        // --threads N: render sounds on N worker threads too (the result is the same).
        if let Some(n) = arg::<usize>(&args, "--threads", 0) {
            engine.pool = Some(std::sync::Arc::new(deckard_engine::pool::RenderPool::new(n, 512, rate)));
        }
        // --ahead MS: render sounds ahead on their players (the result is the same).
        if let Some(ms) = arg::<f64>(&args, "--ahead", 0) {
            engine.render_ahead((ms / 1000.0 * rate as f64) as u64, 2, 512, true);
        }
        let started = std::time::Instant::now();
        let audio = deckard_engine::render(&mut engine, seconds.unwrap_or(30.0 * 60.0));
        eprintln!("rendered in {:.2} s", started.elapsed().as_secs_f64());
        write_wav(&out, rate, &audio).unwrap_or_else(|e| fail(&format!("{out}: {e}")));
        let peak = audio.iter().fold(0f32, |m, s| m.max(s.abs()));
        eprintln!(
            "wrote {out}: {:.1} s, peak {:.2} ({:+.1} dBFS)",
            audio.len() as f64 / 2.0 / rate as f64,
            peak,
            20.0 * peak.max(1e-9).log10()
        );
        return;
    }
    play(&project, Path::new(path), mode, looping, seconds, solo);
}

fn apply_solo(engine: &mut Engine, solo: Option<(usize, Option<usize>)>) {
    if let Some((g, s)) = solo {
        engine.retain_sounds(|gi, si| gi == g && s.is_none_or(|s| s == si));
    }
}

fn play(
    project: &deckard_model::Project,
    path: &Path,
    mode: Mode,
    looping: bool,
    seconds: Option<f64>,
    solo: Option<(usize, Option<usize>)>,
) {
    let out = Output::new().unwrap_or_else(|e| fail(&e));
    let (mut engine, report) = Engine::new(project, Some(path), out.rate, mode, &mut SampleCache::default());
    print_report(project, &engine, &report);
    apply_solo(&mut engine, solo);
    engine.looping = looping;
    out.play(engine);
    eprintln!("playing at {} Hz on {} (Ctrl-C to stop)", out.rate, out.device);
    let start = std::time::Instant::now();
    while out.is_playing() {
        if seconds.is_some_and(|s| start.elapsed().as_secs_f64() >= s) {
            let bar = out.telemetry(|t| t.position) / project.bar_length() as f64;
            eprintln!("stopped at bar {:.2}", bar + 1.0);
            out.stop();
            std::thread::sleep(std::time::Duration::from_millis(50));
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn print_report(project: &deckard_model::Project, engine: &Engine, r: &deckard_engine::LoadReport) {
    let t = engine.timeline();
    eprintln!(
        "{}: {} BPM, {} notes over {:.1} bars; {} of {} sampler zones loaded",
        project.name,
        project.tempo,
        t.events.iter().filter(|e| e.midi.is_none()).count(),
        t.length as f64 / project.bar_length() as f64,
        r.loaded,
        r.zones
    );
    for m in r.missing.iter().take(5) {
        eprintln!("  missing: {m}");
    }
    if r.missing.len() > 5 {
        eprintln!("  ... {} more missing", r.missing.len() - 5);
    }
    for f in r.failed.iter().take(5) {
        eprintln!("  failed: {f}");
    }
    for p in &r.plugins {
        eprintln!("  plug-in: {p}");
    }
    for e in &r.plugin_errors {
        eprintln!("  plug-in failed: {e}");
    }
    if !r.no_sampler.is_empty() {
        eprintln!("  not played (no Sampler or plug-in to play them): {}", r.no_sampler.join(", "));
    }
}

fn write_wav(path: &str, rate: u32, interleaved: &[f32]) -> Result<(), hound::Error> {
    let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &s in interleaved {
        w.write_sample(s)?;
    }
    w.finalize()
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    deckard_plugins::exit(1)
}
