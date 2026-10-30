//! The browser in the window: the panel, its queries and loading what it lists. NI's
//! index is a fixture and DECKARD's own a temp file (never the real ones).

use crate::App;
use crate::browser::BrowseAct;
use crate::layout::{self, Act};
use crate::tests::harness;
use deckard_library::Kind;
use eframe::egui;
use egui_kittest::Harness;
use std::path::{Path, PathBuf};

/// A fresh folder, and the guard that removes it.
fn temp() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    (tmp, dir)
}

fn wav(path: &Path, frames: usize) {
    let spec = hound::WavSpec { channels: 2, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..frames {
        let v = ((i as f32 * 0.05).sin() * 8000.0) as i16;
        w.write_sample(v).unwrap();
        w.write_sample(v).unwrap();
    }
    w.finalize().unwrap();
}

/// The window with the browser open on the fixture index and an own index in `dir`.
fn open_browser(dir: &Path) -> Option<Harness<'static, App>> {
    let mut h = harness(false, false)?;
    let ni = deckard_library::testing::fixture(dir);
    h.state_mut().browser.start(Some(ni), dir.join("library.db3"), Vec::new());
    act(&mut h, BrowseAct::Toggle);
    Some(h)
}

fn act(h: &mut Harness<'static, App>, a: BrowseAct) {
    layout::apply_acts(h.state_mut(), vec![Act::Browse(a)]);
    h.state_mut().browser.wait();
    h.run_steps(2);
}

fn names(h: &Harness<'static, App>) -> Vec<String> {
    h.state().browser.results.items.iter().map(|i| i.name.clone()).collect()
}

#[test]
fn f4_toggles_the_browser() {
    let Some(mut h) = harness(false, false) else { return };
    let (_tmp, dir) = temp();
    h.state_mut().browser.start(None, dir.join("library.db3"), Vec::new());
    assert!(!h.state().browser.open);
    h.key_press(egui::Key::F4);
    h.run_steps(2);
    assert!(h.state().browser.open);
    h.key_press(egui::Key::F4);
    h.run_steps(2);
    assert!(!h.state().browser.open);
}

#[test]
fn choosing_a_type_narrows_results() {
    let (_tmp, dir) = temp();
    let Some(mut h) = open_browser(&dir) else { return };
    act(&mut h, BrowseAct::Kind(Kind::Loop));
    assert_eq!(names(&h).len(), 3);
    act(&mut h, BrowseAct::Types(vec!["Drums".into()]));
    assert_eq!(names(&h), ["Kick[120] Lagoon", "Snare[90] Dust"]);
    act(&mut h, BrowseAct::Character("Clean".into(), true));
    assert_eq!(names(&h), ["Kick[120] Lagoon"]);
    // The panel says how many.
    use egui_kittest::kittest::Queryable;
    assert!(h.query_by_label("1 results").is_some());
}

#[test]
fn double_click_loads_a_one_shot_into_the_focused_sound() {
    let (_tmp, dir) = temp();
    let Some(mut h) = open_browser(&dir) else { return };
    let hit = dir.join("Maschine 3/Samples/Hit.wav");
    std::fs::create_dir_all(hit.parent().unwrap()).unwrap();
    wav(&hit, 4410);
    h.state_mut().browser.add_saved(&hit);
    act(&mut h, BrowseAct::User(true));
    act(&mut h, BrowseAct::Kind(Kind::OneShot));
    let i = names(&h).iter().position(|n| n == "Hit").expect("the saved one-shot is listed");
    // An empty sound gets a Sampler with the sample.
    let (g, s) = empty_sound(&h);
    h.state_mut().group = g;
    h.state_mut().sound = s;
    act(&mut h, BrowseAct::Load(i, None));
    let doc = h.state().doc.clone().unwrap();
    let d = doc.borrow();
    let zones = &d.project().groups[g].sounds[s].sampler.as_ref().expect("a Sampler").zones;
    assert_eq!(zones.len(), 1);
    assert!(zones[0].sample.path.ends_with("Hit.wav"));
}

