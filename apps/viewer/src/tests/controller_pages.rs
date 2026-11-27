use super::*;

#[test]
fn song_view_section_click_sets_the_start_and_queues_a_jump() {
    let Some(mut h) = harness(false, true) else { return };
    if h.state().doc.as_ref().unwrap().borrow().project().sections.len() < 4 {
        return;
    }
    let pos = section_header(&h, 2);
    click(&mut h, pos);
    assert_eq!(h.state().section, Some(2));
    if !play(&mut h) {
        return;
    }
    let start = h.state().doc.as_ref().unwrap().borrow().project().section_start(2) as f64;
    let head = h.state().playhead().unwrap();
    assert!((head.position - start).abs() < 960.0, "plays from the picked section ({} vs {start})", head.position);
    let pos = section_header(&h, 3);
    click(&mut h, pos);
    assert_eq!(h.state().section, Some(3));
    assert_eq!(h.state().queued_section(), Some(3), "jumps when section 3 (index 2) ends");
    h.state_mut().stop();
}

/// RESTART (the header's or the controller's) plays the song from its first bar,
/// even with a later section picked; Loop switches in both and carries over.
#[test]
fn restart_plays_the_song_from_the_beginning_and_loop_carries_over() {
    let Some(mut h) = scripted_harness(true) else { return };
    let later = h.state().doc.as_ref().unwrap().borrow().project().sections.get(2).map(|_| ());
    if later.is_none() {
        return;
    }
    let start = h.state().doc.as_ref().unwrap().borrow().project().section_start(2) as f64;
    h.state_mut().section = Some(2);
    layout::apply_acts(h.state_mut(), vec![layout::Act::Restart]);
    h.run_steps(2);
    if !matches!(h.state().output, Some(Ok(_))) {
        return;
    }
    let head = h.state().playhead().unwrap();
    assert_eq!(head.mode, Mode::Arrangement);
    assert!(head.position < start / 2.0, "from the beginning ({} vs section 3 at {start})", head.position);

    let hw = |h: &mut Harness<'static, App>, button: &str, down: Option<bool>| {
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            let _ = match down {
                Some(d) => c.host.button(button, d),
                None => c.host.press(button),
            };
        }
        h.run_steps(4);
    };
    let seek = h.state().doc.as_ref().unwrap().borrow().project().section_start(2);
    if let Some(Ok(out)) = &h.state().output {
        out.seek(seek);
    }
    h.run_steps(2);
    hw(&mut h, "BUTTON_TRANSPORT_LOOP", None);
    let head = h.state().playhead().unwrap();
    assert!(head.position < start / 2.0, "the controller's RESTART too ({})", head.position);

    assert!(!h.state().looping);
    layout::apply_acts(h.state_mut(), vec![layout::Act::ToggleLoop]);
    assert!(h.state().looping);
    let scripts_loop = |h: &Harness<'static, App>| match &h.state().controller {
        Some(Ok(c)) => c.host.state.borrow().transport.looping,
        _ => false,
    };
    assert!(scripts_loop(&h), "the controller sees Loop on");
    hw(&mut h, "BUTTON_SHIFT", Some(true));
    hw(&mut h, "BUTTON_TRANSPORT_LOOP", None);
    hw(&mut h, "BUTTON_SHIFT", Some(false));
    assert!(!h.state().looping, "SHIFT + RESTART switches Loop off in the app");
    h.state_mut().stop();
}

/// The controller's Song view edits the arrangement's sections (C1).
#[test]
fn the_surface_sets_the_pad_pages_settings() {
    use deckard_surface::{Action as A, ColorOf};
    let Some(mut h) = harness(false, true) else { return };
    let app = h.state_mut();
    let p = project_of_app(app);
    if p.groups.is_empty() || p.groups[0].sounds.is_empty() {
        return;
    }
    (app.group, app.sound) = (0, 0);
    app.surface_act(A::VelocityValue(77));
    app.surface_act(A::Scale { root: 62, bank: 0, kind: 2 });
    app.surface_act(A::PadTouch("curve", 30));
    app.surface_act(A::PadTouch("mode", 3));
    app.surface_act(A::Color { of: ColorOf::Sound, color: Some(5) });
    app.surface_act(A::Color { of: ColorOf::Group, color: None });
    let v = app.surface_view();
    assert_eq!((v.velocity, v.scale_bank, v.scale_kind), (77, 0, 2));
    assert_eq!(app.scale, (62, 0, 2), "the pattern editor's scale too");
    let snd = &v.groups[0].sounds[0];
    assert_eq!((snd.touch_mode, snd.touch_curve), (3, 30));
    assert_eq!((snd.color, snd.default_color), (5, false));
    assert!(v.groups[0].default_color);
    app.user_chords = user_chords::UserChords::default();
    app.surface_act(A::UserChordNote { set: 1, slot: 3, note: 7 });
    app.surface_act(A::UserChordNote { set: 1, slot: 3, note: 0 });
    assert_eq!(app.surface_view().user_chords[1][3], [0, 7]);
    app.surface_act(A::UserChordMove { set: 1, slot: 3, next: false });
    app.surface_act(A::UserChordDelete { set: 1, slot: 3 });
    assert_eq!(app.surface_view().user_chords[1][2], [0, 7]);
}

#[test]
fn the_file_page_runs_the_file_commands() {
    use deckard_surface::Action as A;
    let Some(mut h) = harness(false, true) else { return };
    let app = h.state_mut();
    app.recent = vec![PathBuf::from("/tmp/Song A.mxprj"), PathBuf::from("/tmp/Song B.mxprj")];
    assert_eq!(app.surface_view().recent, ["Song A", "Song B"]);
    app.menu_commands.clear();
    for what in ["new", "save", "save_copy", "export_audio"] {
        app.surface_act(A::File(what, 0));
    }
    app.surface_act(A::File("load", 1));
    assert_eq!(
        app.menu_commands,
        [
            menu::Command::New,
            menu::Command::Save,
            menu::Command::SaveCopyAs,
            menu::Command::ExportAudio,
            menu::Command::OpenRecent(PathBuf::from("/tmp/Song B.mxprj"))
        ]
    );
    app.menu_commands.clear();
    app.surface_act(A::File("save_group", 0));
    assert!(app.save_dialog.is_some(), "a group not saved before: the Save dialog");
}

