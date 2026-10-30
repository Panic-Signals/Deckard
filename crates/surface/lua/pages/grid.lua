-- The Grid page (SHIFT + FOLLOW, while FOLLOW is held): the song's grids on the pads.
--
--   display 1         GRID: pin the page
--   display 2-4       PERFORM (scene and section changes), ARRANGE (pattern and
--                     section lengths), STEP (the step grid)
--   pads              the values, up the columns from the bottom left; pad 16 the
--                     grid OFF (and back on)
--   display 5-8       a step down the pads' column, round to its top (PERFORM and
--                     ARRANGE: 7 and 8 turn the grid off and on; 7 is STEP's only)
--   knob 4            ARRANGE: QUICK lengths; STEP: the nudge grid

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local pin = require("lib.pin")

local grid = {}

local PERFORM, ARRANGE, STEP = 1, 2, 3
local mode = PERFORM

-- Each mode's values by their place on the pads (1 the bottom left, up the column).
local VALUES = {
  [PERFORM] = { "1 Bar", "1/2", "1/4", "1/8", "1/16", "Section" },
  [ARRANGE] = { "1 Bar", "1/2", "1/4", "1/8", "1/16" },
  [STEP] = { "1 Bar", "1/2", "1/4", "1/8", "1/16", "1/32", "1/64", "1/128",
    "1 Bar T", "1/2T", "1/4T", "1/8T", "1/16T", "1/32T", "1/64T" },
}
local NUDGES = { "Step", "Step/2", "Step/4", "Step/8", "Step/16" }

-- The place (1-16) on the pads of pad `n` (pad 1 the bottom left): the columns read
-- up from the bottom. (The same both ways.)
local function place(n)
  return (n - 1) % 4 * 4 + (n - 1) // 4 + 1
end
local pad_at = place

-- (PERFORM's value 0 is its OFF; the others have a switch.)
local function snapping(g)
  if mode == PERFORM then return g.perform ~= 0 end
  if mode == ARRANGE then return g.arrange_snap end
  return g.step_snap
end

-- The value chosen as a place on the pads (none when off).
local function chosen(g)
  if not snapping(g) then return nil end
  if mode == PERFORM then return g.perform end
  if mode == ARRANGE then return g.arrange + 1 end
  return g.step + 1
end

-- PERFORM's value before OFF, for turning it back on.
local last_perform = 5

local function toggle(g)
  if mode == PERFORM then
    if g.perform ~= 0 then
      last_perform = g.perform
      deckard.grid("perform", 0)
    else
      deckard.grid("perform", last_perform)
    end
  elseif mode == ARRANGE then
    deckard.grid("arrange_snap", not g.arrange_snap)
  else
    deckard.grid("step_snap", not g.step_snap)
  end
end

local function pick(g, p)
  if p == 16 then return toggle(g) end
  if p > #VALUES[mode] then return end
  if mode == PERFORM then
    deckard.grid("perform", p)
  else
    deckard.grid(mode == ARRANGE and "arrange" or "step", p - 1)
    if not snapping(g) then toggle(g) end
  end
end

-- Display button 5-8: a step down column `col`, round to its top after the bottom
-- (from elsewhere, its top); in PERFORM and ARRANGE the second column has only its
-- first two or one, and the third and fourth turn the grid off and on.
local function cycle(g, col)
  if mode ~= STEP and col > 2 then return toggle(g) end
  local at = chosen(g) or 16
  local bottom, top = (col - 1) * 4 + 1, col * 4
  local p = (at > bottom and at <= top) and at - 1 or top
  if mode == PERFORM and col == 2 then p = (at == 6) and 5 or 6 end
  if mode == ARRANGE and col == 2 then p = 5 end
  pick(g, p)
end

function grid.button(name, down)
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  -- (Display button 1 is the pin's: global.lua.)
  if not n or n == 1 then return end
  if down then
    if n >= 2 and n <= 4 then
      mode = n - 1
    elseif n >= 5 and (n ~= 7 or mode == STEP) then
      cycle(deckard.state().grid, n - 4)
    end
  end
  return true
end

function grid.pad(n)
  pick(deckard.state().grid, place(n))
  return true
end

function grid.knob(n, amount)
  if n ~= 4 then return true end
  local g = deckard.state().grid
  if mode == ARRANGE and g.arrange_snap then
    local steps = knobs.gather(n, amount)
    if steps ~= 0 then deckard.grid("quick", steps > 0) end
  elseif mode == STEP then
    local steps = knobs.gather(n, amount)
    local to = math.max(0, math.min(#NUDGES - 1, g.nudge + steps))
    if to ~= g.nudge then deckard.grid("nudge", to) end
  end
  return true
end

function grid.leds()
  local g = deckard.state().grid
  deckard.led("SCREEN_1", pin.on() and "on" or "dim")
  for i = 2, 4 do deckard.led("SCREEN_" .. i, mode == i - 1 and "on" or "dim") end
  for i = 5, 8 do deckard.led("SCREEN_" .. i, (i ~= 7 or mode == STEP) and "dim" or "off") end
  deckard.led("FOLLOW", "on")
  deckard.led("LEFT", "off")
  deckard.led("RIGHT", "off")
  local at = chosen(g)
  for p = 1, 16 do
    local name = "PAD_" .. pad_at(p)
    if p == 16 then
      deckard.led(name, 16, not snapping(g))
    elseif p > #VALUES[mode] then
      deckard.led(name, "off")
    else
      deckard.led(name, 16, p == at)
    end
  end
end

function grid.draw(screen, theme)
  local s = deckard.state()
  local g = s.grid
  widgets.buttons(screen, theme, {
    [1] = { text = "GRID", pin = true, selected = pin.on() },
    [5] = { text = "v" },
    [6] = { text = "v" },
    -- (Maschine draws 7 the same when only its light says it does nothing.)
    [7] = { text = "v" },
    [8] = { text = "v" },
  })
  -- (PERFORM, ARRANGE and STEP as one row of tabs, the middle one wider.)
  screen:frame(123, 4, 353, 24, "button", theme.corner)
  for i, tab in ipairs({ { 123, 117, "PERFORM" }, { 240, 119, "ARRANGE" }, { 359, 117, "STEP" } }) do
    local x, w, text = tab[1], tab[2], tab[3]
    local on = mode == i
    if on then
      screen:rect(x, 4, w, 24, "button_selected", theme.corner)
      widgets.chase(screen, theme, x, 4, w, 24)
    end
    screen:label(x, 5, w, 23, text, theme.styles[on and "head_selected" or "head"])
  end
  widgets.info_bar(screen, theme, widgets.focus_info(s))
  local params = {}
  local groups = nil
  if mode == ARRANGE then
    groups = { { text = "Pattern", at = 4 } }
    params[4] = { name = "QUICK", value = g.arrange_snap and (g.quick and "On" or "Off") or "-" }
  elseif mode == STEP then
    params[4] = { name = "NUDGE", value = NUDGES[g.nudge + 1] }
  end
  widgets.params(screen, theme, 0, groups, nil, params)
  local at = chosen(g)
  local pads = {}
  for p = 1, 16 do
    local i = pad_at(p)
    if p == 16 then
      pads[i] = { name = "OFF", palette = 16, on = not snapping(g) }
    elseif VALUES[mode][p] then
      pads[i] = { name = VALUES[mode][p], palette = 16, on = p == at }
    else
      pads[i] = { empty = true }
    end
  end
  widgets.pad_grid(screen, theme, pads)
end

return grid
