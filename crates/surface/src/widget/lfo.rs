//! `lfo`: one cycle of an LFO's shape, with its rate written under it.
//!
//! Inputs (all optional):
//!
//! | input | |
//! |---|---|
//! | `shape` | `"sine"` (default), `"triangle"`, `"saw"` (rising), `"ramp"` (falling), `"square"`, `"random"` (a fixed stepped pattern), `"wander"` (a few sines) |
//! | `rate` | Hz |
//! | `rate_text` | the rate as the plug-in writes it ("1/8", "2.00 Hz"); shown instead of `rate` |
//! | `phase` | 0..1, where the cycle starts (default 0) |
//! | `depth` | 0..1 (default 1): scales the wave around the middle line |
//! | `depth_mod` | 0..1: the modulated depth; the base wave is drawn muted under it |
//! | `position` | 0..1: where in the cycle it is now, a dot on the wave |
//!
//! Touch focus on `rate` writes the rate larger in the text colour and dims the wave;
//! `depth` draws the depth range as two thin lines; `shape` and `phase` keep the wave
//! bright, anything else dims it.

use mlua::Table;

use super::{Align, Canvas, num, text};

#[derive(Clone, Debug, PartialEq)]
pub struct Lfo {
    pub shape: String,
    pub rate: Option<f32>,
    pub rate_text: Option<String>,
    pub phase: f32,
    pub depth: f32,
    pub position: Option<f32>,
    pub depth_mod: Option<f32>,
}

impl Default for Lfo {
    fn default() -> Lfo {
        Lfo { shape: "sine".into(), rate: None, rate_text: None, phase: 0.0, depth: 1.0, position: None, depth_mod: None }
    }
}

/// Steps in the random pattern.
const RANDOM_STEPS: usize = 8;