#[test]
fn the_surface_settings_page_steps_the_settings() {
    use deckard_surface::Action as A;
    let Some(mut h) = harness(false, true) else { return };
    let app = h.state_mut();
    app.settings.count_in_bars = 1;
    app.settings.link = false;
    for (name, step) in [
        ("count_in", 1),
        ("count_in", 1),
        ("count_in", 1),
        ("quantize", 1),
        ("numerator", -1),
        ("denominator", 1),
        ("midi_sync", 1),
        ("midi_sync", 1),
        ("clock_offset", -1),
        ("default_input", 1),
        ("link_start_stop", 1),
        ("sensitivity", -1),
        ("display_brightness", -1),
    ] {
        app.surface_act(A::Setting(name, 0, step));
    }
    let s = &app.settings;
    assert_eq!(s.count_in_bars, 4, "Off, 1, 2, 4 bars: no further");
    assert!(s.input_quantize && s.link_start_stop && !s.sound_midi_focus);
    assert_eq!(s.midi_sync, settings::MidiSync::Receive);
    assert_eq!(s.clock_receive_offset_ms, -1);
    assert_eq!((s.pad_sensitivity, s.display_brightness), (49, 99));
    let p = app.doc.as_ref().unwrap().borrow().project().clone();
    assert_eq!((p.numerator, p.denominator), (3, 8));
    let v = app.settings_view();
    for (key, shown) in
        [("count_in", "4 Bars"), ("quantize", "On"), ("midi_sync", "Receive"), ("clock_offset", "-1"), ("default_input", "None")]
    {
        assert_eq!(v.values.get(key).map(String::as_str), Some(shown), "{key}");
    }
    assert!(v.audio_status[0].starts_with("Device Status: "));
}

#[test]
fn the_surface_sets_the_grids_and_follow() {
    use deckard_surface::Action as A;
    let Some(mut h) = harness(false, true) else { return };
    let app = h.state_mut();
    app.follow = true;
    app.surface_act(A::Follow(false));
    assert!(!app.follow);
    for (name, value) in [("perform", 2), ("arrange", 3), ("arrange_snap", 0), ("quick", 0), ("step_snap", 0), ("nudge", 3)] {
        app.surface_act(A::Grid(name, value));
    }
    app.surface_act(A::Grid("step", 11));
    let g = app.surface_view().grid;
    assert_eq!((g.perform, g.arrange, g.arrange_snap, g.quick), (2, 3, false, false));
    assert_eq!((g.step, g.step_snap, g.nudge), (11, false, 3));
    assert_eq!(app.grid, PPQ / 3, "the pattern editor's grid: 1/8T");
    assert_eq!(app.nudge.0, 3);
}

#[test]
fn the_surface_edits_sections() {
    use deckard_surface::Action as A;
    let Some(mut h) = harness(false, true) else { return };
    let app = h.state_mut();
    let p = project_of_app(app);
    if p.sections.len() < 2 || p.scenes.len() < 2 {
        return;
    }
    let v = app.surface_view();
    assert!(v.song_view);
    assert_eq!(v.sections.len(), p.sections.len());
    assert_eq!(v.section, Some(0), "the first, until one's picked");
    assert_eq!(v.sections[1].start, p.sections[0].length);

    app.surface_act(A::SectionEdit("create"));
    let q = project_of_app(app);
    assert_eq!(q.sections.len(), p.sections.len() + 1);
    assert_eq!(q.sections[1].scene, p.sections[0].scene, "after the focused, playing its scene");
    assert_eq!(app.section, Some(1));

    app.surface_act(A::SectionMove(false));
    assert_eq!(app.section, Some(0));
    app.surface_act(A::SectionLength(3 * 3840));
    let q = project_of_app(app);
    assert_eq!((q.sections[0].length, q.sections[0].auto), (3 * 3840, false));
    app.surface_act(A::SectionEdit("delete"));
    assert_eq!(project_of_app(app).sections.len(), p.sections.len());

    app.surface_act(A::SongView(false));
    assert!(!app.surface_view().song_view);
}

/// DECKARD's own MK3 layer: its actions go through the app's own handlers, and its
/// view is the app's.
#[test]
fn the_surface_plays_and_focuses() {
    use deckard_surface::Action as A;
    let Some(mut h) = harness(false, false) else { return };
    let app = h.state_mut();
    assert!(app.controller.is_none(), "no scripts: DECKARD's layer runs");
    let v = app.surface_view();
    let p = project_of_app(app);
    assert_eq!(v.groups.len(), p.groups.len());
    assert_eq!(v.groups[0].sounds.len(), p.groups[0].sounds.len());
    assert_eq!(v.tempo, p.tempo);

    app.surface_act(A::Tempo(133.0));
    assert_eq!(project_of_app(app).tempo, 133.0);
    app.surface_act(A::Volume(0.5));
    assert_eq!(project_of_app(app).master_level, 0.5);
    app.surface_act(A::Swing(25.0));
    assert_eq!(project_of_app(app).swing.amount, 25.0);
    let muted = project_of_app(app).groups[0].sounds[1].muted;
    app.surface_act(A::MuteSound(1));
    assert_eq!(project_of_app(app).groups[0].sounds[1].muted, !muted);
    app.surface_act(A::FocusSound(2));
    assert_eq!(app.surface_view().sound, 2);
    for on in [false, true] {
        app.surface_act(A::FixedVelocity(on));
        assert_eq!(app.surface_view().fixed_velocity, on);
    }
    if p.groups.len() > 1 {
        app.surface_act(A::FocusGroup(1));
        assert_eq!(app.group, 1);
        app.surface_act(A::FocusGroup(0));
    }

    // ERASE + pad: the sound's notes leave the focused pattern.
    let pat = project_of_app(app).groups[0].patterns[app.pattern].clone();
    if let Some(s) = pat.notes.first().map(|n| n.sound) {
        app.surface_act(A::EraseSound(s));
        let left = &project_of_app(app).groups[0].patterns[app.pattern].notes;
        assert!(left.iter().all(|n| n.sound != s), "sound {s}'s notes erased");
        assert_eq!(left.len(), pat.notes.iter().filter(|n| n.sound != s).count(), "the others kept");
    }

    app.surface_act(A::Play);
    assert!(app.surface_view().playing);
    app.surface_act(A::Stop);
    assert!(!app.surface_view().playing);

    // The shipped scripts see this project: its screens draw without an error.
    let v = app.surface_view();
    assert!(app.surface.screens(&v).is_some());
    assert_eq!(app.surface.error(), None);
}

