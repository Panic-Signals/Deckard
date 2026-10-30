//! Modulation in the app (docs/superpowers/plans/2026-10-22-phase2c-modulation.md):
//! the knobs' scales, which the engine turns points into stored values with.

use crate::layout::Edit;
use deckard_doc::{Chain, Target};
use deckard_engine::modscale::Scale;
use deckard_model::{ModTarget, Project, SlotInfo};

/// How a knob moves its parameter, as modulation scales it.
pub fn scale_of(e: Edit) -> Option<(&'static str, Scale)> {
    Some(match e {
        Edit::Param(_, f, lo, hi, _) => (f, Scale::Lin(lo, hi)),
        Edit::Log(_, f, lo, hi, _) => (f, Scale::Log(lo, hi)),
        Edit::Int(_, f, lo, hi, _) => (f, Scale::Int(lo, hi)),
        Edit::Toggle(_, f, _) | Edit::Choice(_, f, ..) => (f, Scale::Absolute),
        _ => return None,
    })
}

/// Every knob of Maschine's own effects and synths, by (module, field), from their
/// pages.
pub fn scales() -> Vec<((String, String), Scale)> {
    let modules = deckard_model::INTERNAL_EFFECTS
        .iter()
        .map(|(c, _)| *c)
        .chain(["BassSynth", "PolySynth", deckard_model::PERFORM_FX.0])
        .chain(deckard_engine::synth::CLASSES.into_iter().filter(|c| crate::drum_ui::is_drum(c)));
    let mut out = Vec::new();
    // The Sampler's pages, in each layout (polyphony, envelope type, filter).
    for (poly, env, filter) in (0..2).flat_map(|p| (0..3).flat_map(move |e| (0..6).map(move |f| (p * 2, e, f)))) {
        let mut smp = deckard_model::Sampler::default();
        smp.set("polyphony", poly as f64);
        smp.set("envelope_type", env as f64);
        smp.set("filter", filter as f64);
        for page in 0..crate::sampler_ui::PAGES.len() {
            for (field, scale) in crate::sampler_ui::page(&smp, 0, 0, page).slots.iter().flatten().filter_map(|k| k.3.and_then(scale_of)) {
                let key = ("Sampler".to_string(), field.to_string());
                if !out.iter().any(|(k, _)| *k == key) {
                    out.push((key, scale));
                }
            }
        }
    }
    for module in modules {
        let Some(pages) = crate::module_ui::pages(module) else { continue };
        // Perform FX shows other knobs in each of its modes.
        let modes = if module == deckard_model::PERFORM_FX.0 { 8 } else { 1 };
        for (mode, page) in (0..modes).flat_map(|m| (0..pages.len()).map(move |p| (m, p))) {
            let params = vec![("mode".to_string(), mode as f64)];
            let slot = SlotInfo { module: module.into(), name: module.into(), active: true, plugin: None, params };
            let pg = crate::module_ui::page_for(&slot, Target::Module(0, 0, 0), page);
            for (field, scale) in pg.slots.iter().flatten().filter_map(|k| k.3.and_then(scale_of)) {
                let key = (module.to_string(), field.to_string());
                if !out.iter().any(|(k, _)| *k == key) {
                    out.push((key, scale));
                }
            }
        }
    }
    out
}

/// Give the engine the knobs' scales (once).
pub fn register() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| deckard_engine::modscale::set_scales(scales()));
}

/// The channel and parameter a knob changes, when its movements can be written as
/// modulation: a continuous knob of a module in a sound's or a group's chain.
pub fn mod_target(e: Edit) -> Option<(Chain, ModTarget)> {
    let (t, f) = match e {
        Edit::Param(t, f, ..) | Edit::Log(t, f, ..) | Edit::Int(t, f, ..) => (t, f),
        _ => return None,
    };
    let (c, i) = match t {
        Target::Module(g, s, i) => (Chain::Sound(g, s), i),
        Target::ChainModule(c @ (Chain::Sound(..) | Chain::Group(_)), i) => (c, i),
        _ => return None,
    };
    Some((c, ModTarget { slot: Some(i), field: f.into() }))
}

