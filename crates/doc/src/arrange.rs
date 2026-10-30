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

/// Object ids referenced but defined nowhere.
fn dangling(root: &Value) -> Vec<u64> {
    // A reference may come before its object's definition (a side-chain set to a group
    // further on): saving defines the object there. Dangling is defined nowhere.
    fn walk(v: &Value, defined: &mut HashSet<u64>, refs: &mut Vec<u64>) {
        match v {
            Value::Ref(o) => refs.push(*o),
            Value::List(items) => items.iter().for_each(|i| walk(i, defined, refs)),
            Value::Obj(o) => {
                if let Some(id) = o.oid {
                    defined.insert(id);
                }
                o.fields.iter().for_each(|(_, v)| walk(v, defined, refs));
            }
            _ => {}
        }
    }
    let mut defined = HashSet::new();
    let mut refs = Vec::new();
    walk(root, &mut defined, &mut refs);
    let mut bad: Vec<u64> = refs.into_iter().filter(|o| !defined.contains(o)).collect();
    bad.sort_unstable();
    bad.dedup();
    bad
}

/// Path of the first reference to each of `ids`, in archive order.
fn ref_paths(root: &Value, ids: &[u64]) -> HashMap<u64, Path> {
    fn walk(v: &Value, path: &mut Vec<u32>, ids: &[u64], out: &mut HashMap<u64, Path>) {
        match v {
            Value::Ref(o) if ids.contains(o) => {
                out.entry(*o).or_insert_with(|| path.clone());
            }
            Value::List(items) => {
                for (i, item) in items.iter().enumerate() {
                    path.push(i as u32);
                    walk(item, path, ids, out);
                    path.pop();
                }
            }
            Value::Obj(o) => {
                for (i, (_, f)) in o.fields.iter().enumerate() {
                    path.push(i as u32);
                    walk(f, path, ids, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut out = HashMap::new();
    walk(root, &mut Vec::new(), ids, &mut out);
    out
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
    pub(crate) fn rehomed(&mut self, label: &str, mut ops: Vec<Op>) -> Result<Vec<Op>, String> {
        for _ in 0..4 {
            let undo = self.run(&ops)?;
            let bad = dangling(&self.tree);
            let first_refs = if bad.is_empty() { HashMap::new() } else { ref_paths(&self.tree, &bad) };
            self.run(&undo).expect("undo of ops just applied");
            if bad.is_empty() {
                return Ok(ops);
            }
            // Move each removed definition to its first remaining reference.
            let mut originals = HashMap::new();
            self.tree.walk(&mut |o| {
                if let Some(id) = o.oid
                    && bad.contains(&id)
                {
                    originals.entry(id).or_insert_with(|| o.clone());
                }
            });
            // Only the outermost: what's defined inside a moved object moves with it (a
            // group carries its patterns).
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
            let mut inner = HashSet::new();
            for o in originals.values() {
                o.fields.iter().for_each(|(_, f)| ids_in(f, &mut inner));
            }
            for id in bad.iter().filter(|id| !inner.contains(id)) {
                let (Some(path), Some(obj)) = (first_refs.get(id), originals.get(id)) else {
                    return Err(format!("{label}: object #{id} would still be referenced after the edit"));
                };
                ops.push(Op::Set { path: path.clone(), value: Value::Obj(obj.clone()) });
            }
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
        self.apply("Assign Pattern", vec![Op::Set { path: entries_path, value: Value::List(entries) }])
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
        let Value::Obj(o) = &mut entry else { return Err("bad pattern entry".into()) };
        let pat = match o.get_mut("second") {
            Some(Value::Obj(sp)) => match sp.get_mut("px") {
                Some(Value::Obj(p)) => p,
                _ => return Err("pattern entry without a pattern".into()),
            },
            _ => return Err("pattern entry without a pattern".into()),
        };
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
        items.insert(at, entry);
        self.apply("New Pattern", vec![Op::Set { path: items_path, value: Value::List(items) }])?;
        Ok(at)
    }

    /// Take pattern `p` out of its group's list (the ones after it move up, their ids
    /// too). Scenes must not refer to it any more.
    pub(crate) fn remove_pattern_entry(&mut self, group: usize, p: usize) -> Result<(), String> {
        let items_path = self.pattern_items_path(group)?;
        let mut items = self.items(&items_path)?;
        if p >= items.len() {
            return Err("no such pattern".into());
        }
        let removed = items.remove(p);
        if let Some(id) = super::sounds::entry_id(&removed) {
            for e in items.iter_mut() {
                if let Some(i) = super::sounds::entry_id(e)
                    && i > id
                {
                    super::sounds::set_entry_id(e, i - 1);
                }
            }
        }
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
