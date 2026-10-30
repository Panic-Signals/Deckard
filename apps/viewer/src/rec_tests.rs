//! REC in the window: recorded notes land in the playing patterns, quantized when asked,
//! an empty pattern grows to hold them, one undo step per take; the controller's REC.

use crate::tests::harness;
use deckard_engine::{Mode, Recorded};

fn rec(h: &mut egui_kittest::Harness<'static, crate::App>, g: usize, s: usize, tick: f64, len: f64, scene: usize) {
    let on =
        Recorded { group: g, sound: s, pitch: 60, velocity: 100, length: i64::MAX, tick, mode: Mode::Scene(scene), seq: 0, played: tick };
    h.state_mut().take_in_recorded(vec![on, Recorded { velocity: 0, tick: tick + len, played: tick + len, ..on }]);
}

/// The focused group's pattern in the focused scene.
fn scene_pattern(h: &egui_kittest::Harness<'static, crate::App>) -> (usize, usize, usize) {
    let (g, scene) = (h.state().group, h.state().scene);
    let doc = h.state().doc.clone().unwrap();
    let pi = doc.borrow().project().scenes[scene].patterns.iter().find(|x| x.0 == g).map(|x| x.1).expect("a pattern in the scene");
    (g, scene, pi)
}

#[test]
fn recorded_notes_land_in_the_playing_pattern_as_one_take() {
    let Some(mut h) = harness(false, false) else { return };
    let (g, scene, pi) = scene_pattern(&h);
    let doc = h.state().doc.clone().unwrap();
    let (start, len, before) = {
        let d = doc.borrow();
        let p = &d.project().groups[g].patterns[pi];
        (p.start, p.length, p.notes.len())
    };
    let steps = doc.borrow().undo_labels().len();
    h.state_mut().set_recording(true, false);
    assert!(h.state().recording);
    rec(&mut h, g, 5, 100.0, 240.0, scene);
    rec(&mut h, g, 6, (len + 480) as f64, 120.0, scene);
    h.state_mut().set_recording(false, false);
    let d = doc.borrow();
    let notes = &d.project().groups[g].patterns[pi].notes;
    assert_eq!(notes.len(), before + 2);
    assert!(
        notes.iter().any(|n| (n.sound, n.position, n.length) == (5, start + 100, 240)),
        "{:?} start {start} status {}",
        notes.iter().filter(|n| n.sound == 5).collect::<Vec<_>>(),
        h.state().status
    );
    // Past the pattern's end: wrapped into it.
    assert!(notes.iter().any(|n| (n.sound, n.position) == (6, start + 480)));
    assert_eq!(d.undo_labels().len(), steps + 1);
    assert_eq!(d.undo_label(), Some("Record"));
}

#[test]
fn input_quantize_snaps_to_the_grid() {
    let Some(mut h) = harness(false, false) else { return };
    let (g, scene, pi) = scene_pattern(&h);
    h.state_mut().settings.input_quantize = true;
    let grid = h.state().grid;
    h.state_mut().set_recording(true, false);
    rec(&mut h, g, 7, (grid * 2 + grid / 3) as f64, 100.0, scene);
    h.state_mut().set_recording(false, false);
    let doc = h.state().doc.clone().unwrap();
    let d = doc.borrow();
    let p = &d.project().groups[g].patterns[pi];
    assert!(p.notes.iter().any(|n| n.sound == 7 && n.position == p.start + grid * 2), "snapped");
}

/// The controller's REC (startEventRecording / stopEventRecording) records in the app.
#[test]
fn the_controllers_rec_records() {
    let Some(mut h) = harness(true, false) else { return };
    if !matches!(h.state().controller, Some(Ok(_))) {
        return;
    }
    let eval = |h: &mut egui_kittest::Harness<'static, crate::App>, code: &str| {
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            c.host.lua.load(code).exec().unwrap();
        }
        h.run_steps(3);
    };
    eval(&mut h, "NI.DATA.TransportAccess.startEventRecording(App, false, false)");
    assert!(h.state().recording);
    assert!(h.state().playing.is_none(), "stopped: armed, not playing");
    eval(&mut h, "NI.DATA.TransportAccess.stopEventRecording(App)");
    assert!(!h.state().recording);
}

/// Recording into an empty Auto-length pattern: it starts as a bar and grows as it's
/// played into; REC off trims it to the whole bars the take covers.
#[test]
fn an_empty_pattern_grows_to_hold_the_take() {
    let Some(mut h) = harness(false, false) else { return };
    let (g, scene, pi) = scene_pattern(&h);
    let doc = h.state().doc.clone().unwrap();
    doc.borrow_mut().clear_pattern(g, pi).unwrap();
    h.state_mut().pattern = pi;
    let bar = doc.borrow().project().bar_length();
    h.state_mut().set_recording(true, false);
    assert_eq!(doc.borrow().project().groups[g].patterns[pi].length, bar, "a bar to begin with");
    rec(&mut h, g, 3, (bar / 2) as f64, 240.0, scene);
    assert_eq!(doc.borrow().project().groups[g].patterns[pi].length, 2 * bar, "played into: the next bar");
    h.state_mut().set_recording(false, false);
    assert_eq!(doc.borrow().project().groups[g].patterns[pi].length, bar);
}

