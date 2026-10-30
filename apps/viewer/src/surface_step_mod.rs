//! The Step Mod page of DECKARD's own controller layer (a step's pad held in Step
//! mode): the Control page's knobs set the held steps' modulation, a step's holding for
//! that step alone (the MK3 manual, "Recording modulation in Step mode").

use crate::layout::{self, Edit};
use crate::{App, modulation};
use deckard_doc::Chain;
use deckard_engine::modscale::ModLane;
use deckard_model::ModTarget;

impl App {
    /// The edits of the Control page's knobs (the channel's properties or the focused
    /// slot's page), where they have one.
    pub(crate) fn knob_edits(&self) -> Vec<Option<Edit>> {
        let Some(doc) = self.doc.clone() else { return Vec::new() };
        let knobs = if self.channel {
            layout::channel_page(self, &doc.borrow()).map(|p| p.2.slots)
        } else {
            self.slot.and_then(|i| layout::slot_page(self, doc.borrow().project(), i)).map(|p| p.knobs)
        };
        knobs.unwrap_or_default().into_iter().map(|k| k.and_then(|k| k.3)).collect()
    }

    /// [`deckard_surface::Action::TouchWrite`]: knob `pos` held with Touch Auto-write
    /// and Auto-write on while the song plays writes where it is as modulation.
    pub(crate) fn surface_touch_write(&mut self, pos: usize) {
        if self.playing.is_some() {
            let head = self.playhead().map(|h| (h.mode, h.position));
            self.touch_write_at(pos, head);
        }
    }

    /// [`App::surface_touch_write`] with the song at `head` (mode, tick).
    pub(crate) fn touch_write_at(&mut self, pos: usize, head: Option<(deckard_engine::Mode, f64)>) {
        if !(self.settings.touch_auto_write && self.auto_write) || head.is_none() {
            return;
        }
        let Some(doc) = self.doc.clone() else { return };
        let knob = if self.channel {
            layout::channel_page(self, &doc.borrow()).and_then(|p| p.2.slots.get(pos).cloned().flatten())
        } else {
            self.slot.and_then(|i| layout::slot_page(self, doc.borrow().project(), i)).and_then(|p| p.knobs.get(pos).cloned().flatten())
        };
        let Some((_, _, value, Some(edit))) = knob else { return };
        if modulation::mod_target(edit).is_none() {
            return;
        }
        let at = self.mod_position(edit).unwrap_or(f64::from(value));
        self.modulate_at(edit, at as f32, false, head);
    }

    fn knob_edit(&self, pos: usize) -> Option<Edit> {
        self.knob_edits().get(pos).copied().flatten()
    }

    /// `target`'s modulation points (position, deviation) in the focused pattern.
    fn step_mod_points(&self, chain: Chain, target: &ModTarget) -> Vec<(i64, f64)> {
        self.mod_points(chain, target, self.pattern)
    }

    /// `target`'s modulation points (position, deviation) in its group's pattern `pi`.
    fn mod_points(&self, chain: Chain, target: &ModTarget, pi: usize) -> Vec<(i64, f64)> {
        let Some(doc) = self.doc.as_ref() else { return Vec::new() };
        let d = doc.borrow();
        let (g, sound) = match chain {
            Chain::Sound(g, s) => (g, Some(s)),
            Chain::Group(g) => (g, None),
            Chain::Master => return Vec::new(),
        };
        let Some(pat) = d.project().groups.get(g).and_then(|grp| grp.patterns.get(pi)) else { return Vec::new() };
        let mut points: Vec<(i64, f64)> = pat
            .mods
            .iter()
            .filter(|m| m.sound == sound && m.slot == target.slot && m.field == target.field)
            .map(|m| (m.position, m.value))
            .collect();
        points.sort_by_key(|p| p.0);
        points
    }

    /// The deviation of the point starting in the step at `at` (`len` long), if any.
    fn step_deviation(points: &[(i64, f64)], at: i64, len: i64) -> Option<f64> {
        points.iter().find(|p| p.0 >= at && p.0 < at + len).map(|p| p.1)
    }

