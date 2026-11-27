//! Editing hosted plug-in parameter pages: learn parameters from the VST/AU window,
//! assign parameters from the plug-in's list, rename or reset parameters, set section
//! headings, and add, rename, delete, clear or reset parameter pages.

use crate::App;
use crate::layout::KeyFocus;
use crate::remote::AssignTab;
use deckard_doc::Chain;
use eframe::egui;
use std::time::Duration;

/// One knob slot of a hosted plug-in's parameter page: (chain, plugin_slot, page, knob 0..7).
pub type PluginKnobSlot = (Chain, usize, usize, usize);

/// Active VST/AU parameter Learn state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginParamLearn {
    pub slot: PluginKnobSlot,
    /// Parameter just learned before auto-advancing, ignored while its drag continues.
    pub ignore_param: Option<u32>,
}

/// A copied plug-in parameter slot (its plug-in id, parameter id, and custom name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginParamClip {
    pub plugin_id: String,
    pub param_id: u32,
    pub custom_name: String,
}

/// UI state for editing hosted plug-in parameter pages (lives on [`crate::remote::Remote`]).
#[derive(Default)]
pub struct PluginPageEdit {
    /// Parameter slot currently learning from the plug-in's own editor window.
    pub learn: Option<PluginParamLearn>,
    /// Last clicked/focused parameter slot (⌘R, ⌘X, ⌘C, ⌘V act on it in the Pages tab).
    pub focus: Option<PluginKnobSlot>,
    /// Parameter slot currently being renamed inline, with its initial text.
    pub renaming: Option<(PluginKnobSlot, String)>,
    /// Copied parameter slot for Cut / Copy / Paste.
    pub clip: Option<PluginParamClip>,
}

/// Actions for editing hosted plug-in parameter pages.
#[derive(Debug, Clone, PartialEq)]
pub enum PluginPageAct {
    /// Focus a parameter slot (in the Assignment area's Pages tab).
    Focus(PluginKnobSlot),
    /// Toggle VST/AU parameter Learn for `slot` (again: stop).
    Learn(PluginKnobSlot),
    /// Assign parameter `param` (`None`: reset / unassign) to `slot`.
    Assign(PluginKnobSlot, Option<u32>),
    /// Start (`None`) or commit (`Some(name)`) renaming parameter `slot`.
    RenameParam(PluginKnobSlot, Option<String>),
    /// Set the section heading starting at parameter `slot`.
    Section(PluginKnobSlot, String),
    /// Cut, copy or paste a parameter slot.
    Cut(PluginKnobSlot),
    Copy(PluginKnobSlot),
    Paste(PluginKnobSlot),
    /// Rename page `page` of plug-in `(chain, slot)`.
    RenamePage(Chain, usize, usize, String),
    /// Append a new empty page to plug-in `(chain, slot)` and switch to it.
    AddPage(Chain, usize),
    /// Move page `from` of plug-in `(chain, slot)` to index `to`.
    MovePage(Chain, usize, usize, usize),
    /// Delete page `page` of plug-in `(chain, slot)` (last page is cleared instead).
    DeletePage(Chain, usize, usize),
    /// Replace all pages of plug-in `(chain, slot)` with one empty page.
    ClearPages(Chain, usize),
    /// Restore the plug-in's default (automatic / NKS / mapping) pages.
    ResetPages(Chain, usize),
    /// Stop learning or inline renaming.
    Cancel,
}

impl App {
    /// Whether slot `slot` of `chain` is a hosted (non-native) plug-in whose parameter
    /// pages can be edited.
    pub(crate) fn is_editable_plugin(&self, chain: Chain, slot: usize) -> bool {
        let Some(doc) = &self.doc else { return false };
        let Some(pr) = doc.borrow().chain_slots(chain).and_then(|s| s.get(slot)).and_then(|s| s.plugin.as_ref()).cloned() else {
            return false;
        };
        if pr.id.starts_with(deckard_plugins::native::VENDOR) {
            return false;
        }
        self.plugin_views.get(&(chain, slot)).is_some_and(|v| v.error.is_none() && !v.pages.first().is_some_and(|p| p.own))
    }

