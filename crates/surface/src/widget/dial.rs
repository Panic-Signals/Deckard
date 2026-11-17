//! `dial`: one knob's value as a ring, in the style of a hardware encoder's collar. It
//! is drawn where a knob has no picture over it; the knob's own cell shows the name and
//! the value, so the dial has no text.
//!
//! The ring is open at the bottom: its track sweeps 270 degrees clockwise from the lower
//! left, through the top, to the lower right.
//!
//! Inputs (all optional):
//!
//! | input | |
//! |---|---|
//! | `value` | 0..1: the arc from the start of the track to the value (default 0) |
//! | `bipolar` | the arc grows from the top centre instead of the start |
//! | `value_mod` | 0..1: where modulation has the knob, a dot on the ring and a thin arc from the value |
//! | `steps` | a stepped knob: the arc is that many short segments with gaps (2 to 16) |
//!
//! Touch focus on `value` lights the arc (thicker) and the track; the rest dims otherwise.

use mlua::Table;

use super::{Canvas, flag, num};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dial {
    pub value: f32,
    pub bipolar: bool,
    pub value_mod: Option<f32>,
    pub steps: Option<usize>,
}

/// Where the track starts and how far it sweeps clockwise, in degrees (counter-clockwise
/// from 3 o'clock, as in maths).
const START: f32 = 225.0;
const SWEEP: f32 = 270.0;
/// Degrees per polyline segment.
const SEG: f32 = 3.0;
/// The gap between a stepped knob's segments, in degrees.
const GAP: f32 = 6.0;

/// The angle of `v` (0..1) on the track: 0 is 225 degrees, 1 is -45, 0.5 the top (90).
pub fn angle(v: f32) -> f32 {
    START - SWEEP * v.clamp(0.0, 1.0)
}

fn point(cx: f32, cy: f32, r: f32, deg: f32) -> (f32, f32) {
    let a = deg.to_radians();
    (cx + r * a.cos(), cy - r * a.sin())
}

/// An arc of radius `r` from angle `a0` to `a1` (degrees), as short line segments.
fn arc(c: &mut Canvas, (cx, cy): (f32, f32), r: f32, a0: f32, a1: f32, width: f32, ink: super::Paint) {
    let n = ((a1 - a0).abs() / SEG).ceil().max(1.0) as usize;
    let pts: Vec<(f32, f32)> = (0..=n).map(|i| point(cx, cy, r, a0 + (a1 - a0) * i as f32 / n as f32)).collect();
    c.polyline(&pts, width, ink);
}

impl Dial {
    pub fn from_lua(t: &Table) -> mlua::Result<Dial> {
        Ok(Dial {
            value: num(t, "value").unwrap_or(0.0).clamp(0.0, 1.0),
            bipolar: flag(t, "bipolar"),
            value_mod: num(t, "value_mod").map(|v| v.clamp(0.0, 1.0)),
            steps: num(t, "steps").filter(|n| *n >= 2.0).map(|n| (n as usize).min(16)),
        })
    }

