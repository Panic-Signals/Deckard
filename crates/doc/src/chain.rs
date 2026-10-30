//! A sound's plug-in chain: adding, removing and bypassing slots.
//!
//! A `Chain` is an `ObjectVector<Slot>` (each item a `shared_ptr<Slot>`: `active`, the
//! `module`, `sound_info`) followed by one `quick_browse` handle per slot. New slots are
//! built from scratch (`crate::build`), with the module at Maschine's defaults: a
//! Sampler with no zones, an Audio module with no takes, an effect or instrument at its
//! default settings, a plug-in host with the plug-in's id, name and state.
//! Removing a slot removes its automation too (`crate::automation`).

use crate::{Document, Op};

/// A plug-in chain: a sound's, a group's or the master's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Chain {
    Sound(usize, usize),
    Group(usize),
    Master,
}
use deckard_model::SAMPLER_PARAMS;
use mxprj::decode::{Obj, Path, Value};

const SLOT: &str = "NI::MASCOMMON::DATA::Slot";
const SLOT_PTR: &str = "std::shared_ptr<NI::MASCOMMON::DATA::Slot>";
const QUICK_BROWSE: &str = "NI::DB3::QuickBrowseHandle";

/// What to put in a new slot.
#[derive(Debug, Clone)]
pub enum NewModule {
    /// Maschine's Sampler, empty.
    Sampler,
    /// Maschine's Audio module (audio loops), empty.
    Audio,
    /// A plug-in: Maschine's id (`"Native Instruments:VST3:INSTR:<cid>"`), its name, and
    /// its state as Maschine stores it (may be empty: the plug-in's defaults).
    Plugin { id: String, name: String, chunk: Vec<u8> },
    /// One of Maschine's internal effects ([`deckard_model::INTERNAL_EFFECTS`]: its
    /// class without the namespace), at its default settings.
    Effect(&'static str),
    /// One of Maschine's internal instruments (`Kick`, `Snare`, `Hihat`, `Tom`, `Perc`,
    /// `Cymbal`, `BassSynth`, `PolySynth`: the class, with or without its namespace),
    /// with its parameters' default values (`field` or `a/b` paths as the `mxprj`
    /// loaders name them; the rest are 0).
    Instrument { class: String, params: Vec<(String, f64)> },
}

impl NewModule {
    /// The module's class.
    pub fn class(&self) -> String {
        match self {
            NewModule::Sampler => "NI::MASCOMMON::DATA::Sampler".into(),
            NewModule::Audio => "NI::MASCOMMON::DATA::AudioModule".into(),
            NewModule::Plugin { .. } => "NI::MASCHINE::DATA::PluginHost".into(),
            NewModule::Effect(c) => format!("NI::MASCHINE::DATA::{c}"),
            NewModule::Instrument { class, .. } if class.contains("::") => class.clone(),
            NewModule::Instrument { class, .. } => format!("NI::MASCHINE::DATA::{class}"),
        }
    }

