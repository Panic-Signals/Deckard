use super::*;

/// SCALE (next to QUANTIZE) sets the controller's scale, and the piano roll marks
/// the keys outside it.
#[test]
fn scale_menu_sets_the_scale_and_marks_the_piano_roll() {
    let Some(mut h) = scripted_harness(false) else { return };
    h.state_mut().piano_roll = true;
    h.run_steps(3);
    h.get_by_label("SCALE").click();
    h.run_steps(2);
    h.get_by_label_contains("Main").hover();
    h.run_steps(2);
    h.get_by_label("Blues").click();
    h.run_steps(3);
    let (root, bank, kind) = h.state().scale;
    assert_eq!((root.rem_euclid(12), bank, kind), (0, 0, 6), "C Blues");
    let c = h.state().controller.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(c.host.scale(), h.state().scale, "the controller's scale");
    h.get_by_label("C Blues").hover();
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().to_path_buf();
    h.render().unwrap().save(d.join("scale.png")).unwrap();
}

/// PAD TOUCH (next to SCALE) sets the focused sound's Pad Touch, as the MK3's
/// Keyboard page does, and shows the mode it's in.
#[test]
fn pad_touch_menu_sets_the_focused_sounds_pad_touch() {
    use deckard_model::pad_touch::TouchMode;
    let Some(mut h) = harness(false, false) else { return };
    h.run_steps(3);
    let (g, s) = (h.state().group, h.state().sound);
    let touch = |h: &Harness<'static, App>| h.state().doc.as_ref().unwrap().borrow().project().groups[g].sounds[s].pad_touch;
    h.get_by_label("PAD TOUCH").click();
    h.run_steps(2);
    h.get_by_label("Poly AT").click();
    h.run_steps(3);
    assert_eq!(touch(&h).mode, TouchMode::PolyAt);
    // The menu stays open for more changes (a click outside closes it).
    h.get_by_label("CC").click();
    h.run_steps(3);
    assert_eq!(touch(&h).mode, TouchMode::Cc);
    assert!(h.query_by_label("PAD: CC 1 Mod").is_some(), "the CC shows");
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().to_path_buf();
    h.render().unwrap().save(d.join("pad-touch.png")).unwrap();
    // Min turned above Max takes Max with it; Max below Min, Min.
    let at = |t| layout::Act::PadTouch(g, s, t);
    let t = touch(&h);
    layout::apply_acts(h.state_mut(), vec![at(deckard_model::pad_touch::PadTouch { min: 40, max: 30, ..t })]);
    assert_eq!((touch(&h).min, touch(&h).max), (40, 40));
    h.state_mut().doc.as_ref().unwrap().borrow_mut().undo();
    assert_eq!((touch(&h).mode, touch(&h).min, touch(&h).max), (TouchMode::Cc, 0, 100), "one undo step back");
}

/// Wheel input over either the sound names or drum lanes moves both together.
#[test]
fn sound_list_scrolls_with_drum_lanes() {
    for over_names in [true, false] {
        let mut h = blank_harness(false);
        h.state_mut().piano_roll = false;
        h.state_mut().mouse = layout::MouseMode::Draw;
        h.state().doc.as_ref().unwrap().borrow_mut().add_pattern(0, 0, "Pattern 1").unwrap();
        h.run_steps(3);
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        assert!(grid.height() < 16.0 * 24.0 - 48.0, "room to scroll two rows");
        let name = Pos2::new(grid.left() - 80.0, grid.top() + 12.0);
        let note = Pos2::new(grid.left() + 10.0, name.y);
        for (delta, sound) in [(-48.0, 2), (48.0, 0)] {
            h.hover_at(if over_names { name } else { grid.center() });
            h.step();
            h.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, delta),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(30);
            click(&mut h, name);
            assert_eq!(h.state().sound, sound, "sound name after scrolling {delta} over names={over_names}");
            click(&mut h, note);
            assert!(notes_of(&h).iter().any(|n| n.sound == sound), "the lane beside the name belongs to sound {sound}");
        }
    }
}

/// Piano-roll pitches are independent of the sound list's rows.
#[test]
fn sound_list_scroll_keeps_piano_roll_pitches_in_place() {
    let mut h = blank_harness(false);
    h.state_mut().piano_roll = true;
    h.run_steps(3);
    let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
    let key = egui::Id::new(("piano-zoom", h.state().group, h.state().sound));
    let pitches = h.ctx.data(|d| d.get_temp::<(f32, f32)>(key)).expect("pitches shown");
    let name = Pos2::new(grid.left() - 100.0, grid.top() + 12.0);
    h.hover_at(name);
    h.step();
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -48.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(30);
    assert_eq!(h.ctx.data(|d| d.get_temp::<(f32, f32)>(key)), Some(pitches), "scrolling names does not scroll pitches");
    click(&mut h, name);
    assert_eq!(h.state().sound, 2, "the sound list still scrolls in piano-roll mode");
}

/// The Pattern Editor's horizontal zooming scroll bar (as the manual has it): drag it
/// up to zoom in round where it was grabbed, along to scroll, double-click it to see
/// the whole pattern; ⌘ + scroll zooms too.
#[test]
fn zoom_bar_zooms_and_scrolls_the_pattern_editor() {
    let Some(mut h) = harness(false, false) else { return };
    h.run_steps(3);
    let (g, pi) = (h.state().group, h.state().pattern);
    let key = egui::Id::new(("pattern-hzoom", g, pi));
    let window = |h: &Harness<'static, App>| h.ctx.data(|d| d.get_temp::<(f32, f32)>(key)).expect("zoom kept");
    let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
    let lane: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-lane"))).expect("lane drawn");
    let bar_y = lane.bottom() + 1.0 + 6.0;
    assert_eq!(window(&h), (0.0, 1.0), "the whole pattern");
    // Up from the middle: zoomed in, still round the middle.
    let mid = Pos2::new(grid.center().x, bar_y);
    drag(&mut h, mid, mid - egui::vec2(0.0, 60.0));
    let (from, len) = window(&h);
    assert!(len < 0.6, "zoomed in: {len}");
    assert!((from + len / 2.0 - 0.5).abs() < 0.05, "round the middle: {from} {len}");
    // Along: scrolled right.
    let thumb = Pos2::new(grid.left() + (from + len / 2.0) * grid.width(), bar_y);
    drag(&mut h, thumb, thumb + egui::vec2(40.0, 0.0));
    assert!(window(&h).0 > from, "scrolled");
    // Double-click: all of it again.
    let thumb = Pos2::new(grid.left() + (window(&h).0 + window(&h).1 / 2.0) * grid.width(), bar_y);
    for _ in 0..2 {
        click(&mut h, thumb);
    }
    h.run_steps(2);
    assert_eq!(window(&h), (0.0, 1.0), "reset");
    // ⌘ + scroll over the notes zooms in.
    h.hover_at(grid.center());
    h.step();
    h.event(egui::Event::Zoom(2.0));
    h.run_steps(2);
    assert!((window(&h).1 - 0.5).abs() < 0.01, "{:?}", window(&h));
}

