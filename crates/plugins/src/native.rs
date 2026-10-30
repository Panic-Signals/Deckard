//! DECKARD's own instruments and effects ("native" plug-ins).
//!
//! A native plug-in is stored in a project exactly like a VST3 plug-in: a `PluginHost`
//! module whose `plugin_id` is `"DECKARD:VST3:INSTR:<class id>"` (or `:FX:`) and whose
//! chunk is a `.vstpreset` holding [`Instance::state`] as component state. Maschine
//! sees it as a plug-in that isn't installed and keeps the module byte for byte when
//! it saves the project again (checked with `maschine-ref plant` + `roundtrip`); the
//! same class id can later be published as a real VST3.
//!
//! To add an engine: implement [`Engine`] (sound only; parameter values, state and
//! MIDI bookkeeping are handled here), describe its parameters and pages, and list it
//! in [`CLASSES`] under a class id of its own that never changes.
//!
//! State (version 1, little-endian): `"DKRD"`, `u32` version, `u32` n, then n ×
//! (`u32` parameter id, `f64` normalized value), then `u32` length + the engine's own
//! bytes ([`Engine::save`]). Parameters are found by id, so engines can add
//! parameters (missing ones load at their default) and drop them (ignored).

use crate::{ParamInfo, Transport};

/// The vendor name in a native plug-in's id.
pub const VENDOR: &str = "DECKARD";

/// A parameter of a native plug-in. Values are normalized (0..1) everywhere; the
/// engine maps them to its units.
#[derive(Debug, Clone, Copy)]
pub struct Param {
    /// Stable id (saved in projects; never reuse one for another meaning).
    pub id: u32,
    pub name: &'static str,
    /// Label on a knob, upper case as Maschine shows them.
    pub short: &'static str,
    pub default: f64,
    /// 0 = continuous, else the number of steps (a choice of `steps + 1` values).
    pub steps: i32,
}

/// A page of eight knobs: its name, and (parameter id, the section heading starting
/// at that knob, empty for none).
#[derive(Debug, Clone, Copy)]
pub struct Page {
    pub name: &'static str,
    pub knobs: &'static [(u32, &'static str)],
}

/// The sound of a native plug-in.
pub trait Engine: Send {
    /// A parameter changed (normalized value). Called for every parameter before the
    /// first `process`, and whenever one changes.
    fn set_param(&mut self, id: u32, value: f64);

    /// How a value of a parameter reads (`value` normalized).
    fn text(&self, id: u32, value: f64) -> String {
        let _ = id;
        format!("{:.0} %", value * 100.0)
    }

    /// A knob's label now, when it depends on the state (Plaits' change with its
    /// engine); `None` keeps the parameter's own.
    fn label(&self, id: u32) -> Option<&'static str> {
        let _ = id;
        None
    }

    /// Start a note (pitch 0..127, velocity 1..127).
    fn note_on(&mut self, pitch: u8, velocity: u8);

    /// Release a note.
    fn note_off(&mut self, pitch: u8);

    /// A MIDI control change (0..127 each).
    fn control_change(&mut self, number: u8, value: u8) {
        let _ = (number, value);
    }

    /// Pitch bend, -1..1.
    fn pitch_bend(&mut self, bend: f32) {
        let _ = bend;
    }

    /// Silence everything now (stop, a new state).
    fn reset(&mut self);

    /// Render a block: `left`/`right` hold the input (an effect's; silence for an
    /// instrument) and get the output.
    fn process(&mut self, left: &mut [f32], right: &mut [f32], transport: &Transport);

    /// Anything to save besides parameter values (a wavetable, a sequence…).
    fn save(&self) -> Vec<u8> {
        Vec::new()
    }

    /// What [`Engine::save`] returned (empty for a new instance).
    fn load(&mut self, data: &[u8]) {
        let _ = data;
    }
}

/// A native plug-in: its identity, parameters and how to make its engine.
pub struct Class {
    /// The VST3 class id (saved in projects; never change it).
    pub cid: [u8; 16],
    pub name: &'static str,
    /// An instrument (plays notes), else an effect.
    pub instrument: bool,
    pub params: &'static [Param],
    pub pages: &'static [Page],
    /// A new engine at `sample_rate`.
    pub make: fn(sample_rate: f64) -> Box<dyn Engine>,
}

