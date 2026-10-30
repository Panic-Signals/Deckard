//! Helpers for the drum cores' NEON code paths.

/// `FRECPE` (single precision): the Arm reciprocal estimate, ~8 bits (Arm ARM
/// `FPRecipEstimate` / `RecipEstimate`, FPCR.FZ off, round to nearest).
pub fn frecpe(x: f32) -> f32 {
    let b = x.to_bits();
    let sign = b & 0x8000_0000;
    let exp = ((b >> 23) & 0xff) as i32;
    if x.is_nan() {
        return f32::from_bits(b | 0x0040_0000);
    }
    if exp == 0xff {
        return f32::from_bits(sign); // ±inf → ±0
    }
    if b & 0x7fff_ffff == 0 {
        return f32::from_bits(sign | 0x7f80_0000);
    }
    // The fraction as the pseudocode's 52-bit field.
    let mut frac: u64 = ((b & 0x007f_ffff) as u64) << 29;
    let mut e = exp;
    if e == 0 {
        if frac & (1 << 51) == 0 {
            e = -1;
            frac = (frac << 2) & ((1 << 52) - 1);
        } else {
            frac = (frac << 1) & ((1 << 52) - 1);
        }
    }
    let scaled = 256 + (frac >> 44) as u32; // '1':fraction<51:44>
    let mut res_exp = 253 - e;
    if !(-1..=0xfe).contains(&res_exp) {
        // |x| < 2^-128 overflows; |x| ≥ 2^126 underflows (not reached here).
        return if res_exp > 0xfe { f32::from_bits(sign | 0x7f80_0000) } else { f32::from_bits(sign) };
    }
    let a = scaled * 2 + 1;
    let r = ((1u32 << 19) / a).div_ceil(2); // (b + 1) / 2
    let mut f: u64 = ((r & 0xff) as u64) << 44;
    if res_exp == 0 {
        f = (1 << 51) | (f >> 1);
    } else if res_exp == -1 {
        f = (1 << 50) | (f >> 2);
        res_exp = 0;
    }
    f32::from_bits(sign | ((res_exp as u32) << 23) | ((f >> 29) as u32))
}

/// `FRECPS`: the Newton step `2 − a·b` (fused).
#[inline]
pub fn frecps(a: f32, b: f32) -> f32 {
    (-a).mul_add(b, 2.0)
}

/// `1/x` as the cores compute it in NEON: an estimate refined by two Newton steps.
#[inline]
pub fn recip(x: f32) -> f32 {
    let e = frecpe(x);
    let e = e * frecps(x, e);
    frecps(x, e) * e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn frecpe_matches_the_instruction() {
        use std::arch::aarch64::{vdupq_n_f32, vgetq_lane_f32, vrecpeq_f32};
        let mut x = 1.0e-6f32;
        while x < 1.0e6 {
            for v in [x, -x, x * 1.37, x * 1.999] {
                let hw = unsafe { vgetq_lane_f32(vrecpeq_f32(vdupq_n_f32(v)), 0) };
                assert_eq!(frecpe(v).to_bits(), hw.to_bits(), "{v}");
            }
            x *= 1.0123;
        }
    }

    #[test]
    fn recip_is_close() {
        for x in [0.5f32, 1.0, 1.7, 3.3, 9.0] {
            assert!((recip(x) * x - 1.0).abs() < 1e-6);
        }
    }
}