/// ⌘K: the computer keyboard plays the focused sound (held while the key is), and its
/// notes record as the pads' do; off again, the letters do nothing.
#[test]
fn command_k_plays_the_focused_sound_from_the_keyboard() {
    use eframe::egui::{self, Event, Key, Modifiers};
    let Some(mut h) = harness(false, false) else { return };
    let key = |h: &mut egui_kittest::Harness<'static, crate::App>, key: Key, pressed: bool| {
        h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(1);
    };
    h.key_press_modifiers(Modifiers::COMMAND, Key::K);
    h.run_steps(1);
    assert!(h.state().keys.on);
    key(&mut h, Key::D, true);
    assert_eq!(h.state().keys.held.iter().map(|x| x.1).collect::<Vec<_>>(), [64]);
    key(&mut h, Key::D, false);
    assert!(h.state().keys.held.is_empty());
    h.key_press_modifiers(Modifiers::COMMAND, Key::K);
    h.run_steps(1);
    assert!(!h.state().keys.on);
    key(&mut h, Key::D, true);
    assert!(h.state().keys.held.is_empty());
    let _ = egui::Key::A;
}

/// NOTE REPEAT on the controller: Note Repeat is on while the page shows (the Arp with
/// the pads in keyboard mode), off when it's left, kept on by LOCK; the page's knobs
/// edit the project's settings.
#[test]
fn the_note_repeat_page_switches_note_repeat() {
    use deckard_engine::repeat::RepeatMode;
    let Some(mut h) = harness(true, false) else { return };
    if !matches!(h.state().controller, Some(Ok(_))) {
        return;
    }
    let with = |h: &mut egui_kittest::Harness<'static, crate::App>, f: &dyn Fn(&mut deckard_lua_host::Host)| {
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            f(&mut c.host);
        }
        h.run_steps(3);
    };
    assert!(h.state().repeat_sent.is_none());
    with(&mut h, &|host| host.button("BUTTON_NOTE_REPEAT", true).unwrap());
    assert_eq!(h.state().repeat_sent.as_ref().map(|r| r.1), Some(RepeatMode::NoteRepeat));
    // GATE (knob 8) turned: the project's gate.
    let before = h.state().doc.as_ref().unwrap().borrow().project().arp.gate;
    with(&mut h, &|host| host.encoder("ENCODER_SCREEN_8", -0.5).unwrap());
    let after = h.state().doc.as_ref().unwrap().borrow().project().arp.gate;
    assert!(after < before, "{before} → {after}");
    with(&mut h, &|host| host.button("BUTTON_NOTE_REPEAT", false).unwrap());
    assert!(h.state().repeat_sent.is_none(), "off when the page is left");
    with(&mut h, &|host| host.eval_lua("NI.DATA.SongAccess.setArpRepeatLockState(App, true)"));
    assert!(h.state().repeat_sent.is_some(), "LOCK keeps it on");
    h.state_mut().piano_roll = true;
    h.run_steps(2);
    let _ = RepeatMode::Arp;
}

/// The software's panel: On switches Note Repeat on; its settings edit the project.
#[test]
fn the_repeat_panel_switches_and_edits() {
    let Some(mut h) = harness(false, false) else { return };
    h.state_mut().repeat_panel = true;
    h.state_mut().repeat_soft = true;
    h.run_steps(3);
    assert!(h.state().repeat_sent.is_some());
    if let Ok(p) = std::env::var("DECKARD_PICTURE") {
        h.render().unwrap().save(p).unwrap();
    }
    h.state_mut().repeat_soft = false;
    h.run_steps(2);
    assert!(h.state().repeat_sent.is_none());
}

/// A new project whose sound 1 has a Filter, and pattern 1 in scene 1; the Filter's
/// cutoff knob, and where it is.
fn filter_app() -> (crate::App, crate::layout::Edit, f64) {
    use deckard_doc::{NewModule, Target};
    let mut app = crate::App::default();
    app.new_project();
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        d.add_pattern(0, 0, "Pattern 1").unwrap();
        d.assign_pattern(0, 0, Some(0)).unwrap();
        d.add_slot(0, 0, 0, &NewModule::Effect("Filter")).unwrap();
    }
    let doc = app.doc.clone().unwrap();
    let slot = doc.borrow().project().groups[0].sounds[0].slots[0].clone();
    let pg = crate::module_ui::page_for(&slot, Target::Module(0, 0, 0), 0);
    let edit = pg
        .slots
        .iter()
        .flatten()
        .filter_map(|k| k.3)
        .find(|e| matches!(e, crate::layout::Edit::Param(_, "cutoff", ..) | crate::layout::Edit::Log(_, "cutoff", ..)))
        .expect("a cutoff knob");
    crate::modulation::register();
    let (c, t) = crate::modulation::mod_target(edit).unwrap();
    let at = crate::modulation::knob_travel(doc.borrow().project(), c, &t).unwrap();
    (app, edit, at)
}

