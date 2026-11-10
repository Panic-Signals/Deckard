//! DECKARD's LFOs (docs/superpowers/specs/2026-10-28-lfos-design.md): up to 16 project-wide
//! low-frequency oscillators, numbered by pad, each a mod-matrix source (0..1). DECKARD's
//! own: kept on the project's root object as [`FIELD`] (text) and saved in the DECKARD
//! block (docs/deckard-block.md).

use crate::PPQ;

/// The wave an LFO draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Sine,
    Square,
    Triangle,
    Saw,
    /// One held value per cycle, the same on every playback.
    Random,
    /// Four non-harmonic sines summed, so it never repeats.
    Wander,
}

impl Shape {
    /// In the order of the shape picker and knob (display buttons 1–4 pick the first four,
    /// and button 5 cycles [`Shape::Random`] and [`Shape::Wander`]).
    pub const ALL: [Shape; 6] = [Shape::Sine, Shape::Square, Shape::Triangle, Shape::Saw, Shape::Random, Shape::Wander];

    /// Its name as DECKARD shows it.
    pub fn name(self) -> &'static str {
        match self {
            Shape::Sine => "Sine",
            Shape::Square => "Square",
            Shape::Triangle => "Triangle",
            Shape::Saw => "Saw",
            Shape::Random => "Random",
            Shape::Wander => "Wander",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Shape::Sine => "sine",
            Shape::Square => "square",
            Shape::Triangle => "triangle",
            Shape::Saw => "saw",
            Shape::Random => "random",
            Shape::Wander => "wander",
        }
    }

    fn from_key(k: &str) -> Option<Shape> {
        Shape::ALL.into_iter().find(|s| s.key() == k)
    }
}

/// The synced rates: name and length in quarter notes. Plain, dotted, then triplet for
/// each of 4 bars, 2 bars, 1/1 … 1/32.
pub const RATES: [(&str, f64); 24] = [
    ("4 bars", 16.0),
    ("4 bars.", 24.0),
    ("4 bars T", 16.0 * 2.0 / 3.0),
    ("2 bars", 8.0),
    ("2 bars.", 12.0),
    ("2 bars T", 8.0 * 2.0 / 3.0),
    ("1/1", 4.0),
    ("1/1.", 6.0),
    ("1/1T", 4.0 * 2.0 / 3.0),
    ("1/2", 2.0),
    ("1/2.", 3.0),
    ("1/2T", 2.0 * 2.0 / 3.0),
    ("1/4", 1.0),
    ("1/4.", 1.5),
    ("1/4T", 2.0 / 3.0),
    ("1/8", 0.5),
    ("1/8.", 0.75),
    ("1/8T", 0.5 * 2.0 / 3.0),
    ("1/16", 0.25),
    ("1/16.", 0.375),
    ("1/16T", 0.25 * 2.0 / 3.0),
    ("1/32", 0.125),
    ("1/32.", 0.1875),
    ("1/32T", 0.125 * 2.0 / 3.0),
];

/// The index in [`RATES`] of "1/4".
pub const DEFAULT_RATE: usize = 12;

/// How far the picture scrolls compared with the LFO. Slow enough to watch; the
/// rate still shows, because a faster LFO travels further in the same time.
const PICTURE_SCROLL: f64 = 0.125;

/// One LFO: `pad` (1–16) names it; `rate` indexes [`RATES`] when `sync`, else `hz`
/// (0.01–20); `phase` in degrees; `level` 0..1; `offset` −1..1 shifts the wave up or
/// down; `jitter` 0..1 mixes noise into it; `quantize` steps per cycle (0 off, else
/// 2–32); `trigger` the (group, sound) whose notes restart it; `fade` in beats, after a
/// trigger; `muted` silences it. `env` makes it an envelope (ADSR) its trigger's notes
/// play (docs/superpowers/specs/2026-10-29-lfo-envelope-design.md): `attack`, `decay`,
/// `release` the seconds a full 0↔1 move takes, `sustain` 0..1, `retrigger` each new
/// note restarting the attack. Trigger, level, quantize and mute serve both modes.
#[derive(Debug, Clone, PartialEq)]
pub struct Lfo {
    pub pad: u8,
    pub shape: Shape,
    pub sync: bool,
    pub rate: usize,
    pub hz: f32,
    pub phase: f32,
    pub level: f32,
    /// Added to the wave after level, then clipped to 0..1. 0 leaves the wave where it is.
    pub offset: f32,
    /// How much noise replaces the wave, 0..1. 0 is the clean shape.
    pub jitter: f32,
    pub quantize: u8,
    pub trigger: Option<(usize, usize)>,
    pub fade: f32,
    /// Gives nothing: its routes leave their targets alone.
    pub muted: bool,
    pub env: bool,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub retrigger: bool,
}

/// The shortest and longest envelope time (seconds).
pub const ENV_MIN: f32 = 0.001;
pub const ENV_MAX: f32 = 10.0;

/// An envelope time knob's position (0..1) as seconds, exponential from [`ENV_MIN`] to
/// [`ENV_MAX`].
pub fn env_time(x: f32) -> f32 {
    ENV_MIN * (ENV_MAX / ENV_MIN).powf(x.clamp(0.0, 1.0))
}

