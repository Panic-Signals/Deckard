# Jam on-screen overlay: design

## Intent

A Maschine JAM has no screens. Its encoders' settings live in an overlay Maschine draws
in a small window of its own over the app, and it appears when you touch an encoder. The
JAM works in DECKARD without one: touching an encoder walks a line in the status bar
instead, which cannot show a page of eight settings, their headers, or which one the
encoder is on.

This gives the JAM its overlay: the page the controller's script publishes, drawn in a
window that floats over whatever view is showing and disappears when the page does. The
pages themselves are the JAM manual's, and the look is the one Maschine's own overlay
has — because the JAM's owner already knows it, and a second arrangement teaches nothing.

## Goals

- Every page the JAM manual gives an overlay: the eight-plus-one `Scale` split, the
  Arpeggiator's three pages, Note Repeat, Swing, Tune, Lock, the grid, recording's
  settings, the tempo, Perform FX, and the pattern's Variation page.
- The page appears on the control that owns it, and disappears the way that control's
  page does — a button's page on its timer, a mode's page when the mode ends.
- The encoder drives everything: a turn moves along the row, a press brackets a setting,
  a turn changes it. The window takes no clicks except its own bar, which moves it.
- The settings a page shows are current — including on a cold boot, before any menu has
  asked for them.

## Non-goals

- The desktop `BROWSE` overlay and `INSTANCE`: a different window with its own chrome.
  Neither is here; the library stays the app's own browser panel.
- NI-scripted controllers' overlay path, which stays in `lua-host` beside this one.
- Mouse interaction inside the window: no hover, no per-slot animation, no editing by
  click. The strip belongs to the encoder.
- A shared page framework: the composer stays the Jam's until a second device needs the
  same pages.

## Anti-goals

- **No page drawn into the app's main window, and no docked fallback.** One strip, one
  window. A row docked in the app was the earlier arrangement, and it needed a rule to
  ignore the mouse — which is what a separate window makes unnecessary.
- **No mark in the bar.** The reference has NI's wordmark there; DECKARD is not Maschine
  and does not wear another company's logo.
- **The host never decides what a setting means.** No parameter addressing in Rust, no
  routing by page type: every ENCODER event reaches the script, which owns the page.
- A project with no Jam and no overlay is unaffected: the overlay adds no state to the
  format, and every page's settings are read from what the project already holds.

## The page

```lua
deckard.oso({
  title, page, pages,   -- "Scale 1/2": `pages` counts only pages that exist
  slots,                -- left to right; `false` is a hole, and keeps its column
  focus,                -- 0-based index into `slots`, holes counted
  editing,              -- whether the slot at `focus` is being changed
})
```

A slot is `{ value, sub, group }`: `value` the display text already formatted
(`Horizontal`, `C3`, `75 %`), `sub` the small name under it (`Direction`, absent leaves
the row blank), `group` the spanning header over its run (`PAD LAYOUT`, absent leaves
none).

- **`pages` never counts a page that cannot exist.** The Arpeggiator is always three
  (8 / 7 / 2); `Scale` is eight plus one, so it is two while the keyboard layout is
  isomorphic and one otherwise. A `2/2` that renders nothing is a bug in the composer.
- **A hole is `false` and keeps its column.** Holes are the composer's statement that a
  value is unavailable, never the renderer's inference.
- **`focus` and `editing` are the script's on every page.** The host draws them and keeps
  no second copy. In particular it does not settle a cursor onto a filled slot: a cursor
  on a hole underlines nothing, and that is honest.
- **Whose job is it to avoid a hole?** The stepping page's, because it steps: the
  keyboard's page settles its cursor when the pad layout changes the fill under it. A
  menu's cursor moves one place per notch and never lands on an item that does not show.
- **A page a script drew before the host applied what it asked for is stale**, and the
  host fixes it in the same frame: after applying a script's requests it ticks the script
  once more over the state it now holds. That is why Perform FX shows the mode it just
  changed to. It runs only when something was applied, so nothing animates twice.

## The Variation page

