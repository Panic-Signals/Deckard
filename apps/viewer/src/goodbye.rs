//! What the MK3's displays show once DECKARD has closed: on each, an old CRT television
//! glowing teal, a little static on the tube, and DECKARD across it in the logo font
//! (built in, or `logo.ttf` in the settings folder).

/// Both displays side by side, as the display thread takes them.
pub const WIDTH: usize = 960;
pub const HEIGHT: usize = 272;

/// The picture, `WIDTH` × `HEIGHT` RGBA.
pub fn picture() -> Vec<[u8; 4]> {
    let logo = Some(crate::layout::logo_font());
    let font =
        logo.as_deref().and_then(|b| fontdue::Font::from_bytes(b, fontdue::FontSettings::default()).ok()).map(|f| (f, true)).or_else(
            || {
                let defs = eframe::egui::FontDefinitions::default();
                let data = defs.font_data.get("Ubuntu-Light")?;
                fontdue::Font::from_bytes(&data.font[..], fontdue::FontSettings::default()).ok().map(|f| (f, false))
            },
        );
    let mut px = vec![[0u8, 0, 0, 255]; WIDTH * HEIGHT];
    for screen in 0..2 {
        television(&mut px, screen * WIDTH / 2, screen as u32 + 1);
    }
    if let Some((font, logo)) = &font {
        // The logo font's capital R is the whole "BLADE RUNNER" logo: lowercase, the r
        // as U+0072 (as the app's header spells it).
        let word = if *logo { "d\u{65}ck\u{61}\u{72}d" } else { "DECKARD" };
        for screen in 0..2 {
            title(&mut px, font, word, screen * WIDTH / 2);
        }
    }
    px
}

/// Where the tube is on a display (its centre and half sizes) and how square its
/// corners are (a superellipse's power: higher is squarer).
const TUBE: (f32, f32, f32, f32, f32) = (240.0, 134.0, 196.0, 108.0, 7.0);

/// How far (x, y) is inside the tube's outline: below 1 inside, 1 on the edge.
fn tube_shape(x: f32, y: f32, grow: f32) -> f32 {
    let (cx, cy, a, b, n) = TUBE;
    ((x - cx).abs() / (a + grow)).powf(n) + ((y - cy).abs() / (b + grow)).powf(n)
}

/// A small deterministic noise source (xorshift), so the picture is the same each time.
pub(crate) struct Noise(pub(crate) u32);

impl Noise {
    pub(crate) fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 & 0xffff) as f32 / 65535.0
    }
}

pub(crate) fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// One television filling a 480 × 272 display whose left edge is `x0`.
fn television(px: &mut [[u8; 4]], x0: usize, seed: u32) {
    let mut noise = Noise(0x9e37_79b9 ^ seed.wrapping_mul(0x85eb_ca6b));
    let room = [6.0, 16.0, 18.0];
    let cabinet = [18.0, 40.0, 42.0];
    let cabinet_light = [40.0, 76.0, 76.0];
    let cabinet_dark = [12.0, 30.0, 32.0];
    let glass = [60.0, 140.0, 136.0];
    let (cx, cy, _, _, _) = TUBE;
    for y in 0..HEIGHT {
        for x in 0..WIDTH / 2 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let grain = noise.next();
            let inside = tube_shape(fx, fy, 0.0);
            let colour = if inside < 1.0 {
                // The glass: brightest in the middle, darker to the corners, a darker band
                // low down, scanlines and grain.
                let (dx, dy) = ((fx - cx) / 196.0, (fy - cy) / 108.0);
                let d = (dx * dx + dy * dy).sqrt();
                let mut c = mix(glass, [20.0, 58.0, 60.0], (d * 0.7).powf(2.0).min(1.0));
                if (0.42..0.62).contains(&dy) {
                    c = mix(c, [40.0, 110.0, 108.0], 0.25);
                }
                if y % 2 == 1 {
                    c = mix(c, [0.0, 0.0, 0.0], 0.10);
                }
                c = mix(c, [255.0, 255.0, 255.0], (grain - 0.5) * 0.08);
                // Static: specks that thicken towards a cloud right of centre.
                let (sx, sy) = ((fx - cx - 18.0) / 120.0, (fy - cy + 4.0) / 46.0);
                let cloud = (1.0 - (sx * sx + sy * sy)).max(0.0);
                if grain > 1.0 - 0.32 * cloud * cloud {
                    let pink = noise.next();
                    c = mix(c, [235.0, 225.0 - 25.0 * pink, 235.0], 0.55 + 0.4 * noise.next());
                }
                // The edge of the glass falls off into shadow.
                mix(c, [10.0, 30.0, 30.0], ((inside - 0.82) / 0.18).clamp(0.0, 1.0) * 0.8)
            } else if tube_shape(fx, fy, 7.0) < 1.0 {
                // The dark recess round the glass.
                [8.0, 20.0, 22.0]
            } else if (14.0..466.0).contains(&fx) && (4.0..268.0).contains(&fy) {
                // The cabinet: a bevel, lit from the top left, sloping in to the recess.
                let edge = (fx - 14.0).min(466.0 - fx).min(fy - 4.0).min(268.0 - fy);
                let mut c = cabinet;
                if edge < 3.0 {
                    c = if fx - 14.0 < 3.0 || fy - 4.0 < 3.0 { cabinet_light } else { cabinet_dark };
                } else {
                    // The inner slope: lighter on the top and left faces.
                    let (ux, uy) = ((fx - cx) / 226.0, (fy - cy) / 132.0);
                    let lit = if uy.abs() > ux.abs() { -uy.signum() } else { -ux.signum() };
                    c = mix(c, if lit > 0.0 { cabinet_light } else { cabinet_dark }, 0.35);
                }
                // The glow of the glass on the cabinet.
                let glow = (1.0 - (tube_shape(fx, fy, 0.0) - 1.0) * 0.9).clamp(0.0, 1.0);
                mix(c, [60.0, 140.0, 134.0], glow * 0.25)
            } else {
                // The dark room, lit a little by the set.
                let glow = (1.0 - (tube_shape(fx, fy, 0.0) - 1.0) * 0.5).clamp(0.0, 1.0);
                mix(room, [20.0, 46.0, 48.0], glow * 0.5)
            };
            let c = colour.map(|v| v.clamp(0.0, 255.0) as u8);
            px[y * WIDTH + x0 + x] = [c[0], c[1], c[2], 255];
        }
    }
}

