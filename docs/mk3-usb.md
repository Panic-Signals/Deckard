# Maschine MK3 over USB

Learned from the hardware with `crates/mk3/examples/mk3-probe.rs`, `mk3-learn.rs` (button
bits, spoken prompts) and `mk3-learn2.rs` (pads, encoders, touch strip). Driver:
`crates/mk3` (decoding), `apps/viewer/src/mk3_io.rs` (thread, pads straight to the engine,
events to NI's controller scripts).

## Device

VID 0x17CC, PID 0x1600. Interfaces: 0–2 USB audio, 3 MIDI, **4 HID** (vendor page
0xFF01; input reports 0x01/0x02/0x20/0x40, output 0x80/0x81, features 0xD0–0xFF),
**5 "BD"** (vendor bulk — the displays, held by NIHostIntegrationAgent), 6 DFU. The HID
interface opens shared (`hidapi`, non-exclusive) next to NI's agent; reading it needs the
terminal/app to have Input Monitoring permission on macOS.

## Input report 0x01 (42 bytes)

| bytes | |
|---|---|
| 1–10 | 80 button bits (bit n = byte 1 + n/8, mask 1 << n%8); names in `deckard_mk3::BUTTONS` |
| 11 | low nibble: 4-D encoder position, +1 per clockwise click, wraps 0–15 |
| 12–27 | knobs 1–8, u16 LE, 0–999 per turn, wraps |
| 28–29 | touch-strip timestamp |
| 30–31 | touch strip finger 1, 0–1023 (0 = not touched) |
| 32–33 | finger 2 (assumed) |
| 34–41 | unknown (constant: 00 00 da 0f 00 00 57 0f) |

Button bits: 0 encoder push, 2–5 encoder up/right/down/left, 6 SHIFT, 7 display button 8
(verify), 8–15 groups A–H, 16 NOTES, 17 VOLUME, 18 SWING, 19 TEMPO, 20 NOTE REPEAT,
21 LOCK, 24 PAD MODE, 25 KEYBOARD, 26 CHORDS, 27 STEP, 28 FIXED VEL, 29 SCENE,
30 PATTERN, 31 EVENTS, 33 VARIATION, 34 DUPLICATE, 35 SELECT, 36 SOLO, 37 MUTE,
38 PITCH, 39 MOD, 40 PERFORM, 41 RESTART, 42 ERASE, 43 TAP, 44 FOLLOW, 45 PLAY, 46 REC,
47 STOP, 48 MACRO, 49 SETTINGS, 50 page right, 51 SAMPLING, 52 MIXER, 53 PLUG-IN,
56 CHANNEL, 57 ARRANGER, 58 BROWSER, 59 page left, 60 FILE, 61 AUTO, 64–70 display
buttons 1–7, 71 encoder touch, 72–79 knob touch 8…1. Unused: 1, 22, 23, 32, 54, 55,
62, 63.

## Input report 0x02 (pads)

Three-byte entries until an all-zero one: HID pad index, then u16 BE: top nibble = event
(1 press, 4 pressure update, 3 release; a pad pressed slowly, starting light, comes as
0 and goes as 2), low 12 bits = pressure (0–4095). HID index i is
the pad printed `(3 − i/4)·4 + i%4 + 1` (index 12 = pad 1, bottom left). DECKARD takes a
hit's velocity as the peak pressure over its first three reports, or what came in 4 ms:
the MK3 reports pressure only as it changes, so a pad pressed and held steady (often,
with others held) may send no more. Pressure reports from a pad not held are ignored: a
released pad keeps reporting as it springs back. The pressure goes through the pad sensitivity (Preferences > Hardware) before
it's a velocity. `examples/mk3-pad-log.rs` prints the raw entries as they come.

## Scripts

Events go to NI's MK3 scripts as `onSwitchEvent(NI.HW.<button>, pressed)`,
`onPadEvent(pad, trigger, value)`, `onEncoderEvent(NI.HW.ENCODER_SCREEN_n, turns)`
(knob change / 1000), `onWheelEvent(0, clicks)`, `onTouchEvent(finger, 0 touch | 1 move
| 2 release, 0..1)`.

A knob resting between two counts can flicker by one (+1, −1, …). Counts a knob turns
its way pass at once; counts back against its last direction pass only once there are two
(`Decoder::knob_moved`), so a resting knob turns nothing (the Sampling page's zoom
re-centred the waveform on every flicker).

## Not done yet

LEDs (output reports 0x80, 62 bytes, and 0x81, 41 bytes: layout to learn), the two
480×272 displays (bulk interface 5), knob-touch handling. What a held pad's pressure
does is its sound's Pad Touch (KEYBOARD, pages 3 and 4, from DECKARD's built-in mod;
`docs/superpowers/specs/2026-10-24-pad-touch-design.md`; `apps/viewer/src/mk3_io.rs`,
`Touching`): Velocity sets Note Repeat's velocity; Poly AT, Aftertouch and CC play at
Fixed Velocity and send the pressure, between the sound's Min and Max through its
curve, live and into the recording.

