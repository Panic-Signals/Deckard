//! The Audio plug-in's page (docs/audio-plugin.md), as Maschine's panel lays it out:
//! PLAYBACK (MODE, FADE, TUNE, PITCHBEND), ENGINE, SOURCE (TEMPO, LENGTH of the take in
//! focus). The software's chain area and the controller's Control page show it.

use crate::App;
use crate::layout::{Edit, Knob};
use deckard_doc::Target;
use deckard_model::{AudioEngine, AudioLoop, AudioMode};
use std::borrow::Cow;

pub const MODES: &[&str] = &["Loop", "Gate"];
pub const ENGINES: &[&str] = &["Re-Pitch", "Stretch", "Formant"];
/// Gate mode's FADE at its top (normalized 1), ms (as the player: `audio_loop`).
const FADE_MS: f64 = 100.0;

pub struct Page {
    pub knobs: Vec<Option<Knob>>,
    pub sections: Vec<(usize, Cow<'static, str>)>,
}

/// The page of sound (`g`, `s`)'s Audio plug-in `a`, in chain slot `i`.
pub fn page(app: &App, a: &AudioLoop, g: usize, s: usize, i: usize) -> Page {
    let t = Target::Module(g, s, i);
    let mode = usize::from(a.mode == AudioMode::Gate);
    let engine = match a.engine {
        AudioEngine::Repitch => 0,
        AudioEngine::Stretch => 1,
        AudioEngine::Formant => 2,
    };
    let k = a.focused.min(a.takes.len().saturating_sub(1));
    let tt = Target::Take(g, s, k);
    let beats = a.takes.get(k).map_or(0.0, |t| t.beats);
    // The take's tempo, from its frames (once the app has looked at it).
    let tempo = app.take_info.filter(|(at, ..)| *at == (g, s, k)).map(|(_, frames, rate)| a.takes[k].tempo(frames, rate));
    let mut knobs: Vec<Option<Knob>> = vec![
        Some(("MODE".into(), MODES[mode].into(), mode as f32, Some(Edit::Choice(t, "mode", MODES, mode, 0.0)))),
        Some((
            "FADE".into(),
            if a.mode == AudioMode::Gate { format!("{:.0} ms", a.fade * FADE_MS) } else { "—".into() },
            a.fade as f32,
            Some(Edit::Param(t, "fade", 0.0, 1.0, 0.1222)),
        )),
        Some((
            "TUNE".into(),
            format!("{:+.2} st", a.tune),
            ((a.tune + 36.0) / 72.0) as f32,
            Some(Edit::Param(t, "tune", -36.0, 36.0, 0.0)),
        )),
        Some((
            "PITCHBEND".into(),
            format!("{:+.1} st", (a.pitchbend - 0.5) * 24.0),
            a.pitchbend as f32,
            Some(Edit::Param(t, "pitchbend", 0.0, 1.0, 0.5)),
        )),
        Some(("ENGINE".into(), ENGINES[engine].into(), engine as f32 / 2.0, Some(Edit::Choice(t, "engine", ENGINES, engine, 1.0)))),
        None,
        None,
        None,
    ];
    if !a.takes.is_empty() {
        knobs[5] = tempo.map(|bpm| {
            ("TEMPO".into(), format!("{bpm:.2}"), ((bpm - 40.0) / 260.0) as f32, Some(Edit::Param(tt, "tempo", 40.0, 300.0, bpm)))
        });
        knobs[6] =
            Some(("LENGTH".into(), format!("{beats:.2} beats"), (beats / 128.0) as f32, Some(Edit::Int(tt, "length", 1.0, 128.0, beats))));
    }
    Page { knobs, sections: vec![(0, "Playback".into()), (4, "Engine".into()), (5, "Source".into())] }
}
