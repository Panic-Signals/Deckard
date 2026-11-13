# Startup splash

## Why

DECKARD takes a few seconds to start, and for most of that time nothing on screen says
it's working. Two stretches block the main thread with nothing drawn:

1. **Before the first frame.** The `run_native` creation closure (`apps/viewer/src/main.rs`,
   `main()`) reads settings, starts the MK3 and Launchpad threads, loads the plug-in
   catalog, builds the controller surface and opens the last project (`start_project`),
   all before the window shows anything.
2. **Just after the UI appears.** Two frames in, `load_plugins` instantiates every
   plug-in in the project in one call (`preload_plugins` in `App::update`). The UI sits
   frozen on "Loading plug-ins…" until it returns; with Kontakt or Massive this is
   probably most of the wait.

The splash tells people DECKARD is loading, step by step, so it never looks hung.

## Goals

- A small splash window appears immediately on launch, before anything slow happens.
- It shows **real** progress: one log line per startup step that actually ran, a
  progress bar, and the step in progress.
- The main window opens behind it, then the last project and its plug-ins load; the
  splash closes once the project is fully loaded (plug-ins included).
- The splash carries three buttons that open GitHub, Discord and the release notes.
- It looks like a Blade Runner–era CRT terminal booting up, in teal.

## Non-goals

- Making startup faster. The same work runs; it's just split up and shown.
- A splash on the MK3's displays (Maschine has one, `WelcomeSplashPage`; a later change).
- A Preferences switch to turn the splash off (Maschine has one,
  `IsSplashScreenActiveOnStartup`; easy to add later).
- A splash when opening a project after startup (File ▸ Open). The existing loading
  overlay covers that.

## Anti-goals

- **A fake progress bar.** No timed animation pretending to load; the bar only moves
  when a step finishes.
- **A splash that hides what it's stuck on.** Each step's log line is drawn on the frame
  *before* the step runs, so if one call blocks (a single Kontakt instance,
  `Document::open` on a big project), the screen already says what's loading. One
  indivisible main-thread call freezing the cursor for its duration is accepted
  (builder's-call note: the alternative, a splash in another process, isn't worth it).
- **Changing what tests and screenshots see.** `--screenshot` runs and tests don't show
  the splash and load as today.

## Prior art: how Maschine 3 does it

Observed on Maschine 3.7.1 (macOS) by polling the window list every 100 ms during a
launch, with screenshots:

| t      | Window event |
|--------|--------------|
| 1.1 s  | 482×338 borderless splash, centred on the main display, floating level (above normal windows) |
| 2.6 s  | Main window appears behind it, full size, titled "New Project" |
| 3.9 s  | Main window's title becomes the last project's name: the project is in |
| 4.0 s  | Splash hidden |

Maschine's splash (`DialogSplashScreen`, styled by `Maschine/Dialogs/SplashScreen.txt`)
is a static picture with the version, one status line ("Waiting for product scan …",
"Initializing GUI …") and the copyright. No progress bar, no buttons.

DECKARD follows the same window sequence with its own look and real per-step progress.

## Approaches

The decision that mattered was where the loading work runs, since plug-ins must be
instantiated on the main thread (`deckard_plugins::mark_main_thread`), which is also the
thread that draws.

- **A. The splash drives startup (chosen).** Startup becomes a list of steps; one step
  runs per frame, and plug-ins load one per frame. Between steps both windows repaint,
  so the splash animates and the log is real. Cost: reworking the creation closure and
  `load_plugins` into incremental steps.
- **B. Splash only for the first stretch.** Cheaper, but plug-ins would still load in one
  blocking call after the splash closes. That's likely the freeze people notice, so it
  misses the point.
- **Dropped: animated bar beside today's startup.** The main thread is blocked during
  loading, so the bar and log would freeze with it.

For the window, three shapes were considered:

- **Separate splash window over a visible main window (chosen)**, as Maschine does. The
  main window is never hidden, which avoids winit/eframe not redrawing an invisible root
  viewport.
- One window that starts small and borderless, then grows into the main window. Works,
  but isn't what was asked for.
- Splash drawn inside the main window. Simplest, but isn't a splash window.

## Design

### Window sequence

1. The app starts. The root viewport (the main window) is created as today, at its
   normal size and position.
2. On the first frame, the splash opens as a second viewport (`show_viewport_immediate`)
   with no decorations, always on top, not resizable, centred on the monitor the main
   window is on, and with a fixed size (builder's call; about 760×480 logical so the log
   and ASCII buttons fit). The main window draws behind it: an empty project until the
   real one is in.
3. Each frame runs the next startup step (below) and repaints both windows.
4. After the last step, the splash shows `READY` and closes on the next frame. If the
   whole sequence took under about one second, the splash stays up until one second
   has passed, so it doesn't just flash.
5. Dialogs that today appear at startup ("DECKARD was updated to …", the stem
   separation offer, a project error) appear after the splash closes, not under it.

### Input while loading (decided: blocked)

