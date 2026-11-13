//! Startup as a queue of steps. Each step's line is shown (running) before the step
//! runs, so a slow step is visible while it works; the outcome then replaces it.

use crate::{App, StemsOffer, jam_io, layout, menu, mk3_io, recorder, settings, stems, update, user_chords};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long the boot screen stays up at least.
const MIN_SHOWN: Duration = Duration::from_secs(1);

/// One piece of startup work.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Settings,
    AudioDriver,
    UpdateCheck,
    Mk3,
    Launchpad,
    Surface,
    Catalog,
    /// Open this project, or start a new one.
    Project(Option<PathBuf>),
    Plugins,
    AudioOutput,
    /// Load the n-th wanted plug-in.
    Plugin(usize),
    Done,
}

/// How a step went.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Ok,
    Text(String),
    Err(String),
    Skipped,
}

/// A line of the boot log.
pub struct LogLine {
    pub label: String,
    /// `None` while the step runs.
    pub outcome: Option<Outcome>,
}

pub struct Boot {
    pub log: Vec<LogLine>,
    pub done: usize,
    pub total: usize,
    pub started: Instant,
    /// When READY was first shown.
    pub ready_at: Option<Instant>,
    /// The last file dropped while loading, opened after.
    pub held: Option<PathBuf>,
    /// The splash window was closed by hand.
    pub hidden: bool,
    queue: VecDeque<Step>,
    /// The step whose line is showing as running.
    running: Option<Step>,
    // (Filled by the plug-in steps.)
    wanted: Vec<(deckard_plugins::Slot, deckard_model::PluginRef)>,
    failed: Vec<String>,
    /// The audio output's rate, once it is open.
    rate: f64,
    /// When the plug-ins began to load.
    plugins_from: Option<Instant>,
}

impl Boot {
    pub fn new(steps: Vec<Step>) -> Boot {
        Boot {
            log: Vec::new(),
            done: 0,
            total: steps.len(),
            started: Instant::now(),
            ready_at: None,
            held: None,
            hidden: false,
            queue: steps.into(),
            running: None,
            wanted: Vec::new(),
            failed: Vec::new(),
            rate: 0.0,
            plugins_from: None,
        }
    }

    /// How much of the work is done, 0 to 1.
    pub fn progress(&self) -> f32 {
        if self.total == 0 { 1.0 } else { (self.done as f32 / self.total as f32).clamp(0.0, 1.0) }
    }

    /// Whether the last step (READY) has run: the count says so, or nothing is left
    /// to run (so an over-count can't keep the screen up).
    pub fn finished(&self) -> bool {
        self.done >= self.total || (self.queue.is_empty() && self.running.is_none())
    }

    /// Whether the screen may go: everything ran, READY was on screen for a frame, and
    /// it has been up for at least a second.
    pub fn may_close(&self, now: Instant) -> bool {
        self.finished() && self.ready_at.is_some_and(|r| now > r) && now.saturating_duration_since(self.started) >= MIN_SHOWN
    }
}

/// The startup steps in order.
pub fn all_steps(path: Option<PathBuf>) -> Vec<Step> {
    vec![
        Step::Settings,
        Step::AudioDriver,
        Step::UpdateCheck,
        Step::Mk3,
        Step::Launchpad,
        Step::Surface,
        Step::Catalog,
        Step::Project(path),
        Step::Plugins,
        Step::Done,
    ]
}

/// Whether this launch shows the splash: only a plain one, with no arguments or one
/// project path (any `--` flag is a scripted run).
pub fn splash_wanted(args: &[String]) -> bool {
    args.len() <= 1 && !args.iter().any(|a| a.starts_with("--"))
}

/// `OPENING SONG.MXPRJ`.
fn opening(p: &std::path::Path) -> String {
    format!("OPENING {}", p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().to_uppercase()))
}

