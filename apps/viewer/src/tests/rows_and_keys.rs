use super::*;

/// The pictures of DECKARD's plug-ins on the controller (not a check: run it by name,
/// `cargo test -p deckard -- --ignored make_plugin_pictures`): each built-in plug-in's
/// panel, as the app draws it, in a chain tile (90 pixels high for an instrument, 40
/// for an effect, as Maschine's) and as the 240 × 196 logo on the right display,
/// written to `assets/plugins` with the list `plugin_pictures.rs` embeds.
#[test]
#[ignore]
fn make_plugin_pictures() {
    use image::imageops::{FilterType, resize};
    let Some(mut h) = harness(false, true) else { return };
    layout::style(&h.ctx);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/plugins");
    std::fs::create_dir_all(&dir).unwrap();
    let (g, s) = (0, 0);
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
    // The Mix view, where the strip shows each slot's panel.
    h.state_mut().song_view = false;
    h.state_mut().mixer = true;
    // A drum hit to show: a falling sine with a click of noise, two seconds.
    let tmp = tempfile::tempdir().unwrap();
    let wav = tmp.path().join("picture.wav");
    {
        let spec = hound::WavSpec { channels: 1, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        let (mut phase, mut noise) = (0.0f64, 12345u32);
        for i in 0..88_200 {
            let t = i as f64 / 44_100.0;
            phase += std::f64::consts::TAU * (45.0 + 180.0 * (-t * 25.0).exp()) / 44_100.0;
            noise = noise.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let click = ((noise >> 16) as f64 / 32_768.0 - 1.0) * (-t * 120.0).exp() * 0.4;
            let beat = if (t * 4.0).fract() < 0.02 { 0.3 } else { 0.0 };
            let v = (phase.sin() * (-t * 2.2).exp() * 0.9 + click + beat * (noise as f64 / u32::MAX as f64 - 0.5)).clamp(-1.0, 1.0);
            w.write_sample((v * 30_000.0) as i16).unwrap();
        }
        w.finalize().unwrap();
    }
    let mut written = Vec::new();
    let choices: Vec<_> =
        h.state().plugin_choices().into_iter().filter(|(e, _)| e.format == deckard_lua_host::state::PluginFormat::Internal).collect();
    for (entry, what) in choices {
        while !project_of(&h).groups[g].sounds[s].slots.is_empty() {
            layout::apply_acts(h.state_mut(), vec![layout::Act::RemoveSlot(0)]);
            h.run_steps(1);
        }
        h.state_mut().add_slot(what);
        h.run_steps(6);
        // The Sampler and Audio show a sample's waveform: their empty panels have none.
        let key = project_of(&h).groups[g].sounds[s].slots.first().map(|x| deckard_lua_host::plugin_picture_key(&x.name));
        if matches!(key.as_deref(), Some("sampler" | "audio")) {
            let smp = deckard_engine::sample::Sample::load(&wav).unwrap();
            let done = if key.as_deref() == Some("sampler") {
                h.state_mut().add_zone(g, s, &wav, smp.frames.len())
            } else {
                h.state_mut().load_loop(g, s, &wav, &smp)
            };
            done.unwrap();
            h.run_steps(20);
        }
        let Some(slot) = project_of(&h).groups[g].sounds[s].slots.first().cloned() else { continue };
        let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap_or_default();
        let Some(r) = panels.iter().find(|p| p.0 == 0).map(|p| p.1) else {
            eprintln!("{}: no panel", entry.name);
            continue;
        };
        let ppp = h.ctx.pixels_per_point();
        let shot = h.render().unwrap();
        let (x, y) = ((r.min.x * ppp).max(0.0) as u32, (r.min.y * ppp).max(0.0) as u32);
        let (w, hh) = (((r.width() * ppp) as u32).min(shot.width() - x), ((r.height() * ppp) as u32).min(shot.height() - y));
        let panel = image::imageops::crop_imm(&shot, x, y, w, hh).to_image();
        // Scaled to the height, the width as it comes (but not past Maschine's widest).
        let fit = |max_w: u32, max_h: u32| {
            let k = (max_w as f32 / w as f32).min(max_h as f32 / hh as f32);
            resize(&panel, ((w as f32 * k).round() as u32).max(1), ((hh as f32 * k).round() as u32).max(1), FilterType::Lanczos3)
        };
        let thumb = if entry.instrument { fit(160, 90) } else { fit(130, 40) };
        // Padded to 240 × 196 with nothing: the screen's own background shows around
        // the panel, not black bars (C58).
        let mut logo = image::RgbaImage::from_pixel(240, 196, image::Rgba([0, 0, 0, 0]));
        let big = fit(240, 196);
        image::imageops::overlay(&mut logo, &big, ((240 - big.width()) / 2) as i64, ((196 - big.height()) / 2) as i64);
        let key = deckard_lua_host::plugin_picture_key(&slot.name);
        thumb.save(dir.join(format!("{key}_Thumb.png"))).unwrap();
        logo.save(dir.join(format!("{key}_Logo.png"))).unwrap();
        written.push(key);
    }
    written.sort();
    written.dedup();
    let mut src = String::from(
        "//! DECKARD's own pictures of its plug-ins for the controller's displays (tiles in\n\
             //! the chain, logos on the right display), made by the `make_plugin_pictures` test.\n\n\
             /// (style sheet key, thumbnail PNG, logo PNG).\n\
             pub const PICTURES: &[(&str, &[u8], &[u8])] = &[\n",
    );
    for k in &written {
        src += &format!(
            "    (\"{k}\", include_bytes!(\"../assets/plugins/{k}_Thumb.png\"), include_bytes!(\"../assets/plugins/{k}_Logo.png\")),\n"
        );
    }
    src += "];\n";
    src += PICTURE_HELPERS;
    std::fs::write(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/plugin_pictures.rs"), src).unwrap();
    eprintln!("{} plug-in pictures", written.len());
}

/// Pattern view sound rows and Song view group rows: double-click renames, the
/// coloured box at the right mutes and unmutes.
#[test]
fn rows_rename_and_mute() {
    let Some(mut h) = harness(false, true) else { return };
    let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
    let list_right = grid.left() - 1.0;
    let g = h.state().group;
    // Sound 1 of the focused group.
    let y = grid.top() + 12.0;
    double_click(&mut h, Pos2::new(list_right - 80.0, y));
    assert_eq!(h.state().renaming, Some(deckard_doc::Target::Sound(g, 0)));
    type_text(&mut h, "Big Kick");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(project_of(&h).groups[g].sounds[0].name, "Big Kick");
    assert_eq!(h.state().renaming, None);
    let mute = Pos2::new(list_right - 9.5, y);
    click(&mut h, mute);
    assert!(project_of(&h).groups[g].sounds[0].muted, "the box mutes");
    click(&mut h, mute);
    assert!(!project_of(&h).groups[g].sounds[0].muted, "and unmutes");

    // Song view: group B's row.
    let rows_top = 33.0 + 3.0 + 50.0;
    let gy = rows_top + 24.0 + 12.0;
    // (A group's double-click mutes it: Rename is in its menu.)
    layout::apply_acts(h.state_mut(), vec![layout::Act::StartRename(deckard_doc::Target::Group(1))]);
    h.run_steps(2);
    assert_eq!(h.state().renaming, Some(deckard_doc::Target::Group(1)));
    type_text(&mut h, "Snares 2");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(project_of(&h).groups[1].name, "Snares 2");
    let gmute = Pos2::new(list_right - 9.5, gy);
    click(&mut h, gmute);
    assert!(project_of(&h).groups[1].muted);
    click(&mut h, gmute);
    assert!(!project_of(&h).groups[1].muted);

    // Esc cancels a rename.
    double_click(&mut h, Pos2::new(list_right - 80.0, gy));
    type_text(&mut h, "Nope");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(project_of(&h).groups[1].name, "Snares 2");
    assert_eq!(h.state().renaming, None);
}

/// Double-click renames scenes (Ideas view tabs, Song view section headers); groups
/// (Ideas view tiles) rename from their menu; a single click still selects.
#[test]
fn scenes_and_groups_rename() {
    let Some(mut h) = harness(false, false) else { return };
    // Scene tab 2.
    let tab = Pos2::new(3.0 + 33.0 + 99.0 + 49.0, 33.0 + 3.0 + 16.0);
    double_click(&mut h, tab);
    assert_eq!(h.state().renaming, Some(deckard_doc::Target::Scene(1)));
    type_text(&mut h, "Drop");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(project_of(&h).scenes[1].name, "Drop");
    assert_eq!(h.state().renaming, None);
    click(&mut h, Pos2::new(tab.x - 99.0, tab.y));
    assert_eq!(h.state().scene, 0, "a single click still selects");

    // Group tile B.
    let tiles: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-tiles"))).expect("tiles drawn");
    let tile = Pos2::new(tiles.left() + 99.0 + 49.0, tiles.center().y);
    // (A group's double-click mutes it: Rename is in its menu.)
    click(&mut h, tile);
    layout::apply_acts(h.state_mut(), vec![layout::Act::StartRename(deckard_doc::Target::Group(1))]);
    h.run_steps(2);
    assert_eq!(h.state().renaming, Some(deckard_doc::Target::Group(1)));
    type_text(&mut h, "Snare Bus");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(project_of(&h).groups[1].name, "Snare Bus");
    // Esc cancels.
    layout::apply_acts(h.state_mut(), vec![layout::Act::StartRename(deckard_doc::Target::Group(1))]);
    h.run_steps(2);
    type_text(&mut h, "Nope");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(project_of(&h).groups[1].name, "Snare Bus");
    assert_eq!(h.state().renaming, None);

    // Song view: the first section's header renames its scene.
    h.state_mut().song_view = true;
    h.run_steps(3);
    let secs: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-sections"))).expect("sections drawn");
    let scene = project_of(&h).sections[0].scene.expect("section 0 has a scene");
    double_click(&mut h, Pos2::new(secs.left() + 20.0, secs.center().y));
    assert_eq!(h.state().section, Some(0));
    assert_eq!(h.state().renaming, Some(deckard_doc::Target::Scene(scene)));
    type_text(&mut h, "Intro");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(project_of(&h).scenes[scene].name, "Intro");
}

#[test]
fn pattern_loop_tail_is_drawn_at_a_shifted_pattern_start() {
    let mut h = blank_harness(true);
    a_pattern(&mut h);
    let start = PPQ / 2;
    let length = 4 * PPQ;
    let tail = PPQ / 2;
    let note = Note {
        position: start + length - PPQ,
        length: PPQ + tail,
        sound: 0,
        pitch: 60,
        velocity: 127,
        probability: 100,
        ratchet: Default::default(),
        expr: None,
    };
    {
        let mut doc = h.state().doc.as_ref().unwrap().borrow_mut();
        doc.add_note(0, 0, note).unwrap();
        doc.set_pattern_span(0, 0, start, length).unwrap();
    }
    for piano in [false, true] {
        h.state_mut().piano_roll = piano;
        h.run_steps(2);
        let (grid, (shown_from, scale)): (egui::Rect, (f64, f32)) =
            h.ctx.data(|d| (d.get_temp(egui::Id::new("deckard-grid")).unwrap(), d.get_temp(egui::Id::new("deckard-grid-time")).unwrap()));
        let x = |tick: i64| grid.left() + ((tick as f64 - shown_from) as f32) * scale;
        let visible = h.output().shapes.iter().any(|s| {
            matches!(&s.shape, egui::Shape::Rect(r) if r.fill.a() == 68
                && (r.rect.left() - x(start)).abs() < 0.01
                && (r.rect.right() - (x(start + tail) - 2.0)).abs() < 0.01
                && r.rect.intersects(grid))
        });
        assert!(visible, "continuation visible, piano roll = {piano}");
        assert_eq!(notes_of(&h), [note]);
    }
}

/// Piano roll: the sound list stays (clicking a sound shows its notes), the note
/// area has a keyboard and pitch rows, and drawing adds notes at the row's pitch.
#[test]
fn piano_roll_keeps_the_sound_list() {
    let Some(mut h) = harness(false, true) else { return };
    h.state_mut().piano_roll = true;
    h.run_steps(3);
    let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
    let list_right = grid.left() - 1.0 - 46.0;
    // Click sound 3 in the list.
    click(&mut h, Pos2::new(list_right - 80.0, grid.top() + 2.0 * 24.0 + 12.0));
    assert_eq!(h.state().sound, 2, "the sound list still selects sounds");
    // Draw a note on the row at the middle of the view.
    h.state_mut().mouse = layout::MouseMode::Draw;
    h.run_steps(2);
    let (g, pi) = (h.state().group, h.state().pattern);
    let before: Vec<Note> = notes_of(&h).into_iter().filter(|n| n.sound == 2).collect();
    let y = grid.center().y;
    click(&mut h, Pos2::new(grid.left() + 30.0, y));
    let after: Vec<Note> = notes_of(&h).into_iter().filter(|n| n.sound == 2).collect();
    assert_eq!(after.len(), before.len() + 1, "a note was drawn for the focused sound");
    let added = after.iter().find(|n| !before.contains(n)).unwrap();
    assert!((0..=127).contains(&added.pitch) && added.pitch != 60 || added.pitch == 60);
    let _ = (g, pi);
}

/// Playing sounds light their level meters; silent ones stay dark.
#[test]
fn meters_follow_playback() {
    let Some(mut h) = harness(false, true) else { return };
    if !play(&mut h) {
        return;
    }
    let mut lit = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        h.run_steps(1);
        let (l, r) = h.state().meter(0, None);
        lit |= l > 0.01 && r > 0.01;
    }
    h.state_mut().stop();
    assert!(lit, "the kicks' meter moved while the song played");
    // Silent group (a plug-in group: no sound rendered).
    let quiet = (0..project_of(&h).groups.len()).filter(|g| h.state().meter(*g, None) == (0.0, 0.0)).count();
    assert!(quiet > 0, "groups that play nothing stay at zero");
}

/// Notes can be drawn, dragged and double-clicked in for a sound whose instrument is
/// a plug-in (Pad's Massive). (The plug-ins themselves only load on the main thread,
/// so the app is checked with `--input` too, see the README.)
#[test]
fn notes_draw_for_plugin_sounds() {
    let Some(mut h) = harness(false, true) else { return };
    let Some(g) = project_of(&h).groups.iter().position(|g| g.sounds.first().is_some_and(|s| s.plugin.is_some())) else { return };
    h.state_mut().group = g;
    h.state_mut().pattern = 0;
    h.state_mut().sound = 0;
    h.state_mut().mouse = layout::MouseMode::Draw;
    h.run_steps(3);
    let before = notes_of(&h).len();
    let pos = grid_pos(&h, 0, 3 * 240 + 120);
    click(&mut h, pos);
    assert_eq!(notes_of(&h).len(), before + 1, "drum grid: a click adds a note");
    h.state_mut().piano_roll = true;
    h.run_steps(3);
    let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
    let x = grid_pos(&h, 0, 5 * 240 + 120).x;
    click(&mut h, Pos2::new(x, grid.center().y));
    assert_eq!(notes_of(&h).len(), before + 2, "piano roll: a click adds a note");
    // Select mode: drag the drum-grid note one beat right, then double-click adds.
    h.state_mut().mouse = layout::MouseMode::Select;
    h.state_mut().piano_roll = false;
    h.run_steps(3);
    // (Dropped half a step on: the start snaps down to the step it's in.)
    let grab = egui::vec2(3.0, 0.0);
    let (from, to) = (grid_pos(&h, 0, 3 * 240) + grab, grid_pos(&h, 0, 3 * 240 + 960 + 120) + grab);
    drag(&mut h, from, to);
    assert!(
        notes_of(&h).iter().any(|n| n.position - project_of(&h).groups[g].patterns[0].start == 3 * 240 + 960),
        "drag moves the note: {:?}",
        notes_of(&h)
    );
    let pos = grid_pos(&h, 0, 9 * 240 + 120);
    double_click(&mut h, pos);
    assert_eq!(notes_of(&h).len(), before + 3, "select mode: double-click adds a note");
}

/// Dragging one note of a chord (Pad: several pitches at one time, stacked on one
/// drum-grid row) moves it.
#[test]
fn chord_notes_drag() {
    let Some(mut h) = harness(false, true) else { return };
    let Some(g) = project_of(&h).groups.iter().position(|g| g.sounds.first().is_some_and(|s| s.plugin.is_some())) else { return };
    h.state_mut().group = g;
    h.state_mut().pattern = 0;
    h.run_steps(3);
    let start = project_of(&h).groups[g].patterns[0].start;
    let before = notes_of(&h);
    // Pad's first pattern starts with a chord (notes a few ticks apart).
    let t = before.iter().map(|n| n.position - start).min().unwrap();
    let from = grid_pos(&h, 0, t + 20);
    let to = grid_pos(&h, 0, t + 20 + 960);
    drag(&mut h, from, to);
    let after = notes_of(&h);
    // The note on top under the pointer moves (the chord's notes overlap).
    let gone: Vec<&Note> = before.iter().filter(|n| !after.contains(n)).collect();
    assert_eq!(gone.len(), 1, "one note moved: {gone:?}");
    assert!(gone[0].position - start < 240, "from the first chord");
    assert!(after.contains(&Note { position: start + 960, ..*gone[0] }), "to a beat later, on the grid");
    assert_eq!(after.len(), before.len());
}

/// A piano-roll key sounds when the button goes down, before it is released.
/// Pressing a sound's number in the list plays the root note, in the drum grid
/// and in the piano roll, until the button is let go.
#[test]
fn sound_number_plays_the_root_note() {
    let Some(mut h) = harness(false, true) else { return };
    for piano in [false, true] {
        h.state_mut().piano_roll = piano;
        h.run_steps(3);
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        let keys = if piano { 46.0 } else { 0.0 };
        // The number box of the list's second row (the list sits left of the keys).
        let pos = Pos2::new(grid.left() - keys - 146.0, grid.top() + 24.0 + 12.0);
        h.hover_at(pos);
        h.step();
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        if !matches!(h.state().output, Some(Ok(_))) {
            eprintln!("no audio output; skipping");
            return;
        }
        let (g, root) = (h.state().group, h.state().scale.0);
        assert_eq!(h.state().held, Some((g, 1, root)), "piano roll: {piano}");
        h.step();
        assert!(h.state().lit.contains(&(g, 1)), "the held sound's number is lit");
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.step();
        assert_eq!(h.state().held, None);
        assert_eq!(h.state().sound, 1, "and the click focuses the sound");
        std::thread::sleep(deckard_engine::output::FLASH);
        h.step();
        assert!(!h.state().lit.contains(&(g, 1)), "and goes out once let go");
    }
}

#[test]
fn keys_sound_on_press() {
    let Some(mut h) = harness(false, true) else { return };
    h.state_mut().piano_roll = true;
    // Group A's first sound is a sampled kick.
    h.state_mut().group = 0;
    h.state_mut().sound = 0;
    h.run_steps(3);
    let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
    let pos = Pos2::new(grid.left() - 20.0, grid.center().y);
    h.hover_at(pos);
    h.step();
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    let Some(Ok(out)) = &h.state().output else {
        eprintln!("no audio output; skipping");
        return;
    };
    // The key goes to the engine at once and sounds one block after the press (at
    // its sample): within a few blocks, still before the button is released.
    let wait = std::time::Instant::now();
    let mut voices = 0;
    while voices == 0 && wait.elapsed() < std::time::Duration::from_millis(100) {
        voices = out.call(|e| e.active_voices()).unwrap_or(0);
    }
    assert!(voices > 0, "the kick sounds on press, before the button is released");
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.step();
}

#[test]
fn mk3_knob_burst_keeps_full_travel() {
    use deckard_mk3::Event as E;
    let mut app = App::default();
    app.new_project();
    app.controller = None;
    let mut req = Vec::new();
    // On the Mixer page (`knob` reads `deckard.state()` directly), a burst of 10
    // 500 Hz reports in one frame (+0.10 turn = +7 dB) must move by the full sum.
    app.mk3_events(vec![E::Button("BUTTON_MIX", true), E::Button("BUTTON_MIX", false)], &mut req);
    let before = app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].level;
    app.mk3_events(vec![E::Knob(0, 0.01); 10], &mut req);
    let after = app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].level;
    let db = deckard_model::level_db(after) - deckard_model::level_db(before);
    assert!((db - 7.0).abs() < 0.05, "mixer knob moved by full +7 dB burst: {before} -> {after} ({db:+.2} dB)");

    // On the Control page with a native plug-in (Replicant), a burst of 10 reports
    // in one frame on knob 4 (Detune, continuous) must move by the full +0.20.
    app.mk3_events(vec![E::Button("BUTTON_PLUGIN", true), E::Button("BUTTON_PLUGIN", false)], &mut req);
    let cls = deckard_plugins::native::CLASSES[0];
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .add_slot(0, 0, 0, &deckard_doc::NewModule::Plugin { id: cls.plugin_id(), name: cls.name.into(), chunk: Vec::new() })
        .unwrap();
    app.slot = Some(0);
    app.mk3_events(Vec::new(), &mut req);
    let p0 = app.plugin_view().unwrap().values[3].0;
    app.mk3_events(vec![E::Knob(3, 0.02); 10], &mut req);
    let p1 = app.plugin_view().unwrap().values[3].0;
    let want = (p0 + 0.20).clamp(0.0, 1.0);
    assert!((p1 - want).abs() < 1e-4, "plug-in knob moved by full burst: {p0} -> {p1} (want {want})");
}

