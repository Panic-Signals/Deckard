use super::*;

/// An engine at 1000 Hz (120 BPM: a beat is 500 frames) playing one note, C3 for
/// `length` ticks with expression `e`, out to port "Synth" (channel 3), and what it
/// sends.
fn expr_engine(e: deckard_model::Expression, length: i64) -> (Engine, std::sync::mpsc::Receiver<midi::MidiOut>) {
    let mut p = project();
    p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
    p.groups[0].sounds[0].channel.midi_out_channel = 2;
    let n = Note {
        ratchet: Default::default(),
        expr: Some(deckard_model::ExprId(0)),
        sound: 0,
        position: 0,
        length,
        pitch: 60,
        velocity: 100,
        probability: 100,
    };
    p.groups[0].patterns[0].notes = vec![n];
    p.expressions = vec![std::sync::Arc::new(e)];
    let mut engine = click_engine_for(&p, 1000);
    let (tx, rx) = std::sync::mpsc::sync_channel(4096);
    engine.midi_tx = Some(tx);
    (engine, rx)
}

/// An engine like [`expr_engine`] playing `notes` (each with its expression in
/// `exprs`, by index) out to "Synth" set to MPE (bend range 48).
fn mpe_engine(notes: Vec<Note>, exprs: Vec<deckard_model::Expression>) -> (Engine, std::sync::mpsc::Receiver<midi::MidiOut>) {
    let (mut engine, rx) = expr_engine(deckard_model::Expression::default(), PPQ);
    let mut p = project();
    p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
    p.groups[0].sounds[0].channel.midi_out_channel = 2;
    p.groups[0].patterns[0].notes = notes;
    p.expressions = exprs.into_iter().map(std::sync::Arc::new).collect();
    let tx = engine.midi_tx.take();
    engine = click_engine_for(&p, 1000);
    engine.midi_tx = tx;
    engine.set_mpe_out([("Synth".to_string(), 48)].into());
    (engine, rx)
}

fn note_at(position: i64, length: i64, pitch: i64, expr: Option<u32>) -> Note {
    Note {
        ratchet: Default::default(),
        expr: expr.map(deckard_model::ExprId),
        sound: 0,
        position,
        length,
        pitch,
        velocity: 100,
        probability: 100,
    }
}

/// On an MPE port each note gets a member channel of its own (2, 3, 4 …), and
/// ends on it; the sound's channel setting doesn't count.
#[test]
fn mpe_notes_get_channels_of_their_own() {
    let notes = vec![note_at(0, PPQ, 60, None), note_at(10, PPQ, 64, None), note_at(20, PPQ, 67, None)];
    let (mut e, rx) = mpe_engine(notes, vec![]);
    let got = sent(&mut e, &rx, 700);
    let ons: Vec<&Vec<u8>> = got.iter().filter(|m| m[0] & 0xf0 == 0x90).collect();
    assert_eq!(ons, vec![&vec![0x91, 60, 100], &vec![0x92, 64, 100], &vec![0x93, 67, 100]]);
    for off in [[0x81, 60, 0], [0x82, 64, 0], [0x83, 67, 0]] {
        assert!(got.contains(&off.to_vec()), "{off:?} in {got:?}");
    }
}

/// Before its note-on, a note's channel gets the note's starting bend, slide
/// (CC74) and pressure; a note without expression starts neutral.
#[test]
fn mpe_notes_start_with_their_values() {
    let x = deckard_model::Expression {
        pitch: curve(&[(0, 12.0)]),
        slide: curve(&[(0, 0.5)]),
        pressure: curve(&[(0, 0.25)]),
        ..Default::default()
    };
    let (mut e, rx) = mpe_engine(vec![note_at(0, PPQ, 60, Some(0)), note_at(10, PPQ, 62, None)], vec![x]);
    let got = sent(&mut e, &rx, 100);
    assert_eq!(&got[..4], &[vec![0xe1, 0, 80], vec![0xb1, 74, 64], vec![0xd1, 32], vec![0x91, 60, 100]]);
    assert_eq!(&got[4..8], &[vec![0xe2, 0, 64], vec![0xb2, 74, 64], vec![0xd2, 0], vec![0x92, 62, 100]]);
}