    /// Whether it makes sound itself (and so goes first in a chain).
    pub fn is_instrument(&self) -> bool {
        match self {
            NewModule::Plugin { id, .. } => !(id.contains(":FX:") || id.contains(":Fx:")),
            NewModule::Effect(_) => false,
            _ => true,
        }
    }
}

/// The Audio module's settings in a new one, as Maschine makes it.
const AUDIO_DEFAULTS: &[(&str, f64)] = &[("mode", 0.0), ("fade", 0.1222), ("tune", 0.0), ("pitchbend", 0.5), ("engine", 1.0)];

/// A plug-in window's saved state before it was ever opened: format 1, no position or
/// size, the "unset" corner (-10000, -10000).
fn plugin_window() -> Vec<u8> {
    let mut v = Vec::new();
    for x in [1i32, 0, 0, 0, 0, 0, -10000, -10000, 0, 0] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v
}

/// Set a sized blob field `name` (`name` the size, `name_data` the bytes).
fn set_blob(o: &mut Obj, name: &str, bytes: &[u8]) -> Result<(), String> {
    *o.find_mut(name).ok_or_else(|| format!("no {name}"))? = Value::Int(bytes.len() as i64);
    *o.find_mut(&format!("{name}_data")).ok_or_else(|| format!("no {name}_data"))? = Value::Bytes(bytes.to_vec());
    Ok(())
}

/// The module object of a slot item (`shared_ptr<Slot>`), if it is stored there.
pub(crate) fn module_of(item: &Value) -> Option<&Obj> {
    let Value::Obj(ptr) = item else { return None };
    let Some(Value::Obj(slot)) = ptr.get("px") else { return None };
    let Some(Value::Obj(mptr)) = slot.get("module") else { return None };
    match mptr.get("px") {
        Some(Value::Obj(m)) => Some(m),
        _ => None,
    }
}

/// Set a parameter object's value (and `t0`, which mirrors it), keeping its kind.
pub(crate) fn set_param(o: &mut Obj, field: &str, v: f64) {
    let target = match field.split_once('/') {
        Some((a, b)) => match o.get_mut(a) {
            Some(Value::Obj(sub)) => return set_param(sub, b, v),
            _ => return,
        },
        None => o.find_mut(field),
    };
    if let Some(Value::Obj(p)) = target {
        // Floats are written as f32, except a double parameter's.
        let f = if p.type_name.contains("<double>") { v } else { v as f32 as f64 };
        for k in ["value", "t0"] {
            if let Some(slot) = p.find_mut(k) {
                *slot = match slot {
                    Value::Int(_) => Value::Int(v.round() as i64),
                    _ => Value::Float(f),
                };
            }
        }
    }
}

impl Document {
    pub(crate) fn chain_path(&self, g: usize, s: usize) -> Result<Path, String> {
        let sound = self.addresses.groups.get(g).and_then(|ga| ga.sounds.get(s)).ok_or("no such sound")?.clone();
        self.field_path(&sound, &["chain"])
    }

    /// Where chain `c` is.
    pub(crate) fn chain_of(&self, c: Chain) -> Result<Path, String> {
        match c {
            Chain::Sound(g, s) => self.chain_path(g, s),
            Chain::Group(g) => {
                let group = self.addresses.groups.get(g).ok_or("no such group")?.group.clone();
                self.field_path(&group, &["chain"])
            }
            Chain::Master => {
                let song = self.addresses.song.clone().ok_or("the project has no song")?;
                self.field_path(&song, &["chain"])
            }
        }
    }

    /// Chain `c`'s slots, as the model has them.
    pub fn chain_slots(&self, c: Chain) -> Option<&[deckard_model::SlotInfo]> {
        let p = &self.project;
        Some(match c {
            Chain::Sound(g, s) => &p.groups.get(g)?.sounds.get(s)?.slots,
            Chain::Group(g) => &p.groups.get(g)?.slots,
            Chain::Master => &p.master_slots,
        })
    }

    /// A slot item holding a module of `class`, stored in full in this project.
    pub fn slot_template(&self, class: &str) -> Option<Value> {
        let mut found = None;
        self.root().walk_paths(&mut |o, _| {
            if found.is_none() && o.type_name == SLOT_PTR {
                let v = Value::Obj(o.clone());
                if module_of(&v).is_some_and(|m| m.type_name == class) {
                    found = Some(v);
                }
            }
        });
        found
    }

    /// A new slot item (`shared_ptr<Slot>`, switched on) holding `module` at its
    /// defaults, and its quick-browse handle.
    fn new_slot(&mut self, module: &NewModule) -> Result<(Value, Value), String> {
        let class = module.class();
        if !self.types.loaders.contains_key(&class) {
            return Err(format!("{class} is not a module DECKARD knows"));
        }
        self.build(|b| {
            let mut m = b.blank(&class)?;
            match module {
                NewModule::Sampler => {
                    for (k, d) in SAMPLER_PARAMS {
                        set_param(&mut m, k, *d);
                    }
                }
                NewModule::Audio => {
                    for (k, d) in AUDIO_DEFAULTS {
                        set_param(&mut m, k, *d);
                    }
                }
                NewModule::Effect(c) => {
                    let (_, defaults) = deckard_model::INTERNAL_EFFECTS
                        .iter()
                        .chain([&deckard_model::PERFORM_FX])
                        .find(|(k, _)| k == c)
                        .ok_or("not an internal effect")?;
                    for (k, d) in defaults.iter() {
                        set_param(&mut m, k, *d);
                    }
                }
                NewModule::Instrument { params, .. } => {
                    for (k, d) in params {
                        set_param(&mut m, k, *d);
                    }
                }
                NewModule::Plugin { id, name, chunk } => {
                    *m.find_mut("plugin_id").ok_or("plug-in host without plugin_id")? = Value::Str(id.clone());
                    *m.find_mut("plugin_name").ok_or("plug-in host without plugin_name")? = Value::Str(name.clone());
                    set_blob(crate::build::obj_mut(&mut m, "chunk")?, "size", chunk)?;
                    set_blob(crate::build::obj_mut(&mut m, "window")?, "size", &plugin_window())?;
                }
            }
            let mut slot = b.blank(SLOT)?;
            set_param(&mut slot, "active", 1.0);
            *crate::build::obj_mut(&mut slot, "module")?.get_mut("px").ok_or("slot module without px")? = Value::Obj(m);
            let item = b.shared(SLOT, slot);
            let qb = Value::Obj(b.blank(QUICK_BROWSE)?);
            Ok((item, qb))
        })
    }

