//! Stutter, as Maschine's (docs/fx-perform-stutter.md): touched, it loops a step of the
//! input, the strip (or the Length knob) choosing the step: 1/2 down to 1/64 with
//! Quantize (relative to the tempo without). With Quantize and the song playing, a touch
//! in a step's first quarter takes that step from its grid line; later (or in Reverse)
//! the input plays on to the next grid line and the stutter starts there. Up to 4 s
//! after the loop's start is taken, whatever plays, so the Length can grow while held.
//! Pitch is relative to its value at the touch; Gate shortens each pass (a swell in
//! Reverse); Both follows the strip's movement. The repeats pass a 35 Hz high-pass, and
//! each pass fades in and out over half a millisecond.

use super::kit::tan5d;
use super::{Ctx, Mode, Params};

/// The longest loop, s.
const MAX_LOOP: f64 = 4.0;
/// Loops per beat at Length factor 1.
const BASE: f32 = 0.5;
/// The fades at a pass's ends, s.
const DECLICK: f32 = 0.0005;
/// How far the strip moves against the direction before Both turns.
const HYSTERESIS: f32 = 0.1;
/// Pitch at the touch below which the pitch is measured from the bottom.
const PITCH_FLOOR: f64 = 0.15f32 as f64;
/// The pitch at the top: an octave up.
const PITCH_TOP: f64 = 2.0;
/// The wet high-pass, Hz, and its damping.
const DC_HZ: f64 = 35.0;
const DC_K: f64 = 2.0;
/// The dry delay line's length.
const DRY: usize = 3;
/// The processor's control period: the core runs in stretches this long.
const STRETCH: usize = 128;

/// A linear glide: set a target and the value moves in equal steps over its length,
/// landing on it.
#[derive(Debug, Clone, Copy, Default)]
struct Glide {
    cur: f32,
    target: f32,
    step: f32,
    len: f32,
    steps: i32,
    count: i32,
    moving: bool,
}

impl Glide {
    fn new(sr: f32, ms: f32) -> Glide {
        let len = sr * 0.001 * ms;
        Glide { len, steps: len.round() as i32, ..Glide::default() }
    }

    fn set(&mut self, t: f32) {
        self.target = t;
        if self.cur != t {
            self.step = (t - self.cur) / self.len;
            self.count = self.steps;
            self.moving = true;
        }
    }

    /// There at once.
    fn jump(&mut self, v: f32) {
        self.target = v;
        self.cur = v;
        self.step = 0.0;
        self.count = 0;
    }

    fn land(&mut self) {
        if self.moving {
            self.cur = self.target;
            self.step = 0.0;
            self.count = 0;
            self.moving = false;
        }
    }

    fn tick(&mut self) {
        if (self.target - self.cur).abs() > 0.0 {
            let c = self.count;
            self.count = c - 1;
            if c >= 2 {
                self.cur += self.step;
                return;
            }
        }
        self.land();
    }
}

/// The wet signal's high-pass: a 2-pole state-variable filter (trapezoidal), `x − k·bp −
/// lp`, in double.
#[derive(Debug, Clone, Copy, Default)]
struct Hp {
    s1: f64,
    s2: f64,
    a1: f64,
    a2: f64,
    a3: f64,
}

impl Hp {
    fn new(inv_sr: f64) -> Hp {
        let g = tan5d(inv_sr * (DC_HZ * std::f64::consts::PI));
        let a1 = 1.0 / g.mul_add(DC_K + g, 1.0);
        let a2 = g * a1;
        Hp { a1, a2, a3: g * a2, ..Hp::default() }
    }

    fn tick(&mut self, x: f64) -> f32 {
        let v3 = x - self.s2;
        let v1 = self.a1.mul_add(self.s1, v3 * self.a2);
        let v2 = self.s2 + self.a2.mul_add(self.s1, v3 * self.a3);
        self.s1 = v1.mul_add(2.0, -self.s1);
        self.s2 = v2.mul_add(2.0, -self.s2);
        ((-DC_K).mul_add(v1, x) - v2) as f32
    }

