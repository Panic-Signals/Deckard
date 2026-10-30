//! The parameter pages of Maschine's drum synths (Kick, Snare, Hi-hat, Tom, Perc, Cymbal):
//! which parameter sits on each of the 24 knob positions (pages Main, Advanced,
//! Modulation, 8 each) for the engine chosen, with Maschine's labels, sections and value
//! texts (docs/synth-*.md §1; the Kick's from `Kick::getParameterByPosition`).

use crate::layout::{Edit, Knob};
use crate::sampler_ui::Page;
use deckard_doc::Target;
use deckard_model::SlotInfo;

pub const DRUMS: [&str; 6] = ["Kick", "Snare", "Hihat", "Tom", "Perc", "Cymbal"];

/// The pages every drum synth has (`DrumSynth::getPagesNamesStatic`).
pub const PAGES: [&str; 3] = ["Main", "Advanced", "Modulation"];

pub const KICK_ENGINES: [&str; 9] = ["Sub", "Tronic", "Dusty", "Grit", "Rasper", "Snappy", "Bold", "Maple", "Push"];
pub const SNARE_ENGINES: [&str; 10] = ["Volt", "Bit", "Pow", "Sharp", "Airy", "Vintage", "Chrome", "Iron", "Clap", "Breaker"];
pub const HIHAT_ENGINES: [&str; 4] = ["Silver", "Circuit", "Memory", "Hybrid"];
pub const TOM_ENGINES: [&str; 4] = ["Tronic", "Fractal", "Floor", "High"];
pub const PERC_ENGINES: [&str; 3] = ["Fractal", "Kettle", "Shaker"];
pub const CYMBAL_ENGINES: [&str; 2] = ["Crash", "Ride"];
const AB: [&str; 2] = ["A", "B"];
const THIN_THICK: [&str; 2] = ["Thin", "Thick"];
const OSC_MODES: [&str; 2] = ["Tonal", "Punchy"];
const SOURCES: [&str; 6] = ["A", "B", "C", "D", "E", "F"];
const KEYTRACK: [&str; 2] = ["Harmonic", "Dissonant"];
const SHAKER_MODES: [&str; 3] = ["Machine", "Realistic", "Performer"];
const SHAKER_FILLS: [&str; 3] = ["Triple", "None", "Double"];
const SHAKER_RATES: [&str; 7] = ["1/4", "1/4T", "1/8", "1/8T", "1/16", "1/16T", "1/32"];
const SHAKER_LENGTHS: [&str; 8] = ["1", "2", "3", "4", "5", "6", "7", "8"];
const SHAKER_SYNCS: [&str; 2] = ["Retrig", "Lock"];
const CRASH_MODES: [&str; 3] = ["Metallic", "Normal", "Soft"];
/// The Hi-hat's Seed: one character per step (`getSeedValueString`).
const SEEDS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ!?&#*";

/// How a parameter's value is kept and shown.
#[derive(Clone, Copy)]
enum Kind {
    /// 0..1, shown as a percentage.
    Pct,
    /// −1..1, shown as a percentage (Velocity).
    Bipolar,
    /// 0..1 shown as −100 %…+100 % (`(2v − 1)·100`: Bend).
    Bend,
    /// One of these options.
    Choice(&'static [&'static str]),
    /// On or off.
    Toggle,
    /// The Tune text of the engine (a note number or a percentage).
    Tune,
    /// The Hi-hat's Seed (a character).
    Seed,
    /// The Hi-hat's Color: the band-pass frequency for Silver and Circuit.
    Color,
    /// "None" at 0, else a percentage (Glide; Maschine shows a time).
    Glide,
    /// `"%.2f"` of `a·v + b`.
    Num(f64, f64),
}

/// A drum synth parameter: its field, Maschine's label, the section it starts, its default.
struct P {
    field: &'static str,
    label: &'static str,
    section: Option<&'static str>,
    default: f64,
    kind: Kind,
}

const fn p(field: &'static str, label: &'static str, default: f64, kind: Kind) -> P {
    P { field, label, section: None, default, kind }
}

const fn s(field: &'static str, label: &'static str, section: &'static str, default: f64, kind: Kind) -> P {
    P { field, label, section: Some(section), default, kind }
}