    /// Insert a slot holding `module`, at its defaults, at position `at` of sound `s`'s
    /// chain (as one undo step). Built from scratch: no other project or slot is needed.
    pub fn add_slot(&mut self, g: usize, s: usize, at: usize, module: &NewModule) -> Result<(), String> {
        self.add_slot_in(Chain::Sound(g, s), at, module)
    }

    /// Put a Perform FX first in group `g`'s chain, at Maschine's defaults (one per
    /// group).
    pub fn add_perform_fx(&mut self, g: usize) -> Result<(), String> {
        if self.chain_slots(Chain::Group(g)).is_some_and(|s| s.iter().any(|x| x.module == deckard_model::PERFORM_FX.0)) {
            return Err("the group has a Perform FX".into());
        }
        self.add_slot_in(Chain::Group(g), 0, &NewModule::Effect(deckard_model::PERFORM_FX.0))?;
        self.relabel_last("Add Perform FX");
        Ok(())
    }

    /// [`Document::add_slot`] on any chain; a group's and the master's take effects only.
    pub fn add_slot_in(&mut self, c: Chain, at: usize, module: &NewModule) -> Result<(), String> {
        if module.is_instrument() && !matches!(c, Chain::Sound(..)) {
            return Err("an instrument goes in a sound's chain".into());
        }
        // An instrument can't come after an effect.
        let before: Vec<deckard_model::SlotInfo> = self.chain_slots(c).map_or(Vec::new(), |x| x.iter().take(at).cloned().collect());
        if module.is_instrument() && before.iter().any(|x| !x.is_instrument()) {
            return Err("an instrument can't go after an effect".into());
        }
        let chain_path = self.chain_of(c)?;
        let Some(Value::Obj(chain)) = self.tree.at(&chain_path).cloned() else { return Err("the sound has no chain".into()) };
        let mut chain = chain;
        let (item, qb) = self.new_slot(module)?;
        let slots = crate::build::obj_mut(&mut chain, "slots")?;
        let objs = crate::build::obj_mut(slots, "objects")?;
        if !objs.fields.iter().any(|(n, _)| n == "items#item_version") {
            crate::build::push_items(objs, Vec::new(), 1)?;
        }
        let Some(Value::List(items)) = objs.get_mut("items") else { return Err("slots without items".into()) };
        let at = at.min(items.len());
        items.insert(at, item);
        let first_qb = chain.fields.iter().position(|(n, _)| n == "quick_browse").unwrap_or(chain.fields.len());
        chain.fields.insert(first_qb + at, ("quick_browse".into(), qb));
        let label = match module {
            NewModule::Plugin { name, .. } if !name.is_empty() => "Load Plug-in",
            NewModule::Effect(_) => "Load Effect",
            _ => "Load Module",
        };
        let mut ops = vec![Op::Set { path: chain_path, value: Value::Obj(chain) }];
        // Side-chain inputs of the slots after it move up one.
        ops.extend(self.sidechain_rekey(c, |k| Some(if k >= at { k + 1 } else { k })));
        // So do its modulation routes.
        ops.extend(self.mod_routes_rekey(c, |k| Some(if k >= at { k + 1 } else { k })));
        self.apply(label, ops)
    }

