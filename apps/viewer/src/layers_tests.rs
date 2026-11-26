//! A New Sampler's layers, as the app edits them.

use crate::App;
use crate::layers::{can_layer, layered, layers_in, take_choices, window_moved};
use crate::layout::sampling::{LayerAct, MorphAct};
use deckard_model::layers::{Layer, MAX_LAYERS, Morph, Shape, spread};
use deckard_model::new_sampler::{NewSampler, SamplerMode};
use std::path::PathBuf;

/// A tone on `hz`, two seconds, as a file `name` in `dir`.
pub(crate) fn tone(dir: &std::path::Path, name: &str, hz: f32) -> PathBuf {
    let rate = 48_000u32;
    let frames = (0..96_000)
        .map(|i| {
            let t = i as f32 / rate as f32 * std::f32::consts::TAU * hz;
            [0.3 * (t.sin() + 0.5 * (2.0 * t).sin() + 0.25 * (3.0 * t).sin()); 2]
        })
        .collect();
    let file = dir.join(name);
    deckard_engine::edit::write_wav_channels(&file, &deckard_engine::sample::Sample { rate, frames }, 2).unwrap();
    file
}

/// An app on a new project whose first sound is an empty Sampler Preview in Layers.
pub(crate) fn app() -> App {
    app_in(SamplerMode::Layers)
}

/// The same, in `mode`.
pub(crate) fn app_in(mode: SamplerMode) -> App {
    let mut doc = deckard_doc::Document::new_project().unwrap();
    if doc.project().groups[0].sounds[0].slots.is_empty() {
        doc.add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    }
    doc.set_zones(0, 0, &[], "Empty").unwrap();
    doc.set_new_sampler(0, 0, Some(&NewSampler { mode, ..Default::default() })).unwrap();
    let mut app = App { analysis: crate::analysis::Jobs::new(None), ..App::default() };
    app.doc = Some(std::rc::Rc::new(std::cell::RefCell::new(doc)));
    app
}

/// The focused sound's Sampler Preview put in Advanced (a classic Sampler converted
/// starts in Original).
pub(crate) fn advanced(app: &App) {
    let doc = app.doc.as_ref().unwrap();
    doc.borrow_mut().set_sampler_mode(app.group, app.sound, SamplerMode::Advanced, None).unwrap();
}

/// An app with layers on A2, A3 and A4 (the files kept in the folder returned).
pub(crate) fn three() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![tone(dir.path(), "a2.wav", 110.0), tone(dir.path(), "a3.wav", 220.0), tone(dir.path(), "a4.wav", 440.0)];
    let mut app = app();
    app.layer_files(0, &files);
    (dir, app)
}

pub(crate) fn zones(app: &App) -> Vec<deckard_model::Zone> {
    app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].sampler.as_ref().unwrap().zones.clone()
}

fn project(app: &App) -> deckard_model::Project {
    app.doc.as_ref().unwrap().borrow().project().clone()
}

fn morph(app: &App) -> Morph {
    app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].new_sampler.unwrap().morph
}

fn windows(app: &App) -> Vec<[f64; 4]> {
    app.layers_of().iter().map(|(_, l)| l.window).collect()
}

fn names(app: &App) -> Vec<String> {
    zones(app).iter().map(|z| crate::analysis::file_name_of(&z.sample.path)).collect()
}

fn labels(app: &App) -> Vec<String> {
    app.doc.as_ref().unwrap().borrow().undo_labels().iter().map(|s| s.to_string()).collect()
}

#[test]
fn files_become_layers_in_tune() {
    let dir = tempfile::tempdir().unwrap();
    // The third 20 cents sharp of A4.
    let sharp = 440.0 * 2f32.powf(20.0 / 1200.0);
    let files = vec![tone(dir.path(), "a2.wav", 110.0), tone(dir.path(), "a3.wav", 220.0), tone(dir.path(), "a4.wav", sharp)];
    let mut app = app();
    let before = labels(&app).len();
    app.layer_files(0, &files);
    let z = zones(&app);
    assert_eq!(z.len(), 3, "{}", app.status);
    assert_eq!(z.iter().map(|z| z.root_key).collect::<Vec<_>>(), [45, 57, 69]);
    assert!(z.iter().all(|z| z.key_range == (0, 127) && z.velocity_range == (0, 127)));
    assert!((z[2].tune + 0.20).abs() < 0.03, "tuned back down: {}", z[2].tune);
    assert!(z[0].tune.abs() < 0.03);
    assert!(z.iter().all(|z| z.meta.as_ref().is_some_and(|m| m.layer.is_some() && m.file == z.sample.path)));
    assert_eq!(windows(&app), spread(3, Shape::Round));
    assert_eq!(labels(&app).len(), before + 1);
    assert_eq!(labels(&app)[0], "Add Layer");
    assert!(app.doc.as_ref().unwrap().borrow_mut().undo());
    assert!(zones(&app).is_empty(), "one undo takes them all away");
}

#[test]
fn the_ninth_file_does_not_fit() {
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<PathBuf> = (0..10).map(|k| tone(dir.path(), &format!("{k}.wav"), 110.0 * (k + 1) as f32)).collect();
    let mut a = app();
    a.layer_files(0, &files);
    assert_eq!(zones(&a).len(), 8);
    assert_eq!(a.status, "2 didn't fit: a sound has 8 layers");
    assert_eq!(windows(&a), spread(8, Shape::Round));
    // Three on lane 7 of a sound with six: two go in.
    let mut b = app();
    b.layer_files(0, &files[..6]);
    b.layer_files(6, &files[6..9]);
    assert_eq!(names(&b)[6..], ["6.wav", "7.wav"]);
    assert_eq!(b.status, "1 didn't fit: a sound has 8 layers");
    // Nothing fits a full one.
    a.layer_files(8, &files[..1]);
    assert_eq!(zones(&a).len(), 8);
    assert_eq!(a.status, "1 didn't fit: a sound has 8 layers");
}

#[test]
fn a_one_shot_sound_becomes_adsr() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    let envelope = |app: &App| app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].sampler.as_ref().unwrap().envelope_type;
    assert_eq!(envelope(&app), 0);
    app.layer_files(0, &[tone(dir.path(), "a.wav", 220.0)]);
    assert_eq!(envelope(&app), 2);
}

