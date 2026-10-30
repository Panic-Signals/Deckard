//! A session of one, on the computer's clock: what [`crate::Link`] runs where Ableton's
//! Link isn't built. Its timeline starts at beat 0 when made and runs at its tempo;
//! enabling it finds nobody.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// The timeline: beat `beat` at `micros`, `tempo` from there; whether it plays.
#[derive(Debug, Clone, Copy, Default)]
struct Timeline {
    tempo: f64,
    micros: i64,
    beat: f64,
    playing: bool,
}

impl Timeline {
    fn beat_at(&self, at: i64) -> f64 {
        self.beat + (at - self.micros) as f64 / 60e6 * self.tempo
    }
}

pub struct Instance {
    start: Instant,
    enabled: AtomicBool,
    start_stop: AtomicBool,
    session: Mutex<Timeline>,
}

impl Instance {
    pub fn new(bpm: f64) -> Instance {
        Instance {
            start: Instant::now(),
            enabled: AtomicBool::new(false),
            start_stop: AtomicBool::new(false),
            session: Mutex::new(Timeline { tempo: bpm, ..Timeline::default() }),
        }
    }

    pub fn enable(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_start_stop_sync(&self, on: bool) {
        self.start_stop.store(on, Ordering::Relaxed);
    }

    pub fn start_stop_sync(&self) -> bool {
        self.start_stop.load(Ordering::Relaxed)
    }

    pub fn peers(&self) -> u64 {
        0
    }

    pub fn clock_micros(&self) -> i64 {
        self.start.elapsed().as_micros() as i64
    }

    pub fn capture_app(&self, s: &State) {
        if let (Ok(from), Ok(mut to)) = (self.session.lock(), s.0.lock()) {
            *to = *from;
        }
    }

    pub fn commit_app(&self, s: &State) {
        if let (Ok(mut to), Ok(from)) = (self.session.lock(), s.0.lock()) {
            *to = *from;
        }
    }

    /// (Never waits: if the app is committing, the snapshot taken last time stands.)
    pub fn capture_audio(&self, s: &State) {
        if let (Ok(from), Ok(mut to)) = (self.session.try_lock(), s.0.try_lock()) {
            *to = *from;
        }
    }
}

pub struct State(Mutex<Timeline>);

impl State {
    pub fn new() -> State {
        State(Mutex::new(Timeline::default()))
    }

    fn get(&self) -> Timeline {
        self.0.lock().map(|t| *t).unwrap_or_default()
    }

    pub fn tempo(&self) -> f64 {
        self.get().tempo
    }

    pub fn set_tempo(&self, bpm: f64, at: i64) {
        if let Ok(mut t) = self.0.lock() {
            // The beat at `at` stays; from there it runs at the new tempo.
            let beat = t.beat_at(at);
            *t = Timeline { tempo: bpm, micros: at, beat, ..*t };
        }
    }

    pub fn beat_at(&self, at: i64, _quantum: f64) -> f64 {
        self.get().beat_at(at)
    }

    pub fn phase_at(&self, at: i64, quantum: f64) -> f64 {
        if quantum > 0.0 { self.beat_at(at, quantum).rem_euclid(quantum) } else { 0.0 }
    }

    pub fn is_playing(&self) -> bool {
        self.get().playing
    }

    pub fn set_playing(&self, on: bool, _at: i64) {
        if let Ok(mut t) = self.0.lock() {
            t.playing = on;
        }
    }
}
