//! Where the scripts come from: DECKARD's shipped files (in the binary), each replaced
//! by a file of the same path in the user's folder. A controller model's own version of
//! a file (`devices/<model>/pages/control.lua`) comes before the shared one, so other
//! controllers share the pages and change only what differs (their screens, buttons).

use std::path::{Path, PathBuf};

use mlua::{Lua, Table};

/// The controller model the scripts run (the only one DECKARD drives yet).
pub const DEVICE: &str = "mk3";

/// DECKARD's own scripts, by path under `lua/`.
pub const SHIPPED: &[(&str, &str)] = &[
    ("global.lua", include_str!("../lua/global.lua")),
    ("pages/control.lua", include_str!("../lua/pages/control.lua")),
    ("pages/pad_mode.lua", include_str!("../lua/pages/pad_mode.lua")),
    ("pages/keyboard.lua", include_str!("../lua/pages/keyboard.lua")),
    ("pages/step.lua", include_str!("../lua/pages/step.lua")),
    ("pages/step_mod.lua", include_str!("../lua/pages/step_mod.lua")),
    ("pages/hold.lua", include_str!("../lua/pages/hold.lua")),
    ("pages/mixer.lua", include_str!("../lua/pages/mixer.lua")),
    ("pages/arrange.lua", include_str!("../lua/pages/arrange.lua")),
    ("pages/pattern.lua", include_str!("../lua/pages/pattern.lua")),
    ("pages/scene.lua", include_str!("../lua/pages/scene.lua")),
    ("pages/grid.lua", include_str!("../lua/pages/grid.lua")),
    ("pages/file.lua", include_str!("../lua/pages/file.lua")),
    ("pages/save.lua", include_str!("../lua/pages/save.lua")),
    ("pages/sampling.lua", include_str!("../lua/pages/sampling.lua")),
    ("pages/settings.lua", include_str!("../lua/pages/settings.lua")),
    ("pages/macro_set.lua", include_str!("../lua/pages/macro_set.lua")),
    ("pages/lock.lua", include_str!("../lua/pages/lock.lua")),
    ("pages/lfo.lua", include_str!("../lua/pages/lfo.lua")),
    ("pages/mod_matrix.lua", include_str!("../lua/pages/mod_matrix.lua")),
    ("pages/events.lua", include_str!("../lua/pages/events.lua")),
    ("pages/note_repeat.lua", include_str!("../lua/pages/note_repeat.lua")),
    ("pages/song.lua", include_str!("../lua/pages/song.lua")),
    ("pages/browse.lua", include_str!("../lua/pages/browse.lua")),
    ("pages/module.lua", include_str!("../lua/pages/module.lua")),
    ("lib/pattern_editor.lua", include_str!("../lua/lib/pattern_editor.lua")),
    ("lib/util.lua", include_str!("../lua/lib/util.lua")),
    ("lib/chain.lua", include_str!("../lua/lib/chain.lua")),
    ("lib/quick_edit.lua", include_str!("../lua/lib/quick_edit.lua")),
    ("lib/pin.lua", include_str!("../lua/lib/pin.lua")),
    ("lib/paged.lua", include_str!("../lua/lib/paged.lua")),
    ("lib/back.lua", include_str!("../lua/lib/back.lua")),
    ("lib/pads.lua", include_str!("../lua/lib/pads.lua")),
    ("lib/knobs.lua", include_str!("../lua/lib/knobs.lua")),
    ("lib/notes.lua", include_str!("../lua/lib/notes.lua")),
    ("lib/strip.lua", include_str!("../lua/lib/strip.lua")),
    ("lib/wheel.lua", include_str!("../lua/lib/wheel.lua")),
    ("lib/widgets.lua", include_str!("../lua/lib/widgets.lua")),
    ("devices/launchpad_pro_mk3/device.lua", include_str!("../lua/devices/launchpad_pro_mk3/device.lua")),
    ("devices/launchpad_pro_mk3/main.lua", include_str!("../lua/devices/launchpad_pro_mk3/main.lua")),
    ("devices/launchpad_pro_mk3/lib/print.lua", include_str!("../lua/devices/launchpad_pro_mk3/lib/print.lua")),
    ("themes/maschine.lua", include_str!("../lua/themes/maschine.lua")),
    ("themes/palenight.lua", include_str!("../lua/themes/palenight.lua")),
    ("themes/solarized_dark.lua", include_str!("../lua/themes/solarized_dark.lua")),
];

#[derive(Clone, Default)]
pub struct Sources {
    user: Option<PathBuf>,
    /// The controller model (`mk3`), for its own files under `devices/`.
    device: String,
    /// A MIDI device's scripts (`devices/<id>/`): they see their own folder and `lib/`,
    /// never the MK3's pages.
    midi: bool,
    /// The user's files as last loaded (path, text), to see edits.
    seen: Vec<(PathBuf, String)>,
}

