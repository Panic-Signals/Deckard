# Plug-in skins

A plug-in can come with a small Lua file, its **skin**, that gives it a real face on the
MK3's two screens: a filter shows its curve moving, an EQ its bands, a compressor its
transfer curve and gain reduction. The skin **declares** what the plug-in is and which
parameters mean what; DECKARD's Rust code draws it.

Design only: nothing here is built. No project-format change: skins are files beside the
controller scripts, and nothing about them is saved in a project. **Nothing is stored per
preset either:** a skin works from the slot's knob mapping as it is at that moment (see
"Following the knobs").

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
- It reads at a glance and responds the moment a hand moves, on every theme, including the
  two-tone `lcd` one.
- Scripts stay declarative. Drawing, animation and timing are Rust's, so a skin can't make
  the screens slow or ugly, and the UI thread never runs Lua per frame.
- One path: DECKARD's own curves become shipped skins.
- DECKARD comes with skins for its own plug-ins and some common free ones, and users can
  write, install and share their own.
- Making a skin for a new plug-in is mostly automatic: a probe measures what each
  parameter does, and a Claude Code skill turns that into a skin, checked as far as
  measurement can check it.

## The short version

- **A skin is a Lua table.** It says what the plug-in is (`kind = "filter"`), gives the
  parameters it uses names of its own, picks a widget for each screen and binds its inputs
  to those parameters, and can add knob pages and button choices.
- **Rust draws.** A fixed set of widgets (filter curve, EQ, envelope, LFO, scope, spectrum,
  transfer curve, delay taps, reverb decay, meters, knob cells…) is drawn by a compositor
  thread. Every widget has a flat look made from the theme's colours; glow and gradients
  are extras a theme can turn on.
- **One picture per screen.** The two screens have a physical gap between them, so nothing
  crosses it. Each screen has its own picture, and the four knobs under a screen move what
  that screen shows.
- **Following the knobs.** The slot's pages, whatever filled them (an NKS preset, the
  user, automapping), are the one knob-to-parameter mapping. Pictures find their inputs on
  the knobs as they are now, so they change when the pages change and never disagree with
  what the knobs do. A skin's own knob pages apply when the slot has no NKS or user pages,
  or when the user ticks an override for that plug-in in the preferences.
- **Touch focus.** A resting finger on a knob lights the part of its screen's picture it
  moves and puts its value large in the header.
- **Model and measurement.** For effects, the curve drawn from the parameters is the
  model, and a tap before and after the slot gives the measured response and the live
  spectrum. If a skin has the plug-in wrong, it shows.
- **Modulation is visible.** The curve shows where the knob is and where modulation has
  moved it.
- **Shipped and user skins.** DECKARD ships skins for its own plug-ins and a set of free,
  open-source ones. Users add their own by dropping a file in.
- **Automated analysis.** A headless probe measures a plug-in as a black box, and the
  `plugin-skin` Claude Code skill uses its measurements to write a skin, its test data and
  previews.

## What DECKARD already gives, and the gaps

| Need | DECKARD today | Gap |
|---|---|---|
| Draw on both screens | `crates/surface/src/screen.rs`: `draw(screen, theme)` in Lua at each redraw, 960 × 272, themes with named colours and styles. Frames go to the MK3 on their own thread; `deckard_mk3::Displays::show` sends every row from the first changed to the last, at full width, and waits for each transfer | Immediate mode in Lua on the UI thread: right for pages, wrong for moving curves. A small moving curve resends most of a screen. The display protocol's header already carries a rectangle (x, y, width, height) and a skip command (`02`), so sending only what changed needs no new protocol |
| Curves for built-ins | `EqCurveView` (`points`, `bands`, `extent`, `lfo`, `side`, `kind`), drawn by `control.lua` | Computed per effect in the app. Hosted plug-ins get nothing |
| Plug-in parameters | `KnobView` (name, section, text, 0..1 value); `Params::value`, `Params::text`; `ParamInfo` (id, title, short title, units string, steps, default); `layout::Edit::PluginIn` | Values arrive normalised: a curve needs Hz, dB and ms, and a hosted plug-in's taper is unknown. `ParamInfo` has no flags and no section (VST3 unit), and the bypass and program-change parameters are filtered out before anything sees them |
| The knob mapping | The slot's pages, saved with it (`PluginRef::pages`: each knob's parameter id, section and name). Loading an NKS preset through the browser writes its pages there; the user can edit them; when there are none, automapping (the first 128 automatable parameters, eight to a page) | Enough: pictures resolve against it ("Following the knobs") |
| Modulation | `mod_rt.rs` publishes route travels (`meters::RouteTravels`) for each sound's routes; `App::mod_position` turns one into a knob's modulated position | Sounds only: a plug-in on a group or the master shows no modulation until route travels cover those chains |
| Audio inside a chain | The recording `Tap` (kept for Sampling); master and channel meters | No tap around one slot, so no pre/post per plug-in. Needs one ("The slot tap") |
| Knob touch | The MK3's knobs are touch sensitive, and report 0x01's bits 72–79 already arrive as `BUTTON_CAP_1`…`BUTTON_CAP_8` (`deckard_mk3::BUTTONS`). Pages use them for Set Macro and double taps | Nothing reads them for display. The skin layer watches them too ("Touch focus") |
| Sandbox, hot reload, user folder | Safe stdlib only, `<settings>/controller` replaces built-in files by path, reload on save | Reused as is. There is no time limit on scripts, so skins have no per-frame or per-event code in the first version ("What a skin says") |
| Screen tests | `screen.rs`'s `render_screens` test draws pages without a device | No golden-image comparison yet. Widgets need one (phase 2) |

## Where skins live and how one is found

