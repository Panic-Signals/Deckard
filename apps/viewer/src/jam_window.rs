//! The Jam's strip as its own window: a second eframe viewport, chromeless, mouse-inert,
//! always on top, drawn from the same published page `jam_strip` renders.
//!
//! Why a window and not a docked row (`docs/superpowers/specs/2026-11-21-jam-overlay-design.md`):
//! the reference's strip floats over whatever view is showing, and
//! because it is a window it needs no rule to avoid the mouse — where a docked strip had to
//! actively ignore clicks. Both properties come free here.
//!
//! What this module owns: the window's chrome (its bar: the page's name and the `n/N` counter at
//! the left, and nothing at the right — the reference has NI's mark there and this is not
//! Maschine), its geometry, and where it goes. What it must
//! not own: the page, the cursor, or the edit flag — those are the script's, published
//! through `deckard_surface::OsoPage`, and `jam_strip` draws them. This module keeps no
//! state; the only thing it remembers between frames is the settings' saved offset.
//!
//! Lifetimes: the script owns the page's, and the window follows it exactly — no page, no
//! viewport. There is no mode, and no "shown" flag to drift from the script's own.

use crate::jam_strip::{self, Row};
use deckard_surface::OsoPage;
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

/// The window's own bar: the page's name and the counter at the left, and no mark at the right.
/// reference has no window chrome above it, and no buttons.
pub const BAR_H: f32 = 28.0;
/// How much of the window stays on screen whatever was saved: this many points of its
/// width, and (on the y axis) its bar.
const KEEP: f32 = 40.0;
/// The bar's text inset.
const PAD: f32 = 8.0;

/// The window's id: one strip. `from_hash_of`, as the splash window's.
pub fn id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("deckard-jam-strip")
}

/// The window's inner size for a strip `width` points wide: the bar, then the body's rows.
pub fn window_size(width: f32) -> Vec2 {
    Vec2::new(width, jam_strip::BODY_H + BAR_H)
}

/// Where the window goes: `saved` is an offset from the main window's top-left (so the strip
/// comes back beside the app on whatever display the app opens on), clamped so it cannot be
/// lost outside that display. With nothing saved it centres over the main window, as the
/// reference centres over its parent from live bounds.
pub fn position(saved: Option<Vec2>, main: Rect, size: Vec2, monitor: Vec2) -> Pos2 {
    let offset = match saved {
        Some(off) => Vec2::new(clamp_axis(off.x, size.x, monitor.x), clamp_axis(off.y, size.y, monitor.y)),
        None => Vec2::new((main.width() - size.x) / 2.0, (main.height() - size.y) / 2.0),
    };
    main.min + offset
}

/// One axis of the band a saved offset has to sit in: however far the strip was dragged on a
/// bigger desktop, `keep` points of it stay on this one.
fn clamp_axis(offset: f32, len: f32, monitor: f32) -> f32 {
    let keep = KEEP.min(len).min(monitor);
    offset.clamp(-(monitor - keep), monitor - keep)
}

/// Where the window ended up, as the offset from the main window's top-left: what to save.
pub fn offset_of(placed: Rect, main: Rect) -> Vec2 {
    placed.min - main.min
}

/// Open, update or close the strip's window and return the offset worth saving.
///
/// `page` is the strip to draw — `None` closes the window, which is how the strip's own
/// lifetime (the script's idle timer, its static page, a page change) reaches the window
/// without this module knowing any of those rules.
pub fn show(ctx: &egui::Context, page: Option<(&OsoPage, &[Row])>, saved: Option<Vec2>) -> Option<Vec2> {
    let Some((page, rows)) = page else { return None };
    // A strip is a bar: its width comes from the app's window, as the reference's does, not
    // from how many slots the page happens to hold.
    let (main, monitor) = ctx.input_for(egui::ViewportId::ROOT, |i| (i.viewport().outer_rect, i.viewport().monitor_size));
    // (Headless or first frame: no main window yet, so the strip has nothing to sit beside.)
    let main = main.unwrap_or(Rect::from_min_size(Pos2::ZERO, Vec2::new(1500.0, 930.0)));
    let monitor = monitor.unwrap_or(main.size());
    let width = jam_strip::width_for(main.width());
    let size = window_size(width);
    let at = position(saved, main, size, monitor);
    let builder = egui::ViewportBuilder::default()
        .with_title(jam_strip::title(page))
        .with_inner_size(size)
        .with_position(at)
        .with_decorations(false)
        .with_resizable(false)
        // (Shown, never focused: the strip must not take the keyboard from the app.)
        .with_active(false)
        .with_taskbar(false)
        .with_window_level(egui::WindowLevel::AlwaysOnTop);
    let (name, counter) = (jam_strip::name(page), jam_strip::counter(page));
    let trace = std::env::var_os("DECKARD_JAM_TRACE").is_some();
    ctx.show_viewport_immediate(id(), builder, |ui, class| {
        if trace {
            // (A native viewport is a window of its own; `EmbeddedWindow` would mean the
            // strip is being drawn as a panel inside DECKARD, which is the thing it replaced.)
            let kind = if class == egui::ViewportClass::EmbeddedWindow { "EMBEDDED (a panel in the app)" } else { "native window" };
            eprintln!("jam strip: {kind} at {at:?} size {size:?}");
        }
        paint(ui, &name, counter.as_deref(), rows, width, true);
    });
    // What the window (or the window manager) made of that place. With mouse passthrough on
    // the window cannot be dragged, so a difference here is the manager's placement, or a
    // platform that ignores passthrough — either way it is where the user now has it.
    let placed = ctx.input_for(id(), |i| i.viewport().outer_rect)?;
    let found = offset_of(placed, main);
    ((found - (at - main.min)).length() > 1.0).then_some(found)
}

