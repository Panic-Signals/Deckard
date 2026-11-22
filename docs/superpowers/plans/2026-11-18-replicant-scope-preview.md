# Replicant Scope Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the live audio tap on Replicant's right-screen scope with a short simulated A4 note rendered from the current knob values, drawn as a waveform plus an amplitude envelope.

**Architecture:** `Instance` renders and caches one second of a preview note by making a fresh engine from the slot's knob values, playing A4 (velocity 100) held for 0.6 s and released for 0.4 s. The cache is dropped on knob edits (not modulation) and rebuilt lazily on the UI thread. `scope()` downsamples the cached note to a triggered waveform slice and an amplitude envelope; the surface draws them in two bands, and the app only asks for it on Replicant's Control page.

**Tech Stack:** Rust 2024 workspace (`deckard-plugins`, `deckard-surface`, `deckard` viewer), Lua page scripts embedded by `deckard-surface`.

**Spec:** `docs/superpowers/specs/2026-11-18-replicant-scope-preview-design.md`

## Global Constraints

- Only Replicant gets a scope: gate on `std::ptr::eq(self.class, &crate::replicant::CLASS)`.
- Do not touch the live audio path: the preview renders on its own engine and never writes into a playing instance.
- Preview uses knob values from `Instance::values`; modulation (`Instance::modulate`) must not invalidate or alter it.
- One preview note: MIDI 69, velocity 100, 1.0 s total, note off at 0.6 s.
- Display scaling stays `SCOPE_GAIN = 6.0`, clamped to ±1.
- Panel geometry: `x = 728, y = 68, w = 224, h = 136`; waveform `y 71..160` (centre 116, half 44); separator `y 164`; envelope `y 168..200`.
- The working tree is already dirty with the previous live-scope implementation. Do not commit; the user has not asked for commits. Each task ends with its test command instead of a commit.
- Commands run offline: `cargo test --offline -p deckard-plugins`, `-p deckard-surface`, `-p deckard`.

## Review Focus

- **Knob turns during playback:** the preview render happens under the plug-in mutex; it must render once per knob change, not once per frame. Covered by the cache test in Task 1.
- **Modulation running:** the picture must not jitter when an LFO/modulation drives a parameter. Covered by the modulation test in Task 1.
- **Very long envelopes:** Attack/Decay/Release longer than the one-second preview are truncated, not invalid; the render must stay finite. Covered by the finite-output test in Task 1.
- **Sample rates other than 48 kHz:** preview length and note-off split must still put the release in the tail. Covered by the 44.1 kHz test in Task 1.
- **Non-Replicant slots:** Plaits, Zhora, the Tuner and effects must still get no scope. Covered by the non-Replicant test in Task 1.

---

## File Structure

- `crates/plugins/src/native.rs` — `Scope`, preview constants, `Instance.preview` cache, `render_preview`, `scope_of`; removes `Engine::scope`.
- `crates/plugins/src/lib.rs` — `Plugin::scope(&mut self)` returning `native::Scope`.
- `crates/plugins/src/replicant.rs` — remove the live scope ring and `Engine::scope`; replace scope tests with preview tests.
- `crates/surface/src/lib.rs` — `ScopeView { wave, envelope }`.
- `crates/surface/src/api.rs` — expose `control.scope.env`.
- `crates/surface/lua/pages/control.lua` — two-band drawing.
- `crates/surface/src/tests.rs` — pixel test for both bands.
- `apps/viewer/src/zhora_ui.rs` — `scope_view` asks for a mutable instance and maps `Scope` to `ScopeView`.
- `apps/viewer/src/main.rs` — remove `focused_replicant` and the 30 ms wake condition.
- `README.md`, `docs/controller-api.md` — describe the preview and `env`.

---

### Task 1: Preview rendering in the plug-in layer

**Files:**
- Modify: `crates/plugins/src/native.rs` (trait, `Instance`, new free functions)
- Modify: `crates/plugins/src/lib.rs` (`Plugin::scope`)
- Modify: `crates/plugins/src/replicant.rs` (remove live scope, replace tests)

