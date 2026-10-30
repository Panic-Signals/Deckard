-- The Events page (EVENTS held, or pinned): the focused pattern's notes on both
-- screens, and the selected ones edited.
--
--   display 1         EVENTS (its pin)
--   display 2         SELECT: the pads and knobs select notes instead
--   display 4         GRID: the Step Grid on or off
--   display 5 / 6     QUANTIZE (SHIFT: QUANT 50%) / CLEAR ALL; selecting, ALL / NONE
--   display 8         PROB LOCK (the focused sound's chances rolled once and kept)
--   display 7 / 8     selecting, UP / DOWN (the sound before or after)
--   knobs 1-4         POSITION, PITCH, VELOCITY, LENGTH of the selected notes (page 2:
--                     PROBABILITY); selecting, START and END of the time selected, LOW
--                     and HIGH of the pitches (Keyboard mode) or the EVENT alone
--   knobs 5-8         zoom and scroll the editor (8: the pitches, in Keyboard mode)
--   4-D encoder       turned, the note before or after (pressed: its length); left /
--                     right nudge (SHIFT finer), up / down the sound before or after
--                     (Keyboard mode: transpose, SHIFT by octaves)
--   pads              selecting, a sound's notes in or out of the selection (Keyboard
--                     mode: a pitch's); with ERASE, delete them

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local pads = require("lib.pads")
local pin = require("lib.pin")
local util = require("lib.util")
local editor = require("lib.pattern_editor")
local notes = require("lib.notes")

local events = {}

-- Selecting (SELECT), and the knob page (1, 2: PROBABILITY).
local selecting = false
local knob_page = 1
-- The selection's time and pitch range, as the START, END, LOW and HIGH knobs move it.
local range = nil
-- The range is taken from the selection again when next it's shown (a sound's notes
-- all selected, every note, the next note: as Maschine's).
local resync = false
-- The right screen's view: the ticks shown (nil: the whole pattern), the lanes zoomed
-- (8 rows, not 16), Keyboard mode's highest pitch shown.
local view_from, view_to = nil, nil
local zoomed = false
-- Knobs 5-8 touched (zoom and scroll).
local touched = {}
local top_pitch = nil

local MIN_SNAP = notes.MIN_SNAP
local ROWS = 16
local span, shown, selected, value = notes.span, notes.shown, notes.selected, notes.value
local has_notes = notes.any

local function keyboard()
  return pads.keyboard()
end

-- The selection's range: from the first selected note of the focused sound to the
-- last (or the whole pattern), its pitches from the lowest to the highest; with
-- `full` false it only widens to take in what's selected.
local function sync(s, full)
  local a, b = span(s)
  local first, last
  local low, high = 127, 0
  for _, n in ipairs(shown(s, true)) do
    if n.selected then
      first = math.min(first or n.position, n.position)
      last = math.max(last or n.position, n.position)
    end
    low, high = math.min(low, n.pitch), math.max(high, n.pitch)
  end
  if low > high then low, high = 0, 127 end
  if full or resync or not range then
    resync = false
    range = { start = first or a, finish = last or b, low = low, high = high }
  else
    if first and first < range.start then range.start = first end
    if last and last > range.finish then range.finish = last end
  end
  range.start = util.clamp(range.start, a, b)
  range.finish = util.clamp(range.finish, a, b)
end

local function select_range()
  if keyboard() then
    deckard.select_events(range.start, range.finish, range.low, range.high)
  else
    deckard.select_events(range.start, range.finish)
  end
end

-- The EVENT knob's value: the selected note's place among the focused sound's.
local function event_text(s)
  local own = shown(s, true)
  local count, at = 0, nil
  for i, n in ipairs(own) do
    if n.selected then count, at = count + 1, i end
  end
  if count == 1 then return tostring(at) end
  if count > 1 then return "*" end
  return "-"
end

-- The knobs' names and values.
local function params(s)
  if knob_page == 2 then return { notes.probability_param(s) } end
  if selecting then
    local function pos(t) return util.bar_beat(t, s.numerator, s.denominator) end
    sync(s, false)
    local p = {
      { name = "START", value = s.pattern_length > 0 and pos(range.start) or "-" },
      { name = "END", value = s.pattern_length > 0 and pos(range.finish) or "-" },
    }
    if keyboard() then
      p[3] = { name = "LOW", value = pads.note_name(range.low) }
      p[4] = { name = "HIGH", value = pads.note_name(range.high) }
    else
      p[3] = { name = "", value = "" }
      p[4] = { name = "EVENT", value = event_text(s) }
    end
    return p
  end
  return notes.edit_params(s)
end

-- Shown: the first knob page, the range from the selection. (Selecting stays, as
-- Maschine's.)
-- Whether the pads select notes (SELECT on): SHIFT + a pad selects too then.
function events.selecting()
  return selecting
end

function events.shown()
  knob_page = 1
  range = nil
  touched = {}
end

function events.button(name, down)
  local cap = tonumber(name:match("^CAP_(%d)$"))
  if cap and cap >= 5 then touched[cap] = down or nil end
  local tapped = knobs.double_tap(name, down)
  if tapped then notes.reset_knob(tapped, knob_page) end
  local s = deckard.state()
  local shift = deckard.held("SHIFT")
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if n then
    -- (Display button 1 is the pin, global.lua's.)
    if n == 1 then return end
    if not down then return true end
    if n == 2 then
      selecting = not selecting
      sync(s, true)
    elseif n == 4 then
      deckard.grid("step_snap", not s.grid.step_snap)
    elseif selecting then
      if n == 5 then
        deckard.events("select_all")
        resync = true
      elseif n == 6 then
        deckard.events("select_none")
      elseif n == 7 or n == 8 then
        local group = s.groups[s.group]
        local to = s.sound + (n == 7 and -1 or 1)
        if group and to >= 1 and to <= #group.sounds then deckard.focus_sound(to) end
      end
    elseif n == 5 and has_notes(s) then
      deckard.events(shift and "quantize_half" or "quantize")
    elseif n == 6 and has_notes(s) then
      deckard.events("clear_all", keyboard() and 1 or 0)
    elseif n == 8 then
      notes.toggle_prob_lock(s)
    end
    return true
  end
  if name == "LEFT" or name == "RIGHT" then
    if down then knob_page = name == "LEFT" and 1 or 2 end
    return true
  end
  return notes.wheel_button(name, down)
end

events.wheel = notes.wheel

-- The pitch a row of Keyboard mode's lanes shows (the highest at the top).
-- (Keyboard mode's pitch lanes.)
local KEY_ROWS, KEY_ROW = 24, 8

local function pitch_top(s, rows)
  return top_pitch or editor.pitch_top(s, rows or KEY_ROWS)
end

local function touching(from, to)
  for n = from, to do
    if touched[n] then return true end
  end
  return false
end

-- The boxes over knobs 5-8: zoom, scroll, the lanes' size (Keyboard mode: none) and
-- Keyboard mode's pitches.
local function zoom_icons(screen, theme)
  local what = { "zoom", "scroll", not keyboard() and (zoomed and "2×" or "1×") or nil, keyboard() and "pitch" or nil }
  for k = 1, 4 do
    local icon = what[k]
    if icon then
      widgets.knob_icon(screen, theme, k + 4, icon)
    end
  end
end

-- How far knob 5 zooms a tenth of a turn.
local ZOOM = 1.6

local function zoom_scroll(s, n, amount)
  local a, b = span(s)
  local from, to = view_from or a, view_to or b
  local width = to - from
  -- (Zoom and scroll as smooth as Maschine's: x1.6 a tenth of a turn, from the left
  -- edge, a beat at the least and the pattern at most; a view's width a turn.)
  if n == 5 then
    local new = util.clamp(math.floor(width / ZOOM ^ (amount * 10) + 0.5), 960, b - a)
    from = util.clamp(from, a, b - new)
    view_from, view_to = from, from + new
  elseif n == 6 then
    from = util.clamp(math.floor(from + amount * width + 0.5), a, b - width)
    view_from, view_to = from, from + width
  else
    local steps = knobs.gather(n, amount, 0.05)
    if steps == 0 then return end
    if n == 7 and not keyboard() then
      zoomed = steps > 0
    elseif n == 8 and keyboard() then
      top_pitch = util.clamp(pitch_top(s) + steps, KEY_ROWS - 1, 127)
    end
  end
  if view_from == a and view_to == b then view_from, view_to = nil, nil end
end

function events.knob(n, amount)
  local s = deckard.state()
  if n >= 5 then
    zoom_scroll(s, n, amount)
    return true
  end
  if knob_page == 2 then
    local steps = knobs.gather(n, amount, 0.05)
    if n == 1 and steps ~= 0 then deckard.events("probability", steps) end
    return true
  end
  if selecting then
    local steps = knobs.gather(n, amount, n <= 2 and 0.05 or 0.1)
    if steps == 0 then return true end
    local a, b = span(s)
    sync(s, false)
    if n == 1 or n == 2 then
      local snap = s.grid.step_snap ~= false and s.grid.step_ticks or MIN_SNAP
      local by = steps * math.max(snap or MIN_SNAP, 1)
      if n == 1 then
        range.start = util.clamp(range.start + by, a, math.min(range.finish, b))
      else
        range.finish = util.clamp(range.finish + by, math.max(range.start, a), b)
      end
      select_range()
    elseif keyboard() then
      -- (LOW and HIGH go to the next pitch the focused sound plays.)
      local at = n == 3 and range.low or range.high
      local best = nil
      for _, note in ipairs(shown(s, true)) do
        local p = note.pitch
        if (steps > 0 and p > at) or (steps < 0 and p < at) then
          if not best or (steps > 0 and p < best) or (steps < 0 and p > best) then best = p end
        end
      end
      if best then
        if n == 3 then range.low = math.min(best, range.high) else range.high = math.max(best, range.low) end
      end
      select_range()
    elseif n == 4 then
      deckard.events("select_step", steps)
      resync = true
    end
    return true
  end
  local steps = knobs.gather(n, amount, 0.05)
  if steps ~= 0 then notes.edit_knob(n, steps) end
  return true
end

function events.pads()
  if selecting then return { play = false } end
end

function events.pad(n)
  if not selecting then return end
  local s = deckard.state()
  if keyboard() then
    local note = pads.notes()[n]
    if note and note <= 127 then
      deckard.events(deckard.held("ERASE") and "erase_pitch" or "select_pitch", note)
    end
  else
    local group = s.groups[s.group]
    if group and n <= #group.sounds then
      if deckard.held("ERASE") then
        deckard.erase_sound(n)
      else
        deckard.events("select_sound", n)
        resync = true
      end
    end
  end
  return true
end

-- A pad's light while selecting: lit where there are notes, bright where they're all
-- selected (or ERASE is held).
local function select_leds(s)
  local erase = deckard.held("ERASE")
  local group = s.groups[s.group]
  local notes = shown(s, keyboard())
  local key = keyboard() and pads.notes() or nil
  for i = 1, 16 do
    local any, all = false, true
    for _, note in ipairs(notes) do
      local mine
      if key then mine = note.pitch == key[i] else mine = note.sound == i end
      if mine then
        any = true
        if not note.selected then all = false end
      end
    end
    local palette
    if key then
      local snd = group and group.sounds[s.sound]
      palette = snd and snd.palette
    else
      local snd = group and group.sounds[i]
      palette = snd and snd.palette
    end
    if any and palette then
      deckard.led("PAD_" .. i, palette, erase or all)
    else
      deckard.led("PAD_" .. i, "off")
    end
  end
end

function events.leds()
  local s = deckard.state()
  local any = has_notes(s)
  deckard.led("SCREEN_1", pin.on() and "on" or "dim")
  deckard.led("SCREEN_2", selecting and "on" or "dim")
  deckard.led("SCREEN_3", "off")
  deckard.led("SCREEN_4", s.grid.step_snap and "on" or "dim")
  deckard.led("LEFT", knob_page == 2 and "dim" or "off")
  deckard.led("RIGHT", knob_page == 1 and "dim" or "off")
  if selecting then
    for i = 5, 8 do deckard.led("SCREEN_" .. i, "dim") end
    select_leds(s)
  else
    deckard.led("SCREEN_5", any and "dim" or "off")
    deckard.led("SCREEN_6", any and "dim" or "off")
    deckard.led("SCREEN_7", "off")
    deckard.led("SCREEN_8", notes.prob_locked(s) and "on" or "dim")
  end
  notes.wheel_leds(s)
end

-- The right screen's list beside the lanes: the sounds' numbers (the focused one
-- filled), or Keyboard mode's keys.
local LIST_W = 20

local function draw_list(screen, theme, s, y, row, first, rows, high)
  local group = s.groups[s.group]
  -- (Smaller numbers in short rows.)
  local st = { font = theme.styles.default.font, size = row >= 10 and 11 or 8 }
  for r = 1, rows do
    local top = y + (r - 1) * row
    if keyboard() then
      editor.keys(screen, theme, 480, top, LIST_W, row, 1, high - (r - 1), pads.note_name)
    else
      local i = first + r - 1
      local snd = group and group.sounds[i]
      if snd then
        local c = widgets.colour(theme, snd.palette, false, true)
        if i == s.sound then
          screen:rect(480, top, LIST_W, row, widgets.colour(theme, snd.palette))
          c = { 0, 0, 0 }
        end
        screen:label(482, top, LIST_W - 2, row, tostring(i), { font = st.font, size = st.size, color = c }, "left", "center")
      end
    end
  end
end

-- The right screen's pattern name and lanes, down to `bottom` (also the Note Repeat
-- page's).
function events.draw_lanes(screen, theme, s, bottom)
  local group = s.groups[s.group]
  local pattern = group and group.patterns and group.patterns[s.pattern]
  local name = pattern and pattern.name or "NO PATTERN"
  local st = { font = theme.styles.default.font, size = 14,
    color = pattern and widgets.colour(theme, pattern.palette, false, true) or "text" }
  screen:label(481, 35, 470, 24, name, st, "left", "center")
  local y = 60
  local top = y + editor.HEADER
  -- (Keyboard mode's lanes as many as fit.)
  local rows = keyboard() and math.min(KEY_ROWS, (bottom - top) // KEY_ROW) or (zoomed and 8 or ROWS)
  local row = keyboard() and KEY_ROW or (bottom - top) // rows
  local first = 1
  if rows == 8 and not keyboard() then first = s.sound > 8 and 9 or 1 end
  local o = { from = view_from, to = view_to, rows = rows, row = row }
  local high = pitch_top(s, rows)
  if keyboard() then
    o.high = high
    o.row_of = function(n) return n.sound == s.sound and high - n.pitch + 1 or nil end
  else
    o.row_of = function(n) return n.sound - first + 1 end
    o.focus = s.sound - first + 1
  end
  editor.lanes(screen, theme, s, 480 + LIST_W, y, 480 - LIST_W, bottom - y, o)
  draw_list(screen, theme, s, top, row, first, rows, high)
end

function events.draw(screen, theme)
  local s = deckard.state()
  local shift = deckard.held("SHIFT")
  local any = has_notes(s)
  local b = {
    [1] = { text = "EVENTS", pin = true, selected = pin.on() },
    [2] = { text = "SELECT", selected = selecting },
    [4] = { text = "GRID", selected = s.grid.step_snap },
  }
  if selecting then
    b[5], b[6], b[7], b[8] = { text = "ALL" }, { text = "NONE" }, { text = "UP" }, { text = "DOWN" }
  else
    b[5] = { text = shift and "QUANT 50%" or "QUANTIZE", disabled = not any }
    b[6] = { text = "CLEAR ALL", disabled = not any }
    b[8] = { text = "PROB LOCK", selected = notes.prob_locked(s) }
  end
  widgets.buttons(screen, theme, b)
  widgets.info_bar(screen, theme, widgets.focus_info(s))

  -- The left screen: the whole pattern, a row a sound.
  editor.draw(screen, theme, s, 0, 60, 480, 145, false)
  widgets.params(screen, theme, 0, { { text = selecting and "Select" or "Edit", at = 1 } }, nil, params(s))

  -- The right screen: the pattern's name, and its lanes.
  events.draw_lanes(screen, theme, s, 272)

  -- Knobs 5-8 touched: what they do; zooming or scrolling, what the right screen shows
  -- framed on the left.
  if touching(5, 8) then zoom_icons(screen, theme) end
  if touching(5, 6) then
    local a, b = span(s)
    local from, to = view_from or a, view_to or b
    local function x_of(t) return math.floor((t - a) / math.max(1, b - a) * 480 + 0.5) end
    local x0, x1 = x_of(from), math.min(x_of(to), 480)
    screen:frame(x0, 60 + editor.HEADER, math.max(1, x1 - x0), 145 - editor.HEADER, "text")
  end
end

return events
