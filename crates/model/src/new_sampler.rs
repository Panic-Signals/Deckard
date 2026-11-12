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

/// The New Sampler's own settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NewSampler {
    pub stretch: StretchEngine,
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
}

fn word<T: Copy>(table: &[(T, &'static str)], v: T, eq: impl Fn(T, T) -> bool) -> &'static str {
    table.iter().find(|(t, _)| eq(*t, v)).map_or("", |(_, w)| w)
}

fn from_word<T: Copy>(table: &[(T, &'static str)], w: &str) -> Option<T> {
    table.iter().find(|(_, x)| *x == w).map(|(t, _)| *t)
}

const STRETCH: &[(StretchEngine, &str)] = &[(StretchEngine::Rendered, "rendered"), (StretchEngine::Live, "live")];
const MODES: &[(PlayMode, &str)] = &[(PlayMode::Repitch, "repitch"), (PlayMode::Sync, "sync"), (PlayMode::Keys, "keys")];
const SOURCES: &[(BpmSource, &str)] =
    &[(BpmSource::Name, "name"), (BpmSource::Loop, "loop"), (BpmSource::Audio, "audio"), (BpmSource::Manual, "manual")];

/// `key value` lines; unknown keys and bad values are skipped.
fn lines(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|l| l.split_once(' '))
}

impl NewSampler {
    pub fn to_text(&self) -> String {
        format!("v 1\nstretch {}", word(STRETCH, self.stretch, |a, b| a == b))
    }

    pub fn from_text(text: &str) -> NewSampler {
        let mut n = NewSampler::default();
        for (k, v) in lines(text) {
            if k == "stretch"
                && let Some(s) = from_word(STRETCH, v)
            {
                n.stretch = s;
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
            "v 1\nmode {}\nbpm_source {}\ngrid_offset {}\ncents {}\nroot_locked {}\n",
            word(MODES, self.mode, |a, b| a == b),
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
                "bpm_source" => m.bpm_source = from_word(SOURCES, v).unwrap_or_default(),
                "grid_offset" => m.grid_offset = v.parse().unwrap_or(0),
                "cents" => m.cents = v.parse().unwrap_or(0.0),
                "root_locked" => m.root_locked = v == "1",
                "bpm" => m.bpm = v.parse().ok().filter(|b: &f64| b.is_finite() && *b > 0.0),
                "beats" => m.beats = v.parse().ok().filter(|b: &f64| b.is_finite() && *b > 0.0),
                "root" => m.root = v.parse().ok().filter(|r| *r < 128),
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
        for n in [NewSampler::default(), NewSampler { stretch: StretchEngine::Live }] {
            assert_eq!(NewSampler::from_text(&n.to_text()), n);
        }
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
        };
        assert_eq!(ZoneMeta::from_text(&full.to_text()), Some(full));
        let bare = ZoneMeta { file: "x.wav".into(), ..ZoneMeta::default() };
        assert_eq!(ZoneMeta::from_text(&bare.to_text()), Some(bare));
    }

    #[test]
    fn unknown_lines_are_skipped() {
        let m = ZoneMeta::from_text("v 9\nsparkle 3\nnonsense\nbpm -4\nroot 300\nfile a.wav").unwrap();
        assert_eq!(m, ZoneMeta { file: "a.wav".into(), ..ZoneMeta::default() });
        assert_eq!(ZoneMeta::from_text("bpm 120"), None);
        assert_eq!(NewSampler::from_text("stretch warp\nzzz"), NewSampler::default());
    }
}
