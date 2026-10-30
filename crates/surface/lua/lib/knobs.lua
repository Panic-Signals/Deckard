-- Knobs that step through values, as Maschine's do: most take a step for each turn
-- the knob reports, however small; a list of names (a COLOR knob) gathers turns until
-- they make a tenth of a turn, then takes one step.

local knobs = {}

-- How far a gathering knob turns for its step (a fraction of a turn).
knobs.GATHER = 0.1

local gathered = {}

-- Touched twice within this long (seconds), a knob is double-tapped; when each knob
-- was last touched.
knobs.DOUBLE_TAP = 0.4
local touched_at = {}

-- A turn of `amount`: one step up or down (none for no turn).
function knobs.each(amount)
  if amount > 0 then return 1 elseif amount < 0 then return -1 end
  return 0
end

-- Turn gathering knob `n` by `amount`: a step up or down once the turns since its last
-- step make `step` (`knobs.GATHER` if not given; turning back starts again), else none.
function knobs.gather(n, amount, step)
  local g = gathered[n] or 0
  if (g > 0 and amount < 0) or (g < 0 and amount > 0) then g = 0 end
  g = g + amount
  -- (A hair under, so exactly a step's worth is one.)
  if math.abs(g) >= (step or knobs.GATHER) - 1e-9 then
    gathered[n] = 0
    return g > 0 and 1 or -1
  end
  gathered[n] = g
  return 0
end

-- A knob touched (button `name`, "CAP_n"): its number if this touch is its second
-- within `knobs.DOUBLE_TAP`, else nil.
function knobs.double_tap(name, down)
  local n = tonumber(name:match("^CAP_(%d)$"))
  if not n or not down then return nil end
  local now, last = deckard.now(), touched_at[n]
  if last and now - last < knobs.DOUBLE_TAP then
    touched_at[n] = nil
    return n
  end
  touched_at[n] = now
  return nil
end

return knobs
