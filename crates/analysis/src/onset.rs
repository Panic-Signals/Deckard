//! The onset envelope: how much new sound starts in each short hop.

/// The envelope, one value per hop (mean removed), and the hop in frames. Each hop's
/// loudness is measured twice — the whole signal, and its first difference (a crude high
/// band, so hats count) — log-compressed; the envelope is the sum of their rises.
pub(crate) fn envelope(mono: &[f32], rate: u32) -> (Vec<f32>, f64) {
    let hop = ((256.0 * f64::from(rate) / 44_100.0).round() as usize).max(16);
    let blocks = mono.len() / hop;
    let mut prev = [0f32; 2];
    let mut env = Vec::with_capacity(blocks);
    for b in 0..blocks {
        let s = &mono[b * hop..(b + 1) * hop];
        let full = s.iter().map(|x| x * x).sum::<f32>();
        let mut last = if b > 0 { mono[b * hop - 1] } else { 0.0 };
        let mut high = 0f32;
        for &x in s {
            high += (x - last) * (x - last);
            last = x;
        }
        let level = |e: f32| (1.0 + 100.0 * (e / hop as f32).sqrt()).ln();
        let now = [level(full), level(high)];
        env.push((now[0] - prev[0]).max(0.0) + (now[1] - prev[1]).max(0.0));
        prev = now;
    }
    let mean = env.iter().sum::<f32>() / env.len().max(1) as f32;
    for v in &mut env {
        *v -= mean;
    }
    (env, hop as f64)
}
