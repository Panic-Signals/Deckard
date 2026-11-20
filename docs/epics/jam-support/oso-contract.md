# The OSO contract

Status: **landed** — the wire and the routing below are what the code does now. The rule this
doc exists to state: *pages live in the script, rendering and input routing live in the host, and
the script owns the slot's behaviour.*

Date: 2026-11-20 · `crates/surface/src/lib.rs` (`OsoPage`, `OsoSlot`), `crates/surface/src/api.rs`
(`deckard.oso`), `crates/surface/lua/devices/maschine_jam/main.lua` (the composers),
`apps/viewer/src/jam_strip.rs` (the renderer), `apps/viewer/src/jam_io.rs` (the routing).

## The ruling that shapes it

**The script owns the behaviour.** A slot carries its display text and its header; the turn and
the press are the script's, because it already owns the page and the cursor. The host applies
nothing. Reasons:

- Page composition already lives in the script, and a slot's step is the same kind of knowledge:
  which parameter, which table, which clamp, whether Shift refines it.
- Host-applied steps would need a parameter-addressing scheme the host does not have, and every
  new parameter type would be a Rust change.
- NI's own scripts work this way (`m.items[m.at]` with `item.turn`), which is what `maschine_jam`
  already mirrors.

Consequence to accept: a slot's behaviour is only as testable as the script. That is why the
obligations below sit on the *page*, not on the host.

## The page

```lua
deckard.oso({
  title, page, pages,   -- "Scale 1/2": `pages` counts only pages that exist
  slots,                -- left to right; `false` is a hole, and keeps its column
  focus,                -- 0-based index into `slots`, holes counted
  editing,              -- whether the slot at `focus` is being changed
})
```

- **`pages` never counts a page that cannot exist.** `Arpeggiator` is always `3` (8 / 7 / 2);
  `Scale` is `8 + 1`, so it is `2` while the keyboard layout is isomorphic and `1` otherwise.
  A `2/2` that renders nothing is a bug in the composer.
- **A view switch draws nothing.** An OSO appears when the encoder is touched or a menu is
  opened, and the strip is empty otherwise — there is no page for "the keyboard view is
  selected" (one existed once, and it hid the fact that the eight-plus-one split belonged to
  `Scale`).
- **`focus` and `editing` are the script's on every page**, menu or tab. The host does not compute
  them, keep a second copy, or advance them on its own — that is exactly how the underline came to
  sit on a slot the encoder was not on.
- **The strip draws the cursor where it is, holes counted** — it never settles one of its own. A
  cursor on a hole underlines nothing, which is honest: for a menu, the status line names that same
  unavailable item.
- **Whose job is it to avoid a hole?** The tab's, because it steps: `keyboard_oso_page` settles its
  cursor onto a filled slot whenever the pad layout changes the fill under it. A menu's cursor is
  the menu's (`m.at`), and it may rest on an item that a setting has made unavailable.

## The slot

```lua
{ value, sub, group }   -- value: the display text, already formatted ("Horizontal", "C3", "75 %")
                        -- sub:   the small name under it ("Direction"); absent leaves the row blank
                        -- group: the spanning header ("PAD LAYOUT"); absent for a run with none
```

- **Nothing else crosses the wire.** `label` was required and read by nobody: the script keys its
  own slots by it (`RootNote` → its turn function) and no host code looked a slot up by name, so it
  only failed scripts that had a slot the host would have ignored.
- **`kind` was dropped too.** With the press always the script's, the host never needs to know
  whether a slot is an enum, a range or an action: the script does.
- **A hole is `false`**, and keeps its column. Holes are the composer's statement that a value is
  unavailable, never the renderer's inference. A slot with no `value` is a hole, full stop.

## Who does what

| | Script | Host |
|---|---|---|
| Page membership, holes, page count | owner | — |
| Cursor (`focus`) and edit flag | owner | draws |
| What an ENCODER press means | owner | hands it over |
| What an ENCODER turn means | owner | hands it over |
| Value and header text | owner | draws |
| Cell geometry, clipping, colours, title row | — | owner |
| Settings the page depends on | reads | publishes as they are |

**Routing rule:** every ENCODER press and turn reaches the script. The script decides — a menu's
cursor, the tab's edit flag, a value step, its own tempo. The host never acts on an encoder event
and never routes by page type.

**A page a script drew before the host applied what it asked for is stale**, and the host fixes it
in the same frame: after applying the script's requests it ticks the script once more, over the
state it now holds. That one extra tick is why PERFORM FX shows the mode it just changed rather
than the one it replaced. It runs only when something was applied, so nothing animates twice.

