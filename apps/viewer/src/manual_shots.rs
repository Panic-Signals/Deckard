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
