//! Maschine's Cabinet (docs/fx-cabinet.md): `DATA::CabinetEmulation` →
//! `RT::DSPCoreWrapper<DSP::Cabinet>`. The input is convolved with one of 67 recorded
//! guitar-cabinet impulse responses (2048 samples at 44.1 kHz, picked by Cabinet and
//! Microphone), then `DSP::Air` adds band-passed early reflections (Distance), and Mix
//! blends the result (×0.25) with the dry input. IR changes crossfade over ≈ 20 ms.
//!
//! The impulse responses are Native Instruments' recordings, compiled into Maschine's
//! binary as `NI::DSP::CabinetIRs::<Cabinet>_<Mic>[_<Preamp>]` arrays. DECKARD does not
//! ship them: [`bank`] reads them at run time from the user's Maschine install (the app
//! or the VST3 plug-in) by symbol name. Without an install the wet path is silent.
//!
//! Maschine convolves with a non-uniformly partitioned FFT convolver (no latency);
//! DECKARD convolves directly, which is the same up to Maschine's FFT rounding (≈ −135 dB).
//! At other rates than 44.1 kHz Maschine resamples the signal to 44.1 kHz and back around
//! the convolver with `NI::AB::SampleRateConverter`; `Src` is a port of its generic
//! (non-integer ratio) type, and the ×2/×½ types come from the Saturator's Tape (spec §4).

use super::{Effect, param};
use std::sync::OnceLock;

/// An `f32` constant as Maschine stores it.
const fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// Control-rate period: `DSPCoreWrapper::process` calls `processControlRate` every 128
/// samples.
const BLOCK: usize = 128;
/// Length of every impulse response, in samples at 44.1 kHz.
const IR_LEN: usize = 2048;
/// The rate the impulse responses are recorded at (`0x472c4400`).
const IR_RATE: f32 = 44100.0;
/// The wet path's fixed gain (`setParameter(5, 0.25)` in the data constructor).
const WET_GAIN: f32 = 0.25;
/// Ramps snap when their squared per-sample step is below this (`0x3cd203af9ee75616`).
const SNAP: f64 = 1e-15;
/// `1/3` and `2/3` as `f32`: the Cabinet selector's thresholds.
const THIRD: f32 = c(0x3eaa_aaab);
const TWO_THIRDS: f32 = c(0x3f2a_aaab);

/// `NI::DSP::CabinetIRs::s_IRs`: the IR sets in index order, each its variants in vector
/// order (as the static initializer fills them). The selectors only reach sets 0..3;
/// MesaBoogie (4) is unused.
const SETS: [&[&str]; 5] = [
    &[
        "Engl_HeilPR40_GML",
        "Engl_HeilPR40_V72s",
        "Engl_MD421_GML",
        "Engl_MD421_V72s",
        "Engl_MD441_GML",
        "Engl_MD441_V72s",
        "Engl_RE20_GML",
        "Engl_RE20_V72s",
        "Engl_Royer_GML",
        "Engl_Royer_V72s",
        "Engl_SM57full_GML",
        "Engl_SM57full_V72s",
        "Engl_SM57hell_GML",
        "Engl_SM57hell_V72s",
        "Engl_U47_none",
        "Engl_U47_V72s",
        "Engl_U67_GML",
        "Engl_U67_V72s",
    ],
    &["Fender_KM54", "Fender_MD421", "Fender_MD441", "Fender_Royer", "Fender_U47", "Fender_U67", "Fender_U89"],
    &[
        "Marshall_Heil_API",
        "Marshall_M160_Helios",
        "Marshall_MD421_GML",
        "Marshall_MD441_Neve",
        "Marshall_Re20_API",
        "Marshall_Re20_Neve",
        "Marshall_RE20_V72s",
        "Marshall_Royer_API",
        "Marshall_Royer_Neve",
        "Marshall_Royer_V74a",
        "Marshall_SM57full_Neve",
        "Marshall_SM57hell_API",
        "Marshall_U47_none",
        "Marshall_U67_API",
        "Marshall_U67_Neve",
        "Marshall_U67_V72s",
        "Marshall_U89",
    ],
    &[
        "Vox_MD421_API",
        "Vox_MD421_GML",
        "Vox_MD421_V72s",
        "Vox_MD441_API",
        "Vox_MD441_V72s",
        "Vox_MD441_V74a",
        "Vox_Royer_API",
        "Vox_Royer_V72s",
        "Vox_SM57_API",
        "Vox_SM57_GML",
        "Vox_SM57_V72s",
        "Vox_U47_API",
        "Vox_U47_none",
        "Vox_U47_V72s",
        "Vox_U87_API",
        "Vox_U87_GML",
        "Vox_U87_V72s",
    ],
    &[
        "MesaBoogie1_u47",
        "MesaBoogie1_m149",
        "MesaBoogie1_EQ_u47",
        "MesaBoogie1_EQ_m149",
        "MesaBoogie2_u47",
        "MesaBoogie2_m149",
        "MesaBoogie2_EQ_u47",
        "MesaBoogie2_EQ_m149",
    ],
];

/// Where to look for Maschine's binary (the first readable one wins);
/// `MASCHINE_BINARY` comes first when set.
const BINARIES: [&str; 2] = [
    "/Applications/Native Instruments/Maschine 3/Maschine 3.app/Contents/MacOS/Maschine 3",
    "/Library/Audio/Plug-Ins/VST3/Maschine 3.vst3/Contents/MacOS/Maschine 3",
];

/// The impulse responses, `sets[set][variant]`, as read from the user's Maschine.
pub struct Bank {
    sets: Vec<Vec<Box<[f32; IR_LEN]>>>,
}

/// The impulse responses, read once; `None` without a readable Maschine binary.
pub fn bank() -> Option<&'static Bank> {
    static B: OnceLock<Option<Bank>> = OnceLock::new();
    B.get_or_init(|| {
        let env = std::env::var("MASCHINE_BINARY").ok();
        env.iter().map(String::as_str).chain(BINARIES).find_map(|p| macho::load_bank(std::path::Path::new(p)).ok())
    })
    .as_ref()
}

