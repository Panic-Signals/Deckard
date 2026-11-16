# Plug-in mappings

A plug-in can come with a small Lua file, its **mapping**. It holds two things, both data:

- **Declarations:** what the plug-in is and what its parameters mean. This one is the
  cutoff, in Hz on a log taper; that one is the filter mode, with these four choices.
- **Mappings:** where those parameters go on the MK3. Knob pages eight at a time, choices
  on the display buttons, and which pictures sit above which knobs.

It's how Ableton's Push works with Live's devices: each device's parameters are declared
and banked eight at a time, and Push draws the device's picture (EQ Eight's curve, Auto
Filter's response) from them with its own code. Here too, DECKARD's Rust code does all the
drawing: a filter's curve moving, an EQ's bands, a compressor's transfer curve. A mapping
only says what to draw from.

Design only: nothing here is built. No project-format change: mappings are files beside
the controller scripts, and nothing about them is saved in a project. **Nothing is stored
per preset either:** a mapping works from the slot's knob assignments as they are at that
moment (see "Following the knobs").

The scope is kept to what pays for itself: pictures drawn from parameters, touch focus,
and mappings that follow the knobs. Measuring plug-ins, most synth-specific pictures and
install tooling are in "Later", each with the reason it waits.

Items marked *(builder's call made in design)* were chosen while writing this and were not
asked for directly. The user reviews them when reading this document.

## Why

Today the Control page shows a hosted plug-in as eight knob names and values. DECKARD's
own Filter, EQ and a few other effects get a curve on the right screen, each as a special
case in the app (`ControlView::eq`, `filter`, `picture`) and in `control.lua`'s
`eq_curve`. Hardware like Ableton's Push 3 shows that a screen over the knobs is most
useful when it shows what the sound is doing: the filter's curve under the cutoff knob, a
band's dot under its frequency knob, the envelope's shape under its times.

Goals:

- Any plug-in, built-in or hosted, can have a visual face, from one file a user can write.
- NKS plug-ins get pictures with no mapping at all, from the knob names their presets
  already carry.
- It reads at a glance and responds the moment a hand moves, on every theme, including the
  two-tone `lcd` one.
- Mappings stay declarative. Drawing and timing are Rust's, so a mapping can't make the
  screens slow or ugly, and nothing runs Lua per frame.
- One path: DECKARD's own curves become shipped mappings.

## The short version

- **A mapping is a Lua table of declarations and mappings,** like a Push device bank. It
  declares what the plug-in is (`kind = "filter"`) and the parameters it uses, with names
  and units of its own, then maps them: a widget for each screen with its inputs bound to
  those parameters, knob pages and button choices.
- **Rust draws.** A small set of widgets (filter curve, EQ, envelope, LFO, transfer curve,
  delay taps, reverb decay, knob cells…) is drawn on the MK3 screens thread. Every widget
  has a flat look made from the theme's colours; glow and soft fills are extras a theme can
  turn on.
- **Pictures span the knobs they draw.** A picture sits directly above the knobs whose
  parameters it shows, two to four knob cells wide: a filter curve over Cutoff and Reso,
  an envelope over Attack to Release beside it on the same screen. The two screens have a
  physical gap between them, so no picture crosses it.
- **Following the knobs.** The slot's pages, whatever filled them (an NKS preset, the
  user, automapping), are the one source of knob assignments. Pictures find their inputs
  on the knobs as they are now, so they change when the pages change and never disagree
  with what the knobs do. NKS and user pages keep their own order and still get the
  pictures; a mapping's own pages, in its order, apply when the slot has neither, or when
  the user ticks an override for that plug-in in the preferences.
- **Real units from the plug-in's own text.** Most plug-ins already say "1.20 kHz" or
  "-6.0 dB"; DECKARD reads that to learn each parameter's taper, so few mappings need to
  declare one.
- **Touch focus.** A resting finger on a knob lights the part of its screen's picture it
  moves and puts its value large in the header.
- **Modulation is visible.** Pictures and knob cells show where the knob is and where
  modulation has moved it.
- **Shipped and user mappings.** DECKARD ships mappings for its own plug-ins and a few
  open-source ones. Users add their own to a folder, and theirs win.
- **A parameter dump and a skill** make writing one quick: `plugin-probe dump` lists a
  plug-in's parameters and their text, and the `plugin-mapping` Claude Code skill drafts a
  mapping from it.

## What DECKARD already gives, and the gaps

| Need | DECKARD today | Gap |
|---|---|---|
| Draw on both screens | `crates/surface/src/screen.rs`: `draw(screen, theme)` in Lua at each redraw, 960 × 272, themes with named colours and styles. Frames go to the MK3 on their own thread; `deckard_mk3::Displays::show` sends every row from the first changed to the last, at full width, and waits for each transfer | Immediate mode in Lua on the UI thread: right for pages, wrong for moving curves. A small moving curve resends most of a screen. The display protocol's header already carries a rectangle (x, y, width, height) and a skip command (`02`), so sending only what changed needs no new protocol |
| Curves for built-ins | `EqCurveView` (`points`, `bands`, `extent`, `lfo`, `side`, `kind`), drawn by `control.lua` | Computed per effect in the app. Hosted plug-ins get nothing |
| Plug-in parameters | `KnobView` (name, section, text, 0..1 value); `Params::value`, `Params::text`; `ParamInfo` (id, title, short title, units string, steps, default); `layout::Edit::PluginIn` | Values arrive normalised: a curve needs Hz, dB and ms, and a hosted plug-in's taper is unknown. `ParamInfo` has no flags and no section (VST3 unit), and the bypass and program-change parameters are filtered out before anything sees them |
| Knob assignments | The slot's pages, saved with it (`PluginRef::pages`: each knob's parameter id, section and name). Loading an NKS preset through the browser writes its pages there; the user can edit them; when there are none, automapping (the first 128 automatable parameters, eight to a page) | Enough: pictures resolve against it ("Following the knobs") |
| Modulation | `mod_rt.rs` publishes route travels (`meters::RouteTravels`) for each sound's routes; `App::mod_position` turns one into a knob's modulated position | Sounds only: a plug-in on a group or the master shows no modulation until route travels cover those chains |
| Knob touch | The MK3's knobs are touch sensitive, and report 0x01's bits 72–79 already arrive as `BUTTON_CAP_1`…`BUTTON_CAP_8` (`deckard_mk3::BUTTONS`). Pages use them for Set Macro and double taps | Nothing reads them for display. The mapping layer watches them too ("Touch focus") |
| Sandbox, hot reload, user folder | Safe stdlib only, `<settings>/controller` replaces built-in files by path, reload on save | Reused as is. There is no time limit on scripts, so mappings have no code that runs ("What a mapping says") |
| Screen tests | `screen.rs`'s `render_screens` test draws pages without a device | No golden-image comparison yet. Widgets need one (phase 1) |

