------------------------------------------------------------------------------------------------------------------------
-- DECKARD's Pad Touch pages on the MK3's Keyboard page: what pressing a held pad does
-- for the focused sound (docs/superpowers/specs/2026-10-24-pad-touch-design.md).
-- DECKARD's own code, appended to the Keyboard page when DECKARD loads the scripts.
--   Page 3, Pad Touch:    Mode, Curve, Min, Max
--   Page 4, Pad Touch CC: the CC number (for the CC mode)
------------------------------------------------------------------------------------------------------------------------

local DeckardPadTouchBase = KeyboardModePageMK3.updateParameters

function KeyboardModePageMK3:updateParameters(ForceUpdate)

    -- The Keyboard page's own pages first (it counts two, and draws the page number).
    DeckardPadTouchBase(self, ForceUpdate)

    local Handler = self.ParameterHandler
    Handler.NumPages = 4
    if Handler.PageIndex < 3 then
        -- Drawn again, so the page number counts the Pad Touch pages too.
        PageMaschine.updateParameters(self, ForceUpdate)
        return
    end

    local Sound = NI.DATA.StateHelper.getFocusSound(App)
    local function Setting(Name)
        return Sound and Sound["get" .. Name .. "Parameter"](Sound)
    end

    local Parameters, Names, Sections
    if Handler.PageIndex == 3 then
        Parameters = { Setting("PadTouchMode"), Setting("PadTouchCurve"), Setting("PadTouchMin"), Setting("PadTouchMax") }
        Names = { "MODE", "CURVE", "MIN", "MAX" }
        Sections = { "Pad Touch" }
    else
        Parameters = { Setting("PadTouchCC") }
        Names = { "CC" }
        Sections = { "Pad Touch CC" }
    end

    Handler:setParameters(Parameters)
    Handler:setCustomValues({})
    Handler:setCustomNames(Names)
    Handler:setCustomSections(Sections)
    self.Controller.CapacitiveList:assignParametersToCaps(Handler.Parameters)

    PageMaschine.updateParameters(self, ForceUpdate)

end
