# A Plug-in's Extra Outputs in the Mixer: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A plug-in's extra output pairs (VST3, Audio Unit or native, on any instrument or chain slot) get their own mixer strip once sound comes out of them. Each strip has its own level, pan, mute, solo, meter and destination. Strips are saved in the DECKARD block and exported as separate stems.

**Architecture:**
- **Model:** a new `deckard_model::extra_out` module holds `ExtraOut` records in `Project.extra_outs`, keyed by owner slot, bus index and plug-in id.
- **Engine:** two jobs.
  - It scans buses without a strip for sound and reports hits through `Telemetry` without allocating.
  - It mixes buses that have a strip through their own gain ramp and route. Routes are resolved by the existing `dest_route`, plus a connected-outputs mask for the "plays through its parent" fallback.
- **Viewer:** turns hits into strips (`Act::AddExtraOut`, which picks the next free output), draws them next to their owner in the mixer, and adds them to export as stems.

**Tech Stack:** Rust workspace: `crates/plugins`, `crates/model`, `crates/doc`, `crates/engine`, `apps/viewer` (egui).

**Spec:** `docs/superpowers/specs/2026-11-05-plugin-extra-outputs-design.md`

## Global Constraints

- The audio thread never allocates or locks. Detection is a cheap per-block peak check, and only on buses that have no strip yet.
- "Sound came out" means a peak above **−90 dBFS** (`3.1623e-5` linear) on either side in one block.
- A project with no extra outputs in use saves **byte-for-byte as before**: the DECKARD block isn't written just for this.
- Plug-ins with one output pair behave exactly as now.
- Testing: run only the tests of the crate you changed (`cargo test -p <crate> <filter>`). Never run the whole workspace per task (it takes about 45 minutes). Run the full suite once at the end of the branch.
- Public docs never mention reverse-engineering tools.
- Commit on branch `design/plugin-extra-outputs`. Nothing is pushed until the user says so.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **The plug-in is removed, replaced or moved while it has strips.** Its strips disappear from the mixer and the engine, with no stale index or crash. (Task 2 `prune`, Task 5 test.)
2. **The plug-in fails to load**, for example a macOS AU in a project opened on Windows. Its strips are kept in the project, not pruned, and are silent. (Task 2 test.)
3. **A two-output interface, or a saved `Ext(n)` that isn't connected.** The strip plays through its parent, and the picker says so. (Task 5 and Task 7 tests.)
4. **Two buses are heard in the same block** (for example Kontakt). They get distinct free outputs in mixer order, and the same bus never gets two strips. (Task 6 test.)
5. **A bypassed slot.** Its extra buses are silent and aren't detected. (Task 4 test.)

---

### Task 1: Plug-ins report how many output pairs they have and their names

**Files:**
- Modify: `crates/plugins/src/lib.rs`: keep the `BusInfo.name` that the bus setup (around lines 475-549) discards. Add the `Plugin` methods.
- Modify: `crates/plugins/src/au.rs`: read `kAudioUnitProperty_ElementName` (id 30, output scope) for each output element in `negotiate` (around line 784).
- Modify: `crates/plugins/src/native.rs`: generalise the native `Engine` trait so native plug-ins can have several buses.
- Test: `crates/plugins/src/native.rs` tests.

**Interfaces:**
- Produces:
  - `Plugin::output_count(&self) -> usize`: the number of stereo-capable output buses, including bus 0. It is 0 for a plug-in with no output.
  - `Plugin::output_name(&self, bus: usize) -> String`: the plug-in's name for the bus. Without one, it is `"Out {bus+1}"`.
  - The native `Engine` trait gains `fn outputs(&self) -> usize { 1 }` and `fn output(&self, bus: usize, frames: usize) -> Option<(&[f32], &[f32])> { None }`, both defaulted.
  - `Instance::output_bus(bus)` uses `output` for buses 1 and up.
  - `Plugin::output_bus` (it exists already) is unchanged.

