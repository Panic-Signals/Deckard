//! Pictures drawn in Rust: a filter's response, an EQ's bands, an envelope's shape and
//! the rest of the widgets in `docs/superpowers/specs/2026-11-15-plugin-mappings-design.md`
//! ("Widgets"). Scripts draw one with `screen:widget(kind, x, y, w, h, inputs)`; the
//! inputs are a plain table, and each widget reads what it needs from it.
//!
//! Every widget:
//!
//! - draws only inside its box, at every width it can span: two, three or four knob
//!   cells (`CELL_W` each) by the picture band's height (`BAND_H`), and narrower spans
//!   drop secondary detail (axis labels), never the picture;
//! - has a **flat look**: lines and solid fills in the theme's colours ([`Look`]), so it
//!   reads on every theme, the two-tone `lcd` one included. No gradients or dithering;
//! - lights the part an input moves while that input has touch focus (`inputs.focus`,
//!   the input's name) and dims the rest ([`Canvas::ink`]);
//! - shows a modulated input's position (`<input>_mod`, in the input's own units) beside
//!   its base value where the picture can;
//! - has golden images ([`golden`]) on the `maschine` and `lcd` themes at each width.

use mlua::{Lua, Table, Value};

use crate::screen::{DrawOp, Paint, Picture, Style};

pub mod band;
pub mod choices;
pub mod curve;
pub mod dial;
pub mod envelope;
pub mod eq;
pub mod filter;
pub mod layout;
pub mod lfo;
pub mod mapping;
pub mod oscillators;
pub mod transfer;
pub mod value;

#[cfg(test)]
pub(crate) mod golden;
#[cfg(test)]
mod mapping_check;

/// One knob cell's width: a screen's 480 pixels over its four knobs.
pub const CELL_W: i32 = 120;
/// The picture band, the same on both screens (`widgets.BAND_Y`, `widgets.BAND_H` in
/// `lib/widgets.lua`): under the left screen's chain strip, above the knobs' section names.
pub const BAND_Y: i32 = 106;
pub const BAND_H: i32 = 94;
/// Where the knobs' names start (`widgets.params`).
pub const KNOB_NAME_Y: i32 = 233;

/// A box on the screens (960 × 272, the right screen from x = 480).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// How the pictures look as a whole, the same for every widget: how much of the
/// curve's colour fills under it (0: none, lines only), how strongly grids show, and
/// whether faint dividers part neighbouring pictures or each sits on a panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PictureStyle {
    pub fill: f32,
    pub grid: f32,
    pub dividers: bool,
    pub panels: bool,
    /// A soft halo round the picture's lines (0: none), as a lit display's.
    pub glow: f32,
    /// The picture's lines this much thicker (1: as drawn).
    pub stroke: f32,
    /// The band redrawn as a dot matrix of `dots`-pixel dots (0: off).
    pub dots: i32,
    /// Colours of its own instead of the theme's: the picture, the grid, the panel.
    pub ink: Option<[u8; 3]>,
    pub grid_ink: Option<[u8; 3]>,
    pub panel_ink: Option<[u8; 3]>,
    /// Fills under curves fade from the curve's colour at the box's top to the field.
    pub gradient: bool,
    /// A touched dial keeps its arc's width and lights up with a halo round it.
    pub focus_glow: bool,
}

impl PictureStyle {
    const BASE: PictureStyle = PictureStyle {
        fill: 0.0,
        grid: 0.4,
        dividers: false,
        panels: false,
        glow: 0.0,
        stroke: 1.0,
        dots: 0,
        ink: None,
        grid_ink: None,
        panel_ink: None,
        gradient: false,
        focus_glow: false,
    };
    /// Lines only: no fills, faint grids.
    pub const LINES: PictureStyle = PictureStyle::BASE;
    /// A faint fill under curves, dividers between pictures.
    pub const SOFT: PictureStyle = PictureStyle { fill: 0.16, dividers: true, ..PictureStyle::BASE };
    /// Lines on a panel per picture and per knob.
    pub const PANELS: PictureStyle = PictureStyle { panels: true, ..PictureStyle::BASE };
    /// Lit lines with a halo, as a backlit display's segments.
    pub const GLOW: PictureStyle = PictureStyle { glow: 1.0, fill: 0.08, grid: 0.25, ..PictureStyle::BASE };
    /// An oscilloscope: green phosphor with a halo over a graticule.
    pub const SCOPE: PictureStyle = PictureStyle {
        glow: 1.0,
        grid: 1.0,
        ink: Some([0x6c, 0xff, 0x8f]),
        grid_ink: Some([0x16, 0x3a, 0x22]),
        panel_ink: Some([0x04, 0x0c, 0x07]),
        panels: true,
        ..PictureStyle::BASE
    };
    /// A dot-matrix display: the band in chunky dots, unlit ones faint.
    pub const DOTS: PictureStyle = PictureStyle { dots: 3, stroke: 1.6, fill: 0.35, grid: 0.3, ..PictureStyle::BASE };
    /// Bold silhouettes: curves filled solid, thick lines, no grid.
    pub const SOLID: PictureStyle = PictureStyle { fill: 1.0, stroke: 2.0, grid: 0.3, ..PictureStyle::BASE };
    /// Fills that fade from the top, heavier lines: for a theme with lit colour on a
    /// dark ground.
    pub const DEEP: PictureStyle =
        PictureStyle { fill: 0.5, stroke: 1.35, grid: 0.7, gradient: true, focus_glow: true, ..PictureStyle::BASE };
    /// White hairlines on deep blue panels, a drafting sheet's grid.
    pub const BLUEPRINT: PictureStyle = PictureStyle {
        grid: 1.0,
        stroke: 0.75,
        ink: Some([0xee, 0xf4, 0xff]),
        grid_ink: Some([0x2a, 0x4e, 0x7e]),
        panel_ink: Some([0x0c, 0x23, 0x45]),
        panels: true,
        ..PictureStyle::BASE
    };
}

