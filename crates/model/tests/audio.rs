//! The Audio plug-in as Maschine saves it (docs/audio-plugin.md), read from one of its
//! own projects (Solar Breeze's "Rainbow Gun"; skipped if it isn't installed).

use deckard_model::{AudioEngine, AudioMode, Project};

fn rainbow_gun() -> Option<Project> {
    let path = "/Users/Shared/Solar Breeze Library/Projects/Rainbow Gun.mxprj";
    let bytes = std::fs::read(path).map_err(|_| eprintln!("{path} not found; skipping")).ok()?;
    let doc = mxprj::Project::parse(&bytes).unwrap();
    let types = mxprj::types::Types::new();
    Some(Project::from_decoded(doc.decode(&types).unwrap()))
}

/// Its bass loop: one take of "Bass[94] Amber.wav", 16 beats (94 BPM over its
/// 450 372 frames), Loop mode, the Stretch engine; on in the patterns that play it.
#[test]
fn the_audio_plugins_loop_reads_as_maschine_saved_it() {
    let Some(p) = rainbow_gun() else { return };
    let (g, s, a) = p
        .groups
        .iter()
        .enumerate()
        .flat_map(|(g, gr)| gr.sounds.iter().enumerate().filter_map(move |(s, snd)| Some((g, s, snd.audio.clone()?))))
        .find(|(_, _, a)| a.takes.iter().any(|t| t.sample.path.ends_with("Bass[94] Amber.wav")))
        .expect("the bass loop");
    assert_eq!(a.takes.len(), 1);
    assert_eq!(a.takes[0].beats, 16.0);
    assert!((a.takes[0].tempo(450_372, 44_100) - 94.0).abs() < 0.01);
    assert_eq!(a.takes[0].play_range, (0, 450_372));
    assert_eq!((a.mode, a.engine), (AudioMode::Loop, AudioEngine::Stretch));
    assert_eq!(p.groups[g].sounds[s].instrument, "AudioModule");
    let on: Vec<usize> =
        p.groups[g].patterns.iter().enumerate().filter(|(_, pat)| pat.audio.iter().any(|c| c.sound == s)).map(|(i, _)| i).collect();
    assert!(!on.is_empty(), "on in some patterns");
    assert!(p.groups[g].patterns.iter().flat_map(|pat| &pat.audio).all(|c| c.take < 2));
}
