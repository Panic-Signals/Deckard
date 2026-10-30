//! Both MK3 screens, side by side (960 × 272 RGBA), drawn by the scripts through the
//! `screen` they get in `draw(screen, theme)`. The theme (`themes/<name>.lua`) is the
//! stylesheet every page shares: named colours, and named text styles. DECKARD's own
//! fonts are Ubuntu Light and Hack (from egui).

use std::sync::OnceLock;

use fontdue::{Font, FontSettings};
use mlua::{Lua, Table, UserData, UserDataMethods, Value};

pub const WIDTH: usize = 960;
pub const HEIGHT: usize = 272;

/// A picture the scripts can draw by its key (`screen:image`): RGBA, `w` × `h`.
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[u8; 4]>,
}

/// The pictures given to the surface, by (key, thumbnail), and handed to each draw.
pub type Pictures = std::rc::Rc<std::cell::RefCell<std::collections::HashMap<(String, bool), Image>>>;

/// DECKARD's fonts, by the name scripts and themes use: egui's Ubuntu Light and Hack,
/// and Google's Roboto (Apache 2.0) and Roboto Mono (SIL OFL 1.1), as Maschine's screens
/// use (`fonts/`, with their licences).
pub const FONTS: [(&str, &[u8]); 9] = [
    ("ubuntu", epaint_default_fonts::UBUNTU_LIGHT),
    ("hack", epaint_default_fonts::HACK_REGULAR),
    ("roboto", include_bytes!("../fonts/Roboto-Regular.ttf")),
    ("roboto-light", include_bytes!("../fonts/Roboto-Light.ttf")),
    ("roboto-medium", include_bytes!("../fonts/Roboto-Medium.ttf")),
    ("roboto-bold", include_bytes!("../fonts/Roboto-Bold.ttf")),
    ("roboto-mono", include_bytes!("../fonts/RobotoMono-Regular.ttf")),
    ("roboto-mono-medium", include_bytes!("../fonts/RobotoMono-Medium.ttf")),
    ("roboto-mono-bold", include_bytes!("../fonts/RobotoMono-Bold.ttf")),
];

fn font(name: &str) -> mlua::Result<&'static Font> {
    static LOADED: OnceLock<Vec<Font>> = OnceLock::new();
    let all = LOADED.get_or_init(|| FONTS.iter().map(|(n, bytes)| Font::from_bytes(*bytes, FontSettings::default()).expect(n)).collect());
    match FONTS.iter().position(|(n, _)| *n == name) {
        Some(i) => Ok(&all[i]),
        None => {
            let names: Vec<&str> = FONTS.iter().map(|(n, _)| *n).collect();
            Err(mlua::Error::runtime(format!("no font \"{name}\" ({})", names.join(", "))))
        }
    }
}

/// A colour as the scripts write it: `{r, g, b}` (0..255) or `{r = , g = , b = }`.
pub fn rgb(t: &Table) -> mlua::Result<[u8; 3]> {
    let get = |i: usize, k: &str| -> mlua::Result<u8> {
        let v = t.get::<Option<f64>>(i)?.or(t.get::<Option<f64>>(k)?).unwrap_or(0.0);
        Ok(v.clamp(0.0, 255.0) as u8)
    };
    Ok([get(1, "r")?, get(2, "g")?, get(3, "b")?])
}

/// A colour with its opacity (0..1): `{r, g, b, a}`, `a` 0..255 and 255 if left out.
#[derive(Clone, Copy)]
pub struct Paint {
    pub rgb: [u8; 3],
    pub alpha: f32,
}

fn rgba(t: &Table) -> mlua::Result<Paint> {
    let a = t.get::<Option<f64>>(4)?.or(t.get::<Option<f64>>("a")?).unwrap_or(255.0);
    Ok(Paint { rgb: rgb(t)?, alpha: (a / 255.0).clamp(0.0, 1.0) as f32 })
}

/// A colour, or the name of one of the theme's `colors`.
pub fn colour(lua: &Lua, v: &Value) -> mlua::Result<[u8; 3]> {
    Ok(paint(lua, v)?.rgb)
}

