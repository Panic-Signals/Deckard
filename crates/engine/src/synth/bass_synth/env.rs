//! `NI::DSP::UTIL::RX::RxEnvAHD`, the Bass Synth's envelope (docs/synth-bass-synth.md):
//! a compiled Reaktor-Core kernel. Every field sits at the byte offset Maschine's kernel
//! uses ([`Mem`]), and each function follows the arm64 code operation by operation (all
//! `f64` except the inputs and the time conversion, no fused multiply-adds), so the
//! output matches bit for bit.
//!
//! It is a three-stage segment generator. The stage table at +0x120 holds 6 doubles per
//! stage (`T[6k + i]`, indices clamped to 17):
//!
//! | i | meaning |
//! |---|---|
//! | 0 | start level (taken when flag bit 0 is set) |
//! | 1 | target level |
//! | 2 | time in seconds |
//! | 3 | curvature (clamped to ±10) |
//! | 4 | flags: bit 0 jump to the start level; bit 1 sustain (the counter stops, the curve's end factor is 0); bit 2 hold (target = the current level) |
//! | 5 | the next stage (3 = none) |
//!
//! Stage 0 is the attack (from the current level to the peak `1 + (velocity − 1)·VelS`,
//! curvature Ax, time A), stage 1 the hold (flags `4 | 2·Glide`: keep the level for H,
//! or for as long as Glide is on), stage 2 the decay (to 0, curvature Dx, time D).
//! Times arrive in "dB" units: `t = 2^((x − 60)·0.166096)` s ≈ `10^((x − 60)/20)` s,
//! capped at 11184.8 s.
//!
//! A segment lasts `n = max(1, round(t·sr))` samples. Each sample outputs the current
//! level `y` (as `f32`), then `y ← y·mul + add + 1e−100`, with `mul = 2^(c/n)` (`c` the
//! curvature; `1 + x·ln 2` when the exponent is below about 2^−26) and `add` chosen so
//! `y` lands on the target after `n` steps (`add = (target − y)/n` for a straight
//! line). After the decay `mul = 1`, `add = 0`: the level stays where the decay ended
//! (0 within rounding; as `f32` it is exactly 0 once below ~1e−45).

// `dur / dur` and `!(a < b)` are the kernel's own operations (NaN must take the same
// branches), so they stay as written.
#![allow(clippy::eq_op, clippy::neg_cmp_op_on_partial_ord)]

use super::rx::{Exp2Table, Mem, POW2_16, POW2_D, rne, rne_d};

/// `s_00000003[0]` and `[1]`, the only entries of that table the kernel reads.
const S3_0: f32 = 0.0;
const S3_1: f32 = f32::from_bits(0x3ab9_1dee);
/// The sustain flag's curvature (`0xc041128220000000`, −34.14…).
const SUSTAIN_CURVE: f64 = f64::from_bits(0xc041_1282_2000_0000);
/// The per-sample offset that keeps `y` off denormals (`0x2b2bff2ee48e0530`).
const TINY: f64 = 1e-100;
/// The polynomial's linear coefficient, 0.693147 (`0x3fe62e429e0a41a2`).
const LN2_POLY: f64 = f64::from_bits(0x3fe6_2e42_9e0a_41a2);
/// Longest segment time (s).
const MAX_TIME: f64 = 11184.8;

type M = Mem<{ 0x1e8 / 4 }>;

/// Byte offset of stage-table entry `i` (clamped to 17, as the kernel does).
#[inline(always)]
fn tab(i: u32) -> usize {
    0x120 + 8 * if i < 17 { i } else { 17 } as usize
}

