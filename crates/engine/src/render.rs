//! The New Sampler's Rendered stretch engine (roadmap 1c): a zone's play range
//! stretched to the song's tempo, its Tune moving pitch and not length, made off the
//! audio thread and kept in [`Renders`] for the engine builder to swap in.

use crate::sample::Sample;
use deckard_stretch::Stretch;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Output frames stretched at a time.
const CHUNK: usize = 1024;
/// Renders kept at most, in frames (stereo f32: 8 bytes a frame), about 400 MB.
pub const MAX_FRAMES: usize = 50_000_000;

/// Frames `[a, b)` of `s` (clamped to the file) stretched to `time` times their length
/// (2: twice as long) and shifted `semitones` in pitch, with Signalsmith. The output is
/// `round(time · len)` frames, its first on the range's first (latency compensated).
/// Audio either side of the range in the file feeds the stretcher, silence past it.
pub fn stretched(s: &Sample, range: (usize, usize), time: f64, semitones: f64) -> Sample {
    let a = range.0.min(s.frames.len());
    let b = range.1.clamp(a, s.frames.len());
    let len = b - a;
    let time = time.clamp(0.05, 20.0);
    let out_len = (time * len as f64).round() as usize;
    if len == 0 || out_len == 0 || s.rate == 0 {
        return Sample { rate: s.rate, frames: Vec::new() };
    }
    let rate = 1.0 / time;
    let get = |i: f64| -> [f32; 2] {
        let i = i.floor();
        if i >= 0.0 && (i as usize) < s.frames.len() { s.frames[i as usize] } else { [0.0; 2] }
    };
    let mut st = Stretch::new(s.rate as f32, false);
    st.set_transpose(2f64.powf(semitones / 12.0) as f32);
    let (lat_in, lat_out) = st.latency();
    let room = (lat_in * 2).max(4096).max((CHUNK as f64 * rate).ceil() as usize + 2).max((lat_out as f64 * rate).ceil() as usize + 2);
    let (mut il, mut ir) = (vec![0f32; room], vec![0f32; room]);
    let (mut ol, mut or) = (vec![0f32; CHUNK.max(lat_out)], vec![0f32; CHUNK.max(lat_out)]);
    // Pre-roll ending the input latency past the start, then the output latency run
    // through and dropped (as the Audio plug-in starts over).
    let pre = (lat_in * 2).max(4096);
    for k in 0..pre {
        [il[k], ir[k]] = get(a as f64 + lat_in as f64 - pre as f64 + k as f64);
    }
    st.seek(&il[..pre], &ir[..pre], rate);
    let mut feed = a as f64 + lat_in as f64;
    let m = (lat_out as f64 * rate).round() as usize;
    for k in 0..m {
        [il[k], ir[k]] = get(feed + k as f64);
    }
    st.process(&il[..m], &ir[..m], &mut ol[..lat_out], &mut or[..lat_out]);
    feed += m as f64;
    let mut frames = Vec::with_capacity(out_len);
    while frames.len() < out_len {
        let n = CHUNK.min(out_len - frames.len());
        let start = feed.floor();
        let m = ((feed + n as f64 * rate).floor() - start).max(0.0) as usize;
        for k in 0..m {
            [il[k], ir[k]] = get(start + k as f64);
        }
        feed += n as f64 * rate;
        st.process(&il[..m], &ir[..m], &mut ol[..n], &mut or[..n]);
        frames.extend(ol[..n].iter().zip(&or[..n]).map(|(l, r)| [*l, *r]));
    }
    Sample { rate: s.rate, frames }
}

/// Which render: the source sample (by identity; [`Renders`] keeps it alive, so the
/// address isn't reused), its range, and the tempo and pitch it was made for
/// (hundredths of a BPM and of a semitone).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RenderKey {
    pub sample: usize,
    pub range: (usize, usize),
    pub tempo: i64,
    pub cents: i64,
}

/// A render to make: [`stretched`] of `sample` with these.
#[derive(Clone)]
pub struct RenderJob {
    pub key: RenderKey,
    pub sample: Arc<Sample>,
    pub time: f64,
    pub semitones: f64,
}

impl RenderJob {
    /// For zone range `range` of `sample`, in time at `bpm`, played at `tempo` and
    /// shifted `semitones`.
    pub fn new(sample: &Arc<Sample>, range: (usize, usize), bpm: f64, tempo: f64, semitones: f64) -> RenderJob {
        let key = RenderKey {
            sample: Arc::as_ptr(sample) as usize,
            range,
            tempo: (tempo * 100.0).round() as i64,
            cents: (semitones * 100.0).round() as i64,
        };
        RenderJob { key, sample: sample.clone(), time: bpm / (key.tempo as f64 / 100.0), semitones: key.cents as f64 / 100.0 }
    }

    /// Make it (slow: off the audio and UI threads).
    pub fn run(&self) -> Arc<Sample> {
        Arc::new(stretched(&self.sample, self.key.range, self.time, self.semitones))
    }
}

/// Renders made, and those the engine builder asked for since they were last taken.
#[derive(Default)]
pub struct Renders {
    /// The render, its source (kept alive), and when it was last used.
    made: HashMap<RenderKey, (Arc<Sample>, Arc<Sample>, u64)>,
    wanted: Vec<RenderJob>,
    /// Asked for and not made yet (each is asked for once).
    asked: HashSet<RenderKey>,
    clock: u64,
}

