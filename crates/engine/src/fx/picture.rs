//! Right-screen pictures for the built-in effects other than the EQ and the Filter,
//! which draw their own. Each picture is the knob law a person would expect to see:
//! a transfer, a row of taps, a sweep, a tail, or a cloud. It is not a second copy of
//! the audio engine. The Filter's response and the side-chain band are the real ones.

use super::param;

/// What the right screen draws for one effect.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    /// `"freq"`, `"law"`, `"time"`, or `"dots"`.
    pub kind: &'static str,
    pub points: Vec<f32>,
    pub extent: Vec<f32>,
    pub side: Vec<f32>,
    pub side_live: bool,
    pub bands: Vec<(f32, f32)>,
    pub lfo: Vec<f32>,
    /// Perform FX with Engage off: the curve is drawn quiet.
    pub dim: bool,
    /// Cabinet: the engine still passes audio straight through.
    pub preview: bool,
}

impl Shot {
    fn bare(kind: &'static str, points: Vec<f32>) -> Self {
        Self {
            kind,
            points,
            extent: Vec::new(),
            side: Vec::new(),
            side_live: false,
            bands: Vec::new(),
            lfo: Vec::new(),
            dim: false,
            preview: false,
        }
    }
}

const N: usize = 96;

pub fn shot(module: &str, params: &[(String, f64)], phase: f32, side_live: bool, page: usize) -> Option<Shot> {
    let phase = phase.rem_euclid(1.0);
    let mut shot = match module {
        "Compressor" => compressor(params),
        "Limiter" => limiter(params),
        "Gate" => gate(params),
        "Maximizer" => maximizer(params),
        "Saturator" => saturator(params),
        "MullhollandDrive" => distortion(params, page),
        "TransientDesigner" => transient(params),
        "BeatDelay" => beat_delay(params),
        "GrainDelay" => grain_delay(params, phase),
        "GrainStretch" => grain_stretch(params),
        "Resochord" => resochord(params),
        "Chorus" => chorus(params, phase),
        "Flanger" => flanger(params, phase, false),
        "Phaser" => flanger(params, phase, true),
        "FM" => fm(params, phase),
        "FreqShifter" => freq_shifter(params),
        "LoFi" => lofi(params),
        "Reverb" => reverb(params),
        "PlateReverb" => plate(params, phase, page),
        "Metaverb" => metaverb(params),
        "Iceverb" => ice(params, true),
        "Reflex" => ice(params, false),
        _ => return None,
    };
    if matches!(module, "Compressor" | "Limiter" | "Gate" | "Maximizer") {
        side(params, side_live, &mut shot);
    }
    let _ = phase;
    Some(shot)
}

pub fn moves(module: &str, params: &[(String, f64)]) -> bool {
    match module {
        "GrainDelay" => p(params, "modulation", 0.0) > 0.02,
        "Chorus" => p(params, "amount", 0.55).abs() > 0.02,
        "Flanger" => p(params, "source", 0.0).round() < 1.5 && p(params, "amount", 0.75).abs() > 0.02,
        "Phaser" => p(params, "source", 1.0).round() < 1.5 && p(params, "amount", 0.5).abs() > 0.02,
        "FM" => p(params, "amount", 0.75) > 0.02,
        "PlateReverb" => plate_mod(params) > 0.05 && plate_mode(params) != 2,
        _ => {
            let _ = params;
            false
        }
    }
}

pub fn rate_hz(module: &str, params: &[(String, f64)], tempo: f64) -> f32 {
    match module {
        "GrainDelay" => 0.15 + p(params, "modulation", 0.0).clamp(0.0, 1.0) * 3.0,
        "Chorus" => 0.0639 * 256.0f32.powf(p(params, "rate", 0.5).clamp(0.0, 1.0)),
        "Flanger" | "Phaser" => {
            let source = p(params, "source", if module == "Phaser" { 1.0 } else { 0.0 }).round();
            mod_hz(p(params, "speed", 0.5), source == 1.0, tempo)
        }
        "FM" => 0.1 * 32.0f32.powf(p(params, "rate", 0.25).clamp(0.0, 1.0)),
        "PlateReverb" => 0.15 + plate_mod(params) * 1.5,
        _ => {
            let _ = (params, tempo);
            0.0
        }
    }
}

fn p(params: &[(String, f64)], field: &str, default: f64) -> f32 {
    param(params, field, default) as f32
}

