-- Browse (BROWSE): the library. The products as tiles on the left screen, the chosen
-- product's name and the results on the right, what each knob chooses along the
-- bottom. See docs/controller-browse-page.md.
--
--   display 1 / 2     the previous / next content kind (Projects … One-shots)
--   display 3         LOCATE
--   display 4         user content, or factory
--   display 5 / 6     the previous / next result, preheard (samples) or loaded
--   display 7         favourites only; SHIFT: the focused result a favourite (or not)
--   display 8         load the focused result; SHIFT: prehear on or off (samples)
--   SHIFT + 5 / 6     the previous / next plug-in slot (where an instrument loads)
--   knobs 1-8         category, product, bank, sub-bank, type, sub-type, character,
--                     the focused result (SHIFT: twenty at a time)
--   jog wheel         turns the bracketed knob's choice; pressed, loads the focused
--                     result (SHIFT: a favourite, or not)
--   jog wheel left / right
--                     bracket the previous / next knob that has something to choose
--   jog wheel up / down, LEFT / RIGHT
--                     the previous / next six products
--
-- The pads play as on every page. DECKARD's library has no categories or banks yet:
-- knobs 1, 3 and 4 have nothing to choose.

local widgets = require("lib.widgets")
local knobs = require("lib.knobs")
local wheel = require("lib.wheel")
local util = require("lib.util")

local browse = {}

local KINDS = { "PROJECTS", "GROUPS", "SOUNDS", "INSTRUMENTS", "EFFECTS", "LOOPS", "ONE-SHOTS" }
local LOOPS, ONE_SHOTS = 6, 7

-- The knob the jog wheel turns (bracketed): results to begin with, and kept between
-- visits.
local RESULTS = 8
local focus = RESULTS

-- Prehear: the focused sample plays as it's focused.
local prehear = false

-- The products shown, six a page, while none is chosen: the first page's index (0..).
local PRODUCTS_SHOWN = 6
local product_page = 0

-- SHIFT + knob 8 jumps this many results.
local JUMP = 20

local function lib(s)
  return s.browser
end

local function can_prehear(b)
  return prehear and (b.kind == LOOPS or b.kind == ONE_SHOTS) and #b.results > 0
end

-- How many choices knob `n` has.
local function count(b, n)
  if n == 2 then return #b.products end
  if n >= 5 and n <= 7 then return #b.attributes[n - 4].values end
  if n == RESULTS then return #b.results end
  return 0
end

