//! The modulation matrix at play (docs/superpowers/specs/2026-10-24-expression-out-design.md,
//! 2b): each sound's held keys (their live expression) and sounding recorded notes (their
//! curves) give its sources (pressure: the hardest-pressed note's; slide and pitch: the
//! latest note's).

use super::{Dim, Engine, ExprKind, pads};
use deckard_model::ExprId;

/// Notes followed at once (beyond: the oldest goes).
pub(crate) const VOICES: usize = 256;

/// Keys held at once (beyond: the oldest goes).
pub(crate) const KEYS: usize = 128;

/// Plug-in parameters waiting to be let go at once (beyond: left modulated).
pub(crate) const UNROUTED: usize = 64;

/// A sounding recorded note's expression, read `tps` ticks a frame from `started`, until
/// frame `until`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExprVoice {
    pub group: usize,
    pub sound: usize,
    pub pitch: i64,
    pub started: u64,
    pub until: u64,
    pub tps: f64,
    pub expr: ExprId,
    pub order: u64,
}

/// A key (a pad) held down, with what's played on it (by [`Dim::ALL`]): it counts for
/// the sources while it's held, whether or not a note sounds (Note Repeat's gaps).
#[derive(Debug, Clone, Copy)]
pub(crate) struct HeldKey {
    pub group: usize,
    pub sound: usize,
    pub pitch: i64,
    pub live: [Option<f32>; 3],
    pub order: u64,
}

impl Engine {
    /// A note of sound (`group`, `sound`) starts now, for `frames` (`None`: until
    /// released): a held key, and a recorded note's curves.
    pub(crate) fn touch_start(&mut self, group: usize, sound: usize, pitch: i64, frames: Option<u64>, tps: f64, expr: Option<ExprId>) {
        let now = self.time;
        match (frames, expr) {
            (None, _) => self.touch_hold(group, sound, pitch),
            (Some(f), Some(expr)) => {
                self.touch.retain(|v| v.until > now && !(v.group == group && v.sound == sound && v.pitch == pitch));
                if self.touch.len() >= VOICES {
                    self.touch.remove(0);
                }
                self.touch_order += 1;
                self.touch.push(ExprVoice { group, sound, pitch, started: now, until: now + f, tps, expr, order: self.touch_order });
            }
            (Some(_), None) => {}
        }
    }

    /// Key `pitch` of sound (`group`, `sound`) held (kept if it is).
    fn touch_hold(&mut self, group: usize, sound: usize, pitch: i64) {
        if self.held_keys.iter().any(|k| (k.group, k.sound, k.pitch) == (group, sound, pitch)) {
            return;
        }
        if self.held_keys.len() >= KEYS {
            self.held_keys.remove(0);
        }
        self.touch_order += 1;
        self.held_keys.push(HeldKey { group, sound, pitch, live: [None; 3], order: self.touch_order });
    }

    /// A held key's live expression.
    pub(crate) fn touch_live(&mut self, (group, sound, pitch): (usize, usize, i64), kind: ExprKind, value: f32) {
        let ExprKind::Dim(d) = kind else { return };
        let i = Dim::ALL.iter().position(|x| *x == d).unwrap_or(0);
        for k in self.held_keys.iter_mut().filter(|k| (k.group, k.sound, k.pitch) == (group, sound, pitch)) {
            k.live[i] = Some(value);
        }
    }

    /// A held key let go.
    pub(crate) fn touch_release(&mut self, group: usize, sound: usize, pitch: i64) {
        self.held_keys.retain(|k| (k.group, k.sound, k.pitch) != (group, sound, pitch));
    }

    /// Keys pressed or let go live, on each sound the pad plays (before Note Repeat
    /// takes them: its repeats come and go while the key is held).
    pub(crate) fn touch_keys(&mut self, group: usize, sound: usize, pitch: i64, down: bool) {
        let (plays, _) = self.pads.hit(group, sound);
        for s in pads::bits(plays) {
            let p = self.pads.pitch(group, sound, s, pitch);
            if down {
                self.touch_hold(group, s, p);
            } else {
                self.touch_release(group, s, p);
            }
        }
    }

