//! `NI::DSP::UTIL::RX::RxOsc4Wave` (docs/synth-bass-synth.md §3.2), a compiled
//! Reaktor-Core oscillator with four waveforms (sine, triangle, saw, square) that the Shape
//! input morphs between. One phase accumulator drives all four; the triangle, saw and square
//! are made band-limited by polynomial step/slope corrections (polyBLEP/polyBLAMP-like
//! residuals spread over two samples) started at each discontinuity. The code follows the
//! arm64 assembly operation by operation (`f32`, `f64` where the kernel widens, no fused
//! multiply-adds).
//!
//! State offsets (bytes):
//! - inputs: `0x00` sample rate, `0x08` Shape (0..1) with its event flag `0x0c`, `0x10`
//!   pitch (MIDI note), `0x14` octave with its event flag `0x18`;
//! - output: `0x1c`;
//! - `0x20..0x34`: the 100 ms Shape glide (step, running flag, position, value, start,
//!   span); `0x44` phase (−0.5..0.5), `0x50` last frequency, `0x54` 1/sr;
//! - `0x5c..0x184`: correction slots, each `(count, fraction, amplitude)`: a slot runs
//!   for the two samples after its count is set to −2.

use super::rx::{Exp2Table, Mem, rne};

/// The kernel's state, laid out as in Maschine.
pub(crate) struct Osc {
    pub(crate) m: Mem<{ 0x234 / 4 }>,
}

/// 1/12 as the kernel stores it (`0x3daaaaa6`).
const TWELFTH: f32 = f32::from_bits(0x3daa_aaa6);
/// MIDI note 0 in Hz (`0x4102d014`).
const C0_HZ: f32 = f32::from_bits(0x4102_d014);

/// A step correction (for the saw and square): the residual at `fraction + count`
/// samples after the step, a degree-7 polynomial, odd about the step, times `amp`.
/// Runs while the slot's count is ≤ 1, counting it up.
fn step_slot(m: &mut Mem<{ 0x234 / 4 }>, cnt: usize, fr: usize, amp: usize) -> f32 {
    let n = m.i(cnt);
    if n > 1 {
        return 0.0;
    }
    let x = ((m.f(fr) + n as f32) * -0.5).abs();
    let mut p = x * f32::from_bits(0xc00b_86ec); // -2.18011
    for c in [0x40d8_0f91u32, 0xc0b8_346e, 0xbfd0_0f67, 0x407a_ba9d, 0xbdf5_0b0f, 0xbfbe_7137] {
        p = x * (p + f32::from_bits(c));
    }
    let r = p + f32::from_bits(0x3eff_f6d3); // 0.49993
    m.set_i(cnt, n + 1);
    m.f(amp) * if n < 0 { -r } else { r }
}

/// A slope correction (for the triangle's corners): a degree-9 polynomial of the same
/// distance, times `amp`.
fn slope_slot(m: &mut Mem<{ 0x234 / 4 }>, cnt: usize, fr: usize, amp: usize) -> f32 {
    let n = m.i(cnt);
    if n > 1 {
        return 0.0;
    }
    let x = ((m.f(fr) + n as f32) * -0.5).abs();
    let mut p = x * f32::from_bits(0xc00b_86ec); // -2.18011
    for c in [0x40f6_ed3eu32, 0xc0f5_9b3d, 0xc026_72b0, 0x40fa_bab2, 0xbea3_5cc0, 0xc0be_7137] {
        p = x * (p + f32::from_bits(c));
    }
    let p = p + f32::from_bits(0x407f_f6d3); // 3.99944
    m.set_i(cnt, n + 1);
    m.f(amp) * (x * p + f32::from_bits(0xbf53_21e6)) // -0.824736
}

/// Starts a correction slot: count −2, fraction and amplitude.
fn arm(m: &mut Mem<{ 0x234 / 4 }>, cnt: usize, fr: usize, amp: usize, f: f32, a: f32) {
    m.set_i(cnt, -2);
    m.set_f(fr, f);
    m.set_f(amp, a);
}

/// Toggles the flag at `off` (it alternates between a slot pair) and returns its old
/// value.
fn flip(m: &mut Mem<{ 0x234 / 4 }>, off: usize) -> i32 {
    let w = m.i(off);
    m.set_i(off, w ^ 1);
    w
}

/// `2^x · 8.1758` (Hz from octaves above note 0), interpolating as the kernel does.
fn note_hz(t: &Exp2Table, oct: f32) -> f32 {
    t.exp2(oct) * C0_HZ
}

impl Osc {
    pub(crate) fn new() -> Osc {
        Osc { m: Mem::new() }
    }

