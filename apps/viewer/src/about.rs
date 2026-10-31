//! Help ▸ About DECKARD (docs/superpowers/specs/2026-10-31-about-and-updates-design.md):
//! a window with a looping GIF behind a card of frosted glass that carries the
//! version, who made DECKARD, the contributors (from GitHub, with their pictures),
//! whether a newer release exists (and the button that installs it), and the ways to
//! sponsor the work.
//!
//! The glass is real blur: each GIF frame is blurred once when decoded, and the card
//! shows the blurred frame where the window shows the sharp one.

use crate::App;
use crate::github::Contributor;
use eframe::egui::{
    self, Align2, Color32, ColorImage, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2,
};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

const GIF: &[u8] = include_bytes!("../assets/about.gif");
const SIZE: Vec2 = Vec2::new(560.0, 316.0);
const INSET: f32 = 24.0;
const CARD_RADIUS: f32 = 14.0;
/// Blur radius in GIF pixels (the GIF is 220 wide, shown at 560: 6 px here is 15 on
/// screen).
const BLUR: usize = 6;

pub const AUTHOR_URL: &str = "https://github.com/summitt";
pub const PROJECTS_URL: &str = "https://github.com/Panic-Signals";
pub const PATREON_URL: &str = "https://www.patreon.com/c/null0perat0r";
pub const GITHUB_SPONSORS_URL: &str = "https://github.com/sponsors/panic-signals";

/// One frame of the background: the picture, its blurred copy, and how long it shows.
pub struct Frame {
    pub sharp: ColorImage,
    pub blurred: ColorImage,
    pub delay: Duration,
}

/// Decode an animated GIF into full frames (the decoder composes partial frames).
pub fn decode_gif(bytes: &[u8]) -> Result<Vec<Frame>, String> {
    use image::AnimationDecoder;
    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let frames = decoder.into_frames().collect_frames().map_err(|e| e.to_string())?;
    if frames.is_empty() {
        return Err("no frames".into());
    }
    Ok(frames
        .into_iter()
        .map(|f| {
            let (num, den) = f.delay().numer_denom_ms();
            // GIFs with a zero delay are shown at the usual 10 frames a second.
            let ms = if num == 0 || den == 0 { 100 } else { (num / den).max(20) };
            let buf = f.into_buffer();
            let (w, h) = (buf.width() as usize, buf.height() as usize);
            let sharp = ColorImage::from_rgba_unmultiplied([w, h], buf.as_raw());
            let blurred = blur(&sharp, BLUR);
            Frame { sharp, blurred, delay: Duration::from_millis(u64::from(ms)) }
        })
        .collect())
}

/// A box blur of `radius`, two passes (across, then down), edges clamped. Alpha stays
/// opaque: the background shows no transparency.
pub fn blur(img: &ColorImage, radius: usize) -> ColorImage {
    let [w, h] = img.size;
    if radius == 0 || w == 0 || h == 0 {
        return img.clone();
    }
    let px = &img.pixels;
    let mut tmp = vec![[0u32; 3]; w * h];
    let n = (2 * radius + 1) as u32;
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 3];
            for dx in -(radius as isize)..=(radius as isize) {
                let sx = (x as isize + dx).clamp(0, w as isize - 1) as usize;
                let p = px[y * w + sx];
                acc[0] += u32::from(p.r());
                acc[1] += u32::from(p.g());
                acc[2] += u32::from(p.b());
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut out = vec![Color32::BLACK; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 3];
            for dy in -(radius as isize)..=(radius as isize) {
                let sy = (y as isize + dy).clamp(0, h as isize - 1) as usize;
                let t = tmp[sy * w + x];
                acc[0] += t[0];
                acc[1] += t[1];
                acc[2] += t[2];
            }
            let d = n * n;
            out[y * w + x] = Color32::from_rgb((acc[0] / d) as u8, (acc[1] / d) as u8, (acc[2] / d) as u8);
        }
    }
    ColorImage { size: [w, h], source_size: img.source_size, pixels: out }
}

/// The window's state while it is open.
pub struct State {
    frames: Vec<Frame>,
    /// Per frame, the sharp and the blurred texture, made on first paint.
    textures: Vec<(TextureHandle, TextureHandle)>,
    opened: Instant,
    contributors: Option<Result<Vec<Contributor>, String>>,
    contributors_rx: Receiver<Result<Vec<Contributor>, String>>,
    avatars: HashMap<String, TextureHandle>,
    avatars_rx: Receiver<(String, Vec<u8>)>,
    licences: bool,
}

