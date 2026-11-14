//! `deckard.state()`: a plain table to the scripts, though its parts are made as they're
//! read; and what a frame of the controller scripts costs on a full project while it
//! plays (`frame_cost`, a benchmark: the MK3's `tick`, `pads`, `leds` and screens, then the
//! JAM's `tick`, `leds` and `pads`, as the app runs them, the meters and play position
//! moving every frame). `cargo test -p deckard-surface --release frame_cost -- --ignored
//! --nocapture`, with `DECKARD_CONTROLLER_DIR` for a user's scripts.

use std::time::{Duration, Instant};

use crate::device::{Device, PadMap};
use crate::tests_support::view;
use crate::*;

/// `code`'s value, the scripts seeing `v`.
fn eval<T: mlua::FromLuaMulti>(s: &Surface, v: &View, code: &str) -> T {
    s.with(v, None, |run| run.lua.load(code).eval::<T>()).unwrap().0.unwrap()
}

#[test]
fn pairs_sees_every_part() {
    let s = Surface::new();
    let v = view();
    let missing: Vec<String> = eval(
        &s,
        &v,
        r#"local seen, out = {}, {}
           for k in pairs(deckard.state()) do seen[k] = true end
           for k in pairs(deckard.state().groups[1]) do seen["group." .. k] = true end
           for _, k in ipairs({ "project", "scenes", "lfos", "mod_slots", "sections", "notes", "browser", "recent",
                                "settings", "lock", "sampling", "note_repeat", "groups", "control",
                                "group.name", "group.patterns", "group.sounds", "group.meter" }) do
             if not seen[k] then out[#out + 1] = k end
           end
           return out"#,
    );
    assert!(missing.is_empty(), "{missing:?}");
    // (As a profile's copy of the state does it.)
    let same: bool = eval(
        &s,
        &v,
        r#"local s, copy = deckard.state(), {}
           for k, x in pairs(s) do copy[k] = x end
           return copy.groups == s.groups and copy.control == s.control and copy.groups[2].sounds[1].name == "Sound 1""#,
    );
    assert!(same);
}

#[test]
fn what_a_script_sets_stays() {
    let s = Surface::new();
    let v = view();
    let ok: bool = eval(
        &s,
        &v,
        r#"local s = deckard.state()
           s.notes = "mine"
           local groups = s.groups
           s.groups = nil
           s.extra = 1
           local g = groups[1]
           g.sounds = false
           local seen = {}
           for k, x in pairs(s) do seen[k] = x end
           return s.notes == "mine" and s.groups == nil and seen.notes == "mine" and seen.groups == nil
             and s.extra == 1 and s.nothing == nil and s[1] == nil and g.sounds == false
             and deckard.state() == s and groups[1] == g"#,
    );
    assert!(ok);
}

#[test]
fn one_table_until_the_view_changes() {
    let s = Surface::new();
    let mut v = view();
    let same: bool = eval(&s, &v, "kept = deckard.state() return deckard.state().groups == kept.groups");
    assert!(same);
    v.groups[0].name = "Changed".into();
    v.groups[0].sounds[0].name = "New".into();
    // A table kept from before reads the view it was made for, read first now or not.
    let (old, old_sound, new, fresh): (String, String, String, bool) =
        eval(&s, &v, "local s = deckard.state() return kept.groups[1].name, kept.groups[1].sounds[1].name, s.groups[1].name, s ~= kept");
    assert_eq!((old.as_str(), old_sound.as_str(), new.as_str(), fresh), ("Drums", "Sound 1", "Changed", true));
}

#[test]
fn a_device_sees_the_view_its_pads_are_asked_with() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("devices/test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("device.lua"), r#"return { name = "Test", ports = { daw = { "Test DAW" } } }"#).unwrap();
    std::fs::write(
        dir.join("main.lua"),
        "return { tick = function() deckard.state() end,
                  pads = function() return { velocity = 100, [1] = { deckard.state().group, 1, 60 } } end }",
    )
    .unwrap();
    let mut d = Device::new("test", Some(tmp.path().to_path_buf()));
    assert_eq!(d.error(), None);
    let (mut v, now) = (view(), Instant::now());
    d.tick(&v, now);
    v.group = 1;
    let PadMap { pads, .. } = d.pad_map(&v, now);
    assert_eq!(pads[0], [(1, 0, 60)]);
}

