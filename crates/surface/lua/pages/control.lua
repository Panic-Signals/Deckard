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
-- focused, its dial on the right screen under A4's pitch. A synth focused, its signal's
-- scope beside its picture.
--
-- With SHIFT held on PLUG-IN, the focused slot's edits: EDIT (a plug-in's window),
-- INSERT (the plug-in browser for a new slot after the last), SAVE (not yet), MOVE
-- (5 / 6: before or after its neighbour; an instrument stays first), BYPASS, REMOVE.

local widgets = require("lib.widgets")
local chain = require("lib.chain")
local knob_lib = require("lib.knobs")
local zhora = require("lib.zhora")
local touch = require("lib.touch")

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
  local cap = tonumber(name:match("^CAP_(%d)$"))
  if cap then
    if not down then
      knob_lib.erased[cap] = nil
      touch.up(cap, deckard.now())
    end
    if deckard.held("ERASE") then
      if down then knob_lib.erase(cap) end
      return true
    end
    if down then touch.down(cap, deckard.now()) end
  end
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
  if deckard.held("ERASE") then
    knob_lib.erase(n)
    return true
  end
  touch.turned(n, deckard.now())
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
    local colour = on and "text" or "weak"
    if on then screen:rect(x, 108, 96, 52, "bar") end
    screen:frame(x, 108, 96, 52, on and "text" or "button")
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
-- (`pictured[i]`: knob i is in a picture over the band, not a dial.)
local function knobs(screen, theme, c, no_page, knob_of, pictured)
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
        params[k] = {
          name = knob.name:upper(),
          value = value,
          unit = unit,
          macro = knob.macro,
          modulated = knob.modulated,
        }
      end
    end
    local page = nil
    -- (A theme with its knobs centred shows the page up on the bar instead.)
    if half == 1 and #c.pages > 1 and not no_page and not theme.knobs_centred then page = c.page .. "/" .. #c.pages end
    local in_picture = {}
    for k = 1, 4 do in_picture[k] = pictured and pictured[half * 4 + k] or nil end
    widgets.params(screen, theme, half * 480, groups, page, params, in_picture)
  end
end

-- (`opts`, the Step Mod page's: `knob_of` as the knob row's, and `no_browse`.)
-- A knob as it shows while Auto-write is on (the MK3 manual): only what can be
-- modulated, its modulation where the song is in place of its value.
local function auto_knob(knob)
  if not knob.modulatable then return nil end
  return {
    name = knob.name,
    section = knob.section,
    value = knob.value,
    text = knob.auto_mod or knob.text,
    macro = knob.macro,
    modulated = knob.modulated,
  }
end

-- The built-in effect on the right screen, the same on every page, in the picture
-- band: the Rust `curve` widget (crates/surface/src/widget/curve.rs). `kind` is freq
-- (20 Hz to 20 kHz, ±12 dB; the EQ and the Filter), law (input along the bottom,
-- output up the side), time (level over time), or dots (a cloud). Several freq bands
-- are numbered. The corner holds an LFO or envelope when one is sent. It draws in the
-- theme: the curve its accent, the handles its record colour, the grid its buttons.
local function eq_curve(screen, theme, eq)
  screen:widget("curve", 488, widgets.BAND_Y, 456, widgets.BAND_H, eq)
end

