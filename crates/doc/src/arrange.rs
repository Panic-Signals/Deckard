//! Scene and arrangement edits, made the way Maschine 3 makes them. The rules come
//! from comparing files Maschine saved before and after each edit (`mxprj-diff`;
//! docs/mxprj-format.md §9):
//!
//! - A scene maps every group to a `PatternRef`; an empty slot's `pattern` is null.
//! - New and duplicated scenes are copies with fresh object ids. A duplicate goes
//!   right after its original and is named `<name> (<n>)`.
//! - Sections are a map keyed by number, which sets their order. Inserting renumbers
//!   the keys 0..n; deleting leaves a gap.
//! - Duplicating a section also duplicates its scene (appended to the scene list).
//! - Deleting a scene deletes the sections that play it.
//! - Changing a section's length switches its auto-length off.
//! - The scene and section lists' `current` pointers follow the new or edited item.

use super::{Document, Op, Target};
use mxprj::decode::{Obj, Path, Value};
use std::collections::{HashMap, HashSet};

/// Where the scene and section lists live.
struct Lists {
    scenes: Path,
    scenes_current: Path,
    scene_selection: Option<Path>,
    sections: Path,
    sections_current: Path,
}

/// The object an item of an `ObjectVector` (`shared_ptr{px}`) or `ObjectMap`
/// (`pair{first, second: shared_ptr{px}}`) holds.
pub(crate) fn px(item: &Value) -> Option<&Obj> {
    let Value::Obj(o) = item else { return None };
    let holder = match o.get("second") {
        Some(Value::Obj(s)) => s,
        _ => o,
    };
    match holder.get("px") {
        Some(Value::Obj(p)) => Some(p),
        _ => None,
    }
}

pub(crate) fn px_mut(item: &mut Value) -> Option<&mut Obj> {
    let Value::Obj(o) = item else { return None };
    let holder = if matches!(o.get("second"), Some(Value::Obj(_))) {
        match o.get_mut("second") {
            Some(Value::Obj(s)) => s,
            _ => unreachable!(),
        }
    } else {
        o
    };
    match holder.get_mut("px") {
        Some(Value::Obj(p)) => Some(p),
        _ => None,
    }
}

/// Set a parameter inside a detached object: its value, and `t0` to match.
fn set_param(o: &mut Obj, field: &str, value: Value) {
    if let Some(Value::Obj(p)) = o.find_mut(field) {
        if let Some(v) = p.find_mut("value") {
            *v = value.clone();
        }
        if let Some(t0) = p.get_mut("t0")
            && std::mem::discriminant(t0) == std::mem::discriminant(&value)
        {
            *t0 = value;
        }
    }
}

