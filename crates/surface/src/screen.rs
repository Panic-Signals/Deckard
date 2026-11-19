//! Both MK3 screens, side by side (960 × 272 RGBA), drawn by the scripts through the
//! `screen` they get in `draw(screen, theme)`. The theme (`themes/<name>.lua`) is the
//! stylesheet every page shares: named colours, and named text styles. DECKARD's own
//! fonts are Ubuntu Light and Hack (from egui).

use std::collections::HashMap;
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
pub type Pictures = std::rc::Rc<std::cell::RefCell<std::collections::HashMap<(String, bool), std::sync::Arc<Image>>>>;

/// DECKARD's fonts, by the name scripts and themes use: IBM Plex Sans and IBM Plex Mono
/// (SIL OFL 1.1), egui's Ubuntu Light and Hack, and Google's Roboto (Apache 2.0) and
/// Roboto Mono (SIL OFL 1.1), as Maschine's screens use (`fonts/`, with their licences).
pub const FONTS: [(&str, &[u8]); 12] = [
    ("ubuntu", epaint_default_fonts::UBUNTU_LIGHT),
    ("hack", epaint_default_fonts::HACK_REGULAR),
    ("plex", include_bytes!("../fonts/IBMPlexSans-Regular.ttf")),
    ("plex-bold", include_bytes!("../fonts/IBMPlexSans-Bold.ttf")),
    ("plex-mono", include_bytes!("../fonts/IBMPlexMono-Regular.ttf")),
    ("roboto", include_bytes!("../fonts/Roboto-Regular.ttf")),
    ("roboto-light", include_bytes!("../fonts/Roboto-Light.ttf")),
    ("roboto-medium", include_bytes!("../fonts/Roboto-Medium.ttf")),
    ("roboto-bold", include_bytes!("../fonts/Roboto-Bold.ttf")),
    ("roboto-mono", include_bytes!("../fonts/RobotoMono-Regular.ttf")),
    ("roboto-mono-medium", include_bytes!("../fonts/RobotoMono-Medium.ttf")),
    ("roboto-mono-bold", include_bytes!("../fonts/RobotoMono-Bold.ttf")),
];

fn byte_channel(v: f64) -> u8 {
    v.clamp(0.0, 255.0) as u8
}

fn channel(t: &Table, i: usize, k: &str) -> mlua::Result<Option<f64>> {
    if let Some(v) = t.raw_get::<Option<f64>>(i)? {
        return Ok(Some(v));
    }
    if let Some(v) = t.raw_get::<Option<f64>>(k)? {
        return Ok(Some(v));
    }
    if t.metatable().is_some() {
        if let Some(v) = t.get::<Option<f64>>(i)? {
            return Ok(Some(v));
        }
        return t.get::<Option<f64>>(k);
    }
    Ok(None)
}

/// A colour as the scripts write it: `{r, g, b}` (0..255) or `{r = , g = , b = }`.
pub fn rgb(t: &Table) -> mlua::Result<[u8; 3]> {
    if let Some(r) = t.raw_get::<Option<f64>>(1)? {
        let g = t.raw_get::<Option<f64>>(2)?.unwrap_or(0.0);
        let b = t.raw_get::<Option<f64>>(3)?.unwrap_or(0.0);
        return Ok([byte_channel(r), byte_channel(g), byte_channel(b)]);
    }
    let get = |i: usize, k: &str| -> mlua::Result<u8> { Ok(byte_channel(channel(t, i, k)?.unwrap_or(0.0))) };
    Ok([get(1, "r")?, get(2, "g")?, get(3, "b")?])
}

/// A colour with its opacity (0..1): `{r, g, b, a}`, `a` 0..255 and 255 if left out.
#[derive(Clone, Copy, PartialEq)]
pub struct Paint {
    pub rgb: [u8; 3],
    pub alpha: f32,
}

fn rgba(t: &Table) -> mlua::Result<Paint> {
    if let Some(r) = t.raw_get::<Option<f64>>(1)? {
        let g = t.raw_get::<Option<f64>>(2)?.unwrap_or(0.0);
        let b = t.raw_get::<Option<f64>>(3)?.unwrap_or(0.0);
        let a = t.raw_get::<Option<f64>>(4)?.unwrap_or(255.0);
        return Ok(Paint { rgb: [byte_channel(r), byte_channel(g), byte_channel(b)], alpha: (a / 255.0).clamp(0.0, 1.0) as f32 });
    }
    let a = channel(t, 4, "a")?.unwrap_or(255.0);
    Ok(Paint { rgb: rgb(t)?, alpha: (a / 255.0).clamp(0.0, 1.0) as f32 })
}