/// Reading the IR arrays out of a (fat) Mach-O by their symbol names.
mod macho {
    use super::{Bank, IR_LEN, SETS};
    use std::fs::File;
    use std::io::{Error, ErrorKind, Read, Result, Seek, SeekFrom};
    use std::path::Path;

    fn u32_at(b: &[u8], o: usize, be: bool) -> u32 {
        let a = [b[o], b[o + 1], b[o + 2], b[o + 3]];
        if be { u32::from_be_bytes(a) } else { u32::from_le_bytes(a) }
    }
    fn u64_at(b: &[u8], o: usize) -> u64 {
        u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
    }
    fn read_at(f: &mut File, off: u64, len: usize) -> Result<Vec<u8>> {
        let mut v = vec![0; len];
        f.seek(SeekFrom::Start(off))?;
        f.read_exact(&mut v)?;
        Ok(v)
    }
    fn bad(m: &str) -> Error {
        Error::new(ErrorKind::InvalidData, m.to_string())
    }

    /// The `__ZN2NI3DSP10CabinetIRs<len><name>E` symbol of an IR.
    fn mangled(name: &str) -> String {
        format!("__ZN2NI3DSP10CabinetIRs{}{name}E", name.len())
    }

    pub(super) fn load_bank(path: &Path) -> Result<Bank> {
        let mut f = File::open(path)?;
        let flen = f.metadata()?.len() as usize;
        let head = read_at(&mut f, 0, 4096.min(flen))?;
        // A fat binary: the arm64 slice, else x86_64 (both hold the same arrays).
        let base = if u32_at(&head, 0, true) == 0xcafe_babe {
            let n = u32_at(&head, 4, true) as usize;
            let arches: Vec<(u32, u64)> =
                (0..n.min(100)).map(|i| (u32_at(&head, 8 + 20 * i, true), u32_at(&head, 16 + 20 * i, true) as u64)).collect();
            arches
                .iter()
                .find(|a| a.0 == 0x0100_000c)
                .or_else(|| arches.iter().find(|a| a.0 == 0x0100_0007))
                .ok_or_else(|| bad("no 64-bit slice"))?
                .1
        } else {
            0
        };
        let mh = read_at(&mut f, base, 32)?;
        if u32_at(&mh, 0, false) != 0xfeed_facf {
            return Err(bad("not a 64-bit Mach-O"));
        }
        let ncmds = u32_at(&mh, 16, false) as usize;
        let cmds = read_at(&mut f, base + 32, u32_at(&mh, 20, false) as usize)?;
        // (vmaddr, vmsize, fileoff) of each segment, and the symbol table.
        let mut segs = Vec::new();
        let mut symtab = None;
        let mut o = 0;
        for _ in 0..ncmds {
            if o + 8 > cmds.len() {
                return Err(bad("truncated load commands"));
            }
            let (cmd, len) = (u32_at(&cmds, o, false), u32_at(&cmds, o + 4, false) as usize);
            match cmd {
                0x19 => segs.push((u64_at(&cmds, o + 24), u64_at(&cmds, o + 32), u64_at(&cmds, o + 40))),
                0x2 => {
                    let w = |k| u32_at(&cmds, o + k, false) as u64;
                    symtab = Some((w(8), w(12), w(16), w(20)));
                }
                _ => {}
            }
            o += len.max(8);
        }
        let (symoff, nsyms, stroff, strsize) = symtab.ok_or_else(|| bad("no symbol table"))?;
        let syms = read_at(&mut f, base + symoff, nsyms as usize * 16)?;
        let strs = read_at(&mut f, base + stroff, strsize as usize)?;
        let prefix = b"__ZN2NI3DSP10CabinetIRs";
        let mut addr = std::collections::HashMap::new();
        for s in syms.as_chunks::<16>().0 {
            let x = u32_at(s, 0, false) as usize;
            if strs.get(x..x + prefix.len()) == Some(&prefix[..]) {
                let end = strs[x..].iter().position(|&b| b == 0).map_or(strs.len(), |e| x + e);
                addr.insert(String::from_utf8_lossy(&strs[x..end]).into_owned(), u64_at(s, 8));
            }
        }
        let mut sets = Vec::new();
        for names in SETS {
            let mut irs = Vec::new();
            for name in names {
                let a = *addr.get(&mangled(name)).ok_or_else(|| bad("IR symbol missing"))?;
                let end = a + (IR_LEN * 4) as u64;
                let &(vm, _, fo) = segs.iter().find(|s| a >= s.0 && end <= s.0 + s.1).ok_or_else(|| bad("IR outside the segments"))?;
                let raw = read_at(&mut f, base + fo + (a - vm), IR_LEN * 4)?;
                let mut ir = Box::new([0f32; IR_LEN]);
                for (d, b) in ir.iter_mut().zip(raw.as_chunks::<4>().0) {
                    *d = f32::from_le_bytes(*b);
                }
                if ir.iter().any(|v| !v.is_finite() || v.abs() > 16.0) {
                    return Err(bad("IR data implausible"));
                }
                irs.push(ir);
            }
            sets.push(irs);
        }
        Ok(Bank { sets })
    }
}

/// The IR set of a Cabinet value `v` (normalised, `idx/3`; `processCabinetParameter`):
/// `(u32)(x·4)` of 0.4, 0.8, 0.6 or 0.2, so Am. 2x12 → Fender (1), Brit. 2x12A → Vox
/// (3), Vin. 4x12A → Marshall (2), Mod. 4x12 → Engl (0).
fn ir_set(v: f32) -> usize {
    let x: f32 = if v < THIRD {
        0.4
    } else if v < TWO_THIRDS {
        0.8
    } else if v < 1.0 {
        0.6
    } else {
        0.2
    };
    (x * 4.0) as u32 as usize
}

