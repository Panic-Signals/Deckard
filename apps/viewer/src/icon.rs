//! DECKARD's icons. The television the MK3's displays show when DECKARD closes
//! ([`crate::goodbye`]), as an app icon: on macOS's rounded square, a dark room, the
//! old set glowing teal with static on the tube, and deckard across it in the logo font.
//! Or the logo's D, white on black. Preferences ▸ General picks the one the window, the
//! taskbar and the Dock show ([`Style`], [`apply`]).

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

/// The logo's D, white on a black square, `size` × `size` RGBA. Drawn larger and scaled
/// down, centred on the letter's ink: placed in whole pixels at the small sizes Windows
/// shows, it would sit a pixel or two off.
pub fn letter(size: usize) -> Vec<[u8; 4]> {
    let mut px = vec![[0, 0, 0, 255]; size * size];
    let logo = crate::layout::logo_font();
    let Ok(font) = fontdue::Font::from_bytes(logo, fontdue::FontSettings::default()) else { return px };
    let over = (1024 / size.max(1)).clamp(1, 16);
    let big = size * over;
    // The D's own shape, not its advance, fills the square (the letter leans).
    let room = big as f32 * 0.72;
    let at_100 = font.metrics('d', 100.0);
    let sz = 100.0 * room / at_100.width.max(at_100.height).max(1) as f32;
    let (m, bitmap) = font.rasterize('d', sz);
    // Where the ink is: a rasterized letter's box can have empty or faint rows at its edges.
    let ink = |x: usize, y: usize| bitmap[y * m.width + x] >= 128;
    let rows: Vec<usize> = (0..m.height).filter(|&y| (0..m.width).any(|x| ink(x, y))).collect();
    let columns: Vec<usize> = (0..m.width).filter(|&x| (0..m.height).any(|y| ink(x, y))).collect();
    let (Some(&top), Some(&bottom), Some(&left), Some(&right)) = (rows.first(), rows.last(), columns.first(), columns.last()) else {
        return px;
    };
    let x_at = (big as isize - (right + left + 1) as isize) / 2;
    let y_at = (big as isize - (bottom + top + 1) as isize) / 2;
    let mut sum = vec![0u32; size * size];
    for gy in 0..m.height {
        for gx in 0..m.width {
            let (x, y) = (gx as isize + x_at, gy as isize + y_at);
            if (0..big as isize).contains(&x) && (0..big as isize).contains(&y) {
                sum[y as usize / over * size + x as usize / over] += u32::from(bitmap[gy * m.width + gx]);
            }
        }
    }
    let samples = (over * over) as u32;
    for (p, s) in px.iter_mut().zip(sum) {
        let v = (s / samples) as u8;
        *p = [v, v, v, 255];
    }
    px
}

/// [`letter`] on macOS's rounded square, where the television sits (the Dock's icons
/// share that grid): transparent outside it, the edge smoothed.
pub fn letter_tile(size: usize) -> Vec<[u8; 4]> {
    let s = size as f32 / 1024.0;
    let c = 512.0 * s;
    let tile = (c, c, 412.0 * s, 412.0 * s, 5.0);
    let inner = ((824.0 * s).round() as usize).max(1);
    let at = size.saturating_sub(inner) / 2;
    let d = letter(inner);
    let mut px = vec![[0u8; 4]; size * size];
    // About a pixel, in the shape's measure at its edge, to smooth it over.
    let feather = 5.0 / (412.0 * s).max(1.0);
    for y in 0..size {
        for x in 0..size {
            let t = shape(x as f32 + 0.5, y as f32 + 0.5, tile);
            let a = ((1.0 - t) / feather + 0.5).clamp(0.0, 1.0);
            if a == 0.0 {
                continue;
            }
            let (ix, iy) = (x.wrapping_sub(at), y.wrapping_sub(at));
            let v = if ix < inner && iy < inner { d[iy * inner + ix][0] } else { 0 };
            px[y * size + x] = [v, v, v, (a * 255.0).round() as u8];
        }
    }
    px
}