    /// Carry out a plug-in parameter page editing action.
    pub(crate) fn plugin_page_act(&mut self, act: PluginPageAct) {
        match act {
            PluginPageAct::Focus(slot) => {
                self.remote.plugin_pages.focus = Some(slot);
                self.key_focus = KeyFocus::PluginPages;
            }
            PluginPageAct::Learn(slot) => self.toggle_plugin_param_learn(slot),
            PluginPageAct::Assign(slot, param) => self.assign_plugin_param(slot, param),
            PluginPageAct::RenameParam(slot, None) => {
                let start = self.current_plugin_knob_name(slot);
                self.remote.plugin_pages.focus = Some(slot);
                self.key_focus = KeyFocus::PluginPages;
                self.remote.plugin_pages.learn = None;
                self.remote.plugin_pages.renaming = Some((slot, start));
            }
            PluginPageAct::RenameParam(slot, Some(name)) => {
                self.remote.plugin_pages.renaming = None;
                self.rename_plugin_knob(slot, name.trim());
            }
            PluginPageAct::Section(slot, section) => self.set_plugin_knob_section(slot, section.trim()),
            PluginPageAct::Cut(slot) => self.cut_plugin_param(slot),
            PluginPageAct::Copy(slot) => self.copy_plugin_param(slot),
            PluginPageAct::Paste(slot) => self.paste_plugin_param(slot),
            PluginPageAct::RenamePage(chain, slot, page, name) => self.rename_plugin_page_ui(chain, slot, page, name.trim()),
            PluginPageAct::AddPage(chain, slot) => self.add_plugin_page_ui(chain, slot),
            PluginPageAct::MovePage(chain, slot, from, to) => self.move_plugin_page_ui(chain, slot, from, to),
            PluginPageAct::DeletePage(chain, slot, page) => self.delete_plugin_page_ui(chain, slot, page),
            PluginPageAct::ClearPages(chain, slot) => self.clear_plugin_pages_ui(chain, slot),
            PluginPageAct::ResetPages(chain, slot) => self.reset_plugin_pages_ui(chain, slot),
            PluginPageAct::Cancel => {
                self.remote.plugin_pages.learn = None;
                self.remote.plugin_pages.renaming = None;
            }
        }
    }

    /// Toggle VST/AU parameter Learn for `slot`.
    fn toggle_plugin_param_learn(&mut self, slot: PluginKnobSlot) {
        if self.remote.plugin_pages.learn.as_ref().is_some_and(|l| l.slot == slot) {
            self.remote.plugin_pages.learn = None;
            self.status.clear();
            return;
        }
        self.remote.midi_learn = None;
        self.remote.plugin_learn = None;
        self.remote.macro_learn = None;
        self.remote.plugin_pages.renaming = None;
        self.flush_plugin_touched(slot.0, slot.1);
        self.remote.plugin_pages.focus = Some(slot);
        self.remote.plugin_pages.learn = Some(PluginParamLearn { slot, ignore_param: None });
        self.status = "Move a parameter in the plug-in window to learn it (Esc to stop)".into();
    }

    /// Discard any stale editor touch recorded before Learn began.
    fn flush_plugin_touched(&self, chain: Chain, slot: usize) {
        if let Some(p) = self.samples.plugins.existing(self.plugin_slot_key(chain, slot))
            && let Ok(g) = p.try_lock()
        {
            let _ = g.take_touched();
        }
    }

