# Modulation matrix base (expression part 2b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pressure, Slide and Pitch of a sound's notes drive any parameter of its chain
through routes with Low/High/Curve/Range, the first sources of DECKARD's modulation matrix.

**Architecture:** A route is DECKARD-only model data (`deckard_model::mod_matrix`) kept as
a `deckard:mod_routes` string field on the sound (undo, copy, paste) and in the DECKARD
block. The engine builds runtime routes off the audio thread, tracks every sounding note's
expression per sound, works out each source per render span and writes targets: internal
effects through the existing modulation (`modded`), the Sampler through
`Settings::set_field`, drum synths through their parameter list, plug-ins through a new
audio-thread `Plugin::modulate`. The app's Assignment area gets a MOD tab.

**Tech Stack:** Rust (workspace crates model, doc, engine, plugins; apps/viewer egui).

**Spec:** `docs/superpowers/specs/2026-10-24-expression-out-design.md` (section 2b).

## Global Constraints

- Clean room: no NI code or assets; Maschine ignores the DECKARD block and `deckard:` fields are stripped before Maschine's archive is written.
- Audio thread: no allocating, no waiting on locks (plug-ins: `try_lock`).
- Cargo through mise: `mise exec -- cargo …`; targeted tests while building; lint and the full suite at the end.
- Tests don't write library folders or the settings file.

## Review Focus

1. A route to a slot that's removed, moved or replaced: no stray modulation, no panic (Task 2 rekey tests, Task 4 guard test).
2. Removing a route returns the parameter to the knob's value, plug-ins included (Task 4 `removing_a_route_restores_the_plugin`).
3. Live and recorded notes on the same sound at once: pressure takes the hardest, slide the latest (Task 3).
4. A sound with routes whose notes go to a MIDI port: sources still work (Task 3 test on a port route).
5. Old blocks (v1, v2) still read; a v3 block with routes round-trips (Task 2).

---

### Task 1: The model: routes

**Files:** Create `crates/model/src/mod_matrix.rs`; modify `crates/model/src/lib.rs` (Sound gains `mod_routes`), `crates/model/src/pad_touch.rs` (gains `shape`), `apps/viewer/src/mk3_io.rs` (`pressure_shape` calls it).

**Interfaces — Produces:**
```rust
pub enum ModSource { Pressure, Slide, Pitch }            // ALL, name(), from_name()
pub enum ModParam { Field(String), Plugin(u32) }
pub struct ModRoute { source: ModSource, slot: usize, module: String, param: ModParam,
                      low: f32, high: f32, curve: u8, range: u8 }
impl ModRoute { fn new(source, slot, module, param) -> ModRoute; fn travel(&self, x: Option<f32>) -> f32 }
pub const FIELD: &str = "deckard:mod_routes";
pub fn to_text(routes: &[ModRoute]) -> String; pub fn from_text(s: &str) -> Vec<ModRoute>
pub fn pad_touch::shape(x: f64, curve: f64) -> f64
```
`module` is the slot's module class, or the plug-in's id for a PluginHost: a guard. `travel(x)`: the source value (`None`: no note counted) to knob travel 0..1: Pressure/Slide `x.unwrap_or(0)` shaped by curve, `low + (high-low)*shaped`; Pitch `x` in semitones, `t = (x/range).clamp(-1,1)` (None → 0), shaped symmetrically (`sign·shape(|t|)`), `low + (high-low)*(t+1)/2`. Defaults: low 0, high 1, curve 50, range 2. Text: one route per line, `source slot param low high curve range module` with param `f:<field>` or `p:<id>`, module last (may hold spaces); bad lines skipped.

- [ ] Tests (`mod_matrix.rs`): `routes_round_trip_as_text` (two routes, one Field one Plugin, module with spaces), `bad_lines_are_skipped`, `pressure_travels_from_low_to_high` (0→low, 1→high, 0.5→midway, low 0.9 high 0.1 inverts), `no_note_rests_at_low`, `pitch_rests_at_centre_and_spans_the_range` (None→0.5 at 0/1, +2 st→high, −4 st clamps to low), `the_curve_shapes_the_source` (curve 100: 0.25 maps above 0.25).
- [ ] Sound reads `mod_routes` from `FIELD` (Str) in the model build (beside `pad_touch`); test in model or doc.
- [ ] Run `mise exec -- cargo test -p deckard-model mod_matrix` — PASS.

### Task 2: The document: editing, slots, saving

**Files:** Create `crates/doc/src/mod_routes.rs`; modify `crates/doc/src/chain.rs` (add/remove/move rekey), `crates/doc/src/block.rs` (v3), `crates/doc/src/lib.rs` (mod).

**Interfaces — Produces:** `Document::set_mod_routes(g, s, Vec<ModRoute>) -> Result<(), String>` (one undo step "Modulation", merged per `op_target`), `Document::mod_route_ops(g,s,&[ModRoute]) -> Result<Vec<Op>,String>`, `Document::mod_routes_rekey(c: Chain, f: impl Fn(usize)->Option<usize>) -> Vec<Op>` (None drops the route).

