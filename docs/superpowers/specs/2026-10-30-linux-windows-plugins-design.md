# Hosting VST3 plug-ins on Linux and Windows

Status: 2026-10-30. Research into what it takes to lift the "macOS only" limit on
third-party plug-ins, and the plan. **Done the same day:** loaders and folders for
Linux and Windows, the Linux window and run loop (Dexed's JUCE editor draws and
responds), the Windows window (written, not yet run on Windows), the
`plugin-window` example, README and DEVELOPMENT. Still open: the test-fixture plug-in,
and trying Windows with a plug-in.

## Where we stand

`crates/plugins` already hosts VST3 on macOS through the `vst3` crate (coupler-rs,
MIT/Apache-2.0, pre-generated bindings, no Steinberg SDK at build time). Everything that
matters is platform-neutral already: the host (`lib.rs`), the COM helpers (`com.rs`),
state and `.vstpreset` handling (`state.rs`), the scanner's `moduleinfo.json` path and
the `--scan-vst3` helper process, parameter pages, MIDI mapping, the audio-thread
processing. The `vst3` crate carries the Linux and Windows parts of the API we need:
`Steinberg::Linux::IRunLoop`, `IEventHandler`, `ITimerHandler`,
`kPlatformTypeX11EmbedWindowID`, `kPlatformTypeHWND`, and the Windows `tresult`
values (which differ from the other platforms; the crate handles that).

Only four places are gated on `target_os = "macos"`:

| file | what it does on macOS | elsewhere today |
|---|---|---|
| `module.rs` `platform::load` | `CFBundle` load, `bundleEntry`, `GetPluginFactory` | returns "plug-in hosting is only implemented on macOS" |
| `module.rs` / `scan.rs` default folders | `~/Library/Audio/Plug-Ins/VST3`, `/Library/…` | the macOS folders, which don't exist |
| `lib.rs` `on_main_thread` | `pthread_main_np` | always `true` |
| `editor.rs` | an `NSWindow`, `attached(view, NSView)` | returns "plug-in windows are only implemented on macOS" |

So this is a platform-glue job, not a new host.

## Options for the heavy lifting

**1. Keep the `vst3` bindings and write the glue ourselves. Recommended.**
Roughly 500–700 lines across the four places above, with crates already in the
dependency tree: `libloading` (dlopen / LoadLibrary), `x11rb` (pure-Rust X11, no C
headers, pulled in by winit) and `windows-sys` (already a viewer dependency). No licence
change, no C++ bridge, no new system packages for CI or the `.deb`/`.rpm`.

**2. The `vst3-host` crate (MIT, v0.10).** A safe host layer on the same `vst3`
bindings, with optional process isolation (a helper binary, `Error::PluginCrashed`,
`recover()`). Its Linux and Windows window code compiles but has never been run against
a plug-in, it provides no `IRunLoop` (so most Linux plug-in editors would not open), and
it binds `xcb` and `winapi` through C. Adopting it would mean replacing our
Maschine-aware host (`lib.rs`, ~1,000 lines) for no gain on this task. Its process
isolation is the one idea worth borrowing later.

**3. C++ hosting libraries: JUCE, Carla, Steinberg's own hosting classes.** All host
VST3 on Linux and Windows, all mean a C++ bridge and a licence (JUCE: GPL or
commercial; Carla: GPL; the SDK hosting classes: GPLv3 or Steinberg's proprietary
licence). Not worth it for four functions. Steinberg's `editorhost` sample
(`samples/vst-hosting/editorhost/source/platform/linux/runloop.cpp`) is the reference
for how a run loop should behave, and the plan below follows it.

**4. yabridge** is not something we integrate. It is how Linux users run Windows
plug-ins, which is to say NI's, since NI ships nothing for Linux and Maschine projects
mostly reference NI plug-ins. `yabridgectl sync` writes ordinary VST3 bundles into
`~/.vst3/yabridge/`, so a host that handles native Linux VST3 properly gets them for
free. Its proxy queries our frame for `IRunLoop` and falls back to "naive GUI
function execution" when it's missing, so providing the run loop matters for it too.

## What to build

### Module loading (`module.rs`), both platforms, with `libloading`

