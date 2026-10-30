-- The Settings page (SETTINGS): the app's preferences, a tab at a time.
--
--   display 1         SETTINGS
--   display 5 / 6     the tab before / after (GENERAL, AUDIO, MIDI, HARDWARE)
--   jog wheel ◀ ▶     the same
--   LEFT / RIGHT      the tab's knob pages
--   knobs             its settings (one DECKARD doesn't have yet shows "-")

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local paged = require("lib.paged")

local settings = {}

local function value(name)
  return deckard.state().settings.values[name] or "-"
end

-- A knob for one of the app's settings: a step a tenth of a turn (`each`: a step for
-- every turn, for a level).
local function setting(name, key, each)
  return { name = name, value = function() return value(key) end, key = key, each = each }
end

-- A knob for a setting DECKARD doesn't have (yet): its name, nothing to turn.
local function absent(name)
  return { name = name, value = function() return "-" end }
end

local function on_off(on)
  return on and "On" or "Off"
end

-- The routings of four channels (from `first`, 1-based): IN 1L, IN 1R …
local function routings(dir, first)
  local page = { groups = { { text = (dir == "in" and "Input" or "Output") .. " Routings (" .. first .. "-" .. first + 3 .. ")", at = 1 } }, knobs = {} }
  for k = 1, 8 do
    local n = first + (k - 1) // 2
    local ch = (n - 1) * 2 + (k - 1) % 2 + 1
    local key = dir .. "_route"
    page.knobs[k] = {
      name = dir:upper() .. " " .. n .. (k % 2 == 1 and "L" or "R"),
      value = function() return value(key .. "_" .. ch) end,
      key = key,
      index = ch,
    }
  end
  return page
end

-- MIDI's device chosen to switch (1 the first), inputs and outputs.
local chosen = { midi_inputs = 1, midi_outputs = 1 }

local function ports(list)
  return deckard.state().settings[list]
end