    /// Remove slot `at` of sound `s`'s chain (one undo step).
    pub fn remove_slot(&mut self, g: usize, s: usize, at: usize) -> Result<(), String> {
        self.remove_slot_in(Chain::Sound(g, s), at)
    }

    /// [`Document::remove_slot`] on any chain.
    pub fn remove_slot_in(&mut self, c: Chain, at: usize) -> Result<(), String> {
        let chain_path = self.chain_of(c)?;
        let Some(Value::Obj(mut chain)) = self.tree.at(&chain_path).cloned() else { return Err("the sound has no chain".into()) };
        let Some(Value::Obj(slots)) = chain.get_mut("slots") else { return Err("chain without slots".into()) };
        let (removed_id, gone) = {
            let Some(Value::Obj(objs)) = slots.get_mut("objects") else { return Err("slots without objects".into()) };
            let Some(Value::List(items)) = objs.get_mut("items") else { return Err("slots without items".into()) };
            if at >= items.len() {
                return Err("no such slot".into());
            }
            let removed = items.remove(at);
            let gone = crate::automation::defined_ids(&removed);
            let id = match removed {
                Value::Obj(p) => match p.get("px") {
                    Some(Value::Obj(slot)) => slot.oid,
                    _ => None,
                },
                _ => None,
            };
            (id, gone)
        };
        // The vector's current slot, if it was this one.
        if let (Some(id), Some(cur)) = (removed_id, slots.get_mut("current"))
            && *cur == Value::Ref(id)
        {
            *cur = Value::Null;
        }
        if let Some(i) = chain.fields.iter().enumerate().filter(|(_, (n, _))| n == "quick_browse").nth(at).map(|(i, _)| i) {
            chain.fields.remove(i);
        }
        // Its automation goes with it, as in Maschine.
        let mut ops = self.drop_automation_ops(&gone, &chain_path)?;
        ops.insert(0, Op::Set { path: chain_path, value: Value::Obj(chain) });
        // Its side-chain input goes; those after it move down one.
        ops.extend(self.sidechain_rekey(c, |k| match k.cmp(&at) {
            std::cmp::Ordering::Less => Some(k),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(k - 1),
        }));
        // Its routes go too; those after it move down one.
        ops.extend(self.mod_routes_rekey(c, |k| match k.cmp(&at) {
            std::cmp::Ordering::Less => Some(k),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(k - 1),
        }));
        self.apply("Remove Module", ops)
    }

    /// Make sound `s`'s chain a lone Sampler, as for sampling into it (one undo step):
    /// a Sampler already there stays (with its samples), everything else goes. Returns
    /// whether anything changed.
    pub fn sampler_only(&mut self, g: usize, s: usize) -> Result<bool, String> {
        let slots = self.project.groups.get(g).and_then(|x| x.sounds.get(s)).map(|x| x.slots.clone()).ok_or("no such sound")?;
        let keep = slots.iter().position(|x| x.module == "Sampler");
        if keep.is_some() && slots.len() == 1 {
            return Ok(false);
        }
        self.transaction("Sample", |d| {
            for at in (0..slots.len()).rev().filter(|&at| Some(at) != keep) {
                d.remove_slot(g, s, at)?;
            }
            if keep.is_none() {
                d.add_slot(g, s, 0, &NewModule::Sampler)?;
            }
            Ok(())
        })?;
        Ok(true)
    }

    /// Move slot `from` of sound `s`'s chain to position `to` (one undo step). An
    /// instrument stays in the first slot, so it can't move later, nor anything move
    /// in front of it.
    pub fn move_slot(&mut self, g: usize, s: usize, from: usize, to: usize) -> Result<(), String> {
        self.move_slot_in(Chain::Sound(g, s), from, to)
    }

