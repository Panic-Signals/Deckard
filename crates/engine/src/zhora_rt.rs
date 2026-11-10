//! Zhora at play (docs/superpowers/specs/2026-11-09-cv-outs-and-zhora-design.md, Part 2):
//! its tuner hears its input pair, its sound plays the modular's return, and its signals
//! go out on the CV only channels.

use super::{Engine, JOB_FRAMES, SampleCache, SoundBufs};
use deckard_model::{AudioSource, Project, Sound};
use deckard_plugins::{Shared, Slot};
use std::collections::VecDeque;

/// Zhora's signals: pitch, gate, clock, envelope, velocity, mod wheel.
const SIGNALS: usize = 6;
/// The ids of Zhora's output parameters, in signal order, and their steps (saved: never
/// renumbered).
const OUT_IDS: [u32; SIGNALS] = [1, 2, 3, 4, 5, 6];
const OUT_STEPS: f64 = 32.0;
/// The id of Zhora's input parameter, and its steps.
const INPUT_ID: u32 = 30;
const INPUT_STEPS: f64 = 16.0;

/// A Zhora at play: its sound, its instance, and whether the sound is muted (its return
/// is silent; its tuner still hears). `pair` and `outs` are what it had last, for a span
/// its instance is busy.
pub(crate) struct ZhoraRt {
    pub g: usize,
    pub s: usize,
    pub plugin: Shared,
    pub silent: bool,
    pair: Option<usize>,
    outs: [Option<u8>; SIGNALS],
}

/// One of a Zhora's signals on a channel, for the screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZhoraOut {
    pub g: usize,
    pub s: usize,
    pub signal: usize,
    pub channel: u8,
}

/// Whether sound `snd` is played by Zhora.
pub(crate) fn is_zhora(snd: &Sound) -> bool {
    snd.plugin.as_ref().is_some_and(|p| deckard_plugins::zhora::is_zhora(&p.id))
}

/// Whether `p` wants the interface's input: a sound takes it, or a Zhora listens.
pub fn project_takes_input(p: &Project) -> bool {
    p.groups.iter().flat_map(|g| &g.sounds).any(|s| matches!(s.channel.audio_in, AudioSource::Ext(_)) || is_zhora(s))
}

/// The saved parameter values (id, normalized) in a native plug-in's chunk; none when it
/// can't be read.
fn saved_params(chunk: &[u8]) -> Vec<(u32, f64)> {
    let Ok(state) = deckard_plugins::state::unwrap_chunk(chunk, deckard_plugins::state::Format::Vst3) else { return Vec::new() };
    let d = &state.component;
    if d.get(..4) != Some(b"DKRD".as_slice()) {
        return Vec::new();
    }
    let u32_at = |at: usize| d.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let n = u32_at(8).unwrap_or(0) as usize;
    (0..n)
        .map_while(|i| {
            let at = 12 + i * 12;
            Some((u32_at(at)?, f64::from_le_bytes(d.get(at + 4..at + 12)?.try_into().ok()?)))
        })
        .collect()
}

/// Parameter `id`'s saved value (0 when it isn't saved).
fn saved(params: &[(u32, f64)], id: u32) -> f64 {
    params.iter().find(|p| p.0 == id).map_or(0.0, |p| p.1).clamp(0.0, 1.0)
}

/// A picker's choice (0: None) as 0-based, as the instrument reads it.
fn picked(v: f64, steps: f64) -> Option<usize> {
    ((v * steps).round() as usize).checked_sub(1)
}

