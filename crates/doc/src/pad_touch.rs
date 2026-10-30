//! A sound's Pad Touch ([`deckard_model::pad_touch`]): DECKARD-only fields on the
//! sound's object, so copying, pasting and undo carry them, saved in the DECKARD block.

use super::{Document, Op, Target};
use deckard_model::pad_touch::{FIELDS, PadTouch};
use mxprj::decode::Value;

impl Document {
    /// Set sound (`g`, `s`)'s Pad Touch (kept in its ranges), one undo step. The
    /// default takes the fields off.
    pub fn set_pad_touch(&mut self, g: usize, s: usize, t: PadTouch) -> Result<(), String> {
        let ops = self.pad_touch_ops(g, s, t)?;
        if ops.is_empty() { Ok(()) } else { self.apply("Pad Touch", ops) }
    }

    /// The ops that give sound (`g`, `s`) these settings (none if it has them).
    pub(crate) fn pad_touch_ops(&self, g: usize, s: usize, t: PadTouch) -> Result<Vec<Op>, String> {
        let path = self.owner(Target::Sound(g, s)).ok_or_else(|| format!("no sound {} in group {}", s + 1, g + 1))?;
        let Some(Value::Obj(o)) = self.tree.at(&path) else { return Err("a sound that isn't an object".into()) };
        let t = t.clamped();
        let values = t.values();
        let mut ops = Vec::new();
        for (f, v) in FIELDS.iter().zip(values) {
            let want = (!t.is_default()).then_some(Value::Int(v));
            if o.get(f) != want.as_ref() {
                ops.push(Op::SetField { path: path.clone(), field: f.to_string(), value: want });
            }
        }
        Ok(ops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::pad_touch::TouchMode;

    fn touch(d: &Document, g: usize, s: usize) -> PadTouch {
        d.project().groups[g].sounds[s].pad_touch
    }

    fn poly() -> PadTouch {
        PadTouch { mode: TouchMode::PolyAt, cc: 74, curve: 70, min: 10, max: 90 }
    }

    #[test]
    fn pad_touch_is_set_and_undone() {
        let mut d = Document::new_project().unwrap();
        d.set_pad_touch(0, 0, poly()).unwrap();
        assert_eq!(touch(&d, 0, 0), poly());
        assert!(d.undo());
        assert_eq!(touch(&d, 0, 0), PadTouch::default());
        assert!(d.redo());
        assert_eq!(touch(&d, 0, 0), poly());
    }

    /// Two settings turned one after the other, quickly (CURVE, then MIN on the MK3):
    /// undo and redo step back and forth through what was really set.
    #[test]
    fn quick_edits_to_two_settings_undo_in_turn() {
        let mut d = Document::new_project().unwrap();
        let cc = PadTouch { mode: TouchMode::Cc, ..PadTouch::default() };
        let curve = PadTouch { curve: 80, ..cc };
        let min = PadTouch { min: 30, ..curve };
        for t in [cc, curve, min] {
            d.set_pad_touch(0, 0, t).unwrap();
        }
        assert!(d.undo());
        assert_eq!(touch(&d, 0, 0), curve);
        assert!(d.undo());
        assert_eq!(touch(&d, 0, 0), cc);
        assert!(d.redo());
        assert_eq!(touch(&d, 0, 0), curve);
        assert!(d.redo());
        assert_eq!(touch(&d, 0, 0), min);
    }

    /// In a real project, setting one sound's Pad Touch leaves every other sound's as it
    /// was (sounds can share objects in Maschine's files).
    #[test]
    fn only_that_sound_changes_in_a_real_project() {
        let path = std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj");
        if !path.exists() {
            eprintln!("{} not found; skipping", path.display());
            return;
        }
        let mut d = Document::open(path).unwrap();
        d.set_pad_touch(0, 2, poly()).unwrap();
        for (g, group) in d.project().groups.iter().enumerate() {
            for (s, sound) in group.sounds.iter().enumerate() {
                let want = if (g, s) == (0, 2) { poly() } else { PadTouch::default() };
                assert_eq!(sound.pad_touch, want, "group {g} sound {s}");
            }
        }
    }

    #[test]
    fn pad_touch_clamps() {
        let mut d = Document::new_project().unwrap();
        d.set_pad_touch(0, 1, PadTouch { mode: TouchMode::Cc, cc: 200, curve: 150, min: 60, max: 20 }).unwrap();
        assert_eq!(touch(&d, 0, 1), PadTouch { mode: TouchMode::Cc, cc: 127, curve: 100, min: 60, max: 60 });
    }

    #[test]
    fn pad_touch_saves_and_reopens() {
        let mut d = Document::new_project().unwrap();
        let cc = PadTouch { mode: TouchMode::Cc, cc: 1, curve: 30, min: 0, max: 80 };
        d.set_pad_touch(0, 0, poly()).unwrap();
        d.set_pad_touch(0, 3, cc).unwrap();
        let bytes = d.to_bytes().unwrap();
        let back = Document::from_bytes(bytes.clone(), None).unwrap();
        assert_eq!(touch(&back, 0, 0), poly());
        assert_eq!(touch(&back, 0, 3), cc);
        // Only in the block: Maschine's part has none of it.
        let maschine =
            mxprj::container::set_root_child(&bytes, crate::block::DOMAIN, crate::block::ITEM, crate::block::FRAME, None).unwrap();
        assert_eq!(touch(&Document::from_bytes(maschine, None).unwrap(), 0, 0), PadTouch::default());
        assert_eq!(touch(&back, 0, 1), PadTouch::default());
        // Saved again, the same.
        let again = Document::from_bytes(back.to_bytes().unwrap(), None).unwrap();
        assert_eq!(touch(&again, 0, 3), cc);
    }

    /// The settings go with the sound: pasted onto another sound, into another project,
    /// and saved there.
    #[test]
    fn pad_touch_follows_its_sound() {
        let mut d = Document::new_project().unwrap();
        d.set_pad_touch(0, 2, poly()).unwrap();
        let clip = d.copy_sound(0, 2).unwrap();
        d.paste_sounds(0, 5, std::slice::from_ref(&clip)).unwrap();
        assert_eq!(touch(&d, 0, 5), poly());
        let mut other = Document::new_project().unwrap();
        let carried = clip.carried(&d, &mut other);
        other.paste_sounds(0, 0, &[carried]).unwrap();
        assert_eq!(touch(&other, 0, 0), poly());
        let back = Document::from_bytes(other.to_bytes().unwrap(), None).unwrap();
        assert_eq!(touch(&back, 0, 0), poly());
    }

    #[test]
    fn defaults_save_as_before() {
        let d = Document::new_project().unwrap();
        let before = d.to_bytes().unwrap();
        let mut e = Document::new_project().unwrap();
        e.set_pad_touch(0, 0, PadTouch::default()).unwrap();
        e.set_pad_touch(0, 1, poly()).unwrap();
        e.set_pad_touch(0, 1, PadTouch::default()).unwrap();
        let after = e.to_bytes().unwrap();
        assert!(mxprj::container::root_child(&after, crate::block::DOMAIN, crate::block::ITEM).is_none(), "no block");
        assert_eq!(before.len(), after.len());
    }

    /// A version 1 block (expression only, before Pad Touch) still opens.
    #[test]
    fn a_version_1_block_still_reads() {
        let mut d = Document::new_project().unwrap();
        d.add_pattern(0, 0, "Pattern 1").unwrap();
        let n = deckard_model::Note { expr: None, sound: 0, position: 0, length: 960, pitch: 60, velocity: 100, probability: 100 };
        d.add_note(0, 0, n).unwrap();
        let e = deckard_model::Expression { release_velocity: Some(64), ..Default::default() };
        let id = d.add_expression(e.clone());
        d.edit_notes("Expression", 0, 0, &[n], &[deckard_model::Note { expr: Some(id), ..n }]).unwrap();
        let bytes = d.to_bytes().unwrap();
        let block = mxprj::container::root_child(&bytes, crate::block::DOMAIN, crate::block::ITEM).unwrap();
        // Version 1: the header says 1, and there's no Pad Touch section (its count, 0).
        let mut v1 = block[..block.len() - 4].to_vec();
        v1[..4].copy_from_slice(&1u32.to_le_bytes());
        let file =
            mxprj::container::set_root_child(&bytes, crate::block::DOMAIN, crate::block::ITEM, crate::block::FRAME, Some(&v1)).unwrap();
        let back = Document::from_bytes(file, None).unwrap();
        let got = back.project().groups[0].patterns[0].notes[0].expr.and_then(|id| back.expression(id).cloned());
        assert_eq!(got.and_then(|e| e.release_velocity), Some(64));
        assert_eq!(touch(&back, 0, 0), PadTouch::default());
    }
}