    /// Sound (`group`, `sound`)'s sources now, by [`Dim::ALL`] (pitch in semitones,
    /// slide, pressure): `None` where no held key or sounding note has that dimension.
    pub fn sources(&self, group: usize, sound: usize) -> [Option<f32>; 3] {
        let mut out = [None; 3];
        let mut latest = [0u64; 3];
        let mut take = |i: usize, x: f32, order: u64| {
            if i == 2 {
                out[i] = Some(out[i].map_or(x, |o: f32| o.max(x)));
            } else if order > latest[i] {
                out[i] = Some(x);
                latest[i] = order;
            }
        };
        for k in self.held_keys.iter().filter(|k| k.group == group && k.sound == sound) {
            for (i, x) in k.live.iter().enumerate() {
                if let Some(x) = x {
                    take(i, *x, k.order);
                }
            }
        }
        for v in self.touch.iter().filter(|v| v.group == group && v.sound == sound && v.until > self.time) {
            let Some(expr) = self.expression(v.expr) else { continue };
            let at = ((self.time - v.started) as f64 * v.tps) as i64;
            for (i, d) in Dim::ALL.iter().enumerate() {
                if let Some(c) = expr.curve(*d) {
                    take(i, c.value_at(at), v.order);
                }
            }
        }
        out
    }
}

/// What a route drives, found when the engine is made.
pub(crate) enum RouteTarget {
    /// A field of the internal effect in chain slot `slot`, over its knob's scale.
    Fx { slot: usize, field: String, scale: crate::modscale::Scale },
    /// A field of the sound's Sampler or synth, over its scale; `base`: its stored value.
    Sound { field: String, scale: crate::modscale::Scale, base: f64 },
    /// A plug-in's parameter (normalized).
    Plugin { plugin: super::Shared, id: u32 },
}

/// A source resolved off the audio thread, with its original board cell.
pub(crate) struct RouteRt {
    pub index: usize,
    pub key: u32,
    pub route: deckard_model::mod_matrix::ModRoute,
    pub rest: f32,
}

/// All routes on one destination, gathered before playback. The first route owns
/// the board cell for the final travel; other cells stay empty.
pub(crate) struct TargetRt {
    pub group: usize,
    pub sound: usize,
    pub routes: Vec<RouteRt>,
    pub target: RouteTarget,
    pub now: f32,
    pub sent: f32,
}

impl TargetRt {
    fn same(&self, o: &TargetRt) -> bool {
        let (a, b) = (&self.routes[0].route, &o.routes[0].route);
        (self.group, self.sound, a.slot, &a.module, &a.param) == (o.group, o.sound, b.slot, &b.module, &b.param)
            && match (&self.target, &o.target) {
                (RouteTarget::Plugin { plugin: a, .. }, RouteTarget::Plugin { plugin: b, .. }) => std::sync::Arc::ptr_eq(a, b),
                (RouteTarget::Plugin { .. }, _) | (_, RouteTarget::Plugin { .. }) => false,
                _ => true,
            }
    }
}

/// The routes of `project`'s sounds whose slot holds the module they name (plug-ins as
/// loaded in `plugins`).
pub(crate) fn routes_of(project: &deckard_model::Project, plugins: &deckard_plugins::Host) -> Vec<TargetRt> {
    use deckard_model::mod_matrix::ModParam;
    use deckard_plugins::Slot;
    let mut out: Vec<TargetRt> = Vec::new();
    for (g, grp) in project.groups.iter().enumerate() {
        for (s, snd) in grp.sounds.iter().enumerate() {
            for (k, r) in snd.mod_routes.iter().enumerate() {
                let Some(info) = snd.slots.get(r.slot) else { continue };
                let target = match (&r.param, &info.plugin) {
                    (ModParam::Plugin(id), Some(pr)) if pr.id == r.module => {
                        let key = if r.slot == 0 && snd.plugin.is_some() {
                            Slot::Sound { group: g, sound: s }
                        } else {
                            Slot::Chain { group: g, sound: s, slot: r.slot }
                        };
                        let Some(plugin) = plugins.loaded(key, &pr.id, &pr.name) else { continue };
                        RouteTarget::Plugin { plugin, id: *id }
                    }
                    (ModParam::Field(f), None) if info.module == r.module => {
                        let scale = crate::modscale::scale_of(&info.module, f);
                        if info.module == "Sampler" {
                            let Some(smp) = snd.sampler.as_ref().filter(|_| crate::sampler::MODULATED.contains(&f.as_str())) else {
                                continue;
                            };
                            RouteTarget::Sound { field: f.clone(), scale, base: smp.get(f) }
                        } else if r.slot == 0 && crate::synth::CLASSES.contains(&info.module.as_str()) {
                            let Some(base) = info.param(f) else { continue };
                            RouteTarget::Sound { field: f.clone(), scale, base }
                        } else {
                            if info.param(f).is_none() {
                                continue;
                            }
                            RouteTarget::Fx { slot: r.slot, field: f.clone(), scale }
                        }
                    }
                    _ => continue,
                };
                let key = crate::meters::route_key(r);
                let rest = match r.source {
                    deckard_model::mod_matrix::ModSource::Pitch => 0.5,
                    deckard_model::mod_matrix::ModSource::Lfo(n) => {
                        project.lfos.iter().find(|l| l.pad == n && !l.env).map_or(0.0, |l| l.level / 2.0 + l.offset)
                    }
                    _ => 0.0,
                };
                let route = RouteRt { index: k, key, route: r.clone(), rest };
                if let Some(t) = out.iter_mut().find(|t| {
                    let a = &t.routes[0].route;
                    (t.group, t.sound, a.slot, &a.module, &a.param) == (g, s, r.slot, &r.module, &r.param)
                }) {
                    t.routes.push(route);
                } else {
                    out.push(TargetRt { group: g, sound: s, routes: vec![route], target, now: f32::NAN, sent: f32::NAN });
                }
            }
        }
    }
    out
}