/// An ADSR's picture: each of attack, decay and release as wide as its knob (0..1, plus a
/// little), then a fixed sustain stretch, and a tail after the release. The widths as
/// fractions: (attack, decay, sustain, release, total).
fn adsr_widths(attack: f32, decay: f32, release: f32) -> (f64, f64, f64, f64, f64) {
    let w = |k: f32| 0.1 + f64::from(k.clamp(0.0, 1.0));
    let (a, d, r) = (w(attack), w(decay), w(release));
    (a, d, 0.6, r, a + d + 0.6 + r + 0.2)
}

/// `n` samples (0..1) of an ADSR's picture, its times as knob positions (0..1) and
/// `sustain` a level: drawn by its knobs, not to time, as the LFO page's envelope is.
pub fn adsr_picture(n: usize, attack: f32, decay: f32, sustain: f32, release: f32) -> Vec<f32> {
    let (a, d, s, r, total) = adsr_widths(attack, decay, release);
    let sustain = f64::from(sustain.clamp(0.0, 1.0));
    (0..n)
        .map(|i| {
            let x = i as f64 / n.max(1) as f64 * total;
            let v = if x < a {
                x / a
            } else if x < a + d {
                1.0 - (1.0 - sustain) * (x - a) / d
            } else if x < a + d + s {
                sustain
            } else {
                (sustain * (1.0 - (x - a - d - s) / r)).max(0.0)
            };
            v as f32
        })
        .collect()
}

/// Where [`adsr_picture`] shows the note off (0..1 of its width).
pub fn adsr_note_off(attack: f32, decay: f32, release: f32) -> f32 {
    let (a, d, s, _, total) = adsr_widths(attack, decay, release);
    ((a + d + s) / total) as f32
}

/// [`env_time`]'s inverse: the knob position (0..1) of `seconds`.
pub fn env_knob(seconds: f32) -> f32 {
    ((seconds.max(ENV_MIN) / ENV_MIN).ln() / (ENV_MAX / ENV_MIN).ln()).clamp(0.0, 1.0)
}

/// An envelope's run, in seconds: its notes began at `on_at`, rising from `from`; they
/// ended at `off_at` (if they have), the envelope then at `off_level`. The default has
/// ended at 0: silent until a note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvState {
    pub on_at: f64,
    pub from: f32,
    pub off_at: Option<f64>,
    pub off_level: f32,
}

impl Default for EnvState {
    fn default() -> Self {
        EnvState { on_at: 0.0, from: 0.0, off_at: Some(0.0), off_level: 0.0 }
    }
}

/// The project's field holding the LFOs ([`to_text`]).
pub const FIELD: &str = "deckard:lfos";

impl Lfo {
    /// A new LFO: Sine, synced to 1/4, no phase, full level, unquantized, untriggered,
    /// a beat's fade.
    pub fn new(pad: u8) -> Lfo {
        Lfo {
            pad,
            shape: Shape::Sine,
            sync: true,
            rate: DEFAULT_RATE,
            hz: 1.0,
            phase: 0.0,
            level: 1.0,
            offset: 0.0,
            jitter: 0.0,
            quantize: 0,
            trigger: None,
            fade: 1.0,
            muted: false,
            env: false,
            attack: 0.01,
            decay: 0.3,
            sustain: 0.5,
            release: 0.3,
            retrigger: true,
        }
    }

    /// "LFO n", or "ENV n" for an envelope.
    pub fn name(&self) -> String {
        format!("{} {}", if self.env { "ENV" } else { "LFO" }, self.pad)
    }

    /// The envelope's level (0..1) at `now` (seconds), from its run `st`: rising from
    /// `st.from` to 1 over the attack, falling to the sustain over the decay, holding;
    /// after the notes end, falling from `st.off_level` to 0 over the release. Linear.
    pub fn env_at(&self, st: &EnvState, now: f64) -> f32 {
        let slope = |s: f32| 1.0 / f64::from(s.clamp(ENV_MIN, ENV_MAX));
        if let Some(off) = st.off_at.filter(|off| now >= *off) {
            return (f64::from(st.off_level) - (now - off) * slope(self.release)).max(0.0) as f32;
        }
        let t = (now - st.on_at).max(0.0);
        let from = f64::from(st.from.clamp(0.0, 1.0));
        let rise = (1.0 - from) / slope(self.attack);
        if t < rise {
            return (from + t * slope(self.attack)) as f32;
        }
        let sustain = f64::from(self.sustain.clamp(0.0, 1.0));
        (1.0 - (t - rise) * slope(self.decay)).max(sustain) as f32
    }

    /// What an envelope at `raw` (0..1) gives: quantized to `quantize` heights (0 and 1
    /// among them), × level.
    pub fn env_output(&self, raw: f32) -> f32 {
        let mut v = raw.clamp(0.0, 1.0);
        if self.quantize >= 2 {
            let q = f32::from(self.quantize - 1);
            v = (v * q).round() / q;
        }
        v * self.level.clamp(0.0, 1.0)
    }

    /// Where an envelope's picture shows the note off (0..1 of its width); `None` for an LFO.
    pub fn picture_note_off(&self) -> Option<f32> {
        self.env.then(|| adsr_note_off(env_knob(self.attack), env_knob(self.decay), env_knob(self.release)))
    }

    /// The rate's length in quarter notes.
    fn beats(&self) -> f64 {
        RATES[self.rate.min(RATES.len() - 1)].1
    }

    /// Cycles run so far: synced, from song `ticks`; free, from `seconds`.
    pub fn cycles(&self, ticks: f64, seconds: f64) -> f64 {
        if self.sync { ticks / (PPQ as f64 * self.beats()) } else { seconds * f64::from(self.hz) }
    }

