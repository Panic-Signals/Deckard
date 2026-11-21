//! One geometry table for every MK3 surface, and nothing else.
//!
//! The on-screen egui panel and the browser page paint the *same* surface, so
//! neither may carry a rectangle of its own: both read this table. A moved
//! control moves here, once.
//!
//! The geometry is the jamulator fork's `controller-mk3` stage, transcribed
//! from `~/jamini-jamulator/css/jamulator.scss` lines ~1201-1522: the stage is
//! `1518 x 1382`, and every control below is placed by the fork's own
//! percentages of that stage (`pct(left, top, w, h)` keeps the fork's numbers
//! literally, so a typo'd percentage reads as a wrong number, not a wrong
//! formula). There is no grid arithmetic and no margin/gap constant anywhere:
//! the fork has none. Do not re-derive these numbers and do not "improve"
//! them — where a value looks odd, it is a measurement.
//!
//! Relationships (the fork's, kept):
//! - the display bezel (`mk3-display-box`: 23.059%, 1.882%, 74.048%, 30.391%)
//!   sits behind the two screens; the eight display buttons run above them;
//! - the eight knobs run under the screens; the jog sits left of the pads with
//!   its four detent dots around it, and the wheel's push and four directions sit
//!   ON the jog and its pips, as the hardware builds one 4-D wheel. The four
//!   directions paint their own caps; the press paints none ([`Cap::Bare`]), so the
//!   jog's disc is the chrome and the press's rect is the disc's middle;
//! - the pads are a 4 x 4 grid with pad 1 at the bottom left, numbered upward, four
//!   to a row, as the hardware numbers them;
//! - the touch strip is the fork's placeholder bar with its dot row above it;
//! - the wheel's four-way push has no fork counterpart: the fork's jog has no
//!   ring. The four directions are centred on the jog's own detent pips, which is
//!   why they are the one documented overlap in the table (with the push itself).
//!
//! Decoration that is not a control has no id and lives outside this table:
//! the display bezel, the MASCHINE logo, the four jog dots and the strip's dot
//! row. Each painter carries those few rectangles as named constants citing
//! the fork's selector, and the served page carries them as static HTML/CSS.
//!
//! Button ids are the `NI.HW` names the MK3 reports (`deckard_mk3::Event::Button`),
//! so no painter needs a translation table. They come from what the repo already
//! names: the driver's `BUTTONS` table where an LED (`BUTTON_LEDS`, via
//! `names::light`) or a script name exists. Left out, and why: the knobs'
//! touches (`BUTTON_CAP_1..8`) ride with their knob control, and the jog's touch
//! (`BUTTON_WHEEL_TOUCH`) rides with the jog — `Kind` covers what a painter
//! draws differently, and neither touch draws on its own.
//!
//! Labels are the fork's own silkscreen (`index.html` lines 162-340), uppercased
//! by the painters the way the fork's CSS does: pads, knobs, the jog and the
//! display buttons carry no text in the fork, so their labels are empty.
//!
//! The table is ordered back-to-front (screens first, pads last), so `at` reads
//! it reversed: the control drawn on top wins the hit test. This module only
//! names `egui::Rect`/`Pos2` as geometry types; it never draws and needs no window.

use eframe::egui;

/// The panel's width in surface units: the fork's `width: 950px` stage is 1518 wide.
pub const WIDTH: f32 = 1518.0;
/// The panel's height in surface units: the fork's `aspect-ratio: 1518 / 1382`.
pub const HEIGHT: f32 = 1382.0;

/// A rect from the fork's percentages of the stage, so the table keeps the
/// fork's numbers literally and paints nothing.
const fn pct(left: f32, top: f32, w: f32, h: f32) -> egui::Rect {
    egui::Rect {
        min: egui::Pos2 { x: left * WIDTH / 100.0, y: top * HEIGHT / 100.0 },
        max: egui::Pos2 { x: (left + w) * WIDTH / 100.0, y: (top + h) * HEIGHT / 100.0 },
    }
}

/// What a painter draws differently: one variant per drawn shape, no more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pad,
    Button,
    Knob,
    Jog,
    Strip,
    Screen,
}

impl Kind {
    /// The lowercase name the browser page reads from `as_json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Pad => "pad",
            Kind::Button => "button",
            Kind::Knob => "knob",
            Kind::Jog => "jog",
            Kind::Strip => "strip",
            Kind::Screen => "screen",
        }
    }
}

