//! 128×64 Monochrome OLED Display for the Maschine Mikro MK3.
//!
//! The Mikro MK3 display is a single 128×64 1-bit OLED screen. Unlike the full Maschine MK3
//! (which uses two 480×272 colour screens over a USB bulk endpoint), the Mikro MK3 screen
//! is driven entirely over HID output reports (report ID `0xe0`).
//!
//! # Framebuffer Layout
//!
//! - **Resolution**: 128 columns × 32 rows (4096 pixels total = 512 bytes 1-bit).
//! - **Buffer size**: 512 bytes (packed in 8-row chunks, 4 chunks total).
//! - **Chunk layout**:
//!   - `chunk = row / 8` (0..4)
//!   - `bit   = 1 << (row % 8)`
//!   - `index = chunk * 128 + col`
//!   - `0` bit = pixel **lit** (OLED white/blue on)
//!   - `1` bit = pixel **dark** (OLED black off)
//!
//! The blank (all-off) state is `0xff` in every byte.
//!
//! # HID Report 0xe0 Packet Structure
//!
//! Pushing a frame requires two 265-byte HID output writes:
//! - **Top half (rows 0–15)**:
//!   `[0xe0, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x02, 0x00]` + `buffer[0..256]`
//! - **Bottom half (rows 16–31)**:
//!   `[0xe0, 0x00, 0x00, 0x02, 0x00, 0x80, 0x00, 0x02, 0x00]` + `buffer[256..512]`

pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 32;
pub const BUFFER_SIZE: usize = 512;

pub const HEADER_HI: [u8; 9] = [0xe0, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x02, 0x00];
pub const HEADER_LO: [u8; 9] = [0xe0, 0x00, 0x00, 0x02, 0x00, 0x80, 0x00, 0x02, 0x00];

