# Plan — the Jam strip gets its own window (work item 22's demanded plan)

Status: `in progress` · feeds `jam-oso-branch-brief.md`'s "the window" and PR A to
`Panic-Signals/Deckard` · supersedes nothing: the docked strip is what is being replaced.

## What this builds

The strip leaves the main window's `ui` and becomes a **second eframe viewport**: chromeless,
mouse-inert, always-on-top, one window that shows the page the script published and disappears
when the script stops publishing one.

Item 22 promoted the decision, not the work; the brief states the requirements and explicitly
does not plan them. This is that plan.

## Requirements, as the brief states them

1. `show_viewport_immediate` + `with_mouse_passthrough`, not a docked strip with a rule to
   ignore clicks.
2. Chromeless, with the manual's own chrome: the page name at the left of a title bar, the NI
   mark at the right, no buttons. The `n/N` counter moves out of the strip body into that bar.
3. Shown and hidden by the overlay's lifetime — not by a mode: static OSOs die on their key or a
   page change, the rest on the 2 s idle timer. Both already live in `maschine_jam`; the window
   only follows `jam_strip()`.
4. Position persisted, restored on a second monitor without going off-screen.
5. Capture-tested: two-window HIL, one shot of the window alone and one over each view.

## Decisions

**D1 — `jam_strip.rs` keeps no state, and gains no `Strip` struct.** The window is a second
*painter* of the same published page, not a second model. `rows()`, `header_runs()`, `slot_rect()`
and `title()` stay pure functions of `OsoPage`; the only new state anywhere is the window's last
outer rect, and that lives in `Settings`, not in the renderer.

**D2 — the title bar is one more row of the same painter, not a nested `egui::Window`.** A
floating `egui::Window` inside the viewport would drag egui's own window-dragging, shadow and
resize behaviour in, and with mouse passthrough on none of it could be used anyway. The bar is
`BAR_H` points of the strip's own background with a rule under it, the page's name at the left and
the `n/N` counter just after it (`Scale  1/2`) — where the reference puts them, and **no mark at
the right**: DECKARD is not Maschine, and a second company's logo has no business in it (the
reference's own is an NI mark, which the brief transcribed; the human's call is to drop it).

**D3 — the counter moves, and `jam_strip::title` is split to do it.** `title()` keeps its
behaviour for the docked callers that no longer exist, so instead: `name(page)` is the page's
title alone and `counter(page)` is `Some("1/3")` only when `pages > 1`. The bar uses both; the
strip body no longer allocates `TITLE_ROW` when it is drawn inside the window. (The strip's own
`TITLE_ROW` stays for the `paint` path so no existing test changes meaning.)

**D4 — the window takes the mouse, and its bar is what moves it.** *(Reversed 2026-11-21, on the
human's call and the reference's own behaviour.)* This first read the brief literally —
`with_mouse_passthrough(true)` as "the strip belongs to the encoder" — but the reference's overlay
is **movable anywhere on screen** (`work-items/22` says so, and its `isMouseDragging` state only
exists because the window receives presses), so passthrough made a window that could never be put
where the user wanted it. Now: no passthrough, and the bar is a title bar — `Sense::click_and_drag`
on the bar sends `ViewportCommand::StartDrag`. The body still belongs to the encoder: no click
target, no hover, nothing to hit, so a stray click on the slots does nothing at all.

**D5 — the position is a setting, and the drag writes it.** With D4 the window is movable, so the
offset from the main window's top-left (`Settings::jam_strip_pos: Option<(f32, f32)>`) is both read
on show and written when the window reports an outer rect that differs from the one we sent — a
drag, a window manager's placement, or a display change. It is never written in the frame we
commanded a position. The offset, not an absolute point: the strip then comes back beside the app
on whichever display the app opens on, which is the brief's "restored on a second monitor" clause,
and it needs no monitor rectangles (egui exposes a viewport's `monitor_size`, never monitor
origins). `jam_window::clamp_axis` keeps `KEEP` points of the window inside the main window's
display on each axis (its bar on y), so an offset saved on a wider desktop still lands somewhere
usable. Nothing saved centres the window over the main window, as the reference centres over its
parent from live bounds.

**D6 — two lifetimes stay untangled.** The script owns the page's lifetime; the app owns the
*window's*. The window exists exactly while `jam_strip()` returns `Some`. That is one rule, in
one place (`jam_window::show`), and it replaces any notion of a "mode": a static OSO that survives
the idle timer keeps its window, and the moment the script publishes none, the viewport is not
called and eframe drops it.

**D7 — the decoration-free window is what the platform risk is about, so it is proved first.**
`with_decorations(false)` + `with_taskbar(false)` +
`with_window_level(AlwaysOnTop)` + `with_resizable(false)` + `with_active(false)` (never steals
focus on show).

## The strip's metrics, measured off the reference

`docs/superpowers/specs/2026-11-16-jam-overlay-refs.md` §1 says the binary keeps **no** strip pixel
metrics (window state and styling are runtime/style-sheet), so there is nothing to copy: the
numbers below are measured from two 2× captures of Maschine 3.6.0's strip (Grid browsing, Grid
editing, Scale 1/2) and are the closest reading of them.