- [ ] Tests: `routes_are_set_and_undone`, `adding_a_slot_before_moves_its_routes`, `removing_a_slot_removes_its_routes`, `moving_a_slot_takes_its_routes`, `routes_round_trip_through_a_save` (block v3), `version_2_blocks_still_read` (existing v1/v2 tests stay green), `no_routes_no_field` (empty list removes the field).
- [ ] Run `mise exec -- cargo test -p deckard-doc --lib mod_routes block chain` — PASS.

### Task 3: The engine: sources

**Files:** modify `crates/engine/src/lib.rs` (new `mod mods_rt;` or a section), `crates/engine/src/mod_rt.rs` (create).

**Interfaces — Produces:** `ExprVoice { group, sound, pitch, started: u64, tps: f64, expr: Option<ExprId>, live: [Option<f32>; 3], order: u64 }` in `Engine.touch: Vec<ExprVoice>` (capacity 256, made with the engine); `Engine::sources(g, s) -> [Option<f32>; 3]` (Dim::ALL order; pressure max, slide/pitch latest). Voices start in `trigger_with` (every path: port, plug-in, sampler) for sounds with routes, end on their note-off or release (`note_offs`, `end_midi_note`, `note_release`), live values set from `Live::Expression` via the pads fan-out.

- [ ] Tests: `pressure_source_is_the_hardest_pressed_note`, `slide_and_pitch_follow_the_latest_note`, `recorded_curves_drive_the_sources`, `a_note_without_pressure_doesnt_count`, `sources_end_with_their_notes`, `sources_work_for_notes_sent_to_a_port`.
- [ ] Run `mise exec -- cargo test -p deckard-engine --lib sources` — PASS.

### Task 4: The engine: targets

**Files:** modify `crates/engine/src/lib.rs`, `crates/engine/src/sampler.rs` (`Settings::set_field`), `crates/engine/src/modscale.rs` (`Scale::at`), `crates/plugins/src/lib.rs` (+ native) (`Plugin::modulate`).

**Interfaces — Produces:** `Scale::at(self, t: f64) -> f64` (inverse of `travel`); `Settings::set_field(&mut self, k: &str, v: f64, sr: f64) -> bool` (same conversion as `Settings::new` for that field); `SamplerRt::modulate(&mut self, field: &str, v: f64, sr: f64)` (Sampler fields or the synth's parameter list; a synth is re-set once per span); `Plugin::modulate(&mut self, id: u32, v: f64)` (audio thread: VST3 straight to `params_in`, the controller untouched; native: its parameter); `Engine.routes: Vec<RouteRt>` built in `Engine::new`/`new_after` from each sound's `mod_routes` whose guard matches its slot; `Engine.restores: Vec<(Shared, u32, f64)>` (plug-in parameters of routes gone since `prev`, at the controller's value, sent on the first span).

Applied each span from `apply_mods` (routes after lanes, so a route wins), only when the value changed (plug-ins) or always into `modded` (effects).

- [ ] Tests: `scale_at_inverts_travel` (modscale), `set_field_matches_new` (each continuous Sampler field), `pressure_moves_an_effects_parameter`, `pressure_moves_the_samplers_cutoff`, `pressure_moves_a_drum_synths_parameter`, `pressure_moves_a_native_plugins_parameter` (Plaits via `Plugin::native`), `a_route_to_another_module_does_nothing`, `removing_a_route_restores_the_plugin`.
- [ ] Run `mise exec -- cargo test -p deckard-engine --lib route` and `-p deckard-plugins` — PASS.

### Task 5: The software: MOD tab and Routes list

**Files:** modify `apps/viewer/src/remote.rs` (`AssignTab::Mod`), `apps/viewer/src/layout/assign.rs` (MOD fields, route editor popup with `CloseOnClickOutside`, Routes list), `apps/viewer/src/layout.rs` (`Act::ModRoute`), `apps/viewer/src/layout/channel.rs` (`knob_route_target(app, edit) -> Option<(usize, String, ModParam)>`).

**Interfaces — Produces:** `Act::ModRoute { g, s, index: Option<usize>, route: Option<ModRoute> }` (None index: add; None route: remove) applied by `doc.set_mod_routes`.

- [ ] Tests (egui_kittest, like `the_assignment_area`): `mod_tab_assigns_pressure_to_a_knob`, `knob_route_targets` (Sampler field, internal effect field, plug-in id; a group chain knob gives none), `the_routes_list_removes_a_route`.
- [ ] Run `mise exec -- cargo test -p deckard --bin deckard -- mod_` — PASS.

### Task 6: Finish

- [ ] PARITY row 10 (2b done; MK3 assigning next), FEATURES #1 (base built: expression sources), `docs/deckard-block.md` (v3 routes).
- [ ] `mise run lint` PASS; `mise exec -- cargo test --workspace --no-fail-fast` all ok; release build.
