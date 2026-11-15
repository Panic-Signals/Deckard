//! One monotone LCD. The ground and the pixels are a real panel's pair; pads and
//! groups take the mode's one colour. Each mode is a dark field with bright pixels.

/// What the LCD draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    /// The panel, where no pixel is on.
    pub paper: [u8; 3],
    /// A pixel that is on.
    pub ink: [u8; 3],
    /// Pads and groups: the mode's one colour.
    pub hue: [u8; 3],
    /// That colour as an MK3 palette index.
    pub led: i64,
}

/// `yellow`, `red` or `blue`. Anything else is the amber panel.
pub fn mode(name: &str) -> &'static str {
    match name {
        "red" => "red",
        "blue" => "blue",
        _ => "yellow",
    }
}

/// `yellow` is amber segments on a black field (a negative amber STN). `red` is the
/// same in red. `blue` is white segments on the deep blue of a backlit module.
pub fn look(name: &str) -> Look {
    // Paper, ink, the pad colour, its palette index.
    let (paper, ink, hue, led) = match mode(name) {
        // Negative amber: segments about #ffb000, the field a warm black.
        "yellow" => ([0x1a, 0x14, 0x08], [0xff, 0xb0, 0x00], [0xff, 0xb0, 0x00], 3),
        // Negative red: segments a warm red, not a pure primary.
        "red" => ([0x14, 0x06, 0x06], [0xff, 0x24, 0x12], [0xff, 0x24, 0x12], 0),
        // Blue-backlit module: field #0a1a5c, segments a blue-white.
        _ => ([0x0a, 0x1a, 0x5c], [0xd6, 0xe6, 0xff], [0x4c, 0x7c, 0xff], 10),
    };
    Look { paper, ink, hue, led }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lcd_modes_are_dark_fields() {
        for name in ["yellow", "red", "blue", "white", "nope"] {
            let look = look(name);
            let y = |c: [u8; 3]| 0.2126 * f32::from(c[0]) + 0.7152 * f32::from(c[1]) + 0.0722 * f32::from(c[2]);
            assert!(y(look.paper) < y(look.ink), "{name}: the field is darker than the pixels");
        }
        assert_eq!(look("yellow").ink, [0xff, 0xb0, 0x00]);
        assert_eq!(look("white").paper, look("yellow").paper, "a removed mode is the amber panel");
        assert_eq!(mode("nope"), "yellow");
    }
}
