//! The Expression editor's lanes (docs/superpowers/specs/2026-10-25-expression-editor-design.md):
//! Pitch, Slide and Pressure curves over the focused sound's notes, and Release bars.
//! What's here needs no egui state: the lanes' scales, which note an edit goes to, and
//! what's under the pointer.

use super::*;
use deckard_model::expression::{Curve, Dim, Expression};

/// A lane's values: `lo` at its bottom, `hi` at its top.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub lo: f32,
    pub hi: f32,
}

impl Scale {
    /// The height in `r` of value `v`.
    pub fn y(self, r: Rect, v: f32) -> f32 {
        r.bottom() - r.height() * (v - self.lo) / (self.hi - self.lo)
    }

    /// The value at height `y` in `r`, kept on the scale.
    pub fn value(self, r: Rect, y: f32) -> f32 {
        let v = self.lo + (r.bottom() - y) / r.height().max(1.0) * (self.hi - self.lo);
        v.clamp(self.lo.min(self.hi), self.hi.max(self.lo))
    }
}

/// Pitch: ± `pitch_range` semitones; Slide and Pressure: 0–1.
pub fn scale(d: Dim, pitch_range: f32) -> Scale {
    match d {
        Dim::Pitch => Scale { lo: -pitch_range, hi: pitch_range },
        Dim::Slide | Dim::Pressure => Scale { lo: 0.0, hi: 1.0 },
    }
}

/// The Pitch lane's choices of range, in semitones.
pub const PITCH_RANGES: [f32; 4] = [2.0, 12.0, 24.0, 48.0];

/// The smallest range holding every value (semitones).
pub fn fit_range(values: impl Iterator<Item = f32>) -> f32 {
    let most = values.fold(0.0f32, |m, v| m.max(v.abs()));
    PITCH_RANGES.into_iter().find(|r| most <= *r + 1e-4).unwrap_or(48.0)
}

/// Which of `notes` (each with its curve for the lane's dimension `d`) an edit at `tick`,
/// `value` goes to: those whose span holds the tick, the selected ones first, then the
/// one whose curve (or rest value) passes nearest.
pub fn pick(notes: &[(Note, Option<&Curve>)], selected: &[Note], tick: i64, value: f32, d: Dim) -> Option<usize> {
    let under: Vec<usize> =
        (0..notes.len()).filter(|&i| (notes[i].0.position..=notes[i].0.position + notes[i].0.length.max(1)).contains(&tick)).collect();
    let picked: Vec<usize> = under.iter().copied().filter(|&i| selected.contains(&notes[i].0)).collect();
    let from = if picked.is_empty() { under } else { picked };
    let at = |i: usize| notes[i].1.map_or(d.rest(), |c| c.value_at(tick - notes[i].0.position));
    from.into_iter().min_by(|&a, &b| (at(a) - value).abs().total_cmp(&(at(b) - value).abs()))
}

/// Which of `notes` a drawn stroke goes to at `tick`, `value`: the note it's on (`prev`)
/// while the tick is inside it, else [`pick`]'s.
pub fn stroke_note(
    prev: Option<Note>,
    notes: &[(Note, Option<&Curve>)],
    selected: &[Note],
    tick: i64,
    value: f32,
    d: Dim,
) -> Option<usize> {
    let inside = |n: &Note| (n.position..=n.position + n.length.max(1)).contains(&tick);
    prev.and_then(|m| notes.iter().position(|(n, _)| *n == m && inside(n))).or_else(|| pick(notes, selected, tick, value, d))
}

/// Whether `drag` lives on: an Expression drag only while its lane is shown (`lane`),
/// there's a pattern to edit and the button is `down`.
pub(super) fn keep_drag(drag: &Option<NoteDrag>, lane: Lane, editable: bool, down: bool) -> bool {
    match drag {
        Some(NoteDrag::Expr(d)) => d.lane == lane && editable && down,
        _ => true,
    }
}

