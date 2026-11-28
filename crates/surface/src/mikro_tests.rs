//! The Maschine Mikro MK3's script (`lua/devices/maschine_mikro_mk3/`), driven as the driver drives it.
//! Hardware-free: no test opens real USB hardware.

use std::time::Instant;

use crate::device::{Device, Out, devices, manifest};
use crate::tests_support::view;
use crate::{Action, Browse, KnobView, NoteView, PatternView, SceneView, View};


fn mikro_view() -> View {
    let mut v = view();
    let pattern = |color: usize, slot: usize| PatternView {
        name: "P".into(),
        color,
        bars: 1.0,
        slot,
        ..Default::default()
    };
    v.groups[0].color = 2;
    v.groups[1].color = 9;
    v.groups[0].patterns = vec![pattern(2, 0), pattern(3, 1), pattern(2, 2)];
    v.groups[0].scene_pattern = Some(1);
    v.groups[1].patterns = vec![pattern(9, 0)];
    v.groups[1].scene_pattern = Some(0);
    v.scenes = vec![
        SceneView { name: "1".into(), color: 5, patterns: vec![Some(1), Some(0)], ..Default::default() },
        SceneView { name: "2".into(), color: 6, patterns: vec![Some(0), None], ..Default::default() },
    ];
    v.velocity = 100;
    v.grid.step_ticks = 240;
    v.pattern_length = 2 * 3840;
    v.scale_root = 60;
    v.scale_bank = 0;
    v.scale_kind = 0;
    v
}

fn mikro() -> Device {
    let mut d = Device::new("maschine_mikro_mk3", None);
    assert_eq!(d.error(), None, "Maschine Mikro MK3 scripts must load cleanly");
    let v = mikro_view();
    d.open(&v, Instant::now());
    d
}

#[test]
fn mikro_manifest_is_valid() {
    let m = manifest(None, "maschine_mikro_mk3").expect("valid manifest");
    assert_eq!(m.id, "maschine_mikro_mk3");
    assert_eq!(m.name, "Maschine Mikro MK3");
    assert_eq!(m.hid, Some((0x17cc, 0x1700)));
}

#[test]
fn mikro_in_device_list() {
    let all = devices(None);
    assert!(all.iter().any(|d| d.id == "maschine_mikro_mk3"), "mikro must be in shipped devices");
}

#[test]
fn mikro_starts_cleanly() {
    let mut d = mikro();
    assert_eq!(d.error(), None);
    let _ = d.close();
}

#[test]
fn mikro_pad_map_populates() {
    let mut d = mikro();
    let v = mikro_view();
    let now = Instant::now();
    let map = d.pad_map(&v, now);
    assert_eq!(map.velocity, 100);
    // Pad mode should map 16 pads
    let mapped_count = map.pads.iter().filter(|notes| !notes.is_empty()).count();
    assert_eq!(mapped_count, 16, "all 16 pads should have mappings");
}

#[test]
fn mikro_mode_buttons_and_leds() {
    let mut d = mikro();
    let v = mikro_view();
    let now = Instant::now();

    // In Pad mode: LED_PAD_MODE is ON
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_PAD_MODE"), Some(deckard_mikro::leds::ON));
    assert_eq!(leds.get("LED_KEYBOARD"), Some(deckard_mikro::leds::DIM));

    // Switch to Keyboard mode
    d.button("KEYBOARD", true, &v, now);
    d.button("KEYBOARD", false, &v, now);

    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_KEYBOARD"), Some(deckard_mikro::leds::ON));
    assert_eq!(leds.get("LED_PAD_MODE"), Some(deckard_mikro::leds::DIM));
}

#[test]
fn mikro_screen_renders() {
    let mut d = mikro();
    let v = mikro_view();
    let now = Instant::now();

    let screen = d.mikro_screen(&v, now);
    // Screen must not be completely blank (all 0xff)
    let lit_pixels = screen.buf.iter().filter(|&&b| b != 0xff).count();
    assert!(lit_pixels > 0, "screen should have drawn pixels for status and pad view");
}

#[test]
fn mikro_touch_strip_event() {
    let mut d = mikro();
    let v = mikro_view();
    let now = Instant::now();

    // In default Pitch mode: center touch lights center segment 13
    d.strip(0, 0, "down", 0.5, &v, now);
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_TS_13"), Some(deckard_mikro::leds::ON));

    // Switch to Mod mode: touching at 0.5 lights segments 1..=13
    d.button("MOD", true, &v, now);
    d.button("MOD", false, &v, now);
    d.strip(0, 0, "move", 0.5, &v, now);
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_TS_01"), Some(deckard_mikro::leds::ON));
    assert_eq!(leds.get("LED_TS_13"), Some(deckard_mikro::leds::ON));
}

#[test]
fn mikro_encoder_turn() {
    let mut d = mikro();
    let v = mikro_view();
    let now = Instant::now();

    let _out = d.encoder(1, &v, now);
    // Script may emit action or status message
    assert_eq!(d.error(), None);
}

fn actions(out: Vec<Out>) -> Vec<Action> {
    out.into_iter()
        .filter_map(|o| match o {
            Out::App(a) => Some(a),
            _ => None,
        })
        .collect()
}

/// 1-based pad (1 = bottom-left .. 16 = top-right) to (col, row) 0-based (row 0 = top).
fn pad_rc(pad: usize) -> (u8, u8) {
    let col = ((pad - 1) % 4) as u8;
    let row = (3 - (pad - 1) / 4) as u8;
    (col, row)
}

fn pad_map_idx(pad: usize) -> usize {
    let (col, row) = pad_rc(pad);
    usize::from(col) + usize::from(row) * 8
}

#[test]
fn mikro_fixed_velocity_toggle_and_encoder_adjust() {
    let mut d = mikro();
    let mut v = mikro_view();
    v.fixed_velocity = false;
    v.velocity = 100;
    let now = Instant::now();

    // Tap FIXED VEL -> toggles FixedVelocity(true) on release.
    let down = actions(d.button("BUTTON_FIX_VELOCITY", true, &v, now));
    assert!(down.is_empty());
    let up = actions(d.button("BUTTON_FIX_VELOCITY", false, &v, now));
    assert_eq!(up, vec![Action::FixedVelocity(true)]);

    // Hold FIXED VEL + turn encoder +2 -> adjusts VelocityValue(+10) and must NOT toggle on release.
    let _ = d.button("BUTTON_FIX_VELOCITY", true, &v, now);
    let turned = actions(d.encoder(2, &v, now));
    assert_eq!(turned, vec![Action::VelocityValue(110)]);
    let released = actions(d.button("BUTTON_FIX_VELOCITY", false, &v, now));
    assert!(released.is_empty(), "encoder adjust while holding FIXED VEL must not toggle on release");

    // When view.fixed_velocity is true, PadMap forwards fixed_velocity & velocity and LED_FIX_VELOCITY is ON.
    v.fixed_velocity = true;
    v.velocity = 110;
    let map = d.pad_map(&v, now);
    assert!(map.fixed_velocity);
    assert_eq!(map.velocity, 110);
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_FIX_VELOCITY"), Some(deckard_mikro::leds::ON));
}

#[test]
fn mikro_sixteen_velocities_mode() {
    let mut d = mikro();
    let mut v = mikro_view();
    v.sound = 2; // 0-based sound 3
    let now = Instant::now();

    // SHIFT + FIXED VEL turns on 16 Velocities.
    d.button("BUTTON_SHIFT", true, &v, now);
    d.button("BUTTON_FIX_VELOCITY", true, &v, now);
    d.button("BUTTON_FIX_VELOCITY", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);

    let map = d.pad_map(&v, now);
    let mut prev_vel = 0;
    for pad in 1..=16 {
        let idx = pad_map_idx(pad);
        assert_eq!(map.pads[idx], vec![(0, 2, 60)], "pad {pad} must trigger focused sound 3");
        let vel = map.pad_velocities[idx].expect("per-pad velocity in 16 Velocities");
        assert!(vel > prev_vel, "velocities must strictly increase across pads 1..16");
        prev_vel = vel;
    }
    assert_eq!(map.pad_velocities[pad_map_idx(1)], Some(8));
    assert_eq!(map.pad_velocities[pad_map_idx(16)], Some(127));
    assert_eq!(d.mikro_leds(&v, now).get("LED_FIX_VELOCITY"), Some(deckard_mikro::leds::ON));

    // Pressing pad 5 in 16 Velocities must NOT switch focused sound.
    let (c, r) = pad_rc(5);
    let hit = actions(d.pad(c, r, true, &v, now));
    assert!(!hit.iter().any(|a| matches!(a, Action::FocusSound(_))));

    // Tapping FIXED VEL without Shift while in 16 Velocities exits 16 Velocities without toggling FixedVelocity.
    d.button("BUTTON_FIX_VELOCITY", true, &v, now);
    let up = actions(d.button("BUTTON_FIX_VELOCITY", false, &v, now));
    assert!(up.is_empty(), "exiting 16 Velocities via FIXED VEL must not emit FixedVelocity toggle");
    let map = d.pad_map(&v, now);
    assert!(map.pad_velocities.iter().all(|pv| pv.is_none()), "back to normal pad mode");
}