const KICK: &[P] = &[
    s("engine", "ENGINE", "Main", 0.0, Kind::Choice(&KICK_ENGINES)),
    p("tune", "TUNE", 0.5, Kind::Tune),
    p("decay", "DECAY", 0.5, Kind::Pct),
    p("punch", "PUNCH", 1.0, Kind::Pct),
    p("bend", "BEND", 0.068, Kind::Pct),
    p("impact", "IMPACT", 0.75, Kind::Pct),
    s("velocity", "VELOCITY", "Scale", 1.0, Kind::Bipolar),
    p("sub_kick_p_env_decay", "TIME", 0.3, Kind::Pct),
    s("sub_kick_thin_attack", "ATTACK", "Attack", 0.0, Kind::Choice(&THIN_THICK)),
    p("sub_kick_color", "COLOR", 0.5, Kind::Pct),
    p("sub_kick_attack_amount", "AMOUNT", 0.5, Kind::Pct),
    s("clean_bd_drive_gain", "GAIN", "Distortion", 0.7, Kind::Pct),
    p("clean_bd_drive_tone", "TONE", 0.25, Kind::Pct),
    p("clean_bd_bias", "BIAS", 0.5, Kind::Pct),
    p("clean_bd_drive_dw", "MIX", 0.0, Kind::Pct),
    p("dusty_kick_filter", "FILTER", 0.4, Kind::Pct),
    p("dusty_kick_noise", "NOISE", 0.1, Kind::Pct),
    s("grit_kick_gate", "GATE", "Aero", 0.0, Kind::Toggle),
    p("grit_kick_aero_grind", "GRIND", 0.45, Kind::Pct),
    p("grit_kick_aero_amount", "AMOUNT", 0.2, Kind::Pct),
    p("acoustic_supporter_tune_skin", "SKIN TUNE", 0.5, Kind::Pct),
    s("acoustic_supporter_room_character", "ROOM", "Room", 0.0, Kind::Choice(&AB)),
    p("acoustic_supporter_room_size", "SIZE", 0.5, Kind::Pct),
    p("acoustic_supporter_room_amount", "AMOUNT", 0.75, Kind::Pct),
    p("medio_snap", "SNAP", 0.5, Kind::Pct),
    s("medio_punch_mode", "MODE", "Punch", 0.0, Kind::Choice(&AB)),
    p("medio_punch_amount", "AMOUNT", 1.0, Kind::Pct),
    p("medio_punch_decay", "DECAY", 0.5, Kind::Pct),
    s("jungle_crispness_mode", "MODE", "Crispness", 0.0, Kind::Choice(&AB)),
    p("jungle_crispness_decay", "DECAY", 0.34, Kind::Pct),
    p("jungle_crispness_amount", "AMOUNT", 0.5, Kind::Pct),
];

const SNARE: &[P] = &[
    s("mode", "ENGINE", "Main", 0.0, Kind::Choice(&SNARE_ENGINES)),
    p("tune", "TUNE", 0.5, Kind::Tune),
    p("decay", "DECAY", 0.5, Kind::Pct),
    p("skin_tune", "SKIN TUNE", 0.5, Kind::Pct),
    p("impact", "IMPACT", 0.75, Kind::Pct),
    p("body", "BODY", 0.75, Kind::Pct),
    s("spectra_mode", "MODE", "Spectra", 0.0, Kind::Choice(&AB)),
    p("spectra_tune", "TUNE", 0.5, Kind::Pct),
    p("spectra_amount", "AMOUNT", 0.75, Kind::Pct),
    p("spectra_decay", "DECAY", 0.4, Kind::Pct),
    s("velocity", "VELOCITY", "Scale", 1.0, Kind::Bipolar),
    p("osc_mode", "OSC MODE", 0.0, Kind::Choice(&OSC_MODES)),
    s("noise_color", "COLOR", "Noise", 0.5, Kind::Pct),
    p("noise_amount", "AMOUNT", 0.25, Kind::Pct),
    p("punch", "PUNCH", 0.25, Kind::Pct),
    p("osc_mix", "OSC MIX", 0.5, Kind::Pct),
    p("grit", "GRIT", 0.3, Kind::Pct),
    p("gate", "GATE", 0.0, Kind::Toggle),
    p("bend", "BEND", 0.5, Kind::Bend),
    p("room", "ROOM", 0.5, Kind::Pct),
    p("focus", "FOCUS", 0.5, Kind::Pct),
    p("noise", "NOISE", 0.4, Kind::Pct),
];