/// While a note plays, its curves go out on its channel: pressure as channel
/// pressure, slide as CC74, pitch as pitch bend (clipped to the port's range).
#[test]
fn mpe_curves_play_on_the_member_channel() {
    let x = deckard_model::Expression {
        pitch: curve(&[(0, 0.0), (PPQ, 60.0)]),
        slide: curve(&[(0, 0.0), (PPQ, 1.0)]),
        pressure: curve(&[(0, 0.0), (PPQ, 1.0)]),
        ..Default::default()
    };
    let (mut e, rx) = mpe_engine(vec![note_at(0, PPQ, 60, Some(0))], vec![x]);
    let got = sent(&mut e, &rx, 600);
    let off = got.iter().position(|m| m[0] == 0x81).expect("note-off");
    let during = &got[4..off];
    assert!(during.iter().any(|m| m[0] == 0xd1 && m[1] > 100), "pressure");
    assert!(during.iter().any(|m| m[..2] == [0xb1, 74] && m[2] > 100), "slide");
    assert!(during.iter().any(|m| m[..] == [0xe1, 0x7f, 0x7f]), "pitch clipped at +48");
    assert!(during.iter().all(|m| m[0] & 0x0f == 1), "all on its channel: {during:?}");
}

/// A pattern's controller and pitch-bend lanes go on the manager channel (1).
#[test]
fn lanes_go_on_the_manager_channel() {
    let (mut e, rx) = mpe_engine(vec![], vec![]);
    let mut p = project();
    p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
    p.groups[0].sounds[0].channel.midi_out_channel = 2;
    p.groups[0].patterns[0].notes = vec![];
    p.groups[0].patterns[0].midi = vec![
        deckard_model::MidiEvent { sound: 0, position: 0, kind: MidiKind::Controller { number: 1, value: 50 } },
        deckard_model::MidiEvent { sound: 0, position: 10, kind: MidiKind::PitchBend(9000) },
    ];
    let tx = e.midi_tx.take();
    e = click_engine_for(&p, 1000);
    e.midi_tx = tx;
    e.set_mpe_out([("Synth".to_string(), 48)].into());
    let got = sent(&mut e, &rx, 50);
    assert_eq!(&got[..2], &[vec![0xb0, 1, 50], vec![0xe0, (9000 & 0x7f) as u8, (9000 >> 7) as u8]]);
}

/// With all 15 channels busy, a 16th note ends the oldest and takes its channel.
#[test]
fn the_sixteenth_note_steals_the_oldest() {
    let notes = (0..16).map(|i| note_at(i * 10, 3 * PPQ, 60 + i, None)).collect();
    let (mut e, rx) = mpe_engine(notes, vec![]);
    let got = sent(&mut e, &rx, 120);
    let off = got.iter().position(|m| m[..] == [0x81, 60, 0]).expect("the first note ended");
    let on = got.iter().position(|m| m[..] == [0x91, 75, 100]).expect("the 16th on its channel");
    assert!(off < on);
}

/// Stopping resets the member channels DECKARD used, and only those.
#[test]
fn stop_resets_used_member_channels() {
    let (mut e, rx) = mpe_engine(vec![note_at(0, 3 * PPQ, 60, None), note_at(10, 3 * PPQ, 64, None)], vec![]);
    sent(&mut e, &rx, 50);
    e.stop();
    let got: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
    for ch in [1u8, 2] {
        assert!(got.contains(&vec![0x80 | ch, if ch == 1 { 60 } else { 64 }, 0]), "{got:?}");
        for m in [vec![0xe0 | ch, 0, 64], vec![0xb0 | ch, 74, 64], vec![0xd0 | ch, 0]] {
            assert!(got.contains(&m), "{m:?} in {got:?}");
        }
    }
    assert!(got.iter().all(|m| (1..=2).contains(&(m[0] & 0x0f))), "only channels 2 and 3: {got:?}");
}

/// The port set back to normal while a note sounds: the note ends on its member
/// channel, the channels reset, and later notes go on the sound's channel.
#[test]
fn changing_a_ports_format_resets_it() {
    let (mut e, rx) = mpe_engine(vec![note_at(0, PPQ, 60, None), note_at(2 * PPQ, PPQ, 62, None)], vec![]);
    sent(&mut e, &rx, 50);
    e.set_mpe_out(Default::default());
    let got = sent(&mut e, &rx, 1100);
    assert_eq!(&got[..4], &[vec![0x81, 60, 0], vec![0xe1, 0, 64], vec![0xb1, 74, 64], vec![0xd1, 0]]);
    assert!(got.contains(&vec![0x92, 62, 100]), "{got:?}");
}

