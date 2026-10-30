//! Run the controller scripts from the user's own Maschine install.
//!
//! Needs the `ni-res-extract` output (plus `constants.tsv` from `enum-values`) in
//! `$DECKARD_SCRIPTS` (default `<workspace>/out/ni-res`); skipped when it's missing.
//! The project-driven checks use `$DECKARD_TEST_PROJECT`, default Cloud 9 Vibes.

use deckard_lua_host::{Host, Scripts, State};
use std::path::PathBuf;

fn scripts() -> Option<Scripts> {
    let dir = std::env::var_os("DECKARD_SCRIPTS")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/ni-res"), PathBuf::from);
    let s = Scripts::from_dir(&dir).ok().filter(|s| !s.is_empty());
    if s.is_none() {
        eprintln!("no scripts under {}; skipping", dir.display());
    }
    s
}

fn project() -> Option<deckard_doc::Shared> {
    let path = std::env::var_os("DECKARD_TEST_PROJECT")
        .map_or_else(|| PathBuf::from("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj"), PathBuf::from);
    Some(deckard_doc::Document::open(&path).ok()?.shared())
}

fn boot(controller: &str, project: Option<deckard_doc::Shared>) -> Option<Host> {
    let mut host = Host::new(scripts()?, State::new(controller, project)).expect("host");
    host.boot().unwrap_or_else(|e| panic!("{controller}: boot failed: {e}"));
    Some(host)
}

#[test]
fn every_controller_boots_and_ticks() {
    for c in ["MM1", "M2", "M3", "MH1071", "MS1", "MJ1", "KKS2_61", "KKA", "MM3"] {
        let Some(mut host) = boot(c, project()) else { return };
        for _ in 0..5 {
            host.tick();
        }
        assert!(host.errors.is_empty(), "{c}: {:?}", host.errors);
    }
}

