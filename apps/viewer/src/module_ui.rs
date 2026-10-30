//! Pages of Maschine's own effects in the Control area (docs/fx-*.md): which knob is
//! in each of the eight slots, its section heading, and its value in Maschine's units.

use crate::layout::{Edit, Knob};
use crate::sampler_ui::Page;
use deckard_doc::Target;
use deckard_model::SlotInfo;

/// The page names of a module DECKARD shows, if any.
pub fn pages(module: &str) -> Option<&'static [&'static str]> {
    Some(match module {
        "EQ" => &["Freq / Gain", "Width / Output"],
        "Compressor" => &["Main", "Side-Chain Input"],
        "Limiter" => &["Main", "Side-Chain Input"],
        "Chorus" => &["Main"],
        "Maximizer" => &["Main", "Side-Chain Input"],
        "Filter" => &["Main", "Side-Chain Input"],
        "Saturator" => &["Main"],
        "BeatDelay" => &["Main", "Unit"],
        "Gate" => &["Main", "Side-Chain Input"],
        "Iceverb" => &["Main"],
        "Reflex" => &["Main"],
        "Metaverb" => &["Main"],
        "LoFi" => &["Main"],
        "Reverb" => &["Main"],
        "Flanger" => &["Main"],
        "Phaser" => &["Main"],
        "TransientDesigner" => &["Main"],
        "FreqShifter" => &["Main"],
        "FM" => &["Main"],
        "CabinetEmulation" => &["Main"],
        "MullhollandDrive" => &["Main", "Gate"],
        "Resochord" => &["Main"],
        "GrainDelay" => &["Main", "Output"],
        "GrainStretch" => &["Main"],
        "PlateReverb" => &["Main", "EQ"],
        "PerformanceFX" => &["Main", "Setup"],
        "BassSynth" => &["Main", "Advanced"],
        "PolySynth" => &POLY_PAGES,
        m if crate::drum_ui::is_drum(m) => &crate::drum_ui::PAGES,
        _ => return None,
    })
}

/// The Poly Synth's pages (docs/synth-poly-synth.md §1.1).
const POLY_PAGES: [&str; 9] =
    ["Filter", "Envelopes", "Mix / Voices", "Oscillator A", "Oscillator B", "LFO", "Modulation", "Poly Mod", "Global"];
const POLY_FILTER_MODES: [&str; 3] = ["LP", "BP", "HP"];
const POLY_LFO_MODES: [&str; 2] = ["Free", "Sync"];
const POLY_VOICE_MODES: [&str; 3] = ["Poly", "Unison", "Uni + Leg"];
const POLY_VOICES: [&str; 16] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16"];

const CABINETS: [&str; 4] = ["Am. 2x12", "Brit. 2x12A", "Vin. 4x12A", "Mod. 4x12"];
const MICROPHONES: [&str; 6] = ["Dynamic 57", "Dynamic 421", "Dynamic 441", "Ribbon 121", "Condenser 47", "Condenser 67"];
const DISTORTION_MODES: [&str; 2] = ["Mulholland", "Analog"];
const RESOCHORD_MODES: [&str; 2] = ["String", "Chord"];
const RESOCHORD_STYLES: [&str; 5] = ["maj", "min", "alt", "meta", "frank"];
const RESOCHORD_CHORDS: [&str; 5] = ["-", "5", "7", "7/9", "7/11"];
/// Grain Stretch's Loop labels (index n loops n/4 beats; the labels are Maschine's).
const STRETCH_LOOPS: [&str; 9] = ["1", "2", "3", "4", "6", "8", "12", "16", "32"];
const MOD_SOURCES: [&str; 3] = ["LFO", "LFO Sync", "Envelope"];
/// The LFO rates of the Flanger and Phaser, in bars (n/d).
const MOD_RATES: [(i32, i32); 11] = [(16, 1), (8, 1), (4, 1), (2, 1), (1, 1), (1, 2), (3, 8), (1, 4), (3, 16), (1, 8), (1, 16)];
const PLATE_MODES: [&str; 3] = ["Room", "Hall", "Plate"];
const REVERB_ROOMS: [&str; 4] = ["General", "Bright", "Guitar", "Shatter"];
const LIMITER_MODES: [&str; 2] = ["Legacy", "Transparent"];
const COMPRESSOR_MODES: [&str; 2] = ["Classic", "Feedback"];
const DELAY_TIMES: [&str; 17] = [
    "1/32", "1/16", "2/16", "3/16", "4/16", "5/16", "6/16", "7/16", "8/16", "9/16", "10/16", "11/16", "12/16", "13/16", "14/16", "15/16",
    "16/16",
];
const DELAY_OFFSETS: [&str; 9] = ["0", "1", "2", "3", "4", "5", "6", "7", "8"];
const DELAY_UNITS: [&str; 3] = ["1/16", "1/16T", "1/32"];
const SATURATOR_MODES: [&str; 3] = ["Classic", "Tape", "Tube"];
const FILTER_MODES: [&str; 4] = ["LP", "BP", "HP", "Notch"];
const FILTER_SOURCES: [&str; 3] = ["LFO", "LFO Sync", "Env"];
/// The Filter LFO's rates, in bars (docs/fx-filter.md §4).
const FILTER_RATES: [(i32, i32); 12] = [(16, 1), (8, 1), (4, 1), (2, 1), (1, 1), (1, 2), (3, 8), (1, 4), (3, 16), (1, 8), (1, 16), (1, 32)];

/// The Filter's frequency text: "%.2f Hz" under 10 Hz, "%.1f Hz" under 1 kHz, else kHz.
fn filter_hz(hz: f64) -> String {
    if hz < 0.005 {
        "0.00 Hz".into()
    } else if hz < 10.0 {
        format!("{hz:.2} Hz")
    } else if hz < 1000.0 {
        format!("{hz:.1} Hz")
    } else {
        format!("{:.1} kHz", hz / 1000.0)
    }
}

thread_local! {
    static TEMPO: std::cell::Cell<f64> = const { std::cell::Cell::new(120.0) };
}

/// The song's tempo, for the pages that show times at it (the app sets it each frame).
pub fn set_tempo(bpm: f64) {
    TEMPO.with(|t| t.set(bpm));
}

fn tempo() -> f64 {
    TEMPO.with(std::cell::Cell::get)
}

/// Perform FX's choices (docs/superpowers/plans/2026-10-22-phase5-perform-fx.md).
pub const PERFORM_MODES: [&str; 8] = ["Filter", "Flanger", "Burst Echo", "Reso Echo", "Ring", "Stutter", "Tremolo", "Scratcher"];
const PF_FILTER_TYPES: [&str; 3] = ["Low pass", "Band pass", "High pass"];
const PF_TONES: [&str; 2] = ["Flanger", "Phaser"];
const PF_POLARITY: [&str; 2] = ["Negative", "Positive"];
const PF_RING_MIX: [&str; 2] = ["Mix", "Wet only"];
const PF_TIME_MODES: [&str; 4] = ["Straight", "Dotted", "Triplet", "Free"];
const PF_BURST_ASSIGN: [&str; 2] = ["Feedback", "Time"];
const PF_BURST_MIX: [&str; 3] = ["Mix", "Replace", "Wet only"];
const PF_RESO_MIX: [&str; 3] = ["Mix", "Insert", "Wet only"];
const PF_STUTTER_ASSIGN: [&str; 2] = ["Length", "Pitch"];
const PF_DIRECTIONS: [&str; 3] = ["Forward", "Reverse", "Both"];
const PF_VIBR_MODES: [&str; 2] = ["Varispeed", "Freq Shift"];