/// Preferences ▸ Hardware ▸ Controller ▸ DECKARD stops the scripts.
#[test]
fn choosing_deckards_layer_stops_the_scripts() {
    let Some(mut h) = harness(true, false) else { return };
    if !matches!(h.state().controller, Some(Ok(_))) {
        return;
    }
    let app = h.state_mut();
    app.set_controller_choice(settings::ControllerChoice::Deckard);
    assert!(app.controller.is_none());
    assert!(app.controller_scripts().is_none());
    app.set_controller_choice(settings::ControllerChoice::Automatic);
    assert!(matches!(app.controller, Some(Ok(_))), "the scripts again");
}

#[test]
fn theme_names_read_well() {
    assert_eq!(theme_name("solarized_dark"), "Solarized Dark");
    assert_eq!(theme_name("maschine"), "Maschine");
    assert_eq!(theme_name("lcd"), "LCD");
}

#[test]
fn opening_the_controller_folder_copies_the_stock_themes() {
    let dir = controller_folder().unwrap();
    let nord = std::fs::read_to_string(dir.join("defaults/themes/nord.lua")).unwrap();
    assert!(nord.contains("base = \"maschine\""));
    let readme = std::fs::read_to_string(dir.join("README.txt")).unwrap();
    assert!(readme.contains("import or export"));
}

/// TAP on the controller sets the song's tempo from the taps; SHIFT + TAP and the
/// header's switch turn the metronome on and off in both.
#[test]
fn tap_tempo_and_the_metronome() {
    let Some(mut h) = scripted_harness(false) else { return };
    if let Some(Ok(c)) = &mut h.state_mut().controller {
        for k in 0..3 {
            if k > 0 {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            let _ = c.host.press("BUTTON_TAP");
        }
    }
    h.run_steps(4);
    let tempo = h.state().doc.as_ref().unwrap().borrow().project().tempo;
    assert!((tempo - 150.0).abs() < 8.0, "taps 0.4 s apart: about 150 BPM ({tempo})");

    assert!(!h.state().metronome);
    let hw = |h: &mut Harness<'static, App>, button: &str, down: bool| {
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            let _ = c.host.button(button, down);
        }
        h.run_steps(4);
    };
    hw(&mut h, "BUTTON_SHIFT", true);
    hw(&mut h, "BUTTON_TAP", true);
    hw(&mut h, "BUTTON_TAP", false);
    hw(&mut h, "BUTTON_SHIFT", false);
    assert!(h.state().metronome, "SHIFT + TAP: the metronome on");
    layout::apply_acts(h.state_mut(), vec![layout::Act::ToggleMetronome]);
    assert!(!h.state().metronome);
    let scripts = match &h.state().controller {
        Some(Ok(c)) => c.host.state.borrow().transport.metronome,
        _ => true,
    };
    assert!(!scripts, "the controller sees it off");
}

/// A pad hit on the controller focuses its sound in the app too, so the Control
/// area (and the controller's Control page) shows that sound's chain: nothing for
/// an empty sound, not the last one shown.
#[test]
fn controller_pads_focus_their_sound_and_its_chain() {
    let Some(mut h) = scripted_harness(false) else { return };
    h.state_mut().chain_tab = layout::ChainTab::Sound;
    let sounds: Vec<usize> = {
        let d = h.state().doc.as_ref().unwrap().borrow();
        d.project().groups[0].sounds.iter().map(|s| s.slots.len()).collect()
    };
    let (Some(full), Some(empty)) = (sounds.iter().position(|&n| n > 0), sounds.iter().position(|&n| n == 0)) else { return };
    let hit = |h: &mut Harness<'static, App>, s: usize| {
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            c.host.pad_event(s as i64 + 1, true, 0.8);
            c.host.pad_event(s as i64 + 1, false, 0.0);
        }
        h.run_steps(4);
    };
    let shown = |h: &Harness<'static, App>| match &h.state().controller {
        Some(Ok(c)) => c.host.state.borrow().control.slots.len(),
        _ => 0,
    };
    hit(&mut h, full);
    assert_eq!(h.state().sound, full);
    assert_eq!(shown(&h), sounds[full], "the controller shows the sound's chain");
    hit(&mut h, empty);
    assert_eq!(h.state().sound, empty, "the app follows the pad");
    assert_eq!(shown(&h), 0, "an empty sound shows no chain");

    // And a sound picked in the app is the controller's focused sound.
    h.state_mut().sound = full;
    h.state_mut().sync_controller();
    h.run_steps(4);
    let focused = match &h.state().controller {
        Some(Ok(c)) => c.host.state.borrow().focus.sound,
        _ => full,
    };
    assert_eq!(focused, full);
}

