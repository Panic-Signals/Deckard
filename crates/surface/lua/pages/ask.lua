-- The question about unsaved changes (NEW, LOAD, Revert or Quit with changes not
-- saved), as the app's own dialog asks it. It shows over whatever page is up while the
-- question waits (`deckard.state().ask`), and that page comes back once it's answered
-- here or on the computer.
--
--   display 5         SAVE, then go on (not for Revert)
--   display 6         DON'T SAVE: go on without saving (REVERT for Revert)
--   display 8         CANCEL

local widgets = require("lib.widgets")

local ask = {}

local function question()
  return deckard.state().ask
end

function ask.button(name, down)
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if not n then
    -- (Nothing else is pressed under the question: not NEW again, not another page.)
    return name ~= "SHIFT"
  end
  if not down then return true end
  local q = question()
  if n == 5 and q and not q.revert then
    deckard.ask("save")
  elseif n == 6 then
    deckard.ask("discard")
  elseif n == 8 then
    deckard.ask("cancel")
  end
  return true
end

function ask.wheel()
  return true
end

function ask.knob()
  return true
end

function ask.leds()
  local q = question()
  for i = 1, 8 do deckard.led("SCREEN_" .. i, "off") end
  if q and not q.revert then deckard.led("SCREEN_5", "on") end
  deckard.led("SCREEN_6", "dim")
  deckard.led("SCREEN_8", "dim")
end

function ask.draw(screen, theme)
  local q = question()
  if not q then return end
  local specs = { [6] = { text = q.revert and "REVERT" or "DON'T SAVE" }, [8] = { text = "CANCEL" } }
  if not q.revert then specs[5] = { text = "SAVE", selected = true } end
  widgets.buttons(screen, theme, specs)
  screen:label(10, 106, 460, 60, q.question, { font = "roboto-bold", size = 22, color = "text" }, "center", "center")
  local lost = q.revert and "This can't be undone." or "Your changes will be lost if you don't save them."
  screen:label(490, 116, 460, 40, lost, { size = 18, color = "weak" }, "center", "center")
end

return ask