The pattern's Variation page was the one static type with nothing behind it. It publishes
Maschine's page: **Mode** (`Humanize` | `Random` | `Euclid`), **Apply** as a button with no
sub-label, then Velocity Lo, Velocity Hi and Step — and, while the Mode is `Random`, a
second page of Probability, Note Lo, Note Hi, Note Count and the three distributions.

| Mode | Engine |
|---|---|
| `Humanize` | The document's variation pass: velocities into the band, the time shift, the length scale. Nothing is created. |
| `Random` | The generator's Random — the same call the app's Generate ▸ Random makes. |
| `Euclid` | The generator's Euclidean mode. **DECKARD's own**; the reference has two modes. |

Each setting lives where its engine reads it: probability, note range and chords in the
generator's module; the velocity band, note length, time shift and the three distributions
in the sound's DECKARD field. They are the **sound's**, saved with the project, and unset
the engine behaves as it did before the page existed.

One reading is still ours: the reference does not say whether Humanize's velocity band
replaces a velocity or bounds a deviation from it. This build clamps into the band.

## Who owns what

|  | Script | Host |
|---|---|---|
| Page membership, holes, page count | owner | — |
| Cursor and edit flag | owner | draws |
| What an ENCODER press means | owner | hands it over |
| What an ENCODER turn means | owner | hands it over |
| Value and header text | owner | draws |
| Cell geometry, clipping, colours, the bar | — | owner |
| Settings a page depends on | reads | publishes as they are |

The cost of that ruling, taken deliberately: a slot's behaviour is only as testable as the
script. That is why the obligations at the end of this document sit on the *page*, not on
the host.

## When a page goes

Two lifetimes, and the reference shows both:

|  | static | timed |
|---|---|---|
| pages | `GRID`, `TEMPO`, `REC_MODE`, `RANDOMIZER`, `PERFORM_FX` | everything else — `SCALE`, `ARP_REPEAT`, `SWING`, `TUNE`, `LOCK`, `NOTES` |
| opened by | its own key or chord | its trigger — an encoder touch or its own button |
| closed by | that key again, the controller's page changing, the device going | the idle timer, an explicit close, a page change |
| idle timer | never armed | armed on show: 70 ticks, about two seconds |

Perform FX is static even though its settings look like a menu: its own **press** closes
it, not the timer. `NOTES` opens its page but never arms the timer on the press. A page
is held up while its own button is held, while the browser strip is touched, while the
mouse drags the window, or while edit mode is on.

**A page change closes every static page**, which needs the app's controller page as a
*fact*: `View.page` reaches the script as `s.surface_page`, and the script records the
page each static menu opened on. Without that fact the rule is unreachable — the device's
own page is its runtime's, not the controller's.

**A view change draws nothing new.** The one exception the rig found is worth keeping:
`SHIFT` + `PAD_MODE` opens the keyboard's scales, because that switch *is* the keyboard
view's page, rather than there being a page of the viewer's own for "the keyboard view is
selected" — one existed once, and it hid the fact that the eight-plus-one split belonged
to `Scale`.

## Settings a page reads

The script reads `deckard.state().settings.values`, and the host publishes it as a
**projection**, not a push: the view is built from the app's settings, so a value changes
there when it changes here.

The three triggers this replaced — the Settings page opening, a Jam connecting, and a
request from the script — each guessed at who reads the view, and on a cold boot none of
them fired: the `Scale` page's Mode slot drew as a hole until a menu happened to ask for
it. **A lazily built view plus per-consumer guards fails as a missing slot, never as an
error**, which is the worst shape a fault can take.

## The window

A second eframe viewport, not a docked row: chromeless, always on top, never activated on
show, one window that exists exactly while the script publishes a page.

- **Its bar carries the page's name and its counter** — `Scale  1/2` — and nothing else.
  The body keeps eight columns and no title row.
- **Eight columns always, `width / 8`.** A page of one slot fills the first and leaves
  seven empty columns with their rules. Nothing stretches, and nothing shrinks.
- **The bar is the handle.** Pressing it drags the window; the body has no click target,
  no hover and nothing to hit, so a stray click on the slots does nothing.
