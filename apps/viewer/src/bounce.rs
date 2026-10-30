//! Bouncing a sound in place (MK3 manual, "Bouncing Sounds in place"; the Sound menu
//! and PAD MODE's SHIFT + BOUNCE, Ideas view only): after asking, each pattern playing
//! the sound is rendered (the sound alone, before its fader) and the sound becomes an
//! Audio plug-in with those renders as takes, each pattern playing its own. And the
//! pattern editor's audio (MK3 manual, "Exporting audio from Patterns"): a pattern
//! rendered to a file, or into the group's first empty sound (Sample to Sound).

use crate::App;
use crate::export::Offline;
use deckard_doc::TakeSpec;
use deckard_engine::SampleCache;
use deckard_engine::export::{self as ex, BitDepth, FileType, Format, Job, Range, Stem};
use deckard_model::{PPQ, Project};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// The confirmation's answers (Maschine's buttons).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Allow {
    No,
    Once,
    /// Once, and don't ask again (for the session).
    Always,
}

/// Bounce's state in the app.
#[derive(Default)]
pub struct Bounce {
    /// Asking whether to bounce (group, sound).
    pub confirm: Option<(usize, usize)>,
    /// Always Allow was answered.
    pub always: bool,
    /// The bounce rendering: (group, sound) and the job (each pattern's take).
    pub running: Option<((usize, usize), Offline<Vec<(usize, TakeSpec)>>)>,
    /// A pattern's audio rendering, and the group it goes into (Sample to Sound).
    pub pattern: Option<(Option<(usize, usize)>, Offline<Vec<PathBuf>>)>,
}

/// The rate bounces render at when no audio output is open.
const RATE: u32 = 44_100;

/// Render each of `patterns` of sound `s` into `dir` (a WAV each, as long as the
/// pattern, its tail wrapped round so it loops), as takes.
fn render_takes(
    p: &Project,
    path: Option<&Path>,
    (g, s): (usize, usize),
    patterns: &[usize],
    dir: &Path,
    rate: u32,
    report: &dyn Fn(f64),
    cancel: &AtomicBool,
) -> Result<Vec<(usize, TakeSpec)>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let q = ex::bounce_project(p, g, s);
    let fmt = Format { file_type: FileType::Wav, rate, depth: BitDepth::Float32, normalize: false, loop_optimize: true };
    let mut cache = SampleCache::default();
    let mut takes: Vec<(usize, TakeSpec)> = Vec::new();
    let mut written: Vec<PathBuf> = Vec::new();
    let n = patterns.len().max(1) as f64;
    for (k, &pi) in patterns.iter().enumerate() {
        let (q, mode, offset) = ex::playing(&q, Range::Pattern(g, pi));
        let Some(r) = ex::render(&q, path, mode, offset, &fmt, &mut cache, cancel, &mut |f| report((k as f64 + f) / n)) else {
            return Err("cancelled".into());
        };
        let file = ex::unique_path(dir, &ex::pattern_file_stem(p, g, pi, Some(s)), "wav", &written);
        let bytes = ex::encode(&r.audio, r.rate, fmt.file_type, fmt.depth, None);
        std::fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))?;
        written.push(file.clone());
        takes.push((pi, TakeSpec { file, frames: r.frames(), rate: r.rate, beats: r.ticks as f64 / PPQ as f64 }));
    }
    Ok(takes)
}

impl App {
    /// The rate offline renders for the project use: the audio output's, else 44.1 kHz.
    fn render_rate(&self) -> u32 {
        match &self.output {
            Some(Ok(o)) => o.rate,
            _ => RATE,
        }
    }

    /// Bounce (the Sound menu, the MK3): in the Ideas view only; asks first unless
    /// Always Allow was answered.
    pub fn request_bounce(&mut self, g: usize, s: usize) {
        if self.song_view {
            self.status = "Bounce works in the Ideas view".into();
            return;
        }
        if self.bounce.running.is_some() {
            return;
        }
        if self.bounce.always {
            self.start_bounce(g, s);
        } else {
            self.bounce.confirm = Some((g, s));
        }
    }

