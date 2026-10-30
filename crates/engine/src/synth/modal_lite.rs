//! DECKARD's own drum voice for the engines it doesn't port from Maschine (the Kick's
//! Rasper, Snappy, Bold, Maple and Push, the Snare's modal engines, Tom Floor and High,
//! Kettle, the Cymbal and the Hi-hat's Hybrid). Each engine describes a strike as a
//! [`Patch`] from its parameters, the velocity and the note; a [`Voice`] plays it:
//!
//! - a body oscillator: a sine (optionally driven) whose pitch falls from `f_start` to
//!   `f_end`, with a held then decaying level: the kick's main tone;
//! - a bank of up to [`MODES`] decaying modes (frequency, level, decay, pan), all bent
//!   together by a pitch envelope: drum skins, shells, metal;
//! - a click: a short burst of filtered noise at the strike;
//! - a noise layer: white, pink or one of the modal noises (`synth::noise`) through a
//!   band-pass, under its own attack/decay: snare wires, air, a cymbal's wash.
//!
//! The designs are tuned by ear and by measurement against Maschine's renders of the
//! same settings (level, decay times, pitch over time, spectrum); they are not ports.

use super::noise;

/// Modes a voice can hold.
pub const MODES: usize = 64;
/// Samples between control updates (pitch envelopes, filter coefficients).
const BLOCK: u32 = 16;
/// A choke's fade (seconds).
const CHOKE: f32 = 0.008;

/// The body oscillator of a strike.
#[derive(Clone, Copy, Default)]
pub struct Osc {
    pub level: f32,
    /// Pitch at the strike and where it settles (Hz), and the fall's time constant (s).
    pub f_start: f32,
    pub f_end: f32,
    pub fall: f32,
    /// Level held this long (s), then decaying with time constant `decay` (s).
    pub hold: f32,
    pub decay: f32,
    /// tanh drive (0: clean).
    pub drive: f32,
}

/// One decaying mode.
#[derive(Clone, Copy, Default)]
pub struct Mode {
    pub freq: f32,
    pub level: f32,
    /// Time constant (s): the level falls by 1/e in this time.
    pub decay: f32,
    /// −1 (left) … 1 (right).
    pub pan: f32,
}

/// Where a noise layer comes from.
#[derive(Clone, Copy, Default, PartialEq)]
pub enum Source {
    #[default]
    White,
    Pink,
    /// One of the 23 modal noises.
    Modal(usize),
}

/// A noise layer (or the click).
#[derive(Clone, Copy, Default)]
pub struct Noise {
    pub level: f32,
    pub source: Source,
    /// Band-pass centre (Hz) and Q; `hp` (Hz, 0: none) cuts below.
    pub freq: f32,
    pub q: f32,
    pub hp: f32,
    /// Attack and decay time constants (s).
    pub attack: f32,
    pub decay: f32,
    /// Stereo width: the right channel reads the source this far away (0: mono).
    pub width: f32,
}

/// A whole strike.
#[derive(Clone, Copy)]
pub struct Patch {
    pub osc: Osc,
    pub modes: [Mode; MODES],
    pub n_modes: usize,
    /// The modes' pitch envelope: they start `bend` octaves up and fall with time
    /// constant `bend_fall` (s).
    pub bend: f32,
    pub bend_fall: f32,
    pub click: Noise,
    pub noise: Noise,
    /// The whole voice's gain.
    pub gain: f32,
    /// Start the modes at scattered phases (metal) rather than together (skins).
    pub scatter: bool,
    /// The modes swell in with this time constant (s; 0: at once), as a struck ride's
    /// wash builds.
    pub swell: f32,
}

impl Default for Patch {
    fn default() -> Patch {
        Patch {
            osc: Osc::default(),
            modes: [Mode::default(); MODES],
            n_modes: 0,
            bend: 0.0,
            bend_fall: 0.05,
            click: Noise::default(),
            noise: Noise::default(),
            gain: 1.0,
            scatter: false,
            swell: 0.0,
        }
    }
}

