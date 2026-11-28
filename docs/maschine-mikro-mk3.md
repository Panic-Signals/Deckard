# Maschine Mikro MK3

DECKARD drives a Maschine Mikro MK3 over USB HID with no NI software. Every button press,
encoder turn, pad strike and touch-strip event is handled by its own clean-room driver
(`crates/mikro`) and Lua surface scripts
(`crates/surface/lua/devices/maschine_mikro_mk3/`). The integration point with the app is
`apps/viewer/src/mikro_io.rs`. All protocol knowledge comes from open-source community work
and independent USB capture analysis. No proprietary Native Instruments code or binaries are
used at any point.

**Status, 2026-11-28.** Full hardware parity for all pad modes, the step sequencer, the
library browser, transport controls, parameter controls (VOLUME / SWING / TEMPO and their
SHIFT secondaries), Perform FX, Mute, Solo, Group banking, keyboard and chords modes, and
the OLED status screen. Known gaps are listed at the end of this file.

---

## USB Device

VID `0x17CC`, PID `0x1700`. Unlike the full Maschine MK3, the Mikro has no built-in audio
interface and no secondary bulk display endpoint. All control and display I/O goes through
a single USB HID interface (Interface 4).

Under Linux, raw HID access (`/dev/hidraw*`) requires a udev rule:
`scripts/linux/50-deckard-maschine-mikro-mk3.rules`.

For the full byte-level protocol see [`docs/mikro-usb.md`](mikro-usb.md). The crate-level
rustdoc in `crates/mikro/src/lib.rs` has an inline summary.

---

## Crate: `deckard-mikro` (`crates/mikro`)

| File | Contents |
|---|---|
| `src/lib.rs` | Input decoder: report `0x01` (buttons / encoder / strip), report `0x02` (pad events). Button bitmask table, `BUTTONS` constant. |
| `src/leds.rs` | Output report `0x80` (81 bytes): 39 single-channel button LEDs, 16 palette-indexed pad LEDs, 25 touch-strip segment LEDs. Brightness levels (`off` → `dim` → `on`), colour palette lookup. |
| `src/screen.rs` | OLED output: two report-`0xe0` packets (265 bytes each) covering 128 × 32 pixels at 1 bpp, packed left-to-right, top-to-bottom. Primitives: pixels, horizontal/vertical lines, filled rectangles, text (5 × 7 bitmap font, standard size and 2× scaled), progress bars, scrolling marquee. |

### Input reports

**Report `0x01` (64 bytes) — buttons, encoder, touch strip**

| Bytes | Content |
|---|---|
| 1–6 | 48 button bits: `bit n` → `byte[1 + n/8]`, mask `1 << (n%8)` |
| 7 | Encoder relative delta — treat as `i8`, positive = clockwise |
| 10 | Touch strip: `0` = untouched, `1–200` = finger position left → right |

**Report `0x02` (64 bytes) — pad events**

Three-byte entries, terminated by an all-zero entry:
- byte 0: pad HID index (0–15)
- byte 1 high nibble: event type (`0x0`/`0x1` = press/NoteOn; `0x2`/`0x3` = release/NoteOff; `0x4` = aftertouch)
- byte 1 low nibble + byte 2: 12-bit pressure (0–4095)

### Output reports

**Report `0x80` (81 bytes) — all LEDs**

```
byte  0:     report id 0x80
bytes 1–39:  39 button LEDs  (single-channel brightness 0x00–0x7f)
bytes 40–55: 16 pad LEDs     (colour palette index 0x00–0x7f)
bytes 56–80: 25 strip LEDs   (single-channel brightness 0x00–0x7f)
```

**Reports `0xe0` × 2 (265 bytes each) — 128 × 32 OLED**

Each packet covers one half of the display (top/bottom 128 × 16 rows). The screen module
(`crates/mikro/src/screen.rs`) assembles a full 512-byte frame and splits it.

---

## Surface Scripts (`crates/surface/lua/devices/maschine_mikro_mk3/`)