fn on(params: &[(String, f64)], field: &str, default: f64) -> bool {
    param(params, field, default) >= 0.5
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn cl(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// −60..+6 dB onto 0..1, for a transfer chart.
fn law(db: f32) -> f32 {
    cl((db + 60.0) / 66.0)
}

fn samples(n: usize) -> impl Iterator<Item = (usize, f32)> {
    let n = n.max(2);
    (0..n).map(move |i| (i, i as f32 / (n - 1) as f32))
}

fn sine(n: usize) -> Vec<f32> {
    samples(n).map(|(_, t)| 0.5 + 0.5 * (t * std::f32::consts::TAU).cos()).collect()
}

/// Attack, a held open part, then release. Each is 0..1 of the window.
fn envelope(attack: f32, hold: f32, release: f32, n: usize) -> Vec<f32> {
    let rise = 0.04 + 0.5 * attack.clamp(0.0, 1.0);
    let open = 0.06 + 0.45 * hold.clamp(0.0, 1.0);
    let fall = 0.06 + 0.55 * release.clamp(0.0, 1.0);
    let sum = rise + open + fall;
    let (rise, open, fall) = (rise / sum, open / sum, fall / sum);
    samples(n)
        .map(|(_, t)| {
            if t < rise {
                t / rise
            } else if t < rise + open {
                1.0
            } else {
                cl(1.0 - (t - rise - open) / fall)
            }
        })
        .collect()
}

fn hash(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B1);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    (x & 0xFFFF) as f32 / 65535.0
}

fn freq_x(hz: f32) -> f32 {
    let hz = hz.clamp(20.0, 20_000.0);
    cl((hz.ln() - 20f32.ln()) / (20_000f32.ln() - 20f32.ln()))
}

fn side(params: &[(String, f64)], live: bool, shot: &mut Shot) {
    shot.side = super::sidechain::response(params, N);
    shot.side_live = live;
}

/// The picture for `module`, or `None` for the EQ, the Filter, and anything else.
/// `phase` is the modulator's position, 0..1, advanced by [`rate_hz`].
fn mod_hz(speed: f32, synced: bool, tempo: f64) -> f32 {
    const RATES: [(f32, f32); 11] = [
        (16.0, 1.0),
        (8.0, 1.0),
        (4.0, 1.0),
        (2.0, 1.0),
        (1.0, 1.0),
        (1.0, 2.0),
        (3.0, 8.0),
        (1.0, 4.0),
        (3.0, 16.0),
        (1.0, 8.0),
        (1.0, 16.0),
    ];
    if synced {
        let i = (10.0 * speed.clamp(0.0, 1.0)).round() as usize;
        let (n, d) = RATES[i.min(10)];
        let bars = n / d;
        let period = bars * 4.0 * 60.0 / tempo.max(1.0) as f32;
        return 1.0 / period.max(0.05);
    }
    0.05 * 160.0f32.powf(speed.clamp(0.0, 1.0))
}

fn compressor(params: &[(String, f64)]) -> Shot {
    let threshold = p(params, "threshold", 1.0 / 6.0).clamp(0.0, 1.0);
    let knee = p(params, "knee", 0.5).clamp(0.0, 1.0);
    let amount = p(params, "amount", 0.75).clamp(0.0, 1.0);
    let gain = p(params, "gain", 0.916_666_7).clamp(0.0, 1.0);
    let feedback = on(params, "mode", 0.0);
    let makeup = 36.0 * gain - 36.0;
    let curve = |extra: f32| {
        samples(N)
            .map(|(_, x)| {
                let input = -60.0 + x * 66.0;
                let gr = compress(input, -60.0 * threshold, knee, amount) * extra;
                law(input - gr + makeup)
            })
            .collect::<Vec<_>>()
    };
    let mut shot = Shot::bare("law", curve(if feedback { 1.25 } else { 1.0 }));
    if feedback {
        shot.extent = curve(1.0);
    }
    let thr = law(-60.0 * threshold);
    shot.bands = vec![(thr, law(-60.0 * threshold + makeup))];
    shot.lfo = envelope(p(params, "attack", 0.315), 0.15, p(params, "release", 0.5), 48);
    shot
}

fn compress(input: f32, thr: f32, knee: f32, amount: f32) -> f32 {
    let ratio = 1.0 + amount * 19.0;
    let slope = 1.0 - 1.0 / ratio;
    let width = (knee * 18.0).max(0.0);
    let over = input - thr;
    if width < 0.4 {
        return over.max(0.0) * slope;
    }
    if over <= -width * 0.5 {
        0.0
    } else if over >= width * 0.5 {
        over * slope
    } else {
        let u = over + width * 0.5;
        u * u / (2.0 * width) * slope
    }
}

fn limiter(params: &[(String, f64)]) -> Shot {
    let thr = p(params, "threshold", 0.0).clamp(-40.0, 0.0);
    let transparent = on(params, "mode", 1.0);
    let ceiling = -40.0 + 39.7 * p(params, "ceiling", 1.0).clamp(0.0, 1.0);
    let cap = if transparent { thr.min(ceiling) } else { thr };
    let points = samples(N)
        .map(|(_, x)| {
            let input = -60.0 + x * 66.0;
            law(input.min(cap))
        })
        .collect();
    let mut shot = Shot::bare("law", points);
    shot.bands = vec![(law(thr), law(thr))];
    if transparent {
        shot.extent = samples(N).map(|(_, _)| law(ceiling)).collect();
        shot.lfo = envelope(0.04, 0.2, p(params, "release", 0.0), 48);
    }
    shot
}

fn gate(params: &[(String, f64)]) -> Shot {
    let thr_lin = p(params, "threshold", 0.562_341_3).clamp(0.1, 1.0);
    let thr = 40.0 * thr_lin.log10();
    let mix = p(params, "mix", 1.0).clamp(0.0, 1.0);
    let points = samples(N)
        .map(|(_, x)| {
            let input = -60.0 + x * 66.0;
            let closed = if input >= thr { input } else { -60.0 };
            law(lerp(input, closed, mix))
        })
        .collect();
    let mut shot = Shot::bare("law", points);
    shot.bands = vec![(law(thr), law(thr))];
    shot.lfo = envelope(p(params, "attack", 0.0), p(params, "hold", 0.5), p(params, "release", 0.25), 48);
    shot
}

fn maximizer(params: &[(String, f64)]) -> Shot {
    let amount = p(params, "amount", 0.5).clamp(0.0, 1.0);
    let curve = p(params, "curve", 0.5).clamp(0.0, 1.0);
    let turbo = on(params, "turbo", 0.0);
    let bow = 0.45 + curve * 1.6;
    let points = samples(N)
        .map(|(_, x)| {
            let lifted = x + amount * (1.0 - x).powf(bow);
            let y = if turbo && lifted > 0.72 { 0.72 + (lifted - 0.72) * 0.35 } else { lifted };
            cl(y)
        })
        .collect();
    Shot::bare("law", points)
}

fn saturator(params: &[(String, f64)]) -> Shot {
    let mode = p(params, "mode", 0.0).round().clamp(0.0, 2.0) as i32;
    let points = match mode {
        1 => waveshape(
            p(params, "tape_drive", 0.34),
            0.5 + p(params, "tape_input", 0.2) * 4.0,
            0.6 + p(params, "tape_contour", 1.0) * 0.8,
            1.0,
        ),
        2 => {
            let drive = 0.4 + p(params, "tube_drive", 0.25).clamp(0.0, 1.0) * 5.0 + p(params, "tube_charge", 2.0).clamp(0.0, 4.0) * 0.35;
            let drive = if on(params, "tube_overload", 0.0) { drive * 1.7 } else { drive };
            let mut pts = waveshape(0.0, drive, 1.0, p(params, "tube_gain", 0.0));
            if !on(params, "tube_eq_bypass", 0.0) {
                let bass = p(params, "tube_bass", 0.8) - 0.8;
                let treble = p(params, "tube_treble", 0.8) - 0.8;
                for (i, y) in pts.iter_mut().enumerate() {
                    let x = i as f32 / (N - 1) as f32;
                    *y = cl(*y + bass * 0.18 * (1.0 - x) + treble * 0.18 * x);
                }
            }
            pts
        }
        _ => waveshape(
            0.0,
            0.5 + p(params, "classic_drive", 0.5).clamp(0.0, 1.0) * 6.0 * (0.4 + p(params, "classic_input", 0.25)),
            0.4 + p(params, "classic_contour", 0.75).clamp(0.0, 1.0) * 1.6,
            0.0,
        ),
    };
    Shot::bare("law", points)
}

fn waveshape(bias: f32, drive: f32, contour: f32, gain_db: f32) -> Vec<f32> {
    let gain = 10f32.powf(gain_db / 20.0);
    samples(N)
        .map(|(_, x)| {
            let s = (x * 2.0 - 1.0) + bias;
            let y = (s * drive).tanh();
            let shaped = y.abs().powf(contour.max(0.2)) * y.signum();
            cl(0.5 + 0.5 * shaped * gain)
        })
        .collect()
}

fn distortion(params: &[(String, f64)], page: usize) -> Shot {
    let analog = on(params, "mode", 0.0);
    let mut shot = if analog {
        let drive = 0.5 + p(params, "analog_saturation", 0.25).clamp(0.0, 1.0) * 7.0;
        let mut pts = waveshape(0.0, drive, 0.7 + p(params, "analog_definition", 0.5).clamp(0.0, 1.0), 0.0);
        let bass = p(params, "analog_bass", 1.0) - 1.0;
        let tone = p(params, "analog_tone", 1.0) - 1.0;
        let focus = if on(params, "analog_focus", 0.0) { 1.4 } else { 1.0 };
        for (i, y) in pts.iter_mut().enumerate() {
            let x = i as f32 / (N - 1) as f32;
            let d = (*y - 0.5) * focus;
            *y = cl(0.5 + d + bass * 0.15 * (1.0 - x) + tone * 0.15 * x);
        }
        let mix = p(params, "analog_mix", 1.0).clamp(0.0, 1.0);
        for (i, y) in pts.iter_mut().enumerate() {
            let x = i as f32 / (N - 1) as f32;
            *y = lerp(x, *y, mix);
        }
        Shot::bare("law", pts)
    } else {
        let drive = 0.4 + p(params, "drive", 0.25).clamp(0.0, 1.0) * 8.0;
        let color = p(params, "color", 0.25).clamp(0.0, 1.0);
        let tone = p(params, "tone", 0.75).clamp(0.0, 1.0);
        let tone_mod = p(params, "tone_mod", 0.0).clamp(-1.0, 1.0);
        let mut pts = waveshape(0.0, drive, 0.5 + color, 0.0);
        for (i, y) in pts.iter_mut().enumerate() {
            let x = i as f32 / (N - 1) as f32;
            *y = cl(*y + (tone - 0.5) * 0.25 * (x - 0.5) + tone_mod * 0.08 * (0.5 - x));
        }
        let mix = p(params, "mix", 0.5).clamp(0.0, 1.0);
        for (i, y) in pts.iter_mut().enumerate() {
            let x = i as f32 / (N - 1) as f32;
            *y = lerp(x, *y, mix);
        }
        let mut shot = Shot::bare("law", pts);
        let fb = p(params, "feedback", 0.25).clamp(-1.0, 1.0);
        shot.lfo = (0..48)
            .map(|i| {
                let t = i as f32 / 47.0;
                cl(0.85 * (1.0 - t).powf(0.4 + (1.0 - fb.abs()) * 2.0) * fb.signum().max(0.0) + (1.0 - fb.abs()) * 0.15)
            })
            .collect();
        if fb < 0.0 {
            for y in &mut shot.lfo {
                *y = 1.0 - *y;
            }
        }
        shot
    };
    if page == 1 && !analog && on(params, "gate", 1.0) {
        shot.lfo = envelope(0.02, 0.25, p(params, "release", 0.5), 48);
    }
    shot
}

fn transient(params: &[(String, f64)]) -> Shot {
    let gain = p(params, "input_gain", 0.0).clamp(-1.0, 1.0);
    let scale = if gain < 0.0 { 1.0 + gain * 0.7 } else { 1.0 + gain * 0.35 };
    let attack = p(params, "attack", 0.0).clamp(-1.0, 1.0);
    let sustain = p(params, "sustain", 0.0).clamp(-1.0, 1.0);
    let limit = on(params, "limit", 1.0);
    let points = samples(64)
        .map(|(_, t)| {
            let front =
                if t < 0.18 { (t / 0.18) * (0.55 + attack * 0.4) } else { (0.55 + attack * 0.4) * (1.0 - (t - 0.18) / 0.12).max(0.0) };
            let body = if t > 0.22 { (0.28 + sustain * 0.22) * (1.0 - (t - 0.22) / 0.78).max(0.0) } else { 0.0 };
            let mut y = (front + body) * scale;
            if limit {
                y = y.min(0.72);
            }
            cl(y)
        })
        .collect();
    Shot::bare("time", points)
}

fn beat_delay(params: &[(String, f64)]) -> Shot {
    let time = p(params, "time", 3.0).round().clamp(0.0, 16.0);
    let offset = p(params, "offset", 0.0).round().clamp(0.0, 8.0);
    let feedback = p(params, "feedback", 0.5).clamp(0.0, 0.95);
    let color = p(params, "color", 0.5).clamp(0.0, 1.0);
    let crossover = p(params, "crossover", 0.25).clamp(0.0, 1.0);
    let split = p(params, "split", 0.25).clamp(0.0, 1.0);
    let stereo = p(params, "stereo", 0.0).clamp(-1.0, 1.0);
    let mix = p(params, "mix", 0.5).clamp(0.0, 1.0);
    let unit = p(params, "unit", 0.0).round() as i32;
    let unit_scale = match unit {
        1 => 2.0 / 3.0,
        2 => 0.5,
        _ => 1.0,
    };
    let step = ((time + 1.0) / 18.0 * unit_scale).clamp(0.04, 0.85);
    let mut bands = vec![(0.03, cl(0.2 + 0.45 * (1.0 - mix)))];
    let mut level = mix;
    let mut x = step;
    let mut k = 0.0;
    while level > 0.06 && x < 0.97 && bands.len() < 14 {
        let fade = (1.0 - color * 0.55).powf(k);
        bands.push((cl(x), cl(0.12 + 0.8 * level * fade)));
        let echo_x = cl(x + offset / 8.0 * step + split * 0.06 + stereo * 0.05);
        if (echo_x - x).abs() > 0.015 {
            bands.push((echo_x, cl(0.1 + 0.55 * level * fade * (1.0 - crossover * 0.65))));
        }
        level *= feedback;
        x += step;
        k += 1.0;
    }
    let points = samples(N)
        .map(|(_, t)| {
            if t < 0.03 {
                return 0.15;
            }
            let n = ((t - 0.02) / step).floor().max(0.0);
            cl(mix * feedback.powf(n) * (1.0 - color * 0.4).powf(n))
        })
        .collect();
    let mut shot = Shot::bare("time", points);
    shot.bands = bands;
    shot
}

fn grain_delay(params: &[(String, f64)], phase: f32) -> Shot {
    let pitch = p(params, "pitch", 0.0).clamp(-1.0, 1.0);
    let size = p(params, "size", 0.25).clamp(0.0, 1.0);
    let jitter = p(params, "jitter", 0.0).clamp(0.0, 1.0);
    let reverse = on(params, "reverse", 0.0);
    let space = p(params, "space", 0.0).clamp(0.0, 1.0);
    let density = p(params, "density", 0.25).clamp(0.0, 1.0);
    let modulation = p(params, "modulation", 0.0).clamp(0.0, 1.0);
    let mix = p(params, "mix", 0.75).clamp(0.0, 1.0);
    let stereo = p(params, "stereo", 0.75).clamp(0.0, 1.0);
    let count = (5.0 + density * 18.0) as u32;
    let wander = (phase * std::f32::consts::TAU).sin() * modulation * 0.16;
    let mut bands = Vec::with_capacity(count as usize);
    for i in 0..count {
        let mut x = (i as f32 + 0.5) / count as f32;
        x += (hash(i) - 0.5) * jitter * 0.45 + (hash(i + 9) - 0.5) * space * 0.55 + wander;
        if reverse {
            x = 1.0 - x;
        }
        x = 0.5 + (x.clamp(0.02, 0.98) - 0.5) * (0.35 + 0.65 * stereo);
        let y = 0.5 + pitch * 0.38 + (hash(i + 3) - 0.5) * size * 0.4;
        let y = lerp(0.5, cl(y), mix);
        bands.push((cl(x), y));
    }
    let mut shot = Shot::bare("dots", Vec::new());
    shot.bands = bands;
    shot
}

fn grain_stretch(params: &[(String, f64)]) -> Shot {
    if !on(params, "on", 0.0) {
        return Shot::bare("dots", vec![0.18; 8]);
    }
    let stretch = p(params, "stretch", 0.0).clamp(0.0, 1.0);
    let loops = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 32.0];
    let loop_i = p(params, "loop_length", 8.0).round().clamp(0.0, 8.0) as usize;
    let width = (loops[loop_i] / 32.0 * (0.35 + 0.9 * stretch)).clamp(0.08, 0.96);
    let mut pitch = p(params, "pitch", 0.5).clamp(0.0, 1.0);
    if on(params, "link", 0.0) {
        pitch = cl(pitch + (stretch - 0.15) * 0.35);
    }
    let size = p(params, "size", 0.25).clamp(0.0, 1.0);
    let mix = p(params, "mix", 1.0).clamp(0.0, 1.0);
    let count = (4.0 + (1.0 - size) * 14.0) as u32;
    let mut bands = Vec::new();
    for i in 0..count {
        let x = 0.04 + width * (i as f32 + 0.5) / count as f32;
        let y = lerp(0.5, cl(0.18 + pitch * 0.7 + (hash(i + 4) - 0.5) * size * 0.25), mix);
        bands.push((cl(x), y));
    }
    let mut shot = Shot::bare("dots", Vec::new());
    shot.bands = bands;
    shot
}

