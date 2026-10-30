use super::*;
use crate::tests_support::view;

fn press(s: &mut Surface, v: &View, name: &'static str) -> Vec<Action> {
    let now = Instant::now();
    let mut out = s.event(&Event::Button(name, true), v, now);
    out.extend(s.event(&Event::Button(name, false), v, now));
    out
}

fn hold(s: &mut Surface, v: &View, name: &'static str, down: bool) -> Vec<Action> {
    s.event(&Event::Button(name, down), v, Instant::now())
}

#[test]
fn play_toggles() {
    let (mut s, mut v) = (Surface::new(), view());
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_PLAY"), [Action::Play]);
    v.playing = true;
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_PLAY"), [Action::Stop]);
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_STOP"), [Action::Stop]);
}

#[test]
fn shift_rec_counts_in() {
    let (mut s, mut v) = (Surface::new(), view());
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_RECORD"), [Action::Record { on: true, count_in: false }]);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_RECORD"), [Action::Record { on: true, count_in: true }]);
    v.recording = true;
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_RECORD"), [Action::Record { on: false, count_in: false }]);
}

#[test]
fn shift_restart_loops() {
    let (mut s, v) = (Surface::new(), view());
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_LOOP"), [Action::Restart]);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_LOOP"), [Action::Loop(true)]);
    assert_eq!(press(&mut s, &v, "BUTTON_TAP"), [Action::Metronome(true)]);
}

#[test]
fn taps_set_the_tempo() {
    let (mut s, v) = (Surface::new(), view());
    let t0 = Instant::now();
    let tap = |s: &mut Surface, ms: u64| s.event(&Event::Button("BUTTON_TAP", true), &v, t0 + Duration::from_millis(ms));
    assert!(tap(&mut s, 0).is_empty(), "one tap: no tempo yet");
    assert_eq!(tap(&mut s, 500), [Action::Tempo(120.0)]);
    assert_eq!(tap(&mut s, 1000), [Action::Tempo(120.0)]);
    // A long pause starts a new count.
    assert!(tap(&mut s, 5000).is_empty());
    assert_eq!(tap(&mut s, 5400), [Action::Tempo(150.0)]);
}

#[test]
fn auto_is_a_toggle() {
    // (C38: a press on, the next off; DECKARD's, not Maschine's hold-to-write.)
    let (mut s, mut v) = (Surface::new(), view());
    assert_eq!(hold(&mut s, &v, "BUTTON_AUTO", true), [Action::AutoWrite(true)]);
    v.auto_write = true;
    assert!(hold(&mut s, &v, "BUTTON_AUTO", false).is_empty(), "stays on once let go");
    assert_eq!(press(&mut s, &v, "BUTTON_AUTO"), [Action::AutoWrite(false)], "the next press: off");
    v.auto_write = false;
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(press(&mut s, &v, "BUTTON_AUTO"), [Action::AutoWrite(true)], "SHIFT + AUTO as well");
    v.auto_write = true;
    assert_eq!(press(&mut s, &v, "BUTTON_AUTO"), [Action::AutoWrite(false)]);
}

#[test]
fn group_buttons_focus() {
    let (mut s, v) = (Surface::new(), view());
    assert_eq!(press(&mut s, &v, "BUTTON_GROUP_B"), [Action::FocusGroup(1)]);
    hold(&mut s, &v, "BUTTON_MUTE", true);
    assert_eq!(press(&mut s, &v, "BUTTON_GROUP_A"), [Action::MuteGroup(0)]);
    hold(&mut s, &v, "BUTTON_MUTE", false);
    hold(&mut s, &v, "BUTTON_SOLO", true);
    assert_eq!(press(&mut s, &v, "BUTTON_GROUP_B"), [Action::SoloGroup(1)]);
}

#[test]
fn shift_picks_the_group_bank() {
    let (mut s, mut v) = (Surface::new(), view());
    v.groups = (0..10).map(|_| v.groups[0].clone()).collect();
    v.group = 2;
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(press(&mut s, &v, "BUTTON_GROUP_B"), [Action::FocusGroup(8)], "bank 2, its first group");
    assert_eq!(press(&mut s, &v, "BUTTON_GROUP_A"), [Action::FocusGroup(0)], "the bank shown too");
    assert!(press(&mut s, &v, "BUTTON_GROUP_C").is_empty(), "no bank after one with room");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    v.group = 8;
    assert_eq!(press(&mut s, &v, "BUTTON_GROUP_B"), [Action::FocusGroup(9)], "B of bank 2");
}

#[test]
fn the_slot_after_the_last_group_adds_one() {
    let (mut s, mut v) = (Surface::new(), view());
    v.groups = (0..10).map(|_| v.groups[0].clone()).collect();
    v.group = 8;
    let leds = s.leds(&v);
    assert_eq!(leds.get("LED_GROUP_C"), Some(deckard_mk3::leds::palette(16, 2)), "dim white: a new group");
    assert_eq!(leds.get("LED_GROUP_D"), Some(0));
    assert_eq!(press(&mut s, &v, "BUTTON_GROUP_C"), [Action::AddGroup]);
    assert!(press(&mut s, &v, "BUTTON_GROUP_D").is_empty());
    v.groups.truncate(8);
    v.group = 0;
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(s.leds(&v).get("LED_GROUP_B"), Some(deckard_mk3::leds::palette(16, 2)), "SHIFT: a new bank");
    assert_eq!(press(&mut s, &v, "BUTTON_GROUP_B"), [Action::AddGroup], "its first group");
    assert!(press(&mut s, &v, "BUTTON_GROUP_C").is_empty());
}

/// SHIFT + ERASE + a group button: that group (of the focused bank) deleted.
#[test]
fn shift_erase_and_a_group_button_delete_it() {
    let (mut s, mut v) = (Surface::new(), view());
    v.groups = (0..10).map(|_| v.groups[0].clone()).collect();
    v.group = 8;
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    hold(&mut s, &v, "BUTTON_ERASE", true);
    assert_eq!(press(&mut s, &v, "BUTTON_GROUP_B"), [Action::DeleteGroup(9)]);
    assert!(press(&mut s, &v, "BUTTON_GROUP_C").is_empty(), "no group there");
}

#[test]
fn missing_groups_do_nothing() {
    let (mut s, mut v) = (Surface::new(), view());
    assert!(press(&mut s, &v, "BUTTON_GROUP_H").is_empty());
    v.groups = (0..8).map(|_| v.groups[0].clone()).collect();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert!(press(&mut s, &v, "BUTTON_GROUP_C").is_empty(), "no bank 3");
}

/// With fewer than eight groups there are no banks to pick: SHIFT leaves the group
/// buttons as they are (lights and presses), as Maschine's (`PageMaschine`); BUGS #40.
#[test]
fn shift_with_one_bank_leaves_the_group_buttons_alone() {
    let (mut s, v) = (Surface::new(), view());
    let groups = |s: &Surface| {
        let leds = s.leds(&v);
        (b'A'..=b'H').map(|c| leds.get(&format!("LED_GROUP_{}", c as char))).collect::<Vec<_>>()
    };
    let before = groups(&s);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(groups(&s), before, "the groups, lit as without SHIFT");
    assert_eq!(press(&mut s, &v, "BUTTON_GROUP_B"), [Action::FocusGroup(1)]);
    assert_eq!(press(&mut s, &v, "BUTTON_GROUP_C"), [Action::AddGroup], "the place after the last");
}

/// SHIFT + ERASE: the group buttons show the groups (to pick the one to delete), not
/// the banks, as Maschine's.
#[test]
fn shift_and_erase_show_the_groups() {
    let (mut s, mut v) = (Surface::new(), view());
    v.groups = (0..10).map(|_| v.groups[0].clone()).collect();
    v.group = 8;
    let groups = |s: &Surface, v: &View| {
        let leds = s.leds(v);
        (b'A'..=b'H').map(|c| leds.get(&format!("LED_GROUP_{}", c as char))).collect::<Vec<_>>()
    };
    let before = groups(&s, &v);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_ne!(groups(&s, &v), before, "SHIFT alone: the banks");
    hold(&mut s, &v, "BUTTON_TRANSPORT_ERASE", true);
    assert_eq!(groups(&s, &v), before, "with ERASE: the groups");
}

#[test]
fn pads_focus_their_sound() {
    let (mut s, v) = (Surface::new(), view());
    let now = Instant::now();
    assert_eq!(s.event(&Event::PadOn(3, 0.8), &v, now), [Action::FocusSound(2)]);
    assert!(s.event(&Event::PadOn(1, 0.8), &v, now).is_empty(), "already focused");
    assert!(s.pads(&v).play);
}

#[test]
fn select_mute_solo_erase_with_pads() {
    let (mut s, v) = (Surface::new(), view());
    let now = Instant::now();
    for (button, want) in [
        ("BUTTON_SELECT", Action::FocusSound(4)),
        ("BUTTON_MUTE", Action::MuteSound(4)),
        ("BUTTON_SOLO", Action::SoloSound(4)),
        ("BUTTON_TRANSPORT_ERASE", Action::EraseSound(4)),
    ] {
        hold(&mut s, &v, button, true);
        assert_eq!(s.event(&Event::PadOn(5, 0.5), &v, now), [want], "{button}");
        s.event(&Event::PadOff(5), &v, now);
        hold(&mut s, &v, button, false);
    }
}

#[test]
fn pads_play_with_modifiers_held() {
    // (As with Maschine's scripts: only picking patterns or scenes, and Step mode,
    // stop the pads playing.)
    let (mut s, v) = (Surface::new(), view());
    for button in ["BUTTON_SELECT", "BUTTON_MUTE", "BUTTON_SOLO", "BUTTON_TRANSPORT_ERASE"] {
        hold(&mut s, &v, button, true);
        assert!(s.pads(&v).play, "{button} held");
        hold(&mut s, &v, button, false);
    }
    for button in ["BUTTON_PATTERN", "BUTTON_SCENE"] {
        hold(&mut s, &v, button, true);
        assert!(!s.pads(&v).play, "{button} held");
        hold(&mut s, &v, button, false);
        assert!(s.pads(&v).play, "{button} let go");
    }
}

#[test]
fn keyboard_plays_from_c3_and_moves_by_octave() {
    let (mut s, mut v) = (Surface::new(), view());
    v.sound = 2;
    assert_eq!(press(&mut s, &v, "BUTTON_KEYBOARD"), [Action::KeyboardMode(true)]);
    let (sound, keys) = s.pads(&v).keys.expect("keyboard mode");
    assert_eq!((sound, keys[0], keys[15]), (2, 60, 75));
    assert!(s.event(&Event::PadOn(5, 0.5), &v, Instant::now()).is_empty(), "keys don't change the focus");
    press(&mut s, &v, "BUTTON_SCREEN_6");
    assert_eq!(s.pads(&v).keys.unwrap().1[0], 72);
    for _ in 0..20 {
        press(&mut s, &v, "BUTTON_SCREEN_5");
    }
    assert_eq!(s.pads(&v).keys.unwrap().1[0], 0, "no lower than MIDI 0");
    for _ in 0..20 {
        press(&mut s, &v, "BUTTON_SCREEN_6");
    }
    assert!(s.pads(&v).keys.unwrap().1[15] <= 127, "no higher than MIDI 127");
    assert_eq!(press(&mut s, &v, "BUTTON_PAD_MODE"), [Action::KeyboardMode(false)]);
    assert!(s.pads(&v).keys.is_none());
}

#[test]
fn knobs_turn_the_control_page() {
    let (mut s, v) = (Surface::new(), view());
    let now = Instant::now();
    assert_eq!(s.event(&Event::Knob(2, 0.05), &v, now), [Action::Turn { knob: 2, inc: 0.05, fine: false }]);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(s.event(&Event::Knob(7, -0.01), &v, now), [Action::Turn { knob: 7, inc: -0.01, fine: true }]);
    assert_eq!(press(&mut s, &v, "BUTTON_RIGHT"), [Action::Page(true)]);
    assert_eq!(press(&mut s, &v, "BUTTON_LEFT"), [Action::Page(false)]);
    assert_eq!(press(&mut s, &v, "BUTTON_CHANNEL"), [Action::ModulesVisible(false)]);
    assert_eq!(press(&mut s, &v, "BUTTON_PLUGIN"), [Action::ModulesVisible(true)]);
}

#[test]
fn display_buttons_by_mode() {
    let (mut s, mut v) = (Surface::new(), view());
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_1"), [Action::Level(0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_3"), [Action::Level(2)]);
    // PLUG-IN: display 5 / 6 and the jog wheel's left / right the slots, on to the "+".
    assert!(press(&mut s, &v, "BUTTON_SCREEN_5").is_empty(), "the first slot");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::Slot(1)], "on to the +");
    assert_eq!(press(&mut s, &v, "BUTTON_WHEEL_RIGHT"), [Action::Slot(1)]);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_8").is_empty());
    // CHANNEL: display 5 / 6 and the jog wheel's left / right the page groups.
    v.control.modules_visible = false;
    v.control.page_groups = 4;
    assert!(press(&mut s, &v, "BUTTON_SCREEN_5").is_empty(), "Input is the first");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::PageGroup(1)]);
    assert_eq!(press(&mut s, &v, "BUTTON_WHEEL_RIGHT"), [Action::PageGroup(1)]);
    v.control.page_group = 3;
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::PageGroup(2)]);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_6").is_empty(), "Macro is the last");
}

/// CHANNEL's arrow label names the page group; PLUG-IN's the slot, EMPTY on the "+".
#[test]
fn the_arrow_label_names_the_page_group_or_slot() {
    let (s, mut v) = (Surface::new(), view());
    v.control.modules_visible = false;
    v.control.page_groups = 4;
    v.control.page_group = 2;
    assert!(s.screen_text(&v).iter().any(|t| t == "GROOVE"), "{:?}", s.screen_text(&v));
    // (The tiles name their group once: "Input", no "INPUT" under it.)
    assert!(s.screen_text(&v).iter().any(|t| t == "Input"));
    assert!(!s.screen_text(&v).iter().any(|t| t == "INPUT"), "C13");
    v.control.modules_visible = true;
    assert!(s.screen_text(&v).iter().any(|t| t == "COMPRESSOR"));
    v.control.slot_index = None;
    assert!(s.screen_text(&v).iter().any(|t| t == "EMPTY"), "the +");
}

/// PLUG-IN, the jog wheel pressed: Maschine's plug-in browser (the Module page) for the
/// focused slot, or the "+"; LOAD loads the one chosen there.
#[test]
fn the_module_page_loads_a_plugin() {
    let (mut s, mut v) = (Surface::new(), view());
    v.control.modules = vec![
        ModuleView { name: "Sampler".into(), instrument: true, format: "Internal".into(), vendor: "Native Instruments".into() },
        ModuleView { name: "EQ".into(), instrument: false, format: "Internal".into(), vendor: "Native Instruments".into() },
        ModuleView { name: "Compressor".into(), instrument: false, format: "Internal".into(), vendor: "Native Instruments".into() },
        ModuleView { name: "Raum".into(), instrument: false, format: "VST3".into(), vendor: "Native Instruments".into() },
        ModuleView { name: "Microcosm".into(), instrument: false, format: "VST3".into(), vendor: "Panic Signals".into() },
    ];
    v.control.slot_index = None;
    press(&mut s, &v, "BUTTON_WHEEL");
    assert_eq!(s.page(), "module");
    for theme in s.themes() {
        s.set_theme(&theme);
        assert!(s.screens(&v).is_some());
        assert_eq!(s.error(), None, "{theme}");
    }
    let text = s.screen_text(&v);
    assert!(
        ["PREVIOUS", "NEXT", "CANCEL", "LOAD", "(none)", "EQ", "Compressor", "FORMAT", "Internal", "DECKARD"]
            .iter()
            .all(|t| text.contains(&t.to_string())),
        "{text:?}"
    );
    assert!(!text.contains(&"Sampler".to_string()), "an effect slot lists effects");
    assert!(!text.contains(&"NI".to_string()), "the built-ins are DECKARD's (C41)");
    // Not an instrument slot (the sound's first is full): effects, Internal, DECKARD.
    let now = Instant::now();
    assert!(s.event(&Event::Wheel(1), &v, now).is_empty(), "to the first effect");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::LoadModule(Some(1))], "EQ");
    assert_eq!(s.page(), "control", "loading closes it");
    press(&mut s, &v, "BUTTON_WHEEL");
    // FORMAT: VST3; VENDOR: on to Panic Signals.
    assert!(s.event(&Event::Knob(1, 0.1), &v, now).is_empty());
    assert!(s.event(&Event::Knob(2, 0.1), &v, now).is_empty());
    s.event(&Event::Wheel(1), &v, now);
    assert_eq!(press(&mut s, &v, "BUTTON_WHEEL"), [Action::LoadModule(Some(4))], "Microcosm");
    // (none) takes the focused slot out; CANCEL closes the page.
    v.control.slot_index = Some(0);
    press(&mut s, &v, "BUTTON_WHEEL");
    s.event(&Event::Knob(1, -0.1), &v, now);
    s.event(&Event::Wheel(-5), &v, now);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::LoadModule(None)]);
    press(&mut s, &v, "BUTTON_WHEEL");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_7").is_empty());
    assert_eq!(s.page(), "control");
}