/// WAVs dragged from the desktop onto a sound row (lit while over it): on an empty
/// sound, an Audio plug-in playing it in time (its tempo from its name), on in the
/// focused pattern, which holds it all; on a sound with a Sampler, a zone more of it.
/// One undo step each. Two files on empty sounds: a loop each.
#[test]
fn samples_dropped_on_sounds_load_as_loops_or_zones() {
    let Some(mut h) = harness(false, false) else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let wav = |name: &str, frames: usize| {
        let path = dir.join(name);
        let spec = hound::WavSpec { channels: 2, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..frames {
            let v = ((i as f32 * 0.05).sin() * 8000.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        path
    };
    // 8 beats at 94 BPM; a one-shot.
    let (lp, hit, a, b) =
        (wav("Groove[94].wav", 225_186), wav("Hit.wav", 4410), wav("Pad A[120].wav", 88_200), wav("Pad B[120].wav", 88_200));
    let row = |h: &Harness<'static, App>, s: usize| -> egui::Rect {
        let rows: Vec<(egui::Rect, (usize, usize))> = h.ctx.data(|d| d.get_temp(egui::Id::new(layout::DROP_ROWS))).expect("sound rows");
        rows.iter().find(|(_, gs)| *gs == (h.state().group, s)).expect("the sound's row").0
    };
    let drop = |h: &mut Harness<'static, App>, s: usize, files: &[&PathBuf]| {
        let at = row(h, s).center();
        h.hover_at(at);
        h.input_mut().hovered_files = files.iter().map(|f| egui::HoveredFile { path: Some((*f).clone()), ..Default::default() }).collect();
        h.step();
        assert!(h.state().file_drag.is_some_and(|p| row(h, s).contains(p)), "the row is lit");
        h.input_mut().hovered_files.clear();
        h.input_mut().dropped_files = files.iter().map(|f| std::sync::Arc::new(Dropped((*f).clone())) as egui::DroppedFileHandle).collect();
        h.step();
        h.input_mut().dropped_files.clear();
        h.step();
    };
    let g = h.state().group;
    let sound = |h: &Harness<'static, App>, s: usize| project_of(h).groups[g].sounds[s].clone();
    // The rows shown: an empty sound and one with a Sampler.
    let shown: Vec<usize> = (0..7).collect();
    let empty: Vec<usize> = shown.iter().copied().filter(|&s| sound(&h, s).slots.is_empty()).collect();
    let Some(&e) = empty.first() else { return };
    let undo_steps = h.state().doc.as_ref().unwrap().borrow().undo_labels().len();
    drop(&mut h, e, &[&lp]);
    let snd = sound(&h, e);
    let a_loop = snd.audio.clone().expect("an Audio plug-in");
    assert_eq!(snd.instrument, "AudioModule");
    assert_eq!(a_loop.takes.len(), 1);
    assert_eq!(Path::new(&a_loop.takes[0].sample.path), lp.as_path());
    assert!((a_loop.takes[0].beats - 8.0).abs() < 0.01, "8 beats at 94: {}", a_loop.takes[0].beats);
    let pi = h.state().pattern;
    let pat = project_of(&h).groups[g].patterns[pi].clone();
    assert!(pat.audio.iter().any(|c| c.sound == e && c.take == 0), "on in the focused pattern");
    assert!(pat.length >= 8 * PPQ, "the pattern holds it");
    assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_labels().len(), undo_steps + 1, "one undo step");
    // On a Sampler: one zone more.
    if let Some(&with) = shown.iter().find(|&&s| sound(&h, s).sampler.as_ref().is_some_and(|x| !x.zones.is_empty())) {
        let before = sound(&h, with).sampler.unwrap().zones.len();
        drop(&mut h, with, &[&hit]);
        let zones = sound(&h, with).sampler.unwrap().zones;
        assert_eq!(zones.len(), before + 1, "added to the Sampler");
        assert_eq!(Path::new(&zones[before].sample.path), hit.as_path());
        assert!(sound(&h, with).audio.is_none());
    }
    // Two on empty sounds: a loop each.
    if let Some(w) = empty.windows(2).find(|w| w[1] == w[0] + 1).map(|w| w[0]) {
        drop(&mut h, w, &[&a, &b]);
        let paths: Vec<String> = [w, w + 1].iter().map(|&s| sound(&h, s).audio.unwrap().takes[0].sample.path.clone()).collect();
        assert_eq!(paths, [a.display().to_string(), b.display().to_string()]);
    }
}

/// Finished stems become a new group, as Maschine makes it: named after the sample,
/// its first four sounds playing them (saved as "<name> Drums", …) — here Audio
/// plug-ins, in time with the song, on in its pattern, which plays in the scene — as
/// one undo step.
/// STEMS from the controller, with the model installed (`DECKARD_STEMS_MODEL`: its
/// folder): EDIT ▸ STEMS ▸ APPLY separates the focused zone's sample in the
/// background, the controller shows it running, and the stems arrive as a new group.
#[test]
fn controller_stems_separate_the_sample_into_a_new_group() {
    let Some(model) = std::env::var_os("DECKARD_STEMS_MODEL") else { return };
    let Some(mut h) = harness(true, false) else { return };
    let has_zone = project_of(&h).groups[0].sounds[0].sampler.as_ref().is_some_and(|s| !s.zones.is_empty());
    if !has_zone {
        return;
    }
    // The model where the app reads it (the tests' own settings folder), the stems
    // next to a temporary project.
    let settings = menu::settings_dir().unwrap();
    std::fs::create_dir_all(settings.join("models")).unwrap();
    std::fs::copy(Path::new(&model).join(deckard_stems::MODEL_FILE), settings.join("models").join(deckard_stems::MODEL_FILE)).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let groups = project_of(&h).groups.len();
    {
        let app = h.state_mut();
        app.path = Some(dir.join("x.mxprj"));
        (app.group, app.sound) = (0, 0);
        app.sync_controller();
    }
    h.run_steps(2);
    let press = |h: &mut Harness<'static, App>, b: &str| {
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            let _ = c.host.press(b);
            c.host.tick();
        }
        h.run_steps(2);
    };
    press(&mut h, "BUTTON_SAMPLE");
    press(&mut h, "BUTTON_SCREEN_2");
    press(&mut h, "BUTTON_SCREEN_5");
    press(&mut h, "BUTTON_SCREEN_5");
    press(&mut h, "BUTTON_SCREEN_8");
    assert!(h.state().stems.is_some(), "separating: {}", h.state().status);
    let running = match &h.state().controller {
        Some(Ok(c)) => c.host.state.borrow().stems.is_some_and(|r| r.focused),
        _ => false,
    };
    assert!(running, "the controller shows it");
    let t = std::time::Instant::now();
    while h.state().stems.is_some() && t.elapsed() < std::time::Duration::from_secs(300) {
        std::thread::sleep(std::time::Duration::from_millis(200));
        h.run_steps(1);
    }
    let p = project_of(&h);
    assert_eq!(p.groups.len(), groups + 1, "{}", h.state().status);
    let names: Vec<String> = p.groups[groups].sounds[..4].iter().map(|s| s.name.clone()).collect();
    assert!(names[0].ends_with(" Drums") && names[3].ends_with(" Vocal"), "{names:?}");
    let _ = std::fs::remove_dir_all(settings);
}