impl Patch {
    /// Add a mode (ignored past [`MODES`]).
    pub fn mode(&mut self, freq: f32, level: f32, decay: f32, pan: f32) {
        if self.n_modes < MODES && freq > 0.0 && level > 0.0 {
            self.modes[self.n_modes] = Mode { freq, level, decay, pan };
            self.n_modes += 1;
        }
    }
}

/// A state-variable band-pass (Chamberlin, oversampled-safe coefficients).
#[derive(Clone, Copy, Default)]
struct Svf {
    g: f32,
    k: f32,
    s1: f32,
    s2: f32,
}

impl Svf {
    fn set(&mut self, freq: f32, q: f32, sr: f32) {
        let f = freq.clamp(10.0, sr * 0.45);
        self.g = (std::f32::consts::PI * f / sr).tan();
        self.k = 1.0 / q.max(0.05);
    }
    fn band(&mut self, x: f32) -> f32 {
        let (g, k) = (self.g, self.k);
        let hp = (x - (g + k) * self.s1 - self.s2) / (1.0 + g * (g + k));
        let bp = g * hp + self.s1;
        self.s1 = g * hp + bp;
        let lp = g * bp + self.s2;
        self.s2 = g * bp + lp;
        bp * k
    }
}

/// A one-pole high-pass.
#[derive(Clone, Copy, Default)]
struct Hp {
    a: f32,
    x1: f32,
    y1: f32,
}

