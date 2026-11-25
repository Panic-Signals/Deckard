-- Touch focus timing for the knobs (the spec's "Touch focus"): a finger must rest on a
-- knob 120 ms before it takes focus, focus stays 180 ms after the finger lifts, and a
-- knob turned with no touch registered has it for 1 s after its last turn. Times are
-- seconds (`deckard.now()`).
local touch = {}

local REST, LINGER, TURN = 0.12, 0.18, 1.0

local down = {} -- n -> when the finger came down
local lit_until = {} -- n -> end of the linger after a lift
local turned = {} -- n -> when it was last turned untouched

function touch.down(n, now)
  down[n] = down[n] or now
  lit_until[n] = nil
end

function touch.up(n, now)
  local since = down[n]
  down[n] = nil
  if since and now - since >= REST then lit_until[n] = now + LINGER end
end

function touch.turned(n, now)
  if not down[n] then turned[n] = now end
end

-- The knobs in focus at `now` (ascending). Asks for the screens to be redrawn while a
-- timer is still pending, so a focus starting or ending with no event shows up.
function touch.focused(now)
  local out, next_at = {}, nil
  local function pending(t)
    if t > now and (not next_at or t < next_at) then next_at = t end
  end
  for n = 1, 8 do
    local on = false
    if down[n] then
      if now - down[n] >= REST then
        on = true
      else
        pending(down[n] + REST)
      end
    end
    if lit_until[n] then
      if now < lit_until[n] then
        on = true
        pending(lit_until[n])
      else
        lit_until[n] = nil
      end
    end
    if turned[n] then
      if now - turned[n] < TURN then
        on = true
        pending(turned[n] + TURN)
      else
        turned[n] = nil
      end
    end
    if on then out[#out + 1] = n end
  end
  if next_at and deckard.redraw_until then deckard.redraw_until(next_at + 0.03) end
  return out
end

return touch