local function pages(b)
  return math.max(1, (#b.products + PRODUCTS_SHOWN - 1) // PRODUCTS_SHOWN)
end

-- The page of products that shows the chosen one (else the page scrolled to).
local function shown_page(b)
  if b.product then return (b.product - 1) // PRODUCTS_SHOWN end
  return math.min(product_page, pages(b) - 1)
end

-- Knob `n`'s choice `steps` on.
local function step(b, n, steps)
  if n == 2 then
    local p = b.product
    if not p then
      -- (From all, turning on chooses the first product shown.)
      if steps <= 0 or #b.products == 0 then return end
      p = math.min(#b.products, shown_page(b) * PRODUCTS_SHOWN + 1)
    else
      p = math.min(#b.products, p + steps)
      if p < 1 then p = nil end
    end
    if p ~= b.product then deckard.browse_product(p) end
  elseif n >= 5 and n <= 7 then
    local a = b.attributes[n - 4]
    if #a.values == 0 then return end
    local c = util.clamp((a.chosen or 0) + steps, 0, #a.values)
    if c ~= (a.chosen or 0) then deckard.browse_attribute(n - 4, c > 0 and c or nil) end
  elseif n == RESULTS then
    if #b.results == 0 then return end
    local by = deckard.held("SHIFT") and JUMP or 1
    local to = util.clamp(b.focus + steps * by, 1, #b.results)
    if to ~= b.focus then deckard.browse_focus(to - b.focus) end
    if (to ~= b.focus or #b.results == 1) and can_prehear(b) then deckard.browse_prehear() end
  end
end

-- The next knob left (-1) or right (1) of the bracketed one that has choices; results
-- always have.
local function next_focus(b, dir)
  local n = focus + dir
  while n >= 1 and n < RESULTS and count(b, n) == 0 do n = n + dir end
  if n < 1 then return focus end
  return math.min(n, RESULTS)
end

-- The previous (-1) or next six products: the page scrolled while none is chosen,
-- else the chosen one moved by six (back past the first: all of them).
local function page_products(b, dir)
  if not b.product then
    product_page = util.clamp(shown_page(b) + dir, 0, pages(b) - 1)
    return
  end
  local p = math.min(#b.products, b.product + dir * PRODUCTS_SHOWN)
  deckard.browse_product(p >= 1 and p or nil)
end

-- The display buttons' state: what each would do now.
local function buttons(s, shift)
  local b = lib(s)
  local n = #b.results
  if shift then
    local c = s.control
    local sample = b.kind == LOOPS or b.kind == ONE_SHOTS
    return {
      -- (Shown, but does nothing: its light is off.)
      [4] = { text = "USER", selected = b.user, inert = true },
      [5] = {
        arrows = true,
        text = c.slot and c.slot:upper() or "",
        left = c.slots > 0 and (c.slot_index or 1) > 1,
        right = c.slot_index ~= nil and c.slot_index < c.slots,
      },
      [7] = { text = "SET", star = true, inert = n == 0 },
      [8] = sample and { text = "PREHEAR", selected = prehear } or nil,
    }
  end
  return {
    [1] = { arrows = true, text = KINDS[b.kind], left = b.kind > 1, right = b.kind < #KINDS },
    [3] = { text = "LOCATE" },
    [4] = { text = "USER", selected = b.user },
    [5] = { text = "PREVIOUS", disabled = n == 0 or b.focus <= 1 },
    [6] = { text = "NEXT", disabled = n == 0 or b.focus >= n },
    [7] = { star = true, selected = b.favorites },
    [8] = { text = "LOAD", disabled = n == 0 },
  }
end

-- Focus the result `by` on, then prehear it (samples) or load it.
local function step_and_load(b, by)
  deckard.browse_focus(by)
  if can_prehear(b) then deckard.browse_prehear() else deckard.browse_load() end
end

function browse.button(name, down)
  if not down then return end
  local s = deckard.state()
  local b = lib(s)
  local shift = deckard.held("SHIFT")
  local n = tonumber(name:match("^SCREEN_(%d)$"))
  if n then
    if b.busy then return true end
    local spec = buttons(s, shift)
    if shift then
      if n == 5 or n == 6 then
        local arrows = spec[5]
        if (n == 5 and arrows.left) or (n == 6 and arrows.right) then deckard.slot(n == 5 and -1 or 1) end
      elseif n == 7 and #b.results > 0 then
        deckard.browse_set_favorite()
      elseif n == 8 and spec[8] then
        prehear = not prehear
      end
    elseif n == 1 and b.kind > 1 then
      deckard.browse_kind(b.kind - 1)
    elseif n == 2 and b.kind < #KINDS then
      deckard.browse_kind(b.kind + 1)
    elseif n == 4 then
      deckard.browse_user(not b.user)
    elseif n == 5 and not spec[5].disabled then
      step_and_load(b, -1)
    elseif n == 6 and not spec[6].disabled then
      step_and_load(b, 1)
    elseif n == 7 then
      deckard.browse_favorites()
    elseif n == 8 and #b.results > 0 then
      deckard.browse_load()
    end
    -- (LOCATE: DECKARD can't yet find what made the focused sound.)
    return true
  end
  if name == "WHEEL" then
    if focus == RESULTS and #b.results > 0 then
      if shift then deckard.browse_set_favorite() else deckard.browse_load() end
    end
  elseif name == "WHEEL_LEFT" then
    focus = next_focus(b, -1)
  elseif name == "WHEEL_RIGHT" then
    focus = next_focus(b, 1)
  elseif name == "WHEEL_UP" or name == "LEFT" then
    page_products(b, -1)
  elseif name == "WHEEL_DOWN" or name == "RIGHT" then
    page_products(b, 1)
  else
    return
  end
  return true
end

function browse.knob(n, amount)
  local steps = knobs.gather(n, amount)
  if steps ~= 0 then step(lib(deckard.state()), n, steps) end
  return true
end

function browse.wheel(steps)
  -- (TEMPO, SWING and VOLUME keep the wheel.)
  if wheel.mode then return end
  step(lib(deckard.state()), focus, steps)
  return true
end

-- A display button's light: off when it does nothing, lit when chosen or held.
local function button_led(i, spec)
  local name = "SCREEN_" .. i
  if not spec or spec.disabled or spec.inert then
    deckard.led(name, "off")
  elseif spec.selected or deckard.held(name) then
    deckard.led(name, "on")
  else
    deckard.led(name, "dim")
  end
end

function browse.leds()
  local s = deckard.state()
  local b = lib(s)
  local spec = buttons(s, deckard.held("SHIFT"))
  for i = 1, 8 do
    local own = spec[i]
    if spec[i - 1] and spec[i - 1].arrows then
      own = spec[i - 1].right and {} or nil
    elseif own and own.arrows then
      own = own.left and {} or nil
    end
    button_led(i, own)
  end
  deckard.led("BROWSE", "on")
  -- (ERASE would delete user content: not yet.)
  if b.user and not deckard.held("SHIFT") then deckard.led("ERASE", deckard.held("ERASE") and "on" or "dim") end
  local first, last = shown_page(b) == 0, shown_page(b) >= pages(b) - 1
  local up = b.product ~= nil or not first
  local down = #b.products > 0 and b.product ~= #b.products and (b.product ~= nil or not last)
  deckard.led("LEFT", up and "dim" or "off")
  deckard.led("RIGHT", down and "dim" or "off")
  if up then deckard.led("WHEEL_UP", 16, false) end
  if down then deckard.led("WHEEL_DOWN", 16, false) end
  if next_focus(b, -1) ~= focus then deckard.led("WHEEL_LEFT", 16, false) end
  if focus < RESULTS then deckard.led("WHEEL_RIGHT", 16, false) end
end

-- The theme's style `name`, in another colour or size.
local function style(theme, name, colour, size)
  local st = {}
  for k, v in pairs(theme.styles.default) do st[k] = v end
  for k, v in pairs(theme.styles[name] or {}) do st[k] = v end
  if colour then st.color = colour end
  if size then st.size = size end
  return st
end

-- The products, three to a row, two rows: the chosen one filled.
local TILE_X, TILE_Y, TILE_W, TILE_H, TILE_GAP, ROW_GAP = 20, 41, 135, 90, 17, 6
local function products(screen, theme, b)
  local first = shown_page(b) * PRODUCTS_SHOWN
  for i = 1, PRODUCTS_SHOWN do
    local p = first + i
    local name = b.products[p]
    if not name then break end
    local x = TILE_X + (i - 1) % 3 * (TILE_W + TILE_GAP)
    local y = TILE_Y + (i - 1) // 3 * (TILE_H + ROW_GAP)
    local chosen = p == b.product
    if chosen then
      screen:rect(x, y, TILE_W - 1, TILE_H - 1, "text")
    else
      screen:frame(x, y, TILE_W + 1, TILE_H, "browse_tile")
    end
    screen:label(x + 6, y + 67, TILE_W - 12, 22, name, style(theme, "default", chosen and "on_colour" or "text"), "left")
  end
  -- The scrollbar, with more than a page of products.
  if #b.products > PRODUCTS_SHOWN then
    local h = 191
    local n = pages(b)
    screen:rect(475, 36 + h * shown_page(b) // n, 5, math.max(4, h // n), "text")
  end
end

-- The chosen product's name, and the results: seven rows, the focused one filled.
local ROW_X, ROW_Y, ROW_W, ROW_H, ROW_STEP, ROWS = 721, 44, 231, 21, 26, 7
local function results(screen, theme, b)
  local product = b.product and b.products[b.product]
  if product then
    screen:label(500, 36, 200, 196, product, style(theme, "browse_logo"))
  end
  local n = #b.results
  local top = util.clamp(b.focus - ROWS // 2, 1, math.max(1, n - ROWS + 1))
  for row = 1, ROWS do
    local i = top + row - 1
    local r = b.results[i]
    if not r then break end
    local y = ROW_Y + (row - 1) * ROW_STEP
    local focused = i == b.focus
    screen:rect(ROW_X, y, ROW_W - 5, ROW_H, focused and "text" or "browse_row", theme.corner)
    local text = focused and "on_colour" or "text"
    local w = ROW_W - 5 - 14 - (r.favorite and 18 or 0)
    screen:label(ROW_X + 9, y, w, ROW_H, r.name, style(theme, "default", text), "left")
    if r.favorite then widgets.star(screen, ROW_X + ROW_W - 5 - 13, y + ROW_H / 2, 6, text) end
  end
  if n > ROWS then
    local h = 177
    screen:rect(955, ROW_Y + h * (top - 1) // n, 5, math.max(4, h * ROWS // n), "text")
  end
end

-- What a knob's attribute reads with none of its values chosen.
local function all_caption(b, n)
  if n == 5 then return "All Types" end
  if n == 6 then return "All Sub-Types" end
  if b.kind >= 4 then return "All Characters" end
  return "All Sub-Types"
end

-- Under each knob, what it has chosen: the bracketed one's framed.
local function params(screen, theme, b)
  for n = 1, 8 do
    local text, colour = "", (n % 4 == 0) and "browse_param_all" or "browse_param_none"
    if n == 2 and #b.products > 0 then
      text = b.product and b.products[b.product] or "All Products"
      colour = b.product and "text" or "browse_param_all"
    elseif n >= 5 and n <= 7 and count(b, n) > 0 then
      local a = b.attributes[n - 4]
      text = a.chosen and a.values[a.chosen] or all_caption(b, n)
      colour = a.chosen and "text" or "browse_param_all"
    elseif n == RESULTS then
      local r = #b.results
      if b.favorites then
        text = r == 0 and "No ★" or r .. " ★"
      else
        text = r == 0 and "No Results" or (r == 1 and "1 Result" or r .. " Results")
      end
      colour = "text"
    end
    local x = (n - 1) * 120
    if n == focus then
      colour = "text"
      -- Brackets: [ … ].
      screen:rect(x + 3, 240, 1, 26, "browse_bracket")
      screen:rect(x + 3, 240, 3, 1, "browse_bracket")
      screen:rect(x + 3, 265, 3, 1, "browse_bracket")
      screen:rect(x + 116, 240, 1, 26, "browse_bracket")
      screen:rect(x + 114, 240, 3, 1, "browse_bracket")
      screen:rect(x + 114, 265, 3, 1, "browse_bracket")
    end
    screen:label(x + 6, 234, 108, 38, text, style(theme, "default", colour))
  end
end

function browse.draw(screen, theme)
  local s = deckard.state()
  local b = lib(s)
  widgets.buttons(screen, theme, buttons(s, deckard.held("SHIFT")))
  products(screen, theme, b)
  results(screen, theme, b)
  params(screen, theme, b)
end

return browse
