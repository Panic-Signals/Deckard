-- The Sampling page (SAMPLE): the focused sound's sample and the recorder, a tab at a
-- time, as Maschine's (docs/sampling-controller.md).
--
--   display 1-4       the tab: RECORD, EDIT, SLICE, ZONE
--   display 4 again   a sound with layers (a New Sampler's): between ZONE and LAYERS
--   LAYERS            knobs 1-6 the page's (LAYER, ENVELOPE, MORPH, WINDOW), knob 7
--                     the focused layer, knob 8 its LEVEL; 5-8 mute the four layers
--                     shown (SHIFT: solo); SHIFT + 1 REMOVE the focused layer, SHIFT
--                     + 2 SPREAD their windows evenly; the pads play the sounds. The
--                     left display: every layer's window on the morph and where it
--                     is; the right: the four layers. (A layered sound has no SLICE.)
--   RECORD 5-8        START (WAITING, STOP), DELETE (CANCEL), PREVIOUS, NEXT take
--   EDIT 5-6 / 8      ◀ the edit function ▶, APPLY (STEMS: CANCEL while separating)
--   EDIT, STEM …      a zone with stem lanes has three more edit functions: STEM ROOT,
--                     STEM TEMPO, STEM ANALYZE (both). Button 7 goes round its stems
--                     (the bass first for the root, the drums for the tempo); APPLY
--                     sets the zone's from that stem alone
--   EDIT 7            ANALYZE: a New Sampler zone's tempo and root heard again (the
--                     part it plays; its bass stem, if it has one, settles the key)
--   SHIFT+EDIT 5-8    a New Sampler zone's tempo: X2, 1/2, TO PROJECT, MATCH KEY (its
--                     knob pages: TEMPO / PITCH (BEATS: the sample's length in whole
--                     beats, its tempo following: "8 (2 bars)"), SYNC, WARP; its edit functions go on
--                     with WARP MARKERS, QUANTIZE AUDIO, CLEAR WARP; TO (NEW) SAMPLER
--                     converts the sound's Sampler)
--   EDIT, PLAYBACK    a New Sampler slice: its SPEED against the song, REVERSE, its KEY
--   EDIT, TAIL / MOTION  its tail at its end, and the sound's Slice Motion
--   EDIT, STEMS page  a zone with stem lanes: knobs 1-4 their levels, 5-8 mute them
--                     (SHIFT: solo); the right display shows the lanes stacked
--   SLICE 5-8         SPLIT, REMOVE the focused slice, DELETE ALL, APPLY… (the slices
--                     made zones of the focused sound)
--   SHIFT+SLICE 5-8   KEY SHIFT (the focused slice onto the group's next empty sound,
--                     over all its keys), RE-ROLL (Random: pick again), BANK (the
--                     pads' next sixteen slices), KEEP (Find and Random: the focused
--                     slice kept through re-rolls, lit bright; UNKEEP lets it go)
--   ZONE 5-8          REMOVE the focused zone, ADD, PREVIOUS, NEXT zone
--   LEFT / RIGHT      the tab's knob pages
--   knobs 1-4         the page's settings (SHIFT: finer)
--   knobs 5 / 6       zoom / scroll the sample (their icons shown while touched)
--   pads              SLICE: pick a slice and hear it; Manual, while the sample plays,
--                     an empty pad drops a slice there, on the nearest 1/8 note;
--                     else the sounds, as ever. SHIFT + a pad is the shortcut printed
--                     under it on every tab (undo, redo...)

local widgets = require("lib.widgets")

local page = {}

local TABS = { "RECORD", "EDIT", "SLICE", "ZONE" }

-- The EDIT functions, in Maschine's order: what APPLY asks for. (No STRETCH yet.)
local FUNCTIONS = {
  { "TRUNCATE", 1 }, { "NORMALIZE", 2 }, { "REVERSE", 3 }, { "FADE IN", 4 }, { "FADE OUT", 5 },
  { "FIX DC", 6 }, { "SILENCE", 7 }, { "CUT", 8 }, { "COPY", 9 }, { "PASTE", 10 }, { "DUPLICATE", 11 },
  { "STEMS", "stems" }, { "AUTOLOOP", "auto_loop" },
}
local func = 1
-- A New Sampler zone with a tempo has its warp markers too; a Sampler converts.
-- A zone with stem lanes: its root or tempo as one stem alone has it (the third: what
-- `lane_hear` is asked for; the fourth: the stem first offered).
local STEM = { { "STEM ROOT", "lane_hear", 1, "BASS" }, { "STEM TEMPO", "lane_hear", 2, "DRUMS" }, { "STEM ANALYZE", "lane_hear", 3, "BASS" } }
-- The stem picked for each (an index into the lanes shown), once button 7 has.
local stem_pick = {}
local WARP = { { "WARP MARKERS", "warp_markers" }, { "QUANTIZE AUDIO", "warp_quantize" }, { "CLEAR WARP", "warp_clear" } }

-- The knob page shown on each tab, the slice focused (from 1; none until a pad picks
-- one: the buttons then act on the first), the knobs touched.
local at = { 1, 1, 1, 1, 1 }
local slice = nil
local touched = {}

-- Turns gathered toward each setting's next step, and the value last asked for (with
-- the one shown then: until the app's catches up, the next step goes on from it).
local turns = {}
local asked = {}

-- How far knob 5 zooms a turn's hundredth, and how far knob 6 scrolls a turn.
local ZOOM = 1.06
local SCROLL = 2
-- The fewest frames the view shows.
local MIN_VIEW = 64