impl Class {
    /// Maschine's `plugin_id` for it.
    pub fn plugin_id(&self) -> String {
        let role = if self.instrument { "INSTR" } else { "FX" };
        format!("{VENDOR}:VST3:{role}:{}", self.cid.iter().map(|b| format!("{b:02X}")).collect::<String>())
    }
}

/// Every native plug-in.
pub static CLASSES: &[&Class] = &[&crate::replicant::CLASS, &crate::plaits::CLASS];

/// The native plug-in with class id `cid`.
pub fn find(cid: &[u8; 16]) -> Option<&'static Class> {
    CLASSES.iter().copied().find(|c| &c.cid == cid)
}

const MAGIC: &[u8; 4] = b"DKRD";
const VERSION: u32 = 1;

/// A running native plug-in.
pub struct Instance {
    class: &'static Class,
    engine: Box<dyn Engine>,
    /// Normalized values, in `class.params` order.
    values: Vec<f64>,
    /// What modulation gives each, when it drives it (the engine plays that instead).
    modded: Vec<Option<f64>>,
    held: [bool; 128],
    /// Sustain pedal down, and notes it holds.
    pedal: bool,
    sustained: [bool; 128],
    out: (Vec<f32>, Vec<f32>),
    sample_rate: f64,
    dirty: std::sync::atomic::AtomicBool,
    /// The Maschine chunk last applied.
    pub(crate) chunk: Vec<u8>,
}

impl Instance {
    pub fn new(class: &'static Class, sample_rate: f64) -> Instance {
        let mut engine = (class.make)(sample_rate);
        let values: Vec<f64> = class.params.iter().map(|p| p.default).collect();
        for (p, v) in class.params.iter().zip(&values) {
            engine.set_param(p.id, *v);
        }
        Instance {
            class,
            engine,
            modded: vec![None; values.len()],
            values,
            held: [false; 128],
            pedal: false,
            sustained: [false; 128],
            out: (vec![0.0; crate::MAX_BLOCK], vec![0.0; crate::MAX_BLOCK]),
            sample_rate,
            dirty: Default::default(),
            chunk: Vec::new(),
        }
    }

