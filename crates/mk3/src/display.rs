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

/// An RGB565 pixel.
pub fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// The transfer that draws rows `y0..y1` of `px` (one display's pixels, row by row).
pub fn encode(display: u8, px: &[u16], y0: usize, y1: usize) -> Vec<u8> {
    let mut f = vec![0x84, 0x00, display, 0x60, 0, 0, 0, 0];
    for v in [0, y0, WIDTH, y1 - y0] {
        f.extend_from_slice(&(v as u16).to_be_bytes());
    }
    let pairs: Vec<[u16; 2]> = px[y0 * WIDTH..y1 * WIDTH].chunks_exact(2).map(|p| [p[0], p[1]]).collect();
    let count = |f: &mut Vec<u8>, cmd: u8, n: usize| f.extend_from_slice(&[cmd, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    let mut i = 0;
    while i < pairs.len() {
        let run = pairs[i..].iter().take_while(|p| **p == pairs[i]).count();
        if run >= 4 {
            count(&mut f, 0x01, run);
            for v in pairs[i] {
                f.extend_from_slice(&v.to_be_bytes());
            }
            i += run;
            continue;
        }
        // Pixels as they are, up to the next run worth repeating.
        let start = i;
        while i < pairs.len() && !(i + 4 <= pairs.len() && pairs[i..i + 4].iter().all(|p| *p == pairs[i])) {
            i += 1;
        }
        count(&mut f, 0x00, i - start);
        for p in &pairs[start..i] {
            for v in p {
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

impl Displays {
    pub fn open() -> Result<Displays, String> {
        let info = nusb::list_devices()
            .wait()
            .map_err(|e| e.to_string())?
            .find(|d| d.vendor_id() == crate::VENDOR && d.product_id() == crate::PRODUCT)
            .ok_or("no Maschine MK3")?;
        let dev = info.open().wait().map_err(|e| e.to_string())?;
        let iface = dev.claim_interface(5).wait().map_err(|e| format!("the displays are in use (NI's agent?): {e}"))?;
        let out = iface.endpoint::<Bulk, Out>(0x04).map_err(|e| e.to_string())?;
        Ok(Displays { _iface: iface, out, shown: [None, None] })
    }

    /// Show `px` (WIDTH × HEIGHT, row by row) on display 0 (left) or 1, sending only
    /// the rows that changed.
    pub fn show(&mut self, display: usize, px: &[u16]) -> Result<(), String> {
        assert_eq!(px.len(), WIDTH * HEIGHT);
        let (y0, y1) = match &self.shown[display] {
            Some(old) => {
                let row_eq = |y: usize| old[y * WIDTH..(y + 1) * WIDTH] == px[y * WIDTH..(y + 1) * WIDTH];
                let Some(y0) = (0..HEIGHT).find(|&y| !row_eq(y)) else { return Ok(()) };
                let y1 = (0..HEIGHT).rev().find(|&y| !row_eq(y)).unwrap_or(y0) + 1;
                (y0, y1)
            }
            None => (0, HEIGHT),
        };
        self.out.submit(encode(display as u8, px, y0, y1).into());
        let done = self.out.wait_next_complete(Duration::from_secs(1)).ok_or("display write timed out")?;
        if let Err(e) = done.status {
            self.shown[display] = None;
            return Err(e.to_string());
        }
        self.shown[display] = Some(px.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
