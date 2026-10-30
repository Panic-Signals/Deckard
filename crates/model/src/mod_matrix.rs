//! DECKARD's modulation matrix (docs/superpowers/specs/2026-10-24-expression-out-design.md,
//! 2b; docs/FEATURES.md #1): a sound's routes, each a source (its notes' pressure, slide
//! or pitch) driving a parameter of its chain between a low and a high point of the
//! knob's travel. DECKARD's own: kept on the sound's object as [`FIELD`] (text) and saved
//! in the DECKARD block (docs/deckard-block.md).

use crate::lfo::Lfo;
use crate::pad_touch;

/// What drives a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModSource {
    /// The sound's hardest-pressed held note's pressure (0..1).
    Pressure,
    /// The latest held note's slide (CC74; 0..1).
    Slide,
    /// The latest held note's pitch bend (semitones).
    Pitch,
    /// The project's LFO on this pad (1–16; 0..1).
    Lfo(u8),
}

impl ModSource {
    /// The three that come from a sound's notes.
    pub const ALL: [ModSource; 3] = [ModSource::Pressure, ModSource::Slide, ModSource::Pitch];

    /// Its name as DECKARD shows it.
    pub fn name(self) -> String {
        match self {
            ModSource::Pressure => "Pressure".into(),
            ModSource::Slide => "Slide".into(),
            ModSource::Pitch => "Pitch".into(),
            ModSource::Lfo(n) => format!("LFO {n}"),
        }
    }

    fn key(self) -> String {
        match self {
            ModSource::Lfo(n) => format!("lfo{n}"),
            _ => self.name().to_lowercase(),
        }
    }

    fn from_key(k: &str) -> Option<ModSource> {
        if let Some(n) = k.strip_prefix("lfo") {
            return n.parse().ok().filter(|n| (1..=16).contains(n)).map(ModSource::Lfo);
        }
        ModSource::ALL.into_iter().find(|s| s.key() == k)
    }
}

/// What a route can take as its source: the three expressions, then each existing LFO
/// by pad.
pub fn choices(lfos: &[Lfo]) -> Vec<ModSource> {
    ModSource::ALL.into_iter().chain(lfos.iter().map(|l| ModSource::Lfo(l.pad))).collect()
}

/// The parameter of a chain slot a route drives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModParam {
    /// A field of an internal module (the Sampler's `cutoff`, an effect's `freq`, …).
    Field(String),
    /// A hosted plug-in's parameter, by id (normalized 0..1).
    Plugin(u32),
}

/// One route: `source` drives `param` of the module in chain slot `slot` (`module`: its
/// class, or the plug-in's id; a route whose slot holds another module does nothing),
/// from `low` (the source at 0) to `high` (at 1), on the knob's travel (0..1; `low` above
/// `high` inverts), along `curve` (0–100, 50 straight). Pitch spans ±`range` semitones
/// and rests at the centre.
#[derive(Debug, Clone, PartialEq)]
pub struct ModRoute {
    pub source: ModSource,
    pub slot: usize,
    pub module: String,
    pub param: ModParam,
    pub low: f32,
    pub high: f32,
    pub curve: u8,
    pub range: u8,
}

/// The sound object's field holding its routes ([`to_text`]).
pub const FIELD: &str = "deckard:mod_routes";

impl ModRoute {
    /// A route over the whole travel, straight, ±2 semitones for Pitch.
    pub fn new(source: ModSource, slot: usize, module: &str, param: ModParam) -> ModRoute {
        ModRoute { source, slot, module: module.to_string(), param, low: 0.0, high: 1.0, curve: 50, range: 2 }
    }

