-- The LFOs (SHIFT + SELECT; again, or another page's button, leaves).
--
--   left screen       the focused LFO's name, large ("Press a pad" with none)
--   right screen      its wave over two cycles, under its shape, rate and "trig"
--   display 1-5       the shapes: Sine, Square, Triangle, Saw, Random
--   display 6 / 8     SYNC (lit: synced, else Hz) / TRIG (the focused sound, or Off);
--                     with SHIFT held, 8 is DELETE: removes the focused LFO
--   display 7         ENV: Envelope mode (lit), an ADSR its trigger's notes play; then
--                     1-5 are dark, 6 is RETRIG (lit: each note restarts it), and 8
--                     sets the trigger to the focused sound (never Off)
--   knobs 1-7         Shape, Rate, Phase, Level, Quantize, Trigger, Fade in (with a
--                     trigger); an envelope's Attack, Decay, Sustain, Release, Level,
--                     Quantize, Trigger
--   pads              an LFO's pad focuses it; an empty one makes one there; with ERASE
--                     held (with SHIFT too), an LFO's pad removes it; with MUTE held, mutes or unmutes
--                     it (they play nothing). Lit white, red when muted; bright: focused
--   MUTE              held here, it doesn't show the hold page

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local pin = require("lib.pin")

local lfo = {}

local SHAPES = { "Sine", "Square", "Triangle", "Saw", "Random" }
local BUTTONS = { "SINE", "SQR", "TRI", "SAW", "RAND" }

-- Each knob: what deckard.lfo calls it, and how far it turns for a step (lists a
-- tenth or a twentieth of a turn, the rest a hundredth).
local KNOBS = {
  { "shape", 0.1 }, { "rate", 0.05 }, { "phase", 0.01 }, { "level", 0.01 },
  { "quantize", 0.05 }, { "trigger", 0.1 }, { "fade", 0.02 },
}
local ENV_KNOBS = {
  { "attack", 0.01 }, { "decay", 0.01 }, { "sustain", 0.01 }, { "release", 0.01 },
  { "level", 0.01 }, { "quantize", 0.05 }, { "trigger", 0.1 },
}

-- The LFO on pad `pad`, if there is one.
local function on(s, pad)
  for _, l in ipairs(s.lfos) do
    if l.pad == pad then return l end
  end
end

local function shape_index(l)
  for i, name in ipairs(SHAPES) do
    if name == l.shape then return i end
  end
  return 1
end

local function triggered(l)
  return l.trigger ~= "Off" and l.trigger ~= "None"
end

-- A number as short as it goes: 1, 1.5, 0.25.
local function num(x)
  return (string.format("%.2f", x):gsub("%.?0+$", ""))
end

function lfo.button(name, down)
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  -- (MUTE is held for the pads, not for the hold page. One held over from a hold page
  -- shown before this one lets go of that page when it's released.)
  if name == "MUTE" then
    if not down and pin.shown and pin.shown.button == "MUTE" then pin.shown = nil end
    return true
  end
  if name == "LEFT" or name == "RIGHT" then return true end
  if not n then return end
  if not down then return true end
  local s = deckard.state()
  local l = on(s, s.lfo_focus)
  if not l then return true end
  if n == 8 and deckard.held("SHIFT") then
    deckard.lfo("remove", l.pad)
  elseif n == 7 then
    deckard.lfo("env", l.pad)
  elseif l.env then
    if n == 6 then
      deckard.lfo("retrigger", l.pad)
    elseif n == 8 then
      deckard.lfo("trigger_focused", l.pad, 1)
    end
  elseif n <= 5 then
    deckard.lfo("shape", l.pad, n)
  elseif n == 6 then
    deckard.lfo("sync", l.pad, l.sync and 0 or 1)
  elseif n == 8 then
    deckard.lfo("trigger_focused", l.pad, triggered(l) and 0 or 1)
  end
  return true
end

function lfo.pad(n)
  local s = deckard.state()
  local l = on(s, n)
  if deckard.held("ERASE") then
    if l then deckard.lfo("remove", n) end
  elseif deckard.held("MUTE") then
    if l then deckard.lfo("mute", n) end
  elseif l then
    deckard.lfo("focus", n)
  else
    deckard.lfo("create", n)
  end
  return true
end

function lfo.pads()
  return { play = false }
end

function lfo.knob(n, amount)
  local s = deckard.state()
  local l = on(s, s.lfo_focus)
  local k = l and (l.env and ENV_KNOBS or KNOBS)[n]
  if not k or (k[1] == "fade" and not triggered(l)) then return true end
  local steps = knobs.gather(n, amount, k[2])
  if steps == 0 then return true end
  if k[1] == "shape" then
    local to = shape_index(l) + steps
    if to >= 1 and to <= #SHAPES then deckard.lfo("shape", l.pad, to) end
  else
    deckard.lfo(k[1], l.pad, steps)
  end
  return true
end

function lfo.leds()
  local s = deckard.state()
  local l = on(s, s.lfo_focus)
  local env = l and l.env
  for i = 1, 5 do
    deckard.led("SCREEN_" .. i, l and not env and (shape_index(l) == i and "on" or "dim") or "off")
  end
  local six = l and (env and l.retrigger or (not env and l.sync))
  deckard.led("SCREEN_6", l and (six and "on" or "dim") or "off")
  deckard.led("SCREEN_7", l and (env and "on" or "dim") or "off")
  deckard.led("SCREEN_8", l and (triggered(l) and not deckard.held("SHIFT") and "on" or "dim") or "off")
  deckard.led("LEFT", "off")
  deckard.led("RIGHT", "off")
  deckard.led("SELECT", "on")
  for n = 1, 16 do
    local p = on(s, n)
    if p then
      deckard.led("PAD_" .. n, p.muted and 0 or 16, n == s.lfo_focus)
    else
      deckard.led("PAD_" .. n, "off")
    end
  end
end

-- A screen's bar under the display buttons: `left` and `right` in it.
local function bar(screen, x, left, right)
  screen:rect(x, 32, 480, 20, "bar")
  screen:label(x + 10, 32, 300, 20, left, { size = 12, color = "weak" }, "left", "center")
  if right then screen:label(x + 170, 32, 300, 20, right, { size = 12, color = "weak" }, "right", "center") end
end

-- The wave (`points`, 0..1 over two cycles) in the box `x, y, w, h`; an envelope's
-- (`note_off` given) with a dashed line where the note ends instead of the middle one.
local function wave(screen, points, x, y, w, h, note_off)
  screen:rect(x, y, w, 1, "bar")
  screen:rect(x, y + h, w, 1, "bar")
  if note_off then
    local nx = x + math.floor(note_off * w + 0.5)
    for dy = 0, h - 1, 8 do
      screen:rect(nx, y + dy, 1, math.min(4, h - dy), "bar")
    end
  else
    screen:rect(x + w // 2, y, 1, h, "bar")
  end
  local n = #points
  if n < 2 then return end
  local px = function(i) return x + (i - 1) / (n - 1) * w end
  local py = function(i) return y + h - points[i] * h end
  for i = 1, n - 1 do
    screen:line(px(i), py(i), px(i + 1), py(i + 1), 2, "accent")
  end
end

function lfo.draw(screen, theme)
  local s = deckard.state()
  local l = on(s, s.lfo_focus)
  local env = l and l.env
  local specs = {}
  for i, text in ipairs(BUTTONS) do
    specs[i] = env and { text = "" } or { text = text, selected = l and shape_index(l) == i, disabled = not l }
  end
  if env then
    specs[6] = { text = "RETRIG", selected = l.retrigger }
  else
    specs[6] = { text = "SYNC", selected = l and l.sync, disabled = not l }
  end
  specs[7] = { text = "ENV", selected = env, disabled = not l }
  if deckard.held("SHIFT") then
    specs[8] = { text = "DELETE", disabled = not l }
  else
    specs[8] = { text = "TRIG", selected = l and triggered(l), disabled = not l }
  end
  widgets.buttons(screen, theme, specs)

  bar(screen, 0, "LFO", "PAD: CREATE / SELECT")
  if not l then
    screen:label(0, 52, 480, 150, "Press a pad", { size = 28, color = "weak" }, "center", "center")
    bar(screen, 480, "", nil)
    return
  end
  screen:label(0, 52, 480, 150, l.name, { font = "roboto-bold", size = 64, color = l.muted and "weak" or "text" }, "center", "center")
  if l.muted then
    screen:label(0, 166, 480, 30, "MUTED", { size = 18, color = "weak" }, "center", "center")
  end

  if env then
    bar(screen, 480, "Envelope", l.trigger)
    wave(screen, l.points or {}, 492, 64, 456, 132, l.note_off or 0.5)
    widgets.params(screen, theme, 0, { { text = "Envelope", at = 1 } }, nil, {
      { name = "ATTACK", value = l.attack },
      { name = "DECAY", value = l.decay },
      { name = "SUSTAIN", value = tostring(math.floor(l.sustain * 100 + 0.5)), unit = "%" },
      { name = "RELEASE", value = l.release },
    })
    widgets.params(screen, theme, 480, { { text = "Output", at = 1 }, { text = "Trigger", at = 3 } }, nil, {
      { name = "LEVEL", value = tostring(math.floor(l.level * 100 + 0.5)), unit = "%" },
      { name = "QUANTIZE", value = l.quantize == 0 and "Off" or tostring(l.quantize) },
      { name = "TRIGGER", value = l.trigger },
    })
    return
  end
  local head = l.shape .. " · " .. l.rate
  if triggered(l) then head = head .. " · trig" end
  bar(screen, 480, head, triggered(l) and l.trigger or nil)
  wave(screen, l.points or {}, 492, 64, 456, 132)

  widgets.params(screen, theme, 0, { { text = "Wave", at = 1 } }, nil, {
    { name = "SHAPE", value = l.shape },
    { name = "RATE", value = l.rate },
    { name = "PHASE", value = tostring(math.floor(l.phase + 0.5)), unit = "°" },
    { name = "LEVEL", value = tostring(math.floor(l.level * 100 + 0.5)), unit = "%" },
  })
  widgets.params(screen, theme, 480, { { text = "Trigger", at = 2 } }, nil, {
    { name = "QUANTIZE", value = l.quantize == 0 and "Off" or tostring(l.quantize) },
    { name = "TRIGGER", value = l.trigger },
    triggered(l) and { name = "FADE IN", value = num(l.fade), unit = l.fade == 1 and "beat" or "beats" } or nil,
  })
end

return lfo
