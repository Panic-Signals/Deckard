//! Note Echo, a MIDI input plug-in: each note plays again a number of times, quieter each
//! time and optionally at another pitch. Each echo goes through the instrument as a note
//! of its own, so its envelopes start again, which an audio delay can't do.
//!
//! An echo of a key still held at the same pitch can end it early on an instrument that
//! goes by pitch (a hosted plug-in, MIDI out): its note-off is for that pitch.

use crate::midi_input::{Clock, MidiInput, NoteIn, Outbox, ParamDesc};
use deckard_model::PPQ;

pub const NAME: &str = "Note Echo";

const TIME: u32 = 1;
const REPEATS: u32 = 2;
const DECAY: u32 = 3;
const PITCH: u32 = 4;

/// The times offered, and their lengths in quarter notes.
const TIMES: [(&str, f64); 8] = [
    ("1/32", 1.0 / 8.0),
    ("1/16 T", 1.0 / 6.0),
    ("1/16", 1.0 / 4.0),
    ("1/8 T", 1.0 / 3.0),
    ("3/16", 3.0 / 8.0),
    ("1/8", 1.0 / 2.0),
    ("1/4 T", 2.0 / 3.0),
    ("1/4", 1.0),
];
const DEFAULT_TIME: usize = 4;
const DEFAULT_REPEATS: u32 = 3;
const DEFAULT_DECAY: f64 = 35.0;
const MAX_REPEATS: u32 = 8;
/// Keys it remembers, to end them if it's taken out while they're down.
const HELD_ROOM: usize = 64;

pub struct NoteEcho {
    /// Between echoes, in ticks.
    time: f64,
    repeats: u32,
    /// How much of the velocity each echo keeps.
    keep: f64,
    /// Semitones from one echo to the next.
    step: i64,
    held: Vec<(usize, i64)>,
}

impl NoteEcho {
    pub fn new() -> NoteEcho {
        NoteEcho {
            time: Self::ticks(DEFAULT_TIME),
            repeats: DEFAULT_REPEATS,
            keep: 1.0 - DEFAULT_DECAY / 100.0,
            step: 0,
            held: Vec::with_capacity(HELD_ROOM),
        }
    }

    fn ticks(time: usize) -> f64 {
        TIMES[time.min(TIMES.len() - 1)].1 * PPQ as f64
    }

    fn echoes(&self, sound: usize, pitch: i64, velocity: i64, length: Option<i64>, clock: Clock, out: &mut Outbox) {
        // As long as the note, up to the time between echoes. A key held has no length yet.
        let time = self.time as i64;
        let gate = length.map_or(time / 2, |l| l.min(time)).max(1);
        let mut level = velocity as f64;
        for n in 1..=self.repeats as i64 {
            level *= self.keep;
            let (v, p) = (level.round() as i64, pitch + n * self.step);
            if v < 1 || !(0..=127).contains(&p) {
                break;
            }
            if !out.on(clock.now + n as f64 * self.time, sound, p, v, Some(gate)) {
                break;
            }
        }
    }
}

impl Default for NoteEcho {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiInput for NoteEcho {
    fn name(&self) -> String {
        NAME.into()
    }

