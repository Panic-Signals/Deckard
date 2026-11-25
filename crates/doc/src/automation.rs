//! Automation of objects that go away. A sound's automation lanes are its
//! `modulators` (`ObjectVector<Modulator>`, each `target` a pointer to the automated
//! parameter), and the automation itself is `ModulationEvent`s in the sound's sequences
//! (each `parameter` a pointer to that parameter). When a module goes (a slot removed,
//! a sound reset), Maschine deletes its lanes and events; left in place, the pointers
//! would make the writer put the parameter where it is next referenced, and Maschine
//! fails to load such a project (for a plug-in's `PluginParameter`, it crashes).
//!
//! Macros (`ForwardParameter`) assigned to such a parameter are unassigned.

use crate::notes::{events_with, items_of};
use crate::{Document, Op};
use mxprj::decode::{Obj, Path, Value};
use std::collections::HashSet;

const MODULATORS: &str = "NI::ML::TM::ObjectVector<NI::MASCHINE::DATA::Modulator>";
const SEQUENCE: &str = "NI::MASCOMMON::DATA::Sequence";
const FORWARD: &str = "NI::MASCOMMON::DATA::ForwardParameter";

/// Every object id defined in `v`.
pub(crate) fn defined_ids(v: &Value) -> HashSet<u64> {
    let mut ids = HashSet::new();
    v.walk(&mut |o| {
        if let Some(id) = o.oid {
            ids.insert(id);
        }
    });
    ids
}

/// Extend `ids` with every object id defined in `o` without cloning `o`.
pub(crate) fn extend_defined_ids(o: &Obj, ids: &mut HashSet<u64>) {
    o.walk_paths(&mut |obj, _| {
        if let Some(id) = obj.oid {
            ids.insert(id);
        }
    });
}

/// The object id a pointer field holds (a back-reference, or the object itself).
pub(crate) fn pointee(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Ref(id) => Some(*id),
        Value::Obj(o) => o.oid,
        _ => None,
    }
}

fn items_mut<'a>(ov: &'a mut Obj, vector: &str) -> Option<&'a mut Vec<Value>> {
    match ov.get_mut(vector)? {
        Value::Obj(o) => match o.get_mut("items")? {
            Value::List(l) => Some(l),
            _ => None,
        },
        _ => None,
    }
}

/// An `ObjectVector<Modulator>` without the lanes whose target is in `gone`; `None`
/// if it has none.
fn without_lanes(ov: &Obj, gone: &HashSet<u64>) -> Option<Obj> {
    let mut ov = ov.clone();
    let items = items_mut(&mut ov, "objects")?;
    let mut removed = HashSet::new();
    items.retain(|item| {
        let Some(m) = crate::arrange::px(item) else { return true };
        let hit = pointee(m.get("target")).is_some_and(|t| gone.contains(&t));
        if hit && let Some(id) = m.oid {
            removed.insert(id);
        }
        !hit
    });
    if removed.is_empty() {
        return None;
    }
    if pointee(ov.get("current")).is_some_and(|c| removed.contains(&c))
        && let Some(cur) = ov.get_mut("current")
    {
        *cur = Value::Null;
    }
    if let Some(sel) = items_mut(&mut ov, "selection") {
        sel.retain(|p| !pointee(Some(p)).is_some_and(|id| removed.contains(&id)));
    }
    Some(ov)
}

impl Document {
    /// Ops that delete the automation of the parameters in `gone` (object ids of a
    /// subtree being removed, at `skip`): their lanes, their events in every sequence,
    /// and macro assignments to them. Computed on the tree before the removal.
    pub(crate) fn drop_automation_ops(&self, gone: &HashSet<u64>, skip: &Path) -> Result<Vec<Op>, String> {
        self.drop_modulation_ops(gone, skip, true)
    }

    /// [`drop_automation_ops`](Self::drop_automation_ops), leaving macro assignments
    /// alone unless `macros`.
    pub(crate) fn drop_modulation_ops(&self, gone: &HashSet<u64>, skip: &Path, macros: bool) -> Result<Vec<Op>, String> {
        if gone.is_empty() {
            return Ok(Vec::new());
        }
        // Single pass: record matching ModulationEvent ids for back-reference resolution,
        // emit Modulators/Forward edits directly, and defer Sequence checks until after.
        let mut hit_mod_ids: HashSet<u64> = HashSet::new();
        let mut sequences: Vec<(Path, &Obj)> = Vec::new();
        let mut ops = Vec::new();
        self.root().walk_paths(&mut |o, path| {
            if let Some(id) = o.oid
                && o.type_name.contains("ModulationEvent")
                && pointee(o.get("parameter")).is_some_and(|p| gone.contains(&p))
            {
                hit_mod_ids.insert(id);
            }
            if path.starts_with(skip) {
                return;
            }
            match o.type_name.as_str() {
                MODULATORS => {
                    if let Some(ov) = without_lanes(o, gone) {
                        ops.push(Op::Set { path: path.to_vec(), value: Value::Obj(ov) });
                    }
                }
                SEQUENCE => sequences.push((path.to_vec(), o)),
                FORWARD if macros => {
                    if pointee(o.get("target")).is_some_and(|t| gone.contains(&t))
                        && let Some(i) = o.fields.iter().position(|(n, _)| n == "target")
                    {
                        ops.push(Op::Set { path: [path, &[i as u32]].concat(), value: Value::Null });
                    }
                }
                _ => {}
            }
        });
        for (path, o) in sequences {
            if let Some(seq) = without_events(o, gone, &hit_mod_ids)? {
                ops.push(Op::Set { path, value: Value::Obj(seq) });
            }
        }
        Ok(ops)
    }
}

