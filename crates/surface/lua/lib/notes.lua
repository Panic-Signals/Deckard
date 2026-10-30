-- The focused pattern's notes as the Events and Step pages work on them: what's in
-- the pattern, what's selected, and the knobs' values for the selection.

local util = require("lib.util")
local pads = require("lib.pads")

local notes = {}

-- The finest step notes move and grow by (Maschine's smallest snap).
notes.MIN_SNAP = 960 // 32

-- Where the pattern plays: its first tick and the tick after its last.
-- PROB LOCK (the Events and Step pages' display 8): whether the focused sound has it,
-- and switching it.
function notes.prob_locked(s)
  local group = s.groups[s.group]
  local snd = group and group.sounds[s.sound]
  return snd ~= nil and snd.prob_locked == true
end

function notes.toggle_prob_lock(s)
  deckard.prob_lock(not notes.prob_locked(s))
end

function notes.span(s)
  return s.pattern_start, s.pattern_start + math.max(1, s.pattern_length)
end

-- The notes in the pattern's span (the focused sound's only, `own`).
function notes.shown(s, own)
  local a, b = notes.span(s)
  local out = {}
  for _, n in ipairs(s.notes) do
    if n.position >= a and n.position < b and (not own or n.sound == s.sound) then out[#out + 1] = n end
  end
  return out
end

function notes.selected(s)
  local out = {}
  for _, n in ipairs(notes.shown(s, false)) do
    if n.selected then out[#out + 1] = n end
  end
  return out
end

-- Whether there are notes to work on: the focused sound's in Keyboard mode.
function notes.any(s)
  return #notes.shown(s, pads.keyboard()) > 0
end

-- What the selected notes' `field` shows: "-" none, "*" when they differ.
function notes.value(s, field, text)
  local sel = notes.selected(s)
  if #sel == 0 then return "-" end
  local v = sel[1][field]
  for _, n in ipairs(sel) do
    if n[field] ~= v then return "*" end
  end
  return text(v)
end

-- The edit knobs (POSITION, PITCH, VELOCITY, LENGTH), and page 2's PROBABILITY.
function notes.edit_params(s)
  return {
    { name = "POSITION", value = notes.value(s, "position", function(t) return util.bar_beat(t, s.numerator, s.denominator) end) },
    { name = "PITCH", value = notes.value(s, "pitch", pads.note_name) },
    { name = "VELOCITY", value = notes.value(s, "velocity", tostring) },
    { name = "LENGTH", value = notes.value(s, "length", tostring) },
  }
end

function notes.probability_param(s)
  return { name = "PROBABILITY", value = notes.value(s, "probability", function(v) return v .. "%" end) }
end

-- An edit knob turned `steps`: 1-4 POSITION, PITCH, VELOCITY, LENGTH (SHIFT finer).
function notes.edit_knob(n, steps)
  local fine = deckard.held("SHIFT")
  if n == 1 then
    deckard.events(fine and "nudge_fine" or "nudge", steps)
  elseif n == 2 then
    deckard.events("pitch", steps)
  elseif n == 3 then
    deckard.events("velocity", steps)
  elseif n == 4 then
    deckard.events(fine and "length_fine" or "length", steps)
  end
end

-- Knob `n` double-tapped on knob page `knob_page` (the Events and Step pages'): the
-- selected notes' VELOCITY or PROBABILITY back to 100.
function notes.reset_knob(n, knob_page)
  if knob_page == 2 and n == 1 then
    deckard.events("probability_to", 100)
  elseif knob_page == 1 and n == 3 then
    deckard.events("velocity_to", 100)
  end
end

-- The 4-D encoder on the Events and Step pages (Maschine's `Events4DWheel`): turned,
-- the next or previous note (pressed: its length); left / right nudge (SHIFT finer);
-- up / down the sound before or after (Keyboard mode: transpose, SHIFT by octaves).
function notes.wheel(steps)
  if deckard.held("SHIFT") then return end
  if deckard.held("WHEEL") then
    deckard.events("length", steps)
  else
    deckard.events("select_step", steps)
  end
  return true
end

function notes.wheel_button(name, down)
  local s = deckard.state()
  local shift = deckard.held("SHIFT")
  if name == "WHEEL_LEFT" or name == "WHEEL_RIGHT" then
    if down then deckard.events(shift and "nudge_shown_fine" or "nudge_shown", name == "WHEEL_RIGHT" and 1 or -1) end
    return true
  end
  if name == "WHEEL_UP" or name == "WHEEL_DOWN" then
    if not down then return true end
    local up = name == "WHEEL_UP"
    if pads.keyboard() then
      local by = shift and 12 or 1
      deckard.events("transpose", up and by or -by)
    else
      local group = s.groups[s.group]
      local to = s.sound + (up and -1 or 1)
      if group and to >= 1 and to <= #group.sounds then deckard.focus_sound(to) end
    end
    return true
  end
end

-- The 4-D encoder's arrows: left / right while there are notes to nudge; up / down
-- for the other sounds, or in Keyboard mode while there are notes to transpose.
function notes.wheel_leds(s)
  local any = notes.any(s)
  deckard.led("WHEEL_LEFT", any and 16 or "off", false)
  deckard.led("WHEEL_RIGHT", any and 16 or "off", false)
  local updown = any or not pads.keyboard()
  deckard.led("WHEEL_UP", updown and 16 or "off", false)
  deckard.led("WHEEL_DOWN", updown and 16 or "off", false)
end

return notes
