//! Remote control of parameters (docs/superpowers/specs/2026-10-23-phase7-macros-design.md):
//! MIDI learn, the project's MIDI mapper (`midi_mapper/remote_parameters`, an
//! `ObjectMap<key, RemoteParameter>`; `remote_map` before project version 3), and host
//! automation, the project's 128 `remote_parameters` (the slot is the automation ID).
//! Parameters are named by their object ids, as a `RemoteParameter` points at them.

use super::{Document, Op};
use mxprj::decode::{Obj, Path, Value};

const REMOTE_PARAMETER: &str = "NI::MASCOMMON::DATA::RemoteParameter";

/// The remote key of a Program Change (Maschine's: CC n is n, poly pressure on note n
/// is 128 + n).
pub const MIDI_KEY_PROGRAM: u64 = 0x100;
/// Channel pressure.
pub const MIDI_KEY_PRESSURE: u64 = 0x101;
/// The pitch wheel.
pub const MIDI_KEY_PITCH: u64 = 0x102;

/// The remote key of a MIDI message, on any channel, and the value it sets (0..1); `None`
/// for messages that can't control a parameter (notes, system messages).
pub fn midi_key(bytes: &[u8]) -> Option<(u64, f64)> {
    let (&status, rest) = bytes.split_first()?;
    let v7 = |v: u8| v.min(127) as f64 / 127.0;
    match (status & 0xf0, rest) {
        (0xa0, [note, v, ..]) => Some((0x80 + (*note & 0x7f) as u64, v7(*v))),
        (0xb0, [cc, v, ..]) => Some(((*cc & 0x7f) as u64, v7(*v))),
        (0xc0, [p, ..]) => Some((MIDI_KEY_PROGRAM, v7(*p))),
        (0xd0, [v, ..]) => Some((MIDI_KEY_PRESSURE, v7(*v))),
        (0xe0, [lsb, msb, ..]) => Some((MIDI_KEY_PITCH, (((*msb as u32 & 0x7f) << 7) | (*lsb as u32 & 0x7f)) as f64 / 16383.0)),
        _ => None,
    }
}

/// A key as the Assignment area shows it: "CC 7", "PC", "PW", "CP", "PP 60".
pub fn midi_key_name(key: u64) -> String {
    match key {
        0..=0x7f => format!("CC {key}"),
        0x80..=0xff => format!("PP {}", key - 0x80),
        MIDI_KEY_PROGRAM => "PC".into(),
        MIDI_KEY_PRESSURE => "CP".into(),
        MIDI_KEY_PITCH => "PW".into(),
        _ => String::new(),
    }
}

/// How many host automation IDs a project has.
pub const HOST_SLOTS: usize = 128;

fn number(v: &Value) -> Option<u64> {
    match v {
        Value::Int(i) => u64::try_from(*i).ok(),
        Value::Float(f) if *f >= 0.0 => Some(*f as u64),
        _ => None,
    }
}

/// The object a `RemoteParameter` (or its shared_ptr) points at.
fn pointee(v: &Value) -> Option<u64> {
    let Value::Obj(o) = v else { return None };
    let rp = match o.get("px") {
        Some(Value::Obj(rp)) => rp,
        _ => o,
    };
    match rp.get("parameter")? {
        Value::Ref(id) => Some(*id),
        Value::Obj(p) => p.oid,
        _ => None,
    }
}

impl Document {
    /// Where the object with id `oid` is defined.
    pub(crate) fn oid_path(&self, oid: u64) -> Option<Path> {
        let mut found = None;
        self.root().walk_paths(&mut |o, p| {
            if found.is_none() && o.oid == Some(oid) {
                found = Some(p.to_vec());
            }
        });
        found
    }

    /// Path of the MIDI mapper's map.
    fn midi_map_path(&self) -> Result<Path, String> {
        let root = Vec::new();
        self.field_in(&root, "midi_mapper/remote_parameters")
            .or_else(|| self.field_in(&root, "remote_map"))
            .ok_or_else(|| "this project has no MIDI mapper".into())
    }

    /// Every MIDI mapping: (remote key, the parameter's object id), by key.
    pub fn midi_mappings(&self) -> Vec<(u64, u64)> {
        let Ok(path) = self.midi_map_path() else { return Vec::new() };
        let Some(Value::Obj(map)) = self.tree.at(&path) else { return Vec::new() };
        let Some(Value::Obj(objects)) = map.get("objects") else { return Vec::new() };
        let Some(Value::List(items)) = objects.get("items") else { return Vec::new() };
        items
            .iter()
            .filter_map(|it| {
                let Value::Obj(pair) = it else { return None };
                Some((pair.get("first").and_then(number)?, pointee(pair.get("second")?)?))
            })
            .collect()
    }

    /// The MIDI control parameter `param` listens to.
    pub fn midi_key_for(&self, param: u64) -> Option<u64> {
        self.midi_mappings().into_iter().find(|m| m.1 == param).map(|m| m.0)
    }

