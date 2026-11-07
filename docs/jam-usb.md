# Maschine JAM over USB

Learned from one unit on Windows, with the tools in `crates/jam/examples/`. The design is
`docs/superpowers/specs/2026-11-05-maschine-jam-design.md`.

**Status, 2026-11-05.** Everything marked *measured* was read from the device. The map from bits
and bytes to named controls (`crates/jam/src/map.rs`) comes from the open-source
[cabl](https://github.com/shaduzlabs/cabl) driver (MIT, `src/devices/ni/MaschineJam.cpp`), and
nobody has yet pressed each control against it. It agrees with everything measured: 120 button
bits (cabl names 117, leaving room for the footswitch's two and a spare), LED bytes of 37 + 16 +
80 + 88, two unused button LEDs that account for the manual's 35, and a palette in the same order.
Facts about the protocol are all that's taken; no code is. `jam-check` is the check by hand.

## Device

VID 0x17CC, PID 0x1500. Interfaces: **0 HID** (vendor page 0xFF01; Windows' own `HidUsb`
driver), 1 DFU (NI's `nimjcusb`: firmware update, never touched). There's no audio, MIDI or
bulk interface, and no screens. The HID interface opens shared next to NI's agent.

## Input

Reports come only when something changes: 20 seconds untouched brought none (measured).
Asking for one (`HidD_GetInputReport`) works with NI's agent running.

### Report 0x01 (16 bytes): buttons

The descriptor cuts it into two 4-bit fields, then **120 one-bit fields** (measured). All
zeros at rest. The manual's controls come to 119 switches: 64 pads, 8 scene and 8 group
buttons, 35 others, the encoder's push and touch, and the footswitch's tip and ring. So
every switch has its own bit, with one spare. The low 4-bit field is the encoder's position
(from cabl), wrapping round 16 places; what the high one carries is unknown.

### Report 0x02 (48 bytes): Smart Strips

24 u16 values (measured), which is 8 strips of three: 6 bytes a strip. That's the MK3 strip's
record: a timestamp, then two fingers. At rest the first value of most strips is nonzero and
differs from strip to strip, and the other two are 0, which is the MK3's "not touched" (measured).
cabl reads the second value as a little-endian position over 1024. Which end is 0 is still to
confirm by touching them.

### Report 0x40 (24 bytes)

12 u16 values, all zero at rest (measured). Unknown. Twelve values don't fit 64 pads, and
the pads have only their bits in 0x01, so the click-pads very likely report on and off with no
pressure.

## Output

| Report | Bytes | Fields (measured) | Inferred |
|---|---|---|---|
| 0x80 | 53 | 37, then 16 | Plain LEDs, then the two 8-LED meters. The manual gives 35 lit buttons; cabl leaves two bytes unused, after IN1 and CUE, probably the lights of their SHIFT labels IN 2 and MIDI. |
| 0x81 | 80 | 80 | 64 pads, 8 scene, 8 group |
| 0x82 | 88 | 88 | 8 strips × 11 LEDs |
| 0xF4 | 3 | 3 | Unknown. Not written. |

The device accepted frames on 0x80, 0x81 and 0x82 with NI's agent running (measured).

**NI's services write these lights too.** With `NIHardwareService` and `NIHostIntegrationAgent`
running and nothing else using the JAM, a pad tap turned the pads and strips orange at different
levels, and a strip's lights followed the finger. Stopped, nothing reacted (watched by webcam,
2026-11-05). It looks like the JAM's MIDI mode, though that isn't proven. With DECKARD running too,
each overwrites the other, and DECKARD only sends a report when its own lights change, so the
wrong lights stayed. `apps/viewer/src/ni_services.rs` offers to stop them when a JAM connects.

## The palette (feature reports 0xF1, 0xF2)

Each is 204 bytes: **17 colours × 4 shades × RGB** (measured), running red round to fuchsia,
then white. A colour LED takes `(colour + 1) × 4 + shade`, colour 0 red … 15 fuchsia, 16 white
(cabl's table agrees). The shades run faint, half, full, then a paler full: in 0xF1 the fourth of
every colour is paler than the third, in 0xF2 the third and fourth are equal. The manual's
half-lit is shade 1. Which report drives which LEDs isn't known. Read only: nothing here writes
them.

## Other feature reports

0xD0 reads all 0xFF. 0xD8 and 0xDA–0xDD hold data that looks specific to the unit, and 0xD9 an
ASCII number (a serial?). Their values aren't recorded here. Not written.

## Checking and learning

Close Maschine first. The quick check: `jam-check` says the name the map gives whatever you press,
and lights it.

```
cargo run -p deckard-jam --example jam-check
```

If the map turns out wrong altogether, these find it from scratch. The prompts are spoken
(`JAM_SILENT=1` turns that off) and there's no time limit.

```
cargo run -p deckard-jam --example jam-learn        # every control: writes jam-map.txt
cargo run -p deckard-jam --example jam-led-learn 80 # plain LEDs and meters: jam-leds.txt
cargo run -p deckard-jam --example jam-led-learn 81 # pads, scenes, groups
cargo run -p deckard-jam --example jam-led-learn 82 # strips: two strips' worth shows the pattern
cargo run -p deckard-jam --example jam-led-colors   # the palette and brightness: jam-colors.txt
```

`jam-probe` watches the input reports and writes nothing, so it's safe next to Maschine;
`jam-hid-reports` lists the descriptor. Only 0x80, 0x81 and 0x82 are ever written.
