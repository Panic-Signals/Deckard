# Startup Splash Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A teal CRT "boot" splash window that opens first, logs DECKARD's real startup steps one per frame (plug-ins one each) with a progress bar and GitHub / Discord / Release Notes buttons, blocks input to the main window and controllers until loading is done, then closes.

**Architecture:** Startup work moves out of the `run_native` creation closure into a step queue (`boot.rs`, `impl App`) that `App::frame` advances once per frame while `App::boot` is `Some`; during that time `frame` takes an early branch that runs the step, drains controller input, holds dropped files, paints a plain backdrop in the main window and draws the splash in a second, borderless, always-on-top viewport (`splash.rs`, drawing only). Non-splash runs (tests, automation flags) run the same step list to the end inside the closure.

**Tech Stack:** Rust 2024, eframe/egui 0.36, egui_kittest for tests.

**Spec:** `docs/superpowers/specs/2026-11-12-startup-splash-design.md`

## Global Constraints

- Work on a branch `startup-splash` off `main`; first cherry-pick the spec commit `857551f` (it was committed on `groove-live-views`).
- All code is in `apps/viewer/src`. Run tests with `cargo test -p deckard <filter>` from the repo root.
- Plug-in instantiation stays on the main thread (`deckard_plugins::mark_main_thread`).
- Each step's log line is drawn on the frame **before** the step runs.
- The bar moves only when a step finishes; no timed animation of progress.
- Splash stays up at least **1.0 s** from its first frame; closes on the frame after `READY` is shown (and the 1.0 s has passed).
- Splash size **760×480** logical px, no decorations, not resizable, `WindowLevel::AlwaysOnTop`, centred on the main window's monitor.
- Colours: foreground `#4fe0cf`, dim `#1f7a70`, background `#011716`.
- `deckard` in the logo font as `"d\u{65}ck\u{61}\u{72}d"` (family `layout::LOGO_FONT`; fall back to `layout::deckard_logo_scaled` when the logo font flag is false).
- Links: GitHub `https://github.com/Panic-Signals/Deckard`; Discord `https://discord.gg/SGSpFaUnNh`; release notes `update::release_notes_url(VERSION)` when `github::parse_version(VERSION)` is `Some`, else `https://github.com/Panic-Signals/Deckard/releases/latest`. Open with `ctx.open_url(egui::OpenUrl::new_tab(..))`.
- Splash only for plain launches: no arguments, or one project path. Any argument starting with `--` → no splash, steps run in the closure as today.
- Out of scope: MK3 display splash, a Preferences switch, splash for File ▸ Open.

## Review Focus

1. **Plug-ins load twice.** `show_document` → `leave_document` sets `preload_plugins = on_main_thread()`; if the Project step doesn't clear it, the old one-shot `load_plugins` runs after boot and instantiates everything again. Test in Task 2.
2. **Pad hits during loading play afterwards.** `Mk3Io::take_pending` queues hits made "while no engine ran" and `mk3_sync` plays them later; boot must discard both `take_events` and `take_pending` (MK3 and Launchpad). Test in Task 3 by asserting the queues are drained each boot frame.
3. **Keyboard shortcuts during loading.** Space / Cmd-keys handled by `keyboard_keys` / `editing_keys` must not run while booting (no playback starts). Test in Task 3.
4. **A file dropped during loading.** It must not open mid-load and must open after; if several are dropped, the last one wins. Test in Task 3.
5. **Audio output fails with plug-ins in the project.** Plug-ins are skipped with one `SKIPPED` line and the status shows the failure, rather than silently loading nothing. Test in Task 2 via the injected output result.

---

### Task 1: The step queue (`boot.rs`), without plug-ins

**Files:**
- Create: `apps/viewer/src/boot.rs` (add `mod boot;` in `main.rs`)
- Modify: `apps/viewer/src/main.rs` — `App` gets `boot: Option<boot::Boot>` (default `None`)
- Test: `#[cfg(test)] mod tests` at the bottom of `boot.rs`