#[test]
fn mikro_erase_and_select_silence_pads_and_act() {
    let mut d = mikro();
    let v = mikro_view();
    let now = Instant::now();

    // Holding ERASE silences PadMap and erases sound on pad tap.
    d.button("BUTTON_TRANSPORT_ERASE", true, &v, now);
    assert!(d.pad_map(&v, now).pads.iter().all(|p| p.is_empty()), "ERASE held must silence pads");
    let (c2, r2) = pad_rc(2);
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), vec![Action::EraseSound(1)]);

    // SHIFT + ERASE + pad resets sound.
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), vec![Action::ResetSound(1)]);
    d.button("BUTTON_SHIFT", false, &v, now);
    d.button("BUTTON_TRANSPORT_ERASE", false, &v, now);

    // Holding SELECT in Step mode silences PadMap and focuses sound instead of toggling a step.
    d.button("BUTTON_STEP", true, &v, now);
    d.button("BUTTON_STEP", false, &v, now);
    d.button("BUTTON_SELECT", true, &v, now);
    assert!(d.pad_map(&v, now).pads.iter().all(|p| p.is_empty()), "SELECT held must silence pads");
    let (c3, r3) = pad_rc(3);
    assert_eq!(actions(d.pad(c3, r3, true, &v, now)), vec![Action::FocusSound(2)]);
    d.button("BUTTON_SELECT", false, &v, now);
}

#[test]
fn mikro_browse_toggle_scroll_prehear_and_load() {
    let mut d = mikro();
    let mut v = mikro_view();
    v.browser.kind = 6; // One-shots (0-based 6 -> Lua 7)
    v.browser.results = vec![
        ("Kick 01".into(), false),
        ("Kick 02".into(), true),
        ("Snare 01".into(), false),
    ];
    v.browser.focus = 0;
    let now = Instant::now();

    // Toggle BROWSE ON
    let out = actions(d.button("BUTTON_BROWSE", true, &v, now));
    assert_eq!(out, vec![Action::Library(true)]);
    d.button("BUTTON_BROWSE", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_BROWSE"), Some(deckard_mikro::leds::ON));

    // Screen renders browse view (non-blank)
    let scr = d.mikro_screen(&v, now);
    assert!(scr.buf.iter().any(|&b| b != 0xff));

    // Encoder +1 scrolls results and prehears
    let enc = actions(d.encoder(1, &v, now));
    assert_eq!(enc, vec![Action::Browse(Browse::Focus(1)), Action::Browse(Browse::Prehear)]);

    // SHIFT + encoder +1 jumps 7 results
    d.button("BUTTON_SHIFT", true, &v, now);
    let jump = actions(d.encoder(1, &v, now));
    assert_eq!(jump, vec![Action::Browse(Browse::Focus(7)), Action::Browse(Browse::Prehear)]);
    d.button("BUTTON_SHIFT", false, &v, now);

    // Encoder press loads focused result
    let load = actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now));
    assert_eq!(load, vec![Action::Browse(Browse::Load)]);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);

    // Toggle BROWSE OFF
    let off = actions(d.button("BUTTON_BROWSE", true, &v, now));
    assert_eq!(off, vec![Action::Library(false)]);
    d.button("BUTTON_BROWSE", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_BROWSE"), Some(deckard_mikro::leds::DIM));
}

#[test]
fn mikro_browse_star_favorites_and_filter_field_skipping() {
    let mut d = mikro();
    let mut v = mikro_view();
    v.browser.kind = 5; // Loops
    v.browser.products = vec![]; // empty -> must be skipped
    v.browser.attributes[0] = (vec!["Drums".into(), "Bass".into()], None);
    v.browser.attributes[1] = (vec![], None); // empty -> must be skipped
    v.browser.attributes[2] = (vec!["Analog".into()], None);
    v.browser.results = vec![("Loop 1".into(), false), ("Loop 2".into(), false)];
    let now = Instant::now();

    d.button("BUTTON_BROWSE", true, &v, now);
    d.button("BUTTON_BROWSE", false, &v, now);

    // STAR toggles Favorites filter; SHIFT + STAR sets/unsets favorite on current result.
    assert_eq!(actions(d.button("BUTTON_STAR", true, &v, now)), vec![Action::Browse(Browse::Favorites)]);
    d.button("BUTTON_STAR", false, &v, now);

    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.button("BUTTON_STAR", true, &v, now)), vec![Action::Browse(Browse::SetFavorite)]);
    d.button("BUTTON_STAR", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);

    // From Results (6), LEFT skips nothing to Char (5), then skips empty SubType (4) to Type (3),
    // then skips empty Product (2) to Kind (1), then stays at Kind (1).
    d.button("BUTTON_LEFT", true, &v, now); // -> Char (5)
    d.button("BUTTON_LEFT", false, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Browse(Browse::Attribute(2, Some(0)))]);

    d.button("BUTTON_LEFT", true, &v, now); // skips empty SubType -> Type (3)
    d.button("BUTTON_LEFT", false, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Browse(Browse::Attribute(0, Some(0)))]);

    d.button("BUTTON_LEFT", true, &v, now); // skips empty Product -> Kind (1)
    d.button("BUTTON_LEFT", false, &v, now);
    assert_eq!(actions(d.encoder(-1, &v, now)), vec![Action::Browse(Browse::Kind(4))]);

    // Extra LEFT at Kind (1) stays safely at Kind (1).
    d.button("BUTTON_LEFT", true, &v, now);
    d.button("BUTTON_LEFT", false, &v, now);
    assert_eq!(actions(d.encoder(-1, &v, now)), vec![Action::Browse(Browse::Kind(4))]);

    // Pushing encoder on a filter field jumps back to Results (6) without loading.
    assert!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)).is_empty());
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Browse(Browse::Focus(1)), Action::Browse(Browse::Prehear)]);

    // SHIFT + BROWSE toggles prehear off -> subsequent scroll omits Prehear.
    d.button("BUTTON_SHIFT", true, &v, now);
    d.button("BUTTON_BROWSE", true, &v, now);
    d.button("BUTTON_BROWSE", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Browse(Browse::Focus(1))]);
}

#[test]
fn mikro_browse_encoder_press_on_empty_results_does_not_load() {
    let mut d = mikro();
    let mut v = mikro_view();
    v.browser.kind = 6;
    v.browser.results = vec![];
    let now = Instant::now();

    d.button("BUTTON_BROWSE", true, &v, now);
    d.button("BUTTON_BROWSE", false, &v, now);

    // Pushing encoder on Results when results list is empty must NOT emit Browse::Load.
    let out = actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now));
    assert!(out.is_empty(), "empty results must not emit Browse::Load: {out:?}");
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
}

#[test]
fn mikro_scene_hold_and_tap_to_pin() {
    let mut d = mikro();
    let v = mikro_view();
    let now = Instant::now();

    // Hold SCENE, press pad 2 -> focuses scene 2 (0-based 1), pads silent; releasing closes Scene.
    d.button("BUTTON_SCENE", true, &v, now);
    assert!(d.pad_map(&v, now).pads.iter().all(|p| p.is_empty()), "SCENE overlay must silence pads");
    assert_eq!(d.mikro_leds(&v, now).get("LED_SCENE"), Some(deckard_mikro::leds::ON));
    let (c2, r2) = pad_rc(2);
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), vec![Action::FocusScene(1)]);
    d.button("BUTTON_SCENE", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_SCENE"), Some(deckard_mikro::leds::DIM));

    // Quick tap SCENE (no pad/encoder used) -> stays pinned!
    d.button("BUTTON_SCENE", true, &v, now);
    d.button("BUTTON_SCENE", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_SCENE"), Some(deckard_mikro::leds::ON));

    // Pad 3 (#scenes + 1) creates new scene; encoder press also creates scene; encoder turn scrolls scenes.
    let (c3, r3) = pad_rc(3);
    assert_eq!(actions(d.pad(c3, r3, true, &v, now)), vec![Action::SceneEdit("create")]);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::FocusScene(1)]);
    assert_eq!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)), vec![Action::SceneEdit("create")]);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);

    // ERASE + pad 1 deletes scene 1 (0-based 0).
    d.button("BUTTON_TRANSPORT_ERASE", true, &v, now);
    let (c1, r1) = pad_rc(1);
    assert_eq!(actions(d.pad(c1, r1, true, &v, now)), vec![Action::DeleteScene(0)]);
    d.button("BUTTON_TRANSPORT_ERASE", false, &v, now);

    // Tap SCENE again closes pinned Scene.
    d.button("BUTTON_SCENE", true, &v, now);
    d.button("BUTTON_SCENE", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_SCENE"), Some(deckard_mikro::leds::DIM));
}

#[test]
fn mikro_pattern_hold_pin_length_and_crud() {
    let mut d = mikro();
    let v = mikro_view(); // group 0 has patterns at slots 0, 1, 2; pattern_length = 2 * 3840 (2 bars)
    let now = Instant::now();

    // Tap-to-pin PATTERN
    d.button("BUTTON_PATTERN", true, &v, now);
    d.button("BUTTON_PATTERN", false, &v, now);
    assert!(d.pad_map(&v, now).pads.iter().all(|p| p.is_empty()), "PATTERN overlay must silence pads");
    assert_eq!(d.mikro_leds(&v, now).get("LED_PATTERN"), Some(deckard_mikro::leds::ON));

    // Pad 1 (occupied slot 1) picks pattern 0; empty pad 4 creates pattern at slot 3.
    let (c1, r1) = pad_rc(1);
    assert_eq!(actions(d.pad(c1, r1, true, &v, now)), vec![Action::PickPattern(0)]);
    let (c4, r4) = pad_rc(4);
    assert_eq!(actions(d.pad(c4, r4, true, &v, now)), vec![Action::NewPatternAt { group: 0, slot: 3 }]);

    // Encoder +1 doubles length (2 bars -> 4 bars); SHIFT + encoder +1 adds 1 bar (2 -> 3 bars).
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::PatternLength(4 * 3840)]);
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::PatternLength(3 * 3840)]);
    d.button("BUTTON_SHIFT", false, &v, now);

    // ERASE + pad 1 deletes pattern 0.
    d.button("BUTTON_TRANSPORT_ERASE", true, &v, now);
    assert_eq!(actions(d.pad(c1, r1, true, &v, now)), vec![Action::DeletePattern { group: 0, pattern: 0 }]);
    d.button("BUTTON_TRANSPORT_ERASE", false, &v, now);

    // Encoder press creates pattern.
    assert_eq!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)), vec![Action::PatternEdit("create")]);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
}

