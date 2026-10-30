//! Ableton Link: a session of applications on the network sharing a tempo, a beat
//! timeline (its phase over a bar, the quantum) and, for those that want it, whether
//! the transport plays.
//!
//! Built from Ableton's own Link and its C wrapper `abl_link` (vendored, GPL-2.0-or-
//! later). [`Link`] is used from the app's threads; [`AudioClock`] reads the session
//! on the audio thread without blocking. Where the C++ isn't built (see `build.rs`)
//! the same API runs a session of one on the computer's clock ([`AVAILABLE`] is false),
//! as [`Link::local`] does anywhere (for tests: it never goes on the network).

#[cfg(link_native)]
mod ffi;
mod local;

use std::sync::{Arc, Mutex};

/// Ableton's Link, or the session of one.
enum Instance {
    #[cfg(link_native)]
    Native(ffi::Instance),
    Local(local::Instance),
}

/// A session state snapshot of the instance's kind.
enum State {
    #[cfg(link_native)]
    Native(ffi::State),
    Local(local::State),
}

/// Call `$f` on whichever an `Instance` or `State` is.
macro_rules! each {
    ($v:expr, $x:ident => $f:expr) => {
        match $v {
            #[cfg(link_native)]
            Instance::Native($x) => $f,
            Instance::Local($x) => $f,
        }
    };
}

macro_rules! each_state {
    ($v:expr, $x:ident => $f:expr) => {
        match $v {
            #[cfg(link_native)]
            State::Native($x) => $f,
            State::Local($x) => $f,
        }
    };
}

impl Instance {
    fn state(&self) -> State {
        match self {
            #[cfg(link_native)]
            Instance::Native(_) => State::Native(ffi::State::new()),
            Instance::Local(_) => State::Local(local::State::new()),
        }
    }

    fn enable(&self, on: bool) {
        each!(self, i => i.enable(on))
    }

    fn is_enabled(&self) -> bool {
        each!(self, i => i.is_enabled())
    }

    fn set_start_stop_sync(&self, on: bool) {
        each!(self, i => i.set_start_stop_sync(on))
    }

    fn start_stop_sync(&self) -> bool {
        each!(self, i => i.start_stop_sync())
    }

    fn peers(&self) -> u64 {
        each!(self, i => i.peers())
    }

    fn clock_micros(&self) -> i64 {
        each!(self, i => i.clock_micros())
    }

    fn capture_app(&self, s: &State) {
        match (self, s) {
            #[cfg(link_native)]
            (Instance::Native(i), State::Native(s)) => i.capture_app(s),
            (Instance::Local(i), State::Local(s)) => i.capture_app(s),
            #[cfg(link_native)]
            _ => {}
        }
    }

    fn commit_app(&self, s: &State) {
        match (self, s) {
            #[cfg(link_native)]
            (Instance::Native(i), State::Native(s)) => i.commit_app(s),
            (Instance::Local(i), State::Local(s)) => i.commit_app(s),
            #[cfg(link_native)]
            _ => {}
        }
    }

    fn capture_audio(&self, s: &State) {
        match (self, s) {
            #[cfg(link_native)]
            (Instance::Native(i), State::Native(s)) => i.capture_audio(s),
            (Instance::Local(i), State::Local(s)) => i.capture_audio(s),
            #[cfg(link_native)]
            _ => {}
        }
    }
}

impl State {
    fn tempo(&self) -> f64 {
        each_state!(self, s => s.tempo())
    }

    fn set_tempo(&self, bpm: f64, at: i64) {
        each_state!(self, s => s.set_tempo(bpm, at))
    }

    fn beat_at(&self, at: i64, quantum: f64) -> f64 {
        each_state!(self, s => s.beat_at(at, quantum))
    }

    fn phase_at(&self, at: i64, quantum: f64) -> f64 {
        each_state!(self, s => s.phase_at(at, quantum))
    }

    fn is_playing(&self) -> bool {
        each_state!(self, s => s.is_playing())
    }

    fn set_playing(&self, on: bool, at: i64) {
        each_state!(self, s => s.set_playing(on, at))
    }
}

