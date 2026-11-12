//! A sample's root note: the note it plays at, so Keys mode can put it on its own key and
//! Tune can move it to another.

use deckard_library::names::note_in_name;
use deckard_plugins::zhora::pitch::{WINDOW, detect};

use crate::mono;

/// A sample's root note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Root {
    /// MIDI note (60 is C3).
    pub note: u8,
    /// How far the sample sits from `note`: −50 to +50.
    pub cents: f32,
    /// How sure: 0 to 1; 1 when the file's name gives the note.
    pub confidence: f32,
}

/// Above this, pairs of samples are averaged down so a window still spans low notes.
const MAX_RATE: u32 = 60_000;
/// Windows quieter than this (RMS) don't count: −50 dBFS.
const MIN_RMS: f32 = 0.003_162;
/// The fewest detections that may name a root, and the least share of the sound.
const MIN_VOTES: usize = 3;
const MIN_CONFIDENCE: f32 = 0.4;

/// The root of `frames` at `rate`, or `None` when it has no clear pitch (drums, noise,
/// silence): better unset than a guess.
pub fn root(frames: &[[f32; 2]], rate: u32) -> Option<Root> {
    let (mut x, mut rate) = (mono(frames), rate);
    if rate == 0 {
        return None;
    }
    while rate > MAX_RATE {
        x = x.chunks(2).map(|p| p.iter().sum::<f32>() / p.len() as f32).collect();
        rate /= 2;
    }
    // (note, cents, weight) per periodic window, and the weight of every loud window.
    let mut votes: Vec<(u8, f32, f32)> = Vec::new();
    let mut loud = 0f32;
    let mut at = 0;
    while at + WINDOW <= x.len() {
        let w = &x[at..at + WINDOW];
        at += WINDOW / 2;
        let rms = (w.iter().map(|s| s * s).sum::<f32>() / WINDOW as f32).sqrt();
        if rms < MIN_RMS {
            continue;
        }
        loud += rms;
        if let Some(p) = detect(w, rate as f32) {
            let exact = 69.0 + 12.0 * (f64::from(p.hz) / 440.0).log2();
            let note = exact.round();
            if (0.0..=127.0).contains(&note) {
                votes.push((note as u8, (100.0 * (exact - note)) as f32, rms * p.clarity));
            }
        }
    }
    let mut weight = [0f32; 128];
    let mut count = [0usize; 128];
    for &(n, _, w) in &votes {
        weight[n as usize] += w;
        count[n as usize] += 1;
    }
    let note = (0..128).max_by(|&a, &b| weight[a].total_cmp(&weight[b]))?;
    let confidence = if loud > 0.0 { (weight[note] / loud).min(1.0) } else { 0.0 };
    if count[note] < MIN_VOTES || confidence < MIN_CONFIDENCE {
        return None;
    }
    // The weighted median of the winner's cents.
    let mut cents: Vec<(f32, f32)> = votes.iter().filter(|v| v.0 as usize == note).map(|v| (v.1, v.2)).collect();
    cents.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut acc = 0.0;
    let median = cents.iter().find(|c| {
        acc += c.1;
        acc >= weight[note] / 2.0
    });
    Some(Root { note: note as u8, cents: median.map_or(0.0, |c| c.0), confidence })
}

/// The root as [`root`] hears it, but a note in the file's name wins: it sets the pitch
/// class, and the octave too if written (else the heard note's nearest octave, else C3's).
/// The cents are the heard ones when the heard note agrees, else 0.
pub fn root_with_name(frames: &[[f32; 2]], rate: u32, name: &str) -> Option<Root> {
    let heard = root(frames, rate);
    let Some((class, octave)) = note_in_name(name) else {
        return heard;
    };
    let class = i32::from(class);
    let note = match (octave, heard) {
        (Some(o), _) => 12 * (i32::from(o) + 2) + class,
        (None, Some(h)) => {
            let h = i32::from(h.note);
            let below = h - (h - class).rem_euclid(12);
            if h - below <= below + 12 - h { below } else { below + 12 }
        }
        (None, None) => 60 + class,
    };
    let note = u8::try_from(note.clamp(0, 127)).ok()?;
    let cents = heard.filter(|h| h.note % 12 == note % 12).map_or(0.0, |h| h.cents);
    Some(Root { note, cents, confidence: 1.0 })
}

#[cfg(test)]
mod tests {
    use super::{root, root_with_name};
    use crate::test_signals::{drums, tone};

    const SR: u32 = 44_100;

    #[test]
    fn finds_a_sines_root() {
        let r = root(&tone(110.0, 2.0, SR, 1), SR).expect("a root");
        assert_eq!(r.note, 45);
        assert!(r.cents.abs() < 2.0, "{r:?}");
    }

    #[test]
    fn finds_the_cents() {
        let r = root(&tone(110.0 * 2f64.powf(-12.0 / 1200.0), 2.0, SR, 1), SR).expect("a root");
        assert_eq!(r.note, 45);
        assert!((r.cents + 12.0).abs() < 2.0, "{r:?}");
    }

    #[test]
    fn a_rich_tone_keeps_its_fundamental() {
        assert_eq!(root(&tone(130.81, 2.0, SR, 10), SR).map(|r| r.note), Some(48));
    }

    #[test]
    fn the_longest_note_wins() {
        let mut f = tone(82.41, 3.0, SR, 3);
        f.extend(tone(98.0, 1.0, SR, 3));
        assert_eq!(root(&f, SR).map(|r| r.note), Some(40));
    }

    #[test]
    fn drums_have_no_root() {
        assert_eq!(root(&drums(120.0, 4, SR, 0.5), SR), None);
    }

    #[test]
    fn silence_and_tiny_inputs() {
        assert_eq!(root(&[], SR), None);
        assert_eq!(root(&[[0.2; 2]; 100], SR), None);
        assert_eq!(root(&vec![[0.0; 2]; SR as usize], SR), None);
        assert_eq!(root(&tone(220.0, 1.0, SR, 1), 0), None);
    }

    #[test]
    fn other_rates_agree() {
        assert_eq!(root(&tone(220.0, 1.0, 96_000, 3), 96_000).map(|r| r.note), Some(57));
        assert_eq!(root(&tone(220.0, 1.0, 22_050, 3), 22_050).map(|r| r.note), Some(57));
    }

    #[test]
    fn the_name_sets_the_class() {
        let f = tone(220.0, 1.0, SR, 1);
        let a = root_with_name(&f, SR, "Pad A2.wav").unwrap();
        assert_eq!((a.note, a.confidence), (57, 1.0));
        let c = root_with_name(&f, SR, "Pad C.wav").unwrap();
        assert_eq!((c.note, c.cents), (60, 0.0));
        assert_eq!(root_with_name(&drums(120.0, 1, SR, 0.5), SR, "Loop D.wav").map(|r| r.note), Some(62));
        assert_eq!(root_with_name(&f, SR, "Pad.wav").map(|r| r.note), Some(57));
    }
}
