//! SELECT + ALL / NONE / MULTI on the pads of the focused group.

use deckard_mk3::Event;
use std::time::Instant;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app
}

fn act(app: &mut crate::App, ev: Event) {
    let view = app.surface_view();
    for action in app.surface.event(&ev, &view, Instant::now()) {
        app.surface_act(action);
    }
}

fn press(app: &mut crate::App, name: &'static str) {
    act(app, Event::Button(name, true));
    act(app, Event::Button(name, false));
}

fn selected(app: &crate::App) -> Vec<bool> {
    app.surface_view().groups[app.group].sounds.iter().map(|s| s.selected).collect()
}

#[test]
fn select_all_none_and_multi_pads() {
    let mut app = app();
    let n = selected(&app).len();
    assert!(n >= 6, "a group has pads to select");
    assert_eq!(selected(&app).iter().filter(|on| **on).count(), 1, "only the focus starts selected");

    act(&mut app, Event::Button("BUTTON_SELECT", true));
    press(&mut app, "BUTTON_SCREEN_5");
    assert!(selected(&app).iter().all(|on| *on), "ALL selects every pad");
    assert_eq!(app.sound, 0, "ALL leaves the focus");

    press(&mut app, "BUTTON_SCREEN_6");
    let mut only_focus = vec![false; n];
    only_focus[0] = true;
    assert_eq!(selected(&app), only_focus, "NONE keeps the focused pad");

    press(&mut app, "BUTTON_SCREEN_8");
    act(&mut app, Event::PadOn(3, 0.5));
    act(&mut app, Event::PadOff(3));
    only_focus[2] = true;
    assert_eq!(selected(&app), only_focus, "MULTI on: the next pad joins");
    assert_eq!(app.sound, 0, "joining a pad leaves the focus");

    act(&mut app, Event::PadOn(3, 0.5));
    act(&mut app, Event::PadOff(3));
    only_focus[2] = false;
    assert_eq!(selected(&app), only_focus, "MULTI on: that pad leaves");

    press(&mut app, "BUTTON_SCREEN_8");
    act(&mut app, Event::PadOn(5, 0.5));
    act(&mut app, Event::PadOff(5));
    let mut replaced = vec![false; n];
    replaced[4] = true;
    assert_eq!(app.sound, 4, "MULTI off: an unselected pad takes the focus");
    assert_eq!(selected(&app), replaced, "and it is the only one selected");
}
