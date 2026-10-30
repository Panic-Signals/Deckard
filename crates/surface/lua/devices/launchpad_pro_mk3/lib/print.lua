-- Print to Clip: the Launchpad's sequencer sends a pattern in SysEx packets
-- (docs/superpowers/specs/2026-10-28-launchpad-pro-design.md). `d` is the bytes after
-- the command byte (`03`), from 1: d[1] the kind (1 begin, 2 data, 3 end), d[2..9] the
-- packet id (4 bits a byte), d[10] 4, d[11] the sequencer's track (from 0), d[12..] the
-- payload. Times are in 1/500 beat. (The end packet's payload looks like a checksum.)

local print = {}

local BEGIN, DATA, END = 1, 2, 3
-- A data packet's fewest bytes: the kind, the id, two unused, a 3-byte value.
local SHORTEST = 13

function print.new()
  return { active = false, id = 0, bad = false, length = 0, notes = {}, track = nil }
end

-- `n` bytes of 7 bits from d[i], most significant first.
local function value(d, i, n)
  local v = 0
  for k = 0, n - 1 do v = v * 128 + (d[i + k] or 0) end
  return v
end

-- Take packet `d`. At the end: the print ({ length = beats, notes = { {pitch, start,
-- length, velocity} } }), or nil and why it was dropped.
function print.feed(s, d)
  local kind = d[1]
  local id = 0
  for k = 2, 9 do id = id * 16 + ((d[k] or 0) & 15) end
  if kind == BEGIN then
    local cut = s.active
    s.active, s.id, s.bad, s.length, s.notes = true, id, false, 0, {}
    s.track = (d[11] or 0) + 1
    if cut then return nil, "Print to Clip: the end of the last print went missing, it wasn't printed" end
    return nil
  end
  if not s.active then
    -- (Said once, at the end of the print whose start went missing.)
    if kind == END then return nil, "Print to Clip: the start of the print went missing, nothing was printed" end
    return nil
  end
  if id ~= 0 and id ~= s.id + 1 then s.bad = true end
  s.id = id
  if kind == DATA and #d >= SHORTEST then
    local n = #d - 11
    if n == 3 then
      s.length = value(d, 12, 3) / 500
    elseif (n - 3) % 6 == 0 then
      local offset = value(d, 12, 3)
      for i = 15, #d - 5, 6 do
        table.insert(s.notes, {
          d[i + 4],
          (offset + value(d, i, 2)) / 500,
          value(d, i + 2, 2) / 500,
          d[i + 5],
        })
      end
    end
  elseif kind == END then
    s.active = false
    if s.bad then return nil, "Print to Clip: a packet went missing, nothing was printed" end
    table.sort(s.notes, function(a, b) return a[2] < b[2] end)
    return { length = s.length, notes = s.notes, track = s.track }
  end
  return nil
end

return print
