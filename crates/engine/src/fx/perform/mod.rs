//! Perform FX (docs/superpowers/plans/2026-10-22-phase5-perform-fx.md): Maschine's eight
//! performance effects in one module, played from the Smart Strip: `touchstrip_on_off`
//! engages the effect `mode` picks, `touchstrip_pos` moves it. Each effect follows
//! Maschine's own (docs/fx-perform*.md); this module is Maschine's processor around
//! them (docs/fx-perform.md): which effect hears the strip, the 20 ms crossfade on a
//! change of mode, the control ticks.

use super::Effect;

mod burst;
mod filter;
mod flanger;
mod kit;
mod reso;
mod ring;
mod scratcher;
mod stutter;
mod tremolo;

/// The module's parameters, by field (Maschine's defaults for any not given).
pub(crate) struct Params<'a>(&'a [(String, f64)]);

impl Params<'_> {
    pub fn f(&self, k: &str) -> f32 {
        self.0
            .iter()
            .find(|(n, _)| n == k)
            .map(|x| x.1)
            .or_else(|| deckard_model::PERFORM_FX.1.iter().find(|(n, _)| *n == k).map(|x| x.1))
            .unwrap_or(0.0) as f32
    }

    /// A choice's index.
    pub fn i(&self, k: &str) -> usize {
        self.f(k).round().max(0.0) as usize
    }
}

/// What an effect sees each sample.
pub(crate) struct Ctx {
    pub sr: f32,
    /// The strip's position (0–1, glided). Every effect now reads its own from its
    /// parameters, as Maschine's cores do.
    #[allow(dead_code)]
    pub pos: f32,
    pub engaged: bool,
    /// A quarter note in samples (the song's tempo).
    pub quarter: f32,
    /// The song position (quarters) at this sample while the song plays (Stutter, the
    /// one that used it, now has Maschine's own clock: [`Mode::set_position`]).
    #[allow(dead_code)]
    pub song: Option<f64>,
}

/// One of the eight effects. As Maschine's processor treats its cores: every setting
/// reaches every effect, but the strip's position and engage only the current one (an
/// effect sees them again, as they are then, when it becomes current).
pub(crate) trait Mode: Send {
    fn set(&mut self, p: &Params);
    /// One stereo sample.
    fn frame(&mut self, l: f32, r: f32, c: &Ctx) -> (f32, f32);
    /// It becomes the current effect (Maschine: reset and resumed, after the strip's
    /// values are set).
    fn activate(&mut self) {}
    /// The host's tempo (BPM, an `f32` as Maschine's processor has it), given before
    /// each block (`onBPMChanged`).
    fn set_bpm(&mut self, _bpm: f32) {}
    /// It's no longer heard: the crossfade away from it has finished, or another
    /// effect is current at load (Maschine: suspended; its settings then land at once).
    fn deactivate(&mut self) {}
    /// The song position (quarters) at the next block's start, and whether the song
    /// plays (`onPositionChanged`): given to the current effect, and to one fading out.
    fn set_position(&mut self, _quarters: f64, _playing: bool) {}
}

/// The processor's control period: a pending change of mode waits for its tick.
const TICK: usize = 128;

/// A change of mode in progress: the new effect's input ramps in and the outputs cross,
/// both over 20 ms (`PerformanceFXProcessor::processEffectsWithCrossfade`).
struct Fade {
    old: usize,
    input: kit::Glide,
    output: kit::Glide,
}

/// The Perform FX module (Maschine's `PerformanceFXProcessor`).
pub struct PerformFx {
    sr: f32,
    quarter: f32,
    bpm: f32,
    /// The song position (quarters) at the next sample, while the song plays.
    song: Option<f64>,
    /// The song position as last given (it stays put while the song is stopped).
    at: f64,
    params: Vec<(String, f64)>,
    /// The current effect, and one asked for (it waits for the block's start and then a
    /// tick, with no crossfade running).
    mode: usize,
    pending: Option<usize>,
    asked: Option<usize>,
    /// Samples to the next control tick.
    tick: usize,
    engaged: bool,
    pos: kit::Ramp,
    /// The strip's (position, engage) each effect last saw.
    strip: [(f64, f64); 8],
    modes: Vec<Box<dyn Mode>>,
    fade: Option<Fade>,
}

impl PerformFx {
    pub fn new(sr: f64) -> PerformFx {
        let sr = sr as f32;
        let modes: Vec<Box<dyn Mode>> = vec![
            Box::new(filter::Filter::new(sr)),
            Box::new(flanger::Flanger::new(sr)),
            Box::new(burst::BurstEcho::new(sr)),
            Box::new(reso::ResoEcho::new(sr)),
            Box::new(ring::Ring::new(sr)),
            Box::new(stutter::Stutter::new(sr)),
            Box::new(tremolo::Tremolo::new(sr)),
            Box::new(scratcher::Scratcher::new(sr)),
        ];
        let strip = deckard_model::PERFORM_FX.1;
        let at = |k: &str| strip.iter().find(|(n, _)| *n == k).map_or(0.0, |x| x.1);
        PerformFx {
            sr,
            quarter: sr / 2.0,
            bpm: 120.0,
            song: None,
            at: 0.0,
            params: Vec::new(),
            mode: 0,
            pending: None,
            asked: None,
            tick: 0,
            engaged: false,
            pos: kit::Ramp::new(0.5, 20.0, sr),
            strip: [(at("touchstrip_pos"), at("touchstrip_on_off")); 8],
            modes,
            fade: None,
        }
    }