/// A Loop-mode Audio plug-in's pad (MK3 Pad mode) switches its loop off in the
/// pattern the group plays, and on again with its focused take.
#[test]
fn toggling_an_audio_loop_switches_it_in_the_playing_pattern() {
    let mut app = App::default();
    app.new_project();
    {
        let mut d = app.doc.as_ref().unwrap().borrow_mut();
        d.add_pattern(0, 0, "Part").unwrap();
        d.assign_pattern(0, 0, Some(0)).unwrap();
    }
    let tmp = tempfile::tempdir().unwrap();
    let wav = tmp.path().join("Loop 120 bpm.wav");
    {
        let spec = hound::WavSpec { channels: 1, sample_rate: 48_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..96_000 {
            w.write_sample(((i as f64 * 0.05).sin() * 8_000.0) as i16).unwrap();
        }
        w.finalize().unwrap();
    }
    let smp = deckard_engine::sample::Sample::load(&wav).unwrap();
    app.load_loop(0, 2, &wav, &smp).unwrap();
    let on = |app: &App| app.doc.as_ref().unwrap().borrow().project().groups[0].patterns[0].audio.iter().any(|c| c.sound == 2);
    let loop_playing = |app: &App| app.surface_view().groups[0].sounds[2].loop_playing;
    assert!(on(&app), "a loop loaded is on in the pattern");
    assert!(loop_playing(&app), "active in the pattern even while stopped");
    app.playing = Some(deckard_engine::Mode::Scene(0));
    assert!(loop_playing(&app), "song playing and on in the group's pattern");
    assert!(!app.surface_view().groups[0].sounds[3].loop_playing, "another sound");
    app.toggle_audio_loop(0, 2).unwrap();
    assert!(!on(&app), "the pad switches it off");
    assert!(!loop_playing(&app), "off in the pattern: no longer active");
    app.toggle_audio_loop(0, 2).unwrap();
    assert!(on(&app), "and on again");
    assert!(loop_playing(&app), "on again while song plays");
    let slot =
        app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[2].slots.iter().position(|x| x.module == "AudioModule").unwrap();
    layout::apply_acts(&mut app, vec![layout::Act::SetParam(deckard_doc::Target::Module(0, 2, slot), "mode", 1.0)]);
    assert!(!loop_playing(&app), "Gate mode plays with notes, not the pattern");
    assert!(app.toggle_audio_loop(0, 3).is_err(), "a sound without an Audio plug-in");

    // A looped sustaining Sampler sound stays flagged as `loop_playing` while active in the pattern or engine.
    {
        let mut d = app.doc.as_ref().unwrap().borrow_mut();
        d.add_slot(0, 3, 0, &deckard_doc::NewModule::Sampler).unwrap();
        let spec = deckard_doc::ZoneSpec {
            from: deckard_doc::ZoneFrom::Template,
            file: Some(wav.clone()),
            set: vec![
                ("key_range/low".into(), 0.0),
                ("key_range/high".into(), 127.0),
                ("velocity_range/low".into(), 0.0),
                ("velocity_range/high".into(), 127.0),
                ("root_key".into(), 60.0),
                ("play_range/low".into(), 0.0),
                ("play_range/high".into(), 95_999.0),
                ("loop_active".into(), 1.0),
                ("loop_range/low".into(), 0.0),
                ("loop_range/high".into(), 95_999.0),
            ],
        };
        d.set_zones(0, 3, &[spec], "Loop").unwrap();
        d.set("Sustaining", deckard_doc::Target::Sampler(0, 3), "envelope_type", mxprj::decode::Value::Int(2)).unwrap();
        let note = deckard_model::Note {
            ratchet: Default::default(),
            expr: None,
            sound: 3,
            position: 0,
            length: 3840,
            pitch: 60,
            velocity: 100,
            probability: 100,
        };
        d.edit_notes("Loop note", 0, 0, &[], &[note]).unwrap();
    }
    assert!(app.surface_view().groups[0].sounds[3].loop_playing, "looped Sampler note active in playing pattern");
    app.playing = None;
    assert!(!app.surface_view().groups[0].sounds[3].loop_playing, "stopped with no held note: Sampler loop inactive");
    app.held = Some((0, 3, 60));
    assert!(app.surface_view().groups[0].sounds[3].loop_playing, "held looped Sampler pad: active");
    app.held = None;
}

#[test]
fn stems_make_a_new_group_of_audio_loops() {
    let Some(mut h) = harness(false, false) else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let groups = project_of(&h).groups.len();
    let app = h.state_mut();
    app.path = Some(dir.join("x.mxprj"));
    let stems = deckard_stems::Kind::ALL
        .iter()
        .enumerate()
        .map(|(k, &kind)| deckard_stems::Stem { kind, frames: vec![[0.1 * (k + 1) as f32; 2]; 4410] })
        .collect();
    app.finish_stems("Song", 44_100, stems, Some(4.0));
    let p = project_of(&h);
    assert_eq!(p.groups.len(), groups + 1, "{}", h.state().status);
    let g = &p.groups[groups];
    assert_eq!(g.name, "Song");
    let names: Vec<&str> = g.sounds[..4].iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Song Drums", "Song Bass", "Song Other", "Song Vocal"]);
    for (k, s) in g.sounds[..4].iter().enumerate() {
        let a = s.audio.as_ref().expect("an Audio plug-in");
        assert_eq!(a.takes.len(), 1);
        assert_eq!(a.takes[0].beats, 4.0, "the source's beats");
        assert!(std::path::Path::new(&a.takes[0].sample.path).exists(), "{}", a.takes[0].sample.path);
        assert!(g.patterns[0].audio.iter().any(|c| c.sound == k), "on in its pattern");
    }
    assert!(p.scenes[h.state().scene].patterns.iter().any(|&(sg, _)| sg == groups), "playing in the scene");
    assert_eq!(h.state().group, groups, "the new group in focus");
    h.state_mut().undo();
    assert_eq!(project_of(&h).groups.len(), groups, "one undo step");
}

#[test]
fn auto_loop_finds_a_loop_in_the_play_range() {
    let Some(mut h) = harness(false, false) else { return };
    let has_zone = project_of(&h).groups[0].sounds[0].sampler.as_ref().is_some_and(|s| !s.zones.is_empty());
    if !has_zone {
        return;
    }
    let rate = 44_100;
    let tone = deckard_engine::sample::Sample {
        rate,
        frames: (0..rate as usize).map(|i| [(i as f32 * 220.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5; 2]).collect(),
    };
    let app = h.state_mut();
    (app.group, app.sound, app.zone) = (0, 0, 0);
    app.sampling.sample = Some(std::sync::Arc::new(tone));
    let t = deckard_doc::Target::Zone(0, 0, 0);
    layout::apply_acts(app, vec![layout::Act::SetParam(t, "play_range/low", 0.0), layout::Act::SetParam(t, "play_range/high", 44_099.0)]);
    layout::apply_acts(app, vec![layout::Act::SetParam(t, "loop_active", 0.0)]);
    app.sampling_act(layout::sampling::SamplingAct::Edit(layout::sampling::EditOp::AutoLoop));
    let z = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].clone();
    assert!(z.loop_active, "{}", h.state().status);
    assert!(z.loop_range.0 < z.loop_range.1 && z.loop_range.1 <= 44_100, "{:?}", z.loop_range);
    h.state_mut().undo();
    let z = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].clone();
    assert!(!z.loop_active, "one undo step");
}