impl Hp {
    fn set(&mut self, freq: f32, sr: f32) {
        self.a = if freq <= 0.0 { 1.0 } else { (-std::f32::consts::TAU * freq / sr).exp() };
    }
    fn run(&mut self, x: f32) -> f32 {
        if self.a >= 1.0 {
            return x;
        }
        let y = self.a * (self.y1 + x - self.x1);
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// A noise layer while it plays.
#[derive(Clone, Copy, Default)]
struct NoiseRt {
    p: Noise,
    bp: [Svf; 2],
    hp: [Hp; 2],
    env: f32,
    rise: f32,
    att: f32,
    dec: f32,
    pos: usize,
}

impl NoiseRt {
    fn start(&mut self, p: Noise, sr: f32, pos: usize) {
        self.p = p;
        for (bp, hp) in self.bp.iter_mut().zip(self.hp.iter_mut()) {
            bp.set(p.freq, p.q, sr);
            hp.set(p.hp, sr);
        }
        self.att = if p.attack <= 0.0 { 1.0 } else { 1.0 - (-1.0 / (p.attack * sr)).exp() };
        self.dec = (-1.0 / (p.decay.max(1e-4) * sr)).exp();
        self.rise = 0.0;
        self.env = 1.0;
        self.pos = pos;
    }
    fn active(&self) -> bool {
        self.p.level > 0.0 && self.env > 1e-5
    }
    fn next(&mut self, rng: &mut u32, pink: &mut [f32; 3]) -> (f32, f32) {
        if !self.active() {
            return (0.0, 0.0);
        }
        let (a, b) = match self.p.source {
            Source::White => (white(rng), white(rng)),
            Source::Pink => {
                let w = white(rng);
                // Paul Kellet's economy filter.
                pink[0] = 0.99765 * pink[0] + w * 0.099_046;
                pink[1] = 0.963 * pink[1] + w * 0.296_516_4;
                pink[2] = 0.57 * pink[2] + w * 1.052_691_3;
                let p = (pink[0] + pink[1] + pink[2] + w * 0.1848) * 0.2;
                (p, if self.p.width > 0.0 { white(rng) * 0.3 + p * 0.7 } else { p })
            }
            Source::Modal(k) => {
                let all = noise::modal_noises();
                let k = k.min(noise::MODAL - 1);
                let src = &all[k * noise::LEN..(k + 1) * noise::LEN];
                let off = (self.p.width * 4000.0) as usize;
                let a = src[self.pos % noise::LEN];
                let b = src[(self.pos + off) % noise::LEN];
                self.pos += 1;
                (a, b)
            }
        };
        self.rise += (1.0 - self.rise) * self.att;
        let g = self.p.level * self.env * self.rise;
        self.env *= self.dec;
        let l = self.hp[0].run(self.bp[0].band(a));
        let r = if self.p.width > 0.0 { self.hp[1].run(self.bp[1].band(b)) } else { l };
        (l * g, r * g)
    }
}

fn white(s: &mut u32) -> f32 {
    *s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*s as i32) as f32 * (1.0 / 2_147_483_648.0)
}

/// A drum voice playing [`Patch`]es: one strike at a time (a new note restarts it).
pub struct Voice {
    sr: f32,
    p: Patch,
    /// Modes: rotating phasors (re, im), their per-sample rotation and decay.
    re: [f32; MODES],
    im: [f32; MODES],
    cos: [f32; MODES],
    sin: [f32; MODES],
    damp: [f32; MODES],
    gl: [f32; MODES],
    gr: [f32; MODES],
    /// Body oscillator.
    phase: f32,
    osc_level: f32,
    /// The modes' swell: its level and per-sample approach to 1.
    swell: f32,
    swell_k: f32,
    osc_hold: u32,
    osc_dec: f32,
    t: f32,
    click: NoiseRt,
    noise: NoiseRt,
    rng: u32,
    pink: [f32; 3],
    /// Samples to the next control update.
    countdown: u32,
    /// The choke's fade gain (1: none) and its per-sample factor.
    fade: f32,
    fade_k: f32,
    sounding: bool,
    /// Where the modal noises are read from next (varies per note).
    noise_pos: usize,
}

impl Voice {
    pub fn new(sr: f32) -> Voice {
        noise::warm();
        Voice {
            sr,
            p: Patch::default(),
            re: [0.0; MODES],
            im: [0.0; MODES],
            cos: [1.0; MODES],
            sin: [0.0; MODES],
            damp: [0.0; MODES],
            gl: [0.0; MODES],
            gr: [0.0; MODES],
            phase: 0.0,
            osc_level: 0.0,
            swell: 1.0,
            swell_k: 0.0,
            osc_hold: 0,
            osc_dec: 0.0,
            t: 0.0,
            click: NoiseRt::default(),
            noise: NoiseRt::default(),
            rng: 0x1234_5678,
            pink: [0.0; 3],
            countdown: 0,
            fade: 1.0,
            fade_k: 1.0,
            sounding: false,
            noise_pos: 0,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
    }

    /// Start a strike.
    pub fn strike(&mut self, p: Patch) {
        self.p = p;
        let sr = self.sr;
        for k in 0..p.n_modes {
            let m = p.modes[k];
            // Equal-power pan.
            let a = (m.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            self.gl[k] = m.level * a.cos() * std::f32::consts::SQRT_2;
            self.gr[k] = m.level * a.sin() * std::f32::consts::SQRT_2;
            if p.scatter {
                // A fixed spread per mode, so strikes sound alike.
                let a = (k as f32 * 2.399_963).rem_euclid(std::f32::consts::TAU);
                self.re[k] = a.cos();
                self.im[k] = a.sin();
            } else {
                self.re[k] = 1.0;
                self.im[k] = 0.0;
            }
            self.damp[k] = (-1.0 / (m.decay.max(1e-4) * sr)).exp();
        }
        self.phase = 0.0;
        self.osc_level = 1.0;
        (self.swell, self.swell_k) = if p.swell > 0.0 { (0.0, 1.0 - (-1.0 / (p.swell * sr)).exp()) } else { (1.0, 0.0) };
        self.osc_hold = (p.osc.hold.max(0.0) * sr) as u32;
        self.osc_dec = (-1.0 / (p.osc.decay.max(1e-4) * sr)).exp();
        self.t = 0.0;
        self.noise_pos = (self.noise_pos + 7919) % noise::LEN;
        self.click.start(p.click, sr, self.noise_pos);
        self.noise.start(p.noise, sr, self.noise_pos);
        self.countdown = 0;
        self.fade = 1.0;
        self.fade_k = 1.0;
        self.sounding = true;
    }

    /// Fade out quickly (choke, engine change).
    pub fn choke(&mut self) {
        if self.sounding {
            self.fade_k = (-1.0 / (CHOKE * self.sr)).exp();
        }
    }

    /// Stop at once.
    pub fn reset(&mut self) {
        self.sounding = false;
        self.p.n_modes = 0;
    }

    pub fn active(&self) -> bool {
        self.sounding
    }

    /// Per control block: the modes' rotation for the bent pitch.
    fn control(&mut self) {
        let bend = (self.p.bend * (-self.t / self.p.bend_fall.max(1e-4)).exp()).exp2();
        let w0 = std::f32::consts::TAU / self.sr * bend;
        for k in 0..self.p.n_modes {
            let w = (self.p.modes[k].freq * w0).min(std::f32::consts::PI * 0.98);
            self.cos[k] = w.cos() * self.damp[k];
            self.sin[k] = w.sin() * self.damp[k];
        }
    }

    /// Render `l.len()` frames, overwriting `l` and `r`.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if !self.sounding {
            l.fill(0.0);
            r.fill(0.0);
            return;
        }
        let sr = self.sr;
        let dt = 1.0 / sr;
        let o = self.p.osc;
        let mut energy = 0.0f32;
        for (lo, ro) in l.iter_mut().zip(r.iter_mut()) {
            if self.countdown == 0 {
                self.control();
                self.countdown = BLOCK;
            }
            self.countdown -= 1;
            let (mut a, mut b) = (0.0f32, 0.0f32);
            // Body oscillator.
            if o.level > 0.0 && self.osc_level > 1e-6 {
                let f = o.f_end + (o.f_start - o.f_end) * (-self.t / o.fall.max(1e-4)).exp();
                self.phase += f * dt;
                self.phase -= self.phase.floor();
                let mut s = (self.phase * std::f32::consts::TAU).sin();
                if o.drive > 0.0 {
                    s = (s * (1.0 + o.drive)).tanh() / (1.0 + o.drive).tanh();
                }
                let v = s * o.level * self.osc_level;
                a += v;
                b += v;
                if self.osc_hold > 0 {
                    self.osc_hold -= 1;
                } else {
                    self.osc_level *= self.osc_dec;
                }
            }
            // Modes.
            let (mut ma, mut mb) = (0.0f32, 0.0f32);
            for k in 0..self.p.n_modes {
                let (re, im) = (self.re[k], self.im[k]);
                let (c, s) = (self.cos[k], self.sin[k]);
                self.re[k] = re * c - im * s;
                self.im[k] = re * s + im * c;
                let y = self.im[k];
                ma += y * self.gl[k];
                mb += y * self.gr[k];
            }
            a += ma * self.swell;
            b += mb * self.swell;
            self.swell += (1.0 - self.swell) * self.swell_k;
            // Click and noise.
            let (cl, cr) = self.click.next(&mut self.rng, &mut self.pink);
            let (nl, nr) = self.noise.next(&mut self.rng, &mut self.pink);
            a += cl + nl;
            b += cr + nr;
            let g = self.p.gain * self.fade;
            self.fade *= self.fade_k;
            *lo = a * g;
            *ro = b * g;
            energy = energy.max(lo.abs()).max(ro.abs());
            self.t += dt;
        }
        // Silent: the oscillator, every mode and both noise layers have died away.
        let modes_live = (0..self.p.n_modes).any(|k| (self.re[k] * self.re[k] + self.im[k] * self.im[k]) * self.p.modes[k].level > 1e-10);
        let osc_live = o.level > 0.0 && self.osc_level > 1e-6;
        if self.fade < 1e-5 || (!modes_live && !osc_live && !self.click.active() && !self.noise.active() && energy < 1e-5) {
            self.sounding = false;
        }
    }
}

/// A metal's modes: `n` inharmonic modes scattered (from `seed`, the same every time)
/// between `lo` and `hi` Hz on a log scale, levels tilted by `tilt` dB per octave above
/// `lo` and lifted by `boost` dB within the band `band` (Hz), decays from `decay` (s) at
/// `lo` shortened by `shorten` per octave, panned at random within ±`width`.
#[allow(clippy::too_many_arguments)]
pub fn metal(
    x: &mut Patch,
    seed: u32,
    n: usize,
    lo: f32,
    hi: f32,
    tilt: f32,
    band: (f32, f32),
    boost: f32,
    decay: f32,
    shorten: f32,
    width: f32,
) {
    let mut s = seed.wrapping_mul(2_654_435_761) | 1;
    let mut rnd = || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s as f32 / u32::MAX as f32
    };
    let octaves = (hi / lo).log2();
    for i in 0..n {
        let t = (i as f32 + rnd()) / n as f32;
        let f = lo * (t * octaves).exp2();
        let oct = t * octaves;
        let mut db = tilt * oct - 6.0 * rnd();
        if f >= band.0 && f <= band.1 {
            db += boost;
        }
        let d = decay * (-shorten * oct).exp2() * (0.7 + 0.6 * rnd());
        let pan = (rnd() * 2.0 - 1.0) * width;
        x.mode(f, 10f32.powf(db / 20.0), d, pan);
    }
}

