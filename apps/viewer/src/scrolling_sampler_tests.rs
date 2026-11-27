use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn audio_preferences() -> Harness<'static, crate::App> {
    let app = crate::App { prefs: Some(2), ..Default::default() };
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut crate::App| {
            let ctx = ui.ctx().clone();
            app.preferences(&ctx);
        },
        app,
    );
    harness.run_steps(3);
    harness
}

fn enabled_audio_preferences() -> Harness<'static, crate::App> {
    let mut app = crate::App { prefs: Some(2), ..Default::default() };
    app.settings.scrolling_sampler = true;
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut crate::App| {
            let ctx = ui.ctx().clone();
            app.preferences(&ctx);
        },
        app,
    );
    harness.run_steps(3);
    harness
}

#[test]
fn audio_preferences_show_scrolling_sampler_controls() {
    let harness = audio_preferences();
    assert!(harness.query_by_label("Scrolling sampler").is_some());
    assert!(harness.query_by_label("Capture recent master output").is_some());
    assert!(harness.query_by_label("Dropped selection instrument").is_some());
}

#[test]
fn disabled_capture_hides_buffer_controls() {
    let harness = audio_preferences();
    assert!(harness.query_by_label("Buffer").is_none());
    assert!(harness.query_by_label("Waveform colour").is_none());
}

#[test]
fn enabled_capture_shows_selection_and_drop_controls() {
    let harness = enabled_audio_preferences();
    assert!(harness.query_by_label("Buffer").is_some());
    assert!(harness.query_by_label("Waveform colour").is_some());
    assert!(harness.query_by_label("Selection cleanup").is_some());
    assert!(harness.query_by_label("Fade to zero crossing").is_some());
    assert!(harness.query_by_label("Fade duration").is_some());
    assert!(harness.query_by_label("Dropped selection instrument").is_some());
}
