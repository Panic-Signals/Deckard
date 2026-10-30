//! Remote control in the app (docs/superpowers/plans/2026-10-23-phase7-macros.md): MIDI
//! learn, MIDI and host automation values reaching parameters (a macro's reach its
//! target), Set Macro from the controller, and a macro slot's Learn, Cut, Copy and
//! Paste.

use crate::App;
use crate::layout::{self, Act, Edit};
use deckard_doc::{Chain, Macro, Target};
use eframe::egui;
use std::collections::HashMap;

/// The Assignment area's tab: the Pages pane (macros) or the Automation pane's MIDI or
/// Host tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssignTab {
    #[default]
    Pages,
    Midi,
    Host,
    /// Modulation routes (the matrix's): what drives each knob of a sound's chain.
    Mod,
}

/// A macro slot: its channel, page and slot (0–7).
pub type MacroSlot = (Target, usize, usize);

/// What the Assignment area asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteAct {
    /// Open the Assignment area on a tab, or close it.
    Assign(Option<AssignTab>),
    /// Learn a MIDI control for a parameter (again: stop); Unlearn.
    MidiLearn(u64),
    Unlearn(u64),
    /// Learn a MIDI control for a plug-in's parameter (chain, slot, parameter id).
    PluginMidiLearn(Chain, usize, u32),
    /// Give a parameter a host automation ID, or take it away.
    Host(u64),
    /// Focus a macro slot; rename it (`None`: start, with the text field); its Learn;
    /// cut, copy, paste.
    Focus(MacroSlot),
    Rename(MacroSlot, Option<String>),
    Learn(MacroSlot),
    Cut(MacroSlot),
    Copy(MacroSlot),
    Paste(MacroSlot),
    /// Stop learning (Esc), and renaming.
    Cancel,
}

#[derive(Default)]
pub struct Remote {
    /// The Assignment area is open, on this tab.
    pub assign: Option<AssignTab>,
    /// A parameter waiting for a MIDI control (MIDI Learn).
    pub midi_learn: Option<u64>,
    /// A plug-in's parameter (chain, slot, parameter id) waiting for a MIDI control: it
    /// may have no object yet, made as the control is learned.
    pub plugin_learn: Option<(Chain, usize, u32)>,
    /// A macro waiting for a parameter (its menu's Learn): the next parameter changed
    /// fills it, and the focus moves to the next slot.
    pub macro_learn: Option<MacroSlot>,
    /// The focused macro (⌘R, ⌘X, ⌘C, ⌘V act on it) and the copied one.
    pub macro_focus: Option<MacroSlot>,
    pub macro_clip: Option<Macro>,
    /// A macro being renamed in its field, and the text so far.
    pub renaming: Option<(MacroSlot, String)>,
    /// The knob of every parameter id, for the document revision it was found for.
    edits: Option<(u64, HashMap<u64, Edit>)>,
    /// Remote values are being applied (macro Learn leaves them alone).
    applying: bool,
}

/// The parameter an edit sets: (target, field) as a macro or MIDI learn names it.
pub(crate) fn act_param(a: &Act) -> Option<(Target, &'static str)> {
    match *a {
        Act::SetLevel(t, _) => Some((t, "level")),
        Act::SetPan(t, _) => Some((t, "pan")),
        Act::SetSwing(t, _) => Some((t, "swing_amount")),
        Act::SetParam(t, f, _) => Some((t, f)),
        _ => None,
    }
}

/// The knob of plug-in parameter object `param`.
fn plugin_edit(d: &deckard_doc::Document, param: u64) -> Option<Edit> {
    let (c, slot, id) = d.plugin_param_at(param)?;
    Some(Edit::PluginIn(c, slot, id, 0.0))
}

