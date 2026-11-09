//! Lock snapshots (docs/superpowers/plans/2026-10-22-phase6-lock.md): the song's
//! `ParameterSnapshotsManager` holds up to 64 (slot = bank × 16 + pad), each the values
//! of every modulatable parameter, by reference to the parameter (`MaschineParameter*`).
//! Taking one captures what Maschine captures; recalling sets them back.

use crate::{Document, Op, Target};
use mxprj::decode::{Obj, Path, Value};

/// One entry: where the parameter's value is, and the value the snapshot holds.
#[derive(Debug, Clone, PartialEq)]
pub struct LockEntry {
    pub path: Path,
    pub value: f64,
}

const SNAPSHOT: &str = "NI::MASCHINE::DATA::ParameterSnapshot";
const VALUES_ENTRY: &str = "NI::MASCHINE::DATA::ParameterSnapshotValuesMap::Entry";
const ENTRY_PAIR: &str = "std::pair<NI::MASCOMMON::DATA::MaschineParameter*, NI::MASCHINE::DATA::ParameterSnapshotValuesMap::Entry>";
const SLOT: &str = "NI::MASCOMMON::DATA::Slot";

/// A mixer strip's parameters Lock takes (a sound's, a group's).
const MIXER: [&str; 8] = ["level", "pan", "mute", "aux1_level", "aux2_level", "transpose", "input_gain", "audio_mute"];
/// A mixing layer's swing.
const SWING: [&str; 3] = ["swing_amount", "swing_cycle", "swing_invert"];

/// A parameter Lock takes: its object id, its value, and its owner and name (for the
/// level / mute pairs).
struct Taken {
    oid: u64,
    value: f64,
    owner: usize,
    name: String,
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

/// Whether `o` is a parameter (a tracked object with a value).
fn is_param(o: &Obj) -> bool {
    o.oid.is_some() && o.type_name.contains("Parameter") && o.find("value").and_then(number).is_some()
}

/// The parameters under `o` that Lock takes: every one inside a slot's module; a
/// sound's or a group's mixer strip; the swing; each slot's on/off. `owner` numbers
/// the object holding each (to pair levels with mutes).
fn collect(o: &Obj, in_module: bool, owners: &mut usize, out: &mut Vec<Taken>, slots: &mut Vec<u64>) {
    *owners += 1;
    let me = *owners;
    let strip = ["::Sound", "::Track", "::Group"].iter().any(|t| o.type_name.ends_with(t));
    let slot = o.type_name == SLOT;
    if slot && let Some(id) = o.oid {
        slots.push(id);
    }
    let visit = |name: &str, c: &Obj, out: &mut Vec<Taken>, slots: &mut Vec<u64>, owners: &mut usize| {
        if name == "parameter_snapshots" || name == "patterns" || name == "scenes" || name == "sections" {
            return;
        }
        if is_param(c) {
            let take = in_module || (strip && MIXER.contains(&name)) || SWING.contains(&name) || (slot && name == "active");
            if take {
                out.push(Taken { oid: c.oid.unwrap(), value: c.find("value").and_then(number).unwrap(), owner: me, name: name.into() });
            }
            return;
        }
        collect(c, in_module || (slot && name == "module"), owners, out, slots);
    };
    for (name, v) in &o.fields {
        match v {
            Value::Obj(c) => visit(name, c, out, slots, owners),
            Value::List(l) => {
                for it in l {
                    if let Value::Obj(c) = it {
                        visit(name, c, out, slots, owners);
                    }
                }
            }
            _ => {}
        }
    }
}

impl Document {
    /// Path of the song's snapshot map (`ObjectMap<slot, ParameterSnapshot>`).
    fn snapshots_path(&self) -> Result<Path, String> {
        let song = self.owner(Target::Song).ok_or("no song")?;
        let rel = self.field_in(&song, "parameter_snapshots/snapshots").ok_or("the song has no Lock snapshots")?;
        Ok([song.as_slice(), &rel].concat())
    }

