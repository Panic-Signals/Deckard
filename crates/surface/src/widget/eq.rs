//! `eq`: a parametric EQ. The summed response of its bands as one curve, and each band
//! as a numbered dot on it.
//!
//! Inputs (all optional):
//!
//! | input | |
//! |---|---|
//! | `bands` | up to 8 tables: `freq` (Hz), `gain` (dB), `q`, `type` (`"bell"` the default, `"low_shelf"`, `"high_shelf"`, `"low_cut"`, `"high_cut"`), `on` (default true) |
//! | `output` | the output gain in dB, added to the sum |
//!
//! The plot is 20 Hz to 20 kHz across and ±18 dB up. Bands that are off draw as a muted
//! ring and add nothing. A dot's size follows its Q (wide bands are big).
//!
//! Touch focus names are `band1` to `band8`: the touched band's dot becomes a ring, its
//! own curve shows, and the sum and the other dots dim.

use mlua::Table;

use super::{Align, Canvas, num, text};

const FS: f32 = 48000.0;
const RANGE_DB: f32 = 18.0;
const MAX_BANDS: usize = 8;
const NAMES: [&str; MAX_BANDS] = ["band1", "band2", "band3", "band4", "band5", "band6", "band7", "band8"];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum BandKind {
    #[default]
    Bell,
    LowShelf,
    HighShelf,
    LowCut,
    HighCut,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EqBand {
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
    pub kind: BandKind,
    pub on: bool,
}

impl Default for EqBand {
    fn default() -> Self {
        EqBand { freq: 1000.0, gain: 0.0, q: 0.707, kind: BandKind::Bell, on: true }
    }
}

impl EqBand {
    /// This band's response at `hz`, in dB (RBJ cookbook biquads at 48 kHz).
    pub fn response_db(&self, hz: f32) -> f32 {
        let f0 = self.freq.clamp(10.0, FS * 0.45);
        let q = self.q.clamp(0.1, 30.0);
        let a_lin = 10f32.powf(self.gain / 40.0);
        let w0 = std::f32::consts::TAU * f0 / FS;
        let (sw, cw) = w0.sin_cos();
        let alpha = sw / (2.0 * q);
        let (b, a): ([f32; 3], [f32; 3]) = match self.kind {
            BandKind::Bell => {
                ([1.0 + alpha * a_lin, -2.0 * cw, 1.0 - alpha * a_lin], [1.0 + alpha / a_lin, -2.0 * cw, 1.0 - alpha / a_lin])
            }
            BandKind::LowShelf => {
                let s = 2.0 * a_lin.sqrt() * alpha;
                (
                    [
                        a_lin * ((a_lin + 1.0) - (a_lin - 1.0) * cw + s),
                        2.0 * a_lin * ((a_lin - 1.0) - (a_lin + 1.0) * cw),
                        a_lin * ((a_lin + 1.0) - (a_lin - 1.0) * cw - s),
                    ],
                    [
                        (a_lin + 1.0) + (a_lin - 1.0) * cw + s,
                        -2.0 * ((a_lin - 1.0) + (a_lin + 1.0) * cw),
                        (a_lin + 1.0) + (a_lin - 1.0) * cw - s,
                    ],
                )
            }
            BandKind::HighShelf => {
                let s = 2.0 * a_lin.sqrt() * alpha;
                (
                    [
                        a_lin * ((a_lin + 1.0) + (a_lin - 1.0) * cw + s),
                        -2.0 * a_lin * ((a_lin - 1.0) + (a_lin + 1.0) * cw),
                        a_lin * ((a_lin + 1.0) + (a_lin - 1.0) * cw - s),
                    ],
                    [
                        (a_lin + 1.0) - (a_lin - 1.0) * cw + s,
                        2.0 * ((a_lin - 1.0) - (a_lin + 1.0) * cw),
                        (a_lin + 1.0) - (a_lin - 1.0) * cw - s,
                    ],
                )
            }
            BandKind::LowCut => ([(1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0], [1.0 + alpha, -2.0 * cw, 1.0 - alpha]),
            BandKind::HighCut => ([(1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0], [1.0 + alpha, -2.0 * cw, 1.0 - alpha]),
        };
        let w = std::f32::consts::TAU * hz.clamp(1.0, FS * 0.5) / FS;
        let mag2 = |c: [f32; 3]| {
            let re = c[0] + c[1] * w.cos() + c[2] * (2.0 * w).cos();
            let im = -(c[1] * w.sin() + c[2] * (2.0 * w).sin());
            re * re + im * im
        };
        10.0 * (mag2(b).max(1e-20) / mag2(a).max(1e-20)).log10()
    }

    fn is_cut(&self) -> bool {
        matches!(self.kind, BandKind::LowCut | BandKind::HighCut)
    }
}

/// The sum of the bands that are on, plus `output`, at `hz`, in dB.
pub fn sum_db(bands: &[EqBand], output: f32, hz: f32) -> f32 {
    output + bands.iter().filter(|b| b.on).map(|b| b.response_db(hz)).sum::<f32>()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Eq {
    pub bands: Vec<EqBand>,
    pub output: f32,
}

impl Eq {
    pub fn from_lua(t: &Table) -> mlua::Result<Eq> {
        let mut bands = Vec::new();
        if let Ok(Some(list)) = t.raw_get::<Option<Table>>("bands") {
            for b in list.sequence_values::<Table>().flatten().take(MAX_BANDS) {
                let kind = match text(&b, "type").as_deref() {
                    Some("low_shelf") => BandKind::LowShelf,
                    Some("high_shelf") => BandKind::HighShelf,
                    Some("low_cut") => BandKind::LowCut,
                    Some("high_cut") => BandKind::HighCut,
                    _ => BandKind::Bell,
                };
                let on = match b.raw_get::<Option<bool>>("on") {
                    Ok(Some(v)) => v,
                    _ => true,
                };
                bands.push(EqBand {
                    freq: num(&b, "freq").unwrap_or(1000.0),
                    gain: num(&b, "gain").unwrap_or(0.0),
                    q: num(&b, "q").unwrap_or(0.707),
                    kind,
                    on,
                });
            }
        }
        Ok(Eq { bands, output: num(t, "output").unwrap_or(0.0) })
    }

    pub fn draw(&self, c: &mut Canvas) {
        let look = *c.look;
        let (w, h) = (c.r.w, c.r.h);
        c.fill(0, 0, w, h, look.panel);
        let px = |t: f32| 2.0 + t * (w - 5) as f32;
        let py = |db: f32| {
            let mid = (h - 1) as f32 / 2.0;
            (mid - db / RANGE_DB * (mid - 2.0)).clamp(2.0, h as f32 - 3.0)
        };
        for hz in [100.0, 1000.0, 10000.0] {
            let x = px(super::freq_t(hz));
            c.line(x, 1.0, x, (h - 1) as f32, 1.0, look.grid);
        }
        for db in [-12.0f32, -6.0, 0.0, 6.0, 12.0] {
            let y = py(db);
            c.line(1.0, y, (w - 1) as f32, y, 1.0, if db == 0.0 { look.separator } else { look.grid });
        }
        let n = (((w - 5) / 3).max(40)) as usize;
        let hz_at = |i: usize| 20.0 * 1000f32.powf(i as f32 / n as f32);
        let curve: Vec<(f32, f32)> = (0..=n).map(|i| (px(i as f32 / n as f32), py(sum_db(&self.bands, self.output, hz_at(i))))).collect();
        let sum_ink = c.ink(look.accent, &["sum"]);
        c.area(&curve, py(0.0), c.ink(look.fill_of(look.accent), &["sum"]));
        c.polyline(&curve, 2.0, sum_ink);

        for (i, b) in self.bands.iter().enumerate() {
            let name = NAMES[i];
            let t = ((b.q.clamp(0.3, 10.0)).ln() - 0.3f32.ln()) / (10f32.ln() - 0.3f32.ln());
            let radius = 9.0 - 5.0 * t;
            let db = if b.is_cut() { sum_db(&self.bands, self.output, b.freq) } else { b.gain };
            let edge = radius + 2.0;
            let cx = px(super::freq_t(b.freq)).clamp(edge, w as f32 - edge);
            let cy = py(db).clamp(edge, h as f32 - edge);
            let lit = c.lit(name);
            if lit && b.on {
                let own: Vec<(f32, f32)> = (0..=n).map(|k| (px(k as f32 / n as f32), py(b.response_db(hz_at(k))))).collect();
                c.polyline(&own, 1.0, look.marker);
            }
            if !b.on {
                c.ring(cx, cy, radius, 1.5, c.ink(look.muted, &[name]));
            } else if lit {
                c.ring(cx, cy, radius, 2.0, look.record);
            } else {
                c.dot(cx, cy, radius, c.ink(look.record, &[name]));
            }
            if !lit || !b.on {
                let col = if b.on { look.on_colour } else { look.weak };
                c.text(cx as i32 - 7, cy as i32 - 7, 14, 14, &(i + 1).to_string(), look.small, col, Align::Center);
            } else {
                c.text(cx as i32 - 7, cy as i32 - 7, 14, 14, &(i + 1).to_string(), look.small, look.text, Align::Center);
            }
        }
        for (hz, name) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
            let x = px(super::freq_t(hz)) as i32;
            c.text(x - 16, h - 14, 32, 12, name, look.small, look.weak, Align::Center);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::golden;

    fn bell(freq: f32, gain: f32, q: f32) -> EqBand {
        EqBand { freq, gain, q, ..Default::default() }
    }

    fn rig() -> Eq {
        Eq {
            bands: vec![
                EqBand { freq: 60.0, kind: BandKind::LowCut, q: 0.707, ..Default::default() },
                EqBand { freq: 200.0, gain: 4.0, q: 0.7, kind: BandKind::LowShelf, ..Default::default() },
                bell(900.0, -5.0, 3.0),
                bell(3000.0, 6.0, 1.0),
                EqBand { freq: 8000.0, gain: -3.0, q: 0.5, kind: BandKind::HighShelf, ..Default::default() },
                EqBand { freq: 14000.0, gain: 5.0, on: false, ..Default::default() },
            ],
            output: 0.0,
        }
    }

    #[test]
    fn a_bell_peaks_at_its_gain() {
        let b = bell(1000.0, 9.0, 2.0);
        assert!((b.response_db(1000.0) - 9.0).abs() < 0.1);
        assert!(b.response_db(50.0).abs() < 0.5);
    }

    #[test]
    fn bells_sum() {
        let (a, b) = (bell(1000.0, 4.0, 1.0), bell(1000.0, 3.0, 1.0));
        assert!((sum_db(&[a, b], 0.0, 1000.0) - 7.0).abs() < 0.2);
        assert!((sum_db(&[], 2.0, 500.0) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn an_off_band_adds_nothing() {
        let off = EqBand { on: false, ..bell(1000.0, 9.0, 2.0) };
        assert!(sum_db(&[off], 0.0, 1000.0).abs() < 1e-4);
    }

    #[test]
    fn a_low_cut_is_deep_below_its_freq() {
        let b = EqBand { freq: 500.0, kind: BandKind::LowCut, ..Default::default() };
        assert!(b.response_db(50.0) < -30.0);
        assert!(b.response_db(10000.0).abs() < 0.5);
    }

    #[test]
    fn eq_golden() {
        let e = rig();
        golden::check("eq-rig", 3, None, |cv| e.draw(cv));
        let few = Eq { bands: vec![bell(120.0, 3.0, 1.0), bell(5000.0, -6.0, 6.0)], output: 2.0 };
        golden::check("eq-two-bells", 3, None, |cv| few.draw(cv));
    }

    #[test]
    fn touching_a_band_lights_it() {
        let e = rig();
        golden::check("eq-focus-band3", 3, Some("band3"), |cv| e.draw(cv));
        assert!(golden::focus_changes(360, "band3", |cv| e.draw(cv)));
    }

    #[test]
    fn scripts_draw_an_eq() {
        let mut p = crate::screen::Picture::new([0; 3]);
        let lua = mlua::Lua::new();
        let t: Table = lua
            .load("return { output = 1, bands = { { freq = 100, gain = 3, q = 1 }, { freq = 5000, type = 'high_cut', on = false } } }")
            .eval()
            .unwrap();
        let e = Eq::from_lua(&t).unwrap();
        assert_eq!(e.bands.len(), 2);
        assert!(e.bands[0].on && !e.bands[1].on && e.bands[1].kind == BandKind::HighCut);
        let look = crate::widget::Look::fallback();
        crate::widget::draw_lua("eq", &mut p, crate::widget::Rect { x: 0, y: 0, w: 360, h: 104 }, &t, &look).unwrap();
        assert!(p.ops.len() > 10);
    }
}
