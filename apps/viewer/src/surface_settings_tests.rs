//! The Settings page's GENERAL settings that change how the app works (C32): the
//! metronome's TIMESIGNATURE, the Project, Scene / Section and Pattern defaults.

use deckard_surface::Action as A;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app
}

/// A new pattern in the focused group, in the focused scene.
fn new_pattern(app: &mut crate::App) {
    let new = crate::layout::Act::NewPattern(app.scene, app.group);
    crate::layout::apply_acts(app, vec![new]);
}

#[test]
fn the_general_defaults_are_settings() {
    let mut app = app();
    let shown = |app: &mut crate::App, k: &str| app.settings_view().values.get(k).cloned().unwrap_or_default();
    let keys = ["metronome_ticks", "reload_last", "auto_backup", "duplicate", "link_duplicates", "pattern_length", "pattern_grow"];
    assert_eq!(keys.map(|k| shown(&mut app, k)), ["1/4", "On", "On", "Scene", "Off", "1:0:0", "On"]);
    app.surface_act(A::Setting("metronome_ticks", 0, 1));
    app.surface_act(A::Setting("reload_last", 0, -1));
    app.surface_act(A::Setting("auto_backup", 0, -1));
    app.surface_act(A::Setting("duplicate", 0, 1));
    app.surface_act(A::Setting("link_duplicates", 0, 1));
    app.surface_act(A::Setting("pattern_length", 0, 1));
    app.surface_act(A::Setting("pattern_length_fine", 0, 5));
    app.surface_act(A::Setting("pattern_grow", 0, -1));
    assert_eq!(keys.map(|k| shown(&mut app, k)), ["1/8", "Off", "Off", "Scn & Ptn", "On", "2:1:1", "Off"]);
    let k = crate::settings::Settings::parse(&app.settings.to_text());
    let s = &app.settings;
    assert_eq!(
        (k.metronome_ticks, k.reload_last, k.auto_backup, k.duplicate_patterns, k.link_duplicates, k.pattern_length, k.pattern_grow),
        (s.metronome_ticks, s.reload_last, s.auto_backup, s.duplicate_patterns, s.link_duplicates, s.pattern_length, s.pattern_grow)
    );
    // (A sixteenth at the least.)
    app.surface_act(A::Setting("pattern_length", 0, -9));
    app.surface_act(A::Setting("pattern_length_fine", 0, -99));
    assert_eq!(shown(&mut app, "pattern_length"), "0:0:1");
}

/// LENGTH: a new pattern is that long (the project's bars and beats).
#[test]
fn new_patterns_take_the_default_length() {
    let mut app = app();
    app.surface_act(A::Setting("pattern_length", 0, 1));
    app.surface_act(A::Setting("pattern_length_fine", 0, 4));
    new_pattern(&mut app);
    let d = app.doc.as_ref().unwrap().borrow();
    let p = d.project();
    let (bar, beat) = (p.bar_length(), deckard_model::PPQ);
    assert_eq!(p.groups[app.group].patterns[app.pattern].length, 2 * bar + beat);
}

/// GROW off: recording into an empty pattern keeps its length.
#[test]
fn grow_off_keeps_an_empty_patterns_length() {
    let mut app = app();
    new_pattern(&mut app);
    app.surface_act(A::Setting("pattern_grow", 0, -1));
    let before = app.doc.as_ref().unwrap().borrow().project().groups[app.group].patterns[app.pattern].length;
    app.set_recording(true, false);
    assert!(app.grow.is_none());
    let after = app.doc.as_ref().unwrap().borrow().project().groups[app.group].patterns[app.pattern].length;
    assert_eq!(after, before);
}

/// DUPLICATE "Scn & Ptn": the copy plays copies of the scene's patterns.
#[test]
fn duplicating_a_scene_with_its_patterns() {
    let mut app = app();
    new_pattern(&mut app);
    let patterns = |app: &crate::App| app.doc.as_ref().unwrap().borrow().project().groups[0].patterns.len();
    let n = patterns(&app);
    app.surface_act(A::Setting("duplicate", 0, 1));
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::DuplicateScene(0)]);
    assert_eq!(patterns(&app), n + 1);
    let d = app.doc.as_ref().unwrap().borrow();
    let p = d.project();
    assert_eq!(p.scenes.len(), 2);
    assert!(p.scenes[1].patterns.contains(&(0, n)), "the copy's own pattern: {:?}", p.scenes[1].patterns);
}

/// LINK WHEN DUPL: a duplicated section plays the same scene.
#[test]
fn duplicating_a_section_linked() {
    let mut app = app();
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::AddSection(0)]);
    app.surface_act(A::Setting("link_duplicates", 0, 1));
    let scenes = app.doc.as_ref().unwrap().borrow().project().scenes.len();
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::DuplicateSection(0)]);
    let d = app.doc.as_ref().unwrap().borrow();
    let p = d.project();
    assert_eq!(p.scenes.len(), scenes, "no new scene");
    assert_eq!(p.sections.len(), 2);
    assert_eq!(p.sections[0].scene, p.sections[1].scene);
}

/// RELOAD off: DECKARD starts with a new project, unless the last one had unsaved
/// changes (as Maschine offers its backup).
#[test]
fn reload_off_starts_afresh_unless_work_was_unsaved() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("session");
    let mut app = crate::App { session_dir: Some(dir.clone()), ..Default::default() };
    app.new_project();
    app.doc.as_ref().unwrap().borrow_mut().set_tempo(99.0).unwrap();
    app.keep_session();
    // (As though saved.)
    std::fs::write(dir.join("session.txt"), "dirty = false\n").unwrap();
    let start = |dir: &std::path::Path| {
        let mut next = crate::App { session_dir: Some(dir.to_path_buf()), ..Default::default() };
        next.settings.reload_last = false;
        next.start_project();
        next.doc.as_ref().unwrap().borrow().project().tempo
    };
    assert_ne!(start(&dir), 99.0, "saved: a new project");
    app.doc.as_ref().unwrap().borrow_mut().set_tempo(98.0).unwrap();
    app.keep_session();
    assert_eq!(start(&dir), 98.0, "unsaved: back");
}

/// AUTO BACKUP off: edits aren't kept as they're made (only on quitting).
#[test]
fn auto_backup_off_keeps_nothing_after_an_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("session");
    let mut app = crate::App { session_dir: Some(dir.clone()), ..Default::default() };
    app.new_project();
    app.settings.auto_backup = false;
    app.doc.as_ref().unwrap().borrow_mut().set_tempo(97.0).unwrap();
    app.session_due = Some(std::time::Instant::now());
    app.keep_session_soon();
    assert!(!dir.join("session.mxprj").exists());
}
