//! MIDI in and out of the channels (docs/channel-manual.md §2.2, §2.5, §3.1;
//! docs/channel-software.md §3.3, §4): which sounds an incoming note plays, and what
//! a sound sends.

use deckard_model::{MidiDest, Project};
use std::sync::Arc;

/// A message for a MIDI output port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiOut {
    pub port: Arc<str>,
    pub bytes: [u8; 3],
    pub len: u8,
    /// When to send it (MIDI clock, timed to be heard with the audio); `None`: now.
    pub at: Option<std::time::Instant>,
}

impl MidiOut {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

/// A port DECKARD sends MPE to (lower zone: manager channel 1, member channels 2–16):
/// its member channels' pitch-bend range, and which channels notes have.
#[derive(Debug, Clone)]
pub struct MpePort {
    /// ± semitones.
    pub range: u8,
    busy: [bool; 15],
    /// When each channel was last let go (`time`).
    free_since: [u64; 15],
    /// Member channels used since the last reset (bit i: channel nibble i + 1).
    pub used: u16,
}

impl MpePort {
    pub fn new(range: u8) -> MpePort {
        MpePort { range: range.max(1), busy: [false; 15], free_since: [0; 15], used: 0 }
    }

    /// A member channel for a new note (its status nibble, 1–15): the one free longest;
    /// none when all 15 have notes.
    pub fn take(&mut self, _now: u64) -> Option<u8> {
        let i = (0..15).filter(|&i| !self.busy[i]).min_by_key(|&i| self.free_since[i])?;
        self.busy[i] = true;
        self.used |= 1 << i;
        Some(i as u8 + 1)
    }

    /// Member channel `ch` (nibble 1–15) is free again.
    pub fn free(&mut self, ch: u8, now: u64) {
        if let Some(i) = (ch as usize).checked_sub(1).filter(|&i| i < 15) {
            self.busy[i] = false;
            self.free_since[i] = now;
        }
    }
}

/// What sets an MPE synth up for DECKARD (control changes, 3 bytes each): an MPE
/// Configuration Message on channel 1 (RPN 6: a lower zone of 15 member channels), the
/// manager channel's pitch-bend range ±2 and each member channel's ± `range` (RPN 0),
/// each channel's RPN closed after (the null RPN).
pub fn mpe_setup(range: u8) -> Vec<[u8; 3]> {
    let rpn = |st: u8, n: u8, value: u8| [[st, 0x65, 0], [st, 0x64, n], [st, 0x06, value]];
    let close = |st: u8| [[st, 0x65, 0x7f], [st, 0x64, 0x7f]];
    let mut m = rpn(0xb0, 6, 15).to_vec();
    m.extend(rpn(0xb0, 0, 2));
    m.push([0xb0, 0x26, 0]);
    m.extend(close(0xb0));
    for st in 0xb1..=0xbf {
        m.extend(rpn(st, 0, range.clamp(1, 96)));
        m.push([st, 0x26, 0]);
        m.extend(close(st));
    }
    m
}

/// A per-note bend of `semitones` as a 14-bit pitch bend at ± `range` semitones
/// (8192 centre), clipped.
pub fn bend_14(semitones: f32, range: u8) -> u16 {
    (8192.0 + semitones / f32::from(range.max(1)) * 8192.0).round().clamp(0.0, 16383.0) as u16
}

/// Where a sound's events go besides its instrument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutRoute {
    /// A port, on channel 0–15, notes moved by `transpose` semitones.
    Port { port: Arc<str>, channel: u8, transpose: i64 },
    /// Another sound of the same group (a multitimbral plug-in).
    Sound(usize),
}

/// Each sound's MIDI output, `[group][sound]`.
pub fn out_routes(p: &Project) -> Vec<Vec<Option<OutRoute>>> {
    p.groups
        .iter()
        .map(|g| {
            g.sounds
                .iter()
                .enumerate()
                .map(|(si, s)| match &s.channel.midi_out {
                    MidiDest::None => None,
                    MidiDest::Port(name) => Some(OutRoute::Port {
                        port: name.as_str().into(),
                        channel: s.channel.midi_out_channel.clamp(0, 15) as u8,
                        transpose: s.channel.transpose,
                    }),
                    MidiDest::Sound(t) if *t != si && *t < g.sounds.len() => Some(OutRoute::Sound(*t)),
                    MidiDest::Sound(_) => None,
                })
                .collect()
        })
        .collect()
}

