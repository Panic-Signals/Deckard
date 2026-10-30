-- The Mixer (MIX): a strip per sound of the focused group, under a row of the groups'
-- names; or a strip per group.
--
--   display 1-8       focus the strip's sound (or group)
--   knobs 1-8         the strip's level (SHIFT: finely)
--   LEFT / RIGHT      the previous / next eight strips
--   jog wheel up      the groups' strips; down: the sounds' again
--   jog wheel left / right
--                     focus the previous / next sound (or group)

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")

local mixer = {}

-- Strips per bank, and how far a knob's full turn moves a level (dB); SHIFT: a tenth.
local STRIPS = 8
local DB_PER_TURN = 70
local FINE = 0.1

-- The meters' range (dB): under it is silence.
local LOW_DB, HIGH_DB = -60, 10

-- Showing the groups' strips (else the focused group's sounds'), and the bank of eight.
local groups_shown = false
local bank = 1

-- The channels shown: the groups, or the focused group's sounds; and which is focused.
local function channels(s)
  if groups_shown then return s.groups, s.group end
  local group = s.groups[s.group]
  return group and group.sounds or {}, s.sound
end

-- Show the bank with the focused strip in it.
local function follow(focused)
  bank = (focused - 1) // STRIPS + 1
end

local function focus(s, n)
  if groups_shown then deckard.focus_group(n) else deckard.focus_sound(n) end
  follow(n)
end

-- Strip `at`'s level set to `db`.
local function set_level(s, at, db)
  if groups_shown then deckard.set_level(at, nil, db) else deckard.set_level(s.group, at, db) end
end

function mixer.button(name, down)
  local tapped = knobs.double_tap(name, down)
  if not down then return end
  local s = deckard.state()
  local list, focused = channels(s)
  -- (A knob tapped twice: its strip back to 0 dB.)
  if tapped then
    local at = (bank - 1) * STRIPS + tapped
    if list[at] then set_level(s, at, 0) end
    return true
  end
  if name == "LEFT" then
    if bank > 1 then bank = bank - 1 end
  elseif name == "RIGHT" then
    if bank * STRIPS < #list then bank = bank + 1 end
  elseif name == "WHEEL_UP" then
    if not groups_shown then
      groups_shown = true
      follow(s.group)
    end
  elseif name == "WHEEL_DOWN" then
    if groups_shown then
      groups_shown = false
      follow(s.sound)
    end
  elseif name == "WHEEL_LEFT" then
    if focused > 1 then focus(s, focused - 1) end
  elseif name == "WHEEL_RIGHT" then
    if focused < #list then focus(s, focused + 1) end
  else
    local n = tonumber(name:match("^SCREEN_(%d)$"))
    if not n then return end
    local at = (bank - 1) * STRIPS + n
    if list[at] then focus(s, at) end
  end
  return true
end

-- Turning the jog wheel does nothing here.
function mixer.wheel()
  return true
end

function mixer.knob(n, amount)
  local s = deckard.state()
  local list = channels(s)
  local at = (bank - 1) * STRIPS + n
  local ch = list[at]
  if not ch then return true end
  local db = math.max(ch.level, LOW_DB)
  db = db + amount * DB_PER_TURN * (deckard.held("SHIFT") and FINE or 1)
  if db < LOW_DB then db = -math.huge end
  set_level(s, at, db)
  return true
end

