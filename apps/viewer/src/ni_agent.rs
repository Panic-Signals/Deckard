//! NI's software keeps the controller's screens from DECKARD, and they stay dark: it looks
//! as if the controller doesn't work. DECKARD asks to change that.
//!
//! - macOS: NIHostIntegrationAgent holds them while it runs, so it asks to quit that.
//! - Windows: NI's kernel driver (`Model::windows_driver`) has their USB interface whether or not NI's
//!   services run, so quitting wouldn't help. It asks to switch that one interface to
//!   Windows' WinUSB (`scripts/mk3-driver.ps1`), and Preferences ▸ Hardware gives it back.

use eframe::egui;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
const NAME: &str = "NIHostIntegrationAgent";

#[cfg(not(windows))]
mod text {
    pub const TITLE: &str = "Quit NIHostIntegrationAgent?";
    pub const BODY: &str = "Native Instruments' agent is running and holding the controller's screens, so the controller can look \
         as if it doesn't work. DECKARD needs it to quit. If Maschine is open, quit that first.";
    pub const BUTTON: &str = "Quit It";
    pub const DONE: &str = "NIHostIntegrationAgent quit";
}

#[cfg(windows)]
mod text {
    pub const TITLE: &str = "Use the controller's screens in DECKARD?";
    pub const BODY: &str = "Native Instruments' driver has the controller's screens, so they stay dark in DECKARD. DECKARD can switch \
         just the screens to Windows' own WinUSB driver, which asks for administrator approval and briefly stops NI's services. \
         Pads, buttons and audio aren't touched. Maschine can't use the screens until you give them back in Preferences ▸ Hardware. \
         If Maschine is open, quit that first.";
    pub const BUTTON: &str = "Switch Driver";
    pub const DONE: &str = "switching the controller's screens to WinUSB";
}

/// How long the change has after being asked for, before asking again. On Windows that
/// includes someone answering the administrator prompt.
const QUITTING: Duration = Duration::from_secs(if cfg!(windows) { 120 } else { 5 });

/// Whether NI's software is why the screens won't open, given the error from opening them.
pub fn holding(error: &str) -> bool {
    error != deckard_mk3::display::NO_CONTROLLER && blocked(error)
}

#[cfg(target_os = "macos")]
fn blocked(_error: &str) -> bool {
    is_running(NAME)
}

#[cfg(windows)]
fn blocked(error: &str) -> bool {
    deckard_mk3::display::wrong_driver(error)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn blocked(_error: &str) -> bool {
    false
}

/// Make the change the dialog asks for. On Windows it goes on in the background, and
/// [`take_failure`] says if it failed.
#[cfg(target_os = "macos")]
pub fn quit() -> Result<(), String> {
    stop(NAME)
}

#[cfg(windows)]
pub fn quit() -> Result<(), String> {
    use_driver("WinUSB", Duration::ZERO);
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn quit() -> Result<(), String> {
    Ok(())
}

/// The screen drivers Preferences offers. Both stay in the list; [`screen_driver_in_use`]
/// only says which one has the screens now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenDriver {
    /// Windows' WinUSB driver. DECKARD draws the screens.
    Windows,
    /// NI's driver. Maschine draws the screens, and DECKARD clears them first.
    Maschine,
}

impl ScreenDriver {
    pub fn label(self) -> &'static str {
        match self {
            ScreenDriver::Windows => "Windows screen driver",
            ScreenDriver::Maschine => "Maschine screen driver",
        }
    }
}

/// Which driver has the screens. `None` when they are not open: the list still has
/// both choices, and this only says neither is in use. A switch to the Windows driver
/// counts as that choice while it is still finishing.
pub fn screen_driver_in_use(released: bool, connected: bool, switching_to_windows: bool) -> Option<ScreenDriver> {
    if released {
        Some(ScreenDriver::Maschine)
    } else if connected || switching_to_windows {
        Some(ScreenDriver::Windows)
    } else {
        None
    }
}

/// Switch the screens to the Windows driver, and don't ask again until that has had
/// time to finish (it asks for administrator approval).
pub fn use_windows(ask: &mut Ask, status: &mut String) {
    match quit() {
        Ok(()) => {
            *status = "switching to the Windows screen driver".into();
            *ask = Ask::Quitting(Instant::now() + QUITTING);
        }
        Err(e) => {
            *status = e;
            *ask = Ask::Declined;
        }
    }
}

/// Give the screens back to Maschine's driver. The caller lets go of them first, so
/// they can be cleared; the driver change waits for that.
pub fn use_maschine(ask: &mut Ask, status: &mut String) {
    *ask = Ask::Idle;
    *status = "clearing the screens and giving them to Maschine".into();
    give_back();
}

