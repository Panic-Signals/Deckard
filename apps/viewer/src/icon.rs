//! DECKARD's icon: the television the MK3's displays show when DECKARD closes
//! ([`crate::goodbye`]), as an app icon: on macOS's rounded square, a dark room, the
//! old set glowing teal with static on the tube, and deckard across it in the logo font.

use crate::goodbye::{Noise, blur, mix};

/// How far (x, y) is inside a superellipse at (cx, cy) with half sizes (a, b) and power
/// `n`: below 1 inside.
fn shape(x: f32, y: f32, (cx, cy, a, b, n): (f32, f32, f32, f32, f32)) -> f32 {
    ((x - cx).abs() / a).powf(n) + ((y - cy).abs() / b).powf(n)
}

/// The icon, `size` × `size` RGBA (transparent outside the rounded square).
pub fn icon(size: usize) -> Vec<[u8; 4]> {
    let s = size as f32 / 1024.0;
    let c = 512.0 * s;
    // macOS's grid: an 824-point square (corner power ~5), its shadow below it.
    let tile = (c, c, 412.0 * s, 412.0 * s, 5.0);
    let cabinet = (c, c + 6.0 * s, 360.0 * s, 330.0 * s, 9.0);
    let tube = (c, c - 6.0 * s, 300.0 * s, 250.0 * s, 6.0);
    let mut noise = Noise(0x2545_f491);
    let mut px = vec![[0u8; 4]; size * size];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let grain = noise.next();
            let t = shape(fx, fy, tile);
            if t >= 1.0 {
                // A soft shadow under the tile.
                let sh = shape(fx, fy - 14.0 * s, (c, c, 420.0 * s, 420.0 * s, 5.0));
                let a = ((1.0 - (sh - 1.0) * 6.0).clamp(0.0, 1.0) * 70.0) as u8;
                px[y * size + x] = [0, 0, 0, a];
                continue;
            }
            let inside = shape(fx, fy, tube);
            let colour = if inside < 1.0 {
                // The glass: bright in the middle, darker to the corners, scanlines,
                // grain, and static thickening towards a cloud right of centre.
                let (dx, dy) = ((fx - tube.0) / tube.2, (fy - tube.1) / tube.3);
                let d = (dx * dx + dy * dy).sqrt();
                let mut col = mix([66.0, 150.0, 146.0], [18.0, 56.0, 58.0], (d * 0.75).powf(2.0).min(1.0));
                if (y as f32 / (4.0 * s).max(2.0)) as usize % 2 == 1 {
                    col = mix(col, [0.0, 0.0, 0.0], 0.12);
                }
                col = mix(col, [255.0, 255.0, 255.0], (grain - 0.5) * 0.08);
                let (sx, sy) = (dx - 0.12, (dy + 0.05) * 2.2);
                let cloud = (1.0 - (sx * sx + sy * sy)).max(0.0);
                if grain > 1.0 - 0.3 * cloud * cloud {
                    col = mix(col, [235.0, 220.0, 235.0], 0.5 + 0.4 * noise.next());
                }
                mix(col, [10.0, 30.0, 30.0], ((inside - 0.8) / 0.2).clamp(0.0, 1.0) * 0.85)
            } else if shape(fx, fy, (tube.0, tube.1, tube.2 + 16.0 * s, tube.3 + 16.0 * s, tube.4)) < 1.0 {
                // The dark recess round the glass.
                [8.0, 20.0, 22.0]
            } else if shape(fx, fy, cabinet) < 1.0 {
                // The cabinet, lit from the top left, glowing from the glass.
                let (ux, uy) = ((fx - cabinet.0) / cabinet.2, (fy - cabinet.1) / cabinet.3);
                let lit = if uy.abs() > ux.abs() { -uy.signum() } else { -ux.signum() };
                let mut col = mix([18.0, 40.0, 42.0], if lit > 0.0 { [44.0, 82.0, 82.0] } else { [10.0, 26.0, 28.0] }, 0.4);
                let edge = shape(fx, fy, cabinet);
                if edge > 0.93 {
                    col = mix(col, if fy < cabinet.1 { [70.0, 110.0, 108.0] } else { [6.0, 16.0, 18.0] }, 0.6);
                }
                let glow = (1.0 - (inside - 1.0) * 0.8).clamp(0.0, 1.0);
                mix(col, [60.0, 140.0, 134.0], glow * 0.25)
            } else {
                // The room, dark, lit a little by the set; the tile's rim catches light.
                let glow = (1.0 - (inside - 1.0) * 0.35).clamp(0.0, 1.0);
                let mut col = mix([6.0, 14.0, 16.0], [22.0, 50.0, 52.0], glow * 0.6);
                if t > 0.96 {
                    col = mix(col, [40.0, 70.0, 72.0], 0.5);
                }
                col
            };
            let v = colour.map(|v| v.clamp(0.0, 255.0) as u8);
            px[y * size + x] = [v[0], v[1], v[2], 255];
        }
    }
    title(&mut px, size, tube);
    px
}