    /// [`Document::move_slot`] on any chain.
    pub fn move_slot_in(&mut self, c: Chain, from: usize, to: usize) -> Result<(), String> {
        let slots = self.chain_slots(c).map(<[_]>::to_vec).ok_or("no such chain")?;
        if from >= slots.len() || to >= slots.len() {
            return Err("no such slot".into());
        }
        if from == to {
            return Ok(());
        }
        if slots[from].is_instrument() {
            return Err("an instrument stays in the first slot, before the effects".into());
        }
        if to == 0 && slots[0].is_instrument() {
            return Err("effects go after the instrument".into());
        }
        let chain_path = self.chain_of(c)?;
        let Some(Value::Obj(mut chain)) = self.tree.at(&chain_path).cloned() else { return Err("the sound has no chain".into()) };
        {
            let Some(Value::Obj(sl)) = chain.get_mut("slots") else { return Err("chain without slots".into()) };
            let Some(Value::Obj(objs)) = sl.get_mut("objects") else { return Err("slots without objects".into()) };
            let Some(Value::List(items)) = objs.get_mut("items") else { return Err("slots without items".into()) };
            if items.len() != slots.len() {
                return Err("the chain doesn't match its slots".into());
            }
            let item = items.remove(from);
            items.insert(to, item);
        }
        // Each slot's quick-browse handle moves with it.
        let qbs: Vec<usize> = chain.fields.iter().enumerate().filter(|(_, (n, _))| n == "quick_browse").map(|(i, _)| i).collect();
        if qbs.len() == slots.len() {
            let mut handles: Vec<Value> = qbs.iter().map(|&i| chain.fields[i].1.clone()).collect();
            let h = handles.remove(from);
            handles.insert(to, h);
            for (&i, h) in qbs.iter().zip(handles) {
                chain.fields[i].1 = h;
            }
        }
        let mut ops = vec![Op::Set { path: chain_path, value: Value::Obj(chain) }];
        // Side-chain inputs go with their slots.
        let mut order: Vec<usize> = (0..slots.len()).collect();
        let k = order.remove(from);
        order.insert(to, k);
        ops.extend(self.sidechain_rekey(c, |old| order.iter().position(|&o| o == old)));
        ops.extend(self.mod_routes_rekey(c, |old| order.iter().position(|&o| o == old)));
        self.apply("Move Module", ops)
    }

    /// Bypass (`active` false) or enable slot `at` of sound `s`.
    pub fn set_slot_active(&mut self, g: usize, s: usize, at: usize, active: bool) -> Result<(), String> {
        self.set_slot_active_in(Chain::Sound(g, s), at, active)
    }

    /// [`Document::set_slot_active`] on any chain.
    pub fn set_slot_active_in(&mut self, c: Chain, at: usize, active: bool) -> Result<(), String> {
        let chain_path = self.chain_of(c)?;
        let items = self.field_path(&chain_path, &["slots", "objects", "items"])?;
        let slot = [items.as_slice(), &[at as u32]].concat();
        let Some(Value::Obj(ptr)) = self.tree.at(&slot) else { return Err("no such slot".into()) };
        let px = ptr.fields.iter().position(|(n, _)| n == "px").ok_or("bad slot")? as u32;
        let Some(Value::Obj(sl)) = ptr.get("px") else { return Err("bad slot".into()) };
        let ap = sl.find_path("active").ok_or("slot without active")?;
        let Some(Value::Obj(param)) = sl.at(&ap) else { return Err("bad slot".into()) };
        let base = [slot.as_slice(), &[px], &ap].concat();
        let mut ops = Vec::new();
        for k in ["value", "t0"] {
            if let Some(kp) = param.find_path(k) {
                ops.push(Op::Set { path: [base.as_slice(), &kp].concat(), value: Value::Int(active as i64) });
            }
        }
        self.apply(if active { "Enable Module" } else { "Bypass Module" }, ops)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Document, NewModule};

    /// Sampling into a sound leaves it a lone Sampler, in one undo step; a Sampler
    /// already there is kept.
    /// An effect plug-in in an empty sound's first slot (one that takes audio in, say)
    /// is an effect, not the sound's instrument.
    #[test]
    fn an_effect_plugin_first_is_no_instrument() {
        let mut d = Document::new_project().unwrap();
        let fx = NewModule::Plugin {
            id: "BABY Audio:VST3:FX:565354436D36626261627920636F6D65".into(),
            name: "Baby Comeback".into(),
            chunk: Vec::new(),
        };
        d.add_slot(0, 1, 0, &fx).unwrap();
        let snd = &d.project().groups[0].sounds[1];
        assert_eq!(snd.slots.len(), 1);
        assert!(snd.plugin.is_none(), "{:?}", snd.plugin);
    }

