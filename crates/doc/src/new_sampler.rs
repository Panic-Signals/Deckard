//! The New Sampler: marking a sound's Sampler DECKARD's, and its zones' analysis
//! (`deckard_model::new_sampler`).

use super::{Document, Op, Target};
use crate::chain::{Chain, NewModule};
use crate::zones::{ZoneFrom, ZoneSpec};
use deckard_model::new_sampler::{FIELD, NewSampler, PlayMode, ZONE_FIELD, ZoneMeta};
use mxprj::decode::Value;
use std::path::PathBuf;

/// An Audio plug-in's take, as a New Sampler zone (roadmap 1f): its file, the frames
/// it plays (low, high inclusive; `None`: all), and its tempo and length in beats.
#[derive(Debug, Clone)]
pub struct TakeZone {
    pub file: PathBuf,
    pub play: Option<(i64, i64)>,
    pub bpm: Option<f64>,
    pub beats: Option<f64>,
}

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

    /// Sound (`g`, `s`)'s Audio plug-in made a New Sampler (one undo step, "Convert to
    /// New Sampler"): a zone a take, in Sync at its tempo (Repitch without one), the
    /// plug-in's Tune on each. One take plays on every key from C3; more, one a key
    /// from C3 up.
    pub fn audio_to_new_sampler(&mut self, g: usize, s: usize, takes: &[TakeZone], tune: f64) -> Result<(), String> {
        if takes.is_empty() {
            return Err("the Audio plug-in has no takes".into());
        }
        let many = takes.len() > 1;
        self.transaction("Convert to New Sampler", |d| {
            let c = Chain::Sound(g, s);
            d.remove_slot_in(c, 0)?;
            d.add_slot_in(c, 0, &NewModule::Sampler)?;
            let specs: Vec<ZoneSpec> = takes
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let key = (60 + i as i64).min(127) as f64;
                    let mut set = vec![("root_key", key), ("tune", tune.clamp(-36.0, 36.0))];
                    if many {
                        set.extend([("key_range/low", key), ("key_range/high", key)]);
                    }
                    if let Some((lo, hi)) = t.play.filter(|(lo, hi)| hi > lo) {
                        set.extend([("play_range/low", lo as f64), ("play_range/high", hi as f64)]);
                    }
                    ZoneSpec { from: ZoneFrom::Template, file: Some(t.file.clone()), set }
                })
                .collect();
            d.set_zones(g, s, &specs, "Load")?;
            d.set_new_sampler(g, s, Some(&NewSampler::default()))?;
            let paths: Vec<String> = d.project().groups[g].sounds[s]
                .sampler
                .as_ref()
                .map(|x| x.zones.iter().map(|z| z.sample.path.clone()).collect())
                .unwrap_or_default();
            for (z, (t, file)) in takes.iter().zip(paths).enumerate() {
                let meta = ZoneMeta {
                    file,
                    mode: if t.bpm.is_some() { PlayMode::Sync } else { PlayMode::Repitch },
                    bpm: t.bpm,
                    bpm_source: deckard_model::new_sampler::BpmSource::Manual,
                    beats: t.beats,
                    ..ZoneMeta::default()
                };
                d.set_zone_meta(g, s, z, &meta, "Zone Tempo")?;
            }
            Ok(())
        })
    }

    /// Sound (`g`, `s`)'s New Sampler made a plain Sampler (one undo step, "Convert to
    /// Sampler"): its zones, ranges, loops, tune and envelopes stay; Sync, Keys and its
    /// zones' analysis go.
    pub fn new_sampler_to_sampler(&mut self, g: usize, s: usize) -> Result<(), String> {
        let zones =
            self.project().groups.get(g).and_then(|x| x.sounds.get(s)).and_then(|x| x.sampler.as_ref()).map_or(0, |x| x.zones.len());
        self.transaction("Convert to Sampler", |d| {
            let mut ops = d.new_sampler_ops(g, s, None)?;
            for z in 0..zones {
                ops.extend(d.zone_meta_ops(g, s, z, None)?);
            }
            if ops.is_empty() { Ok(()) } else { d.apply("Convert to Sampler", ops) }
        })
    }

    /// Change how zone `z` plays (its mode, its tempo or root set by hand): one undo
    /// step, `label`.
    pub fn set_zone_meta(&mut self, g: usize, s: usize, z: usize, meta: &ZoneMeta, label: &str) -> Result<(), String> {
        let ops = self.zone_meta_ops(g, s, z, Some(meta))?;
        if ops.is_empty() { Ok(()) } else { self.apply(label, ops) }
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
    fn set_zone_meta_is_undoable() {
        let mut d = doc("/s/a.wav");
        let file = path_of(&d);
        d.note_zone_meta(0, 0, 0, &meta(&file)).unwrap();
        let keys = ZoneMeta { mode: PlayMode::Keys, ..meta(&file) };
        d.set_zone_meta(0, 0, 0, &keys, "Play Mode").unwrap();
        assert_eq!(zone_meta(&d).map(|m| m.mode), Some(PlayMode::Keys));
        assert!(d.undo());
        assert_eq!(zone_meta(&d), Some(meta(&file)), "back to what analysis found");
        assert!(d.redo());
        assert_eq!(zone_meta(&d).map(|m| m.mode), Some(PlayMode::Keys));
    }

    #[test]
    fn a_sampler_becomes_a_new_sampler_and_back() {
        let mut d = doc("/s/a.wav");
        d.set_new_sampler(0, 0, Some(&NewSampler::default())).unwrap();
        let file = path_of(&d);
        d.set_zone_meta(0, 0, 0, &meta(&file), "Zone Tempo").unwrap();
        d.new_sampler_to_sampler(0, 0).unwrap();
        let snd = &d.project().groups[0].sounds[0];
        assert_eq!((snd.new_sampler, snd.instrument.as_str()), (None, "Sampler"));
        assert_eq!(zone_meta(&d), None, "its analysis goes");
        assert_eq!(path_of(&d), file, "its zone stays");
        assert_eq!(d.undo_label(), Some("Convert to Sampler"));
        assert!(d.undo());
        assert_eq!(zone_meta(&d), Some(meta(&file)));
    }

    #[test]
    fn an_audio_plugin_becomes_a_new_sampler() {
        let mut d = Document::new_project().unwrap();
        d.audio_only(0, 0).unwrap();
        let takes = [
            TakeZone { file: "/s/loop 94.wav".into(), play: Some((100, 9_999)), bpm: Some(94.0), beats: Some(4.0) },
            TakeZone { file: "/s/hit.wav".into(), play: None, bpm: None, beats: None },
        ];
        d.audio_to_new_sampler(0, 0, &takes, 2.0).unwrap();
        let snd = &d.project().groups[0].sounds[0];
        assert!(snd.new_sampler.is_some());
        assert_eq!(snd.instrument, "Sampler");
        let zones = &snd.sampler.as_ref().unwrap().zones;
        assert_eq!(zones.len(), 2);
        assert_eq!((zones[0].root_key, zones[0].key_range, zones[0].play_range, zones[0].tune), (60, (60, 60), (100, 9_999), 2.0));
        assert_eq!((zones[1].root_key, zones[1].key_range), (61, (61, 61)));
        let m = zones[0].meta.as_ref().unwrap();
        assert_eq!((m.mode, m.bpm, m.beats), (PlayMode::Sync, Some(94.0), Some(4.0)));
        assert_eq!(zones[1].meta.as_ref().unwrap().mode, PlayMode::Repitch);
        assert_eq!(d.undo_label(), Some("Convert to New Sampler"));
        assert!(d.undo());
        assert_eq!(d.project().groups[0].sounds[0].instrument, "AudioModule", "one step back");
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