/// Each Zhora's outputs as saved, in the order they own channels: Zhoras in project order,
/// then signals in order. Outputs set to None are left out.
pub fn zhora_outs(p: &Project) -> Vec<ZhoraOut> {
    let mut out = Vec::new();
    for (g, grp) in p.groups.iter().enumerate() {
        for (s, snd) in grp.sounds.iter().enumerate() {
            let Some(pr) = snd.plugin.as_ref().filter(|_| is_zhora(snd)) else { continue };
            let params = saved_params(&pr.chunk);
            for (signal, &id) in OUT_IDS.iter().enumerate() {
                if let Some(ch) = picked(saved(&params, id), OUT_STEPS) {
                    out.push(ZhoraOut { g, s, signal, channel: ch as u8 });
                }
            }
        }
    }
    out
}

/// `project`'s Zhoras, with the instances `cache` has for them (sounds whose plug-in
/// didn't load are left out).
pub(crate) fn zhoras_of(project: &Project, cache: &SampleCache) -> Vec<ZhoraRt> {
    let mut out = Vec::new();
    for (g, grp) in project.groups.iter().enumerate() {
        for (s, snd) in grp.sounds.iter().enumerate() {
            let Some(pr) = snd.plugin.as_ref().filter(|_| is_zhora(snd)) else { continue };
            let Some(plugin) = cache.loaded_plugin(Slot::Sound { group: g, sound: s }, &pr.id, &pr.name) else { continue };
            let params = saved_params(&pr.chunk);
            let outs = OUT_IDS.map(|id| picked(saved(&params, id), OUT_STEPS).and_then(|c| u8::try_from(c).ok()));
            let pair = picked(saved(&params, INPUT_ID), INPUT_STEPS);
            out.push(ZhoraRt { g, s, plugin, silent: super::muting(project, g, s).muted, pair, outs });
        }
    }
    out
}

/// Each Zhora hears the first `frames` frames of `q` (its input pair, before mute); an
/// unmuted one's sound takes them too, as its return. No allocating.
pub(crate) fn hear(
    zhoras: &mut [ZhoraRt],
    bufs: &mut [Vec<Option<Box<SoundBufs>>>],
    scratch: &mut (Vec<f32>, Vec<f32>),
    q: &VecDeque<[f32; 32]>,
    frames: usize,
) {
    let n = q.len().min(frames).min(JOB_FRAMES);
    for z in zhoras.iter_mut() {
        // Busy: it misses this block, and its return plays the pair it had.
        let mut p = z.plugin.try_lock().ok();
        if let Some(p) = &p {
            z.pair = p.input_pair();
        }
        let Some((l, r)) = z.pair.map(|k| (2 * k, 2 * k + 1)).filter(|&(_, r)| r < 32) else { continue };
        for (f, frame) in q.iter().take(n).enumerate() {
            (scratch.0[f], scratch.1[f]) = (frame[l], frame[r]);
        }
        if let Some(p) = p.as_mut() {
            p.input(&scratch.0[..n], &scratch.1[..n]);
        }
        if z.silent {
            continue;
        }
        let Some(b) = bufs.get_mut(z.g).and_then(|row| row.get_mut(z.s)).and_then(Option::as_mut) else { continue };
        b.inp.clear();
        b.inp.resize(frames * 2, 0.0);
        for f in 0..n {
            (b.inp[f * 2], b.inp[f * 2 + 1]) = (scratch.0[f], scratch.1[f]);
        }
        b.inp_on = true;
    }
}

