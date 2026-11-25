//! `dial`: one knob's value as a ring, in the style of a hardware encoder's collar. It
//! is drawn where a knob has no picture over it; the knob's own cell shows the name and
//! the value, so the dial has no text.
//!
//! The ring is open at the bottom: its track sweeps 270 degrees clockwise from the lower
//! left, through the top, to the lower right. It's at most 20 px in radius, and sits at
//! the bottom of its box, close to the knob's name and value below.
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

/// An arc of radius `r` from angle `a0` to `a1` (degrees), one smooth stroke.
fn arc(c: &mut Canvas, (cx, cy): (f32, f32), r: f32, a0: f32, a1: f32, width: f32, ink: super::Paint) {
    c.arc(cx, cy, r, a0, a1, width, ink);
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
        let r = (c.r.w.min(c.r.h) as f32 / 2.0 - 7.0).clamp(8.0, 20.0);
        // (At the bottom of the box, near the knob's name and value below it: the ring's
        // lowest point, r·sin 45° under its centre, 8 px above the box's foot.)
        let centre = (c.w() / 2.0, c.h() - 8.0 - r * std::f32::consts::FRAC_1_SQRT_2);
        let value = self.value.clamp(0.0, 1.0);
        let from: f32 = if self.bipolar { 0.5 } else { 0.0 };
        let track = c.ink(if focused { look.separator } else { look.grid }, &["value"]);
        let accent = c.ink(look.accent, &["value"]);
        let glow = focused && super::style().focus_glow;
        let arc_w = if focused && !glow { 5.0 } else { 3.0 };
        // (A glow: a faint haze hugging the lit arc, 3 px each side, in its colour faded
        // into the field and falling off smoothly (a ring a pixel apart, none of them a
        // visible band); opaque so the segments' ends don't stack up. Drawn before the
        // track: its rings have round ends, which drawn over the track would cut a gap in
        // it just past the value.)
        let halo = |c: &mut Canvas, a0: f32, a1: f32| {
            if glow {
                for i in (1..=6).rev() {
                    let k = 0.2 * (1.0 - i as f32 / 7.0).powi(2);
                    arc(c, centre, r, a0, a1, arc_w + i as f32, super::mix(look.background, look.accent, k));
                }
            }
        };
        let (lo, hi) = (from.min(value), from.max(value));
        match self.steps {
            Some(n) => {
                let seg = SWEEP / n as f32;
                let segment = |i: usize| (START - seg * i as f32 - GAP / 2.0, START - seg * (i + 1) as f32 + GAP / 2.0);
                // A segment is lit when its middle lies in the arc.
                let on = |i: usize| {
                    let mid = (i as f32 + 0.5) / n as f32;
                    mid >= lo && mid <= hi && (lo < hi)
                };
                for i in (0..n).filter(|&i| on(i)) {
                    let (s0, s1) = segment(i);
                    halo(c, s0, s1);
                }
                for i in 0..n {
                    let (s0, s1) = segment(i);
                    arc(c, centre, r, s0, s1, if on(i) { arc_w } else { 3.0 }, if on(i) { accent } else { track });
                }
            }
            None => {
                if hi > lo {
                    halo(c, angle(lo), angle(hi));
                }
                arc(c, centre, r, angle(0.0), angle(1.0), 3.0, track);
                if hi > lo {
                    arc(c, centre, r, angle(lo), angle(hi), arc_w, accent);
                }
            }
        }
        if let Some(m) = self.value_mod {
            let m = m.clamp(0.0, 1.0);
            let fill = c.ink(super::mix(look.background, look.accent, 0.45), &["value", "value_mod"]);
            if (m - value).abs() > 1e-4 {
                arc(c, centre, r, angle(value), angle(m), 1.5, fill);
            }
            let (x, y) = point(centre.0, centre.1, r, angle(m));
            c.dot(x, y, 2.5, c.ink(look.marker, &["value", "value_mod"]));
        }
        let a = angle(value);
        let (x0, y0) = point(centre.0, centre.1, r - 7.0, a);
        let (x1, y1) = point(centre.0, centre.1, r + 1.5, a);
        c.line(x0, y0, x1, y1, 1.5, c.ink(look.text, &["value"]));
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
        drop(cv);
        let n = p.ops.len();
        let mut p2 = crate::screen::Picture::new([0; 3]);
        let mut cv = Canvas::new(&mut p2, r, &look);
        Dial { value: 0.45, bipolar: true, ..Default::default() }.draw(&mut cv);
        drop(cv);
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

    /// A dial plain and touched in a theme (`DECKARD_THEME`, default deep), side by side,
    /// into `DECKARD_RENDER_DIR` (to look at, not a check).
    #[test]
    #[ignore = "renders a picture to look at"]
    fn render_dial_focus() {
        let Ok(out) = std::env::var("DECKARD_RENDER_DIR") else { return };
        let theme = std::env::var("DECKARD_THEME").unwrap_or_else(|_| "deep".into());
        let look = golden::look(&theme);
        crate::widget::set_theme_style(crate::widget::PictureStyle::named(&theme));
        let d = Dial { value: 0.6, ..Default::default() };
        let (w, h) = (120usize, crate::widget::BAND_H as usize);
        let plain = golden::render(&look, w as i32, None, |cv| d.draw(cv));
        let lit = golden::render(&look, w as i32, Some("value"), |cv| d.draw(cv));
        let mut px = Vec::with_capacity(w * 2 * h);
        for y in 0..h {
            px.extend_from_slice(&plain[y * w..(y + 1) * w]);
            px.extend_from_slice(&lit[y * w..(y + 1) * w]);
        }
        let file = std::path::PathBuf::from(out).join("dial-focus.png");
        golden::write_png(&file, (w * 2) as u32, h as u32, &px);
    }

    /// Turning a knob lights its dial, with a glow in the deep theme; the track just past
    /// the value stays whole (the glow's round ends once cut a gap in it there).
    #[test]
    fn a_glowing_dial_leaves_the_track_whole() {
        let look = golden::look("deep");
        crate::widget::set_theme_style(crate::widget::PictureStyle::named("deep"));
        assert!(crate::widget::style().focus_glow);
        let (w, h) = (120i32, crate::widget::BAND_H);
        let r = (w.min(h) as f32 / 2.0 - 7.0).clamp(8.0, 20.0);
        let (cx, cy) = (w as f32 / 2.0, h as f32 - 8.0 - r * std::f32::consts::FRAC_1_SQRT_2);
        for (value, steps) in [(0.4, None), (0.7, None), (0.5, Some(8))] {
            let d = Dial { value, steps, ..Default::default() };
            let px = golden::render(&look, w, Some("value"), |cv| d.draw(cv));
            let at = |v: f32| {
                let (x, y) = point(cx, cy, r, angle(v));
                px[y as usize * w as usize + x as usize]
            };
            let far = at(steps.map_or(value + 0.25, |n| (n as f32 - 0.5) / n as f32));
            let mut past: Vec<f32> = (4..=10).map(|i| value + i as f32 / 100.0).collect();
            if let Some(n) = steps {
                // (The middle of each unlit segment past the value: the gaps between are dark.)
                past = (0..n).map(|i| (i as f32 + 0.5) / n as f32).filter(|m| *m > value).collect();
            }
            for v in past {
                let p = at(v);
                let diff = (0..3).map(|k| (p[k] as i32 - far[k] as i32).abs()).max().unwrap();
                assert!(diff <= 24, "value {value} steps {steps:?}: the track at {v} is {p:?}, further on {far:?}");
            }
        }
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
        assert!(p.ops.len() >= 5, "five segments, the modulation and the pointer");
        let d = Dial::from_lua(&t).unwrap();
        assert_eq!(d.steps, Some(5));
        assert!(d.bipolar);
    }
}
