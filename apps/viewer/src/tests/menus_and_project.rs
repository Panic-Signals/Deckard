use super::*;

/// Right-clicking a sound row opens Maschine's sound menu: colour, copy and paste
/// (with its notes), reset; undo takes each back in one step.
#[test]
fn sound_menu() {
    let Some(mut h) = harness(false, true) else { return };
    if let Some(d) = render_dir() {
        std::fs::create_dir_all(&d).unwrap();
    }
    let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
    let row = |r: usize| Pos2::new(grid.left() - 81.0, grid.top() + 12.0 + r as f32 * 24.0);
    let g = h.state().group;
    let before = project_of(&h);
    let notes_of = |p: &deckard_model::Project, s: usize| -> Vec<(i64, i64)> {
        p.groups[g]
            .patterns
            .iter()
            .flat_map(|pt| pt.notes.iter().filter(|n| n.sound == s).map(|n| (n.position - pt.start, n.pitch)))
            .collect()
    };
    let empty = (1..16).find(|&s| before.groups[g].sounds[s].slots.is_empty() && notes_of(&before, s).is_empty()).unwrap();

    // The menu, and the colour palette.
    right_click(&mut h, row(0));
    assert_eq!(h.state().sound, 0);
    if let Some(d) = render_dir() {
        h.render().unwrap().save(d.join("menu-sound.png")).unwrap();
    }
    h.get_by_label("Color ⏵").hover();
    h.run_steps(4);
    if let Some(d) = render_dir() {
        h.render().unwrap().save(d.join("menu-sound-color.png")).unwrap();
    }
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.state_mut().menu_act(layout::Act::SetColor(vec![deckard_doc::Target::Sound(g, 0)], deckard_model::Color::Palette(5)));
    assert_eq!(project_of(&h).groups[g].sounds[0].color, deckard_model::Color::Palette(5));

    // Copy sound 1, paste it into an empty slot: its plug-ins, name and notes.
    right_click(&mut h, row(0));
    h.get_by_label("Copy").click();
    h.run_steps(3);
    assert_eq!(h.state().sound_clipboard.len(), 1);
    right_click(&mut h, row(empty));
    h.get_by_label("Paste").click();
    h.run_steps(3);
    let p = project_of(&h);
    assert_eq!(p.groups[g].sounds[empty].name, p.groups[g].sounds[0].name);
    assert_eq!(p.groups[g].sounds[empty].slots, p.groups[g].sounds[0].slots);
    assert_eq!(notes_of(&p, empty), notes_of(&p, 0));
    assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_label(), Some("Paste Sound"));

    // Reset it: empty again, as one undo step.
    right_click(&mut h, row(empty));
    h.get_by_label("Reset").click();
    h.run_steps(3);
    let p = project_of(&h);
    assert!(p.groups[g].sounds[empty].slots.is_empty() && notes_of(&p, empty).is_empty());
    assert_eq!(p.groups[g].sounds[empty].name, format!("Sound {}", empty + 1));
    h.state_mut().undo();
    assert_eq!(notes_of(&project_of(&h), empty), notes_of(&project_of(&h), 0));
}

/// The Ideas view's pattern menu: Duplicate (after it), Double, Rename, Clear,
/// Delete; and the group tile's menu.
#[test]
fn pattern_menu() {
    let Some(mut h) = harness(false, false) else { return };
    let before = project_of(&h);
    let g = 0;
    let n = before.groups[g].patterns.len();
    right_click(&mut h, cell(g, 0));
    if let Some(d) = render_dir() {
        h.render().unwrap().save(d.join("menu-pattern.png")).unwrap();
    }
    h.get_by_label("Duplicate").click();
    h.run_steps(3);
    let p = project_of(&h);
    assert_eq!(p.groups[g].patterns.len(), n + 1);
    assert_eq!(p.groups[g].patterns[1].notes.len(), before.groups[g].patterns[0].notes.len());
    assert_eq!((h.state().group, h.state().pattern), (g, 1));
    right_click(&mut h, cell(g, 1));
    h.get_by_label("Double").click();
    h.run_steps(3);
    assert_eq!(project_of(&h).groups[g].patterns[1].length, 2 * before.groups[g].patterns[0].length);
    right_click(&mut h, cell(g, 1));
    h.get_by_label("Rename").click();
    h.run_steps(3);
    assert_eq!(h.state().renaming, Some(deckard_doc::Target::Pattern(g, 1)));
    type_text(&mut h, "Kicks Long");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(project_of(&h).groups[g].patterns[1].name, "Kicks Long");
    right_click(&mut h, cell(g, 1));
    h.get_by_label("Clear").click();
    h.run_steps(3);
    assert!(project_of(&h).groups[g].patterns[1].notes.is_empty());
    right_click(&mut h, cell(g, 1));
    h.get_by_label("Delete").click();
    h.run_steps(3);
    let p = project_of(&h);
    assert_eq!(p.groups[g].patterns.len(), n);
    // Duplicate moved the next pattern on a place; Delete leaves the others where they are.
    let mut moved = before.groups[g].patterns[1].clone();
    moved.slot += 1;
    assert_eq!(p.groups[g].patterns[1], moved);
    // The group tile: its menu, and a colour for the group.
    h.state_mut().menu_act(layout::Act::SetColor(vec![deckard_doc::Target::Group(1)], deckard_model::Color::Palette(12)));
    assert_eq!(project_of(&h).groups[1].color, deckard_model::Color::Palette(12));
}

