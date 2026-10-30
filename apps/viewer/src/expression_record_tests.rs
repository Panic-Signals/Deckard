//! Recording expression (docs/superpowers/plans/2026-10-24-mpe-capture.md, Task 9): a
//! held note's pressure, slide and pitch become its curves; controllers played while
//! recording overwrite their lane as they're written.

use crate::record::ControlWriter;
use deckard_doc::Document;
use deckard_engine::{Mode, Recorded, RecordedControl, RecordedExpr};
use deckard_model::{Dim, ExprKind, Expression, LaneRef, MidiEvent, MidiKind, Note};
use std::time::{Duration, Instant};

/// A new project: group 0's empty pattern 0 (a bar) in scene 0, focused.
fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_pattern(0, 0, "Pattern 1").unwrap();
    doc.borrow_mut().assign_pattern(0, 0, Some(0)).unwrap();
    (app.group, app.pattern, app.scene) = (0, 0, 0);
    app
}

/// Recorded events in the order played, numbered as the engine numbers them.
#[derive(Default)]
struct Feed {
    notes: Vec<Recorded>,
    exprs: Vec<RecordedExpr>,
    controls: Vec<RecordedControl>,
}

impl Feed {
    fn seq(&self) -> u64 {
        (self.notes.len() + self.exprs.len() + self.controls.len()) as u64
    }
    fn on(&mut self, pitch: i64, tick: f64) {
        let seq = self.seq();
        self.notes.push(Recorded {
            group: 0,
            sound: 0,
            pitch,
            velocity: 100,
            length: i64::MAX,
            tick,
            mode: Mode::Scene(0),
            seq,
            played: tick,
        });
    }
    fn off(&mut self, pitch: i64, tick: f64) {
        let seq = self.seq();
        self.notes.push(Recorded { group: 0, sound: 0, pitch, velocity: 0, length: 0, tick, mode: Mode::Scene(0), seq, played: tick });
    }
    fn expr(&mut self, pitch: i64, kind: ExprKind, value: f32, tick: f64) {
        let seq = self.seq();
        self.exprs.push(RecordedExpr { group: 0, sound: 0, pitch, kind, value, tick, mode: Mode::Scene(0), seq, played: tick });
    }
    fn cc(&mut self, number: u8, value: u8, tick: f64) {
        let seq = self.seq();
        self.controls.push(cc(number, value, tick, seq));
    }
    fn into(self, app: &mut crate::App, now: Instant) {
        app.take_in_all(self.notes, self.exprs, self.controls, now);
    }
}

fn cc(number: u8, value: u8, tick: f64, seq: u64) -> RecordedControl {
    RecordedControl { group: 0, sound: 0, kind: MidiKind::Controller { number, value }, tick, mode: Mode::Scene(0), seq }
}

fn notes(app: &crate::App) -> Vec<Note> {
    app.doc.as_ref().unwrap().borrow().project().groups[0].patterns[0].notes.clone()
}

fn expression(app: &crate::App, n: &Note) -> Expression {
    app.doc.as_ref().unwrap().borrow().expression(n.expr.expect("the note has expression")).unwrap().clone()
}

const PRESSURE: ExprKind = ExprKind::Dim(Dim::Pressure);

#[test]
fn an_mpe_note_records_its_curves() {
    let mut app = app();
    let mut f = Feed::default();
    f.on(60, 0.0);
    for i in 0..200 {
        f.expr(60, PRESSURE, i as f32 / 199.0, i as f64 * 4.0);
    }
    for i in 0..20 {
        f.expr(60, ExprKind::Dim(Dim::Slide), 0.3, 800.0 + i as f64);
        f.expr(60, ExprKind::Dim(Dim::Pitch), i as f32 / 10.0, 800.0 + i as f64 * 5.0);
    }
    f.expr(60, ExprKind::Release, 0.5, 959.0);
    f.off(60, 960.0);
    f.into(&mut app, Instant::now());
    let n = notes(&app);
    assert_eq!(n.len(), 1);
    let e = expression(&app, &n[0]);
    let p = e.pressure.expect("a pressure curve").points;
    assert!((2..=10).contains(&p.len()), "{} points", p.len());
    assert_eq!(p.last().unwrap().value, 1.0);
    assert!(e.slide.is_some() && e.pitch.is_some());
    assert_eq!(e.release_velocity, Some(64));
}

#[test]
fn two_held_notes_keep_their_own_pressure() {
    let mut app = app();
    let mut f = Feed::default();
    f.on(60, 0.0);
    f.on(64, 0.0);
    for i in 0..50 {
        f.expr(60, PRESSURE, 0.2, i as f64 * 10.0);
        f.expr(64, PRESSURE, 0.8, i as f64 * 10.0 + 5.0);
    }
    f.off(60, 600.0);
    f.off(64, 600.0);
    f.into(&mut app, Instant::now());
    for n in notes(&app) {
        let want = if n.pitch == 60 { 0.2 } else { 0.8 };
        let p = expression(&app, &n).pressure.unwrap().points;
        assert!(p.iter().all(|x| x.value == want), "{}: {p:?}", n.pitch);
    }
}

