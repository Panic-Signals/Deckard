use super::*;
use deckard_model::{Group, Note, Pattern, PluginRef, Scene, Section, SlotInfo, Sound};

/// A pattern played by DECKARD's own synth: no plug-in to install, and on any thread.
#[test]
fn native_synth_plays_a_pattern() {
    let class = deckard_plugins::native::CLASSES[0];
    let plugin = PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
    let notes = (0..4)
        .map(|i| Note {
            ratchet: Default::default(),
            expr: None,
            sound: 0,
            position: i * PPQ,
            length: 240,
            pitch: 48 + i,
            velocity: 110,
            probability: 100,
        })
        .collect();
    let p = Project {
        tempo: 120.0,
        numerator: 4,
        denominator: 4,
        groups: vec![Group {
            level: 0.75,
            sounds: vec![Sound {
                level: 0.75,
                instrument: "PluginHost".into(),
                plugin: Some(plugin.clone()),
                slots: vec![SlotInfo {
                    module: "PluginHost".into(),
                    name: class.name.into(),
                    active: true,
                    plugin: Some(plugin),
                    params: Vec::new(),
                }],
                ..Default::default()
            }],
            patterns: vec![Pattern { length: 4 * PPQ, notes, ..Default::default() }],
            ..Default::default()
        }],
        scenes: vec![Scene { patterns: vec![(0, 0)], ..Default::default() }],
        sections: vec![Section { scene: Some(0), length: 4 * PPQ, auto: false }],
        ..Default::default()
    };
    let mut cache = SampleCache::default();
    let (mut e, report) = std::thread::spawn(move || {
        let r = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        (r.0, r.1)
    })
    .join()
    .unwrap();
    assert!(report.plugin_errors.is_empty(), "{:?}", report.plugin_errors);
    assert!(report.no_sampler.is_empty(), "{:?}", report.no_sampler);
    let out = render(&mut e, 3.0);
    let peak = |a: usize, b: usize| out[a * 2..b * 2].iter().fold(0f32, |m, x| m.max(x.abs()));
    // Each beat (0.5 s at 120 BPM) starts a note.
    for beat in 0..4 {
        let at = beat * 24_000;
        assert!(peak(at, at + 4800) > 0.02, "beat {beat} sounds: {}", peak(at, at + 4800));
    }
    assert!(out.iter().all(|x| x.is_finite()));
}

/// A Compressor after the synth posts its input level, gain reduction and output
/// level for its panel; a low threshold shows reduction.
#[test]
fn compressor_posts_its_meters() {
    let class = deckard_plugins::native::CLASSES[0];
    let plugin = PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
    let notes: Vec<Note> = (0..4)
        .map(|i| Note {
            ratchet: Default::default(),
            expr: None,
            sound: 0,
            position: i * PPQ,
            length: 240,
            pitch: 48 + i,
            velocity: 127,
            probability: 100,
        })
        .collect();
    let comp = |threshold: f64| SlotInfo {
        module: "Compressor".into(),
        name: "Compressor".into(),
        active: true,
        plugin: None,
        params: [("mode", 0.0), ("threshold", threshold), ("amount", 1.0), ("knee", 0.5), ("attack", 0.2), ("release", 0.3), ("gain", 1.0)]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect(),
    };
    let take = |threshold: f64| {
        let p = Project {
            tempo: 120.0,
            numerator: 4,
            denominator: 4,
            groups: vec![Group {
                level: 0.75,
                sounds: vec![Sound {
                    level: 0.75,
                    instrument: "PluginHost".into(),
                    plugin: Some(plugin.clone()),
                    slots: vec![
                        SlotInfo {
                            module: "PluginHost".into(),
                            name: class.name.into(),
                            active: true,
                            plugin: Some(plugin.clone()),
                            params: Vec::new(),
                        },
                        comp(threshold),
                    ],
                    ..Default::default()
                }],
                patterns: vec![Pattern { length: 4 * PPQ, notes: notes.clone(), ..Default::default() }],
                ..Default::default()
            }],
            scenes: vec![Scene { patterns: vec![(0, 0)], ..Default::default() }],
            sections: vec![Section { scene: Some(0), length: 4 * PPQ, auto: false }],
            ..Default::default()
        };
        std::thread::spawn(move || {
            let mut cache = SampleCache::default();
            let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
            let m = Arc::new(meters::FxMeters::default());
            e.set_fx_meters(&m);
            render(&mut e, 0.4);
            m.take(0, 0, 1)
        })
        .join()
        .unwrap()
    };
    let (input, gr, output) = take(1.0); // −60 dB
    assert!(input > 0.01 && output > 0.0, "levels {input} {output}");
    assert!(gr > 6.0, "gain reduction {gr} dB");
    let (_, gr, _) = take(0.0); // 0 dB: nothing to reduce
    assert!(gr < 0.5, "gain reduction {gr} dB");
}

#[test]
fn preload_group_samples_decodes_all_sounds_in_parallel() {
    let dir = tempfile::tempdir().unwrap();
    let mut sounds = Vec::new();
    let mut wavs = Vec::new();
    for i in 0..8 {
        let path = dir.path().join(format!("pad_{i}.wav"));
        let val = (i as f32 + 1.0) * 0.05;
        edit::write_wav(&path, &Sample { rate: 48_000, frames: vec![[val, -val]; 256] }).unwrap();
        wavs.push(path.clone());
        sounds.push(Sound {
            name: format!("Pad {i}"),
            level: 1.0,
            instrument: "Sampler".into(),
            sampler: Some(deckard_model::Sampler {
                zones: vec![deckard_model::Zone {
                    sample: deckard_model::SampleRef { kind: 0, path: path.display().to_string(), ..Default::default() },
                    root_key: 60,
                    key_range: (0, 127),
                    velocity_range: (0, 127),
                    gain: 1.0,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    let group = Group { name: "Kit".into(), level: 1.0, sounds, ..Default::default() };
    let mut cache = SampleCache::default();
    let snap = cache.known_sample_state();
    let preloaded = SampleCache::preload_group_samples(&group, None, &snap);
    assert_eq!(preloaded.decoded.len(), 8);
    cache.ingest_preloaded(preloaded);
    for wav in &wavs {
        assert!(cache.has_cached(wav), "{} should be preloaded into SampleCache", wav.display());
    }
    // Once cached, a second snapshot skips re-decoding every file.
    let snap2 = cache.known_sample_state();
    let preloaded2 = SampleCache::preload_group_samples(&group, None, &snap2);
    assert!(preloaded2.decoded.is_empty());
}
