//! Standard MIDI files for Import MIDI… / Export MIDI…: a sound's or a group's notes
//! and controller events in one pattern, or (File ▸ Export MIDI…) the song's.

use deckard_model::{MidiEvent, MidiKind, Note, PPQ};
use midly::num::{u4, u7, u14, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use std::path::Path;

/// Write `notes` and `midi` (positions from the pattern's start, in ticks) as a type 0
/// file with the tempo and time signature, each note's channel (0–15) and key, and each
/// controller event's channel, given.
#[allow(clippy::too_many_arguments)]
pub fn export_mapped(
    path: &Path,
    notes: &[Note],
    midi: &[MidiEvent],
    tempo: f64,
    numerator: i64,
    denominator: i64,
    note_map: &dyn Fn(&Note) -> (u8, i64),
    midi_channel: &dyn Fn(&MidiEvent) -> u8,
) -> Result<(), String> {
    let mut events = meta(tempo, numerator, denominator);
    events.extend(track_events(notes, midi, note_map, midi_channel));
    let smf = Smf { header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(PPQ as u16))), tracks: vec![track(events)] };
    smf.save(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// One sound's part of a type 1 file: its name, events (positions from the start, in
/// ticks), MIDI channel (0–15) and transpose (semitones).
#[derive(Debug, Clone, Default)]
pub struct Track {
    pub name: String,
    pub notes: Vec<Note>,
    pub midi: Vec<MidiEvent>,
    pub channel: u8,
    pub transpose: i64,
}

/// Write `tracks` as a type 1 file: a tempo track, then each track in order (empty ones
/// too, so sounds keep their tracks across files).
pub fn export_tracks(path: &Path, tracks: &[Track], tempo: f64, numerator: i64, denominator: i64) -> Result<(), String> {
    let mut all = vec![track(meta(tempo, numerator, denominator))];
    for t in tracks {
        let mut events: Vec<Timed<'_>> = vec![(0, 0, TrackEventKind::Meta(MetaMessage::TrackName(t.name.as_bytes())))];
        events.extend(track_events(&t.notes, &t.midi, &|n| (t.channel, n.pitch + t.transpose), &|_| t.channel));
        all.push(track(events));
    }
    let smf = Smf { header: Header::new(Format::Parallel, Timing::Metrical(u15::new(PPQ as u16))), tracks: all };
    smf.save(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// An event at a tick, with its order among events at the same tick (offs before ons).
type Timed<'a> = (i64, u8, TrackEventKind<'a>);

/// The tempo and time signature, at the start.
fn meta(tempo: f64, numerator: i64, denominator: i64) -> Vec<Timed<'static>> {
    let tempo_us = (60_000_000.0 / tempo.max(1.0)).round() as u32;
    let den_pow = (denominator.max(1) as f64).log2().round() as u8;
    vec![
        (0, 0, TrackEventKind::Meta(MetaMessage::Tempo(u24::new(tempo_us.min(0xFF_FFFF))))),
        (0, 0, TrackEventKind::Meta(MetaMessage::TimeSignature(numerator.clamp(1, 255) as u8, den_pow, 24, 8))),
    ]
}

/// Notes and controller events as MIDI events, each note's channel and key from
/// `note_map`, each controller event's channel from `midi_channel`.
fn track_events(
    notes: &[Note],
    midi: &[MidiEvent],
    note_map: &dyn Fn(&Note) -> (u8, i64),
    midi_channel: &dyn Fn(&MidiEvent) -> u8,
) -> Vec<Timed<'static>> {
    let mut events = Vec::new();
    for n in notes {
        let (c, pitch) = note_map(n);
        let ch = u4::new(c.min(15));
        let key = u7::new(pitch.clamp(0, 127) as u8);
        let vel = u7::new(n.velocity.clamp(1, 127) as u8);
        events.push((n.position, 2, TrackEventKind::Midi { channel: ch, message: MidiMessage::NoteOn { key, vel } }));
        events.push((
            n.position + n.length.max(1),
            1,
            TrackEventKind::Midi { channel: ch, message: MidiMessage::NoteOff { key, vel: u7::new(0) } },
        ));
    }
    for m in midi {
        let message = match m.kind {
            MidiKind::Controller { number, value } => {
                MidiMessage::Controller { controller: u7::new(number.min(127)), value: u7::new(value.min(127)) }
            }
            MidiKind::PitchBend(v) => MidiMessage::PitchBend { bend: midly::PitchBend(u14::new(v.min(16_383))) },
            MidiKind::ChannelPressure(v) => MidiMessage::ChannelAftertouch { vel: u7::new(v.min(127)) },
        };
        events.push((m.position, 1, TrackEventKind::Midi { channel: u4::new(midi_channel(m).min(15)), message }));
    }
    events
}

/// Events in time order, as a track (with its end).
fn track(mut events: Vec<Timed<'_>>) -> Vec<TrackEvent<'_>> {
    events.sort_by_key(|e| (e.0.max(0), e.1));
    let mut track = Vec::with_capacity(events.len() + 1);
    let mut last = 0;
    for (tick, _, kind) in events {
        let tick = tick.max(0);
        track.push(TrackEvent { delta: u28::new((tick - last) as u32), kind });
        last = tick;
    }
    track.push(TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) });
    track
}

/// The notes and controller events of a MIDI file (every track and channel), in
/// ticks of [`PPQ`] from its start, for sound `sound`; and the file's length (the
/// last event).
pub fn import(path: &Path, sound: usize) -> Result<(Vec<Note>, Vec<MidiEvent>, i64), String> {
    let (notes, midi, end) = import_channels(path)?;
    let notes = notes.into_iter().map(|(_, n)| Note { sound, ..n }).collect();
    let midi = midi.into_iter().map(|(_, m)| MidiEvent { sound, ..m }).collect();
    Ok((notes, midi, end))
}

