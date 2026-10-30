//! Plug-in parameters as objects: Maschine's `PluginParameter`s, eight to each of a
//! `PluginHost`'s saved pages (`parameters.unknown_48`), are what a macro points at to
//! control a plug-in's parameter. Maschine saves the pages it shows; DECKARD saves them
//! when one of their parameters is first made a macro.

use crate::{Chain, Document, Op, Target};
use mxprj::decode::{Obj, Path, Value};

const PAGE: &str = "NI::MASCOMMON::DATA::PluginPage";
const PARAM: &str = "NI::MASCOMMON::DATA::PluginParameter";
/// A page's unused parameter's index (as Maschine writes it).
const UNUSED: i64 = u32::MAX as i64;

/// A plug-in page to save: its name and its knobs (parameter id, section), eight at
/// most.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginPageSpec {
    pub name: String,
    pub knobs: Vec<(u32, String)>,
}

/// The object a list item (`shared_ptr`) holds.
fn px(item: &Value) -> Option<&Obj> {
    match item {
        Value::Obj(sp) => match sp.get("px") {
            Some(Value::Obj(o)) => Some(o),
            _ => None,
        },
        _ => None,
    }
}

fn px_mut(item: &mut Value) -> Result<&mut Obj, String> {
    match item {
        Value::Obj(sp) => match sp.get_mut("px") {
            Some(Value::Obj(o)) => Ok(o),
            _ => Err("bad plug-in page".into()),
        },
        _ => Err("bad plug-in page".into()),
    }
}

impl Document {
    /// Where the saved pages of the plug-in in slot `slot` of chain `c` are (their
    /// `items` list); none if the slot holds no plug-in.
    fn plugin_pages_path(&self, c: Chain, slot: usize) -> Option<Path> {
        let host = self.owner(Target::ChainModule(c, slot))?;
        match self.tree.at(&host) {
            Some(Value::Obj(m)) if m.type_name == "NI::MASCHINE::DATA::PluginHost" => {}
            _ => return None,
        }
        let items = self.field_in(&host, "parameters/unknown_48/objects/items")?;
        Some([host.as_slice(), &items].concat())
    }

    /// The plug-in's saved parameters: each one's id and object id.
    fn plugin_param_objs(&self, c: Chain, slot: usize) -> Vec<(u32, u64)> {
        let Some(items) = self.plugin_pages_path(c, slot) else { return Vec::new() };
        let Some(Value::List(list)) = self.tree.at(&items) else { return Vec::new() };
        let mut out = Vec::new();
        for page in list.iter().filter_map(px) {
            let Some(Value::Obj(arr)) = page.get("parameters") else { continue };
            for p in arr.fields.iter().filter(|(n, _)| n == "item").filter_map(|(_, v)| px(v)) {
                let index = p.get("index").and_then(Value::as_int).unwrap_or(UNUSED);
                if let Some(oid) = p.oid.filter(|_| (0..UNUSED).contains(&index)) {
                    out.push((index as u32, oid));
                }
            }
        }
        out
    }

    /// The object id of parameter `id` of the plug-in in slot `slot` of chain `c`, if
    /// it is on the plug-in's saved pages.
    pub fn plugin_param_oid(&self, c: Chain, slot: usize, id: u32) -> Option<u64> {
        self.plugin_param_objs(c, slot).into_iter().find(|x| x.0 == id).map(|x| x.1)
    }

    /// Which plug-in parameter object `param` is: its chain, slot and parameter id.
    pub fn plugin_param_at(&self, param: u64) -> Option<(Chain, usize, u32)> {
        let p = &self.project;
        let mut chains = vec![Chain::Master];
        for (g, grp) in p.groups.iter().enumerate() {
            chains.push(Chain::Group(g));
            chains.extend((0..grp.sounds.len()).map(|s| Chain::Sound(g, s)));
        }
        chains.into_iter().find_map(|c| {
            let n = self.chain_slots(c).map_or(0, <[_]>::len);
            (0..n).find_map(|i| self.plugin_param_objs(c, i).into_iter().find(|x| x.1 == param).map(|x| (c, i, x.0)))
        })
    }

