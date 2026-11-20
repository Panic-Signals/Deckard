//! The Jam's strip as its own window: a second eframe viewport, chromeless, mouse-inert,
//! always on top, drawn from the same published page `jam_strip` renders.
//!
//! Why a window and not a docked row (`docs/epics/jam-support/work-items/22-jam-overlay-
//! detached-window.md`): the reference's strip floats over whatever view is showing, and
//! because it is a window it needs no rule to avoid the mouse — where a docked strip had to
//! actively ignore clicks. Both properties come free here.
//!
//! What this module owns: the window's chrome (its bar: the page's name at the left, the
//! `n/N` counter and the mark at the right), its geometry, and where it goes. What it must
//! not own: the page, the cursor, or the edit flag — those are the script's, published
//! through `deckard_surface::OsoPage`, and `jam_strip` draws them. This module keeps no
//! state; the only thing it remembers between frames is the settings' saved offset.
//!
//! Lifetimes: the script owns the page's, and the window follows it exactly — no page, no
//! viewport. There is no mode, and no "shown" flag to drift from the script's own.

use crate::jam_strip::{self, Row};
use deckard_surface::OsoPage;
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

/// The window's own bar: the page's name left, the counter and the mark right. The
/// reference has no window chrome above it, and no buttons.
pub const BAR_H: f32 = 18.0;
/// How much of the window stays on screen whatever was saved: this many points of its
/// width, and (on the y axis) its bar.
const KEEP: f32 = 40.0;
/// The bar's text inset, and the mark's size.
const PAD: f32 = 6.0;
const MARK: f32 = 9.0;

/// The window's id: one strip. `from_hash_of`, as the splash window's.
pub fn id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("deckard-jam-strip")
}

/// The window's inner size for a page of `slots` slots: the strip's own size plus its bar.
pub fn window_size(slots: usize) -> Vec2 {
    let strip = jam_strip::strip_size(slots);
    Vec2::new(strip.x, strip.y + BAR_H)
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
    let slots = page.slots.len();
    let size = window_size(slots);
    let (main, monitor) = ctx.input_for(egui::ViewportId::ROOT, |i| (i.viewport().outer_rect, i.viewport().monitor_size));
    // (Headless or first frame: no main window yet, so the strip has nothing to sit beside.)
    let main = main.unwrap_or(Rect::from_min_size(Pos2::ZERO, size));
    let monitor = monitor.unwrap_or(size);
    let at = position(saved, main, size, monitor);
    let builder = egui::ViewportBuilder::default()
        .with_title(jam_strip::title(page))
        .with_inner_size(size)
        .with_position(at)
        .with_decorations(false)
        .with_mouse_passthrough(true)
        .with_resizable(false)
        // (Shown, never focused: the strip must not take the keyboard from the app.)
        .with_active(false)
        .with_taskbar(false)
        .with_window_level(egui::WindowLevel::AlwaysOnTop);
    let (name, counter) = (jam_strip::name(page), jam_strip::counter(page));
    ctx.show_viewport_immediate(id(), builder, |ui, _class| {
        paint(ui, &name, counter.as_deref(), slots, rows);
    });
    // What the window (or the window manager) made of that place. With mouse passthrough on
    // the window cannot be dragged, so a difference here is the manager's placement, or a
    // platform that ignores passthrough — either way it is where the user now has it.
    let placed = ctx.input_for(id(), |i| i.viewport().outer_rect)?;
    let found = offset_of(placed, main);
    ((found - (at - main.min)).length() > 1.0).then_some(found)
}

/// The window's contents: its bar, then the published page's slots.
fn paint(ui: &mut egui::Ui, name: &str, counter: Option<&str>, slots: usize, rows: &[Row]) {
    let width = jam_strip::strip_size(slots).x;
    let (bar, _) = ui.allocate_exact_size(Vec2::new(width, BAR_H), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(bar, 0.0, Color32::from_gray(28));
    painter.hline(bar.x_range(), bar.bottom() - 0.5, egui::Stroke::new(1.0, Color32::from_gray(60)));
    painter.text(
        bar.left_center() + Vec2::new(PAD, 0.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(11.0),
        Color32::from_gray(205),
    );
    let mark = Rect::from_center_size(bar.right_center() - Vec2::new(PAD + MARK / 2.0, 0.0), Vec2::splat(MARK * 2.0));
    mark_ni(painter, mark);
    if let Some(counter) = counter {
        // (The `n/N` counter lives in the bar, not in the strip's body: the reference's
        // window carries it in its title.)
        painter.text(
            Pos2::new(mark.left() - PAD, bar.center().y),
            egui::Align2::RIGHT_CENTER,
            counter,
            egui::FontId::proportional(10.0),
            Color32::from_gray(150),
        );
    }
    jam_strip::slots(ui, slots, rows);
}

/// The mark at the bar's right edge, drawn from primitives: no NI artwork ships with DECKARD
/// (the clean-room boundary, `docs/FEASIBILITY.md`).
fn mark_ni(painter: &egui::Painter, at: Rect) {
    painter.rect_stroke(at, 2.0, egui::Stroke::new(1.0, Color32::from_gray(150)), egui::StrokeKind::Inside);
    painter.text(at.center(), egui::Align2::CENTER_CENTER, "NI", egui::FontId::proportional(8.0), Color32::from_gray(150));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main() -> Rect {
        Rect::from_min_size(Pos2::new(200.0, 100.0), Vec2::new(1200.0, 800.0))
    }

    #[test]
    fn the_window_is_as_wide_as_its_slots_and_one_bar_taller() {
        let strip = jam_strip::strip_size(8);
        assert_eq!(window_size(8), Vec2::new(strip.x, strip.y + BAR_H));
        assert!(window_size(9).x > window_size(8).x, "a wider page is a wider window");
    }

    #[test]
    fn nothing_saved_centres_it_over_the_main_window() {
        let size = window_size(8);
        let at = position(None, main(), size, Vec2::new(1440.0, 900.0));
        assert!((at.x - (main().left() + (main().width() - size.x) / 2.0)).abs() < 0.01);
        assert!((at.y - (main().top() + (main().height() - size.y) / 2.0)).abs() < 0.01);
    }

    #[test]
    fn a_saved_offset_comes_back_with_the_app() {
        let size = window_size(8);
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
        let size = window_size(8);
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
