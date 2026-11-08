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

/// A parameter of a plug-in: what a knob, a modulation lane or a snapshot needs to know.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDesc {
    /// What [`MidiInput::set_param`] is told.
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    /// Where it starts, without a call to [`MidiInput::set_param`].
    pub default: f64,
    /// How many values it can take, from `min` to `max` evenly (a switch has 2). `None`: any.
    pub steps: Option<u32>,
    /// What each step is called (empty: its number).
    pub labels: Vec<String>,
    /// After a number shown.
    pub unit: String,
}

impl ParamDesc {
    /// `value` as this parameter can hold it: in range, and on a step if it has them.
    pub fn fit(&self, value: f64) -> f64 {
        let v = value.clamp(self.min, self.max);
        match self.steps {
            Some(n) if n >= 2 => {
                let step = (self.max - self.min) / (n - 1) as f64;
                self.min + ((v - self.min) / step).round() * step
            }
            Some(_) => self.min,
            None => v,
        }
    }

    /// `value` as it's shown.
    pub fn text(&self, value: f64) -> String {
        let v = self.fit(value);
        if let Some(n) = self.steps.filter(|n| *n >= 2) {
            let step = ((v - self.min) * (n - 1) as f64 / (self.max - self.min)).round() as usize;
            if let Some(label) = self.labels.get(step) {
                return label.clone();
            }
            return format!("{v:.0}{}", self.unit);
        }
        let places = if self.max - self.min > 10.0 { 0 } else { 2 };
        format!("{v:.places$}{}", self.unit)
    }
}

/// A plug-in's parameter and where it is now, as `Output` keeps them for whoever shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub desc: ParamDesc,
    pub value: f64,
}

/// A plug-in on a group, as `Output` keeps it for whoever shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub name: String,
    pub params: Vec<Param>,
}

impl Info {
    /// `plugin`, its parameters as they start.
    pub fn of(plugin: &dyn MidiInput) -> Info {
        let params = plugin
            .params()
            .into_iter()
            .map(|desc| {
                let value = desc.fit(desc.default);
                Param { desc, value }
            })
            .collect();
        Info { name: plugin.name(), params }
    }
}

/// The plug-ins Deckard comes with, by name.
pub const BUILT_IN: [&str; 1] = [crate::note_echo::NAME];

/// A new plug-in of the one called `name`.
pub fn built_in(name: &str) -> Option<Box<dyn MidiInput>> {
    (name == crate::note_echo::NAME).then(|| Box::new(crate::note_echo::NoteEcho::new()) as Box<dyn MidiInput>)
}

/// A MIDI input plug-in. The engine owns it; put one in with `Output::set_midi_input`.
pub trait MidiInput: Send {
    /// A note came in. A plug-in that plays nothing for it has silenced it.
    fn note(&mut self, note: NoteIn, clock: Clock, out: &mut Outbox);

    /// What it's called. Asked once, off the audio thread, as it's put in.
    fn name(&self) -> String {
        "MIDI Plug-in".into()
    }

    /// Its parameters. Asked once, off the audio thread, as it's put in, so this one may
    /// allocate.
    fn params(&self) -> Vec<ParamDesc> {
        Vec::new()
    }

    /// A parameter moved: `id` is from [`MidiInput::params`] and `value` is in its range and
    /// on its steps. On the audio thread, between notes.
    fn set_param(&mut self, _id: u32, _value: f64) {}

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
pub const SLOTS: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(min: f64, max: f64, default: f64, steps: Option<u32>) -> ParamDesc {
        ParamDesc { id: 7, name: "Test".into(), min, max, default, steps, labels: Vec::new(), unit: "%".into() }
    }

    #[test]
    fn a_parameter_stays_in_range_and_on_its_steps() {
        let free = desc(-1.0, 1.0, 0.0, None);
        assert_eq!(free.fit(0.3), 0.3);
        assert_eq!(free.fit(5.0), 1.0);
        assert_eq!(free.fit(-5.0), -1.0);

        let stepped = desc(1.0, 8.0, 3.0, Some(8));
        assert_eq!(stepped.fit(4.4), 4.0);
        assert_eq!(stepped.fit(4.6), 5.0);
        assert_eq!(stepped.fit(99.0), 8.0);

        let switch = desc(0.0, 1.0, 0.0, Some(2));
        assert_eq!(switch.fit(0.4), 0.0);
        assert_eq!(switch.fit(0.6), 1.0);

        // Too few steps to mean anything, and a range with no width, don't divide by zero.
        assert_eq!(desc(2.0, 5.0, 2.0, Some(1)).fit(4.0), 2.0);
        assert_eq!(desc(2.0, 2.0, 2.0, Some(1)).fit(4.0), 2.0);
    }

    #[test]
    fn parameters_start_on_their_defaults_fitted() {
        struct Two;
        impl MidiInput for Two {
            fn note(&mut self, _: NoteIn, _: Clock, _: &mut Outbox) {}
            fn params(&self) -> Vec<ParamDesc> {
                vec![desc(0.0, 1.0, 0.25, None), desc(1.0, 8.0, 3.4, Some(8))]
            }
        }
        let params = Info::of(&Two).params;
        assert_eq!(params[0].value, 0.25);
        assert_eq!(params[1].value, 3.0, "a default between steps lands on one");
    }

    #[test]
    fn a_parameter_shows_its_label_or_its_number() {
        let mut named = desc(0.0, 2.0, 0.0, Some(3));
        named.labels = vec!["Low".into(), "Mid".into(), "High".into()];
        assert_eq!(named.text(1.2), "Mid");
        assert_eq!(desc(1.0, 8.0, 3.0, Some(8)).text(4.4), "4%");
        assert_eq!(desc(0.0, 100.0, 3.0, None).text(35.4), "35%");
        assert_eq!(desc(0.0, 1.0, 0.0, None).text(0.256), "0.26%");
    }

    #[test]
    fn built_in_plug_ins_are_found_by_name() {
        for name in BUILT_IN {
            assert_eq!(built_in(name).unwrap().name(), name);
        }
        assert!(built_in("Nothing").is_none());
    }

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
