//! Where scripts come from: the user's extracted Maschine resources, changed by mod
//! folders. A mod folder either replaces scripts file by file (laid out as
//! `Scripts/Maschine/...`), or carries a manifest of extensions to splice into NI's
//! scripts ([`Scripts::add_extensions`]; the format of vubeatz's "Maschine Update").

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct Scripts {
    /// Module name (`Scripts/Maschine/PageManager`) -> file.
    modules: HashMap<String, PathBuf>,
    /// Modules whose source an extension mod changed (the whole new source).
    patched: HashMap<String, Vec<u8>>,
    /// Constant values the app registers for the scripts, from `constants.tsv`
    /// (`enum-values` output): (registering function, name, value).
    pub constants: Vec<(String, String, i64)>,
}

impl Scripts {
    /// Index every `Scripts/**/*.lua` under `root` (e.g. the `ni-res-extract` output,
    /// whose registrar folders each contain a `Scripts` directory).
    pub fn from_dir(root: &Path) -> std::io::Result<Scripts> {
        let mut s = Scripts::default();
        s.add_dir(root)?;
        Ok(s)
    }

    /// Add (and override with) scripts from another tree, such as a mod folder laid out
    /// as `Scripts/Maschine/...`.
    pub fn add_dir(&mut self, root: &Path) -> std::io::Result<()> {
        if let Ok(tsv) = std::fs::read_to_string(root.join("constants.tsv")) {
            self.constants = tsv
                .lines()
                .filter_map(|l| {
                    let mut f = l.split('\t');
                    let func = f.next()?;
                    let name = f.next()?;
                    let value = f.next()?.parse().ok()?;
                    // `NI::MASCOMMON::scripting::registerJamHW(...)` -> `registerJamHW`
                    let func = func.split('(').next()?.rsplit("::").next()?;
                    Some((func.to_string(), name.to_string(), value))
                })
                .collect();
        }
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir)? {
                let p = e?.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "lua")
                    && let Some(name) = module_name(root, &p)
                {
                    self.modules.insert(name, p);
                }
            }
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    pub fn path(&self, module: &str) -> Option<&Path> {
        self.modules.get(module.trim_end_matches(".lua")).map(PathBuf::as_path)
    }

    pub fn source(&self, module: &str) -> Option<(PathBuf, Vec<u8>)> {
        let p = self.path(module)?;
        if let Some(src) = self.patched.get(module.trim_end_matches(".lua")) {
            return Some((p.to_path_buf(), src.clone()));
        }
        Some((p.to_path_buf(), std::fs::read(p).ok()?))
    }

    /// Add a mod folder: its manifest of extensions if it has one (any `*.json` with a
    /// `resources` list), else its scripts as file-by-file replacements.
    pub fn add_mod(&mut self, root: &Path) -> std::io::Result<ModReport> {
        for e in std::fs::read_dir(root)? {
            let p = e?.path();
            if p.extension().is_some_and(|x| x == "json")
                && let Ok(text) = std::fs::read_to_string(&p)
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
                && json.get("resources").is_some_and(serde_json::Value::is_array)
            {
                return Ok(self.add_extensions(root, &json));
            }
        }
        let before = self.modules.len();
        self.add_dir(root)?;
        Ok(ModReport { replaced: self.modules.len().saturating_sub(before), ..ModReport::default() })
    }

    /// Splice a manifest's extensions into NI's scripts. Each resource names a script by
    /// file name (`MaschineHelper`), the file holding the extension, and where it goes:
    /// `append`, `beforeLine` (before the line equal to `anchor`, the last one) or
    /// `beforeText` (before the first `anchor`). Native patches and libraries (changes
    /// to Maschine's own program) can't apply to DECKARD and are only reported.
    pub fn add_extensions(&mut self, root: &Path, json: &serde_json::Value) -> ModReport {
        self.splice_extensions(json, |file| std::fs::read_to_string(root.join(file)).ok())
    }

    /// DECKARD's own mods, built in and always applied: Pad Touch on the MK3's Keyboard
    /// page (`mods/pad-touch`). Our code, not NI's.
    pub fn add_builtin(&mut self) -> ModReport {
        const MANIFEST: &str = include_str!("../mods/pad-touch/pad-touch.json");
        const KEYBOARD: &str = include_str!("../mods/pad-touch/KeyboardModePageMK3.lua");
        let Ok(json) = serde_json::from_str::<serde_json::Value>(MANIFEST) else { return ModReport::default() };
        self.splice_extensions(&json, |file| (file == "KeyboardModePageMK3.lua").then(|| KEYBOARD.to_string()))
    }

    /// Splice a manifest's extensions in, `read` giving each extension file's text.
    fn splice_extensions(&mut self, json: &serde_json::Value, read: impl Fn(&str) -> Option<String>) -> ModReport {
        let mut report = ModReport::default();
        let str_of = |v: &serde_json::Value, k: &str| v.get(k).and_then(serde_json::Value::as_str).map(str::to_string);
        for r in json["resources"].as_array().into_iter().flatten() {
            let (Some(name), Some(file)) = (str_of(r, "name"), str_of(r, "extensionFile")) else { continue };
            if r.get("css").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                report.skipped.push(format!("{name}: a style sheet (DECKARD draws its own screens)"));
                continue;
            }
            let targets: Vec<String> = self.modules.keys().filter(|m| m.rsplit('/').next() == Some(name.as_str())).cloned().collect();
            let [module] = targets.as_slice() else {
                report.skipped.push(format!("{name}: {} scripts of that name", targets.len()));
                continue;
            };
            let Some(ext) = read(&file) else {
                report.skipped.push(format!("{name}: {file} unreadable"));
                continue;
            };
            let base = match self.source(module) {
                Some((_, b)) => String::from_utf8_lossy(&b).into_owned(),
                None => continue,
            };
            let anchor = str_of(r, "anchor");
            let placed = match (str_of(r, "placement").as_deref(), anchor.as_deref()) {
                (Some("beforeLine"), Some(a)) => base.rfind(&format!("\n{a}")).map(|at| splice(&base, at + 1, &ext)),
                (Some("beforeText"), Some(a)) => base.find(a).map(|at| splice(&base, at, &ext)),
                _ => Some(format!("{base}\n{ext}\n")),
            };
            match placed {
                Some(src) => {
                    self.patched.insert(module.clone(), src.into_bytes());
                    report.applied.push(name);
                }
                None => report.skipped.push(format!("{name}: anchor {:?} not found", anchor.unwrap_or_default())),
            }
        }
        for (key, what) in [("nativePatches", "patches to Maschine's program"), ("library", "a native library")] {
            if json.get(key).is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty())) {
                report.native.push(what.to_string());
            }
        }
        report
    }

    /// Every `NI.…` path that some script looks a key up under (`NI.HW`,
    /// `NI.DATA.StateHelper`, ...), as opposed to leaf constants and functions.
    pub fn namespaces(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        for name in self.modules.keys() {
            let Some((_, src)) = self.source(name) else { continue };
            for path in ni_paths(&src) {
                let parts: Vec<&str> = path.split('.').collect();
                for n in 1..parts.len() {
                    out.insert(parts[..n].join("."));
                }
            }
        }
        out
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.modules.keys().map(String::as_str)
    }
}