#[test]
fn mikro_duplicate_orthogonal_and_double() {
    let mut d = mikro();
    let v = mikro_view();
    let now = Instant::now();

    // Pin PATTERN, then press SHIFT + DUPLICATE (Double): emits PatternEdit("double") and must NOT stick dup_active.
    d.button("BUTTON_PATTERN", true, &v, now);
    d.button("BUTTON_PATTERN", false, &v, now);
    d.button("BUTTON_SHIFT", true, &v, now);
    let dbl = actions(d.button("BUTTON_DUPLICATE", true, &v, now));
    assert_eq!(dbl, vec![Action::PatternEdit("double")]);
    d.button("BUTTON_DUPLICATE", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_DUPLICATE"), Some(deckard_mikro::leds::DIM));

    // While PATTERN is still pinned, tap-to-pin DUPLICATE -> copy pattern slot 1 -> slot 5 (0-based 0 -> 4).
    d.button("BUTTON_DUPLICATE", true, &v, now);
    d.button("BUTTON_DUPLICATE", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_DUPLICATE"), Some(deckard_mikro::leds::ON));
    let (c1, r1) = pad_rc(1);
    let (c5, r5) = pad_rc(5);
    assert!(actions(d.pad(c1, r1, true, &v, now)).is_empty(), "first tap selects source");
    assert_eq!(actions(d.pad(c5, r5, true, &v, now)), vec![Action::CopyPattern { group: 0, from: 0, to: 4 }]);

    // Exit PATTERN + DUPLICATE via PAD_MODE.
    d.button("BUTTON_PAD_MODE", true, &v, now);
    d.button("BUTTON_PAD_MODE", false, &v, now);

    // Hold DUPLICATE in Pad mode: pad 1 -> pad 4 copies sound 0 -> 3; release closes dup_active.
    d.button("BUTTON_DUPLICATE", true, &v, now);
    assert!(d.pad_map(&v, now).pads.iter().all(|p| p.is_empty()), "DUPLICATE must silence pads");
    let (c4, r4) = pad_rc(4);
    assert!(actions(d.pad(c1, r1, true, &v, now)).is_empty());
    assert_eq!(actions(d.pad(c4, r4, true, &v, now)), vec![Action::CopySound { from: 0, to: 3 }]);
    d.button("BUTTON_DUPLICATE", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_DUPLICATE"), Some(deckard_mikro::leds::DIM));

    // Bidirectional: tap-to-pin DUPLICATE first, then hold SCENE -> pad 1 -> pad 2 copies scene 0 -> 1.
    d.button("BUTTON_DUPLICATE", true, &v, now);
    d.button("BUTTON_DUPLICATE", false, &v, now);
    d.button("BUTTON_SCENE", true, &v, now);
    let (c2, r2) = pad_rc(2);
    assert!(actions(d.pad(c1, r1, true, &v, now)).is_empty());
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), vec![Action::CopyScene { from: 0, to: 1 }]);
    d.button("BUTTON_SCENE", false, &v, now);

    // GROUP + DUPLICATE -> copy group 0 -> 2 (pad 1 -> pad 3).
    d.button("BUTTON_GROUP", true, &v, now);
    let (c3, r3) = pad_rc(3);
    assert!(actions(d.pad(c1, r1, true, &v, now)).is_empty());
    assert_eq!(actions(d.pad(c3, r3, true, &v, now)), vec![Action::CopyGroup { from: 0, to: 2 }]);
    d.button("BUTTON_GROUP", false, &v, now);
}

#[test]
fn mikro_events_select_edit_and_erase() {
    let mut d = mikro();
    let mut v = mikro_view();
    v.notes = vec![
        NoteView { sound: 0, position: 0, length: 240, pitch: 60, velocity: 100, probability: 100, selected: false, ..Default::default() },
        NoteView { sound: 2, position: 480, length: 240, pitch: 60, velocity: 90, probability: 100, selected: true, ..Default::default() },
    ];
    let now = Instant::now();

    // Tap-to-pin EVENTS
    d.button("BUTTON_EVENTS", true, &v, now);
    d.button("BUTTON_EVENTS", false, &v, now);
    assert!(d.pad_map(&v, now).pads.iter().all(|p| p.is_empty()), "EVENTS must silence pads");
    assert_eq!(d.mikro_leds(&v, now).get("LED_EVENTS"), Some(deckard_mikro::leds::ON));

    // Pad LEDs: pad 1 (unselected note) dim, pad 3 (all selected) bright, pad 2 (no notes) off.
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_PAD_2"), Some(0), "sound without notes should be dark");
    assert_ne!(leds.get("LED_PAD_1"), Some(0));
    assert_ne!(leds.get("LED_PAD_3"), Some(0));

    // Pad 1 selects sound 1 (0-based 0); ERASE + pad 1 erases sound 1.
    let (c1, r1) = pad_rc(1);
    assert_eq!(actions(d.pad(c1, r1, true, &v, now)), vec![Action::Events("select_sound", 0)]);
    d.button("BUTTON_TRANSPORT_ERASE", true, &v, now);
    assert_eq!(actions(d.pad(c1, r1, true, &v, now)), vec![Action::EraseSound(0)]);
    d.button("BUTTON_TRANSPORT_ERASE", false, &v, now);

    // Default param 1 (POS): encoder +1 nudges; SHIFT nudges fine.
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Events("nudge", 1)]);
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Events("nudge_fine", 1)]);
    d.button("BUTTON_SHIFT", false, &v, now);

    // RIGHT twice -> VEL (3): encoder +1 adjusts velocity.
    d.button("BUTTON_RIGHT", true, &v, now);
    d.button("BUTTON_RIGHT", false, &v, now);
    d.button("BUTTON_RIGHT", true, &v, now);
    d.button("BUTTON_RIGHT", false, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Events("velocity", 1)]);

    // SHIFT + encoder press: since a note is selected, emits select_none; when none selected, emits select_all.
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)), vec![Action::Events("select_none", 0)]);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
    v.notes[1].selected = false;
    assert_eq!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)), vec![Action::Events("select_all", 0)]);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
}

#[test]
fn mikro_variation_toggle_sparse_knobs_and_apply() {
    let mut d = mikro();
    let mut v = mikro_view();
    let knob = |name: &str, text: &str| {
        Some(KnobView { name: name.into(), section: "Variation".into(), text: text.into(), value: 0.5, ..Default::default() })
    };
    v.control.pages = vec!["Swing".into(), "Variation".into(), "Generate".into()];
    v.control.page = 1;
    // Variation has gaps at slots 5 and 6 (0-based 4 and 5).
    v.control.knobs = vec![
        knob("Amount", "0 %"),
        knob("Step", "0 %"),
        knob("Steps", "x1.00"),
        knob("Variation", "0 %"),
        None,
        None,
        knob("Distribution", "Natural"),
        knob("Live", "Off"),
    ];
    let now = Instant::now();

    // Press VARIATION -> opens Groove page 2 (0-based 1), LED_VARIATION ON, pads stay playable.
    assert_eq!(actions(d.button("BUTTON_VARIATION", true, &v, now)), vec![Action::GroovePage(1)]);
    d.button("BUTTON_VARIATION", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_VARIATION"), Some(deckard_mikro::leds::ON));
    assert_eq!(d.pad_map(&v, now).pads.iter().filter(|p| !p.is_empty()).count(), 16, "pads stay playable in Variation");

    // Step RIGHT 4 times: slot 1 -> 2 -> 3 -> 4 -> skips 5 & 6 -> lands on slot 7 (0-based 6).
    for _ in 0..4 {
        d.button("BUTTON_RIGHT", true, &v, now);
        d.button("BUTTON_RIGHT", false, &v, now);
    }
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::StepKnob { knob: 6, steps: 1, fine: false }]);

    // SHIFT + RIGHT switches Groove sub-page (Variation -> Generate).
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.button("BUTTON_RIGHT", true, &v, now)), vec![Action::Page(true)]);
    d.button("BUTTON_RIGHT", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);

    // Encoder press emits GrooveApply.
    assert_eq!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)), vec![Action::GrooveApply]);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);

    // Second VARIATION press exits Variation overlay.
    d.button("BUTTON_VARIATION", true, &v, now);
    d.button("BUTTON_VARIATION", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_VARIATION"), Some(deckard_mikro::leds::DIM));
}

