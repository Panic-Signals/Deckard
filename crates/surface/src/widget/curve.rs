//! `curve`: a curve DECKARD has already worked out, as the built-in effects send it
//! (`ControlView::eq`, `filter`, `picture`): points evenly spaced along x, each 0..1.
//! The reference widget the others follow.
//!
//! Inputs (all optional):
//!
//! | input | |
//! |---|---|
//! | `kind` | `"freq"` (20 Hz to 20 kHz, ±12 dB: the EQ and the Filter; the default), `"law"` (input along the bottom, output up the side), `"time"` (level over time), `"dots"` (a cloud) |
//! | `spectrum` | a `kind` with the same frequency axis and a −90 to 0 dBFS level axis. `side` is input and `points` is output. |
//! | `points` | the curve |
//! | `spectrum_in`, `spectrum_out` | optional live spectra behind the response |
//! | `extent` | how far a modulation sweeps it, faint behind it |
//! | `side`, `side_live` | a side-chain's curve, brighter while it plays |
//! | `bands` | handles: `{ x = 0..1, y = 0..1 }` each; several `freq` bands are numbered |
//! | `lfo` | a modulator's shape (0..1 values), in a box at the bottom right |
//! | `dim` | the curve drawn faint (the effect is bypassed, or shows a preview) |
//! | `preview` | `PREVIEW` in the corner |
//!
//! Touch focus on `"bands"` lights the handles and dims the curve.

use mlua::Table;

use super::{Align, Canvas, flag, nums, text};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve {
    pub spectrum: [Vec<f32>; 2],
    pub kind: String,
    pub points: Vec<f32>,
    pub extent: Vec<f32>,
    pub side: Vec<f32>,
    pub side_live: bool,
    pub bands: Vec<(f32, f32)>,
    pub lfo: Vec<f32>,
    pub dim: bool,
    pub preview: bool,
}

/// The modulator box's size, and the room it takes from the plot.
const LFO_W: i32 = 112;
const LFO_H: i32 = 40;
const LFO_GAP: i32 = 8;

impl Curve {
    pub fn from_lua(t: &Table) -> mlua::Result<Curve> {
        let mut bands = Vec::new();
        if let Ok(Some(list)) = t.raw_get::<Option<Table>>("bands") {
            for b in list.sequence_values::<Table>().flatten() {
                bands.push((super::num(&b, "x").unwrap_or(0.0), super::num(&b, "y").unwrap_or(0.0)));
            }
        }
        Ok(Curve {
            spectrum: [nums(t, "spectrum_in"), nums(t, "spectrum_out")],
            kind: text(t, "kind").filter(|k| !k.is_empty()).unwrap_or_else(|| "freq".into()),
            points: nums(t, "points"),
            extent: nums(t, "extent"),
            side: nums(t, "side"),
            side_live: flag(t, "side_live"),
            bands,
            lfo: nums(t, "lfo"),
            dim: flag(t, "dim"),
            preview: flag(t, "preview"),
        })
    }

