//! Maschine's Grain Stretch (docs/fx-grain-stretch.md): `RT::GrainStretchProcessor` →
//! `FXCORE::GrainStretchCore`, a compiled Reaktor-Core kernel. Two overlapping grains, each
//! with a raised-sine window, read a 2^19-sample stereo buffer with Catmull-Rom
//! interpolation at the Pitch ratio. A free-running phase starts a grain every half grain
//! length. With On, the grains read from where On was switched on plus a position that
//! loops over Loop sixteenths at `1 − Stretch` speed (Stretch 100 % holds one spot). The wet
//! signal only sounds while On and the transport plays. Nothing is random. All arithmetic
//! follows the assembly operation by operation (`f32`, no fused multiply-adds), so the
//! output can match bit for bit.

use super::{Effect, param};

/// Buffer length (indices are masked with 0x7ffff).
const LEN: usize = 1 << 19;
const MASK: i32 = 0x7ffff;
/// The mix smoother's offset that keeps it off denormals (`0x219392ef`).
const TINY: f32 = f32::from_bits(0x2193_92ef);
/// `ln 2` (`0x3f317215`): the mix smoother halves the distance in 5 ms.
const LN2: f32 = f32::from_bits(0x3f31_7215);
/// `1/60` (`0x3c88889a`): BPM to beats per second.
const PER_MIN: f32 = f32::from_bits(0x3c88_889a);
/// Window polynomial: `0.5 + K1·u − K2·u³ + K3·u⁵ ≈ 0.5 + 0.5·sin(2πu)`.
const WIN_K1: f32 = f32::from_bits(0x4048_27fa);
const WIN_K2: f32 = f32::from_bits(0x41a4_9eb8);
const WIN_K3: f32 = f32::from_bits(0x4222_79c1);
/// Max grain length in ms: 330, or 140 with Link (`330 + Link·(−190)`).
const MAX_MS: f32 = 330.0;
const LINK_MS: f32 = f32::from_bits(0xc33e_0000); // −190

/// Semitones above the centre (0x10dc3300c), by `|2·Pitch − 1|·24`.
const UP: [f32; 25] =
    [0.0, 0.0, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5, 8.0, 8.5, 9.0, 9.5, 10.0, 10.5, 11.0, 11.5, 12.0];
/// Semitones below the centre (0x10dc33070). There is no −6.
const DOWN: [f32; 25] = [
    0.0, 0.0, -1.0, -2.0, -3.0, -4.0, -5.0, -7.0, -8.0, -9.0, -10.0, -11.0, -12.0, -16.0, -20.0, -24.0, -28.0, -32.0, -36.0, -40.0, -44.0,
    -48.0, -52.0, -56.0, -60.0,
];

/// The kernel's rounding: `±0.5` added in `f64` by the sign of `x`, then truncated
/// (`fcvtzs`: saturating, NaN → 0).
fn rnd(x: f32) -> i32 {
    ((x as f64) + if x < 0.0 { -0.5 } else { 0.5 }) as i32
}

/// [`rnd`] of the `f32` of `a + b` (an `adds`): the sign test comes from the add's flags,
/// so it is the true sum's sign even when the sum wraps.
fn rnd_sum(a: i32, b: i32) -> i32 {
    let s = a.wrapping_add(b);
    ((s as f32) as f64 + if (a as i64 + b as i64) < 0 { -0.5 } else { 0.5 }) as i32
}

/// `2^n` for an integer `n`, by adding `n << 23` to the bits of 1.0.
fn pow2i(n: i32) -> f32 {
    f32::from_bits(0x3f80_0000i32.wrapping_add(n.wrapping_shl(23)) as u32)
}

/// The Size factor `≈ 2^(6·(1 − (s + (s² − s)·2/3)) − 4)`: 4 at Size 0 down to 1/16.
/// `s2` is `s² − s`.
fn size_factor(s: f32, s2: f32) -> f32 {
    let x = (1.0 - (s2 * f32::from_bits(0x3f2a_aab0) + s)) * 6.0 + -4.0;
    let n = rnd(x);
    let f = x - n as f32;
    (f * (f * f32::from_bits(0x3e7d_f3b6) + f32::from_bits(0x3f35_3f7d)) + 1.0) * pow2i(n)
}

