//! A New Sampler's layers, as the app edits them.

use crate::App;
use crate::layers::{can_layer, layers_in, window_moved};
use crate::layout::sampling::{LayerAct, MorphAct};
use deckard_model::layers::{Layer, Morph, Shape, spread};
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

/// An app on a new project whose first sound is an empty New Sampler.
pub(crate) fn app() -> App {
    let mut doc = deckard_doc::Document::new_project().unwrap();
    if doc.project().groups[0].sounds[0].slots.is_empty() {
        doc.add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    }
    doc.set_zones(0, 0, &[], "Empty").unwrap();
    doc.set_new_sampler(0, 0, Some(&Default::default())).unwrap();
    let mut app = App { analysis: crate::analysis::Jobs::new(None), ..App::default() };
    app.doc = Some(std::rc::Rc::new(std::cell::RefCell::new(doc)));
    app
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

#[test]
fn only_a_sound_without_slices_can_layer() {
    let (dir, mut app) = three();
    let p = |app: &App| app.doc.as_ref().unwrap().borrow().project().clone();
    assert!(can_layer(&p(&app), 0, 0));
    assert!(!can_layer(&p(&app), 0, 1), "not a New Sampler");
    // Three plain zones (slices): no layers, and files don't go in.
    {
        let doc = app.doc.as_ref().unwrap();
        let specs: Vec<_> =
            (0..3).map(|k| deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Zone(k), file: None, set: Vec::new() }).collect();
        let mut d = doc.borrow_mut();
        d.set_zones(0, 0, &specs, "x").unwrap();
        for k in 0..3 {
            let z = d.project().groups[0].sounds[0].sampler.as_ref().unwrap().zones[k].clone();
            let meta = deckard_model::new_sampler::ZoneMeta { layer: None, ..z.meta.unwrap() };
            d.set_zone_meta(0, 0, k, &meta, "x").unwrap();
        }
    }
    assert!(!can_layer(&p(&app), 0, 0));
    assert!(layers_in(&p(&app), 0, 0).is_empty());
    app.layer_files(0, &[tone(dir.path(), "e4.wav", 329.63)]);
    assert_eq!(zones(&app).len(), 3);
}

#[test]
fn morph_is_a_destination_only_with_layers() {
    use deckard_model::mod_matrix::ModParam;
    let has = |app: &App| crate::mod_slots::destinations(app, 0, 0, 0).iter().any(|d| d.param == ModParam::Field("morph".into()));
    assert!(!has(&app()));
    let (_dir, app) = three();
    assert!(has(&app));
}