/// What the pointer is over on a curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// A breakpoint, by index.
    Point(usize),
    /// The segment from point `i` to the next.
    Segment(usize),
    Empty,
}

/// Pixels from a breakpoint (or the line) that still count as on it.
const NEAR: f32 = 6.0;

/// What `pos` is over on curve `c`: `x_of` and `y_of` place a point's tick (from the
/// note's start) and value, `tick` is the pointer's (from the note's start).
pub fn hit(c: &Curve, x_of: &dyn Fn(i64) -> f32, y_of: &dyn Fn(f32) -> f32, tick: i64, pos: Pos2) -> Hit {
    let near = c
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| (i, Pos2::new(x_of(p.at), y_of(p.value)).distance(pos)))
        .filter(|(_, d)| *d <= NEAR)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((i, _)) = near {
        return Hit::Point(i);
    }
    let seg = c.points.windows(2).position(|w| (w[0].at..=w[1].at).contains(&tick));
    match seg {
        Some(i) if (y_of(c.value_at(tick)) - pos.y).abs() <= NEAR => Hit::Segment(i),
        _ => Hit::Empty,
    }
}

/// How a value reads while it's dragged (`None`: release velocity, 1–127).
pub fn readout(d: Option<Dim>, v: f32) -> String {
    match d {
        Some(Dim::Pitch) => format!("Pitch {:+.2} st", v),
        Some(d) => format!("{} {} %", d.name(), (v * 100.0).round() as i32),
        None => format!("Release {}", v.round() as i32),
    }
}

/// A drag in an Expression lane, until the button is released: the notes' expression as
/// it's being edited, and what was grabbed.
#[derive(Debug, Clone)]
pub(super) struct ExprDrag {
    lane: Lane,
    edits: Vec<(Note, Expression)>,
    grab: Grab,
}

#[derive(Debug, Clone)]
enum Grab {
    /// Breakpoint `i` of `edits[0]`'s curve, from `base`.
    Point { base: Expression, i: usize },
    /// The segment from point `i`, grabbed at value `from`.
    Segment { base: Expression, i: usize, from: f32 },
    /// ⌥: the segment from point `i` bent, grabbed at height `y` with curve `from`.
    Bend { base: Expression, i: usize, y: f32, from: f32 },
    /// Draw mode: what each note was, and the values drawn on it (ticks from its start).
    Draw { bases: Vec<(Note, Expression)>, samples: Vec<Vec<(i64, f32)>>, on: Option<Note> },
    /// Release velocities set so far.
    Release { values: Vec<(Note, u8)> },
}

/// Where the lane is and what it shows.
pub(super) struct Area<'a> {
    pub lane: Rect,
    /// The lane less its top and bottom margins: where values go.
    pub inner: Rect,
    pub x_of: &'a dyn Fn(f64) -> f32,
    pub tick_at: &'a dyn Fn(f32) -> f64,
    /// The focused sound's notes.
    pub notes: &'a [Note],
    /// A pattern is shown (there's something to edit).
    pub editable: bool,
    pub color: Color32,
}

/// The dimension lane `l` shows (`None` for Release); `None` outside the editor.
pub(super) fn dim_of(l: Lane) -> Option<Option<Dim>> {
    match l {
        Lane::Expr(d) => Some(Some(d)),
        Lane::Release => Some(None),
        _ => None,
    }
}

/// Note `n`'s expression in `p` (empty when it has none).
pub(super) fn expression_of(p: &Project, n: &Note) -> Expression {
    n.expr.and_then(|id| p.expressions.get(id.0 as usize)).map(|e| (**e).clone()).unwrap_or_default()
}

/// Note `n`'s expression as shown: as being edited, else as it is.
pub(super) fn shown(p: &Project, drag: &Option<NoteDrag>, n: &Note) -> Expression {
    if let Some(NoteDrag::Expr(d)) = drag
        && let Some((_, e)) = d.edits.iter().find(|(m, _)| m == n)
    {
        return e.clone();
    }
    expression_of(p, n)
}

