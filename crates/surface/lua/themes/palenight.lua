-- Palenight (the Material Theme's Palenight palette): deep blue-grey, soft lavender
-- text, purple and cyan accents.
--
-- A theme is the stylesheet every page draws with: copy a theme into your
-- controller folder's themes/ under a new name to make your own. This one builds on
-- the Maschine theme (`base`): what it leaves out comes from there.

local BG = { 0x0f, 0x11, 0x1a }
local DEEP = { 0x07, 0x08, 0x0d }
local PANEL = { 0x1e, 0x20, 0x30 }
local LINE = { 0x44, 0x42, 0x67 }
local COMMENT = { 0x67, 0x6e, 0x95 }
local MUTED = { 0x8a, 0x91, 0xb8 }
local TEXT = { 0xa6, 0xac, 0xcd }
local WHITE = { 0xee, 0xff, 0xff }
local PURPLE = { 0xc7, 0x92, 0xea }
local CYAN = { 0x89, 0xdd, 0xff }
local BLUE = { 0x82, 0xaa, 0xff }
local RED = { 0xff, 0x53, 0x70 }
local ORANGE = { 0xf7, 0x8c, 0x6c }

return {
  base = "maschine",

  colors = {
    background = BG,
    panel = PANEL,
    text = WHITE,
    weak = COMMENT,
    accent = BLUE,
    record = RED,
    on_colour = DEEP,
    button = LINE,
    button_text = TEXT,
    button_selected = PURPLE,
    button_selected_text = DEEP,
    button_off = PANEL,
    arrow_off = LINE,
    bar = DEEP,
    wave_off = PANEL,
    wave_selection = LINE,
    slice_focus = LINE,
    meter_empty = PANEL,
    marker = CYAN,
    zone_white = PANEL,
    zone_black = DEEP,
    zone_line = DEEP,
    pad_empty = PANEL,
    pad_empty_text = MUTED,
    pad_add = LINE,
    separator = COMMENT,
    mixer_name = TEXT,
    mixer_headroom = COMMENT,
    mixer_scale = COMMENT,
    mixer_track = PANEL,
    mixer_zero = MUTED,
    mixer_focused = DEEP,
    muted = COMMENT,
    mixer_value_muted = COMMENT,
    mixer_add = MUTED,
    ideas_bg = DEEP,
    ideas_line = LINE,
    ideas_tab_edge = DEEP,
    ideas_tile_top = LINE,
    ideas_faded = PANEL,
    ideas_group_name = TEXT,
    song_bg = DEEP,
    song_tick = COMMENT,
    song_empty = PANEL,
    song_bar_line = DEEP,
    song_section_edge = DEEP,
    song_tab = PANEL,
    song_tab_focus = LINE,
    song_tab_edge = DEEP,
    song_tab_text = TEXT,
    browse_bracket = CYAN,
    browse_param_none = COMMENT,
    browse_param_all = MUTED,
    browse_row = PANEL,
    browse_tile = LINE,
  },

  -- A chosen display button's border: these two tones fading into each other, running
  -- round its edge once every `lap` seconds, `width` pixels wide.
  chase = { colors = { ORANGE, CYAN }, lap = 1.6, width = 2 },
}