    /// Poll the learning plug-in's editor for a touched parameter each frame.
    pub(crate) fn poll_plugin_param_learn(&mut self, ctx: &egui::Context) {
        let Some(learn) = self.remote.plugin_pages.learn.clone() else { return };
        let (chain, slot, page, _) = learn.slot;
        // Stop learning if the user switched away from this plug-in or page.
        if self.channel || self.plugin_focus != Some((chain, slot)) || self.plugin_page != page || !self.is_editable_plugin(chain, slot) {
            self.remote.plugin_pages.learn = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(50));
        let touched =
            self.samples.plugins.existing(self.plugin_slot_key(chain, slot)).and_then(|p| p.try_lock().ok().and_then(|g| g.take_touched()));
        if let Some(param_id) = touched {
            self.plugin_param_touched(chain, slot, param_id);
        }
    }

    /// Handle parameter `param_id` being touched in plug-in `(chain, slot)`'s editor while
    /// Learn is active. Returns whether the touch was learned.
    pub(crate) fn plugin_param_touched(&mut self, chain: Chain, slot: usize, param_id: u32) -> bool {
        let Some(learn) = self.remote.plugin_pages.learn.clone() else { return false };
        let (lc, lslot, page, knob) = learn.slot;
        if (lc, lslot) != (chain, slot) || learn.ignore_param == Some(param_id) {
            return false;
        }
        let params = self.all_plugin_params(chain, slot);
        let title = if params.is_empty() {
            format!("Parameter {param_id}")
        } else {
            let Some(info) = params.iter().find(|i| i.id == param_id) else {
                return false;
            };
            info.title.clone()
        };
        let Some(doc) = self.doc.clone() else { return false };
        let pages = self.plugin_pages(chain, slot);
        let res = doc.borrow_mut().transaction("Learn Parameter", |d| {
            if !d.has_saved_plugin_pages(chain, slot) {
                d.save_plugin_pages(chain, slot, &pages)?;
            }
            d.set_plugin_param(chain, slot, page, knob, Some(param_id), "")
        });
        match res {
            Ok(()) => {
                self.status = format!("Learned \"{title}\" on slot {}", knob + 1);
                if knob + 1 < 8 {
                    let next = (chain, slot, page, knob + 1);
                    self.remote.plugin_pages.learn = Some(PluginParamLearn { slot: next, ignore_param: Some(param_id) });
                    self.remote.plugin_pages.focus = Some(next);
                } else {
                    self.remote.plugin_pages.learn = None;
                    self.remote.plugin_pages.focus = Some(learn.slot);
                }
                true
            }
            Err(e) => {
                self.status = format!("can't learn parameter: {e}");
                false
            }
        }
    }

    /// Assign parameter `param` (`None`: reset) to knob slot `slot`.
    fn assign_plugin_param(&mut self, slot: PluginKnobSlot, param: Option<u32>) {
        let (chain, plug, page, knob) = slot;
        let Some(doc) = self.doc.clone() else { return };
        let pages = self.plugin_pages(chain, plug);
        let label = if param.is_some() { "Assign Parameter" } else { "Reset Parameter" };
        let res = doc.borrow_mut().transaction(label, |d| {
            if !d.has_saved_plugin_pages(chain, plug) {
                d.save_plugin_pages(chain, plug, &pages)?;
            }
            d.set_plugin_param(chain, plug, page, knob, param, "")
        });
        match res {
            Ok(()) => {
                if self.remote.plugin_pages.learn.as_ref().is_some_and(|l| l.slot == slot) {
                    self.remote.plugin_pages.learn = None;
                }
            }
            Err(e) => self.status = format!("can't update parameter: {e}"),
        }
    }

    /// Rename knob `slot` (empty restores the plug-in's parameter name).
    fn rename_plugin_knob(&mut self, slot: PluginKnobSlot, name: &str) {
        let (chain, plug, page, knob) = slot;
        let Some(doc) = self.doc.clone() else { return };
        let pages = self.plugin_pages(chain, plug);
        let res = doc.borrow_mut().transaction("Rename Parameter", |d| {
            if !d.has_saved_plugin_pages(chain, plug) {
                d.save_plugin_pages(chain, plug, &pages)?;
            }
            d.rename_plugin_param(chain, plug, page, knob, name)
        });
        if let Err(e) = res {
            self.status = format!("can't rename parameter: {e}");
        }
    }