## When a page goes

Two lifetimes, both inherited from NI's model (`JamHelper.isOSOTypeStatic`; the RE notes
`2026-11-16-jam-overlay-refs.md` §4 give the list and the timer):

| | static | non-static |
|---|---|---|
| types | `GRID`, `TEMPO`, `REC_MODE`, `RANDOMIZER`, `PERFORM_FX` | everything else — `SCALE`, `ARP_REPEAT`, `SWING`, `TUNE`, `LOCK`, `NOTES` |
| opened by | its own key or chord, on the press | its trigger, usually the encoder touch |
| closed by | that key again, **the controller's page changing**, the device going | the idle timer, an explicit close, a page change |
| idle timer | **never armed** | armed on show: `DEFAULT_OSO_TIMEOUT = 70` ticks ≈ **2 s** |

`NOTES` never arms the timer on press either. `isOSOLocked` holds a page up while the current page's
button is held, the BROWSE strip is touched, the mouse drags on the strip, edit mode is on, or (for
TUNE/SWING) any touch strip is touched — the re-arm points are show, a group-switch refocus, and
leaving edit.

The page-change rule is NI's `onControllerPageChanged`, which hides every static OSO unless
SWING/TUNE is held (`ParameterHandlerJam.lua:180-196`). A device needs that page as a **fact**,
not as a call: `View.page` (the app's controller page) reaches it as `s.surface_page`, and the
script records the page each static menu opened on. Without the fact the rule is unreachable — the
Jam's own `deckard.page()` is its own runtime's, not the controller's.

## Settings

The script reads `deckard.state().settings.values`, and the host publishes it as a **projection**,
not a push: the View is built from the app's settings, so a value changes there when it changes
here. The three triggers this replaced (Settings page open, `jam_settings_wanted`, a Jam connected)
each guessed at who reads the view, and on a cold boot none of them fired: the strip's Mode slot
drew as a hole until a menu happened to ask first.

## Migration and boundaries

- **NI-scripted controllers are not part of this contract.** Their `setOSOType`/`addParameter` API
  stays in `lua-host` until they are ported; the two shapes coexist deliberately.
- **The renderer must not grow a feature the contract cannot feed.** No hover, no mouse, no
  per-slot animation: the strip belongs to the encoder.
- **The composer stays per-device.** `maschine_jam`'s page builder is the Jam's, not a shared
  framework, until a second device needs the same pages.

## Obligations, and where each one is

1. **A published page renders, and the cursor and edit mark follow it** —
   `jam_strip::tests::the_underline_follows_the_published_cursor_wherever_it_moves`,
   `the_edit_mark_is_the_scripts_and_marks_only_the_focused_slot`.
2. **A turn and a press on the keyboard tab are the script's** —
   `jam_tests::menu_cursor_moves_the_published_focus` (the tab's half of it, on a real script).
3. **A tab cursor whose fill varies stays on a slot** — a published page under changing settings
   (`jam_tests::a_published_page_keeps_its_cursor_on_a_slot_when_its_fill_changes`), and the
   composer's own view of it
   (`jam_strip::tests::a_page_whose_fill_varies_keeps_the_cursor_on_a_slot`).
   A cursor on a hole draws nothing: `jam_strip::tests::a_cursor_on_a_hole_underlines_nothing`.
4. **The settings reach the scripts with nothing asking** —
   `surface_settings_tests::the_scripts_read_the_settings_with_nothing_asking_for_them`.
5. **A turn republishes the page it changed** — `jam_tests::a_turn_republishes_the_page_it_changed`.

`docs/controller-api.md` carries the script-facing half of this section.

## Open questions

- ~~A menu cursor can sit on an unavailable item.~~ **Settled** (`work-items/31-jam-menu-cursor-skips-holes.md`):
  one notch moves one place and the landing is never an item that does not show, as NI's
  `incrementFocusParameter` does. The cursor therefore cannot reach a hole, and the strip's
  "underline nothing" case is left for a page whose fill changed outside a turn.
- **`kind`'s absence is a promise the script keeps**: a slot that cannot be turned must not set
  `editing`. It is worth a test the day a composer grows a read-only slot.
- **PERFORM FX's FX-being-added case**: the extra tick covers it, but only because adding an FX is
  an `Action`. A future request that changes state without an `Action` would be stale again.
- **The strip's own rendering still clips long values mid-word** (`Harmoni`, `Sequenti`). Cell
  width and clipping belong to the proper OSO, not to this contract.
