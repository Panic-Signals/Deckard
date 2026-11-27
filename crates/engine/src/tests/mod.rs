include!("../additive_route_tests.rs");
use super::*;
use deckard_model::{Group, Note, Pattern, Sampler, Scene, Section, Sound, Zone};

mod audio_loops;
mod aux_sends;
mod cv;
mod effects;
mod expression;
mod extra_outs;
mod midi_input_plugins;
mod mixer;
mod modulation;
mod playback;
mod sampler_playing;
mod scenes_and_loops;

pub(crate) fn project() -> Project {
    let notes = (0..4)
        .map(|i| Note {
            ratchet: Default::default(),
            expr: None,
            sound: 0,
            position: i * PPQ,
            length: 240,
            pitch: 60,
            velocity: 127,
            probability: 100,
        })
        .collect();
    Project {
        tempo: 120.0,
        numerator: 4,
        denominator: 4,
        groups: vec![Group {
            level: 0.75,
            sounds: vec![Sound {
                level: 0.75,
                sampler: Some(Sampler {
                    zones: vec![Zone { root_key: 60, key_range: (0, 127), velocity_range: (0, 127), gain: 1.0, ..Default::default() }],
                    velocity_to_volume: 0.0,
                    ..Default::default()
                }),
                ..Default::default()
            }],
            patterns: vec![Pattern { length: 4 * PPQ, notes, ..Default::default() }],
            ..Default::default()
        }],
        scenes: vec![Scene { patterns: vec![(0, 0)], ..Default::default() }],
        sections: vec![Section { scene: Some(0), length: 8 * PPQ, auto: false }],
        ..Default::default()
    }
}

/// An engine whose one zone is a click: frame 0 = 1.0, then silence.
fn click_engine(sr: u32) -> Engine {
    click_engine_for(&project(), sr)
}

pub(crate) fn click_engine_for(p: &Project, sr: u32) -> Engine {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(p, None, sr, Mode::Arrangement, &mut cache);
    let mut frames = vec![[0f32; 2]; 100];
    frames[0] = [1.0, 1.0];
    set_sample(&mut e, frames, &[], false);
    e
}