    pub fn class(&self) -> &'static Class {
        self.class
    }

    pub(crate) fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// The saved form of the current settings (see the module docs).
    pub fn save(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(MAGIC);
        out.extend(VERSION.to_le_bytes());
        out.extend((self.values.len() as u32).to_le_bytes());
        for (p, v) in self.class.params.iter().zip(&self.values) {
            out.extend(p.id.to_le_bytes());
            out.extend(v.to_le_bytes());
        }
        let extra = self.engine.save();
        out.extend((extra.len() as u32).to_le_bytes());
        out.extend(extra);
        out
    }

    /// Restore settings from [`Instance::save`]'s bytes (empty: the defaults).
    pub fn load(&mut self, data: &[u8]) -> Result<(), String> {
        let mut values: Vec<f64> = self.class.params.iter().map(|p| p.default).collect();
        let mut extra: &[u8] = &[];
        if !data.is_empty() {
            let mut r = Reader(data);
            if r.take(4)? != MAGIC {
                return Err(format!("{}: not a DECKARD state", self.class.name));
            }
            let version = r.u32()?;
            if version > VERSION {
                return Err(format!("{}: saved by a newer DECKARD (state version {version})", self.class.name));
            }
            for _ in 0..r.u32()? {
                let (id, v) = (r.u32()?, f64::from_le_bytes(r.take(8)?.try_into().unwrap()));
                if let Some(i) = self.class.params.iter().position(|p| p.id == id)
                    && v.is_finite()
                {
                    values[i] = v.clamp(0.0, 1.0);
                }
            }
            let n = r.u32()? as usize;
            extra = r.take(n)?;
        }
        self.engine.reset();
        self.engine.load(extra);
        for (p, v) in self.class.params.iter().zip(&values) {
            self.engine.set_param(p.id, *v);
        }
        self.values = values;
        self.held = [false; 128];
        self.sustained = [false; 128];
        self.pedal = false;
        Ok(())
    }

    pub(crate) fn set_state(&mut self, state: &crate::state::State) -> Result<(), String> {
        self.load(&state.component)
    }

    pub(crate) fn state(&self) -> crate::state::State {
        crate::state::State { component: self.save(), controller: Vec::new() }
    }

    pub(crate) fn parameter_list(&self, _automatable: bool) -> Vec<ParamInfo> {
        self.class
            .params
            .iter()
            .map(|p| ParamInfo {
                id: p.id,
                title: p.name.to_string(),
                short: p.short.to_string(),
                units: String::new(),
                steps: p.steps,
                default: p.default,
            })
            .collect()
    }

    pub(crate) fn pages(&self) -> Vec<(String, Vec<(u32, String)>)> {
        self.class.pages.iter().map(|pg| (pg.name.to_string(), pg.knobs.iter().map(|(id, s)| (*id, s.to_string())).collect())).collect()
    }

    pub(crate) fn param_value(&self, id: u32) -> f64 {
        self.class.params.iter().position(|p| p.id == id).map_or(0.0, |i| self.values[i])
    }

    pub(crate) fn param_text(&self, id: u32, v: f64) -> String {
        self.engine.text(id, v)
    }

    pub(crate) fn param_label(&self, id: u32) -> Option<String> {
        self.engine.label(id).map(str::to_string)
    }

    pub(crate) fn set_param(&mut self, id: u32, v: f64) {
        let Some(i) = self.class.params.iter().position(|p| p.id == id) else { return };
        let p = &self.class.params[i];
        let v = if p.steps > 0 { (v.clamp(0.0, 1.0) * p.steps as f64).round() / p.steps as f64 } else { v.clamp(0.0, 1.0) };
        if self.values[i] != v {
            self.values[i] = v;
            // Modulation holds it until it lets go (then the knob's value plays).
            if self.modded[i].is_none() {
                self.engine.set_param(id, v);
            }
            self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub(crate) fn modulate(&mut self, id: u32, v: f64) {
        let Some(i) = self.class.params.iter().position(|p| p.id == id) else { return };
        let v = v.clamp(0.0, 1.0);
        if self.modded[i] != Some(v) {
            self.modded[i] = Some(v);
            self.engine.set_param(id, v);
        }
    }

    pub(crate) fn unmodulate(&mut self, id: u32) {
        let Some(i) = self.class.params.iter().position(|p| p.id == id) else { return };
        if self.modded[i].take().is_some() {
            self.engine.set_param(id, self.values[i]);
        }
    }

    pub(crate) fn modulated(&self, id: u32) -> Option<f64> {
        self.class.params.iter().position(|p| p.id == id).and_then(|i| self.modded[i])
    }

    pub(crate) fn take_dirty(&self) -> bool {
        self.dirty.swap(false, std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn note_on(&mut self, pitch: i64, velocity: i64) {
        let p = pitch.clamp(0, 127) as usize;
        if self.held[p] || self.sustained[p] {
            self.engine.note_off(p as u8);
        }
        self.held[p] = true;
        self.sustained[p] = false;
        self.engine.note_on(p as u8, velocity.clamp(1, 127) as u8);
    }

    pub(crate) fn note_off(&mut self, pitch: i64) {
        let p = pitch.clamp(0, 127) as usize;
        if !std::mem::take(&mut self.held[p]) {
            return;
        }
        if self.pedal {
            self.sustained[p] = true;
        } else {
            self.engine.note_off(p as u8);
        }
    }

    pub(crate) fn all_notes_off(&mut self) {
        self.held = [false; 128];
        self.sustained = [false; 128];
        self.pedal = false;
        self.engine.pitch_bend(0.0);
        self.engine.reset();
    }

    /// Native engines take every controller (the sustain pedal is handled here).
    pub(crate) fn takes_controller(&self, _number: usize) -> bool {
        true
    }

    pub(crate) fn control_change(&mut self, number: u8, value: u8) {
        if number == 64 {
            self.pedal = value >= 64;
            if !self.pedal {
                for p in 0..128 {
                    if std::mem::take(&mut self.sustained[p]) {
                        self.engine.note_off(p as u8);
                    }
                }
            }
            return;
        }
        self.engine.control_change(number.min(127), value.min(127));
    }

    pub(crate) fn pitch_bend(&mut self, value: u16) {
        self.engine.pitch_bend((value.min(16383) as f32 - 8192.0) / 8192.0);
    }

    pub(crate) fn process_input(&mut self, frames: usize, transport: &Transport, input: Option<(&[f32], &[f32])>) -> (&[f32], &[f32]) {
        let n = frames.min(crate::MAX_BLOCK);
        let (l, r) = (&mut self.out.0[..n], &mut self.out.1[..n]);
        l.fill(0.0);
        r.fill(0.0);
        if let Some((il, ir)) = input {
            let k = n.min(il.len()).min(ir.len());
            l[..k].copy_from_slice(&il[..k]);
            r[..k].copy_from_slice(&ir[..k]);
        }
        self.engine.process(l, r, transport);
        (&self.out.0[..n], &self.out.1[..n])
    }

    pub(crate) fn output_bus(&self, bus: usize, frames: usize) -> Option<(&[f32], &[f32])> {
        let n = frames.min(crate::MAX_BLOCK);
        (bus == 0).then(|| (&self.out.0[..n], &self.out.1[..n]))
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("DECKARD state is truncated".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Host, Slot};
    use std::sync::Arc;

    /// A native plug-in's state goes through Maschine's chunk (a `.vstpreset` in the
    /// `PluginHost`) and back; an empty chunk, as Maschine re-saves one it never ran,
    /// loads the defaults.
    #[test]
    fn chunks_load_through_the_host() {
        let class = CLASSES[0];
        let id = class.plugin_id();
        let param = class.params.iter().find(|p| p.steps == 0).unwrap().id;
        let mut host = Host::default();
        let a = host.instance(Slot::Sound { group: 0, sound: 0 }, &id, class.name, &[], 48_000.0).unwrap();
        let chunk = {
            let mut p = a.lock().unwrap();
            assert!(p.is_native());
            p.set_param(param, 0.123);
            assert!(p.take_dirty());
            p.save_chunk()
        };
        let b = host.instance(Slot::Sound { group: 0, sound: 1 }, &id, class.name, &chunk, 48_000.0).unwrap();
        assert_eq!(b.lock().unwrap().param_value(param), 0.123);
        assert_eq!(b.lock().unwrap().save_chunk(), chunk);
        let empty = [1u32.to_le_bytes(), 1u32.to_le_bytes(), 0u32.to_le_bytes()].concat();
        let c = host.instance(Slot::Sound { group: 0, sound: 2 }, &id, class.name, &empty, 48_000.0).unwrap();
        let default = class.params.iter().find(|p| p.id == param).unwrap().default;
        assert_eq!(c.lock().unwrap().param_value(param), default);
        // The same slot with another chunk reloads in place.
        let again = host.instance(Slot::Sound { group: 0, sound: 2 }, &id, class.name, &chunk, 48_000.0).unwrap();
        assert!(Arc::ptr_eq(&c, &again));
        assert_eq!(again.lock().unwrap().param_value(param), 0.123);
    }

    /// Modulation moves what the engine plays, not the knob (nor the saved state);
    /// let go, the engine has the knob's value again.
    #[test]
    fn modulation_leaves_the_knob() {
        let class = CLASSES[0];
        let param = class.params.iter().find(|p| p.steps == 0).unwrap().id;
        let mut p = crate::Plugin::native(class, crate::PluginId::parse(&class.plugin_id(), class.name).unwrap(), class.name, 48_000.0);
        let knob = p.param_value(param);
        p.modulate(param, 0.9);
        assert_eq!(p.modulated(param), Some(0.9));
        assert_eq!(p.param_value(param), knob);
        assert!(!p.take_dirty(), "no edit of the state");
        p.unmodulate(param);
        assert_eq!(p.modulated(param), None);
    }

    /// A knob turned while modulation holds the parameter is what plays after it.
    #[test]
    fn a_knob_turned_while_modulated_plays_after() {
        let class = CLASSES[0];
        let param = class.params.iter().find(|p| p.steps == 0).unwrap().id;
        let mut p = crate::Plugin::native(class, crate::PluginId::parse(&class.plugin_id(), class.name).unwrap(), class.name, 48_000.0);
        p.modulate(param, 0.9);
        p.set_param(param, 0.3);
        assert_eq!(p.modulated(param), Some(0.9), "modulation still holds it");
        p.unmodulate(param);
        assert_eq!(p.param_value(param), 0.3);
    }
}
