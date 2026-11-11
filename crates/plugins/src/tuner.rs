//! Tuner: an effect that shows the note it hears (a guitar, a voice, anything) and how
//! many cents off it is, passing the sound through untouched (or muted, to tune quietly).
//! It listens with Zhora's pitch detection, on a thread of its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use crate::Transport;
use crate::native::{Class, Engine, Page, Param, Readout, TuneState};
use crate::replicant::step;
use crate::zhora::pitch::{Pitch, WINDOW, detect};
use crate::zhora::{HOP, MIN_RMS, detector_rate};

pub static CLASS: Class = Class {
    cid: *b"DECKARD:Tuner001",
    name: "Tuner",
    instrument: false,
    params: PARAMS,
    pages: PAGES,
    make: |sr| Box::new(Tuner::new(sr)),
};

/// Whether `id` (a saved `plugin_id`) is the Tuner.
pub fn is_tuner(id: &str) -> bool {
    id == CLASS.plugin_id()
}

// Parameter ids (saved in projects: never renumber).
/// The pitch of A4, 430 to 450 Hz in steps of 1.
pub const REFERENCE: u32 = 1;
/// Off, or on: the sound stops here while tuning.
pub const MUTE: u32 = 2;

/// The lowest Reference, and its steps.
const REFERENCE_LOW: f64 = 430.0;
const REFERENCE_STEPS: usize = 20;

static PARAMS: &[Param] = &[
    Param { id: REFERENCE, name: "Reference", short: "A4", default: 0.5, steps: REFERENCE_STEPS as i32 },
    Param { id: MUTE, name: "Mute", short: "MUTE", default: 0.0, steps: 1 },
];

static PAGES: &[Page] = &[Page { name: "TUNER", knobs: &[(REFERENCE, "TUNER"), (MUTE, "")] }];

/// The tuner stops showing a pitch this long after the last detection.
const STALE_S: f64 = 0.25;

/// The pitch of A4 a Reference value sets.
pub fn reference_hz(v: f64) -> f64 {
    REFERENCE_LOW + step(v, REFERENCE_STEPS) as f64
}

/// The last detection, shared with the analysis thread: hz bits over clarity bits, 0
/// for none, and a count of detections.
#[derive(Default)]
struct Shared {
    reading: AtomicU64,
    seq: AtomicU32,
}

impl Shared {
    fn publish(&self, reading: Option<Pitch>) {
        let bits = reading.map_or(0, |p| ((p.hz.to_bits() as u64) << 32) | p.clarity.to_bits() as u64);
        self.reading.store(bits, Ordering::Relaxed);
        self.seq.fetch_add(1, Ordering::Release);
    }

    fn reading(&self) -> Option<f32> {
        let bits = self.reading.load(Ordering::Relaxed);
        (bits != 0).then(|| f32::from_bits((bits >> 32) as u32))
    }
}

/// The analysis thread: a detection every hop over the last `WINDOW` samples. It ends
/// when the engine drops its end of `blocks`.
fn worker(sample_rate: f32, blocks: Receiver<[f32; HOP]>, shared: Arc<Shared>) {
    let (rate, factor) = detector_rate(sample_rate);
    let shift = HOP / factor;
    let mut window = vec![0f32; WINDOW];
    let (mut have, mut gathered, mut power) = (0, 0, 0.0f32);
    while let Ok(block) = blocks.recv() {
        window.copy_within(shift.., 0);
        for (slot, chunk) in window[WINDOW - shift..].iter_mut().zip(block.chunks_exact(factor)) {
            *slot = chunk.iter().sum::<f32>() / factor as f32;
        }
        have = (have + shift).min(WINDOW);
        power += block.iter().map(|s| s * s).sum::<f32>();
        gathered += 1;
        if gathered < factor {
            continue;
        }
        let rms = (power / (HOP * factor) as f32).sqrt();
        (gathered, power) = (0, 0.0);
        // Written so that a NaN gives no pitch.
        shared.publish(if have == WINDOW && rms >= MIN_RMS { detect(&window, rate) } else { None });
    }
}