impl Engine {
    /// The Zhoras' signals onto the CV only channels they own, `frames` from `at`
    /// (interleaved), straight from Zhora (no glide); before [`Engine::write_cv`], which
    /// leaves those channels alone. A CV route's channel isn't theirs; between Zhoras the
    /// first in the project wins. No allocating.
    pub fn write_zhora_cv(&mut self, ext: &mut [Vec<f32>], at: usize, frames: usize) {
        self.zhora_cv_owned = 0;
        let mask = self.cv_channels;
        if mask == 0 {
            return;
        }
        let bit = |ch: u8| 1u32.checked_shl(u32::from(ch)).unwrap_or(0);
        let mut taken = self.cv_routes.iter().fold(0, |a, r| a | bit(r.channel)) & mask;
        let mut owned = 0;
        for z in self.zhoras.iter_mut() {
            // Busy: its channels hold their levels this span, still its.
            let p = z.plugin.try_lock().ok();
            for signal in 0..SIGNALS {
                let cv = p.as_ref().map(|p| p.cv(signal, frames));
                if let Some(cv) = &cv {
                    z.outs[signal] = cv.and_then(|(ch, _)| u8::try_from(ch).ok());
                }
                let Some(ch) = z.outs[signal] else { continue };
                let b = bit(ch);
                if mask & b == 0 || taken & b != 0 {
                    continue;
                }
                (taken, owned) = (taken | b, owned | b);
                let values = cv.flatten().map_or(&[][..], |(_, v)| v);
                let mut buf = ext.get_mut(ch as usize / 2);
                let side = ch as usize % 2;
                let last = &mut self.cv_last[ch as usize];
                for n in 0..frames {
                    if let Some(&v) = values.get(n)
                        && v.is_finite()
                    {
                        *last = v.clamp(-1.0, 1.0);
                    }
                    if let Some(s) = buf.as_mut().and_then(|b| b.get_mut(at + n * 2 + side)) {
                        *s = *last;
                    }
                }
            }
        }
        self.zhora_cv_owned = owned;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{project, split_plugin};
    use crate::{HELD, Live, LiveIn, Mode};
    use deckard_model::mod_matrix::{CvRoute, ModSource};
    use deckard_model::{PluginRef, SlotInfo};
    use deckard_plugins::Transport;
    use deckard_plugins::zhora::table::Table;
    use std::cell::RefCell;

    const SR: u32 = 1000;
    const PITCH: u32 = 1;
    const GATE: u32 = 2;
    const CLOCK: u32 = 3;
    const VEL: u32 = 5;
    const MOD: u32 = 6;

    /// An output parameter's value for channel `ch`.
    fn on(ch: u8) -> f64 {
        (ch as f64 + 1.0) / 32.0
    }

    /// Zhora, saved with `params` set.
    fn zhora_ref(params: &[(u32, f64)]) -> PluginRef {
        let class = &deckard_plugins::zhora::CLASS;
        let id = deckard_plugins::state::PluginId::parse(&class.plugin_id(), class.name).unwrap();
        let mut p = deckard_plugins::Plugin::native(class, id, class.name, SR as f64);
        for &(k, v) in params {
            p.set_param(k, v);
        }
        PluginRef { id: class.plugin_id(), name: class.name.into(), chunk: p.save_chunk(), ..Default::default() }
    }

    /// Sound `s` of `p` played by Zhora (`params`).
    fn make_zhora(p: &mut Project, s: usize, params: &[(u32, f64)]) {
        let pr = zhora_ref(params);
        let snd = &mut p.groups[0].sounds[s];
        snd.sampler = None;
        snd.plugin = Some(pr.clone());
        snd.slots = vec![SlotInfo { module: "PluginHost".into(), name: pr.name.clone(), active: true, plugin: Some(pr), params: vec![] }];
    }

    /// The test project, its one sound played by Zhora (`params`), with no notes.
    fn zhora_project(params: &[(u32, f64)]) -> Project {
        let mut p = project();
        p.groups[0].patterns[0].notes.clear();
        make_zhora(&mut p, 0, params);
        p
    }

    /// An engine for `p`, stopped, with CV only channels `mask`.
    fn engine(p: &Project, cache: &mut SampleCache, mask: u32) -> Engine {
        let (mut e, _) = Engine::new(p, None, SR, Mode::Arrangement, cache);
        e.sequencing = false;
        e.set_cv_channels(mask);
        e.cv_smooth_left = 0;
        e
    }

    /// `frames` more in one call, on 16 outputs full of `fill`: the main output and those.
    fn run(e: &mut Engine, frames: usize, fill: f32) -> (Vec<f32>, Vec<Vec<f32>>) {
        let mut out = vec![0f32; frames * 2];
        let mut ext = vec![vec![fill; frames * 2]; 16];
        e.process_outputs(&mut out, &mut ext);
        (out, ext)
    }

    /// Channel `ch`'s frames.
    fn channel(ext: &[Vec<f32>], ch: usize) -> Vec<f32> {
        ext[ch / 2].iter().skip(ch % 2).step_by(2).copied().collect()
    }

    fn note(e: &mut Engine, s: usize, pitch: i64) {
        e.schedule(0, Live::NoteOn { group: 0, sound: s, pitch, velocity: 100, length: HELD });
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    thread_local! {
        /// What the [`Ear`]s on this thread heard.
        static HEARD: RefCell<Vec<(f32, f32)>> = const { RefCell::new(Vec::new()) };
    }

    /// A stand-in for Zhora that notes what it hears on In 2.
    struct Ear;

    impl deckard_plugins::native::Engine for Ear {
        fn set_param(&mut self, _: u32, _: f64) {}
        fn note_on(&mut self, _: u8, _: u8) {}
        fn note_off(&mut self, _: u8) {}
        fn reset(&mut self) {}
        fn process(&mut self, _: &mut [f32], _: &mut [f32], _: &Transport) {}
        fn input(&mut self, left: &[f32], right: &[f32]) {
            HEARD.with(|h| h.borrow_mut().extend(left.iter().copied().zip(right.iter().copied())));
        }
        fn input_pair(&self) -> Option<usize> {
            Some(1)
        }
    }

    static EAR: deckard_plugins::native::Class = deckard_plugins::native::Class {
        cid: *b"DECKARD:TestEar0",
        name: "Ear",
        instrument: true,
        params: &[],
        pages: &[],
        make: |_| Box::new(Ear),
    };

    /// An engine for `p` (its sound 0 a Zhora) with an [`Ear`] in Zhora's place, stopped,
    /// and 64 frames of (0.5, -0.25) queued on In 2.
    fn ear_engine(p: &Project) -> Engine {
        HEARD.with(|h| h.borrow_mut().clear());
        let mut cache = SampleCache::default();
        cache.test_plugins.push((Slot::Sound { group: 0, sound: 0 }, split_plugin(&EAR)));
        let mut e = engine(p, &mut cache, 0);
        let live: LiveIn = Default::default();
        let mut frame = [0f32; 32];
        (frame[2], frame[3]) = (0.5, -0.25);
        live.lock().unwrap().extend(std::iter::repeat_n(frame, 64));
        e.live_in = Some(live);
        e
    }

    fn heard() -> Vec<(f32, f32)> {
        HEARD.with(|h| h.borrow().clone())
    }

    #[test]
    fn a_zhora_sound_is_never_rendered_ahead() {
        let mut cache = SampleCache::default();
        let e = engine(&zhora_project(&[]), &mut cache, 0);
        assert!(e.live_hint[0][0]);
        let e = engine(&project(), &mut cache, 0);
        assert!(!e.live_hint[0][0], "a Sampler sound may be");
    }

    #[test]
    fn the_tuner_hears_the_input_when_the_sound_is_muted() {
        let mut p = zhora_project(&[]);
        p.groups[0].sounds[0].muted = true;
        let mut e = ear_engine(&p);
        assert!(e.takes_input());
        run(&mut e, 64, 0.0);
        assert_eq!(heard(), vec![(0.5, -0.25); 64]);
        let mut p = zhora_project(&[]);
        p.groups[0].muted = true;
        let mut e = ear_engine(&p);
        run(&mut e, 64, 0.0);
        assert_eq!(heard(), vec![(0.5, -0.25); 64], "in a muted group");
    }

    #[test]
    fn a_zhora_sound_plays_its_return_through_its_channel() {
        let p = zhora_project(&[]);
        let mut e = ear_engine(&p);
        let (out, _) = run(&mut e, 64, 0.0);
        assert!(out.chunks(2).all(|f| near(f[0], 0.5) && near(f[1], -0.25)), "{:?}", &out[..8]);
        assert_eq!(heard().len(), 64);
        // At the sound's level.
        let mut p = zhora_project(&[]);
        p.groups[0].sounds[0].level = 0.5;
        let mut e = ear_engine(&p);
        let (out, _) = run(&mut e, 64, 0.0);
        assert!(out[126] > 0.01 && out[126] < 0.45, "{}", out[126]);
    }

    #[test]
    fn a_muted_zhora_has_a_silent_return() {
        for group in [false, true] {
            let mut p = zhora_project(&[]);
            if group {
                p.groups[0].muted = true;
            } else {
                p.groups[0].sounds[0].muted = true;
            }
            let mut e = ear_engine(&p);
            let (out, _) = run(&mut e, 64, 0.0);
            assert!(out.iter().all(|&v| v == 0.0), "group muted: {group}");
        }
    }

    #[test]
    fn muting_mid_note_drops_the_gate() {
        let p = zhora_project(&[(GATE, on(2))]);
        let mut cache = SampleCache::default();
        let mut e = engine(&p, &mut cache, 0b100);
        note(&mut e, 0, 60);
        let (_, ext) = run(&mut e, 20, 0.0);
        assert!(near(channel(&ext, 2)[19], 0.5), "{:?}", channel(&ext, 2));
        let mut q = p.clone();
        q.groups[0].sounds[0].muted = true;
        let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, SR, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &q));
        let (_, ext) = run(&mut e, 20, 0.0);
        assert!(channel(&ext, 2).iter().all(|&v| v == 0.0), "{:?}", channel(&ext, 2));
    }