/// Resolved colours, fonts and styles from `deckard.theme`, built once on theme load.
pub struct ThemeCache {
    colors: HashMap<String, Paint>,
    fonts: HashMap<String, usize>,
    styles: HashMap<String, Style>,
    default_style: Style,
    pub background: [u8; 3],
    pub text: [u8; 3],
    pub accent: [u8; 3],
    pub animates: bool,
    /// The theme asks widgets for soft fills (`widgets_soft = true`).
    pub soft: bool,
}

impl ThemeCache {
    pub fn from_theme(theme: &Table) -> mlua::Result<ThemeCache> {
        let mut colors = HashMap::new();
        if let Some(ct) = theme.get::<Option<Table>>("colors")? {
            for pair in ct.pairs::<Value, Value>() {
                if let Ok((Value::String(k), Value::Table(t))) = pair
                    && let Ok(name) = k.to_str()
                    && let Ok(p) = rgba(&t)
                {
                    colors.insert(name.to_string(), p);
                }
            }
        }
        let mut fonts = HashMap::new();
        if let Some(ft) = theme.get::<Option<Table>>("fonts")? {
            for pair in ft.pairs::<Value, Value>() {
                if let Ok((Value::String(k), Value::String(v))) = pair
                    && let (Ok(alias), Ok(real)) = (k.to_str(), v.to_str())
                    && let Ok(idx) = font_index(&real)
                {
                    fonts.insert(alias.to_string(), idx);
                }
            }
        }
        let resolve_color = |v: &Value| -> Option<Paint> {
            match v {
                Value::Table(t) => rgba(t).ok(),
                Value::String(s) => s.to_str().ok().and_then(|n| colors.get(n.as_ref()).copied()),
                _ => None,
            }
        };
        let resolve_font = |v: &Value| -> Option<usize> {
            let Value::String(s) = v else { return None };
            let name = s.to_str().ok()?;
            fonts.get(name.as_ref()).copied().or_else(|| font_index(&name).ok())
        };
        let base_font = font_index("ubuntu").expect("built in");
        let fallback_default =
            Style { font_idx: base_font, font: font_at(base_font), size: 15.0, colour: Paint { rgb: [255; 3], alpha: 1.0 } };
        let parse_style = |t: &Table, base: Style| -> Option<Style> {
            let font_idx = match t.raw_get::<Value>("font").ok()? {
                Value::Nil => base.font_idx,
                v => resolve_font(&v)?,
            };
            let size = match t.raw_get::<Value>("size").ok()? {
                Value::Integer(n) => (n as f32).clamp(4.0, 200.0),
                Value::Number(n) => (n as f32).clamp(4.0, 200.0),
                Value::Nil => base.size,
                _ => 15.0,
            };
            let colour = match t.raw_get::<Value>("color").ok()? {
                Value::Nil => base.colour,
                ref c => resolve_color(c)?,
            };
            Some(Style { font_idx, font: font_at(font_idx), size, colour })
        };
        let mut styles = HashMap::new();
        let mut default_style = fallback_default;
        if let Some(st) = theme.get::<Option<Table>>("styles")? {
            if let Some(def_t) = st.get::<Option<Table>>("default")?
                && let Some(def) = parse_style(&def_t, fallback_default)
            {
                default_style = def;
            }
            for pair in st.pairs::<Value, Value>() {
                if let Ok((Value::String(k), Value::Table(t))) = pair
                    && let Ok(name) = k.to_str()
                    && let Some(parsed) = parse_style(&t, default_style)
                {
                    styles.insert(name.to_string(), parsed);
                }
            }
        }
        let background = colors.get("background").map_or([0; 3], |p| p.rgb);
        let text = colors.get("text").map_or([255; 3], |p| p.rgb);
        let accent = colors.get("accent").map_or([0xff, 0xa5, 0x2a], |p| p.rgb);
        let animates = theme.get::<Value>("chase").is_ok_and(|c| !c.is_nil());
        let soft = theme.get::<Option<bool>>("widgets_soft").ok().flatten().unwrap_or(false);
        Ok(ThemeCache { colors, fonts, styles, default_style, background, text, accent, animates, soft })
    }

    /// The theme's colour `name`, if it has one.
    pub(crate) fn color(&self, name: &str) -> Option<Paint> {
        self.colors.get(name).copied()
    }