- **Linux.** A bundle is a directory `Name.vst3` holding
  `Contents/<uname -m>-linux/Name.so` (`x86_64-linux`, `aarch64-linux`). Open it with
  `RTLD_LAZY | RTLD_LOCAL` as the SDK does (libloading's `os::unix::Library::open`),
  by absolute path so the library's `$ORIGIN` rpath works. Call `ModuleEntry(handle)`
  with the dlopen handle (the SDK treats it as required; call it when present), then
  `GetPluginFactory`. Keep the library loaded for the life of the process, as the
  macOS code keeps its bundle; never `dlclose`, and `ModuleExit` is never called
  because `deckard_plugins::exit` skips plug-ins' teardown anyway.
- **Windows.** A directory `Name.vst3` holds `Contents/x86_64-win/Name.vst3` (a DLL;
  `arm64-win` on ARM). A plain file `Name.vst3` is a legacy single-file plug-in and is
  loaded directly. `LoadLibraryW` with no flags, `InitDll` if exported, then
  `GetPluginFactory`.
- **Errors** stay strings as now ("is not a module directory", the `dlerror` text, and
  so on) so the scanner's failure lines read well.

### Standard folders (`module::default_search_paths`, `scan::default_vst2_paths`)

| | VST3 | VST2 (listed only, for the VST2→VST3 mapping) |
|---|---|---|
| Linux | `~/.vst3`, `/usr/lib64/vst3`, `/usr/lib/vst3`, `/usr/local/lib64/vst3`, `/usr/local/lib/vst3` | `~/.vst`, `/usr/lib/vst`, `/usr/local/lib/vst`, each folder in `$VST_PATH` |
| Windows | `%LOCALAPPDATA%\Programs\Common\VST3`, `%ProgramFiles%\Common Files\VST3` | `%ProgramFiles%\VstPlugins`, `%ProgramFiles%\Steinberg\VstPlugins`, `%ProgramFiles%\Common Files\VST2` |

VST2 files are `.so` on Linux and `.dll` on Windows, not `.vst` bundles, so
`scan::vst2_bundles` takes the extension per platform. One level of subfolders is still
searched, which covers `~/.vst3/yabridge/`.

### Main thread (`lib.rs`)

Linux: `gettid() == getpid()` through `libc`. Windows: `GetCurrentThreadId` compared
with the id recorded by a `mark_main_thread()` the app calls first thing in `main`
(macOS keeps `pthread_main_np`).

### The editor window (`editor.rs`)

The shape stays the same as on macOS: `Editor::open` makes a top-level window of its
own, hands it to the plug-in, and `tick()` (already called every frame from
`poll_plugin_editors`) applies whatever the plug-in asked for in between, never from
inside one of its calls. `is_open`, `raise` and `Drop` keep their meanings.

**Windows.**
- Register a window class once; `CreateWindowExW` an overlapped window sized from
  `getSize`, resizable only when `canResize` says so; `attached(hwnd, "HWND")`;
  `ShowWindow`.
- The window procedure handles `WM_CLOSE` (hide and mark closed; the window is
  destroyed in `Drop`, after `removed()`), `WM_SIZE` (note the size for `onSize` on the
  next tick) and `WM_DPICHANGED` (apply the suggested rectangle and pass the scale to
  `IPlugViewContentScaleSupport::setContentScaleFactor`).
- No message loop of our own: winit's loop on the main thread dispatches messages for
  every window on that thread, and winit already does `OleInitialize`, which plug-in
  drag-and-drop needs.

**Linux.**
- Connect with `x11rb`'s `RustConnection` (talks to the X socket directly, works under
  XWayland, needs `DISPLAY`). Create a top-level window, set `WM_NAME`, `_NET_WM_PID`,
  `WM_DELETE_WINDOW`, and `WM_NORMAL_HINTS` to fix the size unless `canResize`. Map
  it, flush, then `attached(window_id, "X11EmbedWindowID")`: the plug-in creates its
  own child window inside ours, on its own X connection.
