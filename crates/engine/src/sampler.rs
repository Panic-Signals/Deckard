//! Maschine's Sampler, as docs/sampler-dsp.md describes it: per sound, voices that
//! stream a zone's sample (play range, loop with crossfade, zone fades, reverse),
//! resample it with an 8-point interpolator, then run Comp, Drive, Lo-fi, the filter
//! (LP2 / BP2 / HP2 or the EQ band), the amp envelope and the zone's gain and pan.
//! A modulation envelope, an LFO, velocity and the mod wheel modulate pitch, start,
//! cutoff, drive and pan. Section numbers below refer to the spec.
//!
//! A sound whose instrument is one of Maschine's synths (drum synths, Bass Synth, Poly
//! Synth) plays through the same [`SamplerRt`], holding the [`crate::synth::Synth`]
//! instead of zones: it is rendered, kept across edits and rendered ahead the same way.

use crate::sample::Sample;
use crate::sampler_tables as t;
use crate::synth::Synth;
use deckard_model::MidiKind;
use deckard_model::layers::{Layer, Morph};
use deckard_model::new_sampler::Motion;
use std::sync::Arc;

/// Control rate: modulation, filter coefficients and smoothing are updated every
/// 32 frames (§8.1).
const CHUNK: u32 = 32;
/// Voices kept per Sampler at most, whatever the polyphony (a safety limit).
const MAX_VOICES: usize = 128;
/// Below this an envelope counts as silent (§5.1).
const SILENT: f32 = 1e-4;
/// Frames a synth renders at a time.
const SYNTH_CHUNK: usize = 512;

/// `sq(x) = |x|·x`, the shape of almost every modulation amount.
fn sq(x: f32) -> f32 {
    x.abs() * x
}

/// Linear interpolation in `table` at fractional index `x` (0..len−1).
fn at(table: &[f32], x: f32) -> f32 {
    let x = x.clamp(0.0, (table.len() - 1) as f32);
    let i = x as usize;
    if i + 1 < table.len() { table[i] + (x - i as f32) * (table[i + 1] - table[i]) } else { table[i] }
}

/// Envelope segment times (§3), in frames.
fn attack_frames(v: f32, sr: f64) -> u64 {
    ms_to_frames(((t::lookup(&t::ATTACK_CURVE, v) * 8.922792) as f64 + std::f64::consts::LN_2).exp() - 2.0, sr)
}

fn hold_frames(v: f32, sr: f64) -> u64 {
    ms_to_frames(((t::lookup(&t::HOLD_CURVE, v) * 8.922792) as f64 + std::f64::consts::LN_2).exp() - 2.0, sr)
}

fn decay_frames(v: f32, sr: f64) -> u64 {
    ms_to_frames(((t::lookup(&t::DECAY_RELEASE_CURVE, v) * 9.433563) as f64 + std::f64::consts::LN_2).exp() - 2.0, sr)
}

fn ms_to_frames(ms: f64, sr: f64) -> u64 {
    (sr * ms.max(0.0) * 0.001) as u64
}

/// Envelope knob values in milliseconds, for display: (attack, hold, decay/release).
pub fn envelope_ms(attack: f64, hold: f64, decay: f64) -> (f64, f64, f64) {
    let ms = |c: &[f32; 128], v: f64, k: f32| ((t::lookup(c, v as f32) * k) as f64 + std::f64::consts::LN_2).exp() - 2.0;
    (ms(&t::ATTACK_CURVE, attack, 8.922792), ms(&t::HOLD_CURVE, hold, 8.922792), ms(&t::DECAY_RELEASE_CURVE, decay, 9.433563))
}

/// Cutoff knob (0..1, with modulation) to a fraction of Nyquist at 44.1 kHz (§8.5).
fn cutoff_fraction(c: f32) -> f32 {
    let x = c.clamp(0.0, 1.0) * 6.15;
    let e = x.exp() - 1.0;
    (e / 388.67682).clamp(0.0, 1.0) + 0.002
}

/// The cutoff a knob value shows, in Hz, for filter mode 1 (LP2), 2 (BP2) or 3 (HP2).
pub fn cutoff_hz(knob: f64, mode: i64) -> f64 {
    let f = cutoff_fraction(t::lookup(&t::CUTOFF_CURVE, knob as f32)).min(1.0) as f64;
    let k = match mode {
        2 => 0.82,
        3 => 0.92 * 0.92,
        _ => 0.99,
    };
    f * 22050.0 * k
}

/// The Lo-fi sample rate a knob value shows: `None` when off (§8.4).
pub fn lofi_rate_hz(knob: f64, sr: f64) -> Option<f64> {
    let r = t::lookup(&t::LOFI_RATE_CURVE, knob as f32) as f64;
    (r < 0.99999).then(|| (r * sr).max(50.0))
}

/// The Lo-fi bit depth a knob value gives: `None` when off.
pub fn lofi_bits(knob: f64) -> Option<f64> {
    let b = t::lookup(&t::LOFI_BITS_CURVE, knob as f32) as f64;
    (b < 0.99999).then(|| (16.0 * b).max(1.0))
}

/// The EQ band's centre frequency in Hz for a knob value (§8.5).
pub fn eq_hz(knob: f64) -> f64 {
    20.0 * 1000f64.powf(knob.clamp(0.0, 1.0))
}

/// The LFO's free-running rate in Hz, or its note division as (num, den) when synced
/// (§4).
pub fn lfo_rate(speed: f64) -> (f64, (i32, i32)) {
    let d = &t::LFO_RATE_DIVISIONS;
    let n = d.len();
    let x = speed.clamp(0.0, 1.0) * (n - 1) as f64;
    let i = ((x - 0.5).round() as i64).clamp(0, n as i64 - 2) as usize;
    let f = x - i as f64;
    let len = f * d[i + 1].0 as f64 / d[i + 1].1 as f64 + (1.0 - f) * d[i].0 as f64 / d[i].1 as f64;
    let j = ((x).round() as i64).clamp(0, n as i64 - 1) as usize;
    (1.0 / (2.0 * len), d[j])
}

/// One stage of [`Env`]: from level `s` to `e` along Maschine's exponential curve
/// with curvature `c` (§5.2).
#[derive(Clone, Copy, Default)]
struct Segment {
    x: f64,
    m: f64,
    d: f64,
    off: f64,
    left: u64,
}

impl Segment {
    fn new(s: f32, e: f32, frames: u64, curvature: f64) -> Segment {
        const L: f64 = -9.903487552536127;
        const H: f64 = 3.2188758248682006;
        let n = frames.max(1) as f64;
        let c = (L + (H - L) * (1.0 - curvature.abs())).exp();
        let ratio = if curvature > 0.0 { c / (c + 1.0) } else { (c + 1.0) / c };
        let x0 = if curvature > 0.0 { c + 1.0 } else { c };
        let d = if curvature > 0.0 { -(e - s) as f64 } else { (e - s) as f64 };
        Segment { x: x0, m: ratio.powf(1.0 / n), d, off: s as f64 - d * x0, left: frames }
    }

    fn next(&mut self) -> f32 {
        let y = self.off + self.d * self.x;
        self.x *= self.m;
        self.left -= 1;
        y as f32
    }
}

const C_ATTACK: f64 = -0.332;
const C_DECAY: f64 = 0.44269788; // 0x3ee2a84c

/// Envelope segment times and sustain level, for one envelope (§3, §5.1).
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct EnvTimes {
    pub(crate) attack: u64,
    pub(crate) hold: u64,
    pub(crate) decay: u64,
    pub(crate) release: u64,
    pub(crate) sustain: f32,
}

/// A zone that is a layer of its sound (`deckard_model::layers`): its own envelope on
/// top of the sound's, and its window on the sound's morph.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LayerRt {
    /// It is heard (not muted, nor left out by another's solo).
    pub on: bool,
    pub env: EnvTimes,
    pub window: [f64; 4],
}

/// A layer's envelope times at `sr` (its sustain a plain level).
pub fn layer_times(l: &Layer, sr: f64) -> EnvTimes {
    let frames = |s: f64| if s.is_finite() { (s.clamp(0.0, deckard_model::layers::TIME_MAX) * sr).round() as u64 } else { 0 };
    EnvTimes {
        attack: frames(l.attack),
        hold: 0,
        decay: frames(l.decay),
        release: frames(l.release),
        sustain: if l.sustain.is_finite() { l.sustain.clamp(0.0, 1.0) as f32 } else { 1.0 },
    }
}

/// A layer's gain changes (its window, mute, solo) take this long, so nothing clicks.
const LAYER_RAMP_MS: f64 = 5.0;

/// A zone's gain on each side, from its level and pan (§6.6).
fn zone_amp(z: &ZoneRt) -> [f32; 2] {
    let g = 2.3948107 * z.gain.powi(5) + 0.767467 * z.gain * z.gain;
    let p = z.pan;
    // 0.70711 and 0.29289 in Maschine.
    const K: f32 = std::f32::consts::FRAC_1_SQRT_2;
    [(1.0 - K * p - (1.0 - K) * p * p) * g, (1.0 + K * p - (1.0 - K) * p * p) * g]
}

/// How loud layer `l` is to be, and where the morph is, `phase` through its cycle.
fn layer_to(l: &LayerRt, morph: Option<&Morph>, position: f32, phase: f64) -> (f32, f32) {
    let on = if l.on { 1.0 } else { 0.0 };
    match morph.filter(|m| m.on) {
        Some(m) => {
            let p = m.at(f64::from(position), phase);
            (on * deckard_model::layers::window_gain(l.window, p, m.shape, m.fade), p as f32)
        }
        None => (on, 0.0),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Done,
}

/// An AHDSR envelope (§5). `gated`: sustains until note-off, then releases (ADSR);
/// otherwise it runs attack, hold and decay to the sustain level and stops there.
#[derive(Clone, Copy)]
struct Env {
    times: EnvTimes,
    gated: bool,
    stage: Stage,
    seg: Segment,
    level: f32,
}

impl Env {
    fn new(times: EnvTimes, gated: bool) -> Env {
        let mut e = Env { times, gated, stage: Stage::Attack, seg: Segment::default(), level: 0.0 };
        e.enter(Stage::Attack);
        e
    }

    fn enter(&mut self, stage: Stage) {
        self.stage = stage;
        let t = self.times;
        match stage {
            Stage::Attack if t.attack > 0 => self.seg = Segment::new(0.0, 1.0, t.attack, C_ATTACK),
            Stage::Attack => {
                self.level = 1.0;
                self.enter(Stage::Hold)
            }
            Stage::Hold if t.hold > 0 => self.seg = Segment::new(1.0, 1.0, t.hold, 0.0),
            Stage::Hold => self.enter(Stage::Decay),
            Stage::Decay if t.decay > 0 => self.seg = Segment::new(self.level, t.sustain, t.decay, C_DECAY),
            Stage::Decay => {
                self.level = t.sustain;
                self.enter(Stage::Sustain)
            }
            Stage::Sustain if !self.gated => self.stage = Stage::Done,
            Stage::Sustain => {}
            Stage::Release if t.release > 0 => self.seg = Segment::new(self.level, 0.0, t.release, C_DECAY),
            Stage::Release => {
                self.level = 0.0;
                self.stage = Stage::Done
            }
            Stage::Done => {}
        }
    }

    /// Note-off: release from the current level (gated envelopes only).
    fn release(&mut self) {
        if self.gated && self.stage != Stage::Done && self.stage != Stage::Release {
            self.enter(Stage::Release);
        }
    }

    fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Attack | Stage::Hold | Stage::Decay | Stage::Release => {
                self.level = self.seg.next();
                if self.seg.left == 0 {
                    let next = match self.stage {
                        Stage::Attack => Stage::Hold,
                        Stage::Hold => Stage::Decay,
                        Stage::Decay => Stage::Sustain,
                        _ => Stage::Done,
                    };
                    if self.stage == Stage::Release {
                        self.level = 0.0;
                    }
                    self.enter(next);
                }
            }
            Stage::Sustain | Stage::Done => {}
        }
        self.level
    }

    fn done(&self) -> bool {
        self.stage == Stage::Done && self.level < SILENT
    }
}

/// A zone ready to play.
#[derive(Clone)]
pub struct ZoneRt {
    pub sample: Arc<Sample>,
    pub root: i64,
    pub keys: (i64, i64),
    pub vels: (i64, i64),
    /// Zone gain as stored (0.75 = unity) and pan (−1..1).
    pub gain: f32,
    pub pan: f32,
    /// Semitones.
    pub tune: f32,
    /// Play range and loop range, frames [start, end).
    pub play: (usize, usize),
    pub looped: Option<(usize, usize)>,
    pub x_fade: usize,
    /// Zone fade-in and fade-out, frames (§6.5).
    pub fade_in: usize,
    pub fade_out: usize,
    /// A New Sampler zone in Sync or Keys: kept in time with the song.
    pub sync: Option<ZoneSync>,
    /// Its stem lanes, if it has them.
    pub lanes: Option<Lanes>,
    /// It plays backwards (a sound that does too plays it forwards).
    pub reverse: bool,
    /// A one-shot: a note plays it to its end, whenever the note ends.
    pub once: bool,
    /// Its tail, ms of its sample (0: none): at its end it plays back from there, fading.
    pub tail_ms: f32,
    /// It is a layer of its sound.
    pub layer: Option<LayerRt>,
}

/// A zone's stem lanes (roadmap 2b): each as long as the zone's sample, read at the same
/// place. With every lane at unity a voice plays the sample itself; else the lanes'
/// sum at their `levels` (0: muted), crossfading between the two and ramping the levels
/// over [`LANE_RAMP_MS`] so sounding notes don't click.
#[derive(Clone, Default)]
pub struct Lanes {
    pub samples: [Option<Arc<Sample>>; LANES],
    pub levels: [f32; LANES],
    /// A stem was taken away: the lanes play even at unity (the sample still has it).
    pub only: bool,
}

pub const LANES: usize = deckard_model::new_sampler::LANES;
const LANE_RAMP_MS: f64 = 5.0;

impl Lanes {
    /// Every lane there is plays at unity: the sample itself is what's heard.
    fn all_on(&self) -> bool {
        !self.only && (0..LANES).all(|k| self.samples[k].is_none() || self.levels[k] == 1.0)
    }
}

/// How a New Sampler zone in Sync or Keys plays (roadmap 1c): its sample (usually a
/// [`crate::render`] of it, its Tune already in) is in time with the song at `tempo`,
/// and reads `song tempo / tempo` frames a frame, so it stays in time whatever the
/// song does (repitched, until a render for the new tempo comes). Sync plays every
/// note the same; Keys transposes by the note, from `notes` (renders by distance from
/// the root, the same length) when it has the note, else by reading faster or slower.
#[derive(Clone, Default)]
pub struct ZoneSync {
    pub tempo: f64,
    pub keys: bool,
    pub notes: Vec<(i64, Arc<Sample>)>,
    /// The stem lanes' renders for `notes` (as many, or none): a note without them
    /// plays whole, whatever is muted.
    pub note_lanes: Vec<[Option<Arc<Sample>>; LANES]>,
    /// Sync: notes played live start on this grid (beats; 0: when played). One up to
    /// half a division early waits for the line; a later one starts at once, as far
    /// into the sample as it is late.
    pub grid: f64,
    /// A render plays in the sample's place: (its frame, the sample's frame there),
    /// rising, so the playhead shows where in the sample it is. Empty: the sample plays.
    pub shown: Vec<(f64, f64)>,
    /// Stretched as it plays (no render): the zone's sample is the file itself, each
    /// voice time-stretches it to the song's tempo, its pitch moved by these semitones
    /// (its Tune) and, in Keys, the note.
    pub live: Option<f64>,
}

/// How many voices a sampler stretches at once (the rest play repitched), and the
/// frames a stretcher makes at a time.
const LIVE_ROOM: usize = 12;
const LIVE_CHUNK: usize = 64;
const NO_LIVE: u8 = u8::MAX;
/// Frames between two stretchers' starts: a start costs several blocks' work at once, so
/// notes that come together start this far apart (a chord of four over 8 ms at 48 kHz).
const LIVE_APART: u64 = 128;
/// The most a stretcher is fed for a chunk, and to start.
const LIVE_IN: usize = 16_384;

/// A voice's stretcher: fed the zone's frames as fast as the song goes, it gives them
/// at the pitch they were (times `transpose`).
struct Live {
    /// The rate it was made for.
    sr: f64,
    st: deckard_stretch::Stretch,
    /// Input frames it's fed ahead, output frames it comes out behind.
    lat: (usize, usize),
    buf: [Vec<f32>; 4],
    /// Output frames made, and the next to give.
    have: usize,
    at: usize,
    /// The fraction of an input frame owed.
    owed: f64,
    /// Input frames an output frame, and the pitch factor.
    rate: f64,
    /// The rate it was last fed at: it's fed ahead of what it gives by its latency at
    /// that rate, so another rate owes it the difference (or it, the song).
    fed: f64,
    transpose: f32,
    /// A voice was given it and hasn't started it.
    fresh: bool,
    /// Output frames left once the zone's frames ran out.
    left: Option<u32>,
}

/// Stretchers of samplers no longer playing, by rate: every edit builds the engine
/// again, and making a sampler's stretchers takes some 15 ms.
static SPARE_LIVES: std::sync::Mutex<Vec<(u64, Vec<Live>)>> = std::sync::Mutex::new(Vec::new());
/// The most sets kept.
const SPARE_ROOM: usize = 32;

/// A sampler's stretchers. Dropped (wherever its sampler is, the audio thread too) they
/// go to the spare sets while there's room made for them.
#[derive(Default)]
struct Lives(Vec<Live>);

impl std::ops::Deref for Lives {
    type Target = Vec<Live>;
    fn deref(&self) -> &Vec<Live> {
        &self.0
    }
}

impl std::ops::DerefMut for Lives {
    fn deref_mut(&mut self) -> &mut Vec<Live> {
        &mut self.0
    }
}

impl Drop for Lives {
    fn drop(&mut self) {
        if self.0.len() == LIVE_ROOM
            && let Ok(mut spare) = SPARE_LIVES.try_lock()
            && spare.len() < spare.capacity().min(SPARE_ROOM)
            && let Some(l) = self.0.first()
        {
            let rate = l.sr.to_bits();
            spare.push((rate, std::mem::take(&mut self.0)));
        }
    }
}

impl Live {
    /// A sampler's stretchers: a set left by one before it, else made.
    fn pool(sr: f64) -> Lives {
        let kept = SPARE_LIVES.lock().ok().and_then(|mut spare| {
            // (Room made here, off the audio thread, for the sets dropped on it.)
            spare.reserve(SPARE_ROOM);
            let i = spare.iter().position(|(r, _)| *r == sr.to_bits())?;
            Some(spare.swap_remove(i).1)
        });
        Lives(kept.unwrap_or_else(|| (0..LIVE_ROOM).map(|_| Live::new(sr)).collect()))
    }