    /// The theme's style `name`, else its default style.
    pub(crate) fn style(&self, name: &str) -> Style {
        self.styles.get(name).copied().unwrap_or(self.default_style)
    }
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
            let name = s.to_str()?;
            if let Some(cache) = lua.app_data_ref::<ThemeCache>()
                && let Some(p) = cache.colors.get(name.as_ref())
            {
                return Ok(*p);
            }
            let name = name.to_string();
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
#[derive(Clone, Copy)]
pub struct Style {
    pub(crate) font_idx: usize,
    pub(crate) font: &'static Font,
    pub(crate) size: f32,
    pub(crate) colour: Paint,
}

pub(crate) fn font_index(name: &str) -> mlua::Result<usize> {
    match FONTS.iter().position(|(n, _)| *n == name) {
        Some(i) => Ok(i),
        None => {
            let names: Vec<&str> = FONTS.iter().map(|(n, _)| *n).collect();
            Err(mlua::Error::runtime(format!("no font \"{name}\" ({})", names.join(", "))))
        }
    }
}

pub(crate) fn font_at(i: usize) -> &'static Font {
    static LOADED: OnceLock<Vec<Font>> = OnceLock::new();
    let all = LOADED.get_or_init(|| FONTS.iter().map(|(n, bytes)| Font::from_bytes(*bytes, FontSettings::default()).expect(n)).collect());
    &all[i]
}

fn gamma_lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|b| if b == 0 { 0.0 } else { (b as f32 / 255.0).powf(1.0 / 1.45) }))
}

type GlyphKey = (usize, u32, char);
type GlyphEntry = (fontdue::Metrics, std::rc::Rc<[u8]>);

thread_local! {
    static GLYPH_CACHE: std::cell::RefCell<HashMap<GlyphKey, GlyphEntry>> =
        std::cell::RefCell::new(HashMap::with_capacity(512));
}

fn cached_glyph(font_idx: usize, font: &Font, size: f32, ch: char) -> GlyphEntry {
    let key = (font_idx, size.to_bits(), ch);
    GLYPH_CACHE.with(|cache| {
        if let Some(hit) = cache.borrow().get(&key) {
            return hit.clone();
        }
        let (m, bmp) = font.rasterize(ch, size);
        let entry = (m, std::rc::Rc::<[u8]>::from(bmp));
        let mut map = cache.borrow_mut();
        if map.len() >= 4096 {
            map.clear();
        }
        map.insert(key, entry.clone());
        entry
    })
}

fn style(lua: &Lua, v: &Value) -> mlua::Result<Style> {
    if let Some(cache) = lua.app_data_ref::<ThemeCache>() {
        match v {
            Value::String(s) => {
                let name = s.to_str()?;
                if let Some(st) = cache.styles.get(name.as_ref()) {
                    return Ok(*st);
                }
            }
            Value::Nil => return Ok(cache.default_style),
            Value::Table(t) if t.metatable().is_none() => {
                let def = cache.default_style;
                let font_idx = match t.raw_get::<Value>("font")? {
                    Value::Nil => def.font_idx,
                    Value::String(s) => {
                        let name = s.to_str()?;
                        match cache.fonts.get(name.as_ref()) {
                            Some(&idx) => idx,
                            None => font_index(&name)?,
                        }
                    }
                    _ => font_index("ubuntu")?,
                };
                let size = match t.raw_get::<Value>("size")? {
                    Value::Integer(n) => (n as f32).clamp(4.0, 200.0),
                    Value::Number(n) => (n as f32).clamp(4.0, 200.0),
                    Value::Nil => def.size,
                    _ => 15.0,
                };
                let colour = match t.raw_get::<Value>("color")? {
                    Value::Nil => def.colour,
                    ref c => {
                        // Drop app_data_ref before calling paint (which also acquires app_data_ref).
                        drop(cache);
                        return Ok(Style { font_idx, font: font_at(font_idx), size, colour: paint(lua, c)? });
                    }
                };
                return Ok(Style { font_idx, font: font_at(font_idx), size, colour });
            }
            _ => {}
        }
    }
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
    let font_idx = font_index(&font_name)?;
    Ok(Style { font_idx, font: font_at(font_idx), size: size.clamp(4.0, 200.0), colour })
}

