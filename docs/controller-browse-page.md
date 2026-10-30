# MK3 BROWSE page: behaviour spec

What the Maschine MK3 does on its BROWSE page, written so DECKARD's own controller layer
can do the same thing. This is a clean-room description: NI's scripts were read to learn
the behaviour, and none of their code is reproduced here. Geometry comes from DECKARD's
parity capture of the page with an empty library (`$TMPDIR/deckard-parity/browse.layout.txt`).
Colours come from the page's style sheet. Anything not settled is marked **unknown**.

Coordinates are in pixels on the 960 × 272 pair of displays. The left display is x 0–479
and the right is x 480–959. Rectangles are written `[x,y w×h]`. Every font is Pragmatica
(`prg65_j.ttf`), size 14 unless stated otherwise. Colours are `r,g,b` or `r,g,b,a`.

## 1. Model the page works on

The page shows and edits the library browser's query, the same one the desktop browser
uses (`docs/browser.md`). It has eight *parameters*. Knob *n* edits parameter *n*, and
label *n* in the bottom bar shows it:

| # | Knob | Meaning | "All" caption |
|---|------|---------|---------------|
| 1 | knob 1 | Product **category** (or **vendor**, depending on the sort order) | "All Categories" / "All Vendors" |
| 2 | knob 2 | **Product** (the library). This is bank-chain level 0 | "All Products" |
| 3 | knob 3 | **Bank** (bank-chain level 1) | "All Banks" |
| 4 | knob 4 | **Sub-bank** (bank-chain level 2) | "All Sub-Banks" |
| 5 | knob 5 | **Type** (attribute level 0) | "All Types" |
| 6 | knob 6 | **Sub-type** (attribute level 1) | "All Sub-Types" |
| 7 | knob 7 | **Character / mode** (attribute level 2) | "All Characters" for Instruments, Effects, Loops and One-shots; "All Sub-Types" for the other kinds |
| 8 | knob 8 | **Results** (the focused result) | none |

Other state:

- The **content kind** (file type). The visible kinds, in order, are Projects, Groups,
  Sounds, Instruments, Effects, Loops and One-shots.
- **User / Factory** mode.
- The **favourites filter**, on or off.
- The **product sort order**, by category or by vendor.
- The **focus parameter** (`FocusParam`, 1–8). It starts at 8 (Results), and the page
  object keeps it between visits.

A parameter's *count* is how many entries it has:

- Category: the number of product groups. It is 0 when products aren't grouped.
- Product: the products in the chosen category.
- Bank and sub-bank: the slot item counts. A bank's count is forced to 0 while no product
  is chosen. A sub-bank's count is forced to 0 while no bank is chosen.
- Type, sub-type and character: the number of active attribute values.
- Results: the result count.

Index −1 (shown as "All…") means "no filter" for parameters 2–7.

**Debounced product changes.** Choosing a category or a product updates the page's own
cached index, labels, logo and tile highlight at once. The search only runs after a
countdown of page-timer ticks: **20 ticks** for a category and **15 ticks** for a product.
Another turn during the countdown restarts it. Setting either countdown cancels the other
when it fires. Banks, attributes and the result focus go to the search straight away
through the deferred-search controller. The tick period is not in the scripts and is
**unknown**; DECKARD's host timer decides it.

**Busy.** While the browser controller has a job running (a search in flight), screen
buttons do nothing, the screen-button state isn't refreshed, and screens refresh only
when forced.

## 2. Entering and leaving

- **BROWSE** opens the page. It is a normal page, not a modifier page. Opening it clears
  everything but the bottom page off the page stack, then pushes Browse.
- **BROWSE again** while the page is on top pops it, and you return to the page below.
- **SHIFT + BROWSE** opens the **Module** (plug-in browser) page instead. It is a
  different page and isn't covered here.
- Pressing any page button closes the effects-chain **insert mode** if it was on. The two
  exceptions are pressing SHIFT+BROWSE and releasing BROWSE.
- **On show:**
  - The touch overlay switches to its "browser" style (see §9) and the tag browser is
    hidden.
  - The page remembers the current jog-wheel mode and sets the wheel to **custom** mode,
    so the page gets the wheel.
  - The page starts a timer: first after 2 ticks, then every tick while the page is
    visible.
  - The screens fully refresh. This reloads the cache, the touch lists for knobs 1, 3
    and 4, the product, bank, type and result views, and checks that the focus parameter
    is valid.
