//! `NI::DSP::UTIL::RX::RxDrive` (docs/synth-bass-synth.md): the Bass Synth's drive, a
//! compiled Reaktor-Core kernel (`endInit` 0x1018b2e8c, `processAudio` 0x1018b3020; no
//! fused multiply-adds). The input is split by two one-pole (TPT) filters at 30 Hz and
//! 2800 Hz: the band below 30 Hz and the one above 2800 Hz pass clean, the 30–2800 Hz band
//! is amplified (`10^(amount·range/20)`, the amount gliding over 100 ms) and saturated:
//! `x − x³/3` below |x| = 0.05, `tanh` up to 20, ±1 above. The sum is scaled by the
//! output gain.
//!
//! State (byte offsets as in Maschine; the block sits at BassSynthCore+0x3148):
//!
//! | off | field |
//! |---|---|
//! | 0x00 | sample rate (oversampled) |
//! | 0x08 | input |
//! | 0x0c, 0x10 | drive amount (≥ 0) and its "changed" flag (`f32` 1.0) |
//! | 0x14, 0x18 | gain range in dB (20) and its flag |
//! | 0x1c, 0x20 | output gain and its flag |
//! | 0x24 | output |
//! | 0x28..0x38 | 30 Hz filter: `g`, `−g`, state, `0·g`, `1/(g+1)` |
//! | 0x3c..0x4c | 2800 Hz filter: the same |
//! | 0x50 | amount glide step `1/(0.1·sr)` |
//! | 0x54, 0x58 | glide running (`i32`), position 0..1 |
//! | 0x5c, 0x60, 0x64 | glided amount, glide start, glide distance |
//! | 0x68 | gain range (dB) in use |
//! | 0x6c | gain `10^(amount·range/20)` |
//! | 0x70 | output gain in use |
//! | 0x7c | latched amount |

use super::rx::{Exp2Table, Mem};

/// The kernel's state, laid out as in Maschine.
pub(crate) struct Drive {
    pub(crate) m: Mem<{ 0x90 / 4 }>,
}

/// The kernel's `tan` approximation for the filters' prewarping: a (3,2) rational in
/// `x = min(π·f/sr, 1.50845)`.
fn tan_rat(x: f32) -> f32 {
    let lim = f32::from_bits(0x3fc1_14e4); // 1.50845
    let x = if x > lim { lim } else { x };
    let num = x * (x * (f32::from_bits(0x3d1f_1c25) - x * f32::from_bits(0x3db7_a1a8)) + f32::from_bits(0x3f80_01a3));
    let den = x * (f32::from_bits(0x3d25_9bd1) - x * f32::from_bits(0x3edc_9b20)) + 1.0;
    num / den
}

impl Drive {
    pub(crate) fn new() -> Drive {
        Drive { m: Mem::new() }
    }

    /// `endInit`: filter coefficients, the latched inputs, and the glide reset.
    pub(crate) fn end_init(&mut self, t: &Exp2Table) {
        let _ = t;
        let m = &mut self.m;
        m.set_f(0x30, 0.0);
        m.set_f(0x44, 0.0);
        let sr = m.f(0x0);
        m.set_f(0x50, 1.0 / (sr * f32::from_bits(0x3dcc_cccd)));
        let w = f32::from_bits(0x4049_0fd0) / sr; // 3.14159
        let a = if m.f(0x10) as i32 != 0 {
            let v = m.f(0xc);
            let v = if v < 0.0 { 0.0 } else { v };
            m.set_f(0x7c, v);
            m.set_i(0x54, 1);
            m.set_f(0x58, 0.0);
            v
        } else {
            m.f(0x7c)
        };
        m.set_f(0x5c, a);
        m.set_f(0x60, a);
        #[allow(clippy::eq_op)] // as the kernel does it (NaN stays NaN)
        m.set_f(0x64, a - a);
        for (off, hz) in [(0x28, 30.0f32), (0x3c, 2800.0)] {
            let g = tan_rat(w * hz);
            m.set_f(off, g);
            m.set_f(off + 4, -g);
            m.set_f(off + 0xc, g * 0.0);
            m.set_f(off + 0x10, 1.0 / (g + 1.0));
        }
        if m.f(0x18) as i32 != 0 {
            m.set_f(0x68, m.f(0x14));
        }
        if m.f(0x20) as i32 != 0 {
            m.set_f(0x70, m.f(0x1c));
        }
    }

    /// `processAudio`: one sample, input at 0x08, output at 0x24.
    #[allow(clippy::neg_cmp_op_on_partial_ord)] // the kernel's NaN behaviour
    pub(crate) fn process(&mut self, t: &Exp2Table) {
        const TINY: f32 = f32::from_bits(0x2193_92ef); // 1e-18
        let m = &mut self.m;
        let x = m.f(0x8);
        // 30 Hz: high-pass `h1`, low-pass `lp1`.
        let s = m.f(0x30);
        let h1 = x + m.f(0x38) * (x * m.f(0x2c) - (s + m.f(0x34)));
        let v = m.f(0x28) * h1;
        let lp1 = s + v;
        m.set_f(0x30, v + lp1 + TINY);
        // 2800 Hz on the rest: the band `bp` (driven) and `hi` above it (clean).
        let s = m.f(0x44);
        let hi = h1 + m.f(0x4c) * (h1 * m.f(0x40) - (s + m.f(0x48)));
        let v = m.f(0x3c) * hi;
        let bp = s + v;
        m.set_f(0x44, v + bp + TINY);
        let gain = if m.i(0x54) != 0 {
            let mut p = m.f(0x58) + m.f(0x50);
            if !(p < 1.0) {
                m.set_i(0x54, 0);
                p = 1.0;
            }
            let a = p * m.f(0x64) + m.f(0x60);
            m.set_f(0x5c, a);
            let g = t.exp2(a * m.f(0x68) * f32::from_bits(0x3e2a_1512)); // 0.166096
            m.set_f(0x6c, g);
            m.set_f(0x58, p);
            g
        } else {
            m.f(0x6c)
        };
        let d = bp * gain;
        let a = d.abs();
        let sat = if !(a > 0.05) {
            d + d * (d * d) * f32::from_bits(0xbeaa_aa9f) // −1/3
        } else if a <= 20.0 {
            let e = t.exp2((d + d) * f32::from_bits(0x3fb8_aa65)); // 1.4427
            (e + -1.0) / (e + 1.0)
        } else if d < 0.0 {
            -1.0
        } else {
            1.0
        };
        m.set_f(0x24, (lp1 + hi + sat) * m.f(0x70));
    }
}