#[test]
fn a_lone_zone_is_the_first_layer() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    let file = tone(dir.path(), "a.wav", 220.0);
    {
        let doc = app.doc.as_ref().unwrap();
        let spec = deckard_doc::ZoneSpec {
            from: deckard_doc::ZoneFrom::Template,
            file: Some(file),
            set: vec![("key_range/low", 60.0), ("key_range/high", 60.0)],
        };
        doc.borrow_mut().set_zones(0, 0, &[spec], "Load").unwrap();
    }
    assert!(zones(&app)[0].meta.is_none());
    assert_eq!(app.layers_of().len(), 1, "shown as a layer");
    app.layer_act(0, LayerAct::Attack(1.5));
    let z = &zones(&app)[0];
    assert_eq!(z.meta.as_ref().unwrap().layer.unwrap().attack, 1.5);
    assert_eq!(z.key_range, (0, 127));
    assert_eq!(labels(&app)[0], "Layer Envelope");
}

#[test]
fn replace_keeps_the_layers_settings() {
    let (dir, mut app) = three();
    app.layer_act(1, LayerAct::Release(2.0));
    app.layer_act(1, LayerAct::Level(0.5));
    app.layer_act(1, LayerAct::Mute);
    app.layer_files(1, &[tone(dir.path(), "e4.wav", 329.63)]);
    assert_eq!(names(&app), ["a2.wav", "e4.wav", "a4.wav"]);
    let z = &zones(&app)[1];
    let l = z.meta.as_ref().unwrap().layer.unwrap();
    assert!(l.mute && l.release == 2.0);
    assert_eq!((z.gain, z.root_key), (0.5, 64));
    assert_eq!(labels(&app)[0], "Replace Layer");
}

#[test]
fn remove_duplicate_and_move_respread() {
    let (_dir, mut app) = three();
    app.layer_act(1, LayerAct::Duplicate);
    assert_eq!(names(&app), ["a2.wav", "a3.wav", "a3.wav", "a4.wav"]);
    assert_eq!(windows(&app), spread(4, Shape::Round));
    app.layer_act(2, LayerAct::Remove);
    assert_eq!(windows(&app), spread(3, Shape::Round));
    app.layer_act(0, LayerAct::MoveTo(2));
    assert_eq!(names(&app), ["a3.wav", "a4.wav", "a2.wav"]);
    assert_eq!(windows(&app), spread(3, Shape::Round), "the windows stay with the lanes");
    assert_eq!(labels(&app)[..3], ["Move Layer", "Remove Layer", "Duplicate Layer"]);
    // There and back spreads its own way.
    app.morph_act(MorphAct::Shape(Shape::Back));
    assert_eq!(windows(&app), spread(3, Shape::Back));
    // A window moved by hand: they stay as they are, and a new layer has the last's.
    app.layer_act(1, LayerAct::Window(1, 0.3));
    assert!(!morph(&app).spread);
    let moved = windows(&app);
    assert_eq!(moved[1], [0.0, 0.3, 0.5, 1.0]);
    app.layer_act(0, LayerAct::Duplicate);
    assert_eq!(windows(&app)[..3], moved[..]);
    assert_eq!(windows(&app)[3], moved[2]);
    app.morph_act(MorphAct::Spread);
    assert!(morph(&app).spread);
    assert_eq!(windows(&app), spread(4, Shape::Back));
}

#[test]
fn a_window_keeps_its_order() {
    assert_eq!(window_moved([0.2, 0.4, 0.6, 0.8], 0, 0.9), [0.9, 0.9, 0.9, 0.9]);
    assert_eq!(window_moved([0.2, 0.4, 0.6, 0.8], 3, 0.1), [0.1, 0.1, 0.1, 0.1]);
    assert_eq!(window_moved([0.2, 0.4, 0.6, 0.8], 1, 0.5), [0.2, 0.5, 0.6, 0.8]);
    assert_eq!(window_moved([0.2, 0.4, 0.6, 0.8], 2, f64::NAN), [0.2, 0.4, 0.6, 0.8]);
}

#[test]
fn keys_set_every_layer() {
    let (dir, mut app) = three();
    app.morph_act(MorphAct::Keys(72, 36));
    assert!(zones(&app).iter().all(|z| z.key_range == (36, 72)));
    // A layer added after has them too.
    app.layer_files(3, &[tone(dir.path(), "e4.wav", 329.63)]);
    assert_eq!(zones(&app)[3].key_range, (36, 72));
}

#[test]
fn the_morphs_settings() {
    let (_dir, mut app) = three();
    app.morph_act(MorphAct::On(true));
    app.morph_act(MorphAct::Bars(3.1));
    assert_eq!((morph(&app).on, morph(&app).sync, morph(&app).bars), (true, true, 4.0));
    app.morph_act(MorphAct::Seconds(9999.0));
    assert_eq!((morph(&app).sync, morph(&app).seconds), (false, 500.0));
    app.morph_act(MorphAct::Position(1.5));
    assert_eq!(morph(&app).position, 1.0);
    app.layer_act(0, LayerAct::Solo);
    app.layer_act(2, LayerAct::Decay(99.0));
    let l = app.layers_of();
    assert!(l[0].1.solo && l[2].1.decay == 30.0);
    assert_eq!(l[1].1, Layer { window: l[1].1.window, ..Default::default() });
}

/// One plain zone of `file` on the first sound.
fn load_plain(app: &App, file: PathBuf) {
    let spec = deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Template, file: Some(file), set: Vec::new() };
    app.doc.as_ref().unwrap().borrow_mut().set_zones(0, 0, &[spec], "Load").unwrap();
}

/// What a sound offers is its mode's, whatever its zones hold.
#[test]
fn layers_are_the_modes() {
    use crate::layout::sampling::tab_shown;
    let dir = tempfile::tempdir().unwrap();
    // Advanced, with one sample: no layers, a SLICE tab, and files don't become layers.
    let mut app = app_in(SamplerMode::Advanced);
    load_plain(&app, tone(dir.path(), "a.wav", 220.0));
    let p = project(&app);
    assert!(!can_layer(&p, 0, 0) && !layered(&p, 0, 0));
    assert!(layers_in(&p, 0, 0).is_empty());
    assert!(!tab_shown(&p, 0, 0, 4, true, false) && tab_shown(&p, 0, 0, 2, true, false));
    assert!(!can_layer(&p, 0, 1), "not a Sampler Preview");
    app.layer_files(1, &[tone(dir.path(), "e4.wav", 329.63)]);
    assert_eq!(names(&app), ["a.wav"]);
    assert_eq!(app.status, "layers are for a Sampler Preview in Layers");
    // A take recorded is its sample, as a Sampler's.
    let take = tone(dir.path(), "take.wav", 330.0);
    app.take_into(0, 0, &take, 96_000);
    assert_eq!(names(&app), ["take.wav"]);
    assert!(zones(&app)[0].meta.as_ref().is_none_or(|m| m.layer.is_none()));
    // Layers, empty: LAYERS is offered; with a sample, no SLICE.
    let mut app = self::app();
    let p = project(&app);
    assert!(can_layer(&p, 0, 0) && !layered(&p, 0, 0));
    assert!(tab_shown(&p, 0, 0, 4, false, false));
    assert!(!tab_shown(&p, 0, 0, 2, true, false));
    // Its first take is its first layer.
    app.take_into(0, 0, &take, 96_000);
    assert_eq!(names(&app), ["take.wav"]);
    assert_eq!(zones(&app)[0].meta.as_ref().and_then(|m| m.layer).map(|l| l.window), Some(Layer::default().window));
    assert!(layered(&project(&app), 0, 0));
    // Stems go to a group, not onto lanes (those are Advanced's).
    assert!(!app.stems_onto_lanes(), "{:?}", zones(&app)[0].meta);
    let app = app_in(SamplerMode::Advanced);
    load_plain(&app, tone(dir.path(), "a.wav", 220.0));
    let file = zones(&app)[0].sample.path.clone();
    let meta = deckard_model::new_sampler::ZoneMeta { file, ..Default::default() };
    app.doc.as_ref().unwrap().borrow_mut().note_zone_meta(0, 0, 0, &meta).unwrap();
    assert!(app.stems_onto_lanes(), "an analysed zone in Advanced");
}

