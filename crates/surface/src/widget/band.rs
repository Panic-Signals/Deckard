//! `band`: one EQ band close up: its own curve (RBJ cookbook biquads, ±18 dB) on a small
//! plot, with its frequency, gain and Q written large beside it.
//!
//! Inputs (all optional):
//!
//! | input | |
//! |---|---|
//! | `freq` | Hz (1000) |
//! | `gain` | dB (0) |
//! | `q` | 0.707 |
//! | `type` | `"bell"` (default), `"low_shelf"`, `"high_shelf"`, `"low_cut"`, `"high_cut"` |
//! | `freq_mod`, `gain_mod` | the modulated frequency (Hz) and gain (dB): drawn in the accent over the base curve, muted |
//! | `on` | false draws everything muted (true) |
//!
//! Touch focus on `freq` draws a guide line at the frequency, on `gain` a line at the
//! gain, on `q` the bandwidth bracket; each lights its number too.

use mlua::Table;

use super::{Align, Canvas, db_text, freq_t, hz_text, num, text};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Bell,
    LowShelf,
    HighShelf,
    LowCut,
    HighCut,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
    pub kind: Kind,
    pub on: bool,
    pub freq_mod: Option<f32>,
    pub gain_mod: Option<f32>,
}

impl Default for Band {
    fn default() -> Band {
        Band { freq: 1000.0, gain: 0.0, q: 0.707, kind: Kind::Bell, on: true, freq_mod: None, gain_mod: None }
    }
}

const RANGE: f32 = 18.0;
const FS: f32 = 48000.0;

/// The band's magnitude in dB at `f` Hz (RBJ cookbook biquad at 48 kHz).
pub fn magnitude_db(kind: Kind, freq: f32, gain: f32, q: f32, f: f32) -> f32 {
    let w0 = std::f32::consts::TAU * freq.clamp(10.0, 22000.0) / FS;
    let (sin, cos) = w0.sin_cos();
    let q = q.max(0.05);
    let alpha = sin / (2.0 * q);
    let a = 10f32.powf(gain / 40.0);
    let sa = 2.0 * a.sqrt() * alpha;
    let (b0, b1, b2, a0, a1, a2) = match kind {
        Kind::Bell => (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a),
        Kind::LowShelf => (
            a * ((a + 1.0) - (a - 1.0) * cos + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - sa),
            (a + 1.0) + (a - 1.0) * cos + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - sa,
        ),
        Kind::HighShelf => (
            a * ((a + 1.0) + (a - 1.0) * cos + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - sa),
            (a + 1.0) - (a - 1.0) * cos + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - sa,
        ),
        Kind::LowCut => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        Kind::HighCut => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
    };
    let w = std::f32::consts::TAU * f.min(FS / 2.0 - 1.0) / FS;
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let norm = |x0: f32, x1: f32, x2: f32| {
        let re = x0 + x1 * c1 + x2 * c2;
        let im = -(x1 * s1 + x2 * s2);
        re * re + im * im
    };
    10.0 * (norm(b0, b1, b2) / norm(a0, a1, a2).max(1e-12)).max(1e-12).log10()
}

impl Band {
    pub fn from_lua(t: &Table) -> mlua::Result<Band> {
        let kind = match text(t, "type").unwrap_or_default().as_str() {
            "low_shelf" => Kind::LowShelf,
            "high_shelf" => Kind::HighShelf,
            "low_cut" => Kind::LowCut,
            "high_cut" => Kind::HighCut,
            _ => Kind::Bell,
        };
        let on = t.raw_get::<Option<bool>>("on").ok().flatten().unwrap_or(true);
        Ok(Band {
            freq: num(t, "freq").unwrap_or(1000.0).clamp(20.0, 20000.0),
            gain: num(t, "gain").unwrap_or(0.0).clamp(-RANGE, RANGE),
            q: num(t, "q").unwrap_or(0.707).clamp(0.1, 18.0),
            kind,
            on,
            freq_mod: num(t, "freq_mod").map(|m| m.clamp(20.0, 20000.0)),
            gain_mod: num(t, "gain_mod").map(|m| m.clamp(-RANGE, RANGE)),
        })
    }

    fn db_at(&self, f: f32) -> f32 {
        magnitude_db(self.kind, self.freq, self.gain, self.q, f)
    }

