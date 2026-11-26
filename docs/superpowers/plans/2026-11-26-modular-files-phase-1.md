# Modular Files, Phase 1 and the Test Moves: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop `apps/viewer/src/main.rs` and `crates/engine/src/lib.rs` growing, tell coding agents where code and tests go, and move both files' inline tests into their own files.

**Architecture:** A shell script run by `mise run lint` caps file sizes from a limits file and forbids `#[test]` in listed files. `AGENTS.md` carries the layout rules for every coding agent. Two move-only pull requests then turn each file's inline `mod tests { … }` into a `tests/` module folder, changing no test.

**Tech Stack:** bash (must pass `shellcheck`), GitHub Actions (must pass `actionlint`), Rust 1.99.1 via mise, rustfmt (`max_width = 140`, `use_small_heuristics = "Max"`).

**Spec:** `docs/superpowers/specs/2026-11-26-modular-main-and-engine-design.md` (phases 1, 2a, 2b). Phases 2c, 2d, 3 and 4 get their own plans.

## Global Constraints

- Three pull requests, in this order, each from a branch off current `main`: **A** (tasks 1 to 3), **B** (task 4), **C** (task 5). B and C are opened only after A is merged. B and C are never open against the same file as another move.
- Pull request bodies follow `.github/pull_request_template.md`; type is "Other"; Manual is "None". Use the repository's `pull-request` skill.
- Tasks 4 and 5 change no behaviour: no test is added, removed, renamed or edited. The only permitted edits to moved text are indentation, what `cargo fmt` does as a result of the new indentation, `use` lines a file needs to see what it saw before, and `pub(super)`/`pub(crate)` on a helper another file now calls.
- Default cap for any `.rs` file not in the limits file: 3,000 lines.
- A listed file's limit is its line count at the time it is set, rounded up to the next multiple of 250.
- Every pull request passes `mise run lint`, `cargo build --workspace --all-targets --locked` and `cargo test --workspace --locked --no-fail-fast`, and `scripts/check-platforms.sh` on a Mac.
- Run cargo through mise (`mise run …` or `mise exec -- cargo …`).
- Commit messages end with the session's `Co-Authored-By` line. Do not push or open a pull request without Josh's go-ahead for that pull request.

## Review Focus

1. **A listed file is renamed or deleted.** The check must fail with a message naming the stale entry, not pass silently or print a `wc` error. (Task 1, case `stale-entry`.)
2. **A file without a trailing newline, or with CRLF line endings.** The count must match what an editor shows (count lines, not newline bytes), so a 3,000-line file with no final newline is 3,000. (Task 1, case `no-trailing-newline`.)
3. **Untracked and build-output files.** `target/`, a contributor's scratch `.rs` files and anything git-ignored must not be counted; only `git ls-files '*.rs'`. (Task 1, case `untracked-ignored`.)
4. **`#[test]` in text that isn't a test.** A listed no-inline-tests file may mention `#[test]` in a comment or string; only a line that is the attribute itself (optional leading whitespace, then `#[test]` or `#[tokio::test]`-style `#[…::test]`) counts. (Task 1, case `test-in-comment`.)
5. **A label added after the coverage job has failed.** Re-running that one job must see the label; it reads the labels when it runs, not from the event that started the run. (Task 3.)

---

### Task 1: The file size check

**Files:**
- Create: `scripts/check-file-sizes.sh`
- Create: `scripts/check-file-sizes.test.sh`
- Create: `scripts/file-size-limits.txt`
- Create: `scripts/no-inline-tests.txt`
- Modify: `mise.toml` (the `lint` task and the usage comment at the top)