    fn params(&self) -> Vec<ParamDesc> {
        vec![
            ParamDesc {
                id: TIME,
                name: "Time".into(),
                min: 0.0,
                max: (TIMES.len() - 1) as f64,
                default: DEFAULT_TIME as f64,
                steps: Some(TIMES.len() as u32),
                labels: TIMES.iter().map(|t| t.0.to_string()).collect(),
                unit: String::new(),
            },
            ParamDesc {
                id: REPEATS,
                name: "Repeats".into(),
                min: 1.0,
                max: MAX_REPEATS as f64,
                default: DEFAULT_REPEATS as f64,
                steps: Some(MAX_REPEATS),
                labels: Vec::new(),
                unit: String::new(),
            },
            ParamDesc {
                id: DECAY,
                name: "Decay".into(),
                min: 0.0,
                max: 100.0,
                default: DEFAULT_DECAY,
                steps: None,
                labels: Vec::new(),
                unit: "%".into(),
            },
            ParamDesc {
                id: PITCH,
                name: "Pitch".into(),
                min: -12.0,
                max: 12.0,
                default: 0.0,
                steps: Some(25),
                labels: Vec::new(),
                unit: " st".into(),
            },
        ]
    }

    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            TIME => self.time = Self::ticks(value.round().max(0.0) as usize),
            REPEATS => self.repeats = value.round().clamp(1.0, MAX_REPEATS as f64) as u32,
            DECAY => self.keep = 1.0 - value.clamp(0.0, 100.0) / 100.0,
            PITCH => self.step = value.round() as i64,
            _ => {}
        }
    }

    fn note(&mut self, note: NoteIn, clock: Clock, out: &mut Outbox) {
        match note {
            NoteIn::On { sound, pitch, velocity, length } => {
                if length.is_none() && self.held.len() < HELD_ROOM {
                    self.held.push((sound, pitch));
                }
                out.on(clock.now, sound, pitch, velocity, length);
                self.echoes(sound, pitch, velocity, length, clock, out);
            }
            NoteIn::Off { sound, pitch } => {
                if let Some(i) = self.held.iter().position(|h| *h == (sound, pitch)) {
                    self.held.swap_remove(i);
                }
                out.off(clock.now, sound, pitch);
            }
        }
    }

    fn finish(&mut self, clock: Clock, out: &mut Outbox) {
        for (sound, pitch) in self.held.drain(..) {
            out.off(clock.now, sound, pitch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi_input::{NoteOut, OUT_ROOM};

    const CLOCK: Clock = Clock { now: 1000.0, span: 0.0, song: None, seconds: 0.0 };

    /// What a plug-in plays for `note`, as (ticks from now, pitch, velocity, length).
    fn played(echo: &mut NoteEcho, note: NoteIn) -> Vec<(f64, i64, i64, Option<i64>)> {
        let mut notes = Vec::with_capacity(OUT_ROOM);
        echo.note(note, CLOCK, &mut Outbox::new(&mut notes));
        notes
            .iter()
            .filter_map(|n| match *n {
                NoteOut::On { at, pitch, velocity, length, .. } => Some((at - CLOCK.now, pitch, velocity, length)),
                NoteOut::Off { .. } => None,
            })
            .collect()
    }

    fn hit(pitch: i64, velocity: i64, length: Option<i64>) -> NoteIn {
        NoteIn::On { sound: 2, pitch, velocity, length }
    }

    /// Set a parameter as `Output` would: fitted to what it says it takes.
    fn set(echo: &mut NoteEcho, id: u32, value: f64) {
        let desc = echo.params().into_iter().find(|p| p.id == id).unwrap();
        echo.set_param(id, desc.fit(value));
    }

    #[test]
    fn a_note_plays_then_echoes_quieter_at_its_defaults() {
        let mut echo = NoteEcho::new();
        let notes = played(&mut echo, hit(60, 100, Some(120)));
        assert_eq!(
            notes,
            [(0.0, 60, 100, Some(120)), (360.0, 60, 65, Some(120)), (720.0, 60, 42, Some(120)), (1080.0, 60, 27, Some(120)),]
        );
    }

    #[test]
    fn parameters_change_the_echoes() {
        let mut echo = NoteEcho::new();
        set(&mut echo, TIME, 7.0);
        set(&mut echo, REPEATS, 2.0);
        set(&mut echo, DECAY, 50.0);
        set(&mut echo, PITCH, 7.0);
        let notes = played(&mut echo, hit(60, 100, Some(120)));
        assert_eq!(notes, [(0.0, 60, 100, Some(120)), (960.0, 67, 50, Some(120)), (1920.0, 74, 25, Some(120))]);
    }

    #[test]
    fn a_long_note_or_a_held_key_gives_short_echoes() {
        let mut echo = NoteEcho::new();
        assert_eq!(played(&mut echo, hit(60, 100, Some(5000)))[1].3, Some(360), "no longer than the time");
        assert_eq!(played(&mut echo, hit(60, 100, None))[1].3, Some(180), "half the time");
    }

    #[test]
    fn echoes_stop_at_silence_and_at_the_ends_of_the_keyboard() {
        let mut echo = NoteEcho::new();
        set(&mut echo, REPEATS, 8.0);
        set(&mut echo, DECAY, 100.0);
        assert_eq!(played(&mut echo, hit(60, 100, Some(120))).len(), 1, "silent at once");

        let mut echo = NoteEcho::new();
        set(&mut echo, REPEATS, 8.0);
        set(&mut echo, DECAY, 0.0);
        set(&mut echo, PITCH, 12.0);
        assert_eq!(played(&mut echo, hit(100, 100, Some(120))).len(), 3, "100, 112, 124; 136 isn't a note");
    }

    #[test]
    fn taken_out_it_ends_the_keys_still_down_only() {
        let mut echo = NoteEcho::new();
        played(&mut echo, hit(60, 100, None));
        played(&mut echo, hit(62, 100, None));
        played(&mut echo, hit(64, 100, Some(120)));
        let mut notes = Vec::with_capacity(OUT_ROOM);
        echo.note(NoteIn::Off { sound: 2, pitch: 60 }, CLOCK, &mut Outbox::new(&mut notes));
        assert_eq!(notes, [NoteOut::Off { at: CLOCK.now, sound: 2, pitch: 60 }]);

        echo.finish(CLOCK, &mut Outbox::new(&mut notes));
        assert_eq!(notes, [NoteOut::Off { at: CLOCK.now, sound: 2, pitch: 62 }]);
    }

    #[test]
    fn a_time_shows_its_name() {
        let time = NoteEcho::new().params().into_iter().find(|p| p.id == TIME).unwrap();
        assert_eq!(time.text(time.default), "3/16");
        assert_eq!(time.text(7.0), "1/4");
    }
}