#[test]
fn the_wheel_changes_tempo_within_range() {
    let (mut s, mut v) = (Surface::new(), view());
    press(&mut s, &v, "BUTTON_TEMPO");
    assert_eq!(s.event(&Event::Wheel(3), &v, Instant::now()), [Action::Tempo(123.0)]);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(s.event(&Event::Wheel(-2), &v, Instant::now()), [Action::Tempo(119.8)]);
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    v.tempo = 398.0;
    assert_eq!(s.event(&Event::Wheel(50), &v, Instant::now()), [Action::Tempo(400.0)]);
    v.tempo = 12.0;
    assert_eq!(s.event(&Event::Wheel(-50), &v, Instant::now()), [Action::Tempo(10.0)]);
}

#[test]
fn wheel_modes_toggle() {
    let (mut s, v) = (Surface::new(), view());
    let now = Instant::now();
    assert_eq!(s.event(&Event::Wheel(1), &v, now), [Action::FocusSound(1)], "no mode: the sound");
    assert!(s.event(&Event::Wheel(-1), &v, now).is_empty(), "already the first sound");
    press(&mut s, &v, "BUTTON_SWING");
    assert_eq!(s.event(&Event::Wheel(5), &v, now), [Action::Swing(5.0)]);
    press(&mut s, &v, "BUTTON_VOLUME");
    // (Maschine steps the master by 0.5 dB, 0.05 with SHIFT.)
    assert_eq!(s.event(&Event::Wheel(-5), &v, now), [Action::VolumeDb(-2.5)]);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(s.event(&Event::Wheel(1), &v, now), [Action::VolumeDb(0.05)]);
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    press(&mut s, &v, "BUTTON_VOLUME");
    assert_eq!(s.event(&Event::Wheel(1), &v, now), [Action::FocusSound(1)], "pressed again: back to the sound");
}

#[test]
fn fixed_velocity_toggles() {
    let (mut s, mut v) = (Surface::new(), view());
    assert_eq!(press(&mut s, &v, "BUTTON_FIX_VELOCITY"), [Action::FixedVelocity(true)]);
    v.fixed_velocity = true;
    assert_eq!(press(&mut s, &v, "BUTTON_FIX_VELOCITY"), [Action::FixedVelocity(false)]);
}

/// A fresh, empty folder for a user's scripts (under the temporary folder), and the
/// guard that removes it.
fn user_dir() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    (tmp, dir)
}

#[test]
fn unhandled_events_reach_global() {
    // The Control page handles knobs, not PLAY: PLAY reaches global.lua.
    let (mut s, v) = (Surface::new(), view());
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, Instant::now()), [Action::Turn { knob: 0, inc: 0.1, fine: false }]);
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_PLAY"), [Action::Play]);
}

#[test]
fn a_user_page_replaces_only_its_page() {
    let (_tmp, dir) = user_dir();
    std::fs::write(
        dir.join("pages/control.lua"),
        "return { button = function(name, down) if down and name == 'PLAY' then deckard.restart() return true end end }",
    )
    .unwrap();
    let (mut s, v) = (Surface::with_user_dir(Some(dir.clone())), view());
    assert_eq!(s.error(), None);
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_PLAY"), [Action::Restart], "the user's page takes PLAY");
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_STOP"), [Action::Stop], "global.lua is still DECKARD's");
    assert!(s.event(&Event::Knob(0, 0.1), &v, Instant::now()).is_empty(), "the shipped page's knobs are gone");
}

#[test]
fn a_devices_own_file_comes_first() {
    let (_tmp, dir) = user_dir();
    std::fs::create_dir_all(dir.join("devices/mk3/pages")).unwrap();
    std::fs::write(dir.join("pages/control.lua"), "return { knob = function() deckard.play() return true end }").unwrap();
    std::fs::write(dir.join("devices/mk3/pages/control.lua"), "return { knob = function() deckard.stop() return true end }").unwrap();
    let (mut s, v) = (Surface::with_user_dir(Some(dir.clone())), view());
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, Instant::now()), [Action::Stop]);
    assert_eq!(s.eval::<String>("return deckard.device()").unwrap(), "mk3");
}

#[test]
fn a_broken_script_keeps_the_last_working_one() {
    let (_tmp, dir) = user_dir();
    let mut s = Surface::with_user_dir(Some(dir.clone()));
    let v = view();
    std::fs::write(dir.join("global.lua"), "return { this is not lua").unwrap();
    assert!(!s.reload(), "it doesn't load");
    let e = s.error().expect("the error is reported");
    assert!(e.contains("global.lua"), "it names the file: {e}");
    assert_eq!(press(&mut s, &v, "BUTTON_TRANSPORT_PLAY"), [Action::Play], "the last working scripts still run");

    // An error in a handler is reported too, and the next events still work.
    std::fs::write(dir.join("global.lua"), "return { button = function() error('oops') end }").unwrap();
    assert!(s.reload());
    assert_eq!(s.error(), None, "loaded: the old error goes");
    assert!(press(&mut s, &v, "BUTTON_TRANSPORT_PLAY").is_empty());
    assert!(s.error().is_some_and(|e| e.contains("oops")));
    assert_eq!(s.event(&Event::Knob(1, 0.1), &v, Instant::now()).len(), 1, "the page still runs");
}

#[test]
fn edited_user_scripts_reload() {
    let (_tmp, dir) = user_dir();
    let mut s = Surface::with_user_dir(Some(dir.clone()));
    let v = view();
    assert!(!s.reload(), "nothing changed");
    std::fs::write(dir.join("pages/control.lua"), "return { knob = function() deckard.metronome(true) return true end }").unwrap();
    let t0 = Instant::now();
    assert!(s.poll(t0));
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, t0), [Action::Metronome(true)]);
    std::fs::remove_file(dir.join("pages/control.lua")).unwrap();
    assert!(!s.poll(t0 + Duration::from_millis(100)), "checked at most twice a second");
    assert!(s.poll(t0 + Duration::from_secs(1)), "deleted: DECKARD's page again");
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, t0), [Action::Turn { knob: 0, inc: 0.1, fine: false }]);
}

#[test]
fn themes_switch() {
    let (mut s, v) = (Surface::new(), view());
    assert_eq!(s.themes(), ["maschine", "palenight", "solarized_dark"]);
    assert_eq!(s.theme(), "maschine");
    let (_, maschine) = s.screens(&v).unwrap();
    s.set_theme("solarized_dark");
    assert_eq!((s.theme(), s.error()), ("solarized_dark", None));
    let (_, solarized) = s.screens(&v).expect("a new picture");
    assert_eq!(solarized[480 * 200][..3], [0x00, 0x12, 0x17], "Solarized's background");
    assert_ne!(maschine[480 * 200], solarized[480 * 200]);
    s.set_theme("no_such_theme");
    assert_eq!(s.theme(), "solarized_dark", "a theme that doesn't load leaves the last");
}

#[test]
fn a_users_theme_is_listed() {
    let (_tmp, dir) = user_dir();
    std::fs::create_dir_all(dir.join("themes")).unwrap();
    std::fs::write(dir.join("themes/mine.lua"), crate::SHIPPED.iter().find(|(p, _)| *p == "themes/maschine.lua").unwrap().1).unwrap();
    let s = Surface::with_user_dir(Some(dir.clone()));
    assert_eq!(s.themes(), ["maschine", "mine", "palenight", "solarized_dark"]);
}

#[test]
fn a_script_error_shows_on_the_screen() {
    let (_tmp, dir) = user_dir();
    std::fs::write(dir.join("pages/control.lua"), "return { draw = function(screen) screen:text(0, 0, 'x', 'no_such_style') end }")
        .unwrap();
    let (mut s, v) = (Surface::with_user_dir(Some(dir.clone())), view());
    let (_, px) = s.screens(&v).unwrap();
    assert!(s.error().is_some_and(|e| e.contains("no_such_style")));
    assert_eq!(px[(HEIGHT_FOR_TEST - 5) * 960 + 470][..3], [0x8c, 0x1e, 0x1e], "the error bar");
}

const HEIGHT_FOR_TEST: usize = 272;

/// A library with three products, Type values and twelve results, the second
/// focused.
fn library(v: &mut View) {
    v.browser = BrowserView {
        kind: 6,
        products: vec!["Kit A".into(), "Kit B".into(), "Kit C".into()],
        attributes: [(vec!["Drums".into(), "Bass".into()], None), (Vec::new(), None), (Vec::new(), None)],
        results: (1..=12).map(|i| (format!("Kick {i}"), i == 3)).collect(),
        focus: 1,
        ..Default::default()
    };
}

#[test]
fn browse_shows_and_hides_its_page() {
    let (mut s, v) = (Surface::new(), view());
    press(&mut s, &v, "BUTTON_BROWSE");
    assert_eq!(s.page(), "browse");
    press(&mut s, &v, "BUTTON_BROWSE");
    assert_eq!(s.page(), "control", "pressed again, the page it was pressed on");
}

#[test]
fn browse_buttons() {
    let (mut s, mut v) = (Surface::new(), view());
    library(&mut v);
    press(&mut s, &v, "BUTTON_BROWSE");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_1"), [Action::Browse(Browse::Kind(5))], "Loops");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_2").is_empty(), "One-shots are the last");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_4"), [Action::Browse(Browse::User(true))]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_7"), [Action::Browse(Browse::Favorites)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Browse(Browse::Load)]);
    assert_eq!(
        press(&mut s, &v, "BUTTON_SCREEN_6"),
        [Action::Browse(Browse::Focus(1)), Action::Browse(Browse::Load)],
        "NEXT loads (prehear is off)"
    );
    v.browser.results.clear();
    assert!(press(&mut s, &v, "BUTTON_SCREEN_8").is_empty(), "nothing to load");
    v.browser.busy = true;
    assert!(press(&mut s, &v, "BUTTON_SCREEN_4").is_empty(), "not while searching");
}

#[test]
fn browse_knobs_gather_a_tenth_of_a_turn() {
    let (mut s, mut v) = (Surface::new(), view());
    library(&mut v);
    press(&mut s, &v, "BUTTON_BROWSE");
    let now = Instant::now();
    assert!(s.event(&Event::Knob(7, 0.05), &v, now).is_empty());
    assert_eq!(s.event(&Event::Knob(7, 0.05), &v, now), [Action::Browse(Browse::Focus(1))]);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(s.event(&Event::Knob(7, 0.3), &v, now), [Action::Browse(Browse::Focus(10))], "SHIFT: up to twenty, to the last");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.event(&Event::Knob(1, 0.1), &v, now), [Action::Browse(Browse::Product(Some(0)))], "from all: the first shown");
    assert_eq!(s.event(&Event::Knob(4, 0.1), &v, now), [Action::Browse(Browse::Attribute(0, Some(0)))]);
    v.browser.attributes[0].1 = Some(0);
    assert_eq!(s.event(&Event::Knob(4, -0.1), &v, now), [Action::Browse(Browse::Attribute(0, None))], "back to all");
}

#[test]
fn browse_wheel_turns_the_bracketed_knob() {
    let (mut s, mut v) = (Surface::new(), view());
    library(&mut v);
    press(&mut s, &v, "BUTTON_BROWSE");
    let now = Instant::now();
    assert_eq!(s.event(&Event::Wheel(1), &v, now), [Action::Browse(Browse::Focus(1))], "results to begin with");
    assert_eq!(press(&mut s, &v, "BUTTON_WHEEL"), [Action::Browse(Browse::Load)]);
    press(&mut s, &v, "BUTTON_WHEEL_LEFT");
    assert_eq!(
        s.event(&Event::Wheel(1), &v, now),
        [Action::Browse(Browse::Attribute(0, Some(0)))],
        "Type: sub-type and character are empty"
    );
    press(&mut s, &v, "BUTTON_WHEEL_LEFT");
    assert_eq!(s.event(&Event::Wheel(1), &v, now), [Action::Browse(Browse::Product(Some(0)))], "skipping the empty banks");
    press(&mut s, &v, "BUTTON_TEMPO");
    assert!(!s.event(&Event::Wheel(1), &v, now).iter().any(|a| matches!(a, Action::Browse(_))), "TEMPO keeps the wheel");
}

#[test]
fn browse_draws_a_library() {
    let (mut s, mut v) = (Surface::new(), view());
    library(&mut v);
    v.browser.product = Some(1);
    press(&mut s, &v, "BUTTON_BROWSE");
    assert!(s.screens(&v).is_some());
    assert_eq!(s.error(), None);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert!(s.screens(&v).is_some());
    assert_eq!(s.error(), None);
}

#[test]
fn every_theme_draws_every_page() {
    let (mut s, mut v) = (Surface::new(), view());
    library(&mut v);
    for theme in s.themes() {
        s.set_theme(&theme);
        assert_eq!(s.theme(), theme, "{theme} loads");
        for button in [
            "BUTTON_PAD_MODE",
            "BUTTON_KEYBOARD",
            "BUTTON_STEP",
            "BUTTON_MIX",
            "BUTTON_ARRANGE",
            "BUTTON_BROWSE",
            "BUTTON_PATTERN",
            "BUTTON_SCENE",
            "BUTTON_MUTE",
        ] {
            hold(&mut s, &v, button, true);
            s.picture(&v);
            assert_eq!(s.error(), None, "{theme}: {button}");
            hold(&mut s, &v, button, false);
        }
    }
}

/// A note running past the pattern editor's end is cut off at its edge, not drawn on
/// over the right screen (C16).
#[test]
fn long_notes_stay_in_the_pattern_editor() {
    let (mut s, mut v) = (Surface::new(), view());
    v.pattern_start = 0;
    v.pattern_length = 3840;
    hold(&mut s, &v, "BUTTON_PATTERN", true);
    let note = |length| NoteView { sound: 1, position: 1920, length, pitch: 60, velocity: 100, probability: 100, selected: false };
    v.notes = vec![note(1920)];
    let cut = s.picture(&v);
    v.notes = vec![note(30_000)];
    let long = s.picture(&v);
    let right = |px: &[[u8; 4]]| px.chunks(960).flat_map(|row| row[480..].to_vec()).collect::<Vec<_>>();
    assert!(right(&cut) == right(&long), "the right screen is the same");
    assert!(cut == long, "and the left");
}

/// Held from Keyboard mode, PATTERN shows the Pattern page's buttons, not the
/// keyboard's (C20).
#[test]
fn the_pattern_page_shows_its_own_buttons_from_keyboard_mode() {
    let (mut s, v) = (Surface::new(), view());
    press(&mut s, &v, "BUTTON_KEYBOARD");
    hold(&mut s, &v, "BUTTON_PATTERN", true);
    let text = s.screen_text(&v);
    assert!(text.contains(&"CREATE".to_string()), "{text:?}");
    assert!(!text.iter().any(|t| t.starts_with("OCTAVE") || t.starts_with("SEMITONE")), "{text:?}");
    // And they act as the Pattern page's.
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::PatternEdit("create")]);
}

/// Three scenes, two sections (the second 8 bars long), the first focused.
fn song(v: &mut View) {
    v.scenes = ["Intro", "Verse", "Chorus"]
        .iter()
        .enumerate()
        .map(|(i, n)| SceneView {
            name: n.to_string(),
            color: i,
            default_color: false,
            patterns: vec![Some(0), None],
            ..Default::default()
        })
        .collect();
    v.groups[0].patterns = vec![PatternView { name: "Beat".into(), color: 1, default_color: true, bars: 4.0, ..Default::default() }];
    v.sections = vec![
        SectionView { scene: Some(0), start: 0, length: 4 * 3840, auto: true },
        SectionView { scene: Some(1), start: 4 * 3840, length: 8 * 3840, auto: false },
    ];
    v.section = Some(0);
}

/// SHIFT on the Ideas view: IDEAS / SONG, and the focused scene's edits; after one,
/// the scenes are back on the buttons until SHIFT is let go (C1).
#[test]
fn shift_on_the_ideas_view_edits_scenes() {
    let (mut s, mut v) = (Surface::new(), view());
    song(&mut v);
    press(&mut s, &v, "BUTTON_ARRANGE");
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    let text = s.screen_text(&v);
    assert!(["IDEAS", "SONG", "CLEAR", "DUPLICATE", "CREATE", "DELETE"].iter().all(|t| text.contains(&t.to_string())), "{text:?}");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_2"), [Action::SongView(true)]);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_1").is_empty(), "already the Ideas view");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_3"), [Action::SceneEdit("clear")]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::SceneEdit("insert")]);
    // The edit made: the scenes again.
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_2"), [Action::FocusScene(1)]);
    assert!(!s.screen_text(&v).contains(&"CLEAR".to_string()));
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_4"), [Action::SceneEdit("duplicate")]);
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::SceneEdit("delete")]);
}