/// Which icon DECKARD shows (Preferences ▸ General ▸ Icon).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Style {
    /// The television (a new install's).
    #[default]
    Television,
    /// The logo's D, white on black.
    Letter,
}

impl Style {
    pub const ALL: [Style; 2] = [Style::Television, Style::Letter];

    pub fn label(self) -> &'static str {
        match self {
            Style::Television => "Television",
            Style::Letter => "D",
        }
    }

    /// As saved in the settings and given to `--write-icon`.
    pub fn key(self) -> &'static str {
        match self {
            Style::Television => "television",
            Style::Letter => "letter",
        }
    }

    pub fn from_key(key: &str) -> Option<Style> {
        Style::ALL.into_iter().find(|s| s.key() == key)
    }

    /// The icon, `size` × `size` RGBA: the D on the rounded square on macOS (as Dock
    /// icons are), the whole square elsewhere.
    pub fn pixels(self, size: usize) -> Vec<[u8; 4]> {
        match self {
            Style::Television => icon(size),
            Style::Letter if cfg!(target_os = "macos") => letter_tile(size),
            Style::Letter => letter(size),
        }
    }

    /// The window's icon, for egui.
    pub fn icon_data(self) -> eframe::egui::IconData {
        eframe::egui::IconData { rgba: self.pixels(256).into_iter().flatten().collect(), width: 256, height: 256 }
    }
}

/// Shows `style` as the running program's icon: the window's and the taskbar's (Windows,
/// Linux), the Dock's (macOS). The program file's own icon, and shortcuts to it, stay as
/// they were installed.
pub fn apply(ctx: &eframe::egui::Context, style: Style) {
    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Icon(Some(std::sync::Arc::new(style.icon_data()))));
    platform::apply(style);
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Style;

    /// The Dock's icon (winit's window icon does nothing on macOS).
    pub fn apply(style: Style) {
        use objc2::{AnyThread as _, MainThreadMarker};
        use objc2_app_kit::{NSApplication, NSBitmapImageRep, NSDeviceRGBColorSpace, NSImage};
        use objc2_foundation::NSSize;
        let Some(mtm) = MainThreadMarker::new() else { return };
        let n = 512;
        let px = style.pixels(n);
        // SAFETY: given no planes, the image keeps a buffer of its own, n × n × 4 bytes,
        // filled below (premultiplied, as AppKit takes it by default).
        unsafe {
            let Some(rep) = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                std::ptr::null_mut(),
                n as isize,
                n as isize,
                8,
                4,
                true,
                false,
                NSDeviceRGBColorSpace,
                (n * 4) as isize,
                32,
            ) else {
                return;
            };
            let data = rep.bitmapData();
            if data.is_null() {
                return;
            }
            let out = std::slice::from_raw_parts_mut(data, n * n * 4);
            for (o, p) in out.chunks_exact_mut(4).zip(&px) {
                let a = u16::from(p[3]);
                let pre = |v: u8| (u16::from(v) * a / 255) as u8;
                o.copy_from_slice(&[pre(p[0]), pre(p[1]), pre(p[2]), p[3]]);
            }
            let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(n as f64, n as f64));
            image.addRepresentation(&rep);
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::Style;

    /// The taskbar's and Alt+Tab's icons: winit's window icon is only the small one.
    pub fn apply(style: Style) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateIconFromResourceEx, GetSystemMetrics, ICON_BIG, ICON_SMALL, LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON, SendMessageW,
            WM_SETICON,
        };
        // SAFETY: a getter; DECKARD's window is the active one while its preferences change
        // and as it starts.
        let window = unsafe { GetActiveWindow() };
        if window.is_null() {
            return;
        }
        // Big first, then small: given the big one alone, Windows scales it down badly.
        for (which, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            // SAFETY: a getter.
            let size = unsafe { GetSystemMetrics(metric) }.clamp(16, 256) as usize;
            // Drawn at the size asked for; Windows reads a PNG as an icon resource.
            let Ok(mut png) = super::png_bytes(&style.pixels(size), size) else { return };
            // SAFETY: the bytes are a whole PNG and outlive the call, which copies them.
            let icon = unsafe {
                CreateIconFromResourceEx(png.as_mut_ptr(), png.len() as u32, 1, 0x0003_0000, size as i32, size as i32, LR_DEFAULTCOLOR)
            };
            if icon.is_null() {
                return;
            }
            // SAFETY: the window and the icon are both live handles.
            unsafe { SendMessageW(window, WM_SETICON, which as usize, icon as isize) };
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    /// The window icon egui sets is the whole of it.
    pub fn apply(_style: super::Style) {}
}