/// `2^x` in `f64` as the segment code computes it: `2^round(x)` and the nearest
/// sixteenth octave from the tables, times a degree-5 polynomial for the rest
/// (`poly · (POW2_D · POW2_16)`).
#[inline(always)]
fn exp2s(x: f64) -> f64 {
    let n = rne_d(x) as i32;
    let i = (n as u32).wrapping_add(0x80);
    let s0 = POW2_D[if i < 0x100 { i } else { 0x100 } as usize];
    let y = (x - n as f64) * 16.0;
    let m = rne_d(y) as i32;
    let j = (m as u32).wrapping_add(8);
    let s = s0 * POW2_16[if j < 0x10 { j } else { 0x10 } as usize];
    let t = (y - m as f64) * 0.0625;
    let p = t * (t * (t * (t * (t * 0.00133338 + 0.00961835) + 0.0555041) + 0.240227) + LN2_POLY);
    (p + 1.0) * s
}

/// A time input in "dB" units to seconds: `2^((x − 60)·0.166096)`, capped.
#[inline(always)]
fn seconds(t: &Exp2Table, x: f32) -> f64 {
    let v = t.exp2((x + -60.0) * 0.166096) as f64;
    if v > MAX_TIME { MAX_TIME } else { v }
}

/// Curvature clamped to ±10 (NaN passes).
#[inline(always)]
fn clamp10(c: f64) -> f64 {
    let lo = if c < -10.0 { -10.0 } else { c };
    if c > 10.0 { 10.0 } else { lo }
}

/// An input's "changed" flag (truncated to an integer, as the kernel tests it).
#[inline(always)]
fn flag(m: &M, off: usize) -> bool {
    m.f(off) as i32 != 0
}

/// The kernel's state, laid out as in Maschine (0x1e8 bytes). Inputs, each followed by
/// its "changed" flag (read by [`Env::end_init`]): +0x00 sample rate, +0x08 velocity
/// (0..1), +0x10 VelS, +0x18 A, +0x20 Ax, +0x28 gate, +0x30 H, +0x38 D, +0x40 Dx, +0x48
/// gate off, +0x50 Glide, +0x58 decay-flag source, +0x60 attack flags. Output: +0x68.
pub(crate) struct Env {
    pub(crate) m: M,
}

impl Env {
    pub(crate) fn new() -> Env {
        Env { m: Mem::new() }
    }

    /// The output, as last produced by [`Env::process`].
    pub(crate) fn out(&self) -> f32 {
        self.m.f(0x68)
    }