/// Where the knob of `target` (a module's parameter in chain `c`) is, 0..1.
pub fn knob_travel(p: &Project, c: Chain, target: &ModTarget) -> Option<f64> {
    let slots = match c {
        Chain::Sound(g, s) => &p.groups.get(g)?.sounds.get(s)?.slots,
        Chain::Group(g) => &p.groups.get(g)?.slots,
        Chain::Master => return None,
    };
    let slot = slots.get(target.slot?)?;
    let v = slot.param(&target.field)?;
    Some(deckard_engine::modscale::scale_of(&slot.module, &target.field).travel(v))
}

/// A knob touched while writing modulation: its deviations (pattern position, value)
/// as written so far.
pub struct ModTake {
    pub chain: Chain,
    pub target: ModTarget,
    pub pattern: usize,
    pub samples: Vec<(i64, f64)>,
    /// When the knob last moved: idle for [`crate::record::IDLE`], the touch ends.
    pub moved: std::time::Instant,
}

/// A touch's run of samples (rising positions) written as few points over its span.
pub fn write_run(d: &mut deckard_doc::Document, c: Chain, t: &ModTarget, p: usize, s: &[(i64, f64)]) -> Result<(), String> {
    if s.len() > 2 && s.windows(2).all(|w| w[0].0 < w[1].0) {
        d.set_modulation(c, t, p, (s[0].0, s[s.len() - 1].0), &thin(s, 0.005))?;
    }
    Ok(())
}

/// A recorded movement as few points: a point is kept where the movement leaves the
/// line from the last one kept by more than `tol`; the first and last always.
pub fn thin(samples: &[(i64, f64)], tol: f64) -> Vec<(i64, f64)> {
    let Some(&first) = samples.first() else { return Vec::new() };
    let mut out = vec![first];
    let mut k = 0;
    for j in 2..samples.len() {
        let (a, b) = (samples[k], samples[j]);
        let off = samples[k + 1..j].iter().any(|&(x, y)| {
            let f = (x - a.0) as f64 / (b.0 - a.0).max(1) as f64;
            (a.1 + (b.1 - a.1) * f - y).abs() > tol
        });
        if off {
            k = j - 1;
            out.push(samples[k]);
        }
    }
    if samples.len() > 1 {
        out.push(*samples.last().unwrap());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{scales, thin};
    use deckard_engine::modscale::Scale;

    #[test]
    fn the_knobs_give_the_scales() {
        let t = scales();
        let get = |m: &str, f: &str| t.iter().find(|((a, b), _)| a == m && b == f).map(|x| x.1);
        assert_eq!(get("EQ", "low_freq"), Some(Scale::Lin(0.0, 1.0)));
        assert_eq!(get("Compressor", "mode"), Some(Scale::Absolute), "a selector");
        assert!(t.iter().any(|((_, f), s)| f == "threshold" && *s == Scale::Log(0.1, 1.0)), "a log knob");
        // The Sampler's and the drum synths' too (modulation routes drive them).
        assert!(get("Sampler", "cutoff").is_some());
        assert!(get("Sampler", "volume_envelope/attack").is_some());
        assert!(get("Kick", "decay").is_some());
        // Every module with pages has its knobs.
        for (m, _) in deckard_model::INTERNAL_EFFECTS {
            if crate::module_ui::pages(m).is_some() {
                assert!(t.iter().any(|((a, _), _)| a == m), "{m}");
            }
        }
    }

    #[test]
    fn a_movement_is_thinned_to_its_turns() {
        // Up in a line, then flat: the start, the turn, the end.
        let up: Vec<(i64, f64)> = (0..=10).map(|i| (i * 10, i as f64 * 0.01)).chain((11..=20).map(|i| (i * 10, 0.1))).collect();
        assert_eq!(thin(&up, 0.005), [(0, 0.0), (100, 0.1), (200, 0.1)]);
        assert_eq!(thin(&[(5, 0.2)], 0.005), [(5, 0.2)]);
        assert!(thin(&[], 0.005).is_empty());
    }
}
