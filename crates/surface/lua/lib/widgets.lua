-- The parts Maschine's hardware screens are made of, laid out as Maschine lays them
-- out, coloured by the theme: the display buttons' labels, the info bar, the group
-- tiles, the pad grid and the knobs' parameters. Each screen is 480 × 272; the right
-- one starts at x = 480.

local quick_edit = require("lib.quick_edit")

local widgets = {}

-- Display button `i`'s left edge (1-8; 5-8 on the right screen).
function widgets.button_x(i)
  return (i - 1) // 4 * 480 + 4 + (i - 1) % 4 * 119
end

local BUTTON_W, BUTTON_H, BUTTON_Y = 115, 24, 4
widgets.BUTTON_W = BUTTON_W

-- Each screen's rows: the display buttons' labels (to 28), the info bar (35-59), then
-- on the left the chain strip (lib/chain.lua, to 99), then the picture band (106-199),
-- then the knobs' section names (207) and parameters (233-271). The band
-- is the same on both screens, so pictures over the knobs line up across the gap.
widgets.BAND_Y, widgets.BAND_H = 106, 94

-- A palette colour (0..16) from the theme, as a fill or (`text`) as text; `dim` for
-- what's not chosen.
function widgets.colour(theme, palette, dim, text)
  local list = text and theme.palette_text or theme.palette
  local c = list[(palette or 16) + 1] or list[#list]
  if dim then
    local f = theme.unfocused
    return { c[1] * f, c[2] * f, c[3] * f }
  end
  return c
end

-- WCAG contrast. RGB distance treats amber-on-amber as "different" while it is
-- unreadable, and would also trip colours that are far apart in hue but fine.
local function channel(v)
  v = (v or 0) / 255
  if v <= 0.04045 then return v / 12.92 end
  return ((v + 0.055) / 1.055) ^ 2.4
end

local function contrast(a, b)
  if type(a) ~= "table" or type(b) ~= "table" then return 21 end
  local function y(c)
    return 0.2126 * channel(c[1]) + 0.7152 * channel(c[2]) + 0.0722 * channel(c[3])
  end
  local l1, l2 = y(a), y(b)
  if l1 < l2 then l1, l2 = l2, l1 end
  return (l1 + 0.05) / (l2 + 0.05)
end

-- Colour `c` at `a` (0..1) over `base` (both {r, g, b}).
function widgets.blend(c, base, a)
  local mix = function(i) return math.floor(c[i] * a + base[i] * (1 - a) + 0.5) end
  return { mix(1), mix(2), mix(3) }
end

-- A five-pointed star (★) of radius `r` centred on (`cx`, `cy`), filled `colour`.
function widgets.star(screen, cx, cy, r, colour)
  local pts = {}
  for i = 0, 9 do
    local a = math.pi * i / 5 - math.pi / 2
    local d = i % 2 == 0 and r or r * 0.4
    pts[#pts + 1] = { cx + d * math.cos(a), cy + d * math.sin(a) }
  end
  -- Each row of pixels across it: filled between the edges it crosses.
  for y = math.floor(cy - r), math.ceil(cy + r) do
    local yc = y + 0.5
    local xs = {}
    for i = 1, #pts do
      local p, q = pts[i], pts[i % #pts + 1]
      if (p[2] <= yc) ~= (q[2] <= yc) then
        xs[#xs + 1] = p[1] + (yc - p[2]) / (q[2] - p[2]) * (q[1] - p[1])
      end
    end
    table.sort(xs)
    for i = 1, #xs - 1, 2 do
      local x0, x1 = math.floor(xs[i] + 0.5), math.floor(xs[i + 1] + 0.5)
      if x1 > x0 then screen:rect(x0, y, x1 - x0, 1, colour) end
    end
  end
end

-- The theme's style `name`, in another colour if `colour` is given.
local function style(theme, name, colour)
  local s = {}
  for k, v in pairs(theme.styles.default) do s[k] = v end
  for k, v in pairs(theme.styles[name] or {}) do s[k] = v end
  if colour then s.color = colour end
  return s
end

-- How tall a display button's label box is: to its bottom border (a theme that sets
-- text by its capitals, `text_on_caps`: inside the border, the capitals centred in it).
local function label_h(theme)
  return theme.text_on_caps and BUTTON_H - 2 or BUTTON_H - 1
end

-- A display button's label at `x`: its text, a ★ after it (or alone) for `b.star`.
local STAR_R, STAR_GAP = 6, 4
local function label(screen, theme, x, b, st, star_colour)
  local text = b.text or ""
  if not b.star then
    screen:label(x, BUTTON_Y + 1, BUTTON_W, label_h(theme), text, st)
    return
  end
  local tw = text ~= "" and screen:text_width(text, st) + STAR_GAP or 0
  local x0 = x + (BUTTON_W - tw - 2 * STAR_R) // 2
  if text ~= "" then screen:label(x0, BUTTON_Y + 1, tw, label_h(theme), text, st, "left") end
  widgets.star(screen, x0 + tw + STAR_R, BUTTON_Y + BUTTON_H // 2, STAR_R, star_colour)
end

-- A theme's `chase` ({ colors = { a, b }, lap = seconds, width = pixels }): round the
-- edge of `x, y, w, h` (a chosen button or tab), two tones fading into each other that
-- run round it once a lap. Nothing in a theme without one, nor on the pages in `STILL`.
local STILL = {
  -- (The arranger, Ideas and Song, where it would only distract.)
  arrange = true,
  mixer = true,
}
function widgets.chase(screen, theme, x, y, w, h)
  local c = theme.chase
  if not c or STILL[deckard.page()] then return end
  local a, b = c.colors[1], c.colors[2]
  local t = c.width or 2
  local lap = c.lap or 1.6
  -- The edge as one loop: along the top, down the right, back along the bottom, up the
  -- left; `t` pixels a step.
  local across, down = w - t, h - t
  local loop = 2 * (across + down)
  local turn = (deckard.now() / lap) % 1
  for p = 0, loop - 1, t do
    local px, py
    if p < across then
      px, py = x + p, y
    elseif p < across + down then
      px, py = x + w - t, y + p - across
    elseif p < 2 * across + down then
      px, py = x + w - t - (p - across - down), y + h - t
    else
      px, py = x, y + h - t - (p - 2 * across - down)
    end
    local k = 0.5 - 0.5 * math.cos(2 * math.pi * (p / loop - turn))
    screen:rect(px, py, t, t, widgets.blend(a, b, k))
  end
end

-- The display buttons' labels along the top of both screens. `specs[i]` for button i:
--   nil                              nothing
--   { text = "OCTAVE-" }             a button
--   { text = "PAD MODE", selected = true }   what's chosen
--   { text = "MUTE", pin = true }    the page's name, held
--   { arrows = true, text = "BANK 1", left = bool, right = bool }
--                                    two buttons (i and i + 1) stepping through a list
--   { text = "LOAD", disabled = true }   a button that does nothing now
--   { star = true }                  a ★ for its label (with selected / disabled)
--   { dx = 4, ... }                  moved right (a page whose buttons sit apart)
function widgets.buttons(screen, theme, specs)
  local skip = {}
  for i = 1, 8 do
    local b = specs[i]
    local x = widgets.button_x(i) + (b and b.dx or 0)
    if b and not skip[i] then
      if b.arrows then
        skip[i + 1] = true
        local w = 2 * BUTTON_W + 4
        screen:frame(x, BUTTON_Y, w, BUTTON_H, "button", theme.corner)
        screen:triangle(x + 8, 11, 8, 10, "left", b.left and "text" or "arrow_off")
        screen:triangle(x + 218, 11, 8, 10, "right", b.right and "text" or "arrow_off")
        screen:label(x + 27, BUTTON_Y + 4, w - 54, 18, b.text or "", style(theme, "head"), "center", "top")
      elseif b.selected or b.pin then
        screen:rect(x, BUTTON_Y, BUTTON_W, BUTTON_H, b.selected and "button_selected" or "button", theme.corner)
        if b.selected then widgets.chase(screen, theme, x, BUTTON_Y, BUTTON_W, BUTTON_H) end
        local st = b.selected and "head_selected" or "head"
        label(screen, theme, x, b, style(theme, st), theme.styles[st].color)
      elseif b.star or (b.text and b.text ~= "") then
        local c = b.disabled and "button_off" or "button"
        screen:frame(x, BUTTON_Y, BUTTON_W, BUTTON_H, c, theme.corner)
        label(screen, theme, x, b, style(theme, "head", b.disabled and c or nil), b.disabled and c or "text")
      end
    end
  end
end

-- Display buttons 1 and 2 as a pair of tabs (`names`), the page's own (`pinned`)
-- filled as a held button's name is; lit when the page is pinned (`on`).
function widgets.pin_tabs(screen, theme, on, names, pinned)
  screen:frame(4, BUTTON_Y, 2 * 117, BUTTON_H, "button", theme.corner)
  for i, name in ipairs(names) do
    local x = 4 + (i - 1) * 117
    local lit = i == pinned and on
    if i == pinned then screen:rect(x, BUTTON_Y, 117, BUTTON_H, lit and "button_selected" or "button", theme.corner) end
    if lit then widgets.chase(screen, theme, x, BUTTON_Y, 117, BUTTON_H) end
    screen:label(x, BUTTON_Y + 1, 117, label_h(theme), name, style(theme, lit and "head_selected" or "head"))
  end
end

-- The info bar as most pages show it: the focused sound, the tempo and the scene.
function widgets.focus_info(s)
  local group = s.groups[s.group]
  local snd = group and group.sounds[s.sound]
  return {
    number = s.sound,
    palette = snd and snd.palette,
    name = snd and snd.name or "",
    values = {
      { text = string.format("%.2f", s.tempo) },
      { text = "BPM", title = true },
      { text = s.scene_name, palette = s.scene_palette },
    },
  }
end

-- The project's colours by name (palette index + 1), as the COLOR knobs show them.
widgets.COLOUR_NAMES = {
  "Red", "Orange", "Light Orange", "Warm Yellow", "Yellow", "Lime", "Green", "Mint",
  "Turquoise", "Cyan", "Blue", "Plum", "Violet", "Purple", "Magenta", "Fuchsia",
}

-- The bar under the left screen's buttons: the focused sound's number (in its colour)
-- and name, and on the right `values`, laid out from the right edge, each
-- { text, title = true (small and faint) | palette = n (in that colour) }.
function widgets.info_bar(screen, theme, info)
  -- (What the jog wheel just changed shows instead for a moment.)
  local quick = quick_edit.values(deckard.state())
  if quick then info = { number = info.number, palette = info.palette, name = info.name, values = quick } end
  screen:rect(0, 35, 480, 24, "bar")
  screen:rect(0, 35, 24, 24, widgets.colour(theme, info.palette))
  screen:label(0, 35, 24, 24, tostring(info.number or ""), style(theme, "info_number"))
  local right = 470
  for i = #(info.values or {}), 1, -1 do
    local v = info.values[i]
    local st = v.title and style(theme, "info_title") or style(theme, "default", v.palette and widgets.colour(theme, v.palette, false, true))
    local w = screen:text_width(v.text, st)
    if v.title then
      screen:label(right - w, 39, w, 16, v.text, st, "center", "bottom")
    else
      screen:label(right - w, 38, w, 18, v.text, st, "center", "bottom")
    end
    right = right - w - (v.title and 4 or 8)
  end
  screen:label(34, 38, right - 34 - 4, 18, info.name or "", style(theme, "default"), "left", "bottom")
end

-- For a theme without the info bar (`info_in_tabs`): along the row at `y`, ending
-- under display button `button` (default 4) and starting no further left than `least`
-- (default 300), the focused group's or sound's number in a tile of its colour and its
-- name, or for a moment what the jog wheel just changed (the tempo shows only then).
-- Nothing for the master. Returns where it starts.
function widgets.info_on_row(screen, theme, info, y, button, least)
  -- (The ink ending two columns inside the right border of the button above: bright
  -- text in or against the border's own column looked past the dim line.)
  local right = widgets.button_x(button or 4) + BUTTON_W - 2
  least = least or 300
  local quick = quick_edit.values(deckard.state())
  if quick then
    for i = #quick, 1, -1 do
      local v = quick[i]
      local st = v.title and style(theme, "info_title") or style(theme, "default", v.palette and widgets.colour(theme, v.palette, false, true))
      local w = screen:text_width(v.text, st)
      if right - w < least then break end
      -- (The rightmost one's ink on the edge, as the name's.)
      local ink = i == #quick and screen:text_bearing(v.text, st) or 0
      if v.title then
        screen:label(right - w + ink, y + 4, w, 16, v.text, st, "center", "bottom")
      else
        screen:label(right - w + ink, y + 3, w, 18, v.text, st, "center", "bottom")
      end
      right = right - w - (v.title and 4 or 8)
    end
    return right
  end
  local name, number = info.name or "", tostring(info.number or "")
  if name == "" and number == "" then return right end
  local st = style(theme, "default")
  local w = math.min(screen:text_width(name, st), right - least - 30)
  -- (The last letter's ink, not its width, on the edge: "Lead" and "Sound 1" alike.)
  local ink = screen:text_bearing(name, st)
  screen:label(right - w + ink, y, w, 24, name, st, "right", "center")
  local tw = math.max(22, screen:text_width(number, style(theme, "info_number")) + 8)
  local tx = right - w - (w > 0 and 8 or 0) - tw
  local fill = widgets.colour(theme, info.palette)
  screen:rect(tx, y + 1, tw, 22, fill, theme.corner)
  screen:label(tx, y + 1, tw, 22, number, style(theme, "info_number", widgets.on_text(theme, fill)))
  return tx
end

-- One tile (a group, or a pad): filled in its colour when `on`, else faint, with
-- `big` (a group's letter) and `name` in the corner.
local function tile(screen, theme, x, y, w, h, t)
  if t.empty then
    screen:rect(x, y, w, h, "pad_empty", theme.corner)
    return
  end
  -- A tile that isn't chosen is near-neutral dark grey with a trace of its colour,
  -- under text in its colour: a dimmed fill of the same hue put red text on red.
  local fill = widgets.colour(theme, t.palette)
  if not t.on then
    fill = widgets.blend(fill, theme.colors.tile_off or { 0x1c, 0x1c, 0x1c }, theme.tile_tint or 0.08)
  end
  screen:rect(x, y, w, h, fill, theme.corner)
  local text = t.on and theme.colors.on_colour or widgets.colour(theme, t.palette, false, true)
  if not t.on and contrast(text, fill) < 3 then text = theme.colors.on_colour end
  if t.big then
    screen:label(x + 6, y + 3, w - 6, 30, t.big, style(theme, "tile_letter", text), "left", "top")
    screen:label(x + 7, y + 33, w - 21, 18, t.name or "", style(theme, "default", text), "left", "top")
  else
    local st = style(theme, "pad_label", text)
    local first, second = widgets.wrap(screen, t.name or "", st, w - 9)
    screen:label(x + 7, y + 4, w - 9, 16, first, st, "left", "top")
    if second ~= "" then
      screen:label(x + 7, y + 21, w - 9, 16, second, st, "left", "top")
    end
  end
end

-- `s` in two lines `w` wide in style `st`: as it is when it fits, else broken after the
-- last space, "_" or "-" that fits (within a character, with none), so the end of a long
-- name ("ClosedHH Bionic 2", "… 3") shows. The second line is cut as a label is.
function widgets.wrap(screen, s, st, w)
  if s == "" or screen:text_width(s, st) <= w then return s, "" end
  local fit = 0
  for i = 1, #s do
    if screen:text_width(s:sub(1, i), st) > w then break end
    fit = i
  end
  if fit == 0 then return s, "" end
  local cut = fit
  for i = fit, 2, -1 do
    local c = s:sub(i, i)
    if c == " " or c == "_" or c == "-" then
      cut = i
      break
    end
  end
  return (s:sub(1, cut):gsub("%s+$", "")), (s:sub(cut + 1):gsub("^%s+", ""))
end

-- Eight group tiles in two rows of four (`tiles[1..8]`, nil for none), from `y`,
-- each `h` high; `dx` moves them across.
function widgets.group_tiles(screen, theme, tiles, y, h, dx)
  for i = 1, 8 do
    local t = tiles[i]
    if t then
      local x = 4 + (dx or 0) + (i - 1) % 4 * 119
      tile(screen, theme, x, y + (i - 1) // 4 * (h + 4), 115, h, t)
    end
  end
end

-- The right screen's 4 × 4 pads (`pads[1..16]`, pad 1 bottom left): each
-- { name, palette, on }, { empty = true } or { add = true } (a "+" for a new one).
function widgets.pad_grid(screen, theme, pads)
  for i = 1, 16 do
    local t = pads[i] or { empty = true }
    local x = 484 + (i - 1) % 4 * 119
    local y = 213 - (i - 1) // 4 * 59
    if t.add then
      screen:rect(x, y, 115, 54, "pad_add", theme.corner)
      screen:label(x + 7, y + 4, 106, 16, "+", style(theme, "pad_label"), "left", "top")
    else
      tile(screen, theme, x, y, 115, 54, t)
    end
    if t.empty and t.name then
      screen:label(x + 7, y + 4, 106, 16, t.name, style(theme, "pad_label", "pad_empty_text"), "left", "top")
    end
  end
end

-- The knobs' parameters along the bottom of one screen (`x0` 0 or 480): `groups`, the
-- names over them ({ text, at = 1-4 }), `page` ("1/2"), and `params[1..4]`, each
-- { name, value, unit } or nil.
-- The macro icon (a knob: a ring and its pointer) centred at `cx`, `cy`.
local function macro_icon(screen, cx, cy, colour)
  local r, n = 5, 16
  for i = 0, n - 1 do
    local a, b = i / n * 2 * math.pi, (i + 1) / n * 2 * math.pi
    screen:line(cx + math.cos(a) * r, cy + math.sin(a) * r, cx + math.cos(b) * r, cy + math.sin(b) * r, 1.2, colour)
  end
  screen:line(cx, cy, cx + math.cos(-0.8) * 4.5, cy + math.sin(-0.8) * 4.5, 1.2, colour)
end

-- (`params[k].macro`: a macro controls it, the macro icon at the top right.)
-- (`params[k].modulated`: automated in the pattern, a dot at the top right.)
-- (`pictured[k]`: knob k is in a picture, not a dial.)
function widgets.params(screen, theme, x0, groups, page, params, pictured)
  groups = groups or {}
  -- (In a centred theme, one row on every page just over the knobs' names, whatever is
  -- above it (a dial, a picture, tiles), in the theme's `section` style: not to be read
  -- as one more knob's name. The dials and pictures end higher to leave it room.)
  local own_row = theme.knobs_centred
  for i, g in ipairs(groups) do
    local x = x0 + (g.at - 1) * 120
    if own_row then
      if g.text and g.text ~= "" then
        local last = (groups[i + 1] and groups[i + 1].at or 5) - 1
        local to = x0 + last * 120 - 10
        local st = style(theme, theme.styles.section and "section" or "param_name")
        screen:label(x + 11, 217, to - x - 11, 14, g.text:upper(), st, "left", "center")
      end
    else
      screen:label(x + 12, 207, 228, 24, g.text, style(theme, "default"), "left", "center")
    end
  end
  if page then
    screen:label(x0 + 372, 207, 96, 24, page, style(theme, "param_page"), "right", "center")
  end
  -- (With sections in a centred theme, the lines between knobs only where a section
  -- starts: how far each one reaches.)
  local starts
  if own_row then
    for _, g in ipairs(groups) do
      if g.text and g.text ~= "" then starts = {} end
    end
    for _, g in ipairs(starts and groups or {}) do starts[g.at] = true end
  end
  for k = 1, 4 do
    local p = params[k]
    if p then
      local x = x0 + (k - 1) * 120
      if k > 1 and (not starts or starts[k]) then screen:rect(x, 233, 1, 39, "separator") end
      local icons = (p.macro and 1 or 0) + (p.modulated and 1 or 0)
      local name_w = icons == 2 and 78 or (icons == 1 and 90 or 100)
      if theme.knobs_centred and not theme.knobs_left then
        -- (Centred under the knob's dial; a unit runs on after the value.)
        screen:label(x + 6, 233, 108, 14, p.name or "", style(theme, "param_name"), "center", "top")
        local v = p.value or ""
        if p.unit and p.unit ~= "" then v = v .. " " .. p.unit end
        screen:label(x + 4, 248, 112, 27, v, style(theme, "param_value"), "center", "top")
        if p.macro then macro_icon(screen, x + 108, 240, style(theme, "param_name").color) end
        if p.modulated then screen:circle(x + (p.macro and 96 or 108), 240, 3.5, "record") end
        goto continue
      end
      screen:label(x + 11, 233, name_w, 14, p.name or "", style(theme, "param_name"), "left", "top")
      if p.macro then macro_icon(screen, x + 108, 240, style(theme, "param_name").color) end
      if p.modulated then screen:circle(x + (p.macro and 96 or 108), 240, 3.5, "record") end
      local w = screen:label(x + 11, 248, 104, 27, p.value or "", style(theme, "param_value"), "left", "top")
      if p.unit and p.unit ~= "" then
        screen:label(x + 11 + w + 6, 248, 104 - w - 6, 25, p.unit, style(theme, "param_unit"), "left", "bottom")
      end
    end
    ::continue::
  end
end

-- The focused group's sounds as the pad grid shows them: the focused one filled.
function widgets.sound_pads(s)
  local group = s.groups[s.group]
  local out = {}
  for i = 1, 16 do
    local snd = group and group.sounds[i]
    if snd and snd.used then
      out[i] = { name = snd.name, palette = snd.palette, on = snd.selected or i == s.sound }
    elseif (snd and snd.selected) or i == s.sound then
      out[i] = { name = "", palette = 15, on = true }
    else
      out[i] = { empty = true, name = "" }
    end
  end
  return out
end

-- The groups of the focused bank as tiles ("A1".."H1"); `on(i, group)` says which are
-- filled.
function widgets.group_bank(s, on)
  local bank = (s.group - 1) // 8
  local out = {}
  for i = 1, 8 do
    local g = bank * 8 + i
    local group = s.groups[g]
    if group then
      out[i] = {
        big = string.char(string.byte("A") + i - 1) .. (bank + 1),
        name = group.name,
        palette = group.palette,
        on = on(g, group),
      }
    end
  end
  return out, bank + 1
end

-- What knobs 5-8 show above them while touched, as Maschine's: a box with its picture
-- ("#" lit, "+" its edge) or text.
local KNOB_ICONS = {
  zoom = {
    "......................",
    "......................",
    "......................",
    "...........+####......",
    "............+###......",
    "............####......",
    "...........+#++#......",
    "..........+##..+......",
    "......+..+#+..........",
    "......#++#+...........",
    "......###+............",
    "......###+............",
    "......####+...........",
    "......................",
    "......................",
    "......................",
  },
  scroll = {
    "......................",
    "......................",
    "......................",
    "......................",
    "......................",
    ".......+......+.......",
    "......##......##......",
    "....+############+....",
    "....+############+....",
    "......##......##......",
    ".......+......+.......",
    "......................",
    "......................",
    "......................",
    "......................",
    "......................",
  },
  pitch = {
    "......................",
    "..........++..........",
    "..........##..........",
    ".........####.........",
    "........+####+........",
    "..........##..........",
    "..........##..........",
    "..........##..........",
    "..........##..........",
    "..........##..........",
    "..........##..........",
    "........+####+........",
    ".........####.........",
    "..........##..........",
    "..........++..........",
    "......................",
  },
}
local ICON_W, ICON_H, ICON_Y = 24, 18, 250
local ICON_FRAME, ICON_FILL, ICON_LIT, ICON_EDGE = { 0x89, 0x89, 0x89 }, { 0x13, 0x13, 0x13 }, { 0xa9, 0xa9, 0xa9 }, { 0x63, 0x63, 0x63 }

-- Knob `k`'s (5-8) box over it: icon `icon` (`zoom`, `scroll`, `pitch`) or a few
-- letters ("2×").
function widgets.knob_icon(screen, theme, k, icon)
  local x = 480 + (k - 5) * 120 + 46
  screen:rect(x, ICON_Y, ICON_W, ICON_H, ICON_FILL)
  screen:frame(x, ICON_Y, ICON_W, ICON_H, ICON_FRAME)
  local bitmap = KNOB_ICONS[icon]
  if bitmap then
    for row, line in ipairs(bitmap) do
      for col = 1, #line do
        local ch = line:sub(col, col)
        if ch ~= "." then screen:rect(x + col, ICON_Y + row, 1, 1, ch == "#" and ICON_LIT or ICON_EDGE) end
      end
    end
  else
    screen:label(x, ICON_Y, ICON_W, ICON_H, icon, { font = theme.styles.default.font, size = 11, color = ICON_LIT })
  end
end

return widgets
