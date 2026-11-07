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
    /// A device spoken to over USB HID rather than MIDI: its vendor and product ids.
    pub hid: Option<(u16, u16)>,
}

/// What a pad of a device with its own note path plays: (group, sound, pitch) each, all
/// from 0 but the pitch.
pub type PadNotes = Vec<(usize, usize, i64)>;

/// Where a HID device's pads play (the script's `pads()`), read by its thread so that a note
/// never waits for a frame. By pad, from `column + row × 8`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PadMap {
    pub pads: Vec<PadNotes>,
    /// What every hit plays at: click-pads have no pressure.
    pub velocity: i64,
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
        hid: hid_of(&t)?,
    })
}

/// A `device.lua`'s `hid = { vendor = …, product = … }`.
fn hid_of(t: &Table) -> mlua::Result<Option<(u16, u16)>> {
    let Some(h) = t.get::<Option<Table>>("hid")? else { return Ok(None) };
    Ok(Some((h.get("vendor")?, h.get("product")?)))
}

struct Run {
    /// (Held for the state's life; the handlers are in it.)
    _lua: Lua,
    shared: Rc<RefCell<Shared>>,
    outs: Rc<RefCell<Vec<Out>>>,
    handlers: Table,
    /// What a HID device's `leds()` set.
    lights: Rc<RefCell<deckard_jam::Leds>>,
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

    /// A HID device's button, by its name: `deckard.held` follows it.
    pub fn button(&mut self, name: &str, down: bool, v: &View, now: Instant) -> Vec<Out> {
        if let Some(run) = &self.run {
            let mut sh = run.shared.borrow_mut();
            if down {
                sh.held.insert(name.to_string());
            } else {
                sh.held.remove(name);
            }
        }
        self.call(v, now, "button", (name.to_string(), down))
    }

    /// A HID device's pad, from 1: `column` left to right, `row` from the top.
    pub fn pad(&mut self, column: u8, row: u8, down: bool, v: &View, now: Instant) -> Vec<Out> {
        self.call(v, now, "pad", (i64::from(column) + 1, i64::from(row) + 1, down))
    }

    /// A HID device's strip `strip` (from 1): finger `finger` (from 1) went "down", "move"d
    /// or came "up" at `pos` (0..1).
    pub fn strip(&mut self, strip: u8, finger: u8, kind: &str, pos: f64, v: &View, now: Instant) -> Vec<Out> {
        self.call(v, now, "strip", (i64::from(strip) + 1, i64::from(finger) + 1, kind.to_string(), pos))
    }

    /// A HID device's encoder turned `steps` clicks.
    pub fn encoder(&mut self, steps: i32, v: &View, now: Instant) -> Vec<Out> {
        self.call(v, now, "encoder", steps)
    }

    /// What a HID device's lights should show: its `leds()`, on every button at its
    /// backlight and the rest dark. What the handler asked of the app is dropped: lights
    /// only read.
    pub fn leds(&mut self, v: &View, now: Instant) -> deckard_jam::Leds {
        if let Some(run) = &self.run {
            *run.lights.borrow_mut() = deckard_jam::Leds::blank();
        }
        let _ = self.call(v, now, "leds", ());
        self.run.as_ref().map_or_else(deckard_jam::Leds::default, |run| run.lights.borrow().clone())
    }

