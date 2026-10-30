//! The LFOs at play (docs/superpowers/specs/2026-10-28-lfos-design.md): each one's value
//! now, from the song's clock or from its trigger's last note; an envelope
//! (docs/superpowers/specs/2026-10-29-lfo-envelope-design.md) from its trigger's notes
//! held.

use super::{Engine, Mode};
use deckard_model::PPQ;
use deckard_model::lfo::{ENV_MAX, ENV_MIN, EnvState, Lfo};
use deckard_model::mod_matrix::{CC_MAX, CcRoute, ModSource};
use std::sync::Arc;

/// Notes an envelope follows at once (beyond: the oldest is let go).
pub(crate) const ENV_NOTES: usize = 32;

/// An LFO at play; `trig_at`: when its trigger's last note sounded (on the clock's
/// `ever` ticks, and `time` in frames). An envelope's run is `env`; its trigger's notes held are the first `held` of `notes`
/// (pitch, the frame it ends: `u64::MAX` until let go).
pub(crate) struct LfoRt {
    pub lfo: Lfo,
    pub trig_at: Option<(f64, u64)>,
    pub env: EnvState,
    pub notes: [(i64, u64); ENV_NOTES],
    pub held: usize,
}

impl LfoRt {
    fn new(lfo: Lfo) -> LfoRt {
        LfoRt { lfo, trig_at: None, env: EnvState::default(), notes: [(0, 0); ENV_NOTES], held: 0 }
    }

    /// The envelope's notes all ended, the last at `at` (seconds): it releases from there.
    fn env_off(&mut self, at: f64) {
        self.env.off_level = self.lfo.env_at(&self.env, at);
        self.env.off_at = Some(at);
    }

    /// Let go of the notes that ended before frame `now` (`at_now`: or at it).
    fn env_expire(&mut self, now: u64, at_now: bool, sr: f64) {
        let mut last = None;
        let mut i = 0;
        while i < self.held {
            let until = self.notes[i].1;
            if until < now || (at_now && until == now) {
                last = last.max(Some(until));
                self.held -= 1;
                self.notes.copy_within(i + 1..=self.held, i);
            } else {
                i += 1;
            }
        }
        if let (Some(at), 0) = (last, self.held) {
            self.env_off(at as f64 / sr);
        }
    }

    /// A note of its trigger starts at frame `now`, until frame `until`: the attack
    /// restarts (from the level now) if it retriggers, or if no note was held.
    fn env_on(&mut self, pitch: i64, now: u64, until: u64, sr: f64) {
        self.env_expire(now, false, sr);
        if self.lfo.retrigger || self.held == 0 {
            let t = now as f64 / sr;
            self.env = EnvState { on_at: t, from: self.lfo.env_at(&self.env, t), off_at: None, off_level: 0.0 };
        }
        if let Some(i) = self.notes[..self.held].iter().position(|n| n.0 == pitch) {
            self.notes[i].1 = until;
            return;
        }
        if self.held == ENV_NOTES {
            self.notes.copy_within(1.., 0);
            self.held -= 1;
        }
        self.notes[self.held] = (pitch, until);
        self.held += 1;
    }

    /// Its trigger's note `pitch` let go at frame `now` (`None`: all of them).
    fn env_release(&mut self, pitch: Option<i64>, now: u64, sr: f64) {
        // (A note that ended before now goes first: the release never starts in the past.)
        self.env_expire(now, false, sr);
        if self.held == 0 {
            return;
        }
        let before = self.held;
        let mut i = 0;
        while i < self.held {
            if pitch.is_none_or(|p| self.notes[i].0 == p) {
                self.held -= 1;
                self.notes.copy_within(i + 1..=self.held, i);
            } else {
                i += 1;
            }
        }
        if self.held == 0 && before > 0 {
            self.env_off(now as f64 / sr);
        }
    }
}

/// A CC route at play ([`deckard_model::mod_matrix::CcRoute`]): `port` its port's name
/// (shared, so sending allocates nothing), `sent` the value it sent last.
pub(crate) struct CcRt {
    pub route: CcRoute,
    pub port: Arc<str>,
    pub sent: Option<u8>,
    /// Sends nothing before this (`time`, frames): a CC just given (an edit) settles
    /// first, so turning through CCs sends to none of those passed.
    pub settle_until: u64,
}

/// How long a CC route's new place waits before it's sent to (seconds).
const SETTLE: f64 = 0.25;

/// `project`'s CC routes, nothing sent yet.
pub(crate) fn ccs_of(project: &deckard_model::Project) -> Vec<CcRt> {
    project.cc_routes.iter().map(|r| CcRt { route: r.clone(), port: r.port.as_str().into(), sent: None, settle_until: 0 }).collect()
}

