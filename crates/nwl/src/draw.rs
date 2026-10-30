//! Lay out and draw a widget tree the way NI's hardware displays do: each widget
//! styled by the sheet ([`crate::sheet`]), bars lining their children up (right or
//! down, `spacing` apart, inside their `margin`s, `outer-margin`s around each child),
//! one child taking what's left (`setFlex`), stacks showing their top child.
//! Backgrounds, nine-slice pictures (`fixed-*`, frames `animations` high picked by
//! `animation-index`) and text in the sheet's fonts.

use crate::sheet::{Node, Sheet, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Which way a bar lines up its children (`NI.GUI.ALIGN_WIDGET_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dir {
    #[default]
    Right,
    Down,
    Left,
    Up,
}

/// A widget to draw.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Element {
    /// `Bar`, `Label`, `Button`, `Stack`, ...
    pub kind: String,
    pub name: String,
    /// Style class(es).
    pub class: String,
    pub attrs: Vec<(String, String)>,
    /// Hidden widgets keep their space but aren't drawn.
    pub hidden: bool,
    pub selected: bool,
    pub enabled: bool,
    pub text: Option<String>,
    pub dir: Dir,
    /// The child that takes the space left over.
    pub flex: Option<usize>,
    /// For stacks: the child shown.
    pub top: Option<usize>,
    /// Colour index for `*-color-palette` properties.
    pub palette_index: Option<u32>,
    /// Placed by the scripts (`setPos`): out of its bar's line, at this offset from the
    /// bar's content.
    pub pos: Option<(i32, i32)>,
    /// Sized by the scripts (`setWidth`/`setHeight`), over the style sheet's size.
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// For scrollbars: the part shown, as (start, length) fractions of the whole; the
    /// first child (the grip) is laid along it.
    pub scroll: Option<(f32, f32)>,
    /// Drawn by the host over the widget's background: what native widgets (the
    /// pattern editor) show, which have no script-built children.
    pub shapes: Vec<Shape>,
    /// A picture the scripts set (`setPicture`), drawn at its own size inside the
    /// margins, over `bg` (`setBGColor`) if that's set.
    pub picture: Option<String>,
    pub bg: Option<[u8; 4]>,
    pub children: Vec<Element>,
}

/// A coordinate in a widget's box: `frac` of its width (or height) plus `px` pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct At {
    pub frac: f32,
    pub px: i32,
}

impl At {
    pub fn px(px: i32) -> At {
        At { frac: 0.0, px }
    }
    pub fn frac(frac: f32) -> At {
        At { frac, px: 0 }
    }
    fn resolve(self, size: i32) -> i32 {
        (self.frac * size as f32).round() as i32 + self.px
    }
}

/// A colour, or one of the sheet's palettes (`*-color-palette`) at an index, with alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fill {
    Rgba([u8; 4]),
    Palette { palette: u32, index: u32, alpha: u8 },
}

/// Something the host draws in a widget's box (see [`Element::shapes`]): a filled
/// rectangle from (x0, y0) to (x1, y1) (at least a pixel each way), or text left-aligned
/// in it, in a font of the resources at a size.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub x0: At,
    pub y0: At,
    pub x1: At,
    pub y1: At,
    pub fill: Fill,
    pub text: Option<(String, String, f32)>,
    /// Drawn as the ellipse inside the rectangle (a dot), edges smoothed.
    pub round: bool,
    /// Text centred across the rectangle (else from its left).
    pub centred: bool,
    /// Text cut off at the rectangle's edges (else only at the element's).
    pub clip: bool,
}

impl Shape {
    pub fn rect(x0: At, y0: At, x1: At, y1: At, fill: Fill) -> Shape {
        Shape { x0, y0, x1, y1, fill, text: None, round: false, centred: false, clip: false }
    }
}

/// An RGBA picture.
#[derive(Debug, Clone)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub px: Vec<[u8; 4]>,
}

impl Canvas {
    pub fn new(width: usize, height: usize, fill: [u8; 4]) -> Canvas {
        Canvas { width, height, px: vec![fill; width * height] }
    }

    fn blend(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height || c[3] == 0 {
            return;
        }
        let d = &mut self.px[y as usize * self.width + x as usize];
        let a = c[3] as u32;
        for i in 0..3 {
            d[i] = ((c[i] as u32 * a + d[i] as u32 * (255 - a)) / 255) as u8;
        }
        d[3] = d[3].max(c[3]);
    }

    fn fill(&mut self, r: Rect, c: [u8; 4], clip: Rect) {
        let r = r.intersect(clip);
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                self.blend(x, y, c);
            }
        }
    }

    /// Copy the part `src` of `pic` stretched onto `dst` (nearest pixel).
    fn blit(&mut self, pic: &Canvas, src: Rect, dst: Rect, clip: Rect) {
        if src.w <= 0 || src.h <= 0 || dst.w <= 0 || dst.h <= 0 {
            return;
        }
        let shown = dst.intersect(clip);
        for y in shown.y..shown.y + shown.h {
            let sy = src.y + ((y - dst.y) * src.h / dst.h);
            for x in shown.x..shown.x + shown.w {
                let sx = src.x + ((x - dst.x) * src.w / dst.w);
                if sx >= 0 && sy >= 0 && (sx as usize) < pic.width && (sy as usize) < pic.height {
                    self.blend(x, y, pic.px[sy as usize * pic.width + sx as usize]);
                }
            }
        }
    }

    /// Save as a PNG file.
    pub fn write_png(&self, path: &Path) -> Result<(), String> {
        let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), self.width as u32, self.height as u32);
        enc.set_color(png::ColorType::Rgba);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&self.px.concat()).map_err(|e| e.to_string())
    }

    pub fn read_png(path: &Path) -> Result<Canvas, String> {
        load_png(path).ok_or_else(|| format!("{}: not an RGB(A) or grey PNG", path.display()))
    }

    /// Part of this canvas.
    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Canvas {
        let mut out = Canvas::new(w, h, [0, 0, 0, 255]);
        for row in 0..h.min(self.height.saturating_sub(y)) {
            for col in 0..w.min(self.width.saturating_sub(x)) {
                out.px[row * w + col] = self.px[(y + row) * self.width + x + col];
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn intersect(self, o: Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = (self.x + self.w).min(o.x + o.w);
        let b = (self.y + self.h).min(o.y + o.h);
        Rect { x, y, w: (r - x).max(0), h: (b - y).max(0) }
    }
}

/// Google's Noto Emoji (monochrome; SIL Open Font License 1.1, `assets/NotoEmoji-OFL.txt`),
/// for emoji in names: the display's fallback font, and the app's.
pub const EMOJI_FONT: &[u8] = include_bytes!("../assets/NotoEmoji.ttf");

/// NI's look: the style sheet and where its pictures and fonts are.
pub struct Look {
    pub sheet: Sheet,
    roots: Vec<PathBuf>,
    pictures: RefCell<HashMap<String, Option<std::rc::Rc<Canvas>>>>,
    fonts: RefCell<HashMap<String, Option<std::rc::Rc<fontdue::Font>>>>,
    /// For characters a sheet's font hasn't (emoji in names): [`EMOJI_FONT`].
    fallback: std::cell::OnceCell<Option<std::rc::Rc<fontdue::Font>>>,
}

/// One widget's style, its properties by name.
#[derive(Debug, Default, Clone)]
pub struct Style(HashMap<String, Value>);

impl Style {
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.0.get(k)
    }

    pub fn num(&self, k: &str) -> Option<i32> {
        self.0.get(k).and_then(Value::num).map(|n| n.round() as i32)
    }

    pub fn margin(&self, side: &str) -> i32 {
        self.num(&format!("margin-{side}")).or_else(|| self.num("margin")).unwrap_or(0)
    }

    pub fn outer(&self, side: &str) -> i32 {
        self.num(&format!("outer-margin-{side}")).or_else(|| self.num("outer-margin")).unwrap_or(0)
    }
}

/// Properties a widget takes from its parent unless its own rules set them.
const INHERITED: [&str; 3] = ["font-name", "font-size", "foreground-color"];

impl Look {
    /// `res` is an `ni-res-extract` directory (`out/ni-res`); the MK3's sheet.
    pub fn mk3(res: &Path) -> Result<Look, String> {
        let roots = [
            "maschine_nwl_resources",
            "shared_nwl_resources",
            "shared_nwl_pic_resources",
            "maschine_pictures_nwl_resources",
            "shared_nwl_font_resources",
        ]
        .iter()
        .map(|r| res.join(r))
        .collect::<Vec<_>>();
        let sheet = Sheet::load(&roots, "Maschine/HW/MaschineMK3/MaschineMK3Styles.txt")?;
        Ok(Look { sheet, roots, pictures: Default::default(), fonts: Default::default(), fallback: Default::default() })
    }

    fn file(&self, path: &str) -> Option<PathBuf> {
        self.roots.iter().map(|r| r.join(path)).find(|p| p.is_file())
    }

    /// A look with no style sheet or resources (pictures added with `add_picture` only).
    pub fn empty() -> Look {
        Look {
            sheet: Sheet::parse(""),
            roots: Vec::new(),
            pictures: Default::default(),
            fonts: Default::default(),
            fallback: Default::default(),
        }
    }

    /// Add a picture of the host's own (a PNG) under `path`, over any of NI's there.
    pub fn add_picture(&self, path: &str, png: &[u8]) -> bool {
        let pic = decode_png(std::io::Cursor::new(png)).map(std::rc::Rc::new);
        let ok = pic.is_some();
        self.pictures.borrow_mut().insert(path.to_string(), pic);
        ok
    }

    /// A picture's size, if there is one at `path`.
    pub fn picture_size(&self, path: &str) -> Option<(usize, usize)> {
        self.picture(path).map(|p| (p.width, p.height))
    }

    fn picture(&self, path: &str) -> Option<std::rc::Rc<Canvas>> {
        self.pictures
            .borrow_mut()
            .entry(path.to_string())
            .or_insert_with(|| self.file(path).and_then(|f| load_png(&f)).map(std::rc::Rc::new))
            .clone()
    }

    fn font(&self, path: &str) -> Option<std::rc::Rc<fontdue::Font>> {
        self.fonts
            .borrow_mut()
            .entry(path.to_string())
            .or_insert_with(|| {
                let bytes = std::fs::read(self.file(path)?).ok()?;
                fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok().map(std::rc::Rc::new)
            })
            .clone()
    }

    /// The font to draw `c` in: `font`, else the fallback if it has `c`; `None` draws
    /// nothing (joiners and variation selectors neither has).
    fn glyph_font(&self, font: &std::rc::Rc<fontdue::Font>, c: char) -> Option<std::rc::Rc<fontdue::Font>> {
        if font.lookup_glyph_index(c) != 0 {
            return Some(font.clone());
        }
        let fallback = self
            .fallback
            .get_or_init(|| fontdue::Font::from_bytes(EMOJI_FONT, fontdue::FontSettings::default()).ok().map(std::rc::Rc::new));
        fallback.as_ref().filter(|f| f.lookup_glyph_index(c) != 0).cloned()
    }

    fn style(&self, path: &[Node], parent: Option<&Style>) -> Style {
        let mut s = Style::default();
        if let Some(p) = parent {
            for k in INHERITED {
                if let Some(v) = p.0.get(k) {
                    s.0.insert(k.to_string(), v.clone());
                }
            }
        }
        for r in self.sheet.matching(path) {
            for (k, v) in &r.decls {
                s.0.insert(k.clone(), v.clone());
                // A plain colour overrides a palette colour from a weaker rule.
                let palette = format!("{k}-palette");
                if k.ends_with("-color") && !r.decls.iter().any(|(d, _)| *d == palette) {
                    s.0.remove(&palette);
                }
            }
        }
        s
    }

    /// Draw `root` onto a `width` × `height` canvas.
    pub fn render(&self, root: &Element, width: usize, height: usize) -> Canvas {
        let mut canvas = Canvas::new(width, height, [0, 0, 0, 255]);
        self.render_over(&mut canvas, root);
        canvas
    }

    /// Draw `root` over what `canvas` already shows (e.g. an overlay over a page).
    pub fn render_over(&self, canvas: &mut Canvas, root: &Element) {
        let tree = self.styled(root, &mut Vec::new(), None);
        let full = Rect { x: 0, y: 0, w: canvas.width as i32, h: canvas.height as i32 };
        let (w, h) = self.measure(&tree);
        let rect = Rect { x: tree.style.outer("left"), y: tree.style.outer("top"), w: w.max(1), h: h.max(1) };
        let mut boxes = Vec::new();
        self.arrange(&tree, rect, &mut boxes);
        self.paint(&tree, &boxes, &mut 0, canvas, full);
    }

    /// Every box `root` lays out to, depth first, one line each: its depth, kind, name
    /// and class, the rectangle, its text and the style that draws it (fonts, colours,
    /// margins, picture). For measuring what a page looks like.
    pub fn layout(&self, root: &Element, width: usize, height: usize) -> Vec<String> {
        let tree = self.styled(root, &mut Vec::new(), None);
        let (w, h) = self.measure(&tree);
        let rect = Rect { x: tree.style.outer("left"), y: tree.style.outer("top"), w: w.max(1), h: h.max(1) };
        let _ = (width, height);
        let mut boxes = Vec::new();
        self.arrange(&tree, rect, &mut boxes);
        let mut out = Vec::new();
        fn walk(t: &Styled, depth: usize, boxes: &[Rect], at: &mut usize, out: &mut Vec<String>) {
            let r = boxes.get(*at).copied().unwrap_or(Rect { x: 0, y: 0, w: 0, h: 0 });
            *at += 1;
            let mut keys: Vec<_> = t.style.0.iter().filter(|(k, _)| !k.starts_with("animation")).collect();
            keys.sort_by(|a, b| a.0.cmp(b.0));
            let style: Vec<String> = keys.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
            out.push(format!(
                "{}{} {:?} {:?} [{},{} {}x{}]{}{} {}",
                "  ".repeat(depth),
                t.e.kind,
                t.e.name,
                t.e.class,
                r.x,
                r.y,
                r.w,
                r.h,
                t.e.text.as_deref().map(|x| format!(" text={x:?}")).unwrap_or_default(),
                format!("{}{}", if t.e.selected { " selected" } else { "" }, if t.e.hidden { " HIDDEN" } else { "" }),
                style.join(" ")
            ));
            for (_, c) in &t.children {
                walk(c, depth + 1, boxes, at, out);
            }
        }
        walk(&tree, 0, &boxes, &mut 0, &mut out);
        out
    }

    /// The style `chain` (a widget and its ancestors, root first) gets, as the
    /// scripts read it (`getIntProperty`).
    pub fn style_of(&self, chain: &[Element]) -> Style {
        let mut parent: Option<Style> = None;
        let mut path = Vec::new();
        for e in chain {
            path.push((
                e.kind.clone(),
                e.name.clone(),
                e.class.clone(),
                e.attrs.clone(),
                [("selected", e.selected), ("enabled", e.enabled)],
            ));
            let nodes: Vec<Node> = path.iter().map(|(k, n, c, a, s)| Node { kind: k, name: n, classes: c, attrs: a, states: s }).collect();
            parent = Some(self.style(&nodes, parent.as_ref()));
        }
        parent.unwrap_or_default()
    }

    /// How wide `text` is in a style's font (`getStringWidth`).
    pub fn string_width(&self, s: &Style, text: &str) -> i32 {
        self.text_width(s, text)
    }

    fn styled<'e>(
        &self,
        e: &'e Element,
        path: &mut Vec<(String, String, String, Vec<(String, String)>, [(&'static str, bool); 2])>,
        parent: Option<&Style>,
    ) -> Styled<'e> {
        path.push((e.kind.clone(), e.name.clone(), e.class.clone(), e.attrs.clone(), [("selected", e.selected), ("enabled", e.enabled)]));
        let nodes: Vec<Node> = path.iter().map(|(k, n, c, a, s)| Node { kind: k, name: n, classes: c, attrs: a, states: s }).collect();
        let style = self.style(&nodes, parent);
        let shown: Vec<usize> = match (e.kind.as_str(), e.top) {
            ("Stack", Some(t)) => vec![t],
            ("Stack", None) => e.children.iter().rposition(|c| !c.hidden).into_iter().collect(),
            _ => (0..e.children.len()).collect(),
        };
        let mut children: Vec<(usize, Styled)> = shown
            .into_iter()
            .filter_map(|i| e.children.get(i).map(|c| (i, c)))
            .map(|(i, c)| (i, self.styled(c, path, Some(&style))))
            .collect();
        // Higher z-index paints later, over its siblings.
        children.sort_by_key(|(_, c)| c.style.num("z-index").unwrap_or(0));
        path.pop();
        Styled { e, style, children }
    }

    fn text_width(&self, s: &Style, text: &str) -> i32 {
        let (Some(font), size) = (self.text_font(s), font_size(s)) else { return 0 };
        text.chars().filter_map(|c| self.glyph_font(&font, c).map(|f| f.metrics(c, size).advance_width)).sum::<f32>().ceil() as i32
    }

    fn text_font(&self, s: &Style) -> Option<std::rc::Rc<fontdue::Font>> {
        match s.get("font-name") {
            Some(Value::Url(u)) => self.font(u),
            _ => None,
        }
    }

    /// A widget's size by itself: its own width/height, or what its content needs.
    fn measure(&self, t: &Styled) -> (i32, i32) {
        let s = &t.style;
        let pad_w = s.margin("left") + s.margin("right");
        let pad_h = s.margin("top") + s.margin("bottom");
        let (mut cw, mut ch) = (0, 0);
        let (fixed_w, fixed_h) = (t.e.width.or(s.num("width")), t.e.height.or(s.num("height")));
        if fixed_w.is_none() || fixed_h.is_none() {
            if !t.children.is_empty() {
                let sizes: Vec<(i32, i32)> = t.children.iter().map(|(_, c)| outer_size(c, self.measure(c))).collect();
                if t.e.kind == "Stack" {
                    cw = sizes.iter().map(|z| z.0).max().unwrap_or(0);
                    ch = sizes.iter().map(|z| z.1).max().unwrap_or(0);
                } else {
                    // Overlays lie over the line, outside it.
                    let line: Vec<(i32, i32)> = t.children.iter().zip(&sizes).filter(|((_, c), _)| !overlay(c)).map(|(_, z)| *z).collect();
                    let gap = s.num("spacing").unwrap_or(0) * (line.len() as i32 - 1).max(0);
                    if matches!(t.e.dir, Dir::Right | Dir::Left) {
                        cw = line.iter().map(|z| z.0).sum::<i32>() + gap;
                        ch = line.iter().map(|z| z.1).max().unwrap_or(0);
                    } else {
                        cw = line.iter().map(|z| z.0).max().unwrap_or(0);
                        ch = line.iter().map(|z| z.1).sum::<i32>() + gap;
                    }
                    for (z, _) in sizes.iter().zip(&t.children).filter(|(_, (_, c))| overlay(c)) {
                        cw = cw.max(z.0);
                        ch = ch.max(z.1);
                    }
                }
            } else if let Some(text) = t.e.text.as_deref().filter(|t| !t.is_empty()) {
                cw = self.text_width(s, text);
                ch = font_size(s).ceil() as i32 + 2;
            }
            if let Some((pic, frames)) = self.image(s) {
                let fh = pic.height as i32 / frames.max(1);
                if pic_includes_margin(s) {
                    cw = cw.max(pic.width as i32 - pad_w);
                    ch = ch.max(fh - pad_h);
                } else {
                    cw = cw.max(pic.width as i32);
                    ch = ch.max(fh);
                }
            }
        }
        let mut w = fixed_w.unwrap_or(cw + pad_w);
        let mut h = fixed_h.unwrap_or(ch + pad_h);
        // A picture without fixed (nine-slice) borders can't be squeezed: the widget is
        // at least as big as it.
        if let Some((pic, frames)) = self.image(s)
            && t.e.width.is_none()
            && ["fixed-left", "fixed-right", "fixed-top", "fixed-bottom"].iter().all(|f| s.num(f).is_none())
        {
            w = w.max(pic.width as i32);
            h = h.max(pic.height as i32 / frames.max(1));
        }
        let clamp = |v: i32, lo: Option<i32>, hi: Option<i32>| v.max(lo.unwrap_or(0)).min(hi.unwrap_or(i32::MAX));
        (clamp(w, s.num("min-width"), s.num("max-width")), clamp(h, s.num("min-height"), s.num("max-height")))
    }

    fn image(&self, s: &Style) -> Option<(std::rc::Rc<Canvas>, i32)> {
        let Some(Value::Url(u)) = s.get("image") else { return None };
        Some((self.picture(u)?, s.num("animations").unwrap_or(1).max(1)))
    }

    /// Place `t` at `rect` (its border box) and its children inside, depth first.
    fn arrange(&self, t: &Styled, rect: Rect, boxes: &mut Vec<Rect>) {
        boxes.push(rect);
        let s = &t.style;
        let inner = Rect {
            x: rect.x + s.margin("left"),
            y: rect.y + s.margin("top"),
            w: rect.w - s.margin("left") - s.margin("right"),
            h: rect.h - s.margin("top") - s.margin("bottom"),
        };
        if t.children.is_empty() {
            return;
        }
        if let (Some((start, len)), Some((_, grip))) = (t.e.scroll, t.children.first()) {
            let along = |f: f32| (f * inner.h as f32).round() as i32;
            self.arrange(grip, Rect { x: inner.x, y: inner.y + along(start), w: inner.w, h: along(len).max(1) }, boxes);
            for (_, c) in &t.children[1..] {
                self.arrange(c, inner, boxes);
            }
            return;
        }
        if t.e.kind == "Stack" {
            for (_, c) in &t.children {
                let (w, h) = self.measure(c);
                let cs = &c.style;
                let w = if fixed(c).0 { w } else { inner.w - cs.outer("left") - cs.outer("right") };
                let h = if fixed(c).1 { h } else { inner.h - cs.outer("top") - cs.outer("bottom") };
                self.arrange(c, Rect { x: inner.x + cs.outer("left"), y: inner.y + cs.outer("top"), w, h }, boxes);
            }
            return;
        }
        let across = matches!(t.e.dir, Dir::Right | Dir::Left);
        let spacing = s.num("spacing").unwrap_or(0);
        let sizes: Vec<(i32, i32)> = t.children.iter().map(|(_, c)| self.measure(c)).collect();
        let flowing = t.children.iter().filter(|(_, c)| !overlay(c)).count() as i32;
        let main = |c: &Styled, z: (i32, i32)| {
            let (a, b) =
                if across { (c.style.outer("left"), c.style.outer("right")) } else { (c.style.outer("top"), c.style.outer("bottom")) };
            (if across { z.0 } else { z.1 }) + a + b
        };
        let used: i32 = t.children.iter().zip(&sizes).filter(|((_, c), _)| !overlay(c)).map(|((_, c), z)| main(c, *z)).sum::<i32>()
            + spacing * (flowing - 1).max(0);
        let room = if across { inner.w } else { inner.h };
        let flex = t.e.flex.and_then(|f| t.children.iter().position(|(i, _)| *i == f));
        let mut pos = 0;
        let n = t.children.len();
        for k in 0..n {
            // Left/up bars line their children up from the far end.
            let k = if matches!(t.e.dir, Dir::Left | Dir::Up) { n - 1 - k } else { k };
            let (_, c) = &t.children[k];
            let cs = &c.style;
            let (mut w, mut h) = sizes[k];
            if let Some((x, y)) = placed(c) {
                // Placed by the scripts (or the sheet's `x`/`y`).
                self.arrange(c, Rect { x: inner.x + x, y: inner.y + y, w, h }, boxes);
                continue;
            }
            if overlay(c) {
                // Out of the line: over the start of the bar.
                self.arrange(c, Rect { x: inner.x + cs.outer("left"), y: inner.y + cs.outer("top"), w, h }, boxes);
                continue;
            }
            if Some(k) == flex {
                let extra = (room - used).max(0);
                if across { w += extra } else { h += extra }
            }
            // Across the line, a child without its own size or picture fills the bar.
            let pictured = self.image(cs).is_some();
            if across && !fixed(c).1 && !pictured {
                h = inner.h - cs.outer("top") - cs.outer("bottom");
            }
            if !across && !fixed(c).0 && !pictured {
                w = inner.w - cs.outer("left") - cs.outer("right");
            }
            let r = if across {
                let x =
                    if t.e.dir == Dir::Left { inner.x + inner.w - pos - cs.outer("right") - w } else { inner.x + pos + cs.outer("left") };
                pos += w + cs.outer("left") + cs.outer("right") + spacing;
                Rect { x, y: inner.y + cs.outer("top"), w, h }
            } else {
                let y = if t.e.dir == Dir::Up { inner.y + inner.h - pos - cs.outer("bottom") - h } else { inner.y + pos + cs.outer("top") };
                pos += h + cs.outer("top") + cs.outer("bottom") + spacing;
                Rect { x: inner.x + cs.outer("left"), y, w, h }
            };
            self.arrange(c, r, boxes);
        }
    }

    fn colour(&self, s: &Style, e: &Element, prop: &str) -> Option<[u8; 4]> {
        if let (Some(p), Some(i)) = (s.num(&format!("{prop}-palette")), e.palette_index) {
            if let Some(c) = self.sheet.palette(p as u32, i) {
                return Some(c);
            }
        }
        match s.get(prop) {
            Some(Value::Color(c)) => Some(*c),
            _ => None,
        }
    }

    fn paint(&self, t: &Styled, boxes: &[Rect], next: &mut usize, canvas: &mut Canvas, clip: Rect) {
        let rect = boxes[*next];
        *next += 1;
        if t.e.hidden {
            *next += count_boxes(t) - 1;
            return;
        }
        let s = &t.style;
        if std::env::var_os("NWL_DEBUG").is_some() {
            let mut props: Vec<_> = s.0.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
            props.sort();
            eprintln!(
                "{} {}#{}.{} {:?} {:?} sel={} | {}",
                "",
                t.e.kind,
                t.e.name,
                t.e.class,
                rect,
                t.e.text,
                t.e.selected,
                props.join(" ")
            );
        }
        if let Some(bg) = t.e.bg.or_else(|| self.colour(s, t.e, "background-color")) {
            canvas.fill(rect, bg, clip);
        }
        if let Some(pic) = t.e.picture.as_deref().and_then(|p| self.picture(p)) {
            let dst = Rect { x: rect.x + s.margin("left"), y: rect.y + s.margin("top"), w: pic.width as i32, h: pic.height as i32 };
            canvas.blit(&pic, Rect { x: 0, y: 0, w: pic.width as i32, h: pic.height as i32 }, dst, clip.intersect(rect));
        }
        if s.num("max-bars").is_some() {
            self.progress(s, t.e, rect, canvas, clip);
        }
        if let Some((pic, frames)) = self.image(s) {
            let fh = pic.height as i32 / frames;
            let frame = s.num("animation-index").unwrap_or_else(|| default_frame(t.e, frames)).clamp(0, frames - 1);
            let src = Rect { x: 0, y: frame * fh, w: pic.width as i32, h: fh };
            let dst = if pic_includes_margin(s) {
                rect
            } else {
                Rect {
                    x: rect.x + s.margin("left"),
                    y: rect.y + s.margin("top"),
                    w: rect.w - s.margin("left") - s.margin("right"),
                    h: rect.h - s.margin("top") - s.margin("bottom"),
                }
            };
            let fixed = [s.num("fixed-left"), s.num("fixed-right"), s.num("fixed-top"), s.num("fixed-bottom")].map(|v| v.unwrap_or(0));
            nine_slice(canvas, &pic, src, dst, fixed, clip);
        }
        // (`foreground-color: no`: the text isn't shown.)
        let colour = match self.colour(s, t.e, "foreground-color") {
            Some(c) => Some(c),
            None if matches!(s.get("foreground-color"), Some(Value::None)) => None,
            None => Some([255, 255, 255, 255]),
        };
        if let (Some(text), Some(colour)) = (t.e.text.as_deref().filter(|t| !t.is_empty()), colour) {
            // foreground-relative-*: the text moved within the widget.
            let (dx, dy) = (s.num("foreground-relative-left").unwrap_or(0), s.num("foreground-relative-top").unwrap_or(0));
            let inner = Rect {
                x: rect.x + s.margin("left") + dx,
                y: rect.y + s.margin("top") + dy,
                w: rect.w - s.margin("left") - s.margin("right"),
                h: rect.h - s.margin("top") - s.margin("bottom"),
            };
            self.text(canvas, s, text, inner, colour, clip.intersect(rect));
        }
        for shape in &t.e.shapes {
            self.shape(shape, rect, canvas, clip.intersect(rect));
        }
        let clip = clip.intersect(rect);
        for (_, c) in &t.children {
            self.paint(c, boxes, next, canvas, clip);
        }
    }

    fn shape(&self, sh: &Shape, r: Rect, canvas: &mut Canvas, clip: Rect) {
        let colour = match sh.fill {
            Fill::Rgba(c) => c,
            Fill::Palette { palette, index, alpha } => match self.sheet.palette(palette, index) {
                Some([r, g, b, a]) => [r, g, b, (a as u32 * alpha as u32 / 255) as u8],
                None => return,
            },
        };
        let (x0, y0) = (r.x + sh.x0.resolve(r.w), r.y + sh.y0.resolve(r.h));
        let (x1, y1) = (r.x + sh.x1.resolve(r.w), r.y + sh.y1.resolve(r.h));
        let area = Rect { x: x0, y: y0, w: (x1 - x0).max(1), h: (y1 - y0).max(1) };
        match &sh.text {
            Some((text, font, size)) => {
                let mut s = Style::default();
                s.0.insert("font-name".into(), Value::Url(font.clone()));
                s.0.insert("font-size".into(), Value::Num(f64::from(*size)));
                s.0.insert("horizontal-alignment".into(), Value::Ident(if sh.centred { "center" } else { "left" }.into()));
                self.text(canvas, &s, text, area, colour, if sh.clip { clip.intersect(area) } else { clip });
            }
            None if sh.round => {
                // Coverage from 4 × 4 samples per pixel.
                let (cx, cy) = (area.x as f32 + area.w as f32 / 2.0, area.y as f32 + area.h as f32 / 2.0);
                let (rx, ry) = (area.w as f32 / 2.0, area.h as f32 / 2.0);
                let shown = area.intersect(clip);
                for y in shown.y..shown.y + shown.h {
                    for x in shown.x..shown.x + shown.w {
                        let mut hits = 0u32;
                        for i in 0..16 {
                            let (sx, sy) = (x as f32 + ((i % 4) as f32 + 0.5) / 4.0, y as f32 + ((i / 4) as f32 + 0.5) / 4.0);
                            if ((sx - cx) / rx).powi(2) + ((sy - cy) / ry).powi(2) <= 1.0 {
                                hits += 1;
                            }
                        }
                        if hits > 0 {
                            canvas.blend(x, y, [colour[0], colour[1], colour[2], (colour[3] as u32 * hits / 16) as u8]);
                        }
                    }
                }
            }
            None => canvas.fill(area, colour, clip),
        }
    }

    /// A pattern progress bar (the Ideas view's groups): its `Progress` attribute,
    /// "bars/played" (played in bars, fractional), as up to `max-bars` segments
    /// `separator-width` apart, `unfilled-color` filled with `filled-color` as far as
    /// played; `no-pattern-color` across it when the attribute is empty.
    fn progress(&self, s: &Style, e: &Element, rect: Rect, canvas: &mut Canvas, clip: Rect) {
        let value = e.attrs.iter().find(|(k, _)| k == "Progress").map(|(_, v)| v.as_str());
        let Some((bars, played)) = value.and_then(|v| v.split_once('/')) else {
            if let Some(c) = self.colour(s, e, "no-pattern-color") {
                canvas.fill(rect, c, clip);
            }
            return;
        };
        let bars = bars.parse::<i32>().unwrap_or(1).clamp(1, s.num("max-bars").unwrap_or(8).max(1));
        let played = played.parse::<f64>().unwrap_or(0.0);
        let sep = s.num("separator-width").unwrap_or(0).max(0);
        let (filled, unfilled, separator) =
            (self.colour(s, e, "filled-color"), self.colour(s, e, "unfilled-color"), self.colour(s, e, "separator-color"));
        let span = rect.w - sep * (bars - 1);
        for b in 0..bars {
            // Spread the pixels left over across the segments.
            let x0 = rect.x + b * span / bars + b * sep;
            let x1 = rect.x + (b + 1) * span / bars + b * sep;
            if let Some(c) = unfilled {
                canvas.fill(Rect { x: x0, y: rect.y, w: x1 - x0, h: rect.h }, c, clip);
            }
            let part = ((played - b as f64).clamp(0.0, 1.0) * (x1 - x0) as f64).round() as i32;
            if let (Some(c), true) = (filled, part > 0) {
                canvas.fill(Rect { x: x0, y: rect.y, w: part, h: rect.h }, c, clip);
            }
            if let (Some(c), true) = (separator, b + 1 < bars && sep > 0) {
                canvas.fill(Rect { x: x1, y: rect.y, w: sep, h: rect.h }, c, clip);
            }
        }
    }

    fn text(&self, canvas: &mut Canvas, s: &Style, text: &str, r: Rect, colour: [u8; 4], clip: Rect) {
        let Some(font) = self.text_font(s) else { return };
        let size = font_size(s);
        let width = self.text_width(s, text);
        let x0 = match s.get("horizontal-alignment") {
            Some(Value::Ident(a)) if a == "left" => r.x,
            Some(Value::Ident(a)) if a == "right" => r.x + r.w - width,
            _ => r.x + (r.w - width) / 2,
        };
        let lm = font.horizontal_line_metrics(size);
        let (ascent, descent) = lm.map_or((size, 0.0), |m| (m.ascent, m.descent));
        let text_h = (ascent - descent).round() as i32;
        let top = match s.get("vertical-alignment") {
            Some(Value::Ident(a)) if a == "top" => r.y,
            Some(Value::Ident(a)) if a == "bottom" => r.y + r.h - text_h,
            _ => r.y + (r.h - text_h) / 2,
        };
        let baseline = top + ascent.round() as i32;
        // Small sizes are NI's pixel fonts: drawn without smoothing.
        let crisp = size <= 12.0;
        let mut pen = x0 as f32;
        for c in text.chars() {
            let Some(f) = self.glyph_font(&font, c) else { continue };
            let (m, cov) = f.rasterize(c, size);
            let gx = pen.round() as i32 + m.xmin;
            let gy = baseline - m.height as i32 - m.ymin;
            for row in 0..m.height {
                for col in 0..m.width {
                    let a = cov[row * m.width + col];
                    let a = if crisp { if a >= 128 { 255 } else { 0 } } else { a };
                    let (x, y) = (gx + col as i32, gy + row as i32);
                    if a > 0 && x >= clip.x && y >= clip.y && x < clip.x + clip.w && y < clip.y + clip.h {
                        canvas.blend(x, y, [colour[0], colour[1], colour[2], ((a as u32 * colour[3] as u32) / 255) as u8]);
                    }
                }
            }
            pen += m.advance_width;
        }
    }
}

struct Styled<'e> {
    e: &'e Element,
    style: Style,
    /// (index in `e.children`, child).
    children: Vec<(usize, Styled<'e>)>,
}

fn outer_size(c: &Styled, (w, h): (i32, i32)) -> (i32, i32) {
    let s = &c.style;
    (w + s.outer("left") + s.outer("right"), h + s.outer("top") + s.outer("bottom"))
}

fn font_size(s: &Style) -> f32 {
    s.get("font-size").and_then(Value::num).unwrap_or(8.0) as f32
}

fn pic_includes_margin(s: &Style) -> bool {
    matches!(s.get("picture-includes-margin"), Some(Value::Bool(true)))
}

/// Without an `animation-index`, a picture's frame follows the widget's state:
/// normal, selected, then disabled (when the picture has that many).
fn default_frame(e: &Element, frames: i32) -> i32 {
    if !e.enabled && frames > 2 {
        2
    } else if e.selected && frames > 1 {
        1
    } else {
        0
    }
}

/// Draw `src` of `pic` over `dst`, keeping the `fixed` borders (left, right, top,
/// bottom) as they are and stretching the rest.
fn nine_slice(canvas: &mut Canvas, pic: &Canvas, src: Rect, dst: Rect, fixed: [i32; 4], clip: Rect) {
    let [l, r, t, b] = fixed;
    if l + r == 0 && t + b == 0 {
        return canvas.blit(pic, src, dst, clip);
    }
    let sx = [src.x, src.x + l, src.x + src.w - r, src.x + src.w];
    let sy = [src.y, src.y + t, src.y + src.h - b, src.y + src.h];
    let dx = [dst.x, dst.x + l, dst.x + dst.w - r, dst.x + dst.w];
    let dy = [dst.y, dst.y + t, dst.y + dst.h - b, dst.y + dst.h];
    for j in 0..3 {
        for i in 0..3 {
            let s = Rect { x: sx[i], y: sy[j], w: sx[i + 1] - sx[i], h: sy[j + 1] - sy[j] };
            let d = Rect { x: dx[i], y: dy[j], w: dx[i + 1] - dx[i], h: dy[j + 1] - dy[j] };
            canvas.blit(pic, s, d, clip);
        }
    }
}

/// How many boxes `arrange` makes for `t`: one for it and each widget under it.
fn count_boxes(t: &Styled) -> usize {
    1 + t.children.iter().map(|(_, c)| count_boxes(c)).sum::<usize>()
}

/// A child with a positive z-index or placed by the scripts: laid over its siblings,
/// outside the bar's line.
fn overlay(c: &Styled) -> bool {
    placed(c).is_some() || c.style.num("z-index").unwrap_or(0) > 0
}

/// Where a widget is placed out of its bar's line: by the scripts (`setPos`), else by the
/// sheet's `x` and `y` (the capacitive overlays' icons above the knobs).
fn placed(c: &Styled) -> Option<(i32, i32)> {
    c.e.pos.or_else(|| match (c.style.num("x"), c.style.num("y")) {
        (None, None) => None,
        (x, y) => Some((x.unwrap_or(0), y.unwrap_or(0))),
    })
}

/// Whether a widget's width and height are its own (set or styled), not its bar's.
fn fixed(c: &Styled) -> (bool, bool) {
    (c.e.width.is_some() || c.style.num("width").is_some(), c.e.height.is_some() || c.style.num("height").is_some())
}

fn load_png(path: &Path) -> Option<Canvas> {
    decode_png(std::io::BufReader::new(std::fs::File::open(path).ok()?))
}

fn decode_png(data: impl std::io::BufRead + std::io::Seek) -> Option<Canvas> {
    let mut dec = png::Decoder::new(data);
    dec.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let px = match info.color_type {
        png::ColorType::Rgba => buf.chunks_exact(4).take(w * h).map(|p| [p[0], p[1], p[2], p[3]]).collect(),
        png::ColorType::Rgb => buf.chunks_exact(3).take(w * h).map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).take(w * h).map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => return None,
    };
    Some(Canvas { width: w, height: h, px })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(sheet: &str) -> Look {
        Look {
            sheet: Sheet::parse(sheet),
            roots: Vec::new(),
            pictures: Default::default(),
            fonts: Default::default(),
            fallback: Default::default(),
        }
    }

    fn bar(dir: Dir, class: &str, children: Vec<Element>) -> Element {
        Element { kind: "Bar".into(), class: class.into(), dir, enabled: true, children, ..Default::default() }
    }

    /// A label styled `foreground-color: no` shows no text (the MK3 Channel page's tile
    /// names, BUGS.md #11); without it the text is drawn, white.
    #[test]
    fn no_foreground_colour_draws_no_text() {
        let res = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/ni-res/shared_nwl_font_resources");
        if !res.join("Shared/Roboto-Medium.ttf").is_file() {
            return;
        }
        let draw = |extra: &str| {
            let mut l =
                look(&format!("Label.N {{ width: 40; height: 20; font-name: url(\"Shared/Roboto-Medium.ttf\"); font-size: 14; {extra} }}"));
            l.roots = vec![res.clone()];
            let e = Element { kind: "Label".into(), class: "N".into(), enabled: true, text: Some("INPUT".into()), ..Default::default() };
            l.render(&e, 40, 20).px
        };
        let blank = Canvas::new(40, 20, [0, 0, 0, 255]).px;
        assert!(draw("") != blank, "drawn");
        assert!(draw("foreground-color: no;") == blank, "no colour: nothing");
    }

    /// Characters NI's fonts don't have (emoji in names) are drawn from the emoji font,
    /// each as its own glyph rather than the font's blank box.
    #[test]
    fn emoji_are_drawn_from_the_fallback_font() {
        let res = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/ni-res/shared_nwl_font_resources");
        if !res.join("Shared/Roboto-Medium.ttf").is_file() {
            return;
        }
        let mut l = look("");
        l.roots = vec![res];
        let mut style = Style::default();
        style.0.insert("font-name".into(), Value::Url("Shared/Roboto-Medium.ttf".into()));
        style.0.insert("font-size".into(), Value::Num(14.0));
        let draw = |text: &str| {
            let mut c = Canvas::new(40, 20, [0, 0, 0, 255]);
            let r = Rect { x: 0, y: 0, w: 40, h: 20 };
            l.text(&mut c, &style, text, r, [255, 255, 255, 255], r);
            c.px
        };
        let blank = Canvas::new(40, 20, [0, 0, 0, 255]).px;
        assert!(draw("🥁") != blank, "drawn");
        assert!(draw("🥁") != draw("🔥"), "each its own glyph");
        assert!(l.text_width(&style, "Kick 🔥") > l.text_width(&style, "Kick "));
    }

    #[test]
    fn bars_line_up_children_with_spacing_margins_and_flex() {
        let l = look(
            "Bar.Root { width: 100; height: 20; spacing: 2; margin-left: 3; background-color: rgb(1,1,1); }
             Bar.A { width: 10; background-color: rgb(255,0,0); }
             Bar.B { width: 5; outer-margin-left: 4; background-color: rgb(0,255,0); }",
        );
        let mut root = bar(Dir::Right, "Root", vec![bar(Dir::Right, "A", vec![]), bar(Dir::Right, "B", vec![])]);
        let c = l.render(&root, 100, 20);
        let at = |c: &Canvas, x: usize, y: usize| c.px[y * 100 + x];
        assert_eq!(at(&c, 2, 5), [1, 1, 1, 255], "margin");
        assert_eq!(at(&c, 3, 5), [255, 0, 0, 255], "A at 3..13");
        assert_eq!(at(&c, 12, 19), [255, 0, 0, 255], "A fills the height");
        // B: after A (13) + spacing 2 + outer margin 4 = 19..24.
        assert_eq!(at(&c, 18, 5), [1, 1, 1, 255]);
        assert_eq!(at(&c, 19, 5), [0, 255, 0, 255]);
        assert_eq!(at(&c, 24, 5), [1, 1, 1, 255]);
        root.flex = Some(0);
        let c = l.render(&root, 100, 20);
        // A takes the rest: 100 - 3 - (10 + 2 + 9) = 76 more.
        assert_eq!(at(&c, 88, 5), [255, 0, 0, 255]);
        assert_eq!(at(&c, 95, 5), [0, 255, 0, 255]);
    }

    #[test]
    fn down_bars_and_stacks() {
        let l = look(
            "Bar.Root { width: 10; height: 30; }
             Bar.A { height: 4; background-color: rgb(255,0,0); }
             Bar.B { height: 6; background-color: rgb(0,0,255); }",
        );
        let root = bar(Dir::Down, "Root", vec![bar(Dir::Right, "A", vec![]), bar(Dir::Right, "B", vec![])]);
        let c = l.render(&root, 10, 30);
        assert_eq!(c.px[3 * 10 + 9], [255, 0, 0, 255]);
        assert_eq!(c.px[4 * 10], [0, 0, 255, 255]);
        assert_eq!(c.px[10 * 10], [0, 0, 0, 255]);
        let stack = Element { kind: "Stack".into(), top: Some(1), children: root.children.clone(), ..bar(Dir::Right, "Root", vec![]) };
        let c = l.render(&stack, 10, 30);
        assert_eq!(c.px[0], [0, 0, 255, 255], "only the top child, at the top");
    }

    #[test]
    fn progress_bars_fill_the_bars_reached() {
        let l = look(
            "Bar.P { width: 23; height: 2; max-bars: 8; separator-width: 1; separator-color: rgb(9,9,9);
                     filled-color: rgb(255,0,0); unfilled-color: rgb(0,0,255); no-pattern-color: rgb(0,255,0); }",
        );
        let mut p = Element { attrs: vec![("Progress".into(), "4/1.40".into())], ..bar(Dir::Right, "P", vec![]) };
        let c = l.render(&p, 23, 2);
        // Four segments of 5 with separators at 5, 11 and 17: the first played, the second
        // 40% (2 of its 5 pixels, 6..11).
        let row: Vec<_> = (0..23).map(|x| c.px[x]).collect();
        assert_eq!(row[0], [255, 0, 0, 255]);
        assert_eq!(row[4], [255, 0, 0, 255]);
        assert_eq!(row[5], [9, 9, 9, 255]);
        assert_eq!(row[7], [255, 0, 0, 255], "second bar under way");
        assert_eq!(row[8], [0, 0, 255, 255], "the rest of it not yet");
        assert_eq!(row[12], [0, 0, 255, 255], "third not yet");
        assert_eq!(row[22], [0, 0, 255, 255]);
        p.attrs[0].1 = String::new();
        assert_eq!(l.render(&p, 23, 2).px[12], [0, 255, 0, 255], "no pattern");
    }
}