/// The Microphone's position in its set as a fraction (`processMicrophoneParameter`), by
/// the Cabinet value `cv` (`idx/3`) and the Microphone value `mv` (`idx/5`).
fn mic_frac(cv: f32, mv: f32) -> f32 {
    let row: [f32; 6] = if cv < THIRD {
        // Fender: U89, MD421, MD441, Royer, U47, U67.
        [1.0, c(0x3e2a_aaab), THIRD, 0.5, TWO_THIRDS, c(0x3f55_5555)]
    } else if cv < TWO_THIRDS {
        // Vox: SM57_API, MD421_GML, MD441_API, Royer_API, U47_API, U87_GML.
        [0.5, 0.0625, 0.1875, 0.375, 0.6875, 0.9375]
    } else if cv < 1.0 {
        // Marshall: SM57full_Neve, MD421_GML, MD441_Neve, Royer_Neve, U47_none, U67_Neve.
        [0.625, 0.125, 0.1875, 0.5, 0.75, 0.875]
    } else {
        // Engl (k/17): SM57full_GML, MD421_GML, MD441_GML, Royer_GML, U47_none, U67_GML.
        [c(0x3f16_9697), c(0x3df0_f0f1), c(0x3e70_f0f1), c(0x3ef0_f0f1), c(0x3f52_d2d3), c(0x3f70_f0f1)]
    };
    let k = [0.2f32, 0.4, 0.6, 0.8, 1.0].iter().take_while(|&&t| mv >= t).count();
    row[k]
}

/// A variant index: `(int)((count − 1)·frac)`, in `f32`.
fn variant(count: usize, frac: f32) -> usize {
    (((count as i32 - 1) as f32 * frac) as i32).clamp(0, count as i32 - 1) as usize
}

/// A selector's stored index as Maschine normalises it: `idx/(n − 1)`.
fn norm(idx: f64, n: usize) -> f32 {
    (idx.round().clamp(0.0, (n - 1) as f64) as f32) / (n - 1) as f32
}

/// A `SmoothingToolbox` ramp over `N` lanes: linear over `ticks` control ticks of
/// `period` samples, snapping lanes whose step is negligible.
struct Ramp<const N: usize> {
    cur: [f32; N],
    inc: [f32; N],
    tgt: [f32; N],
    dirty: bool,
    countdown: i32,
    ticks: i32,
    inv: f32,
}

impl<const N: usize> Ramp<N> {
    fn new(ticks: i32, period: i32) -> Self {
        Ramp { cur: [0.0; N], inc: [0.0; N], tgt: [0.0; N], dirty: false, countdown: 0, ticks, inv: 1.0 / (ticks * period) as f32 }
    }

    /// `max(1, (int)(sr·0.02/128 + 0.5))` ticks: ≈ 20 ms.
    fn ms20(sr: f32) -> Self {
        Self::new((((sr * c(0x3ca3_d70a)) / BLOCK as f32 + 0.5) as i32).max(1), BLOCK as i32)
    }

    fn step(&mut self) {
        for k in 0..N {
            self.cur[k] += self.inc[k];
        }
    }

    /// A control tick.
    fn tick(&mut self) {
        if self.dirty {
            self.dirty = false;
            let mut any = false;
            for k in 0..N {
                self.inc[k] = (self.tgt[k] - self.cur[k]) * self.inv;
                if ((self.inc[k] * self.inc[k]) as f64) < SNAP {
                    self.inc[k] = 0.0;
                    self.cur[k] = self.tgt[k];
                } else {
                    any = true;
                }
            }
            if any {
                self.countdown = self.ticks;
            }
        } else {
            self.countdown = self.countdown.wrapping_sub(1);
            if self.countdown == 0 {
                self.cur = self.tgt;
                self.inc = [0.0; N];
            }
        }
    }
}

/// Direct convolution of both channels with one (mono) impulse response.
struct Convolver {
    ir: Vec<f32>,
    /// Input history per channel, newest first, stored twice so a window is contiguous.
    hist: [Vec<f32>; 2],
    pos: usize,
}

impl Convolver {
    fn new(ir: Vec<f32>) -> Self {
        let n = ir.len();
        Convolver { ir, hist: [vec![0.0; 2 * n], vec![0.0; 2 * n]], pos: 0 }
    }

    fn process(&mut self, x: [f32; 2]) -> [f32; 2] {
        let n = self.ir.len();
        self.pos = if self.pos == 0 { n - 1 } else { self.pos - 1 };
        let mut y = [0f32; 2];
        for (ch, out) in y.iter_mut().enumerate() {
            let h = &mut self.hist[ch];
            h[self.pos] = x[ch];
            h[self.pos + n] = x[ch];
            // `f64` sums: closer to the exact convolution than Maschine's FFT rounding,
            // so a null test measures Maschine's error alone.
            let mut acc = [0f64; 4];
            for (hk, wk) in self.ir.as_chunks::<4>().0.iter().zip(h[self.pos..self.pos + n].as_chunks::<4>().0) {
                for j in 0..4 {
                    acc[j] += hk[j] as f64 * wk[j] as f64;
                }
            }
            *out = ((acc[0] + acc[1]) + (acc[2] + acc[3])) as f32;
        }
        y
    }
}

/// `SampleRateConverter::InterpolCoefficients` for quality 2: an 8-point interpolation
/// kernel, four segments of degree 5 in `|u|` (coefficients of `u^0 … u^5`).
const KERNEL: [[f32; 6]; 4] = [
    [c(0x3f80_0000), 0.0, c(0xbfef_bf40), c(0xbdab_07b6), c(0x3fc9_948e), c(0xbf1e_49a5)],
    [c(0x3ee7_235d), c(0x4046_ad79), c(0xc10d_895e), c(0x40f7_7d97), c(0xc034_3706), c(0x3ebe_7b5c)],
    [c(0x4110_2672), c(0xc1ad_d1f9), c(0x419d_38bb), c(0xc107_b3d9), c(0x3fe2_31fe), c(0xbe12_d342)],
    [c(0xc0bd_1d3c), c(0x4125_984e), c(0xc0dc_3032), c(0x400d_18b9), c(0xbeaf_eafa), c(0x3cab_bd9a)],
];
const K_POINTS: u32 = 8;
const K_ORDER: usize = 5;

/// `C(j, m)`.
fn binom(j: usize, m: usize) -> f32 {
    let (mut num, mut den) = (1u32, 1u32);
    for k in 0..(j - m) as u32 {
        num *= j as u32 - k;
        den *= k + 1;
    }
    (num / den) as f32
}

