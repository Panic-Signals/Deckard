-- bx_oberhausen (Plugin Alliance): an Oberheim-style analog poly (two VCOs, a SEM-style
-- multimode filter, two ADSR-less envelopes, two LFOs). Dump: tests/mappings/bx-oberhausen.json
-- Parameters are matched by their exact titles.
-- Units: the cutoff reads in Hz and the envelope times in ms / s in the plug-in's own text, so they
-- need no taper (no measuring was done). Not in text, so tabled from the dump's 64 sample texts:
--   * sustain reads in dB (-34 .. 0): drawn as the level 10^(dB/20), 0..1 (SUSTAIN1/2 below);
--   * LFO rates read as bare numbers (0.07-53.9, 0.05-50): taken to be Hz (a guess, not measured).
-- The resonance reads -9.7..26.3 (a gain in dB): the picture takes the knob position 0..1.
-- Guesses: Env 2 is the filter's (the filter's mod source offers Env 2; text), Env 1 is the amp's
-- (only a guess: it is VCO1's mod envelope in the panel). The plug-in has no release knob; the
-- pictures use the decay time as the release (guess). "VCOn Level" reads 100%S .. 100%P: a saw
-- to pulse blend, named Wave here. LFO 1 has no shape control: drawn as a sine (guess).
local SUSTAIN1 = { "table", { 0.000, 0.019 }, { 0.063, 0.027 }, { 0.127, 0.066 }, { 0.190, 0.153 }, { 0.254, 0.219 }, { 0.317, 0.288 }, { 0.381, 0.337 }, { 0.444, 0.397 }, { 0.508, 0.473 }, { 0.571, 0.540 }, { 0.635, 0.630 }, { 0.698, 0.746 }, { 0.762, 0.880 }, { 0.825, 0.912 }, { 0.889, 0.944 }, { 0.952, 0.976 }, { 1.000, 1.000 } }
local SUSTAIN2 = { "table", { 0.000, 0.016 }, { 0.063, 0.019 }, { 0.127, 0.059 }, { 0.190, 0.139 }, { 0.254, 0.200 }, { 0.317, 0.251 }, { 0.381, 0.321 }, { 0.444, 0.382 }, { 0.508, 0.444 }, { 0.571, 0.524 }, { 0.635, 0.614 }, { 0.698, 0.733 }, { 0.762, 0.866 }, { 0.825, 0.906 }, { 0.889, 0.946 }, { 0.952, 0.985 }, { 1.000, 1.000 } }
local LFO1_HZ = { "table", { 0.000, 0.07 }, { 0.063, 0.25 }, { 0.127, 0.58 }, { 0.190, 1.12 }, { 0.254, 1.97 }, { 0.317, 2.94 }, { 0.381, 4.10 }, { 0.444, 5.30 }, { 0.508, 6.50 }, { 0.571, 7.63 }, { 0.635, 9.98 }, { 0.698, 13.34 }, { 0.762, 19.90 }, { 0.825, 28.75 }, { 0.889, 40.89 }, { 0.952, 48.72 }, { 1.000, 53.90 } }
local LFO2_HZ = { "table", { 0.000, 0.050 }, { 0.063, 0.078 }, { 0.127, 0.120 }, { 0.190, 0.186 }, { 0.254, 0.289 }, { 0.317, 0.448 }, { 0.381, 0.695 }, { 0.444, 1.077 }, { 0.508, 1.670 }, { 0.571, 2.590 }, { 0.635, 4.015 }, { 0.698, 6.226 }, { 0.762, 9.653 }, { 0.825, 14.968 }, { 0.889, 23.208 }, { 0.952, 35.984 }, { 1.000, 50.000 } }

return {
  api = 1,
  mapping_version = 1,
  match = { name = "bx_oberhausen", vendor = "Plugin Alliance" },
  kind = "synth",

  params = {
    -- Filter (text in Hz)
    cutoff = { name = "Flt Cutoff Frequency" },
    reso = { name = "Flt Resonance", taper = { "lin", 0, 1 } },
    flt_type = { name = "Flt Type" },
    flt_bp = { name = "Flt Bandpass", choices = { "Off", "On" } },
    flt_amt = { name = "Flt Mod Amount" },
    flt_src = { name = "Flt Mod Source", choices = { "Env 2", "LFO 1", "None" } },
    flt_ms = { name = "VCF M/S Control" },
    -- Envelope 2 (filter), times in text
    f_att = { name = "Env2 Attack" },
    f_dec = { name = "Env2 Decay" },
    f_sus = { name = "Env2 Sustain", taper = SUSTAIN2 },
    -- Envelope 1 (amp)
    a_att = { name = "Env1 Attack" },
    a_dec = { name = "Env1 Decay" },
    a_sus = { name = "Env1 Sustain", taper = SUSTAIN1 },
    level = { name = "Level" },
    voices = { name = "Voices" },
    unison = { name = "Unison", choices = { "Off", "On" } },
    glide = { name = "Glide Time" },
    legato = { name = "Legato Only", choices = { "Off", "On" } },
    -- Oscillators
    v1_freq = { name = "VCO1 Freq" },
    v1_det = { name = "VCO1 Detune" },
    v1_wave = { name = "VCO1 Level" },
    v1_pw = { name = "VCO1 PW" },
    v2_freq = { name = "VCO2 Freq" },
    v2_det = { name = "VCO2 Detune" },
    v2_wave = { name = "VCO2 Level" },
    v2_pw = { name = "VCO2 PW" },
    sync = { name = "VCO Sync", choices = { "Off", "On" } },
    v1_spread = { name = "VCO 1 Spread" },
    v2_spread = { name = "VCO 2 Spread" },
    v1_msrc = { name = "VCO1 Mod Source", choices = { "Env 1", "LFO 1", "None" } },
    v1_mdst = { name = "VCO1 Mod Dest" },
    v2_msrc = { name = "VCO2 Mod Source", choices = { "Env 2", "LFO 1", "None" } },
    v2_mdst = { name = "VCO2 Mod Dest" },
    -- LFOs (rates: guessed Hz)
    lfo1_rate = { name = "LFO 1 Frequency", taper = LFO1_HZ },
    lfo2_rate = { name = "LFO 2 Frequency", taper = LFO2_HZ },
    lfo2_type = { name = "LFO 2 Type", choices = { "Sine", "Square", "Saw" } },
    lfo2_sync = { name = "LFO 2 Sync", choices = { "Free", "Tempo" } },
    lfo2_retrig = { name = "LFO 2 Retrigger", choices = { "Off", "On" } },
    lfo2_div = { name = "LFO 2 Synced Wavelength" },
    -- Performance
    transpose = { name = "Transpose" },
    tune = { name = "Global Tune" },
    bend_up = { name = "Pitch Up" },
    bend_down = { name = "Pitch Down" },
  },

  pictures = {
    response = {
      widget = "filter",
      needs = { "cutoff" },
      model = {
        cutoff = "cutoff",
        resonance = "reso",
        shape = { from = "flt_bp", map = { Off = { const = "lp" }, On = { const = "bp" } } },
        slope = { const = 12 },
      },
    },
    filter_env = {
      widget = "envelope",
      needs = { "f_att", "f_dec" },
      model = { attack = "f_att", decay = "f_dec", sustain = "f_sus", release = "f_dec" },
    },
    amp_env = {
      widget = "envelope",
      needs = { "a_att", "a_dec" },
      model = { attack = "a_att", decay = "a_dec", sustain = "a_sus", release = "a_dec" },
    },
    lfo1 = { widget = "lfo", needs = { "lfo1_rate" }, model = { rate = "lfo1_rate", shape = { const = "sine" } } },
    lfo2 = {
      widget = "lfo",
      needs = { "lfo2_rate" },
      model = {
        rate = "lfo2_rate",
        shape = { from = "lfo2_type", map = { Sine = { const = "sine" }, Square = { const = "square" }, Saw = { const = "saw" } } },
      },
    },
  },

  pages = {
    { name = "FILTER", left = "response", right = "filter_env",
      knobs = { "cutoff", "reso", "flt_type", "flt_bp", "f_att", "f_dec", "f_sus", "flt_amt" } },
    { name = "AMP", right = "amp_env",
      knobs = { "level", "voices", "glide", "unison", "a_att", "a_dec", "a_sus", "legato" } },
    { name = "OSC 1", knobs = { "v1_freq", "v1_det", "v1_wave", "v1_pw", "v1_spread", "sync", "flt_ms", "flt_src" } },
    { name = "OSC 2", knobs = { "v2_freq", "v2_det", "v2_wave", "v2_pw", "v2_spread", "transpose", "tune", false } },
    { name = "LFO", left = "lfo1", right = "lfo2",
      knobs = { "lfo1_rate", false, false, false, "lfo2_rate", "lfo2_type", "lfo2_sync", "lfo2_retrig" } },
    { name = "OSC MOD", knobs = { "v1_msrc", "v1_mdst", "v2_msrc", "v2_mdst", "lfo2_div", "bend_up", "bend_down", false } },
  },
}
