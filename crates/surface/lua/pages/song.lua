-- Arrange's Song view (ARRANGE, SHIFT + SONG): the arrangement's sections. The whole
-- song on the left screen, the sections close up on the right, each group's patterns
-- in its row; the pads are the sections.
--
--   display 1 / 2     SECTION / CLIP (SHIFT: IDEAS / SONG, the Ideas view or this)
--   display 3         CONVERT (to clips)
--   display 4-6       DUPLICATE, CREATE (a new one after the focused), DELETE
--   display 7 / 8     the previous / next sixteen sections on the pads; past the last,
--                     an empty bank for new ones
--   knob 1            POSITION: the focused section swaps places with the next / previous
--   knob 2            SCENE: it plays the next / previous scene
--   knob 4            LENGTH: a bar longer / shorter (SHIFT: a beat)
--   knob 5 / 6        zoom / scroll the right screen
--   LEFT / RIGHT      focus the previous / next section
--   pads              focus a section (while the song plays, it plays next); an empty
--                     pad makes a new one playing the focused scene; with ERASE, delete it

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local util = require("lib.util")

local song = {}

local PADS = 16
local QUARTER = 960

-- The section bank on the pads.
local bank = 1
-- The right screen: pixels a quarter note, and the tick at its left edge.
local zoom = 3.125
local scroll = 0
-- The section kept in view as the focus moves.
local shown = nil

local function bar_ticks(s)
  return QUARTER * 4 * math.max(s.numerator, 1) // math.max(s.denominator, 1)
end

local function beat_ticks(s)
  return QUARTER * 4 // math.max(s.denominator, 1)
end

