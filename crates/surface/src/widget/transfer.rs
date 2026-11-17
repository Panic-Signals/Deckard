//! `transfer`: a compressor's, limiter's or gate's static transfer curve: input level
//! along the bottom, output level up the side (both -60 to 0 dB), with the 1:1 diagonal
//! for reference. A box 240 wide or more also writes the numbers beside the plot.
//!
//! Inputs (all optional):
//!
//! | input | |
//! |---|---|
//! | `threshold` | dB where compression starts (default -18) |
//! | `ratio` | `n:1` above the threshold (default 4, at least 1; 20 or more reads "LIMIT") |
//! | `knee` | soft knee width in dB (default 0: a corner) |
//! | `makeup` | gain added after, in dB (default 0) |
//! | `level` | the current input level in dB: a dot on the curve there |
//! | `reduction` | the gain being taken off now, in dB (0 or more): a bar at the right edge |
//!
//! Touch focus: `threshold` brightens its lines and number, `ratio` the part above the
//! threshold, `knee` marks the knee, `makeup` lights its number.

use mlua::Table;

use super::{Align, Canvas, db_text, num};

#[derive(Clone, Debug, PartialEq)]
pub struct Transfer {
    pub threshold: f32,
    pub ratio: f32,
    pub knee: f32,
    pub makeup: f32,
    pub level: Option<f32>,
    pub reduction: Option<f32>,
}

impl Default for Transfer {
    fn default() -> Self {
        Transfer { threshold: -18.0, ratio: 4.0, knee: 0.0, makeup: 0.0, level: None, reduction: None }
    }
}

/// The plot's side, and its margins in the box.
const SIDE: i32 = 96;
const PX0: i32 = 3;
const PY0: i32 = 4;
const DB_MIN: f32 = -60.0;

impl Transfer {
    pub fn from_lua(t: &Table) -> mlua::Result<Transfer> {
        let d = Transfer::default();
        Ok(Transfer {
            threshold: num(t, "threshold").unwrap_or(d.threshold),
            ratio: num(t, "ratio").unwrap_or(d.ratio),
            knee: num(t, "knee").unwrap_or(d.knee),
            makeup: num(t, "makeup").unwrap_or(d.makeup),
            level: num(t, "level"),
            reduction: num(t, "reduction"),
        })
    }

    /// Output level in dB for input level `inp` (makeup included).
    pub fn out(&self, inp: f32) -> f32 {
        let ratio = self.ratio.max(1.0);
        let knee = self.knee.max(0.0);
        let d = inp - self.threshold;
        let o = if 2.0 * d < -knee {
            inp
        } else if knee > 0.0 && 2.0 * d.abs() <= knee {
            let k = d + knee / 2.0;
            inp + (1.0 / ratio - 1.0) * k * k / (2.0 * knee)
        } else {
            self.threshold + d / ratio
        };
        o + self.makeup
    }