#[test]
fn morph_is_a_destination_only_with_layers() {
    use deckard_model::mod_matrix::ModParam;
    let has = |app: &App| crate::mod_slots::destinations(app, 0, 0, 0).iter().any(|d| d.param == ModParam::Field("morph".into()));
    assert!(!has(&app()));
    let (_dir, app) = three();
    assert!(has(&app));
}

/// The MK3's LAYERS pages (tab 5) hold the focused layer's and the morph's settings, and
/// turning them changes what the software's fields change.
#[test]
fn the_mk3_pages_turn_the_same_values() {
    use deckard_model::layers::Shape;
    use deckard_surface::{Action as A, SamplingCall as C};
    let (_dir, mut app) = three();
    app.sampling_open = true;
    app.sampling.tab = 4;
    app.zone = 1;
    // (The focused zone's sample, as the window's upkeep loads it each frame.)
    app.sampling.sample = Some(std::sync::Arc::new(deckard_engine::sample::Sample { rate: 44_100, frames: vec![[0.0; 2]; 4410] }));
    let view = app.sampling_surface_view();
    assert_eq!(view.tab, 4);
    let names: Vec<Vec<&str>> = view.pages[4].iter().map(|p| p.knobs.iter().map(|k| k.name.as_str()).collect()).collect();
    assert_eq!(view.pages[4].iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["LAYER", "ENVELOPE", "MORPH", "WINDOW"]);
    assert_eq!(names[0], ["PAN", "TUNE", "ROOT"]);
    assert_eq!(names[1], ["ATTACK", "DECAY", "SUSTAIN", "RELEASE"]);
    assert_eq!(names[2], ["MORPH", "POSITION", "RATE", "SHAPE", "DEPTH", "RESET"]);
    assert_eq!(names[3], ["A", "B", "C", "D", "FADE"]);
    let depth = &view.pages[4][2].knobs[4];
    assert_eq!((depth.text.as_str(), depth.min == depth.max), ("-", true), "Round: no depth");
    let s = view.sample.expect("its sample");
    assert_eq!((s.layers.len(), s.layer), (3, 1));
    assert_eq!(s.layers[1].level, "0.0 dB");
    assert!(s.morph.is_some_and(|m| m.round));

    // The focused layer's attack, a few steps up: only its.
    let attack = view.pages[4][1].knobs[0].value;
    app.surface_act(A::SamplingKnob { page: 1, knob: 0, value: attack + 10.0 });
    let l = app.layers_of();
    assert!((l[1].1.attack - 0.005 * 1.1f64.powi(10)).abs() < 0.002, "{}", l[1].1.attack);
    assert_eq!(l[0].1.attack, 0.005);
    // RATE: 64 bars is the last synced step; the next is free.
    app.surface_act(A::SamplingKnob { page: 2, knob: 2, value: 10.0 });
    assert_eq!((morph(&app).sync, morph(&app).bars), (true, 64.0));
    app.surface_act(A::SamplingKnob { page: 2, knob: 2, value: 11.0 });
    assert_eq!((morph(&app).sync, morph(&app).seconds), (false, 0.05));
    let rate = &app.sampling_surface_view().pages[4][2].knobs[2];
    assert_eq!((rate.value, rate.text.as_str()), (11.0, "0.05 s"));
    // Its window's A, and with it the windows are by hand.
    app.surface_act(A::SamplingKnob { page: 3, knob: 0, value: 0.1 });
    assert_eq!(windows(&app)[1][0], 0.1);
    assert!(!morph(&app).spread);
    // SHAPE to There and back: DEPTH turns.
    app.surface_act(A::SamplingKnob { page: 2, knob: 3, value: 1.0 });
    assert_eq!(morph(&app).shape, Shape::Back);
    app.surface_act(A::SamplingKnob { page: 2, knob: 4, value: 0.5 });
    assert_eq!(morph(&app).depth, 0.5);

    // The buttons and knobs 7 and 8.
    app.surface_act(A::SamplingCall(C::FocusLayer(2)));
    assert_eq!(app.zone, 2);
    app.surface_act(A::SamplingCall(C::LayerLevel(0.25)));
    assert_eq!(zones(&app)[2].gain, 0.25);
    app.surface_act(A::SamplingCall(C::LayerMute(0)));
    app.surface_act(A::SamplingCall(C::LayerSolo(1)));
    let l = app.layers_of();
    assert!(l[0].1.mute && l[1].1.solo);
    let s = app.sampling_surface_view().sample.unwrap();
    assert_eq!(s.layers.iter().map(|l| l.heard).collect::<Vec<_>>(), [false, true, false]);
    app.surface_act(A::SamplingCall(C::SpreadLayers));
    assert!(morph(&app).spread);
    app.surface_act(A::SamplingCall(C::RemoveLayer(2)));
    assert_eq!(app.layers_of().len(), 2);
}