#[test]
fn mikro_navigate_toggle_and_pads() {
    use deckard_mikro::leds::{SHADE_BRIGHT, SHADE_DIM, colour};

    let mut d = mikro();
    let mut v = mikro_view();
    let now = Instant::now();

    // SHIFT + VARIATION enters Navigate overlay, pads silent.
    d.button("BUTTON_SHIFT", true, &v, now);
    d.button("BUTTON_VARIATION", true, &v, now);
    d.button("BUTTON_VARIATION", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
    assert!(d.pad_map(&v, now).pads.iter().all(|p| p.is_empty()), "NAVIGATE must silence pads");
    assert_eq!(d.mikro_leds(&v, now).get("LED_VARIATION"), Some(deckard_mikro::leds::ON));

    // Verify color-coded pads in Navigate overlay (dim when inactive).
    v.control.modules_visible = false;
    let leds = d.mikro_leds(&v, now);
    for (pad, pal) in [
        (1, 16), (2, 16),   // WHITE: Page Left/Right
        (3, 0),  (4, 0),    // RED: Octave Down/Up
        (5, 2),  (6, 2),    // ORANGE: Scene Prev/Next
        (7, 14), (8, 14),   // MAGENTA: Pattern Prev/Next
        (9, 6),  (10, 6),   // GREEN: Group Prev/Next
        (11, 4), (12, 4),   // YELLOW: Sound Prev/Next
        (13, 8), (14, 8), (15, 8), (16, 8), // CYAN: Song, Mixer, Follow, Plugins
    ] {
        assert_eq!(leds.get(&format!("LED_PAD_{pad}")), Some(colour(pal, SHADE_DIM)), "pad {pad}");
    }

    // Active top-row toggles illuminate CYAN bright.
    v.song_view = true;
    v.follow = true;
    v.control.modules_visible = true;
    let leds_on = d.mikro_leds(&v, now);
    assert_eq!(leds_on.get("LED_PAD_13"), Some(colour(8, SHADE_BRIGHT)));
    assert_eq!(leds_on.get("LED_PAD_14"), Some(colour(8, SHADE_DIM)));
    assert_eq!(leds_on.get("LED_PAD_15"), Some(colour(8, SHADE_BRIGHT)));
    assert_eq!(leds_on.get("LED_PAD_16"), Some(colour(8, SHADE_BRIGHT)));

    v.song_view = false;
    v.follow = false;
    v.control.modules_visible = false;
    let hit_pad = |d: &mut Device, p: usize| {
        let (c, r) = pad_rc(p);
        actions(d.pad(c, r, true, &v, now))
    };
    assert_eq!(hit_pad(&mut d, 13), vec![Action::SongView(true)]);
    assert_eq!(hit_pad(&mut d, 14), vec![Action::MixerView(true)]);
    // Pad 14 turned st.mixer_view = true -> PAD_14 lights CYAN bright.
    assert_eq!(d.mikro_leds(&v, now).get("LED_PAD_14"), Some(colour(8, SHADE_BRIGHT)));
    assert_eq!(hit_pad(&mut d, 15), vec![Action::Follow(true)]);
    assert_eq!(hit_pad(&mut d, 16), vec![Action::ModulesVisible(!v.control.modules_visible)]);
    assert_eq!(hit_pad(&mut d, 1), vec![Action::Page(false)]);
    assert_eq!(hit_pad(&mut d, 2), vec![Action::Page(true)]);
    assert_eq!(hit_pad(&mut d, 10), vec![Action::FocusGroup(1)]);
    assert_eq!(hit_pad(&mut d, 6), vec![Action::FocusScene(1)]);

    // SHIFT + VARIATION again exits Navigate.
    d.button("BUTTON_SHIFT", true, &v, now);
    d.button("BUTTON_VARIATION", true, &v, now);
    d.button("BUTTON_VARIATION", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_VARIATION"), Some(deckard_mikro::leds::DIM));
}

#[test]
fn mikro_encoder_priority_returns_to_browse() {
    let mut d = mikro();
    let mut v = mikro_view();
    v.tempo = 120.0;
    v.browser.kind = 6;
    v.browser.results = vec![("Kick".into(), false), ("Snare".into(), false)];
    let now = Instant::now();

    // Toggle BROWSE ON
    d.button("BUTTON_BROWSE", true, &v, now);
    d.button("BUTTON_BROWSE", false, &v, now);

    // Hold TEMPO + encoder +1 adjusts Tempo, not Browse; release TEMPO -> Browse works again.
    d.button("BUTTON_TEMPO", true, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Tempo(121.0)]);
    d.button("BUTTON_TEMPO", false, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Browse(Browse::Focus(1)), Action::Browse(Browse::Prehear)]);

    // Hold SCENE + encoder +1 scrolls Scene; release SCENE -> st.browse stays ON and encoder returns to Browse!
    d.button("BUTTON_SCENE", true, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::FocusScene(1)]);
    d.button("BUTTON_SCENE", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_BROWSE"), Some(deckard_mikro::leds::ON));
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Browse(Browse::Focus(1)), Action::Browse(Browse::Prehear)]);
}

#[test]
fn mikro_shift_mute_chokes_all_without_entering_mute_hold() {
    let mut d = mikro();
    let v = mikro_view();
    let now = Instant::now();

    // SHIFT + MUTE -> Action::ChokeAll; does NOT enter Mute hold.
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.button("BUTTON_MUTE", true, &v, now)), vec![Action::ChokeAll]);
    assert_eq!(d.mikro_leds(&v, now).get("LED_MUTE"), Some(deckard_mikro::leds::DIM));
    d.button("BUTTON_MUTE", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
    assert_eq!(d.pad_map(&v, now).pads.iter().filter(|p| !p.is_empty()).count(), 16);

    // MUTE without SHIFT -> Mute hold silences pads and mutes tapped sound.
    assert!(actions(d.button("BUTTON_MUTE", true, &v, now)).is_empty());
    assert_eq!(d.mikro_leds(&v, now).get("LED_MUTE"), Some(deckard_mikro::leds::ON));
    assert!(d.pad_map(&v, now).pads.iter().all(|p| p.is_empty()), "MUTE held must silence pads");
    let (c2, r2) = pad_rc(2);
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), vec![Action::MuteSound(1)]);
    d.pad(c2, r2, false, &v, now);
    d.button("BUTTON_MUTE", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_MUTE"), Some(deckard_mikro::leds::DIM));
}