    fn new(sr: f64) -> Live {
        let st = deckard_stretch::Stretch::spread(sr as f32);
        let lat = st.latency();
        let out = LIVE_CHUNK.max(lat.1);
        let mut live = Live {
            sr,
            st,
            lat,
            buf: [vec![0.0; LIVE_IN], vec![0.0; LIVE_IN], vec![0.0; out], vec![0.0; out]],
            have: 0,
            at: 0,
            owed: 0.0,
            rate: 1.0,
            fed: 1.0,
            transpose: 1.0,
            fresh: false,
            left: None,
        };
        // Run once here (off the audio thread): its memory is touched before a note
        // needs it.
        live.start(&mut || [0.0; 2]);
        live
    }

    /// Start at the frames `pull` gives next: silence before them, the latency run
    /// through and dropped, so the first frame out is the first frame in.
    fn start(&mut self, pull: &mut impl FnMut() -> [f32; 2]) {
        let (lat_in, lat_out) = self.lat;
        let [il, ir, ol, or] = &mut self.buf;
        let rate = self.rate.clamp(0.05, 16.0);
        self.st.reset();
        self.st.set_transpose(self.transpose);
        let pre = (lat_in * 2).max(4096).min(LIVE_IN);
        let ahead = lat_in.min(pre);
        il[..pre - ahead].fill(0.0);
        ir[..pre - ahead].fill(0.0);
        for k in pre - ahead..pre {
            [il[k], ir[k]] = pull();
        }
        self.st.seek(&il[..pre], &ir[..pre], rate);
        let m = ((lat_out as f64 * rate).round() as usize).min(LIVE_IN);
        for k in 0..m {
            [il[k], ir[k]] = pull();
        }
        let drop_n = lat_out.min(ol.len());
        self.st.process(&il[..m], &ir[..m], &mut ol[..drop_n], &mut or[..drop_n]);
        (self.have, self.at, self.owed, self.left, self.fresh) = (0, 0, 0.0, None, false);
        self.fed = rate;
    }

    /// The next frame out.
    fn next(&mut self, pull: &mut impl FnMut() -> [f32; 2]) -> [f32; 2] {
        if self.fresh {
            self.start(pull);
        }
        if self.at == self.have {
            let [il, ir, ol, or] = &mut self.buf;
            self.st.set_transpose(self.transpose);
            let rate = self.rate.clamp(0.05, 16.0);
            // Another tempo: what it holds was fed for the old one, so the note would be
            // left ahead of the song (a slower one) or behind it by the latency's worth.
            // That's taken back over the next chunks, at no less than half the rate and no
            // more than twice.
            self.owed += self.lat.1 as f64 * (rate - self.fed);
            self.fed = rate;
            let chunk = LIVE_CHUNK as f64 * rate;
            let want = chunk + self.owed;
            let m = (want.clamp(chunk * 0.5, chunk * 2.0).floor() as usize).min(LIVE_IN);
            self.owed = want - m as f64;
            for k in 0..m {
                [il[k], ir[k]] = pull();
            }
            self.st.process(&il[..m], &ir[..m], &mut ol[..LIVE_CHUNK], &mut or[..LIVE_CHUNK]);
            (self.have, self.at) = (LIVE_CHUNK, 0);
        }
        let x = [self.buf[2][self.at], self.buf[3][self.at]];
        self.at += 1;
        x
    }

    /// Output frames still to come once the zone's frames have run out.
    fn after(&self) -> u32 {
        (self.lat.1 as f64 + self.lat.0 as f64 / self.rate.clamp(0.05, 16.0)) as u32
    }
}

impl ZoneSync {
    /// The sample's frame played at the render's frame `pos`.
    fn source(&self, pos: f64) -> f64 {
        let i = self.shown.partition_point(|p| p.0 <= pos);
        match (i.checked_sub(1).and_then(|j| self.shown.get(j)), self.shown.get(i)) {
            (Some(a), Some(b)) if b.0 > a.0 => a.1 + (pos - a.0) / (b.0 - a.0) * (b.1 - a.1),
            (Some(a), _) => a.1,
            (None, Some(b)) => b.1,
            (None, None) => pos,
        }
    }

    /// The render for `offset` semitones from the root, in Keys.
    fn note(&self, offset: i64) -> Option<usize> {
        if self.keys { self.notes.iter().position(|(o, _)| *o == offset) } else { None }
    }
}

impl ZoneRt {
    /// The frames voice `variant` reads: a Keys note's render, or the zone's sample.
    fn data(&self, variant: Option<usize>) -> &[[f32; 2]] {
        match variant.and_then(|i| self.sync.as_ref()?.notes.get(i)) {
            Some((_, s)) => &s.frames,
            None => &self.sample.frames,
        }
    }

    /// The lanes voice `variant` reads, and their levels: none when it has no lanes, or
    /// they all play at unity.
    fn lanes_of(&self, variant: Option<usize>) -> Option<(&[Option<Arc<Sample>>; LANES], &[f32; LANES])> {
        let l = self.lanes.as_ref().filter(|l| !l.all_on())?;
        match variant {
            Some(i) => self.sync.as_ref()?.note_lanes.get(i).filter(|s| s.iter().any(Option::is_some)).map(|s| (s, &l.levels)),
            None => Some((&l.samples, &l.levels)),
        }
    }

    /// Speed at song tempo `tempo` (1 unless in Sync or Keys).
    fn speed(&self, tempo: f64) -> f64 {
        match &self.sync {
            Some(s) if s.tempo > 0.0 && tempo > 0.0 => tempo / s.tempo,
            _ => 1.0,
        }
    }
}

/// The Sampler's fields modulation can drive ([`Settings::set_field`]).
pub const MODULATED: [&str; 41] = [
    "glide",
    "pitchbend",
    "tune",
    "start",
    "comp",
    "drive",
    "bits",
    "sample_rate",
    "cutoff",
    "resonance",
    "eq_frequency",
    "eq_width",
    "eq_gain",
    "velocity_start",
    "velocity_decay",
    "velocity_cutoff",
    "velocity_volume",
    "modwheel_start",
    "modwheel_cutoff",
    "modwheel_lfo",
    "modwheel_pan",
    "volume_envelope/attack",
    "volume_envelope/hold",
    "volume_envelope/decay",
    "volume_envelope/release",
    "volume_envelope/sustain",
    "mod_envelope/attack",
    "mod_envelope/hold",
    "mod_envelope/decay",
    "mod_envelope/release",
    "mod_envelope/sustain",
    "mod_envelope/pitch",
    "mod_envelope/cutoff",
    "mod_envelope/drive",
    "mod_envelope/pan",
    "lfo/speed",
    "lfo/phase",
    "lfo/pitch_amount",
    "lfo/cutoff_amount",
    "lfo/drive_amount",
    "lfo/pan_amount",
];

/// The Sampler's settings, converted from the stored values (§1).
#[derive(Clone, PartialEq)]
pub struct Settings {
    polyphony: usize,
    glide: f32,
    bend_range: f32,
    mode: usize,
    env_type: i64,
    reverse: bool,
    tune: f32,
    start: f32,
    comp: f32,
    drive: f32,
    lofi_bits: f32,
    lofi_rate: f32,
    filter: i64,
    cutoff: f32,
    resonance: f32,
    eq: (f32, f32, f32),
    vel_start: f32,
    vel_decay: f32,
    vel_cutoff: f32,
    vel_volume: f32,
    mw_start: f32,
    mw_cutoff: f32,
    mw_lfo: f32,
    mw_pan: f32,
    amp: EnvTimes,
    amp_decay_knob: f32,
    modenv: EnvTimes,
    env_pitch: f32,
    env_cutoff: f32,
    env_drive: f32,
    env_pan: f32,
    lfo_type: i64,
    lfo_speed: f32,
    lfo_phase: f32,
    lfo_sync: i64,
    lfo_pitch: f32,
    lfo_cutoff: f32,
    lfo_drive: f32,
    lfo_pan: f32,
    /// A synth's parameters (stored values, by field), when the sound plays one.
    synth: Vec<(String, f64)>,
}

impl Settings {
    /// Settings of a sound played by a synth: its module's parameters.
    pub fn for_synth(params: &[(String, f64)], sr: f64) -> Settings {
        Settings { synth: params.to_vec(), ..Settings::new(&deckard_model::Sampler::default(), sr) }
    }

    pub fn new(s: &deckard_model::Sampler, sr: f64) -> Settings {
        let g = |k: &str| s.get(k) as f32;
        let env = |e: &str| EnvTimes {
            attack: attack_frames(g(&format!("{e}/attack")), sr),
            hold: hold_frames(g(&format!("{e}/hold")), sr),
            decay: decay_frames(g(&format!("{e}/decay")), sr),
            release: decay_frames(g(&format!("{e}/release")), sr),
            sustain: g(&format!("{e}/sustain")).clamp(0.0, 1.0).powi(3),
        };
        Settings {
            polyphony: g("polyphony").max(0.0) as usize,
            glide: g("glide"),
            bend_range: sq(g("pitchbend")),
            mode: g("mode").max(0.0) as usize,
            env_type: g("envelope_type") as i64,
            reverse: g("reverse") != 0.0,
            tune: g("tune") * 3.0,
            start: sq(g("start")),
            comp: g("comp"),
            drive: g("drive"),
            lofi_bits: g("bits"),
            lofi_rate: g("sample_rate"),
            filter: g("filter") as i64,
            cutoff: g("cutoff"),
            resonance: g("resonance"),
            eq: (g("eq_frequency"), g("eq_width"), g("eq_gain")),
            vel_start: sq(g("velocity_start")),
            vel_decay: sq(g("velocity_decay")),
            vel_cutoff: sq(g("velocity_cutoff")),
            vel_volume: sq(g("velocity_volume")),
            mw_start: sq(g("modwheel_start")),
            mw_cutoff: sq(g("modwheel_cutoff")),
            mw_lfo: sq(g("modwheel_lfo")),
            mw_pan: sq(g("modwheel_pan")),
            amp: env("volume_envelope"),
            amp_decay_knob: g("volume_envelope/decay"),
            modenv: env("mod_envelope"),
            env_pitch: sq(g("mod_envelope/pitch")),
            env_cutoff: sq(g("mod_envelope/cutoff")),
            env_drive: sq(g("mod_envelope/drive")),
            env_pan: sq(g("mod_envelope/pan")),
            lfo_type: g("lfo/type") as i64,
            lfo_speed: g("lfo/speed"),
            lfo_phase: g("lfo/phase"),
            lfo_sync: g("lfo/sync_mode") as i64,
            lfo_pitch: sq(g("lfo/pitch_amount")),
            lfo_cutoff: sq(g("lfo/cutoff_amount")),
            lfo_drive: sq(g("lfo/drive_amount")),
            lfo_pan: sq(g("lfo/pan_amount")),
            synth: Vec::new(),
        }
    }

    /// Set field `k` (as stored: `cutoff`, `volume_envelope/attack`, …) to stored value
    /// `v`, converted as [`Settings::new`] does: modulation, on the audio thread (no
    /// allocating). Returns whether `k` is one of the [`MODULATED`] fields.
    pub fn set_field(&mut self, k: &str, v: f64, sr: f64) -> bool {
        let x = v as f32;
        let env = |e: &mut EnvTimes, part: &str| match part {
            "attack" => e.attack = attack_frames(x, sr),
            "hold" => e.hold = hold_frames(x, sr),
            "decay" => e.decay = decay_frames(x, sr),
            "release" => e.release = decay_frames(x, sr),
            _ => e.sustain = x.clamp(0.0, 1.0).powi(3),
        };
        match k {
            "glide" => self.glide = x,
            "pitchbend" => self.bend_range = sq(x),
            "tune" => self.tune = x * 3.0,
            "start" => self.start = sq(x),
            "comp" => self.comp = x,
            "drive" => self.drive = x,
            "bits" => self.lofi_bits = x,
            "sample_rate" => self.lofi_rate = x,
            "cutoff" => self.cutoff = x,
            "resonance" => self.resonance = x,
            "eq_frequency" => self.eq.0 = x,
            "eq_width" => self.eq.1 = x,
            "eq_gain" => self.eq.2 = x,
            "velocity_start" => self.vel_start = sq(x),
            "velocity_decay" => self.vel_decay = sq(x),
            "velocity_cutoff" => self.vel_cutoff = sq(x),
            "velocity_volume" => self.vel_volume = sq(x),
            "modwheel_start" => self.mw_start = sq(x),
            "modwheel_cutoff" => self.mw_cutoff = sq(x),
            "modwheel_lfo" => self.mw_lfo = sq(x),
            "modwheel_pan" => self.mw_pan = sq(x),
            "mod_envelope/pitch" => self.env_pitch = sq(x),
            "mod_envelope/cutoff" => self.env_cutoff = sq(x),
            "mod_envelope/drive" => self.env_drive = sq(x),
            "mod_envelope/pan" => self.env_pan = sq(x),
            "lfo/speed" => self.lfo_speed = x,
            "lfo/phase" => self.lfo_phase = x,
            "lfo/pitch_amount" => self.lfo_pitch = sq(x),
            "lfo/cutoff_amount" => self.lfo_cutoff = sq(x),
            "lfo/drive_amount" => self.lfo_drive = sq(x),
            "lfo/pan_amount" => self.lfo_pan = sq(x),
            _ => match k.split_once('/') {
                Some(("volume_envelope", part @ ("attack" | "hold" | "decay" | "release" | "sustain"))) => {
                    if part == "decay" {
                        self.amp_decay_knob = x;
                    }
                    env(&mut self.amp, part);
                }
                Some(("mod_envelope", part @ ("attack" | "hold" | "decay" | "release" | "sustain"))) => env(&mut self.modenv, part),
                _ => return false,
            },
        }
        true
    }

    /// Notes that may sound at once (§9: Legato and "1" allow one).
    fn note_limit(&self) -> usize {
        match self.polyphony {
            0 | 1 => 1,
            i => t::POLYPHONY_LIMITS[(i - 2).min(t::POLYPHONY_LIMITS.len() - 1)],
        }
    }

    fn legato(&self) -> bool {
        self.polyphony == 0
    }

    /// Copy all scalar fields and `synth` parameter values from `other` without
    /// touching `synth`'s keys (no allocating on the audio thread).
    pub(crate) fn copy_values_from(&mut self, other: &Settings) {
        let mut synth = std::mem::take(&mut self.synth);
        for (dst, src) in synth.iter_mut().zip(&other.synth) {
            dst.1 = src.1;
        }
        *self = Settings { synth, ..*other };
    }
}

/// Where a voice reads its zone's sample: play range, loop and crossfade, zone fades
/// and direction (§6.4, §6.5). Yields source frames one at a time.
#[derive(Clone, Copy)]
struct Stream {
    pos: i64,
    forward: bool,
    play: (i64, i64),
    looped: Option<(i64, i64)>,
    x_fade: i64,
    fade_in: i64,
    fade_out: i64,
    /// How long the loop crossfade under way runs (frames); 0: none is.
    fading: i64,
    done: bool,
    /// The tail to come when the play range runs out (frames; 0: none, or it has begun),
    /// and the tail under way: frames left of how many.
    tail: i64,
    tail_left: i64,
    tail_len: i64,
}

impl Stream {
    /// Frame `p` under the zone's fades, by its place in the play range: nothing before it.
    fn faded(&self, read: &impl Fn(usize) -> [f32; 2], p: i64) -> [f32; 2] {
        let (start, end) = self.play;
        let mut g = 1.0;
        if self.fade_in > 0 && p >= start && p - start < self.fade_in {
            g *= (p - start) as f32 / self.fade_in as f32;
        }
        if self.fade_out > 0 && p < end && end - p <= self.fade_out {
            g *= (end - p) as f32 / self.fade_out as f32;
        }
        let x = read(p as usize);
        [x[0] * g, x[1] * g]
    }

    fn next(&mut self, data: &[[f32; 2]]) -> [f32; 2] {
        self.next_by(data.len(), &|p| data[p])
    }

    /// [`Stream::next`] of `len` frames given by `read`.
    fn next_by(&mut self, len: usize, read: &impl Fn(usize) -> [f32; 2]) -> [f32; 2] {
        if self.done {
            return [0.0; 2];
        }
        let (start, end) = self.play;
        let mut p = self.pos;
        if p < start || p >= end {
            if self.tail <= 0 || end <= start {
                self.done = true;
                return [0.0; 2];
            }
            // The tail: back from the edge it ran off, fading (the same audio the other
            // way, so the turn can't click).
            self.tail_len = self.tail.min(end - start);
            self.tail_left = self.tail_len;
            self.tail = 0;
            self.forward = !self.forward;
            p = if self.forward { start } else { end - 1 };
            self.pos = p;
        }
        let mut x = self.faded(read, p);
        if self.tail_len > 0 {
            let g = self.tail_left as f32 / self.tail_len as f32;
            x = [x[0] * g, x[1] * g];
            self.tail_left -= 1;
            self.done = self.tail_left <= 0;
        }
        // Loop crossfade: the last frames before the jump back fade into what the jump
        // would otherwise skip, linearly (§6.5). That's in the sample whether or not it's
        // in the play range.
        if let Some((ls, le)) = self.looped {
            let last = len as i64;
            let (left, room, other) = if self.forward {
                (le - p, ls, ls - (le - p))
            } else {
                let left = p + 1 - ls;
                (left, last - le + 1, le + left - 2)
            };
            let xf = self.x_fade.min(le - ls).min(room);
            if xf > 0 && left > 0 && left <= xf {
                // (From where the fade began: a note may start inside it.)
                if self.fading == 0 {
                    self.fading = left;
                }
                if other >= 0 && other < last {
                    let w = left as f32 / self.fading as f32;
                    let y = self.faded(read, other);
                    x = [x[0] * w + y[0] * (1.0 - w), x[1] * w + y[1] * (1.0 - w)];
                }
            }
        }
        // Advance.
        if self.forward {
            self.pos += 1;
            if let Some((ls, le)) = self.looped
                && self.pos >= le
            {
                self.pos = ls;
                self.fading = 0;
            }
        } else {
            self.pos -= 1;
            if let Some((ls, le)) = self.looped
                && self.pos < ls
            {
                self.pos = le - 1;
                self.fading = 0;
            }
        }
        x
    }
}

/// Frame `p` of a zone: `whole` of its sample `data`, the rest of its stem lanes' sum
/// at levels `lane` (lanes shorter than it are silent past their end).
fn mixed(data: &[[f32; 2]], lanes: Option<&[Option<Arc<Sample>>; LANES]>, whole: f32, lane: &[f32; LANES], p: usize) -> [f32; 2] {
    let x = data[p];
    let Some(lanes) = lanes.filter(|_| whole < 1.0) else { return x };
    let mut sum = [0f32; 2];
    for (s, g) in lanes.iter().zip(lane) {
        if let Some(f) = s.as_ref().and_then(|s| s.frames.get(p)).filter(|_| *g != 0.0) {
            sum[0] += f[0] * g;
            sum[1] += f[1] * g;
        }
    }
    [x[0] * whole + sum[0] * (1.0 - whole), x[1] * whole + sum[1] * (1.0 - whole)]
}

