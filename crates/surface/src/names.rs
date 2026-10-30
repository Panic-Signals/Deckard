//! The scripts name buttons and lights by their labels on the MK3 (`PLAY`, `REC`,
//! `GROUP_A`, `PAD_1`, `SCREEN_1`…); the driver by its HID names.

use deckard_mk3::leds::{BUTTON_LEDS, PAD_LEDS};

/// Labels that differ from the driver's names: (label, button, light).
const RENAMED: [(&str, &str, &str); 7] = [
    ("PLAY", "BUTTON_TRANSPORT_PLAY", "LED_TRANSPORT_PLAY"),
    ("STOP", "BUTTON_TRANSPORT_STOP", "LED_TRANSPORT_STOP"),
    ("REC", "BUTTON_TRANSPORT_RECORD", "LED_TRANSPORT_RECORD"),
    ("RESTART", "BUTTON_TRANSPORT_LOOP", "LED_TRANSPORT_LOOP"),
    ("ERASE", "BUTTON_TRANSPORT_ERASE", "LED_TRANSPORT_ERASE"),
    ("FIXED_VEL", "BUTTON_FIX_VELOCITY", "LED_FIX_VELOCITY"),
    ("AUTO", "BUTTON_AUTO", "LED_AUTO_WRITE"),
];

/// A button's label, from the driver's name (`BUTTON_TRANSPORT_PLAY` → `PLAY`).
pub fn button(hw: &str) -> String {
    if let Some((label, ..)) = RENAMED.iter().find(|(_, b, _)| *b == hw) {
        return label.to_string();
    }
    hw.strip_prefix("BUTTON_").unwrap_or(hw).to_string()
}

/// The driver's light for a label (`REC` → `LED_TRANSPORT_RECORD`, `SCREEN_2` →
/// `LED_SCREEN_BUTTON_2`), if there is one.
pub fn light(label: &str) -> Option<String> {
    let name = if let Some((.., l)) = RENAMED.iter().find(|(n, ..)| *n == label) {
        l.to_string()
    } else if let Some(n) = label.strip_prefix("SCREEN_") {
        format!("LED_SCREEN_BUTTON_{n}")
    } else if let Some(arrow) = label.strip_prefix("WHEEL_") {
        format!("LED_WHEEL_BUTTON_{arrow}")
    } else {
        format!("LED_{label}")
    };
    (BUTTON_LEDS.contains(&name.as_str()) || PAD_LEDS.contains(&name.as_str())).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_both_ways() {
        assert_eq!(button("BUTTON_TRANSPORT_RECORD"), "REC");
        assert_eq!(button("BUTTON_GROUP_C"), "GROUP_C");
        assert_eq!(button("BUTTON_SCREEN_4"), "SCREEN_4");
        assert_eq!(light("REC").as_deref(), Some("LED_TRANSPORT_RECORD"));
        assert_eq!(light("AUTO").as_deref(), Some("LED_AUTO_WRITE"));
        assert_eq!(light("SCREEN_4").as_deref(), Some("LED_SCREEN_BUTTON_4"));
        assert_eq!(light("PAD_16").as_deref(), Some("LED_PAD_16"));
        assert_eq!(light("GROUP_H").as_deref(), Some("LED_GROUP_H"));
        assert_eq!(light("WHEEL_UP").as_deref(), Some("LED_WHEEL_BUTTON_UP"));
        assert_eq!(light("NOPE"), None);
    }
}
