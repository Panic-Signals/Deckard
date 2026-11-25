-- Monark (a Reaktor 6 product): its knob pages on the MK3 and the pictures over them.
-- Parameter ids and choices from `plugin-probe dump Monark --state Monark.ens`
-- (the parameters used: tests/mappings/monark.json). Monark's knobs read like the
-- Minimoog's panel (0-10, 0-100), not in Hz or ms, so the tapers below say what they
-- mean, measured with `plugin-probe measure` (ballpark figures):
--   cutoff: where a noise-fed, high-resonance filter peaks (keytracking off). Above 90
--     the peak leaves the audio range;
--   attack: the time to 90% of the held level (0-17 from 95% of the peak instead: the
--     first 20 ms of a note are skipped for its click);
--   decay: the time from the peak to 10% of it, with sustain at 0.
-- The filter envelope takes the amp envelope's figures: the two are one circuit on
-- the Model D, and Monark's filter attack measured by brightness (noise through a
-- closed filter, energy above 2 kHz) followed the amp attack's to within the noise.
-- (A mapping is data: see crates/surface/src/widget/mapping.rs for the format.)
local CUTOFF = { "table", { 0, 27 }, { 0.05, 35 }, { 0.1, 44 }, { 0.15, 85 }, { 0.2, 107 }, { 0.25, 170 }, { 0.3, 254 }, { 0.35, 381 }, { 0.4, 538 }, { 0.45, 806 }, { 0.5, 1174 }, { 0.55, 1709 }, { 0.6, 2348 }, { 0.65, 3517 }, { 0.7, 5120 }, { 0.75, 7035 }, { 0.8, 9949 }, { 0.85, 14069 }, { 0.9, 19331 }, { 1, 20000 } }
local ATTACK = { "table", { 0, 5 }, { 0.083, 10 }, { 0.167, 17 }, { 0.25, 30 }, { 0.333, 95 }, { 0.417, 244 }, { 0.5, 614 }, { 0.583, 1262 }, { 0.667, 2260 }, { 0.75, 3447 }, { 0.833, 4659 }, { 0.917, 6156 }, { 1, 7483 } }
local DECAY = { "table", { 0, 10 }, { 0.083, 20 }, { 0.167, 50 }, { 0.25, 125 }, { 0.333, 299 }, { 0.417, 813 }, { 0.5, 1921 }, { 0.583, 3786 }, { 0.667, 5847 }, { 0.75, 8566 }, { 0.833, 12262 }, { 0.917, 16937 }, { 1, 22474 } }

