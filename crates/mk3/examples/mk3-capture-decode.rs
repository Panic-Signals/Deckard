//! Turn a capture of what NI's software sent the MK3 (`tools/mk3-capture/run.sh`) into
//! PNGs of both screens side by side (960×272), named by the time since the capture
//! started: each screen once settled (unchanged for 0.3 s), or every change with
//! `--all`.
//! `cargo run -p deckard-mk3 --example mk3-capture-decode -- capture.bin OUT_DIR`

use deckard_mk3::display::{Decoder, HEIGHT, WIDTH};
use std::collections::HashMap;

fn main() {
    let all = std::env::args().any(|a| a == "--all");
    let mut args = std::env::args().skip(1).filter(|a| a != "--all");
    let input = args.next().expect("capture file");
    let dir = args.next().unwrap_or_else(|| "mk3-frames".into());
    std::fs::create_dir_all(&dir).expect("output dir");
    let data = std::fs::read(&input).expect("read capture");
    // One decoder per pipe: only the display pipe ever blits.
    let mut decoders: HashMap<u8, Decoder> = HashMap::new();
    let (mut i, mut start, mut frames, mut last) = (0, None, 0, Vec::new());
    // The newest change, written once the next comes 0.3 s later (or the capture ends).
    let mut pending: Option<(u64, Vec<u8>)> = None;
    let mut write = |t: u64, rgb: &[u8]| {
        let path = format!("{dir}/{:08.3}s.png", t as f64 / 1e6);
        let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&path).unwrap()), (WIDTH * 2) as u32, HEIGHT as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.write_header().unwrap().write_image_data(rgb).unwrap();
        frames += 1;
    };
    while i + 13 <= data.len() {
        let us = u64::from_le_bytes(data[i..i + 8].try_into().unwrap());
        let pipe = data[i + 8];
        let len = u32::from_le_bytes(data[i + 9..i + 13].try_into().unwrap()) as usize;
        let Some(bytes) = data.get(i + 13..i + 13 + len) else { break };
        i += 13 + len;
        let t0 = *start.get_or_insert(us);
        let d = decoders.entry(pipe).or_default();
        if d.feed(bytes).is_empty() {
            continue;
        }
        let mut rgb = vec![0u8; WIDTH * 2 * HEIGHT * 3];
        for (half, px) in d.px.iter().enumerate() {
            for (n, &v) in px.iter().enumerate() {
                let o = ((n / WIDTH) * WIDTH * 2 + half * WIDTH + n % WIDTH) * 3;
                let (r, g, b) = ((v >> 11) as u8, (v >> 5 & 0x3f) as u8, (v & 0x1f) as u8);
                rgb[o..o + 3].copy_from_slice(&[r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2]);
            }
        }
        if rgb == last {
            continue;
        }
        if let Some((t, p)) = pending.take()
            && (all || us - t0 >= t + 300_000)
        {
            write(t, &p);
        }
        last = rgb.clone();
        pending = Some((us - t0, rgb));
    }
    if let Some((t, p)) = pending {
        write(t, &p);
    }
    let pipes: Vec<_> = decoders.keys().collect();
    println!("{frames} frames written to {dir} (pipes seen: {pipes:?})");
}
