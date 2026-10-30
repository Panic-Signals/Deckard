use crate::*;

/// A two-group project, its first group focused, its first four sounds used.
pub fn view() -> View {
    let sound = |n: usize| SoundView {
        name: format!("Sound {n}"),
        rgb: [255, 66, 62],
        color: 0,
        muted: false,
        used: n < 4,
        playing: false,
        choke_group: 0,
        choke_mode: 0,
        link_group: 0,
        link_mode: 0,
        base_key: 60,
        touch_cc: 1,
        touch_curve: 50,
        touch_max: 100,
        ..Default::default()
    };
    let group = |n: &str, rgb| GroupView {
        name: n.into(),
        rgb,
        color: 1,
        muted: false,
        playing: false,
        sounds: (1..=16).map(sound).collect(),
        ..Default::default()
    };
    View {
        project: "Lemondrop".into(),
        tempo: 120.0,
        swing: 0.0,
        master_volume: 0.75,
        strip_position: 2,
        numerator: 4,
        denominator: 4,
        groups: vec![group("Drums", [255, 66, 62]), group("Bass", [77, 148, 255])],
        control: ControlView {
            modules_visible: true,
            page_groups: 4,
            slot: Some("Compressor".into()),
            slot_count: 1,
            slot_index: Some(0),
            chain: vec![crate::SlotView { name: "Compressor".into(), instrument: false, active: true, plugin: false, picture: None }],
            pages: vec!["Main".into(), "Side-chain".into()],
            knobs: (0..8)
                .map(|k| {
                    (k < 6).then(|| KnobView {
                        name: format!("Knob {k}"),
                        section: "Main".into(),
                        text: "50 %".into(),
                        value: 0.5,
                        ..Default::default()
                    })
                })
                .collect(),
            ..Default::default()
        },
        // (Two LFOs, on pads 1 and 2, the first focused.)
        lfos: (1..=2)
            .map(|pad| LfoView {
                pad,
                shape: "Sine".into(),
                sync: true,
                rate: "1/4".into(),
                level: 1.0,
                trigger: "Off".into(),
                fade: 1.0,
                name: format!("LFO {pad}"),
                points: (0..240).map(|i| (i as f32 / 240.0 * 4.0 * std::f32::consts::PI).sin() * 0.5 + 0.5).collect(),
                ..Default::default()
            })
            .collect(),
        lfo_focus: 1,
        // (Two routes, then the empty slot.)
        mod_slots: vec![
            ModSlotView {
                rows: ["LFO 1", "A1 – Drums", "S1 – Sound 1", "Compressor", "Knob 0", "+50%"].map(String::from), midi: false
            },
            ModSlotView {
                rows: ["Pressure", "A1 – Drums", "S2 – Sound 2", "Compressor", "Knob 1", "−25%"].map(String::from), midi: false
            },
            ModSlotView { rows: ["—"; 6].map(String::from), midi: false },
        ],
        ..Default::default()
    }
}
