-- The Pattern page (PATTERN held): the focused group's patterns on the pads, the
-- focused one over the left screen.
--
--   pads              focus a pattern (it plays in the focused scene); the pad after
--                     the last makes a new one
--   display 3-6       DOUBLE, DUPLICATE, CREATE, DELETE
--   display 7 / 8     the previous / next sixteen patterns
--   LEFT / RIGHT      the knobs' pages: Pattern (POSITION, START, LENGTH), Color
--   jog wheel         the previous / next pattern

local widgets = require("lib.widgets")
local pin = require("lib.pin")
local editor = require("lib.pattern_editor")
local knobs = require("lib.knobs")
local util = require("lib.util")

local pattern = {}

local PADS = 16
local PAGES = 2

local bank = 1
local page = 1

local EDITS = { [3] = "double", [4] = "duplicate", [5] = "create", [6] = "delete" }

local function patterns(s)
  local group = s.groups[s.group]
  return group and group.patterns or {}
end

local function pick(s, n)
  if patterns(s)[n] then deckard.pick_pattern(n) end
end

function pattern.button(name, down)
  if not down then return end
  local s = deckard.state()
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if n and EDITS[n] then
    deckard.pattern_edit(EDITS[n])
  elseif n == 7 then
    if bank > 1 then bank = bank - 1 end
  elseif n == 8 then
    bank = bank + 1
  elseif name == "LEFT" then
    if page > 1 then page = page - 1 end
  elseif name == "RIGHT" then
    if page < PAGES then page = page + 1 end
  elseif name == "WHEEL_LEFT" or name == "WHEEL_UP" then
    pick(s, s.pattern - 1)
  elseif name == "WHEEL_RIGHT" or name == "WHEEL_DOWN" then
    pick(s, s.pattern + 1)
  else
    return
  end
  return true
end

function pattern.wheel(steps)
  local s = deckard.state()
  pick(s, s.pattern + steps)
  return true
end

function pattern.pad(n)
  local s = deckard.state()
  local at = (bank - 1) * PADS + n
  local list = patterns(s)
  if list[at] then
    deckard.pick_pattern(at)
  elseif at == #list + 1 then
    deckard.pattern_edit("create")
  end
  return true
end

-- A tick on the arrange grid (a bar), or with SHIFT the pattern editor's (a
-- sixteenth), `steps` on: snapped to them.
local function snap(s, t, steps, fine)
  local bar = 960 * 4 // math.max(1, s.denominator) * math.max(1, s.numerator)
  local step = fine and 240 or bar
  if steps > 0 then return (t // step + steps) * step end
  return -((-t) // step) * step + steps * step
end

-- The colours a COLOR knob steps through: the project's, then the default.
local function step_colour(colour, default, steps)
  local i = default and 16 or colour
  i = math.max(0, math.min(16, i + steps))
  return i < 16 and i or nil
end

function pattern.knob(n, amount)
  local s = deckard.state()
  -- (POSITION, START and LENGTH step at each turn; COLOR gathers.)
  local steps = page == 1 and knobs.each(amount) or knobs.gather(n, amount)
  if steps == 0 then return true end
  local fine = deckard.held("SHIFT")
  local start, length = s.pattern_start, s.pattern_length
  if page == 1 then
    if n == 1 then
      deckard.pattern_span(math.max(0, snap(s, start, steps, fine)), length)
    elseif n == 3 then
      local to = math.max(0, snap(s, start, steps, fine))
      if to < start + length then deckard.pattern_span(to, start + length - to) end
    elseif n == 4 then
      local step = fine and 240 or snap(s, 0, 1, false)
      deckard.pattern_length(math.max(step, snap(s, length, steps, fine)))
    end
  elseif n == 1 then
    local p = patterns(s)[s.pattern]
    if p then deckard.set_color("pattern", step_colour(p.palette, p.default_color, steps)) end
  end
  return true
end

function pattern.leds()
  local s = deckard.state()
  for i = 1, 6 do deckard.led("SCREEN_" .. i, "dim") end
  if pin.on() then deckard.led("SCREEN_1", "on") end
  deckard.led("SCREEN_7", "off")
  deckard.led("SCREEN_8", "off")
  deckard.led("PATTERN", "on")
  deckard.led("DUPLICATE", "dim")
  deckard.led("LEFT", page > 1 and "dim" or "off")
  deckard.led("RIGHT", page < PAGES and "dim" or "off")
  for _, arrow in ipairs({ "UP", "LEFT", "RIGHT", "DOWN" }) do deckard.led("WHEEL_" .. arrow, 16, false) end
  local list = patterns(s)
  for i = 1, PADS do
    local p = list[(bank - 1) * PADS + i]
    if p then
      deckard.led("PAD_" .. i, p.palette, (bank - 1) * PADS + i == s.pattern)
    else
      deckard.led("PAD_" .. i, "off")
    end
  end
end

function pattern.draw(screen, theme)
  local s = deckard.state()
  widgets.pin_tabs(screen, theme, pin.on(), { "PATTERN", "CLIP" }, 1)
  widgets.buttons(screen, theme, {
    [3] = { text = "DOUBLE" },
    [4] = { text = "DUPLICATE" },
    [5] = { text = "CREATE" },
    [6] = { text = "DELETE" },
    [7] = { arrows = true, text = "BANK " .. bank, left = bank > 1, right = true },
  })
  widgets.info_bar(screen, theme, widgets.focus_info(s))
  editor.draw(screen, theme, s, 0, 60, 480, 145, false)

  local list = patterns(s)
  if page == 1 then
    local length = s.pattern_auto and "Auto" or util.span(s.pattern_length, s.numerator, s.denominator)
    local start = util.bar_beat(s.pattern_start, s.numerator, s.denominator)
    widgets.params(screen, theme, 0, { { text = "Pattern", at = 1 } }, "1/" .. PAGES, {
      [1] = { name = "POSITION", value = start },
      [3] = { name = "START", value = start },
      [4] = { name = "LENGTH", value = length },
    })
  else
    local p = list[s.pattern]
    local colour = (not p or p.default_color) and "Default" or widgets.COLOUR_NAMES[p.palette + 1]
    widgets.params(screen, theme, 0, { { text = "Color", at = 1 } }, page .. "/" .. PAGES, {
      [1] = { name = "PATTERN", value = colour },
    })
  end

  local pads = {}
  for i = 1, PADS do
    local at = (bank - 1) * PADS + i
    local p = list[at]
    if p then
      pads[i] = { name = p.name, palette = p.palette, on = at == s.pattern }
    else
      pads[i] = { empty = true }
    end
  end
  widgets.pad_grid(screen, theme, pads)
end

return pattern
