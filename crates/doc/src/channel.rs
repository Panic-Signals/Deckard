//! Channel properties (docs/channel-software.md): routing of sounds, groups and the
//! master, and the sound configs that keep a sound's routing in a project.
//!
//! Maschine never saves a sound's routing on the sound itself: each group keeps one
//! `SoundConfig` per sound slot, which is applied to the sound when a project loads and
//! wins over the values the sound stores. So every sound property it holds is written
//! there too (§9).

use super::{Document, Op, Target};
use deckard_model::{AudioDest, AudioSource, MidiDest, ScSource};
use mxprj::decode::{Obj, Path, Value};

/// Sound parameters that the group's sound config also holds: (sound field, config field).
pub(crate) const MIRRORED: [(&str, &str); 17] = [
    ("midi_channel", "midi_channel"),
    ("midi_thru", "midi_thru"),
    ("input_gain", "input_gain"),
    ("mute", "mute"),
    ("audio_mute", "audio_mute"),
    ("aux1_level", "aux1_level"),
    ("aux1_order", "aux1_order"),
    ("aux2_level", "aux2_level"),
    ("aux2_order", "aux2_order"),
    ("event_output_channel", "midi_out_channel"),
    ("transpose", "transpose"),
    ("choke_group", "choke_group"),
    ("choke_mode", "choke_mode"),
    ("link_group", "link_group"),
    ("link_mode", "link_mode"),
    ("swing_bounce", "swing_bounce"),
    ("cue_enabled", "cue_enabled"),
];

const SC_VALUE: &str = "NI::MASCHINE::DATA::SideChainInputParameterValue";

/// A routing property of a sound (kept in its sound config only).
#[derive(Debug, Clone, PartialEq)]
pub enum SoundRoute {
    /// The MIDI input port ("Default", "Focus", "None", "All", or a port's name).
    MidiSource(String),
    /// The MIDI input port a plug-in instance of Maschine uses ("None", "Default",
    /// "Host", "Focus"; saved separately from the standalone app's).
    MidiSourcePlugin(String),
    AudioIn(AudioSource),
    Output(AudioDest),
    Aux(usize, AudioDest),
    MidiOut(MidiDest),
}

impl Document {
    /// Where group `g`'s sound config for slot `s` is.
    pub(crate) fn sound_config_path(&self, g: usize, s: usize) -> Option<Path> {
        let group = self.owner(Target::Group(g))?;
        let items = self.field_in(&group, "sound_configs/items")?;
        let at = [group.as_slice(), &items, &[s as u32]].concat();
        matches!(self.tree.at(&at), Some(Value::Obj(_))).then_some(at)
    }

    /// The op that writes `value` into the sound config's copy of `field` (if it has one).
    pub(crate) fn mirror_op(&self, target: Target, field: &str, value: &Value) -> Option<Op> {
        let Target::Sound(g, s) = target else { return None };
        let (_, cf) = MIRRORED.iter().find(|(f, _)| *f == field)?;
        let at = self.sound_config_path(g, s)?;
        let Some(Value::Obj(cfg)) = self.tree.at(&at) else { return None };
        let i = cfg.fields.iter().position(|(n, _)| n == cf)?;
        let value = match (&cfg.fields[i].1, value) {
            (Value::Int(_), Value::Float(f)) => Value::Int(f.round() as i64),
            (Value::Float(_), Value::Int(n)) => Value::Float(*n as f64),
            (_, v) => v.clone(),
        };
        (cfg.fields[i].1 != value).then(|| Op::Set { path: [at.as_slice(), &[i as u32]].concat(), value })
    }

    /// The object id of a group or sound (for routing pointers).
    fn oid_of(&self, t: Target) -> Option<u64> {
        match self.tree.at(&self.owner(t)?)? {
            Value::Obj(o) => o.oid,
            _ => None,
        }
    }

    /// An `Audio{Output,Input}ParameterValue` like `old` with a new type, index and target.
    fn route_value(&self, old: &Obj, kind: i64, index: i64, group: Option<Target>, sound: Option<Target>) -> Result<Value, String> {
        let mut o = old.clone();
        let reference = |t: Option<Target>| -> Result<Value, String> {
            match t {
                None => Ok(Value::Null),
                Some(t) => self.oid_of(t).map(Value::Ref).ok_or_else(|| format!("{t:?} can't be pointed at")),
            }
        };
        let (gv, sv) = (reference(group)?, reference(sound)?);
        for (name, v) in o.fields.iter_mut() {
            match name.as_str() {
                "type" => *v = Value::Int(kind),
                "index" => *v = Value::Int(index),
                "group" => *v = gv.clone(),
                "sound" => *v = sv.clone(),
                "slot" => *v = Value::Null,
                _ => {}
            }
        }
        Ok(Value::Obj(o))
    }