fn resochord(params: &[(String, f64)]) -> Shot {
    let string_mode = !on(params, "mode", 1.0);
    let notes = if string_mode {
        vec![0.0, p(params, "spread", 0.5).clamp(0.0, 1.0) * 12.0]
    } else {
        chord_notes(p(params, "style", 0.0).round() as i32, p(params, "chord", 1.0).round() as i32)
    };
    let tune = p(params, "tune", 0.0).clamp(-24.0, 24.0);
    let bright = p(params, "brightness", 0.75).clamp(0.0, 1.0);
    let feedback = p(params, "feedback", 0.75).clamp(0.0, 1.0);
    let decay = p(params, "decay", 0.5).clamp(0.0, 1.0);
    let mix = p(params, "mix", 0.75).clamp(0.0, 1.0);
    let repeats = (2.0 + feedback * 3.0 + decay * 4.0) as usize;
    let step = 0.9 / repeats.max(1) as f32;
    let mut bands = Vec::new();
    for r in 0..repeats {
        let fade = (1.0 - r as f32 / repeats as f32).powf(0.6 + (1.0 - decay));
        if fade < 0.18 {
            continue;
        }
        for semi in &notes {
            let y = 0.22 + bright * 0.35 + (semi + tune) / 52.0;
            bands.push((cl(0.05 + r as f32 * step), lerp(0.12, cl(y * fade + 0.08), mix)));
        }
    }
    let mut shot = Shot::bare("dots", Vec::new());
    shot.bands = bands;
    shot
}

