//! The controller's RECORD screen knobs: the recorder's settings as the RECORD tab's
//! footer has them (RECORDING, INPUT, then the Auto Sampler's NOTE MAP, VELOCITY MAP
//! and BATCH PROCESS), in pages of up to four for NI's Sampling page
//! (`State::record_pages`; knobs 5 and 6 zoom and scroll), and what turning each sets.

use crate::layout::sampling::{RecParam, RecorderView, ms_text, threshold_text};
use crate::recorder as r;
use crate::settings::Settings;
use deckard_lua_host::state::{KnobPage, SamplingKnob};
use deckard_model::Project;
use std::collections::HashMap;

/// A choice moves an option, a number a step, every 32nd of a turn.
const TURN: f64 = 1.0 / 32.0;
/// The threshold: half a dB every 120th of a turn (its whole range in a turn).
const THRESHOLD_TURN: f64 = 1.0 / 120.0;

/// What turning a knob sets.
#[derive(Debug, Clone, PartialEq)]
pub enum Apply {
    /// A setting: to `values[value]` for a choice with values of its own, else to the
    /// value itself.
    Param(RecParam, Option<Vec<i64>>),
    Threshold,
    MidiTo(Vec<Option<String>>),
}

#[derive(Default)]
pub struct Knobs {
    pub pages: Vec<KnobPage>,
    pub apply: HashMap<String, Apply>,
}

impl Knobs {
    fn page(&mut self, name: &str, knobs: Vec<(SamplingKnob, Apply)>) {
        let knobs = knobs
            .into_iter()
            .map(|(k, a)| {
                self.apply.insert(k.key.clone(), a);
                k
            })
            .collect();
        self.pages.push(KnobPage { name: name.into(), knobs });
    }
}

fn choice(key: &str, name: &str, options: &[String], at: usize, apply: Apply) -> (SamplingKnob, Apply) {
    let text = options.get(at).cloned().unwrap_or_default();
    let max = options.len().saturating_sub(1) as f64;
    (SamplingKnob { key: key.into(), name: name.into(), text, value: at as f64, min: 0.0, max, step: 1.0, turn: TURN }, apply)
}

fn number(key: &str, name: &str, text: String, v: i64, (lo, hi, by): (i64, i64, i64), p: RecParam) -> (SamplingKnob, Apply) {
    let k = SamplingKnob {
        key: key.into(),
        name: name.into(),
        text,
        value: v as f64,
        min: lo as f64,
        max: hi as f64,
        step: by as f64,
        turn: TURN,
    };
    (k, Apply::Param(p, None))
}

fn toggle(key: &str, name: &str, on: bool, p: RecParam) -> (SamplingKnob, Apply) {
    choice(key, name, &["Off".into(), "On".into()], usize::from(on), Apply::Param(p, None))
}