- **On hide:** the page puts back the remembered jog-wheel mode. If the wheel is still in
  custom mode after that, it is set to **default**.
- **VOLUME / TEMPO / SWING** while on the page toggle the wheel between that mode and
  custom mode. The new mode is stored as the one to restore when you leave. This means
  that if you leave in Volume mode, the wheel stays in Volume mode.
- **Loading doesn't leave the page.** LOAD, a wheel press on Results, and PREVIOUS/NEXT
  all keep you browsing. There are two exceptions:
  - Insert mode (§10), where LOAD loads and then returns to the bottom page.
  - The sampling sub-mode (§10).

## 3. Screen layout (both displays)

Every background is black (0,0,0). Each display is a 480 × 272 column with three parts:
a head row 35 high, a display area about 197 high, and a parameter bar 39 high.

### 3.1 Head row: `[0,0 480×35]` per display, content inset 4 left and 4 top

The head-button looks:

| State | Background | Text | Picture frame |
|-------|-----------|------|---------------|
| Normal | 71,71,71 | 255,255,255,229 | head-button picture, frame 0 |
| Selected | 255,255,255,178 | black | head-button picture |
| Disabled | 41,41,41 | 41,41,41 | head-button picture, frame 2 |

The disabled text colour matches its background, so the text is effectively hidden.
Frame 2 of the picture shows a faint outline. In the capture, PREVIOUS/NEXT/LOAD still
show their text dimly.

**Left display, buttons 1–4** (no SHIFT):

| Button | Rect | Look |
|--------|------|------|
| 1–2 (arrow pair) | bar `[4,4 234×24]`: "◀" hit area `[4,5 22×22]`, "▶" hit area `[216,5 16×22]` | white bar (255,255,255) under the arrow-frame picture (234×24). The label is centred with 27 px margins each side and 4 px top margin, text 255,255,255,229. The label shows the **content kind in capitals plus "S"**: PROJECTS, GROUPS, SOUNDS, INSTRUMENTS, EFFECTS, LOOPS, ONE-SHOTS. An enabled arrow has no fill. A disabled arrow is filled 71,71,71 (greyed). |
| 3 | `[242,4 115×24]` | "LOCATE" |
| 4 | `[361,4 115×24]` | "USER". Selected while User mode is on. |

**Ambiguity, button 3.** The scripts ask for an icon style for button 3: a magnifier
picture, `std_button_browse`, 115×72 with 3 frames, normally drawn with no text. While the
"overridden" flag is set, the button is drawn as a text head button instead. The page
always clears that flag on refresh, so the icon should show. DECKARD's capture rendered
the head-button class with the text "LOCATE". Which of the two real hardware shows is
**unknown**.

**Right display, buttons 5–8** (no SHIFT):

| Button | Rect | Look |
|--------|------|------|
| 5 | `[484,4 115×24]` | "PREVIOUS" |
| 6 | `[603,4 115×24]` | "NEXT" |
| 7 | `[722,4 115×24]` | Favourites-filter picture (`std_button_favfilter`, a ★ glyph, 3 frames) with no text. Frame 1 shows while the filter is on, frame 0 while it is off. |
| 8 | `[841,4 115×24]` | "LOAD" |

Buttons 5 and 6 sit where the second arrow pair would be. That arrow pair is switched off
here and only appears under SHIFT (§5).

### 3.2 Left display area: product tiles

- The product list runs `[0,36 480×196]`. Its tile area is `[0,37 472×195]`, with an
  inset of 20 left and 4 top.
- Tiles come in **rows of 3**:
  - A row is 458 × 90, with 17 px between tiles and 6 px between rows.
  - Two rows are visible, so one **page holds 6 products**.
  - The list's length is always whole pages: `ceil(count / 6) × 2` rows. Tiles past the
    last product are hidden.
