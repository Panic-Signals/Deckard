//! A theme reduced to the colours a person actually picks.
//!
//! The controller already loads any `themes/<name>.lua`. This is the small form of that
//! file: ten colours, written so it builds on Maschine, which is what Preferences saves,
//! imports and exports. A theme someone wrote by hand (with its own `chase`, for
//! instance) is still a normal theme file; this module only generates and reads the
//! short form.

/// The colours the editor changes. Everything else in the stylesheet is derived from
/// these, the way `themes/palenight.lua` fills Maschine's colour names from a short
/// palette.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub background: [u8; 3],
    pub text: [u8; 3],
    pub weak: [u8; 3],
    pub accent: [u8; 3],
    pub record: [u8; 3],
    pub on_colour: [u8; 3],
    pub button: [u8; 3],
    pub button_text: [u8; 3],
    pub selected: [u8; 3],
    pub selected_text: [u8; 3],
}

/// One built-in palette. Its file is generated, so it shows up as `themes/<id>.lua`
/// without a second copy of the stylesheet to keep in step.
pub struct Stock {
    pub id: &'static str,
    pub name: &'static str,
    pub look: Look,
}

/// Themes that ship besides the hand-written ones (`maschine`, `lcd`, `palenight`,
/// `palenight_blue_green`, `solarized_dark`). Twelve of these plus those five is the
/// set that comes with the software.
pub fn stock() -> &'static [Stock] {
    STOCK
}

const STOCK: &[Stock] = &[
    palette("nord", "Nord", [46, 52, 64], [236, 239, 244], [129, 161, 193], [136, 192, 208], [191, 97, 106], [67, 76, 94], [136, 192, 208]),
    palette(
        "gruvbox",
        "Gruvbox",
        [40, 40, 40],
        [235, 219, 178],
        [168, 153, 132],
        [250, 189, 47],
        [251, 73, 52],
        [80, 73, 69],
        [250, 189, 47],
    ),
    palette(
        "dracula",
        "Dracula",
        [40, 42, 54],
        [248, 248, 242],
        [98, 114, 164],
        [189, 147, 249],
        [255, 85, 85],
        [68, 71, 90],
        [255, 121, 198],
    ),
    palette(
        "catppuccin",
        "Catppuccin",
        [30, 30, 46],
        [205, 214, 244],
        [166, 173, 200],
        [137, 180, 250],
        [243, 139, 168],
        [49, 50, 68],
        [203, 166, 247],
    ),
    palette(
        "tokyo_night",
        "Tokyo Night",
        [26, 27, 38],
        [192, 202, 245],
        [86, 95, 137],
        [125, 207, 255],
        [247, 118, 142],
        [36, 40, 59],
        [187, 154, 247],
    ),
    palette(
        "one_dark",
        "One Dark",
        [40, 44, 52],
        [171, 178, 191],
        [92, 99, 112],
        [97, 175, 239],
        [224, 108, 117],
        [55, 60, 70],
        [198, 120, 221],
    ),
    palette(
        "rose_pine",
        "Rosé Pine",
        [25, 23, 36],
        [224, 222, 244],
        [144, 140, 170],
        [156, 207, 216],
        [235, 111, 146],
        [38, 35, 58],
        [196, 167, 231],
    ),
    palette(
        "kanagawa",
        "Kanagawa",
        [31, 31, 40],
        [220, 215, 186],
        [147, 138, 125],
        [126, 156, 166],
        [195, 64, 67],
        [45, 45, 58],
        [255, 160, 102],
    ),
    palette(
        "everforest",
        "Everforest",
        [45, 53, 59],
        [211, 198, 170],
        [131, 146, 138],
        [167, 192, 128],
        [230, 126, 128],
        [55, 66, 72],
        [127, 187, 179],
    ),
    palette(
        "monokai",
        "Monokai",
        [39, 40, 34],
        [248, 248, 242],
        [117, 113, 94],
        [166, 226, 46],
        [249, 38, 114],
        [73, 72, 62],
        [102, 217, 239],
    ),
    palette("ayu", "Ayu", [15, 20, 26], [230, 225, 207], [92, 103, 115], [255, 180, 84], [242, 92, 84], [36, 45, 56], [95, 175, 239]),
    palette(
        "github_dark",
        "GitHub Dark",
        [13, 17, 23],
        [230, 237, 243],
        [139, 148, 158],
        [88, 166, 255],
        [248, 81, 73],
        [48, 54, 61],
        [63, 185, 80],
    ),
];

const fn palette(
    id: &'static str,
    name: &'static str,
    background: [u8; 3],
    text: [u8; 3],
    weak: [u8; 3],
    accent: [u8; 3],
    record: [u8; 3],
    button: [u8; 3],
    selected: [u8; 3],
) -> Stock {
    Stock {
        id,
        name,
        look: Look {
            background,
            text,
            weak,
            accent,
            record,
            on_colour: background,
            button,
            button_text: text,
            selected,
            selected_text: background,
        },
    }
}

