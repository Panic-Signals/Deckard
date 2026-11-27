//! Plug-in parameters as objects: Maschine's `PluginParameter`s, eight to each of a
//! `PluginHost`'s saved pages (`parameters.unknown_48`), are what a macro points at to
//! control a plug-in's parameter. Maschine saves the pages it shows; DECKARD saves them
//! when one of their parameters is first made a macro.

use crate::{Chain, Document, Op, Target};
use mxprj::decode::{Obj, Path, Value};
use std::collections::HashSet;

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
    pub custom_names: Vec<String>,
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

fn custom_name_field(o: &Obj) -> &'static str {
    if o.get("custom_name").is_some() { "custom_name" } else { "custom_name_nis_mapping" }
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

    /// Whether the plug-in in slot `slot` of chain `c` already has saved pages in the
    /// project (Maschine's, NKS, or edited here).
    pub fn has_saved_plugin_pages(&self, c: Chain, slot: usize) -> bool {
        self.plugin_pages_path(c, slot).and_then(|p| self.tree.at(&p)).is_some_and(|v| matches!(v, Value::List(list) if !list.is_empty()))
    }

    /// The saved `PluginPage` objects (inside their `shared_ptr`), with their paths.
    fn plugin_page_objs(&self, c: Chain, slot: usize) -> Result<Vec<(Path, &Obj)>, String> {
        let items = self.plugin_pages_path(c, slot).ok_or("the slot holds no plug-in")?;
        let Some(Value::List(list)) = self.tree.at(&items) else { return Err("bad plug-in pages".into()) };
        let mut out = Vec::new();
        for (i, item) in list.iter().enumerate() {
            let Value::Obj(sp) = item else { continue };
            let Some(px_i) = sp.fields.iter().position(|(n, _)| n == "px") else { continue };
            if let Value::Obj(page) = &sp.fields[px_i].1 {
                out.push(([items.as_slice(), &[i as u32, px_i as u32]].concat(), page));
            }
        }
        Ok(out)
    }

    /// The `PluginParameter` object at knob `knob` (0–7) of saved page `page`, with its path.
    fn plugin_param_obj(&self, c: Chain, slot: usize, page: usize, knob: usize) -> Result<(Path, &Obj), String> {
        let pages = self.plugin_page_objs(c, slot)?;
        let (path, obj) = pages.get(page).ok_or("no such plug-in page")?;
        let pi = obj.fields.iter().position(|(n, _)| n == "parameters").ok_or("bad plug-in page")?;
        let Value::Obj(arr) = &obj.fields[pi].1 else { return Err("bad plug-in page".into()) };
        let (item_i, (_, item)) =
            arr.fields.iter().enumerate().filter(|(_, (n, _))| n == "item").nth(knob).ok_or("no such parameter slot")?;
        let Value::Obj(sp) = item else { return Err("bad plug-in parameter".into()) };
        let px_i = sp.fields.iter().position(|(n, _)| n == "px").ok_or("bad plug-in parameter")?;
        let Value::Obj(param) = &sp.fields[px_i].1 else { return Err("bad plug-in parameter".into()) };
        Ok(([path.as_slice(), &[pi as u32, item_i as u32, px_i as u32]].concat(), param))
    }

    /// Build a `shared_ptr<PluginPage>` populated from `spec` (eight `PluginParameter` slots).
    fn build_plugin_page(&mut self, spec: &PluginPageSpec) -> Result<Value, String> {
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
            let custom = spec.custom_names.get(k).map_or("", String::as_str);
            let custom_field = custom_name_field(o);
            *o.get_mut("index").ok_or("bad plug-in parameter")? = Value::Int(index);
            *o.get_mut(custom_field).ok_or("bad plug-in parameter")? = Value::Str(custom.to_string());
            *o.get_mut("section_name").ok_or("bad plug-in parameter")? = Value::Str(section.to_string());
            params.push(("item".to_string(), p));
        }
        let Some(Value::Obj(arr)) = page.get_mut("parameters") else { return Err("bad plug-in page".into()) };
        arr.fields.retain(|(n, _)| n != "item" && n != "count");
        arr.fields.push(("count".into(), Value::Int(8)));
        arr.fields.extend(params);
        Ok(item)
    }

    /// Unlearn MIDI and host automation for any removed parameter OIDs.
    fn drop_remote_for(&mut self, gone: &HashSet<u64>) -> Result<(), String> {
        let mut ids: Vec<u64> = gone.iter().copied().collect();
        ids.sort_unstable();
        for id in ids {
            if self.midi_key_for(id).is_some() {
                self.unlearn_midi(id)?;
            }
            if self.host_slot_for(id).is_some() {
                self.disable_host_automation(id)?;
            }
        }
        Ok(())
    }

    /// Ops to keep an `ObjectVector<PluginPage>`'s `current` and `selection` valid after
    /// removing pages whose OIDs are in `removed`, pointing `current` at `fallback` if needed.
    fn fix_page_vector_refs(&self, items: &Path, removed: &HashSet<u64>, fallback: Option<u64>) -> Vec<Op> {
        let mut ops = Vec::new();
        let vector = &items[..items.len() - 2];
        let Some(Value::Obj(ov)) = self.tree.at(vector) else { return ops };
        if let Some(ci) = ov.fields.iter().position(|(n, _)| n == "current")
            && matches!(&ov.fields[ci].1, Value::Ref(id) if removed.contains(id))
        {
            ops.push(Op::Set { path: [vector, &[ci as u32]].concat(), value: fallback.map_or(Value::Null, Value::Ref) });
        }
        if let Some(si) = ov.fields.iter().position(|(n, _)| n == "selection")
            && let Value::Obj(sel) = &ov.fields[si].1
            && let Some(ii) = sel.fields.iter().position(|(n, _)| n == "items")
            && let Value::List(sel_items) = &sel.fields[ii].1
        {
            let kept: Vec<Value> = sel_items.iter().filter(|v| !matches!(v, Value::Ref(id) if removed.contains(id))).cloned().collect();
            if kept.len() != sel_items.len() {
                ops.push(Op::Set { path: [vector, &[si as u32, ii as u32]].concat(), value: Value::List(kept) });
            }
        }
        ops
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
        if self.has_saved_plugin_pages(c, slot) {
            return Ok(());
        }
        let mut list = Vec::with_capacity(pages.len());
        for spec in pages {
            list.push(self.build_plugin_page(spec)?);
        }
        self.apply("Save Plug-in Pages", vec![Op::Set { path: items, value: Value::List(list) }])
    }

    /// Assign VST/AU parameter `param` (`None`: reset / unassign the slot) to knob
    /// `knob` (0–7) of saved page `page`, with optional custom `name` (empty: the
    /// plug-in parameter's own name).
    pub fn set_plugin_param(
        &mut self,
        c: Chain,
        slot: usize,
        page: usize,
        knob: usize,
        param: Option<u32>,
        name: &str,
    ) -> Result<(), String> {
        let (path, obj) = self.plugin_param_obj(c, slot, page, knob)?;
        let old_index = obj.get("index").and_then(Value::as_int).unwrap_or(UNUSED);
        let old_oid = obj.oid;
        let idx_fi = obj.fields.iter().position(|(n, _)| n == "index").ok_or("bad plug-in parameter")?;
        let cf = custom_name_field(obj);
        let name_fi = obj.fields.iter().position(|(n, _)| n == cf).ok_or("bad plug-in parameter")?;
        let sec_fi = obj.fields.iter().position(|(n, _)| n == "section_name").ok_or("bad plug-in parameter")?;
        let new_index = param.map_or(UNUSED, i64::from);
        let mut ops = vec![
            Op::Set { path: [path.as_slice(), &[idx_fi as u32]].concat(), value: Value::Int(new_index) },
            Op::Set {
                path: [path.as_slice(), &[name_fi as u32]].concat(),
                value: Value::Str(if param.is_some() { name.to_string() } else { String::new() }),
            },
        ];
        if param.is_none() {
            ops.push(Op::Set { path: [path.as_slice(), &[sec_fi as u32]].concat(), value: Value::Str(String::new()) });
        }
        let changed_target = (0..UNUSED).contains(&old_index) && old_index != new_index;
        let gone: HashSet<u64> = if changed_target { old_oid.into_iter().collect() } else { HashSet::new() };
        let label = if param.is_some() { "Assign Parameter" } else { "Reset Parameter" };
        self.transaction(label, |d| {
            d.drop_remote_for(&gone)?;
            let mut all_ops = d.drop_automation_ops(&gone, &path)?;
            all_ops.extend(ops);
            d.apply(label, all_ops)
        })
    }

    /// Rename knob `knob` (0–7) on saved page `page` (empty: restore the plug-in's name).
    pub fn rename_plugin_param(&mut self, c: Chain, slot: usize, page: usize, knob: usize, name: &str) -> Result<(), String> {
        let (path, obj) = self.plugin_param_obj(c, slot, page, knob)?;
        let cf = custom_name_field(obj);
        let fi = obj.fields.iter().position(|(n, _)| n == cf).ok_or("bad plug-in parameter")?;
        self.apply(
            "Rename Parameter",
            vec![Op::Set { path: [path.as_slice(), &[fi as u32]].concat(), value: Value::Str(name.to_string()) }],
        )
    }

    /// Set the section heading starting at knob `knob` (0–7) on saved page `page`.
    pub fn set_plugin_param_section(&mut self, c: Chain, slot: usize, page: usize, knob: usize, section: &str) -> Result<(), String> {
        let (path, obj) = self.plugin_param_obj(c, slot, page, knob)?;
        let fi = obj.fields.iter().position(|(n, _)| n == "section_name").ok_or("bad plug-in parameter")?;
        self.apply(
            "Set Section Name",
            vec![Op::Set { path: [path.as_slice(), &[fi as u32]].concat(), value: Value::Str(section.to_string()) }],
        )
    }

    /// Rename saved plug-in parameter page `page`.
    pub fn rename_plugin_page(&mut self, c: Chain, slot: usize, page: usize, name: &str) -> Result<(), String> {
        let pages = self.plugin_page_objs(c, slot)?;
        let (path, obj) = pages.get(page).ok_or("no such plug-in page")?;
        let np = obj.find_path("name").ok_or("bad plug-in page")?;
        let Some(Value::Obj(n)) = obj.at(&np) else { return Err("bad plug-in page".into()) };
        let vp = n.find_path("value").ok_or("bad plug-in page")?;
        let path = [path.as_slice(), &np, &vp].concat();
        self.apply("Rename Parameter Page", vec![Op::Set { path, value: Value::Str(name.to_string()) }])
    }

    /// Append a new empty plug-in parameter page (eight unassigned slots); returns its index.
    pub fn add_plugin_page(&mut self, c: Chain, slot: usize) -> Result<usize, String> {
        let items = self.plugin_pages_path(c, slot).ok_or("the slot holds no plug-in")?;
        let Some(Value::List(mut list)) = self.tree.at(&items).cloned() else { return Err("bad plug-in pages".into()) };
        let page = self.build_plugin_page(&PluginPageSpec::default())?;
        list.push(page);
        let idx = list.len() - 1;
        self.apply("Add Parameter Page", vec![Op::Set { path: items, value: Value::List(list) }])?;
        Ok(idx)
    }

    /// Move saved plug-in parameter page `from` to index `to`, keeping every parameter
    /// object id (and its macros/automation) intact.
    pub fn move_plugin_page(&mut self, c: Chain, slot: usize, from: usize, to: usize) -> Result<(), String> {
        let items = self.plugin_pages_path(c, slot).ok_or("the slot holds no plug-in")?;
        let Some(Value::List(mut list)) = self.tree.at(&items).cloned() else { return Err("bad plug-in pages".into()) };
        if from >= list.len() || to >= list.len() {
            return Err("no such plug-in page".into());
        }
        if from == to {
            return Ok(());
        }
        let page = list.remove(from);
        list.insert(to, page);
        self.apply("Move Parameter Page", vec![Op::Set { path: items, value: Value::List(list) }])
    }

    /// Delete saved plug-in parameter page `page` (the last remaining page is cleared instead).
    pub fn remove_plugin_page(&mut self, c: Chain, slot: usize, page: usize) -> Result<(), String> {
        let items = self.plugin_pages_path(c, slot).ok_or("the slot holds no plug-in")?;
        let Some(Value::List(mut list)) = self.tree.at(&items).cloned() else { return Err("bad plug-in pages".into()) };
        if page >= list.len() {
            return Err("no such plug-in page".into());
        }
        if list.len() == 1 {
            return self.clear_plugin_pages(c, slot);
        }
        let gone_page = list.remove(page);
        let gone = crate::automation::defined_ids(&gone_page);
        let fallback = list.first().and_then(px).and_then(|o| o.oid);
        let vector = items[..items.len() - 2].to_vec();
        self.transaction("Delete Parameter Page", |d| {
            d.drop_remote_for(&gone)?;
            let mut ops = d.drop_automation_ops(&gone, &vector)?;
            ops.push(Op::Set { path: items.clone(), value: Value::List(list.clone()) });
            ops.extend(d.fix_page_vector_refs(&items, &gone, fallback));
            d.apply("Delete Parameter Page", ops)
        })
    }

    /// Replace all saved plug-in parameter pages with one empty page.
    pub fn clear_plugin_pages(&mut self, c: Chain, slot: usize) -> Result<(), String> {
        let items = self.plugin_pages_path(c, slot).ok_or("the slot holds no plug-in")?;
        let Some(Value::List(old_list)) = self.tree.at(&items).cloned() else { return Err("bad plug-in pages".into()) };
        let mut gone = HashSet::new();
        for item in &old_list {
            gone.extend(crate::automation::defined_ids(item));
        }
        let fresh = self.build_plugin_page(&PluginPageSpec::default())?;
        let fallback = px(&fresh).and_then(|o| o.oid);
        let vector = items[..items.len() - 2].to_vec();
        self.transaction("Clear Parameter Pages", |d| {
            d.drop_remote_for(&gone)?;
            let mut ops = d.drop_automation_ops(&gone, &vector)?;
            ops.push(Op::Set { path: items.clone(), value: Value::List(vec![fresh.clone()]) });
            ops.extend(d.fix_page_vector_refs(&items, &gone, fallback));
            d.apply("Clear Parameter Pages", ops)
        })
    }

    /// Remove all saved plug-in parameter pages so the plug-in reverts to its default
    /// (automatic / NKS / mapping) pages.
    pub fn reset_plugin_pages(&mut self, c: Chain, slot: usize) -> Result<(), String> {
        let items = self.plugin_pages_path(c, slot).ok_or("the slot holds no plug-in")?;
        let Some(Value::List(old_list)) = self.tree.at(&items).cloned() else { return Err("bad plug-in pages".into()) };
        if old_list.is_empty() {
            return Ok(());
        }
        let mut gone = HashSet::new();
        for item in &old_list {
            gone.extend(crate::automation::defined_ids(item));
        }
        let vector = items[..items.len() - 2].to_vec();
        self.transaction("Reset Parameter Pages", |d| {
            d.drop_remote_for(&gone)?;
            let mut ops = d.drop_automation_ops(&gone, &vector)?;
            ops.push(Op::Set { path: items.clone(), value: Value::List(Vec::new()) });
            ops.extend(d.fix_page_vector_refs(&items, &gone, None));
            d.apply("Reset Parameter Pages", ops)
        })
    }

    /// Ensure parameter `id` of the plug-in in slot `slot` of `c` has a saved
    /// `PluginParameter` object so modulation can reference it, even when the preset
    /// already saved partial NKS pages that omitted `id`.
    pub fn ensure_plugin_param(&mut self, c: Chain, slot: usize, id: u32, fallback_pages: &[PluginPageSpec]) -> Result<(), String> {
        if self.plugin_param_oid(c, slot, id).is_some() {
            return Ok(());
        }
        if !self.has_saved_plugin_pages(c, slot) {
            return self.save_plugin_pages(c, slot, fallback_pages);
        }
        let items_path = self.plugin_pages_path(c, slot).ok_or("the slot holds no plug-in")?;
        let Some(Value::List(mut list)) = self.tree.at(&items_path).cloned() else {
            return Err("bad plug-in pages".into());
        };
        // Reuse a trailing unnamed overflow page's unused slot if one exists; otherwise append one.
        let mut placed = false;
        if let Some(last) = list.last_mut().and_then(|v| px_mut(v).ok()) {
            let unnamed =
                matches!(last.find("name"), Some(Value::Obj(n)) if matches!(n.find("value"), Some(Value::Str(s)) if s.is_empty()));
            if unnamed && let Some(Value::Obj(arr)) = last.get_mut("parameters") {
                for (_, v) in arr.fields.iter_mut().filter(|(n, _)| n == "item") {
                    if let Ok(o) = px_mut(v)
                        && o.get("index").and_then(Value::as_int) == Some(UNUSED)
                    {
                        *o.get_mut("index").ok_or("bad plug-in parameter")? = Value::Int(id as i64);
                        placed = true;
                        break;
                    }
                }
            }
        }
        if !placed {
            let item = self.build_plugin_page(&PluginPageSpec {
                name: String::new(),
                knobs: vec![(id, String::new())],
                custom_names: Vec::new(),
            })?;
            list.push(item);
        }
        self.apply("Save Plug-in Pages", vec![Op::Set { path: items_path, value: Value::List(list) }])
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
            PluginPageSpec { name: "Filter".into(), knobs: vec![(7, "Filter".into()), (8, String::new())], custom_names: Vec::new() },
            PluginPageSpec { name: String::new(), knobs: (100..108).map(|i| (i, String::new())).collect(), custom_names: Vec::new() },
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
        d.save_plugin_pages(c, 0, &[PluginPageSpec { name: "Other".into(), knobs: vec![(1, String::new())], custom_names: Vec::new() }])
            .unwrap();
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

    #[test]
    fn ensure_plugin_param_appends_to_partial_saved_pages() {
        let mut d = with_plugin();
        let c = Chain::Sound(0, 0);
        // Preset saved only parameters 7 and 8 on a named NKS page.
        d.save_plugin_pages(
            c,
            0,
            &[PluginPageSpec { name: "NKS".into(), knobs: vec![(7, String::new()), (8, String::new())], custom_names: Vec::new() }],
        )
        .unwrap();
        let seven = d.plugin_param_oid(c, 0, 7).unwrap();
        assert_eq!(d.plugin_param_oid(c, 0, 169), None);

        // Modulating an extra mapped parameter (e.g. DJFAM Step Pitch 169, Prob 59) creates an OID without altering existing ones.
        d.ensure_plugin_param(c, 0, 169, &[]).unwrap();
        d.ensure_plugin_param(c, 0, 59, &[]).unwrap();
        assert_eq!(d.plugin_param_oid(c, 0, 7), Some(seven));
        assert!(d.plugin_param_oid(c, 0, 169).is_some());
        assert!(d.plugin_param_oid(c, 0, 59).is_some());
    }

    /// Assign, rename, section, reset parameters, and add, rename, delete, clear and
    /// reset pages — all undoable and surviving an `.mxprj` round-trip.
    #[test]
    fn edit_plugin_pages_and_parameters_roundtrip_and_undo() {
        let mut d = with_plugin();
        let c = Chain::Sound(0, 0);
        assert!(!d.has_saved_plugin_pages(c, 0));

        // Save initial pages, then add a blank third page and rename it.
        d.save_plugin_pages(c, 0, &pages()).unwrap();
        assert!(d.has_saved_plugin_pages(c, 0));
        let p2 = d.add_plugin_page(c, 0).unwrap();
        assert_eq!(p2, 2);
        d.rename_plugin_page(c, 0, p2, "Custom").unwrap();

        // Assign a parameter into slot 3 (leaving slots 0..2 empty), rename it, add a section.
        d.set_plugin_param(c, 0, p2, 3, Some(55), "").unwrap();
        d.rename_plugin_param(c, 0, p2, 3, "Warmth").unwrap();
        d.set_plugin_param_section(c, 0, p2, 3, "Tone").unwrap();

        let check = |doc: &Document| {
            let pr = doc.project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap();
            assert_eq!(pr.pages.len(), 3);
            assert_eq!(pr.pages[2].name, "Custom");
            assert_eq!(pr.pages[2].params[0].id, u32::MAX);
            assert_eq!(pr.pages[2].params[3].id, 55);
            assert_eq!(pr.pages[2].params[3].name, "Warmth");
            assert_eq!(pr.pages[2].params[3].section, "Tone");
        };
        check(&d);
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        check(&back);

        // Undo rename restores empty custom name; redo restores "Warmth".
        assert!(d.undo());
        assert_eq!(d.project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap().pages[2].params[3].section, "");
        assert!(d.redo());
        check(&d);

        // Reset parameter clears id, custom name, and section.
        d.set_plugin_param(c, 0, p2, 3, None, "").unwrap();
        let pr = d.project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap();
        assert_eq!(pr.pages[2].params[3].id, u32::MAX);
        assert_eq!(pr.pages[2].params[3].name, "");
        assert_eq!(pr.pages[2].params[3].section, "");

        // Delete page 1, then clear all to 1 empty page, then reset to default (0 saved pages).
        d.remove_plugin_page(c, 0, 1).unwrap();
        assert_eq!(d.project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap().pages.len(), 2);
        d.clear_plugin_pages(c, 0).unwrap();
        assert!(d.has_saved_plugin_pages(c, 0));
        let pr = d.project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap();
        assert_eq!(pr.pages.len(), 1);
        assert!(pr.pages[0].params.iter().all(|k| k.id == u32::MAX));
        // Deleting the last page keeps 1 empty page.
        d.remove_plugin_page(c, 0, 0).unwrap();
        assert_eq!(d.project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap().pages.len(), 1);

        // Reset to default drops all saved pages.
        d.reset_plugin_pages(c, 0).unwrap();
        assert!(!d.has_saved_plugin_pages(c, 0));
        assert!(d.project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap().pages.is_empty());
        let _ = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
    }

    /// Resetting a parameter or deleting its page drops macros, MIDI learn and host
    /// automation pointing at it, so saving never leaves dangling references.
    #[test]
    fn deleting_page_or_resetting_param_cleans_up_references() {
        let mut d = with_plugin();
        let c = Chain::Sound(0, 0);
        let snd = Target::Sound(0, 0);
        d.save_plugin_pages(c, 0, &pages()).unwrap();

        let p7 = d.plugin_param_oid(c, 0, 7).unwrap();
        let p100 = d.plugin_param_oid(c, 0, 100).unwrap();
        d.add_macro_to(snd, p7).unwrap();
        d.learn_midi(p7, 74).unwrap();
        d.enable_host_automation(p7).unwrap();
        d.add_macro_to(snd, p100).unwrap();
        d.learn_midi(p100, 75).unwrap();

        // Resetting slot 0 of page 0 drops p7's macro, MIDI and host mappings.
        d.set_plugin_param(c, 0, 0, 0, None, "").unwrap();
        assert_eq!(d.macro_pages(snd)[0].macros[0].target, None);
        assert_eq!(d.midi_key_for(p7), None);
        assert_eq!(d.host_slot_for(p7), None);

        // Deleting page 1 drops p100's macro and MIDI mapping and encodes cleanly.
        d.remove_plugin_page(c, 0, 1).unwrap();
        assert_eq!(d.macro_pages(snd)[0].macros[1].target, None);
        assert_eq!(d.midi_key_for(p100), None);
        let _ = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
    }

    /// Moving a parameter page reorders pages while preserving parameter OIDs, macros,
    /// undo and `.mxprj` serialization.
    #[test]
    fn moving_plugin_pages_preserves_oids_and_roundtrips() {
        let mut d = with_plugin();
        let c = Chain::Sound(0, 0);
        let snd = Target::Sound(0, 0);
        d.save_plugin_pages(c, 0, &pages()).unwrap();
        let p2 = d.add_plugin_page(c, 0).unwrap();
        d.rename_plugin_page(c, 0, p2, "Custom").unwrap();
        let p7 = d.plugin_param_oid(c, 0, 7).unwrap();
        d.add_macro_to(snd, p7).unwrap();

        // Move page 0 ("Filter") to index 2 -> [page1, "Custom", "Filter"].
        d.move_plugin_page(c, 0, 0, 2).unwrap();
        let names = |doc: &Document| -> Vec<String> {
            doc.project().groups[0].sounds[0].slots[0].plugin.as_ref().unwrap().pages.iter().map(|p| p.name.clone()).collect()
        };
        assert_eq!(names(&d), vec!["", "Custom", "Filter"]);
        assert_eq!(d.plugin_param_oid(c, 0, 7), Some(p7));
        assert_eq!(d.macro_pages(snd)[0].macros[0].target, Some(p7));

        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(names(&back), vec!["", "Custom", "Filter"]);
        assert_eq!(back.macro_pages(snd)[0].macros[0].target.and_then(|id| back.plugin_param_at(id)), Some((c, 0, 7)));

        // Undo puts "Filter" back at index 0.
        assert!(d.undo());
        assert_eq!(names(&d), vec!["Filter", "", "Custom"]);
    }
}