    fn clear(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

/// Raw parameter slots, as the core numbers them.
const POS: usize = 0;
const ON: usize = 1;
const ASSIGN: usize = 2;
const LENGTH: usize = 3;
const PITCH: usize = 4;
const DIRECTION: usize = 5;
const QUANTIZE: usize = 6;
const RETRIGGER: usize = 7;
const GATE: usize = 8;

pub struct Stutter {
    sr: f32,
    inv_sr: f64,
    /// The parameters as the core holds them (enums as Maschine sends them), and those
    /// changed but not yet applied (they land at the next sample, after the tempo and
    /// the song position, as Maschine's events follow its block's start).
    raw: [f32; 9],
    seen: [Option<f32>; 9],
    due: [bool; 9],
    bpm: Option<f32>,
    position: Option<(f64, bool)>,
    engaged: bool,
    /// TS Assign = Pitch.
    pitch_mode: bool,
    strip: f32,
    /// Samples played since the loop's start (fractional), the loop clock (samples; it
    /// wraps at the loop's length without Quantize), the read offset in the loop, the
    /// samples taken since the loop's start, and the seconds since this pass began.
    played: f64,
    clock: f64,
    offset: f64,
    taken: u64,
    pass: f64,
    /// The buffers' length (the most taken is half of it).
    cap: u64,
    /// Beats per second, now and at the touch.
    bps: f64,
    bps0: f64,
    /// The pitch control's value at the touch.
    p0: f32,
    /// Both: the last Length position, and the lowest and highest since the last turn.
    last: f32,
    low: f32,
    high: f32,
    retrigger: bool,
    /// The loop clock's step per sample.
    rate: f64,
    /// Heard at the last sample (the high-pass starts afresh when it's heard again).
    heard: bool,
    /// The loop's length (s) and its inverse, and the length asked for (it takes over at
    /// the next wrap).
    len: f64,
    inv_len: f64,
    next_len: f64,
    /// Loops per beat × 2 (the Length's factor × 0.5).
    factor: f32,
    /// Forward now, forward at the next wrap, and whether a shorter loop asked for cuts
    /// this pass short (it fades out at its end).
    fwd: bool,
    fwd_next: bool,
    cut: bool,
    /// The song position at the last block's start, whether it plays, and the samples
    /// since.
    song: f64,
    playing: bool,
    count: i32,
    /// A late touch waiting for its grid line, and the samples to it.
    late: bool,
    wait: f64,
    /// 0 off; 1–3 the grids (Maschine's stored on/off reaches 0 and 3).
    quantize: u32,
    /// 0 Forward, 1 Reverse, 2 Both.
    direction: u32,
    buf: [Vec<f32>; 2],
    w: usize,
    dry: [[f32; DRY]; 2],
    dw: usize,
    hp: [Hp; 2],
    fade: Glide,
    gate_g: Glide,
    gate: Glide,
    pitch: Glide,
    /// Samples into the current stretch, and the direction as it was at its start.
    stretch: usize,
    fwd_at_stretch: bool,
}

impl Stutter {
    pub fn new(sr: f32) -> Stutter {
        let inv_sr = 1.0 / f64::from(sr);
        let n = MAX_LOOP.mul_add(2.0 * f64::from(sr), 3.0) as usize;
        let mut s = Stutter {
            sr,
            inv_sr,
            raw: [0.0; 9],
            seen: [None; 9],
            due: [false; 9],
            bpm: None,
            position: None,
            engaged: false,
            pitch_mode: false,
            strip: 0.0,
            played: 0.0,
            clock: 0.0,
            offset: 0.0,
            taken: 0,
            pass: 0.0,
            cap: n as u64,
            bps: 2.0,
            bps0: 1.0,
            p0: 1.0,
            last: 0.0,
            low: 0.0,
            high: 0.0,
            retrigger: true,
            rate: 1.0,
            heard: false,
            len: 1.0,
            inv_len: 1.0,
            next_len: 1.0,
            factor: 1.0,
            fwd: true,
            fwd_next: true,
            cut: false,
            song: 0.0,
            playing: false,
            count: 0,
            late: false,
            wait: 0.0,
            quantize: 0,
            direction: 0,
            buf: [vec![0.0; n], vec![0.0; n]],
            w: 0,
            dry: [[0.0; DRY]; 2],
            dw: 0,
            hp: [Hp::new(inv_sr); 2],
            fade: Glide::new(sr, 10.0),
            gate_g: Glide::new(sr, 20.0),
            gate: Glide::new(sr, 20.0),
            pitch: Glide::new(sr, 100.0),
            stretch: 0,
            fwd_at_stretch: true,
        };
        s.loop_length();
        s
    }

    /// The loop length the Length asks for at this tempo (halved until a half note fits
    /// in the buffer), and the Gate's shape for it.
    fn loop_length(&mut self) {
        let bps = if self.bps < 0.001 { 0.001 } else { self.bps };
        let len = 1.0 / (bps * f64::from(self.factor));
        let half = 1.0 / (bps * f64::from(BASE));
        let most = self.inv_sr.mul_add(-10.0, MAX_LOOP);
        let mut k = 1.0;
        if half > most {
            loop {
                k *= 0.5;
                if half * k <= most {
                    break;
                }
            }
        }
        let len = len * k;
        if len != self.next_len {
            self.cut = false;
        }
        self.next_len = len;
        self.gate_shape();
    }

    /// The Gate's open fraction for the loop (at least 10 % of a loop of 0.1 s or more,
    /// up to half of very short ones) and its raw value, both gliding.
    fn gate_shape(&mut self) {
        let gate = self.raw[GATE];
        let s = (self.len as f32 + -0.01) / 0.09;
        let mut c = if s > 1.0 { 1.0 } else { s };
        c = if s < 0.0 { 0.0 } else { c };
        c = if c < 0.0 { 0.0 } else { c };
        let b = (f64::from(c)).mul_add(f64::from(-0.4f32), 0.5) as f32;
        let mut gc = if gate > 1.0 { 1.0 } else { gate };
        gc = if gate < 0.0 { 0.0 } else { gc };
        let g = f64::from(gc).mul_add(f64::from(1.06f32 - b), f64::from(b)) as f32;
        self.gate_g.set(g);
        self.gate.set(gate);
    }

    /// The Length at `x`: the factor (a power of two with Quantize, continuous without)
    /// and, in Both, the direction by the strip's movement.
    fn set_length(&mut self, x: f32) {
        match self.direction {
            0 => self.fwd_next = true,
            1 => self.fwd_next = false,
            2 => {
                let prev = self.last;
                let turn = if prev < x {
                    if !self.fwd_next {
                        if prev < self.low {
                            self.low = prev;
                        }
                        self.low + HYSTERESIS < x
                    } else {
                        false
                    }
                } else if self.fwd_next && prev > x {
                    if prev > self.high {
                        self.high = prev;
                    }
                    self.high - HYSTERESIS > x
                } else {
                    false
                };
                if turn {
                    self.fwd_next = prev < x;
                    self.low = x;
                    self.high = x;
                }
            }
            _ => {}
        }
        self.last = x;
        let f = if self.quantize != 0 {
            let k = ((-x.abs()).mul_add(0.3, 1.3) * x * 5.0 + 0.5) as i32;
            (1i32 << (k & 31)) as f32
        } else {
            exp2f((-x.abs()).mul_add(-0.3, 0.7) * x * 5.0 + 0.5)
        };
        self.factor = f * BASE;
        self.loop_length();
    }

    fn strip_moved(&mut self, x: f32) {
        self.strip = x;
        if !self.pitch_mode {
            self.set_length(x);
        } else {
            self.pitch.set(x);
        }
        if !self.engaged {
            self.pitch.land();
        }
    }

    /// TS Assign: the strip plays Length (false) or Pitch.
    fn set_mode(&mut self, pitch_mode: bool) {
        let was = self.pitch_mode;
        self.pitch_mode = pitch_mode;
        self.strip = self.raw[POS];
        if !pitch_mode {
            self.set_length(self.strip);
        } else {
            self.pitch.set(self.strip);
        }
        if !self.engaged {
            self.pitch.land();
        }
        let from_knob = !pitch_mode;
        if !pitch_mode {
            self.direction = direction(self.raw[DIRECTION]);
            match self.direction {
                0 => self.fwd_next = true,
                1 => self.fwd_next = false,
                _ => {}
            }
            self.retrigger = true;
        } else {
            self.set_length(self.raw[LENGTH]);
            self.direction = 0;
            self.fwd_next = true;
            self.retrigger = self.raw[RETRIGGER] > 0.5;
        }
        if was != pitch_mode {
            self.p0 = if from_knob { self.raw[PITCH] } else { self.strip };
        }
        self.pitch.jump(self.p0);
    }

    /// The strip touched (or let go): the loop starts now (no Quantize), at this step's
    /// grid line (Quantize, touched in its first quarter, not Reverse), or at the next
    /// grid line (the input plays on until then).
    fn set_on(&mut self, on: bool) {
        if !on {
            self.late = false;
        } else if !self.engaged {
            let fwd = self.direction != 1;
            self.fwd_next = fwd;
            self.fwd = fwd;
            self.loop_length();
            let bps = self.bps;
            self.bps0 = bps;
            if self.quantize == 0 {
                self.taken = 0;
                self.wait = 0.0;
                self.played = 0.0;
                self.clock = 0.0;
            } else {
                let q: f32 = match self.quantize {
                    3 => 4.0,
                    2 => 2.0,
                    _ => 1.0,
                };
                let q = if q < self.factor { q } else { self.factor };
                let grid = 1.0 / f64::from(q);
                let at = self.song + bps * (self.inv_sr * f64::from(self.count));
                let into = if at >= 0.0 { at % grid } else { grid - (-at % grid) };
                if into < grid * 0.25 && self.fwd_next {
                    let t = into / bps;
                    let s = t * f64::from(self.sr);
                    self.played = s;
                    self.clock = s;
                    self.taken = s as u64;
                    self.pass = t;
                    self.late = false;
                } else {
                    self.late = true;
                    self.wait = ((grid - into) * f64::from(self.sr)) / bps;
                }
            }
            self.p0 = if self.pitch_mode { self.strip } else { self.raw[PITCH] };
            self.pitch.jump(self.p0);
            self.low = self.strip;
            self.high = self.strip;
        }
        if !self.late {
            self.fade.set(if on { 1.0 } else { 0.0 });
        }
        self.engaged = on;
    }

    fn set_parameter(&mut self, i: usize, v: f32) {
        self.raw[i] = v;
        match i {
            POS => self.strip_moved(v),
            ON => self.set_on(v > 0.5),
            ASSIGN => self.set_mode(v > 0.5),
            LENGTH => {
                if self.pitch_mode {
                    self.set_length(v);
                }
            }
            PITCH => {
                if !self.pitch_mode {
                    self.pitch.set(v);
                }
            }
            DIRECTION => {
                if self.pitch_mode {
                    self.direction = 0;
                    self.fwd_next = true;
                } else {
                    self.direction = direction(v);
                    match self.direction {
                        0 => self.fwd_next = true,
                        1 => self.fwd_next = false,
                        _ => {}
                    }
                }
            }
            QUANTIZE => {
                self.quantize = if v >= 0.25 { if v >= 0.5 { if v >= 0.75 { 3 } else { 2 } } else { 1 } } else { 0 };
                self.set_length(self.last);
                self.late = false;
            }
            RETRIGGER => self.retrigger = !self.pitch_mode || v > 0.5,
            GATE => self.gate_shape(),
            _ => {}
        }
    }

    /// Changes waiting for the next sample, in Maschine's order: the tempo, the song
    /// position, then the parameters by index.
    fn apply(&mut self) -> bool {
        let mut any = false;
        if let Some(bpm) = self.bpm.take() {
            let bps = f64::from(bpm) / 60.0;
            if bps != self.bps {
                self.bps = bps;
                self.loop_length();
            }
        }
        if let Some((q, playing)) = self.position.take() {
            self.song = q;
            if playing || self.playing {
                self.count = 0;
            }
            self.playing = playing;
            any = true;
        }
        for i in 0..9 {
            if std::mem::take(&mut self.due[i]) {
                self.set_parameter(i, self.raw_due(i));
                any = true;
            }
        }
        any
    }

    fn raw_due(&self, i: usize) -> f32 {
        self.seen[i].unwrap_or(0.0)
    }

    /// Maschine's reset: the buffers and filters cleared, the glides landed.
    fn reset(&mut self) {
        for b in &mut self.buf {
            b.fill(0.0);
        }
        self.w = 0;
        self.dry = [[0.0; DRY]; 2];
        self.dw = 0;
        self.heard = false;
        self.late = false;
        self.fade.cur = 0.0;
        self.fade.step = 0.0;
        self.fade.count = 0;
        self.played = 0.0;
        self.clock = 0.0;
        self.taken = 0;
        self.pass = 0.0;
        self.gate_g.land();
        self.gate.land();
        self.pitch.land();
        for h in &mut self.hp {
            h.clear();
        }
    }

    /// Maschine's resume: a strip held stays held, touched afresh.
    fn resume(&mut self) {
        if self.engaged {
            self.late = false;
            self.fade.set(0.0);
            self.engaged = false;
            self.set_on(true);
        }
        self.p0 = if self.pitch_mode { self.strip } else { self.raw[PITCH] };
    }

    /// The pitch's rate (relative to the touch) and level (it fades out at the very
    /// bottom).
    fn pitch_rate(&self) -> (f64, f32) {
        let p = self.pitch.cur;
        let pd = f64::from(p);
        let p0 = f64::from(self.p0);
        if PITCH_FLOOR > p0 {
            let t = PITCH_TOP - (PITCH_TOP - 1.0) / f64::from(1.0 - self.p0);
            return (pd.mul_add(PITCH_TOP - t, t), 1.0);
        }
        let rate = if 1.0 / PITCH_TOP > p0 && p >= self.p0 {
            let u = (pd - p0) / (1.0 - p0);
            let c = if u > 1.0 { 1.0 } else { u };
            let c = if u < 0.0 { 0.0 } else { c };
            c.mul_add(PITCH_TOP - 1.0, 1.0)
        } else {
            pd / p0
        };
        let l = p / 0.1;
        let lc = if l > 1.0 { 1.0 } else { l };
        (rate, if l < 0.0 { 0.0 } else { lc })
    }

    /// One channel's loop sample `back` behind the write head (4-point cubic).
    fn read(&self, ch: usize, base: i64, t: f64) -> f32 {
        let n = self.buf[ch].len() as i64;
        let at = |k: i64| self.buf[ch][(base + k).rem_euclid(n) as usize];
        let (a, b, c, d) = (at(-1), at(0), at(1), at(2));
        let c3 = d.mul_add(0.5, c.mul_add(-1.5, a.mul_add(-0.5, b * 1.5)));
        let c2 = d.mul_add(-0.5, c.mul_add(2.0, b.mul_add(-2.5, a)));
        let c1 = a.mul_add(-0.5, c * 0.5);
        let t2 = t * t;
        let t3 = t * t2;
        let y = f64::from(c3).mul_add(t3, t2 * f64::from(c2));
        (f64::from(c1).mul_add(t, y) + f64::from(b)) as f32
    }
}

fn direction(v: f32) -> u32 {
    if v > 0.666 { 2 } else { u32::from(v > 0.333) }
}

/// `exp2f` from the system math library, which Maschine's Stutter calls.
#[cfg(target_os = "macos")]
fn exp2f(x: f32) -> f32 {
    unsafe extern "C" {
        fn exp2f(x: f32) -> f32;
    }
    // SAFETY: a pure libm function.
    unsafe { exp2f(x) }
}

#[cfg(not(target_os = "macos"))]
fn exp2f(x: f32) -> f32 {
    x.exp2()
}

impl Mode for Stutter {
    fn set(&mut self, p: &Params) {
        let v = [
            p.f("touchstrip_pos"),
            if p.f("touchstrip_on_off") >= 0.5 { 1.0 } else { 0.0 },
            p.i("stutter_ts_assign") as f32,
            p.f("stutter_length"),
            p.f("stutter_pitch"),
            p.i("stutter_direction") as f32 * (1.0 / 3.0f32),
            if p.f("stutter_quantize") >= 0.5 { 1.0 } else { 0.0 },
            if p.f("stutter_retrigger") >= 0.5 { 1.0 } else { 0.0 },
            p.f("stutter_gate"),
        ];
        for (i, x) in v.into_iter().enumerate() {
            if self.seen[i] != Some(x) {
                self.seen[i] = Some(x);
                self.due[i] = true;
            }
        }
    }

    fn set_bpm(&mut self, bpm: f32) {
        self.bpm = Some(bpm);
    }

    fn set_position(&mut self, quarters: f64, playing: bool) {
        self.position = Some((quarters, playing));
    }

    fn activate(&mut self) {
        // Settings first (Maschine's loading order), then reset and resume.
        if let Some(bpm) = self.bpm.take() {
            let bps = f64::from(bpm) / 60.0;
            if bps != self.bps {
                self.bps = bps;
                self.loop_length();
            }
        }
        for i in [ASSIGN, LENGTH, PITCH, DIRECTION, QUANTIZE, GATE, RETRIGGER, POS, ON] {
            if std::mem::take(&mut self.due[i]) {
                self.set_parameter(i, self.raw_due(i));
            }
        }
        self.reset();
        self.resume();
        self.stretch = 0;
    }

    fn frame(&mut self, l: f32, r: f32, _c: &Ctx) -> (f32, f32) {
        if self.apply() || self.stretch == 0 {
            self.fwd_at_stretch = self.fwd_next;
        }
        self.stretch = (self.stretch + 1) % STRETCH;
        self.count += 1;
        self.fade.tick();
        self.gate_g.tick();
        self.gate.tick();
        self.pitch.tick();
        let fade = self.fade.cur;
        if fade > 0.0 && !self.heard {
            for h in &mut self.hp {
                h.clear();
            }
        }
        // The dry signal, two samples late.
        self.dry[0][self.dw] = l;
        self.dry[1][self.dw] = r;
        self.dw = (self.dw + 1) % DRY;
        let read = (self.dw + DRY - 3) % DRY;
        let (dl, dr) = (self.dry[0][read], self.dry[1][read]);
        self.heard = fade > 0.0;
        // Taking: always while not heard, then up to half the buffer.
        if fade <= 0.0 || self.cap > self.taken * 2 {
            self.buf[0][self.w] = l;
            self.buf[1][self.w] = r;
            self.w = (self.w + 1) % self.buf[0].len();
            if fade > 0.0 {
                self.taken += 1;
            }
        }
        // A late touch's grid line.
        if fade < f32::from_bits(0x3f7f_be77) && self.late {
            self.wait += -1.0;
            if self.wait < 0.0 {
                let fwd = self.direction != 1;
                self.fwd_next = fwd;
                self.fwd = fwd;
                self.fade.set(1.0);
                self.taken = 0;
                self.wait = 0.0;
                self.late = false;
                self.played = 0.0;
                self.clock = 0.0;
            }
        }
        if fade <= 0.0 {
            return (fade.mul_add(0.0 - dl, dl), fade.mul_add(0.0 - dr, dr));
        }
        let (prate, level) = self.pitch_rate();
        let sr = f64::from(self.sr);
        let mut len = self.len;
        let phase = self.clock % (len * sr);
        let mut wrap = phase < self.rate;
        let rem = self.inv_sr.mul_add(-phase, len);
        let mut rem_next = rem;
        if self.next_len < len {
            // A shorter loop asked for: this pass may end at its length.
            let phase_next = self.clock % (self.next_len * sr);
            rem_next = (-self.inv_sr).mul_add(phase_next, self.next_len);
            if rem_next > f64::from(DECLICK) && phase_next > f64::from(self.sr * DECLICK) {
                self.cut = true;
            }
            if phase_next < self.rate {
                wrap = self.cut || phase < self.rate;
            }
        }
        if self.quantize == 0 {
            self.clock = phase;
        }
        self.pass += self.inv_sr;
        let mut at = phase;
        if wrap {
            self.pass = 0.0;
            self.offset = 0.0;
        }
        if self.retrigger {
            at = self.offset;
        }
        // The pass's fades: in from its start (not the loop's very first), out to its end
        // (or the shorter loop's).
        let mut out_f = (rem as f32) / DECLICK;
        out_f = if out_f > 1.0 { 1.0 } else { out_f };
        if self.cut {
            let o = (rem_next as f32) / DECLICK;
            out_f = if out_f < o { out_f } else { o };
        }
        let in_f = (self.pass as f32) / DECLICK;
        let in_f = if in_f > 1.0 { 1.0 } else { in_f };
        let in_f = if self.inv_sr * self.played > f64::from(DECLICK) { in_f } else { 1.0 };
        let out_m = out_f.min(1.0);
        let env = if in_f < out_m { in_f } else { out_m };
        if wrap {
            self.cut = false;
            self.len = self.next_len;
            self.inv_len = 1.0 / self.next_len;
            len = self.len;
            self.fwd = self.fwd_at_stretch;
        }
        let taken = self.taken as f64;
        if !self.fwd {
            at = -at;
            if len * sr < taken {
                at = sr.mul_add(len, at);
            }
        }
        let at = if self.played < at { self.played } else { at };
        let back = (taken + 3.0) - at;
        let n = -back;
        let i = (n as i32) - i32::from(back >= 0.0);
        let t = n - f64::from(i);
        let base = (self.w as i64) + self.buf[0].len() as i64 + i64::from(i);
        let yl = level * self.read(0, base, t);
        let yr = level * self.read(1, base, t);
        let hl = self.hp[0].tick(f64::from(yl));
        let hr = self.hp[1].tick(f64::from(yr));
        self.played += 1.0;
        let g = if self.fwd {
            let gg = self.gate_g.cur;
            let lf = self.len as f32;
            let a = gg * 0.95 * lf;
            let b = gg * 1.05 * lf;
            let u = ((phase * self.inv_sr) as f32 - a) / (b - a);
            let x = if u > 1.0 { 0.0 } else { 1.0 - u };
            if u < 0.0 { 1.0 } else { x }
        } else {
            let gr = self.gate.cur;
            let tt = (self.inv_sr * (self.offset * self.inv_len)) as f32;
            let s = gr * gr.abs().mul_add(-0.5, 1.5);
            let st = f64::from(s).mul_add(f64::from(3.95f32), -3.0) as f32;
            let x = f64::from(tt).mul_add(f64::from(1.0 - st), f64::from(st)) as f32;
            let x1 = if x > 1.0 { 1.0 } else { x };
            let x = if x < 0.0 { 0.0 } else { x1 };
            // 3.1415 and −1.57075 as `f32`.
            let ph = f64::from(x).mul_add(f64::from(f32::from_bits(0x4049_0e56)), f64::from(f32::from_bits(0xbfc9_0e56))) as f32;
            let q = ph * ph;
            let mut p = q.mul_add(f32::from_bits(0xb2cd_4ca8), f32::from_bits(0x3638_b94f));
            p = p.mul_add(q, f32::from_bits(0xb950_0c03));
            p = p.mul_add(q, f32::from_bits(0x3c08_8885));
            p = p.mul_add(q, f32::from_bits(0xbe2a_aac1));
            let s = p.mul_add(q, 1.0);
            let v = (s * ph + 1.0) * 0.5 / f32::from_bits(0x3f7f_35f9);
            let v1 = if v > 1.0 { 1.0 } else { v };
            if v < 0.0 { 0.0 } else { v1 }
        };
        let (wl, wr) = (g * hl, g * hr);
        let step = (self.bps / self.bps0) * if self.retrigger { 1.0 } else { prate };
        self.rate = step;
        self.clock += step;
        self.offset = step.mul_add(prate, self.offset);
        let ol = fade.mul_add(env * wl - dl, dl);
        let or = fade.mul_add(env * wr - dr, dr);
        (ol, or)
    }
}

#[cfg(test)]
mod tests {
    use crate::fx::Effect;
    use crate::fx::perform::PerformFx;

    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../out/perform-ref/stutter/");

    fn wav(name: &str) -> Option<(u32, Vec<f32>, Vec<f32>)> {
        let mut r = hound::WavReader::open(format!("{DIR}{name}")).ok()?;
        let sr = r.spec().sample_rate;
        let v: Vec<f32> = r.samples::<f32>().map(Result::unwrap).collect();
        Some((sr, v.iter().step_by(2).copied().collect(), v.iter().skip(1).step_by(2).copied().collect()))
    }

    fn params(extra: &[(&str, f64)]) -> Vec<(String, f64)> {
        let mut p: Vec<(String, f64)> = deckard_model::PERFORM_FX.1.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        for (k, v) in [("mode", 5.0), ("touchstrip_on_off", 0.0)].iter().chain(extra) {
            p.iter_mut().find(|(n, _)| n == k).expect("a Perform FX field").1 = *v;
        }
        p
    }

    /// DECKARD over `input` as `maschine-ref render --play` runs Maschine: a 1 s preroll
    /// with the transport stopped, then playing from 0 at 120 BPM, in 1024-frame blocks,
    /// `changes` (seconds, field, value) at the start of the block they fall in.
    fn play(sr: u32, l: &[f32], r: &[f32], extra: &[(&str, f64)], changes: &[(f64, &str, f64)]) -> (Vec<f32>, Vec<f32>) {
        let mut p = params(extra);
        let mut f = PerformFx::new(sr as f64);
        f.set(&p);
        f.set_tempo(120.0);
        let pre = sr as usize;
        let mut done = 0;
        while done < pre {
            let k = 1024.min(pre - done);
            let (mut a, mut b) = (vec![0f32; k], vec![0f32; k]);
            f.set_transport(None);
            f.process(&mut a, &mut b);
            done += k;
        }
        let (mut ol, mut or) = (l.to_vec(), r.to_vec());
        for (k, (a, b)) in ol.chunks_mut(1024).zip(or.chunks_mut(1024)).enumerate() {
            let at = k * 1024;
            let due: Vec<_> = changes.iter().filter(|(t, ..)| (*t * sr as f64) as usize / 1024 * 1024 == at).collect();
            if !due.is_empty() {
                for (_, field, v) in due {
                    p.iter_mut().find(|(n, _)| n == field).expect("a Perform FX field").1 = *v;
                }
                f.set(&p);
            }
            f.set_transport(Some(maschine_position(at, sr)));
            f.process(a, b);
        }
        (ol, or)
    }

    /// The song position Maschine's Stutter is given for maschine-ref's block at `at` (the
    /// host's is `at/sr·tempo/60`). Maschine keeps its own, whose last bits we couldn't
    /// derive; these were found by search against the renders: one unit in the last
    /// place below the host's at 48 kHz, the host's at 96 kHz, and near the host's
    /// rounded to 240 000 ticks per quarter at 44.1 kHz (0.03 samples off: −97 dB).
    fn maschine_position(at: usize, sr: u32) -> f64 {
        let host = at as f64 / f64::from(sr) * 120.0 / 60.0;
        match sr {
            48_000 if host > 0.0 => f64::from_bits(host.to_bits() - 1),
            44_100 => (host * 240_000.0).round() / 240_000.0,
            _ => host,
        }
    }

    /// The residual against Maschine's render `name`, in dB below its level (L, R).
    fn null(name: &str, extra: &[(&str, f64)], changes: &[(f64, &str, f64)]) -> Option<(f64, f64)> {
        let (sr, il, ir) = wav(&format!("in-{name}.wav"))?;
        let (_, ml, mr) = wav(&format!("{name}.wav"))?;
        let (dl, dr) = play(sr, &il, &ir, extra, changes);
        let db = |d: &[f32], m: &[f32]| {
            let e = d.iter().zip(m).map(|(x, y)| ((x - y) as f64).powi(2)).sum::<f64>();
            let s = m.iter().map(|y| (*y as f64).powi(2)).sum::<f64>();
            10.0 * (e / s.max(1e-30)).max(1e-24).log10()
        };
        if std::env::var("NULL_DUMP").is_ok() {
            let spec = hound::WavSpec { channels: 2, sample_rate: sr, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
            let mut w = hound::WavWriter::create(format!("{DIR}dk-{name}.wav"), spec).unwrap();
            for (a, b) in dl.iter().zip(&dr) {
                w.write_sample(*a).unwrap();
                w.write_sample(*b).unwrap();
            }
        }
        if std::env::var("NULL_DEBUG").is_ok() {
            let w = (sr / 20) as usize;
            let rows: Vec<String> = dl.chunks(w).zip(ml.chunks(w)).map(|(a, b)| format!("{:.0}", db(a, b))).collect();
            eprintln!("{name}: per 50 ms: {}", rows.join(" "));
            if let Some(i) = dl.iter().zip(&ml).position(|(a, b)| a != b) {
                eprintln!("{name}: first difference at {i}: dk {} m {}", dl[i], ml[i]);
            }
        }
        Some((db(&dl, &ml), db(&dr, &mr)))
    }

    /// Maschine's own Stutter (`out/perform-ref/stutter/render.sh`), sample for sample:
    /// early and late touches, Reverse, Both, the Length grown and shrunk while engaged,
    /// Gate, Pitch, TS Assign Pitch, Quantize off, release, three sample rates.
    #[test]
    fn nulls_against_maschine() {
        let on = |t: f64| (t, "touchstrip_on_off", 1.0);
        let off = |t: f64| (t, "touchstrip_on_off", 0.0);
        let pos = |t: f64, v: f64| (t, "touchstrip_pos", v);
        let pitch = |t: f64, v: f64| (t, "stutter_pitch", v);
        let x = |k: &'static str, v: f64| (k, v);
        type Case = (&'static str, Vec<(&'static str, f64)>, Vec<(f64, &'static str, f64)>);
        let cases: Vec<Case> = vec![
            ("early", vec![x("touchstrip_pos", 0.5), x("stutter_direction", 0.0)], vec![on(1.024), off(2.5)]),
            ("late", vec![x("touchstrip_pos", 0.5), x("stutter_direction", 0.0)], vec![on(1.088), off(2.5)]),
            ("rev", vec![x("touchstrip_pos", 0.5), x("stutter_direction", 1.0)], vec![on(1.024), off(2.5)]),
            ("both", vec![x("touchstrip_pos", 0.5), x("stutter_direction", 2.0)], vec![on(1.024), pos(1.5, 0.0), pos(2.7, 1.0), off(3.5)]),
            ("grow", vec![x("touchstrip_pos", 0.5), x("stutter_direction", 0.0)], vec![on(1.024), pos(1.5, 0.0), pos(3.0, 0.5)]),
            ("g0", vec![x("touchstrip_pos", 0.5), x("stutter_gate", 0.0)], vec![on(1.024), off(2.5)]),
            ("g5", vec![x("touchstrip_pos", 0.3), x("stutter_gate", 0.5), x("stutter_direction", 1.0)], vec![on(1.024), off(2.5)]),
            ("pitch", vec![x("touchstrip_pos", 0.5), x("stutter_pitch", 0.75)], vec![on(1.024), pitch(1.6, 1.0), pitch(2.2, 0.05)]),
            ("pitchlo", vec![x("touchstrip_pos", 0.5), x("stutter_pitch", 0.1)], vec![on(1.024), pitch(1.6, 0.6)]),
            (
                "assign",
                vec![x("stutter_ts_assign", 1.0), x("stutter_length", 0.3), x("stutter_retrigger", 0.0), x("touchstrip_pos", 0.4)],
                vec![on(1.024), pos(1.5, 0.9), pos(2.0, 0.2)],
            ),
            ("free", vec![x("touchstrip_pos", 0.4), x("stutter_quantize", 0.0)], vec![on(1.024), pos(1.8, 0.8), off(2.5)]),
            ("early44", vec![x("touchstrip_pos", 0.5), x("stutter_direction", 0.0)], vec![on(1.024), off(2.5)]),
            ("early96", vec![x("touchstrip_pos", 0.5), x("stutter_direction", 0.0)], vec![on(1.024), off(2.5)]),
        ];
        let mut bad = Vec::new();
        for (name, extra, changes) in &cases {
            let Some((l, r)) = null(name, extra, changes) else { continue };
            eprintln!("{name}: {l:.1} / {r:.1} dB");
            // At 44.1 kHz Maschine's song position is only near ours (`maschine_position`).
            let worst = if *name == "early44" { -90.0 } else { -200.0 };
            if l > worst || r > worst {
                bad.push(format!("{name} {l:.1}/{r:.1}"));
            }
        }
        assert!(bad.is_empty(), "not null: {bad:?}");
    }

    fn fx(extra: &[(&str, f64)]) -> (PerformFx, Vec<(String, f64)>) {
        let p = params(extra);
        let mut f = PerformFx::new(48_000.0);
        f.set(&p);
        f.set_tempo(120.0);
        (f, p)
    }

    fn set(f: &mut PerformFx, p: &mut [(String, f64)], k: &str, v: f64) {
        p.iter_mut().find(|(n, _)| n == k).unwrap().1 = v;
        f.set(p);
    }

    fn run(f: &mut PerformFx, x: &[f32]) -> Vec<f32> {
        let (mut l, mut r) = (x.to_vec(), x.to_vec());
        for (a, b) in l.chunks_mut(1024).zip(r.chunks_mut(1024)) {
            f.process(a, b);
        }
        l
    }

    /// Released, the input comes out as it went in, two samples late.
    #[test]
    fn released_it_passes_the_input_two_samples_late() {
        let (mut f, _) = fx(&[]);
        let x: Vec<f32> = (0..4096).map(|i| ((i * 7919 % 1000) as f32 / 1000.0) - 0.5).collect();
        let out = run(&mut f, &x);
        assert_eq!(&out[2..], &x[..x.len() - 2]);
    }

    /// Quantized, the strip's start loops a half note and its end a 64th (120 BPM: 48 000
    /// and 1 500 samples): once taken, the output repeats with that period.
    #[test]
    fn the_loop_runs_from_a_half_note_to_a_sixty_fourth() {
        for (pos, period) in [(0.0, 48_000), (1.0, 1_500)] {
            let (mut f, mut p) = fx(&[("touchstrip_pos", pos), ("stutter_direction", 0.0)]);
            let x: Vec<f32> = (0..300_000u64).map(|i| (((i * 7919 + 13) % 1000) as f32 / 1000.0) - 0.5).collect();
            run(&mut f, &x[..48_000]);
            set(&mut f, &mut p, "touchstrip_on_off", 1.0);
            let out = run(&mut f, &x[48_000..]);
            let at = 200_000;
            let worst = (0..2_000).map(|i| (out[at + i] - out[at + i + period]).abs()).fold(0.0, f32::max);
            assert!(worst < 1e-3, "pos {pos}: {worst}");
        }
    }

    /// Touched late in a step (Quantize, the song playing), the input plays on to the next
    /// grid line, and the step from there repeats.
    #[test]
    fn a_late_touch_repeats_the_next_step() {
        let (mut f, mut p) = fx(&[("touchstrip_pos", 0.5), ("stutter_direction", 0.0)]);
        // 1/16 = 6 000 samples at 120 BPM; the steps begin at multiples of 6 000.
        let mut seed = 7u32;
        let x: Vec<f32> = (0..140_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1 << 24) as f32 - 0.5
            })
            .collect();
        f.set_transport(Some(0.0));
        run(&mut f, &x[..100_352]);
        // 100 352 is 4 352 into the step that began at 96 000 (73 %).
        set(&mut f, &mut p, "touchstrip_on_off", 1.0);
        let out = run(&mut f, &x[100_352..]);
        let at = |n: usize| n - 100_352;
        // Where in the input a stretch of the output came from (best correlation).
        let from = |o: usize| {
            let w = &out[at(o)..at(o) + 2_000];
            (90_000..o)
                .max_by_key(|&j| {
                    let c: f32 = w.iter().zip(&x[j..j + 2_000]).map(|(a, b)| a * b).sum();
                    (c * 1e4) as i64
                })
                .unwrap()
        };
        // The input comes out two samples late; the loop reads three behind its start.
        assert_eq!(from(101_000), 101_000 - 2, "the input plays on");
        assert_eq!(from(108_500), 102_500 - 3, "then the step from 102 000 repeats");
        assert_eq!(from(114_500), 102_500 - 3, "and again");
    }

    /// Gate 0 still lets the start of each pass through: a short blip, not silence.
    #[test]
    fn gate_zero_plays_a_blip() {
        let (mut f, mut p) = fx(&[("touchstrip_pos", 0.5), ("stutter_gate", 0.0), ("stutter_quantize", 0.0)]);
        let x: Vec<f32> = (0..96_000).map(|i| (((i * 7919 + 13) % 1000) as f32 / 1000.0) - 0.5).collect();
        run(&mut f, &x[..24_000]);
        set(&mut f, &mut p, "touchstrip_on_off", 1.0);
        let out = run(&mut f, &x[24_000..]);
        let loud = out[24_000..].iter().filter(|v| v.abs() > 1e-3).count();
        let all = out.len() - 24_000;
        assert!(loud > all / 20 && loud < all / 2, "{loud} of {all}");
    }

    /// The Pitch knob is relative to where it was at the touch: set beforehand, it changes
    /// nothing; moved while engaged, it does.
    #[test]
    fn a_pitch_set_before_the_touch_does_nothing() {
        let x: Vec<f32> = (0..96_000).map(|i| (((i * 7919 + 13) % 1000) as f32 / 1000.0) - 0.5).collect();
        let take = |pitch: f64| {
            let (mut f, mut p) = fx(&[("touchstrip_pos", 0.5), ("stutter_quantize", 0.0), ("stutter_pitch", pitch)]);
            run(&mut f, &x[..24_000]);
            set(&mut f, &mut p, "touchstrip_on_off", 1.0);
            run(&mut f, &x[24_000..])
        };
        assert_eq!(take(0.5), take(0.75));
    }
}