/// The 8-point interpolator's tap weights for `t = 1 − frac` (§7), for taps
/// x[n−3] .. x[n+4].
fn weights(t: f32) -> [f32; 8] {
    let h = |c: [f32; 5], t: f32| ((((c[0] * t + c[1]) * t + c[2]) * t + c[3]) * t + c[4]) * t;
    [
        h([-0.020964433, 0.075699046, -0.061509572, 0.007359662, -0.00058470765], t),
        h([0.14338401, -0.38360944, 0.18016043, 0.03248506, 0.027579939], t),
        h([-0.3720349, 0.90449077, -0.08861077, -0.25935876, -0.18448636], t),
        h([0.6183112, -1.51671, -0.032761108, 1.1424075, 0.78875244], t),
        (((-0.6183112 * t + 1.574846) * t - 0.0835108) * t - 1.873024) * t * t + 1.0,
        h([0.3720349, -0.95568377, 0.1909967, 1.1814046, -0.78875244], t),
        h([-0.14338401, 0.3333106, -0.07956275, -0.2948502, 0.18448636], t),
        h([0.020964433, -0.02912311, -0.031642303, 0.06738092, -0.027579939], t),
    ]
}

/// A direct-form-I biquad (§8.5): y = b0x + b1x1 + b2x2 + a1y1 + a2y2 (feedback signs
/// included in a1, a2).
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: [f64; 2],
    x2: [f64; 2],
    y1: [f64; 2],
    y2: [f64; 2],
}

impl Biquad {
    fn set(&mut self, (b0, b1, b2, a1, a2): (f64, f64, f64, f64, f64)) {
        (self.b0, self.b1, self.b2, self.a1, self.a2) = (b0, b1, b2, a1, a2);
    }

    /// One frame. Computed in double precision: near the lowest cutoffs the poles sit
    /// almost on the unit circle, where single precision drifts and can run away while
    /// the coefficients move.
    fn run(&mut self, x: [f32; 2]) -> [f32; 2] {
        let mut y = [0.0; 2];
        for c in 0..2 {
            let xc = x[c] as f64;
            let yc = self.b0 * xc + self.b1 * self.x1[c] + self.b2 * self.x2[c] + self.a1 * self.y1[c] + self.a2 * self.y2[c] + 1e-28;
            self.x2[c] = self.x1[c];
            self.x1[c] = xc;
            self.y2[c] = self.y1[c];
            self.y1[c] = yc;
            y[c] = yc as f32;
        }
        y
    }

    fn reset(&mut self) {
        (self.x1, self.x2, self.y1, self.y2) = ([0.0; 2], [0.0; 2], [0.0; 2], [0.0; 2]);
    }
}

/// LP2 / HP2 coefficients from the Butterworth prototype (§8.5).
fn two_pole(f: f32, q: f32, high: bool) -> (f64, f64, f64, f64, f64) {
    let [pb1, pb2, pa1, pa2, pb0] = t::FILTER_PROTOTYPE.map(|x| x as f64);
    let (f, q) = (f as f64, q as f64);
    let k = if high {
        let w = f * std::f64::consts::FRAC_PI_2 * 0.92 * 0.9971332;
        -(0.5 + w).cos() / (0.5 - w).cos()
    } else {
        let w = f * std::f64::consts::FRAC_PI_2 * 0.99 * 0.9987809;
        (0.5 - w).sin() / (0.5 + w).sin()
    };
    let a1p = pa1;
    let a2p = pa2 * (1.0 + 2.687 * q);
    let d = 1.0 / ((k * a1p + 1.0) - k * k * a2p);
    let mut a1 = (a1p + 2.0 * k + k * k * a1p - 2.0 * k * a2p) * d;
    let a2 = (a2p - a1p * k - k * k) * d;
    let b0 = (pb0 - k * pb1 + k * k * pb2) * d;
    let mut b1 = (pb1 - 2.0 * k * pb0 + k * k * pb1 - 2.0 * k * pb2) * d;
    let b2 = (pb2 - k * pb1 + k * k * pb0) * d;
    let comp = (1.0 - q) * 0.55 + 0.45;
    let g = if high {
        a1 = -a1;
        b1 = -b1;
        comp * ((a1 - a2) + 1.0) / (b0 - b1 + b2)
    } else {
        comp * (1.0 - (a1 + a2)) / (b0 + b1 + b2)
    };
    (g * b0, g * b1, g * b2, a1, a2)
}

/// BP2: two first-order sections, (b0, b1, a1) each (§8.5).
fn band_pass(f: f32) -> [(f64, f64, f64); 2] {
    let [b1p, a1p, b0p] = t::BANDPASS_PROTOTYPE.map(|x| x as f64);
    let w = f as f64 * std::f64::consts::FRAC_PI_2 * 0.82;
    let k1 = (0.5 - w * 0.9987809).sin() / (0.5 + w * 0.9987809).sin();
    let d1 = 1.0 / (k1 * a1p + 1.0);
    let (b0, b1, a) = (d1 * (b0p - k1 * b1p), d1 * (b1p - k1 * b0p), d1 * (a1p + k1));
    let g1 = (1.0 - a) / (b0 + b1);
    let k2 = -0.99 * (0.5 + w * 0.9971332).cos() / (0.5 - w * 0.9971332).cos();
    let d2 = 1.0 / (k2 * a1p + 1.0);
    let (c0, c1, a2) = (d2 * (b0p - k2 * b1p), d2 * (b1p - k2 * b0p), d2 * (k2 + a1p));
    let g2 = (1.0 - a2) / (c0 + c1);
    [(g1 * b0, g1 * b1, a), (g2 * c0, -g2 * c1, -a2)]
}

/// EQ peaking band (§8.5): (b0, b1, b2, a1, a2) with feedback signs.
fn peak_eq(freq: f32, width: f32, gain: f32, sr: f64) -> (f64, f64, f64, f64, f64) {
    let hz = eq_hz(freq as f64);
    let w0 = (hz / sr).min(0.49) * std::f64::consts::TAU;
    let bw = 2f64.powf(width as f64 * 8.0 / 3.0 + 1.0 / 3.0);
    let alpha = w0.sin() * (bw - 1.0 / bw) / 2.0;
    let a = 2f64.powf(gain as f64 * 18.0 / 6.0);
    let a0 = 1.0 + alpha / a;
    let c = w0.cos();
    (((1.0 + alpha * a) / a0), (-2.0 * c / a0), ((1.0 - alpha * a) / a0), (2.0 * c / a0), ((alpha / a - 1.0) / a0))
}

/// The filter's response for its display: `n` points (x 0..1, gain in dB) at 44.1 kHz.
/// x is the knob position whose cutoff (or EQ frequency) is there, so a handle at the
/// knob's value sits on the corner. Mode as the `filter` parameter (0 off, 1 LP2, 2 BP2,
/// 3 HP2, 4 EQ).
pub fn filter_response(mode: i64, cutoff: f64, resonance: f64, eq: (f64, f64, f64), n: usize) -> Vec<(f32, f32)> {
    let sr = 44_100.0;
    let q = (1.0 - (1.0 - resonance) * (1.0 - resonance)).clamp(0.0, 1.0) as f32;
    let fr = cutoff_fraction(t::lookup(&t::CUTOFF_CURVE, cutoff.clamp(0.0, 1.0) as f32)).min(1.0) * 0.998004;
    // Sections in the engine's form (feedback signs).
    let sections: Vec<(f64, f64, f64, f64, f64)> = match mode {
        1 => vec![two_pole(fr, q, false)],
        3 => vec![two_pole(fr, q, true)],
        2 => band_pass(fr).iter().map(|&(b0, b1, a)| (b0, b1, 0.0, a, 0.0)).collect(),
        4 => vec![peak_eq(eq.0 as f32, eq.1 as f32, eq.2 as f32, sr)],
        _ => Vec::new(),
    };
    (0..n)
        .map(|i| {
            let x = i as f64 / (n - 1).max(1) as f64;
            let hz = if mode == 4 { eq_hz(x) } else { cutoff_hz(x, mode.clamp(1, 3)) };
            let w = std::f64::consts::TAU * (hz / sr).min(0.4999);
            let (c1, s1, c2, s2) = (w.cos(), -w.sin(), (2.0 * w).cos(), -(2.0 * w).sin());
            let mag: f64 = sections
                .iter()
                .map(|&(b0, b1, b2, a1, a2)| {
                    let num = ((b0 + b1 * c1 + b2 * c2).powi(2) + (b1 * s1 + b2 * s2).powi(2)).sqrt();
                    let den = ((1.0 - a1 * c1 - a2 * c2).powi(2) + (a1 * s1 + a2 * s2).powi(2)).sqrt();
                    num / den.max(1e-12)
                })
                .product();
            (x as f32, (20.0 * mag.max(1e-6).log10()) as f32)
        })
        .collect()
}

/// One-pole coefficient reaching 99 % in `ms` (§8.2).
fn smoothing(ms: f64, sr: f64) -> f32 {
    let n = (sr * ms * 0.001 - 1.0).max(1.0);
    (1.0 - 2f64.powf(-(100f64.log2()) / n)) as f32
}

#[derive(Clone, Copy)]
struct Coeffs {
    c50: f32,
    gain: f32,
    release: f32,
}

/// Comp (ShaperDrum, §8.2).
#[derive(Clone, Copy, Default)]
struct Comp {
    cur: f32,
    env: f32,
    g: f32,
}

impl Comp {
    fn run(&mut self, x: [f32; 2], target: f32, c: Coeffs) -> [f32; 2] {
        let step = (target - self.cur) * c.c50;
        self.cur = if step.abs() < 1e-4 { target } else { self.cur + step };
        if self.cur <= 0.0001 * 50.0 && target <= 0.0001 * 50.0 {
            return x;
        }
        let peak = x[0].abs().max(x[1].abs());
        self.env = if peak >= self.env { peak } else { self.env + c.release * (peak - self.env) + 1e-12 };
        let lev = (self.env + 1e-5).min(1.0);
        let row = self.cur.clamp(0.0, 50.0);
        let r0 = (row as usize).min(51);
        let r1 = (r0 + 1).min(51);
        let col = lev * 127.0;
        let curve = |r: usize| at(&t::COMP_CURVES[r * 128..r * 128 + 128], col);
        let fr = row - r0 as f32;
        let v = curve(r0) + fr * (curve(r1) - curve(r0));
        let target_gain = v / lev;
        self.g += c.gain * (target_gain - self.g) + 1e-12;
        let k = self.g * at(&t::COMP_GAIN_CORRECTION, self.cur);
        [x[0] * k, x[1] * k]
    }
}

/// Drive (ShaperEnhanced, §8.3).
#[derive(Clone, Copy, Default)]
struct Drive {
    cur: f32,
}

impl Drive {
    fn run(&mut self, x: [f32; 2], target: f32, c50: f32) -> [f32; 2] {
        let step = (target - self.cur) * c50;
        self.cur = if step.abs() < 1e-4 { target } else { self.cur + step };
        if self.cur <= 0.0001 * 127.0 {
            return x;
        }
        let amount = at(&t::DRIVE_AMOUNT, self.cur);
        let mix = at(&t::DRIVE_MIX, self.cur);
        let corr = at(&t::DRIVE_GAIN_CORRECTION, self.cur);
        let f = |x: f32| {
            let u = (x * amount).clamp(-8.0, 8.0);
            x + mix * (corr * u * (1.0 - u.abs() / 16.0) - x)
        };
        [f(x[0]), f(x[1])]
    }
}

/// Lo-fi: bit reduction then sample-rate hold (§8.4).
#[derive(Clone, Copy, Default)]
struct Lofi {
    count: f32,
    held: [f32; 2],
    primed: bool,
}

impl Lofi {
    fn run(&mut self, x: [f32; 2], levels: Option<f32>, hold: Option<f32>) -> [f32; 2] {
        let mut y = x;
        if let Some(l) = levels {
            y = [(y[0] * l).trunc() / l, (y[1] * l).trunc() / l];
        }
        if let Some(h) = hold {
            if !self.primed {
                self.primed = true;
                self.count = 0.0;
                self.held = y;
            } else if self.count >= h {
                self.count -= h;
                self.held = y;
            } else {
                self.count += 1.0;
            }
            y = self.held;
        }
        y
    }
}

/// Voices a Sampler has room for from the start (more grow the list).
const VOICE_ROOM: usize = 24;

#[derive(Clone, Copy)]
struct Voice {
    id: u64,
    note: i64,
    /// The key its pitch is of: the note, or (Slice Motion) its zone's root.
    pitch: i64,
    zone: usize,
    vel: f32,
    age: u64,
    stream: Stream,
    /// x[n−3] .. x[n+4] and the 16.16 fraction (§7).
    hist: [[f32; 2]; 8],
    frac: u32,
    /// Frames shifted in after the stream ended (the voice ends when the tail is out).
    tail: u32,
    amp: Env,
    amp_on: bool,
    modenv: Env,
    lfo_phase: f64,
    lfo_inc: f64,
    lfo_value: f32,
    lfo_seed: u32,
    /// Pitch in octaves, without glide and modulation.
    octaves: f32,
    rate_ratio: f64,
    /// The Keys render it reads ([`ZoneSync::notes`]), if any.
    variant: Option<usize>,
    /// The stretcher it plays through ([`SamplerRt::lives`]), or [`NO_LIVE`].
    live: u8,
    /// Its zone is a one-shot: the note's end leaves it playing.
    once: bool,
    inc: u32,
    /// Static gain (zone gain, velocity → volume) and zone pan, per channel.
    gain: [f32; 2],
    pan_mod: f32,
    ctl_left: u32,
    // Effects.
    comp: Comp,
    drive: Drive,
    drive_target: f32,
    lofi: Lofi,
    filter: [Biquad; 2],
    /// The filter mode the filter state belongs to.
    filter_mode: i64,
    /// Pan modulation target (set each control step; `pan_mod` follows it).
    pan_target: f32,
    cutoff: f32,
    cutoff_set: bool,
    eq: Biquad,
    /// Stem lanes: how much of the sample itself it plays (1: all of it, 0: the lanes'
    /// sum), and each lane's level, both on their way to the zone's.
    whole: f32,
    lane: [f32; LANES],
    /// Frames of silence before it starts (a Sync note waiting for its grid line).
    wait: u32,
    /// A short fade-in (a Sync note started late, inside its sample): frames left.
    rise: u32,
    /// Linear fade-out (stolen, CC 120, stop): (frames left, total).
    fade: Option<(u32, u32)>,
    released: bool,
    sustained: bool,
    /// A layer's voice: its envelope, its gain (window, mute, solo) on its way to
    /// `layer_to`, how far through the morph's cycle it is (a sweep of its own), and
    /// where that puts the morph.
    layer_env: Option<Env>,
    layer_gain: f32,
    layer_to: f32,
    /// What its velocity makes of its zone's gain, and the gain it's going to (its
    /// zone's level or pan moved under it: a layer's).
    vel_gain: f32,
    gain_to: [f32; 2],
    sweep: f64,
    morph_at: f32,
}

/// The fade-in of a Sync note started inside its sample, frames at most.
const RISE: u32 = 144;

/// Modulation and transport state shared by the voices of one Sampler.
pub struct Context {
    pub tempo: f64,
    /// Song position in beats (quarter notes), for the LFO's Lock mode.
    pub beats: f64,
    /// The note is played live while the song runs (a Sync zone starts it on its grid).
    pub live: bool,
}

/// One sound's Sampler: settings, zones, voices and the controllers.
pub struct SamplerRt {
    pub settings: Settings,
    base: Settings,
    hit: bool,
    modulated: bool,
    pub zones: Vec<ZoneRt>,
    /// Sound and group level/pan combined.
    pub mix: (f32, f32),
    sr: f64,
    voices: Vec<Voice>,
    clock: u64,
    coeffs: Coeffs,
    mod_wheel: f32,
    bend: f32,
    pedal: bool,
    /// Keys held, in order (legato and glide).
    held: Vec<i64>,
    glide: f32,
    glide_target: f32,
    glide_step: f32,
    first_note: i64,
    /// Free-running LFO phase (cycles).
    lfo_phase: f64,
    seed: u32,
    last_peak: f32,
    /// The synth playing instead of zones: its class, and room to render it.
    synth: Option<(&'static str, Box<dyn Synth>, Vec<f32>, Vec<f32>)>,
    /// Takes the synth to new settings (knob moves).
    synth_glide: Option<crate::glide::Glide>,
    /// The Audio plug-in's loop playing instead of zones.
    audio: Option<Box<crate::audio_loop::LoopRt>>,
    /// The song's tempo (Sync and Keys zones follow it).
    tempo: f64,
    /// The stretchers of zones stretched as they play, made with it (off the audio
    /// thread) and given to voices as they start.
    lives: Lives,
    /// Frames rendered, and the frame the next stretcher may start at.
    frame: u64,
    live_from: u64,
    /// Slice Motion: which of the zones a note plays.
    pub motion: Motion,
    /// The next slice Next plays, and the one played last (both counted in key order
    /// among the zones that can play).
    turn: usize,
    last: usize,
    /// The morph of its layers (`None`: it has none), POSITION as it plays now (a
    /// modulation may hold it elsewhere), and how far through its cycle the sound's
    /// own sweep is.
    morph: Option<Morph>,
    morph_pos: f32,
    sweep: f64,
    /// Where the song was at the last render (beats), and whether it had moved.
    song: (f64, bool),
    /// Beats into the note starting now (see [`SamplerRt::note_on_at`]); 0 otherwise.
    skip: f64,
}

impl SamplerRt {
    pub fn new(settings: Settings, zones: Vec<ZoneRt>, mix: (f32, f32), sr: f64) -> SamplerRt {
        let zones_len = zones.len();
        let base = settings.clone();
        let live = zones.iter().any(|z| z.sync.as_ref().is_some_and(|s| s.live.is_some()));
        let lives = if live { Live::pool(sr) } else { Lives::default() };
        SamplerRt {
            lives,
            frame: 0,
            live_from: 0,
            motion: Motion::Off,
            turn: 0,
            last: usize::MAX,
            morph: None,
            morph_pos: 0.0,
            sweep: 0.0,
            song: (0.0, false),
            settings,
            base,
            hit: false,
            modulated: false,
            zones,
            mix,
            sr,
            // Room for the voices a sound usually plays, made here (on the UI thread):
            // growing it while notes start would allocate on the audio thread.
            voices: Vec::with_capacity(if zones_len == 0 { 0 } else { VOICE_ROOM }),
            clock: 0,
            coeffs: Coeffs { c50: smoothing(50.0, sr), gain: smoothing(2.67874, sr), release: smoothing(25.4764, sr) },
            mod_wheel: 0.0,
            bend: 0.0,
            pedal: false,
            held: Vec::with_capacity(16),
            glide: 0.0,
            glide_target: 0.0,
            glide_step: 0.0,
            first_note: 0,
            lfo_phase: 0.0,
            seed: 0x1234_5678,
            last_peak: 0.0,
            synth: None,
            synth_glide: None,
            audio: None,
            tempo: 0.0,
            skip: 0.0,
        }
    }

