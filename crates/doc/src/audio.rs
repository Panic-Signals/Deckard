//! The Audio plug-in (docs/audio-plugin.md): a sound made one, its takes (the Audio
//! Pool), and which take each pattern plays (the pattern's `AudioEvent` for the sound).
//! Its settings (`mode`, `fade`, `tune`, `pitchbend`, `engine`) are the module's fields
//! (`Target::Module(g, s, 0)`); a take's (`length` in beats, `play_range/low`, …) are
//! `Target::Take(g, s, k)`'s.

use crate::arrange::px_mut;
use crate::chain::{NewModule, set_param};
use crate::zones::file_location;
use crate::{Document, Op, Target};
use mxprj::decode::Value;
use std::path::PathBuf;

const TAKE: &str = "NI::MASCOMMON::DATA::Take";

/// A take to make: its file, how many frames long, at what rate, and its length in
/// beats (its tempo is `beats × 60 × rate / frames`).
#[derive(Debug, Clone)]
pub struct TakeSpec {
    pub file: PathBuf,
    pub frames: usize,
    pub rate: u32,
    pub beats: f64,
}

impl Document {
    /// Make sound `s`'s chain a lone Audio plug-in (one undo step): one already there
    /// stays, with its takes; everything else goes. Returns whether anything changed.
    pub fn audio_only(&mut self, g: usize, s: usize) -> Result<bool, String> {
        let slots = self.project.groups.get(g).and_then(|x| x.sounds.get(s)).map(|x| x.slots.clone()).ok_or("no such sound")?;
        let keep = slots.iter().position(|x| x.module == "AudioModule");
        if keep.is_some() && slots.len() == 1 {
            return Ok(false);
        }
        self.transaction("Audio", |d| {
            for at in (0..slots.len()).rev().filter(|&at| Some(at) != keep) {
                d.remove_slot(g, s, at)?;
            }
            if keep.is_none() {
                d.add_slot(g, s, 0, &NewModule::Audio)?;
            }
            Ok(())
        })?;
        Ok(true)
    }

    /// The Audio plug-in's takes list (the path of its items, of its `current`).
    fn take_paths(&self, g: usize, s: usize) -> Result<(mxprj::decode::Path, mxprj::decode::Path), String> {
        let module = self.owner(Target::Module(g, s, 0)).ok_or("no such sound")?;
        let Some(Value::Obj(m)) = self.tree.at(&module) else { return Err("no module".into()) };
        if m.type_name != "NI::MASCOMMON::DATA::AudioModule" {
            return Err("the sound's first module isn't the Audio plug-in".into());
        }
        Ok((self.field_path(&module, &["takes", "objects", "items"])?, self.field_path(&module, &["takes", "current"])?))
    }

    /// A new take (shared_ptr item) of `spec`, and its object id.
    fn take_item(&mut self, spec: &TakeSpec) -> Result<(Value, u64), String> {
        let mut item = self.build(|b| b.item(TAKE))?;
        let t = px_mut(&mut item).ok_or("bad take")?;
        let oid = t.oid.ok_or("take without id")?;
        *t.get_mut("file").ok_or("take without file")? = file_location(&spec.file);
        let frames = spec.frames as f64;
        set_param(t, "length", spec.beats);
        // As Maschine keeps it: the take's duration in beats at 120 BPM.
        if let Some(Value::Obj(p)) = t.get_mut("length")
            && let Some(t1) = p.find_mut("t1")
        {
            *t1 = Value::Float((frames / f64::from(spec.rate.max(1)) * 2.0) as f32 as f64);
        }
        for r in ["play_range", "selection_range"] {
            set_param(t, &format!("{r}/low"), 0.0);
            set_param(t, &format!("{r}/high"), frames);
        }
        set_param(t, "unknown_98", frames);
        set_param(t, "zoom_factor", 1.0);
        Ok((item, oid))
    }

    /// Replace the Audio plug-in's takes with `specs` (the first in focus), as one undo
    /// step.
    pub fn set_takes(&mut self, g: usize, s: usize, specs: &[TakeSpec], label: &str) -> Result<(), String> {
        let (items_path, current_path) = self.take_paths(g, s)?;
        let mut items = Vec::new();
        let mut first = None;
        for spec in specs {
            let (item, oid) = self.take_item(spec)?;
            first = first.or(Some(oid));
            items.push(item);
        }
        let ops = vec![
            Op::Set { path: items_path, value: Value::List(items) },
            Op::Set { path: current_path, value: first.map_or(Value::Null, Value::Ref) },
        ];
        self.apply(label, ops)
    }

    /// Add a take (the Audio Pool's newest), in focus, as one undo step; returns its
    /// index.
    pub fn add_take(&mut self, g: usize, s: usize, spec: &TakeSpec, label: &str) -> Result<usize, String> {
        let (items_path, current_path) = self.take_paths(g, s)?;
        let mut items = self.items(&items_path)?;
        let (item, oid) = self.take_item(spec)?;
        items.push(item);
        let k = items.len() - 1;
        let ops = vec![Op::Set { path: items_path, value: Value::List(items) }, Op::Set { path: current_path, value: Value::Ref(oid) }];
        self.apply(label, ops)?;
        Ok(k)
    }

