//! The recordings the Hi-hat's Memory and Hybrid engines play, read from the user's
//! Maschine install (docs/synth-hihat.md §5). Copied from the Cymbal port's loader
//! (`synth/cymbal/res.rs`) and extended to the six cymbal recordings.
//!
//! `NI::DSP::UTIL::loadSampleFromResources(id, block)` decodes compiled-in `WAVE`
//! resources (`resource_data_WAVE_<id>`): 3 (23 noises × 131072, the modal noise layer),
//! 4 (131072 samples of pink noise) and 5–10 (the six Memory "Source" recordings A–F,
//! 131072 samples each, `IFFTCymbalCore`). All are mono 16-bit 44.1 kHz PCM; a sample
//! `n` becomes `max(n/32767, −1)`. They are Native Instruments' recordings, so DECKARD
//! neither ships nor reads them: only the null tests load them, from the Maschine binary
//! named by `MASCHINE_RES_BINARY`. Otherwise they are DECKARD's own stand-ins
//! (`synth::noise`: noise shaped like each recording).

use std::fs::File;
use std::io::{Error, ErrorKind, Read, Result, Seek, SeekFrom};
use std::path::Path;
use std::sync::OnceLock;

/// Samples per recording (and per noise in [`noises`]).
pub(super) const NOISE_LEN: usize = 131_072;

struct Res {
    #[allow(dead_code)]
    noises: Box<[f32]>,
    #[allow(dead_code)]
    pink: Box<[f32]>,
    cymbals: [Box<[f32]>; 6],
}

fn res() -> Option<&'static Res> {
    static R: OnceLock<Option<Res>> = OnceLock::new();
    R.get_or_init(|| {
        let path = std::env::var_os("MASCHINE_RES_BINARY")?;
        load(Path::new(&path)).ok()
    })
    .as_ref()
}

/// Resource WAVE 3: 23 noises × [`NOISE_LEN`] samples.
#[allow(dead_code)]
pub(super) fn noises() -> Option<&'static [f32]> {
    Some(res().map_or_else(crate::synth::noise::modal_noises, |r| &r.noises[..]))
}

/// Resource WAVE 4: [`NOISE_LEN`] samples of pink noise.
#[allow(dead_code)]
pub(super) fn pink() -> Option<&'static [f32]> {
    Some(res().map_or_else(crate::synth::noise::pink, |r| &r.pink[..]))
}

/// Memory source `k` (0..6, resource WAVE 5 + k).
pub(super) fn cymbal(k: usize) -> Option<&'static [f32]> {
    Some(res().map_or_else(|| crate::synth::noise::cymbal(k), |r| &r.cymbals[k.min(5)][..]))
}

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

/// The WAVE resources `ids` from the (fat) Mach-O at `path`, found by their symbols.
fn load(path: &Path) -> Result<Res> {
    let mut f = File::open(path)?;
    let flen = f.metadata()?.len() as usize;
    let head = read_at(&mut f, 0, 4096.min(flen))?;
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
    let mut addr = [None; 11];
    for s in syms.as_chunks::<16>().0 {
        let x = u32_at(s, 0, false) as usize;
        let end = strs[x.min(strs.len())..].iter().position(|&b| b == 0).map_or(strs.len(), |e| x + e);
        if let Some(name) = strs.get(x..end)
            && let Some(id) = name.strip_prefix(b"__ZL20resource_data_WAVE_").or_else(|| name.strip_prefix(b"__ZL21resource_data_WAVE_"))
            && let Some(id) = std::str::from_utf8(id).ok().and_then(|t| t.parse::<usize>().ok())
            && (3..=10).contains(&id)
        {
            addr[id] = Some(u64_at(s, 8));
        }
    }
    let mut wav = |a: u64| -> Result<Box<[f32]>> {
        let &(vm, _, fo) = segs.iter().find(|s| a >= s.0 && a + 12 <= s.0 + s.1).ok_or_else(|| bad("resource outside the segments"))?;
        let off = base + fo + (a - vm);
        let h = read_at(&mut f, off, 12)?;
        if &h[0..4] != b"RIFF" || &h[8..12] != b"WAVE" {
            return Err(bad("not a WAVE resource"));
        }
        let size = u32_at(&h, 4, false) as usize + 8;
        let w = read_at(&mut f, off, size)?;
        let mut i = 12;
        while i + 8 <= w.len() {
            let sz = u32_at(&w, i + 4, false) as usize;
            if &w[i..i + 4] == b"data" {
                let d = w.get(i + 8..i + 8 + sz).ok_or_else(|| bad("short data chunk"))?;
                let inv = 1.0f32 / 32767.0;
                return Ok(d.as_chunks::<2>().0.iter().map(|c| (f32::from(i16::from_le_bytes(*c)) * inv).max(-1.0)).collect());
            }
            i += 8 + sz + (sz & 1);
        }
        Err(bad("no data chunk"))
    };
    let mut get = |id: usize, len: usize| -> Result<Box<[f32]>> {
        let w = wav(addr[id].ok_or_else(|| bad("WAVE resource missing"))?)?;
        if w.len() != len {
            return Err(bad("unexpected resource length"));
        }
        Ok(w)
    };
    let noises = get(3, 23 * NOISE_LEN)?;
    let pink = get(4, NOISE_LEN)?;
    let mut cymbals: [Box<[f32]>; 6] = Default::default();
    for (k, c) in cymbals.iter_mut().enumerate() {
        *c = get(5 + k, NOISE_LEN)?;
    }
    Ok(Res { noises, pink, cymbals })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recordings_load() {
        let Some(c) = cymbal(0) else { return };
        assert_eq!(c.len(), NOISE_LEN);
        for k in 0..6 {
            let c = cymbal(k).unwrap();
            assert!(c.iter().all(|x| (-1.0..=1.0).contains(x)) && c.iter().any(|x| x.abs() > 0.01));
        }
        assert_eq!(noises().unwrap().len(), 23 * NOISE_LEN);
    }
}