local function view()
  return deckard.state().sampling
end

-- The slicer's mode, by name ("Manual", "Find", …).
local function slicer_mode(v)
  local k = v.slice["Slice.Mode"]
  return k and k.text or ""
end

-- The first slice before the sixteen on the pads: the selected slice's bank (of the
-- slices there are now).
local function pad_bank(s)
  local n = s and #s.slices or 0
  return (math.max(1, math.min(slice or 1, math.max(n, 1))) - 1) // 16 * 16
end

-- The SLICE tab's own pages, from the slicer's knobs (by key): what page 1 shows
-- follows the slicer's mode.
local function slice_pages(v)
  local k = v.slice
  local mode = k["Slice.Mode"] and k["Slice.Mode"].text or ""
  local slicer = { k["Slice.Mode"] }
  if mode == "Detect" then
    slicer[2], slicer[3] = k["Slice.Density"], k["Slice.ZeroX"]
  elseif mode == "Split" then
    slicer[2] = k["Slice.Split"]
  elseif mode == "Find" then
    slicer[2] = k["Slice.Find"]
  elseif mode == "Random" then
    slicer[2] = k["Slice.Random"]
  elseif mode == "Grid" then
    slicer[2], slicer[3], slicer[4] = k["Slice.Grid"], k["Slice.BpmMode"], k["Slice.Bpm"]
  end
  local s = v.sample
  local n = s and #s.slices or 0
  local edit = {
    { name = "SLICE", text = (n > 0 and slice) and (math.min(slice, n) .. "/" .. n) or "-/-", slice = true },
  }
  if s and n > 0 and slice then
    local i = math.min(slice, n)
    local from, to = s.slices[i], s.slices[i + 1] or s.play[2]
    local frames = function(f) return string.format("%.3f s", f / math.max(s.rate, 1)) end
    edit[3] = { name = "START", text = frames(from), frame = from, move = i }
    edit[4] = { name = "END", text = frames(to), frame = to, move = s.slices[i + 1] and i + 1 or nil }
  end
  return {
    { name = "Slicer", knobs = slicer },
    { name = "Edit", knobs = edit },
    { name = "Apply", knobs = { k["Slice.To"], k["Slice.Apply"], k["Slice.Order"], k["Slice.Bpm"] } },
  }
end

-- The tab's knob pages.
local function pages(v)
  if v.tab == 3 then return slice_pages(v) end
  return v.pages[v.tab] or {}
end