impl State {
    /// Open: decode the GIF and ask GitHub for the contributors (and their pictures).
    pub fn open() -> State {
        let (ctx_tx, contributors_rx) = channel();
        let (av_tx, avatars_rx) = channel();
        std::thread::Builder::new()
            .name("about".into())
            .spawn(move || {
                let list = crate::github::contributors();
                let people = list.clone().unwrap_or_default();
                let _ = ctx_tx.send(list);
                for c in people {
                    let url = if c.avatar_url.contains('?') { format!("{}&s=56", c.avatar_url) } else { format!("{}?s=56", c.avatar_url) };
                    if let Ok(bytes) = crate::github::fetch(&url, 1 << 20) {
                        let _ = av_tx.send((c.login, bytes));
                    }
                }
            })
            .ok();
        State {
            frames: decode_gif(GIF).unwrap_or_default(),
            textures: Vec::new(),
            opened: Instant::now(),
            contributors: None,
            contributors_rx,
            avatars: HashMap::new(),
            avatars_rx,
            licences: false,
        }
    }

    /// An empty state (what's left behind when the open one is put back).
    fn closed() -> State {
        let (_, contributors_rx) = channel();
        let (_, avatars_rx) = channel();
        State {
            frames: Vec::new(),
            textures: Vec::new(),
            opened: Instant::now(),
            contributors: None,
            contributors_rx,
            avatars: HashMap::new(),
            avatars_rx,
            licences: false,
        }
    }

    /// The frame to show now, and how long until the next.
    fn current(&self) -> (usize, Duration) {
        if self.frames.is_empty() {
            return (0, Duration::from_secs(1));
        }
        let total: Duration = self.frames.iter().map(|f| f.delay).sum();
        let mut t = Duration::from_nanos((self.opened.elapsed().as_nanos() % total.as_nanos().max(1)) as u64);
        for (i, f) in self.frames.iter().enumerate() {
            if t < f.delay {
                return (i, f.delay - t);
            }
            t -= f.delay;
        }
        (0, self.frames[0].delay)
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Ok(r) = self.contributors_rx.try_recv() {
            self.contributors = Some(r);
        }
        while let Ok((login, bytes)) = self.avatars_rx.try_recv() {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let rgba = img.to_rgba8();
                let ci = ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                self.avatars.insert(login.clone(), ctx.load_texture(format!("avatar-{login}"), ci, TextureOptions::LINEAR));
            }
        }
    }
}