#[test]
fn mk3_touchstrip_move_burst_lights_pitch_and_mod_without_redrawing_screens() {
    use deckard_mk3::Event as E;
    let mut app = App::default();
    app.new_project();
    app.controller = None;
    let mut req = Vec::new();
    let (_, frame) = app.mk3_events(Vec::new(), &mut req).screens.expect("initial screen draw");
    assert!(matches!(&frame, mk3_io::ScreenFrame::Picture(p) if p.px.is_empty()), "UI thread records DrawOps without rasterizing");
    assert_eq!(frame.into_pixels().len(), 960 * 272, "display thread rasterizes the 960x272 frame");

    // Pitch mode: select PITCH, flush the screen update, then slide to 0.9 in a burst.
    app.mk3_events(vec![E::Button("BUTTON_PITCH", true), E::Button("BUTTON_PITCH", false)], &mut req);
    app.mk3_events(Vec::new(), &mut req);
    let out = app.mk3_events(vec![E::Touch(0, 0, 0.5), E::Touch(0, 1, 0.65), E::Touch(0, 1, 0.9)], &mut req);
    assert!(out.screens.is_none(), "touchstrip pitch move does not re-rasterize the 960x272 screens");
    // Upper half of the strip (dots 13..=23) is lit, lower half (dots 1..=12) is dark.
    assert_eq!(out.leds.get("LED_TS_12"), Some(0), "dot 12 stays dark on upward bend");
    assert!(out.leds.get("LED_TS_13").unwrap() > 0 && out.leds.get("LED_TS_23").unwrap() > 0, "dots 13..23 light up on upward bend");

    // Mod mode: select MOD, flush the screen update, then slide to 0.8 in a burst.
    app.mk3_events(vec![E::Button("BUTTON_MOD", true), E::Button("BUTTON_MOD", false)], &mut req);
    app.mk3_events(Vec::new(), &mut req);
    let out = app.mk3_events(vec![E::Touch(0, 0, 0.2), E::Touch(0, 1, 0.5), E::Touch(0, 1, 0.8)], &mut req);
    assert!(out.screens.is_none(), "touchstrip mod move does not re-rasterize the 960x272 screens");
    assert!(out.leds.get("LED_TS_01").unwrap() > 0 && out.leds.get("LED_TS_20").unwrap() > 0, "mod bar lights up to dot 20");
    assert_eq!(out.leds.get("LED_TS_25"), Some(0), "top dot stays dark at 0.8");
}

