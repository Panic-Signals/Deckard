//! The MK3's two 480×272 displays (bulk interface 5, endpoint 0x04; learned with
//! `examples/mk3-display-test.rs`). A transfer draws a rectangle of one display:
//!
//! - a 16-byte header: `84 00 <display> 60 00 00 00 00`, then x, y, width, height
//!   (u16 BE);
//! - commands, each four bytes: `00 nn nn nn` + nn pixel pairs (transmit),
//!   `01 nn nn nn` + one pixel pair repeated nn times, `02 nn nn nn` (skip nn pairs,
//!   leaving them as they are; NI's software uses it for small changes),
//!   `03 00 00 00` (blit),
//!   `40 00 00 00` (end).
//!
//! Pixels are RGB565, big-endian.

use nusb::MaybeFuture;
use nusb::transfer::{Bulk, Out};
use std::time::Duration;

pub const WIDTH: usize = 480;
pub const HEIGHT: usize = 272;

/// An RGB565 pixel (each channel's top bits).
pub fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// The RGB565 pixel nearest (`r`, `g`, `b`), undithered (for a picture's fine detail).
pub fn rgb565_rounded(r: u8, g: u8, b: u8) -> u16 {
    let q = |v: u8, levels: u32| (u32::from(v) * levels + 127) / 255;
    ((q(r, 31) << 11) | (q(g, 63) << 5) | q(b, 31)) as u16
}

/// A 64×64 blue-noise threshold map, 0..255 (`tools/blue_noise.py`).
static BLUE_NOISE: &[u8; 64 * 64] = include_bytes!("blue_noise_64.bin");

/// The RGB565 pixel at (`x`, `y`) on a display: each channel rounded up or down to its
/// nearest levels by a blue-noise threshold fixed to the place, so a shade between two
/// levels shows as a fine even mix of them (their average its own) instead of a band of
/// the lower one. Fixed to the place, a pixel that doesn't change doesn't flicker; one
/// colour for all three channels keeps the grain grey rather than coloured. 0 and 255
/// stay as they are.
pub fn rgb565_dithered(x: usize, y: usize, r: u8, g: u8, b: u8) -> u16 {
    let d = u32::from(BLUE_NOISE[(y % 64) * 64 + x % 64]);
    // floor(v × levels / 255 + (d + 0.5) / 256), in integers.
    let q = |v: u8, levels: u32| (u32::from(v) * levels * 512 + (2 * d + 1) * 255) / (255 * 512);
    ((q(r, 31) << 11) | (q(g, 63) << 5) | q(b, 31)) as u16
}

/// The transfer that draws rows `y0..y1` of `px` (one display's pixels, row by row).
pub fn encode(display: u8, px: &[u16], y0: usize, y1: usize) -> Vec<u8> {
    encode_diff(display, px, None, y0, y1)
}

