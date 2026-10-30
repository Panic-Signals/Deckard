# DECKARD controller scripts

DECKARD runs the MK3 with its own Lua scripts (Lua 5.4). They decide what every button,
pad, knob, light and screen does. You can replace any of them with your own.

## Where they live

DECKARD's scripts are built into the app (`crates/surface/lua/`). Your folder is
`<settings>/controller`. On macOS that is `~/Library/Application Support/DECKARD/controller`.
To open it, choose **Controller ▸ Open Controller Scripts Folder**.

```
controller/
  global.lua            always on: transport, groups, pads, jog wheel, the header
  pages/
    control.lua         the Control page (knobs, CHANNEL / PLUG-IN, display buttons)
    module.lua          the plug-in browser (PLUG-IN, the jog wheel pressed)
  lib/
    util.lua            helpers shared by the pages (require("lib.util"))
  themes/
    maschine.lua        the base theme (Maschine's look; the others build on it)
    palenight.lua       a new install's theme
    solarized_dark.lua
  devices/
    mk3/                files for one controller model only
  defaults/             DECKARD's own scripts, refreshed each time the folder is opened
```

### Replacing a script

A file in your folder replaces DECKARD's file at the same path. To change the Control
page:

1. Copy `defaults/pages/control.lua` to `pages/control.lua`.
2. Edit the copy.

DECKARD reloads a script when you save it. Delete your copy to get DECKARD's back.

### Looking up a file

For each script it needs, DECKARD takes the first of these that exists:

1. `devices/mk3/<path>` in your folder;
2. `<path>` in your folder;
3. DECKARD's `devices/mk3/<path>`;
4. DECKARD's `<path>`.

### When a script fails

If a script fails to load, the last working version keeps running. The error appears in
the status bar and across the bottom of the left screen. The pads always play.

## Pages

`global.lua` and each file in `pages/` return a table of handlers. Every handler is
optional.

| Handler | Called when |
|---|---|
| `button(name, down)` | A button is pressed (`down` is `true`) or let go. |
| `pad(n, velocity)` | Pad `n` (1–16, 1 at the bottom left) is hit, with `velocity` 0–1. |
| `pad_off(n)` | Pad `n` is let go. |
| `knob(n, amount)` | Knob `n` (1–8) turns by `amount`, a fraction of a turn (+ is clockwise). |
| `wheel(steps)` | The jog wheel turns by `steps` clicks. |
| `tick()` | As often as DECKARD updates (while anything is held, at least every 30 ms): for what's timed, as Step mode's hold. The page first, then `global.lua`. |
| `touch(finger, kind, pos)` | The Smart Strip: finger `finger` (1 or 2) went `"down"`, `"move"`d or came `"up"` at `pos` (0 left .. 1 right). DECKARD's global.lua hands it to `lib/strip.lua`. |
| `shift_pad(n, velocity)` | `global.lua` only: pad `n` is hit with SHIFT held. Called before any page; return `true` and the page doesn't get the pad. DECKARD's does the shortcuts printed on the pads (Maschine's SHIFT + pad), and with ERASE held in Pad mode removes the pad's sound. |
| `wheel_mode(steps)` | `global.lua` only: the jog wheel turns by `steps` clicks. Called before any page; return `true` and the page doesn't get the wheel. DECKARD's takes it while TEMPO, SWING or VOLUME is on. |
| `restore(page, mode)` | `global.lua` only: a project opened that was saved on `page` with the pads in `mode` (`"pad"`, `"keyboard"`, `"chords"`, `"step"`). DECKARD's switches the pad mode and shows the page, unless it's a dialog or a page shown while a button is held (docs/deckard-block.md, the screen). |
| `pads()` | DECKARD asks where the pads play. Return `{ play = bool, keys = { sound = n, notes = {16 MIDI notes} } }`, or `nil`. |
| `leds()` | DECKARD asks for the lights; set them with `deckard.led`. |
| `draw(screen, theme)` | DECKARD redraws the screens. |

How DECKARD calls them:

- **Events** (`button`, `pad`, `pad_off`, `knob`, `wheel`) go to the page on screen
  first. If the page's handler doesn't return `true`, the event goes on to `global.lua`.
- **`pads()`** asks the page first, then `global.lua`.
- **`leds()` and `draw()`** run for both: `global.lua` first, then the page on top of it.

The page on screen starts as `control`. Call `deckard.show_page("name")` to show
`pages/name.lua`.

The shipped `lib/` modules are shared by every page that requires them:

- `lib/knobs.lua` steps through values as Maschine's knobs do: `knobs.each(amount)`
  takes a step for every turn, and `knobs.gather(n, amount, step)` gathers turns until
  they make `step` of a turn (a tenth if not given).
- `lib/chain.lua` draws a chain as the Control page and the plug-in browser show it:
  `chain.tabs(screen, theme, level)`, `chain.draw(screen, control)` (the slots, the
  focused one framed, then the "+"), `chain.thumbnail(screen, x, y, w, h, instrument)`.
- `lib/wheel.lua` holds `wheel.mode`, which is `"tempo"`, `"swing"` or `"volume"` while
  TEMPO, SWING or VOLUME has the jog wheel. A page's `wheel` handler should return
  nothing while it's set, so the turn goes on to `global.lua`.
- `lib/quick_edit.lua`: after the jog wheel changes the tempo, swing or master volume,
  `widgets.info_bar` shows the value and its name ("3.0 %", "MASTER SWING") in place of
  the page's values until the wheel's mode is off, as Maschine's quick edit.
- `lib/paged.lua` turns a page's knob pages with LEFT / RIGHT (`paged.new(count)`,
  `paged.button`, `paged.leds`, `paged.label` for the "2/4").