/// Knob moves while the song plays and modulation is written: deviations from the
/// knob's value, at the play position, thinned to points when the knob is let go, one
/// undo step; the knob keeps its own value.
#[test]
fn knob_moves_while_writing_become_modulation() {
    let (mut app, edit, at) = filter_app();
    let doc = app.doc.clone().unwrap();
    let steps = doc.borrow().undo_labels().len();
    let cutoff = || doc.borrow().project().groups[0].sounds[0].slots[0].param("cutoff");
    let before = cutoff();
    let head = |tick: f64| Some((Mode::Scene(0), tick));
    let a = at as f32;
    app.modulate_at(edit, a + 0.1, false, head(100.0));
    for k in 1..=8 {
        app.modulate_at(edit, a + 0.1 + 0.01 * k as f32, false, head(100.0 + 50.0 * k as f64));
    }
    app.modulate_at(edit, a + 0.18, true, head(600.0));
    let d = doc.borrow();
    let pts: Vec<(i64, f64)> = d.project().groups[0].patterns[0]
        .mods
        .iter()
        .filter(|m| m.sound == Some(0) && m.field == "cutoff")
        .map(|m| (m.position, (m.value * 100.0).round() / 100.0))
        .collect();
    assert_eq!(pts, [(100, 0.1), (500, 0.18), (600, 0.18)], "a line up, then flat");
    assert_eq!(cutoff(), before, "the knob's own value stays");
    assert_eq!(d.undo_labels().len(), steps + 1);
    assert_eq!(d.undo_label(), Some("Record Modulation"));
}

/// A knob dragged in the software while the song plays is written as modulation only
/// with Auto-write on (BUGS #44: grabbing the knob by its ring wrote it with Auto off).
#[test]
fn a_dragged_knob_writes_only_with_auto_write_on() {
    let (mut app, edit, _) = filter_app();
    app.playing = Some(Mode::Scene(0));
    app.auto_write = false;
    assert!(!crate::layout::knob_drag_writes(&app, edit), "Auto off: the knob just turns");
    app.auto_write = true;
    assert!(crate::layout::knob_drag_writes(&app, edit), "Auto on: written");
    app.playing = None;
    assert!(!crate::layout::knob_drag_writes(&app, edit), "stopped: nothing to write");
}

/// Removing a knob's modulation (the ring's right-click) takes its points away, one
/// undo step.
#[test]
fn removing_a_knobs_modulation_takes_its_points() {
    let (mut app, edit, at) = filter_app();
    let doc = app.doc.clone().unwrap();
    let head = |tick: f64| Some((Mode::Scene(0), tick));
    app.modulate_at(edit, at as f32 + 0.1, false, head(100.0));
    app.modulate_at(edit, at as f32 + 0.2, true, head(600.0));
    let points = |d: &deckard_doc::Document| d.project().groups[0].patterns[0].mods.iter().filter(|m| m.field == "cutoff").count();
    assert!(points(&doc.borrow()) > 0, "recorded");
    let steps = doc.borrow().undo_labels().len();
    app.clear_modulation(edit);
    assert!(!app.status.starts_with("can't"), "no error: {}", app.status);
    assert_eq!(points(&doc.borrow()), 0, "removed");
    assert_eq!(doc.borrow().undo_labels().len(), steps + 1);
}

/// The cutoff's modulation points in pattern 0: (position, value to 2 places).
fn cutoff_points(app: &crate::App) -> Vec<(i64, f64)> {
    let doc = app.doc.clone().unwrap();
    let d = doc.borrow();
    d.project().groups[0].patterns[0]
        .mods
        .iter()
        .filter(|m| m.sound == Some(0) && m.field == "cutoff")
        .map(|m| (m.position, (m.value * 100.0).round() / 100.0))
        .collect()
}

/// Old modulation on the cutoff, -0.5 at 1000, 2000 and 3000.
fn old_points(app: &crate::App, edit: crate::layout::Edit) {
    let (c, t) = crate::modulation::mod_target(edit).unwrap();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().set_modulation(c, &t, 0, (0, 0), &[(1000, -0.5), (2000, -0.5), (3000, -0.5)]).unwrap();
}

/// Touch writing, as the controller lanes: a knob touched and held still overwrites
/// what was there as the song passes, until it's idle for 250 ms; then the old points
/// play on.
#[test]
fn a_touched_knob_overwrites_until_idle() {
    let (mut app, edit, at) = filter_app();
    old_points(&app, edit);
    let head = |tick: f64| Some((Mode::Scene(0), tick));
    let now = std::time::Instant::now();
    app.modulate_at(edit, at as f32 + 0.2, false, head(100.0));
    app.tick_mod_take(head(2500.0), now);
    let pts = cutoff_points(&app);
    assert!(!pts.iter().any(|p| p.1 == -0.5 && p.0 <= 2500), "the old points passed are gone: {pts:?}");
    assert!(pts.iter().any(|&(x, v)| x >= 2400 && v == 0.2), "the held value written up to the song: {pts:?}");
    app.tick_mod_take(head(2700.0), now + std::time::Duration::from_millis(300));
    assert!(app.mod_take.is_none(), "idle: the touch ends");
    let pts = cutoff_points(&app);
    assert!(pts.contains(&(3000, -0.5)), "what's after it stays: {pts:?}");
}

/// Removing a knob's modulation while its touch is open (Auto-write, an MK3 knob that
/// never says it's let go) isn't written back when the touch ends.
#[test]
fn removing_modulation_ends_its_touch() {
    let (mut app, edit, at) = filter_app();
    let head = |tick: f64| Some((Mode::Scene(0), tick));
    for k in 0..6 {
        app.modulate_at(edit, at as f32 + 0.05 * k as f32, false, head(100.0 + 100.0 * k as f64));
    }
    assert!(app.mod_take.is_some());
    app.clear_modulation(edit);
    app.tick_mod_take(head(900.0), std::time::Instant::now() + std::time::Duration::from_millis(300));
    assert_eq!(cutoff_points(&app), [], "stays removed");
}

