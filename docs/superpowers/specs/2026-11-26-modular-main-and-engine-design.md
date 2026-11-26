# Breaking up `main.rs` and the engine's `lib.rs`

Date: 2026-11-26. Status: awaiting approval.

## Intent

Two files take most of the edits in the project, so pull requests collide in them when
`main` moves:

| | `apps/viewer/src/main.rs` | `crates/engine/src/lib.rs` |
|---|---|---|
| Lines | 22,108 | 16,495 |
| Inline tests | about 7,400 lines, from line 14,654 (168 tests) | about 8,600 lines, from line 7,862 (289 tests, in `tests` and `native_tests`) |
| Biggest block | one `impl App`, 12,348 lines, 344 methods | one `impl Engine`, 4,258 lines; `new_after` is 713 |
| Shared state | `App`: 259 fields | `Engine`: a 233-line struct |

In the 60 days to 2026-11-26, 146 of 314 commits touched `main.rs`, from about ten
contributors, most of them working with coding agents. Almost all of those edits are in
the `impl App` block. The points where two unrelated features meet are the `App` field
list and its defaults, and the long dispatch functions (`surface_act` 531 lines,
`surface_view` 485, `sampling_act` 355, `arrange` 270, `control_request` 247).

The aim is that two pull requests for two features rarely edit the same lines, and that
neither file is the place new code goes by default.

## Goals

- New code and new tests go into topic files, and a check and the agents' instructions
  keep it that way.
- `main.rs` and the engine's `lib.rs` shrink to what only they can hold: the module list,
  the central struct, start-up and the main loop.
- Each feature's state, handlers and tests live together in that feature's files.
- Every step is a small pull request that changes no behaviour and leaves `main` green.

## Non-goals

- No behaviour changes, renames, signature redesigns or reformatting ride along with a
  move.
- No new crates.
- `apps/viewer/src/layout.rs` (8,813 lines) and other large files are not part of this
  work. The size check lists them so they can't grow, and the same method applies to them
  later.

## Anti-goals

- One large pull request that moves everything. It would conflict with every open pull
  request at once.
- A move that also edits what it moves. A reviewer can't then tell the move from the edit.
- A size check so tight that an outside contributor's one-line fix fails it.
- Any change in the engine that alters what the audio thread does, allocates or locks.

## Constraints

- **Patch coverage.** CI fails a pull request unless tests run 70% of the lines it adds
  or changes (`.github/workflows/ci.yml`, job `coverage`). diff-cover can't tell a moved
  line from a new one, so a move of code that is untested today (most of the UI, and the
  tests that skip themselves without a Maschine library) would fail. See "Move-only pull
  requests" below.
- **Rust visibility.** A child module of the crate root can read `App`'s private fields
  and call its private methods. A method that moves to a child module and is called from
  another module needs `pub(crate)`. That is the only edit a move may make to a method.
- **Open pull requests.** A move conflicts with any open pull request that edits the
  moved lines, and Git won't carry the edit across. The contributor has to make the edit
  again in the new file.
- **The existing pattern.** 28 files in `apps/viewer/src` already hold their own
  `impl App` block and 9 in `crates/engine/src` their own `impl Engine`. Test files
  already exist beside the code (`chain_tests.rs`, `browser_tests.rs`,
  `tests/layers_ui.rs`, `sampler_layers_tests.rs`). The work extends these patterns and
  invents none.

## Approaches

1. **Stop the growth:** a CI check that caps file sizes. Cheap, conflicts with nothing,
   shrinks nothing.
2. **Move code as it stands:** tests into their own files, then groups of methods into
   topic files. No behaviour change. The shared struct stays.
3. **Thin the dispatchers:** each arm of the long `match` functions becomes a call into
   the topic's file, so two features add two separate lines.
4. **Split the state:** group the central struct's fields into per-feature structs owned
   by their modules. The only one that removes the coupling, and the riskiest.

Dropped: splitting the viewer into more crates. It is far more work and does nothing for
conflicts that approach 2 doesn't.

**Chosen (by Josh, 2026-11-26): all four, in that order.** Each makes the next one
smaller and safer: the check stops new code landing in the files while they are being
emptied, the moves put each topic's code in one place, the thin dispatchers and the
state split then work one topic at a time.

## Design

### Phase 1: the size check and the agents' instructions

**The check.** `scripts/check-file-sizes.sh`, run by `mise run lint` (which CI's Lint
step runs on Linux), reading `scripts/file-size-limits.txt`:

- The limits file lists a path and a maximum line count per line, for every `.rs` file
  that is over the default cap today.
- The default cap for any file not listed is 3,000 lines. A new file can't be added to
  the list to get round it; the script fails if a listed file is under the default cap
  (the entry must then be removed).
- A listed file's limit is its length when the limit was last set, rounded up to the
  next 250 lines. That leaves room for a small fix and for two pull requests that each
  add a few lines.
- Each pull request that moves code out of a listed file lowers that file's limit in the
  same pull request.
- The failure message names the file, its length and its limit, and says where new code
  goes (the section of DEVELOPMENT.md below).
