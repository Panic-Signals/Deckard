//! Per-note expression (MPE): a note's pitch bend, slide and pressure over its length,
//! and its release velocity. Maschine has no place for these; DECKARD keeps them in its
//! own block in the project file (`docs/deckard-block.md`).

/// The field of a note's `TimedEvent` that holds its [`ExprId`] in DECKARD's tree;
/// never written to Maschine's archive.
pub const EXPR_FIELD: &str = "deckard:expr";

/// A note's expression in the document's table (`Note::expr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExprId(pub u32);

/// One of the per-note dimensions drawn as curves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dim {
    /// Per-note pitch bend, in semitones.
    Pitch,
    /// Per-note Y (CC74), 0–1.
    Slide,
    /// Poly aftertouch / MPE pressure, 0–1.
    Pressure,
}

impl Dim {
    pub const ALL: [Dim; 3] = [Dim::Pitch, Dim::Slide, Dim::Pressure];

    pub fn name(self) -> &'static str {
        match self {
            Dim::Pitch => "Pitch",
            Dim::Slide => "Slide",
            Dim::Pressure => "Pressure",
        }
    }

    /// Where a note without a curve sits.
    pub fn rest(self) -> f32 {
        0.0
    }

    /// How far a thinned curve may stray from what was played.
    pub fn tolerance(self) -> f32 {
        match self {
            Dim::Pitch => 0.05,
            Dim::Slide | Dim::Pressure => 0.005,
        }
    }
}

/// What a live expression event sets: a dimension, or the release velocity (0–1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExprKind {
    Dim(Dim),
    Release,
}

/// A breakpoint: `at` ticks from the note's start; `curve` bends the segment to the
/// next point (0: straight).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub at: i64,
    pub value: f32,
    pub curve: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Curve {
    pub points: Vec<Point>,
}

impl Curve {
    /// The value at `at`: along each segment (bent by its point's `curve`), held before
    /// the first point and after the last (0 for an empty curve).
    pub fn value_at(&self, at: i64) -> f32 {
        let p = &self.points;
        let Some(first) = p.first() else { return 0.0 };
        if at <= first.at {
            return first.value;
        }
        for w in p.windows(2) {
            if at <= w[1].at {
                let span = (w[1].at - w[0].at).max(1) as f32;
                let t = bent((at - w[0].at) as f32 / span, w[0].curve, w[1].value > w[0].value);
                return w[0].value + (w[1].value - w[0].value) * t;
            }
        }
        p.last().unwrap().value
    }

    /// A point at `at` (replacing one on that tick): its index.
    pub fn insert(&mut self, at: i64, value: f32) -> usize {
        let i = self.points.partition_point(|p| p.at < at);
        match self.points.get_mut(i) {
            Some(p) if p.at == at => p.value = value,
            _ => self.points.insert(i, Point { at, value, curve: 0.0 }),
        }
        i
    }

    /// Point `i` moved to `at` (kept between its neighbours, and inside a note `len`
    /// long) and `value`.
    pub fn move_point(&mut self, i: usize, at: i64, value: f32, len: i64) {
        let lo = if i == 0 { 0 } else { self.points[i - 1].at + 1 };
        let hi = self.points.get(i + 1).map_or(len, |p| p.at - 1);
        let Some(p) = self.points.get_mut(i) else { return };
        p.at = at.clamp(lo, hi.max(lo));
        p.value = value;
    }

    /// The segment from point `i` to the next moved up by `dv`, each end kept in `lo..=hi`
    /// (an end already outside it isn't pulled in).
    pub fn move_segment(&mut self, i: usize, dv: f32, lo: f32, hi: f32) {
        for p in self.points.iter_mut().skip(i).take(2) {
            p.value = (p.value + dv).clamp(lo.min(p.value), hi.max(p.value));
        }
    }

    /// The segment from point `i` bent by `curve` (−1..1; 0 straight).
    pub fn bend(&mut self, i: usize, curve: f32) {
        if let Some(p) = self.points.get_mut(i) {
            p.curve = curve.clamp(-1.0, 1.0);
        }
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.points.len() {
            self.points.remove(i);
        }
    }