/// Held over the pattern's end: the end of the pattern and its start are overwritten.
#[test]
fn a_touch_over_the_loop_end_overwrites_both_sides() {
    let (mut app, edit, at) = filter_app();
    let len = app.doc.clone().unwrap().borrow().project().groups[0].patterns[0].length;
    let (c, t) = crate::modulation::mod_target(edit).unwrap();
    app.doc.clone().unwrap().borrow_mut().set_modulation(c, &t, 0, (0, 0), &[(200, -0.5), (len - 200, -0.5)]).unwrap();
    let head = |tick: f64| Some((Mode::Scene(0), tick));
    let now = std::time::Instant::now();
    app.modulate_at(edit, at as f32 + 0.2, false, head((len - 400) as f64));
    app.tick_mod_take(head((len + 400) as f64), now);
    let pts = cutoff_points(&app);
    assert!(!pts.iter().any(|p| p.1 == -0.5), "both old points overwritten: {pts:?}");
}

/// Stopped, a knob move is an ordinary edit.
#[test]
fn stopped_a_knob_move_is_an_edit() {
    let (mut app, edit, at) = filter_app();
    let doc = app.doc.clone().unwrap();
    app.modulate_at(edit, (at + 0.2).min(1.0) as f32, true, None);
    let d = doc.borrow();
    assert!(d.project().groups[0].patterns[0].mods.is_empty());
    let (c, t) = crate::modulation::mod_target(edit).unwrap();
    assert!((crate::modulation::knob_travel(d.project(), c, &t).unwrap() - (at + 0.2).min(1.0)).abs() < 0.01);
}

/// VARIATION on the MK3: Control mode shows the focused sound's Groove ▸ Variation page.
#[test]
fn the_mk3_variation_button_shows_the_grooves_variation() {
    let mut app = crate::App::default();
    app.new_project();
    app.control_request(deckard_lua_host::state::ControlRequest::GroovePage(1));
    assert!(app.channel, "channel properties");
    assert_eq!((app.prop_set, app.prop_page), (2, 1));
    assert!(matches!(app.chain_tab, crate::layout::ChainTab::Sound));
    let doc = app.doc.clone().unwrap();
    let (names, cur, _) = crate::layout::channel_page(&app, &doc.borrow()).expect("a page");
    assert_eq!(names.get(cur).map(String::as_str), Some("Variation"));
}

/// The scale picked in the pattern editor is the one Generate's Random keeps to, with
/// no controller connected (the controller's Keyboard scale is the same one).
#[test]
fn generate_keeps_to_the_scale_picked_in_the_software() {
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    if doc.borrow().project().groups[0].patterns.is_empty() {
        doc.borrow_mut().add_pattern(0, 0, "P").unwrap();
    }
    (app.group, app.sound, app.pattern) = (0, 0, 0);
    app.piano_roll = true;
    app.scale = (60, 0, 0); // as the app starts
    // C major (Main's second scale), then the range: 61..70 (C#3 to A#3, both outside).
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::Scale(60, 0, 1)]);
    let set = [
        ("sequence_generation/probability", 100.0),
        ("sequence_generation/note_range/low", 61.0),
        ("sequence_generation/note_range/high", 70.0),
        ("sequence_generation/note_count", 4.0),
    ]
    .map(|(f, v)| crate::layout::Act::SetParam(deckard_doc::Target::Sound(0, 0), f, v));
    crate::layout::apply_acts(&mut app, set.into_iter().collect());
    assert_eq!(app.groove_setting(0, 0, "sequence_generation/note_range/high"), Some(70.0));
    app.groove(crate::layout::GrooveButton::Generate);
    let n: Vec<i64> = doc.borrow().project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 0).map(|n| n.pitch).collect();
    assert!(!n.is_empty());
    let off: Vec<i64> =
        n.iter().copied().filter(|&p| !deckard_lua_host::api::scales::in_scale(0, 1, 60, p) || !(61..=70).contains(&p)).take(8).collect();
    assert!(off.is_empty(), "outside C major or the range: {off:?}");
}

/// Changing the scale's root moves the focused sound's Generate note range with it.
#[test]
fn the_note_range_follows_the_root() {
    let mut app = crate::App::default();
    app.new_project();
    (app.group, app.sound) = (0, 0);
    app.scale = (60, 0, 0); // as the app starts
    let range = |app: &crate::App| {
        (app.groove_setting(0, 0, "sequence_generation/note_range/low"), app.groove_setting(0, 0, "sequence_generation/note_range/high"))
    };
    assert_eq!(range(&app), (Some(60.0), Some(72.0)));
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::Scale(62, 0, 1)]);
    assert_eq!(range(&app), (Some(62.0), Some(74.0)), "C to D: up two");
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::Scale(57, 0, 1)]);
    assert_eq!(range(&app), (Some(57.0), Some(69.0)), "D to A below: down five");
    // The scale alone (same root): the range stays.
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::Scale(57, 0, 2)]);
    assert_eq!(range(&app), (Some(57.0), Some(69.0)));
}

