//! The New Sampler (working name): DECKARD's own sampler, kept in a project as Maschine's
//! Sampler module with DECKARD's fields beside it, so Maschine opens it as a plain
//! Sampler (docs/superpowers/plans/2026-11-11-sampler-roadmap.md, Architecture).
//!
//! Its settings are a `deckard:new_sampler` field on the sound; each zone's analysis (its
//! tempo, its root note, how it plays) a `deckard:zone` field on the zone, naming the file
//! it describes so a zone given another sample isn't described by the old one's.

/// The sound's field: present, the sound's Sampler is a New Sampler.
pub const FIELD: &str = "deckard:new_sampler";
/// A zone's field: its [`ZoneMeta`].
pub const ZONE_FIELD: &str = "deckard:zone";

/// How Sync and Keys stretch: rendered in the background (the default), or live per voice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StretchEngine {
    #[default]
    Rendered,
    Live,
}

/// Slice Motion: which of the sound's slices a note plays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Motion {
    /// The one on its key.
    #[default]
    Off,
    /// The next in key order, whichever key it is (round to the first).
    Next,
    /// A random one, never the one before.
    Random,
}

impl Motion {
    pub const ALL: [Motion; 3] = [Motion::Off, Motion::Next, Motion::Random];

    pub fn label(self) -> &'static str {
        match self {
            Motion::Off => "Off",
            Motion::Next => "Next",
            Motion::Random => "Random",
        }
    }
}

/// The New Sampler's own settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NewSampler {
    pub stretch: StretchEngine,
    pub motion: Motion,
}

/// How a zone plays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlayMode {
    /// Pitch and speed together (the classic sampler).
    #[default]
    Repitch,
    /// Follows the song's tempo; tune doesn't change the length.
    Sync,
    /// Notes change the pitch and keep the length.
    Keys,
}

/// The grid a Sync zone's notes played live start on (roadmap 1c): a note just before a
/// grid line waits for it, one just after starts at once, that much into the sample.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StartGrid {
    Off,
    #[default]
    Sixteenth,
    Eighth,
    Quarter,
    Bar,
}

impl StartGrid {
    pub const ALL: [StartGrid; 5] = [StartGrid::Off, StartGrid::Sixteenth, StartGrid::Eighth, StartGrid::Quarter, StartGrid::Bar];

    /// The grid in beats (quarter notes): 0 is off.
    pub fn beats(self) -> f64 {
        match self {
            StartGrid::Off => 0.0,
            StartGrid::Sixteenth => 0.25,
            StartGrid::Eighth => 0.5,
            StartGrid::Quarter => 1.0,
            StartGrid::Bar => 4.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StartGrid::Off => "OFF",
            StartGrid::Sixteenth => "1/16",
            StartGrid::Eighth => "1/8",
            StartGrid::Quarter => "1/4",
            StartGrid::Bar => "1 BAR",
        }
    }
}

/// A zone's stem lanes at most (roadmap 2b), in this order.
pub const LANES: usize = 4;
pub const LANE_NAMES: [&str; LANES] = ["Drums", "Vocals", "Bass", "Leads"];

/// One stem of a zone's sample: a file as long as it, at its rate, played with the
/// others in its place when any lane is muted, soloed or off unity.
#[derive(Debug, Clone, PartialEq)]
pub struct StemLane {
    /// The stem's file (an absolute path).
    pub file: String,
    pub mute: bool,
    pub solo: bool,
    /// Linear: 1 is unity.
    pub gain: f64,
}

impl StemLane {
    pub fn new(file: String) -> StemLane {
        StemLane { file, mute: false, solo: false, gain: 1.0 }
    }
}

/// How loud each lane plays (0: silent, or no lane): muted lanes and, when any is
/// soloed, those that aren't, are silent; the others at their gain.
pub fn lane_levels(stems: &[Option<StemLane>; LANES]) -> [f32; LANES] {
    let solo = stems.iter().flatten().any(|l| l.solo);
    std::array::from_fn(|k| match &stems[k] {
        Some(l) if !l.mute && (!solo || l.solo) => l.gain.clamp(0.0, 4.0) as f32,
        _ => 0.0,
    })
}