pub struct Tuner {
    sr: f64,
    shared: Arc<Shared>,
    blocks: SyncSender<[f32; HOP]>,
    block: [f32; HOP],
    filled: usize,
    reference: f64,
    mute: bool,
    /// The detection count last seen, and frames since it moved.
    seen_seq: u32,
    since_seen: u64,
}

impl Tuner {
    pub fn new(sr: f64) -> Tuner {
        let (blocks, block_rx) = sync_channel(16);
        let shared = Arc::new(Shared::default());
        let for_worker = shared.clone();
        let rate = sr as f32;
        std::thread::Builder::new()
            .name("tuner".into())
            .spawn(move || worker(rate, block_rx, for_worker))
            .expect("start the Tuner's thread");
        Tuner { sr, shared, blocks, block: [0.0; HOP], filled: 0, reference: 440.0, mute: false, seen_seq: 0, since_seen: 0 }
    }

    /// The note nearest `hz` and the cents above it, A4 being `self.reference`.
    fn note_of(&self, hz: f32) -> (u8, f32) {
        let exact = 69.0 + 12.0 * (hz as f64 / self.reference).log2();
        let note = exact.round().clamp(0.0, 127.0);
        (note as u8, ((exact - note) * 100.0) as f32)
    }
}

impl Engine for Tuner {
    fn set_param(&mut self, id: u32, value: f64) {
        match id {
            REFERENCE => self.reference = reference_hz(value),
            MUTE => self.mute = value > 0.5,
            _ => {}
        }
    }

    fn text(&self, id: u32, value: f64) -> String {
        match id {
            REFERENCE => format!("{:.0} Hz", reference_hz(value)),
            MUTE => (if value > 0.5 { "On" } else { "Off" }).into(),
            _ => format!("{:.0} %", value * 100.0),
        }
    }

    fn note_on(&mut self, _: u8, _: u8) {}

    fn note_off(&mut self, _: u8) {}

    fn reset(&mut self) {}

    fn process(&mut self, left: &mut [f32], right: &mut [f32], _: &Transport) {
        for (&l, &r) in left.iter().zip(right.iter()) {
            self.block[self.filled] = (l + r) * 0.5;
            self.filled += 1;
            if self.filled == HOP {
                // Behind, the hop is lost; gone, nothing is heard. Neither stops the sound.
                let _ = self.blocks.try_send(self.block);
                self.filled = 0;
            }
        }
        let seq = self.shared.seq.load(Ordering::Acquire);
        if seq != self.seen_seq {
            (self.seen_seq, self.since_seen) = (seq, 0);
        } else {
            self.since_seen += left.len() as u64;
        }
        if self.mute {
            left.fill(0.0);
            right.fill(0.0);
        }
    }

