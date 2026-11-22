use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The default file name for a session recording: `session-<project>-<stamp>.wav`, or
/// `session-<stamp>.wav` when there is no project name.
pub fn session_filename(project: Option<&str>, stamp: &str) -> String {
    match project.filter(|p| !p.is_empty()) {
        Some(project) => format!("session-{}-{stamp}.wav", sanitize(project)),
        None => format!("session-{stamp}.wav"),
    }
}

/// The local time as `YYYY-MM-DD-HHMM`, from the same clock as `recorder::timestamp`.
pub fn session_stamp() -> String {
    // `recorder::timestamp` is `YYYYMMDDTHHMMSS`; reorder its pieces.
    let ts = crate::recorder::timestamp();
    format!("{}-{}-{}-{}", &ts[0..4], &ts[4..6], &ts[6..8], &ts[9..13])
}

/// The local clock time a recording started, like `9:04:18 PM`, from the same clock as
/// `recorder::timestamp` (12-hour, with AM/PM).
pub fn started_label() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let secs = secs + crate::recorder::local_offset(secs);
    let rem = secs.rem_euclid(86_400);
    let (h24, m, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    let (h12, period) = match h24 {
        0 => (12, "AM"),
        1..=11 => (h24, "AM"),
        12 => (12, "PM"),
        _ => (h24 - 12, "PM"),
    };
    format!("{h12}:{m:02}:{s:02} {period}")
}

/// Replaces each character outside `[A-Za-z0-9._-]` with `-`, collapsing runs of `-`.
fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    out
}

/// The largest data payload a WAV file can hold (RIFF sizes are 32-bit), kept under 4 GiB.
pub const WAV_CEILING_BYTES: u64 = 4_000_000_000;

/// Bytes written per second of 24-bit stereo audio at `rate`.
pub fn bytes_per_second(rate: u32) -> u64 {
    rate as u64 * 6
}

/// Seconds that can still be recorded: free space beyond `reserve`, capped at the WAV ceiling.
pub fn seconds_available(free: u64, reserve: u64, rate: u32) -> u64 {
    let per_second = bytes_per_second(rate);
    if per_second == 0 {
        return 0;
    }
    free.saturating_sub(reserve).min(WAV_CEILING_BYTES) / per_second
}

/// Whether there is more free space than the reserve.
pub fn can_start(free: u64, reserve: u64) -> bool {
    free > reserve
}

/// Where a session recording is in its life: armed (file chosen, metering, nothing
/// written), recording (writing), paused (holding — the paused time is left out of the
/// file), or idle (no session).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionPhase {
    #[default]
    Idle,
    Armed,
    Recording,
    Paused,
}

/// Why a recording ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Disk,
    Ceiling,
    RateChange,
    /// The audio output was torn down/rebuilt (device or buffer change) while recording.
    DeviceChange,
    WriteError,
    Manual,
}

/// The reason to stop automatically, if any: the disk is down to the reserve, or the WAV is full.
pub fn autostop_reason(data_bytes: u64, free: u64, reserve: u64) -> Option<StopReason> {
    if free <= reserve {
        Some(StopReason::Disk)
    } else if data_bytes >= WAV_CEILING_BYTES {
        Some(StopReason::Ceiling)
    } else {
        None
    }
}

/// Bytes available to a normal user on the filesystem holding `path` (or its nearest existing
/// ancestor), or `None` when that can't be determined.
pub fn free_bytes(path: &std::path::Path) -> Option<u64> {
    path.ancestors().find(|p| p.exists()).and_then(|p| fs4::available_space(p).ok())
}

/// A 24-bit stereo WAV that stays a valid file while it is being written: the header is
/// flushed about once per second of audio, so a crash loses at most that last second.
pub struct SessionWav {
    writer: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    rate: u32,
    data_bytes: u64,
    frames_since_flush: u32,
}

impl SessionWav {
    pub fn create(path: &std::path::Path, rate: u32) -> Result<SessionWav, String> {
        let rate = rate.max(1);
        let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 24, sample_format: hound::SampleFormat::Int };
        let writer = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
        Ok(SessionWav { writer, rate, data_bytes: 0, frames_since_flush: 0 })
    }

    /// Appends stereo frames, flushing the header once a second's worth has accumulated.
    pub fn write(&mut self, frames: &[[f32; 2]]) -> Result<(), String> {
        for frame in frames {
            for &c in frame {
                let v = (c.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
                self.writer.write_sample(v).map_err(|e| e.to_string())?;
            }
        }
        self.data_bytes += frames.len() as u64 * 6;
        self.frames_since_flush = self.frames_since_flush.saturating_add(frames.len().min(u32::MAX as usize) as u32);
        if self.frames_since_flush >= self.rate {
            self.writer.flush().map_err(|e| e.to_string())?;
            self.frames_since_flush = 0;
        }
        Ok(())
    }

    /// Audio bytes written so far.
    pub fn data_bytes(&self) -> u64 {
        self.data_bytes
    }

    pub fn finalize(self) -> Result<(), String> {
        self.writer.finalize().map_err(|e| e.to_string())
    }
}