/// One drawing command recorded by `draw(screen, theme)`.
#[derive(Clone, PartialEq)]
pub enum DrawOp {
    Fill { x: i32, y: i32, w: i32, h: i32, c: Paint, corner: f32 },
    Frame { x: i32, y: i32, w: i32, h: i32, c: Paint, corner: f32 },
    Triangle { x: i32, y: i32, w: i32, h: i32, dir: String, c: Paint },
    Circle { cx: f32, cy: f32, r: f32, width: Option<f32>, c: Paint, arc: Option<(f32, f32)> },
    Line { p0: (f32, f32), p1: (f32, f32), width: f32, c: Paint },
    Text { font_idx: usize, size: f32, colour: Paint, x: i32, y: i32, max_w: i32, text: String },
    Image { x: i32, y: i32, w: i32, h: i32, key: String, thumb: bool, restyle: bool },
}

/// A plug-in picture's pixel in the theme: orange (the knobs) becomes `accent`, and
/// everything else is the field fading to the writing by how light the pixel was.
pub fn plug_colour(px: [u8; 4], field: [u8; 3], ink: [u8; 3], accent: [u8; 3]) -> [u8; 3] {
    let [r, g, b, _] = px;
    let orange = r > 140 && u16::from(r) > u16::from(g) + 30 && u16::from(g) > u16::from(b) + 20 && g > 50;
    if orange {
        return accent;
    }
    let light = (u16::from(r) * 3 + u16::from(g) * 6 + u16::from(b)) / 10;
    let mix = |i: usize| ((u16::from(field[i]) * (255 - light) + u16::from(ink[i]) * light) / 255) as u8;
    [mix(0), mix(1), mix(2)]
}

/// What the scripts draw into.
#[derive(Clone)]
pub struct Picture {
    pub background: [u8; 3],
    pub ops: Vec<DrawOp>,
    pub images: std::collections::HashMap<(String, bool), std::sync::Arc<Image>>,
    pub px: Vec<[u8; 4]>,
    /// The text drawn, in order (for tests).
    pub texts: Vec<String>,
    /// Plug-in pictures are redrawn with these (the field, the writing, the accent).
    pub restyle_plugs: bool,
    pub plug_field: [u8; 3],
    pub plug_ink: [u8; 3],
    pub plug_accent: [u8; 3],
}

impl Picture {
    pub fn new(background: [u8; 3]) -> Picture {
        Picture {
            background,
            ops: Vec::new(),
            images: std::collections::HashMap::new(),
            px: Vec::new(),
            texts: Vec::new(),
            restyle_plugs: false,
            plug_field: background,
            plug_ink: [255, 255, 255],
            plug_accent: [0xff, 0xa5, 0x2a],
        }
    }

    pub fn rasterize(&mut self) {
        let mut px = std::mem::take(&mut self.px);
        self.rasterize_into(&mut px);
        self.px = px;
    }

    /// Rasterize all recorded drawing operations into `out` (`WIDTH * HEIGHT` RGBA),
    /// reusing `out`'s existing allocation when already sized.
    pub fn rasterize_into(&mut self, out: &mut Vec<[u8; 4]>) {
        let [r, g, b] = self.background;
        let bg = [r, g, b, 255];
        if out.len() == WIDTH * HEIGHT {
            out.fill(bg);
        } else {
            out.clear();
            out.resize(WIDTH * HEIGHT, bg);
        }
        std::mem::swap(&mut self.px, out);
        let ops = std::mem::take(&mut self.ops);
        for op in &ops {
            match op {
                DrawOp::Fill { x, y, w, h, c, corner } => self.fill(*x, *y, *w, *h, *c, *corner),
                DrawOp::Frame { x, y, w, h, c, corner } => self.frame(*x, *y, *w, *h, *c, *corner),
                DrawOp::Triangle { x, y, w, h, dir, c } => self.triangle(*x, *y, *w, *h, dir, *c),
                DrawOp::Circle { cx, cy, r, width, c, arc } => self.circle(*cx, *cy, *r, *width, *c, *arc),
                DrawOp::Line { p0, p1, width, c } => self.line(*p0, *p1, *width, *c),
                DrawOp::Text { font_idx, size, colour, x, y, max_w, text } => {
                    let s = Style { font_idx: *font_idx, font: font_at(*font_idx), size: *size, colour: *colour };
                    self.text(&s, *x, *y, *max_w, text);
                }
                DrawOp::Image { x, y, w, h, key, thumb, restyle } => {
                    if let Some(img) = self.images.get(&(key.clone(), *thumb)).cloned() {
                        self.image(*x, *y, *w, *h, &img, *restyle);
                    }
                }
            }
        }
        self.ops = ops;
        std::mem::swap(&mut self.px, out);
    }