/// Maschine's undo shortcuts: ⌘Z / ⌘Y a take, ⌘⇧Z / ⌘⇧Y a single step.
#[test]
fn the_edit_menu_has_take_and_step_undo() {
    let mut app = crate::App::default();
    app.new_project();
    let menus = crate::menu::menus(&app);
    let edit = &menus.iter().find(|(m, _)| *m == "Edit").unwrap().1;
    let keys: Vec<(String, crate::menu::Command, Option<crate::menu::Shortcut>)> = edit
        .iter()
        .filter_map(|it| match it {
            crate::menu::Item::Command { label, cmd, shortcut, .. } => Some((label.clone(), cmd.clone(), *shortcut)),
            _ => None,
        })
        .collect();
    let key = |c: crate::menu::Command| keys.iter().find(|k| k.1 == c).and_then(|k| k.2).map(|s| (s.key, s.shift));
    use crate::menu::Command as C;
    use eframe::egui::Key;
    assert_eq!(key(C::Undo(1)), Some((Key::Z, false)));
    assert_eq!(key(C::Redo(1)), Some((Key::Y, false)));
    assert_eq!(key(C::UndoStep), Some((Key::Z, true)));
    assert_eq!(key(C::RedoStep), Some((Key::Y, true)));
    assert!(keys.iter().any(|k| k.0 == "Undo Step"), "{:?}", keys.iter().map(|k| &k.0).collect::<Vec<_>>());
}

/// ⌘⇧Z undoes one edit of a recorded take; ⌘Z the rest of it.
#[test]
fn step_undo_from_the_keyboard() {
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    {
        let mut d = doc.borrow_mut();
        if d.project().groups[0].patterns.is_empty() {
            d.add_pattern(0, 0, "P").unwrap();
        }
        let note = |k: i64| deckard_model::Note {
            expr: None,
            sound: 0,
            position: k * 960,
            length: 240,
            pitch: 60,
            velocity: 90,
            probability: 100,
        };
        d.begin_take("Record");
        d.edit_notes("Record A", 0, 0, &[], &[note(0)]).unwrap();
        d.edit_notes("Record B", 0, 0, &[], &[note(1)]).unwrap();
        d.end_take();
    }
    let notes = || doc.borrow().project().groups[0].patterns[0].notes.len();
    app.run_command(&eframe::egui::Context::default(), crate::menu::Command::UndoStep);
    assert_eq!(notes(), 1);
    app.run_command(&eframe::egui::Context::default(), crate::menu::Command::RedoStep);
    assert_eq!(notes(), 2);
    app.run_command(&eframe::egui::Context::default(), crate::menu::Command::Undo(1));
    assert_eq!(notes(), 0);
}

/// The editing keys act on the part of the window clicked last: in the Sound list,
/// ⌘C / ⌘V copy and paste the focused sound, Del resets it, ⌘R renames it; in the Group
/// list, ⌘R renames the group.
#[test]
fn editing_keys_follow_the_list_clicked_last() {
    use crate::layout::{Act, KeyFocus};
    use eframe::egui::{Key, Modifiers};
    let mut app = crate::App::default();
    app.new_project();
    let ctx = eframe::egui::Context::default();
    let press = |app: &mut crate::App, m: Modifiers, k: Key| {
        let ev = eframe::egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m };
        let input = eframe::egui::RawInput { events: vec![ev], ..Default::default() };
        ctx.run_ui(input, |ui| app.editing_keys(ui.ctx())).textures_delta.clear();
    };
    crate::layout::apply_acts(&mut app, vec![Act::FocusSound(2), Act::KeyFocus(KeyFocus::Sounds)]);
    press(&mut app, Modifiers::COMMAND, Key::C);
    assert_eq!(app.sound_clipboard.len(), 1, "the sound copied");
    press(&mut app, Modifiers::COMMAND, Key::R);
    assert_eq!(app.renaming, Some(deckard_doc::Target::Sound(0, 2)));
    app.renaming = None;
    crate::layout::apply_acts(&mut app, vec![Act::KeyFocus(KeyFocus::Groups)]);
    press(&mut app, Modifiers::COMMAND, Key::R);
    assert_eq!(app.renaming, Some(deckard_doc::Target::Group(0)));
    app.renaming = None;
    // In the event area, ⌘C copies notes, not the sound.
    crate::layout::apply_acts(&mut app, vec![Act::KeyFocus(KeyFocus::Events)]);
    app.sound_clipboard.clear();
    press(&mut app, Modifiers::COMMAND, Key::C);
    assert!(app.sound_clipboard.is_empty());
}

/// A new project whose first group has a pattern and a Perform FX.
fn perform_app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    let mut d = doc.borrow_mut();
    d.add_pattern(0, 0, "Pattern 1").unwrap();
    d.assign_pattern(0, 0, Some(0)).unwrap();
    d.add_perform_fx(0).unwrap();
    drop(d);
    crate::modulation::register();
    app
}

/// The Smart Strip: touching engages the group's Perform FX where it's touched, moving
/// moves it, letting go releases it.
#[test]
fn the_strip_plays_the_perform_fx() {
    let mut app = perform_app();
    let doc = app.doc.clone().unwrap();
    let v = |f: &str| doc.borrow().project().groups[0].slots[0].param(f);
    app.perform_strip_at(0, Some(true), 0.3, None);
    assert_eq!(v("touchstrip_on_off"), Some(1.0));
    assert!((v("touchstrip_pos").unwrap() - 0.3).abs() < 1e-6);
    app.perform_strip_at(0, None, 0.6, None);
    assert!((v("touchstrip_pos").unwrap() - 0.6).abs() < 1e-6);
    app.perform_strip_at(0, Some(false), 0.6, None);
    assert_eq!(v("touchstrip_on_off"), Some(0.0));
}