/// In Layers a lone zone is a layer, for the MK3 too (no SLICE); in Advanced the sound
/// has none.
#[test]
fn a_lone_zone_is_layered_for_the_controller_in_layers() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    {
        let doc = app.doc.as_ref().unwrap();
        let spec =
            deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Template, file: Some(tone(dir.path(), "a.wav", 220.0)), set: Vec::new() };
        doc.borrow_mut().set_zones(0, 0, &[spec], "Load").unwrap();
    }
    app.sampling_open = true;
    app.sampling.sample = Some(std::sync::Arc::new(deckard_engine::sample::Sample { rate: 44_100, frames: vec![[0.0; 2]; 4410] }));
    let s = app.sampling_surface_view().sample.unwrap();
    assert_eq!((s.layers.len(), s.layered), (1, true));
    app.layer_files(1, &[tone(dir.path(), "b.wav", 330.0)]);
    let s = app.sampling_surface_view().sample.unwrap();
    assert_eq!((s.layers.len(), s.layered), (2, true));
    let mut app = app_in(SamplerMode::Advanced);
    load_plain(&app, tone(dir.path(), "a.wav", 220.0));
    app.sampling_open = true;
    app.sampling.sample = Some(std::sync::Arc::new(deckard_engine::sample::Sample { rate: 44_100, frames: vec![[0.0; 2]; 4410] }));
    let s = app.sampling_surface_view().sample.unwrap();
    assert_eq!((s.layers.len(), s.layered), (0, false));
}

/// A file dropped on a layered sound anywhere but a lane (its row, another tab) is a
/// layer more, not a plain zone that would leave the sound neither layered nor sliced.
#[test]
fn a_file_dropped_on_a_layered_sound_is_a_layer() {
    let (dir, mut app) = three();
    app.drop_samples((0, 0), &[tone(dir.path(), "more.wav", 500.0)]);
    assert_eq!(app.layers_of().len(), 4, "{}", app.status);
    let p = crate::project_of_app(&app);
    assert!(crate::layers::can_layer(&p, 0, 0) && crate::layers::layered(&p, 0, 0));
    assert!(zones(&app).iter().all(|z| z.meta.as_ref().is_some_and(|m| m.layer.is_some())));
}

/// KEYS dragged through many keys is one undo step.
#[test]
fn dragging_keys_is_one_undo_step() {
    let (_dir, mut app) = three();
    let before = labels(&app).len();
    for lo in 1..=12 {
        app.morph_act(MorphAct::Keys(lo, 127));
    }
    assert_eq!(zones(&app)[0].key_range, (12, 127));
    assert_eq!(labels(&app).len(), before + 1, "{:?}", labels(&app));
    assert!(app.doc.as_ref().unwrap().borrow_mut().undo());
    assert!(zones(&app).iter().all(|z| z.key_range == (0, 127)));
}

/// The first take is the first layer; the next ones wait in the list, on no layer.
#[test]
fn later_takes_go_on_no_layer() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    let take = |app: &mut App, name: &str, hz: f32| {
        let f = tone(dir.path(), name, hz);
        app.take_into(0, 0, &f, 48_000);
    };
    take(&mut app, "one.wav", 220.0);
    assert_eq!(names(&app), ["one.wav"]);
    assert!(zones(&app)[0].meta.as_ref().is_some_and(|m| m.layer.is_some()), "a layer from the start");
    take(&mut app, "two.wav", 330.0);
    assert_eq!(names(&app), ["one.wav"], "the sound plays what it did");
}

/// Three layers, and a take focused that is on none of them.
fn three_and_a_take() -> (tempfile::TempDir, App, PathBuf) {
    let (dir, mut app) = three();
    let f = tone(dir.path(), "take.wav", 500.0);
    app.recorder.view.takes.push(f.clone());
    app.recorder.view.focused = Some(0);
    (dir, app, f)
}

fn record(app: &mut App, a: crate::layout::sampling::RecordAct) {
    app.sampling_act(crate::layout::sampling::SamplingAct::Record(a));
}

/// LAYER is the focused take's: None, the sound's layers, New. Turning it changes
/// nothing in the sound.
#[test]
fn the_layer_choices_are_the_focused_takes() {
    use crate::layers::{NO_LAYER, layer_pending};
    use crate::layout::sampling::{RecParam, RecordAct};
    let (_dir, mut app) = three();
    let choices = |app: &App| take_choices(&project(app), 0, 0, &app.recorder.view).map(|c| (c.0, c.1));
    let pending = |app: &App| layer_pending(&project(app), 0, 0, &app.recorder.view);
    let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(choices(&app), None, "no take focused");
    let f = tone(_dir.path(), "take.wav", 500.0);
    app.recorder.view.takes.push(f);
    app.recorder.view.focused = Some(0);
    assert_eq!(choices(&app), Some((s(&["None", "1", "2", "3", "New"]), 0)));
    assert_eq!(pending(&app), None);
    let before = (names(&app), labels(&app).len());
    record(&mut app, RecordAct::Set(RecParam::Layer, 1));
    assert_eq!(choices(&app).unwrap().1, 2);
    assert_eq!(pending(&app), Some(1));
    assert_eq!((names(&app), labels(&app).len()), before, "only chosen");
    record(&mut app, RecordAct::Set(RecParam::Layer, NO_LAYER));
    assert_eq!(pending(&app), None, "it is on none");
    // The MK3's RECORDING page has it too.
    let knobs = crate::record_knobs::knobs(&app.recorder.view, &app.settings, &project(&app), (0, 0), true, None);
    let k = knobs.pages[0].knobs.iter().find(|k| k.name == "LAYER").expect("a LAYER knob");
    assert_eq!((k.text.as_str(), k.max), ("None", 4.0));
    // Loop has a target of its own.
    app.recorder.view.set(RecParam::Mode, 2);
    assert_eq!(choices(&app), None);
}

/// APPLY puts the take on the layer chosen (New: one more); None takes its layer away.
/// One undo step each.
#[test]
fn apply_puts_the_take_on_the_layer_chosen() {
    use crate::layers::{NO_LAYER, take_layer};
    use crate::layout::sampling::{RecParam, RecordAct};
    let (_dir, mut app, f) = three_and_a_take();
    let on = |app: &App| take_layer(&project(app), 0, 0, &f);
    app.layer_act(1, LayerAct::Level(0.3));
    let before = labels(&app).len();
    record(&mut app, RecordAct::ApplyLayer);
    assert_eq!(labels(&app).len(), before, "nothing chosen: nothing done");
    record(&mut app, RecordAct::Set(RecParam::Layer, 1));
    record(&mut app, RecordAct::ApplyLayer);
    assert_eq!(names(&app), ["a2.wav", "take.wav", "a4.wav"]);
    assert!((zones(&app)[1].gain - 0.3).abs() < 1e-9, "the layer's settings stay");
    assert_eq!((on(&app), app.recorder.view.layer_choice), (Some(1), None));
    assert_eq!(labels(&app).len(), before + 1);
    // New.
    record(&mut app, RecordAct::Set(RecParam::Layer, MAX_LAYERS as i64));
    record(&mut app, RecordAct::ApplyLayer);
    assert_eq!(names(&app), ["a2.wav", "take.wav", "a4.wav", "take.wav"]);
    // On two layers: it shows the first, and None takes only that one away.
    assert_eq!(on(&app), Some(1));
    let before = labels(&app).len();
    record(&mut app, RecordAct::Set(RecParam::Layer, NO_LAYER));
    record(&mut app, RecordAct::ApplyLayer);
    assert_eq!(names(&app), ["a2.wav", "a4.wav", "take.wav"]);
    assert_eq!(labels(&app).len(), before + 1);
    app.undo();
    assert_eq!(names(&app), ["a2.wav", "take.wav", "a4.wav", "take.wav"]);
}

