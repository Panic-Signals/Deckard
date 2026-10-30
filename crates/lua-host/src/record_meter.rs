//! The Sampling page's input meter (`NI.GUI.insertMasterLevelMeter`, the RECORD
//! screen's left display), drawn as shapes over its box: a bar per channel filling in
//! the sound's colour from −60 dB to 0 dB (the app's RECORD meter's scale), a red mark
//! where a channel clips, and in Detect the threshold as a white line. The scripts set
//! its levels (`setLevels(left, right)`, linear), colour and threshold
//! (`setVolume`, 0..1) every tick.
//!
//! Sizes are the style sheet's (`HW/MaschineStudio/SamplingPage.txt`): channels 463 by
//! 10 pixels from 7 across, 15 and 33 down the 58-pixel meter.

use crate::pattern_view::rect;
use crate::state::{Prop, Widgets};
use deckard_nwl::draw::{At, Fill, Shape};

const LEFT: i32 = 7;
const WIDTH: i32 = 463;
const HEIGHT: i32 = 10;
const CHANNELS: [i32; 2] = [15, 33];
const CLIP: i32 = 6;
const RANGE_DB: f64 = 60.0;

const CHANNEL_BG: [u8; 4] = [38, 38, 38, 255];
const UNCOLOURED: [u8; 4] = [200, 200, 200, 255];
const CLIPPED: [u8; 4] = [230, 40, 40, 255];
const THRESHOLD: [u8; 4] = [255, 255, 255, 255];

/// How far along a channel a level (linear) reaches, 0..1.
fn fill(level: f64) -> f64 {
    if level <= 0.0 { 0.0 } else { ((20.0 * level.log10() + RANGE_DB) / RANGE_DB).clamp(0.0, 1.0) }
}

pub fn shapes(w: &Widgets, id: usize) -> Vec<Shape> {
    let Some(x) = w.get(id) else { return Vec::new() };
    let num = |k: &str| match x.props.get(k) {
        Some(Prop::Num(n)) => *n,
        Some(Prop::Int(i)) => *i as f64,
        _ => 0.0,
    };
    let colour = match x.props.get("PaletteColorIndex") {
        Some(Prop::Int(i)) if *i > 0 => Fill::Palette { palette: 1, index: *i as u32, alpha: 255 },
        _ => Fill::Rgba(UNCOLOURED),
    };
    let px = |x: i32| At::px(x);
    let mut out = Vec::new();
    for (k, y) in CHANNELS.into_iter().enumerate() {
        let level = num(if k == 0 { "Levels" } else { "Levels2" });
        out.push(rect(px(LEFT), px(y), px(LEFT + WIDTH), px(y + HEIGHT), CHANNEL_BG));
        let len = (fill(level) * WIDTH as f64).round() as i32;
        if len > 0 {
            out.push(Shape::rect(px(LEFT), px(y), px(LEFT + len), px(y + HEIGHT), colour.clone()));
        }
        if level >= 1.0 {
            let top = y + (HEIGHT - CLIP) / 2;
            out.push(rect(px(LEFT + WIDTH + 2), px(top), px(LEFT + WIDTH + 2 + CLIP), px(top + CLIP), CLIPPED));
        }
    }
    if matches!(x.props.get("SliderVisible"), Some(Prop::Bool(true))) {
        let at = LEFT + (num("Volume").clamp(0.0, 1.0) * WIDTH as f64).round() as i32;
        out.push(rect(px(at - 1), px(CHANNELS[0] - 6), px(at + 1), px(CHANNELS[1] + HEIGHT + 6), THRESHOLD));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::fill;

    /// −60 dB and below show nothing, 0 dB fills the channel, −30 dB half of it.
    #[test]
    fn the_scale_runs_from_minus_60_to_0_db() {
        assert_eq!(fill(0.0), 0.0);
        assert_eq!(fill(0.001), 0.0);
        assert!((fill(10f64.powf(-30.0 / 20.0)) - 0.5).abs() < 1e-9);
        assert_eq!(fill(1.0), 1.0);
        assert_eq!(fill(2.0), 1.0);
    }
}