    /// Where on the knob's travel (0..1) the source's value `x` puts the parameter
    /// (`None`: no note counts for the source; Pressure and Slide rest at `low`, Pitch at
    /// the centre).
    pub fn travel(&self, x: Option<f32>) -> f32 {
        let curve = f64::from(self.curve.min(100)) / 100.0;
        let (lo, hi) = (self.low.clamp(0.0, 1.0), self.high.clamp(0.0, 1.0));
        let at = match self.source {
            ModSource::Pressure | ModSource::Slide | ModSource::Lfo(_) => pad_touch::shape(f64::from(x.unwrap_or(0.0)), curve) as f32,
            ModSource::Pitch => {
                let t = f64::from(x.unwrap_or(0.0)) / f64::from(self.range.max(1));
                let t = t.clamp(-1.0, 1.0);
                ((t.signum() * pad_touch::shape(t.abs(), curve) + 1.0) / 2.0) as f32
            }
        };
        lo + (hi - lo) * at
    }
}

/// Routes as text: one per line, `source slot param low high curve range module`, the
/// param `f:<field>` or `p:<id>`, the module last (it may hold spaces).
pub fn to_text(routes: &[ModRoute]) -> String {
    let mut s = String::new();
    for r in routes {
        let param = match &r.param {
            ModParam::Field(f) => format!("f:{f}"),
            ModParam::Plugin(id) => format!("p:{id}"),
        };
        s.push_str(&format!("{} {} {param} {} {} {} {} {}\n", r.source.key(), r.slot, r.low, r.high, r.curve, r.range, r.module));
    }
    s
}

/// Routes from [`to_text`]'s text; lines that don't read are skipped.
pub fn from_text(s: &str) -> Vec<ModRoute> {
    s.lines().filter_map(line).collect()
}

fn line(l: &str) -> Option<ModRoute> {
    let mut w = l.splitn(8, ' ');
    let source = ModSource::from_key(w.next()?)?;
    let slot = w.next()?.parse().ok()?;
    let param = match w.next()?.split_once(':')? {
        ("f", f) if !f.is_empty() => ModParam::Field(f.to_string()),
        ("p", id) => ModParam::Plugin(id.parse().ok()?),
        _ => return None,
    };
    let low: f32 = w.next()?.parse().ok()?;
    let high: f32 = w.next()?.parse().ok()?;
    let curve: u8 = w.next()?.parse().ok()?;
    let range: u8 = w.next()?.parse().ok()?;
    let module = w.next().unwrap_or("").to_string();
    (low.is_finite() && high.is_finite()).then_some(ModRoute {
        source,
        slot,
        module,
        param,
        low: low.clamp(0.0, 1.0),
        high: high.clamp(0.0, 1.0),
        curve: curve.min(100),
        range: range.max(1),
    })
}

/// A route to a MIDI CC: an LFO (`source`) sends CC `cc` (0–[`CC_MAX`]) on `channel` (1–16) to the
/// output port `port`, at 127 × its travel from `low` to `high` along `curve` (as
/// [`ModRoute::travel`]). Project-wide, not on a sound: kept on the project's root
/// object as [`CC_FIELD`].
#[derive(Debug, Clone, PartialEq)]
pub struct CcRoute {
    pub source: ModSource,
    pub port: String,
    pub channel: u8,
    pub cc: u8,
    pub low: f32,
    pub high: f32,
    pub curve: u8,
}

/// The highest CC a route sends to: 120–127 are channel mode messages (All Sound Off,
/// Reset All Controllers, Local Control, All Notes Off, Omni and Mono/Poly), not
/// parameters.
pub const CC_MAX: u8 = 119;

/// The project root's field holding its CC routes ([`cc_to_text`]).
pub const CC_FIELD: &str = "deckard:cc_routes";

impl CcRoute {
    /// A route at Depth 0 (low = high = 0: it sends nothing until turned up), straight.
    pub fn new(source: ModSource, port: &str, channel: u8, cc: u8) -> CcRoute {
        CcRoute { source, port: port.to_string(), channel: channel.clamp(1, 16), cc: cc.min(CC_MAX), low: 0.0, high: 0.0, curve: 50 }
    }

    /// Where its travel (0..1) is with the LFO at `x` (0..1).
    pub fn travel(&self, x: f32) -> f32 {
        let curve = f64::from(self.curve.min(100)) / 100.0;
        let (lo, hi) = (self.low.clamp(0.0, 1.0), self.high.clamp(0.0, 1.0));
        lo + (hi - lo) * pad_touch::shape(f64::from(x.clamp(0.0, 1.0)), curve) as f32
    }