/// Dragging on the pattern editor's timeline: the right end sets the length, the
/// left end the start (the end stays), the span between them moves both.
#[test]
fn timeline_drags_set_pattern_start_and_length() {
    let Some(mut h) = harness(false, false) else { return };
    h.run_steps(3);
    let (g, pi) = (h.state().group, h.state().pattern);
    let span = |h: &Harness<'static, App>| {
        let p = project_of(h);
        let pat = &p.groups[g].patterns[pi];
        (pat.start, if pat.length > 0 { pat.length } else { p.bar_length() })
    };
    let bar = project_of(&h).bar_length();
    // The timeline is the strip just above the grid; t is from the pattern's start.
    let at = |h: &Harness<'static, App>, t: i64| {
        let p = grid_pos(h, 0, t);
        let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
        Pos2::new(p.x, grid.top() - 9.0)
    };
    let px_per_bar = |h: &Harness<'static, App>| at(h, bar).x - at(h, 0).x;
    let (s0, l0) = span(&h);
    assert_eq!(s0, 0);
    // Right end, one bar further: a bar longer.
    let (from, d) = (at(&h, l0), px_per_bar(&h));
    drag(&mut h, from, from + egui::vec2(d, 0.0));
    assert_eq!(span(&h), (0, l0 + bar), "LENGTH");
    // Left end, one bar in: the start moves, the end stays.
    let (from, d) = (at(&h, 0), px_per_bar(&h));
    drag(&mut h, from + egui::vec2(1.0, 0.0), from + egui::vec2(1.0 + d, 0.0));
    assert_eq!(span(&h), (bar, l0), "START");
    // The middle, one bar on: both move.
    let (from, d) = (at(&h, l0 / 2), px_per_bar(&h));
    drag(&mut h, from, from + egui::vec2(d, 0.0));
    assert_eq!(span(&h), (2 * bar, l0), "POSITION");
    // One undo step each.
    let labels: Vec<String> = h.state().doc.as_ref().unwrap().borrow().undo_labels().iter().rev().take(3).map(|s| s.to_string()).collect();
    assert_eq!(labels, ["Pattern Start"; 3]);
}

/// #22: the end dragged two bars past it (past where the grid ended) makes the
/// pattern two bars longer; the editor shows two bars past the end to drag into.
#[test]
fn the_pattern_end_drags_past_the_grid() {
    let Some(mut h) = harness(false, false) else { return };
    h.run_steps(3);
    let (g, pi) = (h.state().group, h.state().pattern);
    let length = |h: &Harness<'static, App>| {
        let p = project_of(h);
        let pat = &p.groups[g].patterns[pi];
        if pat.length > 0 { pat.length } else { p.bar_length() }
    };
    let bar = project_of(&h).bar_length();
    let grid: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-grid"))).expect("grid drawn");
    let at = |h: &Harness<'static, App>, t: i64| Pos2::new(grid_pos(h, 0, t).x, grid.top() - 9.0);
    let l0 = length(&h);
    let end = at(&h, l0);
    let two = at(&h, l0 + 2 * bar);
    assert!(two.x < grid.right(), "two bars past the end shown: {} < {}", two.x, grid.right());
    drag(&mut h, end, two);
    assert_eq!(length(&h), l0 + 2 * bar);
}

/// A note of a sound with none, a beat long at the pattern's start, focused: (its
/// sound, the note).
fn lone_note(h: &mut Harness<'static, App>) -> Option<(usize, Note)> {
    let p = project_of(h);
    let (g, pi) = (h.state().group, h.state().pattern);
    let before = &p.groups[g].patterns[pi].notes;
    let sound = (0..p.groups[g].sounds.len().min(8)).find(|s| before.iter().all(|n| n.sound != *s))?;
    let start = p.groups[g].patterns[pi].start;
    let n =
        Note { ratchet: Default::default(), expr: None, sound, position: start, length: 960, pitch: 60, velocity: 100, probability: 100 };
    h.state().doc.as_ref().unwrap().borrow_mut().edit_notes("t", g, pi, &[], &[n]).unwrap();
    h.state_mut().sound = sound;
    h.run_steps(3);
    Some((sound, n))
}

/// Pick Expression lane `l` in the "+" dialog (a sound without expression doesn't list
/// them yet).
fn pick_expression_lane(h: &mut Harness<'static, App>, l: layout::Lane) {
    assert!(!lane_list(h).iter().any(|(x, _)| *x == l), "not listed before the sound has expression");
    let plus = lane_list(h).last().unwrap().1;
    click(h, plus.center());
    assert!(h.state().lanes_dialog);
    let name = l.name();
    h.get_by_label(&name).click();
    h.run_steps(3);
    assert_eq!(h.state().lane, l);
    assert!(!h.state().lanes_dialog);
}

/// The Expression editor (part 3): a click in the Pressure lane inside a note adds a
/// point to its curve there; a drag moves it; undo takes the edits back.
#[test]
fn the_pressure_lane_adds_a_point() {
    let Some(mut h) = harness(false, true) else { return };
    let Some((sound, _)) = lone_note(&mut h) else { return };
    pick_expression_lane(&mut h, layout::Lane::Expr(Dim::Pressure));
    let (bottom, lane_h) = lane(&h);
    let x = grid_pos(&h, 0, 480).x;
    click(&mut h, Pos2::new(x, bottom - lane_h * 0.75));
    let e = expression_of(&h, sound).expect("the note has expression now");
    let pts = e.pressure.expect("a pressure curve").points;
    assert_eq!(pts.len(), 1, "{pts:?}");
    assert!((pts[0].at - 480).abs() <= 30 && (pts[0].value - 0.75).abs() < 0.05, "{pts:?}");
    let listed: Vec<layout::Lane> = lane_list(&h).iter().map(|(l, _)| *l).collect();
    // Listed now, alone of the expression lanes (BUGS #18: only what the notes have).
    let expr: Vec<layout::Lane> = listed.iter().copied().filter(|l| matches!(l, layout::Lane::Expr(_) | layout::Lane::Release)).collect();
    assert_eq!(expr, [layout::Lane::Expr(Dim::Pressure)], "listed now: {listed:?}");
    // Drag the point down to a quarter.
    let from = Pos2::new(grid_pos(&h, 0, pts[0].at).x, bottom - lane_h * pts[0].value);
    drag(&mut h, from, Pos2::new(from.x, bottom - lane_h * 0.25));
    let pts = expression_of(&h, sound).unwrap().pressure.unwrap().points;
    assert!(pts.len() == 1 && (pts[0].value - 0.25).abs() < 0.05, "moved: {pts:?}");
    let doc = h.state().doc.clone().unwrap();
    assert_eq!(doc.borrow().undo_label(), Some("Edit Expression"));
    while doc.borrow().undo_label() == Some("Edit Expression") {
        doc.borrow_mut().undo();
    }
    h.run_steps(2);
    assert_eq!(expression_of(&h, sound), None, "undone");
}

/// The Release lane: a click on a note's bar sets its release velocity.
#[test]
fn the_release_lane_sets_release_velocity() {
    let Some(mut h) = harness(false, true) else { return };
    let Some((sound, _)) = lone_note(&mut h) else { return };
    pick_expression_lane(&mut h, layout::Lane::Release);
    let (bottom, lane_h) = lane(&h);
    let x = grid_pos(&h, 0, 0).x + 1.5;
    click(&mut h, Pos2::new(x, bottom - lane_h * 0.5));
    let rv = expression_of(&h, sound).and_then(|e| e.release_velocity).expect("a release velocity");
    assert!((54..=74).contains(&rv), "about half: {rv}");
}