function mixer.leds()
  local s = deckard.state()
  local list, focused = channels(s)
  for i = 1, STRIPS do
    local at = (bank - 1) * STRIPS + i
    if list[at] then deckard.led("SCREEN_" .. i, at == focused and "on" or "dim") end
  end
  deckard.led("MIX", "on")
  deckard.led("SOLO", deckard.held("SOLO") and "on" or "dim")
  deckard.led("MUTE", deckard.held("MUTE") and "on" or "dim")
  deckard.led("LEFT", bank > 1 and "dim" or "off")
  deckard.led("RIGHT", bank * STRIPS < #list and "dim" or "off")
  deckard.led(groups_shown and "WHEEL_DOWN" or "WHEEL_UP", 16, false)
  if focused > 1 then deckard.led("WHEEL_LEFT", 16, false) end
  if focused < #list then deckard.led("WHEEL_RIGHT", 16, false) end
end

-- Where the meters draw `db`: how far above their foot (the scale isn't linear: the
-- top is spread wide).
local SCALE = { { -60, 0 }, { -30, 22 }, { -20, 39 }, { -10, 64 }, { 0, 91 }, { 10, 119 } }
local function scale_height(db)
  if db <= SCALE[1][1] then return 0 end
  for i = 2, #SCALE do
    local d0, h0, d1, h1 = SCALE[i - 1][1], SCALE[i - 1][2], SCALE[i][1], SCALE[i][2]
    if db <= d1 then return h0 + (db - d0) / (d1 - d0) * (h1 - h0) end
  end
  return SCALE[#SCALE][2]
end

-- A strip's parts, up from its foot (`bottom`): the meters' foot, the pan bar, the
-- meters and the fader across.
local FOOT = 37
local HEADER_FOOT = 198
local PAN = 189
local METERS = { { 28, 54 }, { 57, 83 } }
local FADER_X = 88

local function level_y(bottom, db)
  return bottom - FOOT - math.floor(scale_height(db) + 0.5)
end

-- A dB value as the strips write it: "-5.8", "-inf".
local function db_text(db)
  if db <= LOW_DB then return "-inf" end
  if math.abs(db) < 0.05 then return "0.0" end
  return string.format("%.1f", db)
end

-- A long name in up to two lines of about `width` characters, broken at a space or
-- after a hyphen where it can.
local function two_lines(name, width)
  if #name <= width then return name, "" end
  local cut = width
  for i = width, 1, -1 do
    if name:sub(i, i) == "-" or name:sub(i + 1, i + 1) == " " then
      cut = i
      break
    end
  end
  local first = name:sub(1, cut):gsub("%s+$", "")
  local rest = name:sub(cut + 1):gsub("^%s+", "")
  return first, rest:sub(1, width)
end

-- Text from `top` in a style, at `x` (or centred across 0..`w`).
local function text(screen, theme, x, top, w, s, st, colour, centred)
  local style = {}
  for k, v in pairs(theme.styles.default) do style[k] = v end
  for k, v in pairs(theme.styles[st] or {}) do style[k] = v end
  if colour then style.color = colour end
  screen:label(x, top, w, math.floor(style.size) + 4, s, style, centred and "center" or "left")
end

-- One strip at `x`, from `top` to `bottom` (a row of names when `row`).
local function strip(screen, theme, x, top, bottom, ch, short, focused, row, separator)
  local muted = ch.muted
  local tint = muted and "muted" or widgets.colour(theme, ch.palette)
  local tint_text = muted and "muted" or widgets.colour(theme, ch.palette, false, true)
  local header_bottom = row and bottom or bottom - HEADER_FOOT
  if focused and not row then screen:rect(x, header_bottom, 120, bottom - header_bottom, "mixer_focused") end
  if focused then screen:rect(x, top, 120, header_bottom - top, tint) end
  local short_c, name_c = tint_text, "mixer_name"
  if focused then
    short_c, name_c = "text", "text"
  elseif muted then
    short_c, name_c = "muted", "muted"
  end
  text(screen, theme, x + 6, top + 7, 24, short, "mixer_name", short_c)
  local first, second
  if row then first, second = ch.name:sub(1, 10), "" else first, second = two_lines(ch.name, 10) end
  text(screen, theme, x + 28, top + 7, 91, first, "mixer_name", name_c)
  if second ~= "" then text(screen, theme, x + 28, top + 22, 91, second, "mixer_name", name_c) end
  if row then
    if separator then screen:rect(x, bottom - 2, 120, 2, widgets.colour(theme, separator)) end
    return
  end

  -- Pan: the track, the colour from the middle to its side, the middle's mark, and the
  -- headroom under it.
  local pan_y = bottom - PAN
  screen:rect(x + 15, pan_y, 90, 4, "mixer_track")
  local px = 60 + math.floor(math.max(-1, math.min(1, ch.pan)) * 45 + 0.5)
  if px ~= 60 then screen:rect(x + math.min(px, 60), pan_y, math.abs(px - 60), 4, tint) end
  for i = 0, 3 do screen:rect(x + 60 - i, pan_y + 8 + i, 2 * i + 1, 1, "mixer_headroom") end
  text(screen, theme, x, bottom - PAN + 13, 120, db_text(ch.headroom), "mixer_headroom", ch.headroom > 0 and not muted and "text" or nil, true)

  -- The scale, the 0 dB line, the meters.
  for _, mark in ipairs({ { 10, "10 -" }, { 0, "0 -" }, { -10, "10 -" }, { -20, "20 -" }, { -30, "30 -" }, { -math.huge, "∞ -" } }) do
    text(screen, theme, x + 3, level_y(bottom, mark[1]) - 6, 23, mark[2], "mixer_scale")
  end
  local zero = level_y(bottom, 0)
  screen:rect(x + METERS[1][1] + 4, zero, FADER_X - METERS[1][1] - 4, 1, "mixer_zero")
  for k = 1, 2 do
    local db = ch.meter[k]
    if db > LOW_DB then
      local y = level_y(bottom, math.min(db, HIGH_DB))
      screen:rect(x + METERS[k][1], y, METERS[k][2] - METERS[k][1], bottom - FOOT - y, tint)
    end
  end

  -- The fader: a line up to the level, with an arrow at its top.
  local fader = muted and "muted" or "text"
  local at = level_y(bottom, ch.level)
  screen:rect(x + FADER_X, at, 1, bottom - FOOT - at, fader)
  for i = -2, 2 do
    local a = math.abs(i)
    screen:rect(x + FADER_X - 3 + a, at + i, 3 - a, 1, fader)
  end

  -- The plug-ins, from the bottom up: an instrument's bars, an effect's FX.
  local slot_at = FOOT
  for _, p in ipairs(ch.plugins or {}) do
    local c = p.active and tint or "muted"
    local y0 = bottom - slot_at - 9
    if p.instrument then
      for b = 0, 2 do screen:rect(x + 103 + b * 3, y0, 1, 9, c) end
    else
      screen:frame(x + 102, y0, 11, 9, c)
      text(screen, theme, x + 102, y0 + 1, 11, "FX", "mixer_fx", c, true)
    end
    slot_at = slot_at + 11
  end

  -- The level.
  local value_c = "text"
  if muted then value_c = focused and "muted" or "mixer_value_muted" end
  local db = ch.level
  text(screen, theme, x, bottom - 31, 120, db_text(db) .. " dB", "mixer_value", value_c, true)
end

-- A group's short name: its letter and bank ("A1").
local function group_short(g)
  return string.char(string.byte("A") + (g - 1) % 8) .. ((g - 1) // 8 + 1)
end

function mixer.draw(screen, theme)
  local s = deckard.state()
  local bottom = 271
  if groups_shown then
    for i = 1, STRIPS do
      local g = (bank - 1) * STRIPS + i
      local group = s.groups[g]
      local x = (i - 1) * 120
      if group then
        strip(screen, theme, x, 1, bottom, group, group_short(g), g == s.group)
      elseif g == #s.groups + 1 then
        text(screen, theme, x, 5, 120, "+", "mixer_add", nil, true)
        break
      end
    end
    return
  end
  -- The groups' names in a row, the focused group's colour under them.
  local group_bank = (s.group - 1) // STRIPS
  local focused = s.groups[s.group]
  for i = 1, STRIPS do
    local g = group_bank * STRIPS + i
    local group = s.groups[g]
    if group then
      strip(screen, theme, (i - 1) * 120, 1, 29, group, group_short(g), g == s.group, true, focused and focused.palette)
    end
  end
  local sounds = focused and focused.sounds or {}
  for i = 1, STRIPS do
    local n = (bank - 1) * STRIPS + i
    local snd = sounds[n]
    if snd then strip(screen, theme, (i - 1) * 120, 33, bottom, snd, tostring(n), n == s.sound) end
  end
end

return mixer
