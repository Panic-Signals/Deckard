//! expression-test-file: a copy of a project with expression on its first pattern's
//! notes (a pressure swell, a pitch bend, release velocity) and, with `--pressure`, a
//! channel-pressure lane (saved in Maschine's own format with `--native`), to check
//! real Maschine still opens what DECKARD saves.
//!
//! With `--lane-only` it has just an empty "Aftertouch" lane (Maschine's mode 6) on the
//! first pattern's sound: nothing else.
//!
//! Usage: expression-test-file <in.mxprj> <out.mxprj> [--pressure [--native] | --lane-only]

use deckard_doc::Document;
use deckard_model::{Curve, Expression, MidiEvent, MidiKind, Point};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let (Some(src), Some(dst)) = (args.get(1), args.get(2)) else {
        return Err("usage: expression-test-file <in.mxprj> <out.mxprj> [--pressure]".into());
    };
    let mut d = Document::open(std::path::Path::new(src))?;
    let (g, p) = d
        .project()
        .groups
        .iter()
        .enumerate()
        .find_map(|(g, grp)| grp.patterns.iter().position(|pt| !pt.notes.is_empty()).map(|p| (g, p)))
        .ok_or("no pattern with notes")?;
    let notes = d.project().groups[g].patterns[p].notes.clone();
    if args.iter().any(|a| a == "--lane-only") {
        d.add_lane(g, notes[0].sound, deckard_model::LaneRef { kind: 6, number: -1 })?;
        d.save_as(std::path::Path::new(dst))?;
        println!("an Aftertouch lane on group {} sound {} -> {dst}", g + 1, notes[0].sound + 1);
        return Ok(());
    }
    let pt = |at, value| Point { at, value, curve: 0.0 };
    let mut with = Vec::new();
    for n in &notes {
        let len = n.length.max(1);
        let e = Expression {
            pitch: Some(Curve { points: vec![pt(0, 0.0), pt(len, 1.0)] }),
            slide: None,
            pressure: Some(Curve { points: vec![pt(0, 0.2), pt(len / 2, 1.0), pt(len, 0.4)] }),
            release_velocity: Some(64),
        };
        with.push(deckard_model::Note { expr: Some(d.add_expression(e)), ..*n });
    }
    d.edit_notes("Expression", g, p, &notes, &with)?;
    if args.iter().any(|a| a == "--pressure") {
        let len = d.project().groups[g].patterns[p].length.max(1);
        let sound = notes[0].sound;
        let ramp: Vec<MidiEvent> =
            (0..16).map(|i| MidiEvent { sound, position: len * i / 16, kind: MidiKind::ChannelPressure((i * 8) as u8) }).collect();
        d.edit_midi("Pressure", g, p, &[], &ramp)?;
        // Its lane in the editor (a MidiAutomator of mode 6, "Aftertouch").
        d.add_lane(g, sound, deckard_model::LaneRef { kind: 6, number: -1 })?;
        d.set_native_pressure(args.iter().any(|a| a == "--native"));
    }
    d.save_as(std::path::Path::new(dst))?;
    println!("{} notes with expression in group {} pattern {} -> {dst}", with.len(), g + 1, p + 1);
    Ok(())
}
