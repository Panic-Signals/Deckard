//! The parity rig (docs/superpowers/specs/2026-10-25-controller-layer-design.md): the
//! same MK3 events go to an app running Maschine's controller scripts and to one running
//! DECKARD's own, and what comes out is compared: the app's state, the pads' routing,
//! every light and both screens. DECKARD's scripts are written from what this shows,
//! never from Maschine's source.
//!
//! `cargo test -p deckard --bin deckard parity_report -- --ignored --nocapture` writes
//! side-by-side pictures (Maschine above, DECKARD below, their difference last) to
//! `$TMPDIR/deckard-parity/`.

use deckard_mk3::Event;
use egui_kittest::Harness;

use crate::tests::harness;
use crate::{App, settings};

/// One app per layer, on the same project.
pub struct Rig {
    pub ni: Harness<'static, App>,
    pub dk: Harness<'static, App>,
}

/// What a layer gives after some events.
#[derive(Clone)]
pub struct Seen {
    pub state: Vec<(&'static str, String)>,
    pub route: String,
    pub leds: deckard_mk3::leds::Leds,
    pub screens: Option<Vec<[u8; 4]>>,
}

/// Where the two layers differ.
#[derive(Debug, Default)]
pub struct Diff {
    pub state: Vec<String>,
    pub route: Option<String>,
    pub leds: Vec<String>,
    /// The share of screen pixels that differ clearly (0..1), if both drew.
    pub screens: Option<f64>,
}

impl Rig {
    /// `None` without the test project or Maschine's scripts (`out/ni-res`).
    pub fn new() -> Option<Rig> {
        let ni = harness(true, false)?;
        if !matches!(ni.state().controller, Some(Ok(_))) {
            eprintln!("Maschine's scripts not found; skipping");
            return None;
        }
        let mut dk = harness(false, false)?;
        dk.state_mut().settings.controller = settings::ControllerChoice::Deckard;
        let mut rig = Rig { ni, dk };
        // Both start from what they show at rest.
        rig.send(&[]);
        Some(rig)
    }

    /// Events to both layers, then a few frames for each app.
    pub fn send(&mut self, events: &[Event]) {
        for h in [&mut self.ni, &mut self.dk] {
            for e in events {
                let mut requests = Vec::new();
                let app = h.state_mut();
                app.mk3_events(vec![e.clone()], &mut requests);
                app.handle(requests);
            }
            h.run_steps(2);
        }
    }

    pub fn press(&mut self, button: &'static str) {
        self.send(&[Event::Button(button, true), Event::Button(button, false)]);
    }

    pub fn hold(&mut self, button: &'static str, down: bool) {
        self.send(&[Event::Button(button, down)]);
    }

    /// Let `ms` pass (playback moves on), then a frame for each app.
    pub fn wait(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        // (A frame with an MK3 connected: the layers' timers run.)
        for h in [&mut self.ni, &mut self.dk] {
            let mut requests = Vec::new();
            let app = h.state_mut();
            app.mk3_events(Vec::new(), &mut requests);
            app.handle(requests);
        }
        self.send(&[]);
    }

    /// Steps as `PROBE` takes them: `press:BUTTON_MIX hold:BUTTON_SHIFT
    /// release:BUTTON_SHIFT pad:3 padon:3 padoff:3 knob:1:0.1 wheel:1 touch:down:0.5
    /// wait:500`.
    pub fn steps(&mut self, steps: &str) {
        for step in steps.split_whitespace() {
            let mut parts = step.split(':');
            let (what, arg) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            let name: &'static str = Box::leak(arg.to_string().into_boxed_str());
            match what {
                "press" => self.press(name),
                "hold" => self.hold(name, true),
                "release" => self.hold(name, false),
                "pad" => self.pad(arg.parse().unwrap()),
                "padon" => self.send(&[Event::PadOn(arg.parse().unwrap(), 0.8)]),
                "padoff" => self.send(&[Event::PadOff(arg.parse().unwrap())]),
                "wait" => self.wait(arg.parse().unwrap()),
                "knob" => {
                    let amount: f64 = parts.next().unwrap_or("0.1").parse().unwrap();
                    self.send(&[Event::Knob(arg.parse::<usize>().unwrap() - 1, amount)]);
                }
                "wheel" => self.send(&[Event::Wheel(arg.parse().unwrap())]),
                // (The strip's first finger: down, move or up, at 0..1.)
                "touch" => {
                    let kind = ["down", "move", "up"].iter().position(|k| *k == arg).expect("touch:down|move|up:pos") as u8;
                    let pos: f64 = parts.next().unwrap_or("0.5").parse().unwrap();
                    self.send(&[Event::Touch(0, kind, pos)]);
                }
                _ => panic!("unknown step {step}"),
            }
        }
    }

    pub fn pad(&mut self, n: usize) {
        self.send(&[Event::PadOn(n, 0.8), Event::PadOff(n)]);
    }