/// A Perform FX page: Main (Mode, the strip's Engage and position, the effect's own
/// knobs) or Setup, for the effect its Mode picks (manual pp. 381–390).
fn perform_fx_page(slot: &SlotInfo, t: Target, page: usize, slots: &mut [Option<Knob>; 8], sections: &mut Vec<(usize, &'static str)>) {
    let defaults = deckard_model::PERFORM_FX.1;
    let d = |f: &str| defaults.iter().find(|(k, _)| *k == f).map_or(0.0, |x| x.1);
    let v = |f: &str| slot.param(f).unwrap_or_else(|| d(f));
    let pct = |x: f64| format!("{:.1} %", x * 100.0);
    let knob = |label: &'static str, field: &'static str, text: String| -> Option<Knob> {
        Some((label.into(), text, v(field) as f32, Some(Edit::Param(t, field, 0.0, 1.0, d(field)))))
    };
    let pc = |label: &'static str, field: &'static str| knob(label, field, pct(v(field)));
    let choice = |label: &'static str, field: &'static str, list: &'static [&'static str]| -> Option<Knob> {
        let i = (v(field).round().max(0.0) as usize).min(list.len() - 1);
        Some((label.into(), list[i].into(), 0.0, Some(Edit::Choice(t, field, list, i, d(field)))))
    };
    let toggle = |label: &'static str, field: &'static str| -> Option<Knob> {
        let on = v(field) >= 0.5;
        Some((label.into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, field, on))))
    };
    let mode = (v("mode").round().max(0.0) as usize).min(7);
    let pos = v("touchstrip_pos");
    if page == 0 {
        sections.extend([(0, "Mode"), (1, "Touchstrip")]);
        slots[0] = choice("MODE", "mode", &PERFORM_MODES);
        slots[1] = toggle("ENGAGE", "touchstrip_on_off");
    }
    let strip = |label: &'static str, text: String| knob(label, "touchstrip_pos", text);
    match (mode, page) {
        (0, 0) => {
            let (lo, hi) = (v("filter_min_freq"), v("filter_max_freq"));
            slots[2] = strip("FREQUENCY", hz(20.0 * 1000f64.powf(lo + (hi - lo) * pos)));
            sections.push((3, "Filter"));
            slots[3] = choice("TYPE", "filter_type", &PF_FILTER_TYPES);
            slots[4] = pc("RESONANCE", "filter_resonance");
        }
        (0, _) => {
            sections.extend([(0, "Range"), (2, "Saturation")]);
            slots[0] = knob("MIN. FREQ", "filter_min_freq", hz(20.0 * 1000f64.powf(v("filter_min_freq"))));
            slots[1] = knob("MAX. FREQ", "filter_max_freq", hz(20.0 * 1000f64.powf(v("filter_max_freq"))));
            slots[2] = pc("SATURATION", "filter_saturation");
            slots[3] = knob("OUT LEVEL", "filter_out_level", format!("{:+.1} dB", (v("filter_out_level") - 0.74) * 40.0));
        }
        (1, 0) => {
            slots[2] = strip("FREQUENCY", pct(pos));
            sections.push((3, "Flanger"));
            slots[3] = pc("DECAY", "flanger_decay");
            slots[4] = pc("SATURATION", "flanger_saturation");
            slots[5] = pc("STEREO", "flanger_stereo");
        }
        (1, _) => {
            sections.extend([(0, "Color"), (2, "Output")]);
            slots[0] = choice("POLARITY", "flanger_polarity", &PF_POLARITY);
            slots[1] = choice("TONE", "flanger_tone", &PF_TONES);
            slots[2] = pc("WET LEVEL", "flanger_wet_level");
        }
        (2, 0) => {
            let time = v("burst_ts_assign") >= 0.5;
            slots[2] = strip(if time { "TIME" } else { "FEEDBACK" }, pct(pos));
            slots[3] = choice("TS ASSIGN", "burst_ts_assign", &PF_BURST_ASSIGN);
            sections.push((4, "Echo"));
            slots[4] = if time { pc("FEEDBACK", "burst_feedback") } else { pc("TIME", "burst_time") };
            slots[5] = choice("TIME MODE", "burst_time_mode", &PF_TIME_MODES);
            slots[6] = pc("FILTER", "burst_filter");
            slots[7] = knob("WET LEVEL", "burst_wet_level", pct(v("burst_wet_level") * 1.5));
        }
        (2, _) => {
            sections.push((0, "Routing"));
            slots[0] = choice("MIX MODE", "burst_mix_mode", &PF_BURST_MIX);
            slots[1] = pc("FEED", "burst_feed");
        }
        (3, 0) => {
            slots[2] = strip("FREQUENCY", hz(40.0 * 100f64.powf(pos)));
            sections.push((3, "Echo"));
            slots[3] = knob("TIME", "reso_time", format!("{:.1} ms", 1000f64.powf(v("reso_time"))));
            slots[4] = pc("FEEDBACK", "reso_feedback");
            slots[5] = pc("PUMP", "reso_pump");
            slots[6] = pc("WET LEVEL", "reso_wet_level");
            slots[7] = choice("MIX MODE", "reso_mix_mode", &PF_RESO_MIX);
        }
        (3, _) => {
            sections.push((0, "Colour"));
            slots[0] = pc("SATURATION", "reso_saturation");
            slots[1] = pc("RESONANCE", "reso_resonance");
        }
        (4, 0) => {
            slots[2] = strip("FREQUENCY", hz(30.0 * 100f64.powf(pos)));
            sections.extend([(3, "Osc"), (5, "Reverb"), (7, "Mix")]);
            slots[3] = pc("SPREAD", "ring_spread");
            let g = v("ring_glide");
            slots[4] = knob(
                "GLIDE",
                "ring_glide",
                if g <= 0.01 { "Off".into() } else { format!("{:.0} ms", 10f64.powf((40.0 * g + 30.0) / 20.0)) },
            );
            slots[5] = pc("REV TIME", "ring_rev_time");
            slots[6] = pc("REV MIX", "ring_rev_mix");
            slots[7] = pc("DRY/WET", "ring_dry_wet");
        }
        (4, _) => {
            sections.push((0, "Routing"));
            slots[0] = choice("MIX MODE", "ring_mix_mode", &PF_RING_MIX);
        }
        (5, 0) => {
            let pitch = v("stutter_ts_assign") >= 0.5;
            // Length as Maschine shows it: a note value with Quantize, else the loop's time
            // at the tempo.
            let length = |x: f64| deckard_model::stutter_length_text(x, v("stutter_quantize") >= 0.5, tempo());
            slots[2] = strip(if pitch { "PITCH" } else { "LENGTH" }, if pitch { pct(pos) } else { length(pos) });
            slots[3] = choice("TS ASSIGN", "stutter_ts_assign", &PF_STUTTER_ASSIGN);
            sections.extend([(4, "Stutter"), (6, "Loop")]);
            slots[4] = pc("GATE", "stutter_gate");
            slots[5] = if pitch { knob("LENGTH", "stutter_length", length(v("stutter_length"))) } else { pc("PITCH", "stutter_pitch") };
            if !pitch {
                slots[6] = choice("DIRECTION", "stutter_direction", &PF_DIRECTIONS);
            }
            slots[7] = toggle("QUANTIZE", "stutter_quantize");
        }
        (5, _) => {
            sections.push((0, "Loop"));
            slots[0] = toggle("RETRIGGER", "stutter_retrigger");
        }
        (6, 0) => {
            slots[2] = strip("POSITION", pct(pos));
            sections.push((3, "Tremolo"));
            slots[3] = pc("TREM/VIBR", "tremolo_trem_vibr");
            slots[4] = choice("VIBR MODE", "tremolo_vibr_mode", &PF_VIBR_MODES);
            slots[5] = pc("ATTK/REL", "tremolo_attack_release");
            slots[6] = pc("STEREO", "tremolo_stereo");
        }
        (6, _) => {
            sections.extend([(0, "Depth Range"), (2, "Rate Range")]);
            slots[0] = pc("DEPTH MIN", "tremolo_depth_min");
            slots[1] = pc("DEPTH MAX", "tremolo_depth_max");
            slots[2] = pc("RATE MIN", "tremolo_rate_min");
            slots[3] = pc("RATE MAX", "tremolo_rate_max");
        }
        (_, 0) => {
            slots[2] = strip("POSITION", pct(pos));
            sections.push((3, "Swirl"));
            slots[3] = pc("DELAY TIME", "scratcher_delay_time");
            slots[4] = pc("FEEDBACK", "scratcher_feedback");
            let f = v("scratcher_freq_shift");
            slots[5] =
                knob("FREQ SHIFT", "scratcher_freq_shift", format!("{:+.1} Hz", (2.0 * f - 1.0) * if f < 0.5 { 100.0 } else { 50.0 }));
            slots[6] = pc("WET LEVEL", "scratcher_wet_level");
        }
        _ => {}
    }
}

fn hz(v: f64) -> String {
    if v < 1000.0 { format!("{v:.1} Hz") } else { format!("{:.2} kHz", v / 1000.0) }
}

