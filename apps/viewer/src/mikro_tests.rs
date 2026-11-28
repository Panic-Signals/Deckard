//! The Maschine Mikro MK3's script driving a real project: what it asks the app for does what it
//! should. Hardware-free, as `mikro_events` takes made-up events; `App::mikro` stays `None`, so
//! nothing opens a device.

use crate::mikro_io::{DEVICE, coalesced};
use deckard_mikro::{Event, Screen, leds::Leds};
use deckard_surface::device::{Device, PadMap};
use std::time::Instant;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    (app.grid, app.scale) = (deckard_model::PPQ / 4, (60, 0, 0));
    let view = app.surface_view();
    let mut d = Device::new(DEVICE, None);
    assert_eq!(d.error(), None);
    d.open(&view, Instant::now());
    app.mikro_device = Some(d);
    app
}

fn run(app: &mut crate::App, events: Vec<Event>) -> (Leds, Screen, PadMap) {
    let res = app.mikro_events(events, Instant::now());
    assert_eq!(app.mikro_error, None);
    res
}

fn press(app: &mut crate::App, name: &'static str) {
    run(app, vec![Event::Button(name, true)]);
    run(app, vec![Event::Button(name, false)]);
}

#[test]
fn mikro_coalesced_strip_moves() {
    let events = vec![
        Event::Strip(0, 0.1),
        Event::Strip(1, 0.2),
        Event::Strip(1, 0.3),
        Event::Strip(1, 0.4),
        Event::Strip(2, 0.4),
    ];
    let c = coalesced(events);
    assert_eq!(c.len(), 3);
    assert_eq!(c[0], Event::Strip(0, 0.1));
    assert_eq!(c[1], Event::Strip(1, 0.4));
    assert_eq!(c[2], Event::Strip(2, 0.4));
}

#[test]
fn mikro_mode_switching_updates_leds_and_screen() {
    let mut app = app();
    let (leds, _screen, route) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_PAD_MODE"), Some(deckard_mikro::leds::ON));
    assert_eq!(route.pads.iter().filter(|p| !p.is_empty()).count(), 16);

    // Press KEYBOARD button
    press(&mut app, "BUTTON_KEYBOARD");
    let (leds, _screen, _) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_KEYBOARD"), Some(deckard_mikro::leds::ON));
    assert_eq!(leds.get("LED_PAD_MODE"), Some(deckard_mikro::leds::DIM));
}

#[test]
fn mikro_pad_hits_focus_sounds() {
    let mut app = app();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    doc.borrow_mut().add_slot(0, 2, 0, &deckard_doc::NewModule::Sampler).unwrap();

    // In pad mode, hitting loaded pad 3 focuses sound 3 (0-based index 2).
    run(&mut app, vec![Event::PadOn(3, 0.8), Event::PadOff(3)]);
    assert_eq!(app.sound, 2, "focused sound should be 0-based index 2 (sound 3)");

    // Hitting empty pad 5 must NOT change focus away from sound 3.
    run(&mut app, vec![Event::PadOn(5, 0.8), Event::PadOff(5)]);
    assert_eq!(app.sound, 2, "empty pad hit must not focus sound");
}

#[test]
fn mikro_transport_controls() {
    let mut app = app();
    // Record toggle
    assert!(!app.recording);
    press(&mut app, "BUTTON_TRANSPORT_RECORD");
    assert!(app.recording);
    press(&mut app, "BUTTON_TRANSPORT_RECORD");
    assert!(!app.recording);

    // Follow toggle
    assert!(!app.follow);
    press(&mut app, "BUTTON_FOLLOW");
    assert!(app.follow);
    press(&mut app, "BUTTON_FOLLOW");
    assert!(!app.follow);

    // Loop toggle with Shift + Restart
    assert!(!app.looping);
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", true)]);
    press(&mut app, "BUTTON_TRANSPORT_LOOP");
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", false)]);
    assert!(app.looping);

    // Play action attempted
    press(&mut app, "BUTTON_TRANSPORT_PLAY");
    assert!(app.playing.is_some() || app.status.contains("audio"));
    press(&mut app, "BUTTON_TRANSPORT_STOP");
    assert!(app.playing.is_none());
}