#[test]
fn dragging_a_loop_onto_an_empty_sound_makes_an_audio_plugin() {
    let (_tmp, dir) = temp();
    let Some(mut h) = open_browser(&dir) else { return };
    let lp = dir.join("Maschine 3/Samples/Beat[120] Test.wav");
    std::fs::create_dir_all(lp.parent().unwrap()).unwrap();
    wav(&lp, 88_200);
    h.state_mut().browser.add_saved(&lp);
    act(&mut h, BrowseAct::User(true));
    act(&mut h, BrowseAct::Kind(Kind::Loop));
    assert_eq!(names(&h), ["Beat[120] Test"]);
    let (g, s) = empty_sound(&h);
    act(&mut h, BrowseAct::Load(0, Some((g, s))));
    let doc = h.state().doc.clone().unwrap();
    let d = doc.borrow();
    let audio = d.project().groups[g].sounds[s].audio.as_ref().expect("an Audio plug-in");
    assert_eq!(audio.takes.len(), 1);
}

#[test]
fn missing_result_reports_and_loads_nothing() {
    let (_tmp, dir) = temp();
    let Some(mut h) = open_browser(&dir) else { return };
    act(&mut h, BrowseAct::Kind(Kind::Loop));
    let rev = h.state().doc.as_ref().unwrap().borrow().revision();
    act(&mut h, BrowseAct::Load(0, None));
    assert!(h.state().status.contains("isn't there"), "{}", h.state().status);
    assert_eq!(h.state().doc.as_ref().unwrap().borrow().revision(), rev);
}

#[test]
fn favorites_mark_and_filter_results() {
    let (_tmp, dir) = temp();
    let Some(mut h) = open_browser(&dir) else { return };
    act(&mut h, BrowseAct::Kind(Kind::Loop));
    act(&mut h, BrowseAct::Favorite(1, true));
    act(&mut h, BrowseAct::Favorites(true));
    assert_eq!(names(&h), ["Kick[120] Lagoon"]);
}

/// A sound with no plug-ins in the focused group.
fn empty_sound(h: &Harness<'static, App>) -> (usize, usize) {
    let g = h.state().group;
    let doc = h.state().doc.clone().unwrap();
    let d = doc.borrow();
    let s = d.project().groups[g].sounds.iter().position(|x| x.slots.is_empty()).expect("an empty sound");
    (g, s)
}

/// A picture of the panel on a copy of NI's index (run by hand:
/// `DECKARD_NI_COPY=<copy of komplete.db3> cargo test -p deckard -- --ignored browser_picture`).
#[test]
#[ignore]
fn browser_picture() {
    let Ok(copy) = std::env::var("DECKARD_NI_COPY") else { return };
    let out = std::env::var("DECKARD_PICTURE").unwrap_or_else(|_| "browser.png".into());
    let (_tmp, dir) = temp();
    let Some(mut h) = harness(false, false) else { return };
    h.state_mut().browser.start(Some(PathBuf::from(copy)), dir.join("library.db3"), Vec::new());
    act(&mut h, BrowseAct::Toggle);
    act(&mut h, BrowseAct::Kind(Kind::Loop));
    act(&mut h, BrowseAct::Types(vec!["Drums".into()]));
    act(&mut h, BrowseAct::Select(3));
    h.run_steps(4);
    h.render().unwrap().save(out).unwrap();
}

#[test]
fn files_lists_folders_then_supported_files() {
    let (_tmp, dir) = temp();
    std::fs::create_dir_all(dir.join("b folder")).unwrap();
    std::fs::create_dir_all(dir.join(".hidden")).unwrap();
    for f in ["Zed.wav", "a.aif", "kit.mxgrp", "notes.txt", "song.mxprj"] {
        std::fs::write(dir.join(f), b"x").unwrap();
    }
    let names: Vec<(String, bool)> =
        crate::browser::list_folder(&dir).iter().map(|e| (e.path.file_name().unwrap().to_string_lossy().into_owned(), e.dir)).collect();
    assert_eq!(
        names,
        [
            ("b folder".into(), true),
            ("a.aif".into(), false),
            ("kit.mxgrp".into(), false),
            ("song.mxprj".into(), false),
            ("Zed.wav".into(), false)
        ]
    );
}