/// Draw and edit the Expression lane `app.lane` (Pitch, Slide, Pressure or Release).
pub(super) fn lane(ui: &mut Ui, app: &App, p: &Project, vp: &Painter, a: Area, drag: &mut Option<NoteDrag>, acts: &mut Vec<Act>) {
    let Some(dim) = dim_of(app.lane) else { return };
    let resp = match dim {
        Some(d) => curves(ui, app, p, vp, &a, d, drag, acts),
        None => release(ui, app, p, vp, &a, drag, acts),
    };
    if let Some(resp) = resp {
        resp.context_menu(|ui| clear_menu(ui, app, p, &a, dim, acts));
    }
}

fn commit(app: &App, p: &Project, edits: Vec<(Note, Expression)>, acts: &mut Vec<Act>) {
    let edits: Vec<(Note, Expression)> = edits.into_iter().filter(|(n, e)| expression_of(p, n) != *e).collect();
    if !edits.is_empty() {
        acts.push(Act::SetExpressions { g: app.group, p: app.pattern, edits });
    }
}

/// The notes a menu item clears: the selected ones of the sound, else all of them.
fn clear_menu(ui: &mut Ui, app: &App, p: &Project, a: &Area, dim: Option<Dim>, acts: &mut Vec<Act>) {
    let sel: Vec<Note> = a.notes.iter().filter(|n| app.selected.contains(n)).copied().collect();
    let (which, of) = if sel.is_empty() { (a.notes.to_vec(), "all notes") } else { (sel, "the selected notes") };
    let name = dim.map_or("Release", |d| d.name());
    let mut clear = |all: bool| {
        let edits = which
            .iter()
            .map(|n| {
                let mut e = expression_of(p, n);
                match (all, dim) {
                    (true, _) => e = Expression::default(),
                    (false, Some(d)) => *e.curve_mut(d) = None,
                    (false, None) => e.release_velocity = None,
                }
                (*n, e)
            })
            .collect();
        commit(app, p, edits, acts);
    };
    if ui.button(format!("Clear {name} of {of}")).clicked() {
        clear(false);
        ui.close();
    }
    if ui.button(format!("Clear All Expression of {of}")).clicked() {
        clear(true);
        ui.close();
    }
}

/// The Pitch lane's range for the focused sound: chosen, else fitted to its curves.
fn pitch_range(ui: &Ui, app: &App, p: &Project, a: &Area, drag: &Option<NoteDrag>) -> (egui::Id, f32) {
    let key = egui::Id::new(("expr-pitch-range", app.group, app.sound));
    let chosen: Option<f32> = ui.data(|d| d.get_temp(key));
    let fitted = || {
        let es: Vec<Expression> = a.notes.iter().map(|n| shown(p, drag, n)).collect();
        fit_range(es.iter().filter_map(|e| e.pitch.as_ref()).flat_map(|c| c.points.iter().map(|x| x.value)).collect::<Vec<_>>().into_iter())
    };
    (key, chosen.unwrap_or_else(fitted))
}

