//! Plug-in mappings (`docs/superpowers/specs/2026-11-15-plugin-mappings-design.md`): a
//! Lua file of data per plug-in, under `mappings/` in the controller scripts (DECKARD's
//! own, or the user's folder, which wins). It declares the plug-in's parameters (with
//! real units: a taper, or units read from the plug-in's text), the pictures they draw,
//! and knob pages:
//!
//! ```lua
//! return {
//!   api = 1,
//!   match = { name = "Monark", vendor = "Native Instruments" },
//!   params = {
//!     cutoff = { id = 33, taper = { "log", 20, 20000 } },
//!     reso   = { id = 34, knob = { "Resonance", "Emphasis" } },
//!     mode   = { id = 32, choices = { "BP", "LP1", "LP2", "MM" } },
//!     f_dec  = { id = 40, taper = { "log", 4, 20000 } },
//!     f_rel  = { id = 42, choices = { "Off", "On" } },
//!   },
//!   pictures = {
//!     response = { widget = "filter", needs = { "cutoff" },
//!                  model = { cutoff = "cutoff", resonance = "reso",
//!                            shape = { from = "mode", map = { BP = { const = "bp" }, MM = { const = "ladder4" } } } } },
//!     contour  = { widget = "envelope", needs = { "f_dec" },
//!                  model = { decay = "f_dec", release = { from = "f_rel", map = { On = "f_dec", Off = { const = 5 } } } } },
//!   },
//!   pages = {
//!     { name = "FILTER", knobs = { "cutoff", "reso", false, "mode", false, "f_dec", false, "f_rel" },
//!       pictures = { { "response", knobs = { 1, 3 } }, { "contour", knobs = { 5, 8 } } } },
//!     { name = "MORE", left = "response", knobs = { ... } },   -- left = knobs 1-4, right = 5-8
//!   },
//! }
//! ```
//!
//! A widget input (`model`) is a declared parameter's name, `{ const = v }`, or a choice
//! map `{ from = <param>, map = { <choice> = <input>, ... } }` whose values are inputs
//! again (so a switch can pick another parameter). Any other name is an error.
//!
//! A parameter is found on the page's knobs by `id`, then by the names in `knob` (and
//! `section`, if given), then by `name` (its title). Its value in real units is its
//! knob's position through `taper`, else the number and unit in the knob's text. A
//! parameter not on the page leaves the inputs that use it out (the widget's default),
//! and a picture whose `needs` aren't all on the page isn't drawn.

use mlua::{Table, Value};

use super::layout::{Input, Knob, Span};

/// The mapping format this DECKARD reads.
pub const API: u32 = 1;

/// How a parameter's 0..1 position maps to its real units.
#[derive(Clone, Debug, PartialEq)]
pub enum Taper {
    Lin(f32, f32),
    /// Logarithmic from a to b (both above 0): Hz, times.
    Log(f32, f32),
    /// a to b along `t^curve`.
    Exp(f32, f32, f32),
    /// Decibels from a to b, the bottom end at minus infinity if a is.
    Db(f32, f32),
    /// Points (position, value), straight between them.
    Table(Vec<(f32, f32)>),
}

impl Taper {
    pub fn at(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Taper::Lin(a, b) => a + (b - a) * t,
            Taper::Log(a, b) => a * (b / a).powf(t),
            Taper::Exp(a, b, c) => a + (b - a) * t.powf(*c),
            Taper::Db(a, b) => {
                if t <= 0.0 && a.is_infinite() {
                    -200.0
                } else {
                    a.max(-120.0) + (b - a.max(-120.0)) * t
                }
            }
            Taper::Table(pts) => {
                let i = pts.iter().position(|p| p.0 >= t).unwrap_or(pts.len() - 1);
                if i == 0 {
                    return pts[0].1;
                }
                let (p, q) = (pts[i - 1], pts[i]);
                if q.0 - p.0 < 1e-6 { q.1 } else { p.1 + (q.1 - p.1) * (t - p.0) / (q.0 - p.0) }
            }
        }
    }
}