    /// A sound played by the Audio plug-in (its loop, kept in time with the song).
    pub fn audio(audio: crate::audio_loop::LoopRt, sr: f64) -> SamplerRt {
        let mut s = SamplerRt::new(Settings::for_synth(&[], sr), Vec::new(), (1.0, 1.0), sr);
        s.audio = Some(Box::new(audio));
        s
    }

    /// Where the song is (beats) and its tempo, at the start of the next render (the
    /// Audio plug-in follows it).
    pub fn set_song(&mut self, beats: f64, tempo: f64) {
        self.tempo = tempo;
        self.song = (beats, beats != self.song.0);
        if let Some(a) = self.audio.as_mut() {
            a.set_song(beats, tempo);
        }
    }

    /// The Audio plug-in plays take `take` from song beat `origin` on, or stops.
    pub fn cue(&mut self, cue: Option<(usize, f64)>) {
        if let Some(a) = self.audio.as_mut() {
            a.cue(cue);
        }
    }

    /// The Audio plug-in's take playing, if any.
    pub fn cued(&self) -> Option<usize> {
        self.audio.as_ref().and_then(|a| a.cued())
    }

    /// A sound played by the synth of module `class` (see [`crate::synth::make`]), if
    /// DECKARD has it.
    pub fn synth(class: &str, params: &[(String, f64)], mix: (f32, f32), sr: f64) -> Option<SamplerRt> {
        let class = crate::synth::CLASSES.iter().copied().find(|c| *c == class)?;
        let synth = crate::synth::make(class, params, sr)?;
        let mut s = SamplerRt::new(Settings::for_synth(params, sr), Vec::new(), mix, sr);
        s.synth = Some((class, synth, vec![0.0; SYNTH_CHUNK], vec![0.0; SYNTH_CHUNK]));
        s.synth_glide = Some(crate::glide::Glide::new(&s.settings.synth, sr));
        Some(s)
    }

    /// Start modulating this span: if the previous span left modulated values in
    /// `settings`, restore the unmodulated `base` first so only fields hit this span
    /// move away from their knobs.
    pub(crate) fn begin_modulation(&mut self) {
        if !self.hit {
            if self.modulated {
                self.settings.copy_values_from(&self.base);
            }
            self.hit = true;
        }
    }

    /// End the span's modulation: if a previous span modulated `settings` and nothing
    /// hit this span, restore `base` (and retarget the synth's glide back to `base`).
    pub(crate) fn finish_modulation(&mut self) -> bool {
        if !self.hit && self.modulated {
            self.settings.copy_values_from(&self.base);
            self.morph_pos = self.morph.map_or(0.0, |m| m.position.clamp(0.0, 1.0) as f32);
            if let Some((_, synth, ..)) = self.synth.as_mut() {
                let params = &self.settings.synth;
                match self.synth_glide.as_mut() {
                    Some(g) => {
                        if !g.heading_for(params) {
                            synth.set(if g.retarget(params) { g.now() } else { params });
                        }
                    }
                    None => synth.set(params),
                }
            }
        }
        self.modulated = self.hit;
        self.hit = false;
        self.modulated
    }

    /// Modulation (the audio thread): field `k` plays at stored value `v` until the next
    /// edit or modulation: a Sampler field ([`MODULATED`]), or a parameter of the synth
    /// playing (one it has; it glides there).
    pub fn modulate(&mut self, k: &str, v: f64) {
        if k == "morph" {
            if v.is_finite() {
                self.morph_pos = v.clamp(0.0, 1.0) as f32;
            }
            return;
        }
        let Some((_, synth, ..)) = self.synth.as_mut() else {
            self.settings.set_field(k, v, self.sr);
            return;
        };
        let Some(x) = self.settings.synth.iter_mut().find(|(f, _)| f == k) else { return };
        if x.1 == v {
            return;
        }
        x.1 = v;
        let params = &self.settings.synth;
        match self.synth_glide.as_mut() {
            Some(g) => synth.set(if g.retarget(params) { g.now() } else { params }),
            None => synth.set(params),
        }
    }

    #[cfg(test)]
    pub(crate) fn morph_pos(&self) -> f32 {
        self.morph_pos
    }

    #[cfg(test)]
    pub(crate) fn settings_cutoff(&self) -> f32 {
        self.settings.cutoff
    }

    #[cfg(test)]
    pub(crate) fn synth_param(&self, k: &str) -> Option<f64> {
        self.settings.synth.iter().find(|(f, _)| f == k).map(|x| x.1)
    }

    /// Put in new settings (an edit), handing back the old ones in `st`.
    pub fn swap_settings(&mut self, st: &mut Settings) {
        std::mem::swap(&mut self.settings, st);
        self.base.copy_values_from(&self.settings);
        self.modulated = false;
        if let Some((_, synth, ..)) = self.synth.as_mut() {
            let params = &self.settings.synth;
            match self.synth_glide.as_mut() {
                Some(g) => synth.set(if g.retarget(params) { g.now() } else { params }),
                None => synth.set(params),
            }
        }
    }

    /// Whether [`SamplerRt::render`] must run: while voices sound, and always for a
    /// synth (its control clock, LFOs and noise run on while silent, as in Maschine).
    pub fn runs(&self) -> bool {
        self.synth.is_some() || !self.voices.is_empty() || self.audio.as_ref().is_some_and(|a| a.runs())
    }

    pub fn voices(&self) -> usize {
        if let Some(a) = &self.audio {
            return usize::from(a.runs());
        }
        match &self.synth {
            Some((_, s, ..)) => s.voices(),
            None => self.voices.len(),
        }
    }

    /// Where the newest voice playing zone `zone` is in its sample (frames), if any:
    /// the Sample Editor's playhead.
    pub fn play_position(&self, zone: usize) -> Option<usize> {
        // (A stretcher fed its zone's last frame still has its end to give: it plays on.)
        let ending = |v: &Voice| self.lives.get(v.live as usize).and_then(|l| l.left).filter(|&n| n > 0);
        let plays = |v: &&Voice| v.zone == zone && v.fade.is_none() && (!v.stream.done || ending(v).is_some());
        self.voices.iter().filter(plays).max_by_key(|v| v.age).map(|v| {
            let mut pos = v.stream.pos.max(0) as f64;
            // A stretcher is fed ahead of what it gives: the frame heard is behind.
            if let Some(l) = self.lives.get(v.live as usize) {
                let mut ahead = l.lat.0 as f64 + l.lat.1 as f64 * l.rate;
                // Fed to the end: what it has left to give, back from there.
                if let (true, Some(left)) = (v.stream.done, ending(v)) {
                    pos = if v.stream.forward { v.stream.play.1 as f64 } else { v.stream.play.0 as f64 };
                    ahead = f64::from(left) * l.rate;
                }
                pos = if v.stream.forward {
                    (pos - ahead).max(v.stream.play.0 as f64)
                } else {
                    (pos + ahead).min(v.stream.play.1 as f64 - 1.0)
                };
            }
            // A render's frame, back to the sample's.
            match self.zones.get(zone).and_then(|z| z.sync.as_ref()) {
                Some(sync) => sync.source(pos).max(0.0) as usize,
                None => pos as usize,
            }
        })
    }

    /// The peak level of the last [`SamplerRt::render`], before the engine's clamp.
    pub fn last_peak(&self) -> f32 {
        self.last_peak
    }

    /// Take over the voices and controller state of `old` (the same sound before a
    /// project change). Voices of zones that changed sample fade out.
    pub fn adopt(&mut self, mut old: SamplerRt) {
        if let (Some(new), Some(o)) = (self.audio.as_mut(), old.audio.take()) {
            new.adopt(*o);
            return;
        }
        // The same synth keeps playing, with the new settings.
        if let (Some(new), Some(o)) = (self.synth.as_mut(), old.synth.take()) {
            if new.0 == o.0 {
                *new = o;
                new.1.set(&self.settings.synth);
            }
            return;
        }
        let same = |a: &ZoneRt, b: &ZoneRt| {
            // (The same file loaded again is the same sample.)
            Arc::ptr_eq(&a.sample, &b.sample) || (a.sample.frames.len() == b.sample.frames.len() && a.sample.rate == b.sample.rate)
        };
        // Voices stretched as they play keep their stretchers.
        if !old.lives.is_empty() && !self.lives.is_empty() {
            std::mem::swap(&mut self.lives, &mut old.lives);
        }
        let stretched = !self.lives.is_empty();
        for mut v in old.voices.drain(..) {
            // A layer's voice goes with its sample, wherever its layer is now. One whose
            // layer is gone fades out on what it was playing (kept as a zone no key plays).
            let was = v.zone;
            if let Some(a) = old.zones.get(was).filter(|a| a.layer.is_some()) {
                let its = |b: &ZoneRt| b.layer.is_some() && Arc::ptr_eq(&a.sample, &b.sample);
                if !self.zones.get(was).is_some_and(its) {
                    match self.zones.iter().position(its) {
                        Some(i) => v.zone = i,
                        None if self.zones.len() < self.zones.capacity() => {
                            self.zones.push(ZoneRt { keys: (1, 0), ..a.clone() });
                            v.zone = self.zones.len() - 1;
                            v.fade_out(self.sr);
                        }
                        None => continue,
                    }
                }
                // Its level or pan may have moved: it goes there (ramped as it plays).
                if let Some(b) = self.zones.get(v.zone) {
                    v.gain_to = zone_amp(b).map(|c| c * v.vel_gain);
                }
            }
            let live = self.zones.get(v.zone).and_then(|b| b.sync.as_ref()).and_then(|sy| Some((sy.live?, sy.keys)));
            match live {
                // Its Tune may have moved.
                Some((semis, keys)) if stretched && v.live != NO_LIVE => {
                    let root = self.zones[v.zone].root;
                    v.octaves = semis as f32 / 12.0 + if keys { (v.pitch - root) as f32 / 12.0 } else { 0.0 };
                }
                _ => v.live = NO_LIVE,
            }
            match (old.zones.get(was), self.zones.get(v.zone)) {
                (Some(a), Some(b)) if same(a, b) && a.sync.is_none() == b.sync.is_none() => {
                    v.variant =
                        b.sync.as_ref().and_then(|sy| v.variant.and_then(|i| a.sync.as_ref()?.notes.get(i)).and_then(|(o, _)| sy.note(*o)));
                    // Its play range, loop or fades moved (the sample its own still): it
                    // goes on in the new ones, heard at once.
                    let ranges = |z: &ZoneRt| (z.play, z.looped, z.x_fade, z.fade_in, z.fade_out);
                    if v.variant.is_none() && ranges(a) != ranges(b) {
                        v.stream.play = (b.play.0 as i64, b.play.1 as i64);
                        if v.stream.looped.is_some() {
                            v.stream.looped = b.looped.map(|(l, r)| (l as i64, r as i64));
                        }
                        (v.stream.x_fade, v.stream.fade_in, v.stream.fade_out) = (b.x_fade as i64, b.fade_in as i64, b.fade_out as i64);
                        v.stream.fading = 0;
                    }
                }
                // A Sync or Keys zone given another render (a new tempo, or the first):
                // its voices carry on from the same place in the music, in the new one.
                (Some(a), Some(b)) if a.sync.is_some() && b.sync.is_some() && a.sample.rate == b.sample.rate => {
                    let (la, lb) = ((a.play.1 - a.play.0).max(1) as f64, (b.play.1 - b.play.0) as f64);
                    let at = (v.stream.pos - a.play.0 as i64) as f64 / la;
                    v.stream.pos = b.play.0 as i64 + (at * lb) as i64;
                    v.stream.play = (b.play.0 as i64, b.play.1 as i64);
                    if v.stream.looped.is_some() {
                        v.stream.looped = b.looped.map(|(l, r)| (l as i64, r as i64));
                    }
                    v.stream.fade_in = b.fade_in as i64;
                    v.stream.fade_out = b.fade_out as i64;
                    v.stream.x_fade = b.x_fade as i64;
                    v.stream.fading = 0;
                    let offset = v.pitch - b.root;
                    v.variant = b.sync.as_ref().and_then(|sy| sy.note(offset));
                    let keys = b.sync.as_ref().is_some_and(|sy| sy.keys);
                    v.octaves = if keys && v.variant.is_none() { offset as f32 / 12.0 } else { 0.0 };
                    if (v.stream.pos.max(0) as usize) >= b.sample.frames.len() {
                        continue;
                    }
                }
                (Some(a), Some(b)) if a.sample.rate == b.sample.rate && (v.stream.pos.max(0) as usize) < b.sample.frames.len() => {
                    v.stream.play = (b.play.0 as i64, b.play.1 as i64);
                    v.stream.looped = b.looped.map(|(l, r)| (l as i64, r as i64));
                    v.fade_out(self.sr);
                }
                _ => continue,
            }
            self.voices.push(v);
        }
        self.clock = old.clock;
        (self.turn, self.last) = (old.turn, old.last);
        (self.frame, self.live_from) = (old.frame, old.live_from);
        (self.mod_wheel, self.bend, self.pedal) = (old.mod_wheel, old.bend, old.pedal);
        self.held = old.held;
        (self.glide, self.glide_target, self.glide_step, self.first_note) = (old.glide, old.glide_target, old.glide_step, old.first_note);
        self.lfo_phase = old.lfo_phase;
        // The morph goes on from where it was.
        (self.sweep, self.song) = (old.sweep, old.song);
    }

    /// The sound's layers have this morph (`None`: it has no layers).
    pub fn set_morph(&mut self, morph: Option<Morph>) {
        self.morph = morph;
        // Room for a held chord of them all, made here (off the audio thread).
        if morph.is_some() {
            self.voices.reserve(MAX_VOICES.saturating_sub(self.voices.len()));
            // And for layers taken away while they sound ([`SamplerRt::adopt`]).
            self.zones.reserve(deckard_model::layers::MAX_LAYERS);
        }
        self.morph_pos = morph.map_or(0.0, |m| m.position.clamp(0.0, 1.0) as f32);
    }

    /// Where the morph is (0 to 1) for the newest note sounding; `None` with the morph
    /// off or nothing sounding.
    pub fn morph_position(&self) -> Option<f32> {
        self.morph.filter(|m| m.on)?;
        self.voices.iter().filter(|v| v.layer_env.is_some() && v.fade.is_none()).max_by_key(|v| v.age).map(|v| v.morph_at)
    }

    fn free_lfo_hz(&self) -> f64 {
        lfo_rate(self.settings.lfo_speed as f64).0
    }

    /// Whether this sound plays sample zones, which can join a pattern's note part-way
    /// through (a synth or the Audio plug-in can't).
    pub fn can_chase(&self) -> bool {
        self.synth.is_none() && self.audio.is_none() && !self.zones.is_empty()
    }

    /// A note that started `skip` beats ago (its pattern put in under a playing song):
    /// its zones play from as far in as they would be by now. Only zones: a synth's or
    /// the Audio plug-in's note isn't joined late.
    pub fn note_on_at(&mut self, id: u64, note: i64, velocity: i64, ctx: &Context, skip: f64) {
        if skip > 0.0 && !self.can_chase() {
            return;
        }
        self.skip = skip.max(0.0);
        self.note_on(id, note, velocity, ctx);
        self.skip = 0.0;
    }

    /// Note-on (§9). `id` identifies the note for its note-off.
    pub fn note_on(&mut self, id: u64, note: i64, velocity: i64, ctx: &Context) {
        self.tempo = ctx.tempo;
        if let Some(a) = self.audio.as_mut() {
            a.set_song(ctx.beats, ctx.tempo);
            a.note_on(id, note);
            return;
        }
        if let Some((_, s, ..)) = self.synth.as_mut() {
            s.set_tempo(ctx.tempo);
            s.note_on(note, if velocity <= 0 { 64 } else { velocity.min(127) });
            return;
        }
        let velocity = if velocity <= 0 { 64 } else { velocity.min(127) };
        // Slice Motion: the note plays one slice, whichever key it is.
        let in_motion = self.motion != Motion::Off && self.zones.iter().any(|z| z.play.1 > z.play.0);
        if !in_motion
            && !self
                .zones
                .iter()
                .any(|z| (z.keys.0..=z.keys.1).contains(&note) && (z.vels.0..=z.vels.1).contains(&velocity) && z.play.1 > z.play.0)
        {
            return;
        }
        let (legato, glide) = (self.settings.legato(), self.settings.glide);
        if legato {
            let had = !self.held.is_empty();
            self.held.retain(|&n| n != note);
            self.held.push(note);
            let frames = (glide * glide * self.sr as f32).round().max(1.0);
            if had {
                // The sounding voice glides to the new key.
                self.glide_target = (note - self.first_note) as f32 / 12.0;
                self.glide_step = (self.glide_target - self.glide) / frames;
                if self.voices.iter().any(|v| v.fade.is_none()) {
                    return;
                }
            }
            for v in &mut self.voices {
                v.fade_out(self.sr);
            }
            self.first_note = note;
            (self.glide, self.glide_target, self.glide_step) = (0.0, 0.0, 0.0);
        }
        // A layered sound's key struck again: what still rings of it from before (let go
        // already) makes way. The two would add or cancel by where each is in its wave,
        // so the level would jump from one hit to the next. [DECKARD's choice]
        for v in self.voices.iter_mut().filter(|v| v.note == note && v.released && v.layer_env.is_some()) {
            v.fade_out(self.sr);
        }
        self.clock += 1;
        let age = self.clock;
        // (Chosen here: a legato note that only glides takes no turn.)
        let moved = if in_motion { self.next_slice() } else { None };
        let mut started = false;
        for zi in 0..self.zones.len() {
            let z = &self.zones[zi];
            let plays = match moved {
                Some(m) => m == zi,
                None => (z.keys.0..=z.keys.1).contains(&note) && (z.vels.0..=z.vels.1).contains(&velocity) && z.play.1 > z.play.0,
            };
            if !plays {
                continue;
            }
            // (A slice in motion plays at its own pitch.)
            let pitch = if moved.is_some() { z.root } else { note };
            let v = self.make_voice(id, note, pitch, velocity, zi, age, ctx);
            // No room: the oldest note goes, all its voices (its layers) together.
            if self.voices.len() >= MAX_VOICES
                && let Some(oldest) = self.voices.iter().map(|v| v.age).filter(|&a| a != age).min()
            {
                self.voices.retain(|v| v.age != oldest);
            }
            if self.voices.len() >= MAX_VOICES {
                continue;
            }
            self.voices.push(v);
            started = true;
        }
        if !started {
            return;
        }
        // Voice limit, counted in notes (a note's zones share its age): fade out the
        // oldest beyond it. Without allocating (this runs on the audio thread).
        let limit = self.settings.note_limit();
        loop {
            let v = &self.voices;
            // Live voices whose age no earlier live voice has: one per note.
            let notes =
                (0..v.len()).filter(|&i| v[i].fade.is_none() && !v[..i].iter().any(|w| w.fade.is_none() && w.age == v[i].age)).count();
            if notes <= limit {
                break;
            }
            let Some(oldest) = self.voices.iter().filter(|v| v.fade.is_none()).map(|v| v.age).min() else { break };
            for v in self.voices.iter_mut().filter(|v| v.age == oldest) {
                v.fade_out(self.sr);
            }
        }
    }