/// `NI::AB::SampleRateConverter` of type "generic" (any non-integer ratio), quality 2,
/// two channels (spec §4.1). Each output sample is an FIR over the last `taps` inputs
/// whose coefficients are polynomials in the output's fractional phase `x`: per tap, a
/// Taylor-shifted kernel segment, switching to the next segment once `x` passes the
/// tap's boundary. Downsampling stretches the kernel by `in/out` and scales it by
/// `out/in`.
struct Src {
    taps: usize,
    /// Per group of four taps: boundaries, then (lo, hi) coefficient vectors for
    /// `x^5 … x^0`; `groups·(2·5 + 3)` vectors of 4.
    table: Vec<[f32; 4]>,
    /// Inputs per output: `step` + `rem/out`.
    step: u32,
    rem: f32,
    out: f32,
    inv_out: f32,
    /// Input samples still to take before the next output, and the output's phase.
    pending: u32,
    phase: f32,
    /// History per channel, newest at `taps − 1` (padded to whole groups with zeros).
    hist: [Vec<f32>; 2],
}

impl Src {
    /// `init(fin, fout, 2, quality 2)` for a non-integer ratio (`init_generic`).
    fn new(fin: f32, fout: f32) -> Src {
        let r = fout / fin;
        let (mut step, mut rem) = (0, fin);
        if fout <= fin {
            let mut f = fin;
            loop {
                step += 1;
                f -= fout;
                if fout > f {
                    break;
                }
            }
            rem = f;
        }
        let taps = if r > 1.0 { K_POINTS as usize } else { ((K_POINTS as f64 * fin as f64) / fout as f64).ceil() as usize };
        let groups = taps.div_ceil(4);
        let (da, db, g) = if r > 1.0 { (2.0f64, 2.0f64, 1.0f32) } else { (2.0 * fout as f64, 2.0 * fin as f64, fout / fin) };
        let base = (db * K_POINTS as f64) * -0.5;
        let n = groups * 4;
        let mut bound = vec![0f32; n];
        let mut lo = vec![vec![0f32; n]; K_ORDER + 1];
        let mut hi = vec![vec![0f32; n]; K_ORDER + 1];
        let half = K_POINTS / 2;
        for i in 0..taps {
            let idx = taps - 1 - i;
            let di = i as f64;
            let t = di.mul_add(da, base);
            let a = if t >= 0.0 { t } else { -di.mul_add(da, base) };
            let q = a / db;
            let seg0 = q as u32;
            let rm = (-(seg0 as f64)).mul_add(db, a);
            let (down, d16) = if rm != 0.0 { (0, rm) } else { ((t < 0.0) as u32, db) };
            let dist = if t >= 0.0 { db - rm } else { d16 };
            let seg = seg0 - down;
            let dir: i32 = if t < 0.0 { -((seg != 0) as i32) } else { 1 };
            let s28 = if t >= 0.0 { g } else { -g };
            let flip: f32 = if da + t > 0.0 && t < 0.0 { -1.0 } else { 1.0 };
            let u0 = q as f32;
            let hseg = (dir + seg as i32) as u32;
            bound[idx] = (dist / da) as f32;
            let (s30, s31) = (flip * u0, s28 * flip);
            for m in 0..=K_ORDER {
                let mut v = 0f32;
                if seg < half {
                    for j in (m..=K_ORDER).rev() {
                        v = KERNEL[seg as usize][j].mul_add(binom(j, m), v * u0);
                    }
                    for _ in 0..m {
                        v *= s28;
                    }
                }
                lo[m][idx] = g * v;
                let mut v = 0f32;
                if hseg < half {
                    for j in (m..=K_ORDER).rev() {
                        v = KERNEL[hseg as usize][j].mul_add(binom(j, m), s30 * v);
                    }
                    for _ in 0..m {
                        v *= s31;
                    }
                }
                hi[m][idx] = g * v;
            }
        }
        let lanes = |v: &[f32], k: usize| -> [f32; 4] { std::array::from_fn(|j| v[4 * k + j]) };
        let mut table = Vec::with_capacity(groups * (2 * K_ORDER + 3));
        for k in 0..groups {
            table.push(lanes(&bound, k));
            for m in (0..=K_ORDER).rev() {
                table.push(lanes(&lo[m], k));
                table.push(lanes(&hi[m], k));
            }
        }
        Src { taps, table, step, rem, out: fout, inv_out: 1.0 / fout, pending: 1, phase: 0.0, hist: [vec![0.0; n], vec![0.0; n]] }
    }

    /// How many outputs `n` more inputs make (`Cabinet::processAudio`): rounded up.
    fn outputs_for(&self, n: usize) -> usize {
        let num = (n as i64 - self.pending as i64 + 1) as f64 * self.out as f64 - self.phase as f64;
        let den = (self.step as f32).mul_add(self.out, self.rem) as f64;
        (num / den).ceil().max(0.0) as usize
    }

    /// Shift `k` inputs (from `pos`) into the history.
    fn take(&mut self, input: [&[f32]; 2], pos: usize, k: usize) {
        let t = self.taps;
        for (h, x) in self.hist.iter_mut().zip(input) {
            h.copy_within(k..t, 0);
            h[t - k..t].copy_from_slice(&x[pos..pos + k]);
        }
    }

