-- The Note Repeat page (NOTE REPEAT held, or pinned; Maschine's `RepeatPageMK3`): held
-- pads repeat on the rate's grid; in Keyboard mode, the Arp. The screens show the
-- pattern, as the Events page's.
--
--   display 1         NOTE RPT, or ARP (its pin)
--   display 2         LOCK: Note Repeat stays on off the page
--   display 3         HOLD: the notes keep repeating once let go
--   display 4         GATE RESET: the gate back to 100%
--   display 5-8       the presets' rates
--   knobs             RATE, UNIT (3, 4) and GATE (8); in Keyboard mode TYPE, RATE,
--                     UNIT, SEQUENCE, OCTAVES, DYNAMIC, GATE (2-8), and on page 2
--                     RETRIGGER, REPEAT, OFFSET, INVERSION, MIN. KEY, MAX. KEY (1-6)
--   4-D encoder       as the Events page's (lib/notes.lua)

local widgets = require("lib.widgets")
local pads = require("lib.pads")
local pin = require("lib.pin")
local editor = require("lib.pattern_editor")
local notes = require("lib.notes")
local events = require("pages.events")

local page = {}

-- The knob page (Keyboard mode's 1 or 2), and each knob's turn since its last step.
local knob_page = 1
local turned = {}

-- Each knob's setting and the names over the knobs ({ text, at }), by mode and page.
local LAYOUTS = {
  repeat_ = {
    knobs = { [3] = "rate", [4] = "rate_unit", [8] = "gate" },
    groups = { { text = "Rhythm", at = 3 }, { text = "Other", at = 8 } },
  },
  arp = {
    knobs = { [2] = "type", [3] = "rate", [4] = "rate_unit", [5] = "sequence", [6] = "octaves", [7] = "dynamic", [8] = "gate" },
    groups = { { text = "Main", at = 2 }, { text = "Rhythm", at = 3 }, { text = "Other", at = 6 } },
  },
  advanced = {
    knobs = { "retrigger", "repeat", "offset", "inversion", "min_key", "max_key" },
    groups = { { text = "Advanced", at = 1 }, { text = "Range", at = 5 } },
  },
}

local function layout()
  if not pads.keyboard() then return LAYOUTS.repeat_ end
  return knob_page == 2 and LAYOUTS.advanced or LAYOUTS.arp
end

function page.shown()
  knob_page, turned = 1, {}
end

function page.button(name, down)
  local r = deckard.state().note_repeat
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if n then
    -- (Display button 1 is the pin, global.lua's.)
    if n == 1 then return end
    if not down then return true end
    if n == 2 then
      deckard.note_repeat("locked", not r.locked)
    elseif n == 3 then
      deckard.note_repeat("hold", not r.hold)
    elseif n == 4 then
      deckard.note_repeat("gate", 1)
    elseif n >= 5 and r.presets[n - 4] then
      deckard.note_repeat("preset", n - 4)
    end
    return true
  end
  if name == "LEFT" or name == "RIGHT" then
    if down and pads.keyboard() then knob_page = name == "LEFT" and 1 or 2 end
    return true
  end
  return notes.wheel_button(name, down)
end

page.wheel = notes.wheel

function page.knob(n, amount)
  local key = layout().knobs[n]
  local set = key and deckard.state().note_repeat.settings[key]
  if not set then return true end
  -- (A step each `turn`; turning back starts again.)
  local t = turned[n] or 0
  if (t > 0 and amount < 0) or (t < 0 and amount > 0) then t = 0 end
  t = t + amount
  local steps = t > 0 and math.floor(t / set.turn + 1e-9) or -math.floor(-t / set.turn + 1e-9)
  turned[n] = t - steps * set.turn
  if steps == 0 then return true end
  local to = math.max(set.min, math.min(set.max, set.value + steps * set.step))
  -- (Rounded to the step, so 1% steps add up exactly.)
  to = math.floor((to - set.min) / set.step + 0.5) * set.step + set.min
  if to ~= set.value then deckard.note_repeat(key, to) end
  return true
end

function page.leds()
  local s = deckard.state()
  local r = s.note_repeat
  local function on(x) return x and "on" or "dim" end
  deckard.led("SCREEN_1", on(pin.on()))
  deckard.led("SCREEN_2", on(r.locked))
  deckard.led("SCREEN_3", on(r.hold))
  deckard.led("SCREEN_4", "dim")
  for i = 1, 4 do
    deckard.led("SCREEN_" .. (i + 4), r.presets[i] and on(r.preset == i) or "off")
  end
  local keyboard = pads.keyboard()
  deckard.led("LEFT", keyboard and knob_page == 2 and "dim" or "off")
  deckard.led("RIGHT", keyboard and knob_page == 1 and "dim" or "off")
  notes.wheel_leds(s)
end

function page.draw(screen, theme)
  local s = deckard.state()
  local r = s.note_repeat
  local keyboard = pads.keyboard()
  local b = {
    [1] = { text = keyboard and "ARP" or "NOTE RPT", pin = true, selected = pin.on() },
    [2] = { text = "LOCK", selected = r.locked },
    [3] = { text = "HOLD", selected = r.hold },
    [4] = { text = "GATE RESET" },
  }
  for i = 1, 4 do
    if r.presets[i] then b[i + 4] = { text = r.presets[i], selected = r.preset == i } end
  end
  widgets.buttons(screen, theme, b)
  widgets.info_bar(screen, theme, widgets.focus_info(s))

  -- The left screen: the whole pattern; the right, its lanes.
  editor.draw(screen, theme, s, 0, 60, 480, 145, false)
  events.draw_lanes(screen, theme, s, 205)

  -- The knobs along both screens.
  local l = layout()
  local params, groups = { {}, {} }, { {}, {} }
  for n = 1, 8 do
    local set = l.knobs[n] and r.settings[l.knobs[n]]
    if set then params[(n - 1) // 4 + 1][(n - 1) % 4 + 1] = { name = set.name, value = set.text } end
  end
  for _, g in ipairs(l.groups) do
    local side = (g.at - 1) // 4 + 1
    table.insert(groups[side], { text = g.text, at = (g.at - 1) % 4 + 1 })
  end
  local paging = keyboard and (knob_page .. "/2") or nil
  widgets.params(screen, theme, 0, groups[1], nil, params[1])
  widgets.params(screen, theme, 480, groups[2], paging, params[2])
end

return page