const HIHAT: &[P] = &[
    s("mode", "ENGINE", "Main", 0.0, Kind::Choice(&HIHAT_ENGINES)),
    p("tune", "TUNE", 0.5, Kind::Tune),
    p("decay", "DECAY", 0.65, Kind::Pct),
    s("color", "COLOR", "Tone", 0.715, Kind::Color),
    p("impact", "STRIKE", 0.8, Kind::Pct),
    p("gate", "GATE", 0.0, Kind::Toggle),
    s("velocity_scaling", "VELOCITY", "Scale", 1.0, Kind::Bipolar),
    p("hihat606_sat", "SATURATE", 0.19, Kind::Pct),
    p("hihat606_noise", "NOISE", 0.1, Kind::Pct),
    p("hihat606_seed", "SEED", 0.2, Kind::Seed),
    p("hihat606_random", "DISSONANCE", 0.25, Kind::Pct),
    p("cymbal_sample", "SOURCE", 0.0, Kind::Choice(&SOURCES)),
    p("cymbal_distress", "DISTRESS", 0.0, Kind::Pct),
    p("hybrid_metallic", "METALLIC", 0.0, Kind::Pct),
    s("hybrid_bend", "BEND", "Hybrid", 0.5, Kind::Bend),
    p("hybrid_rattle", "RATTLE", 0.5, Kind::Pct),
    p("hybrid_character", "CHARACTER", 0.25, Kind::Pct),
];

const TOM: &[P] = &[
    s("engine", "ENGINE", "Main", 0.0, Kind::Choice(&TOM_ENGINES)),
    p("tune", "TUNE", 0.0, Kind::Tune),
    p("decay", "DECAY", 0.5, Kind::Pct),
    p("bend", "BEND", 0.5, Kind::Bend),
    p("impact", "IMPACT", 0.8, Kind::Pct),
    s("velocity", "VELOCITY", "Scale", 1.0, Kind::Bipolar),
    s("interval", "INTERVAL", "Harmonics", 0.5, Kind::Num(14.0, -0.5)),
    p("fm_freq", "FM FREQ", 0.5, Kind::Pct),
    p("fm_amount", "FM AMOUNT", 0.1, Kind::Pct),
    p("fm_decay", "FM DECAY", 0.35, Kind::Pct),
    s("keytrack_mode", "KTR. MODE", "Harmonics", 0.0, Kind::Choice(&KEYTRACK)),
    p("mix", "MIX", 0.05, Kind::Pct),
    p("glide", "GLIDE", 0.1, Kind::Glide),
    p("transpose", "TRANSPOSE", 0.5, Kind::Num(24.0, -12.0)),
    p("freq_a", "FREQ A", 1.0, Kind::Pct),
    p("freq_b", "FREQ B", 0.75, Kind::Pct),
    p("fm", "FM", 0.5, Kind::Pct),
    p("am", "AM", 0.5, Kind::Pct),
    p("color", "COLOR", 0.5, Kind::Pct),
    p("tension", "TENSION", 1.0, Kind::Pct),
    p("flex", "FLEX", 0.3, Kind::Pct),
    p("air", "AIR", 0.5, Kind::Pct),
    s("skin", "SKIN", "Mute", 0.0, Kind::Pct),
];

const PERC: &[P] = &[
    s("mode", "ENGINE", "Main", 0.0, Kind::Choice(&PERC_ENGINES)),
    p("tune", "TUNE", 0.5, Kind::Tune),
    p("decay", "DECAY", 0.5, Kind::Pct),
    p("impact", "IMPACT", 0.6, Kind::Pct),
    s("velocity", "VELOCITY", "Scale", 1.0, Kind::Bipolar),
    p("shaker_grain", "GRAIN", 0.5, Kind::Pct),
    p("shaker_filter", "FILTER", 0.7, Kind::Pct),
    s("shaker_env_mode", "MODE", "Envelope", 1.0, Kind::Choice(&SHAKER_MODES)),
    p("shaker_attack", "ATTACK", 0.4, Kind::Pct),
    p("shaker_hold", "HOLD", 0.0, Kind::Pct),
    p("shaker_release", "RELEASE", 0.35, Kind::Pct),
    p("shaker_accent", "ACCENT", 0.7, Kind::Pct),
    p("shaker_fill", "FILL", 1.0, Kind::Choice(&SHAKER_FILLS)),
    p("shaker_rate", "RATE", 4.0, Kind::Choice(&SHAKER_RATES)),
    p("shaker_swing", "SWING", 0.0, Kind::Pct),
    p("shaker_twist", "TWIST", 0.0, Kind::Pct),
    p("shaker_length", "LENGTH", 3.0, Kind::Choice(&SHAKER_LENGTHS)),
    p("shaker_offset", "OFFSET", 0.0, Kind::Num(4.0, 0.0)),
    s("shaker_sync", "SYNC", "Envelope", 1.0, Kind::Choice(&SHAKER_SYNCS)),
    p("fractal_freq_a", "FREQ A", 0.5, Kind::Pct),
    p("fractal_freq_b", "FREQ B", 0.4997, Kind::Pct),
    p("fractal_freq_c", "FREQ C", 0.2906, Kind::Pct),
    p("fractal_fm", "FM", 0.3436, Kind::Pct),
    p("fractal_am", "AM", 0.1857, Kind::Pct),
    p("fractal_tune_hold", "TUNE HOLD", 0.0, Kind::Toggle),
    s("fractal_keytrack", "KTR. MODE", "Harmonics", 0.0, Kind::Choice(&KEYTRACK)),
    p("fractal_glide", "GLIDE", 0.0, Kind::Glide),
    p("kettle_puff", "PUFF", 0.25, Kind::Pct),
    p("kettle_damp", "DAMP", 0.0, Kind::Pct),
];

