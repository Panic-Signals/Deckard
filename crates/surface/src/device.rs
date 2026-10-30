//! MIDI controllers as scripts (docs/controller-api.md "MIDI controllers"): a folder
//! `devices/<id>/` with a `device.lua` naming the device's ports and a `main.lua` of
//! handlers (`open`, `close`, `midi`, `tick`). Each device runs in its own Lua state,
//! apart from the MK3's, with the same `deckard` API plus `midi_send`, `print_notes` and
//! `message`.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::{Function, Lua, LuaOptions, StdLib, Table, Value};

use crate::sources::{self, Sources};
use crate::{Action, Shared, View, api, leds};

/// How often the user folder is checked for edited scripts.
const RELOAD_EVERY: Duration = Duration::from_millis(500);

/// A device's `device.lua`.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceManifest {
    /// Its folder's name.
    pub id: String,
    pub name: String,
    /// Each port role and the fragments a port's name may contain to be it.
    pub ports: Vec<(String, Vec<String>)>,
    /// The roles DECKARD must not also take notes from.
    pub exclusive: Vec<String>,
    /// The fragments of the name of the port its notes come on, for a drum channel
    /// (Preferences ▸ MIDI); none if it has no notes to give pads.
    pub drums: Vec<String>,
}

/// A note of a Print to Clip: its pitch, start and length (beats) and velocity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrintedNote {
    pub pitch: u8,
    pub start: f64,
    pub length: f64,
    pub velocity: u8,
}

/// A sequence a device prints into the focused sound's pattern (`length` beats).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Print {
    pub length: f64,
    pub notes: Vec<PrintedNote>,
    /// The device's track it came from (from 1), if it says.
    pub track: Option<u8>,
    /// The device that printed it (its id).
    pub device: String,
}

/// What a device's scripts ask for.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// What the MK3's scripts can ask of the app.
    App(Action),
    /// Bytes to send on the port of role `role`.
    Midi {
        role: String,
        bytes: Vec<u8>,
    },
    Print(Print),
    /// A line for the status bar.
    Message(String),
}

/// Every MIDI device there is, shipped and the user's (`devices/<id>/device.lua`; the
/// MK3's folder isn't one), sorted by id. A manifest that doesn't load is left out.
pub fn devices(user: Option<&Path>) -> Vec<DeviceManifest> {
    let mut ids: Vec<String> =
        sources::SHIPPED.iter().filter_map(|(p, _)| p.strip_prefix("devices/")?.strip_suffix("/device.lua").map(str::to_string)).collect();
    if let Some(dir) = user
        && let Ok(entries) = std::fs::read_dir(dir.join("devices"))
    {
        ids.extend(
            entries.flatten().filter(|e| e.path().join("device.lua").is_file()).filter_map(|e| e.file_name().to_str().map(str::to_string)),
        );
    }
    ids.sort();
    ids.dedup();
    ids.retain(|id| id != sources::DEVICE);
    ids.into_iter().filter_map(|id| manifest(user.map(Path::to_path_buf), &id).ok()).collect()
}

/// Device `id`'s manifest.
pub fn manifest(user: Option<PathBuf>, id: &str) -> mlua::Result<DeviceManifest> {
    let lua = Lua::new_with(StdLib::TABLE | StdLib::STRING | StdLib::MATH, LuaOptions::default())?;
    let t: Table = sources::run(&lua, &Sources::for_device(user, id), "device.lua")?;
    let mut ports = Vec::new();
    if let Some(p) = t.get::<Option<Table>>("ports")? {
        for pair in p.pairs::<String, Vec<String>>() {
            ports.push(pair?);
        }
    }
    ports.sort();
    Ok(DeviceManifest {
        id: id.to_string(),
        name: t.get::<Option<String>>("name")?.unwrap_or_else(|| id.to_string()),
        ports,
        exclusive: t.get::<Option<Vec<String>>>("exclusive")?.unwrap_or_default(),
        drums: t.get::<Option<Vec<String>>>("drums")?.unwrap_or_default(),
    })
}

struct Run {
    /// (Held for the state's life; the handlers are in it.)
    _lua: Lua,
    shared: Rc<RefCell<Shared>>,
    outs: Rc<RefCell<Vec<Out>>>,
    handlers: Table,
}

/// A MIDI device's loaded scripts.
pub struct Device {
    id: String,
    sources: Sources,
    run: Option<Run>,
    error: RefCell<Option<String>>,
    epoch: Instant,
    checked: Option<Instant>,
    /// `open` ran (and `close` hasn't): a reload opens the new scripts.
    open: bool,
    /// The last view, for `close` and a reload's `open`.
    view: View,
}

