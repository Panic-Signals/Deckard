-- The File page (FILE): the project's files.
--
--   display 2-4       NEW, SAVE (the Save page), SAVE COPY
--   display 5         EXPORT AUDIO
--   display 7 / 8     RECENT (the projects opened lately, on the right), LOAD the
--                     chosen one
--   jog wheel, knob 8 choose a recent project; the wheel's push loads it

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local back = require("lib.back")

local file = {}

-- The recent project chosen (1 the latest).
local focus = 1

local function recent()
  return deckard.state().recent
end

local function choose(steps)
  local n = #recent()
  if n > 0 then focus = math.max(1, math.min(n, focus + steps)) end
end

local function load()
  if recent()[focus] then deckard.file("load", focus) end
end

function file.button(name, down)
  if name == "WHEEL" then
    if down then load() end
    return true
  end
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if not n then return end
  if not down then return true end
  if n == 2 then
    deckard.file("new")
  elseif n == 3 then
    deckard.show_page("save")
  elseif n == 4 then
    deckard.file("save_copy")
    back.go()
  elseif n == 5 then
    back.go()
    deckard.file("export_audio")
  elseif n == 8 then
    load()
  end
  return true
end

function file.wheel(steps)
  choose(steps)
  return true
end

function file.knob(n, amount)
  if n == 8 then
    local steps = knobs.gather(n, amount, 0.05)
    if steps ~= 0 then choose(steps) end
  end
  return true
end

function file.leds()
  local any = #recent() > 0
  deckard.led("SCREEN_1", "on")
  for i = 2, 5 do deckard.led("SCREEN_" .. i, "dim") end
  deckard.led("SCREEN_6", "off")
  deckard.led("SCREEN_7", any and "on" or "off")
  deckard.led("SCREEN_8", any and "dim" or "off")
  deckard.led("LEFT", "off")
  deckard.led("RIGHT", "off")
end

-- The info bar with the project's name, as the File and Save pages show it.
function file.project_info(s)
  local info = widgets.focus_info(s)
  info.number, info.palette, info.name = "M", 16, s.project or ""
  return info
end

function file.draw(screen, theme)
  local s = deckard.state()
  local any = #s.recent > 0
  focus = math.max(1, math.min(focus, #s.recent))
  widgets.buttons(screen, theme, {
    [1] = { text = "FILE", pin = true, selected = true },
    [2] = { text = "NEW" },
    [3] = { text = "SAVE" },
    [4] = { text = "SAVE COPY" },
    [5] = { text = "EXPORT AUDIO" },
    [7] = { text = "RECENT", selected = any, disabled = not any },
    [8] = { text = "LOAD", disabled = not any },
  })
  widgets.info_bar(screen, theme, file.project_info(s))
  -- The recent projects down the right half of the right display, the chosen one lit.
  local rows = 9
  local first = math.max(1, math.min(focus - rows // 2, #s.recent - rows + 1))
  for i = first, math.min(#s.recent, first + rows - 1) do
    local y = 41 + (i - first) * 26
    if i == focus then screen:rect(721, y, 239, 26, "button_selected") end
    screen:label(729, y, 225, 26, s.recent[i], theme.styles[i == focus and "head_selected" or "default"], "left", "center")
  end
end

return file
