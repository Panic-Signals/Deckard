-- Maschine's pattern editor on the screens: the focused pattern with a timeline over
-- it, a row per sound of the group (the overview), or the focused sound's notes with
-- their velocities under them (Step mode's detail). A note's velocity is its
-- opacity; the playhead is a white line.

local widgets = require("lib.widgets")
local pads = require("lib.pads")

local editor = {}

local PPQ = 960
local HEADER = 16
local ROW = 8
local LANE = 24
-- The dot on a velocity stem.
local DOT = 7
-- The note's opacity from the softest to the loudest (0-255).
local ALPHA_LOW, ALPHA_HIGH = 68, 216
-- A step: a sixteenth.
local STEP = PPQ // 4

-- The timeline, the lines under the notes (steps, beats, bars), what's past the end,
-- the focused sound's row.
local TIMELINE = { 38, 38, 38 }
local TIMELINE_TEXT = { 170, 170, 170 }
local STEP_LINE = { 255, 255, 255, 12 }
local BEAT_LINE = { 255, 255, 255, 26 }
local BAR_LINE = { 255, 255, 255, 56 }
local PAST_END = { 0, 0, 0, 150 }
local FOCUS_ROW = { 255, 255, 255, 14 }
-- (The lanes' focused row, lighter.)
local FOCUS_LANE = { 255, 255, 255, 30 }
-- Keyboard mode: a white key's row, lighter; the overview's rows.
local WHITE_ROW = { 255, 255, 255, 10 }
local ROLL_ROW = 5
local BLACK = { [1] = true, [3] = true, [6] = true, [8] = true, [10] = true }

-- The bar's and the beat's length in ticks.
local function bar_beat(s)
  local beat = PPQ * 4 // math.max(1, s.denominator)
  return beat * math.max(1, s.numerator), beat
end

-- A note's colour at its velocity's opacity.
local function note_colour(theme, palette, velocity)
  local c = widgets.colour(theme, palette, false, true)
  local v = math.max(1, math.min(127, velocity)) / 127
  return { c[1], c[2], c[3], math.floor(ALPHA_LOW + (ALPHA_HIGH - ALPHA_LOW) * v) }
end

-- The timeline over the box (`x`, `y`, `w`, `h`) for ticks `t0`..`t1`, and the lines
-- under the notes: steps (when there's room), beats, bars; what's past the pattern's
-- end darkened. Returns a tick's x.
local function timeline(screen, theme, s, x, y, w, h, t0, t1)
  local finish = s.pattern_start + math.max(1, s.pattern_length)
  local span = math.max(1, t1 - t0)
  local function at(t)
    return x + math.floor((t - t0) / span * w + 0.5)
  end
  local bar, beat = bar_beat(s)
  local text = { font = theme.styles.default.font, size = 12, color = TIMELINE_TEXT }

  -- Beats marked, bars numbered (beats too when there's room).
  screen:rect(x, y, w, HEADER, TIMELINE)
  local beats_numbered = (t1 - t0) <= 2 * bar
  local t = (t0 // beat) * beat
  while t < t1 do
    if t >= t0 then
      local on_bar = t % bar == 0
      local top = on_bar and 3 or HEADER - 4
      screen:rect(at(t), y + top, 1, HEADER - top, TIMELINE_TEXT)
      if on_bar or beats_numbered then
        local label
        if on_bar and not beats_numbered then
          label = tostring(t // bar + 1)
        else
          label = (t // bar + 1) .. "." .. (t % bar // beat + 1)
        end
        screen:label(at(t) + 3, y, 37, HEADER, label, text, "left")
      end
    end
    t = t + beat
  end

  local steps_drawn = (t1 - t0) // STEP <= 64
  t = (t0 // STEP) * STEP
  while t < t1 do
    if t > t0 then
      local c = STEP_LINE
      if t % bar == 0 then c = BAR_LINE elseif t % beat == 0 then c = BEAT_LINE end
      if steps_drawn or c ~= STEP_LINE then screen:rect(at(t), y + HEADER, 1, h - HEADER, c) end
    end
    t = t + STEP
  end
  if t1 > finish then screen:rect(at(finish), y + HEADER, x + w - at(finish), h - HEADER, PAST_END) end
  return at
end

-- A note drawn in its row (`top`, `height`) from `x0` to `x1`: selected, outlined in
-- white (a sliver of a note all white).
local function note_box(screen, theme, x0, x1, top, height, palette, n)
  local w = math.max(1, x1 - x0)
  if n.selected and w <= 2 then
    screen:rect(x0, top, w, height, "text")
  elseif n.selected then
    screen:rect(x0, top, w, height, "text")
    screen:rect(x0 + 1, top + 1, w - 2, height - 2, note_colour(theme, palette, n.velocity))
  else
    screen:rect(x0, top, w, height, note_colour(theme, palette, n.velocity))
  end
end

-- Keyboard mode's highest pitch in `rows` rows: the focused sound's notes in the
-- middle (C3 without any).
function editor.pitch_top(s, rows)
  local low, high
  for _, n in ipairs(s.notes) do
    if n.sound == s.sound then low, high = math.min(low or n.pitch, n.pitch), math.max(high or n.pitch, n.pitch) end
  end
  local centre = low and (low + high) // 2 or 60
  return math.max(rows - 1, math.min(127, centre + rows // 2))
end

-- The white keys' rows lighter, `high` the pitch of the top one.
local function shade_rows(screen, x, y, w, row, rows, high)
  for r = 1, rows do
    if not BLACK[(high - r + 1) % 12] then screen:rect(x, y + (r - 1) * row, w, row - 1, WHITE_ROW) end
  end
end

-- Where Step mode's detail puts a stem's top for value `v` (0..1) in the box at `y`,
-- `h` high.
function editor.value_y(y, h, v)
  local top = HEADER + LANE + 8 + DOT // 2
  local bottom = DOT // 2
  return y + math.floor((1 - v) * h + 0.5) + math.floor(v * top - (1 - v) * bottom + 0.5)
end

-- Draw the editor in the box (`x`, `y`, `w`, `h`): `detail` for Step mode's (the
-- ticks `from`..`to` of the pattern; the stems the notes' probabilities, with
-- `probability`), else the overview of all of it.
function editor.draw(screen, theme, s, x, y, w, h, detail, from, to, probability)
  local start = s.pattern_start
  local finish = start + math.max(1, s.pattern_length)
  local t0, t1 = start, finish
  if detail then t0, t1 = from or start, to or finish end
  local at = timeline(screen, theme, s, x, y, w, h, t0, t1)

  local group = s.groups[s.group]
  local sounds = group and group.sounds or {}
  local function palette(n) return sounds[n] and sounds[n].palette or 16 end
  local function visible(n) return n.position < t1 and n.position + n.length > t0 end
  -- A note's left and right pixels, cut off at the editor's edges.
  local function span_of(n)
    return math.max(x, at(n.position)), math.min(x + w, at(n.position + n.length)) - 1
  end

  if not detail and pads.keyboard() then
    -- Keyboard mode: the focused sound's notes by pitch.
    local rows = (h - HEADER) // ROLL_ROW
    local high = editor.pitch_top(s, rows)
    shade_rows(screen, x, y + HEADER, w, ROLL_ROW, rows, high)
    for _, n in ipairs(s.notes) do
      local r = high - n.pitch + 1
      if n.sound == s.sound and visible(n) and r >= 1 and r <= rows then
        local x0, x1 = span_of(n)
        note_box(screen, theme, x0, x1, y + HEADER + (r - 1) * ROLL_ROW, ROLL_ROW - 1, palette(n.sound), n)
      end
    end
  elseif not detail then
    -- Every sound of the group, a row each.
    screen:rect(x, y + HEADER + (s.sound - 1) * ROW, w, ROW, FOCUS_ROW)
    for _, n in ipairs(s.notes) do
      if visible(n) and n.sound <= 16 then
        local x0, x1 = span_of(n)
        note_box(screen, theme, x0, x1, y + HEADER + (n.sound - 1) * ROW + 1, ROW - 1, palette(n.sound), n)
      end
    end
  else
    -- The focused sound's notes in a row, and under each its velocity (or
    -- probability): a stem up from the bottom with a dot on top.
    for _, n in ipairs(s.notes) do
      if n.sound == s.sound and visible(n) then
        local x0, x1 = span_of(n)
        note_box(screen, theme, x0, x1, y + HEADER + 1, LANE, palette(n.sound), n)
      end
      -- (Its velocity where it starts, if that's shown.)
      if n.sound == s.sound and visible(n) and n.position >= t0 then
        local x0 = at(n.position)
        -- (A selected note's white.)
        local c = n.selected and "text" or widgets.colour(theme, palette(n.sound), false, true)
        local v
        if probability then
          v = math.max(0, math.min(100, n.probability)) / 100
        else
          v = math.max(0, math.min(127, n.velocity)) / 127
        end
        local sy = editor.value_y(y, h, v)
        local sx = x0 + DOT // 2
        screen:rect(sx - 1, sy, 2, y + h - sy, c)
        screen:circle(sx - DOT // 2 + DOT / 2, sy - DOT // 2 + DOT / 2, DOT / 2, c)
      end
    end
  end

  -- The playhead.
  local p = s.pattern_position
  if p and p >= t0 and p < t1 then screen:rect(at(p), y, 1, h, "text") end
end

-- The editor as lanes (the Events page's): ticks `o.from`..`o.to` in the box, `o.rows`
-- rows of `o.row` pixels; `o.row_of(note)` says which row a note is in (none: not
-- shown), `o.focus` which row is lit; pitch lanes, `o.high` the top one's pitch.
function editor.lanes(screen, theme, s, x, y, w, h, o)
  local t0 = o.from or s.pattern_start
  local t1 = o.to or (s.pattern_start + math.max(1, s.pattern_length))
  local at = timeline(screen, theme, s, x, y, w, h, t0, t1)
  local group = s.groups[s.group]
  local sounds = group and group.sounds or {}
  if o.focus then screen:rect(x, y + HEADER + (o.focus - 1) * o.row, w, o.row, FOCUS_LANE) end
  if o.high then shade_rows(screen, x, y + HEADER, w, o.row, o.rows, o.high) end
  for _, n in ipairs(s.notes) do
    local r = o.row_of(n)
    if r and r >= 1 and r <= o.rows and n.position < t1 and n.position + n.length > t0 then
      local x0 = math.max(x, at(n.position))
      local x1 = math.min(x + w, at(n.position + n.length)) - 1
      local palette = sounds[n.sound] and sounds[n.sound].palette or 16
      note_box(screen, theme, x0, x1, y + HEADER + (r - 1) * o.row + 1, o.row - 1, palette, n)
    end
  end
  local p = s.pattern_position
  if p and p >= t0 and p < t1 then screen:rect(at(p), y, 1, h, "text") end
end

-- Keyboard mode's keys beside pitch lanes: `rows` keys of `row` pixels in the box at
-- (`x`, `y`, `w` wide), `high` the pitch at the top; each C named.
function editor.keys(screen, theme, x, y, w, row, rows, high, name)
  local font = theme.styles.default.font
  for r = 1, rows do
    local p = high - (r - 1)
    local top = y + (r - 1) * row
    local black = ({ [1] = true, [3] = true, [6] = true, [8] = true, [10] = true })[p % 12]
    screen:rect(x, top, w, row - 1, black and { 20, 20, 20 } or { 200, 200, 200 })
    if p % 12 == 0 and name then
      screen:label(x + 1, top, w - 1, row, name(p), { font = font, size = 9, color = { 0, 0, 0 } }, "left", "center")
    end
  end
end

editor.HEADER = HEADER
editor.LANE = LANE

return editor