/// A declared parameter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Param {
    pub name: String,
    pub id: Option<u32>,
    pub knob: Vec<String>,
    pub section: Option<String>,
    pub title: Option<String>,
    pub taper: Option<Taper>,
    pub choices: Vec<String>,
}

/// Where a widget input comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Param(String),
    Num(f32),
    Text(String),
    Choice { from: String, map: Vec<(String, Source)> },
}

/// A picture a mapping can show.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub name: String,
    pub widget: String,
    pub needs: Vec<String>,
    pub model: Vec<(String, Source)>,
}

/// A knob page of a mapping's own: a parameter (or none) per knob, and its pictures
/// (name, first knob, last knob; 0..8).
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub name: String,
    pub knobs: [Option<String>; 8],
    pub pictures: Vec<(String, usize, usize)>,
}

/// What a mapping is for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Match {
    pub name: Option<String>,
    pub vendor: Option<String>,
    pub plugin_id: Option<String>,
    pub builtin: Option<String>,
}

/// One mapping file, checked.
#[derive(Clone, Debug, PartialEq)]
pub struct Mapping {
    /// Its script path (`mappings/native-instruments/monark.lua`), and whether it's the
    /// user's.
    pub path: String,
    pub user: bool,
    pub matches: Match,
    pub kind: String,
    pub params: Vec<Param>,
    pub pictures: Vec<Picture>,
    pub pages: Vec<Page>,
}

fn err(path: &str, what: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(format!("{path}: {what}"))
}

fn strings(v: Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.to_string_lossy()],
        Value::Table(t) => t.sequence_values::<String>().filter_map(Result::ok).collect(),
        _ => Vec::new(),
    }
}

fn number(v: &Value) -> Option<f32> {
    match v {
        Value::Integer(n) => Some(*n as f32),
        Value::Number(n) => Some(*n as f32),
        Value::String(s) if s.to_string_lossy() == "-inf" => Some(f32::NEG_INFINITY),
        _ => None,
    }
}

fn taper(path: &str, t: &Table) -> mlua::Result<Taper> {
    let kind: String = t.raw_get(1).map_err(|_| err(path, "a taper starts with its kind"))?;
    let n = |i: usize| -> mlua::Result<f32> {
        number(&t.raw_get::<Value>(i)?).ok_or_else(|| err(path, format!("taper \"{kind}\": value {i} isn't a number")))
    };
    Ok(match kind.as_str() {
        "lin" => Taper::Lin(n(2)?, n(3)?),
        "log" => {
            let (a, b) = (n(2)?, n(3)?);
            if a <= 0.0 || b <= 0.0 {
                return Err(err(path, "a log taper's ends are above 0"));
            }
            Taper::Log(a, b)
        }
        "exp" => Taper::Exp(n(2)?, n(3)?, n(4).unwrap_or(2.0)),
        "db" => Taper::Db(n(2)?, n(3)?),
        "table" => {
            let mut pts = Vec::new();
            for p in t.sequence_values::<Value>().skip(1) {
                let Value::Table(p) = p? else { return Err(err(path, "a table taper's points are {position, value}")) };
                let (a, b) = (number(&p.raw_get(1)?), number(&p.raw_get(2)?));
                pts.push((a.ok_or_else(|| err(path, "a table taper point"))?, b.ok_or_else(|| err(path, "a table taper point"))?));
            }
            if pts.len() < 2 {
                return Err(err(path, "a table taper has two points or more"));
            }
            Taper::Table(pts)
        }
        _ => return Err(err(path, format!("no taper \"{kind}\" (lin, log, exp, db, table)"))),
    })
}