impl PictureStyle {
    /// The style a theme names (`picture_style`).
    pub fn named(name: &str) -> Option<PictureStyle> {
        Some(match name {
            "lines" => PictureStyle::LINES,
            "soft" => PictureStyle::SOFT,
            "panels" => PictureStyle::PANELS,
            "glow" => PictureStyle::GLOW,
            "scope" => PictureStyle::SCOPE,
            "dots" => PictureStyle::DOTS,
            "solid" => PictureStyle::SOLID,
            "deep" => PictureStyle::DEEP,
            "blueprint" => PictureStyle::BLUEPRINT,
            _ => return None,
        })
    }
}

thread_local! {
    static STYLE: std::cell::Cell<PictureStyle> = const { std::cell::Cell::new(PictureStyle::LINES) };
    static THEME_STYLE: std::cell::Cell<Option<PictureStyle>> = const { std::cell::Cell::new(None) };
}

/// The style pictures draw in (on this thread: the scripts' and the tests'): the
/// theme's own if it names one, else the one set.
pub fn style() -> PictureStyle {
    THEME_STYLE.with(|t| t.get()).unwrap_or_else(|| STYLE.with(|s| s.get()))
}

/// The style the theme drawing now names, if any (set before each frame).
pub fn set_theme_style(s: Option<PictureStyle>) {
    THEME_STYLE.with(|c| c.set(s));
}

/// Draw pictures in `s` from now on (on this thread).
pub fn set_style(s: PictureStyle) {
    STYLE.with(|c| c.set(s));
}

/// The theme's colours and type a widget draws with.
#[derive(Clone, Copy)]
pub struct Look {
    /// The screen's field.
    pub background: Paint,
    /// A plot's own field, a step off the background.
    pub panel: Paint,
    pub text: Paint,
    /// Axis labels and secondary text.
    pub weak: Paint,
    /// Grid lines.
    pub grid: Paint,
    /// The zero line, the unity diagonal.
    pub separator: Paint,
    /// Secondary curves: the range a modulation sweeps, a dimmed curve.
    pub muted: Paint,
    /// The picture itself: the curve, the shape.
    pub accent: Paint,
    /// Handles: a band's dot, the cutoff's mark, the threshold.
    pub record: Paint,
    /// Something live: a side-chain's curve while it plays, a playhead.
    pub marker: Paint,
    /// Text on a filled handle.
    pub on_colour: Paint,
    /// Axis labels (small).
    pub small: Style,
    /// Names (as the knobs' parameter names).
    pub label: Style,
    /// Large numbers (as the knobs' values).
    pub big: Style,
    /// The theme asks for soft fills (`widgets_soft = true`). Off, fills are flat.
    pub soft: bool,
    /// How much of a curve's colour fills under it ([`PictureStyle::fill`]; 0: no fill).
    pub fill: f32,
    /// The theme centres the knobs' names and values in their cells, and the dials over them.
    pub centred: bool,
    /// ... but at the cells' left, lined up with the names and values.
    pub left: bool,
}

impl Look {
    /// The theme the scripts draw with now.
    pub fn of(lua: &Lua) -> Look {
        match lua.app_data_ref::<crate::screen::ThemeCache>() {
            Some(cache) => Look::from_cache(&cache),
            None => Look::fallback(),
        }
    }

