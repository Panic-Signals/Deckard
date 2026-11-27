use super::*;

/// An engine with Group 0's Sound 0 sent out to MIDI, so what it plays can be read, with
/// [`OctaveUp`] on the group.
fn midi_input_engine(later: f64) -> (Engine, std::sync::mpsc::Receiver<midi::MidiOut>, NotesIn, Arc<Mutex<bool>>) {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
    let (tx, rx) = std::sync::mpsc::sync_channel(1024);
    e.midi_tx = Some(tx);
    e.midi_routes = vec![vec![Some(midi::OutRoute::Port { port: "Out".into(), channel: 0, transpose: 0 })]];
    let (seen, stopped) = (NotesIn::default(), Arc::new(Mutex::new(false)));
    let plugin = OctaveUp { seen: seen.clone(), stopped: stopped.clone(), later, held: None };
    assert!(e.set_midi_input(0, Some(Box::new(plugin))).is_none());
    (e, rx, seen, stopped)
}

/// The (status, note) of what went out, and whether each is a note-on.
fn midi_notes(rx: &std::sync::mpsc::Receiver<midi::MidiOut>) -> Vec<(bool, u8)> {
    rx.try_iter()
        .filter_map(|m| match m.bytes()[0] & 0xf0 {
            0x90 => Some((true, m.bytes()[1])),
            0x80 => Some((false, m.bytes()[1])),
            _ => None,
        })
        .collect()
}

/// A pattern's notes go in with their lengths; only what the plug-in plays is heard.
#[test]
fn a_midi_input_plugin_takes_a_patterns_notes() {
    let (mut e, rx, seen, _) = midi_input_engine(0.0);
    blocks(&mut e, 1);
    assert_eq!(seen.lock().unwrap()[0], midi_input::NoteIn::On { sound: 0, pitch: 60, velocity: 127, length: Some(240) });
    assert_eq!(midi_notes(&rx), [(true, 72)], "an octave up, and not as written");
}

/// A key is recorded as played, before the plug-in, and heard as it plays it.
#[test]
fn a_take_holds_what_was_played_before_a_midi_input_plugin() {
    let (mut e, rx, seen, _) = midi_input_engine(0.0);
    e.set_recording(true);
    blocks(&mut e, 1);
    seen.lock().unwrap().clear();
    let _ = midi_notes(&rx);
    e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 50, velocity: 90, length: HELD });
    blocks(&mut e, 1);
    assert_eq!(seen.lock().unwrap()[0], midi_input::NoteIn::On { sound: 0, pitch: 50, velocity: 90, length: None });
    assert_eq!(midi_notes(&rx), [(true, 62)]);
    e.schedule(0, Live::NoteRelease { group: 0, sound: 0, pitch: 50 });
    blocks(&mut e, 1);
    assert_eq!(seen.lock().unwrap()[1], midi_input::NoteIn::Off { sound: 0, pitch: 50 });
    assert_eq!(midi_notes(&rx), [(false, 62)]);
    let rec = e.take_recorded();
    assert_eq!(rec.iter().map(|r| r.pitch).collect::<Vec<_>>(), [50, 50], "{rec:?}");
}

/// What a plug-in plays later plays then, not before.
#[test]
fn a_midi_input_plugin_plays_at_the_time_it_says() {
    let (mut e, rx, _, _) = midi_input_engine(120.0);
    let frames_late = 120.0 * e.samples_per_tick();
    let early = (frames_late / 256.0).floor() as usize - 1;
    blocks(&mut e, early);
    assert_eq!(midi_notes(&rx), [], "120 ticks not gone yet");
    blocks(&mut e, 3);
    assert_eq!(midi_notes(&rx), [(true, 72)]);
}

/// Stopping drops what it was still to play and tells it.
#[test]
fn stopping_ends_what_a_midi_input_plugin_was_to_play() {
    let (mut e, rx, _, stopped) = midi_input_engine(480.0);
    blocks(&mut e, 2);
    e.stop();
    assert!(*stopped.lock().unwrap());
    let n = (960.0 * e.samples_per_tick() / 256.0).ceil() as usize;
    blocks(&mut e, n);
    assert_eq!(midi_notes(&rx), [], "nothing after the stop");
}