    /// What each layer gives now.
    pub fn seen(&mut self) -> (Seen, Seen) {
        (seen(self.ni.state_mut()), seen(self.dk.state_mut()))
    }

    pub fn diff(&mut self) -> Diff {
        let (ni, dk) = self.seen();
        diff(&ni, &dk)
    }

    /// Write Maschine's screens, DECKARD's and their difference, one above the other,
    /// as `<dir>/<name>.png`.
    pub fn picture(&mut self, name: &str) -> std::path::PathBuf {
        let (ni, dk) = self.seen();
        let dir = std::env::temp_dir().join("deckard-parity");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{name}.png"));
        let blank = vec![[0, 0, 0, 255]; 960 * 272];
        let a = ni.screens.unwrap_or_else(|| blank.clone());
        let b = dk.screens.unwrap_or(blank);
        let d: Vec<[u8; 4]> = a
            .iter()
            .zip(&b)
            .map(|(x, y)| {
                let v = (0..3).map(|k| x[k].abs_diff(y[k])).max().unwrap_or(0);
                [v, v, v, 255]
            })
            .collect();
        let mut all = a;
        all.extend(b);
        all.extend(d);
        let f = std::fs::File::create(&path).unwrap();
        let mut e = png::Encoder::new(f, 960, 272 * 3);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&all.concat()).unwrap();
        path
    }
}

/// What one app's MK3 layer gives now: its state, routing, lights and screens.
fn seen(app: &mut App) -> Seen {
    let out = app.mk3_events(Vec::new(), &mut Vec::new());
    let screens = match &mut app.controller {
        Some(Ok(c)) => c.screens().map(|s| s.px.clone()),
        _ => {
            let v = app.surface_view();
            Some(app.surface.picture(&v))
        }
    };
    let r = out.route;
    let route = format!("play {} group {} keys {:?} velocity {:?}", r.play, r.group, r.keys, r.velocity);
    let p = app.doc.as_ref().map(|d| d.borrow().project().clone()).unwrap_or_default();
    let mutes: String =
        p.groups.get(app.group).map_or(String::new(), |g| g.sounds.iter().map(|s| if s.muted { 'M' } else { '.' }).collect());
    let group_mutes: String = p.groups.iter().map(|g| if g.muted { 'M' } else { '.' }).collect();
    let state = vec![
        ("group", app.group.to_string()),
        ("sound", app.sound.to_string()),
        ("pattern", app.pattern.to_string()),
        ("scene", app.scene.to_string()),
        ("playing", app.playing.is_some().to_string()),
        ("recording", app.recording.to_string()),
        ("looping", app.looping.to_string()),
        ("metronome", app.metronome.to_string()),
        ("auto_write", app.auto_write.to_string()),
        ("keyboard", app.piano_roll.to_string()),
        ("song_view", app.song_view.to_string()),
        ("tempo", format!("{:.2}", p.tempo)),
        ("swing", format!("{:.1}", p.swing.amount)),
        ("master", format!("{:.3}", p.master_level)),
        ("sound mutes", mutes),
        ("group mutes", group_mutes),
    ];
    Seen { state, route, leds: out.leds, screens }
}

fn diff(ni: &Seen, dk: &Seen) -> Diff {
    use deckard_mk3::leds::{BUTTON_LEDS, PAD_LEDS};
    let state = ni
        .state
        .iter()
        .zip(&dk.state)
        .filter(|(a, b)| a.1 != b.1)
        .map(|(a, b)| format!("{}: Maschine {} / DECKARD {}", a.0, a.1, b.1))
        .collect();
    let route = (ni.route != dk.route).then(|| format!("Maschine {} / DECKARD {}", ni.route, dk.route));
    let mut leds = Vec::new();
    for (i, name) in BUTTON_LEDS.iter().enumerate() {
        let (a, b) = (ni.leds.buttons[1 + i], dk.leds.buttons[1 + i]);
        if a != b {
            leds.push(format!("{name}: Maschine {a:#04x} / DECKARD {b:#04x}"));
        }
    }
    for (i, name) in PAD_LEDS.iter().enumerate() {
        let (a, b) = (ni.leds.pads[1 + i], dk.leds.pads[1 + i]);
        if a != b {
            leds.push(format!("{name}: Maschine {a:#04x} / DECKARD {b:#04x}"));
        }
    }
    let screens = match (&ni.screens, &dk.screens) {
        (Some(a), Some(b)) if a.len() == b.len() => {
            let off = a.iter().zip(b).filter(|(x, y)| (0..3).any(|k| x[k].abs_diff(y[k]) > 48)).count();
            Some(off as f64 / a.len() as f64)
        }
        _ => None,
    };
    Diff { state, route, leds, screens }
}

