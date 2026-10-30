-- DECKARD's controller: what works on every page, as with Maschine's own scripts.
-- The page on screen (pages/*.lua) gets each event first; what it doesn't handle
-- (returns nothing) comes here.
--
-- Transport, groups, the pad modes, the jog wheel, and their lights. See
-- docs/controller-api.md for the `deckard` API.

local util = require("lib.util")
local pads = require("lib.pads")
local wheel = require("lib.wheel")
local pin = require("lib.pin")
local quick_edit = require("lib.quick_edit")
local back = require("lib.back")
local strip = require("lib.strip")

local global = {}

-- AUTO: a press turns Auto-write on, the next off, with SHIFT or not (CONTROLLER-BUGS
-- C38: DECKARD's; Maschine's `onAutoWriteButton` writes while held, SHIFT locking it).

-- TAP: the times of the last taps; a pause longer than this starts a new count.
local taps = {}
local TAP_RESET = 2.0

-- Buttons that show a page while held (MUTE, SOLO and SELECT one page, PATTERN,
-- SCENE, EVENTS and NOTE REPEAT their own; FOLLOW the Grid page, with SHIFT);
-- lib/pin.lua keeps the one shown and the pins.
local HOLD = {
  MUTE = "hold", SOLO = "hold", SELECT = "hold", PATTERN = "pattern", SCENE = "scene", FOLLOW = "grid", EVENTS = "events",
  NOTE_REPEAT = "note_repeat",
}

-- MIX, ARRANGE and BROWSE show their page; pressed again, the page they were pressed
-- on.
local MODE_BUTTONS = { MIX = "mixer", ARRANGE = "arrange", BROWSE = "browse" }
-- SHIFT + SELECT and SHIFT + SOLO show the LFOs and the mod matrix the same way (the
-- button's release is theirs too, not its held page's).
local SHIFT_PAGES = { SELECT = "lfo", SOLO = "mod_matrix" }
local shift_pressed = {}
local mode_back = nil

local function mode_page(page)
  for _, list in ipairs({ MODE_BUTTONS, SHIFT_PAGES }) do
    for _, p in pairs(list) do
      if p == page then return true end
    end
  end
  return false
end

-- Show `page`, or, shown already, the page it was shown on.
local function toggle_page(page)
  if deckard.page() == page then
    deckard.show_page(mode_back or "control")
  else
    if not mode_page(deckard.page()) then mode_back = deckard.page() end
    deckard.show_page(page)
  end
end

-- The page each pad mode shows.

local function tap()
  local now = deckard.now()
  if #taps > 0 and now - taps[#taps] > TAP_RESET then taps = {} end
  taps[#taps + 1] = now
  if #taps > 5 then table.remove(taps, 1) end
  if #taps >= 2 then
    local beat = (taps[#taps] - taps[1]) / (#taps - 1)
    if beat > 0 then deckard.tempo(util.round(60 / beat, 2)) end
  end
end

local function toggle_wheel(mode)
  if wheel.mode == mode then wheel.mode = nil else wheel.mode = mode end
end

local pad_mode = pads.switch

function global.button(name, down)
  local s = deckard.state()
  local shift = deckard.held("SHIFT")

  -- Another page's button: a pinned page gives way first.
  if down and pin.shown and not deckard.held(pin.shown.button) and (MODE_BUTTONS[name] or name == "PAD_MODE"
      or name == "KEYBOARD" or name == "CHORDS" or name == "STEP" or name == "CHANNEL" or name == "PLUGIN"
      or name == "MACRO" or name == "FILE" or name == "SETTINGS" or name == "SAMPLE" or (name == "LOCK" and shift)
      or (name == "PERFORM" and shift) or (SHIFT_PAGES[name] and shift)) then
    deckard.show_page(pin.shown.back)
    pin.shown = nil
  end

  -- SHIFT + SELECT, SHIFT + SOLO: before their held page.
  if SHIFT_PAGES[name] and ((down and shift) or (not down and shift_pressed[name])) then
    shift_pressed[name] = down or nil
    if down then toggle_page(SHIFT_PAGES[name]) end
    return true
  end

  -- (FOLLOW alone is Follow, below; it closes the Grid page it showed.)
  local held = HOLD[name] and (name ~= "FOLLOW" or shift or (pin.shown and pin.shown.button == name))
  if held then
    local shown = pin.shown
    -- NOTE REPEAT leaves Step mode for Pad mode; with SHIFT, the pads go to Keyboard
    -- mode (the Arp), as Maschine's.
    if down and name == "NOTE_REPEAT" then
      local was = deckard.page()
      if pads.mode == "step" then pads.leave_step() end
      if shift and not pads.keyboard() then
        pad_mode("keyboard")
        if shown and shown.button == name then
          -- (Shown already: it stays, Keyboard mode under it.)
          shown.back = deckard.page()
          deckard.show_page(was)
          return true
        end
      end
    end
    if down then
      if shown and shown.button == name then
        -- (Pinned: pressed again, it closes as it's let go.)
        shown.close = true
      elseif not shown or not deckard.held(shown.button) then
        -- (Another one pinned gives way.)
        local back = shown and shown.back or deckard.page()
        pin.shown = { button = name, back = back }
        if HOLD[name] == "events" or HOLD[name] == "note_repeat" then require("pages." .. HOLD[name]).shown() end
        deckard.show_page(HOLD[name])
      end
    elseif shown and shown.button == name and (shown.close or not pin.pinned[name]) then
      deckard.show_page(shown.back)
      pin.shown = nil
    end
    return true
  end

  -- Display button 1 on a held page: its pin. Taken off with the button up, the page
  -- closes.
  if name == "SCREEN_1" and pin.shown and deckard.page() == HOLD[pin.shown.button] then
    if down then
      local b = pin.shown.button
      pin.pinned[b] = not pin.pinned[b]
      if not pin.pinned[b] and not deckard.held(b) then
        deckard.show_page(pin.shown.back)
        pin.shown = nil
      end
    end
    return true
  end

  if name == "AUTO" then
    if down then deckard.auto_write(not s.auto_write) end
    return true
  end
  if strip.button(name, down) then return true end
  if not down then return end

  if name == "PLAY" then
    if s.playing then deckard.stop() else deckard.play() end
  elseif name == "STOP" then
    deckard.stop()
  elseif name == "REC" then
    deckard.record(not s.recording, shift and not s.recording)
  elseif name == "RESTART" then
    if shift then deckard.set_loop(not s.looping) else deckard.restart() end
  elseif name == "TAP" then
    if shift then deckard.metronome(not s.metronome) else tap() end
  elseif name == "FOLLOW" then
    deckard.follow(not s.follow)
  elseif name == "FIXED_VEL" then
    deckard.fixed_velocity(not s.fixed_velocity)
  elseif name == "PAD_MODE" then
    pad_mode("pad")
  elseif name == "KEYBOARD" then
    pad_mode("keyboard")
  elseif name == "CHORDS" then
    pad_mode("chords")
  elseif name == "STEP" then
    pad_mode("step")
  elseif name == "CHANNEL" then
    deckard.show_plugins(false)
    deckard.show_page("control")
  elseif name == "PLUGIN" then
    deckard.show_plugins(true)
    deckard.show_page("control")
  elseif name == "FILE" then
    -- (The File page, or back from it and its Save page.)
    local page = deckard.page()
    if page == "file" or page == "save" then back.go() else back.over("file") end
  elseif name == "VARIATION" and not shift then
    -- (The focused sound's Groove ▸ Variation, on the Control page.)
    pin.shown = nil
    deckard.show_page("control")
    deckard.groove_page(2)
  elseif name == "LOCK" then
    -- (SHIFT + LOCK: Extended Lock, and LOCK on it back; else take or recall the
    -- focused snapshot.)
    if deckard.page() == "lock" then
      back.go()
    elseif shift then
      back.over("lock")
    else
      deckard.lock("button")
    end
  elseif name == "SAMPLE" then
    -- (The Sampling page, or back from it.)
    if deckard.page() == "sampling" then
      deckard.sampling(false)
      back.go()
    else
      deckard.sampling(true)
      back.over("sampling")
    end
  elseif name == "SETTINGS" then
    -- (The Settings page, or back from it.)
    if deckard.page() == "settings" then back.go() else back.over("settings") end
  elseif name == "MACRO" and deckard.page() == "macro_set" then
    -- (MACRO, or SHIFT + MACRO, leaves Set Macro: back where it was.)
    back.go()
  elseif name == "MACRO" and shift then
    -- (Set Macro.)
    require("pages.macro_set").shown()
    back.over("macro_set")
  elseif name == "MACRO" then
    -- (Channel ▸ Macro, as Maschine's Macro page.)
    deckard.show_plugins(false)
    deckard.show_page("control")
    deckard.page_group(4)
  elseif MODE_BUTTONS[name] then
    toggle_page(MODE_BUTTONS[name])
  elseif name == "TEMPO" then
    toggle_wheel("tempo")
  elseif name == "SWING" then
    toggle_wheel("swing")
  elseif name == "VOLUME" then
    toggle_wheel("volume")
  else
    local letter = name:match("^GROUP_(%u)$")
    if not letter then return end
    local i = string.byte(letter) - string.byte("A") + 1
    -- The group buttons are the focused group's bank of eight, then a new group in
    -- the place after the last; SHIFT (with eight groups or more, so banks to pick):
    -- bank `i`'s first group, or a new one in the bank after a full last one, as
    -- Maschine's (`PageMaschine:onGroupButtonPressed`).
    local bank = (s.group - 1) // 8
    -- SHIFT + ERASE: the bank's group deleted (the last one reset).
    if shift and deckard.held("ERASE") then
      local g = bank * 8 + i
      if g <= #s.groups then deckard.delete_group(g) end
      return true
    end
    if shift and #s.groups >= 8 then
      local first = (i - 1) * 8 + 1
      if first <= #s.groups then
        deckard.focus_group(first)
      elseif first == #s.groups + 1 then
        deckard.add_group()
      end
      return true
    end
    local g = bank * 8 + i
    if g == #s.groups + 1 then
      deckard.add_group()
      return true
    end
    if g > #s.groups then return true end
    if pin.active("MUTE") then
      deckard.mute_group(g)
    elseif pin.active("SOLO") then
      deckard.solo_group(g)
    else
      deckard.focus_group(g)
    end
  end
  return true
end

function global.pad(n, velocity)
  if pads.keys() then return true end
  local s = deckard.state()
  local group = s.groups[s.group]
  if not group or n > #group.sounds then return true end
  if pin.active("MUTE") then
    deckard.mute_sound(n)
  elseif pin.active("SOLO") then
    deckard.solo_sound(n)
  elseif deckard.held("ERASE") then
    deckard.erase_sound(n)
  elseif n ~= s.sound and (not pads.focus_pinned or pin.active("SELECT")) then
    deckard.focus_sound(n)
  end
  return true
end

-- SHIFT + a pad: the shortcut printed under it, before any page (as Maschine's
-- `onPadEventShift`): undo and redo, quantize, nudge, clear, copy and paste,
-- transpose. With ERASE held in Pad mode, the pad's sound removed (on the LFO page,
-- the page's ERASE + pad removes the LFO instead). Not while the
-- Events page selects (the pads select), nor on a page SHIFT + its button shows while
-- that button is still held (Grid, Set Macro).
local SHIFT_PADS = {
  { "edit", "undo" }, { "edit", "redo" }, { "edit", "undo_step" }, { "edit", "redo_step" },
  { "events", "quantize" }, { "events", "quantize_half" },
  { "events", "nudge_shown", -1 }, { "events", "nudge_shown", 1 },
  { "events", "clear" }, { "edit", "clear_modulation" },
  { "edit", "copy" }, { "edit", "paste" },
  { "events", "transpose", -1 }, { "events", "transpose", 1 },
  { "events", "transpose", -12 }, { "events", "transpose", 12 },
}

function global.shift_pad(n)
  local page = deckard.page()
  if page == "events" and require("pages.events").selecting() then return false end
  if (page == "grid" and deckard.held("FOLLOW")) or (page == "macro_set" and deckard.held("MACRO")) then
    return false
  end
  if deckard.held("ERASE") then
    if pads.mode ~= "pad" or page == "lfo" then return false end
    local s = deckard.state()
    local group = s.groups[s.group]
    if group and n <= #group.sounds then deckard.reset_sound(n) end
    return true
  end
  local what = SHIFT_PADS[n]
  if what[1] == "edit" then
    deckard.edit(what[2])
  else
    deckard.events(what[2], what[3] or 0)
  end
  return true
end

-- (While TEMPO, SWING or VOLUME is on, the wheel does that whatever page shows: this
-- gets it before the page.)
function global.wheel_mode(steps)
  if not wheel.mode then return false end
  local s = deckard.state()
  local fine = deckard.held("SHIFT")
  quick_edit.changed(wheel.mode)
  if wheel.mode == "tempo" then
    deckard.tempo(util.round(s.tempo + steps * (fine and 0.1 or 1), 1))
  elseif wheel.mode == "swing" then
    deckard.swing(util.round(s.swing + steps * (fine and 0.1 or 1), 1))
  elseif wheel.mode == "volume" then
    -- (In dB, as Maschine: 0.5 a step, 0.05 with SHIFT.)
    local db = math.max(s.volume_db, -120)
    deckard.volume_db(util.round(db + steps * (fine and 0.05 or 0.5), 2))
  end
  return true
end

function global.wheel(steps)
  if global.wheel_mode(steps) then return true end
  local s = deckard.state()
  local group = s.groups[s.group]
  if not group or #group.sounds == 0 then return true end
  local n = util.clamp(s.sound + steps, 1, #group.sounds)
  if n ~= s.sound then deckard.focus_sound(n) end
  return true
end

-- Where the pads play: Step mode enters steps instead.
-- Touch Auto-write: the Control page's knobs held write their modulation while
-- Auto-write is on and the song plays.
function global.tick()
  local s = deckard.state()
  if not (s.touch_auto_write and s.auto_write and s.playing) or deckard.page() ~= "control" then return end
  for k = 1, 8 do
    if deckard.held("CAP_" .. k) then deckard.touch_write(k) end
  end
end

-- The pages a project reopens on, if it was saved on one (not the dialogs, nor the
-- pages shown while a button is held).
local RESTORES = {
  control = true, pad_mode = true, keyboard = true, step = true, arrange = true, mixer = true, song = true,
}

-- A project opened: the pads' mode and the page it was saved on (BUGS #6).
function global.restore(page, mode)
  if mode ~= pads.mode and (mode == "pad" or mode == "keyboard" or mode == "chords" or mode == "step") then
    pads.switch(mode)
  end
  if RESTORES[page] then deckard.show_page(page) end
  return true
end

function global.pads()
  local keys = nil
  -- (Step mode's KEYBOARD maps the pads too, as Maschine's, though they don't play.)
  if pads.keyboard() then keys = { sound = deckard.state().sound, notes = pads.notes(), chords = pads.chords() } end
  -- (Held, PATTERN and SCENE's pads pick patterns and scenes.)
  local picking = pin.active("PATTERN") or pin.active("SCENE")
  return { play = pads.mode ~= "step" and not picking, keys = keys }
end

-- The touch strip's lights.
local TOUCH_STRIP = 25

-- The palette colour Maschine lights a focused empty sound slot in.
local EMPTY_SLOT = 15

-- A one-colour button: lit, or its faint backlight.
local function lit(on)
  return on and "on" or "off"
end

-- The touch strip (lib/strip.lua).
function global.touch(finger, kind, pos)
  return strip.touch(finger, kind, pos)
end

function global.leds()
  local s = deckard.state()
  local shift = deckard.held("SHIFT")
  local page = deckard.page()

  deckard.led("PLAY", lit(s.playing))
  deckard.led("STOP", s.playing and "dim" or "off")
  deckard.led("REC", lit(s.recording))
  if shift then
    deckard.led("RESTART", s.looping and "on" or "dim")
  else
    deckard.led("RESTART", lit(s.looping))
  end
  deckard.led("ERASE", lit(deckard.held("ERASE")))
  deckard.led("TAP", lit(s.metronome))
  deckard.led("AUTO", lit(s.auto_write))
  deckard.led("SHIFT", lit(shift))
  -- (Step mode's own FOLLOW on its pages.)
  if page == "step" or page == "step_mod" then
    deckard.led("FOLLOW", lit(require("pages.step").following()))
  else
    deckard.led("FOLLOW", lit(s.follow))
  end
  deckard.led("FIXED_VEL", lit(s.fixed_velocity))
  deckard.led("PAD_MODE", lit(pads.mode == "pad"))
  deckard.led("KEYBOARD", lit(pads.mode == "keyboard"))
  deckard.led("CHORDS", lit(pads.mode == "chords"))
  deckard.led("STEP", lit(pads.mode == "step"))
  deckard.led("SELECT", lit(pin.active("SELECT")))
  deckard.led("MUTE", lit(pin.active("MUTE")))
  deckard.led("SOLO", lit(pin.active("SOLO")))
  -- (EVENTS, PATTERN and SCENE: lit while their page shows, pinned too.)
  deckard.led("EVENTS", lit(pin.active("EVENTS")))
  deckard.led("PATTERN", lit(pin.active("PATTERN")))
  deckard.led("SCENE", lit(pin.active("SCENE")))
  deckard.led("TEMPO", lit(wheel.mode == "tempo"))
  deckard.led("SWING", lit(wheel.mode == "swing"))
  deckard.led("VOLUME", lit(wheel.mode == "volume"))
  deckard.led("SAMPLE", page == "sampling" and "on" or "dim")
  -- NOTE REPEAT: lit on its page, dim while it's on (LOCK) elsewhere.
  if page == "note_repeat" then
    deckard.led("NOTE_REPEAT", "on")
  else
    deckard.led("NOTE_REPEAT", s.note_repeat.active and "dim" or "off")
  end
  deckard.led("FILE", lit(page == "file" or page == "save"))
  deckard.led("SETTINGS", lit(page == "settings"))
  deckard.led("VARIATION", lit(deckard.held("VARIATION")))
  -- (Bright on Extended Lock; dim while a snapshot waits to be recalled.)
  deckard.led("LOCK", page == "lock" and "on" or (s.lock.active and "dim" or "off"))
  deckard.led("MIX", lit(page == "mixer"))
  deckard.led("ARRANGE", lit(page == "arrange"))
  deckard.led("BROWSE", lit(page == "browse"))

  -- The Control area stays chosen under the pad modes' pages; LEFT / RIGHT turn its
  -- pages unless a modifier is held.
  local c = s.control
  deckard.led("CHANNEL", lit(not c.plugins))
  deckard.led("PLUGIN", lit(c.plugins))
  -- (CHANNEL ▸ Macro is MACRO's page.)
  deckard.led("MACRO", lit(page == "macro_set" or (not c.plugins and c.page_group == 4)))
  local modifier = pin.active("SELECT") or pin.active("MUTE") or pin.active("SOLO")
  deckard.led("LEFT", (not modifier and c.page > 1) and "dim" or "off")
  deckard.led("RIGHT", (not modifier and c.page < #c.pages) and "dim" or "off")

  local muting = pin.active("MUTE") or pin.active("SOLO")

  -- Groups. SHIFT (not with ERASE, which deletes a group; and with eight groups or
  -- more, so banks to pick): the banks of eight groups, white, the focused group's
  -- bright, as Maschine's (`PageMaschine:updateGroupLEDs`).
  if shift and not deckard.held("ERASE") and #s.groups >= 8 then
    local banks = (#s.groups + 7) // 8
    for i = 1, 8 do
      local name = "GROUP_" .. string.char(string.byte("A") + i - 1)
      if i <= banks then
        deckard.led(name, 16, i == (s.group - 1) // 8 + 1)
      elseif (i - 1) * 8 == #s.groups then
        -- (A new bank: its first group is made.)
        deckard.led(name, 16, false)
      else
        deckard.led(name, "off")
      end
    end
  else
    local first = (s.group - 1) // 8 * 8
    for i = 1, 8 do
      local g = first + i
      local group = s.groups[g]
      local name = "GROUP_" .. string.char(string.byte("A") + i - 1)
      if not group then
        -- (The place after the last group makes a new one.)
        if g == #s.groups + 1 and not muting then deckard.led(name, 16, false) else deckard.led(name, "off") end
      elseif muting then
        deckard.led(name, group.palette, not group.muted)
      else
        -- A group lights while one of its notes sounds.
        deckard.led(name, group.palette, g == s.group or (s.playing and group.playing))
      end
    end
  end

  -- The touch strip: its mode's, or it fills as playback goes on, as Maschine's: the
  -- song in white on the Song view, else the focused group's pattern in its colour.
  local pos, colour = nil, nil
  if strip.leds(s) then
    -- (Its mode's.)
  elseif s.strip_position == 0 or (s.strip_position == 1 and not s.recording) then
    -- (HARDWARE's SHOW POSITION: not now.)
  elseif s.song_view then
    local length = 0
    for _, sec in ipairs(s.sections) do length = math.max(length, sec.start + sec.length) end
    local at = s.position or 0
    if at < length then pos, colour = at / length, 16 end
  elseif s.progress then
    local focused = s.groups[s.group]
    local pattern = focused and focused.scene_pattern and focused.patterns[focused.scene_pattern]
    if pattern then pos, colour = s.progress, pattern.palette end
  end
  if not strip.mode then
    local filled = pos and util.round(pos * (TOUCH_STRIP - 1)) + 1 or 0
    for i = 1, TOUCH_STRIP do
      local name = string.format("TS_%02d", i)
      if i <= filled then deckard.led(name, colour, true) else deckard.led(name, "off") end
    end
  end

  -- Pads, as the pad mode shows them (on the Note Repeat page too, as Maschine's: in
  -- Keyboard mode, the Arp's, the keyboard).
  local group = s.groups[s.group]
  local sounds = group and group.sounds or {}
  if pads.keys() then
    local palette = sounds[s.sound] and sounds[s.sound].palette or 16
    for i, note in ipairs(pads.notes()) do
      deckard.led("PAD_" .. i, palette, deckard.pad_held(i) or note % 12 == pads.base % 12)
    end
    return
  end
  for i = 1, 16 do
    local sound = sounds[i]
    local name = "PAD_" .. i
    if not sound or (not sound.used and i ~= s.sound and not deckard.pad_held(i) and not sound.playing) then
      deckard.led(name, "off")
    elseif muting then
      deckard.led(name, sound.palette, not sound.muted)
    elseif not sound.used then
      -- The focused empty slot: Maschine lights it in its empty-slot colour.
      deckard.led(name, EMPTY_SLOT, true)
    else
      deckard.led(name, sound.palette, deckard.pad_held(i) or i == s.sound or sound.playing)
    end
  end
end

return global