    pub(crate) fn from_cache(cache: &crate::screen::ThemeCache) -> Look {
        let f = Look::fallback();
        let c = |name: &str, or: Paint| cache.color(name).unwrap_or(or);
        let background = c("background", f.background);
        let text = c("text", f.text);
        // (A dot matrix can't draw small labels: they'd be smudges. Its pictures go without.)
        let weak = if style().dots >= 2 { background } else { c("weak", f.weak) };
        let mut small = cache.style("default");
        small.size = 10.0;
        small.colour = weak;
        let mut label = cache.style("param_name");
        label.colour = weak;
        let big = cache.style("param_value");
        Look {
            background,
            // (In the panels style, a plot's own field is the panel it sits on.)
            panel: match (style().panel_ink, style().panels) {
                (Some(p), _) => Paint { rgb: p, alpha: 1.0 },
                (None, true) => mix(background, text, 0.08),
                (None, false) => c("panel", f.panel),
            },
            text,
            weak,
            grid: match style().grid_ink {
                Some(g) => Paint { rgb: g, alpha: 1.0 },
                None => mix(background, c("button", f.grid), style().grid),
            },
            separator: c("separator", f.separator),
            muted: c("muted", f.muted),
            accent: style().ink.map_or(c("accent", f.accent), |i| Paint { rgb: i, alpha: 1.0 }),
            record: c("record", f.record),
            marker: c("marker", f.marker),
            on_colour: c("on_colour", f.on_colour),
            small,
            label,
            big,
            soft: cache.soft,
            fill: style().fill,
            centred: cache.centred,
            left: cache.left,
        }
    }

    /// Maschine's colours, for a Lua without a theme.
    pub fn fallback() -> Look {
        let p = |r: u8, g: u8, b: u8| Paint { rgb: [r, g, b], alpha: 1.0 };
        let idx = crate::screen::font_index("roboto").expect("built in");
        let font = crate::screen::font_at(idx);
        let st = |size: f32, colour: Paint| Style { font_idx: idx, font, size, colour };
        let weak = Paint { rgb: [255; 3], alpha: 0.5 };
        Look {
            background: p(0, 0, 0),
            panel: p(0x14, 0x14, 0x14),
            text: p(255, 255, 255),
            weak,
            grid: p(0x47, 0x47, 0x47),
            separator: p(0x77, 0x6f, 0x6f),
            muted: p(71, 71, 71),
            accent: p(0xff, 0xa5, 0x2a),
            record: p(0xff, 0x30, 0x30),
            marker: p(0xff, 0xff, 0xff),
            on_colour: p(0, 0, 0),
            small: st(10.0, weak),
            label: st(12.0, weak),
            big: st(21.5, p(255, 255, 255)),
            soft: false,
            fill: style().fill,
            centred: false,
            left: false,
        }
    }

    /// The fill under a curve: a dimmer step of the curve's colour toward the field,
    /// opaque, so it reads on the `lcd` theme and keeps the MK3's runs of one colour.
    pub fn fill_of(&self, c: Paint) -> Paint {
        mix(self.background, c, self.fill)
    }
}

/// `a` toward `b` by `t` (0..1), opaque. Each colour's own opacity is applied over the
/// background first, so a translucent theme colour mixes as it would draw.
pub fn mix(a: Paint, b: Paint, t: f32) -> Paint {
    let t = t.clamp(0.0, 1.0);
    let ch = |k: usize| {
        let (x, y) = (f32::from(a.rgb[k]), f32::from(b.rgb[k]));
        let y = x + (y - x) * b.alpha;
        (x + (y - x) * t).round().clamp(0.0, 255.0) as u8
    };
    Paint { rgb: [ch(0), ch(1), ch(2)], alpha: 1.0 }
}

/// Text placement in a box.
#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Drop for Canvas<'_> {
    fn drop(&mut self) {
        self.pic.ops.push(DrawOp::Clip(None));
    }
}

/// What a widget draws into: its box (`r`), the theme (`look`) and the input that has
/// touch focus, if any. **Coordinates are relative to the box's top left** (0..w,
/// 0..h); nothing may be drawn outside it (the golden tests check).
pub struct Canvas<'a> {
    pic: &'a mut Picture,
    pub r: Rect,
    pub look: &'a Look,
    /// The input lit by touch focus (its name in the widget's inputs).
    pub focus: Option<String>,
}