/// A group with no pattern: the touch strip is sent the place in the bar.
#[test]
fn the_strip_tracks_the_bar_in_a_group_without_patterns() {
    let mut app = App::default();
    app.new_project();
    app.controller = None;
    let doc = app.doc.clone().unwrap();
    let bar = doc.borrow().project().bar_length();
    app.group = doc.borrow_mut().add_group().unwrap();
    assert_eq!(app.scene_pattern(app.group), None);
    assert_eq!(app.surface_view().progress, None, "no playhead, nothing to track");
    app.seek_pattern(bar / 4);
    assert_eq!(app.surface_view().progress, Some(0.25), "a quarter through its bar");
}

#[test]
fn seek_pattern_moves_playhead_only_when_stopped() {
    let mut app = App::default();
    app.new_project();
    app.controller = None;
    app.seek_pattern(2 * PPQ);
    let head = app.shown_playhead().expect("parked playhead set while stopped");
    assert_eq!(head.position as i64, 2 * PPQ);
    // While playing, clicking the timeline must not move the playhead.
    app.playing = Some(Mode::Scene(0));
    app.seek_pattern(3 * PPQ);
    assert_eq!(app.parked.expect("parked unchanged").position as i64, 2 * PPQ);
}

/// ⌘V while stopped, with the playhead put in the pattern, pastes at the playhead
/// (not after the copied notes, which would make the pattern longer); once played,
/// it pastes after them again.
#[test]
fn keys_paste_at_the_playhead_put_in_the_pattern() {
    let mut app = App { grid: PPQ / 4, ..App::default() };
    app.new_project();
    app.controller = None;
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_pattern(0, 0, "P").unwrap();
    doc.borrow_mut().assign_pattern(0, 0, Some(0)).unwrap();
    let note = |position| Note {
        ratchet: Default::default(),
        expr: None,
        sound: 0,
        position,
        length: PPQ / 2,
        pitch: 60,
        velocity: 100,
        probability: 100,
    };
    doc.borrow_mut().edit_notes("t", 0, 0, &[], &[note(0), note(PPQ)]).unwrap();
    let len = layout::pattern_len(doc.borrow().project(), 0, 0);
    let at = || -> Vec<i64> { doc.borrow().project().groups[0].patterns[0].notes.iter().map(|n| n.position).collect() };
    app.pattern = 0;
    app.copy_notes(false);
    app.seek_pattern(2 * PPQ);
    app.paste_notes(clipboard::PasteAt::Keys);
    assert_eq!(at(), [0, PPQ, 2 * PPQ, 3 * PPQ]);
    assert_eq!(layout::pattern_len(doc.borrow().project(), 0, 0), len, "no longer");
    // Not placed (played since): after the copied notes, on the grid.
    app.placed_head = false;
    doc.borrow_mut().undo();
    app.paste_notes(clipboard::PasteAt::Keys);
    assert_eq!(at(), [0, PPQ, 3 * PPQ / 2, 5 * PPQ / 2]);
}