#[test]
fn controller_sampling_tabs_show_and_edit_the_sample() {
    let Some(mut h) = scripted_harness(false) else { return };
    {
        let app = h.state_mut();
        (app.group, app.sound) = (0, 0);
        app.sync_controller();
    }
    h.run_steps(2);
    let has_zone = project_of(&h).groups[0].sounds[0].sampler.as_ref().is_some_and(|s| !s.zones.is_empty());
    if !has_zone {
        return;
    }
    let press = |h: &mut Harness<'static, App>, b: &str| {
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            let _ = c.host.press(b);
            c.host.tick();
        }
        h.run_steps(3);
    };
    press(&mut h, "BUTTON_SAMPLE");
    press(&mut h, "BUTTON_SCREEN_2");
    assert_eq!(h.state().sampling.tab, 1, "the app's Sampling view on EDIT too");
    h.run_steps(2);
    let frames = match &h.state().controller {
        Some(Ok(c)) => c.host.state.borrow().sample.as_ref().map(|s| (s.frames, s.overview.len())),
        _ => None,
    };
    let (frames, cols) = frames.expect("the controller has the sample");
    assert!(frames > 0 && cols == 480);
    let before = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].play_range.0;
    if let Some(Ok(c)) = &mut h.state_mut().controller {
        let _ = c.host.encoder("ENCODER_SCREEN_1", 0.05);
    }
    h.run_steps(2);
    let after = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].play_range.0;
    assert!(after > before, "knob 1 moved the start: {before} -> {after}");
    press(&mut h, "BUTTON_SCREEN_3");
    assert_eq!(h.state().sampling.tab, 2, "SLICE");
    h.run_steps(2);
    let slices = match &h.state().controller {
        Some(Ok(c)) => c.host.state.borrow().sample.as_ref().map_or(0, |s| s.slices.len()),
        _ => 0,
    };
    assert_eq!(slices, h.state().sampling.slices.len().max(1), "the app's slices");
    // The app's tab shows on the controller too.
    h.state_mut().sampling_act(layout::sampling::SamplingAct::Tab(3));
    h.run_steps(2);
    let hw_tab = match &h.state().controller {
        Some(Ok(c)) => c.host.state.borrow().sampling_tab,
        _ => -1,
    };
    assert_eq!(hw_tab, 3, "ZONE on the controller");
}

#[test]
fn controller_sample_button_shows_sampling_and_takes_make_a_sampler() {
    let Some(mut h) = scripted_harness(false) else { return };
    let effect = deckard_model::INTERNAL_EFFECTS[0].0;
    {
        let app = h.state_mut();
        let s = app.doc.as_ref().unwrap().borrow().project().groups[0].sounds.iter().position(|x| x.slots.is_empty());
        let Some(s) = s else { return };
        app.sound = s;
        app.sync_controller();
        app.doc.as_ref().unwrap().borrow_mut().add_slot(0, s, 0, &deckard_doc::NewModule::Effect(effect)).unwrap();
    }
    h.run_steps(2);
    let press = |h: &mut Harness<'static, App>| {
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            let _ = c.host.press("BUTTON_SAMPLE");
        }
        h.run_steps(4);
    };
    // The Sampling page's tabs (SLICE among them) show for a sound with a Sampler
    // (they hide for a sound without one).
    let tabs_shown = |h: &Harness<'static, App>| {
        use deckard_lua_host::state::Prop;
        let Some(Ok(c)) = &h.state().controller else { return false };
        let st = c.host.state.borrow();
        let all = &st.widgets.all;
        let shown = |i: usize| !matches!(all[i].props.get("Visible"), Some(Prop::Bool(false)));
        let Some(page) = all.iter().position(|w| w.name == "SamplingPageStudio") else { return false };
        // Down the page's shown widgets to a SLICE button.
        let mut todo = vec![page];
        while let Some(i) = todo.pop() {
            if matches!(all[i].props.get("Text").or(all[i].props.get("style")), Some(Prop::Str(t)) if t == "SLICE") {
                return true;
            }
            todo.extend(all[i].children.iter().copied().filter(|&c| shown(c)));
        }
        false
    };
    press(&mut h);
    let s = h.state().sound;
    let chain = |h: &Harness<'static, App>| -> Vec<String> {
        h.state().doc.as_ref().unwrap().borrow().project().groups[0].sounds[s].slots.iter().map(|x| x.module.clone()).collect()
    };
    assert_eq!(chain(&h), [effect], "SAMPLE keeps the chain");
    assert!(h.state().sampling_open, "the Sampling view shows");
    assert_eq!(h.state().sampling.tab, 0, "on RECORD, with nothing sampled yet");
    assert!(!tabs_shown(&h), "no Sampler: RECORD alone");
    // The controller's RECORD screen meters what the recorder hears.
    h.state_mut().recorder.view.meter = (0.5, 0.25);
    h.run_steps(1);
    // (The scripts' timer: they update the screen on their ticks.)
    if let Some(Ok(c)) = &mut h.state_mut().controller {
        for _ in 0..3 {
            c.host.tick();
        }
    }
    let levels = match &h.state().controller {
        Some(Ok(c)) => {
            let st = c.host.state.borrow();
            st.widgets.all.iter().find(|w| w.kind == "MasterLevelMeter").map(|w| {
                let n = |k: &str| match w.props.get(k) {
                    Some(deckard_lua_host::state::Prop::Num(n)) => *n,
                    _ => 0.0,
                };
                (n("Levels"), n("Levels2"))
            })
        }
        _ => None,
    };
    let (l, r) = levels.expect("the Sampling page's meter");
    assert!(l > 0.05 && r > 0.02 && l > r, "the meter shows the input ({l}, {r})");
    // The RECORD screen's knobs set the recorder: knob 1 the MODE (RECORDING page),
    // and after RIGHT the SOURCE (INPUT page).
    let turn = |h: &mut Harness<'static, App>, b: &str, inc: f64| {
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            let _ = c.host.encoder(b, inc);
        }
        h.run_steps(2);
    };
    assert_eq!(h.state().recorder.view.mode, 0);
    turn(&mut h, "ENCODER_SCREEN_1", 0.04);
    assert_eq!(h.state().recorder.view.mode, 1, "Sync");
    turn(&mut h, "ENCODER_SCREEN_1", -0.04);
    assert_eq!(h.state().recorder.view.mode, 0, "Detect again");
    if let Some(Ok(c)) = &mut h.state_mut().controller {
        let _ = c.host.press("BUTTON_RIGHT");
        // (The scripts' timer shows the page's knobs.)
        c.host.tick();
    }
    h.run_steps(2);
    let source = h.state().recorder.view.source;
    turn(&mut h, "ENCODER_SCREEN_1", if source == 2 { -0.04 } else { 0.04 });
    assert_ne!(h.state().recorder.view.source, source, "SOURCE");
    // START (button 5) arms the app's recorder (here recording what plays), STOP
    // ends it; START then CANCEL (button 6) throws it away. Without an audio output
    // (no device) there's nothing to record from, and the recorder says so.
    h.state_mut().recorder.view.source = 2;
    let screen_button = |h: &mut Harness<'static, App>, b: &str| {
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            let _ = c.host.press(b);
        }
        h.run_steps(2);
    };
    screen_button(&mut h, "BUTTON_SCREEN_5");
    if h.state().recorder.busy() {
        let waiting = matches!(&h.state().controller, Some(Ok(c)) if c.host.state.borrow().record.waiting);
        assert!(waiting, "the RECORD screen shows it waiting for sound");
        screen_button(&mut h, "BUTTON_SCREEN_5");
        assert!(!h.state().recorder.busy(), "STOP");
        screen_button(&mut h, "BUTTON_SCREEN_5");
        assert!(h.state().recorder.busy(), "START again");
        screen_button(&mut h, "BUTTON_SCREEN_6");
        assert!(!h.state().recorder.busy(), "CANCEL");
    } else {
        assert_eq!(h.state().recorder.view.error.as_deref(), Some("no audio output to record from"));
    }
    // A recorded take makes a sound without a Sampler one (its modules go); a sound
    // with a Sampler keeps its chain.
    let take = std::env::temp_dir().join("deckard-take-test.wav");
    h.state_mut().load_take(0, s, &take, 4800);
    assert_eq!(chain(&h), ["Sampler"], "the take replaced the chain");
    h.state().doc.as_ref().unwrap().borrow_mut().add_slot(0, s, 1, &deckard_doc::NewModule::Effect(effect)).unwrap();
    h.state_mut().load_take(0, s, &take, 9600);
    assert_eq!(chain(&h), ["Sampler", effect], "a Sampler's chain stays");
    h.run_steps(4);
    assert!(tabs_shown(&h), "the page's tabs show for a Sampler");
    press(&mut h);
    assert!(!h.state().sampling_open, "SAMPLE again: back to the pattern editor");
}