**Interfaces:**
- Produces:
  - `pub enum Step { Settings, AudioDriver, UpdateCheck, Mk3, Launchpad, Surface, Catalog, Project(Option<PathBuf>), Plugins, AudioOutput, Plugin(usize), Done }`
  - `pub enum Outcome { Ok, Text(String), Err(String), Skipped }`
  - `pub struct LogLine { pub label: String, pub outcome: Option<Outcome> }` (`None` = running now)
  - `pub struct Boot { pub log: Vec<LogLine>, pub done: usize, pub total: usize, pub started: Instant, pub ready_at: Option<Instant>, pub held: Option<PathBuf>, pub hidden: bool, queue: VecDeque<Step>, wanted: Vec<(deckard_plugins::Slot, deckard_doc::PluginRef)>, failed: Vec<String> }` — use whatever type `load_plugins`' `wanted` holds today for the second tuple element.
  - `impl Boot { pub fn new(steps: Vec<Step>) -> Boot; pub fn progress(&self) -> f32 /* done/total, 0..=1 */; pub fn finished(&self) -> bool /* Done ran */; pub fn may_close(&self, now: Instant) -> bool /* finished && ready_at shown ≥1 frame && now-started ≥ 1.0 s */ }`
  - `pub fn all_steps(path: Option<PathBuf>) -> Vec<Step>` — the order in the spec's table: Settings, AudioDriver, UpdateCheck, Mk3, Launchpad, Surface, Catalog, Project(path), Plugins, Done.
  - `pub fn splash_wanted(args: &[String]) -> bool`
  - `impl App { pub(crate) fn boot_next(&mut self); pub(crate) fn boot_to_end(&mut self) }` — `boot_next`: if the last log line's outcome is `None`, run that step and set its outcome (and `done += 1`); otherwise pop the next step and push its line with `outcome: None`. `boot_to_end` calls `boot_next` until `finished()`.

- [ ] **Step 1: Write the failing tests**

```rust
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
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p deckard boot::`
Expected: compile error (module `boot` / `Step` not found).

- [ ] **Step 3: Implement `boot.rs`**

Each step runs today's closure code for it (`main.rs` `main()` lines ~271–313), moved here:
- `Settings`: `user_chords`, `recent`, `settings`, `browser.routing/patterns` → `Ok`.
- `AudioDriver`: `set_driver` when `asio_device` is set → `Ok` / `Skipped`.
- `UpdateCheck`: `update = update::Notice::start(..)`, `after_update(VERSION)` → `Ok`.
- `Mk3`: `mk3 = Some(Mk3Io::start())`, brightness → `Text("FOUND")` if `displays_connected()` else `Text("NOT CONNECTED")`.
- `Launchpad`: `jam = Some(JamIo::start())` → `FOUND` / `NOT CONNECTED` from `connected()`.
- `Surface`: `surface = new_surface()`, recorder, `module::set_search_paths` → `Ok`.
- `Catalog`: `catalog = Catalog::load(..)`, `session_dir` → `Text("<n> READY")` or `Text("SCANNING")` when `catalog.scanning()`. The stems-offer check moves here too.
- `Project(path)`: `Some(p)` → `open` + `remember`; `None` → `start_project`. `error` set → `Err(error)`, else `Text(<file name> | "NEW PROJECT")`. Then `self.preload_plugins = false` (Review Focus 1).
- `Done`: label `READY`, sets `ready_at` on the frame after its line is drawn.

Labels: `READING SETTINGS`, `AUDIO DRIVER`, `CHECKING FOR UPDATES`, `MASCHINE MK3`, `LAUNCHPAD PRO MK3`, `CONTROLLER SURFACE`, `PLUG-IN CATALOG`, `OPENING <name>` / `NEW PROJECT`, `READY`. `total` starts at the queue length; `Plugins` is a no-op `Skipped` until Task 2.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p deckard boot::`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add apps/viewer/src/boot.rs apps/viewer/src/main.rs
git commit -m "Boot: startup as a queue of steps, each logged before it runs"
```