/// Note Echo's echoes come out at their times, and stopping ends those still to come.
#[test]
fn note_echo_plays_later_and_stops_with_the_song() {
    let (mut e, rx, _, _) = midi_input_engine(0.0);
    blocks(&mut e, 1);
    e.set_midi_input(0, Some(Box::new(note_echo::NoteEcho::new())));
    let _ = midi_notes(&rx);

    e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 50, velocity: 90, length: 60 });
    blocks(&mut e, 1);
    assert_eq!(midi_notes(&rx), [(true, 50)], "the note itself, at once");

    // At its defaults, the first echo is 360 ticks on.
    let block = 256.0 / e.samples_per_tick();
    blocks(&mut e, (340.0 / block).floor() as usize);
    assert!(!midi_notes(&rx).contains(&(true, 50)), "not yet");
    blocks(&mut e, (40.0 / block).ceil() as usize);
    assert!(midi_notes(&rx).contains(&(true, 50)), "the first echo");

    e.stop();
    let _ = midi_notes(&rx);
    blocks(&mut e, (1200.0 / block).ceil() as usize);
    assert!(!midi_notes(&rx).contains(&(true, 50)), "the rest are gone");
}

/// A parameter reaches the plug-in, and takes effect on the next note.
#[test]
fn a_midi_input_plugin_hears_its_parameters() {
    struct Transpose(i64);
    impl midi_input::MidiInput for Transpose {
        fn note(&mut self, note: midi_input::NoteIn, clock: midi_input::Clock, out: &mut midi_input::Outbox) {
            match note {
                midi_input::NoteIn::On { sound, pitch, velocity, length } => {
                    out.on(clock.now, sound, pitch + self.0, velocity, length);
                }
                midi_input::NoteIn::Off { sound, pitch } => {
                    out.off(clock.now, sound, pitch + self.0);
                }
            }
        }

        fn set_param(&mut self, id: u32, value: f64) {
            if id == 1 {
                self.0 = value as i64;
            }
        }
    }

    let (mut e, rx, _, _) = midi_input_engine(0.0);
    blocks(&mut e, 1);
    e.set_midi_input(0, Some(Box::new(Transpose(0))));
    let _ = midi_notes(&rx);

    e.set_midi_input_param(0, 1, 5.0);
    // Nothing there to hear it, and no such group: nothing happens.
    e.set_midi_input_param(1, 1, 7.0);
    e.set_midi_input_param(midi_input::SLOTS, 1, 7.0);
    e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 50, velocity: 90, length: HELD });
    blocks(&mut e, 1);
    assert_eq!(midi_notes(&rx), [(true, 55)]);
}

/// Taken out, a plug-in ends what it holds, and notes go straight to the sound again.
#[test]
fn taking_a_midi_input_plugin_out_ends_its_notes() {
    let (mut e, rx, seen, _) = midi_input_engine(0.0);
    blocks(&mut e, 1);
    e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 50, velocity: 90, length: HELD });
    blocks(&mut e, 1);
    let _ = midi_notes(&rx);
    assert!(e.set_midi_input(0, None).is_some(), "it comes back, to be freed");
    assert_eq!(midi_notes(&rx), [(false, 62)]);
    let before = seen.lock().unwrap().len();
    e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 55, velocity: 90, length: HELD });
    blocks(&mut e, 1);
    assert_eq!(seen.lock().unwrap().len(), before);
    assert_eq!(midi_notes(&rx), [(true, 55)]);
}

/// A muted sound's notes don't go in, and a slot past the last refuses.
#[test]
fn a_midi_input_plugin_skips_muted_notes_and_slots_end() {
    let (mut e, _rx, seen, _) = midi_input_engine(0.0);
    e.sound_muted = vec![vec![true]];
    blocks(&mut e, 1);
    assert!(seen.lock().unwrap().is_empty());
    let plugin = OctaveUp { seen: NotesIn::default(), stopped: Default::default(), later: 0.0, held: None };
    assert!(e.set_midi_input(midi_input::SLOTS, Some(Box::new(plugin))).is_some());
}

