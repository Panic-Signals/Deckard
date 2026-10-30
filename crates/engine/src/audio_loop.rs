//! The Audio plug-in's player (docs/audio-plugin.md): a loop kept in time with the
//! song. The timeline cues it on where a pattern plays it ([`LoopRt::cue`]: which take,
//! and the song beat its first beat falls on); from then its place in the take follows
//! the song (the beat since the cue, modulo the take's length in beats), whatever the
//! tempo:
//!
//! - **Re-pitch** reads the take faster or slower (the pitch moves with the speed);
//! - **Stretch** time-stretches it (`deckard_stretch`), its pitch moved only by Tune,
//!   the pitch wheel and, in Gate mode, the note;
//! - **Formant** as Stretch, the formants kept where they were.
//!
//! In Loop mode it sounds while cued; in Gate mode only during notes (fading in and out
//! over FADE), the note's distance from C3 (60) transposing it with Stretch and Formant.
//!
//! The stretcher runs ahead of the song by its latency: on a cue, a jump, or when its
//! place drifts from the song's by more than a little, it starts over there (pre-rolled
//! with `seek`, its first output's latency discarded).

use crate::sample::Sample;
use deckard_model::{AudioEngine, AudioMode, MidiKind};
use deckard_stretch::Stretch;
use std::sync::Arc;

/// Frames rendered at once (the engine's blocks are split to this).
const CHUNK: usize = 512;
/// Fade when the loop stops or starts outside Gate mode (no click), seconds.
const EDGE_FADE: f64 = 0.004;
/// Gate mode's FADE at its top (normalized 1), seconds. [Maschine's scale is
/// unconfirmed; its new 0.1222 makes about 12 ms.]
const FADE_MAX: f64 = 0.1;

pub struct TakeRt {
    pub sample: Arc<Sample>,
    /// Frames played [start, end).
    pub range: (usize, usize),
    /// Its length in beats.
    pub beats: f64,
}

impl TakeRt {
    fn len(&self) -> usize {
        self.range.1.saturating_sub(self.range.0).max(1)
    }

    /// Frame `pos` of the played range, looping.
    fn at(&self, pos: f64) -> [f32; 2] {
        let len = self.len() as f64;
        let p = pos.rem_euclid(len);
        let i = p.floor() as usize;
        let t = (p - i as f64) as f32;
        let f = &self.sample.frames;
        let a = f.get(self.range.0 + i).copied().unwrap_or([0.0; 2]);
        let b = f.get(self.range.0 + (i + 1) % self.len()).copied().unwrap_or([0.0; 2]);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }
}

pub struct Settings {
    pub mode: AudioMode,
    pub engine: AudioEngine,
    /// Semitones.
    pub tune: f64,
    /// The pitch wheel's full throw, semitones.
    pub bend_range: f64,
    /// Gate mode's fade, normalized.
    pub fade: f64,
}

pub struct LoopRt {
    takes: Vec<TakeRt>,
    settings: Settings,
    sr: f64,
    /// The song (beats, tempo) at the start of the next render.
    beats: f64,
    tempo: f64,
    /// Playing: the take, and the song beat its first beat fell on.
    cued: Option<(usize, f64)>,
    /// Gate mode's note (pitch), if one is held; the pitch wheel (−1…1).
    gate: Option<i64>,
    bend: f64,
    /// The loudness now and where it's going (fades between).
    gain: f32,
    stretch: Option<Stretch>,
    /// The take position the stretcher is fed from next (frames of the played range),
    /// and the fraction of a frame owed.
    feed: f64,
    synced: bool,
    scratch: [Vec<f32>; 4],
}

impl LoopRt {
    pub fn new(takes: Vec<TakeRt>, settings: Settings, sr: f64) -> LoopRt {
        let stretch = (settings.engine != AudioEngine::Repitch).then(|| Stretch::new(sr as f32, false));
        // Room for the most input a chunk can need (a take played up to 8× faster),
        // and the pre-roll, made here off the audio thread.
        let room = CHUNK * 8 + 16_384;
        LoopRt {
            takes,
            settings,
            sr,
            beats: 0.0,
            tempo: 120.0,
            cued: None,
            gate: None,
            bend: 0.0,
            gain: 0.0,
            stretch,
            feed: 0.0,
            synced: false,
            scratch: std::array::from_fn(|_| vec![0.0; room]),
        }
    }