    pub fn into_pixels(mut self) -> Vec<[u8; 4]> {
        self.rasterize();
        self.px
    }

    #[inline]
    fn blend(&mut self, x: i32, y: i32, c: [u8; 3], a: f32) {
        if x < 0 || y < 0 || x as usize >= WIDTH || y as usize >= HEIGHT || a <= 0.0 {
            return;
        }
        if self.px.is_empty() {
            let [r, g, b] = self.background;
            self.px = vec![[r, g, b, 255]; WIDTH * HEIGHT];
        }
        let d = &mut self.px[y as usize * WIDTH + x as usize];
        if a >= 1.0 {
            *d = [c[0], c[1], c[2], 255];
            return;
        }
        let inv = 1.0 - a;
        for k in 0..3 {
            d[k] = (d[k] as f32 * inv + c[k] as f32 * a + 0.5) as u8;
        }
    }

    /// A filled rectangle; its four corner pixels at `corner` of its opacity (a little
    /// rounding).
    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: Paint, corner: f32) {
        if w <= 0 || h <= 0 || c.alpha <= 0.0 {
            return;
        }
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x.saturating_add(w)).min(WIDTH as i32), (y.saturating_add(h)).min(HEIGHT as i32));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        if c.alpha >= 1.0 && !self.px.is_empty() {
            let px = [c.rgb[0], c.rgb[1], c.rgb[2], 255];
            if corner >= 1.0 {
                for yy in y0 as usize..y1 as usize {
                    self.px[yy * WIDTH + x0 as usize..yy * WIDTH + x1 as usize].fill(px);
                }
                return;
            }
            for yy in y0..y1 {
                let edge_y = yy == y || yy == y + h - 1;
                if !edge_y {
                    self.px[yy as usize * WIDTH + x0 as usize..yy as usize * WIDTH + x1 as usize].fill(px);
                } else {
                    let mut span_x0 = x0;
                    let mut span_x1 = x1;
                    if span_x0 == x {
                        self.blend(span_x0, yy, c.rgb, corner);
                        span_x0 += 1;
                    }
                    if span_x1 - 1 == x + w - 1 && span_x1 > span_x0 {
                        self.blend(span_x1 - 1, yy, c.rgb, corner);
                        span_x1 -= 1;
                    }
                    if span_x0 < span_x1 {
                        self.px[yy as usize * WIDTH + span_x0 as usize..yy as usize * WIDTH + span_x1 as usize].fill(px);
                    }
                }
            }
            return;
        }
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
        let (x0, y0, x1, y1) = ((cx - r).floor() as i32, (cy - r).floor() as i32, (cx + r).ceil() as i32, (cy + r).ceil() as i32);
        let outer_out = (r + 0.75) * (r + 0.75);
        let outer_in = (r - 0.75).max(0.0) * (r - 0.75).max(0.0);
        let inner_in = if inner > 0.75 { (inner - 0.75) * (inner - 0.75) } else { -1.0 };
        let inner_out = if inner > 0.0 { (inner + 0.75) * (inner + 0.75) } else { -1.0 };
        let r_sq = r * r;
        let inner_sq = if inner > 0.0 { inner * inner } else { -1.0 };
        let span = arc.map(|(from, to)| (from, (to - from).rem_euclid(360.0)));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let d2 = dx * dx + dy * dy;
                if d2 > outer_out || (inner_in >= 0.0 && d2 < inner_in) {
                    continue;
                }
                if span.is_none() && r > 0.75 && d2 <= outer_in && d2 >= inner_out {
                    self.blend(x, y, c.rgb, c.alpha);
                    continue;
                }
                let mut n = 0;
                for sy in 0..4 {
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0 - cy;
                    for sx in 0..4 {
                        let px = x as f32 + (sx as f32 + 0.5) / 4.0 - cx;
                        let sd2 = px * px + py * py;
                        if sd2 <= r_sq && sd2 >= inner_sq {
                            let along = span.is_none_or(|(from, sweep)| {
                                let a = py.atan2(px).to_degrees();
                                (a - from).rem_euclid(360.0) <= sweep
                            });
                            n += along as i32;
                        }
                    }
                }
                self.blend(x, y, c.rgb, c.alpha * n as f32 / 16.0);
            }
        }
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
    pub(crate) fn record_label(&mut self, s: &Style, x: i32, y: i32, w: i32, h: i32, text: String, align: &str, valign: &str) -> i32 {
        let width = Self::width(s, &text).min(w);
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
        self.record_text(s, x0, top + ascent.round() as i32, x + w - x0, text)
    }

    fn record_text(&mut self, s: &Style, x: i32, y: i32, max_w: i32, text: String) -> i32 {
        let mut pen = 0.0f32;
        for ch in text.chars() {
            let m = s.font.metrics(ch, s.size);
            if (pen + m.advance_width) as i32 > max_w {
                break;
            }
            pen += m.advance_width;
        }
        self.ops.push(DrawOp::Text { font_idx: s.font_idx, size: s.size, colour: s.colour, x, y, max_w, text });
        pen.round() as i32
    }

    /// Text with its baseline at `y`, cut at `max_w` pixels; returns its width.
    fn text(&mut self, s: &Style, x: i32, y: i32, max_w: i32, text: &str) -> i32 {
        let lut = gamma_lut();
        let mut pen = 0.0f32;
        let mut prev = None;
        for ch in text.chars() {
            let kern = prev.and_then(|p| s.font.horizontal_kern(p, ch, s.size)).unwrap_or(0.0);
            let (m, bitmap) = cached_glyph(s.font_idx, s.font, s.size, ch);
            if (pen + kern + m.advance_width) as i32 > max_w {
                break;
            }
            pen += kern;
            let x0 = x + (pen + m.xmin as f32).round() as i32;
            let y0 = y - m.height as i32 - m.ymin;
            for row in 0..m.height {
                let row_off = row * m.width;
                for col in 0..m.width {
                    let raw = bitmap[row_off + col];
                    if raw == 0 {
                        continue;
                    }
                    self.blend(x0 + col as i32, y0 + row as i32, s.colour.rgb, lut[raw as usize] * s.colour.alpha);
                }
            }
            pen += m.advance_width;
            prev = Some(ch);
        }
        pen.round() as i32
    }

    pub(crate) fn width(s: &Style, text: &str) -> i32 {
        let mut w = 0.0f32;
        let mut prev = None;
        for ch in text.chars() {
            if let Some(p) = prev {
                w += s.font.horizontal_kern(p, ch, s.size).unwrap_or(0.0);
            }
            w += s.font.metrics(ch, s.size).advance_width;
            prev = Some(ch);
        }
        w.round() as i32
    }

    /// `img` scaled to fit `w` × `h` at (x, y), centred, keeping its shape (each pixel
    /// the average of those it covers). `restyle` redraws it in the theme.
    fn image(&mut self, x: i32, y: i32, w: i32, h: i32, img: &Image, restyle: bool) {
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
                let rgb =
                    if restyle { plug_colour(avg, self.plug_field, self.plug_ink, self.plug_accent) } else { [avg[0], avg[1], avg[2]] };
                self.blend(ox + dx, oy + dy, rgb, f32::from(avg[3]) / 255.0);
            }
        }
    }

    /// The "loading" overlay: both screens dimmed, with LOADING and the name centred on each.
    pub fn loading(&mut self, name: &str) {
        let font_idx = font_index("ubuntu").expect("built in");
        let white = Paint { rgb: [255; 3], alpha: 1.0 };
        let big = Style { font_idx, font: font_at(font_idx), size: 30.0, colour: white };
        let small = Style { font_idx, font: font_at(font_idx), size: 16.0, colour: Paint { rgb: [200; 3], alpha: 1.0 } };
        for screen in 0..2 {
            let x0 = screen * (WIDTH as i32 / 2);
            let w = WIDTH as i32 / 2;
            self.ops.push(DrawOp::Fill { x: x0, y: 0, w, h: HEIGHT as i32, c: Paint { rgb: [0, 0, 0], alpha: 0.78 }, corner: 1.0 });
            let title = "LOADING…";
            #[cfg(test)]
            {
                self.texts.push(title.to_string());
                self.texts.push(name.to_string());
            }
            let tw = Self::width(&big, title).min(w - 20);
            self.record_text(&big, x0 + (w - tw) / 2, HEIGHT as i32 / 2, w - 20, title.to_string());
            let nw = Self::width(&small, name).min(w - 40);
            self.record_text(&small, x0 + (w - nw) / 2, HEIGHT as i32 / 2 + 30, w - 40, name.to_string());
        }
    }

    /// A script error, over the bottom of the left screen.
    pub fn error(&mut self, message: &str) {
        self.ops.push(DrawOp::Fill {
            x: 0,
            y: HEIGHT as i32 - 30,
            w: 480,
            h: 30,
            c: Paint { rgb: [0x8c, 0x1e, 0x1e], alpha: 1.0 },
            corner: 1.0,
        });
        let font_idx = font_index("ubuntu").expect("built in");
        let s = Style { font_idx, font: font_at(font_idx), size: 14.0, colour: Paint { rgb: [255; 3], alpha: 1.0 } };
        self.record_text(&s, 8, HEIGHT as i32 - 10, 464, message.to_string());
    }
}

