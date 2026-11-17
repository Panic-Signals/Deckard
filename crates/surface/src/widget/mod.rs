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
pub mod curve;
pub mod envelope;
pub mod eq;
pub mod filter;
pub mod layout;
pub mod lfo;
pub mod mapping;
pub mod transfer;
pub mod value;

#[cfg(test)]
pub(crate) mod golden;

/// One knob cell's width: a screen's 480 pixels over its four knobs.
pub const CELL_W: i32 = 120;
/// The picture band, the same on both screens (`widgets.BAND_Y`, `widgets.BAND_H` in
/// `lib/widgets.lua`): under the left screen's chain strip, above the knobs' section names.
pub const BAND_Y: i32 = 100;
pub const BAND_H: i32 = 104;

/// A box on the screens (960 × 272, the right screen from x = 480).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
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
        let weak = c("weak", f.weak);
        let mut small = cache.style("default");
        small.size = 10.0;
        small.colour = weak;
        let mut label = cache.style("param_name");
        label.colour = weak;
        let big = cache.style("param_value");
        Look {
            background,
            panel: c("panel", f.panel),
            text,
            weak,
            grid: c("button", f.grid),
            separator: c("separator", f.separator),
            muted: c("muted", f.muted),
            accent: c("accent", f.accent),
            record: c("record", f.record),
            marker: c("marker", f.marker),
            on_colour: c("on_colour", f.on_colour),
            small,
            label,
            big,
            soft: cache.soft,
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
        }
    }

    /// The fill under a curve: a dimmer step of the curve's colour toward the field,
    /// opaque, so it reads on the `lcd` theme and keeps the MK3's runs of one colour.
    pub fn fill_of(&self, c: Paint) -> Paint {
        mix(self.background, c, if self.soft { 0.45 } else { 0.3 })
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

    /// A line `width` thick.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, c: Paint) {
        let (ox, oy) = (self.r.x as f32, self.r.y as f32);
        self.pic.ops.push(DrawOp::Line { p0: (ox + x0, oy + y0), p1: (ox + x1, oy + y1), width, c });
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
        if pts.len() < 2 {
            return;
        }
        let (x0, x1) = (pts[0].0.ceil() as i32, pts[pts.len() - 1].0.floor() as i32);
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
            if bottom > top {
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

/// The pictures for a page of knobs (`knobs`, the Control page's eight, `false` where
/// there's none), each in the picture band over the knobs it draws ([`layout`]), with
/// touch focus from `touched` (knob numbers 1..8; three or more is a resting hand and
/// lights nothing). Returns the spans drawn: `{ first = 1..8, cells = n, kind = "filter" }`.
///
/// `plugin` names the focused slot's plug-in: its mapping's pictures (`mapping`, from
/// the controller scripts' `mappings/`) come first, and the generic patterns fill the
/// knobs they leave.
pub fn draw_pictures(
    lua: &Lua,
    pic: &mut Picture,
    knobs: &Table,
    touched: &[usize],
    plugin: Option<&str>,
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
        })
    });
    let ids: [Option<u32>; 8] = std::array::from_fn(|i| {
        let t = knobs.raw_get::<Option<Table>>(i + 1).ok().flatten()?;
        num(&t, "id").map(|n| n as u32)
    });
    let mut spans = Vec::new();
    if let Some(name) = plugin
        && let Some(all) = lua.app_data_ref::<Mappings>()
        && let Some(m) = mapping::find(&all.0, name, "", "")
    {
        spans = m.spans(&page, &ids);
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
            spans.push(s);
        }
    }
    spans.sort_by_key(|s| s.first);
    let out = lua.create_table()?;
    for span in spans {
        let inputs = lua.create_table()?;
        for (k, v) in &span.inputs {
            match v {
                layout::Input::Num(n) => inputs.raw_set(k.as_str(), *n)?,
                layout::Input::Text(s) => inputs.raw_set(k.as_str(), s.as_str())?,
            }
        }
        if touched.len() < 3
            && let Some(f) = touched.iter().find_map(|&k| k.checked_sub(1).and_then(|k| span.focus_of(k)))
        {
            inputs.raw_set("focus", f)?;
        }
        let r = Rect {
            x: (span.first / 4) as i32 * 480 + (span.first % 4) as i32 * CELL_W,
            y: BAND_Y,
            w: span.cells as i32 * CELL_W,
            h: BAND_H,
        };
        draw_lua(&span.kind, pic, r, &inputs, look)?;
        let s = lua.create_table()?;
        s.raw_set("first", span.first + 1)?;
        s.raw_set("cells", span.cells)?;
        s.raw_set("kind", span.kind.as_str())?;
        out.raw_push(s)?;
    }
    Ok(out)
}

/// The plug-in mappings the scripts loaded (Lua app data).
pub struct Mappings(pub std::rc::Rc<Vec<mapping::Mapping>>);

/// Draw widget `kind` from a script's inputs table.
pub fn draw_lua(kind: &str, pic: &mut Picture, r: Rect, t: &Table, look: &Look) -> mlua::Result<()> {
    let mut c = Canvas::new(pic, r, look);
    c.focus = text(t, "focus");
    match kind {
        "curve" => curve::Curve::from_lua(t)?.draw(&mut c),
        "filter" => filter::Filter::from_lua(t)?.draw(&mut c),
        "band" => band::Band::from_lua(t)?.draw(&mut c),
        "eq" => eq::Eq::from_lua(t)?.draw(&mut c),
        "envelope" => envelope::Envelope::from_lua(t)?.draw(&mut c),
        "lfo" => lfo::Lfo::from_lua(t)?.draw(&mut c),
        "transfer" => transfer::Transfer::from_lua(t)?.draw(&mut c),
        "value" => value::Value::from_lua(t)?.draw(&mut c),
        _ => return Err(mlua::Error::runtime(format!("no widget \"{kind}\""))),
    }
    Ok(())
}
