//! The painter's testable seams, with no window: the screen texture uploads only when
//! the frame's change count moves, and a drawn pointer maps back into its control.

use crate::mk3_io::PadRoute;
use crate::mk3_sink::Mk3Sink;
use crate::onscreen_mk3::Host;
use crate::onscreen_mk3_paint::{Contact, contact_step, docked_panel, docked_scale, docked_width, refresh_screens, table_point};
use deckard_mk3::leds::Leds;
use deckard_model::pad_touch::PadTouch;
use eframe::egui;

/// A frame carrying real-size screens (960 x 272 RGBA), as [`crate::Mk3Out`] documents.
fn paint_frame(count: u64) -> crate::Mk3Out {
    crate::Mk3Out {
        route: PadRoute {
            play: true,
            group: 3,
            keys: None,
            chords: None,
            velocity: None,
            scaling: 0,
            fixed: 100,
            touches: [PadTouch::default(); 16],
        },
        leds: Leds::default(),
        screens: Some((count, crate::mk3_io::ScreenFrame::Pixels(std::sync::Arc::new(vec![[9, 9, 9, 255]; 960 * 272])))),
    }
}

#[test]
fn screens_upload_only_when_the_count_moves() {
    let ctx = egui::Context::default();
    let mut host = Host::default();
    host.open = true;
    assert!(!refresh_screens(&mut host, &ctx), "no frame yet: nothing to upload");
    host.show(&paint_frame(7));
    assert!(refresh_screens(&mut host, &ctx), "the first frame uploads");
    assert!(!refresh_screens(&mut host, &ctx), "the same count never re-uploads the 1 MB image");
    host.show(&paint_frame(7));
    assert!(!refresh_screens(&mut host, &ctx), "a redrawn frame with the same count still skips the upload");
    host.show(&paint_frame(8));
    assert!(refresh_screens(&mut host, &ctx), "a moved count rebuilds the texture");
}

#[test]
fn the_drawn_pointer_maps_back_into_its_control() {
    let scale = 0.5;
    let rect =
        egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(crate::mk3_layout::WIDTH * scale, crate::mk3_layout::HEIGHT * scale));
    let pad = crate::mk3_layout::control("pad1").expect("pad1 is in the table");
    let drawn = rect.min + egui::vec2(pad.rect.center().x * scale, pad.rect.center().y * scale);
    let back = table_point(rect, scale, drawn);
    assert!(
        (back.x - pad.rect.center().x).abs() < 0.01 && (back.y - pad.rect.center().y).abs() < 0.01,
        "the drawn center maps back to pad 1's own center: {back:?}"
    );
    assert!(crate::mk3_layout::at(back).is_some_and(|c| c.id == "pad1"), "the mapped point hit-tests to pad 1, not the gap beside it");
}

/// The docked surface fits a window shorter than its aspect by height: at the
/// default window (~1500 x 930) width alone would overshoot the window's height —
/// taller than the window — so height binds instead.
#[test]
fn the_docked_panel_fits_a_short_window_by_height() {
    use crate::mk3_layout::{HEIGHT, WIDTH};
    let surface = egui::vec2(WIDTH, HEIGHT);
    let scale = docked_scale(egui::vec2(1500.0, 930.0), surface);
    assert!((scale - 930.0 / HEIGHT).abs() < 1e-6, "height binds at the default window size: {scale}");
    assert!(surface.y * scale <= 930.0 + 1e-3, "the panel fits the window it docks in");
    let tall = docked_scale(egui::vec2(2560.0, 900.0), surface);
    assert!((tall - 900.0 / HEIGHT).abs() < 1e-6, "a tall window still fits by height: {tall}");
    let narrow = docked_scale(egui::vec2(800.0, 1200.0), surface);
    assert!((narrow - 800.0 / WIDTH).abs() < 1e-6, "a narrow window fits by width: {narrow}");
}

/// The width the docked panel asks for: the surface at the window's full height, capped so a
/// window too narrow for it keeps a body. The default window is the capped case.
#[test]
fn the_docked_width_follows_the_height_until_the_window_is_narrow() {
    use crate::mk3_layout::{HEIGHT, WIDTH};
    let capped = docked_width(egui::vec2(1500.0, 930.0), false);
    assert!((capped - 1500.0 * 0.45).abs() < 1e-3, "the default window caps the column at 45% of its width: {capped}");
    let whole = docked_width(egui::vec2(2400.0, 930.0), false);
    assert!((whole - 930.0 * WIDTH / HEIGHT).abs() < 1e-3, "a window wide enough takes the surface whole: {whole}");
    let portrait = docked_width(egui::vec2(600.0, 1200.0), true);
    assert!((portrait - 600.0 * 0.45).abs() < 1e-3, "portrait is narrower than the window is tall, so the cap binds: {portrait}");
}

/// Mount the host the way the app docks it and report the width the panel offered the
/// surface. `declared` uses the app's own [`docked_panel`]; `false` rebuilds the unsized
/// panel with no declared width, which is the defect the other test pins — the first dock
/// shipped that mistake at the bottom edge, where egui's default is even smaller.
#[derive(Default)]
struct Dock {
    host: Host,
    offered: f32,
}

fn docked_offered_width(declared: bool, window: egui::Vec2) -> f32 {
    use egui_kittest::Harness;
    let mut state = Dock::default();
    state.host.open = true;
    state.host.show(&paint_frame(1));
    let mut harness = Harness::builder().with_size(window).build_ui_state(
        move |ui, state: &mut Dock| {
            if declared {
                let panel = docked_panel(600.0);
                panel.show(ui, |ui| {
                    state.offered = ui.available_size().x;
                    state.host.ui(ui);
                });
            } else {
                let panel = egui::Panel::right("mk3_panel").resizable(false);
                panel.show(ui, |ui| {
                    state.offered = ui.available_size().x;
                    state.host.ui(ui);
                });
            }
        },
        state,
    );
    harness.run();
    harness.state().offered
}

/// The defect the screenshot found, pinned: a side panel with no declared width takes egui's
/// 200 px default, and [`docked_scale`] — fitting the surface to the space on offer — fits
/// into it and reports success. Only the declared width makes the surface playable, so this
/// test fails if the panel goes back to sizing itself.
#[test]
fn a_docked_panel_is_too_small_until_it_declares_its_width() {
    let window = egui::vec2(1500.0, 930.0);
    let unsized_width = docked_offered_width(false, window);
    let declared_width = docked_offered_width(true, window);
    assert!(declared_width > window.x * 0.3, "the declared width reaches the painter as space it can draw in: {declared_width}");
    assert!(unsized_width < 250.0, "an undeclared side panel keeps egui's 200 px default, less its margins: {unsized_width}");
    assert!(
        unsized_width * 3.0 < declared_width,
        "the declared width is a surface to play, not a strip: unsized {unsized_width}, declared {declared_width}"
    );
}

/// The bug the user hit with a mouse: every tap on the on-screen MK3 did nothing, because
/// the painter asked egui for a drag and a click never moves — `drag_started` needs travel.
/// The button's held-state answers without movement, so a click is a press then a release.
#[test]
fn a_click_is_a_press_and_a_release_without_travel() {
    assert_eq!(contact_step(true, false), Contact::Down, "the press frame starts contact, no matter the travel");
    assert_eq!(contact_step(true, true), Contact::Move, "a held button keeps the contact alive");
    assert_eq!(contact_step(false, true), Contact::Release, "the release frame ends it");
    assert_eq!(contact_step(false, false), Contact::None, "an idle pointer says nothing");
}