    /// The Step Mod page's text for knob `pos`: its modulation in the step shown, as a
    /// deviation ("+25%"), if the step has any.
    pub(crate) fn step_mod_text(&self, edit: Option<Edit>) -> Option<String> {
        let (at, len) = self.step_mod_at?;
        let (chain, target) = modulation::mod_target(edit?)?;
        let dev = Self::step_deviation(&self.step_mod_points(chain, &target), at, len)?;
        let pct = (dev * 100.0).round();
        (pct != 0.0).then(|| format!("{pct:+}%"))
    }

    /// With Auto-write on, knob `edit`'s modulation where the song is (stopped: the
    /// focused pattern's start), as a percentage ("+25%", "0%"); none if it can't be
    /// modulated.
    pub(crate) fn auto_mod_text(&self, edit: Option<Edit>) -> Option<String> {
        if !self.auto_write {
            return None;
        }
        let (chain, target) = modulation::mod_target(edit?)?;
        let g = match chain {
            Chain::Sound(g, _) | Chain::Group(g) => g,
            Chain::Master => return None,
        };
        let (pi, pos, (from, to)) = {
            let doc = self.doc.as_ref()?;
            let d = doc.borrow();
            let p = d.project();
            let (pi, pos) = match self.playhead() {
                Some(h) => crate::record::pattern_at(p, h.mode, h.position, g)?,
                None => (self.pattern, p.groups.get(g)?.patterns.get(self.pattern)?.start),
            };
            let pat = p.groups.get(g)?.patterns.get(pi)?;
            (pi, pos, (pat.start, pat.start + pat.length))
        };
        let points = self.mod_points(chain, &target, pi);
        let dev = if points.is_empty() {
            0.0
        } else {
            ModLane { group: g, sound: None, slot: 0, field: String::new(), from, to, points }.value_at(pos as f64)
        };
        let pct = (dev * 100.0).round();
        Some(if pct == 0.0 { "0%".into() } else { format!("{pct:+}%") })
    }

    /// [`deckard_surface::Action::StepModAt`]: the step shown, or the page gone. The
    /// edits made while it shows are one undo step.
    pub(crate) fn surface_step_mod_at(&mut self, at: Option<(i64, i64)>) {
        let Some(doc) = self.doc.clone() else { return };
        match (self.step_mod_at.is_some(), at.is_some()) {
            (false, true) => doc.borrow_mut().begin_take("Step Modulation"),
            (true, false) => doc.borrow_mut().end_take(),
            _ => {}
        }
        self.step_mod_at = at;
    }

    /// [`deckard_surface::Action::StepMod`]: knob `knob`'s modulation in each held step
    /// moved by `inc` (a fraction of its range), held over the step and back to what was
    /// there either side.
    pub(crate) fn surface_step_mod(&mut self, from: i64, length: i64, pads: u16, knob: usize, inc: f64) {
        let Some(doc) = self.doc.clone() else { return };
        let Some((chain, target)) = self.knob_edit(knob).and_then(modulation::mod_target) else { return };
        let g = match chain {
            Chain::Sound(g, _) | Chain::Group(g) => g,
            Chain::Master => return,
        };
        let Some((start, end)) =
            doc.borrow().project().groups.get(g).and_then(|grp| grp.patterns.get(self.pattern)).map(|p| (p.start, p.start + p.length))
        else {
            return;
        };
        for pad in (0..16).filter(|p| pads & 1 << p != 0) {
            let at = from + pad * length;
            if at < start || at >= end {
                continue;
            }
            let points = self.step_mod_points(chain, &target);
            let lane = ModLane { group: g, sound: None, slot: 0, field: String::new(), from: start, to: end, points: points.clone() };
            let to = (Self::step_deviation(&points, at, length).unwrap_or(0.0) + inc).clamp(-1.0, 1.0);
            let last = (at + length - 1).min(end - 1);
            let mut write = Vec::new();
            if at > start {
                write.push((at - 1, lane.value_at((at - 1) as f64)));
            }
            write.push((at, to));
            if last > at {
                write.push((last, to));
            }
            if last + 1 < end {
                write.push((last + 1, lane.value_at((last + 1) as f64)));
            }
            let span = (write[0].0, write[write.len() - 1].0);
            if let Err(e) = doc.borrow_mut().set_modulation(chain, &target, self.pattern, span, &write) {
                self.status = format!("can't write modulation: {e}");
                return;
            }
        }
    }
}
