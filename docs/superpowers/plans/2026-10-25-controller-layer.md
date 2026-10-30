# DECKARD controller layer (Lua), stage 1: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The MK3 runs on DECKARD's own Lua scripts (one file per page), which users can
override file by file, with themable screens.

**Architecture:** `crates/surface` keeps its public Rust API (`Surface::event`, `pads`,
`leds`, `screens`, `View`, `Action`), but the behaviour moves into Lua. The crate embeds
the shipped scripts (`crates/surface/lua/`), loads an optional user folder over them,
gives the scripts the `deckard` API, and draws what they ask for. The app side (setting,
`surface_view`, `surface_act`) is unchanged from the first build.

**Tech Stack:** Rust, mlua 0.12 (Lua 5.4, vendored), fontdue, epaint_default_fonts 0.36.

**Spec:** `docs/superpowers/specs/2026-10-25-controller-layer-design.md`.

## Global Constraints

- Clean room: the scripts are DECKARD's own, written from behaviour; nothing of NI's.
- Pads keep playing from the MK3 thread; the scripts only say where they go.
- Cargo through mise; targeted tests while building; lint and the full suite at the end.
- Tests don't write the settings file or library folders (user-folder tests use a
  temporary folder).

## Review Focus

1. A script error (on load or in a handler) keeps the last working scripts, and is
   reported (`a_broken_script_keeps_the_last_working_one`).
2. A user file replaces only the shipped file of its name (`a_user_page_replaces_only_its_page`).
3. A page that doesn't handle an event passes it to `global.lua` (`unhandled_events_reach_global`).
4. Lua indices are 1-based, Rust's 0-based: sounds and groups convert at the API edge
   (all action tests).
5. A script that draws nothing, or off the screen, still gives a valid picture
   (`screens_draw_any_project`).

---

### Task 1: The engine and the API

**Files:** `crates/surface/src/{lib.rs,api.rs,names.rs}`, `crates/surface/Cargo.toml` (mlua).

**The `deckard` API** (documented in `docs/controller-api.md`, Task 4):
- Handlers a script file returns (all optional): `button(name, down)`, `pad(n, velocity)`,
  `pad_off(n)`, `knob(n, amount)`, `wheel(steps)`, `pads()`, `leds()`, `draw(screen, theme)`,
  `shown()`. Event handlers return `true` when they handled it.
- State: `deckard.state()` (the `View`, 1-based groups and sounds), `deckard.held(name)`,
  `deckard.pad_held(n)`, `deckard.now()` (seconds), `deckard.page()`.
- Requests: one function per `Action` (`deckard.play()`, `deckard.focus_sound(n)`, …).
- Pages: `deckard.show_page(name)`.
- Lights: `deckard.led(name, level)` for buttons (`"off" | "dim" | "on"`), and
  `deckard.led(name, colour, bright)` for pads and groups.
- Names are the MK3's labels: `PLAY`, `STOP`, `REC`, `RESTART`, `ERASE`, `TAP`, `AUTO`,
  `SHIFT`, `GROUP_A`…, `PAD_1`…, `SCREEN_1`…, `WHEEL_UP`…, `FIXED_VEL`.

- [ ] The first build's behaviour tests (`src/tests.rs`, `leds.rs`) run unchanged against
  the shipped Lua.
- [ ] New: `unhandled_events_reach_global`, `a_user_page_replaces_only_its_page`,
  `a_broken_script_keeps_the_last_working_one`, `edited_user_scripts_reload`.

### Task 2: The shipped scripts

**Files:** `crates/surface/lua/{global.lua,pages/control.lua,lib/util.lua,themes/dark.lua}`:
the stage-1 behaviour from the spec, written fresh.

### Task 3: Screens from the scripts

**Files:** `crates/surface/src/screen.rs`: the `screen` userdata (`rect`, `text`,
`text_width`), fonts by name (`ubuntu`, `hack`); the picture sent only when it changed.

- [ ] `screens_redraw_only_on_change`, `screens_draw_any_project`, `the_header_shows_text`;
  an error shows on the screen (`a_script_error_shows_on_the_screen`).

### Task 4: The app and the docs

- The app passes the user folder (`<settings folder>/controller`), reports script errors
  in the status bar, and has Help ▸ Open Controller Scripts Folder.
- `docs/controller-api.md`: the API, the file layout, how to override a page.
- [ ] `the_surface_plays_and_focuses`, `choosing_deckards_layer_stops_the_scripts`,
  `controller_setting_round_trips`; lint; full suite.

### Task 5: Parity with Maschine's own scripts

The user's requirement: every page, button, light and screen behaves as Maschine's
scripts do; only the theme may differ. The yardstick is `apps/viewer/src/parity_tests.rs`,
which runs NI's scripts and DECKARD's side by side (`PARITY=name` for one scenario,
`PROBE="press:BUTTON_X …"` to see what Maschine does).

Done (state, lights and routing match in every scenario but the playback-timing
jitter of `play*`):

- Control, Pad mode (choke/link), Keyboard, Step (overview), the MUTE/SOLO/SELECT
  page, Mixer, Arrange (Ideas view), Pattern and Scene (held), Browse
  (`docs/controller-browse-page.md`).
- Knobs step as Maschine's do (`lib/knobs.lua`); the group buttons are the focused
  group's bank of eight (SHIFT + a group button: that bank).
- Themes can build on another (`base`); `solarized_dark` builds on `maschine`.

Left:

- Pages: Step's detail editor, Sampling, Note Repeat/Arp, Lock, Perform, Macro,
  Settings, File, Duplicate, Variation, Events, the Song view.
- Gaps on finished pages: Keyboard's scale and chord knobs; Pad mode's page 2; the MUTE
  page's bank / ALL ON / NONE / AUDIO; Mixer pan; Browse's categories, banks, LOCATE,
  ERASE, the SHIFT options for groups and the knob-touch lists; the Control page's
  button 8 (the plug-in browser).