impl UserData for Picture {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("size", |_, _, ()| Ok((WIDTH, HEIGHT)));
        m.add_method_mut("rect", |lua, p, (x, y, w, h, c, corner): (i32, i32, i32, i32, Value, Option<f32>)| {
            let c = paint(lua, &c)?;
            p.ops.push(DrawOp::Fill { x, y, w, h, c, corner: corner.unwrap_or(1.0) });
            Ok(())
        });
        m.add_method_mut("frame", |lua, p, (x, y, w, h, c, corner): (i32, i32, i32, i32, Value, Option<f32>)| {
            let c = paint(lua, &c)?;
            p.ops.push(DrawOp::Frame { x, y, w, h, c, corner: corner.unwrap_or(1.0) });
            Ok(())
        });
        m.add_method_mut("triangle", |lua, p, (x, y, w, h, dir, c): (i32, i32, i32, i32, String, Value)| {
            let c = paint(lua, &c)?;
            p.ops.push(DrawOp::Triangle { x, y, w, h, dir, c });
            Ok(())
        });
        m.add_method_mut(
            "circle",
            |lua, p, (cx, cy, r, c, width, from, to): (f32, f32, f32, Value, Option<f32>, Option<f32>, Option<f32>)| {
                let c = paint(lua, &c)?;
                p.ops.push(DrawOp::Circle { cx, cy, r, width, c, arc: from.zip(to) });
                Ok(())
            },
        );
        m.add_method_mut("line", |lua, p, (x0, y0, x1, y1, width, c): (f32, f32, f32, f32, f32, Value)| {
            let c = paint(lua, &c)?;
            p.ops.push(DrawOp::Line { p0: (x0, y0), p1: (x1, y1), width, c });
            Ok(())
        });
        m.add_method_mut(
            "label",
            |lua, p, (x, y, w, h, text, st, align, valign): (i32, i32, i32, i32, String, Value, Option<String>, Option<String>)| {
                let s = style(lua, &st)?;
                #[cfg(test)]
                p.texts.push(text.clone());
                Ok(p.record_label(&s, x, y, w, h, text, align.as_deref().unwrap_or("center"), valign.as_deref().unwrap_or("center")))
            },
        );
        m.add_method_mut("text", |lua, p, (x, y, text, st, max_w): (i32, i32, String, Value, Option<i32>)| {
            let s = style(lua, &st)?;
            #[cfg(test)]
            p.texts.push(text.clone());
            Ok(p.record_text(&s, x, y, max_w.unwrap_or(WIDTH as i32), text))
        });
        // image(x, y, w, h, key, thumb): a picture given to the surface, fitted into the
        // box; false if there's none by that key.
        m.add_method_mut("image", |lua, p, (x, y, w, h, key, thumb): (i32, i32, i32, i32, String, Option<bool>)| {
            let Some(pictures) = lua.app_data_ref::<Pictures>().map(|p| p.clone()) else { return Ok(false) };
            let pictures = pictures.borrow();
            let thumb = thumb.unwrap_or(true);
            let Some(img) = pictures.get(&(key.clone(), thumb)).cloned() else {
                return Ok(false);
            };
            p.images.insert((key.clone(), thumb), img);
            // Product shots in the browser stay as they are. A plug-in's own picture is
            // redrawn in the theme: its orange knobs take the accent, the rest the field
            // and the writing.
            let restyle = p.restyle_plugs && !key.starts_with("artwork:");
            p.ops.push(DrawOp::Image { x, y, w, h, key, thumb, restyle });
            Ok(true)
        });
        m.add_method("text_width", |lua, _, (text, st): (String, Value)| Ok(Picture::width(&style(lua, &st)?, &text)));
        // widget(kind, x, y, w, h, inputs): one of the Rust widgets (`crate::widget`), drawn
        // in the box from the inputs table, in the theme's colours.
        m.add_method_mut("widget", |lua, p, (kind, x, y, w, h, inputs): (String, i32, i32, i32, i32, Table)| {
            let look = crate::widget::Look::of(lua);
            crate::widget::draw_lua(&kind, p, crate::widget::Rect { x, y, w, h }, &inputs, &look)
        });
        // pictures(knobs, touched): the pictures over a page's knobs (`crate::widget::layout`),
        // in the picture band of both screens; `touched` the knob numbers a finger rests on.
        // pictures(knobs, touched, plugin, no_right_dials, layout_only): `no_right_dials`
        // when the page draws its own picture in the right screen's band (a choice's list
        // still goes over it); `layout_only` returns the spans without drawing them.
        m.add_method_mut(
            "pictures",
            |lua, p, (knobs, touched, plugin, no_right, layout_only): (Table, Option<Table>, Option<String>, Option<bool>, Option<bool>)| {
                let look = crate::widget::Look::of(lua);
                let touched: Vec<usize> =
                    touched.map(|t| t.sequence_values::<usize>().filter_map(Result::ok).collect()).unwrap_or_default();
                let draw = !layout_only.unwrap_or(false);
                crate::widget::draw_pictures(lua, p, &knobs, &touched, plugin.as_deref(), !no_right.unwrap_or(false), draw, &look)
            },
        );
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

