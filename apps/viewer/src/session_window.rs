//! The session Recorder window: a live view of the master-output recording.

use crate::session_record::{SessionPhase, SessionView, StopReason, seconds_available};
use eframe::egui;

/// What the user did in the window this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    None,
    /// The Play/Pause control (or the Space bar): start/resume, or pause.
    TogglePlay,
    /// The Stop button.
    Stop,
    /// The window's close button.
    Hide,
    /// The pin control: float the window above others, or stop doing so.
    TogglePin,
}

/// Whether the Recorder window floats above other windows. On by default so a
/// glance at the laptop always shows the timer during a set.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Pin {
    #[default]
    On,
    Off,
}

impl Pin {
    pub fn on(self) -> bool {
        self == Pin::On
    }
    pub fn toggled(self) -> Pin {
        if self == Pin::On { Pin::Off } else { Pin::On }
    }
}

// Sized to the waveform: the window is as wide as the scrolling wave (edge to edge) and
// just tall enough for the readout, so there is no dead space below the buttons.
const SIZE: egui::Vec2 = egui::vec2(380.0, 334.0);
const RED: egui::Color32 = egui::Color32::from_rgb(0xe0, 0x3c, 0x3c);
const AMBER: egui::Color32 = egui::Color32::from_rgb(0xe0, 0xa0, 0x30);
const OK: egui::Color32 = egui::Color32::from_rgb(0x4f, 0xc0, 0x70);

