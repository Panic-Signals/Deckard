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

/// Free space that must remain on the disk; recording stops before it is used up.
pub const DEFAULT_RESERVE_BYTES: u64 = 1024 * 1024 * 1024;

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

/// Why a recording ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Disk,
    Ceiling,
    RateChange,
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
    /// When `start` opened the file, so `poll` can keep the file's length equal to
    /// wall-clock elapsed time.
    started: Instant,
}

/// A snapshot of the recorder for the UI: everything a window needs to draw, and nothing
/// that outlives a stop. `Default` is the idle state (not recording, no file).
#[derive(Default)]
pub struct SessionView {
    pub recording: bool,
    pub elapsed: f64,
    pub started_label: String,
    pub meter: (f32, f32),
    pub peak: (f32, f32),
    pub bytes: u64,
    pub free: u64,
    pub reserve: u64,
    pub rate: u32,
    pub path: Option<PathBuf>,
    pub wave: VecDeque<(f32, f32)>,
    pub error: Option<String>,
    pub stopped: Option<StopReason>,
}

/// How many wave buckets to keep for the scrolling meter display.
const WAVE_BUCKETS: usize = 240;

/// Records the master output to a WAV file, filling silent gaps so the file's length always
/// equals wall-clock elapsed time. The real-time `poll` wrapper (drain tap, read rate, compute
/// elapsed) is wired in a later task; the testable core is `poll_at`.
pub struct SessionRecorder {
    pub view: SessionView,
    run: Option<SessionRun>,
    tap: Option<Arc<Mutex<Vec<f32>>>>,
}

impl SessionRecorder {
    pub fn new() -> SessionRecorder {
        SessionRecorder { view: SessionView::default(), run: None, tap: None }
    }

    /// Whether a recording is currently in flight.
    pub fn recording(&self) -> bool {
        self.run.is_some()
    }

    /// Opens the WAV file and arms the recorder. Does not touch any `Output`; the caller wires
    /// the tap via `attach_tap`.
    pub fn start(
        &mut self,
        path: PathBuf,
        rate: u32,
        reserve: u64,
        started_label: String,
    ) -> Result<(), String> {
        let wav = SessionWav::create(&path, rate)?;
        self.run = Some(SessionRun { wav, rate, written: 0, started: Instant::now() });
        self.view = SessionView {
            recording: true,
            rate,
            reserve,
            path: Some(path),
            started_label,
            ..SessionView::default()
        };
        Ok(())
    }

    /// Creates the shared sample buffer the caller hands to `Output::set_tap`, keeping a clone
    /// for its own `poll` to drain.
    pub fn attach_tap(&mut self) -> Arc<Mutex<Vec<f32>>> {
        let buf = Arc::new(Mutex::new(Vec::new()));
        self.tap = Some(buf.clone());
        buf
    }

    /// The real-time wrapper, called every frame: drains the tap, reads the output's rate,
    /// computes elapsed wall-clock time and the free space on the recording's disk, then
    /// drives `poll_at`. On a clean/auto stop it releases the engine tap.
    pub fn poll(&mut self, output: Option<&deckard_engine::output::Output>) {
        if !self.recording() {
            return;
        }
        let Some(output) = output else { return };
        let rate = output.rate;
        // The engine's tap holds interleaved stereo; chunk it back into frames.
        let new: Vec<[f32; 2]> = match &self.tap {
            Some(tap) => {
                let raw = std::mem::take(&mut *tap.lock().unwrap());
                raw.as_chunks::<2>().0.to_vec()
            }
            None => Vec::new(),
        };
        let elapsed = self.run.as_ref().map_or(0.0, |r| r.started.elapsed().as_secs_f64());
        let free = self
            .view
            .path
            .as_ref()
            .map_or(u64::MAX, |p| free_bytes(p.parent().unwrap_or(p)).unwrap_or(u64::MAX));
        if self.poll_at(elapsed, rate, &new, free) {
            output.set_tap(None);
        }
    }