    /// The wave (0..1) after `cycles`, with the phase and the quantize applied.
    pub fn wave_at(&self, cycles: f64) -> f32 {
        let mut c = cycles + f64::from(self.phase) / 360.0;
        if self.quantize >= 2 {
            let q = f64::from(self.quantize);
            c = (c * q).floor() / q;
        }
        let f = c.rem_euclid(1.0);
        let w = match self.shape {
            Shape::Sine => 0.5 + 0.5 * (std::f64::consts::TAU * c).sin(),
            Shape::Square => f64::from(f < 0.5),
            Shape::Triangle => 1.0 - (2.0 * f - 1.0).abs(),
            Shape::Saw => f,
            Shape::Random => random(self.pad, c.floor() as i64),
            Shape::Wander => wander(c),
        };
        w as f32
    }

    /// What it gives (0..1): the wave with jitter mixed in, × level, shifted by the
    /// offset and clipped to 0..1, then × fade. `since_trigger` is the beats since its
    /// trigger's last note; a triggered LFO with none yet gives 0.
    pub fn output(&self, cycles: f64, since_trigger: Option<f64>) -> f32 {
        let fade = match (self.trigger, since_trigger) {
            (None, _) => 1.0,
            (Some(_), None) => return 0.0,
            (Some(_), Some(_)) if self.fade <= 0.0 => 1.0,
            (Some(_), Some(s)) => (s / f64::from(self.fade)).clamp(0.0, 1.0),
        };
        self.signal(cycles) * fade as f32
    }

    /// The wave after jitter, level and offset, clipped to 0..1. Jitter 0 and offset 0
    /// leave the wave × level.
    fn signal(&self, cycles: f64) -> f32 {
        let wave = self.wave_at(cycles);
        let jitter = self.jitter.clamp(0.0, 1.0);
        let mixed = if jitter == 0.0 { wave } else { wave * (1.0 - jitter) + noise(self.pad, cycles) as f32 * jitter };
        (mixed * self.level.clamp(0.0, 1.0) + self.offset).clamp(0.0, 1.0)
    }

    /// How many cycles the picture spans. The window is two quarter notes when synced and
    /// two seconds when free, so a faster rate fits more waves and a slower one shows a
    /// shorter stretch of the same wave. The display scrolls that window ([`Self::picture_at`]).
    fn picture_cycles(&self) -> f64 {
        if self.sync { 2.0 / self.beats() } else { 2.0 * f64::from(self.hz).max(0.0) }
    }

    /// Cycles elapsed on the display clock: `seconds` of UI time, at `bpm` when synced.
    ///
    /// The picture moves at an eighth of the LFO, so the two-beat window takes about
    /// eight seconds to cross the panel instead of one. A faster rate still travels
    /// further. The signal itself ([`Self::output`]) is not slowed.
    pub fn display_cycles(&self, seconds: f64, bpm: f64) -> f64 {
        let real = if self.sync { seconds * (bpm.max(1.0) / 60.0) / self.beats() } else { seconds * f64::from(self.hz).max(0.0) };
        real * PICTURE_SCROLL
    }

    /// Kept so a picture can still ask. The time lines are gone: the wave scrolls instead,
    /// so this is empty.
    pub fn picture_marks(&self) -> Vec<f32> {
        Vec::new()
    }

    /// `n` samples of the wave, for drawing, with the window's left edge at cycle 0.
    ///
    /// The fade is a level ramp in beats ([`Self::output`]), not part of the cycle, so
    /// it is not drawn: folding it in made the first cycle a different shape at every
    /// rate. The picture is the wave itself, with level, offset and jitter. A trigger
    /// does not change it. An envelope is drawn as its ADSR, not to time
    /// ([`Self::picture_note_off`]).
    pub fn picture(&self, n: usize) -> Vec<f32> {
        self.picture_at(n, self.picture_cycles())
    }

    /// `n` samples across the fixed window, the right edge at cycle `at` (now). As `at`
    /// grows the wave travels left, so the rate is visible. An envelope ignores `at`.
    pub fn picture_at(&self, n: usize, at: f64) -> Vec<f32> {
        if self.env {
            let (a, d, r) = (env_knob(self.attack), env_knob(self.decay), env_knob(self.release));
            return adsr_picture(n, a, d, self.sustain, r).into_iter().map(|v| self.env_output(v)).collect();
        }
        let span = self.picture_cycles();
        let start = at - span;
        (0..n)
            .map(|i| {
                let c = start + span * i as f64 / n.max(1) as f64;
                self.signal(c)
            })
            .collect()
    }
}

/// Noise for [`Lfo::jitter`]: a new value many times a cycle, steady between them.
/// A different pad from [`random`] so it doesn't copy a Random wave.
fn noise(pad: u8, cycles: f64) -> f64 {
    random(pad.wrapping_add(19), (cycles * 48.0).floor() as i64)
}

/// Four non-harmonic sines summed, scaled to 0..1: the frequency ratios never line up, so
/// the wave drifts without repeating.
fn wander(cycles: f64) -> f64 {
    let t = std::f64::consts::TAU * cycles;
    let s = t.sin() + 0.6 * (t * 1.61803 + 0.5).sin() + 0.4 * (t * 2.31415 + 1.2).sin() + 0.3 * (t * 0.70711 + 2.3).sin();
    0.5 + 0.5 * (s / 2.3)
}

