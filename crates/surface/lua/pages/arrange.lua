-- Arrange (ARRANGE): the Ideas view. The scenes along the top, each group's patterns
-- in a column under them (those the focused scene plays lit), the groups along the
-- bottom.
--
--   display 1-8       focus scene 1-8 (of the eight shown)
--   LEFT / RIGHT      the previous / next eight scenes
--   jog wheel         the focused group plays its previous / next pattern in the scene
--   jog wheel up / down
--                     the same, a pattern at a time
--   jog wheel left / right
--                     focus the previous / next group
--   knobs 1-8         the group above plays its next / previous pattern in the focused
--                     scene, round in a circle: after the last none, then the first
--
-- With SHIFT, the display buttons: IDEAS / SONG (this view, or the Song view:
-- pages/song.lua), CLEAR (no group plays a pattern in the focused scene), DUPLICATE,
-- CREATE (a new scene after the focused), DELETE, and the previous / next eight
-- scenes. After DUPLICATE, CREATE or DELETE they're the scenes again until SHIFT is
-- let go.

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local song = require("pages.song")

local arrange = {}

local COLUMNS = 8

-- The scenes shown: the eight from `scene_bank`.
local scene_bank = 1

-- An edit made with SHIFT: the scenes are back on the buttons until it's let go.
local shift_done = false

-- Whether the display buttons are SHIFT's.
local function shift_layer()
  if not deckard.held("SHIFT") then shift_done = false end
  return deckard.held("SHIFT") and not shift_done
end