    fn readout(&self) -> Option<Readout> {
        let fresh = (self.since_seen as f64) < STALE_S * self.sr;
        Some(Readout {
            note: fresh.then(|| self.shared.reading()).flatten().map(|hz| self.note_of(hz)),
            state: TuneState::Idle,
            worst_cents: None,
            tuned_at: None,
            range: None,
            live_locked: None,
            clamped: false,
            message: None,
            off: Vec::new(),
            holding: self.mute,
            analysed: self.shared.seq.load(Ordering::Acquire),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const SR: f64 = 48_000.0;

    fn sine(hz: f64, from: usize, n: usize) -> Vec<f32> {
        (from..from + n).map(|i| (0.4 * (std::f64::consts::TAU * hz * i as f64 / SR).sin()) as f32).collect()
    }

    /// Plays `hz` through `t` for `blocks` hops, a hop at a time, waiting for each to be
    /// analysed; what comes out.
    fn play(t: &mut Tuner, hz: f64, blocks: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for b in 0..blocks {
            let mut l = if hz > 0.0 { sine(hz, b * HOP, HOP) } else { vec![0.0; HOP] };
            let mut r = l.clone();
            let before = t.shared.seq.load(Ordering::Acquire);
            t.process(&mut l, &mut r, &Transport::default());
            let start = Instant::now();
            while t.shared.seq.load(Ordering::Acquire) == before {
                assert!(start.elapsed() < Duration::from_secs(5), "the tuner never analysed a hop");
                std::thread::sleep(Duration::from_millis(1));
            }
            out.extend(l);
        }
        // What `process` notices of the last detection.
        t.process(&mut [], &mut [], &Transport::default());
        out
    }

    fn tuner(params: &[(u32, f64)]) -> Tuner {
        let mut t = Tuner::new(SR);
        for &(id, v) in params {
            t.set_param(id, v);
        }
        t
    }

    /// The sound comes out as it went in: the Tuner only listens.
    #[test]
    fn the_sound_passes_through_untouched() {
        let mut t = tuner(&[]);
        let out = play(&mut t, 110.0, 6);
        assert_eq!(out, sine(110.0, 0, 6 * HOP));
    }

    /// With Mute on it is silent after the Tuner, and still hears.
    #[test]
    fn mute_silences_it_and_still_tunes() {
        let mut t = tuner(&[(MUTE, 1.0)]);
        let out = play(&mut t, 110.0, 6);
        assert!(out.iter().all(|&s| s == 0.0));
        let r = t.readout().unwrap();
        assert_eq!(r.note.map(|n| n.0), Some(45), "A1");
        assert!(r.holding, "the screens say it is muted");
    }

    /// An A at 110 Hz reads A1 in tune; a sharp one reads its cents; with A4 at 444 Hz,
    /// the 440 Hz world's A reads flat.
    #[test]
    fn it_reads_the_note_and_cents() {
        let mut t = tuner(&[]);
        play(&mut t, 110.0, 6);
        let (note, cents) = t.readout().unwrap().note.expect("a note");
        assert_eq!(note, 45);
        assert!(cents.abs() < 1.0, "{cents}");
        play(&mut t, 110.0 * 2f64.powf(20.0 / 1200.0), 6);
        let (note, cents) = t.readout().unwrap().note.unwrap();
        assert_eq!(note, 45);
        assert!((cents - 20.0).abs() < 1.0, "{cents}");
        let mut t = tuner(&[(REFERENCE, 14.0 / 20.0)]);
        assert_eq!(t.text(REFERENCE, 14.0 / 20.0), "444 Hz");
        play(&mut t, 440.0, 6);
        let (note, cents) = t.readout().unwrap().note.unwrap();
        assert_eq!(note, 69);
        let want = 1200.0 * (440.0f64 / 444.0).log2();
        assert!((cents as f64 - want).abs() < 1.0, "{cents} vs {want}");
    }

    /// Silence shows no note, and neither does a pitch heard long ago.
    #[test]
    fn silence_and_stale_readings_show_no_note() {
        let mut t = tuner(&[]);
        play(&mut t, 0.0, 6);
        assert_eq!(t.readout().unwrap().note, None);
        play(&mut t, 110.0, 6);
        assert!(t.readout().unwrap().note.is_some());
        // Blocks too short to make a hop: no new detection for over a quarter second.
        let mut l = vec![0.0f32; 16];
        let mut r = l.clone();
        for _ in 0..(STALE_S * SR / 16.0) as usize + 1 {
            t.filled = 0;
            t.process(&mut l, &mut r, &Transport::default());
        }
        assert_eq!(t.readout().unwrap().note, None);
    }

    /// The default Reference is 440 Hz and Mute is off.
    #[test]
    fn defaults_are_440_and_not_muted() {
        let p = |id| PARAMS.iter().find(|p| p.id == id).unwrap();
        assert_eq!(reference_hz(p(REFERENCE).default), 440.0);
        assert_eq!(p(MUTE).default, 0.0);
        assert!(!CLASS.instrument);
        assert!(is_tuner(&CLASS.plugin_id()));
        assert_eq!(Tuner::new(SR).text(MUTE, 1.0), "On");
    }
}