impl Device {
    /// Device `id`'s scripts, the files in `user` (if any) in place of DECKARD's.
    pub fn new(id: &str, user: Option<PathBuf>) -> Device {
        let mut d = Device {
            id: id.to_string(),
            sources: Sources::for_device(user, id),
            run: None,
            error: RefCell::new(None),
            epoch: Instant::now(),
            checked: None,
            open: false,
            view: View::default(),
        };
        d.load();
        d
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// The last script error, if there is one.
    pub fn error(&self) -> Option<String> {
        self.error.borrow().clone()
    }

    fn set_error(&self, e: impl std::fmt::Display) {
        *self.error.borrow_mut() = Some(format!("{}: {}", self.id, e.to_string().lines().next().unwrap_or_default()));
    }

    /// Load the scripts; on an error the last working ones stay.
    fn load(&mut self) -> bool {
        match start(&self.sources, &self.id) {
            Ok(run) => {
                self.run = Some(run);
                *self.error.borrow_mut() = None;
                true
            }
            Err(e) => {
                self.set_error(e);
                false
            }
        }
    }

    /// The device was found: the script's `open`.
    pub fn open(&mut self, v: &View, now: Instant) -> Vec<Out> {
        self.open = true;
        self.call(v, now, "open", ())
    }

    /// The device went, or the app quits: the script's `close`.
    pub fn close(&mut self) -> Vec<Out> {
        self.open = false;
        let v = std::mem::take(&mut self.view);
        self.call(&v, Instant::now(), "close", ())
    }

    /// Bytes from the port of role `role` (a whole message; SysEx with `F0`…`F7`).
    pub fn midi(&mut self, role: &str, bytes: &[u8], v: &View, now: Instant) -> Vec<Out> {
        let bytes: Vec<i64> = bytes.iter().map(|&b| b as i64).collect();
        self.call(v, now, "midi", (role.to_string(), bytes))
    }

    /// As often as the app updates.
    pub fn tick(&mut self, v: &View, now: Instant) -> Vec<Out> {
        self.call(v, now, "tick", ())
    }

    /// Load the scripts again if the user's files changed (checked at most twice a
    /// second): what the new scripts' `open` asks, if it was open.
    pub fn poll(&mut self, now: Instant) -> Vec<Out> {
        if self.checked.is_some_and(|t| now.duration_since(t) < RELOAD_EVERY) {
            return Vec::new();
        }
        self.checked = Some(now);
        if self.reload() && self.open {
            let v = self.view.clone();
            return self.open(&v, now);
        }
        Vec::new()
    }

    /// Load the scripts again if the user's files changed. Whether they were reloaded.
    pub fn reload(&mut self) -> bool {
        self.sources.changed() && self.load()
    }

    /// Run handler `name` (if the script has it) with what the scripts see set; take
    /// what they asked for.
    fn call(&mut self, v: &View, now: Instant, name: &str, args: impl mlua::IntoLuaMulti) -> Vec<Out> {
        if self.view != *v {
            self.view = v.clone();
        }
        let Some(run) = &self.run else { return Vec::new() };
        {
            let mut sh = run.shared.borrow_mut();
            sh.view = v.clone();
            sh.now = now.saturating_duration_since(self.epoch).as_secs_f64();
        }
        if let Ok(Some(f)) = run.handlers.get::<Option<Function>>(name)
            && let Err(e) = f.call::<Value>(args)
        {
            self.set_error(e);
        }
        let mut out = std::mem::take(&mut *run.outs.borrow_mut());
        out.extend(std::mem::take(&mut run.shared.borrow_mut().actions).into_iter().map(Out::App));
        out
    }
}

fn start(sources: &Sources, id: &str) -> mlua::Result<Run> {
    // No file, process or OS access: the scripts only get the `deckard` API.
    let lua = Lua::new_with(StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH, LuaOptions::default())?;
    let shared = Rc::new(RefCell::new(Shared {
        view: View::default(),
        now: 0.0,
        held: HashSet::new(),
        pads: [false; 16],
        actions: Vec::new(),
        page: String::new(),
        leds: leds::blank(),
    }));
    api::install(&lua, &shared)?;
    let outs = Rc::new(RefCell::new(Vec::new()));
    install(&lua, id, &outs)?;
    sources::install_require(&lua, sources.clone())?;
    let handlers: Table = sources::run(&lua, sources, "main.lua")?;
    Ok(Run { _lua: lua, shared, outs, handlers })
}

/// The device's own part of the `deckard` API.
fn install(lua: &Lua, id: &str, outs: &Rc<RefCell<Vec<Out>>>) -> mlua::Result<()> {
    let d: Table = lua.globals().get("deckard")?;
    let id = id.to_string();
    let name = id.clone();
    d.set("device", lua.create_function(move |_, ()| Ok(name.clone()))?)?;
    let o = outs.clone();
    d.set(
        "midi_send",
        lua.create_function(move |_, (role, bytes): (String, Vec<i64>)| {
            let bytes = bytes
                .into_iter()
                .map(|b| u8::try_from(b).map_err(|_| mlua::Error::runtime(format!("midi_send: {b} isn't a byte (0–255)"))))
                .collect::<mlua::Result<Vec<u8>>>()?;
            o.borrow_mut().push(Out::Midi { role, bytes });
            Ok(())
        })?,
    )?;
    let o = outs.clone();
    let device = id.clone();
    d.set(
        "print_notes",
        lua.create_function(move |_, t: Table| {
            let length: f64 = t.get("length")?;
            let mut notes = Vec::new();
            for n in t.get::<Table>("notes")?.sequence_values::<Table>() {
                let n = n?;
                let byte = |v: i64| v.clamp(0, 127) as u8;
                notes.push(PrintedNote {
                    pitch: byte(n.get(1)?),
                    start: n.get::<f64>(2)?.max(0.0),
                    length: n.get::<f64>(3)?.max(0.0),
                    velocity: byte(n.get(4)?).max(1),
                });
            }
            let track = t.get::<Option<i64>>("track")?.map(|n| n.clamp(1, 255) as u8);
            o.borrow_mut().push(Out::Print(Print { length: length.max(0.0), notes, track, device: device.clone() }));
            Ok(())
        })?,
    )?;
    let o = outs.clone();
    d.set(
        "message",
        lua.create_function(move |_, text: String| {
            o.borrow_mut().push(Out::Message(text));
            Ok(())
        })?,
    )?;
    Ok(())
}
