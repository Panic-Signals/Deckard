-- Maschine Mikro MK3 workflow overlays & velocity modifiers:
-- FIXED VEL / 16 VEL, SCENE, PATTERN, EVENTS, VARIATION, NAVIGATE, DUPLICATE / DOUBLE.
-- Clean-room Deckard Lua script.

local O = {}

local RED = 0
local ORANGE = 2
local YELLOW = 4
local GREEN = 6
local CYAN = 8
local MAGENTA = 14
local WHITE = 16
local DUP_FLASH = 0.35

O.EVENTS_PARAMS = { "POS", "PITCH", "VEL", "LEN", "PROB", "EVENT" }
O.GRID_PARAMS = { "STEP", "SNAP", "NUDGE", "QUICK" }
O.GRID_STEPS = {
  "1 BAR", "1/2", "1/4", "1/8", "1/16", "1/32", "1/64", "1/128",
  "1B T", "1/2T", "1/4T", "1/8T", "1/16T", "1/32T", "1/64T",
}
O.GRID_NUDGES = { "STEP", "1/2", "1/4", "1/8", "1/16" }
O.PERFORM_MODES = { "Filter", "Flanger", "Burst Echo", "Reso Echo", "Ring", "Stutter", "Tremolo", "Scratcher" }

local function current_perform_mode(s)
  local g = s.groups and s.groups[s.group]
  return (g and g.perform and g.perform.mode) or 1
end

