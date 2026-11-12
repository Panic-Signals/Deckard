-- The Control page (CHANNEL / PLUG-IN): the eight knobs turn the focused chain's
-- plug-in page, or its channel properties.
--
--   display 1-3       MASTER, GROUP, SOUND: the chain shown
--   display 5 / 6     PLUG-IN: the previous / next slot, on to the "+" after the last
--                     (an empty one); CHANNEL: the previous / next page group (Input,
--                     Output, Groove, Macro). The jog wheel's left / right the same.
--   jog wheel press   PLUG-IN: the plug-in browser (pages/module.lua) for the slot
--   LEFT / RIGHT      the previous / next page
--   right screen      the focused plug-in's picture; the built-in EQ's or Filter's curve, on both pages;
--                     CHANNEL's Generate page: Euclidean, its rhythm as a circle; Random,
--                     its notes along the pattern
--
-- Zhora focused, its own buttons and tuner instead (lib/zhora.lua). The Tuner effect
-- focused, its dial on the right screen under A4's pitch.
--
-- With SHIFT held on PLUG-IN, the focused slot's edits: EDIT (a plug-in's window),
-- INSERT (the plug-in browser for a new slot after the last), SAVE (not yet), MOVE
-- (5 / 6: before or after its neighbour; an instrument stays first), BYPASS, REMOVE.

local widgets = require("lib.widgets")
local chain = require("lib.chain")
local zhora = require("lib.zhora")

local control = {}

local PAGE_GROUPS = { "INPUT", "OUTPUT", "GROOVE", "MACRO" }