/// The live, in-flight state of a running recording: the open file, the rate it was
/// opened at, and how many stereo frames have been written so far.
struct SessionRun {
    wav: SessionWav,
    rate: u32,
    written: u64,
    /// The recorded time at which audio last arrived (or the recording began).
    fed_at: f64,
    /// How many frames the file trailed the wall clock (negative: led it) when audio last
    /// arrived: block jitter and the drift of the device's clock against the computer's.
    /// That is how the two clocks run, not missing audio, so it is never padded.
    lag: i64,
}

/// A snapshot of the recorder for the UI: everything a window needs to draw, and nothing
/// that outlives a stop. `Default` is the idle state (not recording, no file).
#[derive(Default)]
pub struct SessionView {
    pub phase: SessionPhase,
    pub elapsed: f64,
    pub started_label: String,
    pub meter: (f32, f32),
    pub peak: (f32, f32),
    pub bytes: u64,
    pub free: u64,
    pub reserve: u64,
    pub rate: u32,
    pub path: Option<PathBuf>,
    /// One entry per scrolling bucket: that bucket's (left peak, right peak), each ≥ 0.
    pub wave: VecDeque<(f32, f32)>,
    pub error: Option<String>,
    pub stopped: Option<StopReason>,
}

/// How many wave buckets to keep for the scrolling meter display.
const WAVE_BUCKETS: usize = 240;

/// The most silence one `poll_at` will pad in a single call: a clock jump beyond this
/// (e.g. the machine sleeping) is caught up over the following polls instead of allocating
/// one enormous silent buffer.
const MAX_PAD_SECONDS: u64 = 4;

/// How long no audio must have arrived, and how much must be missing, before the gap is
/// filled with silence. Audio arrives a block at a time (at most some 4096 frames, under
/// 0.1 s) and a little unevenly, so anything shorter is jitter: padding it would splice
/// zeros into the middle of the music. Half a second is well clear of every block size,
/// and short enough that an engine that really stopped delivering is noticed at once.
/// (The price: after such a gap the file may trail the recorded time by up to this much.)
const GAP_SECONDS: f64 = 0.5;

/// Room for the tap's buffer (interleaved samples): more than a frame's worth of audio, so
/// the audio thread seldom has to grow it.
const TAP_ROOM: usize = 16_384;

/// Records the master output to a WAV file, filling the gaps where the engine delivered
/// nothing with silence, so the file's length follows the recorded time. `poll` is the
/// real-time wrapper (drain tap, read rate, compute elapsed); the testable core is `poll_at`.
pub struct SessionRecorder {
    pub view: SessionView,
    phase: SessionPhase,
    run: Option<SessionRun>,
    tap: Option<Arc<Mutex<Vec<f32>>>>,
    /// Recorded seconds accumulated before the current play span (grows at each pause); the
    /// current span's seconds are added live from `play_started`. Paused time is never
    /// counted, so the file's length always equals the recorded time.
    played_before: f64,
    play_started: Option<Instant>,
    /// The last free-space reading and when it was taken, so `poll` can throttle the
    /// `statvfs` syscall to about once a second instead of running it every frame.
    free_cache: u64,
    free_checked: Option<Instant>,
}

impl SessionRecorder {
    pub fn new() -> SessionRecorder {
        SessionRecorder {
            view: SessionView::default(),
            phase: SessionPhase::Idle,
            run: None,
            tap: None,
            played_before: 0.0,
            play_started: None,
            free_cache: u64::MAX,
            free_checked: None,
        }
    }

    /// Whether the session is actively writing audio.
    pub fn recording(&self) -> bool {
        self.phase == SessionPhase::Recording
    }

    /// Whether a session is live at all (armed, recording or paused) — i.e. it holds the tap.
    pub fn active(&self) -> bool {
        self.phase != SessionPhase::Idle
    }

    pub fn phase(&self) -> SessionPhase {
        self.phase
    }

