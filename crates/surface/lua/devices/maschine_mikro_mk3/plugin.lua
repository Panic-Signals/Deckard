-- Maschine Mikro MK3 PLUG-IN / MACRO & Module Loader overlay.
-- Clean-room Deckard Lua script.

local overlays = require("overlays")

local P = {}

local LEVELS = { "MASTER", "GROUP", "SOUND" }
local PAGE_GROUPS = { "INPUT", "OUTPUT", "GROOVE", "MACRO" }
local FORMATS = { "Internal", "Audio Units", "VST3", "VST" }
local NI = "NI"
local OWN = "DECKARD"

P.FIELDS = { "KIND", "FORMAT", "VENDOR", "MODULE" }

local function vendor_of(m)
  if m.vendor == "Native Instruments" then return NI end
  return m.vendor or ""
end

-- A Sound's first slot (or empty Sound chain) can take Instruments as well as Effects.
local function instrument_slot(c)
  return c and c.level == 3 and (#(c.chain or {}) == 0 or c.slot_index == 1)
end

local function current_kind(st, c)
  return instrument_slot(c) and st.plugin_kind or 2
end

local function of_kind(c, k)
  local out = {}
  local want_inst = (k == 1)
  for i, m in ipairs((c and c.modules) or {}) do
    if (m.instrument == true) == want_inst then
      out[#out + 1] = i
    end
  end
  return out
end

local function available_formats(c, k)
  local out = {}
  local kind_idxs = of_kind(c, k)
  for _, fmt in ipairs(FORMATS) do
    for _, idx in ipairs(kind_idxs) do
      if c.modules[idx].format == fmt then
        out[#out + 1] = fmt
        break
      end
    end
  end
  return out
end

local function available_vendors(c, k, fmt)
  if fmt == "Internal" then return { OWN } end
  local out, seen = {}, {}
  for _, idx in ipairs(of_kind(c, k)) do
    local m = c.modules[idx]
    local v = vendor_of(m)
    if m.format == fmt and not seen[v] then
      seen[v] = true
      if v == NI then table.insert(out, 1, v) else out[#out + 1] = v end
    end
  end
  return out
end

local function index_of(list, target)
  for i, v in ipairs(list) do
    if v == target then return i end
  end
  return nil
end

local function settle(st, c)
  local k = current_kind(st, c)
  local fs = available_formats(c, k)
  if not index_of(fs, st.plugin_format) then
    st.plugin_format = fs[1] or "Internal"
  end
  local vs = available_vendors(c, k, st.plugin_format)
  if not index_of(vs, st.plugin_vendor) then
    st.plugin_vendor = vs[1] or OWN
  end
  return k
end

function P.module_list(st, c)
  local k = settle(st, c)
  local out = {}
  for _, idx in ipairs(of_kind(c, k)) do
    local m = c.modules[idx]
    if m.format == st.plugin_format and (st.plugin_format == "Internal" or vendor_of(m) == st.plugin_vendor) then
      out[#out + 1] = idx
    end
  end
  return out
end

local function loader_key(st, c)
  return current_kind(st, c) .. "/" .. (st.plugin_format or "Internal") .. "/" .. (st.plugin_vendor or OWN)
end

function P.chosen_index(st, c)
  local key = loader_key(st, c)
  local list = P.module_list(st, c)
  local has_slot = c and c.slot_index ~= nil
  local min_idx = (has_slot or #list == 0) and 0 or 1
  local cur = st.plugin_chosen[key]
  if cur == nil then
    cur = min_idx
  end
  return st.clamp(cur, min_idx, #list), list, has_slot
end

local function field_available(st, c, f)
  local k = settle(st, c)
  if f == 1 then return instrument_slot(c) end
  if f == 2 then return #available_formats(c, k) > 1 end
  if f == 3 then return st.plugin_format ~= "Internal" and #available_vendors(c, k, st.plugin_format) > 1 end
  return f == 4
end

local function next_loader_field(st, c, cur, dir)
  local f = cur + dir
  while f >= 1 and f <= #P.FIELDS do
    if field_available(st, c, f) then return f end
    f = f + dir
  end
  return cur
end

function P.loader_field_value(st, c, f)
  local k = settle(st, c)
  if f == 1 then
    return k == 1 and "INSTRUMENT" or "EFFECT"
  elseif f == 2 then
    return st.plugin_format or "Internal"
  elseif f == 3 then
    return st.plugin_vendor or OWN
  else
    local idx, list = P.chosen_index(st, c)
    return string.format("%d/%d", idx, #list)
  end
end

function P.active_knob(st, s)
  local c = s.control or {}
  local knobs = c.knobs
  if st.plugin_wrap and c.page ~= st.plugin_wrap_page then
    local picked = (st.plugin_wrap == "last") and overlays.last_active_knob(knobs) or overlays.first_active_knob(knobs)
    st.plugin_wrap = nil
    st.plugin_wrap_page = nil
    if picked then
      st.plugin_knob = picked
      return picked, knobs[picked]
    end
  end
  if overlays.is_active_knob(knobs, st.plugin_knob) then
    return st.plugin_knob, knobs[st.plugin_knob]
  end
  local first = overlays.first_active_knob(knobs)
  if first then
    st.plugin_knob = first
    return first, knobs[first]
  end
  return nil, nil
end

function P.level_label(level)
  return LEVELS[level] or "SOUND"
end

function P.page_group_label(pg)
  return PAGE_GROUPS[pg] or "INPUT"
end

local function enter_loader(st, c)
  st.plugin_loader = true
  st.plugin_field = 4
  st.plugin_kind = instrument_slot(c) and 1 or 2
  settle(st, c)
end

local function exit_plugin(st)
  st.overlay = nil
  st.overlay_pinned = false
  st.plugin_loader = false
  st.plugin_wrap = nil
  st.plugin_wrap_page = nil
end

function P.button(st, canon, down)
  local s = deckard.state()
  local c = s.control or {}
  local shift = st.shift()

  -- PLUG-IN & SHIFT + PLUG-IN (Channel / Macro)
  if canon == "PLUGIN" then
    if down then
      st.clear_quick_param()
      if shift then
        st.plugin_shift_down = true
        st.held_plugin = false
        st.plugin_used = false
        st.plugin_loader = false
        st.plugin_wrap = nil
        if st.overlay == "plugin" and c.plugins == false and c.page_group == 4 then
          exit_plugin(st)
        else
          st.overlay = "plugin"
          st.overlay_pinned = true
          st.dup = nil
          st.plugin_knob = 1
          deckard.show_plugins(false)
          deckard.page_group(4)
        end
      else
        st.plugin_shift_down = false
        st.held_plugin = true
        st.plugin_used = false
        if st.overlay ~= "plugin" then
          st.overlay = "plugin"
          st.overlay_pinned = true
          st.dup = nil
          st.plugin_loader = false
          st.plugin_knob = 1
          st.plugin_wrap = nil
          deckard.show_plugins(true)
          st.plugin_used = true
        elseif st.plugin_loader then
          st.plugin_loader = false
          st.plugin_used = true
        elseif c.plugins == false then
          st.plugin_knob = 1
          st.plugin_wrap = nil
          deckard.show_plugins(true)
          st.plugin_used = true
        end
      end
    else
      if st.plugin_shift_down then
        st.plugin_shift_down = false
        return true
      end
      local was_held = st.held_plugin
      local was_used = st.plugin_used
      st.held_plugin = false
      st.plugin_used = false
      if was_held and not was_used and st.overlay == "plugin" then
        exit_plugin(st)
      end
    end
    return true
  end

  if st.overlay ~= "plugin" then return false end

  -- While holding PLUG-IN: MUTE toggles bypass, ERASE removes focused slot (only without Shift so SHIFT+MUTE Choke still works).
  if st.held_plugin and not shift then
    if canon == "MUTE" then
      if down and c.plugins and c.slot_index then
        local slot = c.chain and c.chain[c.slot_index]
        deckard.slot_edit("bypass")
        st.plugin_used = true
        st.popup(slot and slot.active and "BYPASSED" or "ACTIVE")
      end
      return true
    elseif canon == "ERASE" then
      if down and c.plugins and c.slot_index then
        deckard.slot_edit("remove")
        st.plugin_used = true
        st.plugin_knob = 1
        st.popup("REMOVED")
      end
      return true
    end
  end

  if canon ~= "LEFT" and canon ~= "RIGHT" then return false end
  if not down then return true end
  local dir = canon == "RIGHT" and 1 or -1

  -- Hold PLUG-IN + LEFT / RIGHT switches chain level (MASTER / GROUP / SOUND).
  if st.held_plugin then
    st.plugin_used = true
    st.plugin_loader = false
    st.plugin_knob = 1
    st.plugin_wrap = nil
    local to = st.clamp((c.level or 3) + dir, 1, 3)
    if to ~= (c.level or 3) then
      deckard.level(to)
      st.popup("LEVEL", LEVELS[to])
    end
    return true
  end

  -- Module Loader field navigation
  if st.plugin_loader then
    if not field_available(st, c, st.plugin_field) then
      st.plugin_field = 4
    end
    st.plugin_field = next_loader_field(st, c, st.plugin_field, dir)
    return true
  end

  -- Parameter page navigation (SHIFT + LEFT / RIGHT)
  local page = c.page or 1
  local pages = #(c.pages or {})
  if shift then
    if dir > 0 and page < pages then
      deckard.next_page()
      st.plugin_knob = 1
      st.plugin_wrap = nil
    elseif dir < 0 and page > 1 then
      deckard.previous_page()
      st.plugin_knob = 1
      st.plugin_wrap = nil
    end
    return true
  end

  -- Parameter navigation (LEFT / RIGHT), wrapping across pages cleanly
  local knobs = c.knobs
  local cur = P.active_knob(st, s)
  if cur then
    local nxt = overlays.next_active_knob(knobs, cur, dir)
    if nxt ~= cur then
      st.plugin_knob = nxt
      st.plugin_wrap = nil
    elseif dir > 0 and page < pages then
      deckard.next_page()
      st.plugin_wrap = "first"
      st.plugin_wrap_page = page
    elseif dir < 0 and page > 1 then
      deckard.previous_page()
      st.plugin_wrap = "last"
      st.plugin_wrap_page = page
    end
  elseif dir > 0 and page < pages then
    deckard.next_page()
  elseif dir < 0 and page > 1 then
    deckard.previous_page()
  end
  return true
end

function P.encoder(st, steps)
  if st.overlay ~= "plugin" or steps == 0 then return false end
  local s = deckard.state()
  local c = s.control or {}
  local shift = st.shift()

  -- Hold PLUG-IN + turn encoder steps Slots (Plug-ins) or Page Group (Channel)
  if st.held_plugin then
    st.plugin_used = true
    st.plugin_loader = false
    st.plugin_knob = 1
    st.plugin_wrap = nil
    if c.plugins then
      deckard.slot(steps)
    else
      local to = st.clamp((c.page_group or 1) + steps, 1, c.page_groups or 4)
      deckard.page_group(to)
    end
    return true
  end

  -- Module Loader encoder scrolling
  if st.plugin_loader then
    if not field_available(st, c, st.plugin_field) then
      st.plugin_field = 4
    end
    local f = st.plugin_field
    if f == 1 and instrument_slot(c) then
      st.plugin_kind = st.clamp(st.plugin_kind + steps, 1, 2)
      settle(st, c)
    elseif f == 2 then
      local fs = available_formats(c, current_kind(st, c))
      local cur = index_of(fs, st.plugin_format) or 1
      st.plugin_format = fs[st.clamp(cur + steps, 1, math.max(1, #fs))] or st.plugin_format
      settle(st, c)
    elseif f == 3 then
      local vs = available_vendors(c, current_kind(st, c), st.plugin_format)
      local cur = index_of(vs, st.plugin_vendor) or 1
      st.plugin_vendor = vs[st.clamp(cur + steps, 1, math.max(1, #vs))] or st.plugin_vendor
    else
      local idx, list, has_slot = P.chosen_index(st, c)
      local min_idx = (has_slot or #list == 0) and 0 or 1
      st.plugin_chosen[loader_key(st, c)] = st.clamp(idx + steps, min_idx, #list)
    end
    return true
  end

  -- Parameter editing
  local idx = P.active_knob(st, s)
  if idx then
    deckard.step_knob(idx, steps, shift)
  end
  return true
end

function P.encoder_press(st)
  if st.overlay ~= "plugin" then return false end
  local s = deckard.state()
  local c = s.control or {}
  local shift = st.shift()

  if not c.plugins then return true end

  if st.plugin_loader then
    if shift then
      st.plugin_loader = false
      return true
    end
    if not field_available(st, c, st.plugin_field) then
      st.plugin_field = 4
    end
    if st.plugin_field < 4 then
      st.plugin_field = 4
      return true
    end
    local idx, list, has_slot = P.chosen_index(st, c)
    if idx == 0 then
      if has_slot then
        deckard.load_module(nil)
        st.popup("REMOVED")
      end
      st.plugin_loader = false
    elseif list[idx] then
      local mod = c.modules[list[idx]]
      deckard.load_module(list[idx])
      st.plugin_loader = false
      st.popup("LOADED", mod and mod.name or "")
    else
      st.plugin_loader = false
    end
    return true
  end

  if shift then
    local slot = c.slot_index and c.chain and c.chain[c.slot_index]
    if slot and slot.plugin then
      deckard.slot_edit("edit")
      st.popup("OPEN EDITOR")
    end
    return true
  end

  enter_loader(st, c)
  return true
end

function P.leds(st, s)
  deckard.led("PLUGIN", st.overlay == "plugin" and "on" or "dim")
  if st.overlay ~= "plugin" then return end

  local c = s.control or {}
  if st.held_plugin then
    deckard.led("LEFT", (c.level or 3) > 1 and "dim" or "off")
    deckard.led("RIGHT", (c.level or 3) < 3 and "dim" or "off")
    return
  end

  if st.plugin_loader then
    if not field_available(st, c, st.plugin_field) then
      st.plugin_field = 4
    end
    local prev = next_loader_field(st, c, st.plugin_field, -1) ~= st.plugin_field
    local nxt = next_loader_field(st, c, st.plugin_field, 1) ~= st.plugin_field
    deckard.led("LEFT", prev and "dim" or "off")
    deckard.led("RIGHT", nxt and "dim" or "off")
    return
  end

  local page = c.page or 1
  local pages = #(c.pages or {})
  if st.shift() then
    deckard.led("LEFT", page > 1 and "dim" or "off")
    deckard.led("RIGHT", page < pages and "dim" or "off")
    return
  end

  local cur = P.active_knob(st, s)
  local prev = (cur and overlays.next_active_knob(c.knobs, cur, -1) ~= cur) or page > 1
  local nxt = (cur and overlays.next_active_knob(c.knobs, cur, 1) ~= cur) or page < pages
  deckard.led("LEFT", prev and "dim" or "off")
  deckard.led("RIGHT", nxt and "dim" or "off")
end

return P
