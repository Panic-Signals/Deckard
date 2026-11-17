//! Preferences ▸ Hardware: the MK3's LED and display brightness, offered for the MK3
//! alone — the only model whose two brightnesses were measured (`settings::sets_brightness`,
//! `docs/mk3-usb.md`).

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// `model`'s Preferences window, on its Hardware tab.
fn hardware(model: &str) -> Harness<'static, crate::App> {
    let app = crate::App { prefs: Some(4), model: model.into(), ..Default::default() };
    let mut h = Harness::builder().with_size(egui::vec2(820.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut crate::App| {
            let ctx = ui.ctx().clone();
            app.preferences(&ctx);
        },
        app,
    );
    h.run_steps(3);
    h
}

/// Both brightnesses are the MK3's own, so its Hardware page sets them: the LEDs' level
/// (Maschine's Hardware > LEDs) and the displays' backlight (Hardware > Display), the
/// latter saying they apply when it connects while none is plugged in.
#[test]
fn the_mk3s_leds_and_displays_are_set_here() {
    let h = hardware("M3");
    assert!(h.query_by_label("LED brightness").is_some(), "the LEDs' slider");
    assert!(h.query_by_label("Display brightness").is_some(), "the displays' slider");
    assert!(h.query_by_label("Applies when the MK3 connects.").is_some(), "said with none attached");
}

/// Another controller's lights aren't DECKARD's to set, so there are no sliders for them
/// rather than sliders that do nothing.
#[test]
fn another_controller_has_neither() {
    let h = hardware("M1");
    assert!(h.query_by_label("LED brightness").is_none());
    assert!(h.query_by_label("Display brightness").is_none());
}

/// The pads' velocity curve is picked here too, as on the MK3's Settings page: Linear to
/// begin with, and choosing another sets it.
#[test]
fn velocity_scaling_is_picked_from_a_list() {
    let mut h = hardware("M3");
    assert!(h.query_by_label("Velocity scaling").is_some());
    // The list shows what's chosen as its value; its entries are labels.
    h.get_by_value("Linear").click();
    h.run_steps(3);
    h.get_by_label("Hard 2").click();
    h.run_steps(3);
    assert_eq!(h.state().settings.velocity_scaling, 2);
    assert!(h.query_by_value("Hard 2").is_some(), "shown as chosen");
}