#[test]
fn mikro_step_knob_moves_discrete_and_continuous_groove_params() {
    use deckard_surface::Action as A;
    let mut app = app();
    app.update_slot_view();
    app.surface_act(A::GroovePage(1)); // Variation page

    let get = |app: &crate::App, f: &str| app.groove_setting(0, 0, f).unwrap_or(0.0);

    // Knob 8 (0-based 7): Live toggle flips in 1 detent click.
    assert_eq!(get(&app, "sequence_variation/live"), 0.0);
    app.surface_act(A::StepKnob { knob: 7, steps: 1, fine: false });
    assert_eq!(get(&app, "sequence_variation/live"), 1.0, "Live should flip on in 1 click");
    app.surface_act(A::StepKnob { knob: 7, steps: -1, fine: false });
    assert_eq!(get(&app, "sequence_variation/live"), 0.0, "Live should flip off in 1 click");

    // Knob 7 (0-based 6): Distribution choice (default 1 = Natural) steps in 1 click.
    let dist_before = app.groove_setting(0, 0, "sequence_variation/distribution").unwrap_or(1.0);
    app.surface_act(A::StepKnob { knob: 6, steps: 1, fine: false });
    assert_eq!(
        app.groove_setting(0, 0, "sequence_variation/distribution").unwrap_or(1.0),
        dist_before + 1.0,
        "Distribution choice should advance in 1 click"
    );

    // Knob 1 (0-based 0): Velocity Amount (0..100 %) moves +5% normal, +1% fine.
    assert_eq!(get(&app, "sequence_variation/velocity_amount"), 0.0);
    app.surface_act(A::StepKnob { knob: 0, steps: 1, fine: false });
    assert_eq!(get(&app, "sequence_variation/velocity_amount"), 5.0);
    app.surface_act(A::StepKnob { knob: 0, steps: 1, fine: true });
    assert_eq!(get(&app, "sequence_variation/velocity_amount"), 6.0);
}

#[test]
fn mikro_groove_apply_triggers_variation() {
    use deckard_surface::Action as A;
    let mut app = app();
    app.update_slot_view();
    // Create a pattern with 4 notes on sound 0 at velocity 100.
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_pattern(0, 0, "P").unwrap();
    for step in 0..4 {
        app.surface_act(A::StepToggle { at: step * 240, length: 240, velocity: 100 });
    }
    app.surface_act(A::GroovePage(1)); // Variation
    app.surface_act(A::StepKnob { knob: 0, steps: 10, fine: false }); // Velocity Amount = 50%
    app.surface_act(A::GrooveApply);

    let d = doc.borrow();
    let notes = &d.project().groups[0].patterns[0].notes;
    assert_eq!(notes.len(), 4);
    assert!(
        notes.iter().any(|n| n.velocity != 100),
        "GrooveApply on Variation with 50% velocity amount should vary note velocities"
    );
}

#[test]
fn mikro_browser_view_populated_when_open() {
    use deckard_surface::{Action as A, Browse};
    let mut app = app();
    app.surface_act(A::Library(true));
    app.surface_act(A::Browse(Browse::Kind(5))); // Loops (0-based)
    let view = app.surface_view();
    assert_eq!(view.browser.kind, 5, "surface_view must populate browser when mikro_device is open and browser.open is true");
}

#[test]
fn mikro_fixed_velocity_and_sixteen_levels_drive_hit_velocity() {
    let mut app = app();
    // New projects open with fixed_velocity = true, velocity = 100.
    let (_leds, _screen, route) = run(&mut app, vec![]);
    assert!(route.fixed_velocity);
    let (_notes, vel_soft) = crate::mikro_io::hit(Some(&route), 1, 0.1, 0);
    let (_notes, vel_hard) = crate::mikro_io::hit(Some(&route), 1, 0.95, 0);
    assert_eq!(vel_soft, 100);
    assert_eq!(vel_hard, 100);

    // Toggle Fixed Velocity OFF -> soft vs hard pressure yields different velocities.
    press(&mut app, "BUTTON_FIX_VELOCITY");
    let (_leds, _screen, route) = run(&mut app, vec![]);
    assert!(!route.fixed_velocity);
    let (_notes, vel_soft) = crate::mikro_io::hit(Some(&route), 1, 0.1, 0);
    let (_notes, vel_hard) = crate::mikro_io::hit(Some(&route), 1, 0.95, 0);
    assert!(vel_hard > vel_soft, "pressure velocity should differ when Fixed Velocity is off");

    // SHIFT + FIXED VEL -> 16 Velocities (pad 1 = 8, pad 16 = 127 regardless of pressure).
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", true)]);
    press(&mut app, "BUTTON_FIX_VELOCITY");
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", false)]);
    let (_leds, _screen, route) = run(&mut app, vec![]);
    let (_n1, v1) = crate::mikro_io::hit(Some(&route), 1, 0.95, 0);
    let (_n16, v16) = crate::mikro_io::hit(Some(&route), 16, 0.1, 0);
    assert_eq!(v1, 8);
    assert_eq!(v16, 127);
}

#[test]
fn mikro_browse_toggles_app_browser_panel() {
    let mut app = app();
    assert!(!app.browser.open);
    press(&mut app, "BUTTON_BROWSE");
    assert!(app.browser.open, "BROWSE toggle should open library panel");
    press(&mut app, "BUTTON_BROWSE");
    assert!(!app.browser.open, "second BROWSE press should close library panel");
}

