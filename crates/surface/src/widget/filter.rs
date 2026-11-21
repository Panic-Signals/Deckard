//! `filter`: a filter's magnitude response over a log frequency axis (20 Hz to 20 kHz,
//! about -36 to +18 dB, 0 dB marked), worked out here from the 2-pole state-variable
//! transfer functions. A 24 dB slope (or `ladder4`) cascades two of them.
//!
//! Inputs (all optional):
//!
//! | input | |
//! |---|---|
//! | `shape` | `"lp"` (default), `"hp"`, `"bp"`, `"notch"`; aliases `"svf_lp"`, `"svf_hp"`, `"svf_bp"`, `"ladder4"` (a 24 dB low-pass) |
//! | `cutoff` | Hz (1000) |
//! | `resonance` | 0..1 (0), mapped to Q 0.707..10 |
//! | `slope` | dB per octave, 12 or 24 (12; `ladder4` is 24) |
//! | `drive` | dB (0): lifts the whole response a little |
//! | `mix` | 0..1 (1): dry/wet blend of the magnitude |
//! | `cutoff_mod` | Hz: the modulated cutoff; the base curve is drawn muted under it |
//! | `resonance_mod` | 0..1: the modulated resonance, drawn the same way |
//! | `spectrum` | the plug-in's live output spectrum behind the response: points 0..1 (−90 to 0 dBFS) evenly over the same 20 Hz to 20 kHz axis (the pictures over the knobs get it from the view) |
//!
//! Touch focus on `cutoff` draws a guide from the curve to the axis and a handle; on
//! `resonance` it rings the peak and writes the Q.

use mlua::Table;

use super::{Align, Canvas, freq_t, num, nums, text};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Lp,
    Hp,
    Bp,
    Notch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub shape: Shape,
    pub cutoff: f32,
    pub resonance: f32,
    pub slope: f32,
    pub drive: f32,
    pub mix: f32,
    pub cutoff_mod: Option<f32>,
    pub resonance_mod: Option<f32>,
    pub spectrum: Vec<f32>,
}

impl Default for Filter {
    fn default() -> Filter {
        Filter {
            shape: Shape::Lp,
            cutoff: 1000.0,
            resonance: 0.0,
            slope: 12.0,
            drive: 0.0,
            mix: 1.0,
            cutoff_mod: None,
            resonance_mod: None,
            spectrum: Vec::new(),
        }
    }
}

const DB_TOP: f32 = 18.0;
const DB_BOTTOM: f32 = -36.0;

/// Q for a resonance of 0..1: 0.707 (no peak) to 10.
pub fn q_of(resonance: f32) -> f32 {
    0.707 * (10.0f32 / 0.707).powf(resonance.clamp(0.0, 1.0))
}

/// The magnitude (linear) of one 2-pole stage at `f`.
fn stage(shape: Shape, cutoff: f32, q: f32, f: f32) -> f32 {
    let x = f / cutoff.max(1.0);
    let (re, im) = (1.0 - x * x, x / q);
    let den = (re * re + im * im).sqrt().max(1e-6);
    match shape {
        Shape::Lp => 1.0 / den,
        Shape::Hp => x * x / den,
        Shape::Bp => (x / q) / den,
        Shape::Notch => re.abs() / den,
    }
}

/// The response in dB at `f` Hz. `four_pole` cascades two stages (each with half the
/// resonance, so the peak stays about as tall as the 12 dB one's).
pub fn response_db(shape: Shape, cutoff: f32, resonance: f32, four_pole: bool, f: f32) -> f32 {
    let mag = if four_pole {
        let q = 0.707 + (q_of(resonance) - 0.707) * 0.5;
        stage(shape, cutoff, q, f).powi(2)
    } else {
        stage(shape, cutoff, q_of(resonance), f)
    };
    20.0 * mag.max(1e-6).log10()
}

impl Filter {
    pub fn from_lua(t: &Table) -> mlua::Result<Filter> {
        let name = text(t, "shape").unwrap_or_default();
        let shape = match name.as_str() {
            "hp" | "svf_hp" => Shape::Hp,
            "bp" | "svf_bp" => Shape::Bp,
            "notch" => Shape::Notch,
            _ => Shape::Lp,
        };
        let slope = if name == "ladder4" { 24.0 } else { num(t, "slope").unwrap_or(12.0) };
        Ok(Filter {
            shape,
            cutoff: num(t, "cutoff").unwrap_or(1000.0).clamp(20.0, 20000.0),
            resonance: num(t, "resonance").unwrap_or(0.0).clamp(0.0, 1.0),
            slope,
            drive: num(t, "drive").unwrap_or(0.0),
            mix: num(t, "mix").unwrap_or(1.0).clamp(0.0, 1.0),
            cutoff_mod: num(t, "cutoff_mod").map(|m| m.clamp(20.0, 20000.0)),
            resonance_mod: num(t, "resonance_mod").map(|m| m.clamp(0.0, 1.0)),
            spectrum: nums(t, "spectrum"),
        })
    }

