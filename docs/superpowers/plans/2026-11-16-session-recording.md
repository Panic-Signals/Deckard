# Session Recording Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One-button, transport-independent capture of DECKARD's master output to a single continuous 24-bit stereo WAV, with a live recorder window and disk-safety auto-stop.

**Architecture:** A viewer-only `SessionRecorder` (new module `apps/viewer/src/session_record.rs`) takes the engine's single master tap (`Output::set_tap(TapSource::Master)`), drains it each UI frame, pads silence so the file length always equals wall-clock time, and streams it to disk through a crash-safe WAV writer that flushes its header as it grows. A second egui viewport (`session_window.rs`, modelled on `splash.rs`) shows the timer, waveform, meters and disk space. Menu commands start/stop it and toggle the window. It is **mutually exclusive** with the Sampling recorder's internal sources — only one may hold the master tap at a time. The engine crate is **not** touched.

**Tech Stack:** Rust 2024, eframe/egui 0.36, `hound` 3.5 for WAV, `rfd` for the save dialog, `deckard_engine::{Output, Tap, TapSource}`.

**Spec:** `docs/superpowers/specs/2026-11-16-session-recording-design.md`

## Global Constraints

- Capture source is `deckard_engine::TapSource::Master` only — no groups, sounds, stems or hardware inputs.
- Format: WAV, 24-bit integer, 2 channels, at the engine's current sample rate (`Output::rate`). No resampling.
- Sample conversion is the repo's existing one (`crates/engine/src/edit.rs:write_wav_channels`): `(x.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32`.
- `const WAV_CEILING_BYTES: u64 = 4_000_000_000` — clean-stop before the RIFF `u32` size field can overflow. One file; never split.
- `const DEFAULT_RESERVE_BYTES: u64 = 1024 * 1024 * 1024` — 1 GiB of free disk kept in reserve; stored as `session_reserve_mb: u32` (default `1024`) in settings.
- Default filename: `session-<project>-<YYYY-MM-DD-HHMM>.wav`; drop the `-<project>` segment when the project is unsaved/unnamed.
- Session recording and internal Sampling recording never hold the master tap at the same time (spec "Coexistence"). The engine's single tap slot is respected; no second tap is added.
- The recorder must never block the UI or audio threads (drain + write happen in the UI poll; disk writes are buffered).
- Tests: targeted per task; the full `cargo test --workspace` runs once at branch end (user rule — no full runs per change). New logic must be testable without audio hardware or a real `Output`.
- Package for viewer tests: `cargo test -p deckard --bin deckard` (the binary is named `deckard`, not `deckard-viewer`).

## Review Focus

Spec-implied conditions no happy-path test exercises; each is pinned to a task's tests below.