| | points |
|---|---|
| window width | `width_for(main, slots) = (main × 0.78).clamp(560, 1180).min(slots × 292)` — 1170 for a 1500-wide app showing eight slots, as the capture's 1181 is; **a page of one slot is one column wide (292), never stretched across the bar** |
| bar | 28 |
| body | 144 = header band 40 + value row 40 + sub row 40 + bottom pad 24 |
| type | header 11.5, value 21, sub 13, page name 13, counter 12 |
| columns | `width / slots`, equal, capped at 292 (`COL_MAX`), 1 pt separators between them (never on the outer edges) |
| layout | group header left-aligned over its run's **first** column; value and sub **centred** |
| cursor | a rule under each column's sub-label; the focused one 2.5 pt white, the rest 1 pt dim |
| editing | corner brackets, inset 6 pt, spanning the value and sub rows |

What this replaced, and what was wrong with it: the strip was a row of fixed 52×44 pt **filled**
cells with the title inside the body and the counter at the right — a button bar. The reference is
a flat bar with thin rules and three lines of type per column, and its window's width follows the
app, not the slot count (four Grid slots fill the same bar eight Scale slots do).

## Platform facts, checked against the pinned versions (eframe/egui 0.36.2)

- `ViewportBuilder::with_mouse_passthrough(bool)` exists and is applied as
  `window.set_cursor_hittest(!passthrough)` (`egui-winit-0.36.2/src/lib.rs:2236`).
- `set_cursor_hittest(false)` is implemented on macOS (`ignoresMouseEvents`) and Windows
  (`WS_EX_TRANSPARENT`) and on X11 (input shape). **On Wayland winit returns an error**, which
  egui-winit logs and otherwise ignores: on Wayland the window will take clicks. Nothing above
  this layer can fix that, so the window must be harmless when it does — no click target, no
  hover, no drag handle (D2): a stray click costs one click and nothing else.
- `show_viewport_immediate` already has a precedent in this app (`splash.rs:81`), and its
  semantics are what D6 uses: not calling it closes the viewport.
- `always_on_top` is spelled `with_window_level(WindowLevel::AlwaysOnTop)` in 0.36.

## Shape

New: `apps/viewer/src/jam_window.rs`

- `pub fn show(ctx, page: Option<(&OsoPage, &[Row])>, saved: Option<Vec2>) -> Option<Vec2>` — no
  page draws nothing and returns `None`; otherwise it places the window (`position`), builds the
  `ViewportBuilder` (inner size from `window_size(slots)`, the flags of D7) and calls
  `ctx.show_viewport_immediate(id(), builder, |ui, _| paint(…))`; it returns the offset the window
  ended up at when that differs from the one it was given.
- `fn paint(ui, name, counter, slots, rows, width, draggable)` — the bar (name and counter at
  the left; dragging it moves the window), then `jam_strip::slots(ui, slots, rows, width)`.
- `pub fn window_size(slots) -> Vec2`, `pub fn position(saved, main, size, monitor) -> Pos2`,
  `pub fn offset_of(placed, main) -> Vec2`, `fn clamp_axis(offset, len, monitor) -> f32`,
  `pub fn id() -> ViewportId` (`"deckard-jam-strip"`).
- `fn mark_ni(painter, at)` — the mark, from primitives (no NI artwork).

Changed:

- `jam_strip.rs` — `name`, `counter`, and a `slots` (`paint`'s slot-drawing half); `title` keeps its
  old meaning by composing the two. `paint` and `TITLE_ROW` are gone: with the strip in its own
  window there is no docked row, so there is exactly one way to draw a page.
- `jam_strip.rs`'s `impl App`: `jam_shown() -> Option<(OsoPage, Vec<Row>)>` is the one lookup, and
  `jam_strip()` (the tests' triple) is now derived from it — one place decides whether there is a
  strip at all.
- `main.rs` — `mod jam_window;`, and the call site: `jam_shown` → `jam_window::show` → save the
  offset it reports.
- `settings.rs` — `jam_strip_pos: Option<(f32, f32)>` with its default, its `x,y` parse and its save.

## Tests (named, and named after the obligation they cover)

- `jam_strip::tests::the_title_bar_carries_the_counter_and_the_body_does_not` — D3.
- `jam_window::tests::the_window_is_as_wide_as_its_slots_and_one_bar_taller` — geometry.
- `jam_window::tests::nothing_saved_centres_it_over_the_main_window` — the reference's centring.
- `jam_window::tests::a_saved_offset_comes_back_with_the_app` — persistence, D5.
- `jam_window::tests::an_offset_from_a_bigger_desktop_still_lands_on_this_one` — the brief's
  "second monitor without going off-screen", both axes.
- `jam_window::tests::the_offset_saved_is_the_one_the_window_is_at` — the write-back, and that an
  unmoved window writes nothing.
- `jam_tests::the_strip_is_a_window_of_its_own` — the app-level proof: a window sized from the
  page, and no page means no window (D6).

## HIL (the brief's "capture-tested")

Two-window captures on the rig machine (`hil/strip-window-2026-11-21.md`):

1. the window alone, over the desktop, with the page name, the counter and the mark;
2. the window over Ideas, over Song, and over the Mixer — the property the docked strip failed;
3. two seconds of no input on a non-static page: the window goes; a static page survives;
4. a click through the strip onto the view beneath, on this machine's compositor;
5. quit and restart: the window comes back where it was, and does so on a second monitor.

Rows 4 and 5 are the ones the platform section above says will differ per compositor; they are
recorded per machine, not asserted from a test.

## Not in this plan

- No docked fallback, no mode switch, no preference to choose one. One strip, one window.
- No mouse interaction inside the window, and therefore no drag handle (D4's cost, taken).
- No NI artwork: the mark is drawn from primitives (clean-room boundary, `docs/FEASIBILITY.md`).
- No third window: the desktop BROWSE overlay stays out (PR A's scope is the Jam strip).
