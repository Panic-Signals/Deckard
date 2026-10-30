# DECKARD's own controller layer, stage 1: design

October 2026. The first stage of the MK3 working with no Maschine install, run by
DECKARD's own Lua scripts that users can read and change.

## Why

Today the MK3's buttons, knobs, screens and LEDs come from NI's controller Lua scripts,
extracted from the user's own Maschine 3 install. Without that install (on Linux, or on a
Mac without Maschine) the MK3 only plays the focused group's pads: no transport, no
groups, no lights, blank screens.

The user decided DECKARD must work standalone, and that its controller layer must be
**Lua scripts users can easily modify**, not code compiled into the app. Later goals
depend on it:

- themable controller screens, separate from the software's themes (FEATURES #2);
- software palettes, each with a matching controller theme (FEATURES #3);
- the Raspberry Pi appliance and the mobile apps (FEATURES #4, #6, #7).

## The shape of it

- **Rust is the engine.** It runs the scripts and gives them a small, documented
  **DECKARD Lua API**:
  - events in: buttons, pads, knobs, the jog wheel, the touch strip;
  - the app's state to read: transport, tempo, groups and sounds with their names,
    colours and mutes, the focus, the Control area's knobs;
  - requests out: play, record, focus, mute, set tempo, turn a knob, and so on;
  - lights: set a button's or pad's LED;
  - screens: draw with the theme's widgets.
- **Lua is the behaviour.** DECKARD ships its own scripts. They decide what every
  button, pad, mode, page, light and screen does.
- **Themes are the look.** The hardware screens draw from theme files (colours, fonts,
  sizes, layout values), separate from the scripts, so a theme can change without
  touching behaviour.

## Clean room

The scripts are written fresh from how the MK3 behaves, which we already know from
running NI's scripts. Nothing of NI's is copied, translated or bundled: no scripts, fonts,
pictures or style sheets. DECKARD's fonts are egui's (Ubuntu Light and Hack), which
DECKARD already ships. NI's scripts stay available for people who own Maschine.

## The target: Maschine, exactly (the user's decision, 2026-10-25)

After trying a first stage with DECKARD's own layouts, the user set the target: DECKARD's
scripts match Maschine's scripts **one for one**. Every page (Control, Mixer, Arrange,
Browse, Pad mode, Keyboard, Chords, Step, Pattern, Scene, Sampling, …), every button and
every screen behaves and looks as it does with Maschine's scripts. **The only difference
is the colour theme.**

### How parity is reached and kept

- **The parity rig** (`apps/viewer/src/parity_tests.rs`). The same MK3 events go to an
  app running Maschine's scripts and to one running DECKARD's, on the same project. The
  rig compares the app's state, the pads' routing, every light and both screens, and
  writes side-by-side pictures.
- **A page is done** when its scenarios match exactly in state, routing and lights, and
  its screens match the Maschine pictures apart from the pictures (below) and font
  rendering. Its scenarios then stay as tests.
- **Clean room.** DECKARD's scripts are written from what the rig shows Maschine doing,
  never by reading and translating Maschine's Lua.

### What can't be identical

- **Pictures** (icons, knob graphics, plug-in thumbnails): DECKARD draws its own
  lookalikes, always, Maschine installed or not (the user's choice).
- **Fonts.** Maschine's screens use Roboto and Roboto Mono, which are open-licensed
  (Apache 2.0). DECKARD ships its own copy from Google's release, not NI's file.

### Themes

- The theme file is the stylesheet: named colours, font roles and named text styles.
  The pages draw only with these names.
- **Maschine**, the default theme, is Maschine's own colours, so with it DECKARD looks
  like Maschine. Other themes (Solarized Dark first) only recolour.
- Preferences ▸ Hardware ▸ *Controller theme* picks one; a user's own theme in
  `themes/` shows there too.

### Running the scripts

- Preferences ▸ Hardware ▸ *Controller*: **Automatic** (the default: Maschine's scripts
  when found, else DECKARD's), **DECKARD** or **Maschine scripts**.
- The scripts ship with DECKARD; a user folder (`<settings>/controller`) replaces them
  file by file, and an edited file reloads as it's saved. Errors keep the last working
  version and show in the status bar and on the screen.
- One file per page (`global.lua`, `pages/*.lua`, `lib/`, `themes/`), and
  `devices/<model>/` for one controller model's own files.

### The order

1. Pad mode, Keyboard, Chords, Step.
2. Control (the resting page).
3. Mixer.
4. Arrange (Ideas and Song).
5. Pattern and Scene.
6. Browse.
7. The rest: Sampling, Note Repeat and Arp, Lock, Perform, Macro, Settings, File,
   Duplicate, Variation, Events.

Until a page is done, Automatic keeps running Maschine's scripts.

### Other controllers

DECKARD drives only the MK3 today. Another model's own files go in `devices/<model>/`.

## Non-goals

- Drivers for other controllers (Mikro MK3, MK2, Studio).
- Themes following the software's palette (FEATURES #3), later.

## Anti-goals

- Controller behaviour compiled into Rust, where users can't change it.
- Layouts or behaviour of DECKARD's own invention: anything that differs from Maschine
  other than the colours.
- Copying or translating NI's scripts, or bundling any of NI's files.
- Changing what NI's scripts do for users who keep them.
- Anything that stops the pads playing at once: pads keep playing from the MK3 thread,
  and the scripts only say where they go.

## Built as

- **The DECKARD Lua API.** It lives in Rust, on the Lua runtime DECKARD already uses
  (mlua, Lua 5.4). It is separate from the NI-API emulation in `crates/lua-host`, and is
  documented in `docs/controller-api.md`.
- **Kept from the first build** (`crates/surface`, uncommitted):
  - the app's snapshot for the layer, and the mapping of its requests onto the app's
    existing handlers;
  - the Controller setting;
  - the screen drawing and fonts;
  - the LED colour matching.
- **Rewritten as Lua:** the first build's Rust button, pad and mode logic.
- **Tests:**
  - Rust tests drive the shipped scripts with made-up MK3 events, and check the
    requests, LEDs and screens that come out.
  - A test loads a user override folder.
  - A test checks that a script error keeps the last working version.

## Left to the builder

- The API's exact function names, recorded in `docs/controller-api.md`.
- Pixel layout and text sizes within the theme.