/// Give the screens back to NI's driver, once DECKARD has let go of them.
#[cfg(windows)]
pub fn give_back() {
    use_driver("NI", Duration::from_millis(500));
}

#[cfg(not(windows))]
pub fn give_back() {}

#[cfg(windows)]
static FAILURE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Why the last driver change failed, once.
pub fn take_failure() -> Option<String> {
    #[cfg(windows)]
    return FAILURE.lock().unwrap().take();
    #[cfg(not(windows))]
    None
}

#[cfg(windows)]
fn use_driver(driver: &'static str, after: Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(after);
        if let Err(e) = run_driver_script(driver) {
            *FAILURE.lock().unwrap() = Some(e);
        }
    });
}

#[cfg(windows)]
fn run_driver_script(driver: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = crate::stems::script("mk3-driver.ps1").ok_or("the controller driver script isn't with DECKARD")?;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .args(["-Use", driver])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("{}: {e}", script.display()))?;
    if out.status.success() { Ok(()) } else { Err(script_error(&String::from_utf8_lossy(&out.stderr))) }
}

/// The message of a PowerShell error: what comes before its "At <script>:<line>".
#[cfg(any(windows, test))]
fn script_error(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().map(str::trim).take_while(|l| !l.starts_with("At ")).collect();
    let message = lines.join(" ");
    if message.is_empty() { "changing the controller's driver failed".into() } else { message }
}

#[cfg(target_os = "macos")]
fn is_running(name: &str) -> bool {
    std::process::Command::new("/usr/bin/pgrep").args(["-x", name]).stdout(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

#[cfg(target_os = "macos")]
fn stop(name: &str) -> Result<(), String> {
    let done = std::process::Command::new("/usr/bin/pkill").args(["-x", name]).status().map_err(|e| e.to_string())?;
    if done.success() { Ok(()) } else { Err(format!("couldn't quit {name}")) }
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
            // (Often: a change going on in the background may fail, or finish.)
            ctx.request_repaint_after((until - now).min(Duration::from_secs(1)));
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
        ui.label(egui::RichText::new(text::TITLE).strong());
        ui.add_space(4.0);
        ui.label(text::BODY);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            quitting = ui.button(text::BUTTON).clicked();
            declined = ui.button("Not Now").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
        });
    });
    if quitting {
        match quit() {
            Ok(()) => {
                *status = text::DONE.into();
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

    #[test]
    fn the_screen_driver_list_keeps_both_choices_when_none_is_open() {
        assert_eq!(screen_driver_in_use(false, false, false), None, "not open is not a missing list");
        assert_eq!(screen_driver_in_use(false, true, false), Some(ScreenDriver::Windows));
        assert_eq!(
            screen_driver_in_use(true, false, true),
            Some(ScreenDriver::Maschine),
            "giving them back wins over a switch still finishing"
        );
        assert_eq!(screen_driver_in_use(false, false, true), Some(ScreenDriver::Windows));
        assert_eq!(ScreenDriver::Windows.label(), "Windows screen driver");
        assert_eq!(ScreenDriver::Maschine.label(), "Maschine screen driver");
    }

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
        h.get_by_label(text::BUTTON).click();
        h.run();
        assert_eq!(calls.get(), 1);
        assert!(matches!(h.state().ask, Ask::Quitting(_)), "{:?}", h.state().ask);
        assert!(h.query_by_label(text::BUTTON).is_none(), "the question goes away while it quits");
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
        h.get_by_label(text::BUTTON).click();
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
        assert!(h.query_by_label(text::BUTTON).is_none());
    }

    #[test]
    fn a_script_error_is_its_message_alone() {
        let stderr = "the MK3's screens use 'nimc3usb',\r\nnot WinUSB\r\nAt C:\\mk3-driver.ps1:166 char:30\r\n+ throw\r\n";
        assert_eq!(script_error(stderr), "the MK3's screens use 'nimc3usb', not WinUSB");
        assert_eq!(script_error(""), "changing the controller's driver failed");
    }

    #[test]
    fn no_controller_is_never_ni_holding_the_screens() {
        assert!(!holding(deckard_mk3::display::NO_CONTROLLER));
    }

    #[cfg(windows)]
    #[test]
    fn only_the_wrong_driver_counts_on_windows() {
        assert!(holding("the displays are in use (NI's agent?): incompatible driver is installed for this interface"));
        assert!(!holding("display write timed out"));
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
