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
    /// Only the output's curve (a filter picture's): the input's is left empty.
    output_only: bool,
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
            let input = if self.output_only { Vec::new() } else { analyzer.curve(window, 0).to_vec() };
            self.shown = Some((now, [input, analyzer.curve(window, 1).to_vec()]));
        }
        self.shown
            .as_ref()
            .filter(|(at, _)| now.duration_since(*at) < STALE_AFTER)
            .map_or_else(Default::default, |(_, curves)| curves.clone())
    }
}

impl crate::App {
    /// The focused slot's EQ curve for the right screen (with live spectra), and the
    /// focused hosted plug-in's output spectrum for the filter pictures over its knobs.
    /// The spectra are tapped (and their FFTs run) only while one is on the screen.
    pub(crate) fn plugin_spectrum_view(&self, chain: Chain, slot: Option<(usize, &SlotInfo)>) -> (Option<EqCurveView>, Vec<f32>) {
        let mut output = Vec::new();
        let curve = self.plugin_spectrum_curves(chain, slot, &mut output);
        (curve, output)
    }

    fn plugin_spectrum_curves(&self, chain: Chain, slot: Option<(usize, &SlotInfo)>, output: &mut Vec<f32>) -> Option<EqCurveView> {
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
                    let analyzer = Some(Analyzer::new(spectrum::FRAMES, output.rate as f32));
                    *focus = Some(Focus { source, reader: board.watch(owner), analyzer, shown: None, output_only: false });
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
        // A synth's (or an effect's) filter pictures over the knobs: its output behind them.
        if !tagged {
            if slot.plugin.is_some() && self.surface.drew_filter_picture() {
                if let Some([_, out]) = self.hosted_curves(&mut focus, chain, i, slot, true) {
                    *output = out;
                }
            } else {
                *focus = None;
            }
            return None;
        }
        let mut curve = EqCurveView { kind: "spectrum".into(), ..Default::default() };
        if let Some(curves) = self.hosted_curves(&mut focus, chain, i, slot, false) {
            [curve.side, curve.points] = curves;
        }
        Some(curve)
    }

    /// The input and output spectra of hosted plug-in `slot` (at `i` in `chain`), tapping
    /// it if `focus` doesn't already; `None` while it isn't loaded (or is bypassed).
    /// `output_only` leaves the input's empty (and its FFTs undone).
    fn hosted_curves(
        &self,
        focus: &mut Option<Focus>,
        chain: Chain,
        i: usize,
        slot: &SlotInfo,
        output_only: bool,
    ) -> Option<[Vec<f32>; 2]> {
        let Some(instance) = slot.active.then(|| self.samples.plugins.existing(self.plugin_slot_key(chain, i))).flatten() else {
            *focus = None;
            return None;
        };
        if !focus.as_ref().is_some_and(|f| {
            f.output_only == output_only
                && matches!(&f.source, Source::Hosted { instance: old, .. } if old.as_ptr() == Arc::as_ptr(&instance))
        }) {
            let (tap, reader) = spectrum::channel();
            let source = Source::Hosted { instance: Arc::downgrade(&instance), pending: Some(tap) };
            *focus = Some(Focus { source, reader, analyzer: None, shown: None, output_only });
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
        Some(focus.curves())
    }
}