    /// `endInit`: rebuild the stage table from the inputs whose flags are set and park
    /// the envelope at level 0 in the hold stage (the decay stage when +0x4c is set or a
    /// Glide-off is pending).
    pub(crate) fn end_init(&mut self, t: &Exp2Table) {
        let m = &mut self.m;
        if flag(m, 0x14) {
            m.set_f(0x70, m.f(0x10));
        }
        if flag(m, 0x5c) {
            m.set_i(0x1b4, rne(m.f(0x58)));
        }
        let vel = if flag(m, 0x0c) {
            let v = m.f(0x08);
            m.set_f(0x1b0, v);
            v
        } else {
            m.f(0x1b0)
        };
        // ln 2 for the linear approximation of tiny exponents, built from s_00000003.
        let k = rne(-0.5);
        let fr = 0.0 - k as f32;
        let ln2 = ((((1.0 - fr) * S3_0) + fr * S3_1) + 1.0) * f32::from_bits(0x3f31_7215);
        let ln2 = ln2 as f64;
        m.set_d(0xe8, ln2);
        m.set_d(0xd8, SUSTAIN_CURVE);
        m.set_d(0xc0, SUSTAIN_CURVE);
        let decay_src = m.i(0x1b4);
        let v1 = vel + -1.0;
        m.set_f(0x6c, v1);
        let peak = ((v1 * m.f(0x70)) + 1.0) as f64;
        let sr = m.f(0x00) as f64;
        // The exponent below which `mul` is linear: about 2^−26.
        let k = rne(f32::from_bits(0xbeff_8060));
        let fa = f32::from_bits(0x3a7f_4000) - k as f32;
        let x = (((1.0 - fa) * S3_0) + fa * S3_1) + -52.0;
        let thr = t.exp2(x * 0.5) as f64;
        m.set_d(0xb0, sr);
        let inv = 1.0 / sr;
        m.set_d(0x80, sr);
        m.set_d(0x88, inv);
        m.set_d(0xc8, thr);
        m.set_d(0xd0, ln2);
        if flag(m, 0x54) {
            let g = rne(rne(m.f(0x50)) as f32);
            m.set_i(0x1b8, g << 1);
            let old = m.i(0x78);
            m.set_i(0x78, g);
            if old > g {
                // Glide went off: queue the decay start (an event list at +0x1d4).
                let e = m.i(0x1d4);
                m.set_i(0x1dc, e);
                m.set_i(0x1d4, 1);
                m.set_i(0x1d8, -1);
            }
        }
        let decay_flags = ((decay_src << 1) ^ 2) as f32;
        let to_decay = m.f(0x4c);
        m.set_d(0xe0, thr);
        m.set_d(0x120, 0.0);
        let glide2 = m.i(0x1b8);
        m.set_d(0x128, peak);
        if flag(m, 0x1c) {
            let s = seconds(t, m.f(0x18));
            m.set_d(0x130, s);
        }
        if flag(m, 0x24) {
            m.set_d(0x138, m.f(0x20) as f64);
        }
        if flag(m, 0x64) {
            let f = rne(rne(m.f(0x60)) as f32);
            m.set_d(0x140, f as f64);
        }
        let hold_flags = glide2 | 4;
        let to_decay = to_decay as i32;
        m.set_i(0x74, 1);
        m.set_d(0x150, 0.0);
        m.set_d(0x158, 0.0);
        m.set_d(0x148, 1.0);
        if flag(m, 0x34) {
            let s = seconds(t, m.f(0x30));
            m.set_d(0x160, s);
        }
        let decay_flags = rne(decay_flags);
        let pending = m.i(0x1d8);
        m.set_d(0x168, 0.0);
        let mut stage = 1.0;
        if to_decay | pending != 0 {
            m.set_i(0x74, 2);
            stage = 2.0;
        }
        m.set_d(0x170, hold_flags as f64);
        m.set_d(0x180, 0.0);
        m.set_d(0x188, 0.0);
        m.set_d(0x178, 2.0);
        if flag(m, 0x3c) {
            let s = seconds(t, m.f(0x38));
            m.set_d(0x190, s);
        }
        if flag(m, 0x44) {
            m.set_d(0x198, m.f(0x40) as f64);
        }
        m.set_d(0x1a0, decay_flags as f64);
        m.set_d(0x1a8, 3.0);

        // The current stage's segment (computed, then the level is parked at 0 and the
        // counter at 0, so the first sample moves on to the next stage).
        let b = (rne_d(stage) as u32).wrapping_mul(6);
        let start = m.d(tab(b));
        let mut target = m.d(tab(b + 1));
        let n = (rne_d(m.d(tab(b + 2)) * sr) as i32).max(1);
        let curv = m.d(tab(b + 3));
        let dn = n as f64;
        let dur = inv * dn;
        m.set_d(0x90, dur);
        m.set_d(0x98, dur);
        let dc = rne_d((dur / dur) * dn) as i32 as f64;
        m.set_i(0xa0, n);
        let rem = inv * dc;
        let curv = clamp10(curv);
        let fl = rne_d(m.d(tab(b + 4))) as i32 as u32;
        let hold = rne_d(((fl >> 2) & 1) as f64) as i32;
        let curv = if fl & 2 == 0 { curv } else { SUSTAIN_CURVE };
        let k = curv / dur;
        let x = inv * k;
        let tot = k * rem;
        let (mulv, m1) = if !(x.abs() < thr) {
            let e = exp2s(x);
            (e, e + -1.0)
        } else {
            let a = x * ln2;
            (a + 1.0, a)
        };
        m.set_d(0xf0, mulv);
        let y = if fl & 1 != 0 {
            m.set_d(0xa8, start);
            start
        } else {
            m.d(0xa8)
        };
        if hold != 0 {
            target = y;
        }
        let add = if !(tot.abs() < thr) {
            let e = exp2s(tot);
            let e = if fl & 2 == 0 { e } else { 0.0 };
            let a = y * e;
            let r = 1.0 / (e + -1.0);
            m.set_d(0x108, r);
            m.set_d(0x110, a);
            (m1 * r) * (target - a)
        } else {
            if fl & 2 != 0 {
                m.set_d(0x110, y * 0.0);
                m.set_d(0x108, -1.0);
            }
            (target - y) * (1.0 / dc)
        };
        m.set_d(0x118, add);
        m.set_d(0xa8, 0.0);
        m.set_i(0xb8, 0);
        m.set_i(0xa4, 0);
        // Clear the pending-event list.
        let mut e = m.i(0x1d4) as u32;
        while e != 0 {
            m.set_i(0x1d4 + 4 * e as usize, 0);
            e = m.i(0x1d4 + 4 * (e + 1) as usize) as u32;
        }
        m.set_i(0x1d4, 0);
    }

