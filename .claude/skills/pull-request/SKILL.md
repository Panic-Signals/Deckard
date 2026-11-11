---
name: pull-request
description: Use when opening, writing or updating a pull request for DECKARD - classifies the change (bug fix, enhancement, documentation, other) for the triager, writes a one-to-two sentence summary, reviewer test steps, and target platforms, and asks whether new hotkeys, hidden UI or non-Maschine features need a pull request to the DECKARD manual, then writes it with screenshots and diagrams and links it.
---

# Writing a DECKARD pull request

A DECKARD pull request is read by two people:

- **The triager** reads the **Type** and adds the matching label. The label decides
  which section of the release notes the change lands in (`.github/release.yml`).
  Contributors don't add labels themselves.
- **The peer reviewer** reads the rest to find out what changed and check that it
  works.

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

## 5. Manual

The user manual lives in a different repository,
[Panic-Signals/panic-signals-site](https://github.com/Panic-Signals/panic-signals-site),
under `src/content/deckard-manual/`, and is published at
`panicsignals.tv/software/deckard/manual/`. A change needs a manual update when it adds
at least one of these:

- **A new hotkey or button combination**, on the keyboard or on a controller
  (e.g. <kbd>SHIFT</kbd> + <kbd>STEP</kbd>, a new modifier + pad action).
- **A new UI element that needs explaining, or that no one would find by
  themselves**: something behind a modifier-click, a right-click menu item, a new
  controller page, or a panel whose controls aren't self-explanatory.
- **A feature that NI's Maschine software doesn't have.** The manual only covers where
  DECKARD differs from Maschine and points to NI's manuals for the rest, so anything
  that works the way Maschine does needs no update.

Bug fixes that make something work like Maschine, and changes a user wouldn't notice,
never need one.

### Ask first

Before opening the DECKARD pull request, check the diff against the list above. If
anything qualifies, ask the user with AskUserQuestion. Name what you found
("Adds <kbd>SHIFT</kbd> + <kbd>DUPLICATE</kbd> on the JAM and a new Take menu in the
pattern editor. Neither is in Maschine."), and offer:

- **Open a manual PR (Recommended):** you write it as described below.
- **Not needed:** write "None" under **Manual**.
- **Later:** write "To do: <what needs documenting>" under **Manual**, so the triager
  sees it's outstanding.

If nothing qualifies, don't ask. Write "None" under **Manual**.

### Write the manual change

1. Clone the site into the scratchpad (never into the DECKARD tree), and branch from
   `prerelease`, not `main`. Manual changes for DECKARD go to `prerelease` first:

   ```sh
   gh repo clone Panic-Signals/panic-signals-site <scratchpad>/panic-signals-site
   cd <scratchpad>/panic-signals-site
   git switch prerelease && git switch -c manual/<deckard-branch-name>
   npm ci
   ```

2. Read `src/content/deckard-manual/README.md` (front matter, `<kbd>`, figures, notes,
   split tables) and the chapter the feature belongs in. Prefer a new section in an
   existing chapter to a new chapter:

   | Feature | Chapter |
   |---------|---------|
   | MK3 buttons and pages | `maschine-mk3.md`, plus its **At a glance** table |
   | Launchpad Pro MK3 | `launchpad-pro-mk3.md` |
   | Something Maschine does differently or doesn't have, in general | `differences-from-maschine.md` |
   | LFOs, envelopes, mod matrix | `modulation.md` |
   | Pressure, bend, note expression | `expression.md` |
   | Built-in instruments | `synths.md` |
   | CV, Zhora, modular | `modular-synths.md` |
   | Multi-output plug-ins, extra outs | `multi-output-plug-ins.md` |
   | Themes and Lua scripts | `themes-and-scripts.md` |

3. Write it the way the existing chapters are written: short sentences in the second
   person, saying what the user does and what happens, buttons and keys as
   `<kbd>…</kbd>`, settings and on-screen labels in **bold** as they appear. Link
   related sections with `[text](#section-slug)`. Don't number headings; the site does
   that.
4. If the DECKARD change isn't in a release yet, mark it: in a new chapter set
   `preview: Only in the next release.` in the front matter; in an existing chapter
   start the section with `> **Note:** Only in the next release.`
5. A new chapter takes the next free `order` (or shifts the ones after it), and
   `tests/e2e/manual.spec.ts` counts the chapters and checks some by position. Update it.
6. Keep to the public-docs rules: no mention of how anything in Maschine was worked
   out, and no tools for taking NI's software apart.

### Screenshots

Screenshots are PNGs in `public/manual/screenshots/`, named in kebab case with the
controller first when it's from a controller (`mk3-tuner.png`, `outs-mixer.png`).
Make them with DECKARD itself, so they're reproducible and match the rest:

- **The app window.** Run the build from this branch with a scratch settings folder
  and the default (Maschine) theme, and without `--scripts`, so the on-screen
  controller panel never shows:

  ```sh
  DECKARD_SETTINGS=<scratchpad>/settings cargo run --release -- \
    [project.mxprj] --view ideas|song|mix|lfos [--group N --sound N --pattern N] \
    [--piano] [--input clicks.txt] --wait 4 --screenshot <scratchpad>/full.png
  ```

  Reach state no flag gets you to (a menu open, a tab chosen) with `--input`: lines
  of `<seconds> <move|down|up> <x> <y>` in window points (DEVELOPMENT.md, "Command
  line"). Use a new project or the built-in instruments, never NI library content
  or the user's own projects.

- **The MK3's screens.** Render both screens (960 × 272) with the controller host,
  pressing the buttons that get to the page:

  ```sh
  cargo run --release -p deckard-lua-host --bin lua-host-run -- <scripts-dir> \
    --project new --hold BUTTON_SHIFT --press BUTTON_SELECT --png <scratchpad>/mk3-<page>.png
  ```

  `<scripts-dir>` is the controller scripts folder set in the user's DECKARD settings.
  Ask the user for it if you can't find it; don't go looking through their Maschine
  install.

- **Crop to the subject.** Keep what the section talks about and enough around it to
  place it. Full-width app crops are 1950 px wide; panels and menus are cropped
  tight (about 400 to 650 px wide). With ImageMagick:
  `magick full.png -crop <w>x<h>+<x>+<y> +repage public/manual/screenshots/<name>.png`
  (or Pillow, `Image.open(...).crop((x, y, x + w, y + h)).save(...)`). Keep each file
  under about 100 KB.
- **Look at every PNG with Read** before committing it: right state, nothing personal
  on screen (file paths, plug-in names from the user's machine), no controller panel.
- Put it in the chapter as a figure. Use `class="screenshot"`, and add `narrow` for
  anything under about 650 px wide. The alt text says what's on screen in enough detail
  for someone who can't see it, the way the existing ones do:

  ```html
  <figure class="screenshot">
    <img src="/manual/screenshots/<name>.png" alt="What is on screen, specifically." loading="lazy" />
    <figcaption>One short sentence.</figcaption>
  </figure>
  ```

### Diagrams

The controller drawings in `public/manual/diagrams/` are made by
`scripts/manual-diagrams.mjs`. Never edit the SVGs by hand.

- To mark a new button on an existing drawing, pass `{ hi: true }` to its `button()`
  or `knob()` call (cyan means DECKARD adds or changes it). If the chapter has an
  **At a glance** table, add a `badge(x, y, n)` whose number matches the new table row.
- For a new page on a controller's screens, write a function on the model of
  `mk3Lfo()` using `displayArea({ title, buttons, knobs, screens, note })`, and add it
  to `files` at the bottom. Use only the palette `C` and the helpers, so it matches the
  others.
- Run `node scripts/manual-diagrams.mjs`, open the SVG with Read to check it, and commit
  the script and the SVGs together.
- Put it in the chapter as a plain `<figure>` (no `screenshot` class), with the SVG's
  `width` and `height` on the `<img>` and alt text that names what's in cyan.

### Check and open it

```sh
npm run build && npm test && npx playwright test tests/e2e/manual.spec.ts
```

If Playwright's browsers aren't installed, run `npx playwright install chromium` first.
Run `npm run dev` and look at the chapter (at desktop and phone width) if the layout
might have changed.

Then commit, push, and open the pull request against `prerelease`:

```sh
gh pr create --repo Panic-Signals/panic-signals-site --base prerelease \
  --head manual/<deckard-branch-name> \
  --title "DECKARD manual: <feature>" --body-file <file>
```

Never open it against `main`.

The body says what was added and where, lists the screenshots and diagrams, and asks for
it to be merged once the DECKARD pull request is merged (or released, if it isn't
marked as a preview). Open the manual pull request **before** the DECKARD one, then:

1. Put the manual pull request's link under **Manual** in the DECKARD pull request
   description: `Manual: Panic-Signals/panic-signals-site#<n>`, with one line on what
   it documents.
2. Once the DECKARD pull request exists, add its link to the manual pull request:
   `gh pr edit <n> --repo Panic-Signals/panic-signals-site --body-file <file>`.

## Before opening it

- [ ] `cargo build` and `cargo test --workspace` pass.
- [ ] Tests run at least 70% of the lines you added or changed. CI's **Patch
      coverage** check enforces this (DEVELOPMENT.md, "Patch coverage" has the commands
      to check locally). Lines covered only by tests that need a Maschine install don't
      count on CI, so give new code a test that runs without one.
- [ ] `cargo fmt` has been run.
- [ ] Every section of the template is filled in, or says "None".
- [ ] If the change needs the manual (section 5), the manual pull request is open and
      linked under **Manual**, or the user chose "Later" and it says what's to do.
- [ ] The title is the summary in a few words, written like the release notes
      ("Load Groups without freezing the UI"), not "fix bug" or a branch name.

Create the pull request with `gh pr create --body-file <file>` using the template's
sections, or paste them into GitHub's form.
