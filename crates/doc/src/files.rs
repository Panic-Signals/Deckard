//! Sounds, groups and plug-in presets as Maschine's own files — `.mxsnd`, `.mxgrp`,
//! `.mxinst` / `.mxfx` — to save from the Sound, Group and plug-in slot menus
//! (docs/superpowers/specs/2026-10-22-browser-design.md).
//!
//! Each is laid out as a project file (`mxprj::container::new_file`) with its kind and
//! a tag block (name, Library, Types, Characters). Its document is the object as the
//! project holds it, changed as Maschine's own files have it:
//! - a sound keeps no pattern events (its `sequences` are empty);
//! - a group keeps no `patterns` or `sequence` ("stored only at project level") and its
//!   routing — MIDI input, audio and aux outputs — is skipped (`skip_value` 1);
//! - a preset is the slot.
//!
//! Saved with samples, each sample is copied into `<name> Samples` beside the file and
//! referenced from there: relative to the user folder (`FileLocation` kind 1, alias
//! "Standard User Directory") when the file is inside it, as Maschine does, else by
//! its absolute path.

use crate::{Document, Op, Target};
use deckard_model::SampleRef;
use mxprj::decode::{Obj, Value};
use mxprj::sound_info::SoundInfo;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const FILE_LOCATION: &str = "NI::MASCOMMON::FileLocation";
const ALIAS_PATH: &str = "NI::MASCOMMON::AliasPath";
const USER_LIBRARY: &str = "Standard User Directory";
const SLOT: &str = "NI::MASCOMMON::DATA::Slot";
const QUICK_BROWSE: &str = "NI::DB3::QuickBrowseHandle";

/// What to save as a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileItem {
    Sound(usize, usize),
    Group(usize),
    /// Slot `.2` of a sound's chain.
    Slot(usize, usize, usize),
}

/// Whether the samples go with the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Samples {
    /// Referenced where they are.
    Reference,
    /// Copied beside the file.
    Copy,
}

impl Document {
    /// A slot's file extension: `mxinst` for an instrument, `mxfx` for an effect.
    pub fn slot_extension(&self, g: usize, s: usize, slot: usize) -> &'static str {
        let instrument =
            self.project.groups.get(g).and_then(|x| x.sounds.get(s)).and_then(|x| x.slots.get(slot)).is_some_and(|x| x.is_instrument());
        if instrument { "mxinst" } else { "mxfx" }
    }

    /// The file `what` saves as, carrying `info`.
    pub fn file_bytes(&self, what: FileItem, info: &SoundInfo) -> Result<Vec<u8>, String> {
        self.encode_file(what, self.file_object(what)?, info)
    }

    /// Save `what` to `path` (through a temporary file), with `info` as its tags. With
    /// [`Samples::Copy`] its samples are copied beside it (found by `resolve`; one it
    /// can't find keeps its reference) and referenced from there; `user_dir` is
    /// Maschine's user folder.
    pub fn save_file(
        &self,
        what: FileItem,
        info: &SoundInfo,
        path: &Path,
        samples: Samples,
        user_dir: &Path,
        resolve: &mut dyn FnMut(&SampleRef) -> Option<PathBuf>,
    ) -> Result<(), String> {
        let mut obj = self.file_object(what)?;
        let dir = path.parent().ok_or("no folder to save in")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        if samples == Samples::Copy {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            copy_samples(&mut obj, &dir.join(format!("{stem} Samples")), user_dir, resolve)?;
        }
        let bytes = self.encode_file(what, obj, info)?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".saving");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The object id of what `what` names now (it changes when the object does, and
    /// when the project is saved and opened again).
    pub fn file_item_id(&self, what: FileItem) -> Option<u64> {
        match self.tree.at(&self.file_item_path(what).ok()?) {
            Some(Value::Obj(o)) => o.oid,
            _ => None,
        }
    }

    fn file_item_path(&self, what: FileItem) -> Result<mxprj::decode::Path, String> {
        Ok(match what {
            FileItem::Sound(g, s) => self.owner(Target::Sound(g, s)).ok_or("no such sound")?,
            FileItem::Group(g) => self.owner(Target::Group(g)).ok_or("no such group")?,
            FileItem::Slot(g, s, slot) => {
                let items = self.field_path(&self.chain_path(g, s)?, &["slots", "objects", "items"])?;
                let mut p = [items.as_slice(), &[slot as u32]].concat();
                let Some(Value::Obj(holder)) = self.tree.at(&p) else { return Err("no such slot".into()) };
                p.extend(holder.find_path("px").ok_or("slot without px")?);
                p
            }
        })
    }

    /// The object `what` is, as its file holds it.
    fn file_object(&self, what: FileItem) -> Result<Obj, String> {
        let at = self.file_item_path(what)?;
        let Some(Value::Obj(obj)) = self.tree.at(&at) else { return Err("not an object".into()) };
        let mut obj = self_contained(obj.clone(), &self.tree);
        match what {
            FileItem::Sound(..) => empty_sequences(&mut obj),
            FileItem::Group(_) => {
                obj.fields.retain(|(n, _)| n != "patterns" && n != "sequence");
                skip_routing(&mut obj, "event_input");
                if let Some(Value::Obj(track)) = obj.get_mut("<NI::MASCHINE::DATA::Track>") {
                    for n in ["audio_output", "aux1_output", "aux2_output"] {
                        skip_routing(track, n);
                    }
                }
            }
            FileItem::Slot(..) => {}
        }
        Ok(obj)
    }

    fn encode_file(&self, what: FileItem, obj: Obj, info: &SoundInfo) -> Result<Vec<u8>, String> {
        let (kind, type_name) = match what {
            FileItem::Sound(..) => (*b"SND ", "NI::MASCHINE::DATA::Sound"),
            FileItem::Group(_) => (*b"GRP ", "NI::MASCHINE::DATA::Group"),
            FileItem::Slot(..) => (*b"SLOT", "NI::MASCOMMON::DATA::Slot"),
        };
        let header = mxprj::Project::parse(&self.file).map_err(|e| e.to_string())?.header;
        // What only DECKARD reads stays out (a file has no DECKARD block).
        let mut obj = Value::Obj(obj);
        if !self.native_pressure {
            crate::block::strip_channel_pressure(&mut obj);
        }
        crate::expression::strip_deckard_fields(&mut obj);
        let Value::Obj(obj) = obj else { unreachable!() };
        let (archive, _) = mxprj::decode::encode(obj, type_name, header, &self.types.loaders, &self.types.exports)
            .map_err(|(e, at)| format!("{e} in {at}"))?;
        Ok(mxprj::container::new_file(kind, &archive, mxprj::types::classes::APP_VERSION, info, &mut crate::build::uuids()))
    }
}