    /// One sample: outputs the current level, then steps it (and moves on to the next
    /// stage when the segment's counter has run out).
    pub(crate) fn process(&mut self, t: &Exp2Table) {
        let _ = t;
        let m = &mut self.m;
        let cnt = m.i(0xa4);
        let (n, dec, mulv, add);
        if cnt >= 1 {
            (n, dec, mulv, add) = (cnt, m.i(0xb8), m.d(0xf0), m.d(0x118));
        } else {
            let six = rne_d(6.0) as i32 as u32;
            let stage = m.i(0x74) as u32;
            let next = rne_d(m.d(tab(stage.wrapping_mul(six).wrapping_add(5)))) as i32;
            let b = (next as u32).wrapping_mul(six);
            let start = m.d(tab(b));
            let target = m.d(tab(b.wrapping_add(1)));
            let nn = (rne_d(m.d(tab(b.wrapping_add(2))) * m.d(0xb0)) as i32).max(1);
            let curv = m.d(tab(b.wrapping_add(3)));
            let dn = nn as f64;
            let inv = m.d(0x88);
            let dur = inv * dn;
            m.set_d(0x98, dur);
            let c = rne_d((dur / dur) * dn) as i32;
            m.set_i(0xa4, c);
            let dc = c as f64;
            let rem = inv * dc;
            let curv = clamp10(curv);
            let fl = rne_d(m.d(tab(b.wrapping_add(4)))) as i32 as u32;
            let sus = (fl >> 1) & 1;
            let mut d = sus ^ 1;
            m.set_i(0xb8, d as i32);
            let curv = if sus != 0 { m.d(0xc0) } else { curv };
            let k = curv / dur;
            let x = inv * k;
            let tot = rem * k;
            let thr = m.d(0xc8);
            let (mut mv, m1) = if !(x.abs() < thr) {
                let e = exp2s(x);
                (e, e + -1.0)
            } else {
                let a = x * m.d(0xd0);
                (a + 1.0, a)
            };
            m.set_d(0xf0, mv);
            let held = m.d(0xa8);
            if next >= 3 {
                // No stage after this one: stay where the level is.
                d = 0;
                m.set_i(0xb8, 0);
                m.set_d(0xf0, 1.0);
                mv = 1.0;
            } else {
                m.set_i(0x74, next);
            }
            let big = !(tot.abs() < thr);
            let e = if big { exp2s(tot) } else { tot };
            let e = if sus == 0 { e } else { 0.0 };
            let wrote = big || sus != 0;
            let r = if wrote {
                let r = 1.0 / (e + -1.0);
                m.set_d(0xf8, r);
                r
            } else {
                m.d(0xf8)
            };
            let hold = rne_d(((fl >> 2) & 1) as f64) as i32;
            let y = if fl & 1 == 0 { m.d(0xa8) } else { start };
            if wrote {
                m.set_d(0x100, e * y);
            }
            let tgt = if hold == 0 { target } else { y };
            let (s, base) = if big { (m1 * r, m.d(0x100)) } else { (1.0 / dc, y) };
            let mut a = s * (tgt - base);
            m.set_d(0x118, if next > 2 { 0.0 } else { a });
            if fl & 1 != 0 {
                m.set_d(0xa8, y);
            }
            if next >= 3 {
                m.set_d(0xa8, held);
                a = 0.0;
            }
            (n, dec, mulv, add) = (nn, d as i32, mv, a);
        }
        let y = m.d(0xa8);
        m.set_d(0xa8, y * mulv + add + TINY);
        m.set_f(0x68, y as f32);
        m.set_i(0xa4, n - dec);
    }