---

### Task 2: Plug-ins one per step

**Files:**
- Modify: `apps/viewer/src/main.rs` — split `load_plugins` (~10486–10543)
- Modify: `apps/viewer/src/boot.rs` — `Plugins`, `AudioOutput`, `Plugin(i)`
- Test: `boot.rs` tests

**Interfaces:**
- Consumes: Task 1's `Boot`, `Step`, `Outcome`, `boot_next`.
- Produces:
  - `fn wanted_plugins(&self) -> Vec<(deckard_plugins::Slot, <today's plugin type>)>` — today's list-building half of `load_plugins`.
  - `fn load_plugin(&mut self, slot: deckard_plugins::Slot, p: &<type>, rate: f64) -> Result<(), String>` — today's per-entry `self.samples.plugins.instance(..)`.
  - `fn plugins_loaded_status(n: usize, secs: f64, failed: &[String]) -> String` — today's final status text.
  - `load_plugins` stays (other callers) and is rewritten over these three; behaviour unchanged.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn each_plugin_is_its_own_step() {
    let mut app = app_with_two_plugins(); // a blank project whose sounds 1–2 carry plug-in refs (test helper; ids need not exist)
    app.boot = Some(Boot::new(vec![Step::Plugins, Step::Done]));
    app.boot_output = Some(Ok(48000.0)); // test hook: what AudioOutput returns
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
}

#[test]
fn the_project_step_stops_the_old_preload() {
    let mut app = crate::App::default();
    app.boot = Some(Boot::new(vec![Step::Project(None), Step::Done]));
    app.boot_to_end();
    assert!(!app.preload_plugins, "plug-ins load in boot steps, not again after");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p deckard boot::`
Expected: compile error (`boot_output`, `wanted_plugins` missing).

- [ ] **Step 3: Implement**

- `Plugins` runs `wanted_plugins()`; none → `Text("NONE")`; else stores `wanted`, pushes `AudioOutput` then `Plugin(0..n)` to the queue front, `total += n + 1`, outcome `Text("<n> TO LOAD")`.
- `AudioOutput`: `#[cfg(test)]`-only field `App::boot_output: Option<Result<f64, String>>` overrides; otherwise `self.output.get_or_insert_with(|| open_output(..))` → rate. Err → `Err(e)`, drop the queued `Plugin` steps, push a line `PLUG-INS (<n>)` with `Skipped`, set `self.status` to `plug-ins not loaded: <e>`. Label `AUDIO OUTPUT`.
- `Plugin(i)`: label `LOADING PLUG-IN <name> [i+1/n]`; `load_plugin` → `Ok` or `Err(e)` (push to `failed`). After the last, `self.status = plugins_loaded_status(..)`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p deckard boot:: && cargo test -p deckard plugin`
Expected: all pass (existing plug-in tests unchanged).

- [ ] **Step 5: Commit**

```bash
git commit -am "Boot: plug-ins load one per step, after the audio output"
```

---

### Task 3: Boot frames in the app: blocked input, held drops, startup moved out of the closure

**Files:**
- Modify: `apps/viewer/src/main.rs` — `main()` closure; `frame()` (~11021); `logic()` (~10972)
- Modify: `apps/viewer/src/boot.rs` — `impl App { pub(crate) fn boot_frame(&mut self, ui: &mut egui::Ui) }`
- Test: `boot.rs` tests (kittest)

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces: `boot_frame(ui)`; on finishing it sets `self.boot = None` and calls `self.open_file(held)` if one was held. Task 4 calls `splash::show(ctx, &boot) ` from inside `boot_frame`.

- [ ] **Step 1: Write the failing tests**

```rust
fn booting_harness() -> egui_kittest::Harness<'static, crate::App> {
    let mut app = crate::App::default();
    app.boot = Some(Boot::new(vec![Step::Project(None), Step::Done]));
    egui_kittest::Harness::builder()
        .with_size(egui::vec2(1500.0, 930.0))
        .build_ui_state(|ui, app: &mut crate::App| app.frame(ui), app)
}