#[test]
fn mk3_transport_pads_and_groups() {
    let Some(p) = project() else { return };
    let groups = p.borrow().project().groups.len();
    let Some(mut host) = boot("M3", Some(p)) else { return };
    host.tick();
    let lit = |host: &Host, led: &str| {
        let id = host.constant(&format!("NI.HW.{led}")).unwrap();
        host.state.borrow().hw.leds.get(&id).is_some_and(|l| l.is_lit())
    };
    assert_eq!(host.state.borrow().hw.page_stack, vec![host.constant("NI.HW.PAGE_CONTROL").unwrap()]);
    assert!(!lit(&host, "LED_TRANSPORT_PLAY"));

    host.press("BUTTON_TRANSPORT_PLAY").unwrap();
    host.tick();
    assert!(host.state.borrow().transport.playing);
    assert!(lit(&host, "LED_TRANSPORT_PLAY"));
    host.press("BUTTON_TRANSPORT_PLAY").unwrap();
    host.tick();
    assert!(!host.state.borrow().transport.playing);

    // Group buttons light for existing groups and change focus.
    assert_eq!(lit(&host, "LED_GROUP_A"), groups > 0);
    host.press("BUTTON_GROUP_B").unwrap();
    host.tick();
    assert_eq!(host.state.borrow().focus.group, 1);

    // Pads are 1-based in the scripts; hitting one focuses its sound.
    host.pad(3, 1.0);
    host.tick();
    assert_eq!(host.state.borrow().focus.sound, 2);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

#[test]
fn control_page_shows_group_and_tempo() {
    let Some(p) = project() else { return };
    let (group, tempo) = {
        let d = p.borrow();
        (d.project().groups[0].name.clone(), format!("{:.2}", d.project().tempo))
    };
    let Some(mut host) = boot("M3", Some(p)) else { return };
    host.tick();
    let st = host.state.borrow();
    let stack = &st.widgets.all[st.hw.display_roots["PageStack"]];
    let page = |prefix: &str| {
        let id = stack.children.iter().find(|&&c| st.widgets.all[c].name.starts_with(prefix)).expect(prefix);
        st.widgets.outline(*id)
    };
    let control = page("ControlPage");
    assert!(control.contains(&format!("InfoBarName = {group:?}")), "{control}");
    let module = page("ModulePage");
    assert!(module.contains(&format!("InfoBarLabelValue = {tempo:?}")), "{module}");
}

#[test]
fn mk3_scene_mode_pads_launch_scenes() {
    let Some(p) = project() else { return };
    let scenes = p.borrow().project().scenes.len();
    if scenes < 2 {
        return;
    }
    let Some(mut host) = boot("M3", Some(p)) else { return };
    host.tick();
    // SCENE is a modifier page: it shows while held.
    host.button("BUTTON_SCENE", true).unwrap();
    host.tick();
    assert_eq!(host.state.borrow().hw.pad_mode, host.constant("NI.HW.PAD_MODE_SCENE").unwrap());
    assert!(!host.pads_play_sounds());
    // Pads light for existing scenes.
    let lit = |host: &Host, led: &str| {
        let id = host.constant(&format!("NI.HW.{led}")).unwrap();
        host.state.borrow().hw.leds.get(&id).is_some_and(|l| l.is_lit())
    };
    assert!(lit(&host, "LED_PAD_1") && lit(&host, "LED_PAD_2"));
    host.pad(2, 1.0);
    host.tick();
    let st = host.state.borrow();
    assert_eq!((st.focus.scene, st.launched_scene), (Some(1), Some(1)));
    drop(st);
    host.button("BUTTON_SCENE", false).unwrap();
    host.tick();
    assert!(host.pads_play_sounds());
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The scripts edit the shared document: an encoder change of the tempo is an undoable
/// edit the rest of the app sees.
#[test]
fn script_edits_reach_the_shared_document() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    let before = doc.borrow().project().tempo;
    host.lua
        .load("local p = App:getWorkspace() and NI.DATA.StateHelper.getFocusSong(App):getTempoParameter(); NI.DATA.ParameterAccess.setFloatParameter(App, p, 101.5)")
        .exec()
        .unwrap();
    assert_eq!(doc.borrow().project().tempo, 101.5);
    assert!(doc.borrow().is_dirty());
    assert_eq!(doc.borrow().undo_label(), Some("Tempo"));
    // The scripts read it back through the same document.
    let shown: f64 = host.lua.load("return NI.DATA.StateHelper.getFocusSong(App):getTempoParameter():getValue()").eval().unwrap();
    assert_eq!(shown, 101.5);
    assert!(doc.borrow_mut().undo());
    let shown: f64 = host.lua.load("return NI.DATA.StateHelper.getFocusSong(App):getTempoParameter():getValue()").eval().unwrap();
    assert_eq!(shown, before);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// In the Song view the SCENE page picks sections (pad mode SECTION).
#[test]
fn mk3_scene_button_picks_sections_in_the_song_view() {
    let Some(doc) = project() else { return };
    let sections = doc.borrow().project().sections.len();
    if sections < 3 {
        return;
    }
    let scene_of_3 = doc.borrow().project().sections[2].scene;
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    host.state.borrow_mut().ideas_view = false;
    host.tick();
    host.button("BUTTON_SCENE", true).unwrap();
    host.tick();
    assert_eq!(host.state.borrow().hw.pad_mode, host.constant("NI.HW.PAD_MODE_SECTION").unwrap());
    assert!(!host.pads_play_sounds());
    host.pad(3, 1.0);
    host.tick();
    let st = host.state.borrow();
    assert_eq!((st.focus.section, st.launched_section), (Some(2), Some(2)));
    assert_eq!(st.focus.scene, scene_of_3);
    drop(st);
    host.button("BUTTON_SCENE", false).unwrap();
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The SCENE page's screen buttons edit the shared document through NI's scripts:
/// 4 duplicates the focused scene, 5 creates one after it, 6 deletes it, and in
/// APPEND mode (3) pads add sections.
#[test]
fn mk3_scene_page_buttons_edit_scenes() {
    let Some(doc) = project() else { return };
    let (scenes, sections) = {
        let d = doc.borrow();
        (d.project().scenes.len(), d.project().sections.len())
    };
    let name0 = doc.borrow().project().scenes[0].name.clone();
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    host.button("BUTTON_SCENE", true).unwrap();
    host.tick();
    let labels: Vec<String> = host.screen_buttons().into_iter().map(|b| b.label).collect();
    eprintln!("scene page buttons: {labels:?}");
    assert_eq!(labels.get(3).map(String::as_str), Some("DUPLICATE"));
    host.press("BUTTON_SCREEN_4").unwrap();
    host.tick();
    assert_eq!(doc.borrow().project().scenes.len(), scenes + 1);
    assert_eq!(doc.borrow().project().scenes[1].name, format!("{name0} (1)"));
    assert_eq!(doc.borrow().undo_label(), Some("Duplicate Scene"));
    assert_eq!(host.state.borrow().focus.scene, Some(1));
    host.press("BUTTON_SCREEN_5").unwrap();
    host.tick();
    assert_eq!(doc.borrow().project().scenes.len(), scenes + 2);
    assert_eq!(host.state.borrow().focus.scene, Some(2));
    assert!(doc.borrow().project().scenes[2].patterns.is_empty());
    host.press("BUTTON_SCREEN_6").unwrap();
    host.tick();
    assert_eq!(doc.borrow().project().scenes.len(), scenes + 1);
    // APPEND, then pad 1 appends a section playing scene 1.
    host.press("BUTTON_SCREEN_3").unwrap();
    host.tick();
    host.pad(1, 1.0);
    host.tick();
    assert_eq!(doc.borrow().project().sections.len(), sections + 1);
    assert_eq!(doc.borrow().project().sections[sections].scene, Some(0));
    host.button("BUTTON_SCENE", false).unwrap();
    assert!(host.errors.is_empty(), "{:?}", host.errors);
    let log = host.state.borrow().log.join("\n");
    assert!(!log.contains("edit:"), "{log}");
}

#[test]
fn mk3_keyboard_and_chord_pads_take_the_sound_colour() {
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    let colour: i64 = host.lua.load("return NI.DATA.StateHelper.getFocusSound(App):getColorParameter():getValue()").eval().unwrap();
    for button in ["BUTTON_KEYBOARD", "BUTTON_CHORDS"] {
        host.press(button).unwrap();
        host.tick();
        for i in 1..=16 {
            let led = host.led(&format!("LED_PAD_{i}")).unwrap_or_else(|| panic!("{button}: pad {i} unset"));
            assert!(led.is_lit(), "{button}: pad {i} dark");
            assert_eq!(led.lut.map(|(c, _)| c), Some(colour), "{button}: pad {i}");
        }
    }
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

#[test]
fn mk3_pattern_mode_empty_pad_makes_a_pattern() {
    let Some(p) = project() else { return };
    let before = p.borrow().project().groups[0].patterns.len();
    assert!(before < 16, "group A has a free pad");
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    host.button("BUTTON_PATTERN", true).unwrap();
    host.tick();
    host.pad(before as i64 + 1, 1.0);
    host.tick();
    let project = p.borrow().project().clone();
    assert_eq!(project.groups[0].patterns.len(), before + 1);
    assert_eq!(host.state.borrow().focus.pattern, Some(before));
    let scene = host.state.borrow().focus.scene.unwrap();
    assert!(project.scenes[scene].patterns.contains(&(0, before)));
    assert!(host.led(&format!("LED_PAD_{}", before + 1)).is_some_and(|l| l.is_lit()));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// PATTERN + CREATE (button 5): a new empty pattern after the focused one, focused and
/// in the scene.
#[test]
fn mk3_pattern_page_create_adds_an_empty_pattern() {
    let Some(p) = project() else { return };
    let before = p.borrow().project().groups[0].patterns.len();
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    let focused = host.state.borrow().focus.pattern.expect("a focused pattern");
    host.button("BUTTON_PATTERN", true).unwrap();
    host.tick();
    assert_eq!(host.screen_buttons()[4].label, "CREATE");
    host.press("BUTTON_SCREEN_5").unwrap();
    host.tick();
    let project = p.borrow().project().clone();
    assert_eq!(project.groups[0].patterns.len(), before + 1);
    assert_eq!(host.state.borrow().focus.pattern, Some(focused + 1));
    assert!(project.groups[0].patterns[focused + 1].notes.is_empty(), "empty");
    let scene = host.state.borrow().focus.scene.unwrap();
    assert!(project.scenes[scene].patterns.contains(&(0, focused + 1)));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The Pattern page's knobs: POSITION (1) moves the pattern, START (3) moves its start
/// with the end held, LENGTH (4) its end, by bars; page 2's COLOR (1) steps the colour.
#[test]
fn mk3_pattern_page_knobs_edit_start_length_and_color() {
    let Some(p) = project() else { return };
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    let (g, pi) = (host.state.borrow().focus.group, host.state.borrow().focus.pattern.expect("a focused pattern"));
    let span = || {
        let pr = p.borrow().project().clone();
        let pat = &pr.groups[g].patterns[pi];
        (pat.start, pat.length, pat.color)
    };
    let bar = p.borrow().project().bar_length();
    let (start, length, _) = span();
    assert_eq!(start, 0);
    host.button("BUTTON_PATTERN", true).unwrap();
    host.tick();
    let turn = |host: &mut Host, knob: usize, inc: f64| {
        host.encoder(&format!("ENCODER_SCREEN_{knob}"), inc).unwrap();
        host.tick();
    };
    turn(&mut host, 4, 0.3);
    assert_eq!(span().1, length + bar, "LENGTH adds a bar");
    turn(&mut host, 3, 0.3);
    assert_eq!(span().0..span().0 + span().1, bar..length + bar, "START moves the start, not the end");
    turn(&mut host, 1, 0.3);
    assert_eq!((span().0, span().1), (2 * bar, length), "POSITION moves both");
    host.press("BUTTON_RIGHT").unwrap();
    host.tick();
    turn(&mut host, 1, 0.3);
    turn(&mut host, 1, 0.3);
    assert_eq!(span().2, deckard_model::Color::Palette(1), "Default, then Red, then Orange");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// SCENE + knob 1 steps the focused scene's colour; the group and sound colour setters
/// write the document too.
#[test]
fn mk3_color_knobs_color_scenes_groups_and_sounds() {
    let Some(p) = project() else { return };
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    let scene = host.state.borrow().focus.scene.expect("a focused scene");
    let before = p.borrow().project().scenes[scene].color;
    host.button("BUTTON_SCENE", true).unwrap();
    host.tick();
    host.encoder("ENCODER_SCREEN_1", 0.3).unwrap();
    host.tick();
    let after = p.borrow().project().scenes[scene].color;
    assert_ne!(after, before, "the scene's colour changed");
    host.lua
        .load(
            "NI.DATA.GroupAccess.setGroupColor(App, NI.DATA.StateHelper.getFocusGroup(App), 5, false)
             NI.DATA.SoundAccess.setSoundColor(App, NI.DATA.StateHelper.getFocusSound(App), 9, false)",
        )
        .exec()
        .unwrap();
    let (g, s) = (host.state.borrow().focus.group, host.state.borrow().focus.sound);
    let project = p.borrow().project().clone();
    assert_eq!(project.groups[g].color, deckard_model::Color::Palette(5));
    assert_eq!(project.groups[g].sounds[s].color, deckard_model::Color::Palette(9));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// A group's or sound's colour changed from the software, or by the scripts' colour
/// calls, is flagged changed on the next update, so the screens redraw it at once.
#[test]
fn colour_changes_reach_the_screens_at_once() {
    let Some(p) = project() else { return };
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    host.eval_lua(
        "local process = ControllerScriptInterface.onCustomProcess
         ControllerScriptInterface.onCustomProcess = function(...)
             GROUP_CHANGED = NI.DATA.StateHelper.getFocusGroup(App):getColorParameter():isChanged()
             SOUND_CHANGED = NI.DATA.StateHelper.getFocusSound(App):getColorParameter():isChanged()
             return process(...)
         end",
    );
    host.tick();
    assert!(!host.lua_global::<bool>("GROUP_CHANGED") && !host.lua_global::<bool>("SOUND_CHANGED"), "nothing changed yet");
    let g = host.state.borrow().focus.group;
    let next = |c| match c {
        deckard_model::Color::Palette(i) => deckard_model::Color::Palette((i + 1) % 16),
        _ => deckard_model::Color::Palette(3),
    };
    let gc = next(p.borrow().project().groups[g].color);
    p.borrow_mut().set_color(&[deckard_doc::Target::Group(g)], gc).unwrap();
    host.tick();
    assert!(host.lua_global::<bool>("GROUP_CHANGED"), "the software's group colour");
    assert!(!host.lua_global::<bool>("SOUND_CHANGED"));
    host.tick();
    assert!(!host.lua_global::<bool>("GROUP_CHANGED"), "flagged once");
    host.eval_lua("NI.DATA.SoundAccess.setSoundColor(App, NI.DATA.StateHelper.getFocusSound(App), 9, false)");
    host.tick();
    assert!(host.lua_global::<bool>("SOUND_CHANGED"), "the scripts' sound colour");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The touch strip shows where the playing scene's pattern for the focused group is.
#[test]
fn mk3_touch_strip_tracks_the_play_position() {
    let Some(p) = project() else { return };
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    let (g, scene) = (host.state.borrow().focus.group, host.state.borrow().focus.scene.expect("a focused scene"));
    let project = p.borrow().project().clone();
    let &(_, pi) = project.scenes[scene].patterns.iter().find(|(pg, _)| *pg == g).expect("the group plays in the scene");
    let len = project.groups[g].patterns[pi].length;
    let lit =
        |host: &Host| -> Vec<usize> { (1..=25).filter(|i| host.led(&format!("LED_TS_{i:02}")).is_some_and(|l| l.is_lit())).collect() };
    host.set_playhead(0, None);
    host.tick();
    let stopped = lit(&host);
    host.set_playhead(len / 2, Some(deckard_lua_host::state::Playing::Scene(scene)));
    host.tick();
    let half = lit(&host);
    host.set_playhead(len * 9 / 10, Some(deckard_lua_host::state::Playing::Scene(scene)));
    host.tick();
    let late = lit(&host);
    assert!(!half.is_empty() && half != stopped, "lit while playing: {half:?}");
    assert!(half.iter().max() < late.iter().max(), "moves on: {half:?} then {late:?}");
    assert!(late.iter().max().is_some_and(|&m| m >= 20), "{late:?}");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// FIXED VEL toggles Fixed Velocity: kept in the project, its LED lit, and every pad
/// plays at the song's fixed velocity (100 unless set).
#[test]
fn mk3_fixed_velocity_button_toggles() {
    let Some(p) = project() else { return };
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    let mode = |p: &deckard_doc::Shared| p.borrow().num(deckard_doc::Target::Song, "pad_velocity_mode");
    assert_eq!(host.fixed_velocity(), None);
    host.press("BUTTON_FIX_VELOCITY").unwrap();
    host.tick();
    assert_eq!(mode(&p), Some(1.0), "PAD_VELOCITY_FIXED in the project");
    let fixed = p.borrow().num(deckard_doc::Target::Song, "fixed_velocity").map_or(100, |v| v as i64);
    assert_eq!(host.fixed_velocity(), Some(fixed));
    assert!(host.led("LED_FIX_VELOCITY").is_some_and(|l| l.is_lit()));
    host.press("BUTTON_FIX_VELOCITY").unwrap();
    host.tick();
    assert_eq!(mode(&p), Some(0.0), "back to normal");
    assert_eq!(host.fixed_velocity(), None);
    assert!(!host.led("LED_FIX_VELOCITY").is_some_and(|l| l.is_lit()));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// Keyboard mode: buttons 5 and 6 move the pads' notes an octave down and up, 7 and 8 a
/// semitone (the group's root note).
#[test]
fn mk3_keyboard_screen_buttons_transpose_the_pads() {
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    host.press("BUTTON_KEYBOARD").unwrap();
    host.tick();
    let notes = |h: &Host| h.keyboard_pads().unwrap().1[0];
    let start = notes(&host);
    for (button, by) in
        [("BUTTON_SCREEN_6", 12), ("BUTTON_SCREEN_5", 0), ("BUTTON_SCREEN_5", -12), ("BUTTON_SCREEN_8", -11), ("BUTTON_SCREEN_7", -12)]
    {
        host.press(button).unwrap();
        host.tick();
        assert_eq!(notes(&host), start + by, "after {button}");
    }
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The scale the app's SCALE menu sets is the controller's: Keyboard mode plays it.
#[test]
fn scale_set_from_the_app_is_the_controllers() {
    use deckard_lua_host::api::scales::{in_scale, scale_names};
    assert_eq!(scale_names(0)[1], "Major");
    // C Major: C, D, E in; C#, Eb out; any octave.
    assert!(in_scale(0, 1, 60, 60) && in_scale(0, 1, 60, 62) && in_scale(0, 1, 60, 76));
    assert!(!in_scale(0, 1, 60, 61) && !in_scale(0, 1, 60, 63));
    assert!((0..128).all(|n| in_scale(0, 0, 60, n)), "Chromatic has every note");
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    host.set_scale(62, 0, 1);
    host.tick();
    assert_eq!(host.scale(), (62, 0, 1));
    assert_eq!(
        host.lua.load("return NI.DATA.StateHelper.getFocusSong(App):getNoteForPad(1)").eval::<i64>().unwrap(),
        64,
        "D Major: pad 2 plays E"
    );
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

#[test]
fn mk3_pads_flash_while_their_notes_play() {
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    let led = |host: &Host, pad: usize| host.led(&format!("LED_PAD_{pad}"));
    // A pad goes dark with the tick after its note ends.
    let settle = |host: &mut Host| host.tick();
    let tap = |host: &mut Host, pad: i64| {
        host.pad_event(pad, true, 1.0);
        host.pad_event(pad, false, 0.0);
        settle(host);
    };
    // A first tap focuses the pad's sound, which lights it too.
    tap(&mut host, 1);
    let idle = led(&host, 1);
    // Held from the hardware: lit up while down.
    host.pad_event(1, true, 1.0);
    host.tick();
    let held = led(&host, 1);
    assert_ne!(held, idle, "held pad");
    // Released: dark with the tick the app runs after the event.
    host.pad_event(1, false, 0.0);
    host.tick();
    assert_eq!(led(&host, 1), idle, "released pad");
    // Played by the sequencer: the pad of the sound playing.
    let group = host.state.borrow().focus.group;
    host.set_notes_played([(group, 0, 60)].into());
    host.tick();
    assert_eq!(led(&host, 1), held, "sequenced pad");
    host.set_notes_played(Default::default());
    settle(&mut host);
    assert_eq!(led(&host, 1), idle, "sequenced note ended");
    // In Keyboard mode, the pads whose note plays.
    host.press("BUTTON_KEYBOARD").unwrap();
    host.tick();
    let sound = host.state.borrow().focus.sound;
    let notes = host.keyboard_pads().unwrap().1;
    host.set_notes_played(Default::default());
    settle(&mut host);
    let dark = led(&host, 2);
    host.set_notes_played([(group, sound, notes[1])].into());
    host.tick();
    assert_ne!(led(&host, 2), dark, "keyboard pad");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

#[test]
fn mk3_ideas_view_screen_buttons_pick_and_add_scenes() {
    let Some(p) = project() else { return };
    let scenes = p.borrow().project().scenes.len();
    assert!(scenes >= 3, "scenes to pick");
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    host.press("BUTTON_ARRANGE").unwrap();
    host.tick();
    // The Ideas view: scene tabs over the buttons, a column of patterns per group.
    let shown = |host: &Host| -> Vec<String> {
        host.lua
            .load("local out = {} for _, b in ipairs(__idea_spaces[1].scenes) do if b.w:isActive() then out[#out + 1] = b.w:getText() end end return out")
            .eval::<Vec<String>>()
            .unwrap()
    };
    let names: Vec<String> = p.borrow().project().scenes.iter().map(|s| s.name.clone()).collect();
    assert_eq!(shown(&host)[..scenes.min(8)], names[..scenes.min(8)]);
    // Button 3 launches scene 3.
    host.press("BUTTON_SCREEN_3").unwrap();
    host.tick();
    assert_eq!(host.state.borrow().focus.scene, Some(2));
    assert_eq!(host.state.borrow_mut().launched_scene.take(), Some(2));
    // The button after the last scene ("+", in the last bank of eight) adds one and
    // focuses it.
    for _ in 0..scenes / 8 {
        host.press("BUTTON_RIGHT").unwrap();
        host.tick();
    }
    host.press(&format!("BUTTON_SCREEN_{}", scenes % 8 + 1)).unwrap();
    host.tick();
    assert_eq!(p.borrow().project().scenes.len(), scenes + 1);
    assert_eq!(host.state.borrow().focus.scene, Some(scenes));
    assert_eq!(shown(&host).len(), (scenes + 1) % 8 + 1);
    // Group progress bars: "bars/played" as the app plays, empty without a pattern.
    host.set_group_progress([(0, (4, 1.375))].into());
    host.tick();
    let progress = |host: &Host| -> Vec<String> {
        host.lua
            .load("local out = {} for _, g in ipairs(__idea_spaces[1].groups) do if g.w:isActive() then out[#out + 1] = g.progress:getAttribute('Progress') end end return out")
            .eval::<Vec<String>>()
            .unwrap()
    };
    let shown_progress = progress(&host);
    assert_eq!(shown_progress[0], "4/1.38");
    assert!(shown_progress[1..].iter().all(String::is_empty), "{shown_progress:?}");
    // SHIFT shows the edit buttons instead; then the buttons don't pick scenes.
    host.button("BUTTON_SHIFT", true).unwrap();
    host.tick();
    assert_eq!(host.screen_buttons()[0].label, "IDEAS");
    host.press("BUTTON_SCREEN_2").unwrap();
    host.tick();
    assert!(!host.state.borrow().ideas_view, "SHIFT + 2: Song view");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The focused pattern's notes of the focused sound, and the step grid's first step.
fn step_notes(host: &Host, doc: &deckard_doc::Shared) -> (Vec<deckard_model::Note>, i64) {
    let st = host.state.borrow();
    let (g, p, s) = (st.focus.group, st.focus.pattern.unwrap(), st.focus.sound);
    let d = doc.borrow();
    let pat = &d.project().groups[g].patterns[p];
    let first = -(-pat.start).div_euclid(st.step_grid) * st.step_grid;
    (pat.notes.iter().filter(|n| n.sound == s).copied().collect(), first)
}

/// STEP mode: an unlit pad adds a step (lit, selected), a lit one takes it away; each
/// is an undo step; the pads don't play sounds.
#[test]
fn mk3_step_mode_pads_add_and_remove_steps() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    host.press("BUTTON_STEP").unwrap();
    host.tick();
    assert_eq!(host.state.borrow().hw.page_stack.last(), Some(&host.constant("NI.HW.PAGE_STEP_STUDIO").unwrap()));
    assert!(!host.pads_play_sounds());
    let grid = host.state.borrow().step_grid;
    let (before, first) = step_notes(&host, &doc);
    // Pad 6 is the sixth step; pick one that's empty.
    let at = first + 5 * grid;
    assert!(!before.iter().any(|n| n.position == at), "step 6 is free");
    let lit = |host: &Host| host.led("LED_PAD_6").is_some_and(|l| l.is_lit());
    assert!(!lit(&host));

    host.pad(6, 0.8);
    host.tick();
    let (after, _) = step_notes(&host, &doc);
    let added: Vec<_> = after.iter().filter(|n| !before.contains(n)).collect();
    assert_eq!(added.len(), 1, "{added:?}");
    assert_eq!((added[0].position, added[0].length), (at, grid));
    assert!(lit(&host));
    let sel = host.state.borrow().selection.clone().unwrap().2;
    assert_eq!(sel, vec![*added[0]], "the new step is selected");

    host.pad(6, 0.8);
    host.tick();
    assert_eq!(step_notes(&host, &doc).0, before);
    assert!(!lit(&host));
    assert!(doc.borrow_mut().undo());
    assert_eq!(step_notes(&host, &doc).0, after, "undo brings the step back");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// STEP mode's knobs edit the selected step: 2 pitch, 3 velocity, 4 length, 1 position.
#[test]
fn mk3_step_mode_knobs_edit_the_selected_step() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    host.press("BUTTON_STEP").unwrap();
    host.tick();
    let grid = host.state.borrow().step_grid;
    let (before, first) = step_notes(&host, &doc);
    host.pad(6, 0.8);
    host.tick();
    let step = *step_notes(&host, &doc).0.iter().find(|n| !before.contains(n)).unwrap();
    for (knob, inc) in [("ENCODER_SCREEN_2", 0.06), ("ENCODER_SCREEN_3", -0.3), ("ENCODER_SCREEN_4", 0.06), ("ENCODER_SCREEN_1", 0.06)] {
        host.encoder(knob, inc).unwrap();
        host.tick();
    }
    let edited = *step_notes(&host, &doc).0.iter().find(|n| !before.contains(n)).unwrap();
    assert_eq!(edited.pitch, step.pitch + 1);
    assert!(edited.velocity < step.velocity);
    assert_eq!(edited.length, 2 * grid);
    // POSITION nudges by the Nudge Grid (half a step by default).
    assert_eq!(edited.position, first + 5 * grid + host.state.borrow().nudge_grid);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// EVENTS + SELECT: ALL and NONE, and a pad toggling its sound's notes.
#[test]
fn mk3_events_select_all_none_and_by_pad() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    let count = |host: &Host| host.state.borrow().selection.as_ref().map_or(0, |s| s.2.len());
    let (mine, _) = step_notes(&host, &doc);
    let all = {
        let st = host.state.borrow();
        doc.borrow().project().groups[st.focus.group].patterns[st.focus.pattern.unwrap()].notes.len()
    };
    host.button("BUTTON_EVENTS", true).unwrap();
    host.tick();
    host.press("BUTTON_SCREEN_2").unwrap();
    host.tick();
    assert!(!host.pads_play_sounds(), "pads select in SELECT mode");
    host.press("BUTTON_SCREEN_5").unwrap();
    host.tick();
    assert_eq!(count(&host), all);
    host.press("BUTTON_SCREEN_6").unwrap();
    host.tick();
    assert_eq!(count(&host), 0);
    host.pad(1, 1.0);
    host.tick();
    assert_eq!(count(&host), mine.len());
    host.pad(1, 1.0);
    host.tick();
    assert_eq!(count(&host), 0);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The Step page's displays draw the pattern editors: the overview and the steps.
#[test]
fn mk3_step_page_draws_the_pattern_editors() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    host.tick();
    host.press("BUTTON_STEP").unwrap();
    host.tick();
    let (page, _) = host.display().expect("display");
    fn editors(e: &deckard_nwl::draw::Element, out: &mut Vec<usize>) {
        if e.kind == "EventPatternEditor" && !e.hidden {
            out.push(e.shapes.len());
        }
        e.children.iter().for_each(|c| editors(c, out));
    }
    let mut found = Vec::new();
    editors(&page, &mut found);
    assert_eq!(found.len(), 2, "overview and detail");
    assert!(found.iter().all(|&n| n > 0), "{found:?}");
}

/// STEP mode: while the focused pattern plays, the pad of the step under the playhead
/// flashes.
#[test]
fn mk3_step_mode_playhead_pad_flashes() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    host.tick();
    host.press("BUTTON_STEP").unwrap();
    host.tick();
    let (g, p) = {
        let st = host.state.borrow();
        (st.focus.group, st.focus.pattern.unwrap())
    };
    let grid = host.state.borrow().step_grid;
    host.state.borrow_mut().transport.playing = true;
    host.set_playhead(3 * grid + 1, Some(deckard_lua_host::state::Playing::Pattern(g, p)));
    let step: i64 = host.lua.load("return App:getStateCache():playPositionStepEditor()").eval().unwrap();
    assert_eq!(step, 3);
    host.lua.load("StepHelper.updatePadLEDs()").exec().unwrap();
    let flash = host.lua.load("return LEDHelper.LS_FLASH").eval::<i64>().unwrap();
    assert_eq!(host.led("LED_PAD_4").and_then(|l| l.lut).map(|(_, s)| s), Some(flash));
}

/// STEP + SHIFT + pads: 14 a semitone up, 1 undo, 2 redo, 11 copy, 9 clear, 12 paste.
#[test]
fn mk3_step_mode_shift_pads_edit_undo_and_paste() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    host.press("BUTTON_STEP").unwrap();
    host.tick();
    let (before, _) = step_notes(&host, &doc);
    host.pad(6, 0.8);
    host.tick();
    let added = |host: &Host| {
        let (now, _) = step_notes(host, &doc);
        now.into_iter().filter(|n| !before.contains(n)).collect::<Vec<_>>()
    };
    let hit = |host: &mut Host, pad: i64| {
        host.pad(pad, 1.0);
        host.tick();
    };
    let step = added(&host)[0];
    host.button("BUTTON_SHIFT", true).unwrap();
    hit(&mut host, 14);
    let up = |n: &deckard_model::Note| n.pitch == step.pitch + 1 && n.position == step.position;
    assert!(added(&host).iter().any(up), "semitone up");
    hit(&mut host, 1);
    assert_eq!(added(&host), vec![step], "undo");
    hit(&mut host, 2);
    assert!(added(&host).iter().any(up), "redo");
    hit(&mut host, 11);
    hit(&mut host, 9);
    assert!(added(&host).is_empty(), "clear removes the selected step");
    hit(&mut host, 12);
    assert!(added(&host).iter().any(up), "paste puts it back");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The focused pattern's notes of sound `s`.
fn sound_notes(host: &Host, doc: &deckard_doc::Shared, s: usize) -> Vec<deckard_model::Note> {
    let st = host.state.borrow();
    let d = doc.borrow();
    d.project().groups[st.focus.group].patterns[st.focus.pattern.unwrap()].notes.iter().filter(|n| n.sound == s).copied().collect()
}

/// EVENTS: the 4-D encoder selects note by note; the knobs edit only the selection;
/// pushed right with nothing selected, every note nudges (round the pattern's end).
#[test]
fn mk3_events_wheel_selects_and_knobs_edit_the_selection() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    let before = sound_notes(&host, &doc, 0);
    host.button("BUTTON_EVENTS", true).unwrap();
    host.tick();
    // Nothing selected: VELOCITY changes nothing.
    host.encoder("ENCODER_SCREEN_3", -0.3).unwrap();
    host.tick();
    assert_eq!(sound_notes(&host, &doc, 0), before);
    host.wheel(1);
    host.tick();
    host.wheel(1);
    host.tick();
    let sel = host.state.borrow().selection.clone().unwrap().2;
    assert_eq!(sel, vec![before[1]], "the second note");
    host.encoder("ENCODER_SCREEN_3", -0.3).unwrap();
    host.tick();
    let after = sound_notes(&host, &doc, 0);
    assert!(after.iter().any(|n| n.position == before[1].position && n.velocity < before[1].velocity));
    assert_eq!(after.iter().filter(|n| before.contains(n)).count(), before.len() - 1, "only the selected note");

    // Deselect, then nudge everything right by the Nudge Grid; the last note wraps if
    // it must.
    host.lua.load("NI.DATA.EventPatternTools.deselectAllEvents(App)").exec().unwrap();
    host.press("BUTTON_WHEEL_RIGHT").unwrap();
    host.tick();
    let grid = host.state.borrow().nudge_grid;
    let (start, end) = {
        let st = host.state.borrow();
        let d = doc.borrow();
        let pat = &d.project().groups[st.focus.group].patterns[st.focus.pattern.unwrap()];
        (pat.start, pat.start + pat.length)
    };
    let mut want: Vec<i64> = after.iter().map(|n| start + (n.position + grid - start).rem_euclid(end - start)).collect();
    let mut got: Vec<i64> = sound_notes(&host, &doc, 0).iter().map(|n| n.position).collect();
    want.sort();
    got.sort();
    assert_eq!(got, want);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// EVENTS knobs 5–7: zoom in, scroll, and double the lanes.
#[test]
fn mk3_events_knobs_zoom_scroll_and_size_lanes() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    host.tick();
    host.button("BUTTON_EVENTS", true).unwrap();
    host.tick();
    let view = |host: &Host| -> (f64, i64) {
        host.lua
            .load("return NI.DATA.StateHelper.getPatternEditorZoomParameter(App, true):getValue(), NI.DATA.StateHelper.getPatternEditorOffsetParameter(App, true):getValue()")
            .eval()
            .unwrap()
    };
    let (fit, offset) = view(&host);
    assert_eq!(offset, 0);
    host.encoder("ENCODER_SCREEN_5", 0.2).unwrap();
    host.tick();
    let (zoom, _) = view(&host);
    assert!(zoom > fit, "{zoom} > {fit}");
    host.encoder("ENCODER_SCREEN_6", 0.5).unwrap();
    host.tick();
    assert!(view(&host).1 > 0, "scrolled");
    let lanes = "return NI.DATA.StateHelper.getFocusGroup(App):getPatternEditorVerticalZoomParameterHW():getValue()";
    assert_eq!(host.lua.load(lanes).eval::<i64>().unwrap(), 0);
    host.encoder("ENCODER_SCREEN_7", 0.1).unwrap();
    host.tick();
    assert_eq!(host.lua.load(lanes).eval::<i64>().unwrap(), 1);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// EVENTS + SELECT + ERASE + pad: that pad's sound loses its notes, the others keep
/// theirs.
#[test]
fn mk3_events_erase_pad_removes_its_sounds_notes() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    let (g, p) = {
        let st = host.state.borrow();
        (st.focus.group, st.focus.pattern.unwrap())
    };
    let extra = deckard_model::Note { expr: None, sound: 1, position: 0, length: 240, pitch: 60, velocity: 100, probability: 100 };
    doc.borrow_mut().edit_notes("test", g, p, &[], &[extra]).unwrap();
    let kicks = sound_notes(&host, &doc, 0);
    host.button("BUTTON_EVENTS", true).unwrap();
    host.tick();
    host.press("BUTTON_SCREEN_2").unwrap();
    host.tick();
    host.button("BUTTON_TRANSPORT_ERASE", true).unwrap();
    host.tick();
    host.pad(2, 1.0);
    host.tick();
    assert!(sound_notes(&host, &doc, 1).is_empty());
    assert_eq!(sound_notes(&host, &doc, 0), kicks);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// FIXED VEL on: new steps take the fixed velocity, not the pad's; it ranges 0–127.
#[test]
fn mk3_step_mode_uses_the_fixed_velocity() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    let set = |host: &Host, v: i64| {
        host.lua
            .load(format!("local p = NI.DATA.StateHelper.getFocusSong(App):getFixedVelocityParameter(); NI.DATA.ParameterAccess.setSizeTParameter(App, p, {v}); return p:getValue()"))
            .eval::<i64>()
            .unwrap()
    };
    assert_eq!(set(&host, 90), 90);
    assert_eq!(set(&host, 0), 0, "down to 0");
    assert_eq!(set(&host, 200), 127, "up to 127");
    set(&host, 90);
    host.press("BUTTON_FIX_VELOCITY").unwrap();
    host.tick();
    host.press("BUTTON_STEP").unwrap();
    host.tick();
    let (before, _) = step_notes(&host, &doc);
    host.pad(6, 0.3);
    host.tick();
    let (after, _) = step_notes(&host, &doc);
    let added: Vec<_> = after.iter().filter(|n| !before.contains(n)).collect();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].velocity, 90);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// A new project's fixed velocity is Maschine's 100.
#[test]
fn new_projects_fixed_velocity_is_100() {
    let doc = deckard_doc::Document::new_project().unwrap();
    assert_eq!(doc.num(deckard_doc::Target::Song, "fixed_velocity"), Some(100.0));
}

/// STEP: DOUBLE (button 3) shows beside KEYBOARD and doubles the pattern; KEYBOARD
/// (button 4) switches to the piano roll, its keys drawn beside it.
#[test]
fn mk3_step_page_double_and_keyboard() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    host.press("BUTTON_STEP").unwrap();
    host.tick();
    let labels: Vec<String> = host.screen_buttons().iter().map(|b| if b.visible { b.label.clone() } else { String::new() }).collect();
    assert_eq!(&labels[2..4], ["DOUBLE", "KEYBOARD"]);
    let len = |host: &Host| {
        let st = host.state.borrow();
        doc.borrow().project().groups[st.focus.group].patterns[st.focus.pattern.unwrap()].length
    };
    let before = len(&host);
    host.press("BUTTON_SCREEN_3").unwrap();
    host.tick();
    assert_eq!(len(&host), 2 * before);

    host.press("BUTTON_SCREEN_4").unwrap();
    host.tick();
    assert!(host.lua.load("return PadModeHelper.getKeyboardMode()").eval::<bool>().unwrap());
    let (page, _) = host.display().expect("display");
    fn keys(e: &deckard_nwl::draw::Element) -> Option<usize> {
        if e.kind == "PianorollKeyboardWidget" && !e.hidden {
            return Some(e.shapes.len());
        }
        e.children.iter().find_map(keys)
    }
    assert!(keys(&page).is_some_and(|n| n >= 24), "two octaves of keys");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// STEP: a double tap on VELOCITY sets the selected step's velocity to 100; on
/// PROBABILITY (the knobs' page 2) its probability to 100 %. One tap does nothing.
#[test]
fn mk3_double_tap_resets_velocity_and_probability() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    host.press("BUTTON_STEP").unwrap();
    host.tick();
    let (before, _) = step_notes(&host, &doc);
    host.pad(6, 0.3);
    host.tick();
    let step = |host: &Host| *step_notes(host, &doc).0.iter().find(|n| !before.contains(n)).unwrap();
    assert!(step(&host).velocity < 100);
    let tap = |host: &mut Host, cap: &str| {
        host.button(cap, true).unwrap();
        host.button(cap, false).unwrap();
        host.tick();
    };
    tap(&mut host, "BUTTON_CAP_3");
    assert!(step(&host).velocity < 100, "one tap");
    tap(&mut host, "BUTTON_CAP_3");
    assert_eq!(step(&host).velocity, 100);

    host.press("BUTTON_RIGHT").unwrap();
    host.tick();
    host.encoder("ENCODER_SCREEN_1", -0.3).unwrap();
    host.tick();
    assert!(step(&host).probability < 100);
    tap(&mut host, "BUTTON_CAP_1");
    tap(&mut host, "BUTTON_CAP_1");
    assert_eq!(step(&host).probability, 100);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// PLUG-IN, then the 4-D encoder: the browser lists the built-in instruments the app
/// offers ("(none)" first); turning picks one, pressing asks the app to load it in the
/// focused slot.
/// MIX: the strips drawn by the host; the 4-D encoder as the manual has it — up for the
/// groups, down for the focused group's sounds, turned for the focused channel's level,
/// pressed then turned for its pan; knobs 1–8 the level of the strip under them.
#[test]
fn mk3_mixer_page_and_its_wheel() {
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    host.press("BUTTON_MIX").unwrap();
    host.tick();
    let (page, _) = host.display().expect("the mixer shows");
    fn strips(e: &deckard_nwl::draw::Element, n: &mut usize) {
        if e.kind == "MixerBar" && !e.shapes.is_empty() {
            *n += 1;
        }
        e.children.iter().for_each(|c| strips(c, n));
    }
    let mut n = 0;
    strips(&page, &mut n);
    assert!(n >= 2, "strips drawn: {n}");
    let get = |host: &Host, lua: &str| host.lua.load(lua).eval::<f64>().unwrap();
    let sounds = |host: &Host| {
        host.lua.load("return NI.DATA.StateHelper.getFocusSong(App):getSoundsVisibleMixerParameter():getValue()").eval::<bool>().unwrap()
    };
    let level = "return NI.DATA.StateHelper.getFocusSound(App):getLevelParameter():getValue()";
    let pan = "return NI.DATA.StateHelper.getFocusSound(App):getPanParameter():getValue()";
    let group_level = "return NI.DATA.StateHelper.getFocusGroup(App):getLevelParameter():getValue()";
    host.press("BUTTON_WHEEL_DOWN").unwrap();
    host.tick();
    assert!(sounds(&host), "down: the sounds");
    let before = get(&host, level);
    host.wheel(4);
    host.tick();
    assert!(get(&host, level) > before, "turned: the sound's level up");
    host.press("BUTTON_WHEEL").unwrap();
    host.tick();
    host.wheel(-10);
    host.tick();
    assert!(get(&host, pan) < -0.1, "pressed, then turned: panned left");
    host.press("BUTTON_WHEEL_UP").unwrap();
    host.tick();
    assert!(!sounds(&host), "up: the groups");
    // (Still in pan, as the wheel was pressed: knob 1 pans group A.)
    let group_pan = "return NI.DATA.StateHelper.getFocusGroup(App):getPanParameter():getValue()";
    let before = (get(&host, group_level), get(&host, group_pan));
    host.encoder("ENCODER_SCREEN_1", 0.05).unwrap();
    host.tick();
    assert_eq!(get(&host, group_level), before.0);
    assert!(get(&host, group_pan) > before.1, "knob 1: group A's pan");
    // A double tap on knob 1: group A's pan back to the middle; in level, to 0 dB.
    let tap = |host: &mut Host| {
        for _ in 0..2 {
            host.button("BUTTON_CAP_1", true).unwrap();
            host.button("BUTTON_CAP_1", false).unwrap();
        }
        host.tick();
    };
    tap(&mut host);
    assert_eq!(get(&host, group_pan), 0.0, "double tap: centred");
    host.press("BUTTON_WHEEL").unwrap();
    host.tick();
    host.encoder("ENCODER_SCREEN_1", -0.2).unwrap();
    host.tick();
    tap(&mut host);
    assert!(deckard_model::level_db(get(&host, group_level)).abs() < 0.01, "double tap: 0 dB");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// MIX with a new project (one group, the sounds shown): the group row stays a row of
/// headers so the sounds' strips show under it, and their meters show the app's levels.
#[test]
fn mk3_mixer_shows_sounds_and_their_levels() {
    let Some(_) = scripts() else { return };
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    host.tick();
    host.press("BUTTON_MIX").unwrap();
    host.tick();
    // (The strips are made when the displays are drawn; the page's timer meters them.)
    host.display();
    host.set_levels(deckard_lua_host::state::Levels { sounds: vec![vec![(0.5, 0.25)]], ..Default::default() });
    for _ in 0..3 {
        host.tick();
    }
    let (page, _) = host.display().expect("the mixer shows");
    fn sound_strips<'a>(e: &'a deckard_nwl::draw::Element, out: &mut Vec<&'a deckard_nwl::draw::Element>, in_sounds: bool) {
        let in_sounds = in_sounds || e.name == "SoundVector";
        if e.kind == "MixerBar" && in_sounds {
            out.push(e);
        }
        e.children.iter().for_each(|c| sound_strips(c, out, in_sounds));
    }
    let mut strips = Vec::new();
    sound_strips(&page, &mut strips, false);
    assert!(strips.len() >= 8 && strips.iter().all(|s| !s.shapes.is_empty()), "the sounds' strips: {}", strips.len());
    // Sound 1's strip (the sound list's first) and its left meter's value.
    let meter = {
        use deckard_lua_host::state::Prop;
        let st = host.state.borrow();
        let w = &st.widgets;
        let list = w.all.iter().position(|x| x.kind == "Vector<MixerBar>" && x.name == "SoundVector").expect("the sound list");
        let bar = w.get(list).unwrap().children.iter().copied().find(|&c| w.get(c).is_some_and(|x| x.name == "widgets")).unwrap();
        let first = w.get(bar).unwrap().children[0];
        let Some(Prop::Widget(m)) = w.get(first).unwrap().props.get("LeftMeter").cloned() else { panic!("no meter set") };
        match w.get(m).unwrap().props.get("Value") {
            Some(Prop::Num(v)) => *v,
            Some(Prop::Int(v)) => *v as f64,
            v => panic!("meter value {v:?}"),
        }
    };
    assert!(meter > 0.0, "sound 1's left meter shows its level: {meter}");
    // Its peak holds a moment once the sound is quiet, then falls back.
    let peak = |host: &Host| -> f64 {
        use deckard_lua_host::state::Prop;
        let st = host.state.borrow();
        let w = &st.widgets;
        let m = w.all.iter().position(|x| {
            x.kind == "LeftMeter" && matches!(x.props.get("Value"), Some(Prop::Num(v)) if *v > 0.0)
                || x.kind == "LeftMeter" && matches!(x.props.get("Peak"), Some(Prop::Num(p)) if *p > 0.0)
        });
        m.and_then(|m| match w.get(m)?.props.get("Peak") {
            Some(Prop::Num(p)) => Some(*p),
            _ => None,
        })
        .unwrap_or(0.0)
    };
    let loud = peak(&host);
    assert!(loud > 0.0, "a peak");
    host.set_levels(Default::default());
    for _ in 0..5 {
        host.tick();
    }
    assert_eq!(peak(&host), loud, "held");
    for _ in 0..60 {
        host.tick();
    }
    assert!(peak(&host) < loud, "then falling: {} from {loud}", peak(&host));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// PLUG-IN: a knob turned goes to the app as turned, all of it (a turn of the MK3's
/// knob, ~1000 steps, is a whole turn: the parameter's whole range).
#[test]
fn mk3_plugin_knobs_turn_as_far_as_turned() {
    use deckard_lua_host::state::{ControlKnob, ControlRequest, ControlSlot};
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    {
        let mut st = host.state.borrow_mut();
        st.control.level = 2;
        st.control.slots = vec![ControlSlot { name: "Filter".into(), plugin: false, active: true, instrument: false }];
        st.control.focus_slot = Some(0);
        st.control.knobs = (0..8)
            .map(|i| {
                Some(ControlKnob { name: format!("K{i}"), section: String::new(), value: 0.5, text: "50 %".into(), ..Default::default() })
            })
            .collect();
    }
    host.tick();
    host.press("BUTTON_PLUGIN").unwrap();
    host.tick();
    host.state.borrow_mut().control_requests.clear();
    for _ in 0..100 {
        host.encoder("ENCODER_SCREEN_2", 0.01).unwrap();
        host.tick();
    }
    let turned: f64 = host
        .state
        .borrow()
        .control_requests
        .iter()
        .map(|r| match r {
            ControlRequest::Turn { pos: 1, inc, .. } => *inc,
            _ => 0.0,
        })
        .sum();
    assert!((turned - 1.0).abs() < 1e-9, "a whole turn: {turned} ({:?})", host.state.borrow().control_requests.first());
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

#[test]
fn mk3_plugin_browser_lists_and_loads() {
    use deckard_lua_host::state::{ControlRequest, ControlSlot, PluginEntry, PluginFormat};
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    let e = |name: &str, instrument: bool, format: PluginFormat| PluginEntry {
        name: name.into(),
        instrument,
        format,
        vendor: "Native Instruments".into(),
    };
    host.set_plugins(vec![
        e("Sampler", true, PluginFormat::Internal),
        e("Kick", true, PluginFormat::Internal),
        e("Compressor", false, PluginFormat::Internal),
        e("Massive X", true, PluginFormat::Vst3),
    ]);
    {
        let mut st = host.state.borrow_mut();
        st.control.level = 2;
        st.control.modules_visible = true;
        st.control.slots = vec![ControlSlot { name: "Sampler".into(), plugin: false, active: true, instrument: true }];
        st.control.focus_slot = Some(0);
    }
    host.tick();
    host.press("BUTTON_PLUGIN").unwrap();
    host.tick();
    host.press("BUTTON_WHEEL").unwrap();
    host.tick();
    assert_eq!(host.state.borrow().hw.page_stack.last(), Some(&host.constant("NI.HW.PAGE_MODULE").unwrap()));
    let names: Vec<String> = host
        .lua
        .load("local o = {} for i = 0, ModuleHelper.getResultListSize() - 1 do o[#o + 1] = ModuleHelper.moduleNameAt(i) end return o")
        .eval()
        .unwrap();
    assert_eq!(names, ["(none)", "Sampler", "Kick"]);
    let vendor: String = host.lua.load("return ModuleHelper.getVendorDisplayName(ModuleHelper.getCurrentVendor())").eval().unwrap();
    assert_eq!(vendor, "DK", "the built-in plug-ins are DECKARD's");
    host.state.borrow_mut().control_requests.clear();
    for _ in 0..2 {
        host.wheel(1);
        host.tick();
    }
    host.press("BUTTON_WHEEL").unwrap();
    host.tick();
    let requests = host.state.borrow().control_requests.clone();
    assert!(requests.contains(&ControlRequest::Load { entry: 1, replace: Some(0) }), "{requests:?}");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The plug-in browser's list follows the selection: turned far down, the selected
/// entry is still among the rows shown.
#[test]
fn mk3_plugin_browser_keeps_the_selection_in_view() {
    use deckard_lua_host::state::{PluginEntry, PluginFormat, Prop};
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    let effects = (1..=30).map(|i| PluginEntry {
        name: format!("Effect {i}"),
        instrument: false,
        format: PluginFormat::Internal,
        vendor: String::new(),
    });
    host.set_plugins(effects.collect());
    {
        let mut st = host.state.borrow_mut();
        st.control.level = 2;
        st.control.modules_visible = true;
    }
    host.tick();
    host.press("BUTTON_PLUGIN").unwrap();
    host.tick();
    host.press("BUTTON_WHEEL").unwrap();
    host.tick();
    // The + of an empty sound takes an instrument: TYPE to Effect.
    host.encoder("ENCODER_SCREEN_1", 0.3).unwrap();
    host.tick();
    for _ in 0..20 {
        host.wheel(1);
        host.tick();
    }
    let st = host.state.borrow();
    // The Module page's list: the one with items (other pages have a ResultList too).
    let bar = st
        .widgets
        .all
        .iter()
        .filter(|w| w.name == "ResultList")
        .filter_map(|w| match w.props.get("Bar") {
            Some(Prop::Widget(b)) => Some(*b),
            _ => None,
        })
        .find(|&b| !st.widgets.all[b].children.is_empty())
        .expect("the list's items");
    let shown: Vec<(String, bool)> = st.widgets.all[bar]
        .children
        .iter()
        .map(|&c| &st.widgets.all[c])
        .filter(|w| !matches!(w.props.get("Active"), Some(Prop::Bool(false))))
        .map(|w| {
            let text = match w.props.get("Text") {
                Some(Prop::Str(t)) => t.clone(),
                _ => String::new(),
            };
            (text, matches!(w.props.get("Selected"), Some(Prop::Bool(true))))
        })
        .collect();
    assert!(shown.contains(&("Effect 20".to_string(), true)), "{shown:?}");
}

/// The Song view on the controller (SHIFT + SONG on the Arranger page): the arranger
/// is drawn on both displays, LEFT/RIGHT focus sections, knob 1 moves the focused one,
/// knob 2 changes its scene, knob 4 its length, and a pad past the last section adds one.
#[test]
fn mk3_song_view_edits_sections() {
    let Some(p) = project() else { return };
    let order = |p: &deckard_doc::Shared| p.borrow().project().sections.iter().map(|s| s.scene).collect::<Vec<_>>();
    let before = order(&p);
    if before.len() < 3 || before[0] == before[1] {
        return;
    }
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    host.press("BUTTON_ARRANGE").unwrap();
    host.tick();
    host.button("BUTTON_SHIFT", true).unwrap();
    host.press("BUTTON_SCREEN_2").unwrap();
    host.button("BUTTON_SHIFT", false).unwrap();
    host.tick();
    assert!(!host.state.borrow().ideas_view);
    assert_eq!(host.state.borrow().focus.section, Some(0), "a section in focus");
    assert_eq!(host.screen_buttons()[0].label, "SECTION");
    // Both arrangers drawn: the overview and the detail.
    let (page, _) = host.display().unwrap();
    fn panels(e: &deckard_nwl::draw::Element, out: &mut Vec<usize>) {
        if e.kind == "ArrangerPanel" {
            out.push(e.shapes.len());
        }
        e.children.iter().for_each(|c| panels(c, out));
    }
    let mut drawn = Vec::new();
    panels(&page, &mut drawn);
    assert_eq!(drawn.len(), 2, "{drawn:?}");
    assert!(drawn.iter().all(|&n| n > 20), "{drawn:?}");

    host.press("BUTTON_RIGHT").unwrap();
    host.tick();
    assert_eq!(host.state.borrow().focus.section, Some(1));
    assert_eq!(host.state.borrow().focus.scene, before[1]);
    // Knob 1: section 2 moves back to the start.
    host.encoder("ENCODER_SCREEN_1", -0.2).unwrap();
    host.tick();
    let mut moved = before.clone();
    moved.swap(0, 1);
    assert_eq!(order(&p), moved);
    assert_eq!(host.state.borrow().focus.section, Some(0));
    // Knob 2: the next scene.
    let scene = order(&p)[0].unwrap();
    if scene + 1 < p.borrow().project().scenes.len() {
        host.encoder("ENCODER_SCREEN_2", 0.2).unwrap();
        host.tick();
        assert_eq!(order(&p)[0], Some(scene + 1));
    }
    // Knob 4: a bar longer, no longer following its scene.
    let (len, bar) = {
        let d = p.borrow();
        (d.project().sections[0].length, d.project().bar_length())
    };
    host.encoder("ENCODER_SCREEN_4", 0.2).unwrap();
    host.tick();
    let s = p.borrow().project().sections[0];
    assert_eq!((s.length, s.auto), ((len / bar + 1) * bar, false));
    // CREATE adds a section after the focused one, DUPLICATE a copy after it, DELETE
    // removes the focused one.
    let n = p.borrow().project().sections.len();
    for (button, after) in [("BUTTON_SCREEN_5", n + 1), ("BUTTON_SCREEN_4", n + 2), ("BUTTON_SCREEN_6", n + 1)] {
        host.press(button).unwrap();
        host.tick();
        assert_eq!(p.borrow().project().sections.len(), after, "{button}");
    }
    let before: Vec<_> = order(&p);
    // A pad after the last section adds one.
    let n = before.len();
    if n < 16 {
        host.pad(n as i64 + 1, 1.0);
        host.tick();
        host.pad(n as i64 + 1, 0.0);
        host.tick();
        assert_eq!(p.borrow().project().sections.len(), n + 1);
        assert_eq!(host.state.borrow().focus.section, Some(n));
    }
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The dim white group button after the last group creates a new group and focuses it
/// (MK3 manual, "Creating Groups").
#[test]
fn mk3_empty_group_button_creates_a_group() {
    let p = deckard_doc::Document::new_project().unwrap().shared();
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    assert_eq!(p.borrow().project().groups.len(), 1);
    host.press("BUTTON_GROUP_B").unwrap();
    host.tick();
    assert_eq!(p.borrow().project().groups.len(), 2);
    assert_eq!(p.borrow().project().groups[1].name, "Group B1");
    assert_eq!(host.state.borrow().focus.group, 1);
    // A button past the next one does nothing.
    host.press("BUTTON_GROUP_E").unwrap();
    host.tick();
    assert_eq!(p.borrow().project().groups.len(), 2);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The Sampling page's RECORD screen shows the app's recorder settings on its knobs:
/// turning one leaves the new value for the app (a choice a step at a time), RIGHT
/// shows the next page.
#[test]
fn mk3_record_screen_knobs_set_the_recorders_settings() {
    use deckard_lua_host::state::{KnobPage, Prop, SamplingKnob};
    let Some(mut host) = boot("M3", project()) else { return };
    let choice = |key: &str, name: &str, text: &str, value: f64, max: f64| SamplingKnob {
        key: key.into(),
        name: name.into(),
        text: text.into(),
        value,
        min: 0.0,
        max,
        step: 1.0,
        turn: 1.0 / 32.0,
    };
    host.state.borrow_mut().sampling_pages[0] = vec![
        KnobPage { name: "RECORDING".into(), knobs: vec![choice("Mode", "MODE", "Detect", 0.0, 3.0)] },
        KnobPage { name: "INPUT".into(), knobs: vec![choice("Source", "SOURCE", "Ext. Mono", 1.0, 2.0)] },
    ];
    host.tick();
    host.button("BUTTON_SAMPLE", true).unwrap();
    host.tick();
    let texts = |host: &Host| -> Vec<String> {
        let st = host.state.borrow();
        st.widgets
            .all
            .iter()
            .flat_map(|w| w.props.values())
            .filter_map(|p| match p {
                Prop::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    };
    let shown = texts(&host);
    assert!(shown.iter().any(|t| t == "MODE") && shown.iter().any(|t| t == "Detect"), "{shown:?}");
    // A little: nothing yet; past a 32nd of a turn: the next mode.
    host.encoder("ENCODER_SCREEN_1", 0.01).unwrap();
    assert!(host.state.borrow().sampling_edits.is_empty());
    host.encoder("ENCODER_SCREEN_1", 0.03).unwrap();
    assert_eq!(std::mem::take(&mut host.state.borrow_mut().sampling_edits), [("Mode".to_string(), 1.0)]);
    host.press("BUTTON_RIGHT").unwrap();
    host.tick();
    assert_eq!(host.state.borrow().sampling_page[0], 1);
    assert!(texts(&host).iter().any(|t| t == "Ext. Mono"), "the INPUT page");
    // Turned down past its first choice: stays there.
    host.encoder("ENCODER_SCREEN_1", -0.2).unwrap();
    assert_eq!(host.state.borrow().sampling_edits, [("Source".to_string(), 0.0)]);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The Sampling page's EDIT, SLICE and ZONE screens show the app's sample: the
/// waveform (both displays), the knobs the app gives each tab; their buttons ask the
/// app for edits (APPLY), slices (SPLIT, the focused slice moved by knob 3) and zones
/// (NEXT).
#[test]
fn mk3_sampling_edit_slice_and_zone_screens() {
    use deckard_lua_host::state::{KnobPage, SampleView, SamplingCall, SamplingKnob};
    let Some(mut host) = boot("M3", project()) else { return };
    let peaks: Vec<(f32, f32)> = (0..480).map(|i| ((i as f32 / 20.0).sin() * -0.8, (i as f32 / 20.0).sin().abs() * 0.8)).collect();
    let knob = |key: &str, name: &str, value: f64| SamplingKnob {
        key: key.into(),
        name: name.into(),
        text: format!("{value}"),
        value,
        min: 0.0,
        max: 48_000.0,
        step: 480.0,
        turn: 1.0 / 100.0,
    };
    {
        let mut st = host.state.borrow_mut();
        st.sample = Some(SampleView {
            name: "Kick 1".into(),
            path: "/tmp/kick.wav".into(),
            frames: 48_000,
            rate: 48_000,
            overview: peaks.clone(),
            detail: peaks,
            view: (0.0, 48_000.0),
            play: (0, 48_000),
            slices: vec![0, 12_000, 24_000, 36_000],
            zones: vec![((0, 127), (0, 127)), ((60, 72), (0, 127))],
            ..SampleView::default()
        });
        st.sampling_pages[1] = vec![KnobPage { name: "PLAY RANGE".into(), knobs: vec![knob("Edit.Start", "START", 0.0)] }];
        st.slice_knobs = vec![SamplingKnob {
            key: "Slice.Mode".into(),
            name: "MODE".into(),
            text: "Split".into(),
            max: 3.0,
            value: 1.0,
            ..knob("", "", 0.0)
        }];
    }
    host.tick();
    host.button("BUTTON_SAMPLE", true).unwrap();
    host.tick();
    let drawn = |host: &Host, kind: &str| -> Vec<usize> {
        fn find(e: &deckard_nwl::draw::Element, kind: &str, out: &mut Vec<usize>) {
            if e.kind == kind {
                out.push(e.shapes.len());
            }
            e.children.iter().for_each(|c| find(c, kind, out));
        }
        let (left, right) = host.display().unwrap();
        let mut out = Vec::new();
        find(&left, kind, &mut out);
        if let Some(right) = &right {
            find(right, kind, &mut out);
        }
        out
    };
    let texts = |host: &Host| -> Vec<String> {
        let st = host.state.borrow();
        st.widgets
            .all
            .iter()
            .flat_map(|w| w.props.values())
            .filter_map(|p| match p {
                deckard_lua_host::state::Prop::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    };
    // EDIT: the waveform on both displays, the knobs, APPLY (Truncate, the first).
    host.press("BUTTON_SCREEN_2").unwrap();
    host.tick();
    let waves = drawn(&host, "SampleOwnerWaveEditor");
    assert!(waves.iter().filter(|&&n| n > 400).count() >= 2, "both displays: {waves:?}");
    assert!(texts(&host).iter().any(|t| t == "Kick 1"), "the sample's name");
    host.encoder("ENCODER_SCREEN_1", 0.02).unwrap();
    assert_eq!(std::mem::take(&mut host.state.borrow_mut().sampling_edits), [("Edit.Start".to_string(), 960.0)]);
    host.press("BUTTON_SCREEN_8").unwrap();
    assert_eq!(std::mem::take(&mut host.state.borrow_mut().sampling_calls), [SamplingCall::Edit(1)]);
    // AUTO LOOP (the function after STRETCH) asks the app to loop the focused zone.
    host.lua.load("SamplingHelper.applyEditFunction(SamplingHelper.EditFuncAutoLoop)").exec().unwrap();
    assert_eq!(std::mem::take(&mut host.state.borrow_mut().sampling_calls), [SamplingCall::AutoLoop]);
    // STEMS (◀ twice from TRUNCATE: AUTOLOOP, STEMS): APPLY asks the app to separate;
    // while it does, the displays show its progress and APPLY cancels it.
    host.press("BUTTON_SCREEN_5").unwrap();
    host.press("BUTTON_SCREEN_5").unwrap();
    host.tick();
    assert!(texts(&host).iter().any(|t| t == "STEMS"), "STEMS picked");
    host.press("BUTTON_SCREEN_8").unwrap();
    assert_eq!(std::mem::take(&mut host.state.borrow_mut().sampling_calls), [SamplingCall::Stems]);
    host.state.borrow_mut().stems = Some(deckard_lua_host::state::StemsRun { focused: true, progress: 0.4 });
    host.tick();
    assert!(texts(&host).iter().any(|t| t == "Stem Separation in Progress"));
    let bar = host.state.borrow().widgets.all.iter().find(|w| w.kind == "ProgressBar").and_then(|w| match w.props.get("Value") {
        Some(deckard_lua_host::state::Prop::Num(v)) => Some(*v),
        Some(deckard_lua_host::state::Prop::Int(v)) => Some(*v as f64),
        _ => None,
    });
    assert_eq!(bar.map(|v| v.round()), Some(400.0), "the progress bar at 40 %");
    host.press("BUTTON_SCREEN_8").unwrap();
    assert_eq!(std::mem::take(&mut host.state.borrow_mut().sampling_calls), [SamplingCall::CancelStems]);
    host.state.borrow_mut().stems = None;
    host.press("BUTTON_SCREEN_6").unwrap();
    host.press("BUTTON_SCREEN_6").unwrap();
    host.tick();
    // Knob 5 zooms in.
    host.encoder("ENCODER_SCREEN_5", 0.1).unwrap();
    let calls = std::mem::take(&mut host.state.borrow_mut().sampling_calls);
    assert!(matches!(calls.last(), Some(SamplingCall::View(a, b)) if b - a < 48_000.0), "{calls:?}");
    // SLICE: slices drawn; page 2 knob 1 focuses one, SPLIT splits it, knob 3 moves it.
    host.press("BUTTON_SCREEN_3").unwrap();
    host.tick();
    assert!(drawn(&host, "SliceWaveEditor").iter().any(|&n| n > 400));
    host.press("BUTTON_RIGHT").unwrap();
    host.tick();
    host.encoder("ENCODER_SCREEN_1", 0.5).unwrap();
    host.tick();
    let focus = host.state.borrow().sample.as_ref().unwrap().focus_slice;
    assert!(focus.is_some(), "a slice in focus");
    host.press("BUTTON_SCREEN_5").unwrap();
    assert_eq!(std::mem::take(&mut host.state.borrow_mut().sampling_calls), [SamplingCall::SplitSlice(focus.unwrap())]);
    // ZONE: the zone map; NEXT focuses the second zone.
    host.press("BUTTON_SCREEN_4").unwrap();
    host.tick();
    assert!(drawn(&host, "ZoneMapEditor").iter().any(|&n| n > 3));
    host.press("BUTTON_SCREEN_8").unwrap();
    assert_eq!(std::mem::take(&mut host.state.borrow_mut().sampling_calls), [SamplingCall::FocusZone(1)]);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// An Audio plug-in's loop on the MK3 (docs/audio-plugin.md): the pattern editors draw
/// its waveform in its lane; in STEP mode its steps are lit while it's on in the pattern,
/// and a step pad switches it off (and on), as one undo step.
#[test]
fn mk3_audio_loops_show_and_switch_on_the_step_page() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    let (g, pi) = {
        let st = host.state.borrow();
        (st.focus.group, st.focus.pattern.unwrap())
    };
    let s = 3;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let file = dir.join("Loop[120].wav");
    std::fs::write(&file, b"not read here").unwrap();
    {
        let mut d = doc.borrow_mut();
        d.audio_only(g, s).unwrap();
        d.set_takes(g, s, &[deckard_doc::TakeSpec { file, frames: 88_200, rate: 44_100, beats: 4.0 }], "Load Loop").unwrap();
        d.set_audio_in(g, s, &[pi], Some(0), "Load Loop").unwrap();
    }
    host.state.borrow_mut().focus.sound = s;
    host.tick();
    let editors = |host: &Host| -> Vec<usize> {
        fn find(e: &deckard_nwl::draw::Element, out: &mut Vec<usize>) {
            if e.kind == "EventPatternEditor" && !e.hidden {
                out.push(e.shapes.len());
            }
            e.children.iter().for_each(|c| find(c, out));
        }
        let (page, _) = host.display().expect("display");
        let mut out = Vec::new();
        find(&page, &mut out);
        out
    };
    host.press("BUTTON_STEP").unwrap();
    host.tick();
    let plain = editors(&host);
    // The app's peaks for it: the loop drawn in its lanes.
    let peaks: Vec<(f32, f32)> = (0..1024).map(|i| ((i as f32 / 30.0).sin() * -0.7, (i as f32 / 30.0).sin().abs() * 0.7)).collect();
    host.state.borrow_mut().loop_peaks.insert((g, s, 0), std::sync::Arc::new(peaks));
    host.tick();
    let with = editors(&host);
    assert!(with.iter().zip(&plain).all(|(a, b)| a > b), "the loop drawn: {plain:?} -> {with:?}");
    // Its steps lit; pad 1 switches it off.
    let lit = |host: &Host| host.led("LED_PAD_1").is_some_and(|l| l.is_lit());
    assert!(lit(&host), "on in the pattern: its steps lit");
    let on = |doc: &deckard_doc::Shared| doc.borrow().project().groups[g].patterns[pi].audio.iter().any(|c| c.sound == s);
    host.pad(1, 0.8);
    host.tick();
    assert!(!on(&doc), "switched off");
    assert!(!lit(&host));
    assert!(doc.borrow_mut().undo());
    assert!(on(&doc), "undo: on again");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// BROWSE on the MK3: NI's browse page runs on DECKARD's library as the app gives it
/// (`BrowserView`), with no browser calls left stubbed and no errors; turning the 4-D
/// encoder moves the result focus and pressing it loads.
#[test]
fn mk3_browse_lists_the_library_and_loads() {
    use deckard_lua_host::state::{BrowseCall, BrowserView};
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    host.set_browser(BrowserView {
        kind: 5,
        products: vec!["Deep Matter".into(), "Lilac Glare".into()],
        attributes: [(vec!["Bass".into(), "Drums".into()], None), (Vec::new(), None), (vec!["Clean".into()], None)],
        results: vec![("Kick[120] Lagoon".into(), false), ("Snare[90] Dust".into(), true), ("Bass[94] Amber".into(), false)],
        ..BrowserView::default()
    });
    host.press("BUTTON_BROWSE").unwrap();
    for _ in 0..3 {
        host.tick();
    }
    let browser_stubs: Vec<String> = host
        .report()
        .into_iter()
        .map(|(k, ..)| k)
        .filter(|k| {
            ["DatabaseFrontend", "BrowserModel", "BrowserController", "NI.DB3", "SamplePrehearAccess", "QuickBrowse"]
                .iter()
                .any(|s| k.contains(s))
        })
        .collect();
    assert!(browser_stubs.is_empty(), "{browser_stubs:?}");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
    host.state.borrow_mut().browse_calls.clear();
    host.wheel(1);
    host.tick();
    assert!(host.state.borrow().browse_calls.contains(&BrowseCall::Focus(1)), "{:?}", host.state.borrow().browse_calls);
    host.press("BUTTON_WHEEL").unwrap();
    host.tick();
    assert!(host.state.borrow().browse_calls.contains(&BrowseCall::Load), "{:?}", host.state.borrow().browse_calls);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// NOTE REPEAT on the MK3: NI's page shows Note Repeat / Arp as the app gives it, with
/// nothing stubbed; turning GATE sends the new value to the app; LOCK too.
#[test]
fn mk3_note_repeat_page_edits_the_arp() {
    use deckard_lua_host::state::{ArpView, SamplingKnob};
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    let knob = |key: &str, name: &str, text: &str, value: f64, max: f64| SamplingKnob {
        key: key.into(),
        name: name.into(),
        text: text.into(),
        value,
        min: 0.0,
        max,
        step: if max > 1.0 { 1.0 } else { 0.01 },
        turn: 0.05,
    };
    host.state.borrow_mut().arp = ArpView {
        knobs: vec![
            knob("arp/rate", "RATE", "1/16", 4.0, 7.0),
            knob("arp/rate_unit", "UNIT", "Normal", 0.0, 2.0),
            knob("arp/gate", "GATE", "100%", 0.5, 1.0),
            knob("arp/type", "TYPE", "Up", 0.0, 4.0),
            knob("arp/hold", "HOLD", "Off", 0.0, 1.0),
            knob("arp/preset", "PRESET", "1/16", 2.0, 3.0),
        ],
        lists: vec![("arp/preset".into(), vec!["1/4".into(), "1/8".into(), "1/16".into(), "1/32".into()])],
        active: true,
        locked: false,
    };
    host.button("BUTTON_NOTE_REPEAT", true).unwrap();
    for _ in 0..3 {
        host.tick();
    }
    let stubs: Vec<String> =
        host.report().into_iter().map(|(k, ..)| k).filter(|k| k.contains("Arpeggiator") || k.contains("ArpRepeat")).collect();
    assert!(stubs.is_empty(), "{stubs:?}");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
    host.state.borrow_mut().arp_edits.clear();
    host.encoder("ENCODER_SCREEN_8", 0.5).unwrap();
    host.tick();
    let edits = host.state.borrow().arp_edits.clone();
    assert!(edits.iter().any(|(k, v)| k == "arp/gate" && *v > 0.5), "{edits:?}");
    // HOLD reads as a switch (off), so display button 3 turns it on.
    host.state.borrow_mut().arp_edits.clear();
    host.press("BUTTON_SCREEN_3").unwrap();
    host.tick();
    let edits = host.state.borrow().arp_edits.clone();
    assert!(edits.iter().any(|(k, v)| k == "arp/hold" && *v == 1.0), "{edits:?}");
    host.eval_lua("NI.DATA.SongAccess.setArpRepeatLockState(App, true)");
    assert!(host.state.borrow().arp.locked);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// NOTE REPEAT's choice lists (shown while a knob is touched) hold every choice: NI's
/// list is `getMax() + 1` long, so RATE's 1/128 and UNIT's Dotted show.
#[test]
fn mk3_note_repeat_lists_show_every_choice() {
    use deckard_lua_host::state::{ArpView, SamplingKnob};
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    let list = |key: &str, items: &[&str]| (key.to_string(), items.iter().map(|x| x.to_string()).collect::<Vec<_>>());
    let knob = |key: &str, max: f64| SamplingKnob {
        key: key.into(),
        name: key.into(),
        text: String::new(),
        value: 0.0,
        min: 0.0,
        max,
        step: 1.0,
        turn: 0.08,
    };
    host.state.borrow_mut().arp = ArpView {
        knobs: vec![knob("arp/rate", 7.0), knob("arp/rate_unit", 2.0)],
        lists: vec![
            list("arp/rate", &["1 Bar", "1/2", "1/4", "1/8", "1/16", "1/32", "1/64", "1/128"]),
            list("arp/rate_unit", &["Normal", "Triplet", "Dotted"]),
        ],
        active: true,
        locked: false,
    };
    let max =
        |host: &deckard_lua_host::Host, f: &str| -> i64 { host.lua.load(format!("return NI.DATA.{f}(App):getMax()")).eval().unwrap() };
    assert_eq!(max(&host, "getArpeggiatorRateParameter") + 1, 8);
    assert_eq!(max(&host, "getArpeggiatorRateUnitParameter") + 1, 3);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// AUTO (docs/superpowers/plans/2026-10-22-phase2c-modulation.md): held, auto-write is
/// on, and off when let go; SHIFT + AUTO pins it on. ERASE + a knob in Control mode asks
/// for its modulation to go; SHIFT + pad 10 (Clear Auto) clears the focused sound's in
/// the focused pattern.
#[test]
fn mk3_auto_writes_and_erase_clears_modulation() {
    use deckard_lua_host::state::ControlRequest;
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    let set = |host: &mut Host| host.state.borrow_mut().auto_write_set.take();
    host.button("BUTTON_AUTO_WRITE", true).unwrap();
    host.tick();
    assert_eq!(set(&mut host), Some(true));
    host.state.borrow_mut().auto_write = true;
    host.button("BUTTON_AUTO_WRITE", false).unwrap();
    host.tick();
    assert_eq!(set(&mut host), Some(false));
    host.state.borrow_mut().auto_write = false;
    // Pinned with SHIFT: on after AUTO is let go.
    host.button("BUTTON_SHIFT", true).unwrap();
    host.button("BUTTON_AUTO_WRITE", true).unwrap();
    host.tick();
    assert_eq!(set(&mut host), Some(true));
    host.state.borrow_mut().auto_write = true;
    host.button("BUTTON_AUTO_WRITE", false).unwrap();
    host.button("BUTTON_SHIFT", false).unwrap();
    host.tick();
    assert_ne!(set(&mut host), Some(false), "pinned");

    // ERASE + knob 3 in Control mode.
    let control = host.constant("NI.HW.ENC_MODE_CONTROL").unwrap();
    host.state.borrow_mut().hw.encoder_mode = control;
    host.state.borrow_mut().control_requests.clear();
    host.button("BUTTON_ERASE", true).unwrap();
    host.encoder("ENCODER_SCREEN_3", 0.05).unwrap();
    host.button("BUTTON_ERASE", false).unwrap();
    assert!(host.state.borrow().control_requests.contains(&ControlRequest::Erase { pos: 2 }));

    // SHIFT + pad 10: modulation written on a sound with one of Maschine's effects.
    let found = {
        let d = doc.borrow();
        d.project().groups.iter().enumerate().find_map(|(g, gr)| {
            (!gr.patterns.is_empty()).then_some(())?;
            gr.sounds.iter().enumerate().find_map(|(s, snd)| {
                let i = snd.slots.iter().position(|x| x.plugin.is_none() && !x.params.is_empty())?;
                Some((g, 0, s, deckard_model::ModTarget { slot: Some(i), field: snd.slots[i].params[0].0.clone() }))
            })
        })
    };
    let (g, p, s, target) = found.expect("a sound with an effect");
    doc.borrow_mut().set_modulation(deckard_doc::Chain::Sound(g, s), &target, p, (0, 0), &[(0, 0.25)]).unwrap();
    assert!(doc.borrow().project().groups[g].patterns[p].mods.iter().any(|m| m.sound == Some(s)));
    {
        let mut st = host.state.borrow_mut();
        st.focus.group = g;
        st.focus.sound = s;
        st.focus.pattern = Some(p);
    }
    host.eval_lua("NI.DATA.EventPatternTools.removeModulationEvents(App)");
    let d = doc.borrow();
    assert!(!d.project().groups[g].patterns[p].mods.iter().any(|m| m.sound == Some(s)), "cleared");
    assert_eq!(d.undo_label(), Some("Clear Modulation"));
}

/// VARIATION on the MK3 (Maschine 3: `MaschineHelper.showGrooveVariationSection`): the
/// focused sound's Groove ▸ Variation page in Control mode.
#[test]
fn mk3_variation_shows_the_grooves_variation() {
    use deckard_lua_host::state::ControlRequest;
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    host.state.borrow_mut().control_requests.clear();
    host.button("BUTTON_VARIATION", true).unwrap();
    host.button("BUTTON_VARIATION", false).unwrap();
    host.tick();
    assert!(host.state.borrow().control_requests.contains(&ControlRequest::GroovePage(1)), "{:?}", host.state.borrow().control_requests);
    let stubs: Vec<String> = host.report().into_iter().map(|(k, ..)| k).filter(|k| k.contains("GroovePageGroup")).collect();
    assert!(stubs.is_empty(), "{stubs:?}");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// SHIFT + pads 1 / 2 undo and redo a take (`…Marker`); pads 3 / 4 a single edit of it.
#[test]
fn mk3_shift_pads_undo_a_take_or_one_step() {
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    doc.borrow_mut().add_pattern(0, 0, "P").unwrap();
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    let note =
        |k: i64| deckard_model::Note { expr: None, sound: 0, position: k * 960, length: 240, pitch: 60, velocity: 90, probability: 100 };
    {
        let mut d = doc.borrow_mut();
        d.begin_take("Record");
        // Different labels: two steps, however close in time.
        d.edit_notes("Record A", 0, 0, &[], &[note(0)]).unwrap();
        d.edit_notes("Record B", 0, 0, &[], &[note(1)]).unwrap();
        d.end_take();
    }
    let notes = || doc.borrow().project().groups[0].patterns[0].notes.len();
    assert_eq!(notes(), 2);
    let hit = |host: &mut Host, pad: i64| {
        host.pad(pad, 1.0);
        host.tick();
    };
    host.button("BUTTON_SHIFT", true).unwrap();
    hit(&mut host, 3);
    assert_eq!(notes(), 1, "step undo: one edit");
    hit(&mut host, 4);
    assert_eq!(notes(), 2, "step redo");
    hit(&mut host, 1);
    assert_eq!(notes(), 0, "undo: the whole take");
    hit(&mut host, 2);
    assert_eq!(notes(), 2, "redo: the whole take");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// SHIFT + pads 7 / 8 nudge by the Nudge Grid (the app's, shared), not the Step Grid.
#[test]
fn mk3_shift_pads_nudge_by_the_nudge_grid() {
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    doc.borrow_mut().add_pattern(0, 0, "P").unwrap();
    let note = deckard_model::Note { expr: None, sound: 0, position: 960, length: 240, pitch: 60, velocity: 90, probability: 100 };
    doc.borrow_mut().add_note(0, 0, note).unwrap();
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    {
        let mut st = host.state.borrow_mut();
        st.step_grid = 240;
        st.nudge_grid = 60;
    }
    host.button("BUTTON_SHIFT", true).unwrap();
    host.pad(8, 1.0);
    host.tick();
    let pos = doc.borrow().project().groups[0].patterns[0].notes[0].position;
    assert_eq!(pos, 1020, "a sixteenth's quarter right");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// SHIFT + FOLLOW (Grid) ▸ STEP: the pads pick the Step Grid (Maschine's 15 values,
/// pad 16 OFF), knob 4 (NUDGE) the Nudge Grid (Step … Step/16 of it).
#[test]
fn mk3_grid_page_sets_the_step_and_nudge_grids() {
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    host.tick();
    host.button("BUTTON_SHIFT", true).unwrap();
    host.button("BUTTON_FOLLOW", true).unwrap();
    host.tick();
    host.press("BUTTON_SCREEN_4").unwrap();
    host.tick();
    // The pads hold the values by columns (the scripts' transposed grid): 1 Bar, 1/2,
    // 1/4, 1/8 up the first, 1/16 … 1/128 the second, the triplets the last two.
    host.pad(9, 1.0);
    host.tick();
    assert_eq!(host.state.borrow().step_grid, 960, "pad 9: 1/4");
    assert!(!host.state.borrow().editor.grid_off);
    host.pad(3, 1.0);
    host.tick();
    assert_eq!(host.state.borrow().step_grid, 2560, "pad 3: 1 Bar T");
    host.pad(2, 1.0);
    host.tick();
    assert_eq!(host.state.borrow().step_grid, 240, "pad 2: 1/16");
    assert_eq!(host.state.borrow().nudge_grid, 120, "Step/2 by default");
    for _ in 0..3 {
        host.encoder("ENCODER_SCREEN_4", 0.1).unwrap();
        host.tick();
    }
    let n = host.state.borrow().nudge_grid;
    assert!(n < 120 && n >= 15, "knob 4 turned right: finer ({n})");
    // The app takes the controller's grids over, then gives its own again.
    let turned = host.state.borrow().nudge_div;
    let (mut grid, mut div) = (480, 1);
    host.state.borrow_mut().sync_grid(&mut grid, &mut div);
    assert_eq!((grid, div), (240, turned));
    assert_eq!(240 >> div, n);
    let (mut g2, mut d2) = (960, 0);
    host.state.borrow_mut().sync_grid(&mut g2, &mut d2);
    assert_eq!((g2, d2), (960, 0), "no change on the controller: the app's stand");
    assert_eq!((host.state.borrow().step_grid, host.state.borrow().nudge_grid), (960, 960));
    host.pad(16, 1.0);
    host.tick();
    assert!(host.state.borrow().editor.grid_off, "pad 16: OFF");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// SHIFT + PERFORM puts a Perform FX on the focused group and shows it; PERFORM sets the
/// strip to play it: touching engages it where touched, letting go releases it.
#[test]
fn mk3_perform_loads_and_plays_the_perform_fx() {
    use deckard_lua_host::state::ControlRequest;
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    let can = |host: &Host| {
        host.lua.load("return NI.DATA.StateHelper.canAddPerformanceFX(NI.DATA.StateHelper.getFocusGroup(App))").eval::<bool>().unwrap()
    };
    assert!(can(&host));
    host.state.borrow_mut().control_requests.clear();
    host.button("BUTTON_SHIFT", true).unwrap();
    host.press("BUTTON_PERFORM").unwrap();
    host.button("BUTTON_SHIFT", false).unwrap();
    host.tick();
    assert_eq!(doc.borrow().project().groups[0].slots[0].module, "PerformanceFX");
    assert!(host.state.borrow().control_requests.contains(&ControlRequest::ShowPerformFx(0)), "{:?}", host.state.borrow().control_requests);
    assert!(!can(&host), "one per group");
    let has = host.lua.load("return NI.DATA.StateHelper.getFocusGroup(App):getPerformanceFX() ~= nil").eval::<bool>().unwrap();
    assert!(has);
    // PERFORM on: the strip plays it.
    if host.lua.load("return NHLController:getContext():getTouchstripModeParameter():getValue()").eval::<i64>().unwrap()
        != host.constant("NI.HW.TS_MODE_PERFORM").unwrap()
    {
        host.press("BUTTON_PERFORM").unwrap();
        host.tick();
    }
    host.state.borrow_mut().control_requests.clear();
    host.touch(0, 0, 0.25);
    host.touch(0, 1, 0.5);
    host.touch(0, 2, 0.5);
    let reqs = host.state.borrow().control_requests.clone();
    let strip: Vec<_> = reqs.iter().filter(|r| matches!(r, ControlRequest::PerformStrip { .. })).cloned().collect();
    assert_eq!(
        strip,
        [
            ControlRequest::PerformStrip { group: 0, touch: Some(true), pos: 0.25 },
            ControlRequest::PerformStrip { group: 0, touch: None, pos: 0.5 },
            ControlRequest::PerformStrip { group: 0, touch: Some(false), pos: 0.5 },
        ]
    );
    // The LEDs read the module's strip parameters.
    host.tick();
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// LOCK and Extended Lock (SHIFT + LOCK): the scripts' snapshot calls reach the app as
/// requests; a pad holding a snapshot recalls it (natively); the manager's morph
/// settings read and write the project.
#[test]
fn mk3_lock_and_extended_lock() {
    use deckard_lua_host::state::{ControlRequest, LockOp};
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    doc.borrow_mut().take_snapshot(0).unwrap();
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    let reqs = |host: &Host| host.state.borrow().control_requests.clone();
    host.state.borrow_mut().control_requests.clear();
    host.press("BUTTON_LOCK").unwrap();
    host.tick();
    assert!(reqs(&host).contains(&ControlRequest::Lock(LockOp::Button)), "{:?}", reqs(&host));
    // Extended Lock.
    host.button("BUTTON_SHIFT", true).unwrap();
    host.press("BUTTON_LOCK").unwrap();
    host.button("BUTTON_SHIFT", false).unwrap();
    host.tick();
    host.state.borrow_mut().control_requests.clear();
    host.pad(1, 1.0);
    host.tick();
    host.pad(3, 1.0);
    host.tick();
    let r = reqs(&host);
    assert!(r.contains(&ControlRequest::Lock(LockOp::Pad(0))), "pad 1 holds one: recalled: {r:?}");
    assert!(r.contains(&ControlRequest::Lock(LockOp::Pad(2))), "pad 3: taken: {r:?}");
    assert_eq!(r.iter().filter(|x| matches!(x, ControlRequest::Lock(LockOp::Pad(0)))).count(), 1, "once");
    host.state.borrow_mut().control_requests.clear();
    host.press("BUTTON_SCREEN_5").unwrap();
    host.tick();
    assert!(reqs(&host).contains(&ControlRequest::Lock(LockOp::Update)), "{:?}", reqs(&host));
    // The morph settings.
    let morph = |host: &Host| {
        host.lua
            .load("return NI.DATA.StateHelper.getFocusSong(App):getParameterSnapshotsManager():getSnapshotMorphParameter():getValue()")
            .eval::<bool>()
            .unwrap()
    };
    assert!(!morph(&host));
    host.lua
        .load("local m = NI.DATA.StateHelper.getFocusSong(App):getParameterSnapshotsManager(); NI.DATA.ParameterAccess.setBoolParameter(App, m:getSnapshotMorphParameter(), true)")
        .exec()
        .unwrap();
    assert!(doc.borrow().project().lock.morph);
    assert!(morph(&host));
    let mode = host
        .lua
        .load("return NI.DATA.StateHelper.getFocusSong(App):getParameterSnapshotsManager():getSnapshotMorphModeParameter():getAsString(1)")
        .eval::<String>()
        .unwrap();
    assert_eq!(mode, "Target");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// PLUG-IN, SHIFT + buttons 5 / 6 (<< >>): the focused effect moves left or right (past
/// another effect; not before the instrument).
#[test]
fn mk3_plugin_page_moves_effects() {
    use deckard_lua_host::state::{ControlRequest, ControlSlot};
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    let slot = |name: &str, instrument: bool| ControlSlot { name: name.into(), plugin: false, active: true, instrument };
    {
        let mut st = host.state.borrow_mut();
        st.control.level = 2;
        st.control.slots = vec![slot("Sampler", true), slot("Filter", false), slot("Compressor", false)];
        st.control.focus_slot = Some(1);
    }
    host.tick();
    host.press("BUTTON_PLUGIN").unwrap();
    host.tick();
    host.state.borrow_mut().control_requests.clear();
    host.button("BUTTON_SHIFT", true).unwrap();
    host.tick();
    host.press("BUTTON_SCREEN_6").unwrap();
    host.tick();
    let reqs = host.state.borrow().control_requests.clone();
    assert!(reqs.contains(&ControlRequest::Move(1, 2)), "right: {reqs:?}");
    host.state.borrow_mut().control.focus_slot = Some(2);
    host.state.borrow_mut().control_requests.clear();
    host.tick();
    host.press("BUTTON_SCREEN_5").unwrap();
    host.tick();
    let reqs = host.state.borrow().control_requests.clone();
    assert!(reqs.contains(&ControlRequest::Move(2, 1)), "left: {reqs:?}");
    // The Filter (slot 1) can't go before the Sampler.
    host.state.borrow_mut().control.focus_slot = Some(1);
    host.state.borrow_mut().control_requests.clear();
    host.tick();
    host.press("BUTTON_SCREEN_5").unwrap();
    host.tick();
    assert!(!host.state.borrow().control_requests.iter().any(|r| matches!(r, ControlRequest::Move(..))));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// PERFORM mode's strip LEDs (NI's `updateLEDsPerformMode`): three lit around the Perform
/// FX's strip position, the middle white while touched; still lit, in the group's
/// colour, once let go; two at either end; each group shows its own.
#[test]
fn mk3_perform_strip_leds_show_the_position() {
    use deckard_doc::{Chain, Target};
    use mxprj::decode::Value;
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    doc.borrow_mut().add_perform_fx(0).unwrap();
    doc.borrow_mut().add_group().unwrap();
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    if host.lua.load("return NHLController:getContext():getTouchstripModeParameter():getValue()").eval::<i64>().unwrap()
        != host.constant("NI.HW.TS_MODE_PERFORM").unwrap()
    {
        host.press("BUTTON_PERFORM").unwrap();
    }
    let set = |field: &str, v: f64| {
        doc.borrow_mut().set("t", Target::ChainModule(Chain::Group(0), 0), field, Value::Float(v)).unwrap();
    };
    let lit = |host: &mut Host| {
        host.tick();
        (1..=25).filter(|i| host.led(&format!("LED_TS_{i:02}")).is_some_and(|l| l.is_lit())).collect::<Vec<_>>()
    };
    let rgb = |host: &Host, i: usize| host.led_rgb(&format!("LED_TS_{i:02}")).unwrap();
    set("touchstrip_pos", 0.5);
    set("touchstrip_on_off", 1.0);
    assert_eq!(lit(&mut host), [12, 13, 14], "touched");
    let (mid, side) = (rgb(&host, 13), rgb(&host, 12));
    assert_ne!(mid, side, "the middle stands out while touched");
    set("touchstrip_on_off", 0.0);
    assert_eq!(lit(&mut host), [12, 13, 14], "let go: still there");
    set("touchstrip_pos", 0.0);
    assert_eq!(lit(&mut host), [1, 2], "the start");
    set("touchstrip_pos", 1.0);
    assert_eq!(lit(&mut host), [24, 25], "the end");
    // Another group (no Perform FX), then back.
    host.state.borrow_mut().focus.group = 1;
    assert_eq!(lit(&mut host), Vec::<usize>::new(), "a group without one");
    host.state.borrow_mut().focus.group = 0;
    assert_eq!(lit(&mut host), [24, 25], "back: where it was left");
    eprintln!("touched middle {mid:?}, sides {side:?}");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The strip's play position (NI's `updateLEDsPlayPositionMode`, outside PERFORM): a bar
/// filling from LED 1 as the scene plays, all 25 lit in its last 1/48; dark when stopped.
#[test]
fn mk3_strip_shows_the_play_position() {
    use deckard_lua_host::state::Playing;
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    doc.borrow_mut().add_pattern(0, 0, "Pattern 1").unwrap();
    doc.borrow_mut().assign_pattern(0, 0, Some(0)).unwrap();
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    let lit = |host: &mut Host| {
        host.tick();
        (1..=25).filter(|i| host.led(&format!("LED_TS_{i:02}")).is_some_and(|l| l.is_lit())).collect::<Vec<_>>()
    };
    assert_eq!(lit(&mut host), Vec::<usize>::new(), "stopped");
    let len = doc.borrow().project().scene_length(0) as f64;
    let mut at = |f: f64| {
        host.set_playhead((f * len) as i64, Some(Playing::Scene(0)));
        lit(&mut host)
    };
    assert_eq!(at(0.0), [1]);
    assert_eq!(at(0.5), (1..=13).collect::<Vec<_>>());
    assert_eq!(at(0.97), (1..=24).collect::<Vec<_>>());
    assert_eq!(at(0.99), (1..=25).collect::<Vec<_>>());
    host.set_playhead(0, None);
    assert_eq!(lit(&mut host), Vec::<usize>::new(), "stopped again");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// PAD MODE's Choke and Link (page 1): Group None/1–8 and Mode Send/Receive lists that
/// stop at their ends, kept in the project (the focused sound's); Focus (page 2) is On
/// Trigger or Pinned, and pads focus their sound On Trigger.
#[test]
fn mk3_pad_mode_choke_link_and_focus() {
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    host.press("BUTTON_PAD_MODE").unwrap();
    host.tick();
    let get = |host: &Host, f: &str, what: &str| -> String {
        host.lua
            .load(format!("local p = NI.DATA.StateHelper.getFocusSound(App):get{f}Parameter() return tostring(p:{what})"))
            .eval()
            .unwrap()
    };
    let channel = || doc.borrow().project().groups[0].sounds[0].channel.clone();
    host.encoder("ENCODER_SCREEN_1", -0.5).unwrap();
    host.tick();
    assert_eq!(get(&host, "ChokeGroup", "getValue()"), "0", "None is the first");
    assert_eq!(get(&host, "ChokeGroup", "getMax()"), "8");
    assert_eq!(get(&host, "ChokeGroup", "getAsString(3)"), "3");
    assert_eq!(get(&host, "ChokeGroup", "getTag()"), host.constant("NI.DATA.MaschineParameter.TAG_ENUM").unwrap().to_string());
    for _ in 0..20 {
        host.encoder("ENCODER_SCREEN_1", 0.5).unwrap();
        host.tick();
    }
    assert_eq!(get(&host, "ChokeGroup", "getValue()"), "8", "8 is the last");
    assert_eq!(channel().choke_group, 8, "kept in the project");
    for k in ["ENCODER_SCREEN_2", "ENCODER_SCREEN_3", "ENCODER_SCREEN_4"] {
        for _ in 0..3 {
            host.encoder(k, 0.5).unwrap();
            host.tick();
        }
    }
    let c = channel();
    assert_eq!((c.choke_mode, c.link_mode), (1, 1), "Receive");
    assert!(c.link_group > 0);
    assert_eq!(get(&host, "ChokeMode", "getValueString()"), "Receive");
    assert_eq!(get(&host, "LinkMode", "getAsString(0)"), "Send");
    // Focus.
    assert_eq!(host.constant("NI.DATA.Workspace.FOCUS_SOUND_ON_PAD_EVENT_ON_TRIGGER").unwrap(), 0);
    assert_eq!(host.constant("NI.DATA.Workspace.FOCUS_SOUND_ON_PAD_EVENT_PINNED").unwrap(), 1);
    let focus = |host: &Host, what: &str| -> String {
        host.lua.load(format!("return tostring(App:getWorkspace():getFocusSoundOnPadEventParameter():{what})")).eval().unwrap()
    };
    assert_eq!(focus(&host, "getMax()"), "1");
    assert_eq!(focus(&host, "getAsString(1)"), "Pinned");
    assert!(host.lua.load("return MaschineHelper.isFocusSoundOnPadEventEnabled()").eval::<bool>().unwrap(), "On Trigger");
    let hit = |host: &mut Host, pad: i64| {
        host.pad(pad, 1.0);
        host.tick();
        host.pad(pad, 0.0);
        host.tick();
        host.state.borrow().focus.sound
    };
    assert_eq!(hit(&mut host, 3), 2, "On Trigger: a pad focuses its sound");
    host.lua
        .load("local p = App:getWorkspace():getFocusSoundOnPadEventParameter() NI.DATA.ParameterAccess.setEnumParameter(App, p, 5)")
        .exec()
        .unwrap();
    assert_eq!(focus(&host, "getValue()"), "1", "kept to Pinned, the last");
    assert_eq!(hit(&mut host, 5), 2, "Pinned: the focus stays");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// DECKARD's built-in mod adds Pad Touch pages to the Keyboard page: page 3 Mode, Curve,
/// Min, Max; page 4 the CC; the focused sound's, kept in the project.
/// The Keyboard page's page number counts the Pad Touch pages from page 1 on ("1/4"),
/// not only once they're reached.
#[test]
fn mk3_keyboard_page_counts_the_pad_touch_pages() {
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    let Some(mut scripts) = scripts() else { return };
    scripts.add_builtin();
    let mut host = Host::new(scripts, State::new("M3", Some(doc))).expect("host");
    host.boot().unwrap();
    host.tick();
    // The page number as drawn: what the section names are drawn with.
    host.eval_lua(
        "local draw = ParameterHandler.updateSectionNames
         ParameterHandler.updateSectionNames = function(self, ...)
             PAGE_SHOWN = tostring(self.PageIndex) .. '/' .. tostring(self.NumPages)
             return draw(self, ...)
         end",
    );
    host.press("BUTTON_KEYBOARD").unwrap();
    host.tick();
    assert!(host.errors.is_empty(), "{:?}", host.errors);
    assert_eq!(host.lua_global::<String>("PAGE_SHOWN"), "1/4");
    host.press("BUTTON_RIGHT").unwrap();
    host.tick();
    assert_eq!(host.lua_global::<String>("PAGE_SHOWN"), "2/4");
    host.press("BUTTON_RIGHT").unwrap();
    host.tick();
    assert_eq!(host.lua_global::<String>("PAGE_SHOWN"), "3/4");
}

#[test]
fn mk3_keyboard_page_has_pad_touch() {
    use deckard_model::pad_touch::TouchMode;
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    let Some(mut scripts) = scripts() else { return };
    assert_eq!(scripts.add_builtin().applied, ["KeyboardModePageMK3"]);
    let mut host = Host::new(scripts, State::new("M3", Some(doc.clone()))).expect("host");
    host.boot().unwrap();
    host.tick();
    host.press("BUTTON_KEYBOARD").unwrap();
    host.tick();
    let touch = || doc.borrow().project().groups[0].sounds[0].pad_touch;
    for _ in 0..2 {
        host.press("BUTTON_RIGHT").unwrap();
        host.tick();
    }
    for _ in 0..3 {
        host.encoder("ENCODER_SCREEN_1", 0.5).unwrap();
        host.tick();
    }
    assert_ne!(touch().mode, TouchMode::Velocity, "Mode turned");
    host.encoder("ENCODER_SCREEN_2", 0.5).unwrap();
    host.tick();
    host.encoder("ENCODER_SCREEN_3", 0.5).unwrap();
    host.tick();
    host.encoder("ENCODER_SCREEN_4", -0.5).unwrap();
    host.tick();
    let t = touch();
    assert!(t.curve > 50 && t.min > 0 && t.max < 100, "{t:?}");
    host.press("BUTTON_RIGHT").unwrap();
    host.tick();
    host.encoder("ENCODER_SCREEN_1", 0.5).unwrap();
    host.tick();
    assert_eq!(touch().cc, 2);
    // Back on page 1, knob 1 is the scale bank again (Pad Touch unchanged).
    for _ in 0..3 {
        host.press("BUTTON_LEFT").unwrap();
        host.tick();
    }
    let (before, t) = (host.scale(), touch());
    host.encoder("ENCODER_SCREEN_1", 0.5).unwrap();
    host.tick();
    assert_ne!(host.scale(), before, "the scale bank turned");
    assert_eq!(touch(), t);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// DECKARD's Pad Touch parameters on a sound (the Keyboard page's mod shows them): Mode
/// a list, CC and the percentages with their value strings, Min and Max pushing each
/// other, kept in the project; and the song's Fixed Velocity whatever the pad mode.
#[test]
fn mk3_pad_touch_parameters() {
    use deckard_model::pad_touch::{PadTouch, TouchMode};
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    let get = |host: &Host, f: &str, what: &str| -> String {
        host.lua
            .load(format!("local p = NI.DATA.StateHelper.getFocusSound(App):get{f}Parameter() return tostring(p:{what})"))
            .eval()
            .unwrap()
    };
    let set = |host: &Host, f: &str, v: i64| {
        host.lua.load(format!("NI.DATA.StateHelper.getFocusSound(App):get{f}Parameter():setValue({v})")).exec().unwrap();
    };
    let touch = || doc.borrow().project().groups[0].sounds[0].pad_touch;
    assert_eq!(get(&host, "PadTouchMode", "getMax()"), "3");
    assert_eq!(get(&host, "PadTouchMode", "getAsString(1)"), "Poly AT");
    assert_eq!(get(&host, "PadTouchMode", "getTag()"), host.constant("NI.DATA.MaschineParameter.TAG_ENUM").unwrap().to_string());
    assert_eq!(get(&host, "PadTouchMode", "getValueString()"), "Velocity");
    set(&host, "PadTouchMode", 3);
    assert_eq!(touch().mode, TouchMode::Cc);
    assert_eq!(get(&host, "PadTouchCC", "getValueString()"), "1 Mod");
    assert_eq!(get(&host, "PadTouchCC", "getMax()"), "127");
    set(&host, "PadTouchCC", 74);
    assert_eq!(get(&host, "PadTouchCC", "getValueString()"), "74");
    assert_eq!(get(&host, "PadTouchCurve", "getValueString()"), "50 %");
    assert_eq!(get(&host, "PadTouchMax", "getMax()"), "100");
    set(&host, "PadTouchMax", 50);
    set(&host, "PadTouchMin", 70);
    assert_eq!((touch().min, touch().max), (70, 70), "Min pushes Max");
    set(&host, "PadTouchMax", 20);
    assert_eq!((touch().min, touch().max), (20, 20), "Max pushes Min");
    assert_eq!(touch(), PadTouch { mode: TouchMode::Cc, cc: 74, curve: 50, min: 20, max: 20 });
    // Quick turns undo together, as any knob's: back to before Min and Max met.
    assert!(doc.borrow_mut().undo());
    assert_eq!((touch().min, touch().max), (0, 50), "undone");
    // Fixed Velocity: the song's, also when the pads aren't in Fixed Velocity.
    assert_eq!(host.fixed_velocity_value(), 100);
    doc.borrow_mut().set("Fixed Velocity", deckard_doc::Target::Song, "fixed_velocity", mxprj::decode::Value::Int(64)).unwrap();
    assert_eq!(host.fixed_velocity_value(), 64);
    assert_eq!(host.fixed_velocity(), None, "pads not in Fixed Velocity");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// Record Prepare (hold REC): a pattern's length reads Auto until one is chosen there
/// (knob 4, buttons 5–8), which turns Auto off.
#[test]
fn mk3_record_prepare_shows_and_ends_auto_length() {
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    doc.borrow_mut().add_pattern(0, 0, "Pattern 1").unwrap();
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.state.borrow_mut().focus.pattern = Some(0);
    host.tick();
    let auto = |host: &Host| {
        host.lua
            .load("return NI.DATA.EventPatternAccess.hasFocusedPatternAutomaticLength(NI.DATA.StateHelper.getFocusSong(App))")
            .eval::<bool>()
            .unwrap()
    };
    assert!(auto(&host));
    host.lua.load("PatternHelper.setPatternOrClipEventLength(4)").exec().unwrap();
    assert!(!auto(&host), "a length chosen");
    assert!(!doc.borrow().length_is_auto(0, 0));
    doc.borrow_mut().set_length_auto(0, 0, true);
    host.lua
        .load("NI.DATA.EventPatternAccess.incrementExplicitLength(App, NI.DATA.StateHelper.getFocusEventPattern(App), 1, false, false)")
        .exec()
        .unwrap();
    assert!(!doc.borrow().length_is_auto(0, 0), "the knob too");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// REC (startEventRecording) doesn't start the song: the app arms, or with SHIFT counts
/// in and starts it itself.
#[test]
fn mk3_rec_leaves_the_transport_to_the_app() {
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    host.tick();
    for shift in [false, true] {
        host.lua.load(format!("NI.DATA.TransportAccess.startEventRecording(App, {shift}, false)")).exec().unwrap();
        let t = host.state.borrow().transport.clone();
        assert!(t.recording);
        assert_eq!(t.count_in, shift);
        assert!(!t.playing, "shift {shift}: not started here");
        host.lua.load("NI.DATA.TransportAccess.stopEventRecording(App)").exec().unwrap();
    }
}

/// MACRO shows the Macro properties (as Maschine does natively) and lights; SHIFT +
/// MACRO (Set Macro): touching a knob asks the app to make its parameter a macro (or
/// stop), each knob showing whether it is one; the parameter cache tells modulated
/// knobs.
#[test]
fn mk3_macro_page_and_set_macro() {
    use deckard_doc::Target;
    use deckard_lua_host::state::{ControlKnob, ControlRequest};
    let doc = deckard_doc::Document::new_project().unwrap().shared();
    let (level, pan) = {
        let d = doc.borrow();
        (d.param_oid(Target::Sound(0, 0), "level").unwrap(), d.param_oid(Target::Sound(0, 0), "pan").unwrap())
    };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    {
        let mut st = host.state.borrow_mut();
        st.control.level = 2;
        st.control.modules_visible = true;
        st.control.page_groups = 4;
        let knob = |name: &str, param| Some(ControlKnob { name: name.into(), param: Some(param), ..Default::default() });
        st.control.knobs = vec![knob("LEVEL", level), knob("PAN", pan), None, None, None, None, None, None];
    }
    host.tick();
    let reqs = |host: &Host| std::mem::take(&mut host.state.borrow_mut().control_requests);
    let eval = |host: &Host, code: &str| host.lua.load(code).eval::<bool>().unwrap();
    let lit = |host: &Host, led: &str| {
        let id = host.constant(&format!("NI.HW.{led}")).unwrap();
        host.state.borrow().hw.leds.get(&id).is_some_and(|l| l.is_lit())
    };
    reqs(&host);
    // Set Macro: touch knob 1.
    host.button("BUTTON_SHIFT", true).unwrap();
    host.press("BUTTON_MACRO").unwrap();
    host.button("BUTTON_SHIFT", false).unwrap();
    host.tick();
    assert_eq!(host.state.borrow().hw.page_stack.last().copied(), host.constant("NI.HW.PAGE_MACRO_ASSIGN").ok());
    reqs(&host);
    host.button("BUTTON_CAP_1", true).unwrap();
    host.button("BUTTON_CAP_1", false).unwrap();
    host.tick();
    assert!(reqs(&host).contains(&ControlRequest::SetMacro { pos: 0, level: 2 }));
    // The app made it one: knob 1 shows it, knob 2 doesn't.
    doc.borrow_mut().add_macro(Target::Sound(0, 0), Target::Sound(0, 0), "level").unwrap();
    host.tick();
    let any = |host: &Host, pos: usize, level: usize| {
        eval(
            host,
            &format!(
                "return NI.DATA.ParameterPageAccess.anyMacrosForParameter(App, NI.DATA.ParameterCache.getParameterByPosition(App, {pos}), {level})"
            ),
        )
    };
    assert!(any(&host, 0, 2));
    assert!(!any(&host, 1, 2));
    assert!(!any(&host, 0, 1), "not a group macro");
    // MACRO leaves Set Macro; again, the Macro properties.
    host.press("BUTTON_MACRO").unwrap();
    host.tick();
    assert_eq!(host.state.borrow().hw.page_stack.last().copied(), host.constant("NI.HW.PAGE_CONTROL").ok());
    host.press("BUTTON_MACRO").unwrap();
    host.tick();
    assert_eq!(host.state.borrow().hw.page_stack.last().copied(), host.constant("NI.HW.PAGE_MACRO").ok());
    let r = reqs(&host);
    assert!(r.contains(&ControlRequest::ModulesVisible(false)) && r.contains(&ControlRequest::PageGroup(3)), "{r:?}");
    assert!(eval(&host, "return NI.DATA.ParameterPageAccess.isMacroPageActive(App)"));
    assert!(lit(&host, "LED_MACRO"));
    // Its knobs are the app's to turn.
    host.encoder("ENCODER_SCREEN_1", 0.1).unwrap();
    assert!(reqs(&host).iter().any(|r| matches!(r, ControlRequest::Turn { pos: 0, .. })));
    // The parameter cache: a modulated knob.
    host.state.borrow_mut().control.knobs[1].as_mut().unwrap().modulated = true;
    assert!(eval(
        &host,
        "return App:getStateCache():getParameterCache():isAutomated(NI.DATA.ParameterCache.getParameterByPosition(App, 1))"
    ));
    let stubs: Vec<String> = host
        .recorder
        .borrow()
        .hits
        .keys()
        .filter(|k| k.contains("ParameterPageAccess") || k.contains("getParameterCache"))
        .cloned()
        .collect();
    assert!(stubs.is_empty(), "{stubs:?}");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The screens that show group and sound colours redraw a colour changed in the
/// software at once, without leaving the page.
#[test]
fn mk3_pages_redraw_colours_at_once() {
    let Some(p) = project() else { return };
    for page in [None, Some("BUTTON_PAD_MODE"), Some("BUTTON_MIXER"), Some("BUTTON_PATTERN")] {
        let Some(mut host) = boot("M3", Some(p.clone())) else { return };
        if let Some(b) = page {
            host.button(b, true).unwrap();
            host.button(b, false).unwrap();
        }
        host.tick();
        let before = format!("{:?}", host.display());
        let (g, s) = (host.state.borrow().focus.group, host.state.borrow().focus.sound);
        let targets = [deckard_doc::Target::Group(g), deckard_doc::Target::Sound(g, s)];
        p.borrow_mut().set_color(&targets, deckard_model::Color::Palette(11)).unwrap();
        host.tick();
        assert_ne!(format!("{:?}", host.display()), before, "{page:?} shows the new colour");
        p.borrow_mut().undo();
        assert!(host.errors.is_empty(), "{:?}", host.errors);
    }
}

/// VOLUME, TEMPO and SWING by the 4-D encoder: each switches the wheel to the master's
/// volume (a click 0.5 dB), tempo (a BPM) or swing (1 %), in the project.
#[test]
fn mk3_wheel_sets_master_volume_tempo_and_swing() {
    let Some(p) = project() else { return };
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    let master = |p: &deckard_doc::Shared| {
        let d = p.borrow();
        let pr = d.project();
        (deckard_model::level_db(pr.master_level), pr.tempo, pr.swing.amount)
    };
    let mut turn = |b: &str, clicks: i64| {
        host.button(b, true).unwrap();
        host.button(b, false).unwrap();
        host.tick();
        host.wheel(clicks);
        host.tick();
        host.button(b, true).unwrap();
        host.button(b, false).unwrap();
        host.tick();
    };
    let (db, tempo, swing) = master(&p);
    turn("BUTTON_VOLUME", -4);
    assert!((master(&p).0 - (db - 2.0)).abs() < 1e-6, "{db} dB less 2: {}", master(&p).0);
    turn("BUTTON_TEMPO", 3);
    assert!((master(&p).1 - (tempo + 3.0)).abs() < 1e-6, "a click a BPM: {}", master(&p).1);
    turn("BUTTON_SWING", 5);
    assert!((master(&p).2 - (swing + 5.0)).abs() < 1e-6, "swing {}", master(&p).2);
    turn("BUTTON_SWING", -200);
    assert_eq!(master(&p).2, 0.0, "not below 0 %");
    // The values as the screen shows them.
    host.eval_lua(
        "local Song = NI.DATA.StateHelper.getFocusSong(App)
         SWING_TEXT = Song:getSwingAmountParameter():getValueString()
         SWING_MAX = Song:getSwingAmountParameter():getMax()",
    );
    assert_eq!(host.lua_global::<String>("SWING_TEXT"), "0.0 %");
    assert_eq!(host.lua_global::<f64>("SWING_MAX"), 100.0);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The CHANNEL page's groups have Maschine's names (its arrow label shows the focused
/// one's): Input, Output, Groove, Macro.
#[test]
fn page_groups_have_their_names() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc)) else { return };
    host.tick();
    let names: Vec<String> = (0..4)
        .map(|i| host.lua.load(format!("return App:getWorkspace():getPageGroupParameter():getAsString({i})")).eval().unwrap())
        .collect();
    assert_eq!(names, ["Input", "Output", "Groove", "Macro"]);
}

/// The MK3's AUTO (its HID name `BUTTON_AUTO`) is Maschine's `BUTTON_AUTO_WRITE`:
/// SHIFT + AUTO locks writing on, AUTO then turns it off; held, it writes.
#[test]
fn mk3_auto_writes_while_held() {
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    let set = |host: &Host| host.state.borrow_mut().auto_write_set.take();
    host.button("BUTTON_SHIFT", true).unwrap();
    host.tick();
    host.press("BUTTON_AUTO").unwrap();
    host.button("BUTTON_SHIFT", false).unwrap();
    host.tick();
    assert_eq!(set(&host), Some(true), "locked");
    host.press("BUTTON_AUTO").unwrap();
    assert_eq!(set(&host), Some(false), "unlocked");
    host.button("BUTTON_AUTO", true).unwrap();
    assert_eq!(set(&host), Some(true));
    host.button("BUTTON_AUTO", false).unwrap();
    assert_eq!(set(&host), Some(false));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// SHIFT + FOLLOW's Grid page: PERFORM's pads set the project's Perform Grid (pad 5,
/// the first column's second, is 1/2), pad 16 turns it off and on again.
#[test]
fn mk3_grid_page_sets_the_perform_grid() {
    let Some(p) = project() else { return };
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    host.tick();
    host.button("BUTTON_SHIFT", true).unwrap();
    host.button("BUTTON_FOLLOW", true).unwrap();
    host.tick();
    let perform = |p: &deckard_doc::Shared| p.borrow().num(deckard_doc::Target::Song, "perform_grid");
    host.pad_event(5, true, 1.0);
    host.tick();
    assert_eq!(perform(&p), Some(2.0));
    host.pad_event(16, true, 1.0);
    host.tick();
    assert_eq!(perform(&p), Some(0.0), "OFF");
    host.pad_event(16, true, 1.0);
    host.tick();
    assert_eq!(perform(&p), Some(2.0), "back on");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The Smart Strip's PITCH and MOD on the MK3 (native in Maschine): a bend of the
/// focused sound that springs back, and its mod wheel, kept for the strip's lights.
#[test]
fn mk3_strip_bends_and_modulates_the_focused_sound() {
    use deckard_lua_host::state::ControlRequest;
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    let strip = |host: &mut Host| -> Vec<(bool, f64)> {
        let mut st = host.state.borrow_mut();
        let out = st.control_requests.iter().filter_map(|r| match r {
            ControlRequest::StripControl { group: 0, sound: 0, pitch, value } => Some((*pitch, *value)),
            _ => None,
        });
        let out = out.collect();
        st.control_requests.clear();
        out
    };
    host.press("BUTTON_PITCH").unwrap();
    host.tick();
    host.touch(0, 0, 0.75);
    host.touch(0, 2, 0.75);
    assert_eq!(strip(&mut host), [(true, 0.5), (true, 0.0)], "back to the middle once let go");
    host.press("BUTTON_MOD").unwrap();
    host.tick();
    host.touch(0, 0, 0.4);
    host.touch(0, 2, 0.4);
    assert_eq!(strip(&mut host), [(false, 0.4)], "it stays");
    host.eval_lua("StripMod = NHLController:getContext():getLastTouchstripValueForSound(NI.DATA.StateHelper.getFocusSound(App))");
    assert_eq!(host.lua_global::<f64>("StripMod"), 0.4);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The Smart Strip's NOTES on the MK3 (native in Maschine, BUGS #13): the strip strums
/// the pads held (else all 16) along its length, each note held till the finger moves
/// on or lifts.
#[test]
fn mk3_strip_strums_the_pads() {
    use deckard_lua_host::state::ControlRequest;
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    let notes = |host: &mut Host| -> Vec<(usize, i64, bool)> {
        let mut st = host.state.borrow_mut();
        let out = st.control_requests.iter().filter_map(|r| match r {
            ControlRequest::StripNote { group: 0, sound, pitch, on, .. } => Some((*sound, *pitch, *on)),
            _ => None,
        });
        let out = out.collect();
        st.control_requests.clear();
        out
    };
    host.press("BUTTON_NOTES").unwrap();
    host.tick();
    notes(&mut host);
    host.touch(0, 0, 0.0);
    host.touch(0, 1, 0.01);
    host.touch(0, 1, 0.99);
    host.touch(0, 2, 0.99);
    let base = |s: usize| project().unwrap().borrow().project().groups[0].sounds[s].channel.base_key;
    assert_eq!(notes(&mut host), [(0, base(0), true), (0, base(0), false), (15, base(15), true), (15, base(15), false)], "all 16 along it");
    // Two pads held: the strip is those two.
    host.pad_event(3, true, 0.5);
    host.pad_event(7, true, 0.5);
    notes(&mut host);
    host.touch(0, 0, 0.2);
    host.touch(0, 1, 0.8);
    host.touch(0, 2, 0.8);
    assert_eq!(notes(&mut host), [(2, base(2), true), (2, base(2), false), (6, base(6), true), (6, base(6), false)]);
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

#[test]
fn mk3_step_mode_prob_lock_locks_the_focused_sound() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    host.press("BUTTON_STEP").unwrap();
    host.tick();
    let (g, s) = {
        let st = host.state.borrow();
        (st.focus.group, st.focus.sound)
    };
    let locked =
        |doc: &std::rc::Rc<std::cell::RefCell<deckard_doc::Document>>| doc.borrow().project().groups[g].sounds[s].probability_locked;
    assert!(!locked(&doc));
    host.press("BUTTON_SCREEN_8").unwrap();
    host.tick();
    assert!(locked(&doc), "PROB LOCK on");
    assert!(host.led("LED_SCREEN_BUTTON_8").is_some_and(|l| l.is_lit()));
    host.press("BUTTON_SCREEN_8").unwrap();
    host.tick();
    assert!(!locked(&doc));
}

#[test]
fn mk3_holding_a_step_shows_step_mod() {
    let Some(doc) = project() else { return };
    let Some(mut host) = boot("M3", Some(doc.clone())) else { return };
    host.tick();
    host.press("BUTTON_STEP").unwrap();
    host.tick();
    host.pad_event(3, true, 0.8);
    for _ in 0..20 {
        host.tick();
    }
    let top = host.state.borrow().hw.page_stack.last().copied();
    assert_eq!(top, host.constant("NI.HW.PAGE_STEP_MOD").ok(), "the Step Mod page: {:?}", host.state.borrow().log);
    // (Pushed from the controller timer: it's the page shown too.)
    let shown: String = host
        .lua
        .load("local p = ControllerScriptInterface.ActivePage; return p and (p.Name or tostring(p)) or ''")
        .eval()
        .unwrap_or_default();
    assert!(shown.contains("StepPageMod"), "shown: {shown:?}");
}

/// #25: with SHIFT held the pads run the shortcuts printed under them and don't play
/// (Maschine does this natively), in Keyboard mode too.
#[test]
fn mk3_shift_keeps_the_pads_from_playing() {
    let Some(mut host) = boot("M3", project()) else { return };
    host.tick();
    for keyboard in [false, true] {
        if keyboard {
            host.button("BUTTON_KEYBOARD", true).unwrap();
            host.button("BUTTON_KEYBOARD", false).unwrap();
            host.tick();
        }
        assert!(host.pads_play_sounds(), "keyboard {keyboard}");
        host.button("BUTTON_SHIFT", true).unwrap();
        host.tick();
        assert!(!host.pads_play_sounds(), "keyboard {keyboard}, SHIFT held");
        host.button("BUTTON_SHIFT", false).unwrap();
        host.tick();
    }
}

/// SHIFT + ERASE + a group button deletes that group (the last one is reset), as
/// Maschine's `removeGroupResetLast`.
#[test]
fn mk3_shift_erase_and_a_group_button_delete_it() {
    let p = deckard_doc::Document::new_project().unwrap().shared();
    let Some(mut host) = boot("M3", Some(p.clone())) else { return };
    p.borrow_mut().add_group().unwrap();
    p.borrow_mut().rename(deckard_doc::Target::Group(1), "Keep").unwrap();
    host.tick();
    host.button("BUTTON_SHIFT", true).unwrap();
    host.button("BUTTON_TRANSPORT_ERASE", true).unwrap();
    host.press("BUTTON_GROUP_A").unwrap();
    host.tick();
    assert_eq!(p.borrow().project().groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Keep"]);
    assert_eq!(host.state.borrow().focus.group, 0);
    host.press("BUTTON_GROUP_A").unwrap();
    host.tick();
    assert_eq!(p.borrow().project().groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), [""], "the last: reset");
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}
