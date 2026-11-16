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
}