/// An envelope's run `st` under `old`'s settings, made `new`'s at `t` (seconds) so it
/// carries on from its level then: rising from it in the attack, falling from it in the
/// decay or the release. Not `held`: it releases from there. (A sustain raised above
/// the level holds at the new sustain.)
fn rebase(old: &Lfo, st: &EnvState, new: &Lfo, held: bool, t: f64) -> EnvState {
    let v = old.env_at(st, t);
    if !held || st.off_at.is_some_and(|off| t >= off) {
        return EnvState { off_at: Some(t), off_level: v, ..*st };
    }
    let rise = f64::from(1.0 - st.from.clamp(0.0, 1.0)) * f64::from(old.attack.clamp(ENV_MIN, ENV_MAX));
    if t - st.on_at < rise {
        return EnvState { on_at: t, from: v, off_at: None, off_level: 0.0 };
    }
    // In the decay (or holding): at the top a moment ago, falling to `v` now.
    let back = f64::from(1.0 - v) * f64::from(new.decay.clamp(ENV_MIN, ENV_MAX));
    EnvState { on_at: t - back, from: 1.0, off_at: None, off_level: 0.0 }
}

/// The LFOs' clock: ticks and seconds counted span by span at the tempo then (a tempo
/// change never moves an LFO). `ticks` and `secs` count from play's start (not during a
/// count-in or a wait for a clock's bar) and run on while stopped; `ever` counts from
/// the engine's start, for triggers.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LfoClock {
    pub ticks: f64,
    pub secs: f64,
    pub ever: f64,
}

/// `project`'s LFOs, none triggered yet.
pub(crate) fn lfos_of(project: &deckard_model::Project) -> Vec<LfoRt> {
    project.lfos.iter().map(|l| LfoRt::new(l.clone())).collect()
}

impl Engine {
    /// The arrangement's ticks (plus the export's offset) while it plays, or waits to.
    fn lfo_song(&self) -> Option<f64> {
        let on = self.sequencing || self.counting > 0 || self.sync.waits();
        (self.mode == Mode::Arrangement && on).then(|| self.position() + self.song_offset)
    }

    /// `l`'s cycles run now. Triggered, from its last note (its wave restarts there);
    /// else on the song's clock: synced, the arrangement's ticks (plus the export's
    /// offset) or the ticks played in Ideas (or since stopping, from there); free, the
    /// seconds played.
    pub(crate) fn lfo_cycles(&self, l: &LfoRt) -> f64 {
        if l.lfo.trigger.is_some() {
            let (t, f) = self.since_trig(l);
            return l.lfo.cycles(t, f / self.sr);
        }
        let ticks = self.lfo_song().unwrap_or(self.lfo_clock.ticks);
        l.lfo.cycles(ticks, self.lfo_clock.secs)
    }

    /// Ticks and frames since `l`'s trigger's last note (0 before one).
    fn since_trig(&self, l: &LfoRt) -> (f64, f64) {
        l.trig_at.map_or((0.0, 0.0), |(t, at)| (self.lfo_clock.ever - t, self.time.saturating_sub(at) as f64))
    }

    /// The LFOs' clock moves on `frames` (after a span); `waited`: the span was a count-in
    /// or a wait for a clock's bar.
    pub(crate) fn lfo_clock_run(&mut self, frames: u64, waited: bool) {
        let ticks = frames as f64 / self.samples_per_tick();
        self.lfo_clock.ever += ticks;
        if let Some(t) = self.lfo_song() {
            self.lfo_clock.ticks = t;
        } else if !waited {
            self.lfo_clock.ticks += ticks;
        }
        if !waited {
            self.lfo_clock.secs += frames as f64 / self.sr;
        }
    }

    /// The song starts playing (a count-in or a wait over): the LFOs' clock restarts.
    pub(crate) fn lfo_clock_start(&mut self) {
        (self.lfo_clock.ticks, self.lfo_clock.secs) = (0.0, 0.0);
    }

    /// LFO `pad`'s value now (0..1); `None` when there's no such LFO, or it's muted, or
    /// it's an envelope with no trigger (its routes leave their targets alone).
    pub fn lfo_value(&self, pad: u8) -> Option<f32> {
        let l = self.lfos.iter().find(|l| l.lfo.pad == pad && !l.lfo.muted)?;
        if l.lfo.env {
            l.lfo.trigger?;
            return Some(l.lfo.env_output(l.lfo.env_at(&l.env, self.time as f64 / self.sr)));
        }
        let since = l.trig_at.map(|_| self.since_trig(l).0 / PPQ as f64);
        Some(l.lfo.output(self.lfo_cycles(l), since))
    }