/// The channel properties: the icon strip, the set tiles, page tabs, and edits
/// from a page (a routing pick, a switch, Generate).
#[test]
fn channel_properties() {
    let Some(mut h) = harness(false, true) else { return };
    layout::style(&h.ctx);
    let knobs = |h: &Harness<'static, App>| -> Vec<(String, egui::Rect)> {
        h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap_or_default()
    };
    let names = |h: &Harness<'static, App>| knobs(h).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
    h.state_mut().channel = true;
    h.run_steps(3);
    let shots = render_dir();
    let shot = |h: &mut Harness<'static, App>, name: &str| {
        if let Some(dir) = &shots {
            h.render().unwrap().save(dir.join(format!("channel-{name}.png"))).unwrap();
        }
    };
    // Every page of every set, for the sound, the group and the master.
    for (tab, level) in [(layout::ChainTab::Sound, "sound"), (layout::ChainTab::Group, "group"), (layout::ChainTab::Master, "master")] {
        // (Coming to a chain shows where it was left: switched first, then paged.)
        h.state_mut().chain_tab = tab;
        h.run_steps(1);
        for set in 0..3 {
            for page in 0..3 {
                (h.state_mut().prop_set, h.state_mut().prop_page) = (set, page);
                h.run_steps(3);
                shot(&mut h, &format!("{level}-{set}-{page}"));
            }
        }
    }
    h.state_mut().chain_tab = layout::ChainTab::Sound;
    h.run_steps(1);
    (h.state_mut().prop_set, h.state_mut().prop_page) = (1, 0);
    h.run_steps(3);
    assert_eq!(names(&h), ["DEST.", "CUE", "LEVEL", "PAN", "AUDIO MUTE"]);
    // Route the sound to the master from the Dest. list.
    let dest = knobs(&h).into_iter().find(|(n, _)| n == "DEST.").unwrap().1;
    click(&mut h, Pos2::new(dest.center().x, dest.top() + 42.0));
    shot(&mut h, "dest-list");
    h.get_by_label("Master").click();
    h.run_steps(3);
    assert_eq!(project_of(&h).groups[0].sounds[0].channel.dest, deckard_model::AudioDest::Master);
    // Audio Mute is a switch.
    let am = knobs(&h).into_iter().find(|(n, _)| n == "AUDIO MUTE").unwrap().1;
    click(&mut h, Pos2::new(am.center().x, am.top() + 42.0));
    assert!(project_of(&h).groups[0].sounds[0].channel.audio_mute);
    // The group's MIDI input: Manual shows its source, channel, start note and thru.
    h.state_mut().chain_tab = layout::ChainTab::Group;
    h.run_steps(1);
    (h.state_mut().prop_set, h.state_mut().prop_page) = (0, 0);
    layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Group(0), "key_mode", 0.0)]);
    h.run_steps(3);
    assert_eq!(names(&h), ["KEY MODE"]);
    layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Group(0), "key_mode", 2.0)]);
    h.run_steps(3);
    assert_eq!(names(&h), ["KEY MODE", "SOURCE", "CHANNEL", "START NOTE", "THRU"]);
    shot(&mut h, "group-midi-manual");
    // Generate: Euclidean fills the sound's part of the pattern.
    h.state_mut().chain_tab = layout::ChainTab::Sound;
    h.run_steps(1);
    (h.state_mut().prop_set, h.state_mut().prop_page) = (2, 2);
    layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Sound(0, 0), "sequence_generation/mode", 1.0)]);
    h.run_steps(3);
    assert_eq!(names(&h), ["MODE", "AMOUNT", "AMOUNT", "STEPS", "DIVISION", "LIVE"]);
    shot(&mut h, "sound-generate-euclid");
    let b: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-groove-button"))).expect("the Generate button");
    click(&mut h, b.center());
    let (g, pi) = (h.state().group, h.state().pattern);
    let n = project_of(&h).groups[g].patterns[pi].notes.iter().filter(|n| n.sound == 0).count();
    assert!(n > 0, "notes generated");
    // Macros: right-click a parameter's name to make it a sound macro.
    (h.state_mut().prop_set, h.state_mut().prop_page) = (1, 0);
    h.run_steps(3);
    let level = knobs(&h).into_iter().find(|(n, _)| n == "LEVEL").unwrap().1;
    right_click(&mut h, Pos2::new(level.center().x, level.top() + 8.0));
    shot(&mut h, "macro-menu");
    h.get_by_label("Create Sound Macro").click();
    h.run_steps(3);
    let ch = deckard_doc::Target::Sound(0, 0);
    let pages = |h: &Harness<'static, App>| h.state().doc.as_ref().unwrap().borrow().macro_pages(ch);
    let oid = |h: &Harness<'static, App>, f: &str| h.state().doc.as_ref().unwrap().borrow().param_oid(ch, f);
    assert_eq!(pages(&h)[0].macros[0].target, oid(&h, "level"));
    // The Macro page shows it; an empty slot picks what it controls.
    (h.state_mut().prop_set, h.state_mut().prop_page) = (3, 0);
    h.run_steps(3);
    assert_eq!(names(&h)[..2], ["LEVEL".to_string(), "MACRO 2".to_string()]);
    let m2 = knobs(&h).into_iter().find(|(n, _)| n == "MACRO 2").unwrap().1;
    click(&mut h, m2.center());
    h.get_by_label("Output ⏵").hover();
    h.run_steps(3);
    h.get_by_label("Audio ⏵").hover();
    h.run_steps(3);
    shot(&mut h, "macro-picker");
    h.get_by_label("Pan").click();
    h.run_steps(3);
    assert_eq!(pages(&h)[0].macros[1].target, oid(&h, "pan"));
    assert_eq!(names(&h)[..3], ["LEVEL".to_string(), "PAN".to_string(), "MACRO 3".to_string()]);
    // Dragging the macro moves its parameter.
    let before = project_of(&h).groups[0].sounds[0].level;
    let lk = knobs(&h).into_iter().find(|(n, _)| n == "LEVEL").unwrap().1;
    drag(&mut h, lk.center(), lk.center() + egui::vec2(0.0, 30.0));
    assert!(project_of(&h).groups[0].sounds[0].level < before, "the macro turns the level down");
    // "+" adds a page.
    layout::apply_acts(h.state_mut(), vec![layout::Act::PropPage(1)]);
    h.run_steps(3);
    assert_eq!(pages(&h).len(), 2);
    assert_eq!(h.state().prop_page, 1);
    shot(&mut h, "macro-page2");
    // The plug-in icon goes back to the chain.
    layout::apply_acts(h.state_mut(), vec![layout::Act::ChannelMode(false)]);
    h.run_steps(3);
    assert!(!h.state().channel);
}

