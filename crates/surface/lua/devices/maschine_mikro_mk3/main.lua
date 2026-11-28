-- Maschine Mikro MK3 clean-room controller script for Deckard (Issue #18).
-- No Native Instruments code is used.
--
-- Modular entry point delegating to:
-- - browse.lua   : BROWSE toggle, push-encoder library navigation, STAR favorites
-- - overlays.lua : FIXED VEL / 16 VEL, SCENE, PATTERN, EVENTS, VARIATION, NAVIGATE, DUPLICATE/DOUBLE
-- - plugin.lua   : PLUG-IN / MACRO & Module Loader overlay
-- - screen.lua   : 128x32 monochrome OLED renderer

local browse = require("browse")
local overlays = require("overlays")
local plugin = require("plugin")
local screen = require("screen")

local M = {}

local WHITE = 16
local TAP_RESET = 2.0
local POPUP_DURATION = 1.5
local taps = {}

-- Canonical button name aliases (strips BUTTON_ prefix and maps HID / short names).
local ALIASES = {
  TRANSPORT_PLAY = "PLAY",
  TRANSPORT_RECORD = "REC",
  RECORD = "REC",
  TRANSPORT_STOP = "STOP",
  TRANSPORT_LOOP = "RESTART",
  TRANSPORT_ERASE = "ERASE",
  FIX_VELOCITY = "FIXED_VEL",
  FIXED_VELOCITY = "FIXED_VEL",
  FIX_VEL = "FIXED_VEL",
}

local HELD_NAMES = {
  SHIFT = { "SHIFT", "BUTTON_SHIFT" },
  TEMPO = { "TEMPO", "BUTTON_TEMPO" },
  VOLUME = { "VOLUME", "BUTTON_VOLUME" },
  SWING = { "SWING", "BUTTON_SWING" },
  FIXED_VEL = { "BUTTON_FIX_VELOCITY", "FIX_VELOCITY", "FIXED_VEL", "BUTTON_FIXED_VEL" },
  DUPLICATE = { "DUPLICATE", "BUTTON_DUPLICATE" },
  ERASE = { "ERASE", "BUTTON_ERASE", "BUTTON_TRANSPORT_ERASE", "TRANSPORT_ERASE" },
}

-- Shared controller state passed to all modules.
local st = {
  -- Base pad mode
  mode = "pad",              -- "pad" | "keyboard" | "chords" | "step"
  sixteen_vel = false,       -- SHIFT + FIXED VEL sub-mode
  strip_mode = "pitch",      -- "pitch" | "mod" | "perform" | "notes"
  octave = 0,                -- -3 .. +3
  step_segment = 0,          -- 0-based 16-step page in Step mode

  -- Toggled modes
  browse = false,
  browse_field = 6,          -- 1=Kind, 2=Product, 3=Type, 4=SubType, 5=Char, 6=Results
  browse_prehear = true,

  -- Active pad overlay: nil | "scene" | "pattern" | "events" | "variation" | "navigate" | "plugin"
  overlay = nil,
  overlay_pinned = false,
  overlay_used = false,

  -- Orthogonal DUPLICATE state
  dup_active = false,
  dup_pinned = false,
  dup_used = false,
  dup = nil,
  dup_flash = nil,

  -- Per-overlay sub-state
  scene_bank = 1,
  pattern_bank = 1,
  group_bank = 1,
  events_param = 1,
  grid_param = 1,
  variation_knob = 1,
  mixer_view = false,
  sampling_open = false,
  sampling_seen_app = false,

  -- Plug-in / Channel / Module Loader sub-state
  held_plugin = false,
  plugin_used = false,
  plugin_shift_down = false,
  plugin_knob = 1,
  plugin_wrap = nil,
  plugin_wrap_page = nil,
  plugin_loader = false,
  plugin_field = 4,
  plugin_kind = 1,
  plugin_format = "Internal",
  plugin_vendor = "DECKARD",
  plugin_chosen = {},

  -- Modifier holds & Quick Parameter modes (VOLUME/VELOCITY, SWING/POSITION, TEMPO/TUNE)
  quick_param = nil,         -- nil | "volume" | "velocity" | "swing" | "position" | "tempo" | "tune"
  quick_held = false,        -- false | "VOLUME" | "SWING" | "TEMPO"
  quick_used = false,
  quick_unlatch_on_up = false,
  last_held_pad = nil,
  held_group = false,
  held_select = false,
  held_solo = false,
  held_mute = false,
  held_erase = false,
  held_fixed_vel = false,
  fixed_vel_used = false,
  auto_held = false,
  auto_used = false,
  scene_shift_down = false,
  held_pads = {},

  -- Strip & temporary OLED popup
  strip_pos = 0.0,
  strip_active = false,
  popup_text = "",
  popup_sub = "",
  popup_until = 0,
}

function st.clamp(v, min, max)
  if v < min then return min end
  if v > max then return max end
  return v
end

function st.normalize(name)
  if not name then return "" end
  local short = name:gsub("^BUTTON_", "")
  return ALIASES[short] or short
end

function st.held(canon)
  local list = HELD_NAMES[canon]
  if list then
    for _, raw in ipairs(list) do
      if deckard.held(raw) then return true end
    end
    return false
  end
  return deckard.held(canon) or deckard.held("BUTTON_" .. canon)
end

function st.shift()
  return st.held("SHIFT")
end

function st.popup(text, sub)
  st.popup_text = text or ""
  st.popup_sub = sub or ""
  st.popup_until = deckard.now() + POPUP_DURATION
end

local QUICK_MODES = {
  VOLUME = { primary = "volume", secondary = "velocity" },
  SWING  = { primary = "swing",  secondary = "position" },
  TEMPO  = { primary = "tempo",  secondary = "tune" },
}

function st.clear_quick_param()
  st.quick_param = nil
  st.quick_held = false
  st.quick_used = false
  st.quick_unlatch_on_up = false
end

function st.active_quick_param()
  if not st.quick_param then return nil end
  if st.quick_held and st.shift() then
    local spec = QUICK_MODES[st.quick_held]
    if spec then return spec.secondary end
  end
  return st.quick_param
end

function st.held_pad_target()
  if st.last_held_pad and st.held_pads[st.last_held_pad] then
    return st.last_held_pad
  end
  for i = 1, 16 do
    if st.held_pads[i] then return i end
  end
  return nil
end

local function grp_label(g)
  if g >= 1 and g <= 26 then
    return "GRP " .. string.char(64 + g)
  end
  return string.format("GRP %d", g)
end

local function format_db(db)
  if not db or db <= -120.0 then return "-INF dB" end
  if math.abs(db) < 0.05 then return "0.0 dB" end
  return string.format("%+.1f dB", db)
end

function st.quick_target(s, qp)
  local pad = st.held_pad_target()
  if st.held_group then
    local g = pad and (((st.group_bank or 1) - 1) * 8 + pad) or (s.group or 1)
    if g < 1 or g > #(s.groups or {}) then return nil end
    return { kind = "group", group = g }
  end
  if pad then
    return { kind = "sound", group = s.group or 1, sound = pad }
  end
  if qp == "volume" then
    return { kind = "master" }
  end
  return { kind = "sound", group = s.group or 1, sound = s.sound or 1 }
end

function st.quick_info(s)
  local qp = st.active_quick_param()
  if not qp then return nil end
  if qp == "tempo" then
    return {
      mode = qp,
      header = "TEMPO",
      target = "MASTER",
      value = string.format("%.1f BPM", s.tempo or 120.0),
    }
  elseif qp == "swing" then
    return {
      mode = qp,
      header = "SWING",
      target = "MASTER",
      value = string.format("SWING: %d%%", math.floor((s.swing or 0) + 0.5)),
    }
  elseif qp == "velocity" then
    return {
      mode = qp,
      header = "VELOCITY",
      target = s.fixed_velocity and "FIXED VEL" or "PAD VEL",
      value = string.format("VEL: %d", s.velocity or 100),
    }
  elseif qp == "position" then
    local start = s.pattern_start or 0
    local pos = s.song_view
      and math.max(0, math.floor((s.position or 0) + 0.5))
      or math.max(0, math.floor((s.pattern_position or s.position or start) - start + 0.5))
    return {
      mode = qp,
      header = "POSITION",
      target = s.song_view and "ARRANGER" or string.format("PAT %d", s.pattern or 1),
      value = string.format("POS: %d", pos),
    }
  elseif qp == "volume" then
    local t = st.quick_target(s, qp)
    if not t then
      return { mode = qp, header = "VOLUME", target = "NONE", value = "--" }
    elseif t.kind == "group" then
      local grp = s.groups and s.groups[t.group]
      local db = (grp and grp.level) or 0.0
      return { mode = qp, header = "VOLUME", target = grp_label(t.group), value = format_db(db) }
    elseif t.kind == "sound" then
      local grp = s.groups and s.groups[t.group]
      local snd = grp and grp.sounds and grp.sounds[t.sound]
      local db = (snd and snd.level) or 0.0
      return { mode = qp, header = "VOLUME", target = string.format("SND %02d", t.sound), value = format_db(db) }
    else
      local db = s.volume_db or deckard.level_to_db(s.volume or 0.75)
      return { mode = qp, header = "VOLUME", target = "MASTER", value = format_db(db) }
    end
  elseif qp == "tune" then
    local t = st.quick_target(s, qp)
    if not t then
      return { mode = qp, header = "TUNE", target = "NONE", value = "--" }
    elseif t.kind == "group" then
      local grp = s.groups and s.groups[t.group]
      return { mode = qp, header = "TUNE", target = grp_label(t.group), value = string.format("%+.1f ST", (grp and grp.tune) or 0.0) }
    else
      local grp = s.groups and s.groups[t.group]
      local snd = grp and grp.sounds and grp.sounds[t.sound]
      return { mode = qp, header = "TUNE", target = string.format("SND %02d", t.sound), value = string.format("%+.1f ST", (snd and snd.tune) or 0.0) }
    end
  end
  return nil
end

function st.clear_overlays()
  st.clear_quick_param()
  st.overlay = nil
  st.overlay_pinned = false
  st.overlay_used = false
  st.held_plugin = false
  st.plugin_used = false
  st.plugin_shift_down = false
  st.plugin_loader = false
  st.plugin_wrap = nil
  st.plugin_wrap_page = nil
  st.dup_active = false
  st.dup_pinned = false
  st.dup_used = false
  st.dup = nil
  st.dup_flash = nil
end

function st.pad_silenced()
  if st.shift() then return true end
  if st.held_group or st.held_select or st.held_solo or st.held_mute or st.held_erase then
    return true
  end
  if st.dup_active then return true end
  if st.overlay == "scene" or st.overlay == "pattern" or st.overlay == "events"
      or st.overlay == "navigate" or st.overlay == "grid" or st.overlay == "perform" then
    return true
  end
  return false
end

function st.sampling_is_open(s)
  local sm = s and s.sampling
  local app_open = sm ~= nil and (
    sm.sample ~= nil or sm.no_sample ~= nil or (sm.pages and sm.pages[1] and #sm.pages[1] > 0)
  )
  if app_open then
    st.sampling_seen_app = true
    st.sampling_open = true
    return true
  end
  if st.sampling_seen_app then
    st.sampling_seen_app = false
    st.sampling_open = false
  end
  return st.sampling_open
end

-- 16 evenly spaced velocities: pad 1 = 8 .. pad 16 = 127.
function st.sixteen_velocity(pad)
  return math.floor(pad * 127 / 16 + 0.5)
end

-- Pad helper: converts col (1..4) and row (1..4, 1 top) to 1-based pad number (1..16, bottom-left first).
local function pad_num(col, row)
  return (4 - row) * 4 + col
end

-- Reverse: pad number (1..16) to col (1..4), row (1..4).
local function pad_coords(pad)
  local row_from_bottom = math.floor((pad - 1) / 4)
  local col = (pad - 1) % 4 + 1
  local row = 4 - row_from_bottom
  return col, row
end

function st.step_info(s)
  local step_ticks = math.max((s.grid and s.grid.step_ticks) or 240, 1)
  local p_start = s.pattern_start or 0
  local p_len = s.pattern_length or 3840
  local first = -(-p_start // step_ticks) * step_ticks
  local range_w = math.max(0, p_len - (first - p_start))
  local segments = math.max(1, -(-(p_len // step_ticks) // 16))
  local seg = st.step_segment or 0
  if s.follow and s.playing and s.pattern_position then
    seg = (s.pattern_position - p_start) // (step_ticks * 16)
  end
  seg = st.clamp(seg, 0, segments - 1)
  local playing_step = nil
  if s.playing and s.pattern_position then
    playing_step = (s.pattern_position - first) // step_ticks - seg * 16 + 1
  end
  return {
    step_ticks = step_ticks,
    first = first,
    range_w = range_w,
    segments = segments,
    seg = seg,
    playing_step = playing_step,
  }
end

function st.has_step_note(s, at, step_ticks)
  for _, n in ipairs(s.notes or {}) do
    if n.sound == s.sound and n.position >= at and n.position < at + step_ticks then
      return true
    end
  end
  return false
end

local function tap()
  local now = deckard.now()
  if #taps > 0 and now - taps[#taps] > TAP_RESET then taps = {} end
  taps[#taps + 1] = now
  if #taps > 5 then table.remove(taps, 1) end
  if #taps >= 2 then
    local beat = (taps[#taps] - taps[1]) / (#taps - 1)
    if beat > 0 then
      local bpm = math.floor(60 / beat * 10 + 0.5) / 10
      deckard.tempo(st.clamp(bpm, 20.0, 400.0))
    end
  end
end

function M.open()
  st.mode = "pad"
  st.sixteen_vel = false
  st.strip_mode = "pitch"
  st.octave = 0
  st.step_segment = 0
  st.browse = false
  st.browse_field = 6
  st.browse_prehear = true
  st.clear_overlays()
  st.last_held_pad = nil
  st.scene_bank = 1
  st.pattern_bank = 1
  st.group_bank = 1
  st.events_param = 1
  st.grid_param = 1
  st.variation_knob = 1
  st.mixer_view = false
  st.sampling_open = false
  st.sampling_seen_app = false
  st.plugin_knob = 1
  st.plugin_kind = 1
  st.plugin_format = "Internal"
  st.plugin_vendor = "DECKARD"
  st.plugin_chosen = {}
  st.held_group = false
  st.held_select = false
  st.held_solo = false
  st.held_mute = false
  st.held_erase = false
  st.held_fixed_vel = false
  st.fixed_vel_used = false
  st.auto_held = false
  st.auto_used = false
  st.scene_shift_down = false
  st.held_pads = {}
  st.strip_active = false
  st.popup_text = ""
  st.popup_sub = ""
  st.popup_until = 0
end

function M.close()
  st.strip_active = false
  if st.browse then
    st.browse = false
    deckard.library(false)
  end
end

function M.button(name, down)
  local s = deckard.state()
  local canon = st.normalize(name)

  -- Encoder push routed by priority: overlay -> plugin -> browse -> quick_param unlatch
  if canon == "ENCODER_PRESS" then
    if not down then return end
    if overlays.encoder_press(st) then return end
    if plugin.encoder_press(st) then return end
    if browse.encoder_press(st) then return end
    if st.quick_param then
      st.clear_quick_param()
      return
    end
    return
  end

  -- VOLUME / SWING / TEMPO: hold-to-adjust or tap-to-latch (with SHIFT for VELOCITY / POSITION / TUNE)
  if QUICK_MODES[canon] then
    if down then
      local spec = QUICK_MODES[canon]
      local shift = st.shift()
      local target = shift and spec.secondary or spec.primary
      if st.quick_param == target or (not shift and (st.quick_param == spec.primary or st.quick_param == spec.secondary)) then
        st.quick_unlatch_on_up = true
      else
        st.quick_param = target
        st.quick_unlatch_on_up = false
      end
      st.quick_held = canon
      st.quick_used = false
    else
      if st.quick_held == canon then
        st.quick_held = false
        if st.quick_used or st.quick_unlatch_on_up then
          st.clear_quick_param()
        end
      end
    end
    return
  end

  -- Plug-in overlay handles PLUGIN and held-PLUGIN+MUTE/ERASE before base modifiers
  if plugin.button(st, canon, down) then return end

  -- Modifier holds
  if canon == "GROUP" then
    st.held_group = down
    return
  end
  if canon == "SELECT" then
    st.held_select = down
    return
  end
  if canon == "SOLO" then
    st.held_solo = down
    return
  end
  if canon == "MUTE" then
    if down then
      if st.shift() then
        deckard.choke_all()
        st.popup("CHOKE ALL")
        st.held_mute = false
      else
        st.held_mute = true
      end
    else
      st.held_mute = false
    end
    return
  end
  if canon == "ERASE" then
    st.held_erase = down
    return
  end

  -- Holding GROUP + LEFT/RIGHT cycles group banks (A-H, I-P, ...)
  if st.held_group and (canon == "LEFT" or canon == "RIGHT") then
    if down then
      local dir = canon == "RIGHT" and 1 or -1
      local max_bank = math.max(1, math.floor(#(s.groups or {}) / 8) + 1)
      st.group_bank = st.clamp((st.group_bank or 1) + dir, 1, max_bank)
    end
    return
  end

  -- AUTO: tap toggles Auto-write; hold + turn writes temporarily
  if canon == "AUTO" then
    if down then
      st.auto_held = not s.auto_write
      st.auto_used = false
      deckard.auto_write(not s.auto_write)
      st.popup("AUTO WRITE", (not s.auto_write) and "ON" or "OFF")
    elseif st.auto_held then
      st.auto_held = false
      if st.auto_used and s.auto_write then
        deckard.auto_write(false)
      end
    end
    return
  end

  -- Delegate to overlays first, then browse
  if overlays.button(st, canon, down) then return end
  if browse.button(st, canon, down) then return end

  if not down then return end

  -- Mode switching (exits pinned overlays and 16 Velocities)
  if canon == "PAD_MODE" then
    st.mode = "pad"
    st.sixteen_vel = false
    st.clear_overlays()
  elseif canon == "KEYBOARD" then
    st.mode = "keyboard"
    st.sixteen_vel = false
    st.clear_overlays()
  elseif canon == "CHORDS" then
    st.mode = "chords"
    st.sixteen_vel = false
    st.clear_overlays()
  elseif canon == "STEP" then
    st.mode = "step"
    st.sixteen_vel = false
    st.clear_overlays()

  -- Sampling, Lock & Note Repeat / Arp
  elseif canon == "SAMPLING" then
    local open = st.sampling_is_open(s)
    local next_open = not open
    st.sampling_open = next_open
    deckard.sampling(next_open)
    if next_open then
      st.clear_overlays()
      st.browse = false
    end
    st.popup("SAMPLING", next_open and "OPEN" or "CLOSED")
  elseif canon == "LOCK" then
    deckard.lock("button")
    local active = s.lock and s.lock.active
    st.popup("LOCK", active and "OFF" or "ON")
  elseif canon == "NOTE_REPEAT" then
    local nr_on = (s.note_repeat and s.note_repeat.active) == true
    if st.shift() then
      st.mode = "keyboard"
      st.sixteen_vel = false
      st.clear_overlays()
    end
    deckard.note_repeat("on", not nr_on)
    st.popup(st.shift() and "ARP" or "NOTE REPEAT", (not nr_on) and "ON" or "OFF")

  -- Strip mode selection & SHIFT + PERFORM (Perform FX add / select)
  elseif canon == "PITCH" then
    st.strip_mode = "pitch"
    if st.overlay == "perform" then st.overlay = nil; st.overlay_pinned = false end
  elseif canon == "MOD" then
    st.strip_mode = "mod"
    if st.overlay == "perform" then st.overlay = nil; st.overlay_pinned = false end
  elseif canon == "PERFORM" then
    st.strip_mode = "perform"
    local grp = s.groups and s.groups[s.group]
    if st.shift() then
      if st.overlay == "perform" then
        st.overlay = nil
        st.overlay_pinned = false
      else
        deckard.perform_fx()
        st.dup_active = false
        st.dup_pinned = false
        st.dup = nil
        st.overlay = "perform"
        st.overlay_pinned = true
        st.popup("PERFORM FX", (grp and grp.perform and grp.perform.name) or "Filter")
      end
    else
      if st.overlay == "perform" then
        st.overlay = nil
        st.overlay_pinned = false
      end
      st.popup("PERFORM", (grp and grp.perform and grp.perform.name) or "SHIFT+PERF: FX")
    end
  elseif canon == "NOTES" then
    st.strip_mode = "notes"
    if st.overlay == "perform" then st.overlay = nil; st.overlay_pinned = false end

  -- Transport controls
  elseif canon == "PLAY" then
    if s.playing then deckard.stop() else deckard.play() end
  elseif canon == "REC" then
    deckard.record(not s.recording, st.shift() and not s.recording)
  elseif canon == "STOP" then
    deckard.stop()
  elseif canon == "RESTART" then
    if st.shift() then deckard.set_loop(not s.looping) else deckard.restart() end
  elseif canon == "TAP" then
    if st.shift() then deckard.metronome(not s.metronome) else tap() end
  elseif canon == "FOLLOW" then
    if st.shift() then
      if st.overlay == "grid" then
        st.overlay = nil
        st.overlay_pinned = false
      else
        st.overlay = "grid"
        st.overlay_pinned = true
        st.dup = nil
      end
    elseif st.overlay == "grid" then
      st.overlay = nil
      st.overlay_pinned = false
    else
      deckard.follow(not s.follow)
    end

  -- Octave or step-page navigation (when no overlay or browse claimed LEFT/RIGHT)
  elseif canon == "LEFT" then
    if st.mode == "step" then
      local info = st.step_info(s)
      st.step_segment = st.clamp(info.seg - 1, 0, info.segments - 1)
    else
      st.octave = st.clamp(st.octave - 1, -3, 3)
    end
  elseif canon == "RIGHT" then
    if st.mode == "step" then
      local info = st.step_info(s)
      st.step_segment = st.clamp(info.seg + 1, 0, info.segments - 1)
    else
      st.octave = st.clamp(st.octave + 1, -3, 3)
    end
  end
end

function M.pad(col, row, down)
  local pad = pad_num(col, row)
  st.held_pads[pad] = down and true or false
  if down then
    st.last_held_pad = pad
  elseif st.last_held_pad == pad then
    st.last_held_pad = nil
  end
  if not down then return end

  local s = deckard.state()
  local qp = st.active_quick_param()

  -- Active overlays / duplicate handle pads first
  if overlays.pad(st, pad) then return end

  -- GROUP + Pad picks/mutes/solos/deletes Group, and pad (#groups + 1) creates a new group
  -- (or targets that Group while VOLUME / TUNE quick-param mode is active).
  if st.held_group then
    if (qp == "volume" or qp == "tune") and not (st.held_erase or st.held_mute or st.held_solo) then
      return
    end
    local base = ((st.group_bank or 1) - 1) * 8
    local g = base + pad
    local count = #(s.groups or {})
    if st.held_erase then
      if g <= count then
        deckard.delete_group(g)
        st.popup("DELETE GROUP", string.format("GRP %d", g))
      end
    elseif st.held_mute then
      if g <= count then deckard.mute_group(g) end
    elseif st.held_solo then
      if g <= count then deckard.solo_group(g) end
    elseif g <= count then
      deckard.focus_group(g)
    elseif g == count + 1 then
      deckard.add_group()
    end
    return
  end

  -- MUTE / SOLO modifiers
  if st.held_mute then
    deckard.mute_sound(pad)
    return
  end
  if st.held_solo then
    deckard.solo_sound(pad)
    return
  end

  -- Base-mode ERASE + Pad
  if st.held_erase then
    if st.mode == "keyboard" or st.mode == "chords" then
      local base = (st.mode == "chords" and 48 or 60) + st.octave * 12
      local pitch = st.clamp(base + (pad - 1), 0, 127)
      deckard.events("erase_pitch", pitch)
    elseif st.shift() then
      deckard.reset_sound(pad)
    else
      deckard.erase_sound(pad)
    end
    return
  end

  -- Base-mode SELECT + Pad silently focuses sound without triggering audio or toggling a step
  if st.held_select then
    deckard.focus_sound(pad)
    return
  end

  -- Silkscreen Pad Shortcuts: SHIFT + Pads 1..16
  if st.shift() then
    if pad == 1 then
      deckard.edit("undo"); st.popup("UNDO")
    elseif pad == 2 then
      deckard.edit("redo"); st.popup("REDO")
    elseif pad == 3 then
      deckard.edit("undo_step"); st.popup("UNDO STEP")
    elseif pad == 4 then
      deckard.edit("redo_step"); st.popup("REDO STEP")
    elseif pad == 5 then
      deckard.events("quantize"); st.popup("QUANTIZE")
    elseif pad == 6 then
      deckard.events("quantize_half"); st.popup("QUANTIZE 50%")
    elseif pad == 7 then
      deckard.events("nudge_shown", -1); st.popup("NUDGE -")
    elseif pad == 8 then
      deckard.events("nudge_shown", 1); st.popup("NUDGE +")
    elseif pad == 9 then
      deckard.events("clear"); st.popup("CLEAR NOTES")
    elseif pad == 10 then
      deckard.edit("clear_modulation"); st.popup("CLEAR MOD")
    elseif pad == 11 then
      deckard.edit("copy"); st.popup("COPY")
    elseif pad == 12 then
      deckard.edit("paste"); st.popup("PASTE")
    elseif pad == 13 then
      deckard.events("transpose", -1); st.popup("SEMITONE -1")
    elseif pad == 14 then
      deckard.events("transpose", 1); st.popup("SEMITONE +1")
    elseif pad == 15 then
      deckard.events("transpose", -12); st.popup("OCTAVE -1")
    elseif pad == 16 then
      deckard.events("transpose", 12); st.popup("OCTAVE +1")
    end
    return
  end

  -- While VOLUME or TUNE quick-param mode is active, holding a pad targets that Sound cleanly
  -- without toggling sequencer steps.
  if qp == "volume" or qp == "tune" then
    return
  end

  -- Base mode pad handling (empty pads in Pad mode must not become selected)
  if st.mode == "pad" then
    if not st.sixteen_vel then
      local grp = s.groups and s.groups[s.group]
      local snd = grp and grp.sounds and grp.sounds[pad]
      if snd and snd.used then
        deckard.focus_sound(pad)
      end
    end
  elseif st.mode == "step" then
    local info = st.step_info(s)
    local at = info.first + (pad - 1 + info.seg * 16) * info.step_ticks
    deckard.step(at, info.step_ticks, s.velocity or 100)
  end
end

function M.strip(_strip_idx, _finger, kind, pos)
  st.strip_pos = pos
  st.strip_active = (kind ~= "up")

  if st.strip_mode == "pitch" then
    local bend = (pos - 0.5) * 2.0
    if kind == "up" then bend = 0.0 end
    deckard.strip("pitch", bend)
  elseif st.strip_mode == "mod" then
    deckard.strip("mod", pos)
  elseif st.strip_mode == "perform" then
    if kind == "down" then
      deckard.strip("perform_touch", pos)
    elseif kind == "up" then
      deckard.strip("perform_release", pos)
    else
      deckard.strip("perform", pos)
    end
  end
end

function M.encoder(steps)
  local s = deckard.state()
  local shift = st.shift()
  if st.auto_held and steps ~= 0 then st.auto_used = true end

  -- 1. Active / held quick parameter modes (highest priority)
  local qp = st.active_quick_param()
  if qp then
    if steps ~= 0 then
      st.quick_used = true
    end
    if qp == "tempo" then
      local bpm = st.clamp(s.tempo + steps * 1.0, 20.0, 400.0)
      deckard.tempo(bpm)
      st.popup(string.format("TEMPO: %.1f BPM", bpm))
    elseif qp == "tune" then
      local t = st.quick_target(s, qp)
      if t then
        if t.kind == "group" then
          local grp = s.groups and s.groups[t.group]
          local cur = (grp and grp.tune) or 0.0
          local to = st.clamp(cur + steps * 1.0, -36.0, 36.0)
          deckard.set_tune(t.group, nil, to)
          st.popup(grp_label(t.group), string.format("%+.1f ST", to))
        else
          local grp = s.groups and s.groups[t.group]
          local snd = grp and grp.sounds and grp.sounds[t.sound]
          local cur = (snd and snd.tune) or 0.0
          local to = st.clamp(cur + steps * 1.0, -36.0, 36.0)
          deckard.set_tune(t.group, t.sound, to)
          st.popup(string.format("SND %02d", t.sound), string.format("%+.1f ST", to))
        end
      end
    elseif qp == "volume" then
      local t = st.quick_target(s, qp)
      if t then
        local function step_channel_db(level_db)
          local cur_db = math.max(level_db or 0.0, -60.0)
          local raw = st.clamp(cur_db + steps * 1.0, -60.0, 10.0)
          local to_db = math.floor(raw * 100 + 0.5) / 100
          if to_db <= -60.0 then to_db = -120.0 end
          return to_db
        end
        if t.kind == "group" then
          local grp = s.groups and s.groups[t.group]
          local to_db = step_channel_db(grp and grp.level)
          deckard.set_level(t.group, nil, to_db)
          st.popup(grp_label(t.group), format_db(to_db))
        elseif t.kind == "sound" then
          local grp = s.groups and s.groups[t.group]
          local snd = grp and grp.sounds and grp.sounds[t.sound]
          local to_db = step_channel_db(snd and snd.level)
          deckard.set_level(t.group, t.sound, to_db)
          st.popup(string.format("SND %02d", t.sound), format_db(to_db))
        else
          local vol = st.clamp(s.volume + steps * 0.05, 0.0, 1.0)
          deckard.volume(vol)
          st.popup("MASTER", format_db(deckard.level_to_db(vol)))
        end
      end
    elseif qp == "velocity" then
      local vel = st.clamp((s.velocity or 100) + steps * 5, 1, 127)
      deckard.velocity(vel)
      st.popup(string.format("VELOCITY: %d", vel))
    elseif qp == "swing" then
      local sw = st.clamp(s.swing + steps * 5, 0.0, 100.0)
      deckard.swing(sw)
      st.popup(string.format("SWING: %d%%", math.floor(sw + 0.5)))
    elseif qp == "position" then
      local step_ticks = (s.grid and s.grid.step_ticks) or 240
      local start = s.pattern_start or 0
      local len = math.max(1, s.pattern_length or 3840)
      local to
      if s.song_view then
        to = math.max(0, math.floor((s.position or 0) + 0.5) + steps * step_ticks)
      else
        local rel = math.max(0, math.floor((s.pattern_position or s.position or start) - start + 0.5))
        to = st.clamp(rel + steps * step_ticks, 0, len - 1)
      end
      deckard.seek(to)
      st.popup(string.format("POS: %d", to))
    end
    return
  end

  -- Holding GROUP + encoder cycles group banks (A-H, I-P, ...)
  if st.held_group and steps ~= 0 then
    local max_bank = math.max(1, math.floor(#(s.groups or {}) / 8) + 1)
    st.group_bank = st.clamp((st.group_bank or 1) + (steps > 0 and 1 or -1), 1, max_bank)
    return
  end

  -- 2. Active overlays (including held FIXED VEL and PLUG-IN)
  if overlays.encoder(st, steps) then return end
  if plugin.encoder(st, steps) then return end

  -- 3. BROWSE toggle active
  if browse.encoder(st, steps) then return end

  -- Sampling view active: encoder cycles Sampling tabs (1=RECORD..4=ZONE)
  if st.sampling_is_open(s) and steps ~= 0 then
    local tab = st.clamp((s.sampling and s.sampling.tab or 1) + (steps > 0 and 1 or -1), 1, 4)
    deckard.sampling_tab(tab)
    return
  end

  -- 4. Idle fallback: navigate sounds / groups
  if shift then
    deckard.focus_group(st.clamp(s.group + steps, 1, math.max(1, #s.groups)))
  else
    deckard.focus_sound(st.clamp(s.sound + steps, 1, 16))
  end
end

-- Fast-path pad notes map for low-latency triggering straight from USB thread.
function M.pads()
  local s = deckard.state()
  local map = {
    velocity = s.velocity or 100,
    fixed_velocity = (s.fixed_velocity == true),
  }

  -- Silence fast-path audio whenever pads are consumed by a modifier or overlay.
  if st.pad_silenced() then
    return map
  end

  if st.mode == "pad" then
    if st.sixteen_vel then
      for i = 1, 16 do
        local col, row = pad_coords(i)
        local key = col + (row - 1) * 8
        map[key] = { s.group, s.sound, 60, velocity = st.sixteen_velocity(i) }
      end
    else
      for i = 1, 16 do
        local col, row = pad_coords(i)
        local key = col + (row - 1) * 8
        map[key] = { s.group, i, 60 }
      end
    end
  elseif st.mode == "keyboard" then
    local base_pitch = 60 + st.octave * 12
    for i = 1, 16 do
      local col, row = pad_coords(i)
      local key = col + (row - 1) * 8
      map[key] = { s.group, s.sound, st.clamp(base_pitch + (i - 1), 0, 127) }
    end
  elseif st.mode == "chords" then
    local base_pitch = 48 + st.octave * 12
    for i = 1, 16 do
      local col, row = pad_coords(i)
      local key = col + (row - 1) * 8
      map[key] = { s.group, s.sound, st.clamp(base_pitch + (i - 1), 0, 127) }
    end
  end

  return map
end

function M.leds()
  local s = deckard.state()

  -- Transport & Mode LEDs
  deckard.led("PLAY", s.playing and "on" or "dim")
  deckard.led("REC", s.recording and "on" or "off")
  deckard.led("STOP", s.playing and "dim" or "off")
  deckard.led("RESTART", s.looping and "on" or "dim")
  deckard.led("TAP", (s.metronome or st.held("TAP")) and "on" or "dim")
  deckard.led("FOLLOW", (s.follow or st.overlay == "grid") and "on" or "dim")
  deckard.led("SHIFT", st.shift() and "on" or "dim")
  local qp = st.active_quick_param()
  deckard.led("VOLUME", (qp == "volume" or qp == "velocity" or st.held("VOLUME")) and "on" or "dim")
  deckard.led("SWING", (qp == "swing" or qp == "position" or st.held("SWING")) and "on" or "dim")
  deckard.led("TEMPO", (qp == "tempo" or qp == "tune" or st.held("TEMPO")) and "on" or "dim")
  deckard.led("SAMPLING", st.sampling_is_open(s) and "on" or "dim")
  deckard.led("AUTO", s.auto_write and "on" or "dim")
  deckard.led("LOCK", (s.lock and s.lock.active) and "on" or "dim")
  deckard.led("NOTE_REPEAT", (s.note_repeat and s.note_repeat.active) and "on" or "dim")
  deckard.led("GROUP", st.held_group and "on" or "dim")
  deckard.led("SELECT", st.held_select and "on" or "dim")
  deckard.led("SOLO", st.held_solo and "on" or "dim")
  deckard.led("MUTE", st.held_mute and "on" or "dim")
  deckard.led("ERASE", st.held_erase and "on" or "dim")

  -- Mode buttons
  deckard.led("PAD_MODE", st.mode == "pad" and "on" or "dim")
  deckard.led("KEYBOARD", st.mode == "keyboard" and "on" or "dim")
  deckard.led("CHORDS", st.mode == "chords" and "on" or "dim")
  deckard.led("STEP", st.mode == "step" and "on" or "dim")

  -- Strip mode buttons
  deckard.led("PITCH", st.strip_mode == "pitch" and "on" or "dim")
  deckard.led("MOD", st.strip_mode == "mod" and "on" or "dim")
  deckard.led("PERFORM", st.strip_mode == "perform" and "on" or "dim")
  deckard.led("NOTES", st.strip_mode == "notes" and "on" or "dim")

  -- Touch strip 25 LEDs
  if st.strip_active then
    local seg = math.floor(st.strip_pos * 24 + 0.5) + 1
    for i = 1, 25 do
      if st.strip_mode == "pitch" then
        local center = 13
        local lit = (seg >= center and i >= center and i <= seg) or
                    (seg < center and i <= center and i >= seg)
        deckard.strip_led(i, lit and "on" or "off")
      else
        deckard.strip_led(i, i <= seg and "on" or "off")
      end
    end
  elseif st.strip_mode == "perform" then
    local grp = s.groups and s.groups[s.group]
    if grp and grp.perform then
      local seg = math.floor((grp.perform.pos or 0.5) * 24 + 0.5) + 1
      for i = 1, 25 do
        local near = math.abs(i - seg) <= 1
        deckard.strip_led(i, near and (grp.perform.touched and "on" or "dim") or "off")
      end
    else
      for i = 1, 25 do deckard.strip_led(i, "off") end
    end
  else
    for i = 1, 25 do deckard.strip_led(i, "off") end
  end

  -- Navigation arrow LEDs (default off; lit in keyboard/chords for octave and held GROUP for banks)
  if st.held_group then
    local max_bank = math.max(1, math.floor(#(s.groups or {}) / 8) + 1)
    local bank = st.group_bank or 1
    deckard.led("LEFT", bank > 1 and (st.held("LEFT") and "on" or "dim") or "off")
    deckard.led("RIGHT", bank < max_bank and (st.held("RIGHT") and "on" or "dim") or "off")
  elseif st.mode == "keyboard" or st.mode == "chords" then
    deckard.led("LEFT", st.octave > -3 and (st.held("LEFT") and "on" or "dim") or "off")
    deckard.led("RIGHT", st.octave < 3 and (st.held("RIGHT") and "on" or "dim") or "off")
  elseif st.mode == "step" then
    local info = st.step_info(s)
    deckard.led("LEFT", info.seg > 0 and (st.held("LEFT") and "on" or "dim") or "off")
    deckard.led("RIGHT", info.seg < info.segments - 1 and (st.held("RIGHT") and "on" or "dim") or "off")
  else
    deckard.led("LEFT", "off")
    deckard.led("RIGHT", "off")
  end

  -- Browse & Overlay button LEDs
  browse.leds(st, s)
  plugin.leds(st, s)

  -- Base Pad LEDs (may be overridden by overlays.leds below)
  if st.held_group then
    local muting_grp = st.held_mute or st.held_solo
    local base = ((st.group_bank or 1) - 1) * 8
    local count = #(s.groups or {})
    for i = 1, 16 do
      local gidx = base + i
      if gidx <= count then
        local g = s.groups[gidx]
        local bright = (muting_grp and (not g.muted)) or (not muting_grp and gidx == s.group)
        local shade = (s.playing and g.playing and not st.held_pads[i]) and "flash" or (bright and "on" or "dim")
        deckard.pad_led(i, g.palette or WHITE, shade)
      elseif gidx == count + 1 then
        deckard.pad_led(i, WHITE, "dim")
      else
        deckard.pad_led(i, "off")
      end
    end
  elseif st.mode == "pad" or st.held_mute or st.held_solo or st.held_select then
    local grp = s.groups[s.group]
    local sounds = (grp and grp.sounds) or {}
    local muting = st.held_mute or st.held_solo
    if st.sixteen_vel and not (st.held_mute or st.held_solo or st.held_select) then
      local snd = sounds[s.sound]
      local spal = (snd and snd.used and snd.palette) or WHITE
      for i = 1, 16 do
        local shade = st.held_pads[i] and "flash" or (i >= 9 and "on" or "dim")
        deckard.pad_led(i, spal, shade)
      end
    else
      for i = 1, 16 do
        local sound = sounds[i]
        local held = st.held_pads[i]
        if not sound or not sound.used then
          deckard.pad_led(i, "off")
        elseif muting then
          if sound.playing and not held then
            deckard.pad_led(i, sound.palette or WHITE, "flash")
          else
            deckard.pad_led(i, sound.palette or WHITE, (not sound.muted) and "on" or "dim")
          end
        else
          local chosen = sound.selected or i == s.sound
          local shade = (held or sound.playing) and "flash" or (chosen and "on" or "dim")
          deckard.pad_led(i, sound.palette or WHITE, shade)
        end
      end
    end
  elseif st.mode == "keyboard" or st.mode == "chords" then
    local grp = s.groups[s.group]
    local snd = grp and grp.sounds and grp.sounds[s.sound]
    local pal = (snd and snd.used and snd.palette) or (snd and not snd.used and WHITE) or (grp and grp.palette) or WHITE
    for i = 1, 16 do
      if st.held_pads[i] then
        deckard.pad_led(i, pal, "flash")
      elseif (i - 1) % 12 == 0 then
        deckard.pad_led(i, WHITE, "on")
      else
        deckard.pad_led(i, pal, "dim")
      end
    end
  elseif st.mode == "step" then
    local info = st.step_info(s)
    local grp = s.groups and s.groups[s.group]
    local pal = (grp and grp.palette) or WHITE
    for i = 1, 16 do
      local offset = (i - 1 + info.seg * 16) * info.step_ticks
      local has_step = offset < info.range_w and st.has_step_note(s, info.first + offset, info.step_ticks)
      if info.playing_step == i then
        deckard.pad_led(i, WHITE, "on")
      elseif has_step then
        deckard.pad_led(i, pal, st.held_pads[i] and "flash" or "on")
      else
        deckard.pad_led(i, "off")
      end
    end
  end

  overlays.leds(st, s)
end

function M.screen()
  screen.draw(st, deckard.state())
end

return M
