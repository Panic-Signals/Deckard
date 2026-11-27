use super::*;

/// The divider above the chain area: dragged, the patterns and scenes take less or more
/// of the window and everything under them the rest; double-clicked, back as it was.
#[test]
fn the_song_divider_resizes_what_is_above_and_below_it() {
    let Some(mut h) = harness(false, false) else { return };
    h.run_steps(3);
    let top = |h: &Harness<'static, App>| h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("deckard-grid"))).expect("grid drawn");
    // (Where the divider is now, across the middle of the window.)
    let at = |h: &Harness<'static, App>| {
        h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("deckard-song-divider"))).expect("divider drawn").center()
    };
    let grid0 = top(&h);
    assert_eq!(h.state().settings.song_panel_h, None, "the window's size decides");
    // Up: less above, and the editor grows by as much.
    let from = at(&h);
    drag(&mut h, from, from - egui::vec2(0.0, 100.0));
    assert_eq!(h.state().settings.song_panel_h, Some(243.0));
    assert!((at(&h).y - (from.y - 100.0)).abs() < 0.5, "it followed the pointer");
    assert!((top(&h).top() - (grid0.top() - 100.0)).abs() < 0.5, "{:?}", top(&h));
    assert!((top(&h).height() - (grid0.height() + 100.0)).abs() < 0.5, "{:?}", top(&h));
    assert_eq!(top(&h).bottom(), grid0.bottom(), "its foot stays");
    // As far up as it goes.
    let from = at(&h);
    drag(&mut h, from, Pos2::new(from.x, 0.0));
    assert_eq!(h.state().settings.song_panel_h, Some(147.0));
    if let Ok(dir) = std::env::var("DECKARD_SHOTS") {
        h.render().unwrap().save(Path::new(&dir).join("divider-up.png")).unwrap();
    }
    // As far down: the editor keeps its room.
    let from = at(&h);
    drag(&mut h, from, Pos2::new(from.x, 2000.0));
    let down = h.state().settings.song_panel_h.unwrap();
    assert!(down > 343.0, "{down}");
    assert!(top(&h).height() > 30.0, "{:?}", top(&h));
    if let Ok(dir) = std::env::var("DECKARD_SHOTS") {
        h.render().unwrap().save(Path::new(&dir).join("divider-down.png")).unwrap();
    }
    // Clicked without moving: stays. Double-clicked: as it was.
    let from = at(&h);
    click(&mut h, from);
    assert_eq!(h.state().settings.song_panel_h, Some(down));
    click(&mut h, from);
    assert_eq!(h.state().settings.song_panel_h, None);
    h.run_steps(2);
    assert_eq!(top(&h), grid0);
}