-- The focused synth's preview beside its picture, on the right screen: the wave's
-- lowest and highest value in each column (0 the middle line), so a fast signal is
-- solid rather than aliased, over the note's amplitude envelope. A pixel per column,
-- with anti-aliased edges, so the curves read smoothly rather than as blocks.
-- `scope.wave` runs low, high, low, high…; `scope.env` is one level per column, 0..1.
local function scope_view(screen, scope)
  local wave = scope.wave or {}
  local env = scope.env or {}
  local x0, y0, w, h = 728, 68, 224, 136
  screen:rect(x0, y0, w, h, "panel")
  screen:frame(x0, y0, w, h, "button")
  local mid = 116
  screen:line(x0 + 1, mid, x0 + w - 2, mid, 1, "separator")
  local left, inner = x0 + 3, w - 6
  local n = math.floor(#wave / 2)
  if n > 0 then
    local cw = inner / n
    local ink = math.max(1, cw + 0.4)
    local half = 44
    local prev_x, prev_lo, prev_hi = nil, nil, nil
    for i = 0, n - 1 do
      local lo = math.min(1, math.max(-1, wave[i * 2 + 1] or 0))
      local hi = math.min(1, math.max(-1, wave[i * 2 + 2] or 0))
      local x = left + (i + 0.5) * cw
      screen:line(x, mid - hi * half, x, mid - lo * half, ink, "accent")
      if prev_x then
        screen:line(prev_x, mid - prev_hi * half, x, mid - hi * half, ink, "accent")
        screen:line(prev_x, mid - prev_lo * half, x, mid - lo * half, ink, "accent")
      end
      prev_x, prev_lo, prev_hi = x, lo, hi
    end
  end
  screen:line(x0 + 1, 164, x0 + w - 2, 164, 1, "button")
  local en = #env
  if en > 0 then
    local cw = inner / en
    local ink = math.max(1, cw + 0.4)
    for i = 0, en - 1 do
      local level = math.min(1, math.max(0, env[i + 1] or 0))
      local x = left + (i + 0.5) * cw
      screen:line(x, 200 - level * 32, x, 200, ink, "accent")
    end
    screen:line(left, 200, left + inner, 200, 1, "accent")
  end
end

-- The Generate page's Euclidean rhythm on the right screen: a dot for each step round
-- a circle (the first at the top, clockwise), each hit an arc along the rim the length
-- of its step, Fills over Steps in the middle and, while playing, a hand out from the
-- middle at where playback is, turning with the beat. Rotate turns the hits round.
-- The Euclidean and Random views' colours: the theme's (the LCD sets them), else
-- these. `hit` a step or note, `now` the one playing, `dim` a rest, `ring` the
-- circle, `text`/`weak` the numbers and names, `field` and `grid`/`bar` the roll,
-- `box` the velocity box.
local function groove_colours(theme)
  local c = theme.colors
  return {
    hit = c.groove_hit or { 0x3d, 0xc8, 0xe6 },
    now = c.groove_now or { 0xff, 0xc2, 0x3a },
    dim = c.groove_dim or { 0x48, 0x48, 0x4e },
    ring = c.groove_ring or { 0x2c, 0x2c, 0x30 },
    text = c.groove_text or { 0xe0, 0xe0, 0xe4 },
    weak = c.groove_weak or { 0x8a, 0x8a, 0x90 },
    field = c.groove_field or { 0x12, 0x12, 0x14 },
    grid = c.groove_grid or { 0x2c, 0x2c, 0x30 },
    bar = c.groove_bar or { 0x3a, 0x3a, 0x40 },
    box = c.groove_box or { 0x0c, 0x0c, 0x0e },
  }
end

local function euclid_circle(screen, theme, e)
  local n = #e.hits
  if n == 0 then return end
  local cx, cy, r = 720, widgets.BAND_Y + widgets.BAND_H // 2, widgets.BAND_H // 2 - 8
  local function angle(i) return -90 + 360 * i / n end
  local function point(i, radius)
    local a = math.rad(angle(i))
    return cx + radius * math.cos(a), cy + radius * math.sin(a)
  end
  local gc = groove_colours(theme)
  local dim, hit, lit = gc.dim, gc.hit, gc.now
  local now = e.position and math.floor(e.position) % n or nil
  screen:circle(cx, cy, r, gc.ring, 1)
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
  local text = gc.text
  screen:label(cx - 30, cy - 24, 60, 22, tostring(e.fills), { size = 18, color = text })
  screen:line(cx - 14, cy, cx + 14, cy, 1.5, gc.weak)
  screen:label(cx - 30, cy + 2, 60, 22, tostring(n), { size = 18, color = text })
  if e.position then
    local a = math.rad(-90 + 360 * e.position / n)
    local c, s = math.cos(a), math.sin(a)
    screen:line(cx + 26 * c, cy + 26 * s, cx + (r - 7) * c, cy + (r - 7) * s, 2, lit)
  end
  screen:label(830, widgets.BAND_Y + widgets.BAND_H - 20, 120, 20, "ROTATE " .. e.rotate, { size = 11, color = gc.weak }, "right", "center")
end

-- The Generate page's Random notes on the right screen: a piano roll of the
-- pattern's grid steps, left to right, and its pitches from Note Lo to Note Hi (a
-- chord a stack in one column), each note as bright as its velocity. While playing,
-- an amber line at where playback is, the notes under it amber. In the corner, the
-- velocities Generate draws from: Velocity's range around 100, shaped by Distribution.
local NOTE_NAMES = { "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B" }
local function random_view(screen, theme, r)
  local n = math.max(1, r.steps)
  -- (A gutter at the left for the note names.)
  local x0, y0, w, h = 512, widgets.BAND_Y, 312, widgets.BAND_H
  local cw = w / n
  local gc = groove_colours(theme)
  local hit, lit = gc.hit, gc.now
  local now = r.position and math.floor(r.position) % n or nil
  local grid = gc.grid
  screen:rect(x0, y0, w, h, gc.field)
  local beat = math.max(1, r.beat or 4)
  for i = 0, n, beat do
    local x = x0 + i * cw
    screen:line(x, y0 + 1, x, y0 + h - 1, 1, i % (beat * 4) == 0 and gc.bar or grid)
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
      screen:label(x0 - 26, math.floor(row_y(p) + rh / 2 - 6), 23, 12, name, { size = 9, color = gc.weak }, "right", "center")
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
  local bx, by, bw, bh = 832, widgets.BAND_Y + widgets.BAND_H - 40, 112, 40
  screen:rect(bx, by, bw, bh, gc.box)
  screen:frame(bx, by, bw, bh, gc.bar)
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
  screen:label(bx, by - 16, bw, 14, "VELOCITY · " .. (names[(r.distribution or 1) + 1] or ""), { size = 9, color = gc.weak }, "left", "center")