/// "deckard" across the tube, glowing (as on the displays).
fn title(px: &mut [[u8; 4]], size: usize, (cx, cy, a, _, _): (f32, f32, f32, f32, f32)) {
    let logo = crate::layout::logo_font();
    let Ok(font) = fontdue::Font::from_bytes(logo, fontdue::FontSettings::default()) else { return };
    // The logo font's capital R is the whole film logo: lowercase, as the header spells it.
    let word = "d\u{65}ck\u{61}\u{72}d";
    let width_at = |sz: f32| word.chars().map(|ch| font.metrics(ch, sz).advance_width).sum::<f32>();
    let sz = 64.0 * (a * 2.0 * 0.8) / width_at(64.0).max(1.0);
    let (ascent, descent) = font.horizontal_line_metrics(sz).map_or((sz * 0.8, -sz * 0.2), |l| (l.ascent, l.descent));
    let baseline = cy + (ascent + descent) / 2.0;
    let mut pen = cx - width_at(sz) / 2.0;
    let mut cover = vec![0f32; size * size];
    for ch in word.chars() {
        let (m, bitmap) = font.rasterize(ch, sz);
        let left = (pen + m.xmin as f32).round() as i32;
        let top = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
        for gy in 0..m.height {
            for gx in 0..m.width {
                let (x, y) = (left + gx as i32, top + gy as i32);
                if (0..size as i32).contains(&x) && (0..size as i32).contains(&y) {
                    let c = &mut cover[y as usize * size + x as usize];
                    *c = c.max(bitmap[gy * m.width + gx] as f32 / 255.0);
                }
            }
        }
        pen += m.advance_width;
    }
    let r = ((size as f32 / 1024.0) * 14.0).max(1.0) as usize;
    let mut halo = cover.clone();
    for _ in 0..3 {
        halo = blur(&halo, size, size, r);
    }
    for i in 0..size * size {
        let (c, g) = (cover[i], halo[i]);
        if (c == 0.0 && g < 0.003) || px[i][3] == 0 {
            continue;
        }
        let p = &mut px[i];
        let mut v = [p[0] as f32, p[1] as f32, p[2] as f32];
        v = mix(v, [10.0, 40.0, 42.0], (g * 1.4).min(0.45));
        v = [v[0] + 120.0 * g * 1.6, v[1] + 255.0 * g * 1.6, v[2] + 245.0 * g * 1.6];
        v = mix(v, [228.0, 255.0, 250.0], c);
        *p = [v[0].min(255.0) as u8, v[1].min(255.0) as u8, v[2].min(255.0) as u8, 255];
    }
}

/// `icon(size)` as a PNG file (for the app bundle's icon set and the Linux desktop).
pub fn write_png(path: &std::path::Path, size: usize) -> Result<(), String> {
    let px = icon(size);
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size as u32, size as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(px.as_flattened()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::icon;

    #[test]
    fn a_rounded_square_with_the_teal_set() {
        let n = 256;
        let px = icon(n);
        assert_eq!(px.len(), n * n);
        let at = |x: usize, y: usize| px[y * n + x];
        assert_eq!(at(1, 1)[3], 0, "outside the rounded square: transparent");
        assert_eq!(at(n / 2, 30)[3], 255, "inside: opaque");
        // The glass, off the title (above it): teal, green and blue over red.
        let g = at(n / 2, n / 2 - 40);
        assert!(g[1] > g[0] + 30 && g[2] > g[0] + 30, "teal: {g:?}");
        // The title glows brighter than the glass around it.
        let row: Vec<u32> = (n / 4..3 * n / 4).map(|x| at(x, n / 2).iter().take(3).map(|v| *v as u32).sum()).collect();
        assert!(*row.iter().max().unwrap() > 600, "the letters: near white");
    }
}