```
crates/surface/lua/skins/            shipped with DECKARD (built into the binary)
  builtin/filter.lua  builtin/eq.lua  builtin/compressor.lua  ...
  vst3/surge-synth-team/surge-xt.lua  vst3/vital-audio/vital.lua  ...
<settings>/controller/skins/         the user's
  <vendor>/<plugin>.lua
  <vendor>/<plugin>/                 the skin's pictures, if it has any
```

Where a skin sits doesn't decide what it matches; its `match` does. The paths are only
tidy. One skin can serve a plug-in in both formats ("Matching").

### Matching

A skin's `match` lists what it's for:

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
(`id = { vst3 = 3, au = 1003 }`, "Parameters").

No match: today's Control page, unchanged. A skin that fails to load or check falls back
the same way, shows a small `SKIN` badge in the header, and its error, with its line, goes
to the script-error report scripts already have.

Skins reload when saved, as controller scripts do, and `defaults/skins/` is refreshed with
the other defaults when the folder is opened.

## Distribution

### What DECKARD ships

- **Every DECKARD plug-in:** its effects, instruments and drum synths. These replace
  today's special-case curves, and a DECKARD plug-in with no skin yet gets the `generic`
  template, never nothing.
- **A set of third-party plug-ins,** chosen so anyone can check them and so measuring
  them raises no question: **open source, free, available on macOS, Linux and Windows,
  widely used, and with stable parameter IDs.** The first set *(builder's call made in
  design)*: Surge XT, Vital, Dexed and OB-Xd. More can be added on the same terms, by
  anyone, as a pull request. Skins for commercial plug-ins are for users to write and
  share themselves, not to ship.
- **No vendor artwork.** A shipped skin uses only parameter IDs, names and units, never a
  vendor's logos, panel images or fonts. Its identity on screen is the plug-in's name in
  DECKARD's own type and the group's colour.
- Shipped skins are built into the binary with the other scripts, and update with
  DECKARD's releases.

**Tests without the plug-in.** Each shipped third-party skin comes with a **parameter dump**
(`crates/surface/tests/skins/<plugin>.json`, made by `plugin-probe dump`): the plug-in's
version, every parameter's id, name and range, and its text at 64 points, plus the
measured responses its model was chosen from ("Automated plug-in analysis"). CI loads the
skin against the dump, checks that every name resolves and every calibration parses, and
renders its golden images. So a skin's pull request can be reviewed and tested on any
machine, and a plug-in update that renames a parameter shows up as a failing test once its
dump is refreshed. Dumps are made only from the plug-in itself: never from NKS presets or
other files of a commercial library.

### How users add their own

- **Write one:** **Controller ▸ Open Controller Scripts Folder**, then `skins/`. The
  shipped skins are in `defaults/skins/` to copy from, as pages are today.
- **Start from the plug-in:** **Controller ▸ Dump Plug-in Parameters** writes the focused
  plug-in's parameter dump (the same JSON as the tests use) and a starter skin with every
  parameter listed and the `generic` template, into `skins/`. Most of a skin is then
  naming parameters and picking a template.
- **Install one someone shared:** drop a `.lua`, or a `.zip` of a `.lua` and its picture
  folder, onto DECKARD's window, or use **Controller ▸ Install Skin…**. DECKARD checks it,
  says which plug-in it's for, and copies it into `skins/`. A skin that doesn't check isn't
  installed, and the error says why.
