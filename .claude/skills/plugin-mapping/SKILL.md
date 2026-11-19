---
name: plugin-mapping
description: Use when writing, fixing or checking a DECKARD plug-in mapping (the Lua file that gives a plug-in its MK3 knob pages and pictures) - finds the plug-in with plugin-probe, dumps its parameters, works out what each one is, measures real units the plug-in's text doesn't give (cutoff in Hz, envelope times in ms), writes the mapping, renders its pages with scripts/mapping-check.sh and fixes what looks wrong, then installs it in the user's controller folder or prepares it as a shipped mapping for a pull request.
---

# Writing a plug-in mapping

A mapping (`docs/controller-api.md`, "Plug-in mappings"; format in
`crates/surface/src/widget/mapping.rs`) is a Lua table of data: the plug-in's
parameters with real units, the pictures they draw (filter curve, envelope, EQ...), and
knob pages. Work from the plug-in itself, through `plugin-probe`, and say plainly what
you're sure of and what you only guessed.

**The black-box rule.** Learn about a plug-in only by hosting it: its parameter list,
the text it gives for each value, and the sound it makes. Never read its files, code,
presets' private data or artwork, never take it apart, and never copy a vendor's
logos, panel images or fonts. This holds for every plug-in, Native Instruments'
included.

Build the probe once (in a worktree, with the shared target dir the workspace rules
name):

```sh
cargo build -p deckard-plugins --bin plugin-probe
PROBE=<target dir>/debug/plugin-probe     # .exe on Windows
```

It hosts the plug-in in its own process with a time limit (`--timeout <s>`, default
60), so a crash takes only the probe with it. Never run it against a plug-in a running
DECKARD is using from the same files mid-edit, and never stop a running DECKARD to free
anything.

## 1. Find the plug-in

```sh
$PROBE list --out plugins.json
```

Find its `identity` (the id mappings and the probe accept) and name. Stop if a mapping
already matches it: the user's `<settings>/controller/mappings/` (or the `mappings/` of the
scripts folder they run DECKARD with) or DECKARD's `crates/surface/lua/mappings/`,
unless you were asked to replace it.

