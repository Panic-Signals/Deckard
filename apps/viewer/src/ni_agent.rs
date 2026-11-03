//! NI's NIHostIntegrationAgent holds the MK3's screens while it runs, and they stay dark:
//! it looks as if the controller doesn't work. DECKARD asks to quit it (macOS).

use eframe::egui;
use std::time::{Duration, Instant};

const NAME: &str = "NIHostIntegrationAgent";

/// How long the agent has to go after being told to quit, before asking again.
const QUITTING: Duration = Duration::from_secs(5);

/// Whether the agent is running.
pub fn running() -> bool {
    is_running(NAME)
}

/// Tell the agent to quit.
pub fn quit() -> Result<(), String> {
    stop(NAME)
}

#[cfg(target_os = "macos")]
fn is_running(name: &str) -> bool {
    std::process::Command::new("/usr/bin/pgrep")
        .args(["-x", name])
        .stdout(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(not(target_os = "macos"))]
fn is_running(_name: &str) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn stop(name: &str) -> Result<(), String> {
    let done = std::process::Command::new("/usr/bin/pkill").args(["-x", name]).status().map_err(|e| e.to_string())?;
    if done.success() { Ok(()) } else { Err(format!("couldn't quit {name}")) }
}

#[cfg(not(target_os = "macos"))]
fn stop(_name: &str) -> Result<(), String> {
    Ok(())
}

/// Where asking about the agent has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Ask {
    #[default]
    Idle,
    Asking,
    /// Told to quit: it has until then.
    Quitting(Instant),
    /// "Not now": not until the agent has gone and come back.
    Declined,
}

/// Ask to quit the agent if it's `holding` the screens. Says what came of it in `status`.
pub fn dialog(ctx: &egui::Context, ask: &mut Ask, holding: bool, status: &mut String, quit: impl FnOnce() -> Result<(), String>) {
    if let Ask::Quitting(until) = *ask {
        let now = Instant::now();
        if now >= until {
            *ask = Ask::Idle;
        } else {
            ctx.request_repaint_after(until - now);
        }
    }
    match *ask {
        Ask::Idle if holding => *ask = Ask::Asking,
        Ask::Asking | Ask::Declined if !holding => *ask = Ask::Idle,
        _ => {}
    }
    if *ask != Ask::Asking {
        return;
    }
    let (mut quitting, mut declined) = (false, false);
    egui::Modal::new(egui::Id::new("ni-agent")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(egui::RichText::new("Quit NIHostIntegrationAgent?").strong());
        ui.add_space(4.0);
        ui.label(
            "Native Instruments' agent is running and holding the Maschine MK3's screens, so the controller can look \
             as if it doesn't work. DECKARD needs it to quit. If Maschine is open, quit that first.",
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            quitting = ui.button("Quit It").clicked();
            declined = ui.button("Not Now").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
        });
    });
    if quitting {
        match quit() {
            Ok(()) => {
                *status = "NIHostIntegrationAgent quit".into();
                *ask = Ask::Quitting(Instant::now() + QUITTING);
            }
            Err(e) => {
                *status = e;
                *ask = Ask::Declined;
            }
        }
    } else if declined {
        *ask = Ask::Declined;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::cell::Cell;

    struct State {
        ask: Ask,
        holding: bool,
        status: String,
        result: Result<(), String>,
    }

    fn harness(calls: &Cell<u32>, result: Result<(), String>) -> Harness<'_, State> {
        let state = State { ask: Ask::Idle, holding: true, status: String::new(), result };
        Harness::new_ui_state(
            move |ui, s: &mut State| {
                let result = s.result.clone();
                dialog(ui.ctx(), &mut s.ask, s.holding, &mut s.status, || {
                    calls.set(calls.get() + 1);
                    result
                })
            },
            state,
        )
    }

    #[test]
    fn it_asks_and_quits_only_when_told() {
        let calls = Cell::new(0);
        let mut h = harness(&calls, Ok(()));
        h.run();
        assert_eq!(h.state().ask, Ask::Asking);
        assert_eq!(calls.get(), 0, "not without asking");
        h.get_by_label("Quit It").click();
        h.run();
        assert_eq!(calls.get(), 1);
        assert!(matches!(h.state().ask, Ask::Quitting(_)), "{:?}", h.state().ask);
        assert!(h.query_by_label("Quit It").is_none(), "the question goes away while it quits");
        // Gone: nothing more to ask.
        h.state_mut().holding = false;
        h.state_mut().ask = Ask::Quitting(Instant::now());
        h.run();
        assert_eq!(h.state().ask, Ask::Idle);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn not_now_holds_until_the_agent_comes_back() {
        let calls = Cell::new(0);
        let mut h = harness(&calls, Ok(()));
        h.run();
        h.get_by_label("Not Now").click();
        h.run();
        assert_eq!(h.state().ask, Ask::Declined);
        assert_eq!(calls.get(), 0, "left running");
        h.run();
        assert!(h.query_by_label("Not Now").is_none(), "doesn't ask again");
        h.state_mut().holding = false;
        h.run();
        h.state_mut().holding = true;
        h.run();
        assert_eq!(h.state().ask, Ask::Asking);
    }

    #[test]
    fn it_asks_again_if_the_agent_outlasts_quitting() {
        let calls = Cell::new(0);
        let mut h = harness(&calls, Ok(()));
        h.state_mut().ask = Ask::Quitting(Instant::now());
        h.run();
        assert_eq!(h.state().ask, Ask::Asking);
    }

    #[test]
    fn a_failed_quit_says_so_and_stops_asking() {
        let calls = Cell::new(0);
        let mut h = harness(&calls, Err("couldn't quit".into()));
        h.run();
        h.get_by_label("Quit It").click();
        h.run();
        assert_eq!(h.state().status, "couldn't quit");
        assert_eq!(h.state().ask, Ask::Declined);
    }

    #[test]
    fn nothing_is_asked_when_the_agent_holds_nothing() {
        let calls = Cell::new(0);
        let mut h = harness(&calls, Ok(()));
        h.state_mut().holding = false;
        h.run();
        assert_eq!(h.state().ask, Ask::Idle);
        assert!(h.query_by_label("Quit It").is_none());
    }

    /// The real lookup and quit, on a process of our own under a long name like the agent's.
    #[cfg(target_os = "macos")]
    #[test]
    fn it_finds_and_quits_a_process_by_its_whole_name() {
        let name = format!("DeckardFakeHostAgent{}", std::process::id());
        let dir = std::env::temp_dir().join(&name);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(&name);
        std::fs::copy("/bin/sleep", &exe).unwrap();
        assert!(!is_running(&name));
        let mut child = std::process::Command::new(&exe).arg("30").spawn().unwrap();
        assert!(is_running(&name), "found");
        assert!(stop(&name).is_ok());
        assert!(!child.wait().unwrap().success(), "stopped, not finished");
        assert!(!is_running(&name));
        assert!(stop(&name).is_err(), "nothing to quit");
        std::fs::remove_file(&exe).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