    /// The testable core. Pads silent frames so the file reaches `floor(elapsed * rate)` total
    /// frames, then writes `new`. Returns `true` when it just auto- or clean-stopped.
    pub fn poll_at(&mut self, elapsed: f64, rate: u32, new: &[[f32; 2]], free: u64) -> bool {
        // A rate different from the one `start` opened at: the device switched under us.
        match self.run.as_ref() {
            None => return false,
            Some(run) if rate != run.rate => return self.clean_stop(StopReason::RateChange),
            Some(_) => {}
        }

        // Target total frames so the file length equals wall-clock elapsed.
        let target = (elapsed * rate as f64) as u64;

        // Do the pad/write work against the `run` borrow, capturing any write error, then drop
        // the borrow before calling `self.fail`/`self.clean_stop`.
        let write_result = {
            let run = self.run.as_mut().expect("run present: checked above");
            let have = run.written + new.len() as u64;
            let pad = target.saturating_sub(have); // silent frames to keep length true
            let mut write = || -> Result<(), String> {
                if pad > 0 {
                    run.wav.write(&vec![[0.0, 0.0]; pad as usize])?;
                }
                if !new.is_empty() {
                    run.wav.write(new)?;
                }
                run.written = run.written.max(have).max(target);
                Ok(())
            };
            write()
        };
        if let Err(e) = write_result {
            return self.fail(e);
        }

        // Update the view for the UI.
        let peak = new
            .iter()
            .fold((0f32, 0f32), |p, f| (p.0.max(f[0].abs()), p.1.max(f[1].abs())));
        self.view.elapsed = elapsed;
        self.view.bytes = self.run.as_ref().expect("run present").wav.data_bytes();
        self.view.free = free;
        self.view.meter = peak;
        self.view.peak = (self.view.peak.0.max(peak.0), self.view.peak.1.max(peak.1));
        if !new.is_empty() {
            self.view.wave.push_back(peak);
            while self.view.wave.len() > WAVE_BUCKETS {
                self.view.wave.pop_front();
            }
        }

        // Auto-stop when the disk is down to the reserve or the WAV is full.
        if let Some(reason) = autostop_reason(self.view.bytes, free, self.view.reserve) {
            return self.clean_stop(reason);
        }
        false
    }

    /// Manual stop: finalize, clear `run`, return the file path.
    pub fn stop(&mut self) -> Option<PathBuf> {
        if self.run.is_none() {
            return None;
        }
        let path = self.view.path.clone();
        self.clean_stop(StopReason::Manual);
        path
    }

    /// Finalizes the `SessionWav`, records the stop reason, drops `run`. Returns `true`.
    fn clean_stop(&mut self, reason: StopReason) -> bool {
        if let Some(run) = self.run.take() {
            if let Err(e) = run.wav.finalize() {
                if self.view.error.is_none() {
                    self.view.error = Some(e);
                }
            }
        }
        self.view.stopped = Some(reason);
        self.view.recording = false;
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
        s.poll_at(1.0, 48_000, &[], u64::MAX);                 // 1s silence
        s.poll_at(1.5, 48_000, &[[0.8, 0.8]; 24_000], u64::MAX); // 0.5s of signal at ~1s mark
        s.stop();
        let mut r = hound::WavReader::open(dir.path().join("s.wav")).unwrap();
        let samples: Vec<i32> = r.samples::<i32>().map(Result::unwrap).collect();
        // First second (96_000 samples) is silence; signal appears after it.
        assert!(samples[..96_000].iter().all(|&x| x == 0));
        assert!(samples[96_000..].iter().any(|&x| x != 0));
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
        let mut s = started(dir.path()); // opened at 48_000
        let stopped = s.poll_at(0.5, 44_100, &[], u64::MAX); // device switched
        assert!(stopped);
        assert_eq!(s.view.stopped, Some(StopReason::RateChange));
        assert!(!s.recording());
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