/// Where a zone's tempo came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BpmSource {
    /// The file's name.
    Name,
    /// Heard, and exact: the sample holds a whole number of bars.
    Loop,
    /// Heard.
    #[default]
    Audio,
    /// Set by hand.
    Manual,
}

/// What DECKARD knows about a zone's sample, and how the zone plays it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ZoneMeta {
    /// The sample's path as the zone saves it: a meta naming another file is stale.
    pub file: String,
    pub mode: PlayMode,
    pub bpm: Option<f64>,
    pub bpm_source: BpmSource,
    /// The sample's length in beats at `bpm`.
    pub beats: Option<f64>,
    /// The frame of the first beat.
    pub grid_offset: u64,
    /// MIDI note (60 is C3).
    pub root: Option<u8>,
    /// How far the sample sits from `root`.
    pub cents: f32,
    /// Set by hand: analysis leaves it alone.
    pub root_locked: bool,
    /// Analysis's best guess: no key stood out (shown with a question mark).
    pub root_guess: bool,
    /// The root is the tonic of the sample's key (it has no single note): whether that
    /// key is minor.
    pub key_minor: Option<bool>,
    /// Warp markers (roadmap 1e): frames of the sample pinned to beats, in order. None:
    /// the sample's beats are evenly spaced at `bpm` from `grid_offset`.
    pub warps: Vec<Warp>,
    /// Sync: the grid notes played live start on.
    pub start: StartGrid,
    /// Its stem lanes ([`LANE_NAMES`]): none, the sample plays as it is.
    pub stems: [Option<StemLane>; LANES],
    /// A stem was taken away: the lanes left are all that plays, even at unity (not the
    /// sample itself, which still has it).
    pub stems_only: bool,
    /// It plays backwards: a slice from its end to its start, a start point (a zone that
    /// plays to the file's end) from there back to the file's beginning.
    pub reverse: bool,
    /// Sync and Keys: how fast it goes against the song (1: in time; 2: twice as fast, 0.5:
    /// half), its pitch unmoved. 0 is 1.
    pub speed: f64,
    /// A one-shot: a note plays it through to its end, however short the note.
    pub once: bool,
    /// Its tail, ms (0: none): a note that reaches its end plays back from there this
    /// long, fading out.
    pub tail: f64,
}

/// The longest tail, ms.
pub const TAIL_MAX: f64 = 2000.0;

impl ZoneMeta {
    /// [`ZoneMeta::tail`] within its limits (0 when unset).
    pub fn tail_ms(&self) -> f64 {
        if self.tail.is_finite() { self.tail.clamp(0.0, TAIL_MAX) } else { 0.0 }
    }

    /// [`ZoneMeta::speed`] as a factor: 1 when unset, within a quarter to four times.
    pub fn speed_factor(&self) -> f64 {
        if self.speed > 0.0 && self.speed.is_finite() { self.speed.clamp(0.25, 4.0) } else { 1.0 }
    }
}

/// What a note does with a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Plays {
    /// It plays for as long as the note is held.
    #[default]
    Held,
    /// A one-shot: it plays through to its end, however short the note.
    Once,
    /// Its loop goes round for as long as the note is held.
    Loop,
}

impl Plays {
    pub const ALL: [Plays; 3] = [Plays::Held, Plays::Once, Plays::Loop];

    pub fn label(self) -> &'static str {
        match self {
            Plays::Held => "Hold",
            Plays::Once => "One shot",
            Plays::Loop => "Loop",
        }
    }

    /// How `zone` plays: its loop when that's on, else a one-shot when its meta says so.
    pub fn of(zone: &crate::Zone) -> Plays {
        if zone.loop_active && zone.loop_range.1 > zone.loop_range.0 {
            Plays::Loop
        } else if zone.meta.as_ref().is_some_and(|m| m.once) {
            Plays::Once
        } else {
            Plays::Held
        }
    }
}