/// 5×7 standard ASCII font table (ASCII 32 ' ' through 126 '~').
/// Each character is 5 bytes (columns), bit 0 is top row.
pub const FONT_5X7: [[u8; 5]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00], // 32 ' '
    [0x00, 0x00, 0x5f, 0x00, 0x00], // 33 '!'
    [0x00, 0x07, 0x00, 0x07, 0x00], // 34 '"'
    [0x14, 0x7f, 0x14, 0x7f, 0x14], // 35 '#'
    [0x24, 0x2a, 0x7f, 0x2a, 0x12], // 36 '$'
    [0x23, 0x13, 0x08, 0x64, 0x62], // 37 '%'
    [0x36, 0x49, 0x55, 0x22, 0x50], // 38 '&'
    [0x00, 0x05, 0x03, 0x00, 0x00], // 39 '\''
    [0x00, 0x1c, 0x22, 0x41, 0x00], // 40 '('
    [0x00, 0x41, 0x22, 0x1c, 0x00], // 41 ')'
    [0x14, 0x08, 0x3e, 0x08, 0x14], // 42 '*'
    [0x08, 0x08, 0x3e, 0x08, 0x08], // 43 '+'
    [0x00, 0x50, 0x30, 0x00, 0x00], // 44 ','
    [0x08, 0x08, 0x08, 0x08, 0x08], // 45 '-'
    [0x00, 0x60, 0x60, 0x00, 0x00], // 46 '.'
    [0x20, 0x10, 0x08, 0x04, 0x02], // 47 '/'
    [0x3e, 0x51, 0x49, 0x45, 0x3e], // 48 '0'
    [0x00, 0x42, 0x7f, 0x40, 0x00], // 49 '1'
    [0x42, 0x61, 0x51, 0x49, 0x46], // 50 '2'
    [0x21, 0x41, 0x45, 0x4b, 0x31], // 51 '3'
    [0x18, 0x14, 0x12, 0x7f, 0x10], // 52 '4'
    [0x27, 0x45, 0x45, 0x45, 0x39], // 53 '5'
    [0x3c, 0x4a, 0x49, 0x49, 0x30], // 54 '6'
    [0x01, 0x71, 0x09, 0x05, 0x03], // 55 '7'
    [0x36, 0x49, 0x49, 0x49, 0x36], // 56 '8'
    [0x06, 0x49, 0x49, 0x29, 0x1e], // 57 '9'
    [0x00, 0x36, 0x36, 0x00, 0x00], // 58 ':'
    [0x00, 0x56, 0x36, 0x00, 0x00], // 59 ';'
    [0x08, 0x14, 0x22, 0x41, 0x00], // 60 '<'
    [0x14, 0x14, 0x14, 0x14, 0x14], // 61 '='
    [0x00, 0x41, 0x22, 0x14, 0x08], // 62 '>'
    [0x02, 0x01, 0x51, 0x09, 0x06], // 63 '?'
    [0x32, 0x49, 0x79, 0x41, 0x3e], // 64 '@'
    [0x7e, 0x11, 0x11, 0x11, 0x7e], // 65 'A'
    [0x7f, 0x49, 0x49, 0x49, 0x36], // 66 'B'
    [0x3e, 0x41, 0x41, 0x41, 0x22], // 67 'C'
    [0x7f, 0x41, 0x41, 0x22, 0x1c], // 68 'D'
    [0x7f, 0x49, 0x49, 0x49, 0x41], // 69 'E'
    [0x7f, 0x09, 0x09, 0x09, 0x01], // 70 'F'
    [0x3e, 0x41, 0x49, 0x49, 0x7a], // 71 'G'
    [0x7f, 0x08, 0x08, 0x08, 0x7f], // 72 'H'
    [0x00, 0x41, 0x7f, 0x41, 0x00], // 73 'I'
    [0x20, 0x40, 0x41, 0x3f, 0x01], // 74 'J'
    [0x7f, 0x08, 0x14, 0x22, 0x41], // 75 'K'
    [0x7f, 0x40, 0x40, 0x40, 0x40], // 76 'L'
    [0x7f, 0x02, 0x0c, 0x02, 0x7f], // 77 'M'
    [0x7f, 0x04, 0x08, 0x10, 0x7f], // 78 'N'
    [0x3e, 0x41, 0x41, 0x41, 0x3e], // 79 'O'
    [0x7f, 0x09, 0x09, 0x09, 0x06], // 80 'P'
    [0x3e, 0x41, 0x51, 0x21, 0x5e], // 81 'Q'
    [0x7f, 0x09, 0x19, 0x29, 0x46], // 82 'R'
    [0x46, 0x49, 0x49, 0x49, 0x31], // 83 'S'
    [0x01, 0x01, 0x7f, 0x01, 0x01], // 84 'T'
    [0x3f, 0x40, 0x40, 0x40, 0x3f], // 85 'U'
    [0x1f, 0x20, 0x40, 0x20, 0x1f], // 86 'V'
    [0x3f, 0x40, 0x38, 0x40, 0x3f], // 87 'W'
    [0x63, 0x14, 0x08, 0x14, 0x63], // 88 'X'
    [0x07, 0x08, 0x70, 0x08, 0x07], // 89 'Y'
    [0x61, 0x51, 0x49, 0x45, 0x43], // 90 'Z'
    [0x00, 0x7f, 0x41, 0x41, 0x00], // 91 '['
    [0x02, 0x04, 0x08, 0x10, 0x20], // 92 '\\'
    [0x00, 0x41, 0x41, 0x7f, 0x00], // 93 ']'
    [0x04, 0x02, 0x01, 0x02, 0x04], // 94 '^'
    [0x40, 0x40, 0x40, 0x40, 0x40], // 95 '_'
    [0x00, 0x01, 0x02, 0x04, 0x00], // 96 '`'
    [0x20, 0x54, 0x54, 0x54, 0x78], // 97 'a'
    [0x7f, 0x48, 0x44, 0x44, 0x38], // 98 'b'
    [0x38, 0x44, 0x44, 0x44, 0x20], // 99 'c'
    [0x38, 0x44, 0x44, 0x48, 0x7f], // 100 'd'
    [0x38, 0x54, 0x54, 0x54, 0x18], // 101 'e'
    [0x08, 0x7e, 0x09, 0x01, 0x02], // 102 'f'
    [0x0c, 0x52, 0x52, 0x52, 0x3e], // 103 'g'
    [0x7f, 0x08, 0x04, 0x04, 0x78], // 104 'h'
    [0x00, 0x44, 0x7d, 0x40, 0x00], // 105 'i'
    [0x20, 0x40, 0x44, 0x3d, 0x00], // 106 'j'
    [0x7f, 0x10, 0x28, 0x44, 0x00], // 107 'k'
    [0x00, 0x41, 0x7f, 0x40, 0x00], // 108 'l'
    [0x7c, 0x04, 0x18, 0x04, 0x78], // 109 'm'
    [0x7c, 0x08, 0x04, 0x04, 0x78], // 110 'n'
    [0x38, 0x44, 0x44, 0x44, 0x38], // 111 'o'
    [0x7c, 0x14, 0x14, 0x14, 0x08], // 112 'p'
    [0x08, 0x14, 0x14, 0x18, 0x7c], // 113 'q'
    [0x7c, 0x08, 0x04, 0x04, 0x08], // 114 'r'
    [0x48, 0x54, 0x54, 0x54, 0x20], // 115 's'
    [0x04, 0x3f, 0x44, 0x40, 0x20], // 116 't'
    [0x3c, 0x40, 0x40, 0x20, 0x7c], // 117 'u'
    [0x1c, 0x20, 0x40, 0x20, 0x1c], // 118 'v'
    [0x3c, 0x40, 0x30, 0x40, 0x3c], // 119 'w'
    [0x44, 0x28, 0x10, 0x28, 0x44], // 120 'x'
    [0x0c, 0x50, 0x50, 0x50, 0x3c], // 121 'y'
    [0x44, 0x64, 0x54, 0x4c, 0x44], // 122 'z'
    [0x00, 0x08, 0x36, 0x41, 0x00], // 123 '{'
    [0x00, 0x00, 0x7f, 0x00, 0x00], // 124 '|'
    [0x00, 0x41, 0x36, 0x08, 0x00], // 125 '}'
    [0x08, 0x04, 0x08, 0x10, 0x08], // 126 '~'
];