#[allow(clippy::too_many_arguments)]
fn curves(
    ui: &mut Ui,
    app: &App,
    p: &Project,
    vp: &Painter,
    a: &Area,
    d: Dim,
    drag: &mut Option<NoteDrag>,
    acts: &mut Vec<Act>,
) -> Option<Response> {
    let (range_key, range) = pitch_range(ui, app, p, a, drag);
    let sc = scale(d, range);
    let inner = a.inner;
    let y_of = |v: f32| sc.y(inner, v);
    let any_selected = a.notes.iter().any(|n| app.selected.contains(n));
    if d == Dim::Pitch {
        vp.hline(a.lane.x_range(), y_of(0.0), Stroke::new(1.0, white(40)));
    }
    // Each note's span, and its curve over it (or a dashed line at its rest).
    let exprs: Vec<Expression> = a.notes.iter().map(|n| shown(p, drag, n)).collect();
    for (n, e) in a.notes.iter().zip(&exprs) {
        let (x0, x1) = ((a.x_of)(n.position as f64), (a.x_of)((n.position + n.length.max(1)) as f64));
        let on = app.selected.contains(n);
        let col = if on {
            Color32::WHITE
        } else if any_selected {
            with_alpha(a.color, 120)
        } else {
            a.color
        };
        vp.rect_filled(Rect::from_x_y_ranges(x0..=x1, a.lane.y_range()), 0.0, with_alpha(a.color, 14));
        vp.vline(x0, a.lane.y_range(), Stroke::new(1.0, with_alpha(a.color, 60)));
        match e.curve(d) {
            Some(c) => {
                let steps = (((x1 - x0) / 3.0).ceil() as usize).clamp(1, 2000);
                let line: Vec<Pos2> = (0..=steps)
                    .map(|k| {
                        let x = x0 + (x1 - x0) * k as f32 / steps as f32;
                        let t = ((a.tick_at)(x) as i64 - n.position).clamp(0, n.length.max(1));
                        Pos2::new(x, y_of(c.value_at(t)))
                    })
                    .collect();
                vp.add(egui::Shape::line(line, Stroke::new(1.5, col)));
                for pt in &c.points {
                    let at = Pos2::new((a.x_of)((n.position + pt.at) as f64), y_of(pt.value));
                    vp.rect_filled(Rect::from_center_size(at, Vec2::splat(if on { 5.0 } else { 4.0 })), 1.0, col);
                }
            }
            None => {
                let y = y_of(d.rest());
                let mut x = x0;
                while x < x1 {
                    vp.hline(x..=(x + 4.0).min(x1), y, Stroke::new(1.0, with_alpha(col, 70)));
                    x += 8.0;
                }
            }
        }
    }
    if !a.editable {
        return None;
    }
    let tip = if app.draw_mode() {
        "Drag across notes to draw"
    } else {
        "Click: add a point. Drag a point or a segment; ⌥-drag a segment to bend it; double-click a point to remove it"
    };
    let resp = ui.interact(a.lane, ui.id().with(("expr-lane", d.name())), Sense::click_and_drag()).on_hover_text(tip);
    let pointer = resp.interact_pointer_pos();
    let snap = |v: f32| if d == Dim::Pitch { (v * 100.0).round() / 100.0 } else { v };
    let value = |pos: Pos2| snap(sc.value(inner, pos.y));
    let tick = |pos: Pos2| (a.tick_at)(pos.x).round() as i64;
    let cands = |exprs: &[Expression]| -> Vec<(Note, Option<Curve>)> {
        a.notes.iter().zip(exprs).map(|(n, e)| (*n, e.curve(d).cloned())).collect()
    };
    // The note an edit at `pos` goes to, its expression and what's under the pointer.
    let target = |pos: Pos2| -> Option<(Note, Expression, Hit)> {
        let c = cands(&exprs);
        let refs: Vec<(Note, Option<&Curve>)> = c.iter().map(|(n, c)| (*n, c.as_ref())).collect();
        let i = pick(&refs, &app.selected, tick(pos), value(pos), d)?;
        let (n, e) = (a.notes[i], exprs[i].clone());
        let h = e.curve(d).map_or(Hit::Empty, |c| {
            let x_of = |t: i64| (a.x_of)((n.position + t) as f64);
            hit(c, &x_of, &y_of, tick(pos) - n.position, pos)
        });
        Some((n, e, h))
    };
    let lo_hi = (sc.lo, sc.hi);
    let rel = |n: &Note, pos: Pos2| (tick(pos) - n.position).clamp(0, n.length.max(1));
    let add_point = |n: &Note, mut e: Expression, pos: Pos2| -> (Expression, usize) {
        let c = e.curve_mut(d).get_or_insert_with(Curve::default);
        let i = c.insert(rel(n, pos), value(pos));
        (e, i)
    };
    if resp.drag_started()
        && let Some(at) = ui.input(|i| i.pointer.press_origin()).or(pointer)
    {
        *drag = if app.draw_mode() {
            Some(NoteDrag::Expr(ExprDrag {
                lane: app.lane,
                edits: Vec::new(),
                grab: Grab::Draw { bases: Vec::new(), samples: Vec::new(), on: None },
            }))
        } else {
            let alt = ui.input(|i| i.modifiers.alt);
            target(at).map(|(n, e, h)| {
                let grab = match h {
                    Hit::Point(i) => Grab::Point { base: e.clone(), i },
                    Hit::Segment(i) if alt => {
                        let from = e.curve(d).and_then(|c| c.points.get(i)).map_or(0.0, |x| x.curve);
                        Grab::Bend { base: e.clone(), i, y: at.y, from }
                    }
                    Hit::Segment(i) => Grab::Segment { base: e.clone(), i, from: value(at) },
                    Hit::Empty => {
                        let (made, i) = add_point(&n, e.clone(), at);
                        Grab::Point { base: made, i }
                    }
                };
                NoteDrag::Expr(ExprDrag { lane: app.lane, edits: vec![(n, e)], grab })
            })
        };
    }
    if let (Some(NoteDrag::Expr(x)), Some(pos)) = (drag.as_mut(), pointer)
        && resp.dragged()
    {
        let n = x.edits.first().map(|(n, _)| *n);
        match &mut x.grab {
            Grab::Point { base, i } => {
                let n = n.unwrap();
                let mut e = base.clone();
                if let Some(c) = e.curve_mut(d) {
                    c.move_point(*i, rel(&n, pos), value(pos), n.length.max(1));
                }
                x.edits[0].1 = e;
            }
            Grab::Segment { base, i, from } => {
                let mut e = base.clone();
                if let Some(c) = e.curve_mut(d) {
                    c.move_segment(*i, value(pos) - *from, lo_hi.0, lo_hi.1);
                }
                x.edits[0].1 = e;
            }
            Grab::Bend { base, i, y, from } => {
                let mut e = base.clone();
                if let Some(c) = e.curve_mut(d) {
                    c.bend(*i, *from + (*y - pos.y) / 50.0);
                }
                x.edits[0].1 = e;
            }
            Grab::Draw { bases, samples, on } => {
                let c = cands(&exprs);
                let refs: Vec<(Note, Option<&Curve>)> = c.iter().map(|(n, c)| (*n, c.as_ref())).collect();
                if let Some(k) = stroke_note(*on, &refs, &app.selected, tick(pos), value(pos), d) {
                    let n = a.notes[k];
                    *on = Some(n);
                    let j = bases.iter().position(|(m, _)| *m == n).unwrap_or_else(|| {
                        bases.push((n, expression_of(p, &n)));
                        samples.push(Vec::new());
                        bases.len() - 1
                    });
                    samples[j].push((rel(&n, pos), value(pos)));
                }
                x.edits = bases
                    .iter()
                    .zip(samples.iter())
                    .map(|((n, e), s)| {
                        let mut e = e.clone();
                        e.curve_mut(d).get_or_insert_with(Curve::default).draw(s, d.tolerance());
                        (*n, e)
                    })
                    .collect();
            }
            Grab::Release { .. } => {}
        }
    }
    // While dragging: the value under the pointer.
    if let (Some(NoteDrag::Expr(x)), Some(pos)) = (drag.as_ref(), pointer) {
        let text = match x.grab {
            Grab::Bend { i, .. } => {
                let c = x.edits[0].1.curve(d).and_then(|c| c.points.get(i)).map_or(0.0, |p| p.curve);
                format!("Curve {:+} %", (c * 100.0).round() as i32)
            }
            _ => readout(Some(d), value(pos)),
        };
        vp.text(Pos2::new(a.lane.left() + 8.0, a.lane.top() + 10.0), Align2::LEFT_CENTER, text, font(11.0), Color32::WHITE);
    }
    if resp.drag_stopped()
        && let Some(NoteDrag::Expr(x)) = drag.take()
    {
        commit(app, p, x.edits, acts);
    }
    if resp.double_clicked()
        && let Some(pos) = pointer
    {
        if let Some((n, mut e, Hit::Point(i))) = target(pos) {
            if let Some(c) = e.curve_mut(d) {
                c.remove(i);
            }
            if e.curve(d).is_some_and(|c| c.points.is_empty()) {
                *e.curve_mut(d) = None;
            }
            commit(app, p, vec![(n, e)], acts);
        }
    } else if resp.clicked()
        && let Some(pos) = pointer
        && let Some((n, e, h)) = target(pos)
        && !matches!(h, Hit::Point(_))
    {
        let (e, _) = add_point(&n, e, pos);
        commit(app, p, vec![(n, e)], acts);
    }
    // The Pitch lane's range, at its top right.
    if d == Dim::Pitch {
        let at = Rect::from_min_size(Pos2::new(a.lane.right() - 70.0, a.lane.top() + 1.0), Vec2::new(66.0, 16.0));
        menu_at(ui, at, format!("±{range} st"), |ui| {
            for r in PITCH_RANGES {
                if ui.selectable_label(r == range, format!("±{r} semitones")).clicked() {
                    ui.data_mut(|x| x.insert_temp(range_key, r));
                    ui.close();
                }
            }
        });
    }
    Some(resp)
}