impl Document {
    /// The edit just made, undone if the project would no longer open (a file from a
    /// Maschine version whose layout DECKARD can't convert).
    fn check_reopens(&mut self) -> Result<(), String> {
        let ok = self.to_bytes().and_then(|b| {
            let p = mxprj::Project::parse(&b).map_err(|e| e.to_string())?;
            p.decode(&self.types).map(drop).map_err(|(e, _)| e.to_string())
        });
        if let Err(e) = ok {
            self.undo();
            // Not to be redone.
            self.redo.pop();
            return Err(format!("this file's format can't be converted ({e})"));
        }
        Ok(())
    }

    /// The document of a Maschine file of `kind` (`SND `, `GRP `, `SLOT`).
    fn file_root(&self, bytes: &[u8], kind: [u8; 4]) -> Result<Obj, String> {
        let p = mxprj::Project::parse(bytes).map_err(|e| e.to_string())?;
        if p.kind() != Some(kind) {
            return Err(format!("not a {} file", String::from_utf8_lossy(&kind).trim()));
        }
        p.decode(&self.types).map_err(|(e, at)| format!("{e} in {at}"))
    }

    /// The file's object with ids of this project's (as a paste).
    fn fresh_obj(&mut self, root: Obj) -> Result<Obj, String> {
        let Value::Obj(o) = self.fresh_copy(&Value::Obj(root)) else { return Err("bad file".into()) };
        let classes = crate::build::class_table(Some(self.root()));
        let before = Value::Obj(o.clone());
        let fitted = self.build(|b| Ok(fit_versions(b, &classes, Value::Obj(o))))?;
        let Value::Obj(fitted) = fitted else { return Err("bad file".into()) };
        // A definition a rebuilt object dropped (a field its version doesn't have),
        // put back at its first remaining reference.
        Ok(self_contained(fitted, &before))
    }

    /// Load a sound file (`.mxsnd`) into sound `s` of group `g`: the sound replaced, its
    /// notes in the patterns kept (as loading onto a pad does). One undo step.
    pub fn load_sound_file(&mut self, g: usize, s: usize, bytes: &[u8]) -> Result<(), String> {
        let root = self.file_root(bytes, *b"SND ")?;
        let path = self.owner(Target::Sound(g, s)).ok_or("no such sound")?;
        let Some(Value::Obj(old)) = self.tree.at(&path).cloned() else { return Err("no such sound".into()) };
        let mut new = self.fresh_obj(root)?;
        // The slot keeps its identity (patterns refer to it) and its own sequences.
        take_id(&mut new, &old);
        if let (Some(i), Some(seq)) = (new.fields.iter().position(|(f, _)| f == "sequences"), old.get("sequences")) {
            new.fields[i].1 = seq.clone();
        }
        self.apply("Load Sound", vec![Op::Set { path, value: Value::Obj(new) }])?;
        self.check_reopens()
    }

