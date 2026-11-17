//! `envelope`: an envelope's shape, level over time, with a flat fill under it.
//!
//! Segments run along x in order delay, attack, hold, decay, sustain, release. Time is
//! compressed (square root of each time, and every present segment gets at least 12% of
//! the width) so a short attack stays visible; the sustain, which has no time, takes a
//! fixed fifth. Under the plot sit the segment letters, and at 360 px and wider the times
//! of attack, decay and release.
//!
//! Inputs (all optional; times in ms):
//!
//! | input | |
//! |---|---|
//! | `delay` | time before the attack (default 0: no segment) |
//! | `attack` | time up to the peak (default 10) |
//! | `hold` | time at the peak (default 0: no segment) |
//! | `decay` | time down to the sustain (default 200) |
//! | `sustain` | level 0..1 (default 0.6). Missing, with `release` missing too: an AD envelope (decays to 0, no sustain or release) |
//! | `release` | time down to 0 after the sustain (default 300) |
//! | `curve` | -1..1 (default 0, linear); positive: decays and releases fall fast first, attacks rise slowly first |
//! | `amount` | -1..1 (default 1); negative draws the shape flipped from the top |
//!
//! Touch focus on `attack`, `decay`, `sustain`, `release`, `delay` or `hold` lights that
//! segment and shows its value larger beside it, dimming the rest; `amount` and `curve`
//! light the whole shape and show their value.

use mlua::Table;

use super::{Align, Canvas, ms_text, num};

#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    pub delay: f32,
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    /// `None`: an AD envelope.
    pub sustain: Option<f32>,
    pub release: f32,
    pub curve: f32,
    pub amount: f32,
}

impl Default for Envelope {
    fn default() -> Envelope {
        Envelope { delay: 0.0, attack: 10.0, hold: 0.0, decay: 200.0, sustain: Some(0.6), release: 300.0, curve: 0.0, amount: 1.0 }
    }
}

/// One segment: x from `x0` to `x1` (0..1 of the plot), level from `l0` to `l1`.
#[derive(Clone, Debug, PartialEq)]
pub struct Seg {
    pub name: &'static str,
    pub x0: f32,
    pub x1: f32,
    pub l0: f32,
    pub l1: f32,
    /// +1 for segments that fall (curve bends them fast first), -1 for attack.
    pub bend: f32,
}

const MIN_SHARE: f32 = 0.12;
const SUSTAIN_SHARE: f32 = 0.2;
const STEPS: usize = 14;

/// Position `t` (0..1) along a segment with curve `k`: 0 linear, otherwise bent.
fn shape(t: f32, k: f32) -> f32 {
    let a = 4.0 * k;
    if a.abs() < 1e-3 { t } else { (1.0 - (-a * t).exp()) / (1.0 - (-a).exp()) }
}

impl Envelope {
    pub fn from_lua(t: &Table) -> mlua::Result<Envelope> {
        let d = Envelope::default();
        let ms = |k: &str, or: f32| num(t, k).unwrap_or(or).max(0.0);
        let sustain = match num(t, "sustain") {
            Some(s) => Some(s.clamp(0.0, 1.0)),
            None if num(t, "release").is_none() && (num(t, "attack").is_some() || num(t, "decay").is_some()) => None,
            None => d.sustain,
        };
        Ok(Envelope {
            delay: ms("delay", d.delay),
            attack: ms("attack", d.attack),
            hold: ms("hold", d.hold),
            decay: ms("decay", d.decay),
            sustain,
            release: ms("release", d.release),
            curve: num(t, "curve").unwrap_or(0.0).clamp(-1.0, 1.0),
            amount: num(t, "amount").unwrap_or(1.0).clamp(-1.0, 1.0),
        })
    }