fn chord_notes(style: i32, chord: i32) -> Vec<f32> {
    let third = match style {
        1 | 2 => 3.0,
        3 => 2.0,
        4 => 1.0,
        _ => 4.0,
    };
    let seventh = if matches!(style, 1 | 2) { 10.0 } else { 11.0 };
    match chord.clamp(0, 4) {
        0 => vec![0.0],
        1 => vec![0.0, 7.0],
        2 => vec![0.0, third, 7.0, seventh],
        3 => vec![0.0, third, 7.0, seventh, 14.0],
        _ => vec![0.0, third, 7.0, seventh, 14.0, 17.0],
    }
}

fn chorus(params: &[(String, f64)], phase: f32) -> Shot {
    let amount = p(params, "amount", 0.55).clamp(0.0, 1.0);
    let mix = p(params, "mix", 0.5).clamp(0.0, 1.0);
    let wobble = (phase * std::f32::consts::TAU).sin() * amount * 0.12;
    let lag = 0.06 + amount * 0.28 + wobble;
    let wave = |shift: f32, t: f32| 0.5 + 0.32 * ((t * 2.0 + shift) * std::f32::consts::TAU).sin();
    let points = samples(N).map(|(_, t)| cl(wave(0.0, t))).collect();
    let extent = samples(N).map(|(_, t)| cl(lerp(wave(0.0, t), wave(-lag, t), mix))).collect();
    let mut shot = Shot::bare("time", points);
    shot.extent = extent;
    shot.lfo = sine(48);
    shot
}