- `lib/back.lua` remembers the page another page was shown over: `back.over(page)`
  shows `page` and remembers the one on screen; `back.go()` returns to it (FILE,
  SETTINGS and SHIFT + MACRO use it).
- `lib/pin.lua` keeps the page a button shows while held (`pin.shown`) and the pins
  (`pin.pinned`): `pin.on()` says whether the page shown is pinned, `pin.active(name)`
  whether a button acts as held (held, or its page pinned).

### Pads play by themselves

Pads play their sounds straight from the controller. Scripts can't delay them; a pad
handler only decides what else happens, such as changing the focus. `pads()` decides
where the pads play: return `play = false` to stop them playing, as Maschine does while
PATTERN or SCENE is held (its pads pick patterns and scenes) and in Step mode.
`keys = { sound, notes, chords }` makes every pad play the sound at `notes[pad]` (Keyboard
mode), or each of `chords[pad]`'s notes (Chords mode).

### The LFO and mod matrix pages

- **`lfo`** (SHIFT + SELECT; again, or another page's button, goes back). The left
  screen shows the focused LFO's name large ("Press a pad" with none); the right, its
  `points` drawn as lines under its shape, rate and "trig". Display buttons 1-5 pick the
  shape (SINE, SQR, TRI, SAW, RAND), 6 SYNC, 8 TRIG (the focused sound, or Off); while
  SHIFT is held, 8 reads DELETE and removes the focused LFO (shown disabled with none). Knobs
  1-7: Shape, Rate, Phase, Level, Quantize, Trigger, Fade in (only with a trigger).
  Display button 7 ENV switches the focused LFO to Envelope mode and back (lit in
  Envelope mode); then the left screen reads "ENV n", 1-5 are dark and do nothing, 6
  is RETRIG (lit when on), 8 TRIG sets the trigger to the focused sound (never Off),
  knobs 1-7 are Attack, Decay, Sustain, Release, Level, Quantize, Trigger, and the
  right screen draws the ADSR with a dashed line at `note_off`. The
  pads don't play: an LFO's pad focuses it, an empty one makes one there, ERASE + an
  LFO's pad (or SHIFT + ERASE + it, which here doesn't reset the sound) removes it, and MUTE + an LFO's pad mutes or unmutes it (MUTE held here
  doesn't show the hold page). Pads light dim where an LFO is, bright for the focused
  one; white, or red when it's muted. A muted LFO's name has "MUTED" under it.
- **`mod_matrix`** (SHIFT + SOLO, the same way). Eight of `mod_slots` a page, one a
  column (four a screen), each column its six rows with one highlighted. A display button
  moves its column's highlight down a row (back to Source after Depth); the knob under it
  turns that row (`deckard.mod_slot`); ERASE + the knob removes the slot's route (once a
  press of ERASE). LEFT / RIGHT show the eight slots before or after. The pads play. A
  CC route's column (`midi`) names rows 3–5 PORT, CHANNEL and CC, on the screen and its
  display button.

Both catch SHIFT + their button in `global.lua` before SELECT's and SOLO's held page,
and give way to it as MIX's page does. Whenever the page shown stops being `mod_matrix`,
DECKARD sends `deckard.mod_slots_closed()`'s action itself, so a slot being made goes
however the page is left (a held page over it, tapped or pinned and closed back to it,
keeps it).

## Names

Buttons and lights use the MK3's labels:

| Group | Names |
|---|---|
| Transport | `PLAY`, `STOP`, `REC`, `RESTART`, `ERASE`, `TAP`, `FOLLOW` |
| Modes | `SHIFT`, `AUTO`, `MACRO`, `SETTINGS`, `FILE`, `BROWSE`, `MIX`, `ARRANGE`, `SAMPLE`, `CHANNEL`, `PLUGIN`, `LEFT`, `RIGHT` |
| Wheel modes | `VOLUME`, `SWING`, `TEMPO`, `NOTE_REPEAT`, `LOCK` |
| Pad modes | `PAD_MODE`, `KEYBOARD`, `CHORDS`, `STEP`, `FIXED_VEL`, `SCENE`, `PATTERN`, `EVENTS`, `VARIATION`, `DUPLICATE`, `SELECT`, `SOLO`, `MUTE` |
| Strip | `PITCH`, `MOD`, `PERFORM`, `NOTES` |
| Numbered | `GROUP_A` … `GROUP_H`, `SCREEN_1` … `SCREEN_8` (the display buttons), `PAD_1` … `PAD_16` |
| Encoder | `WHEEL` (pressed), `WHEEL_UP`, `WHEEL_DOWN`, `WHEEL_LEFT`, `WHEEL_RIGHT` |
| Knob touches | `CAP_1` … `CAP_8`: a button that goes down when a knob is touched and up when it's let go (no light) |

## The `deckard` API

Groups, sounds, knobs, pages and display buttons count from 1.

### Reading

| Function | Gives |
|---|---|
| `deckard.state()` | The app now (see below). |
| `deckard.held(name)` | Whether a button is held. |
| `deckard.pad_held(n)` | Whether pad `n` is held. |
| `deckard.now()` | Seconds, for timing (TAP tempo). |
| `deckard.page()` | The page on screen. |
| `deckard.device()` | The controller model: `"mk3"`. |

`deckard.state()` returns a table with these fields:

| Field | What it is |
|---|---|
| `project` | The project's name. |
| `tempo`, `swing` | The tempo, and the swing (0–100). |
| `volume`, `volume_db` | The master level (0–1; 0.75 is unity), and in dB (−∞ silent). |
| `playing`, `recording`, `looping`, `metronome`, `auto_write`, `fixed_velocity`, `follow` | `true` or `false`. |
| `velocity` | The velocity the pads play with Fixed Velocity on (0–127). |
| `scale` | Keyboard and Chords mode's scale: `{ bank, kind }`, places in `deckard.scales`. |
| `grid` | The song's grids, each value its place in Maschine's list from 0: `perform` (0 Off, 1 Bar, 1/2, 1/4, 1/8, 1/16, Section), `arrange` (1 Bar, 1/2, 1/4, 1/8, 1/16) with `arrange_snap` and `quick` (`true`/`false`), `step` (1 Bar … 1/128, then 1 Bar T … 1/64T) with `step_snap` and `step_ticks` (its step in ticks), `nudge` (Step, Step/2 … Step/16). |
| `recent` | The projects opened lately, most recent first (their names). |
| `settings` | What the Settings page shows: `values` (each setting's value as shown, by the name `deckard.setting` takes: `count_in`, `quantize`, `driver`, `device`, `sample_rate`, `buffer_size`, `in_route_1` … `in_route_32`, `out_route_1` …, `midi_sync`, `clock_offset`, `default_input`, `link_start_stop`, `sensitivity`, `display_brightness`, `metronome_volume`, `metronome_auto`), `midi_inputs` and `midi_outputs` (`{ name, on }`), `audio_status` (the device's status line, its three latencies, their total), `link` and `link_peers`. Filled only while the `settings` page shows. |
| `lock` | Lock: `active` (LOCK took a snapshot to recall), `slots` (`slots[n]` is `true` when slot `n` holds one, from 1; 16 a bank), `focus` (the focused slot, or `nil`), `bank` (shown, from 1) and `banks` (1-4), `morph`, and `mode` (0 Travel, 1 Target), `time` (1/2 Bar, 1 Bar, 2, 4, 8, 16 Bars) and `grid` (1/8, 1/4, 1/2, Bar) as places in those lists from 0. |
| `note_repeat` | Note Repeat and the Arp: `settings` by name (`type`, `rate`, `rate_unit`, `sequence`, `octaves`, `dynamic`, `gate`, `retrigger`, `repeat`, `offset`, `inversion`, `min_key`, `max_key`), each `{ name, text, value, min, max, step, turn }` (`turn`: how far a knob turns for a step); `presets` (the four rates' names, display buttons 5-8) and `preset` (the one chosen, from 1); `hold`; `locked` (LOCK: on off the page too); `active` (on now). |
| `lfos` | The project's LFOs by pad, each `{ pad, shape, sync, rate, phase, level, quantize, trigger, fade, muted, points }`: `shape` its name (Sine, Square, Triangle, Saw, Random), `rate` as written ("1/4", "2.50 Hz"), `phase` in degrees (0-360), `level` 0-1, `quantize` steps per cycle (0 Off), `trigger` "Off" or "A1 / S3" (groups named as Maschine's: A1 … H1, then A2 …), `fade` in beats (0-16), `muted` (its routes leave their targets alone), `points` its output over two cycles (240 samples, 0-1); and its envelope: `env` (Envelope mode: an ADSR its trigger's notes play), `attack`, `decay`, `release` as written ("12 ms", "1.20 s"), `sustain` 0-1, `retrigger`, `name` ("LFO 3", or "ENV 3" in Envelope mode), `note_off` (where an envelope's `points` show the note's end, 0-1 of their width; nil for an LFO). An envelope with no trigger shows `trigger` "None". |
| `lfo_focus` | The pad of the LFO the software panel and the MK3 show (0 with none). |
| `mod_slots` | The mod matrix page's slots: every route in the project, each in the column it had (an edit never moves a slot to another column; erasing one closes the later ones up; routes new to the page come after, by group, then sound, then their place on the sound), then the slot being made (a source and a target chosen, no destination yet; it isn't saved) or an empty one. `mod_slots[n]` is its six rows as shown, from 1: Source ("LFO 3", "LFO 3 (none)" when that LFO is gone), Group ("A1 – name", or "A1" when it has none of its own: none, or "Group A1"; as Maschine's: A1 … H1, then A2 …), Sound ("S3 – name"), Module (the slot's name), Destination (the parameter's name) and Depth ("+15%", "−40%", "0%"); "—" where unset. The sounds' routes come first, then the CC routes. A CC route (or a slot being made into one) has `mod_slots[n].midi` true: its Group reads "MIDI" and rows 3–5 are Port (the output port's name), Channel ("1"–"16") and CC ("0"–"119"); the page names those rows PORT, CHANNEL and CC. |
| `sampling` | What the Sampling page (SAMPLE) shows, while it's open: `tab` (1 RECORD, 2 EDIT, 3 SLICE, 4 ZONE); `pages[tab]`, that tab's knob pages, each `{ name, knobs }`, a knob `{ key, name, text, value, min, max, step, turn }` (`step` how far a step moves it, `turn` how far the knob turns for one); `slice`, the slicer's knobs by key (`Slice.Mode`, `Slice.Split`, `Slice.Grid`, `Slice.Density`, `Slice.ZeroX`, `Slice.Apply`, `Slice.BpmMode`, `Slice.Bpm`), each with its `index` for `deckard.sampling_knob`; `sample`, the focused zone's sample or `nil`: `{ name, frames, rate, view = { from, to }, play = { from, to }, looped, selection, slices (their starts), focus_slice, playhead, zones (each `{ key lo, key hi, vel lo, vel hi }`), focus_zone, palette }` (frames from 0; its peaks from `deckard.peaks`); `record`, the recorder: `{ levels = { l, r } (linear), mode (0 Detect, 1 Sync, 2 Loop, 3 Auto), threshold (dB), waiting, recording }`; `takes` (how many recordings) and `take` (the one focused, from 1); `stems`, a stem separation running: `{ focused, progress }` (0–1), or `nil`. |
| `position` | Ticks into what's playing (960 a quarter note), or `nil` when stopped. |
| `progress` | How far through its pattern the focused group is (0–1), or `nil` when stopped. |
| `numerator`, `denominator` | The time signature. |
| `group`, `sound` | The focused group and sound. |
| `groups[i]` | `{ name, color, palette, default_color, muted, playing, mix, patterns, scene_pattern, sounds, perform }`: `patterns[k]` is `{ name, color, palette, default_color, bars }`; `scene_pattern` the pattern the focused scene plays, or `nil`; `perform` its Perform FX's strip, `{ touched, pos }` (0..1), or `nil` without one. |
| `groups[i].sounds[j]` | `{ name, color, palette, default_color, muted, used, playing, mix, choke_group, choke_mode, link_group, link_mode, base_key, touch, prob_locked }`: `touch` is its Pad Touch, `{ mode (0 Velocity, 1 Poly AT, 2 Aftertouch, 3 CC), cc, curve, min, max }`; `prob_locked` whether PROB LOCK keeps its notes' chances. |
| `mix` (a group's or a sound's) | `{ level (dB, -inf silent), pan (-1–1), meter = { l_db, r_db }, headroom (dB), plugins = { { instrument, active } … } }`. |
| `scenes[i]`, `scene`, `scene_name`, `scene_palette` | The scenes (`{ name, color, palette, default_color, patterns }`) and the focused one. `patterns[g]` is the pattern group `g` plays in the scene, or `false`. |
| `song_view` | ARRANGE shows the Song view (the sections), not the Ideas view. |
| `sections[i]`, `section` | The arrangement's sections (`{ scene, start, length, auto }`: the scene it plays or `nil`, where it starts and how long it is in ticks, its length following its scene's) and the focused one (`nil` with none). |
| `pattern`, `pattern_start`, `pattern_length`, `pattern_auto`, `pattern_position` | The focused group's focused pattern: where it starts and how long it is (ticks), whether its length is Auto, and where playback is in it (`nil` unless the scene plays it). |
| `notes[i]` | That pattern's notes: `{ sound, position, length, pitch, velocity, probability, selected }`. |
| `browser` | The library, as BROWSE shows it (below). It's filled only while the `browse` page is on screen. |
| `control` | The software's Control area, as below. |

`state().control` holds:

| Field | What it is |
|---|---|
| `level` | 1 Master, 2 Group, 3 Sound. |
| `plugins` | `true` when the plug-ins are shown, `false` for the channel properties. |
| `page_group`, `page_groups` | The channel properties' page group, and how many there are. |
| `slot`, `slots`, `slot_index` | The focused slot's name, how many slots there are, and which is focused (`nil` on the "+" after the last). |
| `chain[i]` | Each slot: `{ name, instrument, active, plugin, picture }` (`plugin`: a hosted plug-in, with an editor window; `picture`: the key of DECKARD's picture of one of its own plug-ins, for `screen:image`). |
| `pages`, `page` | The page names, and the page shown. |
| `knobs[1..8]` | `{ name, section, text, value, modulatable, step_mod, auto_mod, macro }` (`value` 0–1; `modulatable` whether its movements can be written as modulation; `step_mod` the Step Mod page's text for the step shown, "+25%", or `nil`; `auto_mod` with Auto-write on, its modulation where the song is, "+25%" or "0%", else `nil`; `macro` whether some channel's macro controls its parameter: the Control page draws the macro icon at the top right of its name, but not on the Macro page itself), or `false` where the page has no knob. |
| `modules[i]` | What the plug-in browser can load, while the `module` page shows (else empty): `{ name, instrument, format, vendor }`. `format` is `"Internal"`, `"Audio Units"`, `"VST3"` or `"VST"`; Maschine's own vendor is `"Native Instruments"`. |

`state().browser` holds:

| Field | What it is |
|---|---|
| `kind` | The content kind: 1 Projects, 2 Groups, 3 Sounds, 4 Instruments, 5 Effects, 6 Loops, 7 One-shots. |
| `user` | `true` for user content, `false` for factory content. |
| `products`, `product` | The products' names, and the chosen one (`nil` for all). |
| `attributes[1..3]` | Type, Sub-type and Character: `{ values = { names }, chosen = n or nil }`. |
| `results[i]`, `focus` | The results (`{ name, favorite }`) and the focused one. |
| `busy` | A search is running. |
| `favorites` | Only favourites are shown. |

`color` is `{ r, g, b }`, each 0–255; `palette` is the project colour's index (0–15,
16 white), which is what lights take.

`deckard.user_chord(set, slot)` is a user chord (sets 1–16, slots 1–12): the
semitones above the root it plays, empty for none. `deckard.note_name(root, note)` names
a note as Maschine spells it in the scale chosen played from `root` (empty off the
scale).

`deckard.peaks(which)` is the focused sample's waveform, a column of a display each (480):
`"overview"` the whole sample, `"detail"` the frames shown (`sampling.sample.view`), as
`min, max, min, max …` (-1–1). Empty without a sample.

`deckard.chord(note, chord, auto)` is the chord a pad sending `note` plays in Chords mode,
as Maschine voices it (`docs/scales.md`, "Chord voicings"), in the scale chosen:

- `chord` holds the settings: `mode` (1 Harmonizer, 2 Chord Set, 3 User Chord);
  `type`, the type in the mode's list (from 1; for User Chord, the set); `chromatic`,
  the Harmonizer's type in `chords_chromatic` (Auto checks it in every mode);
  `position`, the place in `positions`; `root`, the root note.
- It returns the notes and Auto's state after the chord. Pass that state (or nil, to
  start afresh) as `auto` to the next chord played. The state is cleared when the
  settings change.