local function banks(s)
  return math.max(1, (#s.sections + PADS - 1) // PADS, bank)
end

-- Whether display 7 / 8 have a bank to go to: past the last, a full one makes room
-- for an empty one.
local function can_bank(s, next)
  if not next then return bank > 1 end
  if bank < banks(s) then return true end
  return #s.sections >= bank * PADS
end

local function scene_name(s, sec)
  local sc = sec and sec.scene and s.scenes[sec.scene]
  return sc and sc.name or "-"
end

function song.button(name, down)
  if not down then return end
  local s = deckard.state()
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if deckard.held("SHIFT") then
    if n == 1 then deckard.song_view(false) end
    if n then return true end
  end
  if n == 4 then
    if s.section then deckard.section_edit("duplicate") end
  elseif n == 5 then
    deckard.section_edit("create")
  elseif n == 6 then
    if s.section then deckard.section_edit("delete") end
  elseif n == 7 or n == 8 then
    if can_bank(s, n == 8) then bank = bank + (n == 8 and 1 or -1) end
  elseif n then
    -- (SECTION is this; CLIP and CONVERT aren't on the controller yet.)
  elseif name == "LEFT" then
    if s.section and s.section > 1 then deckard.focus_section(s.section - 1) end
  elseif name == "RIGHT" then
    if s.section and s.section < #s.sections then deckard.focus_section(s.section + 1) end
  elseif name == "WHEEL" then
    -- (Maschine's Song view does nothing with it.)
  else
    return
  end
  return true
end

function song.knob(n, amount)
  local s = deckard.state()
  if n == 5 then
    zoom = util.clamp(zoom * 1.06 ^ (amount * 100), 0.25, 100)
    return true
  elseif n == 6 then
    scroll = math.max(0, scroll + math.floor(amount * 1000 / zoom * QUARTER))
    return true
  end
  local steps = knobs.gather(n, amount)
  local sec = s.section and s.sections[s.section]
  if steps == 0 or not sec then return true end
  local next = steps > 0
  if n == 1 then
    if (next and s.section < #s.sections) or (not next and s.section > 1) then deckard.move_section(next) end
  elseif n == 2 then
    local at = sec.scene or 0
    if (next and at < #s.scenes) or (not next and at > 1) then deckard.section_scene(next) end
  elseif n == 4 then
    local step = deckard.held("SHIFT") and beat_ticks(s) or bar_ticks(s)
    local to = sec.length + steps * step
    if to >= step and to ~= sec.length then deckard.section_length(to) end
  end
  return true
end

function song.pad(n)
  local s = deckard.state()
  local at = (bank - 1) * PADS + n
  if s.sections[at] then
    deckard.focus_section(at)
    if deckard.held("ERASE") then deckard.section_edit("delete") end
  elseif not deckard.held("ERASE") then
    deckard.scene_edit("append")
  end
  return true
end

function song.pads()
  return { play = false }
end

function song.leds()
  local s = deckard.state()
  if deckard.held("SHIFT") then
    deckard.led("SCREEN_1", "dim")
    deckard.led("SCREEN_2", "on")
  else
    deckard.led("SCREEN_1", "on")
    deckard.led("SCREEN_2", "dim")
    deckard.led("SCREEN_4", s.section and "dim" or "off")
    deckard.led("SCREEN_5", "dim")
    deckard.led("SCREEN_6", s.section and "dim" or "off")
    deckard.led("SCREEN_7", can_bank(s, false) and "dim" or "off")
    deckard.led("SCREEN_8", can_bank(s, true) and "dim" or "off")
  end
  deckard.led("LEFT", (s.section and s.section > 1) and "dim" or "off")
  deckard.led("RIGHT", (s.section and s.section < #s.sections) and "dim" or "off")
  for _, w in ipairs({ "WHEEL_UP", "WHEEL_DOWN", "WHEEL_LEFT", "WHEEL_RIGHT" }) do deckard.led(w, "off") end
  for i = 1, PADS do
    local at = (bank - 1) * PADS + i
    local sec = s.sections[at]
    if sec then
      local sc = sec.scene and s.scenes[sec.scene]
      deckard.led("PAD_" .. i, sc and sc.palette or 16, at == s.section)
    else
      deckard.led("PAD_" .. i, "off")
    end
  end
end

-- The pattern group `g` plays in section `sec` (nil for none).
local function pattern_of(s, sec, g)
  local sc = sec.scene and s.scenes[sec.scene]
  local p = sc and sc.patterns[g]
  local group = s.groups[g]
  return p and group and group.patterns[p]
end

-- A pattern's fill, and its text: its colour (a muted group's dimmed), darkened.
local function pattern_colours(theme, s, g, p)
  local palette = p.default_color and s.groups[g].palette or p.palette
  local fill = widgets.colour(theme, palette)
  if s.groups[g].muted then fill = widgets.blend(fill, { 0, 0, 0 }, 0.35) end
  return fill, widgets.blend(fill, { 0, 0, 0 }, 0.14)
end

-- Section `i`'s scene colour, dimmed unless it's the focused one.
local function scene_colour(theme, s, i)
  local sec = s.sections[i]
  local sc = sec.scene and s.scenes[sec.scene]
  return widgets.colour(theme, sc and sc.palette or 16, i ~= s.section)
end

-- A pattern block from `a` to `b` (pixels) in a lane at `y`, `h` high: its colour,
-- a line where it repeats, and where the section ends.
local function block(screen, theme, s, g, p, sec, a, b, y, h, ppt, from)
  local fill = pattern_colours(theme, s, g, p)
  screen:rect(a, y, b - a, h, fill)
  local length = math.floor(p.bars * bar_ticks(s) + 0.5)
  if length > 0 then
    for t = sec.start + length, sec.start + sec.length - 1, length do
      local x = math.floor((t - from) * ppt)
      if x > a and x < b - 1 then screen:rect(x, y, 1, h, "song_section_edge") end
    end
  end
  screen:rect(b - 1, y, 1, h, "song_section_edge")
end

-- The playhead at `x`, from `y` for `h`, while playing.
local function playhead(screen, s, x, y, h, lo, hi)
  if s.playing and s.position and x >= lo and x < hi then screen:rect(x, y, 1, h, "text") end
end

-- The bar ruler from `x0` (`w` wide, at `y`): a tick and a number every `every` bars.
local function ruler(screen, theme, s, x0, y, w, px_per_tick, from, every)
  local bar = bar_ticks(s)
  local first = from // bar
  for b = first - first % every, math.huge, every do
    local x = x0 + math.floor((b * bar - from) * px_per_tick)
    if x >= x0 + w then break end
    if x >= x0 then
      screen:rect(x, y, 1, 14, "song_tick")
      screen:label(x + 3, y, 40, 13, tostring(b + 1), "song_ruler", "left", "center")
    end
  end
end

-- Empty bars: dark cells with a line at each bar.
local function empty_cells(screen, s, x, y, w, h, px_per_tick, from)
  screen:rect(x, y, w, h, "song_empty")
  local bar = bar_ticks(s)
  local b = from // bar + 1
  while true do
    local bx = x + math.floor((b * bar - from) * px_per_tick) - 1
    if bx >= x + w then break end
    if bx >= x then screen:rect(bx, y, 1, h, "song_bar_line") end
    b = b + 1
  end
end

-- The whole song on the left screen: each group's patterns in a thin lane.
local function overview(screen, theme, s)
  local bar = bar_ticks(s)
  local length = 0
  for _, sec in ipairs(s.sections) do length = math.max(length, sec.start + sec.length) end
  local span = math.max(length, 4 * bar)
  local ppt = 480 / span
  screen:rect(0, 59, 480, 146, "background")
  screen:rect(0, 60, 480, 14, "song_bg")
  ruler(screen, theme, s, 0, 60, 480, ppt, 0, 4)
  -- (A line of the sections' scene colours.)
  for i, sec in ipairs(s.sections) do
    local a, b = math.floor(sec.start * ppt), math.floor((sec.start + sec.length) * ppt)
    screen:rect(a, 75, b - a, 2, scene_colour(theme, s, i))
  end
  for g = 1, math.min(#s.groups, 15) do
    local y = 78 + (g - 1) * 8
    screen:rect(0, y - 1, 480, 1, "song_bg")
    empty_cells(screen, s, 0, y, 480, 7, ppt, 0)
    for _, sec in ipairs(s.sections) do
      local p = pattern_of(s, sec, g)
      if p then
        local x0 = math.floor(sec.start * ppt)
        local x1 = math.floor((sec.start + sec.length) * ppt)
        block(screen, theme, s, g, p, sec, x0, x1, y, 7, ppt, 0)
      end
    end
  end
  playhead(screen, s, math.floor((s.position or 0) * ppt), 60, 145, 0, 480)
end

local ARRANGER_X, ARRANGER_W = 507, 453
local ROW_Y, ROW_H, ROW_STEP = 75, 22, 24

-- Keep the focused section in view on the right screen.
local function follow(s)
  if s.section == shown then return end
  shown = s.section
  local sec = s.section and s.sections[s.section]
  if not sec then return end
  local ppt = zoom / QUARTER
  local left, right = sec.start, sec.start + sec.length
  if left < scroll or (right - scroll) * ppt > ARRANGER_W then scroll = left end
end

-- The sections close up on the right screen: their tabs (the focused one lit, a line
-- of its scene's colour under it), the ruler, and the focused bank of groups' patterns in rows.
local function arranger(screen, theme, s)
  local ppt = zoom / QUARTER
  local x0 = ARRANGER_X
  screen:rect(480, 35, 480, 237, "song_bg")
  -- The groups.
  local first = (s.group - 1) // 8 * 8
  screen:rect(480, 35, 26, 237, "background")
  for i = 1, 8 do
    local g = first + i
    local group = s.groups[g]
    if group then
      local y = 76 + (i - 1) * 24
      local letter = util.group_letter(g)
      letter = #letter == 1 and (letter .. "1") or letter
      if g == s.group then
        screen:rect(480, y, 26, 23, widgets.colour(theme, group.palette))
        screen:label(480, y, 26, 23, letter, "song_group_on")
      else
        screen:label(480, y, 26, 23, letter, { size = 10, color = widgets.colour(theme, group.palette, false, true) })
      end
    end
  end
  -- The tabs.
  local function span(sec)
    local a = x0 + math.floor((sec.start - scroll) * ppt)
    local b = x0 + math.floor((sec.start + sec.length - scroll) * ppt)
    return math.max(a, x0), math.min(b, x0 + ARRANGER_W)
  end
  for i, sec in ipairs(s.sections) do
    local a, b = span(sec)
    if b > a then
      local focused = i == s.section
      screen:rect(a, 35, b - a, 19, focused and "song_tab_focus" or "song_tab")
      screen:rect(b - 1, 35, 1, 19, "song_tab_edge")
      if focused then screen:rect(a, 54, b - a, 3, scene_colour(theme, s, i)) end
      screen:label(a + 8, 36, math.max(0, b - a - 10), 17, scene_name(s, sec), focused and "song_tab_on" or "song_tab", "left", "center")
    end
  end
  ruler(screen, theme, s, x0, 60, ARRANGER_W, ppt, scroll, math.max(1, 4 * math.ceil(40 / (bar_ticks(s) * ppt * 4))))
  -- The rows.
  for i = 1, 8 do
    local g = first + i
    if not s.groups[g] then break end
    local y = ROW_Y + (i - 1) * ROW_STEP
    empty_cells(screen, s, x0, y, ARRANGER_W, ROW_H, ppt, scroll)
    for _, sec in ipairs(s.sections) do
      local p = pattern_of(s, sec, g)
      local a, b = span(sec)
      if p and b > a then
        local _, ink = pattern_colours(theme, s, g, p)
        block(screen, theme, s, g, p, sec, a, b, y, ROW_H, ppt, scroll - x0 / ppt)
        screen:label(a + 5, y, math.max(0, b - a - 6), ROW_H, p.name, { size = 13, color = ink }, "left", "center")
      end
    end
  end
  playhead(screen, s, x0 + math.floor(((s.position or 0) - scroll) * ppt), 57, 215, x0, x0 + ARRANGER_W)
end

function song.draw(screen, theme)
  local s = deckard.state()
  follow(s)
  if deckard.held("SHIFT") then
    widgets.buttons(screen, theme, {
      [1] = { text = "IDEAS" },
      [2] = { text = "SONG", selected = true },
    })
  else
    local section = s.section ~= nil
    widgets.buttons(screen, theme, {
      [1] = { text = "SECTION", selected = true },
      [2] = { text = "CLIP" },
      [3] = { text = "CONVERT", disabled = true },
      [4] = { text = "DUPLICATE", disabled = not section },
      [5] = { text = "CREATE" },
      [6] = { text = "DELETE", disabled = not section },
      [7] = { arrows = true, text = "BANK " .. bank, left = can_bank(s, false), right = can_bank(s, true) },
    })
  end
  local info = widgets.focus_info(s)
  info.values[3] = { text = util.bar_beat(s.position or 0, s.numerator, s.denominator) }
  widgets.info_bar(screen, theme, info)
  overview(screen, theme, s)
  local sec = s.section and s.sections[s.section]
  local length = "-"
  if sec then
    if sec.auto then
      length = "Auto"
    else
      length = util.span(sec.length, s.numerator, s.denominator)
    end
  end
  widgets.params(screen, theme, 0, { { text = "Section", at = 1 } }, nil, {
    [1] = { name = "POSITION", value = s.section and ("#" .. s.section) or "-" },
    [2] = { name = "SCENE", value = scene_name(s, sec) },
    [4] = { name = "LENGTH", value = length },
  })
  arranger(screen, theme, s)
end

return song
