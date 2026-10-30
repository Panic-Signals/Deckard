-- The page a button shows while it's held (MUTE, SOLO, SELECT, PATTERN, SCENE), and
-- the pins on them: display button 1 pins the page so it stays when the button is
-- let go, as Maschine's. A pin stays for the next time until it's taken off.

local pin = {}

-- The page shown: { button = "SCENE", back = the page it was shown over, close = the
-- button was pressed again while pinned (it closes as it's let go) }, or nil.
pin.shown = nil

-- The buttons whose pages are pinned.
pin.pinned = {}

-- Whether the page shown is pinned.
function pin.on()
  return pin.shown ~= nil and pin.pinned[pin.shown.button] == true
end

-- Whether button `name` acts as held: held, or its page pinned.
function pin.active(name)
  return deckard.held(name) or (pin.shown ~= nil and pin.shown.button == name)
end

return pin