    /// `convert(input, output)` (`process_generic`): all of `input` is offered, and
    /// `output` is filled.
    fn convert(&mut self, input: [&[f32]; 2], output: [&mut [f32]; 2]) {
        let nin = input[0].len();
        let mut used = (self.pending as usize).min(nin);
        if used > 0 {
            self.take(input, 0, used);
        }
        self.pending -= used as u32;
        let groups = self.table.len() / (2 * K_ORDER + 3);
        let mut coef = vec![[0f32; 4]; groups];
        let [ol, or] = output;
        for o in 0..ol.len() {
            let x = self.phase * self.inv_out;
            for (k, cf) in coef.iter_mut().enumerate() {
                let t = &self.table[k * (2 * K_ORDER + 3)..];
                let mask: [bool; 4] = std::array::from_fn(|j| x >= t[0][j]);
                let mut acc: [f32; 4] = std::array::from_fn(|j| if mask[j] { t[2][j] } else { t[1][j] });
                for m in 1..=K_ORDER {
                    let (l, h) = (t[1 + 2 * m], t[2 + 2 * m]);
                    for j in 0..4 {
                        acc[j] = acc[j] * x + if mask[j] { h[j] } else { l[j] };
                    }
                }
                *cf = acc;
            }
            for (h, y) in self.hist.iter().zip([&mut ol[o], &mut or[o]]) {
                let mut s = [0f32; 4];
                for (k, cf) in coef.iter().enumerate() {
                    for j in 0..4 {
                        let p = h[4 * k + j] * cf[j];
                        s[j] = if k == 0 { p } else { s[j] + p };
                    }
                }
                *y = (s[0] + s[2]) + (s[1] + s[3]);
            }
            let mut k = self.step as usize;
            self.phase += self.rem;
            if self.phase > self.out {
                self.phase -= self.out;
                k += 1;
            }
            if k > 0 {
                let total = used + k;
                if total > nin {
                    // Short of input: the rest is taken first in the next call.
                    self.pending = (total - nin) as u32;
                    let k = nin.saturating_sub(used).min(k);
                    if k > 0 {
                        self.take(input, used, k);
                    }
                } else {
                    self.take(input, used, k);
                }
                used = total;
            }
        }
    }
}

// ---- The ×2 / ×½ converters at 88.2 kHz: copied from saturator/tape.rs (its Tape mode
// drives them the same way: n inputs → 2n outputs and back, one pending sample). ----

/// `SampleRateConverter::InterpolCoefficients` of the integer-ratio ×2 / ×½ converters
/// (`init_intrat`, quality 2): `h(t) = sinc(t)·I0(8·√(1 − (t/8)²)) / 427.564…` at
/// `t = k + ½`, `k = −8..7` (Abramowitz–Stegun polynomials for I0).
fn src_coefs() -> [f32; 16] {
    let mut h = [0f32; 16];
    let pi = f64::from_bits(0x4009_21fb_5444_2d11);
    let ph = 0f32 + 1.0 / 2.0; // DOUBLE: 0 + 1/2; HALF: 1 + (−1/2) — the same 0.5
    for (k, o) in (-8i32..8).zip(h.iter_mut()) {
        let t = ph + k as f32;
        if t == 0.0 {
            *o = 1.0;
            continue;
        }
        let a = t as f64 * pi;
        let s = a.sin();
        let u = ((t + t) * c(0x3d80_0000)) as f64;
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
        *o = ((s / a) * i0 / f64::from_bits(0x407a_b906_bec5_d25f)) as f32;
    }
    h
}

/// One input sample into the converters' transposed 16-accumulator FIR (`r` = 16 floats);
/// returns the finished output. Products and sums are separate roundings.
fn fir_step(r: &mut [f32], h: &[f32; 16], x: f32) -> f32 {
    let p = [x * h[0], x * h[1], x * h[2], x * h[3]];
    let t1 = [r[0] + p[0], r[1] + p[1], r[2] + p[2], r[3] + p[3]];
    let t2 = [p[0] + r[4], p[1] + r[5], p[2] + r[6], p[3] + r[7]];
    let q = [x * h[4], x * h[5], x * h[6], x * h[7]];
    let t3 = [r[8] + q[0], r[9] + q[1], r[10] + q[2], r[11] + q[3]];
    let t4 = [q[0] + r[12], q[1] + r[13], q[2] + r[14], q[3] + r[15]];
    r[0..4].copy_from_slice(&[t1[1], t1[2], t1[3], t1[0]]);
    r[4..8].copy_from_slice(&[t2[3], t2[0], t2[1], t2[2]]);
    r[8..12].copy_from_slice(&[t3[1], t3[2], t3[3], t3[0]]);
    r[12..16].copy_from_slice(&[t4[3], t4[0], t4[1], t4[2]]);
    let out = t1[0];
    r[3] = r[11];
    r[11] = r[12];
    r[12] = r[4];
    r[4] = 0.0;
    out
}

/// `process_intratDOUBLE` for one channel (`pend` = 1 at every call, as Tape uses it):
/// even outputs are the input delayed by 8, odd ones the FIR.
struct Up2 {
    delay: [f32; 8],
    acc: [f32; 16],
}

impl Up2 {
    fn run(&mut self, h: &[f32; 16], inp: &[f32], out: &mut [f32]) {
        let n = inp.len();
        for k in 0..n {
            out[2 * k] = if k < 8 { self.delay[k] } else { inp[k - 8] };
            out[2 * k + 1] = fir_step(&mut self.acc, h, inp[k]);
        }
        let mut d = [0f32; 8];
        for (j, o) in d.iter_mut().enumerate() {
            let i = n + j;
            *o = if i < 8 { self.delay[i] } else { inp[i - 8] };
        }
        self.delay = d;
    }
}

/// `process_intratHALF` for one channel (`pend` = 1 at every call): output `k` is
/// `½·(even input delayed by 8 + FIR of the odd inputs up to 2k − 1)`.
struct Down2 {
    delay: [f32; 8],
    acc: [f32; 16],
    sum: f32,
}

impl Down2 {
    fn run(&mut self, h: &[f32; 16], inp: &[f32], out: &mut [f32]) {
        let n = out.len();
        for k in 0..n {
            let d = if k < 8 { self.delay[k] } else { inp[2 * (k - 8)] };
            self.sum += d;
            out[k] = self.sum * 0.5;
            self.sum = 0.0;
            let y = fir_step(&mut self.acc, h, inp[2 * k + 1]);
            self.sum += y;
        }
        let mut d = [0f32; 8];
        for (j, o) in d.iter_mut().enumerate() {
            let i = n + j;
            *o = if i < 8 { self.delay[i] } else { inp[2 * (i - 8)] };
        }
        self.delay = d;
    }
}

