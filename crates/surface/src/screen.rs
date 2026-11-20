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
pub const FONTS: [(&str, &[u8]); 14] = [
    ("ubuntu", epaint_default_fonts::UBUNTU_LIGHT),
    ("hack", epaint_default_fonts::HACK_REGULAR),
    ("plex", include_bytes!("../fonts/IBMPlexSans-Regular.ttf")),
    ("plex-bold", include_bytes!("../fonts/IBMPlexSans-Bold.ttf")),
    ("plex-mono", include_bytes!("../fonts/IBMPlexMono-Regular.ttf")),
    // (The same, a weight heavier: for a small screen.)
    ("plex-medium", include_bytes!("../fonts/IBMPlexSans-Medium.ttf")),
    ("plex-mono-medium", include_bytes!("../fonts/IBMPlexMono-Medium.ttf")),
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
    pub(crate) default_style: Style,
    pub background: [u8; 3],
    pub text: [u8; 3],
    pub accent: [u8; 3],
    pub animates: bool,
    /// The theme asks widgets for soft fills (`widgets_soft = true`).
    pub soft: bool,
    /// The knobs' names, values and dials centred in their cells (`knobs_centred = true`).
    pub centred: bool,
    /// With `centred`, the dials and the names and values at the cells' left instead
    /// (`knobs_left = true`).
    pub left: bool,
    /// The pictures' style the theme asks for (`picture_style = "deep"`), if any.
    pub picture_style: Option<crate::widget::PictureStyle>,
    /// Labels set by their capitals (`text_on_caps = true`): see [`Picture::caps`].
    pub caps: bool,
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
        let centred = theme.get::<Option<bool>>("knobs_centred").ok().flatten().unwrap_or(false);
        let left = theme.get::<Option<bool>>("knobs_left").ok().flatten().unwrap_or(false);
        let picture_style =
            theme.get::<Option<String>>("picture_style").ok().flatten().and_then(|n| crate::widget::PictureStyle::named(&n));
        let caps = theme.get::<Option<bool>>("text_on_caps").ok().flatten().unwrap_or(false);
        Ok(ThemeCache {
            colors,
            fonts,
            styles,
            default_style,
            background,
            text,
            accent,
            animates,
            soft,
            centred,
            left,
            picture_style,
            caps,
        })
    }

    /// The theme's colour `name`, if it has one.
    pub(crate) fn color(&self, name: &str) -> Option<Paint> {
        self.colors.get(name).copied()
    }

    /// Colour `name` is `rgb` from now on (opaque).
    pub fn set_colour(&mut self, name: &str, rgb: [u8; 3]) {
        self.colors.insert(name.to_string(), Paint { rgb, alpha: 1.0 });
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
    /// The opaque background that starts a spectrum widget's clipped drawing.
    SpectrumPanel {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        c: Paint,
    },
    Fill {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        c: Paint,
        corner: f32,
    },
    Frame {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        c: Paint,
        corner: f32,
    },
    Triangle {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        dir: String,
        c: Paint,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        width: Option<f32>,
        c: Paint,
        arc: Option<(f32, f32)>,
    },
    Line {
        p0: (f32, f32),
        p1: (f32, f32),
        width: f32,
        c: Paint,
    },
    /// A stroke `width` thick along the circle of radius `r` about `centre`, from angle
    /// `a0` to `a1` (degrees, counter-clockwise from 3 o'clock, y up), with round ends.
    Arc {
        centre: (f32, f32),
        r: f32,
        a0: f32,
        a1: f32,
        width: f32,
        c: Paint,
    },
    Text {
        font_idx: usize,
        size: f32,
        colour: Paint,
        x: i32,
        y: i32,
        max_w: i32,
        text: String,
    },
    Image {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        key: String,
        thumb: bool,
        restyle: bool,
    },
    /// From here on, draw only inside (x0, y0)-(x1, y1) (a widget's box), or anywhere.
    Clip(Option<(i32, i32, i32, i32)>),
    /// The box redrawn as a dot-matrix display: `cell`-pixel dots with a pixel's gap,
    /// each lit to the brightest of the pixels it covers, unlit ones `ghost`.
    Dots {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        cell: i32,
        ghost: [u8; 3],
    },
    /// A box filled opaque, `top`'s colour at its top row fading to `bottom`'s at its last.
    Gradient {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        top: [u8; 3],
        bottom: [u8; 3],
    },
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
    /// Labels placed in their boxes by their capitals rather than by the font's ascent
    /// and descent: any font's capitals where Roboto's are (a font with a taller ascent,
    /// IBM Plex, otherwise sits lower in the same box).
    pub caps: bool,
    /// Where drawing is allowed now ([`DrawOp::Clip`]).
    clip: Option<(i32, i32, i32, i32)>,
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
            caps: false,
            clip: None,
        }
    }

    pub fn rasterize(&mut self) {
        let mut px = std::mem::take(&mut self.px);
        self.rasterize_into(&mut px);
        self.px = px;
    }

    /// Rasterize a complete picture, reusing the RGBA buffer.
    pub fn rasterize_into(&mut self, out: &mut Vec<[u8; 4]>) {
        let [r, g, b] = self.background;
        let bg = [r, g, b, 255];
        if out.len() == WIDTH * HEIGHT {
            out.fill(bg);
        } else {
            out.clear();
            out.resize(WIDTH * HEIGHT, bg);
        }
        self.replay(out, 0, None);
    }

    /// Update only the spectrum's widget box if everything outside it is identical
    /// to the last picture. Pixels outside the returned box in `out` are unspecified.
    pub fn rasterize_spectrum_update(&mut self, previous: &Picture, out: &mut Vec<[u8; 4]>) -> Option<(usize, usize, usize, usize)> {
        let (start, end, region) = self.spectrum_span()?;
        let (old_start, old_end, old_region) = previous.spectrum_span()?;
        if region != old_region
            || self.background != previous.background
            || (self.plug_field, self.plug_ink, self.plug_accent) != (previous.plug_field, previous.plug_ink, previous.plug_accent)
            || self.ops[..start] != previous.ops[..old_start]
            || self.ops[end..] != previous.ops[old_end..]
            || self.images.len() != previous.images.len()
            || self.images.iter().any(|(key, image)| previous.images.get(key).is_none_or(|old| !std::sync::Arc::ptr_eq(image, old)))
            || self.ops[start..].iter().any(|op| matches!(op, DrawOp::Dots { .. }))
        {
            return None;
        }
        out.resize(WIDTH * HEIGHT, [0, 0, 0, 255]);
        self.replay(out, start, Some(region));
        let (x0, y0, x1, y1) = region;
        Some((x0 as usize, y0 as usize, x1 as usize, y1 as usize))
    }

    fn spectrum_span(&self) -> Option<(usize, usize, (i32, i32, i32, i32))> {
        let start = self.ops.iter().position(|op| matches!(op, DrawOp::SpectrumPanel { .. }))?;
        let DrawOp::SpectrumPanel { x, y, w, h, c } = self.ops[start] else { return None };
        let region = (x, y, x.saturating_add(w), y.saturating_add(h));
        if c.alpha != 1.0
            || x < 0
            || y < 0
            || w <= 0
            || h <= 0
            || region.2 > WIDTH as i32
            || region.3 > HEIGHT as i32
            || self.ops.get(start.checked_sub(1)?) != Some(&DrawOp::Clip(Some(region)))
        {
            return None;
        }
        let end = start + 1 + self.ops[start + 1..].iter().position(|op| matches!(op, DrawOp::Clip(_)))?;
        (self.ops[end] == DrawOp::Clip(None)).then_some((start, end, region))
    }

    fn replay(&mut self, out: &mut Vec<[u8; 4]>, start: usize, region: Option<(i32, i32, i32, i32)>) {
        std::mem::swap(&mut self.px, out);
        let ops = std::mem::take(&mut self.ops);
        self.clip = region;
        for op in &ops[start..] {
            match op {
                DrawOp::Clip(c) => {
                    self.clip = match (region, *c) {
                        (Some((x0, y0, x1, y1)), Some((a, b, c, d))) => Some((x0.max(a), y0.max(b), x1.min(c), y1.min(d))),
                        _ => region.or(*c),
                    };
                }
                DrawOp::SpectrumPanel { x, y, w, h, c } => self.fill(*x, *y, *w, *h, *c, 1.0),
                DrawOp::Dots { x, y, w, h, cell, ghost } => self.dots(*x, *y, *w, *h, *cell, *ghost),
                DrawOp::Gradient { x, y, w, h, top, bottom } => {
                    // (Smooth here; the whole picture is dithered to the MK3's 5-6-5 levels on
                    // its way to the displays: `deckard_mk3::display::rgb565_dithered`.)
                    for row in 0..*h {
                        let t = if *h > 1 { row as f32 / (*h - 1) as f32 } else { 0.0 };
                        let v = |k: usize| f32::from(top[k]) + (f32::from(bottom[k]) - f32::from(top[k])) * t;
                        let (r, g, b) = (v(0), v(1), v(2));
                        let py = *y + row;
                        let q = |v: f32| v.round().clamp(0.0, 255.0) as u8;
                        self.fill(*x, py, *w, 1, Paint { rgb: [q(r), q(g), q(b)], alpha: 1.0 }, 1.0);
                    }
                }
                DrawOp::Fill { x, y, w, h, c, corner } => self.fill(*x, *y, *w, *h, *c, *corner),
                DrawOp::Frame { x, y, w, h, c, corner } => self.frame(*x, *y, *w, *h, *c, *corner),
                DrawOp::Triangle { x, y, w, h, dir, c } => self.triangle(*x, *y, *w, *h, dir, *c),
                DrawOp::Circle { cx, cy, r, width, c, arc } => self.circle(*cx, *cy, *r, *width, *c, *arc),
                DrawOp::Line { p0, p1, width, c } => self.line(*p0, *p1, *width, *c),
                DrawOp::Arc { centre, r, a0, a1, width, c } => self.arc(*centre, *r, *a0, *a1, *width, *c),
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
        self.clip = None;
        std::mem::swap(&mut self.px, out);
    }

    /// [`DrawOp::Dots`].
    fn dots(&mut self, x: i32, y: i32, w: i32, h: i32, cell: i32, ghost: [u8; 3]) {
        if cell < 2 || self.px.is_empty() {
            return;
        }
        let bg = self.background;
        let at = |px: &Vec<[u8; 4]>, xx: i32, yy: i32| px[yy as usize * WIDTH + xx as usize];
        let (x1, y1) = ((x + w).min(WIDTH as i32), (y + h).min(HEIGHT as i32));
        let mut cy = y.max(0);
        while cy + cell <= y1 {
            let mut cx = x.max(0);
            while cx + cell <= x1 {
                // The brightest pixel under the dot, so a thin line still lights it.
                let mut best = [bg[0], bg[1], bg[2], 255];
                let mut lum = -1i32;
                for yy in cy..cy + cell {
                    for xx in cx..cx + cell {
                        let p = at(&self.px, xx, yy);
                        let l = i32::from(p[0]) * 3 + i32::from(p[1]) * 6 + i32::from(p[2]);
                        if l > lum && p[..3] != bg {
                            lum = l;
                            best = p;
                        }
                    }
                }
                let lit = best[..3] != bg;
                for yy in cy..cy + cell {
                    for xx in cx..cx + cell {
                        let gap = xx == cx + cell - 1 || yy == cy + cell - 1;
                        let c = if gap {
                            bg
                        } else if lit {
                            [best[0], best[1], best[2]]
                        } else {
                            ghost
                        };
                        self.px[yy as usize * WIDTH + xx as usize] = [c[0], c[1], c[2], 255];
                    }
                }
                cx += cell;
            }
            cy += cell;
        }
    }

    pub fn into_pixels(mut self) -> Vec<[u8; 4]> {
        self.rasterize();
        self.px
    }

    fn blend(&mut self, x: i32, y: i32, c: [u8; 3], a: f32) {
        if x < 0 || y < 0 || x as usize >= WIDTH || y as usize >= HEIGHT || a <= 0.0 {
            return;
        }
        if let Some((x0, y0, x1, y1)) = self.clip
            && (x < x0 || y < y0 || x >= x1 || y >= y1)
        {
            return;
        }
        if self.px.is_empty() {
            let [r, g, b] = self.background;
            self.px = vec![[r, g, b, 255]; WIDTH * HEIGHT];
        }
        let d = &mut self.px[y as usize * WIDTH + x as usize];
        // (Drawn over opaquely, a picture's pixel is no longer the picture's.)
        if a >= 1.0 {
            *d = [c[0], c[1], c[2], 255];
            return;
        }
        for k in 0..3 {
            d[k] = (d[k] as f32 * (1.0 - a) + c[k] as f32 * a).round() as u8;
        }
    }

    /// Marks the pixel at (`x`, `y`) as a picture's ([`PICTURE_ALPHA`]), where drawing reaches.
    fn mark_picture(&mut self, x: i32, y: i32) {
        let inside = x >= 0 && y >= 0 && (x as usize) < WIDTH && (y as usize) < HEIGHT;
        let clipped = self.clip.is_some_and(|(x0, y0, x1, y1)| x < x0 || y < y0 || x >= x1 || y >= y1);
        if inside
            && !clipped
            && let Some(d) = self.px.get_mut(y as usize * WIDTH + x as usize)
        {
            d[3] = PICTURE_ALPHA;
        }
    }

    /// A filled rectangle; its four corner pixels at `corner` of its opacity (a little
    /// rounding).
    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: Paint, corner: f32) {
        let (mut x0, mut y0) = (x.max(0), y.max(0));
        let (mut x1, mut y1) = ((x.saturating_add(w)).min(WIDTH as i32), (y.saturating_add(h)).min(HEIGHT as i32));
        if let Some((cx0, cy0, cx1, cy1)) = self.clip {
            (x0, y0, x1, y1) = (x0.max(cx0), y0.max(cy0), x1.min(cx1), y1.min(cy1));
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
        let (cx0, cy0, cx1, cy1) = self.clip.unwrap_or((0, 0, WIDTH as i32, HEIGHT as i32));
        if cx0 >= cx1 || cy0 >= cy1 {
            return;
        }
        for y in (y0.floor() as i32).max(cy0)..=(y1.ceil() as i32).min(cy1 - 1) {
            for x in (x0.floor() as i32).max(cx0)..=(x1.ceil() as i32).min(cx1 - 1) {
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

    /// A stroke along an arc ([`DrawOp::Arc`]), in one pass: each pixel covered by how
    /// far its centre lies inside the stroke's edge (an exact edge, no seams or beads
    /// where pieces would meet).
    fn arc(&mut self, (cx, cy): (f32, f32), r: f32, a0: f32, a1: f32, width: f32, c: Paint) {
        let h = width / 2.0;
        let (lo, sweep) = (a0.min(a1), (a1 - a0).abs().min(360.0));
        let end = |deg: f32| {
            let a = deg.to_radians();
            (cx + r * a.cos(), cy - r * a.sin())
        };
        let (e0, e1) = (end(a0), end(a1));
        let reach = r + h + 1.0;
        for y in (cy - reach).floor() as i32..=(cy + reach).ceil() as i32 {
            for x in (cx - reach).floor() as i32..=(cx + reach).ceil() as i32 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let (dx, dy) = (px - cx, cy - py);
                let along = (dy.atan2(dx).to_degrees() - lo).rem_euclid(360.0) <= sweep;
                let d = if along {
                    ((dx * dx + dy * dy).sqrt() - r).abs()
                } else {
                    let to = |(ex, ey): (f32, f32)| ((px - ex).powi(2) + (py - ey).powi(2)).sqrt();
                    to(e0).min(to(e1))
                };
                let cover = (h + 0.5 - d).clamp(0.0, 1.0);
                if cover > 0.0 {
                    self.blend(x, y, c.rgb, c.alpha * cover);
                }
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
        let (ascent, descent) = if self.caps {
            // (Roboto's line, in ems: ascent, descent and cap height; this font's capitals
            // standing where its would.)
            const ASCENT: f32 = 1900.0 / 2048.0;
            const DESCENT: f32 = -500.0 / 2048.0;
            const CAP: f32 = 1456.0 / 2048.0;
            let h = s.font.metrics('H', s.size).bounds;
            (s.size * (ASCENT - CAP) + h.height + h.ymin, s.size * DESCENT)
        } else {
            s.font.horizontal_line_metrics(s.size).map_or((s.size, 0.0), |m| (m.ascent, m.descent))
        };
        let text_h = (ascent - descent).round() as i32;
        let top = match valign {
            "top" => y,
            "bottom" => y + h - text_h,
            _ => y + (h - text_h) / 2,
        };
        let baseline = if self.caps && valign == "center" {
            // (Set by its capitals: those in the middle of the box.)
            let cap = s.font.metrics('H', s.size).bounds;
            y + ((h as f32 + cap.height + cap.ymin) / 2.0).round() as i32
        } else {
            top + ascent.round() as i32
        };
        self.record_text(s, x0, baseline, x + w - x0, text)
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
        let mut pen = 0.0f32;
        let mut prev = None;
        for ch in text.chars() {
            let kern = prev.and_then(|p| s.font.horizontal_kern(p, ch, s.size)).unwrap_or(0.0);
            let (m, bitmap) = s.font.rasterize(ch, s.size);
            if (pen + kern + m.advance_width) as i32 > max_w {
                break;
            }
            pen += kern;
            let x0 = x + (pen + m.xmin as f32).round() as i32;
            let y0 = y - m.height as i32 - m.ymin;
            for row in 0..m.height {
                for col in 0..m.width {
                    let cov = bitmap[row * m.width + col] as f32 / 255.0;
                    let a = if cov > 0.0 { cov.powf(1.0 / 1.45) } else { 0.0 };
                    self.blend(x0 + col as i32, y0 + row as i32, s.colour.rgb, a * s.colour.alpha);
                }
            }
            pen += m.advance_width;
            prev = Some(ch);
        }
        pen.round() as i32
    }

    /// Where `text`'s ink ends, from where it starts, as [`Picture::text`] places its
    /// letters (kerned, each at its rounded place).
    pub(crate) fn ink_end(s: &Style, text: &str) -> i32 {
        let (mut pen, mut end, mut prev) = (0.0f32, 0, None);
        for ch in text.chars() {
            pen += prev.and_then(|p| s.font.horizontal_kern(p, ch, s.size)).unwrap_or(0.0);
            let m = s.font.metrics(ch, s.size);
            if m.width > 0 {
                end = end.max((pen + m.xmin as f32).round() as i32 + m.width as i32);
            }
            pen += m.advance_width;
            prev = Some(ch);
        }
        end
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

    /// `img` scaled to fit `w` × `h` at (x, y), centred, keeping its shape: at its own
    /// size pixel for pixel, else resampled with Spline36 (widened to the shrink, so a
    /// smaller copy stays sharp without aliasing). `restyle` redraws it in the theme.
    fn image(&mut self, x: i32, y: i32, w: i32, h: i32, img: &Image, restyle: bool) {
        if img.w == 0 || img.h == 0 || w <= 0 || h <= 0 {
            return;
        }
        let k = (w as f32 / img.w as f32).min(h as f32 / img.h as f32);
        let (dw, dh) = (((img.w as f32 * k).round() as i32).max(1), ((img.h as f32 * k).round() as i32).max(1));
        let (ox, oy) = (x + (w - dw) / 2, y + (h - dh) / 2);
        if self.clip.is_some_and(|(x0, y0, x1, y1)| ox >= x1 || oy >= y1 || ox + dw <= x0 || oy + dh <= y0) {
            return;
        }
        let px = resample(img, dw as usize, dh as usize);
        for dy in 0..dh {
            for dx in 0..dw {
                let p = px[dy as usize * dw as usize + dx as usize];
                let rgb = if restyle { plug_colour(p, self.plug_field, self.plug_ink, self.plug_accent) } else { [p[0], p[1], p[2]] };
                self.blend(ox + dx, oy + dy, rgb, f32::from(p[3]) / 255.0);
                // (Mostly the picture: marked to stay sharp on the displays, until something
                // is drawn over it.)
                if p[3] >= 128 {
                    self.mark_picture(ox + dx, oy + dy);
                }
            }
        }
    }

    /// The "loading" overlay: both screens dimmed, with LOADING and the name centred on each,
    /// in the theme's font (`theme`: its default style; without a theme, Ubuntu).
    pub fn loading(&mut self, name: &str, theme: Option<Style>) {
        let (font_idx, font) = match theme {
            Some(s) => (s.font_idx, s.font),
            None => {
                let i = font_index("ubuntu").expect("built in");
                (i, font_at(i))
            }
        };
        let white = Paint { rgb: [255; 3], alpha: 1.0 };
        let big = Style { font_idx, font, size: 30.0, colour: white };
        let small = Style { font_idx, font, size: 16.0, colour: Paint { rgb: [200; 3], alpha: 1.0 } };
        for screen in 0..2 {
            let x0 = screen * (WIDTH as i32 / 2);
            let w = WIDTH as i32 / 2;
            self.ops.push(DrawOp::Fill { x: x0, y: 0, w, h: HEIGHT as i32, c: Paint { rgb: [0, 0, 0], alpha: 0.78 }, corner: 1.0 });
            let title = "LOADING…";
            self.texts.push(title.to_string());
            self.texts.push(name.to_string());
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
                p.texts.push(text.clone());
                Ok(p.record_label(&s, x, y, w, h, text, align.as_deref().unwrap_or("center"), valign.as_deref().unwrap_or("center")))
            },
        );
        m.add_method_mut("text", |lua, p, (x, y, text, st, max_w): (i32, i32, String, Value, Option<i32>)| {
            let s = style(lua, &st)?;
            p.texts.push(text.clone());
            Ok(p.record_text(&s, x, y, max_w.unwrap_or(WIDTH as i32), text))
        });
        // image(x, y, w, h, key, thumb, real, crop): a picture given to the surface, fitted
        // into the box; false if there's none by that key. `real`: in its own colours.
        // `crop`: at its own size from its top left, cut to the box (a window onto it,
        // sharp, where shrinking it would smudge it).
        #[allow(clippy::type_complexity)]
        m.add_method_mut(
            "image",
            |lua, p, (x, y, w, h, key, thumb, real, crop): (i32, i32, i32, i32, String, Option<bool>, Option<bool>, Option<bool>)| {
                let Some(pictures) = lua.app_data_ref::<Pictures>().map(|p| p.clone()) else { return Ok(false) };
                let pictures = pictures.borrow();
                let thumb = thumb.unwrap_or(true);
                let Some(img) = pictures.get(&(key.clone(), thumb)).cloned() else {
                    return Ok(false);
                };
                let (iw, ih) = (img.w as i32, img.h as i32);
                p.images.insert((key.clone(), thumb), img);
                // Product shots in the browser stay as they are. A plug-in's own picture is
                // redrawn in the theme: its orange knobs take the accent, the rest the field
                // and the writing.
                let restyle = p.restyle_plugs && !key.starts_with("artwork:") && real != Some(true);
                if crop == Some(true) {
                    p.ops.push(DrawOp::Clip(Some((x, y, x + w, y + h))));
                    p.ops.push(DrawOp::Image { x, y, w: iw, h: ih, key, thumb, restyle });
                    p.ops.push(DrawOp::Clip(None));
                } else {
                    p.ops.push(DrawOp::Image { x, y, w, h, key, thumb, restyle });
                }
                Ok(true)
            },
        );
        // image_size(key, thumb): a picture's own width and height, or nothing if there's
        // none by that key.
        m.add_method("image_size", |lua, _, (key, thumb): (String, Option<bool>)| {
            let Some(pictures) = lua.app_data_ref::<Pictures>().map(|p| p.clone()) else { return Ok((None, None)) };
            let pictures = pictures.borrow();
            let size = pictures.get(&(key, thumb.unwrap_or(true))).map(|img| (img.w as i32, img.h as i32));
            Ok((size.map(|s| s.0), size.map(|s| s.1)))
        });
        m.add_method("text_width", |lua, _, (text, st): (String, Value)| Ok(Picture::width(&style(lua, &st)?, &text)));
        // text_bearing(text, style): the space after the last letter's ink, within its
        // width (pixels; to set the ink, not the width, against an edge).
        m.add_method("text_bearing", |lua, _, (text, st): (String, Value)| {
            let s = style(lua, &st)?;
            Ok(Picture::width(&s, &text) - Picture::ink_end(&s, &text))
        });
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

/// The alpha of a pixel in a finished picture that is a picture's (a plug-in's own, say),
/// rather than 255: the displays show it rounded to their nearest levels, not dithered
/// (dithering's grain only muddies a photo's fine detail).
pub const PICTURE_ALPHA: u8 = 254;

/// The Spline36 kernel (6 taps): sharper than bicubic, with little ringing.
fn spline36(x: f32) -> f32 {
    let x = x.abs();
    if x < 1.0 {
        ((13.0 / 11.0 * x - 453.0 / 209.0) * x - 3.0 / 209.0) * x + 1.0
    } else if x < 2.0 {
        let x = x - 1.0;
        ((-6.0 / 11.0 * x + 270.0 / 209.0) * x - 156.0 / 209.0) * x
    } else if x < 3.0 {
        let x = x - 2.0;
        ((1.0 / 11.0 * x - 45.0 / 209.0) * x + 26.0 / 209.0) * x
    } else {
        0.0
    }
}

/// For each of `out` pixels along an axis of `len`: the first source pixel and the
/// weights from it (summing to 1).
fn spline36_taps(len: usize, out: usize) -> Vec<(usize, Vec<f32>)> {
    let k = out as f32 / len as f32;
    // (Shrinking, the kernel widens by as much, to take in every pixel it covers.)
    let s = (1.0 / k).max(1.0);
    let r = 3.0 * s;
    (0..out)
        .map(|o| {
            let c = (o as f32 + 0.5) / k - 0.5;
            let lo = ((c - r).ceil().max(0.0)) as usize;
            let hi = ((c + r).floor() as usize).min(len - 1);
            let mut w: Vec<f32> = (lo..=hi).map(|i| spline36((i as f32 - c) / s)).collect();
            let sum: f32 = w.iter().sum();
            if sum.abs() > 1e-6 {
                w.iter_mut().for_each(|v| *v /= sum);
            }
            (lo, w)
        })
        .collect()
}

/// `img` at `dw` × `dh`: itself when that's its size, else Spline36 in each direction
/// on premultiplied colour (so a clear edge doesn't darken).
fn resample(img: &Image, dw: usize, dh: usize) -> Vec<[u8; 4]> {
    if dw == img.w && dh == img.h {
        return img.px.clone();
    }
    let pre = |p: [u8; 4]| {
        let a = f32::from(p[3]) / 255.0;
        [f32::from(p[0]) * a, f32::from(p[1]) * a, f32::from(p[2]) * a, f32::from(p[3])]
    };
    let (xs, ys) = (spline36_taps(img.w, dw), spline36_taps(img.h, dh));
    let mut rows = vec![[0f32; 4]; img.h * dw];
    for sy in 0..img.h {
        for (dx, (lo, w)) in xs.iter().enumerate() {
            let mut acc = [0f32; 4];
            for (j, wt) in w.iter().enumerate() {
                let p = pre(img.px[sy * img.w + lo + j]);
                for c in 0..4 {
                    acc[c] += p[c] * wt;
                }
            }
            rows[sy * dw + dx] = acc;
        }
    }
    let mut out = vec![[0u8; 4]; dw * dh];
    for (dy, (lo, w)) in ys.iter().enumerate() {
        for dx in 0..dw {
            let mut acc = [0f32; 4];
            for (j, wt) in w.iter().enumerate() {
                let p = rows[(lo + j) * dw + dx];
                for c in 0..4 {
                    acc[c] += p[c] * wt;
                }
            }
            let a = acc[3].clamp(0.0, 255.0);
            let un = |v: f32| if a > 0.0 { (v * 255.0 / a).round().clamp(0.0, 255.0) as u8 } else { 0 };
            out[dy * dw + dx] = [un(acc[0]), un(acc[1]), un(acc[2]), a.round() as u8];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::view;
    use crate::{GroupView, SoundView, Surface, View};

    /// `DECKARD_RESAMPLE=in.png,out.png,w,h`: `in.png` resampled as the screens do,
    /// on black (`--ignored`; to look at).
    #[test]
    #[ignore]
    fn resample_a_png() {
        let Ok(spec) = std::env::var("DECKARD_RESAMPLE") else { return };
        let a: Vec<&str> = spec.split(',').collect();
        let mut dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(a[0]).unwrap()));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        let ch = info.color_type.samples();
        let px: Vec<[u8; 4]> = buf
            .chunks(ch)
            .take((info.width * info.height) as usize)
            .map(|p| match ch {
                4 => [p[0], p[1], p[2], p[3]],
                3 => [p[0], p[1], p[2], 255],
                2 => [p[0], p[0], p[0], p[1]],
                _ => [p[0], p[0], p[0], 255],
            })
            .collect();
        let img = Image { w: info.width as usize, h: info.height as usize, px };
        let (w, h): (usize, usize) = (a[2].parse().unwrap(), a[3].parse().unwrap());
        let out: Vec<u8> = resample(&img, w, h)
            .iter()
            .flat_map(|p| {
                let k = u16::from(p[3]);
                [p[0], p[1], p[2]].map(|c| (u16::from(c) * k / 255) as u8)
            })
            .collect();
        let mut e = png::Encoder::new(std::fs::File::create(a[1]).unwrap(), w as u32, h as u32);
        e.set_color(png::ColorType::Rgb);
        e.write_header().unwrap().write_image_data(&out).unwrap();
    }

    #[test]
    fn pictures_scale_with_spline36() {
        // At its own size a picture is itself; scaled, a flat one stays flat (the weights
        // sum to 1) and a clear edge doesn't darken what's beside it.
        let px: Vec<[u8; 4]> = (0..40 * 20).map(|i| [(i % 251) as u8, 90, 200, 255]).collect();
        let img = Image { w: 40, h: 20, px: px.clone() };
        assert_eq!(resample(&img, 40, 20), px);
        let flat = Image { w: 40, h: 20, px: vec![[120, 60, 30, 255]; 800] };
        for (w, h) in [(13, 7), (75, 38), (40, 9)] {
            assert!(resample(&flat, w, h).iter().all(|p| *p == [120, 60, 30, 255]), "{w}x{h}");
        }
        let half = Image { w: 8, h: 1, px: (0..8).map(|i| if i < 4 { [255, 255, 255, 255] } else { [0, 0, 0, 0] }).collect() };
        let out = resample(&half, 16, 1);
        assert!(out.iter().filter(|p| p[3] > 0).all(|p| p[0] == 255), "{out:?}");
        assert!((spline36(0.0) - 1.0).abs() < 1e-6 && spline36(1.0).abs() < 1e-6 && spline36(2.0).abs() < 1e-6);
        // The kernel itself: continuous at its knots, small lobes (its known values), and
        // its taps at any phase summing to 1 before normalising.
        for knot in [1.0f32, 2.0, 3.0] {
            assert!((spline36(knot - 1e-4) - spline36(knot + 1e-4)).abs() < 1e-3, "continuous at {knot}");
        }
        assert!((spline36(0.5) - 0.5987).abs() < 1e-3 && (spline36(1.5) + 0.1184).abs() < 1e-3 && (spline36(2.5) - 0.0197).abs() < 1e-3);
        for phase in [0.0f32, 0.25, 0.5] {
            let sum: f32 = (-3..=3).map(|i| spline36(i as f32 - phase)).sum();
            assert!((sum - 1.0).abs() < 1e-3, "phase {phase}: {sum}");
        }
        // A ramp shrunk stays a ramp; a fine checkerboard shrunk turns grey, not to noise.
        let ramp = Image { w: 100, h: 1, px: (0..100).map(|i| [(i * 255 / 99) as u8, 0, 0, 255]).collect() };
        let r = resample(&ramp, 80, 1);
        assert!(r.windows(2).all(|p| u16::from(p[1][0]) + 1 >= u16::from(p[0][0])), "{r:?}");
        let check =
            Image { w: 40, h: 40, px: (0..1600).map(|i| if (i % 40 + i / 40) % 2 == 0 { [255; 4] } else { [0, 0, 0, 255] }).collect() };
        let g = resample(&check, 16, 16);
        assert!(g.iter().all(|p| (90..=165).contains(&p[0])), "{g:?}");
    }

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