- **Its place is an offset from the app window's top-left**, saved when the window reports
  a rect that differs from the one commanded (a drag, a window manager's placement, a
  display change) and never in the frame a position was commanded. An offset rather than
  a point means the window comes back beside the app on whichever display the app opens
  on. `KEEP` points of it are held inside the app's display on each axis, so an offset
  saved on a wider desktop still lands somewhere usable; nothing saved centres it over
  the app.
- **Two lifetimes stay untangled**: the script owns the page's, the app owns the
  window's. The window exists while the strip lookup returns a page; when the script
  publishes none, the viewport is simply not called and eframe drops it. There is no
  "mode", and no shown flag to drift from the script's.

The strip's pixels are measured, not copied: the reference's binary keeps no strip
metrics (its window state and styling are runtime), so the numbers are read off captures
of the running app — bar 28 points, body 144 (header 40, value 40, sub 40, pad 24), type
11.5 / 21 / 13, a rule under each column's sub-label with the focused one at 2.5 points
white, and editing as inset corner brackets.

## Platform facts (eframe/egui 0.36.2)

- `ViewportBuilder::with_mouse_passthrough` exists and is applied as
  `window.set_cursor_hittest(!passthrough)`.
- `set_cursor_hittest(false)` works on macOS, on Windows and on X11; **on Wayland winit
  returns an error**, which egui-winit logs and otherwise ignores. The window will take
  clicks there. Nothing above that layer can fix it, so the window is built to be harmless
  when it does: no click target, no hover, no drag handle outside the bar.
- `show_viewport_immediate` is already used once in this app (the splash), and its
  semantics are the lifetime rule above: not calling it closes the viewport.
- `always_on_top` is `with_window_level(WindowLevel::AlwaysOnTop)`.

## Boundaries

- **The renderer must not grow a feature the contract cannot feed.** No hover, no mouse,
  no per-slot animation.
- **Clean room.** The behaviour here comes from the JAM manual, the panel's printing and
  observing the running app; the pages' labels and ranges were recovered from captures,
  not from NI's code. Nothing in this feature is a translation of NI's software, and the
  mark that would suggest otherwise is not drawn.
- **NI-scripted controllers coexist.** Their `setOSOType` / `addParameter` path stays in
  `lua-host`; the two shapes are separate on purpose.

## Tests

Each obligation is a test with its own name:

- A published page renders, and the cursor and the edit mark follow it —
  `jam_strip::tests::the_underline_follows_the_published_cursor_wherever_it_moves`,
  `the_edit_mark_is_the_scripts_and_marks_only_the_focused_slot`.
- A cursor whose page's fill varies stays on a slot, and a cursor on a hole underlines
  nothing — `a_page_whose_fill_varies_keeps_the_cursor_on_a_slot`,
  `a_cursor_on_a_hole_underlines_nothing`.
- A menu's turn moves the published focus, and steps over what a page cannot show —
  `jam_tests::menu_cursor_moves_the_published_focus`,
  `a_menu_turn_skips_unavailable_items`.
- A turn republishes the page it changed — `a_turn_republishes_the_page_it_changed`.
- The settings reach the scripts with nothing asking for them —
  `surface_settings_tests::the_scripts_read_the_settings_with_nothing_asking_for_them`.
- A static page dies with its page change, and the tempo's button publishes its page —
  `a_page_change_drops_a_static_oso`, `the_tempo_button_publishes_its_oso`.
- The whole overlay runs with no NI install at all — `jam_oso_pages_need_no_ni_install`.
- The window is sized from the page and does not exist without one —
  `jam_tests::the_strip_is_a_window_of_its_own`, plus `jam_window::tests::*` for its
  geometry, the offset band, and that an unmoved window writes nothing.

`docs/controller-api.md` carries the script-facing half of this document.

## Open questions

- A slot that cannot be turned must not set `editing`; that is a promise the script keeps,
  and it is worth a test the day a composer grows a read-only slot.
- Perform FX's missing-effect case is covered by the extra tick only because adding an
  effect is an `Action`. A request that changed state without one would be stale again.
- Long values still clip mid-word in narrow columns (`Harmoni`). Cell width and clipping
  belong to the pages, not to this contract.