    /// Drawn `samples` (ticks in any order): the stretch they cover replaced by them,
    /// thinned within `tolerance`.
    pub fn draw(&mut self, samples: &[(i64, f32)], tolerance: f32) {
        let mut s = samples.to_vec();
        s.sort_by_key(|x| x.0);
        let (Some(a), Some(b)) = (s.first().map(|x| x.0), s.last().map(|x| x.0)) else { return };
        self.points.retain(|p| p.at < a || p.at > b);
        let at = self.points.partition_point(|p| p.at < a);
        self.points.splice(at..at, thin(&s, tolerance));
    }

    /// The part from `from` to `to`, starting at 0, with points at both edges.
    pub fn slice(&self, from: i64, to: i64) -> Curve {
        let p = &self.points;
        let curve = p.iter().find(|x| x.at == from).map_or(0.0, |x| x.curve);
        let mut points = vec![Point { at: 0, value: self.value_at(from), curve }];
        // The tail of a bent segment `from` cuts into isn't a bend of its own: sampled.
        if let Some(k) = p.windows(2).position(|w| w[0].at < from && from < w[1].at)
            && p[k].curve != 0.0
        {
            let end = p[k + 1].at.min(to);
            let step = ((end - from) / 256).max(1) as usize;
            let mut samples: Vec<(i64, f32)> = (from..end).step_by(step).map(|t| (t - from, self.value_at(t))).collect();
            samples.push((end - from, self.value_at(end)));
            points = thin(&samples, 0.001);
        }
        // A bent segment's head is the same bend, so a cut at `to` keeps it.
        points.extend(p.iter().filter(|x| x.at > from && x.at < to).map(|x| Point { at: x.at - from, ..*x }));
        if points.last().is_none_or(|x| x.at < to - from) {
            points.push(Point { at: to - from, value: self.value_at(to), curve: 0.0 });
        }
        Curve { points }
    }

    /// `a` (of a note `a_len` long) followed by `b`.
    pub fn join(a: &Curve, a_len: i64, b: &Curve) -> Curve {
        let mut points: Vec<Point> = a.points.iter().filter(|p| p.at < a_len).copied().collect();
        points.extend(b.points.iter().map(|p| Point { at: p.at + a_len, ..*p }));
        Curve { points }
    }
}

/// How far along a segment (`t`, 0..1) its value is, bent by `curve` (−1..1): a positive
/// amount bows the segment upward whichever way it goes (`rising`), a negative one down.
pub fn bent(t: f32, curve: f32, rising: bool) -> f32 {
    if curve == 0.0 {
        return t;
    }
    let sign = if rising { 1.0 } else { -1.0 };
    t.clamp(0.0, 1.0).powf(4f32.powf(-curve.clamp(-1.0, 1.0) * sign))
}

/// A note's expression. An absent curve means none was played, not 0.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Expression {
    pub pitch: Option<Curve>,
    pub slide: Option<Curve>,
    pub pressure: Option<Curve>,
    /// 1–127.
    pub release_velocity: Option<u8>,
}

impl Expression {
    pub fn curve(&self, d: Dim) -> Option<&Curve> {
        match d {
            Dim::Pitch => self.pitch.as_ref(),
            Dim::Slide => self.slide.as_ref(),
            Dim::Pressure => self.pressure.as_ref(),
        }
    }

