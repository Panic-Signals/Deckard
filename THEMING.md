# Making a DECKARD controller theme

A theme sets how the Maschine MK3's two screens look: their colours, fonts and text
sizes. Every page DECKARD draws on the MK3 uses the theme, so one file restyles them
all. You don't need to know any programming beyond editing a list of values.

Themes apply to the MK3's screens. The DECKARD app's own window isn't themable yet.

| Maschine | Palenight |
|---|---|
| ![LFO page, Maschine theme](docs/screenshots/theme-maschine-lfo.png) | ![LFO page, Palenight theme](docs/screenshots/theme-palenight-lfo.png) |
| ![Mod matrix, Maschine theme](docs/screenshots/theme-maschine-mod-matrix.png) | ![Mod matrix, Palenight theme](docs/screenshots/theme-palenight-mod-matrix.png) |

## The themes DECKARD comes with

- **Maschine:** the look of Maschine's own screens: black, grey buttons, orange accents.
  Every other theme builds on it.
- **Palenight:** deep blue-grey with lavender text and purple and cyan accents. Its
  chosen buttons have an animated orange-and-cyan border. A new install starts with it.
- **Solarized Dark:** Ethan Schoonover's Solarized palette.

Pick one in **Preferences ▸ Hardware ▸ Controller theme**.

## Your first theme in five minutes

1. In DECKARD, choose **Controller ▸ Open Controller Scripts Folder**. A folder opens,
   with a `themes/` folder inside it and DECKARD's own files under `defaults/`.
2. Make a new file in `themes/`, for example `themes/midnight.lua`. The file name is
   the theme's name; underscores become spaces, so `deep_sea.lua` shows as "Deep Sea".
3. Put this in it:

   ```lua
   -- Midnight: navy screens with a green accent.
   return {
     base = "maschine",

     colors = {
       background = { 0x0a, 0x10, 0x24 },
       accent = { 0x3d, 0xdc, 0x84 },
       button_selected = { 0x3d, 0xdc, 0x84 },
     },
   }
   ```

4. Open **Preferences ▸ Hardware ▸ Controller theme** and choose **Midnight**.

`base = "maschine"` means "start from the Maschine theme": anything your file leaves out
comes from there. So a theme only needs the values it changes.

While your theme is chosen, DECKARD reloads it each time you save the file, so you can
keep the MK3 in front of you and tweak values. If the file has a mistake, DECKARD keeps
showing the last version that worked.

## What a theme holds

A theme file returns one table with these parts. All of them are optional when you use
`base`.

### `colors`

Named colours. Write each as `{ red, green, blue }` or `{ red, green, blue, opacity }`,
each 0–255 (hex such as `0x3d` works too).

The colours you'll change first:

| Name | Where it shows |
|---|---|
| `background` | Behind everything. |
| `text` | Values and most text. |
| `weak` | Labels and secondary text (parameter names, hints). |
| `accent` | Highlights on DECKARD's own pages: the selected mod matrix cell, the LFO's wave. |
| `on_colour` | Text drawn on a filled colour, such as a highlighted cell. |
| `button` | The outline of the display buttons along the top. |
| `button_text` | A display button's label. |
| `button_selected` | A display button that's switched on. |
| `button_selected_text` | Its label. |
| `button_off` | A display button that does nothing on this page. |
| `bar` | The strip under the buttons. |
| `panel` | Panels on DECKARD's own pages. |
| `record` | Recording. |
| `separator` | The lines between knobs. |

There are more for the Mixer (`mixer_*`), the Ideas and Song views (`ideas_*`,
`song_*`), the Browser (`browse_*`), Sampling (`wave_*`, `slice_*`, `zone_*`,
`meter_empty`, `marker`) and pads (`pad_*`). `defaults/themes/maschine.lua` lists every
one, each with a note on where it shows. Copy the ones you want into your theme.

### `fonts`

