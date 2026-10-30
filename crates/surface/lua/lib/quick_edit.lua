-- What the jog wheel changed (TEMPO, SWING, VOLUME), as the info bar shows it instead
-- of the tempo and scene: the value, then its name ("3.0 %", "MASTER SWING"), as
-- Maschine's quick edit. It shows from the first turn until the wheel's mode is off.

local wheel = require("lib.wheel")

local quick_edit = {}

local shown = nil

-- The wheel changed `what` ("tempo", "swing" or "volume").
function quick_edit.changed(what)
  shown = what
end

local function db_text(db)
  if db == -math.huge or db < -120 then return "-inf dB" end
  return string.format("%.1f dB", db)
end

-- The info bar's values while it shows, else nil.
function quick_edit.values(s)
  if not shown or wheel.mode ~= shown then
    shown = nil
    return nil
  end
  local value, title
  if shown == "tempo" then
    value, title = string.format("%.2f BPM", s.tempo), "TEMPO"
  elseif shown == "swing" then
    value, title = string.format("%.1f %%", s.swing), "MASTER SWING"
  else
    value, title = db_text(s.volume_db), "MASTER VOL"
  end
  return { { text = value }, { text = title, title = true } }
end

return quick_edit