    /// Arms the recorder: remembers the file to record to and shows live metering, but writes
    /// nothing until `play`. Does not touch any `Output`; the caller wires the tap via
    /// `attach_tap`. The WAV file is only created on the first `play`, so arming and then
    /// stopping leaves no file behind.
    pub fn arm(&mut self, path: PathBuf, rate: u32, reserve: u64) {
        self.run = None;
        self.played_before = 0.0;
        self.play_started = None;
        self.free_checked = None;
        self.phase = SessionPhase::Armed;
        self.view = SessionView { phase: SessionPhase::Armed, rate, reserve, path: Some(path), ..SessionView::default() };
    }

    /// Begins or resumes recording. The WAV file is created lazily on the first write (in
    /// `poll_at`, at the live rate). On the first play it stamps the start time.
    pub fn play(&mut self, started_label: String) {
        if matches!(self.phase, SessionPhase::Armed | SessionPhase::Paused) {
            if self.phase == SessionPhase::Armed {
                self.view.started_label = started_label;
            }
            self.phase = SessionPhase::Recording;
            self.view.phase = SessionPhase::Recording;
            self.play_started = Some(Instant::now());
        }
    }

    /// Pauses recording: freezes the recorded-time clock and stops writing. The file stays
    /// open; `play` resumes appending to it, and the paused span is simply absent from it.
    pub fn pause(&mut self) {
        if self.phase == SessionPhase::Recording {
            if let Some(t) = self.play_started.take() {
                self.played_before += t.elapsed().as_secs_f64();
            }
            self.phase = SessionPhase::Paused;
            self.view.phase = SessionPhase::Paused;
            self.view.elapsed = self.played_before;
        }
    }

    /// Opens the file and begins recording immediately (arm + play). Used by tests and as a
    /// convenience; the file is still created lazily on the first `poll_at`.
    pub fn start(&mut self, path: PathBuf, rate: u32, reserve: u64, started_label: String) -> Result<(), String> {
        self.arm(path, rate, reserve);
        self.play(started_label);
        Ok(())
    }

    /// Recorded seconds so far: the played time, with any paused spans left out.
    fn recorded_elapsed(&self) -> f64 {
        self.played_before + self.play_started.map_or(0.0, |t| t.elapsed().as_secs_f64())
    }

    /// Creates the shared sample buffer the caller hands to `Output::set_tap`, keeping a clone
    /// for its own `poll` to drain.
    pub fn attach_tap(&mut self) -> Arc<Mutex<Vec<f32>>> {
        let buf = Arc::new(Mutex::new(Vec::with_capacity(TAP_ROOM)));
        self.tap = Some(buf.clone());
        buf
    }

    /// The real-time wrapper, called every frame: drains the tap, reads the output's rate,
    /// computes elapsed wall-clock time and the free space on the recording's disk, then
    /// drives `poll_at`. On a clean/auto stop it releases the engine tap.
    pub fn poll(&mut self, output: Option<&deckard_engine::output::Output>) {
        if !self.active() {
            return;
        }
        let Some(output) = output else { return };
        let rate = output.rate;
        let new = self.drain_tap();
        let elapsed = self.recorded_elapsed();
        let free = self.free_throttled();
        if self.poll_at(elapsed, rate, &new, free) {
            output.set_tap(None);
        }
    }

    /// Takes what the engine's tap has gathered (interleaved stereo), as frames. A buffer
    /// with room is left in its place: the audio thread fills it, and should not have to
    /// allocate one from nothing every frame.
    fn drain_tap(&mut self) -> Vec<[f32; 2]> {
        let Some(tap) = &self.tap else { return Vec::new() };
        let raw = std::mem::replace(&mut *tap.lock().unwrap(), Vec::with_capacity(TAP_ROOM));
        raw.as_chunks::<2>().0.to_vec()
    }

    /// Free space on the recording's disk, re-read from the OS at most about once a second
    /// and otherwise served from the cached value (a `statvfs` every display frame is waste).
    fn free_throttled(&mut self) -> u64 {
        let now = Instant::now();
        if self.free_checked.is_some_and(|t| now.duration_since(t).as_secs_f64() < 1.0) {
            return self.free_cache;
        }
        let free = self.view.path.as_ref().map_or(u64::MAX, |p| free_bytes(p.parent().unwrap_or(p)).unwrap_or(u64::MAX));
        self.free_cache = free;
        self.free_checked = Some(now);
        free
    }

