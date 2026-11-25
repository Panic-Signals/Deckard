//! A New Sampler sound's layers and its morph
//! (docs/superpowers/specs/2026-11-21-layers-design.md): up to eight zones over the same
//! keys, each a sample with its own envelope and a window on the sound's one morph
//! position, which a sweep moves through them.

/// A sound's layers at most.
pub const MAX_LAYERS: usize = 8;
/// The longest envelope time, seconds.
pub const TIME_MAX: f64 = 30.0;
/// A free cycle's shortest and longest, seconds.
pub const FREE: (f64, f64) = (0.05, 500.0);
/// The synced cycles, bars.
pub const SYNCED: [f64; 11] = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];

/// What makes a zone a layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub mute: bool,
    pub solo: bool,
    /// Its envelope, on top of the sound's: seconds, and the sustain level (0 to 1).
    pub attack: f64,
    pub decay: f64,
    pub sustain: f64,
    pub release: f64,
    /// Its window on the morph position: silent before A, full from B to C, silent
    /// after D. (Round's may go below 0 or above 1: they wrap.)
    pub window: [f64; 4],
}

impl Default for Layer {
    fn default() -> Layer {
        Layer { mute: false, solo: false, attack: 0.005, decay: 0.0, sustain: 1.0, release: 0.05, window: [0.0, 0.0, 1.0, 1.0] }
    }
}

/// How the sweep goes through the layers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Shape {
    /// First to last, then the first again.
    #[default]
    Round,
    /// First to last and back.
    Back,
}

/// How a window's edges fade.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Fade {
    #[default]
    Linear,
    /// Equal power: the square root of the line.
    Power,
}

/// A layered sound's morph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Morph {
    /// Off: every layer plays at its level, whatever its window.
    pub on: bool,
    pub position: f64,
    /// The cycle is `bars` of the song; else `seconds`.
    pub sync: bool,
    pub bars: f64,
    pub seconds: f64,
    pub shape: Shape,
    /// How far There and back goes (Round always goes all the way).
    pub depth: f64,
    /// Every note starts its own sweep; else the sound has one.
    pub reset: bool,
    pub fade: Fade,
    /// The windows are as SPREAD left them: adding or removing a layer spreads again.
    pub spread: bool,
}

impl Default for Morph {
    fn default() -> Morph {
        Morph {
            on: false,
            position: 0.0,
            sync: true,
            bars: 8.0,
            seconds: 8.0,
            shape: Shape::Round,
            depth: 1.0,
            reset: true,
            fade: Fade::Linear,
            spread: true,
        }
    }
}

fn finite(x: f64, or: f64) -> f64 {
    if x.is_finite() { x } else { or }
}

impl Morph {
    /// A cycle's length at the song's `tempo` (four beats a bar).
    pub fn cycle_seconds(&self, tempo: f64) -> f64 {
        if self.sync {
            let bars = if self.bars > 0.0 { finite(self.bars, 8.0) } else { 8.0 };
            bars * 4.0 * 60.0 / if tempo > 0.0 { tempo } else { 120.0 }
        } else {
            finite(self.seconds, 8.0).clamp(FREE.0, FREE.1)
        }
    }

    /// Where the morph is (0 to 1), `phase` (0 to 1) through a cycle, with POSITION at
    /// `position`.
    pub fn at(&self, position: f64, phase: f64) -> f64 {
        let (position, phase) = (finite(position, 0.0), finite(phase, 0.0));
        match self.shape {
            Shape::Round => (position + phase).rem_euclid(1.0),
            Shape::Back => {
                let up = 1.0 - (2.0 * phase.rem_euclid(1.0) - 1.0).abs();
                // (A whole cycle is back at the start.)
                let up = if phase >= 1.0 && phase.fract() == 0.0 { 0.0 } else { up };
                (position + finite(self.depth, 1.0).clamp(0.0, 1.0) * up).clamp(0.0, 1.0)
            }
        }
    }
}

/// The windows of `n` layers spread evenly: each full at its own place, fading to the
/// next one's, so two neighbours always add to one.
pub fn spread(n: usize, shape: Shape) -> Vec<[f64; 4]> {
    if n <= 1 {
        return vec![[0.0, 0.0, 1.0, 1.0]; n];
    }
    (0..n)
        .map(|i| match shape {
            Shape::Round => {
                let (c, s) = (i as f64 / n as f64, 1.0 / n as f64);
                [c - s, c, c, c + s]
            }
            Shape::Back => {
                let s = 1.0 / (n - 1) as f64;
                let c = i as f64 * s;
                [(c - s).max(0.0), c, c, (c + s).min(1.0)]
            }
        })
        .collect()
}