/// The generated theme for a stock id, if `id` is one.
pub fn source(id: &str) -> Option<String> {
    stock().iter().find(|t| t.id == id).map(|t| lua(t.name, &t.look))
}

/// A file name for a theme title: lower case, letters, digits and single underscores,
/// at most 40 characters. `None` when the title has nothing usable in it.
pub fn slug(name: &str) -> Option<String> {
    let mut s = String::new();
    let mut gap = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !s.is_empty() {
                s.push('_');
            }
            s.push(c.to_ascii_lowercase());
            gap = false;
        } else {
            gap = true;
        }
        if s.len() >= 40 {
            break;
        }
    }
    while s.ends_with('_') {
        s.pop();
    }
    if s.is_empty() { None } else { Some(s) }
}

/// The theme file Preferences writes. It builds on Maschine and sets every colour name
/// the shipped pages use, so a page never falls through to Maschine's own colour for a
/// role this look decided.
pub fn lua(name: &str, look: &Look) -> String {
    let mut out = format!(
        "-- DECKARD theme: {name}\n\
         -- Share this file as it is. The person you send it to loads it with\n\
         -- Preferences > Hardware > Import theme.\n"
    );
    for (field, rgb) in look.fields() {
        let [r, g, b] = rgb;
        out.push_str(&format!("local {field} = {{ {r}, {g}, {b} }}\n"));
    }
    out.push_str("\nreturn {\n    base = \"maschine\",\n    colors = {\n");
    for (role, value) in assignments(look) {
        out.push_str(&format!("        {role} = {value},\n"));
    }
    out.push_str("    },\n}\n");
    out
}

/// Read back a file this module wrote. A hand-written theme has no `local background`,
/// so this returns `None` and the caller keeps that file as it is.
pub fn parse(text: &str) -> Option<Look> {
    let mut look = Look {
        background: [0, 0, 0],
        text: [255, 255, 255],
        weak: [128, 128, 128],
        accent: [255, 165, 42],
        record: [224, 60, 60],
        on_colour: [0, 0, 0],
        button: [71, 71, 71],
        button_text: [255, 255, 255],
        selected: [178, 178, 178],
        selected_text: [0, 0, 0],
    };
    let mut found = 0;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("local ") else { continue };
        let Some((name, value)) = rest.split_once('=') else { continue };
        let Some(rgb) = parse_rgb(value) else { continue };
        if look.set(name.trim(), rgb) {
            found += 1;
        }
    }
    (found == 10).then_some(look)
}

