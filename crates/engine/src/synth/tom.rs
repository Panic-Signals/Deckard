//! Maschine's Tom drum synth (docs/synth-tom.md): `DATA::Tom` →
//! `RT::DrumSynthWrapper<DSP::TomProcessor>`. The processor holds four engines and plays
//! the one the Engine parameter selects: Tronic (`DustyTomCore`), Fractal
//! (`TomFBMatrixCore`), Floor (`FloorTomCore`) and High (`HighTomCore`). Each engine is
//! one monophonic voice; a new note restarts it.
//!
//! The wrapper runs the engine in chunks of at most 16 frames and calls its control-rate
//! update every 128 frames (§2.2). A 128-frame block at whose end the engine reports no
//! output is silenced (§2.3). The engines are ported operation by operation from the arm64 code
//! (`f32`, `f64` where Maschine uses doubles, `mul_add` where it uses fused
//! multiply-adds), so the output matches Maschine bit for bit (§6).

mod dsp;
mod floor;
mod fractal;
mod high;
mod modal;
mod resample;
mod samples;
mod tronic;

use super::{Synth, param};

/// Frames between control-rate updates (`DrumSynthWrapper`, +0x86480).
const CONTROL: usize = 128;
/// The wrapper's largest audio chunk (+0x88cb8).
const CHUNK: usize = 16;

/// The shared parameters by processor index 0..=6 (`TomProcessor::setParameter`), in
/// the order `Tom::connectParameters` applies them: mxprj field and default.
const SHARED: [(&str, f64); 7] =
    [("engine", 0.0), ("tune", 0.0), ("decay", 0.5), ("bend", 0.5), ("impact", 0.8), ("tension", 1.0), ("velocity", 1.0)];

/// The engine parameters (`setCoreParameter(core, index)`), in connection order: mxprj
/// field, default, core (engine), index.
const CORE: [(&str, f64, usize, u32); 16] = [
    ("interval", 0.5, 0, 2),
    ("fm_freq", 0.5, 0, 5),
    ("fm_amount", 0.1, 0, 6),
    ("fm_decay", 0.35, 0, 7),
    ("keytrack_mode", 0.0, 1, 2),
    ("mix", 0.05, 1, 5),
    ("glide", 0.1, 1, 0),
    ("transpose", 0.5, 1, 7),
    ("freq_a", 1.0, 1, 8),
    ("freq_b", 0.75, 1, 9),
    ("fm", 0.5, 1, 10),
    ("am", 0.5, 1, 11),
    ("color", 0.5, 1, 12),
    ("flex", 0.3, 2, 0),
    ("skin", 0.0, 2, 6),
    ("air", 0.5, 2, 5),
];

/// One of the processor's engines. The methods mirror `TomProcessor`'s per-engine code.
trait Engine: Send {
    /// `onSetSampleRate(sr, 128)`.
    fn set_sample_rate(&mut self, sr: f32);
    /// `onReset()`.
    fn reset(&mut self);
    /// `TomProcessor::setParameter(idx, v)`, idx 1..=6, while this engine plays.
    /// `shared` holds the processor's stored values (+0x80), `v` among them.
    fn set_shared(&mut self, idx: u32, v: f32, shared: &[f32; 7]);
    /// `updateGlobalCoreParameters()`: the stored shared values, when this engine takes
    /// over at a note.
    fn update_global(&mut self, shared: &[f32; 7]);
    /// `setCoreParameter(core, idx, v)` for this engine.
    fn set_core(&mut self, idx: u32, v: f32);
    /// `TomProcessor::setNoteOn(vel, note)` for this engine (`vel` = MIDI velocity · 1/127).
    fn note_on(&mut self, vel: f32, note: u32, shared: &[f32; 7]);
    /// `setNoteOff(note)`.
    fn note_off(&mut self, note: u32);
    /// `onChoke()`: start the engine's quick fade.
    fn choke(&mut self);
    /// `processControlRate()`.
    fn control_rate(&mut self);
    /// `processAudio` for `l.len()` (≤ 16) frames. `l` and `r` arrive zeroed; an idle
    /// engine leaves them so.
    fn process(&mut self, l: &mut [f32], r: &mut [f32]);
    /// The wrapper's silence test after a chunk (`DrumSynthWrapper::processAudio`).
    fn silent(&self) -> bool;
}