local function pick_perform_mode(st, s, mode)
  local to = st.clamp(mode, 1, #O.PERFORM_MODES)
  deckard.perform_mode(s.group, to)
  st.popup("PERFORM FX", O.PERFORM_MODES[to])
end

local function patterns_of(s)
  local g = s.groups and s.groups[s.group]
  return (g and g.patterns) or {}
end

local function pattern_places(s)
  local out = {}
  for i, p in ipairs(patterns_of(s)) do
    out[p.slot] = i
  end
  return out
end

local function bar_ticks(s)
  local num = math.max(1, s.numerator or 4)
  local den = math.max(1, s.denominator or 4)
  return math.floor(960 * 4 / den) * num
end

local function step_pattern_length(s, steps, fine)
  local bar = bar_ticks(s)
  local cur = math.max(1, s.pattern_length or bar)
  if fine then
    return math.max(bar, cur + steps * bar)
  end
  local half = math.max(1, math.floor(bar / 2))
  local next_len = cur
  if steps > 0 then
    for _ = 1, steps do
      local candidate = half
      while candidate <= next_len do candidate = candidate * 2 end
      next_len = candidate
    end
  elseif steps < 0 then
    for _ = 1, -steps do
      local candidate = half
      while candidate * 2 < next_len do candidate = candidate * 2 end
      next_len = candidate
    end
  end
  return next_len
end

-- Active knob helpers shared across VARIATION and PLUG-IN overlays (skips false/empty slots).
function O.is_active_knob(knobs, idx)
  return knobs ~= nil and idx ~= nil and type(knobs[idx]) == "table"
end

function O.first_active_knob(knobs)
  if not knobs then return nil end
  for i = 1, 8 do
    if O.is_active_knob(knobs, i) then return i end
  end
  return nil
end

function O.last_active_knob(knobs)
  if not knobs then return nil end
  for i = 8, 1, -1 do
    if O.is_active_knob(knobs, i) then return i end
  end
  return nil
end

function O.next_active_knob(knobs, cur, dir)
  if not knobs then return cur end
  local i = (cur or 1) + dir
  while i >= 1 and i <= 8 do
    if O.is_active_knob(knobs, i) then return i end
    i = i + dir
  end
  return cur
end

function O.variation_knob(st, s)
  local knobs = s.control and s.control.knobs
  if O.is_active_knob(knobs, st.variation_knob) then
    return st.variation_knob, knobs[st.variation_knob]
  end
  local first = O.first_active_knob(knobs)
  if first then
    st.variation_knob = first
    return first, knobs[first]
  end
  return nil, nil
end

-- Notes inside the focused pattern's span.
function O.shown_notes(s)
  local a = s.pattern_start or 0
  local b = a + math.max(1, s.pattern_length or 1)
  local out = {}
  for _, n in ipairs(s.notes or {}) do
    if n.position >= a and n.position < b then
      out[#out + 1] = n
    end
  end
  return out
end

-- Hold-to-use / tap-to-pin lifecycle for SCENE, PATTERN, EVENTS.
local function handle_hold_pin_overlay(st, target, down)
  if down then
    if st.overlay == target and st.overlay_pinned then
      st.overlay = nil
      st.overlay_pinned = false
      st.overlay_used = false
      st.dup = nil
    else
      if st.overlay ~= target then
        st.dup = nil
      end
      st.overlay = target
      st.overlay_pinned = false
      st.overlay_used = false
    end
  else
    if st.overlay == target and not st.overlay_pinned then
      if st.overlay_used then
        st.overlay = nil
        st.overlay_used = false
        st.dup = nil
      else
        st.overlay_pinned = true
      end
    end
  end
end

-- Two-step source -> destination duplicate helper.
local function do_duplicate(st, kind, group, at, copy_fn)
  local now = deckard.now()
  local d = st.dup
  st.dup_used = true
  if st.overlay == "scene" or st.overlay == "pattern" then
    st.overlay_used = true
  end
  if d and d.kind == kind and d.group == group then
    if d.at ~= at then
      copy_fn(d.at, at)
      st.dup.at = at
      st.dup_flash = { kind = kind, group = group, at = at, until_at = now + DUP_FLASH }
      st.popup("DUPLICATED", string.format("%s %d -> %d", string.upper(kind), d.at, at))
    end
  else
    st.dup = { kind = kind, group = group, at = at }
    st.dup_flash = nil
  end
end

function O.dup_kind(st)
  if st.held_group then return "group" end
  if st.overlay == "pattern" then return "pattern" end
  if st.overlay == "scene" then return "scene" end
  return "sound"
end

function O.button(st, canon, down)
  local s = deckard.state()
  local shift = st.shift()

  -- FIXED VEL & SHIFT + FIXED VEL (16 Velocities)
  if canon == "FIXED_VEL" then
    if down then
      if shift then
        st.sixteen_vel = not st.sixteen_vel
        if st.sixteen_vel then
          st.mode = "pad"
          st.clear_overlays()
        end
        st.popup("16 VELOCITIES", st.sixteen_vel and "ON" or "OFF")
        st.held_fixed_vel = false
        st.fixed_vel_used = true
      else
        st.held_fixed_vel = true
        st.fixed_vel_used = false
      end
    else
      local was_held = st.held_fixed_vel
      local was_used = st.fixed_vel_used
      st.held_fixed_vel = false
      st.fixed_vel_used = false
      if was_held and not was_used then
        if st.sixteen_vel then
          st.sixteen_vel = false
          st.popup("16 VELOCITIES", "OFF")
        else
          local next_fv = not s.fixed_velocity
          deckard.fixed_velocity(next_fv)
          st.popup("FIXED VEL", next_fv and string.format("ON (%d)", s.velocity or 100) or "OFF")
        end
      end
    end
    return true
  end

  -- SCENE (hold/pin) & SHIFT + SCENE (SECTION: toggle Song / Arranger view)
  if canon == "SCENE" then
    if down and shift then
      st.scene_shift_down = true
      deckard.song_view(not s.song_view)
      st.popup("VIEW", s.song_view and "IDEAS" or "SONG")
      return true
    elseif not down and st.scene_shift_down then
      st.scene_shift_down = false
      return true
    end
    handle_hold_pin_overlay(st, "scene", down)
    return true
  end
  if canon == "PATTERN" then
    handle_hold_pin_overlay(st, "pattern", down)
    return true
  end
  if canon == "EVENTS" then
    handle_hold_pin_overlay(st, "events", down)
    return true
  end

  -- VARIATION & SHIFT + VARIATION (Navigate) — pure toggles on down
  if canon == "VARIATION" then
    if not down then return true end
    st.clear_quick_param()
    if shift then
      if st.overlay == "navigate" then
        st.overlay = nil
        st.overlay_pinned = false
      else
        st.overlay = "navigate"
        st.overlay_pinned = true
        st.dup = nil
      end
    else
      if st.overlay == "variation" then
        st.overlay = nil
        st.overlay_pinned = false
      else
        st.overlay = "variation"
        st.overlay_pinned = true
        st.dup = nil
        deckard.groove_page(2)
        st.variation_knob = O.first_active_knob(s.control and s.control.knobs) or 1
      end
    end
    return true
  end

  -- DUPLICATE & SHIFT + DUPLICATE (Double)
  if canon == "DUPLICATE" then
    if down then
      if shift then
        deckard.pattern_edit("double")
        st.popup("PATTERN DOUBLED")
        st.dup_shift_down = true
        return true
      end
      st.dup_shift_down = false
      if st.dup_active and st.dup_pinned then
        st.dup_active = false
        st.dup_pinned = false
        st.dup_used = false
        st.dup = nil
        st.dup_flash = nil
      else
        st.dup_active = true
        st.dup_pinned = false
        st.dup_used = false
        st.dup = nil
        st.dup_flash = nil
      end
    else
      if st.dup_shift_down then
        st.dup_shift_down = false
        return true
      end
      if st.dup_active and not st.dup_pinned then
        if st.dup_used then
          st.dup_active = false
          st.dup_used = false
          st.dup = nil
          st.dup_flash = nil
        else
          st.dup_pinned = true
        end
      end
    end
    return true
  end

  -- Overlay-scoped LEFT / RIGHT navigation
  if (canon == "LEFT" or canon == "RIGHT")
      and (st.overlay == "scene" or st.overlay == "pattern" or st.overlay == "events"
        or st.overlay == "grid" or st.overlay == "variation" or st.overlay == "navigate"
        or st.overlay == "perform") then
    if not down then return true end
    local dir = canon == "RIGHT" and 1 or -1
    if st.overlay == "scene" then
      st.scene_bank = math.max(1, st.scene_bank + dir)
      st.overlay_used = true
    elseif st.overlay == "pattern" then
      st.pattern_bank = math.max(1, st.pattern_bank + dir)
      st.overlay_used = true
    elseif st.overlay == "events" then
      st.events_param = st.clamp(st.events_param + dir, 1, #O.EVENTS_PARAMS)
      st.overlay_used = true
    elseif st.overlay == "grid" then
      st.grid_param = st.clamp((st.grid_param or 1) + dir, 1, #O.GRID_PARAMS)
    elseif st.overlay == "variation" then
      if shift then
        if dir > 0 then deckard.next_page() else deckard.previous_page() end
        st.variation_knob = 1
      else
        local knobs = s.control and s.control.knobs
        local cur = O.variation_knob(st, s)
        if cur then
          st.variation_knob = O.next_active_knob(knobs, cur, dir)
        end
      end
    elseif st.overlay == "navigate" then
      if dir > 0 then deckard.next_page() else deckard.previous_page() end
    elseif st.overlay == "perform" then
      pick_perform_mode(st, s, current_perform_mode(s) + dir)
    end
    return true
  end

  return false
end

function O.pad(st, pad)
  local s = deckard.state()

  -- Orthogonal DUPLICATE takes priority across GROUP, PATTERN, SCENE, and Pad mode
  if st.dup_active then
    local kind = O.dup_kind(st)
    if kind == "group" then
      local n = #(s.groups or {})
      local g = ((st.group_bank or 1) - 1) * 8 + pad
      local has_src = st.dup and st.dup.kind == "group"
      if g <= n or (has_src and g == n + 1) then
        do_duplicate(st, "group", 0, g, function(from, to) deckard.copy_group(from, to) end)
      end
    elseif kind == "pattern" then
      local at = (st.pattern_bank - 1) * 16 + pad
      local places = pattern_places(s)
      local has_src = st.dup and st.dup.kind == "pattern" and st.dup.group == s.group
      if places[at] or has_src then
        do_duplicate(st, "pattern", s.group, at, function(from, to) deckard.copy_pattern(s.group, from, to) end)
      end
    elseif kind == "scene" then
      local at = (st.scene_bank - 1) * 16 + pad
      local n = #(s.scenes or {})
      local has_src = st.dup and st.dup.kind == "scene"
      if (s.scenes and s.scenes[at]) or (has_src and at == n + 1) then
        do_duplicate(st, "scene", 0, at, function(from, to) deckard.copy_scene(from, to) end)
      end
    else
      do_duplicate(st, "sound", s.group, pad, function(from, to) deckard.copy_sound(from, to) end)
    end
    return true
  end

  -- SCENE overlay pads
  if st.overlay == "scene" then
    st.overlay_used = true
    local at = (st.scene_bank - 1) * 16 + pad
    local scenes = s.scenes or {}
    if st.held_erase then
      if scenes[at] and #scenes > 1 then
        deckard.delete_scene(at)
      end
    elseif scenes[at] then
      deckard.focus_scene(at)
    elseif at == #scenes + 1 then
      deckard.scene_edit("create")
    end
    return true
  end

  -- PATTERN overlay pads
  if st.overlay == "pattern" then
    st.overlay_used = true
    local at = (st.pattern_bank - 1) * 16 + pad
    local places = pattern_places(s)
    local idx = places[at]
    if st.held_erase then
      if idx then
        deckard.delete_pattern(s.group, idx)
      end
    elseif idx then
      deckard.pick_pattern(idx)
    elseif s.groups and s.groups[s.group] then
      deckard.new_pattern(s.group, at)
    end
    return true
  end

  -- EVENTS overlay pads
  if st.overlay == "events" then
    st.overlay_used = true
    if st.mode == "keyboard" or st.mode == "chords" then
      local base = (st.mode == "chords" and 48 or 60) + st.octave * 12
      local pitch = st.clamp(base + (pad - 1), 0, 127)
      deckard.events(st.held_erase and "erase_pitch" or "select_pitch", pitch)
    else
      if st.held_erase then
        deckard.erase_sound(pad)
      else
        deckard.events("select_sound", pad)
      end
    end
    return true
  end

  -- GRID overlay pads (1..15 step divisions, 16 step snap toggle)
  if st.overlay == "grid" then
    local g = s.grid or {}
    if pad == 16 then
      deckard.grid("step_snap", not g.step_snap)
    else
      deckard.grid("step", pad - 1)
      if not g.step_snap then
        deckard.grid("step_snap", true)
      end
    end
    return true
  end

  -- NAVIGATE overlay pads (16-pad view/focus/page/octave map)
  if st.overlay == "navigate" then
    if pad == 13 then
      deckard.song_view(not s.song_view)
    elseif pad == 14 then
      st.mixer_view = not st.mixer_view
      deckard.mixer_view(st.mixer_view)
    elseif pad == 15 then
      deckard.follow(not s.follow)
    elseif pad == 16 then
      local plugins = s.control and s.control.plugins
      deckard.show_plugins(not plugins)
    elseif pad == 9 or pad == 10 then
      local n = math.max(1, #(s.groups or {}))
      deckard.focus_group(st.clamp((s.group or 1) + (pad == 10 and 1 or -1), 1, n))
    elseif pad == 11 or pad == 12 then
      deckard.focus_sound(st.clamp((s.sound or 1) + (pad == 12 and 1 or -1), 1, 16))
    elseif pad == 5 or pad == 6 then
      local n = #(s.scenes or {})
      if n > 0 then
        deckard.focus_scene(st.clamp((s.scene or 1) + (pad == 6 and 1 or -1), 1, n))
      end
    elseif pad == 7 or pad == 8 then
      local pats = patterns_of(s)
      if #pats > 0 then
        deckard.pick_pattern(st.clamp((s.pattern or 1) + (pad == 8 and 1 or -1), 1, #pats))
      end
    elseif pad == 1 then
      deckard.previous_page()
    elseif pad == 2 then
      deckard.next_page()
    elseif pad == 3 or pad == 4 then
      st.octave = st.clamp(st.octave + (pad == 4 and 1 or -1), -3, 3)
      st.popup(string.format("OCTAVE: %+d", st.octave))
    end
    return true
  end

  -- PERFORM overlay pads: Pads 1-8 pick Perform FX mode (Filter .. Scratcher)
  if st.overlay == "perform" then
    if pad >= 1 and pad <= #O.PERFORM_MODES then
      pick_perform_mode(st, s, pad)
    end
    return true
  end

  -- VARIATION keeps base pads playable for auditioning
  return false
end

function O.encoder(st, steps)
  if steps == 0 then return false end
  local s = deckard.state()
  local shift = st.shift()

  -- Hold FIXED VEL + turn encoder adjusts Fixed Velocity value (1..127)
  if st.held_fixed_vel then
    st.fixed_vel_used = true
    local delta = steps * (shift and 1 or 5)
    local vel = st.clamp((s.velocity or 100) + delta, 1, 127)
    deckard.velocity(vel)
    st.popup(string.format("FIXED VEL: %d", vel))
    return true
  end

  -- PATTERN overlay: encoder adjusts pattern length
  if st.overlay == "pattern" then
    st.overlay_used = true
    local next_len = step_pattern_length(s, steps, shift)
    deckard.pattern_length(next_len)
    local bars = next_len / math.max(1, bar_ticks(s))
    st.popup(string.format("PATTERN LEN: %.1fB", bars))
    return true
  end

  -- SCENE overlay: encoder steps focused scene
  if st.overlay == "scene" then
    st.overlay_used = true
    local count = #(s.scenes or {})
    if count > 0 then
      local to = st.clamp((s.scene or 1) + steps, 1, count)
      deckard.focus_scene(to)
    end
    return true
  end

  -- EVENTS overlay: encoder edits selected notes along st.events_param
  if st.overlay == "events" then
    st.overlay_used = true
    local p = st.events_param
    if p == 1 then
      deckard.events(shift and "nudge_fine" or "nudge", steps)
    elseif p == 2 then
      deckard.events("pitch", steps)
    elseif p == 3 then
      deckard.events("velocity", steps)
    elseif p == 4 then
      deckard.events(shift and "length_fine" or "length", steps)
    elseif p == 5 then
      deckard.events("probability", steps)
    else
      deckard.events("select_step", steps)
    end
    return true
  end

  -- GRID overlay: encoder edits STEP / SNAP / NUDGE / QUICK
  if st.overlay == "grid" then
    local g = s.grid or {}
    local p = st.grid_param or 1
    if p == 1 then
      deckard.grid("step", st.clamp((g.step or 4) + steps, 0, 14))
    elseif p == 2 then
      deckard.grid("step_snap", not g.step_snap)
    elseif p == 3 then
      deckard.grid("nudge", st.clamp((g.nudge or 0) + steps, 0, 4))
    else
      deckard.grid("quick", not g.quick)
    end
    return true
  end

  -- VARIATION overlay: encoder steps selected active Groove knob
  if st.overlay == "variation" then
    local idx = O.variation_knob(st, s)
    if idx then
      deckard.step_knob(idx, steps, shift)
    end
    return true
  end

  -- NAVIGATE overlay: encoder steps Scene (Ideas) or Section (Song)
  if st.overlay == "navigate" then
    if s.song_view and s.sections and #s.sections > 0 then
      local to = st.clamp((s.section or 1) + steps, 1, #s.sections)
      deckard.focus_section(to)
    elseif s.scenes and #s.scenes > 0 then
      local to = st.clamp((s.scene or 1) + steps, 1, #s.scenes)
      deckard.focus_scene(to)
    end
    return true
  end

  -- PERFORM overlay: encoder cycles Perform FX mode (1..8)
  if st.overlay == "perform" then
    pick_perform_mode(st, s, current_perform_mode(s) + steps)
    return true
  end

  return false
end

function O.encoder_press(st)
  local s = deckard.state()
  local shift = st.shift()

  if st.overlay == "variation" then
    deckard.groove_apply()
    local pages = s.control and s.control.pages or {}
    local page_name = pages[s.control and s.control.page or 2] or "Variation"
    st.popup(page_name == "Generate" and "GENERATED" or "APPLIED")
    return true
  end

  if st.overlay == "events" then
    st.overlay_used = true
    if shift then
      local any_sel = false
      for _, n in ipairs(O.shown_notes(s)) do
        if n.selected then
          any_sel = true
          break
        end
      end
      deckard.events(any_sel and "select_none" or "select_all")
    else
      st.events_param = (st.events_param % #O.EVENTS_PARAMS) + 1
    end
    return true
  end

  if st.overlay == "pattern" then
    st.overlay_used = true
    deckard.pattern_edit("create")
    st.popup("NEW PATTERN")
    return true
  end

  if st.overlay == "scene" then
    st.overlay_used = true
    deckard.scene_edit("create")
    st.popup("NEW SCENE")
    return true
  end

  if st.overlay == "perform" then
    st.overlay = nil
    st.overlay_pinned = false
    return true
  end

  return false
end

local function is_dup_highlight(st, kind, group, at)
  if st.dup and st.dup.kind == kind and st.dup.group == group and st.dup.at == at then
    return true
  end
  if st.dup_flash and st.dup_flash.kind == kind and st.dup_flash.group == group and st.dup_flash.at == at
      and deckard.now() < st.dup_flash.until_at then
    return true
  end
  return false
end

function O.leds(st, s)
  local fv_lit = s.fixed_velocity or st.sixteen_vel or st.held_fixed_vel
  deckard.led("FIX_VELOCITY", fv_lit and "on" or "dim")
  deckard.led("SCENE", st.overlay == "scene" and "on" or "dim")
  deckard.led("PATTERN", st.overlay == "pattern" and "on" or "dim")
  deckard.led("EVENTS", st.overlay == "events" and "on" or "dim")
  deckard.led("VARIATION", (st.overlay == "variation" or st.overlay == "navigate") and "on" or "dim")
  deckard.led("DUPLICATE", st.dup_active and "on" or "dim")

  -- SCENE pad LEDs
  if st.overlay == "scene" then
    deckard.led("LEFT", st.scene_bank > 1 and "dim" or "off")
    deckard.led("RIGHT", "dim")
    local scenes = s.scenes or {}
    for i = 1, 16 do
      local at = (st.scene_bank - 1) * 16 + i
      local sc = scenes[at]
      if st.dup_active and is_dup_highlight(st, "scene", 0, at) then
        deckard.pad_led(i, WHITE, "on")
      elseif sc then
        deckard.pad_led(i, sc.palette or WHITE, at == s.scene and "on" or "dim")
      elseif at == #scenes + 1 then
        deckard.pad_led(i, WHITE, "dim")
      else
        deckard.pad_led(i, "off")
      end
    end
    return
  end

  -- PATTERN pad LEDs
  if st.overlay == "pattern" then
    deckard.led("LEFT", st.pattern_bank > 1 and "dim" or "off")
    deckard.led("RIGHT", "dim")
    local list, places = patterns_of(s), pattern_places(s)
    for i = 1, 16 do
      local at = (st.pattern_bank - 1) * 16 + i
      local idx = places[at]
      local p = idx and list[idx]
      if st.dup_active and is_dup_highlight(st, "pattern", s.group, at) then
        deckard.pad_led(i, WHITE, "on")
      elseif p then
        deckard.pad_led(i, p.palette or WHITE, idx == s.pattern and "on" or "dim")
      else
        deckard.pad_led(i, "off")
      end
    end
    return
  end

  -- EVENTS pad LEDs
  if st.overlay == "events" then
    deckard.led("LEFT", st.events_param > 1 and "dim" or "off")
    deckard.led("RIGHT", st.events_param < #O.EVENTS_PARAMS and "dim" or "off")
    local notes = O.shown_notes(s)
    local grp = s.groups and s.groups[s.group]
    local is_keys = (st.mode == "keyboard" or st.mode == "chords")
    local base = (st.mode == "chords" and 48 or 60) + st.octave * 12
    for i = 1, 16 do
      local any, all_sel = false, true
      local target_pitch = st.clamp(base + (i - 1), 0, 127)
      for _, n in ipairs(notes) do
        local match = is_keys and (n.sound == s.sound and n.pitch == target_pitch) or (not is_keys and n.sound == i)
        if match then
          any = true
          if not n.selected then all_sel = false end
        end
      end
      if any then
        local snd = grp and grp.sounds and grp.sounds[is_keys and s.sound or i]
        local pal = (snd and snd.palette) or (grp and grp.palette) or WHITE
        deckard.pad_led(i, pal, (st.held_erase or all_sel) and "on" or "dim")
      else
        deckard.pad_led(i, "off")
      end
    end
    return
  end

  -- VARIATION button LEDs (pads stay in base mode)
  if st.overlay == "variation" then
    local knobs = s.control and s.control.knobs
    local cur = O.variation_knob(st, s)
    if st.shift() then
      deckard.led("LEFT", "dim")
      deckard.led("RIGHT", "dim")
    else
      deckard.led("LEFT", (cur and O.next_active_knob(knobs, cur, -1) ~= cur) and "dim" or "off")
      deckard.led("RIGHT", (cur and O.next_active_knob(knobs, cur, 1) ~= cur) and "dim" or "off")
    end
    return
  end

  -- NAVIGATE pad LEDs (color-coded by function)
  if st.overlay == "navigate" then
    deckard.led("LEFT", "dim")
    deckard.led("RIGHT", "dim")
    local plugins = s.control and s.control.plugins
    -- Row 1: Page Left/Right (White dim), Octave Down/Up (Red dim)
    deckard.pad_led(1, WHITE, "dim")
    deckard.pad_led(2, WHITE, "dim")
    deckard.pad_led(3, RED, "dim")
    deckard.pad_led(4, RED, "dim")
    -- Row 2: Scene Prev/Next (Orange dim), Pattern Prev/Next (Magenta dim)
    deckard.pad_led(5, ORANGE, "dim")
    deckard.pad_led(6, ORANGE, "dim")
    deckard.pad_led(7, MAGENTA, "dim")
    deckard.pad_led(8, MAGENTA, "dim")
    -- Row 3: Group Prev/Next (Green dim), Sound Prev/Next (Yellow dim)
    deckard.pad_led(9, GREEN, "dim")
    deckard.pad_led(10, GREEN, "dim")
    deckard.pad_led(11, YELLOW, "dim")
    deckard.pad_led(12, YELLOW, "dim")
    -- Row 4: Song, Mixer, Follow, Plugins (Cyan bright when active, dim when inactive)
    deckard.pad_led(13, CYAN, s.song_view and "on" or "dim")
    deckard.pad_led(14, CYAN, st.mixer_view and "on" or "dim")
    deckard.pad_led(15, CYAN, s.follow and "on" or "dim")
    deckard.pad_led(16, CYAN, plugins and "on" or "dim")
    return
  end

  -- GRID overlay pad LEDs
  if st.overlay == "grid" then
    deckard.led("LEFT", (st.grid_param or 1) > 1 and "dim" or "off")
    deckard.led("RIGHT", (st.grid_param or 1) < #O.GRID_PARAMS and "dim" or "off")
    local g = s.grid or {}
    local step_pad = (g.step or 4) + 1
    for i = 1, 15 do
      deckard.pad_led(i, WHITE, (g.step_snap and i == step_pad) and "on" or "dim")
    end
    deckard.pad_led(16, WHITE, g.step_snap and "on" or "dim")
    return
  end

  -- PERFORM FX selector pad LEDs (Pads 1..8 select effect mode)
  if st.overlay == "perform" then
    local cur = current_perform_mode(s)
    local grp = s.groups and s.groups[s.group]
    local pal = (grp and grp.palette) or WHITE
    deckard.led("LEFT", cur > 1 and "dim" or "off")
    deckard.led("RIGHT", cur < #O.PERFORM_MODES and "dim" or "off")
    for i = 1, 8 do
      deckard.pad_led(i, pal, i == cur and "on" or "dim")
    end
    for i = 9, 16 do
      deckard.pad_led(i, "off")
    end
    return
  end

  -- DUPLICATE source highlight on Sounds or Groups
  if st.dup_active then
    if st.held_group then
      local base = ((st.group_bank or 1) - 1) * 8
      for i = 1, 16 do
        if is_dup_highlight(st, "group", 0, base + i) then
          deckard.pad_led(i, WHITE, "on")
        end
      end
    else
      for i = 1, 16 do
        if is_dup_highlight(st, "sound", s.group, i) then
          deckard.pad_led(i, WHITE, "on")
        end
      end
    end
  end
end

return O