impl App {
    /// Carry out what the Assignment area asked for.
    pub(crate) fn remote_act(&mut self, a: RemoteAct) {
        match a {
            RemoteAct::Assign(tab) => self.remote.assign = tab,
            RemoteAct::MidiLearn(p) => self.toggle_midi_learn(p),
            RemoteAct::PluginMidiLearn(c, slot, id) => self.toggle_plugin_midi_learn(c, slot, id),
            RemoteAct::Unlearn(p) => self.unlearn_midi(p),
            RemoteAct::Host(p) => self.toggle_host_automation(p),
            RemoteAct::Focus(slot) => {
                self.remote.macro_focus = Some(slot);
                self.key_focus = layout::KeyFocus::Macros;
            }
            RemoteAct::Rename(slot, None) => {
                let name =
                    self.doc.as_ref().and_then(|d| d.borrow().macro_pages(slot.0).get(slot.1)?.macros.get(slot.2).map(|m| m.name.clone()));
                self.remote.macro_focus = Some(slot);
                self.key_focus = layout::KeyFocus::Macros;
                self.remote.renaming = Some((slot, name.unwrap_or_default()));
                self.remote.assign = Some(AssignTab::Pages);
            }
            RemoteAct::Rename((ch, page, m), Some(name)) => {
                self.remote.renaming = None;
                if let Some(doc) = &self.doc
                    && let Err(e) = doc.borrow_mut().rename_macro(ch, page, m, &name)
                {
                    self.status = format!("can't rename the macro: {e}");
                }
            }
            RemoteAct::Learn(slot) => self.toggle_macro_learn(slot),
            RemoteAct::Cut(slot) => self.cut_macro(slot),
            RemoteAct::Copy(slot) => self.copy_macro(slot),
            RemoteAct::Paste(slot) => self.paste_macro(slot),
            RemoteAct::Cancel => {
                self.remote.midi_learn = None;
                self.remote.plugin_learn = None;
                self.remote.macro_learn = None;
                self.remote.renaming = None;
            }
        }
    }