/// A choice not applied goes when another take is focused or the focused one deleted,
/// when a take is recorded, and counts for nothing where it can't be had.
#[test]
fn a_pending_choice_is_dropped() {
    use crate::layers::layer_pending;
    use crate::layout::sampling::{RecParam, RecordAct};
    let (dir, mut app, f) = three_and_a_take();
    let pending = |app: &App| layer_pending(&project(app), 0, 0, &app.recorder.view);
    let choose = |app: &mut App, v: i64| record(app, RecordAct::Set(RecParam::Layer, v));
    app.recorder.view.takes.push(tone(dir.path(), "other.wav", 600.0));
    choose(&mut app, 1);
    record(&mut app, RecordAct::FocusTake(1));
    assert_eq!(app.recorder.view.layer_choice, None, "another take focused");
    choose(&mut app, 1);
    record(&mut app, RecordAct::DeleteTake(1));
    assert_eq!(app.recorder.view.layer_choice, None, "the take deleted");
    // A take recorded: it is focused, on no layer.
    app.recorder.view.focused = Some(0);
    choose(&mut app, 1);
    app.path = Some(dir.path().join("p.mxprj"));
    let sample = deckard_engine::sample::Sample { rate: 48_000, frames: vec![[0.1, 0.1]; 4800] };
    app.finish_recording(crate::recorder::Finished::Take { sample, channels: 2, mode: 0, ticks: 960.0, focus: (0, 0) });
    assert_eq!((app.recorder.view.focused, app.recorder.view.layer_choice), (Some(1), None));
    assert_eq!(names(&app), ["a2.wav", "a3.wav", "a4.wav"]);
    // The take's file gone: APPLY says so and changes nothing.
    app.recorder.view.focused = Some(0);
    std::fs::remove_file(&f).unwrap();
    choose(&mut app, 1);
    record(&mut app, RecordAct::ApplyLayer);
    assert_eq!(names(&app), ["a2.wav", "a3.wav", "a4.wav"]);
    assert_eq!(app.recorder.view.layer_choice, None);
    assert!(app.status.contains("take.wav"), "{}", app.status);
    // Every layer taken: no New, and one chosen before counts for nothing.
    let more: Vec<PathBuf> = (3..MAX_LAYERS).map(|k| tone(dir.path(), &format!("l{k}.wav"), 100.0 + k as f32)).collect();
    app.layer_files(3, &more);
    assert_eq!(zones(&app).len(), MAX_LAYERS);
    app.recorder.view.choose_layer((0, 0), MAX_LAYERS as i64);
    assert_eq!(pending(&app), None);
    assert!(!take_choices(&project(&app), 0, 0, &app.recorder.view).unwrap().0.contains(&"New".to_string()));
    // A sound that can't have layers: no LAYER, and APPLY does nothing.
    choose(&mut app, 1);
    app.sound = 1;
    assert_eq!(take_choices(&project(&app), 0, 1, &app.recorder.view), None);
    let before = labels(&app).len();
    record(&mut app, RecordAct::ApplyLayer);
    assert_eq!(labels(&app).len(), before);
}

/// A take clicked is only focused on a Sampler Preview that has a sample: the first
/// layer stays the take it was given (the take's menu puts another there). An empty one
/// gets the take clicked.
#[test]
fn a_take_clicked_leaves_the_layers_alone() {
    use crate::layout::sampling::{RecordAct, SamplingAct};
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    for (name, hz) in [("one.wav", 220.0), ("two.wav", 330.0)] {
        app.recorder.view.takes.push(tone(dir.path(), name, hz));
    }
    let click = |app: &mut App, i: usize| app.sampling_act(SamplingAct::Record(RecordAct::FocusTake(i)));
    click(&mut app, 0);
    assert_eq!(names(&app), ["one.wav"], "an empty sound gets the take");
    click(&mut app, 1);
    assert_eq!(names(&app), ["one.wav"], "the sample stays");
    assert_eq!(app.recorder.view.focused, Some(1), "the take is focused all the same");
    // With layers, the same.
    app.sampling_act(SamplingAct::Record(RecordAct::TakeToLayer(1, 1)));
    click(&mut app, 1);
    click(&mut app, 0);
    assert_eq!(names(&app), ["one.wav", "two.wav"]);
    // Its menu still replaces the first.
    app.sampling_act(SamplingAct::Record(RecordAct::TakeToLayer(1, 0)));
    assert_eq!(names(&app), ["two.wav", "two.wav"]);
}

/// A layer chosen on one sound isn't a choice on another, nor after the mode was changed
/// and changed back: APPLY there does nothing.
#[test]
fn a_choice_stays_with_its_sound() {
    use crate::layers::layer_pending;
    use crate::layout::sampling::{RecParam, RecordAct};
    let (dir, mut app, _take) = three_and_a_take();
    // A second sound with layers of its own.
    app.sound = 1;
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        d.add_slot(0, 1, 0, &deckard_doc::NewModule::Sampler).unwrap();
        d.set_zones(0, 1, &[], "Empty").unwrap();
        d.set_new_sampler(0, 1, Some(&Default::default())).unwrap();
    }
    app.layer_files(0, &[tone(dir.path(), "b1.wav", 150.0), tone(dir.path(), "b2.wav", 250.0)]);
    app.sound = 0;
    record(&mut app, RecordAct::Set(RecParam::Layer, 1));
    assert_eq!(layer_pending(&project(&app), 0, 0, &app.recorder.view), Some(1));
    app.sound = 1;
    assert_eq!(layer_pending(&project(&app), 0, 1, &app.recorder.view), None, "chosen on the other sound");
    let before = labels(&app).len();
    record(&mut app, RecordAct::ApplyLayer);
    assert_eq!(labels(&app).len(), before);
    // The mode changed and back: the choice is gone.
    app.sound = 0;
    record(&mut app, RecordAct::Set(RecParam::Layer, 1));
    record(&mut app, RecordAct::Set(RecParam::Mode, 2));
    record(&mut app, RecordAct::Set(RecParam::Mode, 0));
    assert_eq!(layer_pending(&project(&app), 0, 0, &app.recorder.view), None);
}