fn live_expr(pitch: i64, kind: ExprKind, value: f32) -> Live {
    Live::Expression { group: 0, sound: 0, pitch, kind, value }
}

/// A sound with modulation routes: pressure of its first slot's cutoff.
fn with_route(p: &mut Project) {
    use deckard_model::mod_matrix::{ModParam, ModRoute, ModSource};
    p.groups[0].sounds[0].mod_routes = vec![ModRoute::new(ModSource::Pressure, 0, "Sampler", ModParam::Field("cutoff".into()))];
}

fn live_engine() -> Engine {
    let mut p = project();
    with_route(&mut p);
    click_engine_for(&p, 1000)
}

/// Pressure: the hardest-pressed held note's.
#[test]
fn pressure_source_is_the_hardest_pressed_note() {
    let mut e = live_engine();
    e.note_hold(0, 0, 60, 100);
    e.play_live(live_expr(60, ExprKind::Dim(Dim::Pressure), 0.3));
    e.note_hold(0, 0, 64, 100);
    e.play_live(live_expr(64, ExprKind::Dim(Dim::Pressure), 0.7));
    assert_eq!(e.sources(0, 0)[2], Some(0.7));
    e.play_live(live_expr(64, ExprKind::Dim(Dim::Pressure), 0.1));
    assert_eq!(e.sources(0, 0)[2], Some(0.3));
}

/// Slide and pitch: the latest held note's; released, the one before's.
#[test]
fn slide_and_pitch_follow_the_latest_note() {
    let mut e = live_engine();
    e.note_hold(0, 0, 60, 100);
    e.play_live(live_expr(60, ExprKind::Dim(Dim::Slide), 0.2));
    e.play_live(live_expr(60, ExprKind::Dim(Dim::Pitch), -1.0));
    e.note_hold(0, 0, 64, 100);
    e.play_live(live_expr(64, ExprKind::Dim(Dim::Slide), 0.8));
    e.play_live(live_expr(64, ExprKind::Dim(Dim::Pitch), 2.0));
    assert_eq!(e.sources(0, 0)[..2], [Some(2.0), Some(0.8)]);
    e.note_release(0, 0, 64);
    assert_eq!(e.sources(0, 0)[..2], [Some(-1.0), Some(0.2)]);
}

/// A note without pressure (a plain hit) doesn't count for it.
#[test]
fn a_note_without_pressure_doesnt_count() {
    let mut e = live_engine();
    e.note_hold(0, 0, 60, 100);
    e.play_live(live_expr(60, ExprKind::Dim(Dim::Pressure), 0.5));
    e.note_hold(0, 0, 64, 100);
    assert_eq!(e.sources(0, 0), [None, None, Some(0.5)]);
}