impl Look {
    fn fields(self) -> [(&'static str, [u8; 3]); 10] {
        [
            ("background", self.background),
            ("text", self.text),
            ("weak", self.weak),
            ("accent", self.accent),
            ("record", self.record),
            ("on_colour", self.on_colour),
            ("button", self.button),
            ("button_text", self.button_text),
            ("selected", self.selected),
            ("selected_text", self.selected_text),
        ]
    }

    fn set(&mut self, name: &str, rgb: [u8; 3]) -> bool {
        let slot = match name {
            "background" => &mut self.background,
            "text" => &mut self.text,
            "weak" => &mut self.weak,
            "accent" => &mut self.accent,
            "record" => &mut self.record,
            "on_colour" => &mut self.on_colour,
            "button" => &mut self.button,
            "button_text" => &mut self.button_text,
            "selected" => &mut self.selected,
            "selected_text" => &mut self.selected_text,
            _ => return false,
        };
        *slot = rgb;
        true
    }
}

/// Every colour name the shipped pages use, and the Lua expression for it. The ten
/// picked colours are the locals; the rest are those colours or a mix, so a saved
/// theme restyles the screens instead of leaving Maschine's orange and grey in place.
fn assignments(look: &Look) -> Vec<(&'static str, String)> {
    // A step off the background, and not the button colour: the EQ, filter and scope
    // on the right screen are filled with this, so Button must not repaint them.
    let panel = mix(look.background, [255, 255, 255], 0.06);
    let deep = mix(look.background, [0, 0, 0], 0.4);
    let paint = |c: [u8; 3]| format!("{{ {}, {}, {} }}", c[0], c[1], c[2]);
    let mut rows: Vec<(&'static str, String)> = Vec::new();
    // A picked colour, by the local's name, and a mixed one written as a literal.
    let mut put = |role: &'static str, value: String| rows.push((role, value));
    put("background", "background".into());
    put("text", "text".into());
    put("weak", "weak".into());
    put("accent", "accent".into());
    put("record", "record".into());
    put("on_colour", "on_colour".into());
    put("button", "button".into());
    put("button_text", "button_text".into());
    put("button_selected", "selected".into());
    put("button_selected_text", "selected_text".into());
    // The same ten, under the names pages actually look up.
    put("white", "text".into());
    put("ink", "text".into());
    put("mixer_on", "text".into());
    put("mixer_name", "text".into());
    put("ideas_group_name", "text".into());
    put("marker", "text".into());
    put("slice_marker", "text".into());
    put("browse_param_all", "text".into());
    put("muted", "weak".into());
    put("comment", "weak".into());
    put("separator", "weak".into());
    put("mixer_headroom", "weak".into());
    put("mixer_scale", "weak".into());
    put("mixer_zero", "weak".into());
    put("mixer_value_muted", "weak".into());
    put("mixer_add", "weak".into());
    put("song_tick", "weak".into());
    put("song_tab_text", "weak".into());
    put("browse_param_none", "weak".into());
    put("pad_empty_text", "weak".into());
    put("wave_unsliced", "weak".into());
    put("arrow_off", "button".into());
    put("browse_tile", "button".into());
    put("song_tab_focus", "button".into());
    put("line", "button".into());
    put("browse_bracket", "accent".into());
    put("mark", "accent".into());
    put("hi", "selected".into());
    put("label", "button_text".into());
    put("on", "on_colour".into());
    put("deep", paint(deep));
    put("panel", paint(panel));
    put("bar", paint(deep));
    put("button_off", paint(panel));
    put("pad_empty", paint(panel));
    put("tile_off", paint(panel));
    put("pad_add", paint(panel));
    put("mixer_track", paint(panel));
    put("mixer_focused", paint(deep));
    put("ideas_bg", paint(look.background));
    put("ideas_line", paint(panel));
    put("ideas_tab_edge", paint(deep));
    put("ideas_tile_top", paint(panel));
    put("ideas_faded", paint(panel));
    put("song_bg", paint(look.background));
    put("song_empty", paint(look.background));
    put("song_bar_line", paint(deep));
    put("song_section_edge", paint(deep));
    put("song_tab", paint(panel));
    put("song_tab_edge", paint(deep));
    put("browse_row", paint(look.background));
    put("wave_off", paint(panel));
    put("wave_selection", paint(deep));
    put("slice_focus", paint(panel));
    put("meter_empty", paint(panel));
    put("zone_white", paint(panel));
    put("zone_black", paint(deep));
    put("zone_line", paint(deep));
    rows
}

/// The same colours `assignments` writes, as RGB, so a live preview can paint them
/// without saving the file first.
pub fn painted(look: &Look) -> Vec<(&'static str, [u8; 3])> {
    let locals = std::collections::HashMap::from(look.fields());
    assignments(look)
        .into_iter()
        .map(|(role, value)| {
            let rgb = locals.get(value.as_str()).copied().or_else(|| parse_rgb(&value)).unwrap_or(look.text);
            (role, rgb)
        })
        .collect()
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let mut out = [0; 3];
    for i in 0..3 {
        out[i] = (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round().clamp(0.0, 255.0) as u8;
    }
    out
}

fn parse_rgb(text: &str) -> Option<[u8; 3]> {
    let inner = text.trim().trim_end_matches(',').trim().strip_prefix('{')?.strip_suffix('}')?;
    let mut n = inner.split(',').map(|p| p.trim().parse::<u16>().ok());
    let r = n.next()??;
    let g = n.next()??;
    let b = n.next()??;
    if r > 255 || g > 255 || b > 255 {
        return None;
    }
    Some([r as u8, g as u8, b as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_theme_reads_back() {
        let look = stock()[0].look;
        let back = parse(&lua("Nord", &look)).expect("the file this module writes is one it reads");
        assert_eq!(back, look);
    }

    #[test]
    fn a_hand_written_theme_is_not_rewritten() {
        assert!(parse("return { base = 'maschine', colors = { accent = { 1, 2, 3 } } }\n").is_none());
    }

    #[test]
    fn a_title_becomes_a_file_name() {
        assert_eq!(slug("Tokyo Night"), Some("tokyo_night".into()));
        assert_eq!(slug("  Rosé Pine!! "), Some("ros_pine".into()));
        assert_eq!(slug("..."), None);
        assert_eq!(slug(&"n".repeat(80)).map(|s| s.len()), Some(40));
    }

    #[test]
    fn stock_themes_are_named_files_and_readable() {
        let mut ids: Vec<&str> = stock().iter().map(|t| t.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "two stock themes share an id");
        assert_eq!(n, 12);
        for theme in stock() {
            assert_eq!(slug(theme.id).as_deref(), Some(theme.id));
            assert_eq!(parse(&source(theme.id).unwrap()).unwrap(), theme.look);
            assert!(far(theme.look.text, theme.look.background), "{}", theme.id);
            assert!(far(theme.look.selected_text, theme.look.selected), "{}", theme.id);
            assert!(far(theme.look.button_text, theme.look.button), "{}", theme.id);
            let panel = painted(&theme.look).into_iter().find(|(role, _)| *role == "panel").map(|(_, c)| c);
            assert_ne!(panel, Some(theme.look.button), "{}: the button colour is not the plot", theme.id);
        }
    }

    /// Far enough apart that light text is not drawn on a light fill.
    fn far(a: [u8; 3], b: [u8; 3]) -> bool {
        let d = |i| (a[i] as i32 - b[i] as i32).pow(2);
        d(0) + d(1) + d(2) > 80 * 80
    }
}
