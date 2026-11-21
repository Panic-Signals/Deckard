//! The single geometry table's contract, with no device and no window: ids are unique and
//! on the surface, every MK3 control the drivers and scripts name is present, no two
//! controls overlap, the fork's pad pitch reproduces, and a point lands on the control
//! drawn under it.

use crate::mk3_layout::{self, Kind};
use eframe::egui;

#[test]
fn every_control_has_a_unique_id_and_lies_on_the_surface() {
    let controls = mk3_layout::controls();
    assert!(!controls.is_empty(), "the table is not empty");
    let mut ids: Vec<&str> = controls.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), before, "control ids are unique");
    for c in controls {
        assert!(c.rect.min.x >= 0.0 && c.rect.min.y >= 0.0, "{} starts on the surface", c.id);
        assert!(c.rect.max.x <= mk3_layout::WIDTH && c.rect.max.y <= mk3_layout::HEIGHT, "{} ends on the surface", c.id);
        assert!(c.rect.width() > 0.0 && c.rect.height() > 0.0, "{} has a size", c.id);
    }
}

#[test]
fn the_mk3_s_controls_are_all_present() {
    let controls = mk3_layout::controls();
    let count = |kind: Kind| controls.iter().filter(|c| c.kind == kind).count();
    assert_eq!(count(Kind::Pad), 16, "16 pads");
    assert_eq!(count(Kind::Knob), 8, "8 knobs");
    assert_eq!(count(Kind::Jog), 1, "1 jog");
    assert_eq!(count(Kind::Strip), 1, "1 strip");
    assert_eq!(count(Kind::Screen), 2, "2 screen controls");
    // Every button the MK3 reports is a control, except the touches that ride with their
    // control: the knobs' (BUTTON_CAP_*) and the jog's (BUTTON_WHEEL_TOUCH).
    for (_, name) in deckard_mk3::BUTTONS {
        if name.starts_with("BUTTON_CAP_") || *name == "BUTTON_WHEEL_TOUCH" {
            continue;
        }
        assert!(mk3_layout::control(name).is_some(), "{name} is in the table");
    }
    // Pad 1 prints at the bottom left, four to a row, numbered upward as the hardware
    // numbers them — the fork mirrors this (Mk3Pad1..4 on its top row) and the hardware
    // wins, because it is what the user plays.
    let pad1 = mk3_layout::control("pad1").expect("pad1");
    let pad4 = mk3_layout::control("pad4").expect("pad4");
    let pad13 = mk3_layout::control("pad13").expect("pad13");
    assert!(pad1.rect.min.x < pad4.rect.min.x, "pad 1 is left of pad 4");
    assert!(pad1.rect.min.y > pad13.rect.min.y, "pad 1 is below pad 13");
    assert!((pad1.rect.min.y - pad4.rect.min.y).abs() < 1e-3, "pads 1-4 share the bottom row");
    // The two screens fill their wells inside the display bezel: 34% x 20.2% at lefts
    // 24.6% / 61.6%, top 9.178% — the wells' own bounds, wider and taller than the fork's
    // 30.567% x 17.077% inset image boxes, which left the displays floating in their frame,
    // and hung by the fork's own 2.895% from the box's lower edge so they sit centred
    // between the mute row above and the bezel's edge rather than sunk against it.
    let left = mk3_layout::control("screen_left").expect("screen_left");
    let right = mk3_layout::control("screen_right").expect("screen_right");
    let eps = 0.05;
    assert!((left.rect.width() - 34.0 / 100.0 * mk3_layout::WIDTH).abs() < eps, "screen width takes the well's 34%");
    assert!((left.rect.height() - 20.2 / 100.0 * mk3_layout::HEIGHT).abs() < eps, "screen height takes the well's 20.2%");
    assert!((left.rect.min.y - right.rect.min.y).abs() < 1e-3, "screens share a top");
    assert!((right.rect.min.x - left.rect.max.x - 3.0 / 100.0 * mk3_layout::WIDTH).abs() < eps, "a 3% gap sits between the screens");
}

#[test]
fn a_point_lands_on_the_control_under_it() {
    let pad1 = mk3_layout::control("pad1").expect("pad1");
    let hit = mk3_layout::at(pad1.rect.center()).expect("pad 1's centre hits");
    assert_eq!(hit.id, "pad1", "the centre of pad 1 lands on pad 1");
    // The wheel's push sits ON the jog now, as the hardware builds it: the press owns the
    // jog's middle, and the jog keeps everything the press and the four directions leave.
    let press = mk3_layout::control("BUTTON_WHEEL").expect("BUTTON_WHEEL");
    let hit = mk3_layout::at(press.rect.center()).expect("the wheel press's centre hits");
    assert_eq!(hit.id, "BUTTON_WHEEL", "the press wins its own middle");
    let jog = mk3_layout::control("jog").expect("jog");
    let hit = mk3_layout::at(jog.rect.center()).expect("the jog's centre hits");
    assert_eq!(hit.id, "BUTTON_WHEEL", "the jog's centre is the press that sits on it");
    let corner = egui::pos2(14.5 / 100.0 * mk3_layout::WIDTH, 52.5 / 100.0 * mk3_layout::HEIGHT);
    assert!(jog.rect.contains(corner), "the probe is inside the jog");
    let hit = mk3_layout::at(corner).expect("the jog's lower right still hits");
    assert_eq!(hit.id, "jog", "a point the ring leaves free is the jog itself");
    // The bottom-right margin is no control's: between controls is nothing.
    let gap = egui::pos2(mk3_layout::WIDTH - 8.0, mk3_layout::HEIGHT - 8.0);
    assert!(mk3_layout::at(gap).is_none(), "a point between controls hits nothing");
}