- **No new tests in the two files.** (My call.) A second, permanent list,
  `scripts/no-inline-tests.txt`, names files that must not contain `#[test]`; the script
  fails if one does. It is separate from the limits file so the rule stays after a file
  drops under the default cap and leaves that list. The pull requests that move the
  tests out add `apps/viewer/src/main.rs` and `crates/engine/src/lib.rs` to it. Other
  crates' `lib.rs` files have inline tests today and are not listed; one is added when
  its tests are moved out. `AGENTS.md` states the rule for all of them, for new tests.

**Instructions for coding agents** (requested by Josh, 2026-11-26). There is no
`AGENTS.md` or `CLAUDE.md` in the repository today, only two skills in `.claude/skills`.
A skill is loaded only when an agent decides it applies, and only by Claude Code, so the
rules go where every agent reads them on every task:

- `AGENTS.md` at the repository root, short, with the layout rules:
  - Tests go in their own file, never inline in `main.rs` or a crate's `lib.rs`: a
    `<topic>_tests.rs` beside the code, or a file under the crate's `tests/` module
    folder, declared with `#[cfg(test)] mod …;`.
  - New code for a feature goes in that feature's file, as its own `impl App` or
    `impl Engine` block or its own type. Add a new file when no topic file fits.
  - Don't add a field to `App` or `Engine` when the feature has its own state struct;
    add it there.
  - A dispatcher arm is one call into the feature's file.
  - Don't move or reformat code you aren't changing.
  - Run `mise run lint` before opening a pull request.
- `CLAUDE.md` at the root containing only `@AGENTS.md`, so Claude Code loads the same
  text.
- A "Where code goes" section in DEVELOPMENT.md with the same rules for people, and the
  reason. `AGENTS.md` links to it rather than repeating the reasoning.
- The `pull-request` skill's "Before opening it" list gains one line: `mise run lint`
  passes, including the file size check.

The check is what enforces the rules; the instructions are there so an agent gets it
right the first time instead of finding out from CI.

