//! The Sampling page of DECKARD's own controller layer (SAMPLE): the app's sampler and
//! recorder as its tabs show them (`sampling_hw`, `record_knobs`, shared with the
//! scripts route), and what its knobs and buttons ask of them
//! (docs/sampling-controller.md).

use crate::layout::sampling::{RecordAct, SamplingAct};
use crate::{App, record_knobs, sampling_hw};
use deckard_surface::{Action, RecordPress, RecordState, SamplingView};

impl App {
    /// The knob pages of every tab, with what each knob sets.
    fn sampling_pages(&self) -> Option<sampling_hw::Pages> {
        let d = self.doc.as_ref()?.borrow();
        let record = record_knobs::knobs(&self.recorder.view, &self.settings, d.project());
        Some(sampling_hw::pages(record, &self.sampling, d.project(), (self.group, self.sound, self.zone)))
    }

    /// What the Sampling page shows (nothing while it's closed).
    pub(crate) fn sampling_surface_view(&self) -> SamplingView {
        if !self.sampling_open {
            return SamplingView::default();
        }
        let v = &self.recorder.view;
        let mut out = SamplingView {
            tab: self.sampling.tab.min(3),
            record: RecordState {
                levels: v.meter,
                mode: v.mode as i64,
                threshold: v.threshold,
                waiting: v.status == crate::layout::sampling::RecStatus::Waiting,
                recording: v.status == crate::layout::sampling::RecStatus::Recording,
            },
            takes: v.takes.len(),
            take: v.focused,
            stems: self
                .stems
                .as_ref()
                .map(|j| deckard_surface::StemsRun { focused: j.source == (self.group, self.sound, self.zone), progress: j.progress() }),
            ..Default::default()
        };
        if let Some(p) = self.sampling_pages() {
            out.pages = p.tabs;
            out.slice = p.slice;
        }
        if let Some(doc) = &self.doc {
            out.sample = sampling_hw::sample_view(&self.sampling, doc.borrow().project(), (self.group, self.sound, self.zone), None);
        }
        out
    }

    /// RECORD's START / STOP (`toggle`) or the recording cancelled, as either
    /// controller layer asks.
    pub(crate) fn record_press(&mut self, toggle: bool) {
        let act = match (toggle, self.recorder.busy()) {
            (true, false) if self.recorder.can_start() => RecordAct::Start,
            (true, false) => return,
            (true, true) => RecordAct::Stop,
            (false, _) => RecordAct::Cancel,
        };
        self.sampling_act(SamplingAct::Record(act));
    }

    /// Carry out one of the Sampling page's actions.
    pub(crate) fn surface_sampling(&mut self, a: Action) {
        match a {
            Action::Sampling(true) => self.sample_focused_sound(),
            Action::Sampling(false) => self.sampling_open = false,
            Action::SamplingTab(t) => self.sampling_act(SamplingAct::Tab(t.min(3))),
            Action::SamplingKnob { page, knob, value } => {
                let Some(p) = self.sampling_pages() else { return };
                let tab = self.sampling.tab.min(3);
                let key = if tab == 2 {
                    p.slice.get(knob).map(|k| k.key.clone())
                } else {
                    p.tabs[tab].get(page).and_then(|pg| pg.knobs.get(knob)).map(|k| k.key.clone())
                };
                self.record_apply = p.apply;
                if let Some(key) = key {
                    self.sampling_knob(&key, value);
                }
            }
            Action::SamplingCall(c) => self.sampling_call(c),
            Action::RecordSample(p) => self.record_press(p == RecordPress::Toggle),
            Action::Take { index, delete } => {
                self.sampling_act(SamplingAct::Record(if delete { RecordAct::DeleteTake(index) } else { RecordAct::FocusTake(index) }))
            }
            _ => {}
        }
    }
}