/// `NI::DSP::Air` (Distance): taps of two band-passed (≈ 262–1109 Hz) feedback delay
/// lines, added to the signal with a weight of `0.7·distance` (§3.3).
struct Air {
    sr: f32,
    /// Distance (raw, unsmoothed): Air is skipped while it is ≤ 0.
    amount: f32,
    mask: usize,
    a: Vec<f32>,
    b: Vec<f32>,
    w: usize,
    taps: [i32; 8],
    /// Lanes: 0.7·distance, 1, 0.7·distance, 1, size (s), 0, 0, 0.
    ramp: Ramp<8>,
    c1: f32,
    c2: f32,
    s1: [f32; 4],
    s2: [f32; 4],
}

/// Tap weights (`Air+0x80`) and the feedback weight (`Air+0x90`, lanes 1 and 3).
const AIR_G: [f32; 4] = [c(0x3f80_0000), c(0xbf3b_02f0), c(0x3f08_9d4b), c(0xbec7_98ee)];
const AIR_FB: f32 = c(0x3e23_d70a);

impl Air {
    fn new(sr: f32) -> Self {
        // `Air::onSetSampleRate`: 2^(⌊ln(sr·0.10504)/ln 2⌋ + 1) samples per line.
        let bits = ((sr * c(0x3dd7_2105)).ln() / c(0x3f31_7218)) as i32 + 1;
        let len = 1usize << bits.clamp(1, 24);
        let w1 = c(0x40c9_0e56) / sr; // 2π/sr
        let mut ramp = Ramp::ms20(sr);
        // `setParameter(3, 1/3)` (fixed): the size, 1/3·0.03 + 0.005 = 0.015 s.
        ramp.tgt = [0.0, 1.0, 0.0, 1.0, THIRD.mul_add(c(0x3cf5_c290), c(0x3ba3_d70a)), 0.0, 0.0, 0.0];
        ramp.cur = ramp.tgt;
        let mut air = Air {
            sr,
            amount: 0.0,
            mask: len - 1,
            a: vec![0.0; len],
            b: vec![0.0; len],
            w: 0,
            taps: [0; 8],
            ramp,
            c1: (w1 * c(0x4382_d013)).min(1.0), // 261.63 Hz
            c2: (w1 * c(0x448a_9760)).min(1.0), // 1108.73 Hz
            s1: [0.0; 4],
            s2: [0.0; 4],
        };
        air.set_taps(air.ramp.cur[4]);
        air
    }

    fn set_taps(&mut self, size: f32) {
        let mut s = self.sr * size;
        for t in &mut self.taps {
            *t = s as i32;
            s *= c(0x3f95_c28f); // 1.17
        }
    }

    fn set_distance(&mut self, v: f32, now: bool) {
        self.amount = v;
        self.ramp.tgt[0] = v * c(0x3f33_3333);
        self.ramp.tgt[2] = v * c(0x3f33_3333);
        self.ramp.dirty = true;
        if now {
            self.ramp.cur = self.ramp.tgt;
        }
    }

    fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.ramp.step();
        if self.ramp.inc[4] != 0.0 {
            self.set_taps(self.ramp.cur[4]);
        }
        if self.amount <= 0.0 {
            return (l, r);
        }
        let (m, w, t) = (self.mask, self.w, self.taps);
        let at = |k: usize| (t[k] as usize).wrapping_add(w) & m;
        let va = [self.a[at(0)], self.a[at(2)], self.a[at(4)], self.a[at(7)]];
        let vb = [self.b[at(1)], self.b[at(3)], self.b[at(5)], self.b[at(7)]];
        let ga: [f32; 4] = std::array::from_fn(|k| va[k] * AIR_G[k]);
        let gb: [f32; 4] = std::array::from_fn(|k| vb[k] * AIR_G[k]);
        let sa = (ga[0] + ga[2]) + (ga[1] + ga[3]);
        let sb = (gb[0] + gb[2]) + (gb[1] + gb[3]);
        let fb = AIR_FB * (sa + sb);
        // Lanes 0 and 2 of the filters run on the dry input and are never read.
        let u = [l + fb, r + fb];
        for (k, x) in [1, 3].into_iter().zip(u) {
            let d = x - self.s1[k];
            self.s1[k] += self.c1 * d;
            let e = d - self.s2[k];
            self.s2[k] += self.c2 * e;
        }
        self.w = w.wrapping_sub(1) & m;
        self.a[self.w] = self.s2[1];
        self.b[self.w] = self.s2[3];
        (sa.mul_add(self.ramp.cur[0], l), sb.mul_add(self.ramp.cur[2], r))
    }
}

/// The stored settings.
#[derive(Clone, Copy, PartialEq)]
struct Settings {
    cabinet: f64,
    microphone: f64,
    distance: f32,
    mix: f32,
}

/// The two converters around the convolver when the rate is not 44.1 kHz.
enum Converters {
    /// Any non-integer ratio.
    Generic { down: Src, up: Src },
    /// 88.2 kHz: `intratHALF` down, `intratDOUBLE` up.
    Half { h: [f32; 16], down: [Down2; 2], up: [Up2; 2] },
    /// 22.05 kHz: `intratDOUBLE` up, `intratHALF` down.
    Double { h: [f32; 16], up: [Up2; 2], down: [Down2; 2] },
}

struct Resampling {
    conv: Converters,
    /// The signal at 44.1 kHz.
    buf: [Vec<f32>; 2],
}

pub struct Cabinet {
    bank: Option<&'static Bank>,
    /// The IR set in use, and the microphone's fraction as last set (§3.1).
    set: usize,
    mic: f32,
    /// The variant last asked for, and the one in use (or being faded to).
    want: (usize, usize),
    have: (usize, usize),
    conv: Convolver,
    /// While crossfading: the new IR's convolver, started from silence.
    next: Option<Convolver>,
    /// Crossfade gains: lanes (old, new).
    fade: Ramp<2>,
    resampling: Option<Resampling>,
    air: Air,
    /// Wet (Mix·0.25) and dry (1 − Mix) gains.
    mix: Ramp<2>,
    /// Samples to the next control tick.
    left: usize,
    values: Option<Settings>,
}