#[test]
fn controller_scene_button_and_pads_pick_scenes_in_the_ideas_view() {
    let Some(mut h) = scripted_harness(true) else { return };
    // Switch to the Ideas view with the strip button, as a user would.
    click(&mut h, Pos2::new(3.0 + 16.0, 33.0 + 3.0 + 6.0 + 12.0));
    assert!(!h.state().song_view);
    if !play(&mut h) {
        return;
    }
    press(&mut h, "SCENE");
    press(&mut h, "4");
    assert_eq!(h.state().scene, 3);
    let head = h.state().playhead().unwrap();
    assert_eq!(head.queued, Some(Mode::Scene(3)));
    press(&mut h, "SCENE");
    h.state_mut().stop();
}

/// Closing the window after the controller changed the project asks about saving
/// first, and doesn't close.
#[test]
fn closing_after_a_controller_edit_asks_to_save() {
    let Some(mut h) = scripted_harness(false) else { return };
    h.run_steps(2);
    assert!(!h.state().dirty(), "nothing changed yet");
    {
        let app = h.state_mut();
        let c = app.controller.as_mut().unwrap().as_mut().unwrap();
        for (b, down) in [("BUTTON_PATTERN", true), ("BUTTON_SCREEN_5", true), ("BUTTON_SCREEN_5", false), ("BUTTON_PATTERN", false)] {
            c.host.button(b, down).unwrap();
            c.host.tick();
        }
    }
    h.run_steps(2);
    assert!(h.state().dirty(), "CREATE changed the project");
    h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    h.run_steps(1);
    assert!(matches!(h.state().confirm, Some(Pending::Quit)), "asks about saving");
    assert!(!h.state().quitting);
}

/// The controller's Control page works the Control area: knob 1 turns the focused
/// effect's first knob, and screen buttons 5/6 move between slots.
#[test]
fn controller_knobs_and_screen_buttons_work_the_control_area() {
    let Some(mut h) = scripted_harness(true) else { return };
    h.state_mut().menu_commands.push(menu::Command::New);
    h.run_steps(3);
    if h.state().confirm.is_some() {
        h.get_by_label("Don't Save").click();
        h.run_steps(3);
    }
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
    h.state_mut().add_slot(layout::AddWhat::Instrument("BassSynth"));
    h.state_mut().add_slot(layout::AddWhat::Effect("Chorus"));
    h.run_steps(4);
    assert_eq!(h.state().slot, Some(1), "{}", h.state().status);
    let rate = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].slots[1].param("rate").unwrap();
    let before = rate(&h);
    {
        let app = h.state_mut();
        let c = app.controller.as_mut().unwrap().as_mut().unwrap();
        let mut req = Vec::new();
        c.encoder("ENCODER_SCREEN_1", 0.2, &mut req);
        let knobs = c.host.state.borrow().control.knobs.clone();
        assert_eq!(knobs[0].as_ref().map(|k| k.name.as_str()), Some("RATE"), "{knobs:?}");
    }
    h.run_steps(3);
    assert!((rate(&h) - (before + 0.2)).abs() < 1e-3, "knob 1 turned the Chorus's Rate: {before} → {}", rate(&h));
    // Screen button 5: the slot before (the Bass Synth).
    {
        let app = h.state_mut();
        let c = app.controller.as_mut().unwrap().as_mut().unwrap();
        c.press("BUTTON_SCREEN_5", &mut Vec::new());
    }
    h.run_steps(3);
    assert_eq!(h.state().slot, Some(0), "screen button 5 focused the previous slot");
    let knobs = h.state().controller.as_ref().unwrap().as_ref().unwrap().host.state.borrow().control.knobs.clone();
    assert_eq!(knobs[0].as_ref().map(|k| k.name.as_str()), Some("SHAPE"), "{knobs:?}");
}