`deckard.scales` holds Keyboard and Chords mode's lists, as Maschine has them:
`banks`; `names[bank]` and `short[bank]` (each bank's scales, full and short names);
`steps[bank][kind]` (the semitones above the root the scale plays); `chord_modes` and
`chord_modes_short` (Harmonizer, Chord Set, User Chord); the chord types
`chords_chromatic` (and `_short`, the Harmonizer's with the chromatic scale),
`chords_scale` (with the others), `chord_sets` (and `_short`) and `user_chord_sets`;
and `positions`.

### Asking the app

| Function | Does |
|---|---|
| `deckard.play()`, `deckard.stop()`, `deckard.restart()` | The transport. |
| `deckard.record(on, count_in)` | Recording on or off, with an optional count-in. |
| `deckard.set_loop(on)`, `deckard.metronome(on)`, `deckard.auto_write(on)`, `deckard.fixed_velocity(on)`, `deckard.keyboard_mode(on)` | Switch each on or off. |
| `deckard.lfo(what, pad, value)` | Edit LFO `pad` (1-16). `what`: `"focus"`, `"create"` (makes it, with the defaults, and focuses it), `"remove"`, `"mute"` (mutes or unmutes it), `"shape"` (`value` 1-5 in the order Sine, Square, Triangle, Saw, Random), `"sync"` (0/1), `"trigger_focused"` (0 Off, 1 the focused sound; an envelope's is never turned Off), `"env"` (toggles Envelope mode; with no trigger it takes the focused sound), `"retrigger"` (toggles), or a knob turned `value` whole steps: `"rate"` (one entry of the list, or 10 % when in Hz), `"phase"` (5 degrees), `"level"` (1 %), `"quantize"` (Off, 2 ... 32), `"trigger"` (Off, then each sound in turn; an envelope's skips Off), `"fade"` (a quarter beat), `"attack"`, `"decay"`, `"release"` (a hundredth of the exponential 1 ms-10 s knob), `"sustain"` (1 %). Every one stops at its ends. |
| `deckard.mod_slot(slot, row, steps)` | Turn row `row` (1 Source, 2 Group, 3 Sound, 4 Module, 5 Destination, 6 Depth) of mod matrix slot `slot` (from 1) `steps` steps; each stops at its ends. Source steps through None, Pressure, Slide, Pitch and each LFO (None erases the slot). Group, Sound and Module step over the project's; on a route they make it a draft, in the route's place, until a Destination is chosen, when it moves there (one undo step; the old route drives until then). Destination steps over the module's parameters as the Control page has them, passing over those another route on the sound already drives (none free: the step does nothing): a new route starts there at the knob's place with Depth 0; an existing one moves, keeping its place. Depth is High − Low: a step moves High 5 % (0–1), Low fixed. `steps` 0 erases the slot. Group stepped past the last group is MIDI (it needs an LFO: Source becomes the first if it isn't one): rows 3–5 step over the output ports, channels 1–16 and CCs 0–119 (120–127 are channel mode messages), Source over None and the LFOs; the first CC step makes the CC route (the first port, channel 1 when unset; from none, `steps` on is CC `steps`), moving a sound's route there in one undo step. On a CC route, Group stepped back is a draft on the last group (the CC route stays until a Destination is chosen). |
| `deckard.mod_slots_closed()` | The mod matrix page closed: the slot being made (no destination yet) is dropped; every route stays as it is. DECKARD does this itself when `deckard.show_page` leaves the page named `mod_matrix` for good: not for a page shown while a button is held over it (`hold`, `pattern`, `scene`, `grid`, `events`, `note_repeat`), only once the next page isn't `mod_matrix` again. |
| `deckard.tempo(bpm)` | The tempo, 10–400. |
| `deckard.swing(amount)` | The swing, 0–100. |
| `deckard.volume(level)` | The master level, 0–1. |
| `deckard.volume_db(db)` | The master level in dB (at most +10). |
| `deckard.focus_group(g)`, `deckard.mute_group(g)`, `deckard.solo_group(g)` | Group `g`. |
| `deckard.focus_sound(s)`, `deckard.mute_sound(s)`, `deckard.solo_sound(s)` | Sound `s` of the focused group. |
| `deckard.erase_sound(s)` | Erase sound `s`'s notes from the focused pattern. |
| `deckard.prob_lock(on)` | PROB LOCK of the focused sound: on, its notes' chances are rolled once and kept every pass. |
| `deckard.reset_sound(s)` | Remove sound `s` of the focused group: an empty "Sound N" in its place, its notes gone (SHIFT + ERASE + its pad). |
| `deckard.edit(what)` | `undo` / `redo` (a whole take, as recorded), `undo_step` / `redo_step` (one edit), `copy` / `paste` (the selected notes; pasted at the focused sound), `clear_modulation` (the focused sound's in the focused pattern). |
| `deckard.turn_knob(n, amount, fine)` | Turn Control knob `n`. |
| `deckard.next_page()`, `deckard.previous_page()` | Change the Control page. |
| `deckard.level(l)` | Show the Master (1), Group (2) or Sound (3) chain. |
| `deckard.slot(delta)` | Focus the slot `delta` places on. |
| `deckard.slot_edit(what)` | The focused slot: `edit` (open its plug-in's window), `move_left` / `move_right` (swap it with its neighbour), `bypass` (switch it off or on), `remove`. |
| `deckard.show_plugins(on)` | Show the plug-ins, or the channel properties. |
| `deckard.page_group(i)` | Show channel-property page group `i`. |
| `deckard.show_page(name)` | Show `pages/name.lua`. |
| `deckard.sound_setting(name, value)` | The focused sound's `choke_group` or `link_group` (0 none, 1–8), `choke_mode` or `link_mode` (0 Send, 1 Receive). |
| `deckard.set_level(g, s, db)`, `deckard.set_pan(g, s, pan)` | Group `g`'s level (dB, up to +10) or pan (-1–1); its sound `s`'s when `s` isn't `nil`. |
| `deckard.focus_scene(n)` | Focus scene `n`; while a scene plays, it plays next. |
| `deckard.pick_pattern(n)` | The focused group plays its pattern `n` in the focused scene, focused. |
| `deckard.scene_pattern(group, n)` | Group `group` plays its pattern `n` in the focused scene, or none with `n` nil (the focus stays). |
| `deckard.sampling(open)` | The Sampling page opened (the focused sound's sample shown; RECORD if it has none) or closed. |
| `deckard.sampling_tab(n)` | Its tab: 1 RECORD, 2 EDIT, 3 SLICE, 4 ZONE. |
| `deckard.sampling_knob(page, n, value)` | Knob `n` of the tab's knob page `page` set to `value` (on SLICE, the slicer's knob `n`, its `index`; `page` is ignored). |
| `deckard.sampling_call(what, a, b)` | Ask something of the focused sample: `edit` (function `a`: 1 TRUNCATE, 2 NORMALIZE, 3 REVERSE, 4 FADE IN, 5 FADE OUT, 6 FIX DC, 7 SILENCE, 8 CUT, 9 COPY, 10 PASTE, 11 DUPLICATE), `auto_loop`, `stems`, `cancel_stems`, `view` (show frames `a` to `b`), `split_slice` / `remove_slice` (slice `a`), `clear_slices`, `move_slice` (slice `a`'s start to frame `b`), `apply_slices` (the slices become the focused sound's zones), `remove_zone` / `focus_zone` (zone `a`), `prehear` (frames `a` to `b`). Slices and zones count from 1, frames from 0. |
| `deckard.record_sample(cancel)` | RECORD's START (or STOP while waiting or recording); with `cancel` true, the recording cancelled. |
| `deckard.take(n, delete)` | Take `n` focused (loaded into the focused sound), or with `delete` true removed from the list. |
| `deckard.pattern_span(start, length)`, `deckard.pattern_length(length)` | Where the focused pattern starts and how long it is (ticks); a length chosen ends Auto. |
| `deckard.set_color("pattern" \| "scene" \| "group" \| "sound", n)` | Colour the focused pattern, scene, group or sound: palette colour `n` (0–15), or `nil` for the default. |
| `deckard.follow(on)` | Follow playback on or off. |
| `deckard.grid(name, value)` | Set one of `state.grid`'s grids: a place in its list, or `true`/`false` for `arrange_snap`, `quick` and `step_snap`. |
| `deckard.file(what, n)` | A File command: `new`, `save`, `save_copy`, `export_audio`, `save_group`, `save_sound`, or `load` (recent project `n`, from 1). |
| `deckard.setting(name, step, index)` | A step (`step` +1 or -1; on or off for a switch) of one of the app's settings: `count_in`, `numerator`, `denominator`, `quantize`, `device`, `buffer_size`, `in_route` / `out_route` (channel `index`, from 1), `midi_sync`, `clock_offset`, `default_input`, `midi_in` / `midi_out` (port `index`, from 1), `link`, `link_start_stop`, `sensitivity`, `display_brightness`, `metronome_volume` (0–100 %), `metronome_auto` (the metronome clicks while recording), `metronome_ticks` (the note value it clicks on: 1/1 … 1/32), `velocity_scaling`, `led_brightness`, `strip_position`, `touch_auto_write`, `reload_last` (open the last project at start), `auto_backup` (keep it as it's edited), `duplicate` (duplicating a scene copies its patterns too), `link_duplicates` (a duplicated section plays the same scene), `pattern_length` (new patterns' length, a bar a step; `pattern_length_fine` a sixteenth), `pattern_grow` (an empty pattern grows as it's recorded into). Saved at once. |
| `deckard.lock(what, n)` | Lock: `button` (LOCK: take or recall), `pad` (pad `n` of the bank shown: recall its snapshot or take one), `update`, `delete` (the focused snapshot), `delete_slot` (slot `n`, from 1), `bank` (`n` +1 the next, -1 the previous; past the last a new one), or a setting: `morph` (`true`/`false`), `morph_mode`, `morph_time`, `morph_grid` (places from 0). |
| `deckard.note_repeat(what, value)` | Note Repeat: a setting of `note_repeat.settings` to `value` (as its `value` counts), `hold` or `locked` (`true`/`false`), or `preset` (1-4: the preset's rate plays). It plays while the page named `note_repeat` shows, or while locked. |
| `deckard.strip(what, value)` | The Smart Strip: `pitch` bends the focused sound (-1..1, 0 the middle), `mod` sets its mod wheel (CC 1, 0..1); both are recorded with REC. `perform_touch`, `perform` and `perform_release` touch, move and let go of the focused group's Perform FX at `value` (0..1). |
| `deckard.strip_note(sound, pitch, velocity, on)` | A note of the focused group's sound `sound` (from 1) starts (`on` true) or stops, as the strip's NOTES strums it. |
| `deckard.perform_fx()` | The focused group's Perform FX shown on the Plug-in page (one loaded if it has none), as SHIFT + PERFORM. |
| `deckard.events(what, n)` | The Events page's edits. Of the selected notes, a step `n` up or down: `nudge` / `nudge_fine` (by the Nudge Grid, or the finest step), `pitch`, `velocity`, `length` / `length_fine` (by the Step Grid, or the finest step), `probability`; `velocity_to` / `probability_to` (set to `n`). Of the selection or, with none, the notes shown (Keyboard mode: the focused sound's): `nudge_shown` / `nudge_shown_fine`, `transpose` (`n` semitones), `quantize` / `quantize_half`, `clear` (deleted). `clear_all` (the group's notes; `n` 1: the focused sound's). The selection: `select_all`, `select_none`, `select_sound` (sound `n`'s notes, from 1, in or out of it; it gets the focus), `select_pitch` / `erase_pitch` (the focused sound's notes of pitch `n`: in or out of the selection / deleted), `select_step` (the focused sound's next note, `n` 1, or previous, alone). |
| `deckard.select_events(start, end, low?, high?)` | The focused sound's notes starting from `start` to `end` (ticks), with pitches from `low` to `high` (0–127 if not given), become the selection. |
| `deckard.step_mod_at(at, length)` | The Step Mod page shows the step at `at` (`length` long): the knobs' `step_mod` are its modulation. With no arguments, the page is gone; the edits made meanwhile are one undo step. |
| `deckard.step_mod(from, length, pads, knob, inc)` | The Step Mod page's knob `knob` (1–8) turned by `inc`: its modulation in each step held (`pads`, a list of pads 1–16; pad `n` the step at `from + (n − 1) × length`) moves by `inc` of its range, held over the step alone. |
| `deckard.step(at, length, velocity)` | Step mode's pad: the focused sound's notes starting from `at` to `at + length` (ticks) go; with none there, a note of its base key goes in at `at`, `length` long, at `velocity` (0–127), and becomes the selection. With no pattern, a new one first. |
| `deckard.groove_page(n)` | Show the focused sound's Groove properties on the Control page, page `n` (1 Swing, 2 Variation, 3 Generate). |
| `deckard.set_macro(knob, level)` | Make knob `knob`'s parameter (1–8, as the Control page shows it) a macro of the master (`level` 0), the focused group (1) or sound (2), or stop it being one. |
| `deckard.velocity(v)` | The velocity the pads play with Fixed Velocity on (0–127). |
| `deckard.scale(root, bank, kind)` | Keyboard and Chords mode's scale: the root (MIDI note) and the scale (places in `deckard.scales`). The pattern editor's scale follows. |
| `deckard.add_group()` | A new empty group after the last, focused. |
| `deckard.delete_group(g)` | Delete group `g` (one undo step; the last group is reset instead): SHIFT + ERASE + its group button. |
| `deckard.user_chord_note(set, slot, note)` | Add `note` (semitones above the root) to a user chord, or take it out. |
| `deckard.user_chord_preview(set, slot, root, on)` | Start (`on`) or stop hearing a user chord played from `root`. |
| `deckard.user_chord_move(set, slot, next)`, `deckard.user_chord_delete(set, slot)` | Swap a user chord with the one after (`next`) or before it; empty it. |
| `deckard.pad_touch(name, value)` | The focused sound's Pad Touch: `mode` (0–3), `cc` (0–127), `curve`, `min` or `max` (0–100). |
| `deckard.pattern_edit(what)` | `"create"`, `"double"`, `"duplicate"` or `"delete"` the focused pattern. |
| `deckard.scene_edit(what)` | `"create"` a scene after the last, `"insert"` one after the focused, `"duplicate"`, `"delete"` or `"clear"` (no group plays a pattern in it) the focused scene, or `"append"` it to the arrangement. |
| `deckard.song_view(on)` | ARRANGE shows the Song view (`true`) or the Ideas view. |
| `deckard.focus_section(n)` | Focus section `n`; while the song plays, it plays next. |
| `deckard.section_edit(what)` | `"create"` a section after the focused one (playing its scene), `"duplicate"` or `"delete"` the focused one. |
| `deckard.move_section(next)` | The focused section swaps places with the next (`true`) or the previous. |
| `deckard.section_scene(next)` | The focused section plays the next (`true`) or the previous scene. |
| `deckard.section_length(ticks)` | The focused section's length; no longer Auto. |
| `deckard.browse_kind(n)`, `deckard.browse_user(on)` | The library's content kind (1–7), user or factory content. |
| `deckard.browse_product(n)`, `deckard.browse_attribute(a, n)` | Choose product `n`, or value `n` of attribute `a` (1 Type, 2 Sub-type, 3 Character); `nil` for all. |
| `deckard.browse_focus(by)` | Move the result focus by `by`. |
| `deckard.browse_load()`, `deckard.browse_prehear()`, `deckard.browse_set_favorite()` | Load or prehear the focused result, or make it a favourite (or not). |
| `deckard.load_module(n)` | Load `control.modules[n]` in place of the focused slot, or into the "+"; `nil` takes the focused slot out. |
| `deckard.browse_favorites()` | Show only favourites, or everything. |

### Lights

| Call | Sets |
|---|---|
| `deckard.led(name, "off" \| "dim" \| "on")` | A button's light. A button left alone, or set `"off"`, keeps a faint backlight so its label reads. |
| `deckard.led(name, palette, bright)` | A pad's, group button's or jog-wheel arrow's colour (a palette index, 16 white), bright or dim. |
| `deckard.led(name, "off")` | Turns a pad or group button off. |

## Screens and themes

`draw(screen, theme)` draws both screens as one picture, 960 × 272 pixels: the left
screen is x 0–479 and the right screen x 480–959.

| Method | Draws |
|---|---|
| `screen:rect(x, y, w, h, color, corner)` | A filled rectangle; `corner` (0–1) softens its corner pixels. |
| `screen:frame(x, y, w, h, color, corner)` | A rectangle's 1-pixel outline. |
| `screen:triangle(x, y, w, h, direction, color)` | A filled triangle pointing `"left"`, `"right"`, `"up"` or `"down"`. |
| `screen:circle(cx, cy, r, color, width, from, to)` | A filled circle, or a ring `width` wide; with `from` and `to` (degrees, 0 to the right, clockwise) only that arc of it. |
| `screen:line(x0, y0, x1, y1, width, color)` | A line. |
| `screen:label(x, y, w, h, text, style, align, valign)` | Text in a box: `align` `"left"`, `"center"` (the default) or `"right"`, `valign` `"top"`, `"center"` or `"bottom"`. Returns its width. |
| `screen:text(x, baseline, text, style, max_width)` | Text, and returns its width. Text past `max_width` is cut. |
| `screen:text_width(text, style)` | Returns how wide the text would be, without drawing it. |
| `screen:image(x, y, w, h, key, thumb)` | DECKARD's picture `key` (a plug-in's thumbnail, or with `thumb` false its logo) fitted into the box, centred; returns false, drawing nothing, if there's none. |
| `screen:size()` | Returns 960, 272. |

`color` is `{ r, g, b }` or the name of one of the theme's colours. `style` is the name
of one of the theme's styles, or a table such as `{ font = "mono", size = 14, color =
"accent" }`.

### Themes

A theme, `themes/<name>.lua`, is the stylesheet every page draws with. It holds:

- **`colors`**: named colours. `themes/maschine.lua` lists every name the shipped pages
  use, each with a note on where it shows.
- **`fonts`**: font roles mapped to fonts. DECKARD's fonts are `"ubuntu"` (Ubuntu Light)
  and `"hack"` (Hack, fixed width).
- **`styles`**: named text styles, each with `font`, `size` and `color`. A style takes
  whatever it leaves out from `styles.default`.
- **Sizes** the pages use: `header_height`, `knob_bar_height`.
- **`base`** (optional): another theme's name. Whatever this theme leaves out of
  `colors`, `styles` (down to each style's fields) and the rest comes from that one,
  so a theme need only list what it changes. `solarized_dark` and `palenight` build on
  `maschine` this way.
- **`chase`** (optional): `{ colors = { a, b }, lap = seconds, width = pixels }`. A
  chosen display button's or tab's border becomes the two colours fading into each other,
  running round its edge once every `lap` seconds, on every page but the arranger (Ideas,
  Song) and the mixer. A page drawing its own tabs calls `widgets.chase(screen, theme, x,
  y, w, h)` for the chosen one. Maschine has nothing like it, so `maschine` leaves it out;
  `palenight` turns it on (orange and cyan).

Because pages draw only with these names, changing a theme restyles every page. To make
your own theme, copy one into `themes/` under a new name; it then shows in
**Preferences ▸ Hardware ▸ Controller theme**.

## MIDI controllers

Any MIDI controller can be scripted the same way. DECKARD ships one, the Launchpad Pro
[MK3] (`devices/launchpad_pro_mk3/`, see `launchpad-pro-mk3.md`). To add yours, make a
folder `devices/<id>/` in the controller folder with two files.

`device.lua` names the device and its ports:

```lua
return {
  name = "Launchpad Pro [MK3]",
  ports = { daw = { "LPProMK3 DAW" } },  -- role → parts of the port's name
  exclusive = { "daw" },                  -- roles whose notes DECKARD must not play
  drums = { "LPProMK3 MIDI" },            -- its notes port, for a drum channel
}
```

DECKARD opens the device when, for every role, there is an input and an output port
whose name contains one of the role's fragments. It closes the device when one of those
ports goes. A port in an `exclusive` role is never a note input, so pressing the
device's buttons doesn't play the focused sound. Any other port the device has (its
notes port) plays DECKARD as any keyboard does.

`drums` (optional) names the port the device's notes come on. A device with it gets a
**drum channel** in Preferences ▸ MIDI (Off, or 1–16): its notes on that channel from
C1 (36) play the focused group's pads, pad 1 bottom left, four by four, each sound at
its root, as the MK3's pads do. Its other channels play as before. DECKARD doesn't
open the port for the device's scripts.

`main.lua` returns its handlers. Each one is optional:

| Handler | When |
|---|---|
| `open()` | The device was found. Set it up and draw it. |
| `close()` | Its port went, or DECKARD quits. Put it back in its own mode. |
| `midi(role, bytes)` | A message came on the port of `role`: its bytes from 1, a whole SysEx `F0 … F7` in one. |
| `tick()` | As often as the app updates. Redraw what changed. |

A device gets the whole `deckard` API, so `deckard.state()`, `focus_group`,
`focus_scene` and the rest work, but it gets no MK3 lights or screens. It also gets:

| Function | |
|---|---|
| `deckard.midi_send(role, bytes)` | Send bytes (0–255 each) on the port of `role`. |
| `deckard.launch_scene(i)` | Launch scene `i`: as clicking it in the Ideas view, and when stopped it starts playing. |
| `deckard.print_notes{ length = beats, notes = { {pitch, start, length, velocity}, … }, track = n }` | Print to Clip: the notes (times in beats) replace the focused sound's in the focused group's pattern in the focused scene. A pattern is made if there is none, and the pattern takes `length`. It is one undo step. `track` (optional, from 1) is the device's track it came from: a print from the track set as the device's drum track (Preferences ▸ MIDI) plays the focused group's pads from C1 instead, replacing the group's notes. |
| `deckard.message(text)` | Show a line in the status bar. |
| `deckard.device()` | The device's id (its folder's name). |

`deckard.state()` also has `playing_scene` and `queued_scene` (from 1, or nil).

`require("lib.x")` looks in the device's own folder first (`devices/<id>/lib/x.lua`),
then the shared `lib/`. A device never loads the MK3's pages. Each device runs in its own
Lua state, so a broken device script shows its error in the status bar and stops only
that device, never the MK3. Edits to its files reload it, and `open()` runs again.

## Not available to scripts

Scripts get Lua's `string`, `table`, `math` and `utf8` libraries, and the `deckard`
API. They don't get files, processes or the network.