    /// Pad mode's page: a sound's tile on the right screen, and a group's on the left,
    /// shows its level as two slim bars (left, right) up its right edge, each taller the
    /// louder its side is, and nothing while it's silent.
    #[test]
    fn tiles_show_their_levels_in_stereo() {
        let (mut s, mut v) = (Surface::new(), view());
        s.event(&deckard_mk3::Event::Button("BUTTON_PAD_MODE", true), &v, std::time::Instant::now());
        let (_, silent) = s.screens(&v).expect("a picture");
        assert_eq!(s.error(), None);
        let changed =
            |px: &[[u8; 4]], x: usize, y0: usize, y1: usize| (y0..y1).filter(|&y| px[y * WIDTH + x] != silent[y * WIDTH + x]).count();
        // Pad 2's tile: x 603..718, y 213..267; its bars x 709..711 and 712..714.
        v.groups[0].sounds[1].mix.meter = (1.0, 0.03);
        // The second group's: x 123..238, y 63..131; its bars x 229..231 and 232..234.
        v.groups[1].mix.meter = (0.03, 1.0);
        let (_, px) = s.screens(&v).expect("a new picture");
        assert_eq!(s.error(), None);
        let (l, r) = (changed(&px, 709, 213, 267), changed(&px, 712, 213, 267));
        assert!(r > 0 && l > r && l < 44, "the sound's: left {l}, right {r}");
        assert_eq!((changed(&px, 710, 213, 267), changed(&px, 713, 213, 267)), (l, r), "2 px wide each");
        assert_eq!(changed(&px, 711, 213, 267), 0, "a gap between them");
        let (l, r) = (changed(&px, 229, 63, 131), changed(&px, 232, 63, 131));
        assert!(l > 0 && r > l && r < 58, "the group's: left {l}, right {r}");
        let bars = |x: usize| (709..714).contains(&x) || (229..234).contains(&x);
        let elsewhere = (0..WIDTH * HEIGHT).filter(|&i| px[i] != silent[i] && !bars(i % WIDTH)).count();
        assert_eq!(elsewhere, 0, "only the bars change");
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

    #[test]
    fn text_gamma_boosts_antialiased_edges() {
        let font_idx = font_index("roboto-medium").unwrap();
        let f = font_at(font_idx);
        let s = Style { font_idx, font: f, size: 14.0, colour: Paint { rgb: [255; 3], alpha: 1.0 } };
        let (_, bitmap) = f.rasterize('O', s.size);
        let mid = *bitmap.iter().find(|&&b| b > 32 && b < 200).expect("an antialiased edge pixel");
        let mut p = Picture::new([0; 3]);
        assert_eq!(p.record_text(&s, 10, 24, 200, "O".into()), Picture::width(&s, "O"));
        p.rasterize();
        let expected = ((mid as f32 / 255.0).powf(1.0 / 1.45) * 255.0).round() as u8;
        assert!(expected > mid, "gamma-corrected edge pixel ({expected}) is brighter than raw linear coverage ({mid})");
        assert!(p.px.iter().any(|px| px[0] == expected), "drawn with gamma-corrected coverage");
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
