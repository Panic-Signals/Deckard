# 27 — `surface` becomes device-aware, and the Jam boots with no NI install

Status: `superseded` (2026-11-20) — the goal is met by another route; nothing needs `Surface` device-aware.

Part of the Jam script port (`jam-script-port.md`), slice 1.

## What

`sources::DEVICE` is `pub const DEVICE: &str = "mk3"` (`crates/surface/src/sources.rs:11`), used in
exactly three places: `device.rs:85` (`ids.retain(|id| id != sources::DEVICE)`), `sources.rs:76`
(the `Sources` default), and `api.rs:86` (the scripts' `device()`). Make the model a parameter of
the surface, keeping `"mk3"` as the default so nothing about the MK3 path changes.

Then add `crates/surface/lua/devices/jam/` — a bootable Jam script set covering the smallest
coherent behaviour: transport, pads, and LEDs. Not the strip (that is item 28), not the touchstrip.

The existing `devices/<model>` lookup in `sources.rs` already resolves a per-device path before the
shared one, so the layout is there; what is missing is the model being chosen at runtime and the
Jam files existing.

## Why slice 1 is first

It is the only slice that can prove the direction without writing the strip: if a Jam script set
boots, takes pad and transport input, and drives LEDs with `DECKARD_SCRIPTS` unset, then the port
is viable and everything after it is scope. It also produces the CI gate the rest of the port needs,
because today every Jam test is `#[ignore]`d behind a real install.

## Tasks

1. `sources::DEVICE` → `Sources::model` (or equivalent), defaulted to `mk3`; `api.device()` returns
   the configured model. No behaviour change for MK3.
2. Choose the model from the controller the app opens (`Controller` already knows M3 vs MJ1), so a
   Jam session loads `devices/jam/` and an MK3 session loads `devices/mk3/` or the shared pages.
3. Write `lua/devices/jam/global.lua`: page stack, button routing from `crates/jam`'s `BUTTONS`,
   pads, transport. LEDs from `crates/jam`'s `leds.rs` tables.
4. Register the files in `sources.rs::SHIPPED`, so they are embedded and user-overridable exactly
   like the MK3's.
5. Fail loudly if both a DECKARD Jam script set and NI's Jam tree are selected: one model, one
   script set, no half-and-half.
6. CI: a test that boots the Jam surface with no `DECKARD_SCRIPTS` and asserts a pad press, a
   transport press, and an LED write land.

## Exit criteria

- Jam scripts boot with `DECKARD_SCRIPTS` unset: pad press opens a sound, PLAY starts the transport,
  an LED write is observed. Evidence is a test, not a screenshot.
- MK3 suite unchanged and green (the guard against the `DEVICE` change).
- The strip is still NI-driven if an install is present; this item does not remove that path yet.
- No NI file paths in the new script set.

## Depends on

Nothing. Can start immediately. `crates/jam` (ids, LEDs) is already on `main`.

## Risks

- The `DEVICE` change is small but wide: MK3 pages under `lua/pages/` are shared. Read the device
  lookup in `sources.rs` before editing; the per-device-before-shared order is load-bearing.
- Do not reimplement NI's page stack faithfully in this slice. Only what the exit criteria name.

## Superseded (2026-11-20)

The goal — the Jam boots and runs with **no NI install** — is met, but not the way this item chose.
Nothing needs `Surface` to be model-parameterised: the Jam is hosted by upstream's
`surface::device::Device`, which takes the script id as an argument (`DEVICE: &str =
"maschine_jam"` in `apps/viewer/src/jam_io.rs`). `sources::DEVICE` stays the `"mk3"` const and
`Surface` keeps booting the MK3's script set only — verified, `with_model` exists nowhere.

The `devices/jam/` script set this item wanted also exists, under another name:
`crates/surface/lua/devices/maschine_jam/` (upstream PR #34), with item 28's strip built on it.

**What stays true and worth keeping:** there are two device hosts, so anything MK3-specific added to
`Surface` gains no Jam coverage by default. That is now a fact to keep in mind, not a task.