/// The Song view: the sections, their edits, knobs and pads (C1).
#[test]
fn the_song_view_edits_sections() {
    let (mut s, mut v) = (Surface::new(), view());
    song(&mut v);
    v.song_view = true;
    press(&mut s, &v, "BUTTON_ARRANGE");
    for theme in s.themes() {
        s.set_theme(&theme);
        assert!(s.screens(&v).is_some());
        assert_eq!(s.error(), None, "{theme}");
    }
    let text = s.screen_text(&v);
    assert!(
        [
            "SECTION",
            "CLIP",
            "CONVERT",
            "DUPLICATE",
            "CREATE",
            "DELETE",
            "BANK 1",
            "Section",
            "POSITION",
            "#1",
            "SCENE",
            "Intro",
            "LENGTH",
            "Auto",
            "A1"
        ]
        .iter()
        .all(|t| text.contains(&t.to_string())),
        "{text:?}"
    );
    assert!(!s.pads(&v).play, "the pads are the sections");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_4"), [Action::SectionEdit("duplicate")]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::SectionEdit("create")]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::SectionEdit("delete")]);
    assert!(press(&mut s, &v, "BUTTON_LEFT").is_empty(), "the first");
    assert_eq!(press(&mut s, &v, "BUTTON_RIGHT"), [Action::FocusSection(1)]);
    let now = Instant::now();
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::SectionMove(true)]);
    assert_eq!(s.event(&Event::Knob(1, 0.1), &v, now), [Action::SectionScene(true)]);
    assert_eq!(s.event(&Event::Knob(3, 0.1), &v, now), [Action::SectionLength(5 * 3840)], "a bar on");
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(s.event(&Event::Knob(3, -0.1), &v, now), [Action::SectionLength(4 * 3840 - 960)], "SHIFT: a beat");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_1"), [Action::SongView(false)]);
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.event(&Event::PadOn(2, 0.8), &v, now), [Action::FocusSection(1)]);
    assert_eq!(s.event(&Event::PadOn(5, 0.8), &v, now), [Action::SceneEdit("append")], "an empty pad: a new section");
    hold(&mut s, &v, "BUTTON_ERASE", true);
    assert_eq!(s.event(&Event::PadOn(2, 0.8), &v, now), [Action::FocusSection(1), Action::SectionEdit("delete")]);
}

/// The strip shows where playback is, as Maschine's `updateLEDsPlayPositionMode`: the
/// focused group's pattern in its colour on the Ideas view, the song in white on the
/// Song view; `round(pos × 24) + 1` lights (C5).
#[test]
fn the_strip_follows_playback() {
    let (s, mut v) = (Surface::new(), view());
    song(&mut v);
    let lit = |s: &Surface, v: &View| (1..=25).filter(|i| s.leds(v).get(&format!("LED_TS_{i:02}")).unwrap() != 0).count();
    assert_eq!(lit(&s, &v), 0, "nothing playing");
    v.playing = true;
    v.groups[0].scene_pattern = Some(0);
    v.progress = Some(0.039);
    assert_eq!(lit(&s, &v), 2, "round(0.94) + 1");
    assert_eq!(s.leds(&v).get("LED_TS_01"), Some(deckard_mk3::leds::palette(1, 4)), "the pattern's colour");
    v.song_view = true;
    v.position = Some(3 * 3840);
    assert_eq!(lit(&s, &v), 7, "a quarter through the song: round(6) + 1");
    assert_eq!(s.leds(&v).get("LED_TS_01"), Some(deckard_mk3::leds::palette(16, 4)), "white");
}

/// While the song plays, the Song view draws the playhead on both screens (C1).
#[test]
fn the_song_view_draws_the_playhead() {
    let (mut s, mut v) = (Surface::new(), view());
    song(&mut v);
    v.song_view = true;
    press(&mut s, &v, "BUTTON_ARRANGE");
    v.playing = true;
    v.position = Some(6 * 3840);
    let px = s.picture(&v);
    let white = |x: usize, y: usize| px[y * 960 + x][..3] == [255, 255, 255];
    // Half the 12-bar song on the left; 6 bars at 12.5 px a bar on the right.
    assert!(white(240, 150), "the overview");
    assert!(white(507 + 75, 230), "the sections");
}

/// Display button 1 pins a held page (SCENE, PATTERN, MUTE…) so it stays when the
/// button's let go, as Maschine's `Page:togglePinState`; the pin stays for next time
/// (C6).
#[test]
fn held_pages_pin() {
    let (mut s, mut v) = (Surface::new(), view());
    song(&mut v);
    let now = Instant::now();
    hold(&mut s, &v, "BUTTON_SCENE", true);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_1").is_empty());
    hold(&mut s, &v, "BUTTON_SCENE", false);
    assert_eq!(s.page(), "scene", "pinned");
    assert!(!s.pads(&v).play, "the pads still pick scenes");
    assert_eq!(s.event(&Event::PadOn(2, 0.8), &v, now), [Action::FocusScene(1)]);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_1").is_empty());
    assert_eq!(s.page(), "control", "unpinned with SCENE up: closed");

    // Pinned, SCENE pressed again closes it as it's let go.
    hold(&mut s, &v, "BUTTON_SCENE", true);
    press(&mut s, &v, "BUTTON_SCREEN_1");
    hold(&mut s, &v, "BUTTON_SCENE", false);
    hold(&mut s, &v, "BUTTON_SCENE", true);
    assert_eq!(s.page(), "scene");
    hold(&mut s, &v, "BUTTON_SCENE", false);
    assert_eq!(s.page(), "control");
    // The pin stays: next time the page stays.
    hold(&mut s, &v, "BUTTON_SCENE", true);
    hold(&mut s, &v, "BUTTON_SCENE", false);
    assert_eq!(s.page(), "scene");
    // A mode button leaves it.
    press(&mut s, &v, "BUTTON_MIX");
    assert_eq!(s.page(), "mixer");
    press(&mut s, &v, "BUTTON_MIX");
    assert_eq!(s.page(), "control");

    // MUTE pinned: the pads mute.
    hold(&mut s, &v, "BUTTON_MUTE", true);
    press(&mut s, &v, "BUTTON_SCREEN_1");
    hold(&mut s, &v, "BUTTON_MUTE", false);
    assert_eq!(s.page(), "hold");
    assert_eq!(s.event(&Event::PadOn(2, 0.8), &v, now), [Action::MuteSound(1)]);
}

/// The info bar shows what the jog wheel just changed, as Maschine's quick edit: the
/// value, then its name.
#[test]
fn the_info_bar_shows_a_wheel_change() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_SWING");
    assert!(!s.screen_text(&v).contains(&"MASTER SWING".to_string()), "not before a turn");
    s.event(&Event::Wheel(3), &v, now);
    v.swing = 3.0;
    let text = s.screen_text(&v);
    assert!(text.contains(&"3.0 %".to_string()) && text.contains(&"MASTER SWING".to_string()), "{text:?}");
    press(&mut s, &v, "BUTTON_VOLUME");
    s.event(&Event::Wheel(-3), &v, now);
    v.master_db = -1.5;
    let text = s.screen_text(&v);
    assert!(text.contains(&"-1.5 dB".to_string()) && text.contains(&"MASTER VOL".to_string()), "{text:?}");
    press(&mut s, &v, "BUTTON_TEMPO");
    s.event(&Event::Wheel(3), &v, now);
    v.tempo = 123.0;
    assert!(s.screen_text(&v).contains(&"123.00 BPM".to_string()));
    assert!(s.screen_text(&v).contains(&"123.00 BPM".to_string()), "it stays while TEMPO is on");
    press(&mut s, &v, "BUTTON_TEMPO");
    assert!(!s.screen_text(&v).contains(&"123.00 BPM".to_string()), "TEMPO off: the tempo and scene again");
    press(&mut s, &v, "BUTTON_TEMPO");
    assert!(!s.screen_text(&v).contains(&"123.00 BPM".to_string()), "on again: not before a turn");
}

#[test]
fn the_pad_mode_page_turns_its_second_page() {
    let (mut s, mut v) = (Surface::new(), view());
    v.velocity = 100;
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_PAD_MODE");
    assert!(press(&mut s, &v, "BUTTON_LEFT").is_empty(), "no page before the first");
    assert_eq!(s.event(&Event::Knob(0, 0.05), &v, now), [Action::SoundSetting("choke_group", 1)]);
    assert!(press(&mut s, &v, "BUTTON_RIGHT").is_empty());
    assert!(s.event(&Event::Knob(0, 0.05), &v, now).is_empty(), "a colour gathers a tenth of a turn");
    assert_eq!(s.event(&Event::Knob(0, 0.05), &v, now), [Action::Color { of: ColorOf::Group, color: Some(2) }]);
    assert_eq!(s.event(&Event::Knob(1, -0.1), &v, now), [Action::Color { of: ColorOf::Sound, color: None }]);
    assert_eq!(s.event(&Event::Knob(2, 0.01), &v, now), [Action::VelocityValue(101)]);
    assert!(s.event(&Event::Knob(3, 0.1), &v, now).is_empty(), "Focus is the layer's own");
    assert!(s.event(&Event::PadOn(5, 0.5), &v, now).is_empty(), "a pinned sound keeps the focus");
    assert!(press(&mut s, &v, "BUTTON_RIGHT").is_empty(), "two pages");
    press(&mut s, &v, "BUTTON_LEFT");
    assert_eq!(s.event(&Event::Knob(0, 0.05), &v, now), [Action::SoundSetting("choke_group", 1)]);
}

#[test]
fn keyboard_mode_plays_the_scale() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_KEYBOARD");
    s.event(&Event::Knob(1, 0.05), &v, now);
    assert_eq!(s.event(&Event::Knob(1, 0.05), &v, now), [Action::Scale { root: 60, bank: 0, kind: 1 }]);
    v.scale_kind = 1;
    let keys = s.pads(&v).keys.unwrap().1;
    assert_eq!(keys[..8], [60, 62, 64, 65, 67, 69, 71, 72], "Major");
    s.event(&Event::Knob(0, 0.05), &v, now);
    assert_eq!(s.event(&Event::Knob(0, 0.05), &v, now), [Action::Scale { root: 60, bank: 1, kind: 0 }], "a new bank starts at its first");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Scale { root: 61, bank: 0, kind: 1 }]);
}

#[test]
fn keyboard_mode_pages_through_velocity_and_pad_touch() {
    let (mut s, mut v) = (Surface::new(), view());
    v.velocity = 127;
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_KEYBOARD");
    press(&mut s, &v, "BUTTON_RIGHT");
    assert_eq!(s.event(&Event::Knob(0, -0.01), &v, now), [Action::VelocityValue(126)]);
    press(&mut s, &v, "BUTTON_RIGHT");
    s.event(&Event::Knob(0, 0.05), &v, now);
    assert_eq!(s.event(&Event::Knob(0, 0.05), &v, now), [Action::PadTouch("mode", 1)]);
    assert_eq!(s.event(&Event::Knob(1, 0.01), &v, now), [Action::PadTouch("curve", 51)]);
    assert!(s.event(&Event::Knob(2, -0.01), &v, now).is_empty(), "MIN stays at 0");
    press(&mut s, &v, "BUTTON_RIGHT");
    assert_eq!(s.event(&Event::Knob(0, 0.01), &v, now), [Action::PadTouch("cc", 2)]);
    assert!(press(&mut s, &v, "BUTTON_RIGHT").is_empty(), "four pages");
    assert_eq!(s.event(&Event::Knob(0, 0.01), &v, now), [Action::PadTouch("cc", 2)]);
}

#[test]
fn chords_mode_has_two_pages() {
    let (mut s, mut v) = (Surface::new(), view());
    v.velocity = 100;
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_CHORDS");
    press(&mut s, &v, "BUTTON_RIGHT");
    assert_eq!(s.event(&Event::Knob(1, 0.01), &v, now), [Action::VelocityValue(101)]);
    press(&mut s, &v, "BUTTON_RIGHT");
    assert_eq!(s.event(&Event::Knob(1, 0.01), &v, now), [Action::VelocityValue(101)], "still page 2");
}

fn no_user_chords(v: &mut View) {
    v.user_chords = vec![vec![Vec::new(); 12]; 16];
}

#[test]
fn user_chd_edits_the_selected_user_chord() {
    let (mut s, mut v) = (Surface::new(), view());
    no_user_chords(&mut v);
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_CHORDS");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_4").is_empty(), "to Keyboard mode, editing");
    assert_eq!(s.event(&Event::PadOn(3, 0.5), &v, now), [Action::UserChordNote { set: 0, slot: 0, note: 2 }]);
    v.user_chords[0][0] = vec![0, 4, 7];
    let pads = s.pads(&v);
    assert_eq!(pads.keys.unwrap().1[4], 64, "the pads play single notes while editing");
    assert!(pads.chords.is_none());
    assert_eq!(hold(&mut s, &v, "BUTTON_SCREEN_3", true), [Action::UserChordPreview { set: 0, slot: 0, root: 60, on: true }]);
    assert_eq!(hold(&mut s, &v, "BUTTON_SCREEN_3", false), [Action::UserChordPreview { set: 0, slot: 0, root: 60, on: false }]);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_4").is_empty(), "back to Chords mode");
    let chords = s.pads(&v).chords.expect("User Chord mode plays chords");
    assert_eq!(chords[0][..4], [60, 64, 67, 128]);
    assert_eq!(chords[1][0], 128, "an empty chord plays nothing");
    assert_eq!(chords[12][0], 128, "pads 13-16 play nothing");
}

#[test]
fn keyboards_user_chd_starts_editing() {
    let (mut s, mut v) = (Surface::new(), view());
    no_user_chords(&mut v);
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_KEYBOARD");
    assert!(s.event(&Event::PadOn(1, 0.5), &v, now).is_empty(), "not editing yet");
    press(&mut s, &v, "BUTTON_SCREEN_4");
    assert_eq!(s.event(&Event::PadOn(1, 0.5), &v, now), [Action::UserChordNote { set: 0, slot: 0, note: 0 }]);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_6").is_empty(), "the octave buttons move the pads, not the root");
    assert_eq!(s.pads(&v).keys.unwrap().1[0], 72);
    assert_eq!(s.event(&Event::PadOn(1, 0.5), &v, now), [Action::UserChordNote { set: 0, slot: 0, note: 12 }]);
    press(&mut s, &v, "BUTTON_PAD_MODE");
    press(&mut s, &v, "BUTTON_KEYBOARD");
    assert_eq!(s.pads(&v).keys.unwrap().1[0], 60, "editing ends with the page");
}

#[test]
fn shift_deletes_and_moves_user_chords() {
    let (mut s, mut v) = (Surface::new(), view());
    no_user_chords(&mut v);
    v.user_chords[0][1] = vec![0, 3, 7];
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_CHORDS");
    s.event(&Event::Knob(2, 0.1), &v, now);
    s.event(&Event::Knob(2, 0.1), &v, now);
    assert!(s.pads(&v).chords.is_some(), "User Chord mode");
    assert!(s.event(&Event::PadOn(2, 0.5), &v, now).is_empty(), "a pad picks its chord");
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_6").is_empty());
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::UserChordMove { set: 0, slot: 1, next: true }]);
    v.user_chords[0].swap(1, 2);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::UserChordDelete { set: 0, slot: 2 }]);
    v.user_chords[0][2].clear();
    // (SHIFT + a pad is its shortcut, as Maschine's: the slot's picked without SHIFT.)
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    s.event(&Event::PadOn(1, 0.5), &v, now);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_7").is_empty(), "no slot before the first");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Scale { root: 61, bank: 0, kind: 0 }]);
}

#[test]
fn follow_toggles_following_playback() {
    let (mut s, mut v) = (Surface::new(), view());
    v.follow = false;
    assert_eq!(press(&mut s, &v, "BUTTON_FOLLOW"), [Action::Follow(true)]);
    v.follow = true;
    assert_eq!(press(&mut s, &v, "BUTTON_FOLLOW"), [Action::Follow(false)]);
}

fn grid_view() -> View {
    let mut v = view();
    v.grid = GridView { perform: 5, arrange: 0, arrange_snap: true, quick: true, step: 4, step_snap: true, step_ticks: 240, nudge: 1 };
    v
}

#[test]
fn shift_follow_shows_the_grid_while_follow_is_held() {
    let (mut s, v) = (Surface::new(), grid_view());
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert!(hold(&mut s, &v, "BUTTON_FOLLOW", true).is_empty(), "no Follow toggle");
    assert_eq!(s.page(), "grid");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), "grid", "FOLLOW still held");
    let leds = s.leds(&v);
    assert_eq!(leds.get("LED_SCREEN_BUTTON_2"), Some(crate::leds::ON), "PERFORM");
    assert_eq!(leds.get("LED_SCREEN_BUTTON_7"), Some(deckard_mk3::leds::BACKLIGHT), "only STEP's");
    assert_eq!(leds.get("LED_PAD_2"), Some(deckard_mk3::leds::palette(16, 4)), "1/16, the second column's first");
    assert_eq!(leds.get("LED_PAD_15"), Some(0), "PERFORM has six values");
    assert!(hold(&mut s, &v, "BUTTON_FOLLOW", false).is_empty());
    assert_eq!(s.page(), "control");
}

#[test]
fn display_button_1_pins_the_grid() {
    let (mut s, v) = (Surface::new(), grid_view());
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    hold(&mut s, &v, "BUTTON_FOLLOW", true);
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    press(&mut s, &v, "BUTTON_SCREEN_1");
    hold(&mut s, &v, "BUTTON_FOLLOW", false);
    assert_eq!(s.page(), "grid", "pinned");
    assert_eq!(s.leds(&v).get("LED_SCREEN_BUTTON_1"), Some(crate::leds::ON));
    press(&mut s, &v, "BUTTON_SCREEN_1");
    assert_eq!(s.page(), "control", "unpinned, FOLLOW up");
}

