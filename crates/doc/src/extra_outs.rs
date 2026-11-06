//! A plug-in's extra-output strips ([`deckard_model::extra_out`]): a DECKARD-only text
//! field on the project's root object, so undo carries them, saved in the DECKARD block.

use super::{Document, Op};
use deckard_model::extra_out::{ExtraOut, FIELD, Owner, from_text, to_text};
use mxprj::decode::Value;

impl Document {
    /// The ops making the project's strips `outs`; none when they already are.
    pub(crate) fn extra_out_ops(&self, outs: &[ExtraOut]) -> Vec<Op> {
        let want = (!outs.is_empty()).then(|| Value::Str(to_text(outs)));
        let Some(Value::Obj(o)) = self.tree.at(&[]) else { return Vec::new() };
        if o.get(FIELD) == want.as_ref() {
            return Vec::new();
        }
        vec![Op::SetField { path: Vec::new(), field: FIELD.to_string(), value: want }]
    }

    /// Replace the project's extra-output strips (one undo step of its own).
    pub fn set_extra_outs(&mut self, outs: Vec<ExtraOut>) -> Result<(), String> {
        self.set_extra_outs_as("Extra Outputs", outs)
    }

    /// Replace the project's extra-output strips as an undo step labelled `label`, of
    /// its own: never merged with the edits around it (a mute, an output picked, strips
    /// added).
    pub fn set_extra_outs_as(&mut self, label: &str, outs: Vec<ExtraOut>) -> Result<(), String> {
        let ops = self.extra_out_ops(&outs);
        if ops.is_empty() { Ok(()) } else { self.transaction(label, |d| d.apply(label, ops)) }
    }

    /// Replace the project's extra-output strips, merging with the step before when it
    /// has the same `label` and is recent (a fader drag): give each strip and control its
    /// own label so only a drag merges with itself.
    pub fn set_extra_outs_merging(&mut self, label: &str, outs: Vec<ExtraOut>) -> Result<(), String> {
        let ops = self.extra_out_ops(&outs);
        if ops.is_empty() { Ok(()) } else { self.apply(label, ops) }
    }

    /// Add strips that appeared (a plug-in's extra pair first heard), outside the undo
    /// history: they come from sound, not an edit, so no step is made and redo is kept.
    /// A strip whose (owner, pair, plug-in) has one already is left out.
    ///
    /// So that undoing or redoing an edit keeps them, every step's record of the strips
    /// gets them too (where it has none for that pair), back to the nearest step that
    /// moved strips between owners (a slot inserted, moved or removed, a group deleted):
    /// past it their owners may name other slots, so only its side nearer now gets them.
    /// Undone past that, a strip still playing appears again when next heard.
    pub fn add_heard_extra_outs(&mut self, fresh: &[ExtraOut]) -> Result<(), String> {
        let mut new: Vec<ExtraOut> = Vec::new();
        for x in fresh {
            if !self.project.extra_outs.iter().chain(&new).any(|e| same_pair(e, x)) {
                new.push(x.clone());
            }
        }
        if new.is_empty() {
            return Ok(());
        }
        let mut all = self.project.extra_outs.clone();
        all.extend(new.iter().cloned());
        let ops = self.extra_out_ops(&all);
        self.run(&ops)?;
        self.refresh();
        // The undo steps, newest first: their "after" is nearer now. The redo steps,
        // next to redo first: their "before" is.
        for (steps, after_is_near) in [(&mut self.undo, true), (&mut self.redo, false)] {
            for step in steps.iter_mut().rev() {
                if !add_to_step(step, &new, after_is_near) {
                    break;
                }
            }
        }
        Ok(())
    }

    /// The op moving the strips of chain `c`'s plug-ins to their slots' new places
    /// (`remap`: a slot's new index, `None` when it's gone, and its strips with it), as
    /// [`Document::sidechain_rekey`] does side-chains; none when nothing moves. A sound's
    /// instrument (`Owner::Sound`) stays first, so only its chain's strips move.
    pub(crate) fn extra_outs_rekey(&self, c: crate::Chain, remap: impl Fn(usize) -> Option<usize>) -> Option<Op> {
        use crate::Chain;
        let moved: Vec<ExtraOut> = self
            .project
            .extra_outs
            .iter()
            .filter_map(|x| {
                let owner = match (c, x.owner) {
                    (Chain::Sound(g, s), Owner::Chain(a, b, k)) if (a, b) == (g, s) => Owner::Chain(g, s, remap(k)?),
                    (Chain::Group(g), Owner::Group(a, k)) if a == g => Owner::Group(g, remap(k)?),
                    (Chain::Master, Owner::Master(k)) => Owner::Master(remap(k)?),
                    (_, o) => o,
                };
                Some(ExtraOut { owner, ..x.clone() })
            })
            .collect();
        if moved == self.project.extra_outs {
            return None;
        }
        self.extra_out_ops(&moved).pop()
    }