/// What adding a mod did.
#[derive(Debug, Default)]
pub struct ModReport {
    /// Scripts it replaced (a file-by-file mod).
    pub replaced: usize,
    /// Scripts an extension was spliced into, and those it couldn't be.
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
    /// Parts that change Maschine's own program, which DECKARD can't apply.
    pub native: Vec<String>,
}

/// `text` with `insert` (on lines of its own) placed at byte `at`.
fn splice(text: &str, at: usize, insert: &str) -> String {
    format!("{}{}\n{}", &text[..at], insert.trim_end_matches('\n'), &text[at..])
}

/// `<root>/<registrar>/Scripts/A/B.lua` -> `Scripts/A/B`.
fn module_name(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let parts: Vec<&str> = rel.iter().filter_map(|c| c.to_str()).collect();
    let at = parts.iter().position(|c| *c == "Scripts")?;
    let name = parts[at..].join("/");
    Some(name.trim_end_matches(".lua").to_string())
}

/// Dotted identifier chains starting with `NI.` in Lua source.
fn ni_paths(src: &[u8]) -> Vec<String> {
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 <= src.len() {
        let boundary = i == 0 || !(ident(src[i - 1]) || src[i - 1] == b'.' || src[i - 1] == b':');
        if boundary && src[i..].starts_with(b"NI.") {
            let mut j = i;
            while j < src.len() && (ident(src[j]) || (src[j] == b'.' && j + 1 < src.len() && ident(src[j + 1]))) {
                j += 1;
            }
            out.push(String::from_utf8_lossy(&src[i..j]).into_owned());
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod mod_tests {
    use super::*;

    /// An extension mod splices into NI's scripts by name: appended, before a line, or
    /// before a text; a missing anchor and native parts are reported, not applied.
    #[test]
    fn extension_mods_splice_into_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let ni = dir.join("ni/Scripts/Maschine");
        let m = dir.join("mod");
        std::fs::create_dir_all(&ni).unwrap();
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(ni.join("A.lua"), "local A = {}\nreturn A\n").unwrap();
        std::fs::write(ni.join("B.lua"), "x = 1\nControllerScriptInterface = B()\n").unwrap();
        std::fs::write(ni.join("C.lua"), "c = 1\n").unwrap();
        for (f, t) in [("A.lua", "A.added = true"), ("B.lua", "b = 2"), ("C.lua", "c = 2"), ("D.lua", "d = 1")] {
            std::fs::write(m.join(f), t).unwrap();
        }
        let manifest = serde_json::json!({
            "resources": [
                {"name": "A", "extensionFile": "A.lua", "placement": "beforeLine", "anchor": "return A"},
                {"name": "B", "extensionFile": "B.lua", "placement": "beforeText", "anchor": "ControllerScriptInterface = B()"},
                {"name": "C", "extensionFile": "C.lua", "placement": "append", "anchor": null},
                {"name": "A", "extensionFile": "D.lua", "placement": "beforeLine", "anchor": "return Nothing"},
            ],
            "nativePatches": [{"name": "x"}],
            "library": {"file": "x.dylib"},
        });
        std::fs::write(m.join("manifest.json"), manifest.to_string()).unwrap();
        let mut s = Scripts::from_dir(&dir.join("ni")).unwrap();
        let r = s.add_mod(&m).unwrap();
        let src = |name: &str| String::from_utf8(s.source(name).unwrap().1).unwrap();
        assert_eq!(src("Scripts/Maschine/A"), "local A = {}\nA.added = true\nreturn A\n");
        assert_eq!(src("Scripts/Maschine/B"), "x = 1\nb = 2\nControllerScriptInterface = B()\n");
        assert_eq!(src("Scripts/Maschine/C"), "c = 1\n\nc = 2\n");
        assert_eq!(r.applied, ["A", "B", "C"]);
        assert_eq!(r.skipped.len(), 1, "{:?}", r.skipped);
        assert_eq!(r.native.len(), 2);
    }

    /// DECKARD's built-in mods (Pad Touch on the MK3's Keyboard page) splice into the
    /// scripts they name, and say so when the script isn't there.
    #[test]
    fn builtin_mods_apply() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let pages = dir.join("Scripts/Maschine/MaschineMK3/Pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("KeyboardModePageMK3.lua"), "KeyboardModePageMK3 = {}\n").unwrap();
        let mut s = Scripts::from_dir(&dir).unwrap();
        let r = s.add_builtin();
        assert_eq!(r.applied, ["KeyboardModePageMK3"], "{:?}", r.skipped);
        let src = String::from_utf8(s.source("Scripts/Maschine/MaschineMK3/Pages/KeyboardModePageMK3").unwrap().1).unwrap();
        assert!(src.starts_with("KeyboardModePageMK3 = {}\n"));
        assert!(src.contains("Pad Touch CC"), "the extension follows");
        drop(tmp);
        let r = Scripts::default().add_builtin();
        assert!(r.applied.is_empty() && r.skipped.len() == 1, "{r:?}");
    }
}
