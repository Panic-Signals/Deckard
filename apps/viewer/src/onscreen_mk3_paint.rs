//! The on-screen MK3's painter: [`Host::ui`](crate::onscreen_mk3::Host) draws the table
//! docked in the main window.
//!
//! The geometry comes from [`crate::mk3_layout`], the lights from
//! [`crate::mk3_lights::color`] — this module owns no palette beyond the fork's
//! own material tokens — and pointer contact is forwarded to
//! [`Panel`](crate::onscreen_mk3::Panel)'s methods with table coordinates. It
//! holds no gesture logic of its own: a behaviour question belongs in the core,
//! with a test, not in a line of painter.
//!
//! The material is the jamulator fork's (`~/jamini-jamulator/css/jamulator.scss`
//! `#controller-mk3`, `index.html` lines 162-340): 1 px corners, a 2 px drop
//! shadow, an inset top highlight, condensed uppercase micro-type on silicon
//! buttons, dark translucent rgb caps on pads/mutes/groups, no text on pads,
//! knobs or the jog, no cap at all on the wheel's press (the jog's disc is what
//! you see there), a display bezel behind the screens, four detent dots
//! around the jog, and the strip's dot row over its placeholder bar. egui
//! offers no radial gradients and no condensed face, so the highlight is two
//! nested rounded rects (or nested discs, for the encoders) and the type is
//! the closest proportional face — both deliberate approximations, said so here.
//! No new dependency; no asset image (the fork uses none either).

use crate::mk3_layout::{self, Kind};
use crate::mk3_lights;
use crate::onscreen_mk3::Host;
use eframe::egui;
use std::time::Instant;

/// The frame's screens: both displays side by side, one image, as [`crate::Mk3Out`] documents.
const SCREEN_W: usize = 960;
const SCREEN_H: usize = 272;

/// The surface behind the controls: the fork's `--bg-controller`.
const SURFACE: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x1f, 0x1f);
/// The display bezel the two screens sit in: the fork's `.mk3-display-box`.
const DISPLAY_BOX: egui::Color32 = egui::Color32::from_rgb(0x10, 0x10, 0x10);
/// An unlit silicon cap: the fork's `.jam-btn` radial base, dark end.
const SILICON_BASE: egui::Color32 = egui::Color32::from_rgb(0x7c, 0x77, 0x72);
/// An unlit rgb cap: the fork's `.btn-rgb:not(.lit)` linear base, dark end.
const RGB_BASE: egui::Color32 = egui::Color32::from_rgb(0x38, 0x37, 0x36);
/// Silkscreen ink, unlit and lit: the fork's `.jam-btn` keeps `#1a1a1a`, `.lit` `#111`.
const INK: egui::Color32 = egui::Color32::from_rgb(0x1a, 0x1a, 0x1a);
const INK_LIT: egui::Color32 = egui::Color32::from_rgb(0x11, 0x11, 0x11);
/// The encoder ring and face: the fork's `.encoder` border and radial stops.
const ENCODER_RING: egui::Color32 = egui::Color32::from_rgb(0x50, 0x50, 0x50);
const ENCODER_EDGE: egui::Color32 = egui::Color32::from_rgb(0x33, 0x33, 0x33);
const ENCODER_MID: egui::Color32 = egui::Color32::from_rgb(0x3c, 0x3c, 0x3c);
const ENCODER_TOP: egui::Color32 = egui::Color32::from_rgb(0x48, 0x48, 0x48);
const ENCODER_TICK: egui::Color32 = egui::Color32::from_rgb(0x88, 0x88, 0x88);

/// A table-space rect from the fork's percentages of the stage: decoration that
/// is not a control (no id) still cites the fork's selector and numbers.
fn deco(left: f32, top: f32, w: f32, h: f32) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(left * mk3_layout::WIDTH / 100.0, top * mk3_layout::HEIGHT / 100.0),
        egui::pos2((left + w) * mk3_layout::WIDTH / 100.0, (top + h) * mk3_layout::HEIGHT / 100.0),
    )
}

/// The display bezel: the fork's `.mk3-display-box` (23.059%, 1.882%, 74.048%, 30.391%).
fn display_box() -> egui::Rect {
    deco(23.059, 1.882, 74.048, 30.391)
}

/// The strip's dot row: the fork's `.mk3-strip-dots` (4.546%, 63.243%, 29.743%, 0.217%).
fn strip_dots() -> egui::Rect {
    deco(4.546, 63.243, 29.743, 0.217)
}