## LEDs (output reports)

Learned with `crates/mk3/examples/mk3-led-learn.rs`; the byte order is in
`crates/mk3/src/leds.rs` (`BUTTON_LEDS`, `PAD_LEDS`).

- **0x80** (63 bytes): report id, then 62 button LEDs: CHANNEL … MACRO, the eight screen
  buttons, VOLUME … NOTES, groups A–H, transport, SHIFT, the pad-mode row, the edit row,
  and the 4-D encoder's four arrows (up, left, right, down).
- **0x81** (42 bytes): report id, then the touch strip's 25 LEDs (left to right), then the
  16 pads from the top row (13–16, 9–12, 5–8, 1–4).

Single-colour LEDs take a brightness 0..0x7f. Colour LEDs (groups, pads, strip) take a
palette byte, `(colour + 1) << 2 | shade`: colour is the scripts' `LEDColors` index
(0 red … 15 fuchsia, 16 white); shade 0 dimmest … 3 brightest. Shades 0 and 2 and the
colours were confirmed on the hardware (`examples/mk3-led-check.rs`); each touch-strip LED
takes its own colour. DECKARD sends dim as shade 0 and every brighter state as 3, as
Maschine's MK3 LED table gives bright and flash the full level; button LEDs that are off
keep a faint backlight (0x09, picked on the hardware) so their labels can be read; dim ones (can be pressed, not
selected) are 0x22, 30% under Maschine's 0.25.

## Other reports (from the HID report descriptor)

`examples/mk3-hid-reports.rs` lists them; `examples/mk3-hid-features.rs` reads the feature
reports (reading changes nothing).

- **Output:** 0x80 and 0x81 (the LEDs, above); 0xf3 (1 byte) and 0xf4 (32 bytes), unknown.
- **Feature 0xf8 / 0xf9:** the left and right display's settings, read as
  `e0 01 10 01 60 01 14 32 00`: width 480 and height 272 (u16 LE), then single bytes.
  **Byte 8 is the backlight, 0..100**; the controller starts at 20. Writing it to either
  report sets both displays (confirmed on the hardware: the screens got as bright as when
  Maschine drives them). DECKARD sets it on connecting (`Device::set_display_brightness`,
  the `display_brightness` setting, default 100). The other bytes are unknown.
- **Feature 0xd0, 0xd8–0xdb, 0xf0, 0xf2, 0xfe, 0xff:** unknown; 0xd9 holds an ASCII
  number (a serial?), 0xfe/0xff are 209 bytes (firmware?). Not written.

## Displays (bulk interface 5, endpoint 0x04)

Two 480×272 screens, RGB565 big-endian. Learned with `examples/mk3-display-test.rs`;
driver `crates/mk3/src/display.rs`. A transfer draws one rectangle of one display:

- header (16 bytes): `84 00 <display 0|1> 60 00 00 00 00`, then x, y, width, height as
  u16 BE;
- commands, 4 bytes each: `00 nn nn nn` followed by nn pixel pairs (sent as is);
  `01 nn nn nn` followed by one pixel pair, repeated nn times; `02 nn nn nn` skip nn
  pairs (left as they are; NI's software sends whole-screen rectangles of skips and
  small changes); `03 00 00 00` blit; `40 00 00 00` end.

NI's NIHostIntegrationAgent claims the interface while it runs; DECKARD gets the displays
when it isn't running (it isn't installed on a machine without NI software). The driver
sends only the rows that changed.

What's shown is drawn by `crates/nwl` from NI's own style sheets, pictures and fonts in
the install (`MaschineMK3Styles.txt` and its imports) and the scripts' widget tree
(`deckard_lua_host::screen`).

### Reference frames from NI's software

`tools/mk3-capture/run.sh` runs a re-signed copy of NIHostIntegrationAgent (NI's install
is untouched) with `capture.c` loaded, which records every USB pipe write; start
Maschine, use the controller, Ctrl-C. `cargo run -p deckard-mk3 --example
mk3-capture-decode -- capture.bin DIR` turns it into PNGs of both screens (`display::Decoder`),
one per settled screen. Captures are kept in `out/mk3-reference/` (from the user's own
machine, never bundled).