#[test]
fn draw_mode_edits_notes_and_lanes() {
    let Some(mut h) = harness(false, true) else { return };
    let p = project_of(&h);
    let (g, pi) = (h.state().group, h.state().pattern);
    let before = notes_of(&h);
    let start = p.groups[g].patterns[pi].start;
    let Some(sound) = (0..p.groups[g].sounds.len().min(8)).find(|s| before.iter().all(|n| n.sound != *s)) else { return };
    // The pencil, third button of the group panel's left strip.
    let panel_top = 33.0 + 3.0 + 343.0 + 2.0 + 140.0 + 2.0;
    click(&mut h, Pos2::new(3.0 + 16.0, panel_top + 7.0 + 2.0 * 28.0 + 12.0));
    assert!(h.state().draw_mode(), "the pencil turns draw mode on");

    // Click an empty step: a note.
    let pos = grid_pos(&h, sound, 2 * 240 + 120);
    click(&mut h, pos);
    let added = Note {
        ratchet: Default::default(),
        expr: None,
        sound,
        position: start + 480,
        length: 240,
        pitch: 60,
        velocity: 100,
        probability: 100,
    };
    assert!(notes_of(&h).contains(&added), "click adds a note");
    assert_eq!(h.state().sound, sound, "and focuses its sound");

    // Drag it one beat right.
    // (Grabbed just inside its start: its right end resizes.)
    let grab = egui::vec2(3.0, 0.0);
    let (from, to) = (grid_pos(&h, sound, 480) + grab, grid_pos(&h, sound, 480 + 960) + grab);
    drag(&mut h, from, to);
    let moved = Note { position: start + 1440, ..added };
    assert!(notes_of(&h).contains(&moved), "drag moves it: {:?}", notes_of(&h));

    // Draw its velocity and probability down to about half.
    let (bottom, lane_h) = lane(&h);
    let x = grid_pos(&h, 0, 1440).x + 1.5;
    drag(&mut h, Pos2::new(x - 6.0, bottom - lane_h * 0.5), Pos2::new(x + 6.0, bottom - lane_h * 0.5));
    pick_lane(&mut h, layout::Lane::Probability);
    drag(&mut h, Pos2::new(x - 6.0, bottom - lane_h * 0.5), Pos2::new(x + 6.0, bottom - lane_h * 0.5));
    pick_lane(&mut h, layout::Lane::Velocity);
    let n = notes_of(&h).into_iter().find(|n| n.sound == sound).unwrap();
    assert!((50..=78).contains(&n.velocity), "velocity drawn to about half: {n:?}");
    assert!((40..=60).contains(&n.probability), "probability drawn to about half: {n:?}");

    // Click the note: gone. Then paint four notes in one drag.
    let pos = grid_pos(&h, sound, 1440 + 60);
    click(&mut h, pos);
    assert!(notes_of(&h).iter().all(|n| n.sound != sound), "click on a note deletes it");
    let (a, b) = (grid_pos(&h, sound, 120), grid_pos(&h, sound, 3 * 240 + 120));
    drag(&mut h, a, b);
    let painted: Vec<i64> = notes_of(&h).iter().filter(|n| n.sound == sound).map(|n| n.position - start).collect();
    assert_eq!(painted, [0, 240, 480, 720], "dragging paints a note per step");
    assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_label(), Some("Draw Notes"));

    // Undo back to the start.
    while notes_of(&h) != before {
        assert!(h.state().doc.as_ref().unwrap().borrow().undo_label().is_some(), "undo history ran out");
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
    }
}

/// E / X / Y / Z switch Draw, Split, Join and Mute on and off (not while the
/// computer keyboard plays notes); Split cuts a note at the step under the mouse,
/// Join puts it back together, Mute mutes and unmutes it.
#[test]
fn mouse_modes_split_join_and_mute() {
    use layout::MouseMode as M;
    let Some(mut h) = harness(false, true) else { return };
    let p = project_of(&h);
    let (g, pi) = (h.state().group, h.state().pattern);
    let start = p.groups[g].patterns[pi].start;
    let Some(sound) = (0..p.groups[g].sounds.len().min(8)).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
    let key = |h: &mut Harness<'static, App>, k: egui::Key| {
        h.key_press(k);
        h.run_steps(2);
    };
    for (k, m) in [(egui::Key::E, M::Draw), (egui::Key::X, M::Split), (egui::Key::Y, M::Join), (egui::Key::Z, M::Mute)] {
        key(&mut h, k);
        assert_eq!(h.state().mouse, m, "{k:?}");
    }
    key(&mut h, egui::Key::Z);
    assert_eq!(h.state().mouse, M::Select, "the same key again: Select");
    h.state_mut().keys.on = true;
    key(&mut h, egui::Key::Z);
    assert_eq!(h.state().mouse, M::Select, "not while Keys plays notes");
    h.state_mut().keys.on = false;

    let note =
        Note { ratchet: Default::default(), expr: None, sound, position: start, length: 960, pitch: 60, velocity: 100, probability: 100 };
    h.state().doc.as_ref().unwrap().borrow_mut().add_note(g, pi, note).unwrap();
    h.run_steps(2);
    let mine = |h: &Harness<'static, App>| notes_of(h).into_iter().filter(|n| n.sound == sound).collect::<Vec<_>>();
    key(&mut h, egui::Key::X);
    let pos = grid_pos(&h, sound, 480 + 40);
    click(&mut h, pos);
    assert_eq!(
        mine(&h),
        [Note { length: 480, ..note }, Note { position: start + 480, length: 480, ..note }],
        "split at the step under the mouse"
    );
    key(&mut h, egui::Key::Y);
    let pos = grid_pos(&h, sound, 120);
    click(&mut h, pos);
    assert_eq!(mine(&h), [note], "joined again");
    assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_label(), Some("Join note events"));
    key(&mut h, egui::Key::Z);
    click(&mut h, pos);
    assert_eq!(mine(&h), [Note { probability: 0, ..note }], "muted");
    click(&mut h, pos);
    assert_eq!(mine(&h), [note], "unmuted");
}

/// ⌥← / ⌥→ nudge by the Nudge Grid (Step/2 by default): the selection, or every
/// note shown when none is selected, coming round at the pattern's ends.
#[test]
fn alt_arrows_nudge_by_the_nudge_grid() {
    let Some(mut h) = harness(false, true) else { return };
    let (g, pi) = (h.state().group, h.state().pattern);
    let p = project_of(&h);
    let (start, len) = (p.groups[g].patterns[pi].start, p.groups[g].patterns[pi].length);
    assert_eq!(h.state().nudge.ticks(h.state().grid), h.state().grid / 2, "Step/2");
    let before = notes_of(&h);
    let one = before[0];
    h.state_mut().selected = vec![one];
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowRight);
    h.run_steps(2);
    let half = h.state().grid / 2;
    let moved = Note { position: start + (one.position + half - start).rem_euclid(len), ..one };
    assert!(notes_of(&h).contains(&moved), "the selected note, half a step right");
    assert_eq!(notes_of(&h).len(), before.len());
    // None selected: all of them, back left (round the start if they must).
    h.state_mut().selected = Vec::new();
    h.state_mut().nudge = layout::NudgeGrid(0);
    h.run_steps(1);
    let now = notes_of(&h);
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
    h.run_steps(2);
    let step = h.state().grid;
    let mut want: Vec<i64> = now.iter().map(|n| start + (n.position - step - start).rem_euclid(len)).collect();
    let mut got: Vec<i64> = notes_of(&h).iter().map(|n| n.position).collect();
    want.sort();
    got.sort();
    assert_eq!(got, want, "every note a step left");
}

