//! Audio file loading: WAV and AIFF/AIFC, to stereo f32.

use std::path::Path;

/// A decoded sample, always stereo.
#[derive(Debug, Clone, Default)]
pub struct Sample {
    pub rate: u32,
    pub frames: Vec<[f32; 2]>,
}

impl Sample {
    pub fn load(path: &Path) -> Result<Sample, String> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "wav" => load_wav(path),
            "aif" | "aiff" | "aifc" => {
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                parse_aiff(&bytes)
            }
            "ogg" => load_ogg(path),
            "ncw" => Err("NCW (NI compressed wave) is not supported".into()),
            other => Err(format!("unsupported sample format {other:?}")),
        }
    }
}

fn to_stereo(channels: usize, samples: impl Iterator<Item = f32>) -> Vec<[f32; 2]> {
    let ch = channels.max(1);
    let mut out = Vec::new();
    let mut frame = Vec::with_capacity(ch);
    for s in samples {
        frame.push(s);
        if frame.len() == ch {
            out.push(if ch == 1 { [frame[0], frame[0]] } else { [frame[0], frame[1]] });
            frame.clear();
        }
    }
    out
}

/// Ogg Vorbis (Maschine's previews of sounds and groups): mono doubled, more than two
/// channels down to the first two.
fn load_ogg(path: &Path) -> Result<Sample, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut r = lewton::inside_ogg::OggStreamReader::new(std::io::BufReader::new(file)).map_err(|e| e.to_string())?;
    let rate = r.ident_hdr.audio_sample_rate;
    let mut frames = Vec::new();
    while let Some(packet) = r.read_dec_packet_generic::<Vec<Vec<f32>>>().map_err(|e| e.to_string())? {
        let Some(left) = packet.first() else { continue };
        let right = packet.get(1).unwrap_or(left);
        frames.extend(left.iter().zip(right).map(|(l, r)| [*l, *r]));
    }
    Ok(Sample { rate, frames })
}

fn load_wav(path: &Path) -> Result<Sample, String> {
    parse_wav(&std::fs::read(path).map_err(|e| e.to_string())?)
}

/// Decode little-endian PCM (8-bit unsigned, 16/24/32-bit signed) or IEEE float.
fn pcm_le(data: &[u8], width: usize, float: bool) -> Result<Vec<f32>, String> {
    Ok(match (width, float) {
        (4, true) => data.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect(),
        (8, true) => data.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c) as f32).collect(),
        (1, false) => data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
        (2..=4, false) => data
            .chunks_exact(width)
            .map(|c| {
                let mut v: i32 = 0;
                for &byte in c.iter().rev() {
                    v = (v << 8) | byte as i32;
                }
                let shift = 32 - 8 * width as u32;
                ((v << shift) >> shift) as f32 / (1i64 << (8 * width - 1)) as f32
            })
            .collect(),
        _ => return Err(format!("unsupported WAV sample width {width} bytes")),
    })
}

/// Parse a RIFF WAVE file. Tolerates odd `fmt ` sizes and extra chunks, which
/// some library samples have.
pub fn parse_wav(bytes: &[u8]) -> Result<Sample, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF WAVE file".into());
    }
    let mut fmt: Option<(u16, usize, u32, usize)> = None; // format, channels, rate, bits
    let mut data: Option<&[u8]> = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8).saturating_add(len).min(bytes.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                let mut format = u16::from_le_bytes([body[0], body[1]]);
                if format == 0xfffe && body.len() >= 26 {
                    // WAVE_FORMAT_EXTENSIBLE: the sub-format GUID starts with the format tag
                    format = u16::from_le_bytes([body[24], body[25]]);
                }
                let channels = u16::from_le_bytes([body[2], body[3]]) as usize;
                let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                let bits = u16::from_le_bytes([body[14], body[15]]) as usize;
                fmt = Some((format, channels, rate, bits));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    let (format, channels, rate, bits) = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    let float = match format {
        1 => false,
        3 => true,
        f => return Err(format!("unsupported WAV format tag {f:#x}")),
    };
    if channels == 0 {
        return Err("zero channels".into());
    }
    let samples = pcm_le(data, bits.div_ceil(8), float)?;
    Ok(Sample { rate, frames: to_stereo(channels, samples.into_iter()) })
}

/// 80-bit IEEE 754 extended (big-endian), as used for the AIFF sample rate.
fn extended_to_f64(b: &[u8]) -> f64 {
    let exp = ((((b[0] as i32) & 0x7f) << 8) | b[1] as i32) - 16383;
    let mant = u64::from_be_bytes(b[2..10].try_into().unwrap());
    let v = mant as f64 * 2f64.powi(exp - 63);
    if b[0] & 0x80 != 0 { -v } else { v }
}