/// Writing (Auto-write, playing): the strip is recorded as the group's modulation: Engage
/// on at the touch and off at the release, the position as it moved. The module's own
/// values stay.
#[test]
fn the_strip_records_while_writing() {
    let mut app = perform_app();
    app.auto_write = true;
    let doc = app.doc.clone().unwrap();
    let head = |tick: f64| Some((Mode::Scene(0), tick));
    app.perform_strip_at(0, Some(true), 0.3, head(100.0));
    app.perform_strip_at(0, None, 0.6, head(200.0));
    app.perform_strip_at(0, Some(false), 0.6, head(400.0));
    let d = doc.borrow();
    let pts = |f: &str| -> Vec<(i64, f64)> {
        d.project().groups[0].patterns[0]
            .mods
            .iter()
            .filter(|m| m.sound.is_none() && m.field == f)
            .map(|m| (m.position, (m.value * 100.0).round() / 100.0))
            .collect()
    };
    assert_eq!(pts("touchstrip_on_off"), [(100, 1.0), (400, 0.0)]);
    let pos = pts("touchstrip_pos");
    assert_eq!(pos.first(), Some(&(100, -0.2)), "a deviation from the knob's 50 %: {pos:?}");
    assert!(pos.iter().any(|p| p.1 == 0.1), "{pos:?}");
    assert_eq!(d.project().groups[0].slots[0].param("touchstrip_on_off"), Some(0.0), "the module's own value stays");
}

/// "+" ▸ Internal ▸ Perform FX in a group's chain puts it first and shows it.
#[test]
fn perform_fx_is_added_first_in_the_groups_chain() {
    let mut app = crate::App::default();
    app.new_project();
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::AddPerformFx(0, 0)]);
    let doc = app.doc.clone().unwrap();
    assert_eq!(doc.borrow().project().groups[0].slots[0].module, "PerformanceFX");
    assert_eq!(app.perform_slot(0), Some(0));
    assert_eq!(app.slot, Some(0), "shown");
    assert!(matches!(app.chain_tab, crate::layout::ChainTab::Group));
}

/// The software's ENGAGE button plays the strip's touch (so it records while writing).
#[test]
fn the_engage_button_engages() {
    let mut app = perform_app();
    let doc = app.doc.clone().unwrap();
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::PerformEngage(0, true)]);
    assert_eq!(doc.borrow().project().groups[0].slots[0].param("touchstrip_on_off"), Some(1.0));
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::PerformEngage(0, false)]);
    assert_eq!(doc.borrow().project().groups[0].slots[0].param("touchstrip_on_off"), Some(0.0));
}

/// The MK3's requests: SHIFT + PERFORM shows the group's Perform FX; the strip in
/// PERFORM mode plays it.
#[test]
fn the_mk3_strip_and_perform_button_reach_the_app() {
    use deckard_lua_host::state::ControlRequest as R;
    let mut app = perform_app();
    let doc = app.doc.clone().unwrap();
    app.control_request(R::ShowPerformFx(0));
    assert_eq!((app.slot_of, app.slot), (Some(deckard_doc::Chain::Group(0)), Some(0)));
    app.control_request(R::PerformStrip { group: 0, touch: Some(true), pos: 0.75 });
    assert_eq!(doc.borrow().project().groups[0].slots[0].param("touchstrip_on_off"), Some(1.0));
    app.control_request(R::PerformStrip { group: 0, touch: Some(false), pos: 0.75 });
    assert_eq!(doc.borrow().project().groups[0].slots[0].param("touchstrip_on_off"), Some(0.0));
}

/// In a group's plug-ins, "+" offers Perform FX first (not under Internal), while the
/// group has none.
#[test]
fn the_groups_internal_menu_offers_the_perform_fx() {
    use egui_kittest::kittest::Queryable;
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    crate::layout::apply_acts(h.state_mut(), vec![crate::layout::Act::ChainTab(crate::layout::ChainTab::Group)]);
    h.run_steps(3);
    let g = h.state().group;
    assert_eq!(h.state().perform_slot(g), None);
    h.get_by_label("Add plug-in").click();
    h.run_steps(3);
    h.get_by_label("Internal ⏵").click();
    h.run_steps(3);
    h.get_by_label("Perform FX ⏵").click();
    h.run_steps(3);
    h.get_by_label("Stutter").click();
    h.run_steps(3);
    assert_eq!(h.state().perform_slot(g), Some(0));
    let doc = h.state().doc.clone().unwrap();
    assert_eq!(doc.borrow().project().groups[g].slots[0].param("mode"), Some(5.0), "Stutter");
}

/// A group's and the master's chains take effects only: their "+" offers no
/// instruments; the master has no Perform FX.
#[test]
fn group_and_master_chains_offer_no_instruments() {
    use egui_kittest::kittest::Queryable;
    for tab in [crate::layout::ChainTab::Group, crate::layout::ChainTab::Master] {
        let Some(mut h) = crate::tests::harness(false, false) else { return };
        crate::layout::apply_acts(h.state_mut(), vec![crate::layout::Act::ChainTab(tab)]);
        h.run_steps(3);
        h.get_by_label("Add plug-in").click();
        h.run_steps(3);
        h.get_by_label("Internal ⏵").click();
        h.run_steps(3);
        for name in ["Sampler", "Audio", "Bass Synth", "Kick", "Poly Synth"] {
            assert!(h.query_by_label(name).is_none(), "{tab:?} offers {name}");
        }
        assert!(h.query_by_label("Compressor").is_some(), "{tab:?} offers effects");
        let master = matches!(tab, crate::layout::ChainTab::Master);
        assert_eq!(h.query_by_label("Perform FX ⏵").is_some(), !master, "{tab:?}");
    }
}