/// Pitch (0..1) to the read-speed ratio: semitones from [`UP`]/[`DOWN`], rounded to
/// quarter tones, then `≈ 2^(st/12)`.
fn pitch_ratio(p: f32) -> f32 {
    let b = (p + p) + -1.0;
    let x = b.abs() * 24.0;
    let i = rnd(x + -0.5);
    let fi = i as f32;
    let i0 = if (i as u32) < 24 { i as usize } else { 24 };
    let j = rnd(fi + 1.0);
    let i1 = if (j as u32) < 24 { j as usize } else { 24 };
    let t = if b > 0.0 { &UP } else { &DOWN };
    let st = t[i0] + (x - fi) * (t[i1] - t[i0]);
    let q = rnd(st * 4.0) as f32 * 0.25;
    let oct = q * f32::from_bits(0x3daa_aab4); // 1/12
    let n = rnd(oct);
    let f = oct - n as f32;
    let poly = f * (f * (f * f32::from_bits(0x3d65_6042) + f32::from_bits(0x3e77_ced9)) + f32::from_bits(0x3f31_6873)) + 1.0;
    poly * pow2i(n)
}

/// The grain window at `x` (phase / length, 0..1; past 1 it is ≈ 0):
/// `≈ 0.5 + 0.5·sin(2π(¼ − |x − ½|))`.
fn window(x: f32) -> f32 {
    let t = if x > 1.0 { 0.5 } else { x + -0.5 };
    let u = 0.25 - t.abs();
    let u2 = u * u;
    let u3 = u * u2;
    let u5 = u2 * u3;
    u * WIN_K1 + (u5 * WIN_K3 - u3 * WIN_K2) + 0.5
}

/// Catmull-Rom read between `buf[i]` and `buf[i + 1]` at fraction `f`.
fn tap(buf: &[f32], i: i32, f: f32) -> f32 {
    let at = |k: i32| buf[(k & MASK) as usize];
    let (y0, y1, ym, y2) = (at(i), at(i.wrapping_add(1)), at(i.wrapping_add(MASK)), at(i.wrapping_add(2)));
    let c1 = (y1 - ym) * 0.5;
    let c2 = (y2 - y0) * 0.5;
    let d = y0 - y1;
    let s = d + c1;
    let q = d + (c2 + s);
    y0 + f * (c1 + f * (f * q - (s + q)))
}

/// The kernel's state (`GrainStretchCore` offsets). Fields it writes but never reads are
/// left out.
#[derive(Default)]
struct Core {
    sr: f32, // +0x00
    // Inputs.
    in_mix: f32,     // +0x10
    playing: f32,    // +0x18 (1 while the transport plays)
    in_on: f32,      // +0x1c
    in_link: f32,    // +0x24
    in_pitch: f32,   // +0x2c
    in_size: f32,    // +0x34
    in_loop: f32,    // +0x3c (option index)
    tempo: f32,      // +0x44 (BPM)
    in_stretch: f32, // +0x54
    // Mix: smoothed, value, last playing, playing (0/1), mix·playing, On, target, coefficient.
    mix_s: f32,      // +0x5c
    mix: f32,        // +0x60
    last_play: f32,  // +0x64
    play: f32,       // +0x68
    mix_p: f32,      // +0x6c
    on: i32,         // +0x70
    on_f: f32,       // +0x74
    target: f32,     // +0x78
    mix_c: f32,      // +0x7c
    per_ms: f32,     // +0x80 sr/1000
    link: f32,       // +0x84
    max_ms: f32,     // +0x88
    ratio: f32,      // +0x8c
    link_d: f32,     // +0x90 80/ratio − 80
    base_ms: f32,    // +0x94 80, or 80/ratio with Link
    size_f: f32,     // +0x98
    len: f32,        // +0x9c grain length (samples)
    inv_len: f32,    // +0xa0
    phase: f32,      // +0xa4 = +0xa8, −0.5..0.5
    w: i32,          // +0xac write counter
    pos: f32,        // +0xb0 loop position
    loop_len: f32,   // +0xb4 sr · sixteenths/4
    last_tempo: f32, // +0xb8
    bps: f32,        // +0xbc beats per second
    loop_n: f32,     // +0xc0 loop length (samples)
    w_cur: i32,      // +0xcc write index of the last sample
    start: f32,      // +0xd0 write index when On was switched on
    lead: f32,       // +0xdc max(ratio − 1, 0)
    lead_n: i32,     // +0xe0 rnd(len·lead): how far behind the read starts
    a_pos: f32,      // +0xe4 grain A read position
    arm: i32,        // +0xe8 On was switched on: count to the hold
    frozen: i32,     // +0xec
    count: i32,      // +0xf0
    sr8: f32,        // +0xf8
    ms100: i32,      // +0xfc
    hold_at: i32,    // +0x100 8 beats + 100 ms − 1
    hold: i32,       // +0x108 stop writing the buffer
    a_ph: f32,       // +0x10c
    a_inv: f32,      // +0x110
    b_pos: f32,      // +0x114
    b_ph: f32,       // +0x118
    b_inv: f32,      // +0x11c
    speed: f32,      // +0x120 min(1 − Stretch, 1)
    a_inc: f32,      // +0x124
    prev_len: f32,   // +0x128
    longer: i32,     // +0x12c len ≥ previous len
    ph_inc: f32,     // +0x130 max(ratio − 1, 1)
    a_k: f32,        // +0x134
    a_len: f32,      // +0x138
    a_dph: f32,      // +0x13c
    b_inc: f32,      // +0x144
    b_k: f32,        // +0x148
    b_len: f32,      // +0x14c
    b_dph: f32,      // +0x150
    k_80r: f32,      // +0x400158 80/ratio
    k_size: f32,     // +0x40015c
    k_size2: f32,    // +0x400160 size² − size
    k_loop: f32,     // +0x400164
    buf_l: Vec<f32>, // +0x158
    buf_r: Vec<f32>, // +0x200158
}

