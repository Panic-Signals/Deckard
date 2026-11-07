# Maschine JAM integration — design

November 2026. **Built, not yet played.** A JAM next to (or instead of) the MK3: its 8×8 grid,
Smart Strips, meters and buttons driving DECKARD directly over USB, with no Maschine install.
`docs/maschine-jam.md` is what it does now, with what differs from the manual; this is why.

Sources, and how far to trust them:

- **Measured:** the JAM's USB interfaces, HID reports and resting values, read from one unit on
  Windows on 2026-11-05 (`docs/jam-usb.md`).
- **Manual:** *MASCHINE JAM Manual*, September 2016, for software 2.5. It predates Maschine 3,
  so the behaviour it describes may have moved on.
- **cabl** (github.com/shaduzlabs/cabl, MIT): an open-source driver, for which bit and byte is
  which control. It agrees with everything measured but hasn't been pressed against this unit.
- **Inferred:** the rest of what each report carries. It fits the arithmetic but hasn't been
  tried. `jam-check` settles it.

## Intent

- **The JAM is a first-class controller**, not a MIDI mapping: every press, strip and light
  goes through DECKARD's own driver.
- **It works alongside the MK3 and shares its focus**, as the manual says of Maschine (§4.5.3).
  Pick a group on either one and both follow.
- **The behaviour is Lua**, in `devices/maschine_jam/`, replaceable from the user's controller
  folder like every other script. The driver is Rust, because timing and bytes belong there.