/// The window's contents: its bar, then the published page's slots.
///
/// `draggable` is the real window's: the bar is what moves it, as a title bar does — the
/// reference's overlay is moved around the screen, and its body still belongs to the encoder
/// (no click target, no hover, nothing to hit).
pub(crate) fn paint(ui: &mut egui::Ui, name: &str, counter: Option<&str>, rows: &[Row], width: f32, draggable: bool) {
    let sense = if draggable { egui::Sense::click_and_drag() } else { egui::Sense::hover() };
    let (bar, grab) = ui.allocate_exact_size(Vec2::new(width, BAR_H), sense);
    if draggable && grab.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    let painter = ui.painter();
    painter.rect_filled(bar, 0.0, Color32::from_rgb(0x44, 0x44, 0x44));
    painter.hline(bar.x_range(), bar.bottom() - 0.5, egui::Stroke::new(1.0, Color32::from_rgb(0x5a, 0x5a, 0x5a)));
    let at = bar.left_center() + Vec2::new(PAD, 0.0);
    let font = egui::FontId::proportional(13.0);
    painter.text(at, egui::Align2::LEFT_CENTER, name, font.clone(), Color32::from_rgb(0xd8, 0xd8, 0xd8));
    if let Some(counter) = counter {
        // (The `n/N` counter lives in the bar, not in the strip's body, and sits just after the
        // page's name — the reference's title reads `Scale  1/2`, not a right-aligned count.)
        let name_w = painter.layout_no_wrap(name.to_owned(), font, Color32::WHITE).size().x;
        painter.text(
            at + Vec2::new(name_w + PAD + 4.0, 0.0),
            egui::Align2::LEFT_CENTER,
            counter,
            egui::FontId::proportional(12.0),
            Color32::from_gray(150),
        );
    }
    jam_strip::slots(ui, rows, width);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main() -> Rect {
        Rect::from_min_size(Pos2::new(200.0, 100.0), Vec2::new(1200.0, 800.0))
    }

    #[test]
    fn the_window_is_the_strip_and_one_bar_taller() {
        // The width is the bar's, not a sum of cells: a page of one slot and a page of eight are
        // the same window, as the reference's Tempo and Grid pages are.
        assert_eq!(window_size(1170.0), Vec2::new(1170.0, jam_strip::BODY_H + BAR_H));
        assert_eq!(window_size(560.0).y, window_size(1170.0).y, "the height is the rows', not the width's");
        assert_eq!(jam_strip::width_for(1500.0), 1170.0, "most of the app's width");
        assert_eq!(jam_strip::width_for(600.0), 880.0, "and never narrower than eight readable columns");
    }

    #[test]
    fn nothing_saved_centres_it_over_the_main_window() {
        let size = window_size(1170.0);
        let at = position(None, main(), size, Vec2::new(1440.0, 900.0));
        assert!((at.x - (main().left() + (main().width() - size.x) / 2.0)).abs() < 0.01);
        assert!((at.y - (main().top() + (main().height() - size.y) / 2.0)).abs() < 0.01);
    }

    #[test]
    fn a_saved_offset_comes_back_with_the_app() {
        let size = window_size(1170.0);
        let saved = Some(Vec2::new(-40.0, -12.0));
        let at = position(saved, main(), size, Vec2::new(1440.0, 900.0));
        assert_eq!(at, main().min + Vec2::new(-40.0, -12.0));
    }

    #[test]
    fn an_offset_from_a_bigger_desktop_still_lands_within_a_display_of_the_app() {
        // Saved beside a 4K display's right edge, opened on a laptop. egui reports a
        // viewport's size, never a monitor's origin, so the band is measured from the app:
        // however far it was dragged, the strip stays within one display of it and `KEEP`
        // points of it are reachable.
        let size = window_size(1170.0);
        let monitor = Vec2::new(1440.0, 900.0);
        let at = position(Some(Vec2::new(3800.0, 12.0)), main(), size, monitor);
        assert!(at.x + KEEP <= main().left() + monitor.x + 0.01, "at most a display right of the app");
        assert!(at.x >= main().left() - monitor.x, "and at most a display left of it");
        // Downwards too: its bar stays within a display whatever was saved.
        let below = position(Some(Vec2::new(0.0, 2400.0)), main(), size, monitor);
        assert!(below.y + BAR_H <= main().top() + monitor.y + 0.01, "its bar stays on this display");
        assert!(below.y >= main().top() - monitor.y);
    }

    #[test]
    fn the_offset_saved_is_the_one_the_window_is_at() {
        let placed = Rect::from_min_size(main().min + Vec2::new(30.0, 44.0), Vec2::new(420.0, 74.0));
        assert_eq!(offset_of(placed, main()), Vec2::new(30.0, 44.0));
        // A window nobody moved reports the offset that was asked for: nothing to save.
        let asked = position(Some(Vec2::new(30.0, 44.0)), main(), placed.size(), Vec2::new(1440.0, 900.0));
        assert_eq!(offset_of(Rect::from_min_size(asked, placed.size()), main()), Vec2::new(30.0, 44.0));
    }
}