#[test]
fn mikro_pad_mode_leds_flash_loaded_and_keep_empty_dark_and_unselectable() {
    use deckard_mikro::leds::{SHADE_BRIGHT, SHADE_DIM, SHADE_FLASH, colour};

    let mut d = mikro();
    let mut v = mikro_view(); // group 0 color=2; sounds 0..3 used=true color=0; sounds 3..16 used=false
    let now = Instant::now();

    // Sound 1 (index 0, used) is focused: pad 1 bright, pads 2..3 dim, pads 4..16 OFF (0).
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_PAD_1"), Some(colour(0, SHADE_BRIGHT)));
    assert_eq!(leds.get("LED_PAD_2"), Some(colour(0, SHADE_DIM)));
    assert_eq!(leds.get("LED_PAD_3"), Some(colour(0, SHADE_DIM)));
    for pad in 4..=16 {
        assert_eq!(leds.get(&format!("LED_PAD_{pad}")), Some(0), "empty pad {pad} must be off");
    }

    // Holding loaded pad 2 flashes SHADE_FLASH and returns to SHADE_DIM on release (after focus).
    let (c2, r2) = pad_rc(2);
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), vec![Action::FocusSound(1)]);
    assert_eq!(d.mikro_leds(&v, now).get("LED_PAD_2"), Some(colour(0, SHADE_FLASH)));
    d.pad(c2, r2, false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_PAD_2"), Some(colour(0, SHADE_DIM)));

    // Empty pad 8: stays OFF even if struck/held or even if v.sound points to it, and striking must NOT select it.
    let (c8, r8) = pad_rc(8);
    assert!(actions(d.pad(c8, r8, true, &v, now)).is_empty(), "empty pad hit must not focus sound");
    assert_eq!(d.mikro_leds(&v, now).get("LED_PAD_8"), Some(0), "held empty pad stays off");
    d.pad(c8, r8, false, &v, now);
    v.sound = 4;
    for pad in 4..=16 {
        assert_eq!(d.mikro_leds(&v, now).get(&format!("LED_PAD_{pad}")), Some(0), "empty pad {pad} always off");
    }
    v.sound = 0;

    // 16 Velocities, Keyboard, and Chords also flash held pads with SHADE_FLASH.
    d.button("BUTTON_SHIFT", true, &v, now);
    d.button("BUTTON_FIX_VELOCITY", true, &v, now);
    d.button("BUTTON_FIX_VELOCITY", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
    let (c1, r1) = pad_rc(1);
    d.pad(c1, r1, true, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_PAD_1"), Some(colour(0, SHADE_FLASH)));
    d.pad(c1, r1, false, &v, now);

    for mode_btn in ["BUTTON_KEYBOARD", "BUTTON_CHORDS"] {
        d.button(mode_btn, true, &v, now);
        d.button(mode_btn, false, &v, now);
        d.pad(c2, r2, true, &v, now);
        assert_eq!(d.mikro_leds(&v, now).get("LED_PAD_2"), Some(colour(0, SHADE_FLASH)));
        d.pad(c2, r2, false, &v, now);
    }

    // Back to Pad mode: playing sound 2 flashes; MUTE hold hides empty pads and dims muted sounds.
    d.button("BUTTON_PAD_MODE", true, &v, now);
    d.button("BUTTON_PAD_MODE", false, &v, now);
    v.groups[0].sounds[1].playing = true;
    assert_eq!(d.mikro_leds(&v, now).get("LED_PAD_2"), Some(colour(0, SHADE_FLASH)));
    v.groups[0].sounds[1].playing = false;
    v.groups[0].sounds[1].muted = true;
    d.button("BUTTON_MUTE", true, &v, now);
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_PAD_1"), Some(colour(0, SHADE_BRIGHT)));
    assert_eq!(leds.get("LED_PAD_2"), Some(colour(0, SHADE_DIM)));
    assert_eq!(leds.get("LED_PAD_4"), Some(0), "empty pad stays off in MUTE mode");
    d.button("BUTTON_MUTE", false, &v, now);
}

#[test]
fn mikro_plugin_mode_params_page_wrap_slots_level_and_actions() {
    let mut d = mikro();
    let mut v = mikro_view();
    let knob = |name: &str, text: &str| {
        Some(KnobView { name: name.into(), section: "Main".into(), text: text.into(), value: 0.5, ..Default::default() })
    };
    v.control.level = 2; // Sound (1-based 3)
    v.control.modules_visible = false;
    v.control.slot_index = Some(0);
    v.control.slot_count = 2;
    v.control.chain = vec![
        crate::SlotView { name: "Zhora".into(), instrument: true, active: true, plugin: false, picture: None },
        crate::SlotView { name: "EQ".into(), instrument: false, active: true, plugin: false, picture: None },
    ];
    v.control.pages = vec!["Filter".into(), "Env".into()];
    v.control.page = 0;
    // Sparse knobs: 1 active, 2 empty, 3 active.
    v.control.knobs = vec![knob("Cutoff", "1.0 kHz"), None, knob("Resonance", "20 %"), None, None, None, None, None];
    let now = Instant::now();

    // Tap PLUG-IN -> opens Plug-ins view, LED_PLUGIN ON, pads stay playable.
    assert_eq!(actions(d.button("BUTTON_PLUGIN", true, &v, now)), vec![Action::ModulesVisible(true)]);
    d.button("BUTTON_PLUGIN", false, &v, now);
    v.control.modules_visible = true;
    assert_eq!(d.mikro_leds(&v, now).get("LED_PLUGIN"), Some(deckard_mikro::leds::ON));
    assert_eq!(d.pad_map(&v, now).pads.iter().filter(|p| !p.is_empty()).count(), 16, "pads stay playable in PLUG-IN mode");

    // Encoder steps active knob 1 (index 0); SHIFT makes it fine.
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::StepKnob { knob: 0, steps: 1, fine: false }]);
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.encoder(-1, &v, now)), vec![Action::StepKnob { knob: 0, steps: -1, fine: true }]);
    d.button("BUTTON_SHIFT", false, &v, now);

    // RIGHT skips empty knob 2 -> selects knob 3 (index 2).
    d.button("BUTTON_RIGHT", true, &v, now);
    d.button("BUTTON_RIGHT", false, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::StepKnob { knob: 2, steps: 1, fine: false }]);

    // RIGHT past last active knob on page 1 wraps to page 2 -> first active knob on page 2.
    assert_eq!(actions(d.button("BUTTON_RIGHT", true, &v, now)), vec![Action::Page(true)]);
    d.button("BUTTON_RIGHT", false, &v, now);
    v.control.page = 1;
    v.control.knobs = vec![None, knob("Attack", "10 ms"), knob("Decay", "200 ms"), None, None, None, None, None];
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::StepKnob { knob: 1, steps: 1, fine: false }]);

    // LEFT past first active knob on page 2 wraps back to page 1's last active knob (knob 3).
    assert_eq!(actions(d.button("BUTTON_LEFT", true, &v, now)), vec![Action::Page(false)]);
    d.button("BUTTON_LEFT", false, &v, now);
    v.control.page = 0;
    v.control.knobs = vec![knob("Cutoff", "1.0 kHz"), None, knob("Resonance", "20 %"), None, None, None, None, None];
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::StepKnob { knob: 2, steps: 1, fine: false }]);

    // SHIFT + RIGHT jumps page directly.
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.button("BUTTON_RIGHT", true, &v, now)), vec![Action::Page(true)]);
    d.button("BUTTON_RIGHT", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);

    // Hold PLUG-IN: encoder steps Slot, LEFT/RIGHT steps Level, MUTE bypasses, ERASE removes; stays open on release.
    d.button("BUTTON_PLUGIN", true, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Slot(1)]);
    assert_eq!(actions(d.button("BUTTON_LEFT", true, &v, now)), vec![Action::Level(1)]);
    d.button("BUTTON_LEFT", false, &v, now);
    assert_eq!(actions(d.button("BUTTON_MUTE", true, &v, now)), vec![Action::SlotEdit("bypass")]);
    d.button("BUTTON_MUTE", false, &v, now);
    assert_eq!(actions(d.button("BUTTON_ERASE", true, &v, now)), vec![Action::SlotEdit("remove")]);
    d.button("BUTTON_ERASE", false, &v, now);
    d.button("BUTTON_PLUGIN", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_PLUGIN"), Some(deckard_mikro::leds::ON));

    // Quick tap PLUG-IN again without using hold shortcuts -> exits PLUG-IN mode.
    d.button("BUTTON_PLUGIN", true, &v, now);
    d.button("BUTTON_PLUGIN", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_PLUGIN"), Some(deckard_mikro::leds::DIM));
}

#[test]
fn mikro_shift_plugin_opens_channel_macro_and_steps_groups() {
    let mut d = mikro();
    let mut v = mikro_view();
    let now = Instant::now();

    // SHIFT + PLUG-IN opens Channel ▸ Macro.
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(
        actions(d.button("BUTTON_PLUGIN", true, &v, now)),
        vec![Action::ModulesVisible(false), Action::PageGroup(3)]
    );
    d.button("BUTTON_PLUGIN", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
    v.control.modules_visible = false;
    v.control.page_group = 3; // 0-based Macro
    assert_eq!(d.mikro_leds(&v, now).get("LED_PLUGIN"), Some(deckard_mikro::leds::ON));

    // Hold PLUG-IN + encoder steps page group (Macro -> Groove).
    d.button("BUTTON_PLUGIN", true, &v, now);
    assert_eq!(actions(d.encoder(-1, &v, now)), vec![Action::PageGroup(2)]);
    d.button("BUTTON_PLUGIN", false, &v, now);

    // Tapping PLUG-IN (without Shift) while in Channel view switches back to Plug-ins view instead of closing.
    assert_eq!(actions(d.button("BUTTON_PLUGIN", true, &v, now)), vec![Action::ModulesVisible(true)]);
    d.button("BUTTON_PLUGIN", false, &v, now);
    v.control.modules_visible = true;
    assert_eq!(d.mikro_leds(&v, now).get("LED_PLUGIN"), Some(deckard_mikro::leds::ON));

    // Pressing SHIFT + PLUG-IN while already in Channel Macro toggles PLUG-IN overlay off.
    v.control.modules_visible = false;
    v.control.page_group = 3;
    d.button("BUTTON_SHIFT", true, &v, now);
    assert!(actions(d.button("BUTTON_PLUGIN", true, &v, now)).is_empty());
    d.button("BUTTON_PLUGIN", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_PLUGIN"), Some(deckard_mikro::leds::DIM));
}

#[test]
fn mikro_plugin_module_loader_filters_loads_and_opens_editor() {
    let mut d = mikro();
    let mut v = mikro_view();
    v.control.level = 2; // Sound
    v.control.modules_visible = true;
    v.control.slot_count = 1;
    v.control.slot_index = Some(0);
    v.control.chain = vec![crate::SlotView { name: "Raum".into(), instrument: false, active: true, plugin: true, picture: None }];
    v.control.modules = vec![
        crate::ModuleView { name: "Sampler".into(), instrument: true, format: "Internal".into(), vendor: "Native Instruments".into() },
        crate::ModuleView { name: "EQ".into(), instrument: false, format: "Internal".into(), vendor: "Native Instruments".into() },
        crate::ModuleView { name: "Raum".into(), instrument: false, format: "VST3".into(), vendor: "Native Instruments".into() },
        crate::ModuleView { name: "Microcosm".into(), instrument: false, format: "VST3".into(), vendor: "Panic Signals".into() },
    ];
    let now = Instant::now();

    d.button("BUTTON_PLUGIN", true, &v, now);
    d.button("BUTTON_PLUGIN", false, &v, now);

    // SHIFT + encoder press opens desktop plugin editor window when slot.plugin is true.
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)), vec![Action::SlotEdit("edit")]);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);

    // ENCODER_PRESS opens Module Loader on slot 1 -> starts on MODULE at "(none)" -> pressing again removes slot.
    assert!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)).is_empty());
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
    assert_eq!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)), vec![Action::LoadModule(None)]);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);

    // Re-open on slot 1 (first sound slot allows switching KIND -> FX, FORMAT -> VST3, VENDOR -> Panic Signals).
    d.button("BUTTON_ENCODER_PRESS", true, &v, now);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
    // LEFT from MODULE -> FORMAT (Internal), LEFT -> KIND (INST) -> turn +1 -> FX.
    d.button("BUTTON_LEFT", true, &v, now);
    d.button("BUTTON_LEFT", false, &v, now);
    d.button("BUTTON_LEFT", true, &v, now);
    d.button("BUTTON_LEFT", false, &v, now);
    d.encoder(1, &v, now); // KIND: EFFECT
    // RIGHT -> FORMAT (Internal) -> turn +1 -> VST3.
    d.button("BUTTON_RIGHT", true, &v, now);
    d.button("BUTTON_RIGHT", false, &v, now);
    d.encoder(1, &v, now); // FORMAT: VST3
    // RIGHT -> VENDOR (NI) -> turn +1 -> Panic Signals.
    d.button("BUTTON_RIGHT", true, &v, now);
    d.button("BUTTON_RIGHT", false, &v, now);
    d.encoder(1, &v, now); // VENDOR: Panic Signals
    // RIGHT -> MODULE, scroll +1 -> Microcosm, push -> LoadModule(Some(3)).
    d.button("BUTTON_RIGHT", true, &v, now);
    d.button("BUTTON_RIGHT", false, &v, now);
    d.encoder(1, &v, now);
    assert_eq!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)), vec![Action::LoadModule(Some(3))]);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);

    // On empty "+" slot (slot_index = None): format persists as VST3; switching FORMAT back to Internal starts at
    // first effect (index 1 = EQ, skipping "(none)"), and encoder push on FORMAT jumps to MODULE before loading EQ.
    v.control.slot_index = None;
    d.button("BUTTON_ENCODER_PRESS", true, &v, now);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
    // Navigate LEFT from MODULE -> VENDOR -> FORMAT, turn -1 back to Internal, push -> MODULE, push -> LoadModule(Some(1)).
    d.button("BUTTON_LEFT", true, &v, now);
    d.button("BUTTON_LEFT", false, &v, now);
    d.button("BUTTON_LEFT", true, &v, now);
    d.button("BUTTON_LEFT", false, &v, now);
    d.encoder(-1, &v, now); // FORMAT: Internal
    assert!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)).is_empty()); // jumps back to MODULE
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
    assert_eq!(actions(d.button("BUTTON_ENCODER_PRESS", true, &v, now)), vec![Action::LoadModule(Some(1))]);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
}