fn param_str(o: &Obj, field: &str) -> String {
    match o.find(field) {
        Some(Value::Obj(p)) => match p.find("value") {
            Some(Value::Str(s)) => s.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// `<base> (<n>)` with the smallest n not used yet, as Maschine names copies.
fn copy_name(base: &str, taken: &[String]) -> String {
    (1..).map(|n| format!("{base} ({n})")).find(|c| !taken.contains(c)).unwrap()
}

/// Copy of `v` with every object id inside it replaced by `fresh(..)`; references
/// between objects of the copy follow, references to other objects stay.
fn remap(v: &Value, map: &HashMap<u64, u64>) -> Value {
    match v {
        Value::Ref(o) => Value::Ref(*map.get(o).unwrap_or(o)),
        Value::List(items) => Value::List(items.iter().map(|i| remap(i, map)).collect()),
        Value::Obj(o) => Value::Obj(Obj {
            type_name: o.type_name.clone(),
            version: o.version,
            oid: o.oid.map(|x| *map.get(&x).unwrap_or(&x)),
            export: o.export.clone(),
            fields: o.fields.iter().map(|(n, v)| (n.clone(), remap(v, map))).collect(),
        }),
        v => v.clone(),
    }
}

/// For each `id` in `candidates`, finds whether it still has a definition in
/// `root` and records the archive-order path of its first reference in `root`.
fn scan_candidates(root: &Value, candidates: &HashSet<u64>) -> (Vec<u64>, HashMap<u64, Path>) {
    fn walk(v: &Value, path: &mut Vec<u32>, candidates: &HashSet<u64>, defined: &mut HashSet<u64>, first_refs: &mut HashMap<u64, Path>) {
        match v {
            Value::Ref(o) if candidates.contains(o) => {
                first_refs.entry(*o).or_insert_with(|| path.clone());
            }
            Value::List(items) => {
                for (i, item) in items.iter().enumerate() {
                    path.push(i as u32);
                    walk(item, path, candidates, defined, first_refs);
                    path.pop();
                }
            }
            Value::Obj(o) => {
                if let Some(id) = o.oid
                    && candidates.contains(&id)
                {
                    defined.insert(id);
                }
                for (i, (_, f)) in o.fields.iter().enumerate() {
                    path.push(i as u32);
                    walk(f, path, candidates, defined, first_refs);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    if candidates.is_empty() {
        return (Vec::new(), HashMap::new());
    }
    let mut defined = HashSet::new();
    let mut first_refs = HashMap::new();
    let mut path = Vec::with_capacity(32);
    walk(root, &mut path, candidates, &mut defined, &mut first_refs);
    let bad: Vec<u64> = first_refs.keys().copied().filter(|id| !defined.contains(id)).collect();
    (bad, first_refs)
}

impl Document {
    /// Path of a chain of fields below the object at `from`.
    pub(crate) fn field_path(&self, from: &[u32], names: &[&str]) -> Result<Path, String> {
        let mut path = from.to_vec();
        for name in names {
            let Some(Value::Obj(o)) = self.tree.at(&path) else {
                return Err(format!("expected an object before {name}"));
            };
            path.extend(o.find_path(name).ok_or_else(|| format!("no {name} field"))?);
        }
        Ok(path)
    }

    fn lists(&self) -> Result<Lists, String> {
        let song = self.addresses.song.clone().ok_or("the project has no song")?;
        Ok(Lists {
            scenes: self.field_path(&song, &["scenes", "objects", "items"])?,
            scenes_current: self.field_path(&song, &["scenes", "current"])?,
            scene_selection: self.field_path(&song, &["scenes", "selection", "items"]).ok(),
            sections: self.field_path(&song, &["sections", "objects", "items"])?,
            sections_current: self.field_path(&song, &["sections", "current"])?,
        })
    }

    pub(crate) fn items(&self, path: &Path) -> Result<Vec<Value>, String> {
        match self.tree.at(path) {
            Some(Value::List(l)) => Ok(l.clone()),
            _ => Err("expected a list".into()),
        }
    }

    pub(crate) fn oid_at(&self, path: &Path) -> Result<u64, String> {
        match self.tree.at(path) {
            Some(Value::Obj(o)) => o.oid.ok_or_else(|| "object has no id".into()),
            _ => Err("expected an object".into()),
        }
    }

    pub(crate) fn fresh_copy(&mut self, v: &Value) -> Value {
        let mut map = HashMap::new();
        v.walk(&mut |o| {
            if let Some(id) = o.oid {
                map.entry(id).or_insert(0);
            }
        });
        let mut ids: Vec<u64> = map.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            map.insert(id, self.next_oid);
            self.next_oid += 1;
        }
        remap(v, &map)
    }

    /// `ops`, made safe to apply: refused (the document left as it was) if they would
    /// leave a reference to a removed object.
    ///
    /// Archives refer back to an object by id after its first occurrence. If an
    /// edit removes the occurrence that defines an object still referenced later
    /// (Maschine reuses addresses, so even node markers are shared this way), the
    /// object moves to its next reference instead, as saving would place it. A side-chain
    /// is such a place: a project read back has a group defined in the first side-chain
    /// listening to it.
    #[cfg(test)]
    pub(crate) fn rehomed(&mut self, label: &str, mut ops: Vec<Op>) -> Result<Vec<Op>, String> {
        let initial_undo = self.run(&ops)?;
        let lost = crate::lost_referenced_ids(&initial_undo, &ops, self.references.as_ref().unwrap());
        if lost.is_empty() {
            self.run(&initial_undo).expect("undo of ops just applied");
            return Ok(ops);
        }
        match self.rehome_applied(label, &initial_undo, &ops, lost) {
            Ok((extra_ops, extra_undo)) => {
                if !extra_undo.is_empty() {
                    self.run(&extra_undo).expect("undo of rehome ops just applied");
                }
                self.run(&initial_undo).expect("undo of ops just applied");
                ops.extend(extra_ops);
                Ok(ops)
            }
            Err(e) => {
                self.run(&initial_undo).expect("undo of ops just applied");
                Err(e)
            }
        }
    }

    /// With `ops` already applied (producing `undo`), incrementally move any removed
    /// definitions that are still referenced in `self.tree` to their first remaining
    /// reference. Returns `(extra_ops, extra_undo)` in apply / undo order.
    pub(crate) fn rehome_applied(&mut self, label: &str, undo: &[Op], ops: &[Op], lost: Vec<u64>) -> Result<(Vec<Op>, Vec<Op>), String> {
        fn ids_in(v: &Value, out: &mut HashSet<u64>) {
            match v {
                Value::Obj(o) => {
                    out.extend(o.oid);
                    o.fields.iter().for_each(|(_, f)| ids_in(f, out));
                }
                Value::List(items) => items.iter().for_each(|i| ids_in(i, out)),
                _ => {}
            }
        }
        fn collect_removed<'a>(undo: &'a [Op], ops: &[Op]) -> HashMap<u64, &'a Obj> {
            let mut removed: HashMap<u64, &'a Obj> = HashMap::new();
            for op in undo.iter().rev() {
                if let Some(v) = crate::op_value(op) {
                    v.walk(&mut |o| {
                        if let Some(id) = o.oid {
                            removed.entry(id).or_insert(o);
                        }
                    });
                }
            }
            for op in ops {
                if let Some(v) = crate::op_value(op) {
                    v.walk(&mut |o| {
                        if let Some(id) = o.oid {
                            removed.remove(&id);
                        }
                    });
                }
            }
            removed
        }

        let mut extra_ops = Vec::new();
        let mut extra_undo = Vec::new();
        let mut removed: Option<HashMap<u64, &Obj>> = None;
        let mut candidates: HashSet<u64> = lost.into_iter().collect();

        for _ in 0..4 {
            if candidates.is_empty() {
                return Ok((extra_ops, extra_undo));
            }
            let (bad, first_refs) = scan_candidates(&self.tree, &candidates);
            if bad.is_empty() {
                return Ok((extra_ops, extra_undo));
            }
            let removed_map = removed.get_or_insert_with(|| collect_removed(undo, ops));
            // Only the outermost: what's defined inside a moved object moves with it (a
            // group carries its patterns).
            let mut inner = HashSet::new();
            for id in &bad {
                if let Some(&o) = removed_map.get(id) {
                    o.fields.iter().for_each(|(_, f)| ids_in(f, &mut inner));
                }
            }
            let mut step_ops = Vec::new();
            let mut moved_ids = HashSet::new();
            for id in bad.iter().filter(|id| !inner.contains(id)) {
                let (Some(path), Some(&obj)) = (first_refs.get(id), removed_map.get(id)) else {
                    if !extra_undo.is_empty() {
                        self.run(&extra_undo).expect("undo of rehome ops just applied");
                    }
                    return Err(format!("{label}: object #{id} would still be referenced after the edit"));
                };
                moved_ids.insert(*id);
                obj.fields.iter().for_each(|(_, f)| ids_in(f, &mut moved_ids));
                step_ops.push(Op::Set { path: path.clone(), value: Value::Obj(obj.clone()) });
            }
            let step_undo = match self.run(&step_ops) {
                Ok(u) => u,
                Err(e) => {
                    if !extra_undo.is_empty() {
                        self.run(&extra_undo).expect("undo of rehome ops just applied");
                    }
                    return Err(e);
                }
            };
            for id in moved_ids {
                removed_map.remove(&id);
            }
            let mut next_undo = step_undo;
            next_undo.extend(extra_undo);
            extra_undo = next_undo;
            extra_ops.extend(step_ops);

            let refs = self.references.as_ref().unwrap();
            candidates = removed_map.keys().copied().filter(|id| refs.get(id).copied().unwrap_or(0) > 0).collect();
        }
        if !extra_undo.is_empty() {
            self.run(&extra_undo).expect("undo of rehome ops just applied");
        }
        Err(format!("{label}: references could not be kept consistent"))
    }

    /// The scene and section objects' ids, in list order.
    fn scene_oids(&self, l: &Lists) -> Result<Vec<u64>, String> {
        self.items(&l.scenes)?.iter().map(|i| px(i).and_then(|o| o.oid).ok_or_else(|| "scene without an id".into())).collect()
    }

    fn section_scene_oid(item: &Value) -> Option<u64> {
        match px(item)?.get("scene") {
            Some(Value::Ref(o)) => Some(*o),
            Some(Value::Obj(o)) => o.oid,
            _ => None,
        }
    }

    // ---- scenes ----------------------------------------------------------------------

    /// Which pattern `group` plays in `scene` (`None` empties the slot).
    pub fn assign_pattern(&mut self, scene: usize, group: usize, pattern: Option<usize>) -> Result<(), String> {
        let l = self.lists()?;
        let ga = self.addresses.groups.get(group).ok_or("no such group")?.clone();
        let group_oid = self.oid_at(&ga.group)?;
        let target = match pattern {
            Some(pi) => Value::Ref(self.oid_at(ga.patterns.get(pi).ok_or("no such pattern")?)?),
            None => Value::Null,
        };
        if scene >= self.items(&l.scenes)?.len() {
            return Err("no such scene".into());
        }
        let mut item = [l.scenes.as_slice(), &[scene as u32]].concat();
        let Some(Value::Obj(holder)) = self.tree.at(&item) else { return Err("scene item".into()) };
        item.extend(holder.find_path("px").ok_or("scene item without px")?);
        let entries_path = self.field_path(&item, &["patterns", "objects", "items"])?;
        let mut entries = self.items(&entries_path)?;
        let is_group = |e: &Value| match e {
            Value::Obj(o) => match o.get("first") {
                Some(Value::Ref(r)) => *r == group_oid,
                Some(Value::Obj(g)) => g.oid == Some(group_oid),
                _ => false,
            },
            _ => false,
        };
        match entries.iter().position(is_group) {
            Some(i) => {
                let pr = px_mut(&mut entries[i]).ok_or("scene entry without a PatternRef")?;
                *pr.get_mut("pattern").ok_or("PatternRef without pattern")? = target;
            }
            None => {
                // A group the scene has no slot for yet: copy another slot, or make one.
                let mut entry = match entries.first().cloned() {
                    Some(t) => self.fresh_copy(&t),
                    None => self.build(|b| b.scene_entry(group_oid))?,
                };
                if let Value::Obj(o) = &mut entry
                    && let Some(f) = o.get_mut("first")
                {
                    *f = Value::Ref(group_oid);
                }
                *px_mut(&mut entry).and_then(|p| p.get_mut("pattern")).ok_or("PatternRef without pattern")? = target;
                entries.push(entry);
            }
        }
        self.apply_with(
            "Assign Pattern",
            vec![Op::Set { path: entries_path, value: Value::List(entries) }],
            vec![Some((crate::Target::Scene(scene), "__patterns".into()))],
        )
    }

    /// Re-read scene `scene`'s pattern assignments in place without decoding the whole
    /// project tree (so live pattern switches and mutes/drops stay sub-millisecond).
    pub(crate) fn patch_scene_patterns(&mut self, scene: usize) -> bool {
        if scene >= self.project.scenes.len() {
            return false;
        }
        let mut group_by_oid = std::collections::HashMap::with_capacity(self.addresses.groups.len());
        let mut pattern_by_oid = std::collections::HashMap::new();
        for (g, ga) in self.addresses.groups.iter().enumerate() {
            let Ok(g_oid) = self.oid_at(&ga.group) else { return false };
            group_by_oid.insert(g_oid, g);
            for (pi, pa) in ga.patterns.iter().enumerate() {
                let Ok(p_oid) = self.oid_at(pa) else { return false };
                pattern_by_oid.insert(p_oid, (g, pi));
            }
        }
        let Some(scene_path) = self.addresses.scenes.get(scene) else { return false };
        let Some(s) = self.obj_by_path(scene_path) else { return false };
        let entries: &[Value] = match s.get("patterns") {
            Some(Value::Obj(m)) => match m.get("objects") {
                Some(Value::Obj(o)) => match o.get("items") {
                    Some(Value::List(items)) => items,
                    _ => return false,
                },
                _ => return false,
            },
            None => &[],
            _ => return false,
        };
        let mut patterns = Vec::with_capacity(entries.len());
        for entry in entries {
            let Value::Obj(eo) = entry else { return false };
            let g_oid = match eo.get("first") {
                Some(Value::Ref(r)) => *r,
                Some(Value::Obj(o)) => match o.oid {
                    Some(id) => id,
                    None => return false,
                },
                _ => continue,
            };
            if !group_by_oid.contains_key(&g_oid) {
                return false;
            }
            let pref = match eo.get("second") {
                Some(Value::Obj(sp)) => match sp.get("px") {
                    Some(Value::Obj(px)) => px,
                    _ => return false,
                },
                _ => return false,
            };
            let p_oid = match pref.get("pattern") {
                Some(Value::Null) | None => continue,
                Some(Value::Ref(r)) => *r,
                Some(Value::Obj(o)) => match o.oid {
                    Some(id) => id,
                    None => return false,
                },
                _ => return false,
            };
            let Some(&(pg, pi)) = pattern_by_oid.get(&p_oid) else { return false };
            patterns.push((pg, pi));
        }
        patterns.sort_unstable();
        self.project.scenes[scene].patterns = patterns;
        let len = self.project.scene_length(scene);
        for sec in &mut self.project.sections {
            if sec.auto && sec.scene == Some(scene) {
                sec.length = len;
            }
        }
        true
    }

    /// A new empty pattern in `group` named `name`, after the group's other patterns,
    /// with pattern `from`'s length and colour. Returns its index.
    pub fn add_pattern(&mut self, group: usize, from: usize, name: &str) -> Result<usize, String> {
        let n = self.project.groups.get(group).ok_or("no such group")?.patterns.len();
        self.insert_pattern(group, from, n, name)
    }

    /// [`Document::add_pattern`] at position `at` of the group's patterns: the ones
    /// from there on move one place on (their ids too). Returns its index.
    pub fn insert_pattern(&mut self, group: usize, from: usize, at: usize, name: &str) -> Result<usize, String> {
        let items_path = self.pattern_items_path(group)?;
        let mut items = self.items(&items_path)?;
        let mut entry = self.new_pattern_entry(group, from, &items, name)?;
        let at = at.min(items.len());
        // Its id: the next after the pattern before it; later ids move up one.
        let id = if at == 0 {
            items.first().and_then(super::sounds::entry_id).unwrap_or(0)
        } else {
            items[at - 1..at].iter().filter_map(super::sounds::entry_id).next().map_or(0, |i| i + 1)
        };
        for e in items.iter_mut() {
            if let Some(i) = super::sounds::entry_id(e)
                && i >= id
            {
                super::sounds::set_entry_id(e, i + 1);
            }
        }
        super::sounds::set_entry_id(&mut entry, id);
        items.insert(at, entry);
        self.apply("New Pattern", vec![Op::Set { path: items_path, value: Value::List(items) }])?;
        Ok(at)
    }

    /// [`Document::add_pattern`] in empty place `slot` of the group (from 0, whether or not
    /// the places before it have patterns): the other patterns stay where they are. Returns
    /// its index in the group's list.
    pub fn add_pattern_in_slot(&mut self, group: usize, from: usize, slot: usize, name: &str) -> Result<usize, String> {
        let items_path = self.pattern_items_path(group)?;
        let mut items = self.items(&items_path)?;
        let id = slot as i64;
        if items.iter().filter_map(super::sounds::entry_id).any(|i| i == id) {
            return Err("there's a pattern in that place".into());
        }
        let mut entry = self.new_pattern_entry(group, from, &items, name)?;
        super::sounds::set_entry_id(&mut entry, id);
        let at = items.iter().position(|e| super::sounds::entry_id(e).is_some_and(|i| i > id)).unwrap_or(items.len());
        items.insert(at, entry);
        self.apply("New Pattern", vec![Op::Set { path: items_path, value: Value::List(items) }])?;
        Ok(at)
    }

    /// Move pattern `p` of `group` to place `slot` (from 0). A pattern already there takes
    /// `p`'s old place. Scenes keep playing the same patterns. Returns `p`'s new index in
    /// the group's list.
    pub fn move_pattern(&mut self, group: usize, p: usize, slot: usize) -> Result<usize, String> {
        let items_path = self.pattern_items_path(group)?;
        let mut items = self.items(&items_path)?;
        let from = items.get(p).and_then(super::sounds::entry_id).ok_or("no such pattern")?;
        let to = slot as i64;
        if from == to {
            return Ok(p);
        }
        if let Some(other) = items.iter().position(|e| super::sounds::entry_id(e) == Some(to)) {
            super::sounds::set_entry_id(&mut items[other], from);
        }
        super::sounds::set_entry_id(&mut items[p], to);
        // The list stays in place order.
        items.sort_by_key(|e| super::sounds::entry_id(e).unwrap_or(i64::MAX));
        let at = items.iter().position(|e| super::sounds::entry_id(e) == Some(to)).ok_or("pattern lost")?;
        self.apply("Move Pattern", vec![Op::Set { path: items_path, value: Value::List(items) }])?;
        Ok(at)
    }

    /// A new pattern's entry for group `group`, whose list is `items`: empty, like pattern
    /// `from`. Its id is still to be set.
    fn new_pattern_entry(&mut self, group: usize, from: usize, items: &[Value], name: &str) -> Result<Value, String> {
        let mut entry = match items.get(from).cloned() {
            Some(t) => self.fresh_copy(&t),
            // The group's first pattern: a bar long, a slot for each sound.
            None if items.is_empty() => {
                let sounds = self.addresses.groups[group].sounds.iter().map(|p| self.oid_at(p)).collect::<Result<Vec<_>, _>>()?;
                let bar = self.project.bar_length();
                self.build(|b| b.pattern_entry(0, name, bar, &sounds))?
            }
            None => return Err("no such pattern".into()),
        };
        let Value::Obj(o) = &mut entry else { return Err("bad pattern entry".into()) };
        let pat = match o.get_mut("second") {
            Some(Value::Obj(sp)) => match sp.get_mut("px") {
                Some(Value::Obj(p)) => p,
                _ => return Err("pattern entry without a pattern".into()),
            },
            _ => return Err("pattern entry without a pattern".into()),
        };
        set_param(pat, "start", Value::Int(0));
        if let Some(Value::Obj(seq)) = pat.get_mut("sequence") {
            if let Some(c) = seq.get_mut("current") {
                *c = Value::Null;
            }
            if let Some(Value::Obj(ev)) = seq.get_mut("events") {
                *ev = crate::notes::events_with(ev, &[]);
            }
        }
        // No notes: each sound's sequence lives in the sound and would be shared.
        if let Some(Value::Obj(ss)) = pat.get_mut("sound_sequences")
            && let Some(Value::Obj(objs)) = ss.get_mut("objects")
            && let Some(Value::List(entries)) = objs.get_mut("items")
        {
            for e in entries {
                match e {
                    Value::Obj(o) => match o.get_mut("reference") {
                        Some(Value::Obj(r)) => match r.get_mut("px") {
                            Some(Value::Obj(sr)) => {
                                if let Some(seq) = sr.get_mut("sequence") {
                                    *seq = Value::Null;
                                }
                            }
                            _ => return Err("a sequence reference defined elsewhere".into()),
                        },
                        _ => return Err("a pattern entry without a reference".into()),
                    },
                    _ => return Err("bad pattern entry".into()),
                }
            }
        }
        if let Some(Value::Obj(n)) = pat.get_mut("name") {
            for k in ["value", "t0"] {
                if let Some(v @ Value::Str(_)) = n.find_mut(k) {
                    *v = Value::Str(name.to_string());
                }
            }
        }
        Ok(entry)
    }

    /// Take pattern `p` out of its group's list. The others keep their places, leaving a gap.
    /// Scenes must not refer to it any more.
    pub(crate) fn remove_pattern_entry(&mut self, group: usize, p: usize) -> Result<(), String> {
        let items_path = self.pattern_items_path(group)?;
        let mut items = self.items(&items_path)?;
        if p >= items.len() {
            return Err("no such pattern".into());
        }
        items.remove(p);
        self.apply("Delete Pattern", vec![Op::Set { path: items_path, value: Value::List(items) }])
    }

    /// A new empty scene at the end; returns its index.
    pub fn add_scene(&mut self) -> Result<usize, String> {
        self.insert_scene(usize::MAX)
    }

    /// A new empty scene at position `at` (clamped to the end); returns its index.
    pub fn insert_scene(&mut self, at: usize) -> Result<usize, String> {
        let l = self.lists()?;
        let mut scenes = self.items(&l.scenes)?;
        let mut item = match scenes.first().cloned() {
            Some(t) => self.fresh_copy(&t),
            None => {
                let groups = self.addresses.groups.iter().map(|g| self.oid_at(&g.group)).collect::<Result<Vec<_>, _>>()?;
                self.build(|b| {
                    let sc = b.scene("", &groups)?;
                    Ok(b.shared("NI::MASCHINE::DATA::Scene", sc))
                })?
            }
        };
        let n = scenes.len();
        {
            let s = px_mut(&mut item).ok_or("scene item")?;
            set_param(s, "name", Value::Str(format!("Scene {}", n + 1)));
            // (Made from the first scene, not with its own tempo.)
            set_param(s, "scene_tempo_active", Value::Int(0));
            s.fields.retain(|(f, _)| f != deckard_model::SCENE_TEMPO_FIELD);
            if let Some(Value::Obj(c)) = s.get_mut("color") {
                for (k, v) in [("color", 0), ("use_default_color", 1)] {
                    if let Some(f) = c.get_mut(k) {
                        *f = Value::Int(v);
                    }
                }
            }
            if let Some(Value::Obj(pm)) = s.get_mut("patterns")
                && let Some(Value::Obj(objs)) = pm.get_mut("objects")
                && let Some(Value::List(entries)) = objs.get_mut("items")
            {
                for e in entries {
                    if let Some(p) = px_mut(e).and_then(|p| p.get_mut("pattern")) {
                        *p = Value::Null;
                    }
                }
            }
        }
        let oid = px(&item).and_then(|o| o.oid).ok_or("scene id")?;
        let at = at.min(n);
        scenes.insert(at, item);
        let ops = vec![Op::Set { path: l.scenes, value: Value::List(scenes) }, Op::Set { path: l.scenes_current, value: Value::Ref(oid) }];
        self.apply("Add Scene", ops)?;
        Ok(at)
    }

    /// Move scene `from` to position `to` (its sections keep playing it).
    pub fn move_scene(&mut self, from: usize, to: usize) -> Result<(), String> {
        let l = self.lists()?;
        let mut scenes = self.items(&l.scenes)?;
        if from >= scenes.len() || to >= scenes.len() {
            return Err("no such scene".into());
        }
        if from == to {
            return Ok(());
        }
        let s = scenes.remove(from);
        scenes.insert(to, s);
        self.apply("Move Scene", vec![Op::Set { path: l.scenes, value: Value::List(scenes) }])
    }

    /// Copy `scene` to just after it, as `<name> (n)`; returns the copy's index.
    pub fn duplicate_scene(&mut self, scene: usize) -> Result<usize, String> {
        let l = self.lists()?;
        let mut scenes = self.items(&l.scenes)?;
        let original = scenes.get(scene).ok_or("no such scene")?.clone();
        let mut item = self.fresh_copy(&original);
        let names: Vec<String> = scenes.iter().filter_map(px).map(|o| param_str(o, "name")).collect();
        let s = px_mut(&mut item).ok_or("scene item")?;
        let name = copy_name(&param_str(s, "name"), &names);
        set_param(s, "name", Value::Str(name));
        let oid = s.oid.ok_or("scene id")?;
        scenes.insert(scene + 1, item);
        let ops = vec![Op::Set { path: l.scenes, value: Value::List(scenes) }, Op::Set { path: l.scenes_current, value: Value::Ref(oid) }];
        self.apply("Duplicate Scene", ops)?;
        Ok(scene + 1)
    }

    /// Delete `scene` and the sections that play it. The last scene stays: a project
    /// always has one.
    pub fn delete_scene(&mut self, scene: usize) -> Result<(), String> {
        let l = self.lists()?;
        let mut scenes = self.items(&l.scenes)?;
        if scene >= scenes.len() {
            return Err("no such scene".into());
        }
        if scenes.len() == 1 {
            return Err("a project keeps at least one scene".into());
        }
        let oid = px(&scenes[scene]).and_then(|o| o.oid).ok_or("scene id")?;
        scenes.remove(scene);
        let mut sections = self.items(&l.sections)?;
        sections.retain(|s| Self::section_scene_oid(s) != Some(oid));
        let mut ops = vec![
            Op::Set { path: l.scenes.clone(), value: Value::List(scenes.clone()) },
            Op::Set { path: l.sections.clone(), value: Value::List(sections.clone()) },
        ];
        ops.extend(self.fix_current(&l.scenes_current, &scenes, Some(scene.min(scenes.len().saturating_sub(1)))));
        ops.extend(self.fix_current(&l.sections_current, &sections, None));
        if let Some(sel) = &l.scene_selection {
            let mut items = self.items(sel)?;
            items.retain(|v| !matches!(v, Value::Ref(o) if *o == oid));
            ops.push(Op::Set { path: sel.clone(), value: Value::List(items) });
        }
        self.apply("Delete Scene", ops)
    }

    /// If a list's `current` pointer refers to an item no longer in `items`, point it
    /// at item `prefer` (or the first), or null if the list is empty.
    pub(crate) fn fix_current(&self, current: &Path, items: &[Value], prefer: Option<usize>) -> Option<Op> {
        let ids: Vec<u64> = items.iter().filter_map(|i| px(i).and_then(|o| o.oid)).collect();
        match self.tree.at(current) {
            Some(Value::Ref(o)) if ids.contains(o) => None,
            Some(Value::Null) if ids.is_empty() => None,
            _ => {
                let pick = prefer.and_then(|i| ids.get(i)).or(ids.first());
                Some(Op::Set { path: current.clone(), value: pick.map_or(Value::Null, |o| Value::Ref(*o)) })
            }
        }
    }

    // ---- sections ----------------------------------------------------------------------

    fn renumber(sections: &mut [Value]) {
        for (k, s) in sections.iter_mut().enumerate() {
            if let Value::Obj(o) = s
                && let Some(f) = o.get_mut("first")
            {
                *f = Value::Int(k as i64);
            }
        }
    }

    /// Duplicate `section` (and, as Maschine 3 does, its scene, appended to the scene
    /// list); the copy goes right after it. Returns the copy's index.
    pub fn duplicate_section(&mut self, section: usize) -> Result<usize, String> {
        let l = self.lists()?;
        let mut sections = self.items(&l.sections)?;
        let original = sections.get(section).ok_or("no such section")?.clone();
        let scene_oid = Self::section_scene_oid(&original).ok_or("section without a scene")?;
        let mut scenes = self.items(&l.scenes)?;
        let k = self.scene_oids(&l)?.iter().position(|o| *o == scene_oid).ok_or("section plays an unknown scene")?;
        let names: Vec<String> = scenes.iter().filter_map(px).map(|o| param_str(o, "name")).collect();
        let mut scene = self.fresh_copy(&scenes[k]);
        let sp = px_mut(&mut scene).ok_or("scene item")?;
        let name = copy_name(&param_str(sp, "name"), &names);
        set_param(sp, "name", Value::Str(name));
        let new_scene = sp.oid.ok_or("scene id")?;
        scenes.push(scene);

        let mut copy = self.fresh_copy(&original);
        let cp = px_mut(&mut copy).ok_or("section item")?;
        *cp.get_mut("scene").ok_or("section without scene")? = Value::Ref(new_scene);
        let new_section = cp.oid.ok_or("section id")?;
        sections.insert(section + 1, copy);
        Self::renumber(&mut sections);
        let ops = vec![
            Op::Set { path: l.scenes, value: Value::List(scenes) },
            Op::Set { path: l.scenes_current, value: Value::Ref(new_scene) },
            Op::Set { path: l.sections, value: Value::List(sections) },
            Op::Set { path: l.sections_current, value: Value::Ref(new_section) },
        ];
        self.apply("Duplicate Section", ops)?;
        Ok(section + 1)
    }

    /// [`Document::duplicate_scene`], with `patterns` the copy playing copies of the
    /// scene's patterns (Maschine's Duplicate setting "Scene and Patterns").
    pub fn duplicate_scene_with(&mut self, scene: usize, patterns: bool) -> Result<usize, String> {
        if !patterns {
            return self.duplicate_scene(scene);
        }
        let mut copy = 0;
        self.transaction("Duplicate Scene", |d| {
            copy = d.duplicate_scene(scene)?;
            d.copy_scene_patterns(copy)
        })?;
        Ok(copy)
    }

    /// [`Document::duplicate_section`], with `link` the copy playing the same scene (a
    /// linked copy: Maschine's Link when duplicating Sections), else a copy of it, with
    /// `patterns` playing copies of its patterns.
    pub fn duplicate_section_with(&mut self, section: usize, link: bool, patterns: bool) -> Result<usize, String> {
        if link {
            return self.link_section(section);
        }
        let mut copy = 0;
        self.transaction("Duplicate Section", |d| {
            copy = d.duplicate_section(section)?;
            if patterns {
                let scene = d.project.sections[copy].scene.ok_or("section without a scene")?;
                d.copy_scene_patterns(scene)?;
            }
            Ok(())
        })?;
        Ok(copy)
    }

    /// Scene `to` plays what scene `from` plays, group by group; past the last scene, a new
    /// one goes at the end. Returns its index. One undo step.
    pub fn copy_scene(&mut self, from: usize, to: usize) -> Result<usize, String> {
        let source = self.project.scenes.get(from).ok_or("no such scene")?.patterns.clone();
        let groups = self.project.groups.len();
        let mut at = to;
        self.transaction("Duplicate Scene", |d| {
            if to >= d.project.scenes.len() {
                at = d.add_scene()?;
            }
            for g in 0..groups {
                let pattern = source.iter().find(|(x, _)| *x == g).map(|&(_, p)| p);
                d.assign_pattern(at, g, pattern)?;
            }
            Ok(())
        })?;
        Ok(at)
    }

    /// Scene `scene` plays copies of its patterns.
    fn copy_scene_patterns(&mut self, scene: usize) -> Result<(), String> {
        let groups: Vec<usize> = self.project.scenes.get(scene).ok_or("no such scene")?.patterns.iter().map(|&(g, _)| g).collect();
        for g in groups {
            let Some(&(_, p)) = self.project.scenes[scene].patterns.iter().find(|(x, _)| *x == g) else { continue };
            let copy = self.duplicate_pattern(g, p)?;
            self.assign_pattern(scene, g, Some(copy))?;
        }
        Ok(())
    }

    /// A copy of `section` right after it, playing the same scene. Returns its index.
    fn link_section(&mut self, section: usize) -> Result<usize, String> {
        let l = self.lists()?;
        let mut sections = self.items(&l.sections)?;
        let original = sections.get(section).ok_or("no such section")?.clone();
        let mut copy = self.fresh_copy(&original);
        let oid = px_mut(&mut copy).and_then(|o| o.oid).ok_or("section id")?;
        sections.insert(section + 1, copy);
        Self::renumber(&mut sections);
        let ops =
            vec![Op::Set { path: l.sections, value: Value::List(sections) }, Op::Set { path: l.sections_current, value: Value::Ref(oid) }];
        self.apply("Duplicate Section", ops)?;
        Ok(section + 1)
    }

    /// A new section at the end of the arrangement playing `scene` (auto length);
    /// returns its index.
    pub fn add_section(&mut self, scene: usize) -> Result<usize, String> {
        let l = self.lists()?;
        let scene_oid = *self.scene_oids(&l)?.get(scene).ok_or("no such scene")?;
        let mut sections = self.items(&l.sections)?;
        let mut copy = match sections.last().cloned() {
            Some(t) => self.fresh_copy(&t),
            None => self.build(|b| b.section_entry(0, scene_oid))?,
        };
        let cp = px_mut(&mut copy).ok_or("section item")?;
        *cp.get_mut("scene").ok_or("section without scene")? = Value::Ref(scene_oid);
        set_param(cp, "auto_length", Value::Int(1));
        let oid = cp.oid.ok_or("section id")?;
        let key =
            sections.iter().filter_map(|s| if let Value::Obj(o) = s { o.get("first")?.as_int() } else { None }).max().map_or(0, |k| k + 1);
        if let Value::Obj(o) = &mut copy
            && let Some(f) = o.get_mut("first")
        {
            *f = Value::Int(key);
        }
        let n = sections.len();
        sections.push(copy);
        let ops =
            vec![Op::Set { path: l.sections, value: Value::List(sections) }, Op::Set { path: l.sections_current, value: Value::Ref(oid) }];
        self.apply("Add Section", ops)?;
        Ok(n)
    }

    pub fn delete_section(&mut self, section: usize) -> Result<(), String> {
        let l = self.lists()?;
        let mut sections = self.items(&l.sections)?;
        if section >= sections.len() {
            return Err("no such section".into());
        }
        sections.remove(section);
        let mut ops = vec![Op::Set { path: l.sections.clone(), value: Value::List(sections.clone()) }];
        ops.extend(self.fix_current(&l.sections_current, &sections, Some(section.min(sections.len().saturating_sub(1)))));
        self.apply("Delete Section", ops)
    }

    /// Move a section to position `to` in the arrangement.
    pub fn move_section(&mut self, from: usize, to: usize) -> Result<(), String> {
        let l = self.lists()?;
        let mut sections = self.items(&l.sections)?;
        if from >= sections.len() || to >= sections.len() {
            return Err("no such section".into());
        }
        if from == to {
            return Ok(());
        }
        let s = sections.remove(from);
        sections.insert(to, s);
        Self::renumber(&mut sections);
        self.apply("Move Section", vec![Op::Set { path: l.sections, value: Value::List(sections) }])
    }

    /// Which scene a section plays.
    pub fn set_section_scene(&mut self, section: usize, scene: usize) -> Result<(), String> {
        let l = self.lists()?;
        let scene_oid = *self.scene_oids(&l)?.get(scene).ok_or("no such scene")?;
        if section >= self.items(&l.sections)?.len() {
            return Err("no such section".into());
        }
        let mut item = [l.sections.as_slice(), &[section as u32]].concat();
        item = self.field_path(&item, &["second", "px", "scene"])?;
        self.apply("Section Scene", vec![Op::Set { path: item, value: Value::Ref(scene_oid) }])
    }

    /// A section's length in ticks, or `None` to follow its scene (auto length).
    pub fn set_section_length(&mut self, section: usize, ticks: Option<i64>) -> Result<(), String> {
        let t = Target::Section(section);
        let mut ops = Vec::new();
        if let Some(ticks) = ticks {
            ops.extend(self.param_ops(t, "length", Value::Int(ticks.max(1)))?);
        }
        ops.extend(self.param_ops(t, "auto_length", Value::Int(ticks.is_none() as i64))?);
        if ops.is_empty() {
            return Ok(());
        }
        self.apply("Section Length", ops)
    }
}

#[cfg(test)]
mod slot_tests {
    use crate::Document;

    /// A pattern goes in the place asked for, and stays there: a gap is what the file has.
    #[test]
    fn a_pattern_can_go_in_any_empty_place() {
        let mut d = Document::new_project().unwrap();
        let slots = |d: &Document| d.project().groups[0].patterns.iter().map(|p| p.slot).collect::<Vec<_>>();
        d.add_pattern(0, 0, "One").unwrap();
        assert_eq!(slots(&d), [0]);
        assert_eq!(d.add_pattern_in_slot(0, 0, 7, "Eight").unwrap(), 1, "listed after the first");
        assert_eq!(slots(&d), [0, 7], "with places left empty before it");
        assert_eq!(d.add_pattern_in_slot(0, 0, 3, "Four").unwrap(), 1, "listed in order");
        assert_eq!(slots(&d), [0, 3, 7], "the others stay where they are");
        assert_eq!(d.project().groups[0].patterns[2].name, "Eight");
        assert!(d.add_pattern_in_slot(0, 0, 3, "Again").is_err(), "that place is taken");
        // A scene goes on playing what it played.
        d.assign_pattern(0, 0, Some(2)).unwrap();
        d.add_pattern_in_slot(0, 0, 1, "Two").unwrap();
        assert_eq!(d.project().scenes[0].patterns, [(0, 3)], "\"Eight\", now further down the list");
        // Saved and opened again, the gaps are still there.
        let again = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(slots(&again), [0, 1, 3, 7]);
        assert_eq!(again.project().scenes[0].patterns, [(0, 3)]);
        // And after them, a pattern added the old way goes after the last.
        d.add_pattern(0, 0, "Nine").unwrap();
        assert_eq!(slots(&d), [0, 1, 3, 7, 8]);
    }

    /// Deleting a pattern leaves a gap: the others stay where they are, and it can be filled.
    #[test]
    fn deleting_a_pattern_leaves_its_place_empty() {
        let mut d = Document::new_project().unwrap();
        let slots = |d: &Document| d.project().groups[0].patterns.iter().map(|p| p.slot).collect::<Vec<_>>();
        for name in ["One", "Two", "Three"] {
            d.add_pattern(0, 0, name).unwrap();
        }
        d.assign_pattern(0, 0, Some(2)).unwrap();
        d.delete_pattern(0, 1).unwrap();
        assert_eq!(slots(&d), [0, 2]);
        assert_eq!(d.project().groups[0].patterns[1].name, "Three");
        assert_eq!(d.project().scenes[0].patterns, [(0, 1)], "the scene plays what it played");
        assert_eq!(d.add_pattern_in_slot(0, 0, 1, "Again").unwrap(), 1);
        assert_eq!(slots(&d), [0, 1, 2]);
        d.undo();
        d.undo();
        assert_eq!(slots(&d), [0, 1, 2], "undone, the one deleted is back");
        assert_eq!(d.project().groups[0].patterns[1].name, "Two");
    }

    /// The first pattern of an empty group can go anywhere too.
    #[test]
    fn a_groups_first_pattern_can_go_in_any_place() {
        let mut d = Document::new_project().unwrap();
        assert_eq!(d.add_pattern_in_slot(0, 0, 5, "Six").unwrap(), 0);
        assert_eq!(d.project().groups[0].patterns[0].slot, 5);
    }

    /// Scene pattern switches and pattern length/start edits patch the model in place
    /// (keeping existing plug-in chunks untouched) and match a full reload across undo/redo.
    #[test]
    fn assign_pattern_and_pattern_span_patch_in_place_without_rebuilding_plugins() {
        let mut d = Document::new_project().unwrap();
        d.add_slot(0, 0, 0, &crate::NewModule::Plugin { id: "Test:VST3:INSTR:1".into(), name: "Synth".into(), chunk: vec![7u8; 4096] })
            .unwrap();
        d.add_pattern(0, 0, "One").unwrap();
        d.add_pattern(0, 0, "Two").unwrap();
        d.add_section(0).unwrap();
        let ptr_before = d.project().groups[0].sounds[0].plugin.as_ref().unwrap().chunk.as_ptr();

        d.assign_pattern(0, 0, Some(1)).unwrap();
        d.set_pattern_span(0, 1, 240, 2 * deckard_model::PPQ * 4).unwrap();
        assert_eq!(d.project().groups[0].sounds[0].plugin.as_ref().unwrap().chunk.as_ptr(), ptr_before);
        let reloaded = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(d.project().scenes, reloaded.project().scenes);
        assert_eq!(d.project().sections, reloaded.project().sections);
        assert_eq!(d.project().groups[0].patterns[1].start, 240);
        assert_eq!(d.project().groups[0].patterns[1].length, 2 * deckard_model::PPQ * 4);

        assert!(d.undo());
        assert!(d.undo());
        assert_eq!(d.project().scenes[0].patterns, Vec::<(usize, usize)>::new());
        assert!(d.redo());
        assert!(d.redo());
        assert_eq!(d.project().scenes, reloaded.project().scenes);
        assert_eq!(d.project().sections, reloaded.project().sections);
        assert_eq!(d.project().groups[0].sounds[0].plugin.as_ref().unwrap().chunk.as_ptr(), ptr_before);
    }
}