    /// The testable core. Writes `new`, after the silence for a gap in which the engine
    /// delivered nothing (see `GAP_SECONDS`), so that the file reaches `floor(elapsed * rate)`
    /// frames less the lag it had while audio flowed. Returns `true` when it just auto- or
    /// clean-stopped.
    pub fn poll_at(&mut self, elapsed: f64, rate: u32, new: &[[f32; 2]], free: u64) -> bool {
        match self.phase {
            SessionPhase::Idle => return false,
            // Armed or paused: keep the meters and waveform alive, but write nothing and let
            // no time pass — so paused spans never reach the file.
            SessionPhase::Armed | SessionPhase::Paused => {
                self.update_meters(new, free);
                return false;
            }
            SessionPhase::Recording => {}
        }

        // The file is created on the first write, at the live rate; a later rate change (the
        // device switched under us) then clean-stops.
        match self.run.as_ref() {
            Some(run) if rate != run.rate => return self.clean_stop(StopReason::RateChange),
            Some(_) => {}
            None => {
                let Some(path) = self.view.path.clone() else { return false };
                match SessionWav::create(&path, rate) {
                    Ok(wav) => self.run = Some(SessionRun { wav, rate, written: 0, fed_at: 0.0, lag: 0 }),
                    Err(e) => return self.fail(e),
                }
            }
        }

        // Target total frames so the file length equals the recorded (played) time.
        let target = (elapsed * rate as f64) as u64;

        // Do the pad/write work against the `run` borrow, capturing any write error, then drop
        // the borrow before calling `self.fail`/`self.clean_stop`. Returns whether the WAV
        // ceiling was reached mid-write (so we clean-stop with Ceiling below).
        let write_result: Result<bool, String> = {
            let run = self.run.as_mut().expect("run present: checked above");
            // Frames that still fit under the 4 GB ceiling (6 bytes per stereo frame).
            let ceiling_frames = WAV_CEILING_BYTES / 6;
            // Silent frames to keep length true, but clamped so a clock jump (e.g. a laptop
            // sleeping mid-record) can't force a multi-GB allocation in one poll: the next
            // poll catches up, or the ceiling clean-stops first.
            let max_pad = (rate as u64).saturating_mul(MAX_PAD_SECONDS);
            // What is missing since audio last arrived: the shortfall against the wall clock
            // beyond the lag the file had then. Only a real gap is padded (no audio for
            // `GAP_SECONDS`, and that much missing): late blocks and a device clock slower
            // than the computer's leave the audio as it came, with nothing spliced in.
            let short = target as i64 - (run.written + new.len() as u64) as i64;
            let missing = (short - run.lag).max(0) as u64;
            let gap = elapsed - run.fed_at > GAP_SECONDS && missing as f64 > GAP_SECONDS * rate as f64;
            let want_pad = if gap { missing } else { 0 };
            let pad = want_pad.min(max_pad);
            // Audio arrived, and nothing is left to catch up: this is the lag from here on.
            if !new.is_empty() && pad == want_pad {
                run.fed_at = elapsed;
                run.lag = short - pad as i64;
            }
            (|| -> Result<bool, String> {
                let mut room = ceiling_frames.saturating_sub(run.written);
                // Silent pad first (to keep the timeline), never past the ceiling.
                let pad = pad.min(room);
                if pad > 0 {
                    run.wav.write(&vec![[0.0, 0.0]; pad as usize])?;
                    run.written += pad;
                    room -= pad;
                }
                if room == 0 {
                    return Ok(true); // ceiling hit while padding
                }
                // Then the new signal, also capped at the ceiling.
                let take = (new.len() as u64).min(room) as usize;
                if take > 0 {
                    run.wav.write(&new[..take])?;
                    run.written += take as u64;
                }
                Ok(take < new.len()) // couldn't fit all of `new`: ceiling reached
            })()
        };
        let hit_ceiling = match write_result {
            Ok(hit) => hit,
            Err(e) => return self.fail(e),
        };

        // Update the view for the UI.
        self.update_meters(new, free);
        self.view.elapsed = elapsed;
        self.view.bytes = self.run.as_ref().expect("run present").wav.data_bytes();

        // A write that reached the 4 GB ceiling: finalize the (valid) file now.
        if hit_ceiling {
            return self.clean_stop(StopReason::Ceiling);
        }
        // Auto-stop when the disk is down to the reserve or the WAV is full.
        if let Some(reason) = autostop_reason(self.view.bytes, free, self.view.reserve) {
            return self.clean_stop(reason);
        }
        false
    }

