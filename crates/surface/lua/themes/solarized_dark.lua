-- Solarized Dark (Ethan Schoonover's palette, ethanschoonover.com/solarized).
--
-- A theme is the stylesheet every page draws with: copy a theme into your
-- controller folder's themes/ under a new name to make your own. This one builds on
-- the Maschine theme (`base`): what it leaves out comes from there.

-- (Its two background tones darker than the palette's, so everything on them stands out.)
local BASE03 = { 0x00, 0x12, 0x17 }
local BASE02 = { 0x02, 0x1c, 0x22 }
local BASE01 = { 0x58, 0x6e, 0x75 }
local BASE00 = { 0x65, 0x7b, 0x83 }
local BASE0 = { 0x83, 0x94, 0x96 }
local BASE1 = { 0x93, 0xa1, 0xa1 }
local BLUE = { 0x26, 0x8b, 0xd2 }
local RED = { 0xdc, 0x32, 0x2f }

return {
  base = "maschine",

  colors = {
    background = BASE03,
    panel = BASE02,
    text = BASE1,
    weak = BASE01,
    accent = BLUE,
    record = RED,
    on_colour = BASE03,
    button = BASE01,
    button_text = BASE1,
    button_selected = BASE1,
    button_selected_text = BASE03,
    button_off = BASE02,
    arrow_off = BASE01,
    bar = BASE02,
    wave_off = BASE02,
    wave_selection = BASE02,
    slice_focus = BASE02,
    meter_empty = BASE02,
    marker = BASE1,
    zone_white = BASE02,
    zone_black = BASE03,
    zone_line = BASE03,
    pad_empty = BASE02,
    pad_empty_text = BASE0,
    pad_add = BASE02,
    separator = BASE01,
    mixer_name = BASE1,
    mixer_headroom = BASE01,
    mixer_scale = BASE00,
    mixer_track = BASE02,
    mixer_zero = BASE00,
    mixer_focused = BASE02,
    muted = BASE01,
    mixer_value_muted = BASE00,
    mixer_add = BASE00,
    ideas_bg = BASE03,
    ideas_line = BASE02,
    ideas_tab_edge = BASE03,
    ideas_tile_top = BASE02,
    ideas_faded = BASE02,
    ideas_group_name = BASE0,
    song_bg = BASE03,
    song_tick = BASE01,
    song_empty = BASE02,
    song_bar_line = BASE03,
    song_section_edge = BASE03,
    song_tab = BASE02,
    song_tab_focus = BASE01,
    song_tab_edge = BASE03,
    song_tab_text = BASE0,
    browse_bracket = BASE1,
    browse_param_none = BASE01,
    browse_param_all = BASE00,
    browse_row = BASE02,
    browse_tile = BASE01,
  },

  fonts = {
    regular = "ubuntu",
    mono = "hack",
  },
}
