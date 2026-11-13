//! The startup splash: a small teal CRT "terminal booting up" window that shows
//! startup's real progress, with links to the source, the community and the release
//! notes.

use crate::boot::{Boot, Outcome};
use crate::{VERSION, github, layout, update};
use eframe::egui::{self, Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Sense, Stroke, Vec2};

pub const GITHUB_URL: &str = "https://github.com/Panic-Signals/Deckard";
pub const DISCORD_URL: &str = "https://discord.gg/SGSpFaUnNh";

/// The window's size.
const SIZE: Vec2 = Vec2::new(760.0, 480.0);

const FG: Color32 = Color32::from_rgb(0x4f, 0xe0, 0xcf);
const DIM: Color32 = Color32::from_rgb(0x1f, 0x7a, 0x70);
const BG: Color32 = Color32::from_rgb(0x01, 0x17, 0x16);

/// The column the results start at, counted from the label's first letter.
const RESULT_COLUMN: usize = 45;
/// The bar's width in blocks.
const BAR_WIDTH: usize = 40;

/// The three buttons: accessible label, ASCII lettering, caption, link.
const GITHUB_ART: &str = r"  ___ ___ _____ _  _ _   _ ___
 / __|_ _|_   _| || | | | | _ )
| (_ || |  | | | __ | |_| | _ \
 \___|___| |_| |_||_|\___/|___/";
const DISCORD_ART: &str = r" ___ ___ ___  ___ ___  ___ ___
|   \_ _/ __|/ __/ _ \| _ \   \
| |) | |\__ \ (_| (_) |   / |) |
|___/___|___/\___\___/|_|_\___/";
const NOTES_ART: &str = r" _  _  ___ _____ ___ ___
| \| |/ _ \_   _| __/ __|
| .` | (_) || | | _|\__ \
|_|\_|\___/ |_| |___|___/";

/// The release notes of `version` when it is a release, else the latest release's.
pub fn release_notes_link(version: &str) -> String {
    if github::parse_version(version).is_some() {
        update::release_notes_url(version)
    } else {
        format!("https://github.com/{}/releases/latest", github::REPO)
    }
}

/// `[█████░░░░░]  50%`: `progress` (0 to 1) as `width` blocks and a percentage.
pub fn bar_text(progress: f32, width: usize) -> String {
    let p = progress.clamp(0.0, 1.0);
    let full = ((p * width as f32).round() as usize).min(width);
    format!("[{}{}] {:>3}%", "█".repeat(full), "░".repeat(width - full), (p * 100.0).round() as u32)
}

/// Where the splash goes: centred on the main window (`main`, its outer rect, on
/// whichever display it is), or on the monitor (of `monitor` size) while that isn't
/// known.
pub fn splash_position(main: Option<Rect>, monitor: Option<Vec2>) -> Option<Pos2> {
    match (main, monitor) {
        (Some(r), _) => Some(r.center() - SIZE / 2.0),
        (None, Some(m)) => Some(((m - SIZE) / 2.0).to_pos2()),
        (None, None) => None,
    }
}

/// Open or update the splash window (unless it was closed), centred on the main
/// window.
pub fn show(ctx: &egui::Context, boot: &mut Boot) {
    if boot.hidden {
        return;
    }
    let mut builder = egui::ViewportBuilder::default()
        .with_title("DECKARD")
        .with_inner_size(SIZE)
        .with_decorations(false)
        .with_resizable(false)
        .with_window_level(egui::WindowLevel::AlwaysOnTop);
    let (main, monitor) = ctx.input_for(egui::ViewportId::ROOT, |i| (i.viewport().outer_rect, i.viewport().monitor_size));
    if let Some(at) = splash_position(main, monitor) {
        builder = builder.with_position(at);
    }
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("deckard-splash"), builder, |ui, class| {
        let rect =
            if class == egui::ViewportClass::EmbeddedWindow { ui.allocate_exact_size(SIZE, Sense::hover()).0 } else { ui.max_rect() };
        paint(ui, rect, boot);
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            boot.hidden = true;
        }
    });
}

/// The splash in `rect`: the tube, the header, the logo, the log, the bar and the
/// buttons.
fn paint(ui: &mut egui::Ui, rect: Rect, boot: &Boot) {
    let painter = ui.painter_at(rect);
    // The bezel and the tube in it, with rounded corners.
    painter.rect_filled(rect, 0.0, Color32::from_gray(0x11));
    let tube = rect.shrink(8.0);
    vignette(&painter, tube);
    painter.rect_stroke(tube, 18.0, Stroke::new(10.0, Color32::from_gray(0x11)), egui::StrokeKind::Outside);
    painter.rect_stroke(tube.expand(9.0), 26.0, Stroke::new(1.0, Color32::from_gray(0x33)), egui::StrokeKind::Inside);
    let painter = painter.with_clip_rect(tube.shrink(2.0));

    let inner = Rect::from_min_max(tube.min + Vec2::new(30.0, 20.0), tube.max - Vec2::new(30.0, 16.0));
    let mono = FontId::monospace(12.0);
    let line_h = 16.0;

    // Header.
    let mut y = inner.top();
    glow_text(&painter, Pos2::new(inner.left(), y), Align2::LEFT_TOP, "PANIC SIGNALS // DECKARD BOOT SEQUENCE", &mono, DIM);
    glow_text(&painter, Pos2::new(inner.right(), y), Align2::RIGHT_TOP, &format!("v{VERSION} · {}", clock()), &mono, DIM);
    y += line_h + 5.0;
    painter.hline(inner.x_range(), y.round() + 0.5, Stroke::new(1.0, DIM));

    // Logo.
    y += 12.0;
    let logo_mid = Pos2::new(inner.center().x, y + 44.0);
    logo(&painter, logo_mid);
    y += 92.0;

    // Tagline.
    let tag_font = FontId::monospace(11.0);
    spaced_text(&painter, Pos2::new(inner.center().x, y), "GROOVEBOX · SAMPLER · SEQUENCER", &tag_font, DIM, 4.0);
    y += line_h + 12.0;

    // The log: as many of the newest lines as fit.
    let log_bottom = inner.bottom() - 130.0;
    let fit = (((log_bottom - y) / line_h).floor() as usize).max(1);
    let cursor_on = boot.started.elapsed().as_secs_f32().fract() < 0.5;
    let char_w = painter.layout_no_wrap("0".into(), mono.clone(), FG).size().x;
    for line in boot.log.iter().skip(boot.log.len().saturating_sub(fit)) {
        let x = inner.left();
        glow_text(&painter, Pos2::new(x, y), Align2::LEFT_TOP, ">", &mono, DIM);
        let label = format!("{} ", line.label);
        glow_text(&painter, Pos2::new(x + 2.0 * char_w, y), Align2::LEFT_TOP, &label, &mono, FG);
        let dots = RESULT_COLUMN.saturating_sub(label.chars().count() + 1).max(3);
        let dots_x = x + (2 + label.chars().count()) as f32 * char_w;
        glow_text(&painter, Pos2::new(dots_x, y), Align2::LEFT_TOP, &".".repeat(dots), &mono, DIM);
        let result_x = dots_x + (dots + 1) as f32 * char_w;
        match &line.outcome {
            None => {
                if cursor_on {
                    glow_text(&painter, Pos2::new(result_x, y), Align2::LEFT_TOP, "█", &mono, FG);
                }
            }
            Some(o) => {
                let (text, color) = result(o);
                let room = ((inner.right() - result_x) / char_w).max(0.0) as usize;
                glow_text(&painter, Pos2::new(result_x, y), Align2::LEFT_TOP, &clip(&text, room), &mono, color);
            }
        }
        y += line_h;
    }

    // The bar, under what is running.
    let y = log_bottom + 8.0;
    let current = boot.log.last().map_or("STARTING", |l| l.label.as_str());
    let bar = format!("{:<34}  {}", clip(current, 34), bar_text(boot.progress(), BAR_WIDTH));
    glow_text(&painter, Pos2::new(inner.left(), y), Align2::LEFT_TOP, &bar, &mono, FG);

    // The buttons.
    let art_font = FontId::monospace(9.0);
    let caption_font = FontId::monospace(10.0);
    let buttons = [
        ("GitHub", GITHUB_ART, "[ SOURCE ]", GITHUB_URL.to_string()),
        ("Discord", DISCORD_ART, "[ COMMUNITY ]", DISCORD_URL.to_string()),
        ("Release notes", NOTES_ART, "[ RELEASE NOTES ]", release_notes_link(VERSION)),
    ];
    let galleys: Vec<_> = buttons.iter().map(|(_, art, ..)| painter.layout_no_wrap((*art).into(), art_font.clone(), FG)).collect();
    let pad = Vec2::new(10.0, 6.0);
    let gap = 22.0;
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 2.0 * pad.x).collect();
    let height = galleys.iter().map(|g| g.size().y).fold(0.0, f32::max) + 2.0 * pad.y;
    let total = widths.iter().sum::<f32>() + gap * (widths.len() - 1) as f32;
    let top = inner.bottom() - 16.0 - height;
    let mut x = inner.center().x - total / 2.0;
    for (((label, _, caption, url), galley), w) in buttons.into_iter().zip(galleys).zip(widths) {
        let r = Rect::from_min_size(Pos2::new(x, top), Vec2::new(w, height));
        let id = egui::Id::new(("deckard-splash-button", label));
        let response = ui.interact(r, id, Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        let hot = response.hovered();
        if hot {
            painter.rect_filled(r, 0.0, FG.gamma_multiply(0.14));
        }
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, if hot { FG } else { DIM }), egui::StrokeKind::Inside);
        let at = Pos2::new(r.center().x - galley.size().x / 2.0, r.top() + pad.y);
        glow_galley(&painter, at, galley, &art_font);
        glow_text(&painter, Pos2::new(r.center().x, r.bottom() + 4.0), Align2::CENTER_TOP, caption, &caption_font, DIM);
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
        x += w + gap;
    }

    // Scanlines over it all.
    let mut sy = tube.top();
    while sy < tube.bottom() {
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(tube.left(), sy), Vec2::new(tube.width(), 1.0)),
            0.0,
            Color32::from_black_alpha(70),
        );
        sy += 3.0;
    }
}

/// A step's result as shown, and its colour: what was found bright, what wasn't dim.
fn result(o: &Outcome) -> (String, Color32) {
    match o {
        Outcome::Ok => ("OK".into(), FG),
        Outcome::Text(t) if t == "NOT CONNECTED" || t == "NONE" => (t.clone(), DIM),
        Outcome::Text(t) => (t.to_uppercase(), FG),
        Outcome::Err(e) => (format!("FAILED: {}", e.to_uppercase()), FG),
        Outcome::Skipped => ("SKIPPED".into(), DIM),
    }
}

/// `text` cut to `n` characters, an ellipsis at the end if it was longer.
fn clip(text: &str, n: usize) -> String {
    if text.chars().count() <= n {
        text.into()
    } else {
        let mut s: String = text.chars().take(n.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

/// The background: lighter in the middle, black at the edges (an elliptical gradient,
/// as a mesh).
fn vignette(painter: &Painter, rect: Rect) {
    let (nx, ny) = (32, 20);
    let centre = mix(BG, FG, 0.06);
    let color = |u: f32, v: f32| {
        let d = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt() / 0.5f32.hypot(0.5);
        if d < 0.7 { mix(centre, BG, d / 0.7) } else { mix(BG, Color32::BLACK, ((d - 0.7) / 0.3).min(1.0)) }
    };
    let mut mesh = egui::Mesh::default();
    for j in 0..=ny {
        for i in 0..=nx {
            let (u, v) = (i as f32 / nx as f32, j as f32 / ny as f32);
            mesh.colored_vertex(rect.lerp_inside(Vec2::new(u, v)), color(u, v));
        }
    }
    let w = nx + 1;
    for j in 0..ny {
        for i in 0..nx {
            let a = (j * w + i) as u32;
            mesh.add_triangle(a, a + 1, a + w as u32);
            mesh.add_triangle(a + 1, a + w as u32 + 1, a + w as u32);
        }
    }
    painter.add(mesh);
    // A darker rim, as the tube's inset shadow.
    for k in 0..8 {
        let r = rect.shrink(k as f32 * 3.0);
        painter.rect_stroke(r, 18.0, Stroke::new(3.0, Color32::from_black_alpha(60 - k * 7)), egui::StrokeKind::Inside);
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Offsets (in parts of the font's size) and strengths of the glow under text.
const GLOW: [(f32, f32); 2] = [(0.07, 0.2), (0.18, 0.07)];

/// `text` with a soft glow: faint copies around it, the sharp text on top.
fn glow_text(painter: &Painter, pos: Pos2, align: Align2, text: &str, font: &FontId, color: Color32) {
    let galley = painter.layout_no_wrap(text.into(), font.clone(), color);
    let at = align.anchor_size(pos, galley.size()).min;
    glow_galley(painter, at, galley, font);
}

fn glow_galley(painter: &Painter, at: Pos2, galley: std::sync::Arc<egui::Galley>, font: &FontId) {
    let color = galley.job.sections.first().map_or(FG, |s| s.format.color);
    for (r, a) in GLOW {
        let r = r * font.size;
        for d in [Vec2::new(r, 0.0), Vec2::new(-r, 0.0), Vec2::new(0.0, r), Vec2::new(0.0, -r)] {
            painter.galley_with_override_text_color(at + d, galley.clone(), FG.gamma_multiply(a * if color == DIM { 0.5 } else { 1.0 }));
        }
    }
    painter.galley(at, galley, color);
}

/// `text` centred on `top_centre`, `spacing` points between the letters.
fn spaced_text(painter: &Painter, top_centre: Pos2, text: &str, font: &FontId, color: Color32, spacing: f32) {
    let galleys: Vec<_> = text.chars().map(|c| painter.layout_no_wrap(c.to_string(), font.clone(), color)).collect();
    let width = galleys.iter().map(|g| g.size().x).sum::<f32>() + spacing * (galleys.len().saturating_sub(1)) as f32;
    let mut x = top_centre.x - width / 2.0;
    for g in galleys {
        let w = g.size().x;
        glow_galley(painter, Pos2::new(x, top_centre.y), g, font);
        x += w + spacing;
    }
}

/// `deckard` in the logo font, glowing, centred on `centre`; the drawn wordmark when
/// there is no logo font.
fn logo(painter: &Painter, centre: Pos2) {
    if painter.ctx().data(|d| d.get_temp::<bool>(egui::Id::new(layout::LOGO_FONT))).unwrap_or(false) {
        let font = FontId::new(92.0, FontFamily::Name(layout::LOGO_FONT.into()));
        let galley = painter.layout_no_wrap("d\u{65}ck\u{61}\u{72}d".into(), font, FG);
        let at = centre - galley.size() / 2.0;
        for (r, a) in [(2.0, 0.18), (5.0, 0.07), (9.0, 0.03)] {
            for k in 0..8 {
                let angle = k as f32 * std::f32::consts::TAU / 8.0;
                let d = Vec2::angled(angle) * r;
                painter.galley_with_override_text_color(at + d, galley.clone(), FG.gamma_multiply(a));
            }
        }
        painter.galley(at, galley, FG);
        return;
    }
    let scale = 2.5;
    // (Measured first, drawn nowhere, to centre it.)
    let size = layout::deckard_logo_scaled(&painter.with_clip_rect(Rect::NOTHING), Pos2::ZERO, scale).width();
    layout::deckard_logo_scaled(painter, Pos2::new(centre.x - size / 2.0 - 4.0, centre.y), scale);
}

/// The local time, `HH:MM:SS`.
fn clock() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let t = (secs + crate::recorder::local_offset(secs)).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VERSION;
    use crate::boot::{Boot, all_steps};
    use eframe::egui;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn release_notes_for_this_version_or_the_latest() {
        assert_eq!(release_notes_link("0.4.2"), "https://github.com/Panic-Signals/Deckard/releases/tag/0.4.2");
        assert_eq!(release_notes_link("0.1.0-dev"), "https://github.com/Panic-Signals/Deckard/releases/latest");
    }

    #[test]
    fn centred_on_the_main_window_wherever_it_is() {
        // The main window on a second display, to the right of the first.
        let main = Rect::from_min_size(Pos2::new(2261.0, 100.0), Vec2::new(1500.0, 930.0));
        let at = splash_position(Some(main), Some(Vec2::new(1800.0, 1169.0))).unwrap();
        assert_eq!(at, main.center() - SIZE / 2.0);
        // Not known yet: the monitor's middle.
        assert_eq!(splash_position(None, Some(Vec2::new(1800.0, 1200.0))), Some(Pos2::new(520.0, 360.0)));
        assert_eq!(splash_position(None, None), None);
    }

    #[test]
    fn the_bar_is_text() {
        assert_eq!(bar_text(0.5, 10), "[█████░░░░░]  50%");
        assert_eq!(bar_text(1.0, 4), "[████] 100%");
        assert_eq!(bar_text(0.0, 4), "[░░░░]   0%");
    }

    /// The URLs the last frame asked to open.
    fn opened(h: &Harness<'_, Boot>) -> Vec<String> {
        h.output()
            .platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::OpenUrl(u) => Some(u.url.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_buttons_open_their_links() {
        let mut h = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_ui_state(|ui, boot: &mut Boot| show(ui.ctx(), boot), Boot::new(all_steps(None)));
        h.run_steps(2);
        for (label, url) in
            [("GitHub", GITHUB_URL.to_string()), ("Discord", DISCORD_URL.into()), ("Release notes", release_notes_link(VERSION))]
        {
            h.get_by_label(label).click();
            h.step();
            assert_eq!(opened(&h), vec![url], "{label}");
            // (Clicking doesn't close the splash.)
            assert!(!h.state().hidden);
        }
    }

    /// A picture of the splash part-way through (run by hand:
    /// `DECKARD_PICTURE=splash.png cargo test -p deckard -- --ignored splash_picture`).
    #[test]
    #[ignore]
    fn splash_picture() {
        use crate::boot::{LogLine, Outcome};
        let out = std::env::var("DECKARD_PICTURE").unwrap_or_else(|_| "splash.png".into());
        let mut boot = Boot::new(all_steps(None));
        let line = |label: &str, outcome: Option<Outcome>| LogLine { label: label.into(), outcome };
        boot.log = vec![
            line("READING SETTINGS", Some(Outcome::Ok)),
            line("AUDIO DRIVER", Some(Outcome::Skipped)),
            line("MASCHINE MK3", Some(Outcome::Text("FOUND".into()))),
            line("LAUNCHPAD PRO MK3", Some(Outcome::Text("NOT CONNECTED".into()))),
            line("PLUG-IN CATALOG", Some(Outcome::Text("214 READY".into()))),
            line("CONTROLLER SURFACE", Some(Outcome::Ok)),
            line("OPENING NIGHT DRIVE.MXPRJ", Some(Outcome::Text("NIGHT DRIVE".into()))),
            line("LOADING PLUG-IN KONTAKT 7 [3/9]", None),
        ];
        boot.total = 19;
        boot.done = 11;
        let mut h = Harness::builder().with_size(SIZE).with_pixels_per_point(2.0).build_ui_state(
            |ui, boot: &mut Boot| {
                let rect = ui.max_rect();
                paint(ui, rect, boot);
            },
            boot,
        );
        crate::layout::style(&h.ctx);
        h.run_steps(3);
        h.render().unwrap().save(out).unwrap();
    }
}