    /// Load a group file (`.mxgrp`) into group `g`: its sounds, chain and patterns
    /// replace the group's; the group keeps its place, routing and handles, and its slot
    /// in each scene is emptied (the old patterns are gone). One undo step.
    pub fn load_group_file(&mut self, g: usize, bytes: &[u8]) -> Result<(), String> {
        let root = self.file_root(bytes, *b"GRP ")?;
        let path = self.owner(Target::Group(g)).ok_or("no such group")?;
        let Some(Value::Obj(old)) = self.tree.at(&path).cloned() else { return Err("no such group".into()) };
        let mut new = self.fresh_obj(root)?;
        take_id(&mut new, &old);
        // Project-level fields a group file leaves out, after its patterns_by_id.
        let at = new.fields.iter().position(|(n, _)| n == "patterns_by_id").map_or(new.fields.len(), |i| i + 1);
        for (k, name) in ["patterns", "sequence"].iter().enumerate() {
            let Some(v) = old.get(name) else { continue };
            match new.get_mut(name) {
                Some(mine) => *mine = v.clone(),
                None => new.fields.insert(at + k, (name.to_string(), v.clone())),
            }
        }
        // The project's routing, as the group had it.
        copy_field(&old, &mut new, "event_input");
        if let (Some(Value::Obj(ot)), Some(Value::Obj(nt))) =
            (old.get("<NI::MASCHINE::DATA::Track>"), new.get_mut("<NI::MASCHINE::DATA::Track>"))
        {
            for n in ["audio_output", "aux1_output", "aux2_output"] {
                copy_field(ot, nt, n);
            }
        }
        let oid = old.oid.ok_or("group without id")?;
        let mut ops = vec![Op::Set { path, value: Value::Obj(new) }];
        // Its slot in every scene, empty.
        let song = self.addresses.song.clone().ok_or("the project has no song")?;
        let scenes = self.field_path(&song, &["scenes", "objects", "items"])?;
        for k in 0..self.items(&scenes)?.len() {
            let mut item = [scenes.as_slice(), &[k as u32]].concat();
            let Some(Value::Obj(holder)) = self.tree.at(&item) else { continue };
            item.extend(holder.find_path("px").ok_or("scene item without px")?);
            let entries_path = self.field_path(&item, &["patterns", "objects", "items"])?;
            let mut entries = self.items(&entries_path)?;
            let Some(i) = entries.iter().position(|e| matches!(e, Value::Obj(p) if p.get("first") == Some(&Value::Ref(oid)))) else {
                continue;
            };
            entries[i] = self.build(|b| b.scene_entry(oid))?;
            ops.push(Op::Set { path: entries_path, value: Value::List(entries) });
        }
        self.apply("Load Group", ops)?;
        self.check_reopens()
    }

    /// Load a preset (`.mxinst`, `.mxfx`) into slot `slot` of sound `s`'s chain, in place
    /// of what's there or (`slot` = the chain's length) after the last. One undo step.
    pub fn load_slot_file(&mut self, g: usize, s: usize, slot: usize, bytes: &[u8]) -> Result<(), String> {
        let root = self.file_root(bytes, *b"SLOT")?;
        let new = self.fresh_obj(root)?;
        let (item, qb) = self.build(|b| Ok((b.shared(SLOT, new), Value::Obj(b.blank(QUICK_BROWSE)?))))?;
        let chain_path = self.chain_path(g, s)?;
        let Some(Value::Obj(mut chain)) = self.tree.at(&chain_path).cloned() else { return Err("the sound has no chain".into()) };
        let slots = crate::build::obj_mut(&mut chain, "slots")?;
        let objs = crate::build::obj_mut(slots, "objects")?;
        if !objs.fields.iter().any(|(n, _)| n == "items#item_version") {
            crate::build::push_items(objs, Vec::new(), 1)?;
        }
        let Some(Value::List(items)) = objs.get_mut("items") else { return Err("slots without items".into()) };
        if slot < items.len() {
            items[slot] = item;
        } else {
            let at = items.len();
            items.push(item);
            let first_qb = chain.fields.iter().position(|(n, _)| n == "quick_browse").unwrap_or(chain.fields.len());
            chain.fields.insert(first_qb + at, ("quick_browse".into(), qb));
        }
        self.apply("Load Preset", vec![Op::Set { path: chain_path, value: Value::Obj(chain) }])?;
        self.check_reopens()
    }
}

/// `v` with each object whose class this project stores at another version rebuilt at
/// the project's (one archive holds one version of a class; files from other Maschine
/// versions differ): a new object of that class with the old one's fields, by name,
/// where it has them.
fn fit_versions(b: &mut crate::build::Builder, classes: &HashMap<String, (u32, bool)>, v: Value) -> Value {
    match v {
        Value::List(items) => Value::List(items.into_iter().map(|i| fit_versions(b, classes, i)).collect()),
        Value::Obj(mut o) => {
            legacy_track(&mut o);
            o.fields = std::mem::take(&mut o.fields).into_iter().map(|(n, v)| (n, fit_versions(b, classes, v))).collect();
            let theirs = classes.get(&o.type_name).map(|c| c.0);
            if o.type_name.is_empty() || theirs.is_none_or(|v| v == o.version) {
                return Value::Obj(o);
            }
            let Ok(mut fresh) = b.blank(&o.type_name) else { return Value::Obj(o) };
            // A browser bookmark's data is laid out per version: a new one starts empty.
            let keep = o.type_name != QUICK_BROWSE;
            for (name, value) in fresh.fields.iter_mut().filter(|_| keep) {
                if let Some(old) = o.fields.iter().find(|(n, v)| n == name && same_kind(v, value)) {
                    *value = old.1.clone();
                }
            }
            // A field stored once per item (a chain's `quick_browse`, one per slot): as
            // many as the old object has, where the new one has its first.
            let mut names: Vec<&String> = o.fields.iter().map(|(n, _)| n).collect();
            names.dedup();
            for name in names {
                let theirs: Vec<(String, Value)> = o.fields.iter().filter(|(n, _)| n == name).cloned().collect();
                let mine = fresh.fields.iter().filter(|(n, _)| n == name).count();
                if theirs.len() <= 1 && mine <= 1 {
                    continue;
                }
                let Some(at) = fresh.fields.iter().position(|(n, _)| n == name) else { continue };
                fresh.fields.retain(|(n, _)| n != name);
                for (k, f) in theirs.into_iter().enumerate() {
                    fresh.fields.insert(at + k, f);
                }
            }
            if o.oid.is_some() {
                fresh.oid = o.oid;
            }
            Value::Obj(fresh)
        }
        v => v,
    }
}

