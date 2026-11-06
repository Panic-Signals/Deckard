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

    /// Replace the project's extra-output strips (one undo step).
    pub fn set_extra_outs(&mut self, outs: Vec<ExtraOut>) -> Result<(), String> {
        let ops = self.extra_out_ops(&outs);
        if ops.is_empty() { Ok(()) } else { self.apply("Extra Outputs", ops) }
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