    /// Set the section heading starting at knob `slot`.
    fn set_plugin_knob_section(&mut self, slot: PluginKnobSlot, section: &str) {
        let (chain, plug, page, knob) = slot;
        let Some(doc) = self.doc.clone() else { return };
        let pages = self.plugin_pages(chain, plug);
        let res = doc.borrow_mut().transaction("Set Section Name", |d| {
            if !d.has_saved_plugin_pages(chain, plug) {
                d.save_plugin_pages(chain, plug, &pages)?;
            }
            d.set_plugin_param_section(chain, plug, page, knob, section)
        });
        if let Err(e) = res {
            self.status = format!("can't set section name: {e}");
        }
    }

    /// Rename parameter page `page` of plug-in `(chain, plug)`.
    fn rename_plugin_page_ui(&mut self, chain: Chain, plug: usize, page: usize, name: &str) {
        let Some(doc) = self.doc.clone() else { return };
        let pages = self.plugin_pages(chain, plug);
        let res = doc.borrow_mut().transaction("Rename Parameter Page", |d| {
            if !d.has_saved_plugin_pages(chain, plug) {
                d.save_plugin_pages(chain, plug, &pages)?;
            }
            d.rename_plugin_page(chain, plug, page, name)
        });
        if let Err(e) = res {
            self.status = format!("can't rename page: {e}");
        }
    }

    /// Append a new empty parameter page to plug-in `(chain, plug)` and focus it.
    fn add_plugin_page_ui(&mut self, chain: Chain, plug: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let pages = self.plugin_pages(chain, plug);
        let mut new_page = 0;
        let res = doc.borrow_mut().transaction("Add Parameter Page", |d| {
            if !d.has_saved_plugin_pages(chain, plug) && !pages.is_empty() {
                d.save_plugin_pages(chain, plug, &pages)?;
            }
            new_page = d.add_plugin_page(chain, plug)?;
            Ok(())
        });
        match res {
            Ok(()) => {
                self.remote.plugin_pages.learn = None;
                self.remote.plugin_pages.renaming = None;
                if self.plugin_focus == Some((chain, plug)) {
                    self.plugin_page = new_page;
                }
                self.chain_pages.entry(chain).or_default().slot_pages.insert(plug, new_page);
            }
            Err(e) => self.status = format!("can't add page: {e}"),
        }
    }

    /// Delete parameter page `page` of plug-in `(chain, plug)`.
    fn delete_plugin_page_ui(&mut self, chain: Chain, plug: usize, page: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let pages = self.plugin_pages(chain, plug);
        let res = doc.borrow_mut().transaction("Delete Parameter Page", |d| {
            if !d.has_saved_plugin_pages(chain, plug) {
                d.save_plugin_pages(chain, plug, &pages)?;
            }
            d.remove_plugin_page(chain, plug, page)
        });
        match res {
            Ok(()) => {
                self.remote.plugin_pages.learn = None;
                self.remote.plugin_pages.renaming = None;
                let count = doc
                    .borrow()
                    .chain_slots(chain)
                    .and_then(|s| s.get(plug))
                    .and_then(|s| s.plugin.as_ref())
                    .map_or(1, |p| p.pages.len().max(1));
                if self.plugin_focus == Some((chain, plug)) {
                    if page < self.plugin_page {
                        self.plugin_page = self.plugin_page.saturating_sub(1);
                    }
                    self.plugin_page = self.plugin_page.min(count - 1);
                }
            }
            Err(e) => self.status = format!("can't delete page: {e}"),
        }
    }

    /// Clear all pages of plug-in `(chain, plug)` down to one empty page.
    fn clear_plugin_pages_ui(&mut self, chain: Chain, plug: usize) {
        let Some(doc) = self.doc.clone() else { return };
        match doc.borrow_mut().clear_plugin_pages(chain, plug) {
            Ok(()) => {
                self.remote.plugin_pages.learn = None;
                self.remote.plugin_pages.renaming = None;
                if self.plugin_focus == Some((chain, plug)) {
                    self.plugin_page = 0;
                }
            }
            Err(e) => self.status = format!("can't clear pages: {e}"),
        }
    }

