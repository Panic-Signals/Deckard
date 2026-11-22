-- The page a button shows while it's held (MUTE, SOLO, SELECT, PATTERN, SCENE), and
-- the pins on them: display button 1 pins the page so it stays when the button is
-- let go, as Maschine's. A pin stays for the next time until it's taken off.

local state = { shown = deckard.shown_pin() }

local pin = {}

-- The buttons whose pages are pinned.
pin.pinned = setmetatable({}, {
  __index = function(_, name)
    return deckard.pinned(name)
  end,
  __newindex = function(_, name, on)
    deckard.set_pinned(name, on == true)
  end,
})

-- Whether the page shown is pinned.
function pin.on()
  return state.shown ~= nil and pin.pinned[state.shown.button] == true
end

-- Whether button `name` acts as held: held, or its page pinned.
function pin.active(name)
  return deckard.held(name) or (state.shown ~= nil and state.shown.button == name)
end

return setmetatable(pin, {
  __index = function(_, k)
    if k == "shown" then return state.shown end
  end,
  __newindex = function(t, k, v)
    if k == "shown" then
      state.shown = v
      deckard.set_shown_pin(v and v.button or nil, v and v.back or nil)
    else
      rawset(t, k, v)
    end
  end,
})
