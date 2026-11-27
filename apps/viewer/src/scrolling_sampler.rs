use eframe::egui::{Align2, Color32, FontId, Pos2, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
pub struct DraggedSelection(pub PathBuf);

pub const HEIGHT: f32 = 29.0;
const WAVE_HEIGHT: f32 = 25.0;

static NEXT_SELECTION_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_directory() -> PathBuf {
    std::env::temp_dir().join("deckard-scrolling-sampler").join(std::process::id().to_string())
}

pub fn cleanup_temporary_files() {
    let directory = temporary_directory();
    let parent = directory.parent().map(PathBuf::from);
    let _ = std::fs::remove_dir_all(directory);
    if let Some(parent) = parent {
        let _ = std::fs::remove_dir(parent);
    }
}

fn effective_fade_ms(app: &crate::App) -> u32 {
    if app.settings.scrolling_fade_enabled { app.settings.scrolling_fade_ms } else { 0 }
}

pub fn panel(ui: &mut Ui, app: &mut crate::App) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, Color32::from_rgb(0x18, 0x18, 0x18));
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, Color32::from_rgb(0x32, 0x32, 0x32)), StrokeKind::Inside);
    let wave = eframe::egui::Rect::from_center_size(rect.center(), Vec2::new((rect.width() - 24.0).max(0.0), WAVE_HEIGHT));
    let response = ui.interact(wave, ui.id().with("scrolling-wave"), Sense::click_and_drag());
    let Some(scrolling) = app.output.as_ref().and_then(|output| output.as_ref().ok()).map(|output| output.scrolling()) else {
        painter.text(wave.center(), Align2::CENTER_CENTER, "Audio output unavailable", FontId::proportional(12.0), Color32::GRAY);
        return;
    };
    poll_cached_selection(app);
    scrolling.sync(app.settings.scrolling_sampler, app.settings.scrolling_seconds as f32);
    if response.clicked() && ui.input(|i| i.modifiers.ctrl) {
        scrolling.set_paused(!scrolling.paused());
        ui.ctx().request_repaint();
    }
    let (all, total, rate) = scrolling.snapshot(1);
    let available = scrolling.available_frames();
    let base_seconds = app.settings.scrolling_seconds as f32;
    let visible_seconds = if app.scroll_view_seconds > 0.0 { app.scroll_view_seconds } else { base_seconds };
    let visible = ((visible_seconds * rate.max(1) as f32) as u64).min(available.max(1));
    let max_pan = available.saturating_sub(visible);
    app.scroll_pan_frames = app.scroll_pan_frames.min(max_pan);
    // Pan is measured backwards from the live edge. With no manual pan, a zoomed
    // view therefore continues following the newest captured audio.
    let from = total.saturating_sub(visible).saturating_sub(app.scroll_pan_frames);
    let pointer_in_wave = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| wave.contains(p)));
    if response.hovered() && pointer_in_wave {
        let scroll: f32 = ui.input(|i| {
            let delta = i.smooth_scroll_delta;
            if delta.y.abs() >= delta.x.abs() { delta.y } else { delta.x }
        });
        if scroll != 0.0 {
            let shift = ui.input(|i| i.modifiers.shift);
            if shift {
                let direction = if scroll.is_sign_positive() { -1.0 } else { 1.0 };
                let steps = (scroll.abs() / 24.0).max(0.5);
                let factor = 1.5_f32.powf(direction * steps).clamp(0.125, 8.0);
                app.scroll_view_seconds = (visible_seconds * factor).clamp(1.0, base_seconds);
                if let Some((start, end)) = app.scroll_selection.filter(|(start, end)| end > start) {
                    // Once the user explicitly zooms, keep the selected range in
                    // focus rather than jumping back to the live edge. Creating
                    // or resizing a selection itself does not change the view.
                    let next_visible = ((app.scroll_view_seconds * rate.max(1) as f32) as u64).min(available.max(1));
                    let center = start.saturating_add(end.saturating_sub(start) / 2);
                    let centered_from = center.saturating_sub(next_visible / 2);
                    let oldest = total.saturating_sub(available);
                    let next_max_pan = available.saturating_sub(next_visible.min(available));
                    let clamped_from = centered_from.clamp(oldest, total.saturating_sub(next_visible));
                    app.scroll_pan_frames = total.saturating_sub(next_visible).saturating_sub(clamped_from).min(next_max_pan);
                }
            } else {
                let step = (visible as f32 * scroll.abs() / 6.0) as u64;
                if scroll < 0.0 {
                    app.scroll_pan_frames = app.scroll_pan_frames.saturating_add(step).min(max_pan);
                } else {
                    app.scroll_pan_frames = app.scroll_pan_frames.saturating_sub(step);
                }
            }
        }
        if response.drag_started() {
            let pointer_frame = ui.input(|i| i.pointer.press_origin()).map(|p| frame_at(p.x, wave, from, visible));
            let pointer_x = ui.input(|i| i.pointer.press_origin()).map(|p| p.x);
            let edge = app.scroll_selection.and_then(|(start, end)| {
                if end <= start {
                    return None;
                }
                let left = wave.left() + (start.saturating_sub(from) as f32 / visible.max(1) as f32) * wave.width();
                let right = wave.left() + (end.saturating_sub(from) as f32 / visible.max(1) as f32) * wave.width();
                pointer_x.and_then(|x| {
                    if (x - left).abs() <= 8.0 {
                        Some(0)
                    } else if (x - right).abs() <= 8.0 {
                        Some(1)
                    } else {
                        None
                    }
                })
            });
            if let Some(edge) = edge {
                app.scroll_resize_edge = Some(edge);
            } else if app
                .scroll_selection
                .is_some_and(|(start, end)| pointer_frame.is_some_and(|f| (start..=end).contains(&f)) && end > start)
            {
                app.scroll_dragging_selection = true;
                if let Some((start, end)) = app.scroll_selection {
                    if let Some(path) = cached_selection_path(app, &scrolling, start, end) {
                        response.dnd_set_drag_payload(DraggedSelection(path));
                    }
                }
            } else {
                app.scroll_drag_start = pointer_frame;
                app.scroll_selection = app.scroll_drag_start.map(|f| (f, f));
            }
        }
        if response.dragged()
            && let (Some(start), Some(pos)) = (app.scroll_drag_start, ui.input(|i| i.pointer.interact_pos()))
        {
            let end = frame_at(pos.x, wave, from, visible);
            app.scroll_selection = Some((start.min(end), start.max(end)));
        }
        if response.dragged()
            && let (Some(edge), Some((start, end)), Some(pos)) =
                (app.scroll_resize_edge, app.scroll_selection, ui.input(|i| i.pointer.interact_pos()))
        {
            let frame = frame_at(pos.x, wave, from, visible);
            app.scroll_selection =
                Some(if edge == 0 { (frame.min(end.saturating_sub(1)), end) } else { (start, frame.max(start.saturating_add(1))) });
        }
    }
    if app.scroll_dragging_selection
        && ui.input(|i| i.pointer.primary_down())
        && let Some((start, end)) = app.scroll_selection.filter(|(start, end)| end > start)
        && app.scroll_cached_selection == Some((start, end))
        && app.scroll_cached_remove_silence == app.settings.scrolling_remove_silence
        && app.scroll_cached_fade_ms == app.settings.scrolling_fade_ms
        && let Some(path) = app.scroll_cached_path.as_ref().filter(|path| path.is_file()).cloned()
    {
        response.dnd_set_drag_payload(DraggedSelection(path));
    }
    if response.drag_stopped() {
        app.scroll_drag_start = None;
        app.scroll_resize_edge = None;
    }
    if ui.input(|i| i.pointer.any_released()) {
        app.scroll_dragging_selection = false;
    }
    if !app.scroll_dragging_selection
        && app.scroll_drag_start.is_none()
        && app.scroll_resize_edge.is_none()
        && let Some((start, end)) = app.scroll_selection.filter(|(start, end)| end > start)
    {
        queue_cached_selection(app, &scrolling, start, end);
    }
    let peaks = scrolling.snapshot_range(from, visible, wave.width().max(1.0) as usize).unwrap_or(all);
    let mid = wave.center().y;
    let amp = wave.height() * 0.48;
    let mut top = Vec::with_capacity(peaks.len());
    let mut bottom = Vec::with_capacity(peaks.len());
    for (i, &(lo, hi)) in peaks.iter().enumerate() {
        let x = wave.left() + i as f32 + 0.5;
        let y_top = mid - hi.clamp(-1.0, 1.0) * amp;
        let y_bottom = mid - lo.clamp(-1.0, 1.0) * amp;
        top.push(Pos2::new(x, y_top));
        bottom.push(Pos2::new(x, y_bottom));
        let fill = waveform_color(app.settings.scrolling_palette, app.settings.scrolling_color, i, peaks.len(), 51);
        painter.rect_filled(
            eframe::egui::Rect::from_min_max(
                Pos2::new(x - 0.5, y_top.min(y_bottom)),
                Pos2::new(x + 0.5, y_top.max(y_bottom).max(y_top.min(y_bottom) + 1.0)),
            ),
            0.0,
            fill,
        );
    }
    let color = waveform_color(app.settings.scrolling_palette, app.settings.scrolling_color, 0, peaks.len(), 240);
    for i in 1..top.len() {
        let outline = waveform_color(app.settings.scrolling_palette, app.settings.scrolling_color, i, peaks.len(), 240);
        let stroke = Stroke::new(1.0, if app.settings.scrolling_palette == 2 { outline } else { color });
        painter.line_segment([top[i - 1], top[i]], stroke);
        painter.line_segment([bottom[i - 1], bottom[i]], stroke);
    }
    if let Some((start, end)) = app.scroll_selection {
        let left = wave.left() + (start.saturating_sub(from) as f32 / visible.max(1) as f32) * wave.width();
        let right = wave.left() + (end.saturating_sub(from) as f32 / visible.max(1) as f32) * wave.width();
        let selection = eframe::egui::Rect::from_x_y_ranges(left.min(right)..=left.max(right), wave.y_range());
        painter.rect_filled(selection, 0.0, Color32::from_rgba_unmultiplied(80, 180, 220, 45));
        painter.rect_stroke(selection, 0.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 210, 240, 220)), StrokeKind::Inside);
    }
    if visible_seconds < app.settings.scrolling_seconds as f32 - 0.05 {
        painter.text(
            Pos2::new(rect.right() - 6.0, rect.top() + 2.0),
            Align2::RIGHT_TOP,
            format!("{visible_seconds:.1} / {} s", app.settings.scrolling_seconds),
            FontId::proportional(9.0),
            Color32::GRAY,
        );
    }
    if scrolling.paused() {
        painter.text(
            wave.right_center() - Vec2::new(4.0, 0.0),
            Align2::RIGHT_CENTER,
            "PAUSED",
            FontId::proportional(10.0),
            Color32::from_rgb(0xe0, 0xa0, 0x60),
        );
    }
}