/// The cap under the silkscreen: the fork's `btn-rgb` (dark translucent caps:
/// pads, display buttons, group buttons) versus the default silicon `btn-onoff`.
/// Only pads and buttons have a cap, and one button has none: the wheel's press sits on
/// the jog, which is the moulding you see there. Every other kind ignores this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Silicon,
    Rgb,
    /// No cap at all: the control is a touch target on a control that is already drawn.
    Bare,
}

impl Cap {
    /// The lowercase name the browser page reads from `as_json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Cap::Silicon => "silicon",
            Cap::Rgb => "rgb",
            Cap::Bare => "bare",
        }
    }
}

/// The cap the fork gives the named control: `Rgb` is the fork's `btn-rgb`
/// class (pads, `Mk3Mute1..8`, `Mk3GroupA..H` in `index.html`); everything
/// else is the default silicon `btn-onoff`, and the wheel's press is no cap at all.
/// Both painters read this, so the rule lives here once, never as a second list elsewhere.
pub fn cap(id: &str) -> Cap {
    if id.starts_with("pad") {
        return Cap::Rgb;
    }
    if let Some(rest) = id.strip_prefix("BUTTON_")
        && (rest.starts_with("SCREEN_") || rest.starts_with("GROUP_"))
    {
        return Cap::Rgb;
    }
    // The hardware has no button in the middle of the wheel: pushing it is pushing the
    // jog. So the press paints nothing — the jog's own disc is the chrome — while its
    // rect stays the hit target at the disc's centre.
    if id == "BUTTON_WHEEL" {
        return Cap::Bare;
    }
    Cap::Silicon
}

/// One control both painters draw: its driver id, its drawn shape, its label, its rect.
#[derive(Debug, Clone, Copy)]
pub struct Control {
    pub id: &'static str,
    pub kind: Kind,
    pub label: &'static str,
    pub rect: egui::Rect,
}

