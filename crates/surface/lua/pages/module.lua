-- The plug-in browser (PLUG-IN, the jog wheel pressed): what to load into the focused
-- slot, or into the "+" after the last. A list of Maschine's own instruments and
-- effects, DECKARD's, and the installed plug-ins, by type, format and vendor.
--
--   display 1-3       MASTER, GROUP, SOUND: the chain shown
--   display 5 / 6     PREVIOUS / NEXT: the one before / after, loaded at once
--   display 7         CANCEL: back to the Control page
--   display 8         LOAD: load the chosen one, and back
--   knobs 1-3         TYPE (instrument or effect: in a sound's first slot only),
--                     FORMAT (Internal, Audio Units, VST3, VST), VENDOR
--   knobs 5-8, wheel  the list; the wheel pressed loads, as LOAD
--   jog wheel's left / right, LEFT / RIGHT    the previous / next slot
--   BROWSE            back (SHIFT: the Browse page)
--
-- The list starts with "(none)": loading it takes the focused slot out.

local widgets = require("lib.widgets")
local chain = require("lib.chain")
local knobs = require("lib.knobs")
local wheel = require("lib.wheel")
local util = require("lib.util")

local module = {}

local INSTRUMENT, EFFECT = 1, 2
local FORMATS = { "Internal", "Audio Units", "VST3", "VST" }
-- Maschine's own vendor, as the list names it; the built-ins' (Internal), DECKARD's own.
local NI = "NI"
local OWN = "DECKARD"
-- How far knobs 5-8 turn for a step through the list.
local LIST_STEP = 0.05

local ROWS = 8
local ROW_X, ROW_Y, ROW_W, ROW_H, ROW_STEP = 721, 44, 231, 21, 26

local kind = INSTRUMENT
local format = "Internal"
local vendor = OWN
-- The place chosen in each list (0: "(none)"), by type, format and vendor.
local chosen = {}

local function vendor_of(m)
  if m.vendor == "Native Instruments" then return NI end
  return m.vendor
end

-- A sound's first slot (or its empty chain) takes an instrument; the rest effects.
local function instrument_slot(c)
  return c.level == 3 and (#c.chain == 0 or c.slot_index == 1)
end

local function current_kind(c)
  return instrument_slot(c) and kind or EFFECT
end

local function of_kind(c, k)
  local out = {}
  for i, m in ipairs(c.modules) do
    if m.instrument == (k == INSTRUMENT) then out[#out + 1] = i end
  end
  return out
end

-- The formats with something of type `k`, in FORMATS' order.
local function formats(c, k)
  local out = {}
  for _, f in ipairs(FORMATS) do
    for _, i in ipairs(of_kind(c, k)) do
      if c.modules[i].format == f then
        out[#out + 1] = f
        break
      end
    end
  end
  return out
end

-- The vendors of type `k` in format `f`: Maschine's own first, then the others.
local function vendors(c, k, f)
  if f == "Internal" then return { OWN } end
  local out, seen = {}, {}
  for _, i in ipairs(of_kind(c, k)) do
    local m = c.modules[i]
    local v = vendor_of(m)
    if m.format == f and not seen[v] then
      seen[v] = true
      if v == NI then table.insert(out, 1, v) else out[#out + 1] = v end
    end
  end
  return out
end

-- The format and vendor chosen, or the first there are.
local function settle(c)
  local k = current_kind(c)
  local fs = formats(c, k)
  if not util.index_of(fs, format) then format = fs[1] or "Internal" end
  local vs = vendors(c, k, format)
  if not util.index_of(vs, vendor) then vendor = vs[1] or OWN end
  return k
end

-- The list shown: "(none)", then the modules' places in `c.modules`.
local function list(c)
  local k = settle(c)
  local out = {}
  for _, i in ipairs(of_kind(c, k)) do
    local m = c.modules[i]
    if m.format == format and (format == "Internal" or vendor_of(m) == vendor) then out[#out + 1] = i end
  end
  return out
end

local function key(c)
  return current_kind(c) .. "/" .. format .. "/" .. vendor
end

local function at(c)
  return chosen[key(c)] or 0
end

-- Move through the list by `steps`; whether it moved.
local function move(c, steps)
  local n = #list(c)
  local from = at(c)
  local to = util.clamp(from + steps, 0, n)
  chosen[key(c)] = to
  return to ~= from
end

local function load(c)
  local i = at(c)
  deckard.load_module(i > 0 and list(c)[i] or nil)
end

local function close()
  deckard.show_page("control")
end

-- Whether the slot before / after (or the "+") can be focused.
local function can_slot(c, next)
  local here = c.slot_index or (#c.chain + 1)
  if next then return here <= #c.chain end
  return #c.chain > 0 and here > 1
end

function module.button(name, down)
  if not down then return end
  local c = deckard.state().control
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if n and n <= 3 then
    deckard.level(n)
  elseif n == 5 or n == 6 then
    -- (Each one stepped to is loaded, as Maschine does.)
    if move(c, n == 5 and -1 or 1) then load(c) end
  elseif n == 7 then
    close()
  elseif n == 8 then
    load(c)
    close()
  elseif name == "WHEEL" then
    if wheel.mode then return end
    load(c)
    close()
  elseif name == "WHEEL_LEFT" or name == "LEFT" then
    if can_slot(c, false) then deckard.slot(-1) end
  elseif name == "WHEEL_RIGHT" or name == "RIGHT" then
    if can_slot(c, true) then deckard.slot(1) end
  elseif name == "BROWSE" then
    if deckard.held("SHIFT") then deckard.show_page("browse") else close() end
  else
    return
  end
  return true
end

function module.wheel(steps)
  if wheel.mode then return end
  move(deckard.state().control, steps)
  return true
end

function module.knob(n, amount)
  local c = deckard.state().control
  if n >= 5 then
    local steps = knobs.gather(n, amount, LIST_STEP)
    if steps ~= 0 then move(c, steps) end
    return true
  end
  local steps = knobs.gather(n, amount)
  if steps == 0 then return true end
  local k = settle(c)
  if n == 1 and instrument_slot(c) then
    local to = util.clamp(kind + steps, INSTRUMENT, EFFECT)
    if to ~= kind then
      kind, format, vendor = to, "Internal", OWN
    end
  elseif n == 2 then
    local fs = formats(c, k)
    format = fs[util.clamp((util.index_of(fs, format) or 1) + steps, 1, math.max(1, #fs))] or format
  elseif n == 3 then
    local vs = vendors(c, k, format)
    vendor = vs[util.clamp((util.index_of(vs, vendor) or 1) + steps, 1, math.max(1, #vs))] or vendor
  end
  return true
end

function module.leds()
  local c = deckard.state().control
  for i = 1, 3 do deckard.led("SCREEN_" .. i, i == c.level and "on" or "dim") end
  deckard.led("SCREEN_4", "off")
  deckard.led("SCREEN_5", at(c) > 0 and "dim" or "off")
  deckard.led("SCREEN_6", at(c) < #list(c) and "dim" or "off")
  deckard.led("SCREEN_7", "dim")
  deckard.led("SCREEN_8", "dim")
  deckard.led("BROWSE", "on")
  deckard.led("LEFT", can_slot(c, false) and "dim" or "off")
  deckard.led("RIGHT", can_slot(c, true) and "dim" or "off")
  deckard.led("WHEEL_UP", "off")
  deckard.led("WHEEL_DOWN", "off")
  if can_slot(c, false) then deckard.led("WHEEL_LEFT", 16, false) else deckard.led("WHEEL_LEFT", "off") end
  if can_slot(c, true) then deckard.led("WHEEL_RIGHT", 16, false) else deckard.led("WHEEL_RIGHT", "off") end
end

-- The list on the right screen: the chosen one filled, a scrollbar while it's longer
-- than the screen.
local function rows(screen, theme, c)
  local names = { "(none)" }
  for _, i in ipairs(list(c)) do names[#names + 1] = c.modules[i].name end
  local focus = at(c) + 1
  local n = #names
  local top = util.clamp(focus - ROWS // 2, 1, math.max(1, n - ROWS + 1))
  for row = 1, ROWS do
    local i = top + row - 1
    if not names[i] then break end
    local y = ROW_Y + (row - 1) * ROW_STEP
    local focused = i == focus
    if focused then screen:rect(ROW_X, y, ROW_W, ROW_H, "text") end
    local colour = focused and theme.colors.on_colour or theme.colors.text
    screen:label(ROW_X + 9, y, ROW_W - 14, ROW_H, names[i], { color = colour }, "right")
  end
  if n > ROWS then
    local h = 213
    screen:rect(955, ROW_Y + h * (top - 1) // n, 5, math.max(4, h * ROWS // n), "text")
  end
end

function module.draw(screen, theme)
  local s = deckard.state()
  local c = s.control
  local group = s.groups[s.group]
  local snd = group and group.sounds[s.sound]
  chain.tabs(screen, theme, c.level)
  widgets.info_bar(screen, theme, {
    number = s.sound,
    palette = snd and snd.palette,
    name = snd and snd.name or "",
    values = {
      { text = string.format("%.2f", s.tempo) },
      { text = "BPM", title = true },
      { text = s.scene_name, palette = s.scene_palette },
    },
  })
  chain.draw(screen, c)
  local k = settle(c)
  local params = {
    [2] = { name = "FORMAT", value = format },
    [3] = { name = "VENDOR", value = vendor ~= "" and vendor or "Other" },
  }
  if instrument_slot(c) then params[1] = { name = "TYPE", value = k == INSTRUMENT and "Instr." or "Effect" } end
  widgets.params(screen, theme, 0, {}, nil, params)
  widgets.buttons(screen, theme, {
    -- (Maschine sets the browser's buttons 4 px further in.)
    [5] = { text = "PREVIOUS", disabled = at(c) == 0, dx = 4 },
    [6] = { text = "NEXT", disabled = at(c) >= #list(c), dx = 4 },
    [7] = { text = "CANCEL", dx = 4 },
    [8] = { text = "LOAD", dx = 4 },
  })
  rows(screen, theme, c)
end

return module