local function scene_banks(s)
  return (#s.scenes + 1 + COLUMNS - 1) // COLUMNS
end

-- Whether the focused scene has patterns in it.
local function scene_used(s)
  local sc = s.scenes[s.scene]
  if not sc then return false end
  for _, p in pairs(sc.patterns) do
    if p then return true end
  end
  return false
end

-- SHIFT's display button `n`.
local function shift_button(s, n)
  if n == 2 then
    deckard.song_view(true)
  elseif n == 3 then
    if scene_used(s) then deckard.scene_edit("clear") end
  elseif n >= 4 and n <= 6 and s.scenes[s.scene] then
    deckard.scene_edit(({ "duplicate", "insert", "delete" })[n - 3])
    shift_done = true
  elseif n == 7 then
    if scene_bank > 1 then scene_bank = scene_bank - 1 end
  elseif n == 8 then
    if scene_bank < scene_banks(s) then scene_bank = scene_bank + 1 end
  end
end

-- The focused group plays its pattern `delta` patterns on (from the one focused).
local function step_pattern(s, delta)
  local group = s.groups[s.group]
  if not group or #group.patterns == 0 then return end
  local to = math.max(1, math.min(#group.patterns, s.pattern + delta))
  if to ~= s.pattern or group.scene_pattern ~= to then deckard.pick_pattern(to) end
end

function arrange.button(name, down)
  local s = deckard.state()
  if s.song_view then return song.button(name, down) end
  if name == "SHIFT" then shift_done = false end
  if not down then return end
  if shift_layer() then
    local n = tonumber(name:match("^SCREEN_(%d)$"))
    if n then
      shift_button(s, n)
      return true
    end
  end
  if name == "LEFT" then
    if scene_bank > 1 then scene_bank = scene_bank - 1 end
  elseif name == "RIGHT" then
    if scene_bank * COLUMNS <= #s.scenes then scene_bank = scene_bank + 1 end
  elseif name == "WHEEL_UP" then
    step_pattern(s, -1)
  elseif name == "WHEEL_DOWN" then
    step_pattern(s, 1)
  elseif name == "WHEEL_LEFT" then
    if s.group > 1 then deckard.focus_group(s.group - 1) end
  elseif name == "WHEEL_RIGHT" then
    if s.group < #s.groups then deckard.focus_group(s.group + 1) end
  else
    local n = tonumber(name:match("^SCREEN_(%d)$"))
    if not n then return end
    local scene = (scene_bank - 1) * COLUMNS + n
    if s.scenes[scene] then deckard.focus_scene(scene) end
  end
  return true
end

function arrange.wheel(steps)
  local s = deckard.state()
  if s.song_view then return song.wheel and song.wheel(steps) end
  step_pattern(s, steps)
  return true
end

-- The first of the eight groups shown (the focused group's eight).
local function first_group(s)
  return (s.group - 1) // COLUMNS * COLUMNS + 1
end

function arrange.knob(n, amount)
  local s = deckard.state()
  if s.song_view then return song.knob(n, amount) end
  local steps = knobs.gather(n, amount)
  local g = first_group(s) + n - 1
  local group = s.groups[g]
  if steps == 0 or not group or #group.patterns == 0 or not s.scenes[s.scene] then return true end
  -- (None is the place after the last.)
  local places = #group.patterns + 1
  local at = group.scene_pattern or places
  local to = (at - 1 + steps) % places + 1
  deckard.scene_pattern(g, to < places and to or nil)
  return true
end

function arrange.pad(n, velocity)
  if deckard.state().song_view then return song.pad(n, velocity) end
end

function arrange.pads()
  if deckard.state().song_view then return song.pads() end
end

function arrange.leds()
  local s = deckard.state()
  if s.song_view then return song.leds() end
  if shift_layer() then
    deckard.led("SCREEN_1", "on")
    deckard.led("SCREEN_2", "dim")
    deckard.led("SCREEN_3", scene_used(s) and "dim" or "off")
    for i = 4, 6 do deckard.led("SCREEN_" .. i, s.scenes[s.scene] and "dim" or "off") end
    deckard.led("SCREEN_7", scene_bank > 1 and "dim" or "off")
    deckard.led("SCREEN_8", scene_bank < scene_banks(s) and "dim" or "off")
  end
  for i = 1, COLUMNS do
    local scene = (scene_bank - 1) * COLUMNS + i
    if s.scenes[scene] and not shift_layer() then deckard.led("SCREEN_" .. i, scene == s.scene and "on" or "dim") end
  end
  deckard.led("ARRANGE", "on")
  deckard.led("LEFT", scene_bank > 1 and "dim" or "off")
  -- (A full bank: the next has room for a new scene.)
  deckard.led("RIGHT", scene_bank * COLUMNS <= #s.scenes and "dim" or "off")
  deckard.led("WHEEL_UP", 16, false)
  deckard.led("WHEEL_DOWN", 16, false)
  if s.group > 1 then deckard.led("WHEEL_LEFT", 16, false) end
  if s.group < #s.groups then deckard.led("WHEEL_RIGHT", 16, false) end
end

-- The theme's style `name` in another colour.
local function style(theme, name, colour)
  local st = {}
  for k, v in pairs(theme.styles.default) do st[k] = v end
  for k, v in pairs(theme.styles[name] or {}) do st[k] = v end
  if colour then st.color = colour end
  return st
end

local function colour(theme, name)
  return theme.colors[name]
end

-- The scenes along the top: the focused one's tab grey under a line of its colour.
local function scenes(screen, theme, s)
  screen:rect(0, 31, 960, 3, "ideas_line")
  for i = 1, COLUMNS do
    local n = (scene_bank - 1) * COLUMNS + i
    local scene = s.scenes[n]
    if not scene then break end
    local x = (i - 1) * 120
    local text
    if n == s.scene then
      screen:rect(x, 0, 119, 3, widgets.colour(theme, scene.palette))
      screen:rect(x, 3, 119, 28, "ideas_line")
      text = "on_colour"
    else
      text = widgets.colour(theme, scene.palette, false, true)
    end
    screen:rect(x + 119, 0, 1, 31, "ideas_tab_edge")
    screen:label(x + 5, 6, 109, 21, scene.name, style(theme, "default", text))
  end
end

-- Each group's patterns in a column: lit where the focused scene plays them, faded
-- where not; the focused group's framed.
-- (Seven rows: the eighth is kept for a new pattern.)
local ROWS = 7
local function patterns(screen, theme, s, first_group)
  screen:rect(0, 34, 960, 1, "background")
  screen:rect(0, 35, 960, 192, "ideas_bg")
  for i = 1, COLUMNS do
    local g = first_group + i - 1
    local group = s.groups[g]
    if group then
      local x = (i - 1) * 120
      -- The rows scroll to keep the focused pattern in view.
      local top = 0
      if g == s.group and s.pattern > ROWS then top = s.pattern - ROWS end
      for row = 1, ROWS do
        local p = top + row
        local pattern = group.patterns[p]
        if not pattern then break end
        local y = 35 + (row - 1) * 24
        local lit = group.scene_pattern == p
        local fill
        if lit then
          fill = widgets.colour(theme, pattern.palette, false, true)
        else
          fill = widgets.blend(widgets.colour(theme, pattern.palette), colour(theme, "ideas_faded"), theme.faded)
        end
        screen:rect(x, y, 119, 23, fill, theme.corner)
        screen:rect(x, y + 23, 119, 1, "background")
        if g == s.group and p == s.pattern then screen:frame(x, y, 119, 23, "text", theme.corner) end
        screen:label(x + 7, y, 110, 23, pattern.name, "pattern_cell", "left")
      end
    end
  end
  screen:rect(0, 227, 960, 1, "background")
end

-- The groups along the bottom: the focused one filled in its colour; under each name,
-- its scene pattern's bars (up to 8), filled as it plays.
local BARS_W = 107
local function groups(screen, theme, s, first_group)
  for i = 1, COLUMNS do
    local g = first_group + i - 1
    local group = s.groups[g]
    if not group then break end
    local x = (i - 1) * 120
    local focused = g == s.group
    local c = widgets.colour(theme, group.palette, false, true)
    local base = focused and c or colour(theme, "ideas_bg")
    screen:rect(x, 228, 119, 44, base)
    if not focused then screen:rect(x, 228, 119, 1, "ideas_tile_top") end
    local letter = string.char(string.byte("A") + (g - 1) % 8) .. ((g - 1) // 8 + 1)
    local letter_c = focused and "on_colour" or (group.muted and "muted" or c)
    screen:label(x + 5, 235, 25, 12, letter, style(theme, "default", letter_c))
    local name_c = focused and "on_colour" or "ideas_group_name"
    screen:label(x + 32, 235, 80, 12, group.name, style(theme, "default", name_c), "left")
    -- The bars.
    local pattern = group.scene_pattern and group.patterns[group.scene_pattern]
    local fill = widgets.colour(theme, group.palette)
    local unfilled = focused and widgets.blend({ 0, 0, 0 }, base, 0.4) or widgets.blend(fill, base, theme.faded)
    local filled = focused and widgets.blend({ 0, 0, 0 }, base, 0.9) or c
    if not pattern then
      screen:rect(x + 6, 258, BARS_W, 6, widgets.blend({ 0, 0, 0 }, base, 0.25))
    else
      local n = math.max(1, math.min(8, math.ceil(pattern.bars - 1e-9)))
      -- (Every group's, so their lengths and places compare.)
      local played = group.progress and math.floor(group.progress * BARS_W) or 0
      for b = 0, n - 1 do
        local x0 = (b * (BARS_W + 1)) // n
        local x1 = ((b + 1) * (BARS_W + 1)) // n - 1
        screen:rect(x + 6 + x0, 258, x1 - x0, 6, unfilled)
        local f = math.min(x1, played) - x0
        if f > 0 then screen:rect(x + 6 + x0, 258, f, 6, filled) end
      end
    end
  end
end

-- SHIFT's display buttons, over the scenes.
local function shift_buttons(screen, theme, s)
  screen:rect(0, 0, 960, 35, "background")
  local scene = s.scenes[s.scene] ~= nil
  widgets.buttons(screen, theme, {
    [1] = { text = "IDEAS", selected = true },
    [2] = { text = "SONG" },
    [3] = { text = "CLEAR", disabled = not scene_used(s) },
    [4] = { text = "DUPLICATE", disabled = not scene },
    [5] = { text = "CREATE" },
    [6] = { text = "DELETE", disabled = not scene },
    [7] = { arrows = true, text = "BANK " .. scene_bank, left = scene_bank > 1, right = scene_bank < scene_banks(s) },
  })
end

function arrange.draw(screen, theme)
  local s = deckard.state()
  if s.song_view then return song.draw(screen, theme) end
  scenes(screen, theme, s)
  patterns(screen, theme, s, first_group(s))
  groups(screen, theme, s, first_group(s))
  if shift_layer() then shift_buttons(screen, theme, s) end
end

return arrange