/// The whole surface, back-to-front: screens first, pads last, so the topmost
/// control wins [`at`]. Every percentage is the fork's (`jamulator.scss`
/// `#controller-mk3`); the comment per row names the fork's control or class.
static TABLE: [Control; 91] = [
    // The displays: the fork's `.mk3-screen-l/r` (30.567% x 17.077% at top 12.301%)
    // widened to the wells' own 34% x 20.2%, because the fork's inset image boxes left
    // the displays floating in their frame with a border on every side. They hang from
    // the display box's lower edge by the fork's own margin — `.mk3-display-box` ends at
    // 32.273%, the fork's screens at 29.378% — so the screens sit centred between the
    // mute row above (bottom 5.934%) and the box's edge. Taking the box's bottom instead
    // (top 11.6%) left 0.473% of bezel below the glass and 5.666% above it: the screens
    // read as sunk, which is the misplacement this row fixes.
    Control { id: "screen_left", kind: Kind::Screen, label: "Left screen", rect: pct(24.600, 9.178, 34.000, 20.200) },
    Control { id: "screen_right", kind: Kind::Screen, label: "Right screen", rect: pct(61.600, 9.178, 34.000, 20.200) },
    // The touch strip: the fork's `.mk3-strip-ph` bar (the dot row above it is
    // decoration, drawn by each painter, not a control).
    Control { id: "strip", kind: Kind::Strip, label: "Touch strip", rect: pct(3.031, 64.256, 34.059, 3.618) },
    // The left column: Channel/Plugin/ArrowLeft/ArrowRight/File/Settings/Auto/
    // Macro are 7.642% x 2.750%; Arranger/Mixer/Browser/Sampling are double tall.
    Control { id: "BUTTON_CHANNEL", kind: Kind::Button, label: "Channel", rect: pct(3.096, 3.256, 7.642, 2.750) },
    Control { id: "BUTTON_PLUGIN", kind: Kind::Button, label: "Plug-in", rect: pct(11.925, 3.256, 7.642, 2.750) },
    Control { id: "BUTTON_ARRANGE", kind: Kind::Button, label: "Arranger", rect: pct(3.096, 8.756, 7.642, 4.920) },
    Control { id: "BUTTON_MIX", kind: Kind::Button, label: "Mixer", rect: pct(11.925, 8.756, 7.642, 4.920) },
    Control { id: "BUTTON_BROWSE", kind: Kind::Button, label: "Browser", rect: pct(3.096, 14.979, 7.642, 4.920) },
    Control { id: "BUTTON_SAMPLE", kind: Kind::Button, label: "Sampling", rect: pct(11.925, 14.979, 7.642, 4.920) },
    Control { id: "BUTTON_LEFT", kind: Kind::Button, label: "◀", rect: pct(3.096, 21.202, 7.642, 2.750) },
    Control { id: "BUTTON_RIGHT", kind: Kind::Button, label: "▶", rect: pct(11.925, 21.202, 7.642, 2.750) },
    Control { id: "BUTTON_FILE", kind: Kind::Button, label: "File", rect: pct(3.096, 25.326, 7.642, 2.750) },
    Control { id: "BUTTON_SETTINGS", kind: Kind::Button, label: "Settings", rect: pct(11.925, 25.326, 7.642, 2.750) },
    Control { id: "BUTTON_AUTO", kind: Kind::Button, label: "Auto", rect: pct(3.096, 29.450, 7.642, 2.750) },
    Control { id: "BUTTON_MACRO", kind: Kind::Button, label: "Macro", rect: pct(11.925, 29.450, 7.642, 2.750) },
    // Display buttons 1-8: the fork's `Mk3Mute1..8`, empty labels, `btn-rgb`.
    Control { id: "BUTTON_SCREEN_1", kind: Kind::Button, label: "", rect: pct(25.693, 3.329, 7.510, 2.605) },
    Control { id: "BUTTON_SCREEN_2", kind: Kind::Button, label: "", rect: pct(34.455, 3.329, 7.510, 2.605) },
    Control { id: "BUTTON_SCREEN_3", kind: Kind::Button, label: "", rect: pct(43.216, 3.329, 7.510, 2.605) },
    Control { id: "BUTTON_SCREEN_4", kind: Kind::Button, label: "", rect: pct(51.978, 3.329, 7.510, 2.605) },
    Control { id: "BUTTON_SCREEN_5", kind: Kind::Button, label: "", rect: pct(60.739, 3.329, 7.510, 2.605) },
    Control { id: "BUTTON_SCREEN_6", kind: Kind::Button, label: "", rect: pct(69.507, 3.329, 7.510, 2.605) },
    Control { id: "BUTTON_SCREEN_7", kind: Kind::Button, label: "", rect: pct(78.334, 3.329, 7.510, 2.605) },
    Control { id: "BUTTON_SCREEN_8", kind: Kind::Button, label: "", rect: pct(87.096, 3.329, 7.510, 2.605) },
    // The eight knobs: the fork's `.encoder.mk3-knob` (5.007% x 5.499%), no text.
    Control { id: "knob1", kind: Kind::Knob, label: "", rect: pct(26.879, 34.298, 5.007, 5.499) },
    Control { id: "knob2", kind: Kind::Knob, label: "", rect: pct(35.640, 34.298, 5.007, 5.499) },
    Control { id: "knob3", kind: Kind::Knob, label: "", rect: pct(44.402, 34.298, 5.007, 5.499) },
    Control { id: "knob4", kind: Kind::Knob, label: "", rect: pct(53.163, 34.298, 5.007, 5.499) },
    Control { id: "knob5", kind: Kind::Knob, label: "", rect: pct(61.992, 34.298, 5.007, 5.499) },
    Control { id: "knob6", kind: Kind::Knob, label: "", rect: pct(70.754, 34.298, 5.007, 5.499) },
    Control { id: "knob7", kind: Kind::Knob, label: "", rect: pct(79.516, 34.298, 5.007, 5.499) },
    Control { id: "knob8", kind: Kind::Knob, label: "", rect: pct(88.279, 34.298, 5.007, 5.499) },
    // Volume/Swing/Tempo down the fork's column (20.686%, 7.642% x 2.750%).
    Control { id: "BUTTON_VOLUME", kind: Kind::Button, label: "Volume", rect: pct(20.686, 43.416, 7.642, 2.750) },
    Control { id: "BUTTON_SWING", kind: Kind::Button, label: "Swing", rect: pct(20.686, 47.614, 7.642, 2.750) },
    Control { id: "BUTTON_TEMPO", kind: Kind::Button, label: "Tempo", rect: pct(20.686, 51.738, 7.642, 2.750) },
    // Note Repeat (double tall) and Lock.
    Control { id: "BUTTON_NOTE_REPEAT", kind: Kind::Button, label: "Note Repeat", rect: pct(29.448, 43.416, 7.642, 6.947) },
    Control { id: "BUTTON_LOCK", kind: Kind::Button, label: "Lock", rect: pct(29.448, 51.738, 7.642, 2.750) },
    Control { id: "BUTTON_FIX_VELOCITY", kind: Kind::Button, label: "Fixed Vel", rect: pct(43.151, 43.416, 7.642, 2.750) },
    Control { id: "BUTTON_PAD_MODE", kind: Kind::Button, label: "Pad Mode", rect: pct(53.232, 43.416, 10.079, 2.750) },
    Control { id: "BUTTON_KEYBOARD", kind: Kind::Button, label: "Keyboard", rect: pct(64.495, 43.416, 10.079, 2.750) },
    Control { id: "BUTTON_CHORDS", kind: Kind::Button, label: "Chords", rect: pct(75.757, 43.416, 10.079, 2.750) },
    Control { id: "BUTTON_STEP", kind: Kind::Button, label: "Step", rect: pct(87.024, 43.416, 10.079, 2.750) },
    // The Scene column down the middle (43.151%, 7.642% x 4.920%).
    Control { id: "BUTTON_SCENE", kind: Kind::Button, label: "Scene", rect: pct(43.151, 48.917, 7.642, 4.920) },
    Control { id: "BUTTON_PATTERN", kind: Kind::Button, label: "Pattern", rect: pct(43.151, 55.140, 7.642, 4.920) },
    Control { id: "BUTTON_EVENTS", kind: Kind::Button, label: "Events", rect: pct(43.151, 61.363, 7.642, 4.920) },
    Control { id: "BUTTON_VARIATION", kind: Kind::Button, label: "Variation", rect: pct(43.151, 67.586, 7.642, 4.920) },
    Control { id: "BUTTON_DUPLICATE", kind: Kind::Button, label: "Dupl", rect: pct(43.151, 73.809, 7.642, 4.920) },
    Control { id: "BUTTON_SELECT", kind: Kind::Button, label: "Select", rect: pct(43.151, 80.032, 7.642, 4.920) },
    Control { id: "BUTTON_SOLO", kind: Kind::Button, label: "Solo", rect: pct(43.151, 86.255, 7.642, 4.920) },
    Control { id: "BUTTON_MUTE", kind: Kind::Button, label: "Mute", rect: pct(43.151, 92.478, 7.642, 4.920) },
    // Pitch/Mod/Perform/Notes across the fork's row (top 58.537%, 7.642% x 2.750%).
    Control { id: "BUTTON_PITCH", kind: Kind::Button, label: "Pitch", rect: pct(3.096, 58.537, 7.642, 2.750) },
    Control { id: "BUTTON_MOD", kind: Kind::Button, label: "Mod", rect: pct(11.925, 58.537, 7.642, 2.750) },
    Control { id: "BUTTON_PERFORM", kind: Kind::Button, label: "Perform", rect: pct(20.686, 58.537, 7.642, 2.750) },
    Control { id: "BUTTON_NOTES", kind: Kind::Button, label: "Notes", rect: pct(29.448, 58.537, 7.642, 2.750) },
    // Group buttons A-H: the fork's `btn-rgb` (7.576% x 4.920%), two rows of four.
    Control { id: "BUTTON_GROUP_A", kind: Kind::Button, label: "A", rect: pct(3.096, 73.009, 7.576, 4.920) },
    Control { id: "BUTTON_GROUP_B", kind: Kind::Button, label: "B", rect: pct(11.878, 73.009, 7.576, 4.920) },
    Control { id: "BUTTON_GROUP_C", kind: Kind::Button, label: "C", rect: pct(20.660, 73.009, 7.576, 4.920) },
    Control { id: "BUTTON_GROUP_D", kind: Kind::Button, label: "D", rect: pct(29.442, 73.009, 7.576, 4.920) },
    Control { id: "BUTTON_GROUP_E", kind: Kind::Button, label: "E", rect: pct(3.096, 79.232, 7.576, 4.920) },
    Control { id: "BUTTON_GROUP_F", kind: Kind::Button, label: "F", rect: pct(11.878, 79.232, 7.576, 4.920) },
    Control { id: "BUTTON_GROUP_G", kind: Kind::Button, label: "G", rect: pct(20.660, 79.232, 7.576, 4.920) },
    Control { id: "BUTTON_GROUP_H", kind: Kind::Button, label: "H", rect: pct(29.442, 79.232, 7.576, 4.920) },
    // Transport: Restart/Erase/Tap/Follow, then Play/Stop/Rec/Shift.
    Control { id: "BUTTON_TRANSPORT_LOOP", kind: Kind::Button, label: "Restart", rect: pct(3.096, 88.133, 7.642, 2.750) },
    Control { id: "BUTTON_TRANSPORT_ERASE", kind: Kind::Button, label: "Erase", rect: pct(11.925, 88.133, 7.642, 2.750) },
    Control { id: "BUTTON_TAP", kind: Kind::Button, label: "Tap", rect: pct(20.686, 88.133, 7.642, 2.750) },
    Control { id: "BUTTON_FOLLOW", kind: Kind::Button, label: "Follow", rect: pct(29.448, 88.133, 7.642, 2.750) },
    Control { id: "BUTTON_TRANSPORT_PLAY", kind: Kind::Button, label: "Play", rect: pct(3.096, 92.330, 7.642, 4.920) },
    Control { id: "BUTTON_TRANSPORT_STOP", kind: Kind::Button, label: "Stop", rect: pct(11.925, 92.330, 7.642, 4.920) },
    Control { id: "BUTTON_TRANSPORT_RECORD", kind: Kind::Button, label: "Rec", rect: pct(20.686, 92.330, 7.642, 4.920) },
    Control { id: "BUTTON_SHIFT", kind: Kind::Button, label: "Shift", rect: pct(29.448, 92.330, 7.642, 4.920) },
    // The jog: the fork's `.encoder.mk3-jog` (7.115%, 44.356%, 8.301%, 9.117%).
    Control { id: "jog", kind: Kind::Jog, label: "", rect: pct(7.115, 44.356, 8.301, 9.117) },
    // The wheel's push: the fork's `Mk3JogPush`, except the fork parks a labelled button
    // beside the jog and the hardware has no button anywhere on the wheel. The press and
    // its four directions therefore sit ON the jog and its pips as one 4-D wheel, and the
    // press paints no cap of its own (`cap`): the jog's disc is the chrome, its middle is
    // the press, and the label is empty because the fork's "Push" is not printed on the
    // hardware. The directions are unlabelled too — at this size the words do not fit and
    // the hardware prints nothing there either, so the pips are the legend.
    Control { id: "BUTTON_WHEEL", kind: Kind::Button, label: "", rect: pct(8.967, 47.815, 4.600, 2.200) },
    Control { id: "BUTTON_WHEEL_UP", kind: Kind::Button, label: "", rect: pct(9.895, 42.234, 3.400, 2.598) },
    Control { id: "BUTTON_WHEEL_DOWN", kind: Kind::Button, label: "", rect: pct(9.895, 55.387, 3.400, 2.598) },
    Control { id: "BUTTON_WHEEL_LEFT", kind: Kind::Button, label: "", rect: pct(3.899, 48.739, 3.400, 2.598) },
    Control { id: "BUTTON_WHEEL_RIGHT", kind: Kind::Button, label: "", rect: pct(15.823, 48.739, 3.400, 2.598) },
    // The pads, four to a row with pad 1 at the BOTTOM left, as the hardware numbers them:
    // the user plays them against a real MK3, and the fork's mirroring (Mk3Pad1..4 on the
    // top row) is the reference's error, not a measurement to keep. Drawn last so they win
    // ties.
    Control { id: "pad1", kind: Kind::Pad, label: "", rect: pct(53.232, 86.179, 10.013, 10.999) },
    Control { id: "pad2", kind: Kind::Pad, label: "", rect: pct(64.561, 86.179, 10.013, 10.999) },
    Control { id: "pad3", kind: Kind::Pad, label: "", rect: pct(75.757, 86.179, 10.013, 10.999) },
    Control { id: "pad4", kind: Kind::Pad, label: "", rect: pct(87.024, 86.179, 10.013, 10.999) },
    Control { id: "pad5", kind: Kind::Pad, label: "", rect: pct(53.232, 73.732, 10.013, 10.999) },
    Control { id: "pad6", kind: Kind::Pad, label: "", rect: pct(64.561, 73.732, 10.013, 10.999) },
    Control { id: "pad7", kind: Kind::Pad, label: "", rect: pct(75.757, 73.732, 10.013, 10.999) },
    Control { id: "pad8", kind: Kind::Pad, label: "", rect: pct(87.024, 73.732, 10.013, 10.999) },
    Control { id: "pad9", kind: Kind::Pad, label: "", rect: pct(53.232, 61.358, 10.013, 10.999) },
    Control { id: "pad10", kind: Kind::Pad, label: "", rect: pct(64.561, 61.358, 10.013, 10.999) },
    Control { id: "pad11", kind: Kind::Pad, label: "", rect: pct(75.757, 61.358, 10.013, 10.999) },
    Control { id: "pad12", kind: Kind::Pad, label: "", rect: pct(87.024, 61.358, 10.013, 10.999) },
    Control { id: "pad13", kind: Kind::Pad, label: "", rect: pct(53.232, 48.989, 10.013, 10.999) },
    Control { id: "pad14", kind: Kind::Pad, label: "", rect: pct(64.561, 48.989, 10.013, 10.999) },
    Control { id: "pad15", kind: Kind::Pad, label: "", rect: pct(75.757, 48.989, 10.013, 10.999) },
    Control { id: "pad16", kind: Kind::Pad, label: "", rect: pct(87.024, 48.989, 10.013, 10.999) },
];

