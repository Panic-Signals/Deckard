//! Macro controls (docs/channel-software.md §6): each channel (a sound, a group, the
//! master) has pages of eight macros, each forwarding to one parameter of that channel
//! or of a channel under it.

use super::{Document, Op, Target};
use mxprj::decode::{Obj, Path, Value};

/// One macro: the parameter it controls (by object id), its own name (empty: the
/// parameter's) and the section heading that starts at it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Macro {
    pub target: Option<u64>,
    pub name: String,
    pub section: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MacroPage {
    pub name: String,
    pub macros: Vec<Macro>,
}

impl Document {
    /// Where a channel's macro pages are (the `items` list of its `macro_pages`).
    fn macro_items(&self, channel: Target) -> Result<Path, String> {
        let owner = self.owner(channel).ok_or("no such channel")?;
        let at = self.field_in(&owner, "macro_pages/objects/items").ok_or("this channel has no macros")?;
        Ok([owner.as_slice(), &at].concat())
    }

    /// The page objects (`MacroPage`, inside their shared_ptr), with their paths.
    fn macro_page_objs(&self, channel: Target) -> Result<Vec<(Path, &Obj)>, String> {
        let items = self.macro_items(channel)?;
        let Some(Value::List(list)) = self.tree.at(&items) else { return Err("bad macro pages".into()) };
        let mut out = Vec::new();
        for (i, item) in list.iter().enumerate() {
            let Value::Obj(sp) = item else { continue };
            let Some(px) = sp.fields.iter().position(|(n, _)| n == "px") else { continue };
            if let Value::Obj(page) = &sp.fields[px].1 {
                out.push(([items.as_slice(), &[i as u32, px as u32]].concat(), page));
            }
        }
        Ok(out)
    }

    /// The macro pages of a channel (`Target::Song`, `Group(g)` or `Sound(g, s)`).
    pub fn macro_pages(&self, channel: Target) -> Vec<MacroPage> {
        let Ok(pages) = self.macro_page_objs(channel) else { return Vec::new() };
        pages
            .into_iter()
            .map(|(_, page)| {
                let name = match page.find("name") {
                    Some(Value::Obj(n)) => match n.find("value") {
                        Some(Value::Str(s)) => s.clone(),
                        _ => String::new(),
                    },
                    _ => String::new(),
                };
                let mut macros = Vec::new();
                if let Some(Value::Obj(params)) = page.get("parameters")
                    && let Some(Value::Obj(items)) = params.get("items")
                {
                    for (_, item) in &items.fields {
                        let fp = match item {
                            Value::Obj(sp) => match sp.get("px") {
                                Some(Value::Obj(fp)) => Some(fp),
                                _ => None,
                            },
                            _ => None,
                        };
                        let text = |k: &str| match fp.and_then(|f| f.get(k)) {
                            Some(Value::Str(s)) => s.clone(),
                            _ => String::new(),
                        };
                        let target = match fp.and_then(|f| f.get("target")) {
                            Some(Value::Ref(id)) => Some(*id),
                            Some(Value::Obj(o)) => o.oid,
                            _ => None,
                        };
                        macros.push(Macro { target, name: text("custom_name"), section: text("section_name") });
                    }
                }
                MacroPage { name, macros }
            })
            .collect()
    }

    /// The object id of parameter `field` of `target` (what a macro points at).
    pub fn param_oid(&self, target: Target, field: &str) -> Option<u64> {
        let owner = self.owner(target)?;
        let fp = self.field_in(&owner, field)?;
        match self.tree.at(&[owner.as_slice(), &fp].concat())? {
            Value::Obj(o) => o.oid,
            _ => None,
        }
    }

    /// Path of field `k` of macro `m` on page `page`.
    fn macro_field(&self, channel: Target, page: usize, m: usize, k: &str) -> Result<Path, String> {
        let pages = self.macro_page_objs(channel)?;
        let (path, obj) = pages.get(page).ok_or("no such macro page")?;
        let pi = obj.fields.iter().position(|(n, _)| n == "parameters").ok_or("bad macro page")?;
        let Value::Obj(params) = &obj.fields[pi].1 else { return Err("bad macro page".into()) };
        let ii = params.fields.iter().position(|(n, _)| n == "items").ok_or("bad macro page")?;
        let Value::Obj(items) = &params.fields[ii].1 else { return Err("bad macro page".into()) };
        let Some(Value::Obj(sp)) = items.fields.get(m).map(|(_, v)| v) else { return Err("no such macro".into()) };
        let px = sp.fields.iter().position(|(n, _)| n == "px").ok_or("bad macro")?;
        let Value::Obj(fp) = &sp.fields[px].1 else { return Err("bad macro".into()) };
        let fi = fp.fields.iter().position(|(n, _)| n == k).ok_or("bad macro")?;
        Ok([path.as_slice(), &[pi as u32, ii as u32, m as u32, px as u32, fi as u32]].concat())
    }

