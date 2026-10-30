//! Modulation's values (docs/superpowers/plans/2026-10-22-phase2c-modulation.md): a
//! knob's point is a deviation from the knob's value, as a fraction of its travel; a
//! selector's or button's is the value itself. How a knob travels over its parameter's
//! stored values (its [`Scale`]) is the app's knowledge (its pages): it registers the
//! scales here ([`set_scales`]), and timelines turn points into stored values with them
//! when they are made (off the audio thread).

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// How a knob travels over a parameter's stored values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scale {
    /// From `.0` to `.1`, evenly.
    Lin(f64, f64),
    /// From `.0` to `.1` (both > 0), evenly in their ratio.
    Log(f64, f64),
    /// From `.0` to `.1` in whole steps.
    Int(f64, f64),
    /// A selector or button: the point is the value.
    Absolute,
}

impl Scale {
    /// The stored value of a parameter at `base` moved by `dev` (a fraction of the
    /// knob's travel; the value itself for [`Scale::Absolute`]), within the range.
    pub fn apply(self, base: f64, dev: f64) -> f64 {
        let lin = |a: f64, b: f64| {
            if b == a {
                return a;
            }
            let at = ((base - a) / (b - a) + dev).clamp(0.0, 1.0);
            a + at * (b - a)
        };
        match self {
            Scale::Lin(a, b) => lin(a, b),
            Scale::Int(a, b) => lin(a, b).round(),
            Scale::Log(lo, hi) if lo > 0.0 && hi > lo && base > 0.0 => {
                let at = ((base / lo).ln() / (hi / lo).ln() + dev).clamp(0.0, 1.0);
                lo * (hi / lo).powf(at)
            }
            Scale::Log(lo, hi) => lin(lo, hi),
            Scale::Absolute => dev,
        }
    }
}

impl Scale {
    /// The stored value at point `t` (0..1) of the knob's travel ([`Scale::travel`]'s
    /// inverse); a selector's: `t`.
    pub fn at(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Scale::Lin(a, b) => a + t * (b - a),
            Scale::Int(a, b) => (a + t * (b - a)).round(),
            Scale::Log(lo, hi) if lo > 0.0 && hi > lo => lo * (hi / lo).powf(t),
            Scale::Log(lo, hi) => lo + t * (hi - lo),
            Scale::Absolute => t,
        }
    }
}

impl Scale {
    /// Where on the knob's travel (0..1) stored value `v` is; a selector's: `v`.
    pub fn travel(self, v: f64) -> f64 {
        let lin = |a: f64, b: f64| if b == a { 0.0 } else { ((v - a) / (b - a)).clamp(0.0, 1.0) };
        match self {
            Scale::Lin(a, b) | Scale::Int(a, b) => lin(a, b),
            Scale::Log(lo, hi) if lo > 0.0 && hi > lo && v > 0.0 => ((v / lo).ln() / (hi / lo).ln()).clamp(0.0, 1.0),
            Scale::Log(lo, hi) => lin(lo, hi),
            Scale::Absolute => v,
        }
    }
}

fn scales() -> &'static RwLock<HashMap<(String, String), Scale>> {
    static SCALES: OnceLock<RwLock<HashMap<(String, String), Scale>>> = OnceLock::new();
    SCALES.get_or_init(Default::default)
}

/// Register scales, by (module class without its namespace, field).
pub fn set_scales(t: impl IntoIterator<Item = ((String, String), Scale)>) {
    let mut s = scales().write().unwrap_or_else(|e| e.into_inner());
    s.extend(t);
}

/// The scale of `module`'s `field`: as registered, else 0..1 (most of Maschine's
/// continuous parameters).
pub fn scale_of(module: &str, field: &str) -> Scale {
    let s = scales().read().unwrap_or_else(|e| e.into_inner());
    s.get(&(module.to_string(), field.to_string())).copied().unwrap_or(Scale::Lin(0.0, 1.0))
}

/// A parameter's modulation over a stretch of a timeline (one pass of a pattern):
/// the module in chain `slot` of sound `sound` (`None`: the group's chain) of `group`,
/// its `field` at stored values `points` (tick, value), from tick `from` to `to`.
#[derive(Debug, Clone, PartialEq)]
pub struct ModLane {
    pub group: usize,
    pub sound: Option<usize>,
    pub slot: usize,
    pub field: String,
    pub from: i64,
    pub to: i64,
    pub points: Vec<(i64, f64)>,
}

impl ModLane {
    /// The value at `tick`: points joined by lines, the first before it, the last after.
    pub fn value_at(&self, tick: f64) -> f64 {
        let p = &self.points;
        let Some(first) = p.first() else { return 0.0 };
        let i = p.partition_point(|x| (x.0 as f64) <= tick);
        if i == 0 {
            return first.1;
        }
        if i == p.len() {
            return p[i - 1].1;
        }
        let (a, b) = (p[i - 1], p[i]);
        let f = (tick - a.0 as f64) / (b.0 - a.0).max(1) as f64;
        a.1 + (b.1 - a.1) * f
    }
}

#[cfg(test)]
mod tests {
    use super::Scale;

    #[test]
    fn deviations_move_within_the_range() {
        assert!((Scale::Lin(0.0, 1.0).apply(0.5, 0.25) - 0.75).abs() < 1e-12);
        assert_eq!(Scale::Lin(0.0, 1.0).apply(0.5, 0.9), 1.0, "clamped");
        assert!((Scale::Lin(-24.0, 24.0).apply(0.0, -0.5) + 24.0).abs() < 1e-12);
        assert_eq!(Scale::Int(0.0, 10.0).apply(5.0, 0.12), 6.0);
        // A log knob: 20 Hz..20 kHz, at 632 Hz (the middle) up a quarter: 3.56 kHz.
        let v = Scale::Log(20.0, 20_000.0).apply(632.455_532, 0.25);
        assert!((v - 3556.558_820).abs() < 0.01, "{v}");
        assert_eq!(Scale::Absolute.apply(2.0, 1.0), 1.0, "a selector's point is its value");
        assert!((Scale::Log(20.0, 20_000.0).travel(632.455_532) - 0.5).abs() < 1e-6);
        assert_eq!(Scale::Lin(-24.0, 24.0).travel(12.0), 0.75);
    }

    #[test]
    fn scale_at_inverts_travel() {
        for sc in [Scale::Lin(-24.0, 24.0), Scale::Log(20.0, 20_000.0), Scale::Lin(0.0, 1.0)] {
            for t in [0.0, 0.3, 0.5, 1.0] {
                assert!((sc.travel(sc.at(t)) - t).abs() < 1e-9, "{sc:?} at {t}");
            }
        }
        assert_eq!(Scale::Int(0.0, 10.0).at(0.44), 4.0);
        assert_eq!(Scale::Absolute.at(0.7), 0.7);
        assert_eq!(Scale::Lin(0.0, 2.0).at(1.5), 2.0, "clamped");
    }
}
