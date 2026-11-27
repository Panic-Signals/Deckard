//! Write panics to disk before `deckard_plugins::exit(101)` terminates the process.
//!
//! Windows release builds set `windows_subsystem = "windows"`, detaching `stderr`, and
//! `deckard_plugins::exit` calls `TerminateProcess` to bypass loaded plug-ins' exit-time
//! destructors (no WER minidump, no Event Log entry). Without a file log, any Rust panic
//! vanishes silently.

use std::backtrace::Backtrace;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

pub(crate) fn record(info: &PanicHookInfo<'_>) {
    let bt = Backtrace::force_capture();
    let entry = format_entry(info, &bt.to_string());
    for path in log_paths() {
        append_capped(&path, entry.as_bytes());
    }
}

pub(crate) fn record_plugin_fault(summary: &str) {
    let ts = utc_timestamp(SystemTime::now());
    let pid = std::process::id();
    let cur = std::thread::current();
    let thread = cur.name().unwrap_or("<unnamed>");
    let build = build_note();
    let bt = Backtrace::force_capture();
    let entry = format!(
        "=== DECKARD PLUGIN FAULT [CONTAINED] {ts} pid={pid} thread={thread:?} ({build}) ===\n\
         fault:    {summary}\n\
         backtrace:\n{bt}\n\n"
    );
    eprintln!("[deckard] {summary}");
    for path in log_paths() {
        append_capped(&path, entry.as_bytes());
    }
}

fn log_paths() -> Vec<PathBuf> {
    if let Some(custom) = std::env::var_os("DECKARD_PANIC_LOG").filter(|v| !v.is_empty()) {
        return vec![PathBuf::from(custom)];
    }
    let mut out = Vec::new();
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        out.push(exe_dir.join("deckard-panic.log"));
    }
    if let Some(cfg) = crate::menu::settings_dir() {
        let p = cfg.join("panic.log");
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

fn build_note() -> String {
    if let Some(note) = std::env::current_exe()
        .ok()
        .map(|p| p.with_extension("txt"))
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| s.lines().next().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned))
    {
        return note;
    }
    format!("version {}", crate::VERSION)
}

fn payload_text<'a>(info: &'a PanicHookInfo<'_>) -> Option<&'a str> {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        Some(s)
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        Some(s.as_str())
    } else {
        None
    }
}

fn utc_timestamp(now: SystemTime) -> String {
    let dur = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    let millis = dur.subsec_millis();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem / 60) % 60, rem % 60);
    // Howard Hinnant's civil-from-days algorithm (UTC).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{millis:03}Z")
}

fn format_entry(info: &PanicHookInfo<'_>, backtrace: &str) -> String {
    let ts = utc_timestamp(SystemTime::now());
    let pid = std::process::id();
    let cur = std::thread::current();
    let thread = cur.name().unwrap_or("<unnamed>");
    let where_at =
        info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_else(|| "<unknown location>".into());
    let msg = payload_text(info).map(str::to_owned).unwrap_or_else(|| info.to_string());
    let build = build_note();
    format!(
        "=== DECKARD PANIC [FATAL] {ts} pid={pid} thread={thread:?} ({build}) ===\n\
         location: {where_at}\n\
         message:  {msg}\n\
         backtrace:\n{backtrace}\n\n"
    )
}

fn append_capped(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        let _ = fs::create_dir_all(parent);
    }
    if fs::metadata(path).map_or(0, |m| m.len()) > MAX_LOG_BYTES {
        let prev = path.with_extension("prev.log");
        let _ = fs::rename(path, prev);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(bytes);
        let _ = f.flush();
        let _ = f.sync_data();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn utc_timestamp_and_capped_append_persist_cleanly() {
        assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        // 2026-11-16T12:27:55.125Z = 1794832075.125s
        let sample = UNIX_EPOCH + Duration::from_millis(1_794_832_075_125);
        assert_eq!(utc_timestamp(sample), "2026-11-16T12:27:55.125Z");

        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("deckard-panic.log");
        append_capped(&log, b"first panic\n");
        append_capped(&log, b"second panic\n");
        let content = fs::read_to_string(&log).unwrap();
        assert_eq!(content, "first panic\nsecond panic\n");
    }
}