**Move-only pull requests.** (My call; Josh to confirm.) A pull request labelled
`move-only` is not held to Patch coverage's 70%. The label is for pull requests that only move
code and make the visibility edits a move needs. A maintainer applies it (contributors
don't add labels), after checking the move is verbatim. The pull request body says it is
move-only and gives the before and after test counts. `ci.yml`'s `coverage` job gains
the exception; DEVELOPMENT.md's Patch coverage paragraph describes the label.

The coverage job reads the pull request's labels as they are when it runs (`gh pr
view`), not from the event that started it, and with `move-only` it reports coverage
without the 70% floor. A maintainer who adds the label after the job has failed re-runs
that one job. (An earlier draft re-ran CI on label events and skipped the build jobs on
them; that was dropped because a skipped build would replace a failed one in the pull
request's checks.)

If the label isn't wanted, the fallback is to leave the coverage check as it is and have
a maintainer merge move pull requests over its failure. Phase 2 is the same either way;
only this paragraph's CI change drops out.

Also in this phase: a `.git-blame-ignore-revs` file, with a line in DEVELOPMENT.md on
`git config blame.ignoreRevsFile`, so blame looks past the move commits. Each move pull
request adds its merge commit's hash in a follow-up line (squash merges mean the hash is
known only after merging; the next move pull request adds the previous one's hash).

### Phase 2: move code as it stands

Order: tests first, because they are the least-edited part of both files (about 30 of
1,300 changed hunks in `main.rs` in 60 days), then method groups.

**2a. The engine's tests.** `mod tests` and `mod native_tests` leave `lib.rs`:

- `crates/engine/src/tests/mod.rs` keeps the shared fixtures (`project()` and the other
  helpers, structs and statics the tests share) and declares one child module per topic.
- Each topic file starts with `use super::*;` and holds that topic's tests. No file over
  about 1,500 lines. The builder chooses the topics from what the tests exercise
  (transport, patterns and scenes, the sampler, effects and sends, modulation, loops and
  audio, plug-ins, `adopt`, and so on).
- `include!("additive_route_tests.rs")` becomes a normal child module if that needs no
  change to its tests; otherwise it stays as it is.
- `lib.rs` keeps `#[cfg(test)] mod tests;`. Module paths that other files use
  (`crate::tests::project`) keep working.

**2b. The app's tests.** The same for `main.rs`'s `mod tests`, into
`apps/viewer/src/tests/mod.rs` (the harness, `harness_of`, `PROJECT` and shared helpers)
and topic files beside the existing `tests/layers_ui.rs`. `crate::tests::harness` keeps
its path, so the existing `*_tests.rs` files don't change.

**2c. Method groups out of `impl App`.** One pull request per topic, each moving whole
methods, unchanged, into that topic's file as an `impl App` block. Where a topic file
exists already (for example `browser.rs`, `controller.rs`, `layout/sampling.rs`), the
methods join it or sit beside it in a sibling file; the builder picks whichever keeps
files under the default cap. First topics, by size:

- the controller surface: `surface_act`, `surface_view`, `control_request`, `mk3_events`
  and their helpers
- preferences: `preferences`, `audio_preferences` and the other preference panes
- sampling: `sampling_act`, `update_sampling`, `sampling_knob`
- arranging: `arrange`
- the browser: `browse`
- meters: `update_meters`

then the remaining methods by topic until the block in `main.rs` holds only start-up,
the frame loop and what several topics share.

**2d. Method groups out of `impl Engine`**, the same way: construction (`new_after` and
its helpers), `adopt_with`, triggering (`trigger_with`, `catch_beat_hits`), rendering
(`process_span`, `render_pass`, `ahead_returns`), modulation (`apply_mods`). The other
types in `lib.rs` that have a clear home (`FxChain`, `SampleCache`, `CueBus`, `GainRamp`,
`Timelines`) move to files of their own, re-exported from `lib.rs` so the crate's public
paths don't change.

**How a move is checked.** For every pull request in this phase:

- `cargo test --workspace -- --list` gives the same number of tests before and after,
  and for a test move the same set of test names apart from their module prefix.
- `cargo test --workspace` and `mise run lint` pass, and `scripts/check-platforms.sh`
  type-checks the other systems. Clippy is not a gate: CI doesn't run it and the
  workspace isn't known to be clean under it. A move must not add a clippy warning in
  the lines it moves, which holds by construction when the text is unchanged.
- The moved text is identical apart from indentation and `pub(crate)`: the builder
  checks this mechanically (for example, sorted non-blank lines removed equal sorted
  non-blank lines added, ignoring leading whitespace and the visibility edits) and says
  so in the pull request.
- For 2d, the engine's existing render and performance-regression tests pass unchanged.

**Aims, not gates:** `main.rs` under 3,000 lines and the engine's `lib.rs` under 3,000
by the end of phase 2, at which point both leave the limits file.

### Phase 3: thin the dispatchers

For `surface_act`, `surface_view`, `sampling_act`, `arrange`, `control_request` and
`browse`, and the engine's equivalents if any remain after 2d:

- An arm whose body is more than about five lines becomes a method in the file of the
  feature it serves, named for what it does, and the arm becomes a call to it.
- Arms keep their order and their patterns. The dispatcher stays in one place; only the
  bodies move.
- One pull request per dispatcher. These are move-only when the body moves verbatim into
  a method whose parameters are exactly the arm's bindings; where a body needs any other
  change to compile, it is an ordinary pull request with ordinary coverage.
- `surface_view` builds one value rather than dispatching: it is split into one helper
  per section of the view, each in its feature's file.

### Phase 4: split the state

One feature per pull request, starting with the most self-contained (the builder ranks
them by how many methods outside the feature read each field; preferences UI state and
sampling state are the likely first two):

- The feature's fields move from `App` into a struct in the feature's file, with its own
  `Default`, and `App` holds one field of that type (`app.sampling.…`).
- Methods that need only that struct become methods on it. Methods that need the rest of
  `App` stay `impl App` in the feature's file.
- Field names inside the struct drop the feature prefix they needed in `App`
  (`sampling_mode` becomes `sampling.mode`). No other renames.
- These are ordinary pull requests: they change code, so patch coverage applies. Where a
  moved field's readers are untested, the pull request adds the tests it needs to reach
  70%, in the feature's test file.
- Anything saved to disk or exposed to Lua scripts or the controller layer keeps its
  format and names. Settings and project files are not touched by this phase.

For the engine, phase 4 is narrower, because the struct is read on the audio thread:

- `new_after` is split into one helper per subsystem, each returning the state it
  builds.
- Fields are grouped into per-subsystem structs only where the group is built, adopted
  and used together already, and only if the grouping changes no allocation, lock or
  ordering on the audio thread.
- Each pull request runs the engine's performance-regression tests and reports the
  numbers before and after.

Phase 4 ends when adding a typical feature needs no new field on `App`; it has no line
target.

## Working with open pull requests

- One topic per pull request, opened when no large pull request is open against that
  topic, and merged the same day where possible. Josh reviews and merges them.
- (My call.) The two test moves, 2a and 2b, are one pull request each, though each is
  several thousand lines: the test sections are rarely edited, so one move conflicts
  with little, and a dozen serial pull requests per file would take weeks for no gain.
  Each has one commit per topic file, so it can be reviewed a topic at a time.
- Before each move, the builder lists open pull requests that touch the lines to be
  moved (`gh pr list` and each one's diff) and names them in the move's description, so
  their authors know to re-apply their edits in the new file.
- No two move pull requests for the same file are open at once.

## Left to the builder

- The topic boundaries for test files and method groups, within the size guide.
- File names, following what's there (`<topic>.rs`, `<topic>_tests.rs`, `tests/<topic>.rs`).
- The order of topics after the first ones listed.
- The script's implementation (bash, passing `shellcheck`).
- The exact wording of `AGENTS.md` and the DEVELOPMENT.md section.

## Plans

Each phase gets its own implementation plan. Phase 1 and phases 2a and 2b are planned
first; 2c and 2d are planned once the tests are out, and phases 3 and 4 once phase 2 has
shown where the topic boundaries fell.