/// The Edit tab always shows the play range, the selection and the loop, in samples; a
/// sample comes up selected; a value can be typed to the sample.
#[test]
fn the_edit_tab_shows_and_types_its_ranges_in_samples() {
    let Some(mut h) = harness(false, true) else { return };
    (h.state_mut().group, h.state_mut().sound) = (0, 0);
    h.run_steps(3);
    let panel_top = 33.0 + 3.0 + 343.0 + 2.0 + 140.0 + 2.0;
    click(&mut h, Pos2::new(3.0 + 16.0, panel_top + 7.0 + 4.0 * 28.0 + 12.0));
    h.run_steps(3);
    let zone = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].clone();
    let (lo, hi) = zone(&h).play_range;
    assert_eq!(h.state().sampling.selection, Some((lo as usize, hi as usize + 1)), "the play range");
    let at = |h: &Harness<'static, App>, group: &str, label: &str| -> egui::Rect {
        let id = egui::Id::new(("deckard-sample-field", group.to_string(), label.to_string()));
        h.ctx.data(|d| d.get_temp(id)).unwrap_or_else(|| panic!("{group} {label} shown"))
    };
    for (group, label) in [
        ("PLAY RANGE", "START"),
        ("PLAY RANGE", "END"),
        ("SELECTION RANGE", "START"),
        ("SELECTION RANGE", "END"),
        ("LOOP", "ACTIVE"),
        ("LOOP", "START"),
        ("LOOP", "END"),
        ("LOOP", "XFADE"),
    ] {
        at(&h, group, label);
    }
    let typed = |h: &mut Harness<'static, App>, r: egui::Rect, text: &str| {
        h.run_steps(40);
        let c = r.center();
        h.hover_at(c);
        h.step();
        for _ in 0..2 {
            for pressed in [true, false] {
                h.event(Event::PointerButton { pos: c, button: PointerButton::Primary, pressed, modifiers: Default::default() });
                h.step();
            }
        }
        h.run_steps(2);
        h.event(Event::Text(text.into()));
        h.step();
        h.event(Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        h.run_steps(3);
    };
    // To the sample, however long the sample is.
    let r = at(&h, "PLAY RANGE", "START");
    typed(&mut h, r, "1237");
    assert_eq!(zone(&h).play_range.0, 1237);
    let r = at(&h, "LOOP", "END");
    typed(&mut h, r, "9011");
    assert_eq!(zone(&h).loop_range.1, 9011);
    let r = at(&h, "LOOP", "START");
    typed(&mut h, r, "1298");
    assert_eq!(zone(&h).loop_range.0, 1298);
    assert_eq!(zone(&h).play_range.0, 1237, "the start stays 61 samples before the loop");
    let r = at(&h, "LOOP", "XFADE");
    typed(&mut h, r, "500");
    assert_eq!(zone(&h).x_fade, 500);
    let r = at(&h, "SELECTION RANGE", "END");
    typed(&mut h, r, "5000");
    assert_eq!(h.state().sampling.selection.map(|s| s.1), Some(5001), "both ends in, as it reads");
    if let Ok(dir) = std::env::var("DECKARD_SHOTS") {
        h.render().unwrap().save(Path::new(&dir).join("sample-edit.png")).unwrap();
    }
    // Nothing selected: still there, and an end selects from the play range.
    let wave: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-sample-wave"))).expect("waveform drawn");
    click(&mut h, Pos2::new(wave.center().x, wave.center().y));
    assert_eq!(h.state().sampling.selection, None, "deselected");
    at(&h, "SELECTION RANGE", "START");
    let r = at(&h, "SELECTION RANGE", "END");
    typed(&mut h, r, "6000");
    assert_eq!(h.state().sampling.selection, Some((1237, 6001)));
}

/// A sampled sound opens on the zone it was saved focused on, not always the first.
#[test]
fn a_sound_opens_on_its_saved_zone() {
    let Some(mut h) = harness(false, false) else { return };
    let p = project_of(&h);
    let found = p.groups.iter().enumerate().find_map(|(g, x)| {
        x.sounds.iter().enumerate().find_map(|(s, y)| y.sampler.as_ref().filter(|z| z.focused_zone > 0).map(|z| (g, s, z.focused_zone)))
    });
    let Some((g, s, saved)) = found else {
        eprintln!("no sound saved on a later zone; skipping");
        return;
    };
    (h.state_mut().group, h.state_mut().sound) = (g, s);
    h.run_steps(3);
    assert_eq!(h.state().zone, saved);
    // Focused by hand, it stays while the sound is focused.
    h.state_mut().zone = 0;
    h.run_steps(3);
    assert_eq!(h.state().zone, 0);
}

