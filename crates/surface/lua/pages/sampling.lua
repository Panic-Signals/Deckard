-- The Sampling page (SAMPLE): the focused sound's sample and the recorder, a tab at a
-- time, as Maschine's (docs/sampling-controller.md).
--
--   display 1-4       the tab: RECORD, EDIT, SLICE, ZONE
--   RECORD 5-8        START (WAITING, STOP), DELETE (CANCEL), PREVIOUS, NEXT take
--   EDIT 5-6 / 8      ◀ the edit function ▶, APPLY (STEMS: CANCEL while separating)
--   SLICE 5-8         SPLIT, REMOVE the focused slice, DELETE ALL, APPLY… (the slices
--                     made zones of the focused sound)
--   ZONE 5-8          REMOVE the focused zone, ADD, PREVIOUS, NEXT zone
--   LEFT / RIGHT      the tab's knob pages
--   knobs 1-4         the page's settings (SHIFT: finer)
--   knobs 5 / 6       zoom / scroll the sample (their icons shown while touched)
--   pads              SLICE: pick a slice and hear it; else the sounds, as ever

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

-- The knob page shown on each tab, the slice focused (from 1; none until a pad picks
-- one: the buttons then act on the first), the knobs touched.
local at = { 1, 1, 1, 1 }
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
    { name = "Apply", knobs = { nil, k["Slice.Apply"], k["Slice.BpmMode"], k["Slice.Bpm"] } },
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

-- RECORD: the takes on the pads (TAKES), and the bank of 16 shown.
local takes_on_pads, take_bank = false, 0

-- What display buttons 5-8 show and do on the tab: { text, act = function, … }.
local function buttons(v)
  local s = v.sample
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
    local f = FUNCTIONS[func]
    local cancel = f[2] == "stems" and v.stems and v.stems.focused
    return {
      { arrows = true, text = f[1], disabled = not s },
      nil,
      nil,
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
      if n <= tab_count(v) then deckard.sampling_tab(n) end
      return true
    end
    if v.tab == 2 and (n == 5 or n == 6) then
      if v.sample then func = (func + (n == 6 and 1 or -1) - 1) % #FUNCTIONS + 1 end
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
-- turns make its `turn`.
local function turn(k, amount, send)
  local fine = deckard.held("SHIFT") and 0.1 or 1
  local g = (turns[k.key] or 0) + amount * fine
  local steps = g >= 0 and math.floor(g / k.turn + 1e-9) or -math.floor(-g / k.turn + 1e-9)
  turns[k.key] = g - steps * k.turn
  if steps == 0 then return end
  local a = asked[k.key]
  local from = (a and a.shown == k.value) and a.value or k.value
  local to = math.max(k.min, math.min(k.max, from + steps * k.step))
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

function page.knob(n, amount)
  local v = view()
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
    local t = { key = "Slice.Move" .. k.move, value = k.frame, min = 0, max = s.frames - 1, step = math.max(1, math.floor((s.view[2] - s.view[1]) / 100)), turn = 0.01 }
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
  local i = ((slice or 1) - 1) // 16 * 16 + n
  if s.slices[i] then
    slice = i
    deckard.sampling_call("prehear", s.slices[i], s.slices[i + 1] or s.play[2])
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
  for i = 1, 4 do deckard.led("SCREEN_" .. i, i == v.tab and "on" or (i <= tab_count(v) and "dim" or "off")) end
  local b = buttons(v)
  -- (WAITING flashes, four times a second.)
  local flash = v.record.waiting and (deckard.now() * 4) % 2 < 1
  for i = 5, 8 do
    local spec = b[i - 4] or (v.tab == 2 and i == 6 and b[1])
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
    local bank = ((slice or 1) - 1) // 16 * 16
    for i = 1, 16 do
      local k = bank + i
      if s and s.slices[k] then
        deckard.led("PAD_" .. i, palette, k == slice)
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

local function draw_tabs(screen, theme, tab, count)
  screen:frame(4, 4, 472, 24, "button", theme.corner)
  local xs, ws = { 4, 121, 240, 359 }, { 117, 119, 119, 117 }
  for i, name in ipairs(TABS) do
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
  for c = 0, w - 1 do
    local lo, hi = peaks[2 * c + 1], peaks[2 * c + 2]
    if lo then
      local f = a + (c + 0.5) * per
      local top = math.floor(mid - hi * h / 2)
      local bottom = math.ceil(mid - lo * h / 2)
      screen:rect(x + c, top, 1, math.max(1, bottom - top), (f >= first and f < last) and colour or off)
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

function page.draw(screen, theme)
  local s = deckard.state()
  local v = s.sampling
  draw_tabs(screen, theme, v.tab, tab_count(v))
  local specs = {}
  for i, b in pairs(buttons(v)) do specs[i + 4] = b end
  if specs[5] and specs[5].arrows then specs[5].left, specs[5].right = not specs[5].disabled, not specs[5].disabled end
  widgets.buttons(screen, theme, specs)
  widgets.info_bar(screen, theme, widgets.focus_info(s))
  sample_bar(screen, theme, v)

  local smp = v.sample
  local colour = widgets.colour(theme, smp and smp.palette or (s.groups[s.group] and s.groups[s.group].sounds[s.sound] or {}).palette)
  if v.tab == 2 and v.stems and v.stems.focused and FUNCTIONS[func][2] == "stems" then
    stems_progress(screen, theme, v)
  elseif v.tab == 1 then
    meters(screen, v, colour)
    take_tiles(screen, theme, v, colour)
  elseif sample_ok(v) then
    local slices = v.tab == 3
    if v.tab == 4 then
      zone_map(screen, theme, smp, colour)
    else
      wave(screen, theme, smp, deckard.peaks("overview"), 0, 60, 480, 144, 0, smp.frames, colour, slices)
    end
    wave(screen, theme, smp, deckard.peaks("detail"), 480, 60, 480, 212, smp.view[1], smp.view[2], colour, slices)
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
end

return page