    /// Updates the live meters, peak hold and scrolling waveform from the latest frames.
    fn update_meters(&mut self, new: &[[f32; 2]], free: u64) {
        let peak = new.iter().fold((0f32, 0f32), |p, f| (p.0.max(f[0].abs()), p.1.max(f[1].abs())));
        self.view.free = free;
        self.view.meter = peak;
        self.view.peak = (self.view.peak.0.max(peak.0), self.view.peak.1.max(peak.1));
        if !new.is_empty() {
            self.view.wave.push_back(peak);
            while self.view.wave.len() > WAVE_BUCKETS {
                self.view.wave.pop_front();
            }
        }
    }

    /// Manual stop: finalize, clear `run`, return the file path (or `None` if nothing was
    /// recorded — armed but never played leaves no file behind).
    pub fn stop(&mut self) -> Option<PathBuf> {
        self.abort(StopReason::Manual)
    }

    /// Stop with an explicit reason, finalizing the file. Used when the caller must stop the
    /// session for a reason the `poll` loop can't see — e.g. the audio `Output` is being torn
    /// down and rebuilt (a device or buffer change) at the same sample rate, which no
    /// `RateChange` would catch. Returns the file path if a recording was in flight.
    pub fn abort(&mut self, reason: StopReason) -> Option<PathBuf> {
        if self.phase == SessionPhase::Idle {
            return None;
        }
        // A file exists only once recording has actually written to one.
        let path = self.run.is_some().then(|| self.view.path.clone()).flatten();
        self.clean_stop(reason);
        path
    }

    /// Finalizes the `SessionWav` (if any), records the stop reason, returns to idle. Returns `true`.
    fn clean_stop(&mut self, reason: StopReason) -> bool {
        if let Some(run) = self.run.take() {
            if let Err(e) = run.wav.finalize() {
                if self.view.error.is_none() {
                    self.view.error = Some(e);
                }
            }
        }
        self.view.stopped = Some(reason);
        self.phase = SessionPhase::Idle;
        self.view.phase = SessionPhase::Idle;
        self.play_started = None;
        true
    }

    /// A write failed: record the error, then clean-stop (keeping a valid, finalized file).
    fn fail(&mut self, e: String) -> bool {
        self.view.error = Some(e);
        self.clean_stop(StopReason::WriteError)
    }
}

impl Default for SessionRecorder {
    fn default() -> SessionRecorder {
        SessionRecorder::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_default_filename() {
        assert_eq!(session_filename(Some("NightSet"), "2026-11-16-2104"), "session-NightSet-2026-11-16-2104.wav");
        assert_eq!(session_filename(None, "2026-11-16-2104"), "session-2026-11-16-2104.wav");
        assert_eq!(session_filename(Some(""), "2026-11-16-2104"), "session-2026-11-16-2104.wav");
        // Filesystem-unfriendly characters in the project name are made safe.
        assert_eq!(session_filename(Some("Night Set/2"), "2026-11-16-2104"), "session-Night-Set-2-2026-11-16-2104.wav");
    }

    #[test]
    fn bytes_per_second_is_rate_times_six() {
        assert_eq!(bytes_per_second(48_000), 288_000); // 2ch * 3 bytes
    }

    #[test]
    fn seconds_available_subtracts_reserve_and_caps_at_ceiling() {
        // 2 GiB free, 1 GiB reserve, 48k: ~1 GiB usable / 288000 ~ 3728 s.
        let s = seconds_available(2 * 1024 * 1024 * 1024, 1024 * 1024 * 1024, 48_000);
        assert!((3700..3760).contains(&s), "got {s}");
        // Huge free space is capped by the 4 GB WAV ceiling (~13 889 s).
        assert_eq!(seconds_available(u64::MAX, 0, 48_000), WAV_CEILING_BYTES / 288_000);
    }

    #[test]
    fn cannot_start_at_or_below_reserve() {
        assert!(!can_start(1024 * 1024 * 1024, 1024 * 1024 * 1024));
        assert!(can_start(1024 * 1024 * 1024 + 1, 1024 * 1024 * 1024));
    }

    #[test]
    fn autostop_reason_fires_on_disk_then_ceiling() {
        assert_eq!(autostop_reason(1000, 500, 1000), Some(StopReason::Disk)); // free <= reserve
        assert_eq!(autostop_reason(WAV_CEILING_BYTES, u64::MAX, 0), Some(StopReason::Ceiling));
        assert_eq!(autostop_reason(1000, u64::MAX, 0), None);
    }

    #[test]
    fn free_bytes_reads_the_temp_dir() {
        let got = free_bytes(&std::env::temp_dir());
        assert!(got.is_some_and(|b| b > 0));
    }

    #[test]
    fn writes_a_readable_24bit_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.wav");
        let mut w = SessionWav::create(&path, 48_000).unwrap();
        let frames = [[0.5f32, -0.5], [1.0, -1.0], [0.0, 0.0]];
        w.write(&frames).unwrap();
        assert_eq!(w.data_bytes(), 3 /*frames*/ * 2 /*ch*/ * 3 /*bytes*/);
        w.finalize().unwrap();
        let r = hound::WavReader::open(&path).unwrap();
        assert_eq!(r.spec().bits_per_sample, 24);
        assert_eq!(r.spec().channels, 2);
        assert_eq!(r.spec().sample_rate, 48_000);
        assert_eq!(r.len(), 6); // samples = 3 frames * 2 ch
    }