    pub fn draw(&self, c: &mut Canvas) {
        if self.freq_mod.is_none() && self.gain_mod.is_none() {
            return self.draw_shown(c, None);
        }
        let shown = Band { freq: self.freq_mod.unwrap_or(self.freq), gain: self.gain_mod.unwrap_or(self.gain), ..self.clone() };
        shown.draw_shown(c, Some(self));
    }

    /// Draw this band, with `base` (where modulation moved it from) as a muted curve.
    fn draw_shown(&self, c: &mut Canvas, base: Option<&Band>) {
        let look = *c.look;
        let (w, h) = (c.r.w, c.r.h);
        let num_w = if w <= 240 { 112 } else { 132 };
        let pw = w - num_w - 4;
        c.fill(0, 0, pw, h, look.panel);
        let px = |t: f32| 1.5 + t * (pw as f32 - 4.0);
        let py = |db: f32| (2.5 + (RANGE - db) / (2.0 * RANGE) * (h as f32 - 5.0)).clamp(2.0, h as f32 - 2.5);
        let bottom = (h - 1) as f32;
        for hz in [100.0, 1000.0, 10000.0] {
            let x = px(freq_t(hz));
            c.line(x, 1.0, x, bottom, 1.0, look.grid);
        }
        for db in [-12.0f32, 12.0] {
            c.line(1.0, py(db), (pw - 1) as f32, py(db), 1.0, look.grid);
        }
        c.line(1.0, py(0.0), (pw - 1) as f32, py(0.0), 1.0, look.separator);

        let accent = if self.on { look.accent } else { look.muted };
        let all = ["freq", "gain", "q"];
        let n = ((pw - 4) / 2).clamp(40, 120);
        let pts: Vec<(f32, f32)> = (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                (px(t), py(self.db_at(20.0 * 1000f32.powf(t))))
            })
            .collect();
        c.area(&pts, py(0.0), c.ink(look.fill_of(accent), &all));
        if let Some(b) = base {
            let under: Vec<(f32, f32)> = (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    (px(t), py(b.db_at(20.0 * 1000f32.powf(t))))
                })
                .collect();
            c.polyline(&under, 1.5, c.ink(look.muted, &all));
        }
        c.polyline(&pts, 2.0, c.ink(accent, &all));

        // (Kept inside the plot at the ends of the range.)
        let hx = px(freq_t(self.freq)).clamp(5.0, pw as f32 - 5.0);
        let hy = py(self.db_at(self.freq));
        let mark = if self.on { look.record } else { look.weak };
        c.dot(hx, hy.clamp(5.0, h as f32 - 5.0), 3.5, c.ink(mark, &["freq", "gain"]));
        if c.lit("freq") {
            c.line(hx, 1.0, hx, bottom, 1.5, mark);
        }
        if c.lit("gain") {
            let y = py(self.gain);
            c.line(1.0, y, (pw - 1) as f32, y, 1.5, mark);
        }
        if c.lit("q") {
            // The bandwidth at half the gain (3 dB for a cut), in octaves.
            let bw = 2.0 * (1.0 / (2.0 * self.q)).asinh() / std::f32::consts::LN_2;
            let half = if matches!(self.kind, Kind::Bell | Kind::LowShelf | Kind::HighShelf) { self.gain / 2.0 } else { -3.0 };
            let (f0, f1) = (self.freq * 2f32.powf(-bw / 2.0), self.freq * 2f32.powf(bw / 2.0));
            let (x0, x1) = (px(freq_t(f0)).max(3.0), px(freq_t(f1)).min(pw as f32 - 4.0));
            let y = py(half);
            c.line(x0, y, x1, y, 1.5, mark);
            for x in [x0, x1] {
                c.line(x, y - 5.0, x, y + 5.0, 1.5, mark);
            }
        }

        let shade = |name: &str| {
            if !self.on {
                look.weak
            } else if c.focus.is_some() && !c.lit(name) {
                look.weak
            } else {
                look.text
            }
        };
        let big = look.big;
        let nx = pw + 8;
        let tw = num_w - 8;
        let rows = [
            ("FREQ", hz_text(self.freq), shade("freq")),
            ("GAIN", db_text(self.gain), shade("gain")),
            ("Q", format!("Q {:.2}", self.q), shade("q")),
        ];
        let row_h = h / 3;
        for (i, (name, value, ink)) in rows.iter().enumerate() {
            let y = i as i32 * row_h;
            if *name != "Q" {
                c.text(nx, y + 2, tw, 10, name, look.small, look.weak, Align::Left);
                c.text(nx, y + 11, tw, 22, value, big, *ink, Align::Left);
            } else {
                c.text(nx, y + 11, tw, 22, value, look.label, *ink, Align::Left);
            }
        }
        if w >= 3 * super::CELL_W {
            for (hz, name) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
                let x = px(freq_t(hz)) as i32;
                c.text(x - 16, h - 14, 32, 12, name, look.small, look.weak, Align::Center);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::golden;

    #[test]
    fn bell_hits_its_gain_at_its_freq() {
        for (g, q) in [(6.0, 0.707), (-9.0, 2.0), (12.0, 8.0)] {
            let db = magnitude_db(Kind::Bell, 1000.0, g, q, 1000.0);
            assert!((db - g).abs() < 0.1, "{g} {q} {db}");
        }
        assert!(magnitude_db(Kind::Bell, 1000.0, 6.0, 2.0, 20.0).abs() < 0.3);
    }

    #[test]
    fn shelves_reach_their_gain() {
        assert!((magnitude_db(Kind::LowShelf, 1000.0, 9.0, 0.707, 20.0) - 9.0).abs() < 0.5);
        assert!(magnitude_db(Kind::LowShelf, 1000.0, 9.0, 0.707, 18000.0).abs() < 0.5);
        assert!((magnitude_db(Kind::HighShelf, 1000.0, -7.0, 0.707, 18000.0) + 7.0).abs() < 0.7);
        assert!(magnitude_db(Kind::HighShelf, 1000.0, -7.0, 0.707, 20.0).abs() < 0.5);
    }

    #[test]
    fn cuts_cut() {
        assert!(magnitude_db(Kind::LowCut, 1000.0, 0.0, 0.707, 50.0) < -30.0);
        assert!(magnitude_db(Kind::LowCut, 1000.0, 0.0, 0.707, 15000.0).abs() < 0.5);
        assert!(magnitude_db(Kind::HighCut, 1000.0, 0.0, 0.707, 15000.0) < -30.0);
    }

    #[test]
    fn band_bell() {
        let b = Band { freq: 2500.0, gain: 7.5, q: 1.8, ..Default::default() };
        golden::check("band-bell", 2, None, |cv| b.draw(cv));
    }

    #[test]
    fn band_shelf_off() {
        let b = Band { freq: 120.0, gain: -6.0, q: 0.9, kind: Kind::LowShelf, on: false, ..Default::default() };
        golden::check("band-low-shelf-off", 2, None, |cv| b.draw(cv));
    }

    #[test]
    fn band_modulated() {
        let b = Band { freq: 1500.0, gain: 3.0, q: 1.2, freq_mod: Some(4000.0), gain_mod: Some(9.0), ..Default::default() };
        golden::check("band-bell-mod", 2, None, |cv| b.draw(cv));
    }

    #[test]
    fn touching_each_input_lights_it() {
        let b = Band { freq: 2500.0, gain: 7.5, q: 1.8, ..Default::default() };
        for name in ["freq", "gain", "q"] {
            golden::check(&format!("band-bell-focus-{name}"), 2, Some(name), |cv| b.draw(cv));
            assert!(golden::focus_changes(240, name, |cv| b.draw(cv)));
        }
    }

    #[test]
    fn scripts_draw_a_band() {
        let mut p = crate::screen::Picture::new([0; 3]);
        let lua = mlua::Lua::new();
        let t: Table = lua.load("return { type = 'high_shelf', freq = 8000, gain = 3, on = false }").eval().unwrap();
        let b = Band::from_lua(&t).unwrap();
        assert_eq!(b.kind, Kind::HighShelf);
        assert!(!b.on);
        let look = crate::widget::Look::fallback();
        crate::widget::draw_lua("band", &mut p, crate::widget::Rect { x: 0, y: 0, w: 240, h: 104 }, &t, &look).unwrap();
        assert!(p.ops.len() > 5);
    }
}
