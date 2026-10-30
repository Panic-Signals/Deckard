//! Set Macro on a plug-in's page (SHIFT + MACRO, a knob touched): the plug-in's
//! parameter becomes a macro of the channel asked for, or stops being one. The
//! plug-in's pages are saved first, as shown (Maschine saves the pages it shows), so
//! the parameter is an object a macro can point at.

use crate::layout::channel::Level;
use crate::layout::{self, Edit};
use deckard_doc::{Chain, PluginPageSpec, Target};

impl crate::App {
    /// Set Macro for knob `pos` when it is a plug-in's (`level`: 0 the master, 1 the
    /// focused group, 2 the focused sound). Whether it was a plug-in's.
    pub(crate) fn set_plugin_macro(&mut self, pos: usize, level: usize) -> bool {
        let (Some(doc), Some(slot)) = (self.doc.clone(), self.slot) else { return false };
        if self.channel {
            return false;
        }
        let edit =
            layout::slot_page(self, doc.borrow().project(), slot).and_then(|pg| pg.knobs.get(pos).cloned().flatten()).and_then(|k| k.3);
        let param = match edit {
            Some(Edit::Plugin(_, id, _) | Edit::PluginChoice(_, id, ..)) => id,
            _ => return false,
        };
        let ch = match level {
            0 => Target::Song,
            1 => Target::Group(self.group),
            _ => Target::Sound(self.group, self.sound),
        };
        self.toggle_plugin_macro(ch, self.chain(), slot, param);
        true
    }

    /// The pages of the plug-in in slot `slot` of `chain` (to save before one of its
    /// parameters is pointed at): as shown if it's the one shown, else as it would show
    /// them; none if it isn't running.
    pub(crate) fn plugin_pages(&self, chain: Chain, slot: usize) -> Vec<PluginPageSpec> {
        let spec = |pages: &[crate::ViewPage]| {
            pages
                .iter()
                .map(|pg| PluginPageSpec {
                    name: pg.name.clone(),
                    knobs: pg.knobs.iter().map(|(info, _, section)| (info.id, section.clone())).collect(),
                })
                .collect()
        };
        match self.plugin_view.as_ref().filter(|v| v.key == (chain, slot)) {
            Some(v) => spec(&v.pages),
            None => spec(&self.view_pages_of(chain, slot)),
        }
    }

    /// The pages the plug-in in slot `slot` of `chain` would show (empty if it isn't
    /// running, or busy).
    fn view_pages_of(&self, chain: Chain, slot: usize) -> Vec<crate::ViewPage> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let saved = doc.borrow().chain_slots(chain).and_then(|s| Some(s.get(slot)?.plugin.as_ref()?.pages.clone())).unwrap_or_default();
        let Some(p) = self.samples.plugins.existing(self.plugin_slot_key(chain, slot)) else { return Vec::new() };
        p.try_lock().map(|p| crate::view_pages(&p, &saved)).unwrap_or_default()
    }

    /// The plug-ins' parameters a macro of `level` can control (its own chain's, and
    /// its sounds' for a group or its groups' for the master): (menu path, (chain,
    /// slot, parameter id)), the path being the plug-in ("1 - name"), its page and the
    /// knob's name, under the sound or group as the macro target menu has them.
    pub(crate) fn plugin_targets(&self, level: Level) -> Vec<(Vec<String>, (Chain, usize, u32))> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let d = doc.borrow();
        let p = d.project();
        let mut chains: Vec<(Chain, Vec<String>)> = Vec::new();
        match level {
            Level::Sound(g, s) => chains.push((Chain::Sound(g, s), Vec::new())),
            Level::Group(g) => {
                chains.push((Chain::Group(g), Vec::new()));
                for (s, snd) in p.groups[g].sounds.iter().enumerate() {
                    let name = if snd.name.is_empty() { format!("Sound {}", s + 1) } else { snd.name.clone() };
                    chains.push((Chain::Sound(g, s), vec!["Sounds".into(), format!("S{} - {name}", s + 1)]));
                }
            }
            Level::Master => {
                chains.push((Chain::Master, Vec::new()));
                for (g, grp) in p.groups.iter().enumerate() {
                    let id = layout::group_id(g);
                    let name = if grp.name.is_empty() { format!("Group {id}") } else { grp.name.clone() };
                    chains.push((Chain::Group(g), vec!["Groups".into(), format!("{id} - {name}")]));
                }
            }
        }
        let mut out = Vec::new();
        for (chain, prefix) in chains {
            let slots = d.chain_slots(chain).unwrap_or_default();
            for (i, slot) in slots.iter().enumerate() {
                let Some(plugin) = &slot.plugin else { continue };
                // (The pages shown, if it's the one shown: its values may be newer.)
                let owned;
                let pages: &[crate::ViewPage] = match self.plugin_view.as_ref().filter(|v| v.key == (chain, i)) {
                    Some(v) => &v.pages,
                    None => {
                        owned = self.view_pages_of(chain, i);
                        &owned
                    }
                };
                for (pi, page) in pages.iter().enumerate() {
                    let page_name = if page.name.is_empty() { format!("Page {}", pi + 1) } else { page.name.clone() };
                    for (info, name, _) in &page.knobs {
                        let mut path = prefix.clone();
                        let knob = if name.is_empty() { info.title.clone() } else { name.clone() };
                        path.extend([format!("{} - {}", i + 1, plugin.name), page_name.clone(), knob]);
                        out.push((path, (chain, i, info.id)));
                    }
                }
            }
        }
        out
    }

    /// Point macro `at` at parameter `param` of the plug-in in slot `slot` of `chain`:
    /// one undo step, its pages saved first.
    pub(crate) fn assign_plugin_macro(&mut self, (ch, page, m): crate::remote::MacroSlot, chain: Chain, slot: usize, param: u32) {
        let Some(doc) = self.doc.clone() else { return };
        let pages = self.plugin_pages(chain, slot);
        let r = doc.borrow_mut().transaction("Set Macro", |d| {
            d.save_plugin_pages(chain, slot, &pages)?;
            let id = d.plugin_param_oid(chain, slot, param).ok_or("the plug-in's pages don't have that parameter")?;
            if !d.reaches(ch, id) {
                return Err("that level's macros can't control this parameter".into());
            }
            d.set_macro_param(ch, page, m, Some(id), "")
        });
        if let Err(e) = r {
            self.status = format!("can't set the macro: {e}");
        }
    }

    /// Make parameter `param` of the plug-in in slot `slot` of `chain` a macro of `ch`,
    /// or stop it being one: one undo step.
    pub(crate) fn toggle_plugin_macro(&mut self, ch: Target, chain: Chain, slot: usize, param: u32) {
        let Some(doc) = self.doc.clone() else { return };
        let pages = self.plugin_pages(chain, slot);
        let r = doc.borrow_mut().transaction("Set Macro", |d| {
            d.save_plugin_pages(chain, slot, &pages)?;
            let id = d.plugin_param_oid(chain, slot, param).ok_or("the plug-in's pages don't have that parameter")?;
            let has = d.macro_pages(ch).iter().any(|p| p.macros.iter().any(|m| m.target == Some(id)));
            if has {
                d.remove_macros_to(ch, id)
            } else if d.reaches(ch, id) {
                d.add_macro_to(ch, id).map(drop)
            } else {
                Err("that level's macros can't control this parameter".into())
            }
        });
        if let Err(e) = r {
            self.status = format!("can't set the macro: {e}");
        }
    }
}
