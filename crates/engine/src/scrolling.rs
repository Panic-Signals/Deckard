//! Session-only rolling capture of the rendered master output.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const PYRAMID: usize = 128;

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
}

impl Default for Scrolling {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            seconds: Mutex::new(120.0),
            data: Mutex::new(Data { left: Vec::new(), right: Vec::new(), total: 0, rate: 0, enabled: false }),
        }
    }
}

impl Scrolling {
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
                    data.left.clear();
                    data.right.clear();
                    data.total = 0;
                    data.rate = 0;
                }
                return;
            }
        }
        if changed && let Ok(mut data) = self.data.lock() {
            data.enabled = enabled;
            data.left.clear();
            data.right.clear();
            data.total = 0;
            data.rate = 0;
        }
    }

    pub fn push(&self, stereo: &[f32], frames: usize, rate: u32) {
        if !self.enabled.load(Ordering::Relaxed) || self.paused.load(Ordering::Relaxed) || frames == 0 {
            return;
        }
        let Ok(mut data) = self.data.try_lock() else { return };
        let seconds = self.seconds.try_lock().map_or(120.0, |s| *s);
        let capacity = ((seconds * rate as f32) as usize).max(PYRAMID).next_multiple_of(PYRAMID);
        if data.left.len() != capacity || data.rate != rate {
            data.left = vec![0.0; capacity];
            data.right = vec![0.0; capacity];
            data.total = 0;
            data.rate = rate;
        }
        for (frame, pair) in stereo.chunks_exact(2).take(frames).enumerate() {
            let index = ((data.total as usize + frame) % capacity) as usize;
            data.left[index] = pair[0];
            data.right[index] = pair[1];
        }
        data.total += frames as u64;
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn snapshot(&self, columns: usize) -> (Vec<(f32, f32)>, u64, u32) {
        let (total, rate) = match self.data.lock() {
            Ok(data) => (data.total, data.rate),
            Err(_) => return (vec![(0.0, 0.0); columns], 0, 0),
        };
        let available = self.available_frames();
        self.snapshot_range(total.saturating_sub(available), available, columns)
            .map_or((vec![(0.0, 0.0); columns], total, rate), |peaks| (peaks, total, rate))
    }

    pub fn available_frames(&self) -> u64 {
        let Ok(data) = self.data.lock() else { return 0 };
        data.total.min(data.left.len() as u64)
    }

    pub fn snapshot_range(&self, from: u64, len: u64, columns: usize) -> Option<Vec<(f32, f32)>> {
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
        let fade_frames = ((fade_ms.clamp(1, 10) as u64 * u64::from(rate)) / 1000) as usize;
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