impl Engine {
    /// Apply one sum per target after the lanes, over the knob (or the last active
    /// Range sweep). No target lookup, allocation or blocking lock on this path.
    pub(crate) fn apply_routes(&mut self) {
        use deckard_model::mod_matrix::{ModSource, RouteMode};
        for i in 0..self.mod_routes.len() {
            let t = &self.mod_routes[i];
            let (g, s) = (t.group, t.sound);
            let mut sum = 0.0f64;
            let mut range = None;
            let mut active = false;
            for r in &t.routes {
                let x = match r.route.source {
                    ModSource::Pitch => self.sources(g, s)[0],
                    ModSource::Slide => self.sources(g, s)[1],
                    ModSource::Pressure => self.sources(g, s)[2],
                    ModSource::Lfo(n) => match self.lfo_value(n) {
                        Some(v) => Some(v),
                        None => continue,
                    },
                };
                let v = match r.route.mode {
                    RouteMode::Range => r.route.travel(x),
                    RouteMode::Add => r.route.contribution(x, r.rest),
                };
                if !v.is_finite() {
                    continue;
                }
                active = true;
                match r.route.mode {
                    RouteMode::Range => range = Some(f64::from(v)),
                    RouteMode::Add => sum += f64::from(v),
                }
            }
            let t = &mut self.mod_routes[i];
            t.now = f32::NAN;
            match &t.target {
                RouteTarget::Fx { slot, field, scale } if active => {
                    let Some(chain) = self.fx.get_mut(g).and_then(|x| x.get_mut(s)).and_then(Option::as_mut) else { continue };
                    for u in chain.units.iter_mut().filter(|u| u.slot == *slot && !u.ghost) {
                        if let super::FxUnit::Internal { params, modded, hit, .. } = &mut u.fx {
                            let Some(base) = params.iter().find(|(k, _)| k == field).map(|(_, v)| *v).filter(|v| v.is_finite()) else {
                                continue;
                            };
                            let travel = (range.unwrap_or_else(|| scale.travel(base)) + sum).clamp(0.0, 1.0);
                            let value = scale.at(travel);
                            if !value.is_finite() {
                                continue;
                            }
                            if !*hit {
                                // The fields were prepared with the effect. Copy only
                                // their values: cloning (String, f64) tuples allocates.
                                for (field, value) in modded.iter_mut() {
                                    if let Some((_, base)) = params.iter().find(|(k, _)| k == field) {
                                        *value = *base;
                                    }
                                }
                                *hit = true;
                            }
                            if let Some(x) = modded.iter_mut().find(|(k, _)| k == field) {
                                x.1 = value;
                                t.now = travel as f32;
                            }
                        }
                    }
                }
                RouteTarget::Sound { field, scale, base } if active && base.is_finite() => {
                    let travel = (range.unwrap_or_else(|| scale.travel(*base)) + sum).clamp(0.0, 1.0);
                    let value = scale.at(travel);
                    if value.is_finite()
                        && let Some(Some(snd)) = self.sounds.get_mut(g).and_then(|x| x.get_mut(s))
                    {
                        snd.begin_modulation();
                        snd.modulate(field, value);
                        t.now = travel as f32;
                    }
                }
                RouteTarget::Plugin { plugin, id } => {
                    if let Ok(mut p) = plugin.try_lock() {
                        if active {
                            let base = range.unwrap_or_else(|| p.knob_value(*id));
                            if base.is_finite() {
                                t.now = (base + sum).clamp(0.0, 1.0) as f32;
                                if t.sent != t.now {
                                    p.modulate(*id, f64::from(t.now));
                                    t.sent = t.now;
                                }
                            }
                        } else if t.sent.is_finite() {
                            p.unmodulate(*id);
                            t.sent = f32::NAN;
                        }
                    } else {
                        // The plug-in still plays the last travel we sent.
                        t.now = t.sent;
                    }
                }
                _ => {}
            }
            if let Some(b) = &self.route_board {
                for (k, r) in t.routes.iter().enumerate() {
                    b.set(g, s, r.index, r.key, if k == 0 { t.now } else { f32::NAN });
                }
            }
        }
    }

