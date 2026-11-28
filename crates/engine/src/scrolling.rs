//! Session-only rolling capture of the rendered master output.

use std::path::Path;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

const PYRAMID: usize = 128;
const QUEUE_SECONDS: usize = 4;

struct CaptureQueue {
    samples: Vec<AtomicU32>,
    read: AtomicUsize,
    written: AtomicUsize,
}

impl CaptureQueue {
    fn new(rate: u32) -> Self {
        let frames = (rate as usize * QUEUE_SECONDS).max(PYRAMID).next_multiple_of(PYRAMID);
        Self { samples: (0..frames * 2).map(|_| AtomicU32::new(0)).collect(), read: AtomicUsize::new(0), written: AtomicUsize::new(0) }
    }

    fn push(&self, stereo: &[f32], frames: usize) {
        let capacity = self.samples.len() / 2;
        let read = self.read.load(Ordering::Acquire);
        let written = self.written.load(Ordering::Relaxed);
        let available = capacity.saturating_sub(written.saturating_sub(read));
        let count = frames.min(available);
        for frame in 0..count {
            let index = (written + frame) % capacity * 2;
            self.samples[index].store(stereo[frame * 2].to_bits(), Ordering::Relaxed);
            self.samples[index + 1].store(stereo[frame * 2 + 1].to_bits(), Ordering::Relaxed);
        }
        self.written.store(written + count, Ordering::Release);
    }

    fn reset(&self) {
        let written = self.written.load(Ordering::Acquire);
        self.read.store(written, Ordering::Release);
    }

    fn drain(&self, data: &mut Data) {
        let capacity = self.samples.len() / 2;
        let read = self.read.load(Ordering::Relaxed);
        let written = self.written.load(Ordering::Acquire);
        for frame in read..written {
            let index = frame % capacity * 2;
            let target = (data.total as usize) % data.left.len();
            data.left[target] = f32::from_bits(self.samples[index].load(Ordering::Relaxed));
            data.right[target] = f32::from_bits(self.samples[index + 1].load(Ordering::Relaxed));
            data.total += 1;
        }
        self.read.store(written, Ordering::Release);
    }
}

struct Data {
    left: Vec<f32>,
    right: Vec<f32>,
    total: u64,
    rate: u32,
    enabled: bool,
}

pub struct Scrolling {
    enabled: AtomicBool,
    paused: AtomicBool,
    seconds: Mutex<f32>,
    data: Mutex<Data>,
    queue: OnceLock<CaptureQueue>,
}

impl Default for Scrolling {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            seconds: Mutex::new(120.0),
            data: Mutex::new(Data { left: Vec::new(), right: Vec::new(), total: 0, rate: 0, enabled: false }),
            queue: OnceLock::new(),
        }
    }
}

impl Scrolling {
    fn capacity(seconds: f32, rate: u32) -> usize {
        ((seconds * rate as f32) as usize).max(PYRAMID).next_multiple_of(PYRAMID)
    }

    /// Prepare the rolling storage before the audio callback starts using it.
    pub fn prepare(&self, rate: u32) {
        let Ok(seconds) = self.seconds.lock().map(|seconds| *seconds) else { return };
        let Ok(mut data) = self.data.lock() else { return };
        let capacity = Self::capacity(seconds, rate);
        if data.left.len() == capacity && data.rate == rate {
            return;
        }
        data.left = vec![0.0; capacity];
        data.right = vec![0.0; capacity];
        data.total = 0;
        data.rate = rate;
        let _ = self.queue.set(CaptureQueue::new(rate));
    }

    pub fn sync(&self, enabled: bool, seconds: f32) {
        let changed = self.enabled.swap(enabled, Ordering::Relaxed) != enabled;
        if !enabled {
            self.paused.store(false, Ordering::Relaxed);
        }
        if let Ok(mut wanted) = self.seconds.lock() {
            let seconds = seconds.max(1.0);
            if (*wanted - seconds).abs() > f32::EPSILON {
                *wanted = seconds;
                if let Ok(mut data) = self.data.lock() {
                    let capacity = Self::capacity(seconds, data.rate);
                    data.left.clear();
                    data.right.clear();
                    if data.rate != 0 {
                        data.left.resize(capacity, 0.0);
                        data.right.resize(capacity, 0.0);
                    }
                    data.total = 0;
                    if let Some(queue) = self.queue.get() {
                        queue.reset();
                    }
                }
                return;
            }
        }
        if changed && let Ok(mut data) = self.data.lock() {
            data.enabled = enabled;
            data.total = 0;
            if let Some(queue) = self.queue.get() {
                queue.reset();
            }
        }
    }

    pub fn push(&self, stereo: &[f32], frames: usize, rate: u32) {
        if !self.enabled.load(Ordering::Relaxed) || self.paused.load(Ordering::Relaxed) || frames == 0 {
            return;
        }
        let Some(queue) = self.queue.get() else { return };
        if queue.samples.len() / 2 < frames || rate == 0 {
            return;
        }
        queue.push(stereo, frames);
    }

