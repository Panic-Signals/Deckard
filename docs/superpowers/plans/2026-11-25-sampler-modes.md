# Sampler Preview's Three Modes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sampler Preview gets a mode (Original, Advanced, Layers) kept on the sound; what a sound offers is asked of the mode; in Layers a take can be edited before and after it is on a layer.

**Architecture:** `NewSampler` gains `mode: SamplerMode`. The app's `layers::can_layer` and every other "is this layered / may this have stems / slices" question become questions of the mode. A mode change is one document transaction that keeps one zone. The MK3 page learns the mode and a pending mode change through `SamplingView`.

**Tech Stack:** Rust workspace (`crates/model`, `crates/doc`, `apps/viewer` = package `deckard`, `crates/surface` = `deckard-surface`), Lua page `crates/surface/lua/pages/sampling.lua`.

**Spec:** `docs/superpowers/specs/2026-11-25-sampler-modes-design.md` (builds on `2026-11-24-take-layers-mk3-design.md`, already on this branch).

## Global Constraints

- Branch `take-layers-mk3`, in the main working tree (no worktree: a cold build is ~10 min).
- Targeted tests only. App tests: `cargo test --offline -p deckard --bin deckard -- <filters>`, always in the background (the build takes minutes). Full workspace suite once, at the end. Five tests fail on `main` already and are not this plan's: `lock_tests::the_midi_change_dialog_sets_lock`, `tests::channel_properties`, `tests::controller_plugin_browser_loads_modules`, `files::tests::old_files_that_failed_load`, `mk3_browse_lists_the_library_and_loads`.
- The classic Sampler (a sound without `new_sampler`) is not touched.
- `NewSampler::default().to_text()` stays `"v 1\nstretch rendered"` (pinned by a test): the default mode, Advanced, is not written.
- The control's name is `SAMPLER` (RECORD already has a `MODE`). Mode names as shown: `Original`, `Advanced`, `Layers`.
- Dialog text, exact: `Changing the mode loses all changes to this sound.` Buttons `OK`, `Cancel` (MK3: `OK`, `CANCEL`). Undo label: `Sampler Mode`.
- Public docs: no mention of reverse-engineering tools.
- `cargo fmt -p <crate>` before each commit.

## Review Focus

1. A project saved before this with layers opens in Layers; with one plain sample, in Advanced; saving and reopening either keeps the mode. (Task 1 tests.)
2. A mode change while a take is being recorded or armed does nothing; a pending confirmation is dropped when the focused sound changes. (Task 5 tests.)
3. Undo after a mode change restores zones, metas, morph and mode in one step; redo applies it again. (Task 1 and 5 tests.)
4. In Layers, an edit that changes the length (TRUNCATE, PASTE) keeps the layer and every other layer's settings. (Task 3 test.)
5. A take edited while on no layer, then applied, puts the edited file on the layer; the LAYER knob then shows that layer. (Task 6 test.)

---

### Task 1: The mode in the model and the document

**Files:**
- Modify: `crates/model/src/new_sampler.rs` (enum, field, text), `crates/model/src/lib.rs:1735` (`sound_own`: mode read from zones when unwritten)
- Modify: `crates/doc/src/new_sampler.rs` (`set_sampler_mode`)
- Test: both files' test modules