fn flanger(params: &[(String, f64)], phase: f32, phaser: bool) -> Shot {
    let frequency = p(params, "frequency", 0.5).clamp(0.0, 1.0);
    let feedback = p(params, "feedback", 0.5).clamp(0.0, 1.0);
    let amount = p(params, "amount", if phaser { 0.5 } else { 0.75 }).clamp(-1.0, 1.0);
    let source = p(params, "source", if phaser { 1.0 } else { 0.0 }).round() as i32;
    let stereo = p(params, "stereo", 0.5).clamp(0.0, 1.0);
    let mix = p(params, "mix", 0.5).clamp(0.0, 1.0);
    let poles = if phaser && on(params, "eight_pole", 0.0) {
        8
    } else if phaser {
        2
    } else {
        1
    };
    let invert = if phaser { false } else { on(params, "invert", 0.0) };
    let wobble = if source == 2 { 0.0 } else { (phase * std::f32::consts::TAU).sin() };
    let live = cl(frequency + amount * wobble * 0.35);
    let far = cl(frequency + amount * 0.35 + stereo * 0.08);
    let comb = |center: f32| {
        samples(N)
            .map(|(_, x)| {
                let mut acc = 1.0;
                for k in 0..poles {
                    let c = (center + k as f32 * 0.055).clamp(0.02, 0.98);
                    let d = (x - c).abs().min((x - c).abs() - 0.0);
                    let dip = (1.0 - (d * (if poles > 2 { 22.0 } else { 14.0 })).min(1.0)) * (0.25 + feedback * 0.7);
                    let v = if invert { 0.35 + dip } else { 0.85 - dip };
                    acc *= v.clamp(0.15, 1.0);
                }
                cl(lerp(0.5, acc, mix))
            })
            .collect()
    };
    let mut shot = Shot::bare("freq", comb(live));
    shot.extent = comb(far);
    shot.lfo = if source == 2 { envelope(0.05, 0.2 + p(params, "shape", 0.5).clamp(0.0, 1.0) * 0.5, 0.35, 48) } else { sine(48) };
    shot
}