- **Pads play from the driver's thread**, like the MK3's, so a note never waits for a frame.
- **Upstream is stock** (the user's decision, 2026-11-05). The shipped script follows the manual.
  Personal layouts, such as a page of mute pads or a Euclid mode, live in the user's controller
  folder and never go in these PRs.

### Non-goals

- **The on-screen overlay (OSO).** It isn't being drawn. "Without the overlay" below says what
  each of its functions does instead.
- MIDI mode, Controller Editor templates and Host Transport Control.
- Maschine's plug-in mode and instance switching (SHIFT + H).
- Writing to the JAM's firmware, or to any feature report.
- Keeping Deckard and Maschine out of each other's way on one JAM (see "Sharing the JAM").

## The hardware

### USB (measured)

VID `0x17CC`, PID `0x1500`, product string "Maschine Jam HID".

| Interface | Windows driver | What it is |
|---|---|---|
| 0 | `HidUsb` | HID, vendor page `0xFF01` |
| 1 | NI's `nimjcusb` | "Maschine Jam DFU" (firmware update): never touched |

Two things follow from that list:

- **There is no bulk or display interface, so the MK3's Windows screen problem doesn't apply.**
  Windows' own HID driver owns interface 0, and `hidapi` opens it with no driver change. The
  open succeeded with `NIHostIntegrationAgent` running; reading input alongside the agent is
  still to be tried.
- **There is no USB MIDI or audio interface.** The JAM's MIDI mode has to be provided by NI's
  software on the computer, which is one reason not to build on it.

### HID reports

| Report | Size (without id) | Measured | Inferred contents |
|---|---|---|---|
| Input `0x01` | 16 | yes | Two 4-bit fields (probably the encoder) and 120 one-bit fields, for the 119 switches below. |
| Input `0x02` | 48 | yes | Smart Strips: 24 u16, so 8 × 3. At rest they read as the MK3's record (timestamp, two fingers at 0). |
| Input `0x40` | 24 | yes | Unknown: 12 u16, all zero at rest. Too few for per-pad pressure. |
| Output `0x80` | 53 | yes | Fields of 37 and 16: plain LEDs, then 2 × 8 meter LEDs. The manual gives 35 lit buttons; two more are to find. |
| Output `0x81` | 80 | yes | Colour LEDs: 64 pads + 8 scene + 8 group = 80. |
| Output `0x82` | 88 | yes | Strip LEDs: 8 strips × 11 = 88. |
| Output `0xF4` | 3 | yes | Unknown. Not written. |
| Feature `0xD0`, `0xD8`–`0xDD` | 32 each | yes | `0xD9` looks like a serial; the rest unknown or unit-specific. Not written. |
| Feature `0xF1`, `0xF2` | 204 each | yes | The palette: 17 colours × 4 shades × RGB, in the MK3's order. Not written. |

Phase 0's first pass is in `docs/jam-usb.md`. Reports come only on a change, and the device
accepts LED frames with NI's agent running. What each bit and byte is named is still to learn
with someone at the controller.

The 119 switches are 64 pads, 8 scene buttons, 8 group buttons, 35 other buttons, the encoder's
press and touch, and the footswitch's tip and ring.

### Controls (manual §3)

| Section | Controls |
|---|---|
| Project View | 8 SCENE buttons (top), 8×8 click-pads, 8 GROUP buttons A–H (bottom); pads 1–16 are labelled on the grid |
| Note input | SONG, STEP, PAD MODE, CLEAR, DUPLICATE |
| Meters | Two columns of 8 LEDs; MST, GRP, IN1, CUE |
| Navigation | Encoder (turn, press, touch), D-pad, BROWSE |
| Smart Strips | 8 strips, 11 LEDs each; MACRO, LEVEL, AUX, CONTROL, PERFORM, NOTES, LOCK, TUNE, SWING, AUTO |
| Performance | SELECT, SOLO, MUTE, SHIFT, NOTE REPEAT |
| Transport | PLAY, REC, page left, page right, TEMPO, GRID |
| Rear | Footswitch jack (tip PLAY, ring REC) |

The JAM has no screens and no STOP button: PLAY toggles. The LED counts (11 a strip, 8 a meter
column) aren't in the manual. They come from the names of Maschine's hardware constants
(`LED_TS_A01` … `LED_TS_H11`, `LED_METER1_01` … `LED_METER2_08`), and Phase 0 confirms them.

## How to build it

### Options considered

| Option | Verdict |
|---|---|
| **A. HID driver + Lua device** | **Recommended.** It works with no NI software on any platform `hidapi` covers, and matches how the MK3 and Launchpad are already split. |
| B. MIDI mode via Controller Editor | No. It needs NI's software running and a template installed, and can't give the note path or full light control. |
| C. NI's Jam scripts through `lua-host` | No. It needs a Maschine install, those scripts expect the overlay, and it can't ship. It could still serve later as a behaviour reference on machines that have Maschine. |

### Shape

```
crates/jam             reports ↔ events and LED frames; no threads, no app
apps/viewer/src/jam_io.rs   the thread: reads events, plays pads, writes changed LEDs
crates/surface/src/device.rs   gains an HID transport beside MIDI
crates/surface/lua/devices/maschine_jam/   device.lua, main.lua, pages/, lib/
```

**`crates/jam`** mirrors `crates/mk3`: `Decoder` turns reports into events, and `Leds` holds
the three output frames.

```rust
pub enum Event {
    Button(&'static str, bool),
    Pad { column: u8, row: u8, down: bool },
    Strip { strip: u8, finger: u8, kind: Touch, pos: f64 },
    Encoder(i32),
}
```

Pads are separate from buttons because the note path needs them without a name lookup.

**`jam_io.rs`** mirrors `mk3_io.rs`. One thread polls the device, plays pads through the
current route, and writes an LED frame only when it changed. Everything else goes to the UI
thread as events. It starts only from the app, never in tests, as the MK3's does.

**The device host** today knows only MIDI ports, and the Launchpad's input reaches its script
on the UI thread. The JAM needs four additions to `deckard_surface::device`:

| Addition | Why |
|---|---|
| `hid = { vendor, product }` in `device.lua`, as an alternative to `ports` | The JAM has no ports. |
| `button(name, down)`, `pad(column, row, down)`, `strip(n, finger, kind, pos)` and `encoder(steps)` handlers, beside `midi` | Decoded events, so scripts never parse bytes. |
| `leds()` handler and a device-side `deckard.led`, with `deckard.strip_led(n, led, palette, bright)` and `deckard.meter(column, n, level)` | Devices have no lights today: `midi_send` is their only output, and `Leds` is MK3-shaped. |
| `pads()` handler returning a 64-pad route | See "Note path". |

Other devices can use the same HID transport later. Each device keeps its own Lua state, so a
broken JAM script can't stop the MK3, and the reverse.

### Note path

`PadRoute` is 16 pads. In Keyboard mode the JAM plays up to 64, so the JAM gets its own route,
set by the script's `pads()` at frame rate and read by the thread:

```rust
pub struct JamRoute {
    /// By pad: what it plays. Empty where a pad isn't an instrument in this mode.
    pads: [PadNotes; 64],   // (group, sound, pitches)
    velocity: i64,
}
```

The thread remembers what each press started, so a release stops the same notes however the
route has changed since, as `mk3_io` does. Notes go out as the same `Live` messages the MK3
sends, so recording and quantizing need nothing new (to confirm in Phase 2).

**Velocity:** the pads very likely report on and off only: the descriptor gives them one bit
each, and input `0x40`'s 12 values are too few for 64 pads. So every hit plays at a fixed
velocity (the song's Fixed Velocity value, or Accent's). Phase 0 confirms this by pressing pads
softly and hard while watching `0x40`.

### Keeping time without the MK3

`tick()` runs "as often as the app updates". The lights must keep following the song with the
window minimized, and blinks must hold time when the MK3 isn't connected. Phase 1 has to check
both: the MK3's held-button tick and `keep_running.rs` are the places to start.

## Sharing the JAM

- **With the MK3:** focus is shared through `View` and `Action`, which already works for the
  Launchpad.
- **With Maschine: Deckard opens the JAM anyway** (the user's decision, 2026-11-05). It doesn't
  check whether Maschine is running. The HID interface opens shared, so both apps will act on
  every press and overwrite each other's lights. That's accepted, not a bug to fix. A Preferences
  switch turns JAM support off, for when Maschine should have the JAM to itself.
- **On closing:** all LEDs off. What the JAM shows after that, once NI's agent is the only thing
  talking to it, is a hardware check.

## Behaviour

Status is against the `deckard` API as it stands in `docs/controller-api.md`:
**ready** means the calls exist, **gap** means new API is needed (listed afterwards).

### Project View (SONG; the default page)

Each column is a group, top to bottom: its scene button, up to 8 patterns, its group button
(manual §7.2).

| Control | Does | Status |
|---|---|---|
| Pad, bright | The pattern the focused scene plays | ready |
| Pad, half-lit | A pattern it doesn't play; a press picks it for the scene | ready: `scene_pattern` |
| Pad, unlit | Empty; a press makes a pattern | ready: `new_pattern` |
| SCENE 1–8 | Follows the manual (§7.1, §7.2; the user's decision, 2026-11-05): while playing, the scene plays next; while stopped, it's selected and PLAY starts it | ready: `focus_scene` |
| SHIFT + SCENE | Scene bank | ready |
| GROUP A–H | Focus the group | ready |
| D-pad | Up/down: patterns; left/right: group banks | ready |
| CLEAR + pad / scene / group | Delete it | ready for scene and group; pattern needs a target (gap 1) |
| DUPLICATE + source, then targets | Copy, with the target blinking, ready to paste again | gap 1 |
| SHIFT + DUPLICATE | Double the pattern | ready: `pattern_edit("double")` |
| SHIFT + SOLO (PAT LENGTH) | Pads set the length, a bar each | ready: `pattern_length` |

### Note input

| Mode | Entered by | Does | Status |
|---|---|---|---|
| Pad | PAD MODE | Pads 1–16 play the focused group; the group buttons pick the group | ready, on the JAM route |
| Keyboard | SHIFT + PAD MODE | 64 pads play the focused sound in the scale, roots white; D-pad up/down moves the range | ready, on the JAM route |
| Step | STEP | 64 steps of the focused sound; a light chases the position; SCENE 1–8 pick the part shown | ready for one sound: `step` |
| Step, 4 or 8 sounds | STEP + 4 or 8 | Lanes per sound | gap 2 |
| Piano Roll | SHIFT + STEP | Across is time, up is pitch | ready: `step_notes` |
| Accent | SHIFT + SELECT | New steps at the accent velocity, brighter | ready |
| Note Repeat | NOTE REPEAT | Held pads repeat | gap 3 |
| Arp | SHIFT + NOTE REPEAT | Keyboard mode with the arp on | gap 3 |
| Chords | From the scale settings | Chord sets on 12 pads; harmonizer on all | ready: `chord` |
| Notes | NOTES + 1, 2 or 3 | Strips strum Guitar, Chords or User sets | ready: `strip_note` |

### Smart Strips

The mode buttons pick what the strips do. Holding one and pressing a group or pad picks the
channel (manual §4.1). LEDs show the value, and a double tap resets (manual §8.1.1).

| Mode | Strips control | Status |
|---|---|---|
| LEVEL | Groups A–H, or sounds 1–8 / 9–16 by page | ready: `set_level` |
| SHIFT + LEVEL | Pan | ready: `set_pan` |
| AUX, SHIFT + AUX | AUX 1, AUX 2 | gap 4 |
| TUNE | Group or sound tune | gap 4 |
| SWING | Group or sound swing | gap 4; the song's swing is ready |
| CONTROL | The Control page's 8 knobs; page buttons turn pages; D-pad up/down moves between plug-ins | ready: `turn_knob`, `next_page`, `slot` |
| MACRO | Macros of the channel | ready, by the Control page |
| PERFORM | The Perform FX: touch, move, let go | ready for the focused group: `strip` |
| SHIFT + PERFORM | Pick the group's Perform FX | see "Without the overlay" |
| AUTO, held | Writes the strip's movement while playing | ready: `auto_write`, `TouchWrite` |
| SHIFT + AUTO + CLEAR | Clears modulation | ready: `edit("clear_modulation")` |

Strips are absolute and `turn_knob` is relative, so the script turns by the difference from
the knob's current value. One touch may need a pickup rule if the jump sounds wrong, which is
a judgement to make by ear.

### Everything else

| Control | Does | Status |
|---|---|---|
| PLAY; SHIFT + PLAY | Start or stop; restart | ready |
| REC; SHIFT + REC | Record; with count-in | ready |
| SHIFT + page left; SHIFT + page right | Metronome; Loop | ready |
| Footswitch | Play; record | ready |
| TEMPO + encoder; SHIFT + TEMPO | Tempo; tap | ready |
| GRID + pad 1–4 | Step grid 1/4, 1/8, 1/16, 1/32 | ready: `grid` |
| SELECT + pad | Focus a sound silently | ready |
| SOLO or MUTE + pad / group | Solo or mute; held, or pinned | ready |
| SHIFT + MUTE | Choke all | gap 5 |
| Mode button + SONG | Pin the mode | Lua only |
| LOCK; SHIFT + LOCK | Snapshot; Lock view on the 64 pads | ready: 16 × 4 banks = 64. A pad needs its bank set first, or a `slot` argument. |
| MST, GRP, CUE, IN1 | Choose what the meters show and the encoder sets | gap 6 |
| SHIFT + group or scene button | Printed shortcuts: Follow (A), Variation (E), Save (G), undo, redo, quantize, transpose | ready, except Variation. The manual's pictures don't name them all, so they come from the panel in Phase 0. |

### API gaps

| # | Needs | Used by |
|---|---|---|
| 1 | Clear and duplicate by target: pattern, scene, group or sound, not only the focused one | CLEAR, DUPLICATE |
| 2 | `step` with an explicit sound | Step mode's lanes |
| 3 | Note Repeat on and off directly. Today "it plays while the page named `note_repeat` shows", and a device has no such page. | NOTE REPEAT, Arp |
| 4 | `set_send`, `set_tune`, `set_swing` by group and sound, and those values in `mix` | AUX, TUNE, SWING |
| 5 | Choke all voices | SHIFT + MUTE |
| 6 | Master, cue and input meters in `state()`. Group and sound meters are there. | Meter LEDs |
| 7 | State that fills only for one page (`browser`, `settings`) must also fill when a device asks | BROWSE |

## Without the overlay

The overlay was a panel on the **computer's** screen. DECKARD already has a window on that same
screen, so its own panels can do that job with no overlay built. Each overlay function gets
one of four treatments:

| Treatment | Applies to |
|---|---|
| **No display needed:** the hardware is enough | Pattern length, step grid by GRID + 1–4, Notes sets by NOTES + 1–3, Lock's pads, level, pan and control on the strips' LEDs |
| **Encoder, with a one-line readout in the status bar** (`deckard.message`) | Tempo, master, cue, group level, swing, tune: "Tempo 128.0" |
| **Encoder and D-pad driving a panel in the main window** | Browser (BROWSE shows the library panel; encoder scrolls, press loads), Perform FX choice, scale |
| **Deferred:** set them with the mouse or the MK3 meanwhile | Note Repeat and Arp parameters, Variation, Lock's morph settings, Rec Mode, Nudge, Arrange and Perform grids |

If Deckard is running on a machine with no display attached, only the first two rows work.

## Plan

All of it was built at once, on the user's request, once cabl's table meant no phase had to wait
for the hand-learning of Phase 0. The phases below stay as the way to split it into pull requests.
No open PR or issue touched the JAM at the time of writing.

| Phase | Delivers | Done when |
|---|---|---|
| **0. Learn the protocol** | `crates/jam/examples/jam-probe.rs`, `jam-learn.rs`, `jam-led-learn.rs`; `docs/jam-usb.md` | Every switch, strip, encoder and LED is mapped; the palette and brightness are known; `0x40` is understood or ruled out |
| **1. Driver and Project View** | `crates/jam`, `jam_io.rs`, the HID transport; transport, groups, scenes and patterns, with lights | Launching and picking patterns works with the MK3 unplugged; lights keep time with the window minimized |
| **2. Playing** | The JAM route; Pad and Keyboard modes, Step, Piano Roll, Accent, recording | Pads feel as immediate as the MK3's, and a recorded take lands on time |
| **3. Strips** | Level, pan, control, macro, perform, AUTO; gaps 4 and 6 | Each mode's LEDs follow the value from either controller |
| **4. Performance** | Solo, mute, select, pin, clear, duplicate, Lock, meters, footswitch; gaps 1 and 5 | Every hold-and-press in the manual's chapters 7 and 8 works |
| **5. Encoder and panels** | Browser, Perform FX choice, scale, status-bar readouts, Note Repeat, Arp, Notes; gaps 2, 3 and 7 | Loading a kit from the JAM works without touching the mouse |

**Safety rules for Phase 0.** Maschine must be closed for anything that writes. The learn tools
write only `0x80`, `0x81` and `0x82`, and never a feature report, `0xF4` or the DFU interface.

## Testing

- **Decoder and LED frames:** unit tests from captured reports, as `crates/mk3` has.
- **Scripts:** headless tests that feed events and check actions and lights, as
  `launchpad_tests.rs` does. No test opens real hardware: the app starts `jam_io`, tests don't.
- **Hardware checklist:** `docs/maschine-jam.md`, in the Launchpad's format, ticked by hand on
  Windows. macOS and Linux stay listed as unchecked until someone tries them.

## Clean room

As with the MK3 layer, the scripts are written from the manual and from watching what Maschine
does with a JAM attached, never by reading and translating NI's Lua. Nothing of NI's is copied
or bundled. Button names follow what's printed on the panel. For this draft, only the names of
hardware constants and handlers were looked up in NI's Jam scripts, to count the controls; no
behaviour here comes from their logic.

## Open questions

1. **What do the columns left of pads 1–16 show in Pad mode?** The manual says only that the pads
   and groups light up. To settle by watching Maschine.
2. **Does a Perform strip mean the focused group, or each strip its own group?** The manual reads
   as the focused group, which is what `strip` does today. To settle by watching Maschine.
3. **How much has Maschine 3 changed since this 2016 manual?** Where they differ, Maschine as it
   behaves now is the target.

## How the gaps were closed

| # | As built |
|---|---|
| 1 | `delete_pattern`, `delete_scene`, `copy_pattern`, `copy_scene`, `copy_group`, `copy_sound`: by target. `copy_group` goes through the clipboard's Copy and Paste Group. |
| 2 | No API needed: a lane focuses its sound, sets the step and focuses back, in one frame's actions. |
| 3 | `note_repeat("on", …)`. The Arp is that plus `keyboard_mode(true)`. |
| 4 | `set_send`, `set_tune`, `set_swing`, and `aux`, `tune`, `swing` on each channel. DECKARD has no group tune, so a group's moves its Samplers together. |
| 5 | `choke_all`, a new `Live::ChokeAll`: each sound as its choke group would end it. |
| 6 | `master_meter`, `cue_meter`, `input_meters`, `cue_level`, `input_levels` in `state()`. The cue bus and the first two inputs meter themselves; those inputs have a level of DECKARD's own. |
| 7 | `library(open)` fills `browser` while the panel's open; `settings_wanted(on)` fills `settings`. |

Two more turned up on the way: `reset_knob` (a double tap on a Control strip) and `perform_mode`
(picking the effect), and `lock("slot", n)`, since the JAM has all 64 snapshots on its pads at once.

The overlay's "deferred" row went further than planned: one status-bar menu on the encoder covers
Note Repeat and the Arp, Rec Mode, Nudge, Arrange and Perform grids and Lock's morphing, so only
Variation's settings stay on the Control page.

## Unverified

Which bit and byte is which control (`jam-check`); that strips and meters read from the bottom; that
streamed input arrives while NI's agent runs (reading on request works, but nothing was pressed); what
`0x40` carries; the report rate; what the JAM shows after DECKARD closes. `docs/maschine-jam.md` has the
checklist.