/// A colour with its opacity, or the name of one of the theme's `colors`.
pub fn paint(lua: &Lua, v: &Value) -> mlua::Result<Paint> {
    match v {
        Value::Table(t) => rgba(t),
        Value::String(s) => {
            let name = s.to_str()?.to_string();
            let theme: Table = lua.named_registry_value("deckard.theme")?;
            let colors: Table = theme.get("colors")?;
            match colors.get::<Option<Table>>(name.as_str())? {
                Some(t) => rgba(&t),
                None => Err(mlua::Error::runtime(format!("the theme has no colour \"{name}\""))),
            }
        }
        _ => Err(mlua::Error::runtime("a colour: {r, g, b} or a theme colour's name")),
    }
}

/// A text style: a table, or the name of one of the theme's `styles`. Its `font`
/// (a font, or the name of one of the theme's `fonts`), `size` and `color`; missing ones
/// come from the theme's `styles.default`.
struct Style {
    font: &'static Font,
    size: f32,
    colour: Paint,
}

fn style(lua: &Lua, v: &Value) -> mlua::Result<Style> {
    let theme: Table = lua.named_registry_value("deckard.theme")?;
    let styles: Table = theme.get("styles")?;
    let own = match v {
        Value::Table(t) => Some(t.clone()),
        Value::String(s) => {
            let name = s.to_str()?.to_string();
            Some(
                styles
                    .get::<Option<Table>>(name.as_str())?
                    .ok_or_else(|| mlua::Error::runtime(format!("the theme has no style \"{name}\"")))?,
            )
        }
        Value::Nil => None,
        _ => return Err(mlua::Error::runtime("a style: a table or a theme style's name")),
    };
    let default = styles.get::<Option<Table>>("default")?;
    let field = |k: &str| -> mlua::Result<Value> {
        for t in own.iter().chain(default.iter()) {
            let v = t.get::<Value>(k)?;
            if !v.is_nil() {
                return Ok(v);
            }
        }
        Ok(Value::Nil)
    };
    let font_name = match field("font")? {
        Value::String(s) => s.to_str()?.to_string(),
        _ => "ubuntu".to_string(),
    };
    let fonts = theme.get::<Option<Table>>("fonts")?;
    let font_name = match fonts.map(|f| f.get::<Option<String>>(font_name.as_str())).transpose()?.flatten() {
        Some(real) => real,
        None => font_name,
    };
    let size = match field("size")? {
        Value::Integer(n) => n as f32,
        Value::Number(n) => n as f32,
        _ => 15.0,
    };
    let colour = match field("color")? {
        Value::Nil => Paint { rgb: [255; 3], alpha: 1.0 },
        c => paint(lua, &c)?,
    };
    Ok(Style { font: font(&font_name)?, size: size.clamp(4.0, 200.0), colour })
}

/// What the scripts draw into.
pub struct Picture {
    pub px: Vec<[u8; 4]>,
    /// The text drawn, in order (for tests).
    pub texts: Vec<String>,
}

impl Picture {
    pub fn new(background: [u8; 3]) -> Picture {
        let [r, g, b] = background;
        Picture { px: vec![[r, g, b, 255]; WIDTH * HEIGHT], texts: Vec::new() }
    }

    fn blend(&mut self, x: i32, y: i32, c: [u8; 3], a: f32) {
        if x < 0 || y < 0 || x as usize >= WIDTH || y as usize >= HEIGHT || a <= 0.0 {
            return;
        }
        let d = &mut self.px[y as usize * WIDTH + x as usize];
        for k in 0..3 {
            d[k] = (d[k] as f32 * (1.0 - a) + c[k] as f32 * a).round() as u8;
        }
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
        self.fill(x, y, w, h, Paint { rgb: c, alpha: 1.0 }, 1.0);
    }

