-- What MUTE, SOLO and SELECT show while held: the groups and the focused group's
-- pads, filled where they play (MUTE, SOLO) or are focused (SELECT).

local widgets = require("lib.widgets")
local pin = require("lib.pin")

local hold = {}

-- Which of them is held.
local function held()
  for _, b in ipairs({ "MUTE", "SOLO", "SELECT" }) do
    if pin.active(b) then return b end
  end
  return "MUTE"
end

local RIGHT = {
  MUTE = { "ALL ON", "NONE", "AUDIO" },
  SOLO = { "ALL ON", "NONE", "AUDIO" },
  SELECT = { "ALL", "NONE", "MULTI" },
}

function hold.leds()
  local what = held()
  deckard.led("SCREEN_1", pin.on() and "on" or "dim")
  for _, i in ipairs({ 4, 5, 6 }) do deckard.led("SCREEN_" .. i, "dim") end
  deckard.led("SCREEN_8", what == "SELECT" and "on" or "dim")
end

function hold.draw(screen, theme)
  local s = deckard.state()
  local group = s.groups[s.group]
  local snd = group and group.sounds[s.sound]
  local what = held()
  local select = what == "SELECT"
  local tiles, bank = widgets.group_bank(s, function(g, grp)
    if select then return g == s.group end
    return not grp.muted
  end)
  local right = RIGHT[what]
  widgets.buttons(screen, theme, {
    { text = what, pin = true, selected = pin.on() },
    nil,
    { arrows = true, text = "BANK " .. bank, left = bank > 1, right = bank <= #s.groups // 8 },
    nil,
    { text = right[1] },
    { text = right[2] },
    nil,
    { text = right[3] },
  })
  widgets.info_bar(screen, theme, {
    number = s.sound,
    palette = snd and snd.palette,
    name = snd and snd.name or "",
    values = {
      { text = string.format("%.2f", s.tempo) },
      { text = "BPM", title = true },
      { text = s.scene_name, palette = s.scene_palette },
    },
  })
  -- SELECT's tiles sit a little higher and further left than MUTE's.
  if select then
    widgets.group_tiles(screen, theme, tiles, 59, 100, -3)
  else
    widgets.group_tiles(screen, theme, tiles, 63, 100)
  end
  local list = widgets.sound_pads(s)
  if not select and group then
    for i, p in ipairs(list) do
      local sound = group.sounds[i]
      if sound and not p.empty then p.on = not sound.muted end
    end
  end
  widgets.pad_grid(screen, theme, list)
end

return hold