/// Whether `a` can stand where `b` is: the same kind of value (an object of the same
/// class, or a reference where an object may be).
fn same_kind(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Obj(x), Value::Obj(y)) => x.type_name == y.type_name,
        (Value::Ref(_) | Value::Null, Value::Obj(_) | Value::Ref(_) | Value::Null) => true,
        (Value::Obj(_), Value::Ref(_) | Value::Null) => true,
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

/// A group from before Maschine kept its mixer settings in a `Track` base (class
/// version < 13): its `track_parameters_legacy` and `MixingLayer` base become one, to
/// be rebuilt as the project's `Track`.
fn legacy_track(o: &mut Obj) {
    const TRACK: &str = "<NI::MASCHINE::DATA::Track>";
    const LAYER: &str = "<NI::MASCHINE::DATA::MixingLayer>";
    if o.type_name != crate::build::GROUP || o.get(TRACK).is_some() {
        return;
    }
    let Some(i) = o.fields.iter().position(|(n, _)| n == "track_parameters_legacy") else { return };
    let Value::Obj(params) = o.fields.remove(i).1 else { return };
    let mut fields = params.fields;
    if let Some(j) = o.fields.iter().position(|(n, _)| n == LAYER) {
        fields.push(o.fields.remove(j));
    }
    let track = Obj { type_name: "NI::MASCHINE::DATA::Track".into(), version: u32::MAX, oid: None, export: None, fields };
    let at = o.fields.iter().position(|(n, _)| n == "patterns_by_id").unwrap_or(o.fields.len());
    o.fields.insert(at, (TRACK.into(), Value::Obj(track)));
}

/// `obj` with every object it refers back to but doesn't define (defined earlier in
/// the project) copied in at its first reference, so it stands as a file of its own.
fn self_contained(obj: Obj, tree: &Value) -> Obj {
    let mut v = Value::Obj(obj);
    for _ in 0..10_000 {
        let Some(id) = first_dangling(&v) else { break };
        let mut def = None;
        tree.walk(&mut |o| {
            if def.is_none() && o.oid == Some(id) {
                def = Some(o.clone());
            }
        });
        let Some(def) = def else { break };
        let mut done = false;
        replace_first_ref(&mut v, id, &mut Some(def), &mut done);
    }
    match v {
        Value::Obj(o) => o,
        _ => unreachable!(),
    }
}

fn first_dangling(v: &Value) -> Option<u64> {
    fn walk(v: &Value, seen: &mut std::collections::HashSet<u64>) -> Option<u64> {
        match v {
            Value::Ref(o) if !seen.contains(o) => Some(*o),
            Value::List(items) => items.iter().find_map(|i| walk(i, seen)),
            Value::Obj(o) => {
                if let Some(id) = o.oid {
                    seen.insert(id);
                }
                o.fields.iter().find_map(|(_, v)| walk(v, seen))
            }
            _ => None,
        }
    }
    walk(v, &mut std::collections::HashSet::new())
}

fn replace_first_ref(v: &mut Value, id: u64, def: &mut Option<Obj>, done: &mut bool) {
    if *done {
        return;
    }
    match v {
        Value::Ref(o) if *o == id => {
            if let Some(d) = def.take() {
                *v = Value::Obj(d);
                *done = true;
            }
        }
        Value::List(items) => items.iter_mut().for_each(|i| replace_first_ref(i, id, def, done)),
        Value::Obj(o) => o.fields.iter_mut().for_each(|(_, v)| replace_first_ref(v, id, def, done)),
        _ => {}
    }
}

/// `new` takes `old`'s place: its id (what refers to `old` refers to it) and export
/// name; its references to itself follow.
fn take_id(new: &mut Obj, old: &Obj) {
    fn repoint(v: &mut Value, from: u64, to: u64) {
        match v {
            Value::Ref(o) if *o == from => *o = to,
            Value::List(items) => items.iter_mut().for_each(|i| repoint(i, from, to)),
            Value::Obj(o) => o.fields.iter_mut().for_each(|(_, v)| repoint(v, from, to)),
            _ => {}
        }
    }
    if let (Some(from), Some(to)) = (new.oid, old.oid) {
        new.fields.iter_mut().for_each(|(_, v)| repoint(v, from, to));
    }
    new.oid = old.oid;
    new.export = old.export.clone();
}