/// A table with a typo'd percentage is exactly what a screenshot catches — and what
/// this catches first: the fork's pad pitch (10.013% x 10.999%, rows at 48.989 /
/// 61.358 / 73.732 / 86.179%) reproduces to the tenth of a unit.
#[test]
fn the_fork_s_pad_pitch_reproduces() {
    let eps = 0.05;
    let w = 10.013 / 100.0 * mk3_layout::WIDTH;
    let h = 10.999 / 100.0 * mk3_layout::HEIGHT;
    let cols = [53.232, 64.561, 75.757, 87.024];
    let rows = [48.989, 61.358, 73.732, 86.179];
    for (i, n) in (1..=16).enumerate() {
        let id = format!("pad{n}");
        let pad = mk3_layout::control(&id).unwrap_or_else(|| panic!("{id} is in the table"));
        assert!((pad.rect.width() - w).abs() < eps, "{id} is the fork's 10.013% wide");
        assert!((pad.rect.height() - h).abs() < eps, "{id} is the fork's 10.999% tall");
        assert!((pad.rect.min.x - cols[i % 4] / 100.0 * mk3_layout::WIDTH).abs() < eps, "{id} sits in its fork column");
        assert!(
            (pad.rect.min.y - rows[3 - i / 4] / 100.0 * mk3_layout::HEIGHT).abs() < eps,
            "{id} sits in its row, counted from the bottom"
        )
    }
    // The columns step by the fork's own three steps (11.329 / 11.196 / 11.267% of
    // the stage — measurements, not uniform, so each step is pinned separately).
    let steps = [cols[1] - cols[0], cols[2] - cols[1], cols[3] - cols[2]];
    for row in 0..4 {
        let base = row * 4 + 1;
        for (s, k) in steps.iter().enumerate() {
            let (a, b) = (format!("pad{}", base + s), format!("pad{}", base + s + 1));
            let gap = mk3_layout::control(&b).expect("pad").rect.min.x - mk3_layout::control(&a).expect("pad").rect.min.x;
            assert!((gap - k / 100.0 * mk3_layout::WIDTH).abs() < eps, "{b} steps the fork's column pitch from {a}");
        }
    }
}

/// The fork's controls never stack: no two controls in the table overlap, so a
/// press always names exactly the control drawn there.
#[test]
fn no_two_controls_overlap() {
    let controls = mk3_layout::controls();
    for (i, a) in controls.iter().enumerate() {
        for b in &controls[i + 1..] {
            // The wheel is one 4-D control drawn on the jog, not beside it: its push and
            // its four directions are meant to overlap the disc, so those pairs are the
            // one documented exception. Everything else must still be disjoint.
            let ring = |s: &str| s.starts_with("BUTTON_WHEEL");
            if (a.id == "jog" && ring(b.id)) || (b.id == "jog" && ring(a.id)) {
                continue;
            }
            assert!(!a.rect.intersects(b.rect), "{} overlaps {}", a.id, b.id);
        }
    }
}

#[test]
fn as_json_carries_the_same_table_the_painter_reads() {
    let json = mk3_layout::as_json();
    assert_eq!(json["width"], mk3_layout::WIDTH, "the page learns the panel's width");
    assert_eq!(json["height"], mk3_layout::HEIGHT, "the page learns the panel's height");
    let list = json["controls"].as_array().expect("controls is a list");
    assert_eq!(list.len(), mk3_layout::controls().len(), "one JSON row per control");
    let pad1 = list.iter().find(|c| c["id"] == "pad1").expect("pad1 in JSON");
    assert_eq!(pad1["kind"], "pad", "kinds serialise lowercase");
    assert_eq!(pad1["cap"], "rgb", "pads carry the fork's dark rgb cap");
    assert!(pad1["rect"]["w"].as_f64().unwrap_or(0.0) > 0.0, "rects carry x, y, w, h");
    let channel = list.iter().find(|c| c["id"] == "BUTTON_CHANNEL").expect("Channel in JSON");
    assert_eq!(channel["cap"], "silicon", "plain buttons carry the default silicon cap");
}