#[test]
fn mikro_browse_utf8_names_do_not_error_on_screen() {
    let mut d = mikro();
    let mut v = mikro_view();
    v.browser.kind = 6;
    // Multi-byte UTF-8 characters straddling the 21-char truncation boundary.
    v.browser.results = vec![("12345678901234567890–—•üé™".into(), false)];
    let now = Instant::now();

    d.button("BUTTON_BROWSE", true, &v, now);
    d.button("BUTTON_BROWSE", false, &v, now);
    let scr = d.mikro_screen(&v, now);
    assert_eq!(d.error(), None, "UTF-8 truncation must not error in mlua");
    assert!(scr.buf.iter().any(|&b| b != 0xff));
}

#[test]
fn mikro_browse_marquee_pauses_then_scrolls_long_names_and_resets() {
    use std::time::Duration;

    let mut d = mikro();
    let mut v = mikro_view();
    v.browser.kind = 6; // 0-based -> Lua 7 ("ONE-SHOTS")
    v.browser.focus = 0; // 0-based -> Lua 1
    let long_a = "ASURA Kick Vintage Analog Tail";
    let long_b = "SUBLIMINAL 808 Deep Saturated";
    v.browser.results = vec![(long_a.into(), false)];
    let t0 = Instant::now();

    d.button("BUTTON_BROWSE", true, &v, t0);
    d.button("BUTTON_BROWSE", false, &v, t0);

    // At t=0 and within the 0.75s initial pause, the display stays at scroll_px=0.
    let s0 = d.mikro_screen(&v, t0);
    let s_pause = d.mikro_screen(&v, t0 + Duration::from_millis(500));
    assert_eq!(d.error(), None);
    assert!(s0 == s_pause, "marquee should hold steady during initial pause");

    let mut want0 = deckard_mikro::Screen::new();
    want0.status_bar("BROWSE", "ONE-SHOTS");
    want0.draw_text(13, 2, "> RESULTS: 1/1", true);
    want0.draw_text_scrolled(23, 2, 125, 0, long_a, true);
    assert!(s0 == want0, "initial browse frame should start un-scrolled");

    // At t=2.0s (1.25s after pause * 26 px/s = 32 px), text has scrolled left.
    let s_scroll = d.mikro_screen(&v, t0 + Duration::from_secs(2));
    assert!(s_scroll != s0, "marquee should advance after initial pause");

    let mut want_scroll = deckard_mikro::Screen::new();
    want_scroll.status_bar("BROWSE", "ONE-SHOTS");
    want_scroll.draw_text(13, 2, "> RESULTS: 1/1", true);
    want_scroll.draw_text_scrolled(23, 2, 125, 32, long_a, true);
    assert!(s_scroll == want_scroll, "scrolled frame should match 32px ticker offset");

    // Changing the hovered result resets its marquee back to offset 0.
    v.browser.results = vec![(long_b.into(), false)];
    let s_reset = d.mikro_screen(&v, t0 + Duration::from_secs(2));
    let mut want_reset = deckard_mikro::Screen::new();
    want_reset.status_bar("BROWSE", "ONE-SHOTS");
    want_reset.draw_text(13, 2, "> RESULTS: 1/1", true);
    want_reset.draw_text_scrolled(23, 2, 125, 0, long_b, true);
    assert!(s_reset == want_reset, "changing item should reset marquee offset to 0");
}

#[test]
fn mikro_group_hold_add_bank_delete_duplicate_solo_mute() {
    use deckard_mikro::leds::{SHADE_DIM, WHITE, colour};

    let mut d = mikro();
    let mut v = mikro_view();
    // Expand to 10 groups (A..J) so banks A-H and I-P can be cycled.
    while v.groups.len() < 10 {
        let idx = v.groups.len();
        v.groups.push(crate::GroupView { name: format!("G{}", idx + 1), color: idx % 16, ..Default::default() });
    }
    let now = Instant::now();

    d.button("BUTTON_GROUP", true, &v, now);
    let leds = d.mikro_leds(&v, now);
    // Pad 11 (#groups + 1) glows dim WHITE to add a group.
    assert_eq!(leds.get("LED_PAD_11"), Some(colour(WHITE, SHADE_DIM)));
    let (c11, r11) = pad_rc(11);
    assert_eq!(actions(d.pad(c11, r11, true, &v, now)), vec![Action::AddGroup]);
    d.pad(c11, r11, false, &v, now);

    // RIGHT (or encoder turn) while holding GROUP shifts bank by 8 (I-P land on pads 1-2, "+" on pad 3).
    d.button("BUTTON_RIGHT", true, &v, now);
    d.button("BUTTON_RIGHT", false, &v, now);
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_PAD_3"), Some(colour(WHITE, SHADE_DIM)));
    let (c1, r1) = pad_rc(1);
    assert_eq!(actions(d.pad(c1, r1, true, &v, now)), vec![Action::FocusGroup(8)]);
    d.pad(c1, r1, false, &v, now);

    // Encoder -1 / LEFT returns to bank 1.
    d.encoder(-1, &v, now);
    // ERASE + GROUP + pad 2 -> DeleteGroup(1).
    d.button("BUTTON_TRANSPORT_ERASE", true, &v, now);
    let (c2, r2) = pad_rc(2);
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), vec![Action::DeleteGroup(1)]);
    d.pad(c2, r2, false, &v, now);
    d.button("BUTTON_TRANSPORT_ERASE", false, &v, now);

    // SOLO + GROUP + pad 2 -> SoloGroup(1); MUTE + GROUP + pad 2 -> MuteGroup(1).
    d.button("BUTTON_SOLO", true, &v, now);
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), vec![Action::SoloGroup(1)]);
    d.pad(c2, r2, false, &v, now);
    d.button("BUTTON_SOLO", false, &v, now);

    d.button("BUTTON_MUTE", true, &v, now);
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), vec![Action::MuteGroup(1)]);
    d.pad(c2, r2, false, &v, now);
    d.button("BUTTON_MUTE", false, &v, now);

    // DUPLICATE + GROUP + pad 1 -> pad 2 -> CopyGroup(0, 1).
    d.button("BUTTON_DUPLICATE", true, &v, now);
    d.button("BUTTON_DUPLICATE", false, &v, now);
    assert!(actions(d.pad(c1, r1, true, &v, now)).is_empty());
    d.pad(c1, r1, false, &v, now);
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), vec![Action::CopyGroup { from: 0, to: 1 }]);
    d.pad(c2, r2, false, &v, now);
    d.button("BUTTON_GROUP", false, &v, now);
}

#[test]
fn mikro_sampling_auto_lock_note_repeat_grid_and_section() {
    let mut d = mikro();
    let mut v = mikro_view();
    let now = Instant::now();

    // SAMPLING toggles Action::Sampling and lights LED_SAMPLING.
    assert_eq!(actions(d.button("BUTTON_SAMPLING", true, &v, now)), vec![Action::Sampling(true)]);
    d.button("BUTTON_SAMPLING", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_SAMPLING"), Some(deckard_mikro::leds::ON));
    v.sampling.no_sample = Some(crate::NoSample { title: "NO SAMPLE".into(), detail: "Load one".into() });
    let scr = d.mikro_screen(&v, now);
    assert!(scr.buf.iter().any(|&b| b != 0xff));
    assert_eq!(actions(d.button("BUTTON_SAMPLING", true, &v, now)), vec![Action::Sampling(false)]);
    d.button("BUTTON_SAMPLING", false, &v, now);
    v.sampling.no_sample = None;

    // AUTO toggles AutoWrite and reflects LED_AUTO.
    assert_eq!(actions(d.button("BUTTON_AUTO", true, &v, now)), vec![Action::AutoWrite(true)]);
    d.button("BUTTON_AUTO", false, &v, now);
    v.auto_write = true;
    assert_eq!(d.mikro_leds(&v, now).get("LED_AUTO"), Some(deckard_mikro::leds::ON));

    // LOCK toggles Action::Lock("button", 0) and reflects LED_LOCK.
    assert_eq!(actions(d.button("BUTTON_LOCK", true, &v, now)), vec![Action::Lock("button", 0)]);
    d.button("BUTTON_LOCK", false, &v, now);
    v.lock.active = true;
    assert_eq!(d.mikro_leds(&v, now).get("LED_LOCK"), Some(deckard_mikro::leds::ON));

    // NOTE_REPEAT toggles Action::Repeat("on", 1.0/0.0) and reflects LED_NOTE_REPEAT.
    assert_eq!(actions(d.button("BUTTON_NOTE_REPEAT", true, &v, now)), vec![Action::Repeat("on", 1.0)]);
    d.button("BUTTON_NOTE_REPEAT", false, &v, now);
    v.note_repeat.active = true;
    assert_eq!(d.mikro_leds(&v, now).get("LED_NOTE_REPEAT"), Some(deckard_mikro::leds::ON));

    // SHIFT + SCENE toggles Song view.
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.button("BUTTON_SCENE", true, &v, now)), vec![Action::SongView(true)]);
    d.button("BUTTON_SCENE", false, &v, now);

    // SHIFT + FOLLOW opens Grid overlay; pad 5 picks 1/16 step grid and enables step_snap.
    d.button("BUTTON_FOLLOW", true, &v, now);
    d.button("BUTTON_FOLLOW", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
    let (c5, r5) = pad_rc(5);
    assert_eq!(
        actions(d.pad(c5, r5, true, &v, now)),
        vec![Action::Grid("step", 4), Action::Grid("step_snap", 1)]
    );
    d.pad(c5, r5, false, &v, now);
    v.grid.step = 4;
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Grid("step", 5)]);
    // Pressing FOLLOW exits Grid overlay.
    d.button("BUTTON_FOLLOW", true, &v, now);
    d.button("BUTTON_FOLLOW", false, &v, now);
}

