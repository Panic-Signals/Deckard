-- deckard Lua host prelude.
--
-- Every engine-provided global (NI, App, NHLController, ...) is a "proxy": a table
-- tagged with a type name. Looking up a key on a proxy first tries the Rust
-- implementation registered for that type (IMPL[type][key]); anything missing becomes
-- a recorded stub, so scripts keep running and the host learns which API to build next.
--
-- Stub conventions:
--   * ALL_CAPS keys on a namespace are constants. Values come from the app's own
--     registrations (constants.tsv, see `enum-values`); unknown ones get stable
--     made-up integers (recorded as "const"), numbered within their family
--     (BUTTON_1, BUTTON_2, ...) because scripts do arithmetic on them.
--   * Calling a stub records the call and returns a value guessed from the name:
--     is*/getIs*/has*/can* -> false, size/count -> 0, *Name/*String/*Text -> "", set*/on* -> nil,
--     otherwise a new proxy.

local host = ...
local record, IMPL, namespaces, known = host.record, host.impl, host.namespaces, host.known
local rawget, rawset, setmetatable, getmetatable, type, tostring =
      rawget, rawset, setmetatable, getmetatable, type, tostring

local Proxy = {}
local constants, families, next_family, next_unique = {}, {}, 0, 1000000
local children = setmetatable({}, { __mode = "k" })

local function is_proxy(v) return type(v) == "table" and getmetatable(v) == Proxy end

local function new_proxy(ty, fields)
    local p = fields or {}
    rawset(p, "__type", ty)
    return setmetatable(p, Proxy)
end

local function known_value(path, key)
    if path:match("^NI%.HW") then return known.hw[key] end
    if path:match("^NI%.HOSTING") then return known.hosting[key] or known.data[key] end
    if path:match("^NI%.NSA") then return known.nsa[key] or known.data[key] end
    return known.data[key]
end

local function constant(path, key)
    local full = path .. "." .. key
    local v = constants[full]
    if v then return v end
    v = known_value(path, key)
    if v then
        constants[full] = v
        return v
    end
    local prefix, n = key:match("^(.-)(%d+)$")
    if prefix then
        local fam = path .. "." .. prefix
        if not families[fam] then
            next_family = next_family + 1
            families[fam] = next_family * 1000
        end
        v = families[fam] + tonumber(n)
    else
        next_unique = next_unique + 1
        v = next_unique
    end
    constants[full] = v
    record(full, "const")
    return v
end

local function stub_result(ty, key)
    if key:match("Parameter$") or key:match("Param$") then
        return new_proxy(ty .. "." .. key .. "()")
    elseif key:match("^is%u") or key:match("^getIs%u") or key:match("^has%u") or key:match("^can%u") or key:match("^was%u")
        or key:match("^should%u") or key:match("^contains") then
        return false
    elseif key == "size" or key == "count" or key:match("^getNum%u") and not key:match("^getNumerator") or key:match("^num%u")
        or key:match("Count$") or key:match("Size$") or key:match("^getSize") then
        return 0
    elseif key:match("Name$") or key:match("String$") or key:match("Text$") or key:match("Label$")
        or key:match("Path$") or key:match("^getUnit") then
        return ""
    elseif key:match("^set%u") or key:match("^on%u") or key:match("^remove") or key:match("^clear")
        or key:match("^reset") or key:match("^update") or key:match("^trigger") then
        return nil
    end
    return new_proxy(ty .. "." .. key .. "()")
end

local function stub_function(ty, key)
    return function(...)
        record(ty .. "." .. key, "call")
        return stub_result(ty, key)
    end
end

Proxy.__index = function(self, key)
    if type(key) ~= "string" then return nil end -- keeps ipairs/# from looping forever
    local ty = rawget(self, "__type")
    local impl = IMPL[ty]
    if impl then
        local f = impl[key]
        if f ~= nil then return f end
    end
    -- Namespaces (NI, NI.HW, NI.DATA.StateHelper, ...) are known from the scripts
    -- themselves: any NI path that something else is looked up under.
    local full = ty .. "." .. key
    if namespaces[full] then
        local sub = new_proxy(full)
        rawset(self, key, sub)
        return sub
    end
    -- Constants: capitals, digits and underscores (a lone small letter allowed, as in
    -- SKIP_NOT_AUv2_PLUGINS).
    local capitals = key:match("^[%u%d_]+$") or (key:match("^%u[%w_]*$") and key:find("_") and not key:find("%l%l"))
    if ty:match("^NI%.") and capitals and key:match("%u") then return constant(ty, key) end
    -- On engine objects, capitalised keys are fields scripts attach themselves
    -- (Button.Color = ...); unset ones read as nil, as on the app's userdata.
    if not ty:match("^NI") and key:match("^%u") then
        record(ty .. "." .. key, "field")
        return nil
    end
    record(ty .. "." .. key, "index")
    local f = stub_function(ty, key)
    rawset(self, key, f)
    return f
end

Proxy.__call = function(self, ...)
    local ty = rawget(self, "__type")
    record(ty, "call")
    return new_proxy(ty .. "()")
end

Proxy.__tostring = function(self) return "<" .. rawget(self, "__type") .. ">" end
Proxy.__concat = function(a, b) return tostring(a) .. tostring(b) end
Proxy.__len = function() return 0 end
Proxy.__lt = function() return false end
Proxy.__le = function() return false end
for _, op in ipairs { "__add", "__sub", "__mul", "__div", "__mod", "__unm", "__idiv", "__pow",
                      "__band", "__bor", "__bxor", "__shl", "__shr", "__bnot" } do
    Proxy[op] = function() return 0 end
end

host.new_proxy = new_proxy
host.is_proxy = is_proxy
host.constants = constants

-- std::string::npos / size_t(-1), which index lookups return for "not found".
NPOS = -1

NI = new_proxy("NI")
App = new_proxy("App")
NHLController = new_proxy("NHLController")

-- Engine types the scripts construct directly (sol2-bound C++ containers and helpers).
for _, name in ipairs { "TickFloatMap", "VectorString", "EventSet", "STLHelper" } do
    _G[name] = new_proxy(name)
end

-- A std::vector as sol2 binds it (0-based `at`), for the containers the scripts fill or
-- the host returns (note events).
local Vector = {}
Vector.__index = Vector
function Vector:push_back(v) self.items[#self.items + 1] = v end
function Vector:size() return #self.items end
function Vector:empty() return #self.items == 0 end
function Vector:at(i) return self.items[i + 1] end
function Vector:clear() self.items = {} end
function host.vector(items) return setmetatable({ items = items or {} }, Vector) end
VectorInt = function() return host.vector() end

-- A note event the host hands out (`getSoundNoteEvents`, `getNoteEventByNote`).
local NoteEvent = {}
NoteEvent.__index = NoteEvent
function NoteEvent:getNote() return self.pitch end
function NoteEvent:getLength() return self.length end
function NoteEvent:getVelocity() return self.velocity end
function NoteEvent:getTick() return self.position end
function NoteEvent:getPosition() return self.position end
function NoteEvent:getProbability() return self.probability end
function host.note_event(t) return setmetatable(t, NoteEvent) end

-- The app's "any pitch" note (Maschine's CONST_INVALID_NOTE).
CONST_INVALID_NOTE = 128

-- Reads of undefined globals are recorded (scripts often probe optional ones) and
-- return nil as in plain Lua.
setmetatable(_G, {
    __index = function(_, key)
        if type(key) == "string" then record("global " .. key, "index") end
        return nil
    end,
})