#[test]
fn pinned_folder_persists() {
    let (_tmp, dir) = temp();
    let Some(mut h) = harness(false, false) else { return };
    act(&mut h, BrowseAct::PinFolder(dir.clone(), true));
    assert!(h.state().settings.favorite_folders.contains(&dir));
    let back = crate::settings::Settings::parse(&h.state().settings.to_text());
    assert!(back.favorite_folders.contains(&dir));
    act(&mut h, BrowseAct::PinFolder(dir.clone(), false));
    assert!(!h.state().settings.favorite_folders.contains(&dir));
}

#[test]
fn double_click_a_wav_in_files_loads_it() {
    let (_tmp, dir) = temp();
    let Some(mut h) = harness(false, false) else { return };
    let lp = dir.join("Groove[94].wav");
    wav(&lp, 225_186);
    act(&mut h, BrowseAct::Folder(dir.clone()));
    assert_eq!(h.state().browser.files.len(), 1);
    let (g, s) = empty_sound(&h);
    h.state_mut().sound = s;
    act(&mut h, BrowseAct::LoadFile(lp.clone(), None));
    let doc = h.state().doc.clone().unwrap();
    assert!(doc.borrow().project().groups[g].sounds[s].audio.is_some(), "a loop on an empty sound is an Audio plug-in");
    // Up goes to the parent folder.
    act(&mut h, BrowseAct::Up);
    assert_eq!(h.state().browser.files_dir, dir.parent().unwrap());
}

/// The window, its user folder a temp one, and the library on it.
fn saving() -> Option<(tempfile::TempDir, Harness<'static, App>, PathBuf)> {
    let (tmp, dir) = temp();
    let mut h = harness(false, false)?;
    h.state_mut().user_dir_override = Some(dir.join("Maschine 3"));
    h.state_mut().browser.start(None, dir.join("library.db3"), Vec::new());
    Some((tmp, h, dir))
}

fn save_as(h: &mut Harness<'static, App>, what: deckard_doc::FileItem, samples: bool, name: &str) {
    layout::apply_acts(
        h.state_mut(),
        vec![Act::SaveItem(what, if samples { crate::SaveMode::WithSamples } else { crate::SaveMode::SaveAs })],
    );
    let d = h.state_mut().save_dialog.as_mut().expect("the Save dialog");
    d.name = name.into();
    d.types = "Drums › Kick".into();
    d.characters = "Punchy, Dark".into();
    h.state_mut().confirm_save();
    h.state_mut().browser.wait();
    h.run_steps(2);
}

fn sound_with_sampler(h: &Harness<'static, App>) -> (usize, usize) {
    let doc = h.state().doc.clone().unwrap();
    let d = doc.borrow();
    let p = d.project();
    (0..p.groups.len())
        .flat_map(|g| (0..p.groups[g].sounds.len()).map(move |s| (g, s)))
        .find(|&(g, s)| p.groups[g].sounds[s].sampler.as_ref().is_some_and(|x| !x.zones.is_empty()))
        .expect("a sound with a sample")
}

#[test]
fn save_sound_writes_mxsnd_with_tags_and_lists_it_under_user() {
    let Some((_tmp, mut h, dir)) = saving() else { return };
    let (g, s) = sound_with_sampler(&h);
    save_as(&mut h, deckard_doc::FileItem::Sound(g, s), false, "My Kick");
    let file = dir.join("Maschine 3/Sounds/My Kick.mxsnd");
    let bytes = std::fs::read(&file).expect("saved in the user folder's Sounds");
    let info = mxprj::Project::parse(&bytes).unwrap().sound_info().unwrap();
    assert_eq!(info.name, "My Kick");
    assert_eq!(info.types(), [vec!["Drums".to_string(), "Kick".to_string()]]);
    assert_eq!(info.characters(), ["Punchy", "Dark"]);
    assert_eq!(info.bank, ["User"]);
    act(&mut h, BrowseAct::User(true));
    act(&mut h, BrowseAct::Kind(Kind::Sound));
    assert_eq!(names(&h), ["My Kick"]);
}