/// ⌘C / ⌘V: a copy pastes after itself; ⌘X cuts. The event area's right-click
/// menu: Paste at the mouse, the mouse modes, the Nudge Grid, Delete.
/// The note menu's Ratchet items change every selected note's ratchet, an undo
/// step each.
#[test]
fn the_ratchet_menu_sets_the_selected_notes_ratchets() {
    use deckard_model::ratchet::{Ratchet, Shape};
    let mut app = App { settings: crate::settings::Settings::default(), ..App::default() };
    app.output = Some(Err("no audio in this test".into()));
    app.new_project();
    let note = |position| Note {
        ratchet: Default::default(),
        expr: None,
        sound: 0,
        position,
        length: 240,
        pitch: 60,
        velocity: 100,
        probability: 100,
    };
    let (a, b, other) = (note(0), note(480), note(960));
    layout::apply_acts(&mut app, vec![layout::Act::EditNotes { g: 0, p: 0, label: "t", remove: Vec::new(), add: vec![a, b, other] }]);
    app.selected = vec![a, b];
    let notes = |app: &App| app.doc.as_ref().unwrap().borrow().project().groups[0].patterns[app.pattern].notes.clone();
    let changes: [fn(Ratchet) -> Ratchet; 3] =
        [|r| Ratchet { repeats: 4, ..r }, |r| Ratchet { shape: Shape::Down, ..r }, |r| Ratchet { floor: 30, ..r }];
    for change in changes {
        let act = layout::ratchet_edit(&app, change).expect("a change");
        layout::apply_acts(&mut app, vec![act]);
    }
    let want = Ratchet { repeats: 4, shape: Shape::Down, floor: 30, ceiling: 127 };
    assert_eq!(notes(&app).iter().map(|n| n.ratchet).collect::<Vec<_>>(), [want, want, Ratchet::default()]);
    assert!(app.selected.iter().all(|n| n.ratchet == want) && app.selected.len() == 2, "the selection follows");
    assert!(layout::ratchet_edit(&app, |r| r).is_none(), "nothing to change, no edit");
    for _ in 0..3 {
        app.undo();
    }
    assert_eq!(notes(&app), [a, b, other]);
}

/// The note menu stays open while its Ratchet numbers are clicked, dragged and
/// typed into; choosing a shape closes it.
#[test]
fn the_ratchet_menu_stays_open_for_its_numbers() {
    use deckard_model::ratchet::Shape;
    let mut h = blank_harness(false);
    let note =
        Note { ratchet: Default::default(), expr: None, sound: 0, position: 0, length: 960, pitch: 60, velocity: 100, probability: 100 };
    layout::apply_acts(h.state_mut(), vec![layout::Act::EditNotes { g: 0, p: 0, label: "t", remove: Vec::new(), add: vec![note] }]);
    h.state_mut().selected = vec![note];
    h.run_steps(2);
    let pos = grid_pos(&h, 0, 60);
    right_click(&mut h, pos);
    h.get_by_label("Ratchet ⏵").hover();
    h.run_steps(3);
    // A drag on Repeats' number sets the repeats, once, when it ends.
    let at = h.get_by_label("Repeats").rect().right_center() + egui::vec2(20.0, 0.0);
    let button = |h: &mut Harness<'static, App>, pos: Pos2, pressed: bool| {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() });
        h.step();
    };
    h.hover_at(at);
    h.step();
    button(&mut h, at, true);
    for dx in [10.0, 20.0, 30.0] {
        h.hover_at(at + egui::vec2(dx, 0.0));
        h.step();
        assert_eq!(notes_of(&h)[0].ratchet.repeats, 1, "nothing changes until the drag ends");
    }
    button(&mut h, at + egui::vec2(30.0, 0.0), false);
    h.run_steps(3);
    let repeats = notes_of(&h)[0].ratchet.repeats;
    assert!(repeats > 1, "the drag set the repeats: {repeats}");
    assert!(h.query_by_label("Repeats").is_some(), "still open after the drag");
    // A click on it makes it a text field: type a number, Enter sets it.
    h.hover_at(at);
    h.step();
    button(&mut h, at, true);
    button(&mut h, at, false);
    h.run_steps(3);
    assert!(h.query_by_label("Repeats").is_some(), "still open after a click on the number");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.step();
    h.event(Event::Text("7".into()));
    h.step();
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(notes_of(&h)[0].ratchet.repeats, 7, "typed");
    assert!(h.query_by_label("Repeats").is_some(), "still open after typing");
    h.get_by_label("Down").click();
    h.run_steps(3);
    assert_eq!(notes_of(&h)[0].ratchet.shape, Shape::Down);
    assert!(h.query_by_label("Repeats").is_none(), "a shape closes the menu");
}

#[test]
fn copy_paste_and_the_event_menu() {
    let Some(mut h) = harness(false, true) else { return };
    let p = project_of(&h);
    let (g, pi) = (h.state().group, h.state().pattern);
    let start = p.groups[g].patterns[pi].start;
    let Some(sound) = (0..p.groups[g].sounds.len().min(8)).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
    let note =
        Note { ratchet: Default::default(), expr: None, sound, position: start, length: 240, pitch: 60, velocity: 100, probability: 100 };
    h.state().doc.as_ref().unwrap().borrow_mut().add_note(g, pi, note).unwrap();
    h.state_mut().sound = sound;
    h.state_mut().selected = vec![note];
    h.run_steps(2);
    let mine = |h: &Harness<'static, App>| notes_of(h).into_iter().filter(|n| n.sound == sound).collect::<Vec<_>>();
    let cmd = |h: &mut Harness<'static, App>, k: egui::Key| {
        h.key_press_modifiers(egui::Modifiers::COMMAND, k);
        h.run_steps(2);
    };
    cmd(&mut h, egui::Key::C);
    cmd(&mut h, egui::Key::V);
    let after = Note { position: start + 240, ..note };
    assert_eq!(mine(&h), [note, after], "pasted on the next step");
    assert_eq!(h.state().selected, [after], "the pasted notes selected");
    cmd(&mut h, egui::Key::X);
    assert_eq!(mine(&h), [note], "cut");

    // Right-click a step: Paste puts the cut note there.
    let pos = grid_pos(&h, sound, 960 + 20);
    right_click(&mut h, pos);
    h.get_by_label("Paste").click();
    h.run_steps(3);
    assert_eq!(mine(&h), [note, Note { position: start + 960, ..note }], "pasted at the mouse");
    let pos = grid_pos(&h, sound, 1920 + 20);
    right_click(&mut h, pos);
    h.get_by_label("Split").click();
    h.run_steps(2);
    assert_eq!(h.state().mouse, layout::MouseMode::Split);
    h.state_mut().mouse = layout::MouseMode::Select;
    right_click(&mut h, pos);
    h.get_by_label("Nudge Grid ⏵").hover();
    h.run_steps(3);
    h.get_by_label("Step/4").click();
    h.run_steps(2);
    assert_eq!(h.state().nudge, layout::NudgeGrid(2));
    // Right-click a note: Delete removes it.
    let pos = grid_pos(&h, sound, 960 + 60);
    right_click(&mut h, pos);
    h.get_by_label("Delete").click();
    h.run_steps(3);
    assert_eq!(mine(&h), [note]);
}