    fn drain(&self) {
        let Some(queue) = self.queue.get() else { return };
        let Ok(mut data) = self.data.try_lock() else { return };
        if data.left.is_empty() {
            return;
        }
        queue.drain(&mut data);
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn snapshot(&self, columns: usize) -> (Vec<(f32, f32)>, u64, u32) {
        self.drain();
        let (total, rate) = match self.data.lock() {
            Ok(data) => (data.total, data.rate),
            Err(_) => return (vec![(0.0, 0.0); columns], 0, 0),
        };
        let available = self.available_frames();
        self.snapshot_range(total.saturating_sub(available), available, columns)
            .map_or((vec![(0.0, 0.0); columns], total, rate), |peaks| (peaks, total, rate))
    }

    pub fn available_frames(&self) -> u64 {
        self.drain();
        let Ok(data) = self.data.lock() else { return 0 };
        data.total.min(data.left.len() as u64)
    }

    pub fn snapshot_range(&self, from: u64, len: u64, columns: usize) -> Option<Vec<(f32, f32)>> {
        self.drain();
        let Ok(data) = self.data.lock() else { return None };
        if data.left.is_empty() || columns == 0 {
            return Some(vec![(0.0, 0.0); columns]);
        }
        let available = data.total.min(data.left.len() as u64);
        let first = data.total.saturating_sub(available);
        let from = from.max(first).min(data.total);
        let len = len.min(data.total.saturating_sub(from));
        let mut out = vec![(0.0, 0.0); columns];
        for (column, peak) in out.iter_mut().enumerate() {
            let start = from + len * column as u64 / columns as u64;
            if start >= data.total {
                continue;
            }
            let end = (from + len * (column as u64 + 1) / columns as u64).max(start + 1);
            let mut lo = 0.0f32;
            let mut hi = 0.0f32;
            let end = end.min(data.total);
            let count = end.saturating_sub(start);
            let samples = count.min(64).max(1);
            for sample in 0..samples {
                let frame = start + count * sample / samples;
                let index = (frame as usize) % data.left.len();
                lo = lo.min(data.left[index].min(data.right[index]));
                hi = hi.max(data.left[index].max(data.right[index]));
            }
            *peak = (lo, hi);
        }
        Some(out)
    }

    pub fn clear(&self) {
        self.drain();
        if let Ok(mut data) = self.data.lock() {
            data.left.clear();
            data.right.clear();
            data.total = 0;
            data.rate = 0;
        }
    }

    /// Render an absolute frame range to a temporary stereo WAV for drag-out.
    /// The capture remains session-only; the file is merely a hand-off to the UI.
    pub fn export_wav(&self, path: &Path, from: u64, len: u64, remove_silence: bool, fade_ms: u32) -> Result<(), String> {
        self.drain();
        let data = self.data.lock().map_err(|_| "scrolling sampler is busy".to_string())?;
        if data.left.is_empty() || data.rate == 0 || len == 0 {
            return Err("the selection is empty".into());
        }
        let first = data.total.saturating_sub(data.left.len() as u64);
        let start = from.max(first).min(data.total);
        let end = start.saturating_add(len).min(data.total);
        if end <= start {
            return Err("the selection is no longer in the capture buffer".into());
        }
        let mut samples = Vec::with_capacity((end - start) as usize * 2);
        let rate = data.rate;
        for frame in start..end {
            let index = frame as usize % data.left.len();
            samples.push((data.left[index], data.right[index]));
        }
        drop(data);

        if remove_silence {
            let threshold = 0.001;
            let first_live = samples.iter().position(|(l, r)| l.abs().max(r.abs()) > threshold).unwrap_or(0);
            let last_live = samples.iter().rposition(|(l, r)| l.abs().max(r.abs()) > threshold).map_or(samples.len(), |i| i + 1);
            if first_live < last_live {
                samples = samples[first_live..last_live].to_vec();
            }
        }
        if !samples.iter().any(|(l, r)| l.abs().max(r.abs()) > 0.001) {
            return Err("the selection is silent".into());
        }
        let fade_frames = ((fade_ms.min(10) as u64 * u64::from(rate)) / 1000) as usize;
        let fade_frames = fade_frames.min(samples.len() / 2);
        for i in 0..fade_frames {
            let gain = i as f32 / fade_frames.max(1) as f32;
            samples[i].0 *= gain;
            samples[i].1 *= gain;
            let end = samples.len() - 1 - i;
            samples[end].0 *= gain;
            samples[end].1 *= gain;
        }
        let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut writer = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
        for (left, right) in samples {
            writer.write_sample(left.clamp(-1.0, 1.0)).map_err(|e| e.to_string())?;
            writer.write_sample(right.clamp(-1.0, 1.0)).map_err(|e| e.to_string())?;
        }
        writer.finalize().map_err(|e| e.to_string())
    }
}