/// Maschine's Tom: `TomProcessor` and the `DrumSynthWrapper` around it.
pub struct Tom {
    /// The stored shared parameters (+0x80), indexed as `SHARED`.
    shared: [f32; 7],
    /// The playing engine (+0x9c) and the selected one (+0xa0); a note switches.
    current: usize,
    selected: usize,
    engines: [Box<dyn Engine>; 4],
    /// The last value `set` passed on for every field (`SHARED`, then `CORE`).
    stored: [f32; 23],
    /// The host rate, applied after the first `set` (then 0).
    pending_sr: f32,
    /// Frames to the next control-rate update and to the end of the current chunk.
    control: usize,
    chunk: usize,
    /// A note started at this frame already: Maschine takes one note-on per frame.
    noted: bool,
}

impl Tom {
    pub fn new(sr: f64) -> Tom {
        let mut engines: [Box<dyn Engine>; 4] = [
            Box::new(tronic::Tronic::new()),
            Box::new(fractal::Fractal::new()),
            Box::new(floor::Floor::new()),
            Box::new(high::High::new()),
        ];
        // `DSPCoreWrapper()`: the processor is made at 44.1 kHz; the host rate follows
        // once the parameters are in.
        for e in engines.iter_mut() {
            e.set_sample_rate(44100.0);
        }
        Tom {
            shared: [0.0; 7],
            current: 0,
            selected: 0,
            engines,
            stored: [f32::NAN; 23],
            pending_sr: sr as f32,
            control: 0,
            chunk: 0,
            noted: false,
        }
    }

    /// `TomProcessor::setParameter(idx, v)`.
    fn set_parameter(&mut self, idx: usize, v: f32) {
        self.shared[idx] = v;
        if idx == 0 {
            // `setParamMode`: the new engine plays from the next note; the current one
            // fades out if it is playing.
            let m = v as i32;
            if (0..4).contains(&m) {
                self.selected = m as usize;
            }
            if self.current as i32 != m {
                self.engines[self.current].choke();
            }
        } else {
            let shared = self.shared;
            self.engines[self.current].set_shared(idx as u32, v, &shared);
        }
    }
}

impl Synth for Tom {
    fn set(&mut self, params: &[(String, f64)]) {
        for (i, &(field, def)) in SHARED.iter().enumerate() {
            let v = param(params, field, def) as f32;
            if v.to_bits() != self.stored[i].to_bits() {
                self.stored[i] = v;
                self.set_parameter(i, v);
            }
        }
        for (i, &(field, def, core, idx)) in CORE.iter().enumerate() {
            let v = param(params, field, def) as f32;
            if v.to_bits() != self.stored[7 + i].to_bits() {
                self.stored[7 + i] = v;
                self.engines[core].set_core(idx, v);
            }
        }
        if self.pending_sr > 0.0 {
            // `onSamplerateChanged`, then `resume()`.
            let sr = self.pending_sr;
            self.pending_sr = 0.0;
            for e in self.engines.iter_mut() {
                e.set_sample_rate(sr);
            }
            for e in self.engines.iter_mut() {
                e.reset();
            }
        }
    }

    fn note_on(&mut self, note: i64, velocity: i64) {
        if velocity <= 0 {
            self.note_off(note);
            return;
        }
        if self.noted {
            return;
        }
        self.noted = true;
        let vel = velocity.min(127) as f32 * 0.007874016;
        // `setNoteOn`: the selected engine takes over, with the stored shared values.
        self.current = self.selected;
        let shared = self.shared;
        let e = &mut self.engines[self.current];
        e.update_global(&shared);
        e.note_on(vel, (note & 0x7f) as u32, &shared);
    }

    fn note_off(&mut self, note: i64) {
        self.engines[self.current].note_off((note & 0x7f) as u32);
    }

