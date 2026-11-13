//! Startup as a queue of steps. Each step's line is shown (running) before the step
//! runs, so a slow step is visible while it works; the outcome then replaces it.

use crate::{App, StemsOffer, jam_io, menu, mk3_io, recorder, settings, stems, update, user_chords};
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
    // (Used by the splash window.)
    #[allow(dead_code)]
    pub held: Option<PathBuf>,
    #[allow(dead_code)]
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

    /// Whether the last step (READY) has run.
    pub fn finished(&self) -> bool {
        self.done >= self.total
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

    /// Run startup to the end at once (also when the queue runs dry before the count
    /// says so, so a miscount can't hang it).
    pub(crate) fn boot_to_end(&mut self) {
        while self.boot.as_ref().is_some_and(|b| !b.finished() && (!b.queue.is_empty() || b.running.is_some())) {
            self.boot_next();
        }
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
            Step::Project(Some(p)) => {
                format!("OPENING {}", p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().to_uppercase()))
            }
            Step::Project(None) => "NEW PROJECT".into(),
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
                let name = path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
                if let Some(p) = &path {
                    self.open(p);
                    self.remember(p);
                } else {
                    self.start_project();
                }
                // (The plug-ins load in the steps after, not again once the window is up.)
                self.preload_plugins = false;
                match &self.error {
                    Some(e) => Outcome::Err(e.clone()),
                    None => Outcome::Text(name.unwrap_or_else(|| "NEW PROJECT".into())),
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
}
