-- Pad mode's page (PAD MODE): the groups, the focused sound's base key, its settings
-- on two pages of knobs, and the focused group's pads.
--
--   display 3 / 4     the previous / next bank of groups
--   display 5 / 6     the base key down / up an octave
--   display 7 / 8     the base key down / up a semitone
--   LEFT / RIGHT      the knobs' pages:
--     1/2  knobs 1-4  Choke group and mode, Link group and mode
--     2/2  knobs 1-2  the focused group's and sound's colours
--          knobs 3-4  Fixed Velocity's velocity; Focus: whether a pad's hit focuses
--                     its sound (On Trigger) or not (Pinned)

local pads = require("lib.pads")
local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local paged = require("lib.paged")

local page = {}

local pages = paged.new(2)
-- (The colour knobs list Default first, then the palette.)
local FOCUS = { "On Trigger", "Pinned" }

local GROUPS = { [0] = "None", "1", "2", "3", "4", "5", "6", "7", "8" }
local MODES = { [0] = "Send", "Receive" }
local SETTINGS = {
  { field = "choke_group", name = "GROUP", list = GROUPS },
  { field = "choke_mode", name = "MODE", list = MODES },
  { field = "link_group", name = "GROUP", list = GROUPS },
  { field = "link_mode", name = "MODE", list = MODES },
}

local function sound()
  local s = deckard.state()
  local g = s.groups[s.group]
  return s, g and g.sounds[s.sound]
end

function page.button(name, down)
  if not down then return end
  local s = deckard.state()
  local steps = { SCREEN_5 = -12, SCREEN_6 = 12, SCREEN_7 = -1, SCREEN_8 = 1 }
  if steps[name] then
    pads.move_base_key(s.group, s.sound, steps[name])
  elseif paged.button(pages, name) then
    return true
  elseif name == "SCREEN_3" or name == "SCREEN_4" then
    local bank = (s.group - 1) // 8 + (name == "SCREEN_3" and -1 or 1)
    local g = bank * 8 + 1
    if bank >= 0 and g <= #s.groups then deckard.focus_group(g) end
  else
    return
  end
  return true
end

-- A colour knob's step: Default, then the palette's 16 (`what` "group" or "sound").
local function colour(what, of, steps)
  local i = of.default_color and 0 or of.palette + 1
  local to = math.max(0, math.min(16, i + steps))
  if to ~= i then deckard.set_color(what, to > 0 and to - 1 or nil) end
end

local function second(n, amount)
  local s, snd = sound()
  if n == 1 or n == 2 or n == 4 then
    local steps = knobs.gather(n, amount)
    if steps == 0 then return end
    if n == 4 then
      pads.focus_pinned = steps > 0
    elseif n == 1 and s.groups[s.group] then
      colour("group", s.groups[s.group], steps)
    elseif n == 2 and snd then
      colour("sound", snd, steps)
    end
  elseif n == 3 then
    local v = math.max(0, math.min(127, s.velocity + knobs.each(amount)))
    if v ~= s.velocity then deckard.velocity(v) end
  end
end

function page.knob(n, amount)
  if pages.at == 2 then
    second(n, amount)
    return true
  end
  local setting = SETTINGS[n]
  if not setting then return true end
  local _, snd = sound()
  if not snd then return true end
  local step = knobs.each(amount)
  if step ~= 0 then
    local v = math.max(0, math.min(#setting.list, snd[setting.field] + step))
    if v ~= snd[setting.field] then deckard.sound_setting(setting.field, v) end
  end
  return true
end

function page.leds()
  deckard.led("SCREEN_1", "on")
  for i = 4, 8 do deckard.led("SCREEN_" .. i, "dim") end
  paged.leds(pages)
end

local function colour_name(of)
  if not of or of.default_color then return "Default" end
  return widgets.COLOUR_NAMES[of.palette + 1] or ""
end

function page.draw(screen, theme)
  local s, snd = sound()
  local tiles, bank = widgets.group_bank(s, function(g) return g == s.group end)
  widgets.buttons(screen, theme, {
    { text = "PAD MODE", selected = true },
    nil,
    { arrows = true, text = "BANK " .. bank, left = bank > 1, right = bank <= #s.groups // 8 },
    nil,
    { text = "OCTAVE-" },
    { text = "OCTAVE+" },
    { text = "SEMITONE-" },
    { text = "SEMITONE+" },
  })
  widgets.info_bar(screen, theme, {
    number = s.sound,
    palette = snd and snd.palette,
    name = snd and snd.name or "",
    values = { { text = pads.note_name(pads.base_key(s.group, s.sound)) }, { text = "BASE KEY", title = true } },
  })
  widgets.group_tiles(screen, theme, tiles, 63, 68)
  if pages.at == 2 then
    widgets.params(screen, theme, 0, { { text = "Colors", at = 1 }, { text = "Fixed Velocity", at = 3 } }, paged.label(pages), {
      { name = "GROUP", value = colour_name(s.groups[s.group]) },
      { name = "SOUND", value = colour_name(snd) },
      { name = "VELOCITY", value = tostring(s.velocity) },
      { name = "MODE", value = FOCUS[pads.focus_pinned and 2 or 1] },
    })
  else
    local params = {}
    for i, setting in ipairs(SETTINGS) do
      params[i] = { name = setting.name, value = snd and setting.list[snd[setting.field]] or "" }
    end
    widgets.params(screen, theme, 0, { { text = "Choke", at = 1 }, { text = "Link", at = 3 } }, paged.label(pages), params)
  end
  widgets.pad_grid(screen, theme, widgets.sound_pads(s))
end

return page