impl Core {
    fn new(sr: f32) -> Self {
        // The processor's presets (Pitch 0.5, Mix 0.5) until the parameters arrive.
        Core { sr, in_pitch: 0.5, in_mix: 0.5, buf_l: vec![0.0; LEN], buf_r: vec![0.0; LEN], ..Default::default() }
    }

    /// `GrainStretchProcessor::resume` (0x101a3d3a8): zero the kernel (buffers included),
    /// keep the inputs, then `endInit` with every input flagged.
    fn resume(&mut self) {
        let mut z = Core {
            sr: self.sr,
            in_mix: self.in_mix,
            in_on: self.in_on,
            in_link: self.in_link,
            in_pitch: self.in_pitch,
            in_size: self.in_size,
            in_loop: self.in_loop,
            in_stretch: self.in_stretch,
            buf_l: std::mem::take(&mut self.buf_l),
            buf_r: std::mem::take(&mut self.buf_r),
            ..Default::default()
        };
        z.buf_l.fill(0.0);
        z.buf_r.fill(0.0);
        *self = z;
        self.end_init();
    }

    /// `GrainStretchCore::endInit` (0x101a3d45c) on a zeroed kernel. Tempo and playing are
    /// still 0 here, so the loop and hold lengths are garbage until the first sample
    /// recomputes them. The kernel then runs one silent sample; on a zeroed kernel that
    /// only writes zeros, so it is left out.
    fn end_init(&mut self) {
        let sr = self.sr;
        self.k_size = self.in_size;
        self.k_size2 = self.in_size * self.in_size - self.in_size;
        self.link = rnd(self.in_link) as f32;
        self.k_loop = rnd(rnd(self.in_loop) as f32) as f32;
        let ms5 = sr * f32::from_bits(0x3ba3_d70a); // 0.005
        self.sr8 = sr * 8.0;
        self.ms100 = rnd(sr / 1000.0 * 100.0);
        self.per_ms = sr * f32::from_bits(0x3a83_126f); // 0.001
        self.phase = -0.5;
        self.size_f = size_factor(self.k_size, self.k_size2);
        self.max_ms = self.link * LINK_MS + MAX_MS;
        self.loop_len = sr * (self.k_loop * 0.25);
        if self.tempo != self.last_tempo {
            self.last_tempo = self.tempo;
            self.bps = self.tempo * PER_MIN;
        }
        self.mix_c = if ms5 < 1.0 { LN2 } else { LN2 / ms5 };
        self.w = 1;
        self.hold_at = rnd_sum(rnd(self.sr8 / self.bps), self.ms100).wrapping_sub(1);
        self.loop_n = rnd(self.loop_len / self.bps) as f32;
        self.ratio = pitch_ratio(self.in_pitch);
        self.k_80r = 80.0 / self.ratio;
        let d = self.ratio + -1.0;
        self.ph_inc = if d < 1.0 { 1.0 } else { d };
        self.lead = if d < 0.0 { 0.0 } else { d };
        self.link_d = self.k_80r + -80.0;
        self.base_ms = self.link * self.link_d + 80.0;
        let ms = self.size_f * self.base_ms;
        let ms = if self.max_ms > ms { ms } else { self.max_ms };
        self.on = rnd(self.in_on);
        self.on_f = self.on as f32;
        if self.on >= 1 {
            self.start = 0.0;
            self.pos = 0.0;
        }
        if self.playing != self.last_play {
            self.last_play = self.playing;
            self.play = rnd(self.playing) as f32;
        }
        self.mix = self.in_mix;
        self.mix_p = self.mix * self.play;
        self.target = self.mix_p * self.on_f;
        self.len = self.per_ms * ms;
        self.prev_len = self.len;
        self.lead_n = rnd(self.len * self.lead);
        self.inv_len = 1.0 / self.len;
        let m = self.mix_s + (self.target - self.mix_s) * self.mix_c;
        self.mix_s = m + TINY;
        self.arm = 0;
        self.longer = 1; // len ≥ 0
        let s = 1.0 - self.in_stretch;
        self.speed = if s > 1.0 { 1.0 } else { s };
        self.pos = self.speed + 0.0;
        self.hold = 0;
        self.frozen = 0;
    }

