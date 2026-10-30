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

/// A route of sound (`group`, `sound`) at play: `now`, the knob travel it gives this
/// span.
pub(crate) struct RouteRt {
    pub group: usize,
    pub sound: usize,
    /// Its place in the sound's routes (its knob's, on [`crate::meters::RouteTravels`]).
    pub index: usize,
    /// Its [`crate::meters::route_key`].
    pub key: u32,
    pub route: deckard_model::mod_matrix::ModRoute,
    pub target: RouteTarget,
    pub now: f32,
    /// What was last sent to a plug-in (sent again only when it changes).
    pub sent: f32,
}

impl RouteRt {
    /// The same route (an edit kept it).
    fn same(&self, o: &RouteRt) -> bool {
        (self.group, self.sound, self.route.slot, &self.route.param) == (o.group, o.sound, o.route.slot, &o.route.param)
    }
}

/// The routes of `project`'s sounds whose slot holds the module they name (plug-ins as
/// loaded in `plugins`).
pub(crate) fn routes_of(project: &deckard_model::Project, plugins: &deckard_plugins::Host) -> Vec<RouteRt> {
    use deckard_model::mod_matrix::ModParam;
    use deckard_plugins::Slot;
    let mut out = Vec::new();
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
                out.push(RouteRt { group: g, sound: s, index: k, key, route: r.clone(), target, now: f32::NAN, sent: f32::NAN });
            }
        }
    }
    out
}

impl Engine {
    /// Each route's travel this span, from its sound's sources or its LFO (`NaN`: none).
    fn route_travels(&mut self) {
        for i in 0..self.mod_routes.len() {
            let (g, s) = (self.mod_routes[i].group, self.mod_routes[i].sound);
            let x = match self.mod_routes[i].route.source {
                deckard_model::mod_matrix::ModSource::Pitch => self.sources(g, s)[0],
                deckard_model::mod_matrix::ModSource::Slide => self.sources(g, s)[1],
                deckard_model::mod_matrix::ModSource::Pressure => self.sources(g, s)[2],
                deckard_model::mod_matrix::ModSource::Lfo(n) => match self.lfo_value(n) {
                    Some(v) => Some(v),
                    // No such LFO: the route sits out (its target keeps its own value; a
                    // plug-in it was sent to is let go, or at a later span if busy).
                    None => {
                        let r = &mut self.mod_routes[i];
                        r.now = f32::NAN;
                        if let RouteTarget::Plugin { plugin, id } = &r.target
                            && !r.sent.is_nan()
                            && let Ok(mut p) = plugin.try_lock()
                        {
                            p.unmodulate(*id);
                            r.sent = f32::NAN;
                        }
                        continue;
                    }
                },
            };
            let r = &mut self.mod_routes[i];
            r.now = r.route.travel(x);
        }
    }

    /// The routes' targets take their values (each span, after the modulation lanes, so
    /// a route wins over a lane), and their travels are published. No allocating.
    pub(crate) fn apply_routes(&mut self) {
        if self.mod_routes.is_empty() {
            return;
        }
        self.route_travels();
        if let Some(b) = &self.route_board {
            for r in &self.mod_routes {
                b.set(r.group, r.sound, r.index, r.key, r.now);
            }
        }
        let Engine { mod_routes, fx, sounds, .. } = self;
        for r in mod_routes.iter_mut().filter(|r| !r.now.is_nan()) {
            let t = f64::from(r.now);
            match &r.target {
                RouteTarget::Fx { slot, field, scale } => {
                    let Some(chain) = fx.get_mut(r.group).and_then(|x| x.get_mut(r.sound)).and_then(Option::as_mut) else { continue };
                    for u in chain.units.iter_mut().filter(|u| u.slot == *slot && !u.ghost) {
                        if let super::FxUnit::Internal { params, modded, hit, .. } = &mut u.fx {
                            if !*hit {
                                modded.clone_from(params);
                                *hit = true;
                            }
                            if let Some(x) = modded.iter_mut().find(|(k, _)| k == field) {
                                x.1 = scale.at(t);
                            }
                        }
                    }
                }
                RouteTarget::Sound { field, scale, .. } => {
                    if let Some(Some(snd)) = sounds.get_mut(r.group).and_then(|x| x.get_mut(r.sound)) {
                        snd.modulate(field, scale.at(t));
                    }
                }
                RouteTarget::Plugin { plugin, id } => {
                    if r.sent != r.now
                        && let Ok(mut p) = plugin.try_lock()
                    {
                        p.modulate(*id, t);
                        r.sent = r.now;
                    }
                }
            }
        }
    }

    /// Route `r` is gone: its target has its own value again (an effect's comes back
    /// by itself, when no lane or route holds it).
    fn unroute(&mut self, r: &RouteRt) {
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
        // Knobs whose route went no longer move (kept ones are published next span).
        // Both lists are in (group, sound, place) order: one walk.
        if let Some(b) = &self.route_board {
            let at = |r: &RouteRt| (r.group, r.sound, r.index);
            let mut j = 0;
            for o in fresh.mod_routes.iter() {
                while j < self.mod_routes.len() && at(&self.mod_routes[j]) < at(o) {
                    j += 1;
                }
                if self.mod_routes.get(j).is_none_or(|n| at(n) != at(o)) {
                    b.set(o.group, o.sound, o.index, o.key, f32::NAN);
                }
            }
        }
        // A kept route on the same plug-in knows what it holds there (so it can let go).
        for n in self.mod_routes.iter_mut() {
            if let RouteTarget::Plugin { plugin, .. } = &n.target
                && let Some(o) = fresh
                    .mod_routes
                    .iter()
                    .find(|o| o.same(n) && matches!(&o.target, RouteTarget::Plugin { plugin: q, .. } if std::sync::Arc::ptr_eq(q, plugin)))
            {
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
                b.set(r.group, r.sound, r.index, r.key, f32::NAN);
            }
        }
        self.mod_routes = routes;
    }
}