/// The Mikro MK3's 128×64 monochrome OLED framebuffer.
#[derive(Clone, PartialEq, Eq)]
pub struct Screen {
    pub buf: [u8; BUFFER_SIZE],
    dirty: bool,
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen {
    /// Create a blank (all-dark) screen.
    pub fn new() -> Self {
        Self {
            buf: [0xff; BUFFER_SIZE],
            dirty: true,
        }
    }

    /// Whether pixels have been modified since the last clean mark.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark the screen as clean (e.g. after sending over USB).
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Turn every pixel off (dark black).
    pub fn clear(&mut self) {
        self.buf.fill(0xff);
        self.dirty = true;
    }

    /// Turn every pixel on (white).
    pub fn fill_white(&mut self) {
        self.buf.fill(0x00);
        self.dirty = true;
    }

    /// Set pixel at (`row` 0–63, `col` 0–127). `lit = true` → bright pixel.
    pub fn set(&mut self, row: usize, col: usize, lit: bool) {
        if row >= HEIGHT || col >= WIDTH {
            return;
        }
        let idx = (row / 8) * WIDTH + col;
        let bit = 1u8 << (row % 8);
        if lit {
            self.buf[idx] &= !bit; // 0 = lit
        } else {
            self.buf[idx] |= bit;  // 1 = dark
        }
        self.dirty = true;
    }

    /// Read pixel at (`row`, `col`). Returns `true` if the pixel is lit.
    pub fn get(&self, row: usize, col: usize) -> bool {
        if row >= HEIGHT || col >= WIDTH {
            return false;
        }
        let idx = (row / 8) * WIDTH + col;
        self.buf[idx] & (1 << (row % 8)) == 0
    }

    /// Invert a single pixel.
    pub fn invert(&mut self, row: usize, col: usize) {
        if row >= HEIGHT || col >= WIDTH {
            return;
        }
        let idx = (row / 8) * WIDTH + col;
        let bit = 1u8 << (row % 8);
        self.buf[idx] ^= bit;
        self.dirty = true;
    }

    // ---- Graphics Primitives ---------------------------------------------

    /// Draw a line between (`r0`, `c0`) and (`r1`, `c1`) using Bresenham's algorithm.
    pub fn line(&mut self, mut r0: usize, mut c0: usize, r1: usize, c1: usize, lit: bool) {
        let dr = (r1 as isize - r0 as isize).abs();
        let dc = (c1 as isize - c0 as isize).abs();
        let sr: isize = if r0 < r1 { 1 } else { -1 };
        let sc: isize = if c0 < c1 { 1 } else { -1 };
        let mut err = dc - dr;

        loop {
            self.set(r0, c0, lit);
            if r0 == r1 && c0 == c1 {
                break;
            }
            let e2 = 2 * err;
            if e2 > -dr {
                err -= dr;
                c0 = (c0 as isize + sc) as usize;
            }
            if e2 < dc {
                err += dc;
                r0 = (r0 as isize + sr) as usize;
            }
        }
    }

    /// Draw a horizontal line.
    pub fn hline(&mut self, row: usize, col_start: usize, width: usize, lit: bool) {
        for c in col_start..(col_start + width).min(WIDTH) {
            self.set(row, c, lit);
        }
    }

    /// Draw a vertical line.
    pub fn vline(&mut self, col: usize, row_start: usize, height: usize, lit: bool) {
        for r in row_start..(row_start + height).min(HEIGHT) {
            self.set(r, col, lit);
        }
    }

    /// Fill a solid rectangle of pixels.
    pub fn rect(&mut self, row: usize, col: usize, height: usize, width: usize, lit: bool) {
        for r in row..(row + height).min(HEIGHT) {
            for c in col..(col + width).min(WIDTH) {
                self.set(r, c, lit);
            }
        }
    }

    /// Draw a hollow rectangular border (1 pixel wide).
    pub fn frame_rect(&mut self, row: usize, col: usize, height: usize, width: usize, lit: bool) {
        if height == 0 || width == 0 {
            return;
        }
        self.hline(row, col, width, lit);
        self.hline(row + height - 1, col, width, lit);
        self.vline(col, row, height, lit);
        self.vline(col + width - 1, row, height, lit);
    }

    /// Invert every pixel in a rectangular area (useful for highlighting selections or inverted tabs).
    pub fn invert_rect(&mut self, row: usize, col: usize, height: usize, width: usize) {
        for r in row..(row + height).min(HEIGHT) {
            for c in col..(col + width).min(WIDTH) {
                self.invert(r, c);
            }
        }
    }

    /// Draw a progress bar with a frame and filled inner indicator.
    ///
    /// `percent`: 0.0..=1.0.
    pub fn progress_bar(&mut self, row: usize, col: usize, width: usize, height: usize, percent: f64) {
        if width < 3 || height < 3 {
            return;
        }
        // Outer border
        self.frame_rect(row, col, height, width, true);

        // Inner fill
        let inner_w = width - 2;
        let inner_h = height - 2;
        let filled_w = ((inner_w as f64) * percent.clamp(0.0, 1.0)).round() as usize;

        if filled_w > 0 {
            self.rect(row + 1, col + 1, inner_h, filled_w, true);
        }
        if filled_w < inner_w {
            self.rect(row + 1, col + 1 + filled_w, inner_h, inner_w - filled_w, false);
        }
    }

    /// Draw a standard top status/mode bar across the top of the display (rows 0–9).
    pub fn status_bar(&mut self, left_text: &str, right_text: &str) {
        // Clear top 10 rows
        self.rect(0, 0, 10, WIDTH, false);
        // Left mode/page label
        self.draw_text(1, 1, left_text, true);
        // Right value/status label
        self.draw_text_right(1, WIDTH - 2, right_text, true);
        // Horizontal divider line at row 9
        self.hline(9, 0, WIDTH, true);
    }

    // ---- Text & Bitmap Rendering -----------------------------------------

    fn glyph_of(ch: char) -> [u8; 5] {
        let ch = match ch {
            '–' | '—' => '-',
            '‘' | '’' => '\'',
            '“' | '”' => '"',
            '•' => '*',
            other => other,
        };
        let code = ch as u32;
        if (32..=126).contains(&code) {
            FONT_5X7[(code - 32) as usize]
        } else {
            [0x7f, 0x41, 0x41, 0x41, 0x7f] // Unknown character box
        }
    }

    /// Draw a single ASCII character at (`row`, `col`) using the 5×7 font.
    ///
    /// Returns the horizontal pixel advance (6 pixels).
    pub fn draw_char(&mut self, row: usize, col: usize, ch: char, lit: bool) -> usize {
        let glyph = Self::glyph_of(ch);

        for (dc, &column_byte) in glyph.iter().enumerate() {
            let c = col + dc;
            if c >= WIDTH {
                break;
            }
            for dr in 0..7 {
                let r = row + dr;
                if r >= HEIGHT {
                    break;
                }
                if (column_byte >> dr) & 1 == 1 {
                    self.set(r, c, lit);
                }
            }
        }
        6 // 5 px glyph + 1 px spacing
    }

    /// Draw `text` clipped to `col..col + max_width`, scrolling smoothly left by `scroll_px`
    /// with a 4-space ticker gap when the text exceeds `max_width` pixels.
    pub fn draw_text_scrolled(
        &mut self,
        row: usize,
        col: usize,
        max_width: usize,
        scroll_px: usize,
        text: &str,
        lit: bool,
    ) {
        if row >= HEIGHT || col >= WIDTH || max_width < 5 {
            return;
        }
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return;
        }
        let clip_w = max_width.min(WIDTH - col);
        let text_px = chars.len() * 6 - 1;
        if text_px <= clip_w {
            self.draw_text(row, col, text, lit);
            return;
        }
        let period_px = (chars.len() + 4) * 6;
        let offset = scroll_px % period_px;
        for dx in 0..clip_w {
            let vx = (offset + dx) % period_px;
            let dc = vx % 6;
            if dc == 5 {
                continue;
            }
            let ch = chars.get(vx / 6).copied().unwrap_or(' ');
            let column_byte = Self::glyph_of(ch)[dc];
            for dr in 0..7 {
                let r = row + dr;
                if r >= HEIGHT {
                    break;
                }
                if (column_byte >> dr) & 1 == 1 {
                    self.set(r, col + dx, lit);
                }
            }
        }
    }

