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
        let layers = deckard_model::new_sampler::NewSampler { mode: deckard_model::new_sampler::SamplerMode::Layers, ..Default::default() };
        d.set_new_sampler(0, 0, Some(&layers)).unwrap();
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

/// In Layers: RECORD and LAYERS when empty, and no SLICE with a sample. In the other
/// modes no LAYERS, and SLICE. No LAYERS for a take or a plain Sampler.
#[test]
fn the_tab_is_offered_where_it_can_be() {
    use deckard_model::new_sampler::SamplerMode;
    let dir = tempfile::tempdir().unwrap();
    let shown = |app: &App, zone: bool| -> Vec<bool> {
        let p = project_of_app(app);
        (0..5).map(|i| tab_shown(&p, 0, 0, i, zone, false)).collect()
    };
    let load = |app: &App| {
        let doc = app.doc.as_ref().unwrap();
        let spec =
            deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Template, file: Some(tone(dir.path(), "a.wav", 220.0)), set: Vec::new() };
        doc.borrow_mut().set_zones(0, 0, &[spec], "Load").unwrap();
    };
    let mut app = crate::layers_tests::app();
    assert_eq!(shown(&app, false), [true, false, false, false, true]);
    load(&app);
    assert_eq!(shown(&app, true), [true, true, false, true, true], "Layers: no SLICE");
    app.layer_files(1, &[tone(dir.path(), "b.wav", 330.0)]);
    assert_eq!(shown(&app, true), [true, true, false, true, true]);
    for mode in [SamplerMode::Advanced, SamplerMode::Original] {
        let app = crate::layers_tests::app_in(mode);
        assert_eq!(shown(&app, false), [true, false, false, false, false], "{mode:?}");
        load(&app);
        assert_eq!(shown(&app, true), [true, true, true, true, false], "{mode:?}: SLICE, no LAYERS");
    }
    let p = project_of_app(&app);
    assert!(!tab_shown(&p, 0, 1, 4, false, false), "not a Sampler Preview");
}

#[test]
fn a_file_dropped_on_a_lane_lands_there() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = layers_harness();
    assert_eq!(h.state().sampling.tab, 4, "an empty Sampler Preview stays on LAYERS");
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

/// A double-click on the field at `r`, `text` typed over its value, Enter.
fn type_into(h: &mut Harness<'static, App>, r: egui::Rect, text: &str) {
    let pos = r.center();
    h.hover_at(pos);
    h.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
            h.step();
        }
    }
    h.run_steps(3);
    h.event(egui::Event::Text(text.into()));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() });
        h.run_steps(3);
    }
    // (Past the double-click time, so the next field's clicks are its own.)
    h.run_steps(40);
}

/// A double-click on a footer value types it: a time with its unit, a level in dB, a pan,
/// a note by name, a synced rate; what can't be read changes nothing.
#[test]
fn a_footer_value_is_typed_with_a_double_click() {
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
    let mut set = |h: &mut Harness<'static, App>, group: &str, label: &str, text: &str| {
        let r = at(h, group, label);
        type_into(h, r, text);
    };
    set(&mut h, "ENVELOPE", "ATTACK", "250");
    assert!((h.state().layers_of()[1].1.attack - 0.25).abs() < 1e-9, "a plain number is in the unit shown (ms)");
    set(&mut h, "ENVELOPE", "RELEASE", "1.5 s");
    assert!((h.state().layers_of()[1].1.release - 1.5).abs() < 1e-9, "{:?}", h.state().layers_of()[1].1);
    set(&mut h, "ENVELOPE", "SUSTAIN", "40");
    assert!((h.state().layers_of()[1].1.sustain - 0.4).abs() < 1e-9);
    set(&mut h, "LAYER 2", "LEVEL", "-6");
    set(&mut h, "LAYER 2", "PAN", "30L");
    set(&mut h, "LAYER 2", "TUNE", "-12");
    set(&mut h, "LAYER 2", "ROOT", "f#2");
    let z = zones(h.state())[1].clone();
    assert!((deckard_model::level_db(z.gain) + 6.0).abs() < 0.01, "{}", deckard_model::level_db(z.gain));
    assert!((z.pan + 0.3).abs() < 1e-9, "{}", z.pan);
    assert_eq!((z.tune, z.root_key), (-12.0, 54));
    assert_eq!(zones(h.state())[0].root_key != 54, true, "the other layer's is its own");
    set(&mut h, "MORPH", "POSITION", "25 %");
    set(&mut h, "MORPH", "RATE", "1/8");
    let m = project_of_app(h.state()).groups[0].sounds[0].new_sampler.unwrap().morph;
    assert_eq!((m.position, m.sync, m.bars), (0.25, true, 0.125));
    set(&mut h, "KEYS", "LOW", "C1");
    assert_eq!(zones(h.state())[0].key_range.0, 36);
    // Nonsense is dropped.
    let undo = |h: &Harness<'static, App>| h.state().doc.as_ref().unwrap().borrow().undo_labels().len();
    let before = undo(&h);
    set(&mut h, "LAYER 2", "TUNE", "loud");
    assert_eq!((zones(h.state())[1].tune, undo(&h)), (-12.0, before));
}

/// A click on a field with several options lists them all, to pick from.
#[test]
fn a_click_on_a_choice_lists_its_options() {
    use egui_kittest::kittest::Queryable;
    let dir = tempfile::tempdir().unwrap();
    let mut h = layers_harness();
    h.state_mut().layer_files(0, &[tone(dir.path(), "a.wav", 110.0)]);
    h.run_steps(3);
    let id = egui::Id::new(("deckard-sample-field", "MORPH".to_string(), "SHAPE".to_string()));
    let r: egui::Rect = h.ctx.data(|d| d.get_temp(id)).expect("SHAPE shown");
    click(&mut h, r.center());
    h.run_steps(3);
    h.get_by_label("There and back").click();
    h.run_steps(3);
    assert_eq!(project_of_app(h.state()).groups[0].sounds[0].new_sampler.unwrap().morph.shape, Shape::Back);
}