- **Each tile** is 135 × 90:
  - Normally it carries the product-frame picture (136×90 outline) and has no fill.
  - The picture is 134 × 66, inset 1 left and 1 top. It is the product's browser icon.
    If there is no icon:
    - a Sounds.com product gets the Sounds.com picture;
    - in User mode, the picture depends on the content kind: a user-loops picture for
      Loops, a user-samples picture for One-shots, a user-effect picture for
      Instruments and Effects, and none for the other kinds;
    - otherwise there is no picture.
  - The name label is 138 × 22, left-aligned, with 6 px side margins and 1 px top margin.
    It is white and cropped to fit. It shows the product's short name if one exists,
    else the product name without its vendor prefix.
- **The focused product's tile:**
  - The frame picture goes away and the tile fills **white**, with 1 px trimmed at the
    right and bottom.
  - The name text turns **black**.
- **Scrolling:**
  - The list scrolls by pages so the focused product's page is shown: the first visible
    row is `floor(index / 6) × 2`.
  - The move uses a smooth-out animation (type 3) with a focus delay of 10. On a full
    refresh it jumps without animating.
  - With "All Products" chosen, the list shows the first page on refresh.
- **Scrollbar:**
  - It sits at `[472,36 8×196]`, with a 3 px left margin and 5 px bottom margin.
  - The thumb is drawn in white. There are no inc/dec arrows and it never auto-hides.
  - It is visible only when there are **more than 6 products**.
  - The style sheet also has a browser-left rule (13 wide, margin-left 8, 5/1 outer
    margins), but the capture shows the generic 8-px bar. Use the capture.

### 3.3 Right display area: logo and results

- The background `[480,36 480×197]` is the right-screen picture: black with a divider
  line.
  - If the product has a background colour (from the product's resources), that colour
    fills it.
  - A Sounds.com product fills it 255,255,255,245.
  - Otherwise it is black.
  - If the product has a **logo picture**, the picture is drawn here and the logo text is
    empty.
- The **logo text** fills the left half, `[480,36 240×196]`:
  - 28 px, white, centred both ways, with 20 px side margins and wrapping.
  - Without a logo picture it shows the product name.
  - It is empty when "All Products" is chosen or there are no products.
- The **result list** sits at `[720,36 240×196]`, with insets 8 top, 6 bottom and 1 left:
  - The rows area is `[721,44 231×182]` (style says 226 wide).
  - Each row is **21 high with 5 px below it**, so **7 rows** fit.
  - A row's background is the list-item picture (20×42, 2 frames, 4-px fixed ends,
    stretched). Frame 0 is the normal row and frame 1 the **focused** row.
  - Row text is white, 14 px, with margins 9 left and 5 right, cropped to fit. The style
    sheet says right-aligned; that is unverified, so check it against hardware. On the
    focused row the text is **black** on the frame-1 (white) fill.
  - **Favourite star:** a 21×21 star (2 frames). Frame 0 shows on normal rows and
    frame 1 on the focused row. Rows that aren't favourites show no star. Where the
    star sits in the row is drawn natively and is **unknown**.
  - The list re-centres on the focused result whenever the results change. It doesn't
    animate (animation type 0).
  - The scrollbar is `[952,44 8×182]`, with a white thumb, no arrows, and it never
    auto-hides. It is visible only when there are **more than 7 results**.
- The **icon bar** is `[490,204 240×18]` (x 10, y 168 within the display area) and holds
  two icons:
  - The **prehear icon** (18×18) shows while prehear can be used and is switched on.
  - The **library-scan animation** (18×18, 32 frames, advancing every 4 ticks) runs
    while a database scan is in progress.
- While a type, sub-type or character knob is touched, the **tag browser** replaces the
  results frame and the background picture is hidden (§9).

### 3.4 Parameter bar: `[0,233 480×39]` and `[480,233 480×39]`

Eight labels, each **120 × 39**, sit at x = 0, 120, 240, 360, 480, 600, 720 and 840.
Text is 14 px and centred, with 6 px side margins and 1 px top margin. All labels are
marked to use brackets.

**Text:**

| Label | Text |
|-------|------|
| 1 | Empty when the category count is 0. Otherwise the category short name: "Native Instruments" → "NI", "Universal Audio (UADx)" → "UA", else the resource short name for the category or vendor (fallback: the full name). The "All…" entry shows "All Categories" or "All Vendors". |
| 2 | Empty, and the label is **disabled**, when there are no products. Otherwise the product short name, or "All Products". |
| 3 | Empty when the bank count is 0 or no product is chosen. Otherwise the bank short name or "All Banks". |
| 4 | Empty when the sub-bank count is 0, or no product or bank is chosen. Otherwise the short name or "All Sub-Banks". |
| 5–7 | Empty when the count is 0. **"\*"** when more than one value is selected (multi-select made on the desktop). Otherwise the value name or the "All…" caption from §1. |
| 8 | Results count. Filter off: "*N* Results", "1 Result" or "No Results". Filter on: "*N* ★" or "No ★". |