/// Release velocity: a bar per note, like Velocity; a hollow stub where it has none.
fn release(
    ui: &mut Ui,
    app: &App,
    p: &Project,
    vp: &Painter,
    a: &Area,
    drag: &mut Option<NoteDrag>,
    acts: &mut Vec<Act>,
) -> Option<Response> {
    let inner = a.inner;
    let bar_x = |n: &Note| (a.x_of)(n.position as f64);
    let editing: Vec<(Note, u8)> = match drag {
        Some(NoteDrag::Expr(ExprDrag { grab: Grab::Release { values }, .. })) => values.clone(),
        _ => Vec::new(),
    };
    for n in a.notes {
        let edited = editing.iter().find(|(m, _)| m == n).map(|(_, v)| *v);
        let x = bar_x(n);
        let col = if edited.is_some() || app.selected.contains(n) { Color32::WHITE } else { a.color };
        match edited.or(expression_of(p, n).release_velocity) {
            Some(v) => {
                let h = inner.height() * f32::from(v) / 127.0;
                vp.rect_filled(Rect::from_min_max(Pos2::new(x, inner.bottom() - h), Pos2::new(x + 3.0, inner.bottom())), 0.0, col);
            }
            None => {
                let r = Rect::from_min_max(Pos2::new(x, inner.bottom() - 6.0), Pos2::new(x + 3.0, inner.bottom()));
                vp.rect_stroke(r, 0.0, Stroke::new(1.0, with_alpha(col, 140)), StrokeKind::Inside);
            }
        }
    }
    if !a.editable {
        return None;
    }
    let tip = if app.draw_mode() { "Drag across to draw release velocities" } else { "Drag a bar up or down" };
    let resp = ui.interact(a.lane, ui.id().with("expr-release"), Sense::click_and_drag()).on_hover_text(tip);
    let pointer = resp.interact_pointer_pos();
    let value_at = |y: f32| (((inner.bottom() - y) / inner.height().max(1.0)) * 127.0).round().clamp(1.0, 127.0) as u8;
    let nearest =
        |x: f32| a.notes.iter().copied().filter(|n| (bar_x(n) + 1.5 - x).abs() < 8.0).min_by_key(|n| (bar_x(n) + 1.5 - x).abs() as i64);
    if resp.drag_started()
        && let Some(at) = ui.input(|i| i.pointer.press_origin()).or(pointer)
    {
        let values = nearest(at.x).map(|n| (n, value_at(at.y))).into_iter().collect();
        *drag = Some(NoteDrag::Expr(ExprDrag { lane: app.lane, edits: Vec::new(), grab: Grab::Release { values } }));
    }
    if let (Some(NoteDrag::Expr(ExprDrag { grab: Grab::Release { values }, .. })), Some(pos)) = (drag.as_mut(), pointer)
        && resp.dragged()
    {
        if app.draw_mode() {
            let x0 = pos.x - resp.drag_delta().x;
            let (l, r) = (x0.min(pos.x) - 2.0, x0.max(pos.x) + 2.0);
            for n in a.notes.iter().filter(|n| (l..=r).contains(&(bar_x(n) + 1.5))) {
                match values.iter_mut().find(|(m, _)| m == n) {
                    Some((_, v)) => *v = value_at(pos.y),
                    None => values.push((*n, value_at(pos.y))),
                }
            }
        } else if let Some((_, v)) = values.first_mut() {
            *v = value_at(pos.y);
        }
        vp.text(
            Pos2::new(a.lane.left() + 8.0, a.lane.top() + 10.0),
            Align2::LEFT_CENTER,
            readout(None, f32::from(value_at(pos.y))),
            font(11.0),
            Color32::WHITE,
        );
    }
    let to_edits = |values: Vec<(Note, u8)>| -> Vec<(Note, Expression)> {
        values
            .into_iter()
            .map(|(n, v)| {
                let mut e = expression_of(p, &n);
                e.release_velocity = Some(v);
                (n, e)
            })
            .collect()
    };
    if resp.drag_stopped()
        && let Some(NoteDrag::Expr(ExprDrag { grab: Grab::Release { values }, .. })) = drag.take()
    {
        commit(app, p, to_edits(values), acts);
    }
    if resp.clicked()
        && let Some(pos) = pointer
        && let Some(n) = nearest(pos.x)
    {
        commit(app, p, to_edits(vec![(n, value_at(pos.y))]), acts);
    }
    Some(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::expression::Point;

    fn note(position: i64, pitch: i64) -> Note {
        Note { sound: 0, position, length: 960, pitch, velocity: 100, probability: 100, expr: None }
    }

    fn flat(v: f32) -> Curve {
        Curve { points: vec![Point { at: 0, value: v, curve: 0.0 }, Point { at: 960, value: v, curve: 0.0 }] }
    }

    #[test]
    fn the_edit_goes_to_the_nearest_curve() {
        let (lo, hi) = (flat(0.1), flat(0.8));
        let notes = [(note(0, 60), Some(&lo)), (note(0, 64), Some(&hi)), (note(2000, 67), Some(&hi))];
        assert_eq!(pick(&notes, &[], 480, 0.7, Dim::Pressure), Some(1));
        assert_eq!(pick(&notes, &[], 480, 0.2, Dim::Pressure), Some(0));
        assert_eq!(pick(&notes, &[], 1500, 0.2, Dim::Pressure), None, "between notes");
        assert_eq!(pick(&notes, &[], 2100, 0.0, Dim::Pressure), Some(2));
    }

    #[test]
    fn selected_notes_come_first() {
        let (lo, hi) = (flat(0.1), flat(0.8));
        let notes = [(note(0, 60), Some(&lo)), (note(0, 64), Some(&hi))];
        assert_eq!(pick(&notes, &[note(0, 60)], 480, 0.8, Dim::Pressure), Some(0));
        assert_eq!(pick(&notes, &[note(5000, 1)], 480, 0.8, Dim::Pressure), Some(1), "a selection elsewhere doesn't count");
    }

    #[test]
    fn a_note_without_a_curve_is_picked_at_its_rest() {
        let hi = flat(0.9);
        let notes = [(note(0, 60), None), (note(0, 64), Some(&hi))];
        assert_eq!(pick(&notes, &[], 100, 0.05, Dim::Slide), Some(0));
    }

    #[test]
    fn hits_prefer_points() {
        let c = Curve { points: vec![Point { at: 0, value: 0.0, curve: 0.0 }, Point { at: 100, value: 1.0, curve: 0.0 }] };
        let x_of = |t: i64| t as f32;
        let y_of = |v: f32| 100.0 - v * 100.0;
        assert_eq!(hit(&c, &x_of, &y_of, 98, Pos2::new(98.0, 3.0)), Hit::Point(1));
        assert_eq!(hit(&c, &x_of, &y_of, 50, Pos2::new(50.0, 52.0)), Hit::Segment(0));
        assert_eq!(hit(&c, &x_of, &y_of, 50, Pos2::new(50.0, 90.0)), Hit::Empty);
        assert_eq!(hit(&Curve::default(), &x_of, &y_of, 50, Pos2::new(50.0, 50.0)), Hit::Empty);
    }

    #[test]
    fn pitch_range_fits_the_curves() {
        assert_eq!(fit_range([].into_iter()), 2.0);
        assert_eq!(fit_range([0.5, -1.9].into_iter()), 2.0);
        assert_eq!(fit_range([-7.0].into_iter()), 12.0);
        assert_eq!(fit_range([30.0].into_iter()), 48.0);
        assert_eq!(fit_range([99.0].into_iter()), 48.0);
        let s = scale(Dim::Pitch, 12.0);
        let r = Rect::from_min_size(Pos2::ZERO, Vec2::new(10.0, 100.0));
        assert_eq!((s.y(r, 0.0), s.y(r, 12.0)), (50.0, 0.0));
        assert_eq!(s.value(r, 75.0), -6.0);
        assert_eq!(scale(Dim::Slide, 12.0).value(r, -40.0), 1.0, "kept on the scale");
    }

    #[test]
    fn readouts() {
        assert_eq!(readout(Some(Dim::Pressure), 0.724), "Pressure 72 %");
        assert_eq!(readout(Some(Dim::Pitch), 1.25), "Pitch +1.25 st");
        assert_eq!(readout(Some(Dim::Pitch), -0.5), "Pitch -0.50 st");
        assert_eq!(readout(None, 96.0), "Release 96");
    }

    /// Final review #2: a drawn stroke stays on its note while it's inside that note's
    /// span, whatever curve passes nearer.
    #[test]
    fn a_stroke_keeps_its_note() {
        let (lo, hi) = (flat(0.2), flat(0.7));
        let notes = [(note(0, 60), Some(&lo)), (note(0, 64), Some(&hi)), (note(960, 67), None)];
        assert_eq!(stroke_note(None, &notes, &[], 100, 0.25, Dim::Pressure), Some(0));
        assert_eq!(stroke_note(Some(note(0, 60)), &notes, &[], 400, 0.9, Dim::Pressure), Some(0), "rising past E's curve");
        assert_eq!(stroke_note(Some(note(0, 60)), &notes, &[], 1500, 0.9, Dim::Pressure), Some(2), "on to the next note");
    }

    /// Final review #4: an Expression drag lasts only while its lane is shown, editable,
    /// and the button is down.
    #[test]
    fn a_stale_drag_is_dropped() {
        let d = Some(NoteDrag::Expr(ExprDrag { lane: Lane::Release, edits: Vec::new(), grab: Grab::Release { values: Vec::new() } }));
        assert!(keep_drag(&d, Lane::Release, true, true));
        assert!(!keep_drag(&d, Lane::Velocity, true, true), "another lane");
        assert!(!keep_drag(&d, Lane::Expr(Dim::Pitch), true, true), "another expression lane");
        assert!(!keep_drag(&d, Lane::Release, false, true), "no pattern");
        assert!(!keep_drag(&d, Lane::Release, true, false), "button up");
        assert!(keep_drag(&None, Lane::Release, true, false));
        let other = Some(NoteDrag::Select { origin: Pos2::ZERO, to: Pos2::ZERO });
        assert!(keep_drag(&other, Lane::Velocity, true, false), "not ours");
    }
}