| File | Responsibilities |
|---|---|
| `main.lua` | Top-level device definition, `M.open` / `M.button` / `M.pad` / `M.strip` / `M.encoder` / `M.pads` / `M.leds`. All mode transitions, transport controls, Group operations, quick-param (VOLUME / SWING / TEMPO), Mute, Solo, Erase, Select, keyboard/chords pad output, step sequencer paging. |
| `overlays.lua` | Named overlay sub-states: `scene`, `pattern`, `events`, `grid`, `variation`, `navigate`, `plugin`, `macro`, `duplicate`, `perform`. Each overlay registers its own button, pad, encoder, and LED handlers that override `main.lua`'s while active. |
| `screen.lua` | 128 × 32 OLED renderer. `S.draw()` dispatches to per-mode drawing routines: `draw_mode`, `draw_browse`, `draw_quick_param`, `draw_perform`, `draw_step`, `draw_events`, `draw_plugin`, `draw_navigate`, `draw_duplicate`, plus the idle/status bar. |
| `browse.lua` | Library browser overlay: encoder to scroll, encoder press to load/prehear, STAR to toggle favourites filter, pads inactive while browsing. |
| `plugin.lua` | Plug-in / Macro overlay: eight knobs via encoder + LEFT / RIGHT, SHIFT + MACRO sets macro targets, unlatches `quick_param`. |
| `device.lua` | Manifest: device name, VID/PID, udev rule path. |

### State object (`st`)

The module-local `st` table persists across callbacks:

```
st.mode          -- "pad" | "keyboard" | "chords" | "step"
st.overlay       -- nil | "scene" | "pattern" | "events" | "grid" | "variation"
                 --      | "navigate" | "plugin" | "macro" | "duplicate" | "perform"
st.octave        -- integer –3..3  (keyboard / chords)
st.step_page     -- 0-based 16-step page index (step sequencer)
st.quick_param   -- nil | "volume" | "swing" | "tempo" | "velocity" | "position" | "tune"
st.quick_target  -- nil | {kind="sound"|"group"|"master", index=…}
st.quick_info    -- nil | {label, value_str, target_str}  (OLED cache)
```

---

## Pad Modes

### Pad Mode (default)

Pads 1–16 correspond to Sounds 1–16 in the focused Group.

| Interaction | Result |
|---|---|
| Strike | Play sound at recorded velocity (12-bit pressure → velocity curve) |
| Strike + hold | Sustain; aftertouch pressure sent as channel pressure |
| FIXED VEL | Force velocity to 100 regardless of strike pressure |
| Pad LED color | Group palette color (`grp.palette`), full brightness while struck, `dim` otherwise |
| Pad LED on empty Sound | White `dim` |

### Keyboard Mode

Pads 1–16 lay out a chromatic scale starting from the current root note at the current
octave. The layout matches the standard Maschine Mikro MK3 layout: pad 1 = root, pads 2–4
= scale degrees, etc. Pads outside the current scale are shown `dim`, in-scale pads `on`.

| Interaction | Result |
|---|---|
| LEFT / RIGHT | Move octave down / up (range −3 to +3); LED `on` while held if move is possible |
| Pad strike | Play focused Sound at the pad's pitch |
| Encoder (while in Keyboard mode) | Fine-tune root note |

Arrow LEDs: `on` when held and the octave can move in that direction; `dim` otherwise.

### Chords Mode

Same pad layout as Keyboard but each pad plays the full chord for that scale degree. The
chord voicing follows the focused scale type. Octave transposition and arrow LEDs behave
identically to Keyboard mode.

### Step Sequencer Mode

Shows the focused Sound's steps for the current 16-step page of the focused Pattern.

| Pad state | LED |
|---|---|
| Empty step | `off` (completely dark — no pad should glow when there is no step) |
| Step with note | Group palette color (`grp.palette`), `on` |
| Playback position (while playing) | `WHITE`, full brightness (overrides step color) |