end

-- The page on the right screen's bar: its name, then a pip per page, the shown one lit.
-- Past PIPS pages, a track as long as eight pips with the shown page lit along it, as
-- a scroll bar's, and the count. Returns where it ends.
local PIPS, TRACK_W = 8, 8 * 10 + 8
local function page_pips(screen, theme, c)
  local n = #c.pages
  local name = c.pages[c.page] or ""
  local w = screen:label(494, 37, 160, 24, name, "param_name", "left", "center")
  local x = 494 + w + 10
  if n <= PIPS then
    for i = 1, n do
      local on = i == c.page
      screen:rect(x, 47, on and 14 or 6, 4, on and "accent" or "button", 0.5)
      x = x + (on and 14 or 6) + 4
    end
    return x
  end
  local thumb = math.max(8, TRACK_W // n)
  screen:rect(x, 48, TRACK_W, 2, "button")
  local at = x + math.floor((TRACK_W - thumb) * (c.page - 1) / (n - 1) + 0.5)
  screen:rect(at, 47, thumb, 4, "accent", 0.5)
  return x + TRACK_W + 8 + screen:label(x + TRACK_W + 8, 37, 60, 24, c.page .. " / " .. n, "param_name", "left", "center")
end

-- The knobs in touch focus (CAP_1..8 rested on, just lifted from, or just turned).
local function touched()
  return touch.focused(deckard.now())
end

-- The widest run of knobs `from`..`to` not in `taken` (a set of knob numbers): its first
-- knob and how many (0 when there's none).
local function free_run(taken, from, to)
  local best_at, best, at, n = from, 0, nil, 0
  for k = from, to do
    if taken[k] then
      n = 0
    else
      if n == 0 then at = k end
      n = n + 1
      if n > best then best_at, best = at, n end
    end
  end
  return best_at, best
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
  local in_tabs = theme.info_in_tabs and not shift_layer and not z and c.plugins
  -- (The level's: the master's "M" in white, the group's label, or the sound's number.)
  local info = widgets.focus_info(s)
  if c.level == 1 then
    info.number, info.palette, info.name = "M", 16, ""
  elseif c.level == 2 and group then
    info.number = string.char(string.byte("A") + (s.group - 1) % 8) .. ((s.group - 1) // 8 + 1)
    info.palette, info.name = group.palette, group.name
  end
  -- (The knobs in a picture over the band rather than a dial.)
  local pictured = {}
  if in_tabs then
    -- (No info bar: the level's number and name at the end of the chain strip's row.)
    chain.tabs(screen, theme, c.level)
    if c.level == 1 then info.number = nil end
    -- (Its name and number go to the right screen's row, after the page: the strip has
    -- the left one to itself.)
  else
    if not shift_layer and not z then chain.tabs(screen, theme, c.level) end
    widgets.info_bar(screen, theme, info)
  end
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
    -- Pictures over the knobs they draw (a filter over Cutoff and Reso, an envelope
    -- over its times...) and a dial over every other knob, in the band on both screens;
    -- not over the right screen's own picture of a built-in effect. A choice's list
    -- goes over the right screen's picture (drawn after it, on its own background)
    -- rather than taking it away.
    local own = slot and (c.tuner or c.eq or c.filter or c.picture)
    local right_free, left_free = true, true
    -- (The knobs whose cells hold a list of choices: a picture goes beside them, whole, or
    -- not at all, never under a list's box.)
    local listed = {}
    for _, span in ipairs(screen:pictures(c.knobs, touched(), slot and slot.name, own and true or false, true)) do
      if span.first > 4 and span.kind ~= "choices" then right_free = false end
      if span.first <= 4 and span.kind ~= "dial" and span.kind ~= "choices" then left_free = false end
      if span.kind ~= "dial" then
        for i = span.first, span.first + span.cells - 1 do pictured[i] = true end
      end
      if span.kind == "choices" then
        for i = span.first, span.first + span.cells - 1 do listed[i] = true end
      end
    end
    -- A theme's showcase: over a left screen of only dials, a hosted plug-in's own picture as
    -- it is, at its own size (shrunk, or in the theme's colours, it smudges).
    -- (At its own size at the left margin, between the strip and the section names;
    -- centred under the bar otherwise.)
    -- (Not DECKARD's own devices: the strip already shows their picture whole.)
    if theme.plugin_showcase and left_free and slot and slot.plugin and slot.picture then
      local w, h = screen:image_size(slot.picture, true)
      if theme.knobs_left and w then
        -- (At its own size, or shrunk to fit, centred in its band: in the widest run of
        -- cells with no list, and only if it keeps half its size there.)
        local first, cells = free_run(listed, 1, 4)
        local k = cells > 0 and math.min(1, 80 / h, (cells * 120 - 22) / w) or 0
        if k >= 0.5 then
          local pw, ph = math.floor(w * k + 0.5), math.floor(h * k + 0.5)
          screen:image((first - 1) * 120 + 11, 78 + (80 - ph) // 2, pw, ph, slot.picture, true, true)
        end
      elseif not (listed[1] or listed[2] or listed[3] or listed[4]) then
        -- (The whole screen's width: only with no list on it.)
        local y = theme.info_in_tabs and 86 or widgets.BAND_Y - 6
        screen:image(0, y, 480, 80, slot.picture, true, true)
      end
    end
    local pips_end = theme.knobs_centred and #c.pages > 1 and page_pips(screen, theme, c) or 494
    if in_tabs then widgets.info_on_row(screen, theme, info, 36, 8, pips_end + 16) end
    if slot then
      -- (Not in a theme whose bar shows the page: the strip names the plug-in.)
      if not theme.knobs_centred then
        screen:label(480 + 21, 36, 480 - 21 - 18, 24, slot.name, "default", "right", "center")
      end
      if c.tuner then
        -- The Tuner effect: A4's pitch (and Muted) on the bar, the dial under it.
        local t = c.tuner
        screen:label(490, 36, 300, 24, t.reference .. (t.muted and "  ·  Muted" or ""), "default", "left", "center")
        zhora.dial(screen, t)
      elseif not right_free then
        -- (The band holds the knobs' pictures.)
      elseif c.eq then eq_curve(screen, theme, c.eq)
      elseif c.filter then eq_curve(screen, theme, c.filter)
      elseif c.picture then eq_curve(screen, theme, c.picture)
      else
        -- (Over two cells with no list: knobs 5 and 6, else 7 and 8, else not at all.)
        local at = not (listed[5] or listed[6]) and 480 or not (listed[7] or listed[8]) and 720 or nil
        if at then chain.thumbnail(screen, theme, at, 68, 240, 136, slot, false) end
        if c.scope then scope_view(screen, c.scope) end
      end
    end
    screen:pictures(c.knobs, touched(), slot and slot.name, own and true or false)
    -- (The band below always has pictures: the chain is the short strip.)
    chain.draw(screen, theme, c, true)
  else
    widgets.buttons(screen, theme, {
      [5] = { arrows = true, text = PAGE_GROUPS[c.page_group] or "", left = can_step(c, false), right = can_step(c, true) },
    })
    channel_tiles(screen, c)
    screen:rect(480, 35, 480, 25, "bar")
    if c.euclid then
      screen:label(490, 36, 300, 24, "Euclidean", "default", "left", "center")
      euclid_circle(screen, theme, c.euclid)
    elseif c.random then
      local r = c.random
      screen:label(490, 36, 200, 24, "Random", "default", "left", "center")
      local made = #r.notes .. (#r.notes == 1 and " note" or " notes")
      screen:label(700, 36, 250, 24, string.format("%.0f %%  ·  %s", r.probability, made), "default", "right", "center")
      random_view(screen, theme, r)
    else
      channel_icon(screen, c.page_group, 720, widgets.BAND_Y + widgets.BAND_H // 2, 37, theme.colors.channel_icon or { 0x3a, 0x3a, 0x3a }, "background")
    end
  end
  knobs(screen, theme, c, false, knob_of, pictured)
end

-- The knobs' row as the page draws it (Set Macro shows it too).
control.knob_row = knobs

return control