    /// Route `r` is gone: its target has its own value again (an effect's comes back
    /// by itself, when no lane or route holds it).
    fn unroute(&mut self, r: &TargetRt) {
        match &r.target {
            RouteTarget::Fx { .. } => {}
            RouteTarget::Sound { field, base, .. } => {
                if let Some(Some(snd)) = self.sounds.get_mut(r.group).and_then(|x| x.get_mut(r.sound)) {
                    snd.modulate(field, *base);
                }
            }
            RouteTarget::Plugin { plugin, id } => match plugin.try_lock() {
                Ok(mut p) => p.unmodulate(*id),
                // Busy: let go at a later span.
                Err(_) if self.unrouted.len() < self.unrouted.capacity() => self.unrouted.push((plugin.clone(), *id)),
                Err(_) => {}
            },
        }
    }

    /// Plug-in parameters a route let go of while the plug-in was busy, let go now if
    /// it's free.
    pub(crate) fn retry_unroutes(&mut self) {
        self.unrouted.retain(|(plugin, id)| match plugin.try_lock() {
            Ok(mut p) => {
                p.unmodulate(*id);
                false
            }
            Err(_) => true,
        });
    }

    /// Take over `old`'s plug-in parameters still to let go (it's being replaced).
    pub fn take_unroutes(&mut self, old: &mut Engine) {
        for x in old.unrouted.drain(..) {
            if self.unrouted.len() < self.unrouted.capacity() {
                self.unrouted.push(x);
            }
        }
    }

    /// Take `fresh`'s routes (an edit), ending the ones it doesn't have; the old ones go
    /// back in `fresh` (to be freed off the audio thread).
    pub(crate) fn adopt_routes(&mut self, fresh: &mut Engine) {
        std::mem::swap(&mut self.mod_routes, &mut fresh.mod_routes);
        // Clear old cells, including ones now belonging to a different first route.
        // The new sums are published on the next span.
        if let Some(b) = &self.route_board {
            for t in &fresh.mod_routes {
                for r in &t.routes {
                    b.set(t.group, t.sound, r.index, r.key, f32::NAN);
                }
            }
        }
        for n in &mut self.mod_routes {
            if let Some(o) = fresh.mod_routes.iter().find(|o| o.same(n)) {
                n.sent = o.sent;
            }
        }
        // (A Sampler's or synth's settings are the edit's own, the knobs' values: only
        // plug-ins, which outlive engines, are let go here.)
        for r in fresh.mod_routes.iter() {
            if matches!(r.target, RouteTarget::Plugin { .. }) && !self.mod_routes.iter().any(|n| n.same(r)) {
                self.unroute(r);
            }
        }
    }

    /// End every route (this engine is replaced by another, or the project closed):
    /// plug-ins play their knobs again.
    pub fn release_routes(&mut self) {
        let routes = std::mem::take(&mut self.mod_routes);
        for r in &routes {
            self.unroute(r);
            if let Some(b) = &self.route_board {
                for source in &r.routes {
                    b.set(r.group, r.sound, source.index, source.key, f32::NAN);
                }
            }
        }
        self.mod_routes = routes;
        for (plugin, id, _) in self.modded_plugins.drain(..) {
            if let Ok(mut p) = plugin.try_lock() {
                p.unmodulate(id);
            } else if self.unrouted.len() < self.unrouted.capacity() {
                self.unrouted.push((plugin, id));
            }
        }
    }
}