    /// Slice Motion: the zone the next note plays (`None`: Motion is off, or no zone can
    /// play). Without allocating: it runs on the audio thread.
    fn next_slice(&mut self) -> Option<usize> {
        if self.motion == Motion::Off {
            return None;
        }
        let zones = &self.zones;
        let playable = |i: usize| zones[i].play.1 > zones[i].play.0;
        let n = (0..zones.len()).filter(|&i| playable(i)).count();
        if n == 0 {
            return None;
        }
        let k = if self.motion == Motion::Random && n > 1 {
            self.seed = self.seed.wrapping_mul(0x0BB3_8435).wrapping_add(0x3619_636B);
            let draw = (self.seed >> 8) as usize;
            // One of the others than the last.
            if self.last < n { (self.last + 1 + draw % (n - 1)) % n } else { draw % n }
        } else {
            self.turn % n
        };
        self.turn = (k + 1) % n;
        self.last = k;
        // The `k`th in key order (zones on one key: in their own order).
        let before = |i: usize, j: usize| (zones[j].keys.0, j) < (zones[i].keys.0, i);
        (0..zones.len()).find(|&i| playable(i) && (0..zones.len()).filter(|&j| playable(j) && before(i, j)).count() == k)
    }

    #[allow(clippy::too_many_arguments)]
    fn make_voice(&mut self, id: u64, note: i64, pitch: i64, velocity: i64, zi: usize, age: u64, ctx: &Context) -> Voice {
        let s = self.settings.clone();
        let free_hz = self.free_lfo_hz();
        self.seed = self.seed.wrapping_mul(0x0BB3_8435).wrapping_add(0x3619_636B);
        let z = &self.zones[zi];
        let vel = velocity as f32 * 0.007874016;
        let mw = self.mod_wheel;
        // Amp envelope: velocity → decay re-derives the decay time (§3).
        let mut amp_t = s.amp;
        if s.vel_decay != 0.0 {
            let a = s.vel_decay;
            let tt = t::lookup(&t::VELOCITY_DECAY_CURVE, s.amp_decay_knob);
            let u = if a >= 0.0 { 1.0 - vel } else { vel };
            let ms = (((1.0 - a.abs() * u) * tt * 8.922792) as f64 + std::f64::consts::LN_2).exp() - 2.0;
            amp_t.decay = ms_to_frames(ms, self.sr);
        }
        let (amp_on, gated) = match s.env_type {
            0 => (false, false),
            1 => (true, false),
            _ => (true, true),
        };
        if s.env_type == 1 {
            amp_t.release = 0;
            amp_t.sustain = 0.0;
        } else if s.env_type == 2 {
            amp_t.hold = 0;
        }
        let mut mod_t = s.modenv;
        if s.env_type == 2 {
            mod_t.hold = 0;
        } else {
            mod_t.sustain = 0.0;
        }
        // Start (§6.2) and the stream.
        let len = (z.play.1 - z.play.0) as f32;
        let start = (s.start + vel * s.vel_start + mw * s.mw_start).clamp(0.0, 1.0);
        let offset = (start * len) as i64;
        let forward = s.reverse == z.reverse;
        let looping = amp_on && z.looped.is_some();
        // A Sync note played live starts on its grid: the nearest line, waited for, or
        // (late) caught up with by starting that far in.
        let (mut wait, mut rise) = (0u32, 0u32);
        let mut offset = offset;
        if let Some(sy) = z.sync.as_ref().filter(|sy| ctx.live && !sy.keys && sy.grid > 0.0 && sy.tempo > 0.0 && ctx.tempo > 0.0) {
            let late = ctx.beats.rem_euclid(sy.grid);
            if late > sy.grid / 2.0 {
                wait = ((sy.grid - late) * 60.0 / ctx.tempo * self.sr).round() as u32;
            } else {
                // Beats to frames of the zone's sample (in time at `sy.tempo`).
                let skip = (late * 60.0 / sy.tempo * z.sample.rate as f64).round() as i64;
                if skip > (z.sample.rate / 500) as i64 && offset + skip < len as i64 {
                    offset += skip;
                    rise = RISE.min((self.sr * 0.003) as u32).max(1);
                }
            }
        }
        // A note joined late: as far in as it would be by now (a loop wrapped round),
        // faded in rather than clicking.
        if self.skip > 0.0 && forward && ctx.tempo > 0.0 {
            let rate = z.sample.rate as f64;
            let frames = match &z.sync {
                Some(sy) if sy.tempo > 0.0 => self.skip * 60.0 / sy.tempo * rate,
                _ => {
                    let octaves = z.tune / 12.0 + s.tune + (note - z.root) as f32 / 12.0;
                    self.skip * 60.0 / ctx.tempo * rate * 2f64.powf(octaves as f64)
                }
            };
            let mut at = z.play.0 as i64 + offset + frames.round() as i64;
            if let Some((a, b)) = z.looped.filter(|_| looping)
                && at >= b as i64
                && b > a
            {
                at = a as i64 + (at - a as i64).rem_euclid((b - a) as i64);
            }
            offset = (at - z.play.0 as i64).clamp(0, (len as i64 - 1).max(0));
            rise = RISE.min((self.sr * 0.003) as u32).max(1);
        }
        let stream = Stream {
            pos: match z.looped {
                _ if forward => z.play.0 as i64 + offset,
                // A slice of its own that loops backwards starts at its loop's end.
                Some((a, b)) if looping && z.reverse => (b as i64 - 1 - offset).max(a as i64),
                _ => z.play.1 as i64 - 1 - offset,
            },
            forward,
            play: (z.play.0 as i64, z.play.1 as i64),
            looped: if looping { z.looped.map(|(a, b)| (a as i64, b as i64)) } else { None },
            x_fade: z.x_fade as i64,
            fade_in: z.fade_in as i64,
            fade_out: z.fade_out as i64,
            fading: 0,
            done: false,
            // (A loop goes round instead; a zone fade at that edge has ended it already, and
            // a note started at the very end has played nothing to come back through.)
            tail: if looping || (if forward { z.fade_out } else { z.fade_in }) > 0 || offset >= len as i64 {
                0
            } else {
                (f64::from(z.tail_ms.max(0.0)) / 1000.0 * f64::from(z.sample.rate)) as i64
            },
            tail_left: 0,
            tail_len: 0,
        };
        // Gain and pan (§6.6).
        let a = s.vel_volume;
        let u = if a >= 0.0 { 1.0 - vel } else { vel };
        let vel_gain = (1.0 - a.abs() * u).powi(3);
        let gain = zone_amp(z).map(|c| c * vel_gain);
        // LFO (§4).
        let (lfo_hz, phase) = match s.lfo_sync {
            0 => (free_hz, self.lfo_phase + s.lfo_phase as f64),
            sync => {
                let (_, (num, den)) = lfo_rate(s.lfo_speed as f64);
                let ratio = den as f64 / num as f64 * 0.25;
                let hz = ctx.tempo / 60.0 * ratio;
                if sync == 1 { (hz, s.lfo_phase as f64) } else { (hz, s.lfo_phase as f64 + ratio * ctx.beats.rem_euclid(1.0 / ratio)) }
            }
        };
        // Sync and Keys: Tune is in the render (the length stays); Keys transposes by
        // the note, from its render if there is one.
        let (octaves, variant) = match &z.sync {
            None => (z.tune / 12.0 + s.tune + (pitch - z.root) as f32 / 12.0, None),
            // Stretched as it plays: its Tune and (Keys) the note are the stretcher's.
            Some(sy) if sy.live.is_some() => {
                (sy.live.unwrap_or(0.0) as f32 / 12.0 + if sy.keys { (pitch - z.root) as f32 / 12.0 } else { 0.0 }, None)
            }
            Some(sy) => match sy.note(pitch - z.root) {
                Some(i) => (0.0, Some(i)),
                None if sy.keys => ((pitch - z.root) as f32 / 12.0, None),
                None => (0.0, None),
            },
        };
        let mut v = Voice {
            id,
            note,
            pitch,
            zone: zi,
            vel,
            age,
            stream,
            hist: [[0.0; 2]; 8],
            frac: 0,
            tail: 0,
            amp: Env::new(amp_t, gated),
            amp_on,
            modenv: Env::new(mod_t, gated),
            lfo_phase: phase.rem_euclid(1.0),
            lfo_inc: lfo_hz / self.sr,
            lfo_value: 0.0,
            lfo_seed: self.seed,
            octaves,
            rate_ratio: z.sample.rate as f64 / self.sr,
            variant,
            live: NO_LIVE,
            inc: 65536,
            gain,
            pan_mod: 0.0,
            ctl_left: 0,
            comp: Comp { g: 1.0, ..Comp::default() },
            drive: Drive::default(),
            drive_target: 0.0,
            lofi: Lofi::default(),
            filter: [Biquad::default(); 2],
            filter_mode: s.filter,
            pan_target: 0.0,
            cutoff: 0.0,
            cutoff_set: false,
            eq: Biquad::default(),
            whole: if z.lanes_of(variant).is_some() { 0.0 } else { 1.0 },
            lane: z.lanes.as_ref().map_or([1.0; LANES], |l| l.levels),
            wait,
            rise,
            fade: None,
            released: false,
            sustained: false,
            once: z.once,
            layer_env: z.layer.map(|l| Env::new(l.env, true)),
            layer_gain: 1.0,
            layer_to: 1.0,
            vel_gain,
            gain_to: gain,
            sweep: 0.0,
            morph_at: 0.0,
        };
        if let Some(l) = &z.layer {
            // A note's own sweep starts with it; the sound's is where it is.
            let reset = self.morph.is_some_and(|m| m.reset);
            v.sweep = if reset { 0.0 } else { self.sweep };
            (v.layer_to, v.morph_at) = layer_to(l, self.morph.as_ref(), self.morph_pos, v.sweep);
            v.layer_gain = v.layer_to;
        }
        // A zone stretched as it plays: a stretcher no voice has, if one is left (else
        // it plays repitched).
        if z.sync.as_ref().is_some_and(|sy| sy.live.is_some())
            && let Some(i) = (0..self.lives.len()).find(|&i| !self.voices.iter().any(|w| w.live as usize == i))
        {
            v.live = i as u8;
            self.lives[i].fresh = true;
            let at = (self.frame + u64::from(v.wait)).max(self.live_from);
            v.wait = (at - self.frame) as u32;
            self.live_from = at + LIVE_APART;
            return v;
        }
        // Prime the interpolator: x[n] .. x[n+4] are the first frames.
        let data = z.data(variant);
        let lanes = z.lanes_of(variant);
        for k in 3..8 {
            v.hist[k] = v.stream.next_by(data.len(), &|p| mixed(data, lanes.map(|l| l.0), v.whole, &v.lane, p));
        }
        v
    }

    /// Note-off for note `id` (§9).
    pub fn note_off(&mut self, id: u64, note: i64) {
        if let Some(a) = self.audio.as_mut() {
            a.note_off(id, note);
            return;
        }
        if let Some((_, s, ..)) = self.synth.as_mut() {
            s.note_off(note);
            return;
        }
        if self.settings.legato() {
            self.held.retain(|&n| n != note);
            if let Some(&last) = self.held.last() {
                let frames = (self.settings.glide * self.settings.glide * self.sr as f32).round().max(1.0);
                self.glide_target = (last - self.first_note) as f32 / 12.0;
                self.glide_step = (self.glide_target - self.glide) / frames;
                return;
            }
            for v in &mut self.voices {
                v.release(self.pedal);
            }
            return;
        }
        for v in self.voices.iter_mut().filter(|v| v.id == id) {
            v.release(self.pedal);
        }
    }

    /// Release every voice playing `note` (a key let go).
    pub fn release_note(&mut self, note: i64) {
        if let Some(a) = self.audio.as_mut() {
            a.release_note(note);
            return;
        }
        if let Some((_, s, ..)) = self.synth.as_mut() {
            s.note_off(note);
            return;
        }
        let ids: Vec<u64> = self.voices.iter().filter(|v| v.note == note).map(|v| v.id).collect();
        for id in ids {
            self.note_off(id, note);
        }
        self.held.retain(|&n| n != note);
    }

    /// Release every held note without allocating (when the sound is muted): gated
    /// voices enter release, Performer shakers stop shaking, and one-shot hits ring out.
    pub fn all_notes_off(&mut self) {
        if let Some(a) = self.audio.as_mut() {
            a.release_all();
            return;
        }
        if let Some((_, s, ..)) = self.synth.as_mut() {
            s.all_notes_off();
            return;
        }
        self.pedal = false;
        self.held.clear();
        for v in &mut self.voices {
            v.sustained = false;
            v.release(false);
        }
    }

    /// MIDI controllers (§6.3): mod wheel, sustain pedal, all sound off, pitch bend.
    pub fn control(&mut self, kind: MidiKind) {
        if let Some(a) = self.audio.as_mut() {
            a.control(kind);
            return;
        }
        if let Some((_, s, ..)) = self.synth.as_mut() {
            s.control(kind);
            return;
        }
        match kind {
            MidiKind::Controller { number: 1, value } => self.mod_wheel = value as f32 / 127.0,
            MidiKind::Controller { number: 64, value } => {
                let down = value >= 64;
                if self.pedal && !down {
                    for v in self.voices.iter_mut().filter(|v| v.sustained) {
                        v.sustained = false;
                        v.release(false);
                    }
                }
                self.pedal = down;
            }
            MidiKind::Controller { number: 120, .. } => self.fade_all(),
            MidiKind::Controller { .. } => {}
            MidiKind::PitchBend(v) => self.bend = v as f32 / 8192.0 - 1.0,
            MidiKind::ChannelPressure(_) => {}
        }
    }

    /// Fade every voice out over 10 ms (stop, all sound off).
    pub fn fade_all(&mut self) {
        if let Some(a) = self.audio.as_mut() {
            a.stop();
            return;
        }
        if let Some((_, s, ..)) = self.synth.as_mut() {
            s.stop();
            return;
        }
        for v in &mut self.voices {
            v.fade_out(self.sr);
        }
        self.held.clear();
        (self.glide, self.glide_target, self.glide_step) = (0.0, 0.0, 0.0);
        self.pedal = false;
        self.bend = 0.0;
        // Slice Motion starts over.
        self.turn = 0;
    }

    /// Whether a loop is actively playing now (a cued Loop-mode Audio plug-in, or a
    /// looped Sampler voice that hasn't been released or choked).
    pub fn loop_playing(&self) -> bool {
        if let Some(a) = &self.audio {
            return a.loop_playing();
        }
        if self.synth.is_some() {
            return false;
        }
        self.voices.iter().any(|v| v.stream.looped.is_some() && !v.stream.done && v.fade.is_none() && !v.released)
    }