/// Parse an AIFF or AIFC file (uncompressed PCM, `sowt` little-endian, or `fl32`/`fl64`).
pub fn parse_aiff(bytes: &[u8]) -> Result<Sample, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"FORM" {
        return Err("not an IFF file".into());
    }
    let aifc = match &bytes[8..12] {
        b"AIFF" => false,
        b"AIFC" => true,
        _ => return Err("not AIFF".into()),
    };
    let (mut channels, mut bits, mut rate, mut comp) = (0usize, 0usize, 0f64, *b"NONE");
    let mut data: Option<&[u8]> = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_be_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
        match id {
            b"COMM" if body.len() >= 18 => {
                channels = u16::from_be_bytes([body[0], body[1]]) as usize;
                bits = u16::from_be_bytes([body[6], body[7]]) as usize;
                rate = extended_to_f64(&body[8..18]);
                if aifc && body.len() >= 22 {
                    comp.copy_from_slice(&body[18..22]);
                }
            }
            b"SSND" if body.len() >= 8 => {
                let offset = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
                data = body.get(8 + offset..);
            }
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    let data = data.ok_or("no SSND chunk")?;
    if channels == 0 {
        return Err("no COMM chunk".into());
    }
    let samples: Vec<f32> = match &comp {
        b"NONE" | b"twos" | b"sowt" => {
            let le = &comp == b"sowt";
            let width = bits.div_ceil(8);
            if !(1..=4).contains(&width) {
                return Err(format!("unsupported bit depth {bits}"));
            }
            data.chunks_exact(width)
                .map(|c| {
                    let mut v: i32 = 0;
                    for i in 0..width {
                        let byte = if le { c[width - 1 - i] } else { c[i] };
                        v = (v << 8) | byte as i32;
                    }
                    // sign-extend from `width` bytes, then scale to [-1, 1)
                    let shift = 32 - 8 * width as u32;
                    ((v << shift) >> shift) as f32 / (1i64 << (8 * width - 1)) as f32
                })
                .collect()
        }
        b"fl32" | b"FL32" => data.as_chunks::<4>().0.iter().map(|c| f32::from_be_bytes(*c)).collect(),
        b"fl64" | b"FL64" => data.as_chunks::<8>().0.iter().map(|c| f64::from_be_bytes(*c) as f32).collect(),
        other => return Err(format!("unsupported AIFC compression {:?}", String::from_utf8_lossy(other))),
    };
    Ok(Sample { rate: rate.round() as u32, frames: to_stereo(channels, samples.into_iter()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aiff(comm_extra: &[u8], form: &[u8; 4], channels: u16, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut comm = Vec::new();
        comm.extend(channels.to_be_bytes());
        comm.extend(((data.len() / (bits as usize / 8) / channels as usize) as u32).to_be_bytes());
        comm.extend(bits.to_be_bytes());
        // 44100 as 80-bit extended
        comm.extend([0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]);
        comm.extend(comm_extra);
        let mut body = form.to_vec();
        body.extend(b"COMM");
        body.extend((comm.len() as u32).to_be_bytes());
        body.extend(&comm);
        body.extend(b"SSND");
        body.extend(((data.len() + 8) as u32).to_be_bytes());
        body.extend([0; 8]);
        body.extend(data);
        let mut out = b"FORM".to_vec();
        out.extend((body.len() as u32).to_be_bytes());
        out.extend(body);
        out
    }

    #[test]
    fn wav_with_long_fmt_chunk() {
        // fmt chunk of 18 bytes (cbSize present), 16-bit stereo, then a LIST chunk.
        let mut fmt = vec![1, 0, 2, 0];
        fmt.extend(48000u32.to_le_bytes());
        fmt.extend((48000u32 * 4).to_le_bytes());
        fmt.extend([4, 0, 16, 0, 0, 0]);
        let data = [0x00, 0x40, 0x00, 0xc0];
        let mut body = b"WAVE".to_vec();
        for (id, chunk) in [(b"fmt ", &fmt[..]), (b"LIST", &[1u8, 2, 3][..]), (b"data", &data[..])] {
            body.extend(id);
            body.extend((chunk.len() as u32).to_le_bytes());
            body.extend(chunk);
            if chunk.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut wav = b"RIFF".to_vec();
        wav.extend((body.len() as u32).to_le_bytes());
        wav.extend(body);
        let s = parse_wav(&wav).unwrap();
        assert_eq!(s.rate, 48000);
        assert_eq!(s.frames, vec![[0.5, -0.5]]);
    }

    #[test]
    fn aiff_16bit_mono() {
        let s = parse_aiff(&aiff(&[], b"AIFF", 1, 16, &[0x40, 0x00, 0xc0, 0x00])).unwrap();
        assert_eq!(s.rate, 44100);
        assert_eq!(s.frames, vec![[0.5, 0.5], [-0.5, -0.5]]);
    }

    #[test]
    fn aifc_sowt_24bit_stereo() {
        // little-endian 24-bit: L = 0x400000 (0.5), R = 0xC00000 (-0.5)
        let s = parse_aiff(&aiff(b"sowt", b"AIFC", 2, 24, &[0, 0, 0x40, 0, 0, 0xc0])).unwrap();
        assert_eq!(s.frames, vec![[0.5, -0.5]]);
    }
}

#[cfg(test)]
mod ogg_tests {
    use super::Sample;

    /// Maschine's previews of sounds and groups are Ogg Vorbis (`.previews/x.mxsnd.ogg`).
    #[test]
    fn a_maschine_preview_loads() {
        let p = std::path::Path::new("/Users/Shared/Velvet Lounge Library/Sounds/Drumsynth/.previews/Clap Coldwire.mxsnd.ogg");
        if !p.exists() {
            return;
        }
        let s = Sample::load(p).unwrap();
        assert!(s.rate >= 22_050);
        assert!(s.frames.len() > s.rate as usize / 10, "{} frames", s.frames.len());
        assert!(s.frames.iter().any(|f| f[0].abs() > 0.01));
    }
}
