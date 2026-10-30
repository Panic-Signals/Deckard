-- Set Macro (SHIFT + MACRO): touch a knob to make its parameter one of a channel's
-- macros (or stop it being one).
--
--   display 1         SET MACRO (the page stays until MACRO, or SHIFT + MACRO, again)
--   display 2-4       whose macro: MASTER, GROUP, SOUND (no lower than the level the
--                     Control area shows)
--   knob touched      its parameter set as a macro there, or taken off
--   knobs             turn as on the Control page
--   LEFT / RIGHT      the previous / next page, still setting macros (DECKARD's:
--                     Maschine's Set Macro has no pages)

local widgets = require("lib.widgets")
local control = require("pages.control")

local macro_set = {}

-- Whose macro (0 master, 1 group, 2 sound); the level shown when the page shows.
local target = nil

-- The level the Control area shows (0 master, 1 group, 2 sound).
local function level()
  return deckard.state().control.level - 1
end

-- (Never lower than the level shown: a sound's macro can't reach a group's knob.)
local function chosen()
  if target == nil or target > level() then target = level() end
  return target
end

function macro_set.button(name, down)
  local cap = tonumber(name:match("^CAP_(%d)$"))
  if cap then
    if down then deckard.set_macro(cap, chosen()) end
    return true
  end
  if name == "LEFT" or name == "RIGHT" then
    if down then
      if name == "LEFT" then deckard.previous_page() else deckard.next_page() end
    end
    return true
  end
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if not n then return end
  if down and n >= 2 and n <= 4 and n - 2 <= level() then target = n - 2 end
  return true
end

function macro_set.knob(n, amount)
  return control.knob(n, amount)
end

function macro_set.leds()
  local t = chosen()
  deckard.led("SCREEN_1", "on")
  for i = 2, 4 do
    deckard.led("SCREEN_" .. i, t == i - 2 and "on" or (i - 2 <= level() and "dim" or "off"))
  end
  for i = 5, 8 do deckard.led("SCREEN_" .. i, "off") end
  -- (LEFT / RIGHT lit as on the Control page: global.lua's.)
end

function macro_set.draw(screen, theme)
  local s = deckard.state()
  local t = chosen()
  local lv = level()
  widgets.buttons(screen, theme, {
    [1] = { text = "SET MACRO", pin = true, selected = true },
    [2] = { text = "MASTER", selected = t == 0 },
    [3] = { text = "GROUP", selected = t == 1, disabled = lv < 1 },
    [4] = { text = "SOUND", selected = t == 2, disabled = lv < 2 },
  })
  widgets.info_bar(screen, theme, widgets.focus_info(s))
  control.knob_row(screen, theme, s.control)
end

-- (Shown again: from the level the Control area shows.)
function macro_set.shown()
  target = nil
end

return macro_set
