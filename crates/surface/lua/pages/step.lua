-- Step mode's page (STEP): the pads are 16 steps of the focused sound (Maschine's
-- `StepPageMK3`).
--
--   display 1         STEP MODE
--   display 3         DOUBLE the pattern
--   display 4         KEYBOARD: Keyboard mode (the right screen by pitch); the pads stay
--                     steps
--   display 5         FOLLOW: the steps shown follow the playhead
--   display 7         PROBABILITY: the stems show the notes' chances, not velocities
--   display 8         PROB LOCK: the focused sound's notes' chances rolled once and kept
--   pads              let go, a step without a note gets one (the sound's base key, at
--                     the pad's velocity or the fixed one), a step with notes loses them;
--                     held a moment, the Step Mod page (pages/step_mod.lua) for the steps
--                     held
--   knobs 1-4         POSITION, PITCH, VELOCITY (FIXEDVELOCITY: the fixed velocity),
--                     LENGTH of the selected notes; page 2 PROBABILITY
--   knob 6            the next or previous 16 steps; knob 8 Keyboard mode's pitches
--   4-D encoder       as the Events page's (lib/notes.lua)

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local pads = require("lib.pads")
local util = require("lib.util")
local editor = require("lib.pattern_editor")
local notes = require("lib.notes")

local page = {}

-- The 16 steps shown (from 0), following the playhead, the knob page, probability
-- shown, Keyboard mode's highest pitch shown, and the pads held (their velocities).
local segment = 0
local follow = false
local knob_page = 1
local probability = false
local top_pitch = nil
local held = {}

-- Held this long (seconds), a pad shows the Step Mod page (Maschine's 8 timer ticks);
-- when each pad held went down; whether the Step Mod page is up.
local MOD_HOLD = 0.4
local held_at = {}
local modding = false

-- Keyboard mode's pitch lanes on the right screen.
local KEY_ROWS, KEY_ROW = 24, 8

-- A step's length: the Step Grid's step (snap on or not, as Maschine's).
local function step(s)
  return math.max(s.grid.step_ticks or 240, 1)
end

-- The first step's tick (the first on the grid in the pattern) and how far steps go.
local function first_step(s)
  local st = step(s)
  return -(-s.pattern_start // st) * st
end

local function range_width(s)
  return s.pattern_length - (first_step(s) - s.pattern_start)
end

local function segments(s)
  return math.max(1, -(-(s.pattern_length // step(s)) // 16))
end

-- Step `pad`'s time from the first step.
local function step_time(s, pad)
  return (pad - 1 + 16 * segment) * step(s)
end

-- The segment shown: kept in the pattern, and with FOLLOW on where the playhead is.
local function sync(s)
  if follow and s.playing and s.pattern_position then
    segment = (s.pattern_position - s.pattern_start) // (step(s) * 16)
  end
  segment = util.clamp(segment, 0, segments(s) - 1)
end

-- Whether step `pad` has a note of the focused sound starting in it.
local function has_note(s, pad)
  local t = first_step(s) + step_time(s, pad)
  for _, n in ipairs(s.notes) do
    if n.sound == s.sound and n.position >= t and n.position < t + step(s) then return true end
  end
  return false
end

function page.following()
  return follow
end

function page.shown()
  knob_page, probability, held, held_at, modding = 1, false, {}, {}, false
end

function page.button(name, down)
  local tapped = knobs.double_tap(name, down)
  if tapped and not (knob_page == 1 and tapped == 3 and deckard.state().fixed_velocity) then
    notes.reset_knob(tapped, knob_page)
  end
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if n then
    if not down then return true end
    if n == 3 then
      deckard.pattern_edit("double")
    elseif n == 4 then
      pads.toggle_step_keyboard()
    elseif n == 5 then
      follow = not follow
    elseif n == 7 and not pads.keyboard() then
      probability = not probability
    elseif n == 8 then
      notes.toggle_prob_lock(deckard.state())
    end
    return true
  end
  if name == "LEFT" or name == "RIGHT" then
    if down then knob_page = name == "LEFT" and 1 or 2 end
    return true
  end
  return notes.wheel_button(name, down)
end

page.wheel = notes.wheel

local function pitch_top(s)
  return top_pitch or editor.pitch_top(s, KEY_ROWS)
end

function page.knob(n, amount)
  local s = deckard.state()
  if n == 6 then
    -- (Moving by hand ends FOLLOW while the pattern plays.)
    local steps = knobs.gather(n, amount)
    if steps ~= 0 then
      if follow and s.playing then follow = false end
      sync(s)
      segment = util.clamp(segment + steps, 0, segments(s) - 1)
    end
  elseif n == 8 then
    local steps = knobs.gather(n, amount, 0.05)
    if pads.keyboard() and steps ~= 0 then top_pitch = util.clamp(pitch_top(s) + steps, KEY_ROWS - 1, 127) end
  elseif n <= 4 then
    if knob_page == 2 then
      local steps = knobs.gather(n, amount, 0.05)
      if n == 1 and steps ~= 0 then deckard.events("probability", steps) end
    elseif n == 3 and s.fixed_velocity then
      local steps = knobs.each(amount)
      if steps ~= 0 then deckard.velocity(util.clamp(s.velocity + steps, 0, 127)) end
    else
      local steps = knobs.gather(n, amount, 0.05)
      if steps ~= 0 then notes.edit_knob(n, steps) end
    end
  end
  return true
end

-- The Step Mod page's values: the highest step held's (as Maschine's), or none.
local function show_mod_step(s)
  local last = nil
  for n in pairs(held) do
    if not last or n > last then last = n end
  end
  if last then
    deckard.step_mod_at(first_step(s) + step_time(s, last), step(s))
  else
    deckard.step_mod_at()
  end
end

-- A pad held long enough: the Step Mod page.
function page.tick()
  if modding then return end
  local s = deckard.state()
  for n, t in pairs(held_at) do
    sync(s)
    if deckard.now() - t >= MOD_HOLD and step_time(s, n) < range_width(s) then
      modding = true
      show_mod_step(s)
      deckard.show_page("step_mod")
      return true
    end
  end
end

-- The Step Mod page's knob `n`: the steps held's modulation.
function page.mod_knob(n, amount)
  local s = deckard.state()
  sync(s)
  local pads = {}
  for p in pairs(held) do pads[#pads + 1] = p end
  table.sort(pads)
  deckard.step_mod(first_step(s) + 16 * segment * step(s), step(s), pads, n, amount)
end

function page.pad(n, velocity)
  if deckard.held("SHIFT") then return true end
  held[n] = velocity
  held_at[n] = deckard.now()
  if modding then show_mod_step(deckard.state()) end
  return true
end

function page.pad_off(n)
  local s = deckard.state()
  local v = held[n]
  held[n], held_at[n] = nil, nil
  -- (Let go on the Step Mod page: no step; the last let go, back to the Step page.)
  if modding then
    show_mod_step(s)
    if next(held) == nil then
      modding = false
      deckard.show_page("step")
    end
    return true
  end
  if not v or deckard.held("SHIFT") then return true end
  sync(s)
  local t = step_time(s, n)
  if t >= range_width(s) then return true end
  local vel = s.fixed_velocity and s.velocity or util.clamp(math.floor(v * 128), 0, 127)
  deckard.step(first_step(s) + t, step(s), vel)
  return true
end

function page.leds()
  local s = deckard.state()
  sync(s)
  deckard.led("SCREEN_1", "on")
  deckard.led("SCREEN_2", "off")
  deckard.led("SCREEN_3", "dim")
  deckard.led("SCREEN_4", pads.keyboard() and "on" or "dim")
  deckard.led("SCREEN_5", follow and "on" or "dim")
  deckard.led("SCREEN_6", "off")
  deckard.led("SCREEN_7", pads.keyboard() and "off" or (probability and "on" or "dim"))
  deckard.led("SCREEN_8", notes.prob_locked(s) and "on" or "dim")
  deckard.led("LEFT", knob_page == 2 and "dim" or "off")
  deckard.led("RIGHT", knob_page == 1 and "dim" or "off")
  notes.wheel_leds(s)
  page.pad_leds(s)
end

-- The steps with notes in the sound's colour; the playing step white.
function page.pad_leds(s)
  sync(s)
  local group = s.groups[s.group]
  local snd = group and group.sounds[s.sound]
  local playing = nil
  if s.pattern_position then
    playing = (s.pattern_position - first_step(s)) // step(s) - segment * 16 + 1
  end
  for i = 1, 16 do
    if i == playing then
      deckard.led("PAD_" .. i, 16, true)
    elseif snd and step_time(s, i) < range_width(s) and has_note(s, i) then
      deckard.led("PAD_" .. i, snd.palette, true)
    else
      deckard.led("PAD_" .. i, "off")
    end
  end
end

local LIST_W = 20

function page.draw(screen, theme)
  local s = deckard.state()
  sync(s)
  local keyboard = pads.keyboard()
  widgets.buttons(screen, theme, {
    [1] = { text = "STEP MODE", selected = true },
    [3] = { text = "DOUBLE" },
    [4] = { text = "KEYBOARD", selected = keyboard },
    [5] = { text = "FOLLOW", selected = follow },
    [7] = not keyboard and { text = "PROBABILITY", selected = probability } or nil,
    [8] = { text = "PROB LOCK", selected = notes.prob_locked(s) },
  })
  widgets.info_bar(screen, theme, widgets.focus_info(s))

  -- The left screen: the whole pattern, the 16 steps shown framed.
  local a, b = notes.span(s)
  local from = first_step(s) + segment * 16 * step(s)
  local to = from + 16 * step(s)
  editor.draw(screen, theme, s, 0, 60, 480, 145, false)
  local function x_of(t) return math.floor((t - a) / math.max(1, b - a) * 480 + 0.5) end
  local x0, x1 = x_of(from), math.min(x_of(to), 480)
  screen:frame(x0, 60 + editor.HEADER, math.max(1, x1 - x0), 145 - editor.HEADER, "text")
  local p
  if knob_page == 2 then
    p = { notes.probability_param(s) }
  else
    p = notes.edit_params(s)
    if s.fixed_velocity then p[3] = { name = "FIXEDVELOCITY", value = tostring(s.velocity) } end
  end
  widgets.params(screen, theme, 0, {}, nil, p)

  -- The right screen: the pattern's name, and the 16 steps.
  local group = s.groups[s.group]
  local pattern = group and group.patterns and group.patterns[s.pattern]
  local snd = group and group.sounds[s.sound]
  local st = { font = theme.styles.default.font, size = 14,
    color = pattern and widgets.colour(theme, pattern.palette, false, true) or "text" }
  screen:label(481, 35, 470, 24, pattern and pattern.name or "NO PATTERN", st, "left", "center")
  local y = 60
  local top = y + editor.HEADER
  if keyboard then
    local high = pitch_top(s)
    editor.lanes(screen, theme, s, 480 + LIST_W, y, 480 - LIST_W, 272 - y, {
      from = from, to = to, rows = KEY_ROWS, row = KEY_ROW, high = high,
      row_of = function(n) return n.sound == s.sound and high - n.pitch + 1 or nil end,
    })
    editor.keys(screen, theme, 480, top, LIST_W, KEY_ROW, KEY_ROWS, high, pads.note_name)
  else
    editor.draw(screen, theme, s, 480 + LIST_W, y, 480 - LIST_W, 272 - y, true, from, to, probability)
    -- The focused sound's number, and the stems' scale.
    if snd then
      screen:rect(480, top, LIST_W, editor.LANE, widgets.colour(theme, snd.palette))
      screen:label(482, top, LIST_W - 2, editor.LANE, tostring(s.sound),
        { font = theme.styles.default.font, size = 11, color = { 0, 0, 0 } }, "left", "center")
    end
    local small = { font = theme.styles.default.font, size = 9, color = { 150, 150, 150 } }
    local marks = probability and { 100, 75, 50, 25, 0 } or { 127, 96, 64, 32, 0 }
    local full = probability and 100 or 127
    for _, m in ipairs(marks) do
      local my = editor.value_y(y, 272 - y, m / full)
      screen:label(480, my - 6, LIST_W - 2, 12, tostring(m), small, "right", "center")
    end
  end
end

return page