fn fm(params: &[(String, f64)], phase: f32) -> Shot {
    let amount = p(params, "amount", 0.75).clamp(0.0, 1.0);
    let split = p(params, "split", 0.75).clamp(0.0, 1.0);
    let contour = p(params, "contour", 0.75).clamp(0.0, 1.0);
    let wobble = 1.0 + (phase * std::f32::consts::TAU).sin() * 0.12 * (0.2 + contour);
    let spread = (0.04 + amount * 0.32) * wobble;
    let right = spread * (0.45 + split);
    let left = spread * (1.2 - split * 0.4);
    let amp = 0.15 + contour * 0.55;
    let bump = |center: f32, height: f32, x: f32| {
        let d = (x - center) / 0.035;
        height * (-d * d).exp()
    };
    let points = samples(N)
        .map(|(_, x)| {
            let y = bump(0.5, 0.7, x)
                + bump(0.5 - left, amp * amount, x)
                + bump(0.5 + right, amp * amount, x)
                + bump(0.5 - left * 2.0, amp * amount * 0.45, x)
                + bump(0.5 + right * 2.0, amp * amount * 0.45, x);
            cl(0.18 + y)
        })
        .collect();
    let mut shot = Shot::bare("freq", points);
    shot.lfo = sine(48);
    shot
}

fn freq_shifter(params: &[(String, f64)]) -> Shot {
    let coarse = p(params, "coarse", 0.5).clamp(0.0, 1.0);
    let fine = p(params, "fine", 0.0).clamp(0.0, 1.0);
    let feedback = p(params, "feedback", 0.75).clamp(0.0, 1.0);
    let stereo = p(params, "stereo", 0.75).clamp(0.0, 1.0);
    let invert = on(params, "invert", 1.0);
    let mix = p(params, "mix", 0.75).clamp(0.0, 1.0);
    let mut shift = (coarse - 0.5) * 0.55 + fine * 0.06;
    if invert {
        shift = -shift;
    }
    let peaks = [0.22, 0.48, 0.7];
    let draw = |extra: f32| {
        samples(N)
            .map(|(_, x)| {
                let mut y = 0.16;
                for (k, c) in peaks.iter().enumerate() {
                    let at = c + shift + extra;
                    let d = (x - at) / 0.03;
                    y += (0.55 - k as f32 * 0.12) * (-d * d).exp();
                    let ghost = c + shift * (1.6 + feedback);
                    let g = (x - ghost) / 0.04;
                    y += feedback * 0.28 * (-g * g).exp();
                }
                cl(lerp(0.28, y, mix))
            })
            .collect()
    };
    let mut shot = Shot::bare("freq", draw(0.0));
    shot.extent = draw(stereo * 0.06);
    shot
}

fn lofi(params: &[(String, f64)]) -> Shot {
    let sr = p(params, "sample_rate", 0.05).clamp(0.0, 1.0);
    let hz = 44_100.0 * 2f32.powf(-127.0 * sr / 12.0);
    let wall = freq_x(hz);
    let bits = (12.0 - 11.0 * p(params, "bits", 0.185).clamp(0.0, 1.0)).clamp(1.0, 12.0);
    let smooth = p(params, "smooth", 1.0).clamp(0.0, 1.0);
    let stereo = p(params, "stereo", 1.0).clamp(0.0, 1.0);
    let mix = p(params, "mix", 1.0).clamp(0.0, 1.0);
    let points = samples(N)
        .map(|(_, x)| {
            let open = if x <= wall { 0.78 } else { 0.16 };
            cl(lerp(0.5, open, mix))
        })
        .collect();
    let steps = bits.round().max(1.0);
    let stair: Vec<f32> = (0..48)
        .map(|i| {
            let t = i as f32 / 47.0;
            let q = (t * steps).floor() / steps;
            let wave = 0.5 + 0.5 * (t * std::f32::consts::TAU).sin();
            cl(lerp(q, wave, smooth))
        })
        .collect();
    let mut shot = Shot::bare("freq", points);
    shot.extent = samples(N)
        .map(|(_, x)| {
            let open = if x <= (wall * (0.55 + 0.45 * stereo)).min(1.0) { 0.62 } else { 0.2 };
            cl(lerp(0.5, open, mix))
        })
        .collect();
    shot.lfo = stair;
    shot.bands = vec![(wall, 0.78)];
    shot
}