    /// Point take `k` at `file` (an edit of its sample, `frames` long), played whole;
    /// its length in beats scaled with its frames, so its tempo stays. One undo step.
    pub fn set_take_sample(
        &mut self,
        g: usize,
        s: usize,
        k: usize,
        file: &std::path::Path,
        frames: usize,
        label: &str,
    ) -> Result<(), String> {
        let take = self.owner(Target::Take(g, s, k)).ok_or("no such take")?;
        let a = self.project().groups[g].sounds[s].audio.clone().ok_or("no Audio plug-in")?;
        let t = a.takes.get(k).cloned().ok_or("no such take")?;
        let old = if t.play_range.1 > t.play_range.0 { (t.play_range.1 - t.play_range.0) as f64 } else { frames as f64 };
        let beats = if old > 0.0 && t.beats > 0.0 { t.beats * frames as f64 / old } else { t.beats };
        let file_path = self.field_path(&take, &["file"])?;
        let mut ops = vec![Op::Set { path: file_path, value: file_location(file) }];
        let tt = Target::Take(g, s, k);
        ops.extend(self.param_ops_pub(tt, "length", Value::Float(beats))?);
        for r in ["play_range", "selection_range"] {
            ops.extend(self.param_ops_pub(tt, &format!("{r}/low"), Value::Int(0))?);
            ops.extend(self.param_ops_pub(tt, &format!("{r}/high"), Value::Int(frames as i64))?);
        }
        ops.extend(self.param_ops_pub(tt, "unknown_98", Value::Int(frames as i64))?);
        self.apply(label, ops)
    }

    /// Bounce sound `s` in place (one undo step, "Bounce"): its chain becomes a lone
    /// Audio plug-in whose takes are `takes` (each pattern's render), each of those
    /// patterns playing its own; the sound's notes and MIDI events leave them.
    pub fn bounce(&mut self, g: usize, s: usize, takes: &[(usize, TakeSpec)]) -> Result<(), String> {
        let patterns = self.project.groups.get(g).map(|x| x.patterns.clone()).ok_or("no such group")?;
        self.transaction("Bounce", |d| {
            d.audio_only(g, s)?;
            let specs: Vec<TakeSpec> = takes.iter().map(|(_, t)| t.clone()).collect();
            d.set_takes(g, s, &specs, "Bounce")?;
            for (k, &(pi, _)) in takes.iter().enumerate() {
                d.set_audio_in(g, s, &[pi], Some(k), "Bounce")?;
                let pat = patterns.get(pi).ok_or("no such pattern")?;
                let notes: Vec<_> = pat.notes.iter().filter(|n| n.sound == s).copied().collect();
                let midi: Vec<_> = pat.midi.iter().filter(|m| m.sound == s).copied().collect();
                if !notes.is_empty() {
                    d.edit_notes("Bounce", g, pi, &notes, &[])?;
                }
                if !midi.is_empty() {
                    d.edit_midi("Bounce", g, pi, &midi, &[])?;
                }
            }
            Ok(())
        })
    }

