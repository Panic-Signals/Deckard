//! The New Sampler's analysis: each zone's tempo and root note, worked out on a thread
//! of its own and kept in the settings folder's `analysis` cache by the audio's content
//! (docs/superpowers/plans/2026-11-12-new-sampler-metadata.md).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use deckard_analysis::{Analysis, Cache, TempoSource, analyse};
use deckard_engine::sample::Sample;
use deckard_model::new_sampler::{BpmSource, PlayMode, ZoneMeta};
use sha2::{Digest, Sha256};

/// A zone's sample to analyse: its path as the zone saves it, its file name, its audio.
struct Ask {
    file: String,
    name: String,
    sample: Arc<Sample>,
}

/// An analysis done: the path it was asked for.
pub struct Done {
    pub file: String,
    pub analysis: Analysis,
}

/// The analysis thread, started on the first ask.
pub struct Jobs {
    cache: Option<PathBuf>,
    send: Option<Sender<Ask>>,
    done: Option<Receiver<Done>>,
    /// Every path asked for: each is analysed once a session.
    asked: HashSet<String>,
}

impl Jobs {
    /// Jobs keeping their results in `cache` (none: kept nowhere, as in tests).
    pub fn new(cache: Option<PathBuf>) -> Jobs {
        Jobs { cache, send: None, done: None, asked: HashSet::new() }
    }

    /// Analyse the sample a zone saves as `file` (once).
    pub fn want(&mut self, file: &str, name: &str, sample: Arc<Sample>) {
        if !self.asked.insert(file.to_string()) {
            return;
        }
        if self.send.is_none() {
            let (send, asks) = channel::<Ask>();
            let (tell, done) = channel();
            let cache = self.cache.clone().map(Cache::new);
            let started = std::thread::Builder::new().name("analysis".into()).spawn(move || {
                for ask in asks {
                    let key = content_key(&ask.sample);
                    let analysis = match cache.as_ref().and_then(|c| c.get(&key)) {
                        Some(a) => a,
                        None => {
                            let a = analyse(&ask.sample.frames, ask.sample.rate, &ask.name);
                            if let Some(c) = &cache {
                                let _ = c.put(&key, &a);
                            }
                            a
                        }
                    };
                    if tell.send(Done { file: ask.file, analysis }).is_err() {
                        return;
                    }
                }
            });
            if started.is_err() {
                return;
            }
            (self.send, self.done) = (Some(send), Some(done));
        }
        if let Some(s) = &self.send {
            let _ = s.send(Ask { file: file.to_string(), name: name.to_string(), sample });
        }
    }

    /// The analyses done since the last call.
    pub fn poll(&mut self) -> Vec<Done> {
        self.done.as_ref().map_or_else(Vec::new, |d| d.try_iter().collect())
    }
}

impl Default for Jobs {
    /// Jobs keeping their results in the settings folder's `analysis`.
    fn default() -> Jobs {
        Jobs::new(crate::menu::settings_dir().map(|d| d.join("analysis")))
    }
}