/// EDIT's MUNGE bar: the tool opens it, the eighths switch off and on, CANCEL closes
/// it; a New Sampler's zone bar (with TAIL and MOTION) is drawn over it.
#[test]
fn the_munge_bar_opens_and_its_eighths_switch() {
    let Some(mut h) = harness(false, true) else { return };
    (h.state_mut().group, h.state_mut().sound) = (0, 0);
    h.run_steps(3);
    let panel_top = 33.0 + 3.0 + 343.0 + 2.0 + 140.0 + 2.0;
    click(&mut h, Pos2::new(3.0 + 16.0, panel_top + 7.0 + 4.0 * 28.0 + 12.0));
    h.run_steps(3);
    h.state_mut().convert_instrument(true);
    // (Converted, it is in Original, which has no MUNGE: Advanced has.)
    h.state_mut()
        .arrange_act(layout::Act::Sampling(layout::sampling::SamplingAct::Sampler(deckard_model::new_sampler::SamplerMode::Advanced)));
    h.state_mut().arrange_act(layout::Act::Sampling(layout::sampling::SamplingAct::Tab(1)));
    h.run_steps(3);
    let tool = |h: &Harness<'static, App>, id: &str| -> egui::Rect {
        h.ctx.data(|d| d.get_temp(egui::Id::new(("deckard-tool", id.to_string())))).expect("tool drawn")
    };
    let r = tool(&h, "MUNGE");
    click(&mut h, r.center());
    h.run_steps(2);
    assert!(h.state().sampling.munge.is_some(), "the bar is open");
    let r = tool(&h, "munge-3");
    click(&mut h, r.center());
    h.run_steps(2);
    assert_eq!(h.state().sampling.munge.unwrap().mutes, [false, false, true, false, false, false, false, false]);
    let r = tool(&h, "munge-3");
    click(&mut h, r.center());
    h.run_steps(2);
    assert!(!h.state().sampling.munge.unwrap().changes(), "and on again");
    let r = tool(&h, "munge-cancel");
    click(&mut h, r.center());
    h.run_steps(2);
    assert!(h.state().sampling.munge.is_none());
}

/// The sample's zooming scroll bar, as the Pattern Editor's: up zooms in round where
/// it was grabbed, along scrolls, an end moves with the other held, double-click shows
/// it all.
#[test]
fn zoom_bar_zooms_and_scrolls_the_sample() {
    let Some(mut h) = harness(false, true) else { return };
    (h.state_mut().group, h.state_mut().sound) = (0, 0);
    h.run_steps(3);
    let panel_top = 33.0 + 3.0 + 343.0 + 2.0 + 140.0 + 2.0;
    click(&mut h, Pos2::new(3.0 + 16.0, panel_top + 7.0 + 4.0 * 28.0 + 12.0));
    h.run_steps(3);
    assert!(h.state().sampling.sample.is_some(), "the kick's sample is shown");
    let wave: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-sample-wave"))).expect("waveform drawn");
    let bar_y = wave.bottom() + 1.0 + 8.0;
    let len = h.state().sampling.len() as f64;
    let window = |h: &Harness<'static, App>| {
        let (a, b) = h.state().sampling.view();
        (a / len, (b - a) / len)
    };
    assert_eq!(h.state().sampling.view, None, "all of it");
    // Up from the middle: zoomed in, still round the middle.
    let mid = Pos2::new(wave.center().x, bar_y);
    drag(&mut h, mid, mid - egui::vec2(0.0, 60.0));
    let (from, part) = window(&h);
    assert!(part < 0.6, "zoomed in: {part}");
    assert!((from + part / 2.0 - 0.5).abs() < 0.05, "round the middle: {from} {part}");
    // Along: scrolled right, as far in.
    let thumb = Pos2::new(wave.left() + ((from + part / 2.0) as f32) * wave.width(), bar_y);
    drag(&mut h, thumb, thumb + egui::vec2(40.0, 0.0));
    assert!(window(&h).0 > from, "scrolled");
    assert!((window(&h).1 - part).abs() < 1e-4, "as far in");
    // Its right end, in: the left stays.
    let (from, part) = window(&h);
    let end = Pos2::new(wave.left() + ((from + part) as f32) * wave.width() - 2.0, bar_y);
    drag(&mut h, end, end - egui::vec2(30.0, 0.0));
    assert!((window(&h).0 - from).abs() < 1e-4, "the left end held");
    assert!(window(&h).1 < part, "shorter");
    // Double-click: all of it again.
    let (from, part) = window(&h);
    let thumb = Pos2::new(wave.left() + ((from + part / 2.0) as f32) * wave.width(), bar_y);
    for _ in 0..2 {
        click(&mut h, thumb);
    }
    h.run_steps(2);
    assert_eq!(h.state().sampling.view, None, "reset");
    // Up and up: no further in than the waveform goes.
    drag(&mut h, mid, mid - egui::vec2(0.0, 900.0));
    let (a, b) = h.state().sampling.view();
    assert!(b - a >= 63.0 && b - a < len, "{a} {b}");
}