/// Review Focus 3.
#[test]
fn a_note_held_across_the_loop_keeps_increasing_offsets() {
    let mut app = app();
    let bar = app.doc.as_ref().unwrap().borrow().project().bar_length() as f64;
    let start = bar - 960.0;
    let mut f = Feed::default();
    f.on(60, start);
    // Played before the note (by the song position): nowhere on it.
    f.expr(60, PRESSURE, 0.9, start - 10.0);
    for i in 0..20 {
        f.expr(60, PRESSURE, if i % 2 == 0 { 0.1 } else { 0.7 }, start + i as f64 * 100.0);
    }
    f.off(60, start + 2000.0);
    f.into(&mut app, Instant::now());
    let n = notes(&app);
    assert_eq!(n[0].position, start as i64);
    let p = expression(&app, &n[0]).pressure.unwrap().points;
    assert!(p.iter().all(|x| x.at >= 0), "{p:?}");
    assert!(p.windows(2).all(|w| w[0].at < w[1].at), "{p:?}");
    assert_eq!(p.last().unwrap().at, 1900);
    assert!(p.iter().all(|x| x.value != 0.9));
}

/// A project to write lanes into: group 0's pattern 0 (a bar) in scene 0.
fn doc() -> Document {
    let mut d = Document::new_project().unwrap();
    d.add_pattern(0, 0, "Pattern 1").unwrap();
    d.assign_pattern(0, 0, Some(0)).unwrap();
    d
}