    /// The op following group `g`'s deletion in the strips: its plug-ins' strips go,
    /// later groups' move down one; a strip playing into the group (or one of its sounds)
    /// goes to its parent instead, one playing into a later group follows it. None when
    /// nothing changes.
    pub(crate) fn extra_outs_group_deleted(&self, g: usize) -> Option<Op> {
        use deckard_model::AudioDest;
        use std::cmp::Ordering;
        let shift = |x: usize| match x.cmp(&g) {
            Ordering::Less => Some(x),
            Ordering::Equal => None,
            Ordering::Greater => Some(x - 1),
        };
        let moved: Vec<ExtraOut> = self
            .project
            .extra_outs
            .iter()
            .filter_map(|x| {
                let owner = match x.owner {
                    Owner::Sound(a, s) => Owner::Sound(shift(a)?, s),
                    Owner::Chain(a, s, k) => Owner::Chain(shift(a)?, s, k),
                    Owner::Group(a, k) => Owner::Group(shift(a)?, k),
                    Owner::Master(k) => Owner::Master(k),
                };
                let dest = match x.dest {
                    AudioDest::Group(a) => shift(a).map_or(AudioDest::Parent, AudioDest::Group),
                    AudioDest::Sound(a, s) => shift(a).map_or(AudioDest::Parent, |a| AudioDest::Sound(a, s)),
                    d => d,
                };
                Some(ExtraOut { owner, dest, ..x.clone() })
            })
            .collect();
        if moved == self.project.extra_outs {
            return None;
        }
        self.extra_out_ops(&moved).pop()
    }

    /// Drop the strips whose slot no longer holds their plug-in, outside the undo history,
    /// after an undo or redo: a step made before those strips appeared (a slot inserted
    /// or moved, a plug-in replaced) doesn't carry them, so it can leave one naming the
    /// wrong slot. A strip still playing appears again, on the right slot, when heard.
    pub(crate) fn prune_extra_outs_quietly(&mut self) {
        if self.project.extra_outs.is_empty() {
            return;
        }
        let mut p = self.project.clone();
        deckard_model::extra_out::prune(&mut p);
        let ops = self.extra_out_ops(&p.extra_outs);
        if !ops.is_empty() && self.run(&ops).is_ok() {
            self.refresh();
        }
    }

    /// Drop the strips whose plug-in is gone from their slot, after an edit that removed,
    /// replaced or moved plug-ins. Folded into that edit's undo step.
    pub(crate) fn prune_extra_outs(&mut self) -> Result<(), String> {
        if self.project.extra_outs.is_empty() {
            return Ok(());
        }
        let mut p = self.project.clone();
        deckard_model::extra_out::prune(&mut p);
        let ops = self.extra_out_ops(&p.extra_outs);
        if ops.is_empty() {
            return Ok(());
        }
        let before = self.undo.pop();
        let result = self.apply("Drop Extra Outputs", ops);
        if let (Some(before), Some(pruned)) = (before, self.undo.pop()) {
            let label = before.label.clone();
            self.undo.push(super::Step::merged(&label, &[before, pruned]));
        }
        result
    }
}

/// Strips for the same pair of the same plug-in in the same slot (either half of a split
/// one).
fn same_pair(a: &ExtraOut, b: &ExtraOut) -> bool {
    a.owner == b.owner && a.bus == b.bus && a.plugin == b.plugin
}

/// The strips an op sets, if it sets them.
fn strips_of(op: &mut Op) -> Option<&mut Option<Value>> {
    match op {
        Op::SetField { path, field, value } if path.is_empty() && field == FIELD => Some(value),
        _ => None,
    }
}