impl Sources {
    pub fn new(user: Option<PathBuf>) -> Sources {
        let mut s = Sources { user, device: DEVICE.to_string(), midi: false, seen: Vec::new() };
        s.seen = s.user_files();
        s
    }

    /// MIDI device `id`'s scripts: `devices/<id>/<path>`, then `lib/<path>`.
    pub fn for_device(user: Option<PathBuf>, id: &str) -> Sources {
        let mut s = Sources { user, device: id.to_string(), midi: true, seen: Vec::new() };
        s.seen = s.user_files();
        s
    }

    /// The script at `path` (`pages/control.lua`): its chunk name and text, the user's
    /// file if there is one.
    pub fn get(&self, path: &str) -> Option<(String, String)> {
        let shared = !self.midi || path.starts_with("lib/");
        let paths = [format!("devices/{}/{path}", self.device), path.to_string()];
        let paths = &paths[..if shared { 2 } else { 1 }];
        if let Some(dir) = &self.user {
            for p in paths {
                if let Ok(text) = std::fs::read_to_string(dir.join(p)) {
                    return Some((format!("@{}", dir.join(p).display()), text));
                }
            }
        }
        paths.iter().find_map(|p| SHIPPED.iter().find(|(s, _)| s == p)).map(|(p, text)| (format!("@{p}"), text.to_string()))
    }

    /// The names of the scripts in folder `dir` (`themes`): DECKARD's and the user's,
    /// sorted, without `.lua`.
    pub fn list(&self, dir: &str) -> Vec<String> {
        let prefix = format!("{dir}/");
        let mut out: Vec<String> =
            SHIPPED.iter().filter_map(|(p, _)| p.strip_prefix(&prefix)?.strip_suffix(".lua").map(str::to_string)).collect();
        if let Some(user) = &self.user
            && let Ok(entries) = std::fs::read_dir(user.join(dir))
        {
            out.extend(entries.flatten().filter_map(|e| e.file_name().to_str()?.strip_suffix(".lua").map(str::to_string)));
        }
        out.sort();
        out.dedup();
        out
    }

    /// Whether the user's files changed since the last call (or since they were found).
    pub fn changed(&mut self) -> bool {
        let now = self.user_files();
        let changed = now != self.seen;
        self.seen = now;
        changed
    }

    /// Every `.lua` file in the user's folder (four folders deep), sorted.
    fn user_files(&self) -> Vec<(PathBuf, String)> {
        fn walk(dir: &Path, depth: usize, out: &mut Vec<(PathBuf, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() && depth > 0 {
                    walk(&p, depth - 1, out);
                } else if p.extension().is_some_and(|x| x == "lua")
                    && let Ok(text) = std::fs::read_to_string(&p)
                {
                    out.push((p, text));
                }
            }
        }
        let mut out = Vec::new();
        if let Some(dir) = &self.user {
            walk(dir, 4, &mut out);
            // (A MIDI device reloads for its own files and the shared ones only.)
            if self.midi {
                let (own, lib) = (dir.join("devices").join(&self.device), dir.join("lib"));
                out.retain(|(p, _)| p.starts_with(&own) || p.starts_with(&lib));
            }
        }
        out.sort();
        out
    }
}

/// Run the script at `path`, returning what it returns.
pub fn run<T: mlua::FromLuaMulti>(lua: &Lua, sources: &Sources, path: &str) -> mlua::Result<T> {
    let (name, text) = sources.get(path).ok_or_else(|| mlua::Error::runtime(format!("no script {path}")))?;
    lua.load(text).set_name(name).call(())
}

/// `require("lib.util")` loads `lib/util.lua` once, from the user's folder or DECKARD's.
pub fn install_require(lua: &Lua, sources: Sources) -> mlua::Result<()> {
    lua.set_app_data(sources);
    lua.set_named_registry_value("deckard.loaded", lua.create_table()?)?;
    let require = lua.create_function(|lua, name: String| {
        let loaded: Table = lua.named_registry_value("deckard.loaded")?;
        if let Some(v) = loaded.get::<Option<mlua::Value>>(name.as_str())? {
            return Ok(v);
        }
        let sources = lua.app_data_ref::<Sources>().map(|s| s.clone()).unwrap_or_default();
        let v: mlua::Value = run(lua, &sources, &format!("{}.lua", name.replace('.', "/")))?;
        let v = if v.is_nil() { mlua::Value::Boolean(true) } else { v };
        loaded.set(name.as_str(), v.clone())?;
        Ok(v)
    })?;
    lua.globals().set("require", require)
}