/// A take dragged onto a sound with layers is one more layer (its sample and layers stay).
#[test]
fn a_take_dropped_on_a_layered_sound_is_a_new_layer() {
    let (_dir, mut app, take) = three_and_a_take();
    app.place_take(0, 0, &take);
    assert_eq!(names(&app), ["a2.wav", "a3.wav", "a4.wav", "take.wav"], "{}", app.status);
}

/// Bring lane `lane`'s sample up in the Sampling view, as a frame would.
fn show(app: &mut App, lane: usize) {
    app.zone = lane;
    let sref = zones(app)[lane].sample.clone();
    let s = app.samples.sample(&sref, None).expect("the layer's sample");
    app.sampling.set_sample((0, 0, lane, sref.path.clone()), s);
}

/// EDIT on a layer's sample (a new file) leaves it the layer it was.
#[test]
fn an_edited_layer_stays_a_layer() {
    use crate::layout::sampling::{EditOp, SamplingAct};
    let (dir, mut app) = three();
    app.path = Some(dir.path().join("x.mxprj"));
    app.layer_act(1, LayerAct::Mute);
    let before = app.layers_of();
    for (op, selection) in [(EditOp::Normalize, None), (EditOp::Truncate, Some((0, 48_000))), (EditOp::Paste, Some((0, 100)))] {
        show(&mut app, 1);
        app.sampling.clipboard = app.sampling.sample.as_ref().map(|s| deckard_engine::edit::copy(s, (0, 1000)));
        app.sampling.selection = selection;
        let file = names(&app)[1].clone();
        app.sampling_act(SamplingAct::Edit(op));
        assert_ne!(names(&app)[1], file, "{op:?}: {}", app.status);
        assert!(can_layer(&project(&app), 0, 0), "{op:?}");
        assert_eq!(app.layers_of(), before, "{op:?}: its mute and window too");
    }
}

/// A take edited on its layer is still on that layer for RECORD's LAYER.
#[test]
fn an_edited_take_is_still_on_its_layer() {
    use crate::layers::{layer_pending, take_layer};
    use crate::layout::sampling::{EditOp, RecordAct, SamplingAct};
    let (dir, mut app, take) = three_and_a_take();
    app.path = Some(dir.path().join("x.mxprj"));
    app.recorder.view.choose_layer((0, 0), 1);
    record(&mut app, RecordAct::ApplyLayer);
    assert_eq!(take_layer(&project(&app), 0, 0, &take), Some(1));
    show(&mut app, 1);
    app.sampling.selection = None;
    app.sampling_act(SamplingAct::Edit(EditOp::Normalize));
    assert_eq!(names(&app)[1], "take - 2.wav", "{}", app.status);
    assert_eq!(take_layer(&project(&app), 0, 0, &take), Some(1));
    assert_eq!(layer_pending(&project(&app), 0, 0, &app.recorder.view), None);
}

fn mode(app: &App) -> SamplerMode {
    crate::layers::mode_of(&project(app), 0, 0).unwrap()
}

fn sampler(app: &mut App, m: SamplerMode) {
    app.sampling_act(crate::layout::sampling::SamplingAct::Sampler(m));
}

fn answer(app: &mut App, ok: bool) {
    app.sampling_act(crate::layout::sampling::SamplingAct::SamplerAnswer(ok));
}

/// TYPE on a sound with samples asks first; Cancel leaves it, OK keeps the sample
/// shown as the sound's only one, in one undo step.
#[test]
fn a_mode_change_asks_first() {
    let (_dir, mut app) = three();
    sampler(&mut app, SamplerMode::Advanced);
    assert_eq!(app.sampler_ask, Some(((0, 0), SamplerMode::Advanced)));
    assert_eq!((mode(&app), zones(&app).len()), (SamplerMode::Layers, 3), "asked, not done");
    answer(&mut app, false);
    assert_eq!((app.sampler_ask, mode(&app), zones(&app).len()), (None, SamplerMode::Layers, 3), "Cancel");
    answer(&mut app, true);
    assert_eq!(mode(&app), SamplerMode::Layers, "nothing asked: nothing answered");
    show(&mut app, 1);
    sampler(&mut app, SamplerMode::Advanced);
    let steps = labels(&app).len();
    answer(&mut app, true);
    assert_eq!((app.sampler_ask, mode(&app)), (None, SamplerMode::Advanced));
    assert_eq!(names(&app), ["a3.wav"], "the sample shown stays");
    let z = &zones(&app)[0];
    assert_eq!((z.meta.clone(), z.key_range, z.play_range), (None, (0, 127), (0, 95_999)));
    assert_eq!(app.zone, 0);
    assert_eq!(labels(&app).len(), steps + 1, "one step");
    assert!(app.doc.as_ref().unwrap().borrow_mut().undo());
    assert_eq!((mode(&app), zones(&app).len()), (SamplerMode::Layers, 3));
    // To the mode it has: nothing, and what was asked is dropped.
    sampler(&mut app, SamplerMode::Original);
    sampler(&mut app, SamplerMode::Layers);
    assert_eq!(app.sampler_ask, None);
}

/// Nothing to lose: no question. (A sound without a sample; Original made Advanced.)
#[test]
fn an_empty_sound_changes_mode_at_once() {
    let mut app = app();
    sampler(&mut app, SamplerMode::Original);
    assert_eq!((app.sampler_ask, mode(&app)), (None, SamplerMode::Original));
    let dir = tempfile::tempdir().unwrap();
    let f = tone(dir.path(), "one.wav", 220.0);
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_zones(0, 0, &[crate::new_zone(&f, 96_000, deckard_doc::ZoneFrom::Template)], "Load")
        .unwrap();
    sampler(&mut app, SamplerMode::Advanced);
    assert_eq!((app.sampler_ask, mode(&app), names(&app)), (None, SamplerMode::Advanced, vec!["one.wav".to_string()]));
    // Back to Original would lose what Advanced has: asked.
    sampler(&mut app, SamplerMode::Original);
    assert_eq!((app.sampler_ask.map(|a| a.1), mode(&app)), (Some(SamplerMode::Original), SamplerMode::Advanced));
}

/// What was asked is dropped when another sound is focused, and nothing is asked while
/// recording.
#[test]
fn a_pending_mode_is_dropped() {
    let (_dir, mut app) = three();
    sampler(&mut app, SamplerMode::Original);
    assert!(app.sampler_ask.is_some());
    app.sound = 1;
    app.update_sampling();
    assert_eq!(app.sampler_ask, None);
    app.sound = 0;
    answer(&mut app, true);
    assert_eq!((mode(&app), zones(&app).len()), (SamplerMode::Layers, 3), "not the answer to anything");
    app.recorder.fake_recording((0, 0));
    sampler(&mut app, SamplerMode::Original);
    assert_eq!(app.sampler_ask, None);
    assert!(app.status.contains("recording"), "{}", app.status);
}

