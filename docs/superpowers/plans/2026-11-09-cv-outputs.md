# CV Outputs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let any LFO or envelope drive one side of a Deckard output, marked CV only, as a control voltage.

**Architecture:** `CvRoute` follows `CcRoute` from model to document to engine. Each span, the engine overwrites the CV-only channels of its `ext` buffers with per-frame values, and the output callback routes those channels past the master. The engine's CV mask is 0 unless the audio output sets it, so offline export never writes CV.

**Tech Stack:** Rust 2024, egui, mlua surface pages. Run cargo through mise: `mise exec -- cargo test --offline -p <crate> <filter>`.

**Spec:** `docs/superpowers/specs/2026-11-09-cv-outs-and-zhora-design.md` (Part 1)

## Global Constraints

- Channels are numbered 0..32: Out 1 L = 0, Out 1 R = 1, ... Out 16 R = 31.
- Low and High are -1.0..=1.0, shown as -100%..+100%.
- No allocating or locking on the audio thread beyond the existing `try_lock` patterns.
- CV skips the master, faders, effects and limiters, and is never in exports or stems.
- Saved text is forgiving: lines that don't read are skipped, values are clamped, older readers ignore the field.
- `cargo fmt` at 140 columns. Do not run `cargo test --workspace` (45 minutes). Test the crates touched.

## Review Focus

- A CV-only channel with no route still carries silence, never leftover audio. Pinned in Task 3.
- The master on an output with one CV-only side: the other side still plays, and the CV side has no master audio. Pinned in Task 4.
- Pressing Play while an LFO is high: no drop to 0 between engines. Pinned in Task 4.
- A project saved on another machine whose routes name unmarked channels: silent, flagged, nothing else breaks. Pinned in Tasks 3 and 5.
- A very short span (1 frame, from a live event): values still advance and smoothing still ends. Pinned in Task 3.

---

### Task 1: The `CvRoute` model and document

**Files:**
- Modify: `crates/model/src/mod_matrix.rs`, `crates/model/src/lib.rs:64,331,1197`
- Create: `crates/doc/src/cv_routes.rs`
- Modify: `crates/doc/src/lib.rs:32`, `crates/doc/src/block.rs:76,279,376,544,554,634`, `docs/deckard-block.md`

**Interfaces:**
- Produces: `CvRoute { source: ModSource, channel: u8, low: f32, high: f32 }`, `CvRoute::new(source, channel) -> CvRoute` (low = high = 0), `CvRoute::value(&self, x: f32) -> f32`, `cv_to_text(&[CvRoute]) -> String`, `cv_from_text(&str) -> Vec<CvRoute>`, `CV_FIELD: &str = "deckard:cv_routes"`, `Project.cv_routes: Vec<CvRoute>`, `Document::set_cv_routes(Vec<CvRoute>) -> Result<(), String>`.

- [ ] **Step 1: Write failing tests in `mod_matrix.rs`**

```rust
#[test]
fn cv_value_maps_low_to_high_bipolar() {
    let r = CvRoute { low: -0.5, high: 0.5, ..CvRoute::new(ModSource::Lfo(1), 2) };
    assert!(close(r.value(0.0), -0.5));
    assert!(close(r.value(1.0), 0.5));
    assert!(close(r.value(0.5), 0.0));
}

#[test]
fn cv_routes_round_trip_and_bad_lines_are_skipped() {
    let a = CvRoute { low: -1.0, high: 0.25, ..CvRoute::new(ModSource::Lfo(16), 31) };
    assert_eq!(cv_from_text(&cv_to_text(std::slice::from_ref(&a))), vec![a]);
    let r = cv_from_text("nonsense\npressure 1 0 1\nlfo1 99 -3 3\nlfo2 x 0 1\n");
    assert_eq!(r.len(), 1);
    assert_eq!((r[0].channel, r[0].low, r[0].high), (31, -1.0, 1.0));
}
```

