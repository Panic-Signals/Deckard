---
name: pull-request
description: Use when opening, writing or updating a pull request for DECKARD - classifies the change (bug fix, enhancement, documentation, other) for the triager, writes a one-to-two sentence summary, reviewer test steps, target platforms, and before/after screenshots of UI changes.
---

# Writing a DECKARD pull request

A DECKARD pull request is read by two people:

- **The triager** reads the **Type** and adds the matching label. The label decides
  which section of the release notes the change lands in (`.github/release.yml`).
  Contributors don't add labels themselves.
- **The peer reviewer** reads the rest to find out what changed, check that it works,
  and see what it looks like.

The body follows `.github/pull_request_template.md`. Fill in every section. Write "None"
when a section doesn't apply, and never delete a heading.

## 1. Type

Tick exactly one box:

| Type | When | Triager's label |
|------|------|-----------------|
| Bug fix | Something that used to work, or should have worked, works now | `bug` |
| Enhancement | A new feature, or an existing one does more or works better | `enhancement` |
| Documentation | Only docs: README, DEVELOPMENT.md, `docs/`, the manual, comments | `documentation` |
| Other | Build, CI, tests, refactoring, dependencies: nothing a user would notice | `internal` |

If a change both fixes a bug and adds a feature, pick the one a user would notice
more, and mention the other in the summary. If the change mixes unrelated work,
split it into separate pull requests.

## 2. Summary

Write one or two sentences, for someone who uses DECKARD rather than someone who reads
its code: what they can do now, or what works now that didn't. Link the issue it fixes
(`Fixes #123`).

- Good: "Loading a Group from the browser no longer freezes the UI while its samples
  load. Fixes #118."
- Bad: "Refactor `preset_load.rs` to use `EditSnapshot`." This says how, not what.

Implementation detail the reviewer needs goes under **Notes for the reviewer**, not
here.

## 3. How to check it works

Give the reviewer numbered steps they can follow without reading the code:

1. **Setup:** the project, controller, plug-in or setting the steps need. Prefer
   something anyone has, like a new project or DECKARD's built-in instruments, over
   files from the NI library or your own machine.
2. **Actions:** exactly what to click, press or play.
3. **Expected result:** what the reviewer should see or hear. For a bug fix, also
   what happened before the fix, so they can confirm the problem first.

Then list the automated checks you ran and what came out of them, e.g.
`cargo test --workspace` and `cargo clippy --workspace --all-targets`. Say which tests
were skipped because they need local data, such as a Maschine install or library
projects (DEVELOPMENT.md, "Tests that need local data").

## 4. Platforms

Tick the systems the change is for: **All**, or any of **macOS**, **Linux** and
**Windows**. A change in shared code is **All**. Code behind `#[cfg(target_os = ...)]`,
Audio Units, ASIO, CoreMIDI or platform installers is for that system only.

Also say which systems you actually tried it on. If you're on a Mac, run
`scripts/check-platforms.sh` to type-check the other two before CI does.

## 5. Screenshots: before and after

For any change a user can see (the app window, menus, dialogs, the MK3's screens),
attach a **before** and an **after** picture of the same view. Documentation and
other changes with nothing to see: write "None".

DECKARD draws its own window with egui; it isn't a web page, so browser tools like
Playwright can't capture it. Use DECKARD's own screenshot mode instead
(DEVELOPMENT.md, "Command line"):

```sh
# Keep screenshots from touching your real settings, recent list and session.
export DECKARD_SETTINGS="$(mktemp -d)"

# Before: build and capture main.
git stash   # or: git switch main
cargo build
./target/debug/deckard path/to/project.mxprj --view ideas --group 0 --screenshot before.png

# After: build and capture your branch, with the same arguments.
git stash pop   # or: git switch -
cargo build
./target/debug/deckard path/to/project.mxprj --view ideas --group 0 --screenshot after.png
```

- **Same view:** use the same project, flags and window for both pictures, so the
  only difference is your change.
- **Choosing the view:** `--view ideas|song|mix|lfos`, `--group N`, `--sound N`,
  `--pattern N`, `--piano`, `--draw`, `--lane ...` and the others in DEVELOPMENT.md.
- **No flag for it:** script pointer input with `--input clicks.txt` (lines of
  `<seconds> <move|down|up> <x> <y>`), and add `--wait SECS` so it finishes before
  the picture is taken.
- **The MK3's screens:** the on-screen controller panel (`--scripts DIR`, `--press`,
  `--hold`, `--pad`). For hardware, a photo is fine.
- **Sound or timing changes:** describe what to listen for under **How to check it
  works**, and attach a short audio or screen recording if it helps.

Drag the PNGs into the pull request's description on GitHub to upload them, and put
them side by side:

```markdown
| Before | After |
|--------|-------|
| ![before](url) | ![after](url) |
```

Don't commit screenshots to the repository unless they're replacing images in
`docs/screenshots/`.

## Before opening it

- [ ] `cargo build` and `cargo test --workspace` pass.
- [ ] `cargo fmt` has been run.
- [ ] Every section of the template is filled in, or says "None".
- [ ] The title is the summary in a few words, written like the release notes
      ("Load Groups without freezing the UI"), not "fix bug" or a branch name.

Create the pull request with `gh pr create --body-file <file>` using the template's
sections, or paste them into GitHub's form. Then upload the screenshots there.