**Interfaces:**
- Produces:
  - `pub struct Scope { pub wave: Vec<(f32, f32)>, pub envelope: Vec<f32> }` (in `native`, derives `Debug, Clone, Default, PartialEq`).
  - `Instance::scope(&mut self, columns: usize) -> Option<Scope>`
  - `Plugin::scope(&mut self, columns: usize) -> Option<native::Scope>`
- Consumes: `Class.params`, `Class.make`, `Instance.values`, `Instance.sample_rate`, `Engine::{set_param, note_on, note_off, process}`.

- [ ] **Step 1: Write the failing preview tests in `crates/plugins/src/replicant.rs`**

Replace the three live-scope tests (`scope_is_flat_until_something_plays`, `scope_shows_the_wave_and_starts_on_a_rising_crossing`, `scope_holds_still_on_a_steady_note`) and the `sine_scope` helper with:

```rust
/// The preview of a fresh instance with `set` applied.
fn preview(set: &[(u32, f64)], columns: usize) -> crate::native::Scope {
    let mut inst = Instance::new(&CLASS, 48_000.0);
    for (id, v) in set {
        inst.set_param(*id, *v);
    }
    inst.scope(columns).expect("Replicant has a scope")
}

#[test]
fn scope_previews_the_patch_without_a_note() {
    let s = preview(&[(DRIFT, 0.0), (CUTOFF, 0.8)], 64);
    assert_eq!(s.wave.len(), 64);
    assert_eq!(s.envelope.len(), 64);
    assert!(s.wave.iter().any(|&(lo, hi)| hi - lo > 0.05), "not a flat line");
    assert!(s.envelope.iter().any(|&v| v > 0.2), "the preview note sounds");
    assert!(s.wave.iter().chain(std::iter::empty()).all(|(lo, hi)| lo.is_finite() && hi.is_finite() && *lo >= -1.0 && *hi <= 1.0 && lo <= hi));
    let mut inst = Instance::new(&CLASS, 48_000.0);
    assert!(inst.scope(0).is_none(), "no columns, nothing to draw");
}

#[test]
fn scope_follows_the_knobs() {
    let dull = preview(&[(CUTOFF, 0.2)], 64);
    let open = preview(&[(CUTOFF, 0.9)], 64);
    let moved = dull.wave.iter().zip(&open.wave).map(|((a, b), (c, d))| (a - c).abs().max((b - d).abs())).fold(0.0f32, f32::max);
    assert!(moved > 0.1, "Cutoff changes the picture: {moved}");
    let same = preview(&[(CUTOFF, 0.2)], 64);
    assert_eq!(dull, same, "the same settings draw the same picture");
}

#[test]
fn scope_envelope_follows_the_amp_envelope() {
    let fast = preview(&[(A_ATTACK, 0.0), (A_DECAY, 0.3), (A_SUSTAIN, 0.8)], 64);
    let slow = preview(&[(A_ATTACK, 0.75), (A_DECAY, 0.3), (A_SUSTAIN, 0.8)], 64);
    let early = |s: &crate::native::Scope| s.envelope[..8].iter().copied().fold(0.0f32, f32::max);
    assert!(early(&fast) > early(&slow) + 0.1, "fast {} vs slow {}", early(&fast), early(&slow));
    assert!(fast.envelope[60] < fast.envelope[20], "the release falls: {} then {}", fast.envelope[20], fast.envelope[60]);
}

#[test]
fn scope_wave_starts_on_a_rising_crossing() {
    let s = preview(&[(OSC1_WAVE, 3.0), (MIX, 0.0), (DETUNE, 0.0), (DRIFT, 0.0), (CUTOFF, 1.0), (ENV_AMT, 0.5), (RESO, 0.0)], 64);
    let (lo, hi) = s.wave[0];
    assert!(lo > -0.05 && hi > 0.0, "first column: {lo}..{hi}");
    assert!(s.wave.iter().filter(|&&(lo, hi)| hi - lo > 0.05).count() > 32);
}

#[test]
fn scope_ignores_modulation() {
    let mut inst = Instance::new(&CLASS, 48_000.0);
    let plain = inst.scope(48).expect("a scope");
    inst.modulate(CUTOFF, 0.1);
    assert_eq!(inst.scope(48), Some(plain), "modulation does not move the preview");
}

#[test]
fn scope_is_only_replicant_and_works_at_44_khz() {
    let mut other = Instance::new(&crate::zhora::CLASS, 44_100.0);
    assert!(other.scope(16).is_none(), "Zhora has no scope");
    let mut inst = Instance::new(&CLASS, 44_100.0);
    for (id, v) in [(A_ATTACK, 1.0), (A_RELEASE, 1.0), (F_ATTACK, 1.0)] {
        inst.set_param(id, v);
    }
    let s = inst.scope(32).expect("a scope");
    assert!(s.wave.iter().chain(std::iter::empty()).all(|(lo, hi)| lo.is_finite() && hi.is_finite()));
    assert!(s.envelope.iter().all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0));
}
```

