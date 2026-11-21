//! The LAYERS tab, driven with the mouse.

use super::*;
use crate::layers_tests::{tone, zones};
use crate::layout::sampling::{layers::LANE_RECTS, tab_shown};
use deckard_model::layers::{Shape, spread};

/// The window on a new project whose first sound is an empty New Sampler, its Sampling
/// view on LAYERS.
fn layers_harness() -> Harness<'static, App> {
    let mut h = blank_harness(false);
    {
        let app = h.state_mut();
        (app.group, app.sound) = (0, 0);
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        if d.project().groups[0].sounds[0].slots.is_empty() {
            d.add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
        }
        d.set_zones(0, 0, &[], "Empty").unwrap();
        d.set_new_sampler(0, 0, Some(&Default::default())).unwrap();
    }
    h.state_mut().sampling_open = true;
    h.state_mut().sampling.tab = 4;
    h.run_steps(4);
    h
}

fn lanes(h: &Harness<'static, App>) -> Vec<egui::Rect> {
    h.ctx.data(|d| d.get_temp(egui::Id::new(LANE_RECTS))).expect("the lanes are shown")
}

fn drop_on(h: &mut Harness<'static, App>, at: Pos2, files: &[PathBuf]) {
    h.hover_at(at);
    h.input_mut().hovered_files = files.iter().map(|f| egui::HoveredFile { path: Some(f.clone()), ..Default::default() }).collect();
    h.step();
    h.input_mut().hovered_files.clear();
    h.input_mut().dropped_files = files.iter().map(|f| std::sync::Arc::new(Dropped(f.clone())) as egui::DroppedFileHandle).collect();
    h.step();
    h.input_mut().dropped_files.clear();
    h.run_steps(3);
}

fn names(h: &Harness<'static, App>) -> Vec<String> {
    zones(h.state()).iter().map(|z| crate::analysis::file_name_of(&z.sample.path)).collect()
}

/// RECORD and LAYERS for an empty New Sampler; all five with one zone; no SLICE once it
/// has a layer; no LAYERS for a sliced sound, a take, or a plain Sampler.
#[test]
fn the_tab_is_offered_where_it_can_be() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = crate::layers_tests::app();
    let shown = |app: &App, zone: bool| -> Vec<bool> {
        let p = project_of_app(app);
        (0..5).map(|i| tab_shown(&p, 0, 0, i, zone, false)).collect()
    };
    assert_eq!(shown(&app, false), [true, false, false, false, true]);
    {
        let doc = app.doc.as_ref().unwrap();
        let spec =
            deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Template, file: Some(tone(dir.path(), "a.wav", 220.0)), set: Vec::new() };
        doc.borrow_mut().set_zones(0, 0, &[spec], "Load").unwrap();
    }
    assert_eq!(shown(&app, true), [true; 5]);
    app.layer_files(1, &[tone(dir.path(), "b.wav", 330.0)]);
    assert_eq!(shown(&app, true), [true, true, false, true, true], "layered: no SLICE");
    // Sliced: three plain zones.
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        let specs: Vec<_> = [0, 0, 0]
            .iter()
            .map(|&k| deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Zone(k), file: None, set: Vec::new() })
            .collect();
        d.set_zones(0, 0, &specs, "x").unwrap();
        for k in 0..3 {
            let z = d.project().groups[0].sounds[0].sampler.as_ref().unwrap().zones[k].clone();
            d.set_zone_meta(0, 0, k, &deckard_model::new_sampler::ZoneMeta { layer: None, ..z.meta.unwrap() }, "x").unwrap();
        }
    }
    assert_eq!(shown(&app, true), [true, true, true, true, false], "sliced: no LAYERS");
    let p = project_of_app(&app);
    assert!(!tab_shown(&p, 0, 1, 4, false, false), "not a New Sampler");
    assert!(!tab_shown(&p, 0, 0, 4, true, true), "a take");
}

#[test]
fn a_file_dropped_on_a_lane_lands_there() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = layers_harness();
    assert_eq!(h.state().sampling.tab, 4, "an empty New Sampler stays on LAYERS");
    let l = lanes(&h);
    assert_eq!(l.len(), 8);
    drop_on(&mut h, l[0].center(), &[tone(dir.path(), "a.wav", 110.0), tone(dir.path(), "b.wav", 220.0)]);
    assert_eq!(names(&h), ["a.wav", "b.wav"], "{}", h.state().status);
    // On the third lane: a third layer. On the first: its sample replaced.
    let l = lanes(&h);
    drop_on(&mut h, l[2].center(), &[tone(dir.path(), "c.wav", 330.0)]);
    drop_on(&mut h, l[0].center(), &[tone(dir.path(), "d.wav", 440.0)]);
    assert_eq!(names(&h), ["d.wav", "b.wav", "c.wav"]);
    assert_eq!(h.state().sampling.layers.iter().flatten().count(), 3, "their waveforms are there");
    if let Some(out) = std::env::var_os("DECKARD_LAYERS_SHOT") {
        h.state_mut().morph_act(crate::layout::sampling::MorphAct::On(true));
        h.run_steps(4);
        h.render().unwrap().save(out).unwrap();
    }
}