local function port_knobs(list, key)
  local function port()
    return ports(list)[chosen[list]]
  end
  return {
    name = "SELECT",
    value = function() local p = port() return p and p.name or "-" end,
    turn = function(step) chosen[list] = math.max(1, math.min(#ports(list), chosen[list] + step)) end,
  }, {
    name = "STATUS",
    value = function() local p = port() return p and on_off(p.on) or "-" end,
    turn = function(step) if port() then deckard.setting(key, step, chosen[list]) end end,
  }
end

local in_select, in_status = port_knobs("midi_inputs", "midi_in")
local out_select, out_status = port_knobs("midi_outputs", "midi_out")

local TABS = {
  {
    name = "GENERAL",
    icon = "general",
    pages = {
      {
        groups = { { text = "Metronome", at = 1 }, { text = "Count-In", at = 5 }, { text = "Time Signature", at = 6 } },
        knobs = {
          {
            name = "ENABLED",
            value = function() return on_off(deckard.state().metronome) end,
            turn = function(step) if (step > 0) ~= deckard.state().metronome then deckard.metronome(step > 0) end end,
          },
          setting("VOLUME", "metronome_volume", true),
          setting("TIMESIGNATURE", "metronome_ticks"),
          setting("AUTOENABLE", "metronome_auto"),
          setting("COUNTINLENGTH", "count_in"),
          { name = "NUMERATOR", value = function() return tostring(deckard.state().numerator) end, key = "numerator" },
          { name = "DENOMINATOR", value = function() return tostring(deckard.state().denominator) end, key = "denominator" },
        },
      },
      {
        groups = { { text = "Project", at = 1 }, { text = "Quantize", at = 3 }, { text = "Scene / Section", at = 5 }, { text = "Pattern", at = 7 } },
        knobs = {
          setting("RELOAD", "reload_last"), setting("AUTO BACKUP", "auto_backup"), setting("QUANTIZEMODE", "quantize"), nil,
          setting("DUPLICATE", "duplicate"), setting("LINK WHEN DUPL", "link_duplicates"),
          -- (A bar a step; with SHIFT, a sixteenth.)
          {
            name = "LENGTH",
            value = function() return value("pattern_length") end,
            turn = function(step) deckard.setting(deckard.held("SHIFT") and "pattern_length_fine" or "pattern_length", step) end,
          },
          setting("GROW", "pattern_grow"),
        },
      },
      {
        groups = { { text = "Colors", at = 1 } },
        knobs = { absent("SCENE"), absent("GROUP"), absent("SOUND"), absent("LOAD COLORS") },
      },
    },
  },
  {
    name = "AUDIO",
    icon = "audio",
    pages = {
      {
        groups = { { text = "Interface", at = 1 } },
        knobs = {
          { name = "DRIVER", value = function() return value("driver") end },
          setting("DEVICE", "device"),
          { name = "SAMPLE RATE", value = function() return value("sample_rate") end },
          setting("BUFFER SIZE", "buffer_size"),
        },
      },
      routings("in", 1), routings("in", 5), routings("in", 9), routings("in", 13),
      routings("out", 1), routings("out", 5), routings("out", 9), routings("out", 13),
    },
  },
  {
    name = "MIDI",
    icon = "midi",
    pages = {
      {
        groups = { { text = "MIDI Sync", at = 1 }, { text = "MIDI Input", at = 3 }, { text = "Input Devices", at = 5 }, { text = "Output Devices", at = 7 } },
        knobs = {
          setting("MODE", "midi_sync"),
          -- (CLOCK OFFSET only while syncing.)
          { name = "CLOCK OFFSET", value = function() return value("clock_offset") end, key = "clock_offset", shown = function() return value("midi_sync") ~= "Off" end },
          setting("DEFAULT", "default_input"),
          nil,
          in_select, in_status, out_select, out_status,
        },
      },
      {
        groups = { { text = "MIDI Change", at = 1 }, { text = "Link", at = 5 } },
        knobs = {
          absent("TYPE"), absent("TRIGGER"), nil, nil,
          { name = "ENABLED", value = function() return on_off(deckard.state().settings.link) end, key = "link" },
          { name = "START SYNC", value = function() return value("link_start_stop") end, key = "link_start_stop" },
        },
      },
    },
  },
  {
    name = "HARDWARE",
    icon = "hardware",
    pages = {
      {
        groups = { { text = "Pads", at = 1 }, { text = "LEDs", at = 3 }, { text = "Touchstrip", at = 4 }, { text = "Display", at = 5 },
          { text = "MK3 Outputs", at = 6 }, { text = "Touch-Sensitive Knobs", at = 7 } },
        knobs = {
          setting("SENSITIVITY", "sensitivity", true),
          setting("SCALING", "velocity_scaling"),
          setting("BRIGHTNESS", "led_brightness", true),
          setting("SHOW POSITION", "strip_position"),
          setting("BRIGHTNESS", "display_brightness", true),
          -- (DECKARD keeps the headphones to their own output.)
          { name = "MAIN", value = function() return "Line Out" end },
          setting("AUTO-WRITE", "touch_auto_write"),
        },
      },
    },
  },
}

local tab = 1
local pages = paged.new(#TABS[1].pages)

local function select(n)
  if n < 1 or n > #TABS or n == tab then return end
  tab = n
  pages = paged.new(#TABS[n].pages)
end

local function page()
  return TABS[tab].pages[pages.at]
end

function settings.button(name, down)
  if name == "LEFT" or name == "RIGHT" then
    if down then paged.button(pages, name) end
    return true
  end
  if name == "WHEEL_LEFT" or name == "WHEEL_RIGHT" then
    if down then select(tab + (name == "WHEEL_RIGHT" and 1 or -1)) end
    return true
  end
  -- (The rows of tabs: four fit one row.)
  if name == "WHEEL_UP" or name == "WHEEL_DOWN" or name == "WHEEL" then return true end
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if not n then return end
  if down and (n == 5 or n == 6) then select(tab + (n == 6 and 1 or -1)) end
  return true
end

function settings.wheel()
  return true
end

function settings.knob(n, amount)
  local k = page().knobs[n]
  if not k or (k.shown and not k.shown()) or not (k.key or k.turn) then return true end
  local step = k.each and knobs.each(amount) or knobs.gather(n, amount)
  if step == 0 then return true end
  if k.turn then
    k.turn(step)
  else
    deckard.setting(k.key, step, k.index)
  end
  return true
end

function settings.leds()
  deckard.led("SCREEN_1", "on")
  for i = 2, 8 do deckard.led("SCREEN_" .. i, "off") end
  deckard.led("SCREEN_5", tab > 1 and "dim" or "off")
  deckard.led("SCREEN_6", tab < #TABS and "dim" or "off")
  deckard.led("WHEEL_LEFT", tab > 1 and 16 or "off", false)
  deckard.led("WHEEL_RIGHT", tab < #TABS and 16 or "off", false)
  deckard.led("WHEEL_UP", "off")
  deckard.led("WHEEL_DOWN", "off")
  paged.leds(pages)
end

-- Each tab's picture, drawn about (cx, cy) in the right screen's left column.
local ICONS = {}

function ICONS.general(screen, cx, cy, c)
  for i = 0, 7 do
    local a = i * math.pi / 4
    screen:line(cx + math.cos(a) * 22, cy + math.sin(a) * 22, cx + math.cos(a) * 36, cy + math.sin(a) * 36, 13, c)
  end
  screen:circle(cx, cy, 27, c)
  screen:circle(cx, cy, 13, "background")
end

function ICONS.audio(screen, cx, cy, c)
  screen:rect(cx - 40, cy - 10, 14, 20, c)
  screen:triangle(cx - 30, cy - 28, 28, 56, "left", c)
  screen:rect(cx + 6, cy - 14, 3, 28, c)
  screen:rect(cx + 16, cy - 22, 3, 44, c)
  screen:rect(cx + 26, cy - 30, 3, 60, c)
end

function ICONS.midi(screen, cx, cy, c)
  screen:circle(cx, cy, 34, c)
  for i = 0, 4 do
    local a = math.pi + i * math.pi / 4
    screen:circle(cx + math.cos(a) * 20, cy + math.sin(a) * 20, 5, "background")
  end
  screen:circle(cx, cy, 5, "background")
  screen:rect(cx - 7, cy + 24, 14, 12, "background")
end

function ICONS.hardware(screen, cx, cy, c)
  screen:frame(cx - 38, cy - 34, 76, 68, c, 2)
  screen:rect(cx - 32, cy - 28, 64, 12, c)
  for r = 0, 3 do
    for col = 0, 3 do screen:rect(cx + 2 + col * 8, cy - 8 + r * 8, 6, 6, c) end
    screen:rect(cx - 32, cy - 8 + r * 8, 28, 5, c)
  end
end

-- An arrow up a line to (x, y).
local function arrow_up(screen, x, y, h, c)
  screen:rect(x - 1, y + 6, 2, h - 6, c)
  screen:triangle(x - 4, y, 8, 8, "up", c)
end

local function draw_tab(screen, theme, s)
  local t = TABS[tab]
  ICONS[t.icon](screen, 600, 120, "button")
  local style = theme.styles.default
  local small = theme.styles.pad_label
  if t.name == "AUDIO" then
    -- (The device's status; its latencies, smaller; their total.)
    local lines = s.settings.audio_status
    if lines[1] then screen:label(720, 72, 240, 24, lines[1], style, "center", "center") end
    for i = 2, #lines - 1 do
      screen:label(720, 96 + (i - 2) * 14, 240, 14, lines[i], small, "center", "center")
    end
    if #lines > 1 then screen:label(720, 148, 240, 16, lines[#lines], small, "center", "center") end
  elseif t.name == "MIDI" then
    local link = "LINK OFF"
    if s.settings.link then
      local n = s.settings.link_peers
      link = n == 1 and "1 LINK" or (n .. " LINKS")
    end
    screen:label(720, 100, 240, 24, link, style, "center", "center")
  elseif t.name == "HARDWARE" then
    -- (The MK3's outputs: Main on 1 and 2, the headphones on 3 and 4.)
    screen:label(786, 22, 12, 16, "L", small, "center", "center")
    screen:label(802, 22, 12, 16, "R", small, "center", "center")
    screen:circle(789, 42, 5, "text")
    screen:circle(808, 42, 5, "text")
    arrow_up(screen, 798, 54, 92, "text")
    screen:label(758, 154, 80, 18, "Main", theme.styles.knob_name, "center", "center")
    screen:label(758, 172, 80, 14, "1, 2", theme.styles.info_title, "center", "center")
    screen:circle(880, 36, 10, "text", 2)
    screen:rect(869, 36, 5, 9, "text")
    screen:rect(886, 36, 5, 9, "text")
    arrow_up(screen, 880, 54, 92, "text")
    screen:label(840, 154, 80, 18, "Phones", theme.styles.knob_name, "center", "center")
    screen:label(840, 172, 80, 14, "3, 4", theme.styles.info_title, "center", "center")
  end
end

-- The tabs, as the left screen lists them: the one shown filled.
local function draw_categories(screen, theme)
  for i, t in ipairs(TABS) do
    local x = 12 + (i - 1) * 117
    if i == tab then
      screen:rect(x, 104, 111, 44, "button_selected", theme.corner)
      widgets.chase(screen, theme, x, 104, 111, 44)
    else
      screen:frame(x, 104, 111, 44, "button", theme.corner)
    end
    screen:label(x, 104, 111, 44, t.name, theme.styles[i == tab and "head_selected" or "head"])
  end
end

function settings.draw(screen, theme)
  local s = deckard.state()
  local t = TABS[tab]
  widgets.buttons(screen, theme, {
    [1] = { text = "SETTINGS", selected = true },
    [5] = { arrows = true, text = t.name, left = tab > 1, right = tab < #TABS },
  })
  widgets.info_bar(screen, theme, widgets.focus_info(s))
  draw_categories(screen, theme)
  draw_tab(screen, theme, s)
  local p = page()
  local sides = { {}, {} }
  for k = 1, 8 do
    local knob = p.knobs[k]
    if knob and (not knob.shown or knob.shown()) then
      sides[(k - 1) // 4 + 1][(k - 1) % 4 + 1] = { name = knob.name, value = knob.value() }
    end
  end
  local groups = { {}, {} }
  for _, g in ipairs(p.groups) do
    table.insert(groups[(g.at - 1) // 4 + 1], { text = g.text, at = (g.at - 1) % 4 + 1 })
  end
  widgets.params(screen, theme, 0, groups[1], nil, sides[1])
  widgets.params(screen, theme, 480, groups[2], pages.count > 1 and paged.label(pages) or nil, sides[2])
end

return settings
