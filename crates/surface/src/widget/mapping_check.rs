//! Checking a plug-in mapping without the plug-in: against its parameter dump
//! (`plugin-probe dump`, the plug-in's parameters and their text at 64 values), as the
//! spec's "Checking a mapping" asks.
//!
//! - **Shipped mappings** (`crates/surface/lua/mappings/`) each come with their dump (the parameters
//!   they name) in `crates/surface/tests/mappings/<plugin>.json`; `shipped_mappings_check_against_their_dumps`
//!   (run with the other tests, so in CI) fails on any problem.
//! - **Any mapping**, a user's included: `scripts/mapping-check.sh <mapping.lua>
//!   <dump.json> [out folder]` runs `check_a_mapping` (ignored by default), which lists
//!   the problems and renders each of the mapping's pages to PNGs (knobs at their
//!   defaults, at 0 and at 1; the default and `lcd` themes) to look at.

use std::path::{Path, PathBuf};

use super::mapping::{Mapping, Source};
use crate::{KnobView, Surface};

/// One parameter of a dump.
#[derive(Clone, Debug, PartialEq)]
pub struct DumpParam {
    pub id: u32,
    pub title: String,
    pub default: f64,
    /// The plug-in's text at 64 evenly spaced values, 0 to 1.
    pub text: Vec<String>,
}

impl DumpParam {
    /// The text at normalised value `v` (the nearest of the 64).
    pub fn text_at(&self, v: f64) -> String {
        let n = self.text.len().max(1) - 1;
        self.text.get((v.clamp(0.0, 1.0) * n as f64).round() as usize).map_or_else(String::new, |t| t.trim().to_string())
    }
}

/// Reads a dump (`plugin-probe dump`'s JSON).
pub fn read_dump(path: &Path) -> Result<Vec<DumpParam>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let list = v["parameters"].as_array().ok_or_else(|| format!("{}: no parameters", path.display()))?;
    Ok(list
        .iter()
        .filter_map(|p| {
            Some(DumpParam {
                id: p["id"].as_u64()? as u32,
                title: p["title"].as_str().unwrap_or("").to_string(),
                default: p["default"].as_f64().unwrap_or(0.0),
                text: p["text"].as_array().map(|t| t.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            })
        })
        .collect())
}

/// Reads and checks a mapping file (as DECKARD loads it).
pub fn read_mapping(path: &Path) -> Result<Mapping, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let lua = mlua::Lua::new();
    let t: mlua::Table = lua.load(text).set_name(format!("@{}", path.display())).eval().map_err(|e| e.to_string())?;
    Mapping::from_lua(&path.display().to_string(), true, &t).map_err(|e| e.to_string())
}

/// The dump's parameter a mapping's parameter `name` is: by id, else its title, else its
/// knob names.
fn find<'a>(m: &Mapping, name: &str, dump: &'a [DumpParam]) -> Option<&'a DumpParam> {
    let p = m.params.iter().find(|p| p.name == name)?;
    if let Some(id) = p.id {
        return dump.iter().find(|d| d.id == id);
    }
    let titles: Vec<&str> = p.title.iter().map(String::as_str).chain(p.knob.iter().map(String::as_str)).collect();
    dump.iter().find(|d| titles.iter().any(|t| t.eq_ignore_ascii_case(d.title.trim())))
}