fn tail(length: f32, low: f32, high: f32, mix: f32, freeze: bool, pre: f32, wobble: f32) -> (Vec<f32>, Vec<f32>) {
    let len = (0.12 + length.clamp(0.0, 1.0) * 0.9) * (1.0 + wobble);
    let high_pts = samples(N)
        .map(|(_, t)| {
            if freeze {
                return cl(0.62 * mix);
            }
            if t < pre {
                return 0.04;
            }
            let u = (t - pre) / (1.0 - pre).max(0.05);
            cl(mix * (-u / (len * (0.25 + high.clamp(0.0, 1.0))).max(0.05)).exp())
        })
        .collect();
    let low_pts = samples(N)
        .map(|(_, t)| {
            if freeze {
                return cl(0.5 * mix);
            }
            if t < pre {
                return 0.04;
            }
            let u = (t - pre) / (1.0 - pre).max(0.05);
            cl(mix * (-u / (len * (0.45 + low.clamp(0.0, 1.0)) * 1.35).max(0.05)).exp())
        })
        .collect();
    (high_pts, low_pts)
}

fn reverb(params: &[(String, f64)]) -> Shot {
    let room = p(params, "room", 0.0).round().clamp(0.0, 3.0) as i32;
    let size = p(params, "size", 0.5);
    let (high, low) = tail(size, p(params, "low", 0.5), p(params, "high", 0.5), p(params, "mix", 0.5), on(params, "freeze", 0.0), 0.0, 0.0);
    let mut shot = Shot::bare("time", high);
    shot.extent = low;
    let early = match room {
        1 => vec![(0.06, 0.7), (0.1, 0.4)],
        2 => vec![(0.05, 0.55), (0.14, 0.35)],
        3 => vec![(0.04, 0.8), (0.08, 0.65), (0.16, 0.3)],
        _ => vec![(0.07, 0.5), (0.13, 0.32)],
    };
    let pan = p(params, "pan", 0.0).clamp(-1.0, 1.0);
    let stereo = p(params, "stereo", 0.0).clamp(-1.0, 1.0);
    shot.bands = early.into_iter().map(|(x, y)| (cl(x + pan * 0.04), cl(y + stereo * 0.05))).collect();
    shot
}

fn plate_mode(params: &[(String, f64)]) -> i32 {
    if params.iter().any(|(k, _)| k == "mode") {
        return p(params, "mode", 0.0).round().clamp(0.0, 2.0) as i32;
    }
    if params.iter().any(|(k, _)| k == "wet_gain") { 0 } else { 2 }
}

fn plate_mod(params: &[(String, f64)]) -> f32 {
    match plate_mode(params) {
        1 => p(params, "hall_mod", 0.4),
        2 => 0.0,
        _ => p(params, "room_mod", 0.2),
    }
}

fn plate(params: &[(String, f64)], phase: f32, page: usize) -> Shot {
    let mode = plate_mode(params);
    if page == 1 && mode != 2 {
        let (low_key, cut_key) = if mode == 1 { ("hall_low_shelf", "hall_high_cut") } else { ("room_low_shelf", "room_high_cut") };
        let shelf = p(params, low_key, 0.0).clamp(0.0, 1.0);
        let cut = p(params, cut_key, 0.0).clamp(0.0, 1.0);
        let cut_hz = 20_000.0 - 18_000.0 * cut;
        let points = samples(N)
            .map(|(_, x)| {
                let hz = 20.0 * 1000.0f32.powf(x);
                let low = if hz < 200.0 { -12.0 * shelf } else { -12.0 * shelf * (200.0 / hz).min(1.0) * 0.25 };
                let high = if hz > cut_hz { -18.0 * ((hz / cut_hz).log2().max(0.0)) } else { 0.0 };
                cl(0.5 + (low + high) / 24.0)
            })
            .collect();
        return Shot::bare("freq", points);
    }
    let (length, low, high, pre) = if mode == 2 {
        (
            p(params, "plate_decay", 0.3),
            1.0 - p(params, "plate_low_shelf", 0.0).clamp(0.0, 1.0),
            1.0 - p(params, "plate_high_damp", 0.0).clamp(0.0, 1.0),
            p(params, "plate_pre_delay", 0.0).clamp(0.0, 1.0) * 0.35,
        )
    } else {
        let prefix = if mode == 1 { "hall" } else { "room" };
        let rt = p(params, &format!("{prefix}_rev_time"), if mode == 1 { 0.4 } else { 0.2 });
        let size = p(params, &format!("{prefix}_size"), if mode == 1 { 0.1 } else { 0.2 }).clamp(0.0, 1.0);
        let secs = (3.7 * rt).exp() * 0.5;
        (
            cl(secs / 12.0) * (0.45 + size),
            1.0 - p(params, &format!("{prefix}_damp"), 0.5) * 0.3,
            1.0 - p(params, &format!("{prefix}_damp"), 0.5),
            p(params, &format!("{prefix}_pre_delay"), 0.0).clamp(0.0, 1.0) * 0.35,
        )
    };
    let wobble = if mode == 2 { 0.0 } else { (phase * std::f32::consts::TAU).sin() * plate_mod(params) * 0.15 };
    let mix = p(params, "mix", 0.5);
    let (high_pts, low_pts) = tail(length, low, high, mix, false, pre, wobble);
    let mut shot = Shot::bare("time", high_pts);
    shot.extent = low_pts;
    if mode != 2 {
        let diff = p(params, if mode == 1 { "hall_diff" } else { "room_diff" }, 0.85);
        let n = (2.0 + diff * 4.0) as usize;
        shot.bands = (0..n).map(|i| (0.04 + i as f32 * 0.03, cl(0.35 + diff * 0.3 - i as f32 * 0.05))).collect();
    }
    shot
}