/// The jog's four detent dots: the fork's `.mk3-jdot.jd1-4`, each 0.198% x 0.217%.
fn jog_dots() -> [egui::Rect; 4] {
    let (w, h) = (0.198, 0.217);
    [(11.595, 43.535), (5.599, 50.036), (17.522, 50.036), (11.595, 56.694)].map(|(l, t)| deco(l, t, w, h))
}

/// The logo's top-left: the fork's `.mk3-logo` (left 3.096%, top 35.746%,
/// 2.5cqw type). The ring reads as the fork's circled mark; the type is the
/// closest proportional face, not condensed.
fn logo() -> egui::Pos2 {
    egui::pos2(3.096 * mk3_layout::WIDTH / 100.0, 35.746 * mk3_layout::HEIGHT / 100.0)
}

/// The left half of the screens' texture: `screen_left` shows the left display.
fn left_uv() -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(0.5, 1.0))
}

/// The right half: `screen_right` shows the right display.
fn right_uv() -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(0.5, 0.0), egui::pos2(1.0, 1.0))
}

/// Upload the frame's screens to the cached texture when the change count moved, and
/// report whether a texture was (re)built: the tests pin the caching, so a frame with
/// an unchanged count never re-uploads the 1 MB image.
pub(crate) fn refresh_screens(host: &mut Host, ctx: &egui::Context) -> bool {
    let Some((count, frame)) = host.last.as_ref().and_then(|out| out.screens.as_ref()) else {
        return false;
    };
    if host.screens.as_ref().is_some_and(|(n, _)| n == count) {
        return false;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([SCREEN_W, SCREEN_H], &frame.clone().into_pixels().concat());
    match &mut host.screens {
        Some((n, tex)) => {
            tex.set(image, egui::TextureOptions::NEAREST);
            *n = *count;
        }
        None => {
            host.screens = Some((*count, ctx.load_texture("mk3-screens", image, egui::TextureOptions::NEAREST)));
        }
    }
    true
}

/// Table coordinates of a pointer over the drawn surface: the surface rect's offset,
/// unscaled. The core hit-tests in table space, so the painter maps back into it.
pub(crate) fn table_point(rect: egui::Rect, scale: f32, p: egui::Pos2) -> egui::Pos2 {
    egui::pos2((p.x - rect.min.x) / scale, (p.y - rect.min.y) / scale)
}

/// The docked surface's scale for the space on offer: width and height both bind
/// it, so a window shorter than the surface's aspect shrinks the whole surface
/// instead of pushing the lower controls off-screen. The painter draws
/// top-aligned inside its column; what is left over letterboxes below.
pub(crate) fn docked_scale(avail: egui::Vec2, surface: egui::Vec2) -> f32 {
    (avail.x / surface.x).min(avail.y / surface.y)
}

/// The most of the window's width the docked surface may claim before the rest letterboxes
/// beside it: a narrow window keeps a body to work in.
const DOCKED_MAX_FRACTION: f32 = 0.45;
/// The narrowest and widest the user may drag the docked panel: narrower than this the
/// surface is a legend nobody can hit, wider and the arrangement has no room left.
const DOCKED_MIN_WIDTH: f32 = 320.0;
const DOCKED_MAX_WIDTH: f32 = 900.0;

/// The width the docked panel must ask for: the surface's width at the window's full
/// height, capped to [`DOCKED_MAX_FRACTION`] of the width on offer so a narrow window keeps a
/// This is the mirror of the height a bottom dock would ask for, and it fits this app better:
/// the window is wider than it is tall, and the stage's 1518 x 1382 is close to square, so a
/// right column can spend the height a bottom strip would waste on width the surface cannot
/// use. The caller declares the result to the panel (`exact_size`), never the reverse.
///
/// This exists because a side panel with no declared size takes egui's default for its side —
/// 200 px — and [`docked_scale`], which faithfully fits the surface to the space on offer,
/// dutifully draws a 200 px surface nobody can play. The bottom edge has the same failure with
/// a nastier default (the interact size, about 18 px), which is the sliver the first dock
/// shipped; the edge only changes the size of the mistake, not its shape. The width has to
/// reach the panel, and only this function knows the surface's aspect.
pub(crate) fn docked_width(avail: egui::Vec2, portrait: bool) -> f32 {
    let surface = crate::onscreen_mk3::surface_size(portrait);
    (avail.y * surface.x / surface.y).min(avail.x * DOCKED_MAX_FRACTION)
}
/// The docked panel itself: the one construction the app and its regression test share,
/// so the test pins the app's own call instead of a copy of it.
///
/// `width` is the width the user dragged the edge to, or the one the window implied when
/// the dock first opened: the panel is theirs from then on.
pub(crate) fn docked_panel(width: f32) -> egui::Panel {
    egui::Panel::right("mk3_panel").resizable(true).default_size(width).size_range(egui::Rangef::new(DOCKED_MIN_WIDTH, DOCKED_MAX_WIDTH))
}
/// One step of the down/move/up triple the core expects, from the pointer button's own
/// held-state.
///
/// The gesture APIs cannot answer this: `drag_started` only fires once the pointer has
/// moved, so a click — press and release in place, which is how every pad is tapped —
/// reported nothing and the on-screen MK3 ignored the mouse entirely. The held-state
/// turns true on the press frame and false on the release frame whatever the travel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Contact {
    Down,
    Move,
    Release,
    None,
}