- [ ] **Step 2: Run to verify they fail** — `mise exec -- cargo test --offline -p deckard-model cv_` — FAIL, `CvRoute` not found.
- [ ] **Step 3: Implement** in `mod_matrix.rs`, in the shape of `CcRoute`. Line format: `source channel low high`. LFO sources only. `value` is `low + (high - low) * x.clamp(0.0, 1.0)`. Add `cv_routes` to `Project`, read in `Project::read` like `CC_FIELD`.
- [ ] **Step 4: Run to verify they pass** — same command — PASS.
- [ ] **Step 5: Write failing document tests** in `cv_routes.rs`, mirroring `cc_routes_are_set_undone_and_saved` and `maschine_does_not_see_the_field` from `crates/doc/src/cc_routes.rs`.
- [ ] **Step 6: Implement** `set_cv_routes` and `cv_route_ops` by mirroring `cc_routes.rs`, and add a `cv_routes` section everywhere `block.rs` handles `cc_routes`. Document the section in `docs/deckard-block.md`.
- [ ] **Step 7: Run** — `mise exec -- cargo test --offline -p deckard-doc cv_` — PASS.
- [ ] **Step 8: Commit** — `git commit -m "Add CV routes to the model and document"`

### Task 2: CV-only channels in settings

**Files:**
- Modify: `apps/viewer/src/settings.rs:42-49,199-212,254-257,423-436,646-660`
- Modify: `apps/viewer/src/main.rs:2613-2650` (the Routing editor)