    /// Point macro `m` of page `page` at parameter `field` of `target` (`None`: empty
    /// it), with a name (empty: the parameter's).
    pub fn set_macro(&mut self, channel: Target, page: usize, m: usize, to: Option<(Target, &str)>, name: &str) -> Result<(), String> {
        let target = match to {
            Some((t, f)) => Some(self.param_oid(t, f).ok_or("that parameter can't be a macro")?),
            None => None,
        };
        self.set_macro_param(channel, page, m, target, name)
    }

    /// Point macro `m` of page `page` of `channel` at the parameter with object id
    /// `param` (`None`: reset it), named `name`.
    pub fn set_macro_param(&mut self, channel: Target, page: usize, m: usize, param: Option<u64>, name: &str) -> Result<(), String> {
        let ops = vec![
            Op::Set { path: self.macro_field(channel, page, m, "target")?, value: param.map_or(Value::Null, Value::Ref) },
            Op::Set { path: self.macro_field(channel, page, m, "custom_name")?, value: Value::Str(name.to_string()) },
        ];
        self.apply(if param.is_some() { "Set Macro" } else { "Reset Macro" }, ops)
    }

    /// Point host automation slot `slot` (0–127: the plug-in's parameter of that id) at
    /// parameter `field` of `target` (`None`: free it). Maschine as a plug-in then lets
    /// the host move that parameter (the Automation pane's Host tab).
    pub fn set_host_automation(&mut self, slot: usize, to: Option<(Target, &str)>) -> Result<(), String> {
        let param = match to {
            Some((t, f)) => Some(self.param_oid(t, f).ok_or("that parameter can't be automated")?),
            None => None,
        };
        self.set_host_slot(slot, param)
    }

    /// The object id of macro `m` of page `page` itself (a `ForwardParameter`: what
    /// MIDI learn and host automation of the macro point at).
    pub fn macro_oid(&self, channel: Target, page: usize, m: usize) -> Option<u64> {
        let path = self.macro_field(channel, page, m, "target").ok()?;
        match self.tree.at(&path[..path.len() - 1])? {
            Value::Obj(o) => o.oid,
            _ => None,
        }
    }

    /// Every parameter some macro controls (by object id), in any channel.
    pub fn macro_targets(&self) -> std::collections::HashSet<u64> {
        let p = &self.project;
        let mut channels = vec![Target::Song];
        for (g, grp) in p.groups.iter().enumerate() {
            channels.push(Target::Group(g));
            channels.extend((0..grp.sounds.len()).map(|s| Target::Sound(g, s)));
        }
        channels.into_iter().flat_map(|c| self.macro_pages(c)).flat_map(|p| p.macros).filter_map(|m| m.target).collect()
    }

    /// Whether `channel`'s macros can control parameter `param`: one of the channel's
    /// own or of a channel under it, and not a macro.
    pub fn reaches(&self, channel: Target, param: u64) -> bool {
        let (Some(owner), Some(at)) = (self.owner(channel), self.oid_path(param)) else { return false };
        let is_macro = matches!(self.tree.at(&at), Some(Value::Obj(o)) if o.type_name.ends_with("::ForwardParameter"));
        at.starts_with(&owner) && !is_macro
    }

    /// Put a copied macro (its target and name) into slot `m` of page `page`.
    pub fn paste_macro(&mut self, channel: Target, page: usize, m: usize, from: &Macro) -> Result<(), String> {
        if let Some(t) = from.target
            && !self.reaches(channel, t)
        {
            return Err("this channel's macros can't control that parameter".into());
        }
        let ops = vec![
            Op::Set { path: self.macro_field(channel, page, m, "target")?, value: from.target.map_or(Value::Null, Value::Ref) },
            Op::Set { path: self.macro_field(channel, page, m, "custom_name")?, value: Value::Str(from.name.clone()) },
        ];
        self.apply("Paste Macro", ops)
    }