pub(crate) fn contact_step(held: bool, was_down: bool) -> Contact {
    match (was_down, held) {
        (false, true) => Contact::Down,
        (true, true) => Contact::Move,
        (true, false) => Contact::Release,
        (false, false) => Contact::None,
    }
}

/// Channel-wise linear mix of two colours: the fork mixes LED into the base cap
/// with `color-mix`, which is perceptual; a straight lerp reads the same on a
/// 1 px cap, and the LED side always comes from [`mk3_lights`].
fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let m = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t).round().clamp(0.0, 255.0) as u8;
    egui::Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// A button or pad cap: drop shadow, base, inset top highlight — the fork's
/// `.jam-btn` material (1 px corners, 2 px drop shadow, inset top highlight).
/// Unlit the base is the cap's dark end; lit the LED owns the base (mixed from
/// the cap, as the fork's `.lit` does) with a glow behind it.
fn paint_cap(painter: &egui::Painter, r: egui::Rect, silicon: bool, lit: Option<egui::Color32>, scale: f32) {
    let rounding = 1.6 * scale;
    // The drop shadow first: 2 px down, half black (egui blurs nothing).
    painter.rect_filled(r.translate(egui::vec2(0.0, 2.0 * scale)), rounding, egui::Color32::from_black_alpha(110));
    let base = match (lit, silicon) {
        (Some(led), true) => mix(egui::Color32::from_rgb(0xd0, 0xcc, 0xc8), led, 0.75),
        (Some(led), false) => mix(egui::Color32::from_rgb(0x28, 0x26, 0x24), led, 0.75),
        (None, true) => SILICON_BASE,
        (None, false) => RGB_BASE,
    };
    if let Some(led) = lit {
        // The LED's glow behind the cap: the fork's `0 0 3px` lit shadow.
        painter.rect_filled(r.expand(2.0 * scale), rounding + 2.0 * scale, led.gamma_multiply(0.30));
    }
    painter.rect_filled(r, rounding, base);
    // The inset top highlight: light from above, like the fork's `inset 0 1px`.
    let hi = egui::Rect::from_min_max(r.min, egui::pos2(r.max.x, r.min.y + (1.6 * scale).max(r.height() * 0.14)));
    painter.rect_filled(hi, rounding, egui::Color32::from_white_alpha(if lit.is_some() { 76 } else { 24 }));
}

/// An encoder: drop shadow, dark ring, stepped face, tick at twelve o'clock —
/// the fork's `.encoder` (2 px `#505050` border, dark radial face, `#888` tick).
/// Knobs and the jog draw the same; the jog's four detent dots are decoration
/// drawn once around it, not part of the control.
fn paint_encoder(painter: &egui::Painter, r: egui::Rect, scale: f32) {
    let c = r.center();
    let radius = r.width().min(r.height()) / 2.0;
    // A shadow under the cap, not a crescent: the same radius drawn a hair lower shows up
    // as a black rim at the bottom and makes the ring read as if it were cut off, which is
    // exactly how "not vertically centred" looks.
    painter.circle_filled(c + egui::vec2(0.0, 1.0 * scale), (radius * 0.97).max(0.0), egui::Color32::from_black_alpha(90));
    // The fork's cap: a 2 px rim, then a radial face lit from the middle — centred, with no
    // vertical offset, so rim and face share one centre.
    painter.circle_filled(c, radius, ENCODER_RING);
    let face = (radius - 2.0 * scale).max(0.0);
    painter.circle_filled(c, face, ENCODER_EDGE);
    painter.circle_filled(c, face * 0.78, ENCODER_MID);
    painter.circle_filled(c, face * 0.55, ENCODER_TOP);
    // The fork's tick is a nub at the top of the cap (`.encoder::after`: top 2 px, 3 px
    // tall), not a needle floating above the centre.
    let w = (face * 0.10).max(1.0 * scale);
    let tall = (radius * 0.26).max(2.0 * scale);
    let top = c.y - radius + 2.0 * scale;
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(c.x - w / 2.0, top), egui::vec2(w, tall)), 1.0 * scale, ENCODER_TICK);
}