/// A group's MIDI: exported with each sound on its own note, imported back to the
/// same sounds; on their own channels, the same.
#[test]
fn group_midi_round_trip() {
    let Some(mut h) = harness(false, true) else { return };
    let g = 0;
    (h.state_mut().group, h.state_mut().pattern) = (g, 0);
    // Notes on three sounds.
    let start = project_of(&h).groups[g].patterns[0].start;
    let extra: Vec<Note> = (1..4)
        .map(|s| Note {
            ratchet: Default::default(),
            expr: None,
            sound: s,
            position: start + s as i64 * 240,
            length: 120,
            pitch: 60,
            velocity: 90 + s as i64,
            probability: 100,
        })
        .collect();
    h.state().doc.as_ref().unwrap().borrow_mut().edit_notes("test", g, 0, &[], &extra).unwrap();
    let before = project_of(&h).groups[g].patterns[0].clone();
    let mut key = |n: &Note| (n.sound, n.position, n.velocity, n.length);
    let mut want: Vec<_> = before.notes.iter().map(&mut key).collect();
    want.sort();
    assert!(want.iter().map(|k| k.0).collect::<std::collections::BTreeSet<_>>().len() > 1, "several sounds play");
    for batch in [1.0, 0.0] {
        layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Group(g), "midi_batch_export", batch)]);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(format!("group-{batch}.mid"));
        h.state_mut().export_group_midi(g, Some(path.clone())).unwrap();
        h.state_mut().import_group_midi(g, Some(path.clone())).unwrap();
        let after = project_of(&h).groups[g].patterns[0].clone();
        let mut got: Vec<_> = after.notes.iter().map(&mut key).collect();
        got.sort();
        assert_eq!(got, want, "batch {batch}");
    }
}