fn read(v: &Option<Value>) -> Vec<ExtraOut> {
    match v {
        Some(Value::Str(t)) => from_text(t),
        _ => Vec::new(),
    }
}

/// The (owner, plug-in) pairs `v`'s strips belong to, sorted, once each: two plug-ins
/// swapping slots changes them, though the owners stay the same.
fn owners(v: &Option<Value>) -> Vec<(Owner, String)> {
    let mut o: Vec<(Owner, String)> = read(v).into_iter().map(|x| (x.owner, x.plugin)).collect();
    o.sort_by_key(|(o, p)| (format!("{o:?}"), p.clone()));
    o.dedup();
    o
}

/// Add `new` to `v`'s strips where they have none for that pair.
fn add_to(v: &mut Option<Value>, new: &[ExtraOut]) {
    let mut outs = read(v);
    for x in new {
        if !outs.iter().any(|e| same_pair(e, x)) {
            outs.push(x.clone());
        }
    }
    *v = Some(Value::Str(to_text(&outs)));
}

/// Give `step`'s records of the strips `new` (see [`Document::add_heard_extra_outs`]):
/// all of them when the step leaves every strip with its owner and plug-in, and true;
/// else only the side nearer now (after it, `after_is_near`, else before it), and false.
///
/// A take's step is its parts (step undo takes them back one at a time): they are given
/// the strips in turn from the side nearer now (the last part first for an undo step),
/// with the same stop, and the take's own ops made from them again, so undoing the whole
/// take and undoing its parts one by one stay the same.
fn add_to_step(step: &mut super::Step, new: &[ExtraOut], after_is_near: bool) -> bool {
    if !step.parts.is_empty() {
        let mut keeps = true;
        let n = step.parts.len();
        for k in 0..n {
            let i = if after_is_near { n - 1 - k } else { k };
            if !add_to_step(&mut step.parts[i], new, after_is_near) {
                keeps = false;
                break;
            }
        }
        let merged = super::Step::merged(&step.label, &step.parts);
        step.undo = merged.undo;
        step.redo = merged.redo;
        step.params = merged.params;
        return keeps;
    }
    let mut seen: Option<Vec<(Owner, String)>> = None;
    let mut keeps = true;
    for op in step.undo.iter_mut().chain(step.redo.iter_mut()) {
        if let Some(v) = strips_of(op) {
            let o = owners(v);
            keeps &= seen.as_ref().is_none_or(|s| *s == o);
            seen = Some(o);
        }
    }
    let (near, far) = if after_is_near { (&mut step.redo, &mut step.undo) } else { (&mut step.undo, &mut step.redo) };
    near.iter_mut().filter_map(strips_of).for_each(|v| add_to(v, new));
    if keeps {
        far.iter_mut().filter_map(strips_of).for_each(|v| add_to(v, new));
    }
    keeps
}

#[cfg(test)]
mod tests {
    use crate::{Document, NewModule};
    use deckard_model::AudioDest;
    use deckard_model::extra_out::{ExtraOut, Owner};

    const PLUGIN: &str = "V:AU:INSTR:1";

    fn strip(owner: Owner, bus: usize, plugin: &str) -> ExtraOut {
        ExtraOut {
            owner,
            bus,
            plugin: plugin.into(),
            name: String::new(),
            level: 1.0,
            pan: 0.0,
            muted: false,
            dest: AudioDest::Parent,
            channels: Default::default(),
            side: None,
        }
    }

    /// A new project with a plug-in first in its first sound's chain.
    fn with_plugin() -> Document {
        let mut d = Document::new_project().unwrap();
        d.add_slot(0, 0, 0, &NewModule::Plugin { id: PLUGIN.into(), name: "P".into(), chunk: Vec::new() }).unwrap();
        d
    }

    fn buses(d: &Document) -> Vec<usize> {
        d.project().extra_outs.iter().map(|x| x.bus).collect()
    }