local function knob_page(v)
  local list = pages(v)
  local n = math.max(1, math.min(at[v.tab], #list))
  return list[n], n, #list
end

-- How many tabs there are: an Audio plug-in's take has RECORD and EDIT only.
local function tab_count(v)
  return (v.sample and v.sample.take) and 2 or #TABS
end

local function sample_ok(v)
  return v.sample ~= nil and v.sample.frames > 0
end

local function busy(v)
  return v.record.waiting or v.record.recording
end

local function stems_running(v)
  return v.stems ~= nil
end

-- A New Sampler zone: the app gives its EDIT tab a TEMPO / PITCH page.
local function tempo_page(v)
  for _, p in ipairs(v.pages[2] or {}) do
    if p.name == "TEMPO / PITCH" then return p end
  end
end

-- The EDIT functions for what's focused.
local function functions(v)
  local list = {}
  for _, f in ipairs(FUNCTIONS) do list[#list + 1] = f end
  local p = tempo_page(v)
  if p and p.knobs[2] and p.knobs[2].text ~= "-" then
    for _, f in ipairs(WARP) do list[#list + 1] = f end
  end
  if p and v.sample and v.sample.lanes and #v.sample.lanes > 0 then
    for _, f in ipairs(STEM) do list[#list + 1] = f end
  end
  if not (v.sample and v.sample.take) then
    list[#list + 1] = { p and "TO SAMPLER" or "TO NEW SAMPLER", "convert" }
  end
  return list
end

local function current(v)
  local list = functions(v)
  func = math.max(1, math.min(func, #list))
  return list[func], #list
end

-- SHIFT on EDIT, for a New Sampler zone with a tempo.
local function tempo_buttons(v)
  if v.tab ~= 2 or not deckard.held("SHIFT") then return nil end
  local p = tempo_page(v)
  if not p then return nil end
  local has = p.knobs[2] and p.knobs[2].text ~= "-"
  return {
    { text = "X2", disabled = not has, act = function() deckard.sampling_call("double_bpm") end },
    { text = "1/2", disabled = not has, act = function() deckard.sampling_call("halve_bpm") end },
    { text = "TO PROJECT", disabled = not has, act = function() deckard.sampling_call("to_project") end },
    { text = "MATCH KEY", disabled = not (p.knobs[4] and p.knobs[4].text ~= "-"), act = function() deckard.sampling_call("match_key") end },
  }
end

-- The stem lanes' colours, by lane: Drums, Vocals, Bass, Leads.
local LANE_COLOURS = { { 242, 162, 60 }, { 232, 88, 138 }, { 90, 156, 242 }, { 98, 199, 138 } }

-- EDIT's STEMS knob page shows (a zone with stem lanes): the display buttons mute its
-- lanes (lit: heard), SHIFT solos them.
local function stem_buttons(v)
  if v.tab ~= 2 or not (v.sample and v.sample.lanes and #v.sample.lanes > 0) then return nil end
  local p = knob_page(v)
  if not p or p.name ~= "STEMS" then return nil end
  local solo = deckard.held("SHIFT")
  local out = {}
  for i, l in ipairs(v.sample.lanes) do
    out[i] = {
      text = l.name,
      selected = solo and l.solo or (not solo and l.heard),
      act = function() deckard.sampling_call(solo and "lane_solo" or "lane_mute", l.lane) end,
    }
  end
  return out
end

-- The layers' colours, by lane (as the software's).
local LAYER_COLOURS = {
  { 242, 162, 60 }, { 232, 88, 138 }, { 90, 156, 242 }, { 98, 199, 138 },
  { 176, 124, 240 }, { 76, 201, 214 }, { 230, 210, 74 }, { 240, 116, 82 },
}

-- The focused sound's layers (none: not a layered sound).
local function layers_of(v)
  return v.sample and v.sample.layers or {}
end

-- The first layer before the four shown: the focused one's four.
local function layer_bank(s)
  return (math.max(1, s.layer or 1) - 1) // 4 * 4
end

-- LAYERS: the display buttons mute the four layers shown (lit: heard), SHIFT solos.
local function layer_buttons(v)
  local s = v.sample
  local out = {}
  if not s then return out end
  local solo = deckard.held("SHIFT")
  local bank = layer_bank(s)
  for i = 1, 4 do
    local l = s.layers[bank + i]
    if l then
      out[i] = {
        text = tostring(bank + i) .. (solo and " SOLO" or " MUTE"),
        selected = solo and l.solo or (not solo and l.heard),
        act = function() deckard.sampling_call(solo and "layer_solo" or "layer_mute", bank + i) end,
      }
    end
  end
  return out
end

-- RECORD: the takes on the pads (TAKES), and the bank of 16 shown.
local takes_on_pads, take_bank = false, 0

-- What display buttons 5-8 show and do on the tab: { text, act = function, … }.
local function buttons(v)
  local s = v.sample
  if v.tab == 5 then return layer_buttons(v) end
  if v.tab == 1 then
    if busy(v) then
      return {
        { text = v.record.waiting and "WAITING" or "STOP", act = function() deckard.record_sample() end },
        { text = "CANCEL", act = function() deckard.record_sample(true) end },
      }
    end
    local takes, take = v.takes, v.take
    -- SHIFT: the takes one at a time.
    if deckard.held("SHIFT") then
      return {
        nil,
        nil,
        { text = "PREVIOUS", disabled = takes == 0 or take == 1, act = function() deckard.take(take and take - 1 or takes) end },
        { text = "NEXT", disabled = takes == 0 or take == takes, act = function() deckard.take(take and take + 1 or 1) end },
      }
    end
    -- TAKES puts the takes on the pads (Maschine's "RecHistOnPads"), 16 a bank: a pad
    -- plays its take. BANK goes round the banks.
    local banks = math.max(1, (takes + 15) // 16)
    if take_bank >= banks then take_bank = 0 end
    return {
      { text = "START", act = function() deckard.record_sample() end },
      { text = "DELETE", disabled = not take, act = function() deckard.take(take, true) end },
      { text = "TAKES", selected = takes_on_pads, disabled = takes == 0 and not takes_on_pads, act = function() takes_on_pads = not takes_on_pads end },
      { text = "BANK " .. (take_bank + 1), disabled = not takes_on_pads or banks < 2, act = function() take_bank = (take_bank + 1) % banks end },
    }
  elseif v.tab == 2 then
    local t = stem_buttons(v) or tempo_buttons(v)
    if t then return t end
    local f = current(v)
    local cancel = f[2] == "stems" and v.stems and v.stems.focused
    if f[2] == "lane_hear" then
      -- The stem heard: the one picked, else the one first offered, else the first.
      local lanes = s.lanes
      local i = stem_pick[f[3]]
      if not i or i > #lanes then
        i = 1
        for j, l in ipairs(lanes) do
          if l.name:upper() == f[4] then i = j end
        end
      end
      return {
        { arrows = true, text = f[1] },
        nil,
        { text = lanes[i].name:upper(), act = function() stem_pick[f[3]] = i % #lanes + 1 end },
        { text = "APPLY", act = function() deckard.sampling_call("lane_hear", lanes[i].lane, f[3]) end },
      }
    end
    return {
      { arrows = true, text = f[1], disabled = not s },
      nil,
      -- (A New Sampler zone's: a Sampler's has no tempo or root to hear.)
      tempo_page(v) and { text = "ANALYZE", disabled = not s, act = function() deckard.sampling_call("reanalyze") end } or nil,
      {
        text = cancel and "CANCEL" or "APPLY",
        disabled = not s or (f[2] == "stems" and stems_running(v) and not cancel),
        act = function()
          if type(f[2]) == "number" then
            deckard.sampling_call("edit", f[2])
          elseif cancel then
            deckard.sampling_call("cancel_stems")
          else
            deckard.sampling_call(f[2])
          end
        end,
      },
    }
  elseif v.tab == 3 then
    local n = s and #s.slices or 0
    local i = math.min(slice or 1, math.max(n, 1))
    if deckard.held("SHIFT") then
      local out = { { text = "KEY SHIFT", disabled = n == 0, act = function() deckard.sampling_call("key_shift", i) end } }
      if slicer_mode(v) == "Random" then out[2] = { text = "RE-ROLL", act = function() deckard.sampling_call("reroll") end } end
      -- BANK: the pads' next sixteen slices (round to the first).
      local banks = math.max(1, (n + 15) // 16)
      local bank = (i - 1) // 16
      out[3] = {
        text = "BANK " .. (bank + 1) .. "/" .. banks,
        disabled = banks < 2,
        act = function() slice = (bank + 1) % banks * 16 + 1 end,
      }
      -- KEEP: the focused slice stays through re-rolls and other counts.
      local mode = slicer_mode(v)
      if mode == "Find" or mode == "Random" then
        local kept = false
        for _, j in ipairs(s and s.kept or {}) do kept = kept or j == i end
        out[4] = { text = kept and "UNKEEP" or "KEEP", disabled = n == 0, act = function() deckard.sampling_call("keep_slice", i) end }
      end
      return out
    end
    return {
      { text = "SPLIT", disabled = n == 0, act = function() deckard.sampling_call("split_slice", i) end },
      { text = "REMOVE", disabled = n == 0, act = function() deckard.sampling_call("remove_slice", i) end },
      { text = "DELETE ALL", disabled = n == 0, act = function() deckard.sampling_call("clear_slices") end },
      { text = "APPLY…", disabled = n == 0, act = function() deckard.sampling_call("apply_slices") end },
    }
  end
  local zones = s and #s.zones or 0
  local z = s and s.focus_zone or 1
  return {
    { text = "REMOVE", disabled = zones == 0, act = function() deckard.sampling_call("remove_zone", z) end },
    -- (A sample browsed for into a new zone: not on the controller yet.)
    { text = "ADD", disabled = true },
    { text = "PREVIOUS", disabled = z <= 1, act = function() deckard.sampling_call("focus_zone", z - 1) end },
    { text = "NEXT", disabled = z >= zones, act = function() deckard.sampling_call("focus_zone", z + 1) end },
  }
end

function page.button(name, down)
  local v = view()
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if n then
    if not down then return true end
    if n <= 4 then
      local layers = #layers_of(v)
      if v.tab == 5 and deckard.held("SHIFT") and n <= 2 then
        if layers > 0 then
          if n == 1 then deckard.sampling_call("remove_layer", v.sample.layer) else deckard.sampling_call("spread_layers") end
        end
      elseif n == 4 and v.tab == 4 and layers > 0 then
        deckard.sampling_tab(5)
      elseif n == 3 and layers > 0 then
        -- (A layered sound has no slices.)
      elseif n <= tab_count(v) then
        deckard.sampling_tab(n)
      end
      return true
    end
    if v.tab == 2 and (n == 5 or n == 6) and not stem_buttons(v) and not tempo_buttons(v) then
      if v.sample then
        local _, count = current(v)
        func = (func + (n == 6 and 1 or -1) - 1) % count + 1
      end
      return true
    end
    local b = buttons(v)[n - 4]
    if b and b.act and not b.disabled then b.act() end
    return true
  end
  if name == "LEFT" or name == "RIGHT" then
    if down then
      local _, p, count = knob_page(v)
      at[v.tab] = math.max(1, math.min(count, p + (name == "RIGHT" and 1 or -1)))
    end
    return true
  end
  local k = tonumber(name:match("^CAP_(%d)$"))
  if k then touched[k] = down or nil end
end

-- Turn setting `k` (a knob's, as the app gives it) by `amount`: a step each time the
-- turns make its `turn`. With SHIFT held, multi-frame steps subdivide down to single
-- frames so fine turns move smoothly instead of waiting for a coarse jump.
local function turn(k, amount, send)
  local shift = deckard.held("SHIFT")
  local step, each = k.step, k.turn
  if shift and step > 1 and step == math.floor(step) and each <= 0.01 then
    each = each / step
    step = 1
  end
  local fine = shift and 0.1 or 1
  local g = (turns[k.key] or 0) + amount * fine
  local steps = g >= 0 and math.floor(g / each + 1e-9) or -math.floor(-g / each + 1e-9)
  turns[k.key] = g - steps * each
  if steps == 0 then return end
  local a = asked[k.key]
  local from = (a and a.shown == k.value) and a.value or k.value
  local to = math.max(k.min, math.min(k.max, from + steps * step))
  if to == from then return end
  asked[k.key] = { value = to, shown = k.value }
  send(to)
end

-- Knob 5 zooms about the middle of what's shown; knob 6 scrolls it.
local function zoom_scroll(v, n, amount)
  local s = v.sample
  if not sample_ok(v) or v.tab == 1 then return end
  local fine = deckard.held("SHIFT") and 0.1 or 1
  local a, b = s.view[1], s.view[2]
  local w = b - a
  if n == 5 then
    local nw = math.max(math.min(s.frames, MIN_VIEW), math.min(s.frames, w / ZOOM ^ (100 * amount * fine)))
    local mid = (a + b) / 2
    a = math.max(0, math.min(s.frames - nw, mid - nw / 2))
    b = a + nw
  else
    local d = SCROLL * w * amount * fine
    a = math.max(0, math.min(s.frames - w, a + d))
    b = a + w
  end
  if a ~= s.view[1] or b ~= s.view[2] then deckard.sampling_call("view", a, b) end
end

-- LAYERS: knobs 1-6 the page's, 7 the focused layer, 8 its level.
local function layer_knob(v, n, amount)
  local s = v.sample
  if not s or #s.layers == 0 then return end
  if n == 7 then
    local to = math.max(1, math.min(#s.layers, s.layer + require("lib.knobs").gather("layer", amount)))
    if to ~= s.layer then deckard.sampling_call("focus_layer", to) end
  elseif n == 8 then
    local l = s.layers[s.layer]
    if not l then return end
    local k = { key = "Layer.Level" .. s.layer, value = l.gain, min = 0, max = 1, step = 0.005, turn = 0.01 }
    turn(k, amount, function(to) deckard.sampling_call("layer_level", to) end)
  else
    local p, pn = knob_page(v)
    local k = p and p.knobs[n]
    if k then turn(k, amount, function(to) deckard.sampling_knob(pn, n, to) end) end
  end
end

function page.knob(n, amount)
  local v = view()
  if v.tab == 5 then
    layer_knob(v, n, amount)
    return true
  end
  if n >= 5 then
    zoom_scroll(v, n, amount)
    return true
  end
  if busy(v) then return true end
  local p, pn = knob_page(v)
  local k = p and p.knobs[n]
  if not k then return true end
  if k.slice then
    -- (The slice focused, a step at a time.)
    local count = v.sample and #v.sample.slices or 0
    local step = require("lib.knobs").gather("slice", amount)
    slice = math.max(1, math.min(count, (slice or 0) + step))
  elseif k.move then
    local s = v.sample
    local t = { key = "Slice.Move" .. k.move, value = k.frame, min = 0, max = s.frames - 1, step = 1, turn = 1 / math.max(100, s.view[2] - s.view[1]) }
    turn(t, amount, function(to) deckard.sampling_call("move_slice", k.move, to) end)
  elseif k.key then
    local knob, kpage = n, pn
    if v.tab == 3 then knob, kpage = k.index, 1 end
    turn(k, amount, function(to) deckard.sampling_knob(kpage, knob, to) end)
  end
  return true
end

function page.pad(n)
  local v = view()
  if v.tab == 1 and takes_on_pads then
    -- A take picked: it plays (Audio plug-in) or is loaded (Sampler).
    local k = take_bank * 16 + n
    if k <= v.takes then deckard.take(k) end
    return true
  end
  if v.tab ~= 3 then return end
  local s = v.sample
  if not s then return true end
  local i = pad_bank(s) + n
  -- (SHIFT + a pad never comes here: it is the pad's printed shortcut.)
  if s.slices[i] then
    slice = i
    deckard.sampling_call("prehear", s.slices[i], s.slices[i + 1] or s.play[2])
  elseif slicer_mode(v) == "Manual" and s.playhead then
    -- An empty pad while the sample plays: a slice there.
    deckard.sampling_call("catch_slice", 0)
  end
  return true
end

function page.pad_off(n)
  if view().tab == 3 then return true end
end

-- RECORD's and SLICE's pads play no sounds.
function page.pads()
  local tab = view().tab
  if tab == 1 or tab == 3 then return { play = false } end
end

function page.leds()
  local v = view()
  if not require("lib.util").sample_led(deckard.state()) then deckard.led("SAMPLE", "on") end
  local layered = #layers_of(v) > 0
  for i = 1, 4 do
    local on = i == v.tab or (i == 4 and v.tab == 5)
    local there = i <= tab_count(v) and not (i == 3 and layered)
    deckard.led("SCREEN_" .. i, on and "on" or (there and "dim" or "off"))
  end
  local b = buttons(v)
  -- (WAITING flashes, four times a second.)
  local flash = v.record.waiting and (deckard.now() * 4) % 2 < 1
  for i = 5, 8 do
    local spec = b[i - 4] or (v.tab == 2 and i == 6 and not stem_buttons(v) and not tempo_buttons(v) and b[1])
    local lit = spec and not spec.disabled and (spec.text or spec.arrows)
    deckard.led("SCREEN_" .. i, lit and (((i == 5 and flash) or spec.selected) and "on" or "dim") or "off")
  end
  local _, p, count = knob_page(v)
  deckard.led("LEFT", p > 1 and "dim" or "off")
  deckard.led("RIGHT", p < count and "dim" or "off")
  if v.tab == 1 and takes_on_pads then
    -- A pad a take: the focused one bright.
    for i = 1, 16 do
      local k = take_bank * 16 + i
      if k <= v.takes then
        deckard.led("PAD_" .. i, 16, k == v.take)
      else
        deckard.led("PAD_" .. i, "off")
      end
    end
  elseif v.tab == 1 then
    -- (RECORD's pads play nothing: dark.)
    for i = 1, 16 do deckard.led("PAD_" .. i, "off") end
  elseif v.tab == 3 then
    local s = v.sample
    local palette = s and s.palette or 16
    local bank = pad_bank(s)
    for i = 1, 16 do
      local k = bank + i
      if s and s.slices[k] then
        local kept = false
        for _, j in ipairs(s.kept or {}) do kept = kept or j == k end
        deckard.led("PAD_" .. i, palette, k == slice or kept)
      else
        deckard.led("PAD_" .. i, "off")
      end
    end
  end
end

-- "0.219 s" as Maschine shows it: the number, and its unit apart.
local function split(text)
  local value, unit = (text or ""):match("^([%-%+]?[%d%.:]+) (%S+)$")
  if value then return value, unit end
  return text or "", nil
end

local function draw_tabs(screen, theme, tab, count, layered)
  screen:frame(4, 4, 472, 24, "button", theme.corner)
  local xs, ws = { 4, 121, 240, 359 }, { 117, 119, 119, 117 }
  local names = { TABS[1], TABS[2], TABS[3], TABS[4] }
  if layered then names[3] = "" end
  if tab == 5 then
    -- (LAYERS is display button 4's other tab; SHIFT: what 1 and 2 do to the layers.)
    names[4] = "LAYERS"
    if deckard.held("SHIFT") and layered then names[1], names[2] = "REMOVE", "SPREAD" end
    tab = 4
  end
  for i, name in ipairs(names) do
    if i > count then break end
    if i == tab then
      screen:rect(xs[i], 4, ws[i], 24, "button_selected", theme.corner)
      widgets.chase(screen, theme, xs[i], 4, ws[i], 24)
    end
    screen:label(xs[i], 5, ws[i], 23, name, { font = theme.styles.default.font, size = theme.styles.default.size, color = i == tab and "button_selected_text" or "button_text" })
  end
end

-- The right screen's bar: the sample's (or take's) name and length.
local function sample_bar(screen, theme, v)
  screen:rect(480, 35, 480, 24, "bar")
  local s = v.sample
  if not s or v.tab == 1 then return end
  local st = { font = theme.styles.default.font, size = theme.styles.default.size, color = "text" }
  local small = { font = theme.styles.default.font, size = theme.styles.info_title.size, color = "weak" }
  local w = screen:text_width("LENGTH", small)
  screen:label(950 - w, 39, w, 16, "LENGTH", small, "center", "bottom")
  local len = string.format("%.3f s", s.frames / math.max(s.rate, 1))
  local lw = screen:text_width(len, st)
  screen:label(950 - w - 4 - lw, 38, lw, 18, len, st, "center", "bottom")
  screen:label(490, 38, 950 - w - lw - 500, 18, s.name, st, "left", "bottom")
end

-- How bright a waveform is where it plays (of its colour: 1 is all of it).
local WAVE_LIT = 1

-- A waveform of `peaks` (min, max a column) in (x, y, w, h), showing frames `a` to `b`
-- of `s`: in `colour` where it plays (or in a slice), faint elsewhere.
local function wave(screen, theme, s, peaks, x, y, w, h, a, b, colour, slices)
  local mid = y + h / 2
  local per = (b - a) / w
  local first = slices and s.slices[1] or s.play[1]
  local last = s.play[2]
  -- The selection's ground, and the focused slice's.
  local function ground(from, to, c)
    local x0 = math.floor((from - a) / per)
    local x1 = math.ceil((to - a) / per)
    x0, x1 = math.max(0, x0), math.min(w, x1)
    if x1 > x0 then screen:rect(x + x0, y, x1 - x0, h, c) end
  end
  if s.selection and not slices then ground(s.selection[1], s.selection[2], "wave_selection") end
  if slices and slice and s.slices[slice] then ground(s.slices[slice], s.slices[slice + 1] or last, "slice_focus") end
  local off = slices and "wave_unsliced" or "wave_off"
  -- What plays, at this much of its colour's brightness.
  local lit = type(colour) == "table" and widgets.blend(colour, { 0, 0, 0 }, WAVE_LIT) or colour
  for c = 0, w - 1 do
    local lo, hi = peaks[2 * c + 1], peaks[2 * c + 2]
    if lo then
      local f = a + (c + 0.5) * per
      local top = math.floor(mid - hi * h / 2)
      local bottom = math.ceil(mid - lo * h / 2)
      screen:rect(x + c, top, 1, math.max(1, bottom - top), (f >= first and f < last) and lit or off)
    end
  end
  local function mark(f, c, width)
    local px = math.floor((f - a) / per)
    if px >= 0 and px < w then screen:rect(x + px, y, width, h, c) end
  end
  if slices then
    for i, f in ipairs(s.slices) do
      if i ~= slice and i ~= (slice or 0) + 1 then mark(f, "slice_marker", 1) end
    end
    mark(s.play[1], "marker", 2)
    if slice and s.slices[slice] then
      mark(s.slices[slice], "marker", 2)
      mark(s.slices[slice + 1] or last, "marker", 2)
    end
  else
    mark(s.play[1], "marker", 2)
    mark(s.play[2], "marker", 2)
    if s.looped then
      local x0 = math.max(0, math.floor((s.looped[1] - a) / per))
      local x1 = math.min(w - 1, math.floor((s.looped[2] - a) / per))
      if x1 > x0 then screen:frame(x + x0, y, x1 - x0 + 1, h, "marker", 1) end
    end
  end
  if s.playhead then
    local c = theme.colors.text
    mark(s.playhead, { c[1], c[2], c[3], 198 }, 1)
  end
end

-- The zone map: the 128 keys across, velocity up, each zone over its keys and velocities.
local BLACK = { [1] = true, [3] = true, [6] = true, [8] = true, [10] = true }
local function zone_map(screen, theme, s, colour)
  local x0, y0, w, h = 0, 60, 480, 144
  local kw = w / 128
  for key = 0, 127 do
    local x = math.floor(x0 + key * kw)
    local nx = math.floor(x0 + (key + 1) * kw)
    screen:rect(x, y0, nx - x, h, BLACK[key % 12] and "zone_black" or "zone_white")
    screen:rect(x, y0, 1, h, key % 12 == 0 and "background" or "zone_line")
  end
  for i, z in ipairs(s.zones) do
    local x = math.floor(x0 + z[1] * kw)
    local xr = math.floor(x0 + (z[2] + 1) * kw)
    local yt = math.floor(y0 + (127 - z[4]) / 128 * h)
    local yb = math.floor(y0 + (128 - z[3]) / 128 * h)
    local focused = i == s.focus_zone
    screen:rect(x, yt, xr - x, yb - yt, { colour[1], colour[2], colour[3], focused and 220 or 50 })
    local edge = theme.colors.text
    screen:frame(x, yt, xr - x, yb - yt, focused and "marker" or { edge[1], edge[2], edge[3], 90 }, 1)
  end
end

-- The recorder's two input meters, and in Detect mode the threshold across them.
-- The takes (the Audio Pool) under the meters, a mini waveform each, the focused one
-- outlined; with the takes on the pads, the bank's 16, else the last 8.
local function take_tiles(screen, theme, v, colour)
  local n = v.takes
  if n == 0 then return end
  local per = takes_on_pads and 16 or 8
  local first = takes_on_pads and take_bank * 16 + 1 or math.max(1, n - per + 1)
  local w = takes_on_pads and 27 or 56
  for i = first, math.min(n, first + per - 1) do
    local x = 7 + (i - first) * (w + 2)
    local y = 132
    screen:rect(x, y, w, 40, { 32, 32, 32 })
    local wave = v.take_waves[i]
    if wave and #wave >= 2 then
      local cols = #wave // 2
      for c = 0, w - 3 do
        local k = math.min(cols - 1, c * cols // (w - 2)) * 2
        local lo, hi = wave[k + 1], wave[k + 2]
        local y0 = math.floor(y + 20 - hi * 18 + 0.5)
        local y1 = math.floor(y + 20 - lo * 18 + 0.5)
        screen:rect(x + 1 + c, y0, 1, math.max(1, y1 - y0), colour)
      end
    end
    if i == v.take then
      screen:rect(x, y, w, 1, "text")
      screen:rect(x, y + 39, w, 1, "text")
      screen:rect(x, y, 1, 40, "text")
      screen:rect(x + w - 1, y, 1, 40, "text")
    end
  end
end

local function meters(screen, v, colour)
  local function at(db) return math.max(0, math.min(1, (db + 60) / 60)) end
  for ch = 1, 2 do
    local y = ch == 1 and 90 or 108
    screen:rect(7, y, 462, 10, "meter_empty")
    local level = v.record.levels[ch] or 0
    local db = level > 0 and 20 * math.log(level, 10) or -100
    local fill = math.floor(462 * at(db))
    if fill > 0 then screen:rect(7, y, fill, 10, colour) end
  end
  if v.record.mode == 0 then
    local x = 7 + math.floor(462 * at(v.record.threshold))
    screen:rect(math.min(x, 469), 84, 2, 40, "marker")
  end
end

-- STEMS' progress over both screens.
local function stems_progress(screen, theme, v)
  local big = { font = theme.styles.default.font, size = 21, color = "text" }
  screen:label(20, 60, 440, 144, "Stem Separation in Progress", big, "center", "center")
  local st = { font = theme.styles.default.font, size = 18, color = "text" }
  screen:label(580, 136, 280, 24, "Exporting Sound...", st, "center", "center")
  local edge = theme.colors.text
  screen:frame(580, 166, 280, 20, { edge[1], edge[2], edge[3], 76 }, 1)
  local w = math.floor(272 * math.max(0, math.min(1, v.stems.progress)))
  if w > 0 then screen:rect(584, 170, w, 12, { edge[1], edge[2], edge[3], 179 }) end
end

-- LAYERS: on the left every layer's window over the morph (the focused one filled in
-- brighter) and where the morph is; on the right the four layers shown, each its
-- number, name, waveform and level (dim: not heard).
local function draw_layers(screen, theme, s)
  local x0, y0, w, h = 8, 68, 464, 128
  screen:rect(0, 60, 480, 144, { 24, 24, 24 })
  local morph = s.morph or {}
  local shifts = morph.round and { -1, 0, 1 } or { 0 }
  -- A line of the window between morph positions `a` and `b`, kept inside the strip.
  local function seg(a, ya, b, yb, width, c)
    if b <= 0 or a >= 1 or b <= a then
      if not (a == b and a >= 0 and a <= 1) then return end
    end
    if a < 0 then ya, a = ya + (yb - ya) * (0 - a) / (b - a), 0 end
    if b > 1 then yb, b = ya + (yb - ya) * (1 - a) / (b - a), 1 end
    screen:line(x0 + a * w, y0 + h - ya * h, x0 + b * w, y0 + h - yb * h, width, c)
  end
  for i, l in ipairs(s.layers) do
    local c = LAYER_COLOURS[i] or { 200, 200, 200 }
    local on = i == s.layer
    local col = { c[1], c[2], c[3], (morph.on and 255 or 110) * (on and 1 or 0.55) // 1 }
    local win = l.window
    for _, o in ipairs(shifts) do
      seg(win[1] + o, 0, win[2] + o, 1, on and 2 or 1, col)
      seg(win[2] + o, 1, win[3] + o, 1, on and 2 or 1, col)
      seg(win[3] + o, 1, win[4] + o, 0, on and 2 or 1, col)
    end
  end
  local edge = theme.colors.text
  local px = math.floor(x0 + math.max(0, math.min(1, morph.position or 0)) * w)
  screen:rect(px, y0 - 4, 2, h + 8, { edge[1], edge[2], edge[3], morph.on and 255 or 90 })
  if not morph.on then
    screen:label(x0, y0, w, 14, "MORPH OFF", { size = 11, color = "weak" }, "left", "top")
  end

  local bank = layer_bank(s)
  for k = 1, 4 do
    local i = bank + k
    local l = s.layers[i]
    local y = 60 + (k - 1) * 36
    if l then
      local c = LAYER_COLOURS[i] or { 200, 200, 200 }
      local on = i == s.layer
      screen:rect(480, y, 480, 35, on and { 48, 48, 48 } or { 28, 28, 28 })
      screen:rect(480, y, 3, 35, c)
      local st = { font = theme.styles.default.font, size = theme.styles.default.size, color = on and "text" or "weak" }
      screen:label(488, y, 18, 35, tostring(i), st, "left", "center")
      screen:label(506, y, 150, 35, l.name, st, "left", "center")
      local tag = l.solo and "S" or (l.mute and "M" or "")
      if tag ~= "" then screen:label(660, y, 14, 35, tag, { size = 11, color = "weak" }, "left", "center") end
      local peaks = deckard.peaks("layer" .. i)
      local wc = l.heard and c or widgets.blend(c, { 0, 0, 0 }, 0.3)
      local wx, ww = 680, 190
      local cols = #peaks // 2
      for col = 0, ww - 1 do
        local j = math.min(math.max(cols - 1, 0), col * cols // ww) * 2
        local lo, hi = peaks[j + 1], peaks[j + 2]
        if lo then
          local top = math.floor(y + 17 - hi * 15)
          local bottom = math.ceil(y + 17 - lo * 15)
          screen:rect(wx + col, top, 1, math.max(1, bottom - top), wc)
        end
      end
      screen:label(876, y, 76, 35, l.level, st, "right", "center")
    else
      screen:rect(480, y, 480, 35, { 20, 20, 20 })
    end
  end
end

function page.draw(screen, theme)
  local s = deckard.state()
  local v = s.sampling
  draw_tabs(screen, theme, v.tab, tab_count(v), #layers_of(v) > 0)
  local specs = {}
  for i, b in pairs(buttons(v)) do specs[i + 4] = b end
  if specs[5] and specs[5].arrows then specs[5].left, specs[5].right = not specs[5].disabled, not specs[5].disabled end
  widgets.buttons(screen, theme, specs)
  widgets.info_bar(screen, theme, widgets.focus_info(s))
  sample_bar(screen, theme, v)

  local smp = v.sample
  local colour = widgets.colour(theme, smp and smp.palette or (s.groups[s.group] and s.groups[s.group].sounds[s.sound] or {}).palette)
  if v.tab == 2 and v.stems and v.stems.focused and current(v)[2] == "stems" then
    stems_progress(screen, theme, v)
  elseif v.tab == 1 then
    meters(screen, v, colour)
    take_tiles(screen, theme, v, colour)
  elseif v.tab == 5 and sample_ok(v) then
    draw_layers(screen, theme, smp)
  elseif sample_ok(v) then
    local slices = v.tab == 3
    if v.tab == 4 then
      zone_map(screen, theme, smp, colour)
    else
      wave(screen, theme, smp, deckard.peaks("overview"), 0, 60, 480, 144, 0, smp.frames, colour, slices)
    end
    local lanes = smp.lanes or {}
    if #lanes > 0 and v.tab == 2 then
      -- Its stem lanes stacked, each in its colour (dim: not heard), its name on it.
      local h = math.floor(212 / #lanes)
      for i, l in ipairs(lanes) do
        local c = LANE_COLOURS[l.lane] or colour
        if not l.heard then c = widgets.blend(c, { 0, 0, 0 }, 0.3) end
        local y = 60 + (i - 1) * h
        wave(screen, theme, smp, deckard.peaks("lane" .. i), 480, y, 480, h - 1, smp.view[1], smp.view[2], c, false)
        screen:label(486, y + 2, 120, 14, l.name .. (l.solo and "  S" or (l.mute and "  M" or "")), { size = 11, color = "weak" }, "left", "top")
      end
    else
      wave(screen, theme, smp, deckard.peaks("detail"), 480, 60, 480, 212, smp.view[1], smp.view[2], colour, slices)
    end
    if touched[5] or touched[6] or touched[7] or touched[8] then
      widgets.knob_icon(screen, theme, 5, "zoom")
      widgets.knob_icon(screen, theme, 6, "scroll")
    end
  elseif v.no_sample then
    -- Nothing to edit, slice or map: say why (an instrument's sound, an empty Sampler).
    screen:label(480, 100, 480, 50, v.no_sample.title, { font = "roboto-bold", size = 36, color = "text" }, "center", "center")
    screen:label(480, 156, 480, 30, v.no_sample.detail, { size = 18, color = "weak" }, "center", "center")
  end

  local p, n, count = knob_page(v)
  if p then
    local params = {}
    for k = 1, 4 do
      local knob = p.knobs[k]
      if knob then
        local value, unit = split(knob.text)
        params[k] = { name = knob.name, value = value, unit = unit }
      end
    end
    widgets.params(screen, theme, 0, { { text = p.name, at = 1 } }, count > 1 and (n .. "/" .. count) or nil, params)
  end
  if v.tab == 5 and smp and #smp.layers > 0 then
    -- Knobs 5 and 6 of the page, then the focused layer and its level.
    local right = {}
    for k = 5, 6 do
      local knob = p and p.knobs[k]
      if knob then
        local value, unit = split(knob.text)
        right[k - 4] = { name = knob.name, value = value, unit = unit }
      end
    end
    local l = smp.layers[smp.layer]
    right[3] = { name = "LAYER", value = smp.layer .. "/" .. #smp.layers }
    if l then
      local value, unit = split(l.level)
      right[4] = { name = "LEVEL", value = value, unit = unit }
    end
    widgets.params(screen, theme, 480, {}, nil, right)
  end
end

return page
