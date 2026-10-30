-- The mod matrix (SHIFT + SOLO; again, or another page's button, leaves): eight slots,
-- one a column, four on each screen. The slots are every route in the project, then an
-- empty one for a new route (`state.mod_slots`). A CC route's slot (`midi`: its Group
-- MIDI) names rows 3-5 Port, Channel and CC.
--
--   display 1-8       its column's highlight down a row (Source, Group, Sound, Module,
--                     Destination, Depth), back to the top after Depth
--   knobs 1-8         its column's highlighted row; with ERASE held, the slot's route
--                     removed
--   LEFT / RIGHT      the eight slots before / after
--
-- When the page goes, DECKARD drops a slot being made (no Destination yet).

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")

local mod_matrix = {}

local ROWS = { "SOURCE", "GROUP", "SOUND", "MODULE", "DESTINATION", "DEPTH" }
local MIDI_ROWS = { "SOURCE", "GROUP", "PORT", "CHANNEL", "CC", "DEPTH" }

-- A slot's row names.
local function names(rows)
  return rows and rows.midi and MIDI_ROWS or ROWS
end

-- How far a knob turns for a step: a tenth for the lists, a fiftieth for Depth.
local TURNS = { 0.1, 0.1, 0.1, 0.1, 0.1, 0.02 }
-- A CC route's: CC (0-119) a fiftieth too.
local MIDI_TURNS = { 0.1, 0.1, 0.1, 0.1, 0.02, 0.02 }

-- Each column's highlighted row, and the eight slots shown (from 0).
local row = { 1, 1, 1, 1, 1, 1, 1, 1 }
local bank = 0

-- The columns ERASE has removed a route from since it went down (once a press: the
-- slots after move up into its place).
local erased = {}

-- The last bank with a slot in it, and the bank shown kept to it.
local function banks(s)
  return math.max(0, (#s.mod_slots - 1) // 8)
end

local function shown(s)
  bank = math.min(bank, banks(s))
  return bank
end

function mod_matrix.button(name, down)
  if name == "ERASE" then
    erased = {}
    return
  end
  local s = deckard.state()
  if name == "LEFT" or name == "RIGHT" then
    if down then
      local b = shown(s) + (name == "LEFT" and -1 or 1)
      if b >= 0 and b <= banks(s) then bank = b end
    end
    return true
  end
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if not n then return end
  if down then row[n] = row[n] % #ROWS + 1 end
  return true
end

function mod_matrix.knob(n, amount)
  local s = deckard.state()
  local slot = shown(s) * 8 + n
  if not s.mod_slots[slot] then return true end
  if deckard.held("ERASE") then
    if not erased[n] then
      erased[n] = true
      deckard.mod_slot(slot, 1, 0)
    end
    return true
  end
  local turns = s.mod_slots[slot].midi and MIDI_TURNS or TURNS
  local steps = knobs.gather(n, amount, turns[row[n]])
  if steps ~= 0 then deckard.mod_slot(slot, row[n], steps) end
  return true
end

function mod_matrix.leds()
  local s = deckard.state()
  local b = shown(s)
  for n = 1, 8 do
    deckard.led("SCREEN_" .. n, s.mod_slots[b * 8 + n] and "dim" or "off")
  end
  deckard.led("LEFT", b > 0 and "dim" or "off")
  deckard.led("RIGHT", b < banks(s) and "dim" or "off")
  deckard.led("SOLO", "on")
end

local HEAD = { size = 12, color = "weak" }
local NAME = { size = 9, color = "weak" }
local NAME_ON = { size = 9, color = "on_colour" }
local VALUE = { size = 13, color = "text" }
local VALUE_ON = { size = 13, color = "on_colour" }
local VALUE_UNSET = { size = 13, color = "weak" }

local TOP, ROW_H = 72, 33

function mod_matrix.draw(screen, theme)
  local s = deckard.state()
  local b = shown(s)
  local specs = {}
  for n = 1, 8 do
    local rows = s.mod_slots[b * 8 + n]
    if rows then specs[n] = { text = names(rows)[row[n]] } end
  end
  widgets.buttons(screen, theme, specs)

  for x = 0, 480, 480 do
    screen:rect(x, 32, 480, 20, "bar")
  end
  screen:label(10, 32, 300, 20, "MOD MATRIX", HEAD, "left", "center")
  screen:label(170, 32, 300, 20, string.format("SLOTS %d–%d", b * 8 + 1, b * 8 + 8), HEAD, "right", "center")
  if banks(s) > 0 then
    screen:label(650, 32, 300, 20, "LEFT / RIGHT: MORE SLOTS", HEAD, "right", "center")
  end

  for n = 1, 8 do
    local slot = b * 8 + n
    local rows = s.mod_slots[slot]
    local x = (n - 1) // 4 * 480 + (n - 1) % 4 * 120
    if n % 4 ~= 1 then screen:rect(x, 54, 1, 218, "bar") end
    if rows then
      screen:label(x + 4, 54, 112, 16, "SLOT " .. slot, HEAD, "center", "center")
      for r = 1, #ROWS do
        local y = TOP + (r - 1) * ROW_H
        local on = r == row[n]
        if on then screen:rect(x + 3, y, 114, ROW_H - 2, "accent", theme.corner) end
        screen:label(x + 7, y + 2, 106, 11, names(rows)[r], on and NAME_ON or NAME, "left", "top")
        local text = rows[r] or "—"
        local st = on and VALUE_ON or (text == "—" and VALUE_UNSET or VALUE)
        screen:label(x + 7, y + 13, 106, 17, text, st, "left", "center")
      end
    end
  end
end

return mod_matrix