#[test]
fn clicking_a_lane_focuses_its_zone() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = layers_harness();
    let files: Vec<PathBuf> = (0..3).map(|k| tone(dir.path(), &format!("{k}.wav"), 110.0 * (k + 1) as f32)).collect();
    h.state_mut().layer_files(0, &files);
    h.run_steps(3);
    assert_eq!(h.state().zone, 2);
    let r = lanes(&h)[0];
    click(&mut h, Pos2::new(r.left() + 60.0, r.center().y));
    assert_eq!(h.state().zone, 0);
    // Dragged down two lanes: it's the third.
    let to = lanes(&h)[2];
    drag(&mut h, Pos2::new(r.left() + 60.0, r.center().y), Pos2::new(to.left() + 60.0, to.center().y));
    assert_eq!(names(&h), ["1.wav", "2.wav", "0.wav"]);
}

#[test]
fn dragging_a_window_point_moves_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = layers_harness();
    h.state_mut().morph_act(crate::layout::sampling::MorphAct::Shape(Shape::Back));
    let files: Vec<PathBuf> = (0..3).map(|k| tone(dir.path(), &format!("{k}.wav"), 110.0 * (k + 1) as f32)).collect();
    h.state_mut().layer_files(0, &files);
    h.state_mut().zone = 1;
    h.run_steps(3);
    let window = |h: &Harness<'static, App>| h.state().layers_of()[1].1.window;
    assert_eq!(window(&h), spread(3, Shape::Back)[1]);
    // Its D, at the right end, dragged a little left.
    let d: egui::Rect = h.ctx.data(|x| x.get_temp(egui::Id::new(("deckard-layer-window", 3usize)))).expect("its points are shown");
    drag(&mut h, d.center(), d.center() - egui::vec2(120.0, 0.0));
    let w = window(&h);
    assert!(w[3] < 0.95 && w[3] > 0.6, "{w:?}");
    assert_eq!(w[..3], spread(3, Shape::Back)[1][..3]);
    let morph = project_of_app(h.state()).groups[0].sounds[0].new_sampler.unwrap().morph;
    assert!(!morph.spread, "by hand now");
}

#[test]
fn the_boxes_edit_the_focused_layer() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = layers_harness();
    let files: Vec<PathBuf> = (0..2).map(|k| tone(dir.path(), &format!("{k}.wav"), 110.0 * (k + 1) as f32)).collect();
    h.state_mut().layer_files(0, &files);
    h.state_mut().zone = 1;
    h.run_steps(3);
    let at = |h: &Harness<'static, App>, group: &str, label: &str| -> egui::Rect {
        let id = egui::Id::new(("deckard-sample-field", group.to_string(), label.to_string()));
        h.ctx.data(|d| d.get_temp(id)).unwrap_or_else(|| panic!("{group} {label} shown"))
    };
    for (group, label) in
        [("LAYER 2", "LEVEL"), ("LAYER 2", "ROOT"), ("ENVELOPE", "RELEASE"), ("MORPH", "RATE"), ("MORPH", "WINDOWS"), ("KEYS", "LOW")]
    {
        at(&h, group, label);
    }
    let undo = |h: &Harness<'static, App>| h.state().doc.as_ref().unwrap().borrow().undo_labels().len();
    let before = undo(&h);
    let r = at(&h, "ENVELOPE", "ATTACK");
    drag(&mut h, r.center(), r.center() - egui::vec2(0.0, 60.0));
    let l = h.state().layers_of();
    assert!(l[1].1.attack > 0.006, "{}", l[1].1.attack);
    assert_eq!(l[0].1.attack, 0.005, "the other layer's is its own");
    assert_eq!(undo(&h), before + 1, "one undo step");
    // The morph switched on with a click.
    let r = at(&h, "MORPH", "ON");
    click(&mut h, r.center());
    assert!(project_of_app(h.state()).groups[0].sounds[0].new_sampler.unwrap().morph.on);
}