    /// The shown response at `f` for cutoff `cutoff`: with drive and mix.
    pub fn db_at(&self, cutoff: f32, f: f32) -> f32 {
        self.db_with(cutoff, self.resonance, f)
    }

    fn db_with(&self, cutoff: f32, resonance: f32, f: f32) -> f32 {
        let wet = 10f32.powf(response_db(self.shape, cutoff, resonance, self.slope >= 18.0, f) / 20.0);
        let mag = (1.0 - self.mix) + self.mix * wet;
        20.0 * mag.max(1e-6).log10() + (self.drive * 0.25).clamp(0.0, 6.0)
    }

    pub fn draw(&self, c: &mut Canvas) {
        let look = *c.look;
        let (w, h) = (c.r.w, c.r.h);
        // (Live: an opaque panel the MK3 can redraw alone, frame after frame.)
        if self.spectrum.len() >= 2 {
            c.spectrum_panel(look.panel);
        } else {
            c.fill(0, 0, w, h, look.panel);
        }
        let px = |t: f32| 1.5 + t * (w as f32 - 4.0);
        let py = |db: f32| (2.5 + (DB_TOP - db) / (DB_TOP - DB_BOTTOM) * (h as f32 - 5.0)).clamp(2.0, h as f32 - 2.5);
        let bottom = (h - 1) as f32;
        for hz in [100.0, 1000.0, 10000.0] {
            let x = px(freq_t(hz));
            c.line(x, 1.0, x, bottom, 1.0, look.grid);
        }
        c.line(1.0, py(0.0), (w - 1) as f32, py(0.0), 1.0, look.separator);

        let all = ["cutoff", "resonance", "shape", "slope", "drive", "mix"];
        let curve = |cutoff: f32, resonance: f32| -> Vec<(f32, f32)> {
            let n = ((w - 4) / 3).clamp(40, 160);
            (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    let f = 20.0 * 1000f32.powf(t);
                    (px(t), py(self.db_with(cutoff, resonance, f)))
                })
                .collect()
        };
        let eff = self.cutoff_mod.unwrap_or(self.cutoff);
        let eff_res = self.resonance_mod.unwrap_or(self.resonance);
        let main = curve(eff, eff_res);
        c.area(&main, bottom, c.ink(look.fill_of(look.accent), &all));
        // The output's spectrum over the fill (which would hide it) and under the curve,
        // on its own level axis: -90 dBFS at the floor, 0 at the top.
        let n = self.spectrum.len();
        if n >= 2 {
            let trace: Vec<(f32, f32)> = self
                .spectrum
                .iter()
                .enumerate()
                .map(|(i, v)| (px(i as f32 / (n - 1) as f32), ((1.0 - v.clamp(0.0, 1.0)) * (h - 1) as f32).clamp(1.5, h as f32 - 2.5)))
                .collect();
            // (A step toward the panel: behind the response, not competing with it.)
            c.polyline(&trace, 1.5, super::mix(look.panel, look.marker, 0.5));
        }
        if self.cutoff_mod.is_some() || self.resonance_mod.is_some() {
            let base = curve(self.cutoff, self.resonance);
            c.polyline(&base, 1.5, c.ink(look.muted, &all));
        }
        c.polyline(&main, 2.0, c.ink(look.accent, &all));