    #[test]
    fn file_is_valid_wav_before_finalize() {
        // Flush-as-you-go: after writing, the file already opens as a valid WAV.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.wav");
        let mut w = SessionWav::create(&path, 48_000).unwrap();
        w.write(&vec![[0.1f32, 0.1]; 48_000]).unwrap(); // ~1s, forces a flush
        assert!(hound::WavReader::open(&path).is_ok());
        w.finalize().unwrap();
    }

    // Helpers in the test module: a recorder started on a temp file.
    fn started(dir: &std::path::Path) -> SessionRecorder {
        let mut s = SessionRecorder::new();
        s.start(dir.join("s.wav"), 48_000, 1_000_000_000, "now".into()).unwrap();
        s
    }

    #[test]
    fn silence_fills_when_no_frames_arrive() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = started(dir.path());
        // 2 seconds elapse with no audio at all.
        s.poll_at(2.0, 48_000, &[], u64::MAX);
        // The file must hold ~2s of (silent) frames: length tracks wall-clock, not input.
        assert!(s.view.bytes >= 2 * bytes_per_second(48_000) - 6);
        assert!(s.recording());
    }

    #[test]
    fn played_audio_lands_near_its_wall_clock_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = started(dir.path());
        s.poll_at(1.0, 48_000, &[], u64::MAX); // 1s silence
        s.poll_at(1.5, 48_000, &[[0.8, 0.8]; 24_000], u64::MAX); // 0.5s of signal at ~1s mark
        s.stop();
        let mut r = hound::WavReader::open(dir.path().join("s.wav")).unwrap();
        let samples: Vec<i32> = r.samples::<i32>().map(Result::unwrap).collect();
        // First second (96_000 samples) is silence; signal appears after it.
        assert!(samples[..96_000].iter().all(|&x| x == 0));
        assert!(samples[96_000..].iter().any(|&x| x != 0));
    }

    /// The signal of the delivery tests: never zero, so every zero in the file was padded.
    const TONE: [f32; 2] = [0.5, 0.5];

    /// The frames of `path`, as (how many, how many of them silent).
    fn frames_and_silent(path: &std::path::Path) -> (usize, usize) {
        let mut r = hound::WavReader::open(path).unwrap();
        let samples: Vec<i32> = r.samples::<i32>().map(Result::unwrap).collect();
        (samples.len() / 2, samples.chunks(2).filter(|f| f[0] == 0 && f[1] == 0).count())
    }

    /// Audio arrives in blocks, some of them late, from a device whose clock runs slower
    /// than the computer's (by far more than a real one, to trail it by over a second
    /// here): nothing is missing, so no silence is spliced into it.
    #[test]
    fn late_blocks_and_a_slow_device_clock_insert_no_silence() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = started(dir.path());
        let (block, device_rate) = (512u64, 48_000.0 * 0.95);
        let (mut delivered, mut held) = (0u64, 0u64);
        for poll in 1..=(25 * 60u64) {
            let elapsed = poll as f64 / 60.0;
            // What the device has produced by now, in whole blocks; every seventh poll its
            // newest block comes late (with the next poll).
            let mut ready = (elapsed * device_rate) as u64 / block * block;
            if poll % 7 == 0 {
                ready = ready.saturating_sub(block).max(delivered);
            }
            let new = vec![TONE; (ready - delivered) as usize];
            delivered = ready;
            held = held.max((elapsed * 48_000.0) as u64 - delivered);
            assert!(!s.poll_at(elapsed, 48_000, &new, u64::MAX));
        }
        assert!(held > 48_000, "the file trailed the wall clock by over a second ({held} frames)");
        s.stop();
        let (frames, silent) = frames_and_silent(&dir.path().join("s.wav"));
        assert_eq!(silent, 0, "no silence in audio that kept coming");
        assert_eq!(frames as u64, delivered, "the file is the audio delivered, and nothing else");
    }

    /// The engine stops delivering for three seconds in the middle of a recording whose
    /// device clock is slow: the gap is filled with silence, and only the gap.
    #[test]
    fn a_real_gap_is_padded_and_the_drift_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = started(dir.path());
        let device_rate = 48_000.0 * 0.99;
        let (mut delivered, mut device_time) = (0u64, 0.0f64);
        for poll in 1..=(8 * 60u64) {
            let elapsed = poll as f64 / 60.0;
            // Seconds 2 to 5: nothing arrives (and the device's clock doesn't run).
            let stalled = (2.0..5.0).contains(&elapsed);
            if !stalled {
                device_time += 1.0 / 60.0;
            }
            let ready = (device_time * device_rate) as u64;
            let new = vec![TONE; (ready - delivered) as usize];
            delivered = ready;
            s.poll_at(elapsed, 48_000, &new, u64::MAX);
        }
        s.stop();
        let (frames, silent) = frames_and_silent(&dir.path().join("s.wav"));
        assert_eq!(frames as u64 - silent as u64, delivered, "all the audio is there");
        // Three seconds, less at most the half second a gap's end may go unfilled.
        assert!((120_000..=144_000 + 800).contains(&silent), "the gap was filled with {silent} silent frames");
    }

    /// The free space is asked of the system once, then kept for a while: also while armed,
    /// before there is a file.
    #[test]
    fn free_space_is_cached_while_armed() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = SessionRecorder::new();
        s.arm(dir.path().join("s.wav"), 48_000, 0);
        assert!(s.free_checked.is_none());
        let free = s.free_throttled();
        let asked = s.free_checked.expect("asked once");
        assert_eq!(s.free_cache, free);
        s.free_cache = 7;
        assert_eq!(s.free_throttled(), 7, "served from the cache within the second");
        assert_eq!(s.free_checked, Some(asked));
        // Arming again starts afresh.
        s.arm(dir.path().join("t.wav"), 48_000, 0);
        assert!(s.free_checked.is_none());
    }

    /// The tap's buffer is handed over with room in it, and left with room when drained.
    #[test]
    fn the_tap_buffer_keeps_its_room() {
        let mut s = SessionRecorder::new();
        let tap = s.attach_tap();
        assert!(tap.lock().unwrap().capacity() >= TAP_ROOM);
        tap.lock().unwrap().extend([0.1, 0.2, 0.3, 0.4]);
        assert_eq!(s.drain_tap(), vec![[0.1, 0.2], [0.3, 0.4]]);
        assert!(tap.lock().unwrap().is_empty());
        assert!(tap.lock().unwrap().capacity() >= TAP_ROOM, "not a buffer the audio thread must grow from nothing");
        assert!(SessionRecorder::new().drain_tap().is_empty(), "no tap, no audio");
    }

    #[test]
    fn autostops_when_disk_hits_reserve() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = started(dir.path()); // reserve 1_000_000_000
        let stopped = s.poll_at(0.5, 48_000, &[[0.1, 0.1]; 100], 500_000_000); // free < reserve
        assert!(stopped);
        assert!(!s.recording());
        assert_eq!(s.view.stopped, Some(StopReason::Disk));
        assert!(hound::WavReader::open(dir.path().join("s.wav")).is_ok()); // finalized, valid
    }

    #[test]
    fn autostops_on_rate_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = started(dir.path());
        s.poll_at(0.1, 48_000, &[], u64::MAX); // first poll opens the file at 48 kHz
        let stopped = s.poll_at(0.5, 44_100, &[], u64::MAX); // device switched
        assert!(stopped);
        assert_eq!(s.view.stopped, Some(StopReason::RateChange));
        assert!(!s.recording());
    }

    #[test]
    fn arming_writes_nothing_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.wav");
        let mut s = SessionRecorder::new();
        s.arm(path.clone(), 48_000, 1_000_000_000);
        assert_eq!(s.phase(), SessionPhase::Armed);
        assert!(s.active() && !s.recording());
        // Metering runs while armed, but nothing is written and no file is created.
        s.poll_at(0.0, 48_000, &[[0.5, 0.4]; 1000], u64::MAX);
        assert_eq!(s.view.meter, (0.5, 0.4), "armed metering updates");
        assert_eq!(s.view.bytes, 0);
        assert!(!path.exists(), "no file yet");
        // Stopping while only armed leaves no file behind.
        assert_eq!(s.stop(), None);
        assert_eq!(s.phase(), SessionPhase::Idle);
        assert!(!path.exists(), "still no file after a stop from armed");
    }

    #[test]
    fn pausing_keeps_the_paused_audio_out_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.wav");
        let mut s = SessionRecorder::new();
        s.arm(path.clone(), 48_000, 1_000_000_000);
        s.play("now".into());
        s.poll_at(1.0, 48_000, &[[0.5, 0.5]; 48_000], u64::MAX); // ~1s recorded
        let after_first = s.view.bytes;
        assert!(after_first >= bytes_per_second(48_000) - 6);
        // Pause: polling while paused updates the meter but writes nothing.
        s.pause();
        assert_eq!(s.phase(), SessionPhase::Paused);
        s.poll_at(1.0, 48_000, &[[0.9, 0.9]; 48_000], u64::MAX);
        assert_eq!(s.view.bytes, after_first, "paused audio is not written");
        assert_eq!(s.view.meter, (0.9, 0.9), "paused metering still updates");
        // Resume: appends to the same file; the paused span is simply absent.
        s.play("now".into());
        assert_eq!(s.phase(), SessionPhase::Recording);
        s.poll_at(1.5, 48_000, &[[0.5, 0.5]; 24_000], u64::MAX); // recorded time 1.5s total
        assert!(s.view.bytes > after_first, "recording resumed into the same file");
        s.stop();
        // The file holds ~1.5s of recorded audio — the paused second is not in it.
        let r = hound::WavReader::open(&path).unwrap();
        let secs = r.len() as f64 / 2.0 / 48_000.0;
        assert!((1.4..1.6).contains(&secs), "file holds the recorded time only, got {secs}s");
    }

    #[test]
    fn abort_device_change_finalizes_and_records_reason() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = started(dir.path());
        s.poll_at(1.0, 48_000, &[[0.3, 0.3]; 24_000], u64::MAX); // half a second of signal
        let path = s.abort(StopReason::DeviceChange);
        assert_eq!(path, Some(dir.path().join("s.wav")), "abort returns the file path");
        assert!(!s.recording());
        assert_eq!(s.view.phase, SessionPhase::Idle);
        assert_eq!(s.view.stopped, Some(StopReason::DeviceChange));
        // The partial file was finalized and opens as a valid, non-empty WAV.
        let mut r = hound::WavReader::open(dir.path().join("s.wav")).unwrap();
        assert!(r.samples::<i32>().count() > 0);
    }

    #[test]
    fn clock_jump_past_ceiling_stops_cleanly_without_overrunning() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = started(dir.path()); // 48 kHz
        s.poll_at(0.0, 48_000, &[], u64::MAX); // open the file (lazily created on first poll)
        // Pretend the file is already just shy of the 4 GB ceiling, without writing 4 GB:
        // the test lives in this module, so it can position the frame counter directly.
        let ceiling_frames = WAV_CEILING_BYTES / 6;
        s.run.as_mut().unwrap().written = ceiling_frames - 100;
        // The machine slept: `elapsed` leaps hours past the ceiling in a single poll, with
        // some signal arriving too. Before the fix this padded multi-GB of silence and
        // overran the RIFF size; now it writes only up to the ceiling and clean-stops.
        let stopped = s.poll_at(100_000.0, 48_000, &[[0.5, 0.5]; 10_000], u64::MAX);
        assert!(stopped, "a jump past the ceiling clean-stops");
        assert!(!s.recording());
        assert_eq!(s.view.stopped, Some(StopReason::Ceiling));
        let path = dir.path().join("s.wav");
        assert!(std::fs::metadata(&path).unwrap().len() < WAV_CEILING_BYTES, "file stayed under the ceiling");
        assert!(hound::WavReader::open(&path).is_ok(), "file opens as a valid WAV");
    }

    #[test]
    fn autostops_at_ceiling() {
        // Drive the ceiling check without writing 4 GB: a tiny-ceiling variant.
        // (Implement poll_at to consult autostop_reason(view.bytes, free, reserve);
        //  this test lowers the pressure by asserting the decision via a short write
        //  once view.bytes is forced past WAV_CEILING_BYTES in a unit of autostop_reason.)
        assert_eq!(autostop_reason(WAV_CEILING_BYTES, u64::MAX, 0), Some(StopReason::Ceiling));
    }
}