fn metaverb(params: &[(String, f64)]) -> Shot {
    let (high, low) = tail(p(params, "size", 0.5), p(params, "low", 0.5), p(params, "high", 0.5), p(params, "mix", 0.5), false, 0.0, 0.0);
    let pan = p(params, "pan", 0.0).clamp(-1.0, 1.0);
    let mut shot = Shot::bare("time", high);
    shot.extent = low.into_iter().enumerate().map(|(i, y)| cl(y + pan * 0.04 * ((i as f32 / N as f32) - 0.5))).collect();
    shot
}

fn ice(params: &[(String, f64)], ice_mode: bool) -> Shot {
    let color = p(params, "color", if ice_mode { 0.75 } else { 0.5 });
    let size = p(params, "size", if ice_mode { 0.5 } else { 0.25 });
    let mix = p(params, "mix", 0.5);
    let (high, low) = tail(size, 1.0 - color * 0.5, color, mix, false, 0.0, 0.0);
    let mut shot = Shot::bare("time", high);
    shot.extent = low;
    if ice_mode {
        let ice = p(params, "ice", 0.75).clamp(0.0, 1.0);
        if let Some(y) = shot.points.first_mut() {
            *y = cl(*y + ice * 0.7);
        }
        shot.bands = vec![(0.04, cl(0.3 + ice * 0.65))];
    } else {
        let smooth = p(params, "smooth", 0.75).clamp(0.0, 1.0);
        for (i, y) in shot.points.iter_mut().take(12).enumerate() {
            let t = i as f32 / 12.0;
            *y *= lerp(t, 1.0, smooth);
        }
    }
    shot
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(field: &str, value: f64) -> Vec<(String, f64)> {
        vec![(field.into(), value)]
    }

    fn changed(module: &str, field: &str, a: f64, b: f64) -> bool {
        let left = shot(module, &one(field, a), 0.0, false, 0).unwrap();
        let right = shot(module, &one(field, b), 0.0, false, 0).unwrap();
        left.points != right.points
            || left.extent != right.extent
            || left.bands != right.bands
            || left.lfo != right.lfo
            || left.side != right.side
    }

    #[test]
    fn every_effect_has_a_picture_and_its_knobs_move_it() {
        let cases = [
            ("Compressor", "threshold", 0.05, 0.9),
            ("Limiter", "threshold", -2.0, -30.0),
            ("Gate", "mix", 0.0, 1.0),
            ("Maximizer", "amount", 0.0, 1.0),
            ("Saturator", "classic_drive", 0.0, 1.0),
            ("MullhollandDrive", "drive", 0.0, 1.0),
            ("TransientDesigner", "attack", -1.0, 1.0),
            ("BeatDelay", "feedback", 0.0, 0.9),
            ("GrainDelay", "pitch", -1.0, 1.0),
            ("GrainStretch", "on", 0.0, 1.0),
            ("Resochord", "tune", -12.0, 12.0),
            ("Chorus", "amount", 0.0, 1.0),
            ("Flanger", "frequency", 0.1, 0.9),
            ("Phaser", "eight_pole", 0.0, 1.0),
            ("FM", "amount", 0.0, 1.0),
            ("FreqShifter", "coarse", 0.1, 0.9),
            ("LoFi", "sample_rate", 0.0, 0.8),
            ("Reverb", "size", 0.05, 0.95),
            ("PlateReverb", "plate_decay", 0.05, 0.9),
            ("Metaverb", "size", 0.05, 0.95),
            ("Iceverb", "size", 0.05, 0.95),
            ("Reflex", "smooth", 0.0, 1.0),
        ];
        for (module, field, a, b) in cases {
            let picture = shot(module, &[], 0.0, false, 0).unwrap();
            assert!(picture.points.iter().chain(&picture.extent).all(|y| y.is_finite()), "{module} finite");
            assert!(changed(module, field, a, b), "{module}: {field} does not move the picture");
        }
    }

    #[test]
    fn the_side_chain_moves_the_picture() {
        let quiet = shot("Compressor", &one("side_chain_filter/gain", 0.5), 0.0, false, 1).unwrap();
        let loud = shot("Compressor", &one("side_chain_filter/gain", 1.0), 0.0, true, 1).unwrap();
        assert!(loud.side[0] > quiet.side[0] + 0.2);
        assert!(loud.side_live);
        assert!(shot("EQ", &[], 0.0, false, 0).is_none());
        assert!(shot("Filter", &[], 0.0, false, 0).is_none());
    }

    #[test]
    fn modulation_scrolls_without_a_jump_from_the_rate_alone() {
        assert!(rate_hz("Chorus", &one("rate", 0.9), 120.0) > rate_hz("Chorus", &one("rate", 0.1), 120.0) * 4.0);
        assert!(moves("Chorus", &one("amount", 0.8)));
        assert!(!moves("Chorus", &one("amount", 0.0)));
        let a = shot("Chorus", &one("amount", 0.8), 0.0, false, 0).unwrap();
        let b = shot("Chorus", &one("amount", 0.8), 0.35, false, 0).unwrap();
        assert_ne!(a.extent, b.extent);
    }

    #[test]
    fn the_plate_page_swaps_in_the_eq() {
        let room = shot("PlateReverb", &one("mode", 0.0), 0.0, false, 0).unwrap();
        let eq = shot("PlateReverb", &[("mode".into(), 0.0), ("room_high_cut".into(), 1.0)], 0.0, false, 1).unwrap();
        assert_eq!(room.kind, "time");
        assert_eq!(eq.kind, "freq");
        assert_ne!(eq.points[10], eq.points[80]);
    }
}