#[test]
fn mikro_parameter_buttons_volume_velocity_swing_position_tempo_tune() {
    let mut d = mikro();
    let mut v = mikro_view();
    v.master_volume = 0.5;
    v.velocity = 100;
    v.swing = 20.0;
    v.tempo = 120.0;
    v.position = Some(480);
    v.pattern_position = Some(480);
    v.pattern_length = 3840;
    v.groups[0].sounds[0].mix.tune = Some(2.0);
    let now = Instant::now();

    // VOLUME vs SHIFT + VOLUME (VELOCITY)
    d.button("BUTTON_VOLUME", true, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Volume(0.55)]);
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::VelocityValue(105)]);
    d.button("BUTTON_SHIFT", false, &v, now);
    d.button("BUTTON_VOLUME", false, &v, now);

    // SWING vs SHIFT + SWING (POSITION)
    d.button("BUTTON_SWING", true, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Swing(25.0)]);
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Seek(720)]);
    d.button("BUTTON_SHIFT", false, &v, now);
    d.button("BUTTON_SWING", false, &v, now);

    // TEMPO vs SHIFT + TEMPO (TUNE)
    d.button("BUTTON_TEMPO", true, &v, now);
    assert_eq!(actions(d.encoder(2, &v, now)), vec![Action::Tempo(122.0)]);
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(
        actions(d.encoder(-1, &v, now)),
        vec![Action::SetTune { group: 0, sound: Some(0), semitones: 1.0 }]
    );
    d.button("BUTTON_SHIFT", false, &v, now);
    d.button("BUTTON_TEMPO", false, &v, now);
}

#[test]
fn mikro_shift_pads_1_to_16_silkscreen_shortcuts() {
    let mut d = mikro();
    let v = mikro_view();
    let now = Instant::now();

    d.button("BUTTON_SHIFT", true, &v, now);
    assert!(d.pad_map(&v, now).pads.iter().all(|p| p.is_empty()), "SHIFT must silence pads for silkscreen shortcuts");

    let expected = [
        (1, Action::Edit("undo")),
        (2, Action::Edit("redo")),
        (3, Action::Edit("undo_step")),
        (4, Action::Edit("redo_step")),
        (5, Action::Events("quantize", 0)),
        (6, Action::Events("quantize_half", 0)),
        (7, Action::Events("nudge_shown", -1)),
        (8, Action::Events("nudge_shown", 1)),
        (9, Action::Events("clear", 0)),
        (10, Action::Edit("clear_modulation")),
        (11, Action::Edit("copy")),
        (12, Action::Edit("paste")),
        (13, Action::Events("transpose", -1)),
        (14, Action::Events("transpose", 1)),
        (15, Action::Events("transpose", -12)),
        (16, Action::Events("transpose", 12)),
    ];

    for (pad, want) in expected {
        let (c, r) = pad_rc(pad);
        assert_eq!(actions(d.pad(c, r, true, &v, now)), vec![want], "SHIFT + Pad {pad}");
        d.pad(c, r, false, &v, now);
    }

    d.button("BUTTON_SHIFT", false, &v, now);
}

#[test]
fn mikro_stop_led_lit_while_playing_and_solo_sound_dispatches_action() {
    let mut d = mikro();
    let mut v = mikro_view();
    let now = Instant::now();

    // When stopped: PLAY dim, STOP backlight (off).
    v.playing = false;
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_TRANSPORT_PLAY"), Some(deckard_mikro::leds::DIM));
    assert_eq!(leds.get("LED_TRANSPORT_STOP"), Some(deckard_mikro::leds::BACKLIGHT));

    // When playing: PLAY on, STOP stays lit (dim).
    v.playing = true;
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_TRANSPORT_PLAY"), Some(deckard_mikro::leds::ON));
    assert_eq!(leds.get("LED_TRANSPORT_STOP"), Some(deckard_mikro::leds::DIM));

    // SOLO + Pad 2 -> SoloSound(1) without touching MUTE.
    d.button("BUTTON_SOLO", true, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_SOLO"), Some(deckard_mikro::leds::ON));
    assert!(d.pad_map(&v, now).pads.iter().all(|p| p.is_empty()), "SOLO held must silence pads");
    let (c2, r2) = pad_rc(2);
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), vec![Action::SoloSound(1)]);
    d.pad(c2, r2, false, &v, now);
    d.button("BUTTON_SOLO", false, &v, now);
}