#[test]
fn sampling_edit_and_slice() {
    let Some(mut h) = harness(false, true) else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    // Edited samples go next to the project: here, a temporary folder.
    h.state_mut().path = Some(dir.join("x.mxprj"));
    (h.state_mut().group, h.state_mut().sound) = (0, 0);
    h.run_steps(3);
    let panel_top = 33.0 + 3.0 + 343.0 + 2.0 + 140.0 + 2.0;
    // The strip's fifth button (after Erase, BUGS #17).
    click(&mut h, Pos2::new(3.0 + 16.0, panel_top + 7.0 + 4.0 * 28.0 + 12.0));
    assert!(h.state().sampling_open);
    h.run_steps(3);
    assert!(h.state().sampling.sample.is_some(), "the kick's sample is shown");
    let wave: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-sample-wave"))).expect("waveform drawn");
    // Select from 40 % to 70 % of the waveform (between the S and E markers).
    let (a, b) = (wave.left() + wave.width() * 0.4, wave.left() + wave.width() * 0.7);
    drag(&mut h, egui::pos2(a, wave.center().y), egui::pos2(b, wave.center().y));
    let sel = h.state().sampling.selection.expect("a selection");
    let len = h.state().sampling.len();
    assert!((sel.0 as f64 / len as f64 - 0.4).abs() < 0.02 && (sel.1 as f64 / len as f64 - 0.7).abs() < 0.02, "{sel:?} of {len}");
    // The S flag at the top of the start line: the pointer shows it can be dragged
    // (not over the plain waveform), and dragging it moves the start to 20 %.
    let cursor_at = |h: &mut Harness<'static, App>, p: Pos2| {
        h.hover_at(p);
        h.step();
        h.output().platform_output.cursor_icon
    };
    let lo = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].play_range.0;
    assert_eq!(lo, 0, "the kick plays from its start");
    let flag = egui::pos2(wave.left() + 10.0, wave.top() + 7.0);
    assert_eq!(cursor_at(&mut h, flag), egui::CursorIcon::ResizeHorizontal);
    assert_eq!(cursor_at(&mut h, egui::pos2(wave.left() + wave.width() * 0.5, wave.center().y)), egui::CursorIcon::Default);
    drag(&mut h, flag, egui::pos2(wave.left() + wave.width() * 0.2, wave.top() + 7.0));
    let lo = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].play_range.0 as f64;
    assert!((lo / len as f64 - 0.2).abs() < 0.02, "the start moved to {lo} of {len}");
    // NORM.
    let tool = |h: &Harness<'static, App>, id: &str| -> egui::Rect {
        h.ctx.data(|d| d.get_temp(egui::Id::new(("deckard-tool", id.to_string())))).expect("tool drawn")
    };
    let before = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].clone();
    let r = tool(&h, "NORM.");
    click(&mut h, r.center());
    let z = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0].clone();
    assert!(z.sample.path.ends_with(" - 2.wav") && z.sample.path.starts_with(dir.to_str().unwrap()), "{}", z.sample.path);
    assert!(std::path::Path::new(&z.sample.path).exists());
    h.run_steps(3);
    let edited = h.state().sampling.sample.clone().unwrap();
    let peak = edited.frames[sel.0..sel.1].iter().fold(0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    assert!((peak - 1.0).abs() < 1e-3, "normalized: {peak}");
    h.state_mut().undo();
    h.run_steps(3);
    assert_eq!(project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones[0], before, "undo restores the original sample");
    // SLICE: Split into 8, APPLY.
    h.state_mut().arrange_act(layout::Act::Sampling(layout::sampling::SamplingAct::Tab(2)));
    h.run_steps(3);
    assert_eq!(h.state().sampling.slices.len(), 8);
    let r = tool(&h, "apply");
    click(&mut h, r.center());
    let zones = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones;
    assert_eq!(zones.len(), 8);
    assert_eq!(zones.iter().map(|z| z.root_key).collect::<Vec<_>>(), (60..68).collect::<Vec<_>>());
    assert!(zones.windows(2).all(|w| w[0].play_range.1 + 1 == w[1].play_range.0), "contiguous slices");
    assert_eq!(h.state().sampling.tab, 3, "then the Zone tab");
    h.run_steps(3);
    let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-zone-grid"))).expect("zone map drawn");
    assert!(grid.width() > 100.0);
    // Back on SLICE, from another slice's zone: the slices applied show, not a new split.
    h.state_mut().zone = 5;
    h.run_steps(3);
    h.state_mut().arrange_act(layout::Act::Sampling(layout::sampling::SamplingAct::Tab(2)));
    h.run_steps(3);
    let starts: Vec<usize> = zones.iter().map(|z| z.play_range.0 as usize).collect();
    assert_eq!(h.state().sampling.slices, starts, "the slices it was cut at");
    assert_eq!(h.state().sampling.slice_mode, 3, "kept as set (Manual)");
    // Another slice's pad: they stay.
    h.state_mut().zone = 2;
    h.run_steps(3);
    assert_eq!(h.state().sampling.slices, starts);
    // One taken out and applied again: seven zones, over the same frames.
    h.state_mut().sampling.slices.remove(3);
    let r = tool(&h, "apply");
    click(&mut h, r.center());
    let again = project_of(&h).groups[0].sounds[0].sampler.clone().unwrap().zones;
    assert_eq!(again.len(), 7);
    assert_eq!((again[0].play_range.0, again[6].play_range.1), (zones[0].play_range.0, zones[7].play_range.1));
}

