-- Extended Lock (SHIFT + LOCK): the snapshots, a bank of 16 on the pads.
--
--   display 1         EXT LOCK (the page stays until LOCK again)
--   display 5 / 6     UPDATE (take the focused snapshot again) / DELETE it
--   display 7 / 8     the bank before / after (past the last, a new one; four at most)
--   pads              a snapshot's pad recalls it; a free one takes one there; with
--                     ERASE held, a snapshot's pad deletes it (they play nothing)
--   knobs 1-3         Morphing: MORPH, and with it MODE and TIME

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")

local lock = {}

local MODES = { "Travel", "Target" }
local TIMES = { "1/2 Bar", "1 Bar", "2 Bars", "4 Bars", "8 Bars", "16 Bars" }
local GRIDS = { "1/8", "1/4", "1/2", "Bar" }
local BANKS = 4

local function state()
  return deckard.state().lock
end

-- The slot (from 1) of pad `n` in the bank shown.
local function slot(l, n)
  return (l.bank - 1) * 16 + n
end

function lock.button(name, down)
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if not n then return end
  if not down then return true end
  local l = state()
  if (n == 5 or n == 6) and l.focus then
    deckard.lock(n == 5 and "update" or "delete")
  elseif n == 7 and l.bank > 1 then
    deckard.lock("bank", -1)
  elseif n == 8 and l.bank < BANKS then
    deckard.lock("bank", 1)
  end
  return true
end

function lock.pad(n)
  local l = state()
  local taken = l.slots[slot(l, n)]
  if deckard.held("ERASE") then
    if taken then deckard.lock("delete_slot", slot(l, n)) end
  else
    deckard.lock("pad", n)
  end
  return true
end

function lock.pads()
  return { play = false }
end

-- Knob `n`'s setting: its name, list, place, and what deckard.lock calls it.
local function setting(l, n)
  if n == 1 then return "MORPH", { "Off", "On" }, l.morph and 1 or 0, "morph" end
  if not l.morph then return end
  if n == 2 then return "MODE", MODES, l.mode, "morph_mode" end
  if n == 3 then
    if l.mode == 1 then return "TIME", GRIDS, l.grid, "morph_grid" end
    return "TIME", TIMES, l.time, "morph_time"
  end
end

function lock.knob(n, amount)
  local _, list, at, what = setting(state(), n)
  if not list then return true end
  local steps = knobs.gather(n, amount)
  local to = math.max(0, math.min(#list - 1, at + steps))
  if steps ~= 0 and to ~= at then deckard.lock(what, to) end
  return true
end

function lock.leds()
  local l = state()
  deckard.led("SCREEN_1", "on")
  for i = 2, 4 do deckard.led("SCREEN_" .. i, "off") end
  deckard.led("SCREEN_5", l.focus and "dim" or "off")
  deckard.led("SCREEN_6", l.focus and "dim" or "off")
  deckard.led("SCREEN_7", l.bank > 1 and "dim" or "off")
  deckard.led("SCREEN_8", l.bank < BANKS and "dim" or "off")
  deckard.led("LEFT", "off")
  deckard.led("RIGHT", "off")
  for n = 1, 16 do
    local at = slot(l, n)
    if l.slots[at] then
      deckard.led("PAD_" .. n, 16, l.focus == at)
    else
      deckard.led("PAD_" .. n, "off")
    end
  end
end

function lock.draw(screen, theme)
  local s = deckard.state()
  local l = s.lock
  widgets.buttons(screen, theme, {
    [1] = { text = "EXT LOCK", selected = true },
    [5] = { text = "UPDATE", disabled = not l.focus },
    [6] = { text = "DELETE", disabled = not l.focus },
    [7] = { arrows = true, text = "BANK " .. l.bank, left = l.bank > 1, right = l.bank < BANKS },
  })
  widgets.info_bar(screen, theme, widgets.focus_info(s))
  local params = {}
  for n = 1, 3 do
    local name, list, at = setting(l, n)
    if name then params[n] = { name = name, value = list[at + 1] } end
  end
  widgets.params(screen, theme, 0, { { text = "Morphing", at = 1 } }, nil, params)
  local pads = {}
  for n = 1, 16 do
    local at = slot(l, n)
    pads[n] = l.slots[at] and { name = tostring(at), palette = 16, on = l.focus == at } or { empty = true }
  end
  widgets.pad_grid(screen, theme, pads)
end

return lock