**Colour rules**, in order of strength:

1. The label of the **focus parameter** is *selected*. It turns **white (255,255,255)**
   and shows the bracket frame picture (`bp_frame_paramLabel`, 120×39 per frame,
   frame 1): a "[ … ]" bracket around the label. This is the framed "No Results" in the
   capture.
2. A label showing an "All…" caption is **255,255,255,102** (40 % white, dimmed).
3. A label showing a real value is **white 255,255,255**.
4. The fourth label on each display (knobs 4 and 8) defaults to **255,255,255,102**.
5. Every other label defaults to **102,102,102**.

The "All" or "value" flag is set whenever a label gets a caption or a value. Empty labels
1, 3 and 4 never get the flag set, so they fall back to the default colours. Labels 2 and
5–7 set the flag to "value" even when empty. Since those labels are empty, the colour
can't be seen.

That explains the capture with an empty library:

- Label 1 is 102,102,102 (no flag).
- Label 2 is white (flag set to "value", label disabled, so no frame).
- Label 3 is 102,102,102.
- Label 4 is 255,255,255,102 (the rule for the fourth label).
- Labels 5–7 are white.
- Label 8 is selected and framed, white, and reads "No Results".

## 4. Knobs (screen encoders 1–8)

**Smoothing.** Every knob goes through a per-knob accumulator:

- The accumulator adds the knob's increments.
- It resets whenever the turn direction reverses.
- When it reaches ±0.1, the event counts. The accumulator keeps the remainder.

**One counted event moves the parameter by one step**, whatever the size of the
increment. Steps aren't multiplied, and a fast turn just counts on more events. The one
exception is SHIFT on knob 8 (and the wheel on Results), which jumps 20.

| Knob | One step does |
|------|---------------|
| 1 | Category ±1 within 0…count−1, with no wrap. "All" is entry 0 of the group list. The search is debounced (20 ticks). The touch list's focus follows. |
| 2 | Product. From "All Products" (index −1): turning right focuses the **first product of the page currently scrolled into view**; turning left stays on All. From a product: ±1 within −1…count−1, where −1 is All Products. Display updates at once; the search is debounced (15 ticks). |
| 3 | Bank ±1 within −1 (All Banks)…count−1. Does nothing when the count is 0. |
| 4 | Sub-bank, same as knob 3. |
| 5 | Type ±1 within −1 (All)…count−1, as a single selection. A multi-selection becomes a single one. Does nothing when the count is 0. |
| 6 | Sub-type, same as knob 5. |
| 7 | Character / mode, same as knob 5. |
| 8 | Result focus ±1 (**±20 with SHIFT**), clamped to 0…N−1. If the focus changed, or there is a single result, the result is prehead when prehear is possible (§8). |

Turning a knob doesn't move the focus parameter. Only the wheel's left/right does (§6).

## 5. Screen buttons 1–8

Pressing a button that is enabled and not selected lights its LED **bright** at once.
After every press the screens refresh. Nothing happens while the browser is busy.

### 5.1 Without SHIFT

| Btn | Label | Enabled when | Does |
|-----|-------|--------------|------|
| 1 | ◀ (arrow) | a previous visible content kind exists | Previous content kind (Projects ← Groups ← … ← One-shots). No wrap. |
| 2 | ▶ (arrow) | a next visible kind exists | Next content kind. |
| 3 | LOCATE | visible and enabled when insert mode is off and this isn't the sampling sub-mode | "Quick browse": puts back the browser query that found the focused sound, group or plug-in. |
| 4 | USER | always | Toggles User / Factory. Selected (lit) in User mode. |
| 5 | PREVIOUS | results > 0 and focus > 0 | Result focus −1, then either prehears it (when prehear is possible) or **loads it** while staying on the page. |
| 6 | NEXT | results > 0 and focus < N−1 | Result focus +1, then prehears or loads, as button 5. |
| 7 | ★ (filter) | always | Toggles the favourites filter. Selected while on. |
| 8 | LOAD | results > 0 | Loads the focused result (§7). |