/// In Layers a take picked that is on no layer is what EDIT works on: the edit is a new
/// file in the take list, the sound and its undo history as they were. A mode change
/// then keeps that take.
#[test]
fn a_take_is_edited_before_it_is_on_a_layer() {
    use crate::layout::sampling::{EditOp, RecordAct, SamplingAct};
    let (dir, mut app, f) = three_and_a_take();
    app.path = Some(dir.path().join("x.mxprj"));
    app.sampling_open = true;
    assert_eq!(app.shown_take(), None, "not picked yet");
    record(&mut app, RecordAct::FocusTake(0));
    assert_eq!(app.shown_take(), Some((0, f.clone())));
    app.update_sampling();
    assert_eq!(app.sampling.sample_key.as_ref().map(|k| (k.2, k.3.clone())), Some((usize::MAX, f.to_string_lossy().into_owned())));
    let before = (names(&app), labels(&app).len());
    app.sampling_act(SamplingAct::Edit(EditOp::Reverse));
    let edited = app.recorder.view.takes[0].clone();
    assert_eq!(edited, dir.path().join("Recordings").join("take - 2.wav"), "{}", app.status);
    assert!(edited.exists() && f.exists(), "the original stays");
    assert!(app.recorder.view.take_waves.contains_key(&edited));
    assert_eq!((names(&app), labels(&app).len()), before, "the sound isn't touched");
    assert_eq!(app.shown_take(), Some((0, edited.clone())), "still the take shown");
    // What is a zone's isn't a take's.
    app.sampling_act(SamplingAct::Edit(EditOp::AutoLoop));
    assert!(app.status.contains("layer first"), "{}", app.status);
    // A layer picked: EDIT is the layer's again.
    app.sampling_act(SamplingAct::Layer(1, LayerAct::Focus));
    assert_eq!(app.shown_take(), None);
    // The mode changed with the take shown keeps the take.
    record(&mut app, RecordAct::FocusTake(0));
    sampler(&mut app, SamplerMode::Advanced);
    assert!(app.sampler_ask.is_some());
    answer(&mut app, true);
    assert_eq!((mode(&app), names(&app)), (SamplerMode::Advanced, vec!["take - 2.wav".to_string()]), "{}", app.status);
    assert_eq!(app.shown_take(), None);
    assert!(app.doc.as_ref().unwrap().borrow_mut().undo());
    assert_eq!((mode(&app), zones(&app).len()), (SamplerMode::Layers, 3), "one step");
}

/// A take picked that is on a layer is edited as that layer, and the list follows.
#[test]
fn an_edit_on_a_layer_renames_its_take_in_the_list() {
    use crate::layout::sampling::{EditOp, RecordAct, SamplingAct};
    let (dir, mut app) = three();
    app.path = Some(dir.path().join("x.mxprj"));
    app.recorder.view.takes.push(dir.path().join("a3.wav"));
    record(&mut app, RecordAct::FocusTake(0));
    assert_eq!((app.zone, app.shown_take()), (1, None), "its layer is shown");
    show(&mut app, 1);
    app.sampling_act(SamplingAct::Edit(EditOp::Reverse));
    let file = PathBuf::from(&zones(&app)[1].sample.path);
    assert_eq!(file.file_name().unwrap(), "a3 - 2.wav", "{}", app.status);
    assert_eq!(app.recorder.view.takes[0].file_name(), file.file_name(), "the list has the layer's file");
    assert_eq!(crate::layers::take_layer(&project(&app), 0, 0, &app.recorder.view.takes[0]), Some(1));
}

/// (Review) From the controller a take on no layer can be selected in, so TRUNCATE has
/// something to cut: EDIT's SELECTION RANGE is the take's, and there is no PLAY RANGE.
#[test]
fn a_take_on_no_layer_has_its_selection_knobs() {
    use crate::layout::sampling::RecordAct;
    let (_dir, mut app, _f) = three_and_a_take();
    app.sampling_open = true;
    record(&mut app, RecordAct::FocusTake(0));
    app.update_sampling();
    let v = app.sampling_surface_view();
    let names: Vec<&str> = v.pages[1].iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["SELECTION RANGE"]);
    assert_eq!(v.pages[1][0].knobs[1].max, 95_999.0);
    assert!(v.pages[2].is_empty() && v.pages[3].is_empty());
    app.record_apply.insert("Sel.End".into(), crate::sampling_hw::Apply::Selection(true));
    app.sampling_knob("Sel.End", 48_000.0);
    assert_eq!(app.sampling.selection, Some((0, 48_000)));
}

/// (Review) LAYERS is still a tab while a take on no layer is shown, and going there
/// leaves the take.
#[test]
fn layers_is_a_tab_away_from_a_take() {
    use crate::layout::sampling::{RecordAct, SamplingAct, tab_shown};
    let (_dir, mut app, _f) = three_and_a_take();
    app.sampling_open = true;
    record(&mut app, RecordAct::FocusTake(0));
    app.update_sampling();
    assert!(tab_shown(&project(&app), 0, 0, 4, true, true));
    app.sampling_act(SamplingAct::Tab(4));
    app.update_sampling();
    assert_eq!((app.shown_take(), app.sampling.tab), (None, 4));
}