    /// The parameter MIDI control `key` sets.
    pub fn midi_param_for(&self, key: u64) -> Option<u64> {
        self.midi_mappings().into_iter().find(|m| m.0 == key).map(|m| m.1)
    }

    /// Set the mapper's pairs to `keep` (the existing ones that stay) plus `add`.
    fn set_midi_map(&mut self, label: &str, keep: impl Fn(u64, u64) -> bool, add: Option<(u64, u64)>) -> Result<(), String> {
        let path = self.midi_map_path()?;
        let Some(Value::Obj(map)) = self.tree.at(&path).cloned() else { return Err("bad MIDI mapper".into()) };
        let mut map = map;
        let pair = match add {
            Some((key, param)) => Some(self.build(|b| {
                let mut rp = b.blank(REMOTE_PARAMETER)?;
                *rp.find_mut("parameter").ok_or("a RemoteParameter without its parameter")? = Value::Ref(param);
                Ok(Value::Obj(Obj {
                    type_name: format!("std::pair<unsigned long const, std::shared_ptr<{REMOTE_PARAMETER}>>"),
                    version: 0,
                    oid: None,
                    export: None,
                    fields: vec![("first".into(), Value::Int(key as i64)), ("second".into(), b.shared(REMOTE_PARAMETER, rp))],
                }))
            })?),
            None => None,
        };
        let objects = crate::build::obj_mut(&mut map, "objects")?;
        if !objects.fields.iter().any(|(n, _)| n == "items") {
            objects.fields.push(("items".into(), Value::List(Vec::new())));
        }
        let Some(Value::List(items)) = objects.get_mut("items") else { return Err("bad MIDI mapper".into()) };
        let key_of = |v: &Value| match v {
            Value::Obj(p) => p.get("first").and_then(number),
            _ => None,
        };
        items.retain(|v| {
            let (Some(k), Some(p)) = (
                key_of(v),
                match v {
                    Value::Obj(pair) => pair.get("second").and_then(pointee),
                    _ => None,
                },
            ) else {
                return true;
            };
            keep(k, p)
        });
        if let Some(pair) = pair {
            // A std::map: in key order.
            let key = add.map_or(0, |a| a.0);
            let at = items.iter().position(|v| key_of(v).is_some_and(|k| k > key)).unwrap_or(items.len());
            items.insert(at, pair);
        }
        self.apply(label, vec![Op::Set { path, value: Value::Obj(map) }])
    }

    /// MIDI control `key` now sets parameter `param`: its other control, and the
    /// control's other parameter, go. One undo step.
    pub fn learn_midi(&mut self, param: u64, key: u64) -> Result<(), String> {
        self.set_midi_map("MIDI Learn", |k, p| k != key && p != param, Some((key, param)))
    }

    /// No MIDI control sets parameter `param` any more.
    pub fn unlearn_midi(&mut self, param: u64) -> Result<(), String> {
        if self.midi_key_for(param).is_none() {
            return Ok(());
        }
        self.set_midi_map("MIDI Unlearn", |_, p| p != param, None)
    }

    /// Path of host automation slot `slot`'s `RemoteParameter`.
    fn host_slot_path(&self, slot: usize) -> Result<Path, String> {
        let items = self.field_in(&Vec::new(), "remote_parameters/objects/items").ok_or("no host automation slots")?;
        let at = [items.as_slice(), &[slot as u32]].concat();
        let Some(Value::Obj(sp)) = self.tree.at(&at) else { return Err("no such host automation slot".into()) };
        let px = sp.fields.iter().position(|(n, _)| n == "px").ok_or("bad host automation slot")?;
        let Value::Obj(rp) = &sp.fields[px].1 else { return Err("bad host automation slot".into()) };
        let fi = rp.fields.iter().position(|(n, _)| n == "parameter").ok_or("bad host automation slot")?;
        Ok([at.as_slice(), &[px as u32, fi as u32]].concat())
    }

    /// What each host automation ID controls (its parameter's object id).
    pub fn host_slots(&self) -> Vec<Option<u64>> {
        let Some(items) = self.field_in(&Vec::new(), "remote_parameters/objects/items") else { return Vec::new() };
        let Some(Value::List(list)) = self.tree.at(&items) else { return Vec::new() };
        list.iter().map(pointee).collect()
    }

    /// The host automation ID of parameter `param`.
    pub fn host_slot_for(&self, param: u64) -> Option<usize> {
        self.host_slots().iter().position(|p| *p == Some(param))
    }

    /// Point host automation ID `slot` at parameter `param` (`None`: free it).
    pub fn set_host_slot(&mut self, slot: usize, param: Option<u64>) -> Result<(), String> {
        let path = self.host_slot_path(slot)?;
        self.apply("Host Automation", vec![Op::Set { path, value: param.map_or(Value::Null, Value::Ref) }])
    }