    /// Effect `m`'s settings: all as set, but the strip as it last saw it.
    fn send(&mut self, m: usize) {
        let (pos, on) = self.strip[m];
        let mut p = self.params.clone();
        for (k, v) in p.iter_mut() {
            match k.as_str() {
                "touchstrip_pos" => *v = pos,
                "touchstrip_on_off" => *v = on,
                _ => {}
            }
        }
        self.modes[m].set(&Params(&p));
    }

    /// The strip as set now.
    fn strip_now(&self) -> (f64, f64) {
        let p = Params(&self.params);
        (f64::from(p.f("touchstrip_pos")), f64::from(p.f("touchstrip_on_off")))
    }

    /// Make `new` current, crossfading from the one that was (`switchEffectMode`).
    fn switch(&mut self, new: usize) {
        let old = self.mode;
        self.mode = new;
        self.strip[new] = self.strip_now();
        self.send(new);
        self.modes[new].activate();
        let mut fade = Fade { old, input: kit::Glide::new(self.sr), output: kit::Glide::new(self.sr) };
        fade.input.set(1.0);
        fade.output.set(1.0);
        self.fade = Some(fade);
    }
}

impl Effect for PerformFx {
    fn set(&mut self, params: &[(String, f64)]) {
        let first = self.params.is_empty();
        self.params = params.to_vec();
        let p = Params(params);
        let mode = p.i("mode").min(7);
        self.engaged = p.f("touchstrip_on_off") >= 0.5;
        self.pos.target = p.f("touchstrip_pos").clamp(0.0, 1.0);
        if first {
            // At load: every effect suspended, the project's current at once (Maschine
            // starts on Filter and crossfades to it in the first 20 ms).
            self.pos.v = self.pos.target;
            self.mode = mode;
            self.strip[mode] = self.strip_now();
            for m in 0..8 {
                self.send(m);
                if m != mode {
                    self.modes[m].deactivate();
                }
            }
            self.modes[mode].activate();
            return;
        }
        self.strip[self.mode] = self.strip_now();
        for m in 0..8 {
            self.send(m);
        }
        self.pending = (mode != self.mode).then_some(mode);
    }

    fn set_tempo(&mut self, bpm: f64) {
        self.bpm = bpm as f32;
        self.quarter = (60.0 / bpm.max(1.0)) as f32 * self.sr;
        for m in &mut self.modes {
            m.set_bpm(self.bpm);
        }
    }

    fn set_transport(&mut self, quarters: Option<f64>) {
        self.song = quarters.map(super::song_position);
        // The position the cores are told is the host's own (not rounded to ticks).
        if let Some(q) = quarters {
            self.at = q;
        }
        let playing = self.song.is_some();
        self.modes[self.mode].set_position(self.at, playing);
        if let Some(fade) = &self.fade {
            self.modes[fade.old].set_position(self.at, playing);
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        // A block's start: a change of mode asked for goes ahead if none is fading.
        if self.fade.is_none()
            && let Some(m) = self.pending.take()
        {
            self.asked = Some(m);
        }
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            if self.tick == 0 {
                self.tick = TICK;
                if self.fade.is_none()
                    && let Some(m) = self.asked.take()
                    && m != self.mode
                {
                    self.switch(m);
                }
            }
            self.tick -= 1;
            let c = Ctx { sr: self.sr, pos: self.pos.next(), engaged: self.engaged, quarter: self.quarter, song: self.song };
            if let Some(q) = &mut self.song {
                *q += 1.0 / f64::from(self.quarter);
            }
            let (x, y) = (*a, *b);
            let Some(fade) = &mut self.fade else {
                (*a, *b) = self.modes[self.mode].frame(x, y, &c);
                continue;
            };
            let gain = fade.input.tick();
            let (nl, nr) = self.modes[self.mode].frame(gain * x, gain * y, &c);
            let (pl, pr) = self.modes[fade.old].frame(x, y, &c);
            let w = fade.output.tick();
            *a = (1.0 - w).mul_add(pl, w * nl);
            *b = (1.0 - w).mul_add(pr, w * nr);
            if fade.input.cur >= 0.99 && fade.input.target >= 0.99 && fade.output.cur >= 0.99 && fade.output.target >= 0.99 {
                let old = fade.old;
                self.fade = None;
                self.modes[old].deactivate();
            }
        }
    }
}

#[cfg(test)]
mod tests;