#[test]
fn the_grid_pads_pick_a_value() {
    let (mut s, mut v) = (Surface::new(), grid_view());
    let now = Instant::now();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    hold(&mut s, &v, "BUTTON_FOLLOW", true);
    assert_eq!(s.event(&Event::PadOn(5, 0.8), &v, now), [Action::Grid("perform", 2)], "1/2");
    assert!(s.event(&Event::PadOn(15, 0.8), &v, now).is_empty(), "nothing there");
    assert_eq!(s.event(&Event::PadOn(16, 0.8), &v, now), [Action::Grid("perform", 0)], "OFF");
    v.grid.perform = 0;
    assert_eq!(s.event(&Event::PadOn(16, 0.8), &v, now), [Action::Grid("perform", 5)], "back on, as it was");

    press(&mut s, &v, "BUTTON_SCREEN_3");
    assert_eq!(s.event(&Event::PadOn(5, 0.8), &v, now), [Action::Grid("arrange", 1)]);
    v.grid.arrange_snap = false;
    assert_eq!(s.event(&Event::PadOn(1, 0.8), &v, now), [Action::Grid("arrange", 0), Action::Grid("arrange_snap", 1)]);
    assert!(s.event(&Event::Knob(3, 0.2), &v, now).is_empty(), "no QUICK with the grid off");
    v.grid.arrange_snap = true;
    assert_eq!(s.event(&Event::Knob(3, -0.2), &v, now), [Action::Grid("quick", 0)]);

    press(&mut s, &v, "BUTTON_SCREEN_4");
    assert_eq!(s.event(&Event::PadOn(3, 0.8), &v, now), [Action::Grid("step", 8)], "1 Bar T");
    assert_eq!(s.event(&Event::PadOn(16, 0.8), &v, now), [Action::Grid("step_snap", 0)]);
    assert_eq!(s.event(&Event::Knob(3, 0.2), &v, now), [Action::Grid("nudge", 2)]);
}

#[test]
fn the_grid_cycle_buttons_step_down_a_column() {
    let (mut s, mut v) = (Surface::new(), grid_view());
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    hold(&mut s, &v, "BUTTON_FOLLOW", true);
    // PERFORM: 1/16 and Section take turns on button 6; 7 and 8 turn it off.
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::Grid("perform", 6)]);
    v.grid.perform = 6;
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::Grid("perform", 5)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::Grid("perform", 4)], "the column's top, from elsewhere");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Grid("perform", 0)]);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_7").is_empty(), "only STEP's");
    press(&mut s, &v, "BUTTON_SCREEN_4");
    v.grid.step = 1;
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::Grid("step", 0)], "1/2 to 1 Bar");
    v.grid.step = 0;
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::Grid("step", 3)], "round to 1/8");
    v.grid.step = 12;
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Grid("step_snap", 0)], "1/16T down to OFF");
}

#[test]
fn macro_shows_the_channels_macros() {
    let (mut s, v) = (Surface::new(), view());
    assert_eq!(press(&mut s, &v, "BUTTON_MACRO"), [Action::ModulesVisible(false), Action::PageGroup(3)]);
    assert_eq!(s.page(), "control");
}

#[test]
fn file_shows_the_file_page() {
    let (mut s, v) = (Surface::new(), view());
    press(&mut s, &v, "BUTTON_FILE");
    assert_eq!(s.page(), "file");
    let leds = s.leds(&v);
    assert_eq!((leds.get("LED_FILE"), leds.get("LED_SCREEN_BUTTON_1")), (Some(crate::leds::ON), Some(crate::leds::ON)));
    assert_eq!(leds.get("LED_SCREEN_BUTTON_8"), Some(deckard_mk3::leds::BACKLIGHT), "no recent projects");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_2"), [Action::File("new", 0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_4"), [Action::File("save_copy", 0)]);
    assert_eq!(s.page(), "control", "back");
    press(&mut s, &v, "BUTTON_FILE");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::File("export_audio", 0)]);
    assert_eq!(s.page(), "control");
    press(&mut s, &v, "BUTTON_FILE");
    press(&mut s, &v, "BUTTON_FILE");
    assert_eq!(s.page(), "control", "FILE again: back");
}

#[test]
fn the_file_page_loads_a_recent_project() {
    let (mut s, mut v) = (Surface::new(), view());
    v.recent = vec!["Song A".into(), "Song B".into(), "Song C".into()];
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_FILE");
    assert!(s.event(&Event::Wheel(1), &v, now).is_empty());
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::File("load", 1)]);
    assert_eq!(s.page(), "file", "still");
    s.event(&Event::Knob(7, 0.05), &v, now);
    s.event(&Event::Knob(7, 0.05), &v, now);
    assert_eq!(press(&mut s, &v, "BUTTON_WHEEL"), [Action::File("load", 2)], "knob 8 too; the wheel's push loads");
}

#[test]
fn the_save_page_saves_the_project_a_group_or_a_sound() {
    let (mut s, v) = (Surface::new(), view());
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_FILE");
    press(&mut s, &v, "BUTTON_SCREEN_3");
    assert_eq!(s.page(), "save");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::File("save", 0)]);
    assert_eq!(s.page(), "control");
    press(&mut s, &v, "BUTTON_FILE");
    press(&mut s, &v, "BUTTON_SCREEN_3");
    s.event(&Event::Knob(0, 0.1), &v, now);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::File("save_group", 0)], "CONTENT TYPE Group");
    press(&mut s, &v, "BUTTON_FILE");
    press(&mut s, &v, "BUTTON_SCREEN_3");
    assert_eq!(s.event(&Event::PadOn(2, 0.8), &v, now), [Action::FocusSound(1)], "a pad: Sound");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::File("save_sound", 0)]);
    press(&mut s, &v, "BUTTON_FILE");
    press(&mut s, &v, "BUTTON_SCREEN_3");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_5").is_empty(), "CANCEL");
    assert_eq!(s.page(), "file");
}

/// Whether `led` is lit (not off).
fn lit(s: &Surface, v: &View, led: &str) -> bool {
    s.leds(v).get(led).is_some_and(|c| c != deckard_mk3::leds::BACKLIGHT && c != 0)
}

#[test]
fn settings_shows_the_settings_tabs() {
    let (mut s, v) = (Surface::new(), view());
    press(&mut s, &v, "BUTTON_SETTINGS");
    assert_eq!(s.page(), "settings");
    let leds = s.leds(&v);
    assert_eq!((leds.get("LED_SETTINGS"), leds.get("LED_SCREEN_BUTTON_1")), (Some(crate::leds::ON), Some(crate::leds::ON)));
    // GENERAL, the first of GENERAL, AUDIO, MIDI and HARDWARE; three knob pages.
    assert!(!lit(&s, &v, "LED_SCREEN_BUTTON_5") && lit(&s, &v, "LED_SCREEN_BUTTON_6"));
    assert!(!lit(&s, &v, "LED_WHEEL_BUTTON_LEFT") && lit(&s, &v, "LED_WHEEL_BUTTON_RIGHT"));
    assert!(!lit(&s, &v, "LED_LEFT") && lit(&s, &v, "LED_RIGHT"));
    press(&mut s, &v, "BUTTON_SCREEN_6");
    assert!(lit(&s, &v, "LED_SCREEN_BUTTON_5") && lit(&s, &v, "LED_SCREEN_BUTTON_6"), "AUDIO");
    press(&mut s, &v, "BUTTON_WHEEL_RIGHT");
    press(&mut s, &v, "BUTTON_WHEEL_RIGHT");
    assert!(!lit(&s, &v, "LED_SCREEN_BUTTON_6") && !lit(&s, &v, "LED_WHEEL_BUTTON_RIGHT"), "HARDWARE, the last");
    assert!(!lit(&s, &v, "LED_RIGHT"), "one knob page");
    press(&mut s, &v, "BUTTON_SCREEN_5");
    assert!(lit(&s, &v, "LED_RIGHT"), "MIDI: two knob pages");
    press(&mut s, &v, "BUTTON_SETTINGS");
    assert_eq!(s.page(), "control", "SETTINGS again: back");
}

#[test]
fn the_settings_knobs_change_the_settings() {
    let (mut s, v) = (Surface::new(), view());
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_SETTINGS");
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::Metronome(true)], "ENABLED");
    assert_eq!(s.event(&Event::Knob(4, 0.1), &v, now), [Action::Setting("count_in", 0, 1)]);
    assert_eq!(s.event(&Event::Knob(5, -0.1), &v, now), [Action::Setting("numerator", 0, -1)]);
    assert_eq!(s.event(&Event::Knob(6, 0.1), &v, now), [Action::Setting("denominator", 0, 1)]);
    assert_eq!(s.event(&Event::Knob(1, 0.1), &v, now), [Action::Setting("metronome_volume", 0, 1)], "VOLUME");
    assert_eq!(s.event(&Event::Knob(3, 0.1), &v, now), [Action::Setting("metronome_auto", 0, 1)], "AUTOENABLE");
    assert_eq!(s.event(&Event::Knob(2, 0.1), &v, now), [Action::Setting("metronome_ticks", 0, 1)], "TIMESIGNATURE");
    press(&mut s, &v, "BUTTON_RIGHT");
    assert_eq!(s.event(&Event::Knob(2, 0.1), &v, now), [Action::Setting("quantize", 0, 1)]);
    for (k, key) in
        [(0, "reload_last"), (1, "auto_backup"), (4, "duplicate"), (5, "link_duplicates"), (6, "pattern_length"), (7, "pattern_grow")]
    {
        assert_eq!(s.event(&Event::Knob(k, 0.1), &v, now), [Action::Setting(key, 0, 1)]);
    }
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(s.event(&Event::Knob(6, -0.1), &v, now), [Action::Setting("pattern_length_fine", 0, -1)], "LENGTH by a sixteenth");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    // AUDIO: the device and buffer size; the routings a page each four channels.
    press(&mut s, &v, "BUTTON_SCREEN_6");
    assert_eq!(s.event(&Event::Knob(3, 0.1), &v, now), [Action::Setting("buffer_size", 0, 1)], "back on its first page");
    press(&mut s, &v, "BUTTON_RIGHT");
    assert_eq!(s.event(&Event::Knob(2, 0.1), &v, now), [Action::Setting("in_route", 2, 1)], "IN 2L");
    // MIDI: Sync and the devices switched on and off.
    press(&mut s, &v, "BUTTON_SCREEN_6");
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::Setting("midi_sync", 0, 1)]);
    assert_eq!(s.event(&Event::Knob(2, -0.1), &v, now), [Action::Setting("default_input", 0, -1)]);
    // HARDWARE: a step for each turn on the levels.
    press(&mut s, &v, "BUTTON_SCREEN_6");
    assert_eq!(s.event(&Event::Knob(0, 0.01), &v, now), [Action::Setting("sensitivity", 0, 1)]);
    assert_eq!(s.event(&Event::Knob(4, -0.01), &v, now), [Action::Setting("display_brightness", 0, -1)]);
    assert_eq!(s.event(&Event::Knob(1, 0.1), &v, now), [Action::Setting("velocity_scaling", 0, 1)], "SCALING");
    assert_eq!(s.event(&Event::Knob(2, -0.01), &v, now), [Action::Setting("led_brightness", 0, -1)], "LEDs BRIGHTNESS");
    assert_eq!(s.event(&Event::Knob(3, -0.1), &v, now), [Action::Setting("strip_position", 0, -1)], "SHOW POSITION");
    assert_eq!(s.event(&Event::Knob(6, 0.1), &v, now), [Action::Setting("touch_auto_write", 0, 1)], "AUTO-WRITE");
}

#[test]
fn the_midi_settings_pick_a_device_and_switch_it() {
    let (mut s, mut v) = (Surface::new(), view());
    v.settings.midi_inputs = vec![("Keys".into(), true), ("Pads".into(), false)];
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_SETTINGS");
    press(&mut s, &v, "BUTTON_SCREEN_6");
    press(&mut s, &v, "BUTTON_SCREEN_6");
    assert_eq!(s.event(&Event::Knob(5, 0.1), &v, now), [Action::Setting("midi_in", 0, 1)], "Keys");
    assert!(s.event(&Event::Knob(4, 0.1), &v, now).is_empty(), "SELECT: Pads");
    assert_eq!(s.event(&Event::Knob(5, -0.1), &v, now), [Action::Setting("midi_in", 1, -1)]);
    // Page 2: Link.
    press(&mut s, &v, "BUTTON_RIGHT");
    assert_eq!(s.event(&Event::Knob(4, 0.1), &v, now), [Action::Setting("link", 0, 1)]);
    assert_eq!(s.event(&Event::Knob(5, 0.1), &v, now), [Action::Setting("link_start_stop", 0, 1)]);
}

#[test]
fn shift_macro_sets_macros_by_touching_the_knobs() {
    let (mut s, mut v) = (Surface::new(), view());
    v.control.level = 2;
    let now = Instant::now();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_MACRO");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), "macro_set");
    let leds = s.leds(&v);
    assert_eq!(
        (leds.get("LED_MACRO"), leds.get("LED_SCREEN_BUTTON_1"), leds.get("LED_SCREEN_BUTTON_4")),
        (Some(crate::leds::ON), Some(crate::leds::ON), Some(crate::leds::ON)),
        "SOUND: the level shown"
    );
    assert_eq!(leds.get("LED_SCREEN_BUTTON_2"), Some(deckard_mk3::leds::DIM));
    assert_eq!(hold(&mut s, &v, "BUTTON_CAP_3", true), [Action::SetMacro { knob: 2, level: 2 }], "a touch sets or clears");
    assert!(hold(&mut s, &v, "BUTTON_CAP_3", false).is_empty());
    press(&mut s, &v, "BUTTON_SCREEN_2");
    assert_eq!(hold(&mut s, &v, "BUTTON_CAP_1", true), [Action::SetMacro { knob: 0, level: 0 }], "MASTER");
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now).len(), 1, "the knobs turn as ever");
    // MACRO leaves Set Macro, back where it came from (not to the channel's macros).
    assert!(press(&mut s, &v, "BUTTON_MACRO").is_empty());
    assert_eq!(s.page(), "control");
    // SHIFT + MACRO again too.
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_MACRO");
    assert_eq!(s.page(), "macro_set");
    assert!(press(&mut s, &v, "BUTTON_MACRO").is_empty());
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), "control");
}

/// Set Macro keeps LEFT / RIGHT turning the pages (and the page shown), so knobs on
/// several pages can be made macros before leaving it.
#[test]
fn set_macro_turns_the_pages() {
    let (mut s, v) = (Surface::new(), view());
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_MACRO");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), "macro_set");
    let leds = s.leds(&v);
    assert_eq!((leds.get("LED_LEFT"), leds.get("LED_RIGHT")), (Some(deckard_mk3::leds::BACKLIGHT), Some(deckard_mk3::leds::DIM)));
    assert_eq!(press(&mut s, &v, "BUTTON_RIGHT"), [Action::Page(true)]);
    assert_eq!(s.page(), "macro_set", "still setting macros");
    assert!(s.screen_text(&v).contains(&"1/2".to_string()), "the page shown");
}

#[test]
fn lock_takes_and_recalls_the_focused_snapshot() {
    let (mut s, mut v) = (Surface::new(), view());
    assert_eq!(press(&mut s, &v, "BUTTON_LOCK"), [Action::Lock("button", 0)]);
    assert_eq!(s.page(), "control", "no page");
    assert_eq!(s.leds(&v).get("LED_LOCK"), Some(deckard_mk3::leds::BACKLIGHT));
    v.lock.active = true;
    assert_eq!(s.leds(&v).get("LED_LOCK"), Some(deckard_mk3::leds::DIM), "dim while locked");
}

#[test]
fn shift_lock_shows_the_snapshots() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert!(press(&mut s, &v, "BUTTON_LOCK").is_empty());
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), "lock");
    let leds = s.leds(&v);
    assert_eq!((leds.get("LED_LOCK"), leds.get("LED_SCREEN_BUTTON_1")), (Some(crate::leds::ON), Some(crate::leds::ON)));
    assert_eq!(leds.get("LED_SCREEN_BUTTON_5"), Some(deckard_mk3::leds::BACKLIGHT), "UPDATE: no snapshot focused");
    assert_eq!(leds.get("LED_SCREEN_BUTTON_8"), Some(deckard_mk3::leds::DIM), "the next bank");
    assert_eq!(s.event(&Event::PadOn(1, 0.8), &v, now), [Action::Lock("pad", 0)], "a pad takes or recalls");
    assert!(!s.pads(&v).play, "and plays nothing");
    v.lock.slots = vec![0, 1];
    v.lock.focus = Some(1);
    let leds = s.leds(&v);
    assert_eq!(leds.get("LED_PAD_1"), Some(deckard_mk3::leds::palette(16, 2)), "a snapshot");
    assert_eq!(leds.get("LED_PAD_2"), Some(deckard_mk3::leds::palette(16, 4)), "the focused one");
    assert_eq!(leds.get("LED_SCREEN_BUTTON_5"), Some(deckard_mk3::leds::DIM));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::Lock("update", 0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::Lock("delete", 0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Lock("bank", 1)]);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_7").is_empty(), "the first bank");
    hold(&mut s, &v, "BUTTON_ERASE", true);
    assert_eq!(s.event(&Event::PadOn(1, 0.8), &v, now), [Action::Lock("delete_slot", 0)], "ERASE + a snapshot's pad");
    assert!(s.event(&Event::PadOn(3, 0.8), &v, now).is_empty(), "ERASE + a free pad");
    hold(&mut s, &v, "BUTTON_ERASE", false);
    // Morphing: MORPH, then its MODE and TIME.
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::Lock("morph", 1)]);
    assert!(s.event(&Event::Knob(1, 0.1), &v, now).is_empty(), "MODE: only with Morph");
    (v.lock.morph, v.lock.time, v.lock.grid) = (true, 1, 3);
    assert_eq!(s.event(&Event::Knob(1, 0.1), &v, now), [Action::Lock("morph_mode", 1)]);
    assert_eq!(s.event(&Event::Knob(2, 0.1), &v, now), [Action::Lock("morph_time", 2)], "Travel: 1 Bar to 2 Bars");
    v.lock.mode = 1;
    assert_eq!(s.event(&Event::Knob(2, -0.1), &v, now), [Action::Lock("morph_grid", 2)], "Target: Bar to 1/2");
    press(&mut s, &v, "BUTTON_LOCK");
    assert_eq!(s.page(), "control", "LOCK on the page: back");
}