impl Mapping {
    /// Reads and checks a mapping's table.
    pub fn from_lua(path: &str, user: bool, t: &Table) -> mlua::Result<Mapping> {
        let api: u32 = t.raw_get::<Option<u32>>("api")?.ok_or_else(|| err(path, "no api (the mapping format: api = 1)"))?;
        if api > API {
            return Err(err(path, format!("written for mapping format {api}; this DECKARD reads up to {API}")));
        }
        let m: Table = t.raw_get::<Option<Table>>("match")?.ok_or_else(|| err(path, "no match"))?;
        let matches = Match {
            name: m.raw_get("name")?,
            vendor: m.raw_get("vendor")?,
            plugin_id: m.raw_get("plugin_id")?,
            builtin: m.raw_get("builtin")?,
        };
        if matches.name.is_none() && matches.plugin_id.is_none() && matches.builtin.is_none() {
            return Err(err(path, "match names the plug-in: name, plugin_id or builtin"));
        }
        let mut params = Vec::new();
        if let Some(pt) = t.raw_get::<Option<Table>>("params")? {
            for pair in pt.pairs::<String, Table>() {
                let (name, p) = pair.map_err(|e| err(path, format!("params: {e}")))?;
                let id = match p.raw_get::<Value>("id")? {
                    Value::Nil => None,
                    v => Some(number(&v).filter(|n| *n >= 0.0).ok_or_else(|| err(path, format!("param {name}: id is a number")))? as u32),
                };
                params.push(Param {
                    id,
                    knob: strings(p.raw_get("knob")?),
                    section: p.raw_get("section")?,
                    title: p.raw_get("name")?,
                    taper: p.raw_get::<Option<Table>>("taper")?.map(|t| taper(path, &t)).transpose()?,
                    choices: strings(p.raw_get("choices")?),
                    name,
                });
            }
        }
        params.sort_by(|a, b| a.name.cmp(&b.name));
        let declared = |n: &str| params.iter().any(|p| p.name == n);
        fn source(path: &str, v: Value, declared: &dyn Fn(&str) -> bool) -> mlua::Result<Source> {
            Ok(match v {
                Value::String(s) => {
                    let s = s.to_string_lossy();
                    if !declared(&s) {
                        return Err(err(path, format!("\"{s}\" isn't declared under params (a constant is {{ const = \"{s}\" }})")));
                    }
                    Source::Param(s)
                }
                Value::Integer(_) | Value::Number(_) => Source::Num(number(&v).unwrap_or(0.0)),
                Value::Table(t) => match t.raw_get::<Value>("const")? {
                    Value::Nil => {
                        let from: String =
                            t.raw_get::<Option<String>>("from")?.ok_or_else(|| err(path, "a choice map has from and map"))?;
                        if !declared(&from) {
                            return Err(err(path, format!("\"{from}\" isn't declared under params")));
                        }
                        let mt: Table = t.raw_get::<Option<Table>>("map")?.ok_or_else(|| err(path, "a choice map has from and map"))?;
                        let mut map = Vec::new();
                        for pair in mt.pairs::<String, Value>() {
                            let (k, v) = pair?;
                            map.push((k, source(path, v, declared)?));
                        }
                        map.sort_by(|a, b| a.0.cmp(&b.0));
                        Source::Choice { from, map }
                    }
                    c @ (Value::Integer(_) | Value::Number(_)) => Source::Num(number(&c).unwrap_or(0.0)),
                    Value::String(s) => Source::Text(s.to_string_lossy()),
                    _ => return Err(err(path, "const is a number or a string")),
                },
                _ => return Err(err(path, "a widget input is a parameter's name, { const = v } or { from = , map = }")),
            })
        }
        let mut pictures = Vec::new();
        if let Some(pt) = t.raw_get::<Option<Table>>("pictures")? {
            for pair in pt.pairs::<String, Table>() {
                let (name, p) = pair.map_err(|e| err(path, format!("pictures: {e}")))?;
                let widget: String =
                    p.raw_get::<Option<String>>("widget")?.ok_or_else(|| err(path, format!("picture {name}: no widget")))?;
                let needs = strings(p.raw_get("needs")?);
                for n in &needs {
                    if !declared(n) {
                        return Err(err(path, format!("picture {name}: needs \"{n}\", which isn't declared under params")));
                    }
                }
                let mut model = Vec::new();
                if let Some(mt) = p.raw_get::<Option<Table>>("model")? {
                    for pair in mt.pairs::<String, Value>() {
                        let (k, v) = pair?;
                        model.push((k, source(path, v, &declared).map_err(|e| err(path, format!("picture {name}: {e}")))?));
                    }
                }
                model.sort_by(|a, b| a.0.cmp(&b.0));
                pictures.push(Picture { name, widget, needs, model });
            }
        }
        pictures.sort_by(|a, b| a.name.cmp(&b.name));
        let picture = |n: &str| pictures.iter().any(|p| p.name == n);
        let mut pages = Vec::new();
        if let Some(pt) = t.raw_get::<Option<Table>>("pages")? {
            for (i, p) in pt.sequence_values::<Table>().enumerate() {
                let p = p?;
                let name: String = p.raw_get::<Option<String>>("name")?.unwrap_or_else(|| format!("PAGE {}", i + 1));
                let mut knobs: [Option<String>; 8] = Default::default();
                if let Some(kt) = p.raw_get::<Option<Table>>("knobs")? {
                    for (k, slot) in knobs.iter_mut().enumerate() {
                        match kt.raw_get::<Value>(k + 1)? {
                            Value::String(s) => {
                                let s = s.to_string_lossy();
                                if !declared(&s) {
                                    return Err(err(
                                        path,
                                        format!("page {name}: knob {} is \"{s}\", which isn't declared under params", k + 1),
                                    ));
                                }
                                *slot = Some(s);
                            }
                            Value::Nil | Value::Boolean(false) => {}
                            _ => return Err(err(path, format!("page {name}: knob {} is a parameter's name or false", k + 1))),
                        }
                    }
                }
                let mut pics = Vec::new();
                for (side, first) in [("left", 0), ("right", 4)] {
                    if let Some(n) = p.raw_get::<Option<String>>(side)? {
                        pics.push((n, first, first + 3));
                    }
                }
                if let Some(list) = p.raw_get::<Option<Table>>("pictures")? {
                    for e in list.sequence_values::<Table>() {
                        let e = e?;
                        let n: String =
                            e.raw_get(1).map_err(|_| err(path, format!("page {name}: a picture is {{ \"name\", knobs = {{ a, b }} }}")))?;
                        let ks: Table =
                            e.raw_get("knobs").map_err(|_| err(path, format!("page {name}: picture {n} has knobs = {{ a, b }}")))?;
                        let (a, b): (usize, usize) = (ks.raw_get(1)?, ks.raw_get(2)?);
                        if !(1..=8).contains(&a) || !(a..=8).contains(&b) || (a - 1) / 4 != (b - 1) / 4 || b - a < 1 {
                            return Err(err(
                                path,
                                format!("page {name}: picture {n} spans knobs {a}-{b}: two to four knobs on one screen"),
                            ));
                        }
                        pics.push((n, a - 1, b - 1));
                    }
                }
                for (n, ..) in &pics {
                    if !picture(n) {
                        return Err(err(path, format!("page {name}: no picture \"{n}\"")));
                    }
                }
                pages.push(Page { name, knobs, pictures: pics });
            }
        }
        Ok(Mapping {
            path: path.to_string(),
            user,
            matches,
            kind: t.raw_get::<Option<String>>("kind")?.unwrap_or_default(),
            params,
            pictures,
            pages,
        })
    }

    fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.name == name)
    }

    /// How well this mapping matches a plug-in (higher is better), or `None`.
    pub fn score(&self, name: &str, vendor: &str, plugin_id: &str) -> Option<u32> {
        let m = &self.matches;
        if let Some(b) = &m.builtin {
            return (b.eq_ignore_ascii_case(name) || b.eq_ignore_ascii_case(plugin_id)).then_some(3);
        }
        if m.plugin_id.as_deref().is_some_and(|id| id == plugin_id) {
            return Some(3);
        }
        let named = m.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name.trim()));
        let vendor_ok = m.vendor.as_deref().is_none_or(|v| vendor.is_empty() || v.eq_ignore_ascii_case(vendor.trim()));
        (named && vendor_ok).then_some(if m.vendor.is_some() && !vendor.is_empty() { 2 } else { 1 })
    }

    /// The knob each parameter is on, on a page of `knobs` (`ids`: each knob's
    /// parameter id, if known).
    fn find_on(&self, knobs: &[Option<Knob>; 8], ids: &[Option<u32>; 8]) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        for p in &self.params {
            let by_id = p.id.and_then(|id| (0..8).find(|&k| ids[k] == Some(id)));
            let by_knob = || {
                (0..8).find(|&k| {
                    knobs[k].as_ref().is_some_and(|kn| {
                        p.knob.iter().any(|n| n.eq_ignore_ascii_case(kn.name.trim()))
                            && p.section.as_deref().is_none_or(|s| s.eq_ignore_ascii_case(&kn.section))
                    })
                })
            };
            let by_title = || {
                let t = p.title.as_deref()?;
                (0..8).find(|&k| knobs[k].as_ref().is_some_and(|kn| kn.name.eq_ignore_ascii_case(t)))
            };
            if let Some(k) = by_id.or_else(by_knob).or_else(by_title) {
                out.push((p.name.clone(), k));
            }
        }
        out
    }

    /// Parameter `name`'s value in its units, from its knob.
    fn value(&self, name: &str, knob: &Knob) -> Option<Input> {
        let p = self.param(name)?;
        if !p.choices.is_empty() {
            return Some(Input::Text(knob.text.trim().to_string()));
        }
        if let Some(t) = &p.taper {
            return Some(Input::Num(t.at(knob.value)));
        }
        // (Real units from the text, as the generic patterns read them.)
        let text = knob.text.trim();
        let v = super::layout::hz(text).or_else(|| super::layout::ms(text)).or_else(|| super::layout::db(text));
        Some(Input::Num(v.unwrap_or(knob.value)))
    }

    /// Parameter `name`'s modulated value in its units, if modulation moves its knob.
    fn mod_value(&self, name: &str, knob: &Knob) -> Option<f32> {
        let p = self.param(name)?;
        if !p.choices.is_empty() {
            return None;
        }
        if let Some(t) = &p.taper {
            return knob.mod_value.map(|v| t.at(v));
        }
        let text = knob.mod_text.as_deref()?.trim();
        super::layout::hz(text).or_else(|| super::layout::ms(text)).or_else(|| super::layout::db(text)).or(knob.mod_value)
    }

    fn resolve(&self, s: &Source, on: &[(String, usize)], knobs: &[Option<Knob>; 8], depth: usize) -> Option<Input> {
        match s {
            Source::Num(n) => Some(Input::Num(*n)),
            Source::Text(t) => Some(Input::Text(t.clone())),
            Source::Param(p) => {
                let k = on.iter().find(|(n, _)| n == p)?.1;
                self.value(p, knobs[k].as_ref()?)
            }
            Source::Choice { from, map } => {
                if depth > 4 {
                    return None;
                }
                let k = on.iter().find(|(n, _)| n == from)?.1;
                let choice = knobs[k].as_ref()?.text.trim().to_string();
                let (_, to) = map.iter().find(|(c, _)| c.eq_ignore_ascii_case(&choice))?;
                self.resolve(to, on, knobs, depth + 1)
            }
        }
    }

    /// The pictures this mapping draws over a page of knobs: the page's own placement if
    /// the knobs are one of the mapping's pages, else each picture wherever its `needs`
    /// sit on adjacent knobs of one screen (spec: "Laying out pictures on any page").
    pub fn spans(&self, knobs: &[Option<Knob>; 8], ids: &[Option<u32>; 8]) -> Vec<Span> {
        let on = self.find_on(knobs, ids);
        let knob_of = |n: &str| on.iter().find(|(p, _)| p == n).map(|x| x.1);
        // (A page of the mapping's that places no pictures lays them out as any page.)
        let page = self.pages.iter().find(|pg| {
            !pg.pictures.is_empty()
                && pg.knobs.iter().any(Option::is_some)
                && pg.knobs.iter().enumerate().all(|(k, p)| p.as_deref().is_none_or(|p| knob_of(p) == Some(k)))
        });
        let mut placed: Vec<(&Picture, usize, usize)> = Vec::new();
        if let Some(page) = page {
            for (n, a, b) in &page.pictures {
                if let Some(pic) = self.pictures.iter().find(|p| &p.name == n) {
                    placed.push((pic, *a, *b));
                }
            }
        } else {
            let mut taken = [false; 8];
            for pic in &self.pictures {
                let ks: Option<Vec<usize>> = pic.needs.iter().map(|n| knob_of(n)).collect();
                let Some(mut ks) = ks else { continue };
                if ks.is_empty() {
                    continue;
                }
                ks.sort();
                let (lo, hi) = (ks[0], ks[ks.len() - 1]);
                if lo / 4 != hi / 4 || hi - lo + 1 != ks.len() {
                    continue;
                }
                // Widened to two knobs inside the screen.
                let (lo, hi) = if hi > lo {
                    (lo, hi)
                } else if lo % 4 < 3 {
                    (lo, lo + 1)
                } else {
                    (lo - 1, lo)
                };
                if (lo..=hi).any(|k| taken[k]) {
                    continue;
                }
                for t in &mut taken[lo..=hi] {
                    *t = true;
                }
                placed.push((pic, lo, hi));
            }
        }
        let mut out = Vec::new();
        for (pic, a, b) in placed {
            if pic.needs.iter().any(|n| knob_of(n).is_none()) {
                continue;
            }
            let mut inputs = Vec::new();
            let mut bound = Vec::new();
            for (input, src) in &pic.model {
                if let Some(v) = self.resolve(src, &on, knobs, 0) {
                    inputs.push((input.clone(), v));
                }
                if let Source::Param(p) = src
                    && let Some(k) = knob_of(p)
                {
                    bound.push((k, input.clone()));
                    if let Some(v) = knobs[k].as_ref().and_then(|kn| self.mod_value(p, kn)) {
                        inputs.push((format!("{input}_mod"), Input::Num(v)));
                    }
                }
            }
            out.push(Span { first: a, cells: b - a + 1, kind: pic.widget.clone(), inputs, knobs: bound });
        }
        out
    }
}