/// (Review) A layer has no play mode of its own: Sync, Keys, speed and reverse are
/// Advanced's.
#[test]
fn layers_has_no_play_modes() {
    use crate::layout::sampling::SamplingAct;
    use deckard_model::new_sampler::PlayMode;
    let (_dir, mut app) = three();
    show(&mut app, 0);
    let before = zones(&app)[0].meta.clone().unwrap();
    app.sampling_act(SamplingAct::PlayMode(PlayMode::Sync));
    app.sampling_act(SamplingAct::Meta(deckard_model::new_sampler::ZoneMeta { reverse: true, ..before.clone() }));
    assert_eq!(zones(&app)[0].meta.clone().unwrap(), before);
    let pages = app.sampling_surface_view_open().pages;
    assert!(
        !pages[1].iter().any(|p| p.name.contains("TEMPO") || p.name.contains("PLAYBACK") || p.name.contains("MOTION")),
        "{:?}",
        pages[1].iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    assert!(!crate::layers::advanced(&project(&app), 0, 0));
}

/// (Review) A sample the browser loads onto a sound in Layers is its layer.
#[test]
fn a_sample_from_the_browser_is_a_layer() {
    let (dir, mut app) = three();
    let f = tone(dir.path(), "new.wav", 330.0);
    let item = deckard_library::Item { kind: deckard_library::Kind::OneShot, name: "new".into(), path: f, ..Default::default() };
    app.load_item(&item, Some((0, 0))).unwrap();
    assert_eq!(names(&app), ["new.wav"]);
    let z = &zones(&app)[0];
    assert!(z.meta.as_ref().is_some_and(|m| m.layer.is_some()), "a layer: {:?}", z.meta);
    assert_eq!(z.key_range, (0, 127));
}

/// (Review) The question goes when the Sampling page is left.
#[test]
fn leaving_the_page_drops_the_question() {
    let (_dir, mut app) = three();
    app.sampling_open = true;
    sampler(&mut app, SamplerMode::Advanced);
    assert!(app.sampler_ask.is_some());
    app.surface_sampling(deckard_surface::Action::Sampling(false));
    assert_eq!(app.sampler_ask, None);
    sampler(&mut app, SamplerMode::Advanced);
    app.sampling_act(crate::layout::sampling::SamplingAct::Open(false));
    assert_eq!(app.sampler_ask, None);
}

/// (Review) The TYPE knob changes nothing until confirmed, Original made Advanced
/// too (a bumped knob mustn't change the sound); OK there keeps everything.
#[test]
fn the_knob_always_asks() {
    let mut app = app_in(SamplerMode::Original);
    let dir = tempfile::tempdir().unwrap();
    let specs: Vec<_> = ["one.wav", "two.wav"]
        .iter()
        .map(|n| crate::new_zone(&tone(dir.path(), n, 220.0), 96_000, deckard_doc::ZoneFrom::Template))
        .collect();
    app.doc.as_ref().unwrap().borrow_mut().set_zones(0, 0, &specs, "Load").unwrap();
    app.record_apply.insert("Type".into(), crate::sampling_hw::Apply::Record(crate::record_knobs::Apply::Sampler));
    app.sampling_knob("Type", 1.0);
    assert_eq!((app.sampler_ask, mode(&app)), (Some(((0, 0), SamplerMode::Advanced)), SamplerMode::Original));
    answer(&mut app, true);
    assert_eq!((mode(&app), zones(&app).len()), (SamplerMode::Advanced, 2), "nothing lost");
}

/// A take just recorded on a sound with layers is on no layer, and is what EDIT works
/// on: NORMALIZE makes the take louder (in the take list too) and leaves the layers,
/// and the take put on a layer is the louder one.
#[test]
fn a_take_just_recorded_is_what_edit_works_on() {
    use crate::layout::sampling::{EditOp, RecParam, RecordAct, SamplingAct};
    let (dir, mut app) = three();
    app.path = Some(dir.path().join("p.mxprj"));
    app.sampling_open = true;
    let frames = (0..4800).map(|i| [0.1 * (i as f32 * 0.05).sin(); 2]).collect();
    let sample = deckard_engine::sample::Sample { rate: 48_000, frames };
    app.finish_recording(crate::recorder::Finished::Take { sample, channels: 2, mode: 0, ticks: 960.0, focus: (0, 0) });
    let take = app.recorder.view.takes[0].clone();
    assert_eq!(app.shown_take(), Some((0, take.clone())), "the new take is shown");
    app.update_sampling();
    let peak = |app: &App| {
        let v = &app.recorder.view;
        v.take_waves[&v.takes[0]].iter().fold(0f32, |m, w| m.max(w.1))
    };
    assert!(peak(&app) < 0.11);
    let before = names(&app);
    app.sampling_act(SamplingAct::Edit(EditOp::Normalize));
    assert_ne!(app.recorder.view.takes[0], take, "{}", app.status);
    assert!(peak(&app) > 0.9, "the take's wave is the louder one: {}", peak(&app));
    assert_eq!(names(&app), before, "the layers aren't touched");
    // On a new layer: the louder file, and its picture.
    record(&mut app, RecordAct::Set(RecParam::Layer, MAX_LAYERS as i64));
    record(&mut app, RecordAct::ApplyLayer);
    let edited = app.recorder.view.takes[0].file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(names(&app).last(), Some(&edited), "{}", app.status);
    app.update_sampling();
    let v = app.sampling_surface_view();
    let l = v.sample.as_ref().unwrap().layers.last().unwrap().clone();
    assert!(l.detail.iter().fold(0f32, |m, w| m.max(w.1)) > 0.9, "the layer's picture is the louder one");
}

/// EDIT's third knob page is LOOP in every mode (the zone's or layer's loop), after
/// PLAY RANGE and SELECTION RANGE; turning ACTIVE loops it.
#[test]
fn edit_page_three_is_loop_in_every_mode() {
    for mode in SamplerMode::ALL {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_in(mode);
        app.sampling_open = true;
        let f = tone(dir.path(), "a.wav", 220.0);
        if mode == SamplerMode::Layers {
            app.layer_files(0, &[f]);
        } else {
            let doc = app.doc.clone().unwrap();
            let spec = deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Template, file: Some(f), set: Vec::new() };
            doc.borrow_mut().set_zones(0, 0, &[spec], "Load").unwrap();
        }
        app.update_sampling();
        let v = app.sampling_surface_view();
        let pages: Vec<&str> = v.pages[1].iter().map(|p| p.name.as_str()).collect();
        assert_eq!(pages[..3], ["PLAY RANGE", "SELECTION RANGE", "LOOP"], "{mode:?}");
        let knobs: Vec<(&str, &str)> = v.pages[1][2].knobs.iter().map(|k| (k.key.as_str(), k.name.as_str())).collect();
        assert_eq!(
            knobs,
            [("Edit.Loop", "ACTIVE"), ("Edit.LoopStart", "START"), ("Edit.LoopEnd", "END"), ("Edit.XFade", "XFADE")],
            "{mode:?}"
        );
        let p = app.sampling_pages().unwrap();
        app.record_apply = p.apply;
        app.sampling_knob("Edit.Loop", 1.0);
        assert!(zones(&app)[0].loop_active, "{mode:?}: {}", app.status);
        // The controller's view says where a layer loops (of its sample), while it does.
        if mode == SamplerMode::Layers {
            let doc = app.doc.clone().unwrap();
            let zt = deckard_doc::Target::Zone(0, 0, 0);
            for (field, v) in [("loop_range/low", 24_000), ("loop_range/high", 72_000)] {
                doc.borrow_mut().set("Loop", zt.clone(), field, mxprj::decode::Value::Int(v)).unwrap();
            }
            app.update_sampling();
            let l = app.sampling_surface_view().sample.unwrap().layers[0].clone();
            let (a, b) = l.loop_points.expect("loops");
            assert!((a - 0.25).abs() < 0.001 && (b - 0.75).abs() < 0.001, "{a} {b}");
            doc.borrow_mut().set("Loop", zt, "loop_active", mxprj::decode::Value::Bool(false)).unwrap();
            assert_eq!(app.sampling_surface_view().sample.unwrap().layers[0].loop_points, None);
        }
    }
}