    /// Mix the voices into interleaved `out`; returns the peak (left, right).
    pub fn render(&mut self, out: &mut [f32]) -> (f32, f32) {
        if self.synth.is_some() {
            return self.render_synth(out);
        }
        if let Some(a) = self.audio.as_mut() {
            // Into a cleared buffer: the loop writes its frames (the caller's are mixed).
            let peak = a.render(out);
            self.last_peak = peak.0.max(peak.1);
            return peak;
        }
        let frames = out.len() / 2;
        self.frame += frames as u64;
        let mut peak = (0f32, 0f32);
        self.last_peak = 0.0;
        let s = self.settings.clone();
        // Lo-fi settings (§8.4).
        let b = t::lookup(&t::LOFI_BITS_CURVE, s.lofi_bits);
        let levels = (b < 0.99999).then(|| 2f32.powf((16.0 * b).max(1.0)));
        let r = t::lookup(&t::LOFI_RATE_CURVE, s.lofi_rate);
        let hold = (r < 0.99999).then(|| ((self.sr as f32) / (r * self.sr as f32).max(50.0)).max(1.0));
        let base_cutoff = t::lookup(&t::CUTOFF_CURVE, s.cutoff.clamp(0.0, 1.0));
        let q = (1.0 - (1.0 - s.resonance) * (1.0 - s.resonance)).clamp(0.0, 1.0);
        let ramp = ((self.sr * 0.02 / CHUNK as f64).round().max(1.0) * CHUNK as f64) as f32;
        let eq = (s.filter == 4).then(|| peak_eq(s.eq.0, s.eq.1, s.eq.2, self.sr));
        let lfo_scale = (1.0 - s.mw_lfo) + self.mod_wheel * s.mw_lfo;
        let free_inc = self.free_lfo_hz() / self.sr;
        let zones = &self.zones;
        let voices = &mut self.voices;
        let lives = &mut self.lives;
        let sr = self.sr;
        let coeffs = self.coeffs;
        let comp_target = s.comp.clamp(0.0, 1.0) * 50.0;
        let mw = self.mod_wheel;
        let bend = self.bend * s.bend_range;
        let tempo = self.tempo;
        let engine_gain = if s.mode == 1 { 0.8637621 } else { 1.0 };
        let quantize = s.mode >= 1;
        let lane_step = (1000.0 / (LANE_RAMP_MS * self.sr)) as f32;
        let layer_step = (1000.0 / (LAYER_RAMP_MS * self.sr)) as f32;
        // The morph: a cycle of its sweep in frames, the sound's own in step with the
        // song while that runs.
        let (morph, morph_pos) = (self.morph, self.morph_pos);
        let cycle = morph.map_or(1.0, |m| m.cycle_seconds(tempo) * self.sr).max(1.0);
        if let Some(m) = morph.filter(|m| m.sync && !m.reset && self.song.1) {
            self.sweep = (self.song.0 / (m.bars.max(deckard_model::layers::SYNCED[0]) * 4.0)).rem_euclid(1.0);
        }
        let sweep_from = self.sweep;
        self.sweep = (self.sweep + frames as f64 / cycle).fract();
        let own_sweep = morph.is_some_and(|m| m.reset);
        for f in 0..frames {
            // Glide (legato).
            if self.glide_step != 0.0 {
                self.glide += self.glide_step;
                if (self.glide_step > 0.0 && self.glide >= self.glide_target) || (self.glide_step < 0.0 && self.glide <= self.glide_target)
                {
                    self.glide = self.glide_target;
                    self.glide_step = 0.0;
                }
            }
            let glide = self.glide;
            let mut l = 0f32;
            let mut rr = 0f32;
            for v in voices.iter_mut() {
                if v.wait > 0 {
                    v.wait -= 1;
                    continue;
                }
                let z = &zones[v.zone];
                // Another filter type starts from rest, as Maschine's separate filters do.
                if v.filter_mode != s.filter {
                    v.filter_mode = s.filter;
                    v.filter.iter_mut().for_each(Biquad::reset);
                    v.eq.reset();
                    v.cutoff_set = false;
                }
                // Control rate: modulation, pitch, cutoff, drive.
                if v.ctl_left == 0 {
                    v.ctl_left = CHUNK;
                    let lfo = v.lfo(s.lfo_type);
                    let env = v.modenv.level;
                    let octaves = v.octaves + glide + s.lfo_pitch * lfo_scale * lfo + s.env_pitch * env + bend;
                    let ratio = 2f64.powf(octaves as f64) * v.rate_ratio * z.speed(tempo);
                    let ratio = if s.mode == 1 { ratio.min(1.49831) } else { ratio };
                    v.inc = (ratio * 65536.0).round().clamp(1.0, 65536.0 * 64.0) as u32;
                    if let Some(live) = lives.get_mut(v.live as usize) {
                        // The song's speed is the stretcher's rate, the pitch its own.
                        live.rate = v.rate_ratio * z.speed(tempo);
                        live.transpose = (2f64.powf(octaves as f64) * v.rate_ratio) as f32;
                    }
                    let dmod = s.env_drive * env + s.lfo_drive * lfo_scale * lfo;
                    let d = (s.drive + dmod).clamp(0.0, 1.0);
                    v.drive_target = if d <= 0.0001 { 0.0 } else { d * 127.0 };
                    if (1..=3).contains(&s.filter) {
                        let cmod = s.env_cutoff * env + s.lfo_cutoff * lfo_scale * lfo + s.vel_cutoff * v.vel + s.mw_cutoff * mw;
                        let target = base_cutoff + cmod;
                        if !v.cutoff_set {
                            v.cutoff = target;
                            v.cutoff_set = true;
                        } else {
                            v.cutoff += (target - v.cutoff) * (CHUNK as f32 / ramp).min(1.0);
                        }
                        let fr = (cutoff_fraction(v.cutoff).min(1.0) * (44100.0 / sr as f32) * 0.998004).clamp(0.0, 1.0);
                        match s.filter {
                            1 => v.filter[0].set(two_pole(fr, q, false)),
                            3 => v.filter[0].set(two_pole(fr, q, true)),
                            _ => {
                                let [(b0, b1, a1), (c0, c1, a2)] = band_pass(fr);
                                v.filter[0].set((b0, b1, 0.0, a1, 0.0));
                                v.filter[1].set((c0, c1, 0.0, a2, 0.0));
                            }
                        }
                    }
                    if let Some(c) = eq {
                        v.eq.set(c);
                    }
                    if let Some(l) = &z.layer {
                        if own_sweep {
                            v.sweep = (v.sweep + CHUNK as f64 / cycle).fract();
                        } else {
                            v.sweep = (sweep_from + f as f64 / cycle).fract();
                        }
                        (v.layer_to, v.morph_at) = layer_to(l, morph.as_ref(), morph_pos, v.sweep);
                    } else {
                        v.layer_to = 1.0;
                    }
                    let pm = s.lfo_pan * lfo_scale * lfo + s.env_pan * env + s.mw_pan * mw;
                    // Hard left to hard right at most (the amounts add up to ±3).
                    v.pan_target = pm.clamp(-1.0, 1.0);
                    v.lfo_phase += v.lfo_inc * CHUNK as f64;
                    if v.lfo_phase >= 1.0 {
                        v.lfo_phase -= v.lfo_phase.floor();
                        v.lfo_seed = v.lfo_seed.wrapping_mul(0x0BB3_8435).wrapping_add(0x3619_636B);
                    }
                }
                v.ctl_left -= 1;
                // The zone's next frame: its sample, or its stem lanes on their way to the
                // zone's levels.
                let source = |v: &mut Voice| -> [f32; 2] {
                    // Stem lanes: toward the zone's levels, and between the sample itself
                    // and the lanes' sum.
                    let data = z.data(v.variant);
                    let lanes = z.lanes_of(v.variant);
                    if lanes.is_some() || v.whole < 1.0 {
                        let toward = |x: &mut f32, to: f32| *x = if *x < to { (*x + lane_step).min(to) } else { (*x - lane_step).max(to) };
                        toward(&mut v.whole, if lanes.is_some() { 0.0 } else { 1.0 });
                        if let Some((_, levels)) = lanes {
                            for k in 0..LANES {
                                toward(&mut v.lane[k], levels[k]);
                            }
                        }
                    }
                    // (Fading back to the sample itself, the lanes still sound.)
                    let lane_samples = if v.whole < 1.0 {
                        lanes.map(|l| l.0).or(match v.variant {
                            Some(i) => z.sync.as_ref().and_then(|sy| sy.note_lanes.get(i)),
                            None => z.lanes.as_ref().map(|l| &l.samples),
                        })
                    } else {
                        None
                    };
                    let (whole, lane) = (v.whole, v.lane);
                    v.stream.next_by(data.len(), &|p| mixed(data, lane_samples, whole, &lane, p))
                };
                let mut x = [0f32; 2];
                if let Some(live) = lives.get_mut(v.live as usize) {
                    // Stretched as it plays: the stretcher takes the zone's frames as fast
                    // as the song goes.
                    x = live.next(&mut || source(v));
                    if v.stream.done {
                        let left = live.left.get_or_insert(live.after());
                        *left = left.saturating_sub(1);
                        if *left == 0 {
                            v.tail = 8;
                        }
                    }
                } else {
                    // Resample (§7).
                    let w = weights(1.0 - v.frac as f32 / 65536.0);
                    for (k, wk) in w.iter().enumerate() {
                        x[0] += wk * v.hist[k][0];
                        x[1] += wk * v.hist[k][1];
                    }
                    v.frac += v.inc;
                    while v.frac >= 65536 {
                        v.frac -= 65536;
                        v.hist.copy_within(1..8, 0);
                        let mut next = source(v);
                        if quantize {
                            next = [(next[0] * 2048.0).trunc() / 2048.0, (next[1] * 2048.0).trunc() / 2048.0];
                        }
                        v.hist[7] = next;
                        if v.stream.done {
                            v.tail += 1;
                        }
                    }
                }
                // Effects (§2, §8).
                x = v.comp.run(x, comp_target, coeffs);
                x = v.drive.run(x, v.drive_target, coeffs.c50);
                x = v.lofi.run(x, levels, hold);
                match s.filter {
                    1 | 3 => x = v.filter[0].run(x),
                    2 => {
                        let y = v.filter[0].run(x);
                        x = v.filter[1].run(y);
                    }
                    4 => x = v.eq.run(x),
                    _ => {}
                }
                // A safety net: a voice whose effects run away starts them again.
                if !(x[0].is_finite() && x[1].is_finite()) || x[0].abs().max(x[1].abs()) > 64.0 {
                    v.filter.iter_mut().for_each(Biquad::reset);
                    v.eq.reset();
                    v.comp = Comp { g: 1.0, ..Comp::default() };
                    v.drive = Drive::default();
                    x = [0.0; 2];
                }
                let amp = if v.amp_on { v.amp.next() } else { 1.0 };
                v.modenv.next();
                // Pan modulation, smoothed (§6.6).
                v.pan_mod = 0.9 * v.pan_mod + 0.1 * v.pan_target;
                let pm = &mut v.pan_mod;
                let mut fade = 1.0;
                if let Some((left, total)) = &mut v.fade {
                    fade = *left as f32 / *total as f32;
                    *left = left.saturating_sub(1);
                }
                let (pl, pr) = if *pm > 0.0 { (1.0 - *pm, 1.0) } else { (1.0, 1.0 + *pm) };
                if v.rise > 0 {
                    fade *= 1.0 - v.rise as f32 / RISE as f32;
                    v.rise -= 1;
                }
                let mut k = amp * fade * engine_gain;
                if let Some(e) = v.layer_env.as_mut() {
                    v.layer_gain = if v.layer_gain < v.layer_to {
                        (v.layer_gain + layer_step).min(v.layer_to)
                    } else {
                        (v.layer_gain - layer_step).max(v.layer_to)
                    };
                    k *= e.next() * v.layer_gain;
                }
                if v.gain != v.gain_to {
                    for c in 0..2 {
                        v.gain[c] = if v.gain[c] < v.gain_to[c] {
                            (v.gain[c] + layer_step).min(v.gain_to[c])
                        } else {
                            (v.gain[c] - layer_step).max(v.gain_to[c])
                        };
                    }
                }
                l += x[0] * v.gain[0] * pl * k;
                rr += x[1] * v.gain[1] * pr * k;
            }
            let (a, b) = (l * self.mix.0, rr * self.mix.1);
            out[f * 2] += a;
            out[f * 2 + 1] += b;
            peak = (peak.0.max(a.abs()), peak.1.max(b.abs()));
            self.last_peak = self.last_peak.max(peak.0).max(peak.1);
            // The free-running LFO.
            self.lfo_phase = (self.lfo_phase + free_inc).fract();
            // Voices that ended.
            if f % CHUNK as usize == 0 || f + 1 == frames {
                voices.retain(|v| v.alive());
            }
        }
        peak
    }

    /// [`SamplerRt::render`] for a sound played by a synth.
    fn render_synth(&mut self, out: &mut [f32]) -> (f32, f32) {
        let Some((_, synth, l, r)) = self.synth.as_mut() else { return (0.0, 0.0) };
        let mut peak = (0f32, 0f32);
        let (mut at, len) = (0, out.len());
        while at < len {
            // Gliding to new settings: a step of it every glide::STEP frames.
            let glide = self.synth_glide.as_mut().filter(|g| g.gliding());
            let frames = if glide.is_some() { crate::glide::STEP } else { SYNTH_CHUNK };
            if let Some(g) = glide
                && g.tick()
            {
                synth.set(g.now());
            }
            let chunk = &mut out[at..(at + frames * 2).min(len)];
            at += chunk.len();
            let n = chunk.len() / 2;
            synth.process(&mut l[..n], &mut r[..n]);
            for (f, x) in chunk.chunks_exact_mut(2).enumerate() {
                let (a, b) = (l[f] * self.mix.0, r[f] * self.mix.1);
                x[0] += a;
                x[1] += b;
                peak = (peak.0.max(a.abs()), peak.1.max(b.abs()));
            }
        }
        self.last_peak = peak.0.max(peak.1);
        peak
    }
}

impl Voice {
    fn lfo(&mut self, kind: i64) -> f32 {
        let p = self.lfo_phase;
        self.lfo_value = match kind {
            0 => (std::f64::consts::TAU * p).cos() as f32,
            1 => {
                let q = (p + 0.5).fract();
                if q <= 0.5 { (4.0 * q - 1.0) as f32 } else { (1.0 - 4.0 * (q - 0.5)) as f32 }
            }
            2 => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            3 => (1.0 - 2.0 * p) as f32,
            _ => f32::from_bits((self.lfo_seed & 0x7F_FFFF) | 0x4000_0000) - 3.0,
        };
        self.lfo_value
    }

    fn fade_out(&mut self, sr: f64) {
        if self.fade.is_none() {
            let n = ((sr * 0.01) as u32).max(1);
            self.fade = Some((n, n));
        }
    }

    fn release(&mut self, pedal: bool) {
        if self.once {
            return;
        }
        if pedal {
            self.sustained = true;
            return;
        }
        if !self.released {
            self.released = true;
            self.amp.release();
            self.modenv.release();
            if let Some(e) = self.layer_env.as_mut() {
                e.release();
            }
        }
    }

    fn alive(&self) -> bool {
        if matches!(self.fade, Some((0, _))) {
            return false;
        }
        if self.amp_on && self.amp.done() {
            return false;
        }
        if self.layer_env.as_ref().is_some_and(Env::done) {
            return false;
        }
        // The stream ran out and the interpolator has emptied.
        self.tail < 8
    }
}

#[cfg(test)]
mod tests {
    /// A modulated field takes the value Settings::new would give it.
    #[test]
    fn set_field_matches_new() {
        let sr = 48_000.0;
        let base = deckard_model::Sampler::default();
        for k in super::MODULATED {
            let mut m = base.clone();
            m.set(k, 0.37);
            let mut st = super::Settings::new(&base, sr);
            assert!(st.set_field(k, 0.37, sr), "{k}");
            assert!(st == super::Settings::new(&m, sr), "{k}");
        }
        assert!(!super::Settings::new(&base, sr).set_field("polyphony", 3.0, sr), "not a modulated field");
    }

    use super::*;

    /// The display's response: LP2 passes lows and cuts highs, HP2 the reverse, BP2
    /// peaks near the cutoff, the EQ shows its gain at its frequency.
    #[test]
    fn filter_response_has_the_right_shape() {
        let at = |r: &[(f32, f32)], x: f32| r.iter().min_by(|a, b| (a.0 - x).abs().total_cmp(&(b.0 - x).abs())).unwrap().1;
        let lp = filter_response(1, 0.5, 0.0, (0.5, 0.5, 0.0), 101);
        assert!(at(&lp, 0.1).abs() < 1.0 && at(&lp, 0.9) < -12.0, "LP2 {} {}", at(&lp, 0.1), at(&lp, 0.9));
        let hp = filter_response(3, 0.5, 0.0, (0.5, 0.5, 0.0), 101);
        assert!(at(&hp, 0.9).abs() < 1.5 && at(&hp, 0.1) < -12.0, "HP2 {} {}", at(&hp, 0.1), at(&hp, 0.9));
        let bp = filter_response(2, 0.5, 0.0, (0.5, 0.5, 0.0), 101);
        assert!(at(&bp, 0.5) > at(&bp, 0.05) + 6.0 && at(&bp, 0.5) > at(&bp, 0.95) + 6.0);
        // ±18 dB shown is about ±36 dB at the centre, as Maschine computes it (sampler-dsp.md).
        let eq = filter_response(4, 0.5, 0.0, (0.5, 0.3, 1.0), 101);
        assert!((at(&eq, 0.5) - 36.0).abs() < 1.0 && at(&eq, 0.0).abs() < 2.0, "EQ {}", at(&eq, 0.5));
        let res = filter_response(1, 0.5, 1.0, (0.5, 0.5, 0.0), 201);
        assert!(res.iter().map(|p| p.1).fold(f32::MIN, f32::max) > 6.0, "resonance peaks");
    }

