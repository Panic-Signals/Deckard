//! A plug-in's extra-output strips ([`deckard_model::extra_out`]): a DECKARD-only text
//! field on the project's root object, so undo carries them, saved in the DECKARD block.

use super::{Document, Op};
use deckard_model::extra_out::{ExtraOut, FIELD, to_text};
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