impl ZoneMeta {
    pub fn has_stems(&self) -> bool {
        self.stems.iter().any(Option::is_some)
    }
}

/// A warp marker: frame `frame` of the sample falls on beat `beat` (from the first, 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Warp {
    pub frame: u64,
    pub beat: f64,
}

/// Where a sample's frames fall in beats, and back: through its warp markers (straight
/// between them; past the ends at the nearest segment's speed), else evenly at its tempo.
#[derive(Debug, Clone, PartialEq)]
pub struct BeatMap {
    /// (frame, beat), both strictly rising; at least two.
    points: Vec<(f64, f64)>,
}

impl BeatMap {
    /// A zone's map, for a sample at `rate` Hz: `None` without a tempo.
    pub fn of(meta: &ZoneMeta, rate: u32) -> Option<BeatMap> {
        let bpm = meta.bpm.filter(|b| b.is_finite() && *b > 0.0)?;
        if rate == 0 {
            return None;
        }
        let per = f64::from(rate) * 60.0 / bpm;
        let mut points: Vec<(f64, f64)> = Vec::new();
        for w in &meta.warps {
            let p = (w.frame as f64, w.beat);
            if points.last().is_none_or(|l| p.0 > l.0 && p.1 > l.1) {
                points.push(p);
            }
        }
        match points.len() {
            0 => points = vec![(meta.grid_offset as f64, 0.0), (meta.grid_offset as f64 + per, 1.0)],
            1 => points.push((points[0].0 + per, points[0].1 + 1.0)),
            _ => {}
        }
        Some(BeatMap { points })
    }

    /// The segment holding `x` (by frame, or by beat), the ends' extended.
    fn seg(&self, x: f64, by_beat: bool) -> ((f64, f64), (f64, f64)) {
        let key = |p: &(f64, f64)| if by_beat { p.1 } else { p.0 };
        let n = self.points.len();
        let i = self.points.partition_point(|p| key(p) <= x).clamp(1, n - 1);
        (self.points[i - 1], self.points[i])
    }

    pub fn beat_at(&self, frame: f64) -> f64 {
        let (a, b) = self.seg(frame, false);
        a.1 + (frame - a.0) * (b.1 - a.1) / (b.0 - a.0)
    }

    pub fn frame_at(&self, beat: f64) -> f64 {
        let (a, b) = self.seg(beat, true);
        a.0 + (beat - a.1) * (b.0 - a.0) / (b.1 - a.1)
    }

    /// A hash of the points (renders made with another map aren't this one's).
    pub fn id(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (f, b) in &self.points {
            f.to_bits().hash(&mut h);
            b.to_bits().hash(&mut h);
        }
        h.finish()
    }
}

/// Warp markers at `frames` (transients, in order), each on the beat it falls on now.
pub fn warps_at(meta: &ZoneMeta, rate: u32, frames: &[usize]) -> Vec<Warp> {
    let Some(map) = BeatMap::of(meta, rate) else { return Vec::new() };
    frames.iter().map(|&f| Warp { frame: f as u64, beat: map.beat_at(f as f64) }).collect()
}

/// Audio quantize: each warp marker moved `strength` (0..1) of the way to the nearest
/// multiple of `division` beats (0.25: sixteenths). Markers that would land on or before
/// the one before are dropped, so the map keeps rising.
pub fn quantize(warps: &[Warp], division: f64, strength: f64) -> Vec<Warp> {
    let s = strength.clamp(0.0, 1.0);
    let mut out: Vec<Warp> = Vec::new();
    for w in warps {
        let target = if division > 0.0 { (w.beat / division).round() * division } else { w.beat };
        let beat = w.beat + s * (target - w.beat);
        if out.last().is_none_or(|l| w.frame > l.frame && beat > l.beat + 1e-6) {
            out.push(Warp { frame: w.frame, beat });
        }
    }
    out
}

