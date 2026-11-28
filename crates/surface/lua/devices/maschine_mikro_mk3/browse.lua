-- Maschine Mikro MK3 library browse module (BROWSE toggle, encoder scroll/prehear/load, STAR favorites).
-- Clean-room Deckard Lua script.

local B = {}

B.KINDS = { "PROJECTS", "GROUPS", "SOUNDS", "INSTRUMENTS", "EFFECTS", "LOOPS", "ONE-SHOTS" }
B.FIELDS = { "KIND", "PRODUCT", "TYPE", "SUB-TYPE", "CHAR", "RESULTS" }

local function field_available(b, f)
  if f == 1 or f == 6 then return true end
  if not b then return false end
  if f == 2 then
    return b.products ~= nil and #b.products > 0
  end
  if f >= 3 and f <= 5 then
    local a = b.attributes and b.attributes[f - 2]
    return a ~= nil and a.values ~= nil and #a.values > 0
  end
  return false
end

local function next_field(b, cur, dir)
  local f = cur + dir
  while f >= 1 and f <= 6 do
    if field_available(b, f) then return f end
    f = f + dir
  end
  return cur
end

local function ensure_valid_field(st, b)
  if not field_available(b, st.browse_field) then
    st.browse_field = 6
  end
end

function B.field_value(b, f)
  if not b then return "-" end
  if f == 1 then
    return B.KINDS[b.kind] or "ALL"
  elseif f == 2 then
    return (b.product and b.products and b.products[b.product]) or "ALL"
  elseif f >= 3 and f <= 5 then
    local a = b.attributes and b.attributes[f - 2]
    if a and a.chosen and a.values and a.values[a.chosen] then
      return a.values[a.chosen]
    end
    return "ALL"
  else
    local n = b.results and #b.results or 0
    if n == 0 then return "(0)" end
    return string.format("%d/%d", b.focus or 1, n)
  end
end

function B.button(st, canon, down)
  if canon == "BROWSE" then
    if not down then return true end
    if st.shift() then
      st.browse_prehear = not st.browse_prehear
      st.popup("PREHEAR", st.browse_prehear and "ON" or "OFF")
    else
      st.browse = not st.browse
      deckard.library(st.browse)
      st.clear_quick_param()
      if st.browse then
        st.clear_overlays()
        st.browse_field = 6
      end
    end
    return true
  end

  if not st.browse then return false end

  if canon == "STAR" then
    if not down then return true end
    if st.shift() then
      deckard.browse_set_favorite()
      st.popup("FAVORITE", "TOGGLED")
    else
      deckard.browse_favorites()
    end
    return true
  end

  if (canon == "LEFT" or canon == "RIGHT") and st.overlay == nil then
    if not down then return true end
    local b = deckard.state().browser
    ensure_valid_field(st, b)
    st.browse_field = next_field(b, st.browse_field, canon == "RIGHT" and 1 or -1)
    return true
  end

  return false
end

function B.encoder(st, steps)
  if not st.browse or steps == 0 then return false end
  local b = deckard.state().browser
  if not b then return true end
  ensure_valid_field(st, b)

  local f = st.browse_field
  if f == 1 then
    local to = st.clamp((b.kind or 1) + steps, 1, #B.KINDS)
    deckard.browse_kind(to)
  elseif f == 2 then
    local cur = b.product or 0
    local to = st.clamp(cur + steps, 0, #(b.products or {}))
    deckard.browse_product(to > 0 and to or nil)
  elseif f >= 3 and f <= 5 then
    local idx = f - 2
    local a = b.attributes and b.attributes[idx]
    if a and a.values and #a.values > 0 then
      local cur = a.chosen or 0
      local to = st.clamp(cur + steps, 0, #a.values)
      deckard.browse_attribute(idx, to > 0 and to or nil)
    end
  else
    local results = b.results or {}
    if #results > 0 then
      local by = steps * (st.shift() and 7 or 1)
      deckard.browse_focus(by)
      if st.browse_prehear and b.kind ~= 1 then
        deckard.browse_prehear()
      end
    end
  end
  return true
end

function B.encoder_press(st)
  if not st.browse then return false end
  if st.browse_field == 6 then
    local b = deckard.state().browser
    if b and b.results and #b.results > 0 then
      deckard.browse_load()
      st.popup("LOADED")
    end
  else
    st.browse_field = 6
  end
  return true
end

function B.leds(st, s)
  deckard.led("BROWSE", st.browse and "on" or "dim")
  if st.browse then
    local b = s.browser
    deckard.led("STAR", (b and b.favorites) and "on" or "dim")
    if st.overlay == nil then
      local prev = next_field(b, st.browse_field, -1) ~= st.browse_field
      local nxt = next_field(b, st.browse_field, 1) ~= st.browse_field
      deckard.led("LEFT", prev and "dim" or "off")
      deckard.led("RIGHT", nxt and "dim" or "off")
    end
  else
    deckard.led("STAR", "off")
  end
end

return B
