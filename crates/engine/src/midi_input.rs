//! MIDI input plug-ins: a slot on a group, before its sounds' instruments, for code that
//! takes notes and plays other notes (an arpeggiator, a chord maker, a note filter). Every
//! note heading for the group's sounds goes in, played or from a pattern, and only what
//! the plug-in plays is heard. Recording happens before it, so a take holds what was
//! played. See docs/midi-input-plugins.md.
//!
//! A plug-in runs on the audio thread: nothing in it may allocate, wait or panic.

/// Where a plug-in is called from, in ticks ([`deckard_model::PPQ`] to a quarter note).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clock {
    /// A clock that runs on whether or not the song plays. Notes are played at times on it.
    pub now: f64,
    /// How far [`MidiInput::process`] looks: it plays what's due before `now + span`.
    pub span: f64,
    /// Where the song is at `now`, `None` when it's stopped.
    pub song: Option<f64>,
    /// The same moment as `now`, in seconds, for what doesn't follow the tempo (a window
    /// for keys struck together, say). It runs on as `now` does.
    pub seconds: f64,
}

/// A note going into a plug-in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoteIn {
    On {
        sound: usize,
        pitch: i64,
        velocity: i64,
        /// Ticks, for a note whose end is known (a pattern's, a repeat's). No [`NoteIn::Off`]
        /// follows one. `None`: a key held down, until its `Off`.
        length: Option<i64>,
    },
    /// Comes for a key the plug-in may not have seen go down, if it was put in meanwhile.
    Off { sound: usize, pitch: i64 },
}

/// A note a plug-in plays, at `at` on [`Clock::now`]'s clock (now, if that's past).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoteOut {
    On {
        at: f64,
        sound: usize,
        pitch: i64,
        velocity: i64,
        /// Ticks. `None`: until the plug-in plays its [`NoteOut::Off`].
        length: Option<i64>,
    },
    Off {
        at: f64,
        sound: usize,
        pitch: i64,
    },
}

/// How many notes a plug-in can play in one call.
pub const OUT_ROOM: usize = 256;
/// Of those, how many only a note's end can take, so that a full box can't leave one stuck.
const OFF_ROOM: usize = 64;

/// Where a plug-in plays its notes. It holds a fixed number and never allocates.
pub struct Outbox<'a> {
    notes: &'a mut Vec<NoteOut>,
}

impl<'a> Outbox<'a> {
    pub(crate) fn new(notes: &'a mut Vec<NoteOut>) -> Outbox<'a> {
        notes.clear();
        Outbox { notes }
    }

    /// Play a note; false if there was no room. `length`: see [`NoteOut::On`].
    pub fn on(&mut self, at: f64, sound: usize, pitch: i64, velocity: i64, length: Option<i64>) -> bool {
        let fits = self.notes.len() < OUT_ROOM - OFF_ROOM;
        if fits {
            self.notes.push(NoteOut::On { at, sound, pitch, velocity, length });
        }
        fits
    }

    /// End a note played with no length; false if there was no room.
    pub fn off(&mut self, at: f64, sound: usize, pitch: i64) -> bool {
        let fits = self.notes.len() < OUT_ROOM;
        if fits {
            self.notes.push(NoteOut::Off { at, sound, pitch });
        }
        fits
    }
}

/// A MIDI input plug-in. The engine owns it; put one in with `Output::set_midi_input`.
pub trait MidiInput: Send {
    /// A note came in. A plug-in that plays nothing for it has silenced it.
    fn note(&mut self, note: NoteIn, clock: Clock, out: &mut Outbox);

    /// Every note due at this instant has been given to [`MidiInput::note`], on every
    /// group; called for the plug-ins that had one. So notes that belong together (a chord
    /// in a pattern, a chord on another group) can be worked on as one, whatever order the
    /// groups came in.
    fn settle(&mut self, _clock: Clock, _out: &mut Outbox) {}

    /// Called once per block, for what the plug-in plays by itself: play what's due
    /// before `clock.now + clock.span`, at its time.
    fn process(&mut self, _clock: Clock, _out: &mut Outbox) {}

    /// The transport stopped: the engine has ended every note, and any it was still to
    /// play are gone. Forget what's held in the song's name; keys may still be down.
    fn stopped(&mut self) {}

    /// It's being taken out: end every note it holds.
    fn finish(&mut self, _clock: Clock, _out: &mut Outbox) {}
}

/// Slots, one per group: all a project can have (eight banks of eight).
pub(crate) const SLOTS: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_box_keeps_room_for_notes_to_end_and_never_grows() {
        let mut notes = Vec::with_capacity(OUT_ROOM);
        let capacity = notes.capacity();
        let mut out = Outbox::new(&mut notes);
        let mut played = 0;
        while out.on(0.0, 0, 60, 100, None) {
            played += 1;
        }
        assert_eq!(played, OUT_ROOM - OFF_ROOM);
        let mut ended = 0;
        while out.off(0.0, 0, 60) {
            ended += 1;
        }
        assert_eq!(ended, OFF_ROOM);
        assert_eq!(notes.capacity(), capacity);
    }
}
