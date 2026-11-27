# Working on DECKARD

DECKARD is a Rust workspace: an egui app (`apps/viewer`) and the crates under `crates/`
that load, play and edit Maschine projects. [DEVELOPMENT.md](DEVELOPMENT.md) is the
technical reference: building, the workspace's layout, testing, and how each part works.

Run tasks through mise (`mise run build`, `mise run test`, `mise run lint`), not bare
`cargo`: the pinned Rust comes from mise.

## Where code goes

`apps/viewer/src/main.rs` and `crates/engine/src/lib.rs` are too large already, and
every edit to them risks a merge conflict. The reasons and the check that enforces this
are in [DEVELOPMENT.md, "Where code goes"](DEVELOPMENT.md#where-code-goes).

- Put tests in their own file: `src/<topic>_tests.rs` beside the code (as
  `apps/viewer/src/chop_tests.rs` is), declared with `#[cfg(test)] mod <topic>_tests;`.
  Never write a test inline in `main.rs` or a crate's `lib.rs`. A crate's top-level
  `tests/` folder is for Cargo's integration tests and can't see private items: unit
  tests don't go there.
- Put a feature's code in that feature's file, as its own `impl App` or `impl Engine`
  block or its own type. Add a new file when no existing one fits.
- Put a feature's state in the feature's own struct where it has one. Don't add a field
  to `App` or `Engine` for it.
- Keep a dispatcher arm (`surface_act`, `sampling_act`, `arrange` and the like) to one
  call into the feature's file.
- Don't move or reformat code you aren't changing.
- Don't raise a limit in `scripts/file-size-limits.txt`. Move your addition to another
  file.

## Before opening a pull request

- `mise run lint` passes (formatting, workflows, scripts, file sizes).
- `mise run test` passes.
- The description follows `.github/pull_request_template.md`. In Claude Code, the
  `pull-request` skill writes it and checks the rules above first.