    /// A strip appearing (its pair heard while playing) is no edit: undo reaches past it
    /// to the edit before, which it doesn't take away, and redo is kept.
    #[test]
    fn undoing_an_edit_while_a_strip_plays_reaches_past_it() {
        let o = Owner::Chain(0, 0, 0);
        let mut d = with_plugin();
        d.add_heard_extra_outs(&[strip(o, 1, PLUGIN)]).unwrap();
        assert_eq!(d.undo_label(), Some("Load Plug-in"), "no step of its own");
        // An edit of the strip, then another strip heard.
        let mut outs = d.project().extra_outs.clone();
        outs[0].level = 0.5;
        d.set_extra_outs_as("Extra Output 1 Level", outs).unwrap();
        d.add_heard_extra_outs(&[strip(o, 2, PLUGIN)]).unwrap();
        assert_eq!(buses(&d), [1, 2]);
        // Undo takes back the level, keeping both strips; then the plug-in itself.
        assert!(d.undo());
        assert_eq!(buses(&d), [1, 2], "the strips that appeared stay");
        assert_eq!(d.project().extra_outs[0].level, 1.0);
        // Still playing: heard again, nothing changes and redo is kept.
        d.add_heard_extra_outs(&[strip(o, 1, PLUGIN), strip(o, 2, PLUGIN)]).unwrap();
        assert_eq!(d.redo_label(), Some("Extra Output 1 Level"));
        assert!(d.undo());
        assert!(d.project().groups[0].sounds[0].slots.is_empty(), "past the strips, to the plug-in");
        assert!(d.project().extra_outs.is_empty(), "no strip for a plug-in that isn't there");
        // Redo all the way: the plug-in (its strips appear again when heard), then the
        // level, which brings the strips with it.
        assert!(d.redo());
        assert!(d.project().extra_outs.is_empty());
        assert!(d.redo());
        assert_eq!(buses(&d), [1, 2]);
        assert_eq!(d.project().extra_outs[0].level, 0.5);
        // A strip removed by an edit (its plug-in gone) stays gone when an earlier edit
        // is undone or redone.
        d.remove_slot(0, 0, 0).unwrap();
        assert!(d.project().extra_outs.is_empty());
        assert!(d.undo(), "the removal");
        assert_eq!(buses(&d), [1, 2], "undoing the removal brings them back");
        assert!(d.redo());
        assert!(d.project().extra_outs.is_empty());
    }

    /// Heard while an edit waits to be redone: redoing it keeps the strip, and so does
    /// undoing it again.
    #[test]
    fn a_strip_heard_before_a_redo_stays() {
        let o = Owner::Chain(0, 0, 0);
        let mut d = with_plugin();
        d.add_heard_extra_outs(&[strip(o, 1, PLUGIN)]).unwrap();
        let mut outs = d.project().extra_outs.clone();
        outs[0].muted = true;
        d.set_extra_outs_as("Mute Extra Output", outs).unwrap();
        assert!(d.undo());
        d.add_heard_extra_outs(&[strip(o, 2, PLUGIN)]).unwrap();
        assert!(d.redo());
        assert_eq!(buses(&d), [1, 2]);
        assert!(d.project().extra_outs[0].muted);
        assert!(d.undo());
        assert_eq!(buses(&d), [1, 2]);
        assert!(!d.project().extra_outs[0].muted);
    }

    const FX: &str = "V:AU:FX:7";

    fn fx() -> NewModule {
        NewModule::Plugin { id: FX.into(), name: "Fx".into(), chunk: Vec::new() }
    }

    /// An effect put in ahead of a plug-in, or the plug-in moved: its strip follows it,
    /// with its settings, in the same undo step.
    #[test]
    fn a_strip_follows_its_plugin_when_slots_move() {
        use crate::Chain;
        let mut d = Document::new_project().unwrap();
        d.add_slot_in(Chain::Group(0), 0, &fx()).unwrap();
        let mut x = strip(Owner::Group(0, 0), 1, FX);
        x.level = 0.25;
        d.set_extra_outs(vec![x.clone()]).unwrap();
        d.add_slot_in(Chain::Group(0), 0, &NewModule::Effect("Filter")).unwrap();
        assert_eq!(d.project().extra_outs, [ExtraOut { owner: Owner::Group(0, 1), ..x.clone() }], "inserted ahead");
        assert!(d.undo());
        assert_eq!(d.project().extra_outs, [x.clone()], "one step");
        assert!(d.redo());
        d.move_slot_in(Chain::Group(0), 1, 0).unwrap();
        assert_eq!(d.project().extra_outs, [x.clone()], "moved back to the front");
        // A sound's chain: an effect put in between its instrument and the plug-in.
        let mut d = with_plugin();
        d.add_slot(0, 0, 1, &fx()).unwrap();
        let y = strip(Owner::Chain(0, 0, 1), 2, FX);
        d.set_extra_outs(vec![y.clone()]).unwrap();
        d.add_slot(0, 0, 1, &NewModule::Effect("Filter")).unwrap();
        assert_eq!(d.project().extra_outs, [ExtraOut { owner: Owner::Chain(0, 0, 2), ..y.clone() }]);
        d.remove_slot(0, 0, 1).unwrap();
        assert_eq!(d.project().extra_outs, [y], "the slot ahead removed");
    }

