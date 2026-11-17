//! Golden images for the widgets: each case drawn on a theme at a width, compared with
//! `crates/surface/tests/golden/<case>-<theme>-<width>.png`.
//!
//! Make or refresh them with `DECKARD_GOLDEN=update cargo test -p deckard-surface widget::`
//! and look at every new or changed PNG before committing it: they are the widget's
//! look, and a test only proves the look didn't change since.

use super::{BAND_H, CELL_W, Canvas, Look, Rect};
use crate::Surface;
use crate::screen::{HEIGHT, Picture, WIDTH};

/// The themes every widget is checked on: DECKARD's default, and the two-tone LCD.
pub const THEMES: [&str; 2] = ["maschine", "lcd"];

/// The widths a widget spans, from `min_cells` knob cells to four.
pub fn widths(min_cells: i32) -> Vec<i32> {
    (min_cells.max(1)..=4).map(|n| n * CELL_W).collect()
}

/// The look of `theme`.
pub fn look(theme: &str) -> Look {
    let mut s = Surface::new();
    s.set_theme(theme);
    assert_eq!(s.theme(), theme, "theme {theme} loads");
    s.widget_look().expect("a theme")
}

/// Draws a widget `w` wide by the band's height (offset from the screen's corner, to
/// catch drawing outside the box) and returns the box's pixels, after checking that
/// nothing outside the box was touched.
pub fn render(look: &Look, w: i32, focus: Option<&str>, draw: impl FnOnce(&mut Canvas)) -> Vec<[u8; 4]> {
    let r = Rect { x: 16, y: 16, w, h: BAND_H };
    let mut pic = Picture::new(look.background.rgb);
    {
        let mut c = Canvas::new(&mut pic, r, look);
        c.focus = focus.map(str::to_string);
        draw(&mut c);
    }
    let px = pic.into_pixels();
    let [br, bg, bb] = look.background.rgb;
    let field = [br, bg, bb, 255];
    let mut outside = Vec::new();
    let mut out = Vec::with_capacity((r.w * r.h) as usize);
    for y in 0..HEIGHT as i32 {
        for x in 0..WIDTH as i32 {
            let p = px[y as usize * WIDTH + x as usize];
            if x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h {
                out.push(p);
            } else if p != field {
                outside.push((x - r.x, y - r.y));
            }
        }
    }
    assert!(
        outside.is_empty(),
        "{} pixels drawn outside the widget's box, at (box-relative) {:?}",
        outside.len(),
        &outside[..outside.len().min(8)]
    );
    out
}

fn path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden").join(format!("{name}.png"))
}

pub(crate) fn write_png(path: &std::path::Path, w: u32, h: u32, px: &[[u8; 4]]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let f = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&px.concat()).unwrap();
}

fn read_png(path: &std::path::Path) -> Option<(u32, u32, Vec<[u8; 4]>)> {
    let f = std::fs::File::open(path).ok()?;
    let mut d = png::Decoder::new(std::io::BufReader::new(f));
    d.set_transformations(png::Transformations::EXPAND);
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    let px = buf[..info.buffer_size()].chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
    Some((info.width, info.height, px))
}

/// Compares case `case` (drawn by `draw`) on every theme at every width from
/// `min_cells`, with touch focus on `focus` if given, against its golden images.
pub fn check(case: &str, min_cells: i32, focus: Option<&str>, draw: impl Fn(&mut Canvas)) {
    let update = std::env::var("DECKARD_GOLDEN").is_ok_and(|v| v == "update");
    let mut failures = Vec::new();
    for theme in THEMES {
        let look = look(theme);
        for w in widths(min_cells) {
            let px = render(&look, w, focus, &draw);
            let name = format!("{case}-{theme}-{w}");
            let p = path(&name);
            if update {
                write_png(&p, w as u32, BAND_H as u32, &px);
                continue;
            }
            let Some((gw, gh, golden)) = read_png(&p) else {
                failures.push(format!("{name}: no golden image (make it with DECKARD_GOLDEN=update)"));
                continue;
            };
            if (gw, gh) != (w as u32, BAND_H as u32) {
                failures.push(format!("{name}: golden is {gw}x{gh}"));
                continue;
            }
            // (A little room for rounding; a real change moves far more.)
            let off = px.iter().zip(&golden).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 3)).count();
            if off > px.len() / 500 {
                let got = std::env::temp_dir().join(format!("deckard-golden-{name}.png"));
                write_png(&got, w as u32, BAND_H as u32, &px);
                failures.push(format!("{name}: {off} pixels differ (drawn: {})", got.display()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A picture of the input lit: whether drawing with touch focus on `input` changes the
/// picture from drawing without (on Maschine's theme, at `w`).
pub fn focus_changes(w: i32, input: &str, draw: impl Fn(&mut Canvas)) -> bool {
    let look = look("maschine");
    render(&look, w, None, &draw) != render(&look, w, Some(input), &draw)
}
