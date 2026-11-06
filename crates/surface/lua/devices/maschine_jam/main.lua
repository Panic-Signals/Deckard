-- Maschine JAM, as its manual (September 2016) describes it. Written from the manual and from
-- what's printed on the panel; nothing here comes from NI's scripts. docs/maschine-jam.md lists
-- what every control does, and what differs.
--
-- The 8x8 matrix shows one view: Project View (SONG), Pad, Keyboard, Step, Piano Roll, Notes,
-- Lock or Pattern Length. While SELECT, SOLO, MUTE or a channel mode's button is held it shows
-- the focused group's sounds whatever the view, on the pads printed 1-16.
--
-- There's no on-screen overlay. What it would show goes to the status bar as a line, and the
-- encoder walks those lines: turn to pick one, press to change it, turn, press again. The
-- library opens as the main window's own panel.
--
-- Columns and rows count from 1, from the top left; strip LEDs and meter LEDs from the bottom.

local M = {}

-- ---- what doesn't change ---------------------------------------------------------------

local STRIP_LEDS, METER_LEDS = 11, 8
local WHITE = 16
-- What a pad plays in Pad mode: a sound's root, as the MK3's pads.
local PAD_PITCH = 60
-- A double tap: two touches this close (seconds).
local DOUBLE_TAP = 0.3
-- A menu opened by touching the encoder closes this long after the last use (seconds).
local MENU_IDLE = 6
-- Taps further apart than this start a new count (seconds).
local TAP_RESET = 2.0
-- The meters' floor (dB).
local METER_FLOOR = -60
-- A note at least this loud shows as an accent.
local ACCENT_FROM = 120

-- Held, these act on whatever is pressed next; SONG pins them (manual 4.3).
local TEMPORARY = { SELECT = true, SOLO = true, MUTE = true, DUPLICATE = true, CLEAR = true }
-- Their buttons pick what the strips do; held, a group, pad or MST picks the channel (manual 4.1).
local CHANNEL_MODES = { MACRO = "macro", LEVEL = "level", AUX = "aux", CONTROL = "control", TUNE = "tune", SWING = "swing" }
local MODE_LIGHTS = {
  MACRO = { macro = true },
  LEVEL = { level = true, pan = true },
  AUX = { aux = true },
  CONTROL = { control = true },
  TUNE = { tune = true },
  SWING = { swing = true },
  PERFORM = { perform = true },
  NOTES = { notes = true },
}

local KINDS = { "PROJECTS", "GROUPS", "SOUNDS", "INSTRUMENTS", "EFFECTS", "LOOPS", "ONE-SHOTS" }
local STEP_GRIDS = { "1 Bar", "1/2", "1/4", "1/8", "1/16", "1/32", "1/64", "1/128", "1 Bar T", "1/2 T", "1/4 T", "1/8 T", "1/16 T", "1/32 T", "1/64 T" }
local NUDGE_GRIDS = { "Step", "Step/2", "Step/4", "Step/8", "Step/16" }
local ARRANGE_GRIDS = { "1 Bar", "1/2", "1/4", "1/8", "1/16" }
local PERFORM_GRIDS = { "Off", "1 Bar", "1/2", "1/4", "1/8", "1/16", "Section" }
local PERFORM_MODES = { "Filter", "Flanger", "Burst Echo", "Reso Echo", "Ring", "Stutter", "Tremolo", "Scratcher" }
local MORPH_MODES = { "Travel", "Target" }
local MORPH_TIMES = { "1/2 Bar", "1 Bar", "2 Bars", "4 Bars", "8 Bars", "16 Bars" }
local MORPH_GRIDS = { "1/8", "1/4", "1/2", "Bar" }
local NOTE_NAMES = { "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B" }
local NOTES_MODES = { "Guitar", "Chords", "User" }
local CHORD_MODES = { [0] = "Off", "Harmonizer", "Chord Set", "User Chord" }
-- Scale degrees above each column's own: barre chords (Guitar), keyboard triads (Chords).
local NOTES_PRESETS = { { 0, 4, 7, 9, 11, 14 }, { 0, 2, 4, 7 }, {} }

-- ---- state ----------------------------------------------------------------------------

local st

local function notes_sets()
  local sets = {}
  for mode, shape in ipairs(NOTES_PRESETS) do
    sets[mode] = {}
    for col = 1, 8 do
      sets[mode][col] = {}
      for _, d in ipairs(shape) do sets[mode][col][col - 1 + d] = true end
    end
  end
  return sets
end

local function fresh()
  return {
    view = "song",
    -- What Step, Piano Roll, Notes, Lock and Pattern Length go back to.
    back = "song",
    -- Scrolling: from 0, the first group, pattern and scene shown.
    group_bank = 0,
    pattern_row = 0,
    scene_bank = 0,
    last_group = nil,
    -- The strips: their mode, groups or sounds, and which eight sounds.
    strip = "level",
    scope = "groups",
    sound_page = 0,
    aux = 1,
    strips = {},
    -- The meters and what the encoder sets when nothing else has it.
    meter = "mst",
    tempo = false,
    -- Modes pinned (SONG), and pressed on.
    pins = {},
    latched = {},
    -- DUPLICATE's source: what the next press copies.
    dup = nil,
    -- Step and Piano Roll.
    lanes = 1,
    step_page = 0,
    step_follow = false,
    accent = false,
    velocity = 100,
    accent_velocity = 127,
    step_used = false,
    step_entered = false,
    -- Keyboard, Piano Roll and Notes: octaves from the root, the lowest degree shown.
    octave = -2,
    roll_low = 0,
    chord = { mode = 0, types = { chromatic = 4, scale = 4, set = 1, user = 1 } },
    notes = { mode = 1, low = 0, sets = notes_sets(), sounding = {} },
    notes_used = false,
    repeating = false,
    arp = false,
    -- Pads held, for their lights.
    pressed = {},
    -- The status bar's menu, and the library.
    menu = nil,
    browser = nil,
    taps = {},
    said = nil,
  }
end

st = fresh()

-- ---- small things ---------------------------------------------------------------------

local function clamp(v, lo, hi)
  return math.max(lo, math.min(hi, v))
end

local function held(name)
  return deckard.held(name)
end

local function active(name)
  return held(name) or st.latched[name] == true
end

local function shift()
  return held("SHIFT")
end

-- A channel mode's button is held.
local function mode_held()
  for name in pairs(CHANNEL_MODES) do
    if held(name) then return name end
  end
  return nil
end

-- The matrix shows the sounds, whatever the view.
local function sounds_shown()
  return active("SELECT") or active("SOLO") or active("MUTE") or mode_held() ~= nil
end

local function say(text)
  if text ~= st.said then
    st.said = text
    deckard.message(text)
  end
end

-- Flashing, on and off several times a second.
local function blink()
  return deckard.now() * 4 % 1 < 0.5
end

-- With the beat: on one beat, off the next.
local function beat_flash(s)
  return math.floor(deckard.now() * math.max(s.tempo or 120, 1) / 60) % 2 == 0
end

local function group_letter(i)
  return string.char(string.byte("A") + i - 1)
end

