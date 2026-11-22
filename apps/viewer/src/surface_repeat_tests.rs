//! Note Repeat on DECKARD's own controller layer (NOTE REPEAT): its settings shown and
//! set ([`deckard_surface::Action::Repeat`]), and on while its page shows.

use deckard_mk3::Event;
use deckard_surface::Action as A;
use std::time::Instant;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app
}

fn text(app: &crate::App, key: &str) -> String {
    app.surface_view().note_repeat.settings.iter().find(|s| s.key == key).map(|s| s.text.clone()).unwrap_or_default()
}

#[test]
fn the_surface_shows_and_sets_note_repeat() {
    let mut app = app();
    let r = app.surface_view().note_repeat;
    assert_eq!(r.presets, ["1/8", "1/16", "1/16T", "1/32"], "Maschine's");
    assert_eq!(r.preset, 1, "1/16");
    assert_eq!(text(&app, "rate"), "1/16");
    assert!(!r.hold && !r.locked && !r.active);
    app.surface_act(A::Repeat("rate", 3.0));
    assert_eq!(text(&app, "rate"), "1/8");
    app.surface_act(A::Repeat("gate", 0.5));
    assert_eq!(text(&app, "gate"), "50%");
    app.surface_act(A::Repeat("max_key", 100.0));
    assert_eq!(app.doc.as_ref().unwrap().borrow().project().arp.max_key, 100);
    app.surface_act(A::Repeat("hold", 1.0));
    assert!(app.surface_view().note_repeat.hold);
    app.surface_act(A::Repeat("preset", 4.0));
    assert_eq!(app.surface_view().note_repeat.preset, 3);
    assert_eq!(text(&app, "rate"), "1/32", "the preset's rate");
    // LOCK: on off the page.
    app.surface_act(A::Repeat("locked", 1.0));
    app.sync_arp();
    assert!(app.repeat_sent.is_some());
    let r = app.surface_view().note_repeat;
    assert!(r.locked && r.active);
    app.surface_act(A::Repeat("locked", 0.0));
    app.sync_arp();
    assert!(app.repeat_sent.is_none());
}

#[test]
fn note_repeat_is_on_while_its_page_shows() {
    let mut app = app();
    let v = app.surface_view();
    app.surface.event(&Event::Button("BUTTON_NOTE_REPEAT", true), &v, Instant::now());
    app.sync_arp();
    assert!(app.repeat_sent.is_some(), "on while held");
    app.surface.event(&Event::Button("BUTTON_NOTE_REPEAT", false), &v, Instant::now());
    app.sync_arp();
    assert!(app.repeat_sent.is_none(), "off once let go");
}

/// The engine started for the pads while stopped (C45: a held pad played once) gets
/// Note Repeat as it is, though it was switched on before there was an engine.
#[test]
fn the_engine_started_for_the_pads_repeats() {
    let mut app = app();
    app.surface_act(A::Repeat("locked", 1.0));
    app.sync_arp();
    assert!(app.repeat_sent.is_some());
    assert!(app.idle_engine(48_000).repeating(), "on");
    app.surface_act(A::Repeat("locked", 0.0));
    app.sync_arp();
    assert!(!app.idle_engine(48_000).repeating(), "off");
}

#[test]
fn events_screen_pin_persists_across_restarts() {
    let mut app = app();
    let mut reqs = Vec::new();
    app.mk3_events(
        vec![
            Event::Button("BUTTON_EVENTS", true),
            Event::Button("BUTTON_SCREEN_1", true),
            Event::Button("BUTTON_SCREEN_1", false),
            Event::Button("BUTTON_EVENTS", false),
        ],
        &mut reqs,
    );
    assert_eq!(app.surface.page(), "events");
    assert!(app.settings.pinned_pages.contains("EVENTS"));
    assert_eq!(app.settings.pinned_open.as_deref(), Some("EVENTS"));
    app.store_screen();
    let bytes = app.doc.as_ref().unwrap().borrow().to_bytes().unwrap();

    // Restart with Events still locked open on screen: reopens on Events even after
    // restoring the project's saved screen.
    let saved_settings = crate::settings::Settings::parse(&app.settings.to_text());
    assert!(saved_settings.pinned_pages.contains("EVENTS"));
    assert_eq!(saved_settings.pinned_open.as_deref(), Some("EVENTS"));
    let mut restarted = crate::App::default();
    restarted.settings = saved_settings;
    restarted.surface = restarted.new_surface();
    restarted.show_document(deckard_doc::Document::from_bytes(bytes.clone(), None));
    assert_eq!(restarted.surface.page(), "events");

    // Tap EVENTS to close it: pinned_open clears, EVENTS stays pinned for the next restart.
    restarted.mk3_events(vec![Event::Button("BUTTON_EVENTS", true), Event::Button("BUTTON_EVENTS", false)], &mut reqs);
    assert_eq!(restarted.surface.page(), "control");
    assert!(restarted.settings.pinned_pages.contains("EVENTS"));
    assert_eq!(restarted.settings.pinned_open, None);

    let next_settings = crate::settings::Settings::parse(&restarted.settings.to_text());
    let mut third = crate::App::default();
    third.settings = next_settings;
    third.surface = third.new_surface();
    third.show_document(deckard_doc::Document::from_bytes(bytes, None));
    assert_eq!(third.surface.page(), "control");
    third.mk3_events(vec![Event::Button("BUTTON_EVENTS", true), Event::Button("BUTTON_EVENTS", false)], &mut reqs);
    assert_eq!(third.surface.page(), "events");
}