    /// In each of `patterns`, sound `s`'s Audio plug-in plays take `take` (none: it's
    /// off there), as one undo step.
    pub fn set_audio_in(&mut self, g: usize, s: usize, patterns: &[usize], take: Option<usize>, label: &str) -> Result<(), String> {
        let take_oid = match take {
            Some(k) => {
                let path = self.owner(Target::Take(g, s, k)).ok_or("no such take")?;
                let Some(Value::Obj(t)) = self.tree.at(&path) else { return Err("no such take".into()) };
                Some(t.oid.ok_or("take without id")?)
            }
            None => None,
        };
        self.transaction(label, |d| {
            for &p in patterns {
                let ops = d.audio_event_ops(g, p, s, take_oid)?;
                if !ops.is_empty() {
                    d.apply(label, ops)?;
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::TakeSpec;
    use crate::{Document, Target};
    use deckard_model::{AudioCue, AudioEngine, AudioMode};
    use mxprj::decode::Value;

    fn check(mut d: Document) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let file = dir.join("Loop[94].wav");
        std::fs::write(&file, b"placeholder").unwrap();
        let (g, s) = (0, 3);
        if d.project().groups[g].patterns.is_empty() {
            d.add_pattern(g, usize::MAX, "Pattern 1").unwrap();
        }
        assert!(d.audio_only(g, s).unwrap());
        let spec = TakeSpec { file: file.clone(), frames: 112_596, rate: 44_100, beats: 4.0 };
        d.set_takes(g, s, &[spec], "Load Loop").unwrap();
        d.set_audio_in(g, s, &[0], Some(0), "Load Loop").unwrap();
        d.set("Audio", Target::Module(g, s, 0), "engine", Value::Float(2.0)).unwrap();
        let look = |d: &Document| {
            let snd = &d.project().groups[g].sounds[s];
            let a = snd.audio.clone().expect("an Audio plug-in");
            assert_eq!(snd.instrument, "AudioModule");
            assert_eq!(a.takes.len(), 1);
            assert_eq!(a.takes[0].beats, 4.0);
            assert!((a.takes[0].tempo(112_596, 44_100) - 94.0).abs() < 0.01);
            assert_eq!(a.takes[0].play_range, (0, 112_596));
            assert_eq!((a.mode, a.engine), (AudioMode::Loop, AudioEngine::Formant));
            assert_eq!(d.project().groups[g].patterns[0].audio, [AudioCue { sound: s, take: 0 }]);
        };
        look(&d);
        let path = dir.join("audio.mxprj");
        d.save_as(&path).unwrap();
        look(&Document::open(&path).unwrap());
        // Off in the pattern, then on again: one undo step each.
        d.set_audio_in(g, s, &[0], None, "Audio Off").unwrap();
        assert!(d.project().groups[g].patterns[0].audio.is_empty());
        d.undo();
        look(&d);
        // A recording more: a second take, in focus.
        let spec = TakeSpec { file: file.clone(), frames: 56_298, rate: 44_100, beats: 2.0 };
        assert_eq!(d.add_take(g, s, &spec, "Record").unwrap(), 1);
        let a = d.project().groups[g].sounds[s].audio.clone().unwrap();
        assert_eq!((a.takes.len(), a.focused), (2, 1));
        // An edit halving the first take: half the beats, the same tempo.
        d.set_take_sample(g, s, 0, &file, 56_298, "Truncate").unwrap();
        let t = d.project().groups[g].sounds[s].audio.clone().unwrap().takes[0].clone();
        assert!((t.beats - 2.0).abs() < 0.01, "{}", t.beats);
        assert!((t.tempo(56_298, 44_100) - 94.0).abs() < 0.05);
    }

    /// A sound made an Audio plug-in playing a take in a pattern, in a new project: saved
    /// and opened again, it's all there; undo takes the pattern's audio back.
    #[test]
    fn a_sound_becomes_an_audio_plugin_playing_a_take() {
        check(Document::new_project().unwrap());
    }

    /// The same in a project Maschine saved (when there's one to read).
    #[test]
    fn the_same_in_a_maschine_project() {
        let src = std::env::var_os("DECKARD_TEST_PROJECT").map_or_else(
            || std::path::PathBuf::from("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj"),
            std::path::PathBuf::from,
        );
        let Ok(d) = Document::open(&src) else { return };
        check(d);
    }

    /// Bounce: the sound becomes a lone Audio plug-in with a take per pattern, each
    /// pattern playing its own; its notes leave those patterns; one undo step.
    #[test]
    fn bounce_makes_a_take_per_pattern() {
        use deckard_model::{Note, PPQ};
        let mut d = Document::new_project().unwrap();
        let (g, s) = (0, 2);
        d.sampler_only(g, s).unwrap();
        while d.project().groups[g].patterns.len() < 3 {
            let n = d.project().groups[g].patterns.len();
            // (A first pattern from nothing; the others like it.)
            d.add_pattern(g, if n == 0 { usize::MAX } else { 0 }, &format!("Pattern {}", n + 1)).unwrap();
        }
        let note = |position| Note { expr: None, sound: s, position, length: PPQ, pitch: 60, velocity: 100, probability: 100 };
        let other = Note { sound: 0, ..note(0) };
        d.edit_notes("Notes", g, 0, &[], &[note(0), other]).unwrap();
        d.edit_notes("Notes", g, 2, &[], &[note(PPQ)]).unwrap();
        let before = d.project().clone();
        let dir = std::env::temp_dir().join(format!("deckard-bounce-{}", std::process::id()));
        let takes: Vec<(usize, TakeSpec)> = [0, 2]
            .into_iter()
            .map(|pi| (pi, TakeSpec { file: dir.join(format!("b{pi}.wav")), frames: 88_200, rate: 44_100, beats: 4.0 }))
            .collect();
        d.bounce(g, s, &takes).unwrap();
        let p = d.project();
        let snd = &p.groups[g].sounds[s];
        assert_eq!(snd.instrument, "AudioModule");
        assert_eq!(snd.slots.len(), 1);
        assert_eq!(snd.audio.as_ref().unwrap().takes.len(), 2);
        assert_eq!(p.groups[g].patterns[0].audio, [AudioCue { sound: s, take: 0 }]);
        assert!(p.groups[g].patterns[1].audio.is_empty());
        assert_eq!(p.groups[g].patterns[2].audio, [AudioCue { sound: s, take: 1 }]);
        assert_eq!(p.groups[g].patterns[0].notes, [other], "only the bounced sound's notes go");
        assert!(p.groups[g].patterns[2].notes.is_empty());
        assert_eq!(d.undo_labels().first().copied(), Some("Bounce"));
        d.undo();
        assert_eq!(d.project().groups[g], before.groups[g], "one undo step");
    }
}