/// The CC `number` lane of group 0's pattern 0: (position, value), in order.
fn lane(d: &Document, number: u8) -> Vec<(i64, u8)> {
    let mut out: Vec<_> = d.project().groups[0].patterns[0]
        .midi
        .iter()
        .filter_map(|e| match e.kind {
            MidiKind::Controller { number: n, value } if n == number => Some((e.position, value)),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

/// Old CC `number` events at 100, 200, … 900.
fn old(d: &mut Document, number: u8) {
    let ev: Vec<MidiEvent> =
        (1..=9).map(|i| MidiEvent { sound: 0, position: i * 100, kind: MidiKind::Controller { number, value: 1 } }).collect();
    d.edit_midi("Old", 0, 0, &[], &ev).unwrap();
}

const MS: Duration = Duration::from_millis(1);

#[test]
fn a_second_loop_pass_replaces_the_first() {
    let mut d = doc();
    let bar = d.project().bar_length();
    let (mut w, t0) = (ControlWriter::default(), Instant::now());
    let mut seq = 0;
    for i in 0..16 {
        let (tick, now) = ((i * 240) as f64, t0 + i as u32 * 10 * MS);
        w.add(&mut d, cc(1, i as u8, tick, seq), now);
        w.tick(&mut d, Mode::Scene(0), tick + 1.0, now);
        seq += 1;
    }
    w.tick(&mut d, Mode::Scene(0), (bar - 1) as f64, t0 + 160 * MS);
    for i in 0..12 {
        let (tick, now) = ((bar + i * 320) as f64, t0 + (170 + i as u32 * 10) * MS);
        w.add(&mut d, cc(1, 100 + i as u8, tick, seq), now);
        w.tick(&mut d, Mode::Scene(0), tick + 1.0, now);
        seq += 1;
    }
    w.tick(&mut d, Mode::Scene(0), (2 * bar - 1) as f64, t0 + 290 * MS);
    w.finish(&mut d);
    let l = lane(&d, 1);
    assert_eq!(l, (0..12).map(|i| (i * 320, 100 + i as u8)).collect::<Vec<_>>());
    assert!(d.project().groups[0].sounds[0].lanes.contains(&LaneRef { kind: 4, number: 1 }), "its lane is shown");
}

#[test]
fn writing_clears_ahead_as_the_playhead_passes() {
    let mut d = doc();
    old(&mut d, 1);
    let (mut w, t0) = (ControlWriter::default(), Instant::now());
    // Two values at one tick: the last is kept.
    w.add(&mut d, cc(1, 98, 100.0, 0), t0);
    w.add(&mut d, cc(1, 99, 100.0, 1), t0);
    w.tick(&mut d, Mode::Scene(0), 300.0, t0 + 50 * MS);
    w.tick(&mut d, Mode::Scene(0), 500.0, t0 + 100 * MS);
    assert_eq!(lane(&d, 1), vec![(100, 99), (500, 1), (600, 1), (700, 1), (800, 1), (900, 1)]);
}

#[test]
fn an_idle_controller_stops_overwriting() {
    let mut d = doc();
    old(&mut d, 1);
    let bar = d.project().bar_length();
    let (mut w, t0) = (ControlWriter::default(), Instant::now());
    w.add(&mut d, cc(1, 99, 100.0, 0), t0);
    w.tick(&mut d, Mode::Scene(0), 500.0, t0 + 100 * MS);
    w.tick(&mut d, Mode::Scene(0), (bar - 1) as f64, t0 + 400 * MS);
    assert_eq!(lane(&d, 1), vec![(100, 99), (500, 1), (600, 1), (700, 1), (800, 1), (900, 1)]);
}

#[test]
fn an_untouched_lane_is_kept() {
    let mut d = doc();
    old(&mut d, 74);
    let before = lane(&d, 74);
    let bar = d.project().bar_length();
    let (mut w, t0) = (ControlWriter::default(), Instant::now());
    for i in 0..16 {
        let (tick, now) = ((i * 240) as f64, t0 + i as u32 * 10 * MS);
        w.add(&mut d, cc(1, 5, tick, i as u64), now);
        w.tick(&mut d, Mode::Scene(0), tick + 1.0, now);
    }
    w.tick(&mut d, Mode::Scene(0), (bar - 1) as f64, t0 + 160 * MS);
    w.finish(&mut d);
    assert_eq!(lane(&d, 74), before);
    assert_eq!(lane(&d, 1).len(), 16);
}

#[test]
fn a_pass_is_one_undo_step() {
    let mut app = app();
    app.set_recording(true, false);
    let t0 = Instant::now();
    let mut f = Feed::default();
    f.on(60, 0.0);
    for i in 0..10 {
        f.expr(60, PRESSURE, i as f32 / 10.0, i as f64 * 20.0);
        f.cc(1, i as u8 * 10, i as f64 * 20.0);
    }
    f.off(60, 480.0);
    f.into(&mut app, t0);
    app.tick_controls(Mode::Scene(0), 481.0, t0 + 10 * MS);
    app.set_recording(false, false);
    let doc = app.doc.clone().unwrap();
    {
        let d = doc.borrow();
        let pat = &d.project().groups[0].patterns[0];
        assert!(pat.notes.len() == 1 && pat.notes[0].expr.is_some());
        assert_eq!(pat.midi.len(), 10);
    }
    assert!(doc.borrow_mut().undo());
    let d = doc.borrow();
    let pat = &d.project().groups[0].patterns[0];
    assert!(pat.notes.is_empty() && pat.midi.is_empty(), "{:?} {:?}", pat.notes, pat.midi);
}

/// Task 10: a note with expression shows a small mark in the grid.
#[test]
fn notes_with_expression_are_marked() {
    let Some(mut h) = crate::tests::harness(false, false) else { return };
    let (g, pi, s) = (h.state().group, h.state().pattern, h.state().sound);
    let doc = h.state().doc.clone().unwrap();
    {
        let mut d = doc.borrow_mut();
        let pat = &d.project().groups[g].patterns[pi];
        let (old, start) = (pat.notes.clone(), pat.start);
        let id = d.add_expression(Expression { release_velocity: Some(30), ..Expression::default() });
        let plain = Note { sound: s, position: start, length: 240, pitch: 60, velocity: 100, probability: 100, expr: None };
        let marked = Note { position: start + 480, expr: Some(id), ..plain };
        assert!(crate::layout::has_expression_mark(&marked) && !crate::layout::has_expression_mark(&plain));
        d.edit_notes("t", g, pi, &old, &[plain, marked]).unwrap();
    }
    h.run_steps(3);
    let marks: Vec<eframe::egui::Rect> =
        h.ctx.data(|d| d.get_temp(crate::layout::header_rect("expression marks"))).expect("the grid drawn");
    assert_eq!(marks.len(), 1, "{marks:?}");
}

/// Final review #4: a controller moving every frame writes its lane a few times a
/// second, not every frame.
#[test]
fn a_moving_controller_writes_its_lane_a_few_times_a_second() {
    let mut d = doc();
    let (mut w, t0) = (ControlWriter::default(), Instant::now());
    let before = d.revision();
    // A second of frames (100), a value in each.
    for i in 0..100 {
        let (tick, now) = ((i * 30) as f64, t0 + i as u32 * 10 * MS);
        w.add(&mut d, cc(1, (i % 128) as u8, tick, i as u64), now);
        w.tick(&mut d, Mode::Scene(0), tick + 1.0, now);
    }
    w.finish(&mut d);
    let edits = d.revision() - before;
    assert!(edits <= 12, "{edits} edits");
    assert_eq!(lane(&d, 1).len(), 100);
}
