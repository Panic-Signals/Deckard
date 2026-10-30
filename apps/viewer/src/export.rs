//! File ▸ Export Audio… (MK3 manual, "Exporting audio"): the Export Song as Audio panel
//! (Song view) or Export Ideas as Audio (Ideas view), its Source, Options and
//! Destination, and the export running in the background. The rendering is
//! `deckard_engine::export`; this is the panel, the files it plans and the job.

use crate::App;
use deckard_engine::export::{self as ex, BitDepth, FileType, Format, Job, Range, Stem};
use deckard_model::Project;
use eframe::egui;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};

/// Whose output the files hold (the panel's Source menu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    Master,
    Group,
    Sound,
}

/// The panel's choices (kept for the session).
#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// Song view: Range ▸ Loop (the picked section) rather than All.
    pub range_loop: bool,
    /// Ideas view: Scenes ▸ Selected (the focused scene) rather than All.
    pub scenes_selected: bool,
    pub source: Source,
    /// Groups and sounds unchecked in the Output list (all are checked at first).
    pub groups_off: HashSet<usize>,
    pub sounds_off: HashSet<(usize, usize)>,
    pub split_by_section: bool,
    pub format: Format,
    /// The destination folder and file name (none: the project's folder and name).
    pub folder: Option<PathBuf>,
    pub name: Option<String>,
}

/// The panel, and the export it runs.
#[derive(Default)]
pub struct ExportPanel {
    pub open: bool,
    pub settings: Settings,
    pub running: Option<Offline<Vec<PathBuf>>>,
}

/// What the app shows of where the song is: the view, the picked section and the
/// focused scene.
#[derive(Debug, Clone, Copy)]
pub struct Where {
    pub song_view: bool,
    pub section: Option<usize>,
    pub scene: usize,
}

fn scene_name(p: &Project, s: usize) -> String {
    let n = p.scenes.get(s).map(|x| x.name.clone()).unwrap_or_default();
    if n.is_empty() { format!("Scene {}", s + 1) } else { n }
}

/// The files an export writes, in order: each part of the range (a scene, a section)
/// for each checked output.
pub fn plan(p: &Project, at: Where, s: &Settings, name: &str) -> Vec<Job> {
    let mut parts: Vec<(Range, String)> = Vec::new();
    if at.song_view {
        let (a, b) = match at.section.filter(|&i| s.range_loop && i < p.sections.len()) {
            Some(i) => (i, i),
            None => (0, p.sections.len().saturating_sub(1)),
        };
        if s.split_by_section && !p.sections.is_empty() {
            for i in a..=b {
                let scene = p.sections[i].scene.map(|x| scene_name(p, x)).unwrap_or_default();
                parts.push((Range::Sections(i, i), format!("{name} - {:02} {scene}", i + 1).trim_end().to_string()));
            }
        } else if p.sections.is_empty() || (a, b) == (0, p.sections.len() - 1) {
            parts.push((Range::Song, name.to_string()));
        } else {
            parts.push((Range::Sections(a, b), name.to_string()));
        }
    } else {
        let scenes: Vec<usize> =
            if s.scenes_selected { vec![at.scene.min(p.scenes.len().saturating_sub(1))] } else { (0..p.scenes.len()).collect() };
        for sc in scenes.into_iter().filter(|&sc| sc < p.scenes.len()) {
            parts.push((Range::Scene(sc), format!("{name} - {}", scene_name(p, sc))));
        }
    }
    let stems = stems(p, s);
    parts.iter().flat_map(|(range, name)| stems.iter().map(|&stem| Job { range: *range, stem, name: name.clone(), file: None })).collect()
}