/// Open or update the Recorder window; `Hide` when its close button was pressed.
/// `pinned` floats it above other windows. The window is a fixed size and cannot be
/// resized — it is a glanceable readout, not a panel to arrange.
pub fn show(ctx: &egui::Context, view: &SessionView, pinned: bool) -> WindowAction {
    let level = if pinned { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
    let builder = egui::ViewportBuilder::default()
        .with_title("DECKARD Recorder")
        .with_inner_size(SIZE)
        .with_min_inner_size(SIZE)
        .with_max_inner_size(SIZE)
        .with_resizable(false)
        .with_window_level(level);
    let mut action = WindowAction::None;
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("deckard-session-recorder"), builder, |ui, class| {
        if class == egui::ViewportClass::EmbeddedWindow {
            ui.set_min_size(SIZE);
        }
        action = body(ui, view, pinned);
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

/// A rough "time remaining" label from a count of seconds, e.g. `~2h 30m left`,
/// `~45m left`, or `~30s left`.
pub fn format_time_left(secs: u64) -> String {
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    if h > 0 {
        format!("~{h}h {m:02}m left")
    } else if m > 0 {
        format!("~{m}m left")
    } else {
        format!("~{secs}s left")
    }
}

/// The message shown after an automatic stop, or `None` when the reason needs no line of
/// its own (a manual stop, or a write error that already shows `view.error`).
fn reason_text(reason: StopReason) -> Option<&'static str> {
    match reason {
        StopReason::Disk => Some("Stopped: disk reserve reached"),
        StopReason::Ceiling => Some("Stopped: 4 GB file limit"),
        StopReason::RateChange | StopReason::DeviceChange => Some("Stopped: audio device changed"),
        StopReason::WriteError | StopReason::Manual => None,
    }
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
pub fn body(ui: &mut egui::Ui, view: &SessionView, pinned: bool) -> WindowAction {
    let mut action = WindowAction::None;
    let recording = view.phase == SessionPhase::Recording;
    let paused = view.phase == SessionPhase::Paused;
    let has_file = recording || paused;
    let active = view.phase != SessionPhase::Idle;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        // The status light: a pulsing red while recording, a steady amber while paused,
        // otherwise a dim grey.
        let (label, dot, pulse) = match view.phase {
            SessionPhase::Recording => ("RECORDING SESSION", RED, true),
            SessionPhase::Paused => ("PAUSED", AMBER, false),
            SessionPhase::Armed => ("READY", egui::Color32::GRAY, false),
            SessionPhase::Idle => ("SESSION RECORDER", egui::Color32::GRAY, false),
        };
        let lit = !pulse || ui.input(|i| i.time).fract() < 0.6;
        let color = if lit { dot } else { dot.gamma_multiply(0.4) };
        ui.painter().circle_filled(rect.center(), 6.0, color);
        ui.label(egui::RichText::new(label).strong());
        // Pin control (floats the window on top), right-aligned.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let hint = if pinned { "Unpin — stop floating above other windows" } else { "Pin — keep this window on top" };
            if ui.selectable_label(pinned, "📌 Pin").on_hover_text(hint).clicked() {
                action = WindowAction::TogglePin;
            }
        });
    });
    if active {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    // The timer, centered above the waveform, with the recording stats beneath it.
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(format_elapsed(view.elapsed)).size(48.0).monospace());
        if has_file {
            ui.horizontal(|ui| {
                ui.label(format!("started {}", view.started_label));
                ui.label(format!("{} on disk", format_bytes(view.bytes)));
                // How much longer it can record before it auto-stops (disk reserve or 4 GB).
                ui.label(format_time_left(seconds_available(view.free, view.reserve, view.rate)));
            });
        }
    });
    ui.add_space(6.0);
    wave(ui, view);
    ui.add_space(6.0);
    let tint = tint(view.free, view.reserve);
    for (name, level, peak) in [("L", view.meter.0, view.peak.0), ("R", view.meter.1, view.peak.1)] {
        ui.horizontal(|ui| {
            ui.label(name);
            let peak_label = format!("peak {} dB", db_text(peak));
            // The bar fills the row: full width minus the channel letter and the peak readout.
            let bar_w = (ui.available_width() - 96.0).max(40.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_w, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(0x22));
            let fill = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * level.clamp(0.0, 1.0), rect.height()));
            ui.painter().rect_filled(fill, 2.0, tint);
            ui.label(peak_label);
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
    // After an automatic stop, say why (a write error already shows above).
    if !active && let Some(text) = view.stopped.and_then(reason_text) {
        ui.colored_label(AMBER, text);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        // Play while armed or paused; pause while recording.
        let (play_label, play_hint) = if recording {
            ("⏸ Pause", "Pause — hold the recording (Space)")
        } else {
            ("▶ Play", "Play — start or resume recording (Space)")
        };
        if ui.add_enabled(active, egui::Button::new(play_label)).on_hover_text(play_hint).clicked() {
            action = WindowAction::TogglePlay;
        }
        if ui.add_enabled(active, egui::Button::new("Stop")).clicked() {
            action = WindowAction::Stop;
        }
    });
    // Space toggles Play/Pause when this window has focus (and nothing is being clicked).
    if action == WindowAction::None && active && ui.input(|i| i.focused && i.key_pressed(egui::Key::Space)) {
        action = WindowAction::TogglePlay;
    }
    action
}