    /// Deleting a group: its plug-ins' strips go; later groups' strips, and strips
    /// playing into later groups, follow them; a strip playing into it goes to its parent.
    #[test]
    fn deleting_a_group_moves_the_strips_after_it() {
        use crate::Chain;
        let mut d = Document::new_project().unwrap();
        d.add_group().unwrap();
        d.add_group().unwrap();
        for g in [0, 2] {
            d.add_slot_in(Chain::Group(g), 0, &fx()).unwrap();
        }
        let mine = strip(Owner::Group(0, 0), 1, FX);
        let into_later = ExtraOut { dest: AudioDest::Group(1), ..strip(Owner::Group(2, 0), 1, FX) };
        let into_gone = ExtraOut { dest: AudioDest::Sound(0, 3), ..strip(Owner::Group(2, 0), 2, FX) };
        let before = vec![mine, into_later.clone(), into_gone.clone()];
        d.set_extra_outs(before.clone()).unwrap();
        d.delete_group(0).unwrap();
        assert_eq!(
            d.project().extra_outs,
            [
                ExtraOut { owner: Owner::Group(1, 0), dest: AudioDest::Group(0), ..into_later },
                ExtraOut { owner: Owner::Group(1, 0), dest: AudioDest::Parent, ..into_gone },
            ]
        );
        assert!(d.undo());
        assert_eq!(d.project().extra_outs, before, "one step");
    }

    /// The strips a step's ops set (the last such op's), before (`undo`) or after it.
    fn snapshot(ops: &[crate::Op]) -> Vec<ExtraOut> {
        use mxprj::decode::Value;
        let mut v = Vec::new();
        for op in ops {
            if let crate::Op::SetField { path, field, value } = op
                && path.is_empty()
                && field == deckard_model::extra_out::FIELD
            {
                v = match value {
                    Some(Value::Str(t)) => deckard_model::extra_out::from_text(t),
                    _ => Vec::new(),
                };
            }
        }
        v
    }

    fn owners(d: &Document) -> Vec<(Owner, usize, String)> {
        d.project().extra_outs.iter().map(|x| (x.owner, x.bus, x.plugin.clone())).collect()
    }

    /// A strip that appeared after an effect was put in ahead of its plug-in: undoing
    /// that (a step from before the strip, which doesn't carry it) leaves no strip on the
    /// emptied slot; redoing it leaves none on the effect now in the plug-in's old slot;
    /// the plug-in's strip comes back when heard.
    #[test]
    fn undoing_a_slot_edit_leaves_no_strip_on_the_wrong_slot() {
        use crate::Chain;
        let mut d = Document::new_project().unwrap();
        d.add_slot_in(Chain::Group(0), 0, &fx()).unwrap();
        d.add_slot_in(Chain::Group(0), 0, &NewModule::Effect("Filter")).unwrap();
        d.add_heard_extra_outs(&[strip(Owner::Group(0, 1), 1, FX)]).unwrap();
        assert!(d.undo());
        assert!(d.project().extra_outs.is_empty(), "slot 1 is empty now: {:?}", owners(&d));
        // Heard again where the plug-in is now.
        d.add_heard_extra_outs(&[strip(Owner::Group(0, 0), 1, FX)]).unwrap();
        assert_eq!(owners(&d), [(Owner::Group(0, 0), 1, FX.to_string())]);
        assert!(d.redo());
        assert!(d.project().extra_outs.is_empty(), "slot 0 holds the Filter: {:?}", owners(&d));
        d.add_heard_extra_outs(&[strip(Owner::Group(0, 1), 1, FX)]).unwrap();
        assert_eq!(owners(&d), [(Owner::Group(0, 1), 1, FX.to_string())], "back on detection");
    }

