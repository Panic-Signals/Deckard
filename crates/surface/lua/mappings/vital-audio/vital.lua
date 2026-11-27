-- Vital (Vital Audio): a wavetable synth. Ids and texts from `plugin-probe dump Vital`
-- (the parameters used: tests/mappings/vital.json); the dump lists ~2900 parameters
-- (mod matrix slots, effects, random LFOs...), this maps the most played ones and
-- DECKARD adds the rest after the pages.
--   cutoff: the text reads "semitones" from -52 to 76 (0 = 261.63 Hz, C4, 12 per octave),
--     so Hz = 261.63 * 2^(n/12), 13.75 Hz to 21.1 kHz. Checked with `plugin-probe
--     measure` (noise through Filter 1 at high resonance: 12 semitones -> 523 Hz, 30 -> 1479, 58 -> 7453). Filter 2 shares the scale.
--   envelope times: text in secs, 0 to 32; the text follows 32 s * t^4 (0.508 -> 2.130 s,
--     as listed), so a taper of exp 0..32000 ms with curve 4 (from the text, not measured).
--   LFO frequency: text is the period in secs, 128 down to 1/512 (log in t), so
--     Hz = 1/128 .. 512 on a log taper (from the text, not measured).
-- Guessed: the filter picture is always a low-pass (Vital's Blend knob morphs LP/BP/HP and
--   Model changes the circuit; neither is drawn), its slope follows Style 12dB/24dB only.
--   The LFO picture's wave is a plain sine (Vital's LFO shape is drawn in its own editor,
--   not a parameter). The picture's rate is the Seconds mode's; in a tempo sync mode it
--   isn't a rate.
local CUTOFF = { "log", 13.75, 21096 }
local TIME = { "exp", 0, 32000, 4 }
local RATE = { "log", 0.0078125, 512 }