#[test]
fn mikro_shift_duplicate_doubles_real_pattern() {
    use deckard_surface::Action as A;
    let mut app = app();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_pattern(0, 0, "P").unwrap();
    let before_len = doc.borrow().project().groups[0].patterns[0].length;
    app.surface_act(A::StepToggle { at: 0, length: 240, velocity: 100 });
    assert_eq!(doc.borrow().project().groups[0].patterns[0].notes.len(), 1);

    // SHIFT + DUPLICATE doubles the focused pattern.
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", true)]);
    press(&mut app, "BUTTON_DUPLICATE");
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", false)]);

    let d = doc.borrow();
    let pat = &d.project().groups[0].patterns[0];
    assert_eq!(pat.length, before_len * 2, "pattern length should double");
    assert_eq!(pat.notes.len(), 2, "notes should be duplicated into the second half");
}

#[test]
fn mikro_scene_and_pattern_create_in_app() {
    let mut app = app();
    let doc = app.doc.clone().unwrap();
    let initial_scenes = doc.borrow().project().scenes.len();

    // Hold SCENE + press pad (initial_scenes + 1) creates a new scene.
    run(&mut app, vec![Event::Button("BUTTON_SCENE", true)]);
    run(&mut app, vec![Event::PadOn(initial_scenes + 1, 0.8), Event::PadOff(initial_scenes + 1)]);
    run(&mut app, vec![Event::Button("BUTTON_SCENE", false)]);
    assert_eq!(doc.borrow().project().scenes.len(), initial_scenes + 1);

    // Hold PATTERN + press pad 2 creates a new pattern in slot 2.
    let initial_pats = doc.borrow().project().groups[0].patterns.len();
    run(&mut app, vec![Event::Button("BUTTON_PATTERN", true)]);
    run(&mut app, vec![Event::PadOn(2, 0.8), Event::PadOff(2)]);
    run(&mut app, vec![Event::Button("BUTTON_PATTERN", false)]);
    assert_eq!(doc.borrow().project().groups[0].patterns.len(), initial_pats + 1);
}

#[test]
fn mikro_variation_encoder_and_apply_end_to_end() {
    use deckard_surface::Action as A;
    let mut app = app();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_pattern(0, 0, "P").unwrap();
    for step in 0..4 {
        app.surface_act(A::StepToggle { at: step * 240, length: 240, velocity: 100 });
    }

    // Press VARIATION -> enters Variation mode and selects Groove ▸ Variation.
    press(&mut app, "BUTTON_VARIATION");
    // Turn encoder +10 on knob 1 (Velocity Amount -> 50%).
    run(&mut app, vec![Event::EncoderTurn(10)]);
    assert_eq!(app.groove_setting(0, 0, "sequence_variation/velocity_amount"), Some(50.0));

    // Push encoder -> GrooveApply varies the notes.
    press(&mut app, "BUTTON_ENCODER_PRESS");
    let d = doc.borrow();
    let notes = &d.project().groups[0].patterns[0].notes;
    assert_eq!(notes.len(), 4);
    assert!(notes.iter().any(|n| n.velocity != 100), "pushing encoder in Variation should vary note velocities");
}

#[test]
fn mikro_empty_pads_are_off_and_loaded_pads_flash_then_settle() {
    use deckard_mikro::leds::{SHADE_BRIGHT, SHADE_FLASH, colour};

    let mut app = app();
    let (leds, _screen, _route) = run(&mut app, vec![]);
    // New project: all sounds empty -> every pad is off (0), including slot 1.
    for pad in 1..=16 {
        assert_eq!(leds.get(&format!("LED_PAD_{pad}")), Some(0), "empty pad {pad} must be dark");
    }

    // Hitting empty pad 5 does NOT select sound 5 or light/flash pad 5.
    let (leds, _screen, _route) = run(&mut app, vec![Event::PadOn(5, 0.8)]);
    assert_eq!(app.sound, 0);
    assert_eq!(leds.get("LED_PAD_5"), Some(0));
    run(&mut app, vec![Event::PadOff(5)]);

    // Loading a sampler onto sound 1: holding pad 1 flashes SHADE_FLASH and returns to SHADE_BRIGHT on release.
    app.doc.clone().unwrap().borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    let group_color = app.surface_view().groups[0].color as i64;
    let (leds, _screen, _route) = run(&mut app, vec![Event::PadOn(1, 0.8)]);
    assert_eq!(leds.get("LED_PAD_1"), Some(colour(group_color, SHADE_FLASH)));
    let (leds, _screen, _route) = run(&mut app, vec![Event::PadOff(1)]);
    assert_eq!(leds.get("LED_PAD_1"), Some(colour(group_color, SHADE_BRIGHT)));
}

#[test]
fn mikro_shift_mute_triggers_choke_without_muting_pads() {
    let mut app = app();
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", true)]);
    let (leds, _screen, _route) = run(&mut app, vec![Event::Button("BUTTON_MUTE", true)]);
    assert_eq!(leds.get("LED_MUTE"), Some(deckard_mikro::leds::DIM));
    let (leds, _screen, route) = run(&mut app, vec![Event::Button("BUTTON_MUTE", false), Event::Button("BUTTON_SHIFT", false)]);
    assert_eq!(leds.get("LED_MUTE"), Some(deckard_mikro::leds::DIM));
    assert_eq!(route.pads.iter().filter(|p| !p.is_empty()).count(), 16, "SHIFT+MUTE choke must not latch MUTE");
}