/// Give sound 0 of group 0 a Sampler playing `frames` (one zone over all keys, at
/// unity gain), with Sampler parameters `params`.
fn set_sample(e: &mut Engine, frames: Vec<[f32; 2]>, params: &[(&str, f64)], looped: bool) {
    let sample = Arc::new(Sample { rate: e.sr as u32, frames });
    let len = sample.frames.len();
    let mut model = Sampler::default();
    for (k, v) in params {
        model.set(k, *v);
    }
    let zone = sampler::ZoneRt {
        sample: sample.clone(),
        root: 60,
        keys: (0, 127),
        vels: (0, 127),
        gain: 0.75,
        pan: 0.0,
        tune: 0.0,
        play: (0, len),
        looped: looped.then_some((0, len)),
        x_fade: 0,
        fade_in: 0,
        fade_out: 0,
        sync: None,
        lanes: None,
        reverse: false,
        once: false,
        tail_ms: 0.0,
        layer: None,
    };
    e.samples = vec![sample];
    e.sounds[0][0] = Some(SamplerRt::new(sampler::Settings::new(&model, e.sr), vec![zone.clone()], (1.0, 1.0), e.sr));
    // Other sounds of the group with a Sampler play it too.
    for k in 1..e.sounds[0].len() {
        if e.sounds[0][k].is_some() {
            e.sounds[0][k] = Some(SamplerRt::new(sampler::Settings::new(&model, e.sr), vec![zone.clone()], (1.0, 1.0), e.sr));
        }
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// A noise sample in a WAV file in `dir`, for projects that load their samples as
/// the app does.
fn noise_wav(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("noise.wav");
    let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(&path, spec).unwrap();
    for i in 0..48_000u32 {
        let x = (i.wrapping_mul(2_654_435_761) >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
        w.write_sample(x).unwrap();
        w.write_sample(-x * 0.7).unwrap();
    }
    w.finalize().unwrap();
    path
}

/// Eight Sampler sounds playing a noise sample (half through effects with state),
/// notes at different times with probabilities, looping. With the folder holding
/// the sample (removed when dropped).
fn eight_project() -> (tempfile::TempDir, Project) {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = project();
    let mut base = p.groups[0].sounds[0].clone();
    if let Some(smp) = base.sampler.as_mut() {
        smp.zones[0].sample =
            deckard_model::SampleRef { kind: 0, path: noise_wav(tmp.path()).to_string_lossy().into(), ..Default::default() };
        smp.zones[0].play_range = (0, 47_999);
    }
    let fx = |module: &str, params: Vec<(String, f64)>| deckard_model::SlotInfo {
        module: module.into(),
        name: module.into(),
        active: true,
        plugin: None,
        params,
    };
    let defaults = |class: &str| -> Vec<(String, f64)> {
        let (_, d) = deckard_model::INTERNAL_EFFECTS.iter().find(|(c, _)| *c == class).unwrap();
        d.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    };
    let chains = [
        vec![fx("BeatDelay", defaults("BeatDelay"))],
        vec![fx("Chorus", defaults("Chorus"))],
        vec![fx("Saturator", defaults("Saturator"))],
    ];
    p.groups[0].sounds = (0..8)
        .map(|k| {
            let mut s = base.clone();
            s.slots = vec![fx("Sampler", Vec::new())];
            if k % 2 == 1 {
                s.slots.extend(chains[k / 2 % chains.len()].clone());
            }
            s
        })
        .collect();
    p.groups[0].patterns[0].notes = (0..48)
        .map(|i| Note {
            ratchet: Default::default(),
            expr: None,
            sound: i % 8,
            position: i as i64 * 79,
            length: 300,
            pitch: 48 + (i as i64 * 5) % 24,
            velocity: 100,
            probability: 60 + (i as i64 * 13) % 41,
        })
        .collect();
    (tmp, p)
}

/// Render `p` in blocks of `block` for `seconds`; `ahead`: render sounds ahead
/// (`lookahead` frames); `each(block index, engine)` runs before each block.
fn render_with(
    p: &Project,
    ahead: Option<u64>,
    block: usize,
    seconds: f64,
    mut each: impl FnMut(usize, &mut Engine, &mut SampleCache),
) -> Vec<f32> {
    let mut cache = SampleCache::default();
    let (mut e, report) = Engine::new(p, None, 48_000, Mode::Arrangement, &mut cache);
    assert!(report.missing.is_empty() && report.failed.is_empty(), "{report:?}");
    e.looping = true;
    if let Some(l) = ahead {
        e.render_ahead(l, 2, block as u32, true);
    }
    let mut out = Vec::new();
    let mut buf = vec![0f32; block * 2];
    for k in 0..((48_000.0 * seconds) as usize / block) {
        each(k, &mut e, &mut cache);
        e.process(&mut buf);
        out.extend_from_slice(&buf);
    }
    if ahead.is_some() {
        assert_eq!(e.ahead_underruns(), 0, "players kept up (offline they're waited for)");
    }
    out
}

/// A loop in a WAV in `dir`: `beats` at `bpm` (48 kHz), a quiet tone, loud for its
/// first 50 ms (its first beat's mark).
fn loop_wav(dir: &std::path::Path, bpm: f64, beats: f64) -> std::path::PathBuf {
    let path = dir.join(format!("loop-{bpm}.wav"));
    let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(&path, spec).unwrap();
    let frames = (48_000.0 * 60.0 / bpm * beats) as usize;
    for i in 0..frames {
        let t = i as f32 / 48_000.0;
        let v =
            (t * 440.0 * std::f32::consts::TAU).sin() * 0.1 + if i < 2400 { (t * 2000.0 * std::f32::consts::TAU).sin() * 0.8 } else { 0.0 };
        w.write_sample(v).unwrap();
        w.write_sample(v).unwrap();
    }
    w.finalize().unwrap();
    path
}

/// A 120 BPM song whose sound 0 is a New Sampler playing a 4-beat 94 BPM loop in
/// `mode` (its zone told so, as analysis does). With the folder holding the loop.
fn new_sampler_project(mode: deckard_model::new_sampler::PlayMode) -> (tempfile::TempDir, Project, usize) {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = project();
    p.tempo = 120.0;
    let path: String = loop_wav(tmp.path(), 94.0, 4.0).to_string_lossy().into();
    let frames = (48_000.0 * 60.0 / 94.0 * 4.0) as usize;
    let snd = &mut p.groups[0].sounds[0];
    snd.new_sampler = Some(Default::default());
    let z = &mut snd.sampler.as_mut().unwrap().zones[0];
    z.sample = deckard_model::SampleRef { kind: 0, path: path.clone(), ..Default::default() };
    z.play_range = (0, frames as i64 - 1);
    z.meta = Some(deckard_model::new_sampler::ZoneMeta { file: path, mode, bpm: Some(94.0), beats: Some(4.0), ..Default::default() });
    (tmp, p, frames)
}

/// Where the loop's mark sounds: frames where the level (256-frame windows) rises past
/// half its loudest.
fn marks(out: &[f32]) -> Vec<usize> {
    let lv: Vec<f32> = out.chunks(512).map(|c| c.iter().map(|v| v * v).sum::<f32>()).collect();
    let top = lv.iter().cloned().fold(0.0, f32::max);
    (1..lv.len()).filter(|&i| lv[i] > top * 0.5 && lv[i - 1] <= top * 0.5).map(|i| i * 256).collect()
}

fn render_eight(ahead: Option<u64>, block: usize, seconds: f64) -> Vec<f32> {
    let (_tmp, p) = eight_project();
    render_with(&p, ahead, block, seconds, |k, e, _| {
        if k == 0 && ahead.is_some() {
            assert_eq!(e.sounds_ahead(), 8, "every sound rendered ahead");
        }
    })
}

/// A driven Saturator, as a group's or the master's effect.
fn driven_saturator() -> deckard_model::SlotInfo {
    let (_, d) = deckard_model::INTERNAL_EFFECTS.iter().find(|(c, _)| *c == "Saturator").unwrap();
    let params = d.iter().map(|(k, v)| (k.to_string(), if k.contains("drive") { 1.0 } else { *v })).collect();
    deckard_model::SlotInfo { module: "Saturator".into(), name: "Saturator".into(), active: true, plugin: None, params }
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()))
}

/// `eight_project` with sound 1's BeatDelay (slot 1) feedback modulated in its pattern: down
/// half its range at the start, up half at the middle (a scale of 0..1).
fn modulated_project() -> (tempfile::TempDir, Project, f64) {
    crate::modscale::set_scales([(("BeatDelay".to_string(), "feedback".to_string()), crate::modscale::Scale::Lin(0.0, 1.0))]);
    let (tmp, mut p) = eight_project();
    let base = p.groups[0].sounds[1].slots[1].params.iter().find(|(k, _)| k == "feedback").unwrap().1;
    let len = p.groups[0].patterns[0].length.max(p.bar_length());
    let point = |position, value| deckard_model::ModPoint { sound: Some(1), slot: Some(1), field: "feedback".into(), position, value };
    p.groups[0].patterns[0].mods = vec![point(0, -0.5), point(len / 2, 0.5)];
    (tmp, p, base)
}

/// The feedback sound 1's delay has now.
fn feedback_now(e: &Engine) -> f64 {
    let chain = e.fx[0][1].as_ref().unwrap();
    match &chain.units.iter().find(|u| u.slot == 1).unwrap().fx {
        FxUnit::Internal { glide, .. } => glide.now().iter().find(|(k, _)| k == "feedback").unwrap().1,
        _ => unreachable!(),
    }
}

fn route(
    source: deckard_model::mod_matrix::ModSource,
    slot: usize,
    module: &str,
    param: deckard_model::mod_matrix::ModParam,
) -> deckard_model::mod_matrix::ModRoute {
    deckard_model::mod_matrix::ModRoute::new(source, slot, module, param)
}

fn field(f: &str) -> deckard_model::mod_matrix::ModParam {
    deckard_model::mod_matrix::ModParam::Field(f.into())
}

/// Sound 1 of group 0 held and pressed at `pressure`, then `blocks` blocks played.
fn press(e: &mut Engine, pressure: f32, blocks: usize) {
    e.note_hold(0, 1, 60, 100);
    e.play_live(Live::Expression { group: 0, sound: 1, pitch: 60, kind: ExprKind::Dim(Dim::Pressure), value: pressure });
    let mut buf = vec![0f32; 256 * 2];
    for _ in 0..blocks {
        e.process(&mut buf);
    }
}

fn blocks(e: &mut Engine, n: usize) {
    let mut buf = vec![0f32; 256 * 2];
    for _ in 0..n {
        e.process(&mut buf);
    }
}

/// The eight sounds with sound 1 a Kick (decay 0.5) whose decay LFO `n` drives, and
/// the project's `lfos`. Sound 1 has no notes in the song (only the tests' trigger).
fn lfo_project(n: u8, lfos: Vec<deckard_model::lfo::Lfo>) -> (tempfile::TempDir, Project) {
    use deckard_model::mod_matrix::ModSource;
    let (tmp, mut p) = eight_project();
    p.groups[0].patterns[0].notes.retain(|x| x.sound != 1);
    let snd = &mut p.groups[0].sounds[1];
    snd.sampler = None;
    snd.slots = vec![deckard_model::SlotInfo {
        module: "Kick".into(),
        name: "Kick".into(),
        active: true,
        plugin: None,
        params: vec![("decay".into(), 0.5)],
    }];
    snd.mod_routes = vec![route(ModSource::Lfo(n), 0, "Kick", field("decay"))];
    p.lfos = lfos;
    (tmp, p)
}

fn decay_now(e: &Engine) -> f64 {
    e.sounds[0][1].as_ref().unwrap().synth_param("decay").unwrap()
}

/// A MIDI input plug-in for the tests: plays what comes in an octave up, tells what it
/// was given, and with `later` plays that many ticks late.
struct OctaveUp {
    seen: Arc<Mutex<Vec<midi_input::NoteIn>>>,
    stopped: Arc<Mutex<bool>>,
    later: f64,
    held: Option<(usize, i64)>,
}

impl midi_input::MidiInput for OctaveUp {
    fn note(&mut self, note: midi_input::NoteIn, clock: midi_input::Clock, out: &mut midi_input::Outbox) {
        self.seen.lock().unwrap().push(note);
        let at = clock.now + self.later;
        match note {
            midi_input::NoteIn::On { sound, pitch, velocity, length } => {
                if length.is_none() {
                    self.held = Some((sound, pitch + 12));
                }
                out.on(at, sound, pitch + 12, velocity, length);
            }
            midi_input::NoteIn::Off { sound, pitch } => {
                self.held = None;
                out.off(at, sound, pitch + 12);
            }
        }
    }

    fn stopped(&mut self) {
        *self.stopped.lock().unwrap() = true;
    }

    fn finish(&mut self, clock: midi_input::Clock, out: &mut midi_input::Outbox) {
        if let Some((sound, pitch)) = self.held.take() {
            out.off(clock.now, sound, pitch);
        }
    }
}

type NotesIn = Arc<Mutex<Vec<midi_input::NoteIn>>>;

/// Plays `frames` output frames in one go.
fn play_frames(e: &mut Engine, frames: usize) {
    let mut buf = vec![0f32; frames * 2];
    e.process(&mut buf);
}

/// A plug-in with three output pairs: as an effect, its input as it is, half of it on
/// its second pair and a quarter on its third; as an instrument, silence on its first
/// two and a steady 0.25 on its third.
#[derive(Default)]
struct Split {
    instrument: bool,
    buses: [(Vec<f32>, Vec<f32>); 2],
}

impl deckard_plugins::native::Engine for Split {
    fn set_param(&mut self, _: u32, _: f64) {}
    fn note_on(&mut self, _: u8, _: u8) {}
    fn note_off(&mut self, _: u8) {}
    fn reset(&mut self) {}
    fn process(&mut self, left: &mut [f32], right: &mut [f32], _: &Transport) {
        if self.instrument {
            left.fill(0.0);
            right.fill(0.0);
            self.buses[0] = (vec![0.0; left.len()], vec![0.0; right.len()]);
            self.buses[1] = (vec![0.25; left.len()], vec![0.25; right.len()]);
        } else {
            for (bus, k) in self.buses.iter_mut().zip([0.5, 0.25]) {
                *bus = (left.iter().map(|v| v * k).collect(), right.iter().map(|v| v * k).collect());
            }
        }
    }
    fn outputs(&self) -> usize {
        3
    }
    fn output(&self, bus: usize, frames: usize) -> Option<(&[f32], &[f32])> {
        let (l, r) = self.buses.get(bus.checked_sub(1)?)?;
        Some((l.get(..frames)?, r.get(..frames)?))
    }
}

pub(crate) static SPLIT: deckard_plugins::native::Class = deckard_plugins::native::Class {
    cid: [7; 16],
    name: "Split",
    instrument: false,
    params: &[],
    pages: &[],
    make: |_| Box::new(Split::default()),
    scope: false,
};

pub(crate) fn split_plugin(class: &'static deckard_plugins::native::Class) -> Shared {
    let id = deckard_plugins::state::PluginId::parse(&class.plugin_id(), class.name).expect("an id");
    deckard_plugins::Plugin::native(class, id, class.name, 1000.0).shared()
}

/// A block of a steady `level` on In 1 (silence for 0): what the master's output and
/// each Out end on.
fn split_block(e: &mut Engine, level: f32) -> ((f32, f32), Vec<(f32, f32)>) {
    let mut frame = [0f32; 32];
    (frame[0], frame[1]) = (level, level);
    e.live_in.as_ref().unwrap().lock().unwrap().extend(std::iter::repeat_n(frame, 64));
    let mut out = vec![0f32; 128];
    let mut ext = vec![vec![0f32; 128]; OUTPUTS];
    e.process_outputs(&mut out, &mut ext);
    ((out[126], out[127]), ext.iter().map(|b| (b[126], b[127])).collect())
}

fn about(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// A slot playing the native plug-in `class`.
pub(crate) fn plugin_slot(class: &deckard_plugins::native::Class) -> deckard_model::SlotInfo {
    deckard_model::SlotInfo {
        module: "PluginHost".into(),
        name: class.name.into(),
        active: true,
        plugin: Some(deckard_model::PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() }),
        params: Vec::new(),
    }
}

/// A strip for [`Split`]'s pair `bus` in `owner`'s slot, at unity, onto `dest`.
pub(crate) fn split_strip(
    owner: deckard_model::extra_out::Owner,
    bus: usize,
    dest: deckard_model::AudioDest,
) -> deckard_model::extra_out::ExtraOut {
    deckard_model::extra_out::ExtraOut {
        owner,
        bus,
        plugin: SPLIT.plugin_id(),
        name: format!("Out {}", bus + 1),
        level: 1.0,
        pan: 0.0,
        muted: false,
        dest,
        channels: Default::default(),
    }
}

/// A playing setup for strips: the project, its engine, the cache and its chains.
struct Strips {
    p: Project,
    e: Engine,
    cache: SampleCache,
    chains: Vec<Vec<bool>>,
}

impl Strips {
    /// Group 0's sound 0 takes In 1 through [`Split`] (its chain's slot 0), at unity,
    /// with one strip: Split's second pair onto Out 3. `edit` changes that. Stopped,
    /// at 1 kHz. Split also plays in group 0's first slot and the master's when a
    /// project puts it there.
    fn new(edit: impl FnOnce(&mut Project)) -> Strips {
        use deckard_model::extra_out::Owner;
        let mut p = project();
        p.groups[0].patterns[0].notes.clear();
        let snd = &mut p.groups[0].sounds[0];
        snd.sampler = None;
        snd.channel.audio_in = deckard_model::AudioSource::Ext(0);
        snd.slots = vec![plugin_slot(&SPLIT)];
        p.extra_outs = vec![split_strip(Owner::Chain(0, 0, 0), 1, deckard_model::AudioDest::Ext(2))];
        edit(&mut p);
        let mut cache = SampleCache::default();
        for slot in [Slot::Chain { group: 0, sound: 0, slot: 0 }, Slot::Group { group: 0, slot: 0 }, Slot::Master { slot: 0 }] {
            cache.test_plugins.push((slot, split_plugin(&SPLIT)));
        }
        let (mut e, report) = Engine::new(&p, None, 1000, Mode::Arrangement, &mut cache);
        e.sequencing = false;
        e.live_in = Some(Default::default());
        Strips { p, e, cache, chains: report.chains }
    }

    /// Take up `edit` to the project, as an edit while playing does.
    fn edit(&mut self, edit: impl FnOnce(&mut Project)) {
        let mut to = self.p.clone();
        edit(&mut to);
        let (fresh, report) = Engine::new_after(&to, Some((&self.p, &self.chains)), None, 1000, self.e.mode(), &mut self.cache);
        drop(self.e.adopt(fresh, &to));
        (self.p, self.chains) = (to, report.chains);
    }

    /// A block of a steady `level` on In 1: the master's output and Out 3's, at its end.
    fn block(&mut self, level: f32) -> ((f32, f32), (f32, f32)) {
        let (master, ext) = split_block(&mut self.e, level);
        (master, ext[2])
    }
}

/// An instrument that plays the channel pressure it's given, as a level.
#[derive(Default)]
struct Pressed(f32);

impl deckard_plugins::native::Engine for Pressed {
    fn set_param(&mut self, _: u32, _: f64) {}
    fn note_on(&mut self, _: u8, _: u8) {}
    fn note_off(&mut self, _: u8) {}
    fn channel_pressure(&mut self, pressure: f32) {
        self.0 = pressure;
    }
    fn reset(&mut self) {}
    fn process(&mut self, left: &mut [f32], right: &mut [f32], _: &Transport) {
        left.fill(self.0);
        right.fill(self.0);
    }
}

/// A fader's lane: a channel's `field`, with no slot.
fn fader_lane(field: &str) -> deckard_model::ModTarget {
    deckard_model::ModTarget { slot: None, field: field.into() }
}

fn fader_point(sound: Option<usize>, field: &str, position: i64, value: f64) -> deckard_model::ModPoint {
    deckard_model::ModPoint { sound, slot: None, field: field.into(), position, value }
}

fn fader_project() -> Project {
    input_project()
}

fn fader_engine(p: &Project) -> Engine {
    input_project_engine(p)
}

/// A project with no notes whose only sound takes In 1, in a looped bar.
fn input_project() -> Project {
    let mut p = project();
    p.groups[0].patterns[0].notes.clear();
    p.groups[0].sounds[0].channel.audio_in = deckard_model::AudioSource::Ext(0);
    p
}

fn input_project_engine(p: &Project) -> Engine {
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(p, None, 1000, Mode::Arrangement, &mut cache);
    e.live_in = Some(Default::default());
    e.looping = true;
    e
}

/// What a steady 0.5 on In 1 comes out as after `frames`, gains settled.
fn steady(e: &mut Engine, frames: usize) -> (f32, f32) {
    let mut frame = [0f32; 32];
    (frame[0], frame[1]) = (0.5, 0.5);
    // In short blocks: gains move at each block's start.
    let mut out = vec![0f32; frames * 2];
    for block in out.chunks_mut(16) {
        e.live_in.as_ref().unwrap().lock().unwrap().extend(std::iter::repeat_n(frame, block.len() / 2));
        e.process(block);
    }
    (out[frames * 2 - 2], out[frames * 2 - 1])
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// `frames` of a steady 0.5 on In 1, as `steady`, with Out 1-2 as buffers: what the
/// master's output and Out 2 end on.
fn steady_outs(e: &mut Engine, frames: usize) -> (f32, f32) {
    let mut frame = [0f32; 32];
    (frame[0], frame[1]) = (0.5, 0.5);
    let mut last = (0.0, 0.0);
    for _ in 0..frames / 8 {
        e.live_in.as_ref().unwrap().lock().unwrap().extend(std::iter::repeat_n(frame, 8));
        let mut out = vec![0f32; 16];
        let mut ext = vec![vec![0f32; 16]; 2];
        e.process_outputs(&mut out, &mut ext);
        last = (out[14], ext[1][14]);
    }
    last
}

/// The prehear of a sample `at` frames long, in `blocks` of 8, into Out 1-2.
fn hear(e: &mut Engine, blocks: usize) -> (f32, f32) {
    e.prehear(Some(Arc::new(Sample { rate: e.sr as u32, frames: vec![[0.5, 0.5]; 1000] })), 1.0);
    let (mut master, mut cue) = (0f32, 0f32);
    for _ in 0..blocks {
        let mut out = vec![0f32; 16];
        let mut ext = vec![vec![0f32; 16]; 2];
        e.process_outputs(&mut out, &mut ext);
        master = master.max(out[14].abs());
        cue = cue.max(ext[1][14].abs());
    }
    (master, cue)
}

/// A project whose second sound takes In 2 (with `slots` and to `dest`), and an
/// engine for it, stopped, with the input queued: 64 frames of (0.5, -0.25).
fn input_engine(slots: Vec<deckard_model::SlotInfo>, dest: deckard_model::AudioDest) -> Engine {
    let mut p = project();
    let mut snd = Sound { level: 0.75, slots, ..Default::default() };
    (snd.channel.audio_in, snd.channel.dest) = (deckard_model::AudioSource::Ext(1), dest);
    p.groups[0].sounds.push(snd);
    let mut cache = SampleCache::default();
    let (mut e, _) = Engine::new(&p, None, 1000, Mode::Arrangement, &mut cache);
    e.sequencing = false;
    let live: LiveIn = Default::default();
    let mut frame = [0f32; 32];
    (frame[2], frame[3]) = (0.5, -0.25);
    live.lock().unwrap().extend(std::iter::repeat_n(frame, 64));
    e.live_in = Some(live);
    e
}

fn curve(points: &[(i64, f32)]) -> Option<deckard_model::Curve> {
    Some(deckard_model::Curve { points: points.iter().map(|&(at, value)| deckard_model::Point { at, value, curve: 0.0 }).collect() })
}

/// Process `frames` one at a time: what was sent, with the frame it was sent in.
fn sent_by_frame(e: &mut Engine, rx: &std::sync::mpsc::Receiver<midi::MidiOut>, frames: usize) -> Vec<(usize, Vec<u8>)> {
    let mut out = vec![0f32; 2];
    let mut got = Vec::new();
    for f in 0..frames {
        e.process(&mut out);
        got.extend(rx.try_iter().map(|m| (f, m.bytes().to_vec())));
    }
    got
}

fn sent(e: &mut Engine, rx: &std::sync::mpsc::Receiver<midi::MidiOut>, frames: usize) -> Vec<Vec<u8>> {
    sent_by_frame(e, rx, frames).into_iter().map(|x| x.1).collect()
}

struct HalfGainEffect;
impl fx::Effect for HalfGainEffect {
    fn set(&mut self, _: &[(String, f64)]) {}
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for s in l.iter_mut() {
            *s *= 0.5;
        }
        for s in r.iter_mut() {
            *s *= 0.5;
        }
    }
}

fn push_half_gain(chain: &mut FxChain) {
    chain.units.push(Unit::new(
        FxUnit::Internal {
            class: "HalfGain".into(),
            params: Vec::new(),
            fx: Box::new(HalfGainEffect),
            glide: glide::Glide::new(&[], 1000.0),
            modded: Vec::new(),
            hit: false,
            modulated: false,
        },
        chain.units.len(),
        Vec::new(),
        true,
    ));
}