/// Eight groups of sixteen sounds, each with plug-ins, patterns and scenes; a pattern
/// of notes; a long chain and plug-in page focused.
fn big_view() -> View {
    let mut v = view();
    let mix = |n: usize| MixView {
        level: 0.0,
        pan: 0.0,
        meter: (0.1 * n as f32, 0.1),
        headroom: 0.5,
        plugins: vec![(true, true), (false, true), (false, false)],
        aux: [0.0, 0.0],
        swing: 0.0,
        tune: Some(0.0),
    };
    let base = v.groups[0].sounds[0].clone();
    v.groups = (0..8)
        .map(|g| GroupView {
            name: format!("Group {g}"),
            rgb: [255, 66, 62],
            color: g,
            mix: mix(g),
            sounds: (0..16)
                .map(|s| SoundView { name: format!("Sound {g}.{s}"), used: true, mix: mix(s), base_key: 60, ..base.clone() })
                .collect(),
            patterns: (0..16)
                .map(|p| PatternView { name: format!("Pattern {p}"), color: p, bars: 2.0, slot: p, ..Default::default() })
                .collect(),
            scene_pattern: Some(0),
            perform: Some((false, 0.5)),
            progress: Some(0.0),
            playing_progress: Some(0.0),
            ..Default::default()
        })
        .collect();
    v.scenes = (0..16)
        .map(|i| SceneView { name: format!("Scene {i}"), color: i, patterns: (0..8).map(|g| Some(g % 4)).collect(), ..Default::default() })
        .collect();
    v.sections = (0..16).map(|i| SectionView { scene: Some(i), start: i as i64 * 7680, length: 7680, auto: true }).collect();
    v.notes = (0..256)
        .map(|i| NoteView {
            sound: i % 16,
            position: (i * 240) as i64,
            length: 120,
            pitch: 60,
            velocity: 100,
            probability: 100,
            selected: false,
        })
        .collect();
    v.pattern_length = 2 * 3840;
    v.grid.step_ticks = 240;
    v.velocity = 100;
    v.scale_root = 60;
    v.playing = true;
    v.recent = (0..10).map(|i| format!("Project {i}")).collect();
    v.settings.values = (0..40).map(|i| (format!("key{i}"), format!("value {i}"))).collect();
    v
}

/// The play position and meters a frame later.
fn advance(v: &mut View, frame: usize) {
    let t = frame as f32 * 0.01;
    v.position = Some(frame as i64 * 40);
    v.pattern_position = Some(frame as i64 * 40 % v.pattern_length.max(1));
    v.progress = Some((frame % 100) as f64 / 100.0);
    v.master_meter = (t.sin().abs(), t.cos().abs());
    for g in &mut v.groups {
        g.mix.meter = (t.sin().abs(), t.cos().abs());
        g.progress = v.progress;
        for s in &mut g.sounds {
            s.mix.meter = (t.sin().abs(), t.cos().abs());
        }
    }
}

/// The scripts' folder to use: `DECKARD_CONTROLLER_DIR`, else DECKARD's own.
fn user_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("DECKARD_CONTROLLER_DIR").map(Into::into)
}

struct Timing {
    name: String,
    per_frame: Duration,
}

/// `frames` frames of the MK3 on `page` and the JAM, as `mk3_events` and `jam_events` run
/// them; the time each took per frame.
fn run(page: &str, frames: usize) -> Vec<Timing> {
    let mut v = big_view();
    let mut mk3 = Surface::with_user_dir(user_dir());
    assert_eq!(mk3.error(), None);
    mk3.eval::<Value>(&format!("deckard.show_page(\"{page}\")")).unwrap();
    let mut jam = Device::new("maschine_jam", user_dir());
    assert_eq!(jam.error(), None);
    let now = Instant::now();
    jam.open(&v, now);
    let (mut t_mk3, mut t_jam) = (Duration::ZERO, Duration::ZERO);
    for f in 0..frames {
        advance(&mut v, f);
        let now = Instant::now();
        let a = Instant::now();
        mk3.tick(&v, now);
        mk3.pads(&v);
        mk3.leds(&v);
        mk3.screen_picture(&v);
        let b = Instant::now();
        jam.tick(&v, now);
        jam.leds(&v, now);
        jam.pad_map(&v, now);
        let c = Instant::now();
        t_mk3 += b - a;
        t_jam += c - b;
    }
    assert_eq!(mk3.error(), None);
    assert_eq!(jam.error(), None);
    let n = frames as u32;
    vec![Timing { name: format!("mk3 {page}"), per_frame: t_mk3 / n }, Timing { name: "jam".into(), per_frame: t_jam / n }]
}

/// Building `deckard.state()` alone, a new view every time.
fn state_only(frames: usize) -> Duration {
    let mut v = big_view();
    let s = Surface::with_user_dir(user_dir());
    let start = Instant::now();
    for f in 0..frames {
        advance(&mut v, f);
        s.with(&v, None, |run| {
            run.lua.load("local s = deckard.state(); return s.groups[1].sounds[1].name").eval::<String>().unwrap();
        });
    }
    start.elapsed() / frames as u32
}

#[test]
#[ignore = "a benchmark: run with --release --ignored --nocapture"]
fn frame_cost() {
    const FRAMES: usize = 300;
    // (Warm up.)
    run("control", 20);
    for page in ["control", "step", "mixer", "pattern"] {
        for t in run(page, FRAMES) {
            println!("{:>14}: {:>8.1} µs a frame", t.name, t.per_frame.as_secs_f64() * 1e6);
        }
    }
    println!("{:>14}: {:>8.1} µs a view", "state() alone", state_only(FRAMES).as_secs_f64() * 1e6);
}