#[test]
fn save_as_existing_name_asks() {
    let Some((_tmp, mut h, dir)) = saving() else { return };
    let file = dir.join("Maschine 3/Sounds/Taken.mxsnd");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"theirs").unwrap();
    layout::apply_acts(h.state_mut(), vec![Act::SaveItem(deckard_doc::FileItem::Sound(0, 0), crate::SaveMode::SaveAs)]);
    h.state_mut().save_dialog.as_mut().unwrap().name = "Taken".into();
    h.state_mut().confirm_save();
    assert!(h.state().save_dialog.as_ref().is_some_and(|d| d.confirm_overwrite), "asks first");
    assert_eq!(std::fs::read(&file).unwrap(), b"theirs");
    // Cancel leaves it.
    h.state_mut().save_dialog = None;
    assert_eq!(std::fs::read(&file).unwrap(), b"theirs");
}

#[test]
fn save_again_overwrites_without_dialog() {
    let Some((_tmp, mut h, dir)) = saving() else { return };
    let (g, s) = sound_with_sampler(&h);
    save_as(&mut h, deckard_doc::FileItem::Sound(g, s), false, "Again");
    let file = dir.join("Maschine 3/Sounds/Again.mxsnd");
    let first = std::fs::read(&file).unwrap();
    // A change, then Save: no dialog, the same file, the same tags.
    h.state().doc.clone().unwrap().borrow_mut().set_level(deckard_doc::Target::Sound(g, s), 0.25).unwrap();
    layout::apply_acts(h.state_mut(), vec![Act::SaveItem(deckard_doc::FileItem::Sound(g, s), crate::SaveMode::Save)]);
    assert!(h.state().save_dialog.is_none());
    let second = std::fs::read(&file).unwrap();
    assert_ne!(first, second);
    let info = mxprj::Project::parse(&second).unwrap().sound_info().unwrap();
    assert_eq!(info.characters(), ["Punchy", "Dark"]);
}

#[test]
fn save_with_samples_copies_them() {
    let Some((_tmp, mut h, dir)) = saving() else { return };
    let (g, s) = sound_with_sampler(&h);
    save_as(&mut h, deckard_doc::FileItem::Sound(g, s), true, "Packed");
    let samples = dir.join("Maschine 3/Sounds/Packed Samples");
    assert!(std::fs::read_dir(&samples).map(|r| r.count()).unwrap_or(0) > 0, "samples copied beside it");
}

#[test]
fn save_preset_writes_mxfx() {
    let Some((_tmp, mut h, dir)) = saving() else { return };
    let (g, s, k) = {
        let doc = h.state().doc.clone().unwrap();
        let d = doc.borrow();
        d.project()
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, x)| x.sounds.iter().enumerate().map(move |(s, y)| (g, s, y)))
            .find_map(|(g, s, y)| y.slots.iter().position(|x| !x.is_instrument()).map(|k| (g, s, k)))
            .unwrap()
    };
    save_as(&mut h, deckard_doc::FileItem::Slot(g, s, k), false, "My FX");
    assert!(dir.join("Maschine 3/Plug-ins/My FX.mxfx").is_file());
}

