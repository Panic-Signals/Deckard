-- Zhora on the Control page, when it is the focused slot:
--
--   display 1-4       its pages (OUTS, ENV, LEVELS, MOD), the one shown lit
--   display 5-8       TUNE, CHECK, LIVE, HOLD, lit while each is on
--   left screen       its name, and what it listens on
--   right screen      the bar (when it was tuned, or a message), the note heard, its
--                     cents, and a needle over -50..+50 cents, green within 5

local widgets = require("lib.widgets")

local zhora = {}

local ACTIONS = { "tune", "check", "live", "hold" }

local GREEN = { 0x3d, 0xd6, 0x6a }
local AMBER = { 0xff, 0xc2, 0x3a }
local FAINT = { 0x8a, 0x8a, 0x90 }

-- Display button `n` pressed.
function zhora.button(c, n)
  if n <= 4 then
    if c.pages[n] then deckard.plugin_page(n) end
  else
    deckard.zhora(ACTIONS[n - 4])
  end
end

function zhora.leds(c)
  for i = 1, 4 do
    deckard.led("SCREEN_" .. i, i == c.page and "on" or (c.pages[i] and "dim" or "off"))
  end
  for i, what in ipairs(ACTIONS) do
    deckard.led("SCREEN_" .. (i + 4), c.zhora[what] and "on" or "dim")
  end
end

function zhora.buttons(screen, theme, c)
  local specs = {}
  for i = 1, 4 do
    if c.pages[i] then specs[i] = { text = c.pages[i]:upper(), selected = i == c.page } end
  end
  for i, what in ipairs(ACTIONS) do
    specs[i + 4] = { text = what:upper(), selected = c.zhora[what] }
  end
  widgets.buttons(screen, theme, specs)
end

-- The left screen's middle: the name, and the input.
function zhora.left(screen, c)
  screen:label(0, 92, 480, 40, "Zhora", { size = 30 })
  screen:label(0, 136, 480, 22, c.zhora.input, { size = 15, color = FAINT })
end

-- "+2¢", "−20¢".
local function cents_text(cents)
  local n = math.floor(math.abs(cents) + 0.5)
  if n == 0 then return "0¢" end
  return (cents < 0 and "−" or "+") .. n .. "¢"
end

-- The right screen: the bar, then the tuner.
function zhora.right(screen, z)
  screen:rect(480, 35, 480, 25, "bar")
  if z.message then
    screen:label(490, 36, 460, 24, z.message, { size = 14, color = AMBER }, "left", "center")
  else
    local st = { size = 14 }
    local rw = z.right ~= "" and screen:text_width(z.right, st) or 0
    if rw > 0 then screen:label(950 - rw, 36, rw, 24, z.right, st, "right", "center") end
    screen:label(490, 36, 460 - rw - (rw > 0 and 12 or 0), 24, z.status, st, "left", "center")
  end
  local ink = z.in_tune and GREEN or "text"
  screen:label(480, 64, 480, 58, z.note or "–", { size = 52, color = z.note and ink or FAINT })
  if z.cents then screen:label(480, 122, 480, 22, cents_text(z.cents), { size = 16, color = ink }) end
  -- The needle's scale: -50 to +50 cents, the green zone ±5 around the middle.
  local x0, w, y = 520, 400, 152
  local function at(cents) return x0 + (math.max(-50, math.min(50, cents)) + 50) / 100 * w end
  screen:rect(math.floor(at(-5)), y, math.floor(at(5) - at(-5)), 40, { 0x1a, 0x4a, 0x2a })
  screen:line(x0, y + 20, x0 + w, y + 20, 1, { 0x48, 0x48, 0x4e })
  for _, t in ipairs({ -50, -25, 0, 25, 50 }) do
    local h = t == 0 and 14 or 8
    screen:line(at(t), y + 20 - h / 2, at(t), y + 20 + h / 2, 1, FAINT)
  end
  if z.cents then
    local x = at(z.cents)
    screen:line(x, y - 2, x, y + 42, 3, z.in_tune and GREEN or AMBER)
  end
end

return zhora