    const FX2: &str = "V:AU:FX:8";

    /// Two plug-ins with strips swapping slots keeps the same owners but not the same
    /// plug-ins on them: a strip that appears after the swap isn't added to the record
    /// from before it, where its slot held the other plug-in.
    #[test]
    fn a_strip_heard_after_a_swap_stays_out_of_the_record_before_it() {
        use crate::Chain;
        let mut d = Document::new_project().unwrap();
        d.add_slot_in(Chain::Group(0), 0, &fx()).unwrap();
        d.add_slot_in(Chain::Group(0), 1, &NewModule::Plugin { id: FX2.into(), name: "Fx2".into(), chunk: Vec::new() }).unwrap();
        d.set_extra_outs(vec![strip(Owner::Group(0, 0), 1, FX), strip(Owner::Group(0, 1), 1, FX2)]).unwrap();
        d.move_slot_in(Chain::Group(0), 0, 1).unwrap();
        assert_eq!(owners(&d), [(Owner::Group(0, 1), 1, FX.to_string()), (Owner::Group(0, 0), 1, FX2.to_string())]);
        d.add_heard_extra_outs(&[strip(Owner::Group(0, 1), 2, FX)]).unwrap();
        let before = snapshot(&d.undo.last().unwrap().undo);
        assert!(before.iter().all(|x| x.bus != 2), "{before:?}");
        assert!(d.undo());
        assert_eq!(owners(&d), [(Owner::Group(0, 0), 1, FX.to_string()), (Owner::Group(0, 1), 1, FX2.to_string())]);
        assert!(d.redo());
        assert_eq!(
            owners(&d),
            [(Owner::Group(0, 1), 1, FX.to_string()), (Owner::Group(0, 0), 1, FX2.to_string()), (Owner::Group(0, 1), 2, FX.to_string())]
        );
    }

    /// A take's parts get a strip that appeared from the last part back, stopping at one
    /// that moved strips (an effect put in ahead); the take's own ops stay its parts', so
    /// undoing it whole or a part at a time is the same.
    #[test]
    fn a_take_gets_a_new_strip_part_by_part() {
        use crate::Chain;
        let mut d = Document::new_project().unwrap();
        d.add_slot_in(Chain::Group(0), 0, &fx()).unwrap();
        let x = strip(Owner::Group(0, 0), 1, FX);
        d.set_extra_outs(vec![x.clone()]).unwrap();
        d.begin_take("Record");
        d.set_extra_outs_as("Extra Output 1 Level", vec![ExtraOut { level: 0.5, ..x.clone() }]).unwrap();
        d.add_slot_in(Chain::Group(0), 0, &NewModule::Effect("Filter")).unwrap();
        d.end_take();
        d.add_heard_extra_outs(&[strip(Owner::Group(0, 1), 2, FX)]).unwrap();
        {
            let take = d.undo.last().unwrap();
            assert_eq!(take.parts.len(), 2);
            let (level, insert) = (&take.parts[0], &take.parts[1]);
            assert!(snapshot(&insert.redo).iter().any(|x| x.bus == 2), "the part nearest now has it");
            assert!(snapshot(&insert.undo).iter().all(|x| x.bus != 2), "not from before the insert");
            assert!(snapshot(&level.undo).iter().chain(&snapshot(&level.redo)).all(|x| x.bus != 2), "nor the part before");
            let undo: Vec<crate::Op> = take.parts.iter().rev().flat_map(|p| p.undo.iter().cloned()).collect();
            let redo: Vec<crate::Op> = take.parts.iter().flat_map(|p| p.redo.iter().cloned()).collect();
            assert_eq!((&take.undo, &take.redo), (&undo, &redo), "the take is its parts");
        }
        assert!(d.undo_step());
        assert_eq!(owners(&d), [(Owner::Group(0, 0), 1, FX.to_string())]);
        assert_eq!(d.project().extra_outs[0].level, 0.5);
        assert!(d.undo_step());
        assert_eq!(d.project().extra_outs, std::slice::from_ref(&x));
        assert!(d.redo() && d.redo());
        assert_eq!(owners(&d), [(Owner::Group(0, 1), 1, FX.to_string()), (Owner::Group(0, 1), 2, FX.to_string())]);
    }
}