/// Everything wrong with a mapping against its plug-in's dump.
pub fn problems(m: &Mapping, dump: &[DumpParam]) -> Vec<String> {
    let mut out = Vec::new();
    for p in &m.params {
        let Some(d) = find(m, &p.name, dump) else {
            out.push(match p.id {
                Some(id) => format!("param {}: the plug-in has no parameter {id}", p.name),
                None => format!("param {}: no parameter titled {:?}", p.name, p.title.iter().chain(&p.knob).collect::<Vec<_>>()),
            });
            continue;
        };
        for c in &p.choices {
            if !d.text.iter().any(|t| t.trim().eq_ignore_ascii_case(c)) {
                out.push(format!(
                    "param {}: choice {c:?} never shows in its text ({:?} ...)",
                    p.name,
                    d.text.iter().take(3).collect::<Vec<_>>()
                ));
            }
        }
    }
    // Inputs read in real units need a taper, or units in the text.
    let units = |d: &DumpParam| {
        let t = d.text_at(d.default);
        super::layout::hz(&t).or_else(|| super::layout::ms(&t)).or_else(|| super::layout::db(&t)).is_some()
    };
    // (Picture, input, the parameter it reads.)
    let mut reads: Vec<(&str, &str, &str)> = Vec::new();
    for pic in &m.pictures {
        for (input, src) in &pic.model {
            match src {
                Source::Param(n) => reads.push((&pic.name, input, n)),
                Source::Choice { from, map } => {
                    let choices = m.params.iter().find(|p| &p.name == from).map(|p| p.choices.clone()).unwrap_or_default();
                    for (k, v) in map {
                        if !choices.is_empty() && !choices.iter().any(|c| c.eq_ignore_ascii_case(k)) {
                            out.push(format!("picture {}: input {input} maps {k:?}, which isn't one of {from}'s choices", pic.name));
                        }
                        if let Source::Param(n) = v {
                            reads.push((&pic.name, input, n));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for (pic, input, name) in reads {
        let Some(p) = m.params.iter().find(|p| p.name == name) else { continue };
        if p.taper.is_none()
            && p.choices.is_empty()
            && let Some(d) = find(m, name, dump)
            && !units(d)
        {
            out.push(format!(
                "picture {pic}: input {input} reads {name}, whose text ({:?}) gives no Hz, ms or dB: declare a taper (measure it: plugin-probe measure)",
                d.text_at(d.default)
            ));
        }
    }
    out
}

/// A knob for the mapping's parameter `name` at normalised `v` (its default if `None`).
fn knob(m: &Mapping, name: &str, dump: &[DumpParam], v: Option<f64>) -> Option<KnobView> {
    let d = find(m, name, dump)?;
    let v = v.unwrap_or(d.default);
    Some(KnobView { name: d.title.trim().to_string(), text: d.text_at(v), value: v, param_id: Some(d.id), ..Default::default() })
}

/// Renders each of the mapping's pages to `out` (as `<page>-<case>-<theme>.png`):
/// knobs at their defaults, at 0 and at 1. Returns the files written.
pub fn render_pages(path: &Path, m: &Mapping, dump: &[DumpParam], out: &Path) -> Result<Vec<PathBuf>, String> {
    // (In a scripts folder of its own, as a user's mapping.)
    let dir = std::env::temp_dir().join(format!("deckard-mapping-check-{}", std::process::id()));
    let maps = dir.join("mappings").join("check");
    std::fs::create_dir_all(&maps).map_err(|e| e.to_string())?;
    std::fs::copy(path, maps.join("mapping.lua")).map_err(|e| e.to_string())?;
    let mut s = Surface::with_user_dir(Some(dir.clone()));
    if let Some(e) = s.error() {
        return Err(e);
    }
    let name = m.matches.name.clone().or_else(|| m.matches.builtin.clone()).unwrap_or_else(|| "Plug-in".into());
    let mut v = crate::tests_support::view();
    v.control.modules_visible = true;
    v.control.chain[0].name = name.clone();
    v.control.chain[0].plugin = true;
    v.control.chain[0].instrument = m.kind == "synth";
    v.control.slot = Some(name);
    v.control.pages = m.pages.iter().map(|p| p.name.clone()).collect();
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut written = Vec::new();
    for theme in ["maschine", "lcd"] {
        s.set_theme(theme);
        for (i, page) in m.pages.iter().enumerate() {
            v.control.page = i;
            for (case, at) in [("default", None), ("min", Some(0.0)), ("max", Some(1.0))] {
                v.control.knobs = page.knobs.iter().map(|k| k.as_deref().and_then(|k| knob(m, k, dump, at))).collect();
                let px = s.picture(&v);
                if let Some(e) = s.error() {
                    return Err(format!("page {}: {e}", page.name));
                }
                let file = out.join(format!("{}-{case}-{theme}.png", page.name.to_ascii_lowercase().replace(' ', "-")));
                super::golden::write_png(&file, crate::screen::WIDTH as u32, crate::screen::HEIGHT as u32, &px);
                written.push(file);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump() -> Vec<DumpParam> {
        let t = |f: &dyn Fn(f64) -> String| (0..64).map(|i| f(i as f64 / 63.0)).collect::<Vec<_>>();
        vec![
            DumpParam { id: 3, title: "Cutoff".into(), default: 0.5, text: t(&|v| format!("{:.0}", v * 100.0)) },
            DumpParam { id: 4, title: "Mode".into(), default: 0.0, text: t(&|v| if v < 0.5 { "LP".into() } else { "HP".into() }) },
            DumpParam { id: 5, title: "Attack".into(), default: 0.2, text: t(&|v| format!("{:.1} ms", 1.0 + v * 999.0)) },
        ]
    }

    fn mapping(src: &str) -> Mapping {
        let lua = mlua::Lua::new();
        Mapping::from_lua("test", true, &lua.load(src).eval().unwrap()).unwrap()
    }

    #[test]
    fn a_mapping_is_checked_against_its_dump() {
        let good = mapping(
            r#"return { api = 1, match = { name = "X" },
              params = { cut = { id = 3, taper = { "log", 20, 20000 } }, mode = { id = 4, choices = { "LP", "HP" } }, att = { name = "Attack" } },
              pictures = { f = { widget = "filter", needs = { "cut" },
                                 model = { cutoff = "cut", shape = { from = "mode", map = { LP = { const = "lp" }, HP = { const = "hp" } } } } },
                           e = { widget = "envelope", needs = { "att" }, model = { attack = "att" } } } }"#,
        );
        assert_eq!(problems(&good, &dump()), Vec::<String>::new());
        let bad = mapping(
            r#"return { api = 1, match = { name = "X" },
              params = { cut = { id = 3 }, mode = { id = 4, choices = { "LP", "BP" } }, gone = { id = 99 } },
              pictures = { f = { widget = "filter", needs = { "cut" },
                                 model = { cutoff = "cut", shape = { from = "mode", map = { NOTCH = { const = "notch" } } } } } } }"#,
        );
        let p = problems(&bad, &dump());
        assert!(p.iter().any(|s| s.contains("no parameter 99")), "{p:?}");
        assert!(p.iter().any(|s| s.contains("\"BP\" never shows")), "{p:?}");
        assert!(p.iter().any(|s| s.contains("declare a taper")), "a cutoff reading 50: {p:?}");
        assert!(p.iter().any(|s| s.contains("\"NOTCH\"")), "{p:?}");
    }

    #[test]
    fn pages_render_against_a_dump() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lua");
        std::fs::write(
            &path,
            r#"return { api = 1, match = { name = "X" },
              params = { cut = { id = 3, taper = { "log", 20, 20000 } }, att = { id = 5 } },
              pictures = { f = { widget = "filter", needs = { "cut" }, model = { cutoff = "cut" } } },
              pages = { { name = "MAIN", left = "f", knobs = { "cut", "att", false, false } } } }"#,
        )
        .unwrap();
        let m = read_mapping(&path).unwrap();
        let files = render_pages(&path, &m, &dump(), &dir.path().join("out")).unwrap();
        assert_eq!(files.len(), 6, "one page, three cases, two themes");
        assert!(files.iter().all(|f| f.exists()));
    }

    /// The worked example in `docs/controller-api.md` ("Example: Acme Ladder") loads.
    #[test]
    fn the_documented_example_loads() {
        let doc = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/controller-api.md")).unwrap();
        let from = doc.find("### Example: Acme Ladder").expect("the example");
        let code = &doc[from..];
        let start = code
            .find(
                "```lua
",
            )
            .expect("its code")
            + 7;
        let end = start + code[start..].find("```").expect("its end");
        let lua = mlua::Lua::new();
        let t: mlua::Table = lua.load(&code[start..end]).eval().unwrap();
        let m = Mapping::from_lua("docs", true, &t).unwrap();
        assert!(!m.pages.is_empty() && !m.pictures.is_empty());
    }

    /// Every mapping DECKARD ships passes its checks against its plug-in's dump.
    #[test]
    fn shipped_mappings_check_against_their_dumps() {
        let dumps = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("mappings");
        let mut failures = Vec::new();
        for (path, text) in crate::sources::SHIPPED.iter().filter(|(p, _)| p.starts_with("mappings/") && !p.contains("/builtin/")) {
            let stem = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            let lua = mlua::Lua::new();
            let m = match lua.load(*text).eval::<mlua::Table>().and_then(|t| Mapping::from_lua(path, false, &t)) {
                Ok(m) => m,
                Err(e) => {
                    failures.push(format!("{path}: {e}"));
                    continue;
                }
            };
            match read_dump(&dumps.join(format!("{stem}.json"))) {
                Ok(d) => failures.extend(problems(&m, &d).into_iter().map(|p| format!("{path}: {p}"))),
                Err(e) => failures.push(format!("{path}: ship its dump (plugin-probe dump) as tests/mappings/{stem}.json: {e}")),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// The mapping in `DECKARD_MAPPING` (against `DECKARD_DUMP`) rendered in each picture
    /// style, into `DECKARD_RENDER_DIR/<style>/`, to compare them (`--ignored`).
    #[test]
    #[ignore = "renders style variants to look at"]
    fn render_picture_styles() {
        let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let (Some(path), Some(dump_path)) = (var("DECKARD_MAPPING"), var("DECKARD_DUMP")) else { return };
        let out = var("DECKARD_RENDER_DIR").unwrap_or_else(|| std::env::temp_dir().join("deckard-styles"));
        let m = read_mapping(&path).unwrap();
        let dump = read_dump(&dump_path).unwrap();
        use super::super::PictureStyle as S;
        for (name, st) in [
            ("lines", S::LINES),
            ("soft", S::SOFT),
            ("panels", S::PANELS),
            ("glow", S::GLOW),
            ("scope", S::SCOPE),
            ("dots", S::DOTS),
            ("solid", S::SOLID),
            ("blueprint", S::BLUEPRINT),
        ] {
            super::super::set_style(st);
            let files = render_pages(&path, &m, &dump, &out.join(name)).unwrap();
            println!("{name}: {} files", files.len());
        }
        super::super::set_style(super::super::PictureStyle::LINES);
    }

    /// One page (`DECKARD_PAGE`) of the mapping in `DECKARD_MAPPING` in theme
    /// `DECKARD_THEME` and the `deep` picture style, to `DECKARD_RENDER_DIR/<page>.png`
    /// (`--ignored`).
    #[test]
    #[ignore = "renders one page to look at"]
    fn render_one_page() {
        let var = |k: &str| std::env::var(k).ok();
        let (Some(path), Some(dump_path), Some(out)) = (var("DECKARD_MAPPING"), var("DECKARD_DUMP"), var("DECKARD_RENDER_DIR")) else {
            return;
        };
        let page_name = var("DECKARD_PAGE").unwrap_or_else(|| "AMP".into());
        let path = PathBuf::from(path);
        let m = read_mapping(&path).unwrap();
        let dump = read_dump(Path::new(&dump_path)).unwrap();
        super::super::set_style(super::super::PictureStyle::DEEP);
        let dir = std::env::temp_dir().join(format!("deckard-one-page-{}", std::process::id()));
        let maps = dir.join("mappings").join("check");
        // (`DECKARD_CONTROLLER`: a controller folder whose scripts replace the built-ins.)
        if let Some(from) = var("DECKARD_CONTROLLER") {
            fn copy(from: &Path, to: &Path) {
                std::fs::create_dir_all(to).unwrap();
                for e in std::fs::read_dir(from).unwrap().flatten() {
                    let (f, t) = (e.path(), to.join(e.file_name()));
                    if f.is_dir() {
                        copy(&f, &t)
                    } else {
                        std::fs::copy(&f, &t).unwrap();
                    }
                }
            }
            copy(Path::new(&from), &dir);
        }
        std::fs::create_dir_all(&maps).unwrap();
        std::fs::copy(&path, maps.join("mapping.lua")).unwrap();
        let mut s = Surface::with_user_dir(Some(dir.clone()));
        s.set_theme(&var("DECKARD_THEME").unwrap_or_else(|| "deep".into()));
        let name = m.matches.name.clone().or_else(|| m.matches.builtin.clone()).unwrap_or_else(|| "Plug-in".into());
        let mut v = crate::tests_support::view();
        v.control.modules_visible = true;
        v.control.chain[0].name = name.clone();
        // (`DECKARD_BUILTIN`: as one of DECKARD's own devices rather than a hosted plug-in.)
        v.control.chain[0].plugin = var("DECKARD_BUILTIN").is_none();
        v.control.chain[0].instrument = m.kind == "synth";
        v.control.slot = Some(name.clone());
        v.control.pages = m.pages.iter().map(|p| p.name.clone()).collect();
        // (`DECKARD_LEVEL`: 0 master, 1 group, 2 sound; `DECKARD_GROUP_NAME` renames the group.)
        if let Some(l) = var("DECKARD_LEVEL").and_then(|l| l.parse().ok()) {
            v.control.level = l;
        }
        if let Some(n) = var("DECKARD_GROUP_NAME") {
            v.groups[v.group].name = n;
        }
        // The plug-in's own pictures, where the host has them (`DECKARD_PICTURES`: a
        // folder with `thumb.png` and `logo.png`).
        if let Some(dir) = var("DECKARD_PICTURES") {
            let key = name.to_lowercase();
            for (file, thumb) in [("thumb.png", true), ("logo.png", false)] {
                let Ok(f) = std::fs::File::open(Path::new(&dir).join(file)) else { continue };
                let mut dec = png::Decoder::new(std::io::BufReader::new(f));
                dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
                let mut r = dec.read_info().unwrap();
                let mut buf = vec![0; r.output_buffer_size().unwrap()];
                let info = r.next_frame(&mut buf).unwrap();
                let (w, h) = (info.width as usize, info.height as usize);
                let ch = info.color_type.samples();
                let px = (0..w * h)
                    .map(|i| {
                        let p = &buf[i * ch..i * ch + ch];
                        match ch {
                            4 => [p[0], p[1], p[2], p[3]],
                            3 => [p[0], p[1], p[2], 255],
                            2 => [p[0], p[0], p[0], p[1]],
                            _ => [p[0], p[0], p[0], 255],
                        }
                    })
                    .collect();
                s.add_picture(&key, thumb, w, h, px);
            }
            v.control.chain[0].picture = Some(key);
        }
        let i = m.pages.iter().position(|p| p.name.eq_ignore_ascii_case(&page_name)).expect("page");
        v.control.page = i;
        // (`DECKARD_PAGES=n:k`: as if the plug-in had n pages and this were page k.)
        if let Some((n, k)) = var("DECKARD_PAGES").and_then(|s| s.split_once(':').map(|(a, b)| (a.parse::<usize>(), b.parse::<usize>())))
            && let (Ok(n), Ok(k)) = (n, k)
        {
            while v.control.pages.len() < n {
                v.control.pages.push(format!("PAGE {}", v.control.pages.len() + 1));
            }
            v.control.pages.truncate(n);
            v.control.pages[k - 1] = page_name.clone();
            v.control.page = k - 1;
        }
        v.control.knobs = m.pages[i].knobs.iter().map(|k| k.as_deref().and_then(|k| knob(&m, k, &dump, None))).collect();
        // (`DECKARD_SECTIONS`: the knobs' section names, comma-separated, as a plug-in's
        // own pages give them.)
        if let Some(names) = var("DECKARD_SECTIONS") {
            for (k, name) in v.control.knobs.iter_mut().zip(names.split(',')) {
                if let Some(k) = k {
                    k.section = name.trim().to_string();
                }
            }
        }
        // (`DECKARD_LOADING`: with the loading overlay over it, naming that.)
        if let Some(name) = var("DECKARD_LOADING") {
            v.loading = Some(name);
        }
        let mut px = s.picture(&v);
        assert!(s.error().is_none(), "{:?}", s.error());
        // (`DECKARD_565`: as the displays show it, dithered to their 5-6-5 levels.)
        if var("DECKARD_565").is_some() {
            let w = crate::screen::WIDTH;
            for (i, p) in px.iter_mut().enumerate() {
                let (x, y) = (i % w, i / w);
                let q = if p[3] == crate::PICTURE_ALPHA {
                    deckard_mk3::display::rgb565_rounded(p[0], p[1], p[2])
                } else {
                    deckard_mk3::display::rgb565_dithered(x % (w / 2), y, p[0], p[1], p[2])
                };
                let up = |v: u16, max: u16| ((u32::from(v) * 255 + u32::from(max) / 2) / u32::from(max)) as u8;
                *p = [up(q >> 11, 31), up((q >> 5) & 63, 63), up(q & 31, 31), p[3]];
            }
        }
        std::fs::create_dir_all(&out).unwrap();
        let file = PathBuf::from(out).join(format!("{}.png", page_name.to_ascii_lowercase()));
        super::super::golden::write_png(&file, crate::screen::WIDTH as u32, crate::screen::HEIGHT as u32, &px);
        println!("wrote {}", file.display());
        super::super::set_style(super::super::PictureStyle::LINES);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `scripts/mapping-check.sh`: checks the mapping in `DECKARD_MAPPING` against the dump
    /// in `DECKARD_DUMP` and renders its pages into `DECKARD_RENDER_DIR`.
    #[test]
    #[ignore = "run by scripts/mapping-check.sh"]
    fn check_a_mapping() {
        let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let (Some(path), Some(dump_path)) = (var("DECKARD_MAPPING"), var("DECKARD_DUMP")) else {
            // (Run with every ignored test (`--ignored`): nothing to check.)
            eprintln!("check_a_mapping: set DECKARD_MAPPING (a mapping .lua) and DECKARD_DUMP (its plugin-probe dump); nothing checked");
            return;
        };
        let out = var("DECKARD_RENDER_DIR").unwrap_or_else(|| std::env::temp_dir().join("deckard-mapping-check"));
        let m = read_mapping(&path).unwrap_or_else(|e| panic!("{e}"));
        let dump = read_dump(&dump_path).unwrap_or_else(|e| panic!("{e}"));
        let p = problems(&m, &dump);
        let files = render_pages(&path, &m, &dump, &out).unwrap_or_else(|e| panic!("{e}"));
        println!("{} pages rendered: {} files in {}", m.pages.len(), files.len(), out.display());
        for p in &p {
            println!("problem: {p}");
        }
        assert!(p.is_empty(), "{} problems", p.len());
    }
}
