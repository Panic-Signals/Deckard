-- The Step Mod page (a step's pad held a moment in Step mode): the Control page, its
-- knobs setting the modulation of the steps held (Maschine's `StepPageModMK3`).
--
--   display 1-3       MASTER, GROUP, SOUND, as the Control page's
--   display 5 / 6     the previous / next slot or page group, as the Control page's
--   CHANNEL / PLUG-IN the channel's properties or its plug-ins, staying on this page
--   LEFT / RIGHT      the previous / next page
--   knobs             the modulation of each step held (only what can be modulated is
--                     shown); a step's modulation holds for that step alone
--   pads              more steps held, or let go: the last one let go, back to Step mode
--                     (no step taken or removed)

local control = require("pages.control")
local step = require("pages.step")

local page = {}

function page.button(name, down)
  if name == "CHANNEL" or name == "PLUGIN" then
    if down then deckard.show_plugins(name == "PLUGIN") end
    return true
  end
  -- (Not the Control page's plug-in browser, nor its SHIFT edits.)
  if name == "WHEEL" or (deckard.held("SHIFT") and name:match("^SCREEN_%d$")) then return true end
  return control.button(name, down)
end

function page.knob(n, amount)
  local c = deckard.state().control
  local knob = c.knobs[n]
  if knob and knob.modulatable then step.mod_knob(n, amount) end
  return true
end

page.pad = step.pad
page.pad_off = step.pad_off

function page.leds()
  local s = deckard.state()
  control.leds()
  deckard.led("SCREEN_4", "off")
  deckard.led("SCREEN_7", "off")
  deckard.led("SCREEN_8", "off")
  step.pad_leds(s)
end

-- A knob as this page shows it: only what can be modulated, with the step's
-- modulation where it has one.
local function knob_of(knob)
  if not knob.modulatable then return nil end
  if not knob.step_mod then return knob end
  return { name = knob.name, section = knob.section, value = knob.value, text = knob.step_mod }
end

function page.draw(screen, theme)
  control.draw(screen, theme, { knob_of = knob_of, no_browse = true })
end

return page