    pub fn draw(&self, c: &mut Canvas) {
        let look = *c.look;
        let focused = c.lit("value");
        let centre = (c.w() / 2.0, c.h() / 2.0);
        let r = (c.r.w.min(c.r.h) as f32 / 2.0 - 12.0).max(8.0);
        let value = self.value.clamp(0.0, 1.0);
        let from: f32 = if self.bipolar { 0.5 } else { 0.0 };
        let track = c.ink(if focused { look.separator } else { look.grid }, &["value"]);
        let accent = c.ink(look.accent, &["value"]);
        let arc_w = if focused { 6.0 } else { 4.0 };
        let (lo, hi) = (from.min(value), from.max(value));
        match self.steps {
            Some(n) => {
                let seg = SWEEP / n as f32;
                for i in 0..n {
                    let (s0, s1) = (START - seg * i as f32 - GAP / 2.0, START - seg * (i + 1) as f32 + GAP / 2.0);
                    // A segment is lit when its middle lies in the arc.
                    let mid = (i as f32 + 0.5) / n as f32;
                    let on = mid >= lo && mid <= hi && (lo < hi);
                    arc(c, centre, r, s0, s1, if on { arc_w } else { 4.0 }, if on { accent } else { track });
                }
            }
            None => {
                arc(c, centre, r, angle(0.0), angle(1.0), 4.0, track);
                if hi > lo {
                    arc(c, centre, r, angle(lo), angle(hi), arc_w, accent);
                }
            }
        }
        if let Some(m) = self.value_mod {
            let m = m.clamp(0.0, 1.0);
            let fill = c.ink(look.fill_of(look.accent), &["value", "value_mod"]);
            if (m - value).abs() > 1e-4 {
                arc(c, centre, r, angle(value), angle(m), 2.0, fill);
            }
            let (x, y) = point(centre.0, centre.1, r, angle(m));
            c.dot(x, y, 3.0, c.ink(look.marker, &["value", "value_mod"]));
        }
        let a = angle(value);
        let (x0, y0) = point(centre.0, centre.1, r - 10.0, a);
        let (x1, y1) = point(centre.0, centre.1, r + 2.0, a);
        c.line(x0, y0, x1, y1, 2.0, c.ink(look.text, &["value"]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::golden;

    #[test]
    fn angles() {
        assert_eq!(angle(0.0), 225.0);
        assert_eq!(angle(1.0), -45.0);
        assert_eq!(angle(0.5), 90.0);
        assert_eq!(angle(2.0), -45.0);
    }

    #[test]
    fn bipolar_middle_is_empty() {
        let d = Dial { value: 0.5, bipolar: true, ..Default::default() };
        let mut p = crate::screen::Picture::new([0; 3]);
        let look = crate::widget::Look::fallback();
        let r = crate::widget::Rect { x: 0, y: 0, w: 120, h: 104 };
        let mut cv = Canvas::new(&mut p, r, &look);
        d.draw(&mut cv);
        let n = p.ops.len();
        let mut p2 = crate::screen::Picture::new([0; 3]);
        let mut cv = Canvas::new(&mut p2, r, &look);
        Dial { value: 0.45, bipolar: true, ..Default::default() }.draw(&mut cv);
        assert!(p2.ops.len() > n);
    }

    #[test]
    fn dial_mid() {
        let d = Dial { value: 0.65, ..Default::default() };
        golden::check("dial-mid", 1, None, |cv| d.draw(cv));
    }

    #[test]
    fn dial_bipolar() {
        let d = Dial { value: 0.3, bipolar: true, value_mod: Some(0.55), ..Default::default() };
        golden::check("dial-bipolar", 1, None, |cv| d.draw(cv));
    }

    #[test]
    fn dial_steps() {
        let d = Dial { value: 0.75, steps: Some(4), ..Default::default() };
        golden::check("dial-steps", 1, None, |cv| d.draw(cv));
    }

    #[test]
    fn dial_focus() {
        let d = Dial { value: 0.4, ..Default::default() };
        golden::check("dial-focus", 1, Some("value"), |cv| d.draw(cv));
    }

    #[test]
    fn focus_changes() {
        let d = Dial { value: 0.4, ..Default::default() };
        assert!(golden::focus_changes(120, "value", |cv| d.draw(cv)));
    }

    #[test]
    fn scripts_draw_a_dial() {
        let mut p = crate::screen::Picture::new([0; 3]);
        let lua = mlua::Lua::new();
        let t: Table = lua.load("return { value = 0.3, bipolar = true, value_mod = 0.6, steps = 5 }").eval().unwrap();
        let look = crate::widget::Look::fallback();
        crate::widget::draw_lua("dial", &mut p, crate::widget::Rect { x: 0, y: 0, w: 120, h: 104 }, &t, &look).unwrap();
        assert!(p.ops.len() > 10);
        let d = Dial::from_lua(&t).unwrap();
        assert_eq!(d.steps, Some(5));
        assert!(d.bipolar);
    }
}