/// Drag a scene tab to move the scene; drag a section's header to move it, its
/// right edge to change its length.
#[test]
fn drag_scenes_and_sections() {
    let Some(mut h) = harness(false, true) else { return };
    let rect = |h: &Harness<'static, App>, key: (&str, usize)| -> egui::Rect { h.ctx.data(|d| d.get_temp(egui::Id::new(key))).unwrap() };
    // Ideas view: scene 1 to the third place.
    h.state_mut().song_view = false;
    h.run_steps(3);
    let names = |h: &Harness<'static, App>| project_of(h).scenes.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    let before = names(&h);
    let (a, c) = (rect(&h, ("deckard-scene-tab", 0)), rect(&h, ("deckard-scene-tab", 2)));
    drag(&mut h, a.center(), c.center());
    let mut want = before.clone();
    let first = want.remove(0);
    want.insert(2, first);
    assert_eq!(names(&h), want);
    // Song view: section 1 after section 3.
    h.state_mut().song_view = true;
    h.run_steps(3);
    let lens = |h: &Harness<'static, App>| project_of(h).sections.iter().map(|s| s.length).collect::<Vec<_>>();
    let scenes = |h: &Harness<'static, App>| project_of(h).sections.iter().map(|s| s.scene).collect::<Vec<_>>();
    let (s0, s2) = (scenes(&h), rect(&h, ("deckard-section", 2)));
    let first = rect(&h, ("deckard-section", 0));
    drag(&mut h, first.center(), s2.center());
    let mut want = s0.clone();
    let x = want.remove(0);
    want.insert(2, x);
    assert_eq!(scenes(&h), want);
    // Section 1's right edge: one bar longer.
    let bar = project_of(&h).bar_length();
    let l0 = lens(&h)[0];
    let r0 = rect(&h, ("deckard-section", 0));
    let r1 = rect(&h, ("deckard-section", 1));
    let per_bar = (r0.width() + 0.0) / (l0 as f32 / bar as f32);
    let _ = r1;
    drag(&mut h, egui::pos2(r0.right(), r0.center().y), egui::pos2(r0.right() + per_bar, r0.center().y));
    assert_eq!(lens(&h)[0], l0 + bar, "one bar longer");
    let r0 = rect(&h, ("deckard-section", 0));
    drag(&mut h, egui::pos2(r0.right(), r0.center().y), egui::pos2(r0.right() - 2.0 * per_bar, r0.center().y));
    assert_eq!(lens(&h)[0], l0 - bar, "then two shorter");
    // Mid-drag pictures: the new length, and where a moved section goes.
    if let Some(dir) = render_dir() {
        let r0 = rect(&h, ("deckard-section", 0));
        for (name, from, to) in [
            ("arranger-resize", egui::pos2(r0.right(), r0.center().y), egui::pos2(r0.right() + 2.0 * per_bar, r0.center().y)),
            ("arranger-move", r0.center(), rect(&h, ("deckard-section", 3)).center()),
        ] {
            h.hover_at(from);
            h.step();
            h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
            h.step();
            for k in 1..=6 {
                h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 6.0)));
                h.step();
            }
            h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
            h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
            h.run_steps(3);
        }
    }
}