## Where mappings live and how one is found

```
crates/surface/lua/mappings/         shipped with DECKARD (built into the binary)
  builtin/filter.lua  builtin/eq.lua  builtin/compressor.lua  ...
  surge-synth-team/surge-xt.lua  vital-audio/vital.lua  ...
<settings>/controller/mappings/      the user's
  <vendor>/<plugin>.lua
  <vendor>/<plugin>/                 the mapping's pictures, if it has any
```

Where a mapping sits doesn't decide what it matches; its `match` does. The paths are only
tidy.

### Matching

A mapping's `match` lists what it's for:

```lua
match = {
  vst3 = "ABCDEF0123456789ABCDEF0123456789",     -- VST3 class ID
  au   = { "aufx", "lpf1", "Acme" },              -- Audio Unit type, subtype, manufacturer
  vendor = "Acme", name = "Ladder",               -- fallback for either format
  version = ">=1.4",                              -- optional
}
```

A plug-in matches on its identity in its own format (the VST3 class ID or the Audio
Unit's three codes), else on vendor and name. A built-in matches on its key
(`match = { builtin = "filter" }`), the key `plugin_pictures.rs` already uses. Parameter
ids differ between a plug-in's VST3 and Audio Unit versions, so a parameter can give both
(`id = { vst3 = 3, au = 1003 }`, "Parameters"), and one mapping serves both formats.

No match: the `generic` template ("Templates"), which is today's Control page plus
pictures where it recognises the knobs. A mapping that fails to load or check falls back
the same way, shows a small `MAP` badge in the header, and its error, with its line, goes
to the script-error report scripts already have.

Mappings reload when saved, as controller scripts do, and `defaults/mappings/` is
refreshed with the other defaults when the folder is opened.

## Distribution

### What DECKARD ships

- **Every DECKARD plug-in:** its effects, instruments and drum synths. These replace
  today's special-case curves. A DECKARD plug-in with no mapping yet gets `generic`.
- **A few third-party plug-ins,** chosen so anyone can check them: **open source, free,
  available on macOS, Linux and Windows, widely used, and with stable parameter IDs.** The
  first set *(builder's call made in design)*: Surge XT, Vital, Dexed and OB-Xd. More can
  be added on the same terms, as a pull request. Mappings for commercial plug-ins are for
  users to write and share themselves, not to ship.
- **No vendor artwork.** A shipped mapping uses only parameter IDs, names and units, never
  a vendor's logos, panel images or fonts.

**Tests without the plug-in.** Each shipped third-party mapping comes with a **parameter
dump** (`crates/surface/tests/mappings/<plugin>.json`, made by `plugin-probe dump`): the
plug-in's version, every parameter's id, name and range, and its text at 64 points. CI
loads the mapping against the dump, checks that every name resolves and every calibration
parses, and renders its golden images. So a mapping's pull request can be checked on any
machine. Dumps are made only by hosting the plug-in itself, never from NKS presets or other
files of a commercial library.

### How users add their own

- **Write one:** **Controller ▸ Open Controller Scripts Folder**, then `mappings/`. The
  shipped mappings are in `defaults/mappings/` to copy from, as pages are today.
- **Start from the plug-in:** **Controller ▸ Dump Plug-in Parameters** writes the focused
  plug-in's parameter dump and a starter mapping with every parameter listed and the
  `generic` template, into `mappings/`. Most of a mapping is then naming parameters and
  picking a template.
- **Share one** by sharing the file (and its picture folder, if it has one). The receiver
  puts it in `mappings/`.
- **Safe to share.** A mapping is a table of data with no code that runs, loaded with the
  same safe Lua as controller scripts: no file, process or network access. Pictures are
  PNG only, at most 480 × 272 and 512 KB each. A shared mapping can draw on the screens and
  move the plug-in's parameters, nothing else.

### Which mapping is used

1. **The user's folder wins over the shipped mappings.** A user who wrote or copied a
   mapping meant it.
2. Within one place, the most specific match wins: the plug-in's identity, then vendor and
   name.

**Preferences ▸ Hardware ▸ Mappings** lists the plug-ins that have a mapping, with two
settings each, both preferences rather than part of any project:

- **Use the mapping's pages** ("Following the knobs").
- **Off:** keep the plain Control page for that plug-in.

### Versions

- **`api`:** each mapping says which version of the mapping format it's written for
  (`api = 1`). DECKARD loads mappings written for its own version or older, and refuses a
  newer one with an error naming the DECKARD version it needs. New widgets and fields only
  ever add to the format, so an old mapping keeps working.
- **`mapping_version`** and **`author`** are for people: they show in the script-error
  report and the preferences.
- **Plug-in updates:** `match.version` limits a mapping to the plug-in versions it was
  written for. If an update renames or removes a parameter, its knob shows `?` and the
  script-error report names it; the rest of the mapping keeps working.

## Writing mappings quickly

**The black-box rule.** Everything here learns about a plug-in only by hosting it: its
parameter list and the text it gives for each value. Nothing reads a plug-in's files,
code, presets' private data or artwork, takes one apart, or looks at how any vendor's
software works inside. This applies to every plug-in, Native Instruments' included.

### The probe

`plugin-probe`, a binary in `crates/plugins` (`src/bin/plugin-probe.rs`). It hosts one
plug-in **in its own process**, with a time limit, and prints JSON. It never opens a
project or touches DECKARD's settings, so a plug-in that crashes takes only the probe with
it.

| Command | What it gives |
|---|---|
| `plugin-probe list` | The installed plug-ins DECKARD can host, with their identities (VST3 class ID, AU type/subtype/manufacturer), vendor, name and version |
| `plugin-probe dump <plugin>` | The **parameter dump**: every parameter's id, name, units string, steps, default, flags (automatable, bypass, program change) and section (VST3 unit) where the plug-in reports them, and its text at 64 points. The probe reads flags and sections itself; DECKARD's `ParamInfo` doesn't carry them yet. This is the same JSON the tests and **Dump Plug-in Parameters** use |

### The skill

`.claude/skills/plugin-mapping/SKILL.md`, beside the existing `pull-request` skill. It
works from the dump: names, units, sections and value text. Its guesses are best-effort,
and it says which roles it's sure of and which it only guessed. Given a plug-in name, it:

1. **Finds the plug-in** with `plugin-probe list`, and stops if a shipped or user mapping
   already matches (unless asked to replace it).
2. **Dumps it,** and reads the names, units and sections: the `kind`, which parameter is
   which role, how they group into pages.
3. **Checks the units.** Where a role's text parses ("1.20 kHz" at one end, "20.0 kHz" at
   the other), it uses `calibrate`; where it doesn't, it says so and leaves the picture
   out rather than guess a taper.
4. **Lays out the pages:** the most-used parameters first, a picture's parameters on the
   knobs under its screen, sections kept together, display buttons 4 and 7 for a small
   choice.
5. **Writes the mapping,** with a comment on each parameter saying how its role was
   chosen, and the dump as its test data.
6. **Renders the previews** (default values, extremes, touch focus on each knob), looks at
   them, and fixes what's wrong: an empty knob, a curve off the plot, a label that doesn't
   fit.
7. **Reports** what it's unsure of. Then, as the user asks, it puts the mapping in their
   `mappings/` folder or prepares it for a pull request as a shipped mapping (the
   `pull-request` skill takes it from there).

## What a mapping says

A mapping returns one table of data. It has no functions: no hooks, no drawing code. What
can't be said with the fields below needs a new widget or field in Rust, which keeps every
mapping fast and consistent.

```lua
-- mappings/acme/ladder.lua (illustrative)
return {
  api    = 1,                        -- the mapping format it's written for
  mapping_version = 3, author = "A. Name",
  match  = { vendor = "Acme", name = "Ladder" },
  kind   = "filter",                 -- picks the template (pictures, focus behaviour)

  params = {
    cutoff = { id = 3, knob = { "Cutoff", "Freq" }, unit = "hz", taper = { "log", 20, 20000 } },
    reso   = { id = 4, knob = { "Resonance", "Reso" }, taper = { "lin", 0, 1 } },
    drive  = { id = 5, unit = "db", calibrate = true },   -- learn the taper from the plug-in's text
    mode   = { id = 6, choices = { "LP24", "LP12", "BP", "HP" } },
    env    = { id = 9, taper = { "lin", -1, 1 } },
    attack = { id = 10, unit = "ms", calibrate = true },
    decay  = { id = 11, unit = "ms", calibrate = true },
    mix    = { name = "Mix" },                            -- by title when there's no stable id
  },

  pictures = {
    response = { widget = "filter", needs = { "cutoff" },
                 model = { shape = { from = "mode", map = { LP24 = "ladder4", LP12 = "svf_lp",
                                                            BP = "svf_bp", HP = "svf_hp" } },
                           cutoff = "cutoff", resonance = "reso", drive = "drive" } },
    motion   = { widget = "motion", needs = { "cutoff" }, param = "cutoff" },
    contour  = { widget = "envelope", needs = { "attack", "decay" },
                 model = { a = "attack", d = "decay" }, amount = "env" },
  },

  pages = {
    { name = "FILTER", left = "response", right = "motion",
      knobs = { "cutoff", "reso", "drive", "mode", "env", false, false, "mix" } },
    { name = "ENV",    left = "contour",  right = "response",
      knobs = { "attack", "decay", "env", false, "cutoff", "reso", false, "mix" } },
  },

  buttons = {                        -- only the display buttons the Control page leaves free ("Controls")
    [4] = { step = "mode" },         -- each press, the next filter mode; its label shows the current one
  },
}
```

### Parameters

A mapping names the parameters it uses once, under its own names, and the rest of the file
uses only those. A name used anywhere else in the mapping that isn't declared under
`params` is an error. Each parameter can be found three ways, tried in this order:

- **`knob`**: the names a knob carries on the slot's pages, optionally with
  `section = "Filter"`. This is how a mapping follows NKS pages and the user's own pages
  ("Following the knobs").
- **`id`**: a VST3 ParamID, an Audio Unit parameter ID, or a built-in field; one number,
  or `{ vst3 = …, au = … }` for a plug-in in both formats.
- **`name`**: the parameter's title, for plug-ins with no stable ids.

Parameters are resolved again whenever the slot's pages change or the plug-in reloads, and
the result is kept only in memory. A parameter that doesn't resolve leaves its widget
input at its default and its knob cell showing `?`. Nothing is guessed.

Every write goes through `layout::Edit::PluginIn` or `Edit::Param`, as a knob turn does
today, so Auto-write, Lock, macros and undo all see it. A mapping adds no write path.

### Real units: calibration or taper

A curve needs 1.2 kHz, not 0.43. Two ways:

- **`calibrate = true`:** when the parameter resolves, DECKARD reads its `Params::text` at
  64 normalised points, spread over UI frames, and parses the numbers and units
  (`"1.20 kHz"`, `"-6.0 dB"`, `"120 ms"`, `"1/8 D"`). The table is kept in memory with the
  binding and read again when the binding changes. It isn't written anywhere. If the text
  doesn't parse, the mapping is reported and must declare a taper. This covers most
  plug-ins, which is why measuring them can wait ("Later").
- **A declared taper:** `{ "log", 20, 20000 }`, `{ "lin", a, b }`, `{ "exp", a, b, curve
  }`, `{ "db", "-inf", 12 }` (minus infinity written as a string, since Lua has no literal
  for it), or `{ "table", {0, 20}, {0.5, 800}, {1, 20000} }`.

Until a calibration is done (a few frames), a picture that needs it uses its parameter's
declared taper if it has one as well, and otherwise shows the plain knob cells on that
screen. It never draws a wrong or flat curve while waiting.

The knob cell always shows the plug-in's own text, so a wrong taper bends the picture,
never the number. The text is the knob's base value; the modulated position shows on the
cell's bar ("Modulation").

### Widget inputs

A widget input is a parameter's name, a constant, or a choice map:

```lua
shape = { from = "mode", map = { LP24 = "ladder4", LP12 = "svf_lp" } }
```

That's all the first version needs: a filter's mode picking its shape is the common case.
Arithmetic on inputs (an envelope amount added to the cutoff, say) is "Later", for when a
real mapping needs it.

### Pictures and pages

`pictures` names the pictures a mapping can show. Each has a widget, its inputs, and
`needs`: the inputs it can't do without.

**A picture spans knobs.** The picture band above the knobs is split into spans of whole
knob cells (120 px each), and each span shows one picture, directly above the knobs whose
parameters it draws. A span is two to four cells wide on one screen, never across the gap
between knobs 4 and 5. Knobs no picture covers keep their plain cells, with the band above
them left empty.

- **A mapping's own pages** place pictures by knob range, or by screen as shorthand:

  ```lua
  { name = "FILTER", knobs = { "cutoff", "reso", "attack", "decay", "env", false, false, "mix" },
    pictures = { { "response", knobs = { 1, 2 } }, { "contour", knobs = { 3, 4 } },
                 { "motion", knobs = { 5, 8 } } } },
  { name = "ENV", left = "contour", right = "response", knobs = { ... } },  -- left = knobs 1-4, right = 5-8
  ```

  An empty knob is `false`, since a `nil` would end the Lua list. A mapping's pages are
  shown when the slot has no pages of its own, or the user asks for them ("Following the
  knobs").
- **The slot's pages** (an NKS preset's, the user's, automapping) say nothing about
  pictures, so the layout finds them from the knobs ("Laying out pictures on any page").

That's the only way to place a picture. Templates fill `pictures` and `pages` for a
`kind`, and a mapping can override any of it.

### Laying out pictures on any page

For a page that doesn't place its own pictures, each screen's four knobs are laid out on
their own:

1. **Find candidates.** Each of the mapping's pictures, and each of `generic`'s patterns,
   is a candidate wherever its `needs` sit on **adjacent knobs** of one screen. Its span
   is the run from its first to its last bound knob, widened to the widget's minimum width
   ("Widgets") where the screen has room.
2. **Pick.** Candidates that bind more knobs win, then the mapping's own pictures over
   `generic`'s, then the leftmost. Picked spans can't overlap. *(builder's call made in
   design)*
3. **Across the gap.** A pattern split over knobs 4 and 5 (Cutoff on 4, Reso on 5) gets
   its picture on the screen that holds its first input (here the cutoff), if that screen
   has room for the widget's minimum width; the knob on the other screen still lights it
   on touch. Otherwise both knobs keep their cells.
4. **Nothing found:** the screen keeps the plain cells, as today.

The layout runs when the page or the slot's pages change, not per frame, so a picture never
jumps while a knob turns.

### Templates

| `kind` | Left screen | Right screen |
|---|---|---|
| `filter` | response curve | what is moving the cutoff: its base and modulated position over the last few seconds |
| `eq` | the whole curve with its bands; the encoder picks a band | the chosen band close up: its frequency, gain and Q large, its own curve |
| `dynamics` | transfer curve | threshold, ratio and makeup large |
| `delay` | the repeats as a timeline, left and right lanes | the feedback path: how fast repeats die |
| `reverb` | the decay: level over time, pre-delay as a gap before it | damping: the tail's tone as a curve over frequency |
| `synth` | per section ("Synths") | per section |
| `drum` | the voice's shape (pitch and amplitude envelopes) | its main parameters large |
| `generic` | knob cells, plus a picture where it recognises the knobs | the same |

**`generic` recognises common patterns** of adjacent knob names on the page it shows
*(builder's call made in design)*, and gives each a picture spanning those knobs:

| Pattern (adjacent knobs) | Picture |
|---|---|
| a cutoff ("Cutoff", "Freq", "Frequency") next to a resonance ("Resonance", "Reso", "Res", "Q"), optionally with a drive or envelope amount beside them | `filter` |
| attack, decay, sustain and release in order, or attack and decay, or attack and release | `envelope` |
| a band's frequency, gain and Q | `band` |
| threshold next to ratio | `transfer` |
| an LFO's rate next to its depth or shape | `lfo` |

Names match ignoring case, with the knobs' section as context (two "Attack" knobs in
"Filter Env" and "Amp Env" are two envelopes). Each picture needs its parameters to
calibrate from their text, since `generic` declares no tapers; where they don't, the cells
stay. This is what gives an NKS plug-in with no mapping of its own pictures on the pages
its presets map, which is most of the value for the least work. NKS pages often mix
sections on one screen, which is why pictures span knobs rather than whole screens.

### Widgets

Every widget takes bound inputs and its colours and type from the theme, and has a **flat
look first**: lines and solid fills in the theme's colours, which reads on every theme,
the two-tone `lcd` one included. Glow, soft fills and accent tints are a theme's extras
(`themes/maschine.lua` turns them on), never needed to read a picture. Each widget draws
at most three layers at once.

Every picture widget draws at **every width it can span**: two, three or four knob cells
(240, 360 or 480 px) by the 172 px picture band, each with its own golden image. Narrower
spans drop secondary detail (axis labels thin out), never the picture itself. The minimum
width, in knob cells, is in brackets below; one cell is too narrow for a curve to read, so
only `value` and `image` go that small.

| Widget | Inputs | How it reads |
|---|---|---|
| `filter` (2) | shape, cutoff, resonance, drive, slope, mix | Analytic response for SVF and ladder shapes, written for the widget (the engine's own filters are biquads). The curve with a flat fill under it, and the modulated curve over it while modulated |
| `motion` (2) | a parameter | Its base value as a line and its modulated value as a second line over the last few seconds, newest on the right |
| `eq` (3) | bands: frequency, gain, Q, type, on | Each band a dot sized by Q, the sum curve; a band's own curve only while its knob is touched |
| `band` (2) | one band's parameters | Large numbers and the band's own curve |
| `envelope` (2) | ADSR, DAHDSR or breakpoints, curvature, amount | The shape, with its times on the axis |
| `lfo` (2) | shape, rate, phase, depth, sync | One cycle of the shape, the rate under it |
| `oscillators` (2) | up to four sources: waveform, octave, tune, pulse width, level; noise level | Each source's cycle faint, their sum bright |
| `transfer` (2) | threshold, ratio, knee, makeup | The curve, the threshold marked |
| `taps` (2) | time, feedback, spread, sync | Repeats as decaying bars |
| `decay` (2) | pre-delay, size, decay | Level over time on a dB scale |
| `damping` (2) | damping, tone | A curve over frequency |
| `value` (1) | one parameter | Large numerals, the unit small |
| `knobs` | the page's knobs | The cells over the knobs. Always drawn ("Knob cells") |
| `image` (1) | a PNG beside the mapping | For a user's own mappings. Shipped mappings have none |

Adding a widget is a Rust change with a golden-image test. That's on purpose: it keeps the
look consistent.

## The screens

The MK3 has four knobs under each screen and four display buttons above it. So each screen
has three bands, lined up with the hardware. Here a filter page from a mapping, one picture
full width over each screen's four knobs:

```
 ┌─────────── left screen 480 ───────────┐   ┌─────────── right screen 480 ──────────┐
 │ MASTER   GROUP   [SOUND]   [LP24]      │   │ ◀ SLOT   SLOT ▶                       │  28  buttons
 │ LADDER · FILTER        CUTOFF 1.20 kHz │   │ CUTOFF MOTION              MAP 1/2    │
 │ ──────────────────╮                    │   │ ────────╮      ╭──────╮      ╭──      │
 │                   │╲                   │   │          ╰────╯        ╰────╯   mod    │ 172  picture
 │                   │ ╲                  │   │ ─────────────────────────────── base   │
 │ ████████████████████╲____              │   │                                       │
 │ 20     100     1k      10k             │   │                          now ┘         │
 ├─────────┬─────────┬─────────┬─────────┤   ├─────────┬─────────┬─────────┬─────────┤
 │ FILTER                                │   │ FILTER          │         │ OUTPUT    │  72  knob cells
 │ CUTOFF  │ RESO    │ DRIVE   │ MODE    │   │ ENV     │         │         │ MIX       │
 │ ▰▰▰▰▰▱▱ │ ▰▰▰▱▱▱▱ │ ▰▰▱▱▱▱▱ │ LP24    │   │ ▱▱▱▰▰▱▱ │         │         │ ▰▰▰▰▰▰▰   │
 │ 1.20 k  │ 42 %    │ 3.0 dB  │         │   │ +18 %   │         │         │ 100 %     │
 └─────────┴─────────┴─────────┴─────────┘   └─────────┴─────────┴─────────┴─────────┘
```

- **Nothing crosses the gap.** Each screen is its own 480 × 272 picture.
- **A picture sits above the knobs that move it,** two to four cells wide, as Push 3
  groups a device's sections. On a page that mixes sections, one screen holds two
  pictures:

```
 ┌─────────── left screen 480 ───────────┐
 │ MASTER   GROUP   [SOUND]               │  28  buttons
 │ SYNTH · PAGE 3          RESO 42 %      │
 │ ────────────╮      │    ╱╲             │
 │             │╲     │   ╱  ╲______      │ 172  two pictures: filter over knobs 1-2,
 │ ████████████│ ╲    │  ╱          ╲     │      envelope over knobs 3-4
 │ 20   1k   10k      │ A    D   S   R    │
 ├─────────┬─────────┬─────────┬─────────┤
 │ FILTER            │ AMP ENV           │  72  knob cells
 │ CUTOFF  │ RESO    │ ATTACK  │ RELEASE │
 │ ▰▰▰▰▰▱▱ │ ▰▰▰▱▱▱▱ │ ▰▱▱▱▱▱▱ │ ▰▰▰▱▱▱▱ │
 │ 1.20 k  │ 42 %    │ 4 ms    │ 380 ms  │
 └─────────┴─────────┴─────────┴─────────┘
```
- **The header** has the page's name and source on the right (`MAP 1/2`, or the slot's
  page name as today), and the touched knob's value large ("Touch focus").
- **Button labels sit exactly over the buttons,** filled when chosen.

### Knob cells

- **Bars, not arcs,** as Push 3 draws them: a horizontal bar per knob (from the middle
  for bipolar parameters), 120 px wide, exactly over its knob. The modulated position is
  a second, thinner mark on the same bar. Stepped parameters show their choice as text
  instead of a bar.
- **Section headings stay,** as the Control page draws them now: a heading over the first
  knob of each section, running across the knobs that share it.
- **Long names are shortened in the middle** (`Filt…toff`), keeping the end that usually
  tells parameters apart.
- What the cells show today for Auto-write, Lock and the macros stays.

### Touch focus

Touching a knob, after the finger has rested for about 120 ms:

1. The bound part of that screen's picture lights: the cutoff gets a guide line down to
   the axis, the resonance peak shows its Q as a bracket, a band's dot becomes a ring and
   its own curve appears.
2. The rest of the picture dims to about 50 %. Names, values and the axis never dim.
3. The header shows the parameter's name and value large, where it covers nothing.
4. On letting go, it returns after 180 ms.

The other screen keeps its picture. The template says what each parameter lights; a
mapping can change it with `focus = { cutoff = "handle", reso = "peak" }`.

**How touch is read.** The MK3 already reports a finger on knob *n* as `BUTTON_CAP_n` down
and its lifting as up. The surface passes those events on to the page and `global.lua`
exactly as now, so Set Macro, double taps and anything a script does with them keep
working, and also tells the mapping layer, which only changes what's drawn. Focus starts
before the knob moves, so you see what a knob will do before turning it, and ends 180 ms
after the finger lifts, or, for a knob turned without a touch registering, 1 s after its
last turn.

- **One or two knobs** touched each light their part.
- **Three or more** touched at once is a resting hand: nothing lights until fewer remain.
- **While Set Macro is armed,** touch picks the knob as now, and touch focus is off.
- Touch focus can be turned off in **Preferences ▸ Hardware**.

### Motion

- **Pictures follow the hand.** A knob's normal steps draw at once. Only a jump of more
  than a tenth of the range (a preset change, a macro, a page of new values) eases, over
  about 60 ms, so it reads as a change rather than a flicker. Stepped parameters never
  ease.
- **Page and slot changes are instant.** A slide would resend every pixel of both
  screens for its whole length.
- **The group's colour is the accent** on themes that use one.
- **Flat fills, no gradients by default.** A fill is one colour (a dimmer step of the
  curve's), which reads on the `lcd` theme, doesn't band on the screens' 16-bit colour,
  and keeps the MK3's run-length encoding effective. A theme can ask for soft fills;
  they're drawn without dithering, which would break the encoding's runs.
- **Still when nothing moves.** With no modulation or touch, the frame doesn't change and
  nothing is sent.

### Getting frames to the screens

Each screen is 480 × 272 at 16 bits, 261 KB a frame uncompressed. Today a moving curve
anywhere on a screen resends every row from its top to its bottom, so a 60 fps picture on
both screens could need tens of MB/s, more than the MK3's link is known to sustain.

- **Measure first** (phase 0): the real rate of full-screen and partial transfers on an
  MK3, and how long each transfer blocks the screens thread.
- **Send only what changed:** drawing tracks dirty rectangles (the picture area, each knob
  cell, the header), and each is sent as its own transfer with its x, y, width and height,
  which the display protocol already supports.
- **30 fps for pictures** *(builder's call made in design)*, which a curve and two knob
  cells fit easily. Knob cells and touch focus are drawn on the next frame after a change,
  so the hand never waits more than 33 ms. If phase 0 shows more room, the rate goes up.

## Controls

The Control page already uses most of the buttons around the screens: display buttons 1–3
pick MASTER, GROUP or SOUND, 5 and 6 step through the slots, SHIFT turns them into the
slot's edits (EDIT, INSERT, MOVE, BYPASS, REMOVE) and the knobs into fine adjustment, LEFT
and RIGHT page, the jog wheel steps slots and opens the browser, and the Smart Strip has
its own modes. A mapping keeps all of that and takes no button away from it:

- **No mode button.** Nothing on the MK3 switches between NKS pages and the mapping's
  pages, or between button rows. Which pages the knobs show is a per-plug-in setting, made
  only in **Preferences ▸ Hardware ▸ Mappings** ("Following the knobs").
- **A mapping's `buttons` use only display buttons 4 and 7,** the two the Control page
  leaves free without SHIFT, for the plug-in's small choices: `choose` (one value),
  `step` (the next of a parameter's choices) or `toggle`. Anything else goes on a knob,
  where a stepped parameter shows its choice as text.
- **SHIFT means what it means today:** the slot's edits on the buttons, fine adjustment on
  the knobs. While SHIFT is held, the button labels change to the slot's edits, as now;
  the pictures stay.
- **LEFT and RIGHT always page** through one list of pages ("Following the knobs").
- **The jog wheel and the Smart Strip are the Control page's,** unchanged.

## Following the knobs

The slot's pages are the knob assignments DECKARD already keeps for every plug-in slot,
saved with the project as Maschine saves it: each knob's parameter id, section and name.
Something fills them, and that's all a mapping needs to know:

- **An NKS preset loaded through the browser** writes its own pages, eight named knobs a
  page by section. In some products (Kontakt libraries, Reaktor ensembles) what a
  parameter id means changes from one preset to the next, and the preset's pages are the
  only place that says what each knob now is.
- **The user** edits pages on the slot, as in Maschine.
- **Automapping** gives the first 128 automatable parameters, eight a page, when nothing
  else has.

A mapping stores nothing per preset. Its pictures work from the pages the knobs show as
they are at that moment:

- **Parameters resolve through the knobs first.** A parameter with `knob = { "Cutoff",
  "Filter Cutoff" }` resolves to whichever parameter the slot's pages put under a knob of
  that name (in that `section`, if given), then by `id`, then by title. It looks at the
  slot's own pages even while the mapping's pages are shown, so a mapping for a host
  plug-in can lay out its own pages from what an NKS preset's knobs are. When the slot's
  pages change (another preset loaded, the user's edit), everything resolves again.
- **Pictures follow the knobs.** On the slot's own pages, pictures are laid out over the
  runs of adjacent knobs that carry their inputs ("Laying out pictures on any page"), so
  a picture always sits above the knobs that move it. Knobs no picture fits keep their
  plain cells.
- **The knobs never disagree with the pictures,** because both come from the same pages.

**Which pages the knobs show.** A mapping's pages are knob pages that make sense for the
plug-in, written once in the mapping file. They apply in two cases:

- **The slot has no pages of its own** (no NKS preset was loaded, and the user hasn't
  edited any): the mapping's pages replace automapping.
- **The user ticks "Use the mapping's pages" for that plug-in** (one checkbox per
  plug-in) in **Preferences ▸ Hardware ▸ Mappings**: the mapping's pages replace the
  slot's own on every slot of that plug-in, NKS pages included. It's off by default
  *(builder's call made in design)*.

When the mapping's pages are shown, the mapping controls their order, and automapping's
remaining parameters follow them, eight a page by section where the plug-in reports
sections, so every parameter stays reachable. The header says `MAP` on them.

**Otherwise the slot's own pages are shown exactly as they are:** an NKS preset's pages in
the preset's order, or the user's in theirs, as today. The mapping adds no pages, removes
none and reorders none. It still gives them the visual treatment: pictures over the runs
of knobs they fit, touch focus lights what a knob moves, and the knob
cells, units and modulation are drawn as on the mapping's own pages. So an NKS plug-in
looks as good as one on its mapping's pages; what it gives up is only the mapping's
choice of what goes where.

Nothing about this is saved in the project. The checkbox is a preference; the slot's own
pages stay saved as they are, and come back when it's unticked. Editing a page on a slot
that shows the mapping's pages (with the existing page editor) saves the edited pages as
the slot's own, as any edit does today, and they're what that slot shows from then on
unless the checkbox is ticked.

**When the preset changes inside the plug-in** (chosen in its own window, say), DECKARD
can't see it in this version: it doesn't act on a plug-in's restart flags yet, and some
products report nothing. The knobs keep the slot's pages, and the pictures follow those
knobs, so the two still agree with each other. Acting on `kParamTitlesChanged` and
program changes is "Later".

## Modulation

- **Where the parameter really is:** route travels give each bound parameter's modulated
  position, as `App::mod_position` does for knobs today. A picture draws the base and the
  modulated position; `motion` shows the last few seconds of both.
- **Knob cells** show it as a second mark on the bar.
- **Chains:** route travels cover sounds' routes today, so a plug-in on a group or the
  master shows its base only, until they cover those chains too.

## Synths

Effects mostly do one thing, so a picture or two covers them. A synth is a voice
made of sections (oscillators, a mixer, a filter, an amp, envelopes and LFOs moving them,
effects after), with tens to hundreds of parameters. Push 3 handles this by showing one
section at a time with an overview to move between them. The `synth` template does the
same, with the widgets effects already use: `filter`, `envelope`, `lfo`, plus
`oscillators`.

```lua
kind = "synth",
sections = {
  { name = "OSC",    left = "oscillators", right = "osc_mix",  knobs = { ... } },
  { name = "FILTER", left = "response",    right = "contour",  knobs = { ... } },
  { name = "AMP",    left = "amp_env",     right = "amp_levels", knobs = { ... } },
  { name = "LFO",    left = "lfo1",        right = "lfo2",     knobs = { ... } },
},
```

- **The overview** is the mapping's first page: the filter's response on the left, the
  amp envelope on the right, and the knobs on the synth's eight most-played parameters
  (cutoff, resonance, envelope amount, attack, decay, the macros…).
- **A section's pages** follow it, with that section's pictures and knobs.
- **Getting around:** LEFT and RIGHT step page by page, through the overview, the sections
  and then the rest, as on any plug-in. The header names the section and the page within
  it (`FILTER 2/3`). No button is given over to jumping between sections.
- **Everything else** stays reachable: automapping's pages for the parameters no section
  uses.
- **NKS presets.** The overview and sections are the mapping's pages, so they follow the
  same rule as any mapping's ("Following the knobs"). A synth loaded from an NKS preset
  shows the preset's pages in the preset's order, with no overview or sections added,
  with pictures over the knobs they fit, unless the user ticks **Use the
  mapping's pages** for it.

**What a host can't see isn't drawn.** A synth's own internal modulation (Vital's
envelopes, LFOs and matrix, say) and its wavetables aren't parameters, so a mapping shows
them only through the parameters that set them (times, rates, position), plus DECKARD's
own routes. It never invents them.

**Products inside a host plug-in.** Kontakt and Reaktor are one plug-in to a host,
whatever is loaded in them, and their parameter ids mean different things in different
products. A mapping can't match the product (DECKARD doesn't keep which one is loaded),
and doesn't need to: an NKS preset's pages say what each knob is, and `generic`'s name
recognition, or a mapping for the host plug-in that binds by `knob` names only, gives it
pictures where its pages have the knobs a picture needs.

## Drawing and data flow

```
 UI thread                                 MK3 screens thread
 ─────────                                 ──────────────────
 on focus, reload or page change:          each frame (up to 30 a second):
   load and check the mapping                read the parameter cells and route travels
   resolve parameters, calibrate             lay out and draw widgets over the page's layer
   → Arc<PluginScene>  ─────────────────►    send each dirty rectangle as its own transfer
 each UI frame:
   publish bound parameters'
   values and text (64 at most)
```

- **Hosted plug-ins are read on the UI thread,** where their reads belong. It writes the
  bound values into a small array of atomics and their text into a double buffer, once per
  UI frame. The screens thread never touches a plug-in.
- **The page still draws around the mapping** with Lua `draw()` (anything the page adds).
  That picture becomes the static layer, and the widgets, header readout and knob cells
  are drawn over it.
- **Drawn on the existing screens thread** *(builder's call made in design)*. Dirty
  rectangles are needed for USB anyway, and a curve and a few cells are small. A separate
  drawing thread is "Later", only if phase 1 shows drawing and sending don't fit together.
- **Budget:** about 4 ms of CPU a frame for 960 × 272. Over budget, the rate drops and it's
  logged once.
- **CPU, not GPU** *(builder's call made in design)*. The MK3 needs pixels on the CPU for
  the USB encoder anyway, and 260,000 pixels is small. The existing rasteriser, extended,
  or `tiny-skia` will do.

## Checking a mapping

- **In the app:** a mapping's errors, undeclared or unresolved names, bad tapers and
  failed calibrations go to the script-error report, with lines.
- **For development:** an ignored test renders each shipped mapping's pages to PNGs at
  default values, at the extremes, and with touch focus on each knob, on the default and
  `lcd` themes, into a directory given by an environment variable. It loads a mapping
  against a saved parameter dump, so a mapping can be checked without the plug-in
  installed.
- **Golden images per widget** in `crates/surface`: new infrastructure (phase 1), built on
  `render_screens`.

## Documentation

`docs/controller-api.md` gains a **Mappings** chapter: where they live, matching,
`params`, calibration and tapers, widget inputs, pictures and pages, following the knobs,
templates, every widget's inputs, the buttons a mapping may use, which mapping is used,
versions, and a worked example. `DEVELOPMENT.md` says how to add a shipped third-party
mapping: the criteria, the parameter dump, the golden images, the black-box rule, and the
`plugin-mapping` skill. `docs/controller-plugins.md` points to it.

## Build order

| Phase | Delivers | Proof |
|---|---|---|
| 0 | Measure the MK3's display link: full and partial transfer rates, and how long transfers block; rectangle transfers in `deckard_mk3::Displays` | Numbers recorded in this document; a moving 120 × 72 cell costs its own pixels, not the screen's |
| 1 | Drawing on the screens thread with dirty rectangles; parameter cells; golden-image tests; `filter`, `eq`, `band`, `transfer`, `envelope`, `lfo`, `value` and knob cells, on every theme and at every width they span; touch focus from `BUTTON_CAP_n`. DECKARD's own Filter, EQ and Compressor pictures move to it, from the data `ControlView` already has, and `eq_curve`'s special cases go | Built-in pictures match on default and `lcd` themes; frame time under 4 ms; nothing sent when still; surface tests that a `BUTTON_CAP_n` press and release focus and unfocus while still reaching the page (Set Macro, double tap), and that three touches light nothing |
| 2 | Mappings: loader, matching, checking, `params` by `knob`, `id` and title, resolved again on page changes; calibration from `Params::text`, in memory; declared tapers; choice maps; pictures, `needs` and knob spans; laying out pictures on any page; following the knobs; **Preferences ▸ Hardware ▸ Mappings** (the checkbox and Off); `buttons` on display 4 and 7; `generic` with its patterns; modulation marks and `motion`; the remaining templates and widgets (`taps`, `decay`, `damping`, `oscillators`, `synth` sections and overview); shipped mappings for every DECKARD plug-in | Unit tests on resolution, calibration and choice maps, including an NKS preset's pages replacing the slot's and the checkbox swapping mapping and slot pages; recorded text dumps from several plug-ins calibrate; layout tests on recorded NKS pages (a page mixing filter and envelope knobs gets both pictures; a pair split over the gap goes to the cutoff's screen); an NKS plug-in with no mapping shows a filter picture on its filter page; MK3 check with one hosted plug-in |
| 3 | `plugin-probe` `list` and `dump`; **Dump Plug-in Parameters** and starter mappings; dumps in CI; the `plugin-mapping` skill; shipped mappings for Surge XT, Vital, Dexed and OB-Xd | Every shipped mapping passes against its dump; the skill drafts a mapping for each of the four that a person then only has to adjust |

Phase 1 gives DECKARD's own effects their new pictures with nothing new to learn; phase 2
opens it to every plug-in, NKS ones included with no mapping needed; phase 3 makes
mappings quick to write.

## Later

Each of these was in an earlier draft and was cut because its cost is high for what it
adds now. They can come back when a real need shows up.

- **A tap around one slot** (a new engine module, one post-slot ring) for a spectrum under
  the filter curve, a scope, meters and gain reduction history. The spectrum is the part
  people would notice; it needs an audio-thread change, so it waits until the pictures
  above are in use.
- **A measured response** (comparing audio before and after a slot, averaged and aligned
  by latency). Effects only, linear ones only, and mostly a check on a curve the model
  already draws.
- **Measuring plug-ins with the probe** (a sweep and an impulse through an effect, notes
  through a synth, fitting filter models, `model = "measured"` tables). The biggest piece
  of work in the earlier draft, for the few plug-ins whose value text doesn't give their
  units.
- **More synth pictures:** `partials` for FM and modal synths, a `path` of the voice's
  sections, `mods` for a synth's own envelopes and LFOs, and a wavetable picture.
- **Envelope playheads** for DECKARD's own instruments and the Sampler, and modelled ones
  for hosted plug-ins.
- **Arithmetic on widget inputs** (`add`, `mul`, `lerp`…), once a mapping needs more than
  a choice map.
- **Installing from the app** (drop a file or `.zip`, **Install Mapping…**), choosing
  between several matching mappings, and tracking a user's copy against the shipped one it
  came from.
- **`steps`, `euclid` and `roll` as widgets** for MIDI plug-ins' mappings.
- **A separate drawing thread**, if drawing and sending don't fit on the screens thread.
- Acting on a plug-in's `kParamTitlesChanged` and program changes, so knob names follow a
  preset chosen inside the plug-in's own window.
- Route travels for groups and the master, so their plug-ins show modulation.
- The Smart Strip as a mapping's control, alongside its PITCH, MOD, PERFORM and NOTES
  modes.
- Event hooks, if a mapping ever needs one that a field can't express, once scripts have a
  time limit.
- A shared mapping repository the preferences can browse and install from.
- The same `PluginScene` drawn in the app's own Control area, so a plug-in has the same
  face on screen as on the controller.
- Other controllers with screens, by giving the drawing their screen sizes and knob
  positions.
