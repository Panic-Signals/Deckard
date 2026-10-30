//! A loop's length in beats, as the Audio plug-in needs it (its Source Length; its
//! Source Tempo follows): Maschine "determines tempo information from the Sample" on
//! loading (docs/audio-plugin.md). DECKARD reads the tempo its name gives — Maschine's
//! libraries put it in brackets ("Bass[94] Amber.wav"), others write "94bpm" or "94
//! BPM" — and failing that takes the whole number of beats whose tempo is nearest the
//! song's.

pub use deckard_library::names::tempo_in_name;

/// The loop's length in beats: from the tempo in its name, else the whole number of
/// beats nearest the song's tempo (a whole bar's worth when it's that long).
pub fn beats(name: &str, frames: usize, rate: u32, song_tempo: f64) -> f64 {
    let seconds = frames as f64 / f64::from(rate.max(1));
    if seconds <= 0.0 {
        return 4.0;
    }
    if let Some(t) = tempo_in_name(name) {
        return (seconds * t / 60.0 * 1000.0).round() / 1000.0;
    }
    let at_song = seconds * song_tempo / 60.0;
    // Whole bars once it's a bar or more (a loop is usually bars long).
    let whole = if at_song >= 3.5 { (at_song / 4.0).round().max(1.0) * 4.0 } else { at_song.round().max(1.0) };
    whole
}

#[cfg(test)]
mod tests {
    use super::{beats, tempo_in_name};

    #[test]
    fn tempos_from_names() {
        assert_eq!(tempo_in_name("Bass[94] Amber.wav"), Some(94.0));
        assert_eq!(tempo_in_name("Synth [120] InyaDM.wav"), Some(120.0));
        assert_eq!(tempo_in_name("drums_174bpm_break.wav"), Some(174.0));
        assert_eq!(tempo_in_name("Pad 90 BPM.aif"), Some(90.0));
        assert_eq!(tempo_in_name("Kick [Hard].wav"), None);
        assert_eq!(tempo_in_name("Vocal TenYearsOff.wav"), None);
    }

    /// 16 beats of a 94 BPM loop (450 372 frames at 44.1 kHz); without a tempo in the
    /// name, the bars nearest the song's tempo.
    #[test]
    fn lengths_in_beats() {
        assert_eq!(beats("Bass[94] Amber.wav", 450_372, 44_100, 120.0), 16.0);
        // 8 s at 120: 16 beats; at 100: 13.3, so 12 (three bars).
        assert_eq!(beats("loop.wav", 8 * 44_100, 44_100, 120.0), 16.0);
        assert_eq!(beats("loop.wav", 8 * 44_100, 44_100, 100.0), 12.0);
        // Under a bar: whole beats.
        assert_eq!(beats("hit.wav", 44_100, 44_100, 120.0), 2.0);
    }
}
