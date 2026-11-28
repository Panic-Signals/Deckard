-- Maschine Mikro MK3 128x32 monochrome OLED screen renderer.
-- Clean-room Deckard Lua script.

local browse = require("browse")
local overlays = require("overlays")
local plugin = require("plugin")

local S = {}

local function trunc(str, max_len)
  if not str then return "" end
  if utf8 then
    local len = utf8.len(str)
    if not len then
      str = str:gsub("[\128-\255]", "?")
      return #str <= max_len and str or string.sub(str, 1, max_len)
    end
    if len <= max_len then return str end
    local cut = utf8.offset(str, max_len + 1)
    return cut and string.sub(str, 1, cut - 1) or str
  end
  if #str <= max_len then return str end
  return string.sub(str, 1, max_len)
end

local MARQUEE_PAUSE = 0.75
local MARQUEE_SPEED = 26
local marquee_state = {}
local last_view_key = nil

local function marquee_text(slot_key, row, col, width, str)
  str = str or ""
  if not deckard.screen.marquee then
    local max_chars = math.max(1, math.floor((width + 1) / 6))
    deckard.screen.text(row, col, trunc(str, max_chars), true)
    return
  end
  local now = deckard.now()
  local entry = marquee_state[slot_key]
  if not entry or entry.text ~= str then
    entry = { text = str, since = now }
    marquee_state[slot_key] = entry
  end
  local elapsed = math.max(0, now - entry.since)
  local scroll_px = elapsed > MARQUEE_PAUSE and math.floor((elapsed - MARQUEE_PAUSE) * MARQUEE_SPEED) or 0
  deckard.screen.marquee(row, col, width, scroll_px, str, true)
end

local function draw_popup(st)
  if deckard.now() < st.popup_until and st.popup_text ~= "" then
    deckard.screen.frame_rect(12, 4, 18, 120, true)
    if st.popup_sub ~= "" then
      deckard.screen.text_centered(14, 6, 116, trunc(st.popup_text, 19), true)
      deckard.screen.text_centered(22, 6, 116, trunc(st.popup_sub, 19), true)
    else
      deckard.screen.text_centered(18, 6, 116, trunc(st.popup_text, 19), true)
    end
    return true
  end
  return false
end

local function draw_browse(st, s)
  local b = s.browser or {}
  local kind_name = browse.KINDS[b.kind] or "LIBRARY"
  local right_hdr = b.favorites and ("*" .. trunc(kind_name, 8)) or trunc(kind_name, 9)
  deckard.screen.status("BROWSE", right_hdr)
  if draw_popup(st) then return end

  local field_name = browse.FIELDS[st.browse_field] or "RESULTS"
  local field_val = browse.field_value(b, st.browse_field)
  marquee_text("browse_row1", 13, 2, 125, "> " .. field_name .. ": " .. field_val)

  local results = b.results or {}
  local focus = b.focus or 1
  local item = results[focus]
  if item then
    local prefix = item.favorite and "* " or ""
    marquee_text("browse_row2", 23, 2, 125, prefix .. (item.name or ""))
  elseif b.busy then
    deckard.screen.text(23, 2, "SEARCHING...", true)
  else
    deckard.screen.text(23, 2, "(NO RESULTS)", true)
  end
end