    /// Give parameter `param` the lowest free host automation ID (its own if it has one).
    pub fn enable_host_automation(&mut self, param: u64) -> Result<usize, String> {
        let slots = self.host_slots();
        if let Some(i) = slots.iter().position(|p| *p == Some(param)) {
            return Ok(i);
        }
        let free = slots.iter().position(Option::is_none).ok_or("every host automation ID is in use")?;
        self.set_host_slot(free, Some(param))?;
        Ok(free)
    }

    /// Take parameter `param`'s host automation ID away.
    pub fn disable_host_automation(&mut self, param: u64) -> Result<(), String> {
        match self.host_slot_for(param) {
            Some(i) => self.set_host_slot(i, None),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Target;

    #[test]
    fn keys_of_midi_messages() {
        assert_eq!(midi_key(&[0xb3, 7, 127]), Some((7, 1.0)), "CC 7 on any channel");
        assert_eq!(midi_key(&[0xa0, 60, 0]), Some((0x80 + 60, 0.0)), "poly pressure on C3");
        assert_eq!(midi_key(&[0xc0, 127]).map(|k| k.0), Some(MIDI_KEY_PROGRAM));
        assert_eq!(midi_key(&[0xd5, 64]).map(|k| k.0), Some(MIDI_KEY_PRESSURE));
        let (k, v) = midi_key(&[0xe0, 0x7f, 0x7f]).unwrap();
        assert_eq!((k, v), (MIDI_KEY_PITCH, 1.0));
        assert_eq!(midi_key(&[0x90, 60, 100]), None, "notes don't");
        assert_eq!(midi_key(&[0xf8]), None);
        let names: Vec<String> = [7, 0x80 + 60, MIDI_KEY_PROGRAM, MIDI_KEY_PRESSURE, MIDI_KEY_PITCH].map(midi_key_name).to_vec();
        assert_eq!(names, ["CC 7", "PP 60", "PC", "CP", "PW"]);
    }

    /// Learn, relearn (the control's other parameter and the parameter's other control
    /// go), unlearn; saved and reopened.
    #[test]
    fn midi_mappings_learn_and_save() {
        let mut d = Document::new_project().unwrap();
        let level = d.param_oid(Target::Sound(0, 0), "level").unwrap();
        let pan = d.param_oid(Target::Sound(0, 0), "pan").unwrap();
        assert!(d.midi_mappings().is_empty());
        d.learn_midi(level, 7).unwrap();
        d.learn_midi(pan, 10).unwrap();
        assert_eq!(d.midi_mappings(), [(7, level), (10, pan)]);
        assert_eq!(d.undo_label(), Some("MIDI Learn"));
        // CC 10 to the level: the level's CC 7 and the pan's CC 10 go.
        d.learn_midi(level, 10).unwrap();
        assert_eq!(d.midi_mappings(), [(10, level)]);
        d.learn_midi(pan, MIDI_KEY_PITCH).unwrap();
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(back.midi_mappings(), [(10, level), (MIDI_KEY_PITCH, pan)]);
        assert_eq!(back.midi_param_for(MIDI_KEY_PITCH), Some(pan));
        d.unlearn_midi(level).unwrap();
        assert_eq!(d.midi_key_for(level), None);
        assert_eq!(d.midi_key_for(pan), Some(MIDI_KEY_PITCH));
        d.undo();
        assert_eq!(d.midi_key_for(level), Some(10));
    }

    /// A Maschine 3 project's (empty, version 1) mapper takes mappings and saves.
    #[test]
    fn a_maschine_project_takes_mappings() {
        let Ok(mut d) = Document::open(std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj")) else {
            return;
        };
        let cutoff = d.param_oid(Target::Sampler(0, 0), "cutoff").unwrap();
        d.learn_midi(cutoff, 74).unwrap();
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(back.midi_mappings(), [(74, cutoff)]);
    }

    /// Host automation: the lowest free ID, kept for a parameter that has one; freed.
    #[test]
    fn host_automation_ids() {
        let mut d = Document::new_project().unwrap();
        let level = d.param_oid(Target::Sound(0, 0), "level").unwrap();
        let pan = d.param_oid(Target::Sound(0, 0), "pan").unwrap();
        assert_eq!(d.host_slots().len(), HOST_SLOTS);
        assert_eq!(d.enable_host_automation(level).unwrap(), 0);
        assert_eq!(d.enable_host_automation(pan).unwrap(), 1);
        assert_eq!(d.enable_host_automation(level).unwrap(), 0, "its own");
        d.disable_host_automation(level).unwrap();
        let mixer = d.param_oid(Target::Song, "level").unwrap();
        assert_eq!(d.enable_host_automation(mixer).unwrap(), 0, "the freed one");
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(back.host_slot_for(pan), Some(1));
        assert_eq!(back.host_slot_for(mixer), Some(0));
    }
}
