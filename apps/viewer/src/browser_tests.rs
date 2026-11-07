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

/// An NKS preset loads as its plug-in, with its state. No window is drawn here, so no
/// audio or MIDI device is opened.
#[test]
fn nks_presets_load_as_their_plugin() {
    use deckard_plugins::{nks, scan};
    let (_tmp, dir) = temp();
    let mut app = App::default();
    app.new_project();
    let entry = |name: &str, instrument: bool, cid: &str| scan::Entry {
        kind: scan::Kind::Vst3,
        vendor: "Vendor".into(),
        name: name.into(),
        instrument,
        location: String::new(),
        cid: cid.into(),
    };
    let synth = "000102030405060708090A0B0C0D0E0F";
    let effect = "101112131415161718191A1B1C1D1E1F";
    app.catalog.entries = vec![entry("Synth", true, synth), entry("Verb", false, effect)];
    let word = |at: usize, hex: &str| u32::from_str_radix(&hex[at * 8..(at + 1) * 8], 16).unwrap();
    let uid = |hex: &str| Some([word(0, hex), word(1, hex), word(2, hex), word(3, hex)]);
    let item = |file: &str, kind: Kind, bytes: &[u8]| {
        let path = dir.join(file);
        std::fs::write(&path, bytes).unwrap();
        deckard_library::Item { kind, name: file.into(), path, ..Default::default() }
    };
    let plugins = |app: &App| -> Vec<(String, String)> {
        let doc = app.doc.clone().unwrap();
        let d = doc.borrow();
        d.project().groups[0].sounds[0].slots.iter().filter_map(|x| x.plugin.as_ref().map(|p| (p.name.clone(), p.id.clone()))).collect()
    };

    let lead = item("Lead.nksf", Kind::Instrument, &nks::fixture(uid(synth), 0, "Synth", b"first"));
    app.load_item(&lead, Some((0, 0))).unwrap();
    assert_eq!(plugins(&app), [("Synth".to_string(), format!("Vendor:VST3:INSTR:{synth}"))]);
    assert_eq!((app.slot_of, app.slot, app.plugin_page), (Some(deckard_doc::Chain::Sound(0, 0)), Some(0), 0));
    assert!(app.plugin_view.is_none());
    // An effect goes last, and an instrument takes the instrument's place.
    let hall = item("Hall.nksfx", Kind::Effect, &nks::fixture(uid(effect), 0, "Verb", b"hall"));
    app.load_item(&hall, Some((0, 0))).unwrap();
    assert_eq!(app.slot, Some(1));
    let nica = vec![
        vec![
            nks::Knob { id: Some(0), name: "Spread".into(), section: "Performance".into() },
            nks::Knob { id: Some(1), name: "Cutoff".into(), section: String::new() },
        ],
        vec![
            nks::Knob { id: Some(10), name: "Drive".into(), section: "Filter 1".into() },
            nks::Knob { id: Some(11), name: "Reso".into(), section: "Filter 2".into() },
        ],
    ];
    let bass = item("Bass.nksf", Kind::Instrument, &nks::fixture_with_pages(uid(synth), 0, "Synth", b"second", &nica));
    app.load_item(&bass, Some((0, 0))).unwrap();
    assert_eq!(plugins(&app).iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["Synth", "Verb"]);
    {
        let doc = app.doc.clone().unwrap();
        let d = doc.borrow();
        let pr = d.project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap();
        assert!(pr.chunk.windows(6).any(|w| w == b"second"), "the second preset's state");
        assert_eq!(pr.pages.len(), 2, "NICA pages saved with the plug-in");
        assert_eq!(
            (pr.pages[0].params[0].id, pr.pages[0].params[0].name.as_str(), pr.pages[0].params[0].section.as_str()),
            (0, "Spread", "Performance")
        );
        assert_eq!(crate::saved_page_name(&pr.pages[0]), "Performance");
        assert_eq!(crate::saved_page_name(&pr.pages[1]), "Filter 1 / Filter 2");
    }

    // What can't load says why, and changes nothing.
    let before = plugins(&app);
    let absent = item("Absent.nksf", Kind::Instrument, &nks::fixture(Some([1, 2, 3, 4]), 0, "Omnisphere", b"x"));
    assert_eq!(app.load_item(&absent, Some((0, 0))), Err("Absent.nksf: Omnisphere isn't installed as VST3".to_string()));
    let massive = item("Bass.nmsv", Kind::Instrument, b"\x01\0\0\0");
    assert_eq!(app.load_item(&massive, Some((0, 0))), Err("Bass.nmsv: Massive isn't installed as VST3".to_string()));
    let unknown = item("Old.fxp", Kind::Instrument, b"\x01\0\0\0");
    assert_eq!(app.load_item(&unknown, Some((0, 0))), Err("Old.fxp: .fxp presets can't be loaded yet".to_string()));
    assert_eq!(plugins(&app), before);

    // Native product presets (.nmsv, .nfm8, .nrkt, .rkplr, .ens) load into their VST3.
    app.catalog.entries.push(entry("Massive", true, "202122232425262728292A2B2C2D2E2F"));
    app.load_item(&massive, Some((0, 0))).unwrap();
    assert_eq!(plugins(&app)[0].0, "Massive");
}

