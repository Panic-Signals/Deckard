//! Link and MIDI clock in the app: joining Link, the tempo both ways, starting and
//! stopping with the session or an external clock, sending clock, and the MK3's
//! Settings ▸ MIDI reaching the app.

use crate::settings::MidiSync;
use deckard_engine::sync::{SyncMsg, Transport};
use std::time::{Duration, Instant};

/// A new project, Link on a session of one (never the network).
fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app.sync.local_only = true;
    app
}

fn tempo(app: &crate::App) -> f64 {
    app.doc.as_ref().unwrap().borrow().project().tempo
}

/// LINK joins (the session takes the song's tempo); a tempo the session gets is the
/// song's (no undo step, nothing to save); a tempo set here goes to the session.
#[test]
fn link_joins_and_the_tempo_goes_both_ways() {
    let mut app = app();
    let ctx = eframe::egui::Context::default();
    app.toggle_link();
    assert!(app.settings.link);
    app.sync_clocks(&ctx);
    let link = app.sync.link().expect("joined");
    assert!(link.is_enabled());
    assert!((link.tempo() - tempo(&app)).abs() < 1e-9);
    link.set_tempo(97.5);
    app.sync_clocks(&ctx);
    assert_eq!(tempo(&app), 97.5);
    let doc = app.doc.clone().unwrap();
    assert!(!doc.borrow().is_dirty(), "following isn't an edit");
    doc.borrow_mut().set_tempo(110.0).unwrap();
    app.sync_clocks(&ctx);
    assert!((app.sync.link().unwrap().tempo() - 110.0).abs() < 1e-9);
    // Off: it leaves the session.
    app.toggle_link();
    app.sync_clocks(&ctx);
    assert!(!app.sync.link().unwrap().is_enabled());
    // Receive Clock isn't offered while Link is on.
    app.settings.midi_sync = MidiSync::Receive;
    app.toggle_link();
    assert_eq!(app.settings.midi_sync, MidiSync::Off);
}

/// Start Stop Sync: the session starting starts the song, the song stopping stops the
/// session. (Needs an audio output.)
#[test]
fn link_start_stop_both_ways() {
    let mut app = app();
    let ctx = eframe::egui::Context::default();
    app.settings.link_start_stop = true;
    app.toggle_link();
    app.sync_clocks(&ctx);
    app.sync.link().unwrap().set_playing(true);
    app.sync_clocks(&ctx);
    if !matches!(app.output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    assert!(app.playing.is_some(), "the session started the song");
    app.stop();
    app.sync_clocks(&ctx);
    assert!(!app.sync.link().unwrap().is_playing(), "the song stopped the session");
    app.transport_play();
    app.sync_clocks(&ctx);
    assert!(app.sync.link().unwrap().is_playing(), "and started it");
    app.stop();
}

/// Receive Clock: the clock's Start plays, Stop stops; PLAY does nothing meanwhile.
/// (Needs an audio output.)
#[test]
fn receiving_clock_starts_and_stops_the_song() {
    let mut app = app();
    let ctx = eframe::egui::Context::default();
    app.settings.midi_sync = MidiSync::Receive;
    app.sync_clocks(&ctx);
    app.transport_play();
    assert!(app.playing.is_none(), "PLAY waits for the clock");
    let t0 = Instant::now();
    app.sync.follower.message(SyncMsg::Start, t0);
    app.sync.follower.message(SyncMsg::Clock, t0 + Duration::from_millis(20));
    app.sync_clocks(&ctx);
    if !matches!(app.output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    assert!(app.playing.is_some(), "Start plays");
    app.sync.follower.message(SyncMsg::Stop, t0 + Duration::from_millis(40));
    app.sync_clocks(&ctx);
    assert!(app.playing.is_none(), "Stop stops");
    assert_eq!(app.sync.follower.take_transport(), None::<Transport>);
}

/// Send Clock: to every MIDI output switched on, that early.
#[test]
fn clock_goes_to_the_outputs_switched_on() {
    let mut app = app();
    app.midi_ports = (Vec::new(), vec!["Drums".into(), "Synth".into()]);
    app.midi_ports_at = Some(Instant::now());
    app.settings.midi_off.insert("out:Synth".into());
    assert_eq!(app.clock_out(), None, "Sync is off");
    app.settings.midi_sync = MidiSync::Send;
    app.settings.clock_send_offset_ms = 15;
    let out = app.clock_out().unwrap();
    assert_eq!(out.ports, vec![std::sync::Arc::<str>::from("Drums")]);
    assert_eq!(out.early, Duration::from_millis(15));
}

/// The MK3's Settings ▸ MIDI page changes the app's settings (and shows the app's).
#[test]
fn mk3_settings_reach_the_app() {
    let Some(mut h) = crate::tests::harness(true, false) else { return };
    let Some(Ok(c)) = &h.state().controller else {
        eprintln!("no controller scripts; skipping");
        return;
    };
    assert!(!c.host.state.borrow().sync.link);
    h.state_mut().sync.local_only = true;
    if let Some(Ok(c)) = &mut h.state_mut().controller {
        let mut st = c.host.state.borrow_mut();
        st.sync.link = true;
        st.sync.link_start_stop = true;
        st.sync.send_offset = 30;
    }
    h.run_steps(3);
    let s = &h.state().settings;
    assert!(s.link && s.link_start_stop);
    assert_eq!(s.clock_send_offset_ms, 30);
    // And back: the app's change shows on the controller.
    h.state_mut().settings.midi_sync = MidiSync::Send;
    h.run_steps(3);
    let Some(Ok(c)) = &h.state().controller else { unreachable!() };
    assert_eq!(c.host.state.borrow().sync.midi_sync, 1);
    h.state_mut().toggle_link();
    h.run_steps(2);
}

/// The header's LINK button joins and leaves; it shows the session's phase.
#[test]
fn the_header_link_button() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    h.state_mut().sync.local_only = true;
    let at = h.ctx.data(|d| d.get_temp::<eframe::egui::Rect>(crate::layout::header_rect("link"))).expect("LINK drawn").center();
    let click = |h: &mut egui_kittest::Harness<'static, crate::App>| {
        h.hover_at(at);
        h.step();
        for pressed in [true, false] {
            h.event(eframe::egui::Event::PointerButton {
                pos: at,
                button: eframe::egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
            h.step();
        }
        h.run_steps(2);
    };
    click(&mut h);
    assert!(h.state().settings.link, "joined");
    assert!(h.state().link_view().is_some());
    click(&mut h);
    assert!(!h.state().settings.link, "left");
}