    /// Where the song is and its tempo, at the start of the next render.
    pub fn set_song(&mut self, beats: f64, tempo: f64) {
        self.beats = beats;
        if tempo > 0.0 {
            self.tempo = tempo;
        }
    }

    /// Play take `take` from now (its first beat on song beat `origin`), or stop.
    pub fn cue(&mut self, cue: Option<(usize, f64)>) {
        if cue.is_some_and(|(t, _)| t >= self.takes.len()) {
            return;
        }
        if cue.map(|c| c.0) != self.cued.map(|c| c.0) {
            self.synced = false;
        }
        self.cued = cue;
    }

    /// The take playing, if any.
    pub fn cued(&self) -> Option<usize> {
        self.cued.map(|c| c.0)
    }

    pub fn note_on(&mut self, note: i64) {
        self.gate = Some(note);
    }

    pub fn note_off(&mut self, note: i64) {
        if self.gate == Some(note) {
            self.gate = None;
        }
    }

    pub fn control(&mut self, kind: MidiKind) {
        if let MidiKind::PitchBend(v) = kind {
            self.bend = f64::from(v) / 8192.0 - 1.0;
        }
    }

    /// Stop (the transport stopped): fade out, forget the cue.
    pub fn stop(&mut self) {
        self.cued = None;
        self.gate = None;
    }

    pub fn runs(&self) -> bool {
        self.cued.is_some() || self.gain > 0.0
    }

    /// Keep `old`'s place (the same sound before an edit), if it plays the same take.
    pub fn adopt(&mut self, old: LoopRt) {
        self.cued = old.cued;
        self.gate = old.gate;
        self.bend = old.bend;
        self.gain = old.gain;
        let same = |a: &TakeRt, b: &TakeRt| Arc::ptr_eq(&a.sample, &b.sample) && a.range == b.range;
        let take = old.cued.map(|c| c.0);
        let kept = take.is_some_and(|t| old.takes.get(t).zip(self.takes.get(t)).is_some_and(|(a, b)| same(a, b)));
        if kept && old.settings.engine == self.settings.engine {
            self.stretch = old.stretch;
            self.feed = old.feed;
            self.synced = old.synced;
        }
    }

    /// Source frames per output frame now.
    fn rate(&self, t: &TakeRt) -> f64 {
        t.len() as f64 / t.beats.max(1e-6) * self.tempo / 60.0 / self.sr
    }

    /// Where in the take (frames of its range) the song is at `beats`.
    fn place(&self, t: &TakeRt, origin: f64, beats: f64) -> f64 {
        ((beats - origin).rem_euclid(t.beats.max(1e-6))) / t.beats.max(1e-6) * t.len() as f64
    }

    /// The transposition now, semitones (not with Re-pitch: its pitch is its speed).
    fn semitones(&self) -> f64 {
        let note = match (self.settings.mode, self.gate) {
            (AudioMode::Gate, Some(n)) => (n - 60) as f64,
            _ => 0.0,
        };
        self.settings.tune + note + self.bend * self.settings.bend_range
    }

