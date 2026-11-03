-- The step grid (SHIFT + STEP): the eight buttons above the screens are eight steps,
-- and the pads choose what they place. PAD MODE, KEYBOARD and CHORDS change that here;
-- SHIFT + STEP goes back.
--
--   Pad mode          a pad's hit focuses its sound; the buttons place it (its base key)
--   Keyboard          the screen shows the piano roll with the scale; a pad picks a note,
--                     pads held together several; the buttons place those
--   Chords            as Keyboard, a pad picking its chord
--   display 1-8       let go, a step's notes in or out; held a moment, or with a knob
--                     turned, placed if they weren't and selected, for knobs 1-4
--   LEFT / RIGHT      the eight steps before / after
--   knobs 1-4         POSITION, PITCH, VELOCITY (FIXEDVELOCITY: the fixed velocity),
--                     LENGTH of the selected notes
--   knobs 5-8         Keyboard: ROOT, BANK, TYPE, the pitches shown; Chords: ROOT, TYPE,
--                     CHORD, the pitches shown
--   4-D encoder       as the Events page's (lib/notes.lua)
--
-- A step placed is a step long, at the fixed velocity or 100.

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local pads = require("lib.pads")
local util = require("lib.util")
local editor = require("lib.pattern_editor")
local notes = require("lib.notes")

local page = {}

local STEPS = 8
local HOLD = 0.4
local NEW_STEP_VELOCITY = 100
local LIST_W = 20
local KEY_ROWS, KEY_ROW = 16, 8
local LANES_H = 145

-- The colours of the rows and keys: out of the scale (red, as the software's piano
-- roll), and picked.
local ROW_OUT, ROW_PICKED = { 58, 22, 22 }, { 96, 96, 96 }
local KEY_OUT, KEY_PICKED = { 130, 50, 50 }, { 255, 255, 255 }

-- The eight steps shown (in eights from the first), the buttons down (the step's tick,
-- when, and whether held long enough), the pads down, what's picked, the highest pitch
-- shown and the pitches to bring into view.
local offset = 0
local pressed = {}
local down = {}
local picked = nil
local top_pitch = nil
local to_show = nil

local function step(s)
  return math.max(s.grid.step_ticks or 240, 1)
end