/// SLICE in the software: CUE and Enter catch a slice where the sample plays, RE-ROLL
/// picks again, a right-click keeps a slice or key shifts it.
#[test]
fn slice_cue_reroll_and_the_slice_menu() {
    use layout::sampling::SamplingAct as S;
    let (app, _tmp) = crate::surface_sampling_tests::app();
    let mut h = harness_of(app, false);
    let act = |h: &mut Harness<'static, App>, a: S| {
        h.state_mut().arrange_act(layout::Act::Sampling(a));
        h.run_steps(3);
    };
    let tool = |h: &Harness<'static, App>, id: &str| -> Option<egui::Rect> {
        h.ctx.data(|d| d.get_temp(egui::Id::new(("deckard-tool", id.to_string()))))
    };
    act(&mut h, S::Tab(2));
    act(&mut h, S::SliceMode(3));
    act(&mut h, S::ClearSlices);
    // Nothing plays: CUE does nothing.
    let cue = tool(&h, "cue").expect("CUE drawn");
    click(&mut h, cue.center());
    assert!(h.state().sampling.slices.is_empty());
    // A slice's pad plays it from 20 000 on (held there: it starts in a while).
    let soon = std::time::Instant::now() + std::time::Duration::from_secs(600);
    h.state_mut().prehear_from = Some((soon, 20_000, 44_100));
    h.run_steps(2);
    assert_eq!(h.state().sampling.playhead, Some(20_000));
    click(&mut h, cue.center());
    assert_eq!(h.state().sampling.slices, [0, 20_000]);
    h.state_mut().prehear_from = Some((soon, 30_000, 44_100));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().sampling.slices, [0, 20_000, 30_000], "Enter catches one too");
    h.state_mut().prehear_from = None;
    h.run_steps(2);
    // Random: RE-ROLL moves its picks on; Manual has no such button.
    assert!(tool(&h, "reroll").is_none());
    act(&mut h, S::SliceMode(5));
    let reroll = tool(&h, "reroll").expect("RE-ROLL drawn");
    click(&mut h, reroll.center());
    assert_eq!(h.state().sampling.seed, 1);
    // Right-click in a slice: kept, then not; and key shifted.
    let wave: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-sample-wave"))).expect("waveform drawn");
    h.state_mut().sampling.slices = vec![0, 11_025, 22_050];
    h.run_steps(2);
    let in_second = egui::pos2(wave.left() + wave.width() * 0.4, wave.center().y);
    right_click(&mut h, in_second);
    h.get_by_label("Keep slice").click();
    h.run_steps(3);
    assert_eq!(h.state().sampling.kept, [11_025]);
    right_click(&mut h, in_second);
    h.get_by_label("Don't keep slice").click();
    h.run_steps(3);
    assert!(h.state().sampling.kept.is_empty());
    right_click(&mut h, in_second);
    h.get_by_label("Key Shift to new sound").click();
    h.run_steps(3);
    let p = project_of(&h);
    let made = p.groups[h.state().group].sounds.iter().find(|x| x.name.ends_with(" keys")).expect("the slice on a new sound");
    assert_eq!(made.sampler.as_ref().unwrap().zones[0].play_range, (11_025, 22_049));
    assert_eq!(h.state().sampling.slices, [0, 11_025, 22_050], "a right-click adds no slice");
}