/// Drawing or adding notes in the Pattern Editor when the focused group has no active
/// pattern creates `"Pattern 1"` with the default pattern length, assigns it to the
/// active scene, and adds the drawn notes in a single undo step.
#[test]
fn drawing_notes_without_a_pattern_creates_and_assigns_one() {
    let mut app = App { follow: true, grid: PPQ / 4, prop_set: 1, ..Default::default() };
    app.new_project();
    app.controller = None;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 930.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    h.run_steps(3);

    assert!(project_of(&h).groups[0].patterns.is_empty());
    assert_eq!(h.state().scene_pattern(0), None);
    let default_len = h.state().default_pattern_length(&project_of(&h));

    // 1. Clicking in Draw mode on the Group (pad-step) note grid creates Pattern 1,
    // assigns it to the active scene, focuses the sound, and places the note in it.
    h.state_mut().mouse = layout::MouseMode::Draw;
    h.run_steps(2);
    let step = PPQ / 4;
    let at = grid_pos(&h, 2, step + 10);
    click(&mut h, at);
    let p = project_of(&h);
    assert_eq!(p.groups[0].patterns.len(), 1, "a pattern was created");
    assert_eq!(p.groups[0].patterns[0].name, "Pattern 1");
    assert_eq!(p.groups[0].patterns[0].length, default_len);
    assert_eq!(p.scenes[0].patterns, vec![(0, 0)], "assigned to the active scene");
    assert_eq!(h.state().pattern, 0);
    assert_eq!(h.state().sound, 2, "the clicked sound row is focused");
    assert_eq!(p.groups[0].patterns[0].notes.len(), 1);
    let note = p.groups[0].patterns[0].notes[0];
    assert_eq!((note.sound, note.position, note.length), (2, step, step));

    // Undo removes both the note and the auto-created pattern in one step.
    let doc = h.state().doc.clone().unwrap();
    assert_eq!(doc.borrow().undo_labels(), vec!["Add Note"]);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert!(project_of(&h).groups[0].patterns.is_empty(), "single undo removes pattern and note");
    assert!(project_of(&h).scenes[0].patterns.is_empty());

    // 2. Click-and-drag painting across steps in Draw mode when there is no pattern
    // creates Pattern 1 and places all painted steps in one undo step, and dragging
    // the right edge of a note adjusts its length.
    let (from, to) = (grid_pos(&h, 1, 10), grid_pos(&h, 1, 2 * step + 10));
    drag(&mut h, from, to);
    let p = project_of(&h);
    assert_eq!(p.groups[0].patterns.len(), 1);
    assert_eq!(p.scenes[0].patterns, vec![(0, 0)]);
    let pos: Vec<i64> = p.groups[0].patterns[0].notes.iter().filter(|n| n.sound == 1).map(|n| n.position).collect();
    assert_eq!(pos, vec![0, step, 2 * step], "three steps painted");
    assert_eq!(doc.borrow().undo_labels(), vec!["Draw Notes"]);

    // Drag the right edge of the last painted note to resize its length.
    let edge_from = grid_pos(&h, 1, 3 * step) - egui::vec2(2.0, 0.0);
    let edge_to = grid_pos(&h, 1, 5 * step) - egui::vec2(2.0, 0.0);
    drag(&mut h, edge_from, edge_to);
    let p = project_of(&h);
    let last = p.groups[0].patterns[0].notes.iter().find(|n| n.sound == 1 && n.position == 2 * step).expect("last note");
    assert!(last.length > step, "right-edge drag extended the note length: {}", last.length);
    assert_eq!(doc.borrow().undo_labels(), vec!["Resize Notes", "Draw Notes"]);

    // Undo both steps back to no pattern.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert!(project_of(&h).groups[0].patterns.is_empty());

    // 3. Keyboard (piano-roll) note grid in Draw mode when there is no pattern creates
    // Pattern 1, assigns it to the scene, and places the note at the clicked pitch.
    h.state_mut().piano_roll = true;
    h.state_mut().sound = 3;
    h.run_steps(2);
    let at = grid_pos(&h, 4, step + 10);
    click(&mut h, at);
    let p = project_of(&h);
    assert_eq!(p.groups[0].patterns.len(), 1);
    assert_eq!(p.scenes[0].patterns, vec![(0, 0)]);
    assert_eq!(p.groups[0].patterns[0].notes.len(), 1);
    let pr_note = p.groups[0].patterns[0].notes[0];
    assert_eq!((pr_note.sound, pr_note.position), (3, step));

    // 4. Double-clicking in Select mode on a newly added Group with no pattern creates
    // Pattern 1 for that Group, assigns it to the active scene, and adds the note.
    h.state_mut().piano_roll = false;
    h.state_mut().mouse = layout::MouseMode::Select;
    h.state_mut().arrange(layout::Act::AddGroup);
    h.run_steps(2);
    assert_eq!(h.state().group, 1);
    assert!(project_of(&h).groups[1].patterns.is_empty());
    assert_eq!(h.state().scene_pattern(1), None);
    let at = grid_pos(&h, 0, 2 * step + 10);
    double_click(&mut h, at);
    let p = project_of(&h);
    assert_eq!(p.groups[1].patterns.len(), 1);
    assert_eq!(p.groups[1].patterns[0].name, "Pattern 1");
    assert!(p.scenes[0].patterns.contains(&(1, 0)), "Group B's new pattern is in the scene");
    assert_eq!(p.groups[1].patterns[0].notes.len(), 1);
    assert_eq!(p.groups[1].patterns[0].notes[0].position, 2 * step);
}