/// The scenarios the report walks through: (name, what to do from rest).
pub fn scenarios() -> Vec<(&'static str, fn(&mut Rig))> {
    vec![
        ("rest", |_| {}),
        ("pad-2", |r| r.pad(2)),
        ("pad-3", |r| r.pad(3)),
        ("pad-4", |r| r.pad(4)),
        ("pad-16", |r| r.pad(16)),
        ("group-b", |r| r.press("BUTTON_GROUP_B")),
        ("play", |r| r.press("BUTTON_TRANSPORT_PLAY")),
        ("play-1s", |r| {
            r.press("BUTTON_TRANSPORT_PLAY");
            r.wait(1000);
        }),
        ("play-3s", |r| {
            r.press("BUTTON_TRANSPORT_PLAY");
            r.wait(3000);
        }),
        ("keyboard", |r| r.press("BUTTON_KEYBOARD")),
        ("keyboard-pad-5", |r| {
            r.press("BUTTON_KEYBOARD");
            r.pad(5);
        }),
        ("pad-mode", |r| r.press("BUTTON_PAD_MODE")),
        ("chords", |r| r.press("BUTTON_CHORDS")),
        ("step", |r| r.press("BUTTON_STEP")),
        ("mute-held", |r| r.hold("BUTTON_MUTE", true)),
        ("select-held", |r| r.hold("BUTTON_SELECT", true)),
        ("shift-held", |r| r.hold("BUTTON_SHIFT", true)),
        ("fixed-vel", |r| r.press("BUTTON_FIX_VELOCITY")),
        ("mixer", |r| r.press("BUTTON_MIX")),
        ("arrange", |r| r.press("BUTTON_ARRANGE")),
        ("browse", |r| r.press("BUTTON_BROWSE")),
        ("pattern", |r| r.hold("BUTTON_PATTERN", true)),
        ("scene", |r| r.hold("BUTTON_SCENE", true)),
        ("channel", |r| r.press("BUTTON_CHANNEL")),
        ("plugin", |r| r.press("BUTTON_PLUGIN")),
        ("tempo", |r| r.press("BUTTON_TEMPO")),
        ("swing", |r| r.press("BUTTON_SWING")),
        ("volume", |r| r.press("BUTTON_VOLUME")),
    ]
}