/// `field` of `to` set to `from`'s (when both have it).
fn copy_field(from: &Obj, to: &mut Obj, field: &str) {
    if let (Some(v), Some(slot)) = (from.get(field), to.get_mut(field)) {
        *slot = v.clone();
    }
}

/// The samples a file's zones and takes name, in order.
pub fn sample_refs(root: &Obj) -> Vec<SampleRef> {
    let mut out = Vec::new();
    Value::Obj(root.clone()).walk(&mut |o| {
        if let Some(Value::Obj(f)) = o.get("file")
            && f.type_name == FILE_LOCATION
        {
            out.push(sample_ref(f));
        }
    });
    out
}

fn sample_ref(f: &Obj) -> SampleRef {
    let text = |o: &Obj, k: &str| match o.get(k) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let alias = match f.get("alias") {
        Some(Value::Obj(a)) => Some(a),
        _ => None,
    };
    SampleRef {
        kind: f.get("kind").and_then(Value::as_int).unwrap_or(0),
        path: text(f, "path"),
        library_root: alias.map(|a| text(a, "root")).unwrap_or_default(),
        library: alias.map(|a| text(a, "library")).unwrap_or_default(),
    }
}

/// A sound's `sequences` emptied (no items, none current or selected).
fn empty_sequences(sound: &mut Obj) {
    let Some(Value::Obj(seq)) = sound.get_mut("sequences") else { return };
    if let Some(Value::Obj(objs)) = seq.get_mut("objects")
        && let Some(Value::List(items)) = objs.get_mut("items")
    {
        items.clear();
    }
    if let Some(cur) = seq.get_mut("current") {
        *cur = Value::Null;
    }
    if let Some(Value::Obj(sel)) = seq.get_mut("selection")
        && let Some(Value::List(items)) = sel.get_mut("items")
    {
        items.clear();
    }
}

/// A routing parameter's value left out, as outside a project (`skip_value` 1).
fn skip_routing(owner: &mut Obj, field: &str) {
    let Some(Value::Obj(p)) = owner.get_mut(field) else { return };
    if let Some(v) = p.get_mut("skip_value") {
        *v = Value::Int(1);
    }
    p.fields.retain(|(n, _)| !n.starts_with("<NI::ML::TM::ValueParameter"));
}

/// Copy every sample `root` names into `dir` and point it there.
fn copy_samples(root: &mut Obj, dir: &Path, user_dir: &Path, resolve: &mut dyn FnMut(&SampleRef) -> Option<PathBuf>) -> Result<(), String> {
    // Source file → its copy.
    let mut copies: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut err = None;
    let mut root_v = Value::Obj(std::mem::take(root));
    root_v.walk_mut(&mut |o| {
        let Some(Value::Obj(f)) = o.get_mut("file") else { return };
        if f.type_name != FILE_LOCATION || err.is_some() {
            return;
        }
        let Some(src) = resolve(&sample_ref(f)) else { return };
        let dest = match copies.get(&src) {
            Some(d) => d.clone(),
            None => {
                let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "sample.wav".into());
                let mut dest = dir.join(&name);
                let mut n = 2;
                while copies.values().any(|d| *d == dest) {
                    let p = Path::new(&name);
                    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                    dest = dir.join(format!("{stem} {n}{ext}"));
                    n += 1;
                }
                if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::copy(&src, &dest)) {
                    err = Some(format!("{}: {e}", src.display()));
                    return;
                }
                copies.insert(src, dest.clone());
                dest
            }
        };
        *f = location(&dest, user_dir);
    });
    let Value::Obj(o) = root_v else { unreachable!() };
    *root = o;
    err.map_or(Ok(()), Err)
}

