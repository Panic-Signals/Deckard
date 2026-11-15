# Plug-in skins

A plug-in can come with a small Lua file, its **skin**, that gives it a real face on the
MK3's two screens: a filter shows its curve moving, an EQ its bands, a compressor its
transfer curve and gain reduction. The skin **declares** what the plug-in is and which
parameters mean what; DECKARD's Rust code draws it.

Design only: nothing here is built. No project-format change: skins are files beside the
controller scripts, and nothing about them is saved in a project.

Items marked *(builder's call made in design)* were chosen while writing this and were not
asked for directly. The user reviews them when reading this document.

## Why

Today the Control page shows a hosted plug-in as eight knob names and values. DECKARD's own
Filter, EQ and a few other effects get a curve on the right screen, each as a special case
in the app (`ControlView::eq`, `filter`, `picture`) and in `control.lua`'s `eq_curve`.
Hardware like Ableton's Push 3 shows that a screen over the knobs is most useful when it
shows what the sound is doing: the filter's curve under the cutoff knob, a band's dot under
its frequency knob, the envelope as it plays.

Goals:

- Any plug-in, built-in or hosted, can have a visual face, from one file a user can write.
- It looks as good as the best hardware: smooth, animated, lit where your hand is.
- Scripts stay declarative. Drawing, animation and timing are Rust's, so a skin can't make
  the screens slow or ugly, and the UI thread never runs Lua per frame.
- One path: DECKARD's own curves become shipped skins.
- DECKARD comes with skins for its own plug-ins and some common free ones, and users can
  write, install and share their own.
- Making a skin for a new plug-in is mostly automatic: a probe measures what each
  parameter really does, and a Claude Code skill turns that into a checked skin.

## The short version

- **A skin is a Lua table.** It says what the plug-in is (`kind = "filter"`), gives the
  parameters it uses names of its own, picks a widget for each screen and binds its inputs
  to those parameters, and maps the knobs, the eight display buttons, the 4-D encoder and
  the Smart Strip.
- **Rust draws.** A fixed set of widgets (filter curve, EQ, envelope, LFO, scope, spectrum,
  wavetable, transfer curve, delay taps, reverb space, XY, meters, knob cells) is drawn by a
  compositor thread with easing, glow and dithering.
- **One picture per screen.** The two screens have a physical gap between them, so nothing
  crosses it. Each screen has its own picture, and the four knobs under a screen move what
  that screen shows.
- **Touch focus.** A touched knob lights the part of its screen's picture it moves and dims
  the rest.
- **Model and measurement, side by side.** The curve drawn from the parameters is the
  model. A tap before and after the slot on screen gives the measured response and the
  live spectrum under it. If a skin has the plug-in wrong, it shows.
- **Modulation is visible.** The curve shows where the knob is and where modulation has
  moved it, with a fading trail of its sweep.
- **Shipped and user skins.** DECKARD ships skins for its own plug-ins and a set of common
  free ones. Users add their own by dropping a file in, and a skin is one file (plus
  optional pictures), so it's easy to share.
- **Automated analysis.** A headless probe treats a plug-in as a black box: it dumps its
  parameters and measures how its sound changes as each one moves. The `plugin-skin`
  Claude Code skill uses those measurements to write the skin, its test data and
  previews.

## What DECKARD already gives, and the gaps

| Need | DECKARD today | Gap |
|---|---|---|
| Draw on both screens | `crates/surface/src/screen.rs`: `draw(screen, theme)` in Lua at each redraw, 960 × 272, themes with named colours and styles. Frames go to the MK3 on their own thread with diff encoding (`deckard_mk3::display::encode_diff`) | Immediate mode in Lua on the UI thread: right for pages, wrong for 60 fps curves. No easing, no dithering for RGB565 gradients |
| Curves for built-ins | `EqCurveView` (`points`, `bands`, `extent`, `lfo`, `side`, `kind`), drawn by `control.lua` | Computed per effect in the app. Hosted plug-ins get nothing |
| Plug-in parameters | `KnobView` (name, section, text, 0..1 value, `mod_position`); `Params::value`, `Params::text`; `layout::Edit::PluginIn` | Values arrive normalised. A curve needs Hz, dB and ms, and a hosted plug-in's taper is unknown |
| Modulation | `mod_rt.rs` publishes route travels (`meters::RouteTravels`); the LFOs | Enough. A skin only says which parameter a widget input is |
| Audio inside a chain | The recording `Tap` (kept for Sampling); master and channel meters | No tap around one slot, so no pre/post per plug-in. Needs one (section "The slot tap") |
| Knob touch | The MK3's knobs are touch sensitive, and report 0x01's bits 72–79 already arrive as `BUTTON_CAP_1`…`BUTTON_CAP_8` (`deckard_mk3::BUTTONS`). Pages use them for Set Macro and double taps | Nothing reads them for display. The skin layer watches them too ("Touch focus") |
| Sandbox, hot reload, user folder | Safe stdlib only, script time limit, `<settings>/controller` replaces built-in files by path, reload on save | Reused as is |

## Where skins live and how one is found

```
crates/surface/lua/skins/            shipped with DECKARD (built into the binary)
  builtin/filter.lua  builtin/eq.lua  builtin/compressor.lua  ...
  vst3/surge-synth-team/surge-xt.lua  vst3/vital-audio/vital.lua  ...
<settings>/controller/skins/         the user's
  vst3/<vendor>/<plugin>.lua
  vst3/<vendor>/<plugin>/            the skin's pictures, if it has any
  au/<manufacturer>/<plugin>.lua
```

Where a skin sits doesn't decide what it matches; its `match` does. The paths are only
tidy. "Distribution" below covers which skins ship, how users add their own and which one
wins.

A skin is matched, most specific first:

1. **The plug-in's identity:** a VST3 class ID (`match = { vst3 = "ABCD…" }`) or an Audio
   Unit's type, subtype and manufacturer (`match = { au = { "aufx", "lpf1", "Acme" } }`).
   Survives renames.
2. **Vendor and name** (`match = { vendor = "Acme", name = "Ladder" }`), optionally with
   `version = ">=1.4"`.
3. **A built-in key** (`match = { builtin = "filter" }`), the key `plugin_pictures.rs`
   already uses.

No match: today's Control page, unchanged. A skin that fails to load or validate falls back
the same way, shows a small `SKIN` badge in the header, and its error, with its line, goes
to the script-error report scripts already have.

Skins reload when saved, as controller scripts do, and `defaults/skins/` is refreshed with
the other defaults when the folder is opened.

## Distribution

### What DECKARD ships

- **Every DECKARD plug-in:** its effects, instruments and drum synths. These replace
  today's special-case curves, and a DECKARD plug-in with no skin yet gets the `generic`
  template, never nothing.
- **A set of common third-party plug-ins,** chosen so anyone can check them: **free,
  available on macOS, Linux and Windows, widely used, and with stable parameter IDs.** The
  first set *(builder's call made in design)*: Surge XT, Vital, Dexed, OB-Xd,
  TAL-NoiseMaker, and Valhalla Supermassive. More can be added on the same terms, by
  anyone, as a pull request.
- **No vendor artwork.** A shipped third-party skin uses only parameter IDs, names and
  units, never the vendor's logos, panel images or fonts. Its identity on screen is the
  plug-in's name in DECKARD's own type and the group's colour.
- Shipped skins are built into the binary with the other scripts, and update with
  DECKARD's releases.

**Tests without the plug-in.** Each shipped third-party skin comes with a **parameter dump**
(`crates/surface/tests/skins/<plugin>.json`, made by `plugin-probe dump`): the plug-in's
version, every parameter's id, name and range, and its text at 64 points, plus the
measured responses its model was chosen from (see "Automated plug-in analysis"). CI loads the skin against the dump, checks that
every name resolves and every calibration parses, and renders its golden images. So a
skin's pull request can be reviewed and tested on any machine, and a plug-in update that
renames a parameter shows up as a failing dump once its dump is refreshed.

### How users add their own

- **Write one:** **Controller ▸ Open Controller Scripts Folder**, then `skins/`. The
  shipped skins are in `defaults/skins/` to copy from, as pages are today.
- **Start from the plug-in:** **Controller ▸ Dump Plug-in Parameters** writes the focused
  plug-in's parameter dump (the same JSON as the tests use) and a starter skin with every
  parameter listed and the `generic` template, into `skins/`. Most of a skin is then
  naming parameters and picking a template.
- **Install one someone shared:** drop a `.lua`, or a `.zip` of a `.lua` and its picture
  folder, onto DECKARD's window, or use **Controller ▸ Install Skin…**. DECKARD checks it,
  says which plug-in it's for, and copies it into `skins/` at its `match` path. A skin
  that doesn't check isn't installed, and the error says why.
- **Safe to share.** A skin gets the same Lua as controller scripts: no file, process or
  network access. Pictures are PNG only, at most 480 × 272 and 512 KB each. A shared skin
  can draw on the screens and move the plug-in's parameters, nothing else.

### Which skin wins

Several skins can match one plug-in: a shipped one, a user's copy of it, and one
downloaded from someone else.

1. The most specific `match` wins (plug-in identity, then vendor and name, then built-in
   key).
2. At equal specificity, the user's folder wins over the shipped skins.
3. Among the user's own, the newest `skin_version`.

**Preferences ▸ Hardware ▸ Skins** lists the plug-ins that have skins, and for each one
every matching skin, where it comes from (shipped or the user's) and which is in use. It
can pick another, or **None** to keep the plain Control page for that plug-in. The choice
is a preference, not part of any project.

### Versions

- **`api`:** each skin says which version of the skin format it's written for
  (`api = 1`). DECKARD loads skins written for its own version or older, and refuses a
  newer one with an error naming the DECKARD version it needs. New widgets and fields
  only ever add to the format, so an old skin keeps working.
- **`skin_version`** and **`author`** say which version of the skin this is and who made
  it. They show in **Preferences ▸ Hardware ▸ Skins**.
- **Plug-in updates:** `match.version` limits a skin to the plug-in versions it was
  written for. If an update renames or removes a parameter, its knob shows `?` and the
  script-error report names it; the rest of the skin keeps working.
- **A user's copy of a shipped skin** records the shipped `skin_version` it started from
  (`based_on`), which **Dump Plug-in Parameters** and copying from `defaults/` fill in.
  When a release ships a newer one, the script-error report says the user's copy is
  behind, so they can merge in the changes or delete their copy.

## Automated plug-in analysis

Writing a good skin by hand means finding out what each parameter really does: which knob
is the cutoff, how its 0..1 maps to Hz, which filter shape each mode is. A plug-in's names
are a hint, not the answer. So DECKARD gets a probe that measures it, and a Claude Code
skill that uses the probe to write the skin. Contributors use it to make shipped skins;
users with Claude Code use it for their own plug-ins.

### The probe

`plugin-probe`, a binary in `crates/plugins` (`src/bin/plugin-probe.rs`). It hosts one
plug-in **offline in its own process**, at 48 kHz with a fixed block size and a time limit,
and prints JSON. It never opens a project or touches DECKARD's settings, so a plug-in that
crashes takes only the probe with it.

| Command | What it gives |
|---|---|
| `plugin-probe list` | The installed plug-ins DECKARD can host, with their identities (VST3 class ID, AU type/subtype/manufacturer), vendor, name and version |
| `plugin-probe dump <plugin>` | The **parameter dump**: every parameter's id, name, units string, steps, flags (automatable, bypass, program change), its unit or section (VST3 units), its default, and its text at 64 points. This is the same JSON the tests and **Dump Plug-in Parameters** use |
| `plugin-probe response <plugin> [--set id=value …]` | For an effect: its frequency response (a log sine sweep, deconvolved), its impulse response, and its level in and out, at the parameter values given (others at their defaults) |
| `plugin-probe sweep <plugin> <id> [--points 16] [--set …]` | The response at each of `points` values of one parameter, plus what changed between them, already classified: a corner frequency moving (cutoff, with Hz at each point), a peak rising (resonance, with its height and Q), the slope changing (filter mode or slope), overall gain (drive, output), the dry part changing (mix), the impulse response's taps moving (delay time) or its decay changing (reverb decay, damping), nothing measurable (a parameter only the plug-in's UI uses) |
| `plugin-probe note <plugin> [--note 48] [--set …]` | For an instrument: a held note and its release, giving the level envelope (attack, decay, sustain and release fitted), the pitch, and the spectrum over time. With `sweep`, this finds an instrument's envelope times and its filter |

Everything goes through the plug-in's ordinary parameters and audio. The probe never reads
a plug-in's files or code, and never takes anything from its UI: it treats every plug-in
as a black box, as a listener with a test signal would.

### The skill

`.claude/skills/plugin-skin/SKILL.md`, beside the existing `pull-request` skill. Given a
plug-in name, it:

1. **Finds the plug-in** with `plugin-probe list`, and stops if a shipped or user skin
   already matches (unless asked to replace it).
2. **Dumps it,** and reads the names, units and sections to form a first guess: the
   `kind`, which parameter is which role, how they group into pages.
3. **Checks every guess by measuring.** For each candidate role it runs `sweep`: a
   "Cutoff" that doesn't move a corner frequency isn't the cutoff. For a filter it finds
   the cutoff's Hz at each point (that becomes the taper, or `calibrate` if the text
   already matches), resonance's peak against its value, and each mode's slope and shape,
   then picks the model shape (`svf_lp`, `ladder4`…) whose analytic response fits the
   measurement best. It does the same for delay times, reverb decay, compressor
   threshold and ratio, and instruments' envelopes.
4. **Lays out the pages:** the most-used parameters first, a picture's parameters on the
   knobs under its screen, sections kept together, buttons for small choices (filter
   mode, on/off).
5. **Writes the skin,** with `skin_version = 1` and a comment on each parameter saying
   how its role was established: *measured* (and how well the model fits) or *named
   only* (nothing to measure, such as a UI-only parameter).
6. **Writes the test data:** the parameter dump, plus the measured responses behind each
   choice (`crates/surface/tests/skins/<plugin>.json`). CI then checks the skin's model
   against those measurements, without the plug-in, and fails if a later change to a
   widget's maths moves the model away from what the plug-in really does.
7. **Renders the previews** (default values, extremes, touch focus on each knob), looks at
   them, and fixes what's wrong: an empty knob, a curve off the plot, a label that doesn't
   fit.
8. **Reports** what's measured and what's only named, the fit of each model in dB, and
   anything left unresolved. Then, as the user asks, it installs the skin in their
   `skins/` folder or prepares it for a pull request as a shipped skin (the
   `pull-request` skill takes it from there).

**When nothing fits.** If no model shape fits a filter within 2 dB where the measurement is
reliable, the skill doesn't pretend: it gives the picture `model = "measured"`, which draws
the measured response stored with the skin and interpolates between the measured values,
and says so in its report. The live slot tap still shows what's really happening.

**What the skill doesn't do.** It doesn't take a vendor's artwork, read a plug-in's files or
take a plug-in apart. The public-docs rule about NI's software applies to every plug-in.

## What a skin says

A skin returns one table. Nothing in it runs per frame: the only functions are optional
event hooks.

```lua
-- skins/vst3/acme/ladder.lua (illustrative)
return {
  api    = 1,                        -- the skin format it's written for
  skin_version = 3, author = "A. Name",
  match  = { vendor = "Acme", name = "Ladder" },
  kind   = "filter",                 -- picks the template (pictures, focus behaviour)
  accent = "group",                  -- the group's colour, as Push uses the track's

  params = {
    cutoff = { id = 3, unit = "hz", taper = { "log", 20, 20000 } },
    reso   = { id = 4, taper = { "lin", 0, 1 } },
    drive  = { id = 5, unit = "db", calibrate = true },   -- learn the taper from the plug-in's text
    mode   = { id = 6, choices = { "LP24", "LP12", "BP", "HP", "Notch" } },
    env    = { id = 9, taper = { "lin", -1, 1 } },
    mix    = { name = "Mix" },                            -- by name when there's no stable id
  },

  left = {                           -- the left screen's picture
    widget = "filter",
    model  = {
      shape     = { from = "mode", map = { LP24 = "ladder4", LP12 = "svf_lp", BP = "svf_bp",
                                           HP = "svf_hp", Notch = "svf_notch" } },
      cutoff    = "cutoff",
      resonance = "reso",
      drive     = "drive",
    },
    measured = true,                 -- the slot tap's measured response and spectrum
    modulation = true,               -- base solid, modulated live, fading trail
  },
  right = { widget = "sweep", cutoff = "cutoff" },   -- output spectrogram with cutoff's path over it

  pages = {
    { name = "FILTER", knobs = { "cutoff", "reso", "drive", "env",   "mode", nil, nil, "mix" } },
    { name = "ENV",    knobs = { "attack", "decay", "sustain", "release", "env", nil, nil, nil },
      right = { widget = "envelope", model = { a = "attack", d = "decay", s = "sustain", r = "release" } } },
  },

  buttons = {                        -- the eight display buttons above the screens
    [1] = { choose = "mode", value = "LP24" }, [2] = { choose = "mode", value = "LP12" },
    [3] = { choose = "mode", value = "BP" },   [4] = { choose = "mode", value = "HP" },
    [8] = { toggle = "bypass" },
  },

  strip = { param = "cutoff" },      -- the Smart Strip sweeps cutoff; its finger shows on the curve
}
```

### Parameters

A skin names the parameters it uses once, under its own names, and the rest of the file
uses those. Each binds by **id** (a VST3 ParamID, an Audio Unit parameter ID, or a built-in
field) or by **name** as a fallback, resolved when the slot is focused or the plug-in
reloads. A name that doesn't resolve leaves its widget input at its default and its knob
cell showing `?`. Nothing is guessed.

Every write goes through `layout::Edit::PluginIn` or `Edit::Param`, as a knob turn does
today, so Auto-write, Lock, macros and undo all see it. A skin adds no write path.

### Real units: taper or calibration

A curve needs 1.2 kHz, not 0.43. Two ways:

- **A declared taper:** `{ "log", 20, 20000 }`, `{ "lin", a, b }`, `{ "exp", a, b, curve }`,
  `{ "db", -inf, 12 }`, or `{ "table", {0, 20}, {0.5, 800}, {1, 20000} }`.
- **`calibrate = true`:** once per plug-in version, off the frame path, DECKARD reads
  `Params::text` at 64 normalised points and parses the numbers and units (`"1.20 kHz"`,
  `"-6.0 dB"`, `"120 ms"`, `"1/8 D"`). The fitted table is cached in
  `<settings>/skins/cache/`. If the text doesn't parse, the skin is reported and must
  declare a taper.

The knob cell always shows the plug-in's own text, so a wrong taper bends the picture,
never the number.

### Value maps

A widget input is a parameter's name, a constant, or a small table expression:

```lua
cutoff = { "add", "cutoff", { "mul", "env", 0.5 } }
shape  = { from = "mode", map = { ... } }
```

Operators: `add mul min max clamp lerp map choose`. They compile to a small expression tree
that Rust evaluates each frame, so a skin can't stall the screens.

### Templates

`kind` picks a template: a picture for each screen, the knob pages' defaults, and what each
parameter lights on touch. A skin may override any part.

| `kind` | Left screen | Right screen |
|---|---|---|
| `filter` | response curve (model, measured ghost, spectrum under it) | sweep: output spectrogram scrolling, the cutoff's path drawn over it |
| `eq` | the whole curve with its bands; the encoder picks a band | the chosen band close up: its frequency, gain and Q large, the spectrum around it, how hard it's working |
| `dynamics` | transfer curve, the input level riding it | gain reduction history scrolling, the needle dropping from the top |
| `delay` | the repeats as a timeline, left and right lanes, each flashing as it fires | the feedback path: its filter curve and how fast repeats die |
| `reverb` | the space: early reflections, then the tail's energy over time | damping and the tail's tone |
| `synth` | per page: oscillator or wavetable | per page: filter or envelope |
| `drum` | the voice's shape (pitch and amplitude envelopes) | hit history |
| `utility` | meters and phase | stereo width |
| `generic` | big knob cells, four a screen | big knob cells, four a screen |

### Widgets

Every widget takes bound inputs and its colours and type from the theme.

| Widget | Inputs | What makes it look good |
|---|---|---|
| `filter` | shape, cutoff, resonance, drive, slope, mix; optional second filter, serial or parallel | Analytic response for SVF, ladder, comb and formant shapes, from the maths already in `crates/engine/src/fx/`. The resonance peak blooms with Q. A gradient fill under the curve fades to black, with the post spectrum glowing through it |
| `sweep` | a frequency parameter, the slot tap | A spectrogram scrolling right to left, with the parameter's path drawn over it, so a sweep leaves a visible trace |
| `eq` | bands: frequency, gain, Q, type, on | Each band a dot sized by Q, the sum curve plus a ghost per band. Bands pulse with the energy in their range |
| `band` | one band's parameters, the slot tap | Large numbers, the spectrum around the band, the band's own curve |
| `envelope` | ADSR, DAHDSR or breakpoints, curvature | A dot per sounding note moving along it (see "Modulation and playheads") |
| `lfo` | shape, rate, phase, depth, sync | The shape scrolls under a fixed cursor, which leaves a short tail |
| `scope` | the slot tap | Phosphor persistence, the last frames fading |
| `spectrum` | the slot tap, pre or post | Log frequency, smoothed, with peak hold |
| `wavetable` | frames (from the plug-in if it shares them, else declared), position | A stack in slight perspective, the current frame lit, its neighbours fading |
| `transfer` | threshold, ratio, knee, makeup | The input level as a dot riding the curve |
| `gr` | gain reduction (published by the plug-in, else measured) | A history scrolling beside a needle |
| `taps` | time, feedback, spread, sync | Repeats as decaying bars, each flashing as it fires (from tempo and the slot's notes) |
| `space` | pre-delay, size, decay, damping | A cloud of early reflections, then a tail darkened by damping |
| `xy` | x, y | A trail that fades, for performance effects |
| `meter` | the slot tap | Peak hold |
| `value` | one parameter | Large numerals, the unit small |
| `knobs` | the page's knobs | The cells over the knobs: arc, value, name, modulation arc. Always drawn |
| `steps`, `euclid`, `roll` | as `EuclidView` and `RandomView` show today | Ported as widgets so a MIDI plug-in's skin can use them |
| `image` | a key | The plug-in's logo, or a PNG beside the skin, for identity |

Adding a widget is a Rust change with a golden-image test. That's on purpose: it keeps the
look consistent.

### Hooks (optional, rare)

```lua
on_page   = function(ctx, page) end
on_button = function(ctx, n, down) end
on_touch  = function(ctx, knob, touched) end
```

`ctx` reads parameter values and keeps the skin's own small state (`ctx.state.band = 3`).
It writes parameters only with `ctx.edit(name, value)`, the same write as a knob. Hooks run
on the UI thread, on events only, inside the scripts' time limit.

**Escape hatch:** `widget = "lua"` with `draw = function(screen, theme, ctx)` draws one
screen with today's `screen` API, at most 15 times a second. It's for something no widget
covers; the script-error report warns about it.

## The screens

The MK3 has four knobs under each screen and four display buttons above it. So each screen
has three bands, lined up with the hardware, and its own picture:

```
 ┌─────────── left screen 480 ───────────┐   ┌─────────── right screen 480 ──────────┐
 │ [LP24]   [LP12]   [ BP ]   [ HP ]      │   │ [    ]   [    ]   [    ]   [BYPASS]   │  28  buttons
 │ LADDER · FILTER               1/2      │   │ SWEEP                Group B · Snd 3  │
 │      ░▒▓▓▓▓█▇▆▅▄                       │   │ ░░▒▒▓▓▓▓▒▒░░░░▒▒▓▓▓▒▒░░    ← time      │
 │ ──────────────────╮ ✶                  │   │      ╭───╮        ╭──╮                │ 172  picture
 │  · · measured · ·  ╲╱╲                 │   │ ────╯     ╰──────╯    ╰──  cutoff     │
 │  ▁▂▃▅▆▅▃ spectrum ▂ ╲  ╲____           │   │ ▒▒▓▓▓▒▒░░░░▒▒▓▓▒▒░░░░▒▒▓▓            │
 │ 20     100     1k      10k             │   │                                       │
 ├─────────┬─────────┬─────────┬─────────┤   ├─────────┬─────────┬─────────┬─────────┤
 │ CUTOFF  │ RESO    │ DRIVE   │ ENV     │   │ MODE    │         │         │ MIX     │  72  knob cells
 │  ◜‾‾◝   │  ◜‾◝    │  ◜◝     │  ◜‾◝    │   │  LP24   │         │         │  100 %  │
 │ 1.20 k  │ 42 %    │ 3.0 dB  │ +18 %   │   │         │         │         │         │
 └─────────┴─────────┴─────────┴─────────┘   └─────────┴─────────┴─────────┴─────────┘
```

- **Nothing crosses the gap.** Each screen is its own 480 × 272 picture.
- **The knobs under a screen move what that screen shows.** Templates put a picture's
  parameters on the knobs beneath it, as Push 3 groups a device's sections.
- **Knob cells sit exactly over the knobs,** 120 px each. The modulation arc is a thin ring
  outside the value arc.
- **Button labels sit exactly over the buttons,** filled when chosen, with the theme's
  `chase` border on the chosen one where the theme has it.

### Touch focus

Touching a knob:

1. Everything on that knob's screen not bound to its parameter dims to 35 %.
2. The bound part lights: the cutoff handle grows and a guide line drops from it, the
   resonance peak shows its Q as a bracket, a band's dot becomes a ring with its frequency
   and gain beside it.
3. The knob's cell grows upward into a `value` readout: the value in large numerals, the
   name, the range, the default as a tick.
4. On letting go, it eases back over 180 ms.

The other screen keeps its picture. The template says what each parameter lights; a skin
can change it with `focus = { cutoff = "handle", reso = "peak" }`.

**How touch is read.** The MK3 already reports a finger on knob *n* as `BUTTON_CAP_n` down
and its lifting as up. The surface passes those events on to the page and `global.lua`
exactly as now, so Set Macro, double taps and anything a script does with them keep
working, and also tells the skin layer, which only changes what's drawn. Focus starts on
touch, before the knob moves, so you see what a knob will do before turning it. It ends
180 ms after the finger lifts, or, for a knob turned without a touch registering (a glove,
a quick flick), 1.5 s after its last turn. Two knobs touched at once each light their part;
a third replaces the oldest.

A skin's `on_touch` hook gets the same events. Touch focus can be turned off in
**Preferences ▸ Hardware** for anyone who rests their hand on the knobs.

### Motion

- **Values ease** with a critically damped spring (about 60 ms), so knob steps glide. The
  knob's text doesn't ease: it always shows the plug-in's value.
- **Pages slide** sideways over 150 ms; changing slot crossfades.
- **The group's colour is the accent.** Changing group recolours the curve's glow.
- **Gradients are dithered** (4 × 4 ordered) so fills and glows don't band on the
  screens' RGB565. This is the biggest single thing that makes these screens look good.
- **Still when nothing moves.** With no audio, modulation or touch, the frame doesn't
  change and `encode_diff` sends nothing, so motion costs USB only where it happens.

## The slot tap

To measure a plug-in, the engine needs the audio just before and just after it.

- A new engine module, `slot_tap.rs`, keeps two stereo rings (before and after) of 4096
  frames, written with stores to atomics only: no lock, no allocation. It has nothing to
  do with the recording `Tap`, which stays free for Sampling.
- **One slot is tapped at a time:** the one the MK3 shows. The UI thread sets the chain and
  slot in an atomic. The engine compares it once per block, and only that slot copies its
  input and output.
- A worker thread (neither the UI thread nor the screen sender) runs a 4096-point windowed
  FFT on both rings 30 times a second, giving the spectrum after, the spectrum before, and
  the **measured response**: after over before, smoothed, drawn only where there's signal
  before (elsewhere the ghost fades out rather than drawing noise).
- A plug-in's own Mix knob shows in the measured curve. That's intended: the picture shows
  what really happens.
- The tap runs only while a skin that uses it (`measured`, `spectrum`, `sweep`, `scope`,
  `meter`, `band`) is on screen, and costs nothing otherwise. *(builder's call made in
  design)* No button turns it on or off. Its cost is one slot's copy on the audio thread and
  an FFT on a worker.

## Modulation and playheads

- **Where the parameter really is:** route travels give each bound parameter's modulated
  position. The widget draws the base solid and the modulated position live, and keeps the
  last ~400 ms of modulated curves fading, so an LFO-swept filter shows its whole sweep as
  a glowing fan. A band shows the route's full depth.
- **Knob cells** show the same as the outer arc.
- **Envelope playheads:** DECKARD's own instruments and the Sampler can publish their
  envelopes' phase. A hosted plug-in can't, so its playheads are modelled from the slot's
  note events (a small ring beside the slot tap) and the declared times, and drawn hollow,
  so they never pretend to be the plug-in's own envelope.
- **MIDI plug-ins** (`docs/midi-input-plugins.md`) get `steps` and `roll` fed from those
  note events.

## Threads and data flow

```
 UI thread                          skin compositor thread               MK3 screens thread
 ─────────                          ──────────────────────               ──────────────────
 on focus or reload:                each frame (up to 60 a second):      unchanged:
   load and check the skin            read the parameter cells, route      encode_diff → USB
   resolve parameters                 travels, LFOs, slot tap spectra
   compile value maps                 evaluate value maps
   → Arc<SkinScene>  ─────────────►   ease, lay out, draw widgets ───────► screen frame
 each UI frame:                       over the page's own layer
   publish bound parameters'
   values and text (64 at most)
 on events: hooks, edits
```

- **Hosted plug-ins are read on the UI thread,** where their reads belong. It writes the
  bound values into a small array of atomics and their text into a double buffer, once per
  UI frame. The compositor never touches a plug-in.
- **The page still draws around the skin** with Lua `draw()` (the header and anything the
  page adds). That picture becomes the static layer, and the compositor draws the pictures
  and knob cells over it. With no skin focused, the compositor doesn't run.
- **Budget:** about 4 ms of CPU a frame for 960 × 272 at 60 fps. Over budget, the
  compositor drops to 30 fps and logs it once.

### CPU, not GPU

The MK3 needs pixels on the CPU for the USB encoder anyway, a GPU readback each frame adds
latency, and 260,000 pixels is small. The existing rasteriser, extended, or `tiny-skia`
will do. *(builder's call made in design)* Revisit only if a widget (the wavetable stack)
doesn't fit the budget.

## Checking a skin

- **In the app:** a skin's errors, unresolved names, bad tapers and failed calibrations go
  to the script-error report, with lines.
- **For development:** an ignored test renders each shipped skin's pages to PNGs at default
  values, at the extremes, and with touch focus on each knob, into a directory given by an
  environment variable. It also loads a skin against a saved dump of a plug-in's parameter
  list and texts, so a skin can be checked without the plug-in installed.
- **Golden images per widget** in `crates/surface`, as `render_screens` does now.

## Documentation

`docs/controller-api.md` gains a **Skins** chapter: where they live, matching, `params`,
tapers and calibration, value maps, templates, every widget's inputs, hooks, installing,
sharing, which skin wins, versions, and a worked example. `CONTRIBUTING` (or
`DEVELOPMENT.md`) says how to add a shipped third-party skin: the criteria, the parameter
dump, the golden images, and the `plugin-skin` skill and `plugin-probe` that make most of
it. `docs/controller-plugins.md` points to it.

## Build order

| Phase | Delivers | Proof |
|---|---|---|
| 1 | Loader, matching, checking, parameter resolution, value maps; the `generic` template (big knob cells, button labels); fallback and `SKIN` badge | Unit tests on resolution and maps; golden images; MK3 check with one hosted plug-in |
| 2 | Compositor thread, parameter cells, easing, dithering, page slides | Frame time under 4 ms; no USB traffic when still |
| 3 | `filter`, `eq`, `band`, `envelope`, `lfo`, `transfer`, `value`; templates; touch focus from `BUTTON_CAP_n`; DECKARD's Filter, EQ and Compressor curves move to shipped skins, and `eq_curve`'s special cases go | Built-in curves match today's golden images; surface tests that a `BUTTON_CAP_n` press and release focus and unfocus while still reaching the page (Set Macro, double tap) |
| 4 | Calibration from `Params::text` and its cache | Tests on recorded text dumps from several hosted plug-ins |
| 5 | Slot tap, FFT worker, measured response, `spectrum`, `sweep`, `scope`, `meter` | No allocation on the audio thread; the built-in Filter's measured response matches its model within 1 dB where there's signal |
| 6 | Modulation: modulated position, trail, depth band; envelope playheads | Route tests; check with an LFO on cutoff |
| 7 | `wavetable`, `taps`, `space`, `xy`, `gr`, `steps`, `roll`; shipped skins for the rest of DECKARD's effects and instruments | Golden images |
| 8 | Distribution: **Dump Plug-in Parameters** and starter skins; **Install Skin…** and drop to install; **Preferences ▸ Hardware ▸ Skins**; `api`, `skin_version`, `based_on`; parameter dumps in CI | Install, override and precedence tests; every shipped skin passes against its dump |
| 9 | `plugin-probe` (`list`, `dump`, `response`, `sweep`, `note`); the `plugin-skin` skill; `model = "measured"`; CI checks models against stored measurements; the first shipped third-party skins made with it | The probe classifies DECKARD's own Filter, EQ, Delay and Compressor parameters correctly, with their answers known; the skill writes a skin for each of the first third-party plug-ins whose models fit their measurements |

Phases 1–3 give DECKARD's effects their pictures and every plug-in a clean face. Phases 5
and 6 make it look alive.

## Later

- A shared skin repository that **Preferences ▸ Hardware ▸ Skins** can browse and install
  from, so a skin someone writes for a plug-in reaches everyone who has it.

- The same `SkinScene` could draw in the app's own Control area, so a plug-in has the same
  face on screen as on the controller.
- Other controllers with screens get skins by giving the compositor their screen sizes and
  knob positions.