/// Like [`encode`], skipping runs of pixel pairs that already match `old` (`0x02`).
pub fn encode_diff(display: u8, px: &[u16], old: Option<&[u16]>, y0: usize, y1: usize) -> Vec<u8> {
    let mut f = vec![0x84, 0x00, display, 0x60, 0, 0, 0, 0];
    for v in [0, y0, WIDTH, y1 - y0] {
        f.extend_from_slice(&(v as u16).to_be_bytes());
    }
    let band = &px[y0 * WIDTH..y1 * WIDTH];
    let old_band = old.map(|o| &o[y0 * WIDTH..y1 * WIDTH]);
    let pair_count = band.len() / 2;
    let pair = |b: &[u16], i: usize| [b[i * 2], b[i * 2 + 1]];
    let eq_old = |i: usize| old_band.is_some_and(|o| pair(o, i) == pair(band, i));
    let count = |f: &mut Vec<u8>, cmd: u8, n: usize| f.extend_from_slice(&[cmd, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    let mut i = 0;
    while i < pair_count {
        if eq_old(i) {
            let start = i;
            i += 1;
            while i < pair_count && eq_old(i) {
                i += 1;
            }
            count(&mut f, 0x02, i - start);
            continue;
        }
        let p0 = pair(band, i);
        if i + 4 <= pair_count && (1..4).all(|k| pair(band, i + k) == p0) {
            let mut end = i + 4;
            while end < pair_count && pair(band, end) == p0 {
                end += 1;
            }
            count(&mut f, 0x01, end - i);
            for v in p0 {
                f.extend_from_slice(&v.to_be_bytes());
            }
            i = end;
            continue;
        }
        // Pixels as they are, up to the next run worth repeating or skipping.
        let start = i;
        i += 1;
        while i < pair_count {
            if (i + 3 <= pair_count && eq_old(i) && eq_old(i + 1) && eq_old(i + 2)) || (i + 2 == pair_count && eq_old(i) && eq_old(i + 1)) {
                break;
            }
            let p = pair(band, i);
            if i + 4 <= pair_count && (1..4).all(|k| pair(band, i + k) == p) {
                break;
            }
            i += 1;
        }
        count(&mut f, 0x00, i - start);
        for j in start..i {
            for v in pair(band, j) {
                f.extend_from_slice(&v.to_be_bytes());
            }
        }
    }
    f.extend_from_slice(&[0x03, 0, 0, 0, 0x40, 0, 0, 0]);
    f
}

/// Applies transfers to the two displays' pixels, as the MK3 does: the reverse of
/// [`encode`], for decoding what other software sends.
pub struct Decoder {
    pub px: [Vec<u16>; 2],
    buf: Vec<u8>,
}

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder { px: [vec![0; WIDTH * HEIGHT], vec![0; WIDTH * HEIGHT]], buf: Vec::new() }
    }
}

impl Decoder {
    /// Feed bytes written to the display endpoint (transfers may be split or joined);
    /// returns the displays blitted, in order.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<usize> {
        self.buf.extend_from_slice(bytes);
        let mut shown = Vec::new();
        loop {
            match self.transfer() {
                Some(Ok((used, display))) => {
                    self.buf.drain(..used);
                    shown.extend(display);
                }
                Some(Err(())) => {
                    // Not a transfer we know: skip to the next header.
                    let next = self.buf[1..].windows(2).position(|w| w == [0x84, 0x00]).map_or(self.buf.len(), |i| i + 1);
                    self.buf.drain(..next);
                }
                None => return shown,
            }
        }
    }

    /// The transfer at the start of `buf`: bytes used and the display blitted, or
    /// `None` if it isn't all there yet.
    fn transfer(&mut self) -> Option<Result<(usize, Option<usize>), ()>> {
        let b = &self.buf;
        if b.len() < 16 {
            return None;
        }
        if b[0] != 0x84 || b[1] != 0 || b[2] > 1 {
            return Some(Err(()));
        }
        let display = b[2] as usize;
        let u16_at = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]) as usize;
        let (x, y, w, h) = (u16_at(8), u16_at(10), u16_at(12), u16_at(14));
        if w == 0 || x + w > WIDTH || y + h > HEIGHT {
            return Some(Err(()));
        }
        let mut i = 16;
        let mut n = 0; // pixels written into the rectangle
        let mut blit = None;
        let mut writes = Vec::new(); // None: left as it is
        loop {
            let cmd = b.get(i..i + 4)?;
            let count = (cmd[1] as usize) << 16 | (cmd[2] as usize) << 8 | cmd[3] as usize;
            i += 4;
            match cmd[0] {
                0x00 => {
                    let data = b.get(i..i + count * 4)?;
                    writes.extend(data.chunks_exact(2).map(|p| Some(u16::from_be_bytes([p[0], p[1]]))));
                    i += count * 4;
                }
                0x01 => {
                    let p = b.get(i..i + 4)?;
                    let pair = [Some(u16::from_be_bytes([p[0], p[1]])), Some(u16::from_be_bytes([p[2], p[3]]))];
                    for _ in 0..count {
                        writes.extend(pair);
                    }
                    i += 4;
                }
                0x02 => writes.extend(std::iter::repeat_n(None, count * 2)),
                0x03 => blit = Some(display),
                0x40 => break,
                _ => return Some(Err(())),
            }
        }
        let px = &mut self.px[display];
        for v in writes {
            if n >= w * h {
                break;
            }
            if let Some(v) = v {
                px[(y + n / w) * WIDTH + x + n % w] = v;
            }
            n += 1;
        }
        Some(Ok((i, blit)))
    }
}

