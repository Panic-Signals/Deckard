//! `NI::AB::SampleRateConverter` from twice the rate down (integer ratio 2, quality 2),
//! as ported for the Distortion (docs/fx-distortion.md §5): one 16-tap windowed-sinc
//! phase and an 8-sample delay, `y = (d + fir)·½`.

/// The converter's 16-tap windowed-sinc phase at fractional position `t` (`init_intrat`):
/// `h[i] = sinc(t + i − 8)·I0(8·√(1 − ((t + i − 8)/8)²)) / 427.564…` (Abramowitz–Stegun I0),
/// in `f64` like the original.
fn src_phase(t: f32) -> [f32; 16] {
    let mut h = [0f32; 16];
    let pi = f64::from_bits(0x4009_21fb_5444_2d11);
    for (i, o) in h.iter_mut().enumerate() {
        let x = t + (i as i32 - 8) as f32;
        if x == 0.0 {
            *o = 1.0;
            continue;
        }
        let s = (x as f64 * pi).sin();
        let u = ((x + x) * 0.0625) as f64;
        let b = (-u).mul_add(u, 1.0).sqrt() * 8.0;
        let i0 = if b >= 3.75 {
            let y = 3.75 / b;
            let mut p = y.mul_add(f64::from_bits(0x3f70_125e_fd3b_7f88), f64::from_bits(0xbf90_df2b_ca08_4c55));
            for k in [
                0x3f9a_fce6_f0e5_5ae3u64,
                0xbf95_1227_1efd_c716,
                0x3f82_c3f3_8a50_d154,
                0xbf59_d0c1_4e13_894a,
                0x3f62_7548_2b92_d5db,
                0x3f8b_35a5_ff2d_9d1c,
                0x3fd9_8845_3365_de00,
            ] {
                p = y.mul_add(p, f64::from_bits(k));
            }
            b.exp() / b.sqrt() * p
        } else {
            let y = b / 3.75;
            let y = y * y;
            let mut p = y.mul_add(f64::from_bits(0x3f72_c3d7_5ac5_4874), f64::from_bits(0x3fa2_78a8_4988_e5fd));
            for k in [0x3fd1_05b4_74e7_32aau64, 0x3ff3_4ed8_3fc7_962c, 0x4008_b833_b894_30a4, 0x400c_1ffe_e624_8ca7] {
                p = y.mul_add(p, f64::from_bits(k));
            }
            y.mul_add(p, 1.0)
        };
        *o = ((s / (x as f64 * pi)) * i0 / f64::from_bits(0x407a_b906_bec5_d25f)) as f32;
    }
    h
}

/// Halves the rate, one channel: every other input is pushed into an 8-deep delay and
/// emits `(d + fir)·½`, the others feed the FIR at phase ½.
pub(super) struct Down {
    h: [f32; 16],
    acc: [f32; 16],
    delay: [f32; 8],
    di: usize,
    sum: f32,
    odd: bool,
}

impl Down {
    pub(super) fn new() -> Down {
        Down { h: src_phase(0.5), acc: [0.0; 16], delay: [0.0; 8], di: 0, sum: 0.0, odd: true }
    }

    /// `resetAllChannels`.
    pub(super) fn clear(&mut self) {
        (self.acc, self.delay, self.di, self.sum, self.odd) = ([0.0; 16], [0.0; 8], 0, 0.0, true);
    }

    /// One input; an output every second one (the first, third, …).
    #[inline]
    pub(super) fn push(&mut self, x: f32) -> Option<f32> {
        if self.odd {
            let d = self.delay[self.di];
            self.delay[self.di] = x;
            self.di = (self.di + 1) & 7;
            let y = (d + self.sum) * 0.5;
            self.sum = 0.0;
            self.odd = false;
            Some(y)
        } else {
            // A transposed 16-tap FIR, each product and sum rounded on its own.
            let out = self.acc[0] + x * self.h[0];
            for i in 0..15 {
                self.acc[i] = self.acc[i + 1] + x * self.h[i + 1];
            }
            self.acc[15] = 0.0;
            self.sum += out;
            self.odd = true;
            None
        }
    }
}