    fn dest_value(&self, old: &Obj, d: AudioDest) -> Result<Value, String> {
        match d {
            AudioDest::None => self.route_value(old, 0, -1, None, None),
            AudioDest::Parent => self.route_value(old, 1, -1, None, None),
            AudioDest::Master => self.route_value(old, 2, -1, None, None),
            AudioDest::Ext(n) => self.route_value(old, 3, n, None, None),
            AudioDest::Group(g) => self.route_value(old, 4, -1, Some(Target::Group(g)), None),
            AudioDest::Sound(g, s) => self.route_value(old, 5, -1, Some(Target::Group(g)), Some(Target::Sound(g, s))),
        }
    }

    fn source_value(&self, old: &Obj, src: AudioSource) -> Result<Value, String> {
        match src {
            AudioSource::None => self.route_value(old, 0, -1, None, None),
            AudioSource::Ext(n) => self.route_value(old, 1, n, None, None),
            AudioSource::Sound(g, s) => self.route_value(old, 2, -1, Some(Target::Group(g)), Some(Target::Sound(g, s))),
            AudioSource::Master => self.route_value(old, 3, -1, None, None),
            AudioSource::Group(g) => self.route_value(old, 4, -1, Some(Target::Group(g)), None),
        }
    }

    /// Change a routing property of sound `s` in group `g` (one undo step).
    pub fn set_sound_route(&mut self, g: usize, s: usize, route: SoundRoute) -> Result<(), String> {
        let at = self.sound_config_path(g, s).ok_or("this sound has no saved routing")?;
        let Some(Value::Obj(cfg)) = self.tree.at(&at) else { return Err("no sound config".into()) };
        let field = |name: &str| cfg.fields.iter().position(|(n, _)| n == name).ok_or_else(|| format!("sound config has no {name}"));
        let obj = |name: &str| -> Result<(usize, &Obj), String> {
            let i = field(name)?;
            match &cfg.fields[i].1 {
                Value::Obj(o) => Ok((i, o)),
                _ => Err(format!("{name} is not stored here")),
            }
        };
        let (i, value, label) = match &route {
            SoundRoute::MidiSource(port) => (field("midi_source")?, Value::Str(port.clone()), "MIDI Source"),
            SoundRoute::MidiSourcePlugin(port) => (field("midi_source_plugin")?, Value::Str(port.clone()), "MIDI Source"),
            SoundRoute::AudioIn(src) => {
                let (i, o) = obj("audio_input")?;
                (i, self.source_value(o, *src)?, "Audio Source")
            }
            SoundRoute::Output(d) => {
                if *d == AudioDest::Sound(g, s) {
                    return Err("a sound can't feed itself".into());
                }
                let (i, o) = obj("audio_output")?;
                (i, self.dest_value(o, *d)?, "Output")
            }
            SoundRoute::Aux(k, d) => {
                let (i, o) = obj(if *k == 0 { "aux1_output" } else { "aux2_output" })?;
                (i, self.dest_value(o, *d)?, "Aux")
            }
            SoundRoute::MidiOut(d) => {
                let (i, o) = obj("event_output")?;
                let mut o = o.clone();
                let (kind, port, sound) = match d {
                    MidiDest::None => (0, "None".to_string(), None),
                    MidiDest::Port(p) => (1, p.clone(), None),
                    MidiDest::Sound(t) => (2, "None".to_string(), Some(Target::Sound(g, *t))),
                };
                let sv = match sound {
                    Some(t) => Value::Ref(self.oid_of(t).ok_or("no such sound")?),
                    None => Value::Null,
                };
                for (n, v) in o.fields.iter_mut() {
                    match n.as_str() {
                        "type" => *v = Value::Int(kind),
                        "port" => *v = Value::Str(port.clone()),
                        "sound" => *v = sv.clone(),
                        "slot" => *v = Value::Null,
                        _ => {}
                    }
                }
                (i, Value::Obj(o), "MIDI Output")
            }
        };
        let ops = vec![Op::Set { path: [at.as_slice(), &[i as u32]].concat(), value }];
        self.apply(label, ops)
    }