    /// The CC value it sends with the LFO at `x`: 127 × its travel, rounded.
    pub fn value(&self, x: f32) -> u8 {
        (127.0 * self.travel(x)).round().clamp(0.0, 127.0) as u8
    }
}

/// CC routes as text: one per line, `source channel cc low high curve port`, the port
/// last (it may hold spaces).
pub fn cc_to_text(routes: &[CcRoute]) -> String {
    let mut s = String::new();
    for r in routes {
        s.push_str(&format!("{} {} {} {} {} {} {}\n", r.source.key(), r.channel, r.cc, r.low, r.high, r.curve, r.port));
    }
    s
}

/// CC routes from [`cc_to_text`]'s text; lines that don't read (or whose source isn't
/// an LFO) are skipped, values out of range clamped.
pub fn cc_from_text(s: &str) -> Vec<CcRoute> {
    s.lines().filter_map(cc_line).collect()
}

fn cc_line(l: &str) -> Option<CcRoute> {
    let mut w = l.splitn(7, ' ');
    let source = ModSource::from_key(w.next()?).filter(|s| matches!(s, ModSource::Lfo(_)))?;
    // (Numbers read wide, so one out of range is clamped rather than the line lost.)
    let channel: i64 = w.next()?.parse().ok()?;
    let cc: i64 = w.next()?.parse().ok()?;
    let low: f32 = w.next()?.parse().ok()?;
    let high: f32 = w.next()?.parse().ok()?;
    let curve: i64 = w.next()?.parse().ok()?;
    let port = w.next().unwrap_or("").to_string();
    (low.is_finite() && high.is_finite()).then_some(CcRoute {
        source,
        port,
        channel: channel.clamp(1, 16) as u8,
        cc: cc.clamp(0, i64::from(CC_MAX)) as u8,
        low: low.clamp(0.0, 1.0),
        high: high.clamp(0.0, 1.0),
        curve: curve.clamp(0, 100) as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc_routes_round_trip_as_text() {
        let a = CcRoute { low: 0.25, high: 0.75, curve: 70, ..CcRoute::new(ModSource::Lfo(2), "IAC Driver Bus 1", 10, 74) };
        let b = CcRoute::new(ModSource::Lfo(16), "Out", 1, 0);
        let routes = vec![a, b];
        assert_eq!(cc_from_text(&cc_to_text(&routes)), routes);
    }

    #[test]
    fn cc_bad_lines_are_skipped_and_values_clamped() {
        let text = "nonsense\npressure 1 1 0 1 50 Out\nlfo1 x 1 0 1 50 Out\nlfo1 0 128 -1 2 200 Out A\nlfo2 17 3 0 0.5 50 B\n";
        let r = cc_from_text(text);
        assert_eq!(r.len(), 2, "{r:?}");
        assert_eq!((r[0].channel, r[0].cc, r[0].low, r[0].high, r[0].curve), (1, 119, 0.0, 1.0, 100));
        assert_eq!(r[0].port, "Out A");
        assert_eq!((r[1].channel, r[1].cc), (16, 3));
    }

    #[test]
    fn cc_numbers_out_of_byte_range_are_clamped() {
        let r = cc_from_text("lfo1 300 -1 0 1 300 Out\nlfo1 -5 999 0 1 -2 Out\n");
        assert_eq!(r.len(), 2, "{r:?}");
        assert_eq!((r[0].channel, r[0].cc, r[0].curve), (16, 0, 100));
        assert_eq!((r[1].channel, r[1].cc, r[1].curve), (1, 119, 0));
        let r = cc_from_text("lfo1 1 127 0 1 50 Out\nlfo1 1 120 0 1 50 Out\nlfo1 1 119 0 1 50 Out\n");
        assert!(r.iter().all(|r| r.cc == 119), "120-127 are channel mode: {r:?}");
        assert_eq!(CcRoute::new(ModSource::Lfo(1), "Out", 1, 127).cc, 119);
    }

    #[test]
    fn a_cc_route_sends_127_times_its_travel() {
        let r = CcRoute { low: 0.0, high: 1.0, ..CcRoute::new(ModSource::Lfo(1), "Out", 1, 1) };
        assert_eq!(r.value(0.0), 0);
        assert_eq!(r.value(1.0), 127);
        assert_eq!(r.value(0.5), 64);
        assert_eq!(CcRoute::new(ModSource::Lfo(1), "Out", 1, 1).value(1.0), 0, "a new route sits at Depth 0");
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn routes_round_trip_as_text() {
        let a = ModRoute {
            low: 0.25,
            high: 0.75,
            curve: 70,
            ..ModRoute::new(ModSource::Pressure, 0, "Sampler", ModParam::Field("cutoff".into()))
        };
        let b = ModRoute { range: 12, ..ModRoute::new(ModSource::Pitch, 2, "Native Instruments:VST3:FX:1 2", ModParam::Plugin(4096)) };
        let routes = vec![a, b];
        assert_eq!(from_text(&to_text(&routes)), routes);
    }

    #[test]
    fn lfo_routes_round_trip() {
        let r = ModRoute::new(ModSource::Lfo(3), 1, "EQ", ModParam::Field("freq".into()));
        assert_eq!(from_text(&to_text(std::slice::from_ref(&r))), vec![r]);
        assert!(from_text("lfo17 0 f:cutoff 0 1 50 2 S\nlfo0 0 f:cutoff 0 1 50 2 S\n").is_empty());
    }

    #[test]
    fn choices_list_existing_lfos() {
        let c = choices(&[Lfo::new(2), Lfo::new(7)]);
        assert_eq!(c.len(), 5);
        assert_eq!(c[3..], [ModSource::Lfo(2), ModSource::Lfo(7)]);
        assert_eq!(c[..3], ModSource::ALL);
    }

    #[test]
    fn bad_lines_are_skipped() {
        let good = to_text(&[ModRoute::new(ModSource::Slide, 1, "Filter", ModParam::Field("cutoff".into()))]);
        let text = format!("nonsense\npressure x f:cutoff 0 1 50 2 Sampler\n{good}pressure 0 q:1 0 1 50 2 S\n");
        assert_eq!(from_text(&text).len(), 1);
    }

    #[test]
    fn pressure_travels_from_low_to_high() {
        let r = ModRoute { low: 0.2, high: 0.8, ..ModRoute::new(ModSource::Pressure, 0, "Sampler", ModParam::Field("cutoff".into())) };
        assert!(close(r.travel(Some(0.0)), 0.2));
        assert!(close(r.travel(Some(1.0)), 0.8));
        assert!(close(r.travel(Some(0.5)), 0.5));
        let inverted = ModRoute { low: 0.9, high: 0.1, ..r };
        assert!(close(inverted.travel(Some(1.0)), 0.1));
        assert!(close(inverted.travel(Some(0.25)), 0.7));
    }

    #[test]
    fn no_note_rests_at_low() {
        let r = ModRoute { low: 0.3, ..ModRoute::new(ModSource::Slide, 0, "Sampler", ModParam::Field("cutoff".into())) };
        assert!(close(r.travel(None), 0.3));
    }

    #[test]
    fn pitch_rests_at_centre_and_spans_the_range() {
        let r = ModRoute::new(ModSource::Pitch, 0, "Sampler", ModParam::Field("tune".into()));
        assert!(close(r.travel(None), 0.5));
        assert!(close(r.travel(Some(0.0)), 0.5));
        assert!(close(r.travel(Some(2.0)), 1.0));
        assert!(close(r.travel(Some(1.0)), 0.75));
        assert!(close(r.travel(Some(-4.0)), 0.0), "clamped to the range");
    }

    #[test]
    fn the_curve_shapes_the_source() {
        let r = ModRoute { curve: 100, ..ModRoute::new(ModSource::Pressure, 0, "Sampler", ModParam::Field("cutoff".into())) };
        assert!(r.travel(Some(0.25)) > 0.25 + 0.1, "light presses count for more");
        let soft = ModRoute { curve: 0, ..r };
        assert!(soft.travel(Some(0.25)) < 0.25 - 0.1);
    }
}