/// More scenarios, as steps (see [`Rig::steps`]).
pub const STEPPED: [(&str, &str); 157] = [
    ("browse-kind", "press:BUTTON_BROWSE wait:1500 press:BUTTON_SCREEN_1 wait:500"),
    ("browse-user", "press:BUTTON_BROWSE wait:1500 press:BUTTON_SCREEN_4"),
    ("browse-favourites", "press:BUTTON_BROWSE wait:1500 press:BUTTON_SCREEN_7"),
    ("browse-shift", "press:BUTTON_BROWSE hold:BUTTON_SHIFT"),
    ("browse-focus-left", "press:BUTTON_BROWSE press:BUTTON_WHEEL_LEFT"),
    ("browse-back", "press:BUTTON_BROWSE press:BUTTON_BROWSE"),
    ("channel-next", "press:BUTTON_CHANNEL press:BUTTON_SCREEN_6"),
    ("channel-wheel", "press:BUTTON_CHANNEL press:BUTTON_WHEEL_RIGHT press:BUTTON_WHEEL_RIGHT"),
    ("plugin-plus", "press:BUTTON_PLUGIN press:BUTTON_WHEEL_RIGHT press:BUTTON_WHEEL_RIGHT press:BUTTON_WHEEL_RIGHT"),
    ("module", "press:BUTTON_PLUGIN press:BUTTON_WHEEL wait:300"),
    ("module-cancel", "press:BUTTON_PLUGIN press:BUTTON_WHEEL wait:300 press:BUTTON_SCREEN_7"),
    ("keyboard-semitone", "press:BUTTON_KEYBOARD press:BUTTON_SCREEN_8"),
    ("module-next", "press:BUTTON_PLUGIN press:BUTTON_WHEEL_RIGHT press:BUTTON_WHEEL wait:300 press:BUTTON_SCREEN_6 wait:300"),
    ("mixer-sound-2", "press:BUTTON_MIX press:BUTTON_SCREEN_2"),
    ("mixer-groups", "press:BUTTON_MIX press:BUTTON_WHEEL_UP"),
    ("mixer-group-knob", "press:BUTTON_MIX press:BUTTON_WHEEL_UP knob:3:0.05"),
    ("mixer-bank-2", "press:BUTTON_MIX press:BUTTON_RIGHT knob:2:0.1"),
    ("mixer-fine", "press:BUTTON_MIX hold:BUTTON_SHIFT knob:1:0.05"),
    ("mixer-back", "press:BUTTON_MIX press:BUTTON_MIX"),
    ("arrange-scene-2", "press:BUTTON_ARRANGE press:BUTTON_SCREEN_2"),
    ("arrange-wheel", "press:BUTTON_ARRANGE wheel:1"),
    ("arrange-down", "press:BUTTON_ARRANGE press:BUTTON_WHEEL_DOWN"),
    ("arrange-group", "press:BUTTON_ARRANGE press:BUTTON_WHEEL_RIGHT"),
    ("arrange-back", "press:BUTTON_ARRANGE press:BUTTON_ARRANGE"),
    ("tempo-wheel", "press:BUTTON_TEMPO wheel:3 wait:300"),
    ("swing-wheel", "press:BUTTON_SWING wheel:3 wait:300"),
    ("volume-wheel", "press:BUTTON_VOLUME wheel:-3 wait:300"),
    ("tempo-fine", "press:BUTTON_TEMPO hold:BUTTON_SHIFT wheel:3 wait:300"),
    ("swing-fine", "press:BUTTON_SWING hold:BUTTON_SHIFT wheel:3 wait:300"),
    ("volume-1", "press:BUTTON_VOLUME wheel:-1 wait:300"),
    ("volume-fine", "press:BUTTON_VOLUME hold:BUTTON_SHIFT wheel:-10 wait:300"),
    ("arrange-shift", "press:BUTTON_ARRANGE hold:BUTTON_SHIFT wait:300"),
    ("song", "press:BUTTON_ARRANGE hold:BUTTON_SHIFT press:BUTTON_SCREEN_2 release:BUTTON_SHIFT wait:300"),
    ("song-right", "press:BUTTON_ARRANGE hold:BUTTON_SHIFT press:BUTTON_SCREEN_2 release:BUTTON_SHIFT press:BUTTON_RIGHT wait:300"),
    (
        "song-play",
        "press:BUTTON_ARRANGE hold:BUTTON_SHIFT press:BUTTON_SCREEN_2 release:BUTTON_SHIFT press:BUTTON_TRANSPORT_PLAY wait:4000",
    ),
    ("song-shift", "press:BUTTON_ARRANGE hold:BUTTON_SHIFT press:BUTTON_SCREEN_2 wait:300"),
    ("pattern-page-2", "hold:BUTTON_PATTERN press:BUTTON_RIGHT"),
    ("pattern-pad-2", "hold:BUTTON_PATTERN pad:2"),
    ("pattern-length", "hold:BUTTON_PATTERN knob:4:0.05"),
    ("pattern-released", "hold:BUTTON_PATTERN pad:2 release:BUTTON_PATTERN"),
    ("scene-pin", "hold:BUTTON_SCENE press:BUTTON_SCREEN_1 release:BUTTON_SCENE wait:300"),
    ("scene-pin-held", "hold:BUTTON_SCENE press:BUTTON_SCREEN_1 wait:300"),
    ("scene-unpin", "hold:BUTTON_SCENE press:BUTTON_SCREEN_1 release:BUTTON_SCENE press:BUTTON_SCREEN_1 wait:300"),
    ("scene-pin-again", "hold:BUTTON_SCENE press:BUTTON_SCREEN_1 release:BUTTON_SCENE press:BUTTON_SCENE wait:300"),
    ("scene-pin-pad", "hold:BUTTON_SCENE press:BUTTON_SCREEN_1 release:BUTTON_SCENE pad:3 wait:300"),
    ("pattern-pin", "hold:BUTTON_PATTERN press:BUTTON_SCREEN_1 release:BUTTON_PATTERN wait:300"),
    ("group-bank", "press:BUTTON_GROUP_C hold:BUTTON_SHIFT press:BUTTON_GROUP_B release:BUTTON_SHIFT wait:300"),
    ("group-new", "hold:BUTTON_SHIFT press:BUTTON_GROUP_B release:BUTTON_SHIFT press:BUTTON_GROUP_H wait:500"),
    (
        "group-new-bank-shift",
        "hold:BUTTON_SHIFT press:BUTTON_GROUP_B release:BUTTON_SHIFT press:BUTTON_GROUP_H wait:300 hold:BUTTON_SHIFT wait:300",
    ),
    (
        "group-new-bank",
        "hold:BUTTON_SHIFT press:BUTTON_GROUP_B release:BUTTON_SHIFT press:BUTTON_GROUP_H wait:300 hold:BUTTON_SHIFT press:BUTTON_GROUP_C release:BUTTON_SHIFT wait:500",
    ),
    // (No plain press of AUTO: DECKARD's stays on, a toggle, where Maschine's goes off
    // once let go, on purpose: CONTROLLER-BUGS C38.)
    ("auto-held", "hold:BUTTON_AUTO wait:300"),
    ("auto-locked", "hold:BUTTON_SHIFT press:BUTTON_AUTO release:BUTTON_SHIFT wait:300"),
    ("auto-unlocked", "hold:BUTTON_SHIFT press:BUTTON_AUTO release:BUTTON_SHIFT press:BUTTON_AUTO wait:300"),
    ("file", "press:BUTTON_FILE wait:300"),
    ("file-back", "press:BUTTON_FILE press:BUTTON_FILE wait:300"),
    ("file-save", "press:BUTTON_FILE press:BUTTON_SCREEN_3 wait:300"),
    ("file-save-group", "press:BUTTON_FILE press:BUTTON_SCREEN_3 knob:1:0.1 wait:300"),
    ("file-save-sound", "press:BUTTON_FILE press:BUTTON_SCREEN_3 pad:2 wait:300"),
    ("variation", "press:BUTTON_VARIATION wait:300"),
    ("variation-held", "hold:BUTTON_VARIATION wait:300"),
    ("variation-file", "press:BUTTON_FILE press:BUTTON_VARIATION wait:300"),
    ("variation-shift", "hold:BUTTON_SHIFT press:BUTTON_VARIATION release:BUTTON_SHIFT wait:300"),
    ("plugin-shift", "hold:BUTTON_SHIFT wait:300"),
    ("plugin-shift-fx", "press:BUTTON_SCREEN_6 hold:BUTTON_SHIFT wait:300"),
    ("step-pad", "press:BUTTON_STEP pad:2 pad:5 wait:300"),
    ("step-page2", "press:BUTTON_STEP knob:6:0.2 wait:300"),
    ("step-keyboard", "press:BUTTON_STEP press:BUTTON_SCREEN_4 wait:300"),
    ("step-follow-prob", "press:BUTTON_STEP press:BUTTON_SCREEN_5 press:BUTTON_SCREEN_7 press:BUTTON_RIGHT wait:300"),
    ("step-remove", "press:BUTTON_STEP pad:1 wait:300"),
    (
        "step-mod",
        "press:BUTTON_STEP padon:3 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:300",
    ),
    (
        "step-mod-knob",
        "press:BUTTON_STEP padon:3 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 knob:1:0.1 wait:300",
    ),
    (
        "step-mod-slot",
        "press:BUTTON_STEP padon:3 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 press:BUTTON_SCREEN_6 wait:300",
    ),
    (
        "step-mod-up",
        "press:BUTTON_STEP padon:3 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 wait:60 knob:1:0.1 padoff:3 wait:300",
    ),
    ("step-prob-lock", "press:BUTTON_STEP press:BUTTON_SCREEN_8 wait:300"),
    ("events-prob-lock", "hold:BUTTON_EVENTS press:BUTTON_SCREEN_8 wait:300"),
    ("events-zoom", "hold:BUTTON_EVENTS press:BUTTON_SCREEN_1 release:BUTTON_EVENTS knob:5:0.1 wait:300"),
    ("events-scroll", "hold:BUTTON_EVENTS press:BUTTON_SCREEN_1 release:BUTTON_EVENTS knob:5:0.1 knob:6:0.05 wait:300"),
    ("sampling-edit", "press:BUTTON_SAMPLE wait:300"),
    ("sampling-edit2", "press:BUTTON_SAMPLE wait:300 press:BUTTON_RIGHT wait:300"),
    ("sampling-record", "press:BUTTON_SAMPLE wait:300 press:BUTTON_SCREEN_1 wait:300"),
    ("sampling-slice", "press:BUTTON_SAMPLE wait:300 press:BUTTON_SCREEN_3 wait:300"),
    ("sampling-zone", "press:BUTTON_SAMPLE wait:300 press:BUTTON_SCREEN_4 wait:300"),
    ("shift-pad-undo", "press:BUTTON_STEP pad:2 pad:5 hold:BUTTON_SHIFT pad:1 release:BUTTON_SHIFT wait:300"),
    ("shift-pad-clear", "press:BUTTON_STEP pad:2 pad:5 hold:BUTTON_SHIFT pad:9 release:BUTTON_SHIFT wait:300"),
    ("shift-pad-octave", "press:BUTTON_STEP pad:2 pad:5 hold:BUTTON_SHIFT pad:15 release:BUTTON_SHIFT wait:300"),
    ("note-repeat", "hold:BUTTON_NOTE_REPEAT wait:300"),
    ("note-repeat-pin", "hold:BUTTON_NOTE_REPEAT press:BUTTON_SCREEN_1 release:BUTTON_NOTE_REPEAT wait:300"),
    ("note-repeat-rate", "hold:BUTTON_NOTE_REPEAT knob:3:0.1 knob:8:-0.1 wait:300"),
    ("note-repeat-preset", "hold:BUTTON_NOTE_REPEAT press:BUTTON_SCREEN_8 press:BUTTON_SCREEN_3 wait:300"),
    ("note-repeat-lock", "hold:BUTTON_NOTE_REPEAT press:BUTTON_SCREEN_2 release:BUTTON_NOTE_REPEAT wait:300"),
    ("note-repeat-arp", "press:BUTTON_KEYBOARD hold:BUTTON_NOTE_REPEAT wait:300"),
    ("note-repeat-arp2", "press:BUTTON_KEYBOARD hold:BUTTON_NOTE_REPEAT press:BUTTON_RIGHT knob:6:-0.1 wait:300"),
    ("note-repeat-shift", "hold:BUTTON_SHIFT hold:BUTTON_NOTE_REPEAT release:BUTTON_SHIFT wait:300"),
    ("note-repeat-step", "press:BUTTON_STEP hold:BUTTON_NOTE_REPEAT release:BUTTON_NOTE_REPEAT wait:300"),
    ("strip-pitch", "press:BUTTON_PITCH wait:300"),
    ("strip-pitch-touch", "press:BUTTON_PITCH touch:down:0.9 wait:300"),
    ("strip-pitch-up", "press:BUTTON_PITCH touch:down:0.9 touch:up:0.9 wait:300"),
    ("strip-mod", "press:BUTTON_MOD wait:300"),
    ("strip-mod-up", "press:BUTTON_MOD touch:down:0.4 touch:up:0.4 wait:300"),
    ("strip-perform", "press:BUTTON_PERFORM wait:300"),
    ("strip-perform-fx", "hold:BUTTON_SHIFT press:BUTTON_PERFORM release:BUTTON_SHIFT wait:300"),
    ("strip-perform-touch", "hold:BUTTON_SHIFT press:BUTTON_PERFORM release:BUTTON_SHIFT touch:down:0.3 wait:300"),
    ("strip-notes", "press:BUTTON_NOTES touch:down:0.6 wait:300"),
    ("strip-off", "press:BUTTON_PITCH press:BUTTON_PITCH wait:300"),
    ("events", "hold:BUTTON_EVENTS wait:300"),
    ("events-pin", "hold:BUTTON_EVENTS press:BUTTON_SCREEN_1 release:BUTTON_EVENTS wait:300"),
    ("events-page2", "hold:BUTTON_EVENTS press:BUTTON_RIGHT wait:300"),
    ("events-select", "hold:BUTTON_EVENTS press:BUTTON_SCREEN_2 pad:1 wait:300"),
    ("events-edit", "hold:BUTTON_EVENTS press:BUTTON_SCREEN_2 pad:1 press:BUTTON_SCREEN_2 knob:3:0.1 wait:300"),
    ("lock", "press:BUTTON_LOCK wait:300"),
    ("lock-recall", "press:BUTTON_LOCK press:BUTTON_LOCK wait:300"),
    ("lock-ext", "hold:BUTTON_SHIFT press:BUTTON_LOCK release:BUTTON_SHIFT wait:300"),
    ("lock-ext-pads", "hold:BUTTON_SHIFT press:BUTTON_LOCK release:BUTTON_SHIFT pad:1 pad:2 wait:300"),
    ("lock-ext-morph", "hold:BUTTON_SHIFT press:BUTTON_LOCK release:BUTTON_SHIFT knob:1:0.2 knob:2:0.2 knob:3:0.2 wait:300"),
    ("lock-ext-bank", "hold:BUTTON_SHIFT press:BUTTON_LOCK release:BUTTON_SHIFT press:BUTTON_SCREEN_8 pad:3 wait:300"),
    (
        "lock-ext-erase",
        "hold:BUTTON_SHIFT press:BUTTON_LOCK release:BUTTON_SHIFT pad:1 pad:2 hold:BUTTON_TRANSPORT_ERASE pad:2 release:BUTTON_TRANSPORT_ERASE wait:300",
    ),
    ("lock-ext-back", "hold:BUTTON_SHIFT press:BUTTON_LOCK release:BUTTON_SHIFT press:BUTTON_LOCK wait:300"),
    ("settings", "press:BUTTON_SETTINGS wait:300"),
    ("settings-audio", "press:BUTTON_SETTINGS press:BUTTON_SCREEN_6 wait:300"),
    ("settings-midi", "press:BUTTON_SETTINGS press:BUTTON_WHEEL_RIGHT press:BUTTON_WHEEL_RIGHT wait:300"),
    ("settings-hardware", "press:BUTTON_SETTINGS press:BUTTON_SCREEN_6 press:BUTTON_SCREEN_6 press:BUTTON_SCREEN_6 wait:300"),
    ("settings-general-2", "press:BUTTON_SETTINGS press:BUTTON_RIGHT wait:300"),
    ("settings-back", "press:BUTTON_SETTINGS press:BUTTON_SETTINGS wait:300"),
    ("macro", "press:BUTTON_MACRO wait:300"),
    ("macro-set", "hold:BUTTON_SHIFT press:BUTTON_MACRO release:BUTTON_SHIFT wait:300"),
    ("macro-set-master", "hold:BUTTON_SHIFT press:BUTTON_MACRO release:BUTTON_SHIFT press:BUTTON_SCREEN_2 wait:300"),
    ("macro-set-back", "hold:BUTTON_SHIFT press:BUTTON_MACRO press:BUTTON_MACRO release:BUTTON_SHIFT wait:300"),
    ("macro-group", "press:BUTTON_MACRO press:BUTTON_SCREEN_2 wait:300"),
    ("grid", "hold:BUTTON_SHIFT hold:BUTTON_FOLLOW wait:300"),
    ("grid-perform-pick", "hold:BUTTON_SHIFT hold:BUTTON_FOLLOW pad:5 wait:300"),
    ("grid-perform-cycle", "hold:BUTTON_SHIFT hold:BUTTON_FOLLOW press:BUTTON_SCREEN_6 wait:300"),
    ("grid-arrange", "hold:BUTTON_SHIFT hold:BUTTON_FOLLOW press:BUTTON_SCREEN_3 wait:300"),
    ("grid-arrange-quick", "hold:BUTTON_SHIFT hold:BUTTON_FOLLOW press:BUTTON_SCREEN_3 pad:9 knob:4:-0.2 wait:300"),
    ("grid-step", "hold:BUTTON_SHIFT hold:BUTTON_FOLLOW press:BUTTON_SCREEN_4 release:BUTTON_SHIFT wait:300"),
    ("grid-step-pick", "hold:BUTTON_SHIFT hold:BUTTON_FOLLOW press:BUTTON_SCREEN_4 pad:3 wait:300"),
    ("grid-step-off", "hold:BUTTON_SHIFT hold:BUTTON_FOLLOW press:BUTTON_SCREEN_4 pad:16 wait:300"),
    (
        "grid-pin",
        "hold:BUTTON_SHIFT hold:BUTTON_FOLLOW press:BUTTON_SCREEN_4 press:BUTTON_SCREEN_1 release:BUTTON_SHIFT release:BUTTON_FOLLOW wait:300",
    ),
    ("grid-closed", "hold:BUTTON_SHIFT hold:BUTTON_FOLLOW press:BUTTON_SCREEN_4 release:BUTTON_SHIFT release:BUTTON_FOLLOW wait:300"),
    ("keyboard-pattern", "press:BUTTON_KEYBOARD hold:BUTTON_PATTERN wait:300"),
    ("keyboard-pattern-pin", "press:BUTTON_KEYBOARD hold:BUTTON_PATTERN press:BUTTON_SCREEN_1 release:BUTTON_PATTERN wait:300"),
    ("mute-pin", "hold:BUTTON_MUTE press:BUTTON_SCREEN_1 release:BUTTON_MUTE wait:300"),
    ("scene-pin-mix", "hold:BUTTON_SCENE press:BUTTON_SCREEN_1 release:BUTTON_SCENE press:BUTTON_MIX wait:300"),
    ("pad-mode-2", "press:BUTTON_PAD_MODE press:BUTTON_RIGHT wait:300"),
    ("keyboard-2", "press:BUTTON_KEYBOARD press:BUTTON_RIGHT wait:300"),
    ("chords-2", "press:BUTTON_CHORDS press:BUTTON_RIGHT wait:300"),
    ("keyboard-3", "press:BUTTON_KEYBOARD press:BUTTON_RIGHT press:BUTTON_RIGHT wait:300"),
    ("keyboard-4", "press:BUTTON_KEYBOARD press:BUTTON_RIGHT press:BUTTON_RIGHT press:BUTTON_RIGHT wait:300"),
    ("keyboard-scale", "press:BUTTON_KEYBOARD knob:2:0.1 knob:2:0.1 wait:300"),
    ("chords-type", "press:BUTTON_CHORDS knob:4:0.1 wait:300"),
    ("keyboard-user", "press:BUTTON_KEYBOARD press:BUTTON_SCREEN_4 wait:300"),
    ("chords-user", "press:BUTTON_CHORDS knob:3:0.1 knob:3:0.1 wait:300"),
    ("chords-user-shift", "press:BUTTON_CHORDS knob:3:0.1 knob:3:0.1 hold:BUTTON_SHIFT wait:300"),
    ("scene-pad-3", "hold:BUTTON_SCENE pad:3"),
    ("scene-colour", "hold:BUTTON_SCENE knob:1:0.05"),
    ("scene-colour-twice", "hold:BUTTON_SCENE knob:1:0.05 knob:1:0.05"),
    ("scene-colour-big", "hold:BUTTON_SCENE knob:1:0.3"),
    ("pad-mode-choke-big", "press:BUTTON_PAD_MODE knob:1:0.3"),
];

