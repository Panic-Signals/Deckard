//! Scene and arrangement edits reproduce what Maschine 3 itself did to the same
//! project. Reference files: `$DECKARD_EDIT_REFS` (default ~/Desktop) holding
//! `test-1-assign.mxprj` … `test-7-section-length.mxprj`, each saved by Maschine after
//! one edit of the same project (see docs/mxprj-format.md §9). Skips if missing.

use deckard_doc::Document;
use deckard_model::Project;
use mxprj::decode::Value;
use std::path::PathBuf;

fn refs() -> Option<PathBuf> {
    let dir = std::env::var_os("DECKARD_EDIT_REFS")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Desktop")))?;
    dir.join("test-7-section-length.mxprj").exists().then_some(dir).or_else(|| {
        eprintln!("reference files not found; skipping");
        None
    })
}

fn open(dir: &std::path::Path, name: &str) -> Document {
    Document::open(&dir.join(format!("test-{name}.mxprj"))).unwrap()
}

/// What an arrangement edit should match: scene names and pattern choices, and per
/// section the scene and length.
type Shape = (Vec<(String, Vec<(usize, usize)>)>, Vec<(Option<usize>, i64)>);

fn shape(p: &Project) -> Shape {
    (p.scenes.iter().map(|s| (s.name.clone(), s.patterns.clone())).collect(), p.sections.iter().map(|s| (s.scene, s.length)).collect())
}

/// The section map's keys, in order.
fn section_keys(d: &Document) -> Vec<i64> {
    let mut keys = Vec::new();
    let mut in_sections = false;
    d.root().walk_paths(&mut |o, _| {
        if o.type_name.starts_with("NI::ML::TM::ObjectMap<unsigned long, NI::MASCHINE::DATA::Section>") {
            in_sections = true;
        }
        if in_sections && o.type_name.starts_with("std::pair<unsigned long const, std::shared_ptr<NI::MASCHINE::DATA::Section>>") {
            keys.push(o.get("first").and_then(Value::as_int).unwrap());
        }
    });
    keys
}

/// The baseline: test-7 with its section length edit undone (Maschine's own edit 7
/// was made on the same project).
fn baseline(dir: &std::path::Path) -> Document {
    let mut d = open(dir, "7-section-length");
    d.set_section_length(0, None).unwrap();
    d
}

/// Save, reopen, and check the saved file decodes to the same arrangement.
fn round_trip(d: &mut Document) -> Document {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("r.mxprj");
    d.save_as(&path).unwrap();
    let again = Document::open(&path).unwrap();
    assert_eq!(shape(again.project()), shape(d.project()), "saved file reads back the same");
    again
}

#[test]
fn section_length() {
    let Some(dir) = refs() else { return };
    let mut d = open(&dir, "1-assign");
    d.set_section_length(0, Some(46080)).unwrap();
    assert_eq!(shape(d.project()).1, shape(open(&dir, "7-section-length").project()).1);
    round_trip(&mut d);
    // Back to auto length: four bars again, as in every other file.
    d.set_section_length(0, None).unwrap();
    assert_eq!(shape(d.project()).1, shape(open(&dir, "1-assign").project()).1);
}

#[test]
fn add_scene() {
    let Some(dir) = refs() else { return };
    let mut d = baseline(&dir);
    let i = d.add_scene().unwrap();
    let (mut ours, want) = (shape(d.project()), shape(open(&dir, "2-add-scene").project()));
    // Maschine's file has the name the user typed.
    assert_eq!(ours.0[i].0, format!("Scene {}", i + 1));
    ours.0[i].0 = want.0[i].0.clone();
    assert_eq!(ours, want);
    round_trip(&mut d);
}

#[test]
fn duplicate_scene() {
    let Some(dir) = refs() else { return };
    let mut d = baseline(&dir);
    assert_eq!(d.duplicate_scene(0).unwrap(), 1);
    assert_eq!(shape(d.project()), shape(open(&dir, "3-dup-scene").project()));
    round_trip(&mut d);
}

#[test]
fn delete_scene() {
    let Some(dir) = refs() else { return };
    let mut d = baseline(&dir);
    let last = d.project().scenes.len() - 1;
    d.delete_scene(last).unwrap();
    let want = open(&dir, "4-del-scene");
    assert_eq!(shape(d.project()), shape(want.project()));
    assert_eq!(section_keys(&d), section_keys(&want));
    round_trip(&mut d);
}