/// Whether this build has Ableton's Link (else a session of one, never on the network).
pub const AVAILABLE: bool = cfg!(link_native);

/// The licence of Ableton Link, for the About box.
pub const LICENSE: &str = "Ableton Link © Ableton AG, under the GNU General Public License, version 2 or later. \
     asio © Christopher M. Kohlhoff, under the Boost Software License 1.0.";

/// A Link instance. Disabled when made: it joins the network's session once enabled.
pub struct Link {
    inner: Arc<Instance>,
    /// The session state the app's threads capture into and commit from.
    app: Mutex<State>,
}

/// Where a session is at a moment: its tempo (BPM), the beat there (on the quantum
/// asked for: beats of a bar line up with the session's bars) and whether its
/// transport plays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moment {
    pub tempo: f64,
    pub beat: f64,
    pub playing: bool,
}

impl Link {
    /// An instance whose session starts at `bpm`.
    pub fn new(bpm: f64) -> Link {
        #[cfg(link_native)]
        let inner = Instance::Native(ffi::Instance::new(bpm));
        #[cfg(not(link_native))]
        let inner = Instance::Local(local::Instance::new(bpm));
        Link::with(inner)
    }

    /// A session of one that never goes on the network, even enabled (for tests).
    pub fn local(bpm: f64) -> Link {
        Link::with(Instance::Local(local::Instance::new(bpm)))
    }

    fn with(inner: Instance) -> Link {
        let app = Mutex::new(inner.state());
        Link { inner: Arc::new(inner), app }
    }

    /// Join (or leave) the session on the network.
    pub fn enable(&self, on: bool) {
        self.inner.enable(on);
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.is_enabled()
    }

    /// Share starting and stopping with the peers that also do.
    pub fn set_start_stop_sync(&self, on: bool) {
        self.inner.set_start_stop_sync(on);
    }

    pub fn start_stop_sync(&self) -> bool {
        self.inner.start_stop_sync()
    }

    /// Other applications in the session.
    pub fn peers(&self) -> u64 {
        self.inner.peers()
    }

    /// The session's tempo.
    pub fn tempo(&self) -> f64 {
        self.capture(|s| s.tempo())
    }

    /// Change the session's tempo (for every peer), now.
    pub fn set_tempo(&self, bpm: f64) {
        let now = self.inner.clock_micros();
        self.change(|s| s.set_tempo(bpm, now));
    }

    /// Whether the session's transport plays (shared only with Start Stop Sync).
    pub fn is_playing(&self) -> bool {
        self.capture(|s| s.is_playing())
    }

    /// Start or stop the session's transport, now.
    pub fn set_playing(&self, on: bool) {
        let now = self.inner.clock_micros();
        self.change(|s| s.set_playing(on, now));
    }

    /// The session's beat within a bar of `quantum` beats, now (0 ≤ phase < quantum).
    pub fn phase(&self, quantum: f64) -> f64 {
        let now = self.inner.clock_micros();
        self.capture(|s| s.phase_at(now, quantum))
    }

    /// A reader of the session for the audio thread.
    pub fn audio_clock(&self) -> AudioClock {
        AudioClock { inner: self.inner.clone(), state: self.inner.state() }
    }

    fn capture<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        let state = self.app.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.capture_app(&state);
        f(&state)
    }

    fn change(&self, f: impl FnOnce(&State)) {
        let state = self.app.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.capture_app(&state);
        f(&state);
        self.inner.commit_app(&state);
    }
}

/// Reads the session on the audio thread: realtime-safe (it neither blocks nor
/// allocates). Made on another thread ([`Link::audio_clock`]), used by one.
pub struct AudioClock {
    inner: Arc<Instance>,
    state: State,
}

impl AudioClock {
    /// The session `ahead_micros` from now (when the audio being rendered is heard), on
    /// a bar of `quantum` beats.
    pub fn at(&mut self, ahead_micros: i64, quantum: f64) -> Moment {
        let at = self.inner.clock_micros() + ahead_micros;
        self.inner.capture_audio(&self.state);
        Moment { tempo: self.state.tempo(), beat: self.state.beat_at(at, quantum), playing: self.state.is_playing() }
    }
}