/// Draw the window; false once it is closed.
pub fn show(app: &mut App, ctx: &egui::Context) -> bool {
    // Taken out while drawn, so the card can also reach the app's update state.
    let Some(mut state) = app.about.take() else { return false };
    state.poll(ctx);
    if state.textures.is_empty() && !state.frames.is_empty() {
        state.textures = state
            .frames
            .iter()
            .enumerate()
            .map(|(i, f)| {
                (
                    ctx.load_texture(format!("about-{i}"), f.sharp.clone(), TextureOptions::LINEAR),
                    ctx.load_texture(format!("about-blur-{i}"), f.blurred.clone(), TextureOptions::LINEAR),
                )
            })
            .collect();
    }
    let (frame, next) = state.current();
    if !state.frames.is_empty() {
        ctx.request_repaint_after(next);
    }
    let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let state = &mut state;
    let update = &mut app.update;
    egui::Window::new("about")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .fixed_size(SIZE)
        .frame(egui::Frame::NONE)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(SIZE, egui::Sense::hover());
            let card = rect.shrink(INSET);
            let painter = ui.painter();
            // The background, and the glass: the blurred frame where the card is.
            let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            if let Some((sharp, blurred)) = state.textures.get(frame) {
                painter.add(egui::epaint::RectShape::new(rect, 10.0, Color32::WHITE, Stroke::NONE, StrokeKind::Inside).with_texture(sharp.id(), full));
                let uv = Rect::from_min_max(
                    Pos2::new(INSET / SIZE.x, INSET / SIZE.y),
                    Pos2::new(1.0 - INSET / SIZE.x, 1.0 - INSET / SIZE.y),
                );
                painter.add(
                    egui::epaint::RectShape::new(card, CARD_RADIUS, Color32::WHITE, Stroke::NONE, StrokeKind::Inside).with_texture(blurred.id(), uv),
                );
            } else {
                painter.rect_filled(rect, 10.0, Color32::from_rgb(0x10, 0x12, 0x1a));
            }
            painter.rect_filled(card, CARD_RADIUS, Color32::from_white_alpha(26));
            painter.rect_stroke(card, CARD_RADIUS, Stroke::new(1.0, Color32::from_white_alpha(60)), StrokeKind::Inside);
            // The lit edges: bright top and left, dark bottom and right.
            let r = CARD_RADIUS;
            painter.line_segment([Pos2::new(card.left() + r, card.top() + 0.5), Pos2::new(card.right() - r, card.top() + 0.5)], Stroke::new(1.0, Color32::from_white_alpha(140)));
            painter.line_segment([Pos2::new(card.left() + 0.5, card.top() + r), Pos2::new(card.left() + 0.5, card.bottom() - r)], Stroke::new(1.0, Color32::from_white_alpha(110)));
            painter.line_segment([Pos2::new(card.left() + r, card.bottom() - 0.5), Pos2::new(card.right() - r, card.bottom() - 0.5)], Stroke::new(1.0, Color32::from_black_alpha(90)));
            painter.line_segment([Pos2::new(card.right() - 0.5, card.top() + r), Pos2::new(card.right() - 0.5, card.bottom() - r)], Stroke::new(1.0, Color32::from_black_alpha(90)));
            // The close cross, top right of the window.
            let x = Rect::from_center_size(Pos2::new(rect.right() - 16.0, rect.top() + 16.0), Vec2::splat(20.0));
            let close = ui.interact(x, ui.id().with("close"), egui::Sense::click());
            let c = if close.hovered() { Color32::WHITE } else { Color32::from_white_alpha(170) };
            painter.line_segment([x.center() + Vec2::new(-5.0, -5.0), x.center() + Vec2::new(5.0, 5.0)], Stroke::new(1.5, c));
            painter.line_segment([x.center() + Vec2::new(-5.0, 5.0), x.center() + Vec2::new(5.0, -5.0)], Stroke::new(1.5, c));
            if close.clicked() {
                open = false;
            }
            // The content.
            let inner = card.shrink2(Vec2::new(18.0, 14.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                let v = ui.visuals_mut();
                v.override_text_color = Some(Color32::WHITE);
                v.hyperlink_color = Color32::from_rgb(0xbf, 0xdc, 0xff);
                v.widgets.inactive.weak_bg_fill = Color32::from_white_alpha(28);
                v.widgets.hovered.weak_bg_fill = Color32::from_white_alpha(60);
                v.widgets.active.weak_bg_fill = Color32::from_white_alpha(90);
                v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(70));
                v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(140));
                v.selection.bg_fill = Color32::from_white_alpha(40);
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                let logo_at = ui.cursor().left_top() + Vec2::new(0.0, 14.0);
                let logo = crate::layout::deckard_logo(ui.painter(), logo_at);
                ui.painter().text(
                    Pos2::new(logo.right() + 12.0, logo.center().y + 1.0),
                    Align2::LEFT_CENTER,
                    crate::VERSION,
                    FontId::proportional(13.0),
                    Color32::from_white_alpha(200),
                );
                ui.add_space(30.0);
                ui.horizontal(|ui| {
                    ui.label("Created by");
                    ui.hyperlink_to("Josh Summitt", AUTHOR_URL).on_hover_text(AUTHOR_URL);
                    ui.label("·");
                    ui.hyperlink_to("More from Panic-Signals", PROJECTS_URL).on_hover_text(PROJECTS_URL);
                });
                contributors_row(state, ui);
                ui.add_space(4.0);
                crate::update::line(update, ui);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("Support DECKARD:");
                    if ui.button("Splice").on_hover_text(crate::splice::DISCLOSURE).clicked() {
                        crate::splice::open(ui.ctx(), None);
                    }
                    if ui.button("Patreon").on_hover_text(PATREON_URL).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(PATREON_URL));
                    }
                    if ui.button("GitHub Sponsors").on_hover_text(GITHUB_SPONSORS_URL).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(GITHUB_SPONSORS_URL));
                    }
                });
                ui.add_space(6.0);
                let licences = egui::CollapsingHeader::new(egui::RichText::new("Licences and trademarks").small())
                    .id_salt("about-licences")
                    .open(Some(state.licences))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(70.0).show(ui, |ui| {
                            ui.label(egui::RichText::new("An independent, open music workstation that opens and plays Maschine projects. Not affiliated with Native Instruments. MASCHINE is a trademark of Native Instruments.").small());
                            ui.label(egui::RichText::new("Plaits is Mutable Instruments' code by Émilie Gillet (MIT License).").small());
                            ui.label(egui::RichText::new(deckard_link::LICENSE).small());
                            ui.label(egui::RichText::new(deckard_plugins::MUTABLE_INSTRUMENTS_LICENSE).small());
                        });
                    });
                if licences.header_response.clicked() {
                    state.licences = !state.licences;
                }
            });
        });
    if open {
        app.about = Some(std::mem::replace(state, State::closed()));
    }
    open
}