/// The displays of the first MK3 connected (NI's agent must not hold them).
pub struct Displays {
    _iface: nusb::Interface,
    out: nusb::Endpoint<Bulk, Out>,
    /// What each display shows now, if known.
    shown: [Option<Vec<u16>>; 2],
}

/// What opening a controller says when none is plugged in.
pub const NO_CONTROLLER: &str = "no Maschine MK3 or Maschine+";

/// Whether an error from [`Displays::open`] means a driver other than WinUSB has the
/// displays' interface (Windows: NI's, see `scripts/mk3-driver.ps1`).
pub fn wrong_driver(error: &str) -> bool {
    error.contains("incompatible driver")
}

impl Displays {
    pub fn open() -> Result<Displays, String> {
        let devices: Vec<_> = nusb::list_devices().wait().map_err(|e| e.to_string())?.filter(|d| d.vendor_id() == crate::VENDOR).collect();
        let (info, model) =
            crate::MODELS.iter().find_map(|m| devices.iter().find(|d| d.product_id() == m.product).map(|d| (d, m))).ok_or(NO_CONTROLLER)?;
        let dev = info.open().wait().map_err(|e| e.to_string())?;
        let iface =
            dev.claim_interface(model.display_interface).wait().map_err(|e| format!("the displays are in use (NI's agent?): {e}"))?;
        let out = iface.endpoint::<Bulk, Out>(0x04).map_err(|e| e.to_string())?;
        Ok(Displays { _iface: iface, out, shown: [None, None] })
    }