/// `word` on the tube of the display at `x0`, glowing: a soft halo, then the letters.
fn title(px: &mut [[u8; 4]], font: &fontdue::Font, word: &str, x0: usize) {
    let (cx, cy, a, _, _) = TUBE;
    // Size it to most of the glass's width.
    let width_at = |size: f32| word.chars().map(|ch| font.metrics(ch, size).advance_width).sum::<f32>();
    let size = 64.0 * (a * 2.0 * 0.78) / width_at(64.0).max(1.0);
    let line = font.horizontal_line_metrics(size);
    let (ascent, descent) = line.map_or((size * 0.8, -size * 0.2), |l| (l.ascent, l.descent));
    let baseline = cy + (ascent + descent) / 2.0;
    let mut pen = cx - width_at(size) / 2.0;
    // The letters' coverage, then a blurred copy for the halo.
    let (w, h) = (WIDTH / 2, HEIGHT);
    let mut cover = vec![0f32; w * h];
    for ch in word.chars() {
        let (m, bitmap) = font.rasterize(ch, size);
        let left = (pen + m.xmin as f32).round() as i32;
        let top = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
        for gy in 0..m.height {
            for gx in 0..m.width {
                let (x, y) = (left + gx as i32, top + gy as i32);
                if (0..w as i32).contains(&x) && (0..h as i32).contains(&y) {
                    let c = &mut cover[y as usize * w + x as usize];
                    *c = c.max(bitmap[gy * m.width + gx] as f32 / 255.0);
                }
            }
        }
        pen += m.advance_width;
    }
    let mut halo = cover.clone();
    for _ in 0..3 {
        halo = blur(&halo, w, h, 6);
    }
    for y in 0..h {
        for x in 0..w {
            let (c, g) = (cover[y * w + x], halo[y * w + x]);
            if c == 0.0 && g < 0.003 {
                continue;
            }
            let p = &mut px[y * WIDTH + x0 + x];
            let mut v = [p[0] as f32, p[1] as f32, p[2] as f32];
            // Darken under the halo a touch so the letters stand off the glass, glow
            // pale cyan, then the letters near white.
            v = mix(v, [10.0, 40.0, 42.0], (g * 1.4).min(0.45));
            v = [v[0] + 120.0 * g * 1.6, v[1] + 255.0 * g * 1.6, v[2] + 245.0 * g * 1.6];
            v = mix(v, [228.0, 255.0, 250.0], c);
            *p = [v[0].min(255.0) as u8, v[1].min(255.0) as u8, v[2].min(255.0) as u8, 255];
        }
    }
}

/// A box blur of radius `r`, across then down.
pub(crate) fn blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let n = (2 * r + 1) as f32;
    let mut across = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (lo, hi) = (x.saturating_sub(r), (x + r).min(w - 1));
            across[y * w + x] = src[y * w + lo..=y * w + hi].iter().sum::<f32>() / n;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        let (lo, hi) = (y.saturating_sub(r), (y + r).min(h - 1));
        for x in 0..w {
            out[y * w + x] = (lo..=hi).map(|yy| across[yy * w + x]).sum::<f32>() / n;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn two_televisions_with_teal_glass() {
        let px = super::picture();
        assert_eq!(px.len(), super::WIDTH * super::HEIGHT);
        // The middle of each tube's glass is teal (green and blue well over red), the
        // corners of the room dark.
        for x0 in [0, 480] {
            let [r, g, b, _] = px[30 * super::WIDTH + x0 + 240];
            assert!(g > r + 40 && b > r + 40, "teal glass at {x0}: {r} {g} {b}");
            let [r, g, b, _] = px[x0 + 2];
            assert!(r < 40 && g < 60 && b < 60, "dark room at {x0}: {r} {g} {b}");
        }
    }
}