- [ ] **Step 1: Write the failing test** `native_plugin_reports_extra_buses`. Give a test native engine `outputs() == 3` and named buses. Assert `output_count() == 3`, `output_name(0) == "Main"`, `output_name(2) == "Aux 2"`, an unnamed bus gives `"Out 2"`, and `output_bus(2, n)` returns what `output` gave. A native engine that keeps the defaults reports `output_count() == 1`.
- [ ] **Step 2:** `cargo test -p deckard-plugins native_plugin_reports_extra_buses`. Expected: FAIL (no such method).
- [ ] **Step 3:** Implement as above. VST3: count only buses with 1 or 2 channels, the same buses `output_bus` can return. AU: the same rule, using the element buffers au.rs already allocates.
- [ ] **Step 4:** Run the test again. Expected: PASS. Then run `cargo test -p deckard-plugins`, which must pass.
- [ ] **Step 5: Commit** with the message "Let plug-ins say how many output pairs they have and what they're called."

### Task 2: The model of an extra output

**Files:**
- Create: `crates/model/src/extra_out.rs`
- Modify: `crates/model/src/lib.rs`: `pub mod extra_out;`, and add `pub extra_outs: Vec<extra_out::ExtraOut>` to `Project`, defaulting to empty.

**Interfaces:**
- Produces:
  ```rust
  pub enum Owner { Sound(usize, usize), Chain(usize, usize, usize), Group(usize, usize), Master(usize) } // mirrors deckard_plugins::Slot
  pub struct ExtraOut { pub owner: Owner, pub bus: usize, pub plugin: String /* PluginRef.id */, pub name: String,
                        pub level: f64 /* linear, 1.0 = 0 dB */, pub pan: f64 /* -1..1 */, pub muted: bool, pub dest: AudioDest }
  pub fn to_text(v: &[ExtraOut]) -> String; pub fn from_text(s: &str) -> Vec<ExtraOut>;
  pub fn plugin_at(p: &Project, o: Owner) -> Option<&PluginRef>;
  pub fn prune(p: &mut Project); // drops strips whose owner slot now holds a different plug-in id or none at all
  ```
  - Text format: one strip per line, in the style of `lfo::to_text` and `from_text`. Lines that don't parse are skipped, and unknown `key=` words are ignored.
  - `prune` keeps a strip when the slot still names the same plug-in id, even if that plug-in failed to load. Load failure isn't visible in the model, and that's the intent.

- [ ] **Step 1: Write the failing tests:**
  - `extra_outs_round_trip_as_text`: covers every `Owner` and every `AudioDest` variant.
  - `prune_drops_strips_of_a_replaced_or_removed_plugin`
  - `prune_keeps_strips_whose_plugin_id_is_unchanged`
  - `bad_lines_are_skipped`
- [ ] **Step 2:** `cargo test -p deckard-model extra_out`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests again. Expected: PASS.
- [ ] **Step 5: Commit** with the message "Model a plug-in's extra output as a strip with its own level and destination."

### Task 3: Save the strips in the DECKARD block (version 9)

**Files:**
- Modify: `crates/doc/src/block.rs`. Follow the steps for adding a section, using the Pad Touch and lfos text sections as the pattern:
  - Set `VERSION` to 9.
  - Add `Block.extra_outs: String`, written after the existing three texts.
  - Decode it only under `if version >= 9`.
  - Fill it in `with_block`.
  - Include it in the "anything to write" check.
  - Apply it in `read_block` by setting `project.extra_outs`. That is the same path the lfos text takes.
- Modify: `docs/deckard-block.md`. Change "Payload, version 8" to 9 and add the new row to the layout table.
- Modify: any doc operation that removes or replaces a plug-in, or moves a slot, so that it calls `extra_out::prune` afterwards. Find these by grepping `crates/doc/src` for the operations that set `SlotInfo.plugin` or `Sound.plugin`.

**Interfaces:**
- Consumes: `extra_out::{to_text, from_text, prune}` from Task 2.

- [ ] **Step 1: Write the failing tests:**
  - `extra_outs_survive_save_and_reopen`
  - `a_project_without_extra_outs_saves_unchanged`: the bytes are equal to a save from before the change. Use the existing no-block fixture.
  - `version_8_blocks_still_read`: copy the pattern of the existing `version_N_blocks_still_read` test.
  - `replacing_a_plugin_drops_its_extra_outs`