#[test]
fn select_mode_selects_moves_and_deletes() {
    let Some(mut h) = harness(false, true) else { return };
    let p = project_of(&h);
    let (g, pi) = (h.state().group, h.state().pattern);
    let before = notes_of(&h);
    let start = p.groups[g].patterns[pi].start;
    let Some(sound) = (0..p.groups[g].sounds.len().min(8)).find(|s| before.iter().all(|n| n.sound != *s)) else { return };
    assert!(!h.state().draw_mode());
    // A single click on an empty step only clears the selection; double-click adds.
    let pos = grid_pos(&h, sound, 120);
    click(&mut h, pos);
    assert_eq!(notes_of(&h), before);
    for at in [120, 960 + 120] {
        // A pause, so the next two clicks are a double-click, not a triple.
        h.run_steps(40);
        let pos = grid_pos(&h, sound, at);
        h.hover_at(pos);
        h.step();
        for _ in 0..2 {
            for pressed in [true, false] {
                h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() });
                h.step();
            }
        }
        h.run_steps(3);
    }
    let two: Vec<Note> = notes_of(&h).into_iter().filter(|n| n.sound == sound).collect();
    assert_eq!(two.len(), 2, "double-clicks add notes");
    // Box-select both, drag one: both move.
    let (a, b) = (grid_pos(&h, sound, 3840) + egui::vec2(0.0, -10.0), grid_pos(&h, sound, 0) + egui::vec2(-4.0, 8.0));
    drag(&mut h, a, b);
    assert_eq!(h.state().selected.len(), 2, "box selection: {:?}", h.state().selected);
    let grab = egui::vec2(3.0, 0.0);
    let (a, b) = (grid_pos(&h, sound, 0) + grab, grid_pos(&h, sound, 240) + grab);
    drag(&mut h, a, b);
    let moved: Vec<i64> = notes_of(&h).iter().filter(|n| n.sound == sound).map(|n| n.position - start).collect();
    assert_eq!(moved, [240, 1200], "the selection moves together");
    // Delete removes the selection.
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(notes_of(&h), before);
}

#[test]
fn grid_length_time_signature_quantize_and_nudge() {
    let Some(mut h) = harness(false, true) else { return };
    let (g, pi) = (h.state().group, h.state().pattern);
    let p = project_of(&h);
    let bar = p.bar_length();
    let start = p.groups[g].patterns[pi].start;

    // Time signature: 4 / 4 -> 3 / 4.
    press(&mut h, "4 / 4  SIG");
    press(&mut h, "3 / 4");
    assert_eq!((project_of(&h).numerator, project_of(&h).denominator), (3, 4));
    let bar3 = project_of(&h).bar_length();
    assert_eq!(bar3, 3 * 960);

    // Pattern length: 2 bars (of 3/4).
    let len = project_of(&h).groups[g].patterns[pi].length;
    let text = format!("{}:{}:{}", len / bar3, len % bar3 / 960, len % 960 / 240);
    press(&mut h, &text);
    press(&mut h, "2 bars");
    assert_eq!(project_of(&h).groups[g].patterns[pi].length, 2 * bar3);

    // Grid 1/8: drawn notes are an eighth long and snap to eighths.
    press(&mut h, "1/16");
    press(&mut h, "1/8");
    assert_eq!(h.state().grid, 480);
    h.state_mut().mouse = layout::MouseMode::Draw;
    let Some(sound) = (0..8).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
    let pos = grid_pos(&h, sound, 480 + 300);
    click(&mut h, pos);
    let drawn = notes_of(&h).into_iter().find(|n| n.sound == sound).expect("a note");
    assert_eq!((drawn.position - start, drawn.length), (480, 480));

    // Quantize: move it off the grid, select it, quantize 100 % to eighths.
    let off = Note { position: drawn.position + 100, ..drawn };
    h.state().doc.as_ref().unwrap().borrow_mut().change_note(g, pi, drawn, off).unwrap();
    h.state_mut().mouse = layout::MouseMode::Select;
    h.state_mut().selected = vec![off];
    h.run_steps(2);
    press(&mut h, "QUANTIZE");
    press(&mut h, "100 %");
    assert!(
        notes_of(&h).contains(&drawn),
        "quantized back onto the grid: {:?}",
        notes_of(&h).iter().filter(|n| n.sound == sound).collect::<Vec<_>>()
    );

    // Nudge: select it, arrow right moves it one grid step.
    h.state_mut().selected = vec![drawn];
    h.run_steps(1);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    assert!(notes_of(&h).contains(&Note { position: drawn.position + 480, ..drawn }));
    let _ = bar;
}

/// Notes drawn on a 1/32 grid, then quantized on a 1/4 grid (as a user did).
#[test]
fn quantize_after_changing_the_grid() {
    let Some(mut h) = harness(false, true) else { return };
    let (g, pi) = (h.state().group, h.state().pattern);
    let start = project_of(&h).groups[g].patterns[pi].start;
    let Some(sound) = (0..8).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
    press(&mut h, "1/16");
    press(&mut h, "1/32");
    assert_eq!(h.state().grid, 120);
    // Pencil, then paint a few 1/32 notes.
    let panel_top = 33.0 + 3.0 + 343.0 + 2.0 + 140.0 + 2.0;
    click(&mut h, Pos2::new(3.0 + 16.0, panel_top + 7.0 + 2.0 * 28.0 + 12.0));
    for t in [120 + 60, 600 + 60, 1320 + 60] {
        let pos = grid_pos(&h, sound, t);
        click(&mut h, pos);
    }
    let mine =
        |h: &Harness<'static, App>| -> Vec<i64> { notes_of(h).iter().filter(|n| n.sound == sound).map(|n| n.position - start).collect() };
    assert_eq!(mine(&h), [120, 600, 1320]);
    press(&mut h, "1/32");
    press(&mut h, "1/4");
    assert_eq!(h.state().grid, 960);
    press(&mut h, "QUANTIZE");
    press(&mut h, "100 %");
    eprintln!("status: {:?}, selected: {:?}", h.state().status, h.state().selected);
    assert_eq!(mine(&h), [0, 960, 960], "quantized to quarters");
}

/// In select mode: notes added by double-click aren't selected, so QUANTIZE
/// snaps all of them.
#[test]
fn quantize_after_double_click_adds() {
    let Some(mut h) = harness(false, true) else { return };
    let (g, pi) = (h.state().group, h.state().pattern);
    let start = project_of(&h).groups[g].patterns[pi].start;
    let Some(sound) = (0..8).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
    press(&mut h, "1/16");
    press(&mut h, "1/32");
    for t in [120 + 60, 600 + 60, 1320 + 60] {
        h.run_steps(40);
        let pos = grid_pos(&h, sound, t);
        h.hover_at(pos);
        h.step();
        for _ in 0..2 {
            for pressed in [true, false] {
                h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() });
                h.step();
            }
        }
        h.run_steps(3);
    }
    let mine =
        |h: &Harness<'static, App>| -> Vec<i64> { notes_of(h).iter().filter(|n| n.sound == sound).map(|n| n.position - start).collect() };
    assert_eq!(mine(&h), [120, 600, 1320]);
    assert!(h.state().selected.is_empty(), "added notes are not selected");
    press(&mut h, "1/32");
    press(&mut h, "1/4");
    press(&mut h, "QUANTIZE");
    press(&mut h, "100 %");
    assert_eq!(mine(&h), [0, 960, 960]);
}

/// Double-clicking a note deletes it (and only it).
#[test]
fn double_click_deletes_a_note() {
    let Some(mut h) = harness(false, true) else { return };
    let (g, pi) = (h.state().group, h.state().pattern);
    let start = project_of(&h).groups[g].patterns[pi].start;
    let Some(sound) = (0..8).find(|s| notes_of(&h).iter().all(|n| n.sound != *s)) else { return };
    let mine =
        |h: &Harness<'static, App>| -> Vec<i64> { notes_of(h).iter().filter(|n| n.sound == sound).map(|n| n.position - start).collect() };
    let others = notes_of(&h).len();
    for t in [0, 960] {
        h.run_steps(40);
        let pos = grid_pos(&h, sound, t + 60);
        double_click(&mut h, pos);
        h.run_steps(3);
    }
    assert_eq!(mine(&h), [0, 960]);
    h.run_steps(40);
    let pos = grid_pos(&h, sound, 60);
    double_click(&mut h, pos);
    h.run_steps(3);
    assert_eq!(mine(&h), [960]);
    assert_eq!(notes_of(&h).len(), others + 1);
}

