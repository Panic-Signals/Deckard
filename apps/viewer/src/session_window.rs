//! The session Recorder window: a live view of the master-output recording.

use crate::session_record::SessionView;
use eframe::egui;

/// What the user did in the window this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    None,
    /// The Stop button.
    Stop,
    /// The window's close button.
    Hide,
}

const SIZE: egui::Vec2 = egui::vec2(440.0, 400.0);
const RED: egui::Color32 = egui::Color32::from_rgb(0xe0, 0x3c, 0x3c);
const AMBER: egui::Color32 = egui::Color32::from_rgb(0xe0, 0xa0, 0x30);
const OK: egui::Color32 = egui::Color32::from_rgb(0x4f, 0xc0, 0x70);

/// Open or update the Recorder window; `Hide` when its close button was pressed.
pub fn show(ctx: &egui::Context, view: &SessionView) -> WindowAction {
    let builder = egui::ViewportBuilder::default().with_title("DECKARD Recorder").with_inner_size(SIZE);
    let mut action = WindowAction::None;
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("deckard-session-recorder"), builder, |ui, class| {
        if class == egui::ViewportClass::EmbeddedWindow {
            ui.set_min_size(SIZE);
        }
        action = body(ui, view);
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            action = WindowAction::Hide;
        }
    });
    action
}

/// `HH:MM:SS` for `secs`.
pub fn format_elapsed(secs: f64) -> String {
    let t = secs.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60)
}

/// `bytes` as MB or GB.
fn format_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    if mb >= 1000.0 { format!("{:.2} GB", mb / 1000.0) } else { format!("{mb:.1} MB") }
}

/// A linear level as dBFS text.
fn db_text(level: f32) -> String {
    if level <= 0.0 { "-inf".into() } else { format!("{:.1}", 20.0 * level.log10()) }
}

/// The meters' tint: green, then amber and red as the free space nears the reserve.
fn tint(free: u64, reserve: u64) -> egui::Color32 {
    if reserve == 0 || free > reserve.saturating_mul(2) {
        OK
    } else if free > reserve.saturating_add(reserve / 2) {
        AMBER
    } else {
        RED
    }
}

/// The window's contents.
pub fn body(ui: &mut egui::Ui, view: &SessionView) -> WindowAction {
    let mut action = WindowAction::None;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        // Pulses while recording.
        let on = view.recording && ui.input(|i| i.time).fract() < 0.6;
        let color = if !view.recording { egui::Color32::GRAY } else if on { RED } else { RED.gamma_multiply(0.4) };
        ui.painter().circle_filled(rect.center(), 6.0, color);
        ui.label(egui::RichText::new(if view.recording { "RECORDING SESSION" } else { "SESSION RECORDER" }).strong());
    });
    if view.recording {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    ui.label(egui::RichText::new(format_elapsed(view.elapsed)).size(48.0).monospace());
    ui.horizontal(|ui| {
        if view.recording {
            ui.label(format!("started {}", view.started_label));
            ui.label(format!("{} on disk", format_bytes(view.bytes)));
        }
    });
    ui.add_space(6.0);
    wave(ui, view);
    ui.add_space(6.0);
    let tint = tint(view.free, view.reserve);
    for (name, level, peak) in [("L", view.meter.0, view.peak.0), ("R", view.meter.1, view.peak.1)] {
        ui.horizontal(|ui| {
            ui.label(name);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(260.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(0x22));
            let fill = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * level.clamp(0.0, 1.0), rect.height()));
            ui.painter().rect_filled(fill, 2.0, tint);
            ui.label(format!("peak {} dB", db_text(peak)));
        });
    }
    ui.add_space(6.0);
    if view.rate > 0 {
        ui.label(format!("{} kHz · 24-bit · stereo", view.rate as f64 / 1000.0));
    }
    if let Some(name) = view.path.as_ref().and_then(|p| p.file_name()) {
        ui.label(name.to_string_lossy().to_string());
    }
    if let Some(e) = &view.error {
        ui.colored_label(RED, e);
    }
    ui.add_space(6.0);
    if ui.add_enabled(view.recording, egui::Button::new("Stop")).clicked() {
        action = WindowAction::Stop;
    }
    action
}

/// The scrolling waveform: one min/max bar per bucket, newest on the right.
fn wave(ui: &mut egui::Ui, view: &SessionView) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(400.0), 70.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(0x16));
    let n = view.wave.len().max(1);
    let step = rect.width() / n as f32;
    let mid = rect.center().y;
    for (i, (lo, hi)) in view.wave.iter().enumerate() {
        let x = rect.left() + (i as f32 + 0.5) * step;
        let (top, bottom) = (mid - hi.clamp(-1.0, 1.0) * rect.height() / 2.0, mid - lo.clamp(-1.0, 1.0) * rect.height() / 2.0);
        painter.line_segment([egui::pos2(x, top), egui::pos2(x, bottom.max(top + 1.0))], egui::Stroke::new(step.max(1.0), RED));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn elapsed_is_hh_mm_ss() {
        assert_eq!(format_elapsed(767.0), "00:12:47");
        assert_eq!(format_elapsed(0.0), "00:00:00");
        assert_eq!(format_elapsed(3661.9), "01:01:01");
        assert_eq!(format_elapsed(-3.0), "00:00:00");
    }

    #[test]
    fn window_renders_idle_and_recording() {
        let mut view = SessionView::default();
        let mut h = Harness::new_ui(|ui| {
            body(ui, &view);
        });
        h.run();
        assert!(h.query_by_label("00:00:00").is_some(), "idle shows a zero timer");
        drop(h);

        view.recording = true;
        view.elapsed = 767.0;
        view.started_label = "14:02".into();
        view.rate = 48_000;
        view.bytes = 5_000_000;
        view.free = 10_000_000_000;
        view.reserve = 1_000_000_000;
        view.meter = (0.5, 0.25);
        view.peak = (0.9, 0.8);
        view.path = Some("/tmp/session.wav".into());
        view.wave = (0..50).map(|i| (i as f32 / 50.0, i as f32 / 100.0)).collect();
        let mut h = Harness::new_ui(|ui| {
            body(ui, &view);
        });
        // (It repaints while recording, so step rather than run to rest.)
        h.run_steps(2);
        assert!(h.query_by_label("00:12:47").is_some(), "timer text is rendered");
        assert!(h.query_by_label("48 kHz · 24-bit · stereo").is_some());
        assert!(h.query_by_label("session.wav").is_some());
    }
}