fn contributors_row(state: &State, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label("Contributors:");
        match &state.contributors {
            None => {
                ui.label(egui::RichText::new("…").weak());
            }
            Some(Err(_)) => {
                ui.label(egui::RichText::new("offline").weak());
            }
            Some(Ok(list)) if list.is_empty() => {
                ui.label(egui::RichText::new("none yet").weak());
            }
            Some(Ok(list)) => {
                for c in list.iter().take(16) {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::click());
                    let hover = format!("{} · {} commit{}", c.login, c.contributions, if c.contributions == 1 { "" } else { "s" });
                    let resp = resp.on_hover_text(hover);
                    match state.avatars.get(&c.login) {
                        Some(tex) => {
                            let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                            ui.painter().add(
                                egui::epaint::RectShape::new(r, CornerRadius::same(14), Color32::WHITE, Stroke::NONE, StrokeKind::Inside)
                                    .with_texture(tex.id(), full),
                            );
                        }
                        None => {
                            ui.painter().circle_filled(r.center(), 14.0, Color32::from_white_alpha(50));
                            let initial = c.login.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, initial, FontId::proportional(13.0), Color32::WHITE);
                        }
                    }
                    if resp.hovered() {
                        ui.painter().circle_stroke(r.center(), 14.0, Stroke::new(1.5, Color32::WHITE));
                    }
                    if resp.clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(c.url.clone()));
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The GIF that ships decodes to full frames with their delays, and the blur
    /// keeps the size and only mixes neighbours.
    #[test]
    fn the_background_decodes_and_blurs() {
        let frames = decode_gif(GIF).unwrap();
        assert!(frames.len() > 1, "{} frames", frames.len());
        let f = &frames[0];
        assert_eq!(f.sharp.size, [220, 129]);
        assert_eq!(f.blurred.size, f.sharp.size);
        assert!(f.delay >= Duration::from_millis(20));

        let mut img = ColorImage::filled([5, 1], Color32::BLACK);
        img.pixels[2] = Color32::WHITE;
        let b = blur(&img, 1);
        assert_eq!(b.size, [5, 1]);
        assert_eq!(b.pixels[0], Color32::BLACK);
        assert_eq!(b.pixels[2].r(), 85);
        assert_eq!(b.pixels[1].r(), 85);
        assert_eq!(b.pixels[4], Color32::BLACK);
        assert!(decode_gif(b"not a gif").is_err());
    }

    /// A picture of the window, with an update on offer (run by hand:
    /// `DECKARD_PICTURE=about.png cargo test -p deckard -- --ignored about_picture`).
    #[test]
    #[ignore]
    fn about_picture() {
        let out = std::env::var("DECKARD_PICTURE").unwrap_or_else(|_| "about.png".into());
        let Some(mut h) = crate::tests::harness(false, false) else { return };
        h.state_mut().about = Some(State::open());
        h.state_mut().update.check = crate::update::Check::Available(crate::github::Release {
            version: (0, 2, 0),
            tag: "0.2.0".into(),
            url: "https://github.com/Panic-Signals/Deckard/releases/tag/0.2.0".into(),
            assets: Vec::new(),
        });
        for _ in 0..40 {
            h.step();
            std::thread::sleep(Duration::from_millis(50));
        }
        h.render().unwrap().save(out).unwrap();
    }
}