impl Renders {
    /// The render for `job`, if made; asked for (once) if not.
    pub fn get_or_want(&mut self, job: RenderJob) -> Option<Arc<Sample>> {
        self.clock += 1;
        if let Some((r, _, used)) = self.made.get_mut(&job.key) {
            *used = self.clock;
            return Some(r.clone());
        }
        if self.asked.insert(job.key) {
            self.wanted.push(job);
        }
        None
    }

    /// Whether `key`'s render is made (without asking for it).
    pub fn has(&self, key: &RenderKey) -> bool {
        self.made.contains_key(key)
    }

    /// The renders asked for since last time.
    pub fn take_wanted(&mut self) -> Vec<RenderJob> {
        std::mem::take(&mut self.wanted)
    }

    /// Keep a finished render; the least recently used go past [`MAX_FRAMES`].
    pub fn add(&mut self, job: &RenderJob, render: Arc<Sample>) {
        self.clock += 1;
        self.asked.remove(&job.key);
        self.made.insert(job.key, (render, job.sample.clone(), self.clock));
        let mut total: usize = self.made.values().map(|(r, _, _)| r.frames.len()).sum();
        while total > MAX_FRAMES && self.made.len() > 1 {
            let (oldest, n) =
                self.made.iter().min_by_key(|(_, (_, _, used))| *used).map(|(k, (r, _, _))| (*k, r.frames.len())).expect("not empty");
            self.made.remove(&oldest);
            total -= n;
        }
    }

    /// How many renders are kept.
    pub fn len(&self) -> usize {
        self.made.len()
    }

    pub fn is_empty(&self) -> bool {
        self.made.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    fn tone(hz: f32, seconds: f32) -> Sample {
        let n = (seconds * RATE as f32) as usize;
        let frames = (0..n)
            .map(|i| {
                let x = (i as f32 * hz * std::f32::consts::TAU / RATE as f32).sin() * 0.5;
                [x, x]
            })
            .collect();
        Sample { rate: RATE, frames }
    }

    /// Rising zero crossings per second.
    fn freq(x: &[[f32; 2]]) -> f32 {
        let n = x.windows(2).filter(|w| w[0][0] <= 0.0 && w[1][0] > 0.0).count();
        n as f32 * RATE as f32 / x.len() as f32
    }

    fn middle(x: &[[f32; 2]]) -> &[[f32; 2]] {
        &x[x.len() / 4..x.len() * 3 / 4]
    }

    #[test]
    fn keeps_pitch_and_changes_length() {
        let s = tone(220.0, 2.0);
        let r = stretched(&s, (0, s.frames.len()), 1.25, 0.0);
        assert_eq!(r.frames.len(), (s.frames.len() as f64 * 1.25).round() as usize);
        let f = freq(middle(&r.frames));
        assert!((f - 220.0).abs() < 4.4, "pitch kept: {f}");
    }

    #[test]
    fn moves_pitch_not_length() {
        let s = tone(220.0, 2.0);
        let r = stretched(&s, (0, s.frames.len()), 1.0, 12.0);
        assert_eq!(r.frames.len(), s.frames.len());
        let f = freq(middle(&r.frames));
        assert!((f - 440.0).abs() < 8.8, "an octave up: {f}");
    }

    /// Clicks every half second land where they were, times `time`.
    #[test]
    fn starts_on_time() {
        let mut s = Sample { rate: RATE, frames: vec![[0.0; 2]; RATE as usize * 2] };
        let at = [4410usize, 4410 + 22_050, 4410 + 44_100];
        for &i in &at {
            for k in 0..64 {
                s.frames[i + k] = [0.9 * (1.0 - k as f32 / 64.0); 2];
            }
        }
        let time = 1.5;
        let r = stretched(&s, (0, s.frames.len()), time, 0.0);
        let tol = (RATE as f64 * 0.003) as usize;
        for &i in &at {
            let want = (i as f64 * time) as usize;
            let win = &r.frames[want.saturating_sub(4 * tol)..(want + 4 * tol).min(r.frames.len())];
            let peak = win.iter().enumerate().max_by(|x, y| x.1[0].abs().total_cmp(&y.1[0].abs())).map(|(k, _)| k).unwrap()
                + want.saturating_sub(4 * tol);
            assert!(peak.abs_diff(want) <= tol, "click at {i} came out at {peak}, want {want}");
        }
    }

    #[test]
    fn a_range_and_empty_inputs() {
        let s = tone(220.0, 1.0);
        assert_eq!(stretched(&s, (1000, 1000), 2.0, 0.0).frames.len(), 0);
        assert_eq!(stretched(&s, (0, 1000), 2.0, 0.0).frames.len(), 2000);
        let past = stretched(&s, (40_000, 90_000), 1.0, 0.0);
        assert_eq!(past.frames.len(), s.frames.len() - 40_000, "clamped to the file");
    }

    #[test]
    fn the_render_store_is_capped() {
        let src = Arc::new(Sample { rate: RATE, frames: vec![[0.0; 2]; 10] });
        let mut r = Renders::default();
        let big = MAX_FRAMES / 3 + 1;
        for t in 0..5 {
            let job = RenderJob::new(&src, (0, 10), 100.0, 100.0 + t as f64, 0.0);
            assert!(r.get_or_want(job.clone()).is_none());
            r.add(&job, Arc::new(Sample { rate: RATE, frames: vec![[0.0; 2]; big] }));
        }
        assert_eq!(r.take_wanted().len(), 5);
        assert!(r.len() <= 2, "{} kept", r.len());
        // The newest is kept.
        let newest = RenderJob::new(&src, (0, 10), 100.0, 104.0, 0.0);
        assert!(r.get_or_want(newest).is_some());
        assert!(r.take_wanted().is_empty());
    }
}