#[test]
fn variation_shows_the_sounds_groove_variation() {
    let (mut s, v) = (Surface::new(), view());
    press(&mut s, &v, "BUTTON_FILE");
    assert_eq!(hold(&mut s, &v, "BUTTON_VARIATION", true), [Action::GroovePage(1)]);
    assert_eq!(s.page(), "control", "from any page");
    assert_eq!(s.leds(&v).get("LED_VARIATION"), Some(crate::leds::ON), "lit while held");
    hold(&mut s, &v, "BUTTON_VARIATION", false);
    assert_eq!(s.leds(&v).get("LED_VARIATION"), Some(deckard_mk3::leds::BACKLIGHT));
}

fn event_note(sound: usize, position: i64, selected: bool) -> crate::NoteView {
    crate::NoteView { sound, position, length: 240, pitch: 60, velocity: 100, probability: 100, selected }
}

#[test]
fn events_held_edits_the_selected_notes() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    v.pattern_length = 3840;
    v.grid.step_snap = true;
    v.notes = vec![event_note(0, 0, false), event_note(0, 960, false)];
    hold(&mut s, &v, "BUTTON_EVENTS", true);
    assert_eq!(s.page(), "events");
    assert!(s.pads(&v).play, "the pads play as ever");
    let leds = s.leds(&v);
    for (led, want) in [("1", deckard_mk3::leds::DIM), ("2", deckard_mk3::leds::DIM), ("4", crate::leds::ON), ("5", deckard_mk3::leds::DIM)]
    {
        assert_eq!(leds.get(&format!("LED_SCREEN_BUTTON_{led}")), Some(want), "display button {led}");
    }
    assert!(lit(&s, &v, "LED_RIGHT") && !lit(&s, &v, "LED_LEFT"), "two knob pages");
    assert!(lit(&s, &v, "LED_WHEEL_BUTTON_LEFT") && lit(&s, &v, "LED_WHEEL_BUTTON_UP"));
    let text = s.screen_text(&v).join("|");
    assert!(text.contains("Edit") && text.contains("POSITION") && text.contains("PROB LOCK"), "{text}");
    assert_eq!(s.error(), None);
    // Nothing selected: the values are "-".
    v.notes[1].selected = true;
    let text = s.screen_text(&v).join("|");
    assert!(text.contains("1.2.1") && text.contains("C3"), "the selected note's position and pitch: {text}");
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::Events("nudge", 1)]);
    assert_eq!(s.event(&Event::Knob(1, -0.1), &v, now), [Action::Events("pitch", -1)]);
    assert_eq!(s.event(&Event::Knob(2, 0.1), &v, now), [Action::Events("velocity", 1)]);
    assert_eq!(s.event(&Event::Knob(3, 0.1), &v, now), [Action::Events("length", 1)]);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::Events("nudge_fine", 1)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::Events("quantize_half", 0)]);
    assert!(s.screen_text(&v).join("|").contains("QUANT 50%"));
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::Events("quantize", 0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::Events("clear_all", 0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_4"), [Action::Grid("step_snap", 0)]);
    // The second knob page: PROBABILITY.
    press(&mut s, &v, "BUTTON_RIGHT");
    assert!(lit(&s, &v, "LED_LEFT") && !lit(&s, &v, "LED_RIGHT"));
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::Events("probability", 1)]);
    assert!(s.screen_text(&v).join("|").contains("PROBABILITY"));
    // The 4-D encoder: a note after another, nudges, other sounds.
    assert_eq!(s.event(&Event::Wheel(1), &v, now), [Action::Events("select_step", 1)]);
    assert_eq!(press(&mut s, &v, "BUTTON_WHEEL_RIGHT"), [Action::Events("nudge_shown", 1)]);
    v.sound = 1;
    assert_eq!(press(&mut s, &v, "BUTTON_WHEEL_DOWN"), [Action::FocusSound(2)]);
    hold(&mut s, &v, "BUTTON_EVENTS", false);
    assert_eq!(s.page(), "control", "let go: back");
}

#[test]
fn the_events_select_mode_selects_with_the_pads() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    v.pattern_length = 3840;
    (v.grid.step_ticks, v.grid.step_snap) = (240, true);
    v.notes = vec![event_note(0, 0, false), event_note(0, 960, false), event_note(1, 480, false)];
    hold(&mut s, &v, "BUTTON_EVENTS", true);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_2").is_empty());
    assert!(!s.pads(&v).play, "the pads select");
    assert_eq!(s.leds(&v).get("LED_SCREEN_BUTTON_2"), Some(crate::leds::ON));
    assert!(lit(&s, &v, "LED_SCREEN_BUTTON_7"));
    let text = s.screen_text(&v).join("|");
    assert!(text.contains("Select") && text.contains("ALL") && text.contains("START") && text.contains("EVENT"), "{text}");
    assert_eq!(s.leds(&v).get("LED_PAD_1"), Some(deckard_mk3::leds::palette(0, 2)), "sound 1 has notes");
    assert_eq!(s.leds(&v).get("LED_PAD_3"), Some(0), "sound 3 has none");
    assert_eq!(s.event(&Event::PadOn(2, 0.8), &v, now), [Action::Events("select_sound", 1)]);
    hold(&mut s, &v, "BUTTON_ERASE", true);
    assert_eq!(s.event(&Event::PadOn(2, 0.8), &v, now), [Action::EraseSound(1)]);
    hold(&mut s, &v, "BUTTON_ERASE", false);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::Events("select_all", 0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::Events("select_none", 0)]);
    v.sound = 1;
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::FocusSound(2)], "DOWN: the next sound");
    v.sound = 0;
    assert_eq!(s.event(&Event::Knob(3, 0.2), &v, now), [Action::Events("select_step", 1)]);
    // START from the pattern's start by the step grid: what's from there to END.
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::EventsRange { start: 240, end: 3840, low: 0, high: 127 }]);
    v.notes[0].selected = true;
    assert_eq!(s.leds(&v).get("LED_PAD_1"), Some(deckard_mk3::leds::palette(0, 2)), "not all of sound 1's");
    v.notes[1].selected = true;
    assert_eq!(s.leds(&v).get("LED_PAD_1"), Some(deckard_mk3::leds::palette(0, 4)), "all of sound 1's");
    assert_eq!(s.error(), None);
}

/// CONTROLLER-BUGS C47: a plug-in DECKARD has a picture of shows it in the chain (its
/// thumbnail) and on the right display (its logo); one without shows a drawing.
#[test]
fn plugin_pictures_show_on_the_plugin_page() {
    let (mut s, mut v) = (Surface::new(), view());
    let (thumb, logo) = ([10, 200, 30, 255], [200, 30, 10, 255]);
    s.add_picture("sampler", true, 160, 90, vec![thumb; 160 * 90]);
    s.add_picture("sampler", false, 240, 196, vec![logo; 240 * 196]);
    let slot = crate::SlotView { name: "Sampler".into(), instrument: true, active: true, plugin: false, picture: Some("sampler".into()) };
    v.control.chain = vec![slot];
    (v.control.slot_count, v.control.slot_index) = (1, Some(0));
    let count =
        |px: &[[u8; 4]], c: [u8; 4], right: bool| px.iter().enumerate().filter(|(i, p)| **p == c && ((i % 960) >= 480) == right).count();
    let px = s.picture(&v);
    assert!(count(&px, thumb, false) > 2000, "the thumbnail in the chain: {}", count(&px, thumb, false));
    assert!(count(&px, logo, true) > 5000, "the logo on the right: {}", count(&px, logo, true));
    // Without the picture: a drawing, none of it.
    v.control.chain[0].picture = Some("nothing".into());
    let px = s.picture(&v);
    assert_eq!((count(&px, thumb, false), count(&px, logo, true)), (0, 0));
    assert_eq!(s.error(), None);
}

#[test]
fn shift_on_the_plugin_page_edits_the_chain() {
    let (mut s, mut v) = (Surface::new(), view());
    let slot = |name: &str, instrument, active, plugin| crate::SlotView { name: name.into(), instrument, active, plugin, picture: None };
    v.control.chain = vec![slot("Sampler", true, true, false), slot("EQ", false, true, false), slot("Reverb", false, false, true)];
    (v.control.slot_count, v.control.slot_index) = (3, Some(1));
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    for (led, on) in [("2", false), ("3", true), ("5", false), ("6", true), ("7", true), ("8", true)] {
        assert_eq!(lit(&s, &v, &format!("LED_SCREEN_BUTTON_{led}")), on, "display button {led}");
    }
    let text = s.screen_text(&v).join("|");
    assert!(text.contains("INSERT") && text.contains("MOVE") && text.contains("BYPASS") && text.contains("REMOVE"), "{text}");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_5").is_empty(), "EQ can't go before the instrument");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::SlotEdit("move_right")]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_7"), [Action::SlotEdit("bypass")]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::SlotEdit("remove")]);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_2").is_empty(), "EDIT: not a plug-in");
    v.control.slot_index = Some(2);
    assert!(lit(&s, &v, "LED_SCREEN_BUTTON_2") && lit(&s, &v, "LED_SCREEN_BUTTON_5"));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_2"), [Action::SlotEdit("edit")]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_5"), [Action::SlotEdit("move_left")]);
    // INSERT: the plug-in browser, for a new slot after the last.
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_3"), [Action::Slot(1)]);
    assert_eq!(s.page(), "module");
    assert_eq!(s.error(), None);
}

#[test]
fn step_mode_enters_steps_of_the_focused_sound() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    (v.pattern_length, v.grid.step_ticks) = (2 * 3840, 240);
    v.notes = vec![event_note(0, 0, false)];
    press(&mut s, &v, "BUTTON_STEP");
    assert_eq!(s.page(), "step");
    assert!(!s.pads(&v).play, "the pads are steps");
    let leds = s.leds(&v);
    assert_eq!(leds.get("LED_SCREEN_BUTTON_1"), Some(crate::leds::ON));
    for led in ["3", "4", "5", "7", "8"] {
        assert_eq!(leds.get(&format!("LED_SCREEN_BUTTON_{led}")), Some(deckard_mk3::leds::DIM), "display button {led}");
    }
    assert!(lit(&s, &v, "LED_RIGHT"), "two knob pages");
    assert_eq!(leds.get("LED_PAD_1"), Some(deckard_mk3::leds::palette(0, 4)), "a step with a note");
    assert_eq!(leds.get("LED_PAD_2"), Some(0));
    let text = s.screen_text(&v).join("|");
    for t in ["STEP MODE", "DOUBLE", "KEYBOARD", "FOLLOW", "PROBABILITY", "PROB LOCK", "POSITION", "VELOCITY", "LENGTH"] {
        assert!(text.contains(t), "{t}: {text}");
    }
    // A step goes in (or out) as its pad's let go, at the pad's velocity.
    assert!(s.event(&Event::PadOn(2, 0.5), &v, now).is_empty());
    assert_eq!(s.event(&Event::PadOff(2), &v, now), [Action::StepToggle { at: 240, length: 240, velocity: 64 }]);
    (v.fixed_velocity, v.velocity) = (true, 100);
    s.event(&Event::PadOn(3, 0.5), &v, now);
    assert_eq!(s.event(&Event::PadOff(3), &v, now), [Action::StepToggle { at: 480, length: 240, velocity: 100 }]);
    assert!(s.screen_text(&v).join("|").contains("FIXEDVELOCITY"));
    assert_eq!(s.event(&Event::Knob(2, 0.1), &v, now), [Action::VelocityValue(101)], "FIXEDVELOCITY's knob");
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::Events("nudge", 1)]);
    // Knob 6: the next 16 steps.
    assert!(s.event(&Event::Knob(5, 0.1), &v, now).is_empty());
    s.event(&Event::PadOn(1, 0.5), &v, now);
    assert_eq!(s.event(&Event::PadOff(1), &v, now), [Action::StepToggle { at: 3840, length: 240, velocity: 100 }]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_3"), [Action::PatternEdit("double")]);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_5").is_empty());
    assert_eq!(s.leds(&v).get("LED_SCREEN_BUTTON_5"), Some(crate::leds::ON), "FOLLOW");
    assert!(lit(&s, &v, "LED_FOLLOW"), "the FOLLOW button shows Step mode's own");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_7").is_empty());
    assert_eq!(s.leds(&v).get("LED_SCREEN_BUTTON_7"), Some(crate::leds::ON), "PROBABILITY shown");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_4"), [Action::KeyboardMode(true)]);
    assert_eq!(s.error(), None);
}

/// Note Repeat's settings as the app gives them: a new project's.
fn repeat_view() -> crate::RepeatView {
    let set = |key: &str, name: &str, text: &str, value: f64, (min, max, step, turn): (f64, f64, f64, f64)| crate::RepeatSetting {
        key: key.into(),
        name: name.into(),
        text: text.into(),
        value,
        min,
        max,
        step,
        turn,
    };
    let list = |n: f64| (0.0, n - 1.0, 1.0, 0.08);
    crate::RepeatView {
        settings: vec![
            set("type", "TYPE", "Up", 0.0, list(5.0)),
            set("rate", "RATE", "1/16", 4.0, list(8.0)),
            set("rate_unit", "UNIT", "Normal", 0.0, list(3.0)),
            set("sequence", "SEQUENCE", "Off", 0.0, list(9.0)),
            set("octaves", "OCTAVES", "1", 1.0, (1.0, 8.0, 1.0, 0.06)),
            set("dynamic", "DYNAMIC", "100%", 1.0, (0.01, 2.0, 0.01, 0.004)),
            set("gate", "GATE", "50%", 0.5, (0.0, 2.0, 0.01, 0.004)),
            set("retrigger", "RETRIGGER", "Off", 0.0, (0.0, 16.0, 1.0, 0.06)),
            set("repeat", "REPEAT", "Off", 0.0, (0.0, 16.0, 1.0, 0.06)),
            set("offset", "OFFSET", "0", 0.0, (-8.0, 8.0, 1.0, 0.06)),
            set("inversion", "INVERSION", "Off", 0.0, (0.0, 4.0, 1.0, 0.06)),
            set("min_key", "MIN. KEY", "C-2", 0.0, (0.0, 127.0, 1.0, 0.06)),
            set("max_key", "MAX. KEY", "G8", 127.0, (0.0, 127.0, 1.0, 0.06)),
        ],
        presets: vec!["1/8".into(), "1/16".into(), "1/16T".into(), "1/32".into()],
        preset: 1,
        hold: false,
        locked: false,
        active: false,
    }
}

#[test]
fn note_repeat_held_shows_its_settings() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    v.note_repeat = repeat_view();
    hold(&mut s, &v, "BUTTON_NOTE_REPEAT", true);
    assert_eq!(s.page(), "note_repeat");
    assert!(s.pads(&v).play, "the pads play (and repeat)");
    assert!(lit(&s, &v, "LED_NOTE_REPEAT"));
    let leds = s.leds(&v);
    for (led, want) in [
        ("1", deckard_mk3::leds::DIM),
        ("2", deckard_mk3::leds::DIM),
        ("3", deckard_mk3::leds::DIM),
        ("6", crate::leds::ON),
        ("7", deckard_mk3::leds::DIM),
    ] {
        assert_eq!(leds.get(&format!("LED_SCREEN_BUTTON_{led}")), Some(want), "display button {led}");
    }
    assert!(!lit(&s, &v, "LED_RIGHT"), "one knob page");
    let text = s.screen_text(&v).join("|");
    for t in ["NOTE RPT", "LOCK", "HOLD", "GATE RESET", "1/8", "1/16T", "1/32", "Rhythm", "RATE", "UNIT", "Other", "GATE", "50%"] {
        assert!(text.contains(t), "{t}: {text}");
    }
    assert!(!text.contains("TYPE"), "the Arp's settings only in Keyboard mode: {text}");
    // RATE, UNIT and GATE on knobs 3, 4 and 8; the others nothing.
    assert_eq!(s.event(&Event::Knob(2, 0.1), &v, now), [Action::Repeat("rate", 5.0)]);
    assert_eq!(s.event(&Event::Knob(3, 0.1), &v, now), [Action::Repeat("rate_unit", 1.0)]);
    assert!(s.event(&Event::Knob(0, 0.1), &v, now).is_empty());
    let Action::Repeat("gate", g) = s.event(&Event::Knob(7, 0.02), &v, now)[0] else { panic!("GATE") };
    assert!((g - 0.55).abs() < 1e-9, "{g}: five steps of 1%");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_2"), [Action::Repeat("locked", 1.0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_3"), [Action::Repeat("hold", 1.0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_4"), [Action::Repeat("gate", 1.0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_7"), [Action::Repeat("preset", 3.0)]);
    // Let go, the page before.
    hold(&mut s, &v, "BUTTON_NOTE_REPEAT", false);
    assert_ne!(s.page(), "note_repeat");
    // Locked, the button dim off the page.
    v.note_repeat.locked = true;
    v.note_repeat.active = true;
    assert_eq!(s.leds(&v).get("LED_NOTE_REPEAT"), Some(deckard_mk3::leds::DIM));
    assert_eq!(s.error(), None);
}