    #[test]
    fn zhora_cv_lands_only_on_marked_channels() {
        let p = zhora_project(&[(PITCH, on(2)), (GATE, on(3))]);
        let mut cache = SampleCache::default();
        let mut e = engine(&p, &mut cache, 0b100);
        note(&mut e, 0, 72);
        let (out, ext) = run(&mut e, 64, 0.9);
        assert!(channel(&ext, 2).iter().all(|&v| near(v, Table::nominal(72.0))), "{:?}", channel(&ext, 2));
        for ch in (0..32).filter(|&c| c != 2) {
            assert!(channel(&ext, ch).iter().all(|&v| v == 0.9), "channel {ch}");
        }
        assert!(out.iter().all(|&v| v == 0.0), "no CV in the mix");
    }

    /// A signal that moves within a span (the clock, playing) follows it frame by frame
    /// past the instrument's block.
    #[test]
    fn zhora_cv_follows_every_frame_of_a_long_span() {
        let p = zhora_project(&[(CLOCK, on(2))]);
        let mut cache = SampleCache::default();
        let mut e = engine(&p, &mut cache, 0b100);
        e.sequencing = true;
        let (_, ext) = run(&mut e, 2048, 0.0);
        let v = channel(&ext, 2);
        // 1/16 at 120 BPM: a 5 ms pulse every 125 frames.
        let rises: Vec<usize> = (1..v.len()).filter(|&n| v[n] > 0.0 && v[n - 1] == 0.0).collect();
        assert_eq!(rises.len(), 16, "{rises:?}");
        assert!(rises.iter().zip(1..).all(|(&n, k)| n.abs_diff(k * 125) <= 1), "{rises:?}");
        assert!(near(v[0], 0.5) && v[1024] == 0.0);
    }