In insert mode the content-kind arrows are narrower:

- Both arrows are disabled unless the focused slot is a sound's first slot (the
  instrument slot).
- "Previous" is also disabled at Instruments, so the kind can't go back before
  Instruments.

### 5.2 With SHIFT held

While SHIFT is held the button row is redrawn:

| Btn | Label | Enabled / shown | Does |
|-----|-------|-----------------|------|
| 1 / 2 | **CATEGORIES** / **VENDORS** as two head buttons; the arrow bar goes off | Shown and enabled only for Instruments and Effects. The current sort order shows selected. | Sorts products by category (1) or by vendor (2). The scripts do this even when the buttons are hidden. |
| 1 (Maschine+ standalone only) | RESCAN; button 2 hidden | enabled when no scan is running | Rescans all content folders. Not relevant to desktop DECKARD. |
| 3 | **+ ROUTING** | Groups only (hidden for other kinds). Selected = the "load group with sound routing" option. | Toggles that option. |
| 4 | **+ PATTERNS** | Groups only. Selected = the "load group with patterns" option. | Toggles that option. |
| 4 | **SPEAK PRESET** | Kinds other than Groups, and only with accessibility on | Toggles speaking the preset name. |
| 4 | still shows **USER** | Kinds other than Groups, with accessibility off | Nothing happens. This is a quirk: the button keeps its unshifted look. |
| 5 / 6 | Arrow pair whose label is the **focused plug-in slot's name** (e.g. "SAMPLER"). If the name is empty, two blank head buttons. | ◀ enabled when the chain has slots and the focus isn't slot 0. ▶ enabled when the focus is a real slot before the end. | Moves the plug-in slot focus −1 or +1 in the focused chain. |
| 7 | **SET ★**, in white text | results > 0 | Toggles the focused result's favourite state. |
| 8 | **PREHEAR** | Shown only when the prehear widget applies (sample kinds). Selected = prehear on. | Toggles prehear (only when a prehear type is selected). Turning it off stops playback. |

## 6. 4-D encoder (jog wheel)

The page only takes the wheel while the wheel is in **custom** mode, which is the default
on this page. In Volume, Tempo or Swing mode, turns go to quick-edit instead.

**Turn:**

- It acts like turning the knob of the **focus parameter**. It uses the same smoothing
  and the same step rules, so it steps the result focus by ±1 (±20 with SHIFT) and
  prehears when focus is on Results.
- Turning while focus is on Results first closes any tag browser or touch list.
- The screens refresh after each counted step.

**Press:**

- The page always takes the press, whatever the wheel mode. Quick-edit never sees it.
- While the encoder is held, the touch overlay is in manual mode. If the focus parameter
  is 1, 3, 4, 5, 6 or 7 (not 2 and not 8), that parameter's list is shown while held:
  - the pop-up list for knobs 1, 3 and 4;
  - the tag browser for knobs 5–7.
- On release the list hides.
- With focus on **Results**:
  - **press** loads the focused result (§7) and you stay on the page;
  - **SHIFT + press** toggles the focused result's favourite instead.

**Left / Right (direction buttons):**

- These move the **focus parameter** −1 or +1 among parameters 1–8.
- Parameters with a count of 0 are skipped.
- Going left and hitting parameter 1 with a count of 0 leaves the focus where it was.
  Going right always ends at 8 (Results) if nothing earlier has a count.
- The new focus label gets the bracket frame.
- If the encoder is held at the same time (manual overlay), the shown list switches to
  the new parameter.

**Up / Down (direction buttons):** these page through the product tiles.

- **Down = next page, Up = previous page.** They do exactly what the hardware page
  buttons RIGHT and LEFT do:
  - With **no product chosen** (All Products), they scroll the tile view by 2 rows (one
    page), clamped to the first and last page. Nothing is chosen.
  - With **a product chosen**, they move the chosen product by ±6, clamped to
    −1…count−1. Reaching −1 means "All Products". The search is debounced as for knob 2.

**Focus validity:**

- After any database refresh, if the focus parameter's count has become 0, the focus
  moves left to the nearest parameter that has entries.
- If that one is also empty, the focus moves right instead.
- Results is always allowed.