    /// The tail of the Pitch, Size and Link events: the new grain length; when it got
    /// shorter, the running grains' phase rates are rescaled to end in time.
    fn new_len(&mut self) {
        let ms = self.size_f * self.base_ms;
        let ms = if self.max_ms > ms { ms } else { self.max_ms };
        let len = self.per_ms * ms;
        let prev = self.prev_len;
        self.prev_len = len;
        self.len = len;
        self.inv_len = 1.0 / len;
        self.lead_n = rnd(len * self.lead);
        self.longer = ((if len >= prev { 1.0f64 } else { 0.0 }) + 0.5) as i32;
        if self.longer <= 0 {
            let a = self.a_k * (self.a_len / len);
            self.b_dph = self.b_k * (self.b_len / len);
            self.a_dph = a;
        }
    }

    /// `processEventInTrans` (0x101a3e624): Pitch.
    fn ev_pitch(&mut self, v: f32) {
        self.in_pitch = v;
        self.ratio = pitch_ratio(v);
        let d = self.ratio + -1.0;
        self.link_d = 80.0 / self.ratio + -80.0;
        self.base_ms = self.link * self.link_d + 80.0;
        self.ph_inc = if d < 1.0 { 1.0 } else { d };
        self.lead = if d < 0.0 { 0.0 } else { d };
        self.new_len();
    }

    /// `GrainStretchProcessor::setSize` (0x101a3e84c).
    fn ev_size(&mut self, v: f32) {
        self.in_size = v;
        self.size_f = size_factor(v, v * v - v);
        self.new_len();
    }

    /// `processBoolParameter(1)`: Link.
    fn ev_link(&mut self, v: bool) {
        self.in_link = v as u8 as f32;
        let l = ((v as u8 as f64) + 0.5) as i32 as f32;
        self.link = l;
        self.max_ms = l * LINK_MS + MAX_MS;
        self.base_ms = self.link_d * l + 80.0;
        self.new_len();
    }

    /// `processBoolParameter(0)`: On. Switching on marks the loop start at the write
    /// position and arms the hold.
    fn ev_on(&mut self, v: bool) {
        self.in_on = v as u8 as f32;
        let o = ((v as u8 as f64) + 0.5) as i32;
        self.on = o;
        self.on_f = o as f32;
        self.target = self.mix_p * self.on_f;
        self.arm = o;
        if o >= 1 {
            self.start = self.w_cur as f32;
            self.pos = 0.0;
            self.count = 0;
        } else {
            self.hold = 0;
            self.frozen = 0;
        }
    }