/// A swing change made while the song plays reaches the running engine.
#[test]
fn swing_reaches_playback() {
    let Some(mut h) = harness(false, true) else { return };
    if !play(&mut h) {
        return;
    }
    let ticks = |h: &Harness<'static, App>, g: usize| -> Vec<i64> {
        let Some(Ok(out)) = &h.state().output else { return Vec::new() };
        out.call(move |e| e.timeline().events.iter().filter(|x| x.group == g && x.midi.is_none()).map(|x| x.tick).collect())
            .unwrap_or_default()
    };
    // The group whose notes swing moves most (off-beat 16ths).
    let g = (0..project_of(&h).groups.len()).max_by_key(|&g| ticks(&h, g).iter().filter(|t| *t % 480 == 240).count()).unwrap();
    let before = ticks(&h, g);
    h.state_mut()
        .arrange(layout::Act::SetSwing(deckard_doc::Target::Group(g), deckard_model::Swing { amount: 60.0, cycle: 0, invert: false }));
    h.run_steps(3);
    let after = ticks(&h, g);
    let moved = before.iter().zip(&after).filter(|(a, b)| a != b).count();
    assert!(moved > 0, "swing moved none of group {g}'s {} notes in the playing engine", before.len());
    // Off-beat sixteenths are late now; on-beat ones stay.
    for (a, b) in before.iter().zip(&after) {
        match a % 480 {
            0 => assert_eq!(a, b),
            240 => assert!(b > a, "{a} -> {b}"),
            _ => {}
        }
    }
    h.state_mut().stop();
}

#[test]
fn select_all_and_escape() {
    let Some(mut h) = harness(false, true) else { return };
    // #31: the focused sound's notes only.
    let sound = notes_of(&h)[0].sound;
    h.state_mut().sound = sound;
    let (g, pi) = (h.state().group, h.state().pattern);
    let other = Note { sound: (sound + 1) % 16, ..notes_of(&h)[0] };
    h.state().doc.as_ref().unwrap().borrow_mut().edit_notes("t", g, pi, &[], &[other]).unwrap();
    h.run_steps(2);
    let n = notes_of(&h).iter().filter(|n| n.sound == sound).count();
    assert!(n > 0 && n < notes_of(&h).len(), "other sounds have notes too");
    // Focus the grid area first (keys go to the editor when nothing else has focus).
    let pos = grid_pos(&h, 15, 60);
    click(&mut h, pos);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(3);
    assert_eq!(h.state().selected.len(), n, "⌘A selects the focused sound's notes");
    assert!(h.state().selected.iter().all(|n| n.sound == sound));
    h.state_mut().selected.clear();
    h.state_mut().menu_commands.push(menu::Command::SelectAll);
    h.run_steps(3);
    assert_eq!(h.state().selected.len(), n, "Edit ▸ Select All Notes too");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().selected.is_empty(), "Esc clears");
}

/// BUGS #17: the Erase tool erases what it's dragged over in a CC lane; a span
/// selected there goes with Delete.
#[test]
fn a_cc_lane_erases_a_span() {
    use deckard_model::{MidiEvent, MidiKind};
    use layout::Lane;
    let Some(mut h) = harness(false, true) else { return };
    let (g, pi, sound) = (h.state().group, h.state().pattern, h.state().sound);
    let start = project_of(&h).groups[g].patterns[pi].start;
    let cc = |t| MidiEvent { sound, position: start + t, kind: MidiKind::Controller { number: 1, value: 64 } };
    let events: Vec<MidiEvent> = (0..8).map(|i| cc(i * 240)).collect();
    h.state().doc.as_ref().unwrap().borrow_mut().edit_midi("t", g, pi, &[], &events).unwrap();
    h.state_mut().lane = Lane::Controller(1);
    h.run_steps(3);
    let left = |h: &Harness<'static, App>| -> Vec<i64> {
        project_of(h).groups[g].patterns[pi]
            .midi
            .iter()
            .filter(|m| m.sound == sound && Lane::of(&m.kind) == Lane::Controller(1))
            .map(|m| m.position - start)
            .collect()
    };
    let (bottom, lane_h) = lane(&h);
    let y = bottom - lane_h * 0.5;
    h.state_mut().mouse = layout::MouseMode::Erase;
    let (a, b) = (grid_pos(&h, 0, 240 + 20).x, grid_pos(&h, 0, 480 + 20).x);
    drag(&mut h, Pos2::new(a, y), Pos2::new(b, y));
    assert_eq!(left(&h), [0, 720, 960, 1200, 1440, 1680], "the steps dragged over");
    h.state_mut().mouse = layout::MouseMode::Select;
    (h.state_mut().lane_span, h.state_mut().key_focus) = (Some((Lane::Controller(1), start + 960, start + 1440)), layout::KeyFocus::Events);
    h.run_steps(2);
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(left(&h), [0, 720, 1440, 1680], "the span selected");
    assert_eq!(h.state().lane_span, None);
}

/// The lane list: velocity, probability and the sound's MIDI controllers (the
/// "Keys" pattern's sustain pedal); "+" adds a lane, which can be drawn into,
/// erased, and removed again.
#[test]
fn midi_lanes_list_draw_and_remove() {
    use deckard_model::MidiKind;
    use layout::Lane;
    let Some(mut h) = harness(false, true) else { return };
    let p = project_of(&h);
    let Some((g, pi, sound)) = p
        .groups
        .iter()
        .enumerate()
        .find_map(|(g, gr)| gr.patterns.iter().enumerate().find_map(|(pi, pt)| pt.midi.first().map(|m| (g, pi, m.sound))))
    else {
        return;
    };
    (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (g, pi, sound);
    h.run_steps(3);
    let listed: Vec<Lane> = lane_list(&h).iter().map(|(l, _)| *l).collect();
    assert_eq!(listed[..4], [Lane::Velocity, Lane::Probability, Lane::Ratchet, Lane::Controller(64)], "{listed:?}");
    pick_lane(&mut h, Lane::Controller(64));

    // "+" opens the dialog; add the mod wheel.
    let plus = lane_list(&h).last().unwrap().1;
    click(&mut h, plus.center());
    assert!(h.state().lanes_dialog);
    h.get_by_label("Mod Wheel (CC 1)").click();
    h.run_steps(3);
    assert_eq!(h.state().lane, Lane::Controller(1), "the new lane is shown");
    let saved = |h: &Harness<'static, App>| project_of(h).groups[g].sounds[sound].lanes.clone();
    assert!(saved(&h).contains(&deckard_model::LaneRef { kind: 4, number: 1 }), "and saved with the sound");
    h.state_mut().lanes_dialog = false;
    h.run_steps(3);

    // Draw half-way values across the first beat: one event per 16th.
    let start = p.groups[g].patterns[pi].start;
    let cc1 = |h: &Harness<'static, App>| -> Vec<(i64, u8)> {
        project_of(h).groups[g].patterns[pi]
            .midi
            .iter()
            .filter_map(|m| match m.kind {
                MidiKind::Controller { number: 1, value } if m.sound == sound => Some((m.position - start, value)),
                _ => None,
            })
            .collect()
    };
    let (bottom, lane_h) = lane(&h);
    let y = bottom - lane_h * 0.5;
    let (a, b) = (grid_pos(&h, 0, 20).x, grid_pos(&h, 0, 960 - 20).x);
    drag(&mut h, Pos2::new(a, y), Pos2::new(b, y));
    let drawn = cc1(&h);
    assert_eq!(drawn.iter().map(|(t, _)| *t).collect::<Vec<_>>(), [0, 240, 480, 720], "{drawn:?}");
    assert!(drawn.iter().all(|(_, v)| (58..=69).contains(v)), "{drawn:?}");
    // A click sets one step.
    let top = Pos2::new(grid_pos(&h, 0, 240 + 100).x, bottom - lane_h);
    click(&mut h, top);
    assert_eq!(cc1(&h)[1], (240, 127));
    // A right-click erases one.
    let pos = Pos2::new(grid_pos(&h, 0, 480 + 100).x, y);
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.run_steps(3);
    assert_eq!(cc1(&h).iter().map(|(t, _)| *t).collect::<Vec<_>>(), [0, 240, 720]);
    // The pedal is untouched.
    let pedal = |h: &Harness<'static, App>| {
        project_of(h).groups[g].patterns[pi].midi.iter().filter(|m| Lane::of(&m.kind) == Lane::Controller(64)).count()
    };
    assert_eq!(pedal(&h), pedal_before(&p, g, pi));

    // Remove the lane: its events go too (one undo step).
    h.state_mut().lanes_dialog = true;
    h.run_steps(3);
    h.get_by_label("Remove (3 events)").click();
    h.run_steps(3);
    assert!(cc1(&h).is_empty());
    assert!(!lane_list(&h).iter().any(|(l, _)| *l == Lane::Controller(1)));
    assert!(!saved(&h).iter().any(|l| l.kind == 4 && l.number == 1));
    assert_eq!(h.state().lane, Lane::Velocity);
    h.state_mut().undo();
    assert_eq!(cc1(&h).len(), 3, "undo brings the events back");
    assert!(saved(&h).iter().any(|l| l.kind == 4 && l.number == 1), "and the lane");
}