    /// Re-times stage `k`'s running segment after one of its inputs changed, keeping
    /// the share of it already done (the A/H/D/Ax/Dx/VelS/NoteVel events).
    fn resegment(&mut self, k: f64) {
        let m = &mut self.m;
        let b = (rne_d(k) as u32).wrapping_mul(6);
        let mut target = m.d(tab(b + 1));
        let time = m.d(tab(b + 2));
        let (sr, inv) = (m.d(0x80), m.d(0x88));
        let dn = (rne_d(time * sr) as i32).max(1) as f64;
        let curv = m.d(tab(b + 3));
        let dur = inv * dn;
        let curv = clamp10(curv);
        let fl = rne_d(m.d(tab(b + 4))) as i32 as u32;
        let hold = rne_d(((fl >> 2) & 1) as f64) as i32;
        let sus = (fl >> 1) & 1;
        let (curv, w) = if sus != 0 { (m.d(0xd8), true) } else { (curv, false) };
        m.set_i(0xb8, (sus ^ 1) as i32);
        let y = m.d(0xa8);
        let old = m.d(0x98);
        m.set_d(0x98, dur);
        let k = curv / dur;
        let x = k * inv;
        let c = rne_d((dur / old) * m.i(0xa4) as f64) as i32;
        let dc = c as f64;
        m.set_i(0xa4, c);
        let rem = inv * dc;
        let tot = k * rem;
        if hold != 0 {
            target = y;
        }
        let thr = m.d(0xe0);
        let m1 = if !(x.abs() < thr) {
            let e = exp2s(x);
            m.set_d(0xf0, e);
            e + -1.0
        } else {
            let a = x * m.d(0xe8);
            m.set_d(0xf0, a + 1.0);
            a
        };
        let big = !(tot.abs() < thr);
        let e = if big { exp2s(tot) } else { target - y };
        if big || w {
            let e = if sus == 0 { e } else { 0.0 };
            m.set_d(0x110, y * e);
            m.set_d(0x108, 1.0 / (e + -1.0));
        }
        let add = if big { (m1 * m.d(0x108)) * (target - m.d(0x110)) } else { (target - y) * (1.0 / dc) };
        m.set_d(0x118, add);
    }

    /// Starts stage `k`'s segment afresh (note on: the attack; note off and Glide
    /// off: the decay), from the current level.
    fn start(&mut self, k: i32) {
        let m = &mut self.m;
        m.set_i(0x74, k);
        let b = (rne_d(k as f64) as u32).wrapping_mul(6);
        let start = m.d(tab(b));
        let mut target = m.d(tab(b + 1));
        let time = m.d(tab(b + 2));
        let (sr, inv) = (m.d(0x80), m.d(0x88));
        let nn = (rne_d(time * sr) as i32).max(1);
        let curv = m.d(tab(b + 3));
        let dn = nn as f64;
        let dur = inv * dn;
        m.set_d(0x90, dur);
        m.set_d(0x98, dur);
        let c = rne_d((dur / dur) * dn) as i32;
        m.set_i(0xa0, nn);
        m.set_i(0xa4, c);
        let dc = c as f64;
        let curv = clamp10(curv);
        let fl = rne_d(m.d(tab(b + 4))) as i32 as u32;
        let hold = rne_d(((fl >> 2) & 1) as f64) as i32;
        let sus = (fl >> 1) & 1;
        let y = if fl & 1 != 0 {
            m.set_d(0xa8, start);
            start
        } else {
            m.d(0xa8)
        };
        let rem = inv * dc;
        if hold != 0 {
            target = y;
        }
        let (curv, w) = if sus != 0 { (m.d(0xd8), true) } else { (curv, false) };
        let k = curv / dur;
        let x = inv * k;
        let tot = rem * k;
        let thr = m.d(0xe0);
        let m1 = if !(x.abs() < thr) {
            let e = exp2s(x);
            m.set_d(0xf0, e);
            e + -1.0
        } else {
            let a = x * m.d(0xe8);
            m.set_d(0xf0, a + 1.0);
            a
        };
        let big = !(tot.abs() < thr);
        let e = if big { exp2s(tot) } else { tot };
        if big || w {
            let e = if sus == 0 { e } else { 0.0 };
            m.set_d(0x110, y * e);
            m.set_d(0x108, 1.0 / (e + -1.0));
        }
        let add = if big { (m1 * m.d(0x108)) * (target - m.d(0x110)) } else { (target - y) * (1.0 / dc) };
        m.set_d(0x118, add);
        m.set_i(0xb8, (sus ^ 1) as i32);
    }