    /// The confirmation answered.
    pub fn answer_bounce(&mut self, a: Allow) {
        let Some((g, s)) = self.bounce.confirm.take() else { return };
        if a == Allow::Always {
            self.bounce.always = true;
        }
        if a != Allow::No {
            self.start_bounce(g, s);
        }
    }

    fn start_bounce(&mut self, g: usize, s: usize) {
        let Some(doc) = &self.doc else { return };
        let p = doc.borrow().project().clone();
        let patterns = ex::bounce_patterns(&p, g, s);
        if patterns.is_empty() {
            self.status = "no pattern plays this sound: nothing to bounce".into();
            return;
        }
        let (path, dir, rate) = (self.path.clone(), self.sample_dir("Bounces"), self.render_rate());
        let job = Offline::start("Bounce", move |report, cancel| {
            render_takes(&p, path.as_deref(), (g, s), &patterns, &dir, rate, report, cancel)
        });
        self.bounce.running = Some(((g, s), job));
    }

    /// A bounce's end: the edit (one undo step), or why not.
    pub fn poll_bounce(&mut self) {
        if let Some(r) = self.bounce.running.as_ref().and_then(|(_, j)| j.poll()) {
            let ((g, s), _) = self.bounce.running.take().unwrap();
            self.finish_bounce(g, s, r);
        }
        if let Some(r) = self.bounce.pattern.as_ref().and_then(|(_, j)| j.poll()) {
            let (into, _) = self.bounce.pattern.take().unwrap();
            self.finish_pattern_audio(into, r);
        }
    }

    fn finish_bounce(&mut self, g: usize, s: usize, r: Result<Vec<(usize, TakeSpec)>, String>) {
        let Some(doc) = self.doc.clone() else { return };
        let done = r.and_then(|takes| doc.borrow_mut().bounce(g, s, &takes).map(|()| takes.len()));
        self.status = match done {
            Ok(n) => format!("bounced sound {} into {n} take{}", s + 1, if n == 1 { "" } else { "s" }),
            Err(e) => format!("can't bounce: {e}"),
        };
    }

    /// Wait for the running bounce or pattern render and take its result (tests).
    #[cfg(test)]
    pub fn finish_offline_now(&mut self) {
        if let Some(((g, s), job)) = self.bounce.running.take() {
            let r = job.wait();
            self.finish_bounce(g, s, r);
        }
        if let Some((into, job)) = self.bounce.pattern.take() {
            let r = job.wait();
            self.finish_pattern_audio(into, r);
        }
    }

    /// The pattern editor's audio: pattern `pi` of group `g` (the group, or in
    /// Keyboard view the focused sound) rendered with the Export Audio panel's options
    /// as a WAV, named as Maschine names it: into the panel's folder, or (`to_sound`)
    /// next to the project and loaded into the group's first empty sound.
    pub fn pattern_audio(&mut self, g: usize, pi: usize, to_sound: bool) {
        let Some(doc) = &self.doc else { return };
        let p = doc.borrow().project().clone();
        let sound = (self.piano_roll && g == self.group).then_some(self.sound);
        let stem = sound.map_or(Stem::Group(g), |s| Stem::Sound(g, s));
        let into = if to_sound {
            match (0..p.groups.get(g).map_or(0, |x| x.sounds.len())).find(|&k| ex::sound_is_empty(&p.groups[g].sounds[k])) {
                Some(k) => Some(k),
                None => {
                    self.status = "the group has no empty sound slot".into();
                    return;
                }
            }
        } else {
            None
        };
        let job = Job { range: Range::Pattern(g, pi), stem, name: String::new(), file: Some(ex::pattern_file_stem(&p, g, pi, sound)) };
        let fmt = Format { file_type: FileType::Wav, ..self.export.settings.format };
        let dir = if to_sound { self.sample_dir("Bounces") } else { self.export_folder() };
        let path = self.path.clone();
        let run = Offline::start("Pattern audio", move |report, cancel| {
            ex::export(&p, path.as_deref(), &[job], &fmt, &dir, cancel, &mut |_, f| report(f))
        });
        self.bounce.pattern = Some((into.map(|k| (g, k)), run));
    }