| Control | Result |
|---|---|
| Pad 1–16 | Toggle step on / off |
| LEFT / RIGHT | Page through 16-step segments; LED `on` while held |
| Encoder | Adjust step resolution (grid size) |

OLED shows current page, bar count, and step resolution.

---

## Overlays

Overlays are transient sub-states that stack on top of the current pad mode. Pressing an
overlay button again, or pressing a conflicting overlay, dismisses it.

### Scene

Pads 1–8 select / create Scenes. The focused Scene pad glows at full brightness; others
glow `dim`. Encoder scrolls the scene list when more than 8 exist.

### Pattern

Pads 1–16 select / create Patterns in the focused Group. Focused Pattern = `on`; others
in the scene = `dim`; empty slots = `off`.

### Events (note micro-editing)

Encoder turns adjust the event property selected by the top-row buttons:

| Button | Property |
|---|---|
| Pad 1 (POS) | Note position (nudge by grid) |
| Pad 2 (PITCH) | Note pitch (semitone) |
| Pad 3 (VEL) | Note velocity |
| Pad 4 (LEN) | Note length |
| Pad 5 (PROB) | Note probability (0–100 %) |
| Pad 6 (EVENT) | Event type selector |

LEFT / RIGHT move the selected event forward / backward by the grid.

### Grid

Hold GRID: encoder selects the step resolution. Pad 1–8 show common grid values (`1/1`
through `1/32T`). Releasing GRID confirms and returns.

### Variation

Encoder adjusts the Groove Variation amount for the focused Sound. OLED shows the
percentage.

### Navigate

4 × 4 color-coded matrix mapped to Groups A–P (up to 16 groups). Each pad glows in that
Group's palette color. Press to focus. Encoder scrolls if more than 16 groups.

### Plug-in / Macro

PLUG-IN opens the instrument and effect chain. Pads 1–8 select chain slots. Encoder with
LEFT / RIGHT steps through pages of plug-in parameters (8 per page). Encoder press
loads / opens the focused slot.

MACRO (SHIFT + PLUG-IN) enters Set Macro mode: touching a parameter knob while in Set
Macro assigns it to the focused Macro slot.

### Duplicate / Double

DUPLICATE + source Pad: sets the duplicate source. DUPLICATE + target Pad: copies source
to target. Source pad flashes the Group color during selection; target confirms with a
white flash. DOUBLE doubles the Pattern length, filling the new half with the existing
content.

### Perform

PERFORM opens the Perform FX selector. SHIFT + PERFORM loads a new Perform FX (calling
`deckard.perform_fx()`) and opens the selector.

Pads 1–8 select the eight Perform FX types (Filter, Flanger, Burst Echo, Reso Echo, Ring,
Stutter, Tremolo, Scratcher). Encoder adjusts the focused FX parameter. Touch strip drives
the ENGAGE position for the active FX. LEFT / RIGHT scroll the parameter pages.

Strip LEDs show a 25-segment live position indicator while PERFORM is active.

---

## Transport & Hardware Controls

### PLAY / REC / STOP

| Button | LED (not held) | LED (held or active) | Action |
|---|---|---|---|
| PLAY | `dim` (always) | `on` while `s.playing` | Toggle playback |
| REC | `off` | `on` while `s.recording` | Arm / disarm recording |
| STOP | `off` | `dim` while `s.playing` | Stop playback; second press → move to start |
| RESTART | `dim` | `on` while `s.looping` | Toggle pattern loop |

> [!NOTE]
> The STOP LED is lit (`dim`) **while the transport is playing**, matching the hardware
> behavior documented in the Maschine Mikro MK3 manual. It is `off` when stopped.

### TAP Tempo

| State | LED |
|---|---|
| Idle | `dim` (always lit) |
| Held | `on` |
| Metronome active (`s.metronome`) | `on` |

Tapping sends a tempo tap event. SHIFT + TAP toggles the metronome.

### FOLLOW

FOLLOW LED is `on` when `s.follow` or while the Grid overlay is open (`st.overlay == "grid"`).