    /// Save `pages` as the pages of the plug-in in slot `slot` of chain `c`, unless it
    /// has pages saved already (Maschine's, or saved before). Its pages are then these
    /// (`PluginRef::pages`).
    pub fn save_plugin_pages(&mut self, c: Chain, slot: usize, pages: &[PluginPageSpec]) -> Result<(), String> {
        let items = self.plugin_pages_path(c, slot).ok_or("the slot holds no plug-in")?;
        if !self.plugin_param_objs(c, slot).is_empty() {
            return Ok(());
        }
        let mut list = Vec::new();
        for spec in pages {
            let mut item = self.build(|b| b.item(PAGE))?;
            let page = px_mut(&mut item)?;
            if let Some(Value::Obj(n)) = page.find_mut("name")
                && let Some(v) = n.find_mut("value")
            {
                *v = Value::Str(spec.name.clone());
            }
            let mut params = Vec::new();
            for k in 0..8 {
                let mut p = self.build(|b| b.item(PARAM))?;
                let o = px_mut(&mut p)?;
                let (index, section) = spec.knobs.get(k).map_or((UNUSED, ""), |(id, s)| (*id as i64, s.as_str()));
                *o.get_mut("index").ok_or("bad plug-in parameter")? = Value::Int(index);
                *o.get_mut("section_name").ok_or("bad plug-in parameter")? = Value::Str(section.to_string());
                params.push(("item".to_string(), p));
            }
            let Some(Value::Obj(arr)) = page.get_mut("parameters") else { return Err("bad plug-in page".into()) };
            arr.fields.retain(|(n, _)| n != "item" && n != "count");
            arr.fields.push(("count".into(), Value::Int(8)));
            arr.fields.extend(params);
            list.push(item);
        }
        self.apply("Save Plug-in Pages", vec![Op::Set { path: items, value: Value::List(list) }])
    }
}

#[cfg(test)]
mod tests {
    use super::PluginPageSpec;
    use crate::{Chain, Document, NewModule, Target};

    fn with_plugin() -> Document {
        let mut d = Document::new_project().unwrap();
        let plugin = NewModule::Plugin { id: "Native Instruments:VST3:INSTR:0123".into(), name: "Synth".into(), chunk: Vec::new() };
        d.add_slot(0, 0, 0, &plugin).unwrap();
        d
    }

    fn pages() -> Vec<PluginPageSpec> {
        vec![
            PluginPageSpec { name: "Filter".into(), knobs: vec![(7, "Filter".into()), (8, String::new())] },
            PluginPageSpec { name: String::new(), knobs: (100..108).map(|i| (i, String::new())).collect() },
        ]
    }

    /// The pages saved are the plug-in's pages when the project is opened again, and
    /// each parameter on them is an object a macro can point at.
    #[test]
    fn saved_plugin_pages_make_parameters_objects() {
        let mut d = with_plugin();
        let c = Chain::Sound(0, 0);
        assert_eq!(d.plugin_param_oid(c, 0, 7), None, "no pages yet");
        d.save_plugin_pages(c, 0, &pages()).unwrap();
        let cutoff = d.plugin_param_oid(c, 0, 7).expect("saved");
        assert_eq!(d.plugin_param_at(cutoff), Some((c, 0, 7)));
        assert!(d.reaches(Target::Sound(0, 0), cutoff));
        assert!(d.reaches(Target::Group(0), cutoff));
        assert!(!d.reaches(Target::Sound(0, 1), cutoff), "another sound's macros");

        // (Unused knobs read as u32::MAX, as in Maschine's pages.)
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        let pr = back.project().groups[0].sounds[0].slots[0].plugin.clone().unwrap();
        let shown: Vec<(String, Vec<u32>)> =
            pr.pages.iter().map(|p| (p.name.clone(), p.params.iter().map(|k| k.id).filter(|&id| id != u32::MAX).collect())).collect();
        assert_eq!(shown[0], ("Filter".to_string(), vec![7, 8]));
        assert_eq!(shown[1].1, (100..108).collect::<Vec<_>>());
        assert_eq!(pr.pages[0].params[0].section, "Filter");
        assert_eq!(back.plugin_param_oid(c, 0, 103).and_then(|id| back.plugin_param_at(id)), Some((c, 0, 103)));
    }

    /// Pages saved already (Maschine's) stay as they are.
    #[test]
    fn saved_pages_stay() {
        let mut d = with_plugin();
        let c = Chain::Sound(0, 0);
        d.save_plugin_pages(c, 0, &pages()).unwrap();
        let before = d.plugin_param_oid(c, 0, 7);
        d.save_plugin_pages(c, 0, &[PluginPageSpec { name: "Other".into(), knobs: vec![(1, String::new())] }]).unwrap();
        assert_eq!(d.plugin_param_oid(c, 0, 7), before);
        assert_eq!(d.plugin_param_oid(c, 0, 1), None);
    }

    /// A plug-in parameter made a macro of its sound, and taken off again.
    #[test]
    fn a_plugin_parameter_is_a_macro() {
        let mut d = with_plugin();
        let c = Chain::Sound(0, 0);
        d.save_plugin_pages(c, 0, &pages()).unwrap();
        let cutoff = d.plugin_param_oid(c, 0, 7).unwrap();
        let snd = Target::Sound(0, 0);
        assert_eq!(d.add_macro_to(snd, cutoff).unwrap(), (0, 0));
        assert_eq!(d.macro_pages(snd)[0].macros[0].target, Some(cutoff));
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(back.macro_pages(snd)[0].macros[0].target.and_then(|id| back.plugin_param_at(id)), Some((c, 0, 7)));
        d.remove_macros_to(snd, cutoff).unwrap();
        assert_eq!(d.macro_pages(snd)[0].macros[0].target, None);
    }
}
