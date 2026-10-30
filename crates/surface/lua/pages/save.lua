-- The Save page (FILE ▸ SAVE): save the project, the focused group or the focused
-- sound.
--
--   knob 1            CONTENT TYPE: Project, Group or Sound
--   knob 2            SELECTED: the group or sound to save
--   group buttons     pick a group (Group); pads pick a sound (Sound)
--   display 3 / 4     the previous / next eight groups
--   display 5 / 8     CANCEL (back to the File page), SAVE

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local back = require("lib.back")

local save = {}

local TYPES = { "Project", "Group", "Sound" }
local PROJECT, GROUP, SOUND = 1, 2, 3
local kind = PROJECT

-- The bank of eight groups shown (from 0); none: the focused group's.
local bank = nil

local function shown_bank(s)
  return bank or (s.group - 1) // 8
end

function save.button(name, down)
  local s = deckard.state()
  local letter = name:match("^GROUP_(%u)$")
  if letter then
    if down then
      local g = shown_bank(s) * 8 + string.byte(letter) - string.byte("A") + 1
      if s.groups[g] then
        kind = GROUP
        deckard.focus_group(g)
      end
    end
    return true
  end
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if not n then return end
  if not down then return true end
  if n == 3 and shown_bank(s) > 0 then
    bank = shown_bank(s) - 1
  elseif n == 4 and (shown_bank(s) + 1) * 8 < #s.groups then
    bank = shown_bank(s) + 1
  elseif n == 5 then
    kind, bank = PROJECT, nil
    deckard.show_page("file")
  elseif n == 8 then
    deckard.file(kind == PROJECT and "save" or kind == GROUP and "save_group" or "save_sound")
    kind, bank = PROJECT, nil
    back.go()
  end
  return true
end

function save.pad(n)
  local s = deckard.state()
  local group = s.groups[s.group]
  -- (An empty slot isn't focused, but the content type is Sound.)
  kind = SOUND
  local snd = group and group.sounds[n]
  if snd and snd.used and n ~= s.sound then deckard.focus_sound(n) end
  return true
end

function save.knob(n, amount)
  local steps = knobs.gather(n, amount)
  if steps == 0 then return true end
  local s = deckard.state()
  if n == 1 then
    kind = math.max(PROJECT, math.min(SOUND, kind + steps))
  elseif n == 2 and kind == GROUP then
    local g = math.max(1, math.min(#s.groups, s.group + steps))
    if g ~= s.group then deckard.focus_group(g) end
  elseif n == 2 and kind == SOUND then
    local group = s.groups[s.group]
    local k = math.max(1, math.min(group and #group.sounds or 1, s.sound + steps))
    if k ~= s.sound then deckard.focus_sound(k) end
  end
  return true
end

function save.leds()
  local s = deckard.state()
  deckard.led("SCREEN_1", "on")
  deckard.led("SCREEN_2", "off")
  deckard.led("SCREEN_3", shown_bank(s) > 0 and "dim" or "off")
  deckard.led("SCREEN_4", (shown_bank(s) + 1) * 8 < #s.groups and "dim" or "off")
  deckard.led("SCREEN_5", "dim")
  deckard.led("SCREEN_6", "off")
  deckard.led("SCREEN_7", "off")
  deckard.led("SCREEN_8", "on")
  deckard.led("LEFT", "off")
  deckard.led("RIGHT", "off")
  local first = shown_bank(s) * 8
  for i = 1, 8 do
    local g = first + i
    local group = s.groups[g]
    local name = "GROUP_" .. string.char(string.byte("A") + i - 1)
    if group then deckard.led(name, group.palette, kind == GROUP and g == s.group) else deckard.led(name, "off") end
  end
  local group = s.groups[s.group]
  for i = 1, 16 do
    local snd = group and group.sounds[i]
    if snd and snd.used then
      deckard.led("PAD_" .. i, snd.palette, kind == SOUND and i == s.sound)
    else
      deckard.led("PAD_" .. i, "off")
    end
  end
end

function save.draw(screen, theme)
  local s = deckard.state()
  local b = shown_bank(s)
  widgets.buttons(screen, theme, {
    [1] = { text = "SAVE", selected = true },
    [3] = { arrows = true, text = "BANK " .. (b + 1), left = b > 0, right = (b + 1) * 8 < #s.groups },
    [5] = { text = "CANCEL" },
    -- (Lit, but drawn as any button.)
    [8] = { text = "SAVE" },
  })
  local info
  if kind == PROJECT then
    info = require("pages.file").project_info(s)
  else
    info = widgets.focus_info(s)
    if kind == GROUP then
      local group = s.groups[s.group]
      info.number = string.char(string.byte("A") + (s.group - 1) % 8) .. ((s.group - 1) // 8 + 1)
      info.palette, info.name = group and group.palette, group and group.name or ""
    end
  end
  widgets.info_bar(screen, theme, info)
  local tiles = {}
  for i = 1, 8 do
    local g = b * 8 + i
    local group = s.groups[g]
    if group then
      tiles[i] = { big = string.char(string.byte("A") + i - 1) .. (b + 1), name = group.name, palette = group.palette,
        on = kind == GROUP and g == s.group }
    end
  end
  widgets.group_tiles(screen, theme, tiles, 59, 68, -3)
  local selected = nil
  if kind == GROUP then
    selected = s.groups[s.group] and s.groups[s.group].name
  elseif kind == SOUND then
    local group = s.groups[s.group]
    selected = group and group.sounds[s.sound] and group.sounds[s.sound].name
  end
  widgets.params(screen, theme, 0, { { text = "Content Selector", at = 1 } }, nil, {
    [1] = { name = "CONTENT TYPE", value = TYPES[kind] },
    [2] = selected and { name = "SELECTED", value = selected } or nil,
  })
  local pads = widgets.sound_pads(s)
  if kind ~= SOUND then
    for _, p in pairs(pads) do p.on = false end
  end
  widgets.pad_grid(screen, theme, pads)
end

return save