#[test]
fn shift_note_repeat_shows_the_arp() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    v.note_repeat = repeat_view();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    let acts = hold(&mut s, &v, "BUTTON_NOTE_REPEAT", true);
    assert!(acts.contains(&Action::KeyboardMode(true)), "{acts:?}");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), "note_repeat");
    let text = s.screen_text(&v).join("|");
    for t in ["ARP", "Main", "TYPE", "Rhythm", "RATE", "UNIT", "SEQUENCE", "Other", "OCTAVES", "DYNAMIC", "GATE", "1/2"] {
        assert!(text.contains(t), "{t}: {text}");
    }
    assert!(lit(&s, &v, "LED_RIGHT") && !lit(&s, &v, "LED_LEFT"));
    // The pads keep showing the keyboard, as Maschine's (its Repeat page lights them
    // as the pad mode does).
    let pads = |s: &mut Surface| (1..=16).map(|i| s.leds(&v).get(&*format!("LED_PAD_{i}"))).collect::<Vec<_>>();
    let shown = pads(&mut s);
    assert_eq!(s.event(&Event::Knob(1, 0.1), &v, now), [Action::Repeat("type", 1.0)]);
    assert!(press(&mut s, &v, "BUTTON_RIGHT").is_empty());
    let text = s.screen_text(&v).join("|");
    for t in ["Advanced", "RETRIGGER", "REPEAT", "OFFSET", "INVERSION", "Range", "MIN. KEY", "MAX. KEY", "2/2"] {
        assert!(text.contains(t), "{t}: {text}");
    }
    assert_eq!(s.event(&Event::Knob(5, -0.1), &v, now), [Action::Repeat("max_key", 126.0)]);
    assert!(lit(&s, &v, "LED_LEFT") && !lit(&s, &v, "LED_RIGHT"));
    assert_eq!(s.error(), None);
    hold(&mut s, &v, "BUTTON_NOTE_REPEAT", false);
    assert_ne!(s.page(), "note_repeat");
    assert_eq!(shown, pads(&mut s), "the keyboard's lights");
}

#[test]
fn note_repeat_leaves_step_mode() {
    let (mut s, mut v) = (Surface::new(), view());
    v.note_repeat = repeat_view();
    press(&mut s, &v, "BUTTON_PLUGIN");
    press(&mut s, &v, "BUTTON_STEP");
    hold(&mut s, &v, "BUTTON_NOTE_REPEAT", true);
    assert_eq!(s.page(), "note_repeat");
    hold(&mut s, &v, "BUTTON_NOTE_REPEAT", false);
    assert_eq!(s.page(), "control", "the page under Step mode's, as Maschine's");
    assert!(s.pads(&v).play);
}

fn touch(s: &mut Surface, v: &View, kind: u8, pos: f64) -> Vec<Action> {
    s.event(&Event::Touch(0, kind, pos), v, Instant::now())
}

/// The touch strip's LEDs, 1-25: (palette, bright) or `None` when off.
fn strip(s: &Surface, v: &View) -> Vec<Option<u8>> {
    let leds = s.leds(v);
    (1..=25).map(|i| leds.get(&format!("LED_TS_{i:02}")).filter(|c| *c != 0)).collect()
}

#[test]
fn the_strip_modes_toggle() {
    let (mut s, v) = (Surface::new(), view());
    for (button, led) in
        [("BUTTON_PITCH", "LED_PITCH"), ("BUTTON_MOD", "LED_MOD"), ("BUTTON_PERFORM", "LED_PERFORM"), ("BUTTON_NOTES", "LED_NOTES")]
    {
        assert!(press(&mut s, &v, button).is_empty());
        assert!(lit(&s, &v, led), "{button}");
        for other in ["LED_PITCH", "LED_MOD", "LED_PERFORM", "LED_NOTES"].iter().filter(|l| **l != led) {
            assert!(!lit(&s, &v, other), "{button}: {other} off");
        }
    }
    press(&mut s, &v, "BUTTON_NOTES");
    assert!(!lit(&s, &v, "LED_NOTES"), "pressed again, the strip's off");
    assert!(touch(&mut s, &v, 0, 0.5).is_empty(), "and does nothing");
    assert_eq!(s.error(), None);
}

#[test]
fn pitch_bends_and_springs_back() {
    let (mut s, v) = (Surface::new(), view());
    let white = deckard_mk3::leds::palette(16, 4);
    let colour = deckard_mk3::leds::palette(0, 4);
    press(&mut s, &v, "BUTTON_PITCH");
    let leds = strip(&s, &v);
    assert_eq!(leds[12], Some(white), "the middle white");
    assert_eq!(leds.iter().flatten().count(), 1);
    assert_eq!(touch(&mut s, &v, 0, 1.0), [Action::Strip("pitch", 1.0)]);
    let leds = strip(&s, &v);
    assert_eq!(leds[24], Some(white), "where it's touched");
    assert!(leds[12..24].iter().all(|l| *l == Some(colour)), "{leds:?}");
    assert!(leds[..12].iter().all(Option::is_none));
    assert_eq!(touch(&mut s, &v, 1, 0.25), [Action::Strip("pitch", -0.5)]);
    assert_eq!(touch(&mut s, &v, 2, 0.25), [Action::Strip("pitch", 0.0)], "let go, back to the middle");
    assert_eq!(strip(&s, &v)[12], Some(white));
}

#[test]
fn mod_stays_where_it_was_let_go() {
    let (mut s, v) = (Surface::new(), view());
    let white = deckard_mk3::leds::palette(16, 4);
    let colour = deckard_mk3::leds::palette(0, 4);
    press(&mut s, &v, "BUTTON_MOD");
    assert_eq!(strip(&s, &v)[0], Some(white), "nothing yet: the first white");
    assert_eq!(touch(&mut s, &v, 0, 0.5), [Action::Strip("mod", 0.5)]);
    assert!(touch(&mut s, &v, 2, 0.5).is_empty(), "let go, it stays");
    let leds = strip(&s, &v);
    assert_eq!(leds[12], Some(white));
    assert!(leds[..12].iter().all(|l| *l == Some(colour)), "{leds:?}");
    assert!(leds[13..].iter().all(Option::is_none));
}

#[test]
fn perform_plays_the_groups_perform_fx() {
    let (mut s, mut v) = (Surface::new(), view());
    press(&mut s, &v, "BUTTON_PERFORM");
    assert!(touch(&mut s, &v, 0, 0.5).is_empty(), "no Perform FX, nothing");
    assert!(strip(&s, &v).iter().all(Option::is_none));
    v.groups[0].perform = Some((false, 0.5));
    assert_eq!(touch(&mut s, &v, 0, 0.5), [Action::Strip("perform_touch", 0.5)]);
    assert_eq!(touch(&mut s, &v, 1, 0.75), [Action::Strip("perform", 0.75)]);
    assert_eq!(touch(&mut s, &v, 2, 0.75), [Action::Strip("perform_release", 0.75)]);
    // Its position lit in the group's colour, white while touched.
    let colour = deckard_mk3::leds::palette(1, 4);
    let leds = strip(&s, &v);
    assert_eq!(&leds[11..14], [Some(colour); 3]);
    v.groups[0].perform = Some((true, 0.5));
    assert_eq!(strip(&s, &v)[12], Some(deckard_mk3::leds::palette(16, 4)));
    // SHIFT + PERFORM: the group's Perform FX (one loaded if none), the strip stays.
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(press(&mut s, &v, "BUTTON_PERFORM"), [Action::PerformFx]);
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert!(lit(&s, &v, "LED_PERFORM"));
    assert_eq!(s.page(), "control");
}

#[test]
fn notes_strums_the_pads() {
    let (mut s, mut v) = (Surface::new(), view());
    press(&mut s, &v, "BUTTON_NOTES");
    let on = |sound, pitch| Action::StripNote { sound, pitch, velocity: 100, on: true };
    let off = |sound, pitch| Action::StripNote { sound, pitch, velocity: 100, on: false };
    // Group mode, no pads held: the 16 sounds along the strip, each at its base key.
    assert_eq!(touch(&mut s, &v, 0, 0.0), [on(0, 60)]);
    assert!(touch(&mut s, &v, 1, 0.03).is_empty(), "still pad 1");
    assert_eq!(touch(&mut s, &v, 1, 0.2), [off(0, 60), on(3, 60)]);
    assert_eq!(touch(&mut s, &v, 2, 0.2), [off(3, 60)]);
    // Pads held: only those.
    s.event(&Event::PadOn(2, 0.5), &v, Instant::now());
    s.event(&Event::PadOn(5, 0.5), &v, Instant::now());
    assert_eq!(touch(&mut s, &v, 0, 0.9), [on(4, 60)]);
    assert_eq!(touch(&mut s, &v, 1, 0.1), [off(4, 60), on(1, 60)]);
    touch(&mut s, &v, 2, 0.1);
    // The touch lit in the group's colour.
    s.event(&Event::PadOff(2), &v, Instant::now());
    s.event(&Event::PadOff(5), &v, Instant::now());
    (v.fixed_velocity, v.velocity) = (true, 90);
    press(&mut s, &v, "BUTTON_KEYBOARD");
    let keys = touch(&mut s, &v, 0, 1.0);
    assert_eq!(keys, [Action::StripNote { sound: 0, pitch: 75, velocity: 90, on: true }], "Keyboard mode: the keys");
    assert_eq!(strip(&s, &v)[24], Some(deckard_mk3::leds::palette(0, 4)));
    assert_eq!(s.error(), None);
}

#[test]
fn the_plugin_pages_info_bar_names_the_level() {
    let (mut s, mut v) = (Surface::new(), view());
    press(&mut s, &v, "BUTTON_PLUGIN");
    v.control.level = 2;
    assert!(s.screen_text(&v).join("|").contains("Sound 1"), "the sound");
    v.control.level = 1;
    let text = s.screen_text(&v).join("|");
    assert!(text.contains("A1") && text.contains("Drums") && !text.contains("Sound 1"), "the group: {text}");
    v.control.level = 0;
    let text = s.screen_text(&v).join("|");
    assert!(text.contains("|M|") && !text.contains("Drums"), "the master: {text}");
}

#[test]
fn shift_and_a_pad_edit_the_pattern() {
    let (mut s, v) = (Surface::new(), view());
    let now = Instant::now();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    let want = [
        Action::Edit("undo"),
        Action::Edit("redo"),
        Action::Edit("undo_step"),
        Action::Edit("redo_step"),
        Action::Events("quantize", 0),
        Action::Events("quantize_half", 0),
        Action::Events("nudge_shown", -1),
        Action::Events("nudge_shown", 1),
        Action::Events("clear", 0),
        Action::Edit("clear_modulation"),
        Action::Edit("copy"),
        Action::Edit("paste"),
        Action::Events("transpose", -1),
        Action::Events("transpose", 1),
        Action::Events("transpose", -12),
        Action::Events("transpose", 12),
    ];
    for (i, a) in want.into_iter().enumerate() {
        assert_eq!(s.event(&Event::PadOn(i + 1, 0.5), &v, now), [a], "pad {}", i + 1);
        s.event(&Event::PadOff(i + 1), &v, now);
    }
    // On a page with its own pads too (Step mode's): its pad not let go.
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    press(&mut s, &v, "BUTTON_KEYBOARD");
    press(&mut s, &v, "BUTTON_STEP");
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert_eq!(s.event(&Event::PadOn(9, 0.5), &v, now), [Action::Events("clear", 0)]);
    assert!(s.event(&Event::PadOff(9), &v, now).is_empty(), "no step let go");
    // ERASE held (Pad mode): the pad's sound removed.
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    press(&mut s, &v, "BUTTON_PAD_MODE");
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    hold(&mut s, &v, "BUTTON_ERASE", true);
    assert_eq!(s.event(&Event::PadOn(2, 0.5), &v, now), [Action::ResetSound(1)]);
    assert_eq!(s.error(), None);
}

#[test]
fn prob_lock_locks_the_focused_sound() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    v.sound = 2;
    press(&mut s, &v, "BUTTON_STEP");
    assert_eq!(s.event(&Event::Button("BUTTON_SCREEN_8", true), &v, now), [Action::ProbLock(true)]);
    s.event(&Event::Button("BUTTON_SCREEN_8", false), &v, now);
    v.groups[v.group].sounds[2].prob_locked = true;
    assert_eq!(s.leds(&v).get("LED_SCREEN_BUTTON_8"), Some(crate::leds::ON), "lit while locked");
    assert_eq!(s.event(&Event::Button("BUTTON_SCREEN_8", true), &v, now), [Action::ProbLock(false)]);
    s.event(&Event::Button("BUTTON_SCREEN_8", false), &v, now);
    // The Events page's too.
    press(&mut s, &v, "BUTTON_PAD_MODE");
    hold(&mut s, &v, "BUTTON_EVENTS", true);
    assert_eq!(s.leds(&v).get("LED_SCREEN_BUTTON_8"), Some(crate::leds::ON));
    assert_eq!(s.event(&Event::Button("BUTTON_SCREEN_8", true), &v, now), [Action::ProbLock(false)]);
    assert_eq!(s.error(), None);
}

#[test]
fn holding_a_step_modulates_it() {
    let (mut s, mut v) = (Surface::new(), view());
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    (v.pattern_length, v.grid.step_ticks) = (2 * 3840, 240);
    for (i, k) in v.control.knobs.iter_mut().enumerate() {
        if let Some(k) = k {
            k.modulatable = i != 1;
        }
    }
    v.control.knobs[2].as_mut().unwrap().step_mod = Some("+25%".into());
    press(&mut s, &v, "BUTTON_STEP");
    assert!(s.event(&Event::PadOn(3, 0.5), &v, at(0)).is_empty());
    assert!(s.tick(&v, at(100)).is_empty(), "not yet");
    assert_eq!(s.page(), "step");
    // Held a moment: the Step Mod page, for that step.
    assert_eq!(s.tick(&v, at(500)), [Action::StepModAt(Some((480, 240)))]);
    assert_eq!(s.page(), "step_mod");
    let text = s.screen_text(&v).join("|");
    assert!(text.contains("KNOB 0") && !text.contains("KNOB 1"), "only what can be modulated: {text}");
    assert!(text.contains("+25%"), "the step's modulation: {text}");
    // The knobs set the held steps' modulation.
    assert_eq!(s.event(&Event::Knob(0, 0.05), &v, at(600)), [Action::StepMod { from: 0, length: 240, pads: 1 << 2, knob: 0, inc: 0.05 }]);
    assert!(s.event(&Event::Knob(1, 0.05), &v, at(600)).is_empty(), "not modulatable");
    // Another pad held too: both, the last one's values shown.
    assert_eq!(s.event(&Event::PadOn(6, 0.5), &v, at(700)), [Action::StepModAt(Some((1200, 240)))]);
    assert_eq!(
        s.event(&Event::Knob(2, -0.1), &v, at(800)),
        [Action::StepMod { from: 0, length: 240, pads: 1 << 2 | 1 << 5, knob: 2, inc: -0.1 }]
    );
    // The Control page's buttons: the next slot.
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::Slot(1)]);
    // Let go: no step taken, and back to the Step page with the last.
    assert_eq!(s.event(&Event::PadOff(6), &v, at(900)), [Action::StepModAt(Some((480, 240)))]);
    assert_eq!(s.event(&Event::PadOff(3), &v, at(900)), [Action::StepModAt(None)]);
    assert_eq!(s.page(), "step");
    // A quick press still takes a step.
    s.event(&Event::PadOn(4, 0.5), &v, at(1000));
    assert!(s.tick(&v, at(1100)).is_empty());
    assert_eq!(s.event(&Event::PadOff(4), &v, at(1150)), [Action::StepToggle { at: 720, length: 240, velocity: 64 }]);
    assert_eq!(s.error(), None);
}

/// A knob touched twice quickly: on the Events and Step pages VELOCITY and PROBABILITY
/// back to 100, on the Mixer the strip back to 0 dB.
#[test]
fn a_knob_tapped_twice_resets_it() {
    use std::time::Duration;
    let (mut s, mut v) = (Surface::new(), view());
    let t0 = Instant::now();
    v.notes = vec![event_note(0, 0, true)];
    let tap = |s: &mut Surface, name: &'static str, at: Instant| {
        let mut out = s.event(&Event::Button(name, true), &v, at);
        out.extend(s.event(&Event::Button(name, false), &v, at));
        out
    };
    hold(&mut s, &v, "BUTTON_EVENTS", true);
    assert!(tap(&mut s, "BUTTON_CAP_3", t0).is_empty());
    assert_eq!(tap(&mut s, "BUTTON_CAP_3", t0 + Duration::from_millis(200)), [Action::Events("velocity_to", 100)]);
    assert!(tap(&mut s, "BUTTON_CAP_3", t0 + Duration::from_secs(2)).is_empty(), "too slow");
    assert!(tap(&mut s, "BUTTON_CAP_2", t0 + Duration::from_millis(2100)).is_empty(), "another knob");
    press(&mut s, &v, "BUTTON_RIGHT");
    tap(&mut s, "BUTTON_CAP_1", t0 + Duration::from_secs(3));
    assert_eq!(tap(&mut s, "BUTTON_CAP_1", t0 + Duration::from_millis(3100)), [Action::Events("probability_to", 100)]);
    hold(&mut s, &v, "BUTTON_EVENTS", false);

    press(&mut s, &v, "BUTTON_STEP");
    tap(&mut s, "BUTTON_CAP_3", t0 + Duration::from_secs(4));
    assert_eq!(tap(&mut s, "BUTTON_CAP_3", t0 + Duration::from_millis(4100)), [Action::Events("velocity_to", 100)]);
    press(&mut s, &v, "BUTTON_STEP");

    press(&mut s, &v, "BUTTON_MIX");
    tap(&mut s, "BUTTON_CAP_2", t0 + Duration::from_secs(5));
    assert_eq!(
        tap(&mut s, "BUTTON_CAP_2", t0 + Duration::from_millis(5100)),
        [Action::ChannelLevel { group: v.group, sound: Some(1), db: 0.0 }]
    );
}