    // `control`: Maschine's sound consumes host CCs before the module; All Sound Off
    // (CC 120), which the wrapper would turn into a choke, does not reach it (§2.2).

    fn stop(&mut self) {
        self.engines[self.current].choke();
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        l.fill(0.0);
        r.fill(0.0);
        self.noted = false;
        let n = l.len().min(r.len());
        let mut pos = 0;
        // Where the current 128-frame block began in this call (0 if earlier).
        let mut block = 0;
        while pos < n {
            if self.control == 0 {
                self.engines[self.current].control_rate();
                self.control = CONTROL;
                block = pos;
            }
            if self.chunk == 0 {
                self.chunk = CHUNK;
            }
            let k = (n - pos).min(self.control).min(self.chunk);
            let e = &mut self.engines[self.current];
            e.process(&mut l[pos..pos + k], &mut r[pos..pos + k]);
            self.control -= k;
            self.chunk -= k;
            pos += k;
            // §2.3: Maschine drops a whole block the engine ends silent.
            if (self.control == 0 || pos == n) && e.silent() {
                l[block..pos].fill(0.0);
                r[block..pos].fill(0.0);
            }
        }
    }

    fn voices(&self) -> usize {
        usize::from(!self.engines[self.current].silent())
    }
}

/// What a new Tom holds: Maschine's defaults (stored values, by field).
pub fn defaults() -> Vec<(&'static str, f64)> {
    vec![
        ("engine", 0.0),
        ("tune", 0.0),
        ("decay", 0.5),
        ("bend", 0.5),
        ("impact", 0.8),
        ("velocity", 1.0),
        ("interval", 0.5),
        ("fm_freq", 0.5),
        ("fm_amount", 0.1),
        ("fm_decay", 0.35),
        ("keytrack_mode", 0.0),
        ("mix", 0.05),
        ("glide", 0.1),
        ("transpose", 0.5),
        ("freq_a", 1.0),
        ("freq_b", 0.75),
        ("fm", 0.5),
        ("am", 0.5),
        ("color", 0.5),
        ("tension", 1.0),
        ("flex", 0.3),
        ("air", 0.5),
        ("skin", 0.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(engine: f64) -> Vec<(String, f64)> {
        vec![("engine".into(), engine)]
    }

    /// Renders `n` frames in 512-frame calls; returns the left channel (Floor and High
    /// are stereo, the others write the same signal to both channels).
    fn render(t: &mut Tom, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        let (mut l, mut r) = (vec![0f32; 512], vec![0f32; 512]);
        while out.len() < n {
            let k = (n - out.len()).min(512);
            t.process(&mut l[..k], &mut r[..k]);
            // The right channel must be as clean as the left one the tests inspect.
            assert!(r[..k].iter().all(|x| x.is_finite() && (*x == 0.0 || x.abs() >= f32::MIN_POSITIVE)));
            out.extend_from_slice(&l[..k]);
        }
        out
    }

    fn tom(engine: f64) -> Tom {
        let mut t = Tom::new(48000.0);
        t.set(&params(engine));
        t
    }

    #[test]
    fn every_engine_sounds_and_ends_in_silence() {
        // Tronic and Fractal (Floor and High aren't ported yet).
        for e in 0..2 {
            let mut t = tom(e as f64);
            assert_eq!(t.voices(), 0);
            t.note_on(60, 100);
            let a = render(&mut t, 4800);
            let peak = a.iter().fold(0f32, |m, x| m.max(x.abs()));
            assert!(peak > 1e-3 && peak < 4.0, "engine {e}: peak {peak}");
            assert!(a.iter().all(|x| x.is_finite()), "engine {e}: NaN");
            assert!(t.voices() > 0, "engine {e}");
            let _ = render(&mut t, 48000 * 6);
            assert_eq!(t.voices(), 0, "engine {e}: still sounding");
            assert!(render(&mut t, 4800).iter().all(|&x| x == 0.0), "engine {e}: not silent");
        }
    }

    #[test]
    fn set_mid_note_keeps_the_voice() {
        // Tronic and Fractal (Floor and High aren't ported yet).
        for e in 0..2 {
            let mut t = tom(e as f64);
            t.note_on(48, 127);
            let _ = render(&mut t, 2000);
            let mut p = params(e as f64);
            p.push(("decay".into(), 0.9));
            p.push(("tune".into(), 0.7));
            t.set(&p);
            let b = render(&mut t, 64);
            assert!(t.voices() > 0, "engine {e}");
            assert!(b.iter().any(|&x| x != 0.0), "engine {e}: went quiet");
        }
    }

    #[test]
    fn choke_fades_out() {
        let mut t = tom(0.0);
        t.note_on(60, 100);
        let _ = render(&mut t, 1000);
        t.stop();
        let _ = render(&mut t, 4800);
        assert_eq!(t.voices(), 0);
    }

    #[test]
    fn engine_change_applies_at_the_next_note() {
        let mut t = tom(0.0);
        t.set(&params(3.0));
        assert_eq!(t.current, 0);
        t.note_on(60, 100);
        assert_eq!(t.current, 3);
    }

    #[test]
    fn no_denormals_in_long_tails() {
        let mut t = tom(0.0);
        let mut p = params(0.0);
        p.push(("decay".into(), 1.0));
        t.set(&p);
        t.note_on(36, 127);
        for x in render(&mut t, 48000 * 4) {
            assert!(x == 0.0 || x.abs() > f32::MIN_POSITIVE, "denormal {x:e}");
        }
    }
}

#[cfg(test)]
mod oracle {
    use super::*;

    /// Null-test aid (needs the noise recordings through `TOM_RES_NOISES`/`TOM_RES_PINK`
    /// for Floor and High): renders like `synth-null`. `TOM_ORACLE` =
    /// "params.json|out.f32|sr|seconds|T:N:V:D,…"; params.json is the flat map
    /// `maschine-ref list` shows; events land at the start of their 1024-frame block.
    #[test]
    #[ignore]
    fn render() {
        let Ok(spec) = std::env::var("TOM_ORACLE") else { return };
        let f: Vec<&str> = spec.split('|').collect();
        let json = std::fs::read_to_string(f[0]).unwrap();
        let params: Vec<(String, f64)> = json
            .trim()
            .trim_matches(|c| c == '{' || c == '}')
            .split(',')
            .map(|kv| {
                let (k, v) = kv.split_once(':').unwrap();
                (k.trim().trim_matches('"').to_string(), v.trim().parse().unwrap())
            })
            .collect();
        let sr: f64 = f[2].parse().unwrap();
        let n = (f[3].parse::<f64>().unwrap() * sr) as usize;
        let mut ev: Vec<(usize, i64, i64)> = Vec::new();
        for e in f[4].split(',') {
            let p: Vec<f64> = e.split(':').map(|x| x.parse().unwrap()).collect();
            ev.push(((p[0] * sr) as usize / 1024 * 1024, p[1] as i64, p[2] as i64));
            ev.push((((p[0] + p[3]) * sr) as usize / 1024 * 1024, p[1] as i64, 0));
        }
        let mut t = Tom::new(sr);
        t.set(&params);
        let mut out = Vec::with_capacity(n * 8);
        let (mut l, mut r) = (vec![0f32; 1024], vec![0f32; 1024]);
        let start = std::time::Instant::now();
        let mut pos = 0;
        while pos < n {
            for &(at, key, vel) in &ev {
                if at == pos {
                    if vel > 0 { t.note_on(key, vel) } else { t.note_off(key) }
                }
            }
            let k = (n - pos).min(1024);
            t.process(&mut l[..k], &mut r[..k]);
            for i in 0..k {
                out.extend_from_slice(&l[i].to_le_bytes());
                out.extend_from_slice(&r[i].to_le_bytes());
            }
            pos += k;
        }
        eprintln!("rendered {n} frames in {:?}", start.elapsed());
        std::fs::write(f[1], out).unwrap();
    }
}
