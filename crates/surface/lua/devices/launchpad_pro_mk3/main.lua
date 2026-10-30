-- Launchpad Pro [MK3] in DAW mode: the Session grid is the Ideas view as Maschine
-- shows it. Each column is a group (the focused group's eight), its patterns down the
-- top seven rows, lit where the focused scene plays them; a pad turns its pattern on
-- or off in the scene, and the place after a group's last pattern makes a new one.
-- The grid's bottom row is the playhead: the longest pattern playing spread over the
-- eight pads, or, stopped, its first pad strobing with the tempo. The row below
-- the grid is the scenes: a press launches one, and the place after the last makes a
-- new scene. Left / right scroll the scenes eight at a time, up / down the patterns
-- seven,
-- SHIFT + left / right focus the previous / next eight groups. Mute is a mode: a pad
-- mutes its column's group, a scene's button the groups that play in it. Play and
-- Record start and stop, and Print to Clip writes into the focused sound's
-- pattern. The bytes are Novation's Programmer's Reference's.

local clip = require("lib.print")

local HEADER = { 0xF0, 0x00, 0x20, 0x29, 0x02, 0x0E }
local DAW, LAYOUT, LIGHTS, PRINT_BUTTON = 0x10, 0x00, 0x03, 0x18
local SESSION = 0x00
local UP, DOWN, LEFT, RIGHT, SESSION_BUTTON, SHIFT = 80, 70, 91, 92, 93, 90
local PLAY, RECORD, MUTE = 20, 10, 2
-- The row of buttons under the grid: scene 1 of the eight shown is 101.
local SCENE_ROW = 101
-- Lighting specs a message takes at most.
local SPECS = 100

local M = {}

local st = {
  session = true,
  lit = {},          -- LED → "r,g,b" as last sent
  patterns = 0,      -- scroll: the first pattern and scene shown, from 0
  scenes = 0,
  shift = false,
  mute = false,      -- Mute mode
  print = clip.new(),
}

local function sysex(...)
  local t = { table.unpack(HEADER) }
  for _, b in ipairs({ ... }) do t[#t + 1] = b end
  t[#t + 1] = 0xF7
  return t
end

local function send(bytes) deckard.midi_send("daw", bytes) end

-- An RGB from the view (0–255) scaled to the device's 0–127, times `level`.
local function scaled(rgb, level)
  local function c(x) return math.min(127, math.floor((x or 0) / 2 * level + 0.5)) end
  return { c(rgb[1]), c(rgb[2]), c(rgb[3]) }
end

local OFF = { 0, 0, 0 }
local WHITE = { 255, 255, 255 }
local GREEN = { 0, 255, 0 }
local RED = { 255, 0, 0 }
local YELLOW = { 255, 255, 0 }
local DIM, FULL = 0.15, 1.0
-- A place for a new pattern or scene: a faint white.
local NEW = 0.05
-- A muted group's pattern the focused scene plays.
local MUTED = 0.35

-- Where the beat is (from the tempo; playing or not), for pulsing and flashing.
local function beat(s)
  return deckard.now() * math.max(s.tempo or 120, 1) / 60
end

-- Pulsing: between 0.4 and full over two beats, in four steps each way (fewer sends).
local function pulse(s)
  local phase = (beat(s) / 2) % 1
  local tri = phase < 0.5 and phase * 2 or (1 - phase) * 2
  return 0.4 + 0.6 * math.floor(tri * 4 + 0.5) / 4
end

-- Flashing: on one beat, off the next.
local function flash_on(s)
  return math.floor(beat(s)) % 2 == 0
end

-- The first of the last `per` of `n` things, from 0.
local function last_offset(n, per)
  per = per or 8
  return math.max(n - 1, 0) // per * per
end

-- The pattern rows (the grid's bottom row is the playhead's).
local ROWS = 7

-- The first of the eight groups shown (the focused group's eight), from 0.
local function first_group(s)
  return (s.group - 1) // 8 * 8
end

-- The most patterns a group shown has, and a place for a new one.
local function pattern_places(s)
  local most = 0
  for i = 1, 8 do
    local group = s.groups[first_group(s) + i]
    if group then most = math.max(most, #group.patterns) end
  end
  return most + 1
end

-- The group and pattern (from 1) under pad `row`, `col`.
local function cell(s, row, col)
  return first_group(s) + col, st.patterns + (8 - row) + 1
end

-- The playhead's pad `col` (the grid's bottom row). Playing: the longest pattern the
-- focused scene plays spread over the eight pads, however long it is (an eighth of it
-- each): the part played dim, the pad playing full, those to come faint. Stopped: the
-- first pad strobes with the tempo, lit for the first quarter of each beat.
local function playhead(s, col)
  if not s.playing then
    return (col == 1 and beat(s) % 1 < 0.25) and scaled(WHITE, FULL) or OFF
  end
  local bars, progress = 0, nil
  for _, group in ipairs(s.groups) do
    local p = group.scene_pattern and group.patterns[group.scene_pattern]
    if p and group.progress and p.bars > bars then bars, progress = p.bars, group.progress end
  end
  if not progress then return OFF end
  local at = math.min(7, math.floor(progress * 8))
  if col - 1 < at then return scaled(WHITE, DIM) end
  if col - 1 == at then return scaled(WHITE, FULL) end
  return scaled(WHITE, NEW)
end

-- What every LED should show.
local function wanted(s)
  local leds = {}
  local flashing = flash_on(s)
  for col = 1, 8 do
    leds[10 + col] = playhead(s, col)
  end
  for row = 2, 8 do
    for col = 1, 8 do
      local g, p = cell(s, row, col)
      local group = s.groups[g]
      local color = OFF
      if group and group.patterns[p] then
        local level = DIM
        if group.scene_pattern == p then level = group.muted and MUTED or FULL end
        color = scaled(group.patterns[p].color, level)
      elseif group and p == #group.patterns + 1 then
        color = scaled(WHITE, NEW)
      end
      leds[10 * row + col] = color
    end
  end
  for col = 1, 8 do
    local sc = st.scenes + col
    local scene = s.scenes[sc]
    local color = OFF
    if scene then
      local level = DIM
      if sc == s.queued_scene then
        level = flashing and FULL or 0
      elseif sc == s.playing_scene then
        level = pulse(s)
      elseif sc == s.scene then
        level = FULL
      end
      color = scaled(scene.color, level)
    elseif sc == #s.scenes + 1 then
      color = scaled(WHITE, NEW)
    end
    leds[SCENE_ROW + col - 1] = color
  end
  local can = scaled(WHITE, DIM)
  leds[UP] = st.patterns > 0 and can or OFF
  leds[DOWN] = st.patterns < last_offset(pattern_places(s), ROWS) and can or OFF
  leds[LEFT] = st.scenes > 0 and can or OFF
  leds[RIGHT] = st.scenes < last_offset(#s.scenes + 1) and can or OFF
  leds[SESSION_BUTTON] = scaled(WHITE, FULL)
  leds[PLAY] = s.playing and scaled(GREEN, FULL) or scaled(WHITE, DIM)
  leds[RECORD] = scaled(RED, s.recording and FULL or DIM)
  leds[MUTE] = scaled(YELLOW, st.mute and FULL or DIM)
  return leds
end

-- Send the LEDs that changed, as RGB specs.
local function draw()
  if not st.session then return end
  local s = deckard.state()
  -- (Keep the scroll in range when groups or scenes go.)
  st.patterns = math.min(st.patterns, last_offset(pattern_places(s), ROWS))
  st.scenes = math.min(st.scenes, last_offset(#s.scenes + 1))
  local specs = {}
  local leds = wanted(s)
  local order = {}
  for led in pairs(leds) do order[#order + 1] = led end
  table.sort(order)
  for _, led in ipairs(order) do
    local c = leds[led]
    local key = c[1] .. "," .. c[2] .. "," .. c[3]
    if st.lit[led] ~= key then
      st.lit[led] = key
      specs[#specs + 1] = { 3, led, c[1], c[2], c[3] }
    end
  end
  for first = 1, #specs, SPECS do
    local msg = { table.unpack(HEADER) }
    msg[#msg + 1] = LIGHTS
    for i = first, math.min(first + SPECS - 1, #specs) do
      for _, b in ipairs(specs[i]) do msg[#msg + 1] = b end
    end
    msg[#msg + 1] = 0xF7
    send(msg)
  end
end

function M.open()
  st.lit, st.session, st.print, st.mute, st.shift = {}, true, clip.new(), false, false
  send(sysex(DAW, 1))
  send(sysex(LAYOUT, SESSION, 0, 0))
  send(sysex(PRINT_BUTTON, 1))
  draw()
end

function M.close()
  send(sysex(DAW, 0))
end

function M.tick()
  draw()
end

local function is_sysex(b, command)
  if b[1] ~= 0xF0 or #b < 8 then return false end
  for i = 1, #HEADER do
    if b[i] ~= HEADER[i] then return false end
  end
  return b[#HEADER + 1] == command
end

-- The bytes after the command, without F7.
local function body(b)
  local d = {}
  for i = #HEADER + 2, #b - 1 do d[#d + 1] = b[i] end
  return d
end

local function scroll(dp, ds)
  local s = deckard.state()
  st.patterns = math.max(0, math.min(st.patterns + dp, last_offset(pattern_places(s), ROWS)))
  st.scenes = math.max(0, math.min(st.scenes + ds, last_offset(#s.scenes + 1)))
  draw()
end

-- Focus the first of the eight groups `dir` eights on, if there are any.
local function group_bank(dir)
  local s = deckard.state()
  local g = first_group(s) + dir * 8 + 1
  if g >= 1 and s.groups[g] then deckard.focus_group(g) end
end

-- Mute or unmute the groups that play in scene `sc`: all of them muted, unmuted;
-- else those not muted, muted.
local function mute_scene(s, sc)
  local groups, all = {}, true
  for g, p in pairs(s.scenes[sc].patterns) do
    if p and s.groups[g] then
      groups[#groups + 1] = g
      all = all and s.groups[g].muted
    end
  end
  table.sort(groups)
  for _, g in ipairs(groups) do
    if all or not s.groups[g].muted then deckard.mute_group(g) end
  end
end

-- Pad `row`, `col`: its pattern on or off in the focused scene, or a new one; in Mute
-- mode, its group muted or not.
local function press(row, col)
  local s = deckard.state()
  local g, p = cell(s, row, col)
  local group = s.groups[g]
  if not group then return end
  if st.mute then
    deckard.mute_group(g)
  elseif group.patterns[p] then
    deckard.focus_group(g)
    if group.scene_pattern == p then deckard.scene_pattern(g, nil) else deckard.pick_pattern(p) end
  elseif p == #group.patterns + 1 then
    deckard.focus_group(g)
    deckard.pattern_edit("create")
  end
end

function M.midi(role, b)
  if role ~= "daw" then return end
  if is_sysex(b, LAYOUT) then
    -- (Back in Session: it drew nothing of ours meanwhile, so all of it again. The
    -- device also reports the layout it was just asked for: nothing to do then.)
    local session = body(b)[1] == SESSION
    if session and not st.session then st.lit = {} end
    st.session = session
    draw()
    return
  end
  if is_sysex(b, LIGHTS) then
    local done, why = clip.feed(st.print, body(b))
    if done then
      deckard.print_notes(done)
      deckard.message(string.format("Printed %d notes from the Launchpad", #done.notes))
    elseif why then
      deckard.message(why)
    end
    return
  end
  local status, data1, data2 = b[1] or 0, b[2] or 0, b[3] or 0
  local kind = status & 0xF0
  if kind == 0x90 and data2 > 0 then
    local row, col = data1 // 10, data1 % 10
    -- (The bottom row is the playhead's.)
    if row < 2 or row > 8 or col < 1 or col > 8 then return end
    press(row, col)
  elseif kind == 0xB0 and data1 == SHIFT then
    st.shift = data2 > 0
  elseif kind == 0xB0 and data2 > 0 then
    if data1 >= SCENE_ROW and data1 < SCENE_ROW + 8 then
      local sc = st.scenes + data1 - SCENE_ROW + 1
      local n = #deckard.state().scenes
      if st.mute then
        if sc <= n then mute_scene(deckard.state(), sc) end
      elseif sc <= n then deckard.launch_scene(sc)
      elseif sc == n + 1 then deckard.scene_edit("create") end
    elseif data1 == MUTE then
      st.mute = not st.mute
      draw()
    elseif data1 == PLAY then
      if deckard.state().playing then deckard.stop() else deckard.play() end
    elseif data1 == RECORD then
      deckard.record(not deckard.state().recording)
    elseif data1 == UP then scroll(-ROWS, 0)
    elseif data1 == DOWN then scroll(ROWS, 0)
    elseif data1 == LEFT then
      if st.shift then group_bank(-1) else scroll(0, -8) end
    elseif data1 == RIGHT then
      if st.shift then group_bank(1) else scroll(0, 8) end
    end
  end
end

return M