    /// The segments present, with their x spans (0..1) and levels (0..1).
    pub fn segments(&self) -> Vec<Seg> {
        let sus = self.sustain;
        let end = sus.unwrap_or(0.0);
        let mut times: Vec<(&'static str, f32, f32, f32, f32)> = Vec::new();
        if self.delay > 0.0 {
            times.push(("delay", self.delay, 0.0, 0.0, 1.0));
        }
        times.push(("attack", self.attack.max(0.5), 0.0, 1.0, -1.0));
        if self.hold > 0.0 {
            times.push(("hold", self.hold, 1.0, 1.0, 1.0));
        }
        times.push(("decay", self.decay.max(0.5), 1.0, end, 1.0));
        let release = sus.is_some() && self.release > 0.0;
        let tail = if release { Some(("release", self.release, sus.unwrap_or(0.0), 0.0, 1.0)) } else { None };
        let sus_share = if sus.is_some() { SUSTAIN_SHARE } else { 0.0 };
        let n = times.len() + usize::from(tail.is_some());
        let avail = 1.0 - sus_share;
        let weights: Vec<f32> = times.iter().chain(tail.iter()).map(|t| t.1.sqrt()).collect();
        let sum: f32 = weights.iter().sum();
        let spare = (avail - n as f32 * MIN_SHARE).max(0.0);
        let share = |i: usize| MIN_SHARE + spare * weights[i] / sum;
        let mut out = Vec::new();
        let mut x = 0.0;
        let mut push = |name, w: f32, l0, l1, bend, x: &mut f32| {
            out.push(Seg { name, x0: *x, x1: *x + w, l0, l1, bend });
            *x += w;
        };
        for (i, t) in times.iter().enumerate() {
            if t.0 == "decay" {
                // (The sustain follows the decay.)
                push(t.0, share(i), t.2, t.3, t.4, &mut x);
                if let Some(s) = sus {
                    push("sustain", SUSTAIN_SHARE, s, s, 1.0, &mut x);
                }
            } else {
                push(t.0, share(i), t.2, t.3, t.4, &mut x);
            }
        }
        if let Some(t) = tail {
            push(t.0, share(times.len()), t.2, t.3, t.4, &mut x);
        }
        // (Rounding: the last segment ends at 1.)
        if let Some(last) = out.last_mut() {
            last.x1 = 1.0;
        }
        out
    }

    fn seg_points(&self, s: &Seg) -> Vec<(f32, f32)> {
        let flat = (s.l0 - s.l1).abs() < 1e-6;
        let n = if flat { 1 } else { STEPS };
        (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                (s.x0 + (s.x1 - s.x0) * t, s.l0 + (s.l1 - s.l0) * shape(t, self.curve * s.bend))
            })
            .collect()
    }

    /// The whole shape as (x, level), both 0..1: starts at level 0 and ends at 0.
    pub fn points(&self) -> Vec<(f32, f32)> {
        let mut out: Vec<(f32, f32)> = Vec::new();
        for s in self.segments() {
            let p = self.seg_points(&s);
            out.extend(if out.is_empty() { &p[..] } else { &p[1..] });
        }
        if self.sustain.is_none() {
            // (AD: ends where the decay does, at 0.)
        }
        out
    }

    fn value_text(&self, name: &str) -> String {
        match name {
            "delay" => ms_text(self.delay),
            "attack" => ms_text(self.attack),
            "hold" => ms_text(self.hold),
            "decay" => ms_text(self.decay),
            "release" => ms_text(self.release),
            "sustain" => format!("{:.0}%", self.sustain.unwrap_or(0.0) * 100.0),
            "amount" => format!("{:.0}%", self.amount * 100.0),
            _ => format!("{:+.2}", self.curve),
        }
    }

