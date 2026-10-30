//! Ableton's `abl_link` C API (`vendor/link/extensions/abl_link/include/abl_link.h`):
//! the functions DECKARD uses, declared by hand.

use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct AblLink {
    imp: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SessionState {
    imp: *mut c_void,
}

unsafe extern "C" {
    fn abl_link_create(bpm: f64) -> AblLink;
    fn abl_link_destroy(link: AblLink);
    fn abl_link_is_enabled(link: AblLink) -> bool;
    fn abl_link_enable(link: AblLink, enable: bool);
    fn abl_link_is_start_stop_sync_enabled(link: AblLink) -> bool;
    fn abl_link_enable_start_stop_sync(link: AblLink, enabled: bool);
    fn abl_link_num_peers(link: AblLink) -> u64;
    fn abl_link_clock_micros(link: AblLink) -> i64;
    fn abl_link_create_session_state() -> SessionState;
    fn abl_link_destroy_session_state(state: SessionState);
    fn abl_link_capture_audio_session_state(link: AblLink, state: SessionState);
    fn abl_link_capture_app_session_state(link: AblLink, state: SessionState);
    fn abl_link_commit_app_session_state(link: AblLink, state: SessionState);
    fn abl_link_tempo(state: SessionState) -> f64;
    fn abl_link_set_tempo(state: SessionState, bpm: f64, at_time: i64);
    fn abl_link_beat_at_time(state: SessionState, time: i64, quantum: f64) -> f64;
    fn abl_link_phase_at_time(state: SessionState, time: i64, quantum: f64) -> f64;
    fn abl_link_set_is_playing(state: SessionState, is_playing: bool, time: i64);
    fn abl_link_is_playing(state: SessionState) -> bool;
}

/// An `abl_link` instance (its functions are thread-safe).
pub struct Instance(AblLink);

// SAFETY: abl_link's instance functions are documented thread-safe.
unsafe impl Send for Instance {}
unsafe impl Sync for Instance {}

impl Instance {
    pub fn new(bpm: f64) -> Instance {
        // SAFETY: plain constructor; destroyed once, in Drop.
        Instance(unsafe { abl_link_create(bpm) })
    }

    pub fn enable(&self, on: bool) {
        unsafe { abl_link_enable(self.0, on) }
    }

    pub fn is_enabled(&self) -> bool {
        unsafe { abl_link_is_enabled(self.0) }
    }

    pub fn set_start_stop_sync(&self, on: bool) {
        unsafe { abl_link_enable_start_stop_sync(self.0, on) }
    }

    pub fn start_stop_sync(&self) -> bool {
        unsafe { abl_link_is_start_stop_sync_enabled(self.0) }
    }

    pub fn peers(&self) -> u64 {
        unsafe { abl_link_num_peers(self.0) }
    }

    pub fn clock_micros(&self) -> i64 {
        unsafe { abl_link_clock_micros(self.0) }
    }

    pub fn capture_app(&self, s: &State) {
        unsafe { abl_link_capture_app_session_state(self.0, s.0) }
    }

    pub fn commit_app(&self, s: &State) {
        unsafe { abl_link_commit_app_session_state(self.0, s.0) }
    }

    pub fn capture_audio(&self, s: &State) {
        unsafe { abl_link_capture_audio_session_state(self.0, s.0) }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe { abl_link_destroy(self.0) }
    }
}

/// A session state snapshot (one per thread that captures).
pub struct State(SessionState);

// SAFETY: a session state is plain data, used by one thread at a time (behind a Mutex,
// or owned by the audio thread).
unsafe impl Send for State {}

impl State {
    pub fn new() -> State {
        State(unsafe { abl_link_create_session_state() })
    }

    pub fn tempo(&self) -> f64 {
        unsafe { abl_link_tempo(self.0) }
    }

    pub fn set_tempo(&self, bpm: f64, at: i64) {
        unsafe { abl_link_set_tempo(self.0, bpm, at) }
    }

    pub fn beat_at(&self, at: i64, quantum: f64) -> f64 {
        unsafe { abl_link_beat_at_time(self.0, at, quantum) }
    }

    pub fn phase_at(&self, at: i64, quantum: f64) -> f64 {
        unsafe { abl_link_phase_at_time(self.0, at, quantum) }
    }

    pub fn is_playing(&self) -> bool {
        unsafe { abl_link_is_playing(self.0) }
    }

    pub fn set_playing(&self, on: bool, at: i64) {
        unsafe { abl_link_set_is_playing(self.0, on, at) }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        unsafe { abl_link_destroy_session_state(self.0) }
    }
}