fn cached_selection_path(
    app: &mut crate::App,
    scrolling: &std::sync::Arc<deckard_engine::scrolling::Scrolling>,
    start: u64,
    end: u64,
) -> Option<PathBuf> {
    let settings_match =
        app.scroll_cached_remove_silence == app.settings.scrolling_remove_silence && app.scroll_cached_fade_ms == effective_fade_ms(app);
    if app.scroll_cached_selection == Some((start, end)) && settings_match {
        if let Some(path) = app.scroll_cached_path.as_ref().filter(|path| path.is_file()) {
            return Some(path.clone());
        }
    }
    queue_cached_selection(app, scrolling, start, end);
    None
}

fn queue_cached_selection(app: &mut crate::App, scrolling: &std::sync::Arc<deckard_engine::scrolling::Scrolling>, start: u64, end: u64) {
    let settings_match =
        app.scroll_cached_remove_silence == app.settings.scrolling_remove_silence && app.scroll_cached_fade_ms == effective_fade_ms(app);
    if app.scroll_cached_selection == Some((start, end)) && settings_match || app.scroll_export.is_some() {
        return;
    }
    let directory = temporary_directory();
    let _ = std::fs::create_dir_all(&directory);
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let sequence = NEXT_SELECTION_ID.fetch_add(1, Ordering::Relaxed);
    let path = directory.join(format!("selection-{timestamp}-{sequence}.wav"));
    let remove_silence = app.settings.scrolling_remove_silence;
    let fade_ms = effective_fade_ms(app);
    let scrolling = std::sync::Arc::clone(scrolling);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = scrolling.export_wav(&path, start, end - start, remove_silence, fade_ms);
        let _ = sender.send((start, end, remove_silence, fade_ms, path, result));
    });
    app.scroll_export = Some(receiver);
    app.scroll_cached_selection = None;
    app.scroll_cached_path = None;
}