/// Sounds rendered ahead have their pattern's notes sent in advance. With a plug-in on
/// the group they aren't: a plug-in that plays nothing silences them, and one that plays
/// its notes is heard.
#[test]
fn a_midi_input_plugin_takes_notes_from_sounds_rendered_ahead() {
    struct Silent;
    impl midi_input::MidiInput for Silent {
        fn note(&mut self, _: midi_input::NoteIn, _: midi_input::Clock, _: &mut midi_input::Outbox) {}
    }
    let (_tmp, p) = eight_project();
    let loudest = |plugin: Option<Box<dyn midi_input::MidiInput>>| {
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default());
        e.looping = true;
        assert!(e.set_midi_input(0, plugin).is_none());
        e.render_ahead(700, 2, 256, true);
        let mut buf = vec![0f32; 512];
        let mut most = 0f32;
        for _ in 0..188 {
            e.process(&mut buf);
            most = buf.iter().fold(most, |a, s| a.max(s.abs()));
        }
        assert_eq!(e.ahead_underruns(), 0);
        most
    };
    assert!(loudest(None) > 0.01, "the project plays");
    assert_eq!(loudest(Some(Box::new(Silent))), 0.0, "nothing got past it");
    let plugin = OctaveUp { seen: NotesIn::default(), stopped: Default::default(), later: 0.0, held: None };
    assert!(loudest(Some(Box::new(plugin))) > 0.01, "what it plays reaches them");
}

/// A plug-in that holds what comes in until the notes are settled, and says what it
/// was called with.
struct Settling {
    group: usize,
    log: Arc<Mutex<Vec<(&'static str, usize, f64)>>>,
    held: Vec<i64>,
}

impl midi_input::MidiInput for Settling {
    fn note(&mut self, note: midi_input::NoteIn, clock: midi_input::Clock, _: &mut midi_input::Outbox) {
        self.log.lock().unwrap().push(("note", self.group, clock.seconds));
        if let midi_input::NoteIn::On { pitch, .. } = note {
            self.held.push(pitch);
        }
    }

    fn settle(&mut self, clock: midi_input::Clock, out: &mut midi_input::Outbox) {
        self.log.lock().unwrap().push(("settle", self.group, clock.seconds));
        for pitch in self.held.drain(..) {
            out.on(clock.now, 0, pitch, 100, Some(120));
        }
    }
}

/// Notes due at one instant all go in before any plug-in settles, in whatever order
/// the groups came; each plug-in settles once, and a later instant is another.
#[test]
fn midi_input_plugins_settle_once_every_note_is_in() {
    let (mut e, rx, _, _) = midi_input_engine(0.0);
    let log = Arc::new(Mutex::new(Vec::new()));
    for group in [0, 1] {
        let plugin = Settling { group, log: log.clone(), held: Vec::with_capacity(8) };
        e.set_midi_input(group, Some(Box::new(plugin)));
    }
    e.next = e.timeline.events.len();
    blocks(&mut e, 4);
    assert!(log.lock().unwrap().is_empty(), "no notes, nothing to settle");
    let _ = midi_notes(&rx);
    // Group 1's first: the later group's chord has to be in before group 0 settles.
    for (group, pitch) in [(1, 50), (0, 60), (0, 64)] {
        e.schedule(0, Live::NoteOn { group, sound: 0, pitch, velocity: 100, length: HELD });
    }
    blocks(&mut e, 1);
    let seen: Vec<(&str, usize)> = log.lock().unwrap().iter().map(|&(what, g, _)| (what, g)).collect();
    assert_eq!(seen, [("note", 1), ("note", 0), ("note", 0), ("settle", 0), ("settle", 1)]);
    assert_eq!(midi_notes(&rx), [(true, 60), (true, 64)], "what group 0 played, together, once settled");
    let now = log.lock().unwrap()[0].2;
    assert!((now - 4.0 * 256.0 / 48_000.0).abs() < 1e-9, "seconds run with the clock: {now}");
    log.lock().unwrap().clear();
    blocks(&mut e, 2);
    assert!(log.lock().unwrap().is_empty(), "settled once");
}

/// An engine that replaces this one has its plug-ins, told the song stopped.
#[test]
fn a_new_engine_keeps_the_midi_input_plugins() {
    let (mut old, _rx, seen, stopped) = midi_input_engine(0.0);
    let mut cache = SampleCache::default();
    let (mut new, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
    new.take_midi_inputs(&mut old);
    assert!(*stopped.lock().unwrap());
    blocks(&mut new, 1);
    assert!(!seen.lock().unwrap().is_empty());
}