A product inside a host plug-in (Reaktor's Monark, a Kontakt instrument) needs its
state: pass `--state <file.nksf | .ens | .nrkt>` to `dump` and `measure`, the product's
own file from the user's library. The mapping then matches the product's name
(`match = { name = "Monark" }`): the slot carries it.

## 2. Dump it

```sh
$PROBE dump <identity or name> [--state <file>] --out <plugin>-dump.json
```

Every parameter: id, title, units, steps, default, whether automapping shows it, and
its text at 64 values. Read it for:

- **What the plug-in is** (`kind`: synth, filter, eq, dynamics, delay, reverb...).
- **Which parameter plays which role**: titles and their text (`"1.20 kHz"` is a
  frequency; three or four values cycling through names are a choice). Ids beyond the
  real controls are often placeholders (`{disabled}`, MIDI CC proxies): leave them out.
- **Units**: where a role's text parses as Hz, kHz, ms, s or dB at both ends, the mapping
  needs no taper. Where it reads bare numbers (`0.0`-`10.0`, `0`-`100`: a vintage
  panel's dial), measure it (step 3) or leave the picture out. Never guess silently.
- **Choices**: copy each choice's text exactly (`choices = { "BP", "LP1", "LP2", "MM" }`).

## 3. Measure what the text doesn't say

`plugin-probe measure` plays the plug-in and times or analyses its output while it
sweeps one knob:

```sh
cp .claude/skills/plugin-mapping/recipes/subtractive-synth.json <plugin>-recipe.json
# fill in the ids and values from the dump; delete what the synth hasn't got
$PROBE measure <identity> [--state <file>] --recipe <plugin>-recipe.json --out <plugin>-measured.json --timeout 1500
```

| Kind | What it measures | Set the recipe up so |
|---|---|---|
| `cutoff` | Hz where the output's spectrum peaks | noise (oscillators off) goes through the filter, resonance high, key tracking and envelope amount 0 |
| `attack` | ms to 90% of the level the note holds with the knob at 0 (40000: 40 s or more) | one plain oscillator, filter open, decay and sustain at full |
| `decay` | ms from the peak to 10% of it | attack 0, sustain 0 |
| `bright_attack`, `bright_decay` | the same on the energy above about 2 kHz | noise through a closed filter, the envelope amount at full: a filter envelope's times, as a cross-check (brightness follows the cutoff on a log scale, so decays read short) |

Every step prints its points as it goes and writes a `taper` for each:
`{ "table", { 0.0, 27.0 }, ... }`. Before using a table, read the points:

- They should rise steadily. A point that jumps back (a long attack reading a few ms, a
  cutoff falling at the top) is a measuring fault, not the plug-in: the note's start
  click, a level drifting between notes, a peak beyond the audio range. Leave those
  points out, re-measure that range, or use another kind's points (a fast attack's
  first points from `decay`-style peak timing), and say which you did.
- Attacks under about 20 ms are below what `attack` resolves (it skips each note's
  first 20 ms for its click).
- A cutoff whose peak leaves 20 kHz: end the table at the last good point and
  `{ 1, 20000 }`.
- Two envelopes that are one circuit on the synth (a Minimoog's) can share the amp
  envelope's tables; check with `bright_attack` that they agree.

Keep the recipe and the measurements beside the dump; the mapping's comment names
them.

## 4. Lay out the pages

- **Most played first**: for a synth, the filter and its envelope, then the amp; then
  oscillators, mixer, modulation, the rest. Every real control on some page, eight a
  page, a page's knobs from one section where they fit.
- **A picture's parameters on the knobs under it**, on one screen: knobs 1-4 or 5-8,
  two to four of them. Never across the gap between 4 and 5.
- `left = "picture"` (knobs 1-4) / `right =` (5-8), or
  `pictures = { { "name", knobs = { a, b } } }`.
- Small choices can stay on knobs: a stepped knob shows its choice as text.

## 5. Write the mapping

`mappings/<vendor>/<plugin>.lua` (lower case, hyphens). Start from the format comment
in `crates/surface/src/widget/mapping.rs` and the worked example in
`docs/controller-api.md`. For each parameter, a comment saying how its role and units
were found ("text in Hz", "measured: <plugin>-measured.json", "guessed from the title").
Bump `mapping_version` when you change a mapping someone already has.

## 6. Check it and look at it

```sh
scripts/mapping-check.sh <mapping.lua> <plugin>-dump.json <out folder>
```

It lists problems (ids the plug-in hasn't got, choices its text never shows, choice
maps naming a choice that isn't one, picture inputs with neither a taper nor units in
their text) and renders every page at the knobs' defaults, at 0 and at 1, on the default
and `lcd` themes. Fix every problem, then **look at the PNGs** and fix what's wrong: an
empty knob, a picture off its plot or flat, a label that doesn't fit, a picture over the
wrong knobs. Re-run until clean.

## 7. Install it or ship it

- **The user's own** (any plug-in): copy it into
  their controller folder's `mappings/<vendor>/` (Controller ▸ Open Controller Scripts
  shows it). DECKARD reloads it when it changes; a mapping that fails shows its
  error on the MK3 and is left out.
- **Shipped with DECKARD** (plug-ins many people play, with stable parameter ids): put it in
  `crates/surface/lua/mappings/<vendor>/<plugin>.lua`, add it to `SHIPPED` in
  `crates/surface/src/sources.rs`, and its dump, cut down to the parameters the mapping
  names (by id or title), to `crates/surface/tests/mappings/<plugin>.json` (the test
  `shipped_mappings_check_against_their_dumps` checks it in CI). No vendor artwork.
  Then the `pull-request` skill: the PR's test steps say which plug-in version the dump
  is from and attach a few of the rendered pages.

## 8. Report

What the mapping covers (pages, pictures), which roles and units are sure (text,
measured) and which guessed, anything measured that looked wrong and what you did about
it, and where the dump, recipe, measurements and previews are.