fn names(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

pub fn knobs(rec: &RecorderView, io: &Settings, p: &Project) -> Knobs {
    let mut out = Knobs::default();
    let a = &rec.auto;
    let auto = rec.mode == 3;
    let external = rec.source < 2;

    let mut recording = vec![choice("Mode", "MODE", &names(&r::MODES), rec.mode, Apply::Param(RecParam::Mode, None))];
    let length = || choice("Length", "LENGTH", &names(&r::LENGTHS), rec.length, Apply::Param(RecParam::Length, None));
    match rec.mode {
        0 => recording.push((
            SamplingKnob {
                key: "Threshold".into(),
                name: "THRESHOLD".into(),
                text: threshold_text(rec.threshold),
                value: rec.threshold,
                min: -60.0,
                max: 0.0,
                step: 0.5,
                turn: THRESHOLD_TURN,
            },
            Apply::Threshold,
        )),
        1 => recording.push(length()),
        2 => {
            recording.push(length());
            recording.push(choice("Target", "TARGET", &names(&r::TARGETS), rec.target, Apply::Param(RecParam::Target, None)));
        }
        _ => {
            let ms = (100, 30_000, 100);
            recording.push(number("SampleMs", "SAMPLE", ms_text(a.sample_ms), a.sample_ms.into(), ms, RecParam::SampleMs));
            recording.push(number("NoteOnMs", "NOTE ON", ms_text(a.note_on_ms), a.note_on_ms.into(), ms, RecParam::NoteOnMs));
        }
    }
    out.page("RECORDING", recording);

    let mut input = vec![choice("Source", "SOURCE", &names(&r::SOURCES), rec.source, Apply::Param(RecParam::Source, None))];
    match rec.source {
        // The connected inputs (and the one picked), by their aliases.
        0 | 1 => {
            let stereo = rec.source == 0;
            let (cur, n) = if stereo { (rec.ext_stereo, r::INPUT_PAIRS) } else { (rec.ext_mono, r::INPUT_PAIRS * 2) };
            let shown: Vec<usize> = (0..n).filter(|&i| i == cur || if stereo { io.input_pair_on(i) } else { io.input_on(i) }).collect();
            let labels: Vec<String> = shown.iter().map(|&i| if stereo { io.input_pair_name(i) } else { io.input_name(i) }).collect();
            let at = shown.iter().position(|&i| i == cur).unwrap_or(0);
            let p = if stereo { RecParam::ExtStereo } else { RecParam::ExtMono };
            input.push(choice("Input", "INPUT", &labels, at, Apply::Param(p, Some(shown.iter().map(|&i| i as i64).collect()))));
        }
        // A sound to play and record: those with an instrument or a Sampler.
        _ if auto => {
            let mut opts = vec![("None".to_string(), -1i64)];
            for (g, gr) in p.groups.iter().enumerate() {
                for (s, snd) in gr.sounds.iter().enumerate() {
                    if !snd.slots.is_empty() || snd.sampler.is_some() {
                        opts.push((format!("{}-{} {}", crate::layout::group_id(g), s + 1, snd.name), (g * 16 + s) as i64));
                    }
                }
            }
            let at = rec.auto_sound.map_or(0, |(g, s)| opts.iter().position(|o| o.1 == (g * 16 + s) as i64).unwrap_or(0));
            let (labels, values): (Vec<String>, Vec<i64>) = opts.into_iter().unzip();
            input.push(choice("Input", "INPUT", &labels, at, Apply::Param(RecParam::AutoSound, Some(values))));
        }
        _ => {
            let mut labels = vec!["Master".to_string()];
            labels.extend(p.groups.iter().enumerate().map(|(g, gr)| format!("{} - {}", crate::layout::group_id(g), gr.name)));
            let values = (-1..p.groups.len() as i64).collect();
            input.push(choice(
                "Input",
                "INPUT",
                &labels,
                rec.internal.map_or(0, |g| g + 1),
                Apply::Param(RecParam::Internal, Some(values)),
            ));
        }
    }
    if external {
        input.push(toggle("Monitor", "MONITOR", rec.monitor, RecParam::Monitor));
    }
    out.page("INPUT", input);

    if auto {
        if external {
            let mut ports: Vec<Option<String>> = vec![None];
            ports.extend(rec.midi_outs.iter().cloned().map(Some));
            let labels: Vec<String> = ports.iter().map(|p| p.clone().unwrap_or_else(|| "None".into())).collect();
            let at = a.midi_to.as_ref().and_then(|m| ports.iter().position(|p| p.as_ref() == Some(m))).unwrap_or(0);
            out.page(
                "MIDI",
                vec![
                    choice("MidiTo", "MIDI TO", &labels, at, Apply::MidiTo(ports)),
                    number("Channel", "CHANNEL", a.channel.to_string(), a.channel.into(), (1, 16, 1), RecParam::Channel),
                ],
            );
        }
        let note = |n: u8| r::note_name(n);
        out.page(
            "NOTE MAP",
            vec![
                number("KeyStride", "STRIDE", a.key_stride.to_string(), a.key_stride.into(), (1, 127, 1), RecParam::KeyStride),
                number("KeyLo", "KEY LO", note(a.key_lo), a.key_lo.into(), (0, 127, 1), RecParam::KeyLo),
                number("KeyHi", "KEY HI", note(a.key_hi), a.key_hi.into(), (0, 127, 1), RecParam::KeyHi),
                toggle("KeyExtend", "EXTEND", a.key_extend, RecParam::KeyExtend),
            ],
        );
        out.page(
            "VELOCITY MAP",
            vec![
                number("VelStride", "STRIDE", a.vel_stride.to_string(), a.vel_stride.into(), (1, 127, 1), RecParam::VelStride),
                number("VelLo", "VEL LO", a.vel_lo.to_string(), a.vel_lo.into(), (0, 127, 1), RecParam::VelLo),
                number("VelHi", "VEL HI", a.vel_hi.to_string(), a.vel_hi.into(), (0, 127, 1), RecParam::VelHi),
                toggle("VelExtend", "EXTEND", a.vel_extend, RecParam::VelExtend),
            ],
        );
        out.page(
            "BATCH PROCESS",
            vec![
                toggle("FindLoop", "FIND LOOP", a.find_loop, RecParam::FindLoop),
                toggle("TrimSilence", "TRIM SILENCE", a.trim_silence, RecParam::TrimSilence),
                toggle("Normalize", "NORMALIZE", a.normalize, RecParam::Normalize),
            ],
        );
    }
    out
}