    #[test]
    fn a_sound_becomes_a_lone_sampler() {
        let mut d = Document::new_project().unwrap();
        let effect = deckard_model::INTERNAL_EFFECTS[0].0;
        d.add_slot(0, 0, 0, &NewModule::Effect(effect)).unwrap();
        d.add_slot(0, 0, 1, &NewModule::Effect(effect)).unwrap();
        let modules = |d: &Document| d.project().groups[0].sounds[0].slots.iter().map(|x| x.module.clone()).collect::<Vec<_>>();
        let before = modules(&d);
        assert_eq!(before.len(), 2);
        assert!(d.sampler_only(0, 0).unwrap());
        assert_eq!(modules(&d), ["Sampler"]);
        assert!(!d.sampler_only(0, 0).unwrap(), "already a lone Sampler");
        assert!(d.undo());
        assert_eq!(modules(&d), before, "one undo step");

        // Sampler + effect: the Sampler stays.
        d.redo();
        d.add_slot(0, 0, 1, &NewModule::Effect(effect)).unwrap();
        assert!(d.sampler_only(0, 0).unwrap());
        assert_eq!(modules(&d), ["Sampler"]);
    }

    /// A group's and the master's chains take effects as a sound's does: added,
    /// bypassed, moved, removed, their parameters set; saved and opened again. An
    /// instrument doesn't go there.
    #[test]
    fn group_and_master_chains_edit_like_sounds() {
        use crate::{Chain, Target};
        let Ok(mut d) = Document::open(std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj")) else {
            return;
        };
        let g0 = d.project().groups[0].slots.len();
        let m0 = d.project().master_slots.len();
        d.add_slot_in(Chain::Group(0), g0, &NewModule::Effect("Filter".into())).unwrap();
        d.add_slot_in(Chain::Master, m0, &NewModule::Effect("Filter".into())).unwrap();
        assert_eq!(d.project().groups[0].slots[g0].module, "Filter");
        assert_eq!(d.project().master_slots[m0].module, "Filter");
        assert!(d.add_slot_in(Chain::Group(0), 0, &NewModule::Sampler).is_err(), "no instrument in a group's chain");
        d.set_slot_active_in(Chain::Master, m0, false).unwrap();
        assert!(!d.project().master_slots[m0].active);
        d.set("Cutoff", Target::ChainModule(Chain::Group(0), g0), "cutoff", mxprj::decode::Value::Float(0.25)).unwrap();
        assert_eq!(d.project().groups[0].slots[g0].param("cutoff"), Some(0.25));
        if m0 > 0 {
            d.move_slot_in(Chain::Master, m0, 0).unwrap();
            assert_eq!(d.project().master_slots[0].module, "Filter");
        }
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("chain.mxprj");
        std::fs::write(&path, d.to_bytes().unwrap()).unwrap();
        let back = Document::open(&path).unwrap();
        assert_eq!(back.project().groups[0].slots[g0].param("cutoff"), Some(0.25));
        assert!(back.project().master_slots.iter().any(|x| x.module == "Filter" && !x.active));
        let mut d = back;
        d.remove_slot_in(Chain::Group(0), g0).unwrap();
        assert_eq!(d.project().groups[0].slots.len(), g0);
    }
}

#[cfg(test)]
mod perform_fx_tests {
    use crate::{Chain, Document};

    /// A group's Perform FX goes first in its chain at Maschine's defaults, once; it
    /// saves and reads back.
    #[test]
    fn a_perform_fx_goes_first_in_a_groups_chain() {
        let mut d = Document::new_project().unwrap();
        d.add_slot_in(Chain::Group(0), 0, &crate::NewModule::Effect("Compressor")).unwrap();
        d.add_perform_fx(0).unwrap();
        let slots = &d.project().groups[0].slots;
        assert_eq!(slots[0].module, "PerformanceFX");
        assert_eq!(slots[1].module, "Compressor");
        assert_eq!(slots[0].param("burst_feedback"), Some(0.95_f32 as f64));
        assert_eq!(slots[0].param("stutter_direction"), Some(2.0));
        assert_eq!(slots[0].param("touchstrip_pos"), Some(0.5));
        assert!(d.add_perform_fx(0).is_err(), "one per group");
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(back.project().groups[0].slots[0].param("filter_out_level"), Some(0.74_f32 as f64));
        assert_eq!(d.undo_label(), Some("Add Perform FX"));
    }
}