    /// Rename macro `m` (empty: the parameter's name again).
    pub fn rename_macro(&mut self, channel: Target, page: usize, m: usize, name: &str) -> Result<(), String> {
        let path = self.macro_field(channel, page, m, "custom_name")?;
        self.apply("Rename Macro", vec![Op::Set { path, value: Value::Str(name.to_string()) }])
    }

    /// Name the section that starts at macro `m`.
    pub fn set_macro_section(&mut self, channel: Target, page: usize, m: usize, name: &str) -> Result<(), String> {
        let path = self.macro_field(channel, page, m, "section_name")?;
        self.apply("Set Section Name", vec![Op::Set { path, value: Value::Str(name.to_string()) }])
    }

    /// Make parameter `field` of `target` a macro of `channel`: the next free slot after
    /// the last one used, on a new page if they are all used. Returns (page, slot).
    pub fn add_macro(&mut self, channel: Target, target: Target, field: &str) -> Result<(usize, usize), String> {
        let id = self.param_oid(target, field).ok_or("that parameter can't be a macro")?;
        self.add_macro_to(channel, id)
    }

    /// Make the parameter with object id `param` a macro of `channel` (as
    /// [`Document::add_macro`]).
    pub fn add_macro_to(&mut self, channel: Target, param: u64) -> Result<(usize, usize), String> {
        let pages = self.macro_pages(channel);
        let last_used = pages
            .iter()
            .enumerate()
            .flat_map(|(p, pg)| pg.macros.iter().enumerate().map(move |(m, x)| (p, m, x.target.is_some())))
            .rfind(|x| x.2);
        let mut at = last_used.map_or((0, 0), |(p, m, _)| (p, m + 1));
        let mut out = at;
        self.transaction("Create Macro", |d| {
            if pages.is_empty() {
                at = (d.add_macro_page(channel)?, 0);
            } else if at.1 >= 8 {
                at = if at.0 + 1 < pages.len() { (at.0 + 1, 0) } else { (d.add_macro_page(channel)?, 0) };
            }
            out = at;
            let ops = vec![
                Op::Set { path: d.macro_field(channel, at.0, at.1, "target")?, value: Value::Ref(param) },
                Op::Set { path: d.macro_field(channel, at.0, at.1, "custom_name")?, value: Value::Str(String::new()) },
            ];
            d.apply("Set Macro", ops)
        })?;
        Ok(out)
    }

    /// Empty every macro of `channel` that points at parameter `field` of `target`.
    pub fn remove_macro_for(&mut self, channel: Target, target: Target, field: &str) -> Result<(), String> {
        let id = self.param_oid(target, field).ok_or("no such parameter")?;
        self.remove_macros_to(channel, id)
    }

