-- Launchpad Pro [MK3] (docs/launchpad-pro-mk3.md): its DAW port drives the Session
-- grid; its MIDI port's notes play DECKARD as any keyboard's do.
return {
  name = "Launchpad Pro [MK3]",
  -- (macOS and Linux name the port "LPProMK3 DAW"; Windows "MIDIIN3 (LPProMK3 MIDI)"
  -- style names differ, so both fragments.)
  ports = { daw = { "LPProMK3 DAW", "MIDIIN3 (LPProMK3", "MIDIOUT3 (LPProMK3" } },
  exclusive = { "daw" },
  -- Its notes port, for a drum channel (Preferences ▸ MIDI): the sequencer's drum
  -- tracks and the Drum layout play the focused group's pads from C1.
  drums = { "LPProMK3 MIDI" },
}