    pub fn draw(&self, c: &mut Canvas) {
        let look = *c.look;
        let side = SIDE as f32;
        let (x0, y0) = (PX0 as f32, PY0 as f32);
        let px = |db: f32| x0 + (db - DB_MIN) / -DB_MIN * side;
        let py = |db: f32| y0 + side - (db - DB_MIN) / -DB_MIN * side;
        let cx = |db: f32| px(db).clamp(x0 + 1.5, x0 + side - 1.5);
        let cy = |db: f32| py(db).clamp(y0 + 1.5, y0 + side - 1.5);
        c.fill(PX0, PY0, SIDE, SIDE, look.panel);
        for db in [-48.0, -36.0, -24.0, -12.0] {
            c.line(px(db), y0 + 1.0, px(db), y0 + side - 1.0, 1.0, look.grid);
            c.line(x0 + 1.0, py(db), x0 + side - 1.0, py(db), 1.0, look.grid);
        }
        c.line(x0 + 1.0, y0 + side - 1.0, x0 + side - 1.0, y0 + 1.0, 1.0, look.separator);

        let thr = self.threshold.clamp(DB_MIN, 0.0);
        let knee = self.knee.max(0.0);
        let lo = (thr - knee / 2.0).max(DB_MIN);
        let hi = (thr + knee / 2.0).min(0.0);
        // The curve in three runs: under the knee, the knee, above it.
        let run = |a: f32, b: f32| -> Vec<(f32, f32)> {
            let n = (((b - a) / 1.25).ceil() as i32).max(1);
            (0..=n)
                .map(|i| {
                    let x = a + (b - a) * i as f32 / n as f32;
                    (cx(x), cy(self.out(x)))
                })
                .collect()
        };
        let below = c.ink(look.accent, &["threshold", "knee", "makeup"]);
        let above = c.ink(look.accent, &["threshold", "ratio", "knee", "makeup"]);
        c.polyline(&run(DB_MIN, lo), 2.0, below);
        c.polyline(&run(lo, hi), 2.0, c.ink(look.accent, &["threshold", "knee", "makeup"]));
        c.polyline(&run(hi, 0.0), 2.0, above);
        if c.lit("knee") && knee > 0.0 {
            c.polyline(&run(lo, hi), 2.0, look.text);
            for db in [lo, hi] {
                c.line(cx(db), y0 + side - 1.0, cx(db), y0 + side - 7.0, 1.0, look.text);
            }
        }

        // The threshold, on both axes.
        let tx = cx(thr);
        let ty = cy(self.out(thr.max(lo)).min(0.0));
        let t_ink = c.ink(look.record, &["threshold"]);
        if c.lit("threshold") {
            c.line(tx, y0 + side - 1.0, tx, ty, 1.0, look.record);
            c.line(x0 + 1.0, ty, tx, ty, 1.0, look.record);
            c.line(tx, y0 + side - 1.5, tx, y0 + side - 9.0, 2.0, look.text);
            c.line(x0 + 1.5, ty, x0 + 9.0, ty, 2.0, look.text);
        } else {
            c.line(tx, y0 + side - 1.0, tx, y0 + side - 7.0, 1.5, t_ink);
            c.line(x0 + 1.0, ty, x0 + 7.0, ty, 1.5, t_ink);
        }

        if let Some(level) = self.level {
            let l = level.clamp(DB_MIN, 0.0);
            c.dot(cx(l), cy(self.out(l)), 3.0, look.marker);
        }

        let w = c.r.w;
        if let Some(r) = self.reduction {
            let h = (r.max(0.0) / -DB_MIN * side).round().clamp(0.0, side - 2.0) as i32;
            if h > 0 {
                c.fill(w - 8, PY0 + 1, 5, h, look.record);
            }
        }

        if w >= 2 * super::CELL_W {
            let x = PX0 + SIDE + 14;
            let room = w - x - 14;
            let weak_or = |c: &Canvas, name: &str| {
                if c.focus.is_none() || c.lit(name) { look.text } else { look.weak }
            };
            let ratio = self.ratio.max(1.0);
            let ratio_text = if ratio >= 20.0 { "LIMIT".to_string() } else { format!("{ratio:.1}:1") };
            let col = weak_or(c, "threshold");
            c.text(x, 2, room, 12, "threshold", look.label, look.weak, Align::Left);
            c.text(x, 14, room, 22, &db_text(self.threshold), look.big, col, Align::Left);
            let col = weak_or(c, "ratio");
            c.text(x, 38, room, 12, "ratio", look.label, look.weak, Align::Left);
            c.text(x, 50, room, 16, &ratio_text, look.label, col, Align::Left);
            if self.makeup.abs() >= 0.05 || c.lit("makeup") {
                let col = weak_or(c, "makeup");
                c.text(x, 70, room, 12, "makeup", look.label, look.weak, Align::Left);
                c.text(x, 82, room, 16, &db_text(self.makeup), look.label, col, Align::Left);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::golden;

    fn comp() -> Transfer {
        Transfer { threshold: -18.0, ratio: 4.0, knee: 6.0, makeup: 3.0, level: Some(-10.0), reduction: Some(8.0), ..Default::default() }
    }

    #[test]
    fn below_threshold_is_unity_plus_makeup() {
        let t = Transfer { makeup: 2.0, knee: 6.0, ..Default::default() };
        assert!((t.out(-40.0) - -38.0).abs() < 1e-4);
        assert!((t.out(-30.0) - -28.0).abs() < 1e-4);
    }

    #[test]
    fn far_above_threshold_the_slope_is_one_over_ratio() {
        let t = Transfer { ratio: 8.0, knee: 6.0, ..Default::default() };
        let s = t.out(-2.0) - t.out(-4.0);
        assert!((s - 2.0 / 8.0).abs() < 1e-4, "{s}");
    }

    #[test]
    fn a_hard_knee_has_a_corner() {
        let t = Transfer::default();
        assert!((t.out(-18.0) - -18.0).abs() < 1e-4);
        let (a, b) = (t.out(-18.0) - t.out(-19.0), t.out(-17.0) - t.out(-18.0));
        assert!((a - 1.0).abs() < 1e-4 && (b - 0.25).abs() < 1e-4);
    }

    #[test]
    fn a_soft_knee_is_continuous() {
        let t = Transfer { knee: 10.0, ..Default::default() };
        assert!((t.out(-23.0) - -23.0).abs() < 1e-3);
        assert!((t.out(-13.0) - (-18.0 + 5.0 / 4.0)).abs() < 1e-3);
    }

    #[test]
    fn transfer_compressor() {
        let t = comp();
        golden::check("transfer-compressor", 2, None, |cv| t.draw(cv));
    }

    #[test]
    fn transfer_limiter() {
        let t = Transfer { threshold: -6.0, ratio: 20.0, ..Default::default() };
        golden::check("transfer-limiter", 2, None, |cv| t.draw(cv));
    }

    #[test]
    fn touching_the_threshold_lights_it() {
        let t = comp();
        golden::check("transfer-focus-threshold", 2, Some("threshold"), |cv| t.draw(cv));
        assert!(golden::focus_changes(240, "threshold", |cv| t.draw(cv)));
        assert!(golden::focus_changes(240, "ratio", |cv| t.draw(cv)));
    }

    #[test]
    fn scripts_draw_a_transfer() {
        let mut p = crate::screen::Picture::new([0; 3]);
        let lua = mlua::Lua::new();
        let t: Table = lua.load("return { threshold = -20, ratio = 3, level = -12 }").eval().unwrap();
        let look = crate::widget::Look::fallback();
        crate::widget::draw_lua("transfer", &mut p, crate::widget::Rect { x: 0, y: 0, w: 240, h: 104 }, &t, &look).unwrap();
        assert!(p.ops.len() > 5);
    }
}