impl<'a> Canvas<'a> {
    pub fn new(pic: &'a mut Picture, r: Rect, look: &'a Look) -> Canvas<'a> {
        // (Nothing a widget draws leaves its box: past x = 480 it would be on the other
        // screen.)
        pic.ops.push(DrawOp::Clip(Some((r.x, r.y, r.x + r.w, r.y + r.h))));
        Canvas { pic, r, look, focus: None }
    }

    pub fn w(&self) -> f32 {
        self.r.w as f32
    }

    pub fn h(&self) -> f32 {
        self.r.h as f32
    }

    /// Whether input `name` has touch focus.
    pub fn lit(&self, name: &str) -> bool {
        self.focus.as_deref() == Some(name)
    }

    /// `c` for a part of the picture moved by `inputs`: as it is, unless another input
    /// has touch focus, then dimmed to about half toward the field. Names, values and
    /// axes don't go through this; they never dim.
    pub fn ink(&self, c: Paint, inputs: &[&str]) -> Paint {
        match &self.focus {
            Some(f) if !inputs.iter().any(|i| i == f) => mix(self.look.background, c, 0.5),
            _ => c,
        }
    }

    /// A filled rectangle.
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: Paint) {
        self.pic.ops.push(DrawOp::Fill { x: self.r.x + x, y: self.r.y + y, w, h, c, corner: 1.0 });
    }

    /// A one-pixel outline.
    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, c: Paint) {
        self.pic.ops.push(DrawOp::Frame { x: self.r.x + x, y: self.r.y + y, w, h, c, corner: 1.0 });
    }

    /// A line `width` thick. In a style with a stroke weight or a glow, a picture's own
    /// lines (1.5 px and up: grids are thinner) are drawn heavier, or with a halo.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, c: Paint) {
        let (ox, oy) = (self.r.x as f32, self.r.y as f32);
        let st = style();
        let width = if width >= 1.5 { width * st.stroke } else { width };
        if st.glow > 0.0 && width >= 1.5 {
            for (extra, a) in [(7.0, 0.12), (3.5, 0.25)] {
                let halo = Paint { rgb: c.rgb, alpha: c.alpha * a * st.glow };
                self.pic.ops.push(DrawOp::Line { p0: (ox + x0, oy + y0), p1: (ox + x1, oy + y1), width: width + extra, c: halo });
            }
        }
        self.pic.ops.push(DrawOp::Line { p0: (ox + x0, oy + y0), p1: (ox + x1, oy + y1), width, c });
    }

    /// An arc `width` thick of the circle of radius `r` about (`cx`, `cy`), from angle
    /// `a0` to `a1` (degrees, counter-clockwise from 3 o'clock), in one stroke; heavier
    /// or with a halo in a style that does that to lines, as [`Canvas::line`].
    #[allow(clippy::too_many_arguments)]
    pub fn arc(&mut self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32, width: f32, c: Paint) {
        let centre = (self.r.x as f32 + cx, self.r.y as f32 + cy);
        let st = style();
        let width = if width >= 1.5 { width * st.stroke } else { width };
        if st.glow > 0.0 && width >= 1.5 {
            for (extra, a) in [(7.0, 0.12), (3.5, 0.25)] {
                let halo = Paint { rgb: c.rgb, alpha: c.alpha * a * st.glow };
                self.pic.ops.push(DrawOp::Arc { centre, r, a0, a1, width: width + extra, c: halo });
            }
        }
        self.pic.ops.push(DrawOp::Arc { centre, r, a0, a1, width, c });
    }

    /// Lines through `pts` in order.
    pub fn polyline(&mut self, pts: &[(f32, f32)], width: f32, c: Paint) {
        for w in pts.windows(2) {
            self.line(w[0].0, w[0].1, w[1].0, w[1].1, width, c);
        }
    }

    /// A flat fill between the curve through `pts` (in x order) and the line `y = base`,
    /// one column per pixel.
    pub fn area(&mut self, pts: &[(f32, f32)], base: f32, c: Paint) {
        // (No fill in this style: the field's own colour.)
        if pts.len() < 2 || c.rgb == self.look.background.rgb {
            return;
        }
        let (x0, x1) = (pts[0].0.ceil() as i32, pts[pts.len() - 1].0.floor() as i32);
        let gradient = style().gradient;
        let mut seg = 0;
        for x in x0..=x1 {
            let xf = x as f32;
            while seg + 2 < pts.len() && pts[seg + 1].0 < xf {
                seg += 1;
            }
            let (a, b) = (pts[seg], pts[seg + 1]);
            let t = if (b.0 - a.0).abs() < 1e-6 { 0.0 } else { ((xf - a.0) / (b.0 - a.0)).clamp(0.0, 1.0) };
            let y = a.1 + (b.1 - a.1) * t;
            let (top, bottom) = if y < base { (y, base) } else { (base, y) };
            let (top, bottom) = (top.round() as i32, bottom.round() as i32);
            if bottom > top && gradient && base > 0.0 {
                // (Strongest at the box's top, fading to a trace of the colour at the base.)
                let at = |yy: i32| mix(self.look.background, c, 0.12 + 0.88 * ((base - yy as f32) / base).clamp(0.0, 1.0)).rgb;
                let (ox, oy) = (self.r.x, self.r.y);
                self.pic.ops.push(DrawOp::Gradient { x: ox + x, y: oy + top, w: 1, h: bottom - top, top: at(top), bottom: at(bottom) });
            } else if bottom > top {
                self.fill(x, top, 1, bottom - top, c);
            }
        }
    }

    /// A disc.
    pub fn dot(&mut self, cx: f32, cy: f32, r: f32, c: Paint) {
        self.pic.ops.push(DrawOp::Circle { cx: self.r.x as f32 + cx, cy: self.r.y as f32 + cy, r, width: None, c, arc: None });
    }

    /// A ring `width` thick.
    pub fn ring(&mut self, cx: f32, cy: f32, r: f32, width: f32, c: Paint) {
        self.pic.ops.push(DrawOp::Circle { cx: self.r.x as f32 + cx, cy: self.r.y as f32 + cy, r, width: Some(width), c, arc: None });
    }

    /// `text` in the box (x, y, w, h) in style `st` with colour `c`, vertically centred;
    /// returns its width. Text wider than the box is cut.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, x: i32, y: i32, w: i32, h: i32, text: &str, st: Style, c: Paint, align: Align) -> i32 {
        let st = Style { colour: c, ..st };
        let align = match align {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        };
        #[cfg(test)]
        self.pic.texts.push(text.to_string());
        self.pic.record_label(&st, self.r.x + x, self.r.y + y, w, h, text.to_string(), align, "center")
    }

    /// How wide `text` is in style `st`.
    pub fn text_width(&self, text: &str, st: Style) -> i32 {
        Picture::width(&st, text)
    }
}