fn png_bytes(px: &[[u8; 4]], size: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, size as u32, size as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(px.as_flattened()).map_err(|e| e.to_string())?;
    w.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// `style`'s icon as a PNG file (for the app bundle's icon set, the Linux desktop and the
/// Windows installer's icon).
pub fn write_png(path: &std::path::Path, size: usize, style: Style) -> Result<(), String> {
    std::fs::write(path, png_bytes(&style.pixels(size), size)?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{Style, icon, letter, letter_tile};

    /// Windows shows the small sizes as they are, so each must be centred for itself.
    #[test]
    fn the_letter_is_centred_at_every_size() {
        for n in [16, 20, 24, 32, 40, 48, 64, 256] {
            let px = letter(n);
            let lit = |x: usize, y: usize| px[y * n + x][0] >= 128;
            let rows: Vec<usize> = (0..n).filter(|&y| (0..n).any(|x| lit(x, y))).collect();
            let columns: Vec<usize> = (0..n).filter(|&x| (0..n).any(|y| lit(x, y))).collect();
            let (top, bottom) = (rows[0], n - 1 - rows[rows.len() - 1]);
            let (left, right) = (columns[0], n - 1 - columns[columns.len() - 1]);
            assert!(top.abs_diff(bottom) <= 1, "{n}: {top} above, {bottom} below");
            assert!(left.abs_diff(right) <= 1, "{n}: {left} left, {right} right");
        }
    }

    #[test]
    fn the_letter_is_white_on_black_at_every_size() {
        for n in [16, 32, 48, 256] {
            let px = letter(n);
            assert_eq!(px.len(), n * n);
            assert!(px.iter().all(|p| p[3] == 255), "{n}: opaque all over");
            for corner in [0, n - 1, n * n - n, n * n - 1] {
                assert_eq!(px[corner], [0, 0, 0, 255], "{n}: black in the corners");
            }
            assert!(px.iter().any(|p| p[0] > 200 && p[0] == p[1] && p[1] == p[2]), "{n}: the white letter");
        }
    }

    #[test]
    fn the_letter_tile_is_rounded_like_the_television() {
        let n = 256;
        let (d, tv) = (letter_tile(n), icon(n));
        assert_eq!(d[n + 1][3], 0, "outside the rounded square: transparent");
        assert_eq!(d[30 * n + n / 2][3], 255, "inside: opaque");
        assert_eq!(d[n / 2 * n + 4][3], 0, "left of the square: transparent");
        assert_eq!(tv[n / 2 * n + 40][3], 255);
        assert_eq!(d[n / 2 * n + 40][3], 255, "where the television's square is");
        assert!(d.iter().any(|p| p[0] > 200), "the white letter");
    }

    #[test]
    fn styles_round_trip_through_their_keys() {
        for s in Style::ALL {
            assert_eq!(Style::from_key(s.key()), Some(s));
        }
        assert_eq!(Style::from_key("nonsense"), None);
        assert_eq!(Style::default(), Style::Television);
    }

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
