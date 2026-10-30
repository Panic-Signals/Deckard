-- The Smart Strip: PITCH, MOD, PERFORM or NOTES above it pick what it does (pressed
-- again, none), as Maschine's `TouchstripControllerMK3` and its native strip.
--
--   PITCH     bends the focused sound, springing back to the middle once let go
--   MOD       the focused sound's mod wheel (CC 1), staying where it's let go
--   PERFORM   the focused group's Perform FX while touched (SHIFT + PERFORM shows it,
--             one loaded if it has none)
--   NOTES     strums the pads held (or all 16) along the strip, a finger each
--
-- With none on, the strip's lights show the play position (global.lua).

local pads = require("lib.pads")

local strip = { mode = nil }

local BUTTONS = { PITCH = "pitch", MOD = "mod", PERFORM = "perform", NOTES = "notes" }
local LEDS = 25
local CENTRE = 13
-- What a strummed note plays at without Fixed Velocity (Maschine's default fixed one).
local STRUM_VELOCITY = 100

-- The first finger: touching, and where (0..1); each sound's mod wheel ("g:s");
-- each finger's strummed note ({ pad, sound, pitches }).
local touched, at = false, nil
local mod_of = {}
local strummed = {}

local function led_of(v)
  return math.floor(v * (LEDS - 1) + 0.5) + 1
end

local function sound_key(s)
  return s.group .. ":" .. s.sound
end

-- A mode button (true if `name` is one).
function strip.button(name, down)
  local mode = BUTTONS[name]
  if not mode then return false end
  if not down then return true end
  local shift = deckard.held("SHIFT")
  if mode == "perform" and shift then
    deckard.perform_fx()
    deckard.show_page("control")
    -- (It stays on.)
    if strip.mode == "perform" then return true end
  end
  strip.mode = strip.mode ~= mode and mode or nil
  return true
end

-- The pads strummed: those held, else all 16.
local function strum_pads()
  local out = {}
  for i = 1, 16 do
    if deckard.pad_held(i) then out[#out + 1] = i end
  end
  if #out == 0 then
    for i = 1, 16 do out[i] = i end
  end
  return out
end

-- What pad `pad` plays: its sound and pitches.
local function pad_notes(s, pad)
  if pads.keys() then
    local chords = pads.chords()
    if chords then return s.sound, chords[pad] end
    local note = pads.notes()[pad]
    return s.sound, (note and note <= 127) and { note } or {}
  end
  local group = s.groups[s.group]
  local snd = group and group.sounds[pad]
  return pad, { snd and snd.base_key or 60 }
end

local function stop_strum(finger)
  local n = strummed[finger]
  if not n then return end
  strummed[finger] = nil
  for _, p in ipairs(n.pitches) do deckard.strip_note(n.sound, p, n.velocity, false) end
end

local function strum(s, finger, pos)
  local list = strum_pads()
  local pad = list[math.min(#list, math.floor(pos * #list) + 1)]
  if strummed[finger] and strummed[finger].pad == pad then return end
  stop_strum(finger)
  local sound, pitches = pad_notes(s, pad)
  local velocity = s.fixed_velocity and s.velocity or STRUM_VELOCITY
  strummed[finger] = { pad = pad, sound = sound, pitches = pitches, velocity = velocity }
  for _, p in ipairs(pitches) do deckard.strip_note(sound, p, velocity, true) end
end

-- Finger `finger` (from 1) went "down", "move"d or came "up" at `pos` (0..1).
function strip.touch(finger, kind, pos)
  local mode = strip.mode
  if not mode then return true end
  local s = deckard.state()
  if finger == 1 then touched, at = kind ~= "up", pos end
  if mode == "notes" then
    if kind == "up" then stop_strum(finger) else strum(s, finger, pos) end
    return true
  end
  -- (The others follow the first finger.)
  if finger ~= 1 then return true end
  if mode == "pitch" then
    deckard.strip("pitch", kind == "up" and 0 or pos * 2 - 1)
  elseif mode == "mod" then
    if kind ~= "up" then
      mod_of[sound_key(s)] = pos
      deckard.strip("mod", pos)
    end
  elseif mode == "perform" then
    local group = s.groups[s.group]
    if group and group.perform then
      local what = kind == "down" and "perform_touch" or kind == "up" and "perform_release" or "perform"
      deckard.strip(what, pos)
    end
  end
  return true
end

-- The mode buttons' lights, and the strip's in a mode (true if it lit the strip).
function strip.leds(s)
  for name, mode in pairs(BUTTONS) do
    deckard.led(name, strip.mode == mode and "on" or "off")
  end
  local mode = strip.mode
  if not mode then return false end
  local group = s.groups[s.group]
  local snd = group and group.sounds[s.sound]
  -- Each LED's palette colour (16 white), or none.
  local lit = {}
  if mode == "pitch" and snd then
    -- From the middle to the finger, the finger white (the middle untouched).
    local p = touched and led_of(at) or CENTRE
    for i = 1, LEDS do
      if (i < p and i >= CENTRE) or (i > p and i <= CENTRE) then lit[i] = snd.palette end
    end
    lit[p] = 16
  elseif mode == "mod" and snd then
    -- From the left to the value, the value white.
    local p = led_of(touched and at or mod_of[sound_key(s)] or 0)
    for i = 1, p - 1 do lit[i] = snd.palette end
    lit[p] = 16
  elseif mode == "perform" and group and group.perform then
    -- The effect's position and either side in the group's colour, white while touched.
    local p = led_of(group.perform.pos)
    for i = p - 1, p + 1 do lit[i] = group.palette end
    if group.perform.touched then lit[p] = 16 end
  elseif mode == "notes" and touched and at then
    local palette = pads.keys() and snd and snd.palette or (group and group.palette)
    lit[led_of(at)] = palette
  end
  for i = 1, LEDS do
    local name = string.format("TS_%02d", i)
    if lit[i] then deckard.led(name, lit[i], true) else deckard.led(name, "off") end
  end
  return true
end

return strip