/// A number from the inputs (`nil` or not a number: `None`).
pub fn num(t: &Table, k: &str) -> Option<f32> {
    match t.raw_get::<Value>(k).ok()? {
        Value::Integer(n) => Some(n as f32),
        Value::Number(n) => Some(n as f32),
        _ => None,
    }
}

/// A list of numbers from the inputs (missing: empty).
pub fn nums(t: &Table, k: &str) -> Vec<f32> {
    let Ok(Some(list)) = t.raw_get::<Option<Table>>(k) else { return Vec::new() };
    list.sequence_values::<f32>().filter_map(Result::ok).collect()
}

/// A string from the inputs.
pub fn text(t: &Table, k: &str) -> Option<String> {
    t.raw_get::<Option<String>>(k).ok().flatten()
}

/// A flag from the inputs (missing: false).
pub fn flag(t: &Table, k: &str) -> bool {
    t.raw_get::<Option<bool>>(k).ok().flatten().unwrap_or(false)
}

/// Where `hz` sits on a log frequency axis from 20 Hz (0) to 20 kHz (1).
pub fn freq_t(hz: f32) -> f32 {
    (hz.max(1.0).ln() - 20f32.ln()) / (20000f32.ln() - 20f32.ln())
}

/// A frequency as the knobs write it: "80 Hz", "1.20 kHz", "12.0 kHz".
pub fn hz_text(hz: f32) -> String {
    if hz >= 10000.0 {
        format!("{:.1} kHz", hz / 1000.0)
    } else if hz >= 1000.0 {
        format!("{:.2} kHz", hz / 1000.0)
    } else {
        format!("{:.0} Hz", hz)
    }
}

/// A level: "-6.0 dB", "+3.5 dB", "-inf dB" (at or under -120).
pub fn db_text(db: f32) -> String {
    if db <= -120.0 {
        "-inf dB".into()
    } else if db > 0.05 {
        format!("+{db:.1} dB")
    } else {
        format!("{db:.1} dB")
    }
}

/// A time: "4.0 ms", "380 ms", "1.20 s".
pub fn ms_text(ms: f32) -> String {
    if ms >= 1000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else if ms >= 100.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{ms:.1} ms")
    }
}

/// With the dials centred: how much higher than the plain look their box (and a
/// picture's floor) ends, leaving a row over the knobs' names for the sections' names.
const CENTRED_GAP: i32 = 14;
/// With the dials centred: how much higher than the band a picture starts (level with
/// the plug-in's own picture beside it).
const CENTRED_RISE: i32 = 28;