/// A sequence without its modulation events of parameters in `gone`; `None` if it has
/// none.
fn without_events(seq: &Obj, gone: &HashSet<u64>, hit_mod_ids: &HashSet<u64>) -> Result<Option<Obj>, String> {
    let Some(Value::Obj(events)) = seq.get("events") else { return Ok(None) };
    let items = items_of(events)?;
    let mut removed = HashSet::new();
    let kept: Vec<_> = items
        .iter()
        .filter(|it| {
            let hit = match it.event {
                Value::Obj(ref te) => match te.get("event") {
                    Some(Value::Obj(ev)) => {
                        let m = ev.type_name.contains("ModulationEvent") && pointee(ev.get("parameter")).is_some_and(|p| gone.contains(&p));
                        if m && let Some(id) = ev.oid {
                            removed.insert(id);
                        }
                        m
                    }
                    Some(Value::Ref(id)) => {
                        let m = hit_mod_ids.contains(id);
                        if m {
                            removed.insert(*id);
                        }
                        m
                    }
                    _ => false,
                },
                _ => false,
            };
            !hit
        })
        .cloned()
        .collect();
    if kept.len() == items.len() {
        return Ok(None);
    }
    let mut seq = seq.clone();
    *seq.get_mut("events").unwrap() = Value::Obj(events_with(events, &kept));
    if pointee(seq.get("current")).is_some_and(|c| removed.contains(&c))
        && let Some(cur) = seq.get_mut("current")
    {
        *cur = Value::Null;
    }
    Ok(Some(seq))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path as FsPath;

    const DJ: &str = "/Users/Shared/Maschine 2 Library/Projects/DJ Jam Meister.mxprj";

    /// Ids targeted by lanes and modulation events anywhere in the tree.
    fn automated(d: &Document) -> HashSet<u64> {
        let mut out = HashSet::new();
        d.root().walk_paths(&mut |o, _| {
            if o.type_name == "NI::MASCHINE::DATA::Modulator" {
                out.extend(pointee(o.get("target")));
            }
            if o.type_name.contains("ModulationEvent") {
                out.extend(pointee(o.get("parameter")));
            }
        });
        out
    }

    #[test]
    fn removing_a_module_removes_its_automation() {
        let Ok(mut d) = Document::open(FsPath::new(DJ)) else {
            eprintln!("{DJ} not found; skipping");
            return;
        };
        // A slot that holds automated parameters.
        let auto = automated(&d);
        let mut found = None;
        'outer: for (g, gr) in d.project().groups.iter().enumerate() {
            for s in 0..gr.sounds.len() {
                let chain = d.chain_path(g, s).unwrap();
                let items = d.field_path(&chain, &["slots", "objects", "items"]).unwrap();
                let Some(Value::List(list)) = d.tree.at(&items) else { continue };
                for (i, item) in list.iter().enumerate() {
                    let ids = defined_ids(item);
                    if ids.iter().any(|id| auto.contains(id)) {
                        found = Some((g, s, i, ids));
                        break 'outer;
                    }
                }
            }
        }
        let (g, s, i, ids) = found.expect("an automated module");
        let before = d.to_bytes().unwrap();
        d.remove_slot(g, s, i).unwrap();
        let left = automated(&d);
        assert!(left.iter().all(|id| !ids.contains(id)), "automation of the removed module is left");
        assert!(!left.is_empty(), "other automation stays");
        // Saved, it reads back with no trace of it.
        let bytes = d.to_bytes().unwrap();
        let back = Document::from_bytes(bytes, None).unwrap();
        let mut types = HashSet::new();
        back.root().walk_paths(&mut |o, _| {
            types.insert(o.type_name.clone());
        });
        assert_eq!(automated(&back).len(), left.len());
        // One undo step brings it all back.
        assert!(d.undo());
        assert_eq!(d.to_bytes().unwrap(), before);
    }

    #[test]
    fn resetting_every_sound_leaves_no_automation() {
        let Ok(mut d) = Document::open(FsPath::new(DJ)) else { return };
        let groups: Vec<usize> = d.project().groups.iter().map(|g| g.sounds.len()).collect();
        for (g, &n) in groups.iter().enumerate() {
            for s in 0..n {
                d.reset_sound(g, s).unwrap();
            }
        }
        // Only automation of what is still there (group and master modules, sound
        // channel parameters) may be left; nothing points at a plug-in parameter.
        let bytes = d.to_bytes().unwrap();
        let back = Document::from_bytes(bytes, None).unwrap();
        let mut plugin_params = 0;
        back.root().walk_paths(&mut |o, _| plugin_params += (o.type_name == "NI::MASCOMMON::DATA::PluginParameter") as usize);
        let mut hosts = 0;
        back.root().walk_paths(&mut |o, _| hosts += (o.type_name == "NI::MASCHINE::DATA::PluginHost") as usize);
        assert!(hosts > 0 || plugin_params == 0, "plug-in parameters left without a plug-in");
    }
}