#[test]
fn unnamed_plugin_pages_derive_category_names_from_knobs() {
    let info = |id: u32, title: &str| deckard_plugins::ParamInfo {
        id,
        title: title.into(),
        short: title.into(),
        units: String::new(),
        steps: 0,
        default: 0.5,
    };
    let page = |labels: &[&str]| crate::ViewPage {
        name: String::new(),
        knobs: labels.iter().enumerate().map(|(i, l)| (info(i as u32, l), (*l).into(), String::new())).collect(),
        choices: Vec::new(),
        own: false,
    };
    let mut pages = vec![
        page(&["Macro 1", "Macro 2", "Macro 3", "Macro 4", "Engine 1 Vol", "Engine 2 Vol"]),
        page(&["Filter 1 Cutoff", "Filter 1 Resonance", "Filter 1 Drive", "Filter 2 Cutoff", "Filter 2 Resonance"]),
        page(&["Pitch Tracking - Enable", "Pitch Tracking - Smooth", "Pitch Shift - Enable", "Pitch Shift - Shift"]),
    ];
    crate::name_unnamed_pages(&mut pages);
    assert_eq!(pages[0].name, "Macros / Engine");
    assert_eq!(pages[1].name, "Filter 1 / Filter 2");
    assert_eq!(pages[1].knobs[0].2, "Filter 1");
    assert_eq!(pages[1].knobs[3].2, "Filter 2");
    assert_eq!(pages[2].name, "Pitch Tracking / Pitch Shift");
}

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