    /// Set (or clear) the side-chain source of the effect in slot `slot` of sound `s`
    /// (group `g`). Maschine keeps it in the group's sound config, by slot.
    /// The op re-keying the side-chain inputs of chain `c` (a sound's) as its slots
    /// moved: `remap` gives each slot's new place, `None` if it's gone (its input goes
    /// too). `None` when nothing changes.
    pub(crate) fn sidechain_rekey(&self, c: crate::Chain, remap: impl Fn(usize) -> Option<usize>) -> Option<Op> {
        let crate::Chain::Sound(g, s) = c else { return None };
        let at = self.sound_config_path(g, s)?;
        let Some(Value::Obj(cfg)) = self.tree.at(&at) else { return None };
        let mi = cfg.fields.iter().position(|(n, _)| n == "sidechain_inputs")?;
        let Value::Obj(map) = &cfg.fields[mi].1 else { return None };
        let Some(Value::List(items)) = map.get("items") else { return None };
        let mut changed = false;
        let mut moved: Vec<Value> = items
            .iter()
            .filter_map(|v| {
                let Value::Obj(pair) = v else { return Some(v.clone()) };
                let Some(slot) = pair.get("first").and_then(Value::as_int) else { return Some(v.clone()) };
                let to = remap(slot.max(0) as usize);
                if to != Some(slot as usize) {
                    changed = true;
                }
                let mut pair = pair.clone();
                *pair.get_mut("first")? = Value::Int(to? as i64);
                Some(Value::Obj(pair))
            })
            .collect();
        if !changed {
            return None;
        }
        moved.sort_by_key(|v| match v {
            Value::Obj(o) => o.get("first").and_then(Value::as_int).unwrap_or(0),
            _ => 0,
        });
        let mut map = map.clone();
        *map.get_mut("items")? = Value::List(moved);
        Some(Op::Set { path: [at.as_slice(), &[mi as u32]].concat(), value: Value::Obj(map) })
    }

    pub fn set_sidechain(&mut self, g: usize, s: usize, slot: usize, src: Option<ScSource>) -> Result<(), String> {
        let at = self.sound_config_path(g, s).ok_or("this sound has no saved routing")?;
        let Some(Value::Obj(cfg)) = self.tree.at(&at) else { return Err("no sound config".into()) };
        let mi = cfg.fields.iter().position(|(n, _)| n == "sidechain_inputs").ok_or("this project's format has no side-chain inputs")?;
        let Value::Obj(map) = &cfg.fields[mi].1 else { return Err("bad side-chain map".into()) };
        let ii = map.fields.iter().position(|(n, _)| n == "items").ok_or("bad side-chain map")?;
        let Value::List(items) = &map.fields[ii].1 else { return Err("bad side-chain map".into()) };
        let mut items: Vec<Value> = items
            .iter()
            .filter(|p| !matches!(p, Value::Obj(o) if o.get("first").and_then(Value::as_int) == Some(slot as i64)))
            .cloned()
            .collect();
        if let Some(src) = src {
            // The value class's version in this project (Maschine 3 writes 2: pointers).
            let mut version = 2;
            self.root().walk_paths(&mut |o, _| {
                if o.type_name == SC_VALUE {
                    version = o.version;
                }
            });
            if version == 1 {
                return Err("this project's side-chain format is too old to edit".into());
            }
            let (kind, group, sound) = match src {
                ScSource::Group(x) => (1, Some(Target::Group(x)), None),
                ScSource::Sound(x, y) => {
                    if (x, y) == (g, s) {
                        return Err("a sound can't side-chain itself".into());
                    }
                    (2, Some(Target::Group(x)), Some(Target::Sound(x, y)))
                }
            };
            let ptr = |t: Option<Target>| -> Result<Value, String> {
                Ok(match t {
                    Some(t) => Value::Ref(self.oid_of(t).ok_or("no such channel")?),
                    None => Value::Null,
                })
            };
            let value = Obj {
                type_name: SC_VALUE.into(),
                version,
                oid: None,
                export: None,
                fields: vec![("type".into(), Value::Int(kind)), ("group".into(), ptr(group)?), ("sound".into(), ptr(sound)?)],
            };
            let pair = Obj {
                type_name: format!("std::pair<unsigned long const, {SC_VALUE}>"),
                version: 0,
                oid: None,
                export: None,
                fields: vec![("first".into(), Value::Int(slot as i64)), ("second".into(), Value::Obj(value))],
            };
            items.push(Value::Obj(pair));
            items.sort_by_key(|p| match p {
                Value::Obj(o) => o.get("first").and_then(Value::as_int).unwrap_or(0),
                _ => 0,
            });
        }
        let path = [at.as_slice(), &[mi as u32, ii as u32]].concat();
        self.apply("Side-Chain Source", vec![Op::Set { path, value: Value::List(items) }])
    }

