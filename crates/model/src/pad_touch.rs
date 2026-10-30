//! A sound's Pad Touch: what pressing a held MK3 pad harder or softer does for it
//! (docs/superpowers/specs/2026-10-24-pad-touch-design.md). DECKARD's own setting: kept
//! on the sound's object as `deckard:` fields ([`FIELDS`]) and saved in the DECKARD
//! block (docs/deckard-block.md).

/// What a held pad's pressure sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TouchMode {
    /// Nothing: the hit's velocity is all (pressure sets Note Repeat's velocity).
    #[default]
    Velocity,
    /// Poly aftertouch on the pad's note.
    PolyAt,
    /// Channel aftertouch.
    Aftertouch,
    /// A controller ([`PadTouch::cc`]).
    Cc,
}

impl TouchMode {
    pub const ALL: [TouchMode; 4] = [TouchMode::Velocity, TouchMode::PolyAt, TouchMode::Aftertouch, TouchMode::Cc];

    /// The mode at `i` (0–3; others clamp).
    pub fn from_index(i: i64) -> TouchMode {
        TouchMode::ALL[i.clamp(0, 3) as usize]
    }

    pub fn index(self) -> i64 {
        self as i64
    }

    /// Its name as DECKARD shows it ("Poly AT").
    pub fn name(self) -> &'static str {
        match self {
            TouchMode::Velocity => "Velocity",
            TouchMode::PolyAt => "Poly AT",
            TouchMode::Aftertouch => "Aftertouch",
            TouchMode::Cc => "CC",
        }
    }
}

/// A sound's Pad Touch settings: `curve`, `min` and `max` in percent (0–100; `max` ≥
/// `min`), `cc` 0–127.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PadTouch {
    pub mode: TouchMode,
    pub cc: u8,
    pub curve: u8,
    pub min: u8,
    pub max: u8,
}

impl Default for PadTouch {
    fn default() -> PadTouch {
        PadTouch { mode: TouchMode::Velocity, cc: 1, curve: 50, min: 0, max: 100 }
    }
}

/// The sound object's fields, in [`PadTouch::values`] order (Int values).
pub const FIELDS: [&str; 5] = ["deckard:touch_mode", "deckard:touch_cc", "deckard:touch_curve", "deckard:touch_min", "deckard:touch_max"];

impl PadTouch {
    pub fn is_default(&self) -> bool {
        *self == PadTouch::default()
    }

    /// Within its ranges: `cc` to 127, the percentages to 100, `max` at least `min`.
    pub fn clamped(self) -> PadTouch {
        let min = self.min.min(100);
        PadTouch { cc: self.cc.min(127), curve: self.curve.min(100), min, max: self.max.min(100).max(min), ..self }
    }

    /// Its values in [`FIELDS`] order.
    pub fn values(&self) -> [i64; 5] {
        [self.mode.index(), self.cc as i64, self.curve as i64, self.min as i64, self.max as i64]
    }

    /// From values in [`FIELDS`] order (each `None` keeps the default), clamped.
    pub fn from_values(v: [Option<i64>; 5]) -> PadTouch {
        let d = PadTouch::default();
        let byte = |x: Option<i64>, or: u8| x.map_or(or, |x| x.clamp(0, 255) as u8);
        PadTouch {
            mode: v[0].map_or(d.mode, TouchMode::from_index),
            cc: byte(v[1], d.cc),
            curve: byte(v[2], d.curve),
            min: byte(v[3], d.min),
            max: byte(v[4], d.max),
        }
        .clamped()
    }
}

/// A pressure (0..1) shaped by a curve (0..1, 0.5 straight): above 0.5 light presses
/// count for more (an exponent down to 1/4), below less (up to 4).
pub fn shape(x: f64, curve: f64) -> f64 {
    x.clamp(0.0, 1.0).powf(4f64.powf((0.5 - curve.clamp(0.0, 1.0)) * 2.0))
}

/// A controller's number with its common name ("1 Mod", "74"), as DECKARD shows it.
pub fn cc_label(n: u8) -> String {
    let name = match n {
        0 => "Bank",
        1 => "Mod",
        2 => "Breath",
        7 => "Volume",
        10 => "Pan",
        11 => "Expression",
        64 => "Sustain",
        _ => return n.to_string(),
    };
    format!("{n} {name}")
}
