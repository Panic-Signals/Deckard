//! The pictures in the DECKARD manual's "Controlling modular synths" chapter, rendered
//! (`cargo test -p deckard -- --ignored manual_shots`) into `$DECKARD_SHOTS` (else the
//! temporary folder): the CV only outputs, the LFO panel's CV section, Zhora and the
//! Tuner in the window, and the MK3's screens for each.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use deckard_model::mod_matrix::{CvRoute, ModSource};
use deckard_plugins::Transport;
use deckard_plugins::native::TuneState;
use deckard_surface::Action as A;
use eframe::egui;
use egui_kittest::Harness;

use crate::App;
use crate::layout::{Act, AddWhat, ChainTab, apply_acts, native_entry};
use deckard_doc::Chain;

fn dir() -> PathBuf {
    let d = std::env::var_os("DECKARD_SHOTS").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("deckard-manual"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The Control area, below the arranger.
const CONTROL: (u32, u32, u32, u32) = (10, 390, 1250, 140);

fn drag(h: &mut Harness<'static, App>, from: egui::Pos2, to: egui::Pos2) {
    use egui::{Event, PointerButton};
    h.hover_at(from);
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for i in 1..=10 {
        h.hover_at(from + (to - from) * (i as f32 / 10.0));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
}

fn harness(app: App) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0 + crate::layout::CONTROLLER_WIDTH, 930.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    h.run_steps(6);
    h
}

/// The whole window, then `crop` (x, y, w, h) of it if given.
fn window(h: &mut Harness<'static, App>, name: &str, crop: Option<(u32, u32, u32, u32)>) {
    h.run_steps(4);
    let img = h.render().unwrap();
    let img = match crop {
        Some((x, y, w, ht)) => image::imageops::crop_imm(&img, x, y, w, ht).to_image(),
        None => img,
    };
    let path = dir().join(name);
    img.save(&path).unwrap();
    eprintln!("{}", path.display());
}

/// Both of the MK3's screens, after a few frames, in the Maschine theme.
fn mk3(h: &mut Harness<'static, App>, name: &str) {
    h.run_steps(4);
    let app = h.state_mut();
    app.surface.set_theme("maschine");
    let px = app.surface.picture(&app.surface_view());
    let path = dir().join(name);
    let f = std::fs::File::create(&path).unwrap();
    let mut e = png::Encoder::new(f, 960, 272);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&px.concat()).unwrap();
    eprintln!("{}", path.display());
}

/// An output picker's value for channel `ch` (None is 0).
fn on(ch: usize) -> f64 {
    (ch as f64 + 1.0) / 32.0
}

/// A new project on settings for a modular: Out 2 L to Out 4 R CV only, with names, and
/// In 1 L named for what comes back.
fn modular_app() -> App {
    let mut app = App { settings: crate::settings::Settings::default(), ..App::default() };
    for (i, alias) in [(2, "Pitch"), (3, "Gate"), (4, "Clock"), (5, "Env"), (6, "Filter"), (7, "Fold")] {
        app.settings.output_cv_only[i] = true;
        app.settings.output_aliases[i] = alias.into();
    }
    for i in 0..8 {
        app.settings.output_routing[i] = Some(i);
    }
    app.settings.input_aliases[0] = "Modular".into();
    quiet(&mut app);
    app.new_project();
    app
}

/// No audio device: what the plug-ins hear is only what the test feeds them.
fn quiet(app: &mut App) {
    app.output = Some(Err("no audio for the manual's pictures".into()));
}

fn set(app: &mut App, chain: Chain, slot: usize, id: u32, v: f64) {
    apply_acts(app, vec![Act::PluginParamIn(chain, slot, id, v)]);
}

/// Feeds `hz` (none: silence) to the plug-in in `slot` of `chain` until it has
/// analysed `hops` more blocks.
fn feed(app: &mut App, chain: Chain, slot: usize, hz: Option<f64>, hops: u32, effect: bool) {
    let inst = app.samples.plugins.existing(app.plugin_slot_key(chain, slot)).unwrap();
    let rate = app.output_rate();
    let t = Transport { tempo: 120.0, playing: false, quarters: 0.0, numerator: 4, denominator: 4 };
    let mut phase = 0.0f64;
    let deadline = Instant::now() + Duration::from_secs(30);
    let start = inst.lock().unwrap().readout().unwrap().analysed;
    let mut sent = 0u32;
    while inst.lock().unwrap().readout().unwrap().analysed.wrapping_sub(start) < hops {
        let mut b = [0f32; 1024];
        if let Some(hz) = hz {
            for s in &mut b {
                *s = (0.4 * (phase * std::f64::consts::TAU).sin()) as f32;
                phase = (phase + hz / rate).fract();
            }
        }
        {
            let mut p = inst.lock().unwrap();
            if effect {
                p.process_input(1024, &t, Some((&b, &b)));
            } else {
                p.input(&b, &b);
                p.process(1024, &t);
            }
        }
        sent += 1;
        if sent % 4 == 0 {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(Instant::now() < deadline, "never analysed");
    }
    // What it shows, now read.
    let mut p = inst.lock().unwrap();
    if effect {
        p.process_input(16, &t, Some((&[0.0; 16], &[0.0; 16])));
    }
}

/// Runs a Tune on sound 1's Zhora, answered by a simulated oscillator that tracks 1 V
/// an octave from C3 a little flat, until it ends.
fn tune(app: &mut App) {
    let chain = Chain::Sound(0, 0);
    let inst = app.samples.plugins.existing(app.plugin_slot_key(chain, 0)).unwrap();
    let rate = app.output_rate();
    let (mut per, mut r) = (1u32, rate);
    while r > 60_000.0 {
        (per, r) = (per * 2, r / 2.0);
    }
    let t = Transport { tempo: 120.0, playing: false, quarters: 0.0, numerator: 4, denominator: 4 };
    inst.lock().unwrap().process(1024, &t);
    let before = inst.lock().unwrap().readout().unwrap().analysed;
    app.zhora_press("tune");
    let deadline = Instant::now() + Duration::from_secs(240);
    let (mut hops, mut phase) = (0u32, 0f64);
    loop {
        {
            let mut z = inst.lock().unwrap();
            z.process(1024, &t);
            if z.readout().unwrap().state != TuneState::Tuning {
                break;
            }
            let v = z.cv(0, 1024).unwrap().1[1023] as f64;
            // An oscillator a little off 1 V/octave, as real ones are.
            let hz = (v.abs() <= 0.3 + 1e-4).then(|| 258.0 * 2f64.powf(v * 10.0 * 0.993));
            let mut b = [0f32; 1024];
            if let Some(hz) = hz {
                for s in &mut b {
                    *s = (0.5 * (phase * std::f64::consts::TAU).sin()) as f32;
                    phase = (phase + hz / rate).fract();
                }
            }
            z.input(&b, &b);
        }
        hops += 1;
        while inst.lock().unwrap().readout().unwrap().analysed != before.wrapping_add(hops / per) {
            std::thread::sleep(Duration::from_millis(1));
            assert!(Instant::now() < deadline, "the analysis fell silent");
        }
        assert!(Instant::now() < deadline, "the tune never finished");
    }
    app.store_plugin_states();
}

#[test]
#[ignore]
fn manual_shots() {
    use deckard_plugins::zhora::*;
    let mut app = modular_app();

    // LFO 1, a triangle, on Filter CV; ENV 2 on Fold CV.
    app.surface_act(A::Lfo("create", 1, 0.0));
    app.surface_act(A::Lfo("shape", 1, 3.0));
    app.surface_act(A::Lfo("create", 2, 0.0));
    app.surface_act(A::Lfo("env", 2, 0.0));
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_cv_routes(vec![
            CvRoute { low: -0.25, high: 0.75, ..CvRoute::new(ModSource::Lfo(1), 6) },
            CvRoute { low: 0.0, high: 0.8, ..CvRoute::new(ModSource::Lfo(2), 7) },
        ])
        .unwrap();

    // Zhora on sound 1, its signals on the first four CV channels, listening on In 1 L.
    (app.group, app.sound, app.chain_tab, app.channel) = (0, 0, ChainTab::Sound, false);
    app.add_slot(AddWhat::Plugin(native_entry(&CLASS)));
    app.slot = Some(0);
    app.slot_plugin_in(Chain::Sound(0, 0), 0).unwrap();
    let z = Chain::Sound(0, 0);
    for (id, ch) in [(PITCH_OUT, 2), (GATE_OUT, 3), (CLOCK_OUT, 4), (ENV_OUT, 5)] {
        set(&mut app, z, 0, id, on(ch));
    }
    set(&mut app, z, 0, INPUT, 2.0 / 33.0);
    for (id, v) in [(ATTACK, 0.35), (DECAY, 0.45), (SUSTAIN, 0.6), (RELEASE, 0.55)] {
        set(&mut app, z, 0, id, v);
    }
    app.store_plugin_states();
    tune(&mut app);
    // Holding A3, its oscillator heard a few cents sharp.
    set(&mut app, z, 0, HOLD, 1.0);
    let sharp = Some(220.0 * 2f64.powf(3.0 / 1200.0));
    let mut h = harness(app);
    feed(h.state_mut(), z, 0, sharp, 6, false);
    mk3(&mut h, "mk3-zhora-outs.png");
    window(&mut h, "zhora.png", Some(CONTROL));
    apply_acts(h.state_mut(), vec![Act::PluginPage(1)]);
    feed(h.state_mut(), z, 0, sharp, 6, false);
    mk3(&mut h, "mk3-zhora-env.png");

    // The LFO panel, with its CV section.
    h.state_mut().lfos = true;
    window(&mut h, "cv-lfo-panel.png", Some((10, 45, 1920, 525)));

    // The mod matrix on the MK3 (SHIFT + SOLO), with the CV routes.
    {
        use deckard_mk3::Event;
        for (name, down) in [("BUTTON_SHIFT", true), ("BUTTON_SOLO", true), ("BUTTON_SOLO", false), ("BUTTON_SHIFT", false)] {
            let app = h.state_mut();
            let v = app.surface_view();
            for a in app.surface.event(&Event::Button(name, down), &v, Instant::now()) {
                app.surface_act(a);
            }
        }
        mk3(&mut h, "mk3-cv-routes.png");
    }

    // Preferences ▸ Audio ▸ Routing ▸ Output, made taller to show the CV only channels.
    h.state_mut().lfos = false;
    h.state_mut().prefs = Some(2);
    h.state_mut().routing_outputs = true;
    h.run_steps(4);
    // Its lower right corner, where egui puts the window on this size.
    let corner = egui::pos2(1512.0, 471.0);
    drag(&mut h, corner, corner + egui::vec2(0.0, 380.0));
    window(&mut h, "cv-only.png", Some((15, 322, 390, 280)));
    h.state_mut().prefs = None;

    // The Tuner, on a guitar's low E a little flat.
    let mut app = App { settings: crate::settings::Settings::default(), ..App::default() };
    quiet(&mut app);
    app.new_project();
    (app.group, app.sound, app.chain_tab, app.channel) = (0, 0, ChainTab::Sound, false);
    app.add_slot(AddWhat::Plugin(native_entry(&deckard_plugins::tuner::CLASS)));
    app.slot = Some(0);
    app.slot_plugin_in(Chain::Sound(0, 0), 0).unwrap();
    let flat_e = Some(82.41 * 2f64.powf(-7.0 / 1200.0));
    let mut h = harness(app);
    feed(h.state_mut(), Chain::Sound(0, 0), 0, flat_e, 6, true);
    mk3(&mut h, "mk3-tuner.png");
    feed(h.state_mut(), Chain::Sound(0, 0), 0, flat_e, 6, true);
    window(&mut h, "tuner.png", Some(CONTROL));
}

/// Ratcheted notes in the pattern editor, with the Ratchet lane and then the Velocity
/// lane under them; and the MK3's Step page, its second knob page.
#[test]
#[ignore]
fn ratchet_shots() {
    use deckard_model::Note;
    use deckard_model::ratchet::{Ratchet, Shape};
    let mut app = App { settings: crate::settings::Settings::default(), ..App::default() };
    quiet(&mut app);
    app.new_project();
    let note = |position, length, velocity, ratchet| Note {
        ratchet,
        expr: None,
        sound: 0,
        position,
        length,
        pitch: 60,
        velocity,
        probability: 100,
    };
    let down = Ratchet { repeats: 4, shape: Shape::Down, floor: 20, ceiling: 127 };
    let up = Ratchet { repeats: 8, shape: Shape::Up, floor: 0, ceiling: 127 };
    let notes = vec![note(0, 960, 110, down), note(1920, 1920, 40, up), note(1440, 240, 100, Ratchet::default())];
    apply_acts(&mut app, vec![Act::EditNotes { g: 0, p: 0, label: "t", remove: Vec::new(), add: notes.clone() }]);
    app.selected = vec![notes[0]];
    let mut h = harness(app);
    // (The first frame shows the sound's saved lane.)
    h.state_mut().lane = crate::layout::Lane::Ratchet;
    window(&mut h, "ratchet-lane.png", Some((0, 520, 1950, 410)));
    h.state_mut().lane = crate::layout::Lane::Velocity;
    window(&mut h, "ratchet-velocity.png", Some((0, 520, 1950, 410)));
    // The MK3: STEP, then the second knob page.
    use deckard_mk3::Event;
    for name in ["BUTTON_STEP", "BUTTON_RIGHT"] {
        for down in [true, false] {
            let app = h.state_mut();
            let v = app.surface_view();
            for a in app.surface.event(&Event::Button(name, down), &v, Instant::now()) {
                app.surface_act(a);
            }
            h.run_steps(2);
        }
    }
    mk3(&mut h, "mk3-ratchet.png");
}

/// The Session Recorder window (manual: "Session recording"), mid-set: a running set
/// timer, the scrolling waveform and the L/R meters, in the default Maschine theme. The
/// window reads `session.view`, so no audio engine is needed — the state is set directly.
#[test]
#[ignore]
fn session_recorder_shots() {
    use crate::session_record::{SessionPhase, SessionView, bytes_per_second};
    use std::collections::VecDeque;

    let mut app = App { settings: crate::settings::Settings::default(), ..App::default() };
    quiet(&mut app);
    app.new_project();
    let mut h = harness(app);

    // A believable mid-set stereo waveform: per-bucket (left peak, right peak), the two
    // channels pulsing a little differently.
    let rate = 48_000u32;
    let mut wave: VecDeque<(f32, f32)> = VecDeque::new();
    for i in 0..180u32 {
        let t = i as f32 / 180.0;
        let env = 0.30 + 0.55 * (t * std::f32::consts::TAU * 3.0).sin().abs() * (0.6 + 0.4 * (t * 37.0).sin().abs());
        let l = (env * (0.7 + 0.3 * (t * 101.0).sin()).abs()).min(0.97);
        let r = (env * (0.7 + 0.3 * (t * 61.0).cos()).abs()).min(0.97);
        wave.push_back((l, r));
    }

    let app = h.state_mut();
    app.session_window_open = true;
    app.session.view = SessionView {
        phase: SessionPhase::Recording,
        elapsed: 2537.0, // 00:42:17
        started_label: "22:14".into(),
        meter: (0.62, 0.48),
        peak: (0.86, 0.74),
        bytes: 2537 * bytes_per_second(rate),
        free: 212_000_000_000,
        reserve: 1_000_000_000,
        rate,
        path: Some(std::path::PathBuf::from("session-night-run-2026-11-17-2214.wav")),
        wave,
        error: None,
        stopped: None,
    };
    // Cropped to the Recorder window, which egui lays out at the top-left.
    window(&mut h, "session-recorder.png", Some((6, 14, 406, 383)));
}

/// A frame sequence of the play/pause flow (arm → record → pause → resume), one PNG per
/// frame (`flow-000.png` …) in `$DECKARD_SHOTS`, for assembling into a demo GIF with
/// ffmpeg. The clock advances a second per recording frame and holds while paused; the
/// stereo waveform scrolls while recording and freezes while paused.
#[test]
#[ignore]
fn session_recorder_flow_frames() {
    use crate::session_record::{SessionPhase, SessionView, bytes_per_second};
    use std::collections::VecDeque;

    let mut app = App { settings: crate::settings::Settings::default(), ..App::default() };
    quiet(&mut app);
    app.new_project();
    let mut h = harness(app);
    let rate = 48_000u32;

    const CAP: usize = 180;
    // A continuous stereo signal: global bucket index `g` -> (left peak, right peak).
    let sig = |g: f32, amp: f32| -> (f32, f32) {
        let env = amp * (0.35 + 0.5 * (g * 0.11).sin().abs() * (0.6 + 0.4 * (g * 0.027).sin().abs()));
        let l = (env * (0.7 + 0.3 * (g * 0.9).sin()).abs()).min(0.97);
        let r = (env * (0.7 + 0.3 * (g * 0.5).cos()).abs()).min(0.97);
        (l, r)
    };

    h.state_mut().session_window_open = true;

    let mut g = 0.0f32;
    let mut elapsed = 0.0f64;
    let mut wave: VecDeque<(f32, f32)> = (0..CAP).map(|_| (0.012, 0.012)).collect();
    let mut frame = 0u32;

    // (phase, frames to hold, is the transport running, waveform amplitude).
    let beats = [
        (SessionPhase::Armed, 3u32, false, 0.0f32),
        (SessionPhase::Recording, 9, true, 0.95),
        (SessionPhase::Paused, 4, false, 0.0),
        (SessionPhase::Recording, 9, true, 0.95),
    ];
    for (phase, frames, running, amp) in beats {
        for _ in 0..frames {
            if running {
                for _ in 0..22 {
                    wave.push_back(sig(g, amp));
                    g += 1.0;
                    if wave.len() > CAP {
                        wave.pop_front();
                    }
                }
                elapsed += 1.0;
            }
            let (meter, peak) = if running {
                let p = 0.45 + 0.25 * (g * 0.3).sin().abs();
                ((p * 0.8, p * 0.66), (p.min(0.97), (p * 0.85).min(0.97)))
            } else {
                ((0.015, 0.015), (0.0, 0.0))
            };
            h.state_mut().session.view = SessionView {
                phase,
                elapsed,
                started_label: "22:14".into(),
                meter,
                peak,
                bytes: (elapsed as u64) * bytes_per_second(rate),
                free: 212_000_000_000,
                reserve: 1_000_000_000,
                rate,
                path: Some(std::path::PathBuf::from("session-night-run-2026-11-19-2214.wav")),
                wave: wave.clone(),
                error: None,
                stopped: None,
            };
            window(&mut h, &format!("flow-{frame:03}.png"), Some((6, 14, 406, 383)));
            frame += 1;
        }
    }
}

/// The whole window in the desktop theme `$DECKARD_SHOT_DESKTOP` (LCD: `$DECKARD_SHOT_LCD`):
/// the Ideas, Song, Mix and LFO views, and the Sampling page.
#[test]
#[ignore]
fn desktop_theme_shots() {
    let theme = std::env::var("DECKARD_SHOT_DESKTOP").unwrap_or("palenight".into());
    let mut app = App { settings: crate::settings::Settings::default(), ..App::default() };
    app.settings.desktop_theme = theme.clone();
    if let Ok(mode) = std::env::var("DECKARD_SHOT_LCD") {
        app.settings.lcd_mode = mode;
    }
    quiet(&mut app);
    app.new_project();
    let mut h = harness(app);
    window(&mut h, &format!("desktop-{theme}-ideas.png"), None);
    h.state_mut().song_view = true;
    window(&mut h, &format!("desktop-{theme}-song.png"), None);
    h.state_mut().song_view = false;
    h.state_mut().mixer = true;
    window(&mut h, &format!("desktop-{theme}-mix.png"), None);
    h.state_mut().mixer = false;
    h.state_mut().lfos = true;
    window(&mut h, &format!("desktop-{theme}-lfos.png"), None);
    h.state_mut().lfos = false;
    h.state_mut().sampling_open = true;
    window(&mut h, &format!("desktop-{theme}-sampling.png"), None);
}
