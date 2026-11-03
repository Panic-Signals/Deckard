//! Scene Tempo (a scene's right-click menu, the header's red line, the tab's metronome) and
//! Edit ▸ MIDI Change's Scene and Section blocks.

use crate::layout::Act;
use crate::midi_io::{LockMidi, Recall, recall_trigger};
use deckard_engine::midi::InMsg;
use eframe::egui::{Event, PointerButton, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::{Duration, Instant};

type H = Harness<'static, crate::App>;

fn drawn(h: &H, name: &str) -> Option<Rect> {
    h.ctx.data(|d| d.get_temp(crate::layout::header_rect(name)))
}

fn forget(h: &H, name: &str) {
    h.ctx.data_mut(|d| d.remove::<Rect>(crate::layout::header_rect(name)));
}

fn scene_tempo(h: &H, s: usize) -> Option<f64> {
    h.state().doc.as_ref().unwrap().borrow().project().scenes[s].tempo
}

/// Right-click the first scene's tab, Scene Tempo on: the song's tempo, and its metronome;
/// off again, neither.
#[test]
fn a_scenes_menu_gives_it_its_own_tempo() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    h.run_steps(3);
    let tab = drawn(&h, "scene-tab-0").expect("the tab");
    assert!(drawn(&h, "scene-metronome-0").is_none());
    let menu = |h: &mut H| {
        let pos = tab.center();
        h.hover_at(pos);
        h.step();
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.run_steps(2);
    };
    menu(&mut h);
    h.get_by_label("Scene Tempo").click();
    h.run_steps(3);
    let song = h.state().doc.as_ref().unwrap().borrow().project().tempo;
    assert_eq!(scene_tempo(&h, 0), Some(song), "{}", h.state().status);
    assert!(drawn(&h, "scene-metronome-0").is_some(), "the metronome");
    // The menu stays open for its BPM, which sets the scene's tempo.
    h.get_by_label("BPM");
    h.get_by_label("Scene Tempo").click();
    h.run_steps(3);
    forget(&h, "scene-metronome-0");
    h.run_steps(2);
    assert_eq!(scene_tempo(&h, 0), None);
    assert!(drawn(&h, "scene-metronome-0").is_none());
}