- [ ] **Step 2:** `cargo test -p deckard-doc block`. Expected: the new tests FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p deckard-doc`. Expected: PASS.
- [ ] **Step 5: Commit** with the message "Keep extra-output strips in the DECKARD block (version 9)."

### Task 4: The engine notices sound on extra buses that have no strip

**Files:**
- Modify: `crates/engine/src/lib.rs`:
  - `FxChain` (around line 778) and `FxChain::run` (around line 874), for chain slots.
  - `render_sound` (around line 4653), for the instrument slot. Read the extra buses **before** the plug-in guard is dropped.
  - `Telemetry` (around line 1218) and `publish` (around line 4139).
- Test: engine tests. Reuse the `Split` harness from commit `39b3b32` on branch `feat/master-four-outs` (`git show 39b3b32 -- crates/engine/src/lib.rs`): `SPLIT`, `split_unit`, `split_engine`, `split_block`, `about`. Port it so its second bus comes from the native `Engine::output` (Task 1), and add a third bus.

**Interfaces:**
- Consumes:
  - `Plugin::output_count` and `output_bus` (Task 1).
  - `Project.extra_outs` (Task 2): it decides which buses already have a strip.
- Produces:
  - `pub struct Heard { pub owner: deckard_model::extra_out::Owner, pub bus: usize }`
  - The field `Telemetry.heard: Vec<Heard>`. The engine fills a fixed-capacity array of **32** entries and `publish` copies it into this field. It holds hits since the last publish, each (owner, bus) once, in mixer order (group, sound, slot, bus).
  - Each unit precomputes `scan: u64`, a bitmask of the buses to check, when it is built. It is not worked out per block.

- [ ] **Step 1: Write the failing tests:**
  - `a_silent_extra_bus_is_not_heard`
  - `sound_on_an_extra_bus_is_heard_once`
  - `a_bus_with_a_strip_is_not_scanned`
  - `a_bypassed_slot_is_not_heard`: `unit.mix == 0`.
  - `an_instruments_extra_bus_is_heard`
  - `a_one_pair_plugin_reports_nothing`
- [ ] **Step 2:** `cargo test -p deckard-engine heard`. Expected: FAIL.
- [ ] **Step 3:** Implement. The peak threshold is `3.1623e-5`. Nothing on the audio thread may allocate. The existing "no allocating in `publish`" rule applies: `heard` reuses its buffer.
- [ ] **Step 4:** Run the tests again, then `cargo test -p deckard-engine lfo split`. Expected: PASS.
- [ ] **Step 5: Commit** with the message "Notice when a plug-in's extra output starts making sound."

### Task 5: The engine mixes each strip with its own gain, route and meter

**Files:**
- Modify: `crates/engine/src/lib.rs`:
  - Per unit, keep `extras: Vec<ExtraRt>` for the buses that have a strip. Allocate it when the engine is built, never per block. The struct is `ExtraRt { index: usize /* into project.extra_outs */, bus: usize, buf: Vec<f32> /* JOB_FRAMES*2 */, route: Route, ramp: GainRamp }`.
  - Sum the bus × `unit.mix` into `buf` during `FxChain::run` and `render_sound`.
  - Mix into the destination in the same three-way match used by `render_pass` (around lines 4387-4417) for sounds, and by `run_buses` (around lines 3921-3947) for group- and master-owned strips.
  - Resolve the route with `dest_route(p, x.dest, parent, 0)` times `channel_gain(x.level, x.pan)`, and `SILENT` when muted. For a group- or master-owned strip, `parent` is the group or `None` (the master).
  - Keep the strip's meter in `meters_extra: Vec<(f32, f32)>`, indexed like `project.extra_outs`, and publish it as `Telemetry.extra_meters`.
  - Rebuild-on-edit: an edit that changes only `extra_outs` levels or routes updates the routes the way level edits do today. It doesn't rebuild the chain.
- Modify: `crates/engine/src/output.rs`. `set_routing` also tells the engine which Deckard outputs are connected, as `Engine::set_connected_outputs(mask: u16)`, sent the same way other engine settings reach the callback. `ext_route` (around line 584) uses the mask. A strip whose `Ext(n)` isn't connected is routed as `AudioDest::Parent`.
- **Owner muted, strip audible:** a muted sound or group still renders its plug-in when any of its strips is unmuted. Only its main pair is silenced. Find where muting skips rendering (grep `muted` around lines 1691 and 2037) and make an exception for owners whose strips are audible.
- Modify: `docs/engine-design.md`. Add a short section on extra outputs.

**Interfaces:**
- Consumes: Tasks 1, 2 and 4.
- Produces:
  - `Telemetry.extra_meters: Vec<(f32, f32)>`
  - `Engine::set_connected_outputs(&mut self, mask: u16)`
  - `Output::take_meters` also returns the extra meters, or a new `Output::take_extra_meters() -> Vec<(f32, f32)>`. Implementer's choice, but say which in the commit.

- [ ] **Step 1: Write the failing tests:**
  - `a_strip_on_ext_reaches_that_output`: Out 3 carries the bus, and the master doesn't.
  - `a_strips_level_and_pan_are_its_own`: moving the owner's or the master's fader doesn't change it.
  - `a_strip_into_a_group_goes_through_that_group`
  - `a_strip_into_a_sound_goes_through_that_sound`
  - `an_unconnected_ext_falls_back_to_the_parent`
  - `a_muted_owner_keeps_its_plugin_running_for_its_strip`
  - `a_muted_strip_is_silent`
  - `removing_the_plugin_drops_its_strip_without_panic`
  - `a_one_pair_plugin_sounds_unchanged`: compare against a render with no `extra_outs`.
- [ ] **Step 2:** `cargo test -p deckard-engine strip`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests again, plus `cargo test -p deckard-engine route mute`. Expected: PASS.
- [ ] **Step 5: Commit** with the message "Mix a plug-in's extra outputs through their own strips."

### Task 6: Strips appear when heard, on the next free output

**Files:**
- Create: `apps/viewer/src/extra_outs.rs`
- Modify: `apps/viewer/src/main.rs`. Where telemetry is read each frame (next to `update_meters`, around line 5219), turn `heard` into acts. Add `Act::AddExtraOut(ExtraOut)` to the act enum and its doc op.

**Interfaces:**
- Consumes:
  - `Telemetry.heard` (Task 4).
  - `Settings::output_pair_on(pair)` (`apps/viewer/src/settings.rs:234`).
  - `Plugin::output_name` (Task 1), through the host instance for that slot.
- Produces:
  - `pub fn next_free_output(p: &Project, connected: impl Fn(usize) -> bool, taken: &[usize]) -> AudioDest`. "Free" means connected and used by none of: `master_output`, `cue_output`, any sound's or group's `Ext` dest or aux dest, any `extra_outs` dest, and `taken`. `taken` holds outputs handed out earlier in the same batch. It returns `Ext(n)` for the lowest free `n`, or `AudioDest::Parent` when none is free.
  - `pub fn strips_for(p: &Project, heard: &[Heard], name: impl Fn(Owner, usize) -> String, connected: impl Fn(usize) -> bool) -> Vec<ExtraOut>`. It returns one strip per newly heard (owner, bus), with level 1.0, pan 0 and `plugin` set from the slot. It skips any (owner, bus) that already has a strip.
- A new strip is one undoable edit. All strips from a single frame go in one edit.

- [ ] **Step 1: Write the failing tests:**
  - `next_free_skips_the_master_and_used_outputs`
  - `next_free_falls_back_to_parent_when_none_free`: a two-output interface.
  - `two_buses_heard_together_get_distinct_outputs_in_mixer_order`
  - `a_bus_heard_twice_makes_one_strip`
- [ ] **Step 2:** `cargo test -p deckard extra_outs`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests again. Expected: PASS.
- [ ] **Step 5: Commit** with the message "Add a strip when a plug-in's extra output first makes sound."

### Task 7: Strips in the mixer, next to their owner

**Files:**
- Modify: `apps/viewer/src/layout/mixer.rs`:
  - Add `Channel::Extra(usize)`, an index into `project.extra_outs`. It must work with `app.peaks`, `app.headroom` and `.target()`.
  - In `mixer` (around lines 256-300), insert strips right after their owner:
    - In the groups view, a group's strips (owner `Group(g, _)`) go after that group.
    - In a group's sounds view, a sound's strips (`Sound(g, s)` and `Chain(g, s, _)`) go after that sound.
    - Master-owned strips go immediately before the master column, in both views.
  - Label: the top line is the plug-in name (small, accent colour) and the second line is `"Out {bus+1}"`, followed by the plug-in's bus name only when that name isn't generic (see Task 7b's `strip_label`; until 7b lands, `"Out {bus+1}"` alone is fine). Never the word "Aux": that means the send effects in Deckard. The strip background is tinted (blue, dashed border, as in the mockup).
  - Level, pan, mute and solo emit new acts on the strip: `Act::SetExtraOut(usize, ExtraOutEdit)` with `ExtraOutEdit::{Level(f64), Pan(f64), Mute(bool), Dest(AudioDest)}`.
  - The meter comes from the extra meters (Task 5), with the same decay as `update_meters`.
- Modify: `apps/viewer/src/layout/channel.rs`. The output picker reuses `destinations` and `ext_outputs`.
  - For a master-owned strip, offer only `Ext` outputs and None, since anything inside Deckard would feed back into the master.
  - For other strips, exclude destinations that lead back to the owner. Reuse the loop check `destinations` already applies to a channel's own output.
  - When the chosen `Ext(n)` isn't connected, the picker text reads `"Out N (not connected) → <parent name>"`.
- Modify: `apps/viewer/src/main.rs`, `App::solo` (around line 7356). Soloing `Channel::Extra(i)` mutes everything else, including the owner's main pair, while leaving the owner's plug-in running (Task 5). Soloing an owner keeps its strips audible.

**Interfaces:**
- Consumes: Tasks 2, 5 and 6.

- [ ] **Step 1: Write the failing tests** using the egui harness, like the `lfo_panel` tests:
  - `a_strip_is_drawn_after_its_group`
  - `a_sound_strip_is_drawn_after_its_sound`
  - `master_owned_strips_sit_before_the_master`
  - `the_strip_shows_plugin_and_output_names`
  - `dragging_a_strip_fader_sets_only_its_level`
  - `a_master_strip_offers_only_outputs`
  - `an_unconnected_output_shows_the_fallback`

  Add a plain unit test, `soloing_a_strip_mutes_the_rest_but_not_its_plugin`, in `main.rs` tests.
- [ ] **Step 2:** `cargo test -p deckard mixer solo`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests again. Expected: PASS.
- [ ] **Step 5: Commit** with the message "Show a plug-in's extra outputs as strips next to their channel."

### Task 7b: Stereo, mono or split, and the strip's label

Spec: "Stereo, mono or split" and the naming bullets under "Each extra output is its own mixer strip".

**Files:**
- Modify: `crates/model/src/extra_out.rs`:
  - `pub enum Channels { Stereo, Mono, Left, Right }`, default `Stereo`; new field `ExtraOut.channels: Channels`.
  - Text: a `ch=` word with `stereo|mono|left|right`. It is written only when not `Stereo`, and a missing `ch=` parses as `Stereo`, so existing text is unchanged. No block version bump.
  - `pub fn split(v: &mut Vec<ExtraOut>, i: usize)`: replaces a `Stereo` or `Mono` strip at `i` with two strips, `Left` then `Right`, in that place, both copying its other fields.
  - `pub fn join(v: &mut Vec<ExtraOut>, i: usize)`: given a `Left` or `Right` strip, removes its partner (same owner, bus, plugin) and turns the `Left` one (or `i` if the partner is missing) into `Stereo`, keeping the L half's settings.
  - `pub fn strip_label(bus: usize, plugin_bus_name: &str, ch: Channels) -> String`: `"Out {bus+1}"`, then `" {name}"` only when the name is not generic, then `" L"` / `" R"` for split halves. Generic means: after lower-casing and removing digits, spaces and `./-_:#()`, what's left is empty or one of `aux`, `out`, `output`, `outs`, `bus`, `st`, `stereo`, `main`, `ch`, `channel`.