    fn finish_pattern_audio(&mut self, into: Option<(usize, usize)>, r: Result<Vec<PathBuf>, String>) {
        match (r, into) {
            (Ok(files), Some(at)) if !files.is_empty() => self.drop_samples(at, &files),
            (Ok(files), _) if !files.is_empty() => self.status = format!("exported {}", files[0].display()),
            (Ok(_), _) => self.status = "pattern audio cancelled".into(),
            (Err(e), _) => self.status = format!("can't render the pattern: {e}"),
        }
    }

    /// Bounce's confirmation (Allow Once, Always Allow, Don't Allow) and its progress
    /// (Cancel); a pattern render's progress.
    pub fn bounce_windows(&mut self, ctx: &egui::Context) {
        if self.bounce.confirm.is_some() {
            let mut answer = None;
            egui::Window::new("Bounce").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
                ctx,
                |ui| {
                    ui.label(
                        "Bouncing renders the sound's patterns to audio and replaces its plug-ins with an Audio plug-in playing them.",
                    );
                    ui.label(egui::RichText::new("You can undo it while this project is open.").weak());
                    ui.horizontal(|ui| {
                        for (a, label) in [(Allow::No, "Don't Allow"), (Allow::Once, "Allow Once"), (Allow::Always, "Always Allow")] {
                            if ui.button(label).clicked() {
                                answer = Some(a);
                            }
                        }
                    });
                },
            );
            if let Some(a) = answer {
                self.answer_bounce(a);
            }
        }
        let progress = self
            .bounce
            .running
            .as_ref()
            .map(|(_, j)| (j.what.clone(), j.progress()))
            .or_else(|| self.bounce.pattern.as_ref().map(|(_, j)| (j.what.clone(), j.progress())));
        if let Some((what, f)) = progress {
            let mut cancel = false;
            egui::Window::new(format!("{what}…")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
                ctx,
                |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::ProgressBar::new(f).desired_width(220.0).show_percentage());
                        cancel = ui.button("Cancel").clicked();
                    });
                },
            );
            if cancel {
                if let Some((_, j)) = &self.bounce.running {
                    j.cancel();
                }
                if let Some((_, j)) = &self.bounce.pattern {
                    j.cancel();
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::Note;

    /// A new project whose sound 1 has notes in patterns 1 and 3 (of three).
    fn app() -> App {
        let mut app = App::default();
        app.new_project();
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        d.sampler_only(0, 0).unwrap();
        while d.project().groups[0].patterns.len() < 3 {
            let n = d.project().groups[0].patterns.len();
            d.add_pattern(0, if n == 0 { usize::MAX } else { 0 }, &format!("Pattern {}", n + 1)).unwrap();
        }
        let note = Note { expr: None, sound: 0, position: 0, length: PPQ, pitch: 60, velocity: 100, probability: 100 };
        d.edit_notes("Notes", 0, 0, &[], &[note]).unwrap();
        d.edit_notes("Notes", 0, 2, &[], &[note]).unwrap();
        drop(d);
        app.song_view = false;
        app
    }

    /// The Sound menu's Bounce asks, then renders each pattern playing the sound into
    /// a take of an Audio plug-in; one undo step brings the sound back.
    #[test]
    fn bounce_asks_then_makes_takes() {
        let mut app = app();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("song");
        app.path = Some(dir.join("Song.mxprj"));
        crate::layout::apply_acts(&mut app, vec![crate::layout::Act::Bounce(0, 0)]);
        assert_eq!(app.bounce.confirm, Some((0, 0)), "it asks first");
        app.answer_bounce(Allow::Once);
        app.finish_offline_now();
        let doc = app.doc.clone().unwrap();
        let p = doc.borrow().project().clone();
        let snd = &p.groups[0].sounds[0];
        assert_eq!(snd.instrument, "AudioModule", "{}", app.status);
        let takes = &snd.audio.as_ref().unwrap().takes;
        assert_eq!(takes.len(), 2);
        assert!(p.groups[0].patterns[0].notes.is_empty());
        // Each take is its pattern long (a bar at 120 BPM), beside the project.
        let file = dir.join("Bounces").join("Group A1 - Pattern 1 Sound 1 - 120 BPM.wav");
        let s = deckard_engine::sample::Sample::load(&file).unwrap();
        assert_eq!(s.frames.len(), 88_200);
        assert_eq!(takes[0].beats, 4.0);
        doc.borrow_mut().undo();
        assert_eq!(doc.borrow().project().groups[0].sounds[0].instrument, "Sampler");
    }

    /// Don't Allow does nothing; Always Allow stops asking; the Song view doesn't bounce.
    #[test]
    fn the_confirmation() {
        let mut app = app();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("song");
        app.path = Some(dir.join("Song.mxprj"));
        app.request_bounce(0, 0);
        app.answer_bounce(Allow::No);
        assert!(app.bounce.running.is_none() && app.bounce.confirm.is_none());
        app.always_allow_without_rendering();
        app.request_bounce(0, 0);
        assert!(app.bounce.confirm.is_none() && app.bounce.running.is_some(), "Always Allow: no question");
        let (_, job) = app.bounce.running.take().unwrap();
        job.cancel();
        // Stopped before its folder goes.
        let _ = job.wait();
        app.song_view = true;
        app.request_bounce(0, 0);
        assert!(app.bounce.confirm.is_none() && app.bounce.running.is_none());
    }

    /// The MK3: PAD MODE, SHIFT + button 6 (BOUNCE) asks the app to bounce the focused
    /// sound.
    #[test]
    fn the_mk3_bounces_the_focused_sound() {
        let Some(mut h) = crate::tests::harness(true, false) else { return };
        let hw = |h: &mut egui_kittest::Harness<'static, App>, button: &str, down: bool| {
            if let Some(Ok(c)) = &mut h.state_mut().controller {
                let _ = c.host.button(button, down);
            }
            h.run_steps(4);
        };
        for (b, down) in [
            ("BUTTON_PAD_MODE", true),
            ("BUTTON_PAD_MODE", false),
            ("BUTTON_SHIFT", true),
            ("BUTTON_SCREEN_6", true),
            ("BUTTON_SCREEN_6", false),
            ("BUTTON_SHIFT", false),
        ] {
            hw(&mut h, b, down);
        }
        let (g, s) = (h.state().group, h.state().sound);
        assert_eq!(h.state().bounce.confirm, Some((g, s)));
        h.state_mut().answer_bounce(Allow::No);
    }

    impl App {
        fn always_allow_without_rendering(&mut self) {
            self.bounce.always = true;
        }
    }

    /// The pattern's audio: a file named as Maschine names it in the export folder; or
    /// into the group's first empty sound as a loop.
    #[test]
    fn pattern_audio_to_a_file_or_a_sound() {
        let mut app = app();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("song");
        app.path = Some(dir.join("Song.mxprj"));
        app.export.settings.folder = Some(dir.join("Out"));
        app.pattern_audio(0, 0, false);
        app.finish_offline_now();
        assert!(dir.join("Out").join("Group A1 - Pattern 1 - 120 BPM.wav").is_file(), "{}", app.status);
        app.piano_roll = true;
        app.pattern_audio(0, 2, true);
        app.finish_offline_now();
        assert!(dir.join("Bounces").join("Group A1 - Pattern 3 Sound 1 - 120 BPM.wav").is_file(), "{}", app.status);
        let doc = app.doc.clone().unwrap();
        let snd = doc.borrow().project().groups[0].sounds[1].clone();
        assert_eq!(snd.instrument, "AudioModule", "sound 2 was the first empty one: {}", app.status);
    }
}