    /// `processFloatParameter(0)`: Stretch.
    fn ev_stretch(&mut self, v: f32) {
        self.in_stretch = v;
        let s = 1.0 - v;
        self.speed = if s > 1.0 { 1.0 } else { s };
    }

    /// `processFloatParameter(3)`: Mix.
    fn ev_mix(&mut self, v: f32) {
        self.in_mix = v;
        self.mix = v;
        self.mix_p = self.play * v;
        self.target = self.mix_p * self.on_f;
    }

    /// `processEnumParameter(0)`: Loop (the option index).
    fn ev_loop(&mut self, idx: u64) {
        let f = idx as f32;
        self.in_loop = f;
        let i = (f as f64 + 0.5) as i32;
        self.loop_len = rnd(i as f32) as f32 * 0.25 * self.sr;
        self.loop_n = rnd(self.loop_len / self.bps) as f32;
    }

    /// `GrainStretchCore::processAudio` (0x101a3dff4): one sample.
    fn tick(&mut self, xl: f32, xr: f32) -> (f32, f32) {
        if self.tempo != self.last_tempo {
            self.last_tempo = self.tempo;
            self.bps = self.tempo * PER_MIN;
            let t = rnd(self.sr8 / self.bps);
            self.hold_at = rnd_sum(self.ms100, t).wrapping_sub(1);
            self.loop_n = rnd(self.loop_len / self.bps) as f32;
        }
        if self.playing != self.last_play {
            self.last_play = self.playing;
            self.play = rnd(self.playing) as f32;
            self.mix_p = self.mix * self.play;
            self.target = self.mix_p * self.on_f;
        }
        let w = self.w & MASK;
        let fw = w as f32;
        self.w = rnd(fw + 1.0);
        self.w_cur = w;
        let frozen = self.frozen;
        let m = self.mix_s + (self.target - self.mix_s) * self.mix_c;
        self.mix_s = m + TINY;
        // The grain clock: grain A starts when it crosses 0 upwards, B when it wraps.
        let old = self.phase;
        let p = old + self.inv_len;
        let ph = if p > 0.5 { p + -1.0 } else { p };
        self.phase = ph;
        // The loop position (with On) or the write index gives the read base.
        let mut pos = self.pos;
        if pos >= self.loop_n {
            pos -= self.loop_n;
        }
        self.pos = pos + self.speed;
        let n = rnd(pos);
        let base = if self.on > 0 { self.start + n as f32 } else { fw };
        let from = rnd(base).wrapping_sub(self.lead_n) as f32;
        if ph > 0.0 || ph.is_nan() {
            if old <= 0.0 || old.is_nan() {
                let (l, k) = (self.len, self.len);
                self.a_inv = 1.0 / l;
                self.a_k = self.ph_inc;
                self.a_len = l;
                self.a_ph = 0.0;
                self.a_inc = self.ratio;
                self.a_pos = from;
                self.a_dph = if self.longer > 0 { self.ph_inc } else { (l / k) * self.ph_inc };
            }
        } else if old > 0.0 {
            let (l, k) = (self.len, self.len);
            self.b_inv = 1.0 / l;
            self.b_k = self.ph_inc;
            self.b_len = l;
            self.b_ph = 0.0;
            self.b_inc = self.ratio;
            self.b_pos = from;
            self.b_dph = if self.longer > 0 { self.ph_inc } else { (l / k) * self.ph_inc };
        }
        let b_ph = self.b_ph;
        self.b_ph = b_ph + self.b_dph;
        let a_ph = self.a_ph;
        self.a_ph = a_ph + self.a_dph;
        let b_pos = self.b_pos;
        self.b_pos = b_pos + self.b_inc;
        let a_pos = self.a_pos;
        self.a_pos = a_pos + self.a_inc;
        // After On, the buffer stops recording 8 beats + 100 ms later.
        if frozen <= 0 && self.arm >= 1 {
            let c = self.count;
            self.count = c + 1;
            if c >= self.hold_at {
                self.hold = 1;
                self.frozen = 1;
            }
        }
        if self.hold <= 0 {
            self.buf_r[w as usize] = xr;
            self.buf_l[w as usize] = xl;
        }
        let wa = window(a_ph * self.a_inv);
        let wb = window(b_ph * self.b_inv);
        let ia = rnd(a_pos + -0.5);
        let ib = rnd(b_pos + -0.5);
        let fa = a_pos - ia as f32;
        let fb = b_pos - ib as f32;
        let wet = m * (2.0 - m);
        let dry = 1.0 - m * m;
        let r = xr * dry + wet * (wa * tap(&self.buf_r, ia, fa) + wb * tap(&self.buf_r, ib, fb));
        let l = xl * dry + wet * (wa * tap(&self.buf_l, ia, fa) + wb * tap(&self.buf_l, ib, fb));
        (l, r)
    }
}