    /// A filled rectangle; its four corner pixels at `corner` of its opacity (a little
    /// rounding).
    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: Paint, corner: f32) {
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x.saturating_add(w)).min(WIDTH as i32), (y.saturating_add(h)).min(HEIGHT as i32));
        for yy in y0..y1 {
            for xx in x0..x1 {
                let at_corner = (xx == x || xx == x + w - 1) && (yy == y || yy == y + h - 1);
                self.blend(xx, yy, c.rgb, c.alpha * if at_corner { corner } else { 1.0 });
            }
        }
    }

    /// A one-pixel outline, its corner pixels at `corner` of its opacity.
    fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, c: Paint, corner: f32) {
        for xx in x..x + w {
            for yy in [y, y + h - 1] {
                let at_corner = xx == x || xx == x + w - 1;
                self.blend(xx, yy, c.rgb, c.alpha * if at_corner { corner } else { 1.0 });
            }
        }
        for yy in y + 1..y + h - 1 {
            self.blend(x, yy, c.rgb, c.alpha);
            self.blend(x + w - 1, yy, c.rgb, c.alpha);
        }
    }

    /// A triangle filling `w` × `h` at (x, y), pointing "left", "right", "up" or "down".
    fn triangle(&mut self, x: i32, y: i32, w: i32, h: i32, dir: &str, c: Paint) {
        let (wf, hf) = (w as f32, h as f32);
        for yy in 0..h {
            for xx in 0..w {
                // Coverage from 4 × 4 samples per pixel.
                let mut n = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let (px, py) = ((xx as f32 + (sx as f32 + 0.5) / 4.0) / wf, (yy as f32 + (sy as f32 + 0.5) / 4.0) / hf);
                        let inside = match dir {
                            "left" => (py - 0.5).abs() <= px / 2.0,
                            "right" => (py - 0.5).abs() <= (1.0 - px) / 2.0,
                            "up" => (px - 0.5).abs() <= py / 2.0,
                            _ => (px - 0.5).abs() <= (1.0 - py) / 2.0,
                        };
                        n += inside as i32;
                    }
                }
                self.blend(x + xx, y + yy, c.rgb, c.alpha * n as f32 / 16.0);
            }
        }
    }

    /// Pixels near `inside` (a test on pixel centres in screen space), smoothed by 4 × 4
    /// samples, over the box (x0, y0)-(x1, y1).
    fn shape(&mut self, (x0, y0, x1, y1): (f32, f32, f32, f32), c: Paint, inside: impl Fn(f32, f32) -> bool) {
        for y in y0.floor() as i32..=y1.ceil() as i32 {
            for x in x0.floor() as i32..=x1.ceil() as i32 {
                let mut n = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        n += inside(x as f32 + (sx as f32 + 0.5) / 4.0, y as f32 + (sy as f32 + 0.5) / 4.0) as i32;
                    }
                }
                self.blend(x, y, c.rgb, c.alpha * n as f32 / 16.0);
            }
        }
    }

    /// A disc of radius `r` at (cx, cy), or a ring `width` thick; only from one angle to
    /// another if given (degrees, 0 to the right, clockwise).
    fn circle(&mut self, cx: f32, cy: f32, r: f32, width: Option<f32>, c: Paint, arc: Option<(f32, f32)>) {
        let inner = width.map_or(-1.0, |w| r - w);
        self.shape((cx - r, cy - r, cx + r, cy + r), c, |x, y| {
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            let along = arc.is_none_or(|(from, to)| {
                let a = (y - cy).atan2(x - cx).to_degrees();
                (a - from).rem_euclid(360.0) <= (to - from).rem_euclid(360.0)
            });
            d <= r && d >= inner && along
        });
    }

    /// A line `width` thick from (x0, y0) to (x1, y1).
    fn line(&mut self, (x0, y0): (f32, f32), (x1, y1): (f32, f32), width: f32, c: Paint) {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        let h = width / 2.0;
        let bounds = (x0.min(x1) - h, y0.min(y1) - h, x0.max(x1) + h, y0.max(y1) + h);
        self.shape(bounds, c, |x, y| {
            let t = (((x - x0) * dx + (y - y0) * dy) / len2).clamp(0.0, 1.0);
            let (px, py) = (x0 + t * dx, y0 + t * dy);
            ((x - px).powi(2) + (y - py).powi(2)).sqrt() <= h
        });
    }

    /// Text placed in the box (x, y, w, h): `align` "left", "center" or "right",
    /// `valign` "top", "center" or "bottom" (the font's ascent to descent). Returns its
    /// width.
    #[allow(clippy::too_many_arguments)]
    fn label(&mut self, s: &Style, x: i32, y: i32, w: i32, h: i32, text: &str, align: &str, valign: &str) -> i32 {
        let width = Self::width(s, text).min(w);
        let x0 = match align {
            "left" => x,
            "right" => x + w - width,
            _ => x + (w - width) / 2,
        };
        let (ascent, descent) = s.font.horizontal_line_metrics(s.size).map_or((s.size, 0.0), |m| (m.ascent, m.descent));
        let text_h = (ascent - descent).round() as i32;
        let top = match valign {
            "top" => y,
            "bottom" => y + h - text_h,
            _ => y + (h - text_h) / 2,
        };
        self.text(s, x0, top + ascent.round() as i32, x + w - x0, text)
    }

    /// Text with its baseline at `y`, cut at `max_w` pixels; returns its width.
    fn text(&mut self, s: &Style, x: i32, y: i32, max_w: i32, text: &str) -> i32 {
        let mut pen = 0.0f32;
        for ch in text.chars() {
            let (m, bitmap) = s.font.rasterize(ch, s.size);
            if (pen + m.advance_width) as i32 > max_w {
                break;
            }
            let x0 = x + (pen + m.xmin as f32).round() as i32;
            let y0 = y - m.height as i32 - m.ymin;
            for row in 0..m.height {
                for col in 0..m.width {
                    let a = bitmap[row * m.width + col] as f32 / 255.0;
                    self.blend(x0 + col as i32, y0 + row as i32, s.colour.rgb, a * s.colour.alpha);
                }
            }
            pen += m.advance_width;
        }
        pen.round() as i32
    }

    fn width(s: &Style, text: &str) -> i32 {
        text.chars().map(|ch| s.font.metrics(ch, s.size).advance_width).sum::<f32>().round() as i32
    }

    /// `img` scaled to fit `w` × `h` at (x, y), centred, keeping its shape (each pixel
    /// the average of those it covers).
    fn image(&mut self, x: i32, y: i32, w: i32, h: i32, img: &Image) {
        if img.w == 0 || img.h == 0 || w <= 0 || h <= 0 {
            return;
        }
        let k = (w as f32 / img.w as f32).min(h as f32 / img.h as f32);
        let (dw, dh) = (((img.w as f32 * k).round() as i32).max(1), ((img.h as f32 * k).round() as i32).max(1));
        let (ox, oy) = (x + (w - dw) / 2, y + (h - dh) / 2);
        for dy in 0..dh {
            let (sy0, sy1) = ((dy as f32 / k) as usize, (((dy + 1) as f32 / k).ceil() as usize).clamp(1, img.h));
            for dx in 0..dw {
                let (sx0, sx1) = ((dx as f32 / k) as usize, (((dx + 1) as f32 / k).ceil() as usize).clamp(1, img.w));
                let (mut sum, mut n) = ([0u32; 4], 0u32);
                for sy in sy0.min(sy1 - 1)..sy1 {
                    for sx in sx0.min(sx1 - 1)..sx1 {
                        let p = img.px[sy * img.w + sx];
                        for c in 0..4 {
                            sum[c] += u32::from(p[c]);
                        }
                        n += 1;
                    }
                }
                let avg = sum.map(|v| (v / n.max(1)) as u8);
                self.blend(ox + dx, oy + dy, [avg[0], avg[1], avg[2]], f32::from(avg[3]) / 255.0);
            }
        }
    }

    /// A script error, over the bottom of the left screen.
    pub fn error(&mut self, message: &str) {
        self.rect(0, HEIGHT as i32 - 30, 480, 30, [0x8c, 0x1e, 0x1e]);
        let s = Style { font: font("ubuntu").expect("built in"), size: 14.0, colour: Paint { rgb: [255; 3], alpha: 1.0 } };
        self.text(&s, 8, HEIGHT as i32 - 10, 464, message);
    }
}