    /// Attack time (input +0x18).
    pub(crate) fn ev_a(&mut self, t: &Exp2Table) {
        let s = seconds(t, self.m.f(0x18));
        self.m.set_d(0x130, s);
        if self.m.i(0x74) == 0 {
            self.resegment(0.0);
        }
    }

    /// Hold time (input +0x30).
    pub(crate) fn ev_h(&mut self, t: &Exp2Table) {
        let s = seconds(t, self.m.f(0x30));
        self.m.set_d(0x160, s);
        if self.m.i(0x74) == 1 {
            self.resegment(1.0);
        }
    }

    /// Decay time (input +0x38).
    pub(crate) fn ev_d(&mut self, t: &Exp2Table) {
        let s = seconds(t, self.m.f(0x38));
        self.m.set_d(0x190, s);
        if self.m.i(0x74) == 2 {
            self.resegment(2.0);
        }
    }

    /// Attack curvature (input +0x20).
    pub(crate) fn ev_ax(&mut self) {
        self.m.set_d(0x138, self.m.f(0x20) as f64);
        if self.m.i(0x74) == 0 {
            self.resegment(0.0);
        }
    }

    /// Decay curvature (input +0x40).
    pub(crate) fn ev_dx(&mut self) {
        self.m.set_d(0x198, self.m.f(0x40) as f64);
        if self.m.i(0x74) == 2 {
            self.resegment(2.0);
        }
    }

    /// Velocity sensitivity (input +0x10): peak = 1 + (velocity − 1)·VelS.
    pub(crate) fn ev_vel_s(&mut self) {
        let v = self.m.f(0x10);
        self.m.set_f(0x70, v);
        let p = (v * self.m.f(0x6c)) + 1.0;
        self.m.set_d(0x128, p as f64);
        if self.m.i(0x74) == 0 {
            self.resegment(0.0);
        }
    }

    /// Velocity (input +0x08, 0..1).
    pub(crate) fn ev_note_vel(&mut self) {
        let v1 = self.m.f(0x08) + -1.0;
        self.m.set_f(0x6c, v1);
        let p = (v1 * self.m.f(0x70)) + 1.0;
        self.m.set_d(0x128, p as f64);
        if self.m.i(0x74) == 0 {
            self.resegment(0.0);
        }
    }

    /// Glide (input +0x50, 0/1): the hold stage sustains while it is on; switching it
    /// off starts the decay.
    pub(crate) fn ev_glide(&mut self) {
        let g = rne(rne(self.m.f(0x50)) as f32);
        let old = self.m.i(0x78);
        self.m.set_i(0x78, g);
        self.m.set_d(0x170, ((g << 1) | 4) as f64);
        if old > g {
            self.start(2);
        }
    }

    /// Note on: restart the attack from the current level.
    pub(crate) fn ev_note_on(&mut self) {
        self.start(0);
    }

    /// Note off: start the decay from the current level.
    pub(crate) fn ev_note_off(&mut self) {
        self.start(2);
    }
}
