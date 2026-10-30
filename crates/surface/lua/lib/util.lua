-- Helpers the pages share.

local util = {}

-- Round to `places` decimals.
function util.round(x, places)
  local m = 10 ^ (places or 0)
  return math.floor(x * m + 0.5) / m
end

function util.clamp(x, lo, hi)
  return math.max(lo, math.min(hi, x))
end

-- Where `x` is in list `t` (from 1), or nil.
function util.index_of(t, x)
  for i, v in ipairs(t) do
    if v == x then return i end
  end
  return nil
end

-- The play position as bar.beat.sixteenth, counting from 1 (ticks: 960 a quarter).
function util.bar_beat(ticks, numerator, denominator)
  local beat = 960 * 4 // math.max(denominator or 4, 1)
  local bar = beat * math.max(numerator or 4, 1)
  local t = math.max(ticks or 0, 0)
  local sixteenth = (t % beat) // math.max(math.min(240, beat), 1)
  return string.format("%d.%d.%d", t // bar + 1, (t % bar) // beat + 1, sixteenth + 1)
end

-- A length as bars:beats:sixteenths from 0 ("4:0:0").
function util.span(ticks, numerator, denominator)
  local beat = 960 * 4 // math.max(denominator or 4, 1)
  local bar = beat * math.max(numerator or 4, 1)
  return string.format("%d:%d:%d", ticks // bar, ticks % bar // beat, ticks % beat // 240)
end

-- The focused group and sound (nil when there are none).
function util.focus(s)
  local g = s.groups[s.group]
  return g, g and g.sounds[s.sound]
end

-- A name, or what stands in for an empty one.
function util.label(name, otherwise)
  if name == nil or name:match("^%s*$") then return otherwise end
  return name
end

-- A..H for groups 1-8, then A2.. (as the software names them).
function util.group_letter(g)
  local letter = string.char(string.byte("A") + (g - 1) % 8)
  if g <= 8 then return letter end
  return letter .. ((g - 1) // 8 + 1)
end

return util