**Interfaces:**
- Produces: `Routing.cv_only: Vec<bool>`, `Settings.output_cv_only: Vec<bool>` (loaded and stored per device with routing and aliases), `Settings::cv_mask(&self) -> u32`, `Settings::cv_channels(&self) -> Vec<usize>`, `Settings::may_mark_cv_only(&self, p: &Project, channel: usize) -> bool`, `Settings::audio_on_cv_only(&self, p: &Project) -> Vec<usize>` (the external outputs the project's master, cue or strips send audio to that have a CV-only side).

- [ ] **Step 1: Write failing tests in `settings.rs`**

```rust
#[test]
fn cv_only_is_kept_per_device_and_makes_a_mask() {
    let mut s = Settings::default();
    s.output_cv_only[2] = true;
    s.output_cv_only[3] = true;
    assert_eq!(s.cv_mask(), 0b1100);
    assert_eq!(s.cv_channels(), vec![2, 3]);
    // Round trip through save text as `output_routing_saved_per_device` does at line 1016.
}

#[test]
fn a_pair_with_a_cv_only_side_is_not_offered_to_audio() {
    let mut s = Settings::default();
    s.output_routing[2] = Some(2);
    s.output_routing[3] = Some(3);
    assert!(s.output_pair_on(1));
    s.output_cv_only[2] = true;
    assert!(!s.output_pair_on(1));
}

#[test]
fn the_masters_and_cues_channels_cannot_be_marked() {
    let s = Settings::default();
    let p = Project { master_output: AudioDest::Ext(0), cue_output: AudioDest::Ext(4), ..Default::default() };
    assert!(!s.may_mark_cv_only(&p, 1));
    assert!(!s.may_mark_cv_only(&p, 8));
    assert!(s.may_mark_cv_only(&p, 2));
}
```

- [ ] **Step 2: Run** — `mise exec -- cargo test --offline -p deckard -- cv_only the_masters` — FAIL.
- [ ] **Step 3: Implement.** Saved key `output_cv`, in the existing `<device>\t<channel>:<value>` form. `output_pair_on` returns false if either side is CV only, which removes the pair from every audio picker and from `next_free_output`.
- [ ] **Step 4: Add the "CV only" checkbox** per output channel to the egui Routing editor at `main.rs:2625`, disabled with a tooltip when `may_mark_cv_only` is false.
- [ ] **Step 5: Run** — same command — PASS.
- [ ] **Step 6: Commit** — `git commit -m "Let output channels be marked CV only"`

### Task 3: The engine writes CV

**Files:**
- Modify: `crates/engine/src/lfo_rt.rs`, `crates/engine/src/lib.rs:2123,2868,3026,4835` and after `self.cue.finish` in `process_span`

**Interfaces:**
- Consumes: `CvRoute`, `Project.cv_routes`.
- Produces: `pub(crate) struct CvRt { route: CvRoute, last: f32 }`, `Engine::set_cv_channels(&mut self, mask: u32)`, `Engine::lfo_value_after(&self, pad: u8, frames: u64) -> Option<f32>` (`lfo_value` becomes `lfo_value_after(pad, 0)`), `Engine::write_cv(&mut self, ext: &mut [Vec<f32>], at: usize, frames: usize)`, `Engine::take_cv(&mut self, old: &Engine)`, `Engine::cv_conflicts(project: &Project, mask: u32) -> Vec<usize>` (indices of routes that are inactive).

Spans already split at every event, so an envelope's state is constant within a span. That is why per-frame evaluation with an offset is exact.

- [ ] **Step 1: Write failing tests in `lib.rs`'s test module.** Use a project with one free-running 1 Hz saw LFO on pad 1, `sr = 1000.0`, `ext` as 16 `vec![0.0; 2 * frames]`.

```rust
#[test] fn cv_is_written_per_frame_not_per_span()    // two adjacent frames in one span differ by 2 * (1.0 / sr) for Low -1, High 1
#[test] fn cv_overwrites_audio_on_its_channel()      // ext[1][0] pre-filled with 0.9; route on channel 2; ext[1][0] != 0.9
#[test] fn a_cv_only_channel_without_a_route_is_silent() // ext[1][1] pre-filled with 0.9, mask 0b1100, assert == 0.0
#[test] fn a_muted_source_sits_at_low()              // Low -0.5, muted LFO, all frames == -0.5
#[test] fn an_unmarked_channel_gets_no_cv_and_is_flagged() // mask 0, ext untouched, cv_conflicts == [0]
#[test] fn the_first_route_owns_a_shared_channel()   // two routes on channel 2, second in cv_conflicts, first's values written
#[test] fn the_lfo_restart_is_smoothed()             // jump across lfo_clock_start: each step <= full_jump / (0.005 * sr), arrives within 5 ms
#[test] fn smoothing_ends_with_one_frame_spans()     // process 1-frame spans past 5 ms: output equals the raw target
#[test] fn export_writes_no_cv()                     // engine with default mask: ext unchanged
```

- [ ] **Step 2: Run** — `mise exec -- cargo test --offline -p deckard-engine -- cv_ muted_source unmarked_channel first_route the_lfo_restart smoothing_ends` — FAIL.
- [ ] **Step 3: Implement.** `lfo_value_after` adds `frames` to `self.time`, `frames / samples_per_tick` to the ticks, and `frames / sr` to the seconds. `write_cv`: for each set bit in the mask, zero that side of `ext`, then write its first owning route's `value(lfo_value_after(pad, n).unwrap_or(0.0))`, with `None` meaning Low. Add `cv_smooth_left: u32`, set to `(0.005 * sr) as u32` in `lfo_clock_start` and `take_cv`. While it is above 0 each frame moves `last` by `(target - last) / left`. Call `write_cv` immediately after `self.cue.finish(...)`, so a click routed to `ext` can't overwrite it. Add `adopt_cvs` beside `adopt_ccs`, keeping `last` per channel.
- [ ] **Step 4: Run** — same command — PASS. Then `mise exec -- cargo test --offline -p deckard-engine lfo` — PASS, proving `lfo_value` is unchanged.
- [ ] **Step 5: Commit** — `git commit -m "Write CV routes into the engine's outputs"`

### Task 4: The output callback

**Files:**
- Modify: `crates/engine/src/output.rs:253-256,297-313,510,589,629-639,724-732,1187`
- Modify: `apps/viewer/src/main.rs:739,2644-2647`

**Interfaces:**
- Consumes: `Engine::set_cv_channels`, `Engine::take_cv`, `Settings::cv_mask`.
- Produces: `Output::set_routing(&self, routing: &[Option<usize>], cv_mask: u32)` (changed signature), `pub fn effective_routing(routing: &[Option<usize>], cv_mask: u32) -> Vec<Option<usize>>`, `pub fn channel_uses_master(channel: usize, master_bus: Option<usize>, cv_mask: u32) -> bool`.

- [ ] **Step 1: Write failing tests in `output.rs`'s test module** (run with `--features output`).

```rust
#[test]
fn audio_mapped_to_a_cv_jack_is_dropped() {
    let mut r = vec![None; 32];
    r[2] = Some(4);
    r[6] = Some(4);
    let e = effective_routing(&r, 1 << 2);
    assert_eq!((e[2], e[6]), (Some(4), None));
}

#[test]
fn a_cv_side_of_the_masters_output_does_not_play_the_master() {
    assert!(!channel_uses_master(0, Some(0), 0b01));
    assert!(channel_uses_master(1, Some(0), 0b01));
    assert!(channel_uses_master(1, Some(0), 0));
}

#[test]
fn play_carries_cv_across_the_new_engine() // an engine with `last` = 0.7 replaced by Command::Play: new.write_cv's first frame stays near 0.7
```

- [ ] **Step 2: Run** — `mise exec -- cargo test --offline -p deckard-engine --features output -- audio_mapped a_cv_side play_carries` — FAIL.
- [ ] **Step 3: Implement.** Store the mask in an `AtomicU32`. Call `engine.set_cv_channels` beside `set_connected_outputs` (line 589). `set_routing` stores `effective_routing`. Line 631 becomes a per-channel choice using `channel_uses_master`. In `Command::Play`, call `new.take_cv(old)` beside `new.take_unroutes(old)`. Update both callers in `main.rs`, and re-send when marks change.
- [ ] **Step 4: Run** — same command, then `mise exec -- cargo check --offline -p deckard` — PASS.
- [ ] **Step 5: Commit** — `git commit -m "Route CV-only channels past the master"`

### Task 5: CV routes on the mod matrix and the computer screen

**Files:**
- Modify: `apps/viewer/src/mod_slots.rs`, `crates/surface/src/lib.rs:291-294`, `crates/surface/src/api.rs`, `crates/surface/lua/pages/mod_matrix.lua`, `apps/viewer/src/layout/lfo_panel.rs:217,369`, `apps/viewer/src/layout.rs:1254`, `apps/viewer/src/layout/mixer.rs` (the flag on a strip), `apps/viewer/src/export.rs:109` (no stem)
- Test: `apps/viewer/src/surface_lfo_tests.rs`, `crates/surface/src/tests.rs`

**Interfaces:**
- Consumes: `Document::set_cv_routes`, `Settings::cv_channels`, `Engine::cv_conflicts`, `Settings::output_name`.
- Produces: `ModSlot::Cv(usize)`, `Key::Cv(ModSource, u8)`, `At::Cv(usize)`, `ModDraft.cv: Option<CvDraft>`, `CvDraft { channel: Option<u8> }`, `ModSlotView.cv: bool`.

Past MIDI, the Group row offers CV. A CV slot's rows are SOURCE, GROUP ("CV"), OUT, (blank), LOW, HIGH. The route is made when OUT is chosen. Low and High step by 0.05. A new CV route starts at Low 0%, High 50%, because `CvRoute::new` at Depth 0 would look broken.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn a_cv_slot_shows_its_rows()                    // rows == ["LFO 1", "CV", "Out 2 L", "", "0%", "50%"], cv == true
#[test] fn a_cv_slots_out_row_offers_only_cv_channels()  // marks 2 and 3; stepping the OUT row visits only those
#[test] fn turning_source_to_none_removes_a_cv_route()   // as CC routes do
#[test] fn low_steps_below_zero_to_minus_100()           // clamps at -1.0
#[test] fn with_no_cv_channels_the_group_row_skips_cv()  // an empty mask never offers CV
#[test] fn an_inactive_route_says_so()                   // cv_conflicts route: OUT row reads "Out 2 L !"
#[test] fn audio_on_a_cv_only_output_is_flagged()        // strip on Ext(1), channel 2 marked: strip's flag says why it is silent
#[test] fn a_silent_master_says_why()                    // master on a marked output: the status line names the output
#[test] fn a_cv_only_output_has_no_stem()                // stems() omits an extra-out on a pair with a CV-only side
```

- [ ] **Step 2: Run** — `mise exec -- cargo test --offline -p deckard -- cv_slot cv_route with_no_cv low_steps inactive_route cv_only_output a_silent_master` — FAIL.
- [ ] **Step 3: Implement** by following each `ModSlot::Cc`, `Key::Cc` and `At::Cc` arm in `mod_slots.rs`. In `mod_matrix.lua`, add `CV_ROWS` and `CV_TURNS` beside `MIDI_ROWS`, chosen by `slot.cv`. Show `audio_on_cv_only` as a flag on the affected strips and in the status line for the master. `stems` at `export.rs:109` takes the settings' `cv_mask` and omits those outputs.
- [ ] **Step 4: Add `cv_section`** to `lfo_panel.rs` beside `cc_section`, with Source, Out, Low and High, and a conflict note.
- [ ] **Step 5: Run** — same command, then `mise exec -- cargo test --offline -p deckard-surface mod_matrix` — PASS.
- [ ] **Step 6: Run the app** with an interface or the system output, mark a channel, make a route, and confirm with a scope or meter that the channel moves. Report what was and wasn't checked.
- [ ] **Step 7: Commit** — `git commit -m "Edit CV routes on the mod matrix"`
