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

/// The key of a sample's audio (kept by it in the analysis cache).
pub fn content_key(s: &Sample) -> String {
    Cache::audio_key(&s.frames, s.rate)
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
        warps: Vec::new(),
        start: Default::default(),
        stems: Default::default(),
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
/// −12 ct", or "Analysing…" (the play mode shows on its buttons).
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
        assert_eq!(readout(Some(&meta_from(&a, "f", "x.wav"))), "120.00 BPM · 16 beats · Root A1 −12 ct");
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

        // In Sync, playback asks for its render at the song's tempo; made on the render
        // thread, it goes to the samples and playback is built again to play it.
        let build = |app: &mut App| {
            let d = app.doc.as_ref().unwrap().clone();
            let mut p = d.borrow().project().clone();
            p.tempo = 100.0;
            drop(deckard_engine::Engine::new(&p, None, 48_000, deckard_engine::Mode::Arrangement, &mut app.samples));
        };
        build(&mut app);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.seen_revision != u64::MAX {
            app.follow_renders();
            assert!(std::time::Instant::now() < until, "no render");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.built.is_none(), "built again in full");
        build(&mut app);
        assert!(app.samples.wanted_renders().is_empty(), "it plays the render");
    }

    /// An MP3 dropped on a sound: an Audio plug-in's take on an empty sound, a zone in a
    /// Sampler and in a New Sampler, and it sounds when its key is played.
    #[test]
    fn an_mp3_dropped_on_each_kind_of_sound() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Tone.MP3");
        std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/engine/testdata/tone.mp3"), &file).unwrap();
        assert!(crate::is_sample_file(&file));
        assert_eq!(deckard_library::Kind::of_file(&file), Some(deckard_library::Kind::OneShot));

        let mut doc = deckard_doc::Document::new_project().unwrap();
        for s in [1, 2] {
            if doc.project().groups[0].sounds[s].sampler.is_none() {
                doc.add_slot(0, s, 0, &deckard_doc::NewModule::Sampler).unwrap();
            }
        }
        doc.set_new_sampler(0, 2, Some(&Default::default())).unwrap();
        // Sound 4 is left with nothing on it.
        let empty = (0..16).find(|&s| doc.project().groups[0].sounds[s].slots.is_empty()).expect("an empty sound");
        let mut app = App { analysis: super::Jobs::new(None), ..App::default() };
        app.doc = Some(std::rc::Rc::new(std::cell::RefCell::new(doc)));
        for s in [empty, 1, 2] {
            app.drop_samples((0, s), std::slice::from_ref(&file));
            assert!(!app.status.contains("can't"), "{}", app.status);
        }
        let p = app.doc.as_ref().unwrap().borrow().project().clone();
        let sounds = &p.groups[0].sounds;
        let take = &sounds[empty].audio.as_ref().expect("an Audio plug-in").takes[0];
        assert!(take.sample.path.ends_with("Tone.MP3"), "{:?}", take.sample);
        for s in [1, 2] {
            let z = sounds[s].sampler.as_ref().unwrap().zones.last().unwrap();
            assert!(z.sample.path.ends_with("Tone.MP3"), "{:?}", z.sample);
            assert!((z.play_range.1 - 44_099).abs() <= 2, "{:?}", z.play_range);
            let key = z.root_key;
            let (mut e, _) = deckard_engine::Engine::new(&p, None, 44_100, deckard_engine::Mode::Arrangement, &mut app.samples);
            e.note_on(0, s, key, 127, 960);
            let mut out = vec![0.0f32; 2 * 8192];
            e.process(&mut out);
            assert!(out.iter().any(|v| v.abs() > 0.01), "sound {s} is silent");
        }
    }

    /// MARKERS puts warp markers on a loop's hits, QUANTIZE moves late ones onto the
    /// grid (one undo step each), CLEAR takes them away.
    #[test]
    fn warp_markers_on_the_hits_quantized() {
        use crate::layout::sampling::WarpAct;
        let dir = tempfile::tempdir().unwrap();
        let rate = 48_000u32;
        // Clicks on the beats at 120, the third 30 ms late.
        let mut frames = vec![[0.0f32; 2]; rate as usize * 2];
        for (k, at) in [0usize, 24_000, 48_000 + 1_440, 72_000].into_iter().enumerate() {
            for i in 0..400 {
                frames[at + i] = [0.9 * (1.0 - i as f32 / 400.0) * if k % 2 == 0 { 1.0 } else { -1.0 }; 2];
            }
        }
        let file = dir.path().join("Hits.wav");
        let sample = deckard_engine::sample::Sample { rate, frames };
        deckard_engine::edit::write_wav_channels(&file, &sample, 2).unwrap();
        let mut doc = deckard_doc::Document::new_project().unwrap();
        if doc.project().groups[0].sounds[0].slots.is_empty() {
            doc.add_slot(0, 0, 0, &deckard_doc::NewModule::Sampler).unwrap();
        }
        let spec = deckard_doc::ZoneSpec { from: deckard_doc::ZoneFrom::Template, file: Some(file.clone()), set: Vec::new() };
        doc.set_zones(0, 0, &[spec], "Load").unwrap();
        doc.set_new_sampler(0, 0, Some(&Default::default())).unwrap();
        let path = doc.project().groups[0].sounds[0].sampler.as_ref().unwrap().zones[0].sample.path.clone();
        use deckard_model::new_sampler::ZoneMeta;
        let meta = ZoneMeta { file: path, mode: PlayMode::Sync, bpm: Some(120.0), beats: Some(4.0), ..ZoneMeta::default() };
        doc.note_zone_meta(0, 0, 0, &meta).unwrap();
        let mut app = App { analysis: super::Jobs::new(None), ..App::default() };
        app.doc = Some(std::rc::Rc::new(std::cell::RefCell::new(doc)));
        app.sampling.sample = Some(std::sync::Arc::new(sample));
        let warps = |app: &App| {
            app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].sampler.as_ref().unwrap().zones[0].meta.clone().unwrap().warps
        };
        app.warp_act(WarpAct::Detect);
        let w = warps(&app);
        assert_eq!(w.len(), 4, "{w:?}");
        assert!((w[2].beat - 2.06).abs() < 0.02, "the late hit is late: {w:?}");
        app.warp_act(WarpAct::Quantize);
        let w = warps(&app);
        assert!(w.iter().enumerate().all(|(k, x)| (x.beat - k as f64).abs() < 1e-9), "on the grid: {w:?}");
        assert!(w[2].frame > 49_000, "the marker stays on its hit");
        assert_eq!(app.doc.as_ref().unwrap().borrow().undo_label(), Some("Quantize Audio"));
        app.warp_act(WarpAct::Clear);
        assert!(warps(&app).is_empty());
        // STEMS on a New Sampler zone: the stems go on its lanes (Drums, Vocals, Bass,
        // Leads), all on; M mutes one; one undo step each.
        app.path = Some(dir.path().join("x.mxprj"));
        let stems = deckard_stems::Kind::ALL.iter().map(|&kind| deckard_stems::Stem { kind, frames: vec![[0.1; 2]; 96_000] }).collect();
        app.finish_stem_lanes("Hits", rate, stems, (0, 0, 0));
        let lanes = |app: &App| {
            app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].sampler.as_ref().unwrap().zones[0].meta.clone().unwrap().stems
        };
        let l = lanes(&app);
        let names: Vec<String> =
            l.iter().map(|x| x.as_ref().unwrap().file.rsplit('/').next().unwrap().trim_end_matches(".wav").to_string()).collect();
        assert_eq!(names, ["Hits Drums", "Hits Vocal", "Hits Bass", "Hits Other"], "{}", app.status);
        assert!(l.iter().flatten().all(|x| !x.mute && x.gain == 1.0 && std::path::Path::new(&x.file).exists()));
        assert_eq!(app.doc.as_ref().unwrap().borrow().undo_label(), Some("Stems"));
        use crate::layout::sampling::LaneAct;
        app.lane_act(1, LaneAct::Mute);
        app.lane_act(3, LaneAct::Gain(0.5));
        let l = lanes(&app);
        assert_eq!(deckard_model::new_sampler::lane_levels(&l), [1.0, 0.0, 1.0, 0.5]);
        assert_eq!(app.doc.as_ref().unwrap().borrow().undo_label(), Some("Stem Gain"));
        app.lane_act(0, LaneAct::Clear);
        assert!(lanes(&app).iter().all(Option::is_none));
        app.doc.as_ref().unwrap().borrow_mut().undo();
        assert!(lanes(&app).iter().all(Option::is_some), "back with one undo");
        // Stems of your own: by name onto their lanes; a short one padded, a long one
        // and one at another rate left out.
        app.lane_act(0, LaneAct::Clear);
        let own = |name: &str, rate: u32, n: usize| {
            let f = dir.path().join(name);
            deckard_engine::edit::write_wav_channels(&f, &deckard_engine::sample::Sample { rate, frames: vec![[0.2; 2]; n] }, 2).unwrap();
            f
        };
        app.load_stems(&[
            own("My Bass.wav", rate, 96_000),
            own("My Vox.wav", rate, 1000),
            own("Long.wav", rate, 96_001),
            own("Slow.wav", 44_100, 100),
        ]);
        let l = lanes(&app);
        assert!(l[0].is_none() && l[3].is_none(), "{l:?}");
        assert!(l[2].as_ref().unwrap().file.ends_with("My Bass.wav"));
        let padded = deckard_engine::sample::Sample::load(std::path::Path::new(&l[1].as_ref().unwrap().file)).unwrap();
        assert_eq!(padded.frames.len(), 96_000, "padded to the sample's length");
        assert!(app.status.contains("Long.wav") && app.status.contains("Slow.wav"), "{}", app.status);
        app.doc.as_ref().unwrap().borrow_mut().undo();
        app.doc.as_ref().unwrap().borrow_mut().undo();
        assert!(lanes(&app).iter().all(Option::is_some), "the separated stems again");
        // SLICE's Grid follows the sample's own beats (120 here), not the song's (90).
        app.doc.as_ref().unwrap().borrow_mut().set_tempo(90.0).unwrap();
        (app.sampling.slice_mode, app.sampling.grid) = (1, 0);
        app.compute_slices();
        assert_eq!(app.sampling.slices, [0, 24_000, 48_000, 72_000]);
        // The controller's MATCH KEY: a root of A1, 12 cents flat, in a song in C
        // plays at C2 (up 3, its cents back).
        let zone = |app: &App| app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].sampler.as_ref().unwrap().zones[0].clone();
        let m = ZoneMeta { root: Some(45), cents: -12.0, ..zone(&app).meta.unwrap() };
        app.sampling_act(crate::layout::sampling::SamplingAct::Meta(m));
        app.scale.0 = 60;
        app.sampling_call(deckard_lua_host::state::SamplingCall::MatchKey);
        assert_eq!(zone(&app).tune, 3.12);
        // Its START knob, on the SYNC page.
        use deckard_model::new_sampler::StartGrid;
        assert_eq!(zone(&app).meta.unwrap().start, StartGrid::Sixteenth);
        app.record_apply.insert("Beat.Start".into(), crate::sampling_hw::Apply::StartGrid);
        app.sampling_knob("Beat.Start", 3.0);
        assert_eq!(zone(&app).meta.unwrap().start, StartGrid::Quarter);
        // Slices to the next slice (a Sampler's way): each its own piece, one undo step.
        let zones = |app: &App| app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].sampler.as_ref().unwrap().zones.clone();
        let envelope = |app: &App| app.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].sampler.as_ref().unwrap().envelope_type;
        let ranges = |app: &App| zones(app).iter().map(|z| z.play_range).collect::<Vec<_>>();
        let last = 95_999;
        app.sampling.slices = vec![0, 24_000, 48_000, 72_000];
        app.sampling.pattern_mode = 2;
        app.sampling_act(crate::layout::sampling::SamplingAct::SliceToEnd(false));
        app.apply_slices();
        assert_eq!(ranges(&app), [(0, 23_999), (24_000, 47_999), (48_000, 71_999), (72_000, last)], "{}", app.status);
        assert_eq!(envelope(&app), 0, "still a one-shot");
        app.doc.as_ref().unwrap().borrow_mut().undo();
        assert_eq!(zones(&app).len(), 1);
        // A New Sampler's own way: start points, each playing on to the sample's end
        // while its pad is held (the notes end it: not a one-shot any more).
        app.sampling.slice_to_end = None;
        app.zone = 0;
        app.sampling.slices = vec![0, 24_000, 48_000, 72_000];
        app.apply_slices();
        assert_eq!(ranges(&app), [(0, last), (24_000, last), (48_000, last), (72_000, last)], "{}", app.status);
        assert_eq!(envelope(&app), 2);
        app.doc.as_ref().unwrap().borrow_mut().undo();
        assert_eq!((zones(&app).len(), envelope(&app)), (1, 0), "one undo step");
        app.doc.as_ref().unwrap().borrow_mut().redo();
        // Slices keep the stems, each with mutes of its own.
        assert_eq!(zones(&app).len(), 4, "{}", app.status);
        assert!(zones(&app).iter().all(|z| z.meta.as_ref().is_some_and(|m| m.has_stems())), "each slice has the lanes");
        app.zone = 1;
        app.lane_act(0, LaneAct::Mute);
        let muted: Vec<bool> = zones(&app).iter().map(|z| z.meta.as_ref().unwrap().stems[0].as_ref().unwrap().mute).collect();
        assert_eq!(muted, [false, true, false, false]);
    }

    /// A groove on DECKARD's own drum synths: (kick, snare, hat) steps of 16 in a bar,
    /// `bars` long at `bpm`, the off sixteenths `swing` late (0.5: straight).
    fn groove(steps: (&[usize], &[usize], &[usize]), bpm: f64, bars: usize, swing: f64, rate: u32) -> Vec<[f32; 2]> {
        use deckard_engine::sampler::{Context, SamplerRt};
        let step = f64::from(rate) * 60.0 / bpm / 4.0;
        let len = (step * 16.0 * bars as f64).round() as usize;
        let mut mix = vec![0f32; len * 2];
        for (class, at, velocity) in [("Kick", steps.0, 120), ("Snare", steps.1, 105), ("Hihat", steps.2, 80)] {
            let mut synth = SamplerRt::synth(class, &[], (1.0, 1.0), f64::from(rate)).expect(class);
            let mut hits: Vec<usize> = (0..bars)
                .flat_map(|b| at.iter().map(move |&k| (b, k)))
                .map(|(b, k)| ((b * 16 + k) as f64 * step + if k % 2 == 1 { (swing - 0.5) * 2.0 * step } else { 0.0 }).round() as usize)
                .collect();
            hits.push(len);
            let mut out = vec![0f32; len * 2];
            let mut from = 0;
            for (n, &to) in hits.iter().enumerate() {
                synth.render(&mut out[from * 2..to * 2]);
                if to < len {
                    synth.note_on(n as u64 + 1, 60, velocity, &Context { tempo: bpm, beats: 0.0, live: false });
                }
                from = to;
            }
            for (m, o) in mix.iter_mut().zip(&out) {
                *m += o * 0.5;
            }
        }
        mix.chunks(2).map(|c| [c[0], c[1]]).collect()
    }

    /// The tempo corpus (roadmap 1a): grooves played by DECKARD's drum synths at known
    /// tempos. A loop cut on its bars: exact, or exactly twice or half (the fold into
    /// 85–170). The same with a second of silence after it (not a loop): within 1 %.
    #[test]
    fn the_tempo_of_grooves_on_our_own_drums() {
        let rate = 44_100u32;
        let grooves: [(&str, (&[usize], &[usize], &[usize]), f64); 6] = [
            ("four on the floor", (&[0, 4, 8, 12], &[4, 12], &[2, 6, 10, 14]), 0.5),
            ("boom bap", (&[0, 7, 10], &[4, 12], &[0, 2, 4, 6, 8, 10, 12, 14]), 0.58),
            ("break", (&[0, 10], &[4, 7, 9, 12], &[0, 2, 4, 6, 8, 10, 12, 14]), 0.5),
            ("half time", (&[0, 11], &[8], &[0, 2, 4, 6, 8, 10, 12, 14]), 0.5),
            ("two step", (&[0, 10], &[4, 12], &[2, 3, 6, 10, 11, 14]), 0.5),
            ("sixteenths", (&[0, 6, 8], &[4, 12], &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]), 0.5),
        ];
        let mut wrong = Vec::new();
        for (name, steps, swing) in grooves {
            for bpm in [72.0, 86.0, 94.0, 103.0, 120.0, 128.0, 140.0, 160.0, 174.0] {
                for bars in [2, 4] {
                    let mut audio = groove(steps, bpm, bars, swing, rate);
                    let near = |got: f64, by: f64| [bpm, bpm * 2.0, bpm / 2.0].iter().any(|b| (got - b).abs() <= b * by);
                    let looped = deckard_analysis::tempo(&audio, rate, None);
                    if !looped.is_some_and(|t| near(t.bpm, 0.0005)) {
                        wrong.push((name, bpm, bars, "loop", looped.map(|t| t.bpm)));
                    }
                    audio.extend(std::iter::repeat_n([0.0; 2], rate as usize));
                    let free = deckard_analysis::tempo(&audio, rate, None);
                    if !free.is_some_and(|t| near(t.bpm, 0.01)) {
                        wrong.push((name, bpm, bars, "free", free.map(|t| t.bpm)));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{} wrong: {wrong:?}", wrong.len());
    }

    /// An Audio plug-in's take becomes a New Sampler zone in Sync at the take's tempo;
    /// converting back leaves a plain Sampler.
    #[test]
    fn an_audio_plugin_converts_and_back() {
        let dir = tempfile::tempdir().unwrap();
        let rate = 48_000u32;
        let frames = vec![[0.1f32; 2]; rate as usize * 2];
        let file = dir.path().join("Loop.wav");
        deckard_engine::edit::write_wav_channels(&file, &deckard_engine::sample::Sample { rate, frames }, 2).unwrap();
        let mut doc = deckard_doc::Document::new_project().unwrap();
        doc.audio_only(0, 0).unwrap();
        // Four beats in two seconds: 120 BPM.
        let spec = deckard_doc::TakeSpec { file: file.clone(), frames: rate as usize * 2, rate, beats: 4.0 };
        doc.set_takes(0, 0, &[spec], "Load").unwrap();
        let mut app = App { analysis: super::Jobs::new(None), ..App::default() };
        app.doc = Some(std::rc::Rc::new(std::cell::RefCell::new(doc)));
        app.convert_instrument(true);
        {
            let d = app.doc.as_ref().unwrap().borrow();
            let snd = &d.project().groups[0].sounds[0];
            assert!(snd.new_sampler.is_some(), "{}", app.status);
            let m = snd.sampler.as_ref().unwrap().zones[0].meta.clone().unwrap();
            assert_eq!(m.mode, PlayMode::Sync);
            assert!((m.bpm.unwrap() - 120.0).abs() < 0.1, "{m:?}");
            assert_eq!(d.undo_label(), Some("Convert to New Sampler"));
        }
        app.convert_instrument(false);
        let d = app.doc.as_ref().unwrap().borrow();
        assert!(d.project().groups[0].sounds[0].new_sampler.is_none());
        assert!(app.status.contains("Sync"), "says what it dropped: {}", app.status);
    }
}
