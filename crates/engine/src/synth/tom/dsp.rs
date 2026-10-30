//! Small kernels the Tom engines share.

/// `NI::AB::ExpTable` lookup `E(x) ≈ 2^((x − 1200)/60)` (docs/fx-eq.md §2.1).
pub(super) use crate::fx::exp_e;

/// `E(x)` of a pitch in semitones: `exp_e(st·5 + 1381.8816)` = `440·2^((st−69)/12)` Hz
/// (`fmadd`).
#[inline]
pub(super) fn pitch_hz(st: f32) -> f32 {
    exp_e(st.mul_add(5.0, f32::from_bits(0x44ac_bc36)))
}

/// `E(x)` of a level in dB: `exp_e(db·9.965784 + 602.0529)` (`fmadd`).
#[inline]
pub(super) fn db_ms(db: f32) -> f32 {
    exp_e(db.mul_add(f32::from_bits(0x411f_73da), f32::from_bits(0x4416_8363)))
}

/// `clamp(x, 0, 1)` as Maschine writes it (`fcmp`/`fcsel` on `mi`, then `gt`).
#[inline]
pub(super) fn clamp01(x: f32) -> f32 {
    let a = if x < 0.0 { 0.0 } else { x };
    if a > 1.0 { 1.0 } else { a }
}

/// `frecpe`: the ARMv8 reciprocal estimate (8-bit, `FPRecipEstimate` with FPCR = 0),
/// done in software so it is the same on every machine.
pub(super) fn frecpe(x: f32) -> f32 {
    let b = x.to_bits();
    let sign = b & 0x8000_0000;
    let mut exp = ((b >> 23) & 0xff) as i32;
    let mut frac = b & 0x7f_ffff;
    if x.is_nan() {
        return f32::from_bits(b | 0x40_0000);
    }
    if exp == 0xff {
        return f32::from_bits(sign); // ±inf → ±0
    }
    if exp == 0 && frac == 0 {
        return f32::from_bits(sign | 0x7f80_0000); // ±0 → ±inf
    }
    if x.abs() < f32::from_bits(0x2000_0000) {
        // |x| < 2^-128: the estimate overflows.
        return f32::from_bits(sign | 0x7f80_0000);
    }
    if exp == 0 {
        if frac & 0x40_0000 == 0 {
            exp = -1;
            frac = (frac & 0x1f_ffff) << 2;
        } else {
            frac = (frac & 0x3f_ffff) << 1;
        }
    }
    let scaled = 0x100 | (frac >> 15); // '1':fraction<22:15>, 256..511
    let mut rexp = 253 - exp;
    let a = scaled * 2 + 1;
    let r = ((1u32 << 19) / a).div_ceil(2); // RecipEstimate: 256..511
    let mut rfrac = (r & 0xff) << 15;
    if rexp == 0 {
        rfrac = 0x40_0000 | (rfrac >> 1);
    } else if rexp == -1 {
        rfrac = 0x20_0000 | (rfrac >> 2);
        rexp = 0;
    }
    f32::from_bits(sign | ((rexp as u32 & 0xff) << 23) | rfrac)
}
