# Developing DECKARD

The technical side of DECKARD: building it, how the workspace is laid out, the
command-line tools, and how the main parts work. For what DECKARD is and how to use it,
see [README.md](README.md).

## Building and running

The workspace targets the latest stable Rust (pinned in `mise.toml`: 1.99.1, edition
2024) and current crate versions (egui/eframe 0.36, mlua 0.12, cpal 0.18). With
[mise](https://mise.jdx.dev), `mise install` sets up the pinned Rust and tools, and then
the stem separation model.

```sh
cargo build --release --offline -p deckard
./target/release/deckard ["path/to/project.mxprj"]   # or drop a file on the window
```

- **Linux:** needs `pkg-config` and the ALSA headers (`libasound2-dev`).
- **Windows:** needs the Visual Studio Build Tools (C++).
- **MK3:** with one connected, DECKARD drives it directly. Quit Maschine and NI's
  hardware agents (NIHardwareAgent, NIHostIntegrationAgent) first.

`mise run build` builds the whole workspace (debug). The other mise tasks are listed at
the top of `mise.toml`.

## Testing

The full `cargo test --workspace` takes about 45 minutes, mostly the `deckard-stems`
doctests. Don't run it for every change. Test the crates you touch:

```sh
cargo test --offline -p <crate> [filter]      # e.g. -p deckard-doc, -p deckard lfo
```

Run the full suite once, at the end of a branch. CI runs it on all three systems.

- **Formatting:** `cargo fmt` uses `rustfmt.toml` (140 columns). CI's lint step runs
  `cargo fmt --all --check`, together with actionlint on the workflows and shellcheck
  on the scripts (`mise run lint`).
- **Tests that need local data:** tests that need a Maschine install or projects skip
  themselves when it isn't there.
- **Settings:** tests use a settings folder of their own, never the user's.

## The workspace

| Path | Crate | What it is |
|---|---|---|
| `apps/viewer` | `deckard` | The app: the egui UI, audio and MIDI I/O, recording, the MK3 and Launchpad wiring |
| `apps/player` | `deckard-player` | Command-line player and renderer |
| `crates/mxprj` | `mxprj` | Reader and writer for `.mxprj`, `.mxgrp`, `.mxsnd`, `.mxinst`, `.mxfx` |
| `crates/model` | `deckard-model` | Typed project model (songs, groups, sounds, patterns, scenes, sections, LFOs, routes) |
| `crates/doc` | `deckard-doc` | The open project: one shared, editable copy with undo; the DECKARD block |
| `crates/engine` | `deckard-engine` | Audio engine: Sampler, drum synths, effects, LFOs, modulation, export |
| `crates/plugins` | `deckard-plugins` | VST3 hosting, and DECKARD's own plug-ins (Replicant, Plaits) |
| `crates/mi` | `deckard-mi` | Mutable Instruments' Plaits, vendored C++ with a small bridge |
| `crates/link` | `deckard-link` | Ableton Link, vendored C++ |
| `crates/surface` | `deckard-surface` | DECKARD's own controller layer: Lua pages for the MK3, the Launchpad script |
| `crates/lua-host` | `deckard-lua-host` | Runs Maschine's own controller scripts, when the user supplies them |
| `crates/mk3` | `deckard-mk3` | USB driver for the MK3: pads, buttons, encoders, LEDs, displays |
| `crates/nwl` | `deckard-nwl` | Draws Maschine's display widget trees, for the Maschine-scripts mode |
| `crates/library` | `deckard-library` | The browser's library: NI's index (read-only) and DECKARD's own |
| `crates/stems` | `deckard-stems` | Stem separation (Demucs v4 through demucs-rs) |
| `crates/stretch` | `deckard-stretch` | Time-stretching and pitch-shifting (Signalsmith Stretch, vendored) |

## Documentation map

- **Project format:**
  - [docs/mxprj-format.md](docs/mxprj-format.md): the file format.
  - [docs/deckard-block.md](docs/deckard-block.md): DECKARD's own data in a project
    (expression, routes, LFOs, CC routes, Pad Touch, …).
- **Engine and sound:**
  - [docs/engine-design.md](docs/engine-design.md): how the engine works.
  - [docs/sampler-dsp.md](docs/sampler-dsp.md) and
    [docs/sampler-pages.md](docs/sampler-pages.md): the Sampler.
  - `docs/synth-*.md`, `docs/fx-*.md` and
    [docs/drum-engines-own.md](docs/drum-engines-own.md): one file per drum synth and
    effect.
  - [docs/native-plugins.md](docs/native-plugins.md): DECKARD's own plug-ins and how to
    write one (Replicant is the worked example).
  - [docs/mutable-instruments.md](docs/mutable-instruments.md): Plaits.
- **Modulation and expression:**
  - [docs/modulation.md](docs/modulation.md): automation, LFOs, envelopes, the mod
    matrix, MIDI CC routes.
  - The specs in `docs/superpowers/specs/` (MPE capture, Pad Touch, expression out, the
    expression editor, LFOs, LFO envelopes).
- **Controllers:**
  - [docs/controller-api.md](docs/controller-api.md): the Lua API for DECKARD's
    controller scripts (MK3 pages, themes, MIDI devices).
  - [docs/launchpad-pro-mk3.md](docs/launchpad-pro-mk3.md): the Launchpad Pro MK3 script
    and its hardware checklist.
  - [docs/mk3-usb.md](docs/mk3-usb.md): the MK3's USB protocol.
  - [docs/lua-mods.md](docs/lua-mods.md): mods.
- **The app:**
  - [docs/ui-layout.md](docs/ui-layout.md), [docs/mixer-layout.md](docs/mixer-layout.md)
    and [docs/sampling-layout.md](docs/sampling-layout.md): the software's layout.
  - [docs/context-menus.md](docs/context-menus.md), [docs/browser.md](docs/browser.md),
    [docs/note-repeat-arp.md](docs/note-repeat-arp.md), [docs/scales.md](docs/scales.md)
    and [docs/stems.md](docs/stems.md).
- **Recording and channels:**
  [docs/recording-software.md](docs/recording-software.md),
  [docs/recording-manual.md](docs/recording-manual.md),
  [docs/recording-notes.md](docs/recording-notes.md),
  [docs/channel-software.md](docs/channel-software.md) and
  [docs/channel-manual.md](docs/channel-manual.md).
- **Status:**
  - [docs/PARITY.md](docs/PARITY.md): what Maschine has that DECKARD lacks.
  - [docs/FEATURES.md](docs/FEATURES.md): enhancements beyond Maschine.
  - `docs/BUGS.md` and `docs/CONTROLLER-BUGS.md`: known bugs.
- **Plans:** `docs/superpowers/plans/` holds the implementation plans.

## Releases and CI

Each script builds a release version and packages it in `dist/`. The version defaults
to the crate's and is shown in About DECKARD.

| System | Command | Output |
|---|---|---|
| macOS | `scripts/build-macos.sh [version]` (`mise run release:macos`) | `DECKARD-<version>-macos-universal.zip`: a universal DECKARD.app, signed ad hoc or with `MACOS_SIGN_IDENTITY`, and notarized with `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` |
| Linux | `scripts/build-linux.sh [version]` (`mise run release:linux`) | `deckard-<version>-linux-<arch>.tar.gz`: the binary, a .desktop entry and the MK3 udev rule (`scripts/linux/`); and a `.deb` and an `.rpm` that install them, with the icon and the `.mxprj` file type (`scripts/linux/nfpm.yaml`, built with [nfpm](https://nfpm.goreleaser.com), which mise fetches) |
| Windows | `scripts\build-windows.ps1 [-Version <version>]` (`mise run release:windows`) | `DECKARD-<version>-windows-x64.zip`, signed with `WINDOWS_SIGN_CERT` / `WINDOWS_SIGN_PASSWORD` if set |

- **Stem model:** the packages carry the Demucs installer (`scripts/install-demucs.sh`
  / `.ps1`), not the model. DECKARD runs the installer the first time STEMS is used.
- **Cross-platform check:** `scripts/check-platforms.sh` (`mise run check-platforms`)
  type-checks Linux and Windows from a Mac (rustup and `brew install zig`), to catch
  breakage before CI.
- **CI:** `.github/workflows/ci.yml` builds and tests all three systems on every push
  to main and every pull request. Lint runs on Linux.
- **Release builds:** `release.yml` builds all three when a release is published,
  versioned from the release's name (else its tag; a leading `v` is dropped), and
  attaches them to the release. It can also be run by hand with a version, which keeps
  the builds as workflow artifacts.

Plug-ins are hosted on all three systems; see [Plug-ins](#plug-ins-cratesplugins) for
what each one needs. Windows hosting is type-checked by CI but hasn't been tried with a
plug-in yet.

## The app (`apps/viewer`)

An egui app laid out like Maschine's main window, after the sizes and colours in
[docs/ui-layout.md](docs/ui-layout.md). The drawing is DECKARD's own, with no NI
bitmaps or fonts.

### Command line

```
deckard [file.mxprj] [--screenshot out.png] [--view ideas|song|mix|lfos]
        [--group N] [--sound N] [--pattern N] [--scene N]
        [--play song|scene|pattern] [--launch N] [--tempo BPM] [--wait SECS]
        [--draw] [--piano] [--select-all] [--lane velocity|probability|bend|<CC>]
        [--lanes-dialog] [--plugin-window] [--input FILE]
        [--scripts DIR] [--controller M3] [--press BUTTON_X]... [--hold BUTTON_X]... [--pad N]...
deckard --write-icon OUT.png [SIZE]     # draw the app icon and exit
deckard --goodbye OUT.png               # draw what the MK3's screens show after closing
```

- **`--screenshot`** renders, saves a PNG and exits: after 2 s when playing, or after
  `--wait SECS`. The window is 1500 points wide (wider with the controller panel).
- **`--input FILE`** feeds scripted pointer input, lines of
  `<seconds> <move|down|up> <x> <y>` in window points. It's the way to reach UI state
  that has no flag, and to check the app with plug-ins loaded (they're only created on
  the main thread, so the UI tests run without them).
  - Example: `deckard file.mxprj --group 7 --draw --input clicks.txt --wait 8 --screenshot out.png`.
- **`--launch N` and `--tempo BPM`** act one second after start-up:
  - `--launch N` queues scene N;
  - `--tempo BPM` changes the tempo as an undoable edit.
- **The controller panel:**
  - `--scripts DIR` (or `$DECKARD_SCRIPTS`) points at a folder of Maschine's own
    controller scripts from your install, and adds an on-screen MK3 panel run by them.
    Without them, DECKARD's own controller layer runs the MK3.
  - `--press`, `--hold` (latch) and `--pad` operate that panel at start-up, in order.
- **The settings folder:** `$DECKARD_SETTINGS`, else `~/Library/Application
  Support/deckard` (macOS), `%APPDATA%\deckard` (Windows) or `$XDG_CONFIG_HOME/deckard`
  / `~/.config/deckard` (Linux). It holds settings, the plug-in list, recent projects,
  the session backup, the controller scripts folder and the stem model.
  - Point `$DECKARD_SETTINGS` at a scratch folder for screenshots and experiments, so
    the real recent list and session aren't touched.
- **`DECKARD_MENU_DUMP=1`** prints the macOS menu bar as installed.

The screenshots in `docs/screenshots/` were made this way: with `--view`, `--group`,
`--sound` and `--piano`, and with `--input` clicks for the MOD tab, the route editor
and adding Plaits.

### Plug-in scanning

Preferences ▸ Plug-ins:

- **Manager:** choose whether to use VST3, VST2 and Audio Units, rescan at start-up,
  switch individual plug-ins on and off, and Rescan.
- **Location:** the folders searched for VST plug-ins.

Scanning runs in the background. VST3 bundles without a `moduleinfo.json` are loaded
in a helper process (`deckard --scan-vst3 <bundle>`), so a crashing plug-in can't take
the app down.

## Project files (`crates/mxprj`)

Reader and writer for `.mxprj`, `.mxgrp`, `.mxsnd`, `.mxinst` and `.mxfx`; see
[docs/mxprj-format.md](docs/mxprj-format.md).

```sh
./target/release/mxprj-dump "path/to/project.mxprj" [--decode] [--notes] [--hex 64]
./target/release/mxprj-coverage [--ext mxgrp] [--show-file]   # decode all local documents, rank stopping points
./target/release/mxprj-roundtrip [--ext mxprj] [dir ...]      # decode + re-encode all local documents, compare bytes
./target/release/mxprj-set in.mxprj out.mxprj [--tempo 97.5] [--name NAME]   # write an edited copy
./target/release/mxprj-diff before.mxprj after.mxprj            # what changed, as tree paths
./target/release/mxprj-chunks file.mxprj [--out DIR]            # list hosted plug-ins, dump their state
cargo test --offline -p mxprj --release   # parses and round-trips every document under $MXPRJ_CORPUS (default /Users/Shared)
```

Writing:

- **How it works:** the type loaders run in reverse (`Project::encode`,
  `decode::encode`). Each field is taken from the tree in order and written, and boost's
  class ids, preambles and object ids are re-created as it goes.
- **Round trip:** every local document comes back byte for byte. At last count that was
  119 projects, 1,406 groups, 1,872 sounds and 286 instrument/effect presets.
- **Edits:** an edited tree gets fresh, consistent object ids. Maschine 3 opens a
  project edited this way and loads all of its plug-ins.
- **The DECKARD block:** DECKARD's own data (per-note expression, channel pressure
  lanes, Pad Touch, mod routes, LFOs, CC routes, PROB LOCK, screen state) goes in an
  extra root item, domain `DKRD`, id `EXPR`. Maschine 3 opens such a project and
  ignores the block, and drops it if it re-saves. The block is only written when there's
  something in it, so other projects save byte for byte as before. See
  [docs/deckard-block.md](docs/deckard-block.md); `crates/doc/examples/expression-test-file.rs`
  writes a test file.

## Project model (`crates/model`)

```sh
./target/release/project-summary "path/to/project.mxprj"      # groups, patterns, scenes, arrangement
```

`crates/model` turns a decoded project into typed songs, groups, sounds, patterns
(notes in ticks, 960 per quarter note), scenes and sections. It also models DECKARD's
own parts: `mod_matrix` (routes: source, low/high/curve, target), `lfo` (16 LFO and
envelope slots) and `pad_touch`.

## The open project (`crates/doc`)

`deckard-doc`'s `Document` is the one editable copy of the project. The app, the
playback engine and the controller layers all share it (`Rc<RefCell<Document>>`).

- **Source of truth:** the decoded `mxprj` tree. Saving writes it back with the encoder,
  so parts of the file DECKARD doesn't understand survive edits.
- **Edits:** a change is a list of `Op::Set { path, value }` on the tree, recorded with
  its inverse for undo.
- **Merging:** repeated changes to the same parameter within 1.5 s merge into one undo
  step, so a knob turn is one step. Recording and modulation takes are one step each.
- **Keeping the model current:** `Project::read` reports where each part lives in the
  tree (`Addresses`).
  - Simple parameter edits patch the typed model in place (about 6 µs).
  - Anything else rebuilds it (about 55 ms for a 250k-object project).
  - Tests check that the patched model always equals a rebuild.
- **Revision:** `revision()` goes up with every change, undo and redo. The app then has
  the engine take up the new project mid-playback with `Engine::adopt`.
  - The play position is kept.
  - Sounding notes follow level and pan changes.
  - Muted sounds fade out.
- **Controllers:** parameter writes from the controller layers are document edits too.
  A tempo change made on the MK3 reaches the app and the engine, and can be undone.

## Playback (`crates/engine`, `apps/player`)

```sh
./target/release/deckard-player file.mxprj                    # play the arrangement
./target/release/deckard-player file.mxprj --pattern 0 1 --loop
./target/release/deckard-player file.mxprj --render out.wav [--scene N] [--solo G[:S]]
./target/release/deckard-player file.mxprj --events           # note events in seconds
./target/release/sample-survey [/Users/Shared]                 # do the samples resolve locally?
./target/release/deckard file.mxprj --play song|scene|pattern [--scene N] [--launch N]
```

- **The Sampler** follows the spec in [docs/sampler-dsp.md](docs/sampler-dsp.md):
  - zones by key and velocity (overlapping zones layer), the 8-point resampler, sample
    start, reverse, play range, loops with crossfade and zone fades;
  - the amp envelope (one-shot / AHD / ADSR), the modulation envelope and the LFO;
  - Comp, Drive, Lo-fi, the LP2/BP2/HP2 filters and the EQ band;
  - velocity, mod wheel, pitch bend and sustain pedal;
  - polyphony with voice stealing, legato and glide.
- **Drum synths, Bass/Poly Synth and effects** are DECKARD's own DSP, in
  `crates/engine/src/synth` and `crates/engine/src/fx`, one doc each in `docs/`.
- **Group and master chains:** group buses and the master chain play.
- **Modulation:**
  - Pattern modulation (automation) is mapped through each knob's scale (`modscale.rs`).
  - Routes and LFOs run on the audio thread (`mod_rt.rs`, `lfo_rt.rs`).
  - Each route's travel is published lock-free (`meters::RouteTravels`) so knobs can
    show where a route has them.
- **Expression out:** recorded and live per-note expression goes out to MIDI ports, as
  poly aftertouch on normal ports or per-note channels on MPE ports.
- **Library samples** are found through the `ContentDir` NI installers write to
  `/Library/Preferences/com.native-instruments.<library>.plist` (macOS).
- **Exit:** the app and player end with `deckard_plugins::exit`, which skips plug-ins'
  exit-time destructors (Massive's and Reaktor's crash in them).

## Ableton Link and MIDI clock (`crates/link`)

The header's LINK joins an Ableton Link session. That shares tempo and beat phase with
other apps on the network, and the transport too with Start Stop Sync. Preferences ▸
MIDI ▸ Sync sends MIDI clock to the outputs switched on, or follows the clock of the
inputs switched on. `crates/engine/src/sync.rs` is the engine's side (following a clock
each audio block, sending clock) and `apps/viewer/src/transport_sync.rs` is the app's.

- **Licence:** Link is Ableton's own code (3.1.5, with its C wrapper `abl_link` and
  asio's headers), vendored in `crates/link/vendor` and built with the `cc` crate.
  - Link is GPL-2.0-or-later (`crates/link/vendor/link/LICENSE.md`); asio is under the
    Boost Software License.
  - A DECKARD built with Link is distributed under the GPL, and the About box says so.
- **Build flags:** `crates/link/build.rs` skips the C++ when the target's OS isn't the
  host's (`mise run check-platforms`), or with `DECKARD_LINK=off`. The crate then runs
  a session of one that never goes on the network.
- **Tests** never join a real session (`Link::local`). `crates/link/tests/link.rs`'s
  `two_meet` does, but only with `DECKARD_LINK_NETWORK_TESTS=1`, because it would change
  the tempo of any session on the network.

## Plug-ins (`crates/plugins`)

### Hosting

Sounds whose instrument or effect is a VST or VST3 plug-in (Maschine's `PluginHost`
module) play through the installed VST3, restored to the state Maschine saved.

- **Per system** (`module.rs` loads the bundle, `editor.rs` makes the window):
  - **macOS:** the bundle is loaded with `CFBundle`; the editor gets an `NSWindow`.
  - **Linux:** the bundle's `Contents/<uname -m>-linux/<name>.so` is `dlopen`ed
    (`RTLD_LAZY`, as Steinberg's loader) and its `ModuleEntry` run. The editor gets an
    X11 window of its own (`x11rb`, on its own connection, so XWayland serves it on a
    Wayland desktop), and the host's `IPlugFrame` is also the plug-in's
    `Linux::IRunLoop` (`runloop.rs`): the plug-in registers its X connection and its
    timers there, and `Editor::tick` serves them every frame. Plug-ins can't draw
    without it (VSTGUI, JUCE, nih-plug and yabridge all use it), which is why the app
    repaints every 16 ms while a window is open. Folders: `~/.vst3`, `/usr/lib64/vst3`,
    `/usr/lib/vst3`, `/usr/local/lib64/vst3`, `/usr/local/lib/vst3`.
  - **Windows:** the bundle's `Contents/x86_64-win/<name>.vst3` DLL (or an old
    single-file `.vst3`) is loaded and its `InitDll` run. The editor gets a window of
    the `DeckardPlugin` class; the app's winit loop dispatches its messages, and
    `WM_DPICHANGED` is passed on through `IPlugViewContentScaleSupport`. Folders:
    `%LOCALAPPDATA%\Programs\Common\VST3` and `Common Files\VST3`.
  - `cargo run --release -p deckard-plugins --example plugin-window <bundle.vst3>`
    opens a plug-in's window on its own, to try a system without a project.

- **Formats:**
  - VST3 is hosted directly.
  - VST2 plug-ins are hosted through their VST3 versions. By Steinberg's convention the
    VST3 class id is `"VST" + <VST2 id> + <name>`, and NI's VST2 and VST3 states are
    the same format.
  - Audio Units are listed but not hosted yet.
- **Where the state is:** a `PluginHost` stores `plugin_id`, `plugin_name` and `chunk`
  (`[u32 1][u32 1][u32 n][n bytes]`: a `.vstpreset` for VST3, the plug-in's own state
  for VST2). See `crates/plugins/src/state.rs`.
- **MIDI controllers:** recorded sustain pedal, pitch bend and other controller events
  play back with the notes.
  - VST3 has no MIDI controller messages, so they go to the parameter the plug-in maps
    each controller to (`IMidiMapping`).
  - Stopping lets go of the pedal and centres the bend.
- **Playback:** notes go to the plug-in with their sample position. Its output gets the
  channel's level, pan and mute, and feeds the meters. Instances are kept across edits.
- **Plug-in window:** opening it is macOS only (`editor.rs`). Changes made there are
  stored in the project when the window closes or the project is saved, as one undoable
  "Plug-in Edit". Plug-ins you didn't touch are saved byte for byte as they were.
- **Main thread only:** plug-ins are only created on the main thread (NI's abort
  otherwise).

```sh
cargo run --release --offline -p deckard-plugins --example plugin-state file.mxprj   # load each one
./target/release/deckard file.mxprj --group 7 --sound 0 --plugin-window
```

### DECKARD's own plug-ins

Replicant (`replicant.rs`) and Plaits (`plaits.rs`, around `crates/mi`) are native
plug-ins.

- **Storage:** they're stored as `PluginHost` modules with a `DECKARD:VST3:INSTR:<cid>`
  id. Their state is a `.vstpreset` whose component state is DECKARD's (`native.rs`).
- **Platforms:** they work on every platform.
- **Maschine:** keeps them when it re-saves a project.

[docs/native-plugins.md](docs/native-plugins.md) explains how to write another one.
Plaits' vendored code is MIT (`crates/mi/vendor/LICENSE`, with the upstream commits).

## Controllers

### DECKARD's own layer (`crates/surface`)

The default. DECKARD's own Lua scripts (Lua 5.4) run the MK3. There is one file per
page in `crates/surface/lua/pages/`:

- control, browse, mixer, step, keyboard, lfo, mod_matrix, sampling, …

Alongside them are themes (`themes/`) and devices (`devices/`, such as the Launchpad
Pro MK3).

- **Overriding:** a user's controller folder (Controller ▸ Open Controller Scripts
  Folder) overrides any file by path, and DECKARD reloads a script when it's saved.
- **API:** see [docs/controller-api.md](docs/controller-api.md).
- **Tests:** Rust tests drive the shipped scripts with made-up MK3 events.

### Maschine's own scripts (`crates/lua-host`)

Maschine's hardware behaviour is about 102k lines of Lua calling about 1,640 `NI.*`
engine functions. `deckard-lua-host` runs those scripts unmodified, against a Rust
implementation of that API.

- **Loading:** the scripts (and a `constants.tsv` of their constants) must come from the
  user's own Maschine install, through `--scripts DIR` or `$DECKARD_SCRIPTS`.
  - Preferences ▸ Hardware ▸ Controller chooses DECKARD, Maschine scripts or Automatic.
  - NI's scripts are never committed or bundled.
- **Coverage:** every controller script boots and ticks without errors: MM1, M2, M3,
  MH1071, MS1, MJ1, KKS2_*, KKA and MM3.
- **Implemented in Rust (`src/api`):**
  - project focus and lists;
  - parameters backed by the model or the transport;
  - the transport;
  - the page stack, LEDs, pad/jog/encoder modes;
  - the `NI.GUI` widget tree, drawn by `crates/nwl` from the install's style sheets.
- **Not implemented:** recorded stubs (see `prelude.lua`) keep the scripts running, and
  `lua-host-run` lists the most-used ones.
- **Sandbox:** scripts get no `io` or `os`, and no C modules.

```sh
./target/release/lua-host-run <scripts dir> --controller M3 --project file.mxprj \
    --press BUTTON_TRANSPORT_PLAY --pad 3 --screen PageStack --eval 'return NI.HW.PAGE_CONTROL'
./target/release/deckard file.mxprj --scripts <scripts dir> [--controller M3] [--press BUTTON_GROUP_B]...
```

### The MK3 driver (`crates/mk3`)

The pads, buttons and encoders come over HID; on macOS the device is opened shared with
NI's agent. The two 480×272 displays are driven over the bulk interface, which NI's
NIHostIntegrationAgent claims while it runs. See [docs/mk3-usb.md](docs/mk3-usb.md).

## Roadmap

What's still open lives in two lists:

- **[docs/PARITY.md](docs/PARITY.md)**, what Maschine has that DECKARD lacks:
  - audio recording onto tracks;
  - Replace-mode recording;
  - assigning mod routes from the MK3;
  - group and master modulation targets;
  - per-note expression to MPE plug-ins;
  - per-voice response in DECKARD's own sounds.
- **[docs/FEATURES.md](docs/FEATURES.md)**, enhancements beyond Maschine:
  - themable controller screens and software;
  - a Raspberry Pi appliance;
  - a browser-based controller;
  - an on-screen MK3;
  - Lua MIDI plug-ins;
  - per-note ratchets and parameter locks;
  - multitrack export.
- **Also missing:** plug-in hosting on Linux and Windows, and Audio Unit hosting.

## `out/`

`out/` is git-ignored scratch space. Don't commit or redistribute anything in it.