    /// The Assignment area's keys: ⌘R, ⌘X, ⌘C, ⌘V on the focused macro (Pages tab, Macro
    /// properties shown, a macro clicked last); Esc stops learning. Returns whether it took the keys.
    pub(crate) fn remote_keys(&mut self, ctx: &egui::Context) -> bool {
        let learning = self.remote.midi_learn.is_some() || self.remote.plugin_learn.is_some() || self.remote.macro_learn.is_some();
        if learning && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.remote_act(RemoteAct::Cancel);
            return true;
        }
        let on_macros = self.channel
            && self.prop_set == 3
            && self.remote.assign == Some(AssignTab::Pages)
            && self.key_focus == layout::KeyFocus::Macros;
        let Some(slot) = self.remote.macro_focus.filter(|_| on_macros && self.remote.renaming.is_none()) else { return false };
        let cmd = |k: egui::Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, k));
        let ev = |f: fn(&egui::Event) -> bool| ctx.input(|i| i.events.iter().any(f));
        let act = if cmd(egui::Key::R) {
            RemoteAct::Rename(slot, None)
        } else if cmd(egui::Key::X) || ev(|e| matches!(e, egui::Event::Cut)) {
            RemoteAct::Cut(slot)
        } else if cmd(egui::Key::C) || ev(|e| matches!(e, egui::Event::Copy)) {
            RemoteAct::Copy(slot)
        } else if cmd(egui::Key::V) || ev(|e| matches!(e, egui::Event::Paste(_))) {
            RemoteAct::Paste(slot)
        } else {
            return false;
        };
        self.remote_act(act);
        true
    }

    /// The knob of parameter `param`: its own, or for a macro its target's.
    fn remote_edit(&mut self, param: u64) -> Option<Edit> {
        let doc = self.doc.clone()?;
        let rev = doc.borrow().revision();
        if self.remote.edits.as_ref().is_none_or(|(r, _)| *r != rev) {
            let d = doc.borrow();
            let mut map: HashMap<u64, Edit> = HashMap::new();
            for t in layout::every_target(self, &d) {
                if let (Some(id), Some(edit)) = (d.param_oid(t.t, t.field), t.knob.3) {
                    map.entry(id).or_insert(edit);
                }
            }
            // Macros: their targets' knobs.
            let p = d.project();
            let mut channels = vec![Target::Song];
            for (g, grp) in p.groups.iter().enumerate() {
                channels.push(Target::Group(g));
                channels.extend((0..grp.sounds.len()).map(|s| Target::Sound(g, s)));
            }
            for ch in channels {
                for (pi, page) in d.macro_pages(ch).iter().enumerate() {
                    for (m, x) in page.macros.iter().enumerate() {
                        let edit = x.target.and_then(|t| map.get(&t).copied().or_else(|| plugin_edit(&d, t)));
                        if let (Some(id), Some(edit)) = (d.macro_oid(ch, pi, m), edit) {
                            map.insert(id, edit);
                        }
                    }
                }
            }
            drop(d);
            self.remote.edits = Some((rev, map));
        }
        let edit = self.remote.edits.as_ref()?.1.get(&param).copied();
        edit.or_else(|| plugin_edit(&doc.borrow(), param))
    }

    /// Set parameter `param` (a macro: its target) to knob position `v` (0..1), as its
    /// knob does. Returns whether it is one the app knows.
    pub fn remote_set(&mut self, param: u64, v: f64) -> bool {
        let Some(edit) = self.remote_edit(param) else { return false };
        self.remote.applying = true;
        layout::apply_acts(self, vec![edit.to_act(v.clamp(0.0, 1.0) as f32)]);
        self.remote.applying = false;
        // Only a value changed: what each id's knob is still holds.
        if let (Some(doc), Some((rev, _))) = (&self.doc, &mut self.remote.edits) {
            *rev = doc.borrow().revision();
        }
        true
    }

    /// A MIDI control moved (`key`: its remote key, `v`: 0..1): it is learned by the
    /// parameter waiting for one, or sets the parameter mapped to it.
    pub fn midi_remote(&mut self, key: u64, v: f64) {
        let Some(doc) = self.doc.clone() else { return };
        if let Some(param) = self.remote.midi_learn.take() {
            match doc.borrow_mut().learn_midi(param, key) {
                Ok(()) => self.status = format!("Learned {}", deckard_doc::remote::midi_key_name(key)),
                Err(e) => self.status = format!("can't learn: {e}"),
            }
            return;
        }
        if let Some((c, slot, id)) = self.remote.plugin_learn.take() {
            // (The plug-in's pages saved first, so the parameter has an object.)
            let pages = self.plugin_pages(c, slot);
            let r = doc.borrow_mut().transaction("MIDI Learn", |d| {
                d.save_plugin_pages(c, slot, &pages)?;
                let param = d.plugin_param_oid(c, slot, id).ok_or("the plug-in's pages don't have that parameter")?;
                d.learn_midi(param, key)
            });
            match r {
                Ok(()) => self.status = format!("Learned {}", deckard_doc::remote::midi_key_name(key)),
                Err(e) => self.status = format!("can't learn: {e}"),
            }
            return;
        }
        let param = doc.borrow().midi_param_for(key);
        if let Some(param) = param {
            self.remote_set(param, v);
        }
    }

    /// Tell the MIDI inputs what remote control takes (learning, the mapped keys) and
    /// act on what came.
    pub(crate) fn remote_midi_sync(&mut self) {
        let mapped = self.doc.as_ref().map(|d| d.borrow().midi_mappings().into_iter().map(|m| m.0).collect()).unwrap_or_default();
        self.midi.set_remote(self.remote.midi_learn.is_some() || self.remote.plugin_learn.is_some(), mapped);
        for (key, v) in self.midi.take_remote() {
            self.midi_remote(key, v);
        }
    }

    /// The host moved automation ID `id` to `v` (0..1). Maschine as a plug-in gets these
    /// from its host; DECKARD has no plug-in build yet, so only tests call it.
    pub fn host_automate(&mut self, id: usize, v: f64) {
        let param = self.doc.as_ref().and_then(|d| d.borrow().host_slots().get(id).copied().flatten());
        if let Some(param) = param {
            self.remote_set(param, v);
        }
    }

    /// Learn (or stop learning) a MIDI control for parameter `param`.
    pub fn toggle_midi_learn(&mut self, param: u64) {
        self.remote.midi_learn = if self.remote.midi_learn == Some(param) { None } else { Some(param) };
        self.remote.plugin_learn = None;
    }

    /// Learn (or stop learning) a MIDI control for parameter `id` of the plug-in in slot
    /// `slot` of `chain`.
    pub fn toggle_plugin_midi_learn(&mut self, chain: Chain, slot: usize, id: u32) {
        let p = Some((chain, slot, id));
        self.remote.plugin_learn = if self.remote.plugin_learn == p { None } else { p };
        self.remote.midi_learn = None;
    }

    /// Unlearn: no MIDI control sets parameter `param`.
    pub fn unlearn_midi(&mut self, param: u64) {
        if let Some(doc) = &self.doc
            && let Err(e) = doc.borrow_mut().unlearn_midi(param)
        {
            self.status = format!("can't unlearn: {e}");
        }
    }

    /// The Host tab's field: give parameter `param` an automation ID, or take it away.
    pub fn toggle_host_automation(&mut self, param: u64) {
        let Some(doc) = &self.doc else { return };
        let has = doc.borrow().host_slot_for(param).is_some();
        let r =
            if has { doc.borrow_mut().disable_host_automation(param) } else { doc.borrow_mut().enable_host_automation(param).map(drop) };
        if let Err(e) = r {
            self.status = format!("can't change host automation: {e}");
        }
    }

    /// Set Macro (SHIFT + MACRO, touching knob `pos`): the knob's parameter becomes a
    /// macro of the master (`level` 0), the focused group (1) or sound (2), or stops
    /// being one.
    pub fn set_macro_for_knob(&mut self, pos: usize, level: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let knobs = if self.channel {
            let Some((_, _, pg)) = layout::channel_page(self, &doc.borrow()) else { return };
            pg.slots
        } else {
            let Some(pg) = self.slot.and_then(|i| layout::slot_page(self, doc.borrow().project(), i)) else { return };
            pg.knobs
        };
        let Some(Some((_, _, _, Some(edit)))) = knobs.get(pos).cloned() else { return };
        let Some((t, f)) = layout::channel::knob_param(&edit) else { return };
        let ch = match level {
            0 => Target::Song,
            1 => Target::Group(self.group),
            _ => Target::Sound(self.group, self.sound),
        };
        let mut d = doc.borrow_mut();
        let Some(id) = d.param_oid(t, f) else { return };
        let has = d.macro_pages(ch).iter().any(|p| p.macros.iter().any(|m| m.target == Some(id)));
        let r = if has {
            d.remove_macro_for(ch, t, f)
        } else if d.reaches(ch, id) {
            d.add_macro(ch, t, f).map(drop)
        } else {
            Err("that level's macros can't control this parameter".into())
        };
        if let Err(e) = r {
            self.status = format!("can't set the macro: {e}");
        }
    }

    /// The macro menu's Learn: the next parameter changed fills macro `slot` (again: stop).
    pub fn toggle_macro_learn(&mut self, slot: MacroSlot) {
        self.remote.macro_learn = if self.remote.macro_learn == Some(slot) { None } else { Some(slot) };
        self.remote.macro_focus = Some(slot);
    }

    /// A parameter was changed (`act`): while a macro learns, it becomes that macro's
    /// target and the next slot learns (none after the eighth). Changes from the Macro
    /// page itself, and remote ones, don't count.
    pub(crate) fn macro_learn_saw(&mut self, act: &Act) {
        let Some((ch, page, m)) = self.remote.macro_learn else { return };
        let on_macros = self.channel && self.prop_set == 3;
        let Some((t, f)) = act_param(act).filter(|_| !self.remote.applying && !on_macros) else { return };
        let Some(doc) = self.doc.clone() else { return };
        let r = {
            let mut d = doc.borrow_mut();
            match d.param_oid(t, f) {
                Some(id) if d.reaches(ch, id) => d.set_macro(ch, page, m, Some((t, f)), ""),
                _ => Err("this macro can't control that parameter".into()),
            }
        };
        match r {
            Ok(()) => {
                let next = (m + 1 < 8).then_some((ch, page, m + 1));
                self.remote.macro_learn = next;
                self.remote.macro_focus = next.or(Some((ch, page, m)));
            }
            Err(e) => self.status = e,
        }
    }

    /// ⌘C on a macro: it (target and name) to the macro clipboard.
    pub fn copy_macro(&mut self, (ch, page, m): MacroSlot) {
        let Some(doc) = &self.doc else { return };
        let x = doc.borrow().macro_pages(ch).get(page).and_then(|p| p.macros.get(m).cloned());
        if x.is_some() {
            self.remote.macro_clip = x;
        }
    }

    /// ⌘X: copy, then Reset.
    pub fn cut_macro(&mut self, slot: MacroSlot) {
        self.copy_macro(slot);
        let Some(doc) = &self.doc else { return };
        let r = doc.borrow_mut().set_macro(slot.0, slot.1, slot.2, None, "");
        match r {
            Ok(()) => doc.borrow_mut().relabel_last("Cut Macro"),
            Err(e) => self.status = format!("can't cut the macro: {e}"),
        }
    }

    /// ⌘V: the copied macro into `slot`.
    pub fn paste_macro(&mut self, (ch, page, m): MacroSlot) {
        let (Some(doc), Some(clip)) = (&self.doc, self.remote.macro_clip.clone()) else { return };
        if let Err(e) = doc.borrow_mut().paste_macro(ch, page, m, &clip) {
            self.status = format!("can't paste the macro: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_doc::remote::MIDI_KEY_PITCH;

    fn app() -> App {
        let mut app = App::default();
        app.new_project();
        app
    }

    fn num(app: &App, t: Target, f: &str) -> f64 {
        app.doc.as_ref().unwrap().borrow().num(t, f).unwrap()
    }

    fn oid(app: &App, t: Target, f: &str) -> u64 {
        app.doc.as_ref().unwrap().borrow().param_oid(t, f).unwrap()
    }

    const SND: Target = Target::Sound(0, 0);

    /// A learned control moves its parameter, and a macro's moves the macro's target.
    #[test]
    fn midi_values_set_parameters_and_macros() {
        let mut app = app();
        let doc = app.doc.clone().unwrap();
        let level = oid(&app, SND, "level");
        doc.borrow_mut().learn_midi(level, 7).unwrap();
        app.midi_remote(7, 0.5);
        assert!((num(&app, SND, "level") - 0.5).abs() < 1e-6);
        doc.borrow_mut().add_macro(SND, SND, "pan").unwrap();
        let m = doc.borrow().macro_oid(SND, 0, 0).unwrap();
        doc.borrow_mut().learn_midi(m, MIDI_KEY_PITCH).unwrap();
        app.midi_remote(MIDI_KEY_PITCH, 1.0);
        assert!((num(&app, SND, "pan") - 1.0).abs() < 1e-6);
        app.midi_remote(MIDI_KEY_PITCH, 0.0);
        assert!((num(&app, SND, "pan") + 1.0).abs() < 1e-6);
        app.midi_remote(99, 1.0);
        assert!((num(&app, SND, "level") - 0.5).abs() < 1e-6, "not mapped");
    }

    /// MIDI Learn: the next control moved is learned (and sets nothing); again stops.
    #[test]
    fn midi_learn_takes_the_next_control() {
        let mut app = app();
        let level = oid(&app, SND, "level");
        let before = num(&app, SND, "level");
        app.toggle_midi_learn(level);
        app.midi_remote(74, 0.1);
        assert_eq!(app.remote.midi_learn, None);
        assert_eq!(app.doc.as_ref().unwrap().borrow().midi_key_for(level), Some(74));
        assert!((num(&app, SND, "level") - before).abs() < 1e-9);
        app.toggle_midi_learn(level);
        app.toggle_midi_learn(level);
        assert_eq!(app.remote.midi_learn, None, "stopped");
        app.unlearn_midi(level);
        assert_eq!(app.doc.as_ref().unwrap().borrow().midi_key_for(level), None);
    }

    /// Host automation of a macro (as a plug-in build will drive it) moves its target.
    #[test]
    fn host_automation_moves_macros() {
        let mut app = app();
        let doc = app.doc.clone().unwrap();
        doc.borrow_mut().add_macro(Target::Group(0), Target::Sound(0, 3), "level").unwrap();
        let m = doc.borrow().macro_oid(Target::Group(0), 0, 0).unwrap();
        app.toggle_host_automation(m);
        assert_eq!(doc.borrow().host_slot_for(m), Some(0));
        app.host_automate(0, 0.25);
        assert!((num(&app, Target::Sound(0, 3), "level") - 0.25).abs() < 1e-6);
        app.toggle_host_automation(m);
        assert_eq!(doc.borrow().host_slot_for(m), None);
    }

    /// Set Macro from the controller: a knob's parameter becomes a macro of the level
    /// asked for; again, it stops being one.
    #[test]
    fn set_macro_from_the_controller() {
        use deckard_lua_host::state::ControlRequest as R;
        let mut app = app();
        (app.channel, app.chain_tab, app.prop_set, app.prop_page) = (true, layout::ChainTab::Sound, 1, 0);
        let doc = app.doc.clone().unwrap();
        // Output ▸ Audio: DEST., CUE, LEVEL, PAN, AUDIO MUTE.
        app.control_request(R::SetMacro { pos: 2, level: 2 });
        assert_eq!(doc.borrow().macro_pages(SND)[0].macros[0].target, Some(oid(&app, SND, "level")));
        app.control_request(R::SetMacro { pos: 3, level: 0 });
        assert_eq!(doc.borrow().macro_pages(Target::Song)[0].macros[0].target, Some(oid(&app, SND, "pan")));
        app.control_request(R::SetMacro { pos: 2, level: 2 });
        assert_eq!(doc.borrow().macro_pages(SND)[0].macros[0].target, None);
        // The controller sees each knob's parameter.
        let snap = app.control_snapshot([false; 8]);
        assert_eq!(snap.knobs[2].as_ref().and_then(|k| k.param), Some(oid(&app, SND, "level")));
    }

    /// The macro menu's Learn: each parameter changed fills the slot, then the next.
    #[test]
    fn macro_learn_fills_slots_in_turn() {
        let mut app = app();
        let doc = app.doc.clone().unwrap();
        app.toggle_macro_learn((SND, 0, 6));
        layout::apply_acts(&mut app, vec![Act::SetPan(SND, 0.2)]);
        assert_eq!(app.remote.macro_learn, Some((SND, 0, 7)));
        layout::apply_acts(&mut app, vec![Act::SetLevel(Target::Sound(0, 1), 0.3)]);
        assert_eq!(app.remote.macro_learn, Some((SND, 0, 7)), "another sound's: not this sound's macro");
        layout::apply_acts(&mut app, vec![Act::SetLevel(SND, 0.3)]);
        assert_eq!(app.remote.macro_learn, None, "after the eighth");
        let macros = &doc.borrow().macro_pages(SND)[0].macros;
        assert_eq!(macros[6].target, Some(oid(&app, SND, "pan")));
        assert_eq!(macros[7].target, Some(oid(&app, SND, "level")));
    }

    /// Cut, copy and paste of macros.
    #[test]
    fn macro_cut_copy_paste() {
        let mut app = app();
        let doc = app.doc.clone().unwrap();
        doc.borrow_mut().add_macro(SND, SND, "level").unwrap();
        doc.borrow_mut().rename_macro(SND, 0, 0, "Vol").unwrap();
        app.copy_macro((SND, 0, 0));
        app.paste_macro((SND, 0, 2));
        assert_eq!(doc.borrow().macro_pages(SND)[0].macros[2].name, "Vol");
        app.cut_macro((SND, 0, 0));
        assert_eq!(doc.borrow().macro_pages(SND)[0].macros[0].target, None);
        assert_eq!(doc.borrow().undo_label(), Some("Cut Macro"));
        app.paste_macro((SND, 0, 5));
        assert_eq!(doc.borrow().macro_pages(SND)[0].macros[5].target, Some(oid(&app, SND, "level")));
    }

    /// The Assignment area: opened with its arrow; MIDI Learn and Host Enable under a
    /// knob; the Pages tab's macro menu (Learn), ⌘C / ⌘V and ⌘R on the focused macro.
    #[test]
    fn mod_tab_assigns_pressure_to_a_knob() {
        use deckard_model::mod_matrix::{ModParam, ModSource};
        use egui_kittest::kittest::Queryable;
        let Some(mut h) = crate::tests::harness(false, true) else { return };
        (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
        h.state_mut().channel = false;
        layout::apply_acts(
            h.state_mut(),
            vec![layout::Act::SelectSlot(Some(1)), layout::Act::Remote(RemoteAct::Assign(Some(AssignTab::Mod)))],
        );
        h.run_steps(4);
        let routes = |h: &egui_kittest::Harness<'static, App>| {
            h.state().doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].mod_routes.clone()
        };
        h.get_all_by_label("Assign").next().expect("an Assign field under the EQ's first knob").click();
        h.run_steps(3);
        h.get_by_label("Pressure").click();
        h.run_steps(3);
        let r = routes(&h);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].source, r[0].slot, r[0].module.as_str()), (ModSource::Pressure, 1, "EQ"));
        assert!(matches!(r[0].param, ModParam::Field(_)));
        // The menu stays open for more changes.
        h.get_by_label("Slide").click();
        h.run_steps(3);
        assert_eq!(routes(&h)[0].source, ModSource::Slide);
        assert!(h.query_by_label("Slide 0–100").is_some(), "the field shows the route");
        // The ROUTES list removes it (a first click may only close the route's menu).
        h.get_by_label("ROUTES (1)").click();
        h.run_steps(3);
        if h.query_by_label("×").is_none() {
            h.get_by_label("ROUTES (1)").click();
            h.run_steps(3);
        }
        h.get_by_label("×").click();
        h.run_steps(3);
        assert!(routes(&h).is_empty());
        assert!(h.state().doc.as_ref().unwrap().borrow_mut().undo(), "one undo step back");
        assert_eq!(routes(&h).len(), 1);
    }

    #[test]
    fn the_assignment_area() {
        use egui_kittest::kittest::Queryable;
        let Some(mut h) = crate::tests::harness(false, false) else { return };
        {
            let app = h.state_mut();
            (app.group, app.sound, app.chain_tab) = (0, 0, layout::ChainTab::Sound);
            (app.channel, app.prop_set, app.prop_page) = (true, 1, 0);
        }
        h.run_steps(3);
        h.get_by_label("Assignment area").click();
        h.run_steps(3);
        assert_eq!(h.state().remote.assign, Some(AssignTab::Midi));
        let doc = h.state().doc.clone().unwrap();
        let level = doc.borrow().param_oid(SND, "level").unwrap();
        // Output ▸ Audio: DEST., CUE, LEVEL, …: the level's Learn.
        h.get_all_by_label("Learn").nth(1).unwrap().click();
        h.run_steps(3);
        assert_eq!(h.state().remote.midi_learn, Some(level));
        h.state_mut().midi_remote(21, 0.5);
        h.run_steps(3);
        assert_eq!(doc.borrow().midi_key_for(level), Some(21));
        assert!(h.query_by_label("CC 21").is_some());
        // Host: an automation ID. The tab is hidden (no plug-in build yet), but its
        // fields still work when it's open.
        assert!(h.query_by_label("HOST").is_none());
        h.state_mut().remote.assign = Some(AssignTab::Host);
        h.run_steps(3);
        h.get_all_by_label("Enable").nth(1).unwrap().click();
        h.run_steps(3);
        assert_eq!(doc.borrow().host_slot_for(level), Some(0));
        assert!(h.query_by_label("ID 1").is_some());
        // Pages, on the Macro properties: Learn from the first slot's menu, then change the
        // pan (on its own page).
        h.state_mut().prop_set = 3;
        h.run_steps(2);
        h.get_by_label("PAGES").click();
        h.run_steps(3);
        let pos = h.get_all_by_label("Select").next().unwrap().rect().center();
        h.hover_at(pos);
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.run_steps(3);
        h.get_by_label("Learn").click();
        h.run_steps(3);
        assert_eq!(h.state().remote.macro_learn, Some((SND, 0, 0)));
        h.state_mut().prop_set = 1;
        layout::apply_acts(h.state_mut(), vec![Act::SetPan(SND, 0.25)]);
        h.state_mut().prop_set = 3;
        h.run_steps(3);
        let pan = doc.borrow().param_oid(SND, "pan").unwrap();
        assert_eq!(doc.borrow().macro_pages(SND)[0].macros[0].target, Some(pan));
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert_eq!(h.state().remote.macro_learn, None);
        // ⌘C on it, ⌘V into slot 4.
        layout::apply_acts(h.state_mut(), vec![Act::Remote(RemoteAct::Focus((SND, 0, 0)))]);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::C);
        h.run_steps(2);
        layout::apply_acts(h.state_mut(), vec![Act::Remote(RemoteAct::Focus((SND, 0, 3)))]);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::V);
        h.run_steps(3);
        assert_eq!(doc.borrow().macro_pages(SND)[0].macros[3].target, Some(pan));
        // ⌘R: its name, typed in its field.
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
        h.run_steps(3);
        assert!(h.state().remote.renaming.is_some());
        h.event(egui::Event::Text("Width".into()));
        h.run_steps(1);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(doc.borrow().macro_pages(SND)[0].macros[3].name, "Width");
        if let Some(dir) = std::env::var_os("DECKARD_RENDER_DIR") {
            h.render().unwrap().save(std::path::Path::new(&dir).join("assignment-pages.png")).unwrap();
        }
    }
}