## 7. Loading

All loading goes through the app's "load focused result" action:

- The result loads into the current focus, following the same rules as a double-click in
  the desktop browser (see `docs/browser.md` ▸ Loading).
- A sound replaces the focused sound, and a group replaces the focused group.
- An instrument or effect goes into the focused chain.
- A sample goes into a Sampler or Audio plug-in, and so on.
- The focused sound is whatever was last chosen with the pads (§11) or the group buttons.

Ways to load:

- **LOAD (button 8)** loads only when results > 0. The LED goes bright first, then the
  item loads and the page stays.
- **Wheel press** on Results loads the same way.
- **PREVIOUS / NEXT** step the focus and then load. When prehear can play the item
  (sample kinds with prehear on), they prehear it instead of loading.

**Insert mode** is the effects-chain mode entered from the plug-in page's "+":

- **First load:** the first load inserts a new slot. After that, loads replace that slot.
- **LOAD or wheel press** closes insert mode and returns to the bottom page.
- **PREVIOUS / NEXT** keep browsing and stay in insert mode.
- **Allowed content kinds:** if the current kind isn't allowed for the slot, insert mode
  closes on the next refresh. Allowed kinds are:
  - Effects in any slot;
  - Instruments, Loops and One-shots only in a sound's first slot.

## 8. Prehear

Prehear is *possible* when all three hold:

- A prehear type is selected, which means the content kind is samples.
- There are results.
- The app says prehear can play.

When prehear is possible:

- Knob 8 and the wheel prehear the newly focused result.
- PREVIOUS and NEXT prehear instead of loading.

The prehear icon in the icon bar shows when prehear applies and is on. SHIFT + button 8
toggles prehear.

## 9. Knob touch

The MK3 knobs are touch-sensitive. On this page:

- **Knobs 1, 3 and 4 show a pop-up list** of their entries: categories, banks or
  sub-banks.
  - The list sits above the knob in the browser style: top at y 37, at most **7 rows**,
    and a triangle pointer under it (pointer top margin 222).
  - It is 1 to 4 sections wide (one section = 120 px), wide enough for the longest entry
    plus the scrollbar.
  - It starts above the touched knob and grows to the right, but never crosses onto the
    other display.
  - It is centred on the current entry, which is shown selected.
  - Rows are 14 px, left-aligned, with 5 px spacing and 8 px side margins.
- **Knob 2** has no pop-up. The tiles on the left display already show the products.
- **Knobs 5, 6 and 7, when they have entries, open the tag browser** on the right display
  instead of a pop-up:
  - It replaces the logo and result list, and the background picture is hidden.
  - It shows every value of that attribute level as a flowing cloud of tags.
  - Each tag is 14 px with a rounded-label frame, 4 px side padding and 5 px gaps.
  - **Chosen tags** have black text on the bright frame.
  - **Other tags** have 255,255,255,153 text on a 50 %-black frame.
  - The cloud area is 182 high, with a top margin of 8 and a left margin of 8. It scrolls
    with a smooth-out animation (focus delay 10) and has a scrollbar inset 8 top and
    12 bottom.
  - Knob 7 shows the *modes* (characters) list for Instruments and Effects.
  - If the touched level's count drops to 0, the browser closes.
- **Knob 8** doesn't show anything:
  - Touching it closes any open pop-up or tag browser.
  - While knob 8 is touched, knobs 5–7 can't open the tag browser.
- The lists for knobs 1, 3 and 4 are refreshed whenever the browser or bank chain
  changes. Their focus entry follows knob turns.

## 10. Sampling sub-mode (for completeness)

The SAMPLE page can open this same page to pick a sample for a new zone. In that case:

- The right buttons read "" / CANCEL / "" / LOAD.
- PREVIOUS is hidden. NEXT becomes **CANCEL**, which goes back to the sampling page;
  SHIFT + 6 does the same.
- The content-kind arrows only switch between One-shots and Loops.
- LOAD puts the focused sample into a new zone, then returns to the sampling page.
- Changing the focused sound, or leaving the sample kinds, returns to the sampling page.
- LOCATE and the plug-in-slot arrows don't apply.

## 11. Pads and group buttons

- **Pads** keep their normal routing: the page doesn't block note playing, and pad LEDs
  follow the normal pad-mode display.