return {
  api = 1,
  mapping_version = 1,
  match = { name = "Vital", vendor = "Vital Audio" },
  kind = "synth",

  params = {
    macro1 = { id = 49618 }, macro2 = { id = 49619 }, macro3 = { id = 49620 }, macro4 = { id = 49621 },
    volume = { id = 51696 },
    vel_track = { id = 51671 },
    polyphony = { id = 51603 },
    portamento = { id = 51607 },

    f1_cutoff = { id = 48629, taper = CUTOFF },
    f1_reso = { id = 48661, taper = { "lin", 0, 1 } },
    f1_drive = { id = 48630 },
    f1_mix = { id = 48658 },
    f1_model = { id = 48659 },
    f1_style = { id = 48662, choices = { "12dB", "24dB" } },
    f1_keytrack = { id = 48657 },
    f1_blend = { id = 48627 },

    f2_cutoff = { id = 48665, taper = CUTOFF },
    f2_reso = { id = 48718, taper = { "lin", 0, 1 } },
    f2_drive = { id = 48687 },
    f2_mix = { id = 48694 },
    f2_model = { id = 48695 },
    f2_style = { id = 48719, choices = { "12dB", "24dB" } },
    f2_keytrack = { id = 48693 },
    f2_blend = { id = 48663 },

    -- Envelope 1 is the amp envelope, envelope 2 the filter's by default.
    e1_att = { id = 1668, taper = TIME }, e1_dec = { id = 1691, taper = TIME },
    e1_sus = { id = 1695, taper = { "lin", 0, 1 } }, e1_rel = { id = 1693, taper = TIME },
    e1_attp = { id = 1669 }, e1_decp = { id = 1692 }, e1_relp = { id = 1694 },
    e2_att = { id = 1696, taper = TIME }, e2_dec = { id = 1698, taper = TIME },
    e2_sus = { id = 1723, taper = { "lin", 0, 1 } }, e2_rel = { id = 1700, taper = TIME },
    e2_attp = { id = 1697 }, e2_decp = { id = 1699 }, e2_relp = { id = 1722 },

    lfo1_rate = { id = 48787, taper = RATE }, lfo1_phase = { id = 48788, taper = { "lin", 0, 1 } },
    lfo1_delay = { id = 48785 }, lfo1_fade = { id = 48786 },
    lfo1_sync = { id = 48789 }, lfo1_tempo = { id = 48812 }, lfo1_mode = { id = 48811 },
    lfo2_rate = { id = 48815, taper = RATE }, lfo2_phase = { id = 48816, taper = { "lin", 0, 1 } },
    lfo2_delay = { id = 48813 }, lfo2_fade = { id = 48814 },
    lfo2_sync = { id = 48817 }, lfo2_tempo = { id = 48819 }, lfo2_mode = { id = 48818 },

    o1_level = { id = 50797 }, o2_level = { id = 51513 }, o3_level = { id = 52503 },
    sample_level = { id = 51641 },
    o1_transpose = { id = 50827 }, o2_transpose = { id = 51543 }, o3_transpose = { id = 52537 },
    o1_frame = { id = 50833 }, o2_frame = { id = 51570 }, o3_frame = { id = 52565 },
    o1_voices = { id = 50832 }, o2_voices = { id = 51548 }, o3_voices = { id = 52563 },
    o1_detune = { id = 50831 }, o2_detune = { id = 51547 }, o3_detune = { id = 52562 },
  },

  pictures = {
    f1_curve = { widget = "filter", needs = { "f1_cutoff" },
      model = { cutoff = "f1_cutoff", resonance = "f1_reso",
                slope = { from = "f1_style", map = { ["12dB"] = { const = 12 }, ["24dB"] = { const = 24 } } } } },
    f2_curve = { widget = "filter", needs = { "f2_cutoff" },
      model = { cutoff = "f2_cutoff", resonance = "f2_reso",
                slope = { from = "f2_style", map = { ["12dB"] = { const = 12 }, ["24dB"] = { const = 24 } } } } },
    env1 = { widget = "envelope", needs = { "e1_att", "e1_dec" },
      model = { attack = "e1_att", decay = "e1_dec", sustain = "e1_sus", release = "e1_rel" } },
    env2 = { widget = "envelope", needs = { "e2_att", "e2_dec" },
      model = { attack = "e2_att", decay = "e2_dec", sustain = "e2_sus", release = "e2_rel" } },
    lfo1 = { widget = "lfo", needs = { "lfo1_rate" }, model = { rate = "lfo1_rate", phase = "lfo1_phase" } },
    lfo2 = { widget = "lfo", needs = { "lfo2_rate" }, model = { rate = "lfo2_rate", phase = "lfo2_phase" } },
  },

  pages = {
    { name = "MACROS",
      knobs = { "macro1", "macro2", "macro3", "macro4", "volume", "vel_track", "polyphony", "portamento" } },
    { name = "FILTER 1", left = "f1_curve",
      knobs = { "f1_cutoff", "f1_reso", "f1_drive", "f1_mix", "f1_model", "f1_style", "f1_keytrack", "f1_blend" } },
    { name = "FILTER 2", left = "f2_curve",
      knobs = { "f2_cutoff", "f2_reso", "f2_drive", "f2_mix", "f2_model", "f2_style", "f2_keytrack", "f2_blend" } },
    { name = "ENV 1 AMP", left = "env1",
      knobs = { "e1_att", "e1_dec", "e1_sus", "e1_rel", "e1_attp", "e1_decp", "e1_relp", "volume" } },
    { name = "ENV 2", left = "env2",
      knobs = { "e2_att", "e2_dec", "e2_sus", "e2_rel", "e2_attp", "e2_decp", "e2_relp", false } },
    { name = "LFO 1", left = "lfo1",
      knobs = { "lfo1_rate", "lfo1_phase", "lfo1_delay", "lfo1_fade", "lfo1_sync", "lfo1_tempo", "lfo1_mode", false } },
    { name = "LFO 2", left = "lfo2",
      knobs = { "lfo2_rate", "lfo2_phase", "lfo2_delay", "lfo2_fade", "lfo2_sync", "lfo2_tempo", "lfo2_mode", false } },
    { name = "OSC LEVELS",
      knobs = { "o1_level", "o2_level", "o3_level", "sample_level", "o1_transpose", "o2_transpose", "o3_transpose", false } },
    { name = "WAVETABLES",
      knobs = { "o1_frame", "o2_frame", "o3_frame", false, false, false, false, false } },
    { name = "UNISON",
      knobs = { "o1_detune", "o2_detune", "o3_detune", false, "o1_voices", "o2_voices", "o3_voices", false } },
  },
}
