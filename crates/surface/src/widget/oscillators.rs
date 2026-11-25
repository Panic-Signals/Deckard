//! `oscillators`: a synth's oscillators drawn as waveforms, with their sum.
//!
//! Up to four sources. One source draws its wave in the accent. With several, each is a
//! thin muted line and their sum (levels applied, divided by the sum of the levels, at
//! least 1, so it fits the box) is the accent line. A touched source is drawn over the
//! rest in the text colour. At 360 px and wider the sources' names label the top left.
//! Pulses and saws are drawn from their corners, so edges are vertical and crisp.
//!
//! Inputs, per source `n` = 1..4 (a source is present when `wave<n>` is given):
//!
//! | input | |
//! |---|---|
//! | `wave<n>` | the waveform's name as the plug-in writes it, matched ignoring case, spaces and punctuation: sine/sin, triangle/tri, tri/saw (the average of a triangle and a saw), saw/sawtooth/ramp up, ramp/ramp down/reverse saw, square/pulse, narrow (25% pulse), very nrw/very narrow (10% pulse), noise. An unknown name draws nothing |
//! | `pw<n>` | pulse width 0..1 of square/pulse (default 0.5) |
//! | `octave<n>` | relative octave (default 0): 2^octave times the cycles of octave 0, which shows 2 (0.5 to 16 cycles) |
//! | `level<n>` | 0..1 (default 1); a level of 0 is a flat faint line |
//!
//! `noise` (optional, 0..1): a noise level added to the sum.
//!
//! Touch focus on `wave<n>`, `octave<n>`, `level<n>` or `pw<n>` lights source `n`; `noise`
//! lights the sum.

use mlua::Table;

use super::{Align, Canvas, num, text};

const MAX_SOURCES: usize = 4;
/// Cycles shown at octave 0.
const BASE_CYCLES: f32 = 2.0;
const SINE_STEPS: usize = 16;

/// A waveform's shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sine,
    Triangle,
    TriSaw,
    /// Rising, then a drop.
    Saw,
    /// Falling, then a jump up.
    Ramp,
    /// High for this share (0..1) of the cycle.
    Pulse(f32),
    Noise,
}

impl Shape {
    /// The shape a plug-in's waveform name means. `pw` is the width of a plain square or pulse.
    pub fn parse(name: &str, pw: f32) -> Option<Shape> {
        let key: String = name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect();
        Some(match key.as_str() {
            "sine" | "sin" => Shape::Sine,
            "triangle" | "tri" => Shape::Triangle,
            "trisaw" | "trisawtooth" | "trianglesaw" => Shape::TriSaw,
            "saw" | "sawtooth" | "rampup" => Shape::Saw,
            "ramp" | "rampdown" | "reversesaw" | "reversesawtooth" => Shape::Ramp,
            "square" | "pulse" => Shape::Pulse(pw.clamp(0.02, 0.98)),
            "narrow" => Shape::Pulse(0.25),
            "verynrw" | "verynarrow" => Shape::Pulse(0.1),
            "noise" => Shape::Noise,
            _ => return None,
        })
    }
}

/// A deterministic value in -1..1 for step `i`.
fn hash(i: u32, seed: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ seed.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x & 0xFFFF) as f32 / 32767.5 - 1.0
}

/// The wave's value (-1..1) at `phase` (any number; only the fraction counts).
pub fn sample(shape: Shape, phase: f32) -> f32 {
    let p = phase.rem_euclid(1.0);
    let tri = |p: f32| if p < 0.5 { 4.0 * p - 1.0 } else { 3.0 - 4.0 * p };
    match shape {
        Shape::Sine => -(std::f32::consts::TAU * p).cos(),
        Shape::Triangle => tri(p),
        Shape::TriSaw => (tri(p) + 2.0 * p - 1.0) / 2.0,
        Shape::Saw => 2.0 * p - 1.0,
        Shape::Ramp => 1.0 - 2.0 * p,
        Shape::Pulse(w) => {
            if p < w {
                1.0
            } else {
                -1.0
            }
        }
        Shape::Noise => hash((p * 32.0) as u32, 1),
    }
}