#[test]
fn mikro_keyboard_mode_lights_octave_arrow_leds() {
    use deckard_mikro::leds::{BACKLIGHT, DIM, ON};

    let mut d = mikro();
    let v = mikro_view();
    let now = Instant::now();

    // Pad mode: arrows stay at ambient backlight.
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_LEFT"), Some(BACKLIGHT));
    assert_eq!(leds.get("LED_RIGHT"), Some(BACKLIGHT));

    // Keyboard mode at octave 0: both LEFT and RIGHT glow (DIM), brightening to ON while held.
    d.button("BUTTON_KEYBOARD", true, &v, now);
    d.button("BUTTON_KEYBOARD", false, &v, now);
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_LEFT"), Some(DIM));
    assert_eq!(leds.get("LED_RIGHT"), Some(DIM));

    d.button("BUTTON_RIGHT", true, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_RIGHT"), Some(ON));
    d.button("BUTTON_RIGHT", false, &v, now);

    // Pushing right to max octave (+3) turns LED_RIGHT to backlight while LED_LEFT stays lit.
    for _ in 0..2 {
        d.button("BUTTON_RIGHT", true, &v, now);
        d.button("BUTTON_RIGHT", false, &v, now);
    }
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_LEFT"), Some(DIM));
    assert_eq!(leds.get("LED_RIGHT"), Some(BACKLIGHT));

    // Stepping down to min octave (-3) turns LED_LEFT to backlight while LED_RIGHT stays lit.
    for _ in 0..6 {
        d.button("BUTTON_LEFT", true, &v, now);
        d.button("BUTTON_LEFT", false, &v, now);
    }
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_LEFT"), Some(BACKLIGHT));
    assert_eq!(leds.get("LED_RIGHT"), Some(DIM));
}

#[test]
fn mikro_step_mode_lights_only_active_steps_in_group_color() {
    use deckard_mikro::leds::{BACKLIGHT, DIM, SHADE_BRIGHT, WHITE, colour};

    let mut d = mikro();
    let mut v = mikro_view(); // group 0 color = 2, sound = 0
    v.grid.step_ticks = 240;
    v.pattern_start = 0;
    v.pattern_length = 3840;
    v.playing = false;
    v.pattern_position = Some(0);
    let now = Instant::now();

    d.button("BUTTON_STEP", true, &v, now);
    d.button("BUTTON_STEP", false, &v, now);

    // Stopped, no notes: ALL 16 pads must be completely dark (0), even when parked at step 1.
    let leds = d.mikro_leds(&v, now);
    for pad in 1..=16 {
        assert_eq!(leds.get(&format!("LED_PAD_{pad}")), Some(0), "empty step pad {pad} must stay dark");
    }
    assert_eq!(leds.get("LED_LEFT"), Some(BACKLIGHT));
    assert_eq!(leds.get("LED_RIGHT"), Some(BACKLIGHT));

    // Notes for focused sound (sound 0) on step 1 & step 5; step 3 belongs to sound 1.
    v.notes = vec![
        crate::NoteView { sound: 0, position: 0, length: 240, pitch: 60, velocity: 100, ..Default::default() },
        crate::NoteView { sound: 1, position: 480, length: 240, pitch: 60, velocity: 100, ..Default::default() },
        crate::NoteView { sound: 0, position: 960, length: 240, pitch: 60, velocity: 100, ..Default::default() },
    ];
    let grp_color = v.groups[0].color as i64;
    let leds = d.mikro_leds(&v, now);
    for pad in 1..=16 {
        let want = if pad == 1 || pad == 5 { colour(grp_color, SHADE_BRIGHT) } else { 0 };
        assert_eq!(leds.get(&format!("LED_PAD_{pad}")), Some(want), "pad {pad}");
    }

    // While playing at step 3 (480 ticks): step 3 chaser lights white, steps 1 & 5 stay group color, others 0.
    v.playing = true;
    v.pattern_position = Some(480);
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_PAD_1"), Some(colour(grp_color, SHADE_BRIGHT)));
    assert_eq!(leds.get("LED_PAD_3"), Some(colour(WHITE, SHADE_BRIGHT)));
    assert_eq!(leds.get("LED_PAD_5"), Some(colour(grp_color, SHADE_BRIGHT)));
    assert_eq!(leds.get("LED_PAD_2"), Some(0));

    // Multi-bar pattern enables RIGHT arrow paging to bar 2.
    v.playing = false;
    v.pattern_length = 2 * 3840;
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_RIGHT"), Some(DIM));
    d.button("BUTTON_RIGHT", true, &v, now);
    d.button("BUTTON_RIGHT", false, &v, now);
    let (c1, r1) = pad_rc(1);
    assert_eq!(actions(d.pad(c1, r1, true, &v, now)), vec![Action::StepToggle { at: 16 * 240, length: 240, velocity: 100 }]);
    d.pad(c1, r1, false, &v, now);
}

#[test]
fn mikro_perform_and_shift_perform_selects_perform_fx() {
    use deckard_mikro::leds::{DIM, ON, SHADE_BRIGHT, SHADE_DIM, colour};

    let mut d = mikro();
    let mut v = mikro_view();
    let now = Instant::now();

    // Plain PERFORM activates strip mode and lights LED_PERFORM.
    assert!(actions(d.button("BUTTON_PERFORM", true, &v, now)).is_empty());
    d.button("BUTTON_PERFORM", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_PERFORM"), Some(ON));

    // SHIFT + PERFORM emits PerformFx, opens selector overlay, silences pads, and renders OLED.
    d.button("BUTTON_SHIFT", true, &v, now);
    assert_eq!(actions(d.button("BUTTON_PERFORM", true, &v, now)), vec![Action::PerformFx]);
    d.button("BUTTON_PERFORM", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
    assert!(d.pad_map(&v, now).pads.iter().all(|p| p.is_empty()), "perform selector must silence pads");

    v.groups[0].perform = Some((false, 0.5));
    v.groups[0].perform_mode = Some((0, "Filter".into()));
    let grp_color = v.groups[0].color as i64;
    let leds = d.mikro_leds(&v, now);
    assert_eq!(leds.get("LED_PERFORM"), Some(ON));
    assert_eq!(leds.get("LED_PAD_1"), Some(colour(grp_color, SHADE_BRIGHT)));
    assert_eq!(leds.get("LED_PAD_3"), Some(colour(grp_color, SHADE_DIM)));
    assert_eq!(leds.get("LED_PAD_9"), Some(0));

    // Encoder turn, RIGHT button, and Pads 1-8 pick PerformMode (0-based in Action).
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::PerformMode { group: 0, mode: 1 }]);
    assert_eq!(actions(d.button("BUTTON_RIGHT", true, &v, now)), vec![Action::PerformMode { group: 0, mode: 1 }]);
    d.button("BUTTON_RIGHT", false, &v, now);
    let (c3, r3) = pad_rc(3);
    assert_eq!(actions(d.pad(c3, r3, true, &v, now)), vec![Action::PerformMode { group: 0, mode: 2 }]);
    d.pad(c3, r3, false, &v, now);
    assert!(d.mikro_screen(&v, now).buf.iter().any(|&b| b != 0xff));

    // Encoder press closes overlay while keeping PERFORM strip mode active.
    d.button("BUTTON_ENCODER_PRESS", true, &v, now);
    d.button("BUTTON_ENCODER_PRESS", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_PERFORM"), Some(ON));
    assert_eq!(d.mikro_leds(&v, now).get("LED_TS_13"), Some(DIM));
    assert_eq!(d.pad_map(&v, now).pads.iter().filter(|p| !p.is_empty()).count(), 16);
}

#[test]
fn mikro_quick_param_tap_latch_hold_targeting_and_oled() {
    use deckard_mikro::leds::{DIM, ON};

    let mut d = mikro();
    let mut v = mikro_view();
    v.groups.push(crate::GroupView { name: "G2".into(), color: 4, ..Default::default() });
    v.groups[0].mix.level = 0.0;
    v.groups[1].mix.level = 0.0;
    v.groups[1].mix.tune = Some(0.0);
    v.groups[0].sounds[0].mix.level = 0.0;
    v.groups[0].sounds[1].mix.level = 0.0;
    v.groups[0].sounds[1].mix.tune = Some(1.5);
    v.master_volume = 0.5;
    v.tempo = 120.0;
    v.swing = 20.0;
    v.velocity = 100;
    v.pattern_position = Some(240);
    v.pattern_length = 3840;
    let now = Instant::now();

    // 1. Tap TEMPO -> latches on (LED_TEMPO ON), encoder adjusts without unlatching, second tap unlatches.
    d.button("BUTTON_TEMPO", true, &v, now);
    d.button("BUTTON_TEMPO", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_TEMPO"), Some(ON));
    assert_eq!(d.mikro_leds(&v, now).get("LED_VOLUME"), Some(DIM));
    assert_eq!(actions(d.encoder(4, &v, now)), vec![Action::Tempo(124.0)]);
    assert_eq!(d.mikro_leds(&v, now).get("LED_TEMPO"), Some(ON), "latched TEMPO stays ON after encoder turn");
    assert!(d.mikro_screen(&v, now).buf.iter().any(|&b| b != 0xff));

    // Switching directly to SWING unlatches TEMPO and latches SWING; tapping another mode (PAD_MODE) unlatches SWING.
    d.button("BUTTON_SWING", true, &v, now);
    d.button("BUTTON_SWING", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_TEMPO"), Some(DIM));
    assert_eq!(d.mikro_leds(&v, now).get("LED_SWING"), Some(ON));
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Swing(25.0)]);
    assert!(d.mikro_screen(&v, now).buf.iter().any(|&b| b != 0xff));

    d.button("BUTTON_PAD_MODE", true, &v, now);
    d.button("BUTTON_PAD_MODE", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_SWING"), Some(DIM));

    // 2. Tap VOLUME -> latches Master volume; holding Pad 2 adjusts Sound 2; holding GROUP + Pad 2 adjusts Group 2.
    d.button("BUTTON_VOLUME", true, &v, now);
    d.button("BUTTON_VOLUME", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_VOLUME"), Some(ON));
    assert_eq!(actions(d.encoder(1, &v, now)), vec![Action::Volume(0.55)]);
    assert!(d.mikro_screen(&v, now).buf.iter().any(|&b| b != 0xff));

    let (c2, r2) = pad_rc(2);
    assert!(actions(d.pad(c2, r2, true, &v, now)).is_empty(), "pad hold in latched VOLUME targets sound cleanly");
    assert_eq!(actions(d.encoder(-2, &v, now)), vec![Action::ChannelLevel { group: 0, sound: Some(1), db: -2.0 }]);
    assert!(d.mikro_screen(&v, now).buf.iter().any(|&b| b != 0xff));
    d.pad(c2, r2, false, &v, now);

    d.button("BUTTON_GROUP", true, &v, now);
    assert!(actions(d.pad(c2, r2, true, &v, now)).is_empty());
    assert_eq!(actions(d.encoder(3, &v, now)), vec![Action::ChannelLevel { group: 1, sound: None, db: 3.0 }]);
    assert!(d.mikro_screen(&v, now).buf.iter().any(|&b| b != 0xff));
    d.pad(c2, r2, false, &v, now);
    d.button("BUTTON_GROUP", false, &v, now);

    // Second tap on VOLUME unlatches it.
    d.button("BUTTON_VOLUME", true, &v, now);
    d.button("BUTTON_VOLUME", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_VOLUME"), Some(DIM));

    // 3. SHIFT + TEMPO latches TUNE (LED_TEMPO ON even after SHIFT & TEMPO release); Sound 2 / Group 2 targeting works.
    d.button("BUTTON_SHIFT", true, &v, now);
    d.button("BUTTON_TEMPO", true, &v, now);
    d.button("BUTTON_TEMPO", false, &v, now);
    d.button("BUTTON_SHIFT", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_TEMPO"), Some(ON));
    assert_eq!(actions(d.pad(c2, r2, true, &v, now)), Vec::<Action>::new());
    assert_eq!(actions(d.encoder(2, &v, now)), vec![Action::SetTune { group: 0, sound: Some(1), semitones: 3.5 }]);
    assert!(d.mikro_screen(&v, now).buf.iter().any(|&b| b != 0xff));
    d.pad(c2, r2, false, &v, now);

    d.button("BUTTON_GROUP", true, &v, now);
    d.pad(c2, r2, true, &v, now);
    assert_eq!(actions(d.encoder(-3, &v, now)), vec![Action::SetTune { group: 1, sound: None, semitones: -3.0 }]);
    assert!(d.mikro_screen(&v, now).buf.iter().any(|&b| b != 0xff));
    d.pad(c2, r2, false, &v, now);
    d.button("BUTTON_GROUP", false, &v, now);

    // Tap TEMPO unlatches TUNE back to idle.
    d.button("BUTTON_TEMPO", true, &v, now);
    d.button("BUTTON_TEMPO", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_TEMPO"), Some(DIM));

    // 4. SHIFT + VOLUME (VELOCITY) and SHIFT + SWING (POSITION) latch cleanly and render OLED.
    for (btn, want) in [("BUTTON_VOLUME", Action::VelocityValue(105)), ("BUTTON_SWING", Action::Seek(480))] {
        d.button("BUTTON_SHIFT", true, &v, now);
        d.button(btn, true, &v, now);
        d.button(btn, false, &v, now);
        d.button("BUTTON_SHIFT", false, &v, now);
        assert_eq!(actions(d.encoder(1, &v, now)), vec![want]);
        assert!(d.mikro_screen(&v, now).buf.iter().any(|&b| b != 0xff));
        d.button(btn, true, &v, now);
        d.button(btn, false, &v, now);
    }
}

#[test]
fn mikro_tap_button_led_is_lit_and_brightens_on_hold_or_metronome() {
    use deckard_mikro::leds::{DIM, ON};

    let mut d = mikro();
    let mut v = mikro_view();
    let now = Instant::now();

    assert_eq!(d.mikro_leds(&v, now).get("LED_TAP"), Some(DIM), "TAP stays lit (dim) at idle");

    d.button("BUTTON_TAP", true, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_TAP"), Some(ON), "TAP glows bright while held");
    d.button("BUTTON_TAP", false, &v, now);
    assert_eq!(d.mikro_leds(&v, now).get("LED_TAP"), Some(DIM));

    v.metronome = true;
    assert_eq!(d.mikro_leds(&v, now).get("LED_TAP"), Some(ON), "TAP glows bright when metronome is active");
}