    pub fn curve_mut(&mut self, d: Dim) -> &mut Option<Curve> {
        match d {
            Dim::Pitch => &mut self.pitch,
            Dim::Slide => &mut self.slide,
            Dim::Pressure => &mut self.pressure,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pitch.is_none() && self.slide.is_none() && self.pressure.is_none() && self.release_velocity.is_none()
    }

    /// Each curve sliced to `from..to` (a split note's part).
    pub fn slice(&self, from: i64, to: i64) -> Expression {
        let mut e = Expression { release_velocity: self.release_velocity, ..Expression::default() };
        for d in Dim::ALL {
            *e.curve_mut(d) = self.curve(d).map(|c| c.slice(from, to));
        }
        e
    }

    /// `a` (of a note `a_len` long) followed by `b` (a join). A curve only one has is
    /// held at its edge value across the other.
    pub fn join(a: &Expression, a_len: i64, b: &Expression) -> Expression {
        let mut e = Expression { release_velocity: b.release_velocity.or(a.release_velocity), ..Expression::default() };
        for d in Dim::ALL {
            *e.curve_mut(d) = match (a.curve(d), b.curve(d)) {
                (None, None) => None,
                (x, y) => Some(Curve::join(x.unwrap_or(&Curve::default()), a_len, y.unwrap_or(&Curve::default()))),
            };
        }
        e
    }
}

/// Played samples `(tick, value)` as breakpoints that keep the shape within
/// `tolerance` (Ramer–Douglas–Peucker), first and last kept, runs of one value
/// collapsed.
pub fn thin(samples: &[(i64, f32)], tolerance: f32) -> Vec<Point> {
    let mut s: Vec<(i64, f32)> = Vec::with_capacity(samples.len());
    for &x in samples {
        match s.last_mut() {
            Some(last) if last.0 == x.0 => *last = x,
            _ => s.push(x),
        }
    }
    if s.len() <= 2 {
        return s.iter().map(|&(at, value)| Point { at, value, curve: 0.0 }).collect();
    }
    let mut keep = vec![false; s.len()];
    keep[0] = true;
    keep[s.len() - 1] = true;
    let mut stack = vec![(0, s.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (t0, v0) = s[a];
        let (t1, v1) = s[b];
        let span = (t1 - t0).max(1) as f32;
        let (mut worst, mut at) = (0.0f32, a);
        for (i, &(t, v)) in s.iter().enumerate().take(b).skip(a + 1) {
            let d = (v - (v0 + (v1 - v0) * (t - t0) as f32 / span)).abs();
            if d > worst {
                (worst, at) = (d, i);
            }
        }
        if worst > tolerance {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    s.iter().zip(keep).filter(|(_, k)| *k).map(|(&(at, value), _)| Point { at, value, curve: 0.0 }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(points: &[(i64, f32)]) -> Curve {
        Curve { points: points.iter().map(|&(at, value)| Point { at, value, curve: 0.0 }).collect() }
    }

    #[test]
    fn a_ramp_thins_to_its_ends() {
        let s: Vec<(i64, f32)> = (0..1000).map(|i| (i, i as f32 / 999.0)).collect();
        let t = thin(&s, 0.005);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].at, t[1].at, t[1].value), (0, 999, 1.0));
    }

    #[test]
    fn a_sine_keeps_its_shape() {
        let s: Vec<(i64, f32)> = (0..2000).map(|i| (i, 0.5 + 0.5 * (i as f32 / 200.0).sin())).collect();
        let c = Curve { points: thin(&s, 0.005) };
        assert!(c.points.len() < 100, "{} points", c.points.len());
        for &(t, v) in &s {
            assert!((c.value_at(t) - v).abs() <= 0.005 + 1e-6, "at {t}: {} vs {v}", c.value_at(t));
        }
    }

    #[test]
    fn equal_runs_collapse() {
        let s = [(0, 0.3), (10, 0.3), (20, 0.3), (30, 0.8)];
        let c = Curve { points: thin(&s, 0.005) };
        assert!(c.points.len() <= 3, "{:?}", c.points);
        for (t, v) in s {
            assert!((c.value_at(t) - v).abs() < 1e-6);
        }
    }

    #[test]
    fn value_at_holds_and_interpolates() {
        let c = curve(&[(10, 0.0), (20, 1.0)]);
        assert_eq!((c.value_at(0), c.value_at(15), c.value_at(99)), (0.0, 0.5, 1.0));
    }

    #[test]
    fn slice_rebases() {
        let c = curve(&[(10, 0.0), (20, 1.0)]).slice(15, 20);
        assert_eq!(c, curve(&[(0, 0.5), (5, 1.0)]));
    }

    #[test]
    fn join_offsets_the_second() {
        let j = Curve::join(&curve(&[(0, 0.2)]), 100, &curve(&[(0, 0.7), (50, 0.1)]));
        assert_eq!(j, curve(&[(0, 0.2), (100, 0.7), (150, 0.1)]));
    }

    fn pts(c: &Curve) -> Vec<(i64, f32)> {
        c.points.iter().map(|p| (p.at, p.value)).collect()
    }

    #[test]
    fn value_at_follows_the_bend() {
        let mut c = curve(&[(0, 0.0), (100, 1.0)]);
        assert!((c.value_at(50) - 0.5).abs() < 1e-6, "straight");
        c.bend(0, 1.0);
        assert!(c.value_at(50) > 0.5 + 0.2, "a rising segment bows up: {}", c.value_at(50));
        assert_eq!((c.value_at(0), c.value_at(100)), (0.0, 1.0), "the ends stay");
        let mut f = curve(&[(0, 1.0), (100, 0.0)]);
        f.bend(0, 1.0);
        assert!(f.value_at(50) > 0.5 + 0.2, "a falling one bows up too: {}", f.value_at(50));
        f.bend(0, -1.0);
        assert!(f.value_at(50) < 0.5 - 0.2);
        let mut flat = curve(&[(0, 0.4), (100, 0.4)]);
        flat.bend(0, 1.0);
        assert!((flat.value_at(50) - 0.4).abs() < 1e-6);
        c.bend(0, 7.0);
        assert_eq!(c.points[0].curve, 1.0, "clamped");
    }

    #[test]
    fn inserting_keeps_points_in_order() {
        let mut c = curve(&[(0, 0.0), (100, 1.0)]);
        assert_eq!(c.insert(50, 0.2), 1);
        assert_eq!(c.insert(200, 0.9), 3);
        assert_eq!(c.insert(50, 0.3), 1, "a point on the same tick is replaced");
        assert_eq!(pts(&c), vec![(0, 0.0), (50, 0.3), (100, 1.0), (200, 0.9)]);
        let mut e = Curve::default();
        assert_eq!(e.insert(10, 0.5), 0);
    }

    #[test]
    fn a_point_stays_between_its_neighbours() {
        let mut c = curve(&[(0, 0.0), (50, 0.5), (100, 1.0)]);
        c.move_point(1, 500, 2.0, 120);
        assert_eq!(pts(&c)[1], (99, 2.0), "before the next; the value's the caller's to clamp");
        c.move_point(1, -5, 0.1, 120);
        assert_eq!(pts(&c)[1], (1, 0.1));
        c.move_point(2, 900, 1.0, 120);
        assert_eq!(pts(&c)[2], (120, 1.0), "inside the note");
        c.move_point(0, -9, 0.0, 120);
        assert_eq!(pts(&c)[0], (0, 0.0));
    }

    #[test]
    fn moving_a_segment_moves_both_ends() {
        let mut c = curve(&[(0, 0.2), (50, 0.5), (100, 0.9)]);
        c.move_segment(1, 0.3, 0.0, 1.0);
        assert_eq!(pts(&c), vec![(0, 0.2), (50, 0.8), (100, 1.0)]);
    }

    #[test]
    fn drawing_replaces_the_span() {
        let mut c = curve(&[(0, 0.1), (40, 0.9), (60, 0.9), (100, 0.1)]);
        let s: Vec<(i64, f32)> = (30..=70).map(|t| (t, 0.5)).collect();
        c.draw(&s, 0.005);
        assert_eq!(pts(&c), vec![(0, 0.1), (30, 0.5), (70, 0.5), (100, 0.1)]);
        let mut e = Curve::default();
        e.draw(&[(20, 0.3), (10, 0.6)], 0.005);
        assert_eq!(pts(&e), vec![(10, 0.6), (20, 0.3)], "drawn right to left");
    }

    #[test]
    fn removing_points() {
        let mut c = curve(&[(0, 0.1), (40, 0.9)]);
        c.remove(0);
        assert_eq!(pts(&c), vec![(40, 0.9)]);
        c.remove(5);
        assert_eq!(c.points.len(), 1);
    }

    /// Final review #1: a split keeps a bent segment's shape on both sides.
    #[test]
    fn slicing_keeps_bends() {
        let mut c = curve(&[(0, 0.0), (960, 1.0)]);
        c.bend(0, 1.0);
        for (from, to) in [(0, 480), (480, 960), (200, 700)] {
            let s = c.slice(from, to);
            for t in (from..=to).step_by(20) {
                assert!(
                    (s.value_at(t - from) - c.value_at(t)).abs() < 0.01,
                    "{from}..{to} at {t}: {} vs {}",
                    s.value_at(t - from),
                    c.value_at(t)
                );
            }
        }
    }

    /// Final review #6: an end already outside the lane's range isn't pulled into it.
    #[test]
    fn moving_a_segment_leaves_ends_outside_the_range() {
        let mut c = curve(&[(0, 7.0), (10, 1.0)]);
        c.move_segment(0, 0.5, -2.0, 2.0);
        assert_eq!(pts(&c), vec![(0, 7.0), (10, 1.5)]);
        c.move_segment(0, -1.0, -2.0, 2.0);
        assert_eq!(pts(&c), vec![(0, 6.0), (10, 0.5)]);
    }
}