    /// Reset plug-in `(chain, plug)` to its default (automatic / NKS / mapping) pages.
    fn reset_plugin_pages_ui(&mut self, chain: Chain, plug: usize) {
        let Some(doc) = self.doc.clone() else { return };
        match doc.borrow_mut().reset_plugin_pages(chain, plug) {
            Ok(()) => {
                self.remote.plugin_pages.learn = None;
                self.remote.plugin_pages.renaming = None;
                if self.plugin_focus == Some((chain, plug)) {
                    self.plugin_page = 0;
                }
            }
            Err(e) => self.status = format!("can't reset pages: {e}"),
        }
    }

    /// Current display label of knob `slot` (pre-filled when starting inline rename).
    fn current_plugin_knob_name(&self, (chain, plug, page, knob): PluginKnobSlot) -> String {
        self.plugin_views
            .get(&(chain, plug))
            .and_then(|v| v.pages.get(page))
            .and_then(|pg| pg.knobs.get(knob))
            .and_then(Option::as_ref)
            .map(|(_, label, _)| label.clone())
            .unwrap_or_default()
    }

    /// Every parameter of the plug-in in slot `slot` of `chain`, cached per plug-in id.
    pub(crate) fn all_plugin_params(&self, chain: Chain, slot: usize) -> Vec<deckard_plugins::ParamInfo> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let Some(plugin_id) = doc.borrow().chain_slots(chain).and_then(|s| s.get(slot)).and_then(|s| Some(s.plugin.as_ref()?.id.clone()))
        else {
            return Vec::new();
        };
        {
            let cache = self.plugin_params.borrow();
            if let Some((id, list)) = cache.get(&(chain, slot))
                && *id == plugin_id
            {
                return list.clone();
            }
        }
        let Some(p) = self.samples.plugins.existing(self.plugin_slot_key(chain, slot)) else { return Vec::new() };
        let Ok(g) = p.try_lock() else { return Vec::new() };
        let list = g.parameter_list(false);
        self.plugin_params.borrow_mut().insert((chain, slot), (plugin_id, list.clone()));
        list
    }

    /// Copy parameter slot `slot` (its parameter id and custom name) to the parameter clipboard.
    fn copy_plugin_param(&mut self, (chain, plug, page, knob): PluginKnobSlot) {
        let Some(doc) = &self.doc else { return };
        let Some(plugin_id) = doc.borrow().chain_slots(chain).and_then(|s| s.get(plug)).and_then(|s| Some(s.plugin.as_ref()?.id.clone()))
        else {
            return;
        };
        let Some((info, label, _)) = self
            .plugin_views
            .get(&(chain, plug))
            .and_then(|v| v.pages.get(page))
            .and_then(|pg| pg.knobs.get(knob))
            .and_then(Option::as_ref)
        else {
            return;
        };
        let custom_name = if label != &info.title { label.clone() } else { String::new() };
        self.remote.plugin_pages.clip = Some(PluginParamClip { plugin_id, param_id: info.id, custom_name });
        self.clipboard_text = Some(label.clone());
    }

    /// Cut parameter slot `slot`: copy it, then reset the slot (one undo step).
    fn cut_plugin_param(&mut self, slot: PluginKnobSlot) {
        self.copy_plugin_param(slot);
        let (chain, plug, page, knob) = slot;
        let Some(doc) = self.doc.clone() else { return };
        let pages = self.plugin_pages(chain, plug);
        let res = doc.borrow_mut().transaction("Cut Parameter", |d| {
            if !d.has_saved_plugin_pages(chain, plug) {
                d.save_plugin_pages(chain, plug, &pages)?;
            }
            d.set_plugin_param(chain, plug, page, knob, None, "")
        });
        if let Err(e) = res {
            self.status = format!("can't cut parameter: {e}");
        }
    }

    /// Paste the copied parameter into slot `slot` (only for the same plug-in type).
    fn paste_plugin_param(&mut self, (chain, plug, page, knob): PluginKnobSlot) {
        let (Some(doc), Some(clip)) = (self.doc.clone(), self.remote.plugin_pages.clip.clone()) else { return };
        let target_id = doc.borrow().chain_slots(chain).and_then(|s| s.get(plug)).and_then(|s| Some(s.plugin.as_ref()?.id.clone()));
        if target_id.as_deref() != Some(clip.plugin_id.as_str()) {
            self.status = "that copied parameter belongs to a different plug-in".into();
            return;
        }
        let pages = self.plugin_pages(chain, plug);
        let res = doc.borrow_mut().transaction("Paste Parameter", |d| {
            if !d.has_saved_plugin_pages(chain, plug) {
                d.save_plugin_pages(chain, plug, &pages)?;
            }
            d.set_plugin_param(chain, plug, page, knob, Some(clip.param_id), &clip.custom_name)
        });
        if let Err(e) = res {
            self.status = format!("can't paste parameter: {e}");
        }
    }

    /// Move parameter page `from` of plug-in `(chain, plug)` to index `to`, keeping focus
    /// on the same page.
    fn move_plugin_page_ui(&mut self, chain: Chain, plug: usize, from: usize, to: usize) {
        if from == to {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let pages = self.plugin_pages(chain, plug);
        let res = doc.borrow_mut().transaction("Move Parameter Page", |d| {
            if !d.has_saved_plugin_pages(chain, plug) {
                d.save_plugin_pages(chain, plug, &pages)?;
            }
            d.move_plugin_page(chain, plug, from, to)
        });
        match res {
            Ok(()) => {
                self.remote.plugin_pages.learn = None;
                self.remote.plugin_pages.renaming = None;
                if self.plugin_focus == Some((chain, plug)) {
                    self.plugin_page = if self.plugin_page == from {
                        to
                    } else if from < self.plugin_page && self.plugin_page <= to {
                        self.plugin_page - 1
                    } else if to <= self.plugin_page && self.plugin_page < from {
                        self.plugin_page + 1
                    } else {
                        self.plugin_page
                    };
                    self.chain_pages.entry(chain).or_default().slot_pages.insert(plug, self.plugin_page);
                }
            }
            Err(e) => self.status = format!("can't move page: {e}"),
        }
    }

    /// Keyboard shortcuts for plug-in parameter page editing: Esc stops Learn; ⌘R, ⌘X,
    /// ⌘C, ⌘V act on the focused parameter slot in the Assignment area's Pages tab.
    pub(crate) fn plugin_page_keys(&mut self, ctx: &egui::Context) -> bool {
        if self.remote.plugin_pages.learn.is_some() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.plugin_page_act(PluginPageAct::Cancel);
            self.status.clear();
            return true;
        }
        let on_pages = !self.channel
            && self.remote.assign == Some(AssignTab::Pages)
            && self.key_focus == KeyFocus::PluginPages
            && self.remote.plugin_pages.renaming.is_none();
        let Some(slot) = self.remote.plugin_pages.focus.filter(|_| on_pages) else { return false };
        let cmd = |k: egui::Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, k));
        let ev = |f: fn(&egui::Event) -> bool| ctx.input(|i| i.events.iter().any(f));
        let act = if cmd(egui::Key::R) {
            PluginPageAct::RenameParam(slot, None)
        } else if cmd(egui::Key::X) || ev(|e| matches!(e, egui::Event::Cut)) {
            PluginPageAct::Cut(slot)
        } else if cmd(egui::Key::C) || ev(|e| matches!(e, egui::Event::Copy)) {
            PluginPageAct::Copy(slot)
        } else if cmd(egui::Key::V) || ev(|e| matches!(e, egui::Event::Paste(_))) {
            PluginPageAct::Paste(slot)
        } else {
            return false;
        };
        self.plugin_page_act(act);
        true
    }
}