return {
  api = 1,
  mapping_version = 4,
  match = { name = "Monark", vendor = "Native Instruments" },
  kind = "synth",

  params = {
    level = { id = 0 },
    octave = { id = 1 },
    fine = { id = 2 },
    glide_type = { id = 3, choices = { "Silver", "MM" } },
    glide_resp = { id = 4, choices = { "Off", "Legato", "Always" } },
    glide_time = { id = 5 },
    mod_wheel = { id = 6 },
    mod_mix = { id = 7 },
    osc1_range = { id = 8, choices = { "Lo", "32'", "16'", "8'", "4'", "2'" } },
    opm = { id = 9, choices = { "Off", "On" } },
    osc1_wave = { id = 10, choices = { "Triangle", "Tri/Saw", "Saw", "Pulse", "Narrow", "Very Nrw" } },
    osc2_range = { id = 11, choices = { "Lo", "32'", "16'", "8'", "4'", "2'" } },
    osc2_freq = { id = 12 },
    osc2_wave = { id = 13, choices = { "Triangle", "Tri/Saw", "Saw", "Pulse", "Narrow", "Very Nrw" } },
    osc3_ktr = { id = 14, choices = { "Off", "On" } },
    osc3_range = { id = 15, choices = { "Lo", "32'", "16'", "8'", "4'", "2'" } },
    osc3_freq = { id = 16 },
    osc3_wave = { id = 17, choices = { "Triangle", "Tri/Saw", "Saw", "Pulse", "Narrow", "Very Nrw" } },
    osc1_on = { id = 18, choices = { "Off", "On" } },
    osc1_vol = { id = 19 },
    osc2_on = { id = 20, choices = { "Off", "On" } },
    osc2_vol = { id = 21 },
    osc3_on = { id = 22, choices = { "Off", "On" } },
    osc3_vol = { id = 23 },
    load = { id = 24 },
    fdbk_on = { id = 25, choices = { "Off", "On" } },
    fdbk_vol = { id = 26 },
    fdbk_type = { id = 27, choices = { "Off", "On" } },
    noise_on = { id = 28, choices = { "Off", "On" } },
    noise_vol = { id = 29 },
    noise_type = { id = 30, choices = { "Off", "On" } },
    fpm = { id = 31, choices = { "Off", "On" } },
    flt_type = { id = 32, knob = { "FltType", "Filter Type" }, choices = { "BP", "LP1", "LP2", "MM" } },
    -- The cutoff dial 0-100, in Hz (measured).
    cutoff = { id = 33, knob = { "Cutoff" }, taper = CUTOFF },
    -- Emphasis 0-10.
    reso = { id = 34, knob = { "Resonance", "Emphasis" }, taper = { "lin", 0, 1 } },
    contour = { id = 35, knob = { "Contour" }, taper = { "lin", 0, 1 } },
    polarity = { id = 36, choices = { "Off", "On" } },
    f_ktr1 = { id = 37, choices = { "Off", "On" } },
    f_ktr2 = { id = 38, choices = { "Off", "On" } },
    -- Attack and decay in ms (measured), sustain 0-10.
    f_att = { id = 39, knob = { "Flt Att" }, taper = ATTACK },
    f_dec = { id = 40, knob = { "Flt Dec" }, taper = DECAY },
    f_sus = { id = 41, knob = { "Flt Sus" }, taper = { "lin", 0, 1 } },
    -- The release switch: on, the release takes the decay's time.
    f_rel = { id = 42, knob = { "Flt Rel" }, choices = { "Off", "On" } },
    a_att = { id = 43, knob = { "Amp Att" }, taper = ATTACK },
    a_dec = { id = 44, knob = { "Amp Dec" }, taper = DECAY },
    a_sus = { id = 45, knob = { "Amp Sus" }, taper = { "lin", 0, 1 } },
    a_rel = { id = 46, knob = { "Amp Rel" }, choices = { "Off", "On" } },
  },

  pictures = {
    -- The oscillators' waves, each range switch drawn as more or fewer cycles.
    osc12 = {
      widget = "oscillators",
      needs = { "osc1_wave" },
      model = {
        wave1 = "osc1_wave", octave1 = { from = "osc1_range", map = { Lo = { const = -1 }, ["32'"] = { const = -1 }, ["16'"] = { const = 0 }, ["8'"] = { const = 1 }, ["4'"] = { const = 2 }, ["2'"] = { const = 3 } } },
        wave2 = "osc2_wave", octave2 = { from = "osc2_range", map = { Lo = { const = -1 }, ["32'"] = { const = -1 }, ["16'"] = { const = 0 }, ["8'"] = { const = 1 }, ["4'"] = { const = 2 }, ["2'"] = { const = 3 } } },
      },
    },
    osc3 = {
      widget = "oscillators",
      needs = { "osc3_wave" },
      model = { wave1 = "osc3_wave", octave1 = { from = "osc3_range", map = { Lo = { const = -1 }, ["32'"] = { const = -1 }, ["16'"] = { const = 0 }, ["8'"] = { const = 1 }, ["4'"] = { const = 2 }, ["2'"] = { const = 3 } } } },
    },
    response = {
      widget = "filter",
      needs = { "cutoff" },
      model = {
        cutoff = "cutoff",
        resonance = "reso",
        shape = { from = "flt_type", map = { BP = { const = "bp" }, LP1 = { const = "lp" }, LP2 = { const = "lp" }, MM = { const = "ladder4" } } },
        slope = { from = "flt_type", map = { BP = { const = 12 }, LP1 = { const = 12 }, LP2 = { const = 24 }, MM = { const = 24 } } },
      },
    },
    filter_env = {
      widget = "envelope",
      needs = { "f_att", "f_dec" },
      model = {
        attack = "f_att",
        decay = "f_dec",
        sustain = "f_sus",
        release = { from = "f_rel", map = { On = "f_dec", Off = { const = 8 } } },
      },
    },
    amp_env = {
      widget = "envelope",
      needs = { "a_att", "a_dec" },
      model = {
        attack = "a_att",
        decay = "a_dec",
        sustain = "a_sus",
        release = { from = "a_rel", map = { On = "a_dec", Off = { const = 8 } } },
      },
    },
  },

  -- Monark's own list starts with Master Level and the oscillators; what's played
  -- first is the filter and its envelope.
  pages = {
    { name = "FILTER", left = "response", right = "filter_env",
      knobs = { "cutoff", "reso", "contour", "flt_type", "f_att", "f_dec", "f_sus", "f_rel" } },
    { name = "AMP", right = "amp_env",
      knobs = { "level", "glide_time", "octave", "fine", "a_att", "a_dec", "a_sus", "a_rel" } },
    { name = "OSCILLATORS", left = "osc12", right = "osc3",
      knobs = { "osc1_range", "osc1_wave", "osc2_range", "osc2_wave", "osc3_range", "osc3_wave", "osc2_freq", "osc3_freq" } },
    { name = "MIXER",
      knobs = { "osc1_vol", "osc2_vol", "osc3_vol", "noise_vol", "fdbk_vol", "load", "mod_mix", "mod_wheel" } },
    { name = "SWITCHES",
      knobs = { "osc1_on", "osc2_on", "osc3_on", "noise_on", "fdbk_on", "opm", "fpm", "osc3_ktr" } },
    { name = "MORE",
      knobs = { "glide_type", "glide_resp", "noise_type", "fdbk_type", "polarity", "f_ktr1", "f_ktr2", false } },
  },
}