const CYMBAL: &[P] = &[
    s("engine", "ENGINE", "Main", 0.0, Kind::Choice(&CYMBAL_ENGINES)),
    p("tune", "TUNE", 0.3, Kind::Tune),
    p("decay", "DECAY", 0.7, Kind::Pct),
    p("impact", "IMPACT", 0.7, Kind::Pct),
    p("width", "WIDTH", 0.5, Kind::Pct),
    s("velocity", "VELOCITY", "Scale", 1.0, Kind::Bipolar),
    s("density", "DENSITY", "Color", 1.0, Kind::Pct),
    p("tone", "TONE", 0.5, Kind::Pct),
    p("crash_mode", "MODE", 0.0, Kind::Choice(&CRASH_MODES)),
    s("edge", "EDGE", "Strike", 0.5, Kind::Pct),
    p("bell", "BELL", 0.0, Kind::Pct),
    s("character", "CHARACTER", "Color", 0.2, Kind::Pct),
    p("tail", "TAIL", 1.0, Kind::Pct),
];

fn params(module: &str) -> &'static [P] {
    match module {
        "Kick" => KICK,
        "Snare" => SNARE,
        "Hihat" => HIHAT,
        "Tom" => TOM,
        "Perc" => PERC,
        "Cymbal" => CYMBAL,
        _ => &[],
    }
}

/// The field holding a drum's engine.
pub fn engine_field(module: &str) -> &'static str {
    if matches!(module, "Kick" | "Tom" | "Cymbal") { "engine" } else { "mode" }
}

/// A drum's engine names.
#[cfg(test)]
fn engines(module: &str) -> &'static [&'static str] {
    match module {
        "Kick" => &KICK_ENGINES,
        "Snare" => &SNARE_ENGINES,
        "Hihat" => &HIHAT_ENGINES,
        "Tom" => &TOM_ENGINES,
        "Perc" => &PERC_ENGINES,
        "Cymbal" => &CYMBAL_ENGINES,
        _ => &[],
    }
}