    /// Show `px` (WIDTH × HEIGHT, row by row) on display 0 (left) or 1, sending only
    /// what changed since the previous frame.
    pub fn show(&mut self, display: usize, px: &[u16]) -> Result<(), String> {
        assert_eq!(px.len(), WIDTH * HEIGHT);
        let old = self.shown[display].as_deref();
        let (y0, y1) = match old {
            Some(old) => {
                let row_eq = |y: usize| old[y * WIDTH..(y + 1) * WIDTH] == px[y * WIDTH..(y + 1) * WIDTH];
                let Some(y0) = (0..HEIGHT).find(|&y| !row_eq(y)) else { return Ok(()) };
                let y1 = (0..HEIGHT).rev().find(|&y| !row_eq(y)).unwrap_or(y0) + 1;
                (y0, y1)
            }
            None => (0, HEIGHT),
        };
        self.out.submit(encode_diff(display as u8, px, old, y0, y1).into());
        let done = self.out.wait_next_complete(Duration::from_secs(1)).ok_or("display write timed out")?;
        if let Err(e) = done.status {
            self.shown[display] = None;
            return Err(e.to_string());
        }
        match &mut self.shown[display] {
            Some(prev) => prev.copy_from_slice(px),
            slot @ None => *slot = Some(px.to_vec()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dithering_keeps_the_average_and_the_ends() {
        let split = |p: u16| (p >> 11, (p >> 5) & 63, p & 31);
        assert_eq!(rgb565_dithered(5, 9, 0, 0, 0), 0);
        assert_eq!(rgb565_dithered(5, 9, 255, 255, 255), 0xffff);
        // Over a tile, a shade's levels average to the shade itself (not the level under it).
        for v in [21u8, 100, 130, 200, 250] {
            let (mut r, mut g) = (0.0, 0.0);
            for y in 0..64 {
                for x in 0..64 {
                    let (pr, pg, _) = split(rgb565_dithered(x, y, v, v, v));
                    r += f64::from(pr) / 31.0 * 255.0;
                    g += f64::from(pg) / 63.0 * 255.0;
                }
            }
            let (r, g) = (r / 4096.0, g / 4096.0);
            assert!((r - f64::from(v)).abs() < 0.6 && (g - f64::from(v)).abs() < 0.6, "{v}: {r:.2} {g:.2}");
        }
        // Each threshold once per 16 pixels: an even mix, every value used.
        let mut n = [0u32; 256];
        BLUE_NOISE.iter().for_each(|&d| n[usize::from(d)] += 1);
        assert!(n.iter().all(|&c| c == 16));
    }

    #[test]
    fn decoding_reverses_encoding() {
        let px: Vec<u16> = (0..WIDTH * HEIGHT).map(|i| if i % 7 < 4 { 5 } else { i as u16 }).collect();
        let f = encode(1, &px, 3, 40);
        let mut d = Decoder::default();
        // Split mid-transfer: nothing shown until the rest arrives.
        assert!(d.feed(&f[..100]).is_empty());
        assert_eq!(d.feed(&f[100..]), vec![1]);
        assert_eq!(d.px[1][3 * WIDTH..40 * WIDTH], px[3 * WIDTH..40 * WIDTH]);
        assert!(d.px[1][..3 * WIDTH].iter().all(|&v| v == 0));
    }

    #[test]
    fn skips_leave_pixels_as_they_are() {
        let mut d = Decoder::default();
        d.px[0].fill(9);
        let mut f = vec![0x84, 0, 0, 0x60, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0xe0, 0, 1];
        f.extend_from_slice(&[0x02, 0, 0, 2, 0x00, 0, 0, 1, 0, 7, 0, 8, 0x03, 0, 0, 0, 0x40, 0, 0, 0]);
        assert_eq!(d.feed(&f), vec![0]);
        assert_eq!(d.px[0][..7], [9, 9, 9, 9, 7, 8, 9]);
    }

    #[test]
    fn runs_repeat_and_the_rest_is_sent_as_is() {
        let mut px = vec![rgb565(255, 0, 0); WIDTH * HEIGHT];
        px[2] = 1;
        let f = encode(0, &px, 0, 1);
        assert_eq!(&f[..16], &[0x84, 0, 0, 0x60, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0xe0, 0, 1]);
        // Pair 0 (red, red) alone, pair 1 (1, red), then 238 red pairs repeated.
        assert_eq!(&f[16..20], &[0x00, 0, 0, 2]);
        assert_eq!(&f[28..32], &[0x01, 0, 0, 238]);
        assert_eq!(&f[f.len() - 8..], &[0x03, 0, 0, 0, 0x40, 0, 0, 0]);
    }

    #[test]
    fn diff_skips_unchanged_pairs_on_playhead_move() {
        let mut old = vec![0u16; WIDTH * HEIGHT];
        for y in 40..250 {
            for x in 0..WIDTH {
                old[y * WIDTH + x] = ((x * 7 + y * 13) & 0xffff) as u16;
            }
            old[y * WIDTH + 100] = 0xffff;
        }
        let mut next = old.clone();
        for y in 40..250 {
            next[y * WIDTH + 100] = ((100 * 7 + y * 13) & 0xffff) as u16;
            next[y * WIDTH + 101] = 0xffff;
        }
        let full = encode(0, &next, 40, 250);
        let diff = encode_diff(0, &next, Some(&old), 40, 250);
        assert!(diff.len() < full.len() / 20, "diff ({} B) should be <5% of full ({} B)", diff.len(), full.len());
        let mut d = Decoder::default();
        d.px[0].copy_from_slice(&old);
        assert_eq!(d.feed(&diff), vec![0]);
        assert_eq!(d.px[0], next);
    }
}