/// Mute toggles on a sound whose Compressor is side-chained from another sound.
#[test]
fn a_side_chained_sound_toggles_mute() {
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Effect("Compressor")).unwrap();
    doc.borrow_mut().set_sidechain(0, 0, 0, Some(deckard_model::ScSource::Sound(0, 1))).unwrap();
    let muted = || doc.borrow().project().groups[0].sounds[0].muted;
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::ToggleMute(deckard_doc::Target::Sound(0, 0))]);
    assert!(muted(), "{}", app.status);
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::ToggleMute(deckard_doc::Target::Sound(0, 0))]);
    assert!(!muted(), "{}", app.status);
}

/// Stutter's Length reads as Maschine shows it: 1/2 at the strip's start to 1/64 at its
/// end with Quantize, on Maschine's curve (on the strip, or on the LENGTH knob when the
/// strip plays Pitch); without, the loop's time at the tempo.
#[test]
fn stutter_shows_its_length() {
    use deckard_doc::{Chain, Target};
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_perform_fx(0).unwrap();
    let t = Target::ChainModule(Chain::Group(0), 0);
    let set = |f: &str, v: f64| doc.borrow_mut().set("t", t, f, mxprj::decode::Value::Float(v)).unwrap();
    let text = |label: &str| {
        let slot = doc.borrow().project().groups[0].slots[0].clone();
        let pg = crate::module_ui::page_for(&slot, t, 0);
        pg.slots.iter().flatten().find(|k| k.0 == label).map(|k| k.1.clone()).unwrap_or_else(|| panic!("no {label}"))
    };
    set("mode", 5.0);
    for (pos, shown) in [(0.0, "1/2"), (0.07, "1/2"), (0.09, "1/4"), (0.2, "1/4"), (0.55, "1/16"), (0.9, "1/64"), (1.0, "1/64")] {
        set("touchstrip_pos", pos);
        assert_eq!(text("LENGTH"), shown, "strip at {pos}");
    }
    set("stutter_ts_assign", 1.0);
    set("stutter_length", 0.4);
    assert_eq!(text("LENGTH"), "1/8", "the knob");
    set("stutter_quantize", 0.0);
    set("stutter_length", 1.0);
    assert_eq!(text("LENGTH"), "22.1 ms", "at 120 BPM");
    set("stutter_length", 0.0);
    assert_eq!(text("LENGTH"), "707.1 ms");
    crate::module_ui::set_tempo(60.0);
    assert_eq!(text("LENGTH"), "1.4 s", "at 60 BPM");
    crate::module_ui::set_tempo(120.0);
}

/// The strip's position is kept as touched, as Maschine keeps it (a quantized Stutter's
/// Length steps only in what it plays and shows), and stays once let go.
#[test]
fn the_strip_is_kept_where_touched() {
    use deckard_doc::{Chain, Target};
    let mut app = perform_app();
    let doc = app.doc.clone().unwrap();
    let t = Target::ChainModule(Chain::Group(0), 0);
    doc.borrow_mut().set("t", t, "mode", mxprj::decode::Value::Float(5.0)).unwrap();
    let pos = || doc.borrow().project().groups[0].slots[0].param("touchstrip_pos");
    app.perform_strip_at(0, Some(true), 0.07, None);
    assert_eq!(pos(), Some(0.07));
    app.perform_strip_at(0, None, 0.13, None);
    assert_eq!(pos(), Some(0.13));
    app.perform_strip_at(0, Some(false), 0.13, None);
    assert_eq!(pos(), Some(0.13), "let go: kept");
}

/// Stutter's Direction offers Forward, Reverse and Both (the strip's movement).
#[test]
fn stutter_direction_offers_both() {
    use deckard_doc::{Chain, Target};
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_perform_fx(0).unwrap();
    let t = Target::ChainModule(Chain::Group(0), 0);
    doc.borrow_mut().set("t", t, "mode", mxprj::decode::Value::Float(5.0)).unwrap();
    let knob = || {
        let slot = doc.borrow().project().groups[0].slots[0].clone();
        crate::module_ui::page_for(&slot, t, 0).slots.iter().flatten().find(|k| k.0 == "DIRECTION").cloned().expect("DIRECTION")
    };
    let Some(crate::layout::Edit::Choice(_, _, options, ..)) = knob().3 else { panic!("a choice") };
    assert_eq!(options, ["Forward", "Reverse", "Both"]);
    assert_eq!(knob().1, "Both", "Maschine's default");
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::SetParam(t, "stutter_direction", 0.0)]);
    assert_eq!(knob().1, "Forward");
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::SetParam(t, "stutter_direction", 2.0)]);
    assert_eq!(knob().1, "Both");
}

