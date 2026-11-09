//! Session-only hosted parameter snapshots. No instances are loaded for LOCK.
use crate::App;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::{Rc, Weak as RcWeak},
    sync::{Arc, Mutex, Weak},
};

#[derive(Default)]
pub struct HostedLocks {
    doc: RcWeak<RefCell<deckard_doc::Document>>,
    pub(crate) slots: BTreeMap<usize, Vec<HostedSnapshot>>,
}

#[derive(Clone)]
pub(crate) struct HostedSnapshot {
    instance: Weak<Mutex<deckard_plugins::Plugin>>,
    values: Vec<(u32, i32, f64)>,
}

#[derive(Clone)]
pub(crate) struct HostedGlide {
    snapshot: HostedSnapshot,
    from: Vec<f64>,
}

impl App {
    pub(crate) fn lock_hosted_prepare(&mut self) {
        if let Some(doc) = &self.doc {
            let same = self.lock_plugins.doc.upgrade().is_some_and(|old| Rc::ptr_eq(&old, doc));
            if !same {
                self.lock_plugins = HostedLocks { doc: Rc::downgrade(doc), ..Default::default() };
            }
        }
    }

    pub(crate) fn lock_hosted_capture(&self) -> Vec<HostedSnapshot> {
        self.samples
            .plugins
            .instances()
            .filter_map(|(_, instance)| {
                let p = instance.lock().ok()?;
                let values = p.parameters().iter().map(|info| (info.id, info.steps, p.knob_value(info.id))).collect();
                Some(HostedSnapshot { instance: Arc::downgrade(instance), values })
            })
            .collect()
    }

    pub(crate) fn lock_hosted_glides(&self, slot: usize) -> Vec<HostedGlide> {
        self.lock_plugins
            .slots
            .get(&slot)
            .into_iter()
            .flatten()
            .filter_map(|snapshot| {
                let instance = snapshot.instance.upgrade()?;
                if !self.samples.plugins.instances().any(|(_, current)| Arc::ptr_eq(current, &instance)) {
                    return None;
                }
                let p = instance.lock().ok()?;
                let from = snapshot.values.iter().map(|(id, _, _)| p.knob_value(*id)).collect();
                Some(HostedGlide { snapshot: snapshot.clone(), from })
            })
            .collect()
    }

    pub(crate) fn lock_hosted_step(&mut self, glides: &[HostedGlide], t: f64) {
        let mut changed = false;
        for glide in glides {
            let Some(instance) = glide.snapshot.instance.upgrade() else { continue };
            if !self.samples.plugins.instances().any(|(_, current)| Arc::ptr_eq(current, &instance)) {
                continue;
            }
            let Ok(mut p) = instance.lock() else { continue };
            let changes: Vec<_> = glide
                .snapshot
                .values
                .iter()
                .zip(&glide.from)
                .filter_map(|(&(id, steps, target), &from)| {
                    if steps != 0 && t < 1.0 {
                        return None;
                    }
                    let value = from + (target - from) * t;
                    (value.is_finite() && (p.knob_value(id) - value).abs() > 1e-9).then_some((id, value))
                })
                .collect();
            if changes.is_empty() {
                continue;
            }
            changed = true;
            if let Some(params) = p.params() {
                drop(p);
                for (id, value) in changes {
                    params.set(id, value);
                }
            } else {
                for (id, value) in changes {
                    p.set_param(id, value);
                }
            }
        }
        if changed {
            for view in self.plugin_views.values_mut() {
                view.page = usize::MAX;
            }
        }
    }
}