/// How loud window `w` is with the morph at `p` (0: silent, 1: full).
pub fn window_gain(w: [f64; 4], p: f64, shape: Shape, fade: Fade) -> f32 {
    if !p.is_finite() {
        return 0.0;
    }
    // In order, whatever was stored.
    let a = finite(w[0], 0.0);
    let b = finite(w[1], a).max(a);
    let c = finite(w[2], b).max(b);
    let d = finite(w[3], c).max(c);
    let at = |p: f64| {
        if p < a || p > d {
            0.0
        } else if p < b {
            (p - a) / (b - a)
        } else if p <= c {
            1.0
        } else {
            (d - p) / (d - c)
        }
    };
    let g = match shape {
        Shape::Round => at(p - 1.0).max(at(p)).max(at(p + 1.0)),
        Shape::Back => at(p),
    };
    let g = g.clamp(0.0, 1.0);
    (match fade {
        Fade::Linear => g,
        Fade::Power => g.sqrt(),
    }) as f32
}

/// Which layers are heard: not the muted ones, and when any is soloed only those.
pub fn audible(layers: &[Layer]) -> Vec<bool> {
    let solo = layers.iter().any(|l| l.solo);
    layers.iter().map(|l| !l.mute && (!solo || l.solo)).collect()
}

/// The sound's layer zones, in zone order.
pub fn zones(smp: &crate::Sampler) -> Vec<usize> {
    smp.zones.iter().enumerate().filter(|(_, z)| z.meta.as_ref().is_some_and(|m| m.layer.is_some())).map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spread_evenly() {
        assert_eq!(spread(1, Shape::Round), vec![[0.0, 0.0, 1.0, 1.0]]);
        assert_eq!(spread(2, Shape::Round), vec![[-0.5, 0.0, 0.0, 0.5], [0.0, 0.5, 0.5, 1.0]]);
        assert_eq!(spread(3, Shape::Back), vec![[0.0, 0.0, 0.0, 0.5], [0.0, 0.5, 0.5, 1.0], [0.5, 1.0, 1.0, 1.0]]);
        // Neighbours add to one, everywhere.
        for shape in [Shape::Round, Shape::Back] {
            for n in [2, 3, 8] {
                let w = spread(n, shape);
                for k in 0..=100 {
                    let p = f64::from(k) / 100.0;
                    let sum: f32 = w.iter().map(|&w| window_gain(w, p, shape, Fade::Linear)).sum();
                    assert!((sum - 1.0).abs() < 1e-5, "{n} {shape:?} at {p}: {sum}");
                }
            }
        }
    }

    #[test]
    fn a_window_fades() {
        let w = [0.2, 0.4, 0.6, 0.8];
        let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
        assert_eq!(window_gain(w, 0.1, Shape::Back, Fade::Linear), 0.0);
        assert!(near(window_gain(w, 0.3, Shape::Back, Fade::Linear), 0.5));
        assert!(near(window_gain(w, 0.3, Shape::Back, Fade::Power), 0.707_106_77));
        assert_eq!(window_gain(w, 0.5, Shape::Back, Fade::Linear), 1.0);
        assert!(near(window_gain(w, 0.7, Shape::Back, Fade::Linear), 0.5));
        // Round wraps: the first layer of two is full at 1 as at 0.
        assert_eq!(window_gain([-0.5, 0.0, 0.0, 0.5], 1.0, Shape::Round, Fade::Linear), 1.0);
        assert!(near(window_gain([-0.5, 0.0, 0.0, 0.5], 0.75, Shape::Round, Fade::Linear), 0.5));
        // Out of order, no width, not a number: within 0 to 1 all the same.
        for w in [[0.5, 0.2, 0.1, 0.0], [0.3, 0.3, 0.3, 0.3], [f64::NAN, 0.0, 1.0, 1.0], [0.0, f64::INFINITY, 1.0, f64::NAN]] {
            for k in 0..=10 {
                let g = window_gain(w, f64::from(k) / 10.0, Shape::Round, Fade::Power);
                assert!((0.0..=1.0).contains(&g), "{w:?}: {g}");
            }
        }
        assert_eq!(window_gain(w, f64::NAN, Shape::Round, Fade::Linear), 0.0);
    }

    #[test]
    fn where_the_morph_is() {
        let m = Morph::default();
        assert!((m.at(0.25, 0.9) - 0.15).abs() < 1e-9);
        let b = Morph { shape: Shape::Back, depth: 0.5, ..m };
        assert_eq!(b.at(0.0, 0.5), 0.5);
        assert_eq!(b.at(0.0, 1.0), 0.0);
        assert_eq!(b.at(0.0, 0.0), 0.0);
        assert_eq!(b.at(0.9, 0.5), 1.0);
        assert_eq!(Morph { sync: true, bars: 8.0, ..m }.cycle_seconds(120.0), 16.0);
        assert_eq!(Morph { sync: false, seconds: 3.0, ..m }.cycle_seconds(120.0), 3.0);
        assert_eq!(Morph { sync: false, seconds: 0.0, ..m }.cycle_seconds(120.0), FREE.0);
    }

    #[test]
    fn solo_and_mute() {
        let l = Layer::default();
        assert_eq!(audible(&[l, Layer { mute: true, ..l }, l]), [true, false, true]);
        assert_eq!(audible(&[l, Layer { solo: true, ..l }, Layer { solo: true, mute: true, ..l }]), [false, true, false]);
    }
}