/// A note number moved by `transpose`, if it stays in range (else as it was, as
/// Maschine does).
pub fn transposed(note: i64, transpose: i64) -> u8 {
    let t = note + transpose;
    if (0..128).contains(&t) { t as u8 } else { note.clamp(0, 127) as u8 }
}

#[derive(Debug, Clone, Default)]
struct SoundIn {
    source: String,
    /// 0 = all, else 1–16.
    channel: u8,
    thru: bool,
}

#[derive(Debug, Clone, Default)]
struct GroupIn {
    /// 0 Off, 1 Drumkit, 2 Manual.
    key_mode: i64,
    source: String,
    channel: u8,
    start: i64,
    thru: bool,
    sounds: usize,
}

/// Which channels incoming MIDI reaches: built from the project, with the focused
/// sound (which "Focus" sounds follow).
#[derive(Debug, Clone, Default)]
pub struct MidiTable {
    groups: Vec<GroupIn>,
    sounds: Vec<Vec<SoundIn>>,
    pub focus: (usize, usize),
    /// What a sound set to "Default" does (Preferences ▸ Default ▸ Sound): follow the
    /// focus (`true`) or take nothing.
    pub default_focus: bool,
    /// Controllers' drum channels (Preferences ▸ MIDI): a port whose name has the text
    /// and the channel (1–16). Its notes from C1 play the focused group's pads.
    pub drums: Vec<(String, u8)>,
}

/// A note an incoming message plays: (group, sound, pitch).
pub type Hit = (usize, usize, i64);

/// The note Drumkit mode gives sound 1 (C2; the next sounds go up chromatically).
pub const DRUMKIT_START: i64 = 48;

/// The note a controller's drum channel gives pad 1 (C1; the next pads go up
/// chromatically, four by four from the bottom left as the pads are).
pub const PAD_START: i64 = 36;

impl MidiTable {
    pub fn new(p: &Project) -> MidiTable {
        MidiTable {
            groups: p
                .groups
                .iter()
                .map(|g| GroupIn {
                    key_mode: g.channel.key_mode,
                    source: g.channel.midi_source.clone(),
                    channel: g.channel.midi_channel.clamp(0, 16) as u8,
                    start: g.channel.start_note,
                    thru: g.channel.midi_thru,
                    sounds: g.sounds.len(),
                })
                .collect(),
            sounds: p
                .groups
                .iter()
                .map(|g| {
                    g.sounds
                        .iter()
                        .map(|s| SoundIn {
                            source: s.channel.midi_source.clone(),
                            channel: s.channel.midi_channel.clamp(0, 16) as u8,
                            thru: s.channel.midi_thru,
                        })
                        .collect()
                })
                .collect(),
            focus: (0, 0),
            default_focus: true,
            drums: Vec::new(),
        }
    }

    /// Whether a source setting takes messages from `port` (`focused`: the channel has
    /// the focus).
    fn takes(&self, source: &str, port: &str, focused: bool) -> bool {
        match source {
            "None" | "" => false,
            "All" | "Host" => true,
            "Focus" => focused,
            "Default" => self.default_focus && focused,
            name => name == port,
        }
    }

    fn channel_ok(want: u8, ch: u8) -> bool {
        want == 0 || want == ch
    }