/// Notes and controller events with their channel (0–15), and the length.
pub type Channels = (Vec<(u8, Note)>, Vec<(u8, MidiEvent)>, i64);

/// [`import`] keeping each event's channel (0–15); sounds are 0.
pub fn import_channels(path: &Path) -> Result<Channels, String> {
    let sound = 0;
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let smf = Smf::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let Timing::Metrical(ppq) = smf.header.timing else { return Err("timecode-based MIDI files aren't supported".into()) };
    let scale = PPQ as f64 / ppq.as_int().max(1) as f64;
    let mut notes = Vec::new();
    let mut midi = Vec::new();
    let mut end = 0;
    for track in &smf.tracks {
        let mut tick: i64 = 0;
        // Sounding notes: (channel, key) -> (start, velocity).
        let mut on: std::collections::HashMap<(u8, u8), (i64, u8)> = std::collections::HashMap::new();
        for ev in track {
            tick += ev.delta.as_int() as i64;
            let at = (tick as f64 * scale).round() as i64;
            end = end.max(at);
            let TrackEventKind::Midi { channel, message } = ev.kind else { continue };
            let c = channel.as_int();
            let mut off = |key: u8, notes: &mut Vec<(u8, Note)>| {
                if let Some((start, vel)) = on.remove(&(c, key)) {
                    notes.push((
                        c,
                        Note {
                            expr: None,
                            sound,
                            position: start,
                            length: (at - start).max(1),
                            pitch: key as i64,
                            velocity: vel as i64,
                            probability: 100,
                        },
                    ));
                }
            };
            match message {
                MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                    off(key.as_int(), &mut notes);
                    on.insert((c, key.as_int()), (at, vel.as_int()));
                }
                MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => off(key.as_int(), &mut notes),
                MidiMessage::Controller { controller, value } => midi.push((
                    c,
                    MidiEvent { sound, position: at, kind: MidiKind::Controller { number: controller.as_int(), value: value.as_int() } },
                )),
                MidiMessage::PitchBend { bend } => {
                    midi.push((c, MidiEvent { sound, position: at, kind: MidiKind::PitchBend(bend.0.as_int()) }))
                }
                _ => {}
            }
        }
    }
    notes.sort_by_key(|(c, n)| (n.position, n.pitch, *c));
    midi.sort_by_key(|(_, m)| m.position);
    Ok((notes, midi, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_survive_export_and_import() {
        let notes = vec![
            Note { expr: None, sound: 3, position: 0, length: 240, pitch: 60, velocity: 100, probability: 100 },
            Note { expr: None, sound: 3, position: 480, length: 960, pitch: 64, velocity: 90, probability: 100 },
            Note { expr: None, sound: 3, position: 480, length: 120, pitch: 67, velocity: 127, probability: 100 },
        ];
        let midi = vec![
            MidiEvent { sound: 3, position: 0, kind: MidiKind::Controller { number: 64, value: 127 } },
            MidiEvent { sound: 3, position: 960, kind: MidiKind::PitchBend(7192) },
        ];
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notes.mid");
        export_mapped(&path, &notes, &midi, 97.0, 3, 4, &|n| (0, n.pitch), &|_| 0).unwrap();
        let (n, m, end) = import(&path, 3).unwrap();
        assert_eq!(n, notes);
        assert_eq!(m, midi);
        assert_eq!(end, 1440);
    }

    /// A group's (or the song's) file: type 1, a tempo track then a track per sound
    /// in order, named, the empty ones kept; each on its channel, transposed.
    #[test]
    fn tracks_export_as_type_1_with_empty_tracks_kept() {
        let note = |position, pitch| Note { expr: None, sound: 0, position, length: 240, pitch, velocity: 100, probability: 100 };
        let tracks = vec![
            Track { name: "Kick".into(), notes: vec![note(0, 36)], midi: vec![], channel: 9, transpose: 0 },
            Track { name: "Empty".into(), notes: vec![], midi: vec![], channel: 0, transpose: 0 },
            Track { name: "Bass".into(), notes: vec![note(480, 40)], midi: vec![], channel: 2, transpose: 12 },
        ];
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tracks.mid");
        export_tracks(&path, &tracks, 120.0, 4, 4).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let (notes, _, _) = import_channels(&path).unwrap();
        assert_eq!(notes.len(), 2, "imports back");
        let smf = Smf::parse(&bytes).unwrap();
        assert_eq!(smf.header.format, Format::Parallel);
        assert_eq!(smf.tracks.len(), 4);
        let name = |t: &[TrackEvent]| {
            t.iter().find_map(|e| match e.kind {
                TrackEventKind::Meta(MetaMessage::TrackName(n)) => Some(String::from_utf8_lossy(n).into_owned()),
                _ => None,
            })
        };
        let names: Vec<_> = smf.tracks[1..].iter().map(|t| name(t).unwrap()).collect();
        assert_eq!(names, ["Kick", "Empty", "Bass"]);
        let ons = |t: &[TrackEvent]| -> Vec<(u8, u8)> {
            t.iter()
                .filter_map(|e| match e.kind {
                    TrackEventKind::Midi { channel, message: MidiMessage::NoteOn { key, .. } } => Some((channel.as_int(), key.as_int())),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(ons(&smf.tracks[1]), [(9, 36)]);
        assert!(ons(&smf.tracks[2]).is_empty());
        assert_eq!(ons(&smf.tracks[3]), [(2, 52)]);
    }
}