/// PLUG-IN on an empty sound: the "+" is focused; the 4-D encoder opens the plug-in
/// browser, turns to a module and loads it (pressed). Then on to the "+" again, TYPE to
/// Effect, and an effect after it.
#[test]
fn controller_plugin_browser_loads_modules() {
    let Some(mut h) = scripted_harness(true) else { return };
    h.state_mut().menu_commands.push(menu::Command::New);
    h.run_steps(3);
    if h.state().confirm.is_some() {
        h.get_by_label("Don't Save").click();
        h.run_steps(3);
    }
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
    h.run_steps(3);
    let slots = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].slots.iter().map(|x| x.module.clone()).collect::<Vec<_>>();
    assert!(slots(&h).is_empty());
    let with = |h: &mut Harness<'static, App>, f: &dyn Fn(&mut controller::Controller)| {
        let c = h.state_mut().controller.as_mut().unwrap().as_mut().unwrap();
        f(c);
        h.run_steps(3);
    };
    with(&mut h, &|c| c.press("BUTTON_PLUGIN", &mut Vec::new()));
    let focus = h.state().controller.as_ref().unwrap().as_ref().unwrap().host.state.borrow().control.focus_slot;
    assert_eq!(focus, None, "the + is focused");
    with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
    // (none), Sampler, Audio, Kick: a step at a time.
    for _ in 0..3 {
        with(&mut h, &|c| c.wheel(1, &mut Vec::new()));
    }
    with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
    assert_eq!(slots(&h), ["Kick"], "{}", h.state().status);
    // The right display's logo follows: the Kick's, then (removed) none.
    let logo = |h: &mut Harness<'static, App>| {
        // The controller's next timers (every 50 ms of real time in the app) see the
        // new chain.
        for _ in 0..3 {
            h.state_mut().controller.as_mut().unwrap().as_mut().unwrap().host.tick();
            h.run_steps(1);
        }
        let st = h.state().controller.as_ref().unwrap().as_ref().unwrap().host.state.borrow();
        let bg = st.widgets.all.iter().find(|w| w.name == "PluginBG").expect("the logo's bar");
        match bg.props.get("PictureUrl") {
            Some(deckard_lua_host::state::Prop::Str(p)) => p.clone(),
            _ => String::new(),
        }
    };
    let shown = logo(&mut h);
    assert!(shown.to_lowercase().contains("kick"), "{shown}");
    with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
    for _ in 0..3 {
        with(&mut h, &|c| c.wheel(-1, &mut Vec::new()));
    }
    with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
    assert!(slots(&h).is_empty(), "(none) removed it");
    assert_eq!(logo(&mut h), "", "no logo left behind");
    with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
    for _ in 0..3 {
        with(&mut h, &|c| c.wheel(1, &mut Vec::new()));
    }
    with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
    assert_eq!(slots(&h), ["Kick"], "{}", h.state().status);
    let shown = logo(&mut h);
    assert!(shown.to_lowercase().contains("kick"), "{shown}");

    with(&mut h, &|c| c.press("BUTTON_WHEEL_RIGHT", &mut Vec::new()));
    assert_eq!(h.state().slot, None, "past the last slot: the +");
    with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
    with(&mut h, &|c| c.encoder("ENCODER_SCREEN_1", 0.3, &mut Vec::new()));
    with(&mut h, &|c| c.wheel(1, &mut Vec::new()));
    with(&mut h, &|c| c.press("BUTTON_WHEEL", &mut Vec::new()));
    assert_eq!(slots(&h), ["Kick", deckard_model::INTERNAL_EFFECTS[0].0], "{}", h.state().status);
}

/// The controller's buttons switch the app's views: KEYBOARD shows the pattern
/// editor as a piano roll and PAD MODE back as the drum grid; the Arranger page's
/// SHIFT + IDEAS / SONG switch the song panel, and LEFT / RIGHT there pick the
/// section the app shows.
/// The "+" after the groups adds an empty group at the end and focuses it.
#[test]
fn plus_after_the_groups_adds_one() {
    let Some(mut h) = harness(false, false) else { return };
    let n = h.state().doc.as_ref().unwrap().borrow().project().groups.len();
    layout::apply_acts(h.state_mut(), vec![layout::Act::AddGroup]);
    h.run_steps(2);
    let d = h.state().doc.as_ref().unwrap().clone();
    assert_eq!(d.borrow().project().groups.len(), n + 1);
    assert_eq!(h.state().group, n);
    assert_eq!(d.borrow().project().groups[n].sounds.len(), 16);
}

#[test]
fn controller_buttons_switch_the_apps_views() {
    let Some(mut h) = scripted_harness(false) else { return };
    let hw = |h: &mut Harness<'static, App>, button: &str, down: Option<bool>| {
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            match down {
                Some(d) => {
                    let _ = c.host.button(button, d);
                }
                None => {
                    let _ = c.host.press(button);
                }
            }
        }
        h.run_steps(4);
    };
    assert!(!h.state().piano_roll);
    hw(&mut h, "BUTTON_KEYBOARD", None);
    assert!(h.state().piano_roll, "Keyboard mode: a piano roll");
    hw(&mut h, "BUTTON_PAD_MODE", None);
    assert!(!h.state().piano_roll, "Pad mode: the drum grid");

    if h.state().doc.as_ref().unwrap().borrow().project().sections.len() < 2 {
        return;
    }
    assert!(!h.state().song_view);
    hw(&mut h, "BUTTON_ARRANGE", None);
    hw(&mut h, "BUTTON_SHIFT", Some(true));
    hw(&mut h, "BUTTON_SCREEN_2", None);
    hw(&mut h, "BUTTON_SHIFT", Some(false));
    assert!(h.state().song_view, "SONG: the Song view");
    hw(&mut h, "BUTTON_RIGHT", None);
    assert_eq!(h.state().section, Some(1));
    hw(&mut h, "BUTTON_SHIFT", Some(true));
    hw(&mut h, "BUTTON_SCREEN_1", None);
    hw(&mut h, "BUTTON_SHIFT", Some(false));
    assert!(!h.state().song_view, "IDEAS: the Ideas view");
}

#[test]
fn controller_scene_button_and_pads_pick_sections_in_the_song_view() {
    let Some(mut h) = scripted_harness(true) else { return };
    assert!(h.state().song_view);
    if h.state().doc.as_ref().unwrap().borrow().project().sections.len() < 4 {
        return;
    }
    if !play(&mut h) {
        return;
    }
    press(&mut h, "SCENE");
    press(&mut h, "4");
    assert_eq!(h.state().section, Some(3));
    assert_eq!(h.state().queued_section(), Some(3));
    press(&mut h, "SCENE");
    h.state_mut().stop();
}