/// The outputs checked in the panel's list.
pub fn stems(p: &Project, s: &Settings) -> Vec<Stem> {
    match s.source {
        Source::Master => vec![Stem::Master],
        Source::Group => ex::exportable_groups(p).into_iter().filter(|g| !s.groups_off.contains(g)).map(Stem::Group).collect(),
        Source::Sound => ex::exportable_groups(p)
            .into_iter()
            .flat_map(|g| ex::exportable_sounds(p, g).into_iter().map(move |k| (g, k)))
            .filter(|gs| !s.sounds_off.contains(gs))
            .map(|(g, k)| Stem::Sound(g, k))
            .collect(),
    }
}

/// Work done off the UI thread (an export, a bounce): its progress, cancelling it, and
/// its result.
pub struct Offline<T> {
    pub what: String,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    done: mpsc::Receiver<Result<T, String>>,
}

impl<T: Send + 'static> Offline<T> {
    /// Run `work` on a thread of its own; it reports progress (0..1) and sees the
    /// cancel flag.
    pub fn start(what: &str, work: impl FnOnce(&dyn Fn(f64), &AtomicBool) -> Result<T, String> + Send + 'static) -> Offline<T> {
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, done) = mpsc::channel();
        let (p, c) = (progress.clone(), cancel.clone());
        std::thread::Builder::new()
            .name(what.to_lowercase())
            .spawn(move || {
                let report = |f: f64| p.store((f as f32).to_bits(), Ordering::Relaxed);
                let _ = tx.send(work(&report, &c));
            })
            .expect("a thread for offline rendering");
        Offline { what: what.into(), progress, cancel, done }
    }

    pub fn progress(&self) -> f32 {
        f32::from_bits(self.progress.load(Ordering::Relaxed))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The result, once there is one.
    pub fn poll(&self) -> Option<Result<T, String>> {
        match self.done.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(format!("{} stopped", self.what))),
        }
    }

    /// Wait for the result (tests).
    #[cfg(test)]
    pub fn wait(&self) -> Result<T, String> {
        self.done.recv().unwrap_or_else(|_| Err(format!("{} stopped", self.what)))
    }
}

/// Export `jobs` of `p` into `dir` in the background.
pub fn start(p: Project, path: Option<PathBuf>, jobs: Vec<Job>, format: Format, dir: PathBuf) -> Offline<Vec<PathBuf>> {
    Offline::start("Export", move |report, cancel| {
        let n = jobs.len().max(1) as f64;
        ex::export(&p, path.as_deref(), &jobs, &format, &dir, cancel, &mut |k, f| report((k as f64 + f) / n))
    })
}

