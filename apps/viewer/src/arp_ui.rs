//! Note Repeat / Arp's settings as knobs (the MK3's NOTE REPEAT page, the software's
//! panel), and their edits into the song's `arpeggiator` as Maschine stores it
//! (docs/superpowers/plans/2026-10-22-phase2b-repeat-arp.md). The rate is stored in
//! ticks; it shows as a note value (1 BAR, 1/2 … 1/128) and a unit (Normal, Triplet,
//! Dotted).

use deckard_doc::{Document, Target};
use deckard_lua_host::state::{ArpView, SamplingKnob};
use deckard_model::{Arp, ArpType, PPQ};
use mxprj::decode::Value;

pub const RATES: [&str; 8] = ["1 Bar", "1/2", "1/4", "1/8", "1/16", "1/32", "1/64", "1/128"];
pub const UNITS: [&str; 3] = ["Normal", "Triplet", "Dotted"];
/// A preset's name: its note value and unit ("1/16", "1/16T", "1/8D").
fn preset_name(&(rate, unit): &(i64, i64)) -> String {
    let r = RATES.get(rate.max(0) as usize).copied().unwrap_or("?");
    format!("{r}{}", ["", "T", "D"].get(unit.max(0) as usize).copied().unwrap_or(""))
}

/// The preset (0–3) whose rate `ticks` is, if one is: the one chosen to begin with.
pub fn preset_at(presets: &[(i64, i64)], ticks: i64, bar: i64) -> Option<usize> {
    let (r, u) = rate_of(ticks, bar);
    presets.iter().position(|&p| p == (r as i64, u as i64))
}

/// Ticks of note value `rate` (index in [`RATES`]) in `unit`, a bar being `bar` ticks.
pub fn ticks_of(rate: usize, unit: usize, bar: i64) -> i64 {
    let base = if rate == 0 { bar } else { (PPQ * 4) >> rate };
    match unit {
        1 => base * 2 / 3,
        2 => base * 3 / 2,
        _ => base,
    }
}

/// The note value and unit nearest `ticks`.
pub fn rate_of(ticks: i64, bar: i64) -> (usize, usize) {
    let mut best = (4, 0);
    let mut d = i64::MAX;
    for r in 0..RATES.len() {
        for u in 0..UNITS.len() {
            let e = (ticks_of(r, u, bar) - ticks).abs();
            if e < d {
                (best, d) = ((r, u), e);
            }
        }
    }
    best
}

fn note_name(pitch: i64) -> String {
    crate::keys::note_name(pitch)
}

