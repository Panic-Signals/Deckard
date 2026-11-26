# Takes onto Layers from the MK3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** RECORD's LAYER becomes the focused take's layer, chosen by turning and committed with APPLY (MK3 display button 8, and a button in the app); the MK3's tab button 4 reads `ZONE · LAYERS`.

**Architecture:** The pending choice lives in the app's `RecorderView`; `layers.rs` works out a take's layer and the choices, and applies one. The MK3 script only learns whether a choice is pending (`SamplingView::layer_pending`) and asks for it to be applied with a sampling call. `record_knobs` is shared with the NI-scripts route, which is told not to show the knob.

**Tech Stack:** Rust (apps/viewer `deckard`, crates/surface `deckard-surface`, crates/lua-host `deckard-lua-host`), Lua (crates/surface/lua/pages/sampling.lua).

**Spec:** `docs/superpowers/specs/2026-11-24-take-layers-mk3-design.md`

## Global Constraints

- Turning LAYER never edits the project; only APPLY does, in one undo step.
- Loop and Auto modes (`rec.mode > 1`), and sounds where `layers::can_layer` is false: behaviour unchanged.
- Run targeted tests only (named below); never the whole workspace (it takes ~45 min).
- Comments in the codebase's voice: short, what and why, no narration.
- Public docs: nothing about how Maschine's behaviour was learnt; no MK3 dev panel in screenshots.

## Review Focus

- The focused take's file is on two layers: LAYER shows the first; `None` removes only that one.
- The sound has `MAX_LAYERS` layers: no `New` among the choices; a pending `New` left over from before is dropped.
- The take's file was deleted from disk, then APPLY: a status line, no layer changed, the choice dropped.
- A recording finishes while a choice is pending for another take: the new take is focused, the choice dropped.
- A choice is pending, then the focused sound changes to one that can't have layers: no LAYER, APPLY dim, pressing it does nothing.

---

### Task 1: A take's layer, the choices, and APPLY (app logic)

**Files:**
- Modify: `apps/viewer/src/layers.rs` (`take_choices`, `take_lane`, `App::take_into`; new functions below)
- Modify: `apps/viewer/src/layout/sampling.rs` (`RecorderView`, `RecParam::Layer`, `RecordAct`)
- Modify: `apps/viewer/src/main.rs` (`sampling_act`: `RecordAct` arms near 9265; `finish_recording`)
- Test: `apps/viewer/src/layers_tests.rs`

**Interfaces — produces:**
- `layers::NO_LAYER: i64 = -1` (the `None` choice; lanes are `0..`, `MAX_LAYERS as i64` is `New`).
- `layers::take_layer(p: &Project, g: usize, s: usize, take: &Path) -> Option<usize>`: the first lane whose zone's sample has the take's file name.
- `layers::take_choices(p, g, s, rec: &RecorderView) -> Option<(Vec<String>, usize, Vec<i64>)>`: `None` unless `rec.mode <= 1`, `can_layer`, and a take is focused. Options `"None", "1"…"n", "New"` (`New` only while `n < MAX_LAYERS`); values `NO_LAYER, 0…n-1, MAX_LAYERS`; `at` is the pending choice if it is among the values, else the take's layer.
- `layers::layer_pending(p, g, s, rec) -> Option<i64>`: the pending choice when it is among the values and differs from the take's layer.
- `RecorderView::layer_choice: Option<i64>` replaces `layer: usize`; `RecorderView::set(RecParam::Layer, v)` stores `Some(v)`.
- `RecordAct::ApplyLayer`; `App::apply_take_layer(&mut self)`.

