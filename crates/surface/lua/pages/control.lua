-- The Control page (CHANNEL / PLUG-IN): the eight knobs turn the focused chain's
-- plug-in page, or its channel properties.
--
--   display 1-3       MASTER, GROUP, SOUND: the chain shown
--   display 5 / 6     PLUG-IN: the previous / next slot, on to the "+" after the last
--                     (an empty one); CHANNEL: the previous / next page group (Input,
--                     Output, Groove, Macro). The jog wheel's left / right the same.
--   jog wheel press   PLUG-IN: the plug-in browser (pages/module.lua) for the slot
--   LEFT / RIGHT      the previous / next page
--   right screen      the focused plug-in's picture; the built-in EQ's or Filter's curve, on both pages
--
-- With SHIFT held on PLUG-IN, the focused slot's edits: EDIT (a plug-in's window),
-- INSERT (the plug-in browser for a new slot after the last), SAVE (not yet), MOVE
-- (5 / 6: before or after its neighbour; an instrument stays first), BYPASS, REMOVE.

local widgets = require("lib.widgets")
local chain = require("lib.chain")

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
    if n <= 3 then
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
  for i = 1, 3 do deckard.led("SCREEN_" .. i, i == c.level and "on" or "dim") end
  if can_step(c, false) then
    deckard.led("SCREEN_5", "dim")
    deckard.led("WHEEL_LEFT", 16, false)
  end
  if can_step(c, true) then
    deckard.led("SCREEN_6", "dim")
    deckard.led("WHEEL_RIGHT", 16, false)
  end
  if c.plugins and c.slot_index then deckard.led("SCREEN_8", "dim") end
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

function control.draw(screen, theme, opts)
  opts = opts or {}
  local s = deckard.state()
  local knob_of = opts.knob_of or (s.auto_write and auto_knob or nil)
  local c = s.control
  local group = s.groups[s.group]
  local snd = group and group.sounds[s.sound]
  local shift_layer = c.plugins and deckard.held("SHIFT")
  if not shift_layer then chain.tabs(screen, theme, c.level) end
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
  elseif c.plugins then
    local slot = c.chain[slot_at(c)]
    widgets.buttons(screen, theme, {
      [5] = { arrows = true, text = slot and slot.name:upper() or "EMPTY", left = can_step(c, false), right = can_step(c, true) },
    })
  end
  if c.plugins then
    local slot = c.chain[slot_at(c)]
    -- Browse.
    if not shift_layer and not opts.no_browse then
      screen:frame(841, 4, 115, 24, "button", theme.corner)
      screen:circle(902, 13.5, 5.6, { 0xee, 0xee, 0xee }, 1.6)
      screen:line(897.5, 18, 892, 23.5, 1.8, { 0xee, 0xee, 0xee })
    end
    screen:rect(480, 35, 480, 25, "bar")
    if slot then
      screen:label(480 + 21, 36, 480 - 21 - 18, 24, slot.name, "default", "right", "center")
      if c.eq then eq_curve(screen, c.eq)
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
    channel_icon(screen, c.page_group, 720, 126, 37, { 0x3a, 0x3a, 0x3a }, "background")
  end
  knobs(screen, theme, c, false, knob_of)
end

-- The knobs' row as the page draws it (Set Macro shows it too).
control.knob_row = knobs

return control