impl Host {
    /// Draw the whole surface scaled into the available rect, and forward pointer
    /// position to [`Panel`](crate::onscreen_mk3::Panel)'s methods. Closed, or with no
    /// frame yet, nothing is drawn.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.open || self.last.is_none() {
            return;
        }
        refresh_screens(self, ui.ctx());
        let now = Instant::now();
        // The presented surface: landscape it is the table itself, portrait the whole
        // surface turned, so the window's aspect follows the toggle. One transform for
        // the painter and the hit test, never a second layout.
        let portrait = self.portrait;
        let surface = crate::onscreen_mk3::surface_size(portrait);
        // Both dimensions bind the scale (see `docked_scale`): a short window fits
        // by height and letterboxes below, instead of losing the lower controls.
        let scale = docked_scale(ui.available_size(), surface);
        let size = egui::vec2(surface.x * scale, surface.y * scale);
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        // Touch contacts, each with its own identity: egui reports a `Touch` event per
        // finger *in addition to* the pointer the first touch drives, so while any
        // touch is live the pointer path below would double-report it — it stays out
        // of the way until every touch lifts.
        let contacts: Vec<(egui::TouchId, egui::TouchPhase, egui::Pos2, Option<f32>)> = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match *e {
                    egui::Event::Touch { id, phase, pos, force, .. } => Some((id, phase, pos, force)),
                    _ => None,
                })
                .collect()
        });
        if !contacts.is_empty() || !self.touches.is_empty() {
            self.forward_touches(rect, scale, portrait, &contacts);
        // Pointer contact, in table coordinates: press, drag and release pair exactly
        // as the core's down/move/up do, so a click is a down then an up.
        } else {
            // The button's held-state, not egui's drag gesture: a tap does not move, so
            // `drag_started` never fired for it and every click was swallowed.
            let held = resp.is_pointer_button_down_on();
            let at = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
            match contact_step(held, self.pointer_down) {
                Contact::Down => {
                    if let Some(p) = at {
                        self.panel.pointer_down(0, crate::onscreen_mk3::to_landscape(table_point(rect, scale, p), portrait), None);
                    }
                    self.pointer_down = true;
                }
                Contact::Move => {
                    if let Some(p) = at {
                        self.panel.pointer_move(0, crate::onscreen_mk3::to_landscape(table_point(rect, scale, p), portrait));
                    }
                }
                Contact::Release => {
                    self.panel.pointer_up(0);
                    self.pointer_down = false;
                }
                Contact::None => {}
            }
        }
        // A wheel tick over the surface is the jog's own event.
        if resp.hovered() {
            let ticks = ui.input(|i| i.smooth_scroll_delta.y);
            if ticks != 0.0 {
                self.panel.wheel(ticks.signum() as i32);
            }
        }
        // Paint, back-to-front like the table: the surface, the bezel, the dots,
        // the logo, the controls (screens take their texture halves, pads last).
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, SURFACE);
        let place = |t: egui::Rect| {
            let s = crate::onscreen_mk3::to_surface(t, portrait);
            egui::Rect::from_min_size(
                rect.min + egui::vec2(s.min.x * scale, s.min.y * scale),
                egui::vec2(s.width() * scale, s.height() * scale),
            )
        };
        // The display bezel behind the screens.
        painter.rect_filled(place(display_box()), 4.8 * scale, DISPLAY_BOX);
        // The strip's dot row over its bar-to-be: 21 dots, the fork's
        // `repeating-linear-gradient(90deg, #ddd 0 .664%, transparent .664% 4.762%)`.
        {
            let dots = place(strip_dots());
            let step = dots.width() * 0.04762;
            let w = dots.width() * 0.00664;
            let mut x = dots.min.x;
            while x + w <= dots.max.x + 1.0 {
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::pos2(x, dots.min.y), egui::vec2(w, dots.height())),
                    0.0,
                    egui::Color32::from_rgb(0xdd, 0xdd, 0xdd),
                );
                x += step;
            }
        }
        // The jog's four detent dots.
        for dot in jog_dots() {
            painter.rect_filled(place(dot), 0.0, egui::Color32::from_rgb(0xdd, 0xdd, 0xdd));
        }
        // The logo: ring plus MASCHINE silkscreen in the fork's spot.
        {
            let at = place(egui::Rect::from_min_size(logo(), egui::Vec2::ZERO));
            let r = 14.4 * scale;
            let c = at.min + egui::vec2(r, r);
            painter.circle_stroke(c, r, egui::Stroke::new((2.5 * scale).max(1.0), egui::Color32::from_rgb(0xf2, 0xf2, 0xf2)));
            painter.text(
                c + egui::vec2(r + 10.0 * scale, 0.0),
                egui::Align2::LEFT_CENTER,
                "MASCHINE",
                egui::FontId::proportional(38.0 * scale),
                egui::Color32::from_rgb(0xf2, 0xf2, 0xf2),
            );
        }
        let tex = self.screens.as_ref().map(|(_, t)| t.id());
        let leds = &self.last.as_ref().expect("last is Some: checked above").leds;
        // Resolved once per frame: the family exists only after the app's fonts are
        // installed, and asking per control would rebuild the family list 60 times.
        let label_family = crate::layout::mk3_family(ui.ctx());
        for control in mk3_layout::controls() {
            let r = place(control.rect);
            let lit = mk3_lights::color(leds, control.id, now).map(|[r, g, b]| egui::Color32::from_rgb(r, g, b));
            match control.kind {
                Kind::Screen => match tex {
                    Some(id) => {
                        painter.image(id, r, if control.id == "screen_left" { left_uv() } else { right_uv() }, egui::Color32::WHITE);
                    }
                    None => {
                        painter.rect_filled(r, 0.0, RGB_BASE);
                    }
                },
                // Pads carry no text in the fork: the cap and its light are the label.
                Kind::Pad => {
                    paint_cap(&painter, r, false, lit, scale);
                }
                Kind::Button => {
                    let cap = mk3_layout::cap(control.id);
                    if cap == mk3_layout::Cap::Bare {
                        // The wheel's press is a touch target on the jog: the disc under it is
                        // the chrome, so there is nothing to paint (nor any label to ink).
                        continue;
                    }
                    paint_cap(&painter, r, cap == mk3_layout::Cap::Silicon, lit, scale);
                    if !control.label.is_empty() {
                        // Bold Roboto at the print's weight: the fork's 600-weight
                        // condensed face is not in the tree, and egui's default is thin.
                        let font = egui::FontId::new(15.5 * scale, label_family.clone());
                        let ink = if lit.is_some() { INK_LIT } else { INK };
                        painter.text(r.center(), egui::Align2::CENTER_CENTER, control.label.to_uppercase(), font, ink);
                    }
                }
                Kind::Knob | Kind::Jog => {
                    // No LED names a knob or the jog, so there is no travelled position
                    // in the frame to draw: the encoder with its tick at twelve o'clock.
                    paint_encoder(&painter, r, scale);
                }
                Kind::Strip => {
                    // The fork's placeholder bar: dark middle, lighter 3% rails, 1 px
                    // `#222` border, and the 26 `LED_TS_nn` segments lit over it.
                    painter.rect_filled(r, 6.4 * scale, egui::Color32::from_rgb(0x16, 0x16, 0x16));
                    let rail = r.width() * 0.03;
                    for side in [r.min.x, r.max.x - rail] {
                        painter.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(side, r.min.y), egui::vec2(rail, r.height())),
                            6.4 * scale,
                            egui::Color32::from_rgb(0x44, 0x44, 0x44),
                        );
                    }
                    painter.rect_stroke(
                        r,
                        6.4 * scale,
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x22, 0x22, 0x22)),
                        egui::StrokeKind::Middle,
                    );
                    // The strip's own LEDs, one per segment: `LED_TS_01`..`LED_TS_26`. They
                    // are colour LEDs, so a lit segment is a palette byte and an unlit one
                    // paints nothing over the bar.
                    let seg = r.width() / crate::mk3_lights::STRIP_SEGMENTS as f32;
                    for i in 0..crate::mk3_lights::STRIP_SEGMENTS {
                        let Some([cr, cg, cb]) = crate::mk3_lights::strip_segment(leds, i, now) else {
                            continue;
                        };
                        let x = r.min.x + seg * i as f32 + seg * 0.12;
                        let wide = (seg * 0.76).max(1.0);
                        let tall = r.height() * 0.62;
                        painter.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(x, r.min.y + (r.height() - tall) / 2.0), egui::vec2(wide, tall)),
                            1.0 * scale,
                            egui::Color32::from_rgb(cr, cg, cb),
                        );
                    }
                }
            }
        }
    }
}
