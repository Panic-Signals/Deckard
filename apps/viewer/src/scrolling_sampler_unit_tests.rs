use super::{
    cleanup_temporary_files, effective_fade_ms, frame_at, hsv_color, panel, poll_cached_selection, queue_cached_selection, waveform_color,
};
use eframe::egui::{self, Color32, Rect};
use egui_kittest::Harness;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn panel_reports_missing_audio_output() {
    let app = crate::App::default();
    let mut harness = Harness::builder().with_size(egui::vec2(800.0, 80.0)).build_ui_state(|ui, app: &mut crate::App| panel(ui, app), app);
    harness.run_steps(2);
}

#[test]
fn waveform_palettes_and_frame_mapping_are_stable() {
    assert_eq!(waveform_color(0, 0, 0, 1, 51), Color32::from_rgba_unmultiplied(0xb9, 0xbf, 0xc3, 51));
    assert_eq!(waveform_color(1, 0x123456, 0, 1, 240), Color32::from_rgba_unmultiplied(0x12, 0x34, 0x56, 240));
    assert_eq!(waveform_color(2, 0, 1, 3, 240).a(), 240);
    assert_eq!(hsv_color(360.0, 1.0, 1.0, 200).a(), 200);

    let wave = Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(100.0, 20.0));
    assert_eq!(frame_at(10.0, wave, 50, 200), 50);
    assert_eq!(frame_at(110.0, wave, 50, 200), 250);
}

fn prepared_scrolling(samples: &[f32], frames: usize) -> Arc<deckard_engine::scrolling::Scrolling> {
    let scrolling = Arc::new(deckard_engine::scrolling::Scrolling::default());
    scrolling.sync(true, 1.0);
    scrolling.prepare(1_000);
    scrolling.push(samples, frames, 1_000);
    scrolling
}

fn wait_for_export(app: &mut crate::App) {
    for _ in 0..100 {
        poll_cached_selection(app);
        if app.scroll_export.is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("selection export did not finish");
}

#[test]
fn cached_selection_export_reuses_matching_file_and_cleans_up() {
    cleanup_temporary_files();
    let scrolling = prepared_scrolling(&[0.25, 0.25, 0.5, 0.5], 2);
    let mut app = crate::App::default();
    app.settings.scrolling_fade_enabled = false;
    queue_cached_selection(&mut app, &scrolling, 0, 2);
    wait_for_export(&mut app);

    let path = app.scroll_cached_path.clone().expect("export path");
    assert!(path.is_file());
    assert_eq!(effective_fade_ms(&app), 0);
    queue_cached_selection(&mut app, &scrolling, 0, 2);
    assert!(app.scroll_export.is_none());
    cleanup_temporary_files();
    assert!(!path.exists());
}

#[test]
fn silent_selection_is_cached_without_a_file() {
    cleanup_temporary_files();
    let scrolling = prepared_scrolling(&[0.0, 0.0], 1);
    let mut app = crate::App::default();
    app.settings.scrolling_remove_silence = true;
    queue_cached_selection(&mut app, &scrolling, 0, 1);
    wait_for_export(&mut app);

    assert_eq!(app.scroll_cached_selection, Some((0, 1)));
    assert!(app.scroll_cached_path.is_none());
    cleanup_temporary_files();
}
