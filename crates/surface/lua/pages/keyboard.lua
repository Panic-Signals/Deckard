-- Keyboard and Chords mode's page (KEYBOARD, CHORDS): the root note, the scale (and
-- the chord), and the pads' notes.
--
--   display 4         USER CHD: User Chord Edit mode, from Keyboard or Chords mode
--                     (the pads add their notes to the user chord picked, or take
--                     them out); pressed again, Chords mode plays the user chords
--   display 3         PREVIEW (while editing): hear the chord
--   display 5 / 6     the root note down / up an octave (editing: the pads)
--   display 7 / 8     the root note down / up a semitone (editing: the pads)
--   SHIFT + 5         Chords mode's user chords: delete the one picked
--   SHIFT + 7 / 8     move it to the slot before / after
--   pads 1–12         Chords mode's user chords: pick one (and play it)
--   LEFT / RIGHT      the knobs' pages. Keyboard:
--     1/4  knobs 1-2  the scale's bank and the scale
--     2/4  knob 1     Fixed Velocity's velocity
--     3/4  knobs 1-4  the focused sound's Pad Touch: mode, curve, min, max
--     4/4  knob 1     its CC
--                     Chords:
--     1/2  knobs 1-4  the scale's bank and the scale, the chord mode and type
--     2/2  knobs 1-2  the chord's position, Fixed Velocity's velocity
--
-- (The user chords play; the Harmonizer's and the chord sets are shown and kept, not
-- yet played.)

local pads = require("lib.pads")
local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local paged = require("lib.paged")

local page = {}

local pages = { keyboard = paged.new(4), chords = paged.new(2) }
local TOUCH_MODES = { "Velocity", "Poly AT", "Aftertouch", "CC" }

local function shown()
  return pages[pads.mode] or pages.keyboard
end

local function sound()
  local s = deckard.state()
  local group = s.groups[s.group]
  return s, group and group.sounds[s.sound]
end

local STEPS = { SCREEN_5 = -12, SCREEN_6 = 12, SCREEN_7 = -1, SCREEN_8 = 1 }

-- The user chord picked: its set and slot.
local function picked()
  return pads.chord_types.user, pads.user_slot
end

-- SHIFT's buttons in Chords mode with the user chords: whether the picked chord can be
-- deleted, or moved to the slot before / after (`next`).
local function managing()
  return pads.user_chords() and deckard.held("SHIFT")
end

local function can_delete()
  return managing() and #deckard.user_chord(picked()) > 0
end

local function can_move(next)
  local to = pads.user_slot + (next and 1 or -1)
  return managing() and to >= 1 and to <= 12
end