#[test]
fn mikro_plugin_button_loads_module_edits_param_bypasses_and_opens_macro() {
    let mut app = app();
    let doc = app.doc.clone().unwrap();

    // Press PLUG-IN -> enters plug-in view in app.
    press(&mut app, "BUTTON_PLUGIN");
    assert!(!app.channel, "PLUG-IN must switch viewer control panel to plug-ins");
    assert!(app.surface_view().control.modules.iter().any(|m| m.name == "EQ"));

    // Push encoder -> Module Loader; Sound 1 is empty, starts at KIND=INST, item 1 (Sampler).
    press(&mut app, "BUTTON_ENCODER_PRESS");
    press(&mut app, "BUTTON_ENCODER_PRESS");
    assert!(
        !doc.borrow().project().groups[0].sounds[0].slots.is_empty(),
        "pushing encoder twice on empty Sound 1 should load Sampler into slot 1"
    );

    // Move to "+" slot (Hold PLUG-IN + turn +1) and load the first internal effect (Compressor).
    run(&mut app, vec![Event::Button("BUTTON_PLUGIN", true)]);
    run(&mut app, vec![Event::EncoderTurn(1)]);
    run(&mut app, vec![Event::Button("BUTTON_PLUGIN", false)]);
    assert_eq!(app.slot, None, "should advance to the + slot");
    press(&mut app, "BUTTON_ENCODER_PRESS");
    press(&mut app, "BUTTON_ENCODER_PRESS");
    {
        let d = doc.borrow();
        let slots = &d.project().groups[0].sounds[0].slots;
        assert_eq!(slots.len(), 2, "first internal effect should load into slot 2");
        assert!(slots[1].active);
    }

    // Turning encoder in PLUG-IN mode tweaks the focused effect knob (text and/or value changes).
    let knob_state = |app: &crate::App| {
        app.surface_view().control.knobs.iter().flatten().next().map(|k| (k.text.clone(), k.value)).unwrap()
    };
    let before_knob = knob_state(&app);
    run(&mut app, vec![Event::EncoderTurn(1)]);
    let after_knob = knob_state(&app);
    assert_ne!(before_knob, after_knob, "turning encoder in PLUG-IN mode must edit the focused parameter");

    // Hold PLUG-IN + MUTE -> bypasses focused slot 2.
    run(&mut app, vec![Event::Button("BUTTON_PLUGIN", true)]);
    press(&mut app, "BUTTON_MUTE");
    run(&mut app, vec![Event::Button("BUTTON_PLUGIN", false)]);
    assert!(!doc.borrow().project().groups[0].sounds[0].slots[1].active, "PLUG-IN + MUTE should bypass slot");

    // SHIFT + PLUG-IN -> opens Channel ▸ Macro.
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", true)]);
    press(&mut app, "BUTTON_PLUGIN");
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", false)]);
    assert!(app.channel);
    assert_eq!(app.prop_set, 3, "SHIFT + PLUG-IN must open Channel Macro (prop_set = 3)");
}

#[test]
fn mikro_browse_screen_renders_live_deckard_results_and_kind_changes() {
    use deckard_library::{Item, Kind};

    let mut app = app();
    app.browser.open = true;
    app.browser.query.kind = Kind::OneShot;
    app.browser.results.items = vec![
        Item { name: "ASURA kick 1".into(), kind: Kind::OneShot, ..Default::default() },
        Item { name: "Snare 02".into(), kind: Kind::OneShot, favorite: true, ..Default::default() },
    ];
    app.browser.selected = Some(0);

    // Press BROWSE on Mikro -> screen returned from mikro_events MUST show live results, not (NO RESULTS)/PROJECTS!
    press(&mut app, "BUTTON_BROWSE");
    let (_leds, screen, _route) = run(&mut app, vec![]);
    let mut want = Screen::new();
    want.status_bar("BROWSE", "ONE-SHOTS");
    want.draw_text(13, 2, "> RESULTS: 1/2", true);
    want.draw_text(23, 2, "ASURA kick 1", true);
    assert!(screen == want, "Mikro OLED must show Deckard's live browse kind and focused result");

    // Encoder +1 moves focus to item 2 ("* Snare 02") -> screen updates immediately.
    let (_leds, screen, _route) = run(&mut app, vec![Event::EncoderTurn(1)]);
    let mut want_next = Screen::new();
    want_next.status_bar("BROWSE", "ONE-SHOTS");
    want_next.draw_text(13, 2, "> RESULTS: 2/2", true);
    want_next.draw_text(23, 2, "* Snare 02", true);
    assert!(screen == want_next, "Mikro OLED must update when scrolling browse results");

    // Changing kind in Deckard (e.g. to Sounds) reflects on Mikro OLED immediately.
    app.browser.query.kind = Kind::Sound;
    let (_leds, screen, _route) = run(&mut app, vec![]);
    let mut want_sounds = Screen::new();
    want_sounds.status_bar("BROWSE", "SOUNDS");
    want_sounds.draw_text(13, 2, "> RESULTS: 2/2", true);
    want_sounds.draw_text(23, 2, "* Snare 02", true);
    assert!(screen == want_sounds, "Mikro OLED must reflect kind change in Deckard");
}

