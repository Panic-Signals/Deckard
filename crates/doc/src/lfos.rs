//! The project's LFOs ([`deckard_model::lfo`]): a DECKARD-only text field on the project's
//! root object, so undo carries them, saved in the DECKARD block.

use super::{Document, Op};
use deckard_model::lfo::{FIELD, Lfo, to_text};
use mxprj::decode::Value;

impl Document {
    /// Give the project this LFO, replacing the one on its pad or adding it. One undo
    /// step; quick changes to the same LFO (a knob dragged) merge into one (labelled by
    /// its pad, so another LFO's don't), but a mute or unmute is always a step of its own.
    pub fn set_lfo(&mut self, lfo: Lfo) -> Result<(), String> {
        let mut lfos = self.project.lfos.clone();
        let pad = lfo.pad;
        let (label, merges) = match lfos.iter_mut().find(|l| l.pad == pad) {
            Some(l) => {
                let label = match (l.muted, lfo.muted) {
                    (false, true) => ("Mute LFO", false),
                    (true, false) => ("Unmute LFO", false),
                    _ => ("LFO", true),
                };
                *l = lfo;
                label
            }
            None => {
                lfos.push(lfo);
                ("Add LFO", false)
            }
        };
        let ops = self.lfo_ops(&lfos);
        if ops.is_empty() {
            return Ok(());
        }
        self.apply(&format!("{label} {pad}"), ops)?;
        if !merges {
            // (Nothing merges into it: muting another LFO next is another step.)
            if let Some(last) = self.undo.last_mut() {
                last.at = std::time::Instant::now() - super::MERGE_WINDOW;
            }
        }
        Ok(())
    }

    /// Take away the LFO on `pad` (its routes stay: they just find nothing to read).
    pub fn remove_lfo(&mut self, pad: u8) -> Result<(), String> {
        let lfos: Vec<Lfo> = self.project.lfos.iter().filter(|l| l.pad != pad).cloned().collect();
        let ops = self.lfo_ops(&lfos);
        if ops.is_empty() { Ok(()) } else { self.apply(&format!("Remove LFO {pad}"), ops) }
    }

    /// The ops that give the project these LFOs (none if it has them; no field if none).
    pub(crate) fn lfo_ops(&self, lfos: &[Lfo]) -> Vec<Op> {
        let want = (!lfos.is_empty()).then(|| Value::Str(to_text(lfos)));
        let Some(Value::Obj(o)) = self.tree.at(&[]) else { return Vec::new() };
        if o.get(FIELD) == want.as_ref() {
            return Vec::new();
        }
        vec![Op::SetField { path: Vec::new(), field: FIELD.to_string(), value: want }]
    }

    /// Move the LFOs' triggers by `remap` (see [`Self::lfo_trigger_fixup`]), inside the
    /// edit that moved the sounds.
    pub(crate) fn fix_lfo_triggers(&mut self, remap: impl Fn(usize, usize) -> Option<(usize, usize)>) -> Result<(), String> {
        let ops = self.lfo_trigger_fixup(remap);
        if ops.is_empty() { Ok(()) } else { self.apply("LFO Trigger", ops) }
    }