/// Every scenario: what differs, and pictures of both layers' screens.
#[test]
#[ignore]
fn parity_report() {
    let only = std::env::var("PARITY").ok();
    let all = scenarios().into_iter().map(|(name, run)| (name, Ok(run))).chain(STEPPED.iter().map(|&(name, steps)| (name, Err(steps))));
    for (name, run) in all {
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        let Some(mut rig) = Rig::new() else { return };
        match run {
            Ok(run) => run(&mut rig),
            Err(steps) => rig.steps(steps),
        }
        let d = rig.diff();
        let path = rig.picture(name);
        let screens = d.screens.map_or("-".to_string(), |s| format!("{:.1} %", s * 100.0));
        println!(
            "\n== {name}: {} state, {} lights, route {}, screens {screens} differ  ({})",
            d.state.len(),
            d.leds.len(),
            if d.route.is_some() { "differs" } else { "same" },
            path.display()
        );
        for s in d.state.iter().chain(&d.route).chain(d.leds.iter().take(12)) {
            println!("   {s}");
        }
        if d.leds.len() > 12 {
            println!("   … {} more lights", d.leds.len() - 12);
        }
    }
}

/// A light byte as read: a one-colour button's level, or a palette colour and shade.
fn light(name: &str, b: u8) -> String {
    if deckard_mk3::leds::is_colour(name) {
        if b == 0 { "off".into() } else { format!("colour {} shade {}", (b >> 2) as i64 - 1, b & 3) }
    } else {
        match b {
            0x09 => "backlight".into(),
            0x22 => "dim".into(),
            0x7f => "on".into(),
            b => format!("{b:#04x}"),
        }
    }
}