/// Field names and defaults (the module's constructor, 0x101a3c170).
const DEFAULTS: [(&str, f64); 7] =
    [("on", 0.0), ("stretch", 0.0), ("loop_length", 8.0), ("pitch", 0.5), ("link", 0.0), ("size", 0.25), ("mix", 1.0)];

/// Maschine's Grain Stretch.
pub struct GrainStretch {
    core: Box<Core>,
    /// The stored values last sent, in [`DEFAULTS`] order (`None` before the first `set`).
    values: Option<[f64; 7]>,
    bpm: f32,
    playing: bool,
    /// The first callbacks, held back until this many samples have run (see
    /// [`GrainStretch::defer_first_events`]).
    pending: Option<(usize, [f64; 7])>,
    defer: usize,
}

impl GrainStretch {
    pub fn new(sr: f64) -> Self {
        GrainStretch { core: Box::new(Core::new(sr as f32)), values: None, bpm: 120.0, playing: false, pending: None, defer: 0 }
    }

    /// Hold back the callbacks that follow the first resume until `samples` have been
    /// processed (call before the first `set`). Maschine sends them from its main thread:
    /// in `maschine-ref` renders they land 128 samples into the audio, which moves the
    /// loop start (docs/fx-grain-stretch.md §2). Null tests use this; DECKARD sends them
    /// at once.
    pub fn defer_first_events(&mut self, samples: usize) {
        self.defer = samples;
    }

    /// The parameters' callbacks, in `connectParameters` order, for the values that
    /// changed (all of them when `old` is `None`).
    fn events(&mut self, old: Option<[f64; 7]>, new: [f64; 7]) {
        let c = &mut self.core;
        let ch = |k: usize| old.is_none_or(|o| o[k] != new[k]);
        if ch(0) {
            c.ev_on(new[0] != 0.0);
        }
        if ch(1) {
            c.ev_stretch(new[1] as f32);
        }
        if ch(2) {
            c.ev_loop(new[2] as u64);
        }
        if ch(3) {
            c.ev_pitch(new[3] as f32);
        }
        if ch(4) {
            c.ev_link(new[4] != 0.0);
        }
        if ch(5) {
            c.ev_size(new[5] as f32);
        }
        if ch(6) {
            c.ev_mix(new[6] as f32);
        }
    }
}

impl Effect for GrainStretch {
    fn set(&mut self, params: &[(String, f64)]) {
        let new = DEFAULTS.map(|(k, d)| param(params, k, d));
        match (self.values, self.pending) {
            (None, _) => {
                // The processor resumes with the stored values, then gets every callback
                // once (On then marks the loop start and arms the hold).
                let c = &mut self.core;
                c.in_on = (new[0] != 0.0) as u8 as f32;
                c.in_stretch = new[1] as f32;
                c.in_loop = new[2] as u64 as f32;
                c.in_pitch = new[3] as f32;
                c.in_link = (new[4] != 0.0) as u8 as f32;
                c.in_size = new[5] as f32;
                c.in_mix = new[6] as f32;
                c.resume();
                if self.defer > 0 {
                    self.pending = Some((self.defer, new));
                } else {
                    self.events(None, new);
                }
            }
            // Still held back: the first callbacks will carry the new values.
            (Some(_), Some((k, _))) => self.pending = Some((k, new)),
            (Some(old), None) => self.events(Some(old), new),
        }
        self.values = Some(new);
    }

    fn set_tempo(&mut self, bpm: f64) {
        self.bpm = bpm as f32;
    }

    fn set_transport(&mut self, quarters: Option<f64>) {
        self.playing = quarters.is_some();
    }