/// A knob page of a mapping's own, for the app to put on the knobs: each knob's
/// parameter by id, title or knob names (`None`: an empty knob).
#[derive(Clone, Debug, PartialEq)]
pub struct PageParams {
    pub name: String,
    pub knobs: [Option<ParamRef>; 8],
}

/// How the app finds a mapping's parameter among the plug-in's.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamRef {
    pub id: Option<u32>,
    pub title: Option<String>,
    pub knob: Vec<String>,
}

impl Mapping {
    /// The mapping's own knob pages.
    pub fn page_params(&self) -> Vec<PageParams> {
        self.pages
            .iter()
            .map(|pg| PageParams {
                name: pg.name.clone(),
                knobs: std::array::from_fn(|k| {
                    let p = self.param(pg.knobs[k].as_deref()?)?;
                    Some(ParamRef { id: p.id, title: p.title.clone(), knob: p.knob.clone() })
                }),
            })
            .collect()
    }
}

/// Every mapping in the controller scripts' `mappings/` folders (the user's file
/// replacing DECKARD's of the same path), checked, and the errors of those that aren't.
pub fn load_all(lua: &mlua::Lua, sources: &crate::sources::Sources) -> (Vec<Mapping>, Vec<String>) {
    let (mut out, mut errors) = (Vec::new(), Vec::new());
    for (path, user) in sources.mapping_files() {
        let Some((chunk, text)) = sources.get(&path) else { continue };
        let res = lua.load(text).set_name(chunk).eval::<Table>().and_then(|t| Mapping::from_lua(&path, user, &t));
        match res {
            Ok(m) => out.push(m),
            Err(e) => errors.push(format!("mapping {path}: {}", e.to_string().lines().next().unwrap_or_default())),
        }
    }
    (out, errors)
}

