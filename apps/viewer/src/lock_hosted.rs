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
    // Snapshot version identities survive undo/redo; payloads are session-only.
    pub(crate) slots: BTreeMap<u64, Vec<HostedSnapshot>>,
    states: BTreeMap<u64, Vec<HostedSnapshot>>,
    applied_state: Option<u64>,
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
        let Some(id) = self.doc.as_ref().and_then(|d| d.borrow().snapshot_id(slot)) else { return Vec::new() };
        let Some(snapshots) = self.lock_plugins.slots.get(&id) else { return Vec::new() };
        self.lock_hosted_glides_of(snapshots)
    }

    fn lock_hosted_glides_of(&self, snapshots: &[HostedSnapshot]) -> Vec<HostedGlide> {
        snapshots
            .iter()
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

    /// Record each recall's own before/after values. Tokens live in Document's
    /// session history; parameter payloads stay here and never enter saved files.
    pub(crate) fn lock_hosted_record(&mut self, glides: &[HostedGlide]) -> Option<u64> {
        if !glides
            .iter()
            .any(|g| g.snapshot.values.iter().zip(&g.from).any(|((_, _, to), from)| to.is_finite() && (to - from).abs() > 1e-9))
        {
            return None;
        }
        let Some(doc) = self.doc.clone() else { return None };
        let before = self.lock_plugins.states.len() as u64;
        let after = before + 1;
        let from = glides
            .iter()
            .map(|g| {
                let mut snapshot = g.snapshot.clone();
                for ((_, _, value), from) in snapshot.values.iter_mut().zip(&g.from) {
                    *value = *from;
                }
                snapshot
            })
            .collect();
        self.lock_plugins.states.insert(before, from);
        self.lock_plugins.states.insert(after, glides.iter().map(|g| g.snapshot.clone()).collect());
        let mut d = doc.borrow_mut();
        d.init_session_value("hosted-lock", before);
        if let Err(e) = d.set_session_value("Recall Lock Snapshot", "hosted-lock", after) {
            self.status = format!("can't record hosted Lock: {e}");
        }
        self.lock_plugins.applied_state = Some(after);
        Some(after)
    }

    pub(crate) fn lock_capture_morph_state(&mut self) {
        if let Some(id) = self.lock_morph.as_ref().and_then(|m| m.hosted_state) {
            let current = self.lock_hosted_capture();
            self.lock_plugins.states.insert(id, current);
        }
    }

    /// All undo paths share the document token, including controller step undo.
    pub(crate) fn lock_hosted_sync(&mut self) {
        self.lock_hosted_prepare();
        let token = self.doc.as_ref().and_then(|d| d.borrow().session_value("hosted-lock"));
        if token == self.lock_plugins.applied_state {
            return;
        }
        self.lock_capture_morph_state();
        self.lock_morph = None;
        let glides = token
            .and_then(|id| self.lock_plugins.states.get(&id))
            .map(|snapshots| self.lock_hosted_glides_of(snapshots))
            .unwrap_or_default();
        self.lock_hosted_step(&glides, 1.0);
        self.lock_plugins.applied_state = token;
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
