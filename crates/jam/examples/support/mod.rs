//! What the prompted tools share: speaking a prompt, and lines typed at the terminal.

#![allow(dead_code)]

use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

/// Speak `s`, without waiting, so a prompt can be followed with eyes on the controller.
/// `JAM_SILENT` in the environment turns it off.
pub fn say(s: &str) {
    if std::env::var_os("JAM_SILENT").is_some() {
        return;
    }
    #[cfg(target_os = "macos")]
    let _ = Command::new("say").arg(s).spawn();
    #[cfg(windows)]
    {
        // Only letters, digits and spaces reach the command line.
        let clean: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ').collect();
        let script =
            format!("Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak('{clean}')");
        let _ = Command::new("powershell").args(["-NoProfile", "-Command", &script]).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = (s, Command::new("true").stdout(Stdio::null()).spawn());
}

/// Lines typed at the terminal, as they come.
pub fn typed() -> Receiver<String> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        while std::io::stdin().read_line(&mut line).is_ok_and(|n| n > 0) {
            if tx.send(line.trim().to_string()).is_err() {
                return;
            }
            line.clear();
        }
    });
    rx
}

/// Stop if NI's Maschine could be using the JAM: these tools change what its lights show.
pub fn refuse_if_maschine_runs() {
    #[cfg(windows)]
    {
        let list = Command::new("tasklist").args(["/FI", "IMAGENAME eq Maschine 3.exe", "/NH"]).output();
        if list.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("Maschine 3.exe")) {
            eprintln!("Maschine is running: close it first. This tool writes to the JAM's lights.");
            std::process::exit(1);
        }
    }
}