/// A `FileLocation` for `file`: relative to the user folder when inside it.
fn location(file: &Path, user_dir: &Path) -> Obj {
    let obj = |kind: i64, path: String, alias: Option<Value>| Obj {
        type_name: FILE_LOCATION.into(),
        version: 0,
        oid: None,
        export: None,
        fields: [("kind".to_string(), Value::Int(kind)), ("path".to_string(), Value::Str(path))]
            .into_iter()
            .chain(alias.map(|a| ("alias".to_string(), a)))
            .collect(),
    };
    match file.strip_prefix(user_dir) {
        Ok(rel) => {
            let rel = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            let alias = Obj {
                type_name: ALIAS_PATH.into(),
                version: 0,
                oid: None,
                export: None,
                fields: vec![
                    ("root".into(), Value::Str(user_dir.display().to_string())),
                    ("library".into(), Value::Str(USER_LIBRARY.into())),
                ],
            };
            obj(1, rel, Some(Value::Obj(alias)))
        }
        Err(_) => obj(0, file.display().to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use crate::Document;
    use crate::files::{FileItem, Samples};
    use mxprj::sound_info::SoundInfo;
    use std::path::{Path, PathBuf};

    const CLOUD9: &str = "/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj";

    fn open() -> Option<Document> {
        Document::open(Path::new(CLOUD9)).ok()
    }

    fn tagged(name: &str) -> SoundInfo {
        let mut info = SoundInfo::default();
        info.name = name.into();
        info.bank = vec!["User".into()];
        info.set_tags(&[vec!["Drums".into(), "Kick".into()]], &["Punchy".into()]);
        info
    }

    /// The item tree of a file: each item's domain/kind and frame ids, depth first.
    fn outline(file: &[u8]) -> Vec<String> {
        fn walk(i: &mxprj::Item, out: &mut Vec<String>, at: &str) {
            out.push(format!("{at} {:?}", i.frames.iter().map(|f| f.id).collect::<Vec<_>>()));
            for c in &i.children {
                walk(&c.item, out, &format!("{}/{}", String::from_utf8_lossy(&c.domain), c.item_id));
            }
        }
        let mut out = Vec::new();
        walk(&mxprj::Item::parse(file).unwrap(), &mut out, "root");
        out
    }

    fn decoded(file: &[u8]) -> mxprj::decode::Obj {
        mxprj::Project::parse(file).unwrap().decode(&mxprj::types::Types::new()).map_err(|(e, at)| format!("{e} at {at}")).unwrap()
    }

    #[test]
    fn a_saved_sound_carries_its_tags() {
        let Some(d) = open() else { return };
        let info = tagged("Kick Test");
        let file = d.file_bytes(FileItem::Sound(0, 0), &info).unwrap();
        let p = mxprj::Project::parse(&file).unwrap();
        assert_eq!(p.kind(), Some(*b"SND "));
        assert_eq!(p.sound_info(), Some(info));
        let root = decoded(&file);
        assert_eq!(root.type_name, "NI::MASCHINE::DATA::Sound");
        // A sound file has no pattern events of its own (as Maschine's).
        let mut sequences = 0;
        mxprj::decode::Value::Obj(root.clone()).walk(&mut |o| {
            if o.type_name == "NI::MASCOMMON::DATA::Sequence" {
                sequences += 1;
            }
        });
        assert_eq!(sequences, 0);
    }

    #[test]
    fn a_saved_group_matches_maschines_layout() {
        let Some(d) = open() else { return };
        let Some(theirs) = std::fs::read_dir("/Users/Shared/Velvet Lounge Library/Groups/Kits")
            .ok()
            .and_then(|mut r| r.find_map(|e| e.ok().filter(|e| e.path().extension().is_some_and(|x| x == "mxgrp"))))
        else {
            return;
        };
        let theirs = std::fs::read(theirs.path()).unwrap();
        let ours = d.file_bytes(FileItem::Group(0), &tagged("Kit")).unwrap();
        assert_eq!(outline(&ours), outline(&theirs));
        let root = decoded(&ours);
        assert_eq!(root.type_name, "NI::MASCHINE::DATA::Group");
        assert!(root.get("patterns").is_none(), "patterns are stored only at project level");
        // Routing values skipped, as in Maschine's group files.
        let Some(mxprj::decode::Value::Obj(input)) = root.get("event_input") else { panic!("no event_input") };
        assert_eq!(input.get("skip_value"), Some(&mxprj::decode::Value::Int(1)));
    }

    #[test]
    fn a_slot_preset_is_mxfx_or_mxinst() {
        let Some(d) = open() else { return };
        let (g, s, slot) = d
            .project()
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, x)| x.sounds.iter().enumerate().map(move |(s, y)| (g, s, y)))
            .find_map(|(g, s, y)| y.slots.iter().position(|x| !x.is_instrument()).map(|k| (g, s, k)))
            .expect("an effect somewhere");
        assert_eq!(d.slot_extension(g, s, slot), "mxfx");
        assert_eq!(d.slot_extension(0, 0, 0), if d.project().groups[0].sounds[0].slots[0].is_instrument() { "mxinst" } else { "mxfx" });
        let file = d.file_bytes(FileItem::Slot(g, s, slot), &tagged("FX")).unwrap();
        assert_eq!(mxprj::Project::parse(&file).unwrap().kind(), Some(*b"SLOT"));
        assert_eq!(decoded(&file).type_name, "NI::MASCOMMON::DATA::Slot");
    }

    #[test]
    fn save_with_samples_copies_and_relinks() {
        let Some(d) = open() else { return };
        let (g, s) = (0..d.project().groups.len())
            .flat_map(|g| (0..d.project().groups[g].sounds.len()).map(move |s| (g, s)))
            .find(|&(g, s)| d.project().groups[g].sounds[s].sampler.as_ref().is_some_and(|x| !x.zones.is_empty()))
            .expect("a sound with a sample");
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path();
        let src = user.join("source.wav");
        std::fs::write(&src, b"RIFF....WAVE").unwrap();
        let path = user.join("Sounds/My Sound.mxsnd");
        d.save_file(FileItem::Sound(g, s), &tagged("My Sound"), &path, Samples::Copy, &user, &mut |_| Some(src.clone())).unwrap();
        let copied = user.join("Sounds/My Sound Samples/source.wav");
        assert!(copied.is_file(), "the sample is copied beside the file");
        // The zone points at it, relative to the user folder.
        let refs = crate::files::sample_refs(&decoded(&std::fs::read(&path).unwrap()));
        assert!(!refs.is_empty());
        assert!(refs.iter().all(|r| r.kind == 1 && r.path == "Sounds/My Sound Samples/source.wav"), "{refs:?}");
        assert!(refs.iter().all(|r| Path::new(&r.library_root) == user));
    }

    #[test]
    fn saving_without_samples_keeps_their_references() {
        let Some(d) = open() else { return };
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = dir.join("Kit.mxgrp");
        d.save_file(FileItem::Group(0), &tagged("Kit"), &path, Samples::Reference, &dir, &mut |_| None).unwrap();
        let ours = crate::files::sample_refs(&decoded(&std::fs::read(&path).unwrap()));
        let theirs: Vec<_> = d.project().groups[0]
            .sounds
            .iter()
            .flat_map(|x| x.sampler.iter().flat_map(|y| y.zones.iter().map(|z| z.sample.clone())))
            .collect();
        assert!(!theirs.is_empty());
        assert!(theirs.iter().all(|r| ours.contains(r)), "every zone keeps its sample");
    }

    /// Save to a temporary file and read it back.
    fn reload(d: &Document) -> Document {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("p.mxprj");
        std::fs::write(&path, d.to_bytes().unwrap()).unwrap();
        Document::open(&path).unwrap()
    }

    fn modules(d: &Document, g: usize, s: usize) -> Vec<String> {
        d.project().groups[g].sounds[s].slots.iter().map(|x| x.module.clone()).collect()
    }

    #[test]
    fn factory_group_loads_with_its_samples() {
        let Some(mut d) = open() else { return };
        let Some(file) = std::fs::read_dir("/Users/Shared/Velvet Lounge Library/Groups/Kits")
            .ok()
            .and_then(|mut r| r.find_map(|e| e.ok().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "mxgrp"))))
        else {
            return;
        };
        let bytes = std::fs::read(&file).unwrap();
        let patterns = {
            let root = decoded(&bytes);
            let mut n = 0;
            if let Some(v) = root.get("patterns_by_id") {
                v.walk(&mut |o| n += usize::from(o.type_name == "NI::MASCHINE::DATA::EventPattern"));
            }
            n
        };
        d.load_group_file(0, &bytes).unwrap();
        let g = &d.project().groups[0];
        assert_eq!(g.patterns.len(), patterns);
        let mut resolver = deckard_engine::library::Resolver::new(Some(&file));
        let zones: Vec<_> = g.sounds.iter().flat_map(|s| s.sampler.iter().flat_map(|x| x.zones.iter())).collect();
        assert!(!zones.is_empty());
        for z in zones {
            assert!(resolver.resolve(&z.sample).is_some(), "{:?} resolves", z.sample);
        }
        // It saves and opens again the same.
        let back = reload(&d);
        assert_eq!(back.project().groups[0].sounds.len(), d.project().groups[0].sounds.len());
        assert_eq!(back.project().groups[0].patterns.len(), patterns);
    }

    #[test]
    fn loading_a_sound_is_one_undo_step() {
        let Some(mut d) = open() else { return };
        let before = modules(&d, 0, 5);
        let bytes = d.file_bytes(FileItem::Sound(0, 0), &tagged("K")).unwrap();
        d.load_sound_file(0, 5, &bytes).unwrap();
        assert_eq!(modules(&d, 0, 5), modules(&d, 0, 0));
        assert!(d.undo());
        assert_eq!(modules(&d, 0, 5), before);
    }

    #[test]
    fn a_loaded_sound_keeps_the_pads_notes() {
        let Some(mut d) = open() else { return };
        let (g, s) = (0..d.project().groups.len())
            .flat_map(|g| (0..d.project().groups[g].sounds.len()).map(move |s| (g, s)))
            .find(|&(g, s)| d.project().groups[g].patterns.iter().any(|p| p.notes.iter().any(|n| n.sound == s)))
            .expect("a sound with notes");
        let notes =
            |d: &Document| d.project().groups[g].patterns.iter().map(|p| p.notes.iter().filter(|n| n.sound == s).count()).sum::<usize>();
        let before = notes(&d);
        let bytes = d.file_bytes(FileItem::Sound(g, (s + 1) % 16), &tagged("K")).unwrap();
        d.load_sound_file(g, s, &bytes).unwrap();
        assert_eq!(notes(&d), before);
        let back = reload(&d);
        assert_eq!(notes(&back), before);
    }

    #[test]
    fn loading_a_preset_appends_a_slot() {
        let Some(mut d) = open() else { return };
        let (g, s, k) = d
            .project()
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, x)| x.sounds.iter().enumerate().map(move |(s, y)| (g, s, y)))
            .find_map(|(g, s, y)| y.slots.iter().position(|x| !x.is_instrument()).map(|k| (g, s, k)))
            .unwrap();
        let module = d.project().groups[g].sounds[s].slots[k].module.clone();
        let bytes = d.file_bytes(FileItem::Slot(g, s, k), &tagged("FX")).unwrap();
        let n = d.project().groups[0].sounds[0].slots.len();
        d.load_slot_file(0, 0, n, &bytes).unwrap();
        assert_eq!(d.project().groups[0].sounds[0].slots.len(), n + 1);
        assert_eq!(d.project().groups[0].sounds[0].slots[n].module, module);
        let back = reload(&d);
        assert_eq!(back.project().groups[0].sounds[0].slots[n].module, module);
    }

    #[test]
    fn the_wrong_kind_of_file_is_refused() {
        let Some(mut d) = open() else { return };
        let bytes = d.file_bytes(FileItem::Group(0), &tagged("G")).unwrap();
        assert!(d.load_sound_file(0, 0, &bytes).is_err());
        assert!(d.load_slot_file(0, 0, 0, &bytes).is_err());
    }

    /// Factory files from every Maschine version load into a project that then saves
    /// and opens again: a few of each kind here; the full spread (25 of each, minutes in
    /// a debug build) with `cargo test --release -p deckard-doc -- --ignored factory_files`.
    #[test]
    fn factory_files_load_across_versions() {
        factory_files_load(3);
    }

    #[test]
    #[ignore]
    fn factory_files_load_across_versions_all() {
        factory_files_load(25);
    }

    fn factory_files_load(per_kind: usize) {
        let Some(d0) = open() else { return };
        fn collect(dir: &Path, ext: &str, out: &mut Vec<PathBuf>, depth: usize) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() && depth < 5 {
                    collect(&p, ext, out, depth + 1);
                } else if p.extension().is_some_and(|x| x == ext) {
                    out.push(p);
                }
            }
        }
        let mut failures = Vec::new();
        let mut refused = Vec::new();
        let mut loaded = 0;
        for ext in ["mxgrp", "mxsnd", "mxfx", "mxinst"] {
            let mut files = Vec::new();
            collect(Path::new("/Users/Shared"), ext, &mut files, 0);
            files.sort();
            let step = (files.len() / per_kind).max(1);
            for f in files.iter().step_by(step).take(per_kind) {
                let mut d = Document::from_bytes(d0.to_bytes().unwrap(), None).unwrap();
                let bytes = std::fs::read(f).unwrap();
                let r = match ext {
                    "mxgrp" => d.load_group_file(0, &bytes),
                    "mxsnd" => d.load_sound_file(0, 3, &bytes),
                    _ => {
                        let n = if ext == "mxinst" { 0 } else { d.project().groups[0].sounds[3].slots.len() };
                        d.load_slot_file(0, 3, n, &bytes)
                    }
                };
                match r {
                    Ok(()) => match d.to_bytes().and_then(|b| Document::from_bytes(b, None).map(drop)) {
                        Ok(()) => loaded += 1,
                        Err(e) => failures.push(format!("{}: loaded but doesn't reopen: {e}", f.display())),
                    },
                    Err(e) => refused.push(format!("{}: {e}", f.display())),
                }
            }
        }
        eprintln!("{loaded} loaded, {} refused:\n{}", refused.len(), refused.join("\n"));
        assert!(failures.is_empty(), "{} failed:\n{}", failures.len(), failures.join("\n"));
        assert!(refused.len() * 10 <= loaded + refused.len(), "too many refused");
    }

    /// Files that once failed: old groups whose sidechain refers to the group itself
    /// (they load), and a Maschine 1 sound DECKARD can't convert (refused, nothing
    /// changed).
    #[test]
    fn old_files_that_failed_load() {
        let Some(d0) = open() else { return };
        for (f, group) in [
            ("/Users/Shared/Chromatic Fire Library/Groups/Kits/Black Earth Kit.mxgrp", true),
            ("/Users/Shared/Maschine 2 Library/Groups/Kits/Float Kit.mxgrp", true),
            ("/Users/Shared/Maschine 2 Library/Sounds/Drumsynth/Tronic TomSat.mxsnd", false),
        ] {
            let Ok(bytes) = std::fs::read(f) else { continue };
            let mut d = Document::from_bytes(d0.to_bytes().unwrap(), None).unwrap();
            let before = d.to_bytes().unwrap();
            let r = if group { d.load_group_file(0, &bytes) } else { d.load_sound_file(0, 3, &bytes) };
            match r {
                // Loaded: the project saves and opens again.
                Ok(()) => drop(Document::from_bytes(d.to_bytes().unwrap(), None).unwrap_or_else(|e| panic!("{f}: {e}"))),
                // Or refused, and the project is as it was.
                Err(e) => {
                    eprintln!("{f}: {e}");
                    assert!(d.to_bytes().unwrap() == before, "{f}: refused but changed");
                    assert!(!d.redo(), "{f}: the refused load can't be redone");
                }
            }
        }
    }
}