/// What Maschine's scripts do in each scenario (`PARITY=<name>` for one): every light
/// that isn't the backlight or off, the routing, and the screens' picture.
#[test]
#[ignore]
fn parity_observe() {
    use deckard_mk3::leds::{BUTTON_LEDS, PAD_LEDS};
    let only = std::env::var("PARITY").ok();
    for (name, run) in scenarios() {
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        let Some(mut rig) = Rig::new() else { return };
        run(&mut rig);
        let (ni, _) = rig.seen();
        let at = rig.ni.state().playhead().map(|h| h.position);
        println!("\n== {name}: {} at {at:?}", ni.route);
        let mut lit = Vec::new();
        for (i, n) in BUTTON_LEDS.iter().enumerate() {
            let b = ni.leds.buttons[1 + i];
            if b != 0x09 && b != 0 {
                lit.push(format!("{n}={}", light(n, b)));
            }
        }
        for (i, n) in PAD_LEDS.iter().enumerate() {
            let b = ni.leds.pads[1 + i];
            if b != 0 {
                lit.push(format!("{n}={}", light(n, b)));
            }
        }
        println!("   {}", lit.join("  "));
        println!("   {}", rig.picture(name).display());
    }
}

/// Maschine's screens in each scenario as laid-out boxes (`PARITY=<name>`), written to
/// `$TMPDIR/deckard-parity/<name>.layout.txt`: what DECKARD's lookalikes measure from.
#[test]
#[ignore]
fn parity_layout() {
    let only = std::env::var("PARITY").ok();
    let dir = std::env::temp_dir().join("deckard-parity");
    let _ = std::fs::create_dir_all(&dir);
    for (name, run) in scenarios() {
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        let Some(mut rig) = Rig::new() else { return };
        run(&mut rig);
        let _ = rig.seen();
        let Some(Ok(c)) = &rig.ni.state().controller else { continue };
        let pals: Vec<String> =
            c.palettes().iter().map(|(p, i, c)| format!("palette {p} {i} #{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])).collect();
        std::fs::write(dir.join("palettes.txt"), pals.join("\n")).unwrap();
        let path = dir.join(format!("{name}.layout.txt"));
        std::fs::write(&path, c.screen_layout().join("\n")).unwrap();
        println!("{}", path.display());
    }
}

/// Try anything on Maschine's scripts: `PROBE="press:BUTTON_MIX hold:BUTTON_SHIFT
/// release:BUTTON_SHIFT pad:3 knob:1:0.1 wait:500"` runs the steps from rest, then shows
/// the state, routing and lit lights, and writes `probe.png` and `probe.layout.txt`.
#[test]
#[ignore]
fn parity_probe() {
    use deckard_mk3::leds::{BUTTON_LEDS, PAD_LEDS};
    let Ok(steps) = std::env::var("PROBE") else { return };
    let Some(mut rig) = Rig::new() else { return };
    rig.steps(&steps);
    let (ni, _) = rig.seen();
    println!("route: {}", ni.route);
    for (k, v) in &ni.state {
        println!("{k}: {v}");
    }
    let mut lit = Vec::new();
    for (i, n) in BUTTON_LEDS.iter().enumerate() {
        let b = ni.leds.buttons[1 + i];
        if b != 0x09 && b != 0 {
            lit.push(format!("{n}={}", light(n, b)));
        }
    }
    for (i, n) in PAD_LEDS.iter().enumerate() {
        let b = ni.leds.pads[1 + i];
        if b != 0 {
            lit.push(format!("{n}={}", light(n, b)));
        }
    }
    println!("lit: {}", lit.join("  "));
    println!("{}", rig.picture("probe").display());
    if let Some(Ok(c)) = &rig.ni.state().controller {
        // PROBE_LUA: a Lua expression to print from Maschine's side, and its log.
        if std::env::var("PROBE_LUA").is_ok() {
            let r: Result<String, _> = c.host.lua.load(&std::env::var("PROBE_LUA").unwrap()).eval();
            println!("lua: {r:?}");
            println!("log: {:?}", c.host.state.borrow().log.iter().rev().take(8).collect::<Vec<_>>());
        }
        let dir = std::env::temp_dir().join("deckard-parity");
        std::fs::write(dir.join("probe.layout.txt"), c.screen_layout().join("\n")).unwrap();
    }
}