impl App {
    /// The project's name for files (its file's name, else its own).
    pub fn project_name(&self) -> String {
        let from_file = self.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned());
        let own = self.doc.as_ref().map(|d| d.borrow().project().name.clone()).filter(|n| !n.is_empty());
        from_file.or(own).unwrap_or_else(|| "Untitled".into())
    }

    /// Where exports go unless the panel says: the project's folder, else DECKARD's in
    /// the user's Music folder.
    pub fn export_folder(&self) -> PathBuf {
        if let Some(f) = &self.export.settings.folder {
            return f.clone();
        }
        match self.path.as_ref().and_then(|p| p.parent()) {
            Some(dir) => dir.to_path_buf(),
            None => std::env::home_dir().unwrap_or_default().join("Music").join("DECKARD"),
        }
    }

    fn export_where(&self) -> Where {
        Where { song_view: self.song_view, section: self.section, scene: self.scene }
    }

    /// Start exporting what the panel says (into its folder).
    pub fn start_export(&mut self) {
        let Some(doc) = &self.doc else { return };
        let p = doc.borrow().project().clone();
        let name = self.export.settings.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| self.project_name());
        let jobs = plan(&p, self.export_where(), &self.export.settings, &name);
        if jobs.is_empty() {
            self.status = "nothing to export".into();
            return;
        }
        let dir = self.export_folder();
        self.export.running = Some(start(p, self.path.clone(), jobs, self.export.settings.format, dir));
    }

    /// An export's end: the status line says what was written.
    pub fn poll_export(&mut self) {
        let Some(r) = self.export.running.as_ref().and_then(Offline::poll) else { return };
        let cancelled = self.export.running.as_ref().is_some_and(Offline::cancelled);
        self.export.running = None;
        self.status = match r {
            Ok(files) if cancelled => format!("export cancelled ({} files written)", files.len()),
            Ok(files) if files.len() == 1 => format!("exported {}", files[0].display()),
            Ok(files) => {
                format!("exported {} files to {}", files.len(), files.first().and_then(|f| f.parent()).unwrap_or(Path::new("")).display())
            }
            Err(e) => format!("can't export: {e}"),
        };
    }

    /// File ▸ Export MIDI… (DECKARD's; Maschine exports patterns only): what the song
    /// plays (the focused scene in the Ideas view) as a type 1 file, a track per loaded
    /// sound (named `A1 Group - Sound`), each on its MIDI output channel, transposed.
    /// `to`: the file (else asked).
    pub fn export_song_midi(&mut self, to: Option<PathBuf>) -> Result<(), String> {
        let Some(doc) = &self.doc else { return Ok(()) };
        let p = doc.borrow().project().clone();
        let mode = if self.song_view { deckard_engine::Mode::Arrangement } else { deckard_engine::Mode::Scene(self.scene) };
        let path = match to {
            Some(path) => path,
            None => {
                let name = format!("{}.mid", ex::safe_name(&self.project_name()));
                let Some(path) = rfd::FileDialog::new().add_filter("MIDI", &["mid"]).set_file_name(name).save_file() else { return Ok(()) };
                path
            }
        };
        let t = deckard_engine::timeline(&p, mode);
        let mut tracks = Vec::new();
        for g in ex::exportable_groups(&p) {
            for s in ex::exportable_sounds(&p, g) {
                let snd = &p.groups[g].sounds[s];
                let mine = t.events.iter().filter(|e| (e.group, e.sound) == (g, s) && e.cue.is_none());
                let mut track = crate::midi_file::Track {
                    name: format!(
                        "{} - {}",
                        group_label(&p, g),
                        if snd.name.is_empty() { format!("Sound {}", s + 1) } else { snd.name.clone() }
                    ),
                    channel: snd.channel.midi_out_channel.clamp(0, 15) as u8,
                    transpose: snd.channel.transpose,
                    ..Default::default()
                };
                for e in mine {
                    match e.midi {
                        None => track.notes.push(deckard_model::Note {
                            expr: None,
                            sound: s,
                            position: e.tick,
                            length: e.length,
                            pitch: e.pitch,
                            velocity: e.velocity,
                            probability: e.probability,
                        }),
                        Some(kind) => track.midi.push(deckard_model::MidiEvent { sound: s, position: e.tick, kind }),
                    }
                }
                tracks.push(track);
            }
        }
        crate::midi_file::export_tracks(&path, &tracks, p.tempo, p.numerator, p.denominator)?;
        self.status = format!("exported {}", path.display());
        Ok(())
    }

    /// The Export Song as Audio / Export Ideas as Audio panel.
    pub fn export_window(&mut self, ctx: &egui::Context) {
        let Some(doc) = self.doc.clone() else { return };
        let p = doc.borrow().project().clone();
        let title = if self.song_view { "Export Song as Audio" } else { "Export Ideas as Audio" };
        let mut open = self.export.open;
        let mut export = false;
        let mut close = false;
        let folder = self.export_folder();
        let default_name = self.project_name();
        let running = self.export.running.as_ref().map(|r| r.progress());
        let song_view = self.song_view;
        let s = &mut self.export.settings;
        egui::Window::new(title).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.add_enabled_ui(running.is_none(), |ui| {
                ui.heading("Source");
                egui::Grid::new("export-source").num_columns(2).show(ui, |ui| {
                    if song_view {
                        ui.label("Range");
                        combo(ui, "export-range", &mut s.range_loop, &[(false, "All"), (true, "Loop")]);
                    } else {
                        ui.label("Scenes");
                        combo(ui, "export-scenes", &mut s.scenes_selected, &[(false, "All"), (true, "Selected")]);
                    }
                    ui.end_row();
                    ui.label("Source");
                    combo(
                        ui,
                        "export-source",
                        &mut s.source,
                        &[(Source::Master, "Master"), (Source::Group, "Group"), (Source::Sound, "Sound")],
                    );
                    ui.end_row();
                });
                output_list(ui, &p, s);
                ui.separator();
                ui.heading("Options");
                egui::Grid::new("export-options").num_columns(2).show(ui, |ui| {
                    ui.checkbox(&mut s.format.normalize, "Normalize");
                    ui.checkbox(&mut s.format.loop_optimize, "Loop Optimize");
                    ui.end_row();
                    if song_view {
                        ui.checkbox(&mut s.split_by_section, "Split By Section");
                        ui.end_row();
                    }
                    ui.label("Type");
                    combo(ui, "export-type", &mut s.format.file_type, &[(FileType::Wav, "WAV"), (FileType::Aiff, "AIFF")]);
                    ui.end_row();
                    ui.label("Sample Rate");
                    let rates: Vec<(u32, String)> = ex::RATES.iter().map(|&r| (r, format!("{r} Hz"))).collect();
                    let rates: Vec<(u32, &str)> = rates.iter().map(|(r, t)| (*r, t.as_str())).collect();
                    combo(ui, "export-rate", &mut s.format.rate, &rates);
                    ui.end_row();
                    ui.label("Bit Depth");
                    combo(
                        ui,
                        "export-depth",
                        &mut s.format.depth,
                        &[(BitDepth::Int16, "16 bit"), (BitDepth::Int24, "24 bit"), (BitDepth::Float32, "32 bit float")],
                    );
                    ui.end_row();
                });
                ui.separator();
                ui.heading("Destination");
                egui::Grid::new("export-dest").num_columns(2).show(ui, |ui| {
                    ui.label("Folder");
                    if ui.button(folder.display().to_string()).on_hover_text("Choose the folder").clicked()
                        && let Some(dir) = rfd::FileDialog::new().set_directory(&folder).pick_folder()
                    {
                        s.folder = Some(dir);
                    }
                    ui.end_row();
                    ui.label("Name");
                    let mut name = s.name.clone().unwrap_or(default_name);
                    if ui.text_edit_singleline(&mut name).changed() {
                        s.name = Some(name);
                    }
                    ui.end_row();
                });
            });
            ui.separator();
            match running {
                Some(f) => {
                    ui.horizontal(|ui| {
                        ui.add(egui::ProgressBar::new(f).desired_width(220.0).show_percentage());
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                None => {
                    ui.horizontal(|ui| {
                        if ui.button("Export").clicked() {
                            export = true;
                        }
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                }
            }
        });
        if close {
            match &self.export.running {
                Some(r) => r.cancel(),
                None => open = false,
            }
        }
        self.export.open = open;
        if export {
            self.start_export();
        }
        if self.export.running.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

/// A menu of `choices` for `v`.
fn combo<T: PartialEq + Copy>(ui: &mut egui::Ui, id: &str, v: &mut T, choices: &[(T, &str)]) {
    let shown = choices.iter().find(|(c, _)| c == v).map_or("", |(_, t)| *t);
    egui::ComboBox::from_id_salt(id).selected_text(shown).show_ui(ui, |ui| {
        for (c, t) in choices {
            ui.selectable_value(v, *c, *t);
        }
    });
}

/// The Output list: Master Output (checked, greyed), the groups, or the groups' sounds
/// as a tree (a group's box checks all its sounds).
fn output_list(ui: &mut egui::Ui, p: &Project, s: &mut Settings) {
    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| match s.source {
        Source::Master => {
            ui.add_enabled(false, egui::Checkbox::new(&mut true, "Master Output"));
        }
        Source::Group => {
            for g in ex::exportable_groups(p) {
                let mut on = !s.groups_off.contains(&g);
                if ui.checkbox(&mut on, group_label(p, g)).changed() {
                    toggle(&mut s.groups_off, g, on);
                }
            }
        }
        Source::Sound => {
            for g in ex::exportable_groups(p) {
                let sounds = ex::exportable_sounds(p, g);
                let checked = sounds.iter().filter(|&&k| !s.sounds_off.contains(&(g, k))).count();
                ui.horizontal(|ui| {
                    let mut all = checked == sounds.len();
                    let r = ui.add(egui::Checkbox::new(&mut all, "").indeterminate(checked > 0 && checked < sounds.len()));
                    if r.changed() {
                        for &k in &sounds {
                            toggle(&mut s.sounds_off, (g, k), all);
                        }
                    }
                    egui::CollapsingHeader::new(group_label(p, g)).id_salt(("export-group", g)).default_open(false).show(ui, |ui| {
                        for &k in &sounds {
                            let mut on = !s.sounds_off.contains(&(g, k));
                            let name = &p.groups[g].sounds[k].name;
                            let name = if name.is_empty() { format!("Sound {}", k + 1) } else { name.clone() };
                            if ui.checkbox(&mut on, name).changed() {
                                toggle(&mut s.sounds_off, (g, k), on);
                            }
                        }
                    });
                });
            }
        }
    });
}

fn group_label(p: &Project, g: usize) -> String {
    let n = &p.groups[g].name;
    format!("{} {}", ex::group_index(g), if n.is_empty() { "Group" } else { n })
}

fn toggle<T: std::hash::Hash + Eq>(off: &mut HashSet<T>, k: T, on: bool) {
    if on {
        off.remove(&k);
    } else {
        off.insert(k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::{Group, PPQ, Scene, Section, Sound};

    fn loaded() -> Sound {
        Sound { instrument: "Sampler".into(), sampler: Some(Default::default()), ..Default::default() }
    }

    fn project() -> Project {
        let group = |name: &str, n: usize| Group {
            name: name.into(),
            sounds: (0..n).map(|_| loaded()).chain([Sound::default()]).collect(),
            ..Default::default()
        };
        Project {
            name: "Song".into(),
            tempo: 120.0,
            numerator: 4,
            denominator: 4,
            groups: vec![group("Drums", 2), Group { sounds: vec![Sound::default()], ..Default::default() }, group("Keys", 1)],
            scenes: vec![Scene { name: "Intro".into(), ..Default::default() }, Scene::default()],
            sections: vec![
                Section { scene: Some(0), length: 4 * PPQ, auto: false },
                Section { scene: Some(1), length: 4 * PPQ, auto: false },
                Section { scene: None, length: 4 * PPQ, auto: false },
            ],
            ..Default::default()
        }
    }

    fn song(section: Option<usize>) -> Where {
        Where { song_view: true, section, scene: 0 }
    }

    fn parts(jobs: &[Job]) -> Vec<(Range, Stem, &str)> {
        jobs.iter().map(|j| (j.range, j.stem, j.name.as_str())).collect()
    }

    #[test]
    fn the_song_or_its_loop_to_the_master() {
        let p = project();
        let mut s = Settings::default();
        assert_eq!(parts(&plan(&p, song(Some(1)), &s, "Song")), [(Range::Song, Stem::Master, "Song")]);
        s.range_loop = true;
        assert_eq!(parts(&plan(&p, song(Some(1)), &s, "Song")), [(Range::Sections(1, 1), Stem::Master, "Song")]);
        assert_eq!(parts(&plan(&p, song(None), &s, "Song")), [(Range::Song, Stem::Master, "Song")], "no section picked: all");
    }

    #[test]
    fn split_by_section_gives_a_file_per_section() {
        let p = project();
        let s = Settings { split_by_section: true, ..Default::default() };
        assert_eq!(
            parts(&plan(&p, song(None), &s, "Song")),
            [
                (Range::Sections(0, 0), Stem::Master, "Song - 01 Intro"),
                (Range::Sections(1, 1), Stem::Master, "Song - 02 Scene 2"),
                (Range::Sections(2, 2), Stem::Master, "Song - 03"),
            ]
        );
    }

    #[test]
    fn ideas_export_every_scene_or_the_selected_one() {
        let p = project();
        let ideas = Where { song_view: false, section: None, scene: 1 };
        let mut s = Settings::default();
        assert_eq!(
            parts(&plan(&p, ideas, &s, "Song")),
            [(Range::Scene(0), Stem::Master, "Song - Intro"), (Range::Scene(1), Stem::Master, "Song - Scene 2")]
        );
        s.scenes_selected = true;
        assert_eq!(parts(&plan(&p, ideas, &s, "Song")), [(Range::Scene(1), Stem::Master, "Song - Scene 2")]);
    }

    #[test]
    fn stems_are_the_checked_groups_or_sounds() {
        let p = project();
        let mut s = Settings { source: Source::Group, ..Default::default() };
        assert_eq!(stems(&p, &s), [Stem::Group(0), Stem::Group(2)], "the empty group isn't listed");
        s.groups_off.insert(2);
        assert_eq!(stems(&p, &s), [Stem::Group(0)]);
        s.source = Source::Sound;
        s.sounds_off.insert((0, 1));
        assert_eq!(stems(&p, &s), [Stem::Sound(0, 0), Stem::Sound(2, 0)], "empty slots aren't listed");
        assert_eq!(plan(&p, song(None), &s, "Song").len(), 2);
    }

    /// From the app: File ▸ Export Audio's job writes the files into the panel's folder,
    /// in the background.
    #[test]
    fn the_app_exports_into_the_folder() {
        let mut app = App::default();
        app.new_project();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("export");
        app.export.settings.folder = Some(dir.clone());
        app.export.settings.name = Some("Take".into());
        app.export.settings.format.rate = 48_000;
        app.start_export();
        let files = app.export.running.as_ref().expect("running").wait().unwrap();
        let names: Vec<String> = files.iter().map(|f| f.file_name().unwrap().to_string_lossy().into()).collect();
        assert!(!names.is_empty() && names.iter().all(|n| n.starts_with("Take") && n.ends_with(" - 120 BPM.wav")), "{names:?}");
        let s = deckard_engine::sample::Sample::load(&files[0]).unwrap();
        assert_eq!(s.rate, 48_000);
        assert!(!s.frames.is_empty());
    }

    /// File ▸ Export MIDI…: what the song (or, in the Ideas view, the focused scene)
    /// plays, a track per loaded sound, the patterns repeated as they play.
    #[test]
    fn the_song_as_midi() {
        use deckard_model::{Note, PPQ};
        let mut app = App::default();
        app.new_project();
        let doc = app.doc.clone().unwrap();
        {
            let mut d = doc.borrow_mut();
            d.sampler_only(0, 2).unwrap();
            if d.project().groups[0].patterns.is_empty() {
                d.add_pattern(0, usize::MAX, "Pattern 1").unwrap();
            }
            let note = Note { expr: None, sound: 2, position: PPQ, length: PPQ, pitch: 64, velocity: 100, probability: 100 };
            d.edit_notes("Notes", 0, 0, &[], &[note]).unwrap();
            d.set_pattern_length(0, 0, 4 * PPQ).unwrap();
            d.assign_pattern(0, 0, Some(0)).unwrap();
        }
        app.song_view = false;
        app.focus_scene(0);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("song.mid");
        app.export_song_midi(Some(path.clone())).unwrap();
        let (notes, _, end) = crate::midi_file::import_channels(&path).unwrap();
        assert_eq!(notes.iter().map(|(_, n)| (n.position, n.pitch)).collect::<Vec<_>>(), [(PPQ, 64)], "{}", app.status);
        assert!(end >= 2 * PPQ);
    }
}
