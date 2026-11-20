# HIL: the Jam strip's own window — `strip-window-2026-11-21`

Status: **run sheet, not yet run.** The two-window captures the brief calls for (one of the window
alone, one of it over each view) need a machine with a Jam or a published page and a compositor to
look at; nothing here was captured from the branch as committed. What *is* proved headless is
listed under "Headless, already green" so the rig pass only has to answer what a test cannot.

Plan this implements: `docs/epics/jam-support/plan-2026-11-21-overlay-window.md` (work item
`22-jam-overlay-detached-window.md`'s required plan). Contract: `oso-contract.md` — the page, its
cursor and its lifetime are the script's, and the window adds none of them.

## Rig

As `hil/strip-repoint-2026-11-20.md`: Jam on USB, Maschine quit **and its background agents gone**
(`pgrep -fl 'NIH|NTKDaemon'`, then `pkill -f 'NIH'; pkill -f NTKDaemon`), no script override folder
under `$DECKARD_SETTINGS/controller/`, and `env -u DECKARD_SCRIPTS cargo run --offline -p deckard`
so the run proves DECKARD's own `maschine_jam`, not NI's copy.

## Headless, already green on the branch

- `jam_tests::the_strip_is_a_window_of_its_own` — a window sized from the published page, and no
  page means no window.
- `jam_strip::tests::the_title_bar_carries_the_counter_and_the_body_does_not` — the `n/N` counter
  is the bar's, not the strip body's.
- `jam_window::tests::*` — the window's geometry, and the offset band that keeps a saved place
  usable on a smaller display.
- `jam_tests::jam_oso_pages_need_no_ni_install` — the overlay with no NI install at all.

## The pass

1. **The window alone.** Touch the encoder on a fresh project (SCALE) and capture the window
   against the desktop, not the app: a chromeless window, one bar reading `Scale` at its left,
   `1/2` at its right before the mark. No buttons, no title bar of the system's.
2. **Over every view.** With the strip up, switch Ideas → Song → Mixer: the window stays where it
   is and stays legible over each. (This is the property the docked strip could not have.)
3. **Its own lifetime.** `TEMPO` (static): the window stays up past two seconds of no input and
   goes on the next page button. `SCALE` (non-static): untouched for ~2 s, it goes by itself. The
   window must never outlive the page, and never lag it.
4. **No mouse.** Click, double-click and drag on the strip: the click lands on whatever is under
   it (the app's view, the desktop), and the strip neither moves nor takes focus nor changes the
   page. **On Wayland this row is expected to FAIL** (`set_cursor_hittest` is unimplemented there;
   egui-winit logs it) — record the compositor (Wayland vs X11 vs macOS) with the result. What
   must hold even there: a click on the strip does nothing at all — no hover, no drag, no cursor.
5. **Position.** Quit and restart with the strip showing: it comes back at the same offset from the
   app's top-left. Then move the app to a second display (or shrink the desktop): the strip comes
   back beside it and some of it is still reachable — never wholly off-screen.
6. **Never steals focus.** With the strip open, type into the app: keystrokes go to the app. Show
   the strip and check the focused window did not change.

## Sign-off

Rows 1–3 and 5–6 pass, with row 4 recorded per compositor and its Wayland note. Captures filed
beside this page as `strip-window-alone.png`, `strip-window-ideas.png`, `strip-window-song.png`,
`strip-window-mixer.png`. If row 4 fails on this machine's compositor, that is the platform
finding the plan predicted, not a build failure — say so in the PR body rather than working around
it in the viewer.