#[test]
fn duplicate_section() {
    let Some(dir) = refs() else { return };
    let mut d = baseline(&dir);
    assert_eq!(d.duplicate_section(0).unwrap(), 1);
    let want = open(&dir, "5-dup-section");
    assert_eq!(shape(d.project()), shape(want.project()));
    assert_eq!(section_keys(&d), section_keys(&want));
    round_trip(&mut d);
}

#[test]
fn delete_section() {
    let Some(dir) = refs() else { return };
    let mut d = baseline(&dir);
    d.delete_section(1).unwrap();
    let want = open(&dir, "6-del-section");
    assert_eq!(shape(d.project()), shape(want.project()));
    assert_eq!(section_keys(&d), section_keys(&want), "deleting leaves a gap in the keys");
    round_trip(&mut d);
}

#[test]
fn assign_and_clear_a_pattern() {
    let Some(dir) = refs() else { return };
    let mut d = baseline(&dir);
    let before = d.project().scenes[0].patterns.clone();
    let group = 1;
    let pattern = d.project().groups[group].patterns.len() - 1;
    d.assign_pattern(0, group, Some(pattern)).unwrap();
    assert!(d.project().scenes[0].patterns.contains(&(group, pattern)));
    let again = round_trip(&mut d);
    assert!(again.project().scenes[0].patterns.contains(&(group, pattern)));
    d.assign_pattern(0, group, None).unwrap();
    assert!(!d.project().scenes[0].patterns.iter().any(|(g, _)| *g == group));
    assert!(d.undo() && d.undo());
    assert_eq!(d.project().scenes[0].patterns, before);
}

#[test]
fn move_and_rescene_sections_then_undo_everything() {
    let Some(dir) = refs() else { return };
    let mut d = baseline(&dir);
    let start = shape(d.project());
    d.move_section(0, 3).unwrap();
    assert_eq!(d.project().sections[3].scene, start.1[0].0);
    assert_eq!(section_keys(&d), (0..start.1.len() as i64).collect::<Vec<_>>());
    d.set_section_scene(0, 7).unwrap();
    assert_eq!(d.project().sections[0].scene, Some(7));
    let n = d.add_section(2).unwrap();
    assert_eq!(d.project().sections[n].scene, Some(2));
    round_trip(&mut d);
    while d.undo_label() != Some("Section Length") {
        assert!(d.undo());
    }
    assert_eq!(shape(d.project()), start);
}

#[test]
fn move_scenes_keeps_what_sections_play() {
    let Ok(mut d) = Document::open(std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj")) else {
        return;
    };
    let names = |d: &Document| d.project().scenes.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    // What each section plays, by scene name.
    let played =
        |d: &Document| d.project().sections.iter().map(|s| s.scene.map(|i| d.project().scenes[i].name.clone())).collect::<Vec<_>>();
    let (before, plays) = (names(&d), played(&d));
    d.move_scene(0, 3).unwrap();
    let mut want = before.clone();
    let s = want.remove(0);
    want.insert(3, s);
    assert_eq!(names(&d), want);
    assert_eq!(played(&d), plays, "sections play the same scenes");
    let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
    assert_eq!(names(&back), want);
    assert_eq!(played(&back), plays);
    assert!(d.undo());
    assert_eq!(names(&d), before);
}

/// A pattern's length is Auto until one is chosen for it (this session; not saved).
#[test]
fn pattern_length_is_auto_until_chosen() {
    let mut d = deckard_doc::Document::new_project().unwrap();
    d.add_pattern(0, 0, "Pattern 1").unwrap();
    assert!(d.length_is_auto(0, 0));
    d.set_length_auto(0, 0, false);
    assert!(!d.length_is_auto(0, 0));
    assert!(d.length_is_auto(0, 1), "another pattern");
    d.set_length_auto(0, 0, true);
    assert!(d.length_is_auto(0, 0));
}

/// Note Repeat's four presets (rate, unit), as the song stores them: a new project has
/// Maschine's (1/8, 1/16, 1/16 T, 1/32); one is changed as an undo step and saved.
#[test]
fn arp_presets_are_read_changed_and_saved() {
    let mut d = deckard_doc::Document::new_project().unwrap();
    assert_eq!(d.project().arp_presets, vec![(3, 0), (4, 0), (4, 1), (5, 0)]);
    d.set_arp_preset(0, 4, 1).unwrap();
    assert_eq!(d.project().arp_presets[0], (4, 1));
    assert_eq!(d.undo_label(), Some("Note Repeat"));
    let back = deckard_doc::Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
    assert_eq!(back.project().arp_presets[0], (4, 1), "saved");
    d.undo();
    assert_eq!(d.project().arp_presets[0], (3, 0));
    assert!(d.set_arp_preset(4, 4, 0).is_err(), "four presets");
}