        if w >= 2 * super::CELL_W {
            for (hz, name) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
                let x = px(freq_t(hz)) as i32;
                c.text(x - 16, h - 14, 32, 12, name, look.small, look.weak, Align::Center);
            }
            c.text(3, (py(0.0) as i32 - 12).max(0), 24, 11, "0 dB", look.small, look.weak, Align::Left);
        }

        let y_at = |f: f32| py(self.db_with(eff, eff_res, f));
        if c.lit("cutoff") {
            // (Kept inside the box at the ends of the range.)
            let (x, y) = (px(freq_t(eff)).clamp(6.0, w as f32 - 6.0), y_at(eff).clamp(6.0, h as f32 - 6.0));
            c.line(x, y, x, bottom - 1.0, 1.5, look.record);
            // (No frequency written over it: the knob's own value shows it.)
            c.dot(x, y, 5.0, look.record);
        }
        if c.lit("resonance") {
            // The peak: the tallest point of the curve.
            let mut best = (px(freq_t(eff)), y_at(eff));
            for &(x, y) in &main {
                if y < best.1 {
                    best = (x, y);
                }
            }
            c.ring(best.0.clamp(8.0, w as f32 - 8.0), best.1.clamp(8.0, h as f32 - 8.0), 6.0, 1.5, look.record);
            let label = format!("Q {:.1}", q_of(eff_res));
            let tw = c.text_width(&label, look.small) + 6;
            let tx = if best.0 > w as f32 / 2.0 { 3 } else { w - tw - 3 };
            c.text(tx, 3, tw, 12, &label, look.small, look.text, Align::Center);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::golden;

    #[test]
    fn low_pass_is_minus_3_at_cutoff() {
        let db = response_db(Shape::Lp, 1000.0, 0.0, false, 1000.0);
        assert!((db + 3.0).abs() < 0.1, "{db}");
        assert!(response_db(Shape::Lp, 1000.0, 0.0, false, 50.0).abs() < 0.1);
    }

    #[test]
    fn high_pass_passes_the_highs() {
        assert!(response_db(Shape::Hp, 1000.0, 0.0, false, 20000.0).abs() < 0.1);
        assert!(response_db(Shape::Hp, 1000.0, 0.0, false, 50.0) < -20.0);
    }

    #[test]
    fn steeper_slope_falls_faster() {
        let a = response_db(Shape::Lp, 1000.0, 0.0, false, 8000.0);
        let b = response_db(Shape::Lp, 1000.0, 0.0, true, 8000.0);
        assert!(b < a - 10.0, "{a} {b}");
    }

    #[test]
    fn resonance_peaks_and_notch_dips() {
        assert!(response_db(Shape::Lp, 1000.0, 1.0, false, 1000.0) > 15.0);
        assert!(response_db(Shape::Notch, 1000.0, 0.0, false, 1000.0) < -30.0);
        assert!(response_db(Shape::Bp, 1000.0, 0.5, false, 1000.0).abs() < 0.1);
    }

    fn lp() -> Filter {
        Filter { cutoff: 800.0, resonance: 0.6, ..Default::default() }
    }

    #[test]
    fn filter_low_pass() {
        let f = lp();
        golden::check("filter-lp", 2, None, |cv| f.draw(cv));
    }

    #[test]
    fn filter_band_pass_modulated() {
        let f = Filter { shape: Shape::Bp, cutoff: 2000.0, resonance: 0.4, cutoff_mod: Some(4500.0), ..Default::default() };
        golden::check("filter-bp-mod", 2, None, |cv| f.draw(cv));
    }

    #[test]
    fn filter_resonance_modulated() {
        let f = Filter { cutoff: 1200.0, resonance: 0.2, resonance_mod: Some(0.85), ..Default::default() };
        assert!(f.db_with(1200.0, 0.85, 1200.0) > f.db_with(1200.0, 0.2, 1200.0) + 6.0);
        golden::check("filter-resonance-mod", 2, None, |cv| f.draw(cv));
    }

    #[test]
    fn filter_high_pass_24() {
        let f = Filter { shape: Shape::Hp, cutoff: 300.0, slope: 24.0, drive: 6.0, mix: 0.8, ..Default::default() };
        golden::check("filter-hp24", 2, None, |cv| f.draw(cv));
    }

    #[test]
    fn touching_cutoff_and_resonance_lights_them() {
        let f = lp();
        golden::check("filter-lp-focus-cutoff", 2, Some("cutoff"), |cv| f.draw(cv));
        golden::check("filter-lp-focus-resonance", 2, Some("resonance"), |cv| f.draw(cv));
        assert!(golden::focus_changes(240, "cutoff", |cv| f.draw(cv)));
        assert!(golden::focus_changes(240, "resonance", |cv| f.draw(cv)));
    }

    #[test]
    fn a_live_spectrum_draws_on_a_redrawable_panel() {
        let spectrum: Vec<f32> = (0..96).map(|i| 0.8 - 0.006 * i as f32 + 0.05 * (i as f32 * 0.7).sin()).collect();
        let f = Filter { cutoff: 2500.0, resonance: 0.5, spectrum, ..Default::default() };
        golden::check("filter-lp-spectrum", 2, None, |cv| f.draw(cv));
        let mut p = crate::screen::Picture::new([0; 3]);
        let look = crate::widget::Look::fallback();
        let mut c = crate::widget::Canvas::new(&mut p, crate::widget::Rect { x: 0, y: 40, w: 240, h: 104 }, &look);
        f.draw(&mut c);
        drop(c);
        assert!(p.ops.iter().any(|op| matches!(op, crate::screen::DrawOp::SpectrumPanel { .. })));
        let mut p = crate::screen::Picture::new([0; 3]);
        let mut c = crate::widget::Canvas::new(&mut p, crate::widget::Rect { x: 0, y: 40, w: 240, h: 104 }, &look);
        lp().draw(&mut c);
        drop(c);
        assert!(!p.ops.iter().any(|op| matches!(op, crate::screen::DrawOp::SpectrumPanel { .. })));
    }

    #[test]
    fn scripts_draw_a_filter() {
        let mut p = crate::screen::Picture::new([0; 3]);
        let lua = mlua::Lua::new();
        let t: Table = lua.load("return { shape = 'ladder4', cutoff = 500, resonance = 0.3 }").eval().unwrap();
        let f = Filter::from_lua(&t).unwrap();
        assert_eq!(f.slope, 24.0);
        let look = crate::widget::Look::fallback();
        crate::widget::draw_lua("filter", &mut p, crate::widget::Rect { x: 0, y: 0, w: 240, h: 104 }, &t, &look).unwrap();
        assert!(p.ops.len() > 5);
    }
}