Until the last step finishes, the main window and both controllers ignore input:
clicks, keys, menus, drag and drop, and MK3/Launchpad presses (the MK3 can show the
existing "loading" overlay from #146). Nothing can start playback, edit the project or
open another one mid-load. A project file opened during loading (drag and drop, or the
OS's open-file event) is held and opened once loading completes. Quit stays available.
The splash's own three buttons work throughout.

Closing the splash window by hand (it has no close button, but the OS may allow it) is
the same as reaching the end: the main window keeps loading behind it. Quitting during
startup quits as usual.

### Startup steps

The current closure's work is split into steps, in today's order. Each step has a log
label and reports OK, a short result (for example `FOUND`, `214 READY`) or `ERR <reason>`.

| Step | Today | Log label (wording is the builder's call) |
|------|-------|-------------------------------------------|
| Settings, recent files, user chords | `Settings::load`, `load_recent` | READING SETTINGS |
| Audio driver | `devices::set_driver` | AUDIO DRIVER |
| Update check (starts a thread) | `update::Notice::start` | CHECKING FOR UPDATES (result not awaited) |
| MK3 | `Mk3Io::start` | MASCHINE MK3: FOUND / NOT CONNECTED |
| Launchpad | `JamIo::start` | LAUNCHPAD PRO MK3: FOUND / NOT CONNECTED |
| Controller surface | `new_surface` | CONTROLLER SURFACE |
| Plug-in catalog | `Catalog::load` | PLUG-IN CATALOG: n READY (or SCANNING, in the background) |
| Open project | `start_project`, or `open` + `remember` when a path was passed on the command line (how a double-clicked file arrives on Windows and Linux) | OPENING `<name>` / NEW PROJECT / RESTORING BACKUP |
| Audio output (only when the project has plug-ins, as today) | `open_output` (today inside `load_plugins`) | AUDIO OUTPUT `<device>`; on failure `ERR`, and the plug-in steps are logged as skipped |
| Each plug-in | one `plugins.instance(...)` call | LOADING PLUG-IN `<name>` [i/n] |
| Done | | READY |

Things that must happen before the first frame stay in the closure: `layout::style`,
the app icon, and command-line handling for non-splash runs. The MK3/Launchpad threads
could instead start in the closure if a later step depends on them; the order above is
today's and should be kept unless there's a reason.

`load_plugins` becomes incremental: it builds the `wanted` list once, then the step
runner instantiates one entry per frame, collecting failures. The final status line
("n plug-ins loaded (x s)" or "plug-ins failed: …") is set as today when the list is
done. After startup, every other caller of `load_plugins` (opening a project later)
keeps today's behaviour; whether it also goes incremental is the builder's call.

### Progress bar

The bar's position is steps done ÷ total steps, where the plug-in count is known once
the project is open (until then, the total uses the fixed steps only, so the bar may
step back slightly when plug-ins are added; the builder may weight plug-ins instead).
Below or beside the bar: the current step's label.

### Look

Teal phosphor, as in the chosen mockup (`.superpowers/brainstorm/…/splash-phosphor.html`,
option A), matching the MK3 goodbye screen (`goodbye.rs`):

- Near-black teal background, radial vignette, faint horizontal scanlines, soft glow on
  text, rounded "tube" corners.
- Header row: `PANIC SIGNALS // DECKARD BOOT SEQUENCE` left, version and clock right.
- `deckard` in the logo font (`layout::logo_font()`, the built-in Blade Runner Movie
  Font or the user's `logo.ttf`), lowercase as the header spells it, because the font's
  capital R is the whole Blade Runner logo. Under it: `GROOVEBOX · SAMPLER · SEQUENCER`.
- The boot log in a monospace font: `> LABEL ........ RESULT`, dim prompt and dots,
  bright results; the current line ends in a blinking block cursor. When there are more
  lines than fit, the oldest scroll off the top.
- Text bar: `[████████░░░░]  58%`.
- Three buttons, ASCII-art lettering (figlet "small" style) in a thin teal frame that
  brightens on hover: GITHUB, DISCORD, NOTES, captioned `[ SOURCE ]`, `[ COMMUNITY ]`,
  `[ RELEASE NOTES ]`.

### Links

- GitHub: `https://github.com/Panic-Signals/Deckard`
- Discord: `https://discord.gg/SGSpFaUnNh`
- Release notes: `https://github.com/Panic-Signals/Deckard/releases/tag/<VERSION>` when
  `VERSION` parses as a release version (`github::parse_version`), else
  `https://github.com/Panic-Signals/Deckard/releases/latest`.

Links open in the default browser (as Help ▸ DECKARD Manual does). Clicking one doesn't
close or pause the splash.

### When there's no splash

The splash is for plain launches: no arguments, or just a project path. Any run with
the automation flags (`--screenshot`, `--group`, `--pattern`, `--scene`, `--sound`,
`--view`, `--lane`, `--select-all`, `--press`, `--pad`, `--hold`, `--play`, `--launch`,
`--tempo`, `--input`, `--plugin-window`, `--wait`, `--draw`, `--piano`,
`--lanes-dialog`) and tests skip the splash and run every step at once in the closure,
as today, then apply the flags as today. (`--write-icon`, `--goodbye` and `--scan-vst3`
return before a window exists.) The step
list is shared, so both paths do the same work in the same order.

## Constraints

- egui/eframe 0.36; plug-in instantiation on the main thread.
- Wayland doesn't let apps position windows or keep them on top: there the splash may
  open off-centre or go behind the main window. Accepted.
- The splash must keep repainting (request a repaint each frame while it's up) so the
  cursor blinks and steps advance without input.

## Left to the builder

- Splash pixel size, fonts sizes, exact colours (start from the mockup's
  `#4fe0cf` on `#011716`), scanline/glow technique in egui.
- Exact log wording and result texts.
- How the step runner is structured (enum of steps, queue of closures, etc.) and where
  it lives (a new `splash.rs` is the natural home).
- Whether to weight plug-in steps in the progress bar.
- Tests: at least the step list running to completion headless, the release-notes URL
  choice, and that `--screenshot` runs don't open the splash.