/// One cycle's corner points (phase 0..1, value), equal phases being a vertical edge.
fn cycle(shape: Shape) -> Vec<(f32, f32)> {
    match shape {
        Shape::Sine => (0..=SINE_STEPS).map(|i| (i as f32 / SINE_STEPS as f32, sample(shape, i as f32 / SINE_STEPS as f32))).collect(),
        Shape::Triangle => vec![(0.0, -1.0), (0.5, 1.0), (1.0, -1.0)],
        Shape::TriSaw => vec![(0.0, -1.0), (0.5, 0.5), (1.0, 0.0), (1.0, -1.0)],
        Shape::Saw => vec![(0.0, -1.0), (1.0, 1.0), (1.0, -1.0)],
        Shape::Ramp => vec![(0.0, 1.0), (1.0, -1.0), (1.0, 1.0)],
        Shape::Pulse(w) => vec![(0.0, 1.0), (w, 1.0), (w, -1.0), (1.0, -1.0), (1.0, 1.0)],
        Shape::Noise => Vec::new(),
    }
}

/// The wave across the box: points (x 0..1, value -1..1) for `cycles` cycles.
pub fn trace(shape: Shape, cycles: f32, seed: u32) -> Vec<(f32, f32)> {
    if shape == Shape::Noise {
        let n = ((cycles * 12.0) as usize).max(4);
        return (0..=n).map(|i| (i as f32 / n as f32, hash(i as u32, seed))).collect();
    }
    let one = cycle(shape);
    let mut out: Vec<(f32, f32)> = Vec::new();
    for k in 0..cycles.ceil() as usize {
        for &(p, v) in &one {
            let x = (k as f32 + p) / cycles;
            if x > 1.0 {
                if let Some(&(px, py)) = out.last() {
                    let t = (1.0 - px) / (x - px);
                    out.push((1.0, py + (v - py) * t));
                }
                return out;
            }
            if out.last() != Some(&(x, v)) {
                out.push((x, v));
            }
        }
    }
    out
}

/// A polyline's value at `x`, as approached from the left, or (`right`) from the right.
fn value_at(poly: &[(f32, f32)], x: f32, right: bool) -> f32 {
    let (Some(&first), Some(&last)) = (poly.first(), poly.last()) else { return 0.0 };
    let j = if right { poly.partition_point(|p| p.0 <= x) } else { poly.partition_point(|p| p.0 < x) };
    if j == 0 {
        return first.1;
    }
    if j >= poly.len() {
        return last.1;
    }
    let (a, b) = (poly[j - 1], poly[j]);
    if b.0 <= a.0 { b.1 } else { a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0) }
}