/// Maschine's "ScaleNormalizedToPercentOneDecimal".
fn pct(v: f64) -> String {
    format!("{:.1} %", v * 100.0)
}

/// The Flanger's (and Phaser's) Speed: a free rate interpolated over [`MOD_RATES`], or
/// the synced rate.
fn mod_speed(sp: f64, synced: bool) -> String {
    if synced {
        let (n, d) = MOD_RATES[((10.0 * sp).round().clamp(0.0, 10.0)) as usize];
        return format!("{n}/{d}");
    }
    let bars = |i: usize| MOD_RATES[i].0 as f64 / MOD_RATES[i].1 as f64;
    let pos = (10.0 * sp).clamp(0.0, 10.0);
    let i = (pos.floor() as usize).min(9);
    let f = pos - i as f64;
    let hz = 1.0 / (2.0 * ((1.0 - f) * bars(i) + f * bars(i + 1)));
    if hz < 0.005 {
        "0.00 Hz".into()
    } else if hz < 10.0 {
        format!("{hz:.2} Hz")
    } else {
        format!("{hz:.1} Hz")
    }
}

/// Maschine's `hzToString`: "17.0 kHz", "28.8 Hz", "9.50 Hz".
fn lofi_hz(hz: f64) -> String {
    if hz >= 1000.0 {
        format!("{:.1} kHz", hz / 1000.0)
    } else if hz >= 10.0 {
        format!("{hz:.1} Hz")
    } else {
        format!("{hz:.2} Hz")
    }
}

/// `lo·(hi/lo)^v`: Maschine's exponential frequency knobs.
fn exp_hz(lo: f64, hi: f64, v: f64) -> f64 {
    lo * (hi / lo).powf(v)
}

/// Page `page` of the module in slot `i` of sound `s` (group `g`).
pub fn page(slot: &SlotInfo, g: usize, s: usize, i: usize, page: usize) -> Page {
    if crate::drum_ui::is_drum(&slot.module) {
        return crate::drum_ui::page(slot, g, s, i, page);
    }
    page_for(slot, Target::Module(g, s, i), page)
}