    pub fn draw(&self, c: &mut Canvas) {
        let look = *c.look;
        let show_lfo = self.lfo.len() >= 2 && c.r.w >= 3 * super::CELL_W;
        let w = if show_lfo { c.r.w - LFO_W - LFO_GAP } else { c.r.w };
        let h = c.r.h;
        if !show_lfo && (self.kind == "spectrum" || self.spectrum.iter().any(|trace| !trace.is_empty())) {
            c.spectrum_panel(look.panel);
        } else {
            c.fill(0, 0, w, h, look.panel);
        }
        // (Inset a pixel, so a curve's 2-pixel line stays in the box.)
        let px = |t: f32| 1.0 + t * (w - 3) as f32;
        let py = |v: f32| (1.0 - v) * (h - 1) as f32;
        let grid = look.grid;
        let kind = self.kind.as_str();
        match kind {
            "freq" | "" | "spectrum" => {
                for hz in [100.0, 1000.0, 10000.0] {
                    let x = px(super::freq_t(hz));
                    c.line(x, 1.0, x, (h - 1) as f32, 1.0, grid);
                }
                if kind == "spectrum" {
                    for t in [1.0 / 3.0, 2.0 / 3.0] {
                        c.line(1.0, py(t), (w - 1) as f32, py(t), 1.0, grid);
                    }
                } else {
                    for db in [-6.0f32, 0.0, 6.0] {
                        let y = py(db / 24.0 + 0.5);
                        c.line(1.0, y, (w - 1) as f32, y, 1.0, if db == 0.0 { look.separator } else { grid });
                    }
                }
            }
            "law" => {
                c.line(1.0, (h - 2) as f32, (w - 2) as f32, 1.0, 1.0, look.separator);
                for t in [0.25, 0.5, 0.75] {
                    c.line(1.0, py(t), (w - 1) as f32, py(t), 1.0, grid);
                    c.line(px(t), 1.0, px(t), (h - 1) as f32, 1.0, grid);
                }
            }
            _ => {
                c.line(1.0, py(0.5), (w - 1) as f32, py(0.5), 1.0, look.separator);
                for t in [0.25, 0.5, 0.75] {
                    c.line(px(t), 1.0, px(t), (h - 1) as f32, 1.0, grid);
                }
            }
        }
        let stroke = |c: &mut Canvas, pts: &[f32], width: f32, ink| {
            let n = pts.len();
            if n < 2 {
                return;
            }
            let line: Vec<(f32, f32)> =
                pts.iter().enumerate().map(|(i, v)| (px(i as f32 / (n - 1) as f32), py(*v).clamp(1.5, h as f32 - 2.5))).collect();
            c.polyline(&line, width, ink);
        };
        let curve_ink = c.ink(if self.dim { look.muted } else { look.accent }, &["points"]);
        stroke(c, &self.spectrum[0], 1.0, look.weak);
        stroke(c, &self.spectrum[1], 1.5, look.marker);
        stroke(c, &self.extent, 1.0, c.ink(look.muted, &["points"]));
        stroke(c, &self.side, 1.5, c.ink(if self.side_live { look.marker } else { look.weak }, &["side"]));
        stroke(c, &self.points, 2.0, curve_ink);
        let freq = matches!(kind, "freq" | "");
        let radius = if freq { 7.0 } else { 4.0 };
        let handle = c.ink(look.record, &["bands"]);
        for (i, &(bx, by)) in self.bands.iter().enumerate() {
            let (cx, cy) = (px(bx), py(by));
            // (Kept inside the box: a handle at an edge sits against it.)
            let edge = radius + 5.0;
            let cx = cx.clamp(edge, w as f32 - edge);
            let cy = cy.clamp(edge, h as f32 - edge);
            if kind == "time" {
                c.line(cx, py(0.0).min(h as f32 - 2.0), cx, cy, 1.5, handle);
            }
            if c.lit("bands") {
                c.ring(cx, cy, radius + 3.0, 1.5, look.text);
            }
            c.dot(cx, cy, radius, handle);
            if freq && self.bands.len() > 1 {
                c.text(cx as i32 - 7, cy as i32 - 7, 14, 14, &(i + 1).to_string(), look.small, look.on_colour, Align::Center);
            }
        }
        if (freq || kind == "spectrum") && w >= 2 * super::CELL_W {
            for (hz, name) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
                let x = px(super::freq_t(hz)) as i32;
                c.text(x - 16, h - 14, 32, 12, name, look.small, look.weak, Align::Center);
            }
        }
        if self.preview {
            c.text(8, 4, 80, 14, "PREVIEW", look.small, look.weak, Align::Left);
        }
        if show_lfo {
            let (bx, by) = (c.r.w - LFO_W, h - LFO_H);
            c.fill(bx, by, LFO_W, LFO_H, look.background);
            c.frame(bx, by, LFO_W, LFO_H, look.grid);
            let m = self.lfo.len();
            let pts: Vec<(f32, f32)> = self
                .lfo
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    (bx as f32 + 4.0 + i as f32 / (m - 1) as f32 * (LFO_W - 8) as f32, by as f32 + 4.0 + (1.0 - v) * (LFO_H - 8) as f32)
                })
                .collect();
            c.polyline(&pts, 1.5, c.ink(look.accent, &["lfo"]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::golden;

    fn eq() -> Curve {
        let points = (0..120)
            .map(|i| {
                let t = i as f32 / 119.0;
                0.5 + 0.18 * (-((t - 0.3) / 0.08).powi(2)).exp() - 0.12 * (-((t - 0.7) / 0.1).powi(2)).exp()
            })
            .collect();
        Curve { kind: "freq".into(), points, bands: vec![(0.15, 0.5), (0.3, 0.68), (0.7, 0.38), (0.9, 0.5)], ..Default::default() }
    }

    #[test]
    fn curve_eq() {
        let c = eq();
        golden::check("curve-eq", 2, None, |cv| c.draw(cv));
    }

    #[test]
    fn curve_filter_with_lfo() {
        let points = (0..120).map(|i| if i < 70 { 0.5 } else { (0.5 - (i - 70) as f32 * 0.012).max(0.0) }).collect();
        let lfo = (0..48).map(|i| 0.5 + 0.45 * (i as f32 / 47.0 * std::f32::consts::TAU).sin()).collect();
        let c = Curve { kind: "freq".into(), points, bands: vec![(0.58, 0.5)], lfo, ..Default::default() };
        golden::check("curve-filter-lfo", 2, None, |cv| c.draw(cv));
    }

    #[test]
    fn curve_law() {
        let points = (0..64).map(|i| (i as f32 / 63.0).min(0.6 + (i as f32 / 63.0 - 0.6) * 0.25)).collect();
        let c = Curve { kind: "law".into(), points, bands: vec![(0.6, 0.6)], ..Default::default() };
        golden::check("curve-law", 2, None, |cv| c.draw(cv));
    }

    #[test]
    fn touching_the_bands_lights_them() {
        let c = eq();
        golden::check("curve-eq-focus-bands", 2, Some("bands"), |cv| c.draw(cv));
        assert!(golden::focus_changes(240, "bands", |cv| c.draw(cv)));
    }

    #[test]
    fn scripts_draw_a_curve() {
        let s = crate::Surface::new();
        let mut p = crate::screen::Picture::new([0; 3]);
        let lua = mlua::Lua::new();
        let t: Table = lua.load("return { kind = 'law', points = { 0, 0.5, 1 }, bands = { { x = 0.5, y = 0.5 } } }").eval().unwrap();
        let look = crate::widget::Look::fallback();
        crate::widget::draw_lua("curve", &mut p, crate::widget::Rect { x: 0, y: 0, w: 240, h: 104 }, &t, &look).unwrap();
        assert!(p.ops.len() > 5);
        assert!(crate::widget::draw_lua("nothing", &mut p, crate::widget::Rect { x: 0, y: 0, w: 240, h: 104 }, &t, &look).is_err());
        drop(s);
    }
}
