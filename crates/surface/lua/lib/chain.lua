-- A chain's slots as the Control page and the plug-in browser draw them: the MASTER /
-- GROUP / SOUND tabs, and the slots along the left screen.

local widgets = require("lib.widgets")

local chain = {}

local LEVELS = { "MASTER", "GROUP", "SOUND" }

-- MASTER / GROUP / SOUND: one frame, the chosen tab filled.
function chain.tabs(screen, theme, level)
  screen:frame(4, 4, 353, 24, "button", theme.corner)
  local xs = { { 4, 117 }, { 121, 119 }, { 240, 117 } }
  for i, name in ipairs(LEVELS) do
    local x, w = xs[i][1], xs[i][2]
    local st = "head"
    if i == level then
      screen:rect(x + 2, 4, w - 1, 24, "button_selected")
      widgets.chase(screen, theme, x + 2, 4, w - 1, 24)
      st = "head_selected"
    end
    screen:label(x, 5, w, 23, name, st)
  end
end

-- A hosted plug-in's picture, as DECKARD has none of its own: a synthesizer for an
-- instrument, an effect pedal for an effect.
local function generic(screen, x, y, w, h, instrument)
  local body, edge, knob = { 0x2a, 0x2a, 0x2e }, { 0x55, 0x55, 0x5c }, { 0xc8, 0xc8, 0xc8 }
  if instrument then
    -- A synth: a panel of knobs over a keyboard.
    local sw = math.min(w, math.floor(h * 3.2))
    local sx = x + (w - sw) // 2
    screen:rect(sx, y, sw, h, body, 0.6)
    screen:frame(sx, y, sw, h, edge, 0.6)
    local n = math.max(3, sw // 16)
    local r = math.max(2, math.min(sw / n * 0.3, h * 0.12))
    for k = 0, n - 1 do
      screen:circle(sx + (k + 0.5) * sw / n, y + h * 0.22, r, knob, 1.2)
    end
    local ky, kh = y + math.floor(h * 0.45), h - math.floor(h * 0.45) - 3
    local keys = math.max(7, sw // 9)
    local kw = (sw - 6) / keys
    screen:rect(sx + 3, ky, sw - 6, kh, { 0xe8, 0xe8, 0xe8 })
    for k = 1, keys - 1 do
      screen:line(sx + 3 + k * kw, ky, sx + 3 + k * kw, ky + kh, 1, { 0x60, 0x60, 0x60 })
      if k % 7 ~= 3 and k % 7 ~= 0 then
        screen:rect(math.floor(sx + 3 + k * kw - kw * 0.3), ky, math.max(1, math.floor(kw * 0.6)), math.floor(kh * 0.6), { 0x18, 0x18, 0x18 })
      end
    end
  else
    -- A pedal: a box with knobs, a light and a footswitch.
    local pw = math.min(w, math.floor(h * 0.8))
    local px = x + (w - pw) // 2
    screen:rect(px, y, pw, h, body, 0.6)
    screen:frame(px, y, pw, h, edge, 0.6)
    for k = 0, 1 do
      screen:circle(px + pw * (0.3 + k * 0.4), y + h * 0.2, math.max(2, pw * 0.1), knob, 1.2)
    end
    screen:circle(px + pw / 2, y + h * 0.42, math.max(1, pw * 0.05), { 0xe0, 0x40, 0x30 })
    screen:circle(px + pw / 2, y + h * 0.72, math.max(2, pw * 0.16), knob, 1.4)
  end
end

-- A slot's picture: DECKARD's own of the plug-in where it has one, else a drawing (of
-- a synthesizer or pedal for a hosted plug-in; a panel for DECKARD's own). `thumb`:
-- the chain's (else the right display's logo).
function chain.thumbnail(screen, x, y, w, h, slot, thumb)
  local instrument = slot.instrument
  if slot.picture and screen:image(x, y, w, h, slot.picture, thumb ~= false) then return end
  if slot.plugin then return generic(screen, x, y, w, h, instrument) end
  screen:rect(x, y, w, h, { 0x1e, 0x1e, 0x1e })
  screen:frame(x, y, w, h, { 0x3a, 0x3a, 0x3a })
  screen:rect(x + 1, y + 1, w - 2, 6, { 0x2c, 0x2c, 0x2c })
  screen:rect(x + 3, y + 3, math.min(24, w // 4), 2, { 0x8a, 0x8a, 0x8a })
  if instrument then
    screen:rect(x + 4, y + 10, w - 8, h - 14, { 0x24, 0x24, 0x24 })
  else
    local n = math.max(2, (w - 8) // 14)
    for k = 0, n - 1 do
      screen:circle(x + 9 + k * 14, y + h - 12, 4, { 0x9a, 0x9a, 0x9a }, 1.4)
    end
  end
end

-- The thumbnail sizes of Maschine's own plug-ins (others: an effect's or an
-- instrument's).
local SIZES = { Sampler = { 160, 42 }, EQ = { 90, 40 }, Compressor = { 62, 40 } }

-- A slot's kind, as a small icon: keys for an instrument, FX for an effect.
local function kind_icon(screen, x, y, instrument)
  if instrument then
    for k = 0, 2 do
      screen:rect(x + k * 4, y, 2, 7, "text")
      screen:rect(x + k * 4, y + 7, 3, 5, "text")
    end
  else
    screen:frame(x, y, 12, 12, { 0xaa, 0xaa, 0xaa }, 0.5)
    screen:label(x, y + 1, 12, 11, "FX", { size = 8, color = { 0xaa, 0xaa, 0xaa } })
  end
end

-- How wide the "+" for a new slot is, and the left screen.
local NEW_W, SCREEN_W = 58, 480

-- Slot `slot`'s picture size and how wide it is in the chain.
local function slot_size(screen, slot)
  local size = SIZES[slot.name] or (slot.instrument and { 160, 42 } or { 90, 40 })
  local name_w = math.max(54, screen:text_width(slot.name, "default"))
  return size, name_w, 4 + math.max(size[1] + 16 + 3, 28 + name_w) + 3
end

-- The chain along the left screen: each slot's kind, name and picture, the focused
-- one framed, then a "+" for a new one. Longer than the screen, it scrolls to keep the
-- focused slot (or the "+") in view, showing only whole slots.
function chain.draw(screen, c)
  local widths = {}
  for i, slot in ipairs(c.chain) do
    local _, _, w = slot_size(screen, slot)
    widths[i] = w
  end
  widths[#c.chain + 1] = NEW_W
  local focus = c.slot_index or #c.chain + 1
  local first, span = focus, widths[focus]
  while first > 1 and span + widths[first - 1] <= SCREEN_W do
    first = first - 1
    span = span + widths[first]
  end
  local x = 0
  for i = first, #c.chain do
    local slot = c.chain[i]
    local size, name_w, w = slot_size(screen, slot)
    if x + w > SCREEN_W then return end
    local focused = i == c.slot_index
    kind_icon(screen, x + 13, 72, slot.instrument)
    local colour = focused and "text" or { 0xb1, 0xb1, 0xb1 }
    screen:label(x + 29, 71, name_w, 17, slot.name, { color = colour }, "left", "center")
    if focused then
      screen:frame(x + 9, 91, size[1] + 6, size[2] + 6, "text", 0.76)
      screen:frame(x + 10, 92, size[1] + 4, size[2] + 4, "text")
    end
    chain.thumbnail(screen, x + 12, 94, size[1], size[2], slot)
    x = x + w
  end
  if x + NEW_W > SCREEN_W then return end
  -- A new slot, framed while it's the one focused (the plug-in browser loads into it).
  if not c.slot_index then
    screen:frame(x + 9, 91, 46, 46, "text", 0.76)
    screen:frame(x + 10, 92, 44, 44, "text")
  end
  screen:rect(x + 12 + 11, 113, 18, 2, { 0xbb, 0xbb, 0xbb })
  screen:rect(x + 12 + 19, 105, 2, 18, { 0xbb, 0xbb, 0xbb })
end

return chain