-- The focused slot's place, the "+" being the one after the last.
local function slot_at(c)
  return c.slot_index or (#c.chain + 1)
end

-- Whether display 5 / 6 (`next`) have somewhere to go: a page group, or a slot.
local function can_step(c, next)
  if not c.plugins then
    if next then return c.page_group < c.page_groups end
    return c.page_group > 1
  end
  local at = slot_at(c)
  if next then return at <= #c.chain end
  return #c.chain > 0 and at > 1
end

local function step(c, next)
  if not can_step(c, next) then return end
  if c.plugins then
    deckard.slot(next and 1 or -1)
  else
    deckard.page_group(c.page_group + (next and 1 or -1))
  end
end

-- SHIFT's slot edits: whether the focused slot can move before (or after, `right`)
-- its neighbour. (Instruments stay first, effects after them.)
local function can_move(c, right)
  local at = c.slot_index
  local slot = at and c.chain[at]
  if not slot or slot.instrument then return false end
  if right then return at < #c.chain end
  return at > 1 and not c.chain[at - 1].instrument
end

local function shift_button(c, n)
  local slot = c.slot_index and c.chain[c.slot_index]
  if n == 2 then
    if slot and slot.plugin then deckard.slot_edit("edit") end
  elseif n == 3 then
    -- (To the "+" after the last, then the browser.)
    local steps = #c.chain + 1 - slot_at(c)
    if steps ~= 0 then deckard.slot(steps) end
    deckard.show_page("module")
  elseif (n == 5 or n == 6) and can_move(c, n == 6) then
    deckard.slot_edit(n == 6 and "move_right" or "move_left")
  elseif n == 7 and slot then
    deckard.slot_edit("bypass")
  elseif n == 8 and slot then
    deckard.slot_edit("remove")
  end
end

function control.button(name, down)
  if not down then return end
  local s = deckard.state()
  local c = s.control
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if n and c.plugins and deckard.held("SHIFT") then
    shift_button(c, n)
    return true
  end
  if name == "LEFT" then
    deckard.previous_page()
  elseif name == "RIGHT" then
    deckard.next_page()
  elseif name == "WHEEL_LEFT" or name == "WHEEL_RIGHT" then
    step(c, name == "WHEEL_RIGHT")
  elseif name == "WHEEL" then
    if not c.plugins or deckard.held("SHIFT") then return end
    deckard.show_page("module")
  else
    if not n then return end
    if c.plugins and c.zhora then
      zhora.button(c, n)
    elseif n <= 3 then
      deckard.level(n)
    elseif n == 5 or n == 6 then
      step(c, n == 6)
    end
  end
  return true
end

function control.knob(n, amount)
  deckard.turn_knob(n, amount, deckard.held("SHIFT"))
  return true
end

function control.leds()
  local c = deckard.state().control
  if deckard.held("SHIFT") then
    local slot = c.plugins and c.slot_index and c.chain[c.slot_index]
    deckard.led("SCREEN_2", slot and slot.plugin and "dim" or "off")
    deckard.led("SCREEN_3", "dim")
    deckard.led("SCREEN_5", c.plugins and can_move(c, false) and "dim" or "off")
    deckard.led("SCREEN_6", c.plugins and can_move(c, true) and "dim" or "off")
    deckard.led("SCREEN_7", slot and (slot.active and "dim" or "on") or "off")
    deckard.led("SCREEN_8", slot and "dim" or "off")
    if c.slot_index and c.slot_index > 1 then deckard.led("WHEEL_LEFT", 16, false) end
    if c.slot_index and c.slot_index < c.slots then deckard.led("WHEEL_RIGHT", 16, false) end
    return
  end
  local z = c.plugins and c.zhora
  if z then
    zhora.leds(c)
  else
    for i = 1, 3 do deckard.led("SCREEN_" .. i, i == c.level and "on" or "dim") end
  end
  if can_step(c, false) then
    if not z then deckard.led("SCREEN_5", "dim") end
    deckard.led("WHEEL_LEFT", 16, false)
  end
  if can_step(c, true) then
    if not z then deckard.led("SCREEN_6", "dim") end
    deckard.led("WHEEL_RIGHT", 16, false)
  end
  if c.plugins and c.slot_index and not z then deckard.led("SCREEN_8", "dim") end
end

-- A channel properties group's icon: Input, Output, Groove or Macro.
local function channel_icon(screen, which, cx, cy, r, colour, bg)
  -- (As Maschine's, at `r` 8: Input and Output a ring open where a line runs into a dot,
  -- from the left or out to the right; Macro a knob, its pointer up to the right.)
  local u = r / 8
  if which == 1 then
    local x, y = cx + u, cy + 0.5 * u
    screen:circle(x, y, 9.25 * u, colour, 2 * u, -150, 150)
    screen:line(x - 10.5 * u, y, x, y, 1.2 * u, colour)
    screen:circle(x - 0.5 * u, y, 3.5 * u, colour)
  elseif which == 2 then
    local x, y = cx, cy + 0.5 * u
    screen:circle(x, y, 9.25 * u, colour, 2 * u, 30, 330)
    screen:line(x, y, x + 10 * u, y, 1.5 * u, colour)
    screen:circle(x + 0.5 * u, y, 3.5 * u, colour)
  elseif which == 3 then
    -- Groove, as Maschine's: a dashed line (the beat), a bar pushed off it either side,
    -- each with a dot beside it (pixels at `r` 8).
    local u = r / 8
    local function px(x, y, w, h)
      screen:rect(math.floor(cx + x * u), math.floor(cy + y * u), math.max(1, math.floor(w * u)), math.max(1, math.floor(h * u)), colour)
    end
    px(-1, -9, 1, 4)
    px(-1, 0, 1, 2)
    px(-1, 7, 1, 4)
    px(-3, -4, 9, 3)
    px(-6, 3, 9, 3)
    screen:circle(cx + 9.5 * u, cy - 2.5 * u, 1.5 * u, colour)
    screen:circle(cx - 9.5 * u, cy + 4.5 * u, 1.5 * u, colour)
  else
    local x, y = cx, cy + u
    screen:circle(x, y, 10 * u, colour, 1.5 * u)
    screen:circle(x, y, 6 * u, colour)
    screen:line(x, y, x + 5 * u, y - 5 * u, 1.6 * u, bg)
  end
end

-- The channel properties' groups: a tile each, the shown one lit.
local function channel_tiles(screen, c)
  for i, name in ipairs(PAGE_GROUPS) do
    if i > c.page_groups then break end
    local x = 36 + (i - 1) * 104
    local on = i == c.page_group
    local colour = on and "text" or { 0x7a, 0x7a, 0x7a }
    if on then screen:rect(x, 108, 96, 52, "bar") end
    screen:frame(x, 108, 96, 52, on and { 0xb9, 0xb9, 0xb9 } or { 0x66, 0x66, 0x66 })
    channel_icon(screen, i, x + 48, 124, 8, colour, on and "bar" or "background")
    local title = name:sub(1, 1) .. name:sub(2):lower()
    screen:label(x, 138, 96, 18, title, { size = 12, color = colour })
  end
end

-- A value and its unit, as the knobs show them ("-5.8" "dB").
local UNITS = { dB = true, Hz = true, kHz = true, ms = true, ["%"] = true }
local function split(text)
  local v, u = text:match("^(.-)%s+(%S+)$")
  if v and UNITS[u] then return v, u end
  return text, nil
end

-- The knobs' parameters on both screens: a section's name where it starts, and the
-- page ("1/6") at the right.
-- (`no_page`: without the "2/4" over the right knob.)
-- (`knob_of`, if given, shows each knob as it returns it, or not at all for nil.)
local function knobs(screen, theme, c, no_page, knob_of)
  local function at(i)
    local knob = c.knobs[i]
    if knob and knob_of then return knob_of(knob) end
    return knob or nil
  end
  for half = 0, 1 do
    local groups, params = {}, {}
    for k = 1, 4 do
      local i = half * 4 + k
      local knob = at(i)
      if knob then
        local before = i > 1 and at(i - 1)
        if i == 1 or not before or before.section ~= knob.section then
          groups[#groups + 1] = { text = knob.section, at = k }
        end
        local value, unit = split(knob.text)
        params[k] = { name = knob.name:upper(), value = value, unit = unit, macro = knob.macro }
      end
    end
    local page = nil
    if half == 1 and #c.pages > 1 and not no_page then page = c.page .. "/" .. #c.pages end
    widgets.params(screen, theme, half * 480, groups, page, params)
  end
end

-- (`opts`, the Step Mod page's: `knob_of` as the knob row's, and `no_browse`.)
-- A knob as it shows while Auto-write is on (the MK3 manual): only what can be
-- modulated, its modulation where the song is in place of its value.
local function auto_knob(knob)
  if not knob.modulatable then return nil end
  return { name = knob.name, section = knob.section, value = knob.value, text = knob.auto_mod or knob.text }
end

-- The built-in effect on the right screen, the same on every page. `kind` is freq
-- (20 Hz to 20 kHz, ±12 dB; the EQ and the Filter), law (input along the bottom,
-- output up the side), time (level over time), or dots (a cloud). Several freq bands
-- are numbered. The corner holds an LFO or envelope when one is sent.
local function eq_curve(screen, eq)
  local wave = eq.lfo or {}
  local show_lfo = #wave >= 2
  local kind = eq.kind or "freq"
  if kind == "" then kind = "freq" end
  -- With the modulator shape in the bottom right, the plot stops short of it.
  local x0, y0, w, h = 488, 66, show_lfo and 336 or 456, 132
  screen:rect(x0, y0, w, h, { 0x12, 0x12, 0x14 })
  local function px(t) return x0 + t * (w - 1) end
  local function py(v) return y0 + (1 - v) * (h - 1) end
  local function freq_x(hz)
    return (math.log(hz) - math.log(20)) / (math.log(20000) - math.log(20))
  end
  local grid = { 0x2c, 0x2c, 0x30 }
  if kind == "freq" then
    for _, hz in ipairs({ 100, 1000, 10000 }) do
      local x = px(freq_x(hz))
      screen:line(x, y0 + 1, x, y0 + h - 1, 1, grid)
    end
    for _, db in ipairs({ -6, 0, 6 }) do
      local y = py(db / 24 + 0.5)
      screen:line(x0 + 1, y, x0 + w - 1, y, 1, db == 0 and { 0x48, 0x48, 0x4e } or grid)
    end
  elseif kind == "law" then
    screen:line(px(0), py(0), px(1), py(1), 1, { 0x48, 0x48, 0x4e })
    for _, t in ipairs({ 0.25, 0.5, 0.75 }) do
      screen:line(x0 + 1, py(t), x0 + w - 1, py(t), 1, grid)
      screen:line(px(t), y0 + 1, px(t), y0 + h - 1, 1, grid)
    end
  else
    screen:line(x0 + 1, py(0.5), x0 + w - 1, py(0.5), 1, { 0x48, 0x48, 0x4e })
    for _, t in ipairs({ 0.25, 0.5, 0.75 }) do
      screen:line(px(t), y0 + 1, px(t), y0 + h - 1, 1, grid)
    end
  end
  local function stroke(pts, width, ink)
    local n = pts and #pts or 0
    if n < 2 then return end
    for i = 1, n - 1 do
      screen:line(px((i - 1) / (n - 1)), py(pts[i]), px(i / (n - 1)), py(pts[i + 1]), width, ink)
    end
  end
  stroke(eq.extent, 1, { 0x2a, 0x6a, 0x78 })
  stroke(eq.side, 1.5, eq.side_live and { 0xe6, 0xc2, 0x6a } or { 0x5c, 0x50, 0x3a })
  local curve_ink = eq.dim and { 0x2a, 0x6a, 0x78 } or { 0x3d, 0xc8, 0xe6 }
  stroke(eq.points, 2, curve_ink)
  local bands = eq.bands or {}
  local radius = kind == "freq" and 7 or 4
  for i, b in ipairs(bands) do
    local cx, cy = px(b.x), py(b.y)
    if kind == "time" then
      screen:line(cx, py(0), cx, cy, 1.5, { 0xff, 0xc2, 0x3a })
    end
    screen:circle(cx, cy, radius, { 0xff, 0xc2, 0x3a })
    if kind == "freq" and #bands > 1 then
      screen:label(math.floor(cx - 7), math.floor(cy - 7), 14, 14, tostring(i), { size = 10, color = { 0x14, 0x14, 0x16 } })
    end
  end
  if kind == "freq" then
    for _, hz in ipairs({ { 100, "100" }, { 1000, "1k" }, { 10000, "10k" } }) do
      local x = px(freq_x(hz[1]))
      screen:label(math.floor(x - 16), y0 + h - 14, 32, 12, hz[2], { size = 10, color = { 0x8a, 0x8a, 0x90 } })
    end
  end
  if eq.preview then
    screen:label(math.floor(x0 + 8), math.floor(y0 + 4), 80, 14, "PREVIEW", { size = 10, color = { 0x8a, 0x8a, 0x90 } })
  end
  if show_lfo then
    -- Bottom right of the right screen, beside the curve.
    local bx, by, bw, bh = 832, y0 + h - 40, 112, 40
    screen:rect(bx, by, bw, bh, { 0x0c, 0x0c, 0x0e })
    screen:frame(bx, by, bw, bh, { 0x3a, 0x3a, 0x40 })
    local m = #wave
    for i = 1, m - 1 do
      local x1 = bx + 4 + (i - 1) / (m - 1) * (bw - 8)
      local x2 = bx + 4 + i / (m - 1) * (bw - 8)
      local y1 = by + 4 + (1 - wave[i]) * (bh - 8)
      local y2 = by + 4 + (1 - wave[i + 1]) * (bh - 8)
      screen:line(x1, y1, x2, y2, 1.5, { 0xff, 0xc2, 0x3a })
    end
  end
end

-- The Generate page's Euclidean rhythm on the right screen: a dot for each step round
-- a circle (the first at the top, clockwise), each hit an arc along the rim the length
-- of its step, Fills over Steps in the middle and, while playing, a hand out from the
-- middle at where playback is, turning with the beat. Rotate turns the hits round.
local function euclid_circle(screen, e)
  local n = #e.hits
  if n == 0 then return end
  local cx, cy, r = 720, 133, 60
  local function angle(i) return -90 + 360 * i / n end
  local function point(i, radius)
    local a = math.rad(angle(i))
    return cx + radius * math.cos(a), cy + radius * math.sin(a)
  end
  local dim, hit, lit = { 0x48, 0x48, 0x4e }, { 0x3d, 0xc8, 0xe6 }, { 0xff, 0xc2, 0x3a }
  local now = e.position and math.floor(e.position) % n or nil
  screen:circle(cx, cy, r, { 0x2c, 0x2c, 0x30 }, 1)
  -- (A gap at each arc's end, so hits side by side read as two.)
  local gap = math.min(4, 120 / n)
  for i = 0, n - 1 do
    if e.hits[i + 1] then
      screen:circle(cx, cy, r + 2.5, i == now and lit or hit, 5, angle(i), angle(i + 1) - gap)
    end
  end
  for i = 0, n - 1 do
    local x, y = point(i, r)
    local on = e.hits[i + 1]
    local ink = (i == now and lit) or (on and hit) or dim
    screen:circle(x, y, (i == now and 5) or (on and 4) or 2.5, ink)
  end
  -- Fills over Steps.
  local text = { 0xe0, 0xe0, 0xe4 }
  screen:label(cx - 30, cy - 24, 60, 22, tostring(e.fills), { size = 18, color = text })
  screen:line(cx - 14, cy, cx + 14, cy, 1.5, { 0x8a, 0x8a, 0x90 })
  screen:label(cx - 30, cy + 2, 60, 22, tostring(n), { size = 18, color = text })
  if e.position then
    local a = math.rad(-90 + 360 * e.position / n)
    local c, s = math.cos(a), math.sin(a)
    screen:line(cx + 26 * c, cy + 26 * s, cx + (r - 7) * c, cy + (r - 7) * s, 2, lit)
  end
  screen:label(830, 180, 120, 20, "ROTATE " .. e.rotate, { size = 11, color = { 0x8a, 0x8a, 0x90 } }, "right", "center")
end

-- The Generate page's Random notes on the right screen: a piano roll of the
-- pattern's grid steps, left to right, and its pitches from Note Lo to Note Hi (a
-- chord a stack in one column), each note as bright as its velocity. While playing,
-- an amber line at where playback is, the notes under it amber. In the corner, the
-- velocities Generate draws from: Velocity's range around 100, shaped by Distribution.
local NOTE_NAMES = { "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B" }
local function random_view(screen, r)
  local n = math.max(1, r.steps)
  -- (A gutter at the left for the note names.)
  local x0, y0, w, h = 512, 66, 312, 132
  local cw = w / n
  local hit, lit = { 0x3d, 0xc8, 0xe6 }, { 0xff, 0xc2, 0x3a }
  local now = r.position and math.floor(r.position) % n or nil
  local grid = { 0x2c, 0x2c, 0x30 }
  screen:rect(x0, y0, w, h, { 0x12, 0x12, 0x14 })
  local beat = math.max(1, r.beat or 4)
  for i = 0, n, beat do
    local x = x0 + i * cw
    screen:line(x, y0 + 1, x, y0 + h - 1, 1, i % (beat * 4) == 0 and { 0x3a, 0x3a, 0x40 } or grid)
  end
  local lo, hi = math.min(r.low, r.high), math.max(r.low, r.high)
  local rh = (h - 2) / (hi - lo + 1)
  local function row_y(p) return y0 + 1 + (hi - p) * rh end
  for p = lo, hi do
    if p % 12 == 0 then
      local y = row_y(p) + rh
      screen:line(x0, y, x0 + w - 1, y, 1, grid)
    end
    if p % 12 == 0 or p == lo or p == hi then
      local name = NOTE_NAMES[p % 12 + 1] .. (p // 12 - 2)
      screen:label(x0 - 26, math.floor(row_y(p) + rh / 2 - 6), 23, 12, name, { size = 9, color = { 0x8a, 0x8a, 0x90 } }, "right", "center")
    end
  end
  for _, note in ipairs(r.notes) do
    if note.pitch >= lo and note.pitch <= hi then
      local ink = note.step == now and lit or hit
      local a = 0.35 + 0.65 * math.max(0, math.min(1, note.velocity / 127))
      ink = { math.floor(ink[1] * a), math.floor(ink[2] * a), math.floor(ink[3] * a) }
      screen:rect(math.floor(x0 + note.step * cw + 1), math.floor(row_y(note.pitch)), math.max(1, math.floor(cw - 1)), math.max(2, math.floor(rh - 1)), ink)
    end
  end
  if r.position then
    local x = x0 + (r.position % n) * cw
    screen:line(x, y0, x, y0 + h - 1, 2, lit)
  end
  -- The velocities drawn from: 1 to 127 across the box, Velocity's range about 100.
  local bx, by, bw, bh = 832, 158, 112, 40
  screen:rect(bx, by, bw, bh, { 0x0c, 0x0c, 0x0e })
  screen:frame(bx, by, bw, bh, { 0x3a, 0x3a, 0x40 })
  local spread = (r.velocity_amount or 0) / 100 * 63.5
  local lo_v, hi_v = 100 - spread, 100 + spread
  local function vx(v) return bx + 4 + (math.max(1, math.min(127, v)) - 1) / 126 * (bw - 8) end
  local function vy(t) return by + bh - 4 - t * (bh - 10) end
  screen:line(vx(100), by + 3, vx(100), by + bh - 3, 1, grid)
  if spread < 0.5 then
    screen:line(vx(100), vy(0), vx(100), vy(1), 2, lit)
  else
    local m, prev = 24, nil
    for k = 0, m do
      local t = k / m
      local v = lo_v + t * (hi_v - lo_v)
      local d = r.distribution or 1
      local y
      if d == 0 then y = 0.7
      elseif d == 1 then y = math.exp(-((t - 0.5) / 0.25) ^ 2 / 2)
      else y = math.exp(-(t / 0.25) ^ 2 / 2) end
      -- (Draws past 127 or under 1 are drawn again: they pile up at the ends.)
      local pt = { vx(v), vy(y) }
      if v >= 1 and v <= 127 then
        if prev then screen:line(prev[1], prev[2], pt[1], pt[2], 1.5, lit) end
        prev = pt
      end
    end
  end
  local names = { "EVEN", "NATURAL", "SKEWED" }
  screen:label(bx, by - 16, bw, 14, "VELOCITY · " .. (names[(r.distribution or 1) + 1] or ""), { size = 9, color = { 0x8a, 0x8a, 0x90 } }, "left", "center")
end

function control.draw(screen, theme, opts)
  opts = opts or {}
  local s = deckard.state()
  local knob_of = opts.knob_of or (s.auto_write and auto_knob or nil)
  local c = s.control
  local group = s.groups[s.group]
  local snd = group and group.sounds[s.sound]
  local shift_layer = c.plugins and deckard.held("SHIFT")
  local z = c.plugins and not shift_layer and c.zhora
  if not shift_layer and not z then chain.tabs(screen, theme, c.level) end
  -- (The level's: the master's "M" in white, the group's label, or the sound's number.)
  local info = widgets.focus_info(s)
  if c.level == 1 then
    info.number, info.palette, info.name = "M", 16, ""
  elseif c.level == 2 and group then
    info.number = string.char(string.byte("A") + (s.group - 1) % 8) .. ((s.group - 1) // 8 + 1)
    info.palette, info.name = group.palette, group.name
  end
  widgets.info_bar(screen, theme, info)
  if shift_layer then
    local slot = c.slot_index and c.chain[c.slot_index]
    widgets.buttons(screen, theme, {
      [2] = { text = "EDIT", disabled = not (slot and slot.plugin) },
      [3] = { text = "INSERT" },
      [4] = { text = "SAVE", disabled = true },
      [5] = { arrows = true, text = "MOVE", left = can_move(c, false), right = can_move(c, true) },
      [7] = { text = "BYPASS", disabled = not slot, selected = slot and not slot.active },
      [8] = { text = "REMOVE", disabled = not slot },
    })
  elseif z then
    zhora.buttons(screen, theme, c)
  elseif c.plugins then
    local slot = c.chain[slot_at(c)]
    widgets.buttons(screen, theme, {
      [5] = { arrows = true, text = slot and slot.name:upper() or "EMPTY", left = can_step(c, false), right = can_step(c, true) },
    })
  end
  if c.plugins then
    local slot = c.chain[slot_at(c)]
    -- Browse.
    if not shift_layer and not z and not opts.no_browse then
      widgets.buttons(screen, theme, { [8] = { text = "BROWSE" } })
    end
    if z then
      zhora.right(screen, z)
      zhora.left(screen, c)
      knobs(screen, theme, c, false, knob_of)
      return
    end
    screen:rect(480, 35, 480, 25, "bar")
    if slot then
      screen:label(480 + 21, 36, 480 - 21 - 18, 24, slot.name, "default", "right", "center")
      if c.tuner then
        -- The Tuner effect: A4's pitch (and Muted) on the bar, the dial under it.
        local t = c.tuner
        screen:label(490, 36, 300, 24, t.reference .. (t.muted and "  ·  Muted" or ""), "default", "left", "center")
        zhora.dial(screen, t)
      elseif c.eq then eq_curve(screen, c.eq)
      elseif c.filter then eq_curve(screen, c.filter)
      elseif c.picture then eq_curve(screen, c.picture)
      else chain.thumbnail(screen, 480, 68, 240, 136, slot, false) end
    end
    chain.draw(screen, c)
  else
    widgets.buttons(screen, theme, {
      [5] = { arrows = true, text = PAGE_GROUPS[c.page_group] or "", left = can_step(c, false), right = can_step(c, true) },
    })
    channel_tiles(screen, c)
    screen:rect(480, 35, 480, 25, "bar")
    if c.euclid then
      screen:label(490, 36, 300, 24, "Euclidean", "default", "left", "center")
      euclid_circle(screen, c.euclid)
    elseif c.random then
      local r = c.random
      screen:label(490, 36, 200, 24, "Random", "default", "left", "center")
      local made = #r.notes .. (#r.notes == 1 and " note" or " notes")
      screen:label(700, 36, 250, 24, string.format("%.0f %%  ·  %s", r.probability, made), "default", "right", "center")
      random_view(screen, r)
    else
      channel_icon(screen, c.page_group, 720, 126, 37, { 0x3a, 0x3a, 0x3a }, "background")
    end
  end
  knobs(screen, theme, c, false, knob_of)
end

-- The knobs' row as the page draws it (Set Macro shows it too).
control.knob_row = knobs

return control