    /// `endInit`: derived constants, takes pending Shape/octave events, and resets the
    /// phase, the frequency memory and the square's correction slots.
    pub(crate) fn end_init(&mut self, t: &Exp2Table) {
        let m = &mut self.m;
        let sr = m.f(0x00);
        m.set_f(0x20, 1.0 / (sr * f32::from_bits(0x3dcc_cccd))); // Shape glide: 0.1 s
        m.set_f(0x54, 1.0 / sr);
        if m.f(0x0c) as i32 != 0 {
            m.set_f(0x190, m.f(0x08));
            m.set_i(0x24, 1);
            m.set_i(0x28, 0);
        }
        let oct = if m.f(0x18) as i32 != 0 {
            let v = m.f(0x14);
            m.set_f(0x4c, v);
            v
        } else {
            m.f(0x4c)
        };
        m.set_i(0xd4, 0);
        let hz = note_hz(t, (m.f(0x10) + oct * 12.0) * TWELFTH);
        let s = m.f(0x190);
        m.set_f(0x2c, s);
        m.set_f(0x30, s);
        #[allow(clippy::eq_op)] // the kernel's own `s − s`
        m.set_f(0x34, s - s);
        m.set_f(0x50, hz);
        m.set_i(0x40, 0);
        m.set_i(0x44, 0);
        m.set_f(0x138, -0.5);
        let c = m.f(0x1b4);
        let v =
            m.f(0x1b0) * (c * ((m.f(0x1b8) + f32::from_bits(0x4022_4841)) * c + f32::from_bits(0xc0a5_540d)) + f32::from_bits(0x4049_0fd0));
        m.set_f(0xa0, v);
        m.set_i(0xe0, 1);
        for (dst, src) in [(0x58, 0x1a0), (0xac, 0x1c4), (0xfc, 0x1cc), (0x118, 0x1d0), (0x7c, 0x1a8)] {
            m.set_i(dst, m.i(src) ^ 1);
        }
        m.set_f(0xd8, m.f(0x1c8));
        m.set_f(0x13c, m.f(0x1d4));
        let p = m.f(0x1d8);
        let d = -0.5 - p;
        m.set_f(0x154, d);
        let w8;
        if p == -0.5 {
            w8 = m.i(0x1e8);
        } else {
            let a = -d;
            m.set_i(0x140, -2);
            let w = m.i(0x144);
            m.set_i(0x1e8, w);
            if w != 0 {
                let w9 = m.i(0x168);
                m.set_i(0x200, w9);
                if w9 != 0 {
                    arm(m, 0x178, 0x17c, 0x180, 1.0, a);
                } else {
                    arm(m, 0x16c, 0x170, 0x174, 1.0, a);
                }
                w8 = w;
            } else {
                let w9 = m.i(0x148);
                m.set_i(0x1ec, w9);
                if w9 != 0 {
                    arm(m, 0x15c, 0x160, 0x164, 1.0, a);
                } else {
                    arm(m, 0x14c, 0x150, 0x158, 1.0, a);
                }
                w8 = 0;
            }
        }
        m.set_i(0x148, m.i(0x1ec) ^ 1);
        m.set_i(0x168, m.i(0x200) ^ 1);
        m.set_i(0x144, w8 ^ 1);
    }

