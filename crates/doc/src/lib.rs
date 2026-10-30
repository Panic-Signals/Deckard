//! The open project: one editable copy that the viewer, the playback engine and the
//! controller scripts all share ([`Shared`]).
//!
//! The decoded `mxprj` tree is the source of truth, so saving writes back everything
//! the file had, understood or not. Every change is a set of [`Op`]s on that tree,
//! recorded with its inverse for undo. After each change the typed [`Project`] is
//! rebuilt from the tree and [`Document::revision`] goes up, which is how the engine
//! and the views know to refresh.

use deckard_model::{Addresses, Project};
use mxprj::decode::{Obj, Path, Value};
use std::cell::RefCell;
use std::path::{Path as FsPath, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

mod arrange;
mod automation;
mod block;
mod build;
mod clips;
pub use block::SavedScreen;
pub use clips::CLIP;
mod chain;
pub use chain::{Chain, NewModule};
pub use files::{FileItem, Samples, sample_refs};
mod audio;
mod lanes;
mod zones;
pub use audio::TakeSpec;
pub use zones::{ZoneFrom, ZoneSpec};
mod cc_routes;
mod expression;
mod files;
mod lfos;
mod mod_routes;
pub mod note_tools;
mod notes;
mod pad_touch;
mod prob_lock;
mod sounds;
pub use sounds::SoundClip;
mod channel;
mod groove;
mod groups;
mod lock;
mod macros;
pub use lock::LockEntry;
mod modulation;
pub use channel::SoundRoute;
pub use macros::{Macro, MacroPage};
mod plugin_params;
pub use plugin_params::PluginPageSpec;
mod plugins;
pub mod remote;

/// The document as the app's parts share it (all on the UI thread).
pub type Shared = Rc<RefCell<Document>>;

/// A change to the tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Replace the value at `path` (from the root object).
    Set { path: Path, value: Value },
    /// Set field `field` of the object at `path` (`None`: take it off). For DECKARD's
    /// own fields, which Maschine's objects don't have.
    SetField { path: Path, field: String, value: Option<Value> },
}

/// The part of the project a parameter belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Project,
    Song,
    Group(usize),
    /// Where a group's level, pan and mute live.
    GroupStrip(usize),
    Sound(usize, usize),
    /// The Sampler of a sound (its first, in the plug-in chain). Its fields are
    /// paths: `cutoff`, `volume_envelope/attack`, ... (see `SAMPLER_PARAMS`).
    Sampler(usize, usize),
    /// Zone `z` of that Sampler: `play_range/low`, `loop_active`, `x_fade`, ...
    Zone(usize, usize, usize),
    /// The module in slot `.2` of a sound's chain (an internal effect, the Audio
    /// plug-in): its fields.
    Module(usize, usize, usize),
    /// Take `.2` of a sound's Audio plug-in: `length` (beats), `play_range/low`, …
    Take(usize, usize, usize),
    /// The module in slot `.1` of any chain (a group's or the master's effects).
    ChainModule(chain::Chain, usize),
    Pattern(usize, usize),
    Scene(usize),
    Section(usize),
}

/// One undo step: what to apply to undo it and to redo it.
#[derive(Debug, Clone)]
struct Step {
    label: String,
    undo: Vec<Op>,
    redo: Vec<Op>,
    /// The parameter each op sets, when known: the model is patched in place
    /// instead of rebuilt.
    params: Vec<Option<(Target, String)>>,
    at: Instant,
    /// A take's edits, oldest first (step undo takes them back one at a time); empty for
    /// any other step.
    parts: Vec<Step>,
    /// An edit of a take, taken back by step undo: redone, it rejoins the take.
    of_take: bool,
}

impl Step {
    fn new(label: &str, undo: Vec<Op>, redo: Vec<Op>, params: Vec<Option<(Target, String)>>, at: Instant) -> Step {
        Step { label: label.to_string(), undo, redo, params, at, parts: Vec::new(), of_take: false }
    }

    /// One step doing `steps` in order (undone in reverse), at no time to merge into.
    fn merged(label: &str, steps: &[Step]) -> Step {
        let undo: Vec<Op> = steps.iter().rev().flat_map(|s| s.undo.iter().cloned()).collect();
        let redo: Vec<Op> = steps.iter().flat_map(|s| s.redo.iter().cloned()).collect();
        let params = vec![None; redo.len()];
        Step::new(label, undo, redo, params, Instant::now() - MERGE_WINDOW)
    }
}

/// Changes to the same parameters within this time make one undo step (a knob turn).
const MERGE_WINDOW: Duration = Duration::from_millis(1500);

pub struct Document {
    /// The file as read (or last saved): its container is kept when saving.
    file: Vec<u8>,
    path: Option<PathBuf>,
    tree: Value,
    project: Project,
    addresses: Addresses,
    types: mxprj::types::Types,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// An open take (`begin_take`): its name and where its steps start.
    take: Option<(String, usize)>,
    revision: u64,
    saved_revision: u64,
    /// Object ids for new objects (copies): above every id in the file.
    next_oid: u64,
    /// The patterns (group, pattern) a length was chosen for this session; the others'
    /// is Auto (Maschine keeps this outside the project too).
    chosen_lengths: std::collections::HashSet<(usize, usize)>,
    /// Notes' expression (MPE), by `ExprId`: only ever added to (see `expression.rs`).
    expressions: Vec<std::sync::Arc<deckard_model::Expression>>,
    /// Channel pressure saved in Maschine's archive (kind 6) instead of the DECKARD
    /// block: off until real Maschine is shown to read it (see `block.rs`).
    native_pressure: bool,
    /// Where the project was left, kept in the DECKARD block ([`SavedScreen`]).
    screen: Option<SavedScreen>,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("path", &self.path)
            .field("name", &self.project.name)
            .field("revision", &self.revision)
            .field("dirty", &self.is_dirty())
            .finish_non_exhaustive()
    }
}