/// Drag an effect in the chain to move it; the instrument stays first.
#[test]
fn drag_effects_in_the_chain() {
    let Some(mut h) = harness(false, true) else { return };
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
    h.state_mut().channel = false;
    h.run_steps(4);
    let names = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].slots.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&h), ["Sampler", "EQ", "Compressor"]);
    let rows: Vec<egui::Rect> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-chain-rows"))).unwrap();
    drag(&mut h, rows[2].center(), rows[1].center());
    assert_eq!(names(&h), ["Sampler", "Compressor", "EQ"]);
    assert_eq!(h.state().slot, Some(1), "the moved slot stays shown");
    // The Sampler can't be dragged after the effects, nor an effect above it.
    drag(&mut h, rows[0].center(), rows[2].center());
    drag(&mut h, rows[2].center(), rows[0].center() - egui::vec2(0.0, 8.0));
    assert_eq!(names(&h), ["Sampler", "EQ", "Compressor"], "EQ dragged to just after the Sampler");
}

/// Maschine's own effects show their pages; their knobs edit the project.
#[test]
fn effect_pages() {
    let Some(mut h) = harness(false, true) else { return };
    layout::style(&h.ctx);
    let names = |h: &Harness<'static, App>| -> Vec<String> {
        let k: Vec<(String, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap_or_default();
        k.into_iter().map(|(n, _)| n).collect()
    };
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
    h.state_mut().channel = false;
    layout::apply_acts(h.state_mut(), vec![layout::Act::SelectSlot(Some(1))]);
    h.run_steps(4);
    assert_eq!(names(&h), ["FREQ", "GAIN", "FREQ", "GAIN", "FREQ", "GAIN", "FREQ", "GAIN"], "EQ: Freq / Gain");
    if let Some(dir) = render_dir() {
        h.render().unwrap().save(dir.join("fx-eq.png")).unwrap();
    }
    layout::apply_acts(h.state_mut(), vec![layout::Act::PluginPage(1)]);
    h.run_steps(3);
    assert_eq!(names(&h), ["WIDTH", "WIDTH", "GAIN"], "EQ: Width / Output");
    layout::apply_acts(h.state_mut(), vec![layout::Act::SelectSlot(Some(2))]);
    h.run_steps(3);
    assert_eq!(names(&h), ["MODE", "THRESHOLD", "KNEE", "AMOUNT", "ATTACK", "RELEASE", "GAIN"], "Compressor");
    if let Some(dir) = render_dir() {
        h.render().unwrap().save(dir.join("fx-compressor.png")).unwrap();
    }
    // A knob edits the module's parameter.
    layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Module(0, 0, 2), "threshold", 0.5)]);
    h.run_steps(2);
    assert_eq!(project_of(&h).groups[0].sounds[0].slots[2].param("threshold"), Some(0.5));
    assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_label(), Some("Effect Parameter"));
    // The Side-Chain Input page: pick the snares as the source; the filter's knobs.
    layout::apply_acts(h.state_mut(), vec![layout::Act::PluginPage(1)]);
    h.run_steps(3);
    assert_eq!(names(&h), ["SOURCE", "GAIN", "FILTER", "CENTER FREQ", "WIDTH"]);
    let k: Vec<(String, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap();
    let src = k[0].1;
    click(&mut h, Pos2::new(src.center().x, src.top() + 42.0));
    if let Some(dir) = render_dir() {
        h.render().unwrap().save(dir.join("fx-sidechain-list.png")).unwrap();
    }
    h.get_by_label("B1 - Snares").click();
    h.run_steps(3);
    assert_eq!(project_of(&h).groups[0].sounds[0].channel.sidechain, vec![(2, deckard_model::ScSource::Group(1))]);
    if let Some(dir) = render_dir() {
        h.render().unwrap().save(dir.join("fx-sidechain.png")).unwrap();
    }
    layout::apply_acts(h.state_mut(), vec![layout::Act::SetParam(deckard_doc::Target::Module(0, 0, 2), "side_chain_filter/filter", 1.0)]);
    h.run_steps(2);
    assert_eq!(project_of(&h).groups[0].sounds[0].slots[2].param("side_chain_filter/filter"), Some(1.0));
}

/// Where the ignored render tests (and others, optionally) save their screenshots.
/// Pad velocity plays as Maschine does: hits recorded from Maschine 3 on an MK3 (the
/// pressure the MK3 reported, the velocity Maschine sent; docs/mk3-usb.md) come out
/// within a velocity step, two at each velocity scaling.
#[test]
fn pad_velocity_matches_maschine() {
    let velocity = |pressure: u16, scaling| (mk3_io::pad_velocity(f64::from(pressure) / 4095.0, scaling) * 127.0).round() as i64;
    let heard = [
        (-3, 275, 53),
        (-3, 2123, 117),
        (-2, 331, 51),
        (-2, 899, 80),
        (-1, 335, 45),
        (-1, 2789, 111),
        (0, 255, 31),
        (0, 1324, 72),
        (1, 257, 26),
        (1, 1557, 67),
        (2, 229, 19),
        (2, 1774, 60),
        (3, 252, 10),
        (3, 1624, 39),
    ];
    for (scaling, pressure, sent) in heard {
        let v = velocity(pressure, scaling);
        assert!((v - sent).abs() <= 1, "scaling {scaling}, pressure {pressure}: {v}, Maschine sent {sent}");
    }
}