impl Cabinet {
    pub fn new(sr: f64) -> Self {
        let sr = sr as f32;
        // The crossfade runs at the convolver's rate (44.1 kHz): ⌊882/period + ½⌋ ticks
        // of period = (int)(5644800/sr) samples.
        let period = ((5_644_800.0 / sr) as i32).max(1);
        let fade_ticks = ((882.0 / period as f32 + 0.5) as i32).max(1);
        let mut fade = Ramp::new(fade_ticks, period);
        fade.cur = [1.0, 0.0];
        fade.tgt = [1.0, 0.0];
        let resampling = (sr != IR_RATE).then(|| Resampling {
            conv: if sr == 2.0 * IR_RATE {
                Converters::Half {
                    h: src_coefs(),
                    down: std::array::from_fn(|_| Down2 { delay: [0.0; 8], acc: [0.0; 16], sum: 0.0 }),
                    up: std::array::from_fn(|_| Up2 { delay: [0.0; 8], acc: [0.0; 16] }),
                }
            } else if 2.0 * sr == IR_RATE {
                Converters::Double {
                    h: src_coefs(),
                    up: std::array::from_fn(|_| Up2 { delay: [0.0; 8], acc: [0.0; 16] }),
                    down: std::array::from_fn(|_| Down2 { delay: [0.0; 8], acc: [0.0; 16], sum: 0.0 }),
                }
            } else {
                // Other integer ratios (11.025, 176.4 kHz …) use Maschine's `intratUP/DOWN`,
                // not ported: the generic converter stands in.
                Converters::Generic { down: Src::new(sr, IR_RATE), up: Src::new(IR_RATE, sr) }
            },
            buf: [vec![0.0; 2 * BLOCK], vec![0.0; 2 * BLOCK]],
        });
        let bank = bank();
        let mut cab = Cabinet {
            bank,
            set: 0,
            mic: 0.0,
            want: (0, 0),
            have: (0, 0),
            conv: Convolver::new(vec![0.0; IR_LEN]),
            next: None,
            fade,
            resampling,
            air: Air::new(sr),
            mix: Ramp::ms20(sr),
            left: 0,
            values: None,
        };
        cab.conv = Convolver::new(cab.load((0, 0)));
        cab
    }

    /// The impulse response of (set, variant), or silence without a Maschine install.
    fn load(&self, sel: (usize, usize)) -> Vec<f32> {
        self.bank.map_or_else(|| vec![0.0; IR_LEN], |b| b.sets[sel.0][sel.1].to_vec())
    }

    /// `processCabinetParameter`: a new set keeps the microphone's fraction.
    fn set_cabinet(&mut self, v: f32) {
        let set = ir_set(v);
        if set != self.set {
            self.set = set;
            self.want = (set, variant(SETS[set].len(), self.mic));
        }
    }

    /// `processMicrophoneParameter`: the fraction comes from the current Cabinet's row.
    fn set_microphone(&mut self, cv: f32, v: f32) {
        self.mic = mic_frac(cv, v);
        self.want = (self.set, variant(SETS[self.set].len(), self.mic));
    }

    fn set_mix(&mut self, v: f32, now: bool) {
        self.mix.tgt = [v * WET_GAIN, 1.0 - v];
        self.mix.dirty = true;
        if now {
            self.mix.cur = self.mix.tgt;
        }
    }

    /// `processControlRate` (§3.4).
    fn control(&mut self) {
        self.air.ramp.tick();
        if self.next.is_none() && self.want != self.have {
            self.have = self.want;
            self.next = Some(Convolver::new(self.load(self.want)));
            self.fade.tgt = [0.0, 1.0];
            self.fade.dirty = true;
        }
        self.fade.tick();
        self.mix.tick();
        if self.next.is_some() && self.fade.inc[0] == 0.0 {
            // Done: the new convolver takes over.
            self.conv = self.next.take().unwrap();
            self.fade.cur = [1.0, 0.0];
            self.fade.tgt = [1.0, 0.0];
        }
    }

    /// The convolution (with a running crossfade), in place, at 44.1 kHz.
    fn convolve(conv: &mut Convolver, next: &mut Option<Convolver>, fade: &mut Ramp<2>, l: &mut [f32], r: &mut [f32]) {
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            let x = [*a, *b];
            let [wl, wr] = conv.process(x);
            (*a, *b) = match next {
                Some(n) => {
                    fade.step();
                    let [nl, nr] = n.process(x);
                    let [g_old, g_new] = fade.cur;
                    (wl.mul_add(g_old, nl * g_new), wr.mul_add(g_old, nr * g_new))
                }
                None => (wl, wr),
            };
        }
    }

    /// `processAudio` for one stretch inside a control period.
    fn chunk(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        let mut dry = [[0f32; BLOCK]; 2];
        dry[0][..n].copy_from_slice(l);
        dry[1][..n].copy_from_slice(r);
        match &mut self.resampling {
            None => Self::convolve(&mut self.conv, &mut self.next, &mut self.fade, l, r),
            Some(rs) => {
                let m = match &rs.conv {
                    Converters::Generic { down, .. } => down.outputs_for(n),
                    Converters::Half { .. } => n.div_ceil(2),
                    Converters::Double { .. } => 2 * n,
                };
                for b in &mut rs.buf {
                    if b.len() < m {
                        b.resize(m, 0.0);
                    }
                }
                let [bl, br] = &mut rs.buf;
                let (bl, br) = (&mut bl[..m], &mut br[..m]);
                match &mut rs.conv {
                    Converters::Generic { down, .. } => down.convert([&dry[0][..n], &dry[1][..n]], [&mut *bl, &mut *br]),
                    Converters::Half { h, down, .. } => {
                        // Odd stretches (odd host blocks) are not modelled: pad with silence.
                        let mut x = [[0f32; BLOCK + 1]; 2];
                        x[0][..n].copy_from_slice(&dry[0][..n]);
                        x[1][..n].copy_from_slice(&dry[1][..n]);
                        down[0].run(h, &x[0][..2 * m], bl);
                        down[1].run(h, &x[1][..2 * m], br);
                    }
                    Converters::Double { h, up, .. } => {
                        up[0].run(h, &dry[0][..n], bl);
                        up[1].run(h, &dry[1][..n], br);
                    }
                }
                Self::convolve(&mut self.conv, &mut self.next, &mut self.fade, bl, br);
                match &mut rs.conv {
                    Converters::Generic { up, .. } => up.convert([&*bl, &*br], [l, r]),
                    Converters::Half { h, up, .. } => {
                        let mut y = [[0f32; BLOCK + 2]; 2];
                        up[0].run(h, bl, &mut y[0][..2 * m]);
                        up[1].run(h, br, &mut y[1][..2 * m]);
                        l.copy_from_slice(&y[0][..n]);
                        r.copy_from_slice(&y[1][..n]);
                    }
                    Converters::Double { h, down, .. } => {
                        down[0].run(h, bl, l);
                        down[1].run(h, br, r);
                    }
                }
            }
        }
        for i in 0..n {
            let (wl, wr) = self.air.process(l[i], r[i]);
            self.mix.step();
            let [wet, dry_g] = self.mix.cur;
            l[i] = wl.mul_add(wet, dry[0][i] * dry_g);
            r[i] = wr.mul_add(wet, dry[1][i] * dry_g);
        }
    }
}