/// With several notes selected, resizing one resizes all, and dragging one
/// velocity or probability bar changes all of them by as much.
#[test]
fn edits_apply_to_the_whole_selection() {
    let Some(mut h) = harness(false, true) else { return };
    let (g, pi) = (h.state().group, h.state().pattern);
    let start = project_of(&h).groups[g].patterns[pi].start;
    // Two notes of the focused sound, on the 16th grid.
    let sound = h.state().sound;
    let a = Note {
        ratchet: Default::default(),
        expr: None,
        sound,
        position: start + 960,
        length: 240,
        pitch: 60,
        velocity: 100,
        probability: 100,
    };
    let b = Note {
        ratchet: Default::default(),
        expr: None,
        sound,
        position: start + 2880,
        length: 240,
        pitch: 60,
        velocity: 80,
        probability: 100,
    };
    let existing: Vec<Note> = notes_of(&h).into_iter().filter(|n| n.sound == sound).collect();
    h.state().doc.as_ref().unwrap().borrow_mut().edit_notes("setup", g, pi, &existing, &[a, b]).unwrap();
    h.state_mut().selected = vec![a, b];
    h.run_steps(2);

    // Resize: drag a's right edge one beat right.
    let row = sound;
    let (from, to) = (grid_pos(&h, row, 960 + 240) - egui::vec2(2.0, 0.0), grid_pos(&h, row, 960 + 240 + 960) - egui::vec2(2.0, 0.0));
    drag(&mut h, from, to);
    let lens: Vec<i64> = notes_of(&h).iter().filter(|n| n.sound == sound).map(|n| n.length).collect();
    assert_eq!(lens, [1200, 1200], "both notes resized");
    assert_eq!(h.state().selected.len(), 2, "still selected");

    // Velocity: drag a's bar down by about 30 (relative: b goes down as much).
    let (bottom, lane_h) = lane(&h);
    let (vel_bottom, prob_bottom) = (bottom, bottom);
    let x = grid_pos(&h, 0, 960).x + 1.5;
    let y = |bottom: f32, v: f32, max: f32| bottom - lane_h * v / max;
    drag(&mut h, Pos2::new(x, y(vel_bottom, 100.0, 127.0)), Pos2::new(x, y(vel_bottom, 70.0, 127.0)));
    let vels: Vec<i64> = notes_of(&h).iter().filter(|n| n.sound == sound).map(|n| n.velocity).collect();
    assert!((60..=80).contains(&vels[0]), "{vels:?}");
    assert_eq!(vels[0] - vels[1], 20, "relative change keeps the difference: {vels:?}");

    // Probability: drag a's bar from 100 to about 50; b follows.
    pick_lane(&mut h, layout::Lane::Probability);
    drag(&mut h, Pos2::new(x, y(prob_bottom, 100.0, 100.0)), Pos2::new(x, y(prob_bottom, 50.0, 100.0)));
    let probs: Vec<i64> = notes_of(&h).iter().filter(|n| n.sound == sound).map(|n| n.probability).collect();
    assert!((40..=60).contains(&probs[0]) && probs[0] == probs[1], "{probs:?}");
}