/// Velocity SCALING: Soft lifts every press short of full, Hard holds it back, more
/// at each step; full pressure stays full. The sensitivity isn't in it: the MK3 applies
/// that to the pressure it reports.
#[test]
fn velocity_scaling_bends_the_presses() {
    use mk3_io::pad_velocity;
    let at = |scaling| pad_velocity(0.3, scaling);
    let all: Vec<f64> = (-3..=3).map(at).collect();
    assert!(all.windows(2).all(|w| w[0] > w[1]), "{all:?}");
    for scaling in -3..=3 {
        assert_eq!(pad_velocity(1.0, scaling), 1.0);
        assert_eq!(pad_velocity(0.0, scaling), 0.0);
    }
    assert_eq!(settings::Settings::default().pad_sensitivity, 50);
}

/// A knob on the controller's plug-in page goes the whole way in one turn, however
/// finely the MK3 reports it (~1000 steps a turn; here 200): every continuous knob of every
/// built-in plug-in's first page.
#[test]
fn a_controller_knob_turn_spans_the_range() {
    use deckard_lua_host::state::ControlRequest as R;
    let mut slow = Vec::new();
    let choices = App::default().plugin_choices();
    for (entry, what) in choices.into_iter().filter(|(e, _)| e.format == deckard_lua_host::state::PluginFormat::Internal) {
        let mut app = App::default();
        app.new_project();
        app.add_slot_at(what, 0);
        app.slot = Some(0);
        let knobs = |app: &App| {
            let doc = app.doc.clone().unwrap();
            layout::slot_page(app, doc.borrow().project(), 0)
                .map(|pg| {
                    pg.knobs
                        .iter()
                        .map(|k| {
                            k.as_ref().map(|k| (k.0.clone(), k.2, matches!(k.3, Some(layout::Edit::Param(..) | layout::Edit::Log(..)))))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        for (pos, k) in knobs(&app).into_iter().enumerate() {
            let Some((name, _, true)) = k else { continue };
            for inc in [-0.005, 0.005] {
                for _ in 0..200 {
                    app.control_request(R::Turn { pos, inc, fine: false });
                }
            }
            let v = knobs(&app)[pos].as_ref().map_or(0.0, |k| k.1);
            if v < 0.99 {
                slow.push(format!("{} {name}: {v:.3}", entry.name));
            }
        }
    }
    assert!(slow.is_empty(), "short of the top after a turn up from the bottom:\n{}", slow.join("\n"));
}

/// CHANNEL on the controller: the channel page's knobs; a routing choice shows
/// shortened, whole while its knob is touched, and turning it steps through them.
#[test]
fn controller_channel_routes_show_short_and_turn() {
    use deckard_lua_host::state::ControlRequest as R;
    let mut app = App::default();
    app.new_project();
    app.settings.output_routing[2..4].copy_from_slice(&[Some(2), Some(3)]);
    app.settings.output_aliases[2] = "Studio Monitors".into();
    app.settings.output_aliases[3] = "Studio Monitors".into();
    (app.channel, app.chain_tab, app.prop_set, app.prop_page) = (true, layout::ChainTab::Sound, 1, 0);
    let dest = |app: &App| {
        let doc = app.doc.clone().unwrap();
        let (_, _, pg) = layout::channel_page(app, &doc.borrow()).unwrap();
        pg.slots.iter().position(|k| matches!(k, Some((n, ..)) if n.as_ref() == "DEST.")).unwrap()
    };
    let k = dest(&app);
    let text = |app: &App, touched: bool| {
        let mut t = [false; 8];
        t[k] = touched;
        app.control_snapshot(t).knobs[k].as_ref().unwrap().text.clone()
    };
    assert_eq!(text(&app, false), "Group");
    // Up through Master and Ext. 1 to the aliased Out 2.
    for _ in 0..3 {
        app.control_request(R::Turn { pos: k, inc: ROUTE_STEP, fine: false });
    }
    assert_eq!(text(&app, false), "Stu..ors", "shortened once picked");
    assert_eq!(text(&app, true), "Studio Monitors", "whole while touched");
    let snd = &project_of_app(&app).groups[0].sounds[0];
    assert_eq!(snd.channel.dest, deckard_model::AudioDest::Ext(1));
}

/// The open project is kept as it's edited (AUTO BACKUP): after a crash the next
/// start brings it back as it was, still unsaved.
#[test]
fn a_crash_brings_back_the_unsaved_session() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("session");
    let mut app = App { session_dir: Some(dir.clone()), ..Default::default() };
    app.new_project();
    app.doc.as_ref().unwrap().borrow_mut().set_tempo(97.0).unwrap();
    app.keep_session();
    app.session_written();
    let mut next = App { session_dir: Some(dir.clone()), ..Default::default() };
    next.start_project();
    let d = next.doc.as_ref().unwrap().borrow();
    assert_eq!(d.project().tempo, 97.0);
    assert!(d.is_dirty(), "still unsaved");
    assert_eq!(d.path(), None, "never saved: untitled");
    drop(d);
}

/// #5: quitting with unsaved changes asks Save / Discard, as Maschine does, and a
/// discarded change doesn't come back: the next start opens the project as saved
/// (RELOAD), or a new one if it never was.
/// BUGS #10: File ▸ New starts from the default project when one is set: its
/// content, untitled and unchanged; without one (or if it's gone) an empty project.
#[test]
fn new_starts_from_the_default_project() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let file = dir.join("Template.mxprj");
    let mut app = App::default();
    app.new_project();
    app.doc.as_ref().unwrap().borrow_mut().set_tempo(87.0).unwrap();
    app.doc.as_ref().unwrap().borrow_mut().save_as(&file).unwrap();
    let mut app = App::default();
    app.settings.default_project = Some(file.clone());
    app.new_project();
    {
        let d = app.doc.as_ref().unwrap().borrow();
        assert_eq!((d.project().tempo, d.is_dirty(), d.path(), app.path.clone()), (87.0, false, None, None), "the template's, untitled");
    }
    std::fs::remove_file(&file).unwrap();
    app.new_project();
    assert_ne!(app.doc.as_ref().unwrap().borrow().project().tempo, 87.0, "gone: an empty project");
}

#[test]
fn quitting_asks_and_discarded_changes_stay_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let file = dir.join("Song.mxprj");
    let mut app = App { session_dir: Some(dir.clone()), ..Default::default() };
    app.new_project();
    app.doc.as_ref().unwrap().borrow_mut().set_tempo(96.0).unwrap();
    app.doc.as_ref().unwrap().borrow_mut().save_as(&file).unwrap();
    app.path = Some(file.clone());
    app.doc.as_ref().unwrap().borrow_mut().set_tempo(95.0).unwrap();
    assert!(app.asks_on_quit(), "unsaved: asks");
    // Discard: quit as it is.
    app.end_session();
    let tempo = |dir: &std::path::Path| {
        let mut next = App { session_dir: Some(dir.to_path_buf()), ..Default::default() };
        next.start_project();
        let d = next.doc.as_ref().unwrap().borrow();
        (d.project().tempo, d.is_dirty(), d.path().map(Path::to_path_buf))
    };
    assert_eq!(tempo(&dir), (96.0, false, Some(file.clone())), "as saved");
    // Never saved: a new project.
    let mut app = App { session_dir: Some(dir.clone()), ..Default::default() };
    app.new_project();
    app.doc.as_ref().unwrap().borrow_mut().set_tempo(94.0).unwrap();
    app.end_session();
    assert_eq!(tempo(&dir), (Document::new_project().unwrap().project().tempo, false, None));
}

/// Pads start in Fixed Velocity (100): a project opens with it on, unchanged (no
/// undo step, nothing to save).
#[test]
fn projects_open_in_fixed_velocity() {
    let mut app = App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    let d = doc.borrow();
    assert_eq!(d.num(deckard_doc::Target::Song, "pad_velocity_mode"), Some(1.0));
    assert_eq!(d.num(deckard_doc::Target::Song, "fixed_velocity"), Some(100.0));
    assert!(!d.is_dirty() && d.undo_label().is_none(), "{d:?}");
}

/// Every built-in plug-in the controller's browser offers has DECKARD's pictures (run
/// `make_plugin_pictures` after adding one), and they decode.
#[test]
fn every_builtin_plugin_has_pictures() {
    let app = App::default();
    let keys: Vec<&str> = plugin_pictures::PICTURES.iter().map(|(k, ..)| *k).collect();
    for (entry, _) in app.plugin_choices().into_iter().filter(|(e, _)| e.format == deckard_lua_host::state::PluginFormat::Internal) {
        let key = deckard_lua_host::plugin_picture_key(&entry.name);
        assert!(keys.contains(&key.as_str()), "{} ({key}) has no picture", entry.name);
    }
    let look = deckard_nwl::draw::Look::empty();
    for (key, thumb, logo) in plugin_pictures::PICTURES {
        assert!(look.add_picture("t", thumb) && look.add_picture("l", logo), "{key}");
        assert_eq!(look.picture_size("l"), Some((240, 196)), "{key}");
    }
}

#[test]
fn ni_resources_pictures_resolve_slots_and_browser_products() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, thumb_png, logo_png) = plugin_pictures::PICTURES[0];
    let omni_art = tmp.path().join("omnisphere");
    let omni_vendor = tmp.path().join("spectrasonics").join("omnisphere");
    let kontakt = tmp.path().join("kontakt");
    let monark = tmp.path().join("monark");
    let prism = tmp.path().join("prism");
    for d in [&omni_art, &omni_vendor, &kontakt, &monark, &prism] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(omni_art.join("MST_artwork.png"), thumb_png).unwrap();
    std::fs::write(omni_vendor.join("MST_plugin.png"), thumb_png).unwrap();
    std::fs::write(omni_vendor.join("MST_logo.png"), logo_png).unwrap();
    std::fs::write(kontakt.join("MST_artwork.png"), thumb_png).unwrap();
    std::fs::write(kontakt.join("MST_logo.png"), logo_png).unwrap();
    std::fs::write(monark.join("MST_plugin.png"), thumb_png).unwrap();
    std::fs::write(monark.join("MST_logo.png"), logo_png).unwrap();
    std::fs::write(prism.join("MST_plugin.png"), thumb_png).unwrap();
    std::fs::write(prism.join("MST_logo.png"), logo_png).unwrap();

    let prev = std::env::var_os("DECKARD_NI_IMAGE_DIR");
    // SAFETY: scoped to this unit test; `with_ni_pictures` rescans when the path changes.
    unsafe { std::env::set_var("DECKARD_NI_IMAGE_DIR", tmp.path()) };
    let s = deckard_surface::Surface::new();
    assert_eq!(plugin_pictures::slot_key(&s, "Omnisphere").as_deref(), Some("omnisphere"));
    assert!(s.has_picture("omnisphere", true) && s.has_picture("omnisphere", false));
    assert_eq!(plugin_pictures::slot_key(&s, "Kontakt 7").as_deref(), Some("kontakt7"));
    assert!(s.has_picture("kontakt7", true) && s.has_picture("kontakt7", false));
    assert_eq!(plugin_pictures::slot_key(&s, "Monark").as_deref(), Some("monark"));
    assert!(s.has_picture("monark", true) && s.has_picture("monark", false));
    assert_eq!(plugin_pictures::slot_key(&s, "Prism FX").as_deref(), Some("prismfx"));
    assert!(s.has_picture("prismfx", true) && s.has_picture("prismfx", false));
    let mut app = App::default();
    app.new_project();
    app.load_resolved_preset(
        "Bass Sting",
        (0, 0),
        deckard_plugins::nks::Resolved {
            id: "Native Instruments:VST3:INSTR:5653544E6952367265616B746F722036".into(),
            name: "Monark".into(),
            chunk: Vec::new(),
            pages: Vec::new(),
        },
    )
    .unwrap();
    let doc = app.doc.as_ref().unwrap().borrow();
    let slot = &doc.project().groups[0].sounds[0].slots[0];
    assert_eq!(slot.name, "Monark");
    assert_eq!(plugin_pictures::slot_key(&s, &slot.name).as_deref(), Some("monark"));
    plugin_pictures::ensure_browser_product(&s, "Omnisphere");
    assert!(s.has_picture("artwork:omnisphere", true) && s.has_picture("artwork:omnisphere", false));
    match prev {
        Some(v) => unsafe { std::env::set_var("DECKARD_NI_IMAGE_DIR", v) },
        None => unsafe { std::env::remove_var("DECKARD_NI_IMAGE_DIR") },
    }
}