/// Each result has a heart: a click adds the favorite, another takes it off, with no
/// right-click menu.
#[test]
fn a_rows_heart_adds_and_removes_a_favorite() {
    use egui_kittest::kittest::Queryable;
    let (_tmp, dir) = temp();
    let Some(mut h) = open_browser(&dir) else { return };
    act(&mut h, BrowseAct::Kind(Kind::Loop));
    let favorites = |h: &Harness<'static, App>| h.state().browser.results.items.iter().filter(|i| i.favorite).count();
    assert_eq!(favorites(&h), 0);
    h.query_all_by_label("Add to Favorites").next().expect("a heart on each row").click();
    h.run_steps(2);
    h.state_mut().browser.wait();
    h.run_steps(2);
    assert_eq!(favorites(&h), 1);
    assert_eq!(h.state().browser.selected, None, "the click didn't select the row");
    h.query_by_label("Remove from Favorites").expect("the favorite's heart").click();
    h.run_steps(2);
    h.state_mut().browser.wait();
    h.run_steps(2);
    assert_eq!(favorites(&h), 0);
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

/// On DECKARD's MK3 surface, BROWSE loads via both display button 8 (LOAD) and the 4-D
/// encoder press (even after moving the wheel bracket to a filter), replacing an
/// existing Sampler sample with a full-range one-shot and focusing the loaded slot.
#[test]
fn mk3_surface_browse_load_button_and_wheel_load_one_shots() {
    use deckard_mk3::Event;
    let (_tmp, dir) = temp();
    let mut app = App::default();
    app.new_project();
    let (hit, clap) = (dir.join("Maschine 3/Samples/Hit.wav"), dir.join("Maschine 3/Samples/Clap.wav"));
    std::fs::create_dir_all(hit.parent().unwrap()).unwrap();
    wav(&hit, 4410);
    wav(&clap, 4410);
    let ni = deckard_library::testing::fixture(&dir);
    app.browser.start(Some(ni), dir.join("library.db3"), Vec::new());
    app.browser.add_saved(&hit);
    app.browser.add_saved(&clap);
    app.browse(BrowseAct::User(true));
    app.browse(BrowseAct::Kind(Kind::OneShot));
    app.browser.wait();
    let list = |app: &App| -> Vec<String> { app.browser.results.items.iter().map(|i| i.name.clone()).collect() };
    let hit_i = list(&app).iter().position(|n| n == "Hit").unwrap();
    let clap_i = list(&app).iter().position(|n| n == "Clap").unwrap();
    let (g, s) = (0, 0);
    app.group = g;
    app.sound = s;
    let mut reqs = Vec::new();
    app.mk3_events(vec![Event::Button("BUTTON_BROWSE", true), Event::Button("BUTTON_BROWSE", false)], &mut reqs);
    assert_eq!(app.surface.page(), "browse");

    // Display button 8 (LOAD) sets up a Sampler with a single full-range zone and focuses slot 0.
    app.browse(BrowseAct::Focus(hit_i));
    app.mk3_events(vec![Event::Button("BUTTON_SCREEN_8", true), Event::Button("BUTTON_SCREEN_8", false)], &mut reqs);
    {
        let doc = app.doc.clone().unwrap();
        let d = doc.borrow();
        let snd = &d.project().groups[g].sounds[s];
        let zones = &snd.sampler.as_ref().expect("a Sampler").zones;
        assert_eq!(zones.len(), 1);
        assert!(zones[0].sample.path.ends_with("Hit.wav"));
        assert_eq!(zones[0].key_range, (0, 127));
        assert_eq!(snd.name, "Hit");
    }
    assert_eq!(app.slot, Some(0));

    // Move the wheel bracket left onto a filter, then press the 4-D encoder (BUTTON_WHEEL):
    // it still loads the focused result into the sound, replacing the previous one-shot.
    app.browse(BrowseAct::Focus(clap_i));
    app.mk3_events(
        vec![
            Event::Button("BUTTON_WHEEL_LEFT", true),
            Event::Button("BUTTON_WHEEL_LEFT", false),
            Event::Button("BUTTON_WHEEL", true),
            Event::Button("BUTTON_WHEEL", false),
        ],
        &mut reqs,
    );
    let doc = app.doc.clone().unwrap();
    let d = doc.borrow();
    let snd = &d.project().groups[g].sounds[s];
    let zones = &snd.sampler.as_ref().expect("a Sampler").zones;
    assert_eq!(zones.len(), 1);
    assert!(zones[0].sample.path.ends_with("Clap.wav"));
    assert_eq!(snd.name, "Clap");
}

#[test]
fn many_types_and_characters_keep_results_on_screen() {
    use egui_kittest::kittest::Queryable;
    let mut app = App::default();
    app.browser.open = true;
    app.browser.results.types = (0..60).map(|i| (format!("Type {i:02}"), 10)).collect();
    app.browser.results.characters = (0..60).map(|i| (format!("Character {i:02}"), 5)).collect();
    app.browser.results.items = (0..20)
        .map(|i| deckard_library::Item {
            kind: Kind::Instrument,
            name: format!("Preset {i:02}"),
            path: PathBuf::from(format!("preset-{i:02}.mxinst")),
            ..Default::default()
        })
        .collect();
    let height = 720.0;
    let mut h = Harness::builder()
        .with_size(egui::vec2(600.0, height))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut App| layout::browser::panel(app, ui), app);
    h.run_steps(3);
    let count_rect = h.get_by_label("20 results").rect();
    let prehear_rect = h.get_by_label("▶ Prehear").rect();
    assert!(
        count_rect.max.y + 120.0 < prehear_rect.min.y,
        "results list has vertical room: count={count_rect:?}, prehear={prehear_rect:?}"
    );
    assert!(prehear_rect.max.y <= height, "prehear row stays inside the window: {prehear_rect:?}");
}
