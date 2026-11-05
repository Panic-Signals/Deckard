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
        _ => {
            let _ = params;
            false
        }
    }
}

pub fn rate_hz(module: &str, params: &[(String, f64)], tempo: f64) -> f32 {
    match module {
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

fn side(params: &[(String, f64)], live: bool, shot: &mut Shot) {
    shot.side = super::sidechain::response(params, N);
    shot.side_live = live;
}

/// The picture for `module`, or `None` for the EQ, the Filter, and anything else.
/// `phase` is the modulator's position, 0..1, advanced by [`rate_hz`].
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
}