    /// One sample.
    pub(crate) fn process(&mut self, t: &Exp2Table) {
        let m = &mut self.m;
        let hz = note_hz(t, (m.f(0x10) + m.f(0x4c) * 12.0) * TWELFTH);

        // Sine from the phase, a quarter period back: a folded triangle through an odd
        // polynomial. The kernel outputs last sample's values (one sample of latency).
        let ph = m.f(0x44);
        let tri_prev = m.f(0x48);
        let s0 = ph + -0.25;
        let s0 = s0 - rne(s0) as f32;
        let s0 = 0.5 - (s0 + s0).abs();
        let s1 = s0 * s0;
        let mut q = s1 * f32::from_bits(0xbf0a_542e) + f32::from_bits(0x4022_4841);
        q = s1 * q + f32::from_bits(0xc0a5_540d);
        let sine = s0 * (s1 * q + f32::from_bits(0x4049_0fd0));
        let sine_prev = m.f(0xa0);
        m.set_f(0xa0, sine);
        m.set_f(0x48, ph.abs() + ph.abs() + -0.5);
        let saw_prev = m.f(0xc8);
        m.set_f(0xc8, ph);

        // Shape glide and the region it falls in: 0..⅓ sine→triangle, ⅓..⅔
        // triangle→saw, ⅔..1 saw→square (each crossfade at 3× the shape).
        if m.i(0x24) != 0 {
            let mut pos = m.f(0x28) + m.f(0x20);
            if pos >= 1.0 {
                m.set_i(0x24, 0);
                pos = 1.0;
            }
            let s = pos * m.f(0x34) + m.f(0x30);
            m.set_f(0x28, pos);
            m.set_f(0x2c, s);
            let (w8, a) = if s <= f32::from_bits(0x3eaa_a64c) {
                m.set_f(0x3c, s * 3.0);
                (-1, 1.0f32)
            } else {
                let sq = if s <= f32::from_bits(0x3f2a_a64c) {
                    m.set_f(0xa8, (s + f32::from_bits(0xbeaa_a64c)) * 3.0);
                    0.0f32
                } else {
                    let w = m.i(0x224);
                    m.set_i(0x22c, w);
                    m.set_i(0x224, 1);
                    m.set_i(0x228, -1);
                    m.set_f(0xd0, (s + f32::from_bits(0xbf2a_a64c)) * 3.0);
                    1.0
                };
                m.set_i(0xcc, rne(sq));
                (0, 0.0)
            };
            let w8 = w8 | m.i(0x228);
            m.set_i(0x38, rne(a));
            m.set_i(0xa4, rne(if w8 == 0 { 1.0 } else { 0.0 }));
        }

        // Phase: the increment is the mean of this and the last frequency over sr,
        // clamped to ±½; the phase runs downwards (by −inc) in −0.5..0.5, in f64.
        let hz_prev = m.f(0x50);
        let isr = m.f(0x54);
        m.set_f(0x50, hz);
        let s7 = (hz + hz_prev) * 0.5 * isr;
        let s5c = if s7 < -0.5 { -0.5 } else { s7 };
        let inc = if s7 > 0.5 { 0.5 } else { s5c };
        let ninc = -inc;
        let d17 = ph as f64 + ninc as f64;
        let dist = d17.abs() as f32;
        let dir: f32 = if inc < 0.0 { 1.0 } else { -1.0 };
        let mut s7 = -dir;
        let wrapped = dist > 0.5;
        let nph;
        if wrapped {
            // Wrapped: the fraction of a sample since the wrap starts the saw's step
            // corrections, the square's, and the triangle's.
            let a = -1.0 / inc;
            nph = (d17 + s7 as f64) as f32;
            let fr = a * (dir * 0.5 + nph);
            let w8 = flip(m, 0x58);
            let w10 = flip(m, 0xe0);
            let w9 = flip(m, 0xac);
            if w10 != 0 {
                arm(m, 0xf0, 0xf4, 0xf8, fr, dir);
            } else {
                arm(m, 0xe4, 0xe8, 0xec, fr, dir);
            }
            if w9 != 0 {
                arm(m, 0xbc, 0xc0, 0xc4, fr, dir);
            } else {
                arm(m, 0xb0, 0xb4, 0xb8, fr, dir);
            }
            if w8 != 0 {
                arm(m, 0x6c, 0x70, 0x74, fr, ninc);
                m.set_f(0x78, dir);
            } else {
                arm(m, 0x5c, 0x60, 0x64, fr, ninc);
                m.set_f(0x68, dir);
            }
        } else {
            nph = d17 as f32;
        }
        m.set_f(0x44, nph);

        // The saw's step corrections.
        let saw_a = step_slot(m, 0xb0, 0xb4, 0xb8);
        let s20 = ph * dir;
        let s21 = dir * nph;
        let saw_b = step_slot(m, 0xbc, 0xc0, 0xc4);
        // The triangle's corner at phase 0 (the sign of phase·dir changes).
        if ((s20.to_bits() & !s21.to_bits()) as i32) < 0 {
            let fr = ((1.0 / ninc as f64) * nph as f64) as f32;
            if flip(m, 0x7c) != 0 {
                arm(m, 0x90, 0x94, 0x98, fr, inc);
                m.set_f(0x9c, dir);
            } else {
                arm(m, 0x80, 0x84, 0x88, fr, inc);
                m.set_f(0x8c, dir);
            }
        }
        let c6c = slope_slot(m, 0x6c, 0x70, 0x74);
        let c5c = slope_slot(m, 0x5c, 0x60, 0x64);
        let c90 = slope_slot(m, 0x90, 0x94, 0x98);
        let c80 = slope_slot(m, 0x80, 0x84, 0x88);

        // The square: edges at the wrap and at the pulse-width point `0xd4`.
        let pw = m.f(0xd4);
        let pw_prev = m.f(0xd8);
        m.set_f(0xd8, pw);
        let rate = 1.0 / ((pw_prev - pw) - inc);
        let mut tri = c5c * m.f(0x68);
        let t2 = c6c * m.f(0x78);
        let t3 = c80 * m.f(0x8c);
        let t4 = c90 * m.f(0x9c);
        let level;
        let mut s16 = 0.0f32;
        let mut s17 = nph;
        if wrapped {
            let s22 = dir * (ph - pw_prev);
            if s22 < 0.0 {
                let f = 1.0 - (pw_prev - ph) * rate;
                if flip(m, 0xfc) != 0 {
                    arm(m, 0x10c, 0x110, 0x114, f, s7);
                } else {
                    arm(m, 0x100, 0x104, 0x108, f, s7);
                }
            }
            let x = nph - pw;
            let s6 = dir * x;
            if s6 > 0.0 {
                let f = x * rate;
                if flip(m, 0x118) != 0 {
                    arm(m, 0x128, 0x12c, 0x130, f, s7);
                } else {
                    arm(m, 0x11c, 0x120, 0x124, f, s7);
                }
            }
            m.set_i(0xdc, -2);
            let l = m.f(0x138);
            let s23 = if s22 < 0.0 { -l } else { l };
            m.set_f(0x138, -s23);
            level = if s6 > 0.0 {
                m.set_f(0x138, s23);
                s23
            } else {
                -s23
            };
        } else {
            let x = nph - pw;
            let l = m.f(0x138);
            s16 = x * l;
            if s16 < 0.0 {
                let nl = -l;
                s17 = nl - l;
                m.set_f(0x138, nl);
                s7 = x * rate;
                level = nl;
            } else {
                level = l;
            }
        }
        tri += t2;
        let t34 = t3 + t4;
        let w_dc = m.i(0xdc);
        let sq_prev2 = m.f(0x18c);
        m.set_f(0x18c, m.f(0x13c));
        m.set_f(0x13c, level);
        let mut w8 = 0;
        if w_dc <= 2 {
            m.set_i(0xdc, w_dc + 1);
            let a = step_slot(m, 0x10c, 0x110, 0x114);
            let b = step_slot(m, 0x100, 0x104, 0x108);
            let c = step_slot(m, 0xf0, 0xf4, 0xf8);
            let d = step_slot(m, 0xe4, 0xe8, 0xec);
            let e = step_slot(m, 0x11c, 0x120, 0x124);
            let f = step_slot(m, 0x128, 0x12c, 0x130);
            m.set_f(0x134, ((a + b) + (c + d)) + (e + f));
            w8 = -1;
        }
        let tri = tri + t34;

        // The square's falling-edge corrections when the level flips mid-cycle.
        let mut w9 = m.i(0x140);
        let mut run = w9 <= 2;
        if !wrapped && s16 < 0.0 {
            let a = -s17;
            if flip(m, 0x144) != 0 {
                if flip(m, 0x168) != 0 {
                    arm(m, 0x178, 0x17c, 0x180, s7, a);
                } else {
                    arm(m, 0x16c, 0x170, 0x174, s7, a);
                }
            } else if flip(m, 0x148) != 0 {
                arm(m, 0x15c, 0x160, 0x164, s7, a);
            } else {
                arm(m, 0x14c, 0x150, 0x158, s7, a);
            }
            w9 = -2;
            run = true;
        }
        if run {
            m.set_i(0x140, w9 + 1);
            let a = step_slot(m, 0x14c, 0x150, 0x158);
            let b = step_slot(m, 0x15c, 0x160, 0x164);
            let c = step_slot(m, 0x16c, 0x170, 0x174);
            let d = step_slot(m, 0x178, 0x17c, 0x180);
            m.set_f(0x184, (a + b) + (c + d));
            w8 = -1;
        }
        let saw_corr = saw_a + saw_b;
        let tri_val = tri_prev + tri;
        let sq = if w8 != 0 {
            let v = m.f(0x134) + m.f(0x184);
            m.set_f(0x188, v);
            v
        } else {
            m.f(0x188)
        };
        let saw = (saw_prev + saw_corr) + (saw_prev + saw_corr);
        let tri2 = tri_val + tri_val;

        // Crossfades of the active region.
        let mut out = sine_prev;
        let mut any = false;
        if m.i(0x38) >= 1 {
            out = sine_prev + (tri2 - sine_prev) * m.f(0x3c);
            any = true;
        }
        if m.i(0xa4) >= 1 {
            out = tri2 + (saw - tri2) * m.f(0xa8);
            any = true;
        }
        if m.i(0xcc) >= 1 {
            let s = sq_prev2 + sq;
            out = saw + ((s + s) - saw) * m.f(0xd0);
            any = true;
        }
        if any {
            m.set_f(0x1c, out);
        }

        // Clears the event list the Shape region change used.
        let mut w = m.i(0x224) as u32;
        while w != 0 {
            let k = 0x224 / 4 + w as usize;
            if k + 1 >= m.0.len() {
                break;
            }
            m.0[k] = 0;
            w = m.0[k + 1];
        }
        m.set_i(0x224, 0);
    }
}