- In **step mode**, pads edit steps as usual.
- Otherwise a **pad press focuses that pad's sound**, which changes where a load goes.
  The exception is when **pad prehear** is on (a pad-prehear type is selected and prehear
  is enabled):
  - The press doesn't change focus.
  - It **prehears the focused result through that pad**.
  - This needs results to exist, step mode to be off, and the app to accept that pad.
- **Group buttons A–H** keep their normal behaviour:
  - focus a group;
  - on the empty slot after the last group, create a group;
  - with SHIFT and 8 or more groups, choose a group bank;
  - SHIFT + ERASE resets a group.

## 12. ERASE

- **ERASE (no SHIFT)**, in User mode only, deletes the selected result files from the
  user library. The MK3 allows deleting files here.
- Whether the app asks for confirmation is decided natively and is **unknown**.
- With SHIFT, or outside User mode, ERASE isn't handled by the page.

## 13. LEDs

LED levels are **off**, **dim** (enabled) and **bright** (selected, or held down). Most
button LEDs follow one rule:

- Enabled → dim, and bright while pressed.
- Selected → bright.
- Disabled or hidden → off. The exception is a disabled button that is held down after
  having been enabled: it stays bright until released.

| LED | State on this page |
|-----|--------------------|
| BROWSE | **Bright** while the page is in the stack. |
| Screen buttons 1–8 | Follow the rule above, from §5. Without SHIFT: 1/2 by previous/next kind; 3 dim when LOCATE is shown; 4 bright in User mode, else dim; 5/6 dim when a previous/next result exists; 7 bright with the filter on, else dim; 8 dim with results. With SHIFT, from the shifted table. All white (no colour given). |
| Page LEFT / RIGHT | Same as wheel Up/Down below; refreshed on every timer tick. |
| Wheel UP / DOWN (white) | **UP** lit (dim) when the tile view isn't on its first page or a product is chosen. **DOWN** lit when the chosen product isn't the last one and either a product is chosen or the view isn't on its last page. Quirk: with no products at all, DOWN and page-RIGHT still light dim. What pressing them does then is **unknown** (the clamp runs from 0 down to −2). |
| Wheel LEFT / RIGHT (white) | **LEFT** lit when moving the focus left would change it. **RIGHT** lit while the focus parameter is below 8. |
| ERASE | In User mode with SHIFT up: **bright** while ERASE is held, **dim** otherwise. **Off** in Factory mode or while SHIFT is held. |
| SHIFT | Bright while held (global). |
| Pads, groups, transport | Normal global behaviour; this page doesn't change them. |

The wheel LEDs, the ERASE LED and the LEFT/RIGHT LEDs are updated on every page-timer
tick. The screen-button LEDs update with each screen refresh.

## 14. Accessibility (speech) labels

DECKARD only needs these if it speaks:

| Control | Spoken name / value |
|---------|---------------------|
| Knobs | "Category" or "Vendor", "Product", "Bank", "SubBank", "Type", "Subtype", "Character", "Preset". "Jump By Twenty" is added for knob 8 with SHIFT. A multi-selection is spoken as "Multiple Selected". |
| Buttons 1/2 | "Previous/Next Content Type" |
| Button 3 | "Quick Browse" |
| Button 4 | "Preset Collection, USER/FACTORY" |
| Buttons 5/6 | "Previous/Next Plug-in" |
| Button 7 | "Filter by Favourites, ON/OFF"; with SHIFT, "Set Favourite, ON/OFF" |
| Button 8 | "Load Preset"; with SHIFT, "Pre Hear" |
| Page LEFT/RIGHT | "File type page up/down" |
| The page | "Browser" |

## 15. Unknowns and quirks, collected

- How long a timer tick lasts, which sets the length of the 20- and 15-tick debounce.
- Whether LOCATE is drawn as an icon or as text on real hardware.
- Where the favourite star sits in a result row, and whether row text is really
  right-aligned.
- Whether ERASE asks for confirmation.
- What DOWN / page-RIGHT do with zero products.
- SHIFT + button 4 for non-group kinds with accessibility off shows USER but does nothing.
- SHIFT + buttons 1/2 change the sort order even when the buttons are hidden.
- The left scrollbar style conflict: the style sheet asks for 13 px, the capture shows
  8 px.
