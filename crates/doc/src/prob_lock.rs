//! A sound's PROB LOCK ([`deckard_model::Sound::probability_locked`]): a DECKARD-only
//! field on the sound's object, so copying, pasting and undo carry it, saved in the
//! DECKARD block.

use super::{Document, Op, Target};
use deckard_model::PROB_LOCK;
use mxprj::decode::Value;

impl Document {
    /// PROB LOCK of sound (`g`, `s`) on or off, one undo step.
    pub fn set_probability_locked(&mut self, g: usize, s: usize, on: bool) -> Result<(), String> {
        let ops = self.prob_lock_ops(g, s, on)?;
        if ops.is_empty() { Ok(()) } else { self.apply("Prob Lock", ops) }
    }

    /// The ops that turn sound (`g`, `s`)'s PROB LOCK to `on` (none if it is).
    pub(crate) fn prob_lock_ops(&self, g: usize, s: usize, on: bool) -> Result<Vec<Op>, String> {
        let path = self.owner(Target::Sound(g, s)).ok_or_else(|| format!("no sound {} in group {}", s + 1, g + 1))?;
        let Some(Value::Obj(o)) = self.tree.at(&path) else { return Err("a sound that isn't an object".into()) };
        let want = on.then_some(Value::Int(1));
        if o.get(PROB_LOCK) == want.as_ref() {
            return Ok(Vec::new());
        }
        Ok(vec![Op::SetField { path, field: PROB_LOCK.to_string(), value: want }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locked(d: &Document, s: usize) -> bool {
        d.project().groups[0].sounds[s].probability_locked
    }

    #[test]
    fn prob_lock_is_set_undone_and_saved() {
        let mut d = Document::new_project().unwrap();
        d.set_probability_locked(0, 2, true).unwrap();
        assert!(locked(&d, 2) && !locked(&d, 1));
        assert!(d.undo());
        assert!(!locked(&d, 2));
        assert!(d.redo());
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert!(locked(&back, 2) && !locked(&back, 1), "kept in the DECKARD block");
        d.set_probability_locked(0, 2, false).unwrap();
        assert!(!locked(&d, 2));
    }
}