/// The scrolling stereo waveform: the left channel in the top lane and the right in the
/// bottom lane, each a bar mirrored around its lane centre, newest on the right. Each
/// bucket in `view.wave` is that bucket's (left peak, right peak).
fn wave(ui: &mut egui::Ui, view: &SessionView) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 70.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(0x16));
    let n = view.wave.len().max(1);
    let step = rect.width() / n as f32;
    let lane = rect.height() / 2.0;
    let l_mid = rect.top() + lane * 0.5;
    let r_mid = rect.top() + lane * 1.5;
    // A faint line between the two lanes.
    painter.line_segment(
        [egui::pos2(rect.left(), rect.center().y), egui::pos2(rect.right(), rect.center().y)],
        egui::Stroke::new(1.0, egui::Color32::from_gray(0x2a)),
    );
    let stroke = egui::Stroke::new(step.max(1.0), RED);
    for (i, (l, r)) in view.wave.iter().enumerate() {
        let x = rect.left() + (i as f32 + 0.5) * step;
        // Each channel mirrored around its lane centre, leaving a hair of gap at the line.
        let lh = (l.clamp(0.0, 1.0) * (lane * 0.5 - 1.0)).max(0.5);
        let rh = (r.clamp(0.0, 1.0) * (lane * 0.5 - 1.0)).max(0.5);
        painter.line_segment([egui::pos2(x, l_mid - lh), egui::pos2(x, l_mid + lh)], stroke);
        painter.line_segment([egui::pos2(x, r_mid - rh), egui::pos2(x, r_mid + rh)], stroke);
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
    fn time_left_reads_naturally() {
        assert_eq!(format_time_left(9_000), "~2h 30m left"); // 2h 30m
        assert_eq!(format_time_left(2_700), "~45m left");
        assert_eq!(format_time_left(30), "~30s left");
        assert_eq!(format_time_left(3_600), "~1h 00m left");
    }

    #[test]
    fn stopped_reason_line_appears() {
        let view = SessionView { phase: SessionPhase::Idle, stopped: Some(StopReason::Disk), ..SessionView::default() };
        let mut h = Harness::new_ui(|ui| {
            body(ui, &view, true);
        });
        h.run();
        assert!(h.query_by_label("Stopped: disk reserve reached").is_some(), "the auto-stop reason is shown");
    }

    #[test]
    fn pin_logic_toggles() {
        assert!(Pin::default().on(), "pinned by default");
        assert!(!Pin::On.toggled().on());
        assert!(Pin::Off.toggled().on());
    }

    #[test]
    fn pin_control_renders_and_toggles() {
        let view = SessionView::default();
        // Latch any non-None action: `run()` settles over several frames and `clicked()`
        // is true on only one of them.
        let action = std::cell::Cell::new(WindowAction::None);
        let mut h = Harness::new_ui(|ui| {
            let a = body(ui, &view, true);
            if a != WindowAction::None {
                action.set(a);
            }
        });
        h.run();
        assert!(h.query_by_label("📌 Pin").is_some(), "the pin control is shown");
        h.get_by_label("📌 Pin").click();
        h.run();
        assert_eq!(action.get(), WindowAction::TogglePin, "clicking the pin control toggles the pin");
    }

    #[test]
    fn window_renders_idle_and_recording() {
        let mut view = SessionView::default();
        let mut h = Harness::new_ui(|ui| {
            body(ui, &view, true);
        });
        h.run();
        assert!(h.query_by_label("00:00:00").is_some(), "idle shows a zero timer");
        drop(h);

        view.phase = SessionPhase::Recording;
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
            body(ui, &view, true);
        });
        // (It repaints while recording, so step rather than run to rest.)
        h.run_steps(2);
        assert!(h.query_by_label("00:12:47").is_some(), "timer text is rendered");
        assert!(h.query_by_label("48 kHz · 24-bit · stereo").is_some());
        assert!(h.query_by_label("session.wav").is_some());
    }

    #[test]
    fn armed_shows_ready_and_a_play_button() {
        let view = SessionView { phase: SessionPhase::Armed, ..SessionView::default() };
        let mut h = Harness::new_ui(|ui| {
            body(ui, &view, true);
        });
        h.run_steps(2);
        assert!(h.query_by_label("READY").is_some(), "armed shows READY");
        assert!(h.query_by_label("▶ Play").is_some(), "armed offers Play");
    }

    #[test]
    fn play_pause_control_toggles() {
        for (phase, label) in [(SessionPhase::Armed, "▶ Play"), (SessionPhase::Recording, "⏸ Pause"), (SessionPhase::Paused, "▶ Play")]
        {
            let view = SessionView { phase, ..SessionView::default() };
            let action = std::cell::Cell::new(WindowAction::None);
            let mut h = Harness::new_ui(|ui| {
                let a = body(ui, &view, true);
                if a != WindowAction::None {
                    action.set(a);
                }
            });
            h.run_steps(2);
            assert!(h.query_by_label(label).is_some(), "{phase:?} shows {label}");
            h.get_by_label(label).click();
            h.run_steps(2);
            assert_eq!(action.get(), WindowAction::TogglePlay, "{phase:?}: the control toggles play/pause");
        }
    }
}