#[test]
fn mikro_complete_workflows_group_controls_and_shift_shortcuts_end_to_end() {
    let mut app = app();
    let doc = app.doc.clone().unwrap();

    // GROUP + pad 2 (#groups+1) adds group 2; ERASE + GROUP + pad 2 deletes it.
    run(&mut app, vec![Event::Button("BUTTON_GROUP", true), Event::PadOn(2, 0.8), Event::PadOff(2), Event::Button("BUTTON_GROUP", false)]);
    assert_eq!(doc.borrow().project().groups.len(), 2, "GROUP + next pad should add a group");

    run(&mut app, vec![
        Event::Button("BUTTON_TRANSPORT_ERASE", true),
        Event::Button("BUTTON_GROUP", true),
        Event::PadOn(2, 0.8),
        Event::PadOff(2),
        Event::Button("BUTTON_GROUP", false),
        Event::Button("BUTTON_TRANSPORT_ERASE", false),
    ]);
    assert_eq!(doc.borrow().project().groups.len(), 1, "ERASE + GROUP + pad should delete group");

    // SAMPLING, AUTO, LOCK, NOTE_REPEAT, SHIFT+SCENE (Song View), SHIFT+FOLLOW (Grid)
    assert!(!app.sampling_open);
    press(&mut app, "BUTTON_SAMPLING");
    assert!(app.sampling_open);
    press(&mut app, "BUTTON_SAMPLING");
    assert!(!app.sampling_open);

    press(&mut app, "BUTTON_AUTO");
    assert!(app.auto_write);

    press(&mut app, "BUTTON_LOCK");
    assert!(app.surface_view().lock.active);

    press(&mut app, "BUTTON_NOTE_REPEAT");
    assert!(app.repeat_soft);

    run(&mut app, vec![
        Event::Button("BUTTON_SHIFT", true),
        Event::Button("BUTTON_SCENE", true),
        Event::Button("BUTTON_SCENE", false),
        Event::Button("BUTTON_SHIFT", false),
    ]);
    assert!(app.song_view);
    run(&mut app, vec![
        Event::Button("BUTTON_SHIFT", true),
        Event::Button("BUTTON_SCENE", true),
        Event::Button("BUTTON_SCENE", false),
        Event::Button("BUTTON_SHIFT", false),
    ]);
    assert!(!app.song_view);

    let before_grid = app.grid;
    run(&mut app, vec![
        Event::Button("BUTTON_SHIFT", true),
        Event::Button("BUTTON_FOLLOW", true),
        Event::Button("BUTTON_FOLLOW", false),
        Event::Button("BUTTON_SHIFT", false),
        Event::PadOn(3, 0.8), // 1/4 step grid
        Event::PadOff(3),
    ]);
    assert_ne!(app.grid, before_grid, "Grid overlay pad pick must update step grid");
    press(&mut app, "BUTTON_FOLLOW");

    // SHIFT + SWING (POSITION) scrubs parked playhead.
    run(&mut app, vec![
        Event::Button("BUTTON_SWING", true),
        Event::Button("BUTTON_SHIFT", true),
        Event::EncoderTurn(2),
        Event::Button("BUTTON_SHIFT", false),
        Event::Button("BUTTON_SWING", false),
    ]);
    assert!(app.parked.is_some_and(|p| p.position > 0.0), "SHIFT + SWING should scrub playhead");

    // SHIFT + VOLUME (VELOCITY) updates velocity.
    let v0 = app.surface_view().velocity;
    run(&mut app, vec![
        Event::Button("BUTTON_VOLUME", true),
        Event::Button("BUTTON_SHIFT", true),
        Event::EncoderTurn(1),
        Event::Button("BUTTON_SHIFT", false),
        Event::Button("BUTTON_VOLUME", false),
    ]);
    assert_eq!(app.surface_view().velocity, (v0 + 5).clamp(1, 127));

    // SHIFT + Pad 14 transposes notes +1 semitone, SHIFT + Pad 1 undoes it.
    doc.borrow_mut().add_pattern(0, 0, "P").unwrap();
    doc.borrow_mut()
        .add_note(0, 0, deckard_model::Note { ratchet: Default::default(), expr: None, sound: 0, position: 0, length: 240, pitch: 60, velocity: 100, probability: 100 })
        .unwrap();
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", true), Event::PadOn(14, 0.8), Event::PadOff(14), Event::Button("BUTTON_SHIFT", false)]);
    assert_eq!(doc.borrow().project().groups[0].patterns[0].notes[0].pitch, 61);
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", true), Event::PadOn(1, 0.8), Event::PadOff(1), Event::Button("BUTTON_SHIFT", false)]);
    assert_eq!(doc.borrow().project().groups[0].patterns[0].notes[0].pitch, 60);
}

