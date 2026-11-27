use super::{frame_at, hsv_color, panel, waveform_color};
use eframe::egui::{self, Color32, Rect};
use egui_kittest::Harness;

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