/// The settings as the NOTE REPEAT page shows them: the project's four presets
/// (`presets`, buttons 5–8) and `preset` (0–3) the one chosen, which the rate and unit
/// are kept in. Maschine numbers the presets 1–4 (`arp/preset`'s value; 0 names none),
/// as NI's scripts ask for them.
pub fn view(a: &Arp, presets: &[(i64, i64)], bar: i64, preset: usize, active: bool, locked: bool) -> ArpView {
    let (rate, unit) = rate_of(a.rate, bar);
    let choice = |key: &str, name: &str, list: &[String], i: usize| SamplingKnob {
        key: key.into(),
        name: name.into(),
        text: list.get(i).cloned().unwrap_or_default(),
        value: i as f64,
        min: 0.0,
        max: list.len().saturating_sub(1) as f64,
        step: 1.0,
        turn: 0.08,
    };
    let num = |key: &str, name: &str, text: String, value: f64, (min, max, step): (f64, f64, f64)| SamplingKnob {
        key: key.into(),
        name: name.into(),
        text,
        value,
        min,
        max,
        step,
        turn: if step < 1.0 { 0.004 } else { 0.06 },
    };
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
    let types: Vec<String> = ArpType::ALL.iter().map(|t| t.name().to_string()).collect();
    let sequences: Vec<String> = std::iter::once("Off".to_string()).chain((1..=8).map(|n| n.to_string())).collect();
    // Item 0 is no preset; 1–4 the presets.
    let presets: Vec<String> = std::iter::once("–".to_string()).chain(presets.iter().map(preset_name)).collect();
    let last = presets.len().saturating_sub(2);
    let pct = |v: f64| format!("{:.0}%", v * 100.0);
    let off_or = |n: i64| if n == 0 { "Off".to_string() } else { n.to_string() };
    let knobs = vec![
        choice("arp/type", "TYPE", &types, ArpType::ALL.iter().position(|t| *t == a.kind).unwrap_or(0)),
        choice("arp/rate", "RATE", &s(&RATES), rate),
        choice("arp/rate_unit", "UNIT", &s(&UNITS), unit),
        choice("arp/sequence", "SEQUENCE", &sequences, a.sequence.clamp(0, 8) as usize),
        num("arp/octaves", "OCTAVES", a.octaves.to_string(), a.octaves as f64, (1.0, 8.0, 1.0)),
        num("arp/dynamic", "DYNAMIC", pct(a.dynamic), a.dynamic, (0.01, 2.0, 0.01)),
        num("arp/gate", "GATE", pct(a.gate), a.gate, (0.0, 2.0, 0.01)),
        choice("arp/hold", "HOLD", &s(&["Off", "On"]), usize::from(a.hold)),
        num("arp/retrigger", "RETRIGGER", off_or(a.retrigger), a.retrigger as f64, (0.0, 16.0, 1.0)),
        num("arp/repeat", "REPEAT", off_or(a.repeat), a.repeat as f64, (0.0, 16.0, 1.0)),
        num("arp/offset", "OFFSET", a.offset.to_string(), a.offset as f64, (-8.0, 8.0, 1.0)),
        num("arp/inversion", "INVERSION", off_or(a.inversion), a.inversion as f64, (0.0, 4.0, 1.0)),
        num("arp/min_key", "MIN. KEY", note_name(a.min_key), a.min_key as f64, (0.0, 127.0, 1.0)),
        num("arp/max_key", "MAX. KEY", note_name(a.max_key), a.max_key as f64, (0.0, 127.0, 1.0)),
        SamplingKnob { min: 1.0, ..choice("arp/preset", "PRESET", &presets, preset.min(last) + 1) },
    ];
    let lists = vec![
        ("arp/type".to_string(), types),
        ("arp/rate".to_string(), s(&RATES)),
        ("arp/rate_unit".to_string(), s(&UNITS)),
        ("arp/sequence".to_string(), sequences),
        ("arp/hold".to_string(), s(&["Off", "On"])),
        ("arp/preset".to_string(), presets),
    ];
    ArpView { knobs, lists, active, locked }
}

/// Set a setting (`arp/<field>` = `value`, as the knob has it) in the project. The rate
/// and unit go into `preset` (0–3, the one chosen) too; choosing a preset (1–4) plays
/// its rate.
pub fn apply(d: &mut Document, key: &str, value: f64, preset: usize) -> Result<(), String> {
    let bar = d.project().bar_length();
    let a = d.project().arp.clone();
    let int = |v: f64| Value::Int(v.round() as i64);
    let (field, v) = match key {
        "arp/type" => ("type", int(value)),
        "arp/sequence" => ("sequence", int(value)),
        "arp/octaves" => ("octaves", int(value.clamp(1.0, 8.0) - 1.0)),
        "arp/dynamic" => ("dynamic", Value::Float(value.clamp(0.01, 2.0))),
        "arp/gate" => ("gate", Value::Float(value.clamp(0.0, 2.0))),
        "arp/hold" => ("hold", int(value)),
        "arp/retrigger" => ("retrigger", int(value)),
        "arp/repeat" => ("repeat", int(value)),
        "arp/offset" => ("offset", int(value.clamp(-8.0, 8.0) + 8.0)),
        "arp/inversion" => ("inversion", int(value)),
        "arp/min_key" => ("min_key", int(value)),
        "arp/max_key" => ("max_key", int(value)),
        "arp/rate" | "arp/rate_unit" | "arp/preset" => {
            let (mut rate, mut unit) = rate_of(a.rate, bar);
            match key {
                "arp/rate" => rate = (value.round() as usize).min(RATES.len() - 1),
                "arp/rate_unit" => unit = (value.round() as usize).min(UNITS.len() - 1),
                // Presets 1–4 (0: none, no change).
                _ => {
                    let Some(&(r, u)) = (value >= 0.5).then(|| d.project().arp_presets.get(value.round() as usize - 1)).flatten() else {
                        return Ok(());
                    };
                    (rate, unit) = ((r.max(0) as usize).min(RATES.len() - 1), (u.max(0) as usize).min(UNITS.len() - 1));
                }
            }
            // Kept in the preset chosen.
            if key != "arp/preset" && preset < d.project().arp_presets.len() {
                d.set_arp_preset(preset, rate as i64, unit as i64)?;
            }
            ("pc0", Value::Int(ticks_of(rate, unit, bar)))
        }
        _ => return Err(format!("no such setting: {key}")),
    };
    d.set("Note Repeat", Target::Song, &format!("arpeggiator/{field}"), v)
}