/// On SLICE two boxes are called SLICE (the slice's own settings over the waveform, the
/// slicing in the footer): a click on the footer's MODE lists the modes and picks one, and
/// a double-click on its count types nothing into the slice's TAIL.
#[test]
fn the_two_slice_boxes_are_each_their_own() {
    use egui_kittest::kittest::Queryable;
    let dir = tempfile::tempdir().unwrap();
    let mut h = layers_harness();
    h.state_mut().layer_files(0, &[tone(dir.path(), "a.wav", 110.0)]);
    {
        // The same sample as a plain zone with a tempo, so it can be sliced.
        let mut m = zones(h.state())[0].meta.clone().unwrap();
        (m.layer, m.bpm, m.beats) = (None, Some(120.0), Some(4.0));
        let doc = h.state().doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        d.set_zone_meta(0, 0, 0, &m, "Plain").unwrap();
        // (In Advanced: Layers has no SLICE.)
        d.set_new_sampler(0, 0, Some(&Default::default())).unwrap();
    }
    (h.state_mut().sampling.tab, h.state_mut().zone) = (2, 0);
    h.run_steps(4);
    let at = |h: &Harness<'static, App>, label: &str| -> egui::Rect {
        let id = egui::Id::new(("deckard-sample-field", "SLICE".to_string(), label.to_string()));
        h.ctx.data(|d| d.get_temp(id)).unwrap_or_else(|| panic!("SLICE {label} shown"))
    };
    at(&h, "TAIL");
    let r = at(&h, "MODE");
    click(&mut h, r.center());
    h.get_by_label("Grid").click();
    h.run_steps(3);
    assert_eq!(h.state().sampling.slice_mode, 1);
    assert_eq!(deckard_model::new_sampler::Plays::of(&zones(h.state())[0]), deckard_model::new_sampler::Plays::Held);
    h.run_steps(40);
    let r = at(&h, "SLICES");
    type_into(&mut h, r, "22");
    assert_eq!(zones(h.state())[0].meta.as_ref().unwrap().tail_ms(), 0.0);
}

/// RECORD's LAYER is the focused take's; APPLY shows once it is turned, and puts the take
/// on that layer.
#[test]
fn the_record_footer_applies_a_takes_layer() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = layers_harness();
    h.state_mut().layer_files(0, &[tone(dir.path(), "a.wav", 110.0), tone(dir.path(), "b.wav", 220.0)]);
    h.state_mut().recorder.view.takes.push(tone(dir.path(), "take.wav", 330.0));
    h.state_mut().recorder.view.focused = Some(0);
    h.state_mut().sampling.tab = 0;
    h.run_steps(4);
    let at = |h: &Harness<'static, App>, label: &str| -> Option<egui::Rect> {
        h.ctx.data(|d| d.get_temp(egui::Id::new(("deckard-sample-field", "RECORDING".to_string(), label.to_string()))))
    };
    assert!(at(&h, "LAYER").is_some(), "the take's LAYER");
    h.state_mut().recorder.view.choose_layer((0, 0), 1);
    h.run_steps(3);
    let apply = at(&h, "TAKE").expect("APPLY, once a layer is chosen");
    click(&mut h, apply.center());
    h.run_steps(3);
    assert_eq!(names(&h), ["a.wav", "take.wav"]);
    assert_eq!(h.state().recorder.view.layer_choice, None);
}

/// The header's SAMPLER picks the mode: a sound with samples is asked first, in a dialog
/// with the words agreed, OK and Cancel.
#[test]
fn the_header_has_sampler() {
    use deckard_model::new_sampler::SamplerMode;
    use egui_kittest::kittest::Queryable;
    const ASK: &str = "Changing the mode loses all changes to this sound.";
    let dir = tempfile::tempdir().unwrap();
    let mut h = layers_harness();
    let mode = |h: &Harness<'static, App>| project_of_app(h.state()).groups[0].sounds[0].new_sampler.unwrap().mode;
    let at = |h: &Harness<'static, App>, k: usize| -> Pos2 {
        let r: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new(("deckard-sampler-mode", k)))).expect("SAMPLER shown");
        r.center()
    };
    h.state_mut().layer_files(0, &[tone(dir.path(), "a.wav", 110.0), tone(dir.path(), "b.wav", 220.0)]);
    h.run_steps(3);
    let p = at(&h, 1);
    click(&mut h, p);
    h.run_steps(3);
    h.get_by_label(ASK);
    assert_eq!(mode(&h), SamplerMode::Layers, "asked first");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.query_by_label(ASK).is_none());
    assert_eq!((mode(&h), zones(h.state()).len()), (SamplerMode::Layers, 2));
    click(&mut h, p);
    h.run_steps(3);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.query_by_label(ASK).is_none());
    assert_eq!((mode(&h), zones(h.state()).len()), (SamplerMode::Advanced, 1));
    assert!(!tab_shown(&project_of_app(h.state()), 0, 0, h.state().sampling.tab, true, false) || h.state().sampling.tab != 4, "off LAYERS");
    if let Ok(dir) = std::env::var("DECKARD_SHOTS") {
        h.render().unwrap().save(std::path::Path::new(&dir).join("sampler-mode.png")).unwrap();
    }
}