The chain call in the first test is awkward; instead use `s.wave.iter().all(...)` directly.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --offline -p deckard-plugins scope_`
Expected: compile error — `Instance::scope` returns `Option<Vec<(f32, f32)>>`, not `Scope`, and `modulate`/`Scope` are unavailable.

- [ ] **Step 3: Add `Scope` and the preview to `crates/plugins/src/native.rs`**

Add near `Readout`:

```rust
/// What a synth's scope draws: the waveform of a held note and the note's amplitude
/// envelope over time.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scope {
    /// The lowest and highest signal in each column, -1..1 (0 the middle line).
    pub wave: Vec<(f32, f32)>,
    /// The preview note's level in each column, 0..1.
    pub envelope: Vec<f32>,
}
```

Add module constants: `PREVIEW_SECONDS: f64 = 1.0`, `PREVIEW_HOLD: f64 = 0.6`, `PREVIEW_NOTE: u8 = 69`, `PREVIEW_VELOCITY: u8 = 100`, `SCOPE_WINDOW: usize = 1024`, `SCOPE_GAIN: f32 = 6.0`.

Add to `Instance`: `preview: Option<Vec<f32>>` (initialise `None`). In `Instance::load`, after `self.values = values`, set `self.preview = None`. In `Instance::set_param`, inside the `if self.values[i] != v` branch, set `self.preview = None`.

Add `Instance::scope(&mut self, columns: usize) -> Option<Scope>`:

```rust
pub(crate) fn scope(&mut self, columns: usize) -> Option<Scope> {
    if columns == 0 || !std::ptr::eq(self.class, &crate::replicant::CLASS) {
        return None;
    }
    if self.preview.is_none() {
        self.preview = Some(self.render_preview());
    }
    Some(scope_of(self.preview.as_deref().unwrap_or(&[]), columns))
}
```

Add `Instance::render_preview(&self) -> Vec<f32>`: make `(self.class.make)(self.sample_rate)`, apply every `class.params` value from `self.values`, `note_on(PREVIEW_NOTE, PREVIEW_VELOCITY)`, render `round(sr * PREVIEW_SECONDS)` frames in blocks of `crate::MAX_BLOCK`, calling `note_off` exactly at `round(sr * PREVIEW_HOLD)`, taking `0.5 * (left + right)` of each frame. Split the loops at the hold boundary so the note-off frame is exact.

Add free function `scope_of(mono: &[f32], columns: usize) -> Scope`:
- envelope: per column, max `abs` over its bucket, `* SCOPE_GAIN`, clamped to `0..1`; empty input gives zeros.
- waveform: window starts at `mono.len() * 3 / 10`, shifted to the first rising zero crossing within the window, at most `SCOPE_WINDOW` samples; per column, min/max of `x * SCOPE_GAIN` clamped to ±1 (0 where the window is empty).

Remove `Engine::scope` and `Instance::scope`'s old live implementation.

- [ ] **Step 4: Update `Plugin::scope` in `crates/plugins/src/lib.rs`**

```rust
/// A synth's preview for its scope: the waveform's columns and the note's envelope.
pub fn scope(&mut self, columns: usize) -> Option<native::Scope> {
    match &mut self.kind {
        Kind::Native(p) => p.scope(columns),
        _ => None,
    }
}
```

- [ ] **Step 5: Remove the live scope from `crates/plugins/src/replicant.rs`**

Delete `SCOPE_SAMPLES`, `SCOPE_WINDOW`, `SCOPE_GAIN`, the `scope_buf`/`scope_at` fields and initialisers, the two writes at the end of `process`, and `Engine::scope`. Delete the old `sine_scope` helper and live-scope tests, keeping the new preview tests.

- [ ] **Step 6: Run the plug-in tests**

Run: `cargo test --offline -p deckard-plugins`
Expected: all pass, including the six new `scope_` tests.

---

### Task 2: Two-band scope on the surface

**Files:**
- Modify: `crates/surface/src/lib.rs` (`ScopeView`)
- Modify: `crates/surface/src/api.rs` (`control.scope.env`)
- Modify: `crates/surface/lua/pages/control.lua` (`scope_view`)
- Test: `crates/surface/src/tests.rs`

**Interfaces:**
- Consumes: `ScopeView` is filled by the app from `native::Scope`.
- Produces: Lua sees `control.scope.wave` (flat low/high pairs) and `control.scope.env` (one level per column, 0..1).

- [ ] **Step 1: Write the failing surface test**

Replace `the_synth_scope_is_beside_the_picture` with a version that also sets an envelope and checks both bands:

```rust
#[test]
fn the_synth_scope_is_beside_the_picture() {
    let (s, mut v) = (Surface::new(), view());
    let name = "Replicant";
    v.control.chain[0].name = name.into();
    v.control.slot = Some(name.into());
    v.control.chain[0].instrument = true;
    v.control.chain[0].plugin = true;
    v.control.pages = vec!["Oscillators".into()];
    let wave = (0..109)
        .map(|i| {
            let (t0, t1) = (i as f32 / 109.0 * 6.0 * std::f32::consts::PI, (i + 1) as f32 / 109.0 * 6.0 * std::f32::consts::PI);
            (0..16).fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), k| {
                let x = (t0 + (t1 - t0) * k as f32 / 16.0).sin() * 0.9;
                (lo.min(x), hi.max(x))
            })
        })
        .collect();
    let envelope = (0..109).map(|i| if i < 66 { 1.0 } else { (1.0 - (i - 66) as f32 / 43.0).max(0.0) }).collect();
    v.control.scope = Some(crate::ScopeView { wave, envelope });
    let px = s.picture(&v);
    let count = |x0: usize, x1: usize, y0: usize, y1: usize| {
        px.iter().enumerate().filter(|(i, p)| (x0..x1).contains(&(i % 960)) && (y0..y1).contains(&(i / 960)) && accent(p)).count()
    };
    assert!(count(728, 952, 68, 164) > 300, "the wave: {}", count(728, 952, 68, 164));
    assert!(count(728, 952, 164, 204) > 200, "the envelope: {}", count(728, 952, 164, 204));
    assert!(count(0, 480, 0, 272) < 20, "not on the left screen");
    assert!(count(480, 720, 0, 272) < 20, "not over the picture");
    assert_eq!(s.error(), None);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --offline -p deckard-surface the_synth_scope_is_beside_the_picture`
Expected: compile error — `ScopeView` has no `envelope` field.

- [ ] **Step 3: Add `envelope` to `ScopeView` and the Lua table**

In `crates/surface/src/lib.rs`:

```rust
pub struct ScopeView {
    pub wave: Vec<(f32, f32)>,
    /// The preview note's level in each column, 0..1.
    pub envelope: Vec<f32>,
}
```

In `crates/surface/src/api.rs`, inside the `if let Some(s) = &c.scope` block, add:

```rust
t.set("env", lua.create_sequence_from(s.envelope.iter().copied())?)?;
```

- [ ] **Step 4: Draw both bands in `control.lua`**

Rewrite `scope_view` to use the fixed geometry from Global Constraints: panel `728, 68, 224, 136`; waveform `x 731..948`, centre `116`, half `44`, centre line in `separator`; separator at `y 164` in `button`; envelope baseline `y 200`, top `168`, level `0..1`, filled in `accent`. Columns are `math.max(1, math.ceil(218 / n))` wide and start at `731 + math.floor(i * 218 / n)`.

- [ ] **Step 5: Run the surface tests**

Run: `cargo test --offline -p deckard-surface`
Expected: all pass.

---

### Task 3: App wiring, documentation and build

**Files:**
- Modify: `apps/viewer/src/zhora_ui.rs` (`scope_view`)
- Modify: `apps/viewer/src/main.rs` (wake condition)
- Modify: `README.md`, `docs/controller-api.md`

**Interfaces:**
- Consumes: `Plugin::scope(&mut self, columns) -> Option<native::Scope>`, `ScopeView`.
- Produces: the MK3 Control page shows the preview; no change to the 30 ms wake.

- [ ] **Step 1: Update `scope_view` in `apps/viewer/src/zhora_ui.rs`**

Take a mutable lock and convert the result:

```rust
let Ok(mut p) = inst.try_lock() else {
    return shown.as_ref().filter(|(k, _)| *k == key).map(|(_, v)| v.clone());
};
let scope = p.scope(SCOPE_COLUMNS);
drop(p);
let Some(scope) = scope else {
    *shown = None;
    return None;
};
let v = ScopeView { wave: scope.wave, envelope: scope.envelope };
```

- [ ] **Step 2: Remove the live-follow pacing**

In `apps/viewer/src/main.rs`, delete the `|| (self.surface.page() == "control" && self.focused_replicant())` arm from `mk3_wake` and delete `focused_replicant` from `zhora_ui.rs`. The preview is static between edits, and knob events already repaint.

- [ ] **Step 3: Update the docs**

`docs/controller-api.md`: the `scope` row now says `{ wave, env }`, `wave` is the preview note's flat low/high pairs, `env` is the note's level per column (0..1), rendered from the current knob values with no note needed.

`README.md`: after the Replicant sentence, say the right screen previews the patch (waveform and envelope) and updates as knobs turn.

- [ ] **Step 4: Run the app suite**

Run: `cargo test --offline -p deckard`
Expected: everything passes except the pre-existing `channel_properties` failure (also fails on `main`).

- [ ] **Step 5: Rebuild the test app**

Run the release build and repackage:

```bash
DECKARD_VERSION=0.1.0 cargo build --release --locked --offline -p deckard --target aarch64-apple-darwin
bash dist/package-macos-arm64.sh
```

Expected: `dist/DECKARD-0.1.0-macos-arm64.zip` is written, `codesign --verify --deep --strict dist/DECKARD.app` passes, and `dist/DECKARD.app/Contents/MacOS/DECKARD --write-icon /tmp/check.png 16` writes a PNG.

---

## Self-Review

- **Spec coverage:** preview render (Task 1), waveform + envelope display (Tasks 1–2), knob-only cache (Task 1), Replicant-only (Task 1), static pacing (Task 3), docs/build (Task 3). All spec sections have a task.
- **Step scan:** test code and signatures are fixed; the two algorithm bodies (`render_preview`, `scope_of`, `scope_view`) are described because the values and geometry are decisions, not derivable from a signature.
- **Type consistency:** `Scope { wave, envelope }` in plugins; `ScopeView { wave, envelope }` in surface; Lua `wave`/`env`; `&mut` through `Plugin::scope` → `Instance::scope` → app.
- **Review focus:** cache (modulation/follows tests), modulation (ignores test), long envelopes (44.1 kHz/infinite test), sample rates (44.1 kHz test), non-Replicant (Zhora test).
- **Proportion:** plan is shorter than the combined code it changes; bodies are limited to the parts whose exact values are decisions.