/// BROWSE on the controller shows the app's library, and its LOAD loads into the
/// focused sound (the 4-D encoder moves the focus first).
#[test]
fn controller_browse_load_loads_into_the_focused_sound() {
    let (_tmp, dir) = temp();
    let Some(mut h) = harness(true, false) else { return };
    if !matches!(h.state().controller, Some(Ok(_))) {
        return;
    }
    let hit = dir.join("Maschine 3/Samples/Hit.wav");
    std::fs::create_dir_all(hit.parent().unwrap()).unwrap();
    wav(&hit, 4410);
    let ni = deckard_library::testing::fixture(&dir);
    h.state_mut().browser.start(Some(ni), dir.join("library.db3"), Vec::new());
    h.state_mut().browser.add_saved(&hit);
    act(&mut h, BrowseAct::User(true));
    act(&mut h, BrowseAct::Kind(Kind::OneShot));
    let i = names(&h).iter().position(|n| n == "Hit").unwrap();
    let (g, s) = empty_sound(&h);
    h.state_mut().group = g;
    h.state_mut().sound = s;
    h.run_steps(3);
    // What the controller shows is the app's library.
    let view = match &h.state().controller {
        Some(Ok(c)) => c.host.state.borrow().browser.clone(),
        _ => unreachable!(),
    };
    assert_eq!(view.kind, 6);
    assert!(view.user);
    assert!(view.results.iter().any(|r| r.0 == "Hit"));
    // The page's calls: focus on "Hit", then LOAD.
    if let Some(Ok(c)) = &h.state().controller {
        let mut st = c.host.state.borrow_mut();
        st.browse_calls.push(deckard_lua_host::state::BrowseCall::Focus(i as i64));
        st.browse_calls.push(deckard_lua_host::state::BrowseCall::Load);
    }
    h.run_steps(3);
    let doc = h.state().doc.clone().unwrap();
    let d = doc.borrow();
    let zones = &d.project().groups[g].sounds[s].sampler.as_ref().expect("a Sampler").zones;
    assert!(zones[0].sample.path.ends_with("Hit.wav"));
}

/// A library folder added in Preferences is listed under Factory, its name (less
/// " Library") as the product; removing it takes it out.
#[test]
fn library_folder_items_show_as_factory() {
    let Some((_tmp, mut h, dir)) = saving() else { return };
    let pack = dir.join("Retro Pack Library");
    deckard_library::testing::tagged_file(&pack.join("Sounds/Tape Snare.mxsnd"), "Tape Snare", &[&["Drums", "Snare"]], &[]);
    act(&mut h, BrowseAct::LibraryFolder(pack.clone(), true));
    assert!(h.state().settings.library_folders.contains(&pack));
    act(&mut h, BrowseAct::Kind(Kind::Sound));
    let item = h.state().browser.results.items.iter().find(|i| i.name == "Tape Snare").cloned().expect("listed under Factory");
    assert_eq!(item.product.as_deref(), Some("Retro Pack"));
    assert!(!item.user);
    act(&mut h, BrowseAct::LibraryFolder(pack.clone(), false));
    act(&mut h, BrowseAct::Kind(Kind::Sound));
    assert!(names(&h).iter().all(|n| n != "Tape Snare"));
}

/// Save goes back to the file only for the same sound: in another project (or after
/// the sound at that place changed) it asks again rather than overwrite.
#[test]
fn save_after_opening_another_project_asks_again() {
    let Some((_tmp, mut h, _dir)) = saving() else { return };
    let (g, s) = sound_with_sampler(&h);
    save_as(&mut h, deckard_doc::FileItem::Sound(g, s), false, "First");
    let path = h.state().path.clone().unwrap();
    h.state_mut().open(&path);
    h.run_steps(2);
    layout::apply_acts(h.state_mut(), vec![Act::SaveItem(deckard_doc::FileItem::Sound(g, s), crate::SaveMode::Save)]);
    assert!(h.state().save_dialog.is_some(), "asks where to save");
}

/// The controller is handed the library only when it changed.
#[test]
fn the_view_revision_moves_only_on_change() {
    let (_tmp, dir) = temp();
    let Some(mut h) = open_browser(&dir) else { return };
    let r0 = h.state().browser.revision();
    h.run_steps(3);
    assert_eq!(h.state().browser.revision(), r0);
    act(&mut h, BrowseAct::Kind(Kind::Loop));
    assert_ne!(h.state().browser.revision(), r0);
    let r1 = h.state().browser.revision();
    act(&mut h, BrowseAct::Focus(1));
    assert_ne!(h.state().browser.revision(), r1);
}