    /// Render into interleaved `out` (adding nothing where it's silent); returns the
    /// peak (left, right).
    pub fn render(&mut self, out: &mut [f32]) -> (f32, f32) {
        let frames = out.len() / 2;
        let mut peak = (0.0f32, 0.0f32);
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(CHUNK);
            let chunk = &mut out[done * 2..(done + n) * 2];
            self.render_chunk(chunk, n);
            for f in chunk.chunks_exact(2) {
                peak = (peak.0.max(f[0].abs()), peak.1.max(f[1].abs()));
            }
            self.beats += n as f64 * self.tempo / 60.0 / self.sr;
            done += n;
        }
        peak
    }

    fn render_chunk(&mut self, out: &mut [f32], n: usize) {
        // How loud: open while cued (Loop), or during a note (Gate); fades between.
        let open = self.cued.is_some() && (self.settings.mode == AudioMode::Loop || self.gate.is_some());
        let fade_s = if self.settings.mode == AudioMode::Gate { (self.settings.fade * FADE_MAX).max(EDGE_FADE) } else { EDGE_FADE };
        let step = (1.0 / (fade_s * self.sr)) as f32;
        let target = if open { 1.0 } else { 0.0 };
        if self.gain == 0.0 && target == 0.0 {
            out.fill(0.0);
            return;
        }
        let Some((ti, origin)) = self.cued.or_else(|| self.takes.first().map(|_| (0, self.beats))) else {
            out.fill(0.0);
            return;
        };
        let Some(take) = self.takes.get(ti) else { return };
        let rate = self.rate(take);
        let at = self.place(take, origin, self.beats);
        match self.settings.engine {
            AudioEngine::Repitch => {
                for f in 0..n {
                    let [l, r] = take.at(at + f as f64 * rate);
                    out[f * 2] = l;
                    out[f * 2 + 1] = r;
                }
            }
            _ => self.render_stretched(ti, at, rate, out, n),
        }
        // The loudness, gliding to where it's going.
        for f in 0..n {
            self.gain = if self.gain < target { (self.gain + step).min(target) } else { (self.gain - step).max(target) };
            out[f * 2] *= self.gain;
            out[f * 2 + 1] *= self.gain;
        }
    }

    fn render_stretched(&mut self, ti: usize, at: f64, rate: f64, out: &mut [f32], n: usize) {
        let semis = self.semitones();
        let formant = self.settings.engine == AudioEngine::Formant;
        let Some(st) = self.stretch.as_mut() else { return };
        let take = &self.takes[ti];
        let len = take.len() as f64;
        st.set_transpose(2f64.powf(semis / 12.0) as f32);
        if formant {
            st.set_formant(1.0, true);
        }
        let (lat_in, lat_out) = st.latency();
        let [il, ir, ol, or] = &mut self.scratch;
        // Where the output now comes from in the take, as fed: compare with the song.
        let fed_at = (self.feed - lat_in as f64 - lat_out as f64 * rate).rem_euclid(len);
        let drift = (fed_at - at).abs().min(len - (fed_at - at).abs());
        if !self.synced || drift > (CHUNK as f64 * rate).max(64.0) {
            // Start over here: pre-roll ending the input latency past `at`, then the
            // output latency run through and dropped.
            st.reset();
            let pre = (lat_in * 2).max(4096).min(il.len());
            for k in 0..pre {
                let [l, r] = take.at(at + lat_in as f64 - pre as f64 + k as f64);
                il[k] = l;
                ir[k] = r;
            }
            st.seek(&il[..pre], &ir[..pre], rate);
            self.feed = at + lat_in as f64;
            let m = ((lat_out as f64 * rate).round() as usize).min(il.len());
            let drop_n = lat_out.min(ol.len());
            for k in 0..m {
                let [l, r] = take.at(self.feed + k as f64);
                il[k] = l;
                ir[k] = r;
            }
            st.process(&il[..m], &ir[..m], &mut ol[..drop_n], &mut or[..drop_n]);
            self.feed += m as f64;
            self.synced = true;
        }
        // This chunk's input: as much as the rate asks (whole frames, the rest owed).
        let want = n as f64 * rate;
        let start = self.feed.floor();
        let m = ((self.feed + want).floor() - start).max(0.0) as usize;
        let m = m.min(il.len());
        for k in 0..m {
            let [l, r] = take.at(start + k as f64);
            il[k] = l;
            ir[k] = r;
        }
        self.feed = (self.feed + want).rem_euclid(len);
        st.process(&il[..m], &ir[..m], &mut ol[..n], &mut or[..n]);
        for f in 0..n {
            out[f * 2] = ol[f];
            out[f * 2 + 1] = or[f];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 44_100.0;

    /// A take `beats` long at `bpm`: a quiet 440 Hz tone, loud (a 2 kHz burst) for the
    /// first 50 ms of its first beat only.
    fn take(bpm: f64, beats: f64) -> TakeRt {
        let frames = (SR * 60.0 / bpm * beats) as usize;
        let mark = (SR * 0.05) as usize;
        let tone = (0..frames)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let v = (t * 440.0 * std::f32::consts::TAU).sin() * 0.2
                    + if i < mark { (t * 2000.0 * std::f32::consts::TAU).sin() * 0.7 } else { 0.0 };
                [v, v]
            })
            .collect();
        TakeRt { sample: Arc::new(Sample { rate: SR as u32, frames: tone }), range: (0, frames), beats }
    }

    fn player(engine: AudioEngine, mode: AudioMode) -> LoopRt {
        LoopRt::new(vec![take(94.0, 4.0)], Settings { mode, engine, tune: 0.0, bend_range: 2.0, fade: 0.1222 }, SR)
    }

    /// Play `seconds` of the song at `tempo` from beat 0, in 256-frame blocks.
    fn play(p: &mut LoopRt, tempo: f64, seconds: f64) -> Vec<f32> {
        p.set_song(0.0, tempo);
        p.cue(Some((0, 0.0)));
        let mut out = vec![0.0; (seconds * SR) as usize * 2];
        for block in out.chunks_mut(512) {
            p.render(block);
        }
        out.chunks_exact(2).map(|f| f[0]).collect()
    }

    /// Frames between the loud marks: the loop's length as heard. The level in 256-frame
    /// windows; a mark where it rises past half the loudest.
    fn loop_frames(x: &[f32]) -> f64 {
        let lv: Vec<f32> = x.chunks(256).map(|c| c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).collect();
        let top = lv.iter().cloned().fold(0.0, f32::max);
        let marks: Vec<usize> = (1..lv.len()).filter(|&i| lv[i] > top * 0.5 && lv[i - 1] <= top * 0.5).map(|i| i * 256).collect();
        let gaps: Vec<usize> = marks.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(!gaps.is_empty(), "loop marks heard: {marks:?}");
        gaps.iter().sum::<usize>() as f64 / gaps.len() as f64
    }

    /// The tone's frequency: rising zero crossings, away from the marks.
    fn tone_hz(x: &[f32]) -> f64 {
        let quiet: Vec<f32> = x.iter().map(|v| if v.abs() > 0.3 { 0.0 } else { *v }).collect();
        let mut n = 0;
        let mut span = 0;
        for (i, w) in quiet.windows(2).enumerate().skip(SR as usize / 2) {
            // Skip where the marks are (a burst then is louder than the tone).
            if x[i.saturating_sub(4000)..(i + 4000).min(x.len())].iter().any(|v| v.abs() > 0.3) {
                continue;
            }
            span += 1;
            if w[0] <= 0.0 && w[1] > 0.0 {
                n += 1;
            }
        }
        n as f64 * SR / span.max(1) as f64
    }

    /// A 94 BPM loop in a 120 BPM song plays at 120: Re-pitch faster and higher, Stretch
    /// and Formant at the same pitch.
    #[test]
    fn a_loop_follows_the_songs_tempo() {
        // Four beats at 120 BPM.
        let song_loop = SR * 60.0 / 120.0 * 4.0;
        for engine in [AudioEngine::Repitch, AudioEngine::Stretch, AudioEngine::Formant] {
            let x = play(&mut player(engine, AudioMode::Loop), 120.0, 8.0);
            let heard = loop_frames(&x);
            assert!((heard - song_loop).abs() < 300.0, "{engine:?}: loops every {heard} frames, not {song_loop}");
            let hz = tone_hz(&x);
            let want = if engine == AudioEngine::Repitch { 440.0 * 120.0 / 94.0 } else { 440.0 };
            assert!((hz - want).abs() < want * 0.03, "{engine:?}: {hz} Hz, not {want}");
        }
    }

    /// Tune moves Stretch's pitch, not its tempo; Gate mode is silent without a note,
    /// and a note an octave over C3 plays it an octave up.
    #[test]
    fn tune_and_gate_notes_pitch_it() {
        let mut p = player(AudioEngine::Stretch, AudioMode::Loop);
        p.settings.tune = 12.0;
        let x = play(&mut p, 94.0, 3.0);
        assert!((tone_hz(&x) - 880.0).abs() < 30.0, "{}", tone_hz(&x));
        let mut g = player(AudioEngine::Stretch, AudioMode::Gate);
        let silent = play(&mut g, 94.0, 1.0);
        assert!(silent.iter().all(|v| v.abs() < 1e-6), "gate closed");
        g.note_on(72);
        let mut out = vec![0.0; (3.0 * SR) as usize * 2];
        for block in out.chunks_mut(512) {
            g.render(block);
        }
        let x: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
        assert!((tone_hz(&x) - 880.0).abs() < 30.0, "{}", tone_hz(&x));
    }

    /// The song jumping (a pattern starting over early) moves the loop with it.
    #[test]
    fn it_follows_the_song_when_it_jumps() {
        let mut p = player(AudioEngine::Stretch, AudioMode::Loop);
        play(&mut p, 120.0, 1.0);
        // Back to the cue's beat: the first beat's click again soon after.
        p.set_song(0.0, 120.0);
        let mut out = vec![0.0; 4096 * 2];
        p.render(&mut out);
        let first = out.chunks_exact(2).position(|f| f[0].abs() > 0.5).expect("the mark");
        assert!(first < 600, "the loop's start, at {first}");
    }
}
