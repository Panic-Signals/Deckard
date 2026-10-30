-- The Scene page (SCENE held): the scenes on the pads.
--
--   pads              focus a scene (while a scene plays, it plays next); the "+"
--                     after the last makes a new one
--   display 3-6       APPEND (to the arrangement), DUPLICATE, CREATE, DELETE
--   display 7 / 8     the previous / next sixteen scenes
--   knob 1            the scene's colour
--   jog wheel         the previous / next scene

local widgets = require("lib.widgets")
local pin = require("lib.pin")
local knobs = require("lib.knobs")

local scene = {}

local PADS = 16

local bank = 1

local EDITS = { [3] = "append", [4] = "duplicate", [5] = "create", [6] = "delete" }

local function focus(s, n)
  if s.scenes[n] then deckard.focus_scene(n) end
end

function scene.button(name, down)
  if not down then return end
  local s = deckard.state()
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if n and EDITS[n] then
    deckard.scene_edit(EDITS[n])
  elseif n == 7 then
    if bank > 1 then bank = bank - 1 end
  elseif n == 8 then
    bank = bank + 1
  elseif name == "WHEEL_LEFT" then
    focus(s, s.scene - 1)
  elseif name == "WHEEL_RIGHT" then
    focus(s, s.scene + 1)
  else
    return
  end
  return true
end

function scene.wheel(steps)
  local s = deckard.state()
  focus(s, s.scene + steps)
  return true
end

function scene.pad(n)
  local s = deckard.state()
  local at = (bank - 1) * PADS + n
  if s.scenes[at] then
    deckard.focus_scene(at)
  elseif at == #s.scenes + 1 then
    deckard.scene_edit("create")
  end
  return true
end

function scene.knob(n, amount)
  local steps = knobs.gather(n, amount)
  if n ~= 1 or steps == 0 then return true end
  local s = deckard.state()
  local sc = s.scenes[s.scene]
  if sc then
    local i = sc.default_color and 16 or sc.palette
    i = math.max(0, math.min(16, i + steps))
    deckard.set_color("scene", i < 16 and i or nil)
  end
  return true
end

function scene.leds()
  local s = deckard.state()
  for i = 1, 6 do deckard.led("SCREEN_" .. i, "dim") end
  if pin.on() then deckard.led("SCREEN_1", "on") end
  deckard.led("SCREEN_7", "off")
  deckard.led("SCREEN_8", "off")
  deckard.led("SCENE", "on")
  deckard.led("DUPLICATE", "dim")
  deckard.led("LEFT", "off")
  deckard.led("RIGHT", "off")
  deckard.led("WHEEL_RIGHT", 16, false)
  for i = 1, PADS do
    local at = (bank - 1) * PADS + i
    local sc = s.scenes[at]
    if sc then
      deckard.led("PAD_" .. i, sc.palette, at == s.scene)
    elseif at == #s.scenes + 1 then
      deckard.led("PAD_" .. i, 16, false)
    else
      deckard.led("PAD_" .. i, "off")
    end
  end
end

function scene.draw(screen, theme)
  local s = deckard.state()
  widgets.pin_tabs(screen, theme, pin.on(), { "SCENE", "UNIQUE" }, 1)
  widgets.buttons(screen, theme, {
    [3] = { text = "APPEND" },
    [4] = { text = "DUPLICATE" },
    [5] = { text = "CREATE" },
    [6] = { text = "DELETE" },
    [7] = { arrows = true, text = "BANK " .. bank, left = bank > 1, right = true },
  })
  widgets.info_bar(screen, theme, widgets.focus_info(s))
  local sc = s.scenes[s.scene]
  local colour = (not sc or sc.default_color) and "Default" or widgets.COLOUR_NAMES[sc.palette + 1]
  widgets.params(screen, theme, 0, { { text = "Color", at = 1 }, { text = "Perform", at = 2 } }, nil, {
    [1] = { name = "SCENE", value = colour },
    [2] = { name = "RETRIGGER", value = "0" },
  })
  local pads = {}
  for i = 1, PADS do
    local at = (bank - 1) * PADS + i
    local x = s.scenes[at]
    if x then
      pads[i] = { name = x.name, palette = x.palette, on = at == s.scene }
    elseif at == #s.scenes + 1 then
      pads[i] = { add = true }
    else
      pads[i] = { empty = true }
    end
  end
  widgets.pad_grid(screen, theme, pads)
end

return scene