/// Every control on the panel, back-to-front: screens first, pads last.
pub fn controls() -> &'static [Control] {
    &TABLE
}

/// The LED a control's light comes from, for `deckard_mk3::leds::Leds::get`, if one
/// control is one light: pads are `padN`, buttons swap `BUTTON_` for `LED_`, except the
/// display buttons (`BUTTON_SCREEN_N` → `LED_SCREEN_BUTTON_N`), the wheel's arrows
/// (`BUTTON_WHEEL_UP` → `LED_WHEEL_BUTTON_UP`) and AUTO (`BUTTON_AUTO` →
/// `LED_AUTO_WRITE`) — the way the scripts name them. The strip paints 25 segments, and
/// the knobs, the jog, the screens and the wheel's press have no light: [`None`].
pub fn led(id: &str) -> Option<String> {
    if let Some(n) = id.strip_prefix("pad") {
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        return Some(format!("LED_PAD_{n}"));
    }
    let rest = id.strip_prefix("BUTTON_")?;
    if rest == "WHEEL" {
        return None;
    }
    if rest == "AUTO" {
        return Some("LED_AUTO_WRITE".to_string());
    }
    if let Some(n) = rest.strip_prefix("SCREEN_") {
        return Some(format!("LED_SCREEN_BUTTON_{n}"));
    }
    if let Some(arrow) = rest.strip_prefix("WHEEL_") {
        return Some(format!("LED_WHEEL_BUTTON_{arrow}"));
    }
    Some(format!("LED_{rest}"))
}

/// The control called `id`, if there is one.
pub fn control(id: &str) -> Option<&'static Control> {
    TABLE.iter().find(|c| c.id == id)
}

/// The topmost control containing `p`: the table reads back-to-front, so the first hit
/// from the back is what the painter draws there.
pub fn at(p: egui::Pos2) -> Option<&'static Control> {
    TABLE.iter().rev().find(|c| c.rect.contains(p))
}

/// The same table for the browser page: `{width, height, controls:
/// [{id, kind, cap, label, rect: {x, y, w, h}}]}`. Built from [`controls`],
/// never a second list.
pub fn as_json() -> serde_json::Value {
    serde_json::json!({
        "width": WIDTH,
        "height": HEIGHT,
        "controls": controls()
            .iter()
            .map(|c| serde_json::json!({
                "id": c.id,
                "kind": c.kind.as_str(),
                "cap": cap(c.id).as_str(),
                "label": c.label,
                "rect": {"x": c.rect.min.x, "y": c.rect.min.y, "w": c.rect.width(), "h": c.rect.height()},
            }))
            .collect::<Vec<_>>(),
    })
}