/// The pictures for a page of knobs (`knobs`, the Control page's eight, `false` where
/// there's none), each in the picture band over the knobs it draws ([`layout`]), with
/// touch focus from `touched` (knob numbers 1..8; three or more is a resting hand and
/// lights nothing). Returns the spans drawn: `{ first = 1..8, cells = n, kind = "filter" }`.
///
/// `plugin` names the focused slot's plug-in: its mapping's pictures (`mapping`, from
/// the controller scripts' `mappings/`) come first, the generic patterns fill the knobs
/// they leave, and every knob still bare gets a `dial` of its own (on the right screen
/// only if `right_dials`: not over a picture the page draws there itself). A choice's
/// list goes over such a picture all the same, on its own background. `draw` false
/// lays the spans out without drawing them, so a page can draw its own picture first.
#[allow(clippy::too_many_arguments)]
pub fn draw_pictures(
    lua: &Lua,
    pic: &mut Picture,
    knobs: &Table,
    touched: &[usize],
    plugin: Option<&str>,
    right_dials: bool,
    draw: bool,
    look: &Look,
) -> mlua::Result<Table> {
    let page: [Option<layout::Knob>; 8] = std::array::from_fn(|i| {
        let t = knobs.raw_get::<Option<Table>>(i + 1).ok().flatten()?;
        Some(layout::Knob {
            name: text(&t, "name").unwrap_or_default(),
            section: text(&t, "section").unwrap_or_default(),
            text: text(&t, "text").unwrap_or_default(),
            value: num(&t, "value").unwrap_or(0.0),
            mod_text: text(&t, "mod_text"),
            mod_value: num(&t, "mod_position"),
            choices: match t.raw_get::<Option<Table>>("choices") {
                Ok(Some(list)) => list.sequence_values::<String>().filter_map(Result::ok).collect(),
                _ => Vec::new(),
            },
        })
    });
    let ids: [Option<u32>; 8] = std::array::from_fn(|i| {
        let t = knobs.raw_get::<Option<Table>>(i + 1).ok().flatten()?;
        // (An integer: ids are 32-bit, past what an f32 holds exactly.)
        t.raw_get::<Option<i64>>("id").ok().flatten().and_then(|n| u32::try_from(n).ok())
    });
    let mut page = page;
    let mut spans = Vec::new();
    if let Some(name) = plugin
        && let Some(all) = lua.app_data_ref::<Mappings>()
        && let Some(m) = mapping::find(&all.0, name, "", "")
    {
        spans = m.spans(&page, &ids);
        // (A mapping's choices name what a plug-in that hides them picks among.)
        for (k, choices) in m.choices_on(&page, &ids) {
            if let Some(knob) = page[k].as_mut().filter(|kn| kn.choices.is_empty()) {
                knob.choices = choices;
            }
        }
    }
    let mut taken = [false; 8];
    for s in &spans {
        for k in s.first..s.first + s.cells {
            taken[k] = true;
        }
        for (k, _) in &s.knobs {
            taken[*k] = true;
        }
    }
    for s in layout::layout(&page) {
        if !(s.first..s.first + s.cells).any(|k| taken[k]) && !s.knobs.iter().any(|(k, _)| taken[*k]) {
            for k in s.first..s.first + s.cells {
                taken[k] = true;
            }
            spans.push(s);
        }
    }
    // Over each knob still bare: a choice's few values (one lit), else a dial; a choice
    // of many, or a value that isn't a number ("Off", "LP24"), gets nothing.
    for (k, knob) in page.iter().enumerate() {
        let Some(knob) = knob else { continue };
        if taken[k] {
            continue;
        }
        if !knob.choices.is_empty() {
            let n = knob.choices.len();
            if (2..=choices::MAX).contains(&n) {
                let at = knob.choices.iter().position(|c| c.trim().eq_ignore_ascii_case(knob.text.trim()));
                let active = at.unwrap_or_else(|| (knob.value.clamp(0.0, 1.0) * (n - 1) as f32).round() as usize);
                let inputs = vec![
                    ("items".to_string(), layout::Input::List(knob.choices.clone())),
                    ("active".to_string(), layout::Input::Num((active + 1) as f32)),
                ];
                spans.push(layout::Span { first: k, cells: 1, kind: "choices".into(), inputs, knobs: vec![(k, "value".into())] });
            }
            continue;
        }
        if k >= 4 && !right_dials {
            continue;
        }
        let numeric = knob.text.trim().chars().next().is_none_or(|ch| ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.' | '∞'));
        if !numeric {
            continue;
        }
        let mut inputs = vec![("value".to_string(), layout::Input::Num(knob.value))];
        if let Some(m) = knob.mod_value {
            inputs.push(("value_mod".to_string(), layout::Input::Num(m)));
        }
        spans.push(layout::Span { first: k, cells: 1, kind: "dial".into(), inputs, knobs: vec![(k, "value".into())] });
    }
    spans.sort_by_key(|s| s.first);
    let st = style();
    // Faint dividers where two pictures meet on one screen.
    if st.dividers {
        let ink = mix(look.background, look.separator, 0.45);
        for w in spans.windows(2) {
            let edge = w[0].first + w[0].cells;
            if edge == w[1].first && edge % 4 != 0 {
                let x = ((edge / 4) * 480 + (edge % 4) * CELL_W as usize) as f32;
                pic.ops.push(DrawOp::Line { p0: (x, (BAND_Y + 10) as f32), p1: (x, (BAND_Y + BAND_H - 10) as f32), width: 1.0, c: ink });
            }
        }
    }
    let out = lua.create_table()?;
    for span in spans {
        let inputs = lua.create_table()?;
        for (k, v) in &span.inputs {
            match v {
                layout::Input::Num(n) => inputs.raw_set(k.as_str(), *n)?,
                layout::Input::Text(s) => inputs.raw_set(k.as_str(), s.as_str())?,
                layout::Input::List(l) => inputs.raw_set(k.as_str(), lua.create_sequence_from(l.iter().map(String::as_str))?)?,
            }
        }
        if touched.len() < 3
            && let Some(f) = touched.iter().find_map(|&k| k.checked_sub(1).and_then(|k| span.focus_of(k)))
        {
            inputs.raw_set("focus", f)?;
        }
        let x = (span.first / 4) as i32 * 480 + (span.first % 4) as i32 * CELL_W;
        // (A dial's box: lined up with its knob's name and value, which start 11 px into
        // the cell, rather than centred in the cell.)
        // (Centred: in the middle of the cell (or at its left), down close over the knob's
        // name, every dial at one height; the section names go over them. A picture there
        // ends on the dials' line, the ring's lowest point 8 px above its box's foot, so
        // a curve's floor and a dial's foot sit level.)
        let r = if span.kind == "dial" && look.centred {
            Rect { x: if look.left { x + 6 } else { x + (CELL_W - 54) / 2 }, y: BAND_Y, w: 54, h: KNOB_NAME_Y - 2 - CENTRED_GAP - BAND_Y }
        } else if span.kind == "dial" {
            Rect { x: x + 6, y: BAND_Y, w: 54, h: BAND_H }
        } else {
            // (Centred, the sections' names are under the band, not over it: a picture
            // starts higher, into the room they left.)
            let top = if look.centred { BAND_Y - CENTRED_RISE } else { BAND_Y };
            let h = if look.centred { KNOB_NAME_Y - 2 - CENTRED_GAP - 8 + 1 - top } else { BAND_H };
            // (At the left: in from the cells' edges as far as the names under it, so it
            // keeps the screen's margins.)
            let inset = if look.left { 11 } else { 0 };
            Rect { x: x + inset, y: top, w: span.cells as i32 * CELL_W - 2 * inset, h }
        };
        // Each picture (and each knob's dial or list) on a panel of its own.
        if draw && st.panels {
            let w = span.cells as i32 * CELL_W;
            pic.ops.push(DrawOp::Fill { x: x + 4, y: BAND_Y, w: w - 8, h: BAND_H, c: look.panel, corner: 0.35 });
        }
        if draw {
            draw_lua(&span.kind, pic, r, &inputs, look)?;
        }
        let s = lua.create_table()?;
        s.raw_set("first", span.first + 1)?;
        s.raw_set("cells", span.cells)?;
        s.raw_set("kind", span.kind.as_str())?;
        out.raw_push(s)?;
    }
    // A dot-matrix style: the band on both screens redrawn in dots.
    if st.dots >= 2 {
        let ghost = mix(look.background, look.text, 0.06).rgb;
        for x in [0, 480] {
            pic.ops.push(DrawOp::Dots { x, y: BAND_Y, w: 480, h: BAND_H, cell: st.dots, ghost });
        }
    }
    Ok(out)
}

/// The plug-in mappings the scripts loaded (Lua app data).
pub struct Mappings(pub std::rc::Rc<Vec<mapping::Mapping>>);

/// Draw widget `kind` from a script's inputs table.
pub fn draw_lua(kind: &str, pic: &mut Picture, r: Rect, t: &Table, look: &Look) -> mlua::Result<()> {
    let mut c = Canvas::new(pic, r, look);
    c.focus = text(t, "focus");
    draw_on(kind, &mut c, t)
}

/// Draw widget `kind` from `t` on canvas `c`.
pub fn draw_on(kind: &str, c: &mut Canvas, t: &Table) -> mlua::Result<()> {
    let c = &mut *c;
    match kind {
        "curve" => curve::Curve::from_lua(t)?.draw(c),
        "dial" => dial::Dial::from_lua(t)?.draw(c),
        "choices" => choices::Choices::from_lua(t)?.draw(c),
        "filter" => filter::Filter::from_lua(t)?.draw(c),
        "band" => band::Band::from_lua(t)?.draw(c),
        "eq" => eq::Eq::from_lua(t)?.draw(c),
        "envelope" => envelope::Envelope::from_lua(t)?.draw(c),
        "lfo" => lfo::Lfo::from_lua(t)?.draw(c),
        "oscillators" => oscillators::Oscillators::from_lua(t)?.draw(c),
        "transfer" => transfer::Transfer::from_lua(t)?.draw(c),
        "value" => value::Value::from_lua(t)?.draw(c),
        _ => return Err(mlua::Error::runtime(format!("no widget \"{kind}\""))),
    }
    Ok(())
}

#[cfg(test)]
mod edge_tests {
    use super::golden;

    /// Every widget, pushed to the ends of its inputs, still draws only inside its box
    /// (the MK3 shows each screen apart: a mark past a picture's edge at x = 480 lands
    /// on the other screen).
    #[test]
    fn widgets_stay_in_their_box_at_the_extremes() {
        let lua = mlua::Lua::new();
        let cases = [
            ("curve", "{ kind = 'freq', points = { 0, 1, 0 }, bands = { { x = 0, y = 0 }, { x = 1, y = 1 }, { x = 0, y = 1 } } }", 2),
            ("curve", "{ kind = 'time', points = { 1, 0 }, bands = { { x = 0, y = 1 }, { x = 1, y = 0 } }, focus = 'bands' }", 2),
            ("filter", "{ cutoff = 20, resonance = 1, focus = 'cutoff' }", 2),
            ("filter", "{ cutoff = 20000, resonance = 1, shape = 'hp', focus = 'resonance', cutoff_mod = 20 }", 2),
            ("band", "{ freq = 20, gain = 24, q = 20, focus = 'freq' }", 2),
            ("band", "{ freq = 20000, gain = -24, q = 0.1, type = 'high_shelf', focus = 'q', freq_mod = 20 }", 2),
            (
                "eq",
                "{ bands = { { freq = 20, gain = 18, q = 10 }, { freq = 20000, gain = -18, q = 0.1 }, { freq = 20, gain = -18, type = 'low_cut' } }, focus = 'band1' }",
                3,
            ),
            ("eq", "{ bands = { { freq = 10, gain = 30, q = 40 }, { freq = 30000, gain = -30, q = 40 } }, focus = 'band2' }", 3),
            ("envelope", "{ attack = 0, decay = 0, sustain = 1, release = 0, focus = 'attack' }", 2),
            ("envelope", "{ attack = 100000, decay = 100000, sustain = 0, release = 100000, amount = -1, attack_mod = 0 }", 2),
            ("lfo", "{ shape = 'square', depth = 1, position = 1, phase = 1, focus = 'depth', depth_mod = 1 }", 2),
            ("transfer", "{ threshold = 0, ratio = 100, makeup = 24, level = 0, reduction = 60, focus = 'threshold' }", 2),
            ("transfer", "{ threshold = -60, ratio = 1, knee = 24, makeup = -24, level = -100 }", 2),
            (
                "value",
                "{ name = 'A VERY LONG PARAMETER NAME', text = '-120000.00 kHz', value = 1, value_mod = 0, bipolar = true, focus = 'value' }",
                1,
            ),
            ("oscillators", "{ wave1 = 'very nrw', octave1 = 4, wave2 = 'saw', octave2 = -3, noise = 1, focus = 'wave1' }", 2),
            ("dial", "{ value = 1, value_mod = 0, focus = 'value' }", 1),
            ("dial", "{ value = 0, bipolar = true, steps = 16, value_mod = 1 }", 1),
            ("choices", "{ items = { 'A VERY LONG CHOICE NAME', 'B', 'C', 'D', 'E', 'F' }, active = 6, focus = 'value' }", 1),
        ];
        let mut failures = Vec::new();
        for theme in golden::THEMES {
            let look = golden::look(theme);
            for (kind, src, min_cells) in cases {
                let t: mlua::Table = lua.load(format!("return {src}")).eval().unwrap();
                for w in golden::widths(min_cells) {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        golden::render(&look, w, None, |cv| {
                            cv.focus = super::text(&t, "focus");
                            super::draw_on(kind, cv, &t).unwrap();
                        })
                    }));
                    if r.is_err() {
                        failures.push(format!("{kind} {src} at {w} on {theme}"));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "drawing outside the box:
{}",
            failures.join(
                "
"
            )
        );
    }
}