    /// The notes an incoming note `note` on MIDI channel `ch` (1–16) from `port`
    /// plays. Group input (Manual: from its start note; Drumkit: from C2) plays each
    /// sound at its root note; sound input plays the note as it is. When neither the
    /// focused sound nor its group takes MIDI, the focused sound takes everything.
    pub fn route_note(&self, port: &str, ch: u8, note: i64) -> Vec<Hit> {
        // A controller's drum channel: the focused group's pads from C1, as the MK3's.
        if self.drums.iter().any(|(name, c)| *c == ch && port.contains(name.as_str())) {
            let (g, k) = (self.focus.0, note - PAD_START);
            return match self.groups.get(g) {
                Some(gi) if (0..16).contains(&k) && (k as usize) < gi.sounds => vec![(g, k as usize, 60)],
                _ => Vec::new(),
            };
        }
        let mut out: Vec<Hit> = Vec::new();
        for (g, gi) in self.groups.iter().enumerate() {
            let start = match gi.key_mode {
                1 => Some(DRUMKIT_START),
                2 if self.takes(&gi.source, port, false) && Self::channel_ok(gi.channel, ch) => Some(gi.start),
                _ => None,
            };
            if let Some(start) = start {
                let k = note - start;
                if (0..16).contains(&k) && (k as usize) < gi.sounds {
                    out.push((g, k as usize, 60));
                }
            }
        }
        for (g, row) in self.sounds.iter().enumerate() {
            for (s, si) in row.iter().enumerate() {
                if self.takes(&si.source, port, self.focus == (g, s)) && Self::channel_ok(si.channel, ch) && !out.contains(&(g, s, note)) {
                    out.push((g, s, note));
                }
            }
        }
        let (fg, fs) = self.focus;
        let quiet = |src: &str| matches!(src, "None" | "") || (src == "Default" && !self.default_focus);
        if out.is_empty()
            && self.sounds.get(fg).and_then(|r| r.get(fs)).is_some_and(|si| quiet(&si.source))
            && self.groups.get(fg).is_some_and(|gi| gi.key_mode == 0 || quiet(&gi.source))
        {
            out.push((fg, fs, note));
        }
        out
    }

    /// The sounds that take a controller or pitch-bend message: those a note on the
    /// same port and channel would reach.
    pub fn route_control(&self, port: &str, ch: u8) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (g, gi) in self.groups.iter().enumerate() {
            if gi.key_mode == 2 && self.takes(&gi.source, port, false) && Self::channel_ok(gi.channel, ch) {
                out.extend((0..gi.sounds).map(|s| (g, s)));
            }
        }
        for (g, row) in self.sounds.iter().enumerate() {
            for (s, si) in row.iter().enumerate() {
                if self.takes(&si.source, port, self.focus == (g, s)) && Self::channel_ok(si.channel, ch) && !out.contains(&(g, s)) {
                    out.push((g, s));
                }
            }
        }
        if out.is_empty() && self.sounds.get(self.focus.0).is_some_and(|r| self.focus.1 < r.len()) {
            out.push(self.focus);
        }
        out
    }

    /// Whether sound `s` of group `g` takes `port`'s channel `ch` by its own MIDI input
    /// setting (not through its group's keyboard, not as the focus).
    pub fn own_input(&self, g: usize, s: usize, port: &str, ch: u8) -> bool {
        self.sounds.get(g).and_then(|r| r.get(s)).is_some_and(|si| self.takes(&si.source, port, false) && Self::channel_ok(si.channel, ch))
    }

    /// Whether incoming MIDI that a sound takes also goes on to its MIDI output.
    pub fn thru(&self, g: usize, s: usize) -> bool {
        self.sounds.get(g).and_then(|r| r.get(s)).is_some_and(|x| x.thru) || self.groups.get(g).is_some_and(|x| x.thru)
    }
}

/// An incoming MIDI message, decoded: channel 1–16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InMsg {
    NoteOn {
        ch: u8,
        note: u8,
        velocity: u8,
    },
    /// A note-off and its release velocity (0 for a note-on of velocity 0).
    NoteOff {
        ch: u8,
        note: u8,
        velocity: u8,
    },
    /// Poly aftertouch (0xA0): a key's pressure, 0–127.
    PolyPressure {
        ch: u8,
        note: u8,
        value: u8,
    },
    /// Channel pressure (0xD0), 0–127.
    ChannelPressure {
        ch: u8,
        value: u8,
    },
    Control {
        ch: u8,
        number: u8,
        value: u8,
    },
    PitchBend {
        ch: u8,
        value: i16,
    },
    /// A program change (0–127, as sent).
    Program {
        ch: u8,
        program: u8,
    },
}

