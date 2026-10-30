-- The pads' mode, shared by global.lua and the pad pages: "pad", "keyboard",
-- "chords" or "step"; Keyboard mode's root note; each sound's base key; the Chords
-- page's settings; whether a pad's hit focuses its sound.

local pads = {
  mode = "pad",
  -- Keyboard mode's lowest pad, the scale's root: C3 (MIDI 60), moved by octaves and
  -- semitones.
  base = 60,
  -- Pad mode's base key per sound ("group:sound"), C3 unless moved.
  base_keys = {},
  -- The Chords page: its mode (1 Harmonizer, 2 Chord Set, 3 User Chord), the chord
  -- type kept for each kind of list (`pads.chord_list`), and the position (10: Root,
  -- in `deckard.scales.positions`). As a new project starts: Harmonizer, Major / 1-3-5.
  chord_mode = 1,
  chord_types = { chromatic = 4, scale = 4, set = 1, user = 1 },
  chord_position = 10,
  -- Focus (PAD MODE's second page): a pad's hit focuses its sound ("On Trigger"), or
  -- the focused sound stays ("Pinned").
  focus_pinned = false,
  -- User Chord Edit mode (USER CHD): Keyboard mode's pads add their notes to the user
  -- chord picked (`user_slot`, 1–12, in the set `chord_types.user`), or take them out;
  -- the octave and semitone buttons move the pads (`view`, semitones) and not the root.
  chord_edit = false,
  user_slot = 1,
  view = 0,
  -- Step mode's KEYBOARD (`pads.keyboard`).
  step_keyboard = false,
}

-- The page each mode shows.
local MODE_PAGES = { pad = "pad_mode", keyboard = "keyboard", chords = "keyboard", step = "step" }

-- Switch the pads to `mode` and show its page. (Editing a user chord ends unless
-- `editing`.)
function pads.switch(mode, editing)
  local was_keys = pads.keyboard()
  -- (Step mode keeps Keyboard mode on, as Maschine's group keyboard mode stays.)
  pads.step_keyboard = mode == "step" and was_keys
  pads.mode = mode
  pads.chord_edit = editing or false
  pads.view = 0
  if pads.keyboard() ~= was_keys then deckard.keyboard_mode(pads.keyboard()) end
  if mode == "step" then
    require("pages.step").shown()
    if deckard.page() ~= "step" then pads.before_step = deckard.page() end
  end
  deckard.show_page(MODE_PAGES[mode])
end

-- Step mode off for Pad mode (NOTE REPEAT), back on the page Step mode was entered
-- from, as Maschine takes its Step page off.
function pads.leave_step()
  local was_keys = pads.keyboard()
  pads.mode, pads.step_keyboard = "pad", false
  if was_keys then deckard.keyboard_mode(false) end
  deckard.show_page(pads.before_step or MODE_PAGES.pad)
end

-- Step mode's KEYBOARD: Keyboard mode on or off with the pads still steps.
function pads.toggle_step_keyboard()
  pads.step_keyboard = not pads.step_keyboard
  deckard.keyboard_mode(pads.step_keyboard)
end

local LOWEST, HIGHEST = 0, 127 - 15

-- The semitones above the root the scale chosen plays.
local function steps()
  local scale = deckard.state().scale
  return deckard.scales.steps[scale.bank][scale.kind]
end

-- The 16 pads' notes in Keyboard and Chords mode (pad 1 at the bottom left): the
-- scale's notes up from the root (from `pads.view` above it while editing a user
-- chord), an octave higher each time round; 128 (none) outside MIDI's range.
function pads.notes()
  local out, st = {}, steps()
  -- The scale's notes from the first at or above where the pads start.
  local from = pads.base + pads.view
  local k = 0
  while pads.base + 12 * ((k - 1) // #st) + st[(k - 1) % #st + 1] >= from do k = k - 1 end
  while pads.base + 12 * (k // #st) + st[k % #st + 1] < from do k = k + 1 end
  for i = 1, 16 do
    local note = pads.base + 12 * (k // #st) + st[k % #st + 1]
    out[i] = (note >= 0 and note <= 127) and note or 128
    k = k + 1
  end
  return out
end

-- Whether the Chords page plays the user chords.
function pads.user_chords()
  return pads.mode == "chords" and pads.chord_mode == 3
end

-- Chords mode's Auto position: the state each chord leads from (`deckard.chord`), and
-- what each pad's hit would make of it; the pads held last time, to see new hits.
pads.auto = nil
local auto_after, was_held = {}, {}

-- The notes each pad plays in Chords mode, where they're chords (docs/scales.md,
-- "Chord voicings"): the Harmonizer's on Keyboard mode's notes; a chord set's or the
-- user chord set's on pads 1–12, from the root (none on pads 13–16).
function pads.chords()
  if pads.mode ~= "chords" then return nil end
  -- (A pad hit since last time played the chord worked out then: Auto moves on.)
  for i = 1, 16 do
    local held = deckard.pad_held(i)
    if held and not was_held[i] and auto_after[i] then pads.auto = auto_after[i] end
    was_held[i] = held
  end
  local _, _, key = pads.chord_list()
  local spec = {
    mode = pads.chord_mode, type = pads.chord_types[key], chromatic = pads.chord_types.chromatic,
    position = pads.chord_position, root = pads.base,
  }
  local keys = pads.chord_mode == 1 and pads.notes() or nil
  local out = {}
  for i = 1, 16 do
    local note
    if keys then note = keys[i] elseif i <= 12 then note = pads.base + i - 1 end
    if note and note <= 127 then
      out[i], auto_after[i] = deckard.chord(note, spec, pads.auto)
    else
      out[i], auto_after[i] = {}, nil
    end
  end
  return out
end

-- Choose the scale: bank and kind (from 1) played from the root.
function pads.scale(bank, kind)
  deckard.scale(pads.base, bank, kind)
end

-- Move the root by `semitones`, within MIDI's range.
function pads.move(semitones)
  pads.base = math.max(LOWEST, math.min(HIGHEST, pads.base + semitones))
  local scale = deckard.state().scale
  pads.scale(scale.bank, scale.kind)
end

-- The chord types the Chords page lists now (full and short names) and the key its
-- choice is kept under: the Harmonizer's for the chromatic scale or for the others,
-- the chord sets, the user chord sets.
function pads.chord_list()
  local sc, scale = deckard.scales, deckard.state().scale
  if pads.chord_mode == 2 then return sc.chord_sets, sc.chord_sets_short, "set" end
  if pads.chord_mode == 3 then return sc.user_chord_sets, sc.user_chord_sets, "user" end
  if scale.bank == 1 and scale.kind == 1 then return sc.chords_chromatic, sc.chords_chromatic_short, "chromatic" end
  return sc.chords_scale, sc.chords_scale, "scale"
end

-- The focused sound's base key, and moving it (0..127).
function pads.base_key(group, sound)
  return pads.base_keys[group .. ":" .. sound] or 60
end

function pads.move_base_key(group, sound, semitones)
  local k = group .. ":" .. sound
  pads.base_keys[k] = math.max(0, math.min(127, pads.base_key(group, sound) + semitones))
end

-- Whether the pads play notes of the focused sound (Keyboard, Chords).
function pads.keys()
  return pads.mode == "keyboard" or pads.mode == "chords"
end

-- Whether Keyboard mode is on (the focused sound's notes by pitch): Keyboard and
-- Chords mode, or Step mode's KEYBOARD.
function pads.keyboard()
  return pads.keys() or (pads.mode == "step" and pads.step_keyboard == true)
end

-- A note's name as the screens show it: C3, C#3 (C3 = MIDI 60).
local NAMES = { "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B" }
function pads.note_name(n)
  return NAMES[n % 12 + 1] .. (n // 12 - 2)
end

return pads