fn poll_cached_selection(app: &mut crate::App) {
    let Some(receiver) = app.scroll_export.as_ref() else { return };
    let Ok((start, end, remove_silence, fade_ms, path, result)) = receiver.try_recv() else { return };
    app.scroll_export = None;
    if result.is_ok() {
        if let Some(old_path) = app.scroll_cached_path.replace(path) {
            let _ = std::fs::remove_file(old_path);
        }
        app.scroll_cached_selection = Some((start, end));
        app.scroll_cached_remove_silence = remove_silence;
        app.scroll_cached_fade_ms = fade_ms;
    } else {
        let _ = std::fs::remove_file(path);
        if result.as_ref().err().is_some_and(|error| error == "the selection is silent") {
            app.scroll_cached_selection = Some((start, end));
            app.scroll_cached_remove_silence = remove_silence;
            app.scroll_cached_fade_ms = fade_ms;
            app.scroll_cached_path = None;
        }
    }
}

fn waveform_color(palette: u8, packed: u32, column: usize, columns: usize, alpha: u8) -> Color32 {
    match palette {
        1 => Color32::from_rgba_unmultiplied((packed >> 16) as u8, (packed >> 8) as u8, packed as u8, alpha),
        2 => hsv_color(column as f32 / columns.saturating_sub(1).max(1) as f32 * 300.0, 1.0, 1.0, alpha),
        _ => Color32::from_rgba_unmultiplied(0xb9, 0xbf, 0xc3, alpha),
    }
}

fn hsv_color(hue: f32, saturation: f32, value: f32, alpha: u8) -> Color32 {
    let h = (hue.rem_euclid(360.0) / 60.0).floor() as i32;
    let f = hue.rem_euclid(60.0) / 60.0;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - f * saturation);
    let t = value * (1.0 - (1.0 - f) * saturation);
    let (r, g, b) = match h {
        0 => (value, t, p),
        1 => (q, value, p),
        2 => (p, value, t),
        3 => (p, q, value),
        4 => (t, p, value),
        _ => (value, p, q),
    };
    Color32::from_rgba_unmultiplied((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, alpha)
}

fn frame_at(x: f32, wave: eframe::egui::Rect, from: u64, visible: u64) -> u64 {
    from.saturating_add((((x - wave.left()) / wave.width().max(1.0)).clamp(0.0, 1.0) * visible as f32) as u64)
}