/// A repeatable value (0..1) for a pad's Random LFO in `cycle`.
fn random(pad: u8, cycle: i64) -> f64 {
    let mut z = (u64::from(pad) << 56) ^ (cycle as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// LFOs as text: one per line, `pad shape sync rate hz phase level quantize fade trig`,
/// `sync` 0 or 1 and `trig` `-` or `group:sound`, then `muted` for a muted one, `env`
/// for an envelope, and its settings `a=` `d=` `s=` `r=` (seconds; sustain 0..1) `rt=`
/// (retrigger 0 or 1), `off=` (−1..1) and `jit=` (0..1), kept in either mode (words
/// older readers skip).
pub fn to_text(lfos: &[Lfo]) -> String {
    let mut s = String::new();
    for l in lfos {
        let trig = l.trigger.map_or("-".to_string(), |(g, k)| format!("{g}:{k}"));
        s.push_str(&format!(
            "{} {} {} {} {} {} {} {} {} {trig}{}{} a={} d={} s={} r={} rt={} off={} jit={}\n",
            l.pad,
            l.shape.key(),
            u8::from(l.sync),
            l.rate,
            l.hz,
            l.phase,
            l.level,
            l.quantize,
            l.fade,
            if l.muted { " muted" } else { "" },
            if l.env { " env" } else { "" },
            l.attack,
            l.decay,
            l.sustain,
            l.release,
            u8::from(l.retrigger),
            l.offset,
            l.jitter,
        ));
    }
    s
}

/// LFOs from [`to_text`]'s text, sorted by pad. Lines that don't read are skipped, a
/// pad's repeat too, and values are clamped to their ranges.
pub fn from_text(s: &str) -> Vec<Lfo> {
    let mut out: Vec<Lfo> = Vec::new();
    for l in s.lines().filter_map(line) {
        if !out.iter().any(|o| o.pad == l.pad) {
            out.push(l);
        }
    }
    out.sort_by_key(|l| l.pad);
    out
}

fn line(l: &str) -> Option<Lfo> {
    let mut w = l.split_whitespace();
    let pad: u8 = w.next()?.parse().ok().filter(|p| (1..=16).contains(p))?;
    let shape = Shape::from_key(w.next()?)?;
    let sync = w.next()?.parse::<u8>().ok()? != 0;
    let rate: usize = w.next()?.parse().ok()?;
    let hz: f32 = w.next()?.parse().ok()?;
    let phase: f32 = w.next()?.parse().ok()?;
    let level: f32 = w.next()?.parse().ok()?;
    let quantize: u8 = w.next()?.parse().ok()?;
    let fade: f32 = w.next()?.parse().ok()?;
    let trigger = w.next().and_then(|t| {
        let (g, s) = t.split_once(':')?;
        Some((g.parse().ok()?, s.parse().ok()?))
    });
    let mut l = Lfo::new(pad);
    for word in w {
        let num = |k: &str| word.strip_prefix(k).and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite());
        let time = |v: f32| v.clamp(ENV_MIN, ENV_MAX);
        match word {
            "muted" => l.muted = true,
            "env" => l.env = true,
            _ => {
                if let Some(v) = num("a=") {
                    l.attack = time(v);
                } else if let Some(v) = num("d=") {
                    l.decay = time(v);
                } else if let Some(v) = num("s=") {
                    l.sustain = v.clamp(0.0, 1.0);
                } else if let Some(v) = num("r=") {
                    l.release = time(v);
                } else if let Some(v) = num("rt=") {
                    l.retrigger = v != 0.0;
                } else if let Some(v) = num("off=") {
                    l.offset = v.clamp(-1.0, 1.0);
                } else if let Some(v) = num("jit=") {
                    l.jitter = v.clamp(0.0, 1.0);
                }
            }
        }
    }
    (hz.is_finite() && phase.is_finite() && level.is_finite() && fade.is_finite()).then_some(Lfo {
        pad,
        shape,
        sync,
        rate: rate.min(RATES.len() - 1),
        hz: hz.clamp(0.01, 20.0),
        phase: phase.rem_euclid(360.0),
        level: level.clamp(0.0, 1.0),
        quantize: if quantize < 2 { 0 } else { quantize.min(32) },
        trigger,
        fade: fade.clamp(0.0, 16.0),
        ..l
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn lfos_round_trip_as_text() {
        let b = Lfo {
            shape: Shape::Random,
            sync: false,
            hz: 2.5,
            phase: 90.0,
            level: 0.5,
            quantize: 8,
            trigger: Some((1, 2)),
            fade: 2.0,
            offset: -0.25,
            jitter: 0.4,
            ..Lfo::new(5)
        };
        let v = vec![Lfo::new(1), b];
        assert_eq!(from_text(&to_text(&v)), v);
    }

    #[test]
    fn bad_text_is_skipped_and_clamped() {
        let text = "garbage\n\
            0 sine 1 12 1 0 1 0 1 -\n\
            17 sine 1 12 1 0 1 0 1 -\n\
            3 sine 1 12 1 0 1 1 1 -\n\
            3 saw 1 12 1 0 1 0 1 -\n\
            4 sine 1 12 1 0 1 99 1 -\n\
            5 sine 1 12 0 0 1 0 1 -\n\
            6 sine 1 12 1 0 3 0 1 -\n\
            7 sine 1 12 1 400 1 0 1 -\n\
            8 sine 1 12 1 0 1 0 -1 -\n\
            9 sine 1 99 1 0 1 0 1 x\n";
        let v = from_text(text);
        assert_eq!(v.iter().map(|l| l.pad).collect::<Vec<_>>(), [3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(v[0].quantize, 0);
        assert_eq!(v[0].shape, Shape::Sine, "the first of a duplicate pad wins");
        assert_eq!(v[1].quantize, 32);
        assert_eq!(v[2].hz, 0.01);
        assert_eq!(v[3].level, 1.0);
        assert_eq!(v[4].phase, 40.0);
        assert_eq!(v[5].fade, 0.0);
        assert_eq!(v[6].rate, 23);
        assert_eq!(v[6].trigger, None);
    }

    #[test]
    fn muted_round_trips_and_old_text_reads_unmuted() {
        let v = vec![Lfo { muted: true, ..Lfo::new(2) }, Lfo::new(3)];
        let text = to_text(&v);
        assert_eq!(from_text(&text), v);
        assert!(!text.lines().nth(1).unwrap().contains("muted"), "unmuted says nothing");
        let old = from_text("4 sine 1 12 1 0 1 0 1 -\n");
        assert!(!old[0].muted);
    }

    #[test]
    fn env_text_round_trips_and_old_text_reads_as_an_lfo() {
        let e = Lfo {
            env: true,
            attack: 0.02,
            decay: 1.5,
            sustain: 0.25,
            release: 4.0,
            retrigger: false,
            trigger: Some((0, 3)),
            muted: true,
            ..Lfo::new(2)
        };
        let kept = Lfo { attack: 0.5, ..Lfo::new(3) };
        let v = vec![e, kept];
        let text = to_text(&v);
        assert_eq!(from_text(&text), v);
        let first: Vec<&str> = text.lines().next().unwrap().split_whitespace().collect();
        assert_eq!(first[10], "muted", "muted stays where older readers look for it");
        let old = from_text("4 sine 1 12 1 0 1 0 1 -\n");
        assert!(!old[0].env);
        assert_eq!((old[0].attack, old[0].decay, old[0].sustain, old[0].release, old[0].retrigger), (0.01, 0.3, 0.5, 0.3, true));
        let bad = from_text("5 sine 1 12 1 0 1 0 1 - env a=99 d=x s=2 r=0 rt=7 zz\n");
        assert!(bad[0].env);
        assert_eq!((bad[0].attack, bad[0].decay, bad[0].sustain, bad[0].release), (ENV_MAX, 0.3, 1.0, ENV_MIN));
    }

    #[test]
    fn env_times_are_exponential_1ms_to_10s() {
        assert!(close(env_time(0.0), 0.001));
        assert!(close(env_time(1.0), 10.0));
        assert!((env_time(0.5) - 0.1).abs() < 1e-4);
        assert!(close(env_knob(env_time(0.37)), 0.37));
        assert_eq!(env_knob(100.0), 1.0);
        assert_eq!(env_knob(0.0), 0.0);
    }

    fn adsr() -> Lfo {
        Lfo { env: true, attack: 0.1, decay: 0.1, sustain: 0.5, release: 0.1, ..Lfo::new(1) }
    }

    #[test]
    fn env_attack_decay_sustain_release() {
        let l = adsr();
        let on = EnvState { on_at: 0.0, from: 0.0, off_at: None, off_level: 0.0 };
        for (t, v) in [(0.05, 0.5), (0.1, 1.0), (0.125, 0.75), (0.15, 0.5), (1.0, 0.5)] {
            assert!(close(l.env_at(&on, t), v), "at {t}: {}", l.env_at(&on, t));
        }
        let off = EnvState { off_at: Some(1.0), off_level: 0.5, ..on };
        assert!(close(l.env_at(&off, 1.025), 0.25));
        assert!(close(l.env_at(&off, 1.05), 0.0));
        assert!(close(l.env_at(&off, 9.0), 0.0));
        assert!(close(l.env_at(&EnvState::default(), 3.0), 0.0), "silent before any note");
    }

    #[test]
    fn env_note_off_in_the_attack_releases_from_there() {
        let l = adsr();
        let on = EnvState { on_at: 0.0, from: 0.0, off_at: None, off_level: 0.0 };
        let reached = l.env_at(&on, 0.03);
        assert!(close(reached, 0.3));
        let off = EnvState { off_at: Some(0.03), off_level: reached, ..on };
        assert!(close(l.env_at(&off, 0.03), 0.3), "no jump at the note-off");
        assert!(close(l.env_at(&off, 0.04), 0.2));
        assert!(close(l.env_at(&off, 0.06), 0.0));
    }

    #[test]
    fn env_retrigger_rises_from_its_level() {
        let l = adsr();
        let st = EnvState { on_at: 0.0, from: 0.4, off_at: None, off_level: 0.0 };
        assert!(close(l.env_at(&st, 0.0), 0.4));
        assert!(close(l.env_at(&st, 0.03), 0.7));
        assert!(close(l.env_at(&st, 0.06), 1.0));
        assert!(close(l.env_at(&st, 0.085), 0.75));
    }

    #[test]
    fn env_quantize_steps_the_height() {
        let l = Lfo { quantize: 4, level: 0.8, ..adsr() };
        let on = EnvState { on_at: 0.0, from: 0.0, off_at: None, off_level: 0.0 };
        let mut seen = Vec::new();
        for i in 0..200 {
            let v = l.env_output(l.env_at(&on, f64::from(i) * 0.001)) / 0.8;
            assert!(close(v, (v * 3.0).round() / 3.0), "{v}");
            if !seen.iter().any(|s| close(*s, v)) {
                seen.push(v);
            }
        }
        assert_eq!(seen.len(), 4, "4 steps, 4 heights: {seen:?}");
        assert!(close(l.env_output(0.3), 0.8 / 3.0));
        assert!(close(Lfo { level: 0.5, ..adsr() }.env_output(0.3), 0.15), "off is smooth");
    }

    #[test]
    fn env_name_reads_env() {
        assert_eq!(Lfo::new(3).name(), "LFO 3");
        assert_eq!(Lfo { env: true, ..Lfo::new(3) }.name(), "ENV 3");
    }

    /// A longer attack draws a later peak, as its knob turns.
    #[test]
    fn adsr_picture_follows_its_knobs() {
        let peak = |a: f32| {
            let p = adsr_picture(200, a, 0.3, 0.5, 0.3);
            p.iter().position(|&v| v > 0.99).unwrap()
        };
        assert!(peak(0.0) < peak(0.5) && peak(0.5) < peak(1.0), "{} {} {}", peak(0.0), peak(0.5), peak(1.0));
        let p = adsr_picture(200, 0.2, 0.2, 0.7, 0.2);
        let off = adsr_note_off(0.2, 0.2, 0.2);
        assert!(close(p[(off * 200.0) as usize - 1], 0.7), "sustain until the note off");
        assert!(close(p[199], 0.0));
    }

    #[test]
    fn env_picture_shows_the_adsr_and_its_note_off() {
        let l = adsr();
        let p = l.picture(200);
        let off = l.picture_note_off().unwrap();
        assert!((0.25..1.0).contains(&off));
        assert!(close(p[0], 0.0));
        assert!(p.iter().any(|v| *v > 0.95), "the peak shows");
        assert!(close(p[(off * 200.0) as usize - 1], 0.5), "sustain until the note off");
        assert!(close(p[199], 0.0), "released to 0");
        assert_eq!(Lfo::new(1).picture_note_off(), None);
    }

    #[test]
    fn sine_quarter_cycle_is_top() {
        assert!(close(Lfo::new(1).wave_at(0.25), 1.0));
        assert!(close(Lfo { phase: 90.0, ..Lfo::new(1) }.wave_at(0.0), 1.0));
    }

    #[test]
    fn quantize_holds_steps() {
        let l = Lfo { shape: Shape::Saw, quantize: 4, ..Lfo::new(1) };
        assert!(close(l.wave_at(0.3), 0.25));
        assert!(close(l.wave_at(0.49), 0.25));
    }

    #[test]
    fn square_triangle_saw_values() {
        let at = |s, c| Lfo { shape: s, ..Lfo::new(1) }.wave_at(c);
        assert!(close(at(Shape::Square, 0.25), 1.0));
        assert!(close(at(Shape::Square, 0.75), 0.0));
        assert!(close(at(Shape::Triangle, 0.25), 0.5));
        assert!(close(at(Shape::Triangle, 0.5), 1.0));
        assert!(close(at(Shape::Saw, 0.5), 0.5));
    }

    #[test]
    fn random_is_deterministic_and_held() {
        let l = Lfo { shape: Shape::Random, ..Lfo::new(3) };
        assert_eq!(l.wave_at(0.1), l.wave_at(0.9));
        assert_eq!(l.wave_at(2.2), Lfo { shape: Shape::Random, ..Lfo::new(3) }.wave_at(2.7));
        let v: Vec<f32> = (0..4).map(|c| l.wave_at(f64::from(c) + 0.5)).collect();
        assert!(v.iter().any(|x| x != &v[0]));
        assert!(v.iter().all(|x| (0.0..=1.0).contains(x)));
    }

    #[test]
    fn synced_cycles_follow_ticks() {
        let l = Lfo::new(1);
        assert!((l.cycles(960.0 * 3.0, 0.0) - 3.0).abs() < 1e-9);
        let bars = Lfo { rate: RATES.iter().position(|r| r.0 == "4 bars").unwrap(), ..Lfo::new(1) };
        assert!((bars.cycles(960.0 * 16.0, 0.0) - 1.0).abs() < 1e-9);
        assert_eq!(RATES[DEFAULT_RATE].0, "1/4");
        let free = Lfo { sync: false, hz: 2.0, ..Lfo::new(1) };
        assert!((free.cycles(0.0, 1.5) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn triggered_output_fades_in() {
        let l = Lfo { shape: Shape::Square, trigger: Some((0, 0)), fade: 2.0, ..Lfo::new(1) };
        assert_eq!(l.output(0.1, None), 0.0);
        assert!(close(l.output(0.1, Some(1.0)), 0.5));
        assert!(close(l.output(0.1, Some(4.0)), 1.0));
    }

    #[test]
    fn picture_draws_two_cycles() {
        let p = Lfo::new(1).picture(8);
        assert_eq!(p.len(), 8);
        assert!(close(p[0], 0.5), "a 1/4 sine still starts mid-wave");
        assert!(close(p[1], 1.0), "two cycles across the picture: the top is the second sample");
        // A trigger does not reshape the wave. The rate does change how many waves fit.
        let wave = Lfo { shape: Shape::Square, ..Lfo::new(1) }.picture(8);
        let triggered = Lfo { shape: Shape::Square, trigger: Some((0, 0)), ..Lfo::new(1) }.picture(8);
        let eighth = RATES.iter().position(|r| r.0 == "1/8").unwrap();
        let faster = Lfo { shape: Shape::Square, rate: eighth, ..Lfo::new(1) }.picture(8);
        assert_eq!(triggered, wave);
        assert_ne!(faster, wave, "1/8 draws twice as many waves as 1/4");
        assert!(close(wave[0], 1.0));
        let hz = |h| Lfo { shape: Shape::Square, sync: false, hz: h, ..Lfo::new(1) }.picture(8);
        assert_ne!(hz(1.0), hz(2.0), "Hz changes the frequency too");
    }

    fn at_rate(name: &str) -> Lfo {
        Lfo { rate: RATES.iter().position(|r| r.0 == name).unwrap(), ..Lfo::new(1) }
    }

    #[test]
    fn a_slow_rate_shows_a_short_stretch() {
        let quarter = at_rate("1/4").picture(240);
        let slow = at_rate("4 bars").picture(240);
        assert_ne!(slow, quarter, "two beats of a 4-bar wave is a short stretch, not two cycles");
        assert!(slow.iter().all(|v| (v - 0.5).abs() < 0.45), "that stretch stays near the sine's start");
        let free = |hz| Lfo { sync: false, hz, ..Lfo::new(1) }.picture(8);
        assert_ne!(free(0.01), free(1.0), "a slow free rate shows less of the wave");
    }

    #[test]
    fn the_picture_scrolls_slower_than_the_lfo() {
        let l = Lfo::new(1);
        // One second at 120 BPM is two quarter notes: two real cycles of a 1/4 LFO.
        let shown = l.display_cycles(1.0, 120.0);
        assert!(shown < 0.5, "a second of the picture moves less than half a cycle, got {shown}");
        let eighth = RATES.iter().position(|r| r.0 == "1/8").unwrap();
        let faster = Lfo { rate: eighth, ..Lfo::new(1) }.display_cycles(1.0, 120.0);
        assert!(faster > shown, "a faster rate still travels further");
        let free = Lfo { sync: false, hz: 1.0, ..Lfo::new(1) };
        assert!(free.display_cycles(1.0, 120.0) < 0.5);
    }

    #[test]
    fn the_picture_scrolls_and_has_no_time_lines() {
        let l = Lfo::new(1);
        let still = l.picture(8);
        let later = l.picture_at(8, 2.25);
        assert!(close(later[0], still[1]), "a quarter cycle later, the wave has moved left");
        assert!(l.picture_marks().is_empty());
        assert!(at_rate("4 bars").picture_marks().is_empty());
        assert!(Lfo { sync: false, hz: 0.01, ..Lfo::new(1) }.picture_marks().is_empty());
        assert!(Lfo { env: true, ..Lfo::new(1) }.picture_marks().is_empty());
    }

    #[test]
    fn offset_shifts_and_clips_the_signal() {
        assert!(close(Lfo { offset: 0.25, ..Lfo::new(1) }.output(0.0, None), 0.75));
        assert!(close(Lfo { offset: 1.0, ..Lfo::new(1) }.output(0.0, None), 1.0));
        assert!(close(Lfo { offset: -1.0, ..Lfo::new(1) }.output(0.0, None), 0.0));
        assert!(close(Lfo { offset: 0.25, ..Lfo::new(1) }.picture(8)[0], 0.75));
    }

    #[test]
    fn jitter_mixes_noise_into_the_signal() {
        let clean = Lfo::new(1).output(0.3, None);
        assert!(close(Lfo { jitter: 0.0, ..Lfo::new(1) }.output(0.3, None), clean));
        let rough = Lfo { jitter: 1.0, ..Lfo::new(1) };
        let a = rough.output(0.0, None);
        let b = rough.output(1.0, None);
        assert_ne!(a, b, "full jitter is noise, and it changes along the wave");
        assert!((0.0..=1.0).contains(&a) && (0.0..=1.0).contains(&b));
        let mild = Lfo { jitter: 0.05, ..Lfo::new(1) }.output(0.0, None);
        assert!((mild - 0.5).abs() < 0.05, "a little jitter stays near the sine");
    }

    #[test]
    fn old_lfo_text_without_offset_or_jitter_still_loads() {
        let l = from_text("1 sine 1 12 1 0 1 0 1 - a=0.01 d=0.3 s=0.5 r=0.3 rt=1\n");
        assert_eq!(l, vec![Lfo::new(1)]);
    }

    #[test]
    fn project_reads_lfos_from_the_root() {
        use mxprj::decode::{Obj, Value};
        let root = Obj { fields: vec![(FIELD.into(), Value::Str(to_text(&[Lfo::new(4)])))], ..Default::default() };
        let (p, _) = crate::Project::read(&root);
        assert_eq!(p.lfos[0].pad, 4);
    }

    #[test]
    fn wander_matches_four_sine_formula_and_stays_in_range() {
        let l = Lfo { shape: Shape::Wander, ..Lfo::new(1) };
        for c in [-2.5, 0.0, 0.25, 0.73, 1.0, 7.3, 42.125] {
            let t = std::f64::consts::TAU * c;
            let raw = (t.sin() + 0.6 * (t * 1.61803 + 0.5).sin() + 0.4 * (t * 2.31415 + 1.2).sin() + 0.3 * (t * 0.70711 + 2.3).sin()) / 2.3;
            let expected = (0.5 + 0.5 * raw) as f32;
            assert!(close(l.wave_at(c), expected), "at {c}: {} vs {expected}", l.wave_at(c));
            let cc = (64 + (63.0 * raw).round() as i32).clamp(0, 127);
            let from_wave = (64 + (63.0 * (f64::from(l.wave_at(c)) * 2.0 - 1.0)).round() as i32).clamp(0, 127);
            assert_eq!(from_wave, cc);
        }
        for rate in 0..RATES.len() {
            let synced = Lfo { shape: Shape::Wander, rate, ..Lfo::new(1) };
            for step in 0..500 {
                let c = synced.cycles(f64::from(step) * 197.0, 0.0);
                let w = synced.wave_at(c);
                let out = synced.output(c, None);
                assert!((0.0..=1.0).contains(&w) && (0.0..=1.0).contains(&out), "rate {rate} step {step}: {w} {out}");
            }
        }
        for hz in [0.01, 0.15, 0.5, 2.5, 20.0] {
            let free = Lfo { shape: Shape::Wander, sync: false, hz, ..Lfo::new(1) };
            for step in 0..500 {
                let c = free.cycles(0.0, f64::from(step) * 0.037);
                let w = free.wave_at(c);
                let out = free.output(c, None);
                assert!((0.0..=1.0).contains(&w) && (0.0..=1.0).contains(&out), "hz {hz} step {step}: {w} {out}");
            }
        }
    }

    #[test]
    fn wander_does_not_repeat_over_a_long_window() {
        let l = Lfo { shape: Shape::Wander, ..Lfo::new(1) };
        let window: Vec<f32> = (0..128).map(|i| l.wave_at(f64::from(i) * 0.05)).collect();
        for shift in 1..=500 {
            let offset = f64::from(shift) * 0.5;
            let shifted: Vec<f32> = (0..128).map(|i| l.wave_at(offset + f64::from(i) * 0.05)).collect();
            let max_diff = window.iter().zip(&shifted).map(|(a, b)| (a - b).abs()).fold(0.0_f32, f32::max);
            assert!(max_diff > 1e-2, "shift {offset} repeated the window (max_diff {max_diff})");
        }
        // Unlike Sine, scrolling the picture by one full cycle draws a new stretch of the curve.
        let sine = Lfo::new(1);
        assert_eq!(sine.picture_at(64, 2.0), sine.picture_at(64, 3.0));
        assert_ne!(l.picture_at(64, 2.0), l.picture_at(64, 3.0));
    }

    #[test]
    fn wander_rate_level_offset_phase_quantize_jitter_and_trigger() {
        let base = Lfo { shape: Shape::Wander, ..Lfo::new(1) };
        let eighth = Lfo { rate: RATES.iter().position(|r| r.0 == "1/8").unwrap(), ..base.clone() };
        assert!(close(eighth.cycles(960.0, 0.0) as f32, 2.0 * base.cycles(960.0, 0.0) as f32));
        let shifted = Lfo { phase: 90.0, ..base.clone() };
        assert!(close(shifted.wave_at(0.0), base.wave_at(0.25)));
        let stepped = Lfo { quantize: 4, ..base.clone() };
        assert_eq!(stepped.wave_at(0.26), stepped.wave_at(0.49));
        assert_ne!(stepped.wave_at(0.24), stepped.wave_at(0.26));
        let w = base.wave_at(0.37);
        assert!(close(Lfo { level: 0.4, ..base.clone() }.output(0.37, None), w * 0.4));
        assert!(close(Lfo { level: 0.5, offset: 0.2, ..base.clone() }.output(0.37, None), (w * 0.5 + 0.2).clamp(0.0, 1.0)));
        assert!(close(Lfo { offset: 1.0, ..base.clone() }.output(0.37, None), 1.0));
        assert!(close(Lfo { offset: -1.0, ..base.clone() }.output(0.37, None), 0.0));
        let noisy = Lfo { jitter: 1.0, ..base.clone() };
        assert_ne!(noisy.output(0.0, None), base.output(0.0, None));
        let trig = Lfo { trigger: Some((0, 0)), fade: 2.0, ..base };
        assert_eq!(trig.output(0.37, None), 0.0);
        assert!(close(trig.output(0.37, Some(1.0)), w * 0.5));
        assert!(close(trig.output(0.37, Some(2.0)), w));
    }

    #[test]
    fn wander_round_trips_and_old_projects_still_load() {
        let w = Lfo { shape: Shape::Wander, sync: false, hz: 0.5, phase: 45.0, level: 0.75, offset: 0.1, jitter: 0.05, ..Lfo::new(1) };
        let text = to_text(&[w.clone(), Lfo::new(2)]);
        assert!(text.starts_with("1 wander 0 "), "{text}");
        assert_eq!(from_text(&text), vec![w.clone(), Lfo::new(2)]);
        let old = from_text("1 sine 1 12 1 0 1 0 1 -\n2 random 0 12 2.5 90 0.5 8 2 1:2\n");
        assert_eq!(old.len(), 2);
        assert_eq!((old[0].shape, old[1].shape), (Shape::Sine, Shape::Random));
        use mxprj::decode::{Obj, Value};
        let root = Obj { fields: vec![(FIELD.into(), Value::Str(text))], ..Default::default() };
        let (p, _) = crate::Project::read(&root);
        assert_eq!(p.lfos[0], w);
    }
}
