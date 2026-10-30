-- Maschine: DECKARD's default theme, the colours of Maschine's own hardware screens
-- (black, grey buttons, the project's colours), drawn with Roboto.
--
-- A theme is the stylesheet every page draws with. Pages use these colour and
-- style names, so changing them here restyles every page.

-- The project's 16 colours and white, as the screens fill with them.
local PALETTE = {
  { 0xff, 0x13, 0x0f }, { 0xff, 0x3c, 0x14 }, { 0xff, 0x78, 0x00 }, { 0xff, 0xb9, 0x00 },
  { 0xff, 0xff, 0x00 }, { 0x92, 0xff, 0x00 }, { 0x26, 0xff, 0x26 }, { 0x00, 0xd1, 0x81 },
  { 0x00, 0xb4, 0xe9 }, { 0x00, 0x78, 0xff }, { 0x00, 0x48, 0xff }, { 0x82, 0x00, 0xff },
  { 0xa2, 0x00, 0xc8 }, { 0xf5, 0x00, 0xc8 }, { 0xff, 0x00, 0x78 }, { 0xf7, 0x07, 0x3e },
  { 0xff, 0xff, 0xff },
}

-- The same colours as text (the first is a little bluer).
local PALETTE_TEXT = { { 0xff, 0x00, 0x1e } }
for i = 2, #PALETTE do PALETTE_TEXT[i] = PALETTE[i] end

return {
  -- Colours: {red, green, blue} or {red, green, blue, opacity}, 0-255.
  colors = {
    background = { 0, 0, 0 },
    -- Display buttons: their outline, a mode's label, what's chosen.
    button = { 0x47, 0x47, 0x47 },
    button_text = { 255, 255, 255, 229 },
    button_selected = { 255, 255, 255, 178 },
    button_selected_text = { 0, 0, 0 },
    arrow_off = { 0x47, 0x47, 0x47 },
    -- A display button that does nothing now.
    button_off = { 41, 41, 41 },
    -- The bar under the buttons, and text.
    bar = { 0x1a, 0x1a, 0x1a },
    text = { 255, 255, 255 },
    weak = { 255, 255, 255, 128 },
    -- Text on a filled colour.
    on_colour = { 0, 0, 0 },
    -- An empty pad, and the line between knobs' parameters.
    pad_empty = { 0x10, 0x10, 0x10 },
    pad_empty_text = { 0xa0, 0xa0, 0xa0 },
    -- A pad that makes a new pattern or scene.
    pad_add = { 0x2a, 0x2a, 0x2a },
    separator = { 0x77, 0x6f, 0x6f },
    -- The Mixer: long names, the headroom, the scale, the pan's track, the 0 dB line,
    -- a focused strip's body, a muted strip, a muted value, the "+" for a new group.
    mixer_name = { 220, 220, 220 },
    mixer_headroom = { 80, 80, 80 },
    mixer_scale = { 95, 95, 95 },
    mixer_track = { 48, 48, 48 },
    mixer_zero = { 85, 85, 85 },
    mixer_focused = { 13, 13, 13 },
    muted = { 71, 71, 71 },
    mixer_value_muted = { 90, 90, 90 },
    mixer_add = { 110, 110, 110 },
    -- Arrange's Ideas view: its background, the line under the scenes (and the
    -- focused scene's tab), the edge between tabs, a group tile's top edge, what a
    -- pattern not in the scene is faded over, a group's name.
    ideas_bg = { 8, 8, 8 },
    -- Browse: the brackets round the parameter the jog wheel turns, a parameter with
    -- nothing to choose, "All …", a result's row (the focused one's is "text"), a
    -- product's tile edge.
    browse_bracket = { 179, 179, 179 },
    browse_param_none = { 102, 102, 102 },
    browse_param_all = { 255, 255, 255, 102 },
    browse_row = { 26, 26, 26 },
    browse_tile = { 0x47, 0x47, 0x47 },
    ideas_line = { 40, 40, 40 },
    ideas_tab_edge = { 15, 15, 15 },
    ideas_tile_top = { 14, 14, 14 },
    ideas_faded = { 30, 30, 30 },
    ideas_group_name = { 255, 255, 255, 153 },
    -- Arrange's Song view: its background, a bar ruler's ticks and numbers, empty bars
    -- and the line between them, the line where a section ends (or its pattern
    -- repeats), a section's tab (the focused one's, the edge between tabs, its name).
    song_bg = { 16, 16, 16 },
    song_tick = { 132, 132, 132 },
    song_empty = { 26, 26, 26 },
    song_bar_line = { 17, 17, 17 },
    song_section_edge = { 33, 33, 33 },
    song_tab = { 51, 51, 51 },
    song_tab_focus = { 90, 90, 90 },
    song_tab_edge = { 26, 26, 26 },
    song_tab_text = { 170, 170, 170 },
    -- Sampling: a sample's waveform outside the play range (and outside every slice),
    -- the selection's and the focused slice's ground, a slice's start, an empty
    -- meter, the threshold and markers, the zone map's white and black keys and the
    -- lines between keys.
    wave_off = { 34, 34, 34 },
    wave_unsliced = { 89, 89, 89 },
    wave_selection = { 16, 16, 16 },
    slice_focus = { 41, 41, 41 },
    slice_marker = { 255, 255, 255, 153 },
    meter_empty = { 38, 38, 38 },
    marker = { 255, 255, 255 },
    zone_white = { 53, 53, 53 },
    zone_black = { 47, 47, 47 },
    zone_line = { 38, 38, 38 },
    -- DECKARD's own pages that Maschine hasn't.
    panel = { 0x2a, 0x2a, 0x2a },
    accent = { 0xff, 0xa5, 0x2a },
    record = { 0xe0, 0x3c, 0x3c },
  },

  -- The project's colours (palette index + 1), as fills and as text.
  palette = PALETTE,
  palette_text = PALETTE_TEXT,
  -- What's not chosen shows its colour this bright (0..1).
  unfocused = 0.15,
  -- Corner pixels' opacity: a little rounding.
  corner = 0.65,
  -- How strongly a pattern not in the scene (and a group's bars) shows its colour.
  faded = 0.3,

  -- Font roles: ubuntu, hack, roboto, roboto-light, roboto-medium, roboto-bold,
  -- roboto-mono, roboto-mono-medium, roboto-mono-bold.
  fonts = {
    regular = "roboto-medium",
    mono = "roboto-mono-medium",
  },

  -- Text styles: font, size (pixels) and color; what a style leaves out comes from
  -- `default`. Roboto draws a little smaller than Maschine's own font, so the sizes
  -- are Maschine's and 8 % more.
  styles = {
    default = { font = "regular", size = 15, color = "text" },
    head = { color = "button_text" },
    head_selected = { color = "button_selected_text" },
    info_number = { size = 14, color = "on_colour" },
    info_title = { size = 12, color = "weak" },
    tile_letter = { size = 27 },
    pad_label = { size = 14 },
    param_page = { color = "weak" },
    param_name = { size = 12, color = "weak" },
    param_value = { size = 21.5 },
    param_unit = { size = 16, color = "weak" },
    mixer_name = { size = 14 },
    mixer_headroom = { size = 12, color = "mixer_headroom" },
    mixer_scale = { size = 9.7, color = "mixer_scale" },
    mixer_value = { size = 21.5 },
    mixer_add = { size = 19.5, color = "mixer_add" },
    mixer_fx = { size = 7.5 },
    pattern_cell = { size = 13, color = "on_colour" },
    song_ruler = { size = 10, color = "song_tick" },
    song_group_on = { size = 10, color = "on_colour" },
    song_tab = { size = 14, color = "song_tab_text" },
    song_tab_on = { size = 14 },
    browse_logo = { size = 30 },
    -- DECKARD's own pages.
    button_label = { color = "weak" },
    button_label_active = { color = "background" },
    title = { size = 18 },
    number = { font = "mono" },
    page = { color = "weak" },
    knob_name = { size = 14, color = "weak" },
    knob_value = { font = "mono", size = 14 },
  },

  -- Sizes and spacing.
  header_height = 32,
  knob_bar_height = 6,
}