/// The Ideas view's knobs: the group above each steps through its patterns in the
/// focused scene, round in a circle with none after the last (C36).
#[test]
fn the_ideas_knobs_step_through_each_groups_patterns() {
    let (mut s, mut v) = (Surface::new(), view());
    song(&mut v);
    v.groups[0].patterns.push(PatternView { name: "Fill".into(), color: 2, default_color: true, bars: 1.0, ..Default::default() });
    v.groups[0].scene_pattern = Some(0);
    press(&mut s, &v, "BUTTON_ARRANGE");
    let now = Instant::now();
    let pick = |pattern| [Action::ScenePattern { group: 0, pattern }];
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), pick(Some(1)));
    v.groups[0].scene_pattern = Some(1);
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), pick(None), "past the last: none");
    v.groups[0].scene_pattern = None;
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), pick(Some(0)), "round to the first");
    assert_eq!(s.event(&Event::Knob(0, -0.1), &v, now), pick(Some(1)), "back from none: the last");
    assert!(s.event(&Event::Knob(1, 0.1), &v, now).is_empty(), "a group without patterns");
    assert!(s.event(&Event::Knob(2, 0.1), &v, now).is_empty(), "no group");
    v.scene = 99;
    assert!(s.event(&Event::Knob(0, 0.1), &v, now).is_empty(), "no scene");
}

/// A chain longer than the left screen scrolls there, keeping the focused slot in view;
/// nothing of it spills onto the right screen (C37).
#[test]
fn a_long_chain_scrolls_on_the_left_screen() {
    let (mut s, mut v) = (Surface::new(), view());
    let fx = |n: usize| crate::SlotView { name: format!("Effect {n}"), instrument: false, active: true, plugin: false, picture: None };
    v.control.chain = (1..=8).map(fx).collect();
    (v.control.slot_count, v.control.slot_index) = (8, Some(0));
    press(&mut s, &v, "BUTTON_PLUGIN");
    let names = |s: &Surface, v: &View| s.screen_text(v).into_iter().filter(|t| t.starts_with("Effect ")).collect::<Vec<_>>();
    let shown = names(&s, &v);
    assert!(shown.contains(&"Effect 1".to_string()), "{shown:?}");
    assert!(!shown.contains(&"Effect 8".to_string()), "the rest off the left screen: {shown:?}");
    v.control.slot_index = Some(7);
    let shown = names(&s, &v);
    assert!(shown.contains(&"Effect 8".to_string()) && !shown.contains(&"Effect 1".to_string()), "scrolled: {shown:?}");
    v.control.slot_index = None;
    let shown = names(&s, &v);
    assert!(!shown.contains(&"Effect 1".to_string()), "the new slot's \"+\" in view: {shown:?}");
}

/// Touching one of the Events page's knobs 5-8 shows what each does (zoom, scroll, the
/// lanes' size) in a small box above it, as Maschine's; let go, they go (C42).
#[test]
fn the_events_zoom_knobs_show_icons_while_touched() {
    let (mut s, v) = (Surface::new(), view());
    hold(&mut s, &v, "BUTTON_EVENTS", true);
    press(&mut s, &v, "BUTTON_SCREEN_1");
    hold(&mut s, &v, "BUTTON_EVENTS", false);
    let boxed = |s: &mut Surface| {
        let (_, px) = s.screens(&v).expect("redrawn");
        px[250 * 960 + 480 + 46][0] == 0x89 && px[250 * 960 + 480 + 166][0] == 0x89
    };
    assert!(!boxed(&mut s) && !s.screen_text(&v).contains(&"1×".to_string()), "not before a touch");
    hold(&mut s, &v, "BUTTON_CAP_6", true);
    assert!(boxed(&mut s), "the boxes over knobs 5 and 6");
    assert!(s.screen_text(&v).contains(&"1×".to_string()), "knob 7: the lanes' size");
    hold(&mut s, &v, "BUTTON_CAP_6", false);
    assert!(!boxed(&mut s), "let go: gone");
}

/// With AUTO on, the Control page's knobs are only what can be modulated, each showing
/// its modulation where the song is as a percentage, as the MK3 manual has it (C31).
#[test]
fn auto_write_shows_the_knobs_modulation() {
    let (s, mut v) = (Surface::new(), view());
    for (i, k) in v.control.knobs.iter_mut().enumerate() {
        if let Some(k) = k {
            k.modulatable = i != 1;
            k.auto_mod = Some(if i == 2 { "+25%".into() } else { "0%".into() });
        }
    }
    let text = s.screen_text(&v).join("|");
    assert!(text.contains("KNOB 1") && !text.contains("+25%"), "AUTO off: as ever: {text}");
    v.auto_write = true;
    let text = s.screen_text(&v).join("|");
    assert!(text.contains("KNOB 0") && !text.contains("KNOB 1"), "only what can be modulated: {text}");
    assert!(text.contains("+25%") && text.contains("0%"), "the modulation: {text}");
}

/// A knob whose parameter a macro controls shows the macro icon at the top right of
/// its name, and nothing else changes.
#[test]
fn a_knob_a_macro_controls_shows_the_macro_icon() {
    let (s, mut v) = (Surface::new(), view());
    let before = s.picture(&v);
    v.control.knobs[2].as_mut().unwrap().in_macro = true;
    let after = s.picture(&v);
    let changed: Vec<(usize, usize)> = (0..before.len()).filter(|&i| before[i] != after[i]).map(|i| (i % 960, i / 960)).collect();
    assert!(!changed.is_empty(), "an icon");
    // Knob 3 is x 240-360 on the left screen; its name 233-247 down.
    assert!(changed.iter().all(|&(x, y)| (330..360).contains(&x) && (231..250).contains(&y)), "{changed:?}");
}

#[test]
fn show_position_keeps_the_strip_dark_unless_recording() {
    let (s, mut v) = (Surface::new(), view());
    v.playing = true;
    v.progress = Some(0.5);
    v.groups[0].scene_pattern = Some(0);
    v.groups[0].patterns = vec![PatternView { color: 1, ..Default::default() }];
    let lit = |s: &Surface, v: &View| s.leds(v).get("LED_TS_01").unwrap_or(0) != 0;
    assert!(lit(&s, &v), "On: the play position shows");
    v.strip_position = 1;
    assert!(!lit(&s, &v), "Rec. Only, not recording");
    v.recording = true;
    assert!(lit(&s, &v), "Rec. Only, recording");
    v.strip_position = 0;
    assert!(!lit(&s, &v), "Off");
}

#[test]
fn a_held_knob_writes_with_touch_auto_write() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    (v.playing, v.auto_write) = (true, true);
    s.event(&Event::Button("CAP_3", true), &v, now);
    assert!(!s.tick(&v, now).contains(&Action::TouchWrite(2)), "Touch Auto-write off");
    v.touch_auto_write = true;
    assert!(s.tick(&v, now).contains(&Action::TouchWrite(2)));
    s.event(&Event::Button("CAP_3", false), &v, now);
    assert!(!s.tick(&v, now).contains(&Action::TouchWrite(2)), "let go");
}

/// C51: EVENTS, PATTERN and SCENE stay lit while their page is pinned, as SELECT does,
/// and go dark once it's gone.
#[test]
fn a_pinned_pages_button_stays_lit() {
    let (mut s, v) = (Surface::new(), view());
    for (button, led) in [("BUTTON_EVENTS", "LED_EVENTS"), ("BUTTON_PATTERN", "LED_PATTERN"), ("BUTTON_SCENE", "LED_SCENE")] {
        hold(&mut s, &v, button, true);
        assert_eq!(s.leds(&v).get(led), Some(crate::leds::ON), "{button} held");
        press(&mut s, &v, "BUTTON_SCREEN_1");
        hold(&mut s, &v, button, false);
        assert_eq!(s.leds(&v).get(led), Some(crate::leds::ON), "{button} pinned");
        // Taken off: the page goes.
        press(&mut s, &v, "BUTTON_SCREEN_1");
        assert_ne!(s.leds(&v).get(led), Some(crate::leds::ON), "{button} gone");
    }
}

/// C52: while TEMPO, SWING or VOLUME is on, the wheel does that on every page, its own
/// use of the wheel or not.
#[test]
fn the_wheel_modes_win_over_the_pages() {
    let (mut s, v) = (Surface::new(), view());
    let now = Instant::now();
    // (SCENE, PATTERN and EVENTS held; MIX and ARRANGE pressed.)
    for (page, held) in
        [("BUTTON_SCENE", true), ("BUTTON_PATTERN", true), ("BUTTON_EVENTS", true), ("BUTTON_MIX", false), ("BUTTON_ARRANGE", false)]
    {
        if held {
            hold(&mut s, &v, page, true);
        } else {
            press(&mut s, &v, page);
        }
        press(&mut s, &v, "BUTTON_VOLUME");
        assert_eq!(s.event(&Event::Wheel(-5), &v, now), [Action::VolumeDb(-2.5)], "{page}");
        press(&mut s, &v, "BUTTON_VOLUME");
        if held {
            hold(&mut s, &v, page, false);
        } else {
            press(&mut s, &v, page);
        }
    }
    assert_eq!(s.error(), None);
}

/// #25: with SHIFT held the pads run their shortcuts only, never playing (in Keyboard
/// mode too), so nothing is played or recorded.
#[test]
fn shift_keeps_the_pads_from_playing() {
    let (mut s, v) = (Surface::new(), view());
    for mode in [None, Some("BUTTON_KEYBOARD")] {
        if let Some(b) = mode {
            press(&mut s, &v, b);
        }
        assert!(s.pads(&v).play, "{mode:?}");
        hold(&mut s, &v, "BUTTON_SHIFT", true);
        assert!(!s.pads(&v).play, "{mode:?} with SHIFT");
        hold(&mut s, &v, "BUTTON_SHIFT", false);
    }
}

/// The Ideas page: every group with a pattern in the scene shows how far through it
/// the song is, not only the focused group, so their lengths and places compare.
#[test]
fn every_groups_bars_fill_as_it_plays() {
    let (mut s, mut v) = (Surface::new(), view());
    song(&mut v);
    v.groups.push(v.groups[0].clone());
    for g in &mut v.groups {
        g.patterns = vec![PatternView { name: "P".into(), color: 2, default_color: true, bars: 2.0, ..Default::default() }];
        g.scene_pattern = Some(0);
    }
    v.group = 0;
    v.playing = true;
    press(&mut s, &v, "BUTTON_ARRANGE");
    // Group 2's bars (the tile at x 120): a pixel a quarter of the way along.
    let at = |s: &Surface, v: &View| s.picture(v)[260 * 960 + 120 + 6 + 26];
    let before = at(&s, &v);
    v.groups[1].progress = Some(0.5);
    assert_ne!(at(&s, &v), before, "the unfocused group's bars fill");
}

/// Palenight: a chosen display button's border is two tones fading into each other that
/// run round its edge; the Maschine theme's stays still, as Maschine's.
/// The top edge (row 4, `xs`) of the screens at 0 and 400 ms in `theme`, after `setup`.
fn edges(theme: &str, v: &View, setup: fn(&mut Surface, &View), xs: std::ops::Range<usize>) -> [Vec<[u8; 4]>; 2] {
    let t0 = Instant::now();
    let mut s = Surface::new();
    s.set_theme(theme);
    assert_eq!(s.error(), None);
    setup(&mut s, v);
    [0, 400].map(|ms| {
        s.tick(v, t0 + std::time::Duration::from_millis(ms));
        let px = s.picture(v);
        xs.clone().map(|x| px[4 * 960 + x]).collect()
    })
}

#[test]
fn palenights_chosen_buttons_border_chases_round() {
    let v = view();
    // The Control page's MASTER / GROUP / SOUND tabs (the chosen one).
    let control: fn(&mut Surface, &View) = |_, _| {};
    let [a, b] = edges("maschine", &v, control, 4..357);
    assert!(a == b, "Maschine: still");
    let [a, b] = edges("palenight", &v, control, 4..357);
    assert!(a != b, "it moves");
    let tones: std::collections::HashSet<_> = a.iter().collect();
    assert!(tones.len() > 4, "a fade, not one colour: {tones:?}");
}

/// The chosen tab chases on every page with one, but the arranger's (Ideas, Song).
#[test]
fn palenights_border_chases_on_every_page_but_the_arrangers() {
    let v = grid_view();
    let grid: fn(&mut Surface, &View) = |s, v| {
        hold(s, v, "BUTTON_SHIFT", true);
        hold(s, v, "BUTTON_FOLLOW", true);
    };
    let [a, b] = edges("palenight", &v, grid, 123..476);
    assert!(a != b, "the Grid page's tabs");
    let ideas: fn(&mut Surface, &View) = |s, v| {
        press(s, v, "BUTTON_ARRANGE");
        hold(s, v, "BUTTON_SHIFT", true);
    };
    let [a, b] = edges("palenight", &v, ideas, 4..119);
    assert!(a == b, "IDEAS, on the Ideas page: still");
}

/// A theme with a moving border wants the screens redrawn even when nothing happens.
#[test]
fn a_chasing_theme_animates() {
    let mut s = Surface::new();
    assert!(!s.animates(), "Maschine's: still");
    s.set_theme("palenight");
    assert!(s.animates());
    s.set_theme("solarized_dark");
    assert!(!s.animates());
}

/// BUGS #6: a project reopens on the page and pad mode it was saved on (a page left on
/// its own, a dialog or a held page, isn't come back to).
#[test]
fn a_saved_screen_comes_back() {
    let (mut s, v) = (Surface::new(), view());
    press(&mut s, &v, "BUTTON_KEYBOARD");
    press(&mut s, &v, "BUTTON_MIX");
    assert_eq!(s.screen(), Some(("mixer".to_string(), "keyboard".to_string())));
    let mut s = Surface::new();
    let acts = s.restore("mixer", "keyboard", &v, Instant::now());
    assert_eq!(s.screen(), Some(("mixer".to_string(), "keyboard".to_string())));
    assert!(acts.contains(&Action::KeyboardMode(true)), "{acts:?}");
    let mut s = Surface::new();
    s.restore("file", "pad", &v, Instant::now());
    assert_eq!(s.page(), FIRST_PAGE);
}

/// SHIFT + ERASE + a group button deletes the group, and SHIFT + a group button picks
/// its bank, whatever page is shown (BUGS #40, #41): the pages leave the group buttons
/// to global.lua.
#[test]
fn shift_group_buttons_work_on_every_page() {
    let pages = [
        "arrange",
        "browse",
        "control",
        "events",
        "file",
        "grid",
        "keyboard",
        "lock",
        "macro_set",
        "mixer",
        "module",
        "note_repeat",
        "pad_mode",
        "pattern",
        "sampling",
        "save",
        "scene",
        "settings",
        "song",
        "step",
        "step_mod",
    ];
    let mut failed = Vec::new();
    for page in pages {
        let (mut s, mut v) = (Surface::new(), view());
        v.groups = (0..10).map(|_| v.groups[0].clone()).collect();
        v.group = 8;
        s.restore(page, "pad", &v, Instant::now());
        let shown = s.page();
        hold(&mut s, &v, "BUTTON_SHIFT", true);
        hold(&mut s, &v, "BUTTON_TRANSPORT_ERASE", true);
        let erase = press(&mut s, &v, "BUTTON_GROUP_B");
        hold(&mut s, &v, "BUTTON_TRANSPORT_ERASE", false);
        let bank = press(&mut s, &v, "BUTTON_GROUP_A");
        hold(&mut s, &v, "BUTTON_SHIFT", false);
        if erase != [Action::DeleteGroup(9)] || bank != [Action::FocusGroup(0)] {
            failed.push(format!("{page} (shown {shown}): erase {erase:?}, bank {bank:?}"));
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

/// SHIFT + SELECT shows the LFO page and, again, the page it was pressed on; SELECT
/// alone still shows the hold page.
#[test]
fn shift_select_opens_the_lfo_page() {
    let (mut s, v) = (Surface::new(), view());
    hold(&mut s, &v, "BUTTON_SELECT", true);
    assert_eq!(s.page(), "hold", "SELECT alone");
    hold(&mut s, &v, "BUTTON_SELECT", false);
    assert_eq!(s.page(), FIRST_PAGE);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert!(press(&mut s, &v, "BUTTON_SELECT").is_empty());
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), "lfo");
    assert_eq!(s.leds(&v).get("LED_SELECT"), Some(crate::leds::ON), "lit on its page");
    // (Let go of SHIFT first: SELECT's release is still the LFO page's.)
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    hold(&mut s, &v, "BUTTON_SELECT", true);
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    hold(&mut s, &v, "BUTTON_SELECT", false);
    assert_eq!(s.page(), FIRST_PAGE, "pressed again: back");
    assert_eq!(s.error(), None);
}

#[test]
fn lfo_page_pads_pick_and_create() {
    let (mut s, v) = (Surface::new(), view());
    let now = Instant::now();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SELECT");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert!(!s.pads(&v).play, "the pads pick LFOs");
    assert_eq!(s.event(&Event::PadOn(3, 0.8), &v, now), [Action::Lfo("create", 3, 0.0)], "an empty pad");
    assert_eq!(s.event(&Event::PadOn(1, 0.8), &v, now), [Action::Lfo("focus", 1, 0.0)], "an LFO's pad");
    hold(&mut s, &v, "BUTTON_TRANSPORT_ERASE", true);
    assert_eq!(s.event(&Event::PadOn(1, 0.8), &v, now), [Action::Lfo("remove", 1, 0.0)], "ERASE + an LFO's pad");
    assert!(s.event(&Event::PadOn(3, 0.8), &v, now).is_empty(), "ERASE + an empty pad");
    hold(&mut s, &v, "BUTTON_TRANSPORT_ERASE", false);
    let leds = s.leds(&v);
    assert_eq!(leds.get("LED_PAD_1"), Some(deckard_mk3::leds::palette(16, 4)), "the focused LFO");
    assert_eq!(leds.get("LED_PAD_2"), Some(deckard_mk3::leds::palette(16, 2)), "an LFO");
    assert_eq!(leds.get("LED_PAD_3"), Some(0), "none");
}

/// On the LFO page, MUTE + an LFO's pad mutes or unmutes it, and MUTE doesn't show the
/// hold page; elsewhere it still does.
#[test]
fn lfo_page_lets_go_of_a_hold_page_held_over() {
    let (mut s, v) = (Surface::new(), view());
    hold(&mut s, &v, "BUTTON_MUTE", true);
    assert_eq!(s.page(), "hold");
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SELECT");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), "lfo");
    hold(&mut s, &v, "BUTTON_MUTE", false);
    assert_eq!(s.page(), "lfo", "letting go of MUTE stays");
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SELECT");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_ne!(s.page(), "lfo", "one SHIFT + SELECT leaves");
}

#[test]
fn lfo_page_mute_and_a_pad() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SELECT");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    hold(&mut s, &v, "BUTTON_MUTE", true);
    assert_eq!(s.page(), "lfo", "MUTE held");
    assert_eq!(s.event(&Event::PadOn(1, 0.8), &v, now), [Action::Lfo("mute", 1, 0.0)]);
    assert!(s.event(&Event::PadOn(3, 0.8), &v, now).is_empty(), "MUTE + an empty pad");
    hold(&mut s, &v, "BUTTON_MUTE", false);
    assert_eq!(s.page(), "lfo");
    v.lfos[0].muted = true;
    let leds = s.leds(&v);
    assert_eq!(leds.get("LED_PAD_1"), Some(deckard_mk3::leds::palette(0, 4)), "muted, focused");
    assert_eq!(leds.get("LED_PAD_2"), Some(deckard_mk3::leds::palette(16, 2)), "unmuted");
    assert!(s.screen_text(&v).contains(&"MUTED".to_string()));
    v.lfos[0].muted = false;
    assert!(!s.screen_text(&v).contains(&"MUTED".to_string()));
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SELECT");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), FIRST_PAGE);
    hold(&mut s, &v, "BUTTON_MUTE", true);
    assert_eq!(s.page(), "hold", "MUTE on another page");
    hold(&mut s, &v, "BUTTON_MUTE", false);
    assert_eq!(s.error(), None);
}