    /// Change where group `g`'s main output (`aux` None) or aux send goes.
    pub fn set_group_route(&mut self, g: usize, aux: Option<usize>, d: AudioDest) -> Result<(), String> {
        if d == AudioDest::Group(g) || matches!(d, AudioDest::Sound(x, _) if x == g) {
            return Err("a group can't feed itself".into());
        }
        let field = match aux {
            None => "audio_output",
            Some(0) => "aux1_output",
            Some(_) => "aux2_output",
        };
        self.set_route_param(Target::GroupStrip(g), field, |doc, o| doc.dest_value(o, d), if aux.is_some() { "Aux" } else { "Output" })
    }

    /// Change where the master (`cue` false) or the cue bus goes.
    pub fn set_master_output(&mut self, cue: bool, d: AudioDest) -> Result<(), String> {
        let field = if cue { "cue_output" } else { "master_output" };
        self.set_route_param(Target::Song, field, |doc, o| doc.dest_value(o, d), "Output")
    }

    fn set_route_param(
        &mut self,
        target: Target,
        field: &str,
        make: impl FnOnce(&Document, &Obj) -> Result<Value, String>,
        label: &str,
    ) -> Result<(), String> {
        let path = self.param_path(target, field)?;
        let Some(Value::Obj(old)) = self.tree.at(&path) else { return Err(format!("{field} is not stored here")) };
        let value = make(self, old)?;
        if self.tree.at(&path) == Some(&value) {
            return Ok(());
        }
        self.apply(label, vec![Op::Set { path, value }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud9() -> Option<Document> {
        Document::open(std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj")).ok()
    }

    #[test]
    fn routing_survives_save_and_reload() {
        let Some(mut d) = cloud9() else { return };
        let ch = &d.project().groups[0].sounds[0].channel;
        assert_eq!(ch.dest, AudioDest::Parent);
        assert_eq!(ch.midi_out, MidiDest::None);
        // Sound 1 of group 1 to group 5 (written ahead of where it is), aux 1 to the
        // master, MIDI out to a port on channel 3 transposed; group 2 to group 1.
        d.set_sound_route(0, 0, SoundRoute::Output(AudioDest::Group(4))).unwrap();
        d.set_sound_route(0, 0, SoundRoute::Aux(0, AudioDest::Master)).unwrap();
        d.set_sound_route(0, 0, SoundRoute::MidiOut(MidiDest::Port("IAC Driver Bus 1".into()))).unwrap();
        d.set_sound_route(0, 0, SoundRoute::MidiSource("All".into())).unwrap();
        d.set("MIDI", Target::Sound(0, 0), "event_output_channel", Value::Int(2)).unwrap();
        d.set("MIDI", Target::Sound(0, 0), "transpose", Value::Int(-12)).unwrap();
        d.set("Mute", Target::Sound(0, 1), "mute", Value::Int(1)).unwrap();
        d.set_group_route(1, None, AudioDest::Group(0)).unwrap();
        d.set_master_output(true, AudioDest::Ext(1)).unwrap();
        // The compressor in slot 2 of the kick, side-chained from the snare group's
        // first sound; then from a whole group.
        d.set_sidechain(0, 0, 2, Some(ScSource::Sound(1, 0))).unwrap();
        d.set_sidechain(0, 1, 1, Some(ScSource::Group(3))).unwrap();
        assert!(d.set_sidechain(0, 0, 2, Some(ScSource::Sound(0, 0))).is_err(), "not itself");
        assert!(d.set_group_route(1, None, AudioDest::Group(1)).is_err(), "not into itself");
        let check = |d: &Document| {
            let p = d.project();
            let c = &p.groups[0].sounds[0].channel;
            assert_eq!(c.dest, AudioDest::Group(4));
            assert_eq!(c.aux_dest[0], AudioDest::Master);
            assert_eq!(c.midi_out, MidiDest::Port("IAC Driver Bus 1".into()));
            assert_eq!(c.midi_source, "All");
            assert_eq!((c.midi_out_channel, c.transpose), (2, -12));
            assert!(p.groups[0].sounds[1].muted, "mute kept in the sound config");
            assert_eq!(p.groups[1].channel.dest, AudioDest::Group(0));
            assert_eq!(p.cue_output, AudioDest::Ext(1));
            assert_eq!(p.groups[0].sounds[0].channel.sidechain, vec![(2, ScSource::Sound(1, 0))]);
            assert_eq!(p.groups[0].sounds[1].channel.sidechain, vec![(1, ScSource::Group(3))]);
        };
        check(&d);
        let bytes = d.to_bytes().unwrap();
        let back = Document::from_bytes(bytes, None).unwrap();
        check(&back);
        // Undo takes them back.
        while d.undo() {}
        assert_eq!(d.project().groups[0].sounds[0].channel.dest, AudioDest::Parent);
        assert!(!d.project().groups[0].sounds[1].muted);
    }
}

#[cfg(test)]
mod sidechain_read {
    use super::*;

    /// Maschine's own side-chain settings read back as sources.
    #[test]
    fn reads_maschine_sidechains() {
        let Ok(d) = Document::open(std::path::Path::new("/Users/Shared/Chromatic Fire Library/Projects/Black Earth.mxprj")) else { return };
        let all: Vec<(usize, usize, usize, ScSource)> = d
            .project()
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, gr)| {
                gr.sounds.iter().enumerate().flat_map(move |(s, snd)| snd.channel.sidechain.iter().map(move |&(k, src)| (g, s, k, src)))
            })
            .collect();
        assert!(!all.is_empty(), "Black Earth side-chains some sounds");
        for (g, s, k, _) in &all {
            assert!(*k < d.project().groups[*g].sounds[*s].slots.len().max(1) + 8, "slot {k} of {g}/{s}");
        }
    }
}

#[cfg(test)]
mod sidechain_mute_tests {
    use crate::{Document, NewModule, Target};
    use deckard_model::ScSource;

    /// A sound whose Compressor is side-chained from another mutes as any other: its
    /// mute and its audio mute set.
    #[test]
    fn a_side_chained_sound_mutes() {
        let mut d = Document::new_project().unwrap();
        d.add_slot(0, 0, 0, &NewModule::Effect("Compressor")).unwrap();
        let before = d.project().groups[0].sounds[0].channel.clone();
        d.set_sidechain(0, 0, 0, Some(ScSource::Sound(0, 1))).unwrap();
        assert_eq!(d.project().groups[0].sounds[0].channel.sidechain, [(0, ScSource::Sound(0, 1))]);
        d.set_mute(Target::Sound(0, 0), true).unwrap();
        let s = &d.project().groups[0].sounds[0];
        assert!(s.muted, "muted");
        assert_eq!(s.channel.audio_mute, before.audio_mute, "the audio mute as it was");
        let mut plain = Document::new_project().unwrap();
        plain.set_mute(Target::Sound(0, 0), true).unwrap();
        assert_eq!(plain.project().groups[0].sounds[0].channel.audio_mute, s.channel.audio_mute, "as without a side-chain");
    }
}

/// BUGS #2: a project saved with group 0's Compressor listening to group 1 reopens
/// with group 1 defined inside that side-chain (its first place in the file). Changing
/// or clearing the side-chain, or removing its effect, keeps group 1.
#[cfg(test)]
mod sidechain_keeps_groups_tests {
    use crate::{Document, NewModule};
    use deckard_model::{Project, ScSource};

    fn reopened() -> Document {
        let mut d = Document::new_project().unwrap();
        let g = d.add_group().unwrap();
        d.add_slot(0, 0, 0, &NewModule::Effect("Compressor")).unwrap();
        d.set_sidechain(0, 0, 0, Some(ScSource::Group(g))).unwrap();
        // Group 1 plays a pattern in the scene: the scene refers into it.
        let pi = d.add_pattern(g, 0, "Pattern 1").unwrap();
        d.assign_pattern(0, g, Some(pi)).unwrap();
        Document::from_bytes(d.to_bytes().unwrap(), None).unwrap()
    }

    /// The groups and what the scene plays.
    fn groups(p: &Project) -> (usize, Vec<(usize, usize)>) {
        (p.groups.len(), p.scenes[0].patterns.clone())
    }

    #[test]
    fn changing_a_side_chain_keeps_the_group_it_listened_to() {
        let edits: [(&str, fn(&mut Document) -> Result<(), String>); 3] = [
            ("cleared", |d| d.set_sidechain(0, 0, 0, None)),
            ("changed", |d| d.set_sidechain(0, 0, 0, Some(ScSource::Sound(0, 1)))),
            ("its effect removed", |d| d.remove_slot(0, 0, 0)),
        ];
        for (what, edit) in edits {
            let mut d = reopened();
            let before = groups(d.project());
            assert_eq!(d.project().groups[0].sounds[0].channel.sidechain, [(0, ScSource::Group(1))], "set up");
            assert_eq!(before.0, 2);
            assert!(before.1.iter().any(|&(g, _)| g == 1), "the scene plays group 1: {before:?}");
            edit(&mut d).unwrap();
            assert_eq!(groups(d.project()), before, "{what}: the group stays, and what the scene plays");
            let again = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap_or_else(|e| panic!("{what}: reopens: {e}"));
            assert_eq!(groups(again.project()), before, "{what}: and is saved");
            assert!(d.undo());
            assert_eq!(d.project().groups[0].sounds[0].channel.sidechain, [(0, ScSource::Group(1))], "{what}: undone");
        }
    }
}

#[cfg(test)]
mod sidechain_follows_tests {
    use crate::{Document, NewModule};
    use deckard_model::ScSource;

    fn sc(d: &Document) -> Vec<(usize, ScSource)> {
        d.project().groups[0].sounds[0].channel.sidechain.clone()
    }

    /// A side-chain belongs to its effect: moving, adding or removing slots before it
    /// takes it along; removing its effect removes it.
    #[test]
    fn side_chains_follow_their_effect() {
        let mut d = Document::new_project().unwrap();
        d.add_slot(0, 0, 0, &NewModule::Effect("Filter")).unwrap();
        d.add_slot(0, 0, 1, &NewModule::Effect("Compressor")).unwrap();
        d.set_sidechain(0, 0, 1, Some(ScSource::Sound(0, 1))).unwrap();
        d.move_slot(0, 0, 1, 0).unwrap();
        assert_eq!(sc(&d), [(0, ScSource::Sound(0, 1))], "moved");
        d.add_slot(0, 0, 0, &NewModule::Effect("EQ")).unwrap();
        assert_eq!(sc(&d), [(1, ScSource::Sound(0, 1))], "a slot added before it");
        d.remove_slot(0, 0, 0).unwrap();
        assert_eq!(sc(&d), [(0, ScSource::Sound(0, 1))], "a slot removed before it");
        // (A moment later: not merged into the removal before.)
        if let Some(last) = d.undo.last_mut() {
            last.at -= crate::MERGE_WINDOW * 2;
        }
        d.remove_slot(0, 0, 0).unwrap();
        assert!(sc(&d).is_empty(), "its effect removed");
        assert!(d.undo());
        assert_eq!(sc(&d), [(0, ScSource::Sound(0, 1))], "undone with it");
    }

    /// Choke and Link (Pad mode): group None/1–8 and mode Send/Receive, kept in the
    /// sound and its group's sound config; saved, read back, undone.
    #[test]
    fn choke_and_link_are_kept() {
        use crate::Target;
        use mxprj::decode::Value;
        let mut d = Document::new_project().unwrap();
        for (f, v) in [("choke_group", 3), ("choke_mode", 1), ("link_group", 8), ("link_mode", 1)] {
            d.set("Pad", Target::Sound(0, 1), f, Value::Int(v)).unwrap();
        }
        let check = |d: &Document| {
            let c = &d.project().groups[0].sounds[1].channel;
            assert_eq!((c.choke_group, c.choke_mode, c.link_group, c.link_mode), (3, 1, 8, 1));
        };
        check(&d);
        check(&Document::from_bytes(d.to_bytes().unwrap(), None).unwrap());
        while d.undo() {}
        let c = &d.project().groups[0].sounds[1].channel;
        assert_eq!((c.choke_group, c.choke_mode, c.link_group, c.link_mode), (0, 0, 0, 0));
    }
}