**Interfaces:**
- Produces: `deckard_model::new_sampler::SamplerMode { Original, #[default] Advanced, Layers }` with `SamplerMode::ALL: [SamplerMode; 3]` and `fn name(self) -> &'static str`; `NewSampler.mode: SamplerMode`.
- Produces: `NewSampler::from_text` unchanged in signature; new `pub fn mode_written(text: &str) -> bool`.
- Produces: `Document::set_sampler_mode(&mut self, g: usize, s: usize, mode: SamplerMode, keep: Option<usize>) -> Result<(), String>`: one undo step `Sampler Mode`. `keep: None`: only the setting changes (an empty sound, or Original to Advanced). `keep: Some(z)`: zone `z` becomes the only zone, play range the whole file (`set_zone_sample` needs the frame count: take it from the zone's `play_range`/sample length already in the model, else leave the range), keys and velocities 0..127, its meta `ZoneMeta { file, layer: (mode == Layers).then(Layer::default), ..Default::default() }` (None for Original); settings `NewSampler { mode, ..Default::default() }`; for Layers a One-shot envelope (`envelope_type == 0`) becomes ADSR (2).

- [ ] **Step 1: failing model tests** in `crates/model/src/new_sampler.rs`:

```rust
#[test]
fn the_mode_is_written_unless_advanced() {
    let n = |mode| NewSampler { mode, ..Default::default() };
    assert_eq!(n(SamplerMode::Advanced).to_text(), "v 1\nstretch rendered");
    for m in [SamplerMode::Original, SamplerMode::Layers] {
        let t = n(m).to_text();
        assert!(NewSampler::mode_written(&t), "{t}");
        assert_eq!(NewSampler::from_text(&t).mode, m);
    }
    assert!(!NewSampler::mode_written("v 1\nstretch rendered"));
}
```

and, beside the existing parse tests in `crates/model/src/lib.rs` (or `crates/doc` if the model has no document fixture: a doc test `an_unwritten_mode_is_read_from_the_zones` that builds a Sampler Preview with `set_new_sampler(.., Some(&Default::default()))`, gives zone 0 a meta with `layer: Some(Layer::default())`, and asserts `project().groups[0].sounds[0].new_sampler.unwrap().mode == SamplerMode::Layers`; and `Advanced` without the layer).

- [ ] **Step 2:** `cargo test --offline -p deckard-model new_sampler` and `-p deckard-doc new_sampler`. Expected: FAIL to compile (`SamplerMode` unknown).
- [ ] **Step 3:** implement the enum, the field, the text line `mode original` / `mode layers`, `mode_written`, and in `sound_own`: when the text has no mode and a zone of the sound has `meta.layer`, the mode is Layers. (Zones are parsed after `new_sampler` there: compute the mode once the `Sound` is built.)
- [ ] **Step 4: failing doc tests** in `crates/doc/src/new_sampler.rs`:

```rust
#[test]
fn a_mode_change_keeps_one_sample() {
    // three zones, zone 1 kept
    d.set_sampler_mode(0, 0, SamplerMode::Layers, Some(1)).unwrap();
    // one zone, its file zone 1's, meta.layer == Some(Layer::default()), mode Layers,
    // key_range (0, 127), envelope_type 2, undo_label Some("Sampler Mode")
    assert!(d.undo());
    // three zones again, mode Advanced
}

#[test]
fn original_to_advanced_keeps_everything() {
    // mode Original, two zones; set_sampler_mode(.., Advanced, None): two zones still
}
```

- [ ] **Step 5:** run, expect FAIL (no method); implement `set_sampler_mode` as one `transaction("Sampler Mode", ..)` using `set_zones`, `zone_meta_ops`/`set_zone_meta`, `set_new_sampler_as`; run, expect PASS.
- [ ] **Step 6:** `new_sampler_to_sampler` and `audio_to_new_sampler` unchanged (Audio converts to the default, Advanced). The app's "Convert to Sampler Preview" for a classic Sampler passes `NewSampler { mode: SamplerMode::Original, ..Default::default() }` (`apps/viewer/src/main.rs:13157`); an existing test there asserts the label only. Commit: `model: Sampler Preview has a mode`.

### Task 2: Layers is a mode, not what the zones hold

**Files:**
- Modify: `apps/viewer/src/layers.rs` (`can_layer`, `layers_in`, `layered`, `take_into`, `layer_files`, `make_layered`, `plain`), `apps/viewer/src/layout/sampling.rs:18` (`tab_shown`), `apps/viewer/src/main.rs` (`start_stems` ~8452, `FocusTake` ~9278, `drop_samples` ~11749, `place_take` ~11916), `apps/viewer/src/layers.rs:224` and `main.rs:8636` status texts
- Test: `apps/viewer/src/layers_tests.rs`, `apps/viewer/src/tests/layers_ui.rs`

**Interfaces:**
- Consumes: `NewSampler.mode`.
- Produces: `layers::mode_of(p: &Project, g: usize, s: usize) -> Option<SamplerMode>` (None: not a Sampler Preview); `can_layer(p, g, s)` is `mode_of == Some(Layers)`; `layers_in` returns every zone of a Layers sound (a zone without layer data as `Layer::default()`), up to `MAX_LAYERS`; `layered` is `can_layer && has a zone`.

- [ ] **Step 1: failing tests.** In `layers_tests.rs` change the helper `app()` to `set_new_sampler(0, 0, Some(&NewSampler { mode: SamplerMode::Layers, ..Default::default() }))`, and add:

```rust
/// What a sound offers is its mode's.
#[test]
fn layers_are_the_modes() {
    // an Advanced Sampler Preview with one zone: !can_layer, layers_in empty,
    // tab_shown(.., 4, ..) false, tab_shown(.., 2, true, false) true;
    // layer_files on it: status says layers are for a sound in Layers, zones unchanged.
    // a Layers sound, empty: can_layer, tab 4 shown, tab 2 not shown (with a zone).
    // a Layers sound's first take (take_into): names == ["take.wav"], and
    //   zones[0].meta.layer == Some(Layer::default()).
    // start_stems(false) on a Layers sound: no job onto lanes (app.stems is None or
    //   job.into_lanes == false).
}
```

Rewrite `a_lone_zone_is_the_first_layer`, `only_a_sound_without_slices_can_layer`, `a_lone_zone_is_not_layered_for_the_controller`, `later_takes_go_on_no_layer` and `tests/layers_ui.rs`'s tab assertions to the mode rule (a lone zone in Layers *is* layered; an Advanced one has no LAYERS).
- [ ] **Step 2:** run `-- layers surface_sampling record_knobs` in the background. Expected: the new and rewritten tests FAIL.
- [ ] **Step 3:** implement. `take_into`: in Layers with no zone, `take_to_layer(g, s, path, 0)`; with zones, the take waits; otherwise `load_take`. `drop_samples` and `place_take` branch on `can_layer`, not `layered`. `start_stems`: `into_lanes` only when the mode is Advanced. `FocusTake`: `kept` is "Layers and has a zone". `tab_shown`: `4 => !take && can_layer`, `2 => zone && !take && !can_layer`. Status texts: `layers are for a Sampler Preview in Layers`, stems text likewise for Advanced.
- [ ] **Step 4:** run the same filter plus `takes recordings sampling chop`. Expected: PASS (fix any test whose fixture needs `mode: Layers`).
- [ ] **Step 5:** commit `sampling: layers are Sampler Preview's Layers mode`.

### Task 3: An edit keeps the layer

**Files:**
- Modify: `apps/viewer/src/main.rs` (`store_edit` ~8876, `carry_lanes` ~8829), `apps/viewer/src/layers.rs` (`take_layer`), `crates/engine/src/edit.rs` (`edited_base`)
- Test: `apps/viewer/src/layers_tests.rs` (`an_edited_layer_stays_a_layer`, `an_edited_take_is_still_on_its_layer`: already written and RED in the working tree), `crates/engine/src/edit.rs`

**Interfaces:**
- Produces: `deckard_engine::edit::edited_base(file_name: &str) -> String`: the stem without a trailing ` - N` (the rule `edited_path` uses), so `take.wav`, `take - 2.wav` and `take - 13.wav` all give `take`.
- Produces: `take_layer` matches a zone whose `edited_base` equals the take's.

- [ ] **Step 1:** add `edited_base_drops_the_number` to `edit.rs` tests (`"take - 2.wav"`, `"take.wav"`, `"a - b.wav"` → `take`, `take`, `a - b`); run `cargo test --offline -p deckard-engine edited_base`: FAIL. Implement (share the rule with `edited_path`); PASS.
- [ ] **Step 2:** `store_edit`: when no stem lanes were carried and the zone's meta has `layer`, the zone keeps its meta on the new file: stems cleared, `stems_only` false, and the length rule `carry_lanes` applies (extract it: `fn resized(meta, old_len, new_len, rate, (shift, stretched))`). `take_layer` by `edited_base`.
- [ ] **Step 3:** run `-- an_edited_ layers` in the background. Expected: PASS, including the two that were RED.
- [ ] **Step 4:** commit `sampling: an edited layer stays a layer`.

### Task 4: Original does what a classic Sampler does

**Files:**
- Modify: the `new_sampler.is_some()` gates that offer Sampler Preview's own things: `apps/viewer/src/main.rs:7559, 8457, 8501 (follow_analysis: no Sync), 9430, 9597`, `apps/viewer/src/layout/sampling.rs:717, 749, 2028, 2243`, `apps/viewer/src/sampling_hw.rs:485, 552`, `apps/viewer/src/chop_app.rs:282`, `apps/viewer/src/chop_group.rs:32`
- Test: `apps/viewer/src/analysis.rs` tests, `apps/viewer/src/sampling_hw.rs` tests, `apps/viewer/src/chop_tests.rs`

**Interfaces:**
- Produces: `layers::own(p, g, s) -> bool`: a Sampler Preview not in Original (`mode_of` is Advanced or Layers). Each gate above asks `own` where it asked `new_sampler.is_some()`, except those that must hold for any Sampler Preview (engine routing, the slot's name, Convert).

- [ ] **Step 1: failing tests:** `an_original_zone_is_not_put_in_sync` (analysis of a loop on an Original sound leaves `meta.mode == PlayMode::Repitch`; on Advanced, Sync as today); `original_has_no_play_mode_knobs` (the ZONE knob pages of an Original sound equal a classic Sampler's names); `slices_to_pads_keep_the_mode` (an Original source gives Original sounds).
- [ ] **Step 2:** run `-- analysis sampling_hw chop`. Expected: the three FAIL.
- [ ] **Step 3:** add `own`, switch the gates, make `meta_from`'s caller force Repitch for Original. Run: PASS.
- [ ] **Step 4:** commit `sampling: Original mode is the classic Sampler's`.

### Task 5: SAMPLER in the app, with its confirmation

**Files:**
- Modify: `apps/viewer/src/layout/sampling.rs` (`SamplingAct`, the header), `apps/viewer/src/main.rs` (`sampling_act`, the dialog beside `stems_offer_dialog`, clearing on focus change), `apps/viewer/src/layers.rs` (`App::ask_sampler_mode`, `App::answer_sampler_mode`)
- Test: `apps/viewer/src/layers_tests.rs`, `apps/viewer/src/tests/layers_ui.rs`

**Interfaces:**
- Produces: `SamplingAct::Sampler(SamplerMode)`, `SamplingAct::SamplerAnswer(bool)`; `App.sampler_ask: Option<((usize, usize), SamplerMode)>`.
- `ask_sampler_mode(mode)`: nothing if not a Sampler Preview, the mode is already it, or `recorder.busy()`. No zone, or Original to Advanced: `set_sampler_mode(g, s, mode, None)` at once. Else `sampler_ask = Some(((g, s), mode))`.
- `answer_sampler_mode(ok)`: takes `sampler_ask`; on `ok` and the same focused sound, `set_sampler_mode(g, s, mode, Some(shown))` where `shown` is the Sampling view's zone (`sampling.sample_key`'s, else `self.zone`); then `self.zone = 0`, `sampling.selection = None`, tab to RECORD if the current one isn't offered. A take on no layer being shown (Task 6): its file becomes the one zone (`layer_files(0, ..)` after the change with `keep: None` on emptied zones; do this in Task 6).
- `sampler_ask` is cleared whenever `(group, sound)` differs from its sound (in the per-frame update next to `sync_sampling_page`).

- [ ] **Step 1: failing tests** in `layers_tests.rs`:

```rust
#[test]
fn a_mode_change_asks_first() {
    let (_dir, mut app) = three();
    app.zone = 1;
    app.sampling_act(SamplingAct::Sampler(SamplerMode::Advanced));
    assert_eq!(app.sampler_ask, Some(((0, 0), SamplerMode::Advanced)));
    assert_eq!(names(&app).len(), 3, "nothing yet");
    app.sampling_act(SamplingAct::SamplerAnswer(false));
    assert_eq!((app.sampler_ask, names(&app).len()), (None, 3));
    app.sampling_act(SamplingAct::Sampler(SamplerMode::Advanced));
    app.sampling_act(SamplingAct::SamplerAnswer(true));
    assert_eq!(names(&app), ["a3.wav"]);
    assert_eq!(labels(&app).last().map(String::as_str), Some("Sampler Mode"));
    // undo: three layers, Layers again
}

#[test]
fn an_empty_sound_changes_mode_at_once() { /* app(): Sampler(Advanced) → mode Advanced, sampler_ask None */ }

#[test]
fn a_pending_mode_is_dropped() { /* ask, then app.sound = 1 and one update tick (or the clearing fn): None; while recorder busy: Sampler(..) does nothing */ }
```

and in `tests/layers_ui.rs` `the_header_has_sampler` (the harness finds a `SAMPLER` control whose value is `Layers`; clicking `Advanced` shows a dialog with the exact text and `OK` / `Cancel`; `Cancel` leaves three layers).
- [ ] **Step 2:** run `-- layers`. Expected: FAIL to compile, then FAIL.
- [ ] **Step 3:** implement the acts, the header selector (three toggle fields under a `SAMPLER` label, shown only for a Sampler Preview, disabled while `recorder.busy()`), and the dialog (`egui::Modal` as the unsaved-changes one: the sentence strong, `OK`, `Cancel`, Escape cancels).
- [ ] **Step 4:** run `-- layers sampling`. Expected: PASS. Commit `viewer: SAMPLER chooses Sampler Preview's mode`.

### Task 6: Editing a take that is on no layer

**Files:**
- Modify: `apps/viewer/src/main.rs` (`focused_sample` ~8477, the per-frame sample load ~8258, `edit_range`, `store_edit`, `FocusTake`, `LayerAct::Focus` handling), `apps/viewer/src/layout/sampling.rs` (`tab_shown` call sites, `edited`), `apps/viewer/src/layers.rs`
- Test: `apps/viewer/src/layers_tests.rs`

**Interfaces:**
- Produces: `App.take_shown: bool`: in Layers, EDIT works on the focused take (set by `FocusTake`, cleared by `LayerAct::Focus`, by a change of focused sound, and when the take list no longer has it).
- `App::shown_take(&self) -> Option<(usize, PathBuf)>`: the focused take's index and file when `take_shown`, the sound is in Layers, and `take_layer` finds it on no lane. (A take that *is* on a lane isn't a special case: `FocusTake` sets `self.zone` to that lane.)
- With `shown_take` Some: the Sampling view loads that file under key `(g, s, usize::MAX, path)`; `tab_shown` is asked with `zone = true, take = true` (RECORD and EDIT only); `edit_range` is the selection or the whole file; `store_edit` writes `edited_path(&self.sample_dir("Recordings"), &take)`, replaces `recorder.view.takes[i]` with it, sets the status, and touches no document.
- `store_edit` on a layer's zone (Task 3) also replaces a take-list entry whose file name equals the zone's old file name with the edited file.
- `answer_sampler_mode(true)` with `shown_take` Some: the mode changes with `keep: None` after the zones are emptied in the same transaction, then the take is the one layer / zone.

- [ ] **Step 1: failing tests:**

```rust
/// Choose a take, edit it, give it a layer.
#[test]
fn a_take_is_edited_before_it_is_on_a_layer() {
    let (dir, mut app, take) = three_and_a_take();
    app.path = Some(dir.path().join("x.mxprj"));
    record(&mut app, RecordAct::FocusTake(0));
    assert_eq!(app.shown_take(), Some((0, take.clone())));
    // (load it as a frame would: helper `show_take`)
    app.sampling.selection = Some((0, 48_000));
    app.sampling_act(SamplingAct::Edit(EditOp::Truncate));
    let edited = app.recorder.view.takes[0].clone();
    assert_eq!(edited.file_name().unwrap(), "take - 2.wav");
    assert_eq!(names(&app).len(), 3, "the sound untouched");
    assert_eq!(labels(&app).len(), before, "no undo step");
    app.recorder.view.choose_layer((0, 0), 1);
    record(&mut app, RecordAct::ApplyLayer);
    assert_eq!(names(&app)[1], "take - 2.wav");
    assert_eq!(take_layer(&project(&app), 0, 0, &edited), Some(1));
    assert_eq!(app.shown_take(), None, "now its layer is what is edited");
}

#[test]
fn an_edit_on_a_layer_renames_its_take_in_the_list() { /* apply take to lane 1, show(1), Normalize: takes[0] file name "take - 2.wav"; undo: LAYER still 1 (take_layer == Some(1)) */ }
```

- [ ] **Step 2:** run `-- layers`. Expected: FAIL.
- [ ] **Step 3:** implement; `focused_sample` returns the take's `SampleRef { path, .. }` when `shown_take` is Some. Run `-- layers sampling takes recordings`: PASS.
- [ ] **Step 4:** commit `sampling: a take is edited before it has a layer`.

### Task 7: The MK3

**Files:**
- Modify: `crates/model/src/sampling_view.rs` (`SamplingCall`), `crates/surface/src/lib.rs` (`SamplingView`), `crates/surface/src/api.rs`, `crates/surface/lua/pages/sampling.lua`, `apps/viewer/src/surface_sampling.rs`, `apps/viewer/src/record_knobs.rs`, `apps/viewer/src/main.rs` (`sampling_call`)
- Test: `crates/surface/src/sampling_tests.rs`, `apps/viewer/src/surface_sampling_tests.rs`, `apps/viewer/src/record_knobs.rs`

**Interfaces:**
- Produces: `SamplingView.sampler: Option<u8>` (0 Original, 1 Advanced, 2 Layers; None: not a Sampler Preview) → Lua `v.sampler` as `"original" | "advanced" | "layers" | nil`; `SamplingView.sampler_ask: Option<u8>` → `v.sampler_ask` (the mode asked for, nil if none).
- Produces: `SamplingCall::SamplerMode(u8)`, `SamplingCall::SamplerAnswer(bool)`; Lua `deckard.sampling_call("sampler_mode", n)`, `("sampler_answer", bool)`.
- `record_knobs::knobs(.., layer: true)`: a last knob page `SAMPLER` with one knob `SAMPLER` (choices `Original`, `Advanced`, `Layers`; shows `sampler_ask`'s mode while one is pending) whose turn is `Sampler(mode)`; not given when `layer` is false or the sound isn't a Sampler Preview.
- Lua: while `v.sampler_ask`, both displays show the sentence (left: `Changing the mode loses`, right: `all changes to this sound.`), display buttons 7 `CANCEL` and 8 `OK` and nothing else acts; tab button 4 stacks `ZONE`/`LAYERS` when `v.sampler == "layers"` (not `#layers_of(v) > 0`); LAYER's APPLY button only in Layers.

- [ ] **Step 1: failing tests:** surface `the_mode_change_is_confirmed_on_the_displays` (with `sampler_ask = Some(1)`: button 8 text `OK`, 7 `CANCEL`, pressing 8 calls `SamplerAnswer(true)`, pressing 7 `SamplerAnswer(false)`, pressing 1 calls nothing), `button_4_follows_the_mode` (Layers and empty: both names; Advanced: `ZONE`); app `the_record_pages_have_sampler` (knob page `SAMPLER`, text `Layers`, turning to 1 leaves `sampler_ask == Some(((0,0), Advanced))`), and extend `the_scripts_route_has_no_layer_knob` with "nor SAMPLER".
- [ ] **Step 2:** `cargo test --offline -p deckard-surface sampling` (fast) and the app filter `-- surface_sampling record_knobs`. Expected: FAIL.
- [ ] **Step 3:** implement. Render both screens to PNG with the existing snapshot helper and look at them (the sentence fits; `OK`/`CANCEL` lit).
- [ ] **Step 4:** run both again: PASS. Commit `MK3: SAMPLER, and its confirmation on the displays`.

### Task 8: Docs

**Files:**
- Modify: `docs/controller-api.md` (`sampler`, `sampler_ask`, the two calls), `docs/sampling-controller.md` (§3.3 and the RECORD/LAYERS notes: modes, what each offers, the confirmation, editing takes), `DEVELOPMENT.md` if it describes Sampler Preview's layers rule

- [ ] **Step 1:** update; grep the three files for `lone`, `plain sample`, `first layer` and fix what the mode rule replaced.
- [ ] **Step 2:** `cargo test --offline -p deckard-surface` (the doc-checked API list, if any, passes). Commit `doc: Sampler Preview's modes`.

The manual's section is written when the pull request is opened (the `pull-request` skill).