- **`Frame` also implements `Linux::IRunLoop`** (`Class` interfaces
  `(IPlugFrame, IRunLoop)`), which the plug-in queries from the frame in `setFrame`.
  This is the part no Rust crate provides and the part Linux plug-ins can't do
  without: VSTGUI, JUCE (it asserts when the frame has no run loop), nih-plug and
  yabridge all register their X connection's file descriptor and their timers with
  it.
  - `registerEventHandler(handler, fd)` / `unregisterEventHandler` keep a list of
    `(fd, ComPtr<IEventHandler>)`.
  - `registerTimer(handler, ms)` / `unregisterTimer` keep `(ComPtr<ITimerHandler>,
    interval, next due)`.
  - `tick()` does one `poll()` over the registered fds with a zero timeout and calls
    `onFDIsSet(fd)` for each ready one, fires every timer that is due (`onTimer`) and
    reschedules it, drains our own connection's events (`ClientMessage` with
    `WM_DELETE_WINDOW` marks the window closed; `ConfigureNotify` notes a new size for
    `onSize`), and then applies the pending resize as today. Handlers may unregister
    themselves from inside a callback, so iterate over a snapshot.
  - `Drop` calls `removed()` and `setFrame(null)` first, then drops the handlers, so no
    event reaches the plug-in after its view is gone (JUCE crashes otherwise).
- **Repaints.** egui only redraws on input, so while any editor is open the app must
  `request_repaint_after` a short interval (16 ms is fine) so `tick` keeps driving the
  plug-in's timers. Without that a plug-in's UI freezes whenever the mouse leaves
  DECKARD's window. This is the one change outside `crates/plugins`.
- **Wayland.** VST3 has no Wayland platform type; every Linux host embeds through X11,
  so under a Wayland session the editor window goes through XWayland while DECKARD's
  own window can stay native. Nothing to force in winit.
- **Later, not now:** giving the run loop to plug-ins through the host context as
  well (`IPluginFactory3::setHostContext`), for plug-ins that want timers while their
  editor is closed; keyboard focus hand-off between our window and the embedded one.

### Plumbing

- `apps/viewer/src/settings.rs:260` already seeds the folder list from the defaults;
  the per-platform defaults above make it right everywhere.
- README "What you need" and "Plug-ins", DEVELOPMENT.md ("Not yet on Linux and
  Windows…", "Hosting (macOS)") get updated; the Linux notes mention yabridge for NI
  plug-ins and `DISPLAY` under Wayland.
- CI needs nothing new: `x11rb` without the `xcb` feature has no C dependency, and
  `windows-sys` is already built on the Windows job. The `.deb`/`.rpm` gain no runtime
  dependency.

## Testing

- **Unit:** the run loop, in the style of `a_resize_waits_until_the_plugin_has_returned`:
  a timer fires once it is due and not before; an fd handler is called when its pipe is
  readable; unregistering from inside a callback is safe; nothing is called after
  `Drop`.
- **A plug-in of our own as a fixture.** The `vst3` crate ships `examples/gain.rs`, a
  complete VST3 plug-in in Rust. A tiny `crates/test-plugin` cdylib laid out as a
  bundle in `target/` would let CI load a module, create an instance, set and get
  state and process a block on all three systems with no third-party plug-in installed.
  Optional, but it is the only way to keep the loaders from rotting.
- **By hand.** This machine has no plug-ins and no display session, so the Linux
  editor work needs a desktop session and some native Linux VST3s: Surge XT, Vital,
  Dexed, Odin2 and u-he's free Podolski/TyrellN6 cover VSTGUI, JUCE and custom
  toolkits; nih-plug plug-ins (e.g. Diopser) cover the Rust wrapper; then yabridge
  with an NI plug-in for the real Maschine case. Windows needs a Windows box or VM
  with the same set.

## Order of work

1. Loaders and folders on both platforms, `on_main_thread`, the VST2 extension. After
   this a Maschine project's plug-ins play on Linux and Windows, without windows.
2. The Windows editor window (smaller: no run loop).
3. The Linux window and `IRunLoop`, plus the repaint request in the app.
4. Docs, README, DEVELOPMENT, and the test fixture plug-in.

## Out of scope

- **Hosting VST2 directly.** Still through VST3 versions, as on macOS (the VST2 SDK can
  no longer be licensed; Maschine's own arm64 build doesn't scan VST2 either).
- **CLAP and LV2**, which is what most native Linux plug-ins actually ship as. The
  `clack` and `lilv` crates exist if that ever matters, but a Maschine project can't
  reference them.
- **Process isolation** of running plug-ins (as `vst3-host` does). The scanner already
  isolates loading; crashes while playing are a separate project.