local function first_step(s)
  local st = step(s)
  return -(-s.pattern_start // st) * st
end

local function range_width(s)
  return s.pattern_length - (first_step(s) - s.pattern_start)
end

local function last_offset(s)
  return math.max(0, -(-range_width(s) // (step(s) * STEPS)) - 1)
end

local function sync(s)
  offset = util.clamp(offset, 0, last_offset(s))
end

-- Button `n`'s step's tick, or none past the pattern's end.
local function step_at(s, n)
  local t = (offset * STEPS + n - 1) * step(s)
  if t >= range_width(s) then return nil end
  return first_step(s) + t
end

-- What the chosen scale and chords are now: what's picked goes with them.
local function pick_key()
  local scale = deckard.state().scale
  local _, _, key = pads.chord_list()
  return table.concat({
    pads.mode, pads.base, scale.bank, scale.kind, pads.chord_mode, pads.chord_types[key], pads.chord_position,
  }, ":")
end

local function pad_pitches(n)
  if pads.mode == "chords" then return pads.chord_at(n) end
  local note = pads.notes()[n]
  return note < 128 and { note } or {}
end

-- What the buttons place in Keyboard and Chords mode (pad 1's until a pad's picked).
local function pick()
  local key = pick_key()
  if not picked or picked.key ~= key then
    picked = { key = key, pads = { [1] = true }, pitches = pad_pitches(1) }
    to_show = picked.pitches
  end
  return picked
end

-- Whether the step at tick `at` has the focused sound's picked notes all (in Pad mode,
-- any of its notes).
local function placed(s, at)
  local wanted = {}
  if pads.keys() then
    local list = pick().pitches
    if #list == 0 then return false end
    for _, p in ipairs(list) do wanted[p] = true end
  end
  local any = false
  for _, n in ipairs(s.notes) do
    if n.sound == s.sound and n.position >= at and n.position < at + step(s) then
      any = true
      wanted[n.pitch] = nil
    end
  end
  return pads.keys() and next(wanted) == nil or (not pads.keys() and any)
end

local function place(s, at)
  local velocity = s.fixed_velocity and s.velocity or NEW_STEP_VELOCITY
  if pads.keys() then
    deckard.step_notes(at, step(s), velocity, pick().pitches)
  else
    deckard.step(at, step(s), velocity)
  end
end

-- A button held: its step placed if it wasn't, and selected.
local function hold(b)
  if b.held then return end
  b.held = true
  local s = deckard.state()
  if pads.keys() and #pick().pitches == 0 then return end
  if not placed(s, b.at) then place(s, b.at) end
  local low, high
  if pads.keys() then
    low, high = math.min(table.unpack(pick().pitches)), math.max(table.unpack(pick().pitches))
  end
  deckard.select_events(b.at, b.at + step(s) - 1, low, high)
end

local function hold_pressed()
  for _, b in pairs(pressed) do hold(b) end
end

function page.shown()
  pressed, down = {}, {}
end

function page.tick()
  local now = deckard.now()
  for _, b in pairs(pressed) do
    if not b.held and now - b.since >= HOLD then hold(b) end
  end
end

function page.button(name, is_down)
  local tapped = knobs.double_tap(name, is_down)
  if tapped and not (tapped == 3 and deckard.state().fixed_velocity) then notes.reset_knob(tapped, 1) end
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if n then
    local s = deckard.state()
    if is_down then
      sync(s)
      local at = not deckard.held("SHIFT") and step_at(s, n)
      if at then pressed[n] = { at = at, since = deckard.now(), held = false } end
    elseif pressed[n] then
      if not pressed[n].held then place(s, pressed[n].at) end
      pressed[n] = nil
    end
    return true
  end
  if name == "LEFT" or name == "RIGHT" then
    if is_down then
      local s = deckard.state()
      sync(s)
      offset = util.clamp(offset + (name == "LEFT" and -1 or 1), 0, last_offset(s))
    end
    return true
  end
  return notes.wheel_button(name, is_down)
end

page.wheel = notes.wheel

-- Keep `v` within lo..hi, stepped by `steps`; `set` takes a changed value.
local function change(v, steps, lo, hi, set)
  local to = util.clamp(v + steps, lo, hi)
  if to ~= v then set(to) end
end

local function pitch_top(s)
  return top_pitch or editor.pitch_top(s, KEY_ROWS)
end

local function keys_knob(s, n, amount)
  local steps = knobs.gather(n, amount, n == 8 and 0.05 or nil)
  if steps == 0 then return end
  local sc, scale = deckard.scales, s.scale
  local bank_knob, type_knob = 6, 7
  if pads.mode == "chords" then bank_knob, type_knob = nil, 6 end
  if n == 5 then
    pads.move(steps)
  elseif n == 8 then
    top_pitch = util.clamp(pitch_top(s) + steps, KEY_ROWS - 1, 127)
  elseif pads.mode == "chords" and n == 7 then
    local list, _, key = pads.chord_list()
    change(pads.chord_types[key], steps, 1, #list, function(t) pads.chord_types[key] = t end)
  elseif n == bank_knob then
    change(scale.bank, steps, 1, #sc.banks, function(b) pads.scale(b, 1) end)
  elseif n == type_knob and not (pads.mode == "chords" and pads.chord_mode ~= 1) then
    change(scale.kind, steps, 1, #sc.names[scale.bank], function(k) pads.scale(scale.bank, k) end)
  end
end

function page.knob(n, amount)
  local s = deckard.state()
  if n <= 4 then
    if n == 3 and s.fixed_velocity then
      local steps = knobs.each(amount)
      if steps ~= 0 then deckard.velocity(util.clamp(s.velocity + steps, 0, 127)) end
    else
      -- (A button down: its step is what's edited.)
      hold_pressed()
      local steps = knobs.gather(n, amount, 0.05)
      if steps ~= 0 then notes.edit_knob(n, steps) end
    end
  elseif pads.keys() then
    keys_knob(s, n, amount)
  end
  return true
end

-- A pad's hit in Keyboard and Chords mode picks: alone, its notes; with others down,
-- its notes too. (Then global.lua's: in Pad mode it focuses its sound. It plays as ever.)
function page.pad(n)
  if not pads.keys() or deckard.held("ERASE") then return end
  local list = pad_pitches(n)
  if #list == 0 then return end
  local key = pick_key()
  if next(down) == nil or not picked or picked.key ~= key then
    picked = { key = key, pads = {}, pitches = {} }
  end
  down[n] = true
  picked.pads[n] = true
  local have = {}
  for _, p in ipairs(picked.pitches) do have[p] = true end
  for _, p in ipairs(list) do
    if not have[p] then
      have[p] = true
      picked.pitches[#picked.pitches + 1] = p
    end
  end
  table.sort(picked.pitches)
  to_show = picked.pitches
end

function page.pad_off(n)
  down[n] = nil
end

function page.leds()
  local s = deckard.state()
  sync(s)
  for i = 1, STEPS do
    local at = step_at(s, i)
    deckard.led("SCREEN_" .. i, at and (placed(s, at) and "on" or "dim") or "off")
  end
  deckard.led("LEFT", offset > 0 and "dim" or "off")
  deckard.led("RIGHT", offset < last_offset(s) and "dim" or "off")
  notes.wheel_leds(s)
  -- What's picked, white over global.lua's pads.
  if pads.keys() then
    for i in pairs(pick().pads) do deckard.led("PAD_" .. i, 16, true) end
  end
end

-- What's picked, as names.
local function pick_names()
  local names = {}
  for _, p in ipairs(pick().pitches) do names[#names + 1] = pads.note_name(p) end
  return table.concat(names, " ")
end

-- Whether the scale says anything about these pads: a chord set's or the user chords
-- aren't bound to it.
local function scale_shown()
  return pads.mode ~= "chords" or pads.chord_mode == 1
end

local function is_picked(p)
  for _, q in ipairs(pick().pitches) do
    if q == p then return true end
  end
  return false
end

-- The knobs 5-8.
local function keys_params(s, high)
  local sc, scale = deckard.scales, s.scale
  local root = { name = "ROOT", value = deckard.note_name(pads.base, pads.base) }
  local bank = { name = "BANK", value = sc.banks[scale.bank] }
  local kind = { name = "TYPE", value = sc.short[scale.bank][scale.kind] }
  local view = { name = "PITCHES", value = pads.note_name(high) }
  if pads.mode == "chords" then
    local _, short, key = pads.chord_list()
    return { root, scale_shown() and kind or nil, { name = "CHORD", value = short[pads.chord_types[key]] }, view }
  end
  return { root, bank, kind, view }
end

-- The pitches picked brought into view, once.
local function show_picked(s)
  if not to_show or #to_show == 0 then
    to_show = nil
    return
  end
  local low, high = math.min(table.unpack(to_show)), math.max(table.unpack(to_show))
  local top = pitch_top(s)
  if high > top then top = high end
  if low < top - KEY_ROWS + 1 then top = low + KEY_ROWS - 1 end
  top_pitch = util.clamp(top, KEY_ROWS - 1, 127)
  to_show = nil
end

function page.draw(screen, theme)
  local s = deckard.state()
  sync(s)
  local keyboard = pads.keys()
  local specs = {}
  for i = 1, STEPS do
    local at = step_at(s, i)
    if at then specs[i] = { text = tostring(offset * STEPS + i), selected = placed(s, at) } end
  end
  widgets.buttons(screen, theme, specs)
  widgets.info_bar(screen, theme, widgets.focus_info(s))

  -- The left screen: the whole pattern, the eight steps shown framed.
  local a, b = notes.span(s)
  local from = first_step(s) + offset * STEPS * step(s)
  local to = from + STEPS * step(s)
  editor.draw(screen, theme, s, 0, 60, 480, LANES_H, false)
  local function x_of(t) return math.floor((t - a) / math.max(1, b - a) * 480 + 0.5) end
  local x0, x1 = x_of(from), math.min(x_of(to), 480)
  screen:frame(x0, 60 + editor.HEADER, math.max(1, x1 - x0), LANES_H - editor.HEADER, "text")
  local p = notes.edit_params(s)
  if s.fixed_velocity then p[3] = { name = "FIXEDVELOCITY", value = tostring(s.velocity) } end
  widgets.params(screen, theme, 0, {}, nil, p)

  -- The right screen: the pattern's name and what's picked, and the eight steps.
  local group = s.groups[s.group]
  local pattern = group and group.patterns and group.patterns[s.pattern]
  local title = { font = theme.styles.default.font, size = 14,
    color = pattern and widgets.colour(theme, pattern.palette, false, true) or "text" }
  screen:label(481, 35, 230, 24, pattern and pattern.name or "NO PATTERN", title, "left", "center")
  local y = 60
  if keyboard then
    screen:label(712, 35, 240, 24, pick_names(), { font = theme.styles.default.font, size = 14, color = "text" },
      "right", "center")
    show_picked(s)
    local high = pitch_top(s)
    local bound = scale_shown()
    editor.lanes(screen, theme, s, 480 + LIST_W, y, 480 - LIST_W, LANES_H, {
      from = from, to = to, rows = KEY_ROWS, row = KEY_ROW, high = high,
      row_of = function(n) return n.sound == s.sound and high - n.pitch + 1 or nil end,
      tint = function(pitch)
        if is_picked(pitch) then return ROW_PICKED end
        if bound and not pads.in_scale(pitch) then return ROW_OUT end
      end,
    })
    editor.keys(screen, theme, 480, y + editor.HEADER, LIST_W, KEY_ROW, KEY_ROWS, high, pads.note_name, function(pitch)
      if is_picked(pitch) then return KEY_PICKED end
      if bound and not pads.in_scale(pitch) then return KEY_OUT end
    end)
    widgets.params(screen, theme, 480, {}, nil, keys_params(s, high))
  else
    editor.draw(screen, theme, s, 480 + LIST_W, y, 480 - LIST_W, 272 - y, true, from, to, false)
    local snd = group and group.sounds[s.sound]
    if snd then
      screen:rect(480, y + editor.HEADER, LIST_W, editor.LANE, widgets.colour(theme, snd.palette))
      screen:label(482, y + editor.HEADER, LIST_W - 2, editor.LANE, tostring(s.sound),
        { font = theme.styles.default.font, size = 11, color = { 0, 0, 0 } }, "left", "center")
    end
  end
end

return page