/// A fixed pseudo-random level (-1..1) for step `i`.
fn random_step(i: usize) -> f32 {
    let mut h = (i as u32).wrapping_add(7).wrapping_mul(0x9E37_79B1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    (h % 2001) as f32 / 1000.0 - 1.0
}

/// The wave `shape` at cycle position `p` (any real; taken mod 1): -1..1.
pub fn wave(shape: &str, p: f32) -> f32 {
    let p = p.rem_euclid(1.0);
    let tau = std::f32::consts::TAU;
    match shape {
        "triangle" => {
            if p < 0.25 {
                4.0 * p
            } else if p < 0.75 {
                2.0 - 4.0 * p
            } else {
                4.0 * p - 4.0
            }
        }
        "saw" => 2.0 * p - 1.0,
        "ramp" => 1.0 - 2.0 * p,
        "square" => {
            if p < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        "random" => random_step((p * RANDOM_STEPS as f32) as usize % RANDOM_STEPS),
        "wander" => ((tau * p).sin() * 0.55 + (tau * 2.0 * p + 1.3).sin() * 0.3 + (tau * 3.0 * p + 2.1).sin() * 0.2) / 1.05,
        _ => (tau * p).sin(),
    }
}

impl Lfo {
    pub fn from_lua(t: &Table) -> mlua::Result<Lfo> {
        Ok(Lfo {
            shape: text(t, "shape").filter(|s| !s.is_empty()).unwrap_or_else(|| "sine".into()),
            rate: num(t, "rate"),
            rate_text: text(t, "rate_text").filter(|s| !s.is_empty()),
            phase: num(t, "phase").unwrap_or(0.0).rem_euclid(1.0),
            depth: num(t, "depth").unwrap_or(1.0).clamp(0.0, 1.0),
            position: num(t, "position").map(|p| p.clamp(0.0, 1.0)),
            depth_mod: num(t, "depth_mod").map(|d| d.clamp(0.0, 1.0)),
        })
    }

    /// The wave's level (-1..1, depth applied) at `x` (0..1 across the drawn cycle).
    pub fn level(&self, x: f32) -> f32 {
        wave(&self.shape, self.phase + x) * self.depth
    }

    fn rate_label(&self) -> Option<String> {
        self.rate_text.clone().or_else(|| self.rate.map(|r| format!("{r:.2} Hz")))
    }

    pub fn draw(&self, c: &mut Canvas) {
        let Some(m) = self.depth_mod else { return self.draw_shown(c, None) };
        Lfo { depth: m, depth_mod: None, ..self.clone() }.draw_shown(c, Some(self));
    }

    /// Draw this wave, with `base` (where modulation moved its depth from) muted under it.
    fn draw_shown(&self, c: &mut Canvas, base: Option<&Lfo>) {
        let look = *c.look;
        let (w, h) = (c.r.w, c.r.h);
        c.fill(0, 0, w, h, look.panel);
        let label = self.rate_label();
        let reserve = if label.is_some() { 20.0 } else { 0.0 };
        let (x0, pw) = (4.0, (w - 8) as f32);
        let (top, bottom) = (6.0, h as f32 - reserve - 6.0);
        let mid = (top + bottom) / 2.0;
        let amp = (bottom - top) / 2.0;
        let px = |x: f32| x0 + x * pw;
        let py = |v: f32| mid - v * amp;
        c.line(x0, mid, x0 + pw, mid, 1.0, look.separator);
        if c.lit("depth") {
            for s in [1.0, -1.0] {
                let y = py(s * self.depth);
                c.line(x0, y, x0 + pw, y, 1.0, look.text);
            }
        }
        let n = pw as usize;
        let pts: Vec<(f32, f32)> = (0..=n)
            .map(|i| {
                let x = i as f32 / n as f32;
                (px(x), py(self.level(x)))
            })
            .collect();
        if let Some(b) = base {
            let under: Vec<(f32, f32)> = (0..=n)
                .map(|i| {
                    let x = i as f32 / n as f32;
                    (px(x), py(b.level(x)))
                })
                .collect();
            c.polyline(&under, 1.5, c.ink(look.muted, &["shape", "phase", "depth"]));
        }
        c.polyline(&pts, 2.0, c.ink(look.accent, &["shape", "phase", "depth"]));
        if let Some(p) = self.position {
            let (x, y) = (px(p), py(self.level(p)));
            c.dot(x.clamp(5.0, w as f32 - 5.0), y, 3.5, c.ink(look.marker, &["position"]));
        }
        if let Some(label) = label {
            let y = h - 18;
            if c.lit("rate") {
                c.text(0, y, w - 8, 16, &label, look.label, look.text, Align::Right);
            } else {
                c.text(0, y, w - 8, 16, &label, look.small, look.weak, Align::Right);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::golden;

    #[test]
    fn sine_starts_in_the_middle_and_peaks_at_a_quarter() {
        let l = Lfo::default();
        assert!(l.level(0.0).abs() < 1e-6);
        assert!((l.level(0.25) - 1.0).abs() < 1e-6);
        assert!((l.level(0.75) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn phase_shifts_the_start() {
        let l = Lfo { phase: 0.25, ..Default::default() };
        assert!((l.level(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn square_is_two_levels() {
        let l = Lfo { shape: "square".into(), ..Default::default() };
        let mut levels: Vec<f32> = (0..100).map(|i| l.level(i as f32 / 100.0)).collect();
        levels.dedup();
        assert_eq!(levels, [1.0, -1.0]);
    }

    #[test]
    fn depth_zero_is_flat() {
        for shape in ["sine", "triangle", "saw", "ramp", "square", "random", "wander"] {
            let l = Lfo { shape: shape.into(), depth: 0.0, ..Default::default() };
            assert!((0..50).all(|i| l.level(i as f32 / 50.0) == 0.0), "{shape}");
        }
    }

    #[test]
    fn random_is_deterministic_and_stepped() {
        let a: Vec<f32> = (0..64).map(|i| wave("random", i as f32 / 64.0)).collect();
        let b: Vec<f32> = (0..64).map(|i| wave("random", i as f32 / 64.0)).collect();
        assert_eq!(a, b);
        let mut d = a.clone();
        d.dedup();
        assert!(d.len() > 3 && d.len() <= RANDOM_STEPS);
        assert!(a.iter().all(|v| (-1.0..=1.0).contains(v)));
    }

    #[test]
    fn wander_stays_in_range() {
        assert!((0..200).all(|i| wave("wander", i as f32 / 200.0).abs() <= 1.0));
    }

    #[test]
    fn lfo_sine() {
        let l = Lfo { rate: Some(2.0), position: Some(0.3), ..Default::default() };
        golden::check("lfo-sine", 2, None, |cv| l.draw(cv));
    }

    #[test]
    fn lfo_square_text() {
        let l = Lfo { shape: "square".into(), rate_text: Some("1/8".into()), depth: 0.6, phase: 0.25, ..Default::default() };
        golden::check("lfo-square", 2, None, |cv| l.draw(cv));
    }

    #[test]
    fn lfo_depth_modulated() {
        let l = Lfo { rate: Some(2.0), depth: 0.3, depth_mod: Some(0.9), ..Default::default() };
        golden::check("lfo-depth-mod", 2, None, |cv| l.draw(cv));
    }

    #[test]
    fn lfo_random_and_wander() {
        let r = Lfo { shape: "random".into(), rate: Some(0.5), ..Default::default() };
        golden::check("lfo-random", 2, None, |cv| r.draw(cv));
        let w = Lfo { shape: "wander".into(), ..Default::default() };
        golden::check("lfo-wander", 2, None, |cv| w.draw(cv));
    }

    #[test]
    fn touching_rate_and_depth() {
        let l = Lfo { shape: "triangle".into(), rate: Some(1.5), depth: 0.7, ..Default::default() };
        golden::check("lfo-triangle-focus-rate", 2, Some("rate"), |cv| l.draw(cv));
        golden::check("lfo-triangle-focus-depth", 2, Some("depth"), |cv| l.draw(cv));
        assert!(golden::focus_changes(240, "rate", |cv| l.draw(cv)));
        assert!(golden::focus_changes(360, "depth", |cv| l.draw(cv)));
    }

    #[test]
    fn scripts_draw_an_lfo() {
        let lua = mlua::Lua::new();
        let t: Table = lua.load("return { shape = 'saw', rate_text = '1/8', depth = 0.5, position = 0.4 }").eval().unwrap();
        let look = crate::widget::Look::fallback();
        let mut p = crate::screen::Picture::new([0; 3]);
        crate::widget::draw_lua("lfo", &mut p, crate::widget::Rect { x: 0, y: 0, w: 240, h: 104 }, &t, &look).unwrap();
        assert!(p.ops.len() > 10);
        let l = Lfo::from_lua(&t).unwrap();
        assert_eq!((l.shape.as_str(), l.rate_text.as_deref(), l.depth), ("saw", Some("1/8"), 0.5));
        let empty: Table = lua.load("return {}").eval().unwrap();
        assert_eq!(Lfo::from_lua(&empty).unwrap(), Lfo::default());
    }
}