#[test]
fn lfo_page_buttons_and_knobs() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SELECT");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_2"), [Action::Lfo("shape", 1, 2.0)], "Square");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::Lfo("sync", 1, 0.0)], "synced: to Hz");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_7"), [Action::Lfo("env", 1, 0.0)], "ENV: to Envelope mode");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Lfo("trigger_focused", 1, 1.0)], "Off: the focused sound");
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::Lfo("shape", 1, 2.0)], "Shape: Sine to Square");
    assert!(s.event(&Event::Knob(0, -0.1), &v, now).is_empty(), "Sine is the first");
    assert_eq!(s.event(&Event::Knob(1, 0.1), &v, now), [Action::Lfo("rate", 1, 1.0)]);
    assert_eq!(s.event(&Event::Knob(2, 0.1), &v, now), [Action::Lfo("phase", 1, 1.0)]);
    assert_eq!(s.event(&Event::Knob(3, -0.1), &v, now), [Action::Lfo("level", 1, -1.0)]);
    assert_eq!(s.event(&Event::Knob(4, 0.1), &v, now), [Action::Lfo("quantize", 1, 1.0)]);
    assert_eq!(s.event(&Event::Knob(5, 0.1), &v, now), [Action::Lfo("trigger", 1, 1.0)]);
    assert!(s.event(&Event::Knob(6, 0.1), &v, now).is_empty(), "Fade in: only with a trigger");
    v.lfos[0].trigger = "A1 / S1".into();
    assert_eq!(s.event(&Event::Knob(6, 0.1), &v, now), [Action::Lfo("fade", 1, 1.0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Lfo("trigger_focused", 1, 0.0)], "set: Off");
    let leds = s.leds(&v);
    assert_eq!(leds.get("LED_SCREEN_BUTTON_1"), Some(crate::leds::ON), "Sine");
    assert_eq!(leds.get("LED_SCREEN_BUTTON_2"), Some(deckard_mk3::leds::DIM));
    assert_eq!(leds.get("LED_SCREEN_BUTTON_6"), Some(crate::leds::ON), "synced");
    assert_eq!(leds.get("LED_SCREEN_BUTTON_8"), Some(crate::leds::ON), "triggered");
    v.lfo_focus = 0;
    assert!(press(&mut s, &v, "BUTTON_SCREEN_2").is_empty(), "no LFO focused");
    assert!(s.event(&Event::Knob(2, 0.1), &v, now).is_empty());
    assert_eq!(s.error(), None);
}

/// SHIFT + ERASE + an LFO's pad removes it, as ERASE + the pad does, and never resets
/// the sound under it; other SHIFT + pad shortcuts still work on the page.
#[test]
fn lfo_page_shift_erase_pad_removes() {
    let (mut s, v) = (Surface::new(), view());
    let now = Instant::now();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SELECT");
    hold(&mut s, &v, "BUTTON_TRANSPORT_ERASE", true);
    assert_eq!(s.event(&Event::PadOn(1, 0.8), &v, now), [Action::Lfo("remove", 1, 0.0)]);
    assert!(s.event(&Event::PadOn(3, 0.8), &v, now).is_empty(), "an empty pad");
    hold(&mut s, &v, "BUTTON_TRANSPORT_ERASE", false);
    assert_eq!(s.event(&Event::PadOn(1, 0.8), &v, now), [Action::Edit("undo")], "SHIFT + pad 1: undo");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), "lfo");
    assert_eq!(s.error(), None);
}

/// With SHIFT held, display button 8 reads DELETE and removes the focused LFO.
#[test]
fn lfo_page_shift_shows_delete() {
    let (mut s, mut v) = (Surface::new(), view());
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SELECT");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    let delete = "DELETE".to_string();
    assert!(!s.screen_text(&v).contains(&delete));
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    let text = s.screen_text(&v);
    assert!(text.contains(&delete) && !text.contains(&"TRIG".to_string()), "{text:?}");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Lfo("remove", 1, 0.0)]);
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert!(!s.screen_text(&v).contains(&delete));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Lfo("trigger_focused", 1, 1.0)], "TRIG again");
    v.lfo_focus = 0;
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert!(s.screen_text(&v).contains(&delete), "shown, disabled");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_8").is_empty(), "none focused");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), "lfo");
    assert_eq!(s.error(), None);
}

#[test]
fn lfo_page_draws_the_name() {
    let (mut s, mut v) = (Surface::new(), view());
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SELECT");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    let text = s.screen_text(&v);
    assert!(text.contains(&"LFO 1".to_string()), "{text:?}");
    assert!(text.contains(&"SINE".to_string()) && text.contains(&"TRIG".to_string()), "{text:?}");
    v.lfo_focus = 0;
    assert!(s.screen_text(&v).contains(&"Press a pad".to_string()));
    for theme in s.themes() {
        s.set_theme(&theme);
        s.picture(&v);
        assert_eq!(s.error(), None, "{theme}");
    }
}

/// In Envelope mode: ENV lit, the shapes dark and inert, SYNC is RETRIG, the knobs are
/// Attack, Decay, Sustain, Release, Level, Quantize, Trigger, and TRIG never turns the
/// trigger off; SHIFT still shows DELETE.
#[test]
fn lfo_page_in_envelope_mode() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SELECT");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.leds(&v).get("LED_SCREEN_BUTTON_7"), Some(deckard_mk3::leds::DIM), "LFO mode");
    let l = &mut v.lfos[0];
    (l.env, l.name, l.trigger, l.retrigger) = (true, "ENV 1".into(), "A1 / S3".into(), true);
    (l.attack, l.decay, l.release, l.sustain) = ("12 ms".into(), "300 ms".into(), "1.20 s".into(), 0.5);
    l.note_off = Some(0.6);
    let text = s.screen_text(&v);
    for t in ["ENV 1", "RETRIG", "ENV", "ATTACK", "DECAY", "SUSTAIN", "RELEASE", "12 ms", "1.20 s"] {
        assert!(text.contains(&t.to_string()), "{t}: {text:?}");
    }
    for t in ["SINE", "SYNC", "PHASE", "RATE", "LFO 1"] {
        assert!(!text.contains(&t.to_string()), "{t}: {text:?}");
    }
    let leds = s.leds(&v);
    assert_eq!(leds.get("LED_SCREEN_BUTTON_7"), Some(crate::leds::ON), "ENV lit");
    assert_eq!(leds.get("LED_SCREEN_BUTTON_1"), Some(deckard_mk3::leds::BACKLIGHT), "the shapes dark");
    assert_eq!(leds.get("LED_SCREEN_BUTTON_6"), Some(crate::leds::ON), "RETRIG on");
    assert!(press(&mut s, &v, "BUTTON_SCREEN_1").is_empty(), "no shapes");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_6"), [Action::Lfo("retrigger", 1, 0.0)]);
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_7"), [Action::Lfo("env", 1, 0.0)], "back to LFO");
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Lfo("trigger_focused", 1, 1.0)], "TRIG: the focused sound");
    let knob = |s: &mut Surface, n| s.event(&Event::Knob(n, 0.1), &v, now);
    assert_eq!(knob(&mut s, 0), [Action::Lfo("attack", 1, 1.0)]);
    assert_eq!(knob(&mut s, 1), [Action::Lfo("decay", 1, 1.0)]);
    assert_eq!(knob(&mut s, 2), [Action::Lfo("sustain", 1, 1.0)]);
    assert_eq!(knob(&mut s, 3), [Action::Lfo("release", 1, 1.0)]);
    assert_eq!(knob(&mut s, 4), [Action::Lfo("level", 1, 1.0)]);
    assert_eq!(knob(&mut s, 5), [Action::Lfo("quantize", 1, 1.0)]);
    assert_eq!(knob(&mut s, 6), [Action::Lfo("trigger", 1, 1.0)]);
    assert!(knob(&mut s, 7).is_empty());
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert!(s.screen_text(&v).contains(&"DELETE".to_string()));
    assert_eq!(press(&mut s, &v, "BUTTON_SCREEN_8"), [Action::Lfo("remove", 1, 0.0)]);
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    for theme in s.themes() {
        s.set_theme(&theme);
        s.picture(&v);
        assert_eq!(s.error(), None, "{theme}");
    }
}

/// SHIFT + SOLO shows the mod matrix page; SOLO alone still shows the hold page. As it
/// goes, the slot being made is dropped.
#[test]
fn shift_solo_opens_the_mod_matrix() {
    let (mut s, v) = (Surface::new(), view());
    hold(&mut s, &v, "BUTTON_SOLO", true);
    assert_eq!(s.page(), "hold", "SOLO alone");
    hold(&mut s, &v, "BUTTON_SOLO", false);
    assert_eq!(s.page(), FIRST_PAGE);
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    assert!(press(&mut s, &v, "BUTTON_SOLO").is_empty());
    assert_eq!(s.page(), "mod_matrix");
    assert_eq!(press(&mut s, &v, "BUTTON_SOLO"), [Action::ModSlotsClosed], "again: closed");
    assert_eq!(s.page(), FIRST_PAGE);
    press(&mut s, &v, "BUTTON_SOLO");
    assert_eq!(press(&mut s, &v, "BUTTON_SELECT"), [Action::ModSlotsClosed], "the LFO page in its place");
    assert_eq!(s.page(), "lfo");
    press(&mut s, &v, "BUTTON_SOLO");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(press(&mut s, &v, "BUTTON_MIX"), [Action::ModSlotsClosed], "the Mixer in its place");
    assert_eq!(s.page(), "mixer");
    assert_eq!(s.error(), None);
}

#[test]
fn mod_matrix_buttons_move_rows_and_knobs_step() {
    let (mut s, mut v) = (Surface::new(), view());
    let now = Instant::now();
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SOLO");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert!(press(&mut s, &v, "BUTTON_SCREEN_1").is_empty());
    press(&mut s, &v, "BUTTON_SCREEN_1");
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::ModSlot(0, 2, 1)], "slot 1's Sound");
    for _ in 0..4 {
        press(&mut s, &v, "BUTTON_SCREEN_1");
    }
    assert_eq!(s.event(&Event::Knob(0, -0.1), &v, now), [Action::ModSlot(0, 0, -1)], "wrapped to Source");
    assert!(s.event(&Event::Knob(4, 0.1), &v, now).is_empty(), "no slot 5");
    assert!(press(&mut s, &v, "BUTTON_RIGHT").is_empty());
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::ModSlot(0, 0, 1)], "no more slots: the same eight");
    v.mod_slots = vec![v.mod_slots[0].clone(); 10];
    assert!(press(&mut s, &v, "BUTTON_RIGHT").is_empty());
    assert_eq!(s.event(&Event::Knob(0, 0.1), &v, now), [Action::ModSlot(8, 0, 1)], "slot 9");
    press(&mut s, &v, "BUTTON_LEFT");
    hold(&mut s, &v, "BUTTON_TRANSPORT_ERASE", true);
    assert_eq!(s.event(&Event::Knob(1, 0.01), &v, now), [Action::ModSlot(1, 0, 0)], "ERASE + a knob");
    assert!(s.event(&Event::Knob(1, 0.01), &v, now).is_empty(), "once a press");
    hold(&mut s, &v, "BUTTON_TRANSPORT_ERASE", false);
    assert_eq!(s.error(), None);
}

#[test]
fn mod_matrix_draws_slots() {
    let (mut s, v) = (Surface::new(), view());
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SOLO");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    let text = s.screen_text(&v);
    for t in ["SLOT 1", "SLOT 3", "LFO 1", "Pressure", "+50%", "SOURCE"] {
        assert!(text.contains(&t.to_string()), "{t}: {text:?}");
    }
    for theme in s.themes() {
        s.set_theme(&theme);
        s.picture(&v);
        assert_eq!(s.error(), None, "{theme}");
    }
}

/// A CC slot's rows read Port, Channel and CC (on the screen and the display buttons).
#[test]
fn mod_matrix_names_a_cc_slots_rows() {
    let (mut s, mut v) = (Surface::new(), view());
    v.mod_slots[1] = ModSlotView { rows: ["LFO 2", "MIDI", "IAC Driver Bus 1", "3", "74", "+50%"].map(String::from), midi: true };
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SOLO");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    for _ in 0..2 {
        press(&mut s, &v, "BUTTON_SCREEN_2");
    }
    let text = s.screen_text(&v);
    for t in ["PORT", "CHANNEL", "CC", "MIDI", "IAC Driver Bus 1", "SOUND", "MODULE", "DESTINATION"] {
        assert!(text.contains(&t.to_string()), "{t}: {text:?}");
    }
    assert_eq!(text.iter().filter(|t| *t == "PORT").count(), 2, "the row and its button: {text:?}");
    // CC (0-119) takes a fiftieth of a turn a step, as Depth; Channel a tenth.
    let now = Instant::now();
    press(&mut s, &v, "BUTTON_SCREEN_2");
    assert!(s.event(&Event::Knob(1, 0.02), &v, now).is_empty(), "Channel: not yet a step");
    press(&mut s, &v, "BUTTON_SCREEN_2");
    assert_eq!(s.event(&Event::Knob(1, 0.02), &v, now), [Action::ModSlot(1, 4, 1)], "CC");
    assert_eq!(s.error(), None);
}

/// A page shown while a button is held over the mod matrix doesn't drop its slot being
/// made: it's only dropped when the page goes for good.
#[test]
fn mod_matrix_keeps_its_draft_under_a_hold_page() {
    let (mut s, v) = (Surface::new(), view());
    hold(&mut s, &v, "BUTTON_SHIFT", true);
    press(&mut s, &v, "BUTTON_SOLO");
    hold(&mut s, &v, "BUTTON_SHIFT", false);
    assert_eq!(s.page(), "mod_matrix");
    for button in ["BUTTON_MUTE", "BUTTON_SOLO", "BUTTON_SELECT", "BUTTON_PATTERN", "BUTTON_SCENE"] {
        let acts = press(&mut s, &v, button);
        assert!(!acts.contains(&Action::ModSlotsClosed), "{button}: {acts:?}");
        assert_eq!(s.page(), "mod_matrix", "{button}: back");
    }
    // Pinned, then given way to MIX: dropped, once.
    let mut acts = hold(&mut s, &v, "BUTTON_MUTE", true);
    acts.extend(press(&mut s, &v, "BUTTON_SCREEN_1"));
    acts.extend(hold(&mut s, &v, "BUTTON_MUTE", false));
    assert_eq!(s.page(), "hold", "pinned");
    acts.extend(press(&mut s, &v, "BUTTON_MIX"));
    assert_eq!(s.page(), "mixer");
    assert_eq!(acts.iter().filter(|a| **a == Action::ModSlotsClosed).count(), 1, "{acts:?}");
    assert_eq!(s.error(), None);
}