#[test]
fn mikro_browse_encoder_scrolls_past_16_wrap_and_prehears_sounds() {
    use deckard_library::{Item, Kind};

    let mut app = app();
    app.browser.open = true;
    app.browser.query.kind = Kind::Sound;
    app.browser.results.items = (1..=30)
        .map(|i| Item { name: format!("Sound {i:02}"), kind: Kind::Sound, ..Default::default() })
        .collect();
    app.browser.selected = None;

    press(&mut app, "BUTTON_BROWSE");

    // Simulate raw 4-bit hardware HID encoder reports cycling 0..15 -> 0..9 (25 clockwise steps total).
    let mut dec = deckard_mikro::Decoder::new();
    let mut r = [0u8; 64];
    r[0] = 0x01;
    r[7] = 0;
    let mut dummy = Vec::new();
    dec.decode(&r, &mut dummy); // baseline 0

    for step in 1u8..=25 {
        r[7] = step & 0x0f;
        let mut ev = Vec::new();
        dec.decode(&r, &mut ev);
        run(&mut app, ev);
        assert_eq!(app.browser.selected, Some((step - 1) as usize), "step {step} should advance monotonically without wrapping back");
    }
    assert_eq!(app.browser.selected, Some(24));

    // Prehear on a fresh idle project starts an engine on Output if an output device is open
    // (previously Output::prehear silently dropped previews until a sound was loaded/played).
    let tmp = tempfile::tempdir().unwrap();
    let wav_path = tmp.path().join("preview.wav");
    let mut wav_bytes = Vec::new();
    // Minimal 1-sample 44100 Hz stereo 16-bit PCM WAV header + frame.
    wav_bytes.extend_from_slice(b"RIFF");
    wav_bytes.extend_from_slice(&(36u32 + 4).to_le_bytes());
    wav_bytes.extend_from_slice(b"WAVEfmt ");
    wav_bytes.extend_from_slice(&16u32.to_le_bytes());
    wav_bytes.extend_from_slice(&1u16.to_le_bytes());
    wav_bytes.extend_from_slice(&2u16.to_le_bytes());
    wav_bytes.extend_from_slice(&44100u32.to_le_bytes());
    wav_bytes.extend_from_slice(&(44100u32 * 4).to_le_bytes());
    wav_bytes.extend_from_slice(&4u16.to_le_bytes());
    wav_bytes.extend_from_slice(&16u16.to_le_bytes());
    wav_bytes.extend_from_slice(b"data");
    wav_bytes.extend_from_slice(&4u32.to_le_bytes());
    wav_bytes.extend_from_slice(&[0u8; 4]);
    std::fs::write(&wav_path, wav_bytes).unwrap();

    if let Ok(out) = deckard_engine::output::Output::new() {
        assert!(!out.has_engine(), "fresh output starts with no engine");
        app.output = Some(Ok(out));
        app.browser.query.kind = Kind::OneShot;
        app.browser.results.items = vec![
            Item { name: "A".into(), kind: Kind::OneShot, path: wav_path.clone(), ..Default::default() },
            Item { name: "B".into(), kind: Kind::OneShot, path: wav_path, ..Default::default() },
        ];
        app.browser.selected = Some(0);
        run(&mut app, vec![Event::EncoderTurn(1)]);
        assert!(app.output.as_ref().unwrap().as_ref().unwrap().has_engine(), "scrolling to prehear must start an idle engine");
    }
}