    /// `GrainStretchProcessor::process` (0x101a3ded4): tempo and "playing" for the block,
    /// then the core per sample.
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.core.tempo = self.bpm;
        self.core.playing = if self.playing { 1.0 } else { 0.0 };
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            if let Some((k, v)) = self.pending {
                if k == 0 {
                    self.pending = None;
                    self.events(None, v);
                } else {
                    self.pending = Some((k - 1, v));
                }
            }
            (*a, *b) = self.core.tick(*a, *b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(kv: &[(&str, f64)]) -> Vec<(String, f64)> {
        kv.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn noise(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 7919 + 13) % 1000) as f32 / 1000.0 - 0.5).collect()
    }

    fn run(fx: &mut GrainStretch, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut a, mut b) = (l.to_vec(), r.to_vec());
        for (x, y) in a.chunks_mut(512).zip(b.chunks_mut(512)) {
            fx.process(x, y);
        }
        (a, b)
    }

    fn playing(kv: &[(&str, f64)], sr: f64) -> GrainStretch {
        let mut fx = GrainStretch::new(sr);
        fx.set(&p(kv));
        fx.set_tempo(120.0);
        fx.set_transport(Some(0.0));
        fx
    }

    #[test]
    fn pitch_table_gives_ratios() {
        assert_eq!(pitch_ratio(0.5), 1.0);
        assert!((pitch_ratio(1.0) - 2.0).abs() < 1e-5); // +12 semitones
        assert!((pitch_ratio(0.0) - 2f32.powi(-5)).abs() < 1e-6); // −60 semitones
    }

    #[test]
    fn size_factor_spans_4_to_one_sixteenth() {
        assert_eq!(size_factor(0.0, 0.0), 4.0);
        assert_eq!(size_factor(1.0, 0.0), 0.0625);
    }

    #[test]
    fn defaults_and_stopped_transport_pass_through() {
        let x = noise(30_000);
        // Defaults: On is off.
        // The wet gain decays to about 2e-18 (the smoother's offset), so "equal" is close.
        let near = |(l, r): (Vec<f32>, Vec<f32>)| l.iter().chain(&r).zip(x.iter().chain(&x)).all(|(a, b)| (a - b).abs() < 1e-12);
        let mut fx = playing(&[], 48_000.0);
        assert!(near(run(&mut fx, &x, &x)));
        // On, but the transport is stopped.
        let mut fx = GrainStretch::new(44_100.0);
        fx.set(&p(&[("on", 1.0)]));
        assert!(near(run(&mut fx, &x, &x)));
    }

    #[test]
    fn on_and_playing_stretches() {
        let x = noise(96_000);
        let mut fx = playing(&[("on", 1.0), ("stretch", 0.6), ("pitch", 0.8), ("size", 0.7)], 48_000.0);
        let (l, r) = run(&mut fx, &x, &x);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 4.0));
        assert!(l.iter().zip(&x).map(|(a, b)| (a - b).abs()).sum::<f32>() > 100.0);
    }

    #[test]
    fn channels_are_independent_and_silence_stays_silent() {
        let x = noise(60_000);
        let z = vec![0.0; x.len()];
        let mut fx = playing(&[("on", 1.0), ("pitch", 0.3), ("link", 1.0)], 44_100.0);
        let (l, r) = run(&mut fx, &x, &z);
        assert!(l.iter().any(|v| *v != 0.0));
        assert!(r.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn set_keeps_state() {
        let x = noise(60_000);
        let kv = [("on", 1.0), ("stretch", 0.4)];
        let mut a = playing(&kv, 48_000.0);
        let (l1, _) = run(&mut a, &x[..20_000], &x[..20_000]);
        a.set(&p(&kv)); // unchanged values send nothing: On must not restart the loop
        let (l2, _) = run(&mut a, &x[20_000..], &x[20_000..]);
        let mut b = playing(&kv, 48_000.0);
        let (lb, _) = run(&mut b, &x, &x);
        assert_eq!([l1, l2].concat(), lb);
    }

    #[test]
    fn deferred_first_events_differ_only_in_loop_start() {
        let x = noise(20_000);
        let mut a = GrainStretch::new(48_000.0);
        a.defer_first_events(128);
        a.set(&p(&[("on", 1.0), ("size", 1.0)]));
        a.set_transport(Some(0.0));
        let (l, _) = run(&mut a, &x, &x);
        assert!(l.iter().all(|v| v.is_finite()));
    }
}