impl Effect for Cabinet {
    fn set(&mut self, params: &[(String, f64)]) {
        let s = Settings {
            cabinet: param(params, "cabinet", 0.0),
            microphone: param(params, "microphone", 0.0),
            distance: param(params, "distance", 0.0) as f32,
            mix: param(params, "mix", 1.0) as f32,
        };
        let (cv, mv) = (norm(s.cabinet, 4), norm(s.microphone, 6));
        match self.values {
            None => {
                // The first settings reach the processor while it is suspended: no ramps
                // and no crossfade.
                self.set_cabinet(cv);
                self.set_microphone(cv, mv);
                self.have = self.want;
                self.conv = Convolver::new(self.load(self.want));
                self.air.set_distance(s.distance, true);
                self.set_mix(s.mix, true);
            }
            Some(old) => {
                if s.cabinet != old.cabinet {
                    self.set_cabinet(cv);
                }
                if s.microphone != old.microphone {
                    self.set_microphone(cv, mv);
                }
                if s.distance.to_bits() != old.distance.to_bits() {
                    self.air.set_distance(s.distance, false);
                }
                if s.mix.to_bits() != old.mix.to_bits() {
                    self.set_mix(s.mix, false);
                }
            }
        }
        self.values = Some(s);
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let mut i = 0;
        while i < l.len() {
            if self.left == 0 {
                self.control();
                self.left = BLOCK;
            }
            let n = self.left.min(l.len() - i);
            self.chunk(&mut l[i..i + n], &mut r[i..i + n]);
            self.left -= n;
            i += n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(cab: f64, mic: f64, dist: f64, mix: f64) -> Vec<(String, f64)> {
        vec![("cabinet".into(), cab), ("microphone".into(), mic), ("distance".into(), dist), ("mix".into(), mix)]
    }

    #[test]
    fn selectors_pick_the_documented_irs() {
        let pick = |cab: usize, mic: usize| {
            let (cv, mv) = (norm(cab as f64, 4), norm(mic as f64, 6));
            let set = ir_set(cv);
            SETS[set][variant(SETS[set].len(), mic_frac(cv, mv))]
        };
        assert_eq!(pick(0, 0), "Fender_U89");
        assert_eq!(pick(0, 1), "Fender_MD421");
        assert_eq!(pick(0, 3), "Fender_Royer");
        assert_eq!(pick(0, 5), "Fender_U67");
        assert_eq!(pick(1, 0), "Vox_SM57_API");
        assert_eq!(pick(1, 5), "Vox_U87_GML");
        assert_eq!(pick(2, 0), "Marshall_SM57full_Neve");
        assert_eq!(pick(2, 4), "Marshall_U47_none");
        assert_eq!(pick(3, 0), "Engl_SM57full_GML");
        assert_eq!(pick(3, 1), "Engl_MD421_GML");
        assert_eq!(pick(3, 4), "Engl_U47_none");
        assert_eq!(pick(3, 5), "Engl_U67_GML");
    }

    #[test]
    fn dry_mix_passes_through() {
        let mut fx = Cabinet::new(44100.0);
        fx.set(&params(2.0, 3.0, 0.0, 0.0));
        let mut l: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut r: Vec<f32> = l.iter().map(|v| -v).collect();
        let (l0, r0) = (l.clone(), r.clone());
        fx.process(&mut l, &mut r);
        assert_eq!(l, l0);
        assert_eq!(r, r0);
    }

    #[test]
    fn wet_is_the_scaled_impulse_response() {
        let Some(b) = bank() else { return };
        let mut fx = Cabinet::new(44100.0);
        fx.set(&params(0.0, 0.0, 0.0, 1.0));
        let mut l = vec![0f32; 300];
        let mut r = vec![0f32; 300];
        l[0] = 1.0;
        fx.process(&mut l, &mut r);
        let ir = &b.sets[1][6];
        for i in 0..300 {
            assert_eq!(l[i], ir[i] * WET_GAIN);
        }
        assert!(r.iter().all(|&v| v == 0.0), "channels are independent");
    }

    #[test]
    fn stays_finite_with_changes() {
        let mut fx = Cabinet::new(48000.0);
        fx.set(&params(3.0, 2.0, 1.0, 0.7));
        let mut l: Vec<f32> = (0..4096).map(|i| ((i * 7919) % 200) as f32 / 100.0 - 1.0).collect();
        let mut r = l.clone();
        fx.process(&mut l, &mut r);
        fx.set(&params(1.0, 4.0, 0.3, 1.0));
        fx.process(&mut l, &mut r);
        fx.set(&params(1.0, 4.0, 0.0, 0.2));
        let mut z = vec![0f32; 48000];
        let mut z2 = z.clone();
        fx.process(&mut z, &mut z2);
        assert!(l.iter().chain(&r).chain(&z).all(|v| v.is_finite()));
        // Silence decays to (near) zero without denormal blow-ups.
        assert!(z[47000..].iter().all(|v| v.abs() < 1e-6));
    }
}