/// Right-clicking a modulated knob in the Control area (anywhere on it, not only its
/// thin ring) offers Remove Modulation, which removes it.
#[test]
fn right_clicking_a_knobs_ring_removes_its_modulation() {
    let Some(mut h) = harness(false, false) else { return };
    let doc = h.state().doc.clone().unwrap();
    let (g, s) = (h.state().group, h.state().sound);
    let n = doc.borrow().project().groups[g].sounds[s].slots.len();
    doc.borrow_mut().add_slot(g, s, n, &deckard_doc::NewModule::Effect("Filter")).unwrap();
    h.state_mut().channel = false;
    h.state_mut().chain_tab = layout::ChainTab::Sound;
    h.state_mut().slot = Some(n);
    h.run_steps(3);
    let rects: Vec<Option<egui::Rect>> = h.ctx.data(|d| d.get_temp(egui::Id::new("knob-rects"))).expect("knobs shown");
    // The first knob that can be modulated, given modulation.
    let slot = doc.borrow().project().groups[g].sounds[s].slots[n].clone();
    let pg = module_ui::page_for(&slot, deckard_doc::Target::Module(g, s, n), h.state().plugin_page);
    let (k, edit) = pg
        .slots
        .iter()
        .enumerate()
        .find_map(|(k, x)| x.as_ref().and_then(|x| x.3).filter(|e| modulation::mod_target(*e).is_some()).map(|e| (k, e)))
        .expect("a modulatable knob");
    modulation::register();
    let (c, t) = modulation::mod_target(edit).unwrap();
    let pi = doc.borrow().project().scenes[h.state().scene].patterns.iter().find(|x| x.0 == g).map(|x| x.1).unwrap_or(0);
    doc.borrow_mut().set_modulation(c, &t, pi, (0, 0), &[(100, 0.3), (900, -0.2)]).unwrap();
    let count = |d: &deckard_doc::Document| d.project().groups[g].patterns.iter().map(|p| p.mods.len()).sum::<usize>();
    let before = count(&doc.borrow());
    assert!(before >= 2);
    h.run_steps(2);
    let r = rects[k].expect("its rect");
    // Anywhere on the knob (its centre is r.top() + 40): a menu.
    right_click(&mut h, Pos2::new(r.center().x, r.top() + 40.0));
    h.get_by_label("Remove Modulation").click();
    h.run_steps(3);
    assert_eq!(count(&doc.borrow()), before - 2, "removed; status {:?}", h.state().status);
}

/// The header with Auto-write off and on (the bullseye) to PNGs in
/// `$DECKARD_RENDER_DIR` (run with `--ignored`; needs a GPU).
#[test]
#[ignore]
fn render_auto_write() {
    let Some(dir) = std::env::var_os("DECKARD_RENDER_DIR").map(PathBuf::from) else { return };
    let Some(mut h) = harness(false, true) else { return };
    layout::style(&h.ctx);
    for (on, name) in [(false, "auto-off.png"), (true, "auto-on.png")] {
        h.state_mut().auto_write = on;
        h.run_steps(3);
        h.render().unwrap().save(dir.join(name)).unwrap();
    }
}

/// Renders the lane list, a MIDI lane and the "+" dialog to PNGs in
/// The Expression editor's lanes over a chord with curves, in the piano roll, to
/// `$DECKARD_RENDER_DIR` (run with `--ignored`; needs a GPU).
#[test]
#[ignore]
fn render_expression_lanes() {
    use deckard_model::{Curve, Expression, Point};
    let Some(dir) = std::env::var_os("DECKARD_RENDER_DIR").map(PathBuf::from) else { return };
    let Some(mut h) = harness(false, true) else { return };
    layout::style(&h.ctx);
    let Some((sound, n)) = lone_note(&mut h) else { return };
    let pt = |at: i64, value: f32, curve: f32| Point { at, value, curve };
    let e = Expression {
        pitch: Some(Curve { points: vec![pt(0, 0.0, 0.6), pt(480, 1.0, 0.0), pt(960, 0.5, 0.0)] }),
        pressure: Some(Curve { points: vec![pt(0, 0.1, 0.8), pt(600, 0.9, -0.5), pt(960, 0.3, 0.0)] }),
        release_velocity: Some(80),
        ..Expression::default()
    };
    let (g, pi) = (h.state().group, h.state().pattern);
    let doc = h.state().doc.clone().unwrap();
    let now = doc.borrow_mut().set_expressions(g, pi, &[(n, e)]).unwrap();
    let other = Note { pitch: 64, expr: None, ..n };
    doc.borrow_mut().edit_notes("t", g, pi, &[], &[other]).unwrap();
    h.state_mut().selected = now.clone();
    h.state_mut().piano_roll = true;
    h.state_mut().sound = sound;
    for (l, name) in [
        (layout::Lane::Expr(Dim::Pressure), "expr-pressure.png"),
        (layout::Lane::Expr(Dim::Pitch), "expr-pitch.png"),
        (layout::Lane::Release, "expr-release.png"),
    ] {
        h.state_mut().lane = l;
        h.run_steps(3);
        h.render().unwrap().save(dir.join(name)).unwrap();
    }
    h.state_mut().lanes_dialog = true;
    h.run_steps(3);
    h.render().unwrap().save(dir.join("expr-dialog.png")).unwrap();
}