/// The best mapping for a plug-in: the user's over DECKARD's, then the most specific.
pub fn find<'a>(all: &'a [Mapping], name: &str, vendor: &str, plugin_id: &str) -> Option<&'a Mapping> {
    all.iter().filter_map(|m| m.score(name, vendor, plugin_id).map(|s| (m, (m.user, s)))).max_by_key(|(_, k)| *k).map(|(m, _)| m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONARKISH: &str = r#"return {
      api = 1,
      match = { name = "Monark", vendor = "Native Instruments" },
      params = {
        cutoff = { id = 33, taper = { "log", 20, 20000 } },
        reso = { id = 34, taper = { "lin", 0, 1 } },
        mode = { id = 32, choices = { "BP", "LP1", "LP2", "MM" } },
        f_att = { id = 39, taper = { "log", 1, 10000 } },
        f_dec = { id = 40, taper = { "log", 4, 20000 } },
        f_sus = { id = 41, taper = { "lin", 0, 1 } },
        f_rel = { id = 42, choices = { "Off", "On" } },
      },
      pictures = {
        response = { widget = "filter", needs = { "cutoff" },
          model = { cutoff = "cutoff", resonance = "reso",
                    shape = { from = "mode", map = { BP = { const = "bp" }, MM = { const = "ladder4" } } } } },
        contour = { widget = "envelope", needs = { "f_att", "f_dec" },
          model = { attack = "f_att", decay = "f_dec", sustain = "f_sus",
                    release = { from = "f_rel", map = { On = "f_dec", Off = { const = 5 } } } } },
      },
      pages = {
        { name = "FILTER", knobs = { "cutoff", "reso", false, "mode", "f_att", "f_dec", "f_sus", "f_rel" },
          pictures = { { "response", knobs = { 1, 3 } }, { "contour", knobs = { 5, 8 } } } },
      },
    }"#;

    fn load(src: &str) -> mlua::Result<Mapping> {
        let lua = mlua::Lua::new();
        let t: Table = lua.load(src).eval()?;
        Mapping::from_lua("mappings/test.lua", true, &t)
    }

    fn knob(name: &str, text: &str, value: f32) -> Option<Knob> {
        Some(Knob { name: name.into(), text: text.into(), value, ..Default::default() })
    }

    #[test]
    fn tapers() {
        assert!((Taper::Log(20.0, 20000.0).at(0.5) - 632.5).abs() < 1.0);
        assert_eq!(Taper::Lin(0.0, 10.0).at(0.3), 3.0);
        assert_eq!(Taper::Table(vec![(0.0, 0.0), (0.5, 100.0), (1.0, 1000.0)]).at(0.75), 550.0);
        assert_eq!(Taper::Db(f32::NEG_INFINITY, 6.0).at(0.0), -200.0);
    }

    #[test]
    fn a_mapping_checks_its_names() {
        let m = load(MONARKISH).unwrap();
        assert_eq!(m.params.len(), 7);
        assert_eq!(m.pages[0].pictures, [("response".to_string(), 0, 2), ("contour".to_string(), 4, 7)]);
        let bad = MONARKISH.replace("attack = \"f_att\"", "attack = \"f_attack\"");
        assert!(load(&bad).unwrap_err().to_string().contains("f_attack"), "an undeclared name");
        let bad = MONARKISH.replace("knobs = { 1, 3 }", "knobs = { 3, 6 }");
        assert!(load(&bad).unwrap_err().to_string().contains("one screen"));
        assert!(load("return { match = { name = 'x' } }").is_err(), "no api");
        assert!(load("return { api = 9, match = { name = 'x' } }").unwrap_err().to_string().contains("format 9"));
    }

    #[test]
    fn matching_prefers_the_users_and_the_most_specific() {
        let mut a = load(MONARKISH).unwrap();
        assert_eq!(a.score("Monark", "Native Instruments", ""), Some(2));
        assert_eq!(a.score("monark", "", ""), Some(1));
        assert_eq!(a.score("Monark", "Acme", ""), None);
        assert_eq!(a.score("Reaktor 6", "Native Instruments", ""), None);
        a.user = false;
        let mut b = a.clone();
        b.user = true;
        b.path = "mine".into();
        let all = [a, b];
        assert_eq!(find(&all, "Monark", "Native Instruments", "").unwrap().path, "mine");
    }

    #[test]
    fn its_page_places_its_pictures_with_real_units() {
        let m = load(MONARKISH).unwrap();
        let knobs = [
            knob("Cutoff", " 51", 0.5),
            knob("Resonance", " 5.1", 0.4),
            None,
            knob("FltType", "MM", 1.0),
            knob("Flt Att", "  0", 0.0),
            knob("Flt Dec", " 51", 0.5),
            knob("Flt Sus", " 5.1", 0.5),
            knob("Flt Rel", "On", 1.0),
        ];
        let ids = [Some(33), Some(34), None, Some(32), Some(39), Some(40), Some(41), Some(42)];
        let spans = m.spans(&knobs, &ids);
        assert_eq!(spans.len(), 2);
        assert_eq!((spans[0].kind.as_str(), spans[0].first, spans[0].cells), ("filter", 0, 3));
        let input = |s: &Span, n: &str| s.inputs.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
        assert!(matches!(input(&spans[0], "cutoff"), Some(Input::Num(hz)) if (hz - 632.5).abs() < 1.0));
        assert_eq!(input(&spans[0], "shape"), Some(Input::Text("ladder4".into())));
        assert_eq!(spans[0].focus_of(1), Some("resonance"));
        // The release switch on: the release is the decay's time.
        assert_eq!(input(&spans[1], "release"), input(&spans[1], "decay"));
        let mut off = knobs.clone();
        off[7] = knob("Flt Rel", "Off", 0.0);
        assert_eq!(input(&m.spans(&off, &ids)[1], "release"), Some(Input::Num(5.0)));
        // Modulation moving the cutoff: its modulated position through the taper.
        let mut moved = knobs.clone();
        moved[0].as_mut().unwrap().mod_value = Some(1.0);
        assert_eq!(input(&m.spans(&moved, &ids)[0], "cutoff_mod"), Some(Input::Num(20000.0)));
    }

    #[test]
    fn on_another_page_pictures_go_where_their_needs_are() {
        let m = load(MONARKISH).unwrap();
        // An NKS page with the filter envelope on knobs 2-3 and the cutoff on knob 6.
        let knobs = [None, knob("Flt Att", "  0", 0.0), knob("Flt Dec", " 51", 0.5), None, None, knob("Cutoff", " 51", 0.5), None, None];
        let ids = [None, Some(39), Some(40), None, None, Some(33), None, None];
        let spans: Vec<_> = m.spans(&knobs, &ids).into_iter().map(|s| (s.kind, s.first, s.cells)).collect();
        assert_eq!(spans, [("envelope".to_string(), 1, 2), ("filter".to_string(), 5, 2)]);
    }
}