/// The sum of waves `(points, weight)`, times `norm`, keeping their vertical edges.
fn sum(waves: &[(Vec<(f32, f32)>, f32)], norm: f32) -> Vec<(f32, f32)> {
    let mut xs: Vec<f32> = waves.iter().flat_map(|(p, _)| p.iter().map(|q| q.0)).collect();
    xs.sort_by(f32::total_cmp);
    xs.dedup();
    let mut out = Vec::with_capacity(xs.len() + 8);
    for x in xs {
        let at = |right| waves.iter().map(|(p, w)| value_at(p, x, right) * w).sum::<f32>() * norm;
        let (l, r) = (at(false), at(true));
        out.push((x, l));
        if (l - r).abs() > 1e-4 {
            out.push((x, r));
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// 1..4: the number in its input names.
    pub n: usize,
    /// The name as written, for the label.
    pub name: String,
    /// `None` for a name that isn't known.
    pub shape: Option<Shape>,
    pub octave: f32,
    pub level: f32,
}

impl Source {
    fn cycles(&self) -> f32 {
        (BASE_CYCLES * self.octave.exp2()).clamp(0.5, 16.0)
    }

    /// The inputs this source depends on.
    fn inputs(&self) -> Vec<String> {
        ["wave", "octave", "level", "pw"].iter().map(|k| format!("{k}{}", self.n)).collect()
    }

    fn lit(&self, c: &Canvas) -> bool {
        self.inputs().iter().any(|i| c.lit(i))
    }

    fn ink(&self, c: &Canvas, colour: super::Paint) -> super::Paint {
        let names = self.inputs();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        c.ink(colour, &refs)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Oscillators {
    pub sources: Vec<Source>,
    pub noise: f32,
}

impl Oscillators {
    pub fn from_lua(t: &Table) -> mlua::Result<Oscillators> {
        let mut sources = Vec::new();
        for n in 1..=MAX_SOURCES {
            let Some(name) = text(t, &format!("wave{n}")) else { continue };
            let pw = num(t, &format!("pw{n}")).unwrap_or(0.5);
            sources.push(Source {
                n,
                shape: Shape::parse(&name, pw),
                name: name.trim().to_string(),
                octave: num(t, &format!("octave{n}")).unwrap_or(0.0),
                level: num(t, &format!("level{n}")).unwrap_or(1.0).clamp(0.0, 1.0),
            });
        }
        Ok(Oscillators { sources, noise: num(t, "noise").unwrap_or(0.0).clamp(0.0, 1.0) })
    }

    pub fn draw(&self, c: &mut Canvas) {
        let look = *c.look;
        let (w, h) = (c.r.w, c.r.h);
        c.fill(0, 0, w, h, look.panel);
        let (x0, pw) = (4.0, (w - 8) as f32);
        let (top, bottom) = (15.0, (h - 5) as f32);
        let (mid, amp) = ((top + bottom) / 2.0, (bottom - top) / 2.0);
        let px = |x: f32| x0 + x * pw;
        let screen =
            |pts: &[(f32, f32)], level: f32| -> Vec<(f32, f32)> { pts.iter().map(|&(x, v)| (px(x), mid - v * level * amp)).collect() };
        c.line(x0, mid, x0 + pw, mid, 1.0, look.separator);

        let drawn: Vec<&Source> = self.sources.iter().filter(|s| s.shape.is_some()).collect();
        let several = drawn.len() > 1 || (!drawn.is_empty() && self.noise > 0.0);
        let traces: Vec<Vec<(f32, f32)>> = drawn.iter().map(|s| trace(s.shape.unwrap_or(Shape::Sine), s.cycles(), s.n as u32)).collect();
        if several {
            for (s, tr) in drawn.iter().zip(&traces) {
                if s.level <= 0.0 {
                    c.line(x0, mid, x0 + pw, mid, 1.0, s.ink(c, look.grid));
                } else {
                    c.polyline(&screen(tr, s.level), 1.0, s.ink(c, look.muted));
                }
            }
            let mut waves: Vec<(Vec<(f32, f32)>, f32)> =
                drawn.iter().zip(&traces).filter(|(s, _)| s.level > 0.0).map(|(s, tr)| (tr.clone(), s.level)).collect();
            let mut total: f32 = drawn.iter().map(|s| s.level).sum();
            if self.noise > 0.0 {
                waves.push((trace(Shape::Noise, 16.0, 9), self.noise));
                total += self.noise;
            }
            let pts = sum(&waves, 1.0 / total.max(1.0));
            c.polyline(&screen(&pts, 1.0), 2.0, c.ink(look.accent, &["noise"]));
        } else if let Some((s, tr)) = drawn.first().zip(traces.first()) {
            if s.level <= 0.0 {
                c.line(x0, mid, x0 + pw, mid, 1.0, s.ink(c, look.grid));
            } else {
                c.polyline(&screen(tr, s.level), 2.0, s.ink(c, look.accent));
            }
        }
        for (s, tr) in drawn.iter().zip(&traces) {
            if s.lit(c) && s.level > 0.0 {
                c.polyline(&screen(tr, s.level), 1.5, look.text);
            }
        }

        if w >= 360 {
            let label: Vec<String> = self.sources.iter().map(|s| format!("{} {}", s.n, s.name.to_uppercase())).collect();
            c.text(4, 2, w - 8, 12, &label.join("  "), look.small, look.weak, Align::Left);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::golden;

    fn src(n: usize, name: &str, octave: f32, level: f32) -> Source {
        Source { n, name: name.into(), shape: Shape::parse(name, 0.5), octave, level }
    }

    fn three() -> Oscillators {
        Oscillators { sources: vec![src(1, "Saw", 0.0, 1.0), src(2, "Triangle", -1.0, 0.6), src(3, "Very Nrw", 1.0, 0.4)], noise: 0.0 }
    }

    #[test]
    fn saw_rises() {
        assert!(sample(Shape::Saw, 0.1) < sample(Shape::Saw, 0.6));
        assert!(sample(Shape::Saw, 0.99) > 0.9 && sample(Shape::Saw, 0.0) < -0.9);
        assert!(sample(Shape::Ramp, 0.1) > sample(Shape::Ramp, 0.6));
    }

    #[test]
    fn square_has_two_levels() {
        let v: Vec<f32> = (0..100).map(|i| sample(Shape::Pulse(0.5), i as f32 / 100.0)).collect();
        assert!(v.iter().all(|&x| x == 1.0 || x == -1.0));
        assert!(v.contains(&1.0) && v.contains(&-1.0));
    }

    #[test]
    fn narrow_is_high_a_quarter_of_the_cycle() {
        let Some(Shape::Pulse(w)) = Shape::parse("Narrow", 0.5) else { panic!("a pulse") };
        let high = (0..1000).filter(|&i| sample(Shape::Pulse(w), i as f32 / 1000.0) > 0.0).count();
        assert!((high as f32 / 1000.0 - 0.25).abs() < 0.01);
    }

    #[test]
    fn tri_saw_lies_between_triangle_and_saw() {
        for i in 0..100 {
            let p = i as f32 / 100.0;
            let (t, s, m) = (sample(Shape::Triangle, p), sample(Shape::Saw, p), sample(Shape::TriSaw, p));
            assert!(m >= t.min(s) - 1e-5 && m <= t.max(s) + 1e-5, "phase {p}");
        }
    }

    #[test]
    fn names_are_matched_loosely() {
        assert_eq!(Shape::parse("Tri/Saw", 0.5), Some(Shape::TriSaw));
        assert_eq!(Shape::parse(" RAMP up ", 0.5), Some(Shape::Saw));
        assert_eq!(Shape::parse("Very Nrw", 0.5), Some(Shape::Pulse(0.1)));
        assert_eq!(Shape::parse("Square", 0.3), Some(Shape::Pulse(0.3)));
        assert_eq!(Shape::parse("wobble", 0.5), None);
    }

    #[test]
    fn saw_traces_have_vertical_edges() {
        let t = trace(Shape::Saw, 2.0, 1);
        assert_eq!(t.iter().filter(|p| (p.0 - 0.5).abs() < 1e-6).count(), 2);
        assert!(t.iter().all(|p| p.0 <= 1.0 + 1e-6));
        assert!(trace(Shape::Sine, 0.5, 1).last().is_some_and(|p| (p.0 - 1.0).abs() < 1e-6));
    }

    #[test]
    fn oscillators_saw() {
        let o = Oscillators { sources: vec![src(1, "Saw", 0.0, 1.0)], noise: 0.0 };
        golden::check("oscillators-saw", 2, None, |cv| o.draw(cv));
    }

    #[test]
    fn oscillators_three() {
        let o = three();
        golden::check("oscillators-three", 2, None, |cv| o.draw(cv));
    }

    #[test]
    fn oscillators_focus() {
        let o = three();
        golden::check("oscillators-focus", 2, Some("wave2"), |cv| o.draw(cv));
    }

    #[test]
    fn focus_changes() {
        let o = three();
        assert!(golden::focus_changes(240, "wave2", |cv| o.draw(cv)));
        assert!(golden::focus_changes(480, "level1", |cv| o.draw(cv)));
    }

    #[test]
    fn scripts_draw_oscillators() {
        let lua = mlua::Lua::new();
        let t: Table = lua
            .load("return { wave1 = 'Saw', wave2 = 'Square', pw2 = 0.3, octave2 = 1, level2 = 0.5, wave3 = 'bogus', noise = 0.1, focus = 'wave1' }")
            .eval()
            .unwrap();
        let o = Oscillators::from_lua(&t).unwrap();
        assert_eq!(o.sources.len(), 3);
        assert_eq!(o.sources[1].shape, Some(Shape::Pulse(0.3)));
        assert_eq!(o.sources[2].shape, None);
        let look = crate::widget::Look::fallback();
        let mut p = crate::screen::Picture::new([0; 3]);
        crate::widget::draw_lua("oscillators", &mut p, crate::widget::Rect { x: 0, y: 0, w: 240, h: 104 }, &t, &look).unwrap();
        assert!(p.ops.len() > 5);
        let empty: Table = lua.load("return {}").eval().unwrap();
        assert_eq!(Oscillators::from_lua(&empty).unwrap(), Oscillators::default());
    }
}
