//! NI's services light a JAM too, whatever DECKARD asks it to show: with them running, its
//! pads turn orange and its lights flash at every press (measured, docs/jam-usb.md). So when a
//! JAM connects, DECKARD offers to stop them, and Preferences ▸ MIDI starts them again, as
//! Maschine needs them. Never while Maschine runs: it would lose them under its feet.

use std::time::{Duration, Instant};

/// How often the services are looked at while a JAM is connected.
const EVERY: Duration = Duration::from_secs(5);

pub const WORDS: crate::ni_agent::Words = crate::ni_agent::Words {
    title: "Let DECKARD have the JAM's lights?",
    body: "Native Instruments' services are running, and they light the JAM too, so its pads turn orange and its lights flash when you press things. DECKARD can stop them, which asks for administrator approval. Maschine needs them back: start them again in Preferences > MIDI, or restart Windows.",
    button: "Stop Services",
    done: "stopping NI's services",
};

/// What was last found.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Seen {
    /// Any of NI's services that light the JAM is running.
    pub services: bool,
    pub maschine: bool,
}

impl Seen {
    /// Whether to offer to stop the services: never while Maschine is using them.
    pub fn offer(self, connected: bool) -> bool {
        connected && self.services && !self.maschine
    }
}

/// Looks at the services now and then.
#[derive(Default)]
pub struct Watch {
    pub seen: Seen,
    looked: Option<Instant>,
}

impl Watch {
    /// What's known, looked at again if it's time.
    pub fn look(&mut self, now: Instant) -> Seen {
        if self.looked.is_none_or(|t| now.duration_since(t) >= EVERY) {
            self.looked = Some(now);
            self.seen = imp::seen();
        }
        self.seen
    }
}

/// Stop the services (the dialog's button). It goes on in the background, as Windows asks for approval.
pub fn stop() -> Result<(), String> {
    imp::change(false);
    Ok(())
}

/// Preferences' button: start the services, or stop them.
pub fn start_or_stop(start: bool) {
    imp::change(start);
}

/// Why the last change failed, once.
pub fn take_failure() -> Option<String> {
    imp::take_failure()
}

/// Whether this system has the services to look after.
pub const SUPPORTED: bool = cfg!(windows);

#[cfg(windows)]
mod imp {
    use super::Seen;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS, SERVICE_RUNNING,
        SERVICE_STATUS,
    };

    /// The services that light the JAM.
    const SERVICES: [&str; 2] = ["NIHostIntegrationAgent", "NIHardwareService"];
    const MASCHINE: &str = "maschine 3.exe";

    static FAILURE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    fn service_running(name: &str) -> bool {
        let name = wide(name);
        unsafe {
            let manager = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
            if manager.is_null() {
                return false;
            }
            let service = OpenServiceW(manager, name.as_ptr(), SERVICE_QUERY_STATUS);
            let mut running = false;
            if !service.is_null() {
                let mut status: SERVICE_STATUS = std::mem::zeroed();
                running = QueryServiceStatus(service, &mut status) != 0 && status.dwCurrentState == SERVICE_RUNNING;
                CloseServiceHandle(service);
            }
            CloseServiceHandle(manager);
            running
        }
    }

    fn process_running(exe: &str) -> bool {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return false;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = false;
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more && !found {
                let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
                found = String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(exe);
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            found
        }
    }

    pub fn seen() -> Seen {
        Seen { services: SERVICES.iter().any(|s| service_running(s)), maschine: process_running(MASCHINE) }
    }

    /// `net` for each service, elevated, on a thread: Windows' prompt can wait a long time.
    pub fn change(start: bool) {
        std::thread::spawn(move || {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            // Start the hardware service first; stop the agent first.
            let verb = if start { "start" } else { "stop" };
            let mut names = SERVICES;
            if start {
                names.reverse();
            }
            let commands = names.map(|n| format!("net {verb} {n}")).join(" & ");
            let script = format!("Start-Process cmd -Verb RunAs -WindowStyle Hidden -Wait -ArgumentList '/c {commands}'");
            let out = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            let failure = match out {
                Ok(o) if o.status.success() => return,
                Ok(_) => format!("NI's services weren't changed: Windows' approval was refused or {verb} failed"),
                Err(e) => format!("NI's services weren't changed: {e}"),
            };
            *FAILURE.lock().unwrap() = Some(failure);
        });
    }

    pub fn take_failure() -> Option<String> {
        FAILURE.lock().unwrap().take()
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Seen;

    pub fn seen() -> Seen {
        Seen::default()
    }

    pub fn change(_start: bool) {}

    pub fn take_failure() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_offers_only_with_a_jam_and_the_services_and_no_maschine() {
        let running = Seen { services: true, maschine: false };
        assert!(running.offer(true));
        assert!(!running.offer(false), "no JAM connected");
        assert!(!Seen { services: false, maschine: false }.offer(true), "already stopped");
        assert!(!Seen { services: true, maschine: true }.offer(true), "Maschine needs them");
    }

    #[test]
    fn it_looks_now_and_then_not_every_frame() {
        let mut w = Watch::default();
        let t = Instant::now();
        w.look(t);
        // (What a look found stays until the next: changed here, and still seen.)
        w.seen = Seen { services: true, maschine: true };
        assert!(w.look(t + EVERY / 2).maschine, "too soon to look again");
        assert_eq!(w.looked, Some(t));
        w.look(t + EVERY);
        assert_eq!(w.looked, Some(t + EVERY), "time to");
    }
}