    /// A note (`pitch`) of sound (`group`, `sound`) sounds now, for `frames` (`None`:
    /// until let go): the LFOs it triggers restart, and the envelopes it plays take it
    /// (muted ones too, so an unmute carries on from the note).
    pub(crate) fn lfo_trigger(&mut self, group: usize, sound: usize, pitch: i64, frames: Option<u64>) {
        let (now, sr, ever) = (self.time, self.sr, self.lfo_clock.ever);
        let until = frames.map_or(u64::MAX, |f| now.saturating_add(f));
        for l in self.lfos.iter_mut().filter(|l| l.lfo.trigger == Some((group, sound))) {
            l.trig_at = Some((ever, now));
            if l.lfo.env {
                l.env_on(pitch, now, until, sr);
            }
        }
    }

    /// Sound (`group`, `sound`)'s note `pitch` ends now (`None`: all its notes, a
    /// choke): the envelopes it plays let go of it.
    pub(crate) fn lfo_release(&mut self, group: usize, sound: usize, pitch: Option<i64>) {
        let (now, sr) = (self.time, self.sr);
        for l in self.lfos.iter_mut().filter(|l| l.lfo.trigger == Some((group, sound))) {
            l.env_release(pitch, now, sr);
        }
    }

    /// Every envelope lets go of its notes (stop): each releases.
    pub(crate) fn lfo_release_all(&mut self) {
        let (now, sr) = (self.time, self.sr);
        for l in self.lfos.iter_mut() {
            l.env_release(None, now, sr);
        }
    }

    /// The envelopes let go of the notes whose lengths have run out (each span, before
    /// the values are read), each releasing from where its last note ended.
    pub(crate) fn lfo_tick(&mut self) {
        let (now, sr) = (self.time, self.sr);
        for l in self.lfos.iter_mut().filter(|l| l.held > 0) {
            l.env_expire(now, true, sr);
        }
    }

    /// Take `fresh`'s LFOs (an edit), each keeping when it was last triggered while its
    /// trigger stays (a fade carries on; another trigger waits for its sound's note). An
    /// envelope keeps its notes held while its trigger stays; with another trigger it
    /// lets go of them and releases. Either way it carries on from its level
    /// now, its new times taking over from there. No allocating.
    pub(crate) fn adopt_lfos(&mut self, fresh: &mut Engine) {
        std::mem::swap(&mut self.lfos, &mut fresh.lfos);
        let t = self.time as f64 / self.sr;
        for l in self.lfos.iter_mut() {
            let Some(o) = fresh.lfos.iter().find(|o| o.lfo.pad == l.lfo.pad) else { continue };
            let same = o.lfo.trigger == l.lfo.trigger;
            if same {
                l.trig_at = o.trig_at;
            }
            if !(l.lfo.env && o.lfo.env) {
                continue;
            }
            if same {
                (l.notes, l.held) = (o.notes, o.held);
            }
            let times = |x: &Lfo| (x.attack, x.decay, x.sustain, x.release);
            l.env = if same && times(&o.lfo) == times(&l.lfo) { o.env } else { rebase(&o.lfo, &o.env, &l.lfo, l.held > 0, t) };
        }
    }

    /// The CC routes send their LFOs' values (each span), each only when it changes; an
    /// LFO that's muted or gone sends nothing; a route at Depth 0 nothing until it has
    /// sent (then it settles once on its constant). No allocating.
    pub(crate) fn send_ccs(&mut self) {
        for i in 0..self.cc_routes.len() {
            let r = &self.cc_routes[i].route;
            let ModSource::Lfo(pad) = r.source else { continue };
            if r.low == r.high && self.cc_routes[i].sent.is_none() {
                continue;
            }
            if self.time < self.cc_routes[i].settle_until {
                continue;
            }
            let Some(x) = self.lfo_value(pad) else { continue };
            let c = &self.cc_routes[i];
            let v = c.route.value(x);
            if c.sent != Some(v) {
                self.send_midi(&c.port, [0xb0 | (c.route.channel.clamp(1, 16) - 1), c.route.cc.min(CC_MAX), v], 3);
                self.cc_routes[i].sent = Some(v);
            }
        }
    }

    /// Take `fresh`'s CC routes (an edit), each knowing what the same CC (port, channel,
    /// number) sent last; one at a new CC (or new) settles first. No allocating.
    pub(crate) fn adopt_ccs(&mut self, fresh: &mut Engine) {
        std::mem::swap(&mut self.cc_routes, &mut fresh.cc_routes);
        let settled = self.time + (SETTLE * self.sr) as u64;
        for c in self.cc_routes.iter_mut() {
            let same = |o: &&CcRt| o.port == c.port && o.route.channel == c.route.channel && o.route.cc == c.route.cc;
            match fresh.cc_routes.iter().find(same) {
                Some(o) => (c.sent, c.settle_until) = (o.sent, o.settle_until),
                None => (c.sent, c.settle_until) = (None, settled),
            }
        }
    }
}
