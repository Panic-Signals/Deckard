-- A monotone LCD. The field and the pixels come from LCD_PAPER and LCD_INK (a real
-- panel's pair). Every project colour is the same pixel, so groups and sounds do
-- not bring their own hues onto the screen.

local paper = LCD_PAPER or { 0x1a, 0x14, 0x08 }
local ink = LCD_INK or { 0xff, 0xb0, 0x00 }

local function mix(a, b, t)
  local function m(i)
    return math.floor(a[i] * (1 - t) + b[i] * t + 0.5)
  end
  return { m(1), m(2), m(3) }
end

-- A half mix is where yellow/black and red/black turn to mud. Text stays near the
-- pixel. Panels and rules stay near the field, far enough to see, not a third colour.
local function lum(c)
  return 0.2126 * c[1] + 0.7152 * c[2] + 0.0722 * c[3]
end

local ghost = mix(paper, ink, 0.88)
local lift = mix(paper, ink, 0.10)
local line = mix(paper, ink, 0.24)
local rule = mix(paper, ink, 0.42)
-- The field brown does not read on a bright segment. Black does. On a dark segment
-- the field (the pale one, when reversed) does.
local on_fill = lum(ink) > lum(paper) and { 0, 0, 0 } or paper

local palette, palette_text = {}, {}
for i = 1, 17 do
  -- Fills and coloured labels are the pixel. Text that sits on a fill uses the field
  -- (`on_colour`, `mixer_on`), so a selected strip is never the pixel on itself.
  palette[i] = ink
  palette_text[i] = ink
end

return {
  base = "maschine",

  colors = {
    background = paper,
    panel = lift,
    text = ink,
    weak = ghost,
    accent = ink,
    record = ink,
    on_colour = on_fill,
    button = line,
    button_text = ink,
    button_selected = ink,
    button_selected_text = on_fill,
    button_off = lift,
    arrow_off = line,
    bar = paper,
    wave_off = lift,
    wave_unsliced = ghost,
    wave_selection = paper,
    slice_focus = lift,
    slice_marker = ink,
    meter_empty = lift,
    marker = ink,
    zone_white = lift,
    zone_black = paper,
    zone_line = line,
    pad_empty = lift,
    pad_empty_text = ghost,
    pad_add = line,
    separator = line,
    mixer_name = ink,
    -- The selected strip's name, drawn on the pixel-coloured header.
    mixer_on = on_fill,
    mixer_headroom = ghost,
    mixer_scale = ghost,
    mixer_track = lift,
    mixer_zero = ghost,
    mixer_focused = paper,
    muted = ghost,
    mixer_value_muted = ghost,
    mixer_add = ghost,
    ideas_bg = paper,
    ideas_line = ink,
    ideas_tab_edge = paper,
    ideas_tile_top = line,
    ideas_faded = lift,
    ideas_group_name = ink,
    song_bg = paper,
    song_tick = ghost,
    song_empty = lift,
    song_bar_line = paper,
    song_section_edge = line,
    song_tab = lift,
    song_tab_focus = ink,
    song_tab_edge = paper,
    song_tab_text = ink,
    -- A pattern block is filled with the pixel, so its name takes the field.
    song_ink = on_fill,
    -- Chain names and the pattern grid. Other themes leave these unset and keep the
    -- greys they already drew, so a dim LCD step does not dim Palenight.
    chain_idle = ghost,
    chain_mark = ghost,
    chain_fx = ghost,
    channel_icon = line,
    grid_step = line,
    grid_beat = rule,
    grid_bar = ghost,
    grid_row = lift,
    browse_bracket = ink,
    browse_param_none = ghost,
    browse_param_all = ghost,
    browse_row = lift,
    browse_tile = line,
  },

  palette = palette,
  palette_text = palette_text,
  -- Sharp corners: a segment doesn't have a rounded bezel.
  corner = 0,
  -- Dimmed pixels stay a segment. 0.55 of amber or red is the muddy middle.
  unfocused = 0.8,
  faded = 0.72,

  fonts = {
    regular = "roboto-mono-medium",
    mono = "roboto-mono-medium",
  },

  styles = {
    -- The focused section's name sits on `song_tab_focus` (the pixel).
    song_tab_on = { color = "on_colour" },
  },
}