#[test]
fn mikro_stop_led_and_solo_sound_and_group() {
    let mut app = app();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_group().unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    doc.borrow_mut().add_slot(0, 1, 0, &deckard_doc::NewModule::Sampler).unwrap();

    // Stopped: PLAY dim, STOP backlight (off).
    let (leds, _screen, _route) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_TRANSPORT_PLAY"), Some(deckard_mikro::leds::DIM));
    assert_eq!(leds.get("LED_TRANSPORT_STOP"), Some(deckard_mikro::leds::BACKLIGHT));

    // Simulate active playback: STOP must remain lit while playing.
    app.playing = Some(deckard_engine::Mode::Arrangement);
    let (leds, _screen, _route) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_TRANSPORT_PLAY"), Some(deckard_mikro::leds::ON));
    assert_eq!(leds.get("LED_TRANSPORT_STOP"), Some(deckard_mikro::leds::DIM));
    app.playing = None;

    // SOLO + Pad 1 mutes sounds 2..16 in the group and leaves sound 1 unmuted; second tap unsolos.
    run(&mut app, vec![Event::Button("BUTTON_SOLO", true), Event::PadOn(1, 0.8), Event::PadOff(1), Event::Button("BUTTON_SOLO", false)]);
    {
        let d = doc.borrow();
        let sounds = &d.project().groups[0].sounds;
        assert!(!sounds[0].muted, "soloed sound 1 must stay unmuted");
        assert!(sounds[1..].iter().all(|s| s.muted), "all other sounds in group must be muted");
    }
    run(&mut app, vec![Event::Button("BUTTON_SOLO", true), Event::PadOn(1, 0.8), Event::PadOff(1), Event::Button("BUTTON_SOLO", false)]);
    assert!(doc.borrow().project().groups[0].sounds.iter().all(|s| !s.muted), "second solo tap unmutes all sounds");

    // SOLO + GROUP + Pad 1 mutes group 2 and leaves group 1 unmuted; second tap unsolos.
    run(&mut app, vec![
        Event::Button("BUTTON_GROUP", true),
        Event::Button("BUTTON_SOLO", true),
        Event::PadOn(1, 0.8),
        Event::PadOff(1),
        Event::Button("BUTTON_SOLO", false),
        Event::Button("BUTTON_GROUP", false),
    ]);
    {
        let d = doc.borrow();
        assert!(!d.project().groups[0].muted, "soloed group 1 must stay unmuted");
        assert!(d.project().groups[1].muted, "other groups must be muted");
    }
    run(&mut app, vec![
        Event::Button("BUTTON_GROUP", true),
        Event::Button("BUTTON_SOLO", true),
        Event::PadOn(1, 0.8),
        Event::PadOff(1),
        Event::Button("BUTTON_SOLO", false),
        Event::Button("BUTTON_GROUP", false),
    ]);
    assert!(doc.borrow().project().groups.iter().all(|g| !g.muted), "second group solo tap unmutes all groups");
}

#[test]
fn mikro_keyboard_mode_lights_arrows_and_shifts_octaves() {
    use deckard_mikro::leds::{BACKLIGHT, DIM};

    let mut app = app();
    let (leds, _screen, _route) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_LEFT"), Some(BACKLIGHT));
    assert_eq!(leds.get("LED_RIGHT"), Some(BACKLIGHT));

    press(&mut app, "BUTTON_KEYBOARD");
    let (leds, _screen, route) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_LEFT"), Some(DIM));
    assert_eq!(leds.get("LED_RIGHT"), Some(DIM));
    assert_eq!(route.pads[24].as_slice(), &[(0, 0, 60)]);

    press(&mut app, "BUTTON_RIGHT");
    let (leds, _screen, route) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_LEFT"), Some(DIM));
    assert_eq!(leds.get("LED_RIGHT"), Some(DIM));
    assert_eq!(route.pads[24].as_slice(), &[(0, 0, 72)]);
}

#[test]
fn mikro_step_sequencer_leds_match_group_color_and_dark_empty_pads() {
    use deckard_mikro::leds::{SHADE_BRIGHT, colour};

    let mut app = app();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
    let grp_color = app.surface_view().groups[0].color as i64;

    press(&mut app, "BUTTON_STEP");

    // Initially no steps: all 16 pads must be completely dark (0).
    let (leds, _screen, _route) = run(&mut app, vec![]);
    for pad in 1..=16 {
        assert_eq!(leds.get(&format!("LED_PAD_{pad}")), Some(0), "empty step pad {pad} must be off");
    }

    // Toggle step 1 and step 5 on.
    run(&mut app, vec![Event::PadOn(1, 0.8), Event::PadOff(1), Event::PadOn(5, 0.8), Event::PadOff(5)]);
    let (leds, _screen, _route) = run(&mut app, vec![]);
    for pad in 1..=16 {
        let want = if pad == 1 || pad == 5 { colour(grp_color, SHADE_BRIGHT) } else { 0 };
        assert_eq!(leds.get(&format!("LED_PAD_{pad}")), Some(want), "pad {pad} step color");
    }

    // Toggle step 1 back off -> returns to 0 while step 5 stays lit in group color.
    run(&mut app, vec![Event::PadOn(1, 0.8), Event::PadOff(1)]);
    let (leds, _screen, _route) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_PAD_1"), Some(0));
    assert_eq!(leds.get("LED_PAD_5"), Some(colour(grp_color, SHADE_BRIGHT)));
}