fn word<T: Copy>(table: &[(T, &'static str)], v: T, eq: impl Fn(T, T) -> bool) -> &'static str {
    table.iter().find(|(t, _)| eq(*t, v)).map_or("", |(_, w)| w)
}

fn from_word<T: Copy>(table: &[(T, &'static str)], w: &str) -> Option<T> {
    table.iter().find(|(_, x)| *x == w).map(|(t, _)| *t)
}

const STRETCH: &[(StretchEngine, &str)] = &[(StretchEngine::Rendered, "rendered"), (StretchEngine::Live, "live")];
const MOTIONS: &[(Motion, &str)] = &[(Motion::Off, "off"), (Motion::Next, "next"), (Motion::Random, "random")];
const MODES: &[(PlayMode, &str)] = &[(PlayMode::Repitch, "repitch"), (PlayMode::Sync, "sync"), (PlayMode::Keys, "keys")];
const STARTS: &[(StartGrid, &str)] =
    &[(StartGrid::Off, "off"), (StartGrid::Sixteenth, "16"), (StartGrid::Eighth, "8"), (StartGrid::Quarter, "4"), (StartGrid::Bar, "bar")];
const SOURCES: &[(BpmSource, &str)] =
    &[(BpmSource::Name, "name"), (BpmSource::Loop, "loop"), (BpmSource::Audio, "audio"), (BpmSource::Manual, "manual")];

/// `key value` lines; unknown keys and bad values are skipped.
fn lines(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|l| l.split_once(' '))
}

impl NewSampler {
    pub fn to_text(&self) -> String {
        let mut t = format!("v 1\nstretch {}", word(STRETCH, self.stretch, |a, b| a == b));
        if self.motion != Motion::Off {
            t += &format!("\nmotion {}", word(MOTIONS, self.motion, |a, b| a == b));
        }
        t
    }

    pub fn from_text(text: &str) -> NewSampler {
        let mut n = NewSampler::default();
        for (k, v) in lines(text) {
            match k {
                "stretch" => n.stretch = from_word(STRETCH, v).unwrap_or(n.stretch),
                "motion" => n.motion = from_word(MOTIONS, v).unwrap_or_default(),
                _ => {}
            }
        }
        n
    }
}

impl ZoneMeta {
    /// The meta as text; the file comes last, so its path may hold anything but a line
    /// break.
    pub fn to_text(&self) -> String {
        let mut t = format!(
            "v 1\nmode {}\nstart {}\nbpm_source {}\ngrid_offset {}\ncents {}\nroot_locked {}\n",
            word(MODES, self.mode, |a, b| a == b),
            word(STARTS, self.start, |a, b| a == b),
            word(SOURCES, self.bpm_source, |a, b| a == b),
            self.grid_offset,
            self.cents,
            u8::from(self.root_locked),
        );
        if let Some(b) = self.bpm {
            t += &format!("bpm {b}\n");
        }
        if let Some(b) = self.beats {
            t += &format!("beats {b}\n");
        }
        if let Some(r) = self.root {
            t += &format!("root {r}\n");
        }
        if self.root_guess {
            t += "root_guess 1\n";
        }
        if let Some(minor) = self.key_minor {
            t += if minor { "key minor\n" } else { "key major\n" };
        }
        for w in &self.warps {
            t += &format!("warp {} {}\n", w.frame, w.beat);
        }
        for (k, l) in self.stems.iter().enumerate() {
            if let Some(l) = l {
                t += &format!("stem {k} {}{} {} {}\n", u8::from(l.mute), u8::from(l.solo), l.gain, l.file);
            }
        }
        if self.stems_only {
            t += "stems_only 1\n";
        }
        if self.reverse {
            t += "reverse 1\n";
        }
        if self.speed_factor() != 1.0 {
            t += &format!("speed {}\n", self.speed_factor());
        }
        if self.once {
            t += "once 1\n";
        }
        if self.tail_ms() > 0.0 {
            t += &format!("tail {}\n", self.tail_ms());
        }
        t + "file " + &self.file
    }

    /// The meta, or `None` without a file.
    pub fn from_text(text: &str) -> Option<ZoneMeta> {
        let mut m = ZoneMeta::default();
        let mut file = None;
        for (k, v) in lines(text) {
            match k {
                "file" => file = Some(v.to_string()),
                "mode" => m.mode = from_word(MODES, v).unwrap_or_default(),
                "start" => m.start = from_word(STARTS, v).unwrap_or_default(),
                "bpm_source" => m.bpm_source = from_word(SOURCES, v).unwrap_or_default(),
                "grid_offset" => m.grid_offset = v.parse().unwrap_or(0),
                "cents" => m.cents = v.parse().unwrap_or(0.0),
                "root_locked" => m.root_locked = v == "1",
                "root_guess" => m.root_guess = v == "1",
                "key" => m.key_minor = Some(v == "minor"),
                "stems_only" => m.stems_only = v == "1",
                "reverse" => m.reverse = v == "1",
                "once" => m.once = v == "1",
                "tail" => m.tail = v.parse().ok().filter(|t: &f64| t.is_finite()).map_or(0.0, |t| t.clamp(0.0, TAIL_MAX)),
                "speed" => m.speed = v.parse().unwrap_or(0.0),
                "bpm" => m.bpm = v.parse().ok().filter(|b: &f64| b.is_finite() && *b > 0.0),
                "beats" => m.beats = v.parse().ok().filter(|b: &f64| b.is_finite() && *b > 0.0),
                "root" => m.root = v.parse().ok().filter(|r| *r < 128),
                "stem" => {
                    let mut it = v.splitn(4, ' ');
                    if let (Some(k), Some(flags), Some(gain), Some(file)) = (it.next(), it.next(), it.next(), it.next())
                        && let (Ok(k), Ok(gain)) = (k.parse::<usize>(), gain.parse::<f64>())
                        && k < LANES
                        && gain.is_finite()
                        && !file.is_empty()
                    {
                        let b = flags.as_bytes();
                        m.stems[k] =
                            Some(StemLane { file: file.to_string(), mute: b.first() == Some(&b'1'), solo: b.get(1) == Some(&b'1'), gain });
                    }
                }
                "warp" => {
                    if let Some((f, b)) = v.split_once(' ')
                        && let (Ok(frame), Ok(beat)) = (f.parse(), b.parse::<f64>())
                        && beat.is_finite()
                    {
                        m.warps.push(Warp { frame, beat });
                    }
                }
                _ => {}
            }
        }
        m.file = file?;
        Some(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_as_text() {
        let live = NewSampler { stretch: StretchEngine::Live, ..Default::default() };
        for n in [
            NewSampler::default(),
            live,
            NewSampler { motion: Motion::Next, ..live },
            NewSampler { motion: Motion::Random, ..Default::default() },
        ] {
            assert_eq!(NewSampler::from_text(&n.to_text()), n);
        }
        assert_eq!(NewSampler::default().to_text(), "v 1\nstretch rendered", "nothing new said when Motion is off");
        assert_eq!(NewSampler::from_text("stretch live\nmotion sideways"), live, "a motion not known is off");
    }

    #[test]
    fn zone_meta_round_trips() {
        let full = ZoneMeta {
            file: "/Samples/My Loops/Break 94 BPM.wav".into(),
            mode: PlayMode::Sync,
            bpm: Some(94.25),
            bpm_source: BpmSource::Loop,
            beats: Some(16.0),
            grid_offset: 1234,
            root: Some(45),
            cents: -12.5,
            root_locked: true,
            root_guess: true,
            key_minor: Some(true),
            warps: vec![Warp { frame: 0, beat: 0.0 }, Warp { frame: 22_050, beat: 1.0625 }],
            start: StartGrid::Bar,
            stems: [
                Some(StemLane { file: "/Samples/Stems/Break 94 BPM (Drums).wav".into(), mute: true, solo: false, gain: 0.5 }),
                None,
                Some(StemLane { file: "/b.wav".into(), mute: false, solo: true, gain: 1.0 }),
                None,
            ],
            stems_only: true,
            reverse: true,
            speed: 1.5,
            once: true,
            tail: 120.0,
        };
        assert_eq!(lane_levels(&full.stems), [0.0, 0.0, 1.0, 0.0], "the soloed lane only");
        let mut open = full.stems.clone();
        open[2].as_mut().unwrap().solo = false;
        assert_eq!(lane_levels(&open), [0.0, 0.0, 1.0, 0.0], "the muted lane is silent");
        assert_eq!(ZoneMeta::from_text(&full.to_text()), Some(full));
        let bare = ZoneMeta { file: "x.wav".into(), ..ZoneMeta::default() };
        assert_eq!(ZoneMeta::from_text(&bare.to_text()), Some(bare));
    }

    /// Without markers, beats are even at the tempo from the grid offset; with them,
    /// straight between and at the ends' speed beyond.
    #[test]
    fn the_beat_map() {
        let m = ZoneMeta { bpm: Some(120.0), grid_offset: 1000, ..ZoneMeta::default() };
        let even = BeatMap::of(&m, 48_000).unwrap();
        assert_eq!((even.beat_at(1000.0), even.beat_at(25_000.0), even.frame_at(2.0)), (0.0, 1.0, 49_000.0));
        assert!(BeatMap::of(&ZoneMeta::default(), 48_000).is_none(), "no tempo, no map");
        let warped = ZoneMeta {
            warps: vec![Warp { frame: 0, beat: 0.0 }, Warp { frame: 30_000, beat: 1.0 }, Warp { frame: 48_000, beat: 2.0 }],
            ..m
        };
        let w = BeatMap::of(&warped, 48_000).unwrap();
        assert_eq!((w.beat_at(15_000.0), w.beat_at(39_000.0), w.frame_at(3.0)), (0.5, 1.5, 66_000.0));
        assert_eq!(w.frame_at(w.beat_at(12_345.0)).round(), 12_345.0);
        assert_ne!(w.id(), even.id());
    }

    #[test]
    fn quantize_moves_markers_to_the_grid() {
        let w = [
            Warp { frame: 0, beat: 0.02 },
            Warp { frame: 100, beat: 0.27 },
            Warp { frame: 200, beat: 0.26 },
            Warp { frame: 300, beat: 0.6 },
        ];
        let q = quantize(&w, 0.25, 1.0);
        assert_eq!(q.iter().map(|w| w.beat).collect::<Vec<_>>(), [0.0, 0.25, 0.5], "one past the next dropped, the rest snapped");
        let half = quantize(&w[..2], 0.25, 0.5);
        assert!((half[1].beat - 0.26).abs() < 1e-9);
        let m = ZoneMeta { bpm: Some(120.0), ..ZoneMeta::default() };
        assert_eq!(warps_at(&m, 48_000, &[12_000]), [Warp { frame: 12_000, beat: 0.5 }]);
    }

    #[test]
    fn unknown_lines_are_skipped() {
        let m = ZoneMeta::from_text("v 9\nsparkle 3\nnonsense\nbpm -4\nroot 300\nfile a.wav").unwrap();
        assert_eq!(m, ZoneMeta { file: "a.wav".into(), ..ZoneMeta::default() });
        assert_eq!(ZoneMeta::from_text("bpm 120"), None);
        assert_eq!(ZoneMeta::from_text("tail 99999\nfile a.wav").unwrap().tail, TAIL_MAX);
        assert_eq!(ZoneMeta::from_text("tail soon\nfile a.wav").unwrap().tail, 0.0);
        assert!(!ZoneMeta { file: "a.wav".into(), ..ZoneMeta::default() }.to_text().contains("tail"));
        assert_eq!(NewSampler::from_text("stretch warp\nzzz"), NewSampler::default());
    }
}