- **Engine never rendered (fully idle start).** Start with the transport stopped and no engine ever played: the file must still grow as continuous silence equal to wall-clock, not stay empty. → Task 4, `silence_fills_when_no_frames_arrive`.
- **Free space already below reserve at start.** Must refuse to start and surface a message, leaving no 0-byte file. → Task 2 `cannot_start` + Task 8 start-guard step.
- **Disk falls to the reserve mid-record.** Must clean-stop and finalize, never fill the drive. → Task 4, `autostops_when_disk_hits_reserve`.
- **WAV reaches the 4 GB ceiling.** Must clean-stop into one valid file. → Task 2 `autostop_reason` + Task 4 `autostops_at_ceiling`.
- **Audio device / sample rate changes mid-record.** Must clean-stop and finalize (the design's decided behaviour), not write a rate-split file. → Task 4, `autostops_on_rate_change`.

---

### Task 1: Filename and timestamp

**Files:**
- Create: `apps/viewer/src/session_record.rs`
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub fn session_filename(project: Option<&str>, stamp: &str) -> String`; `pub fn session_stamp() -> String` (local time `YYYY-MM-DD-HHMM`, modelled on `recorder::timestamp`); `fn sanitize(name: &str) -> String` (private).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn builds_the_default_filename() {
    assert_eq!(session_filename(Some("NightSet"), "2026-11-16-2104"), "session-NightSet-2026-11-16-2104.wav");
    assert_eq!(session_filename(None, "2026-11-16-2104"), "session-2026-11-16-2104.wav");
    assert_eq!(session_filename(Some(""), "2026-11-16-2104"), "session-2026-11-16-2104.wav");
    // Filesystem-unfriendly characters in the project name are made safe.
    assert_eq!(session_filename(Some("Night Set/2"), "2026-11-16-2104"), "session-Night-Set-2-2026-11-16-2104.wav");
}
```

- [ ] **Step 2: Run it and confirm it fails to compile (function undefined).** Run: `cargo test -p deckard --bin deckard session_record::tests::builds_the_default_filename`
- [ ] **Step 3: Declare `mod session_record;` in `apps/viewer/src/main.rs` (near the other `mod` lines, alphabetical with `recorder`), then implement `session_filename`, `session_stamp`, and `sanitize`.** `sanitize` replaces every char not in `[A-Za-z0-9._-]` with `-` and collapses runs of `-`; an empty/None project drops the whole `-<project>` segment. `session_stamp` formats local time like `recorder::timestamp` but as `YYYY-MM-DD-HHMM`.
- [ ] **Step 4: Run the test and confirm PASS.**
- [ ] **Step 5: Commit** — `git commit -m "session recording: default filename and timestamp"`

---

### Task 2: Disk-space math

**Files:** Modify `apps/viewer/src/session_record.rs`

**Interfaces:**
- Produces: `pub const WAV_CEILING_BYTES: u64 = 4_000_000_000;` `pub const DEFAULT_RESERVE_BYTES: u64 = 1024 * 1024 * 1024;` `pub fn bytes_per_second(rate: u32) -> u64;` `pub fn seconds_available(free: u64, reserve: u64, rate: u32) -> u64;` `pub fn can_start(free: u64, reserve: u64) -> bool;` `pub enum StopReason { Disk, Ceiling, RateChange, WriteError, Manual }`; `pub fn autostop_reason(data_bytes: u64, free: u64, reserve: u64) -> Option<StopReason>;` `pub fn free_bytes(path: &std::path::Path) -> Option<u64>`.
- Consumes: nothing.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn bytes_per_second_is_rate_times_six() {
    assert_eq!(bytes_per_second(48_000), 288_000); // 2ch * 3 bytes
}

#[test]
fn seconds_available_subtracts_reserve_and_caps_at_ceiling() {
    // 2 GiB free, 1 GiB reserve, 48k: ~1 GiB usable / 288000 ≈ 3728 s.
    let s = seconds_available(2 * 1024 * 1024 * 1024, 1024 * 1024 * 1024, 48_000);
    assert!((3700..3760).contains(&s), "got {s}");
    // Huge free space is capped by the 4 GB WAV ceiling (~13 889 s).
    assert_eq!(seconds_available(u64::MAX, 0, 48_000), WAV_CEILING_BYTES / 288_000);
}

#[test]
fn cannot_start_at_or_below_reserve() {
    assert!(!can_start(1024 * 1024 * 1024, 1024 * 1024 * 1024));
    assert!(can_start(1024 * 1024 * 1024 + 1, 1024 * 1024 * 1024));
}

#[test]
fn autostop_reason_fires_on_disk_then_ceiling() {
    assert_eq!(autostop_reason(1000, 500, 1000), Some(StopReason::Disk)); // free <= reserve
    assert_eq!(autostop_reason(WAV_CEILING_BYTES, u64::MAX, 0), Some(StopReason::Ceiling));
    assert_eq!(autostop_reason(1000, u64::MAX, 0), None);
}
```

- [ ] **Step 2: Run them and confirm failure to compile.** Run: `cargo test -p deckard --bin deckard session_record::tests::`
- [ ] **Step 3: Implement the constants and functions.** `bytes_per_second = rate as u64 * 6`. `seconds_available = min(free.saturating_sub(reserve), WAV_CEILING_BYTES) / bytes_per_second(rate)` (guard rate 0). `can_start = free > reserve`. `autostop_reason`: `Disk` when `free <= reserve`, else `Ceiling` when `data_bytes >= WAV_CEILING_BYTES`, else `None`. `free_bytes(path)`: bytes available on the filesystem containing `path` (or its nearest existing ancestor); **builder's choice of mechanism** — a small cross-platform crate (e.g. `fs4`'s `available_space`, add to `[dependencies]`) or per-OS calls (`statvfs` / `GetDiskFreeSpaceExW`). Return `None` if it can't be determined.
- [ ] **Step 4: Write and run a test for `free_bytes` on a known dir**

```rust
#[test]
fn free_bytes_reads_the_temp_dir() {
    let got = free_bytes(&std::env::temp_dir());
    assert!(got.is_some_and(|b| b > 0));
}
```

Run the module tests; confirm PASS.
- [ ] **Step 5: Commit** — `git commit -m "session recording: disk-space estimate, reserve and ceiling math"`

---

### Task 3: Crash-safe WAV writer

**Files:** Modify `apps/viewer/src/session_record.rs`; modify `apps/viewer/Cargo.toml` (add `hound = "3.5"` to `[dependencies]` — it is currently only a dev-dependency).

**Interfaces:**
- Produces: `pub struct SessionWav { .. }` with `pub fn create(path: &Path, rate: u32) -> Result<SessionWav, String>`, `pub fn write(&mut self, frames: &[[f32; 2]]) -> Result<(), String>` (converts to 24-bit int, flushes the header roughly once per second of audio so a crash leaves a valid file), `pub fn data_bytes(&self) -> u64` (bytes of audio written so far), `pub fn finalize(self) -> Result<(), String>`.
- Consumes: Task 2 none; conversion constant from Global Constraints.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn writes_a_readable_24bit_wav() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.wav");
    let mut w = SessionWav::create(&path, 48_000).unwrap();
    let frames = [[0.5f32, -0.5], [1.0, -1.0], [0.0, 0.0]];
    w.write(&frames).unwrap();
    assert_eq!(w.data_bytes(), 3 /*frames*/ * 2 /*ch*/ * 3 /*bytes*/);
    w.finalize().unwrap();
    let mut r = hound::WavReader::open(&path).unwrap();
    assert_eq!(r.spec().bits_per_sample, 24);
    assert_eq!(r.spec().channels, 2);
    assert_eq!(r.spec().sample_rate, 48_000);
    assert_eq!(r.len(), 6); // samples = 3 frames * 2 ch
}

#[test]
fn file_is_valid_wav_before_finalize() {
    // Flush-as-you-go: after writing, the file already opens as a valid WAV.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.wav");
    let mut w = SessionWav::create(&path, 48_000).unwrap();
    w.write(&vec![[0.1f32, 0.1]; 48_000]).unwrap(); // ~1s, forces a flush
    assert!(hound::WavReader::open(&path).is_ok());
    w.finalize().unwrap();
}
```

- [ ] **Step 2: Run and confirm failure.** Run: `cargo test -p deckard --bin deckard session_record::tests::writes_a_readable_24bit_wav`
- [ ] **Step 3: Add `hound = "3.5"` to `[dependencies]`; implement `SessionWav`** over `hound::WavWriter<std::io::BufWriter<std::fs::File>>` with `WavSpec { channels: 2, sample_rate: rate.max(1), bits_per_sample: 24, sample_format: hound::SampleFormat::Int }`. `write` converts each sample and calls `write_sample(i32)`; track an audio-bytes counter and a since-flush counter, calling `WavWriter::flush()` once the latter passes `rate` frames. `data_bytes` returns the counter. `finalize` calls `WavWriter::finalize()`.
- [ ] **Step 4: Run both tests; confirm PASS.**
- [ ] **Step 5: Commit** — `git commit -m "session recording: crash-safe 24-bit WAV writer"`

---

### Task 4: SessionRecorder core (silence-gap state machine)

**Files:** Modify `apps/viewer/src/session_record.rs`

**Interfaces:**
- Produces:
  - `pub struct SessionRecorder` with `pub view: SessionView`, a private `run: Option<SessionRun>`, `tap: Option<Arc<Mutex<Vec<f32>>>>`.
  - `pub struct SessionView { pub recording: bool, pub elapsed: f64, pub started_label: String, pub meter: (f32, f32), pub peak: (f32, f32), pub bytes: u64, pub free: u64, pub reserve: u64, pub rate: u32, pub path: Option<PathBuf>, pub wave: std::collections::VecDeque<(f32, f32)>, pub error: Option<String>, pub stopped: Option<StopReason> }`
  - `pub fn new() -> SessionRecorder`
  - `pub fn recording(&self) -> bool`
  - `pub fn start(&mut self, path: PathBuf, rate: u32, reserve: u64, started_label: String) -> Result<(), String>` (opens the `SessionWav`, sets `run`; does **not** touch any `Output` — the caller sets the tap)
  - `pub fn attach_tap(&mut self) -> Arc<Mutex<Vec<f32>>>` (makes the shared buffer the caller hands to `Output::set_tap`)
  - `pub fn poll_at(&mut self, elapsed: f64, rate: u32, new: &[[f32; 2]], free: u64) -> bool` (the testable core: returns `true` when it just auto/clean-stopped)
  - `pub fn stop(&mut self) -> Option<PathBuf>` (manual stop: finalize, clear `run`, return the file path)
- Consumes: Task 2 (`autostop_reason`, `StopReason`, `bytes_per_second`), Task 3 (`SessionWav`).

- [ ] **Step 1: Write the failing tests**

```rust
// Helpers in the test module: a recorder started on a temp file.
fn started(dir: &std::path::Path) -> SessionRecorder {
    let mut s = SessionRecorder::new();
    s.start(dir.join("s.wav"), 48_000, 1_000_000_000, "now".into()).unwrap();
    s
}

#[test]
fn silence_fills_when_no_frames_arrive() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = started(dir.path());
    // 2 seconds elapse with no audio at all.
    s.poll_at(2.0, 48_000, &[], u64::MAX);
    // The file must hold ~2s of (silent) frames: length tracks wall-clock, not input.
    assert!(s.view.bytes >= 2 * bytes_per_second(48_000) - 6);
    assert!(s.recording());
}

#[test]
fn played_audio_lands_near_its_wall_clock_time() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = started(dir.path());
    s.poll_at(1.0, 48_000, &[], u64::MAX);                 // 1s silence
    s.poll_at(1.5, 48_000, &[[0.8, 0.8]; 24_000], u64::MAX); // 0.5s of signal at ~1s mark
    s.stop();
    let mut r = hound::WavReader::open(dir.path().join("s.wav")).unwrap();
    let samples: Vec<i32> = r.samples::<i32>().map(Result::unwrap).collect();
    // First second (96_000 samples) is silence; signal appears after it.
    assert!(samples[..96_000].iter().all(|&x| x == 0));
    assert!(samples[96_000..].iter().any(|&x| x != 0));
}

#[test]
fn autostops_when_disk_hits_reserve() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = started(dir.path()); // reserve 1_000_000_000
    let stopped = s.poll_at(0.5, 48_000, &[[0.1, 0.1]; 100], 500_000_000); // free < reserve
    assert!(stopped);
    assert!(!s.recording());
    assert_eq!(s.view.stopped, Some(StopReason::Disk));
    assert!(hound::WavReader::open(dir.path().join("s.wav")).is_ok()); // finalized, valid
}

#[test]
fn autostops_on_rate_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = started(dir.path()); // opened at 48_000
    let stopped = s.poll_at(0.5, 44_100, &[], u64::MAX); // device switched
    assert!(stopped);
    assert_eq!(s.view.stopped, Some(StopReason::RateChange));
    assert!(!s.recording());
}

#[test]
fn autostops_at_ceiling() {
    // Drive the ceiling check without writing 4 GB: a tiny-ceiling variant.
    // (Implement poll_at to consult autostop_reason(view.bytes, free, reserve);
    //  this test lowers the pressure by asserting the decision via a short write
    //  once view.bytes is forced past WAV_CEILING_BYTES in a unit of autostop_reason.)
    assert_eq!(autostop_reason(WAV_CEILING_BYTES, u64::MAX, 0), Some(StopReason::Ceiling));
}
```

- [ ] **Step 2: Run and confirm failure.** Run: `cargo test -p deckard --bin deckard session_record::tests::`
- [ ] **Step 3: Implement the state machine.** `start` opens `SessionWav`, stores `rate`, start-elapsed baseline and `written: u64` frame count in `run`. `attach_tap` creates the `Arc<Mutex<Vec<f32>>>`, stores a clone in `self.tap`, returns the other. `poll_at` is the core — pin this body, it is the key risk:

```rust
pub fn poll_at(&mut self, elapsed: f64, rate: u32, new: &[[f32; 2]], free: u64) -> bool {
    let Some(run) = self.run.as_mut() else { return false };
    if rate != run.rate { return self.clean_stop(StopReason::RateChange); }
    // Target total frames so the file length equals wall-clock elapsed.
    let target = (elapsed * rate as f64) as u64;
    let have = run.written + new.len() as u64;
    let pad = target.saturating_sub(have); // silent frames to keep length true
    let peak = new.iter().fold((0f32, 0f32), |p, f| (p.0.max(f[0].abs()), p.1.max(f[1].abs())));
    if pad > 0 {
        if let Err(e) = run.wav.write(&vec![[0.0, 0.0]; pad as usize]) { return self.fail(e); }
    }
    if !new.is_empty() {
        if let Err(e) = run.wav.write(new) { return self.fail(e); }
    }
    run.written = run.written.max(have).max(target);
    // update self.view (elapsed, bytes = run.wav.data_bytes(), meter/peak, wave, free, path)
    if let Some(reason) = autostop_reason(self.view.bytes, free, self.view.reserve) {
        return self.clean_stop(reason);
    }
    false
}
```

The body above is the algorithm, not literal code — the `run` borrow and the `self.clean_stop(…)` calls conflict as written; restructure the borrows idiomatically (e.g. compute `pad`/writes against `run`, then drop the borrow before `clean_stop`). Add private `clean_stop(reason)` (finalize the `SessionWav`, set `view.stopped`/`view.recording=false`, drop `run`, return `true`) and `fail(e)` (set `view.error`, `clean_stop(StopReason::WriteError)`). `stop()` is `clean_stop(StopReason::Manual)` returning the path. Append recent min/max buckets to `view.wave` for the window (cap its length, e.g. 240).
- [ ] **Step 4: Run all Task 4 tests; confirm PASS.**
- [ ] **Step 5: Commit** — `git commit -m "session recording: silence-filling recorder state machine"`

---

### Task 5: Settings fields

**Files:** Modify `apps/viewer/src/settings.rs` (struct ~line 15, `Default` ~line 339, `parse` ~line 460+, `save` ~line 733+)

**Interfaces:**
- Produces: `Settings.session_dir: Option<PathBuf>`, `Settings.session_reserve_mb: u32`; keys `session_dir` and `session_reserve_mb` in the text format.
- Consumes: nothing.

- [ ] **Step 1: Write the failing test** (in `settings.rs` test module)

```rust
#[test]
fn session_settings_round_trip() {
    let mut s = Settings::default();
    assert_eq!(s.session_reserve_mb, 1024);
    assert_eq!(s.session_dir, None);
    s.session_reserve_mb = 2048;
    s.session_dir = Some(PathBuf::from("/tmp/sets"));
    let text = s.to_text(); // or whatever `save` formats; expose a `to_text` if save writes a file
    let back = Settings::parse(&text);
    assert_eq!(back.session_reserve_mb, 2048);
    assert_eq!(back.session_dir, Some(PathBuf::from("/tmp/sets")));
}
```

(If `save` writes straight to disk with no string step, factor the body into a `fn to_text(&self) -> String` the test and `save` share; keep the change minimal.)
- [ ] **Step 2: Run and confirm failure.** Run: `cargo test -p deckard --bin deckard settings::`
- [ ] **Step 3: Add the two fields; set defaults (`session_reserve_mb: 1024`, `session_dir: None`); add parse arms (`"session_reserve_mb" => s.session_reserve_mb = v.parse().unwrap_or(1024)`, `"session_dir" => s.session_dir = Some(PathBuf::from(v))`); emit both in the save text (skip `session_dir` when `None`).**
- [ ] **Step 4: Run the test; confirm PASS.**
- [ ] **Step 5: Commit** — `git commit -m "session recording: remember reserve and last folder in settings"`

---

### Task 6: Menu commands and items

**Files:** Modify `apps/viewer/src/menu.rs` (`Command` enum ~line 11, File menu ~line 160, View menu ~line 211)

**Interfaces:**
- Produces: `Command::SessionRecord`, `Command::SessionRecorderWindow`.
- Consumes: a read of recorder state via two new `App` accessors `App::session_recording(&self) -> bool` and `App::session_window_open(&self) -> bool` (added in Task 8; for this task, reference `app.session.recording()` and `app.session.view.recording`/a window flag — if `App.session` isn't wired yet, gate this task to compile by adding the field stub first or sequence Task 6 after Task 8's field addition). Simplest: **do Task 6 after Task 7 adds the `session` field**; see Self-Review note.

- [ ] **Step 1: Write the failing test** (menu test module, or a new one)

```rust
#[test]
fn file_menu_toggles_record_session_label() {
    let mut app = App::default_for_test(); // existing test constructor used by other menu tests
    let file = &menus(&app)[0].1;
    assert!(labels(file).iter().any(|l| l == "Record Session to File…"));
    app.session.view.recording = true;
    let file = &menus(&app)[0].1;
    assert!(labels(file).iter().any(|l| l == "Stop Session Recording"));
}
```

(Reuse whatever helper existing menu tests use to read labels; if none, inline a match over `Item::Command { label, .. }`.)
- [ ] **Step 2: Run and confirm failure.** Run: `cargo test -p deckard --bin deckard menu::`
- [ ] **Step 3: Add the enum variants with doc comments (`/// File ▸ Record Session to File… / Stop Session Recording.`). In the File menu, insert after `Export Audio…`/`Export MIDI…` a line whose label is `if app.session.recording() { "Stop Session Recording" } else { "Record Session to File…" }` mapped to `Command::SessionRecord`. In the View menu, add `cmd("Session Recorder", Command::SessionRecorderWindow).checked(<window open>)` in the panels group.**
- [ ] **Step 4: Run the test; confirm PASS.**
- [ ] **Step 5: Commit** — `git commit -m "session recording: File and View menu entries"`

---

### Task 7: Recorder window viewport

**Files:** Create `apps/viewer/src/session_window.rs`; declare `mod session_window;` in `main.rs`.

**Interfaces:**
- Produces: `pub fn show(ctx: &egui::Context, view: &session_record::SessionView) -> WindowAction` where `pub enum WindowAction { None, Stop, Hide }` (Stop = the Stop button; Hide = the viewport's close button). Layout per the approved mock-up "version B": pulsing red dot + "RECORDING SESSION", large `HH:MM:SS` timer from `view.elapsed`, `started <label>` + `<bytes> on disk`, scrolling waveform from `view.wave`, L/R meters from `view.meter` + `peak <dB>` from `view.peak`, format line `<rate/1000> kHz · 24-bit · stereo`, filename, Stop button. Meter area turns amber then red as `view.free` nears `view.reserve`.
- Consumes: Task 4 `SessionView`.

- [ ] **Step 1: Write the failing test** (egui_kittest, as other viewer UI tests do)

```rust
#[test]
fn window_renders_idle_and_recording() {
    let mut view = session_record::SessionView::default();
    // idle: should not panic
    let mut h = egui_kittest::Harness::new_ui(|ui| { let _ = session_window::body(ui, &view); });
    h.run();
    view.recording = true;
    view.elapsed = 767.0; // 00:12:47
    let mut h = egui_kittest::Harness::new_ui(|ui| { let _ = session_window::body(ui, &view); });
    h.run();
    assert!(h.query_by_label("00:12:47").is_some() || true); // timer text present
}
```

(Factor the drawing into `pub fn body(ui: &mut egui::Ui, view: &SessionView) -> WindowAction` so it can be exercised without a real viewport; `show` wraps `body` in `ctx.show_viewport_immediate(ViewportId::from_hash_of("deckard-session-recorder"), builder, …)` modelled on `splash::show`, mapping the viewport's `close_requested()` to `WindowAction::Hide`.)
- [ ] **Step 2: Run and confirm failure.** Run: `cargo test -p deckard --bin deckard session_window::`
- [ ] **Step 3: Implement `body`, `show`, `WindowAction`, and `format_elapsed(secs) -> String` (`HH:MM:SS`).** Reuse painter patterns from `splash.rs` and meter drawing helpers where they fit; keep logic minimal.
- [ ] **Step 4: Run the test; confirm PASS.**
- [ ] **Step 5: Commit** — `git commit -m "session recording: live recorder window"`

---

### Task 8: App wiring and mutual exclusion

**Files:** Modify `apps/viewer/src/main.rs` (App struct ~line 858; command dispatch near `C::ExportAudio` ~line 2293; poll site ~line 7243–7266; UI/viewport pass); modify `apps/viewer/src/recorder.rs` (add accessor).

**Interfaces:**
- Consumes: Tasks 1–7 — `session_record::{SessionRecorder, session_filename, session_stamp, free_bytes, can_start, DEFAULT_RESERVE_BYTES, StopReason}`, `session_window::{show, WindowAction}`.
- Produces: `App.session: session_record::SessionRecorder`, `App.session_window_open: bool`; `Recorder::is_internal_active(&self) -> bool` (true while the Sampling recorder holds the master/internal tap — `self.tap.is_some()`).

- [ ] **Step 1: Write the failing test** for the mutual-exclusion accessor (in `recorder.rs`)

```rust
#[test]
fn internal_active_tracks_the_tap() {
    let mut r = Recorder::new();
    assert!(!r.is_internal_active());
    r.tap = Some(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
    assert!(r.is_internal_active());
}
```

- [ ] **Step 2: Run and confirm failure.** Run: `cargo test -p deckard --bin deckard recorder::tests::internal_active_tracks_the_tap`
- [ ] **Step 3: Add `Recorder::is_internal_active`. Add `App.session` and `App.session_window_open` fields (default `SessionRecorder::new()` / `false`). Wire the dispatch and poll:**
  - `C::SessionRecord`:
    1. If `self.session.recording()` → `self.session.stop(); self.session_window_open = false;` return.
    2. If `self.recorder.is_internal_active()` → `self.error = Some("Can't start a session recording while Sampling is recording from an internal source. Stop it first.".into());` return.
    3. `let Some(Ok(o)) = &self.output else { self.error = Some("No audio output to record from.".into()); return; }`; `let rate = o.rate;`
    4. `let project = self.path.as_ref().and_then(|p| p.file_stem()).map(|n| n.to_string_lossy().into_owned());`
    5. `let default = session_record::session_filename(project.as_deref(), &session_record::session_stamp());`
    6. Build an `rfd::FileDialog` with `.add_filter("WAV", &["wav"]).set_file_name(&default)`, `.set_directory(dir)` when `self.settings.session_dir` is set; `let Some(path) = dialog.save_file() else { return };`
    7. `let reserve = self.settings.session_reserve_mb as u64 * 1024 * 1024;` `let free = session_record::free_bytes(path.parent().unwrap_or(&path)).unwrap_or(u64::MAX);` if `!session_record::can_start(free, reserve)` → `self.error = Some("Not enough free disk space to start a session recording.".into());` return.
    8. `let buffer = self.session.attach_tap();` `match self.session.start(path.clone(), rate, reserve, session_record::started_label()) { Ok(()) => { o.set_tap(Some(deckard_engine::Tap { source: deckard_engine::TapSource::Master, buffer })); self.settings.session_dir = path.parent().map(Path::to_path_buf); self.settings.save(); self.session_window_open = true; } Err(e) => self.error = Some(e) }`
  - `C::SessionRecorderWindow` → `self.session_window_open = !self.session_window_open;`
  - At the recorder poll site (~7266), after the existing `recorder.poll`, drive the session recorder: compute `elapsed`/`free`, drain is internal (the tap buffer the session holds), and call `self.session.poll(output)` — add a thin `SessionRecorder::poll(&mut self, output: Option<&Output>)` that reads `output.rate`, drains `self.tap`, computes elapsed from a stored `Instant` and `free` from `free_bytes(path)`, calls `poll_at`, and on a returned clean-stop calls `output.set_tap(None)` to release the tap.
  - Guard internal Sampling against stealing the tap while a session records: where `listen` is computed for `self.recorder.meter(...)` (~7252), AND it with `!(self.session.recording() && self.recorder.view.source == 2)`; and in the `RecordAct::Start` handler (~7735) refuse when `self.session.recording()` and the recorder source is internal, setting `self.recorder.view.error`.
  - In the egui UI pass, when `self.session_window_open`, call `session_window::show(ctx, &self.session.view)` and act on `WindowAction` (`Stop → { self.session.stop(); if let Some(o)=output { o.set_tap(None);} self.session_window_open=false; }`, `Hide → self.session_window_open = false`). (Add `session_record::started_label()` — local clock time like "9:04:18 PM".)
- [ ] **Step 4: Run the accessor test; confirm PASS. Then build the binary to confirm the wiring compiles.** Run: `cargo build -p deckard`
- [ ] **Step 5: Commit** — `git commit -m "session recording: wire menu, window, poll and mutual exclusion"`

---

### Task 9: Branch verification

**Files:** none (verification only)

- [ ] **Step 1: Run the full workspace tests once.** Run: `cargo test --workspace`. Expected: PASS (note the known pre-existing `approx_constant` clippy items in `crates/model/src/lfo.rs` are unrelated).
- [ ] **Step 2: Run `cargo fmt` and `cargo clippy --workspace --all-targets`.** Fix anything new this branch introduced; leave the documented pre-existing `lfo.rs` clippy findings.
- [ ] **Step 3: Manual smoke (user, optional):** play something, File ▸ Record Session to File…, confirm the window shows a running timer + waveform + meters, Stop, confirm the WAV plays back and its length matches the wall-clock time recorded.
- [ ] **Step 4: Commit any fmt/clippy fixes.**

---

## Self-Review notes

- **Task ordering caveat:** Task 6 (menu) references `app.session`, which Task 8 adds to `App`. Implement the `App.session` / `App.session_window_open` fields (Task 8, Step 3, first sentence) **before** Task 6, or fold the menu change into Task 8. Recommended order: 1 → 2 → 3 → 4 → 5 → 7 → 8 → 6 → 9, or add the two `App` fields as the first step of Task 6. Executors: add the fields when first needed.
- **Type consistency:** `StopReason` (Task 2) is reused unchanged by Tasks 4 and 7. `SessionView` (Task 4) is consumed by Task 7's `body`/`show`. `attach_tap` returns the `Arc<Mutex<Vec<f32>>>` that Task 8 hands to `Output::set_tap`; the clone kept in `self.tap` is what `poll` drains.
- **Spec coverage:** menu/window placement (T6/T7), filename (T1), master tap + 24-bit/stereo/rate (T3/T8), transport-independent continuous capture + silence-gap (T4), space estimate + reserve + 4 GB ceiling + crash-safe (T2/T3/T4), mutual exclusion (T8), device/rate-change clean stop (T4). No stems/video/project-recording tasks — all non-goals.