/// The fields on knob positions 0–23 (`""`: none) of `module` with `v`'s settings.
fn layout(module: &str, v: &dyn Fn(&str, f64) -> f64) -> [&'static str; 24] {
    let engine = v(engine_field(module), 0.0).round() as i32;
    let mut at = [""; 24];
    let mut put = |from: usize, fields: &[&'static str]| at[from..from + fields.len()].copy_from_slice(fields);
    let (main, advanced, velocity): (&[&'static str], &[&'static str], &'static str) = match (module, engine) {
        // Kick::getParameterByPosition.
        ("Kick", 0) => {
            let attack = if v("sub_kick_thin_attack", 0.0).round() != 0.0 { "sub_kick_attack_amount" } else { "" };
            (
                &["engine", "tune", "decay", "bend", "sub_kick_p_env_decay", "sub_kick_thin_attack", attack, "sub_kick_color"][..],
                &[],
                "velocity",
            )
        }
        ("Kick", 1) => (
            &["engine", "tune", "decay", "bend", "impact"],
            &["clean_bd_drive_gain", "clean_bd_drive_tone", "clean_bd_bias", "clean_bd_drive_dw"],
            "velocity",
        ),
        ("Kick", 2) => (&["engine", "tune", "decay", "", "impact", "dusty_kick_filter", "dusty_kick_noise"], &[], "velocity"),
        ("Kick", 3) => (
            &["engine", "tune", "decay", "bend", "impact", "grit_kick_gate", "grit_kick_aero_grind", "grit_kick_aero_amount"],
            &[],
            "velocity",
        ),
        ("Kick", 4) => (
            &["engine", "tune", "decay", "punch", "impact", "jungle_crispness_mode", "jungle_crispness_decay", "jungle_crispness_amount"],
            &[],
            "velocity",
        ),
        ("Kick", 5) => (
            &["engine", "tune", "decay", "medio_snap", "impact", "medio_punch_mode", "medio_punch_amount", "medio_punch_decay"],
            &[],
            "velocity",
        ),
        ("Kick", 7) => (
            &[
                "engine",
                "tune",
                "decay",
                "acoustic_supporter_tune_skin",
                "impact",
                "acoustic_supporter_room_character",
                "acoustic_supporter_room_size",
                "acoustic_supporter_room_amount",
            ],
            &[],
            "velocity",
        ),
        ("Kick", _) => (&["engine", "tune", "decay", "punch", "impact"], &[], "velocity"),
        // docs/synth-snare.md §1.2.
        ("Snare", 0) => {
            let tone = if v("osc_mode", 0.0).round() != 0.0 { "punch" } else { "osc_mix" };
            (&["engine_", "tune", "decay", "gate", "osc_mode", tone, "noise_color", "noise_amount"][..], &[], "velocity")
        }
        ("Snare", 1) => (&["engine_", "tune", "decay", "gate", "grit", "noise_color", "noise_amount"], &[], "velocity"),
        ("Snare", 2) => (&["engine_", "tune", "decay", "bend", "focus"], &[], "velocity"),
        ("Snare", 8) => (&["engine_", "tune", "decay", "room", "focus"], &[], "velocity"),
        ("Snare", 3) => (&["engine_", "tune", "decay", "skin_tune", "impact", "body", "noise"], &[], "velocity"),
        ("Snare", 7) => (
            &["engine_", "tune", "decay", "skin_tune", "impact", "body"],
            &["spectra_mode", "", "spectra_decay", "spectra_amount"],
            "velocity",
        ),
        ("Snare", _) => (
            &["engine_", "tune", "decay", "skin_tune", "impact"],
            &["spectra_mode", "spectra_tune", "spectra_decay", "spectra_amount"],
            "velocity",
        ),
        // docs/synth-hihat.md §1.3.
        ("Hihat", 0) => (&["engine_", "tune", "decay", "gate", "color", "hihat606_sat", "hihat606_noise"], &[], "velocity_scaling"),
        ("Hihat", 1) => {
            (&["engine_", "tune", "decay", "gate", "color", "hihat606_sat", "hihat606_seed", "hihat606_random"], &[], "velocity_scaling")
        }
        ("Hihat", 2) => {
            (&["engine_", "tune", "decay", "gate", "cymbal_sample", "color", "impact", "cymbal_distress"], &[], "velocity_scaling")
        }
        ("Hihat", _) => (
            &["engine_", "tune", "decay", "", "color", "hybrid_metallic", "impact", "hybrid_character"],
            &["hybrid_bend", "hybrid_rattle"],
            "velocity_scaling",
        ),
        // docs/synth-tom.md §1.3.
        ("Tom", 0) => (&["engine", "tune", "decay", "bend", "impact"], &["interval", "fm_freq", "fm_decay", "fm_amount"], "velocity"),
        ("Tom", 1) => (
            &["engine", "tune", "decay", "bend", "impact", "color", "glide"],
            &["keytrack_mode", "mix", "transpose", "freq_a", "freq_b", "fm", "am"],
            "velocity",
        ),
        ("Tom", 2) => (&["engine", "tune", "decay", "tension", "impact", "flex", "skin", "air"], &[], "velocity"),
        ("Tom", _) => (&["engine", "tune", "decay", "tension", "impact"], &[], "velocity"),
        // docs/synth-perc.md §1.3; the Shaker's follow its envelope mode.
        ("Perc", 0) => (
            &["engine_", "tune", "decay", "fractal_tune_hold", "fractal_glide", "impact"],
            &["fractal_keytrack", "fractal_freq_a", "fractal_freq_b", "fractal_freq_c", "fractal_fm", "fractal_am"],
            "velocity",
        ),
        ("Perc", 1) => (&["engine_", "tune", "decay", "kettle_puff", "kettle_damp", "impact"], &[], "velocity"),
        ("Perc", _) => match v("shaker_env_mode", 1.0).round() as i32 {
            0 => (
                &["engine_", "tune", "shaker_filter", "shaker_grain", "shaker_env_mode", "shaker_attack", "shaker_hold", "shaker_release"]
                    [..],
                &[][..],
                "velocity",
            ),
            1 => (
                &["engine_", "tune", "shaker_filter", "shaker_grain", "shaker_env_mode", "shaker_attack", "", "shaker_release"],
                &[],
                "velocity",
            ),
            _ => (
                &["engine_", "tune", "shaker_filter", "shaker_grain", "shaker_env_mode", "shaker_accent", "shaker_fill", "shaker_release"],
                &["shaker_sync", "shaker_rate", "shaker_length", "shaker_offset", "shaker_swing", "shaker_twist"],
                "velocity",
            ),
        },
        // docs/synth-cymbal.md §1.3.
        ("Cymbal", 0) => (&["engine", "tune", "decay", "impact", "width", "density", "tone", "crash_mode"], &[], "velocity"),
        ("Cymbal", _) => (&["engine", "tune", "decay", "impact", "width", "edge", "bell"], &["character", "tail"], "velocity"),
        _ => (&[], &[], ""),
    };
    put(0, main);
    put(8, advanced);
    at[16] = velocity;
    // "engine_": the drum's engine field, whatever it is called.
    for f in &mut at {
        if *f == "engine_" {
            *f = engine_field(module);
        }
    }
    at
}

/// The Tune text: a note number (`"%.2f"`) for the engines that show one, else a
/// percentage (docs §1.3/§1.5; the Kick's note engines are shown as percentages too).
fn tune_text(module: &str, engine: i32, v: f64) -> String {
    let note = match (module, engine) {
        ("Snare", 0) => Some(53.0 + 24.0 * v),
        ("Snare", 4) => Some(52.0 + 24.0 * v),
        ("Snare", 5) => Some(61.0 + 18.0 * v),
        ("Snare", 6) => Some(60.0 + 24.0 * v),
        ("Snare", 7) => Some(46.0 + 24.0 * v),
        ("Snare", 9) => Some(61.0 + 24.0 * v),
        ("Tom", 0) => Some(36.0 + 24.0 * v),
        ("Tom", 1) => Some(48.0 + 24.0 * v),
        ("Tom", 2) => Some(48.0 + 14.0 * v),
        ("Tom", 3) => Some(57.0 + 14.0 * v),
        ("Perc", 0) => Some(60.0 + 48.0 * v),
        ("Perc", 1) => Some(33.0 + 24.0 * v),
        _ => None,
    };
    match note {
        Some(n) => format!("{n:.2}"),
        None => pct(v),
    }
}

/// Whether Tune shows as a note (and so has a NOTE knob in the editor).
pub fn tune_is_note(module: &str, engine: i32) -> bool {
    !tune_text(module, engine, 0.5).ends_with('%')
}

fn pct(v: f64) -> String {
    format!("{:.1} %", v * 100.0)
}

/// The Hi-hat's Color for Silver and Circuit: its band-pass centre, from the colour's
/// pitch (82…132 for 0…1 at C3), as Maschine formats frequencies [inferred: the pitch
/// is taken as a MIDI note].
fn color_hz(v: f64) -> String {
    let hz = 440.0 * 2f64.powf((v.clamp(0.0, 1.0).mul_add(50.0, 82.0) - 69.0) / 12.0);
    if hz < 1000.0 { format!("{hz:.1} Hz") } else { format!("{:.1} kHz", hz / 1000.0) }
}

/// The knob of `p` in slot `t` with `v`'s settings.
fn knob(p: &P, t: Target, module: &str, engine: i32, v: &dyn Fn(&str, f64) -> f64) -> Knob {
    let x = v(p.field, p.default);
    let label = p.label.into();
    match p.kind {
        Kind::Choice(options) => {
            let cur = (x.round().max(0.0) as usize).min(options.len() - 1);
            (
                label,
                options[cur].into(),
                cur as f32 / (options.len() - 1).max(1) as f32,
                Some(Edit::Choice(t, p.field, options, cur, p.default)),
            )
        }
        Kind::Toggle => {
            let on = x != 0.0;
            (label, if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, p.field, on)))
        }
        Kind::Bipolar => (label, pct(x), ((x + 1.0) / 2.0) as f32, Some(Edit::Param(t, p.field, -1.0, 1.0, p.default))),
        _ => {
            let text = match p.kind {
                Kind::Bend => format!("{:.1} %", (2.0 * x - 1.0) * 100.0),
                Kind::Tune => tune_text(module, engine, x),
                Kind::Seed => {
                    let k = ((x * 30.0) as usize).min(30);
                    SEEDS[k..k + 1].to_string()
                }
                Kind::Color if module == "Hihat" && engine < 2 => color_hz(x),
                Kind::Glide if x <= 0.0 => "None".into(),
                Kind::Num(a, b) => format!("{:.2}", a * x + b),
                _ => pct(x),
            };
            // The Shaker's Offset is −1..1 in 8 steps.
            if p.field == "shaker_offset" {
                return (label, text, ((x + 1.0) / 2.0) as f32, Some(Edit::Param(t, p.field, -1.0, 1.0, p.default)));
            }
            (label, text, x as f32, Some(Edit::Param(t, p.field, 0.0, 1.0, p.default)))
        }
    }
}