    /// Draw a string of text at (`row`, `col`). Returns the total horizontal pixels advanced.
    pub fn draw_text(&mut self, row: usize, col: usize, text: &str, lit: bool) -> usize {
        let mut x = col;
        for ch in text.chars() {
            if x + 5 > WIDTH {
                break;
            }
            x += self.draw_char(row, x, ch, lit);
        }
        x - col
    }

    /// Draw text centered within a column range `col_start..col_start + width`.
    pub fn draw_text_centered(&mut self, row: usize, col_start: usize, width: usize, text: &str, lit: bool) {
        let text_w = text.chars().count() * 6;
        let c = if text_w < width {
            col_start + (width - text_w) / 2
        } else {
            col_start
        };
        self.draw_text(row, c, text, lit);
    }

    /// Draw text right-aligned ending at column `col_end`.
    pub fn draw_text_right(&mut self, row: usize, col_end: usize, text: &str, lit: bool) {
        let text_w = text.chars().count() * 6;
        let col_start = col_end.saturating_sub(text_w);
        self.draw_text(row, col_start, text, lit);
    }

    /// Draw a large digit (0–9) scaled by integer `scale`.
    ///
    /// Useful for large BPM, volume level, or octave displays.
    pub fn draw_big_digit(&mut self, row: usize, col: usize, digit: u8, scale: usize, lit: bool) -> usize {
        let code = (b'0' + (digit % 10)) as u32;
        let glyph = FONT_5X7[(code - 32) as usize];
        let scale = scale.max(1);

        for (dc, &column_byte) in glyph.iter().enumerate() {
            for dr in 0..7 {
                if (column_byte >> dr) & 1 == 1 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            self.set(row + dr * scale + sy, col + dc * scale + sx, lit);
                        }
                    }
                }
            }
        }
        (5 + 1) * scale
    }

    /// Blit a row-major 1-bit bitmap onto the screen at (`row`, `col`).
    ///
    /// `data`: rows * ceil(width / 8) bytes.
    pub fn blit_bitmap(&mut self, row: usize, col: usize, width: usize, height: usize, data: &[u8]) {
        let stride = (width + 7) / 8;
        for r in 0..height {
            for c in 0..width {
                let byte = data.get(r * stride + c / 8).copied().unwrap_or(0);
                let bit = (byte >> (7 - (c % 8))) & 1 == 1;
                self.set(row + r, col + c, bit);
            }
        }
    }

    // ---- HID Packets -----------------------------------------------------

    /// Encode the framebuffer into two 265-byte HID output packets ready to write.
    pub fn packets(&self) -> ([u8; 265], [u8; 265]) {
        let mut hi = [0u8; 265];
        let mut lo = [0u8; 265];
        hi[..9].copy_from_slice(&HEADER_HI);
        hi[9..].copy_from_slice(&self.buf[..256]);
        lo[..9].copy_from_slice(&HEADER_LO);
        lo[9..].copy_from_slice(&self.buf[256..]);
        (hi, lo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_initialization() {
        let s = Screen::new();
        assert_eq!(s.buf.len(), 512);
        assert_eq!(s.buf[0], 0xff, "starts blank (dark)");
        assert!(s.is_dirty());
    }

    #[test]
    fn text_rendering() {
        let mut s = Screen::new();
        let adv = s.draw_text(0, 0, "TEST", true);
        assert_eq!(adv, 24, "4 chars * 6 px = 24 px");
        assert!(s.get(0, 2), "T top bar has lit pixel");
        assert!(s.is_dirty());
    }

    #[test]
    fn primitives_and_inversion() {
        let mut s = Screen::new();
        s.frame_rect(10, 10, 20, 30, true);
        assert!(s.get(10, 10));
        assert!(s.get(29, 39));
        assert!(!s.get(15, 15), "inside is hollow");

        s.rect(10, 10, 20, 30, true);
        assert!(s.get(15, 15), "now filled");

        s.invert_rect(10, 10, 20, 30);
        assert!(!s.get(15, 15), "inverted to dark");
    }

    #[test]
    fn status_and_progress_bars() {
        let mut s = Screen::new();
        s.status_bar("PAD MODE", "120.0 BPM");
        assert!(s.get(9, 0), "divider line at row 9 is lit");

        s.progress_bar(20, 10, 100, 10, 0.5);
        assert!(s.get(20, 10), "frame top-left is lit");
        assert!(s.get(24, 20), "50% mark has inner fill");
        assert!(!s.get(24, 80), "past 50% is unfilled");
    }

    #[test]
    fn big_digit_scaled() {
        let mut s = Screen::new();
        let adv = s.draw_big_digit(10, 10, 5, 2, true);
        assert_eq!(adv, 12, "6 * scale 2 = 12 px");
        assert!(s.get(10, 10));
    }

    #[test]
    fn scrolled_marquee_draws_tail_and_keeps_short_text_static() {
        let mut short_0 = Screen::new();
        short_0.draw_text_scrolled(23, 2, 125, 0, "ASURA kick 1", true);
        let mut short_50 = Screen::new();
        short_50.draw_text_scrolled(23, 2, 125, 50, "ASURA kick 1", true);
        assert!(short_0 == short_50, "short text that fits must stay static regardless of scroll_px");

        // Long text (30 chars): at scroll_px = 60 (10 chars shifted), char 10 ('K') aligns at col 2!
        let mut s = Screen::new();
        s.draw_text_scrolled(23, 2, 125, 60, "1234567890Kick Long Tail End", true);
        let mut expected_head = Screen::new();
        expected_head.draw_text(23, 2, "Kick Long Tail End", true);
        // Columns 2..110 match "Kick Long Tail End" rendered directly at col 2.
        for r in 23..30 {
            for c in 2..110 {
                assert_eq!(s.get(r, c), expected_head.get(r, c), "mismatch at ({r}, {c})");
            }
        }
    }
}