impl UserData for Picture {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("size", |_, _, ()| Ok((WIDTH, HEIGHT)));
        m.add_method_mut("rect", |lua, p, (x, y, w, h, c, corner): (i32, i32, i32, i32, Value, Option<f32>)| {
            let c = paint(lua, &c)?;
            p.fill(x, y, w, h, c, corner.unwrap_or(1.0));
            Ok(())
        });
        m.add_method_mut("frame", |lua, p, (x, y, w, h, c, corner): (i32, i32, i32, i32, Value, Option<f32>)| {
            let c = paint(lua, &c)?;
            p.frame(x, y, w, h, c, corner.unwrap_or(1.0));
            Ok(())
        });
        m.add_method_mut("triangle", |lua, p, (x, y, w, h, dir, c): (i32, i32, i32, i32, String, Value)| {
            let c = paint(lua, &c)?;
            p.triangle(x, y, w, h, &dir, c);
            Ok(())
        });
        m.add_method_mut(
            "circle",
            |lua, p, (cx, cy, r, c, width, from, to): (f32, f32, f32, Value, Option<f32>, Option<f32>, Option<f32>)| {
                let c = paint(lua, &c)?;
                p.circle(cx, cy, r, width, c, from.zip(to));
                Ok(())
            },
        );
        m.add_method_mut("line", |lua, p, (x0, y0, x1, y1, width, c): (f32, f32, f32, f32, f32, Value)| {
            let c = paint(lua, &c)?;
            p.line((x0, y0), (x1, y1), width, c);
            Ok(())
        });
        m.add_method_mut(
            "label",
            |lua, p, (x, y, w, h, text, st, align, valign): (i32, i32, i32, i32, String, Value, Option<String>, Option<String>)| {
                let s = style(lua, &st)?;
                p.texts.push(text.clone());
                Ok(p.label(&s, x, y, w, h, &text, align.as_deref().unwrap_or("center"), valign.as_deref().unwrap_or("center")))
            },
        );
        m.add_method_mut("text", |lua, p, (x, y, text, st, max_w): (i32, i32, String, Value, Option<i32>)| {
            let s = style(lua, &st)?;
            p.texts.push(text.clone());
            Ok(p.text(&s, x, y, max_w.unwrap_or(WIDTH as i32), &text))
        });
        // image(x, y, w, h, key, thumb): a picture given to the surface, fitted into the
        // box; false if there's none by that key.
        m.add_method_mut("image", |lua, p, (x, y, w, h, key, thumb): (i32, i32, i32, i32, String, Option<bool>)| {
            let Some(pictures) = lua.app_data_ref::<Pictures>().map(|p| p.clone()) else { return Ok(false) };
            let pictures = pictures.borrow();
            let Some(img) = pictures.get(&(key, thumb.unwrap_or(true))) else { return Ok(false) };
            p.image(x, y, w, h, img);
            Ok(true)
        });
        m.add_method("text_width", |lua, _, (text, st): (String, Value)| Ok(Picture::width(&style(lua, &st)?, &text)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::view;
    use crate::{GroupView, SoundView, Surface, View};

    #[test]
    fn screens_redraw_only_on_change() {
        let (mut s, mut v) = (Surface::new(), view());
        let (n, px) = s.screens(&v).expect("first picture");
        assert_eq!(s.error(), None);
        assert_eq!((n, px.len()), (1, WIDTH * HEIGHT));
        assert!(s.screens(&v).is_none(), "nothing changed");
        v.tempo = 121.0;
        assert_eq!(s.screens(&v).map(|(n, _)| n), Some(2));
    }

    #[test]
    fn screens_draw_any_project() {
        let mut s = Surface::new();
        assert!(s.screens(&View::default()).is_some(), "an empty view");
        assert_eq!(s.error(), None, "an empty view");
        let mut v = view();
        v.groups.push(GroupView { name: String::new(), sounds: vec![SoundView::default(); 40], ..Default::default() });
        v.group = 2;
        v.sound = 39;
        v.control.page = 9;
        v.control.level = 7;
        v.project = "x".repeat(300);
        assert!(s.screens(&v).is_some());
        assert_eq!(s.error(), None);
    }

    fn lit(px: &[[u8; 4]], x0: usize, x1: usize, y0: usize, y1: usize) -> usize {
        (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| px[y * WIDTH + x][0] > 0x80).count()
    }

    /// A ring drawn from one angle to another leaves the rest open (0° to the right,
    /// clockwise).
    #[test]
    fn a_ring_can_be_open() {
        let mut p = Picture::new([0; 3]);
        let white = Paint { rgb: [255; 3], alpha: 1.0 };
        p.circle(100.0, 100.0, 10.0, Some(2.0), white, Some((-150.0, 150.0)));
        assert!(lit(&p.px, 108, 111, 98, 102) > 0, "the right side");
        assert_eq!(lit(&p.px, 89, 92, 98, 102), 0, "the left side open");
        assert!(lit(&p.px, 98, 102, 89, 92) > 0, "the top");
    }

    #[test]
    fn the_header_shows_text() {
        let (mut s, v) = (Surface::new(), view());
        let (_, px) = s.screens(&v).unwrap();
        assert!(lit(&px, 10, 200, 34, 56) > 50, "the project name");
        assert!(lit(&px, 490, 900, 34, 56) > 50, "the slot");
        assert!(lit(&px, 10, 110, 200, 240) > 20, "knob 1");
        assert_eq!(lit(&px, 720, 960, 200, 240), 0, "no knobs 7 and 8");
    }

    #[test]
    fn bar_beat_counts_from_one() {
        let s = Surface::new();
        let bb = |t: i64, n: i64, d: i64| s.eval::<String>(&format!("require('lib.util').bar_beat({t}, {n}, {d})")).unwrap();
        assert_eq!(bb(0, 4, 4), "1.1.1");
        assert_eq!(bb(960 * 4 + 960 + 240, 4, 4), "2.2.2");
        assert_eq!(bb(480 * 7, 7, 8), "2.1.1");
    }

    /// Writes both screens to the temporary folder to look at (`--ignored`); with
    /// `RENDER_PRESS="BROWSE SHIFT"`, after pressing those buttons (a library to
    /// browse).
    #[test]
    #[ignore]
    fn render_screens() {
        let (mut s, mut v) = (Surface::new(), view());
        v.playing = true;
        v.position = Some(960 * 9 + 480);
        v.browser = crate::BrowserView {
            kind: 6,
            products: (1..=8).map(|i| format!("Expansion {i}")).collect(),
            product: Some(1),
            attributes: [(vec!["Drums".into(), "Bass".into()], Some(0)), (vec!["Kick".into()], None), (Vec::new(), None)],
            results: (1..=12).map(|i| (format!("Kick Deep {i}"), i % 4 == 0)).collect(),
            focus: 3,
            ..Default::default()
        };
        for name in std::env::var("RENDER_PRESS").unwrap_or_default().split_whitespace() {
            let name: &'static str = Box::leak(format!("BUTTON_{name}").into_boxed_str());
            let now = std::time::Instant::now();
            s.event(&deckard_mk3::Event::Button(name, true), &v, now);
            if name != "BUTTON_SHIFT" {
                s.event(&deckard_mk3::Event::Button(name, false), &v, now);
            }
        }
        for theme in s.themes() {
            s.set_theme(&theme);
            let (_, px) = s.screens(&v).unwrap();
            let path = std::env::temp_dir().join(format!("deckard-surface-{theme}.png"));
            let f = std::fs::File::create(&path).unwrap();
            let mut e = png::Encoder::new(f, WIDTH as u32, HEIGHT as u32);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header().unwrap().write_image_data(&px.concat()).unwrap();
            eprintln!("{}", path.display());
        }
    }
}