/// The Project line for how the project was (or will be) started.
fn project_label(start: &crate::Start) -> String {
    match start {
        crate::Start::Backup => "RESTORING BACKUP".into(),
        crate::Start::Reopen(p) => opening(p),
        crate::Start::New => "NEW PROJECT".into(),
    }
}

fn found(yes: bool) -> Outcome {
    Outcome::Text(if yes { "FOUND" } else { "NOT CONNECTED" }.into())
}

impl App {
    /// Advance startup by one half-step: run the step whose line is showing, or else
    /// show the next step's line (so it is drawn before it runs).
    pub(crate) fn boot_next(&mut self) {
        let Some(mut boot) = self.boot.take() else { return };
        if boot.log.last().is_some_and(|l| l.outcome.is_none()) {
            let step = boot.running.take();
            // (A step may add lines of its own after its).
            let at = boot.log.len() - 1;
            let outcome = step.map_or(Outcome::Ok, |s| self.run_step(&mut boot, s));
            if let Some(line) = boot.log.get_mut(at) {
                line.outcome = Some(outcome);
            }
            boot.done += 1;
        } else if let Some(step) = boot.queue.pop_front() {
            boot.log.push(LogLine { label: self.step_label(&boot, &step), outcome: None });
            boot.running = Some(step);
        }
        self.boot = Some(boot);
    }

    /// Run startup to the end at once (finished also when the queue runs dry before
    /// the count says so, so a miscount can't hang it).
    pub(crate) fn boot_to_end(&mut self) {
        while self.boot.as_ref().is_some_and(|b| !b.finished()) {
            self.boot_next();
        }
    }