/// Whether `module` is a drum synth.
pub fn is_drum(module: &str) -> bool {
    DRUMS.contains(&module)
}

/// Page `page` (0 Main, 1 Advanced, 2 Modulation) of drum synth slot `i`.
pub fn page(slot: &SlotInfo, g: usize, s: usize, i: usize, page: usize) -> Page {
    let t = Target::Module(g, s, i);
    let v = |f: &str, d: f64| slot.param(f).unwrap_or(d);
    let module = slot.module.as_str();
    let engine = v(engine_field(module), 0.0).round() as i32;
    let at = layout(module, &v);
    let all = params(module);
    let mut slots: [Option<Knob>; 8] = Default::default();
    let mut sections = Vec::new();
    for k in 0..8 {
        let field = at[page * 8 + k];
        let Some(p) = all.iter().find(|p| p.field == field) else { continue };
        if let Some(sec) = p.section {
            sections.push((k, sec));
        }
        slots[k] = Some(knob(p, t, module, engine, &v));
    }
    Page { slots, sections }
}

/// The stored field of a drum editor's QML property (`None`: the snake-cased name).
pub fn qml_field(module: &str, prop: &str) -> Option<&'static str> {
    Some(match (module, prop) {
        ("Kick" | "Tom" | "Cymbal", "mode") => "engine",
        ("Snare", "analogMode") => "osc_mode",
        ("Snare", "clapBend") => "bend",
        ("Snare", "clapRoom") => "room",
        ("Snare", "clapSharp") => "focus",
        ("Snare", "classicSnareGrit") => "grit",
        ("Snare", "classicSnareNoiseAmount") => "noise_amount",
        ("Snare", "classicSnareNoiseColor") => "noise_color",
        ("Snare", "classicSnarePunch") => "punch",
        ("Snare", "classicSnareTone") => "osc_mix",
        ("Snare", "discoSDNoise") => "noise",
        ("Snare", "modalSDBodyLevel") => "body",
        ("Hihat", "hihat606Sat") => "hihat606_sat",
        ("Hihat", "hihat606Noise") => "hihat606_noise",
        ("Hihat", "hihat606Seed") => "hihat606_seed",
        ("Hihat", "hihat606Random") => "hihat606_random",
        ("Tom", "dustyTomInterval") => "interval",
        ("Tom", "dustyTomFMP") => "fm_freq",
        ("Tom", "dustyTomFMD") => "fm_decay",
        ("Tom", "dustyTomFMA") => "fm_amount",
        ("Tom", "fbMatrixTomGlide") => "glide",
        ("Tom", "fbMatrixTomColor") => "color",
        ("Tom", "fbMatrixTomMode") => "keytrack_mode",
        ("Tom", "fbMatrixTomToneHarm") => "mix",
        ("Tom", "fbMatrixTomPA") => "freq_a",
        ("Tom", "fbMatrixTomPB") => "freq_b",
        ("Tom", "fbMatrixTomAM") => "am",
        ("Tom", "fbMatrixTomFM") => "fm",
        ("Tom", "fbMatrixTomTrans") => "transpose",
        ("Tom", "tomTension") => "tension",
        ("Tom", "floorTomElasticity") => "flex",
        ("Tom", "floorTomSkinMute") => "skin",
        ("Tom", "floorTomAirMute") => "air",
        ("Perc", "shakerEnvAtt") => "shaker_attack",
        ("Perc", "shakerEnvHold") => "shaker_hold",
        ("Perc", "shakerEnvRel") => "shaker_release",
        ("Perc", "shakerSyncMode") => "shaker_sync",
        ("Perc", "shakerSwingA") => "shaker_swing",
        ("Perc", "shakerSwingB") => "shaker_twist",
        ("Perc", "fbMatrixGlide") => "fractal_glide",
        ("Perc", "fbMatrixF0") => "fractal_freq_a",
        ("Perc", "fbMatrixF1") => "fractal_freq_b",
        ("Perc", "fbMatrixF2") => "fractal_freq_c",
        ("Perc", "fbMatrixAM") => "fractal_am",
        ("Perc", "fbMatrixFM") => "fractal_fm",
        ("Perc", "fbMatrixKeytrack") => "fractal_keytrack",
        ("Cymbal", "crashDensity") => "density",
        ("Cymbal", "crashTone") => "tone",
        ("Cymbal", "crashMode") => "crash_mode",
        ("Cymbal", "rideEdge") => "edge",
        ("Cymbal", "rideBell") => "bell",
        ("Cymbal", "rideCharacter") => "character",
        ("Cymbal", "rideTail") => "tail",
        ("PolySynth", "ampHold") => "hold",
        ("PolySynth", "masterTune") => "tune",
        ("PolySynth", "voiceCount") => "voices",
        ("PolySynth", "modulationAmount") => "mod_amount",
        ("PolySynth", "modulationLFONoise") => "mod_lfo_noise",
        ("PolySynth", "modulationDestFilter") => "mod_filter",
        ("PolySynth", "modulationDestFreqA") => "mod_freq_a",
        ("PolySynth", "modulationDestFreqB") => "mod_freq_b",
        ("PolySynth", "modulationDestPWA") => "mod_pw_a",
        ("PolySynth", "modulationDestPWB") => "mod_pw_b",
        ("PolySynth", "polyModFilterEnv") => "pm_filter_env",
        ("PolySynth", "polyModOscB") => "pm_osc_b",
        ("PolySynth", "polyModDestFreqA") => "pm_freq_a",
        ("PolySynth", "polyModDestPWA") => "pm_pw_a",
        ("PolySynth", "polyModDestFilter") => "pm_filter",
        _ => return None,
    })
}