    #[test]
    fn a_zhora_signal_loses_a_channel_a_route_owns() {
        let mut p = zhora_project(&[(PITCH, on(2)), (GATE, on(3))]);
        let second = p.groups[0].sounds[0].clone();
        p.groups[0].sounds.push(second);
        make_zhora(&mut p, 1, &[(GATE, on(3)), (VEL, on(4))]);
        p.cv_routes = vec![CvRoute { low: 0.25, high: 0.25, ..CvRoute::new(ModSource::Lfo(1), 2) }];
        let mut cache = SampleCache::default();
        let mut e = engine(&p, &mut cache, 0b11100);
        note(&mut e, 1, 72);
        let (_, ext) = run(&mut e, 64, 0.9);
        assert!(channel(&ext, 2).iter().all(|&v| v == 0.25), "the route's: {:?}", channel(&ext, 2));
        assert!(channel(&ext, 3).iter().all(|&v| v == 0.0), "the first Zhora's gate, not the second's");
        assert!(channel(&ext, 4).iter().all(|&v| near(v, 0.5 * 100.0 / 127.0)), "the second's velocity");
    }

    #[test]
    fn zhora_outs_lists_each_output_in_owner_order() {
        let mut p = zhora_project(&[(PITCH, on(5)), (MOD, on(0))]);
        let second = p.groups[0].sounds[0].clone();
        p.groups[0].sounds.push(second);
        make_zhora(&mut p, 1, &[(GATE, on(31))]);
        let o = |s, signal, channel| ZhoraOut { g: 0, s, signal, channel };
        assert_eq!(zhora_outs(&p), vec![o(0, 0, 5), o(0, 5, 0), o(1, 1, 31)]);
        assert!(zhora_outs(&project()).is_empty());
    }

