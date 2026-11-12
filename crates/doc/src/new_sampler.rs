//! The New Sampler: marking a sound's Sampler DECKARD's, and its zones' analysis
//! (`deckard_model::new_sampler`).

use super::{Document, Op, Target};
use deckard_model::new_sampler::{FIELD, NewSampler, ZONE_FIELD, ZoneMeta};
use mxprj::decode::Value;

impl Document {
    /// Make sound (`g`, `s`)'s Sampler a New Sampler with these settings, or (`None`) a
    /// plain Sampler again: one undo step.
    pub fn set_new_sampler(&mut self, g: usize, s: usize, settings: Option<&NewSampler>) -> Result<(), String> {
        let ops = self.new_sampler_ops(g, s, settings)?;
        let label = if settings.is_some() { "New Sampler" } else { "Convert to Sampler" };
        if ops.is_empty() { Ok(()) } else { self.apply(label, ops) }
    }

    /// The ops giving sound (`g`, `s`) these New Sampler settings (none if it has them).
    pub(crate) fn new_sampler_ops(&self, g: usize, s: usize, settings: Option<&NewSampler>) -> Result<Vec<Op>, String> {
        let path = self.owner(Target::Sound(g, s)).ok_or_else(|| format!("no sound {} in group {}", s + 1, g + 1))?;
        let Some(Value::Obj(o)) = self.tree.at(&path) else { return Err("a sound that isn't an object".into()) };
        let want = settings.map(|n| Value::Str(n.to_text()));
        Ok(if o.get(FIELD) != want.as_ref() { vec![Op::SetField { path, field: FIELD.to_string(), value: want }] } else { Vec::new() })
    }

    /// Keep what analysis found about zone `z`'s sample. Not an undo step: it's what the
    /// file is, not an edit (undo goes past it, and a zone given another file drops it).
    pub fn note_zone_meta(&mut self, g: usize, s: usize, z: usize, meta: &ZoneMeta) -> Result<(), String> {
        let ops = self.zone_meta_ops(g, s, z, Some(meta))?;
        if ops.is_empty() {
            return Ok(());
        }
        self.run(&ops)?;
        self.refresh();
        Ok(())
    }