/// The control for `field` (on any page), for the editor's own controls.
pub fn control(slot: &SlotInfo, g: usize, s: usize, i: usize, field: &str) -> Option<Knob> {
    let t = Target::Module(g, s, i);
    let v = |f: &str, d: f64| slot.param(f).unwrap_or(d);
    let module = slot.module.as_str();
    let engine = v(engine_field(module), 0.0).round() as i32;
    params(module).iter().find(|p| p.field == field).map(|p| knob(p, t, module, engine, &v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(module: &str, params: &[(&str, f64)]) -> SlotInfo {
        SlotInfo {
            module: module.into(),
            name: module.into(),
            active: true,
            plugin: None,
            params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    fn labels(module: &str, params: &[(&str, f64)], n: usize) -> Vec<String> {
        page(&slot(module, params), 0, 0, 0, n).slots.iter().map(|k| k.as_ref().map_or(String::new(), |k| k.0.to_string())).collect()
    }

    /// Every field a layout names is a parameter of its drum, and every drum's stored
    /// fields are DECKARD's engine fields.
    #[test]
    fn layouts_name_known_fields() {
        for module in DRUMS {
            let defaults = deckard_engine::synth::defaults(module).unwrap();
            for p in params(module) {
                assert!(defaults.iter().any(|(f, _)| *f == p.field), "{module}: {} isn't stored", p.field);
            }
            for engine in 0..engines(module).len() {
                for mode in 0..3 {
                    let v = |f: &str, d: f64| {
                        if f == engine_field(module) {
                            engine as f64
                        } else if f == "shaker_env_mode" || f == "osc_mode" || f == "sub_kick_thin_attack" {
                            mode as f64
                        } else {
                            d
                        }
                    };
                    for f in layout(module, &v) {
                        assert!(f.is_empty() || params(module).iter().any(|p| p.field == f), "{module} engine {engine}: {f}");
                    }
                }
            }
        }
    }

    #[test]
    fn pages_follow_the_engine() {
        assert_eq!(labels("Snare", &[], 0), ["ENGINE", "TUNE", "DECAY", "GATE", "OSC MODE", "OSC MIX", "COLOR", "AMOUNT"]);
        assert_eq!(labels("Snare", &[("osc_mode", 1.0)], 0)[5], "PUNCH");
        assert_eq!(labels("Snare", &[("mode", 4.0)], 1)[..4], ["MODE", "TUNE", "DECAY", "AMOUNT"]);
        assert_eq!(labels("Snare", &[], 2)[0], "VELOCITY");
        assert_eq!(labels("Kick", &[("engine", 7.0)], 0)[3], "SKIN TUNE");
        assert_eq!(labels("Hihat", &[("mode", 3.0)], 1)[..2], ["BEND", "RATTLE"]);
        assert_eq!(labels("Perc", &[("mode", 2.0), ("shaker_env_mode", 2.0)], 1)[0], "SYNC");
        assert_eq!(labels("Cymbal", &[("engine", 1.0)], 1)[..2], ["CHARACTER", "TAIL"]);
        // Value texts.
        let first = |module: &str, params: &[(&str, f64)], n: usize, k: usize| {
            page(&slot(module, params), 0, 0, 0, n).slots[k].as_ref().unwrap().1.clone()
        };
        assert_eq!(first("Snare", &[], 0, 1), "65.00", "Volt's Tune is a note");
        assert_eq!(first("Snare", &[("mode", 1.0)], 0, 1), "50.0 %", "Bit's a percentage");
        assert_eq!(first("Snare", &[("mode", 0.0)], 0, 0), "Volt");
        assert_eq!(first("Tom", &[], 1, 0), "6.50", "Interval");
        assert_eq!(first("Hihat", &[("mode", 1.0)], 0, 6), "G", "Seed");
    }
}