    #[test]
    fn export_writes_no_zhora_cv() {
        let p = zhora_project(&[(PITCH, on(2)), (GATE, on(3))]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, SR, Mode::Arrangement, &mut cache);
        note(&mut e, 0, 72);
        let (out, ext) = run(&mut e, 64, 0.25);
        assert!(ext.iter().flatten().all(|&v| v == 0.25));
        assert!(out.iter().all(|&v| v == 0.0), "its instrument is silent, and it has no input");
        assert!(e.instruments[0][0].is_some(), "it rendered its instrument");
    }

    #[test]
    fn projects_take_input_for_a_zhora() {
        assert!(!project_takes_input(&project()));
        assert!(project_takes_input(&zhora_project(&[])));
        let mut p = project();
        p.groups[0].sounds[0].channel.audio_in = AudioSource::Ext(1);
        assert!(project_takes_input(&p));
    }

    #[cfg(feature = "output")]
    #[test]
    fn pitch_holds_its_value_across_play() {
        use crate::output::{Command, apply};
        use std::time::{Duration, Instant};
        let p = zhora_project(&[(PITCH, on(2)), (VEL, on(3)), (MOD, on(4)), (GATE, on(5))]);
        let mask = 0b11_1100;
        let mut cache = SampleCache::default();
        let mut old = engine(&p, &mut cache, mask);
        note(&mut old, 0, 72);
        old.instruments[0][0].as_ref().unwrap().plugin.lock().unwrap().control_change(1, 127);
        let (_, ext) = run(&mut old, 200, 0.0);
        let before: Vec<f32> = (2..6).map(|ch| channel(&ext, ch)[199]).collect();
        assert!(near(before[0], Table::nominal(72.0)) && before[1] > 0.1 && near(before[2], 0.5) && near(before[3], 0.5), "{before:?}");
        let new = Box::new(engine(&p, &mut cache, 0));
        let mut playing = Some(Box::new(old));
        apply(&mut playing, Command::Play(new), Instant::now(), Duration::from_millis(10), SR as f64, 1, false);
        let e = playing.as_mut().unwrap();
        e.set_cv_channels(mask);
        let (_, ext) = run(e, 10, 0.0);
        for (i, ch) in (2..5).enumerate() {
            assert!(channel(&ext, ch).iter().all(|&v| v == before[i]), "channel {ch}: {:?}", channel(&ext, ch));
        }
        assert!(channel(&ext, 5).iter().all(|&v| v == 0.0), "the gate dropped");
    }
}
