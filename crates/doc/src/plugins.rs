//! Plug-in state: what a hosted plug-in saves into the project.

use crate::{Document, Op};
use mxprj::decode::{Path, Value};

impl Document {
    /// Where the instrument of sound `sound` in group `group` lives: its first
    /// `PluginHost` module.
    fn plugin_host_path(&self, group: usize, sound: usize) -> Result<Path, String> {
        let owner = self.owner(crate::Target::Sound(group, sound)).ok_or("no such sound")?;
        let Some(Value::Obj(snd)) = self.tree.at(&owner) else { return Err("no such sound".into()) };
        let mut found = None;
        snd.walk_paths(&mut |o, p| {
            if found.is_none() && o.type_name == "NI::MASCHINE::DATA::PluginHost" {
                found = Some(p.to_vec());
            }
        });
        let rel = found.ok_or("the sound has no plug-in")?;
        Ok([owner.as_slice(), &rel].concat())
    }

    /// The `PluginHost` module in slot `slot` of the sound's chain.
    fn slot_host_path(&self, group: usize, sound: usize, slot: usize) -> Result<Path, String> {
        let owner = self.owner(crate::Target::Sound(group, sound)).ok_or("no such sound")?;
        let items = self.field_path(&owner, &["chain", "slots", "objects", "items"])?;
        let at = [items.as_slice(), &[slot as u32]].concat();
        let mut path = at.clone();
        for name in ["px", "module", "px"] {
            let Some(Value::Obj(o)) = self.tree.at(&path) else { return Err("no such slot".into()) };
            path.push(o.fields.iter().position(|(n, _)| n == name).ok_or("no such slot")? as u32);
        }
        match self.tree.at(&path) {
            Some(Value::Obj(m)) if m.type_name == "NI::MASCHINE::DATA::PluginHost" => Ok(path),
            _ => Err("the slot holds no plug-in".into()),
        }
    }

    /// Store the state of the plug-in in chain slot `slot` (as [`Document::set_plugin_chunk`]).
    pub fn set_slot_plugin_chunk(&mut self, group: usize, sound: usize, slot: usize, chunk: Vec<u8>) -> Result<(), String> {
        let host = self.slot_host_path(group, sound, slot)?;
        self.store_chunk(host, chunk)
    }

    /// Store the state of the plug-in in slot `slot` of a group's or the master's chain.
    pub fn set_chain_plugin_chunk(&mut self, c: crate::Chain, slot: usize, chunk: Vec<u8>) -> Result<(), String> {
        let host = self.owner(crate::Target::ChainModule(c, slot)).ok_or("no such slot")?;
        match self.tree.at(&host) {
            Some(Value::Obj(m)) if m.type_name == "NI::MASCHINE::DATA::PluginHost" => self.store_chunk(host, chunk),
            _ => Err("the slot holds no plug-in".into()),
        }
    }

    /// Store a plug-in instrument's state (Maschine's chunk format, see
    /// `deckard_plugins::state`), as one undo step.
    pub fn set_plugin_chunk(&mut self, group: usize, sound: usize, chunk: Vec<u8>) -> Result<(), String> {
        let host = self.plugin_host_path(group, sound)?;
        self.store_chunk(host, chunk)
    }

    fn store_chunk(&mut self, host: Path, chunk: Vec<u8>) -> Result<(), String> {
        let Some(Value::Obj(h)) = self.tree.at(&host) else { return Err("plug-in vanished".into()) };
        let cp = h.find_path("chunk").ok_or("the plug-in has no chunk")?;
        let Some(Value::Obj(c)) = h.at(&cp) else { return Err("the plug-in's chunk is not an object".into()) };
        let (sp, dp) = (c.find_path("size").ok_or("chunk has no size")?, c.find_path("size_data").ok_or("chunk has no data")?);
        if c.at(&dp) == Some(&Value::Bytes(chunk.clone())) {
            return Ok(());
        }
        let base = [host.as_slice(), &cp].concat();
        let ops = vec![
            Op::Set { path: [base.as_slice(), &sp].concat(), value: Value::Int(chunk.len() as i64) },
            Op::Set { path: [base.as_slice(), &dp].concat(), value: Value::Bytes(chunk) },
        ];
        self.apply("Plug-in Edit", ops)
    }
}