local function bar_ticks(s)
  return math.max(1, (s.numerator or 4) * 3840 // math.max(s.denominator or 4, 1))
end

local function focused_group(s)
  return s.groups[s.group]
end

local function focused_sound(s)
  local g = focused_group(s)
  return g and g.sounds[s.sound]
end

-- Pad `n` (1-16) printed on the panel: at the bottom right, 1 at its bottom left.
local function sound_at(col, row)
  if col < 5 or row < 5 then return nil end
  return (8 - row) * 4 + (col - 4)
end

local function place_of(n)
  local i = n - 1
  return 5 + i % 4, 8 - i // 4
end

local function pad_key(col, row)
  return col + (row - 1) * 8
end

-- ---- scales and notes -----------------------------------------------------------------

local function scale_steps(s)
  return deckard.scales.steps[s.scale.bank][s.scale.kind]
end

-- Degree `d` of the scale played from `base`, an octave higher each time round.
local function degree_note(s, base, d)
  local steps = scale_steps(s)
  local n = #steps
  return base + 12 * (d // n) + steps[d % n + 1]
end

local function keyboard_base(s)
  return (s.scale.root or 60) + 12 * st.octave
end

local function chord_list(s)
  local sc = deckard.scales
  local mode = st.chord.mode
  if mode == 2 then return sc.chord_sets, "set" end
  if mode == 3 then return sc.user_chord_sets, "user" end
  if s.scale.bank == 1 and s.scale.kind == 1 then return sc.chords_chromatic, "chromatic" end
  return sc.chords_scale, "scale"
end

-- What a note plays with the chord settings: itself, or its chord.
local function chord_of(s, note)
  if st.chord.mode == 0 then return { note } end
  local _, key = chord_list(s)
  local spec = {
    mode = st.chord.mode,
    type = st.chord.types[key],
    chromatic = st.chord.types.chromatic,
    position = 10,
    root = s.scale.root or 60,
  }
  return (deckard.chord(note, spec, nil))
end

-- The note of a Keyboard pad: up from the bottom left. With a chord set, 12 pads, each a
-- semitone, each its chord (manual 6.3.4.3).
local function key_note(s, col, row)
  local k = (8 - row) * 8 + (col - 1)
  if st.chord.mode >= 2 then
    if k >= 12 then return nil end
    return keyboard_base(s) + k
  end
  local note = degree_note(s, keyboard_base(s), k)
  return (note >= 0 and note <= 127) and note or nil
end

local function is_root(s, note)
  return note % 12 == (s.scale.root or 60) % 12
end

-- The pitches of a Piano Roll row: the top row highest.
local function roll_note(s, row)
  return degree_note(s, keyboard_base(s), st.roll_low + (8 - row))
end

-- ---- steps ----------------------------------------------------------------------------

local function step_ticks(s)
  return math.max(s.grid.step_ticks or 240, 1)
end

-- The first step's tick: the first on the grid in the pattern.
local function first_step(s)
  local t = step_ticks(s)
  return -(-s.pattern_start // t) * t
end

-- How many steps one part shows.
local function part_steps()
  if st.view == "roll" then return 8 end
  if st.lanes == 8 then return 8 end
  if st.lanes == 4 then return 16 end
  return 64
end

-- The step and sound under a pad, in Step mode.
local function step_cell(s, col, row)
  local index, sound
  if st.lanes == 8 then
    sound = (s.sound - 1) // 8 * 8 + row
    index = col - 1
  elseif st.lanes == 4 then
    sound = (s.sound - 1) // 4 * 4 + (row - 1) // 2 + 1
    index = ((row - 1) % 2) * 8 + col - 1
  else
    sound = s.sound
    index = (row - 1) * 8 + col - 1
  end
  return sound, st.step_page * part_steps() + index
end

local function step_at(s, index)
  return first_step(s) + index * step_ticks(s)
end

-- Inside the pattern.
local function step_in_pattern(s, at)
  return at < s.pattern_start + math.max(s.pattern_length, 1)
end

local function parts(s)
  local ticks = part_steps() * step_ticks(s)
  return math.max(1, math.min(8, (math.max(s.pattern_length, 1) + ticks - 1) // ticks))
end

-- Which step plays now, in the focused pattern.
local function playing_step(s)
  if not s.pattern_position then return nil end
  return (s.pattern_position - first_step(s)) // step_ticks(s)
end

-- Follow: the part shown goes with the song.
local function follow_steps(s)
  if not st.step_follow or (st.view ~= "step" and st.view ~= "roll") then return end
  local at = playing_step(s)
  if at then st.step_page = clamp(at // part_steps(), 0, 7) end
end

local function step_velocity()
  return st.accent and st.accent_velocity or st.velocity
end

local function toggle_follow(s)
  st.step_follow = not st.step_follow
  deckard.follow(st.step_follow)
  say("Follow: " .. (st.step_follow and "On" or "Off"))
  return s
end

-- ---- the views ------------------------------------------------------------------------

local function set_view(view)
  if view == st.view then return end
  if st.view == "song" or st.view == "pad" or st.view == "keyboard" then st.back = st.view end
  st.view = view
  st.step_page = 0
  deckard.keyboard_mode(view == "keyboard" or view == "roll" or view == "notes")
end

local function close_menu()
  if st.menu and st.menu.settings then deckard.settings_wanted(false) end
  st.menu = nil
end

-- ---- the status bar's menus -----------------------------------------------------------

local function menu_line()
  local m = st.menu
  if not m then return end
  local s = deckard.state()
  local item = m.items[m.at]
  local value = item.text and item.text(s) or ""
  local shown = m.editing and ("[" .. value .. "]") or value
  say(m.title .. " ▸ " .. item.name .. (value ~= "" and (": " .. shown) or ""))
end

local function open_menu(title, items, opts)
  close_menu()
  opts = opts or {}
  st.menu = {
    title = title,
    items = items,
    at = 1,
    editing = #items == 1 and items[1].turn ~= nil,
    settings = opts.settings,
    idle = opts.idle,
    used = deckard.now(),
  }
  if opts.settings then deckard.settings_wanted(true) end
  menu_line()
end

local function menu_turn(steps)
  local m = st.menu
  m.used = deckard.now()
  if m.editing then
    local item = m.items[m.at]
    if item.turn then item.turn(deckard.state(), steps, shift()) end
  else
    m.at = clamp(m.at + steps, 1, #m.items)
  end
  menu_line()
end

local function menu_press()
  local m = st.menu
  m.used = deckard.now()
  local item = m.items[m.at]
  if item.press then
    item.press(deckard.state())
  elseif item.turn then
    m.editing = not m.editing
  end
  menu_line()
end

-- An item choosing from `list` by its place from 0.
local function list_item(name, list, get, set)
  return {
    name = name,
    text = function(s) return list[get(s) + 1] or "-" end,
    turn = function(s, steps)
      local to = clamp(get(s) + steps, 0, #list - 1)
      if to ~= get(s) then set(to) end
    end,
  }
end

local function grid_menu()
  local function item(name, list, key)
    return list_item(name, list, function(s) return s.grid[key] end, function(to) deckard.grid(key, to) end)
  end
  open_menu("GRID", {
    item("Step", STEP_GRIDS, "step"),
    item("Nudge", NUDGE_GRIDS, "nudge"),
    item("Arrange", ARRANGE_GRIDS, "arrange"),
    item("Perform", PERFORM_GRIDS, "perform"),
  })
end

local function rec_mode_menu()
  local function setting(name, key)
    return {
      name = name,
      text = function(s) return s.settings.values[key] or "-" end,
      turn = function(_, steps) deckard.setting(key, steps) end,
    }
  end
  local function local_number(name, key)
    return {
      name = name,
      text = function() return tostring(st[key]) end,
      turn = function(_, steps) st[key] = clamp(st[key] + steps, 1, 127) end,
    }
  end
  open_menu("REC MODE", {
    setting("Metronome Level", "metronome_volume"),
    setting("Metronome Time", "metronome_ticks"),
    setting("Metronome Auto", "metronome_auto"),
    setting("Count-in", "count_in"),
    setting("Quantize", "quantize"),
    local_number("Default Velocity", "velocity"),
    local_number("Accent Velocity", "accent_velocity"),
    {
      name = "Follow",
      text = function() return st.step_follow and "On" or "Off" end,
      turn = function(s) toggle_follow(s) end,
    },
  }, { settings = true })
end

local function scale_menu(opts)
  local function set_scale(s, root, bank, kind)
    deckard.scale(root or s.scale.root, bank or s.scale.bank, kind or s.scale.kind)
  end
  open_menu("SCALE", {
    {
      name = "Root Note",
      text = function(s) return NOTE_NAMES[(s.scale.root or 60) % 12 + 1] end,
      turn = function(s, steps) set_scale(s, clamp((s.scale.root or 60) + steps, 48, 71)) end,
    },
    {
      name = "Bank",
      text = function(s) return deckard.scales.banks[s.scale.bank] end,
      turn = function(s, steps)
        set_scale(s, nil, clamp(s.scale.bank + steps, 1, #deckard.scales.banks), 1)
      end,
    },
    {
      name = "Type",
      text = function(s) return deckard.scales.names[s.scale.bank][s.scale.kind] or "-" end,
      turn = function(s, steps)
        local kinds = deckard.scales.names[s.scale.bank]
        set_scale(s, nil, nil, clamp(s.scale.kind + steps, 1, #kinds))
      end,
    },
    {
      name = "Chord Mode",
      text = function() return CHORD_MODES[st.chord.mode] end,
      turn = function(_, steps) st.chord.mode = clamp(st.chord.mode + steps, 0, 3) end,
    },
    {
      name = "Chord Type",
      text = function(s)
        if st.chord.mode == 0 then return "-" end
        local list, key = chord_list(s)
        return list[st.chord.types[key]] or "-"
      end,
      turn = function(s, steps)
        if st.chord.mode == 0 then return end
        local list, key = chord_list(s)
        st.chord.types[key] = clamp(st.chord.types[key] + steps, 1, #list)
      end,
    },
  }, opts)
end

local function repeat_menu(opts)
  local function setting(key, name)
    return {
      name = name,
      text = function(s)
        local k = s.note_repeat.settings[key]
        return k and k.text or "-"
      end,
      turn = function(s, steps)
        local k = s.note_repeat.settings[key]
        if k then deckard.note_repeat(key, clamp(k.value + steps * k.step, k.min, k.max)) end
      end,
    }
  end
  local function switch(key, name)
    return {
      name = name,
      text = function(s) return s.note_repeat[key] and "On" or "Off" end,
      turn = function(s) deckard.note_repeat(key, not s.note_repeat[key]) end,
    }
  end
  local items
  if st.arp then
    items = {
      setting("type", "Type"), setting("rate", "Rate"), setting("rate_unit", "Unit"), setting("sequence", "Sequence"),
      setting("octaves", "Octaves"), setting("dynamic", "Dynamic"), setting("gate", "Gate"), switch("locked", "Lock"),
      switch("hold", "Hold"),
    }
  else
    items = { setting("rate", "Rate"), setting("rate_unit", "Unit"), setting("gate", "Gate"), switch("locked", "Lock"), switch("hold", "Hold") }
  end
  open_menu(st.arp and "ARP" or "NOTE REPEAT", items, opts)
end

local function lock_menu(opts)
  open_menu("LOCK", {
    {
      name = "Morph",
      text = function(s) return s.lock.morph and "On" or "Off" end,
      turn = function(s) deckard.lock("morph", not s.lock.morph) end,
    },
    list_item("Mode", MORPH_MODES, function(s) return s.lock.mode end, function(to) deckard.lock("morph_mode", to) end),
    {
      name = "Time / Grid",
      text = function(s)
        if s.lock.mode == 1 then return MORPH_GRIDS[s.lock.grid + 1] end
        return MORPH_TIMES[s.lock.time + 1]
      end,
      turn = function(s, steps)
        if s.lock.mode == 1 then
          deckard.lock("morph_grid", clamp(s.lock.grid + steps, 0, #MORPH_GRIDS - 1))
        else
          deckard.lock("morph_time", clamp(s.lock.time + steps, 0, #MORPH_TIMES - 1))
        end
      end,
    },
  }, opts)
end

local function swing_menu()
  local function by(steps, fine) return steps * (fine and 0.1 or 1) end
  open_menu("SWING", {
    {
      name = "Master",
      text = function(s) return string.format("%.1f %%", s.swing) end,
      turn = function(s, steps, fine) deckard.swing(s.swing + by(steps, fine)) end,
    },
    {
      name = "Group",
      text = function(s)
        local g = focused_group(s)
        return g and string.format("%.1f %%", g.swing or 0) or "-"
      end,
      turn = function(s, steps, fine)
        local g = focused_group(s)
        if g then deckard.set_swing(s.group, nil, (g.swing or 0) + by(steps, fine)) end
      end,
    },
    {
      name = "Sound",
      text = function(s)
        local x = focused_sound(s)
        return x and string.format("%.1f %%", x.swing or 0) or "-"
      end,
      turn = function(s, steps, fine)
        local x = focused_sound(s)
        if x then deckard.set_swing(s.group, s.sound, (x.swing or 0) + by(steps, fine)) end
      end,
    },
  })
end

local function tune_menu()
  local function text(c)
    return c and c.tune and string.format("%+.2f st", c.tune) or "no Sampler"
  end
  local function by(steps, fine) return steps * (fine and 0.01 or 1) end
  open_menu("TUNE", {
    {
      name = "Group",
      text = function(s) return text(focused_group(s)) end,
      turn = function(s, steps, fine)
        local g = focused_group(s)
        if g and g.tune then deckard.set_tune(s.group, nil, g.tune + by(steps, fine)) end
      end,
    },
    {
      name = "Sound",
      text = function(s) return text(focused_sound(s)) end,
      turn = function(s, steps, fine)
        local x = focused_sound(s)
        if x and x.tune then deckard.set_tune(s.group, s.sound, x.tune + by(steps, fine)) end
      end,
    },
  })
end

local function perform_menu()
  local function mode_of(s)
    local g = focused_group(s)
    return g and g.perform and g.perform.mode and g.perform.mode - 1 or 0
  end
  open_menu("PERFORM FX", {
    list_item("Effect", PERFORM_MODES, mode_of, function(to) deckard.perform_mode(deckard.state().group, to + 1) end),
  })
end

-- ---- the library ----------------------------------------------------------------------

local function browser_line()
  local b = st.browser
  if not b then return end
  local lib = deckard.state().browser
  if not lib then return end
  local kind = KINDS[lib.kind] or "?"
  local text
  if b.section == "kind" then
    text = b.user_place and ("Content: " .. (lib.user and "User" or "Factory")) or ("Type of file: " .. kind)
  elseif b.section == "favorites" then
    text = "Favorites: " .. (lib.favorites and "On" or "Off")
  elseif b.section == "results" then
    local r = lib.results[lib.focus]
    text = string.format("%d/%d %s%s", #lib.results == 0 and 0 or lib.focus, #lib.results, r and r.name or "(none)", r and r.favorite and " *" or "")
  else
    local a = lib.attributes[b.section]
    local name = ({ "Types", "Sub-types", "Modes" })[b.section]
    text = name .. ": " .. ((a and a.chosen and a.values[a.chosen]) or "All")
  end
  say("LIBRARY " .. kind .. " ▸ " .. text)
end

local function browser_turn(steps)
  local b, s = st.browser, deckard.state()
  local lib = s.browser
  if not lib then return end
  if b.section == "kind" then
    if b.user_place then
      if steps < 0 then b.user_place = false end
    elseif lib.kind + steps > #KINDS then
      b.user_place = true
    else
      deckard.browse_kind(clamp(lib.kind + steps, 1, #KINDS))
    end
  elseif b.section == "results" then
    deckard.browse_focus(steps * (shift() and 10 or 1))
  elseif type(b.section) == "number" then
    local a = lib.attributes[b.section]
    if a then
      local at = clamp((a.chosen or 0) + steps, 0, #a.values)
      deckard.browse_attribute(b.section, at > 0 and at or nil)
    end
  end
end

local function browser_press()
  local b, s = st.browser, deckard.state()
  local lib = s.browser
  if not lib then return end
  if b.section == "results" then
    if shift() then deckard.browse_set_favorite() else deckard.browse_load() end
  elseif b.section == "favorites" then
    deckard.browse_favorites()
  elseif b.section == "kind" and b.user_place then
    deckard.browse_user(not lib.user)
  else
    -- (Filters apply as they're turned: pressing goes on to what they found.)
    b.section = "results"
  end
end

-- The D-pad, while the library has it: true if it took the press.
local function browser_dpad(name)
  local b = st.browser
  if not b then return false end
  local lib = deckard.state().browser
  if shift() then
    if name == "LEFT" then
      -- Reset the filters, keeping them for SHIFT + right.
      if lib then
        b.saved = {}
        for a = 1, 3 do b.saved[a] = lib.attributes[a] and lib.attributes[a].chosen end
      end
      for a = 1, 3 do deckard.browse_attribute(a, nil) end
      deckard.browse_product(nil)
      b.section = "kind"
    elseif name == "RIGHT" and b.saved then
      for a = 1, 3 do deckard.browse_attribute(a, b.saved[a]) end
    elseif name == "UP" or name == "DOWN" then
      deckard.browse_focus(name == "UP" and -1 or 1)
      deckard.browse_load()
    end
    return true
  end
  local filters = { "kind", 1, 2, 3 }
  if name == "RIGHT" then
    b.section = "results"
  elseif name == "LEFT" then
    if b.section == "results" or b.section == "favorites" then b.section = "kind" end
  elseif b.section == "results" then
    if name == "UP" then b.section = "favorites" end
  elseif b.section == "favorites" then
    if name == "DOWN" then b.section = "results" end
  else
    for i, f in ipairs(filters) do
      if f == b.section then
        b.section = filters[clamp(i + (name == "DOWN" and 1 or -1), 1, #filters)]
        break
      end
    end
  end
  return true
end

local function toggle_browser()
  if st.browser then
    st.browser = nil
    deckard.library(false)
    say("Library closed")
  else
    close_menu()
    st.browser = { section = "kind", user_place = false }
    deckard.library(true)
  end
end

-- ---- what the strips do ---------------------------------------------------------------

-- The channel strip `n` works on: a group, and a sound if they're sounds.
local function strip_channel(s, n)
  if st.scope == "sounds" then
    local g = focused_group(s)
    local sound = st.sound_page * 8 + n
    return s.group, sound, g and g.sounds[sound]
  end
  local group = st.group_bank * 8 + n
  return group, nil, s.groups[group]
end

-- Strip `n`'s value (0..1), its colour and whether it reads from the middle; nil with nothing
-- to control.
local function strip_value(s, n)
  local mode = st.strip
  if mode == "control" or mode == "macro" then
    local knob = s.control.knobs[n]
    if not knob then return nil end
    local g = focused_group(s)
    return knob.value, g and g.palette or WHITE, false
  end
  if mode == "perform" then
    local g = focused_group(s)
    if not (g and g.perform) then return nil end
    return g.perform.pos, g.palette, false
  end
  local _, _, c = strip_channel(s, n)
  if not c then return nil end
  if mode == "level" then
    return deckard.db_to_level(c.level), c.palette, false
  elseif mode == "pan" then
    return (c.pan + 1) / 2, c.palette, true
  elseif mode == "aux" then
    return c.aux[st.aux], c.palette, false
  elseif mode == "tune" then
    if not c.tune then return nil end
    return (c.tune + 36) / 72, c.palette, true
  elseif mode == "swing" then
    return c.swing / 100, c.palette, false
  end
  return nil
end

-- Strip `n` set to `v` (0..1).
local function strip_set(s, n, v)
  local mode = st.strip
  if mode == "control" or mode == "macro" then
    local knob = s.control.knobs[n]
    if knob then deckard.turn_knob(n, v - knob.value) end
    return
  end
  local group, sound, c = strip_channel(s, n)
  if not c then return end
  if mode == "level" then
    deckard.set_level(group, sound, deckard.level_to_db(v))
  elseif mode == "pan" then
    deckard.set_pan(group, sound, v * 2 - 1)
  elseif mode == "aux" then
    deckard.set_send(group, sound, st.aux, v)
  elseif mode == "tune" then
    local semis = v * 72 - 36
    deckard.set_tune(group, sound, shift() and semis or math.floor(semis + 0.5))
  elseif mode == "swing" then
    deckard.set_swing(group, sound, v * 100)
  end
end

-- Strip `n` back to its default (a double tap).
local function strip_reset(s, n)
  local mode = st.strip
  if mode == "control" or mode == "macro" then
    deckard.reset_knob(n)
    return
  end
  local group, sound, c = strip_channel(s, n)
  if not c then return end
  if mode == "level" then
    deckard.set_level(group, sound, 0)
  elseif mode == "pan" then
    deckard.set_pan(group, sound, 0)
  elseif mode == "aux" then
    deckard.set_send(group, sound, st.aux, 0)
  elseif mode == "tune" then
    deckard.set_tune(group, sound, 0)
  elseif mode == "swing" then
    deckard.set_swing(group, sound, 0)
  end
end

-- Notes mode: strip `n`'s notes, low to high.
local function strum_notes(s, n)
  local set = st.notes.sets[st.notes.mode][n] or {}
  local degrees = {}
  for d in pairs(set) do degrees[#degrees + 1] = d end
  table.sort(degrees)
  local base = keyboard_base(s) + 12
  local out = {}
  for _, d in ipairs(degrees) do
    local note = degree_note(s, base, d)
    if note >= 0 and note <= 127 then out[#out + 1] = note end
  end
  return out
end

local function strum_stop(key)
  local playing = st.notes.sounding[key]
  if not playing then return end
  st.notes.sounding[key] = nil
  for _, p in ipairs(playing.pitches) do deckard.strip_note(playing.sound, p, playing.velocity, false) end
end

local function strum(s, key, n, pos)
  local notes = strum_notes(s, n)
  if #notes == 0 then return end
  local note = notes[math.min(#notes, math.floor(pos * #notes) + 1)]
  local playing = st.notes.sounding[key]
  if playing and playing.note == note then return end
  strum_stop(key)
  local velocity = st.velocity
  local pitches = chord_of(s, note)
  st.notes.sounding[key] = { note = note, sound = s.sound, pitches = pitches, velocity = velocity }
  for _, p in ipairs(pitches) do deckard.strip_note(s.sound, p, velocity, true) end
end

local function set_strip(mode)
  st.strip = mode
  if mode == "control" then
    deckard.show_plugins(true)
  elseif mode == "macro" then
    deckard.show_plugins(false)
    deckard.page_group(4)
  end
end

-- ---- pressing things ------------------------------------------------------------------

local function tap()
  local now = deckard.now()
  if #st.taps > 0 and now - st.taps[#st.taps] > TAP_RESET then st.taps = {} end
  st.taps[#st.taps + 1] = now
  if #st.taps > 5 then table.remove(st.taps, 1) end
  if #st.taps >= 2 then
    local beat = (st.taps[#st.taps] - st.taps[1]) / (#st.taps - 1)
    if beat > 0 then deckard.tempo(math.floor(60 / beat * 100 + 0.5) / 100) end
  end
end

-- DUPLICATE: the first press takes the source; each later press copies it there and the copy
-- is the source (manual 7.5).
local function duplicate(kind, group, at, copy)
  local d = st.dup
  if d and d.kind == kind and d.group == group then
    if d.at ~= at then
      copy(d.at, at)
      st.dup.at = at
    end
  else
    st.dup = { kind = kind, group = group, at = at }
  end
end

local function dup_is(kind, group, at)
  local d = st.dup
  return d ~= nil and d.kind == kind and d.group == group and d.at == at
end

-- A sound's pad, while the matrix shows the sounds.
local function sound_action(s, n)
  local g = focused_group(s)
  if not g or n > #g.sounds then return end
  if active("MUTE") then
    deckard.mute_sound(n)
  elseif active("SOLO") then
    deckard.solo_sound(n)
  elseif active("SELECT") then
    deckard.focus_sound(n)
  else
    -- A channel mode's button: the strips show this sound's eight (manual 8.1.1.1).
    deckard.focus_sound(n)
    st.scope = "sounds"
    st.sound_page = (n - 1) // 8
    local name = mode_held()
    if name == "CONTROL" or name == "MACRO" then deckard.level(3) end
  end
end

-- Which of a group's patterns is in place `place` of its column (a place can be empty):
-- its number in the group's list, and the pattern.
local function pattern_in(group, place)
  for i, pattern in ipairs(group.patterns) do
    if pattern.slot == place then return i, pattern end
  end
end

local function press_project(s, col, row)
  local g = st.group_bank * 8 + col
  local group = s.groups[g]
  if not group then return end
  local place = st.pattern_row + row
  local p = pattern_in(group, place)
  if active("CLEAR") then
    if p then deckard.delete_pattern(g, p) end
  elseif active("DUPLICATE") then
    if p then
      duplicate("pattern", g, p, function(from, to) deckard.copy_pattern(g, from, to) end)
    elseif st.dup and st.dup.kind == "pattern" and st.dup.group == g then
      -- (An empty place: a new pattern, after the group's last.)
      duplicate("pattern", g, #group.patterns + 1, function(from, to) deckard.copy_pattern(g, from, to) end)
    end
  elseif not p then
    -- An unlit pad makes a pattern there (manual 6.1.3).
    if g ~= s.group and not s.keep_focus then deckard.focus_group(g) end
    deckard.new_pattern(g, place)
  elseif group.scene_pattern == p then
    deckard.scene_pattern(g, nil)
  elseif g ~= s.group and s.keep_focus then
    deckard.scene_pattern(g, p)
  else
    if g ~= s.group then deckard.focus_group(g) end
    deckard.pick_pattern(p)
  end
end

local function press_pad_view(s, col, row)
  local n = sound_at(col, row)
  if not n then return end
  local g = focused_group(s)
  if not g or n > #g.sounds then return end
  if active("CLEAR") then
    deckard.reset_sound(n)
  elseif active("DUPLICATE") then
    duplicate("sound", s.group, n, function(from, to) deckard.copy_sound(from, to) end)
  else
    deckard.focus_sound(n)
  end
end

local function press_step(s, col, row)
  local sound, index = step_cell(s, col, row)
  local at = step_at(s, index)
  local g = focused_group(s)
  if not g or sound > #g.sounds or not step_in_pattern(s, at) then return end
  -- (A lane of another sound: the step is that sound's, and the focus stays.)
  if sound ~= s.sound then deckard.focus_sound(sound) end
  deckard.step(at, step_ticks(s), step_velocity())
  if sound ~= s.sound then deckard.focus_sound(s.sound) end
end

local function press_roll(s, col, row)
  local note = roll_note(s, row)
  if note < 0 or note > 127 then return end
  local at = step_at(s, st.step_page * 8 + col - 1)
  if not step_in_pattern(s, at) then return end
  deckard.step_notes(at, step_ticks(s), step_velocity(), { note })
end

local function press_length(s, col, row)
  deckard.pattern_length(pad_key(col, row) * bar_ticks(s))
end

local function press_lock(s, col, row)
  local slot = pad_key(col, row)
  if active("CLEAR") then
    if s.lock.slots[slot] then deckard.lock("delete_slot", slot) end
  else
    deckard.lock("slot", slot)
  end
end

local function press_notes(_, col, row)
  local set = st.notes.sets[st.notes.mode][col]
  local d = st.notes.low + (8 - row)
  set[d] = (not set[d]) or nil
end

local VIEW_PRESS = {
  song = press_project,
  pad = press_pad_view,
  step = press_step,
  roll = press_roll,
  length = press_length,
  lock = press_lock,
  notes = press_notes,
}

-- SHIFT + the top row: what's printed over it.
local function shift_pad(col, row)
  if row ~= 1 then return end
  if col == 1 then deckard.edit("undo_step")
  elseif col == 2 then deckard.edit("redo_step")
  elseif col == 3 then deckard.events("quantize", 0)
  elseif col == 4 then deckard.events("quantize_half", 0)
  elseif col == 5 then deckard.events("transpose", -1)
  elseif col == 6 then deckard.events("transpose", 1)
  elseif col == 7 then deckard.events("transpose", -12)
  elseif col == 8 then deckard.events("transpose", 12)
  end
end

-- GRID held: the four common step grids.
local function pick_step_grid(n)
  if n >= 1 and n <= 4 then deckard.grid("step", n + 1) end
end

function M.pad(col, row, down)
  st.pressed[pad_key(col, row)] = down or nil
  if not down then return end
  local s = deckard.state()
  if shift() then
    shift_pad(col, row)
    return
  end
  local n = sound_at(col, row)
  if held("GRID") then
    if n then pick_step_grid(n) end
    return
  end
  if sounds_shown() then
    if n then sound_action(s, n) end
    return
  end
  local press = VIEW_PRESS[st.view]
  if press then press(s, col, row) end
end

local function scene_button(s, n)
  if held("STEP") then
    if n == 1 or n == 4 or n == 8 then
      st.lanes = n
      st.step_page = 0
      st.step_used = true
    end
    return
  end
  if held("NOTES") then
    if n <= 3 then
      st.notes.mode = n
      st.notes_used = true
      say("Notes: " .. NOTES_MODES[n])
    end
    return
  end
  if held("GRID") then
    pick_step_grid(n)
    return
  end
  if shift() then
    -- A bank of eight scenes: the ones there are, and the place after them.
    local first = (n - 1) * 8
    if first <= #s.scenes then
      st.scene_bank = first
      deckard.show_scenes(first + 1)
    end
    return
  end
  if st.view == "step" or st.view == "roll" then
    st.step_page = clamp(n - 1, 0, parts(s) - 1)
    return
  end
  local sc = st.scene_bank + n
  local exists = s.scenes[sc] ~= nil
  if active("CLEAR") then
    if exists then deckard.delete_scene(sc) end
  elseif active("DUPLICATE") then
    if exists or (st.dup and st.dup.kind == "scene" and sc == #s.scenes + 1) then
      duplicate("scene", 0, sc, function(from, to) deckard.copy_scene(from, to) end)
    end
  elseif exists then
    deckard.focus_scene(sc)
  elseif sc == #s.scenes + 1 then
    deckard.scene_edit("create")
  end
end

local function group_button(s, i)
  if shift() then
    if i == 1 then
      toggle_follow(s)
    elseif i == 5 then
      -- VARIATION: the focused sound's Groove ▸ Variation, on the strips.
      st.strip = "control"
      deckard.groove_page(2)
      say("Variation: on the strips")
    elseif i == 7 then
      deckard.file("save")
    end
    return
  end
  local g = st.group_bank * 8 + i
  if g > #s.groups then
    if g ~= #s.groups + 1 then return end
    if active("DUPLICATE") then
      if st.dup and st.dup.kind == "group" then
        duplicate("group", 0, g, function(from, to) deckard.copy_group(from, to) end)
      end
    elseif not (active("MUTE") or active("SOLO") or active("CLEAR")) then
      deckard.add_group()
    end
    return
  end
  if active("CLEAR") then
    deckard.delete_group(g)
  elseif active("DUPLICATE") then
    duplicate("group", 0, g, function(from, to) deckard.copy_group(from, to) end)
  elseif active("MUTE") then
    deckard.mute_group(g)
  elseif active("SOLO") then
    deckard.solo_group(g)
  else
    deckard.focus_group(g)
    local name = mode_held()
    if name then
      st.scope = "groups"
      if name == "CONTROL" or name == "MACRO" then deckard.level(2) end
    end
  end
end

local function move_group_bank(s, by)
  local to = st.group_bank + by
  if to >= 0 and to * 8 <= #s.groups then st.group_bank = to end
end

local function dpad(s, name)
  if browser_dpad(name) then
    browser_line()
    return
  end
  -- CONTROL held: up and down step through the plug-ins, past the last to the "+".
  if held("CONTROL") then
    if name == "UP" then deckard.slot(-1) elseif name == "DOWN" then deckard.slot(1) end
    return
  end
  if name == "LEFT" or name == "RIGHT" then
    move_group_bank(s, name == "LEFT" and -1 or 1)
    return
  end
  local by = name == "UP" and -1 or 1
  if st.view == "song" then
    st.pattern_row = math.max(0, st.pattern_row + by)
  elseif st.view == "keyboard" then
    st.octave = clamp(st.octave - by, -5, 4)
  elseif st.view == "roll" then
    st.roll_low = math.max(-48, st.roll_low - by)
  elseif st.view == "notes" then
    st.notes.low = clamp(st.notes.low - by, -14, 28)
  end
end

local function meter_button(name)
  close_menu()
  st.tempo = false
  if name == "MST" then
    st.meter = "mst"
    if st.strip == "control" or st.strip == "macro" or mode_held() then deckard.level(1) end
  elseif name == "GRP" then
    st.meter = "grp"
  elseif name == "IN1" then
    st.meter = shift() and "in2" or "in1"
  elseif name == "CUE" and not shift() then
    st.meter = "cue"
  end
  local watching = st.meter == "in1" or st.meter == "in2"
  if watching ~= (st.watching or false) then
    st.watching = watching
    deckard.watch_input(watching)
  end
end

-- A channel mode's button.
local function channel_mode(name)
  if name == "LEVEL" then
    set_strip(shift() and "pan" or "level")
  elseif name == "AUX" then
    st.aux = shift() and 2 or 1
    set_strip("aux")
  elseif name == "SWING" then
    set_strip("swing")
    swing_menu()
  elseif name == "TUNE" then
    set_strip("tune")
    tune_menu()
  else
    set_strip(CHANNEL_MODES[name])
  end
end

local function step_button(down)
  if down then
    if shift() then
      if st.view == "roll" then set_view(st.back) else set_view("roll") end
      st.step_entered = true
      return
    end
    st.step_used = false
    st.step_entered = st.view ~= "step"
    if st.step_entered then set_view("step") end
  elseif not st.step_entered and not st.step_used and st.view == "step" then
    set_view(st.back)
  end
end

local function temporary(name, down)
  if not down then
    if name == "DUPLICATE" and not st.latched.DUPLICATE then st.dup = nil end
    return
  end
  -- Pinned: each press turns it on or off (manual 4.3).
  if st.pins[name] then
    st.latched[name] = (not st.latched[name]) or nil
    if name == "DUPLICATE" and not st.latched.DUPLICATE then st.dup = nil end
  end
end

function M.button(name, down)
  local s = deckard.state()

  local scene = tonumber(name:match("^SCENE_(%d)$"))
  if scene then
    if down then scene_button(s, scene) end
    return
  end
  local letter = name:match("^GROUP_(%u)$")
  if letter then
    if down then group_button(s, string.byte(letter) - string.byte("A") + 1) end
    return
  end

  if TEMPORARY[name] then
    if down and shift() then
      if name == "SOLO" then
        if st.view == "length" then set_view(st.back) else set_view("length") end
      elseif name == "MUTE" then
        deckard.choke_all()
      elseif name == "SELECT" then
        st.accent = not st.accent
        say("Accent: " .. (st.accent and "On" or "Off"))
      elseif name == "DUPLICATE" then
        deckard.pattern_edit("double")
      elseif name == "CLEAR" then
        deckard.edit("clear_modulation")
      end
      return
    end
    temporary(name, down)
    return
  end

  if name == "AUTO" then
    -- Held, the strips write modulation (manual 8.5). (SHIFT + AUTO + CLEAR: CLEAR's.)
    deckard.auto_write(down)
    return
  end

  if name == "STEP" then
    step_button(down)
    return
  end

  if name == "NOTES" then
    -- Let go: Notes mode on or off; NOTES + 1, 2 or 3 picks a set and stays on.
    if down then
      st.notes_used = false
    elseif st.view == "notes" and not st.notes_used then
      set_view(st.back)
    else
      set_strip("notes")
      set_view("notes")
    end
    return
  end

  if CHANNEL_MODES[name] then
    if down then channel_mode(name) end
    return
  end

  if not down then return end

  if name == "SONG" then
    -- A temporary mode held: pinned, or no longer (manual 4.3).
    for mode in pairs(TEMPORARY) do
      if held(mode) then
        st.pins[mode] = (not st.pins[mode]) or nil
        st.latched[mode] = st.pins[mode]
        say(mode .. (st.pins[mode] and " pinned" or " unpinned"))
        return
      end
    end
    close_menu()
    set_view("song")
  elseif name == "PAD_MODE" then
    set_view(shift() and "keyboard" or "pad")
  elseif name == "NOTE_REPEAT" then
    if shift() then
      st.arp = not st.arp
      st.repeating = st.arp
      if st.arp then set_view("keyboard") end
    else
      st.repeating = not st.repeating
      st.arp = false
      if st.repeating and st.view ~= "keyboard" then set_view("pad") end
    end
    deckard.note_repeat("on", st.repeating)
    say((st.arp and "Arp: " or "Note Repeat: ") .. (st.repeating and "On" or "Off"))
  elseif name == "UP" or name == "DOWN" or name == "LEFT" or name == "RIGHT" then
    dpad(s, name)
  elseif name == "MST" or name == "GRP" or name == "IN1" or name == "CUE" then
    meter_button(name)
  elseif name == "BROWSE" then
    toggle_browser()
  elseif name == "ENCODER" then
    if st.browser then
      browser_press()
      browser_line()
    elseif st.menu then
      menu_press()
    end
  elseif name == "ENCODER_TOUCH" then
    if st.browser or st.menu then
      return
    elseif st.repeating then
      repeat_menu({ idle = true })
    elseif st.view == "lock" then
      lock_menu({ idle = true })
    elseif st.view == "keyboard" or st.view == "roll" or st.view == "notes" then
      scale_menu({ idle = true })
    end
  elseif name == "PERFORM" then
    set_strip("perform")
    if shift() then
      -- (One goes in if the group has none, shown on the Control page.)
      deckard.perform_fx()
      perform_menu()
    end
  elseif name == "LOCK" then
    if shift() then
      if st.view == "lock" then set_view(st.back) else set_view("lock") end
    elseif st.view == "lock" then
      if s.lock.focus then deckard.lock("update") end
    else
      deckard.lock("button")
    end
  elseif name == "PLAY" or name == "FOOT_TIP" then
    if name == "PLAY" and shift() then
      deckard.restart()
    elseif s.playing then
      deckard.stop()
    else
      deckard.play()
    end
  elseif name == "REC" or name == "FOOT_RING" then
    deckard.record(not s.recording, name == "REC" and shift() and not s.recording)
  elseif name == "PAGE_LEFT" or name == "PAGE_RIGHT" then
    local left = name == "PAGE_LEFT"
    if shift() then
      if left then deckard.metronome(not s.metronome) else deckard.set_loop(not s.looping) end
    elseif st.strip == "control" or st.strip == "macro" then
      if left then deckard.previous_page() else deckard.next_page() end
    elseif st.scope == "sounds" then
      st.sound_page = left and 0 or 1
    end
  elseif name == "TEMPO" then
    if shift() then
      tap()
    else
      close_menu()
      st.tempo = not st.tempo
      if st.tempo then say(string.format("Tempo %.1f", s.tempo)) end
    end
  elseif name == "GRID" then
    if st.menu and (st.menu.title == "GRID" or st.menu.title == "REC MODE") then
      close_menu()
    elseif shift() then
      rec_mode_menu()
    else
      grid_menu()
    end
  end
end

function M.encoder(steps)
  local s = deckard.state()
  if st.browser then
    browser_turn(steps)
    return
  end
  if st.menu then
    menu_turn(steps)
    return
  end
  local fine = shift()
  if st.tempo then
    local to = math.floor((s.tempo + steps * (fine and 0.1 or 1)) * 10 + 0.5) / 10
    deckard.tempo(to)
    say(string.format("Tempo %.1f", to))
    return
  end
  local by = steps * (fine and 0.05 or 0.5)
  if st.meter == "mst" then
    local to = math.max(s.volume_db, -120) + by
    deckard.volume_db(to)
    say(string.format("Master %.1f dB", to))
  elseif st.meter == "grp" then
    local g = focused_group(s)
    if g then
      local to = math.max(g.level, -120) + by
      deckard.set_level(s.group, nil, to)
      say(string.format("%s %.1f dB", g.name, to))
    end
  elseif st.meter == "cue" then
    local to = math.max(deckard.level_to_db(s.cue_level), -120) + by
    deckard.cue_db(to)
    say(string.format("Cue %.1f dB", to))
  else
    local n = st.meter == "in2" and 2 or 1
    local to = math.max(deckard.level_to_db(s.input_levels[n]), -120) + by
    deckard.input_db(n, to)
    say(string.format("In %d %.1f dB", n, to))
  end
end

-- Relative touchstrips: how far a value moves for the finger's travel along the strip.
-- Tune spans 72 semitones, so it takes less.
local RELATIVE_RATE = { tune = 0.25 }
local RELATIVE_RATE_DEFAULT = 0.35

function M.strip(n, finger, kind, pos)
  local s = deckard.state()
  local mode = st.strip
  if mode == "notes" then
    local key = n .. ":" .. finger
    if kind == "up" then strum_stop(key) else strum(s, key, n, pos) end
    local rec = st.strips[n] or {}
    rec.touched = kind ~= "up"
    rec.pos = pos
    st.strips[n] = rec
    return
  end
  -- (The others follow the first finger.)
  if finger ~= 1 then return end
  local rec = st.strips[n] or {}
  st.strips[n] = rec
  if mode == "perform" then
    local g = focused_group(s)
    rec.touched = kind ~= "up"
    rec.pos = pos
    if g and g.perform then
      local what = kind == "down" and "perform_touch" or kind == "up" and "perform_release" or "perform"
      deckard.strip(what, pos)
    end
    return
  end
  if kind == "down" then
    local now = deckard.now()
    rec.tapped = rec.last_down ~= nil and now - rec.last_down < DOUBLE_TAP
    rec.last_down = now
    rec.touched = true
    rec.pos = pos
    if rec.tapped then
      strip_reset(s, n)
      return
    end
    if s.relative_strips then
      -- A touch only picks the value up; the travel moves it.
      rec.value = strip_value(s, n)
    elseif not shift() then
      strip_set(s, n, pos)
    end
  elseif kind == "move" then
    if rec.tapped then return end
    if s.relative_strips then
      if rec.value then
        local rate = RELATIVE_RATE[mode] or RELATIVE_RATE_DEFAULT
        rec.value = clamp(rec.value + (pos - (rec.pos or pos)) * rate * (shift() and 0.1 or 1), 0, 1)
        strip_set(s, n, rec.value)
      end
    elseif shift() then
      -- SHIFT: the same travel moves it a tenth as far.
      local v = strip_value(s, n)
      if v then strip_set(s, n, clamp(v + (pos - (rec.pos or pos)) * 0.1, 0, 1)) end
    else
      strip_set(s, n, pos)
    end
    rec.pos = pos
  else
    rec.touched = false
    rec.tapped = false
  end
end

-- ---- the timer ------------------------------------------------------------------------

function M.tick()
  local s = deckard.state()
  if s.group ~= st.last_group then
    st.last_group = s.group
    st.group_bank = (s.group - 1) // 8
  end
  follow_steps(s)
  if st.menu then
    if st.menu.idle and not held("ENCODER_TOUCH") and deckard.now() - st.menu.used > MENU_IDLE then
      close_menu()
    else
      menu_line()
    end
  elseif st.browser then
    browser_line()
  end
  -- Touching a Control strip with AUTO held writes where it is, moving or not.
  if s.auto_write and s.playing and (st.strip == "control" or st.strip == "macro") then
    for n = 1, 8 do
      if st.strips[n] and st.strips[n].touched then deckard.touch_write(n) end
    end
  end
end

-- ---- lights ---------------------------------------------------------------------------

local function light_sounds(s)
  local g = focused_group(s)
  local muting = active("MUTE") or active("SOLO")
  for n = 1, 16 do
    local col, row = place_of(n)
    local x = g and g.sounds[n]
    if not x then
      deckard.pad_led(col, row, "off")
    elseif muting then
      deckard.pad_led(col, row, x.palette, not x.muted)
    elseif dup_is("sound", s.group, n) then
      deckard.pad_led(col, row, blink() and x.palette or "off", true)
    elseif st.pressed[pad_key(col, row)] then
      deckard.pad_led(col, row, x.palette, "pale")
    elseif n == s.sound then
      deckard.pad_led(col, row, x.used and x.palette or WHITE, true)
    elseif x.used then
      deckard.pad_led(col, row, x.palette, x.playing == true)
    else
      deckard.pad_led(col, row, "off")
    end
  end
end

local function light_project(s)
  local coming = s.queued_scene and s.scenes[s.queued_scene]
  for col = 1, 8 do
    local g = st.group_bank * 8 + col
    local group = s.groups[g]
    for row = 1, 8 do
      local p, pattern
      if group then p, pattern = pattern_in(group, st.pattern_row + row) end
      if not pattern then
        deckard.pad_led(col, row, "off")
      elseif dup_is("pattern", g, p) then
        deckard.pad_led(col, row, blink() and pattern.palette or "off", true)
      elseif coming and coming.patterns[g] == p then
        deckard.pad_led(col, row, pattern.palette, beat_flash(s))
      elseif group.scene_pattern == p then
        deckard.pad_led(col, row, pattern.palette, not group.muted)
      else
        deckard.pad_led(col, row, pattern.palette, "faint")
      end
    end
  end
end

local function light_keyboard(s)
  local x = focused_sound(s)
  local palette = x and x.palette or WHITE
  for col = 1, 8 do
    for row = 1, 8 do
      local note = key_note(s, col, row)
      if not note then
        deckard.pad_led(col, row, "off")
      elseif st.pressed[pad_key(col, row)] then
        deckard.pad_led(col, row, palette, "pale")
      elseif is_root(s, note) then
        deckard.pad_led(col, row, WHITE, true)
      else
        deckard.pad_led(col, row, palette, false)
      end
    end
  end
end

local function note_in(s, sound, at, length, pitch)
  for _, n in ipairs(s.notes) do
    if n.sound == sound and n.position >= at and n.position < at + length and (not pitch or n.pitch == pitch) then return n end
  end
  return nil
end

local function light_steps(s)
  local g = focused_group(s)
  local now = playing_step(s)
  local length = step_ticks(s)
  for col = 1, 8 do
    for row = 1, 8 do
      local sound, index = step_cell(s, col, row)
      local at = step_at(s, index)
      local x = g and g.sounds[sound]
      if now == index and step_in_pattern(s, at) then
        deckard.pad_led(col, row, WHITE, true)
      elseif not x or not step_in_pattern(s, at) then
        deckard.pad_led(col, row, "off")
      else
        local n = note_in(s, sound, at, length)
        if not n then
          deckard.pad_led(col, row, x.palette, "faint")
        elseif n.velocity >= ACCENT_FROM then
          deckard.pad_led(col, row, x.palette, "pale")
        else
          deckard.pad_led(col, row, x.palette, true)
        end
      end
    end
  end
end

local function light_roll(s)
  local x = focused_sound(s)
  local palette = x and x.palette or WHITE
  local now = playing_step(s)
  local length = step_ticks(s)
  for col = 1, 8 do
    local index = st.step_page * 8 + col - 1
    local at = step_at(s, index)
    local inside = step_in_pattern(s, at)
    for row = 1, 8 do
      local pitch = roll_note(s, row)
      local n = inside and note_in(s, s.sound, at, length, pitch)
      if now == index and inside then
        deckard.pad_led(col, row, WHITE, n and true or false)
      elseif n then
        deckard.pad_led(col, row, palette, n.velocity >= ACCENT_FROM and "pale" or true)
      elseif inside and is_root(s, pitch) then
        deckard.pad_led(col, row, WHITE, "faint")
      elseif inside then
        deckard.pad_led(col, row, palette, "faint")
      else
        deckard.pad_led(col, row, "off")
      end
    end
  end
end

local function light_length(s)
  local g = focused_group(s)
  local palette = g and g.palette or WHITE
  local bars = math.max(1, math.ceil(s.pattern_length / bar_ticks(s)))
  for col = 1, 8 do
    for row = 1, 8 do
      local k = pad_key(col, row)
      if k == bars then
        deckard.pad_led(col, row, palette, true)
      elseif k < bars then
        deckard.pad_led(col, row, palette, false)
      else
        deckard.pad_led(col, row, "off")
      end
    end
  end
end

local function light_lock(s)
  for col = 1, 8 do
    for row = 1, 8 do
      local k = pad_key(col, row)
      if s.lock.focus == k then
        deckard.pad_led(col, row, WHITE, true)
      elseif s.lock.slots[k] then
        deckard.pad_led(col, row, WHITE, false)
      else
        deckard.pad_led(col, row, "off")
      end
    end
  end
end

local function light_notes(s)
  local x = focused_sound(s)
  local palette = x and x.palette or WHITE
  local set = st.notes.sets[st.notes.mode]
  local n = #scale_steps(s)
  for col = 1, 8 do
    for row = 1, 8 do
      local d = st.notes.low + (8 - row)
      if set[col] and set[col][d] then
        deckard.pad_led(col, row, d % n == 0 and WHITE or palette, true)
      elseif d % n == 0 then
        deckard.pad_led(col, row, WHITE, "faint")
      else
        deckard.pad_led(col, row, "off")
      end
    end
  end
end

local function all_pads_off()
  for col = 1, 8 do
    for row = 1, 8 do deckard.pad_led(col, row, "off") end
  end
end

local VIEW_LIGHTS = {
  song = light_project,
  keyboard = light_keyboard,
  step = light_steps,
  roll = light_roll,
  length = light_length,
  lock = light_lock,
  notes = light_notes,
}

local function light_matrix(s)
  if sounds_shown() or st.view == "pad" then
    all_pads_off()
    light_sounds(s)
  else
    VIEW_LIGHTS[st.view](s)
  end
  -- SHIFT: the shortcuts over the top row.
  if shift() then
    for col = 1, 8 do deckard.pad_led(col, 1, WHITE, false) end
  end
end

local function light_scenes(s)
  if held("STEP") then
    for n = 1, 8 do
      local on = n == 1 or n == 4 or n == 8
      deckard.led("SCENE_" .. n, on and WHITE or "off", st.lanes == n)
    end
    return
  end
  if held("NOTES") then
    for n = 1, 8 do deckard.led("SCENE_" .. n, n <= 3 and WHITE or "off", st.notes.mode == n) end
    return
  end
  if held("GRID") then
    for n = 1, 8 do deckard.led("SCENE_" .. n, n <= 4 and WHITE or "off", s.grid.step == n + 1) end
    return
  end
  if st.view == "step" or st.view == "roll" then
    local shown = parts(s)
    for n = 1, 8 do
      if n > shown then
        deckard.led("SCENE_" .. n, "off")
      else
        deckard.led("SCENE_" .. n, WHITE, n == st.step_page + 1)
      end
    end
    return
  end
  for n = 1, 8 do
    local sc = st.scene_bank + n
    local scene = s.scenes[sc]
    local name = "SCENE_" .. n
    if not scene then
      if sc == #s.scenes + 1 then deckard.led(name, WHITE, "faint") else deckard.led(name, "off") end
    elseif dup_is("scene", 0, sc) then
      deckard.led(name, blink() and scene.palette or "off", true)
    elseif sc == s.queued_scene then
      deckard.led(name, beat_flash(s) and scene.palette or "off", true)
    else
      deckard.led(name, scene.palette, sc == s.scene or sc == s.playing_scene)
    end
  end
end

local function light_groups(s)
  local muting = active("MUTE") or active("SOLO")
  for i = 1, 8 do
    local g = st.group_bank * 8 + i
    local group = s.groups[g]
    local name = "GROUP_" .. group_letter(i)
    if not group then
      if g == #s.groups + 1 and not muting then deckard.led(name, WHITE, "faint") else deckard.led(name, "off") end
    elseif muting then
      deckard.led(name, group.palette, not group.muted)
    elseif dup_is("group", 0, g) then
      deckard.led(name, blink() and group.palette or "off", true)
    elseif g == s.group and mode_held() then
      -- Held, a mode's button makes the focused group blink (manual 8.1.1.1).
      deckard.led(name, blink() and group.palette or "off", true)
    else
      deckard.led(name, group.palette, g == s.group or (s.playing and group.playing == true))
    end
  end
end

local function light_strips(s)
  for n = 1, 8 do
    local rec = st.strips[n]
    local shown = {}
    local colour = WHITE
    if st.strip == "notes" then
      local x = focused_sound(s)
      colour = x and x.palette or WHITE
      if rec and rec.touched then shown[clamp(math.floor(rec.pos * (STRIP_LEDS - 1) + 0.5) + 1, 1, STRIP_LEDS)] = true end
    else
      local v, palette, centred = strip_value(s, n)
      if v then
        colour = palette or WHITE
        local at = clamp(math.floor(clamp(v, 0, 1) * (STRIP_LEDS - 1) + 0.5) + 1, 1, STRIP_LEDS)
        if centred then
          local middle = (STRIP_LEDS + 1) // 2
          for i = math.min(at, middle), math.max(at, middle) do shown[i] = true end
        elseif v > 0 then
          for i = 1, at do shown[i] = true end
        end
      end
    end
    for led = 1, STRIP_LEDS do
      deckard.strip_led(n, led, shown[led] and colour or "off", true)
    end
  end
end

local SILENT = { -math.huge, -math.huge }

local function meter_db(s)
  if st.meter == "mst" then return s.master_meter end
  if st.meter == "grp" then
    local g = focused_group(s)
    return g and g.meter or SILENT
  end
  if st.meter == "in1" then return s.input_meters[1] end
  if st.meter == "in2" then return s.input_meters[2] end
  if st.meter == "cue" then return s.cue_meter end
  return SILENT
end

local function light_meters(s)
  local m = meter_db(s)
  for col = 1, 2 do
    local db = math.max(m[col], METER_FLOOR)
    local count = clamp(math.floor((db - METER_FLOOR) / -METER_FLOOR * METER_LEDS + 0.5), 0, METER_LEDS)
    for led = 1, METER_LEDS do deckard.meter(col, led, led <= count and 1 or 0) end
  end
end

local function lit(on)
  return on and "on" or "off"
end

local function light_buttons(s)
  local sh = shift()
  deckard.led("SONG", lit(st.view == "song"))
  deckard.led("STEP", lit(st.view == "step" or st.view == "roll"))
  deckard.led("PAD_MODE", lit(st.view == "pad" or st.view == "keyboard"))
  deckard.led("CLEAR", lit(active("CLEAR")))
  deckard.led("DUPLICATE", lit(active("DUPLICATE")))
  deckard.led("NOTE_REPEAT", lit(st.repeating))
  deckard.led("MST", lit(st.meter == "mst"))
  deckard.led("GRP", lit(st.meter == "grp"))
  deckard.led("IN1", lit(st.meter == "in1"))
  deckard.led("IN2", lit(st.meter == "in2"))
  deckard.led("CUE", lit(st.meter == "cue"))
  deckard.led("BROWSE", lit(st.browser ~= nil))
  for name, modes in pairs(MODE_LIGHTS) do deckard.led(name, lit(modes[st.strip])) end
  deckard.led("AUTO", lit(s.auto_write))
  deckard.led("LOCK", st.view == "lock" and "on" or (s.lock.active and "dim" or "off"))
  deckard.led("SHIFT", lit(sh))
  deckard.led("PLAY", lit(s.playing))
  deckard.led("REC", lit(s.recording))
  deckard.led("TEMPO", lit(st.tempo))
  deckard.led("GRID", lit(st.menu ~= nil and (st.menu.title == "GRID" or st.menu.title == "REC MODE")))
  deckard.led("SOLO", lit(active("SOLO") or st.view == "length"))
  deckard.led("MUTE", lit(active("MUTE")))
  deckard.led("SELECT", lit(active("SELECT") or st.accent))
  -- The page buttons: their SHIFT functions, else whether there's a page that way.
  local left, right = false, false
  if sh then
    deckard.led("PAGE_LEFT", lit(s.metronome))
    deckard.led("PAGE_RIGHT", lit(s.looping))
  else
    if st.strip == "control" or st.strip == "macro" then
      left, right = s.control.page > 1, s.control.page < #s.control.pages
    elseif st.scope == "sounds" then
      left, right = st.sound_page > 0, st.sound_page < 1
    end
    deckard.led("PAGE_LEFT", left and "dim" or "off")
    deckard.led("PAGE_RIGHT", right and "dim" or "off")
  end
  -- The D-pad: which way there's something.
  local up, down = false, false
  if st.browser or held("CONTROL") or st.view == "keyboard" or st.view == "roll" or st.view == "notes" then
    up, down = true, true
  elseif st.view == "song" then
    local most = 0
    for col = 1, 8 do
      local group = s.groups[st.group_bank * 8 + col]
      local last = group and group.patterns[#group.patterns]
      if last then most = math.max(most, last.slot) end
    end
    up, down = st.pattern_row > 0, st.pattern_row + 8 < most + 1
  end
  deckard.led("UP", up and "dim" or "off")
  deckard.led("DOWN", down and "dim" or "off")
  deckard.led("LEFT", (st.browser or st.group_bank > 0) and "dim" or "off")
  deckard.led("RIGHT", (st.browser or (st.group_bank + 1) * 8 <= #s.groups) and "dim" or "off")
end

function M.leds()
  local s = deckard.state()
  light_matrix(s)
  light_scenes(s)
  light_groups(s)
  light_strips(s)
  light_meters(s)
  light_buttons(s)
end

-- ---- where the pads play --------------------------------------------------------------

function M.pads()
  if shift() or sounds_shown() or active("CLEAR") or active("DUPLICATE") or held("GRID") then return nil end
  local s = deckard.state()
  local out = { velocity = s.velocity > 0 and s.velocity or 100 }
  if st.view == "pad" then
    local g = focused_group(s)
    for n = 1, 16 do
      if g and g.sounds[n] then
        local col, row = place_of(n)
        out[pad_key(col, row)] = { s.group, n, PAD_PITCH }
      end
    end
  elseif st.view == "keyboard" then
    for col = 1, 8 do
      for row = 1, 8 do
        local note = key_note(s, col, row)
        if note then
          local entry = { s.group, s.sound }
          for _, p in ipairs(chord_of(s, note)) do entry[#entry + 1] = p end
          out[pad_key(col, row)] = entry
        end
      end
    end
  else
    return nil
  end
  return out
end

-- ---- opening and closing --------------------------------------------------------------

function M.open()
  st = fresh()
  local s = deckard.state()
  st.last_group = s.group
  st.group_bank = (s.group - 1) // 8
end

function M.close()
  for key in pairs(st.notes.sounding) do strum_stop(key) end
  if st.repeating then deckard.note_repeat("on", false) end
  close_menu()
  if st.browser then deckard.library(false) end
end

return M