impl Document {
    pub fn open(path: &FsPath) -> Result<Document, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Document::from_bytes(bytes, Some(path.to_path_buf()))
    }

    pub fn from_bytes(file: Vec<u8>, path: Option<PathBuf>) -> Result<Document, String> {
        let types = mxprj::types::Types::new();
        let root = {
            let doc = mxprj::Project::parse(&file).map_err(|e| e.to_string())?;
            if doc.kind() != Some(*b"PROJ") {
                return Err("not a project (.mxprj) file".into());
            }
            doc.decode(&types).map_err(|(e, at)| format!("{e} in {at}"))?
        };
        let (project, addresses) = Project::read(&root);
        let mut max_oid = 0;
        root.walk_paths(&mut |o, _| max_oid = max_oid.max(o.oid.unwrap_or(0)));
        let mut d = Document {
            file,
            path,
            tree: Value::Obj(root),
            project,
            addresses,
            types,
            undo: Vec::new(),
            redo: Vec::new(),
            take: None,
            revision: 0,
            saved_revision: 0,
            next_oid: 0,
            chosen_lengths: Default::default(),
            expressions: Vec::new(),
            native_pressure: false,
            screen: None,
        };
        d.next_oid = max_oid + 1;
        d.read_block();
        Ok(d)
    }

    pub fn shared(self) -> Shared {
        Rc::new(RefCell::new(self))
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn addresses(&self) -> &Addresses {
        &self.addresses
    }

    pub fn root(&self) -> &Obj {
        match &self.tree {
            Value::Obj(o) => o,
            _ => unreachable!("the tree root is an object"),
        }
    }

    pub fn path(&self) -> Option<&FsPath> {
        self.path.as_deref()
    }

    /// Goes up with every change, undo and redo.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Changed since opened or last saved.
    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// Count as changed since last saved (a project brought back with unsaved edits).
    pub fn mark_unsaved(&mut self) {
        self.saved_revision = self.revision.wrapping_sub(1);
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }

    /// The steps that can be undone, most recent first.
    pub fn undo_labels(&self) -> Vec<&str> {
        self.undo.iter().rev().map(|s| s.label.as_str()).collect()
    }

    /// The steps that can be redone, next first.
    pub fn redo_labels(&self) -> Vec<&str> {
        self.redo.iter().rev().map(|s| s.label.as_str()).collect()
    }

    // ---- changing parameters ---------------------------------------------------

    fn owner(&self, t: Target) -> Option<Path> {
        let a = &self.addresses;
        match t {
            Target::Project => Some(Vec::new()),
            Target::Song => a.song.clone(),
            Target::Group(g) => a.groups.get(g).map(|g| g.group.clone()),
            Target::GroupStrip(g) => a.groups.get(g).map(|g| g.strip.clone()),
            Target::Sound(g, s) => a.groups.get(g)?.sounds.get(s).cloned(),
            Target::Sampler(g, s) => {
                let sound = a.groups.get(g)?.sounds.get(s)?;
                let Some(Value::Obj(snd)) = self.tree.at(sound) else { return None };
                // The first Sampler module in the sound's chain.
                let mut found = None;
                snd.walk_paths(&mut |o, p| {
                    if found.is_none() && o.type_name == "NI::MASCOMMON::DATA::Sampler" {
                        found = Some(p.to_vec());
                    }
                });
                Some([sound.as_slice(), &found?].concat())
            }
            Target::Zone(g, s, z) => {
                let sampler = self.owner(Target::Sampler(g, s))?;
                let items = self.field_in(&sampler, "zones/objects/items")?;
                let at = [sampler.as_slice(), &items, &[z as u32]].concat();
                let Some(Value::Obj(item)) = self.tree.at(&at) else { return None };
                // A shared_ptr: the zone is its `px`.
                let px = item.fields.iter().position(|(n, _)| n == "px")?;
                Some([at.as_slice(), &[px as u32]].concat())
            }
            Target::Take(g, s, k) => {
                let sound = a.groups.get(g)?.sounds.get(s)?;
                let Some(Value::Obj(snd)) = self.tree.at(sound) else { return None };
                let mut found = None;
                snd.walk_paths(&mut |o, p| {
                    if found.is_none() && o.type_name == "NI::MASCOMMON::DATA::AudioModule" {
                        found = Some(p.to_vec());
                    }
                });
                let module = [sound.as_slice(), &found?].concat();
                let items = self.field_in(&module, "takes/objects/items")?;
                let at = [module.as_slice(), &items, &[k as u32]].concat();
                let Some(Value::Obj(item)) = self.tree.at(&at) else { return None };
                let px = item.fields.iter().position(|(n, _)| n == "px")?;
                Some([at.as_slice(), &[px as u32]].concat())
            }
            Target::Module(g, s, i) => self.owner(Target::ChainModule(chain::Chain::Sound(g, s), i)),
            Target::ChainModule(c, i) => {
                let chain = self.chain_of(c).ok()?;
                let items = self.field_path(&chain, &["slots", "objects", "items"]).ok()?;
                // shared_ptr<Slot> → Slot → module (shared_ptr) → the module.
                let mut path = [items.as_slice(), &[i as u32]].concat();
                for k in ["px", "module", "px"] {
                    let Some(Value::Obj(o)) = self.tree.at(&path) else { return None };
                    path.push(o.fields.iter().position(|(n, _)| n == k)? as u32);
                }
                Some(path)
            }
            Target::Pattern(g, p) if p >= clips::CLIP => a.groups.get(g)?.clip_patterns.get(p - clips::CLIP).cloned(),
            Target::Pattern(g, p) => a.groups.get(g)?.patterns.get(p).cloned(),
            Target::Scene(s) => a.scenes.get(s).cloned(),
            Target::Section(s) => a.sections.get(s).cloned(),
        }
    }

    /// Where parameter `field` of `target` is (relative to its owner), following
    /// `a/b` paths through sub-objects.
    fn field_in(&self, owner: &Path, field: &str) -> Option<Path> {
        let mut path = Vec::new();
        for seg in field.split('/') {
            let Some(Value::Obj(o)) = self.tree.at(&[owner.as_slice(), &path].concat()) else { return None };
            path.extend(o.find_path(seg)?);
        }
        Some(path)
    }

    /// Path of `field.value` of the parameter `field` on `target`'s object.
    pub fn param_path(&self, target: Target, field: &str) -> Result<Path, String> {
        let owner = self.owner(target).ok_or_else(|| format!("no {target:?} in this project"))?;
        let fp = self.field_in(&owner, field).ok_or_else(|| format!("{target:?} has no {field}"))?;
        let Some(Value::Obj(param)) = self.tree.at(&[owner.as_slice(), &fp].concat()) else {
            return Err(format!("{target:?}.{field} is shared with another object and can't be changed here"));
        };
        let vp = param.find_path("value").ok_or_else(|| format!("{target:?}.{field} has no value"))?;
        Ok([owner.as_slice(), &fp, &vp].concat())
    }

    /// The value of parameter `field` of `target`, as stored.
    pub fn value(&self, target: Target, field: &str) -> Option<&Value> {
        self.tree.at(&self.param_path(target, field).ok()?)
    }

    /// A number parameter (or a switch, 0/1; or a choice, its index).
    pub fn num(&self, target: Target, field: &str) -> Option<f64> {
        match self.value(target, field)? {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// A text parameter.
    pub fn text(&self, target: Target, field: &str) -> Option<&str> {
        match self.value(target, field)? {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Set the parameter `field` of `target` (as one undo step called `label`).
    /// The value is converted to the kind stored (integer or float).
    pub fn set(&mut self, label: &str, target: Target, field: &str, value: Value) -> Result<(), String> {
        let ops = self.param_ops(target, field, value)?;
        if ops.is_empty() {
            return Ok(());
        }
        let params = vec![Some((target, field.to_string())); ops.len()];
        self.apply_with(label, ops, params)
    }

    /// Set the parameter `field` of `target` as the document's starting state, not as
    /// an edit: no undo step, and a document unchanged before stays unchanged.
    pub fn set_initial(&mut self, target: Target, field: &str, value: Value) -> Result<(), String> {
        let clean = !self.is_dirty();
        let ops = self.param_ops(target, field, value)?;
        if ops.is_empty() {
            return Ok(());
        }
        self.run(&ops)?;
        self.update(&vec![Some((target, field.to_string())); ops.len()]);
        if clean {
            self.saved_revision = self.revision;
        }
        Ok(())
    }

    /// [`Document::param_ops`] for the other modules.
    pub(crate) fn param_ops_pub(&self, target: Target, field: &str, value: Value) -> Result<Vec<Op>, String> {
        self.param_ops(target, field, value)
    }

    /// The ops that set parameter `field` of `target`, converted to the kind stored
    /// (integer or float); none if it already has that value. Numeric parameters
    /// also keep `t0` (the value) and `t1` (the value before the last change), as
    /// Maschine writes them.
    fn param_ops(&self, target: Target, field: &str, value: Value) -> Result<Vec<Op>, String> {
        let path = self.param_path(target, field)?;
        let old = self.tree.at(&path).ok_or("parameter vanished")?;
        let value = match (old, value) {
            (Value::Int(_), Value::Float(f)) => Value::Int(f.round() as i64),
            (Value::Float(_), Value::Int(i)) => Value::Float(i as f64),
            (Value::Int(_), v @ Value::Int(_)) | (Value::Float(_), v @ Value::Float(_)) | (Value::Str(_), v @ Value::Str(_)) => v,
            (old, v) => return Err(format!("{target:?}.{field} holds {old:?}, can't set {v:?}")),
        };
        // The sound config's copy (Maschine restores it over the sound's own).
        let mirror = self.mirror_op(target, field, &value);
        if *old == value {
            return Ok(mirror.into_iter().collect());
        }
        let mut ops = vec![Op::Set { path: path.clone(), value: value.clone() }];
        ops.extend(mirror);
        // t0/t1 sit on the MaschineNumericParameter: the parameter itself, or one of its
        // bases (EnumParameter).
        let owner = self.owner(target).unwrap_or_default();
        if let Some(fp) = self.field_in(&owner, field)
            && let Some(Value::Obj(param)) = self.tree.at(&[owner.as_slice(), &fp].concat())
        {
            let base = [owner.as_slice(), &fp].concat();
            for (name, v) in [("t0", value), ("t1", old.clone())] {
                if let Some(tp) = param.find_path(name)
                    && param.at(&tp).is_some_and(|cur| std::mem::discriminant(cur) == std::mem::discriminant(&v))
                {
                    ops.push(Op::Set { path: [base.as_slice(), &tp].concat(), value: v });
                }
            }
        }
        Ok(ops)
    }

    /// Note Repeat's preset `k` (0–3, the NOTE REPEAT page's buttons 5–8) to (`rate`,
    /// `unit`), as the song stores it ([`deckard_model::Project::arp_presets`]); one
    /// undo step.
    pub fn set_arp_preset(&mut self, k: usize, rate: i64, unit: i64) -> Result<(), String> {
        let song = self.addresses.song.clone().ok_or("no song")?;
        let obj_at = |p: &[u32]| match self.tree.at(p) {
            Some(Value::Obj(o)) => Some(o),
            _ => None,
        };
        let presets = [song.as_slice(), &obj_at(&song).and_then(|s| s.find_path("arp_presets")).ok_or("no Note Repeat presets")?].concat();
        let items = [presets.as_slice(), &obj_at(&presets).and_then(|o| o.find_path("items")).ok_or("no Note Repeat presets")?].concat();
        let Some(Value::List(list)) = self.tree.at(&items) else { return Err("no Note Repeat presets".into()) };
        if k >= list.len() {
            return Err(format!("no Note Repeat preset {}", k + 1));
        }
        let item = [items.as_slice(), &[k as u32]].concat();
        let o = obj_at(&item).ok_or("a Note Repeat preset that isn't one")?;
        let mut ops = Vec::new();
        for (f, v) in [("i00", rate), ("i08", unit)] {
            let path = [item.as_slice(), &o.find_path(f).ok_or("a Note Repeat preset without its rate")?].concat();
            if self.tree.at(&path) != Some(&Value::Int(v)) {
                ops.push(Op::Set { path, value: Value::Int(v) });
            }
        }
        if ops.is_empty() { Ok(()) } else { self.apply("Note Repeat", ops) }
    }

    pub fn set_tempo(&mut self, bpm: f64) -> Result<(), String> {
        self.set("Tempo", Target::Song, "tempo", Value::Float(bpm.clamp(5.0, 999.0)))
    }

    /// The song's time signature: `numerator` / `denominator` (1, 2, 4, 8, 16 or 32;
    /// stored as a power of two).
    pub fn set_time_signature(&mut self, numerator: i64, denominator: i64) -> Result<(), String> {
        if !(1..=32).contains(&numerator) {
            return Err(format!("{numerator} beats per bar is out of range"));
        }
        let exp = match denominator {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            16 => 4,
            32 => 5,
            _ => return Err(format!("1/{denominator} is not a note value")),
        };
        let mut ops = self.param_ops(Target::Song, "numerator", Value::Int(numerator))?;
        ops.extend(self.param_ops(Target::Song, "denominator", Value::Int(exp))?);
        if ops.is_empty() {
            return Ok(());
        }
        self.apply("Time Signature", ops)
    }

    /// A pattern's length in ticks.
    /// Whether pattern `p` of group `g` has an Auto length (none chosen for it): recorded
    /// into from empty, it grows (docs/superpowers/plans/2026-10-23-auto-length-recording.md).
    pub fn length_is_auto(&self, g: usize, p: usize) -> bool {
        !self.chosen_lengths.contains(&(g, p))
    }

    /// Auto length on (`true`) or off (a length chosen) for pattern `p` of group `g`.
    pub fn set_length_auto(&mut self, g: usize, p: usize, auto: bool) {
        if auto {
            self.chosen_lengths.remove(&(g, p));
        } else {
            self.chosen_lengths.insert((g, p));
        }
    }

    pub fn set_pattern_length(&mut self, g: usize, p: usize, ticks: i64) -> Result<(), String> {
        let ops = self.param_ops(Target::Pattern(g, p), "length", Value::Int(ticks.max(deckard_model::PPQ / 4)))?;
        if ops.is_empty() {
            return Ok(());
        }
        self.apply("Pattern Length", ops)
    }

    /// A pattern's start (where in its notes it begins, ticks, from 0) and length, as one
    /// edit.
    pub fn set_pattern_span(&mut self, g: usize, p: usize, start: i64, length: i64) -> Result<(), String> {
        let mut ops = self.param_ops(Target::Pattern(g, p), "start", Value::Int(start.max(0)))?;
        ops.extend(self.param_ops(Target::Pattern(g, p), "length", Value::Int(length.max(deckard_model::PPQ / 4)))?);
        if ops.is_empty() {
            return Ok(());
        }
        self.apply("Pattern Start", ops)
    }

    /// The groove of a sound (`Target::Sound`), group (`Target::Group`) or the master
    /// (`Target::Song`).
    pub fn set_swing(&mut self, target: Target, swing: deckard_model::Swing) -> Result<(), String> {
        let mut ops = self.param_ops(target, "swing_amount", Value::Float(swing.amount.clamp(0.0, 100.0)))?;
        ops.extend(self.param_ops(target, "swing_cycle", Value::Int(swing.cycle.clamp(0, 7)))?);
        ops.extend(self.param_ops(target, "swing_invert", Value::Int(swing.invert as i64))?);
        if ops.is_empty() {
            return Ok(());
        }
        self.apply("Swing", ops)
    }

    pub fn rename(&mut self, target: Target, name: &str) -> Result<(), String> {
        self.set("Rename", target, "name", Value::Str(name.to_string()))
    }

    /// Level (stored: 0..1, 0.75 = 0 dB) of a group (`Target::Group`) or sound.
    pub fn set_level(&mut self, target: Target, level: f64) -> Result<(), String> {
        self.set("Level", strip(target), "level", Value::Float(level.clamp(0.0, 1.0)))
    }

    pub fn set_pan(&mut self, target: Target, pan: f64) -> Result<(), String> {
        self.set("Pan", strip(target), "pan", Value::Float(pan.clamp(-1.0, 1.0)))
    }

    pub fn set_mute(&mut self, target: Target, mute: bool) -> Result<(), String> {
        self.set(if mute { "Mute" } else { "Unmute" }, strip(target), "mute", Value::Int(mute as i64))
    }

    /// Mute and unmute several channels as one step (solo: the others are muted).
    pub fn set_mutes(&mut self, label: &str, mutes: &[(Target, bool)]) -> Result<(), String> {
        let mut ops = Vec::new();
        let mut params = Vec::new();
        for (t, m) in mutes {
            let t = strip(*t);
            let o = self.param_ops(t, "mute", Value::Int(*m as i64))?;
            params.extend(std::iter::repeat_n(Some((t, "mute".to_string())), o.len()));
            ops.extend(o);
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.apply_with(label, ops, params)
    }

    /// Send a group or sound to the cue bus, or stop.
    pub fn set_cue(&mut self, target: Target, cue: bool) -> Result<(), String> {
        self.set("Cue", strip(target), "cue_enabled", Value::Int(cue as i64))
    }

    // ---- applying, undo, redo ------------------------------------------------------

    /// Apply `ops` as one undo step. Repeated changes to the same values within a
    /// moment (dragging a knob) extend the previous step instead.
    pub fn apply(&mut self, label: &str, ops: Vec<Op>) -> Result<(), String> {
        let params = vec![None; ops.len()];
        self.apply_with(label, ops, params)
    }

    fn apply_with(&mut self, label: &str, mut ops: Vec<Op>, mut params: Vec<Option<(Target, String)>>) -> Result<(), String> {
        let mut undo = self.run(&ops)?;
        // An edit that took away an object's definition: anything still referring to
        // it keeps it (see `rehomed`).
        if undo.iter().any(defines_objects) {
            self.run(&undo).expect("undo of ops just applied");
            ops = self.rehomed(label, ops)?;
            params.resize(ops.len(), None);
            undo = self.run(&ops)?;
        }
        let now = Instant::now();
        let same_targets =
            |s: &Step| s.label == label && s.redo.len() == ops.len() && s.redo.iter().zip(&ops).all(|(a, b)| op_target(a) == op_target(b));
        match self.undo.last_mut() {
            Some(last) if same_targets(last) && now.duration_since(last.at) < MERGE_WINDOW => {
                last.redo = ops;
                last.at = now;
            }
            _ => self.undo.push(Step::new(label, undo, ops, params.clone(), now)),
        }
        self.redo.clear();
        self.update(&params);
        Ok(())
    }

    /// Give the last undo step another name (an edit made by another, e.g. Cut is a
    /// Reset after a copy).
    pub fn relabel_last(&mut self, label: &str) {
        if let Some(last) = self.undo.last_mut() {
            last.label = label.to_string();
        }
    }

    /// Run several edits as one undo step labelled `label`. If `f` fails, its edits
    /// are taken back and the error returned.
    pub fn transaction(&mut self, label: &str, f: impl FnOnce(&mut Document) -> Result<(), String>) -> Result<(), String> {
        // The first edit must not merge into the step before.
        if let Some(last) = self.undo.last_mut() {
            last.at = Instant::now() - MERGE_WINDOW;
        }
        let before = self.undo.len();
        let redo = self.redo.clone();
        let result = f(self);
        let steps: Vec<Step> = self.undo.drain(before..).collect();
        if result.is_err() {
            for step in steps.iter().rev() {
                let _ = self.run(&step.undo);
            }
            self.redo = redo;
            self.refresh();
            return result;
        }
        if steps.is_empty() {
            return Ok(());
        }
        self.undo.push(Step::merged(label, &steps));
        Ok(())
    }

    /// Start a take (a recording): the edits until [`Document::end_take`] undo as one
    /// step named `label`.
    pub fn begin_take(&mut self, label: &str) {
        self.end_take();
        if let Some(last) = self.undo.last_mut() {
            last.at = Instant::now() - MERGE_WINDOW;
        }
        self.take = Some((label.to_string(), self.undo.len()));
    }

    /// End the take begun: its edits become one undo step (none if it made none).
    pub fn end_take(&mut self) {
        let Some((label, start)) = self.take.take() else { return };
        if self.undo.len() <= start {
            return;
        }
        let steps: Vec<Step> = self.undo.drain(start..).collect();
        let mut take = Step::merged(&label, &steps);
        take.parts = steps;
        self.undo.push(take);
    }

    /// An edit of the user's own (a quantize, say) while a take is open: its own undo
    /// step, between the take's edits before and after it (which become two steps).
    pub fn outside_take<R>(&mut self, f: impl FnOnce(&mut Document) -> Result<R, String>) -> Result<R, String> {
        let Some((label, _)) = self.take.clone() else { return f(self) };
        self.end_take();
        if let Some(last) = self.undo.last_mut() {
            last.at = Instant::now() - MERGE_WINDOW;
        }
        let r = f(self);
        self.begin_take(&label);
        r
    }

    pub fn undo(&mut self) -> bool {
        // An undo in the middle of a take ends it first.
        self.end_take();
        let Some(step) = self.undo.pop() else { return false };
        if self.run(&step.undo).is_err() {
            return false;
        }
        let params = step.params.clone();
        self.redo.push(step);
        self.update(&params);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(mut step) = self.redo.pop() else { return false };
        if self.run(&step.redo).is_err() {
            return false;
        }
        step.at = Instant::now() - MERGE_WINDOW; // don't merge into a redone step
        let params = step.params.clone();
        match self.undo.last_mut() {
            // An edit of a take, back where step undo took it from.
            Some(take) if step.of_take && !take.parts.is_empty() => {
                step.of_take = false;
                take.parts.push(step);
                let parts = std::mem::take(&mut take.parts);
                *take = Step::merged(&take.label, &parts);
                take.parts = parts;
            }
            _ => self.undo.push(step),
        }
        self.update(&params);
        true
    }

    /// Undo one edit (⌘⇧Z): the last edit of a take, not the whole take, else as
    /// [`Document::undo`].
    pub fn undo_step(&mut self) -> bool {
        self.end_take();
        let Some(take) = self.undo.last_mut() else { return false };
        if take.parts.len() < 2 {
            return self.undo();
        }
        let mut part = take.parts.pop().expect("parts");
        let parts = std::mem::take(&mut take.parts);
        let label = take.label.clone();
        if self.run(&part.undo).is_err() {
            let take = self.undo.last_mut().expect("the take");
            take.parts = parts;
            take.parts.push(part);
            return false;
        }
        let take = self.undo.last_mut().expect("the take");
        *take = Step::merged(&label, &parts);
        take.parts = parts;
        part.of_take = true;
        let params = part.params.clone();
        self.redo.push(part);
        self.update(&params);
        true
    }

    /// Redo one edit (⌘⇧Y).
    pub fn redo_step(&mut self) -> bool {
        self.redo()
    }

    /// Apply ops in order; returns their inverses (to apply in order to undo).
    /// Nothing is applied if any op fails.
    fn run(&mut self, ops: &[Op]) -> Result<Vec<Op>, String> {
        // In order (an op may set a value inside one an earlier op put there); if one
        // has nowhere to go, those before it are taken back.
        let mut inverse = Vec::with_capacity(ops.len());
        for op in ops {
            match run_one(&mut self.tree, op) {
                Some(undo) => inverse.push(undo),
                None => {
                    for undo in inverse.iter().rev() {
                        let _ = run_one(&mut self.tree, undo);
                    }
                    return Err(format!("no value at {:?}", op_path(op)));
                }
            }
        }
        inverse.reverse();
        Ok(inverse)
    }

    /// Bring the model up to date after ops: patch it in place when every op set a
    /// known parameter, else rebuild it from the tree.
    fn update(&mut self, params: &[Option<(Target, String)>]) {
        let patched = params.iter().all(|p| match p {
            Some((target, field)) => self.patch(*target, field),
            None => false,
        });
        if patched {
            self.revision += 1;
        } else {
            self.refresh();
        }
    }

    /// Copy a parameter's value from the tree into the model, for parameters that
    /// nothing else in the model derives from. False if it has to be rebuilt.
    fn patch(&mut self, target: Target, field: &str) -> bool {
        let Ok(path) = self.param_path(target, field) else { return false };
        let value = match self.tree.at(&path) {
            Some(v) => v.clone(),
            None => return false,
        };
        let num = match value {
            Value::Float(f) => Some(f),
            Value::Int(i) => Some(i as f64),
            _ => None,
        };
        let text = match &value {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        };
        let p = &mut self.project;
        match (target, field, num, text) {
            (Target::Song, "tempo", Some(n), _) => p.tempo = n,
            (Target::Song, "level", Some(n), _) => p.master_level = n,
            (Target::Song, "pan", Some(n), _) => p.master_pan = n,
            (Target::Song, "sounds_visible_mixer", Some(n), _) => p.mixer_sounds = n != 0.0,
            (Target::GroupStrip(g), "cue_enabled", Some(n), _) => p.groups[g].cue = n != 0.0,
            (Target::Sound(g, s), "cue_enabled", Some(n), _) => p.groups[g].sounds[s].cue = n != 0.0,
            (Target::GroupStrip(g), "aux1_level", Some(n), _) => p.groups[g].aux[0] = n,
            (Target::GroupStrip(g), "aux2_level", Some(n), _) => p.groups[g].aux[1] = n,
            (Target::Sound(g, s), "aux1_level", Some(n), _) => p.groups[g].sounds[s].aux[0] = n,
            (Target::Sound(g, s), "aux2_level", Some(n), _) => p.groups[g].sounds[s].aux[1] = n,
            (Target::Project, "name", _, Some(t)) => p.name = t,
            (Target::Group(g), "name", _, Some(t)) => p.groups[g].name = t,
            (Target::GroupStrip(g), "level", Some(n), _) => p.groups[g].level = n,
            (Target::GroupStrip(g), "pan", Some(n), _) => p.groups[g].pan = n,
            (Target::GroupStrip(g), "mute", Some(n), _) => p.groups[g].muted = n != 0.0,
            (Target::Sound(g, s), "name", _, Some(t)) => p.groups[g].sounds[s].name = t,
            (Target::Sound(g, s), "level", Some(n), _) => p.groups[g].sounds[s].level = n,
            (Target::Sound(g, s), "pan", Some(n), _) => p.groups[g].sounds[s].pan = n,
            (Target::Sound(g, s), "mute", Some(n), _) => p.groups[g].sounds[s].muted = n != 0.0,
            (Target::Pattern(g, i), "name", _, Some(t)) if i < clips::CLIP => p.groups[g].patterns[i].name = t,
            (Target::Scene(s), "name", _, Some(t)) => p.scenes[s].name = t,
            (Target::Zone(g, s, z), f, Some(n), _) => {
                let zone = p.groups[g].sounds[s].sampler.as_mut().and_then(|smp| smp.zones.get_mut(z));
                if !zone.is_some_and(|zone| zone.set(f, n)) {
                    return false;
                }
            }
            (Target::Sampler(g, s), f, Some(n), _) if deckard_model::SAMPLER_PARAMS.iter().any(|(k, _)| *k == f) => {
                match &mut p.groups[g].sounds[s].sampler {
                    Some(smp) => smp.set(f, n),
                    None => return false,
                }
            }
            _ => return false,
        }
        true
    }

    fn refresh(&mut self) {
        let (project, addresses) = Project::read(self.root());
        self.project = Project { expressions: self.expressions.clone(), ..project };
        self.addresses = addresses;
        self.revision += 1;
    }

    // ---- saving ----------------------------------------------------------------------

    /// The project file as it would be saved now.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let doc = mxprj::Project::parse(&self.file).map_err(|e| e.to_string())?;
        let mut root = Value::Obj(self.root().clone());
        if !self.native_pressure {
            block::strip_channel_pressure(&mut root);
        }
        expression::strip_deckard_fields(&mut root);
        let Value::Obj(root) = root else { unreachable!() };
        let archived = doc.encode(&self.types, root).map_err(|(e, at)| format!("{e} in {at}"))?;
        self.with_block(&archived)
    }

    /// Write the project to `path` (through a temporary file, so a failed write
    /// leaves the old file intact). It becomes the document's path.
    pub fn save_as(&mut self, path: &FsPath) -> Result<(), String> {
        let bytes = self.to_bytes()?;
        let tmp = path.with_extension("mxprj.saving");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.file = bytes;
        self.path = Some(path.to_path_buf());
        self.saved_revision = self.revision;
        Ok(())
    }
}

/// Groups keep level, pan and mute on their strip.
fn strip(t: Target) -> Target {
    match t {
        Target::Group(g) => Target::GroupStrip(g),
        t => t,
    }
}

/// Whether an op's value holds an object's definition (one with an id others may
/// refer back to). Run on an edit's undo: what it replaced.
fn defines_objects(op: &Op) -> bool {
    fn has(v: &Value) -> bool {
        match v {
            Value::Obj(o) => o.oid.is_some() || o.fields.iter().any(|(_, f)| has(f)),
            Value::List(items) => items.iter().any(has),
            _ => false,
        }
    }
    match op {
        Op::Set { value, .. } | Op::SetField { value: Some(value), .. } => has(value),
        Op::SetField { value: None, .. } => false,
    }
}

fn op_path(op: &Op) -> &Path {
    match op {
        Op::Set { path, .. } | Op::SetField { path, .. } => path,
    }
}

/// What an op changes, for merging quick edits: its path, and the field it sets.
fn op_target(op: &Op) -> (&Path, Option<&str>) {
    match op {
        Op::Set { path, .. } => (path, None),
        Op::SetField { path, field, .. } => (path, Some(field)),
    }
}

/// Apply one op to `tree`; its inverse, or `None` if it has nowhere to go.
fn run_one(tree: &mut Value, op: &Op) -> Option<Op> {
    match op {
        Op::Set { path, value } => {
            let slot = tree.at_mut(path)?;
            let old = std::mem::replace(slot, value.clone());
            Some(Op::Set { path: path.clone(), value: old })
        }
        Op::SetField { path, field, value } => {
            let Value::Obj(o) = tree.at_mut(path)? else { return None };
            let at = o.fields.iter().position(|(f, _)| f == field);
            let old = at.map(|i| o.fields[i].1.clone());
            match (at, value) {
                (Some(i), Some(v)) => o.fields[i].1 = v.clone(),
                (Some(i), None) => {
                    o.fields.remove(i);
                }
                (None, Some(v)) => o.fields.push((field.clone(), v.clone())),
                (None, None) => {}
            }
            Some(Op::SetField { path: path.clone(), field: field.clone(), value: old })
        }
    }
}

#[cfg(test)]
mod tests {

    /// A recording take: every edit between `begin_take` and `end_take`, however far
    /// apart in time, undoes as one step with the take's name.
    #[test]
    fn a_take_undoes_as_one_step() {
        let Ok(mut d) = Document::open(std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj")) else {
            return;
        };
        let before = d.project().groups[0].patterns[0].notes.clone();
        let steps = d.undo_labels().len();
        d.begin_take("Record");
        for k in 0..3 {
            let note = deckard_model::Note {
                expr: None,
                sound: 2,
                position: 100 + k * 960,
                length: 240,
                pitch: 60,
                velocity: 90,
                probability: 100,
            };
            let old: Vec<_> = d.project().groups[0].patterns[0].notes.iter().filter(|n| n.sound == 2).copied().collect();
            let mut new = old.clone();
            new.push(note);
            d.edit_notes("Record", 0, 0, &old, &new).unwrap();
            // Edits further apart than the merge window.
            if let Some(last) = d.undo.last_mut() {
                last.at -= MERGE_WINDOW * 2;
            }
        }
        d.end_take();
        assert_eq!(d.undo_labels().len(), steps + 1);
        assert_eq!(d.undo_label(), Some("Record"));
        assert_eq!(d.project().groups[0].patterns[0].notes.len(), before.len() + 3);
        assert!(d.undo());
        assert_eq!(d.project().groups[0].patterns[0].notes, before);
        // An empty take leaves no step.
        d.begin_take("Record");
        d.end_take();
        assert_eq!(d.undo_labels().len(), steps);
    }
    use super::*;
    use deckard_model::Swing;

    /// #7: an edit made while recording (quantizing, say) is its own step, between the
    /// take's notes before it and those after: undo takes back only the later notes.
    #[test]
    fn an_edit_during_a_take_is_its_own_step() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "P").unwrap();
        let note =
            |position| deckard_model::Note { expr: None, sound: 0, position, length: 240, pitch: 60, velocity: 90, probability: 100 };
        let positions = |d: &Document| d.project().groups[0].patterns[0].notes.iter().map(|n| n.position).collect::<Vec<_>>();
        d.begin_take("Record");
        d.edit_notes("Record", 0, 0, &[], &[note(10)]).unwrap();
        d.outside_take(|d| d.edit_notes("Quantize", 0, 0, &[note(10)], &[note(0)])).unwrap();
        d.edit_notes("Record", 0, 0, &[], &[note(960)]).unwrap();
        d.end_take();
        assert_eq!(positions(&d), [0, 960]);
        assert!(d.undo());
        assert_eq!(positions(&d), [0], "the later notes only");
        assert!(d.undo());
        assert_eq!(positions(&d), [10], "then the quantize");
        assert!(d.undo());
        assert!(positions(&d).is_empty(), "then the first notes");
    }

    /// Step undo (⌘⇧Z, SHIFT + pad 3) takes back one edit, inside a take too; take undo
    /// (⌘Z) the rest of the take; step redo puts edits back one at a time.
    #[test]
    fn step_undo_takes_back_one_edit_of_a_take() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "P").unwrap();
        let notes = |d: &Document| d.project().groups[0].patterns[0].notes.len();
        let steps = d.undo_labels().len();
        d.begin_take("Record");
        for k in 0..3 {
            let note =
                deckard_model::Note { expr: None, sound: 0, position: k * 960, length: 240, pitch: 60, velocity: 90, probability: 100 };
            d.add_note(0, 0, note).unwrap();
            if let Some(last) = d.undo.last_mut() {
                last.at -= MERGE_WINDOW * 2;
            }
        }
        d.end_take();
        assert_eq!(notes(&d), 3);
        assert!(d.undo_step());
        assert_eq!(notes(&d), 2, "one edit of the take");
        assert_eq!(d.undo_labels().len(), steps + 1, "the take's rest is still a step");
        assert!(d.redo_step());
        assert_eq!(notes(&d), 3);
        assert!(d.undo_step() && d.undo_step());
        assert_eq!(notes(&d), 1);
        assert!(d.undo());
        assert_eq!(notes(&d), 0, "take undo: the rest of the take");
        assert_eq!(d.undo_labels().len(), steps);
        // Redo brings each edit back on its own, then the take's rest at once.
        assert!(d.redo());
        assert_eq!(notes(&d), 1);
        assert!(d.redo() && d.redo());
        assert_eq!(notes(&d), 3);
        // Redone edits rejoin their take: take undo still takes all of it back.
        assert_eq!(d.undo_label(), Some("Record"));
        assert!(d.undo());
        assert_eq!(notes(&d), 0);
        assert!(d.redo());
        // Outside a take, a step is an undo.
        d.add_note(0, 0, deckard_model::Note { expr: None, sound: 1, position: 0, length: 240, pitch: 60, velocity: 90, probability: 100 })
            .unwrap();
        assert_eq!(notes(&d), 4);
        assert!(d.undo_step());
        assert_eq!(notes(&d), 3);
        assert_eq!(d.undo_label(), Some("Record"));
    }

    fn open() -> Option<Document> {
        let path = std::env::var("DECKARD_TEST_PROJECT")
            .unwrap_or_else(|_| "/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj".into());
        match Document::open(FsPath::new(&path)) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("{e}; skipping");
                None
            }
        }
    }

    /// The model, however it was updated, equals one built from the tree.
    fn assert_consistent(d: &Document) {
        let (p, a) = Project::read(d.root());
        assert!(p == *d.project(), "patched model differs from a rebuild");
        assert!(a == *d.addresses());
    }

    #[test]
    fn unchanged_document_saves_identically() {
        let Some(d) = open() else { return };
        let original = std::fs::read(d.path().unwrap()).unwrap();
        assert_eq!(d.to_bytes().unwrap(), original);
        assert!(!d.is_dirty());
    }

    #[test]
    fn new_pattern_survives_save_and_reload() {
        let Some(mut d) = open() else { return };
        let n = d.project().groups[0].patterns.len();
        let len = d.project().groups[0].patterns[0].length;
        let i = d.add_pattern(0, 0, "Recording 1").unwrap();
        assert_eq!(i, n);
        let p = d.project().groups[0].patterns[i].clone();
        assert_eq!((p.name.as_str(), p.length), ("Recording 1", len));
        // Empty, and its notes are its own.
        assert!(p.notes.is_empty());
        let before = d.project().groups[0].patterns[0].notes.clone();
        let note = deckard_model::Note { expr: None, sound: 0, position: p.start, length: 960, pitch: 60, velocity: 100, probability: 100 };
        d.edit_notes("Add", 0, i, &[], &[note]).unwrap();
        assert_eq!(d.project().groups[0].patterns[i].notes, vec![note]);
        assert_eq!(d.project().groups[0].patterns[0].notes, before);
        assert_consistent(&d);
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let file = dir.join("p.mxprj");
        std::fs::write(&file, d.to_bytes().unwrap()).unwrap();
        let back = Document::open(&file).unwrap();
        assert_eq!(back.project().groups[0].patterns.len(), n + 1);
        assert_eq!(back.project().groups[0].patterns[i].name, "Recording 1");
        assert_eq!(back.project().groups[0].patterns[i].notes.len(), 1);
        assert!(d.undo() && d.undo());
        assert_eq!(d.project().groups[0].patterns.len(), n);
    }

    #[test]
    fn edits_undo_and_redo() {
        let Some(mut d) = open() else { return };
        let (tempo, level) = (d.project().tempo, d.project().groups[0].level);
        let t0 = Instant::now();
        d.set_tempo(97.5).unwrap();
        eprintln!("one edit (apply + update the model): {:?}", t0.elapsed());
        assert_eq!(d.project().tempo, 97.5);
        assert_consistent(&d);
        assert!(d.is_dirty());
        d.set_level(Target::Group(0), 0.5).unwrap();
        d.set_mute(Target::Sound(0, 0), true).unwrap();
        d.rename(Target::Group(1), "Renamed").unwrap();
        assert_eq!(d.project().groups[0].level, 0.5);
        assert!(d.project().groups[0].sounds[0].muted);
        assert_eq!(d.project().groups[1].name, "Renamed");
        assert_consistent(&d);

        assert!(d.undo() && d.undo() && d.undo());
        assert_consistent(&d);
        assert_eq!(d.project().groups[0].level, level);
        assert!(!d.project().groups[0].sounds[0].muted);
        assert_eq!(d.project().tempo, 97.5);
        assert!(d.undo());
        assert_eq!(d.project().tempo, tempo);
        assert!(!d.undo());
        assert!(d.redo());
        assert_eq!(d.project().tempo, 97.5);
        assert_eq!(d.redo_label(), Some("Level"));
        assert_consistent(&d);
        // Ops without a known parameter rebuild the model.
        let path = d.param_path(Target::Song, "tempo").unwrap();
        d.apply("Raw", vec![Op::Set { path, value: Value::Float(120.0) }]).unwrap();
        assert_eq!(d.project().tempo, 120.0);
        assert_consistent(&d);
    }

    #[test]
    fn knob_turns_merge_into_one_step() {
        let Some(mut d) = open() else { return };
        let tempo = d.project().tempo;
        for bpm in [100.0, 101.0, 102.0, 103.0] {
            d.set_tempo(bpm).unwrap();
        }
        d.set_level(Target::Group(0), 0.25).unwrap();
        assert!(d.undo());
        assert_eq!(d.project().tempo, 103.0);
        assert!(d.undo());
        assert_eq!(d.project().tempo, tempo, "four tempo changes were one step");
        assert!(!d.undo());
    }

    #[test]
    fn time_signature_pattern_length_and_swing() {
        let Some(mut d) = open() else { return };
        d.set_time_signature(3, 4).unwrap();
        assert_eq!((d.project().numerator, d.project().denominator), (3, 4));
        assert_eq!(d.project().bar_length(), 3 * 960);
        d.set_time_signature(6, 8).unwrap();
        assert_eq!((d.project().numerator, d.project().denominator), (6, 8));
        assert!(d.set_time_signature(4, 3).is_err());
        d.set_pattern_length(0, 0, 2 * 3840).unwrap();
        assert_eq!(d.project().groups[0].patterns[0].length, 2 * 3840);
        let swing = deckard_model::Swing { amount: 40.0, cycle: 2, invert: true };
        d.set_swing(Target::Group(0), swing).unwrap();
        d.set_swing(Target::Sound(0, 1), swing).unwrap();
        d.set_swing(Target::Song, Swing { amount: 10.0, ..Default::default() }).unwrap();
        assert_eq!(d.project().groups[0].swing, swing);
        assert_eq!(d.project().groups[0].sounds[1].swing, swing);
        assert_eq!(d.project().swing.amount, 10.0);
        assert_consistent(&d);
        // Saved and read back.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let out = dir.join("ts.mxprj");
        d.save_as(&out).unwrap();
        let again = Document::open(&out).unwrap();
        let p = again.project();
        assert_eq!((p.numerator, p.denominator, p.groups[0].patterns[0].length), (6, 8, 2 * 3840));
        assert_eq!((p.groups[0].swing, p.swing.amount), (swing, 10.0));
        // All undoable.
        while d.undo() {}
        assert_eq!((d.project().numerator, d.project().denominator), (4, 4));
    }

    #[test]
    fn saved_edits_read_back() {
        let Some(mut d) = open() else { return };
        d.set_tempo(88.0).unwrap();
        d.set_pan(Target::Sound(0, 0), -0.5).unwrap();
        d.rename(Target::Scene(0), "First").unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let out = dir.join("saved.mxprj");
        d.save_as(&out).unwrap();
        assert!(!d.is_dirty());
        let again = Document::open(&out).unwrap();
        let p = again.project();
        assert_eq!(p.tempo, 88.0);
        assert_eq!(p.groups[0].sounds[0].pan, -0.5);
        assert_eq!(p.scenes[0].name, "First");
        assert_eq!(p.groups.len(), d.project().groups.len());
    }

    /// Names with emoji (beyond UTF-16's first plane, and joined sequences) are kept.
    #[test]
    fn emoji_names_save_and_read_back() {
        let Some(mut d) = open() else { return };
        let (group, sound, scene) = ("🥁 Drums", "Kick 🔥", "🌙 Intro 👩‍🎤");
        d.rename(Target::Group(0), group).unwrap();
        d.rename(Target::Sound(0, 0), sound).unwrap();
        d.rename(Target::Scene(0), scene).unwrap();
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        let p = back.project();
        assert_eq!((p.groups[0].name.as_str(), p.groups[0].sounds[0].name.as_str(), p.scenes[0].name.as_str()), (group, sound, scene));
    }
}