- **Safe to share.** A skin is a table of data: it has no code that runs (see "What a skin
  says"), and is loaded with the same safe Lua as controller scripts, with no file,
  process or network access. Pictures are PNG only, at most 480 × 272 and 512 KB each. A
  shared skin can draw on the screens and move the plug-in's parameters, nothing else.

### Which skin wins

Several skins can match one plug-in: a shipped one, a user's copy of it, and one
downloaded from someone else.

1. **The user's folder wins over the shipped skins,** whatever their match. A user who
   wrote or installed a skin meant it.
2. Within one place, the most specific match wins: the plug-in's identity, then vendor and
   name.
3. Then the newest `skin_version`.

**Preferences ▸ Hardware ▸ Skins** lists the plug-ins that have skins, and for each one
every matching skin, where it comes from (shipped or the user's) and which is in use. It
can pick another, or **None** to keep the plain Control page for that plug-in, and has the
**Use the skin's pages** checkbox ("Following the knobs"). These are preferences, not part
of any project.

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

Writing a good skin by hand means finding out what each parameter does: which knob is the
cutoff, how its 0..1 maps to Hz, which filter shape each mode is. A plug-in's names are a
hint, not the answer. So DECKARD gets a probe that measures it, and a Claude Code skill
that uses the probe to draft the skin. Contributors use it to make shipped skins; users
with Claude Code use it for their own plug-ins.

**The black-box rule.** The probe and the skill learn about a plug-in only by hosting it
and listening to it: its parameters and its audio, as a listener with a test signal would.
They never read a plug-in's files, code, presets' private data or artwork, never take one
apart, and never look at how any vendor's software works inside. This applies to every
plug-in, Native Instruments' included.

### The probe

`plugin-probe`, a binary in `crates/plugins` (`src/bin/plugin-probe.rs`), is a measurement
tool. It hosts one plug-in **offline in its own process**, at 48 kHz with a fixed block
size and a time limit, and prints JSON. It never opens a project or touches DECKARD's
settings, so a plug-in that crashes takes only the probe with it.

| Command | What it gives |
|---|---|
| `plugin-probe list` | The installed plug-ins DECKARD can host, with their identities (VST3 class ID, AU type/subtype/manufacturer), vendor, name and version |
| `plugin-probe dump <plugin>` | The **parameter dump**: every parameter's id, name, units string, steps, default, flags (automatable, bypass, program change) and section (VST3 unit) where the plug-in reports them, and its text at 64 points. The probe reads flags and sections itself; DECKARD's `ParamInfo` doesn't carry them yet. This is the same JSON the tests and **Dump Plug-in Parameters** use |
| `plugin-probe response <plugin> [--set id=value …]` | For an effect, at the parameter values given (others at their defaults), from two test signals: a **log sine sweep**, deconvolved into a frequency and impulse response, and a **real impulse**, a single-sample click at −12, −24 and −36 dBFS, recorded as it comes out. It reports the frequency response, both impulse responses, the level in and out, and what comparing them shows (below) |
| `plugin-probe sweep <plugin> <id> [--points 16] [--set …]` | The response at each of `points` values of one parameter, plus what changed between them: a corner frequency moving (with Hz at each point), a peak rising (with its height and Q), the slope changing, overall gain, the dry part changing, the impulse response's taps moving or its decay changing, or nothing measurable |
| `plugin-probe note <plugin> [--note 48] [--set …]` | For an instrument: a held note and its release, giving the level envelope (attack, decay, sustain and release fitted), the pitch, and the spectrum over time |

**Why both a sweep and an impulse.** For a linear, steady plug-in (an EQ, a static filter, a
plain delay) the two impulse responses match, and the sweep's is the cleaner one. Where
they differ, that difference is information:

- **They differ, and the impulse's shape changes with its level:** the plug-in is
  nonlinear (drive, saturation, a compressor or gate acting on the click). The skill
  then probes it as dynamics or distortion rather than as a filter, and the skin turns off
  the measured response for it.
- **They differ, and repeated impulses differ from each other:** the plug-in moves on its
  own (chorus, phaser, an internal LFO). Its rate is measured from the impulses' spacing.
- **Latency:** where the impulse comes out, against the latency the plug-in reports. A
  mismatch is reported, since the slot tap aligns its two rings by the reported latency.
- **Pre-ringing** before the impulse's arrival: a linear-phase EQ or filter.
- **A gate or expander** passes the loud impulse and swallows the quiet ones, which gives
  its threshold.

### The skill

`.claude/skills/plugin-skin/SKILL.md`, beside the existing `pull-request` skill. Its
classification is best-effort: it says what it measured and what it only guessed from
names. Given a plug-in name, it:

1. **Finds the plug-in** with `plugin-probe list`, and stops if a shipped or user skin
   already matches (unless asked to replace it).
2. **Dumps it,** and reads the names, units and sections to form a first guess: the
   `kind`, which parameter is which role, how they group into pages.
3. **Checks each guess by measuring.** It first runs `response` once at the defaults: the
   sweep against the impulse says whether the plug-in is linear and steady, nonlinear or
   moving on its own, which decides how the rest is measured. For each candidate role it
   then runs `sweep`: a "Cutoff" that doesn't move a corner frequency isn't the cutoff.
   For a filter it finds the cutoff's Hz at each point (that becomes the taper, or
   `calibrate` if the text already matches), resonance's peak against its value, and each
   mode's slope, then picks the model shape (`svf_lp`, `ladder4`…) that fits best. It does
   the same for delay times, reverb decay, compressor threshold and ratio. For a synth it
   follows "Measuring a synth".
4. **Lays out the pages:** the most-used parameters first, a picture's parameters on the
   knobs under its screen, sections kept together, buttons for small choices (filter
   mode, on/off). For a synth, the sections and the overview ("Synths").
5. **Writes the skin,** with `skin_version = 1` and a comment on each parameter saying
   how its role was established: *measured* (and how well the model fits) or *named
   only*.
6. **Writes the test data:** the parameter dump, plus the measured responses behind each
   choice (`crates/surface/tests/skins/<plugin>.json`). CI then checks the skin's model
   against those measurements without the plug-in, and fails if a later change to a
   widget's maths moves the model away from them.
7. **Renders the previews** (default values, extremes, touch focus on each knob), looks at
   them, and fixes what's wrong: an empty knob, a curve off the plot, a label that doesn't
   fit.
8. **Reports** what's measured and what's only named, the fit of each model in dB, and
   anything left unresolved. Then, as the user asks, it installs the skin in their
   `skins/` folder or prepares it for a pull request as a shipped skin (the
   `pull-request` skill takes it from there).

**When nothing fits.** If no model shape fits a filter within 2 dB where the measurement is
reliable, the skill gives the picture `model = "measured"`, which draws the measured
response stored with the skin and interpolates between the measured parameter values, and
says so in its report.

## What a skin says

A skin returns one table of data. It has no functions: no hooks, no drawing code. What
can't be said with the fields below needs a new widget or field in Rust, which keeps every
skin fast and consistent.

```lua
-- skins/acme/ladder.lua (illustrative)
return {
  api    = 1,                        -- the skin format it's written for
  skin_version = 3, author = "A. Name",
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
                           cutoff = "cutoff", resonance = "reso", drive = "drive" },
                 measured = true },
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

  buttons = {                        -- the skin's row of display buttons ("Controls")
    [1] = { choose = "mode", value = "LP24" }, [2] = { choose = "mode", value = "LP12" },
    [3] = { choose = "mode", value = "BP" },   [5] = { choose = "mode", value = "HP" },
  },
}
```

### Parameters

A skin names the parameters it uses once, under its own names, and the rest of the file
uses only those. A name used anywhere else in the skin that isn't declared under `params`
is an error. Each parameter can be found three ways, tried in this order:

- **`knob`**: the names a knob carries on the slot's pages, optionally with
  `section = "Filter"`. This is how a skin follows NKS pages and the user's own pages
  ("Following the knobs").
- **`id`**: a VST3 ParamID, an Audio Unit parameter ID, or a built-in field; one number,
  or `{ vst3 = …, au = … }` for a plug-in in both formats.
- **`name`**: the parameter's title, for plug-ins with no stable ids.

Parameters are resolved again whenever the slot's pages change or the plug-in reloads, and
the result is kept only in memory. A parameter that doesn't resolve leaves its widget input
at its default and its knob cell showing `?`. Nothing is guessed.

Every write goes through `layout::Edit::PluginIn` or `Edit::Param`, as a knob turn does
today, so Auto-write, Lock, macros and undo all see it. A skin adds no write path.

### Real units: taper or calibration

A curve needs 1.2 kHz, not 0.43. Two ways:

- **A declared taper:** `{ "log", 20, 20000 }`, `{ "lin", a, b }`, `{ "exp", a, b, curve }`,
  `{ "db", "-inf", 12 }` (minus infinity written as a string, since Lua has no literal for
  it), or `{ "table", {0, 20}, {0.5, 800}, {1, 20000} }`.
- **`calibrate = true`:** when the parameter resolves, DECKARD reads its `Params::text` at
  64 normalised points, spread over UI frames, and parses the numbers and units
  (`"1.20 kHz"`, `"-6.0 dB"`, `"120 ms"`, `"1/8 D"`). The table is kept in memory with the
  binding and read again when the binding changes. It isn't written anywhere. If the text
  doesn't parse, the skin is reported and must declare a taper.

Until a calibration is done (a few frames), a picture that needs it uses its parameter's
declared taper if it has one as well, and otherwise shows the plain knob cells on that
screen. It never draws a wrong or flat curve while waiting.

The knob cell always shows the plug-in's own text, so a wrong taper bends the picture,
never the number. The text is the knob's base value; the modulated position shows on the
cell's bar ("Modulation").

### Value maps

A widget input is a parameter's name, a constant, or a small table expression:

```lua
cutoff = { "add", "cutoff", { "mul", "env", 0.5 } }     -- in the input's unit: here Hz
shape  = { from = "mode", map = { ... } }
```

Arithmetic works on real units, after the taper: `"cutoff"` above is in Hz, and `"env"`
in its own unit (−1..1). Operators: `add mul min max clamp lerp map choose`. They compile
to a small expression tree that Rust evaluates each frame.

### Pictures and pages

`pictures` names the pictures a skin can show. Each has a widget, its inputs, and
`needs`: the inputs it can't do without.

- **A skin's own pages** say which picture each screen shows (`left`, `right`) and what
  the eight knobs are. An empty knob is `false`, since a `nil` would end the Lua list.
  They're shown when the slot has no pages of its own, or the user asks for them
  ("Following the knobs").
- **The slot's pages** (an NKS preset's, the user's, automapping) say nothing about
  pictures, so each screen shows the first of the skin's pictures whose `needs` are all
  on the four knobs under that screen ("Following the knobs").

That's the only way to place a picture. Templates fill `pictures` and `pages` for a
`kind`, and a skin can override any of it.

### Templates

| `kind` | Left screen | Right screen |
|---|---|---|
| `filter` | response curve | what is moving the cutoff: its base and modulated position over the last few seconds, and the routes doing it |
| `eq` | the whole curve with its bands; the encoder picks a band | the chosen band close up: its frequency, gain and Q large, the spectrum around it |
| `dynamics` | transfer curve, the input level riding it | gain reduction history |
| `delay` | the repeats as a timeline, left and right lanes | the feedback path: its filter curve and how fast repeats die |
| `reverb` | the decay: level over time, pre-delay as a gap before it | damping: the tail's tone as a curve over frequency |
| `synth` | per section ("Synths") | per section |
| `drum` | the voice's shape (pitch and amplitude envelopes) | hit history |
| `utility` | meters and phase | stereo width |
| `generic` | big knob cells, four a screen | big knob cells, four a screen |

`generic` also recognises common knob names on the page it shows *(builder's call made in
design)*: a screen whose knobs include a cutoff and a resonance ("Cutoff", "Freq",
"Resonance", "Reso", "Q") shows a filter curve, and one whose knobs include an attack,
decay, sustain and release shows an envelope. Each needs its parameters to calibrate from
their text, since `generic` declares no tapers; where they don't, the cells stay. So an
NKS plug-in with no skin of its own still gets pictures for the pages its presets map.

### Widgets

Every widget takes bound inputs and its colours and type from the theme, and has a **flat
look first**: lines and solid fills in the theme's colours, which reads on every theme,
the two-tone `lcd` one included. Glow, soft fills and accent tints are a theme's extras
(`themes/maschine.lua` turns them on), never needed to read a picture. Each widget draws
at most three layers at once.

| Widget | Inputs | How it reads |
|---|---|---|
| `filter` | shape, cutoff, resonance, drive, slope, mix | Analytic response for SVF and ladder shapes, written for the widget (the engine's own filters are biquads). Three layers at most: the curve with a flat fill under it; the post spectrum under that; and one more, the modulated curve while modulated, else the measured response while the cutoff knob is touched or where it differs from the model by more than 3 dB |
| `motion` | a parameter | Its base value as a line and its modulated value as a second line over the last few seconds, newest on the right, with the routes moving it named beside |
| `eq` | bands: frequency, gain, Q, type, on | Each band a dot sized by Q, the sum curve; a band's own curve only while its knob is touched |
| `band` | one band's parameters, the slot tap | Large numbers, the spectrum around the band, the band's own curve |
| `envelope` | ADSR, DAHDSR or breakpoints, curvature, amount | A dot per sounding note moving along it ("Modulation") |
| `lfo` | shape, rate, phase, depth, sync | The shape scrolls under a fixed cursor |
| `scope` | the slot tap | The last frame, steady when the pitch is |
| `spectrum` | the slot tap, pre or post | Log frequency, smoothed, with peak hold |
| `oscillators` | up to four sources: waveform, octave, tune, pulse width, level; noise level | Each source's cycle faint, their sum bright |
| `wavetable` | position | The live output's pitch-locked cycle, with the position as a bar under it |
| `partials` | partial frequencies, levels and decays (declared or measured tables) | Each partial a vertical line that rises on a note and falls at its own rate |
| `path` | the voice's sections and which are on | Labelled blocks for oscillators → mix → filter → amp → effects, the section the page edits filled |
| `mods` | the synth's envelopes and LFOs (by parameter) and DECKARD's routes to it | Small multiples, one per source, each with its shape and playhead |
| `transfer` | threshold, ratio, knee, makeup | The input level as a dot riding the curve |
| `gr` | gain reduction (from the slot tap: level in against level out) | A history scrolling beside a bar |
| `taps` | time, feedback, spread, sync | Repeats as decaying bars |
| `decay` | pre-delay, size, decay | Level over time on a dB scale |
| `damping` | damping, tone | A curve over frequency |
| `meter` | the slot tap | Peak hold |
| `value` | one parameter | Large numerals, the unit small |
| `knobs` | the page's knobs | The cells over the knobs. Always drawn ("Knob cells") |
| `steps`, `euclid`, `roll` | as `EuclidView` and `RandomView` show today | Ported as widgets so a MIDI plug-in's skin can use them |
| `image` | a PNG beside the skin | For a user's own skins. Shipped skins have none |

Adding a widget is a Rust change with a golden-image test. That's on purpose: it keeps the
look consistent.

## The screens

The MK3 has four knobs under each screen and four display buttons above it. So each screen
has three bands, lined up with the hardware, and its own picture:

```
 ┌─────────── left screen 480 ───────────┐   ┌─────────── right screen 480 ──────────┐
 │ [LP24]   [LP12]   [ BP ]   [SKIN]      │   │ [ HP ]   [    ]   [    ]   [    ]     │  28  buttons
 │ LADDER · FILTER        CUTOFF 1.20 kHz │   │ CUTOFF MOTION              SKIN 1/2   │
 │ ──────────────────╮                    │   │ ────────╮      ╭──────╮      ╭──      │
 │                   │╲                   │   │          ╰────╯        ╰────╯   mod    │ 172  picture
 │  ▁▂▃▅▆▅▃ spectrum ▂ ╲                  │   │ ─────────────────────────────── base   │
 │ ████████████████████╲____              │   │ LFO 1 → cutoff                        │
 │ 20     100     1k      10k             │   │                          now ┘         │
 ├─────────┬─────────┬─────────┬─────────┤   ├─────────┬─────────┬─────────┬─────────┤
 │ FILTER                                │   │ FILTER          │         │ OUTPUT    │  72  knob cells
 │ CUTOFF  │ RESO    │ DRIVE   │ MODE    │   │ ENV     │         │         │ MIX       │
 │ ▰▰▰▰▰▱▱ │ ▰▰▰▱▱▱▱ │ ▰▰▱▱▱▱▱ │ LP24    │   │ ▱▱▱▰▰▱▱ │         │         │ ▰▰▰▰▰▰▰   │
 │ 1.20 k  │ 42 %    │ 3.0 dB  │         │   │ +18 %   │         │         │ 100 %     │
 └─────────┴─────────┴─────────┴─────────┘   └─────────┴─────────┴─────────┴─────────┘
```

- **Nothing crosses the gap.** Each screen is its own 480 × 272 picture.
- **The knobs under a screen move what that screen shows.** A skin's pages put a picture's
  parameters on the knobs beneath it, as Push 3 groups a device's sections. On the slot's
  own pages, pictures are chosen to fit the knobs ("Following the knobs").
- **The header** has the page's name and source on the right (`SKIN 1/2`, or the slot's
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
   its own curve appears, and a filter shows its measured response.
2. The rest of the picture dims to about 50 %. Names, values and the axis never dim.
3. The header shows the parameter's name and value large, where it covers nothing.
4. On letting go, it returns after 180 ms.

The other screen keeps its picture. The template says what each parameter lights; a skin
can change it with `focus = { cutoff = "handle", reso = "peak" }`.

**How touch is read.** The MK3 already reports a finger on knob *n* as `BUTTON_CAP_n` down
and its lifting as up. The surface passes those events on to the page and `global.lua`
exactly as now, so Set Macro, double taps and anything a script does with them keep
working, and also tells the skin layer, which only changes what's drawn. Focus starts
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
  curve's), which reads on the `lcd` theme, doesn't band on the screens' 16-bit colour, and
  keeps the MK3's run-length encoding effective. A theme can ask for soft fills; they're
  drawn without dithering, which would break the encoding's runs.
- **Still when nothing moves.** With no audio, modulation or touch, the frame doesn't
  change and nothing is sent.

### Getting frames to the screens

Each screen is 480 × 272 at 16 bits, 261 KB a frame uncompressed. Today a moving curve
anywhere on a screen resends every row from its top to its bottom, so a 60 fps picture on
both screens could need tens of MB/s, more than the MK3's link is known to sustain.

- **Measure first** (phase 0): the real rate of full-screen and partial transfers on an
  MK3, and how long each transfer blocks the screens thread.
- **Send only what changed:** the compositor tracks dirty rectangles (the picture area,
  each knob cell, the header), and the screens thread sends each as its own transfer with
  its x, y, width and height, which the display protocol already supports.
- **30 fps for pictures** *(builder's call made in design)*, which a curve and two
  knob cells fit easily. Knob cells and touch focus are drawn on the next frame after a
  change, so the hand never waits more than 33 ms. If phase 0 shows more room, the rate
  goes up.

## Controls

The Control page already uses most of the buttons around the screens: display buttons 1–3
pick MASTER, GROUP or SOUND, 5 and 6 step through the slots, SHIFT turns them into the
slot's edits (EDIT, INSERT, MOVE, BYPASS, REMOVE) and the knobs into fine adjustment, LEFT
and RIGHT page, the jog wheel steps slots and opens the browser, and the Smart Strip has
its own modes. A skin keeps all of that and adds one layer:

- **Display button 4 is SKIN** on a skinned slot (it's free on the Control page today).
  Pressing it swaps the top row between the Control page's own buttons and the skin's
  `buttons`, lit while the skin's row is shown. In the skin's row, button 4 stays SKIN, so
  the way back is the same button; the skin has buttons 1–3 and 5–8. Leaving the slot
  returns to the Control page's row.
- **SHIFT means what it means today** in either row: the slot's edits on the buttons,
  fine adjustment on the knobs. While SHIFT is held, the button labels change to the
  slot's edits, as now; the pictures stay.
- **LEFT and RIGHT always page** through one list of pages ("Following the knobs").
- **The jog wheel and the Smart Strip are the Control page's,** unchanged.

## Following the knobs

The slot's pages are the knob-to-parameter mapping DECKARD already keeps for every
plug-in slot, saved with the project as Maschine saves it: each knob's parameter id,
section and name. Something fills them, and that's all a skin needs to know:

- **An NKS preset loaded through the browser** writes its own pages, eight named knobs a
  page by section. In some products (Kontakt libraries, Reaktor ensembles) what a
  parameter id means changes from one preset to the next, and the preset's pages are the
  only place that says what each knob now is.
- **The user** edits pages on the slot, as in Maschine.
- **Automapping** gives the first 128 automatable parameters, eight a page, when nothing
  else has.

A skin stores nothing per preset. Its pictures work from the pages the knobs show as they
are at that moment:

- **Parameters resolve through the knobs first.** A parameter with `knob = { "Cutoff",
  "Filter Cutoff" }` resolves to whichever parameter the slot's pages put under a knob of
  that name (in that `section`, if given), then by `id`, then by title. It looks at the
  slot's own pages even while the skin's pages are shown, so a skin for a host plug-in can
  lay out its own pages from what an NKS preset's knobs are. When the slot's pages change
  (another preset loaded, the user's edit), everything resolves again.
- **Pictures follow the knobs.** On the slot's own pages, each screen shows the first of
  the skin's pictures whose `needs` are all on the four knobs under it, so the picture
  above the knobs always shows what those knobs move. A screen whose knobs fit no picture
  shows `generic`'s, or the plain cells.
- **The knobs never disagree with the pictures,** because both come from the same pages.

**Which pages the knobs show.** A skin's pages are a mapping that makes sense for the
plug-in, written once in the skin file. They apply in two cases:

- **The slot has no pages of its own** (no NKS preset was loaded, and the user hasn't
  edited any): the skin's pages replace automapping.
- **The user ticks "Use the skin's pages"** against the plug-in in **Preferences ▸
  Hardware ▸ Skins**: the skin's pages replace the slot's own on every slot of that
  plug-in, NKS pages included. It's off by default *(builder's call made in design)*.

Otherwise the knobs show the slot's own pages, as today, with pictures chosen to fit them.
Whichever pages are shown, automapping's remaining parameters follow them, eight a page by
section where the plug-in reports sections, so every parameter stays reachable. The header
says `SKIN` on the skin's pages.

Nothing about this is saved in the project. The checkbox is a preference; the slot's own
pages stay saved as they are, and come back when it's unticked. Editing a page on a slot
that shows the skin's pages (with the existing page editor) saves the edited pages as the
slot's own, as any edit does today, and they're what that slot shows from then on unless
the checkbox is ticked.

**When the preset changes inside the plug-in** (chosen in its own window, say), DECKARD
can't see it in this version: it doesn't act on a plug-in's restart flags yet, and some
products report nothing. The knobs keep the slot's pages, and the pictures follow those
knobs, so the two still agree with each other. Acting on `kParamTitlesChanged` and
program changes is "Later".

## The slot tap

To measure an effect, the engine needs the audio just before and just after it.

- A new engine module, `slot_tap.rs`, keeps two stereo rings (before and after) of 4096
  frames, written with stores to atomics only: no lock, no allocation. It has nothing to
  do with the recording `Tap`, which stays free for Sampling.
- **One slot is tapped at a time:** the one the MK3 shows. The UI thread sets the chain and
  slot in an atomic. The engine compares it once per block, and only that slot copies its
  input and output.
- A worker thread (neither the UI thread nor the screens thread) gives the spectrum
  before, the spectrum after, and the **measured response**: the cross-spectrum of before
  and after over the spectrum before, averaged over about a second of overlapping 4096-
  point windows, with the after ring shifted back by the plug-in's reported latency so the
  two line up. It's drawn only where there's signal before and the two stay coherent;
  elsewhere it isn't drawn at all, rather than drawing noise.
- **Effects only.** An instrument has nothing before it to compare. A plug-in the skin or
  the probe marks nonlinear or moving (`measured = false`) shows only the spectrum.
- A plug-in's own Mix knob shows in the measured curve. That's intended: the picture shows
  what really happens.
- The tap runs only while a picture that uses it (`measured`, `spectrum`, `scope`,
  `meter`, `band`, `gr`) is on screen, and costs nothing otherwise. *(builder's call made
  in design)* No button turns it on or off.

## Modulation

- **Where the parameter really is:** route travels give each bound parameter's modulated
  position, as `App::mod_position` does for knobs today. A picture draws the base and the
  modulated position; `motion` shows the last few seconds of both.
- **Knob cells** show it as a second mark on the bar.
- **Chains:** route travels cover sounds' routes today, so a plug-in on a group or the
  master shows its base only, until they cover those chains too.
- **Envelope playheads:** DECKARD's own instruments and the Sampler can publish their
  envelopes' phase. A hosted plug-in can't, so its playheads are modelled from the slot's
  note events and the declared times, and drawn hollow, so they never pretend to be the
  plug-in's own envelope.
- **MIDI plug-ins** (`docs/midi-input-plugins.md`) get `steps` and `roll` fed from those
  note events.

## Synths

Effects mostly do one thing, so one picture per screen covers them. A synth is a voice made
of sections (oscillators, a mixer, a filter, an amp, envelopes and LFOs moving them,
effects after), with tens to hundreds of parameters. Push 3 handles this by showing one
section at a time with an overview to move between them. The `synth` template does the
same.

### Sections and pages

A synth skin declares its sections, each with its parameters and its pictures:

```lua
kind = "synth",
sections = {
  { name = "OSC",    left = "oscillators", right = "spectrum", knobs = { ... } },
  { name = "FILTER", left = "response",    right = "contour",  knobs = { ... } },
  { name = "AMP",    left = "amp_env",     right = "scope",    knobs = { ... } },
  { name = "MOD",    left = "mods",        right = "lfo1",     knobs = { ... } },
},
```

- **The overview** is the skin's first page: `path` on the left, `mods` on the right, and
  the knobs on the synth's eight most-played parameters (cutoff, resonance, envelope
  amount, attack, decay, the macros…).
- **A section's pages** follow it, with that section's pictures and knobs.
- **Getting around:** LEFT and RIGHT step page by page, through the overview, the sections
  and then the rest. In the skin's button row ("Controls"), a button for each section
  (up to seven) jumps to its first page; pressing the lit one again returns to the
  overview.
- **Everything else** stays reachable: automapping's pages for the parameters no section
  uses.
- **NKS presets.** The overview and sections are the skin's pages, so they follow the
  same rule as any skin's ("Following the knobs"). A synth loaded from an NKS preset shows
  the preset's pages, with pictures chosen to fit their knobs, unless the user ticks **Use
  the skin's pages** for it.

### Three hard cases

These three show what a synth skin has to deal with, and they're the probe's and the
skill's test cases. All three are open source, so their skins can ship and CI can measure
them.

**A classic subtractive synth (OB-Xd).** Oscillators, a mixer, a multimode filter with its
envelope, an amp envelope. Its pictures are the easy kind: `oscillators` draws the
waveforms and their sum from the waveform, octave and tune parameters, `filter` is the
curve with the filter envelope's amount as its modulation, and the envelopes are plain
ADSR.

**An FM synth (Dexed).** Six operators, their ratios and levels, an algorithm. There's no
filter, and no one knob moves the spectrum in a way a formula from the knobs would draw
fairly. Its picture is `partials` from **measured tables**: the probe plays notes across
the operators' levels and ratios and records the partials' frequencies and levels, and
the widget interpolates between them. The skin says `model = "measured"`, so no one takes
it for the synth's own maths. `path` shows the algorithm as labelled blocks.

**A wavetable synth (Vital).** Its wavetables are part of the preset, not parameters, and
its own envelopes, LFOs and modulation matrix live inside it, invisible to a host. So:

- **The wavetable picture is the live output's cycle,** pitch-locked, with the position as
  a bar. A stack of the table's frames is "Later": it would need the frames, and rendering
  them takes a second instance of the synth.
- **Its own modulation is invisible,** and the skin says so rather than inventing it:
  `mods` shows Vital's envelopes and LFOs only through the parameters that set their
  times and rates, plus DECKARD's own routes. The live tap (spectrum, scope) shows the
  result of everything.
- **Macros** are parameters like any other, and go on the overview's knobs.

### Products inside a host plug-in

Kontakt and Reaktor are one plug-in to a host, whatever is loaded in them, and their
parameter ids mean different things in different products. A skin can't match the product
(DECKARD doesn't keep which one is loaded), and doesn't need to: an NKS preset's pages say
what each knob is, and a skin for the host plug-in binds by `knob` names only, with no ids.
`generic` does the same for the commonest names. So a product loaded from an NKS preset
gets pictures where its pages have the knobs a picture needs, and plain cells elsewhere.

### Measuring a synth

The probe's `note` does for instruments what `response` does for effects, with more care,
because a synth's sections hide each other:

- **Isolate first.** The skill measures in an order that lets it isolate each section: amp
  envelope first (sustain to full, so later notes hold steady), then the mixer (each
  source alone), then each oscillator, then the filter, then the modulation. Each step
  uses `--set` to hold the others still. It starts from the plug-in's initial state, or a
  preset given with `--state`, and records which in the test data.
- **Oscillators:** one cycle of a held note, pitch-locked, at each waveform choice and
  pulse width, checked against the `oscillators` widget's shapes. Tune and octave from the
  measured pitch.
- **Filters inside a synth** have no input to sweep. The probe plays a sawtooth (its
  harmonics fall at a known 1/n) or a noise source if the synth has one, and reads the
  filter's response from the output spectrum against that, at each cutoff and resonance
  value.
- **Envelopes:** level over time of held notes at each time value, which gives the taper in
  milliseconds, or confirms the plug-in's text.
- **FM and additive models:** the partials' frequencies and levels, as tables.

## Threads and data flow

```
 UI thread                          skin compositor thread               MK3 screens thread
 ─────────                          ──────────────────────               ──────────────────
 on focus, reload or page change:   each frame (up to 30 a second):      each dirty rectangle
   load and check the skin            read the parameter cells, route      → its own transfer
   resolve parameters                 travels, slot tap spectra
   compile value maps                 evaluate value maps
   → Arc<SkinScene>  ─────────────►   lay out, draw widgets ─────────────► frame + dirty list
 each UI frame:                       over the page's own layer
   publish bound parameters'
   values and text (64 at most)
```

- **Hosted plug-ins are read on the UI thread,** where their reads belong. It writes the
  bound values into a small array of atomics and their text into a double buffer, once per
  UI frame. The compositor never touches a plug-in.
- **The page still draws around the skin** with Lua `draw()` (anything the page adds).
  That picture becomes the static layer, and the compositor draws the pictures, header
  readout and knob cells over it. With no skin focused, the compositor doesn't run.
- **Budget:** about 4 ms of CPU a frame for 960 × 272. Over budget, the compositor lowers
  its rate and logs it once.

### CPU, not GPU

The MK3 needs pixels on the CPU for the USB encoder anyway, a GPU readback each frame adds
latency, and 260,000 pixels is small. The existing rasteriser, extended, or `tiny-skia`
will do. *(builder's call made in design)*

## Checking a skin

- **In the app:** a skin's errors, undeclared or unresolved names, bad tapers and failed
  calibrations go to the script-error report, with lines.
- **For development:** an ignored test renders each shipped skin's pages to PNGs at default
  values, at the extremes, and with touch focus on each knob, on the default and `lcd`
  themes, into a directory given by an environment variable. It loads a skin against a
  saved parameter dump, so a skin can be checked without the plug-in installed.
- **Golden images per widget** in `crates/surface`: new infrastructure (phase 2), built on
  `render_screens`.

## Documentation

`docs/controller-api.md` gains a **Skins** chapter: where they live, matching, `params`,
tapers and calibration, value maps, pictures and pages, following the knobs, templates,
every widget's inputs, the SKIN button, installing, sharing, which skin wins, versions,
and a worked example. `DEVELOPMENT.md` says how to add a shipped third-party skin: the
criteria, the parameter dump, the golden images, the black-box rule, and the
`plugin-skin` skill and `plugin-probe` that make most of it. `docs/controller-plugins.md`
points to it.

## Build order

| Phase | Delivers | Proof |
|---|---|---|
| 0 | Measure the MK3's display link: full and partial transfer rates, and how long transfers block; rectangle transfers in `deckard_mk3::Displays` | Numbers recorded in this document; a moving 120 × 72 cell costs its own pixels, not the screen's |
| 1 | Compositor thread, parameter cells, dirty rectangles, golden-image tests; `filter`, `eq`, `band`, `transfer`, `value` and knob cells drawn in Rust from the data `ControlView` already has, on every theme; touch focus from `BUTTON_CAP_n`. DECKARD's own Filter, EQ and Compressor pictures move to it, and `eq_curve`'s special cases go | Built-in pictures match on default and `lcd` themes; frame time under 4 ms; nothing sent when still; surface tests that a `BUTTON_CAP_n` press and release focus and unfocus while still reaching the page (Set Macro, double tap), and that three touches light nothing |
| 2 | Skins for hosted plug-ins: loader, matching, checking, `params` by `knob`, `id` and title, resolved again on page changes; declared tapers; value maps; pictures and `needs`; skin pages when the slot has none of its own or **Use the skin's pages** is ticked; the SKIN button row; `generic`; fallback and `SKIN` badge | Unit tests on resolution and maps, including an NKS preset's pages replacing the slot's and the checkbox swapping skin and slot pages; MK3 check with one hosted plug-in |
| 3 | `plugin-probe` `list` and `dump`; calibration from `Params::text`, in memory | Tests on recorded text dumps from the shipped third-party plug-ins |
| 4 | Slot tap, worker, measured response, `spectrum`, `scope`, `meter`, `gr` | No allocation on the audio thread; the built-in Filter's measured response matches its model within 1 dB where there's signal; latency alignment tested with a delayed copy |
| 5 | Everything else: modulation and `motion`; `envelope`, `lfo`, `taps`, `decay`, `damping`, `steps`, `roll`; the `synth` template, sections, overview, `oscillators`, `wavetable`, `partials`, `path`, `mods`; distribution (**Dump Plug-in Parameters**, **Install Skin…**, **Preferences ▸ Hardware ▸ Skins**, `api`, `skin_version`, `based_on`, dumps in CI); the probe's `response`, `sweep` and `note`; the `plugin-skin` skill; `model = "measured"`; the first shipped third-party skins | Golden images; install and precedence tests; every shipped skin passes against its dump; the probe classifies DECKARD's own Filter, EQ, Delay and Compressor parameters correctly and calls the EQ linear, the Compressor nonlinear and the Chorus moving; the skill writes skins for OB-Xd, Dexed and Vital whose models fit their measurements |

Phase 1 gives DECKARD's own effects their new pictures with nothing new to learn; phase 2
opens it to every plug-in.

## Later

- Acting on a plug-in's `kParamTitlesChanged` and program changes, so knob names follow a
  preset chosen inside the plug-in's own window.
- Route travels for groups and the master, so their plug-ins show modulation.
- The Smart Strip as a skin's control, alongside its PITCH, MOD, PERFORM and NOTES modes.
- A wavetable stack drawn from the table's frames, once there's a way to get them without
  a second instance.
- Event hooks, if a skin ever needs one that a field can't express, once scripts have a
  time limit.
- A shared skin repository that **Preferences ▸ Hardware ▸ Skins** can browse and install
  from.
- The same `SkinScene` could draw in the app's own Control area, so a plug-in has the same
  face on screen as on the controller.
- Other controllers with screens get skins by giving the compositor their screen sizes and
  knob positions.
