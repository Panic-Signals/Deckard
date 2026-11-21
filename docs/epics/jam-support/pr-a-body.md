# PR A — the JAM's overlay, in a window of its own

The body for the pull request to `Panic-Signals/Deckard`, branch `jam-oso`, cut from `upstream/main`
(`f2b5ea3`). Fill the manual's number in **Manual** when the manual pull request opens, and delete this
paragraph.

---

## Type

- [x] Enhancement

## Summary

Touching a Maschine JAM's encoder now opens the overlay its manual describes — the page's settings,
in a small window of its own over the app — instead of the single line in the status bar it used to
be. The Variation page comes with it, including a third **Mode**, **Euclid**, that varies a sound's
notes with DECKARD's own generator.

## How to check it works

A JAM is not needed: `--jam` presses named keys on a machine with no hardware, and `--jam-shot` draws
the overlay into the app window so `--screenshot` can capture it.

1. `cargo run -- --jam SHIFT --jam PAD_MODE --jam-shot --screenshot /tmp/overlay.png` — the Scale
   page appears beside the app: `Scale 1/2` in its bar, eight columns, the cursor rule under Root
   Note. Before this branch, the same keys put one line in the status bar and nothing on screen
   showed the page.
2. `cargo run -- --jam TEMPO` — the Tempo page, one slot, `120.00`. It stays until TEMPO again or
   another view.
3. `cargo run -- --jam SHIFT --jam E` — the focused sound's Variation page: Mode, Apply, Velocity Lo,
   Velocity Hi, Step. Static too; the encoder's press on Apply varies the notes.
4. With a JAM plugged in: touch the encoder in Keyboard and wait two seconds — the page goes; touch
   the encoder again — it comes back, because it is still that view's page. Press PAD MODE with any
   page up — it goes, the static pages included.
5. **Preferences ▸ Hardware ▸ Show the on-screen overlay** off — nothing is drawn, and the encoders
   and lights behave as they did.

Automated, on this branch's tip: `cargo test --workspace` — 64 suites, 2566 passed, 0 failed, 35
ignored (the ignored ones need local data, as always without a Maschine install). The overlay's own
path needs no NI install at all (`jam_oso_pages_need_no_ni_install`), which is what CI runs.
`scripts/check-platforms.sh` type-checks the workspace for `x86_64-unknown-linux-gnu` and
`x86_64-pc-windows-gnu`, both clean, so the second viewport compiles beyond macOS. Patch coverage is
76% of 1079 changed lines (the 70% gate).

## Platforms

- [x] All
- [ ] macOS
- [ ] Linux
- [ ] Windows

Tried on: macOS (arm64), the window with and without a JAM; the steps above with no hardware.

## Manual

Panic-Signals/panic-signals-site#<n> — the JAM chapter, which now covers the overlay as a window, and
the Variation page's third Mode.

## Notes for the reviewer

- **Where the page lives.** The script owns the page and each slot's behaviour; the host stores the
  published page, draws it, and hands every ENCODER press and turn back to the script.
  `docs/epics/jam-support/oso-contract.md` is the contract, and every obligation in it names the test
  that holds it.
- **One cursor.** The renderer keeps no state: it draws the cursor the script published. Earlier
  drafts had the viewer settle a cursor onto the nearest filled slot, which underlined a different
  setting from the one the status line named.
- **The window** (`apps/viewer/src/jam_window.rs`) is a second eframe viewport, chromeless and always
  on top, with mouse passthrough: the strip belongs to the encoder, so nothing in the window takes a
  click except its own bar, which drags it. Its place is saved as an offset from the app window and
  clamped, so it cannot be restored off-screen.
- **The rest of the diff is not the overlay.** `crates/doc/src/groove.rs` is the largest file here
  because the Variation page needs an engine behind its Mode, so the page and the engine travel
  together: Humanize (velocity band, time shift, length scale), Random (the generator's Random, the
  same call Generate ▸ Random makes), and Euclid. The settings are the sound's and are saved with the
  project; unset, the engine behaves as before.
- **Deliberate departures**, all in the `docs/maschine-jam.md` table: Euclid as a third mode; the
  settings belong to the sound; and no mark in the window's bar — the reference has NI's wordmark and
  DECKARD does not wear another company's logo.
- **Not here:** the virtual MK3/JAM panel work (separate pull request), `INSTANCE`, the desktop
  `BROWSE` overlay, and NI-scripted controllers' own overlay path, which stays in `lua-host`.
- `--jam-shot` is not only for review: a second viewport is not in the main window's capture, so it
  is how the strip's own pixels can be looked at in a screenshot at all.