    pub fn draw(&self, c: &mut Canvas) {
        let look = *c.look;
        let (w, h) = (c.r.w, c.r.h);
        c.fill(0, 0, w, h, look.panel);
        let times = w >= 360;
        let base = (h - if times { 29 } else { 17 }) as f32;
        let top = 3.0;
        let (x0, pw) = (4.0, (w - 8) as f32);
        let flip = self.amount < 0.0;
        let amt = self.amount.abs();
        let px = |x: f32| x0 + x * pw;
        let py = |l: f32| {
            let v = if flip { 1.0 - l * amt } else { l * amt };
            base - v * (base - top)
        };
        let segs = self.segments();
        c.line(x0, base, x0 + pw, base, 1.0, look.separator);
        for s in segs.iter().skip(1) {
            c.line(px(s.x0), top, px(s.x0), base, 1.0, look.grid);
        }
        for s in &segs {
            let names = [s.name, "amount", "curve"];
            let pts: Vec<(f32, f32)> = self.seg_points(s).iter().map(|&(x, l)| (px(x), py(l))).collect();
            c.area(&pts, base, c.ink(look.fill_of(look.accent), &names));
            c.polyline(&pts, 2.0, c.ink(look.accent, &names));
        }
        // Letters under the plot, and the times.
        let mut lit: Option<(f32, String)> = None;
        for s in &segs {
            let mid = px((s.x0 + s.x1) / 2.0);
            let letter = match s.name {
                "delay" | "decay" => "D",
                "attack" => "A",
                "hold" => "H",
                "sustain" => "S",
                _ => "R",
            };
            c.text(mid as i32 - 12, base as i32 + 3, 24, 12, letter, look.small, look.weak, Align::Center);
            if times && matches!(s.name, "attack" | "decay" | "release") {
                let x = (mid as i32 - 28).clamp(0, w - 56);
                c.text(x, base as i32 + 15, 56, 12, &self.value_text(s.name), look.small, look.weak, Align::Center);
            }
            if c.lit(s.name) {
                lit = Some((mid, self.value_text(s.name)));
            }
        }
        if c.lit("amount") || c.lit("curve") {
            let name = if c.lit("amount") { "amount" } else { "curve" };
            lit = Some((x0 + pw / 2.0, self.value_text(name)));
        }
        if let Some((mid, txt)) = lit {
            let x = (mid as i32 - 40).clamp(0, w - 80);
            c.text(x, base as i32 - 20, 80, 16, &txt, look.label, look.text, Align::Center);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::golden;

    fn adsr() -> Envelope {
        Envelope { attack: 20.0, decay: 400.0, sustain: Some(0.55), release: 800.0, ..Default::default() }
    }

    #[test]
    fn adsr_shape() {
        let e = adsr();
        let p = e.points();
        assert_eq!(p[0], (0.0, 0.0));
        let last = p[p.len() - 1];
        assert!((last.0 - 1.0).abs() < 1e-5 && last.1.abs() < 1e-6);
        let segs = e.segments();
        let attack = &segs[0];
        assert_eq!(attack.name, "attack");
        assert!(p.iter().any(|&(x, l)| (x - attack.x1).abs() < 1e-5 && (l - 1.0).abs() < 1e-5));
        assert!(p.iter().all(|&(_, l)| l <= 1.0 + 1e-5));
        let sus = segs.iter().find(|s| s.name == "sustain").unwrap();
        let mid = (sus.x0 + sus.x1) / 2.0;
        assert!(p.iter().any(|&(x, l)| x >= mid - 0.01 && x <= sus.x1 && (l - 0.55).abs() < 1e-5));
        // Every segment is at least 12% wide, and they fill the plot.
        assert!(segs.iter().filter(|s| s.name != "sustain").all(|s| s.x1 - s.x0 >= MIN_SHARE - 1e-4));
        assert!((segs[segs.len() - 1].x1 - 1.0).abs() < 1e-5);
    }

    #[test]
    fn ad_has_no_release() {
        let e = Envelope { sustain: None, ..adsr() };
        let names: Vec<_> = e.segments().iter().map(|s| s.name).collect();
        assert_eq!(names, ["attack", "decay"]);
        let p = e.points();
        assert!(p[p.len() - 1].1.abs() < 1e-6);
    }

    #[test]
    fn delay_and_hold_add_segments() {
        let e = Envelope { delay: 50.0, hold: 80.0, ..adsr() };
        let names: Vec<_> = e.segments().iter().map(|s| s.name).collect();
        assert_eq!(names, ["delay", "attack", "hold", "decay", "sustain", "release"]);
    }

    #[test]
    fn envelope_adsr() {
        let e = adsr();
        golden::check("envelope-adsr", 2, None, |cv| e.draw(cv));
    }

    #[test]
    fn envelope_ad_inverted() {
        let e = Envelope { attack: 4.0, decay: 600.0, sustain: None, curve: 0.6, amount: -0.8, ..Default::default() };
        golden::check("envelope-ad-inverted", 2, None, |cv| e.draw(cv));
    }

    #[test]
    fn envelope_full() {
        let e = Envelope { delay: 120.0, hold: 150.0, curve: -0.4, ..adsr() };
        golden::check("envelope-full", 2, None, |cv| e.draw(cv));
    }

    #[test]
    fn touching_the_decay_lights_it() {
        let e = adsr();
        golden::check("envelope-adsr-focus-decay", 2, Some("decay"), |cv| e.draw(cv));
        assert!(golden::focus_changes(240, "decay", |cv| e.draw(cv)));
        assert!(golden::focus_changes(480, "sustain", |cv| e.draw(cv)));
    }

    #[test]
    fn scripts_draw_an_envelope() {
        let lua = mlua::Lua::new();
        let t: Table = lua.load("return { attack = 5, decay = 300, sustain = 0.4, release = 500, focus = 'attack' }").eval().unwrap();
        let look = crate::widget::Look::fallback();
        let mut p = crate::screen::Picture::new([0; 3]);
        crate::widget::draw_lua("envelope", &mut p, crate::widget::Rect { x: 0, y: 0, w: 240, h: 104 }, &t, &look).unwrap();
        assert!(p.ops.len() > 10);
        let ad: Table = lua.load("return { attack = 5, decay = 300 }").eval().unwrap();
        assert_eq!(Envelope::from_lua(&ad).unwrap().sustain, None);
        let empty: Table = lua.load("return {}").eval().unwrap();
        assert_eq!(Envelope::from_lua(&empty).unwrap(), Envelope::default());
    }
}
