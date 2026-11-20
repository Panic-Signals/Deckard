use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use deckard_doc::Chain;
use deckard_engine::eq_spectrum::EqSpectrum;
use deckard_engine::spectrum::Analyzer;
use deckard_model::{SlotInfo, extra_out::Owner};
use deckard_plugins::spectrum::{self, Reader, Tap};
use deckard_surface::EqCurveView;

pub(crate) const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const STALE_AFTER: Duration = Duration::from_millis(250);

enum Source {
    Hosted { instance: Weak<Mutex<deckard_plugins::Plugin>>, pending: Option<Tap> },
    Internal { board: Weak<EqSpectrum>, owner: Owner, generation: u64 },
}

pub(crate) struct Focus {
    source: Source,
    reader: Reader,
    analyzer: Option<Analyzer>,
    shown: Option<(Instant, [Vec<f32>; 2])>,
}

impl Focus {
    pub(crate) fn is_live(&self) -> bool {
        self.shown.as_ref().is_some_and(|(at, _)| at.elapsed() < STALE_AFTER)
    }

    fn curves(&mut self) -> [Vec<f32>; 2] {
        let now = Instant::now();
        if self.shown.as_ref().is_none_or(|(at, _)| now.duration_since(*at) >= FRAME_INTERVAL)
            && let Some(analyzer) = &mut self.analyzer
            && let Some(window) = self.reader.receive()
        {
            let curves = analyzer.curves(window);
            self.shown = Some((now, curves.map(|curve| curve.to_vec())));
        }
        self.shown
            .as_ref()
            .filter(|(at, _)| now.duration_since(*at) < STALE_AFTER)
            .map_or_else(Default::default, |(_, curves)| curves.clone())
    }
}

impl crate::App {
    pub(crate) fn plugin_spectrum_view(&self, chain: Chain, slot: Option<(usize, &SlotInfo)>) -> Option<EqCurveView> {
        let mut focus = self.plugin_spectrum.borrow_mut();
        let Some((i, slot)) = slot.filter(|_| !self.channel) else {
            *focus = None;
            return None;
        };
        let mut response = Self::eq_curve(slot);
        if self.surface.page() != "control" || matches!(self.controller, Some(Ok(_))) {
            *focus = None;
            return response;
        }

        if let Some(curve) = &mut response {
            if slot.active
                && self.surface.mapping_shows_eq(&slot.module, "", "")
                && let Some(Ok(output)) = &self.output
            {
                let owner = match chain {
                    Chain::Sound(g, s) => Owner::Chain(g, s, i),
                    Chain::Group(g) => Owner::Group(g, i),
                    Chain::Master => Owner::Master(i),
                };
                let board = output.eq_spectrum();
                if !focus.as_ref().is_some_and(|f| matches!(&f.source,
                    Source::Internal { board: old, owner: at, generation } if old.as_ptr() == Arc::as_ptr(board) && *at == owner && *generation == self.doc_generation
                )) {
                    let source = Source::Internal { board: Arc::downgrade(board), owner, generation: self.doc_generation };
                    *focus = Some(Focus { source, reader: board.watch(owner), analyzer: Some(Analyzer::new(spectrum::FRAMES, output.rate as f32)), shown: None });
                }
                curve.spectrum = focus.as_mut()?.curves();
            } else {
                *focus = None;
            }
            return response;
        }

        let tagged = slot.plugin.as_ref().is_some_and(|plugin| {
            let mut id = plugin.id.split(':');
            let vendor = id.next().unwrap_or_default();
            matches!(id.nth(1), Some("FX" | "Fx" | "MF")) && self.surface.mapping_shows_eq(&plugin.name, vendor, &plugin.id)
        });
        if !tagged {
            *focus = None;
            return None;
        }
        let mut curve = EqCurveView { kind: "spectrum".into(), ..Default::default() };
        let Some(instance) = slot.active.then(|| self.samples.plugins.existing(self.plugin_slot_key(chain, i))).flatten() else {
            *focus = None;
            return Some(curve);
        };
        if !focus
            .as_ref()
            .is_some_and(|f| matches!(&f.source, Source::Hosted { instance: old, .. } if old.as_ptr() == Arc::as_ptr(&instance)))
        {
            let (tap, reader) = spectrum::channel();
            let source = Source::Hosted { instance: Arc::downgrade(&instance), pending: Some(tap) };
            *focus = Some(Focus { source, reader, analyzer: None, shown: None });
        }
        let focus = focus.as_mut()?;
        if let Source::Hosted { pending, .. } = &mut focus.source
            && pending.is_some()
            && let Ok(mut plugin) = instance.try_lock()
        {
            let rate = plugin.sample_rate() as f32;
            if let Some(tap) = pending.take() {
                plugin.set_spectrum_tap(tap);
            }
            drop(plugin);
            focus.analyzer = Some(Analyzer::new(spectrum::FRAMES, rate));
        }
        [curve.side, curve.points] = focus.curves();
        Some(curve)
    }
}