#[test]
fn mikro_shift_perform_adds_and_selects_perform_fx_and_touchstrip_works() {
    let mut app = app();
    assert_eq!(app.surface_view().groups[0].perform_mode, None);

    // SHIFT + PERFORM adds Perform FX to focused group and opens selector.
    run(&mut app, vec![
        Event::Button("BUTTON_SHIFT", true),
        Event::Button("BUTTON_PERFORM", true),
        Event::Button("BUTTON_PERFORM", false),
        Event::Button("BUTTON_SHIFT", false),
    ]);
    assert!(app.perform_slot(0).is_some(), "SHIFT + PERFORM must load Perform FX");
    assert_eq!(app.surface_view().groups[0].perform_mode, Some((0, "Filter".to_string())));

    // Tap Pad 3 (Burst Echo) and turn encoder +1 (Reso Echo) while selector is open.
    run(&mut app, vec![Event::PadOn(3, 0.8), Event::PadOff(3)]);
    assert_eq!(app.surface_view().groups[0].perform_mode, Some((2, "Burst Echo".to_string())));

    run(&mut app, vec![Event::EncoderTurn(1)]);
    assert_eq!(app.surface_view().groups[0].perform_mode, Some((3, "Reso Echo".to_string())));

    // Confirm with encoder press, then engage & move Perform strip.
    press(&mut app, "BUTTON_ENCODER_PRESS");
    let (leds, _screen, _route) = run(&mut app, vec![Event::Strip(0, 0.25), Event::Strip(1, 0.75)]);
    assert_eq!(leds.get("LED_PERFORM"), Some(deckard_mikro::leds::ON));
    assert_eq!(app.surface_view().groups[0].perform, Some((true, 0.75)));
    run(&mut app, vec![Event::Strip(2, 0.75)]);
    assert_eq!(app.surface_view().groups[0].perform, Some((false, 0.75)));
}

#[test]
fn mikro_quick_param_latch_and_sound_group_targeting() {
    use deckard_mikro::leds::{DIM, ON};

    let mut app = app();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_group().unwrap();
    doc.borrow_mut().add_slot(0, 1, 0, &deckard_doc::NewModule::Sampler).unwrap();
    doc.borrow_mut().add_slot(1, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();

    // Tap VOLUME -> latches on, holds Pad 2 to lower Sound 2 level, GROUP + Pad 2 to raise Group 2 level,
    // and tapping VOLUME again unlatches.
    press(&mut app, "BUTTON_VOLUME");
    let (leds, _screen, _) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_VOLUME"), Some(ON));

    let s2_before = doc.borrow().project().groups[0].sounds[1].level;
    run(&mut app, vec![Event::PadOn(2, 0.8), Event::EncoderTurn(-3), Event::PadOff(2)]);
    assert!(doc.borrow().project().groups[0].sounds[1].level < s2_before, "Pad 2 held in VOLUME lowers Sound 2 level");

    let g2_before = doc.borrow().project().groups[1].level;
    run(&mut app, vec![
        Event::Button("BUTTON_GROUP", true),
        Event::PadOn(2, 0.8),
        Event::EncoderTurn(3),
        Event::PadOff(2),
        Event::Button("BUTTON_GROUP", false),
    ]);
    assert!(doc.borrow().project().groups[1].level > g2_before, "GROUP + Pad 2 in VOLUME raises Group 2 level");

    press(&mut app, "BUTTON_VOLUME");
    let (leds, _screen, _) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_VOLUME"), Some(DIM));

    // SHIFT + TEMPO latches TUNE, Pad 2 held tunes Sound 2, GROUP + Pad 2 tunes Group 2.
    run(&mut app, vec![Event::Button("BUTTON_SHIFT", true), Event::Button("BUTTON_TEMPO", true), Event::Button("BUTTON_TEMPO", false), Event::Button("BUTTON_SHIFT", false)]);
    run(&mut app, vec![Event::PadOn(2, 0.8), Event::EncoderTurn(2), Event::PadOff(2)]);
    assert_eq!(doc.borrow().project().groups[0].sounds[1].sampler.as_ref().unwrap().tune, 2.0);

    run(&mut app, vec![
        Event::Button("BUTTON_GROUP", true),
        Event::PadOn(2, 0.8),
        Event::EncoderTurn(-4),
        Event::PadOff(2),
        Event::Button("BUTTON_GROUP", false),
    ]);
    assert_eq!(doc.borrow().project().groups[1].sounds[0].sampler.as_ref().unwrap().tune, -4.0);

    press(&mut app, "BUTTON_TEMPO");
    let (leds, _screen, _) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_TEMPO"), Some(DIM));
}

#[test]
fn mikro_tap_led_lit_and_shift_tap_toggles_metronome() {
    use deckard_mikro::leds::{DIM, ON};

    let mut app = app();
    let (leds, _screen, _) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_TAP"), Some(DIM));

    let (leds, _screen, _) = run(&mut app, vec![Event::Button("BUTTON_TAP", true)]);
    assert_eq!(leds.get("LED_TAP"), Some(ON));
    let (leds, _screen, _) = run(&mut app, vec![Event::Button("BUTTON_TAP", false)]);
    assert_eq!(leds.get("LED_TAP"), Some(DIM));

    run(&mut app, vec![
        Event::Button("BUTTON_SHIFT", true),
        Event::Button("BUTTON_TAP", true),
        Event::Button("BUTTON_TAP", false),
        Event::Button("BUTTON_SHIFT", false),
    ]);
    assert!(app.metronome);
    let (leds, _screen, _) = run(&mut app, vec![]);
    assert_eq!(leds.get("LED_TAP"), Some(ON));
}