    /// Starting notes (up to the voice limit) never grows the voice list: that would
    /// allocate on the audio thread. The limit still fades the oldest notes.
    #[test]
    fn notes_start_without_allocating() {
        let sample = Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[0.5, 0.5]; 48_000] });
        let zone = ZoneRt {
            sample,
            root: 60,
            keys: (0, 127),
            vels: (0, 127),
            gain: 1.0,
            pan: 0.0,
            tune: 0.0,
            play: (0, 48_000),
            looped: None,
            x_fade: 0,
            fade_in: 0,
            fade_out: 0,
            sync: None,
            lanes: None,
            reverse: false,
            once: false,
            tail_ms: 0.0,
            layer: None,
        };
        let model = deckard_model::Sampler::default();
        let mut smp = SamplerRt::new(Settings::new(&model, 48_000.0), vec![zone], (1.0, 1.0), 48_000.0);
        let ctx = Context { tempo: 120.0, beats: 0.0, live: false };
        let buffer = smp.voices.as_ptr();
        let limit = smp.settings.note_limit();
        for n in 0..VOICE_ROOM.min(limit + 4) as i64 {
            smp.note_on(n as u64 + 1, 40 + n, 100, &ctx);
        }
        assert_eq!(smp.voices.as_ptr(), buffer, "the voice list was not reallocated");
        let sounding = smp.voices.iter().filter(|v| v.fade.is_none()).count();
        assert!(sounding <= limit, "{sounding} notes sound, limit {limit}");
    }

    /// A Sampler of one zone (a 2 s ramp at 48 kHz), in Sync or Keys when given.
    fn synced(sync: Option<ZoneSync>) -> SamplerRt {
        let sample = Arc::new(crate::sample::Sample { rate: 48_000, frames: (0..96_000).map(|i| [i as f32 / 96_000.0; 2]).collect() });
        let zone = ZoneRt {
            sample,
            root: 60,
            keys: (0, 127),
            vels: (0, 127),
            gain: 1.0,
            pan: 0.0,
            tune: 5.0,
            play: (0, 96_000),
            looped: None,
            x_fade: 0,
            fade_in: 0,
            fade_out: 0,
            sync,
            lanes: None,
            reverse: false,
            once: false,
            tail_ms: 0.0,
            layer: None,
        };
        SamplerRt::new(Settings::new(&deckard_model::Sampler::default(), 48_000.0), vec![zone], (1.0, 1.0), 48_000.0)
    }

    /// Frames its first voice read over `n` frames of output.
    fn read(smp: &mut SamplerRt, n: usize) -> i64 {
        let from = smp.voices[0].stream.pos;
        let mut out = vec![0.0; n * 2];
        smp.render(&mut out);
        smp.voices[0].stream.pos - from
    }

    /// A zone in time at 100 BPM reads 1.2 frames a frame in a 120 BPM song, and
    /// follows the song when its tempo moves; Tune (in the render) doesn't move it.
    #[test]
    fn sync_voices_follow_the_tempo() {
        let mut smp = synced(Some(ZoneSync { tempo: 100.0, ..ZoneSync::default() }));
        smp.set_song(0.0, 120.0);
        smp.note_on(1, 60, 100, &Context { tempo: 120.0, beats: 0.0, live: false });
        let got = read(&mut smp, 4800);
        assert!((got - 5760).abs() <= 2, "read {got}");
        smp.set_song(0.0, 100.0);
        let got = read(&mut smp, 4800);
        assert!((got - 4800).abs() <= 33, "read {got}");
        // Repitch: the zone's Tune (5 semitones) moves the speed.
        let mut plain = synced(None);
        plain.set_song(0.0, 120.0);
        plain.note_on(1, 60, 100, &Context { tempo: 120.0, beats: 0.0, live: false });
        let got = read(&mut plain, 4800);
        assert!((got - (4800.0 * 2f64.powf(5.0 / 12.0)) as i64).abs() <= 2, "read {got}");
    }

    /// Sync plays every note alike; Keys transposes, from a note's render when it has
    /// one (the same length), else by speed.
    #[test]
    fn sync_ignores_the_note_and_keys_plays_its_render() {
        let ctx = Context { tempo: 100.0, beats: 0.0, live: false };
        let mut smp = synced(Some(ZoneSync { tempo: 100.0, ..ZoneSync::default() }));
        smp.set_song(0.0, 100.0);
        smp.note_on(1, 72, 100, &ctx);
        assert_eq!((smp.voices[0].octaves, smp.voices[0].variant), (0.0, None));
        let up = Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[0.25; 2]; 96_000] });
        let mut keys = synced(Some(ZoneSync { tempo: 100.0, keys: true, notes: vec![(12, up)], ..ZoneSync::default() }));
        keys.set_song(0.0, 100.0);
        keys.note_on(1, 72, 100, &ctx);
        keys.note_on(2, 67, 100, &ctx);
        let (a, b) = (&keys.voices[0], &keys.voices[1]);
        assert_eq!((a.octaves, a.variant), (0.0, Some(0)), "the octave's render");
        assert_eq!((b.octaves, b.variant), (7.0 / 12.0, None), "no render: by speed");
        assert_eq!(read(&mut keys, 480), 480, "a note's render plays at the tempo's speed");
    }

    /// A zone stretched as it plays: at a faster song it goes through its sample faster
    /// and keeps its pitch; Tune moves the pitch and not the speed; it starts at once and
    /// ends when the sample has.
    #[test]
    fn a_live_zone_keeps_its_pitch_at_any_tempo() {
        let sr = 48_000.0;
        // Two seconds of 220 Hz, in time at 100 BPM.
        let tone = Arc::new(crate::sample::Sample {
            rate: 48_000,
            frames: (0..96_000).map(|i| [(std::f64::consts::TAU * 220.0 * i as f64 / sr).sin() as f32 * 0.5; 2]).collect(),
        });
        // (Rising zero crossings a second, over the half second from 0.25 s; the peak in the first 30 ms; the frames it took.)
        let heard = |tempo: f64, semis: f64| {
            let mut smp = synced(Some(ZoneSync { tempo: 100.0, live: Some(semis), ..ZoneSync::default() }));
            smp.zones[0].tune = 0.0;
            smp.zones[0].sample = tone.clone();
            smp.set_song(0.0, tempo);
            smp.note_on(1, 60, 127, &Context { tempo, beats: 0.0, live: false });
            let mut out = vec![0.0; 48_000 * 2];
            smp.render(&mut out);
            let x: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
            let ups = x[12_000..36_000].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f64 * 2.0;
            let early = x[..1440].iter().fold(0f32, |m, v| m.max(v.abs()));
            (ups, early, smp.play_position(0).unwrap_or(0) as f64, smp)
        };
        let (hz, early, at, _) = heard(100.0, 0.0);
        assert!((hz - 220.0).abs() < 4.0 && early > 0.1, "as it is: {hz} Hz, {early} at the start");
        assert!((at - 48_000.0).abs() < 2400.0, "a second in: {at}");
        let (hz, early, at, _) = heard(150.0, 0.0);
        assert!((hz - 220.0).abs() < 4.0 && early > 0.1, "half as fast again, the same note: {hz} Hz, {early}");
        assert!((at - 72_000.0).abs() < 2400.0, "a second and a half of it in a second: {at}");
        let (hz, _, at, _) = heard(100.0, 12.0);
        assert!((hz - 440.0).abs() < 8.0, "an octave up: {hz} Hz");
        assert!((at - 48_000.0).abs() < 2400.0, "no faster: {at}");
        // Twice as fast it's over in a second, and the voice goes.
        let (_, _, _, mut smp) = heard(200.0, 0.0);
        let mut out = vec![0.0; 24_000 * 2];
        smp.render(&mut out);
        assert!(smp.voices.is_empty(), "ended with its sample");
        assert!(out[40_000..].iter().all(|v| v.abs() < 1e-3), "and silent");
    }

    /// A live zone's muted lane is out of what the stretcher is fed.
    #[test]
    fn a_live_zone_plays_its_lanes() {
        let flat = |v: f32| Some(Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[v; 2]; 96_000] }));
        let level = |levels: [f32; LANES]| {
            let mut smp = synced(Some(ZoneSync { tempo: 100.0, live: Some(0.0), ..ZoneSync::default() }));
            smp.zones[0].tune = 0.0;
            smp.zones[0].sample = Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[0.5; 2]; 96_000] });
            smp.zones[0].lanes = Some(Lanes { samples: [flat(0.1), flat(0.2), None, flat(0.4)], levels, only: false });
            smp.set_song(0.0, 120.0);
            smp.note_on(1, 60, 127, &Context { tempo: 120.0, beats: 0.0, live: false });
            let mut out = vec![0.0; 24_000 * 2];
            smp.render(&mut out);
            out[24_000]
        };
        let whole = level([1.0, 1.0, 0.0, 1.0]);
        let muted = level([0.0, 1.0, 0.0, 1.0]);
        assert!(whole > 0.0 && (muted / whole - 0.6 / 0.5).abs() < 0.05, "the other lanes' sum: {muted} against {whole}");
    }

    /// What stretching costs: `cargo test -p deckard-engine --lib live_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_cost() {
        for voices in [1u64, 4, 12] {
            let mut smp = synced(Some(ZoneSync { tempo: 100.0, live: Some(0.0), ..ZoneSync::default() }));
            smp.settings.polyphony = 32;
            smp.zones[0].sample = Arc::new(crate::sample::Sample {
                rate: 48_000,
                frames: (0..960_000u64).map(|i| [((i * 7919) % 1000) as f32 / 1000.0 - 0.5; 2]).collect(),
            });
            smp.zones[0].play = (0, 960_000);
            smp.set_song(0.0, 120.0);
            if voices == 1 {
                let t = std::time::Instant::now();
                let made: Vec<Live> = (0..LIVE_ROOM).map(|_| Live::new(48_000.0)).collect();
                let t2 = std::time::Instant::now();
                drop(Lives(made));
                let again = Live::pool(48_000.0);
                eprintln!("and had again in {:.3} ms", t2.elapsed().as_secs_f64() * 1000.0);
                let made = again;
                eprintln!("{} stretchers made in {:.1} ms", made.len(), t.elapsed().as_secs_f64() * 1000.0);
            }
            let t = std::time::Instant::now();
            for k in 0..voices {
                smp.note_on(k + 1, 60, 127, &Context { tempo: 120.0, beats: 0.0, live: false });
            }
            // A block of 128 frames lasts 2.67 ms: the worst one is what a device hears.
            let mut out = vec![0.0; 128 * 2];
            smp.render(&mut out);
            let first = t.elapsed().as_secs_f64();
            let (mut worst, mut sum) = (0f64, 0f64);
            let blocks = 5 * 48_000 / 128;
            for _ in 0..blocks {
                let t = std::time::Instant::now();
                smp.render(&mut out);
                let d = t.elapsed().as_secs_f64();
                worst = worst.max(d);
                sum += d;
            }
            eprintln!(
                "{voices} voices: the first block {:.2} ms, then {:.1} % of real time, the worst block {:.2} ms of 2.67",
                first * 1000.0,
                sum / 5.0 * 100.0,
                worst * 1000.0
            );
        }
    }

    /// More notes than stretchers: the rest play (repitched) all the same.
    #[test]
    fn notes_past_the_stretchers_still_play() {
        let mut smp = synced(Some(ZoneSync { tempo: 100.0, live: Some(0.0), ..ZoneSync::default() }));
        smp.settings.polyphony = 32;
        let ctx = Context { tempo: 100.0, beats: 0.0, live: false };
        for k in 0..LIVE_ROOM as u64 + 3 {
            smp.note_on(k + 1, 60, 127, &ctx);
        }
        let live = smp.voices.iter().filter(|v| v.live != NO_LIVE).count();
        assert_eq!(live, LIVE_ROOM.min(smp.voices.len()));
        // Notes that come together start their stretchers a block apart (a start is
        // several blocks' work); the ones without start at once.
        let waits: Vec<u64> = smp.voices.iter().filter(|v| v.live != NO_LIVE).map(|v| u64::from(v.wait)).collect();
        assert_eq!(waits, (0..live as u64).map(|k| k * LIVE_APART).collect::<Vec<_>>());
        assert!(smp.voices.iter().filter(|v| v.live == NO_LIVE).all(|v| v.wait == 0));
        let mut ids: Vec<u8> = smp.voices.iter().filter(|v| v.live != NO_LIVE).map(|v| v.live).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), live, "a stretcher a voice");
        let mut out = vec![0.0; 4800 * 2];
        smp.render(&mut out);
        assert!(out.iter().all(|v| v.is_finite()));
    }

    /// A zone set to play backwards starts on its last frame and goes down (and a sound
    /// that plays backwards plays it forwards again).
    #[test]
    fn a_reversed_zone_plays_backwards() {
        let ctx = Context { tempo: 120.0, beats: 0.0, live: false };
        // A ramp: frame f is f / 96,000.
        let ramp = Arc::new(crate::sample::Sample { rate: 48_000, frames: (0..96_000).map(|f| [f as f32 / 96_000.0; 2]).collect() });
        let heard = |reverse: bool| {
            let mut smp = synced(None);
            smp.zones[0].tune = 0.0;
            smp.zones[0].sample = ramp.clone();
            smp.zones[0].play = (24_000, 72_000);
            smp.zones[0].reverse = reverse;
            smp.note_on(1, 60, 127, &ctx);
            let mut out = vec![0.0; 4800 * 2];
            smp.render(&mut out);
            (out[2 * 480], out[2 * 4799])
        };
        let (a, b) = heard(false);
        assert!(a > 0.0 && b > a, "forwards it rises: {a} to {b}");
        let (c, d) = heard(true);
        assert!(c > b && d < c, "backwards it falls from its end: {c} to {d}");
    }

    /// A one-shot zone plays on when its note ends; a zone of its own direction that
    /// loops goes round its loop backwards, from the loop's end.
    #[test]
    fn a_slice_plays_once_or_round_its_loop() {
        let ctx = Context { tempo: 120.0, beats: 0.0, live: false };
        let ramp = Arc::new(crate::sample::Sample { rate: 48_000, frames: (0..96_000).map(|f| [f as f32 / 96_000.0; 2]).collect() });
        let zone = |once: bool| {
            let mut smp = synced(None);
            smp.settings.env_type = 2;
            smp.zones[0].tune = 0.0;
            smp.zones[0].sample = ramp.clone();
            smp.zones[0].play = (24_000, 72_000);
            smp.zones[0].once = once;
            smp
        };
        let after_the_note = |once: bool| {
            let mut smp = zone(once);
            smp.note_on(1, 60, 127, &ctx);
            smp.note_off(1, 60);
            let mut out = vec![0.0; 24_000 * 2];
            smp.render(&mut out);
            out[2 * 23_999]
        };
        assert!(after_the_note(true) > 0.4, "the one-shot plays on: {}", after_the_note(true));
        assert!(after_the_note(false).abs() < 1e-3, "held, it ends with its note: {}", after_the_note(false));
        // Backwards round frames 30,000 to 31,000: every frame heard is one of the loop's.
        let mut smp = zone(false);
        smp.zones[0].reverse = true;
        smp.zones[0].looped = Some((30_000, 31_000));
        smp.note_on(1, 60, 127, &ctx);
        let mut out = vec![0.0; 4800 * 2];
        smp.render(&mut out);
        let tail: Vec<f32> = out[2 * 1200..].chunks(2).map(|f| f[0]).collect();
        let (lo, hi) = tail.iter().fold((f32::MAX, 0f32), |a, x| (a.0.min(*x), a.1.max(*x)));
        assert!(lo > 0.0 && hi / lo < 1.04, "it stays in its loop: {lo} to {hi}");
        assert!(out[2 * 600] < out[2 * 300], "going down");
        assert!(tail.windows(2).filter(|w| w[1] > w[0] * 1.01).count() >= 3, "and round again");
    }

    /// A sampler of the 2 s ramp cut in `plays`, a zone a key from 36 (Repitch, one-shot).
    fn chopped(plays: &[(usize, usize)]) -> SamplerRt {
        let mut smp = synced(None);
        let z = ZoneRt { tune: 0.0, ..smp.zones[0].clone() };
        smp.zones = plays.iter().enumerate().map(|(i, &play)| ZoneRt { play, keys: (36 + i as i64, 36 + i as i64), ..z.clone() }).collect();
        smp
    }

    /// The first frame heard of a note of `key` (the ramp tells where in the sample).
    fn first_heard(smp: &mut SamplerRt, id: u64, key: i64) -> usize {
        let ctx = Context { tempo: 120.0, beats: 0.0, live: false };
        smp.voices.clear();
        smp.note_on(id, key, 100, &ctx);
        smp.voices.last().map_or(usize::MAX, |v| v.stream.play.0 as usize)
    }

    /// Slice Motion, Next: every note plays the next slice in key order, whichever key,
    /// at the slice's own pitch; zones that can't play are passed over, one zone plays
    /// every time, and stopping starts over.
    #[test]
    fn motion_plays_the_slices_in_turn() {
        let mut smp = chopped(&[(0, 1000), (5000, 5000), (20_000, 21_000), (40_000, 41_000)]);
        assert_eq!(first_heard(&mut smp, 1, 60), usize::MAX, "off: no zone on that key");
        smp.motion = Motion::Next;
        let heard: Vec<usize> = (0..4).map(|i| first_heard(&mut smp, 2 + i, 60)).collect();
        assert_eq!(heard, [0, 20_000, 40_000, 0], "in turn, the empty one passed over");
        assert_eq!(smp.voices.last().unwrap().octaves, 0.0, "at its own pitch, two octaves from its key");
        assert_eq!(first_heard(&mut smp, 9, 37), 20_000, "whichever key");
        smp.fade_all();
        assert_eq!(first_heard(&mut smp, 10, 60), 0, "stopped, it starts over");
        // Zones on one key go in their own order; one zone plays every time.
        smp.zones[3].keys = (36, 36);
        smp.fade_all();
        assert_eq!([first_heard(&mut smp, 11, 60), first_heard(&mut smp, 12, 60)], [0, 40_000]);
        let mut one = chopped(&[(300, 900)]);
        one.motion = Motion::Random;
        assert_eq!([first_heard(&mut one, 1, 50), first_heard(&mut one, 2, 51)], [300, 300]);
        let mut none = chopped(&[(5, 5)]);
        none.motion = Motion::Next;
        assert_eq!(first_heard(&mut none, 1, 36), usize::MAX);
        // Legato: a note that glides the sounding voice takes no turn.
        let ctx = Context { tempo: 120.0, beats: 0.0, live: false };
        let mut smp = chopped(&[(0, 90_000), (20_000, 90_000), (40_000, 90_000)]);
        smp.motion = Motion::Next;
        smp.settings.polyphony = 0;
        assert!(smp.settings.legato());
        smp.note_on(1, 60, 100, &ctx);
        smp.note_on(2, 62, 100, &ctx);
        assert_eq!(smp.voices.len(), 1, "it glides");
        smp.voices.clear();
        smp.held.clear();
        smp.note_on(3, 60, 100, &ctx);
        assert_eq!(smp.voices[0].stream.play.0, 20_000, "the second slice, not the third");
    }

    #[test]
    fn random_motion_never_repeats() {
        let mut smp = chopped(&[(0, 1000), (20_000, 21_000), (40_000, 41_000)]);
        smp.motion = Motion::Random;
        let heard: Vec<usize> = (0..40).map(|i| first_heard(&mut smp, i, 60)).collect();
        assert!(heard.windows(2).all(|w| w[0] != w[1]), "{heard:?}");
        for a in [0, 20_000, 40_000] {
            assert!(heard.contains(&a), "{a} in {heard:?}");
        }
        // Kept through an edit: the sampler made again goes on from where it was.
        smp.motion = Motion::Next;
        smp.fade_all();
        first_heard(&mut smp, 50, 60);
        let mut again = chopped(&[(0, 1000), (20_000, 21_000), (40_000, 41_000)]);
        again.motion = Motion::Next;
        again.adopt(smp);
        assert_eq!(first_heard(&mut again, 51, 60), 20_000);
    }

    /// Tail: at the slice's end the note plays back from there, fading to nothing; no
    /// longer than the slice; a reversed slice's tail goes forwards from its start.
    #[test]
    fn a_tail_plays_back_from_the_end_and_fades() {
        let ctx = Context { tempo: 120.0, beats: 0.0, live: false };
        let heard = |tail_ms: f32, reverse: bool, n: usize| {
            let mut smp = chopped(&[(48_000, 49_000)]);
            smp.zones[0].keys = (0, 127);
            smp.zones[0].tail_ms = tail_ms;
            smp.zones[0].reverse = reverse;
            smp.note_on(1, 60, 127, &ctx);
            let mut out = vec![0.0; n * 2];
            smp.render(&mut out);
            out.chunks(2).map(|f| f[0]).collect::<Vec<f32>>()
        };
        let peak = |x: &[f32]| x.iter().fold(0f32, |m, v| m.max(v.abs()));
        let none = heard(0.0, false, 3000);
        assert!(peak(&none[1010..]) < 1e-4, "no tail: silent past the end");
        // 500 frames of tail (at 48 kHz).
        let tail = heard(500.0 / 48.0, false, 3000);
        assert!(tail[1100] > 0.2 && tail[1100] < none[900], "there, and quieter than the slice: {}", tail[1100]);
        assert!(tail[1100] > tail[1300] && tail[1300] > tail[1480], "fading");
        assert!(peak(&tail[1510..]) < 1e-4, "gone after 500 frames");
        assert!((tail[1001] - tail[997]).abs() < 0.01, "no jump at the turn: {} {}", tail[997], tail[1001]);
        let long = heard(1000.0, false, 4000);
        assert!(long[1500] > 0.05 && peak(&long[2010..]) < 1e-4, "at most as long as the slice");
        let back = heard(500.0 / 48.0, true, 3000);
        assert!(back[1100] > 0.2 && peak(&back[1510..]) < 1e-4, "a reversed slice's tail");
        // A loop has none.
        let mut smp = chopped(&[(48_000, 49_000)]);
        smp.settings.env_type = 2;
        smp.zones[0].keys = (0, 127);
        smp.zones[0].tail_ms = 100.0;
        smp.zones[0].looped = Some((48_000, 49_000));
        smp.note_on(1, 60, 127, &ctx);
        assert_eq!(smp.voices[0].stream.tail, 0);
        // Nor has a slice that fades out at that end.
        let mut smp = chopped(&[(48_000, 49_000)]);
        smp.zones[0].keys = (0, 127);
        smp.zones[0].tail_ms = 100.0;
        smp.note_on(1, 60, 127, &ctx);
        assert!(smp.voices[0].stream.tail > 0);
        smp.zones[0].fade_out = 200;
        smp.note_on(2, 60, 127, &ctx);
        assert_eq!(smp.voices[1].stream.tail, 0);
    }

    /// Stem lanes: with them all on a voice plays the sample itself; a muted lane leaves
    /// the others' sum, reached over a few milliseconds on a sounding note, and unmuting
    /// goes back to the sample.
    #[test]
    fn stem_lanes_mute_on_sounding_notes() {
        let flat = |v: f32| Some(Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[v; 2]; 96_000] }));
        let lanes = |levels| Lanes { samples: [flat(0.1), flat(0.2), None, flat(0.4)], levels, only: false };
        let ctx = Context { tempo: 120.0, beats: 0.0, live: false };
        let level = |smp: &mut SamplerRt, n: usize| {
            let mut out = vec![0.0; n * 2];
            smp.render(&mut out);
            out[n * 2 - 2]
        };
        // The sample itself is 0.5 throughout (not the lanes' sum, 0.7).
        let mut smp = synced(None);
        smp.zones[0].tune = 0.0;
        smp.zones[0].sample = Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[0.5; 2]; 96_000] });
        smp.zones[0].lanes = Some(lanes([1.0, 1.0, 0.0, 1.0]));
        smp.note_on(1, 60, 127, &ctx);
        let whole = level(&mut smp, 480);
        assert!(whole > 0.0);
        // The first lane muted, as an edit does it (a new Sampler adopting the voices).
        let mut muted = synced(None);
        muted.zones[0] = smp.zones[0].clone();
        muted.zones[0].lanes = Some(lanes([0.0, 1.0, 0.0, 1.0]));
        muted.adopt(smp);
        let soon = level(&mut muted, 48);
        assert!((soon - whole).abs() < whole * 0.25, "no jump: {soon} from {whole}");
        let after = level(&mut muted, 960);
        assert!((after / whole - 0.6 / 0.5).abs() < 1e-3, "the other lanes' sum: {after} against {whole}");
        // A lane at half: 0.1 + 0.1 + 0.4.
        let mut half = synced(None);
        half.zones[0] = muted.zones[0].clone();
        half.zones[0].lanes = Some(lanes([1.0, 0.5, 0.0, 1.0]));
        half.adopt(muted);
        let after = level(&mut half, 960);
        assert!((after / whole - 0.6 / 0.5).abs() < 1e-3, "{after}");
        // All on again: the sample itself.
        let mut on = synced(None);
        on.zones[0] = half.zones[0].clone();
        on.zones[0].lanes = Some(lanes([1.0, 1.0, 0.0, 1.0]));
        on.adopt(half);
        let after = level(&mut on, 960);
        assert!((after - whole).abs() < 1e-6, "{after} against {whole}");
        assert_eq!(on.voices[0].whole, 1.0);
        // A note begun with a lane muted starts on the sum.
        let mut fresh = synced(None);
        fresh.zones[0] = muted_zone(&on.zones[0], lanes([0.0, 1.0, 0.0, 1.0]));
        fresh.note_on(1, 60, 127, &ctx);
        let first = level(&mut fresh, 480);
        assert!((first / whole - 0.6 / 0.5).abs() < 1e-3, "{first}");
        // A stem taken away: the lanes left play, at unity too (0.1 + 0.2 + 0.4), not
        // the sample, which still has it.
        let mut left = synced(None);
        left.zones[0] = muted_zone(&on.zones[0], Lanes { only: true, ..lanes([1.0, 1.0, 0.0, 1.0]) });
        left.note_on(1, 60, 127, &ctx);
        let first = level(&mut left, 480);
        assert!((first / whole - 0.7 / 0.5).abs() < 1e-3, "{first}");
    }

    fn muted_zone(z: &ZoneRt, lanes: Lanes) -> ZoneRt {
        ZoneRt { lanes: Some(lanes), ..z.clone() }
    }

    /// A Sync note played live starts on its grid: just early, it waits for the line;
    /// just late, it starts at once that far into the sample. The song's own notes
    /// (not live) start where they are.
    #[test]
    fn live_sync_notes_start_on_the_grid() {
        // 120 BPM, a 1/16 grid: 6000 frames a division at 48 kHz.
        let sync = || synced(Some(ZoneSync { tempo: 120.0, grid: 0.25, ..ZoneSync::default() }));
        // 0.05 beats (1200 frames) before beat 1.
        let mut early = sync();
        early.note_on(1, 60, 100, &Context { tempo: 120.0, beats: 0.95, live: true });
        assert_eq!((early.voices[0].wait, early.voices[0].stream.pos), (1200, 5), "waits, from the start (5 frames primed)");
        let mut out = vec![0.0; 2400];
        early.render(&mut out);
        assert!(out.iter().all(|x| *x == 0.0), "silent until the line");
        assert_eq!(early.voices[0].wait, 0);
        // 0.05 beats after it.
        let mut late = sync();
        late.note_on(1, 60, 100, &Context { tempo: 120.0, beats: 1.05, live: true });
        assert_eq!((late.voices[0].wait, late.voices[0].stream.pos), (0, 1205), "starts 1200 frames in");
        assert!(late.voices[0].rise > 0, "faded in");
        // On the line, and from the timeline: as played.
        for ctx in [Context { tempo: 120.0, beats: 1.0, live: true }, Context { tempo: 120.0, beats: 1.05, live: false }] {
            let mut on = sync();
            on.note_on(1, 60, 100, &ctx);
            assert_eq!((on.voices[0].wait, on.voices[0].rise, on.voices[0].stream.pos), (0, 0, 5));
        }
    }

    /// A Sync zone given a render for another tempo: its voice carries on from the
    /// same place in the music.
    #[test]
    fn a_new_render_takes_over_where_the_music_is() {
        let ctx = Context { tempo: 120.0, beats: 0.0, live: false };
        let mut old = synced(Some(ZoneSync { tempo: 100.0, ..ZoneSync::default() }));
        old.set_song(0.0, 120.0);
        old.note_on(1, 60, 100, &ctx);
        read(&mut old, 9600);
        let at = old.voices[0].stream.pos as f64 / 96_000.0;
        // The render at 120: 100/120 as long.
        let mut new = synced(Some(ZoneSync { tempo: 120.0, ..ZoneSync::default() }));
        let len = 80_000;
        new.zones[0].sample = Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[0.1; 2]; len] });
        new.zones[0].play = (0, len);
        new.adopt(old);
        assert_eq!(new.voices.len(), 1, "it plays on");
        let v = &new.voices[0];
        assert!(v.fade.is_none(), "not faded out");
        assert!(((v.stream.pos as f64 / len as f64) - at).abs() < 1e-3, "same place: {} vs {at}", v.stream.pos as f64 / len as f64);
    }

    /// A zone's loop and play range moved under a held note: the note goes on, in the new
    /// ones (a loop end pulled back past where it is brings it round at once).
    #[test]
    fn a_held_note_follows_its_loop_and_end() {
        let ctx = Context { tempo: 120.0, beats: 0.0, live: false };
        let adsr = || {
            let mut s = synced(None);
            s.settings.env_type = 2;
            s
        };
        let mut old = adsr();
        old.zones[0].looped = Some((1000, 40_000));
        old.note_on(1, 60, 100, &ctx);
        assert!(old.voices[0].stream.looped.is_some(), "it loops");
        read(&mut old, 12_000);
        assert!(old.voices[0].stream.pos > 10_000);
        let mut new = adsr();
        new.zones[0].sample = old.zones[0].sample.clone();
        (new.zones[0].play, new.zones[0].looped) = ((0, 60_000), Some((2000, 6000)));
        new.adopt(old);
        assert_eq!(new.voices.len(), 1);
        let v = &new.voices[0];
        assert!(v.fade.is_none(), "it plays on");
        assert_eq!((v.stream.play, v.stream.looped), ((0, 60_000), Some((2000, 6000))));
        read(&mut new, 64);
        let pos = new.voices[0].stream.pos;
        assert!((2000..6000).contains(&pos), "round the new loop: {pos}");
        // The loop switched off: it plays out to the new end.
        let mut off = adsr();
        off.zones[0].sample = new.zones[0].sample.clone();
        off.zones[0].play = (0, 30_000);
        off.adopt(new);
        assert_eq!((off.voices[0].stream.play, off.voices[0].stream.looped), ((0, 30_000), None));
    }

    /// A stream over frames whose value is their index, so a mix shows what it read.
    fn stream(play: (i64, i64), looped: (i64, i64), x_fade: i64, forward: bool) -> (super::Stream, Vec<[f32; 2]>) {
        let data = (0..1000).map(|i| [i as f32, i as f32]).collect();
        let pos = if forward { play.0 } else { play.1 - 1 };
        let st = super::Stream {
            pos,
            forward,
            play,
            looped: Some(looped),
            x_fade,
            fade_in: 0,
            fade_out: 0,
            fading: 0,
            done: false,
            tail: 0,
            tail_left: 0,
            tail_len: 0,
        };
        (st, data)
    }

    fn near(got: f32, want: f32, at: i64) {
        assert!((got - want).abs() < 1e-2, "at {at}: {got}, not {want}");
    }

    /// The fade reaches past the Start marker: Start 10 frames before the loop, fade of 50.
    #[test]
    fn a_loop_crossfade_reads_the_audio_before_start() {
        let (mut st, data) = stream((90, 1000), (100, 200), 50, true);
        for pass in 0..3 {
            let first = if pass == 0 { 90 } else { 100 };
            for p in first..200 {
                let x = st.next(&data)[0];
                let left = 200 - p;
                if left <= 50 {
                    let w = left as f32 / 50.0;
                    near(x, p as f32 * w + (100 - left) as f32 * (1.0 - w), p);
                } else {
                    near(x, p as f32, p);
                }
            }
        }
    }

    /// What it may reach: the fade, the loop's length, and the file before the loop.
    #[test]
    fn a_loop_crossfade_is_limited_by_the_loop_and_the_file() {
        let first_faded = |play, looped: (i64, i64), x_fade| {
            let (mut st, data) = stream(play, looped, x_fade, true);
            st.pos = looped.0;
            (looped.0..looped.1).find(|&p| st.next(&data)[0] != p as f32)
        };
        // A fade longer than the loop runs for the loop.
        assert_eq!(first_faded((0, 1000), (500, 600), 400), Some(501), "loop length (500's weight is 1)");
        // Audio before the loop limits it, wherever Start is.
        assert_eq!(first_faded((25, 1000), (30, 130), 100), Some(101), "30 frames before the loop");
        // A loop from the file's first frame has nothing to fade from.
        assert_eq!(first_faded((0, 1000), (0, 100), 50), None, "none before frame 0");
        // None asked for.
        assert_eq!(first_faded((0, 1000), (100, 200), 0), None);
    }

    /// Reversed, it fades into what comes after the loop's end, and lands on the jump.
    #[test]
    fn a_reversed_loop_crossfades_past_its_end() {
        let (mut st, data) = stream((0, 1000), (100, 200), 50, false);
        st.pos = 199;
        for p in (100..200).rev() {
            let x = st.next(&data)[0];
            let left = p + 1 - 100;
            if left <= 50 {
                let w = left as f32 / 50.0;
                near(x, p as f32 * w + (200 + left - 2) as f32 * (1.0 - w), p);
            } else {
                near(x, p as f32, p);
            }
        }
        assert_eq!(st.pos, 199, "jumped back");
        // No more than the file holds past the end: its last frame reads inside the data.
        let (mut st, data) = stream((0, 1000), (900, 1000), 50, false);
        for _ in 0..500 {
            st.next(&data);
        }
        assert!(!st.done);
    }

    /// Each side of the fade takes the zone fade by its own place; before Start, none.
    #[test]
    fn zone_fades_apply_to_each_side_of_a_crossfade() {
        let (mut st, data) = stream((95, 1000), (100, 200), 50, true);
        st.fade_in = 20;
        let gain = |q: i64| if q < 95 || q - 95 >= 20 { 1.0 } else { (q - 95) as f32 / 20.0 };
        for p in 95..200 {
            let x = st.next(&data)[0];
            let left = 200 - p;
            let want = if left <= 50 {
                let w = left as f32 / 50.0;
                let q = 100 - left;
                p as f32 * gain(p) * w + q as f32 * gain(q) * (1.0 - w)
            } else {
                p as f32 * gain(p)
            };
            near(x, want, p);
        }
    }

    /// A note begun inside the fade starts on the tail alone and still lands on the jump.
    #[test]
    fn a_note_begun_inside_a_crossfade_starts_on_the_tail() {
        let (mut st, data) = stream((90, 1000), (100, 200), 50, true);
        st.pos = 190;
        for p in 190..200 {
            let x = st.next(&data)[0];
            let left = 200 - p;
            let w = left as f32 / 10.0;
            near(x, p as f32 * w + (100 - left) as f32 * (1.0 - w), p);
        }
        // The next pass fades over its whole length.
        for p in 100..200 {
            let x = st.next(&data)[0];
            let left = 200 - p;
            if left <= 50 {
                let w = left as f32 / 50.0;
                near(x, p as f32 * w + (100 - left) as f32 * (1.0 - w), p);
            }
        }
    }

    #[test]
    fn segments_reach_their_end() {
        for c in [-0.332, 0.0, 0.4427] {
            let mut s = Segment::new(0.2, 0.9, 1000, c);
            let mut y = 0.0;
            while s.left > 0 {
                y = s.next();
            }
            // The last value is one step short of the end (the next stage starts there).
            assert!((y - 0.9).abs() < 5e-3, "curvature {c}: {y}");
        }
        // The attack is slow at first, the decay fast.
        let mut a = Segment::new(0.0, 1.0, 1000, C_ATTACK);
        let mid = (0..500).map(|_| a.next()).last().unwrap();
        assert!((0.25..0.4).contains(&mid), "attack at half time: {mid}");
        let mut d = Segment::new(1.0, 0.0, 1000, C_DECAY);
        let mid = (0..500).map(|_| d.next()).last().unwrap();
        assert!((0.1..0.3).contains(&mid), "decay at half time: {mid}");
    }

    #[test]
    fn times_and_displays() {
        let (a, h, d) = envelope_ms(1.0, 1.0, 1.0);
        assert!((7000.0..8500.0).contains(&a) && (7000.0..8500.0).contains(&h), "{a} {h}");
        assert!((11500.0..13500.0).contains(&d), "{d}");
        assert_eq!(envelope_ms(0.0, 0.0, 0.0).0, 0.0);
        let (hz, _) = lfo_rate(0.0);
        assert!((hz - 1.0 / 32.0).abs() < 1e-9);
        assert!((lfo_rate(1.0).0 - 16.0).abs() < 1e-9);
        assert_eq!(lofi_rate_hz(0.0, 44100.0), None, "SR at 0 is off");
        assert!(cutoff_hz(1.0, 1) > 18000.0 && cutoff_hz(0.0, 1) < 200.0);
    }

    #[test]
    fn interpolator_weights_sum_to_one() {
        for i in 0..=16 {
            let w = weights(i as f32 / 16.0);
            let sum: f32 = w.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4, "t = {}: {sum}", i as f32 / 16.0);
        }
        let w = weights(1.0);
        assert!((w[3] - 1.0).abs() < 1e-4 && w[4].abs() < 1e-4, "t = 1 is x[n]: {w:?}");
    }

    #[test]
    fn low_pass_has_unity_gain_at_dc() {
        for q in [0.0, 0.5] {
            let (b0, b1, b2, a1, a2) = two_pole(0.2, q, false);
            let dc = (b0 + b1 + b2) / (1.0 - a1 - a2);
            let want = (1.0 - q as f64) * 0.55 + 0.45;
            assert!((dc - want).abs() < 1e-3, "q {q}: {dc}");
        }
        let (b0, b1, b2, ..) = two_pole(0.2, 0.0, true);
        assert!((b0 + b1 + b2).abs() < 1e-3, "high pass blocks DC");
    }

    #[test]
    fn swapping_a_zone_sample_fades_out_voices_instead_of_cutting_off() {
        let sample1 = Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[0.5, 0.5]; 48_000] });
        let sample2 = Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[0.2, 0.2]; 48_000] });
        let sample3 = Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[0.1, 0.1]; 24_000] });
        let make_zone = |sample: Arc<crate::sample::Sample>| {
            let n = sample.frames.len();
            ZoneRt {
                sample,
                root: 60,
                keys: (0, 127),
                vels: (1, 127),
                gain: 1.0,
                pan: 0.0,
                tune: 0.0,
                play: (0, n),
                looped: None,
                x_fade: 0,
                fade_in: 0,
                fade_out: 0,
                sync: None,
                lanes: None,
                reverse: false,
                once: false,
                tail_ms: 0.0,
                layer: None,
            }
        };
        let settings = Settings::new(&deckard_model::Sampler::default(), 48_000.0);
        let mut smp1 = SamplerRt::new(settings.clone(), vec![make_zone(sample1)], (1.0, 1.0), 48_000.0);
        let ctx = Context { tempo: 120.0, beats: 0.0, live: false };
        smp1.note_on(1, 60, 100, &ctx);
        assert_eq!(smp1.voices.len(), 1);
        assert!(smp1.voices[0].fade.is_none());

        // Swapping to identical dimensions (sample2): voice continues seamlessly without interruption.
        let mut smp2 = SamplerRt::new(settings.clone(), vec![make_zone(sample2)], (1.0, 1.0), 48_000.0);
        smp2.adopt(smp1);
        assert_eq!(smp2.voices.len(), 1);
        assert!(smp2.voices[0].fade.is_none(), "matching dimension sample continues seamlessly");

        // Swapping to different sample bounds (sample3): voice is retained and fades out, not abruptly dropped.
        let mut smp3 = SamplerRt::new(settings, vec![make_zone(sample3)], (1.0, 1.0), 48_000.0);
        smp3.adopt(smp2);
        assert_eq!(smp3.voices.len(), 1, "voice is retained across the sample swap");
        assert!(smp3.voices[0].fade.is_some(), "voice is fading out to prevent digital click");
    }

    #[test]
    fn legato_out_of_zone_note_does_not_glide_or_fade_voices() {
        let sample = Arc::new(crate::sample::Sample { rate: 48_000, frames: vec![[0.5, 0.5]; 48_000] });
        let zone = ZoneRt {
            sample,
            root: 60,
            keys: (60, 72),
            vels: (1, 127),
            gain: 1.0,
            pan: 0.0,
            tune: 0.0,
            play: (0, 48_000),
            looped: None,
            x_fade: 0,
            fade_in: 0,
            fade_out: 0,
            sync: None,
            lanes: None,
            reverse: false,
            once: false,
            tail_ms: 0.0,
            layer: None,
        };
        let mut settings = Settings::new(&deckard_model::Sampler::default(), 48_000.0);
        settings.polyphony = 0; // Legato
        settings.glide = 0.5;
        let mut smp = SamplerRt::new(settings, vec![zone], (1.0, 1.0), 48_000.0);
        let ctx = Context { tempo: 120.0, beats: 0.0, live: false };
        smp.note_on(1, 60, 100, &ctx);
        assert_eq!(smp.voices.len(), 1);
        assert_eq!(smp.voices[0].note, 60);
        assert!(smp.voices[0].fade.is_none());
        assert_eq!(smp.glide_target, 0.0);

        // Out-of-zone note (pitch 48 is below keys 60..=72): must not glide or fade the voice.
        smp.note_on(2, 48, 100, &ctx);
        assert_eq!(smp.voices.len(), 1);
        assert_eq!(smp.voices[0].note, 60);
        assert!(smp.voices[0].fade.is_none());
        assert_eq!(smp.glide_target, 0.0);
        assert_eq!(smp.glide_step, 0.0);
    }
}

#[cfg(test)]
#[path = "sampler_layers_tests.rs"]
mod layers_tests;