    /// The ops moving the LFOs' triggers when sounds move (`remap`: a sound's new place,
    /// `None` when it's gone and its LFOs' triggers go Off); none when nothing moves.
    pub(crate) fn lfo_trigger_fixup(&self, remap: impl Fn(usize, usize) -> Option<(usize, usize)>) -> Vec<Op> {
        let lfos: Vec<Lfo> =
            self.project.lfos.iter().map(|l| Lfo { trigger: l.trigger.and_then(|(g, s)| remap(g, s)), ..l.clone() }).collect();
        self.lfo_ops(&lfos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::mod_matrix::{ModParam, ModRoute, ModSource};

    fn lfo(pad: u8, phase: f32) -> Lfo {
        Lfo { phase, ..Lfo::new(pad) }
    }

    fn triggered(d: &Document, pad: u8) -> Option<(usize, usize)> {
        d.project().lfos.iter().find(|l| l.pad == pad).unwrap().trigger
    }

    #[test]
    fn lfos_are_set_undone_and_saved() {
        let mut d = Document::new_project().unwrap();
        assert!(d.project().lfos.is_empty());
        d.set_lfo(Lfo::new(3)).unwrap();
        assert_eq!(d.project().lfos, vec![Lfo::new(3)]);
        assert!(d.undo());
        assert!(d.project().lfos.is_empty());
        assert!(d.redo());
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(back.project().lfos, vec![Lfo::new(3)], "kept in the DECKARD block");
    }

    #[test]
    fn a_knob_drag_is_one_undo_step() {
        let mut d = Document::new_project().unwrap();
        d.set_lfo(lfo(2, 0.0)).unwrap();
        d.set_lfo(lfo(2, 10.0)).unwrap();
        d.set_lfo(lfo(2, 20.0)).unwrap();
        d.set_lfo(lfo(2, 30.0)).unwrap();
        assert_eq!(d.project().lfos[0].phase, 30.0);
        assert!(d.undo());
        assert_eq!(d.project().lfos[0].phase, 0.0);
    }

    /// Quick edits to two LFOs, or removing two: a step each.
    #[test]
    fn edits_to_different_lfos_are_steps_of_their_own() {
        let mut d = Document::new_project().unwrap();
        d.set_lfo(lfo(1, 0.0)).unwrap();
        d.set_lfo(lfo(2, 0.0)).unwrap();
        d.set_lfo(lfo(1, 10.0)).unwrap();
        d.set_lfo(lfo(2, 20.0)).unwrap();
        assert!(d.undo());
        assert_eq!(d.project().lfos[1].phase, 0.0);
        assert_eq!(d.project().lfos[0].phase, 10.0, "LFO 1's edit is another step");
        assert!(d.undo());
        assert_eq!(d.project().lfos[0].phase, 0.0);
        assert_eq!(d.project().lfos.len(), 2);
        d.remove_lfo(1).unwrap();
        d.remove_lfo(2).unwrap();
        assert!(d.undo());
        assert_eq!(d.project().lfos.len(), 1, "only LFO 2 back");
    }

    #[test]
    fn a_mute_is_its_own_undo_step_and_saved() {
        let mut d = Document::new_project().unwrap();
        d.set_lfo(lfo(1, 0.0)).unwrap();
        d.set_lfo(lfo(2, 0.0)).unwrap();
        d.set_lfo(lfo(1, 10.0)).unwrap();
        d.set_lfo(Lfo { muted: true, ..lfo(1, 10.0) }).unwrap();
        d.set_lfo(Lfo { muted: true, ..lfo(2, 0.0) }).unwrap();
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert!(back.project().lfos.iter().all(|l| l.muted), "kept in the DECKARD block");
        assert!(d.undo());
        assert!(!d.project().lfos[1].muted, "the second mute alone");
        assert!(d.project().lfos[0].muted);
        assert!(d.undo());
        assert!(!d.project().lfos[0].muted);
        assert_eq!(d.project().lfos[0].phase, 10.0, "not merged into the knob turn");
    }

    #[test]
    fn removing_an_lfo_keeps_its_routes() {
        let mut d = Document::new_project().unwrap();
        d.set_lfo(Lfo::new(3)).unwrap();
        let r = ModRoute::new(ModSource::Lfo(3), 0, "Sampler", ModParam::Field("cutoff".into()));
        d.set_mod_routes(0, 0, vec![r.clone()]).unwrap();
        d.remove_lfo(3).unwrap();
        assert!(d.project().lfos.is_empty());
        assert_eq!(d.project().groups[0].sounds[0].mod_routes, vec![r]);
    }

    fn with_triggers() -> Document {
        let mut d = Document::new_project().unwrap();
        d.add_group().unwrap();
        d.add_group().unwrap();
        d.set_lfo(Lfo { trigger: Some((2, 0)), ..Lfo::new(1) }).unwrap();
        d.set_lfo(Lfo { trigger: Some((1, 4)), ..Lfo::new(2) }).unwrap();
        d
    }

    #[test]
    fn deleting_a_group_moves_triggers() {
        let mut d = with_triggers();
        assert_eq!(d.project().groups.len(), 3);
        d.delete_group(1).unwrap();
        assert_eq!(triggered(&d, 1), Some((1, 0)));
        assert_eq!(triggered(&d, 2), None);
        assert!(d.undo());
        assert_eq!(triggered(&d, 1), Some((2, 0)));
        assert_eq!(triggered(&d, 2), Some((1, 4)));
    }

    /// Deleting the only group (it's replaced by an empty one): every trigger goes Off.
    #[test]
    fn deleting_the_only_group_turns_triggers_off() {
        let mut d = Document::new_project().unwrap();
        assert_eq!(d.project().groups.len(), 1);
        d.set_lfo(Lfo { trigger: Some((0, 2)), ..Lfo::new(1) }).unwrap();
        d.set_lfo(Lfo { trigger: Some((0, 5)), ..Lfo::new(2) }).unwrap();
        d.delete_group(0).unwrap();
        assert_eq!(d.project().groups.len(), 1);
        assert_eq!(triggered(&d, 1), None);
        assert_eq!(triggered(&d, 2), None);
        assert!(d.undo(), "one step");
        assert_eq!(triggered(&d, 1), Some((0, 2)));
        assert_eq!(triggered(&d, 2), Some((0, 5)));
    }

    #[test]
    fn resetting_the_trigger_sound_turns_it_off() {
        let mut d = Document::new_project().unwrap();
        d.set_lfo(Lfo { trigger: Some((0, 2)), ..Lfo::new(1) }).unwrap();
        d.set_lfo(Lfo { trigger: Some((0, 3)), ..Lfo::new(2) }).unwrap();
        d.reset_sound(0, 2).unwrap();
        assert_eq!(triggered(&d, 1), None);
        assert_eq!(triggered(&d, 2), Some((0, 3)));
        assert!(d.undo());
        assert_eq!(triggered(&d, 1), Some((0, 2)));
    }

    #[test]
    fn maschine_does_not_see_the_field() {
        let mut d = Document::new_project().unwrap();
        d.set_lfo(Lfo::new(1)).unwrap();
        let Some(Value::Obj(root)) = d.tree.at(&[]) else { panic!("no root") };
        assert!(root.get(FIELD).is_some(), "the document has the field");
        let saved = d.to_bytes().unwrap();
        let archive = mxprj::Project::parse(&saved).unwrap().decode(&mxprj::types::Types::new()).unwrap();
        let mut found = false;
        archive.walk_paths(&mut |o, _| found |= o.fields.iter().any(|(f, _)| f.starts_with("deckard:")));
        assert!(!found);
    }
}