/// `$DECKARD_RENDER_DIR` (run with `--ignored`; needs a GPU).
#[test]
#[ignore]
fn render_midi_lanes() {
    let Some(dir) = std::env::var_os("DECKARD_RENDER_DIR").map(PathBuf::from) else { return };
    let Some(mut h) = harness(false, true) else { return };
    layout::style(&h.ctx);
    let p = project_of(&h);
    let (g, pi, sound) = p
        .groups
        .iter()
        .enumerate()
        .find_map(|(g, gr)| gr.patterns.iter().enumerate().find_map(|(pi, pt)| pt.midi.first().map(|m| (g, pi, m.sound))))
        .unwrap();
    (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound) = (g, pi, sound);
    h.state_mut().lane = layout::Lane::Controller(64);
    h.run_steps(3);
    h.render().unwrap().save(dir.join("lane-cc64.png")).unwrap();
    h.state_mut().lanes_dialog = true;
    h.run_steps(3);
    h.render().unwrap().save(dir.join("lane-dialog.png")).unwrap();
    h.state_mut().lanes_dialog = false;
    h.run_steps(3);
    // The MASCHINE menu, with Edit open.
    click(&mut h, Pos2::new(40.0, 16.0));
    h.get_by_label("Edit ⏵").hover();
    h.run_steps(5);
    h.render().unwrap().save(dir.join("menu-edit.png")).unwrap();
    h.get_by_label("View ⏵").hover();
    h.run_steps(30);
    h.render().unwrap().save(dir.join("menu-view.png")).unwrap();
    // The Sampler's pages for the kick (group 0, sound 0).
    h.key_press(egui::Key::Escape);
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
    for page in 0..sampler_ui::PAGES.len() {
        h.state_mut().sampler_page = page;
        h.run_steps(4);
        h.render().unwrap().save(dir.join(format!("sampler-page{}.png", page + 1))).unwrap();
    }
    // The waveform's playhead while the kick plays, then the waveform hidden.
    h.state_mut().sampler_page = 0;
    h.state_mut().play_pitch(0, 0, 60);
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.step();
        if h.state().zone_playhead.is_some_and(|f| f > 0) {
            break;
        }
    }
    assert!(h.state().zone_playhead.is_some(), "the kick's zone plays");
    h.render().unwrap().save(dir.join("sampler-playhead.png")).unwrap();
    h.state_mut().settings.show_waveform = false;
    h.run_steps(3);
    h.render().unwrap().save(dir.join("sampler-no-waveform.png")).unwrap();
    h.state_mut().settings.show_waveform = true;
    // A click on a choice's value lists the choices; picking one sets it.
    h.state_mut().sampler_page = 0;
    h.run_steps(4);
    let knobs: Vec<(String, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap();
    let mode = knobs.iter().find(|(n, _)| n == "MODE").unwrap().1;
    click(&mut h, Pos2::new(mode.center().x, mode.top() + 42.0));
    h.render().unwrap().save(dir.join("sampler-mode-list.png")).unwrap();
    click(&mut h, Pos2::new(mode.center().x, mode.top() + 42.0 + 8.0 + 21.0 * 1.5));
    h.render().unwrap().save(dir.join("sampler-mode-picked.png")).unwrap();
    let p = project_of(&h);
    let smp = p.groups[0].sounds[0].sampler.as_ref().unwrap();
    eprintln!("sampler mode after picking: {}", smp.get("mode"));
    // The Mixer, at group and sound level.
    h.state_mut().mixer = true;
    h.state_mut().mixer_sounds = false;
    h.run_steps(4);
    h.render().unwrap().save(dir.join("mixer-groups.png")).unwrap();
    h.state_mut().mixer_sounds = true;
    h.state_mut().mixer_sections.io = true;
    h.state_mut().mixer_sections.aux = true;
    h.run_steps(4);
    h.render().unwrap().save(dir.join("mixer-sounds.png")).unwrap();
    // Sampling, each tab, on the kick.
    h.state_mut().mixer = false;
    (h.state_mut().group, h.state_mut().sound) = (0, 0);
    h.state_mut().sampling_open = true;
    for tab in 0..4 {
        h.state_mut().sampling.tab = tab;
        if tab == 2 {
            h.state_mut().arrange_act(layout::Act::Sampling(layout::sampling::SamplingAct::Tab(2)));
        }
        if tab == 0 {
            // A hot input: peaks past full scale must stay inside the waveform.
            h.state_mut().recorder.view.preview = (0..400).map(|i| (-(i as f32 / 100.0), i as f32 / 100.0)).collect();
        }

        if tab == 1 {
            h.state_mut().sampling.selection = Some((2000, 5000));
        }
        h.run_steps(4);
        h.render().unwrap().save(dir.join(format!("sampling-{tab}.png"))).unwrap();
        if tab == 0 {
            // Each mode's and source's footer.
            use layout::sampling::RecParam;
            for (name, mode, source) in [("loop", 2, 0), ("auto-ext", 3, 0), ("auto-int", 3, 2), ("internal", 1, 2), ("detect-mono", 0, 1)]
            {
                let v = &mut h.state_mut().recorder.view;
                v.set(RecParam::Mode, mode);
                v.set(RecParam::Source, source);
                if name == "auto-ext" {
                    v.midi_outs = vec!["IAC Driver Bus 1".into(), "Prophet Rev2".into()];
                    v.auto.midi_to = Some("Prophet Rev2".into());
                    v.set(RecParam::KeyHi, 84);
                    v.set(RecParam::KeyLo, 36);
                    v.set(RecParam::KeyStride, 6);
                }
                h.run_steps(3);
                h.render().unwrap().save(dir.join(format!("sampling-0-{name}.png"))).unwrap();
            }
            let v = &mut h.state_mut().recorder.view;
            v.set(RecParam::Mode, 1);
            v.set(RecParam::Source, 0);
            h.run_steps(3);
            // The MODE list.
            click(&mut h, Pos2::new(240.0, 895.0));
            h.render().unwrap().save(dir.join("sampling-0-mode.png")).unwrap();
            click(&mut h, Pos2::new(1200.0, 640.0));
        }
    }
    // Preferences ▸ Audio.
    h.state_mut().prefs = Some(2);
    h.state_mut().inputs = Some(vec![("Apollo Twin X".into(), 10), ("MacBook Pro Microphone".into(), 1)]);
    h.state_mut().settings.input_device = Some("Apollo Twin X".into());
    h.run_steps(4);
    h.render().unwrap().save(dir.join("prefs-audio.png")).unwrap();
    h.state_mut().prefs = None;
    h.state_mut().settings.input_device = None;
    // Zoomed in to a few hundred frames: the signal as a curve through the frames.
    h.state_mut().sampling.tab = 1;
    h.state_mut().sampling.selection = None;
    h.state_mut().sampling.view = Some((1000.0, 1300.0));
    h.run_steps(4);
    h.render().unwrap().save(dir.join("sampling-zoom.png")).unwrap();
    // A long sample, all of it: the envelope.
    let key = h.state().sampling.sample_key.clone().unwrap();
    let frames: Vec<[f32; 2]> = (0..48_000 * 20)
        .map(|i| {
            let t = i as f32 / 48_000.0;
            let v = (t * 220.0 * std::f32::consts::TAU).sin() * (0.2 + 0.7 * (t * 0.7).sin().abs()) * (-(t % 1.0) * 3.0).exp();
            [v, v * 0.8]
        })
        .collect();
    h.state_mut().sampling.set_sample(key, std::sync::Arc::new(deckard_engine::sample::Sample { rate: 48_000, frames }));
    h.state_mut().sampling.view = None;
    h.run_steps(1);
    h.render().unwrap().save(dir.join("sampling-long.png")).unwrap();
}