#[cfg(test)]
mod tests {
    use super::{apply, preset_at, rate_of, ticks_of, view};
    use deckard_model::{Arp, ArpType};

    #[test]
    fn rates_are_a_note_value_and_a_unit() {
        assert_eq!(rate_of(240, 3840), (4, 0), "1/16");
        assert_eq!(rate_of(160, 3840), (4, 1), "1/16 triplet");
        assert_eq!(rate_of(360, 3840), (4, 2), "1/16 dotted");
        assert_eq!(rate_of(3840, 3840), (0, 0), "1 bar");
        assert_eq!(ticks_of(5, 1, 3840), 80, "1/32 triplet");
        assert_eq!(ticks_of(rate_of(320, 3840).0, rate_of(320, 3840).1, 3840), 320);
    }

    #[test]
    fn the_view_shows_the_arps_settings() {
        let a = Arp { kind: ArpType::OrderPlayed, gate: 0.7, octaves: 2, ..Arp::default() };
        let v = view(&a, &deckard_model::ARP_PRESETS, 3840, 2, true, false);
        let text = |k: &str| v.knobs.iter().find(|x| x.key == k).map(|x| x.text.clone());
        assert_eq!(text("arp/type").as_deref(), Some("Order Played"));
        assert_eq!(text("arp/rate").as_deref(), Some("1/16"));
        assert_eq!(text("arp/rate_unit").as_deref(), Some("Normal"));
        assert_eq!(text("arp/gate").as_deref(), Some("70%"));
        assert_eq!(text("arp/octaves").as_deref(), Some("2"));
        assert_eq!(text("arp/offset").as_deref(), Some("0"));
        assert!(v.active && !v.locked);
        let names = &v.lists.iter().find(|(k, _)| k == "arp/preset").unwrap().1;
        assert_eq!(names, &["–", "1/8", "1/16", "1/16T", "1/32"], "none, then the project's presets 1–4");
        // The preset chosen (the third: 3).
        assert_eq!(v.knobs.iter().find(|x| x.key == "arp/preset").map(|x| x.value), Some(3.0));
    }

    /// Edits go into the song's arpeggiator as Maschine stores it, and read back.
    #[test]
    fn edits_go_into_the_project() {
        let Ok(mut d) =
            deckard_doc::Document::open(std::path::Path::new("/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj"))
        else {
            return;
        };
        apply(&mut d, "arp/gate", 0.5, 1).unwrap();
        apply(&mut d, "arp/rate", 3.0, 1).unwrap();
        apply(&mut d, "arp/rate_unit", 1.0, 1).unwrap();
        apply(&mut d, "arp/type", 2.0, 1).unwrap();
        apply(&mut d, "arp/octaves", 3.0, 1).unwrap();
        apply(&mut d, "arp/offset", -2.0, 1).unwrap();
        apply(&mut d, "arp/hold", 1.0, 1).unwrap();
        let a = &d.project().arp;
        assert_eq!((a.gate, a.rate, a.kind, a.octaves, a.offset, a.hold), (0.5, 320, ArpType::UpDown, 3, -2, true));
    }

    /// The rate and unit go into the preset chosen too; choosing a preset plays its rate.
    #[test]
    fn the_chosen_preset_keeps_the_rate() {
        let mut d = deckard_doc::Document::new_project().unwrap();
        let bar = d.project().bar_length();
        assert_eq!(preset_at(&d.project().arp_presets, d.project().arp.rate, bar), Some(1), "1/16: the second");
        // The first (1/8) chosen, then set to 1/16 Triplet.
        apply(&mut d, "arp/preset", 1.0, 1).unwrap();
        assert_eq!(rate_of(d.project().arp.rate, bar), (3, 0), "1/8 plays");
        apply(&mut d, "arp/rate", 4.0, 0).unwrap();
        apply(&mut d, "arp/rate_unit", 1.0, 0).unwrap();
        assert_eq!(d.project().arp_presets[0], (4, 1), "kept in preset 1");
        assert_eq!(rate_of(d.project().arp.rate, bar), (4, 1));
        assert_eq!(d.project().arp_presets[1], (4, 0), "the others as they were");
    }
}