/// The MK3's knob steps through every choice: Stutter's Direction reaches Both (a
/// choice knob turns from the choice it's on).
#[test]
fn the_mk3_knob_reaches_every_choice() {
    use deckard_doc::{Chain, Target};
    use deckard_lua_host::state::ControlRequest as R;
    let mut app = perform_app();
    let doc = app.doc.clone().unwrap();
    let t = Target::ChainModule(Chain::Group(0), 0);
    doc.borrow_mut().set("t", t, "mode", mxprj::decode::Value::Float(5.0)).unwrap();
    doc.borrow_mut().set("t", t, "stutter_direction", mxprj::decode::Value::Float(0.0)).unwrap();
    app.chain_tab = crate::layout::ChainTab::Group;
    app.slot = Some(0);
    app.channel = false;
    let dir = || doc.borrow().project().groups[0].slots[0].param("stutter_direction");
    // DIRECTION is the seventh knob.
    app.control_request(R::Turn { pos: 6, inc: 0.5, fine: false });
    assert_eq!(dir(), Some(1.0), "Reverse");
    app.control_request(R::Turn { pos: 6, inc: 0.5, fine: false });
    assert_eq!(dir(), Some(2.0), "Both");
    app.control_request(R::Turn { pos: 6, inc: -0.5, fine: false });
    assert_eq!(dir(), Some(1.0), "back to Reverse");
}

/// NOTE REPEAT's preset buttons (5–8) are the project's presets 1–4: a new project's
/// 1/8, 1/16, 1/16T, 1/32, the 1/16 lit to begin with. Choosing one plays its rate; the
/// rate and unit turned go into the one chosen (which stays chosen) and the project.
#[test]
fn the_mk3_note_repeat_presets_are_kept() {
    let Some(mut h) = harness(true, false) else { return };
    if !matches!(h.state().controller, Some(Ok(_))) {
        return;
    }
    h.state_mut().new_project();
    h.run_steps(3);
    let lua = |h: &mut egui_kittest::Harness<'static, crate::App>, code: &str| -> String {
        let Some(Ok(c)) = &mut h.state_mut().controller else { unreachable!() };
        c.host.lua.load(code).eval::<String>().unwrap()
    };
    let names = |h: &mut egui_kittest::Harness<'static, crate::App>| {
        lua(
            h,
            "local p = NI.DATA.getArpeggiatorPresetParameter(App); local t = {} \
             for i = 1, 4 do t[#t + 1] = p:getAsString(i) end return table.concat(t, ',')",
        )
    };
    let chosen = |h: &mut egui_kittest::Harness<'static, crate::App>| {
        lua(h, "return string.format('%d', NI.DATA.getArpeggiatorPresetParameter(App):getValue())")
    };
    assert_eq!(names(&mut h), "1/8,1/16,1/16T,1/32");
    assert_eq!(chosen(&mut h), "2", "the 1/16 lit to begin with");
    // Button 5: preset 1, 1/8.
    lua(&mut h, "NI.DATA.ParameterAccess.setSizeTParameterNoUndo(App, NI.DATA.getArpeggiatorPresetParameter(App), 1) return ''");
    h.run_steps(3);
    let doc = h.state().doc.clone().unwrap();
    let bar = doc.borrow().project().bar_length();
    assert_eq!(crate::arp_ui::rate_of(doc.borrow().project().arp.rate, bar), (3, 0), "1/8 plays");
    assert_eq!(chosen(&mut h), "1");
    // RATE to 1/16, UNIT to Triplet: kept in preset 1, which stays chosen.
    lua(&mut h, "NI.DATA.getArpeggiatorRateParameter(App):setValue(4) return ''");
    h.run_steps(3);
    lua(&mut h, "NI.DATA.getArpeggiatorRateUnitParameter(App):setValue(1) return ''");
    h.run_steps(3);
    assert_eq!(names(&mut h), "1/16T,1/16,1/16T,1/32");
    assert_eq!(chosen(&mut h), "1", "still the first");
    assert_eq!(doc.borrow().project().arp_presets[0], (4, 1), "in the project");
}

/// A pad held while recording is in the pattern from its press, and a chord held over
/// the loop end keeps every note, each as long as it was held (the user's report).
#[test]
fn a_held_pad_shows_while_held() {
    let Some(mut h) = harness(false, false) else { return };
    let (g, scene, pi) = scene_pattern(&h);
    let doc = h.state().doc.clone().unwrap();
    let (start, len) = {
        let d = doc.borrow();
        let p = &d.project().groups[g].patterns[pi];
        (p.start, p.length)
    };
    let notes = |doc: &std::rc::Rc<std::cell::RefCell<deckard_doc::Document>>| doc.borrow().project().groups[g].patterns[pi].notes.clone();
    h.state_mut().set_recording(true, false);
    let on = |pitch: i64| Recorded {
        group: g,
        sound: 5,
        pitch,
        velocity: 100,
        length: i64::MAX,
        tick: 0.0,
        mode: Mode::Scene(scene),
        seq: 0,
        played: 0.0,
    };
    h.state_mut().take_in_recorded(vec![on(60), on(64), on(67)]);
    let held: Vec<_> = notes(&doc).into_iter().filter(|n| n.sound == 5 && n.position == start).collect();
    assert_eq!(held.len(), 3, "the chord shows while held: {held:?}");
    // Released half a pattern after the loop wrapped.
    let off = |pitch: i64| Recorded { velocity: 0, tick: len as f64 / 2.0, played: len as f64 * 1.5, ..on(pitch) };
    h.state_mut().take_in_recorded(vec![off(60), off(64), off(67)]);
    h.state_mut().set_recording(false, false);
    let done: Vec<_> = notes(&doc).into_iter().filter(|n| n.sound == 5 && n.position == start).collect();
    assert_eq!(done.len(), 3, "{done:?}");
    assert!(done.iter().all(|n| n.length == len + len / 2), "as long as held: {done:?}");
}