- [ ] **Step 1: Write the failing tests** in `layers_tests.rs` (use the file's `three()`, `tone()`, `names()`, `zones()`, `project()` helpers; replace the old next-take assertions in the test around line 410):

```rust
/// The first take is the sound's sample; the next ones wait in the list.
#[test]
fn later_takes_go_on_no_layer() // empty Sampler Preview sound; take_into twice:
    // after the first: zones == 1, names[0] == first file
    // after the second: zones == 1, names[0] still the first file

/// LAYER is the focused take's: None, the layers, New.
#[test]
fn the_layer_choices_are_the_focused_takes()
    // three(): no take focused -> take_choices == None
    // a take pushed and focused, on no layer -> (["None","1","2","3","New"], at 0)
    // set(RecParam::Layer, 1) -> at 2, layer_pending == Some(1), zones/names unchanged
    // set(RecParam::Layer, NO_LAYER) -> layer_pending == None (it is on none)
    // mode 2 -> None

/// APPLY puts the take there; None takes its layer away.
#[test]
fn apply_puts_the_take_on_the_layer_chosen()
    // set Layer 1, ApplyLayer -> names[1] == "take.wav", zones == 3, layer_choice == None,
    //   take_layer == Some(1), one more undo label than before
    // set Layer MAX_LAYERS, ApplyLayer -> zones == 4, names[3] == "take.wav"
    // take_layer is now Some(1) (the first): set NO_LAYER, ApplyLayer -> zones == 3,
    //   names[1] != "take.wav", the other one still there
    // ApplyLayer with nothing pending -> no new undo label

/// What drops the choice (Review Focus).
#[test]
fn a_pending_choice_is_dropped()
    // FocusTake(other), DeleteTake(focused), finish of a new take, focus moved to a
    // sound that can't layer (take_choices None, ApplyLayer changes nothing),
    // MAX_LAYERS layers with a stale Some(MAX_LAYERS) -> layer_pending None,
    // take file removed from disk then ApplyLayer -> zones unchanged, status set, choice None
```

- [ ] **Step 2: Run them, see them fail**

Run: `cargo test -p deckard --bin deckard layers_tests -- --test-threads 4`
Expected: compile errors (`NO_LAYER`, `layer_choice`, `ApplyLayer` unknown).

- [ ] **Step 3: Implement** the Interfaces above.
  - `take_into`: on a `can_layer` sound with any layer (`layers_in` non-empty) do nothing; otherwise `load_take` as now. Delete `take_lane`.
  - `apply_take_layer`: with `layer_pending` → `Some(v)`: `NO_LAYER` → `self.sampling_act(SamplingAct::Layer(lane, LayerAct::Remove))` for the take's lane; else `take_to_layer(g, s, &path, v as usize)` (it clamps `New` to the next lane). Clear `layer_choice` either way.
  - `sampling_act`: `RecordAct::ApplyLayer` → `apply_take_layer()`; `FocusTake`, `DeleteTake` and `finish_recording` (where it sets `focused`) clear `layer_choice`. `RecordAct::Set(RecParam::Layer, _)` must work while not `busy()` as the other params do.
  - Fix the other users of `rec.layer` / `view.layer` the compiler names.

- [ ] **Step 4: Run the tests, see them pass**

Run: `cargo test -p deckard --bin deckard layers_tests -- --test-threads 4`
Expected: all pass.

- [ ] **Step 5: Commit** — `sampling: LAYER is the focused take's, set with APPLY`

### Task 2: The app's RECORD footer

**Files:**
- Modify: `apps/viewer/src/layout/sampling.rs:2793-2800` (the LAYER field)
- Test: `apps/viewer/src/tests/layers_ui.rs`

**Interfaces — consumes:** `take_choices`, `layer_pending`, `RecordAct::ApplyLayer` (Task 1).

- [ ] **Step 1: Failing test** `the_record_footer_applies_a_takes_layer` in `layers_ui.rs` (the file's harness): with a take focused the footer has a LAYER field and a button labelled `APPLY`, disabled; after `set(RecParam::Layer, 1)` it is enabled; clicking it leaves the take on layer 2 (`names[1]`).
- [ ] **Step 2:** `cargo test -p deckard --bin deckard layers_ui` → fails (no `APPLY`).
- [ ] **Step 3:** After the LAYER field, a small `APPLY` button in the RECORDING box, enabled while `layer_pending` is `Some`, pushing `sact(SamplingAct::Record(RecordAct::ApplyLayer))`. The field's tip: `"The layer this take plays on. Turn or pick, then APPLY: a layer's sample is replaced, its settings kept; None takes it off"`.
- [ ] **Step 4:** `cargo test -p deckard --bin deckard layers_ui` → passes.
- [ ] **Step 5: Commit** — `viewer: RECORD's LAYER has an APPLY`

### Task 3: The MK3: APPLY on button 8, BANK gone

**Files:**
- Modify: `crates/lua-host/src/state.rs` (`SamplingCall`: add `ApplyTakeLayer`)
- Modify: `crates/surface/src/lib.rs` (`SamplingView`: add `pub layer_pending: bool`), `crates/surface/src/api.rs:667` (`"apply_take_layer" => C::ApplyTakeLayer`) and `:2271` (`t.set("layer_pending", v.layer_pending)`)
- Modify: `crates/surface/lua/pages/sampling.lua` (header comment line 12; `buttons()` lines 281-300; `take_bank` uses at 268, 520-523, 571-576, 729-733)
- Modify: `apps/viewer/src/surface_sampling.rs` (`sampling_surface_view`), `apps/viewer/src/main.rs:10179` (`sampling_call`: `C::ApplyTakeLayer => S::Record(RecordAct::ApplyLayer)`)
- Modify: `apps/viewer/src/record_knobs.rs` (`knobs` gains `layer: bool`), its two callers (`surface_sampling.rs:14` passes `true`, `main.rs:9985` passes `false`)
- Test: `crates/surface/src/tests.rs`, `apps/viewer/src/surface_sampling_tests.rs`, `apps/viewer/src/record_knobs.rs` (tests module)

- [ ] **Step 1: Failing tests**
  - `crates/surface/src/tests.rs`, `record_button_8_applies_a_takes_layer`: sampling page, `v.sampling.tab = 0`, `takes = 20`, `take = Some(18)`; `SCREEN_8` label is `APPLY`, pressing it with `layer_pending = false` gives no action, with `true` gives `[Action::SamplingCall(SamplingCall::ApplyTakeLayer)]`. With TAKES on (press `SCREEN_7`), pad 3 gives `Action::Take { index: 18, delete: false }` (the focused take's bank: takes 17-20).
  - `surface_sampling_tests.rs`, `the_record_page_says_a_layer_choice_is_pending`: a take focused on a layered sound, `set(RecParam::Layer, 1)` → `sampling_surface_view().layer_pending`; after `surface_sampling(Action::SamplingCall(ApplyTakeLayer))` it is false and the take is on layer 2; the RECORDING page has a `LAYER` knob with text `None` before.
  - `record_knobs.rs`, `the_scripts_route_has_no_layer_knob`: `knobs(.., false)` has no knob keyed `Layer`; `knobs(.., true)` has.
- [ ] **Step 2:** `cargo test -p deckard-surface record_button_8` and `cargo test -p deckard --bin deckard -- surface_sampling_tests record_knobs` → fail.
- [ ] **Step 3: Implement.** In the Lua: unshifted RECORD buttons are `START`, `DELETE`, `TAKES`, `APPLY` (`selected = v.layer_pending`, `disabled = not v.layer_pending`, `act = deckard.sampling_call("apply_take_layer")`); SHIFT keeps PREVIOUS / NEXT. Remove the `take_bank` variable: one local function `take_bank(v)` returns `((v.take or v.takes) - 1) // 16` (0 with no takes), used by the pads, their lights and `take_tiles`.
- [ ] **Step 4:** the same commands → pass. Also `cargo test -p deckard-surface sampling` (the page's other tests).
- [ ] **Step 5: Commit** — `MK3: RECORD's button 8 applies a take's layer; the pads follow the focused take`

### Task 4: `ZONE · LAYERS` on tab button 4

**Files:**
- Modify: `crates/surface/lua/pages/sampling.lua` (`draw_tabs` 608-627, its caller; header comment lines 4-5)
- Test: `crates/surface/src/tests.rs`

- [ ] **Step 1: Failing test** `tab_button_4_says_zone_and_layers`: a sample view with one layer (`layers.len() == 1`, `layered = false`), tab ZONE: the left screen's text contains both `ZONE` and `LAYERS`; `SCREEN_4` gives `Action::SamplingTab(4)`; from LAYERS it gives `Action::SamplingTab(3)`. With `layers` empty the screen has no `LAYERS` and `SCREEN_4` on ZONE gives nothing new. (Use whatever the file's other tests use to read a screen's labels; if none does, assert on the actions only and check the label by eye in Step 5.)
- [ ] **Step 2:** `cargo test -p deckard-surface tab_button_4` → fails.
- [ ] **Step 3:** `draw_tabs` takes whether the sound has layers (`#layers_of(v) > 0`). Then button 4's box is drawn selected on either tab, with two labels stacked in it: the tab showing in the row's usual size on top, the other small (size 9) right under it. (Spec, changed by Josh after the plan was written.) SHIFT's REMOVE / SPREAD on 1 and 2 stay. In `page.button`, on LAYERS (tab 5) button 4 goes to tab 4 (check it does today; add it if not).
- [ ] **Step 4:** `cargo test -p deckard-surface tab_button_4` and `cargo test -p deckard-surface sampling` → pass.
- [ ] **Step 5:** Look at it: run the app with the MK3 (the `run` skill), a Sampler Preview sound with a sample, SAMPLING: button 4 reads `ZONE · LAYERS`, toggles; RECORD: record two takes, turn LAYER to 2, APPLY lights, press it.
- [ ] **Step 6: Commit** — `MK3: the LAYERS tab is written on button 4`

### Task 5: Docs

**Files:**
- Modify: `docs/sampling-controller.md` (RECORD's buttons and LAYER; the tab row), `docs/sampling-layout.md` if it describes RECORD's LAYER field
- Modify: `apps/viewer/src/layout/sampling.rs` doc comments on `RecorderView::layer_choice` and `RecParam::Layer` (if not done in Task 1)

- [ ] **Step 1:** Rewrite the passages that say LAYER is where the next take goes, and that BANK changes the pads' bank; add `ZONE · LAYERS`.
- [ ] **Step 2: Commit** — `doc: a take's layer is applied; the MK3's LAYERS tab`

The manual (a separate repository) wants the same: the `pull-request` skill asks about it when the PR is opened.