/// A pad held with Note Repeat on: its pressure is the source the whole time it's
/// held, between the repeats too, until it's let go.
#[test]
fn a_repeated_pads_pressure_holds_between_repeats() {
    let mut p = project();
    with_route(&mut p);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
    e.looping = true;
    e.set_repeat(Some((deckard_model::Arp { rate: 240, gate: 0.5, ..Default::default() }, repeat::RepeatMode::NoteRepeat)));
    e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
    e.schedule(0, live_expr(60, ExprKind::Dim(Dim::Pressure), 0.6));
    let mut out = vec![0f32; 512];
    // Several repeats (a quarter: four at 240 ticks), checked every block.
    let n = (960.0 * e.samples_per_tick() / 256.0).ceil() as usize;
    for i in 0..n {
        e.process(&mut out);
        assert_eq!(e.sources(0, 0)[2], Some(0.6), "block {i}");
    }
    e.schedule(0, Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
    for _ in 0..n {
        e.process(&mut out);
    }
    assert_eq!(e.sources(0, 0)[2], None, "let go");
}

/// A pattern's note with a pressure ramp drives the source as it plays, and ends
/// with it: on the Sampler and on a MIDI port alike.
#[test]
fn recorded_curves_drive_the_sources() {
    let ramp = deckard_model::Expression { pressure: curve(&[(0, 0.0), (PPQ, 1.0)]), ..Default::default() };
    let mut p = project();
    with_route(&mut p);
    p.groups[0].patterns[0].notes = vec![note_at(0, PPQ, 60, Some(0))];
    p.expressions = vec![std::sync::Arc::new(ramp.clone())];
    let mut on_sampler = click_engine_for(&p, 1000);
    let (mut on_port, _rx) = expr_engine(ramp, PPQ);
    for e in [&mut on_sampler, &mut on_port] {
        let mut out = vec![0f32; 2];
        // Half the note (120 BPM at 1 kHz: 500 frames a quarter).
        for _ in 0..250 {
            e.process(&mut out);
        }
        let x = e.sources(0, 0)[2].expect("pressure while the note plays");
        assert!((x - 0.5).abs() < 0.02, "{x}");
        for _ in 0..300 {
            e.process(&mut out);
        }
        assert_eq!(e.sources(0, 0), [None; 3], "the note ended");
    }
}

/// Played live into an MPE port: a held key's pressure, slide and pitch go out
/// on its own channel.
#[test]
fn live_mpe_expression_goes_out_per_note() {
    let (mut e, rx) = mpe_engine(vec![], vec![]);
    e.note_in(0, 0, 60, 90, true);
    rx.try_iter().count();
    e.play_live(live_expr(60, ExprKind::Dim(Dim::Pressure), 0.5));
    e.play_live(live_expr(60, ExprKind::Dim(Dim::Slide), 0.25));
    e.play_live(live_expr(60, ExprKind::Dim(Dim::Pitch), 2.0));
    let got: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
    let b = midi::bend_14(2.0, 48);
    assert_eq!(got, vec![vec![0xd1, 64], vec![0xb1, 74, 32], vec![0xe1, (b & 0x7f) as u8, (b >> 7) as u8]]);
}

/// A key from an MPE controller comes with its starting bend and slide (after its
/// note-on, as the input reads them): on an MPE port they still go out before the
/// note-on, so the synth starts the note where it was played.
#[test]
fn live_mpe_notes_start_with_the_controllers_values() {
    let (mut e, rx) = mpe_engine(vec![], vec![]);
    e.schedule(0, Live::NoteIn { group: 0, sound: 0, pitch: 60, velocity: 90, thru: true });
    e.schedule(0, live_expr(60, ExprKind::Dim(Dim::Pitch), 2.0));
    e.schedule(0, live_expr(60, ExprKind::Dim(Dim::Slide), 0.25));
    let got = sent(&mut e, &rx, 1);
    let b = midi::bend_14(2.0, 48);
    assert_eq!(&got[..4], &[vec![0xe1, (b & 0x7f) as u8, (b >> 7) as u8], vec![0xb1, 74, 32], vec![0xd1, 0], vec![0x91, 60, 90]]);
    assert_eq!(got.len(), 4, "nothing again after: {got:?}");
}

/// Into a normal port only pressure goes, as poly aftertouch.
#[test]
fn live_pitch_and_slide_stay_off_normal_ports() {
    let (mut e, rx) = expr_engine(deckard_model::Expression::default(), PPQ);
    e.note_in(0, 0, 60, 90, true);
    rx.try_iter().count();
    e.play_live(live_expr(60, ExprKind::Dim(Dim::Slide), 0.25));
    e.play_live(live_expr(60, ExprKind::Dim(Dim::Pitch), 2.0));
    e.play_live(live_expr(60, ExprKind::Dim(Dim::Pressure), 0.5));
    let got: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
    assert_eq!(got, vec![vec![0xa2, 60, 64]]);
}

/// A key's release velocity, played live, goes on its note-off.
#[test]
fn live_release_velocity_reaches_the_note_off() {
    let (mut e, rx) = expr_engine(deckard_model::Expression::default(), PPQ);
    e.note_in(0, 0, 60, 90, true);
    rx.try_iter().count();
    e.play_live(live_expr(60, ExprKind::Release, 0.5));
    e.play_live(Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
    let got: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
    assert_eq!(got, vec![vec![0x82, 60, 64]]);
}

/// With Note Repeat / the Arp on, a keyboard's key (which never goes into the
/// repeater) still ends on its release, not when the transport stops.
#[test]
fn keyboard_release_ends_its_note_while_note_repeat_is_on() {
    let (mut e, rx) = expr_engine(deckard_model::Expression::default(), PPQ);
    e.set_repeat(Some((deckard_model::Arp { rate: 240, gate: 0.5, ..Default::default() }, repeat::RepeatMode::Arp)));
    e.play_live(Live::NoteIn { group: 0, sound: 0, pitch: 60, velocity: 90, thru: true });
    rx.try_iter().count();
    e.play_live(Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
    let got: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
    assert!(got.iter().any(|m| m[0] & 0xf0 == 0x80 && m[1] == 60), "the key's note-off: {got:?}");
}

/// A recorded note's pressure plays out with it as poly aftertouch on its key,
/// rising with the curve, and stops with the note.
#[test]
fn recorded_pressure_plays_as_poly_aftertouch() {
    let e = deckard_model::Expression { pressure: curve(&[(0, 0.0), (PPQ, 1.0)]), ..Default::default() };
    let (mut engine, rx) = expr_engine(e, PPQ);
    let got = sent_by_frame(&mut engine, &rx, 600);
    let off = got.iter().position(|(_, m)| m[0] == 0x82).expect("note-off");
    assert_eq!(got[0].1, vec![0x92, 60, 100]);
    let at: Vec<u8> = got[..off].iter().filter(|(_, m)| m[0] == 0xa2 && m[1] == 60).map(|(_, m)| m[2]).collect();
    assert!(at.len() > 10, "{at:?}");
    assert!(at.windows(2).all(|w| w[1] > w[0]), "rising: {at:?}");
    assert!(at[0] < 5 && *at.last().unwrap() >= 120, "{at:?}");
    assert!(!got[off..].iter().any(|(_, m)| m[0] == 0xa2), "none after the note-off");
}

/// Only changes are sent, and no more often than every 5 ms (5 frames here).
#[test]
fn expression_sends_changes_at_most_every_5_ms() {
    let e = deckard_model::Expression { pressure: curve(&[(0, 0.5)]), ..Default::default() };
    let (mut engine, rx) = expr_engine(e, PPQ);
    let at: Vec<Vec<u8>> = sent_by_frame(&mut engine, &rx, 600).into_iter().map(|x| x.1).filter(|m| m[0] == 0xa2).collect();
    assert_eq!(at, vec![vec![0xa2, 60, 64]], "a steady pressure, once");
    let e = deckard_model::Expression { pressure: curve(&[(0, 0.0), (10, 1.0)]), ..Default::default() };
    let (mut engine, rx) = expr_engine(e, PPQ);
    let frames: Vec<usize> = sent_by_frame(&mut engine, &rx, 100).into_iter().filter(|(_, m)| m[0] == 0xa2).map(|x| x.0).collect();
    assert!(frames.len() >= 2, "{frames:?}");
    assert!(frames.windows(2).all(|w| w[1] - w[0] >= 5), "{frames:?}");
}

/// The note's release velocity goes on its note-off (0 without one, as before).
#[test]
fn release_velocity_on_the_note_off() {
    let e = deckard_model::Expression { release_velocity: Some(40), ..Default::default() };
    let (mut engine, rx) = expr_engine(e, PPQ);
    let got: Vec<Vec<u8>> = sent_by_frame(&mut engine, &rx, 600).into_iter().map(|x| x.1).collect();
    assert!(got.contains(&vec![0x82, 60, 40]), "{got:?}");
    let (mut engine, rx) = expr_engine(deckard_model::Expression::default(), PPQ);
    let got: Vec<Vec<u8>> = sent_by_frame(&mut engine, &rx, 600).into_iter().map(|x| x.1).collect();
    assert!(got.contains(&vec![0x82, 60, 0]), "{got:?}");
}

/// A normal port has no per-note pitch or slide: only the note goes out.
#[test]
fn pitch_and_slide_stay_off_normal_ports() {
    let e =
        deckard_model::Expression { pitch: curve(&[(0, 0.0), (PPQ, 2.0)]), slide: curve(&[(0, 0.2), (PPQ, 0.9)]), ..Default::default() };
    let (mut engine, rx) = expr_engine(e, PPQ);
    let got: Vec<Vec<u8>> = sent_by_frame(&mut engine, &rx, 600).into_iter().map(|x| x.1).collect();
    assert_eq!(got, vec![vec![0x92, 60, 100], vec![0x82, 60, 0]]);
}

/// A pattern note with expression keeps its id on the timeline, and the timeline
/// carries the table it indexes; the engine finds it there (an unknown id: none).
#[test]
fn timeline_notes_carry_their_expression() {
    let mut p = project();
    p.groups[0].patterns[0].notes[0].expr = Some(deckard_model::ExprId(0));
    let e = deckard_model::Expression { release_velocity: Some(40), ..Default::default() };
    p.expressions = vec![std::sync::Arc::new(e.clone())];
    let t = timeline(&p, Mode::Arrangement);
    let with: Vec<_> = t.events.iter().filter(|n| n.expr.is_some()).collect();
    assert!(!with.is_empty());
    // The first note, at each pass of the pattern; the others have none.
    assert!(with.iter().all(|n| n.tick % (4 * PPQ) == 0 && n.expr == Some(deckard_model::ExprId(0))), "{with:?}");
    assert_eq!(t.exprs.len(), 1);
    let engine = click_engine_for(&p, 1000);
    assert_eq!(engine.expression(deckard_model::ExprId(0)).as_deref(), Some(&e));
    assert!(engine.expression(deckard_model::ExprId(9)).is_none());
}

/// The first pass's hits of the test project's one pattern with `note` its only note.
fn ratchet_hits(note: Note) -> Vec<NoteEvent> {
    let mut p = project();
    p.groups[0].patterns[0].notes = vec![note];
    let t = timeline(&p, Mode::Arrangement);
    let bar = own_length(&p, &p.groups[0].patterns[0]);
    t.events.into_iter().filter(|e| e.midi.is_none() && e.cue.is_none() && e.tick < bar).collect()
}

fn ratcheted(position: i64, length: i64, velocity: i64, ratchet: deckard_model::ratchet::Ratchet) -> Note {
    Note { ratchet, expr: None, sound: 0, position, length, pitch: 60, velocity, probability: 100 }
}

#[test]
fn a_ratcheted_note_plays_its_hits() {
    use deckard_model::ratchet::{Ratchet, Shape};
    let down = Ratchet { repeats: 4, shape: Shape::Down, floor: 0, ceiling: 127 };
    let hits = ratchet_hits(ratcheted(0, PPQ, 100, down));
    let got: Vec<(i64, i64, i64)> = hits.iter().map(|e| (e.tick, e.length, e.velocity)).collect();
    assert_eq!(got, [(0, PPQ / 4, 100), (PPQ / 4, PPQ / 4, 75), (PPQ / 2, PPQ / 4, 50), (3 * PPQ / 4, PPQ / 4, 25)]);
    assert!(hits[0].roll.is_some() && hits.iter().all(|e| e.roll == hits[0].roll), "one roll for the note");
    // An ordinary note is the one event it always was.
    let plain = ratchet_hits(ratcheted(0, PPQ, 100, Ratchet::default()));
    assert_eq!(plain.iter().map(|e| (e.tick, e.length, e.velocity, e.roll)).collect::<Vec<_>>(), [(0, PPQ, 100, None)]);
}

/// Each ratcheted note has a roll of its own, whatever has been taken out of the
/// timeline before it is filled in (a clip's span is, before the clip's notes go in).
#[test]
fn two_ratcheted_notes_never_share_a_roll() {
    use deckard_model::ratchet::{Ratchet, Shape};
    let mut p = project();
    let r = Ratchet { repeats: 2, shape: Shape::Flat, floor: 0, ceiling: 127 };
    p.groups[0].patterns[0].notes = vec![ratcheted(0, PPQ, 100, r)];
    let bar = own_length(&p, &p.groups[0].patterns[0]);
    let mut t = Timeline::default();
    fill_with(&p, 0, &p.groups[0].patterns[0], 0, bar, &mut t);
    let first = t.events.iter().find_map(|e| e.roll).unwrap();
    t.events.clear();
    fill_with(&p, 0, &p.groups[0].patterns[0], bar, bar, &mut t);
    let second = t.events.iter().find_map(|e| e.roll).unwrap();
    assert_ne!(first, second);
}

/// A ratcheted note's expression runs over the whole note: each hit starts in the
/// curves where it starts in the note, and a sound's sources read them from there.
#[test]
fn a_ratchets_hits_read_their_own_stretch_of_the_notes_expression() {
    use deckard_model::ratchet::{Ratchet, Shape};
    use deckard_model::{Curve, ExprId, Expression, Point};
    let r = Ratchet { repeats: 4, shape: Shape::Flat, floor: 0, ceiling: 127 };
    let hits = ratchet_hits(Note { expr: Some(ExprId(0)), ..ratcheted(0, PPQ, 100, r) });
    assert_eq!(hits.iter().map(|e| e.expr_at).collect::<Vec<_>>(), [0, PPQ / 4, PPQ / 2, 3 * PPQ / 4]);
    // Pressure rising 0 to 1 over the note: a hit from halfway reads a half.
    let mut p = project();
    let ramp = vec![Point { at: 0, value: 0.0, curve: 0.0 }, Point { at: PPQ, value: 1.0, curve: 0.0 }];
    p.expressions = vec![std::sync::Arc::new(Expression { pressure: Some(Curve { points: ramp }), ..Default::default() })];
    let mut e = click_engine_for(&p, 1000);
    e.touch_start(0, 0, 60, Some(1000), 1.0, Some(ExprId(0)), PPQ / 2);
    let pressure = e.sources(0, 0)[2].unwrap();
    assert!((pressure - 0.5).abs() < 0.01, "{pressure}");
}

#[test]
fn hits_past_the_patterns_end_do_not_play() {
    use deckard_model::ratchet::{Ratchet, Shape};
    let mut p = project();
    let bar = own_length(&p, &p.groups[0].patterns[0]);
    // From a quarter before the end, two quarters long, a hit every eighth: two fit.
    let r = Ratchet { repeats: 4, shape: Shape::Flat, floor: 0, ceiling: 127 };
    p.groups[0].patterns[0].notes = vec![ratcheted(bar - PPQ, 2 * PPQ, 100, r)];
    let t = timeline(&p, Mode::Arrangement);
    let ticks: Vec<i64> = t.events.iter().filter(|e| e.midi.is_none() && e.cue.is_none() && e.tick < 2 * bar).map(|e| e.tick).collect();
    let first: Vec<i64> = ticks.iter().copied().filter(|&x| x < bar).collect();
    assert_eq!(first, [bar - PPQ, bar - PPQ / 2]);
    assert!(ticks.iter().all(|&x| x % bar >= bar - PPQ), "none fires into the next pass: {ticks:?}");
}

#[test]
fn a_ratchets_hits_take_one_roll() {
    use deckard_model::ratchet::{Ratchet, Shape};
    let r = Ratchet { repeats: 8, shape: Shape::Flat, floor: 0, ceiling: 127 };
    let hits = ratchet_hits(Note { probability: 50, ..ratcheted(0, PPQ, 100, r) });
    assert_eq!(hits.len(), 8);
    let mut seen = [false; 2];
    for pass in 0..64u64 {
        let plays: Vec<bool> =
            hits.iter().enumerate().map(|(i, e)| chance(e.probability, e.roll.map_or(i, |k| k as usize), pass, 7)).collect();
        assert!(plays.iter().all(|&x| x == plays[0]), "pass {pass}: {plays:?}");
        seen[usize::from(plays[0])] = true;
    }
    assert_eq!(seen, [true, true], "it plays on some passes and not on others");
}

/// Pressure on a held note sent out (a pad, or a key passed thru) goes out with it
/// as poly aftertouch on its key; a note not sent out, or released, sends none.
#[test]
fn pressure_on_a_held_note_goes_out_as_poly_aftertouch() {
    let mut p = project();
    p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
    p.groups[0].sounds[0].channel.midi_out_channel = 2;
    p.groups[0].sounds[0].channel.transpose = -12;
    let mut e = click_engine_for(&p, 1000);
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    e.midi_tx = Some(tx);
    let pressure = |pitch, value| Live::Expression { group: 0, sound: 0, pitch, kind: ExprKind::Dim(Dim::Pressure), value };
    e.play_live(Live::NoteIn { group: 0, sound: 0, pitch: 64, velocity: 90, thru: true });
    e.play_live(pressure(64, 0.5));
    e.play_live(Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
    e.play_live(pressure(60, 1.0));
    e.play_live(pressure(61, 1.0));
    e.play_live(Live::NoteRelease { group: 0, sound: 0, pitch: 64 });
    e.play_live(pressure(64, 0.2));
    let msgs: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
    assert_eq!(msgs, vec![vec![0x92, 52, 90], vec![0xa2, 52, 64], vec![0x92, 48, 100], vec![0xa2, 48, 127], vec![0x82, 52, 0]]);
}