local function user_chd()
  if pads.mode == "chords" then
    pads.switch("keyboard", true)
  elseif pads.chord_edit then
    -- (Done: Chords mode plays the set's chords.)
    pads.chord_mode = 3
    pads.switch("chords")
  else
    pads.chord_edit = true
    pads.view = 0
  end
end

function page.button(name, down)
  if name == "SCREEN_3" and pads.chord_edit then
    local set, slot = picked()
    deckard.user_chord_preview(set, slot, pads.base, down)
    return true
  end
  if not down then return end
  if paged.button(shown(), name) then return true end
  if name == "SCREEN_4" then
    user_chd()
  elseif managing() and STEPS[name] then
    local set, slot = picked()
    if name == "SCREEN_5" and can_delete() then
      deckard.user_chord_delete(set, slot)
    elseif (name == "SCREEN_7" or name == "SCREEN_8") and can_move(name == "SCREEN_8") then
      local next = name == "SCREEN_8"
      deckard.user_chord_move(set, slot, next)
      pads.user_slot = slot + (next and 1 or -1)
    end
  elseif STEPS[name] then
    if pads.chord_edit then
      pads.view = math.max(-pads.base, math.min(127 - pads.base, pads.view + STEPS[name]))
    else
      pads.move(STEPS[name])
    end
  else
    return
  end
  return true
end

function page.pad(n)
  if pads.chord_edit then
    local note = pads.notes()[n]
    if note < 128 then
      local set, slot = picked()
      deckard.user_chord_note(set, slot, note - pads.base)
    end
  elseif pads.user_chords() and n <= 12 then
    pads.user_slot = n
  end
  return true
end

-- Keep `v` within lo..hi, stepped by `steps`; `set` takes a changed value.
local function step(v, steps, lo, hi, set)
  local to = math.max(lo, math.min(hi, v + steps))
  if to ~= v then set(to) end
end

local function scale_knob(n, amount)
  local steps = knobs.gather(n, amount)
  if steps == 0 then return end
  local scale = deckard.state().scale
  if n == 1 then
    step(scale.bank, steps, 1, #deckard.scales.banks, function(b) pads.scale(b, 1) end)
  else
    step(scale.kind, steps, 1, #deckard.scales.names[scale.bank], function(k) pads.scale(scale.bank, k) end)
  end
end

local function chord_knob(n, amount)
  local steps = knobs.gather(n, amount)
  if steps == 0 then return end
  if n == 3 then
    step(pads.chord_mode, steps, 1, #deckard.scales.chord_modes, function(m) pads.chord_mode = m end)
  elseif n == 4 then
    local list, _, key = pads.chord_list()
    step(pads.chord_types[key], steps, 1, #list, function(t) pads.chord_types[key] = t end)
  else
    step(pads.chord_position, steps, 1, #deckard.scales.positions, function(p) pads.chord_position = p end)
  end
end

local function velocity_knob(amount)
  local s = deckard.state()
  step(s.velocity, knobs.each(amount), 0, 127, deckard.velocity)
end

local function touch_knob(field, n, amount)
  local _, snd = sound()
  if not snd then return end
  local t = snd.touch
  if field == "mode" then
    local steps = knobs.gather(n, amount)
    step(t.mode, steps, 0, 3, function(v) deckard.pad_touch("mode", v) end)
  else
    local lo, hi = 0, field == "cc" and 127 or 100
    if field == "min" then hi = t.max elseif field == "max" then lo = t.min end
    step(t[field], knobs.each(amount), lo, hi, function(v) deckard.pad_touch(field, v) end)
  end
end

local TOUCH = { "mode", "curve", "min", "max" }

function page.knob(n, amount)
  local at = shown().at
  if pads.mode == "chords" then
    if at == 1 then
      if n > 2 then chord_knob(n, amount) elseif pads.chord_mode == 1 then scale_knob(n, amount) end
    elseif n == 1 then
      chord_knob(n, amount)
    elseif n == 2 then
      velocity_knob(amount)
    end
  elseif at == 1 then
    if n <= 2 then scale_knob(n, amount) end
  elseif at == 2 then
    if n == 1 then velocity_knob(amount) end
  elseif at == 3 then
    touch_knob(TOUCH[n], n, amount)
  elseif n == 1 then
    touch_knob("cc", n, amount)
  end
  return true
end

-- The pads' lights over global.lua's: Chords mode's user chords (white while empty,
-- the one picked or held bright, pads 13–16 off), or the notes in the user chord being
-- edited white.
local function pad_leds()
  local _, snd = sound()
  local palette = snd and snd.palette or 16
  if pads.user_chords() then
    for i = 1, 16 do
      if i > 12 then
        deckard.led("PAD_" .. i, "off")
      else
        local empty = #deckard.user_chord(pads.chord_types.user, i) == 0
        deckard.led("PAD_" .. i, empty and 16 or palette, i == pads.user_slot or deckard.pad_held(i))
      end
    end
  elseif pads.chord_edit then
    local chord = {}
    for _, n in ipairs(deckard.user_chord(picked())) do chord[n] = true end
    for i, note in ipairs(pads.notes()) do
      if chord[note - pads.base] then
        deckard.led("PAD_" .. i, 16, deckard.pad_held(i) or note % 12 == pads.base % 12)
      end
    end
  end
end

function page.leds()
  deckard.led("SCREEN_1", "on")
  deckard.led("SCREEN_3", pads.chord_edit and "dim" or "off")
  deckard.led("SCREEN_4", pads.chord_edit and "on" or "dim")
  if managing() then
    deckard.led("SCREEN_5", can_delete() and "dim" or "off")
    deckard.led("SCREEN_6", "off")
    deckard.led("SCREEN_7", can_move(false) and "dim" or "off")
    deckard.led("SCREEN_8", can_move(true) and "dim" or "off")
  else
    for i = 5, 8 do deckard.led("SCREEN_" .. i, "dim") end
  end
  paged.leds(shown())
  pad_leds()
end

-- The knobs' sections and parameters on the page shown.
local function params(s, snd)
  local sc, scale = deckard.scales, s.scale
  local bank = { name = "BANK", value = sc.banks[scale.bank] }
  local kind = { name = "TYPE", value = sc.short[scale.bank][scale.kind] }
  local velocity = { name = "FIXEDVELOCITY", value = tostring(s.velocity) }
  local at = shown().at
  if pads.mode == "chords" then
    if at == 1 then
      local _, short, key = pads.chord_list()
      local chord = {
        { name = "MODE", value = sc.chord_modes_short[pads.chord_mode] },
        { name = key == "user" and "SET" or "TYPE", value = short[pads.chord_types[key]] },
      }
      -- (The chord sets and user chords aren't bound to the scale: no Scale knobs.)
      if pads.chord_mode > 1 then return { { text = "Chord", at = 3 } }, { nil, nil, chord[1], chord[2] } end
      return { { text = "Scale", at = 1 }, { text = "Chord", at = 3 } }, { bank, kind, chord[1], chord[2] }
    end
    return { { text = "Chord", at = 1 }, { text = "Fixed Velocity", at = 2 } }, {
      { name = "CHORDPOSITION", value = sc.positions[pads.chord_position] },
      velocity,
    }
  end
  local t = snd and snd.touch or { mode = 0, cc = 1, curve = 50, min = 0, max = 100 }
  if at == 1 then return { { text = "Scale", at = 1 } }, { bank, kind } end
  if at == 2 then return { { text = "Fixed Velocity", at = 1 } }, { velocity } end
  if at == 3 then
    return { { text = "Pad Touch", at = 1 } }, {
      { name = "MODE", value = TOUCH_MODES[t.mode + 1] },
      { name = "CURVE", value = tostring(t.curve) },
      { name = "MIN", value = tostring(t.min) },
      { name = "MAX", value = tostring(t.max) },
    }
  end
  return { { text = "Pad Touch CC", at = 1 } }, { { name = "CC", value = tostring(t.cc) } }
end

-- The right screen's pads: their notes (white where they're in the user chord being
-- edited), or Chords mode's user chords (white while empty, pads 13–16 off).
local function pad_tiles(snd)
  local keys = {}
  local palette = snd and snd.palette
  if pads.user_chords() then
    for i = 1, 12 do
      local empty = #deckard.user_chord(pads.chord_types.user, i) == 0
      keys[i] = { name = empty and "" or ("Chord " .. i), palette = empty and 16 or palette, on = i == pads.user_slot }
    end
    return keys
  end
  local chord = {}
  if pads.chord_edit then
    for _, n in ipairs(deckard.user_chord(picked())) do chord[n] = true end
  end
  for i, note in ipairs(pads.notes()) do
    keys[i] = {
      name = deckard.note_name(pads.base, note),
      palette = chord[note - pads.base] and 16 or palette,
      on = note % 12 == pads.base % 12 or deckard.pad_held(i),
    }
  end
  return keys
end

function page.draw(screen, theme)
  local s, snd = sound()
  local chords = pads.mode == "chords"
  local buttons = {
    { text = chords and "CHORDS" or "KEYBOARD", selected = true },
    nil,
    pads.chord_edit and { text = "PREVIEW" } or nil,
    { text = "USER CHD", selected = pads.chord_edit },
    { text = "OCTAVE-" },
    { text = "OCTAVE+" },
    { text = "SEMITONE-" },
    { text = "SEMITONE+" },
  }
  if managing() then
    buttons[5] = { text = "DELETE", disabled = not can_delete() }
    buttons[6] = nil
    buttons[7] = { arrows = true, text = "MOVE", left = can_move(false), right = can_move(true) }
    buttons[8] = nil
  end
  widgets.buttons(screen, theme, buttons)
  widgets.info_bar(screen, theme, {
    number = s.sound,
    palette = snd and snd.palette,
    name = snd and snd.name or "",
    values = { { text = deckard.note_name(pads.base, pads.base) }, { text = "ROOT NOTE", title = true } },
  })
  if pads.chord_edit then
    local set, slot = picked()
    local name = #deckard.user_chord(set, slot) > 0 and ("Chord " .. slot) or ""
    screen:label(4, 60, 464, 24, deckard.scales.user_chord_sets[set] .. " - " .. name, "default", "left", "center")
  end
  local groups, list = params(s, snd)
  widgets.params(screen, theme, 0, groups, paged.label(shown()), list)
  widgets.pad_grid(screen, theme, pad_tiles(snd))
end

return page