- Modify: the engine (`crates/engine/src/lib.rs`, `ExtraRt`, `refresh_strips`, `mix_strip`, and the route list it's built from): a strip is now keyed by (owner, bus, channels), so two halves of one bus are two `ExtraRt`s. `Mono` feeds `(L+R)/2` to both sides before pan; `Left`/`Right` feed that side alone to both sides before pan. Pan on a mono-fed strip uses the same pan law as a channel's mono pan. Still no allocation on the audio thread. Detection (Task 4) is unchanged: one heard bus still makes one `Stereo` strip.
- Modify: `apps/viewer/src/extra_outs.rs`: `strips_for` skips a bus that already has any strip, whatever its channels. Names use `strip_label`.
- Modify: `apps/viewer/src/layout/mixer.rs`: a small menu on the strip (a button under the label): `Stereo`, `Mono`, `Split` on a stereo or mono strip; `Join` on a split half. Each emits `Act::SetExtraOut(i, ExtraOutEdit::Channels(..))`, `Act::SplitExtraOut(i)` or `Act::JoinExtraOut(i)`, each one undoable edit through `Document::set_extra_outs`. Split halves stay next to each other in the mixer. Fix indices held elsewhere (selection, meters) after a split or join changes the list length.

**Interfaces:**
- Consumes: Tasks 2, 5, 6 and 7.
- Produces: `Channels`, `ExtraOut.channels`, `split`, `join`, `strip_label` for Task 8.

- [ ] **Step 1: Write the failing tests:**
  - model: `channels_round_trip_and_default_to_stereo` (old text with no `ch=` parses as `Stereo`; `to_text` of all-stereo strips equals today's output), `split_makes_left_then_right_with_the_same_settings`, `join_keeps_the_left_settings_and_drops_the_partner`, `strip_label_hides_generic_names` (`"Aux 1"`, `"Out 3"`, `"Output 3/4"`, `"St. 3/4"`, `""` → `"Out 3"` for bus 2; `"Kick"` → `"Out 3 Kick"`; `Left` → `"Out 3 L"`).
  - engine: `a_mono_strip_sums_both_sides`, `split_halves_carry_one_side_each_with_their_own_level`.
  - viewer: `splitting_a_strip_shows_two_strips_next_to_each_other`, `join_is_offered_only_on_a_split_half`.
- [ ] **Step 2:** `cargo test -p deckard-model extra_out`, `cargo test -p deckard-engine strip`, `cargo test -p deckard mixer`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests again. Expected: PASS.
- [ ] **Step 5: Commit** with the message "Let an extra-output strip be stereo, mono, or split into two mono strips."

### Task 8: Export each strip on a hardware output as its own stem

**Files:**
- Modify: `crates/engine/src/export.rs`:
  - Add `Stem::Extra(usize)`.
  - In `stem_project`, keep the owner's plug-in running, mute everything else and the owner's main pair, and set the strip's `dest` to `Ext(15)`. That gives it a private output.
  - Add a render path that collects `ext[15]` through `process_outputs` instead of `out`.
  - In `file_stem`, name it `"{name} - {owner channel name} - {plugin} {strip_label} - {bpm} BPM"` (Task 7b's label, e.g. "Portal Out 3 L"). A mono or split-half strip renders the same mono signal on both channels of its stem.
- Modify: `apps/viewer/src/export.rs`. `stems()` and `output_list` (around lines 95 and 408) add a `Stem::Extra(i)` for every strip whose dest is `Ext`. A strip routed into Deckard gets no stem.

**Interfaces:**
- Consumes: Tasks 2 and 5.

- [ ] **Step 1: Write the failing tests:**
  - `a_strip_on_an_output_exports_as_its_own_stem`: the stem holds the bus and the master file doesn't.
  - `a_strip_routed_into_a_group_gets_no_stem`
  - `the_extra_stem_is_named_after_its_strip`
- [ ] **Step 2:** `cargo test -p deckard-engine export` and `cargo test -p deckard export`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests again. Expected: PASS.
- [ ] **Step 5: Commit** with the message "Export a plug-in's extra outputs as their own stems."

### Task 8b: Mono and split strips have no pan

Spec: "Stereo, mono or split" (2026-11-06: no pan on a mono strip or a split half).

**Files:**
- Modify: `crates/engine/src/lib.rs` (`strip_route`/`strip_gain`): for `Channels::Mono | Left | Right` the pan is ignored and treated as centre (`pan_gains(0.0)`, unity on both sides). Stereo is unchanged.
- Modify: `apps/viewer/src/layout/mixer.rs`: on a mono strip or split half, the pan bar isn't drawn or interactive. Leave the space empty so the level module lines up.
- Saved pan values stay in the file untouched; switching back to Stereo or joining uses the stored pan again.

- [ ] **Step 1: Write the failing tests:** engine `a_mono_strip_ignores_its_pan` and `a_split_strip_half_ignores_its_pan` (pan -1.0 plays equally on both sides); viewer `mono_and_split_strips_have_no_pan`.
- [ ] **Step 2:** `cargo test -p deckard-engine --lib strip`, `cargo test -p deckard --bin deckard mixer`. Expected: FAIL.
- [ ] **Step 3:** Implement. **Step 4:** Tests pass.
- [ ] **Step 5: Commit** "Mono and split extra-output strips have no pan."

### Task 8c: Group tabs in the mixer

User request (2026-11-06), Maschine's behaviour: the Mixer has a tab per group. Today the mixer can't get back to group level: at sound level the fixed right column (the focused group) ignores double-clicks, so `Act::MixerSounds(false)` is unreachable.

**Files:**
- Modify: `apps/viewer/src/layout/mixer.rs` (and `docs/mixer-layout.md` §7):
  - A tab row across the top of the strip area, always shown in the Mixer: one tab per group (its ID letter and name, in its colour), like the group tabs Maschine shows over its mixer. At sound level the focused group's tab is selected; at group level none is.
  - **Click a tab:** focus that group and show its sounds (`Act::FocusGroup(g)` + `Act::MixerSounds(true)`).
  - **Double-click a tab:** show the groups mixer (`Act::MixerSounds(false)`).
  - **Double-click a group strip's header in the groups mixer:** show that group's sounds. This exists; keep it.
  - **Double-click the fixed group column at sound level:** back to the groups mixer.
  - Too many groups for the width: the row scrolls with the strips' horizontal scroll, or clips with the focused tab kept in view (builder's call).
  - The strips move down by the tab row's height; the sidebar toggles and master column line up with them.

- [ ] **Step 1: Write the failing tests** (egui harness): `clicking_a_group_tab_shows_its_sounds`, `double_clicking_a_tab_shows_the_groups`, `double_clicking_the_fixed_group_column_shows_the_groups`, `the_focused_groups_tab_is_selected_at_sound_level`.
- [ ] **Step 2:** `cargo test -p deckard --bin deckard mixer`. Expected: FAIL.
- [ ] **Step 3:** Implement. **Step 4:** Tests pass.
- [ ] **Step 5: Commit** "Add group tabs to the Mixer, and a way back to the groups."

### Task 8d: Every plug-in panel shows its parameters

User request (2026-11-06): in the chain area (the Mixer shows a whole chain side by side), a hosted VST/AU panel that isn't the focused slot shows a "Show parameters" button (`apps/viewer/src/layout/strip.rs` ~647) instead of its knobs, because `App.plugin_view` (`apps/viewer/src/main.rs` ~877, filled ~7475-7560) holds one live view: the focused slot's.

**Files:**
- Modify: `apps/viewer/src/main.rs`: replace the single `plugin_view: Option<PluginView>` with live views for every hosted plug-in panel drawn this frame (e.g. a map keyed by `(Chain, slot)`). Each refreshes its shown page's values every 100 ms with the same non-blocking read (`try_lock`, VST3 controller values without holding the instance), and keeps its own page (today `plugin_page` is one number for all). Views of slots not drawn for a while are dropped, so the cost follows what's on screen. Keep `hold_sent` behaviour per view.
- Modify: `apps/viewer/src/layout/strip.rs`: every hosted plug-in panel draws its pages and knobs; the "Show parameters" button goes. A plug-in that failed to load still shows its error.
- Keep the focused slot's behaviour (MK3 hardware pages, `Act::PluginPage`) working; it reads the focused slot's view.

- [ ] **Step 1: Write the failing tests:** `every_plugin_panel_shows_its_knobs` (two native test plug-ins in one chain, the unfocused one draws knobs, no "Show parameters"), `each_plugin_panel_keeps_its_own_page`, `views_of_slots_no_longer_drawn_are_dropped`.
- [ ] **Step 2:** `cargo test -p deckard --bin deckard plugin`. Expected: FAIL.
- [ ] **Step 3:** Implement. **Step 4:** Tests pass.
- [ ] **Step 5: Commit** "Show every plug-in panel's parameters, not just the focused one's."

### Task 9: Branch end

- [ ] Run `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- [ ] Run the full suite once: `cargo test --workspace --locked`.
- [ ] By hand on macOS:
  - Kontakt with a multi-out instrument: only the pairs that play get strips.
  - Portal (AU) on a group.
  - A master plug-in with an extra pair on a 2-output interface: it plays through the master, and the picker says so.
  - Save, reopen, and export stems.
  - Split a drum plug-in's pair into L and R, set different levels, then Join it back.
- [ ] Report to the user. Don't push.