Maps the two font roles the pages use to real fonts:

```lua
fonts = {
  regular = "roboto-medium",
  mono = "roboto-mono-medium",   -- numbers that shouldn't jump around
},
```

The fonts built into DECKARD are `roboto`, `roboto-light`, `roboto-medium`,
`roboto-bold`, `roboto-mono`, `roboto-mono-medium`, `roboto-mono-bold`, `ubuntu`
(Ubuntu Light) and `hack` (Hack, fixed width).

### `styles`

Named text styles, each with a `font` (a role from `fonts`, or a font name), a `size`
in pixels and a `color` (a name from `colors`, or `{ r, g, b }`). A style takes anything
it leaves out from `styles.default`, and with `base`, from the base theme's same style.

```lua
styles = {
  default = { font = "regular", size = 15, color = "text" },
  title = { size = 22 },                -- bigger page titles
  knob_value = { color = "accent" },    -- knob values in the accent colour
},
```

The styles you'll most likely touch are `default`, `title`, `param_name`,
`param_value`, `knob_name` and `knob_value`. Again, `defaults/themes/maschine.lua` has
them all.

### Sizes

- `header_height`: the height of the button row along the top (32).
- `knob_bar_height`: the bar showing a knob's value (6).
- `corner`: how rounded corners are, 0–1 (0.65).
- `unfocused`: how bright the project's colours show where something isn't chosen, 0–1.
- `faded`: how strongly a pattern that isn't in the scene shows its colour, 0–1.

### `palette` and `palette_text`

The project's 16 colours (and white) as the screens show them, as fills and as text.
Most themes leave these alone, so groups and sounds keep the colours you gave them in
the project.

### `chase` (optional)

An animated border on the chosen display button or tab: two colours fading into each
other, running round its edge.

```lua
chase = { colors = { { 0xf7, 0x8c, 0x6c }, { 0x89, 0xdd, 0xff } }, lap = 1.6, width = 2 },
```

`lap` is the seconds for one trip round, and `width` the border's width in pixels. The
Maschine theme has no chase. Palenight turns it on.

### `base`

Another theme's name. Whatever your theme leaves out of `colors`, `styles` (down to
each style's single fields) and the rest comes from that theme. You can build on any
theme, including your own: `base = "palenight"` gives you Palenight with your changes.

## A worked example

Palenight is a complete theme in about 80 lines: it names its colours once at the top,
then maps them onto the theme's names. Find it at `defaults/themes/palenight.lua` and
use it as a starting point:

```lua
local BG = { 0x0f, 0x11, 0x1a }
local PURPLE = { 0xc7, 0x92, 0xea }
local BLUE = { 0x82, 0xaa, 0xff }

return {
  base = "maschine",
  colors = {
    background = BG,
    accent = BLUE,
    button_selected = PURPLE,
    -- …
  },
  chase = { colors = { { 0xf7, 0x8c, 0x6c }, { 0x89, 0xdd, 0xff } }, lap = 1.6, width = 2 },
}
```

## Tips

- **Check every page.** Look at Control, Browse, Mixer, Arrange (Ideas and Song),
  Sampling, the LFO page (SHIFT + SELECT) and the mod matrix (SHIFT + SOLO).
- **Mind the contrast.** The MK3's screens are small and often seen at an angle. Keep
  `text` well apart from `background`, and `on_colour` readable on `accent` and on the
  project colours.
- **Don't edit `defaults/`.** DECKARD rewrites that folder each time it opens it. Your
  own files go in `themes/`.
- **Remove a theme** by deleting its file. If it was chosen, DECKARD keeps drawing with
  it until you pick another.
- **Share a theme** by sending its `.lua` file; others drop it into their own `themes/`
  folder.

## Going further

Themes only change how things look. To change what a page shows or what a button does,
see [docs/controller-api.md](docs/controller-api.md): every MK3 page is a script in the
same folder, and you can replace any of them.