/// The key of a sample's audio: the SHA-256 of its rate and frames.
pub fn content_key(s: &Sample) -> String {
    let mut h = Sha256::new();
    h.update(s.rate.to_le_bytes());
    for f in &s.frames {
        h.update(f[0].to_le_bytes());
        h.update(f[1].to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// A zone's meta from its analysis. The play mode follows Serato's Auto rule: Sync for a
/// sample with a tempo, unless it's under 3 s, or under 10 s with "808" in its name.
pub fn meta_from(a: &Analysis, file: &str, name: &str) -> ZoneMeta {
    let one_shot = a.seconds < 3.0 || (a.seconds < 10.0 && name.contains("808"));
    ZoneMeta {
        file: file.to_string(),
        mode: if a.tempo.is_some() && !one_shot { PlayMode::Sync } else { PlayMode::Repitch },
        bpm: a.tempo.map(|t| t.bpm),
        bpm_source: match a.tempo.map(|t| t.source) {
            Some(TempoSource::Hint) => BpmSource::Name,
            Some(TempoSource::Loop) => BpmSource::Loop,
            _ => BpmSource::Audio,
        },
        beats: a.tempo.map(|t| t.beats),
        grid_offset: a.tempo.map_or(0, |t| t.offset as u64),
        root: a.root.map(|r| r.note),
        cents: a.root.map_or(0.0, |r| r.cents),
        root_locked: false,
    }
}

/// The file name at the end of a zone's saved path (either slash).
pub fn file_name_of(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// A note's name in DECKARD's naming (60 is C3).
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[note as usize % 12], i32::from(note) / 12 - 2)
}

/// The Sampling page's line for a New Sampler zone: "120.00 BPM · 16 beats · Root A1
/// −12 ct · SYNC", or "Analysing…".
pub fn readout(meta: Option<&ZoneMeta>) -> String {
    let Some(m) = meta else { return "Analysing…".into() };
    let mut parts = Vec::new();
    match (m.bpm, m.beats) {
        (Some(b), Some(n)) => parts.push(format!("{b:.2} BPM · {} beats", (n * 100.0).round() / 100.0)),
        (Some(b), None) => parts.push(format!("{b:.2} BPM")),
        _ => parts.push("No tempo".into()),
    }
    match m.root {
        Some(r) if m.cents.abs() >= 1.0 => parts.push(format!("Root {} {:+.0} ct", note_name(r), m.cents).replace('-', "−")),
        Some(r) => parts.push(format!("Root {}", note_name(r))),
        None => parts.push("No root".into()),
    }
    parts.push(match m.mode {
        PlayMode::Repitch => "REPITCH".into(),
        PlayMode::Sync => "SYNC".into(),
        PlayMode::Keys => "KEYS".into(),
    });
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_analysis::{Root, Tempo};

    fn tempo(bpm: f64, seconds: f64) -> Option<Tempo> {
        Some(Tempo { bpm, beats: seconds * bpm / 60.0, offset: 0, confidence: 1.0, source: TempoSource::Loop, candidates: [0.0; 2] })
    }

    #[test]
    fn the_auto_rule() {
        let loop4 = Analysis { tempo: tempo(120.0, 4.0), root: None, seconds: 4.0 };
        assert_eq!(meta_from(&loop4, "a", "Loop.wav").mode, PlayMode::Sync);
        let short = Analysis { tempo: tempo(120.0, 2.0), root: None, seconds: 2.0 };
        assert_eq!(meta_from(&short, "a", "Hit.wav").mode, PlayMode::Repitch);
        let eight = Analysis { tempo: tempo(120.0, 8.0), root: None, seconds: 8.0 };
        assert_eq!(meta_from(&eight, "a", "808 Long.wav").mode, PlayMode::Repitch);
        let pad = Analysis { tempo: None, root: Some(Root { note: 57, cents: 0.0, confidence: 1.0 }), seconds: 6.0 };
        let m = meta_from(&pad, "a", "Pad.wav");
        assert_eq!((m.mode, m.root, m.bpm), (PlayMode::Repitch, Some(57), None));
    }

    #[test]
    fn the_readout() {
        let a = Analysis { tempo: tempo(120.0, 8.0), root: Some(Root { note: 45, cents: -12.4, confidence: 1.0 }), seconds: 8.0 };
        assert_eq!(readout(Some(&meta_from(&a, "f", "x.wav"))), "120.00 BPM · 16 beats · Root A1 −12 ct · SYNC");
        assert_eq!(readout(None), "Analysing…");
    }

    #[test]
    fn jobs_analyse_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut jobs = Jobs::new(Some(dir.path().to_path_buf()));
        let sample = Arc::new(Sample { rate: 44_100, frames: vec![[0.0; 2]; 44_100] });
        jobs.want("/a.wav", "a.wav", sample.clone());
        jobs.want("/a.wav", "a.wav", sample.clone());
        let mut done = Vec::new();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while done.is_empty() && std::time::Instant::now() < until {
            done.extend(jobs.poll());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        done.extend(jobs.poll());
        assert_eq!(done.len(), 1);
        assert_eq!((done[0].file.as_str(), done[0].analysis.tempo), ("/a.wav", None));
        // Kept by content.
        assert!(Cache::new(dir.path()).get(&content_key(&sample)).is_some());
    }
}

#[cfg(test)]
mod app_tests {
    use crate::App;
    use deckard_model::new_sampler::PlayMode;

    /// A New Sampler loaded with a 120 BPM loop: analysed on its own, and the zone shows
    /// the tempo with no undo step for it.
    #[test]
    fn a_new_sampler_learns_its_loop() {
        let dir = tempfile::tempdir().unwrap();
        // Four bars of clicks at 120 BPM.
        let rate = 44_100u32;
        let beat = rate as usize / 2;
        let frames: Vec<[f32; 2]> =
            (0..beat * 16).map(|i| if i % beat < 200 { [0.8 * (1.0 - (i % beat) as f32 / 200.0); 2] } else { [0.0; 2] }).collect();
        let file = dir.path().join("Clicks.wav");
        deckard_engine::edit::write_wav_channels(&file, &deckard_engine::sample::Sample { rate, frames }, 2).unwrap();

        let mut doc = deckard_doc::Document::new_project().unwrap();
        if doc.project().groups[0].sounds[0].slots.is_empty() {
            doc.add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
        }
        let spec = deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Template, file: Some(file.clone()), set: Vec::new() };
        doc.set_zones(0, 0, &[spec], "Load").unwrap();
        doc.set_new_sampler(0, 0, Some(&Default::default())).unwrap();
        let steps = doc.undo_labels().len();
        let mut app = App { analysis: super::Jobs::new(None), ..App::default() };
        app.doc = Some(std::rc::Rc::new(std::cell::RefCell::new(doc)));

        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let meta = loop {
            app.follow_analysis();
            let d = app.doc.as_ref().unwrap().borrow();
            if let Some(m) = d.project().groups[0].sounds[0].sampler.as_ref().unwrap().zones[0].meta.clone() {
                break m;
            }
            assert!(std::time::Instant::now() < until, "no analysis");
            drop(d);
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!((meta.bpm.unwrap() - 120.0).abs() < 0.05, "{meta:?}");
        assert_eq!(meta.mode, PlayMode::Sync);
        assert_eq!(app.doc.as_ref().unwrap().borrow().undo_labels().len(), steps);
    }
}