/// Page `page` of the module that target `t` names (a slot of any chain).
pub fn page_for(slot: &SlotInfo, t: Target, page: usize) -> Page {
    if let Target::Module(g, s, i) = t
        && crate::drum_ui::is_drum(&slot.module)
    {
        return crate::drum_ui::page(slot, g, s, i, page);
    }
    let v = |f: &str, d: f64| slot.param(f).unwrap_or(d);
    let knob = |label: &'static str, field: &'static str, lo: f64, hi: f64, default: f64, text: String| -> Option<Knob> {
        let x = v(field, default);
        Some((label.into(), text, ((x - lo) / (hi - lo)) as f32, Some(Edit::Param(t, field, lo, hi, default))))
    };
    let mut slots: [Option<Knob>; 8] = Default::default();
    let mut sections = Vec::new();
    match (slot.module.as_str(), page) {
        ("EQ", 0) => {
            let bands = [
                ("Low", "low_freq", "low_gain", 20.0, 8000.0, 0.420_214),
                ("Low-Mid", "low_mid_freq", "low_mid_gain", 40.0, 16000.0, 0.460_128),
                ("High-Mid", "high_mid_freq", "high_mid_gain", 40.0, 16000.0, 0.650_41),
                ("High", "high_freq", "high_gain", 50.0, 20000.0, 0.660_119),
            ];
            for (k, (section, freq, gain, lo, hi, d)) in bands.into_iter().enumerate() {
                sections.push((2 * k, section));
                let f = v(freq, d);
                slots[2 * k] = knob("FREQ", freq, 0.0, 1.0, d, hz(exp_hz(lo, hi, f)));
                let gv = v(gain, 0.0);
                slots[2 * k + 1] = knob("GAIN", gain, -1.0, 1.0, 0.0, format!("{:.1} dB", gv * 20.0));
            }
        }
        ("EQ", _) => {
            for (k, section, field) in [(2, "Low-Mid", "low_mid_width"), (5, "High-Mid", "high_mid_width")] {
                sections.push((k, section));
                let w = v(field, 0.5);
                slots[k] = knob("WIDTH", field, 0.0, 1.0, 0.5, format!("{:.1} oc", 0.1 + 3.9 * w));
            }
            sections.push((7, "Output"));
            let o = v("output_gain", 0.0);
            slots[7] = knob("GAIN", "output_gain", -1.0, 1.0, 0.0, format!("{:.1} dB", o * 20.0));
        }
        ("PerformanceFX", page) => perform_fx_page(slot, t, page, &mut slots, &mut sections),
        ("Compressor" | "Limiter" | "Maximizer" | "Filter" | "Gate", 1) => {
            // Slot 0 is the Source (a routing list the Control area adds).
            sections.extend([(0, "Input"), (2, "Filter")]);
            let gain = v("side_chain_filter/gain", 1.0);
            slots[1] = knob("GAIN", "side_chain_filter/gain", 0.0, 1.0, 1.0, format!("{:+.1} dB", 40.0 * gain - 20.0));
            let on = v("side_chain_filter/filter", 0.0) != 0.0;
            slots[2] = Some((
                "FILTER".into(),
                if on { "On" } else { "Off" }.into(),
                on as i32 as f32,
                Some(Edit::Toggle(t, "side_chain_filter/filter", on)),
            ));
            let c = v("side_chain_filter/center_freq", 0.5);
            slots[3] = knob("CENTER FREQ", "side_chain_filter/center_freq", 0.0, 1.0, 0.5, hz(20.0 * 1000f64.powf(c)));
            let w = v("side_chain_filter/width", 0.5);
            slots[4] = knob("WIDTH", "side_chain_filter/width", 0.0, 1.0, 0.5, format!("{:.1} %", w * 100.0));
        }
        ("Compressor", _) => {
            let mode = (v("mode", 0.0) as usize).min(1);
            sections.extend([(0, "Mode"), (1, "Depth"), (4, "Time"), (6, "Output")]);
            slots[0] =
                Some(("MODE".into(), COMPRESSOR_MODES[mode].into(), 0.0, Some(Edit::Choice(t, "mode", &COMPRESSOR_MODES, mode, 0.0))));
            let thr = v("threshold", 1.0 / 6.0);
            slots[1] = knob("THRESHOLD", "threshold", 0.0, 1.0, 1.0 / 6.0, format!("{:.1} dB", -60.0 * thr));
            slots[2] = knob("KNEE", "knee", 0.0, 1.0, 0.5, format!("{:.1} %", v("knee", 0.5) * 100.0));
            slots[3] = knob("AMOUNT", "amount", 0.0, 1.0, 0.75, format!("{:.1} %", v("amount", 0.75) * 100.0));
            let attack_default = (0.25f64 / 8.0).powf(1.0 / 3.0);
            let a = v("attack", attack_default);
            slots[4] = knob("ATTACK", "attack", 0.0, 1.0, attack_default, format!("{:.2} ms", 8.0 * a * a * a));
            let r = v("release", 0.5);
            let rel = 1.122_018_5f64.powf(16.0 * r + 32.0).clamp(40.0, 251.0);
            slots[5] = knob("RELEASE", "release", 0.0, 1.0, 0.5, format!("{rel:.1} ms"));
            let gn = v("gain", 0.916_666_7);
            slots[6] = knob("GAIN", "gain", 0.0, 1.0, 0.916_666_7, format!("{:.1} dB", 36.0 * gn - 36.0));
        }
        ("Limiter", _) => {
            let mode = (v("mode", 1.0) as usize).min(1);
            sections.extend([(0, "Mode"), (1, "Depth")]);
            slots[0] = Some(("MODE".into(), LIMITER_MODES[mode].into(), 0.0, Some(Edit::Choice(t, "mode", &LIMITER_MODES, mode, 1.0))));
            let thr = v("threshold", 0.0);
            slots[1] = knob("THRESHOLD", "threshold", -40.0, 0.0, 0.0, format!("{thr:.1} dB"));
            // Release and Ceiling only in Transparent mode.
            if mode == 1 {
                let r = v("release", 0.0);
                slots[2] = knob("RELEASE", "release", 0.0, 1.0, 0.0, format!("{:.1} ms", 500f64.powf(r)));
                sections.push((3, "Output"));
                let c = v("ceiling", 1.0);
                slots[3] = knob("CEILING", "ceiling", 0.0, 1.0, 1.0, format!("{:.1} dB", -40.0 + 39.7 * c));
            }
        }
        ("Gate", _) => {
            sections.extend([(0, "Depth"), (1, "Time"), (4, "Output")]);
            // Stored linear (0.1..1), shown as 20·log10(v²): −40..0 dB.
            let thr = v("threshold", 0.562_341_3);
            let a = ((thr / 0.1).log10()) as f32;
            slots[0] = Some((
                "THRESHOLD".into(),
                format!("{:.1} dB", 40.0 * thr.log10()),
                a,
                Some(Edit::Log(t, "threshold", 0.1, 1.0, 0.562_341_3)),
            ));
            for (k, label, field, d) in
                [(1, "ATTACK", "attack", 0.0), (2, "HOLD", "hold", 0.5), (3, "RELEASE", "release", 0.25), (4, "MIX", "mix", 1.0)]
            {
                slots[k] = knob(label, field, 0.0, 1.0, d, pct(v(field, d)));
            }
        }
        ("Iceverb" | "Reflex", _) => {
            sections.extend([(0, "Room"), (3, "Output")]);
            let defaults = if slot.module == "Iceverb" { [0.75, 0.75, 0.5, 0.5] } else { [0.5, 0.75, 0.25, 0.5] };
            let second = if slot.module == "Iceverb" { ("ICE", "ice") } else { ("SMOOTH", "smooth") };
            for (k, (label, field)) in [("COLOR", "color"), second, ("SIZE", "size"), ("MIX", "mix")].into_iter().enumerate() {
                slots[k] = knob(label, field, 0.0, 1.0, defaults[k], pct(v(field, defaults[k])));
            }
        }
        ("Metaverb", _) => {
            sections.extend([(0, "Room"), (1, "EQ"), (3, "Position"), (4, "Output")]);
            for (k, label, field) in [(0, "SIZE", "size"), (1, "LOW", "low"), (2, "HIGH", "high"), (4, "MIX", "mix")] {
                slots[k] = knob(label, field, 0.0, 1.0, 0.5, pct(v(field, 0.5)));
            }
            slots[3] = knob("PAN", "pan", -1.0, 1.0, 0.0, crate::layout::pan(v("pan", 0.0)));
        }
        ("LoFi", _) => {
            sections.extend([(0, "Resample"), (1, "Bitcrush"), (4, "Output")]);
            let sr_default = f32::from_bits(0x3e04_def7) as f64;
            let sr = v("sample_rate", sr_default);
            slots[0] = knob("SR", "sample_rate", 0.0, 1.0, sr_default, lofi_hz(44100.0 * 2f64.powf(-127.0 * sr / 12.0)));
            slots[1] = knob("BITS", "bits", 0.0, 1.0, 0.185, format!("{:.1}", 12.0 - 11.0 * v("bits", 0.185)));
            for (k, label, field) in [(2, "SMOOTH", "smooth"), (3, "STEREO", "stereo"), (4, "MIX", "mix")] {
                slots[k] = knob(label, field, 0.0, 1.0, 1.0, pct(v(field, 1.0)));
            }
        }
        ("PlateReverb", p) => {
            // New modules are Room; version-0 projects (no Mode stored) are Plate.
            let mode = (slot.param("mode").unwrap_or(if slot.param("wet_gain").is_some() { 0.0 } else { 2.0 }).max(0.0) as usize).min(2);
            let x = if mode == 1 { "hall" } else { "room" };
            let f = |name: &str| -> &'static str {
                match (x, name) {
                    ("hall", "rev_time") => "hall_rev_time",
                    ("hall", "size") => "hall_size",
                    ("hall", "damp") => "hall_damp",
                    ("hall", "mod") => "hall_mod",
                    ("hall", "diff") => "hall_diff",
                    ("hall", "pre_delay") => "hall_pre_delay",
                    ("hall", "low_shelf") => "hall_low_shelf",
                    ("hall", "high_cut") => "hall_high_cut",
                    (_, "rev_time") => "room_rev_time",
                    (_, "size") => "room_size",
                    (_, "damp") => "room_damp",
                    (_, "mod") => "room_mod",
                    (_, "diff") => "room_diff",
                    (_, "pre_delay") => "room_pre_delay",
                    (_, "low_shelf") => "room_low_shelf",
                    _ => "room_high_cut",
                }
            };
            if p == 1 {
                if mode != 2 {
                    sections.push((0, "EQ"));
                    let ls = v(f("low_shelf"), 0.0);
                    slots[0] = knob("LOW SHELF", f("low_shelf"), 0.0, 1.0, 0.0, format!("{:.1} dB", -12.0 * ls));
                    let hc = v(f("high_cut"), 0.0);
                    slots[1] = knob("HIGH CUT", f("high_cut"), 0.0, 1.0, 0.0, format!("{:.1} kHz", 20.0 - 18.0 * hc));
                }
            } else {
                sections.extend([(0, "Room"), (7, "Output")]);
                slots[0] = Some(("MODE".into(), PLATE_MODES[mode].into(), 0.0, Some(Edit::Choice(t, "mode", &PLATE_MODES, mode, 0.0))));
                if mode == 2 {
                    sections.push((4, "Color"));
                    slots[1] = knob("DECAY", "plate_decay", 0.0, 1.0, 0.3, pct(v("plate_decay", 0.3)));
                    slots[4] = knob("LOW SHELF", "plate_low_shelf", 0.0, 1.0, 0.0, pct(v("plate_low_shelf", 0.0)));
                    slots[5] = knob("HI DAMP", "plate_high_damp", 0.0, 1.0, 0.0, pct(v("plate_high_damp", 0.0)));
                    let ms = 250.0 * v("plate_pre_delay", 0.0);
                    let text = if ms < 0.005 {
                        "0.00 ms".to_string()
                    } else if ms < 1.0 {
                        format!("{ms:.2} ms")
                    } else {
                        format!("{ms:.1} ms")
                    };
                    slots[6] = knob("PRE DELAY", "plate_pre_delay", 0.0, 1.0, 0.0, text);
                } else {
                    sections.push((3, "Color"));
                    let hall = mode == 1;
                    let d = |room: f64, hall_d: f64| if hall { hall_d } else { room };
                    let rt = v(f("rev_time"), d(0.2, 0.4));
                    slots[1] = knob("REVERB TIME", f("rev_time"), 0.0, 1.0, d(0.2, 0.4), format!("{:.1} s", (3.7 * rt).exp() * 0.5));
                    slots[2] = knob("ROOM SIZE", f("size"), 0.0, 1.0, d(0.2, 0.1), pct(v(f("size"), d(0.2, 0.1))));
                    slots[3] = knob("DAMPING", f("damp"), 0.0, 1.0, 0.5, pct(v(f("damp"), 0.5)));
                    slots[4] = knob("MODULATION", f("mod"), 0.0, 1.0, d(0.2, 0.4), pct(v(f("mod"), d(0.2, 0.4))));
                    let diff = if hall { "SOFTNESS" } else { "DIFFUSION" };
                    slots[5] = knob(diff, f("diff"), 0.0, 1.0, d(0.8, 0.9), pct(v(f("diff"), d(0.8, 0.9))));
                    slots[6] = knob("PRE DELAY", f("pre_delay"), 0.0, 1.0, 0.0, format!("{:.1} ms", 250.0 * v(f("pre_delay"), 0.0)));
                }
                slots[7] = knob("MIX", "mix", 0.0, 1.0, 0.5, pct(v("mix", 0.5)));
            }
        }
        ("CabinetEmulation", _) => {
            sections.extend([(0, "Cabinet"), (1, "Microphone"), (3, "Output")]);
            let c = (v("cabinet", 0.0).max(0.0) as usize).min(3);
            slots[0] = Some(("CABINET".into(), CABINETS[c].into(), 0.0, Some(Edit::Choice(t, "cabinet", &CABINETS, c, 0.0))));
            let m = (v("microphone", 0.0).max(0.0) as usize).min(5);
            slots[1] = Some(("MICROPHONE".into(), MICROPHONES[m].into(), 0.0, Some(Edit::Choice(t, "microphone", &MICROPHONES, m, 0.0))));
            slots[2] = knob("DISTANCE", "distance", 0.0, 1.0, 0.0, pct(v("distance", 0.0)));
            slots[3] = knob("MIX", "mix", 0.0, 1.0, 1.0, pct(v("mix", 1.0)));
        }
        ("MullhollandDrive", p) => {
            // Projects from before Analog mode (no Mode stored) are Mulholland.
            let mode = (v("mode", 0.0).max(0.0) as usize).min(1);
            let toggle = |label: &'static str, field: &'static str, d: f64| -> Option<Knob> {
                let on = v(field, d) != 0.0;
                Some((label.into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, field, on))))
            };
            let bi = |label: &'static str, field: &'static str, d: f64| {
                knob(label, field, -1.0, 1.0, d, format!("{:.1} %", 100.0 * v(field, d)))
            };
            if p == 1 {
                if mode == 0 {
                    sections.push((0, "Gate"));
                    slots[0] = toggle("GATE", "gate", 1.0);
                    let r = v("release", 0.5);
                    let secs = 4.99 * (r * r + 0.8096 * (r.powi(4) - r * r)) + 0.01;
                    slots[1] = knob("RELEASE", "release", 0.0, 1.0, 0.5, format!("{:.1} ms", 1000.0 * secs));
                }
            } else {
                sections.extend([(0, "Mode"), (1, "Distortion"), (7, "Output")]);
                slots[0] =
                    Some(("MODE".into(), DISTORTION_MODES[mode].into(), 0.0, Some(Edit::Choice(t, "mode", &DISTORTION_MODES, mode, 1.0))));
                if mode == 0 {
                    slots[1] = knob("DRIVE", "drive", 0.0, 1.0, 0.25, pct(v("drive", 0.25)));
                    slots[2] = knob("COLOR", "color", 0.0, 1.0, 0.25, pct(v("color", 0.25)));
                    slots[3] = bi("FEEDBACK", "feedback", 0.25);
                    slots[4] = knob("TONE", "tone", 0.0, 1.0, 0.75, pct(v("tone", 0.75)));
                    slots[5] = bi("TONE MOD", "tone_mod", 0.0);
                    slots[7] = knob("MIX", "mix", 0.0, 1.0, 0.5, pct(v("mix", 0.5)));
                } else {
                    slots[1] = knob("SATURATION", "analog_saturation", 0.0, 1.0, 0.25, pct(v("analog_saturation", 0.25)));
                    slots[2] = knob("BASS", "analog_bass", 0.0, 1.0, 1.0, pct(v("analog_bass", 1.0)));
                    slots[3] = knob("TONE", "analog_tone", 0.0, 1.0, 1.0, pct(v("analog_tone", 1.0)));
                    slots[4] = toggle("FOCUS", "analog_focus", 0.0);
                    slots[5] = knob("DEFINITION", "analog_definition", 0.0, 1.0, 0.5, pct(v("analog_definition", 0.5)));
                    slots[7] = knob("MIX", "analog_mix", 0.0, 1.0, 1.0, pct(v("analog_mix", 1.0)));
                }
            }
        }
        ("Resochord", _) => {
            sections.extend([(0, "Pitch"), (4, "Color"), (7, "Output")]);
            let mode = (v("mode", 1.0).max(0.0) as usize).min(1);
            slots[0] = Some(("MODE".into(), RESOCHORD_MODES[mode].into(), 0.0, Some(Edit::Choice(t, "mode", &RESOCHORD_MODES, mode, 1.0))));
            if mode == 0 {
                slots[1] = knob("SPREAD", "spread", 0.0, 1.0, 0.5, pct(v("spread", 0.5)));
            } else {
                let st = (v("style", 0.0).max(0.0) as usize).min(4);
                slots[1] =
                    Some(("STYLE".into(), RESOCHORD_STYLES[st].into(), 0.0, Some(Edit::Choice(t, "style", &RESOCHORD_STYLES, st, 0.0))));
                let ch = (v("chord", 1.0).max(0.0) as usize).min(4);
                slots[2] =
                    Some(("CHORD".into(), RESOCHORD_CHORDS[ch].into(), 0.0, Some(Edit::Choice(t, "chord", &RESOCHORD_CHORDS, ch, 1.0))));
            }
            let tune = v("tune", 0.0);
            slots[3] =
                Some(("TUNE".into(), format!("{tune:.0}"), ((tune + 24.0) / 48.0) as f32, Some(Edit::Int(t, "tune", -24.0, 24.0, 0.0))));
            for (k, label, field, d) in [
                (4, "BRIGHTNESS", "brightness", 0.75),
                (5, "FEEDBACK", "feedback", 0.75),
                (6, "DECAY", "decay", 0.5),
                (7, "MIX", "mix", 0.75),
            ] {
                slots[k] = knob(label, field, 0.0, 1.0, d, pct(v(field, d)));
            }
        }
        ("GrainStretch", _) => {
            sections.extend([(0, "Master"), (1, "Time"), (3, "Pitch"), (6, "Output")]);
            let toggle = |label: &'static str, field: &'static str| -> Option<Knob> {
                let on = v(field, 0.0) != 0.0;
                Some((label.into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, field, on))))
            };
            slots[0] = toggle("ON", "on");
            slots[1] = knob("STRETCH", "stretch", 0.0, 1.0, 0.0, pct(v("stretch", 0.0)));
            let lp = (v("loop_length", 8.0).max(0.0) as usize).min(8);
            slots[2] = Some(("LOOP".into(), STRETCH_LOOPS[lp].into(), 0.0, Some(Edit::Choice(t, "loop_length", &STRETCH_LOOPS, lp, 8.0))));
            slots[3] = knob("PITCH", "pitch", 0.0, 1.0, 0.5, format!("{:.1} %", 200.0 * v("pitch", 0.5)));
            slots[4] = toggle("LINK", "link");
            slots[5] = knob("SIZE", "size", 0.0, 1.0, 0.25, pct(v("size", 0.25)));
            slots[6] = knob("MIX", "mix", 0.0, 1.0, 1.0, pct(v("mix", 1.0)));
        }
        ("GrainDelay", 1) => {
            sections.push((0, "Output"));
            slots[0] = knob("STEREO", "stereo", 0.0, 1.0, 0.75, pct(v("stereo", 0.75)));
        }
        ("GrainDelay", _) => {
            sections.extend([(0, "Grain"), (4, "Cloud")]);
            slots[0] = knob("PITCH", "pitch", -1.0, 1.0, 0.0, format!("{:.1}", 48.0 * v("pitch", 0.0)));
            for (k, label, field, d) in [
                (1, "SIZE", "size", 0.25),
                (2, "JITTER", "jitter", 0.0),
                (4, "SPACE", "space", 0.0),
                (5, "DENSITY", "density", 0.25),
                (6, "MOD", "modulation", 0.0),
                (7, "MIX", "mix", 0.75),
            ] {
                slots[k] = knob(label, field, 0.0, 1.0, d, pct(v(field, d)));
            }
            let on = v("reverse", 0.0) != 0.0;
            slots[3] =
                Some(("REVERSE".into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, "reverse", on))));
        }
        ("FM", _) => {
            sections.extend([(0, "Freq"), (2, "Depth")]);
            for (k, label, field, d) in
                [(0, "RATE", "rate", 0.25), (1, "SPLIT", "split", 0.75), (2, "CONTOUR", "contour", 0.75), (3, "AMOUNT", "amount", 0.75)]
            {
                slots[k] = knob(label, field, 0.0, 1.0, d, pct(v(field, d)));
            }
        }
        ("FreqShifter", _) => {
            sections.extend([(0, "Freq"), (2, "Output")]);
            for (k, label, field, d) in [
                (0, "COARSE", "coarse", 0.5),
                (1, "FINE", "fine", 0.0),
                (2, "FEEDBACK", "feedback", 0.75),
                (3, "STEREO", "stereo", 0.75),
                (5, "MIX", "mix", 0.75),
            ] {
                slots[k] = knob(label, field, 0.0, 1.0, d, pct(v(field, d)));
            }
            let on = v("invert", 1.0) != 0.0;
            slots[4] = Some(("INVERT".into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, "invert", on))));
        }
        ("TransientDesigner", _) => {
            let g = v("input_gain", 0.0);
            slots[0] = knob("INPUT GAIN", "input_gain", -1.0, 1.0, 0.0, format!("{:.1} dB", g * if g < 0.0 { 24.0 } else { 6.0 }));
            slots[1] = knob("ATTACK", "attack", -1.0, 1.0, 0.0, format!("{:.1} %", 100.0 * v("attack", 0.0)));
            slots[2] = knob("SUSTAIN", "sustain", -1.0, 1.0, 0.0, format!("{:.1} %", 100.0 * v("sustain", 0.0)));
            let on = v("limit", 1.0) != 0.0;
            slots[3] = Some(("LIMIT".into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, "limit", on))));
        }
        ("Flanger" | "Phaser", _) => {
            // The two share a layout: Invert on the Flanger is 8 Pole on the Phaser.
            let phaser = slot.module == "Phaser";
            let (amount_d, source_d) = if phaser { (0.5, 1.0) } else { (0.75, 0.0) };
            sections.extend([(0, "Main"), (3, "Mod"), (7, "Output")]);
            slots[0] = knob("FREQUENCY", "frequency", 0.0, 1.0, 0.5, pct(v("frequency", 0.5)));
            slots[1] = knob("FEEDBACK", "feedback", 0.0, 1.0, 0.5, pct(v("feedback", 0.5)));
            let (switch, sw) = if phaser { ("8POLE", "eight_pole") } else { ("INVERT", "invert") };
            let on = v(sw, 0.0) != 0.0;
            slots[2] = Some((switch.into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, sw, on))));
            slots[3] = knob("AMOUNT", "amount", -1.0, 1.0, amount_d, format!("{:.1} %", 100.0 * v("amount", amount_d)));
            let src = (v("source", source_d).max(0.0) as usize).min(2);
            slots[4] = Some(("SOURCE".into(), MOD_SOURCES[src].into(), 0.0, Some(Edit::Choice(t, "source", &MOD_SOURCES, src, source_d))));
            slots[5] = if src == 2 {
                knob("SHAPE", "shape", 0.0, 1.0, 0.5, pct(v("shape", 0.5)))
            } else {
                knob("SPEED", "speed", 0.0, 1.0, 0.5, mod_speed(v("speed", 0.5), src == 1))
            };
            slots[6] = knob("STEREO", "stereo", 0.0, 1.0, 0.5, pct(v("stereo", 0.5)));
            slots[7] = knob("MIX", "mix", 0.0, 1.0, 0.5, pct(v("mix", 0.5)));
        }
        ("Reverb", _) => {
            sections.extend([(0, "Room"), (2, "EQ"), (4, "Position"), (6, "Output")]);
            let room = (v("room", 0.0).max(0.0) as usize).min(3);
            slots[0] = Some(("ROOM".into(), REVERB_ROOMS[room].into(), 0.0, Some(Edit::Choice(t, "room", &REVERB_ROOMS, room, 0.0))));
            for (k, label, field) in [(1, "SIZE", "size"), (2, "LOW", "low"), (3, "HIGH", "high"), (7, "MIX", "mix")] {
                slots[k] = knob(label, field, 0.0, 1.0, 0.5, pct(v(field, 0.5)));
            }
            slots[4] = knob("PAN", "pan", -1.0, 1.0, 0.0, crate::layout::pan(v("pan", 0.0)));
            slots[5] = knob("STEREO", "stereo", -1.0, 1.0, 0.0, format!("{:.1} %", v("stereo", 0.0) * 100.0));
            let on = v("freeze", 0.0) != 0.0;
            slots[6] = Some(("FREEZE".into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, "freeze", on))));
        }
        ("Maximizer", _) => {
            sections.push((0, "Depth"));
            slots[0] = knob("AMOUNT", "amount", 0.0, 1.0, 0.5, pct(v("amount", 0.5)));
            slots[1] = knob("CURVE", "curve", 0.0, 1.0, 0.5, pct(v("curve", 0.5)));
            let on = v("turbo", 0.0) != 0.0;
            slots[2] = Some(("TURBO".into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, "turbo", on))));
        }
        ("Filter", _) => {
            let mode = (v("mode", 0.0).max(0.0) as usize).min(3);
            let source = (v("source", 0.0).max(0.0) as usize).min(2);
            sections.extend([(0, "Type"), (1, "Freq"), (3, "Mod")]);
            slots[0] = Some(("MODE".into(), FILTER_MODES[mode].into(), 0.0, Some(Edit::Choice(t, "mode", &FILTER_MODES, mode, 0.0))));
            let cut_default = 0.549_053_7;
            // In f32, as Maschine's converter computes it (the default shows 1.0 kHz).
            let hzv = (46f32 * (12540f32 / 46.0).powf(v("cutoff", cut_default) as f32)).clamp(46.0, 12540.0) as f64;
            slots[1] = knob("CUTOFF", "cutoff", 0.0, 1.0, cut_default, filter_hz(hzv));
            if mode != 3 {
                slots[2] = knob("RESONANCE", "resonance", 0.0, 1.0, 0.5, pct(v("resonance", 0.5)));
            }
            slots[3] = knob("AMOUNT", "amount", -1.0, 1.0, 0.0, pct(v("amount", 0.0)));
            slots[4] =
                Some(("SOURCE".into(), FILTER_SOURCES[source].into(), 0.0, Some(Edit::Choice(t, "source", &FILTER_SOURCES, source, 0.0))));
            if source == 2 {
                slots[5] = knob("DECAY", "decay", 0.0, 1.0, 0.5, pct(v("decay", 0.5)));
                slots[6] = knob("SMOOTH", "smooth", 0.0, 1.0, 0.5, pct(v("smooth", 0.5)));
                slots[7] = knob("SHAPE", "env_shape", -1.0, 1.0, 0.0, pct(v("env_shape", 0.0)));
            } else {
                let sp = v("speed", 0.5);
                let bars = |i: usize| FILTER_RATES[i].0 as f64 / FILTER_RATES[i].1 as f64;
                let text = if source == 1 {
                    let (n, d) = FILTER_RATES[((11.0 * sp).round() as usize).min(11)];
                    format!("{n}/{d}")
                } else {
                    let pos = 11.0 * sp;
                    let i = ((pos - 0.5).round() as usize).min(10);
                    let f = pos - i as f64;
                    filter_hz(1.0 / (2.0 * ((1.0 - f) * bars(i) + f * bars(i + 1))))
                };
                slots[5] = knob("SPEED", "speed", 0.0, 1.0, 0.5, text);
                slots[6] = knob("LFO SHAPE", "lfo_shape", 0.0, 1.0, 0.5, pct(v("lfo_shape", 0.5)));
                if source == 1 {
                    slots[7] = knob("PHASE", "phase", -1.0, 1.0, 0.0, pct(v("phase", 0.0)));
                }
            }
        }
        ("BeatDelay", 1) => {
            sections.push((0, "Unit"));
            let u = (v("unit", 0.0).max(0.0) as usize).min(2);
            slots[0] = Some(("UNIT".into(), DELAY_UNITS[u].into(), 0.0, Some(Edit::Choice(t, "unit", &DELAY_UNITS, u, 0.0))));
        }
        ("BeatDelay", _) => {
            sections.extend([(0, "Delay"), (6, "Output")]);
            let time = (v("time", 3.0).max(0.0) as usize).min(16);
            slots[0] = Some(("TIME".into(), DELAY_TIMES[time].into(), 0.0, Some(Edit::Choice(t, "time", &DELAY_TIMES, time, 3.0))));
            let off = (v("offset", 0.0).max(0.0) as usize).min(8);
            slots[1] = Some(("OFFSET".into(), DELAY_OFFSETS[off].into(), 0.0, Some(Edit::Choice(t, "offset", &DELAY_OFFSETS, off, 0.0))));
            for (k, label, field, d) in [
                (2, "FEEDBACK", "feedback", 0.5),
                (3, "CROSSOVER", "crossover", 0.25),
                (4, "COLOR", "color", 0.5),
                (5, "SPLIT", "split", 0.25),
            ] {
                slots[k] = knob(label, field, 0.0, 1.0, d, pct(v(field, d)));
            }
            slots[6] = knob("STEREO", "stereo", -1.0, 1.0, 0.0, pct(v("stereo", 0.0)));
            slots[7] = knob("MIX", "mix", 0.0, 1.0, 0.5, pct(v("mix", 0.5)));
        }
        ("Saturator", _) => {
            let mode = (v("mode", 0.0).max(0.0) as usize).min(2);
            sections.push((0, "Main"));
            slots[0] = Some(("MODE".into(), SATURATOR_MODES[mode].into(), 0.0, Some(Edit::Choice(t, "mode", &SATURATOR_MODES, mode, 0.0))));
            let toggle = |label: &'static str, field: &'static str| -> Option<Knob> {
                let on = v(field, 0.0) != 0.0;
                Some((label.into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, field, on))))
            };
            match mode {
                0 => {
                    slots[1] = knob("INPUT", "classic_input", 0.0, 1.0, 0.25, pct(v("classic_input", 0.25)));
                    slots[2] = knob("CONTOUR", "classic_contour", 0.0, 1.0, 0.75, pct(v("classic_contour", 0.75)));
                    slots[3] = knob("DRIVE", "classic_drive", 0.0, 1.0, 0.5, pct(v("classic_drive", 0.5)));
                }
                1 => {
                    slots[1] = knob("INPUT", "tape_input", 0.0, 1.0, 0.2, format!("{:.1} dB", 30.0 * v("tape_input", 0.2)));
                    slots[2] = knob("CONTOUR", "tape_contour", 0.0, 1.0, 1.0, format!("{:.1} k", 8.0 * v("tape_contour", 1.0) + 12.0));
                    slots[3] = knob("DRIVE", "tape_drive", -1.0, 1.0, 0.34, format!("{:.1} dB", 3.0 * v("tape_drive", 0.34)));
                }
                _ => {
                    sections.extend([(4, "EQ"), (7, "Output")]);
                    slots[1] = knob("CHARGE", "tube_charge", 0.0, 4.0, 2.0, format!("{:.1} %", 25.0 * v("tube_charge", 2.0)));
                    slots[2] = toggle("OVERLOAD", "tube_overload");
                    slots[3] = knob("DRIVE", "tube_drive", 0.0, 1.0, 0.25, pct(v("tube_drive", 0.25)));
                    slots[4] = toggle("BYPASS", "tube_eq_bypass");
                    slots[5] = knob("BASS", "tube_bass", 0.0, 1.0, 0.8, pct(v("tube_bass", 0.8)));
                    slots[6] = knob("TREBLE", "tube_treble", 0.0, 1.0, 0.8, pct(v("tube_treble", 0.8)));
                    slots[7] = knob("GAIN", "tube_gain", -12.0, 12.0, 0.0, format!("{:.1} dB", v("tube_gain", 0.0)));
                }
            }
        }
        ("BassSynth", 0) => {
            // docs/synth-bass-synth.md.
            sections.extend([(0, "Osc1"), (2, "Osc2"), (5, "Osc Mix"), (6, "Filter")]);
            for (k, label, field) in [(0, "SHAPE", "shape"), (2, "SHAPE 2", "shape_2")] {
                slots[k] = knob(label, field, 0.0, 1.0, 0.66, format!("{:.2}", v(field, 0.66)));
            }
            for (k, label, field) in [(1, "OCTAVE", "octave"), (3, "OCTAVE 2", "octave_2")] {
                let o = v(field, -2.0).round();
                slots[k] = Some((label.into(), format!("{o:.0}"), ((o + 3.0) / 6.0) as f32, Some(Edit::Int(t, field, -3.0, 3.0, -2.0))));
            }
            slots[4] = knob("DETUNE", "detune", 0.0, 1.0, 0.0, format!("{:.2}", v("detune", 0.0)));
            slots[5] = knob("OSC MIX", "osc_mix", 0.0, 1.0, 0.0, format!("{:.2}", v("osc_mix", 0.0)));
            let c = v("cutoff", 0.2);
            slots[6] = knob("CUTOFF", "cutoff", 0.0, 1.0, 0.2, lofi_hz(8.175_799 * 2f64.powf((10.0 + 128.0 * c) / 12.0)));
            slots[7] = knob("RESONANCE", "resonance", 0.0, 1.0, 0.75, pct(1.2 * v("resonance", 0.75)));
        }
        ("BassSynth", 1) => {
            sections.extend([(0, "Envelope"), (2, "Tone"), (3, "Note")]);
            slots[0] = knob("MOD. AMT.", "mod_amount", 0.0, 1.0, 0.5, pct(v("mod_amount", 0.5)));
            slots[1] = knob("DECAY", "decay", 0.0, 1.0, 0.5, pct(v("decay", 0.5)));
            slots[2] = knob("DRIVE", "drive", -1.0, 1.0, 0.0, pct(v("drive", 0.0)));
            let gt = v("glide_time", 0.090_909_1);
            slots[3] = knob("GLIDE TIME", "glide_time", 0.0, 1.0, 0.090_909_1, format!("{:.0} ms", 10.0 + 990.0 * gt));
            let gl = v("glide", 0.0).round();
            slots[4] = Some(("GLIDE".into(), format!("{gl:.0}"), gl as f32, Some(Edit::Int(t, "glide", 0.0, 1.0, 0.0))));
        }
        ("PolySynth", _) => {
            // docs/synth-poly-synth.md §1.3.
            let toggle = |label: &'static str, field: &'static str, d: f64| -> Option<Knob> {
                let on = v(field, d) != 0.0;
                Some((label.into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, field, on))))
            };
            let choice = |label: &'static str, field: &'static str, options: &'static [&'static str], d: f64| -> Option<Knob> {
                let cur = (v(field, d).round().max(0.0) as usize).min(options.len() - 1);
                Some((
                    label.into(),
                    options[cur].into(),
                    cur as f32 / (options.len() - 1) as f32,
                    Some(Edit::Choice(t, field, options, cur, d)),
                ))
            };
            let p01 = |label: &'static str, field: &'static str, d: f64| knob(label, field, 0.0, 1.0, d, pct(v(field, d)));
            let semis = |label: &'static str, field: &'static str, lo: f64, hi: f64, d: f64| -> Option<Knob> {
                let x = v(field, d).round();
                Some((label.into(), format!("{x:+.0}"), ((x - lo) / (hi - lo)) as f32, Some(Edit::Int(t, field, lo, hi, d))))
            };
            let row: [Option<Knob>; 8] = match page {
                0 => {
                    sections.push((0, "Filter"));
                    [
                        p01("CUTOFF", "filter_cutoff", 1.0),
                        p01("RESONANCE", "filter_resonance", 0.0),
                        p01("ENV AMOUNT", "filter_env_amount", 0.0),
                        toggle("INVERT", "filter_invert", 0.0),
                        p01("KBD", "filter_kbd", 0.0),
                        choice("MODE", "filter_mode", &POLY_FILTER_MODES, 0.0),
                        None,
                        None,
                    ]
                }
                1 => {
                    sections.extend([(0, "Filter Envelope"), (4, "Amp Envelope")]);
                    [
                        p01("ATTACK", "filter_attack", 0.0),
                        p01("DECAY", "filter_decay", 0.0),
                        p01("SUSTAIN", "filter_sustain", 1.0),
                        p01("RELEASE", "filter_release", 0.0),
                        p01("ATTACK", "amp_attack", 0.0),
                        p01("DECAY", "amp_decay", 0.0),
                        p01("SUSTAIN", "amp_sustain", 1.0),
                        p01("RELEASE", "amp_release", 0.0),
                    ]
                }
                2 => {
                    sections.extend([(0, "Mix"), (4, "Voices")]);
                    [
                        p01("OSC A", "mixer_osc_a", 1.0),
                        p01("OSC B", "mixer_osc_b", 0.0),
                        p01("NOISE", "mixer_noise", 0.0),
                        None,
                        p01("GLIDE", "glide", 0.0),
                        choice("MODE", "voice_mode", &POLY_VOICE_MODES, 0.0),
                        choice("VOICES", "voices", &POLY_VOICES, 5.0),
                        p01("DETUNE", "detune", 0.0),
                    ]
                }
                3 => {
                    sections.extend([(0, "Oscillator A"), (4, "Shapes")]);
                    [
                        semis("TUNE", "osc_a_tune", -24.0, 24.0, 0.0),
                        toggle("SYNC", "osc_a_sync", 0.0),
                        None,
                        None,
                        toggle("SAW", "osc_a_saw", 1.0),
                        toggle("PULSE", "osc_a_pulse", 0.0),
                        p01("PULSE WIDTH", "osc_a_pulse_width", 0.5),
                        None,
                    ]
                }
                4 => {
                    sections.extend([(0, "Oscillator B"), (4, "Shapes")]);
                    [
                        semis("TUNE", "osc_b_tune", -24.0, 24.0, 0.0),
                        p01("FINE", "osc_b_fine", 0.0),
                        toggle("LOW", "osc_b_low", 0.0),
                        toggle("KBD", "osc_b_kbd", 1.0),
                        toggle("SAW", "osc_b_saw", 1.0),
                        toggle("TRIANGLE", "osc_b_triangle", 0.0),
                        toggle("PULSE", "osc_b_pulse", 0.0),
                        p01("PULSE WIDTH", "osc_b_pulse_width", 0.5),
                    ]
                }
                5 => {
                    sections.extend([(0, "LFO"), (4, "Shapes")]);
                    [
                        p01("RATE", "lfo_rate", 0.0),
                        choice("MODE", "lfo_mode", &POLY_LFO_MODES, 0.0),
                        toggle("VOICE TRIG", "lfo_voice_trig", 0.0),
                        None,
                        toggle("SAW", "lfo_saw", 0.0),
                        toggle("TRIANGLE", "lfo_triangle", 1.0),
                        toggle("PULSE", "lfo_pulse", 0.0),
                        None,
                    ]
                }
                6 => {
                    sections.extend([(0, "Modulation"), (2, "Destinations")]);
                    [
                        p01("AMOUNT", "mod_amount", 0.0),
                        p01("LFO<>NOISE", "mod_lfo_noise", 0.0),
                        toggle("FREQ A", "mod_freq_a", 0.0),
                        toggle("FREQ B", "mod_freq_b", 0.0),
                        toggle("PW A", "mod_pw_a", 0.0),
                        toggle("PW B", "mod_pw_b", 0.0),
                        toggle("FILTER", "mod_filter", 0.0),
                        None,
                    ]
                }
                7 => {
                    sections.extend([(0, "Polymod Source"), (2, "Destinations")]);
                    [
                        p01("FILTER ENV", "pm_filter_env", 0.0),
                        p01("OSC B", "pm_osc_b", 0.0),
                        toggle("FREQ A", "pm_freq_a", 0.0),
                        toggle("PW A", "pm_pw_a", 0.0),
                        toggle("FILTER", "pm_filter", 0.0),
                        None,
                        None,
                        None,
                    ]
                }
                _ => {
                    sections.push((0, "Global"));
                    let tune = v("tune", 0.0);
                    [
                        Some((
                            "TUNE".into(),
                            format!("{:+.2}", tune * 0.7),
                            ((tune + 1.0) / 2.0) as f32,
                            Some(Edit::Param(t, "tune", -1.0, 1.0, 0.0)),
                        )),
                        semis("PITCHBEND", "pitchbend_range", 0.0, 24.0, 2.0),
                        toggle("VELOCITY", "velocity", 0.0),
                        toggle("RELEASE", "release", 1.0),
                        toggle("HOLD", "hold", 0.0),
                        None,
                        None,
                        None,
                    ]
                }
            };
            slots = row;
        }
        ("Chorus", _) => {
            sections.extend([(0, "Mod"), (2, "Output")]);
            for (k, label, field) in [(0, "RATE", "rate"), (1, "AMOUNT", "amount"), (2, "MIX", "mix")] {
                slots[k] = knob(label, field, 0.0, 1.0, 0.5, pct(v(field, 0.5)));
            }
        }
        _ => {}
    }
    Page { slots, sections }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The knob labels of `page` for a module with these parameters.
    fn labels(module: &str, params: &[(&str, f64)], n: usize) -> Vec<String> {
        let slot = SlotInfo {
            module: module.into(),
            name: module.into(),
            active: true,
            plugin: None,
            params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        };
        page(&slot, 0, 0, 1, n).slots.iter().map(|k| k.as_ref().map_or(String::new(), |k| k.0.to_string())).collect()
    }

    /// The Poly Synth's nine pages (docs/synth-poly-synth.md §1.3).
    #[test]
    fn poly_synth_pages() {
        assert_eq!(pages("PolySynth").unwrap().len(), 9);
        assert_eq!(labels("PolySynth", &[], 0)[..6], ["CUTOFF", "RESONANCE", "ENV AMOUNT", "INVERT", "KBD", "MODE"]);
        assert_eq!(labels("PolySynth", &[], 2)[4..], ["GLIDE", "MODE", "VOICES", "DETUNE"]);
        assert_eq!(labels("PolySynth", &[], 8)[..5], ["TUNE", "PITCHBEND", "VELOCITY", "RELEASE", "HOLD"]);
        let slot = SlotInfo { module: "PolySynth".into(), name: String::new(), active: true, plugin: None, params: Vec::new() };
        assert_eq!(page(&slot, 0, 0, 1, 2).slots[6].as_ref().unwrap().1, "6", "five is six voices");
    }

    /// Perform FX: Main and Setup, following the mode (manual pp. 381–390).
    #[test]
    fn perform_fx_pages_follow_the_mode() {
        assert_eq!(pages("PerformanceFX").unwrap(), ["Main", "Setup"]);
        let pf = |mode: f64, page: usize| labels("PerformanceFX", &[("mode", mode)], page);
        assert_eq!(pf(0.0, 0)[..5], ["MODE", "ENGAGE", "FREQUENCY", "TYPE", "RESONANCE"]);
        assert_eq!(pf(0.0, 1)[..4], ["MIN. FREQ", "MAX. FREQ", "SATURATION", "OUT LEVEL"]);
        assert_eq!(pf(1.0, 0)[3], "DECAY");
        assert!(pf(1.0, 1).contains(&"POLARITY".to_string()) && pf(1.0, 1).contains(&"TONE".to_string()));
        assert_eq!(pf(2.0, 0)[3], "TS ASSIGN");
        assert_eq!(pf(5.0, 0)[2], "LENGTH", "the strip's knob is named for what it sets");
        assert_eq!(labels("PerformanceFX", &[("mode", 5.0), ("stutter_ts_assign", 1.0)], 0)[2], "PITCH");
        assert_eq!(pf(6.0, 1)[..4], ["DEPTH MIN", "DEPTH MAX", "RATE MIN", "RATE MAX"]);
        assert_eq!(pf(7.0, 0)[2], "POSITION");
    }

    #[test]
    fn tier_one_pages() {
        assert_eq!(labels("BeatDelay", &[], 0), ["TIME", "OFFSET", "FEEDBACK", "CROSSOVER", "COLOR", "SPLIT", "STEREO", "MIX"]);
        assert_eq!(labels("BeatDelay", &[], 1)[0], "UNIT");
        assert_eq!(labels("Chorus", &[], 0)[..3], ["RATE", "AMOUNT", "MIX"]);
        assert_eq!(labels("Maximizer", &[], 0)[..3], ["AMOUNT", "CURVE", "TURBO"]);
        assert_eq!(labels("Maximizer", &[], 1)[1], "GAIN", "the side-chain page");
        // The Filter's last knobs follow its modulation source; Notch has no resonance.
        assert_eq!(labels("Filter", &[], 0), ["MODE", "CUTOFF", "RESONANCE", "AMOUNT", "SOURCE", "SPEED", "LFO SHAPE", ""]);
        assert_eq!(labels("Filter", &[("source", 1.0)], 0)[7], "PHASE");
        assert_eq!(
            labels("Filter", &[("source", 2.0), ("mode", 3.0)], 0),
            ["MODE", "CUTOFF", "", "AMOUNT", "SOURCE", "DECAY", "SMOOTH", "SHAPE"]
        );
        // The Saturator's knobs follow its mode.
        assert_eq!(labels("Saturator", &[], 0)[..4], ["MODE", "INPUT", "CONTOUR", "DRIVE"]);
        assert_eq!(labels("Saturator", &[("mode", 2.0)], 0), ["MODE", "CHARGE", "OVERLOAD", "DRIVE", "BYPASS", "BASS", "TREBLE", "GAIN"]);
        let slot =
            SlotInfo { module: "BeatDelay".into(), name: String::new(), active: true, plugin: None, params: vec![("time".into(), 0.0)] };
        assert_eq!(page(&slot, 0, 0, 1, 0).slots[0].as_ref().unwrap().1, "1/32");
        let slot = SlotInfo { module: "Filter".into(), name: String::new(), active: true, plugin: None, params: Vec::new() };
        assert_eq!(page(&slot, 0, 0, 1, 0).slots[1].as_ref().unwrap().1, "1.0 kHz", "the default cutoff shows 1 kHz");
    }
}