#[test]
fn space_does_nothing_while_loading() {
    let mut h = booting_harness();
    h.step();
    h.key_press(egui::Key::Space);
    h.step();
    assert!(h.state().playing.is_none());
}

#[test]
fn a_file_dropped_while_loading_opens_after() {
    let mut h = booting_harness();
    h.step();
    // A project that exists on any machine (CI too): a new project saved into a
    // tempfile::tempdir() with the call File ▸ Save As uses.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("dropped.mxprj");
    save_new_project_to(&p); // test helper in this module
    h.input_mut().dropped_files = vec![egui::DroppedFile { path: Some(p.clone()), ..Default::default() }];
    h.step();
    h.input_mut().dropped_files.clear();
    assert_eq!(h.state().boot.as_ref().unwrap().held.as_deref(), Some(p.as_path()));
    assert_ne!(h.state().path.as_deref(), Some(p.as_path()));
    h.run_steps(90); // > 1 s at 60 fps
    assert!(h.state().boot.is_none());
    assert_eq!(h.state().path.as_deref(), Some(p.as_path()));
}

#[test]
fn the_ui_appears_when_loading_ends() {
    let mut h = booting_harness();
    h.run_steps(90);
    assert!(h.state().boot.is_none());
    assert!(h.state().doc.is_some());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p deckard boot::`
Expected: FAIL (space/drop handled normally; `boot` never cleared).

- [ ] **Step 3: Implement**

- `frame()`: right after the first `finish_quit` check, `if self.boot.is_some() { self.boot_frame(ui); return; }`.
- `logic()`: return early when `self.boot.is_some()`.
- `boot_frame`: (a) `if let Some(m) = &self.mk3 { m.set_context(ctx); m.take_events(); m.take_pending(); }`, same for `jam`; (b) last non-empty `raw.dropped_files` path → `boot.held`; (c) `self.boot_next()`; (d) fill the main window with `layout::S00` (plain backdrop, no widgets); (e) splash drawing (Task 4; until then nothing); (f) when `may_close(Instant::now())` → `self.boot = None`, open `held` via `open_file`; else `ctx.request_repaint()`.
- `main()` closure: keep `layout::style`, the icon, `App { model, scripts, show_controller, .. }` fields that aren't steps, and `_running`. If `boot::splash_wanted(&args)`: `app.boot = Some(Boot::new(boot::all_steps(path)))` and return the app (no flags apply: there are none). Else: `app.boot = Some(..)`, `app.boot_to_end()`, `app.boot = None`, then the existing flag handling (`--group` … `--tempo`) unchanged.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p deckard`
Expected: all pass, including the existing suites (they build `App` directly and never set `boot`).

- [ ] **Step 5: Run the app**

Run: `cargo run -p deckard` — the main window shows a plain backdrop for the loading time, then the normal UI with the last project; Space/pads do nothing until then. `cargo run -p deckard -- --screenshot /tmp/x.png` behaves as before.

- [ ] **Step 6: Commit**

```bash
git commit -am "Boot: run startup a step per frame, input blocked and drops held until loaded"
```

---

### Task 4: The splash window (`splash.rs`)

**Files:**
- Create: `apps/viewer/src/splash.rs` (add `mod splash;`)
- Modify: `apps/viewer/src/boot.rs` — call `splash::show` in `boot_frame` step (e)
- Test: `splash.rs` tests

**Interfaces:**
- Consumes: `boot::Boot` (`log`, `progress()`, `hidden`), `VERSION`.
- Produces:
  - `pub const GITHUB_URL: &str`, `pub const DISCORD_URL: &str`
  - `pub fn release_notes_link(version: &str) -> String`
  - `pub fn bar_text(progress: f32, width: usize) -> String` — e.g. `[█████░░░░░]  50%`
  - `pub fn show(ctx: &egui::Context, boot: &mut Boot)` — opens/updates the viewport; sets `boot.hidden` on a close request.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn release_notes_for_this_version_or_the_latest() {
    assert_eq!(release_notes_link("0.4.2"), "https://github.com/Panic-Signals/Deckard/releases/tag/0.4.2");
    assert_eq!(release_notes_link("0.1.0-dev"), "https://github.com/Panic-Signals/Deckard/releases/latest");
}

#[test]
fn the_bar_is_text() {
    assert_eq!(bar_text(0.5, 10), "[█████░░░░░]  50%");
    assert_eq!(bar_text(1.0, 4), "[████] 100%");
    assert_eq!(bar_text(0.0, 4), "[░░░░]   0%");
}

#[test]
fn the_buttons_open_their_links() {
    // kittest: show() in a harness (embedded viewport), click the GITHUB button by its
    // accessible label "GitHub", assert ctx.output().open_url is GITHUB_URL; same for
    // "Discord" → DISCORD_URL and "Release notes" → release_notes_link(VERSION).
}
```

The third test's body is the implementer's, with those three label → URL assertions.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p deckard splash::`
Expected: compile error.

- [ ] **Step 3: Implement `show`**

- `ctx.show_viewport_immediate(ViewportId::from_hash_of("deckard-splash"), builder, |ctx, _class| ..)` unless `boot.hidden`; builder per Global Constraints, title `DECKARD`, position = centre of `ctx.input(|i| i.viewport().monitor_size)` minus half of 760×480 (omit when unknown).
- Paint (in a `CentralPanel` with background `#011716`): radial vignette (a few concentric translucent rects or a mesh), scanlines (a 1 px line every 3 px at ~25 % black), glow = the same text drawn 2–3 times offset/alpha-faded under the sharp text.
- Layout top to bottom: header row (`PANIC SIGNALS // DECKARD BOOT SEQUENCE` left; `v{VERSION} · HH:MM:SS` right, dim, rule under it); logo (logo font, ~92 px); `GROOVEBOX · SAMPLER · SEQUENCER` dim, spaced; the log in monospace (`> LABEL ..... RESULT`, dots to a fixed column, the running line ends in a block cursor blinking at 1 Hz, oldest lines scroll off); `<current label>  bar_text(progress, 40)`; three framed ASCII buttons (texts below), dim frame, brightens on hover, with captions `[ SOURCE ]`, `[ COMMUNITY ]`, `[ RELEASE NOTES ]`. Buttons get accessible labels `GitHub`, `Discord`, `Release notes`.
- `ctx.input(|i| i.viewport().close_requested())` inside the viewport → `boot.hidden = true`.

ASCII art (exact copy):

```text
  ___ ___ _____ _  _ _   _ ___
 / __|_ _|_   _| || | | | | _ )
| (_ || |  | | | __ | |_| | _ \
 \___|___| |_| |_||_|\___/|___/

 ___ ___ ___  ___ ___  ___ ___
|   \_ _/ __|/ __/ _ \| _ \   \
| |) | |\__ \ (_| (_) |   / |) |
|___/___|___/\___\___/|_|_\___/

 _  _  ___ _____ ___ ___
| \| |/ _ \_   _| __/ __|
| .` | (_) || | | _|\__ \
|_|\_|\___/ |_| |___|___/
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p deckard splash:: && cargo test -p deckard`
Expected: all pass.

- [ ] **Step 5: Look at it**

Run: `cargo run -p deckard` with a project that has plug-ins. Check: splash opens centred before the UI, lines appear one by one, `LOADING PLUG-IN <name> [i/n]` shows before each freeze, bar advances, buttons open the browser without closing the splash, splash closes after `READY` and the normal UI is there. Compare against the mockup (`.superpowers/brainstorm/8750-1794543603/content/splash-phosphor.html`, option A).

- [ ] **Step 6: Commit**

```bash
git add apps/viewer/src/splash.rs apps/viewer/src/boot.rs apps/viewer/src/main.rs
git commit -m "Splash: a teal CRT boot screen with the log, the bar and GitHub, Discord and release notes"
```