/// `x` (0..1) mapped onto `lo..hi` exponentially (both > 0).
pub fn expmap(x: f32, lo: f32, hi: f32) -> f32 {
    lo * (hi / lo).powf(x.clamp(0.0, 1.0))
}

/// Linear map of `x` (0..1) onto `lo..hi`.
pub fn lin(x: f32, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * x.clamp(0.0, 1.0)
}

/// A velocity's gain under a Velocity setting `s` (−1..1): full at `s = 0`; at 1 the
/// level follows velocity, at −1 it is inverted.
pub fn vel_gain(v: f32, s: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    let w = if s < 0.0 { v } else { 1.0 - v };
    let x = 1.0 - w * s.abs();
    x * x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(v: &mut Voice, n: usize) -> Vec<f32> {
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        let mut out = Vec::new();
        while out.len() < n {
            v.process(&mut l, &mut r);
            out.extend_from_slice(&l);
        }
        out
    }

    /// A strike of each part sounds, then dies away and reports silence.
    #[test]
    fn strikes_sound_and_end() {
        let mut v = Voice::new(48_000.0);
        let mut p = Patch {
            osc: Osc { level: 0.8, f_start: 200.0, f_end: 50.0, fall: 0.03, hold: 0.02, decay: 0.1, drive: 0.5 },
            ..Patch::default()
        };
        p.mode(120.0, 0.3, 0.2, 0.0);
        p.click = Noise { level: 0.5, source: Source::White, freq: 3000.0, q: 0.7, hp: 0.0, attack: 0.0, decay: 0.003, width: 0.0 };
        p.noise = Noise { level: 0.2, source: Source::Modal(3), freq: 5000.0, q: 1.0, hp: 200.0, attack: 0.001, decay: 0.1, width: 0.5 };
        v.strike(p);
        let y = render(&mut v, 24_000);
        let peak = y.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.3 && y.iter().all(|x| x.is_finite()), "peak {peak}");
        render(&mut v, 96_000);
        assert!(!v.active(), "it ends");
    }

    #[test]
    fn choke_fades_fast() {
        let mut v = Voice::new(48_000.0);
        let mut p = Patch::default();
        p.mode(200.0, 0.5, 5.0, 0.0);
        v.strike(p);
        render(&mut v, 4800);
        v.choke();
        render(&mut v, 4800);
        assert!(!v.active());
    }
}
