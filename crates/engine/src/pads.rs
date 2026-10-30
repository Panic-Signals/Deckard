//! Pad mode's Choke and Link groups (MK3 manual: Using Choke groups, Using Link groups),
//! within a group: a note on a Send sound mutes the other sounds in its choke group, and
//! also plays the other sounds in its link group (each at its own base key's offset).
//! Receive sounds are only acted on. Links play but aren't recorded: they join the
//! timeline and live notes, never the patterns.

use crate::NoteEvent;
use deckard_model::{MidiKind, Project};

/// All sound off (MIDI CC 120): how a choke reaches a sound (samplers, drum synths and
/// plug-ins fade out).
pub const CHOKE: MidiKind = MidiKind::Controller { number: 120, value: 0 };

#[derive(Debug, Clone, Copy, Default)]
struct Pad {
    choke: i64,
    choke_send: bool,
    link: i64,
    link_send: bool,
    base_key: i64,
}

/// Each group's sounds' choke and link settings.
#[derive(Debug, Clone, Default)]
pub struct Pads(Vec<Vec<Pad>>);

impl Pads {
    pub fn of(p: &Project) -> Pads {
        Pads(
            p.groups
                .iter()
                .map(|g| {
                    g.sounds
                        .iter()
                        .map(|s| {
                            let c = &s.channel;
                            Pad {
                                choke: c.choke_group,
                                choke_send: c.choke_mode == 0,
                                link: c.link_group,
                                link_send: c.link_mode == 0,
                                base_key: c.base_key,
                            }
                        })
                        .collect()
                })
                .collect(),
        )
    }

    /// A note on `sound`: the sounds it plays (itself and those linked) and the sounds
    /// it chokes, as bit sets (bit `n`: sound `n`; sounds past 32 aren't linked).
    pub fn hit(&self, group: usize, sound: usize) -> (u32, u32) {
        let Some(row) = self.0.get(group) else { return (bit(sound), 0) };
        let Some(me) = row.get(sound) else { return (bit(sound), 0) };
        let mut plays = bit(sound);
        if me.link > 0 && me.link_send {
            for (i, p) in row.iter().enumerate().take(32) {
                if p.link == me.link {
                    plays |= bit(i);
                }
            }
        }
        let mut chokes = 0;
        for i in bits(plays) {
            let p = row[i];
            if p.choke > 0 && p.choke_send {
                for (j, q) in row.iter().enumerate().take(32) {
                    if q.choke == p.choke {
                        chokes |= bit(j);
                    }
                }
            }
        }
        (plays, chokes & !plays)
    }

    /// The pitch a note at `pitch` on `from` plays `to` at: as far from `to`'s base key.
    pub fn pitch(&self, group: usize, from: usize, to: usize, pitch: i64) -> i64 {
        let key = |s: usize| self.0.get(group).and_then(|r| r.get(s)).map_or(60, |p| p.base_key);
        (pitch - key(from) + key(to)).clamp(0, 127)
    }
}

fn bit(n: usize) -> u32 {
    if n < 32 { 1 << n } else { 0 }
}

/// The sounds in a bit set.
pub fn bits(set: u32) -> impl Iterator<Item = usize> {
    (0..32).filter(move |i| set & (1 << i) != 0)
}

/// Add to `events` what their notes link and choke: linked sounds' notes, and an all
/// sound off at each note for the sounds it chokes.
pub(crate) fn expand(p: &Project, events: &mut Vec<NoteEvent>) {
    let pads = Pads::of(p);
    let mut more = Vec::new();
    for e in events.iter().filter(|e| e.midi.is_none() && e.cue.is_none()) {
        let (plays, chokes) = pads.hit(e.group, e.sound);
        for s in bits(plays).filter(|&s| s != e.sound) {
            more.push(NoteEvent { sound: s, pitch: pads.pitch(e.group, e.sound, s, e.pitch), from: Some(e.sound), ..*e });
        }
        for s in bits(chokes) {
            more.push(NoteEvent {
                sound: s,
                pitch: 0,
                velocity: 0,
                length: 0,
                probability: 100,
                midi: Some(CHOKE),
                from: Some(e.sound),
                ..*e
            });
        }
    }
    more.sort_by_key(|e| (e.tick, e.group, e.sound, e.midi.is_none(), e.pitch));
    more.dedup_by(|a, b| a.midi.is_some() && a.midi == b.midi && (a.tick, a.group, a.sound) == (b.tick, b.group, b.sound));
    events.extend(more);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mode, timeline};
    use deckard_model::{Group, Note, PPQ, Pattern, Scene, Sound};

    /// Group 1: hats (closed 0 Send, open 1 Receive, choke group 1), a kick (2) linked
    /// as Send to a sub (3, Receive, base key 48) in link group 2, and a snare (4).
    fn project(notes: Vec<Note>) -> Project {
        let sound = |choke, choke_mode, link, link_mode, base_key| {
            let mut s = Sound::default();
            let c = &mut s.channel;
            (c.choke_group, c.choke_mode, c.link_group, c.link_mode, c.base_key) = (choke, choke_mode, link, link_mode, base_key);
            s
        };
        Project {
            tempo: 120.0,
            numerator: 4,
            denominator: 4,
            groups: vec![Group {
                sounds: vec![
                    sound(1, 0, 0, 0, 60),
                    sound(1, 1, 0, 0, 60),
                    sound(0, 0, 2, 0, 60),
                    sound(0, 0, 2, 1, 48),
                    sound(0, 0, 0, 0, 60),
                ],
                patterns: vec![Pattern { length: 4 * PPQ, notes, ..Default::default() }],
                ..Default::default()
            }],
            scenes: vec![Scene { patterns: vec![(0, 0)], ..Default::default() }],
            ..Default::default()
        }
    }

    fn note(sound: usize, position: i64, pitch: i64) -> Note {
        Note { expr: None, sound, position, length: 240, pitch, velocity: 100, probability: 100 }
    }

    #[test]
    fn a_send_chokes_its_choke_group() {
        let pads = Pads::of(&project(vec![]));
        assert_eq!(pads.hit(0, 0), (0b1, 0b10), "the closed hat chokes the open one");
        assert_eq!(pads.hit(0, 1), (0b10, 0), "a Receive chokes nothing");
        assert_eq!(pads.hit(0, 4), (0b1_0000, 0), "no choke group");
    }

    #[test]
    fn a_send_plays_its_link_group() {
        let pads = Pads::of(&project(vec![]));
        assert_eq!(pads.hit(0, 2), (0b1100, 0), "the kick plays the sub too");
        assert_eq!(pads.hit(0, 3), (0b1000, 0), "the sub alone");
        assert_eq!(pads.pitch(0, 2, 3, 62), 50, "two up from each base key");
    }

    #[test]
    fn the_timeline_plays_links_and_chokes() {
        let p = project(vec![note(0, 0, 60), note(1, PPQ, 60), note(2, 2 * PPQ, 60)]);
        let t = timeline(&p, Mode::Pattern(0, 0));
        let at = |tick: i64| t.events.iter().filter(|e| e.tick == tick).map(|e| (e.sound, e.midi, e.pitch)).collect::<Vec<_>>();
        assert_eq!(at(0), [(1, Some(CHOKE), 0), (0, None, 60)], "the open hat is choked first");
        assert_eq!(at(PPQ), [(1, None, 60)], "the open hat chokes nothing");
        assert_eq!(at(2 * PPQ), [(2, None, 60), (3, None, 48)], "the kick and its sub");
        assert_eq!(p.groups[0].patterns[0].notes.len(), 3, "the pattern keeps its own notes");
    }
}