    /// One frame of the main window while starting up: input is drained and dropped
    /// files held, the next step runs, and the window is a plain backdrop. Ends startup
    /// (and opens a held file) when the screen may go.
    pub(crate) fn boot_frame(&mut self, ui: &mut eframe::egui::Ui) {
        let ctx = ui.ctx().clone();
        // Closing the window with unsaved changes (a restored backup) asks first, once
        // loading ends, as it does after.
        if ctx.input(|i| i.viewport().close_requested()) && self.asks_on_quit() && !self.quitting {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::CancelClose);
            self.confirm = Some(crate::Pending::Quit);
        }
        // Pad hits while loading are never played later.
        if let Some(m) = &self.mk3 {
            m.set_context(&ctx);
            m.take_events();
            m.take_pending();
        }
        if let Some(m) = &self.jam {
            m.set_context(&ctx);
            m.take_events();
            m.take_pending();
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).rfind(|p| !p.as_os_str().is_empty()));
        if let (Some(p), Some(b)) = (dropped, &mut self.boot) {
            b.held = Some(p);
        }
        self.boot_next();
        ui.painter().rect_filled(ui.max_rect(), 0.0, layout::S00);
        if let Some(b) = self.boot.as_mut() {
            crate::splash::show(&ctx, b);
        }
        if self.boot.as_ref().is_some_and(|b| b.may_close(Instant::now())) {
            let held = self.boot.take().and_then(|b| b.held);
            if let Some(p) = held {
                self.open_file(p);
            }
        }
        // (Also on the frame that ends it, so the UI is drawn without waiting for input.)
        ctx.request_repaint();
    }

    /// Startup while the main window is hidden (eframe draws no frames then): go on
    /// with it, a step at a time, drawing nothing. (Shown, `boot_frame` runs it.)
    pub(crate) fn boot_hidden(&mut self, ctx: &eframe::egui::Context) {
        if self.boot.is_none() {
            return;
        }
        if ctx.input(|i| i.viewport().visible()) == Some(false) {
            self.boot_next();
        }
        ctx.request_repaint();
    }

    fn step_label(&self, boot: &Boot, step: &Step) -> String {
        match step {
            Step::Settings => "READING SETTINGS".into(),
            Step::AudioDriver => "AUDIO DRIVER".into(),
            Step::UpdateCheck => "CHECKING FOR UPDATES".into(),
            Step::Mk3 => "MASCHINE MK3".into(),
            Step::Launchpad => "LAUNCHPAD PRO MK3".into(),
            Step::Surface => "CONTROLLER SURFACE".into(),
            Step::Catalog => "PLUG-IN CATALOG".into(),
            Step::Project(Some(p)) => opening(p),
            Step::Project(None) => project_label(&self.start_plan()),
            Step::Plugins => "PLUG-INS".into(),
            Step::AudioOutput => "AUDIO OUTPUT".into(),
            Step::Plugin(i) => {
                let name = boot.wanted.get(*i).map_or_else(String::new, |(_, p)| p.name.to_uppercase());
                format!("LOADING PLUG-IN {name} [{}/{}]", i + 1, boot.wanted.len())
            }
            Step::Done => "READY".into(),
        }
    }

    fn run_step(&mut self, boot: &mut Boot, step: Step) -> Outcome {
        match step {
            Step::Settings => {
                self.user_chords = user_chords::UserChords::load();
                self.recent = menu::load_recent();
                self.settings = settings::Settings::load();
                self.browser.routing = self.settings.load_group_routing;
                self.browser.patterns = self.settings.load_group_patterns;
                Outcome::Ok
            }
            Step::AudioDriver => {
                // (Nothing to open through ASIO before a driver is picked.)
                if self.settings.asio_device.is_some() {
                    deckard_engine::devices::set_driver(self.settings.audio_driver);
                    Outcome::Ok
                } else {
                    Outcome::Skipped
                }
            }
            Step::UpdateCheck => {
                self.update = update::Notice::start(self.settings.check_updates);
                self.after_update(crate::VERSION);
                Outcome::Ok
            }
            Step::Mk3 => {
                let mk3 = mk3_io::Mk3Io::start();
                mk3.set_display_brightness(self.settings.display_brightness);
                let outcome = found(mk3.displays_connected());
                self.mk3 = Some(mk3);
                outcome
            }
            Step::Launchpad => {
                let jam = jam_io::JamIo::start();
                let outcome = found(jam.connected());
                self.jam = Some(jam);
                outcome
            }
            Step::Surface => {
                self.surface = self.new_surface();
                self.recorder = recorder::Recorder::new();
                deckard_plugins::module::set_search_paths(self.settings.scan_dirs());
                Outcome::Ok
            }
            Step::Catalog => {
                self.catalog = settings::Catalog::load(&self.settings);
                self.session_dir = menu::settings_dir();
                // The first start without the stem separation model: offer it (not when
                // rendering a screenshot).
                if self.screenshot.is_none()
                    && !self.settings.stems_offered
                    && let Some(dir) = &self.session_dir
                    && !stems::installed(dir)
                {
                    self.stems_offer = Some(StemsOffer::Asking);
                }
                if self.catalog.scanning() {
                    Outcome::Text("SCANNING".into())
                } else {
                    Outcome::Text(format!("{} READY", self.catalog.entries.len()))
                }
            }
            Step::Project(path) => {
                let started = if let Some(p) = path {
                    self.open(&p);
                    self.remember(&p);
                    crate::Start::Reopen(p)
                } else {
                    self.start_project()
                };
                // (What was done, if not what was planned: a backup that wouldn't read.)
                if let Some(line) = boot.log.last_mut() {
                    line.label = project_label(&started);
                }
                // (The plug-ins load in the steps after, not again once the window is up.)
                self.preload_plugins = false;
                match (&self.error, &started) {
                    (Some(e), _) => Outcome::Err(e.clone()),
                    (None, crate::Start::Reopen(p)) => {
                        Outcome::Text(p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned()))
                    }
                    (None, crate::Start::Backup) => Outcome::Text("RESTORED".into()),
                    (None, crate::Start::New) => Outcome::Ok,
                }
            }
            Step::Plugins => {
                boot.wanted = self.wanted_plugins();
                let n = boot.wanted.len();
                if n == 0 {
                    return Outcome::Text("NONE".into());
                }
                // The output opens first, then each plug-in loads at its rate.
                for i in (0..n).rev() {
                    boot.queue.push_front(Step::Plugin(i));
                }
                boot.queue.push_front(Step::AudioOutput);
                boot.total += n + 1;
                boot.plugins_from = Some(Instant::now());
                Outcome::Text(format!("{n} TO LOAD"))
            }
            Step::AudioOutput => {
                #[cfg(test)]
                let opened = self.boot_output.clone();
                #[cfg(not(test))]
                let opened = None;
                let opened = opened.unwrap_or_else(|| {
                    match self.output.get_or_insert_with(|| crate::open_output(&self.settings, &mut self.output_fallback)) {
                        Ok(out) => Ok(out.rate as f64),
                        Err(e) => Err(e.clone()),
                    }
                });
                match opened {
                    Ok(rate) => {
                        boot.rate = rate;
                        Outcome::Ok
                    }
                    Err(e) => {
                        let before = boot.queue.len();
                        boot.queue.retain(|s| !matches!(s, Step::Plugin(_)));
                        boot.total -= before - boot.queue.len();
                        boot.log.push(LogLine { label: format!("PLUG-INS ({})", boot.wanted.len()), outcome: Some(Outcome::Skipped) });
                        self.status = format!("plug-ins not loaded: {e}");
                        Outcome::Err(e)
                    }
                }
            }
            Step::Plugin(i) => {
                let Some((slot, p)) = boot.wanted.get(i).cloned() else { return Outcome::Skipped };
                let result = self.load_plugin(slot, &p, boot.rate);
                if let Err(e) = &result {
                    boot.failed.push(e.clone());
                }
                if i + 1 == boot.wanted.len() {
                    let secs = boot.plugins_from.map_or(0.0, |t| t.elapsed().as_secs_f64());
                    self.status = App::plugins_loaded_status(boot.wanted.len(), secs, &boot.failed);
                }
                result.map_or_else(Outcome::Err, |()| Outcome::Ok)
            }
            Step::Done => {
                boot.ready_at = Some(Instant::now());
                Outcome::Ok
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    fn app_with_two_plugins() -> crate::App {
        let mut app = crate::App::default();
        let plugin = |name: &str| deckard_model::PluginRef { id: format!("{name}:none"), name: name.into(), ..Default::default() };
        app.boot_wanted = Some(vec![
            (deckard_plugins::Slot::Sound { group: 0, sound: 0 }, plugin("One")),
            (deckard_plugins::Slot::Sound { group: 0, sound: 1 }, plugin("Two")),
        ]);
        app
    }

    #[test]
    fn each_plugin_is_its_own_step() {
        let mut app = app_with_two_plugins();
        app.boot = Some(Boot::new(vec![Step::Plugins, Step::Done]));
        app.boot_output = Some(Ok(48000.0));
        app.boot_to_end();
        let b = app.boot.as_ref().unwrap();
        let plugin_lines = b.log.iter().filter(|l| l.label.starts_with("LOADING PLUG-IN")).count();
        assert_eq!(plugin_lines, 2);
        assert!(b.log.iter().any(|l| l.label.ends_with("[2/2]")));
        assert_eq!(b.progress(), 1.0);
    }

    #[test]
    fn output_failure_skips_the_plugins_and_says_so() {
        let mut app = app_with_two_plugins();
        app.boot = Some(Boot::new(vec![Step::Plugins, Step::Done]));
        app.boot_output = Some(Err("no device".into()));
        app.boot_to_end();
        let b = app.boot.as_ref().unwrap();
        assert!(b.log.iter().any(|l| matches!(l.outcome, Some(Outcome::Err(_))) && l.label.starts_with("AUDIO OUTPUT")));
        assert!(b.log.iter().any(|l| matches!(l.outcome, Some(Outcome::Skipped)) && l.label.contains("PLUG-INS")));
        assert!(!app.status.is_empty());
        assert!(b.finished() && b.progress() == 1.0);
    }

    #[test]
    fn the_project_step_stops_the_old_preload() {
        let mut app = crate::App::default();
        app.boot = Some(Boot::new(vec![Step::Project(None), Step::Done]));
        app.boot_to_end();
        assert!(!app.preload_plugins, "plug-ins load in boot steps, not again after");
    }

    #[test]
    fn a_miscount_cannot_hang_startup() {
        let mut app = crate::App::default();
        let mut b = Boot::new(vec![Step::Done]);
        b.total = 5;
        app.boot = Some(b);
        app.boot_to_end();
    }

    #[test]
    fn a_line_shows_before_its_step_runs() {
        let mut app = crate::App::default();
        app.boot = Some(Boot::new(vec![Step::Project(None), Step::Done]));
        app.boot_next();
        let b = app.boot.as_ref().unwrap();
        assert_eq!(b.log.len(), 1);
        assert!(b.log[0].outcome.is_none(), "drawn as running first");
        assert!(app.doc.is_none(), "the project isn't opened yet");
        app.boot_next();
        assert!(app.doc.is_some());
        assert!(matches!(app.boot.as_ref().unwrap().log[0].outcome, Some(Outcome::Text(_)) | Some(Outcome::Ok)));
    }

    #[test]
    fn runs_to_ready_and_counts_progress() {
        let mut app = crate::App::default();
        app.boot = Some(Boot::new(vec![Step::Project(None), Step::Done]));
        app.boot_to_end();
        let b = app.boot.as_ref().unwrap();
        assert!(b.finished());
        assert_eq!(b.progress(), 1.0);
        assert_eq!(b.log.last().unwrap().label, "READY");
    }

    #[test]
    fn a_missing_project_is_an_err_line_and_loading_goes_on() {
        let mut app = crate::App::default();
        app.boot = Some(Boot::new(vec![Step::Project(Some("/no/such.mxprj".into())), Step::Done]));
        app.boot_to_end();
        let b = app.boot.as_ref().unwrap();
        assert!(matches!(b.log[0].outcome, Some(Outcome::Err(_))));
        assert!(b.finished());
    }

    #[test]
    fn splash_only_for_plain_launches() {
        assert!(splash_wanted(&[]));
        assert!(splash_wanted(&["song.mxprj".into()]));
        assert!(!splash_wanted(&["--screenshot".into(), "a.png".into()]));
        assert!(!splash_wanted(&["song.mxprj".into(), "--play".into(), "song".into()]));
        assert!(!splash_wanted(&["--group".into(), "1".into()]));
        assert!(!splash_wanted(&["a.mxprj".into(), "b.mxprj".into()]));
    }

    #[test]
    fn stays_up_a_second_at_least() {
        let mut b = Boot::new(vec![Step::Done]);
        b.started = Instant::now();
        // (finish it by hand: the Done line with READY, shown one frame)
        b.log.push(LogLine { label: "READY".into(), outcome: Some(Outcome::Ok) });
        b.ready_at = Some(b.started);
        b.done = b.total;
        assert!(!b.may_close(b.started + Duration::from_millis(500)));
        assert!(b.may_close(b.started + Duration::from_millis(1100)));
    }

    fn booting_harness() -> egui_kittest::Harness<'static, crate::App> {
        booting_harness_of(crate::App::default())
    }

    /// `app` starting up, in a window.
    fn booting_harness_of(mut app: crate::App) -> egui_kittest::Harness<'static, crate::App> {
        app.boot = Some(Boot::new(vec![Step::Project(None), Step::Done]));
        egui_kittest::Harness::builder().with_size(egui::vec2(1500.0, 930.0)).build_ui_state(|ui, app: &mut crate::App| app.frame(ui), app)
    }

    /// Save a new project to `path`, as File > Save As does.
    fn save_new_project_to(path: &std::path::Path) {
        let mut doc = deckard_doc::Document::new_project().unwrap();
        doc.save_as(path).unwrap();
    }

    #[derive(Debug)]
    struct Dropped(PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }

    #[test]
    fn keys_do_nothing_while_loading() {
        // F4 toggles the browser and needs no audio device.
        let mut h = booting_harness();
        h.step();
        let before = h.state().browser.open;
        h.key_press(egui::Key::F4);
        h.step();
        assert_eq!(h.state().browser.open, before, "blocked while loading");
        h.state_mut().boot = None;
        h.step();
        h.key_press(egui::Key::F4);
        h.step();
        assert_ne!(h.state().browser.open, before, "the same key works once loaded");
    }

    #[test]
    fn a_file_dropped_while_loading_opens_after() {
        let mut h = booting_harness();
        // (Held on screen however slow the machine: the clock starts a minute ahead.)
        h.state_mut().boot.as_mut().unwrap().started += Duration::from_secs(60);
        h.step();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("dropped.mxprj");
        save_new_project_to(&p);
        h.input_mut().dropped_files = vec![std::sync::Arc::new(Dropped(p.clone())) as egui::DroppedFileHandle];
        h.step();
        h.input_mut().dropped_files.clear();
        assert_eq!(h.state().boot.as_ref().unwrap().held.as_deref(), Some(p.as_path()));
        assert_ne!(h.state().path.as_deref(), Some(p.as_path()));
        // (Steps run faster than the clock: age the screen past its minimum second.)
        h.state_mut().boot.as_mut().unwrap().started -= Duration::from_secs(62);
        h.run_steps(90);
        assert!(h.state().boot.is_none());
        assert_eq!(h.state().path.as_deref(), Some(p.as_path()));
    }

    /// A backup left by a crash, with unsaved changes, in `dir`.
    fn crashed_session(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).unwrap();
        save_new_project_to(&dir.join("session.mxprj"));
        std::fs::write(dir.join("session.txt"), "dirty = true\nclean = false\n").unwrap();
    }

    /// The Project line's label while it runs and after.
    fn project_labels(app: &mut crate::App, path: Option<PathBuf>) -> (String, String) {
        app.boot = Some(Boot::new(vec![Step::Project(path), Step::Done]));
        app.boot_next();
        let running = app.boot.as_ref().unwrap().log[0].label.clone();
        app.boot_next();
        (running, app.boot.as_ref().unwrap().log[0].label.clone())
    }

    #[test]
    fn the_project_line_says_new_project_only_for_a_new_one() {
        let mut app = crate::App::default();
        let (running, done) = project_labels(&mut app, None);
        assert_eq!((running.as_str(), done.as_str()), ("NEW PROJECT", "NEW PROJECT"));
    }

    #[test]
    fn the_project_line_names_a_project_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Song.mxprj");
        save_new_project_to(&p);
        let mut app = crate::App::default();
        let (running, done) = project_labels(&mut app, Some(p));
        assert_eq!((running.as_str(), done.as_str()), ("OPENING SONG.MXPRJ", "OPENING SONG.MXPRJ"));
    }

    #[test]
    fn the_project_line_names_the_last_project_reopened() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Last.mxprj");
        save_new_project_to(&p);
        let session = dir.path().join("session");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("session.txt"), format!("dirty = false\nclean = true\npath = {}\n", p.display())).unwrap();
        let mut app = crate::App { session_dir: Some(session), ..Default::default() };
        app.settings.reload_last = true;
        let (running, done) = project_labels(&mut app, None);
        assert_eq!((running.as_str(), done.as_str()), ("OPENING LAST.MXPRJ", "OPENING LAST.MXPRJ"));
        assert_eq!(app.path.as_deref(), Some(p.as_path()));
    }

    #[test]
    fn the_project_line_says_a_backup_is_restored() {
        let dir = tempfile::tempdir().unwrap();
        crashed_session(dir.path());
        let mut app = crate::App { session_dir: Some(dir.path().to_path_buf()), ..Default::default() };
        let (running, done) = project_labels(&mut app, None);
        assert_eq!((running.as_str(), done.as_str()), ("RESTORING BACKUP", "RESTORING BACKUP"));
        assert!(app.dirty());
    }

    #[test]
    fn closing_while_loading_a_restored_backup_asks_first() {
        let dir = tempfile::tempdir().unwrap();
        crashed_session(dir.path());
        let mut h = booting_harness_of(crate::App { session_dir: Some(dir.path().to_path_buf()), ..Default::default() });
        // (Held on screen however slow the machine.)
        h.state_mut().boot.as_mut().unwrap().started += Duration::from_secs(60);
        h.run_steps(3);
        assert!(h.state().dirty(), "the backup's unsaved changes are back");
        assert!(h.state().boot.is_some(), "still loading");
        h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        h.step();
        let commands = h.output().viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default();
        assert!(!commands.contains(&egui::ViewportCommand::Close), "{commands:?}");
        assert!(commands.contains(&egui::ViewportCommand::CancelClose), "{commands:?}");
        assert!(matches!(h.state().confirm, Some(crate::Pending::Quit)));
        assert!(!h.state().quitting);
    }

    #[test]
    fn the_frame_that_ends_loading_asks_for_another() {
        let mut app = crate::App::default();
        let mut b = Boot::new(vec![Step::Done]);
        b.started -= Duration::from_secs(2);
        // (Without the splash, which asks for frames of its own.)
        b.hidden = true;
        app.boot = Some(b);
        app.boot_to_end();
        app.boot.as_mut().unwrap().ready_at = Some(Instant::now() - Duration::from_secs(1));
        // (A fresh context: no frame asked for by an earlier one.)
        let ctx = egui::Context::default();
        for _ in 0..3 {
            ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        }
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.boot_frame(ui));
        out.textures_delta.clear();
        assert!(app.boot.is_none(), "this frame ended loading");
        let delay = out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
        assert_eq!(delay, Some(Duration::ZERO), "the UI is drawn without waiting for input");
    }

    #[test]
    fn loading_goes_on_while_the_window_is_hidden() {
        let mut app = crate::App::default();
        app.boot = Some(Boot::new(vec![Step::Project(None), Step::Done]));
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().minimized = Some(true);
        for _ in 0..4 {
            ctx.begin_pass(raw.clone());
            app.boot_hidden(&ctx);
            let mut out = ctx.end_pass();
            out.textures_delta.clear();
            assert_eq!(out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay), Some(Duration::ZERO));
        }
        assert!(app.boot.as_ref().unwrap().finished());
        assert!(app.doc.is_some());
    }

    #[test]
    fn loading_isnt_run_twice_a_frame_while_shown() {
        let mut app = crate::App::default();
        app.boot = Some(Boot::new(vec![Step::Project(None), Step::Done]));
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        app.boot_hidden(&ctx);
        ctx.end_pass().textures_delta.clear();
        assert!(app.boot.as_ref().unwrap().log.is_empty(), "boot_frame runs it when shown");
    }

    #[test]
    fn an_overcount_still_finishes() {
        let mut app = crate::App::default();
        let mut b = Boot::new(vec![Step::Done]);
        b.total = 5;
        app.boot = Some(b);
        app.boot_next();
        app.boot_next();
        assert!(app.boot.as_ref().unwrap().finished());
    }

    #[test]
    fn an_overcount_still_closes_the_screen() {
        let mut h = booting_harness();
        h.state_mut().boot.as_mut().unwrap().total += 3;
        h.state_mut().boot.as_mut().unwrap().started -= Duration::from_secs(2);
        h.run_steps(90);
        assert!(h.state().boot.is_none());
    }

    #[test]
    fn the_ui_appears_when_loading_ends() {
        let mut h = booting_harness();
        // (Steps run faster than the clock: age the screen past its minimum second.)
        h.state_mut().boot.as_mut().unwrap().started -= Duration::from_secs(2);
        h.run_steps(90);
        assert!(h.state().boot.is_none());
        assert!(h.state().doc.is_some());
    }
}