local function draw_scene(st, s)
  local right = string.format("BNK %d", st.scene_bank or 1)
  deckard.screen.status(st.dup_active and "DUP SCENE" or "SCENE", right)
  if draw_popup(st) then return end
  local scenes = s.scenes or {}
  local sc = scenes[s.scene]
  local name = (sc and sc.name and sc.name ~= "") and sc.name or string.format("Scene %d", s.scene or 1)
  marquee_text("scene_row1", 13, 2, 125, string.format("SCN %d: %s", s.scene or 1, name))
  local bot = string.format("%d SCENES", #scenes)
  if st.dup_active then
    bot = st.dup and string.format("SRC %d -> DEST", st.dup.at) or "TAP SRC SCENE"
  end
  deckard.screen.text(23, 2, trunc(bot, 17), true)
  if st.overlay_pinned then deckard.screen.text_right(23, 126, "PIN", true) end
end

local function draw_pattern(st, s)
  local right = string.format("BNK %d", st.pattern_bank or 1)
  deckard.screen.status(st.dup_active and "DUP PATTERN" or "PATTERN", right)
  if draw_popup(st) then return end
  local grp = s.groups and s.groups[s.group]
  local pats = (grp and grp.patterns) or {}
  local p = pats[s.pattern]
  local name = (p and p.name and p.name ~= "") and p.name or string.format("Pattern %d", s.pattern or 1)
  marquee_text("pattern_row1", 13, 2, 125, string.format("PTN %d: %s", s.pattern or 1, name))
  local num = math.max(1, s.numerator or 4)
  local den = math.max(1, s.denominator or 4)
  local bar = math.max(1, math.floor(960 * 4 / den) * num)
  local bars = (s.pattern_length or bar) / bar
  local bot = string.format("LEN: %.1fB", bars)
  if st.dup_active then
    bot = st.dup and string.format("SRC %d -> DEST", st.dup.at) or "TAP SRC PTN"
  end
  deckard.screen.text(23, 2, trunc(bot, 17), true)
  if st.overlay_pinned then deckard.screen.text_right(23, 126, "PIN", true) end
end

local function draw_duplicate(st)
  local kind = string.upper(overlays.dup_kind(st))
  deckard.screen.status("DUPLICATE", trunc(kind, 8))
  if draw_popup(st) then return end
  if st.dup then
    deckard.screen.text(13, 2, trunc(string.format("SRC: %s %d", kind, st.dup.at), 21), true)
    deckard.screen.text(23, 2, "TAP DEST TO COPY", true)
  else
    deckard.screen.text(13, 2, trunc("TAP SOURCE " .. kind, 21), true)
    deckard.screen.text(23, 2, st.dup_pinned and "PINNED" or "HOLD OR PIN", true)
  end
end

local function draw_events(st, s)
  local param = overlays.EVENTS_PARAMS[st.events_param] or "POS"
  deckard.screen.status("EVENTS", param)
  if draw_popup(st) then return end
  local shown = overlays.shown_notes(s)
  local sel = {}
  for _, n in ipairs(shown) do
    if n.selected then sel[#sel + 1] = n end
  end
  deckard.screen.text(13, 2, trunc(string.format("%d SEL / %d NOTES", #sel, #shown), 21), true)

  local val = "-"
  if #sel > 0 then
    local field = ({ "position", "pitch", "velocity", "length", "probability" })[st.events_param]
    if field then
      local first = sel[1][field]
      local same = true
      for _, n in ipairs(sel) do
        if n[field] ~= first then same = false; break end
      end
      val = same and tostring(first) or "*"
    else
      val = #sel == 1 and "1" or "*"
    end
  end
  deckard.screen.text(23, 2, trunc(param .. ": " .. val, 17), true)
  if st.overlay_pinned then deckard.screen.text_right(23, 126, "PIN", true) end
end

local function draw_knob_row(idx, knob)
  local title = string.format("%d:%s", idx, knob.name or "")
  if knob.macro then title = title .. " [M]" end
  marquee_text("knob_title", 13, 2, 125, title)
  deckard.screen.text(23, 2, trunc(knob.text or "", 10), true)
  deckard.screen.bar(23, 66, 60, 7, knob.value or 0.0)
end

local function draw_variation(st, s)
  local c = s.control or {}
  local pages = c.pages or {}
  local page_name = string.upper(pages[c.page or 2] or "VARIATION")
  deckard.screen.status(trunc(page_name, 12), "PUSH:GO")
  if draw_popup(st) then return end
  local idx, knob = overlays.variation_knob(st, s)
  if knob then
    draw_knob_row(idx, knob)
  else
    deckard.screen.text(16, 2, "NO GROOVE PARAMS", true)
  end
end

local function draw_plugin(st, s)
  local c = s.control or {}
  local lvl = plugin.level_label(c.level or 3)
  local lvl_short = string.sub(lvl, 1, 1)

  -- Module Loader sub-view
  if st.plugin_loader and c.plugins then
    local target = c.slot_index and string.format("%s%d", lvl_short, c.slot_index) or (lvl_short .. "+")
    local kind_lbl = plugin.loader_field_value(st, c, 1) == "INSTRUMENT" and "INST" or "FX"
    deckard.screen.status("LOAD " .. target, kind_lbl)
    if draw_popup(st) then return end

    local fname = plugin.FIELDS[st.plugin_field] or "MODULE"
    local fval = plugin.loader_field_value(st, c, st.plugin_field)
    marquee_text("mod_row1", 13, 2, 125, "> " .. fname .. ": " .. fval)

    local idx, list = plugin.chosen_index(st, c)
    local item_name = "(NO MODULES)"
    if idx == 0 then
      item_name = "(none)"
    elseif list[idx] and c.modules and c.modules[list[idx]] then
      item_name = c.modules[list[idx]].name or ""
    end
    marquee_text("mod_row2", 23, 2, 125, item_name)
    return
  end

  -- Status bar
  local slot = c.slot_index and c.chain and c.chain[c.slot_index]
  local left_hdr
  local right_hdr
  if c.plugins then
    if c.slot_index then
      local sname = (slot and slot.name) or c.slot or "PLUGIN"
      left_hdr = string.format("%s%d:%s", lvl_short, c.slot_index, string.upper(sname))
    else
      left_hdr = string.format("%s:+ ADD", string.sub(lvl, 1, 3))
    end
    local p = c.page or 1
    local np = #(c.pages or {})
    if slot and not slot.active then
      right_hdr = np > 1 and string.format("BYP %d/%d", p, np) or "BYP"
    elseif np > 1 then
      right_hdr = string.format("P%d/%d", p, np)
    else
      right_hdr = string.sub(lvl, 1, 3)
    end
  else
    local pg_name = plugin.page_group_label(c.page_group or 1)
    left_hdr = lvl_short .. ":" .. pg_name
    local p = c.page or 1
    local np = #(c.pages or {})
    right_hdr = np > 1 and string.format("P%d/%d", p, np) or "CH"
  end

  deckard.screen.status(trunc(left_hdr, 13), trunc(right_hdr, 7))
  if draw_popup(st) then return end

  -- Hold PLUG-IN overlay hint for Slot / Level / Bypass / Remove
  if st.held_plugin then
    if c.plugins then
      local nslots = #(c.chain or {})
      local cur_pos = c.slot_index or (nslots + 1)
      local slot_name = (slot and slot.name) or "+ ADD SLOT"
      deckard.screen.text(13, 2, trunc(string.format("SLOT %d/%d: %s", cur_pos, nslots + 1, slot_name), 21), true)
      deckard.screen.text(23, 2, trunc("LVL:" .. string.sub(lvl, 1, 3) .. " <>  MUTE:BYP", 21), true)
    else
      local pg_name = plugin.page_group_label(c.page_group or 1)
      deckard.screen.text(13, 2, trunc(string.format("CH %d/4: %s", c.page_group or 1, pg_name), 21), true)
      deckard.screen.text(23, 2, trunc("LVL:" .. string.sub(lvl, 1, 3) .. " <>  TURN:CH", 21), true)
    end
    return
  end

  -- Empty (+) slot
  if c.plugins and not c.slot_index then
    deckard.screen.text(13, 2, "EMPTY SLOT (+)", true)
    deckard.screen.text(23, 2, "PUSH ENC: LOAD PLUGIN", true)
    return
  end

  -- Parameter view
  local idx, knob = plugin.active_knob(st, s)
  if knob then
    draw_knob_row(idx, knob)
  else
    deckard.screen.text(16, 2, "NO PARAMETERS", true)
  end
end

local function draw_navigate(st, s)
  deckard.screen.status("NAVIGATE", s.song_view and "SONG" or "IDEAS")
  if draw_popup(st) then return end
  deckard.screen.text(13, 2, trunc(string.format("GRP %d  SND %02d", s.group or 1, s.sound or 1), 21), true)
  deckard.screen.text(23, 2, trunc(string.format("S%d P%d OCT%+d", s.scene or 1, s.pattern or 1, st.octave or 0), 21), true)
end

local function draw_grid(st, s)
  local g = s.grid or {}
  local p = st.grid_param or 1
  local pname = overlays.GRID_PARAMS[p] or "STEP"
  local val
  if p == 1 then
    val = overlays.GRID_STEPS[(g.step or 4) + 1] or "1/16"
  elseif p == 2 then
    val = g.step_snap and "ON" or "OFF"
  elseif p == 3 then
    val = overlays.GRID_NUDGES[(g.nudge or 0) + 1] or "STEP"
  else
    val = g.quick and "ON" or "OFF"
  end
  deckard.screen.status("GRID", pname)
  if draw_popup(st) then return end
  deckard.screen.text(13, 2, trunc(string.format("> %s: %s", pname, val), 21), true)
  local step_lbl = overlays.GRID_STEPS[(g.step or 4) + 1] or "1/16"
  deckard.screen.text(23, 2, trunc("STEP " .. step_lbl .. (g.step_snap and " [SNAP]" or " [FREE]"), 21), true)
end

local SAMPLING_TABS = { "RECORD", "EDIT", "SLICE", "ZONE" }

local function draw_sampling(st, s)
  local sm = s.sampling or {}
  local tab_name = SAMPLING_TABS[sm.tab or 1] or "RECORD"
  deckard.screen.status("SAMPLING", tab_name)
  if draw_popup(st) then return end
  local title
  if sm.sample and sm.sample.name and sm.sample.name ~= "" then
    title = sm.sample.name
  elseif sm.no_sample and sm.no_sample.title then
    title = sm.no_sample.title
  else
    title = "SAMPLER"
  end
  marquee_text("sampling_row1", 13, 2, 125, title)
  local sub = "TURN: TAB"
  if sm.record and sm.record.recording then
    sub = "RECORDING..."
  elseif sm.record and sm.record.waiting then
    sub = "WAITING INPUT"
  elseif sm.sample and sm.sample.slices then
    sub = string.format("SLICES: %d", #sm.sample.slices)
  elseif sm.no_sample and sm.no_sample.detail then
    sub = sm.no_sample.detail
  end
  marquee_text("sampling_row2", 23, 2, 125, sub)
end

local function draw_perform(st, s)
  local g = s.groups and s.groups[s.group]
  local cur = (g and g.perform and g.perform.mode) or 1
  local name = (g and g.perform and g.perform.name) or overlays.PERFORM_MODES[cur] or "Filter"
  deckard.screen.status("PERFORM FX", string.format("%d/%d", cur, #overlays.PERFORM_MODES))
  if draw_popup(st) then return end
  marquee_text("perform_name", 13, 2, 125, "> " .. name)
  deckard.screen.text(23, 2, "PAD 1-8/TURN  OK:ENC", true)
end

local function draw_quick_param(st, s)
  local info = st.quick_info(s)
  if not info then return end
  local tag = st.quick_held and "HOLD" or "LATCH"
  deckard.screen.status(info.header, tag)
  deckard.screen.text(13, 2, trunc(info.target, 21), true)
  deckard.screen.text(23, 2, trunc(info.value, 21), true)
end

function S.draw(st, s)
  if not deckard.screen then return end
  deckard.screen.clear()

  local sampling_on = st.sampling_is_open(s)
  local qp = st.active_quick_param()
  local view_key = qp or st.overlay or (st.dup_active and "dup") or (sampling_on and "sampling") or (st.browse and "browse") or st.mode
  if view_key ~= last_view_key then
    marquee_state = {}
    last_view_key = view_key
  end

  if qp then
    draw_quick_param(st, s)
    return
  end

  if st.overlay == "scene" then
    draw_scene(st, s)
    return
  elseif st.overlay == "pattern" then
    draw_pattern(st, s)
    return
  elseif st.overlay == "events" then
    draw_events(st, s)
    return
  elseif st.overlay == "grid" then
    draw_grid(st, s)
    return
  elseif st.overlay == "variation" then
    draw_variation(st, s)
    return
  elseif st.overlay == "navigate" then
    draw_navigate(st, s)
    return
  elseif st.overlay == "plugin" then
    draw_plugin(st, s)
    return
  elseif st.overlay == "perform" then
    draw_perform(st, s)
    return
  elseif st.dup_active then
    draw_duplicate(st)
    return
  elseif sampling_on and st.overlay == nil then
    draw_sampling(st, s)
    return
  elseif st.overlay == nil and st.browse then
    draw_browse(st, s)
    return
  end

  -- Top status bar (rows 0..9)
  local mode_label = string.upper(st.mode) .. " MODE"
  if st.sixteen_vel and st.mode == "pad" then
    mode_label = "16 VELOCITIES"
  end
  if st.held_group then
    mode_label = "SELECT GROUP"
  elseif st.held_erase then
    mode_label = "ERASE"
  elseif st.held_select then
    mode_label = "SELECT"
  end

  local right_label = string.format("%.1f BPM", s.tempo or 120.0)
  if st.sixteen_vel then
    right_label = "16V"
  elseif s.fixed_velocity then
    right_label = string.format("FV %d", s.velocity or 100)
  end

  deckard.screen.status(trunc(mode_label, 14), trunc(right_label, 9))

  if draw_popup(st) then return end

  -- Base mode screens (rows 11..31)
  local play_state = s.playing and "PLAY" or "STOP"
  if s.recording then play_state = "REC" end

  if st.mode == "pad" then
    local grp_name = string.format("GRP %d: SOUND %02d", s.group or 1, s.sound or 1)
    deckard.screen.text(13, 4, grp_name, true)
    local left_bot = st.sixteen_vel and "VEL 8..127" or string.format("PAD %02d", s.sound or 1)
    deckard.screen.text(23, 4, left_bot, true)
    deckard.screen.text_right(23, 124, play_state, true)
  elseif st.mode == "keyboard" then
    local root_pitch = 60 + st.octave * 12
    deckard.screen.text(13, 4, string.format("SCALE: ROOT %d", root_pitch), true)
    deckard.screen.text(23, 4, string.format("OCT: %+d", st.octave), true)
    deckard.screen.text_right(23, 124, play_state, true)
  elseif st.mode == "chords" then
    local root_pitch = 48 + st.octave * 12
    deckard.screen.text(13, 4, string.format("CHORDS: ROOT %d", root_pitch), true)
    deckard.screen.text(23, 4, string.format("OCT: %+d", st.octave), true)
    deckard.screen.text_right(23, 124, play_state, true)
  elseif st.mode == "step" then
    local info = st.step_info(s)
    local hdr = info.segments > 1
      and string.format("STEP SEQ P%d/%d", info.seg + 1, info.segments)
      or "16-STEP SEQUENCER"
    deckard.screen.text(13, 4, hdr, true)
    for step = 1, 16 do
      local x = 4 + (step - 1) * 7
      local offset = (step - 1 + info.seg * 16) * info.step_ticks
      local has_step = offset < info.range_w and st.has_step_note(s, info.first + offset, info.step_ticks)
      if info.playing_step == step or has_step then
        deckard.screen.rect(23, x, 7, 5, true)
      else
        deckard.screen.frame_rect(23, x, 7, 5, true)
      end
    end
  end
end

return S
