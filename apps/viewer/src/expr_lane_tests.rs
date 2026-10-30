//! The Expression editor's lanes in the lane list (BUGS #18): listed while the sound's
//! notes have that expression, and removable.

use crate::layout::{Act, Lane, apply_acts, lane_list_of};
use deckard_model::{Curve, Dim, Expression, Note, Point};

/// A new project's first sound with two notes: one with pressure and a release
/// velocity, one with only a release velocity.
fn app_with_expression() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    apply_acts(&mut app, vec![Act::NewPattern(0, 0)]);
    let (g, pi) = (app.group, app.pattern);
    let mut d = app.doc.as_ref().unwrap().borrow_mut();
    let pressure = Curve { points: vec![Point { at: 0, value: 0.5, curve: 0.0 }] };
    let a = d.add_expression(Expression { pressure: Some(pressure), release_velocity: Some(30), ..Expression::default() });
    let b = d.add_expression(Expression { release_velocity: Some(40), ..Expression::default() });
    let note = |position, expr| Note { sound: 0, position, length: 240, pitch: 60, velocity: 100, probability: 100, expr: Some(expr) };
    d.edit_notes("t", g, pi, &[], &[note(0, a), note(480, b)]).unwrap();
    drop(d);
    app
}

fn expr_lanes(app: &crate::App) -> Vec<Lane> {
    lane_list_of(app).into_iter().filter(|l| matches!(l, Lane::Expr(_) | Lane::Release)).collect()
}

#[test]
fn only_the_expression_the_notes_have_is_listed() {
    let app = app_with_expression();
    assert_eq!(expr_lanes(&app), [Lane::Expr(Dim::Pressure), Lane::Release]);
}

#[test]
fn removing_an_expression_lane_clears_it_from_the_notes() {
    let mut app = app_with_expression();
    apply_acts(&mut app, vec![Act::RemoveLane(Lane::Expr(Dim::Pressure))]);
    assert_eq!(expr_lanes(&app), [Lane::Release]);
    {
        let d = app.doc.as_ref().unwrap().borrow();
        let notes = &d.project().groups[app.group].patterns[app.pattern].notes;
        let exprs: Vec<Expression> = notes.iter().map(|n| d.expression(n.expr.unwrap()).unwrap().clone()).collect();
        assert!(exprs.iter().all(|e| e.pressure.is_none()), "{exprs:?}");
        assert_eq!(exprs.iter().map(|e| e.release_velocity).collect::<Vec<_>>(), [Some(30), Some(40)], "the release stays");
    }
    apply_acts(&mut app, vec![Act::RemoveLane(Lane::Release)]);
    assert_eq!(expr_lanes(&app), []);
    let d = app.doc.as_ref().unwrap().borrow();
    assert!(d.project().groups[app.group].patterns[app.pattern].notes.iter().all(|n| n.expr.is_none()), "nothing left: no expression");
    drop(d);
    app.undo();
    assert_eq!(expr_lanes(&app), [Lane::Release], "one undo step each");
}

/// BUGS #17: erasing a span of a CC lane takes the sound's events in it, one undo step.
#[test]
fn erasing_a_span_of_a_cc_lane() {
    use deckard_model::{MidiEvent, MidiKind};
    let mut app = app_with_expression();
    let (g, pi) = (app.group, app.pattern);
    let cc = |position| MidiEvent { sound: 0, position, kind: MidiKind::Controller { number: 1, value: 64 } };
    let other = MidiEvent { sound: 1, position: 240, kind: MidiKind::Controller { number: 1, value: 64 } };
    let events = [cc(0), cc(240), cc(480), cc(720), other];
    app.doc.as_ref().unwrap().borrow_mut().edit_midi("t", g, pi, &[], &events).unwrap();
    apply_acts(&mut app, vec![Act::EraseSpan(Lane::Controller(1), 200, 500)]);
    let d = app.doc.as_ref().unwrap().borrow();
    let left: Vec<(usize, i64)> = d.project().groups[g].patterns[pi].midi.iter().map(|m| (m.sound, m.position)).collect();
    assert_eq!(left, [(0, 0), (1, 240), (0, 720)], "only this sound's, in the span");
}

/// BUGS #17: erasing a span of an expression lane takes the curves' points in it (and,
/// for Release, the release velocity of the notes starting in it).
#[test]
fn erasing_a_span_of_an_expression_lane() {
    let mut app = app_with_expression();
    let (g, pi) = (app.group, app.pattern);
    {
        let mut d = app.doc.as_ref().unwrap().borrow_mut();
        let pts = |ats: &[i64]| Curve { points: ats.iter().map(|&at| Point { at, value: 0.5, curve: 0.0 }).collect() };
        let n = d.project().groups[g].patterns[pi].notes[0];
        let e = Expression { pressure: Some(pts(&[0, 100, 200])), release_velocity: Some(30), ..Expression::default() };
        d.set_expressions(g, pi, &[(n, e)]).unwrap();
    }
    let exprs = |app: &crate::App| -> Vec<Expression> {
        let d = app.doc.as_ref().unwrap().borrow();
        let notes = d.project().groups[g].patterns[pi].notes.clone();
        notes.iter().map(|n| n.expr.and_then(|i| d.expression(i)).cloned().unwrap_or_default()).collect()
    };
    apply_acts(&mut app, vec![Act::EraseSpan(Lane::Expr(Dim::Pressure), 50, 150)]);
    let ats: Vec<i64> = exprs(&app)[0].pressure.as_ref().unwrap().points.iter().map(|p| p.at).collect();
    assert_eq!(ats, [0, 200]);
    apply_acts(&mut app, vec![Act::EraseSpan(Lane::Release, 400, 600)]);
    let releases: Vec<Option<u8>> = exprs(&app).iter().map(|e| e.release_velocity).collect();
    assert_eq!(releases, [Some(30), None], "the note at 480 loses its release");
}