### Group Creation & Banking

| Interaction | Result |
|---|---|
| GROUP + empty Pad | Create new Group at that slot |
| GROUP alone | Enter group-bank mode; pads show bank A–D |
| SHIFT + GROUP + Pad | Jump to Group bank (A, B, C, D = banks of 8 Groups) |
| GROUP pad LED | Dim white for existing Groups; bright white for focused Group |

Arrow LEDs in GROUP bank mode: `on` while held and more banks exist in that direction.

---

## Parameter Controls (VOLUME / SWING / TEMPO and SHIFT variants)

### Tap-to-Latch

Each button toggles between *active* (latched) and *idle*:

- **First press** → mode becomes active, encoder now controls that parameter.
- **Second press** (same button) → mode returns to idle.
- **Press a different param button** → switches directly to the new mode.
- **SHIFT + VOLUME** → activates VELOCITY mode.
- **SHIFT + SWING** → activates POSITION mode.
- **SHIFT + TEMPO** → activates TUNE mode.

### LED States

| Mode state | LED |
|---|---|
| Active / latched | `on` |
| Inactive | `dim` |

### Context-Sensitive Targeting

| Held while turning encoder | Target |
|---|---|
| Nothing | Focused Sound (or Master for VOLUME) |
| Pad 1–16 | That Sound's volume or tune |
| GROUP + Pad 1–16 | That entire Group's volume or tune |

### OLED Status Screen (while active)

```
┌────────────────────────────────┐
│ TEMPO                          │
│ MASTER                         │
│ 124.0 BPM                      │
└────────────────────────────────┘
```

Header = parameter name (`TEMPO`, `VOLUME`, `TUNE`, `SWING`, `VELOCITY`, `POSITION`).
Line 2 = target (`SND 01`, `GRP A`, `MASTER`). Line 3 = formatted live value.

---

## Mute & Solo

### MUTE

MUTE + Pad: toggle Sound mute. Pad LED = `dim` when muted, `on` otherwise.
MUTE + GROUP + Pad: toggle Group mute.
MUTE LED = `on` while MUTE is held.

### SOLO

SOLO + Pad: solo that Sound — silence everything except that Sound.
SOLO + GROUP + Pad: solo that Group — silence all other Groups.
SOLO LED = `on` while SOLO is held.

> [!IMPORTANT]
> Solo silences **all other** Sounds or Groups (not just dims them). This is verified by
> `mikro_stop_led_and_solo_sound_and_group` in `crates/surface/src/mikro_tests.rs`.

---

## ERASE & SELECT

ERASE + Pad (in Step mode): clear all steps on that Sound.
ERASE + Pad (in Pad mode): reset that Sound to default.
SELECT + Pad: focus that Sound without playing it.

---

## SHIFT + Pads (Silkscreen Shortcuts)

| Pad | Silkscreen label | Action |
|---|---|---|
| 1 | UNDO | Undo one step |
| 2 | REDO | Redo one step |
| 3 | QUANTIZE | Quantize focused pattern |
| 4 | QUANTIZE 50% | Quantize at 50 % strength |
| 5 | CLEAR | Clear focused pattern |
| 6 | DOUBLE | Double pattern length |
| 7 | SEMI-UP | Transpose selection +1 semitone |
| 8 | SEMI-DOWN | Transpose selection −1 semitone |
| 9 | OCT-UP | Transpose selection +1 octave |
| 10 | OCT-DOWN | Transpose selection −1 octave |
| 11 | SET ROOT | Open root-note selector |
| 12 | SET SCALE | Open scale selector |
| 13 | IN LEFT | Trim pattern start right |
| 14 | IN RIGHT | Trim pattern start left |
| 15 | OUT LEFT | Trim pattern end left |
| 16 | OUT RIGHT | Trim pattern end right |

---

## Library Browser

BROWSE opens the library browser overlay. The OLED shows:

```
┌────────────────────────────────┐
│ ★ MASCHINE  [category]         │
│ ▶ Preset Name                  │
│   Preset Name 2                │
└────────────────────────────────┘
```

| Control | Result |
|---|---|
| Encoder (turn) | Scroll preset list |
| Encoder (press) | Load focused preset (or prehear if held briefly) |
| STAR | Toggle Favourites filter |
| LEFT / RIGHT | Navigate folder hierarchy (up / down) |
| Pads | Inactive while browsing |

**Prehear:** a brief encoder press triggers `deckard.preview()` on the focused item. The
first prehear after loading a Group initialises the prehear player; subsequent prehears
work immediately. This aligns with the prehear lifecycle in `App::sampling_part`.

**Folder-based type tags:** items whose folder path contains "One Shot" or "Loop" are
tagged accordingly without reading the file. Factory vs. user library is distinguished by
path prefix.

**Favourites:** `STAR` toggles `deckard.browser_star()`. The filter persists until toggled
off or BROWSE is closed.

---

## Known Limitations & Follow-up Items

### NOTE REPEAT

The initial NOTE REPEAT framework is in place: the button opens the Note Repeat / Arp
page, and the hardware strip drives repeat rate. Full engine-synchronized repeat timing
(subdivisions aligned to the song position, not wall-clock) is still being verified
against hardware behavior and is tracked in `docs/note-repeat-arp.md`.

### NOTES (Strum / Smart Strip)

The NOTES button activates Smart Strip strum mode. The basic strumming dispatch (`Host::strum`,
`ControlRequest::StripNote`) is working. The strumming velocity *curve* (the nonlinear
pressure-to-velocity mapping over the strip length that the hardware applies) is not yet
fully parity-aligned with the reference hardware output. Tracked as a follow-up in
`docs/CONTROLLER-BUGS.md`.

### Open Controller Bug Tracker Items (Mikro-relevant)

| # | Area | Description |
|---|---|---|
| C62 | Keyboard | Notes the sequencer plays do not light their pads in Keyboard mode while a pattern is running |
| C60 | Plug-in | A bypassed effect doesn't look visually bypassed in the plug-in chain |

---

## Test Coverage

| Test file | Suite | Count (2026-11-28) |
|---|---|---|
| `crates/surface/src/mikro_tests.rs` | `deckard-surface` | 329 passed, 0 failed |
| `apps/viewer/src/mikro_tests.rs` | `deckard` (mikro filter) | 24 passed, 0 failed |
| `crates/mikro/src/lib.rs`, `leds.rs`, `screen.rs` | `deckard-mikro` | 18 passed, 0 failed |

Selected integration tests:

- `mikro_stop_led_lit_while_playing_and_solo_sound_dispatches_action` — STOP LED and SOLO routing
- `mikro_keyboard_mode_lights_octave_arrow_leds` — arrow LEDs in keyboard / chords
- `mikro_step_mode_lights_only_active_steps_in_group_color` — step sequencer pad colors
- `mikro_perform_and_shift_perform_selects_perform_fx` — PERFORM overlay and SHIFT + PERFORM
- `mikro_quick_param_tap_latch_hold_targeting_and_oled` — VOLUME / SWING / TEMPO latch & OLED
- `mikro_tap_button_led_is_lit_and_brightens_on_hold_or_metronome` — TAP LED states
- `mikro_tap_led_lit_and_shift_tap_toggles_metronome` — SHIFT + TAP metronome toggle

---

## Related Documents

- [`docs/mikro-usb.md`](mikro-usb.md) — full USB HID protocol reference
- [`docs/controller-api.md`](controller-api.md) — Lua surface script API
- [`docs/PARITY.md`](PARITY.md) — full Maschine 3 parity status
- [`docs/CONTROLLER-BUGS.md`](CONTROLLER-BUGS.md) — open controller bug tracker
- [`docs/note-repeat-arp.md`](note-repeat-arp.md) — Note Repeat and Arpeggiator details
- [`docs/fx-perform.md`](fx-perform.md) — Perform FX DSP and strip integration