    /// The ops giving zone `z` this meta (none if it has it).
    pub(crate) fn zone_meta_ops(&self, g: usize, s: usize, z: usize, meta: Option<&ZoneMeta>) -> Result<Vec<Op>, String> {
        let path = self.owner(Target::Zone(g, s, z)).ok_or("no such zone")?;
        let Some(Value::Obj(o)) = self.tree.at(&path) else { return Err("a zone that isn't an object".into()) };
        let want = meta.map(|m| Value::Str(m.to_text()));
        Ok(if o.get(ZONE_FIELD) != want.as_ref() {
            vec![Op::SetField { path, field: ZONE_FIELD.to_string(), value: want }]
        } else {
            Vec::new()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::NewModule;
    use deckard_model::new_sampler::{PlayMode, StretchEngine};
    use std::path::Path;

    /// A new project's first sound with a Sampler holding one zone of `file`.
    fn doc(file: &str) -> Document {
        let mut d = Document::new_project().unwrap();
        if d.project().groups[0].sounds[0].slots.is_empty() {
            d.add_slot(0, 0, 0, &NewModule::Sampler).unwrap();
        }
        let spec = crate::zones::ZoneSpec { from: crate::zones::ZoneFrom::Template, file: Some(file.into()), set: Vec::new() };
        d.set_zones(0, 0, &[spec], "Load").unwrap();
        d
    }

    fn meta(file: &str) -> ZoneMeta {
        ZoneMeta { file: file.into(), mode: PlayMode::Sync, bpm: Some(120.0), beats: Some(16.0), root: Some(45), ..ZoneMeta::default() }
    }

    fn path_of(d: &Document) -> String {
        d.project().groups[0].sounds[0].sampler.as_ref().unwrap().zones[0].sample.path.clone()
    }

    fn zone_meta(d: &Document) -> Option<ZoneMeta> {
        d.project().groups[0].sounds[0].sampler.as_ref().unwrap().zones[0].meta.clone()
    }

    #[test]
    fn marks_and_unmarks_a_sound() {
        let mut d = doc("/s/a.wav");
        assert_eq!(d.project().groups[0].sounds[0].new_sampler, None);
        let live = NewSampler { stretch: StretchEngine::Live };
        d.set_new_sampler(0, 0, Some(&live)).unwrap();
        assert_eq!(d.project().groups[0].sounds[0].new_sampler, Some(live));
        assert!(d.undo());
        assert_eq!(d.project().groups[0].sounds[0].new_sampler, None);
        assert!(d.redo());
        d.set_new_sampler(0, 0, None).unwrap();
        assert_eq!(d.project().groups[0].sounds[0].new_sampler, None);
    }

    #[test]
    fn zone_meta_is_no_undo_step() {
        let mut d = doc("/s/a.wav");
        d.set_new_sampler(0, 0, Some(&NewSampler::default())).unwrap();
        let file = path_of(&d);
        d.note_zone_meta(0, 0, 0, &meta(&file)).unwrap();
        assert_eq!(zone_meta(&d), Some(meta(&file)));
        assert!(d.undo()); // the New Sampler goes; the meta stays
        assert_eq!(d.project().groups[0].sounds[0].new_sampler, None);
        assert_eq!(zone_meta(&d), Some(meta(&file)));
    }

    #[test]
    fn meta_for_another_file_is_ignored() {
        let mut d = doc("/s/a.wav");
        let file = path_of(&d);
        d.note_zone_meta(0, 0, 0, &meta(&file)).unwrap();
        d.set_zone_sample(0, 0, 0, Path::new("/s/b.wav"), 1000, "Edit").unwrap();
        assert_eq!(zone_meta(&d), None);
    }

    #[test]
    fn only_a_sampler_is_a_new_sampler() {
        let mut d = doc("/s/a.wav");
        d.set_new_sampler(0, 0, Some(&NewSampler::default())).unwrap();
        let n = d.project().groups[0].sounds[0].slots.len();
        for i in (0..n).rev() {
            d.remove_slot(0, 0, i).unwrap();
        }
        d.add_slot(0, 0, 0, &NewModule::Audio).unwrap();
        assert_eq!(d.project().groups[0].sounds[0].new_sampler, None);
    }

    #[test]
    fn both_round_trip_through_a_save() {
        let mut d = doc("/s/a.wav");
        d.set_new_sampler(0, 0, Some(&NewSampler { stretch: StretchEngine::Live })).unwrap();
        let file = path_of(&d);
        d.note_zone_meta(0, 0, 0, &meta(&file)).unwrap();
        let again = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(again.project().groups[0].sounds[0].new_sampler, Some(NewSampler { stretch: StretchEngine::Live }));
        assert_eq!(zone_meta(&again), Some(meta(&file)));
    }

    #[test]
    fn maschine_sees_a_sampler() {
        let mut d = doc("/s/a.wav");
        d.set_new_sampler(0, 0, Some(&NewSampler::default())).unwrap();
        let file = path_of(&d);
        d.note_zone_meta(0, 0, 0, &meta(&file)).unwrap();
        let bytes = d.to_bytes().unwrap();
        let without =
            mxprj::container::set_root_child(&bytes, crate::block::DOMAIN, crate::block::ITEM, crate::block::FRAME, None).unwrap();
        let plain = Document::from_bytes(without, None).unwrap();
        let snd = &plain.project().groups[0].sounds[0];
        assert_eq!((snd.instrument.as_str(), snd.new_sampler), ("Sampler", None));
        assert_eq!(zone_meta(&plain), None);
    }
}