**Interfaces:**
- Produces: `scripts/check-file-sizes.sh [--root DIR]`. `DIR` defaults to the repository root (the script's parent directory); it must be a git work tree. Exit 0 when every rule holds, 1 otherwise, printing one line per violation to stderr and then one line pointing at `DEVELOPMENT.md`, "Where code goes".
- Produces: `scripts/file-size-limits.txt`: lines of `<path> <max-lines>`, `#` comments and blank lines ignored, paths relative to the root.
- Produces: `scripts/no-inline-tests.txt`: one path per line, same comment rules. Created empty apart from its header comment; tasks 4 and 5 add to it.

Rules the script enforces, each with its message:

| Rule | Message (stderr) |
|---|---|
| A tracked `.rs` file not in the limits file has more than 3,000 lines | `<path>: <n> lines, over the 3000-line cap` |
| A listed file has more lines than its limit | `<path>: <n> lines, over its limit of <max>` |
| A listed file has 3,000 lines or fewer | `<path>: <n> lines, under the cap: remove it from scripts/file-size-limits.txt` |
| A listed file (either list) is not a tracked file | `<path>: listed in <list file> but not in the repository` |
| A file in `no-inline-tests.txt` has a test attribute line | `<path>:<line>: inline test; tests go in their own file` |

- [ ] **Step 1: Write the failing tests** in `scripts/check-file-sizes.test.sh`. Each case builds a temporary git repository (`git init`, files made with `seq`/`printf`, `git add`), copies in the two list files it needs under `scripts/`, runs `scripts/check-file-sizes.sh --root "$tmp"`, and asserts on exit status and stderr. The script prints `ok <case>` or `FAIL <case>` per case and exits non-zero if any failed. Cases:

  - `clean`: one 10-line `.rs` file, empty lists. Exit 0, empty stderr.
  - `over-default-cap`: `a.rs` with 3,001 lines. Exit 1, stderr contains `a.rs: 3001 lines, over the 3000-line cap`.
  - `at-default-cap`: `a.rs` with exactly 3,000 lines. Exit 0.
  - `listed-within-limit`: `big.rs` 3,400 lines, limit 3,500. Exit 0.
  - `listed-over-limit`: `big.rs` 3,501 lines, limit 3,500. Exit 1, `big.rs: 3501 lines, over its limit of 3500`.
  - `listed-under-cap`: `big.rs` 2,000 lines, limit 3,500. Exit 1, `under the cap: remove it`.
  - `stale-entry`: limits file lists `gone.rs`, which doesn't exist. Exit 1, `gone.rs: listed in scripts/file-size-limits.txt but not in the repository`.
  - `no-trailing-newline`: `a.rs` with 3,000 lines and no final newline (`printf` without the last `\n`). Exit 0. And with 3,001 lines and no final newline: exit 1, reports 3001.
  - `untracked-ignored`: a tracked 10-line file plus an untracked 5,000-line `scratch.rs` and a `.gitignore`d `target/x.rs` of 5,000 lines. Exit 0.
  - `inline-test`: `lib.rs` listed in `no-inline-tests.txt`, containing `    #[test]` on line 7. Exit 1, `lib.rs:7: inline test`.
  - `test-in-comment`: the listed file contains `// see #[test] below` and `let s = "#[test]";` only. Exit 0.
  - `path-test`: the listed file contains `#[tokio::test]` on its own line. Exit 1.
  - `comments-and-blanks`: list files with `#` comment lines and blank lines parse without error.
  - `path-with-space`: a tracked `my dir/a.rs` with 3,001 lines. Exit 1, the path printed whole.

- [ ] **Step 2: Run them to see them fail**

Run: `bash scripts/check-file-sizes.test.sh`
Expected: every case `FAIL` (the script under test doesn't exist).

- [ ] **Step 3: Write `scripts/check-file-sizes.sh`.** `set -euo pipefail`; list files with `git -C "$root" ls-files -z '*.rs'`; count lines with `grep -c ''` (counts a last line that has no newline); collect all violations before exiting, so one run reports everything. Header comment in the style of the other scripts: what it checks, how to run it, where the lists are.

- [ ] **Step 4: Run the tests**

Run: `bash scripts/check-file-sizes.test.sh`
Expected: every case `ok`, exit 0.

- [ ] **Step 5: Write the two list files.** `scripts/file-size-limits.txt` gets a header comment (what the number means, that a move lowers it in the same pull request, that new entries are not added to get round the cap) and one entry for every tracked `.rs` file over 3,000 lines. Recompute at execution time with `git ls-files '*.rs' | xargs wc -l`; on 2026-11-26 that gave:

  | File | Lines | Limit |
  |---|---|---|
  | `apps/viewer/src/main.rs` | 22,108 | 22250 |
  | `crates/engine/src/lib.rs` | 16,495 | 16500 |
  | `apps/viewer/src/layout.rs` | 8,813 | 9000 |
  | `crates/surface/src/tests.rs` | 4,685 | 4750 |
  | `crates/engine/src/sampler.rs` | 3,481 | 3500 |
  | `apps/viewer/src/layout/sampling.rs` | 3,259 | 3500 |

  `scripts/no-inline-tests.txt` gets a header comment only.

- [ ] **Step 6: Wire it into lint.** In `mise.toml`, add `"scripts/check-file-sizes.test.sh"` and `"scripts/check-file-sizes.sh"` to the `lint` task's `run` list (after `shellcheck`), change the task's description to `rustfmt, the GitHub workflows, the shell scripts and file sizes`, and update the `mise run lint` line in the usage comment to match. Make both scripts executable.

- [ ] **Step 7: Verify**

Run: `mise run lint`
Expected: passes, including `shellcheck` on both new scripts.

- [ ] **Step 8: Commit** (`lint: cap file sizes and forbid inline tests in listed files`).

---

### Task 2: Instructions for agents and people

**Files:**
- Create: `AGENTS.md`
- Create: `CLAUDE.md`
- Create: `.git-blame-ignore-revs`
- Modify: `DEVELOPMENT.md` (new section "Where code goes", after "The workspace"; a line in "Releases and CI"; an entry in "Documentation map" if that section lists top-level files)
- Modify: `.claude/skills/pull-request/SKILL.md` ("Before opening it")

**Interfaces:**
- Consumes: `scripts/check-file-sizes.sh`, the two list files and `mise run lint` from Task 1.
- Produces: the DEVELOPMENT.md heading `## Where code goes`, which the check's failure message and `AGENTS.md` point to.

- [ ] **Step 1: Write `## Where code goes` in DEVELOPMENT.md.** For people, in the document's existing voice, about 25 lines. It says:
  - why: `main.rs` and the engine's `lib.rs` take most of the project's edits, and two pull requests that touch the same lines conflict;
  - tests go in their own file: `<topic>_tests.rs` beside the code or a file in the crate's `tests/` module folder, declared `#[cfg(test)] mod …;`, never inline in `main.rs` or a crate's `lib.rs`;
  - a feature's code goes in that feature's file as its own `impl App` / `impl Engine` block or its own type; add a file when none fits;
  - a feature's state goes in the feature's own struct where it has one, not a new field on `App` or `Engine`;
  - a dispatcher arm is one call into the feature's file;
  - don't move or reformat code you aren't changing; moves are done one topic at a time in their own pull requests;
  - the check: what `scripts/check-file-sizes.sh` enforces, the 3,000-line cap, the two list files, and that it runs in `mise run lint`.

- [ ] **Step 2: Write `AGENTS.md`**, under 40 lines: one paragraph on what DECKARD is and that DEVELOPMENT.md is the technical reference; a "Where code goes" list with the six rules above as imperatives, linking to the DEVELOPMENT.md section for the reasons; "Before opening a pull request": `mise run lint`, `mise run test`, and the pull request template (in Claude Code, the `pull-request` skill). Use `mise run`, not bare `cargo`.

- [ ] **Step 3: Write `CLAUDE.md`** containing exactly one line: `@AGENTS.md`.

- [ ] **Step 4: Create `.git-blame-ignore-revs`** with a header comment only: what the file is for, that each move-only pull request's merge commit is added by the next pull request, and the one-time command `git config blame.ignoreRevsFile .git-blame-ignore-revs`. Add one sentence on it to the end of "Where code goes".

- [ ] **Step 5: Add to the `pull-request` skill's "Before opening it" list:** `- [ ] \`mise run lint\` passes, including the file size check (DEVELOPMENT.md, "Where code goes").`

- [ ] **Step 6: Verify**

Run: `mise run lint && grep -c '' AGENTS.md CLAUDE.md`
Expected: lint passes; `AGENTS.md` under 40 lines; `CLAUDE.md` is 1 line.

- [ ] **Step 7: Commit** (`docs: where code goes, for people and coding agents`).

---

### Task 3: The `move-only` label skips Patch coverage

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `DEVELOPMENT.md` (the "Patch coverage" bullet in "Releases and CI")
- Modify: `.claude/skills/pull-request/SKILL.md` (the Type table's note, one sentence)

**Interfaces:**
- Produces: the label name `move-only`, used by tasks 4 and 5's pull requests.

- [ ] **Steps 1 to 3 (revised while executing): lift the 70% for a labelled pull request, in the enforcing step only.** The triggers, the concurrency group and the `build` job don't change. In the `coverage` job's "At least 70% of changed lines covered" step, read the pull request's labels as they are now (`gh pr view "$PR" --repo "$GITHUB_REPOSITORY" --json labels`, with `GH_TOKEN: ${{ github.token }}` and `PR: ${{ github.event.pull_request.number }}`), and pass `--fail-under=0` to diff-cover when `move-only` is among them, `70` otherwise. Coverage is still measured and reported. A maintainer who adds the label after the job failed re-runs that job. The first draft of this task re-ran CI on label events with the build jobs skipped; a skipped build would have replaced a failed one in the pull request's checks, so it was dropped.

- [ ] **Step 4: Document it.** In DEVELOPMENT.md's Patch coverage bullet, add two sentences: what `move-only` is for, and that a maintainer applies it after checking the move is verbatim. In the `pull-request` skill, under the Type table: a move-only pull request is "Other", says "move-only" in its summary, and gives the test counts before and after.

- [ ] **Step 5: Verify**

Run: `mise run lint`
Expected: `actionlint` passes on `ci.yml`.

- [ ] **Step 6: Commit** (`CI: a move-only label skips patch coverage`).

- [ ] **Step 7: Open pull request A** (with Josh's go-ahead): title "Cap file sizes and say where code goes". In "Notes for the reviewer": the `move-only` label has to be created in the repository (`gh label create move-only --description "Code moved, not changed: skips patch coverage" --color ededed`); ask Josh to create it or to approve running that command.

---

### Task 4: Move the engine's tests out of `lib.rs` (pull request B)

**Files:**
- Modify: `crates/engine/src/lib.rs` (remove lines 7862 to 16495 as of 2026-11-26: `mod tests { … }` and `mod native_tests { … }`; leave two declarations)
- Create: `crates/engine/src/tests/mod.rs` and topic files `crates/engine/src/tests/<topic>.rs`
- Create: `crates/engine/src/native_tests.rs`
- Modify: `scripts/file-size-limits.txt`, `scripts/no-inline-tests.txt`, `.git-blame-ignore-revs` (nothing to add yet: A was not a move)

**Interfaces:**
- Consumes: the check and lists from Task 1; the `move-only` label from Task 3.
- Produces: `crate::tests` with the same shared items it has today (`project()` and the other fixtures keep their names, signatures and visibility), so `additive_route_tests.rs`, `sampler_layers_tests.rs` and any file that names `crate::tests::…` compile unchanged.

- [ ] **Step 1: Record the baseline.** Branch from current `main`.

Run: `mise exec -- cargo test -p deckard-engine --locked -- --list 2>/dev/null | grep ': test$' | sed 's/: test$//' | awk -F'::' '{print $NF}' | sort > /tmp/engine-tests-before.txt; wc -l < /tmp/engine-tests-before.txt`
Expected: a count (about 300 for the lib target plus the crate's other test targets). Keep the file. Use the session's scratchpad folder in place of `/tmp` if it has one.

- [ ] **Step 2: Move `mod native_tests`** to `crates/engine/src/native_tests.rs`: the module's body, de-indented one level, unchanged. `lib.rs` keeps `#[cfg(test)]\nmod native_tests;`. Its module path doesn't change.

- [ ] **Step 3: Move `mod tests`** into `crates/engine/src/tests/`:
  - `tests/mod.rs` holds the module's `use` lines, every item more than one topic uses (fixtures, helper structs and their `impl`s, statics, consts), and one `mod <topic>;` per topic file. The existing `include!("additive_route_tests.rs")` becomes `include!("../additive_route_tests.rs")` and stays in `mod.rs`.
  - Each topic file starts with `use super::*;` and holds whole test functions, plus helpers only that topic uses. Topics follow what the tests exercise (for example transport, patterns and scenes, sampler, effects and sends, modulation, audio loops, plug-ins, `adopt`, export). No topic file over 1,500 lines; `mod.rs` under 1,500.
  - Within a file, tests keep the relative order they had in `lib.rs`.
  - A helper in one topic file that another topic file needs moves to `mod.rs` rather than being made `pub`.
  - `lib.rs` keeps `#[cfg(test)]\nmod tests;` where the module was.
  - One commit per topic file, so the pull request can be reviewed a topic at a time. Each commit compiles.

- [ ] **Step 4: Format.**

Run: `mise exec -- cargo fmt --all`
Expected: only the new files change (shorter indentation lets rustfmt join some wrapped lines).

- [ ] **Step 5: Check the same tests exist.**

Run the Step 1 command into `/tmp/engine-tests-after.txt`, then `diff /tmp/engine-tests-before.txt /tmp/engine-tests-after.txt`
Expected: no output.

- [ ] **Step 6: Check the text is unchanged.** Compare the removed and added Rust text with all whitespace deleted and a trailing comma before `)`, `]` or `}` deleted (what rustfmt changes when it re-joins a line), as counts of each character:

Run:

```sh
git diff main -- crates/engine/src | python3 -c '
import sys, re, collections
removed, added = collections.Counter(), collections.Counter()
for line in sys.stdin:
    if line.startswith(("+++", "---")) or line[:1] not in "+-":
        continue
    (added if line[0] == "+" else removed).update(re.sub(r"\s", "", line[1:]))
print("only removed:", dict(removed - added))
print("only added:", dict(added - removed))
'
```

Expected: the character counts differ only by the characters of the new `mod <topic>;`, `use super::*;` and `include!` path lines, commas, and any `pub(super)` added. List every difference in the pull request body. Any other difference means text was edited: find it and undo it.

- [ ] **Step 7: Lower the limit and forbid inline tests.** In `scripts/file-size-limits.txt`, set `crates/engine/src/lib.rs` to its new length rounded up to the next 250 (about 8,000). Add `crates/engine/src/lib.rs` to `scripts/no-inline-tests.txt`.

- [ ] **Step 8: Verify.**

Run: `mise run lint && mise exec -- cargo build --workspace --all-targets --locked && mise exec -- cargo test --workspace --locked --no-fail-fast && scripts/check-platforms.sh`
Expected: all pass; the engine's test count matches Step 1.

- [ ] **Step 9: Commit the list changes, then open pull request B** (with Josh's go-ahead): title "Move the engine's tests into their own files". The body says it is move-only, gives the test counts before and after, pastes Step 6's differences, lists any open pull requests that edit `crates/engine/src/lib.rs` after line 7861 (`gh pr list` and `gh pr diff <n> --name-only`), and asks for the `move-only` label.

---

### Task 5: Move the app's tests out of `main.rs` (pull request C)

**Files:**
- Modify: `apps/viewer/src/main.rs` (remove `mod tests { … }`, lines 14654 to 21085 as of 2026-11-26; the code after it stays)
- Create: `apps/viewer/src/tests/mod.rs` and topic files `apps/viewer/src/tests/<topic>.rs` (beside the existing `tests/layers_ui.rs`, which doesn't change)
- Modify: `scripts/file-size-limits.txt`, `scripts/no-inline-tests.txt`, `.git-blame-ignore-revs` (add pull request B's merge commit hash, with a comment naming it)

**Interfaces:**
- Consumes: the check and lists from Task 1; the `move-only` label from Task 3.
- Produces: `crate::tests` with today's shared items at the same paths and visibility: `pub(super) fn harness(scripts: bool, song_view: bool) -> Option<Harness<'static, App>>`, `harness_of`, `PROJECT`, and whatever else the existing `*_tests.rs` files and `tests/layers_ui.rs` name. Those files compile unchanged.

- [ ] **Step 1: Record the baseline**, as Task 4 Step 1 with `-p deckard` (about 1,020 tests in the binary's test target).

- [ ] **Step 2: Move the module** by Task 4 Step 3's rules, with these differences: `tests/mod.rs` keeps the module's doc comment (`//! The window driven with simulated mouse input (egui_kittest).`), its `use` lines, `mod layers_ui;`, the harness functions and shared helpers; topics follow the app's areas (for example mixer and extra outputs, pattern editor, arranger and song view, browser, sampling, plug-in chains, controller pages, preferences, project files). `main.rs` keeps `#[cfg(test)]\nmod tests;` where the module was. One commit per topic file.

- [ ] **Step 3: Format, check the test names and check the text**, as Task 4 Steps 4 to 6, on `apps/viewer/src`.

- [ ] **Step 4: Lower the limit and forbid inline tests.** Set `apps/viewer/src/main.rs` in `scripts/file-size-limits.txt` to its new length rounded up to the next 250 (about 15,750). Add `apps/viewer/src/main.rs` to `scripts/no-inline-tests.txt`. Add B's merge commit to `.git-blame-ignore-revs`.

- [ ] **Step 5: Verify**, as Task 4 Step 8. Tests that need a Maschine library skip themselves as before; the count of tests listed, not run, is what must match.

- [ ] **Step 6: Commit, then open pull request C** (with Josh's go-ahead): title "Move the app's tests into their own files", body as Task 4 Step 9, for `apps/viewer/src/main.rs` lines 14654 to 21085.

---

## After this plan

The next plan covers phases 2c and 2d (method groups out of `impl App` and `impl Engine`), written once B and C are merged and the limits show what is left.