    /// Empty every macro of `channel` that points at the parameter with object id `id`.
    pub fn remove_macros_to(&mut self, channel: Target, id: u64) -> Result<(), String> {
        let mut ops = Vec::new();
        for (p, page) in self.macro_pages(channel).iter().enumerate() {
            for (m, x) in page.macros.iter().enumerate() {
                if x.target == Some(id) {
                    ops.push(Op::Set { path: self.macro_field(channel, p, m, "target")?, value: Value::Null });
                    ops.push(Op::Set { path: self.macro_field(channel, p, m, "custom_name")?, value: Value::Str(String::new()) });
                }
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.apply("Remove Macro", ops)
    }

    /// Add an empty macro page at the end; returns its index.
    pub fn add_macro_page(&mut self, channel: Target) -> Result<usize, String> {
        let items = self.macro_items(channel)?;
        let Some(Value::List(list)) = self.tree.at(&items).cloned() else { return Err("bad macro pages".into()) };
        // A copy of a page (with new object ids), emptied; or a new one.
        let template = list
            .iter()
            .find(|v| matches!(v, Value::Obj(sp) if matches!(sp.get("px"), Some(Value::Obj(_)))))
            .cloned()
            .or_else(|| self.any_macro_page());
        let mut page = match template {
            Some(t) => self.fresh_copy(&t),
            None => self.build(|b| b.item("NI::MASCHINE::DATA::MacroPage"))?,
        };
        empty_page(&mut page);
        let mut list = list;
        list.push(page);
        let n = list.len() - 1;
        self.apply("Add Macro Page", vec![Op::Set { path: items, value: Value::List(list) }])?;
        Ok(n)
    }

    /// Some channel's first macro page (to copy).
    fn any_macro_page(&self) -> Option<Value> {
        let p = &self.project;
        let mut channels = vec![Target::Song];
        for (g, grp) in p.groups.iter().enumerate() {
            channels.push(Target::Group(g));
            channels.extend((0..grp.sounds.len()).map(|s| Target::Sound(g, s)));
        }
        channels.into_iter().find_map(|c| {
            let items = self.macro_items(c).ok()?;
            match self.tree.at(&items)? {
                Value::List(l) => l.iter().find(|v| matches!(v, Value::Obj(sp) if matches!(sp.get("px"), Some(Value::Obj(_))))).cloned(),
                _ => None,
            }
        })
    }

    /// Delete macro page `page` with its macros (the last page is emptied instead).
    pub fn remove_macro_page(&mut self, channel: Target, page: usize) -> Result<(), String> {
        let items = self.macro_items(channel)?;
        let Some(Value::List(list)) = self.tree.at(&items).cloned() else { return Err("bad macro pages".into()) };
        if page >= list.len() {
            return Err("no such macro page".into());
        }
        if list.len() == 1 {
            return self.clear_macro_page(channel, 0);
        }
        let mut list = list;
        let gone = list.remove(page);
        let mut ops = vec![Op::Set { path: items.clone(), value: Value::List(list.clone()) }];
        // The vector's current page: another one.
        let gone_id = match &gone {
            Value::Obj(sp) => match sp.get("px") {
                Some(Value::Obj(o)) => o.oid,
                _ => None,
            },
            _ => None,
        };
        let vector = &items[..items.len() - 2];
        if let Some(Value::Obj(ov)) = self.tree.at(vector)
            && let Some(ci) = ov.fields.iter().position(|(n, _)| n == "current")
            && matches!(&ov.fields[ci].1, Value::Ref(id) if Some(*id) == gone_id)
        {
            let keep = list.first().and_then(|v| match v {
                Value::Obj(sp) => match sp.get("px") {
                    Some(Value::Obj(o)) => o.oid,
                    _ => None,
                },
                _ => None,
            });
            if let Some(k) = keep {
                ops.push(Op::Set { path: [vector, &[ci as u32]].concat(), value: Value::Ref(k) });
            }
        }
        self.apply("Delete Macro Page", ops)
    }

    /// Empty page `page`: no macros, no names.
    fn clear_macro_page(&mut self, channel: Target, page: usize) -> Result<(), String> {
        let pages = self.macro_page_objs(channel)?;
        let (path, obj) = pages.get(page).ok_or("no such macro page")?;
        let mut v = Value::Obj((*obj).clone());
        empty_page(&mut v);
        let path = path.clone();
        self.apply("Clear Macros", vec![Op::Set { path, value: v }])
    }

    /// Delete all macro pages but an empty first one.
    pub fn clear_macros(&mut self, channel: Target) -> Result<(), String> {
        while self.macro_pages(channel).len() > 1 {
            let n = self.macro_pages(channel).len();
            self.remove_macro_page(channel, n - 1)?;
        }
        self.clear_macro_page(channel, 0)?;
        Ok(())
    }

    /// Rename macro page `page`.
    pub fn rename_macro_page(&mut self, channel: Target, page: usize, name: &str) -> Result<(), String> {
        let pages = self.macro_page_objs(channel)?;
        let (path, obj) = pages.get(page).ok_or("no such macro page")?;
        let np = obj.find_path("name").ok_or("bad macro page")?;
        let Some(Value::Obj(n)) = obj.at(&np) else { return Err("bad macro page".into()) };
        let vp = n.find_path("value").ok_or("bad macro page")?;
        let path = [path.as_slice(), &np, &vp].concat();
        self.apply("Rename Macro Page", vec![Op::Set { path, value: Value::Str(name.to_string()) }])
    }
}

/// Empty a macro page (or its shared_ptr): no name, no targets or names.
fn empty_page(v: &mut Value) {
    let Value::Obj(o) = v else { return };
    for (n, f) in o.fields.iter_mut() {
        match (n.as_str(), &mut *f) {
            ("target", f) => *f = Value::Null,
            ("custom_name" | "section_name", f) => *f = Value::Str(String::new()),
            ("value", Value::Str(s)) => s.clear(),
            (_, f @ Value::Obj(_)) => empty_page(f),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mxprj::decode::Value as V;

    /// A macro's own parameter (what MIDI learn and host automation point at); copy and
    /// paste carry the target and the name; a target the channel can't reach is refused.
    #[test]
    fn macro_slots_copy_paste_and_ids() {
        let mut d = Document::new_project().unwrap();
        let (snd, grp) = (Target::Sound(0, 0), Target::Group(0));
        d.add_macro(snd, snd, "level").unwrap();
        d.rename_macro(snd, 0, 0, "Vol").unwrap();
        let id = d.macro_oid(snd, 0, 0).unwrap();
        assert_ne!(Some(id), d.param_oid(snd, "level"), "the macro, not its target");
        assert_ne!(d.macro_oid(snd, 0, 1), Some(id));
        let m = d.macro_pages(snd)[0].macros[0].clone();
        d.paste_macro(snd, 0, 3, &m).unwrap();
        assert_eq!(d.macro_pages(snd)[0].macros[3], Macro { section: String::new(), ..m.clone() });
        // Into the group (it reaches its sounds), not into another sound.
        d.paste_macro(grp, 0, 0, &m).unwrap();
        assert_eq!(d.macro_pages(grp)[0].macros[0].target, d.param_oid(snd, "level"));
        assert!(d.paste_macro(Target::Sound(0, 1), 0, 0, &m).is_err());
        assert!(d.reaches(Target::Song, m.target.unwrap()));
        assert!(!d.reaches(snd, d.macro_oid(snd, 0, 0).unwrap()), "macros don't control macros");
        // An empty macro pastes as an empty one.
        d.paste_macro(snd, 0, 3, &Macro::default()).unwrap();
        assert_eq!(d.macro_pages(snd)[0].macros[3].target, None);
    }

    #[test]
    fn macros_assign_page_and_save() {
        let Some(mut d) = Document::open(std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj")).ok()
        else {
            return;
        };
        let snd = Target::Sound(0, 0);
        assert_eq!(d.macro_pages(snd).len(), 1);
        assert!(d.macro_pages(snd)[0].macros.iter().all(|m| m.target.is_none()));
        // The sound's level and pan, then a sampler parameter.
        assert_eq!(d.add_macro(snd, snd, "level").unwrap(), (0, 0));
        assert_eq!(d.add_macro(snd, snd, "pan").unwrap(), (0, 1));
        assert_eq!(d.add_macro(snd, Target::Sampler(0, 0), "cutoff").unwrap(), (0, 2));
        d.rename_macro(snd, 0, 1, "Width").unwrap();
        // A group macro for one of its sounds; a master macro for a group.
        d.add_macro(Target::Group(0), Target::Sound(0, 1), "level").unwrap();
        d.add_macro(Target::Song, Target::GroupStrip(3), "pan").unwrap();
        // A second page, renamed; fill the first so the next macro goes to page 2.
        let p2 = d.add_macro_page(snd).unwrap();
        d.rename_macro_page(snd, p2, "More").unwrap();
        for f in ["input_gain", "aux1_level", "aux2_level", "transpose", "swing_amount"] {
            d.add_macro(snd, snd, f).unwrap();
        }
        assert_eq!(d.add_macro(snd, snd, "audio_mute").unwrap(), (1, 0));
        let check = |d: &Document| {
            let pages = d.macro_pages(snd);
            assert_eq!(pages.len(), 2);
            assert_eq!(pages[1].name, "More");
            assert_eq!(pages[0].macros[0].target, d.param_oid(snd, "level"));
            assert_eq!(pages[0].macros[1].name, "Width");
            assert_eq!(pages[0].macros[2].target, d.param_oid(Target::Sampler(0, 0), "cutoff"));
            assert_eq!(pages[1].macros[0].target, d.param_oid(snd, "audio_mute"));
            assert_eq!(d.macro_pages(Target::Group(0))[0].macros[0].target, d.param_oid(Target::Sound(0, 1), "level"));
            assert_eq!(d.macro_pages(Target::Song)[0].macros[0].target, d.param_oid(Target::GroupStrip(3), "pan"));
        };
        check(&d);
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        check(&back);
        // Remove: a page, a macro by its parameter, all.
        d.remove_macro_page(snd, 1).unwrap();
        assert_eq!(d.macro_pages(snd).len(), 1);
        d.remove_macro_for(snd, snd, "pan").unwrap();
        assert!(d.macro_pages(snd)[0].macros[1].target.is_none());
        d.clear_macros(snd).unwrap();
        assert!(d.macro_pages(snd)[0].macros.iter().all(|m| m.target.is_none() && m.name.is_empty()));
        let _ = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        let _ = V::Null;
    }
}
