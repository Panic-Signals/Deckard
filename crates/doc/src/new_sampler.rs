//! The New Sampler: marking a sound's Sampler DECKARD's, and its zones' analysis
//! (`deckard_model::new_sampler`).

use super::{Document, Op, Target};
use crate::chain::{Chain, NewModule};
use crate::zones::{ZoneFrom, ZoneSpec};
use deckard_model::layers::Layer;
use deckard_model::new_sampler::{FIELD, NewSampler, PlayMode, SamplerMode, ZONE_FIELD, ZoneMeta};
use deckard_model::{Note, PPQ};
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
        let label = if settings.is_some() { "Sampler Preview" } else { "Convert to Sampler" };
        if ops.is_empty() { Ok(()) } else { self.apply(label, ops) }
    }

    /// Sound (`g`, `s`)'s New Sampler settings changed, as the undo step `label` (a
    /// setting dragged is one step, as a zone's are).
    pub fn set_new_sampler_as(&mut self, g: usize, s: usize, settings: &NewSampler, label: &str) -> Result<(), String> {
        let ops = self.new_sampler_ops(g, s, Some(settings))?;
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
    ///
    /// Where a pattern played a take (Loop mode), it gets the take's notes: one each
    /// time the take came round in the pattern (from its offset; a part cut off by the
    /// pattern's start isn't played), as long as the take or what's left of the
    /// pattern, and the Sampler's envelope made ADSR so they end with it. In Gate mode
    /// (`gate`) the pattern's notes play it as they did: a lone take in Keys.
    pub fn audio_to_new_sampler(&mut self, g: usize, s: usize, takes: &[TakeZone], tune: f64, gate: bool) -> Result<(), String> {
        if takes.is_empty() {
            return Err("the Audio plug-in has no takes".into());
        }
        let many = takes.len() > 1;
        // (pattern, its notes for the takes it played).
        let mut cued: Vec<(usize, Vec<Note>)> = Vec::new();
        {
            let p = self.project();
            let bar = p.bar_length();
            for (pi, pat) in p.groups.get(g).map_or(&[][..], |x| &x.patterns).iter().enumerate() {
                let Some(c) = pat.audio.iter().find(|c| c.sound == s) else { continue };
                let mut notes = Vec::new();
                if !gate && let Some(t) = takes.get(c.take) {
                    let (from, to) = (pat.start, pat.start + if pat.length > 0 { pat.length } else { bar });
                    let pitch = 60 + if many { c.take as i64 } else { 0 };
                    let note = |position: i64, length: i64| Note {
                        sound: s,
                        position,
                        length,
                        pitch,
                        velocity: 127,
                        probability: 100,
                        expr: None,
                        ratchet: Default::default(),
                    };
                    match t.beats.filter(|b| t.bpm.is_some() && *b > 0.0).map(|b| (b * PPQ as f64).round() as i64).filter(|l| *l > 0) {
                        Some(len) => {
                            // The first time it starts in the pattern.
                            let mut at = c.offset + (from - c.offset).div_euclid(len) * len;
                            if at < from {
                                at += len;
                            }
                            while at < to {
                                notes.push(note(at, len.min(to - at)));
                                at += len;
                            }
                        }
                        None if (from..to).contains(&c.offset) => notes.push(note(c.offset, to - c.offset)),
                        None => {}
                    }
                }
                cued.push((pi, notes));
            }
        }
        self.transaction("Convert to Sampler Preview", |d| {
            let patterns: Vec<usize> = cued.iter().map(|c| c.0).collect();
            if !patterns.is_empty() {
                d.set_audio_in(g, s, &patterns, None, "Audio")?;
            }
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
                    mode: match t.bpm {
                        Some(_) if gate && !many => PlayMode::Keys,
                        Some(_) => PlayMode::Sync,
                        None => PlayMode::Repitch,
                    },
                    bpm: t.bpm,
                    bpm_source: deckard_model::new_sampler::BpmSource::Manual,
                    beats: t.beats,
                    ..ZoneMeta::default()
                };
                d.set_zone_meta(g, s, z, &meta, "Zone Tempo")?;
            }
            if cued.iter().any(|c| !c.1.is_empty()) {
                d.set("Envelope", Target::Sampler(g, s), "envelope_type", Value::Float(2.0))?;
                d.set("Envelope", Target::Sampler(g, s), "volume_envelope/release", Value::Float(0.1))?;
                for (pi, notes) in &cued {
                    d.edit_notes("Notes", g, *pi, &[], notes)?;
                }
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

    /// Sound (`g`, `s`)'s Sampler Preview put in `mode`: one undo step, "Sampler Mode".
    /// `keep: None`: only the setting changes (a sound without a sample, or Original made
    /// Advanced, which loses nothing). `keep: Some((z, frames))`: zone `z` becomes the
    /// sound's only zone, on every key and velocity, its loop off, over its whole file
    /// (`frames` long; 0: its play range stays), with the mode's settings at their
    /// defaults. What DECKARD knew of it goes, as for a sample just loaded; in Layers it
    /// is the first layer, and a One-shot envelope becomes ADSR (as a first layer makes
    /// it).
    pub fn set_sampler_mode(&mut self, g: usize, s: usize, mode: SamplerMode, keep: Option<(usize, usize)>) -> Result<(), String> {
        let snd = self.project().groups.get(g).and_then(|x| x.sounds.get(s)).ok_or("no such sound")?;
        let now = snd.new_sampler.ok_or("not a Sampler Preview")?;
        let zones = snd.sampler.as_ref().map_or(0, |x| x.zones.len());
        if keep.is_some_and(|(z, _)| z >= zones) {
            return Err("no such zone".into());
        }
        let label = "Sampler Mode";
        self.transaction(label, |d| {
            let Some((z, frames)) = keep else {
                return d.set_new_sampler_as(g, s, &NewSampler { mode, ..now }, label);
            };
            let mut set = vec![
                ("key_range/low", 0.0),
                ("key_range/high", 127.0),
                ("velocity_range/low", 0.0),
                ("velocity_range/high", 127.0),
                ("loop_active", 0.0),
            ];
            if frames > 0 {
                let end = (frames - 1) as f64;
                set.extend([("play_range/low", 0.0), ("play_range/high", end), ("loop_range/low", 0.0), ("loop_range/high", end)]);
            }
            d.set_zones(g, s, &[ZoneSpec { from: ZoneFrom::Zone(z), file: None, set }], label)?;
            let file = d.project().groups[g].sounds[s].sampler.as_ref().ok_or("no sampler")?.zones[0].sample.path.clone();
            let meta = (mode == SamplerMode::Layers).then(|| ZoneMeta { file, layer: Some(Layer::default()), ..Default::default() });
            let ops = d.zone_meta_ops(g, s, 0, meta.as_ref())?;
            if !ops.is_empty() {
                d.apply(label, ops)?;
            }
            if mode == SamplerMode::Layers && d.project().groups[g].sounds[s].sampler.as_ref().is_some_and(|x| x.envelope_type == 0) {
                d.set(label, Target::Sampler(g, s), "envelope_type", Value::Float(2.0))?;
            }
            d.set_new_sampler_as(g, s, &NewSampler { mode, ..Default::default() }, label)
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

    fn sound(d: &Document) -> deckard_model::Sound {
        d.project().groups[0].sounds[0].clone()
    }

    /// A Sampler Preview made before modes: in Layers if it has a layer, else Advanced;
    /// a save keeps it.
    #[test]
    fn an_unwritten_mode_is_read_from_the_zones() {
        let mut d = doc("/s/a.wav");
        d.set_new_sampler(0, 0, Some(&NewSampler::default())).unwrap();
        assert_eq!(sound(&d).new_sampler.unwrap().mode, SamplerMode::Advanced);
        let file = path_of(&d);
        let layer = ZoneMeta { file, layer: Some(Layer::default()), ..Default::default() };
        d.set_zone_meta(0, 0, 0, &layer, "Layer").unwrap();
        assert_eq!(sound(&d).new_sampler.unwrap().mode, SamplerMode::Layers);
        let again = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(sound(&again).new_sampler.unwrap().mode, SamplerMode::Layers);
        // Said, the mode is the one said, whatever the zones hold.
        d.set_new_sampler(0, 0, Some(&NewSampler { mode: SamplerMode::Original, ..Default::default() })).unwrap();
        assert_eq!(sound(&d).new_sampler.unwrap().mode, SamplerMode::Original);
        let again = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(sound(&again).new_sampler.unwrap().mode, SamplerMode::Original);
    }

    fn three_zones() -> Document {
        let mut d = doc("/s/a.wav");
        let spec = |f: &str| crate::zones::ZoneSpec {
            from: crate::zones::ZoneFrom::Template,
            file: Some(f.into()),
            set: vec![("key_range/low", 60.0), ("key_range/high", 60.0), ("play_range/low", 10.0), ("play_range/high", 20.0)],
        };
        d.set_zones(0, 0, &[spec("/s/a.wav"), spec("/s/b.wav"), spec("/s/c.wav")], "Load").unwrap();
        d.set_new_sampler(0, 0, Some(&NewSampler { motion: deckard_model::new_sampler::Motion::Next, ..Default::default() })).unwrap();
        d.set("Envelope", Target::Sampler(0, 0), "envelope_type", Value::Float(0.0)).unwrap();
        d
    }

    #[test]
    fn a_mode_change_keeps_one_sample() {
        let mut d = three_zones();
        let b = sound(&d).sampler.unwrap().zones[1].sample.path.clone();
        d.set_zone_meta(0, 0, 1, &meta(&b), "Zone Tempo").unwrap();
        d.set_sampler_mode(0, 0, SamplerMode::Layers, Some((1, 1000))).unwrap();
        let snd = sound(&d);
        let smp = snd.sampler.as_ref().unwrap();
        assert_eq!(smp.zones.len(), 1);
        let z = &smp.zones[0];
        assert_eq!((z.sample.path.as_str(), z.key_range, z.play_range), (b.as_str(), (0, 127), (0, 999)));
        assert_eq!(z.meta, Some(ZoneMeta { file: b.clone(), layer: Some(Layer::default()), ..Default::default() }));
        assert_eq!(snd.new_sampler, Some(NewSampler { mode: SamplerMode::Layers, ..Default::default() }));
        assert_eq!(smp.envelope_type, 2, "ADSR: a layer's release has something to follow");
        assert_eq!(d.undo_label(), Some("Sampler Mode"));
        assert!(d.undo());
        let snd = sound(&d);
        let smp = snd.sampler.as_ref().unwrap();
        assert_eq!((smp.zones.len(), smp.envelope_type), (3, 0), "one step back");
        assert_eq!(smp.zones[1].meta, Some(meta(&b)));
        assert_eq!(snd.new_sampler.unwrap().mode, SamplerMode::Advanced);
        assert!(d.redo());
        assert_eq!(sound(&d).sampler.unwrap().zones.len(), 1);
        // To Advanced or Original: no layer; what was known of the sample goes.
        d.set_sampler_mode(0, 0, SamplerMode::Original, Some((0, 0))).unwrap();
        let snd = sound(&d);
        assert_eq!(snd.sampler.as_ref().unwrap().zones[0].meta, None);
        assert_eq!(snd.sampler.as_ref().unwrap().zones[0].play_range, (0, 999), "frames not known: the range stays");
        assert_eq!(snd.new_sampler.unwrap().mode, SamplerMode::Original);
        assert!(d.set_sampler_mode(0, 0, SamplerMode::Advanced, Some((4, 0))).is_err(), "no such zone");
    }

    #[test]
    fn original_to_advanced_keeps_everything() {
        let mut d = three_zones();
        let n = NewSampler { mode: SamplerMode::Original, ..sound(&d).new_sampler.unwrap() };
        d.set_new_sampler(0, 0, Some(&n)).unwrap();
        d.set_sampler_mode(0, 0, SamplerMode::Advanced, None).unwrap();
        let snd = sound(&d);
        assert_eq!(snd.sampler.as_ref().unwrap().zones.len(), 3);
        assert_eq!(snd.new_sampler, Some(NewSampler { mode: SamplerMode::Advanced, ..n }));
        assert_eq!(d.undo_label(), Some("Sampler Mode"));
        let mut plain = doc("/s/a.wav");
        assert!(plain.set_sampler_mode(0, 0, SamplerMode::Layers, None).is_err(), "not a Sampler Preview");
    }

    #[test]
    fn marks_and_unmarks_a_sound() {
        let mut d = doc("/s/a.wav");
        assert_eq!(d.project().groups[0].sounds[0].new_sampler, None);
        let live = NewSampler { stretch: StretchEngine::Live, ..Default::default() };
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
        d.audio_to_new_sampler(0, 0, &takes, 2.0, false).unwrap();
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
        assert_eq!(d.undo_label(), Some("Convert to Sampler Preview"));
        assert!(d.undo());
        assert_eq!(d.project().groups[0].sounds[0].instrument, "AudioModule", "one step back");
    }

    /// Patterns that played a take get its notes: one each time it came round (from
    /// its offset), cut at the pattern's end; one undo step with the rest.
    #[test]
    fn a_converted_audio_plugin_keeps_its_patterns() {
        use crate::TakeSpec;
        let mut d = Document::new_project().unwrap();
        while d.project().groups[0].patterns.len() < 2 {
            let n = d.project().groups[0].patterns.len();
            d.add_pattern(0, if n == 0 { usize::MAX } else { 0 }, "P").unwrap();
        }
        d.audio_only(0, 0).unwrap();
        let spec = |beats| TakeSpec { file: "/s/a.wav".into(), frames: 48_000, rate: 48_000, beats };
        d.set_takes(0, 0, &[spec(1.5), spec(2.0)], "Load").unwrap();
        d.set_audio_in(0, 0, &[0], Some(0), "Audio").unwrap();
        d.set_audio_in(0, 0, &[1], Some(1), "Audio").unwrap();
        // The second pattern's take starts half a beat early.
        d.set_audio_offset(0, 1, 0, -PPQ / 2, "Move").unwrap();
        let bar = d.project().bar_length();
        assert_eq!(bar, 4 * PPQ);
        let takes = [
            TakeZone { file: "/s/a.wav".into(), play: None, bpm: Some(90.0), beats: Some(1.5) },
            TakeZone { file: "/s/b.wav".into(), play: None, bpm: Some(120.0), beats: Some(2.0) },
        ];
        d.audio_to_new_sampler(0, 0, &takes, 0.0, false).unwrap();
        let p = d.project();
        let got = |pi: usize| -> Vec<(i64, i64, i64)> {
            p.groups[0].patterns[pi].notes.iter().map(|n| (n.position, n.length, n.pitch)).collect()
        };
        let h = PPQ / 2;
        // 1.5 beats, three times in a bar: the last cut to a beat.
        assert_eq!(got(0), [(0, 3 * h, 60), (3 * h, 3 * h, 60), (6 * h, 2 * h, 60)]);
        // 2 beats from half a beat early: whole ones from 1.5 on, the last cut.
        assert_eq!(got(1), [(3 * h, 4 * h, 61), (7 * h, h, 61)]);
        assert!(p.groups[0].patterns.iter().all(|x| x.audio.is_empty()), "no audio events left");
        assert_eq!(p.groups[0].sounds[0].sampler.as_ref().unwrap().envelope_type, 2, "ADSR: the notes end it");
        assert!(d.undo());
        let p = d.project();
        assert_eq!(p.groups[0].sounds[0].instrument, "AudioModule", "one step back");
        assert_eq!(p.groups[0].patterns[1].audio.len(), 1);
        assert!(p.groups[0].patterns[0].notes.is_empty());
        // Gate mode: the pattern's own notes play it, a lone take in Keys.
        let mut d = Document::new_project().unwrap();
        d.audio_only(0, 0).unwrap();
        d.audio_to_new_sampler(0, 0, &takes[..1], 0.0, true).unwrap();
        assert_eq!(d.project().groups[0].sounds[0].sampler.as_ref().unwrap().zones[0].meta.as_ref().unwrap().mode, PlayMode::Keys);
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
        d.set_new_sampler(0, 0, Some(&NewSampler { stretch: StretchEngine::Live, ..Default::default() })).unwrap();
        let file = path_of(&d);
        d.note_zone_meta(0, 0, 0, &meta(&file)).unwrap();
        let again = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert_eq!(
            again.project().groups[0].sounds[0].new_sampler,
            Some(NewSampler { stretch: StretchEngine::Live, ..Default::default() })
        );
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