    /// Where a HID device's pads play now: its `pads()`, which returns
    /// `{ velocity = n, [column + (row − 1) × 8] = { group, sound, pitch, pitch … } }`, or
    /// `nil` for nothing played. Groups and sounds count from 1 in the script.
    pub fn pad_map(&mut self, v: &View, now: Instant) -> PadMap {
        let empty = PadMap { pads: vec![Vec::new(); 64], velocity: 100 };
        if self.run.is_none() {
            return empty;
        }
        {
            let run = self.run.as_ref().expect("checked");
            let mut sh = run.shared.borrow_mut();
            sh.view = v.clone();
            sh.now = now.saturating_duration_since(self.epoch).as_secs_f64();
        }
        let run = self.run.as_ref().expect("checked");
        let Ok(Some(f)) = run.handlers.get::<Option<Function>>("pads") else { return empty };
        match f.call::<Option<Table>>(()) {
            Ok(Some(t)) => match pad_map_of(&t) {
                Ok(m) => m,
                Err(e) => {
                    self.set_error(e);
                    empty
                }
            },
            Ok(None) => empty,
            Err(e) => {
                self.set_error(e);
                empty
            }
        }
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
    let lights = Rc::new(RefCell::new(deckard_jam::Leds::blank()));
    // (Only a HID device's: a MIDI device's lights are its own bytes.)
    let manifest: Option<Table> = sources::run(&lua, sources, "device.lua").ok();
    if manifest.as_ref().is_some_and(|t| hid_of(t).ok().flatten().is_some()) {
        install_lights(&lua, &lights)?;
    }
    sources::install_require(&lua, sources.clone())?;
    let handlers: Table = sources::run(&lua, sources, "main.lua")?;
    Ok(Run { _lua: lua, shared, outs, handlers, lights })
}

/// `{ velocity = n, [pad] = { group, sound, pitch … } }` as a [`PadMap`].
fn pad_map_of(t: &Table) -> mlua::Result<PadMap> {
    let mut pads = vec![Vec::new(); 64];
    for (i, notes) in pads.iter_mut().enumerate() {
        let Some(p) = t.get::<Option<Table>>(i + 1)? else { continue };
        let group = p.get::<usize>(1)?.max(1) - 1;
        let sound = p.get::<usize>(2)?.max(1) - 1;
        for pitch in p.sequence_values::<i64>().skip(2) {
            let pitch = pitch?;
            if (0..=127).contains(&pitch) {
                notes.push((group, sound, pitch));
            }
        }
    }
    Ok(PadMap { pads, velocity: t.get::<Option<i64>>("velocity")?.unwrap_or(100).clamp(1, 127) })
}

/// What a colour light's arguments make: `"off"`, `"dim"` or `"on"` (white), or a palette
/// colour and whether it's bright. `pale` (a third argument of `"pale"`) is the palette's
/// brightest shade, for what must stand out from what's merely on.
fn colour_of(name: &str, how: &Value, bright: &Value) -> mlua::Result<u8> {
    use deckard_jam::leds::{SHADE_BRIGHT, SHADE_DIM, SHADE_FAINT, SHADE_PALE, WHITE, colour};
    let shade = match bright {
        Value::Boolean(true) | Value::Nil => SHADE_BRIGHT,
        Value::Boolean(false) => SHADE_DIM,
        Value::String(s) => match s.to_str()?.as_ref() {
            "pale" => SHADE_PALE,
            "faint" => SHADE_FAINT,
            "dim" => SHADE_DIM,
            "bright" => SHADE_BRIGHT,
            other => return Err(mlua::Error::runtime(format!("light {name}: shade \"{other}\"? (faint, dim, bright, pale)"))),
        },
        _ => return Err(mlua::Error::runtime(format!("light {name}: bright is true, false or a shade's name"))),
    };
    match how {
        Value::Nil => Ok(0),
        Value::Integer(i) => Ok(colour(*i, shade)),
        Value::Number(n) => Ok(colour(*n as i64, shade)),
        Value::String(s) => match s.to_str()?.as_ref() {
            "off" => Ok(0),
            "dim" => Ok(colour(WHITE, SHADE_DIM)),
            "on" => Ok(colour(WHITE, SHADE_BRIGHT)),
            other => Err(mlua::Error::runtime(format!("light {name}: \"{other}\"? (off, dim, on, or a palette colour)"))),
        },
        _ => Err(mlua::Error::runtime(format!("light {name}: a level or a palette colour"))),
    }
}

/// A HID device's lights, in place of the MK3's `deckard.led`.
fn install_lights(lua: &Lua, lights: &Rc<RefCell<deckard_jam::Leds>>) -> mlua::Result<()> {
    use deckard_jam::leds::{BACKLIGHT, DIM, ON};
    let d: Table = lua.globals().get("deckard")?;
    let l = lights.clone();
    d.set(
        "led",
        lua.create_function(move |_, (name, how, bright): (String, Value, Value)| {
            let mut leds = l.borrow_mut();
            // (Scene and group buttons have colours; the rest a brightness.)
            if let Some(n) = name.strip_prefix("SCENE_").and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=8).contains(n)) {
                leds.scene(n - 1, colour_of(&name, &how, &bright)?);
                return Ok(());
            }
            if let Some(g) =
                name.strip_prefix("GROUP_").filter(|g| g.len() == 1).and_then(|g| g.bytes().next()).filter(|g| (b'A'..=b'H').contains(g))
            {
                leds.group(usize::from(g - b'A'), colour_of(&name, &how, &bright)?);
                return Ok(());
            }
            let level = match &how {
                Value::String(s) => match s.to_str()?.as_ref() {
                    "off" => BACKLIGHT,
                    "dark" => 0,
                    "dim" => DIM,
                    "on" => ON,
                    other => return Err(mlua::Error::runtime(format!("light {name}: \"{other}\"? (off, dim, on)"))),
                },
                _ => return Err(mlua::Error::runtime(format!("light {name}: off, dim or on"))),
            };
            if !leds.button(&name, level) {
                return Err(mlua::Error::runtime(format!("no light {name}")));
            }
            Ok(())
        })?,
    )?;
    let l = lights.clone();
    d.set(
        "pad_led",
        lua.create_function(move |_, (column, row, how, bright): (usize, usize, Value, Value)| {
            let byte = colour_of("pad", &how, &bright)?;
            l.borrow_mut().pad(column.wrapping_sub(1), row.wrapping_sub(1), byte);
            Ok(())
        })?,
    )?;
    let l = lights.clone();
    d.set(
        "strip_led",
        lua.create_function(move |_, (strip, led, how, bright): (usize, usize, Value, Value)| {
            let byte = colour_of("strip", &how, &bright)?;
            l.borrow_mut().strip(strip.wrapping_sub(1), led.wrapping_sub(1), byte);
            Ok(())
        })?,
    )?;
    let l = lights.clone();
    d.set(
        "meter",
        lua.create_function(move |_, (column, led, level): (usize, usize, f64)| {
            let byte = (level.clamp(0.0, 1.0) * f64::from(ON)).round() as u8;
            l.borrow_mut().meter(column.wrapping_sub(1), led.wrapping_sub(1), byte);
            Ok(())
        })?,
    )?;
    Ok(())
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