/// Playing a scene with its own tempo: the header shows it, the song's under a red line;
/// it's what's heard and what the header changes. Stopped, the song's again.
#[test]
fn the_header_shows_a_playing_scenes_tempo_over_the_songs() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    crate::layout::apply_acts(h.state_mut(), vec![Act::SetSceneTempo(0, Some(97.0))]);
    h.run_steps(2);
    assert!(drawn(&h, "bpm-song-struck").is_none(), "not until it plays");
    h.state_mut().transport_play();
    if !matches!(h.state().output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    let wait = Instant::now();
    while h.state().output.as_ref().unwrap().as_ref().unwrap().telemetry(|t| t.tempo) != 97.0 && wait.elapsed() < Duration::from_secs(5) {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(h.state().output.as_ref().unwrap().as_ref().unwrap().telemetry(|t| t.tempo), 97.0, "heard");
    h.run_steps(2);
    assert!(drawn(&h, "bpm-song-struck").is_some(), "the red line");
    let song = h.state().doc.as_ref().unwrap().borrow().project().tempo;
    // Changing the tempo while it plays: the scene's.
    crate::layout::apply_acts(h.state_mut(), vec![Act::SetTempo(101.0)]);
    assert_eq!(scene_tempo(&h, 0), Some(101.0));
    assert_eq!(h.state().doc.as_ref().unwrap().borrow().project().tempo, song, "the song's stays");
    h.state_mut().stop();
    forget(&h, "bpm-song-struck");
    h.run_steps(3);
    assert!(drawn(&h, "bpm-song-struck").is_none(), "stopped");
    crate::layout::apply_acts(h.state_mut(), vec![Act::SetTempo(133.0)]);
    assert_eq!(scene_tempo(&h, 0), Some(101.0), "stopped: the song's is changed");
    assert_eq!(h.state().doc.as_ref().unwrap().borrow().project().tempo, 133.0);
}

/// Link or incoming MIDI clock sets the tempo: scenes' own don't count (as Maschine's).
#[test]
fn a_clock_turns_scene_tempos_off() {
    let mut app = crate::App::default();
    assert!(app.scene_tempo_available());
    app.settings.link = true;
    assert!(!app.scene_tempo_available());
    app.settings.link = false;
    app.settings.midi_sync = crate::settings::MidiSync::Receive;
    assert!(!app.scene_tempo_available());
    app.settings.midi_sync = crate::settings::MidiSync::Send;
    assert!(app.scene_tempo_available());
}

/// Note or program n picks number n from the first, on the input and channel chosen.
#[test]
fn midi_picks_a_scene_by_its_number() {
    let notes = LockMidi { source: "Keys".into(), channel: 2, trigger: 1 };
    let on = |note| InMsg::NoteOn { ch: 2, note, velocity: 100 };
    assert_eq!(recall_trigger(&notes, "Keys", &on(0), 128), Some(Some(0)));
    assert_eq!(recall_trigger(&notes, "Keys", &on(127), 128), Some(Some(127)));
    assert_eq!(recall_trigger(&notes, "Keys", &InMsg::NoteOff { ch: 2, note: 5, velocity: 0 }, 128), Some(None), "taken, nothing to do");
    assert_eq!(recall_trigger(&notes, "Pads", &on(5), 128), None, "another input plays");
    assert_eq!(recall_trigger(&notes, "Keys", &InMsg::NoteOn { ch: 3, note: 5, velocity: 100 }, 128), None, "another channel plays");
    let programs = LockMidi { source: "Keys".into(), channel: 2, trigger: 2 };
    assert_eq!(recall_trigger(&programs, "Keys", &InMsg::Program { ch: 2, program: 9 }, 128), Some(Some(9)));
    assert_eq!(recall_trigger(&programs, "Keys", &on(5), 128), None, "notes play");
    // Not controllers (Maschine's takes only these two).
    assert_eq!(recall_trigger(&notes, "Keys", &InMsg::Control { ch: 2, number: 5, value: 127 }, 128), None);
    assert_eq!(recall_trigger(&programs, "Keys", &InMsg::Control { ch: 2, number: 5, value: 127 }, 128), None);
}

/// What's picked focuses like a click; past the last, nothing.
#[test]
fn a_recall_focuses_the_scene() {
    let mut app = crate::App::default();
    app.new_project();
    crate::layout::apply_acts(&mut app, vec![Act::InsertScene(1)]);
    app.midi_recall(Recall::Scene(1));
    assert_eq!(app.scene, 1);
    app.midi_recall(Recall::Scene(40));
    assert_eq!(app.scene, 1);
    app.midi_recall(Recall::Section(40));
    assert_eq!(app.scene, 1);
}

/// Edit ▸ MIDI Change's Scene block sets the project's.
#[test]
fn the_midi_change_dialog_sets_the_scene_block() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    h.state_mut().menu_commands.push(crate::menu::Command::MidiChange);
    h.run_steps(3);
    h.get_by_label("Scene trigger").click();
    h.run_steps(2);
    h.get_by_label("Program Change").click();
    h.run_steps(3);
    let doc = h.state().doc.clone().unwrap();
    assert_eq!(doc.borrow().project().scene_recall.trigger, 2, "{}", h.state().status);
    assert_eq!(doc.borrow().project().section_recall.trigger, 0);
    assert_eq!(doc.borrow().project().lock.midi_trigger, 0);
}

/// A controller scrolling its scenes (the Launchpad's left and right) scrolls the tabs.
#[test]
fn a_controller_scrolls_the_scene_tabs() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    let tab = |h: &H, s: usize| h.ctx.data(|d| d.get_temp::<Rect>(eframe::egui::Id::new(("deckard-scene-tab", s))));
    for _ in 0..40 {
        h.state().doc.as_ref().unwrap().borrow_mut().insert_scene(0).unwrap();
    }
    h.run_steps(2);
    let left = tab(&h, 0).expect("the first tab").left();
    h.state_mut().surface_act(deckard_surface::Action::ShowScenes(8));
    h.run_steps(2);
    assert_eq!(h.state().show_scenes, None, "done");
    assert!((tab(&h, 8).expect("the ninth tab").left() - left).abs() < 1.0, "the ninth first");
    // It stays: a scroll, not a hold.
    h.run_steps(2);
    assert!((tab(&h, 8).unwrap().left() - left).abs() < 1.0);
}