    fn snapshots_map(&self) -> Option<&Obj> {
        match self.tree.at(&self.snapshots_path().ok()?)? {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }

    /// The object with id `oid`, wherever it is.
    fn by_oid(&self, oid: u64) -> Option<(&Obj, Path)> {
        let mut found = None;
        self.root().walk_paths(&mut |o, p| {
            if found.is_none() && o.oid == Some(oid) {
                found = Some((o, p.to_vec()));
            }
        });
        found
    }

    fn deref<'a>(&'a self, v: &'a Value) -> Option<&'a Obj> {
        match v {
            Value::Obj(o) => Some(o),
            Value::Ref(id) => self.by_oid(*id).map(|x| x.0),
            _ => None,
        }
    }

    /// The (slot, snapshot) pairs of the song's map.
    fn snapshot_pairs(&self) -> Vec<(usize, &Obj)> {
        let Some(map) = self.snapshots_map() else { return Vec::new() };
        let Some(Value::Obj(objects)) = map.get("objects") else { return Vec::new() };
        let Some(Value::List(items)) = objects.get("items") else { return Vec::new() };
        items
            .iter()
            .filter_map(|it| {
                let Value::Obj(pair) = it else { return None };
                let slot = pair.get("first").and_then(number)? as usize;
                let Value::Obj(sp) = pair.get("second")? else { return None };
                Some((slot, self.deref(sp.get("px")?)?))
            })
            .collect()
    }

    /// The snapshot in `slot`.
    pub(crate) fn snapshot_obj(&self, slot: usize) -> Option<&Obj> {
        self.snapshot_pairs().into_iter().find(|(s, _)| *s == slot).map(|x| x.1)
    }

    /// Take a snapshot into `slot` (0–63), replacing one there; it becomes the focused
    /// one. One undo step.
    pub fn take_snapshot(&mut self, slot: usize) -> Result<(), String> {
        if slot >= 64 {
            return Err("Lock has 64 slots".into());
        }
        let path = self.snapshots_path()?;
        let song = self.owner(Target::Song).ok_or("no song")?;
        let Some(Value::Obj(song_obj)) = self.tree.at(&song) else { return Err("no song".into()) };
        let (mut taken, mut slots, mut owners) = (Vec::new(), Vec::new(), 0);
        collect(song_obj, false, &mut owners, &mut taken, &mut slots);
        // Each level with its mute and the mute with its level.
        let partner = |t: &Taken, other: &str| taken.iter().find(|x| x.owner == t.owner && x.name == other);
        let entries: Vec<Value> = taken
            .iter()
            .map(|t| {
                let mut e = vec![("value".to_string(), Value::Float(t.value)), ("has_target".to_string(), Value::Int(0))];
                let other = match t.name.as_str() {
                    "level" => partner(t, "mute"),
                    "mute" => partner(t, "level"),
                    _ => None,
                };
                if let Some(o) = other {
                    e[1].1 = Value::Int(1);
                    e.push(("target".into(), Value::Ref(o.oid)));
                    e.push(("target_value".into(), Value::Float(o.value)));
                }
                let entry = Obj { type_name: VALUES_ENTRY.into(), version: 0, oid: None, export: None, fields: e };
                Value::Obj(Obj {
                    type_name: ENTRY_PAIR.into(),
                    version: 0,
                    oid: None,
                    export: None,
                    fields: vec![("parameter".into(), Value::Ref(t.oid)), ("entry".into(), Value::Obj(entry))],
                })
            })
            .collect();
        let slot_refs: Vec<Value> = slots
            .iter()
            .map(|id| {
                Value::Obj(Obj {
                    type_name: format!("std::shared_ptr<{SLOT}>"),
                    version: 1,
                    oid: None,
                    export: None,
                    fields: vec![("px".into(), Value::Ref(*id))],
                })
            })
            .collect();
        let Some(Value::Obj(map)) = self.tree.at(&path).cloned() else { return Err("no snapshot map".into()) };
        let mut map = map;
        let (snap_value, snap_oid) = self.build(|b| {
            let mut snap = b.blank(SNAPSHOT)?;
            crate::build::push_items(crate::build::obj_mut(&mut snap, "slots")?, slot_refs, 1)?;
            let values = crate::build::obj_mut(&mut snap, "values")?;
            let flat = crate::build::obj_mut(values, "entries")?;
            crate::build::push_items(flat, entries, 0)?;
            let oid = snap.oid.ok_or("a snapshot without an id")?;
            Ok((b.shared(SNAPSHOT, snap), oid))
        })?;
        let pair_type = format!("std::pair<unsigned long const, std::shared_ptr<{SNAPSHOT}>>");
        let pair = Value::Obj(Obj {
            type_name: pair_type,
            version: 0,
            oid: None,
            export: None,
            fields: vec![("first".into(), Value::Int(slot as i64)), ("second".into(), snap_value)],
        });
        let objects = crate::build::obj_mut(&mut map, "objects")?;
        if !objects.fields.iter().any(|(n, _)| n == "items") {
            objects.fields.push(("items".into(), Value::List(Vec::new())));
        }
        let Some(Value::List(items)) = objects.get_mut("items") else { return Err("snapshot map without items".into()) };
        let key = |v: &Value| match v {
            Value::Obj(p) => p.get("first").and_then(number).unwrap_or(-1.0) as i64,
            _ => -1,
        };
        items.retain(|v| key(v) != slot as i64);
        let at = items.iter().position(|v| key(v) > slot as i64).unwrap_or(items.len());
        items.insert(at, pair);
        *map.get_mut("current").ok_or("snapshot map without current")? = Value::Ref(snap_oid);
        self.apply("Take Lock Snapshot", vec![Op::Set { path, value: Value::Obj(map) }])
    }

    /// Delete the snapshot in `slot`. One undo step.
    pub fn delete_snapshot(&mut self, slot: usize) -> Result<(), String> {
        let path = self.snapshots_path()?;
        let oid = self.snapshot_obj(slot).and_then(|o| o.oid).ok_or("no snapshot there")?;
        let Some(Value::Obj(mut map)) = self.tree.at(&path).cloned() else { return Err("no snapshot map".into()) };
        let objects = crate::build::obj_mut(&mut map, "objects")?;
        if let Some(Value::List(items)) = objects.get_mut("items") {
            items.retain(|v| !matches!(v, Value::Obj(p) if p.get("first").and_then(number) == Some(slot as f64)));
        }
        if matches!(map.get("current"), Some(Value::Ref(id)) if *id == oid) {
            *map.get_mut("current").unwrap() = Value::Null;
        }
        self.apply("Delete Lock Snapshot", vec![Op::Set { path, value: Value::Obj(map) }])
    }

    /// Focus the snapshot in `slot` (the one LOCK updates and recalls).
    pub fn focus_snapshot(&mut self, slot: usize) -> Result<(), String> {
        let path = self.snapshots_path()?;
        let oid = self.snapshot_obj(slot).and_then(|o| o.oid).ok_or("no snapshot there")?;
        let Some(Value::Obj(map)) = self.tree.at(&path) else { return Err("no snapshot map".into()) };
        let at = map.find_path("current").ok_or("snapshot map without current")?;
        self.apply("Focus Lock Snapshot", vec![Op::Set { path: [path.as_slice(), &at].concat(), value: Value::Ref(oid) }])
    }

    /// The snapshot in `slot`: its parameters' value paths and stored values (either
    /// of Maschine's two value maps).
    pub fn snapshot_entries(&self, slot: usize) -> Vec<LockEntry> {
        let Some(snap) = self.snapshot_obj(slot) else { return Vec::new() };
        let mut raw: Vec<(u64, f64)> = Vec::new();
        snap.walk_paths(&mut |o, _| {
            let (param, value) = match (o.get("parameter"), o.get("entry"), o.get("value")) {
                // v1: parameter -> Entry { value, … }.
                (Some(p), Some(Value::Obj(e)), _) => (p, e.get("value").and_then(number)),
                // v0: parameter -> value.
                (Some(p), None, Some(v)) => (p, number(v)),
                _ => return,
            };
            let id = match param {
                Value::Ref(id) => Some(*id),
                Value::Obj(o) => o.oid,
                _ => None,
            };
            if let (Some(id), Some(v)) = (id, value) {
                raw.push((id, v));
            }
        });
        // Where each parameter is, in one walk.
        let want: std::collections::HashSet<u64> = raw.iter().map(|x| x.0).collect();
        let mut at: std::collections::HashMap<u64, Path> = std::collections::HashMap::new();
        self.root().walk_paths(&mut |o, p| {
            if let Some(id) = o.oid
                && want.contains(&id)
                && !at.contains_key(&id)
                && let Some(vp) = o.find_path("value")
            {
                at.insert(id, [p, vp.as_slice()].concat());
            }
        });
        raw.into_iter().filter_map(|(id, value)| Some(LockEntry { path: at.get(&id)?.clone(), value })).collect()
    }

    /// The number at `path` and whether it's continuous (a float, not a switch or a
    /// choice).
    pub fn number_at(&self, path: &[u32]) -> Option<(f64, bool)> {
        match self.tree.at(path)? {
            Value::Int(i) => Some((*i as f64, false)),
            Value::Float(f) => Some((*f, true)),
            _ => None,
        }
    }

    /// MIDI Lock Change's trigger: 0 None, 1 MIDI notes, 2 program changes. Maschine 3
    /// stores it as such; projects from before keep on/off and a kind (0 notes,
    /// 1 programs). One undo step.
    pub fn set_lock_midi_trigger(&mut self, trigger: usize) -> Result<(), String> {
        let t = trigger.min(2);
        if self.num(Target::Song, "snapshot_recall/trigger").is_some() {
            return self.set("MIDI Change", Target::Song, "snapshot_recall/trigger", Value::Int(t as i64));
        }
        self.transaction("MIDI Change", |d| {
            d.set("MIDI Change", Target::Song, "snapshot_recall/legacy_bool", Value::Int(i64::from(t != 0)))?;
            if t != 0 {
                d.set("MIDI Change", Target::Song, "snapshot_recall/legacy_enum", Value::Int(t as i64 - 1))?;
            }
            Ok(())
        })
    }

    /// Set Edit ▸ MIDI Change's `block` (`scene_recall`, `section_recall`): `field` is
    /// `trigger`, `input` or `midi_channel`. A project too old to hold it keeps it in
    /// the DECKARD block.
    pub fn set_midi_recall(&mut self, block: &str, field: &str, value: Value) -> Result<(), String> {
        let path = format!("{block}/{field}");
        if self.param_path(Target::Song, &path).is_ok() {
            return self.set("MIDI Change", Target::Song, &path, value);
        }
        let p = self.project();
        let mut r = if block == "scene_recall" { p.scene_recall.clone() } else { p.section_recall.clone() };
        match (field, value) {
            ("trigger", Value::Int(t)) => r.trigger = t.clamp(0, 2) as usize,
            ("midi_channel", Value::Int(c)) => r.channel = c.clamp(1, 16),
            ("input", Value::Str(s)) => r.source = s,
            (f, v) => return Err(format!("{block}: can't set {f} to {v:?}")),
        }
        let want = Some(Value::Str(deckard_model::recall_to_text(&r)));
        self.apply("MIDI Change", vec![Op::SetField { path: Vec::new(), field: format!("deckard:{block}"), value: want }])
    }

    /// What of Scene Tempo and Edit ▸ MIDI Change only the DECKARD block can keep.
    pub(crate) fn scene_settings_text(&self) -> String {
        let mut out = String::new();
        for s in 0..self.project().scenes.len() {
            let Some(path) = self.owner(Target::Scene(s)) else { continue };
            if let Some(Value::Obj(o)) = self.tree.at(&path)
                && let Some(Value::Float(bpm)) = o.get(deckard_model::SCENE_TEMPO_FIELD)
            {
                out.push_str(&format!("tempo {s} {bpm}\n"));
            }
        }
        if let Some(Value::Obj(root)) = self.tree.at(&[]) {
            for block in ["scene_recall", "section_recall"] {
                if let Some(Value::Str(t)) = root.get(&format!("deckard:{block}")) {
                    out.push_str(&format!("{block} {t}\n"));
                }
            }
        }
        out
    }

    /// Take back what [`Document::scene_settings_text`] kept (a scene that's gone: dropped).
    pub(crate) fn read_scene_settings(&mut self, text: &str) {
        for line in text.lines() {
            let Some((what, rest)) = line.split_once(' ') else { continue };
            let op = if what == "tempo" {
                let Some((s, bpm)) = rest.split_once(' ') else { continue };
                let (Ok(s), Ok(bpm)) = (s.parse::<usize>(), bpm.parse::<f64>()) else { continue };
                let Some(path) = self.owner(Target::Scene(s)) else { continue };
                Op::SetField { path, field: deckard_model::SCENE_TEMPO_FIELD.to_string(), value: Some(Value::Float(bpm)) }
            } else {
                Op::SetField { path: Vec::new(), field: format!("deckard:{what}"), value: Some(Value::Str(rest.to_string())) }
            };
            let _ = self.run(&[op]);
        }
    }

    /// Give `scene` its own tempo, or (`None`) back to the song's. One undo step.
    pub fn set_scene_tempo(&mut self, scene: usize, bpm: Option<f64>) -> Result<(), String> {
        let bpm = bpm.map(|b| b.clamp(5.0, 999.0));
        // A project too old to hold it keeps it in the DECKARD block.
        if self.param_path(Target::Scene(scene), "scene_tempo_active").is_err() {
            let path = self.owner(Target::Scene(scene)).ok_or_else(|| format!("no scene {}", scene + 1))?;
            let field = deckard_model::SCENE_TEMPO_FIELD.to_string();
            return self.apply("Scene Tempo", vec![Op::SetField { path, field, value: bpm.map(Value::Float) }]);
        }
        self.transaction("Scene Tempo", |d| {
            if let Some(bpm) = bpm {
                d.set("Scene Tempo", Target::Scene(scene), "tempo", Value::Float(bpm))?;
            }
            d.set("Scene Tempo", Target::Scene(scene), "scene_tempo_active", Value::Int(i64::from(bpm.is_some())))
        })
    }

    /// The ops setting `entries` (the parameters that differ), each as its kind.
    pub(crate) fn entry_ops(&self, entries: &[LockEntry]) -> Vec<Op> {
        // SoundConfig overrides the sound's own mute, aux and channel values.
        // A recall must use the same mirrored write as a knob or mute gesture.
        let mut mirrored = std::collections::HashMap::new();
        for (g, group) in self.project().groups.iter().enumerate() {
            for s in 0..group.sounds.len() {
                for (field, _) in crate::channel::MIRRORED {
                    if let Ok(path) = self.param_path(Target::Sound(g, s), field) {
                        mirrored.insert(path, (Target::Sound(g, s), field));
                    }
                }
            }
        }
        let mut ops = Vec::new();
        for entry in entries {
            let Some(old) = self.tree.at(&entry.path) else { continue };
            let value = match old {
                Value::Int(_) => Value::Int(entry.value.round() as i64),
                Value::Float(_) => Value::Float(entry.value),
                _ => continue,
            };
            if let Some(&(target, field)) = mirrored.get(&entry.path) {
                if let Ok(edits) = self.param_ops(target, field, value) {
                    ops.extend(edits);
                }
            } else if match (old, &value) {
                (Value::Int(a), Value::Int(b)) => a != b,
                (Value::Float(a), Value::Float(b)) => (a - b).abs() > 1e-9,
                _ => false,
            } {
                ops.push(Op::Set { path: entry.path.clone(), value });
            }
        }
        ops
    }

    /// Set every parameter of the snapshot in `slot` back (one undo step, "Recall Lock").
    pub fn recall_snapshot(&mut self, slot: usize) -> Result<(), String> {
        if self.snapshot_obj(slot).is_none() {
            return Err("no snapshot there".into());
        }
        let ops = self.entry_ops(&self.snapshot_entries(slot));
        if ops.is_empty() {
            return Ok(());
        }
        self.apply("Recall Lock", ops)
    }

    /// Set parameters to values on the way to a snapshot (a morph's step), as one undo
    /// step with the steps before it ("Morph Lock").
    pub fn morph_step(&mut self, entries: &[LockEntry]) -> Result<(), String> {
        let ops = self.entry_ops(entries);
        if ops.is_empty() {
            return Ok(());
        }
        self.apply("Morph Lock", ops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewModule;

    fn project() -> Document {
        let mut d = Document::new_project().unwrap();
        d.add_slot(0, 0, 0, &NewModule::Effect("Filter")).unwrap();
        d
    }

    fn level(d: &Document) -> f64 {
        d.num(Target::Sound(0, 0), "level").unwrap()
    }

    fn cutoff(d: &Document) -> f64 {
        d.num(Target::Module(0, 0, 0), "cutoff").unwrap()
    }

    #[test]
    fn a_snapshot_recalls_mixer_and_module_parameters() {
        let mut d = project();
        d.set("Level", Target::Sound(0, 0), "level", Value::Float(0.5)).unwrap();
        d.set("Cutoff", Target::Module(0, 0, 0), "cutoff", Value::Float(0.2)).unwrap();
        d.take_snapshot(3).unwrap();
        assert_eq!(d.project().lock.slots, [3]);
        assert_eq!(d.project().lock.focus, Some(3));
        assert_eq!(d.undo_label(), Some("Take Lock Snapshot"));
        assert!(d.snapshot_entries(3).len() > 20, "the mixer and the module's parameters");
        d.set("Level", Target::Sound(0, 0), "level", Value::Float(0.9)).unwrap();
        d.set("Cutoff", Target::Module(0, 0, 0), "cutoff", Value::Float(0.8)).unwrap();
        d.recall_snapshot(3).unwrap();
        assert!((level(&d) - 0.5).abs() < 1e-6);
        assert!((cutoff(&d) - 0.2).abs() < 1e-6);
        assert_eq!(d.undo_label(), Some("Recall Lock"));
        assert!(d.undo());
        assert!((level(&d) - 0.9).abs() < 1e-6, "recall undoes");
    }

    #[test]
    fn lock_recall_restores_sound_config_mirrors_and_undoes_together() {
        let mut d = project();
        let initial_aux = d.project().groups[0].sounds[2].aux[0];
        d.take_snapshot(0).unwrap();
        d.set_mute(Target::Sound(0, 2), true).unwrap();
        d.set("Aux", Target::Sound(0, 2), "aux1_level", Value::Float(0.2)).unwrap();
        d.recall_snapshot(0).unwrap();
        assert!(!d.project().groups[0].sounds[2].muted);
        assert!((d.project().groups[0].sounds[2].aux[0] - initial_aux).abs() < 1e-6);
        let rev = d.revision();
        d.recall_snapshot(0).unwrap();
        assert_eq!(d.revision(), rev, "unchanged recall is silent");
        d.undo();
        assert!(d.project().groups[0].sounds[2].muted);
        assert!((d.project().groups[0].sounds[2].aux[0] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn snapshots_save_and_delete() {
        let mut d = project();
        d.set("Level", Target::Sound(0, 0), "level", Value::Float(0.5)).unwrap();
        d.take_snapshot(0).unwrap();
        d.take_snapshot(17).unwrap();
        let mut back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(back.project().lock.slots, [0, 17]);
        assert_eq!(back.project().lock.focus, Some(17));
        back.set("Level", Target::Sound(0, 0), "level", Value::Float(0.9)).unwrap();
        back.recall_snapshot(0).unwrap();
        assert!((level(&back) - 0.5).abs() < 1e-6, "recalled after reopening");
        back.delete_snapshot(0).unwrap();
        assert_eq!(back.project().lock.slots, [17]);
        assert!(back.undo());
        assert_eq!(back.project().lock.slots, [0, 17]);
        back.focus_snapshot(0).unwrap();
        assert_eq!(back.project().lock.focus, Some(0));
    }

    /// A level is stored with its mute (and the mute with its level), as Maschine does.
    #[test]
    fn levels_are_paired_with_mutes() {
        let mut d = project();
        d.take_snapshot(0).unwrap();
        let snap = d.snapshot_obj(0).unwrap();
        let mut pairs = 0;
        snap.walk_paths(&mut |o, _| {
            if o.type_name == "NI::MASCHINE::DATA::ParameterSnapshotValuesMap::Entry" && o.get("target").is_some() {
                pairs += 1;
            }
        });
        // A new project: 16 sounds and the group, each a level and a mute.
        assert!(pairs >= 2 * 17, "{pairs}");
    }

    /// Maschine 2's snapshots (the older value map) read too.
    #[test]
    fn older_snapshots_read() {
        let Ok(d) = Document::open(std::path::Path::new("/Users/Shared/Chromatic Fire Library/Projects/Lightspeed.mxprj")) else {
            return;
        };
        let slot = d.project().lock.slots[0];
        // Maschine 2 kept only some parameters (here five), each found.
        let mut pairs = 0;
        d.snapshot_obj(slot).unwrap().walk_paths(&mut |o, _| pairs += usize::from(o.get("parameter").is_some()));
        assert!(pairs > 0);
        assert_eq!(d.snapshot_entries(slot).len(), pairs);
    }
}

#[cfg(test)]
mod midi_tests {
    use crate::{Document, Target};
    use mxprj::decode::Value;

    /// Scene Tempo: stored in the scene, one undo, and kept by a copy of the scene.
    #[test]
    fn a_scene_keeps_its_own_tempo() {
        let mut d = Document::new_project().unwrap();
        let song = d.project().tempo;
        assert_eq!(d.project().scenes[0].tempo, None);
        d.set_scene_tempo(0, Some(90.5)).unwrap();
        assert_eq!(d.project().scenes[0].tempo, Some(90.5));
        assert_eq!(d.project().tempo, song, "the song's stays");
        let copy = d.duplicate_scene(0).unwrap();
        assert_eq!(d.project().scenes[copy].tempo, Some(90.5));
        assert!(d.undo());
        d.set_scene_tempo(0, None).unwrap();
        assert_eq!(d.project().scenes[0].tempo, None);
        assert!(d.undo());
        assert_eq!(d.project().scenes[0].tempo, Some(90.5));
    }

    /// A new scene starts at the song's tempo, even when it's made from one with its own.
    #[test]
    fn a_new_scene_has_no_tempo_of_its_own() {
        let mut d = Document::new_project().unwrap();
        d.set_scene_tempo(0, Some(90.5)).unwrap();
        let new = d.insert_scene(1).unwrap();
        assert_eq!(d.project().scenes[new].tempo, None);
        assert_eq!(d.project().scenes[0].tempo, Some(90.5));
    }

    /// A project from before scenes had tempos, or Scene changes: without those fields.
    fn an_old_project() -> Document {
        let mut d = Document::new_project().unwrap();
        for s in 0..d.project().scenes.len() {
            let path = d.owner(Target::Scene(s)).unwrap();
            let Some(Value::Obj(o)) = d.tree.at_mut(&path) else { panic!("scene") };
            o.fields.retain(|(n, _)| n != "scene_tempo_active" && n != "tempo");
        }
        let path = d.owner(Target::Song).unwrap();
        let Some(Value::Obj(o)) = d.tree.at_mut(&path) else { panic!("song") };
        o.fields.retain(|(n, _)| n != "scene_recall");
        d.refresh();
        d
    }

    /// An old project keeps them too, undoable, and saved where Maschine won't look.
    #[test]
    fn an_old_project_keeps_scene_tempos_and_midi_changes() {
        let mut d = an_old_project();
        d.set_scene_tempo(0, Some(88.0)).unwrap();
        assert_eq!(d.project().scenes[0].tempo, Some(88.0));
        assert!(d.undo());
        assert_eq!(d.project().scenes[0].tempo, None);
        assert!(d.redo());
        let copy = d.duplicate_scene(0).unwrap();
        assert_eq!(d.project().scenes[copy].tempo, Some(88.0));
        let new = d.insert_scene(5).unwrap();
        assert_eq!(d.project().scenes[new].tempo, None, "a new one starts without");
        d.set_midi_recall("scene_recall", "trigger", Value::Int(2)).unwrap();
        d.set_midi_recall("scene_recall", "input", Value::Str("My Keys".into())).unwrap();
        d.set_midi_recall("scene_recall", "midi_channel", Value::Int(4)).unwrap();
        let want = deckard_model::MidiRecall { trigger: 2, source: "My Keys".into(), channel: 4 };
        assert_eq!(d.project().scene_recall, want);
        assert_eq!(d.scene_settings_text().lines().count(), 3, "{}", d.scene_settings_text());
        // Saved, and read back through the block alone.
        let text = d.scene_settings_text();
        let mut back = an_old_project();
        back.insert_scene(1).unwrap();
        back.read_scene_settings(&text);
        back.refresh();
        assert_eq!(back.project().scenes[0].tempo, Some(88.0));
        assert_eq!(back.project().scene_recall, want);
    }

    /// Edit ▸ MIDI Change's Scene and Section blocks are the song's.
    #[test]
    fn scene_and_section_midi_change_are_the_songs() {
        let mut d = Document::new_project().unwrap();
        for block in ["scene_recall", "section_recall"] {
            d.set_midi_recall(block, "trigger", Value::Int(2)).unwrap();
            d.set_midi_recall(block, "input", Value::Str("Keys".into())).unwrap();
            d.set_midi_recall(block, "midi_channel", Value::Int(3)).unwrap();
        }
        let want = deckard_model::MidiRecall { trigger: 2, source: "Keys".into(), channel: 3 };
        assert_eq!(d.project().scene_recall, want);
        assert_eq!(d.project().section_recall, want);
    }

    /// MIDI Lock Change (Edit ▸ MIDI Change ▸ Lock): the song's `snapshot_recall`.
    #[test]
    fn lock_midi_change_is_the_songs() {
        let mut d = Document::new_project().unwrap();
        let l = d.project().lock.clone();
        assert_eq!((l.midi_trigger, l.midi_source.as_str(), l.midi_channel), (0, "None", 1));
        d.set("MIDI Change", Target::Song, "snapshot_recall/trigger", Value::Int(2)).unwrap();
        d.set("MIDI Change", Target::Song, "snapshot_recall/input", Value::Str("Keys".into())).unwrap();
        d.set("MIDI Change", Target::Song, "snapshot_recall/midi_channel", Value::Int(3)).unwrap();
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        let l = &back.project().lock;
        assert_eq!((l.midi_trigger, l.midi_source.as_str(), l.midi_channel), (2, "Keys", 3));
    }

    /// A project from before Maschine 3 keeps it as on/off and a kind (note, program).
    #[test]
    fn older_projects_keep_it_as_on_and_a_kind() {
        let Ok(mut d) = Document::open(std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj")) else {
            return;
        };
        d.set_lock_midi_trigger(2).unwrap();
        assert_eq!(d.project().lock.midi_trigger, 2);
        d.set_lock_midi_trigger(1).unwrap();
        assert_eq!(d.project().lock.midi_trigger, 1);
        d.set_lock_midi_trigger(0).unwrap();
        assert_eq!(d.project().lock.midi_trigger, 0);
        let mut n = Document::new_project().unwrap();
        n.set_lock_midi_trigger(2).unwrap();
        assert_eq!(n.project().lock.midi_trigger, 2);
    }
}