impl InMsg {
    pub fn parse(b: &[u8]) -> Option<InMsg> {
        let (&status, rest) = b.split_first()?;
        let ch = (status & 0x0f) + 1;
        match (status & 0xf0, rest) {
            (0x90, [n, 0, ..]) => Some(InMsg::NoteOff { ch, note: *n, velocity: 0 }),
            (0x90, [n, v, ..]) => Some(InMsg::NoteOn { ch, note: *n, velocity: *v }),
            (0x80, [n, v, ..]) => Some(InMsg::NoteOff { ch, note: *n, velocity: *v }),
            (0x80, [n]) => Some(InMsg::NoteOff { ch, note: *n, velocity: 0 }),
            (0xa0, [n, v, ..]) => Some(InMsg::PolyPressure { ch, note: *n, value: *v }),
            (0xd0, [v, ..]) => Some(InMsg::ChannelPressure { ch, value: *v }),
            (0xb0, [c, v, ..]) => Some(InMsg::Control { ch, number: *c, value: *v }),
            (0xe0, [lsb, msb, ..]) => Some(InMsg::PitchBend { ch, value: ((*msb as i16) << 7 | *lsb as i16) - 8192 }),
            (0xc0, [p, ..]) => Some(InMsg::Program { ch, program: *p }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    /// The set-up for an MPE synth: an MPE Configuration Message (RPN 6) for a lower
    /// zone of 15 members on channel 1, the manager's bend range ±2 and each member's
    /// at the port's range (RPN 0), each channel's RPN closed after.
    #[test]
    fn mpe_setup_configures_the_lower_zone() {
        let m = mpe_setup(48);
        assert_eq!(&m[..3], &[[0xb0, 0x65, 0], [0xb0, 0x64, 6], [0xb0, 0x06, 15]]);
        let at = |w: &[[u8; 3]]| m.windows(w.len()).position(|x| x == w);
        assert!(at(&[[0xb0, 0x65, 0], [0xb0, 0x64, 0], [0xb0, 0x06, 2], [0xb0, 0x26, 0]]).is_some(), "manager ±2");
        for st in [0xb1u8, 0xbf] {
            assert!(
                at(&[[st, 0x65, 0], [st, 0x64, 0], [st, 0x06, 48], [st, 0x26, 0], [st, 0x65, 0x7f], [st, 0x64, 0x7f]]).is_some(),
                "{st:x}"
            );
        }
        assert_eq!(&m[m.len() - 2..], &[[0xbf, 0x65, 0x7f], [0xbf, 0x64, 0x7f]]);
        assert!(at(&[[0xb0, 0x65, 0x7f], [0xb0, 0x64, 0x7f]]).is_some(), "the manager's closed");
    }

    /// A port's member channels go free longest first: never two notes on one.
    #[test]
    fn mpe_channels_are_taken_free_longest_first() {
        let mut m = MpePort::new(48);
        assert_eq!((m.take(0), m.take(0), m.take(0)), (Some(1), Some(2), Some(3)));
        m.free(2, 10);
        m.free(1, 20);
        assert_eq!(m.take(30), Some(4), "never used: free longest");
        let mut m = MpePort::new(48);
        for _ in 0..15 {
            m.take(0);
        }
        assert_eq!(m.take(0), None, "all 15 busy");
        m.free(9, 5);
        m.free(3, 7);
        assert_eq!((m.take(8), m.take(8)), (Some(9), Some(3)));
        assert_eq!(m.used, 0x7fff, "every channel used");
    }

    #[test]
    fn bends_scale_by_the_range_and_clip() {
        assert_eq!(bend_14(0.0, 48), 8192);
        assert_eq!(bend_14(12.0, 48), 10240);
        assert_eq!(bend_14(-24.0, 48), 4096);
        assert_eq!(bend_14(60.0, 48), 16383);
        assert_eq!(bend_14(-60.0, 48), 0);
    }

    use super::*;
    use deckard_model::{Group, GroupChannel, Sound, SoundChannel};

    #[test]
    fn program_changes_parse() {
        assert_eq!(InMsg::parse(&[0xc1, 5]), Some(InMsg::Program { ch: 2, program: 5 }));
    }

    fn project() -> Project {
        let sound = |src: &str, ch: i64| Sound {
            channel: SoundChannel { midi_source: src.into(), midi_channel: ch, ..Default::default() },
            ..Default::default()
        };
        Project {
            groups: vec![
                Group {
                    sounds: vec![sound("Default", 0), sound("Keys In", 2), sound("Default", 0)],
                    channel: GroupChannel { key_mode: 0, midi_source: "None".into(), ..Default::default() },
                    ..Default::default()
                },
                Group {
                    sounds: (0..16).map(|_| sound("Default", 0)).collect(),
                    channel: GroupChannel {
                        key_mode: 2,
                        midi_source: "Pads".into(),
                        midi_channel: 10,
                        start_note: 36,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// A controller's drum channel (Preferences ▸ MIDI): its notes from C1 play the
    /// focused group's pads, each sound at its root, and nothing else.
    #[test]
    fn a_drum_channel_plays_the_focused_groups_pads() {
        let mut t = MidiTable::new(&project());
        // (Group 1 has three sounds.)
        t.focus = (0, 0);
        t.drums = vec![("LPProMK3 MIDI".into(), 10)];
        let port = "Launchpad Pro MK3 LPProMK3 MIDI";
        assert_eq!(t.route_note(port, 10, 36), vec![(0, 0, 60)]);
        assert_eq!(t.route_note(port, 10, 38), vec![(0, 2, 60)]);
        // Past the group's sounds, or below C1: nothing.
        assert_eq!(t.route_note(port, 10, 39), vec![]);
        assert_eq!(t.route_note(port, 10, 35), vec![]);
        // Its other channels, and other ports, as before.
        assert_eq!(t.route_note(port, 1, 38), vec![(0, 0, 38)]);
        assert_eq!(t.route_note("Keyboard", 10, 38), vec![(0, 0, 38)]);
    }

    #[test]
    fn notes_reach_the_channels_set_to_take_them() {
        let mut t = MidiTable::new(&project());
        t.focus = (0, 2);
        // "Default" follows the focus: only the focused sound plays a keyboard note.
        assert_eq!(t.route_note("Keyboard", 1, 64), vec![(0, 2, 64)]);
        // A sound set to a port and channel takes only those (and the focus too).
        assert_eq!(t.route_note("Keys In", 2, 50), vec![(0, 1, 50), (0, 2, 50)]);
        assert_eq!(t.route_note("Keys In", 3, 50), vec![(0, 2, 50)]);
        // The group in Manual: notes from its start note play its sounds at their root.
        assert_eq!(t.route_note("Pads", 10, 38), vec![(1, 2, 60), (0, 2, 38)]);
        assert_eq!(t.route_note("Pads", 9, 38), vec![(0, 2, 38)], "wrong channel");
        // Default set to take nothing: no sound plays, so the focused one takes all.
        t.default_focus = false;
        assert_eq!(t.route_note("Keyboard", 1, 64), vec![(0, 2, 64)]);
        assert_eq!(transposed(60, 12), 72);
        assert_eq!(transposed(120, 12), 120, "out of range: unchanged");
        assert_eq!(InMsg::parse(&[0x91, 60, 0]), Some(InMsg::NoteOff { ch: 2, note: 60, velocity: 0 }));
        assert_eq!(InMsg::parse(&[0xe0, 0, 64]), Some(InMsg::PitchBend { ch: 1, value: 0 }));
    }

    /// Poly and channel pressure, and a note-off's release velocity.
    #[test]
    fn parses_pressure_and_release_velocity() {
        assert_eq!(InMsg::parse(&[0xa2, 60, 90]), Some(InMsg::PolyPressure { ch: 3, note: 60, value: 90 }));
        assert_eq!(InMsg::parse(&[0xd0, 40]), Some(InMsg::ChannelPressure { ch: 1, value: 40 }));
        assert_eq!(InMsg::parse(&[0x80, 60, 70]), Some(InMsg::NoteOff { ch: 1, note: 60, velocity: 70 }));
        assert_eq!(InMsg::parse(&[0x90, 60, 0]), Some(InMsg::NoteOff { ch: 1, note: 60, velocity: 0 }));
    }
}
