-- Knob pages a page turns with LEFT / RIGHT (Pad mode's, Keyboard's): which one is
-- shown, the arrows' lights and the "2/4" over the right knob.

local paged = {}

-- Pages `count` long, starting at the first.
function paged.new(count)
  return { at = 1, count = count }
end

-- LEFT / RIGHT turn the pages (whether `name` was one of them).
function paged.button(p, name)
  if name == "LEFT" then
    p.at = math.max(1, p.at - 1)
  elseif name == "RIGHT" then
    p.at = math.min(p.count, p.at + 1)
  else
    return false
  end
  return true
end

function paged.leds(p)
  deckard.led("LEFT", p.at > 1 and "dim" or "off")
  deckard.led("RIGHT", p.at < p.count and "dim" or "off")
end

function paged.label(p)
  return p.at .. "/" .. p.count
end

return paged
