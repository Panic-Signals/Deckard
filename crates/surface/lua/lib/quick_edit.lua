-- What the jog wheel changed (TEMPO, SWING, VOLUME), as the info bar shows it instead
-- of the tempo and scene: the value, then its name ("3.0 %", "MASTER SWING"), as
-- Maschine's quick edit. It shows from the first turn while the wheel's mode is on, and
-- for LINGER seconds after the last turn.

local wheel = require("lib.wheel")

local quick_edit = {}

local shown, changed_at = nil, nil
local LINGER = 1.5

-- The wheel changed `what` ("tempo", "swing" or "volume").
function quick_edit.changed(what)
  shown, changed_at = what, deckard.now()
  -- (Drawn again when it's due to go.)
  if deckard.redraw_until then deckard.redraw_until(changed_at + LINGER + 0.03) end
end

local function db_text(db)
  if db == -math.huge or db < -120 then return "-inf dB" end
  return string.format("%.1f dB", db)
end

-- The info bar's values while it shows, else nil.
function quick_edit.values(s)
  local lingers = changed_at and deckard.now() - changed_at < LINGER
  if not shown or (wheel.mode ~= shown and not lingers) then
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
