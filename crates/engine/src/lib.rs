//! Sample playback for Maschine projects.
//!
//! [`timeline`] flattens a project (its arrangement, one scene, or one pattern) into
//! note events in ticks. [`Engine`] turns those into audio: sounds whose instrument is
//! Maschine's Sampler play through [`sampler`] (docs/sampler-dsp.md), and sounds whose
//! instrument is a VST/VST3 plug-in through it (`deckard-plugins`). Sound and group
//! level, pan and mute apply. Effects, drum synths and NCW samples are not rendered.
//!
//! [`Engine::process`] fills interleaved stereo blocks without allocating, so it can
//! run in an audio callback; [`render`] runs it offline.

mod ahead;
pub mod audio_loop;
mod channel_sends;
use channel_sends as aux;
#[cfg(feature = "output")]
pub mod devices;
pub mod edit;
pub mod export;
pub mod fx;
pub mod glide;
mod group_move;
mod lfo_rt;
pub mod library;
pub mod meters;
mod metronome;
pub mod midi;
pub mod midi_input;
mod mod_rt;
pub mod modscale;
pub mod note_echo;
#[cfg(feature = "output")]
pub mod output;
pub mod pads;
pub mod pool;
mod prehear;
pub mod queue;
pub mod repeat;
pub use ahead::Junk;
pub mod sample;
pub mod sampler;
pub mod sync;
pub mod synth;
// The MP60 and S1200 filter tables wait for those engines (docs/sampler-dsp.md §10).
#[allow(dead_code)]
mod sampler_tables;

use deckard_model::extra_out::{self, Channels, ExtraOut, Owner};
use deckard_model::{Dim, ExprKind, MidiKind, PPQ, Project};
use deckard_plugins::{MAX_BLOCK, Shared, Slot, Transport};
use sample::Sample;
use sampler::SamplerRt;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// What to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The song's sections in order (or every scene in turn if it has none).
    Arrangement,
    Scene(usize),
    /// (group, pattern)
    Pattern(usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteEvent {
    pub tick: i64,
    pub group: usize,
    pub sound: usize,
    pub pitch: i64,
    pub velocity: i64,
    /// Ticks.
    pub length: i64,
    /// Chance of playing, 0–100 %.
    pub probability: i64,
    /// A note, or a recorded MIDI controller/pitch-bend event (then only `tick`,
    /// `group` and `sound` matter).
    pub midi: Option<MidiKind>,
    /// The sound's Audio plug-in starts its loop here (playing a take, its first beat
    /// on this tick) or stops it (then only `tick`, `group` and `sound` matter).
    pub cue: Option<Cue>,
    /// A pattern note's per-note expression ([`Timeline::exprs`]).
    pub expr: Option<deckard_model::ExprId>,
    /// A note a link added ([`pads`]): the sound whose note played it. Muting that sound
    /// silences this too (#21).
    pub from: Option<usize>,
}

/// Whether `e` is muted: its sound is, or (a link's note) the sound whose note played it.
fn note_muted(muted: &[Vec<bool>], e: &NoteEvent) -> bool {
    let of = |s: usize| muted.get(e.group).and_then(|r| r.get(s)).copied().unwrap_or(false);
    of(e.sound) || e.from.is_some_and(of)
}

/// An Audio plug-in's loop starting (with this take) or stopping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    On(usize),
    Off,
}

/// Note events sorted by tick, and the total length in ticks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timeline {
    pub events: Vec<NoteEvent>,
    pub length: i64,
    /// Modulation, by pass of a pattern.
    pub mods: Vec<modscale::ModLane>,
    /// Modulation of channels' faders (their `level` and `pan`), by pass of a pattern.
    /// Unlike [`Timeline::mods`], the points stay deviations: a moved fader needs no new
    /// timeline.
    pub faders: Vec<modscale::ModLane>,
    /// The project's per-note expression, which notes' `expr` index.
    pub exprs: Vec<std::sync::Arc<deckard_model::Expression>>,
    /// The sounds (group, sound) with PROB LOCK on: their notes' chances are the first
    /// pass's on every pass.
    pub prob_locked: Vec<(usize, usize)>,
    /// From each tick on, in order: the scene's own tempo, or `None` for the song's. Empty:
    /// the song's throughout.
    pub tempos: Vec<(i64, Option<f64>)>,
}

impl Timeline {
    /// The scene tempo in force at `tick`, if any.
    fn tempo_at(&self, tick: i64) -> Option<f64> {
        let i = self.tempos.partition_point(|t| t.0 <= tick);
        i.checked_sub(1).and_then(|i| self.tempos[i].1)
    }

    /// The tick after `tick` where the tempo may change.
    fn next_tempo(&self, tick: i64) -> Option<i64> {
        self.tempos.get(self.tempos.partition_point(|t| t.0 <= tick)).map(|t| t.0)
    }

    /// The pass whose chances event `e` takes on pass `pass`.
    fn chance_pass(&self, e: &NoteEvent, pass: u64) -> u64 {
        if self.prob_locked.contains(&(e.group, e.sound)) { 0 } else { pass }
    }
}

/// Timelines made ahead (off the audio thread) for an engine to switch to: the one for
/// the mode it plays, and the one for a mode it has queued.
#[derive(Debug, Default)]
pub struct Timelines(pub Vec<(Mode, Timeline)>);

impl Timelines {
    /// The timelines of `modes` for `project`.
    pub fn of(project: &Project, modes: &[Mode]) -> Timelines {
        let mut v: Vec<(Mode, Timeline)> = Vec::new();
        for &m in modes {
            if !v.iter().any(|(x, _)| *x == m) {
                v.push((m, timeline(project, m)));
            }
        }
        Timelines(v)
    }

    /// Swap the timeline of `mode` into `slot` (the slot's old one stays here, to be
    /// freed elsewhere). False if there is none for `mode`.
    fn swap_into(&mut self, mode: Mode, slot: &mut Timeline) -> bool {
        match self.0.iter_mut().find(|(m, _)| *m == mode) {
            Some((_, t)) => {
                std::mem::swap(t, slot);
                true
            }
            None => false,
        }
    }
}

/// Something played live (a pad, a key, a MIDI message), for [`Engine::schedule`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Live {
    /// A note of `length` ticks (`i64::MAX`: until released).
    NoteOn {
        group: usize,
        sound: usize,
        pitch: i64,
        velocity: i64,
        length: i64,
    },
    NoteRelease {
        group: usize,
        sound: usize,
        pitch: i64,
    },
    /// A note from MIDI input (see [`Engine::note_in`]).
    NoteIn {
        group: usize,
        sound: usize,
        pitch: i64,
        velocity: i64,
        thru: bool,
    },
    /// A note a MIDI input plug-in played, come due.
    MidiInputOut {
        group: usize,
        note: midi_input::NoteOut,
    },
    /// A channel-wide controller from MIDI input; `record`: the sound that records it.
    ControlIn {
        group: usize,
        sound: usize,
        kind: MidiKind,
        thru: bool,
        record: bool,
    },
    /// Per-note expression for a held note (MPE, poly pressure, pad pressure; or its
    /// release velocity, 0–1, before the release). Recorded; pressure also goes out as
    /// poly aftertouch with its note when that note went to a MIDI output.
    Expression {
        group: usize,
        sound: usize,
        pitch: i64,
        kind: ExprKind,
        value: f32,
    },
    /// A held pad pressed harder or softer in the Velocity pad mode: Note Repeat / the
    /// Arp play its notes at `velocity` (1–127) from now on. Not recorded, not sent.
    RepeatPressure {
        group: usize,
        sound: usize,
        pitch: i64,
        velocity: i64,
    },
    /// Every voice of every sound ends at once (Maschine's Choke All Notes): as a choke
    /// group's, a quick fade. The next notes play as usual. Not recorded.
    ChokeAll,
}

/// Rendering sounds ahead (see [`ahead`]): the players' links, as the audio thread
/// sees them.
struct Ahead {
    driver: ahead::Driver,
    /// `[group][sound]` (the shape when rendering ahead started; sounds added later
    /// stay live).
    links: Vec<Vec<Option<Link>>>,
    /// Frames players render ahead.
    lookahead: u64,
    /// Timeline events already sent to players (tick, group, sound, pitch, pass, frame
    /// sent for), so a block sends only the new ones.
    sent: Vec<(i64, usize, usize, i64, u64, u64)>,
    /// Note ids for players' Sampler notes (apart from the engine's own).
    next_id: u64,
    /// Offline: wait for players as long as they take (else up to the block's budget).
    wait: bool,
    /// Blocks where a player hadn't rendered in time (its sound was silent there).
    underruns: u64,
    /// How long the audio thread may wait for a player in this span (half of it).
    budget: std::time::Duration,
    /// Frames in the current span.
    span: u64,
    /// What came back from players, for the output to free off the audio thread.
    junk: Vec<ahead::Junk>,
}

/// One sound's link to its player.
struct Link {
    ring: Arc<ahead::Ring>,
    inbox: Arc<queue::Queue<ahead::Msg>>,
    voices: Arc<std::sync::atomic::AtomicUsize>,
    state: LinkState,
    /// The gains (Sampler's, plug-in's) its output takes in the mix, and whether it
    /// has effects (its output is then checked for runaway values).
    gains: [(f32, f32); 2],
    finite: bool,
    /// A box to move the sound's parts in (none while it is with the player).
    spare: Option<Box<ahead::Parts>>,
    /// For the note-offs that go with them.
    offs: Vec<(u64, u64, i64)>,
    /// What its player handed back, to take over at frame `.0`.
    returned: Option<Returned>,
}

/// A sound handed back by its player: from which frame, its parts, its note-offs to
/// come, and live events it hadn't played.
type Returned = (u64, Box<ahead::Parts>, Vec<(u64, u64, i64)>, Vec<(u64, ahead::Ev)>);

#[derive(Debug, Clone, Copy, PartialEq)]
enum LinkState {
    /// Rendered here.
    Live,
    /// Rendered by its player.
    Ahead,
    /// Asked back: its player renders up to frame `.0`; rendered here from there.
    Returning(u64),
}

/// What [`Engine::foresee`] sees coming, `t` frames from now.
enum Seen<'a> {
    /// A timeline event: whether its probability lets it play, the song frame it fires
    /// at, and the pass through the timeline.
    Event { t: u64, e: &'a NoteEvent, plays: bool, frame: u64, pass: u64 },
    /// Where the song is from here (at the start, and after a jump, a loop or a
    /// switch; `playing` false once the sequence has ended).
    Position { t: u64, frame: u64, playing: bool },
}

/// Whether timeline event `idx` plays on pass `pass` (`probability` in %): decided by a
/// hash of the event, so it comes out the same whenever and wherever it's asked
/// (rendering ahead asks before the audio thread gets there).
fn chance(probability: i64, idx: usize, pass: u64, seed: u64) -> bool {
    if probability >= 100 {
        return true;
    }
    // splitmix64
    let mut z = seed ^ (pass << 32) ^ idx as u64;
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z % 100) as i64) < probability
}

/// Live events an engine holds at once (more are played at once rather than placed).
const LIVE_ROOM: usize = 512;

/// Flatten `mode` into note events. Patterns repeat to fill their section/scene.
pub fn timeline(p: &Project, mode: Mode) -> Timeline {
    let prob_locked = p
        .groups
        .iter()
        .enumerate()
        .flat_map(|(g, grp)| grp.sounds.iter().enumerate().filter(|(_, s)| s.probability_locked).map(move |(s, _)| (g, s)))
        .collect();
    let mut t = Timeline { exprs: p.expressions.clone(), prob_locked, ..Timeline::default() };
    let add_scene = |t: &mut Timeline, scene: usize, start: i64, length: i64| {
        for &(g, pi) in &p.scenes[scene].patterns {
            fill_pattern(p, g, pi, start, length, t);
        }
    };
    match mode {
        Mode::Arrangement if !p.sections.is_empty() => {
            for s in &p.sections {
                let scene = s.scene.filter(|&i| i < p.scenes.len());
                t.tempos.push((t.length, scene.and_then(|i| p.scenes[i].tempo)));
                if let Some(scene) = scene {
                    let at = t.length;
                    add_scene(&mut t, scene, at, s.length);
                }
                t.length += s.length;
            }
            add_clips(p, &mut t);
        }
        Mode::Arrangement => {
            for scene in 0..p.scenes.len() {
                let len = p.scene_length(scene);
                let at = t.length;
                t.tempos.push((at, p.scenes[scene].tempo));
                add_scene(&mut t, scene, at, len);
                t.length += len;
            }
        }
        Mode::Scene(scene) if scene < p.scenes.len() => {
            t.length = p.scene_length(scene);
            let len = t.length;
            t.tempos.push((0, p.scenes[scene].tempo));
            add_scene(&mut t, scene, 0, len);
        }
        Mode::Pattern(g, pi) if g < p.groups.len() && pi < p.groups[g].patterns.len() => {
            t.length = pattern_length(p, g, pi);
            let len = t.length;
            fill_pattern(p, g, pi, 0, len, &mut t);
        }
        _ => {}
    }
    pads::expand(p, &mut t.events);
    // At the same tick, controller changes come first (a pedal pressed with a chord, a
    // choke before the note that chokes).
    t.events.sort_by_key(|e| (e.tick, e.midi.is_none(), e.group, e.sound, e.pitch));
    t
}

/// The groups' clips over the song: each plays its pattern (repeated to fill it), and
/// what the group's section pattern would play inside it (notes, controller events,
/// loop cues, modulation) goes; a clip past the last section lengthens the song.
fn add_clips(p: &Project, t: &mut Timeline) {
    for (g, grp) in p.groups.iter().enumerate() {
        for c in grp.clips.iter().filter(|c| c.length > 0) {
            let Some(pat) = grp.clip_patterns.get(c.pattern) else { continue };
            let span = c.position..c.position + c.length;
            // (A loop stopping where the clip starts still stops.)
            t.events.retain(|e| e.group != g || !span.contains(&e.tick) || (e.cue == Some(Cue::Off) && e.tick == span.start));
            t.mods.retain(|m| m.group != g || m.to <= span.start || m.from >= span.end);
            t.faders.retain(|m| m.group != g || m.to <= span.start || m.from >= span.end);
            fill_with(p, g, pat, c.position, c.length, t);
            t.length = t.length.max(span.end);
        }
    }
}

/// The take a cue starts (none: it stops).
fn cue_take(c: Cue) -> Option<usize> {
    match c {
        Cue::On(t) => Some(t),
        Cue::Off => None,
    }
}

fn pattern_length(p: &Project, g: usize, pi: usize) -> i64 {
    own_length(p, &p.groups[g].patterns[pi])
}

/// A pattern's (or clip's) length, a bar when it has none.
fn own_length(p: &Project, pat: &deckard_model::Pattern) -> i64 {
    if pat.length > 0 { pat.length } else { p.bar_length() }
}

/// A pattern's modulation of instruments and effects, as lanes: (sound, `None` for the
/// group; chain slot; field; points from the pattern's start, at stored values or, for
/// plug-ins, normalized deviations). A point is a deviation from the knob's value
/// ([`modscale`]).
fn mod_lanes(p: &Project, g: usize, pat: &deckard_model::Pattern) -> Vec<(Option<usize>, usize, String, Vec<(i64, f64)>)> {
    let grp = &p.groups[g];
    let plen = own_length(p, pat);
    let mut out: Vec<(Option<usize>, usize, String, Vec<(i64, f64)>)> = Vec::new();
    for m in &pat.mods {
        let Some(slot) = m.slot else { continue };
        let slots = match m.sound {
            Some(s) => grp.sounds.get(s).map(|x| &x.slots),
            None => Some(&grp.slots),
        };
        let Some(info) = slots.and_then(|x| x.get(slot)) else { continue };
        let pos = m.position - pat.start;
        if !(0..plen).contains(&pos) {
            continue;
        }
        let v = if info.plugin.is_some() {
            if m.field.parse::<u32>().is_err() {
                continue;
            }
            m.value
        } else if slot == 0 && info.module == "Sampler" {
            let Some(smp) =
                m.sound.and_then(|s| grp.sounds.get(s)?.sampler.as_ref()).filter(|_| sampler::MODULATED.contains(&m.field.as_str()))
            else {
                continue;
            };
            modscale::scale_of("Sampler", &m.field).apply(smp.get(&m.field), m.value)
        } else {
            let Some(&(_, base)) = info.params.iter().find(|(k, _)| *k == m.field) else { continue };
            modscale::scale_of(&info.module, &m.field).apply(base, m.value)
        };
        match out.iter_mut().find(|l| l.0 == m.sound && l.1 == slot && l.2 == m.field) {
            Some(l) => l.3.push((pos, v)),
            None => out.push((m.sound, slot, m.field.clone(), vec![(pos, v)])),
        }
    }
    for l in &mut out {
        l.3.sort_by_key(|x| x.0);
    }
    out
}

/// A pattern's modulation of channels' faders, as lanes: (sound, `None` for the group;
/// field; points from the pattern's start, at their deviations).
fn fader_lanes(p: &Project, pat: &deckard_model::Pattern) -> Vec<(Option<usize>, String, Vec<(i64, f64)>)> {
    let plen = own_length(p, pat);
    let mut out: Vec<(Option<usize>, String, Vec<(i64, f64)>)> = Vec::new();
    for m in pat.mods.iter().filter(|m| m.slot.is_none() && matches!(m.field.as_str(), "level" | "pan" | "aux1_level" | "aux2_level")) {
        let pos = m.position - pat.start;
        if !(0..plen).contains(&pos) {
            continue;
        }
        match out.iter_mut().find(|l| l.0 == m.sound && l.1 == m.field) {
            Some(l) => l.2.push((pos, m.value)),
            None => out.push((m.sound, m.field.clone(), vec![(pos, m.value)])),
        }
    }
    for l in &mut out {
        l.2.sort_by_key(|x| x.0);
    }
    out
}

/// Notes of a pattern looped over `[start, start + length)`.
fn fill_pattern(p: &Project, g: usize, pi: usize, start: i64, length: i64, t: &mut Timeline) {
    fill_with(p, g, &p.groups[g].patterns[pi], start, length, t);
}

/// [`fill_pattern`] for any of group `g`'s patterns (a clip's too).
fn fill_with(p: &Project, g: usize, pat: &deckard_model::Pattern, start: i64, length: i64, t: &mut Timeline) {
    let plen = own_length(p, pat);
    let lanes = mod_lanes(p, g, pat);
    let fader_lanes = fader_lanes(p, pat);
    let out = &mut t.events;
    let mut rep = 0;
    while rep < length {
        let pass = (start + rep, start + rep + plen.min(length - rep));
        for (sound, field, points) in &fader_lanes {
            let points: Vec<(i64, f64)> = points.iter().map(|&(pos, v)| (pass.0 + pos, v)).filter(|x| x.0 < pass.1).collect();
            if !points.is_empty() {
                t.faders.push(modscale::ModLane {
                    group: g,
                    sound: *sound,
                    slot: 0,
                    field: field.clone(),
                    from: pass.0,
                    to: pass.1,
                    points,
                });
            }
        }
        for (sound, slot, field, points) in &lanes {
            let points: Vec<(i64, f64)> = points.iter().map(|&(pos, v)| (pass.0 + pos, v)).filter(|x| x.0 < pass.1).collect();
            if !points.is_empty() {
                t.mods.push(modscale::ModLane {
                    group: g,
                    sound: *sound,
                    slot: *slot,
                    field: field.clone(),
                    from: pass.0,
                    to: pass.1,
                    points,
                });
            }
        }
        // Audio plug-ins on in the pattern: each instance starts its loop over (its first
        // beat on the pattern's), and stops it at its end (a stop sorts before a start
        // at the same tick, a start before the notes).
        for c in &pat.audio {
            let at = start + rep;
            let end = start + rep + plen.min(length - rep);
            let cue = |tick: i64, cue: Cue| NoteEvent {
                tick,
                group: g,
                sound: c.sound,
                pitch: if cue == Cue::Off { -2 } else { -1 },
                velocity: 0,
                length: 0,
                probability: 100,
                midi: None,
                cue: Some(cue),
                expr: None,
                from: None,
            };
            out.push(cue(at, Cue::On(c.take)));
            out.push(cue(end, Cue::Off));
        }
        for n in &pat.notes {
            let pos = n.position - pat.start;
            if pos < 0 || pos >= plen || rep + pos >= length {
                continue;
            }
            out.push(NoteEvent {
                tick: swung(p, g, n.sound, start + rep + pos),
                group: g,
                sound: n.sound,
                pitch: n.pitch,
                velocity: n.velocity,
                length: n.length,
                probability: n.probability,
                midi: None,
                cue: None,
                expr: n.expr,
                from: None,
            });
        }
        for m in &pat.midi {
            let pos = m.position - pat.start;
            if pos < 0 || pos >= plen || rep + pos >= length {
                continue;
            }
            out.push(NoteEvent {
                tick: swung(p, g, m.sound, start + rep + pos),
                group: g,
                sound: m.sound,
                pitch: 0,
                velocity: 0,
                length: 0,
                probability: 100,
                midi: Some(m.kind),
                cue: None,
                expr: None,
                from: None,
            });
        }
        rep += plen;
    }
}

/// Where a note at `tick` plays with its sound's, group's and the master groove
/// (applied in that order; each adds to the one before, as in Maschine).
pub fn swung(p: &Project, g: usize, sound: usize, tick: i64) -> i64 {
    let group = &p.groups[g];
    let grooves = [group.sounds.get(sound).map(|s| s.swing), Some(group.swing), Some(p.swing)];
    if grooves.iter().flatten().all(|s| s.is_off()) {
        return tick;
    }
    let mut beats = tick as f64 / PPQ as f64;
    for s in grooves.iter().flatten().filter(|s| !s.is_off()) {
        beats = s.apply(beats);
    }
    (beats * PPQ as f64).round() as i64
}

/// Constant-power pan gains, normalized to unity at center.
fn pan_gains(pan: f64) -> (f32, f32) {
    let th = (pan.clamp(-1.0, 1.0) + 1.0) * std::f64::consts::FRAC_PI_4;
    ((th.cos() * std::f64::consts::SQRT_2) as f32, (th.sin() * std::f64::consts::SQRT_2) as f32)
}

/// The gain (left, right) from a sound's output to the output it plays on, along its
/// routing: its level and pan, then where its Dest. leads (its group or another, a bus
/// sound, the master, an external output, or the cue bus), each channel on the way
/// adding its level and pan. Effects aren't rendered, so a bus passes the sound through
/// as it is. The master's own level and pan come later, so a channel that bypasses the
/// master onto the master's output is divided by them here. Which output: [`sound_bus`].
pub fn sound_gain(p: &Project, g: usize, s: usize) -> (f32, f32) {
    sound_route(p, g, s).1
}

/// The external output (Out 1–16, from 0) a sound plays on when it isn't the master's
/// (an Ext. n of its own, or the cue bus's); `None`: the master's output.
pub fn sound_bus(p: &Project, g: usize, s: usize) -> Option<usize> {
    sound_route(p, g, s).0
}

/// [`sound_gain`] and [`sound_bus`].
fn sound_route(p: &Project, g: usize, s: usize) -> Route {
    let Some(snd) = p.groups.get(g).and_then(|x| x.sounds.get(s)) else { return SILENT };
    if !snd.cue && has_sound_bus(snd) && reaches_main(p, snd.channel.dest, (g, Some(s)), Some(g), 0) {
        return SILENT;
    }
    let own = fader_gain(snd.level, snd.pan, &snd.modulators);
    let (bus, gain) = if snd.cue { cue_route(p) } else { dest_route(p, snd.channel.dest, Some(g), 0) };
    (bus, mul(own, gain))
}

/// An output (see [`sound_bus`]) and the gain to it.
type Route = (Option<usize>, (f32, f32));
const SILENT: Route = (None, (0.0, 0.0));

/// External outputs: Out 1–16, stereo.
pub const OUTPUTS: usize = 16;

fn mul(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    (a.0 * b.0, a.1 * b.1)
}

fn channel_gain(level: f64, pan: f64) -> (f32, f32) {
    let (l, r) = pan_gains(pan);
    let lvl = deckard_model::level_gain(level) as f32;
    (lvl * l, lvl * r)
}

/// Whether a channel's property `field` is one of its fader's.
fn is_fader(field: &str) -> bool {
    matches!(field, "level" | "pan")
}

/// Whether a channel's fader has a modulation lane: its gain then isn't in the routes
/// through it, which [`channel_faders`] lists for the engine to apply at each moment.
fn moving(modulators: &[deckard_model::ModTarget]) -> bool {
    modulators.iter().any(|m| m.slot.is_none() && is_fader(&m.field))
}

/// A channel rendered without its fader goes without the fader's modulation too.
fn drop_fader_lanes(modulators: &mut Vec<deckard_model::ModTarget>) {
    modulators.retain(|m| m.slot.is_some() || !is_fader(&m.field));
}

/// A channel's fader in a route: unity when it's modulated ([`moving`]).
fn fader_gain(level: f64, pan: f64, modulators: &[deckard_model::ModTarget]) -> (f32, f32) {
    if moving(modulators) { (1.0, 1.0) } else { channel_gain(level, pan) }
}

/// A channel whose fader is modulated, as the audio thread has it.
#[derive(Debug, Clone, PartialEq)]
struct Fader {
    group: usize,
    sound: Option<usize>,
    /// The fader's own place.
    level: f64,
    pan: f64,
    /// Its gain now.
    gain: (f32, f32),
}

/// The gain of the faders on a path.
fn path_gain(faders: &[Fader], path: &[usize]) -> (f32, f32) {
    path.iter().filter_map(|&i| faders.get(i)).fold((1.0, 1.0), |gain, f| mul(gain, f.gain))
}

/// The modulated faders along the routes (what [`sound_route`] and [`group_out_route`]
/// leave out): the faders, then each sound's path and each group bus's, as places in them.
fn channel_faders(p: &Project) -> (Vec<Fader>, Vec<Vec<Vec<usize>>>, Vec<Vec<usize>>) {
    let mut faders = Vec::new();
    for (g, grp) in p.groups.iter().enumerate() {
        if moving(&grp.modulators) {
            faders.push(Fader { group: g, sound: None, level: grp.level, pan: grp.pan, gain: channel_gain(grp.level, grp.pan) });
        }
        for (s, snd) in grp.sounds.iter().enumerate().filter(|(_, snd)| moving(&snd.modulators)) {
            faders.push(Fader { group: g, sound: Some(s), level: snd.level, pan: snd.pan, gain: channel_gain(snd.level, snd.pan) });
        }
    }
    let place = |g: usize, s: Option<usize>| faders.iter().position(|f| (f.group, f.sound) == (g, s));
    // As `dest_route`, `group_route` and `group_fader_route` walk.
    fn after_group(p: &Project, g: usize, depth: u8, place: &dyn Fn(usize, Option<usize>) -> Option<usize>, out: &mut Vec<usize>) {
        let Some(grp) = p.groups.get(g) else { return };
        out.extend(place(g, None));
        if !grp.cue {
            after_dest(p, grp.channel.dest, None, depth, place, out);
        }
    }
    fn after_dest(
        p: &Project,
        dest: deckard_model::AudioDest,
        parent: Option<usize>,
        depth: u8,
        place: &dyn Fn(usize, Option<usize>) -> Option<usize>,
        out: &mut Vec<usize>,
    ) {
        use deckard_model::AudioDest as D;
        if depth > 8 {
            return;
        }
        let into = |g: usize, out: &mut Vec<usize>| {
            if p.groups.get(g).is_some_and(|grp| !has_bus(grp)) {
                after_group(p, g, depth + 1, place, out);
            }
        };
        match dest {
            D::Parent => {
                if let Some(g) = parent {
                    into(g, out);
                }
            }
            D::Group(g) => into(g, out),
            D::Sound(g, s) => {
                if let Some(bus) = p.groups.get(g).and_then(|x| x.sounds.get(s)).filter(|bus| !has_sound_bus(bus)) {
                    out.extend(place(g, Some(s)));
                    if !bus.cue {
                        after_dest(p, bus.channel.dest, Some(g), depth + 1, place, out);
                    }
                }
            }
            D::None | D::Master | D::Ext(_) => {}
        }
    }
    let sounds = p
        .groups
        .iter()
        .enumerate()
        .map(|(g, grp)| {
            grp.sounds
                .iter()
                .enumerate()
                .map(|(s, snd)| {
                    let mut path = Vec::new();
                    path.extend(place(g, Some(s)));
                    if !snd.cue {
                        after_dest(p, snd.channel.dest, Some(g), 0, &place, &mut path);
                    }
                    path
                })
                .collect()
        })
        .collect();
    let buses = (0..p.groups.len())
        .map(|g| {
            let mut path = Vec::new();
            after_group(p, g, 1, &place, &mut path);
            path
        })
        .collect();
    (faders, sounds, buses)
}

/// Undo the master's gain (for what doesn't go through it).
fn past_master(p: &Project) -> (f32, f32) {
    let m = channel_gain(p.master_level, p.master_pan);
    let inv = |x: f32| if x > 1e-6 { 1.0 / x } else { 0.0 };
    (inv(m.0), inv(m.1))
}

/// Straight onto external output `n`: the master's (past its gain), or another.
fn ext_route(p: &Project, n: i64, gain: (f32, f32)) -> Route {
    use deckard_model::AudioDest as D;
    if D::Ext(n) == p.master_output {
        (None, mul(gain, past_master(p)))
    } else if (0..OUTPUTS as i64).contains(&n) {
        (Some(n as usize), gain)
    } else {
        SILENT
    }
}

/// The cue bus as the audio thread has it: where it goes, what's on it, and its meter.
struct CueBus {
    /// Where the metronome and the prehear go: the bus, or with no output for it the
    /// master's output as it is.
    clicks_route: Route,
    /// From what's mixed onto the bus's output to the bus's own level: a bus on the
    /// master's output has been divided by the master's gain, which comes later.
    to_meter: (f32, f32),
    /// `[group][sound]`: this sound's output is on the bus. A sound going through a group
    /// with effects isn't, its group's bus is.
    sounds: Vec<Vec<bool>>,
    /// Group `g`'s bus (after its effects) is on the bus.
    groups: Vec<bool>,
    /// What's on the bus over the part being rendered, and the click's and prehear's.
    mix: Vec<f32>,
    clicks: Vec<f32>,
    /// Peak (left, right) since the UI last took it.
    peak: (f32, f32),
}

impl CueBus {
    fn of(p: &Project) -> CueBus {
        let route = cue_route(p);
        let clicks_route = match p.cue_output {
            deckard_model::AudioDest::Ext(n) if (0..OUTPUTS as i64).contains(&n) => route,
            _ => (None, (1.0, 1.0)),
        };
        let to_meter = if route.0.is_none() { channel_gain(p.master_level, p.master_pan) } else { (1.0, 1.0) };
        let sounds = p
            .groups
            .iter()
            .enumerate()
            .map(|(g, grp)| grp.sounds.iter().map(|s| s.cue || dest_cued(p, s.channel.dest, Some(g), 0)).collect())
            .collect();
        let groups = p.groups.iter().map(|g| g.cue || dest_cued(p, g.channel.dest, None, 1)).collect();
        CueBus {
            clicks_route,
            to_meter,
            sounds,
            groups,
            mix: vec![0.0; JOB_FRAMES * 2],
            clicks: vec![0.0; JOB_FRAMES * 2],
            peak: (0.0, 0.0),
        }
    }

    /// The click and the prehear (in `clicks`) over `frames`, onto `dst` and the bus; then
    /// the meter takes what's on the bus, ready for the next part.
    fn finish(&mut self, frames: usize, dst: &mut [f32]) {
        let gain = self.clicks_route.1;
        for f in 0..frames {
            let (a, z) = (self.clicks[f * 2] * gain.0, self.clicks[f * 2 + 1] * gain.1);
            dst[f * 2] += a;
            dst[f * 2 + 1] += z;
            self.mix[f * 2] += a;
            self.mix[f * 2 + 1] += z;
        }
        for frame in self.mix[..frames * 2].chunks_exact(2) {
            self.peak.0 = self.peak.0.max((frame[0] * self.to_meter.0).abs());
            self.peak.1 = self.peak.1.max((frame[1] * self.to_meter.1).abs());
        }
        self.mix[..frames * 2].fill(0.0);
    }
}

/// Whether entering `dest` leads onto the cue bus (as [`dest_route`] walks; a group with
/// effects ends it, its own bus going on from there).
fn dest_cued(p: &Project, dest: deckard_model::AudioDest, parent: Option<usize>, depth: u8) -> bool {
    use deckard_model::AudioDest as D;
    if depth > 8 {
        return false;
    }
    let group = |g: usize| p.groups.get(g).is_some_and(|grp| !has_bus(grp) && (grp.cue || dest_cued(p, grp.channel.dest, None, depth + 1)));
    match dest {
        D::Parent => parent.is_some_and(group),
        D::Group(g) => group(g),
        D::Sound(g, s) => p
            .groups
            .get(g)
            .and_then(|x| x.sounds.get(s))
            .is_some_and(|bus| !has_sound_bus(bus) && (bus.cue || dest_cued(p, bus.channel.dest, Some(g), depth + 1))),
        D::None | D::Master | D::Ext(_) => false,
    }
}

/// The cue bus, on its output.
fn cue_route(p: &Project) -> Route {
    match p.cue_output {
        deckard_model::AudioDest::Ext(n) => ext_route(p, n, channel_gain(p.cue_level, p.cue_pan)),
        _ => SILENT,
    }
}

/// Where entering `dest` leads, and the gain on the way. `parent`: the group above
/// (for a sound's "Group"); none for a group, whose parent is the master.
fn dest_route(p: &Project, dest: deckard_model::AudioDest, parent: Option<usize>, depth: u8) -> Route {
    use deckard_model::AudioDest as D;
    if depth > 8 {
        return SILENT;
    }
    match dest {
        D::None => SILENT,
        D::Parent => match parent {
            Some(g) => group_route(p, g, depth + 1),
            None => (None, (1.0, 1.0)),
        },
        D::Master => (None, (1.0, 1.0)),
        D::Ext(n) => ext_route(p, n, (1.0, 1.0)),
        D::Group(g) => group_route(p, g, depth + 1),
        D::Sound(g, s) => sound_in_route(p, g, s, depth + 1),
    }
}

/// Where a plug-in's extra output pair with a strip goes: its route (its own level and
/// pan, then its Dest.), and, for one onto an external output (`ext`), the route through
/// its parent, taken while that output isn't connected ([`Engine::set_connected_outputs`]).
/// A muted strip is silent.
#[derive(Debug, Clone, Copy)]
struct StripRoute {
    route: Route,
    fallback: Route,
    ext: Option<usize>,
}

/// A strip's own gain: its level (linear, unlike a channel's fader value) and its pan.
/// A mono strip (mono, or a split half) feeds its one signal to both sides first, so
/// this places it as a channel's pan places a sound: the same law ([`pan_gains`]).
fn strip_gain(level: f64, pan: f64) -> (f32, f32) {
    let lvl = if level.is_finite() { level.clamp(0.0, 4.0) as f32 } else { 0.0 };
    let (l, r) = pan_gains(if pan.is_finite() { pan } else { 0.0 });
    (lvl * l, lvl * r)
}

/// Where strip `x` goes. Its parent: a sound's plug-in's, the sound's group; a group's
/// or the master's, the master.
fn strip_route(p: &Project, x: &ExtraOut) -> StripRoute {
    use deckard_model::AudioDest as D;
    if x.muted {
        return StripRoute { route: SILENT, fallback: SILENT, ext: None };
    }
    let parent = match x.owner {
        Owner::Sound(g, _) | Owner::Chain(g, _, _) => Some(g),
        Owner::Group(..) | Owner::Master(_) => None,
    };
    let gain = strip_gain(x.level, x.pan);
    let to = |d: D| {
        let (to, g) = dest_route(p, d, parent, 0);
        (to, mul(gain, g))
    };
    let route = to(x.dest);
    let ext = match (x.dest, route.0) {
        (D::Ext(_), Some(n)) if n < OUTPUTS => Some(n),
        _ => None,
    };
    StripRoute { route, fallback: if ext.is_some() { to(D::Parent) } else { route }, ext }
}

/// The strips of `owner`'s slot playing `plugin_id`, each with its place in
/// `project.extra_outs` (a pair beyond the plug-in's first; one it doesn't have stays
/// silent).
fn strips_of(owner: Owner, plugin_id: &str, project: &Project) -> Vec<ExtraRt> {
    project
        .extra_outs
        .iter()
        .enumerate()
        .filter(|(_, x)| x.owner == owner && x.plugin == plugin_id && x.bus >= 1)
        .map(|(index, x)| {
            let StripRoute { route, fallback, ext } = strip_route(project, x);
            ExtraRt {
                index,
                bus: x.bus,
                channels: x.channels,
                buf: vec![0.0; JOB_FRAMES * 2],
                route,
                fallback,
                ext,
                ramp: GainRamp::default(),
            }
        })
        .collect()
}

/// A strip that can be heard: unmuted, going somewhere, and its slot still holds its
/// plug-in.
fn strip_sounds(p: &Project, x: &ExtraOut) -> bool {
    !x.muted && x.dest != deckard_model::AudioDest::None && extra_out::plugin_at(p, x.owner).is_some_and(|pr| pr.id == x.plugin)
}

/// Sound (`g`, `s`)'s plug-ins feed a strip that can be heard.
fn sound_feeds_strip(p: &Project, g: usize, s: usize) -> bool {
    p.extra_outs.iter().any(|x| matches!(x.owner, Owner::Sound(a, b) | Owner::Chain(a, b, _) if (a, b) == (g, s)) && strip_sounds(p, x))
}

/// Group `g`'s own plug-ins feed a strip that can be heard.
fn group_feeds_strip(p: &Project, g: usize) -> bool {
    p.extra_outs.iter().any(|x| matches!(x.owner, Owner::Group(a, _) if a == g) && strip_sounds(p, x))
}

/// Sound (`g`, `s`) has a strip (heard or not): it plays live, where its strips are mixed.
fn sound_has_strip(p: &Project, g: usize, s: usize) -> bool {
    p.extra_outs.iter().any(|x| matches!(x.owner, Owner::Sound(a, b) | Owner::Chain(a, b, _) if (a, b) == (g, s)))
}

/// How mutes leave a sound. A muted sound (or one in a muted group) whose plug-ins feed a
/// strip that can be heard, or in a muted group whose own plug-ins do, still plays, for
/// those strips: only its main pair is silent (`hushed`; a muted group's is silent at
/// the group). Without such strips, as ever.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Muting {
    /// It plays no new notes (notes ring out).
    muted: bool,
    /// It isn't played at all (Audio Mute, or a muted group).
    silent: bool,
    /// It plays, but its main pair is silent.
    hushed: bool,
}

fn muting(p: &Project, g: usize, s: usize) -> Muting {
    let (grp, snd) = (&p.groups[g], &p.groups[g].sounds[s]);
    let own = sound_feeds_strip(p, g, s);
    let plays = (!snd.muted || own) && (!grp.muted || own || group_feeds_strip(p, g));
    Muting { muted: !plays, silent: !plays && (grp.muted || (snd.muted && snd.channel.audio_mute)), hushed: plays && snd.muted }
}

/// A route's output for group `g`'s bus (`GROUP_BUS + g`): a group with effects takes
/// its sounds in there, runs its effects, then its fader (see [`group_out_route`]).
const GROUP_BUS: usize = 1 << 20;
const SOUND_BUS: usize = 1 << 21;
const SOUND_STRIDE: usize = 1 << 10;

fn sound_bus_id(g: usize, s: usize) -> usize {
    SOUND_BUS + g * SOUND_STRIDE + s
}

fn decode_sound_bus(n: usize) -> Option<(usize, usize)> {
    (n >= SOUND_BUS).then(|| ((n - SOUND_BUS) / SOUND_STRIDE, (n - SOUND_BUS) % SOUND_STRIDE))
}

/// Whether group `g` has a bus: effects in its chain, or an Aux send.
fn has_bus(grp: &deckard_model::Group) -> bool {
    !grp.slots.is_empty() || grp.channel.aux_dest.iter().any(|d| *d != deckard_model::AudioDest::None)
}

/// Whether sound `snd` has a bus: active effects in its chain, or an Aux send.
fn has_sound_bus(snd: &deckard_model::Sound) -> bool {
    !effect_slots(snd).is_empty() || snd.channel.aux_dest.iter().any(|d| *d != deckard_model::AudioDest::None)
}

/// Whether sound (`g`, `s`) has a bus and another channel routes or sends audio into it.
fn is_sound_bus(p: &Project, g: usize, s: usize) -> bool {
    let Some(snd) = p.groups.get(g).and_then(|grp| grp.sounds.get(s)) else { return false };
    if !has_sound_bus(snd) {
        return false;
    }
    let target = deckard_model::AudioDest::Sound(g, s);
    p.groups.iter().enumerate().any(|(gi, grp)| {
        grp.channel.dest == target
            || grp.channel.aux_dest.contains(&target)
            || grp
                .sounds
                .iter()
                .enumerate()
                .any(|(si, x)| (gi, si) != (g, s) && (x.channel.dest == target || x.channel.aux_dest.contains(&target)))
    }) || p.extra_outs.iter().any(|x| x.dest == target && strip_sounds(p, x))
}

fn sound_in_route(p: &Project, g: usize, s: usize, depth: u8) -> Route {
    let Some((grp, bus)) = p.groups.get(g).and_then(|grp| Some((grp, grp.sounds.get(s)?))) else { return SILENT };
    if (grp.muted || (bus.muted && bus.channel.audio_mute)) && !(has_sound_bus(bus) && sound_feeds_strip(p, g, s)) {
        return SILENT;
    }
    if has_sound_bus(bus) {
        return (Some(sound_bus_id(g, s)), (1.0, 1.0));
    }
    let (to, gain) = if bus.cue { cue_route(p) } else { dest_route(p, bus.channel.dest, Some(g), depth) };
    (to, mul(fader_gain(bus.level, bus.pan, &bus.modulators), gain))
}

fn reaches_main(p: &Project, dest: deckard_model::AudioDest, wanted: (usize, Option<usize>), parent: Option<usize>, depth: u8) -> bool {
    use deckard_model::AudioDest as D;
    if depth > 16 {
        return true;
    }
    let group = |g: usize| {
        if (g, None) == wanted {
            return true;
        }
        p.groups.get(g).is_some_and(|grp| !grp.cue && reaches_main(p, grp.channel.dest, wanted, None, depth + 1))
    };
    match dest {
        D::None | D::Master | D::Ext(_) => false,
        D::Parent => parent.is_some_and(group),
        D::Group(g) => group(g),
        D::Sound(g, s) => {
            if (g, Some(s)) == wanted {
                return true;
            }
            p.groups
                .get(g)
                .and_then(|grp| grp.sounds.get(s))
                .is_some_and(|snd| !snd.cue && reaches_main(p, snd.channel.dest, wanted, Some(g), depth + 1))
        }
    }
}

/// A group channel's route (silent when muted): its bus, if it has effects; else its
/// fader and where it goes, folded into what enters it. (A muted group whose plug-ins
/// feed a strip that can be heard still takes its sounds in, for them: its bus's own
/// way on is silent, see [`group_out_route`].)
fn group_route(p: &Project, g: usize, depth: u8) -> Route {
    let Some(grp) = p.groups.get(g) else { return SILENT };
    if grp.muted && !(has_bus(grp) && group_feeds_strip(p, g)) {
        return SILENT;
    }
    if has_bus(grp) {
        return (Some(GROUP_BUS + g), (1.0, 1.0));
    }
    group_fader_route(p, g, depth)
}

/// Where a group's bus goes after its effects: its fader, then its Dest.
fn group_out_route(p: &Project, g: usize) -> Route {
    match p.groups.get(g) {
        Some(grp) if !grp.muted => group_fader_route(p, g, 1),
        _ => SILENT,
    }
}

fn group_fader_route(p: &Project, g: usize, depth: u8) -> Route {
    let grp = &p.groups[g];
    if !grp.cue && has_bus(grp) && reaches_main(p, grp.channel.dest, (g, None), None, 0) {
        return SILENT;
    }
    let own = fader_gain(grp.level, grp.pan, &grp.modulators);
    let (to, gain) = if grp.cue { cue_route(p) } else { dest_route(p, grp.channel.dest, None, depth) };
    (to, mul(own, gain))
}

/// A group with effects: what its sounds send it over a span (interleaved stereo),
/// its chain, and where it goes after it (its fader and Dest.), gliding.
struct Bus {
    chain: FxChain,
    buf: Vec<f32>,
    route: Route,
    ramp: GainRamp,
}

/// The groups' buses in the order they run: one that feeds another first (by its fader,
/// or by a strip of one of its plug-ins).
fn bus_order(buses: &[Option<Bus>]) -> Vec<usize> {
    /// How many buses `g` feeds in a row, each bus worked out once (`memo`): a bus met
    /// again on its own way (a loop) counts as none there.
    fn depth(buses: &[Option<Bus>], g: usize, memo: &mut [Option<usize>], busy: &mut [bool]) -> usize {
        let Some(Some(b)) = buses.get(g) else { return 0 };
        if let Some(d) = memo[g] {
            return d;
        }
        if busy[g] {
            return 0;
        }
        busy[g] = true;
        let strips = b.chain.units.iter().flat_map(|u| &u.extras).flat_map(|x| [x.route.0, x.fallback.0]);
        let next: Vec<usize> = std::iter::once(b.route.0)
            .chain(strips)
            .filter_map(|n| n.filter(|&n| (GROUP_BUS..SOUND_BUS).contains(&n)))
            .map(|n| n - GROUP_BUS)
            .collect();
        let d = next.into_iter().map(|n| 1 + depth(buses, n, memo, busy)).max().unwrap_or(0);
        busy[g] = false;
        memo[g] = Some(d);
        d
    }
    let mut memo = vec![None; buses.len()];
    let mut busy = vec![false; buses.len()];
    let depths: Vec<usize> = (0..buses.len()).map(|g| depth(buses, g, &mut memo, &mut busy)).collect();
    let mut order: Vec<usize> = (0..buses.len()).filter(|&g| buses[g].is_some()).collect();
    order.sort_by_key(|&g| std::cmp::Reverse(depths[g]));
    order
}

/// A chain of `units` with no fader of its own (a group's, the master's).
fn bare_chain(units: Vec<Unit>, sr_f: f64) -> FxChain {
    FxChain {
        units,
        gain: (1.0, 1.0),
        l: vec![0.0; MAX_BLOCK],
        r: vec![0.0; MAX_BLOCK],
        dl: vec![0.0; MAX_BLOCK],
        dr: vec![0.0; MAX_BLOCK],
        fade: (FX_FADE_SECONDS * sr_f).round() as u32,
        meters: None,
        tap: Vec::new(),
        sl: vec![0.0; MAX_BLOCK],
        sr: vec![0.0; MAX_BLOCK],
        heard: None,
    }
}

fn plugin_unit(plugin: Shared, params: &[(String, f64)], sr_f: f64) -> FxUnit {
    let mut sc_filter = fx::sidechain::SidechainFilter::new(sr_f);
    sc_filter.set(params);
    FxUnit::Plugin { plugin, params: params.to_vec(), sc_filter }
}

/// The effects of a group's or the master's chain (all its slots), as a sound's are
/// made: plug-ins (`key` names their slot), Maschine's own effects DECKARD renders.
#[allow(clippy::too_many_arguments)]
fn effect_units(
    project: &Project,
    slots: &[deckard_model::SlotInfo],
    key: impl Fn(usize) -> Slot,
    hears: impl Fn(usize) -> Vec<usize>,
    owner: &str,
    tempo: f64,
    sr_f: f64,
    cache: &mut SampleCache,
    report: &mut LoadReport,
) -> Vec<Unit> {
    let mut units = Vec::with_capacity(slots.len() + FX_SPARE);
    for (slot, info) in slots.iter().enumerate() {
        let listen = if info.active { hears(slot) } else { Vec::new() };
        match &info.plugin {
            Some(pr) if !info.active => {
                if let Some(p) = cache.loaded_plugin(key(slot), &pr.id, &pr.name) {
                    let mut u = Unit::new(plugin_unit(p.clone(), &info.params, sr_f), slot, Vec::new(), false);
                    u.listen(owner_of(key(slot)), &p, &pr.id, project);
                    units.push(u);
                }
            }
            Some(pr) => match cache.plugin(key(slot), &pr.id, &pr.name, &pr.chunk, sr_f) {
                Ok(p) => {
                    report.plugins.push(format!("{owner}: {}", pr.name));
                    let mut u = Unit::new(plugin_unit(p.clone(), &info.params, sr_f), slot, listen, true);
                    u.listen(owner_of(key(slot)), &p, &pr.id, project);
                    units.push(u);
                }
                Err(e) => report.plugin_errors.push(e),
            },
            None => {
                if let Some(mut fx) = fx::make(&info.module, &info.params, sr_f) {
                    fx.set_tempo(tempo);
                    let glide = glide::Glide::new(&info.params, sr_f);
                    let unit = FxUnit::Internal {
                        class: info.module.clone(),
                        params: info.params.clone(),
                        fx,
                        glide,
                        modded: info.params.clone(),
                        hit: false,
                        modulated: false,
                    };
                    units.push(Unit::new(unit, slot, listen, info.active));
                }
            }
        }
    }
    units
}

/// Run `buf` (interleaved stereo, `frames` long) through `chain` in place; `at(done)`:
/// the song's position `done` frames in.
fn run_chain(chain: &mut FxChain, buf: &mut [f32], frames: usize, at: impl Fn(usize) -> Transport, sc: &[Vec<f32>]) {
    let mut done = 0;
    while done < frames {
        let n = (frames - done).min(MAX_BLOCK);
        for f in 0..n {
            chain.l[f] = buf[(done + f) * 2];
            chain.r[f] = buf[(done + f) * 2 + 1];
        }
        chain.run(n, &at(done), sc, done);
        for f in 0..n {
            buf[(done + f) * 2] = chain.l[f];
            buf[(done + f) * 2 + 1] = chain.r[f];
        }
        done += n;
    }
}

/// A sound's effect plug-ins (its chain after the instrument), in order, and the
/// sound's level and pan, which come after them.
struct FxChain {
    units: Vec<Unit>,
    gain: (f32, f32),
    l: Vec<f32>,
    r: Vec<f32>,
    /// The input of a unit being faded in or out (its dry signal).
    dl: Vec<f32>,
    dr: Vec<f32>,
    /// Frames an effect takes to fade in or out (bypassed, added, removed).
    fade: u32,
    /// Where metered effects post their meters, and this chain's sound (group, sound).
    meters: Option<(Arc<meters::FxMeters>, usize, usize)>,
    /// This sound feeds side-chains: per tap, its buffer and gain (1 before its fader,
    /// for a sound source; its fader, for its group's sum).
    tap: Vec<(usize, (f32, f32))>,
    sl: Vec<f32>,
    sr: Vec<f32>,
    /// Where its plug-ins' extra output pairs that have no strip yet are noted when they
    /// make sound (see [`Unit::scan`]).
    heard: Option<Arc<meters::HeardBoard>>,
}

/// How long an effect takes to fade in or out when it is switched on or off, added or
/// removed while playing (Maschine crossfades graph changes too).
const FX_FADE_SECONDS: f64 = 0.02;

/// Effects a chain has room for beyond its own (ones fading out after removal).
const FX_SPARE: usize = 4;

/// An effect in a chain, with how much of it is heard: `mix` 1 is the effect alone, 0
/// the signal passing by (bypassed, frozen: it uses no time and keeps its state).
struct Unit {
    fx: FxUnit,
    /// Its slot in the sound's chain.
    slot: usize,
    /// The side-chain source buffers (indices into the engine's `sc_bufs`) it listens
    /// to, if any.
    listen: Vec<usize>,
    mix: f32,
    /// What `mix` is moving to (1 on, 0 off), by `step` for `left` more frames.
    target: f32,
    step: f32,
    left: u32,
    /// Removed from the chain: it plays out its fade, then stays silent until the next
    /// edit drops it.
    ghost: bool,
    /// A plug-in's: its slot as the project's extra outputs name it.
    owner: Option<Owner>,
    /// A plug-in's output pairs (bits, bus 1 and up) with no strip yet, checked for
    /// sound after each block it processes (set when it's made: see [`scan_of`]).
    scan: u64,
    /// A plug-in's pairs that have a strip: what each played over the span, times
    /// `mix`, to be mixed where its strip goes.
    extras: Vec<ExtraRt>,
}

/// A plug-in's extra output pair that has a strip: what it played over the span being
/// rendered (interleaved stereo, `JOB_FRAMES` long; summed, as a chain may run twice in
/// a span), and where it goes (see [`StripRoute`]), gliding.
struct ExtraRt {
    /// Its strip's place in `project.extra_outs` (its meter's too).
    index: usize,
    bus: usize,
    /// What it takes from the pair (a split pair has a strip for each side).
    channels: Channels,
    buf: Vec<f32>,
    route: Route,
    fallback: Route,
    ext: Option<usize>,
    ramp: GainRamp,
}

/// Silence `extras`' first `n2` samples (before a span is rendered into them).
fn clear_strips(extras: &mut [ExtraRt], n2: usize) {
    for x in extras {
        if let Some(b) = x.buf.get_mut(..n2) {
            b.fill(0.0);
        }
    }
}

/// Add what `plugin` just played on each of `extras`' pairs (`n` frames) at `offset`
/// frames into their buffers, times `mix` (`left` frames still moving by `step` to
/// `target`, as [`Unit::fade_to`] moves it).
fn take_strips(plugin: &deckard_plugins::Plugin, extras: &mut [ExtraRt], n: usize, offset: usize, fade: (f32, f32, u32, f32)) {
    for x in extras {
        let Some((l, r)) = plugin.output_bus(x.bus, n) else { continue };
        let (mut mix, step, mut left, target) = fade;
        let Some(buf) = x.buf.get_mut(offset * 2..) else { continue };
        for ((o, a), b) in buf.as_chunks_mut::<2>().0.iter_mut().zip(l).zip(r) {
            if left > 0 {
                mix += step;
                left -= 1;
                if left == 0 {
                    mix = target;
                }
            }
            o[0] += a * mix;
            o[1] += b * mix;
        }
    }
}

/// Take each of `extras`' routes from `routes` (a strip for each of `project.extra_outs`,
/// with its owner, pair and channels): after an edit kept the part that has them, where
/// they are and where they go may have changed. No allocating (the audio thread).
fn refresh_strips(owner: Option<Owner>, extras: &mut [ExtraRt], routes: &[(Owner, usize, Channels, StripRoute)]) {
    let Some(owner) = owner else { return };
    for x in extras {
        match routes.iter().position(|(o, bus, ch, _)| *o == owner && *bus == x.bus && *ch == x.channels) {
            Some(i) => {
                let r = routes[i].3;
                (x.index, x.route, x.fallback, x.ext) = (i, r.route, r.fallback, r.ext);
            }
            None => (x.index, x.route, x.fallback, x.ext) = (usize::MAX, SILENT, SILENT, None),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn route_dst<'a>(
    bus: Option<usize>,
    frames: usize,
    at: usize,
    inp_at: usize,
    out: &'a mut [f32],
    ext: &'a mut [Vec<f32>],
    buses: &'a mut [Option<Bus>],
    bufs: &'a mut [Vec<Option<Box<SoundBufs>>>],
) -> Option<&'a mut [f32]> {
    match bus {
        None => out.get_mut(..frames * 2),
        Some(n) if n >= SOUND_BUS => {
            let (g, s) = decode_sound_bus(n)?;
            let b = bufs.get_mut(g)?.get_mut(s)?.as_deref_mut()?;
            b.inp_on = true;
            b.inp.get_mut(inp_at * 2..(inp_at + frames) * 2)
        }
        Some(n) if n >= GROUP_BUS => buses.get_mut(n - GROUP_BUS)?.as_mut()?.buf.get_mut(..frames * 2),
        Some(n) => ext.get_mut(n)?.get_mut(at..at + frames * 2),
    }
}

/// Mix strip `x`'s span (`frames`) onto where it goes (`out`: the master's input; `ext`
/// from `at`: the other outputs; `buses`: the groups'; `bufs`: the sounds'), gliding to
/// its route, and note its peak in `meters`. `connected`: the outputs (bits) that are
/// connected.
#[allow(clippy::too_many_arguments)]
fn mix_strip(
    x: &mut ExtraRt,
    frames: usize,
    glide: u32,
    connected: u16,
    out: &mut [f32],
    ext: &mut [Vec<f32>],
    at: usize,
    inp_at: usize,
    buses: &mut [Option<Bus>],
    bufs: &mut [Vec<Option<Box<SoundBufs>>>],
    meters: &mut [(f32, f32)],
) {
    let route = match x.ext {
        Some(n) if connected & (1 << n) == 0 => x.fallback,
        _ => x.route,
    };
    x.ramp.aim(route.1, glide);
    if x.ramp.left == 0 && x.ramp.now == (0.0, 0.0) {
        return;
    }
    let Some(dst) = route_dst(route.0, frames, at, inp_at, out, ext, buses, bufs) else {
        for _ in 0..frames {
            x.ramp.next();
        }
        return;
    };
    let mut peak = (0f32, 0f32);
    let channels = x.channels;
    for (o, i) in dst.as_chunks_mut::<2>().0.iter_mut().zip(x.buf.as_chunks::<2>().0) {
        let gain = x.ramp.next();
        // A mono strip's one signal on both sides, before its pan.
        let (l, r) = match channels {
            Channels::Stereo => (i[0], i[1]),
            Channels::Mono => {
                let m = (i[0] + i[1]) * 0.5;
                (m, m)
            }
            Channels::Left => (i[0], i[0]),
            Channels::Right => (i[1], i[1]),
        };
        let (a, z) = (l * gain.0, r * gain.1);
        let (a, z) = (if a.is_finite() { a } else { 0.0 }, if z.is_finite() { z } else { 0.0 });
        o[0] += a;
        o[1] += z;
        peak = (peak.0.max(a.abs()), peak.1.max(z.abs()));
    }
    if let Some(m) = meters.get_mut(x.index) {
        *m = (m.0.max(peak.0), m.1.max(peak.1));
    }
}

impl Unit {
    fn new(fx: FxUnit, slot: usize, listen: Vec<usize>, on: bool) -> Unit {
        let m = if on { 1.0 } else { 0.0 };
        Unit { fx, slot, listen, mix: m, target: m, step: 0.0, left: 0, ghost: false, owner: None, scan: 0, extras: Vec::new() }
    }

    /// Listen for sound on `plugin`'s extra pairs that have no strip in `project`, as the
    /// plug-in `plugin_id` in `owner`'s slot; take what those that have one play.
    fn listen(&mut self, owner: Owner, plugin: &Shared, plugin_id: &str, project: &Project) {
        self.owner = Some(owner);
        self.scan = scan_of(plugin, owner, plugin_id, project);
        self.extras = strips_of(owner, plugin_id, project);
    }

    /// Move to `target` (on 1, off 0) over `frames`, from where it is.
    fn fade_to(&mut self, target: f32, frames: u32) {
        self.target = target;
        if self.mix == target || frames == 0 {
            self.mix = target;
            self.left = 0;
        } else {
            self.left = frames;
            self.step = (target - self.mix) / frames as f32;
        }
    }

    /// What identifies it across edits: the same plug-in instance, or the same class
    /// of Maschine's own effects.
    fn same(&self, other: &Unit) -> bool {
        match (&self.fx, &other.fx) {
            (FxUnit::Plugin { plugin: a, .. }, FxUnit::Plugin { plugin: b, .. }) => Arc::ptr_eq(a, b),
            (FxUnit::Internal { class: a, .. }, FxUnit::Internal { class: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// One effect of a chain: a plug-in, or one of Maschine's own (by class).
enum FxUnit {
    Plugin {
        plugin: Shared,
        params: Vec<(String, f64)>,
        sc_filter: fx::sidechain::SidechainFilter,
    },
    /// `params`: the settings `fx` was made with (an adopted effect takes them);
    /// `glide` takes it to new ones.
    Internal {
        class: String,
        params: Vec<(String, f64)>,
        fx: Box<dyn fx::Effect>,
        glide: glide::Glide,
        /// Modulation: `params` with the modulated values (made with the unit: no
        /// allocating after), whether a lane plays now (`hit`, this span) and whether
        /// the effect has modulated values.
        modded: Vec<(String, f64)>,
        hit: bool,
        modulated: bool,
    },
}

impl FxChain {
    /// Run `l`/`r` (at most [`MAX_BLOCK`] frames) through the effects in place. A busy
    /// plug-in is passed by (the audio thread never waits).
    fn run(&mut self, n: usize, transport: &Transport, bufs: &[Vec<f32>], offset: usize) {
        for unit in self.units.iter_mut() {
            // Bypassed (and not fading): passed by, frozen.
            if unit.mix == 0.0 && unit.left == 0 {
                continue;
            }
            // Fading: keep the dry signal to mix with.
            let fading = unit.left > 0 || unit.mix != 1.0;
            if fading {
                self.dl[..n].copy_from_slice(&self.l[..n]);
                self.dr[..n].copy_from_slice(&self.r[..n]);
            }
            // A metered effect (the Compressor): its input level, before.
            let metered = self.meters.is_some() && matches!(&unit.fx, FxUnit::Internal { fx, .. } if fx.metered());
            let peak = |l: &[f32], r: &[f32]| l.iter().chain(r).fold(0f32, |m, x| m.max(x.abs()));
            let input = if metered { peak(&self.l[..n], &self.r[..n]) } else { 0.0 };
            let u = &mut unit.fx;
            // A side-chain input: the sum of its sources over these frames.
            let sources = unit.listen.as_slice();
            if !sources.is_empty() {
                self.sl[..n].fill(0.0);
                self.sr[..n].fill(0.0);
                for &b in sources {
                    let Some(buf) = bufs.get(b) else { continue };
                    for f in 0..n {
                        if let Some(x) = buf.get((offset + f) * 2..(offset + f) * 2 + 2) {
                            self.sl[f] += x[0];
                            self.sr[f] += x[1];
                        }
                    }
                }
            }
            match u {
                FxUnit::Plugin { plugin: shared, sc_filter, .. } => {
                    if !sources.is_empty() {
                        sc_filter.process(&mut self.sl[..n], &mut self.sr[..n]);
                    }
                    let started = std::time::Instant::now();
                    let Ok(mut p) = shared.lock() else {
                        skipped();
                        continue;
                    };
                    let sc = (!sources.is_empty()).then_some((&self.sl[..n], &self.sr[..n]));
                    let (ol, or) = p.process_input_sc(n, transport, Some((&self.l[..n], &self.r[..n])), sc);
                    if ol.len() >= n && or.len() >= n {
                        self.l[..n].copy_from_slice(&ol[..n]);
                        self.r[..n].copy_from_slice(&or[..n]);
                    }
                    if !unit.extras.is_empty() {
                        take_strips(&p, &mut unit.extras, n, offset, (unit.mix, unit.step, unit.left, unit.target));
                    }
                    if let (Some(owner), Some(board)) = (unit.owner, &self.heard) {
                        hear(&p, owner, unit.scan, n, board);
                    }
                    drop(p);
                    timed(shared, started);
                }
                FxUnit::Internal { fx, glide, .. } => {
                    fx.set_transport(transport.playing.then_some(transport.quarters));
                    // Gliding to new settings: a step of it every glide::STEP frames.
                    let mut at = 0;
                    while at < n {
                        let to = if glide.gliding() { (at + glide::STEP).min(n) } else { n };
                        if glide.tick() {
                            fx.set(glide.now());
                        }
                        let (l, r) = (&mut self.l[at..to], &mut self.r[at..to]);
                        if sources.is_empty() {
                            fx.process(l, r)
                        } else {
                            fx.process_sc(l, r, Some((&self.sl[at..to], &self.sr[at..to])))
                        }
                        at = to;
                    }
                }
            }
            if metered && let (Some((m, g, s)), FxUnit::Internal { fx, .. }) = (&self.meters, &unit.fx) {
                let gr = -20.0 * fx.gain_reduction().max(1e-6).log10();
                m.record(*g, *s, unit.slot, input, gr, peak(&self.l[..n], &self.r[..n]));
            }
            if fading {
                for f in 0..n {
                    if unit.left > 0 {
                        unit.mix += unit.step;
                        unit.left -= 1;
                        if unit.left == 0 {
                            unit.mix = unit.target;
                        }
                    }
                    let m = unit.mix;
                    self.l[f] = self.dl[f] + (self.l[f] - self.dl[f]) * m;
                    self.r[f] = self.dr[f] + (self.r[f] - self.dr[f]) * m;
                }
            }
        }
    }

    /// Whether an effect of the chain listens to a side-chain (it renders after the
    /// sounds that feed it).
    fn listens(&self) -> bool {
        self.units.iter().any(|u| !u.listen.is_empty())
    }

    /// New settings for effects of the chain, by slot: they take them in place (as a
    /// knob turned). What they had goes back in `params`, to be freed elsewhere.
    fn set_params(&mut self, params: &mut [(usize, Vec<(String, f64)>)]) {
        for (slot, new) in params.iter_mut() {
            for u in self.units.iter_mut().filter(|u| u.slot == *slot && !u.ghost) {
                match &mut u.fx {
                    FxUnit::Plugin { params: had, sc_filter, .. } => {
                        std::mem::swap(had, new);
                        sc_filter.set(had);
                    }
                    FxUnit::Internal { params: had, fx, glide, .. } => {
                        std::mem::swap(had, new);
                        fx.set(if glide.retarget(had) { glide.now() } else { had });
                    }
                }
            }
        }
    }

    /// Record this sound's output for the span (`out`: interleaved stereo, after its
    /// effects, before its level) for the side-chains it feeds.
    fn record_from(&self, out: &[f32], bufs: &mut [Vec<f32>]) {
        for &(b, g) in &self.tap {
            let Some(buf) = bufs.get_mut(b) else { continue };
            for (x, y) in buf.as_chunks_mut::<2>().0.iter_mut().zip(out.as_chunks::<2>().0) {
                x[0] = y[0] * g.0;
                x[1] = y[1] * g.1;
            }
        }
    }

    /// Take over `old`'s effects (their state, and how far they are faded) where the
    /// chains match: effects are paired in order (the longest common run of the same
    /// classes or plug-in instances), so removing or adding one leaves the others' state
    /// with them. Effects switched on or off fade; new ones fade in; Maschine's own that
    /// were removed stay (in their place) to fade out, until a later edit drops them.
    /// No allocation: the audio thread runs this.
    fn adopt(&mut self, old: &mut FxChain) {
        const N: usize = 32;
        const NONE: usize = usize::MAX;
        // Old units that take part in pairing (not ones already removed).
        let mut live = [NONE; N];
        let mut n_old = 0;
        for (i, u) in old.units.iter().enumerate().take(N) {
            if !u.ghost {
                live[n_old] = i;
                n_old += 1;
            }
        }
        let n_new = self.units.len().min(N - 1);
        // Longest common subsequence, on the stack.
        let mut table = [[0u8; N]; N];
        for i in (0..n_new).rev() {
            for j in (0..n_old).rev() {
                table[i][j] =
                    if self.units[i].same(&old.units[live[j]]) { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
            }
        }
        // For each old unit (by index), the new one it became.
        let mut became = [NONE; N];
        let mut paired_new = [false; N];
        let (mut i, mut j) = (0, 0);
        let fade = self.fade;
        while i < n_new && j < n_old {
            if self.units[i].same(&old.units[live[j]]) {
                let o = &mut old.units[live[j]];
                let u = &mut self.units[i];
                match (&mut u.fx, &mut o.fx) {
                    (FxUnit::Plugin { params, sc_filter, .. }, FxUnit::Plugin { params: op, sc_filter: osf, .. }) => {
                        std::mem::swap(sc_filter, osf);
                        if params != op {
                            sc_filter.set(params);
                        }
                    }
                    (
                        FxUnit::Internal { params, fx, glide, modulated, .. },
                        FxUnit::Internal { params: op, fx: ofx, glide: og, modulated: om, .. },
                    ) => {
                        std::mem::swap(fx, ofx);
                        std::mem::swap(glide, og);
                        // Modulated values it plays go back to the knob's when nothing holds them.
                        *modulated = *om;
                        if params != op {
                            fx.set(if glide.retarget(params) { glide.now() } else { params });
                        }
                    }
                    _ => {}
                }
                // Its strips' gains glide on from where they are.
                for x in &mut u.extras {
                    if let Some(ox) = o.extras.iter().find(|ox| ox.bus == x.bus && ox.channels == x.channels) {
                        x.ramp = ox.ramp;
                    }
                }
                // Carry on from how far it was faded, toward its new state.
                let target = u.target;
                (u.mix, u.target, u.step, u.left) = (o.mix, o.target, o.step, o.left);
                if target != u.target {
                    u.fade_to(target, fade);
                }
                became[live[j]] = i;
                paired_new[i] = true;
                i += 1;
                j += 1;
            } else if table[i + 1][j] >= table[i][j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
        // New effects fade in.
        for (k, u) in self.units.iter_mut().enumerate() {
            if !paired_new.get(k).copied().unwrap_or(false) && u.target > 0.0 {
                u.mix = 0.0;
                u.fade_to(1.0, fade);
            }
        }
        // Removed ones (Maschine's own, still heard) fade out where they were: after
        // the new effect the one before them became. From the last, so earlier
        // positions stay right.
        for idx in (0..old.units.len().min(N)).rev() {
            let u = &old.units[idx];
            let heard = u.mix > 0.0 || u.left > 0;
            if became[idx] != NONE || !heard || !matches!(u.fx, FxUnit::Internal { .. }) || self.units.len() >= self.units.capacity() {
                continue;
            }
            let at = (0..idx).rev().find_map(|k| (became[k] != NONE).then(|| became[k] + 1)).unwrap_or(0);
            let mut u = old.units.remove(idx);
            u.ghost = true;
            if u.target != 0.0 {
                u.fade_to(0.0, fade);
            }
            self.units.insert(at.min(self.units.len()), u);
        }
    }
}

/// Whether a plug-in (Maschine's id) is an effect.
pub fn is_effect(plugin_id: &str) -> bool {
    plugin_id.contains(":FX:") || plugin_id.contains(":Fx:") || plugin_id.contains(":MF:")
}

/// The effects of a sound's chain: slots after the first that are effects (plug-ins,
/// or Maschine's own), switched on or not.
fn effect_slots(s: &deckard_model::Sound) -> Vec<(usize, &deckard_model::SlotInfo)> {
    // (The first slot too: a sound with no instrument, taking audio in, has effects
    // only.)
    s.slots.iter().enumerate().filter(|(_, x)| !x.is_instrument()).collect()
}

/// A sound played by a plug-in instrument.
struct InstrumentRt {
    plugin: Shared,
    /// Sound and group level/pan combined (0 when muted).
    gain: (f32, f32),
    muted: bool,
    /// Its slot as the project's extra outputs name it, its output pairs with no strip
    /// yet and where they're noted when they make sound (as [`Unit::owner`],
    /// [`Unit::scan`] and [`FxChain::heard`]).
    owner: Option<Owner>,
    scan: u64,
    heard: Option<Arc<meters::HeardBoard>>,
    /// Its pairs that have a strip (as [`Unit::extras`]).
    extras: Vec<ExtraRt>,
}

impl InstrumentRt {
    fn new(plugin: Shared, gain: (f32, f32), muted: bool) -> InstrumentRt {
        InstrumentRt { plugin, gain, muted, owner: None, scan: 0, heard: None, extras: Vec::new() }
    }

    /// As [`Unit::listen`].
    fn listen(&mut self, owner: Owner, plugin: &Shared, plugin_id: &str, project: &Project) {
        self.owner = Some(owner);
        self.scan = scan_of(plugin, owner, plugin_id, project);
        self.extras = strips_of(owner, plugin_id, project);
    }
}

/// What an internal recording takes its audio from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapSource {
    /// The master output (after its level and pan).
    Master,
    /// One group's output: its sounds mixed.
    Group(usize),
    /// One sound's output.
    Sound(usize, usize),
}

/// Audio taken from inside the engine for recording: what to take, and where to put
/// it (interleaved stereo; the recorder empties it).
#[derive(Clone)]
pub struct Tap {
    pub source: TapSource,
    pub buffer: Arc<Mutex<Vec<f32>>>,
}

/// Whether sound `s` of group `g` goes to `tap`.
fn taps(tap: Option<&Tap>, g: usize, s: usize) -> bool {
    match tap.map(|t| t.source) {
        Some(TapSource::Group(tg)) => tg == g,
        Some(TapSource::Sound(tg, ts)) => (tg, ts) == (g, s),
        _ => false,
    }
}

/// Plug-in blocks the audio thread skipped (an instance whose mutex was poisoned by a
/// panic; each one a gap in that plug-in's sound). For diagnostics.
pub static PLUGIN_SKIPS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The slowest single plug-in call on the audio thread since [`take_slowest`] was last
/// called: (µs, the instance, as `Arc::as_ptr` of its `Shared`). For diagnostics.
static SLOWEST_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SLOWEST_WHO: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn timed(who: &Shared, started: std::time::Instant) {
    use std::sync::atomic::Ordering::Relaxed;
    let us = started.elapsed().as_micros() as u64;
    if us > SLOWEST_US.load(Relaxed) {
        SLOWEST_US.store(us, Relaxed);
        SLOWEST_WHO.store(Arc::as_ptr(who) as usize, Relaxed);
    }
}

/// The slowest plug-in call since the last time (µs, and which instance: compare
/// with `Arc::as_ptr` of the host's instances), then start over.
pub fn take_slowest() -> (u64, usize) {
    use std::sync::atomic::Ordering::Relaxed;
    (SLOWEST_US.swap(0, Relaxed), SLOWEST_WHO.swap(0, Relaxed))
}

fn skipped() {
    PLUGIN_SKIPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// What the UI shows of a playing engine, published by the audio thread after each
/// block (see [`Engine::publish`]) so the UI never holds the engine to read it.
/// A live note on (`velocity` > 0) or release (0) while recording: the song position
/// (ticks in what played, `mode`) it played at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recorded {
    pub group: usize,
    pub sound: usize,
    pub pitch: i64,
    pub velocity: i64,
    /// Ticks it lasts when played with a length (a pad hit); `i64::MAX` held until
    /// its release.
    pub length: i64,
    pub tick: f64,
    pub mode: Mode,
    /// Its place among everything recorded (notes, expression, controls), which the
    /// three streams lose when taken apart.
    pub seq: u64,
    /// Ticks on a clock that never wraps (loops and jumps don't move it back): what a
    /// held note's length is measured by.
    pub played: f64,
    /// It came in over MIDI (not from a pad, Note Repeat or the Arp).
    pub midi: bool,
}

/// Per-note expression played while recording, at the song position it came.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordedExpr {
    pub group: usize,
    pub sound: usize,
    pub pitch: i64,
    pub kind: ExprKind,
    pub value: f32,
    pub tick: f64,
    pub mode: Mode,
    pub seq: u64,
    /// See [`Recorded::played`].
    pub played: f64,
}

/// A channel-wide controller played while recording.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordedControl {
    pub group: usize,
    pub sound: usize,
    pub kind: MidiKind,
    pub tick: f64,
    pub mode: Mode,
    pub seq: u64,
}

/// A count-in's last stretch (ticks, a 1/16) whose live notes are recorded, at the start.
const PICKUP: i64 = 240;

/// The most recorded events the audio thread keeps for the UI between its looks.
const RECORDED_ROOM: usize = 4096;

#[derive(Debug, Clone, Default)]
pub struct Telemetry {
    /// Peak (left, right) of each sound since the UI last took them, `[group][sound]`.
    pub meters: Vec<Vec<(f32, f32)>>,
    pub master_peak: (f32, f32),
    /// Peak (left, right) of the cue bus since the UI last took it.
    pub cue_peak: (f32, f32),
    /// Ticks.
    pub position: f64,
    pub length: f64,
    pub sequencing: bool,
    pub mode: Option<Mode>,
    pub queued: Option<Mode>,
    pub finished: bool,
    pub active_voices: usize,
    /// Zones whose play position the UI wants, (group, sound, zone), and where each
    /// is playing.
    pub watch: Vec<(usize, usize, usize)>,
    pub zones: Vec<Option<usize>>,
    /// The last note played live: how many so far, then its group, sound, pitch and
    /// velocity. (The Sampler's pages show the zone it lands in.)
    pub live_hit: (u64, usize, usize, i64, i64),
    /// How many commands the audio thread had applied when this was published (see
    /// `Output::is_current`).
    pub actions: u64,
    /// An engine is playing.
    pub has_engine: bool,
    /// A jump queued within the timeline: at tick `.0` to tick `.1`.
    pub pending_jump: Option<(i64, i64)>,
    /// Frames the recording tap has produced.
    pub tap_frames: u64,
    /// Live notes recorded since the UI last took them.
    pub recorded: Vec<Recorded>,
    pub recorded_expr: Vec<RecordedExpr>,
    pub recorded_controls: Vec<RecordedControl>,
    /// [`Engine::played`] after the block.
    pub played: f64,
    pub tempo: f64,
    /// The last edit didn't reach the timeline (see [`Engine::stale`]).
    pub stale: bool,
    /// Sounds rendered ahead now, and blocks where one wasn't ready in time.
    pub sounds_ahead: usize,
    pub ahead_underruns: u64,
    /// Plug-ins' extra output pairs with no strip yet that made sound since the UI last
    /// took them: each once, in mixer order (see [`heard_rank`]), at most [`HEARD_ROOM`].
    pub heard: Vec<Heard>,
    /// Peak (left, right) of each extra output's strip since the UI last took them, as
    /// `project.extra_outs` lists them (after the strip's level and pan).
    pub extra_meters: Vec<(f32, f32)>,
}

/// Strips whose meters [`Engine::publish`] hands over without allocating (more: once,
/// when there come to be more).
pub const EXTRA_METERS_ROOM: usize = 64;

impl Telemetry {
    /// Telemetry with room made up front for what the audio thread hands over, so
    /// [`Engine::publish`] doesn't allocate for it.
    pub fn with_room() -> Telemetry {
        Telemetry { heard: Vec::with_capacity(HEARD_ROOM), extra_meters: Vec::with_capacity(EXTRA_METERS_ROOM), ..Default::default() }
    }
}

/// A plug-in's extra output pair (`bus`, 1 and up) that made sound, in `owner`'s slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heard {
    pub owner: Owner,
    pub bus: usize,
}

/// The most [`Heard`] pairs handed over at once (more wait until they play again).
pub const HEARD_ROOM: usize = 32;

/// A peak above this (-90 dBFS, linear) on either side in a block is sound.
const HEARD_FLOOR: f32 = 3.1623e-5;

/// Where a [`Heard`] comes in the mixer: by group (a group's own slots before its
/// sounds, the master's last), sound, slot (a sound's instrument first) and bus.
fn heard_rank(h: &Heard) -> (usize, usize, usize, usize) {
    match h.owner {
        Owner::Group(g, k) => (g, 0, k, h.bus),
        Owner::Sound(g, s) => (g, s + 1, 0, h.bus),
        Owner::Chain(g, s, k) => (g, s + 1, k + 1, h.bus),
        Owner::Master(k) => (usize::MAX, 0, k, h.bus),
    }
}

/// The slot an extra output's owner names for a plug-in slot.
fn owner_of(slot: Slot) -> Owner {
    match slot {
        Slot::Sound { group, sound } => Owner::Sound(group, sound),
        Slot::Chain { group, sound, slot } => Owner::Chain(group, sound, slot),
        Slot::Group { group, slot } => Owner::Group(group, slot),
        Slot::Master { slot } => Owner::Master(slot),
    }
}

/// The output pairs (bits, bus 1 and up, below 64) of `plugin`, as the plug-in
/// `plugin_id` in `owner`'s slot, that have no strip in `project`: the ones to listen
/// to. 0 for a plug-in with one pair. (Off the audio thread: it locks the plug-in.)
fn scan_of(plugin: &Shared, owner: Owner, plugin_id: &str, project: &Project) -> u64 {
    let count = plugin.lock().map_or(1, |p| p.output_count()).min(64);
    let mut bits = (1..count).fold(0u64, |b, bus| b | 1 << bus);
    for x in project.extra_outs.iter().filter(|x| x.owner == owner && x.plugin == plugin_id && x.bus < 64) {
        bits &= !(1 << x.bus);
    }
    bits
}

/// Note on `board` each of `plugin`'s pairs in `scan` that made sound over its last
/// `n` frames. No allocating or locking (the audio thread, or a sound's renderer).
fn hear(plugin: &deckard_plugins::Plugin, owner: Owner, scan: u64, n: usize, board: &meters::HeardBoard) {
    let mut bits = scan;
    while bits != 0 {
        let bus = bits.trailing_zeros() as usize;
        bits &= bits - 1;
        if let Some((l, r)) = plugin.output_bus(bus, n)
            && l.iter().chain(r).any(|x| x.abs() > HEARD_FLOOR)
        {
            board.hit(owner, bus);
        }
    }
}

/// A note length meaning "until released" (see [`Engine::note_hold`]).
const HELD: i64 = i64::MAX;

pub struct Engine {
    sr: f64,
    /// What it plays at now: the song's tempo, a clock's, or the playing scene's own.
    tempo: f64,
    /// The song's tempo.
    song_tempo: f64,
    /// A clock sets the tempo (see [`Engine::set_followed`]).
    followed: bool,
    timeline: Timeline,
    mode: Mode,
    /// What to play when the current pass ends (a scene launched while playing).
    queued: Option<(Mode, Timeline)>,
    /// A jump within the timeline: at tick `.0`, continue from tick `.1` (a section
    /// picked while the song plays).
    jump: Option<(i64, i64)>,
    samples: Vec<Arc<Sample>>,
    /// [group][sound]: sounds played by the Sampler.
    sounds: Vec<Vec<Option<SamplerRt>>>,
    /// Samplers of sounds that went away (muted, removed) while their voices fade.
    retired: Vec<SamplerRt>,
    /// [group][sound]: sounds played by plug-ins.
    instruments: Vec<Vec<Option<InstrumentRt>>>,
    /// Pending note-offs: (at `time`, group, sound, pitch, note id).
    note_offs: Vec<(u64, usize, usize, i64, u64)>,
    /// Live events to play at output frame `.0` (of `time`), in the order they came.
    live: Vec<(u64, Live)>,
    /// The timeline couldn't follow the last edit (no timeline for the mode it plays
    /// was sent): the edit should be sent again.
    pub stale: bool,
    /// Output frames since the engine started (never resets).
    time: u64,
    /// Frames since a plug-in last made sound.
    quiet: u64,
    numerator: i32,
    denominator: i32,
    /// Output frames since the timeline (or the current loop pass) started.
    frame: u64,
    next: usize,
    pub sequencing: bool,
    pub looping: bool,
    /// The metronome clicks on the beats while the sequencer runs.
    pub metronome: bool,
    /// The metronome clicks while recording, on or not (the Settings page's AUTOENABLE).
    pub metronome_auto: bool,
    /// The note value it clicks on (8: eighths; the Settings page's TIMESIGNATURE); 0
    /// the song's beats.
    pub metronome_ticks: u32,
    click: metronome::Click,
    prehear: prehear::Prehear,
    /// A count-in: frames still to click before the song plays, and how far into it.
    counting: u64,
    count_frame: u64,
    /// Note Repeat / Arp: held live notes feed it; it plays on the rate's grid.
    repeater: Option<repeat::Repeater>,
    repeat_buf: Vec<repeat::Fired>,
    /// MIDI input plug-ins, a slot per group, and what one just played.
    midi_inputs: Vec<Option<Box<dyn midi_input::MidiInput>>>,
    midi_input_out: Vec<midi_input::NoteOut>,
    /// The slots that had a note since they last settled (a bit each).
    midi_inputs_unsettled: u64,
    /// A plug-in's note is playing: it doesn't go back in.
    from_midi_input: bool,
    /// Recording (REC): live notes are noted with their song position.
    recording: bool,
    recorded: Vec<Recorded>,
    recorded_expr: Vec<RecordedExpr>,
    recorded_controls: Vec<RecordedControl>,
    /// The next recorded event's `seq`.
    record_seq: u64,
    /// The master channel's level (linear) and pan gains.
    pub master: f32,
    master_pan: (f32, f32),
    /// The external output the master plays on (Out n, from 0), if any.
    master_bus: Option<usize>,
    /// [group][sound]: the external output a sound plays on when it isn't the master's
    /// ([`sound_bus`]).
    routes: Vec<Vec<Option<usize>>>,
    /// The cue bus.
    cue: CueBus,
    /// The faders with modulation, and which are on each sound's way out ([group][sound])
    /// and each group bus's ([`channel_faders`]).
    faders: Vec<Fader>,
    sound_faders: Vec<Vec<Vec<usize>>>,
    bus_faders: Vec<Vec<usize>>,
    /// Peak output level (left, right) since the last `take_master_peak`.
    master_peak: (f32, f32),
    /// The last note id given out.
    clock: u64,
    /// For note probability (a fixed seed, so renders repeat).
    rng: u64,
    /// Passes through the timeline so far (it loops, or switches mode): with the event's
    /// index, what decides a note's probability, whenever it is decided.
    pass: u64,
    /// Every timeline event fired: (output frame, tick, sound, pitch, plays). Tests
    /// only.
    #[cfg(test)]
    fired: Vec<(u64, i64, usize, i64, bool)>,
    /// Peak output level (left, right) of each sound, [group][sound], since the last
    /// `take_meters`.
    meters: Vec<Vec<(f32, f32)>>,
    /// An internal recording's tap, and what it collected that the recorder hasn't
    /// taken yet (when its buffer was busy).
    tap: Option<Tap>,
    tap_pending: Vec<f32>,
    /// Frames the tap has produced since it was set.
    tap_total: u64,
    /// [`Telemetry::live_hit`].
    live_hit: (u64, usize, usize, i64, i64),
    /// Scratch for rendering a tapped sound, and the tapped audio of the current block.
    /// Each sound's output buffers, `[group][sound]` (sounds with an instrument).
    bufs: Vec<Vec<Option<Box<SoundBufs>>>>,
    /// The sounds of the pass being rendered, and their parts (room made up front).
    job_list: Vec<(usize, usize)>,
    job_ptrs: Vec<JobPtrs>,
    /// Threads that render sounds alongside the audio thread (none: all on it).
    pub pool: Option<Arc<pool::RenderPool>>,
    /// Rendering ahead, when on.
    ahead: Option<Ahead>,
    /// Sounds that must play live (audio or MIDI input, side-chains, MIDI routes
    /// between sounds), `[group][sound]`.
    live_hint: Vec<Vec<bool>>,
    /// The event being played comes from the timeline (not live input).
    from_timeline: bool,
    /// The group that plays live (pads, keys): its sounds aren't rendered ahead.
    focus: Option<usize>,
    /// An edit's engine: per sound, what to keep of the playing one's (see
    /// [`Engine::new_after`]); and whether to keep its timelines.
    reuse: Vec<Vec<Option<Reuse>>>,
    keep_timeline: bool,
    group_swaps: Vec<(usize, usize)>,
    tap_block: Vec<f32>,
    /// Sounds' effect chains, `[group][sound]`.
    fx: Vec<Vec<Option<FxChain>>>,
    /// Effects have modulated values (to give back their own when the lanes stop).
    modulating: bool,
    /// Groups with effects: their buses (`[group]`), and the order they run in.
    buses: Vec<Option<Bus>>,
    bus_order: Vec<usize>,
    sound_aux: Vec<Vec<Vec<aux::Send>>>,
    group_aux: Vec<Vec<aux::Send>>,
    /// The master's effects, run on its output before its level.
    master_fx: Option<FxChain>,
    /// What sounds feeding side-chains played over the span being rendered
    /// (interleaved stereo, one buffer each).
    sc_bufs: Vec<Vec<f32>>,
    /// Each sound's MIDI output, and which sounds are muted, `[group][sound]`.
    midi_routes: Vec<Vec<Option<midi::OutRoute>>>,
    sound_muted: Vec<Vec<bool>>,
    /// Pad mode's choke and link groups, for live notes (the timeline has them already).
    pads: pads::Pads,
    /// Where MIDI for output ports goes (a thread that sends it); none: dropped.
    pub midi_tx: Option<std::sync::mpsc::SyncSender<midi::MidiOut>>,
    /// The notes sent to MIDI ports and not ended yet.
    midi_offs: Vec<MidiNote>,
    /// Output ports set to MPE, by name ([`Engine::set_mpe_out`]).
    mpe: std::collections::HashMap<String, midi::MpePort>,
    /// Live events are being played together (their MPE notes wait for their starting
    /// values).
    batching: bool,
    /// Sounding notes' expression, for the modulation sources ([`mod_rt`]).
    touch: Vec<mod_rt::ExprVoice>,
    held_keys: Vec<mod_rt::HeldKey>,
    touch_order: u64,
    /// The sounds' modulation routes ([`mod_rt`]).
    mod_routes: Vec<mod_rt::TargetRt>,
    /// Plug-in parameters modulated by pattern lanes (`(plugin, id, hit_this_span)`).
    modded_plugins: Vec<(Shared, u32, bool)>,
    /// Plug-in parameters routes let go of while the plug-in was busy ([`mod_rt`]).
    unrouted: Vec<(Shared, u32)>,
    /// Where the routes' travels are published, for their knobs.
    route_board: Option<Arc<meters::RouteTravels>>,
    /// The project's LFOs ([`lfo_rt`]).
    lfos: Vec<lfo_rt::LfoRt>,
    /// The project's CC routes ([`lfo_rt::CcRt`]).
    cc_routes: Vec<lfo_rt::CcRt>,
    /// Ticks before the timeline's start on the song's clock (an export of later
    /// sections), for synced LFOs.
    pub song_offset: f64,
    /// The LFOs' clock ([`lfo_rt::LfoClock`]); kept across edits.
    lfo_clock: lfo_rt::LfoClock,
    /// Sounds a solo left out ([`Engine::retain_sounds`]), `[group][sound]`: their
    /// notes don't trigger LFOs.
    silent: Vec<Vec<bool>>,
    /// Audio coming in (Maschine's 32 input channels, In 1 L … In 16 R, per frame),
    /// and the sounds that take it: (group, sound, left and right input channel,
    /// gain).
    pub live_in: Option<LiveIn>,
    /// Where plug-ins note their extra pairs making sound (its [`SampleCache`]'s), and
    /// what [`Engine::publish`] took from it last (`heard_len` of them).
    heard_board: Arc<meters::HeardBoard>,
    heard: [Heard; HEARD_ROOM],
    heard_len: usize,
    /// Each of `project.extra_outs`' strips: its owner, pair and route (the parts an edit
    /// keeps take theirs from here, [`Engine::refresh_strips`]); and its peak since the
    /// last [`Engine::publish`].
    strips: Vec<(Owner, usize, Channels, StripRoute)>,
    meters_extra: Vec<(f32, f32)>,
    /// The external outputs (bits, Out 1 = bit 0) the interface has connected: a strip
    /// onto one that isn't plays through its parent. All, unless told otherwise.
    connected: u16,
    in_routes: Vec<InRoute>,
    /// Where in the span the sub-span being rendered starts (frames): where its sounds'
    /// input is.
    inp_at: usize,
    /// Sounds played by an Audio plug-in (group, sound): their loops are recued when the
    /// song moves ([`Engine::seek`]).
    audio_sounds: Vec<(usize, usize)>,
    /// Following an outside clock and sending MIDI clock ([`sync`]).
    sync: sync::Sync,
}

/// A note sent to a MIDI port, until its note-off: where it went, and its recorded
/// expression playing out with it ([`Engine::play_expression`]).
#[derive(Debug, Clone)]
struct MidiNote {
    /// When its note-off is due (`time`); `u64::MAX`: when the held note is released.
    at: u64,
    port: Arc<str>,
    /// 0–15.
    channel: u8,
    note: u8,
    /// The note it plays: group, sound, pitch.
    held: (usize, usize, i64),
    /// The note-off's velocity.
    release: u8,
    /// On a member channel of an MPE port (`channel` is its nibble).
    mpe: bool,
    /// A live MPE note whose note-on (this velocity) waits for its starting values
    /// ([`Engine::flush_waiting`]).
    waiting: Option<u8>,
    expr: Option<Arc<deckard_model::Expression>>,
    /// When it started (`time`), and ticks per sample then: where it is in its curves.
    started: u64,
    tps: f64,
    /// The value last sent for each dimension ([`Dim::ALL`]'s order), and when.
    sent: [Option<u16>; 3],
    sent_at: [u64; 3],
}

/// The message for a note's expression dimension `d` at `x` (semitones for pitch, else
/// 0–1): on an MPE port (member bend range `range`) on the note's channel `ch`; on a
/// normal port only pressure, as poly aftertouch on `note`. (The value as sent, the
/// bytes, their length.)
fn expr_message(d: Dim, x: f32, range: Option<u8>, ch: u8, note: u8) -> Option<(u16, [u8; 3], u8)> {
    let unit = |x: f32| (x.clamp(0.0, 1.0) * 127.0).round() as u16;
    Some(match (d, range) {
        (Dim::Pitch, Some(r)) => {
            let v = midi::bend_14(x, r);
            (v, [0xe0 | ch, (v & 0x7f) as u8, (v >> 7) as u8], 3)
        }
        (Dim::Slide, Some(_)) => (unit(x), [0xb0 | ch, 74, unit(x) as u8], 3),
        (Dim::Pressure, Some(_)) => (unit(x), [0xd0 | ch, unit(x) as u8, 0], 2),
        (Dim::Pressure, None) => (unit(x), [0xa0 | ch, note, unit(x) as u8], 3),
        (Dim::Pitch | Dim::Slide, None) => return None,
    })
}

/// How often a note's expression may send a dimension (5 ms).
const EXPR_STEP_SECS: f64 = 0.005;

/// A sound taking an external input: (group, sound, left channel, right channel, gain).
type InRoute = (usize, usize, usize, usize, (f32, f32));

/// Frames of interface input, routed to Maschine's 32 input channels.
pub type LiveIn = Arc<Mutex<std::collections::VecDeque<[f32; 32]>>>;

/// Sounds taking an external input: (group, sound, left channel, right channel, input
/// gain). (The sound's fader applies after its effects, as its Sampler's does.)
fn input_routes(p: &Project) -> Vec<InRoute> {
    let mut out = Vec::new();
    for (g, grp) in p.groups.iter().enumerate() {
        for (s, snd) in grp.sounds.iter().enumerate() {
            let deckard_model::AudioSource::Ext(n) = snd.channel.audio_in else { continue };
            // A chain starting with an instrument plays that, not the input.
            if muting(p, g, s).silent || snd.slots.first().is_some_and(|x| x.is_instrument()) {
                continue;
            }
            let n = n.max(0) as usize;
            let (l, r) = if n < 16 { (2 * n, 2 * n + 1) } else { (n - 16, n - 16) };
            if r >= 32 {
                continue;
            }
            let gain = deckard_model::level_gain(snd.channel.input_gain) as f32;
            out.push((g, s, l, r, (gain, gain)));
        }
    }
    out
}

/// Samples shared between engines (e.g. replaying another scene without reloading).
#[derive(Default)]
pub struct SampleCache {
    by_path: HashMap<std::path::PathBuf, Option<Arc<Sample>>>,
    /// Library folders added in Preferences, searched with the installed ones.
    pub library_dirs: Vec<std::path::PathBuf>,
    /// Where samples were found (by how the project refers to them, and the project's
    /// folder): each edit doesn't search the disk again. Only files found are kept.
    found: HashMap<(i64, String, String, String, Option<std::path::PathBuf>), std::path::PathBuf>,
    /// Plug-in instances, kept across engines (they are slow to load and hold state).
    pub plugins: deckard_plugins::Host,
    /// Where the engines made with it note plug-ins' extra pairs making sound (one board,
    /// so parts an edit keeps and parts it makes anew note on the same).
    heard: Arc<meters::HeardBoard>,
    /// Tests: plug-ins (made by the test) that play these slots, whatever the project
    /// names.
    #[cfg(test)]
    test_plugins: Vec<(Slot, Shared)>,
}

impl SampleCache {
    /// The instance for `slot` (see [`deckard_plugins::Host::instance`]).
    fn plugin(&mut self, slot: Slot, plugin_id: &str, name: &str, chunk: &[u8], sample_rate: f64) -> Result<Shared, String> {
        #[cfg(test)]
        if let Some((_, p)) = self.test_plugins.iter().find(|(s, _)| *s == slot) {
            return Ok(p.clone());
        }
        self.plugins.instance(slot, plugin_id, name, chunk, sample_rate)
    }

    /// The instance already playing `slot` (see [`deckard_plugins::Host::loaded`]).
    fn loaded_plugin(&self, slot: Slot, plugin_id: &str, name: &str) -> Option<Shared> {
        #[cfg(test)]
        if let Some((_, p)) = self.test_plugins.iter().find(|(s, _)| *s == slot) {
            return Some(p.clone());
        }
        self.plugins.loaded(slot, plugin_id, name)
    }

    /// The sample `sample` (as a project refers to it) decoded, loading it if needed.
    pub fn sample(&mut self, sample: &deckard_model::SampleRef, project_path: Option<&Path>) -> Option<Arc<Sample>> {
        let mut resolver = library::Resolver::new(project_path);
        resolver.set_extra(&self.library_dirs);
        let path = resolver.resolve(sample)?;
        self.by_path
            .entry(path.clone())
            .or_insert_with(|| Sample::load(&path).ok().filter(|s| !s.frames.is_empty() && s.rate > 0).map(Arc::new))
            .clone()
    }
}

/// What loading found.
#[derive(Debug, Default, Clone)]
pub struct LoadReport {
    pub zones: usize,
    pub loaded: usize,
    pub missing: Vec<String>,
    pub failed: Vec<String>,
    /// "group / sound (instrument)" for sounds that have notes but nothing to play
    /// them (drum synths, plug-ins that failed to load).
    pub no_sampler: Vec<String>,
    /// Plug-in instruments playing ("group / sound: plug-in").
    pub plugins: Vec<String>,
    /// Plug-ins that could not be loaded, and why.
    pub plugin_errors: Vec<String>,
    /// Which sounds have an effect chain, `[group][sound]` (for [`Engine::new_after`]).
    pub chains: Vec<Vec<bool>>,
}

impl Engine {
    /// Prepare `project` (loaded from `project_path`) for playback at `sr` Hz.
    pub fn new(project: &Project, project_path: Option<&Path>, sr: u32, mode: Mode, cache: &mut SampleCache) -> (Engine, LoadReport) {
        Engine::new_after(project, None, project_path, sr, mode, cache)
    }

    /// An engine for `project`, to be adopted ([`Engine::adopt`]) by the one playing
    /// `prev.0` (`prev.1`: its load report's `chains`): only what an edit changed is
    /// built. Sounds keep their Sampler, plug-in or effect chain when the edit didn't
    /// touch them (an effect's or the Sampler's settings are changed in place); the
    /// timeline, gains and routing are always made new. Without `prev` (or when the
    /// project's shape or its side-chains changed), everything is built.
    pub fn new_after(
        project: &Project,
        prev: Option<(&Project, &[Vec<bool>])>,
        project_path: Option<&Path>,
        sr: u32,
        mode: Mode,
        cache: &mut SampleCache,
    ) -> (Engine, LoadReport) {
        let mut report = LoadReport::default();
        let mut resolver = library::Resolver::new(project_path);
        resolver.set_extra(&cache.library_dirs);
        let mut samples: Vec<Arc<Sample>> = Vec::new();
        let mut index: HashMap<*const Sample, usize> = HashMap::new();
        let sr_f = sr as f64;
        // Effect chains first: a sound with effects plays at unity into them, and its
        // level and pan come after.
        // Side-chains: which sounds feed one (a buffer each), and what each effect hears.
        // As Maschine taps them (docs/fx-*.md, null-tested): a sound before its fader
        // (whatever its output); a group as the sum of the sounds going into it, each
        // after its fader. Keys: (group, sound, after the fader).
        let sc_index = sidechain_index(project);
        let source_sounds = |src| sidechain_sources(project, src);
        // The side-chain buffers the effect in chain slot `slot` of sound (`gi`, `si`) hears.
        let hears = |gi: usize, si: usize, slot: usize| -> Vec<usize> {
            project.groups[gi].sounds[si]
                .channel
                .sidechain
                .iter()
                .filter(|(k, _)| *k == slot)
                .flat_map(|&(_, src)| source_sounds(src))
                .filter(|&(g, s, _)| (g, s) != (gi, si))
                .filter_map(|k| sc_index.get(&k).copied())
                .collect()
        };
        // The side-chain buffers the effect in chain slot `slot` of group `gi` hears.
        let hears_group = |gi: usize, slot: usize| -> Vec<usize> {
            project.groups[gi]
                .channel
                .sidechain
                .iter()
                .filter(|(k, src)| *k == slot && *src != deckard_model::ScSource::Group(gi))
                .flat_map(|&(_, src)| source_sounds(src))
                .filter_map(|k| sc_index.get(&k).copied())
                .collect()
        };
        // What the playing engine can keep: only when the project's shape and its
        // side-chains are the same.
        let same_shape = |a: &Project| {
            a.groups.len() == project.groups.len() && a.groups.iter().zip(&project.groups).all(|(x, y)| x.sounds.len() == y.sounds.len())
        };
        let prev = prev.filter(|(p, chains)| {
            same_shape(p)
                && chains.len() == project.groups.len()
                && chains.iter().zip(&project.groups).all(|(c, g)| c.len() == g.sounds.len())
                && sidechain_index(p) == sc_index
        });
        let mut reuse: Vec<Vec<Option<Reuse>>> = project
            .groups
            .iter()
            .enumerate()
            .map(|(g, grp)| {
                (0..grp.sounds.len())
                    .map(|s| {
                        let (p, _) = prev?;
                        let mut r = reuse_of(p, project, g, s, sr_f, &cache.plugins);
                        // A sound whose level feeds a side-chain (its group's sum)
                        // has that in its chain.
                        let (a, b) = (&p.groups[g].sounds[s], &grp.sounds[s]);
                        if sc_index.contains_key(&(g, s, true)) && (a.level, a.pan) != (b.level, b.pan) {
                            r.chain = false;
                            r.params.clear();
                        }
                        Some(r)
                    })
                    .collect()
            })
            .collect();
        let kept =
            |g: usize, s: usize, part: fn(&Reuse) -> bool| reuse.get(g).and_then(|r| r.get(s)).and_then(Option::as_ref).is_some_and(part);
        let mut fx: Vec<Vec<Option<FxChain>>> = Vec::new();
        for (gi, g) in project.groups.iter().enumerate() {
            let mut row = Vec::new();
            for (si, s) in g.sounds.iter().enumerate() {
                if kept(gi, si, |r| r.chain) {
                    row.push(None);
                    continue;
                }
                let slots = effect_slots(s);
                let mut units = Vec::with_capacity(slots.len() + FX_SPARE);
                for (slot, info) in slots {
                    let hears = hears(gi, si, slot);
                    let key = Slot::Chain { group: gi, sound: si, slot };
                    match &info.plugin {
                        // A bypassed plug-in stays only if it is loaded already (to
                        // fade back in); none is loaded just to sit bypassed.
                        Some(pr) if !info.active => {
                            if let Some(p) = cache.loaded_plugin(key, &pr.id, &pr.name) {
                                let mut u = Unit::new(plugin_unit(p.clone(), &info.params, sr_f), slot, Vec::new(), false);
                                u.listen(owner_of(key), &p, &pr.id, project);
                                units.push(u);
                            }
                        }
                        Some(pr) => match cache.plugin(key, &pr.id, &pr.name, &pr.chunk, sr_f) {
                            Ok(p) => {
                                report.plugins.push(format!("{} / {}: {}", g.name, s.name, pr.name));
                                let mut u = Unit::new(plugin_unit(p.clone(), &info.params, sr_f), slot, hears, true);
                                u.listen(owner_of(key), &p, &pr.id, project);
                                units.push(u);
                            }
                            Err(e) => report.plugin_errors.push(e),
                        },
                        None => {
                            // Maschine's own effects that DECKARD renders (others pass).
                            if let Some(mut fx) = fx::make(&info.module, &info.params, sr_f) {
                                fx.set_tempo(project.tempo);
                                let glide = glide::Glide::new(&info.params, sr_f);
                                let unit = FxUnit::Internal {
                                    class: info.module.clone(),
                                    params: info.params.clone(),
                                    fx,
                                    glide,
                                    modded: info.params.clone(),
                                    hit: false,
                                    modulated: false,
                                };
                                units.push(Unit::new(unit, slot, if info.active { hears } else { Vec::new() }, info.active));
                            }
                        }
                    }
                }
                let m = muting(project, gi, si);
                let mut tap = Vec::new();
                if let Some(&b) = sc_index.get(&(gi, si, false)) {
                    tap.push((b, (1.0, 1.0)));
                }
                if let Some(&b) = sc_index.get(&(gi, si, true)) {
                    tap.push((b, channel_gain(s.level, s.pan)));
                }
                row.push((!units.is_empty() || !tap.is_empty()).then(|| FxChain {
                    units,
                    gain: if m.silent || m.hushed { (0.0, 0.0) } else { sound_gain(project, gi, si) },
                    l: vec![0.0; MAX_BLOCK],
                    r: vec![0.0; MAX_BLOCK],
                    dl: vec![0.0; MAX_BLOCK],
                    dr: vec![0.0; MAX_BLOCK],
                    fade: (FX_FADE_SECONDS * sr_f).round() as u32,
                    meters: None,
                    tap,
                    sl: vec![0.0; MAX_BLOCK],
                    sr: vec![0.0; MAX_BLOCK],
                    heard: Some(cache.heard.clone()),
                }));
            }
            fx.push(row);
        }
        let had_chain = |g: usize, s: usize| prev.is_some_and(|(_, c)| c[g][s]);
        let has_fx = |g: usize, s: usize| {
            fx.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some) || (kept(g, s, |r| r.chain) && had_chain(g, s))
        };
        // A sample a zone or take names: found (remembered), loaded once, counted.
        let mut load = |r: &deckard_model::SampleRef| -> Option<Arc<Sample>> {
            let key = (r.kind, r.path.clone(), r.library.clone(), r.library_root.clone(), project_path.map(Path::to_path_buf));
            let path = match cache.found.get(&key) {
                Some(p) => Some(p.clone()),
                None => resolver.resolve(r).inspect(|p| {
                    cache.found.insert(key, p.clone());
                }),
            };
            let Some(path) = path else {
                report.missing.push(format!("{} ({})", r.path, r.library));
                return None;
            };
            let entry = cache.by_path.entry(path.clone()).or_insert_with(|| match Sample::load(&path) {
                Ok(s) if !s.frames.is_empty() && s.rate > 0 => Some(Arc::new(s)),
                Ok(_) => None,
                Err(e) => {
                    report.failed.push(format!("{}: {e}", path.display()));
                    None
                }
            });
            let sample = entry.clone()?;
            report.loaded += 1;
            index.entry(Arc::as_ptr(&sample)).or_insert_with(|| {
                samples.push(sample.clone());
                samples.len() - 1
            });
            Some(sample)
        };
        let sounds: Vec<Vec<Option<SamplerRt>>> = project
            .groups
            .iter()
            .enumerate()
            .map(|(gi, g)| {
                g.sounds
                    .iter()
                    .enumerate()
                    .map(|(si, s)| {
                        if kept(gi, si, |r| r.sampler) {
                            return None;
                        }
                        // A muted sound plays no new notes but lets its notes ring out
                        // (a trigger mute), unless it's an audio mute (Audio Mute, or a
                        // muted group); unless it plays on for its strips ([`muting`]).
                        if muting(project, gi, si).silent {
                            return None;
                        }
                        if let Some((class, params)) = synth_of(s) {
                            return SamplerRt::synth(class, params, (1.0, 1.0), sr_f);
                        }
                        // The Audio plug-in: its takes (a missing one silent, so the
                        // patterns' take numbers still match).
                        if let Some(a) = s.audio.as_ref().filter(|_| s.instrument == "AudioModule") {
                            let takes = a
                                .takes
                                .iter()
                                .map(|t| {
                                    report.zones += 1;
                                    let sample = load(&t.sample).unwrap_or_else(|| Arc::new(Sample { rate: sr, frames: vec![[0.0; 2]] }));
                                    let len = sample.frames.len();
                                    let (lo, hi) = t.play_range;
                                    let range =
                                        if hi > lo { ((lo.max(0) as usize).min(len), (hi.max(0) as usize).min(len)) } else { (0, len) };
                                    // Without a length, as many beats as fit at the song's tempo.
                                    let beats = if t.beats > 0.0 {
                                        t.beats
                                    } else {
                                        ((range.1 - range.0) as f64 / sample.rate as f64 * project.tempo / 60.0).round().max(1.0)
                                    };
                                    audio_loop::TakeRt { sample, range, beats }
                                })
                                .collect();
                            let settings = audio_loop::Settings {
                                mode: a.mode,
                                engine: a.engine,
                                tune: a.tune,
                                bend_range: (a.pitchbend - 0.5) * 24.0,
                                fade: a.fade,
                            };
                            return Some(SamplerRt::audio(audio_loop::LoopRt::new(takes, settings, sr_f), sr_f));
                        }
                        let smp = s.sampler.as_ref()?;
                        let mut zones = Vec::new();
                        for z in &smp.zones {
                            report.zones += 1;
                            let Some(sample) = load(&z.sample) else { continue };
                            let len = sample.frames.len();
                            // Ranges are inclusive in the file.
                            let (lo, hi) = z.play_range;
                            let play = if hi > lo { ((lo.max(0) as usize).min(len), ((hi + 1) as usize).min(len)) } else { (0, len) };
                            let (ll, lh) = z.loop_range;
                            let looped = (z.loop_active && lh > ll).then(|| {
                                let a = (ll.max(0) as usize).clamp(play.0, play.1);
                                (a, ((lh + 1) as usize).clamp(a, play.1))
                            });
                            zones.push(sampler::ZoneRt {
                                sample,
                                root: z.root_key,
                                keys: z.key_range,
                                vels: z.velocity_range,
                                gain: z.gain as f32,
                                pan: z.pan as f32,
                                tune: z.tune as f32,
                                play,
                                looped: looped.filter(|(a, b)| b > a),
                                x_fade: z.x_fade.max(0) as usize,
                                fade_in: z.fade_in.max(0) as usize,
                                fade_out: z.fade_out.max(0) as usize,
                            });
                        }
                        // At unity: the sound's level and pan are the mixer's (they glide).
                        Some(SamplerRt::new(sampler::Settings::new(smp, sr_f), zones, (1.0, 1.0), sr_f))
                    })
                    .collect()
            })
            .collect();
        let mut instruments: Vec<Vec<Option<InstrumentRt>>> = Vec::new();
        for (gi, g) in project.groups.iter().enumerate() {
            let mut row = Vec::new();
            for (si, s) in g.sounds.iter().enumerate() {
                let Some(pr) = &s.plugin else {
                    row.push(None);
                    continue;
                };
                if kept(gi, si, |r| r.instrument) {
                    row.push(None);
                    continue;
                }
                match cache.plugin(Slot::Sound { group: gi, sound: si }, &pr.id, &pr.name, &pr.chunk, sr_f) {
                    Ok(plugin) => {
                        report.plugins.push(format!("{} / {}: {}", g.name, s.name, pr.name));
                        let m = muting(project, gi, si);
                        let gain = if has_fx(gi, si) {
                            (1.0, 1.0)
                        } else if m.silent || m.hushed {
                            (0.0, 0.0)
                        } else {
                            sound_gain(project, gi, si)
                        };
                        let mut inst = InstrumentRt::new(plugin.clone(), gain, m.muted);
                        inst.listen(Owner::Sound(gi, si), &plugin, &pr.id, project);
                        inst.heard = Some(cache.heard.clone());
                        row.push(Some(inst));
                    }
                    Err(e) => {
                        report.plugin_errors.push(e);
                        row.push(None);
                    }
                }
            }
            instruments.push(row);
        }
        let mod_routes = mod_rt::routes_of(project, &cache.plugins);
        let (faders, sound_faders, bus_faders) = channel_faders(project);
        // The playing timeline stays when the edit didn't change what makes it.
        let keep_timeline = prev.is_some_and(|(p, _)| same_timelines(p, project));
        let timeline = if keep_timeline { Timeline::default() } else { timeline(project, mode) };
        let mut played: Vec<(usize, usize)> = timeline.events.iter().map(|e| (e.group, e.sound)).collect();
        played.sort_unstable();
        played.dedup();
        for (g, s) in played {
            let (group, sound) = (&project.groups[g], project.groups[g].sounds.get(s));
            let has_instrument = instruments.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some) || kept(g, s, |r| r.instrument);
            let has_synth = sounds.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some);
            if sound.is_some_and(|x| x.sampler.is_none() && !x.muted) && !group.muted && !has_instrument && !has_synth {
                let x = sound.unwrap();
                report.no_sampler.push(format!(
                    "{} / {} ({})",
                    group.name,
                    x.name,
                    if x.instrument.is_empty() { "empty" } else { &x.instrument }
                ));
            }
        }
        // Sounds that must play live: they take audio or MIDI input, feed or hear a
        // side-chain, or pass notes to another sound.
        let routes = midi::out_routes(project);
        let routed_to = |g: usize, s: usize| routes.get(g).is_some_and(|r| r.iter().any(|x| x == &Some(midi::OutRoute::Sound(s))));
        let explicit = |src: &str| !matches!(src, "Default" | "Focus" | "None" | "");
        let live_hint: Vec<Vec<bool>> = project
            .groups
            .iter()
            .enumerate()
            .map(|(g, gr)| {
                (0..gr.sounds.len())
                    .map(|s| {
                        let snd = &gr.sounds[s];
                        matches!(snd.channel.audio_in, deckard_model::AudioSource::Ext(_))
                            || sc_index.keys().any(|&(sg, ss, _)| (sg, ss) == (g, s))
                            // (From the project: an edit leaves a kept chain unbuilt here.)
                            || effect_slots(snd).iter().any(|&(slot, x)| x.active && !hears(g, s, slot).is_empty())
                            || matches!(routes.get(g).and_then(|r| r.get(s)), Some(Some(midi::OutRoute::Sound(_))))
                            || routed_to(g, s)
                            || explicit(&snd.channel.midi_source)
                            || gr.channel.key_mode == 1
                            // Modulated: its effects follow the song position here.
                            || gr.patterns.iter().any(|pt| pt.mods.iter().any(|m| m.sound == Some(s)))
                            // Its routes follow its notes' expression here.
                            || !snd.mod_routes.is_empty()
                            || (gr.channel.key_mode == 2 && explicit(&gr.channel.midi_source))
                            // Its strips are mixed where it plays.
                            || sound_has_strip(project, g, s)
                            // A sound bus waits for the channels that feed it.
                            || is_sound_bus(project, g, s)
                    })
                    .collect()
            })
            .collect();
        let chains: Vec<Vec<bool>> =
            project.groups.iter().enumerate().map(|(g, grp)| (0..grp.sounds.len()).map(|s| has_fx(g, s)).collect()).collect();
        // Output buffers for the sounds that play something.
        let in_routes = input_routes(project);
        let bufs: Vec<Vec<Option<Box<SoundBufs>>>> = project
            .groups
            .iter()
            .enumerate()
            .map(|(g, gr)| {
                (0..gr.sounds.len())
                    .map(|s| {
                        let has = sounds.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some)
                            || instruments.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some)
                            || in_routes.iter().any(|r| (r.0, r.1) == (g, s))
                            || is_sound_bus(project, g, s);
                        let smp_gain = if has_fx(g, s) {
                            (1.0, 1.0)
                        } else if muting(project, g, s).hushed {
                            (0.0, 0.0)
                        } else {
                            sound_gain(project, g, s)
                        };
                        has.then(|| Box::new(SoundBufs::new(smp_gain)))
                    })
                    .collect()
            })
            .collect();
        // The gains kept parts play at now.
        for (g, grp) in project.groups.iter().enumerate() {
            for s in 0..grp.sounds.len() {
                let fx_here = chains[g][s];
                let Some(r) = reuse[g][s].as_mut() else { continue };
                let m = muting(project, g, s);
                let gain = if m.hushed { (0.0, 0.0) } else { sound_gain(project, g, s) };
                r.chain_gain = if m.silent { (0.0, 0.0) } else { gain };
                r.inst_gain = if fx_here {
                    (1.0, 1.0)
                } else if m.silent {
                    (0.0, 0.0)
                } else {
                    gain
                };
                r.inst_muted = m.muted;
                r.smp_gain = if fx_here { (1.0, 1.0) } else { gain };
            }
        }
        report.chains = chains;
        // Groups with effects get a bus; the master its chain.
        let buses: Vec<Option<Bus>> = project
            .groups
            .iter()
            .enumerate()
            .map(|(gi, grp)| {
                has_bus(grp).then(|| {
                    let units = effect_units(
                        project,
                        &grp.slots,
                        |slot| Slot::Group { group: gi, slot },
                        |slot| hears_group(gi, slot),
                        &grp.name,
                        project.tempo,
                        sr_f,
                        cache,
                        &mut report,
                    );
                    Bus {
                        chain: FxChain { heard: Some(cache.heard.clone()), ..bare_chain(units, sr_f) },
                        buf: vec![0.0; JOB_FRAMES * 2],
                        route: group_out_route(project, gi),
                        ramp: GainRamp::default(),
                    }
                })
            })
            .collect();
        let master_fx = (!project.master_slots.is_empty()).then(|| {
            let units = effect_units(
                project,
                &project.master_slots,
                |slot| Slot::Master { slot },
                |_| Vec::new(),
                "Master",
                project.tempo,
                sr_f,
                cache,
                &mut report,
            );
            FxChain { heard: Some(cache.heard.clone()), ..bare_chain(units, sr_f) }
        });
        let (sound_aux, group_aux) = aux::build(project, &faders);
        let sound_routes: Vec<Vec<Option<usize>>> =
            project.groups.iter().enumerate().map(|(g, grp)| (0..grp.sounds.len()).map(|s| sound_bus(project, g, s)).collect()).collect();
        let sound_buses: Vec<Vec<bool>> = project
            .groups
            .iter()
            .enumerate()
            .map(|(g, grp)| (0..grp.sounds.len()).map(|s| is_sound_bus(project, g, s)).collect())
            .collect();
        let strips: Vec<(Owner, usize, Channels, StripRoute)> =
            project.extra_outs.iter().map(|x| (x.owner, x.bus, x.channels, strip_route(project, x))).collect();
        let order = aux::order(&buses, &group_aux, &sound_buses, &sound_routes, &sound_aux, &strips);
        let engine = Engine {
            sr: sr_f,
            tempo: project.tempo,
            song_tempo: project.tempo,
            followed: false,
            timeline,
            mode,
            queued: None,
            jump: None,
            samples,
            sounds,
            // Room for sounds that go away while they ring (no allocating on the audio
            // thread).
            retired: Vec::with_capacity(64),
            instruments,
            note_offs: Vec::with_capacity(256),
            live: Vec::with_capacity(LIVE_ROOM),
            stale: false,
            time: 0,
            quiet: u64::MAX / 2,
            numerator: project.numerator as i32,
            denominator: project.denominator as i32,
            frame: 0,
            next: 0,
            sequencing: true,
            looping: false,
            metronome: false,
            metronome_auto: false,
            metronome_ticks: 0,
            click: metronome::Click::default(),
            prehear: prehear::Prehear::default(),
            recording: false,
            repeater: None,
            repeat_buf: Vec::with_capacity(256),
            midi_inputs: (0..midi_input::SLOTS).map(|_| None).collect(),
            midi_input_out: Vec::with_capacity(midi_input::OUT_ROOM),
            midi_inputs_unsettled: 0,
            from_midi_input: false,
            counting: 0,
            count_frame: 0,
            recorded: Vec::with_capacity(RECORDED_ROOM),
            recorded_expr: Vec::with_capacity(RECORDED_ROOM),
            recorded_controls: Vec::with_capacity(RECORDED_ROOM),
            record_seq: 0,
            master: deckard_model::level_gain(project.master_level) as f32,
            master_pan: pan_gains(project.master_pan),
            master_bus: match project.master_output {
                deckard_model::AudioDest::Ext(n) if (0..OUTPUTS as i64).contains(&n) => Some(n as usize),
                _ => None,
            },
            routes: sound_routes,
            cue: CueBus::of(project),
            faders,
            sound_faders,
            bus_faders,
            master_peak: (0.0, 0.0),
            clock: 0,
            rng: 0x9E37_79B9_7F4A_7C15,
            pass: 0,
            #[cfg(test)]
            fired: Vec::new(),
            meters: Vec::new(),
            tap: None,
            tap_pending: Vec::new(),
            tap_total: 0,
            live_hit: (0, 0, 0, 0, 0),
            bufs,
            job_list: Vec::with_capacity(project.groups.iter().map(|g| g.sounds.len()).sum()),
            job_ptrs: Vec::with_capacity(project.groups.iter().map(|g| g.sounds.len()).sum()),
            pool: None,
            ahead: None,
            live_hint,
            from_timeline: false,
            focus: None,
            reuse,
            keep_timeline,
            group_swaps: Vec::new(),
            tap_block: Vec::new(),
            fx,
            modulating: false,
            bus_order: order,
            buses,
            sound_aux,
            group_aux,
            master_fx,
            sc_bufs: vec![Vec::new(); sc_index.len()],
            midi_routes: midi::out_routes(project),
            sound_muted: project
                .groups
                .iter()
                .enumerate()
                .map(|(g, grp)| (0..grp.sounds.len()).map(|s| muting(project, g, s).muted).collect())
                .collect(),
            lfos: lfo_rt::lfos_of(project),
            cc_routes: lfo_rt::ccs_of(project),
            song_offset: 0.0,
            lfo_clock: lfo_rt::LfoClock::default(),
            silent: project.groups.iter().map(|g| vec![false; g.sounds.len()]).collect(),
            pads: pads::Pads::of(project),
            midi_tx: None,
            midi_offs: Vec::with_capacity(256),
            mpe: std::collections::HashMap::new(),
            batching: false,
            touch: Vec::with_capacity(mod_rt::VOICES),
            held_keys: Vec::with_capacity(mod_rt::KEYS),
            touch_order: 0,
            mod_routes,
            modded_plugins: Vec::with_capacity(mod_rt::UNROUTED),
            unrouted: Vec::with_capacity(mod_rt::UNROUTED),
            route_board: None,
            live_in: None,
            heard_board: cache.heard.clone(),
            heard: [Heard { owner: Owner::Master(0), bus: 0 }; HEARD_ROOM],
            heard_len: 0,
            strips,
            meters_extra: vec![(0.0, 0.0); project.extra_outs.len()],
            connected: u16::MAX,
            in_routes,
            inp_at: 0,
            audio_sounds: project
                .groups
                .iter()
                .enumerate()
                .flat_map(|(g, gr)| {
                    gr.sounds
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| s.audio.is_some() && s.instrument == "AudioModule")
                        .map(move |(s, _)| (g, s))
                })
                .collect(),
            sync: sync::Sync::default(),
        };
        (engine, report)
    }

    /// Note expression `id` of the timeline playing; none if it has no such id.
    pub fn expression(&self, id: deckard_model::ExprId) -> Option<std::sync::Arc<deckard_model::Expression>> {
        self.timeline().exprs.get(id.0 as usize).cloned()
    }

    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Take up changes to `project` while playing: tempo, notes, scenes and the mixer.
    /// Playback keeps its musical position (in ticks); sounding notes follow level
    /// and pan changes, and fade out if their sound was muted.
    pub fn update(&mut self, project: &Project, project_path: Option<&Path>, cache: &mut SampleCache) {
        let (fresh, _) = Engine::new(project, project_path, self.sr as u32, self.mode, cache);
        drop(self.adopt(fresh, project));
    }

    /// The second half of [`Engine::update`], for when `fresh` (an engine for the
    /// changed project, same rate and mode) was built without holding the audio lock.
    ///
    /// Returns what it replaced (holding the old effects, samplers and timeline): drop
    /// it after letting go of the lock the audio thread waits on, as freeing it takes
    /// longer than the adopting.
    #[must_use = "drop the replaced parts outside the engine's lock"]
    pub fn adopt(&mut self, fresh: Engine, project: &Project) -> Engine {
        let mut modes = vec![self.mode];
        modes.extend(self.queued());
        let mut timelines = Timelines::of(project, &modes);
        let old = self.adopt_with(fresh, &mut timelines);
        drop(timelines);
        old
    }

    /// [`Engine::adopt`] with the timelines made ahead (for the mode playing and the
    /// one queued, as [`Timelines::of`] makes them): nothing is built or freed here,
    /// so the audio thread can do it. What it replaced comes back in the returned
    /// engine and in `timelines`. If no timeline for the playing mode came, the old one
    /// plays on and [`Engine::stale`] is set.
    #[must_use = "drop the replaced parts off the audio thread"]
    pub fn adopt_with(&mut self, fresh: Engine, timelines: &mut Timelines) -> Engine {
        let mut fresh = fresh;
        for (a, b) in fresh.group_swaps.drain(..) {
            self.swap_group_runtime(a, b);
        }
        self.forward_edits(&mut fresh);
        self.keep_parts(&mut fresh);
        std::mem::swap(&mut self.live_hint, &mut fresh.live_hint);
        let ticks = self.position();
        // Sounding Sampler voices carry over to the same sounds; sounds that went away
        // fade out.
        for (g, row) in std::mem::take(&mut self.sounds).into_iter().enumerate() {
            for (si, old) in row.into_iter().enumerate() {
                let Some(mut old) = old else { continue };
                match fresh.sounds.get_mut(g).and_then(|r| r.get_mut(si)).and_then(Option::as_mut) {
                    Some(new) => new.adopt(old),
                    None if old.voices() > 0 => {
                        // It rings out at the level it had (now outside the mixer).
                        if let Some(b) = self.bufs.get(g).and_then(|r| r.get(si)).and_then(Option::as_deref) {
                            old.mix = b.ramps[0].now;
                        }
                        old.fade_all();
                        self.retired.push(old);
                    }
                    None => {}
                }
            }
        }
        self.note_offs.retain(|&(_, g, s, _, _)| {
            fresh.sounds.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some)
                || fresh.instruments.get(g).and_then(|r| r.get(s)).is_some_and(|i| i.as_ref().is_some_and(|i| !i.muted))
        });
        std::mem::swap(&mut self.samples, &mut fresh.samples);
        std::mem::swap(&mut self.sounds, &mut fresh.sounds);
        self.adopt_routes(&mut fresh);
        std::mem::swap(&mut self.modded_plugins, &mut fresh.modded_plugins);
        self.adopt_lfos(&mut fresh);
        self.adopt_ccs(&mut fresh);
        // Notes held by plug-ins that are gone or muted now end.
        let kept = |p: &Shared| fresh.instruments.iter().flatten().flatten().any(|i| !i.muted && Arc::ptr_eq(&i.plugin, p));
        for i in self.instruments.iter().flatten().flatten() {
            if !kept(&i.plugin)
                && let Ok(mut p) = i.plugin.lock()
            {
                p.all_notes_off();
            }
        }

        std::mem::swap(&mut self.instruments, &mut fresh.instruments);
        std::mem::swap(&mut self.midi_routes, &mut fresh.midi_routes);
        for (g, row) in fresh.fx.iter_mut().enumerate() {
            for (s, chain) in row.iter_mut().enumerate() {
                if let (Some(new), Some(Some(old))) = (chain.as_mut(), self.fx.get_mut(g).and_then(|r| r.get_mut(s))) {
                    new.adopt(old);
                }
            }
        }
        std::mem::swap(&mut self.fx, &mut fresh.fx);
        // Groups' and the master's effects carry their state (tails, plug-ins) over.
        for (g, new) in fresh.buses.iter_mut().enumerate() {
            if let (Some(new), Some(Some(old))) = (new.as_mut(), self.buses.get_mut(g)) {
                new.chain.adopt(&mut old.chain);
                new.ramp = old.ramp;
                if new.route.1 == (0.0, 0.0) && (old.ramp.left > 0 || old.ramp.now != (0.0, 0.0)) {
                    new.route.0 = old.route.0;
                }
            }
        }
        std::mem::swap(&mut self.buses, &mut fresh.buses);
        std::mem::swap(&mut self.bus_order, &mut fresh.bus_order);
        for (old, new) in self.sound_aux.iter_mut().flatten().zip(fresh.sound_aux.iter_mut().flatten()) {
            aux::adopt(old, new);
        }
        for (old, new) in self.group_aux.iter_mut().zip(&mut fresh.group_aux) {
            aux::adopt(old, new);
        }
        std::mem::swap(&mut self.sound_aux, &mut fresh.sound_aux);
        std::mem::swap(&mut self.group_aux, &mut fresh.group_aux);
        if let (Some(new), Some(old)) = (fresh.master_fx.as_mut(), self.master_fx.as_mut()) {
            new.adopt(old);
        }
        std::mem::swap(&mut self.master_fx, &mut fresh.master_fx);
        std::mem::swap(&mut self.sc_bufs, &mut fresh.sc_bufs);
        std::mem::swap(&mut self.heard_board, &mut fresh.heard_board);
        // Strips: where each goes now (parts kept take theirs), and meters of the new list.
        std::mem::swap(&mut self.strips, &mut fresh.strips);
        std::mem::swap(&mut self.meters_extra, &mut fresh.meters_extra);
        self.refresh_strips();
        std::mem::swap(&mut self.in_routes, &mut fresh.in_routes);
        std::mem::swap(&mut self.sound_muted, &mut fresh.sound_muted);
        std::mem::swap(&mut self.silent, &mut fresh.silent);
        std::mem::swap(&mut self.pads, &mut fresh.pads);
        // Gains glide on from where they are.
        for (row, old) in fresh.bufs.iter_mut().zip(&self.bufs) {
            for (b, ob) in row.iter_mut().zip(old) {
                if let (Some(b), Some(ob)) = (b.as_deref_mut(), ob.as_deref()) {
                    b.ramps = ob.ramps;
                }
            }
        }
        std::mem::swap(&mut self.bufs, &mut fresh.bufs);
        std::mem::swap(&mut self.job_list, &mut fresh.job_list);
        std::mem::swap(&mut self.job_ptrs, &mut fresh.job_ptrs);
        self.numerator = fresh.numerator;
        self.denominator = fresh.denominator;
        // Meters of the new shape, made here rather than on the audio thread.
        fresh.meters = self.sounds.iter().map(|g| vec![(0.0, 0.0); g.len()]).collect();
        std::mem::swap(&mut self.meters, &mut fresh.meters);
        self.tempo = fresh.tempo;
        self.song_tempo = fresh.song_tempo;
        for u in self.fx.iter_mut().flatten().flatten().flat_map(|c| c.units.iter_mut()) {
            if let FxUnit::Internal { fx, .. } = &mut u.fx {
                fx.set_tempo(self.tempo);
            }
        }
        self.master = fresh.master;
        self.master_pan = fresh.master_pan;
        self.master_bus = fresh.master_bus;
        std::mem::swap(&mut self.routes, &mut fresh.routes);
        // (Its meter goes on.)
        fresh.cue.peak = self.cue.peak;
        std::mem::swap(&mut self.cue, &mut fresh.cue);
        std::mem::swap(&mut self.faders, &mut fresh.faders);
        std::mem::swap(&mut self.sound_faders, &mut fresh.sound_faders);
        std::mem::swap(&mut self.bus_faders, &mut fresh.bus_faders);
        let fading_buses: Vec<usize> = self
            .routes
            .iter()
            .flatten()
            .copied()
            .chain(self.buses.iter().flatten().map(|b| b.route.0))
            .chain(self.sound_aux.iter().flatten().flatten().map(|s| s.route.0))
            .chain(self.group_aux.iter().flatten().map(|s| s.route.0))
            .flatten()
            .filter(|&n| n >= SOUND_BUS)
            .collect();
        for n in fading_buses {
            if let Some((g, s)) = decode_sound_bus(n) {
                if self.bufs.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_none)
                    && let Some(old_b) = fresh.bufs.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::take)
                {
                    self.bufs[g][s] = Some(old_b);
                }
                if self.bufs.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some) && !self.bus_order.contains(&n) {
                    self.bus_order.push(n);
                    if let Some(h) = self.live_hint.get_mut(g).and_then(|r| r.get_mut(s)) {
                        *h = true;
                    }
                }
            }
        }
        self.stale = false;
        if fresh.keep_timeline {
            // The same timelines: this one's play on.
        } else if fresh.mode == self.mode {
            std::mem::swap(&mut self.timeline, &mut fresh.timeline);
        } else if !timelines.swap_into(self.mode, &mut self.timeline) {
            self.stale = true;
        }
        if !fresh.keep_timeline
            && let Some((mode, t)) = self.queued.as_mut()
            && !timelines.swap_into(*mode, t)
        {
            self.stale = true;
        }
        let len = self.timeline.length as f64;
        let ticks = if self.looping && len > 0.0 { ticks % len } else { ticks };
        self.frame = (ticks * self.samples_per_tick()).round().max(0.0) as u64;
        self.next =
            self.timeline.events.iter().position(|e| self.frame_of(e.tick) >= self.frame.max(1)).unwrap_or(self.timeline.events.len());
        // Events exactly at the current frame were already played unless we're at the start.
        if self.frame == 0 {
            self.next = 0;
        }
        fresh
    }

    /// Every strip of the sounds playing here, and of the groups' and the master's
    /// chains, takes where it is and where it goes from [`Engine::strips`] (after an
    /// edit, which may keep the parts that have them). No allocating.
    fn refresh_strips(&mut self) {
        for g in 0..self.fx.len().max(self.instruments.len()) {
            for s in 0..self.fx.get(g).map_or(0, Vec::len).max(self.instruments.get(g).map_or(0, Vec::len)) {
                self.refresh_sound_strips(g, s);
            }
        }
        let routes = &self.strips;
        let chains = self.buses.iter_mut().flatten().map(|b| &mut b.chain).chain(self.master_fx.as_mut());
        for u in chains.flat_map(|c| c.units.iter_mut()) {
            refresh_strips(u.owner, &mut u.extras, routes);
        }
    }

    /// [`Engine::refresh_strips`] for sound (`g`, `s`)'s parts here.
    fn refresh_sound_strips(&mut self, g: usize, s: usize) {
        let routes = &self.strips;
        if let Some(Some(i)) = self.instruments.get_mut(g).and_then(|r| r.get_mut(s)) {
            refresh_strips(i.owner, &mut i.extras, routes);
        }
        if let Some(Some(c)) = self.fx.get_mut(g).and_then(|r| r.get_mut(s)) {
            for u in &mut c.units {
                refresh_strips(u.owner, &mut u.extras, routes);
            }
        }
    }

    /// The external outputs (bits: Out 1 is bit 0) the interface has connected (see
    /// `output::connected_outputs`): a strip onto one that isn't plays through its
    /// parent. All are, until this says otherwise.
    pub fn set_connected_outputs(&mut self, mask: u16) {
        self.connected = mask;
    }

    /// For an edit built with [`Engine::new_after`]: move the parts it keeps into it (so
    /// the rest of adopting leaves them be), with the gains and settings they now have.
    fn keep_parts(&mut self, fresh: &mut Engine) {
        for g in 0..fresh.reuse.len() {
            for s in 0..fresh.reuse[g].len() {
                // (A sound rendered ahead: its player keeps its parts; only its buffers
                // here are kept, below.)
                let here = self.link_ahead(g, s).is_none();
                if !here
                    && fresh.bufs[g][s].is_none()
                    && let Some(b) = self.bufs.get_mut(g).and_then(|x| x.get_mut(s)).and_then(Option::take)
                {
                    // What its player plays (a tail fading out, say) is still mixed.
                    fresh.bufs[g][s] = Some(b);
                }
                let Some(r) = fresh.reuse[g][s].as_mut() else { continue };
                if here
                    && r.sampler
                    && let Some(mut smp) = self.sounds.get_mut(g).and_then(|x| x.get_mut(s)).and_then(Option::take)
                {
                    if let Some(st) = r.settings.as_mut() {
                        // The old settings go back in the edit, to be freed there.
                        smp.swap_settings(st);
                    }
                    fresh.sounds[g][s] = Some(smp);
                }
                if here
                    && r.instrument
                    && let Some(mut i) = self.instruments.get_mut(g).and_then(|x| x.get_mut(s)).and_then(Option::take)
                {
                    (i.gain, i.muted) = (r.inst_gain, r.inst_muted);
                    fresh.instruments[g][s] = Some(i);
                }
                if here
                    && r.chain
                    && let Some(mut c) = self.fx.get_mut(g).and_then(|x| x.get_mut(s)).and_then(Option::take)
                {
                    c.gain = r.chain_gain;
                    c.set_params(&mut r.params);
                    fresh.fx[g][s] = Some(c);
                }
                // Its buffers (and their gains' glides) stay.
                if fresh.bufs[g][s].is_none()
                    && let Some(mut b) = self.bufs.get_mut(g).and_then(|x| x.get_mut(s)).and_then(Option::take)
                {
                    b.smp_gain = r.smp_gain;
                    fresh.bufs[g][s] = Some(b);
                }
            }
        }
    }

    /// Send an edit's parts for the sounds players render to them (they take over the
    /// playing parts' state there); `fresh` keeps the rest.
    fn forward_edits(&mut self, fresh: &mut Engine) {
        let now = self.time;
        let Some(a) = self.ahead.as_mut() else { return };
        let edit_at = now + a.lookahead + a.span;
        for g in 0..a.links.len() {
            for s in 0..a.links[g].len() {
                let Some(link) = a.links[g][s].as_mut().filter(|l| l.state != LinkState::Live) else { continue };
                let smp = fresh.sounds.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::take);
                let inst = fresh.instruments.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::take);
                let chain = fresh.fx.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::take);
                // (A box made here only if two edits come before the player sent the
                // last one's back.)
                let mut parts = link.spare.take().unwrap_or_else(|| Box::new(ahead::Parts::empty()));
                match fresh.reuse.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut) {
                    // Built after the playing parts: the player keeps what didn't change.
                    Some(r) => {
                        let has_chain = if r.chain { link.finite } else { chain.is_some() };
                        link.gains[0] = if has_chain { r.chain_gain } else { r.smp_gain };
                        link.gains[1] = if has_chain { r.chain_gain } else { r.inst_gain };
                        link.finite = has_chain;
                        parts.keep = [r.sampler, r.instrument, r.chain];
                        parts.settings = r.settings.take();
                        std::mem::swap(&mut parts.params, &mut r.params);
                        parts.inst_gain = (r.inst_gain, r.inst_muted);
                    }
                    None => {
                        let smp_gain =
                            fresh.bufs.get(g).and_then(|r| r.get(s)).and_then(Option::as_deref).map_or((1.0, 1.0), |b| b.smp_gain);
                        link.gains[0] = chain.as_ref().map_or(smp_gain, |c| c.gain);
                        link.gains[1] = chain.as_ref().map_or(inst.as_ref().map_or((0.0, 0.0), |i| i.gain), |c| c.gain);
                        link.finite = chain.is_some();
                        parts.keep = [false; 3];
                    }
                }
                (parts.smp, parts.inst, parts.chain) = (smp, inst, chain);
                // From a frame its player can't have rendered yet (as live input), so the
                // edit lands at the same frame every time.
                if let Err(ahead::Msg::Adopt(_, parts)) = link.inbox.push(ahead::Msg::Adopt(edit_at, parts)) {
                    a.junk.push(Junk(ahead::JunkKind::Parts(parts)));
                }
            }
        }
        a.driver.wake();
    }

    /// An edit's engine that keeps the playing timelines (none need making for it).
    pub fn keeps_timelines(&self) -> bool {
        self.keep_timeline
    }

    /// What is playing now.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// What will play after the current pass, if anything was queued.
    pub fn queued(&self) -> Option<Mode> {
        self.queued.as_ref().map(|(m, _)| *m)
    }

    /// Switch to `mode` when the current pass (scene, pattern or song) ends, as
    /// launching a scene does in Maschine. Queuing the playing mode cancels a switch.
    pub fn queue(&mut self, project: &Project, mode: Mode) {
        let mut t = timeline(project, mode);
        self.queue_timeline(mode, &mut t);
    }

    /// [`Engine::queue`] with `mode`'s timeline made ahead, swapped in through `t` (which
    /// gets back what it replaced, to be freed off the audio thread).
    pub fn queue_timeline(&mut self, mode: Mode, t: &mut Timeline) {
        if mode == self.mode {
            if let Some((_, old)) = self.queued.as_mut() {
                std::mem::swap(old, t);
            }
            self.queued = None;
            return;
        }
        match self.queued.as_mut() {
            Some((m, old)) => {
                *m = mode;
                std::mem::swap(old, t);
            }
            None => self.queued = Some((mode, std::mem::take(t))),
        }
    }

    /// When playback reaches tick `at`, continue from tick `to` (in this timeline).
    /// Replaces any earlier jump; `None` cancels.
    pub fn jump_at(&mut self, jump: Option<(i64, i64)>) {
        self.jump = jump;
    }

    pub fn pending_jump(&self) -> Option<(i64, i64)> {
        self.jump
    }

    /// Continue from tick `to` now.
    pub fn seek(&mut self, to: i64) {
        self.frame = self.frame_of(to.max(0));
        self.next = self.timeline.events.iter().position(|e| e.tick >= to).unwrap_or(self.timeline.events.len());
        self.jump = None;
        self.recue();
    }

    /// The Audio plug-ins' loops as they'd be where the song now is (the last cue before
    /// it, for each): the song started or jumped into a pattern, past its start.
    fn recue(&mut self) {
        for k in 0..self.audio_sounds.len() {
            let (g, s) = self.audio_sounds[k];
            let last = self.timeline.events[..self.next].iter().rev().find(|e| e.cue.is_some() && e.group == g && e.sound == s);
            let cue = last.and_then(|e| cue_take(e.cue?).map(|t| (t, e.tick as f64 / PPQ as f64)));
            if self.link_ahead(g, s).is_some() {
                self.send_live(g, s, ahead::Ev::Cue(cue.map(|c| c.0), cue.map_or(0.0, |c| c.1)));
            } else if let Some(Some(snd)) = self.sounds.get_mut(g).and_then(|r| r.get_mut(s)) {
                snd.cue(cue);
            }
        }
    }

    /// Switch to `mode` now, from its start.
    pub fn jump(&mut self, project: &Project, mode: Mode) {
        self.pass += 1;
        self.queued = None;
        self.timeline = timeline(project, mode);
        self.mode = mode;
        self.frame = 0;
        self.next = 0;
        self.sequencing = true;
    }

    /// The timeline events that will fire from `skip` to `skip + frames` output frames
    /// from now, if nothing changes meanwhile: each with its offset from now, its index,
    /// its pass, whether its probability lets it play, and the song frame it fires at.
    /// Follows the sequencer exactly (jumps, the queued switch, looping), without
    /// playing anything.
    fn foresee(&self, skip: u64, frames: u64, mut fire: impl FnMut(Seen<'_>)) {
        let (mut frame, mut next, mut sequencing, mut jump, mut pass) = (self.frame, self.next, self.sequencing, self.jump, self.pass);
        let mut switched = false;
        let total = skip + frames;
        let mut t = 0u64;
        // Where the song is at the start, and after each jump.
        let mut moved = true;
        while t < total {
            let tl = match (&self.queued, switched) {
                (Some((_, q)), true) => q,
                _ => &self.timeline,
            };
            let end = self.frame_of(tl.length);
            if sequencing
                && let Some((at, to)) = jump
                && frame >= self.frame_of(at)
            {
                frame = self.frame_of(to.max(0));
                next = tl.events.iter().position(|e| e.tick >= to).unwrap_or(tl.events.len());
                jump = None;
                moved = true;
            }
            if sequencing {
                while let Some(e) = tl.events.get(next) {
                    if self.frame_of(e.tick) > frame {
                        break;
                    }
                    if t >= skip {
                        fire(Seen::Event { t, e, plays: chance(e.probability, next, tl.chance_pass(e, pass), self.rng), frame, pass });
                    }
                    next += 1;
                }
                if next >= tl.events.len() && frame >= end {
                    if self.queued.is_some() && !switched {
                        frame -= end.min(frame);
                        next = 0;
                        pass += 1;
                        switched = true;
                        moved = true;
                        continue;
                    }
                    if self.looping && end > 0 {
                        frame -= end;
                        next = 0;
                        pass += 1;
                        moved = true;
                        continue;
                    }
                    sequencing = false;
                    moved = true;
                }
            }
            if moved && t >= skip {
                fire(Seen::Position { t, frame, playing: sequencing });
                moved = false;
            }
            let mut until = total - t;
            if sequencing {
                let mut target = tl.events.get(next).map_or(end, |e| self.frame_of(e.tick));
                if let Some((at, _)) = jump {
                    target = target.min(self.frame_of(at));
                }
                until = until.min(target.saturating_sub(frame).max(1));
                frame += until;
            }
            t += until;
        }
    }

    /// Current position in ticks.
    pub fn position(&self) -> f64 {
        self.frame as f64 / self.samples_per_tick()
    }

    /// Ticks on a clock that never wraps (see [`Recorded::played`]).
    pub fn played(&self) -> f64 {
        self.time as f64 / self.samples_per_tick()
    }

    /// The tempo (BPM) it plays at: the song's, or the playing scene's own.
    pub fn tempo(&self) -> f64 {
        self.tempo
    }

    /// A clock sets the tempo (Link, incoming MIDI clock): scenes' own tempos don't count.
    pub(crate) fn set_followed(&mut self, on: bool) {
        self.followed = on;
    }

    /// Play at the tempo the timeline wants where it is: true if that changed it. The
    /// position stays.
    pub(crate) fn follow_scene_tempo(&mut self) -> bool {
        if self.followed || !self.sequencing {
            return false;
        }
        let want = self.timeline.tempo_at(self.position() as i64).unwrap_or(self.song_tempo);
        if want == self.tempo {
            return false;
        }
        self.play_at(want);
        true
    }

    /// How many frames the timeline runs, through its tempo changes.
    pub(crate) fn timeline_frames(&self) -> f64 {
        let spt = |bpm: f64| self.sr * 60.0 / (bpm * PPQ as f64);
        let (mut at, mut bpm, mut frames) = (0, self.timeline.tempo_at(0).unwrap_or(self.song_tempo), 0.0);
        for &(tick, tempo) in self.timeline.tempos.iter().filter(|t| t.0 > 0 && t.0 < self.timeline.length) {
            frames += (tick - at) as f64 * spt(bpm);
            (at, bpm) = (tick, tempo.unwrap_or(self.song_tempo));
        }
        frames + (self.timeline.length - at) as f64 * spt(bpm)
    }

    fn samples_per_tick(&self) -> f64 {
        self.sr * 60.0 / (self.tempo * PPQ as f64)
    }

    /// Output frame (from the start of the timeline) at which `tick` falls.
    fn frame_of(&self, tick: i64) -> u64 {
        (tick as f64 * self.samples_per_tick()).round().max(0.0) as u64
    }

    /// Keep only the sounds for which `keep(group, sound)` is true (a solo).
    pub fn retain_sounds(&mut self, keep: impl Fn(usize, usize) -> bool) {
        for (g, sounds) in self.sounds.iter_mut().enumerate() {
            for (s, snd) in sounds.iter_mut().enumerate() {
                if !keep(g, s) {
                    *snd = None;
                    if let Some(x) = self.silent.get_mut(g).and_then(|r| r.get_mut(s)) {
                        *x = true;
                    }
                }
            }
        }
        for (g, row) in self.instruments.iter_mut().enumerate() {
            for (s, i) in row.iter_mut().enumerate() {
                if !keep(g, s) {
                    *i = None;
                }
            }
        }
    }

    /// Voices sounding now.
    pub fn active_voices(&self) -> usize {
        let ahead = self.ahead.as_ref().map_or(0, |a| {
            a.links
                .iter()
                .flatten()
                .flatten()
                .filter(|l| l.state != LinkState::Live)
                .map(|l| l.voices.load(std::sync::atomic::Ordering::Relaxed))
                .sum()
        });
        self.sounds.iter().flatten().flatten().chain(&self.retired).map(SamplerRt::voices).sum::<usize>() + ahead
    }

    /// Nothing more will sound (plug-ins have been quiet for half a second).
    pub fn finished(&self) -> bool {
        // (A count-in, or waiting for a clock's bar, is the song waiting to start, not
        // its end.)
        !self.sequencing
            && self.counting == 0
            && !self.sync.waits()
            && self.active_voices() == 0
            && self.note_offs.is_empty()
            && self.quiet as f64 > 0.5 * self.sr
    }

    /// End the notes plug-ins are playing (before this engine is replaced by another
    /// using the same instances).
    pub fn release_plugins(&mut self) {
        for i in self.instruments.iter().flatten().flatten() {
            if let Ok(mut p) = i.plugin.lock() {
                p.all_notes_off();
            }
        }
        self.note_offs.clear();
    }

    /// Stop the sequencer and fade out all voices quickly.
    pub fn stop(&mut self) {
        self.sequencing = false;
        self.clock_stop();
        self.live.retain(|(_, e)| !matches!(e, Live::MidiInputOut { .. }));
        self.midi_inputs_unsettled = 0;
        for p in self.midi_inputs.iter_mut().flatten() {
            p.stopped();
        }
        if let Some(a) = &self.ahead {
            for l in a.links.iter().flatten().flatten().filter(|l| l.state != LinkState::Live) {
                let _ = l.inbox.push(ahead::Msg::At(self.time, ahead::Ev::Stop, false));
            }
            a.driver.wake();
        }
        for i in self.instruments.iter().flatten().flatten() {
            if let Ok(mut p) = i.plugin.try_lock() {
                p.all_notes_off();
            }
        }
        self.note_offs.clear();
        self.touch.clear();
        self.lfo_release_all();
        self.end_midi();
        for s in self.sounds.iter_mut().flatten().flatten().chain(&mut self.retired) {
            s.fade_all();
        }
    }

    fn send_midi(&self, port: &Arc<str>, bytes: [u8; 3], len: u8) {
        if let Some(tx) = &self.midi_tx {
            let _ = tx.try_send(midi::MidiOut { port: port.clone(), bytes, len, at: None });
        }
    }

    /// End a note sent to a MIDI port: its note-off (with its release velocity). The only
    /// place notes sent out end.
    fn end_midi_note(&mut self, n: MidiNote) {
        // (A note still waiting was never sent.)
        if n.waiting.is_none() {
            self.send_midi(&n.port, [0x80 | n.channel, n.note, n.release], 3);
        }
        if n.mpe
            && let Some(m) = self.mpe.get_mut(&*n.port)
        {
            m.free(n.channel, self.time);
        }
    }

    /// End every note sent to a MIDI port and reset the MPE channels used (stopping, or
    /// this engine replaced by another).
    pub fn end_midi(&mut self) {
        for n in std::mem::take(&mut self.midi_offs) {
            self.end_midi_note(n);
        }
        let used: Vec<(String, u16)> = self.mpe.iter_mut().map(|(k, m)| (k.clone(), std::mem::take(&mut m.used))).collect();
        for (port, used) in used {
            self.reset_mpe(&port, used);
        }
    }

    /// Output ports to send MPE to: name → member pitch-bend range (others are normal).
    /// A port whose format or range changes ends its notes (each on the channel it
    /// started on) and has its used member channels reset.
    pub fn set_mpe_out(&mut self, ports: std::collections::BTreeMap<String, u8>) {
        let mut changed: Vec<String> = self.mpe.iter().filter(|(k, m)| ports.get(*k) != Some(&m.range)).map(|(k, _)| k.clone()).collect();
        changed.extend(ports.keys().filter(|k| !self.mpe.contains_key(*k)).cloned());
        for name in changed {
            let mut i = 0;
            while i < self.midi_offs.len() {
                if *self.midi_offs[i].port == *name {
                    let n = self.midi_offs.remove(i);
                    self.end_midi_note(n);
                } else {
                    i += 1;
                }
            }
            if let Some(m) = self.mpe.remove(&name) {
                self.reset_mpe(&name, m.used);
            }
            if let Some(&range) = ports.get(&name) {
                self.mpe.insert(name, midi::MpePort::new(range));
            }
        }
    }

    /// Member channels `used` (bit i: nibble i + 1) of MPE port `port` back to neutral:
    /// bend centred, CC74 64, pressure 0.
    fn reset_mpe(&self, port: &str, used: u16) {
        let port: Arc<str> = port.into();
        for ch in (1..=15u8).filter(|c| used & (1 << (c - 1)) != 0) {
            self.send_midi(&port, [0xe0 | ch, 0, 64], 3);
            self.send_midi(&port, [0xb0 | ch, 74, 64], 3);
            self.send_midi(&port, [0xd0 | ch, 0, 0], 2);
        }
    }

    /// A member channel of MPE port `port` for a new note; with all busy, the port's
    /// oldest note ends and gives up its channel.
    fn mpe_channel(&mut self, port: &Arc<str>) -> u8 {
        loop {
            let now = self.time;
            if let Some(c) = self.mpe.get_mut(&**port).and_then(|m| m.take(now)) {
                return c;
            }
            let oldest =
                self.midi_offs.iter().enumerate().filter(|(_, n)| n.mpe && n.port == *port).min_by_key(|(_, n)| n.started).map(|(i, _)| i);
            match oldest {
                Some(i) => {
                    let n = self.midi_offs.remove(i);
                    self.end_midi_note(n);
                }
                None => return 1,
            }
        }
    }

    /// Frames between two sends of one dimension of a note's expression.
    fn expr_step(&self) -> u64 {
        (self.sr * EXPR_STEP_SECS).round().max(1.0) as u64
    }

    /// Live expression for held note `held` (group, sound, pitch), out with it.
    fn expression_out(&mut self, held: (usize, usize, i64), kind: ExprKind, value: f32) {
        let mpe = &self.mpe;
        for n in self.midi_offs.iter_mut().filter(|n| n.at == u64::MAX && n.held == held) {
            let d = match kind {
                ExprKind::Release => {
                    n.release = (value.clamp(0.0, 1.0) * 127.0).round().max(1.0) as u8;
                    continue;
                }
                ExprKind::Dim(d) => d,
            };
            let range = if n.mpe { mpe.get(&*n.port).map(|m| m.range) } else { None };
            let Some((v, bytes, len)) = expr_message(d, value, range, n.channel, n.note) else { continue };
            n.sent[Dim::ALL.iter().position(|x| *x == d).unwrap_or(0)] = Some(v);
            if n.waiting.is_some() {
                continue;
            }
            if let Some(tx) = &self.midi_tx {
                let _ = tx.try_send(midi::MidiOut { port: n.port.clone(), bytes, len, at: None });
            }
        }
    }

    /// The sounding notes' recorded expression at the playhead, sent where it changed
    /// (each dimension at most every [`EXPR_STEP_SECS`]): on a port, pressure as poly
    /// aftertouch on the note.
    fn play_expression(&mut self) {
        let now = self.time;
        let step = self.expr_step();
        let Some(tx) = &self.midi_tx else { return };
        let mpe = &self.mpe;
        for n in &mut self.midi_offs {
            let Some(e) = &n.expr else { continue };
            if n.at <= now {
                continue;
            }
            let tick = (now.saturating_sub(n.started) as f64 * n.tps) as i64;
            let range = if n.mpe { mpe.get(&*n.port).map(|m| m.range) } else { None };
            for (i, d) in Dim::ALL.into_iter().enumerate() {
                let Some(c) = e.curve(d) else { continue };
                let Some((v, bytes, len)) = expr_message(d, c.value_at(tick), range, n.channel, n.note) else { continue };
                if n.sent[i] == Some(v) || n.sent[i].is_some_and(|_| now < n.sent_at[i] + step) {
                    continue;
                }
                n.sent[i] = Some(v);
                n.sent_at[i] = now;
                let _ = tx.try_send(midi::MidiOut { port: n.port.clone(), bytes, len, at: None });
            }
        }
    }

    /// A note from MIDI input: played by the sound's instrument; sent on to its MIDI
    /// output when `thru` is set or the sound has no instrument.
    pub fn note_in(&mut self, group: usize, sound: usize, pitch: i64, velocity: i64, thru: bool) {
        let has_instrument = self.sounds.get(group).and_then(|r| r.get(sound)).is_some_and(Option::is_some)
            || self.instruments.get(group).and_then(|r| r.get(sound)).is_some_and(Option::is_some);
        let tps = 1.0 / self.samples_per_tick();
        let e = NoteEvent {
            tick: 0,
            group,
            sound,
            pitch,
            velocity,
            length: HELD,
            probability: 100,
            midi: None,
            cue: None,
            expr: None,
            from: None,
        };
        self.trigger_with(e, tps, thru || !has_instrument);
    }

    /// A controller or pitch bend from MIDI input, for a sound.
    pub fn control_in(&mut self, group: usize, sound: usize, kind: MidiKind, thru: bool) {
        let tps = 1.0 / self.samples_per_tick();
        let e = NoteEvent {
            tick: 0,
            group,
            sound,
            pitch: 0,
            velocity: 0,
            length: 0,
            probability: 100,
            midi: Some(kind),
            cue: None,
            expr: None,
            from: None,
        };
        self.trigger_with(e, tps, thru);
    }

    /// Play one note now, outside the sequence (a pad hit). `length` (ticks) gates
    /// sounds with an envelope; one-shots play to the end.
    pub fn note_on(&mut self, group: usize, sound: usize, pitch: i64, velocity: i64, length: i64) {
        let tps = 1.0 / self.samples_per_tick();
        self.trigger(
            NoteEvent { tick: 0, group, sound, pitch, velocity, length, probability: 100, midi: None, cue: None, expr: None, from: None },
            tps,
        );
    }

    /// Start a note now that sounds until [`Engine::note_release`] (a key held down).
    pub fn note_hold(&mut self, group: usize, sound: usize, pitch: i64, velocity: i64) {
        self.note_on(group, sound, pitch, velocity, HELD);
    }

    /// End a note started with [`Engine::note_hold`]: its envelope releases (one-shots
    /// play on), and a plug-in gets its note-off.
    pub fn note_release(&mut self, group: usize, sound: usize, pitch: i64) {
        if self.midi_input_takes(group) {
            self.midi_input_note(group, midi_input::NoteIn::Off { sound, pitch });
            return;
        }
        self.touch_release(group, sound, pitch);
        // Held notes sent out end now (also those passed on to another sound).
        let mut i = 0;
        while i < self.midi_offs.len() {
            let n = &self.midi_offs[i];
            if n.at == u64::MAX && n.held == (group, sound, pitch) {
                let n = self.midi_offs.remove(i);
                self.end_midi_note(n);
            } else {
                i += 1;
            }
        }
        if let Some(Some(midi::OutRoute::Sound(t))) = self.midi_routes.get(group).and_then(|r| r.get(sound)).cloned() {
            self.release_local(group, t, pitch);
        }
        self.release_local(group, sound, pitch);
    }

    fn release_local(&mut self, group: usize, sound: usize, pitch: i64) {
        self.lfo_release(group, sound, Some(pitch));
        if self.link_ahead(group, sound).is_some() {
            self.send_live(group, sound, ahead::Ev::Release { pitch });
            return;
        }
        if let Some(Some(s)) = self.sounds.get_mut(group).and_then(|g| g.get_mut(sound)) {
            s.release_note(pitch);
        }
        if let Some(Some(inst)) = self.instruments.get(group).and_then(|g| g.get(sound))
            && let Ok(mut p) = inst.plugin.lock()
        {
            p.note_off(pitch);
        }
        self.note_offs.retain(|o| !(o.1 == group && o.2 == sound && o.3 == pitch));
    }

    /// Render the sounds that can't get live input ahead of the audio thread, on a
    /// thread of their own (and `workers` helpers), `lookahead` frames ahead (see the
    /// `ahead` module). `block`: the output's block size (for real-time scheduling).
    /// `wait`: wait for players as long as they take (offline rendering), else at most
    /// half a block (then the sound is silent there).
    pub fn render_ahead(&mut self, lookahead: u64, workers: usize, block: u32, wait: bool) {
        if self.ahead.is_some() || lookahead == 0 {
            return;
        }
        let ring = (lookahead as usize + JOB_FRAMES * 4).max(8192);
        let sr = self.sr;
        let mut players = Vec::new();
        let links = self
            .bufs
            .iter()
            .enumerate()
            .map(|(g, row)| {
                row.iter()
                    .enumerate()
                    .map(|(s, b)| {
                        b.as_ref()?;
                        let p = ahead::Player::new(g, s, sr, ring);
                        let link = Link {
                            ring: p.ring.clone(),
                            inbox: p.inbox.clone(),
                            voices: p.voices.clone(),
                            state: LinkState::Live,
                            gains: [(1.0, 1.0); 2],
                            finite: false,
                            spare: Some(Box::new(ahead::Parts::empty())),
                            offs: Vec::with_capacity(512),
                            returned: None,
                        };
                        players.push(p);
                        Some(link)
                    })
                    .collect()
            })
            .collect();
        let driver = ahead::Driver::start(players, lookahead, workers, block, sr as u32);
        driver.advance(self.time);
        self.ahead = Some(Ahead {
            driver,
            links,
            lookahead,
            sent: Vec::with_capacity(4096),
            next_id: 1 << 48,
            wait,
            underruns: 0,
            budget: std::time::Duration::ZERO,
            span: 0,
            junk: Vec::with_capacity(64),
        });
        self.rebalance();
    }

    /// The group whose sounds play live (pads and keys play it); the others may be
    /// rendered ahead.
    pub fn set_focus(&mut self, group: Option<usize>) {
        self.focus = group;
    }

    /// Have metered effects (the Compressor) post their meters to `m` (for their
    /// panels), in this engine and in what moves to its players.
    pub fn set_fx_meters(&mut self, m: &Arc<meters::FxMeters>) {
        for (g, row) in self.fx.iter_mut().enumerate() {
            for (s, c) in row.iter_mut().enumerate() {
                if let Some(c) = c {
                    c.meters = Some((m.clone(), g, s));
                }
            }
        }
    }

    /// Publish the routes' travels on `b` (for their knobs).
    pub fn set_route_travels(&mut self, b: &Arc<meters::RouteTravels>) {
        self.route_board = Some(b.clone());
    }

    /// Stop rendering ahead (this engine is being replaced).
    pub fn halt_ahead(&self) {
        if let Some(a) = &self.ahead {
            a.driver.halt();
        }
    }

    /// Blocks where a sound rendered ahead wasn't ready in time.
    pub fn ahead_underruns(&self) -> u64 {
        self.ahead.as_ref().map_or(0, |a| a.underruns)
    }

    /// How many sounds are rendered ahead now.
    pub fn sounds_ahead(&self) -> usize {
        self.ahead.as_ref().map_or(0, |a| a.links.iter().flatten().flatten().filter(|l| l.state != LinkState::Live).count())
    }

    /// Something to free off the audio thread (what players sent back).
    pub fn take_junk(&mut self) -> Option<Junk> {
        self.ahead.as_mut()?.junk.pop()
    }

    /// Sound (`g`, `s`)'s player, when it renders the sound (ahead, or until it hands
    /// it back).
    fn link_ahead(&self, g: usize, s: usize) -> Option<&Link> {
        self.ahead.as_ref()?.links.get(g)?.get(s)?.as_ref().filter(|l| l.state != LinkState::Live)
    }

    /// Hand sounds to their players or take them back, as the focus and the sounds'
    /// inputs say (at the end of a span, so a player starts where the audio thread
    /// stopped).
    fn rebalance(&mut self) {
        // Sounds about to go ahead get their notes for the lookahead first (their
        // player may start rendering the moment it has them).
        let Some(a) = self.ahead.as_ref() else { return };
        let mut going = [(0usize, 0usize); 32];
        let mut n = 0;
        for (g, row) in a.links.iter().enumerate() {
            for (s, link) in row.iter().enumerate() {
                let Some(link) = link else { continue };
                let want = !self.live_hint.get(g).and_then(|r| r.get(s)).copied().unwrap_or(true) && self.focus != Some(g);
                let has = self.sounds.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some)
                    || self.instruments.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some);
                if link.state == LinkState::Live && want && has && link.spare.is_some() && n < going.len() {
                    going[n] = (g, s);
                    n += 1;
                }
            }
        }
        for &(g, s) in &going[..n] {
            if let Some(a) = self.ahead.as_mut() {
                a.sent.retain(|x| (x.1, x.2) != (g, s));
            }
            let lookahead = self.ahead.as_ref().map_or(0, |a| a.lookahead);
            self.send_notes(lookahead, Some((g, s)));
        }
        let Some(a) = self.ahead.as_mut() else { return };
        let now = self.time;
        for g in 0..a.links.len() {
            for s in 0..a.links[g].len() {
                let Some(link) = a.links[g][s].as_mut() else { continue };
                let want = !self.live_hint.get(g).and_then(|r| r.get(s)).copied().unwrap_or(true) && self.focus != Some(g);
                match link.state {
                    LinkState::Live if want => {
                        let has = self.sounds.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some)
                            || self.instruments.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some);
                        if !has || link.spare.is_none() {
                            continue;
                        }
                        let mut parts = link.spare.take().unwrap();
                        parts.smp = self.sounds.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::take);
                        parts.inst = self.instruments.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::take);
                        parts.chain = self.fx.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::take);
                        let smp_gain =
                            self.bufs.get(g).and_then(|r| r.get(s)).and_then(Option::as_deref).map_or((1.0, 1.0), |b| b.smp_gain);
                        link.gains[0] = parts.chain.as_ref().map_or(smp_gain, |c| c.gain);
                        link.gains[1] = parts.chain.as_ref().map_or(parts.inst.as_ref().map_or((0.0, 0.0), |i| i.gain), |c| c.gain);
                        link.finite = parts.chain.is_some();
                        // Its note-offs to come go with it.
                        link.offs.clear();
                        let offs = &mut link.offs;
                        self.note_offs.retain(|&(at, og, os, pitch, id)| {
                            if (og, os) == (g, s) {
                                offs.push((at, id, pitch));
                                false
                            } else {
                                true
                            }
                        });
                        let offs = std::mem::take(&mut link.offs);
                        link.ring.restart(now);
                        // It starts knowing where the song is (its notes were sent above).
                        let position = ahead::Ev::Position {
                            frame: self.frame,
                            playing: self.sequencing,
                            tempo: self.tempo,
                            numerator: self.numerator,
                            denominator: self.denominator,
                        };
                        let _ = link.inbox.push(ahead::Msg::At(now, position, false));
                        if let Err(back) = link.inbox.push(ahead::Msg::Install { at: now, parts, offs }) {
                            // No room: it plays on here.
                            if let ahead::Msg::Install { mut parts, offs, .. } = back {
                                if let Some(slot) = self.sounds.get_mut(g).and_then(|r| r.get_mut(s)) {
                                    *slot = parts.smp.take();
                                }
                                if let Some(slot) = self.instruments.get_mut(g).and_then(|r| r.get_mut(s)) {
                                    *slot = parts.inst.take();
                                }
                                if let Some(slot) = self.fx.get_mut(g).and_then(|r| r.get_mut(s)) {
                                    *slot = parts.chain.take();
                                }
                                link.spare = Some(parts);
                                link.offs = offs;
                            }
                            continue;
                        }
                        link.state = LinkState::Ahead;
                    }
                    LinkState::Ahead if !want => {
                        // Back from the furthest it may have rendered (players stop at the
                        // span's end plus the lookahead), so the frame is known now.
                        let at = now + a.lookahead;
                        if link.inbox.push(ahead::Msg::Return(at)).is_ok() {
                            link.state = LinkState::Returning(at);
                        }
                    }
                    _ => {}
                }
            }
        }
        a.driver.wake();
    }

    /// Take what players sent back; sounds handed back play here from their frame.
    fn ahead_returns(&mut self) {
        let Some(a) = self.ahead.as_mut() else { return };
        // A sound due back now whose parts haven't come yet: wait for them (its player
        // is finishing the last frames). Other sounds' messages may come first, so wait
        // for these ones', not for any.
        let now = self.time;
        let late = |a: &Ahead| {
            a.links.iter().flatten().flatten().any(|l| matches!(l.state, LinkState::Returning(at) if at <= now) && l.returned.is_none())
        };
        let mut waited: Option<std::time::Instant> = None;
        loop {
            while let Some(back) = a.driver.shared.back.pop() {
                match back {
                    ahead::Back::Returned { g, s, at, parts, offs, live } => {
                        if let Some(link) = a.links.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut) {
                            link.returned = Some((at, parts, offs, live));
                        }
                    }
                    ahead::Back::Spare { g, s, parts } => match a.links.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut) {
                        Some(link) if link.spare.is_none() => link.spare = Some(parts),
                        _ => a.junk.push(Junk(ahead::JunkKind::Parts(parts))),
                    },
                }
            }
            if !late(a) {
                break;
            }
            let start = *waited.get_or_insert_with(|| {
                a.driver.wake();
                std::time::Instant::now()
            });
            let limit = if a.wait { std::time::Duration::from_secs(10) } else { a.budget };
            if start.elapsed() >= limit {
                break;
            }
            std::hint::spin_loop();
        }
        // Those whose frame has come play here now.
        let now = self.time;
        let mut due = [(0usize, 0usize); 16];
        let mut n = 0;
        for (g, row) in a.links.iter().enumerate() {
            for (s, l) in row.iter().enumerate() {
                if let Some(l) = l
                    && let Some((at, ..)) = &l.returned
                    && *at <= now
                    && n < due.len()
                {
                    due[n] = (g, s);
                    n += 1;
                }
            }
        }
        for &(g, s) in &due[..n] {
            let a = self.ahead.as_mut().unwrap();
            let link = a.links[g][s].as_mut().unwrap();
            let (_, mut parts, mut offs, live) = link.returned.take().unwrap();
            // A sound an edit took away (an undo, say): its parts and notes go.
            let Some(slot) = self.sounds.get_mut(g).and_then(|r| r.get_mut(s)) else {
                link.state = LinkState::Live;
                if link.spare.is_none() {
                    link.spare = Some(parts);
                } else {
                    a.junk.push(Junk(ahead::JunkKind::Parts(parts)));
                }
                offs.clear();
                link.offs = offs;
                drop(live);
                continue;
            };
            *slot = parts.smp.take();
            if let Some(slot) = self.instruments[g].get_mut(s) {
                *slot = parts.inst.take();
            }
            if let Some(slot) = self.fx.get_mut(g).and_then(|r| r.get_mut(s)) {
                *slot = parts.chain.take();
            }
            for (at, id, pitch) in offs.drain(..) {
                self.note_offs.push((at, g, s, pitch, id));
            }
            link.offs = offs;
            if link.spare.is_none() {
                link.spare = Some(parts);
            } else {
                a.junk.push(Junk(ahead::JunkKind::Parts(parts)));
            }
            link.state = LinkState::Live;
            // (Edits while it was away may have moved its strips.)
            self.refresh_sound_strips(g, s);
            // Live input its player hadn't played yet plays now.
            for &(_, ev) in &live {
                match ev {
                    ahead::Ev::NoteOn { pitch, velocity, gate, .. } => {
                        let ticks = gate.map_or(HELD, |f| (f as f64 / self.samples_per_tick()).max(1.0) as i64);
                        self.note_on(g, s, pitch, velocity, ticks);
                    }
                    ahead::Ev::Release { pitch } => self.note_release(g, s, pitch),
                    ahead::Ev::Control(kind) => self.control_in(g, s, kind, false),
                    _ => {}
                }
            }
            if let Some(a) = self.ahead.as_mut() {
                a.junk.push(Junk(ahead::JunkKind::Live(live)));
            }
        }
    }

    /// Send players their notes for the coming frames (from now to the end of their
    /// lookahead past this span), apart from those sent already: after a jump or an
    /// edit, notes that are new arrive (late, at once), none twice.
    fn send_ahead(&mut self, frames: u64) {
        self.send_notes(frames, None);
    }

    /// [`Engine::send_ahead`] for every sound rendered ahead, or (`only`) just one,
    /// whatever its state (it is about to be handed to its player).
    fn send_notes(&mut self, frames: u64, only: Option<(usize, usize)>) {
        let Some(a) = self.ahead.as_mut() else { return };
        if only.is_none() {
            a.budget = std::time::Duration::from_secs_f64(frames as f64 / self.sr * 0.5);
            a.span = frames;
        }
        // Players render to the end of this span and their lookahead past it.
        let span_end = self.time + frames;
        // What came before now is played (or was never wanted: a sound with nothing to
        // mix, an audio mute): players may write over it.
        if only.is_none() {
            for l in a.links.iter().flatten().flatten().filter(|l| l.state != LinkState::Live) {
                l.ring.read_to(self.time);
            }
        }
        if only.is_none() && !a.links.iter().flatten().flatten().any(|l| l.state == LinkState::Ahead) {
            a.driver.advance(span_end);
            return;
        }
        let window = a.lookahead + frames;
        let now = self.time;
        let spt = self.samples_per_tick();
        let tps = 1.0 / spt;
        let (tempo, num, den) = (self.tempo, self.numerator, self.denominator);
        // Forget what was sent for frames gone by.
        let a = self.ahead.as_mut().unwrap();
        a.sent.retain(|x| x.5 >= now);
        let mut ahead = self.ahead.take().unwrap();
        let muted = &self.sound_muted;
        let inputs = &self.midi_inputs;
        self.foresee(0, window, |seen| match seen {
            Seen::Event { t, e, plays, frame, pass } => {
                if !plays || note_muted(muted, &e) {
                    return;
                }
                // Its notes go to its MIDI input plug-in, when they're due.
                if e.cue.is_none() && e.midi.is_none() && inputs.get(e.group).is_some_and(Option::is_some) {
                    return;
                }
                let Some(link) = ahead.links.get(e.group).and_then(|r| r.get(e.sound)).and_then(Option::as_ref) else { return };
                let wanted = match only {
                    Some(gs) => gs == (e.group, e.sound),
                    None => link.state == LinkState::Ahead,
                };
                if !wanted {
                    return;
                }
                let key = (e.tick, e.group, e.sound, e.pitch, pass);
                if ahead.sent.iter().any(|x| (x.0, x.1, x.2, x.3, x.4) == key) {
                    return;
                }
                let ev = match (e.cue, e.midi) {
                    (Some(c), _) => ahead::Ev::Cue(cue_take(c), e.tick as f64 / PPQ as f64),
                    (None, Some(kind)) => ahead::Ev::Control(kind),
                    (None, None) => {
                        ahead.next_id += 1;
                        let gate = (e.length != HELD).then(|| ((e.length.max(1) as f64) / tps).max(1.0) as u64);
                        ahead::Ev::NoteOn {
                            id: ahead.next_id,
                            pitch: e.pitch,
                            velocity: e.velocity,
                            gate,
                            beats: frame as f64 / spt / PPQ as f64,
                        }
                    }
                };
                if link.inbox.push(ahead::Msg::At(now + t, ev, false)).is_ok() && ahead.sent.len() < ahead.sent.capacity() {
                    ahead.sent.push((e.tick, e.group, e.sound, e.pitch, pass, now + t));
                }
            }
            Seen::Position { t, frame, playing } => {
                let ev = ahead::Ev::Position { frame, playing, tempo, numerator: num, denominator: den };
                for (g, row) in ahead.links.iter().enumerate() {
                    for (s, l) in row.iter().enumerate() {
                        let Some(l) = l else { continue };
                        if only.map_or(l.state == LinkState::Ahead, |gs| gs == (g, s)) {
                            let _ = l.inbox.push(ahead::Msg::At(now + t, ev, false));
                        }
                    }
                }
            }
        });
        if only.is_none() {
            ahead.driver.advance(span_end);
        }
        self.ahead = Some(ahead);
    }

    /// Send a live event to sound (`g`, `s`)'s player: it plays it a lookahead and a
    /// span later, a frame its player can't have rendered yet, so live input keeps its
    /// timing (with that much more delay).
    fn send_live(&mut self, g: usize, s: usize, ev: ahead::Ev) {
        let Some(a) = &self.ahead else { return };
        let at = self.time + a.lookahead + a.span;
        if let Some(l) = self.link_ahead(g, s) {
            let _ = l.inbox.push(ahead::Msg::At(at, ev, true));
            if let Some(a) = &self.ahead {
                a.driver.wake();
            }
        }
    }

    /// Fill sound (`g`, `s`)'s buffers with what its player rendered for the next
    /// `frames` frames (waiting a moment if it isn't there yet).
    fn ahead_fill(&mut self, g: usize, s: usize, frames: usize) {
        let now = self.time;
        let Some(a) = self.ahead.as_mut() else { return };
        let budget = a.budget;
        let Some(link) = a.links.get(g).and_then(|r| r.get(s)).and_then(Option::as_ref) else { return };
        let Some(b) = self.bufs.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_deref_mut) else { return };
        let need = now + frames as u64;
        if link.ring.written() < need {
            let start = std::time::Instant::now();
            let limit = if a.wait { std::time::Duration::from_secs(10) } else { budget };
            a.driver.wake();
            while link.ring.written() < need && start.elapsed() < limit {
                std::hint::spin_loop();
            }
        }
        let have = (link.ring.written().saturating_sub(now) as usize).min(frames);
        link.ring.read_into(now, have, &mut b.smp, &mut b.inst);
        b.smp[have * 2..frames * 2].fill(0.0);
        b.inst[have * 2..frames * 2].fill(0.0);
        link.ring.read_to(need);
        b.smp_out = true;
        b.inst_out = true;
        if have < frames {
            a.underruns += 1;
        }
    }

    /// Play `event` `offset` frames into the next block (at once if the schedule is
    /// full). Live input placed this way keeps its timing to the sample.
    pub fn schedule(&mut self, offset: u64, event: Live) {
        if self.live.len() < LIVE_ROOM {
            self.live.push((self.time + offset, event));
        } else {
            self.play_live(event);
        }
    }

    /// The metronome's volume, 0..1.
    pub fn set_metronome_volume(&mut self, volume: f32) {
        self.click.volume = volume.clamp(0.0, 1.0);
    }

    /// Count in for `frames` (the metronome clicks, the song waits), then play.
    pub fn count_in(&mut self, frames: u64) {
        if frames > 0 {
            self.sequencing = false;
            self.counting = frames;
            self.count_frame = 0;
        }
    }

    /// Note Repeat or the Arp on (with these settings) or off. Its settings change
    /// without letting go of what's held.
    pub fn set_repeat(&mut self, repeat: Option<(deckard_model::Arp, repeat::RepeatMode)>) {
        match (repeat, self.repeater.as_mut()) {
            (Some((arp, mode)), Some(r)) => {
                if !arp.hold && r.arp.hold {
                    r.clear();
                }
                r.arp = arp;
                r.mode = mode;
            }
            (Some((arp, mode)), None) => self.repeater = Some(repeat::Repeater::new(arp, mode)),
            (None, _) => self.repeater = None,
        }
    }

    /// Whether Note Repeat or the Arp is on.
    pub fn repeating(&self) -> bool {
        self.repeater.is_some()
    }

    /// Put a MIDI input plug-in on `group`, or take it out with `None`. The notes the old
    /// one holds end. Returns it, to be freed off the audio thread (`plugin` itself if
    /// there's no such slot).
    pub fn set_midi_input(
        &mut self,
        group: usize,
        plugin: Option<Box<dyn midi_input::MidiInput>>,
    ) -> Option<Box<dyn midi_input::MidiInput>> {
        let Some(slot) = self.midi_inputs.get_mut(group) else { return plugin };
        let mut old = std::mem::replace(slot, plugin);
        if let Some(p) = old.as_mut() {
            let clock = self.midi_input_clock(0.0);
            let mut out = std::mem::take(&mut self.midi_input_out);
            p.finish(clock, &mut midi_input::Outbox::new(&mut out));
            self.play_midi_input_out(group, clock.now, &out);
            self.midi_input_out = out;
        }
        old
    }

    /// Move a parameter of `group`'s MIDI input plug-in, if it has one there.
    pub fn set_midi_input_param(&mut self, group: usize, id: u32, value: f64) {
        if let Some(p) = self.midi_inputs.get_mut(group).and_then(Option::as_mut) {
            p.set_param(id, value);
        }
    }

    /// The plug-ins go on with an engine that replaces this one. What they hold has been
    /// ended, as the old engine's notes are.
    #[cfg(any(test, feature = "output"))]
    pub(crate) fn take_midi_inputs(&mut self, old: &mut Engine) {
        old.live.retain(|(_, e)| !matches!(e, Live::MidiInputOut { .. }));
        std::mem::swap(&mut self.midi_inputs, &mut old.midi_inputs);
        for p in self.midi_inputs.iter_mut().flatten() {
            p.stopped();
        }
    }

    /// Whether `group` has a MIDI input plug-in.
    pub fn has_midi_input(&self, group: usize) -> bool {
        self.midi_inputs.get(group).is_some_and(Option::is_some)
    }

    /// Whether `group`'s notes go to a MIDI input plug-in (not one it's playing itself).
    fn midi_input_takes(&self, group: usize) -> bool {
        !self.from_midi_input && self.midi_inputs.get(group).is_some_and(Option::is_some)
    }

    fn midi_input_clock(&self, span: f64) -> midi_input::Clock {
        let spt = self.samples_per_tick();
        midi_input::Clock {
            now: self.time as f64 / spt,
            span,
            song: self.sequencing.then(|| self.frame as f64 / spt),
            seconds: self.time as f64 / self.sr,
        }
    }

    /// A note into `group`'s MIDI input plug-in, and what it plays for it.
    fn midi_input_note(&mut self, group: usize, note: midi_input::NoteIn) {
        let Some(mut p) = self.midi_inputs.get_mut(group).and_then(Option::take) else { return };
        let clock = self.midi_input_clock(0.0);
        let mut out = std::mem::take(&mut self.midi_input_out);
        p.note(note, clock, &mut midi_input::Outbox::new(&mut out));
        self.midi_inputs[group] = Some(p);
        self.midi_inputs_unsettled |= 1 << group;
        self.play_midi_input_out(group, clock.now, &out);
        self.midi_input_out = out;
    }

    /// Every note due at this instant is in: the plug-ins that had one work on them.
    fn midi_inputs_settle(&mut self) {
        let mut slots = std::mem::take(&mut self.midi_inputs_unsettled);
        while slots != 0 {
            let group = slots.trailing_zeros() as usize;
            slots &= slots - 1;
            let Some(mut p) = self.midi_inputs.get_mut(group).and_then(Option::take) else { continue };
            let clock = self.midi_input_clock(0.0);
            let mut out = std::mem::take(&mut self.midi_input_out);
            p.settle(clock, &mut midi_input::Outbox::new(&mut out));
            self.midi_inputs[group] = Some(p);
            self.play_midi_input_out(group, clock.now, &out);
            self.midi_input_out = out;
        }
    }

    /// What the MIDI input plug-ins play by themselves over the next `frames`.
    fn midi_inputs_ahead(&mut self, frames: usize) {
        for group in 0..self.midi_inputs.len() {
            let Some(mut p) = self.midi_inputs[group].take() else { continue };
            let clock = self.midi_input_clock(frames as f64 / self.samples_per_tick());
            let mut out = std::mem::take(&mut self.midi_input_out);
            p.process(clock, &mut midi_input::Outbox::new(&mut out));
            self.midi_inputs[group] = Some(p);
            self.play_midi_input_out(group, clock.now, &out);
            self.midi_input_out = out;
        }
    }

    /// Play notes a plug-in played: at once, or queued for their frames.
    fn play_midi_input_out(&mut self, group: usize, now: f64, notes: &[midi_input::NoteOut]) {
        let spt = self.samples_per_tick();
        for &note in notes {
            let at = match note {
                midi_input::NoteOut::On { at, .. } | midi_input::NoteOut::Off { at, .. } => at,
            };
            let offset = ((at - now) * spt).round().max(0.0) as u64;
            if offset == 0 {
                self.play_midi_input_note(group, note);
            } else {
                self.schedule(offset, Live::MidiInputOut { group, note });
            }
        }
    }

    /// A note a plug-in played sounds, as a live note would (a pattern's, if it was one,
    /// has become one).
    fn play_midi_input_note(&mut self, group: usize, note: midi_input::NoteOut) {
        let timeline = std::mem::replace(&mut self.from_timeline, false);
        self.from_midi_input = true;
        match note {
            midi_input::NoteOut::On { sound, pitch, velocity, length, .. } => {
                self.note_on(group, sound, pitch, velocity, length.unwrap_or(HELD))
            }
            midi_input::NoteOut::Off { sound, pitch, .. } => self.note_release(group, sound, pitch),
        }
        self.from_midi_input = false;
        self.from_timeline = timeline;
    }

    /// Record live notes (REC) or stop.
    pub fn set_recording(&mut self, on: bool) {
        self.recording = on;
    }

    /// The live notes recorded since last asked.
    pub fn take_recorded(&mut self) -> Vec<Recorded> {
        std::mem::take(&mut self.recorded)
    }

    /// The per-note expression recorded since last asked.
    pub fn take_recorded_expr(&mut self) -> Vec<RecordedExpr> {
        std::mem::take(&mut self.recorded_expr)
    }

    /// The channel-wide controllers recorded since last asked.
    pub fn take_recorded_controls(&mut self) -> Vec<RecordedControl> {
        std::mem::take(&mut self.recorded_controls)
    }

    /// Prehear `sample` (the browser's preview: once, at `gain`, beside the song;
    /// `None` stops it). Returns the sample it replaces, to be freed off the audio
    /// thread.
    pub fn prehear(&mut self, sample: Option<Arc<sample::Sample>>, gain: f32) -> Option<Arc<sample::Sample>> {
        self.prehear.set(sample, gain)
    }

    /// Play live events, then send the MPE notes they started (see
    /// [`Engine::flush_waiting`]).
    fn play_live(&mut self, event: Live) {
        self.batching = true;
        self.play_live_one(event);
        self.batching = false;
        self.flush_waiting();
    }

    fn play_live_one(&mut self, event: Live) {
        // Held keys count for the modulation sources while held.
        match event {
            Live::NoteOn { group, sound, pitch, length: HELD, .. } | Live::NoteIn { group, sound, pitch, .. } => {
                self.touch_keys(group, sound, pitch, true)
            }
            Live::NoteRelease { group, sound, pitch } => self.touch_keys(group, sound, pitch, false),
            _ => {}
        }
        // Held pads (and keys) feed Note Repeat / the Arp, which plays them on its grid.
        if let Some(r) = self.repeater.as_mut() {
            match event {
                Live::NoteOn { group, sound, pitch, velocity, length: i64::MAX } => {
                    r.press(group, sound, pitch, velocity);
                    return;
                }
                Live::NoteRelease { group, sound, pitch } => {
                    r.release(group, sound, pitch);
                    return;
                }
                Live::RepeatPressure { group, sound, pitch, velocity } => {
                    r.pressure(group, sound, pitch, velocity);
                    return;
                }
                _ => {}
            }
        }
        // The count-in's last 1/16 records too, at where the song starts.
        let pickup = self.counting > 0 && self.sync.waiting == 0 && self.counting as f64 <= PICKUP as f64 * self.samples_per_tick();
        if self.recording && (self.sequencing || pickup) {
            let (tick, mode, seq) = (self.position(), self.mode, self.record_seq);
            let played = self.played();
            self.record_seq += 1;
            match event {
                Live::Expression { group, sound, pitch, kind, value } if self.recorded_expr.len() < RECORDED_ROOM => {
                    self.recorded_expr.push(RecordedExpr { group, sound, pitch, kind, value, tick, mode, seq, played });
                }
                Live::ControlIn { group, sound, kind, record: true, .. } if self.recorded_controls.len() < RECORDED_ROOM => {
                    self.recorded_controls.push(RecordedControl { group, sound, kind, tick, mode, seq });
                }
                _ => {}
            }
            let midi = matches!(event, Live::NoteIn { .. });
            let note = match event {
                Live::NoteOn { group, sound, pitch, velocity, length } => Some((group, sound, pitch, velocity, length)),
                Live::NoteIn { group, sound, pitch, velocity, .. } => Some((group, sound, pitch, velocity, i64::MAX)),
                Live::NoteRelease { group, sound, pitch } => Some((group, sound, pitch, 0, 0)),
                Live::ControlIn { .. }
                | Live::Expression { .. }
                | Live::RepeatPressure { .. }
                | Live::MidiInputOut { .. }
                | Live::ChokeAll => None,
            };
            if let Some((group, sound, pitch, velocity, length)) = note
                && self.recorded.len() < RECORDED_ROOM
            {
                self.recorded.push(Recorded { group, sound, pitch, velocity, length, tick, mode, seq, played, midi });
            }
        }
        match event {
            Live::NoteOn { group, sound, pitch, velocity, length } => {
                self.live_hit = (self.live_hit.0 + 1, group, sound, pitch, velocity);
                self.hit_pads(group, sound, pitch, |e, s, p| e.note_on(group, s, p, velocity, length));
            }
            Live::NoteRelease { group, sound, pitch } => {
                let (plays, _) = self.pads.hit(group, sound);
                for s in pads::bits(plays) {
                    let p = self.pads.pitch(group, sound, s, pitch);
                    self.note_release(group, s, p);
                }
            }
            Live::NoteIn { group, sound, pitch, velocity, thru } => {
                self.live_hit = (self.live_hit.0 + 1, group, sound, pitch, velocity);
                self.hit_pads(group, sound, pitch, |e, s, p| e.note_in(group, s, p, velocity, thru));
            }
            Live::ControlIn { group, sound, kind, thru, .. } => self.control_in(group, sound, kind, thru),
            Live::MidiInputOut { group, note } => self.play_midi_input_note(group, note),
            // Expression follows its held note out (above, it's recorded): on an MPE
            // port all of it on the note's channel, on a normal port its pressure as poly
            // aftertouch; a release velocity goes on the note-off.
            Live::Expression { group, sound, pitch, kind, value } => {
                let (plays, _) = self.pads.hit(group, sound);
                for s in pads::bits(plays) {
                    let p = self.pads.pitch(group, sound, s, pitch);
                    self.touch_live((group, s, p), kind, value);
                    self.expression_out((group, s, p), kind, value);
                }
            }
            Live::RepeatPressure { .. } => {}
            Live::ChokeAll => self.choke_all(),
        }
    }

    /// Every voice of every sound ends: each sound as its choke group would end it.
    fn choke_all(&mut self) {
        let tps = 1.0 / self.samples_per_tick();
        let shape: Vec<usize> = self.fx.iter().map(Vec::len).collect();
        for (group, sounds) in shape.into_iter().enumerate() {
            for sound in 0..sounds {
                let e = NoteEvent {
                    tick: 0,
                    group,
                    sound,
                    pitch: 0,
                    velocity: 0,
                    length: 0,
                    probability: 100,
                    midi: Some(pads::CHOKE),
                    cue: None,
                    expr: None,
                    from: None,
                };
                self.trigger(e, tps);
            }
        }
    }

    /// A live note on `sound`: choke what it chokes, then `play` it and the sounds it
    /// links, each at its pitch (see [`pads`]).
    fn hit_pads(&mut self, group: usize, sound: usize, pitch: i64, mut play: impl FnMut(&mut Self, usize, i64)) {
        let (plays, chokes) = self.pads.hit(group, sound);
        let tps = 1.0 / self.samples_per_tick();
        for s in pads::bits(chokes) {
            let e = NoteEvent {
                tick: 0,
                group,
                sound: s,
                pitch: 0,
                velocity: 0,
                length: 0,
                probability: 100,
                midi: Some(pads::CHOKE),
                cue: None,
                expr: None,
                from: None,
            };
            self.trigger(e, tps);
        }
        for s in pads::bits(plays) {
            let p = self.pads.pitch(group, sound, s, pitch);
            play(self, s, p);
        }
    }

    /// Play the live events due by now, in the order they came.
    fn fire_live(&mut self) {
        self.batching = true;
        while let Some(i) = self.live.iter().position(|(at, _)| *at <= self.time) {
            let (_, e) = self.live.remove(i);
            self.play_live_one(e);
        }
        self.batching = false;
        self.flush_waiting();
    }

    /// Send the live MPE notes started by the live events just played: each one's
    /// starting bend, slide and pressure (an MPE controller's come just after its
    /// note-on), then its note-on.
    fn flush_waiting(&mut self) {
        let Some(tx) = &self.midi_tx else { return };
        for n in self.midi_offs.iter_mut() {
            let Some(velocity) = n.waiting.take() else { continue };
            let (ch, [bend, slide, pressure]) = (n.channel, n.sent.map(|v| v.unwrap_or(0)));
            for (bytes, len) in [
                ([0xe0 | ch, (bend & 0x7f) as u8, (bend >> 7) as u8], 3),
                ([0xb0 | ch, 74, slide as u8], 3),
                ([0xd0 | ch, pressure as u8, 0], 2),
                ([0x90 | ch, n.note, velocity], 3),
            ] {
                let _ = tx.try_send(midi::MidiOut { port: n.port.clone(), bytes, len, at: None });
            }
        }
    }

    /// Whether any sound takes audio from the interface.
    pub fn takes_input(&self) -> bool {
        !self.in_routes.is_empty()
    }

    /// The interface input for the next `frames`, into the buffers of the sounds that
    /// take it (they play it in [`Engine::render_pass`]).
    fn take_input(&mut self, frames: usize) {
        for b in self.bufs.iter_mut().flatten().flatten() {
            if b.inp_on {
                let end = (frames * 2).min(b.inp.len());
                b.inp[..end].fill(0.0);
                b.inp_on = false;
            }
        }
        let Some(live) = &self.live_in else { return };
        let Ok(mut q) = live.try_lock() else { return };
        // Keep up: never more than a few blocks behind.
        while q.len() > frames * 4 + 512 {
            q.pop_front();
        }
        for &(g, s, ..) in &self.in_routes {
            if let Some(b) = self.bufs.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut) {
                if b.inp.len() < frames * 2 {
                    b.inp.resize(frames * 2, 0.0);
                }
                b.inp[..frames * 2].fill(0.0);
                b.inp_on = true;
            }
        }
        for f in 0..frames {
            let Some(frame) = q.pop_front() else { break };
            for &(g, s, l, r, (gl, gr)) in &self.in_routes {
                if let Some(b) = self.bufs.get_mut(g).and_then(|row| row.get_mut(s)).and_then(Option::as_mut) {
                    (b.inp[f * 2], b.inp[f * 2 + 1]) = (frame[l] * gl, frame[r] * gr);
                }
            }
        }
    }

    /// Fill interleaved stereo `out` (length = 2 × frames).
    pub fn process(&mut self, out: &mut [f32]) {
        self.process_outputs(out, &mut []);
    }

    /// [`Engine::process`], with what goes to the other external outputs: `ext[n]` is
    /// Out n+1 (interleaved stereo, as long as `out` or longer; added to). Outputs it
    /// has no buffer for are dropped. `out` is the master's output, whichever that is
    /// ([`Engine::master_bus`]).
    pub fn process_outputs(&mut self, out: &mut [f32], ext: &mut [Vec<f32>]) {
        // In spans no longer than the sounds' buffers.
        for (i, span) in out.chunks_mut(JOB_FRAMES * 2).enumerate() {
            self.process_span(span, ext, i * JOB_FRAMES * 2);
        }
    }

    /// The external output the master plays on (Out n, from 0).
    pub fn master_bus(&self) -> Option<usize> {
        self.master_bus
    }

    fn process_span(&mut self, out: &mut [f32], ext: &mut [Vec<f32>], at: usize) {
        out.fill(0.0);
        let frames = out.len() / 2;
        self.take_input(frames);
        // Players' notes for the frames ahead of this span.
        self.send_ahead(frames as u64);
        self.repeat_ahead(frames);
        self.midi_inputs_ahead(frames);
        self.follow_scene_tempo();
        let mut tps = 1.0 / self.samples_per_tick();
        let mut end = self.frame_of(self.timeline.length);
        let mut done = 0;
        while done < frames {
            if self.follow_scene_tempo() {
                tps = 1.0 / self.samples_per_tick();
                end = self.frame_of(self.timeline.length);
            }
            self.ahead_returns();
            self.fire_note_offs();
            self.play_expression();
            self.fire_live();
            if let Some((at, to)) = self.jump.filter(|_| self.sequencing)
                && self.frame >= self.frame_of(at)
            {
                self.seek(to);
            }
            if self.sequencing {
                // Events due now.
                while let Some(&e) = self.timeline.events.get(self.next) {
                    if self.frame_of(e.tick) > self.frame {
                        break;
                    }
                    let idx = self.next;
                    self.next += 1;
                    let plays = chance(e.probability, idx, self.timeline.chance_pass(&e, self.pass), self.rng);
                    #[cfg(test)]
                    self.fired.push((self.time, e.tick, e.sound, e.pitch, plays));
                    if plays {
                        self.from_timeline = true;
                        self.trigger(e, tps);
                        self.from_timeline = false;
                    }
                }
                if self.next >= self.timeline.events.len() && self.frame >= end {
                    if let Some((mode, timeline)) = self.queued.take() {
                        self.frame -= end.min(self.frame);
                        self.next = 0;
                        self.pass += 1;
                        self.mode = mode;
                        self.timeline = timeline;
                        end = self.frame_of(self.timeline.length);
                        continue;
                    }
                    if self.looping && end > 0 {
                        self.frame -= end;
                        self.next = 0;
                        self.pass += 1;
                        continue;
                    }
                    self.sequencing = false;
                }
            }
            self.midi_inputs_settle();
            self.lfo_tick();
            self.apply_mods();
            if !self.cc_routes.is_empty() {
                self.send_ccs();
            }
            // Render up to the next event (or the end of the timeline).
            let mut until = (frames - done) as u64;
            if self.sequencing {
                let mut target = self.timeline.events.get(self.next).map_or(end, |e| self.frame_of(e.tick));
                if let Some((at, _)) = self.jump {
                    target = target.min(self.frame_of(at));
                }
                if let Some(at) = self.timeline.next_tempo(self.position() as i64) {
                    target = target.min(self.frame_of(at));
                }
                until = until.min(target.saturating_sub(self.frame).max(1));
            }
            if let Some(at) = self.live.iter().map(|o| o.0).min() {
                until = until.min(at.saturating_sub(self.time).max(1));
            }
            // A sound handed back by its player plays here from its frame on.
            if let Some(a) = &self.ahead {
                for l in a.links.iter().flatten().flatten() {
                    if let LinkState::Returning(at) = l.state
                        && at > self.time
                    {
                        until = until.min(at - self.time);
                    }
                }
            }
            if let Some(at) = self.note_offs.iter().map(|o| o.0).min() {
                until = until.min(at.saturating_sub(self.time).max(1));
            }
            // Notes playing expression out: often enough to follow their curves.
            if self.midi_offs.iter().any(|n| n.expr.is_some() && n.at > self.time) {
                until = until.min(self.expr_step());
            }
            if self.counting > 0 {
                until = until.min(self.counting);
            }
            if self.sync.waiting > 0 {
                until = until.min(self.sync.waiting);
            }
            let until = until as usize;
            if self.tap.as_ref().is_some_and(|t| t.source != TapSource::Master) {
                self.tap_block.clear();
                self.tap_block.resize(until * 2, 0.0);
            }
            // Sounds that feed side-chains (and all others) first, then the sounds whose
            // effects listen to them.
            for b in &mut self.sc_bufs {
                b.clear();
                b.resize(until * 2, 0.0);
            }
            self.inp_at = done;
            for pass in 0..2 {
                self.render_pass(&mut out[done * 2..(done + until) * 2], ext, at + done * 2, pass);
            }
            // The groups' effects and faders, then the master's effects.
            self.run_buses(&mut out[done * 2..(done + until) * 2], ext, at + done * 2);
            if self.tap.as_ref().is_some_and(|t| t.source != TapSource::Master) {
                let block = std::mem::take(&mut self.tap_block);
                self.flush_tap(&block);
                self.tap_block = block;
            }
            let ticks = if self.metronome_ticks > 0 { i64::from(self.metronome_ticks) } else { self.denominator.max(1) as i64 };
            let beat_frames = self.samples_per_tick() * (PPQ * 4) as f64 / ticks as f64;
            // (Its ticks in a bar: the first higher.)
            let per_bar = (self.numerator as i64 * ticks / self.denominator.max(1) as i64).max(1);
            let frame = if self.sync.waiting > 0 {
                None
            } else if self.counting > 0 {
                Some(self.count_frame)
            } else {
                (self.sequencing && (self.metronome || (self.metronome_auto && self.recording))).then_some(self.frame)
            };
            let part = until as usize;
            let clicks = &mut self.cue.clicks[..part * 2];
            clicks.fill(0.0);
            self.click.mix(clicks, frame, beat_frames, per_bar, self.sr);
            self.prehear.mix(clicks, self.sr);
            // On the cue bus's output; one the interface hasn't: the master's.
            let on_ext = self.cue.clicks_route.0.and_then(|n| ext.get_mut(n)?.get_mut(at + done * 2..at + (done + part) * 2));
            self.cue.finish(part, on_ext.unwrap_or(&mut out[done * 2..(done + part) * 2]));
            self.clock_span(until as u64);
            if self.sequencing {
                self.frame += until as u64;
            }
            // Waiting for a clock's bar, then the count-in; their end: the song plays
            // from the next frame.
            let waited = self.counting > 0 || self.sync.waits();
            if self.sync.waiting > 0 {
                self.sync.waiting -= (until as u64).min(self.sync.waiting);
                if self.sync.waiting == 0 && self.counting == 0 {
                    self.sequencing = true;
                }
            } else if self.counting > 0 {
                self.counting -= (until as u64).min(self.counting);
                self.count_frame += until as u64;
                if self.counting == 0 {
                    self.sequencing = true;
                }
            }
            self.lfo_clock_run(until as u64, waited);
            if waited && self.sequencing {
                self.lfo_clock_start();
            }
            self.time += until as u64;
            done += until;
        }
        // The master channel; never pass on NaN or runaway levels (a safety net for
        // speakers and ears).
        let gain = [self.master * self.master_pan.0, self.master * self.master_pan.1];
        for (i, s) in out.iter_mut().enumerate() {
            let v = *s * gain[i % 2];
            *s = if v.is_finite() { v.clamp(-4.0, 4.0) } else { 0.0 };
            if i % 2 == 0 {
                self.master_peak.0 = self.master_peak.0.max(s.abs());
            } else {
                self.master_peak.1 = self.master_peak.1.max(s.abs());
            }
        }
        if self.tap.as_ref().is_some_and(|t| t.source == TapSource::Master) {
            self.flush_tap(out);
        }
        // Hand sounds to players or take them back, from the end of this span.
        if self.ahead.is_some() {
            self.rebalance();
        }
    }

    /// What Note Repeat / the Arp plays over the next `frames`, queued as live notes
    /// (with their lengths) at their frames.
    fn repeat_ahead(&mut self, frames: usize) {
        if self.repeater.is_none() {
            return;
        }
        let spt = self.samples_per_tick();
        let now = self.time as f64 / spt;
        let song = self.sequencing.then(|| self.frame as f64 / spt);
        let Some(r) = self.repeater.as_mut() else { return };
        let mut fired = std::mem::take(&mut self.repeat_buf);
        r.due_into(now, frames as f64 / spt, song, &mut fired);
        for &f in &fired {
            let offset = ((f.at - now) * spt).round().max(0.0) as u64;
            let note = Live::NoteOn { group: f.group, sound: f.sound, pitch: f.pitch, velocity: f.velocity, length: f.length };
            self.schedule(offset, note);
        }
        self.repeat_buf = fired;
    }

    /// Modulation: instruments and effects with a lane playing at the song's position
    /// take its values; ones whose lanes stopped (or the song) have their own again. No
    /// allocating (the audio thread).
    fn apply_mods(&mut self) {
        if !self.unrouted.is_empty() {
            self.retry_unroutes();
        }
        self.apply_faders();
        if !self.modulating
            && self.modded_plugins.is_empty()
            && (self.timeline.mods.is_empty() || !self.sequencing)
            && self.mod_routes.is_empty()
        {
            return;
        }
        let modulate_plugin = |modded_plugins: &mut Vec<(Shared, u32, bool)>, plugin: &Shared, id: u32, dev: f64| {
            if let Some(m) = modded_plugins.iter_mut().find(|(p, i, _)| *i == id && Arc::ptr_eq(p, plugin)) {
                m.2 = true;
            } else if modded_plugins.len() < modded_plugins.capacity() {
                modded_plugins.push((plugin.clone(), id, true));
            }
            if let Ok(mut p) = plugin.try_lock() {
                let target = (p.knob_value(id) + dev).clamp(0.0, 1.0);
                p.modulate(id, target);
            }
        };
        let tick = self.frame as f64 / self.samples_per_tick();
        if self.sequencing {
            for lane in &self.timeline.mods {
                if !((lane.from as f64)..(lane.to as f64)).contains(&tick) {
                    continue;
                }
                let v = lane.value_at(tick);
                if let Some(s) = lane.sound
                    && lane.slot == 0
                {
                    if let Some(Some(inst)) = self.instruments.get(lane.group).and_then(|r| r.get(s))
                        && let Ok(id) = lane.field.parse::<u32>()
                    {
                        modulate_plugin(&mut self.modded_plugins, &inst.plugin, id, v);
                    } else if let Some(Some(smp)) = self.sounds.get_mut(lane.group).and_then(|r| r.get_mut(s)) {
                        smp.begin_modulation();
                        smp.modulate(&lane.field, v);
                    }
                }
                let chain = match lane.sound {
                    Some(s) => self.fx.get_mut(lane.group).and_then(|r| r.get_mut(s)).and_then(Option::as_mut),
                    None => self.buses.get_mut(lane.group).and_then(Option::as_mut).map(|b| &mut b.chain),
                };
                let Some(chain) = chain else { continue };
                for u in chain.units.iter_mut().filter(|u| u.slot == lane.slot && !u.ghost) {
                    match &mut u.fx {
                        FxUnit::Internal { params, modded, hit, .. } => {
                            if !*hit {
                                modded.clone_from(params);
                                *hit = true;
                            }
                            if let Some(x) = modded.iter_mut().find(|(k, _)| *k == lane.field) {
                                x.1 = v;
                            }
                        }
                        FxUnit::Plugin { plugin, .. } => {
                            if let Ok(id) = lane.field.parse::<u32>() {
                                modulate_plugin(&mut self.modded_plugins, plugin, id, v);
                            }
                        }
                    }
                }
            }
        }
        self.apply_routes();
        let mut any = false;
        for smp in self.sounds.iter_mut().flatten().flatten() {
            any |= smp.finish_modulation();
        }
        let chains = self.fx.iter_mut().flatten().flatten().chain(self.buses.iter_mut().flatten().map(|b| &mut b.chain));
        for chain in chains {
            for u in &mut chain.units {
                let FxUnit::Internal { params, fx, glide, modded, hit, modulated, .. } = &mut u.fx else { continue };
                let to: Option<&[(String, f64)]> = if *hit {
                    Some(modded)
                } else if *modulated {
                    Some(params)
                } else {
                    None
                };
                if let Some(to) = to
                    && !glide.heading_for(to)
                {
                    fx.set(if glide.retarget(to) { glide.now() } else { to });
                }
                *modulated = *hit;
                *hit = false;
                any |= *modulated;
            }
        }
        let unrouted = &mut self.unrouted;
        let mod_routes = &self.mod_routes;
        self.modded_plugins.retain_mut(|(plugin, id, hit)| {
            if std::mem::take(hit) {
                return true;
            }
            if mod_routes.iter().any(|r| {
                !r.now.is_nan()
                    && matches!(&r.target, mod_rt::RouteTarget::Plugin { plugin: q, id: j } if *j == *id && Arc::ptr_eq(q, plugin))
            }) {
                return false;
            }
            match plugin.try_lock() {
                Ok(mut p) => p.unmodulate(*id),
                Err(_) if unrouted.len() < unrouted.capacity() => unrouted.push((plugin.clone(), *id)),
                Err(_) => {}
            }
            false
        });
        any |= !self.modded_plugins.is_empty();
        self.modulating = any;
    }

    /// Modulated faders take their lanes' places at the song's position: their own moved
    /// by the deviation there, or their own when no lane plays. No allocating (the audio
    /// thread).
    fn apply_faders(&mut self) {
        if self.faders.is_empty() {
            return;
        }
        let tick = self.frame as f64 / self.samples_per_tick();
        for f in &mut self.faders {
            let (mut level, mut pan) = (f.level, f.pan);
            if self.sequencing {
                let here =
                    |l: &&modscale::ModLane| l.group == f.group && l.sound == f.sound && ((l.from as f64)..(l.to as f64)).contains(&tick);
                for lane in self.timeline.faders.iter().filter(here) {
                    let dev = lane.value_at(tick);
                    match lane.field.as_str() {
                        "level" => level = (f.level + dev).clamp(0.0, 1.0),
                        // Its knob travels over −1..1.
                        "pan" => pan = (f.pan + dev * 2.0).clamp(-1.0, 1.0),
                        _ => (),
                    }
                }
            }
            f.gain = channel_gain(level, pan);
        }
    }

    /// The song's position now, `done` frames into the span being rendered.
    fn transport_at(&self, done: usize) -> Transport {
        let spt = self.samples_per_tick();
        Transport {
            tempo: self.tempo,
            playing: self.sequencing,
            quarters: (self.frame as f64 + done as f64) / spt / PPQ as f64,
            numerator: self.numerator,
            denominator: self.denominator,
        }
    }

    /// The groups' and bus sounds' buses (what their senders sent them over `out`'s
    /// frames): each through its effects, then its fader and sends onto where it goes;
    /// then the master's effects over `out`.
    fn run_buses(&mut self, out: &mut [f32], ext: &mut [Vec<f32>], at: usize) {
        let frames = out.len() / 2;
        let glide = (GAIN_GLIDE_SECONDS * self.sr).round() as u32;
        let base = self.transport_at(0);
        let spt = self.samples_per_tick();
        let pos = |done: usize| Transport { quarters: base.quarters + done as f64 / spt / PPQ as f64, ..base };
        for k in 0..self.bus_order.len() {
            let id = self.bus_order[k];
            if let Some((g, s)) = decode_sound_bus(id) {
                self.run_sound_bus(g, s, frames, at, glide, &base, spt, out, ext);
                continue;
            }
            let g = id;
            let Some(mut bus) = self.buses.get_mut(g).and_then(Option::take) else { continue };
            for u in &mut bus.chain.units {
                clear_strips(&mut u.extras, frames * 2);
            }
            run_chain(&mut bus.chain, &mut bus.buf[..frames * 2], frames, pos, &self.sc_bufs);
            // Its plug-ins' strips.
            for x in bus.chain.units.iter_mut().flat_map(|u| &mut u.extras) {
                mix_strip(
                    x,
                    frames,
                    glide,
                    self.connected,
                    out,
                    ext,
                    at,
                    self.inp_at,
                    &mut self.buses,
                    &mut self.bufs,
                    &mut self.meters_extra,
                );
            }
            let fader = self.bus_faders.get(g).map_or((1.0, 1.0), |path| path_gain(&self.faders, path));
            bus.ramp.aim(mul(bus.route.1, fader), glide);
            if let Some(dst) = route_dst(bus.route.0, frames, at, self.inp_at, out, ext, &mut self.buses, &mut self.bufs) {
                let cued = self.cue.groups.get(g).copied().unwrap_or(false);
                for f in 0..frames {
                    let gain = bus.ramp.next();
                    let (a, z) = (bus.buf[f * 2] * gain.0, bus.buf[f * 2 + 1] * gain.1);
                    let (a, z) = (if a.is_finite() { a } else { 0.0 }, if z.is_finite() { z } else { 0.0 });
                    dst[f * 2] += a;
                    dst[f * 2 + 1] += z;
                    if cued {
                        self.cue.mix[f * 2] += a;
                        self.cue.mix[f * 2 + 1] += z;
                    }
                }
            }
            if let Some(sends) = self.group_aux.get_mut(g) {
                aux::mix(
                    sends,
                    0,
                    &bus.buf[..frames * 2],
                    out,
                    ext,
                    &mut self.buses,
                    &mut self.bufs,
                    at,
                    self.inp_at,
                    &self.faders,
                    &self.timeline.faders,
                    self.frame as f64 / spt,
                    self.sequencing,
                    glide,
                );
            }
            bus.buf[..frames * 2].fill(0.0);
            self.buses[g] = Some(bus);
        }
        if let Some(mut chain) = self.master_fx.take() {
            for u in &mut chain.units {
                clear_strips(&mut u.extras, frames * 2);
            }
            run_chain(&mut chain, out, frames, pos, &self.sc_bufs);
            // Its plug-ins' strips (one onto a group's bus plays there next span: the
            // buses have run).
            for x in chain.units.iter_mut().flat_map(|u| &mut u.extras) {
                mix_strip(
                    x,
                    frames,
                    glide,
                    self.connected,
                    out,
                    ext,
                    at,
                    self.inp_at,
                    &mut self.buses,
                    &mut self.bufs,
                    &mut self.meters_extra,
                );
            }
            self.master_fx = Some(chain);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_sound_bus(
        &mut self,
        g: usize,
        s: usize,
        frames: usize,
        at: usize,
        glide: u32,
        base: &Transport,
        spt: f64,
        out: &mut [f32],
        ext: &mut [Vec<f32>],
    ) {
        if self.meters.len() != self.sounds.len() {
            self.meters = self.sounds.iter().map(|g| vec![(0.0, 0.0); g.len()]).collect();
        }
        let Some(mut b) = self.bufs.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::take) else { return };
        let smp = self.sounds.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut);
        let inst = self.instruments.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut);
        let chain = self.fx.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut);
        if smp.is_none() && inst.is_none() && !b.inp_on && chain.is_none() {
            self.bufs[g][s] = Some(b);
            return;
        }
        if smp.is_none() && inst.is_none() && chain.is_some() {
            b.inp_on = true;
        }
        render_sound(smp, inst, chain, &mut b, (frames, self.inp_at), self.frame, base, spt, &self.sc_bufs);
        b.inp[self.inp_at * 2..(self.inp_at + frames) * 2].fill(0.0);
        b.inp_on = self.in_routes.iter().any(|r| (r.0, r.1) == (g, s));
        self.bufs[g][s] = Some(b);
        let mut loud = false;
        for kind in [0u8, 1] {
            loud |= self.mix_sound_output(g, s, kind, frames, at, glide, true, out, ext);
        }
        self.mix_sound_strips(g, s, frames, at, glide, out, ext);
        if loud {
            self.quiet = 0;
        }
    }

    fn trigger(&mut self, e: NoteEvent, tps: f64) {
        self.trigger_with(e, tps, true);
    }

    /// Play an event (its probability already decided); `midi_out`: also send it to
    /// the sound's MIDI output.
    fn trigger_with(&mut self, e: NoteEvent, tps: f64, midi_out: bool) {
        // An Audio plug-in's loop starting or stopping (a player ahead has it already).
        if let Some(c) = e.cue {
            if self.link_ahead(e.group, e.sound).is_none()
                && let Some(Some(s)) = self.sounds.get_mut(e.group).and_then(|g| g.get_mut(e.sound))
            {
                s.cue(cue_take(c).map(|t| (t, e.tick as f64 / PPQ as f64)));
            }
            return;
        }
        if note_muted(&self.sound_muted, &e) {
            return;
        }
        // A MIDI input plug-in on the group has its notes, played or from a pattern.
        if e.midi.is_none() && self.midi_input_takes(e.group) {
            let length = (e.length != HELD).then_some(e.length);
            self.midi_input_note(e.group, midi_input::NoteIn::On { sound: e.sound, pitch: e.pitch, velocity: e.velocity, length });
            return;
        }
        let frames = (e.length != HELD).then(|| ((e.length.max(1) as f64) / tps).max(1.0) as u64);
        // A note that sounds (not a choke or a controller) restarts the LFOs it triggers
        // and plays its envelopes; a choke ends their notes.
        if e.midi.is_none() && !self.silent.get(e.group).and_then(|r| r.get(e.sound)).copied().unwrap_or(false) {
            self.lfo_trigger(e.group, e.sound, e.pitch, frames);
        } else if e.midi == Some(pads::CHOKE) {
            self.lfo_release(e.group, e.sound, None);
        }
        if e.midi.is_none() {
            self.touch_start(e.group, e.sound, e.pitch, frames, tps, e.expr);
        }
        if midi_out && let Some(route) = self.midi_routes.get(e.group).and_then(|r| r.get(e.sound)).cloned().flatten() {
            match route {
                midi::OutRoute::Port { port, channel, transpose } => {
                    // An MPE port: channel-wide messages on the manager channel (1).
                    let range = self.mpe.get(&*port).map(|m| m.range);
                    let wide = if range.is_some() { 0 } else { channel };
                    match e.midi {
                        Some(MidiKind::Controller { number, value }) => {
                            self.send_midi(&port, [0xb0 | wide, number & 0x7f, value & 0x7f], 3)
                        }
                        Some(MidiKind::PitchBend(v)) => self.send_midi(&port, [0xe0 | wide, (v & 0x7f) as u8, ((v >> 7) & 0x7f) as u8], 3),
                        Some(MidiKind::ChannelPressure(v)) => self.send_midi(&port, [0xd0 | wide, v & 0x7f, 0], 2),
                        None => {
                            let note = midi::transposed(e.pitch, transpose);
                            let expr = e.expr.and_then(|id| self.expression(id));
                            let release = expr.as_ref().and_then(|x| x.release_velocity).unwrap_or(0);
                            let mut sent = [None; 3];
                            // A live key on an MPE port: its starting values may come just
                            // after it (an MPE controller's); it goes out once they have.
                            let wait = range.is_some() && self.batching && expr.is_none() && e.length == HELD;
                            let channel = match range {
                                Some(_) if wait => {
                                    sent = [Some(8192), Some(64), Some(0)];
                                    self.mpe_channel(&port)
                                }
                                Some(range) => {
                                    // A member channel of its own, set to where the note starts.
                                    let ch = self.mpe_channel(&port);
                                    let start = |d: Dim| expr.as_ref().and_then(|x| x.curve(d)).map(|c| c.value_at(0));
                                    let bend = midi::bend_14(start(Dim::Pitch).unwrap_or(0.0), range);
                                    let slide = start(Dim::Slide).map_or(64, |v| (v.clamp(0.0, 1.0) * 127.0).round() as u16);
                                    let pressure = start(Dim::Pressure).map_or(0, |v| (v.clamp(0.0, 1.0) * 127.0).round() as u16);
                                    self.send_midi(&port, [0xe0 | ch, (bend & 0x7f) as u8, (bend >> 7) as u8], 3);
                                    self.send_midi(&port, [0xb0 | ch, 74, slide as u8], 3);
                                    self.send_midi(&port, [0xd0 | ch, pressure as u8, 0], 2);
                                    sent = [Some(bend), Some(slide), Some(pressure)];
                                    ch
                                }
                                None => {
                                    // A note still sounding on that key ends first.
                                    if let Some(i) =
                                        self.midi_offs.iter().position(|o| *o.port == *port && o.channel == channel && o.note == note)
                                    {
                                        let n = self.midi_offs.remove(i);
                                        self.end_midi_note(n);
                                    }
                                    channel
                                }
                            };
                            let velocity = e.velocity.clamp(1, 127) as u8;
                            if !wait {
                                self.send_midi(&port, [0x90 | channel, note, velocity], 3);
                            }
                            let at = if e.length == HELD { u64::MAX } else { self.time + ((e.length.max(1) as f64) / tps).max(1.0) as u64 };
                            self.midi_offs.push(MidiNote {
                                at,
                                port,
                                channel,
                                note,
                                held: (e.group, e.sound, e.pitch),
                                release,
                                mpe: range.is_some(),
                                waiting: wait.then_some(velocity),
                                expr,
                                started: self.time,
                                tps,
                                sent,
                                sent_at: [self.time; 3],
                            });
                        }
                    }
                }
                // To another sound's instrument (its own MIDI output aside).
                midi::OutRoute::Sound(t) => self.trigger_with(NoteEvent { sound: t, ..e }, tps, false),
            }
        }
        // A sound rendered ahead: its player has the timeline's events already (sent
        // ahead); live ones go to it now.
        if self.link_ahead(e.group, e.sound).is_some() {
            if !self.from_timeline {
                let ev = match e.midi {
                    Some(kind) => ahead::Ev::Control(kind),
                    None => {
                        let id = self.ahead.as_mut().map_or(0, |a| {
                            a.next_id += 1;
                            a.next_id
                        });
                        let gate = (e.length != HELD).then(|| ((e.length.max(1) as f64) / tps).max(1.0) as u64);
                        ahead::Ev::NoteOn { id, pitch: e.pitch, velocity: e.velocity, gate, beats: self.position() / PPQ as f64 }
                    }
                };
                self.send_live(e.group, e.sound, ev);
            }
            return;
        }
        if let Some(kind) = e.midi {
            if let Some(Some(s)) = self.sounds.get_mut(e.group).and_then(|g| g.get_mut(e.sound)) {
                s.control(kind);
            }
            if let Some(Some(inst)) = self.instruments.get(e.group).and_then(|g| g.get(e.sound))
                && let Ok(mut p) = inst.plugin.lock()
            {
                match kind {
                    MidiKind::Controller { number, value } => p.control_change(number, value),
                    MidiKind::PitchBend(v) => p.pitch_bend(v),
                    MidiKind::ChannelPressure(_) => {}
                }
            }
            return;
        }
        if let Some(Some(inst)) = self.instruments.get(e.group).and_then(|g| g.get(e.sound)) {
            if inst.muted {
                return;
            }
            // Other threads hold an instance only for moments (a parameter, a note), so
            // the audio thread waits for it rather than drop the note.
            if let Ok(mut p) = inst.plugin.lock() {
                p.note_on(e.pitch, e.velocity);
                self.note_offs.retain(|o| !(o.1 == e.group && o.2 == e.sound && o.3 == e.pitch));
                if e.length != HELD {
                    let gate = ((e.length.max(1) as f64) / tps).max(1.0) as u64;
                    self.note_offs.push((self.time + gate, e.group, e.sound, e.pitch, 0));
                }
            }
            return;
        }
        let ctx = sampler::Context { tempo: self.tempo, beats: self.position() / PPQ as f64 };
        let Some(Some(snd)) = self.sounds.get_mut(e.group).and_then(|g| g.get_mut(e.sound)) else { return };
        self.clock += 1;
        let id = self.clock;
        snd.note_on(id, e.pitch, e.velocity.clamp(0, 127), &ctx);
        if e.length != HELD {
            let gate = ((e.length.max(1) as f64) / tps).max(1.0) as u64;
            self.note_offs.push((self.time + gate, e.group, e.sound, e.pitch, id));
        }
    }

    /// Peak levels (left, right; linear) of each sound, `[group][sound]`, since the last
    /// call, for level meters. Sounds that played nothing read 0.
    pub fn take_meters(&mut self) -> Vec<Vec<(f32, f32)>> {
        let shape: Vec<usize> = self.sounds.iter().map(Vec::len).collect();
        let mut out: Vec<Vec<(f32, f32)>> = shape.iter().map(|n| vec![(0.0, 0.0); *n]).collect();
        for (g, row) in std::mem::take(&mut self.meters).into_iter().enumerate() {
            for (s, v) in row.into_iter().enumerate() {
                if let Some(slot) = out.get_mut(g).and_then(|r| r.get_mut(s)) {
                    *slot = v;
                }
            }
        }
        out
    }

    /// Extra pairs heard since the last look, merged into `t.heard` (each once, in mixer
    /// order). Room is made only when the UI took `t.heard`'s buffer away.
    fn publish_heard(&mut self, t: &mut Telemetry) {
        self.heard_len = 0;
        let (heard, len) = (&mut self.heard, &mut self.heard_len);
        self.heard_board.take(|owner, bus| {
            if *len < HEARD_ROOM {
                heard[*len] = Heard { owner, bus };
                *len += 1;
            }
        });
        if self.heard_len == 0 {
            return;
        }
        // (Only for a `t` not made with [`Telemetry::with_room`], once.)
        if t.heard.capacity() < HEARD_ROOM {
            t.heard.reserve_exact(HEARD_ROOM - t.heard.len());
        }
        for h in &self.heard[..self.heard_len] {
            if let Err(i) = t.heard.binary_search_by_key(&heard_rank(h), heard_rank)
                && t.heard.len() < HEARD_ROOM
            {
                t.heard.insert(i, *h);
            }
        }
    }

    /// Hand what the UI shows to `t` (from the audio thread, after a block): peaks
    /// since the UI last took them (merged in, and reset here without allocating),
    /// where playback is, and the zones `t.watch` asks about.
    pub fn publish(&mut self, t: &mut Telemetry) {
        // Allocation here happens only when the project's shape or its list of strips
        // changed (rare), or the first time for `t`s not made with [`Telemetry::with_room`].
        if t.meters.len() != self.meters.len() || t.meters.iter().zip(&self.meters).any(|(a, b)| a.len() != b.len()) {
            t.meters = self.meters.iter().map(|r| vec![(0.0, 0.0); r.len()]).collect();
        }
        for (trow, row) in t.meters.iter_mut().zip(self.meters.iter_mut()) {
            for (tm, m) in trow.iter_mut().zip(row.iter_mut()) {
                *tm = (tm.0.max(m.0), tm.1.max(m.1));
                *m = (0.0, 0.0);
            }
        }
        if t.extra_meters.len() != self.meters_extra.len() {
            // (Strips came or went: what was there is another strip's now.)
            t.extra_meters.clear();
            t.extra_meters.resize(self.meters_extra.len(), (0.0, 0.0));
        }
        for (tm, m) in t.extra_meters.iter_mut().zip(self.meters_extra.iter_mut()) {
            *tm = (tm.0.max(m.0), tm.1.max(m.1));
            *m = (0.0, 0.0);
        }
        let mp = std::mem::take(&mut self.master_peak);
        t.master_peak = (t.master_peak.0.max(mp.0), t.master_peak.1.max(mp.1));
        let cp = std::mem::take(&mut self.cue.peak);
        t.cue_peak = (t.cue_peak.0.max(cp.0), t.cue_peak.1.max(cp.1));
        t.position = self.position();
        t.played = self.played();
        // Recorded notes, handed over without allocating here.
        let room = RECORDED_ROOM.saturating_sub(t.recorded.len());
        t.recorded.extend(self.recorded.drain(..).take(room));
        let room = RECORDED_ROOM.saturating_sub(t.recorded_expr.len());
        t.recorded_expr.extend(self.recorded_expr.drain(..).take(room));
        let room = RECORDED_ROOM.saturating_sub(t.recorded_controls.len());
        t.recorded_controls.extend(self.recorded_controls.drain(..).take(room));
        t.length = self.timeline.length as f64;
        t.sequencing = self.sequencing;
        t.mode = Some(self.mode);
        t.queued = self.queued();
        t.finished = self.finished();
        t.active_voices = self.active_voices();
        t.pending_jump = self.jump;
        t.tap_frames = self.tap_total;
        t.live_hit = self.live_hit;
        t.tempo = self.tempo;
        t.stale = self.stale;
        t.sounds_ahead = self.sounds_ahead();
        t.ahead_underruns = self.ahead_underruns();
        self.publish_heard(t);
        t.zones.resize(t.watch.len(), None);
        for (pos, &(g, s, z)) in t.zones.iter_mut().zip(&t.watch) {
            *pos = self.zone_position(g, s, z);
        }
    }

    /// Where zone `zone` of sound `sound` in group `group` is playing (sample frames).
    pub fn zone_position(&self, group: usize, sound: usize, zone: usize) -> Option<usize> {
        self.sounds.get(group)?.get(sound)?.as_ref()?.play_position(zone)
    }

    /// The master output's peak (left, right) since the last call.
    pub fn take_master_peak(&mut self) -> (f32, f32) {
        std::mem::take(&mut self.master_peak)
    }

    /// Start (or with `None` stop) copying audio from inside the engine to a tap.
    pub fn set_tap(&mut self, tap: Option<Tap>) {
        self.tap = tap;
        self.tap_pending.clear();
        self.tap_total = 0;
    }

    /// Frames the tap has produced since it was set: the frame of the tap's audio at
    /// which something triggered now (a note) is heard.
    pub fn tap_frames(&self) -> u64 {
        self.tap_total
    }

    /// Take over a tap from another engine that has produced `total` frames for it.
    pub fn continue_tap(&mut self, tap: Tap, total: u64) {
        self.set_tap(Some(tap));
        self.tap_total = total;
    }

    pub fn tap(&self) -> Option<&Tap> {
        self.tap.as_ref()
    }

    /// Hand the tapped audio of this block to the recorder (kept if its buffer is busy;
    /// the audio thread never waits).
    fn flush_tap(&mut self, block: &[f32]) {
        let Some(tap) = &self.tap else { return };
        self.tap_total += (block.len() / 2) as u64;
        self.tap_pending.extend_from_slice(block);
        if let Ok(mut b) = tap.buffer.try_lock() {
            b.append(&mut self.tap_pending);
        } else if self.tap_pending.len() > 48_000 * 2 * 10 {
            // The recorder stopped taking audio: don't grow without end.
            self.tap_pending.clear();
        }
    }

    /// Send the note-offs that are due.
    fn fire_note_offs(&mut self) {
        let now = self.time;
        if self.midi_offs.iter().any(|o| o.at <= now) {
            let mut i = 0;
            while i < self.midi_offs.len() {
                if self.midi_offs[i].at <= now {
                    let n = self.midi_offs.remove(i);
                    self.end_midi_note(n);
                } else {
                    i += 1;
                }
            }
        }
        let instruments = &self.instruments;
        let sounds = &mut self.sounds;
        self.note_offs.retain(|&(at, g, s, pitch, id)| {
            if at > now {
                return true;
            }
            if let Some(Some(smp)) = sounds.get_mut(g).and_then(|r| r.get_mut(s)) {
                smp.note_off(id, pitch);
                return false;
            }
            match instruments.get(g).and_then(|r| r.get(s)).and_then(Option::as_ref) {
                Some(i) => {
                    if let Ok(mut p) = i.plugin.lock() {
                        p.note_off(pitch);
                    }
                    false
                }
                None => false,
            }
        });
    }

    /// Render the sounds of side-chain pass `pass` (0: those whose effects don't listen
    /// to a side-chain, and retired voices; 1: those that do) for `out`'s frames and mix
    /// them in.
    ///
    /// Each sound renders into its own buffers ([`render_sound`]), in parallel on the
    /// render pool when there is one and it's worth it. Feeding side-chains, mixing,
    /// taps and meters then happen here, in a fixed order, so the result is the same
    /// however many threads helped.
    /// `ext` from `at` (samples): the other external outputs' buffers for this span.
    fn render_pass(&mut self, out: &mut [f32], ext: &mut [Vec<f32>], at: usize, pass: u8) {
        let frames = out.len() / 2;
        if self.meters.len() != self.sounds.len() {
            self.meters = self.sounds.iter().map(|g| vec![(0.0, 0.0); g.len()]).collect();
        }
        let spt = self.samples_per_tick();
        let frame = self.frame;
        let base = Transport {
            tempo: self.tempo,
            playing: self.sequencing,
            quarters: self.frame as f64 / spt / PPQ as f64,
            numerator: self.numerator,
            denominator: self.denominator,
        };
        // This pass's sounds, and where their parts are.
        self.job_list.clear();
        self.job_ptrs.clear();
        for g in 0..self.bufs.len() {
            for s in 0..self.bufs[g].len() {
                if self.bus_order.contains(&sound_bus_id(g, s)) {
                    continue;
                }
                // Rendered ahead: mixed with the others (they never listen to a
                // side-chain, so in pass 0).
                if self.link_ahead(g, s).is_some() {
                    if pass == 0 && self.bufs[g][s].is_some() {
                        self.job_list.push((g, s));
                    }
                    continue;
                }
                let Some(bufs) = self.bufs[g][s].as_deref_mut() else { continue };
                let smp = self.sounds.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut);
                let inst = self.instruments.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut);
                if smp.is_none() && inst.is_none() && !bufs.inp_on {
                    continue;
                }
                let chain = self.fx.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut);
                if chain.as_ref().is_some_and(|c| c.listens()) != (pass == 1) {
                    continue;
                }
                self.job_list.push((g, s));
                self.job_ptrs.push(JobPtrs {
                    smp: smp.map_or(std::ptr::null_mut(), |x| x as *mut SamplerRt),
                    inst: inst.map_or(std::ptr::null_mut(), |x| x as *mut InstrumentRt),
                    chain: chain.map_or(std::ptr::null_mut(), |x| x as *mut FxChain),
                    bufs: bufs as *mut SoundBufs,
                });
            }
        }
        // A listening sound that also feeds a side-chain: the sounds after it may hear
        // it, so this pass runs one sound at a time, each feeding its side-chains
        // before the next.
        let chained = pass == 1 && self.job_ptrs.iter().any(|j| unsafe { j.chain.as_ref() }.is_some_and(|c| !c.tap.is_empty()));
        // Each job has its own sound's parts (distinct sounds, see above).
        let inp_at = self.inp_at;
        let run = |j: &JobPtrs, sc: &[Vec<f32>]| unsafe {
            render_sound(j.smp.as_mut(), j.inst.as_mut(), j.chain.as_mut(), &mut *j.bufs, (frames, inp_at), frame, &base, spt, sc)
        };
        if chained {
            for i in 0..self.job_ptrs.len() {
                let j = self.job_ptrs[i];
                run(&j, &self.sc_bufs);
                let b = unsafe { &*j.bufs };
                if let Some(c) = unsafe { j.chain.as_ref() } {
                    let src = if b.smp_out { &b.smp } else { &b.inst };
                    c.record_from(&src[..frames * 2], &mut self.sc_bufs);
                }
            }
        } else {
            let (ptrs, sc) = (&self.job_ptrs, &self.sc_bufs);
            let task = |i: usize| run(&ptrs[i], sc);
            match &self.pool {
                Some(pool) if ptrs.len() > 1 && frames >= MIN_PARALLEL_FRAMES => pool.run(ptrs.len(), &task),
                _ => (0..ptrs.len()).for_each(task),
            }
        }
        // What players rendered ahead.
        if self.ahead.is_some() && pass == 0 {
            for i in 0..self.job_list.len() {
                let (g, s) = self.job_list[i];
                if self.link_ahead(g, s).is_some() {
                    self.ahead_fill(g, s, frames);
                }
            }
        }
        // Mixing, in order: Sampler sounds, retired voices, then plug-in instruments.
        let glide = (GAIN_GLIDE_SECONDS * self.sr).round() as u32;
        let mut loud = false;
        let jobs = std::mem::take(&mut self.job_list);
        for kind in [0u8, 1] {
            if kind == 1 && pass == 0 {
                for smp in &mut self.retired {
                    smp.render(out);
                }
                self.retired.retain(|s| s.voices() > 0);
            }
            for &(g, s) in &jobs {
                loud |= self.mix_sound_output(g, s, kind, frames, at, glide, !chained, out, ext);
            }
        }
        // Their plug-ins' strips, each onto where it goes (sounds with strips play here,
        // not ahead: see `live_hint`).
        for &(g, s) in &jobs {
            self.mix_sound_strips(g, s, frames, at, glide, out, ext);
        }
        self.job_list = jobs;
        // Counted once per span (pass 0); the listening sounds (pass 1) can only make it
        // loud.
        if loud {
            self.quiet = 0;
        } else if pass == 0 {
            self.quiet = self.quiet.saturating_add(frames as u64);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mix_sound_output(
        &mut self,
        g: usize,
        s: usize,
        kind: u8,
        frames: usize,
        at: usize,
        glide: u32,
        record_sc: bool,
        out: &mut [f32],
        ext: &mut [Vec<f32>],
    ) -> bool {
        let Some(mut b) = self.bufs.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::take) else { return false };
        let rendered = if kind == 0 { b.smp_out } else { b.inst_out };
        if !rendered {
            self.bufs[g][s] = Some(b);
            return false;
        }
        let src = if kind == 0 { &b.smp[..frames * 2] } else { &b.inst[..frames * 2] };
        let chain = self.fx.get(g).and_then(|r| r.get(s)).and_then(Option::as_ref);
        let link = self.ahead.as_ref().and_then(|a| a.links.get(g)?.get(s)?.as_ref()).filter(|l| l.state != LinkState::Live);
        let gain = match (link, chain, kind) {
            (Some(l), ..) => l.gains[kind as usize],
            (None, Some(c), _) => c.gain,
            (None, None, 0) => b.smp_gain,
            (None, None, _) => self.instruments[g][s].as_ref().map_or((0.0, 0.0), |i| i.gain),
        };
        let checked = chain.is_some() || link.is_some_and(|l| l.finite);
        let fader = self.sound_faders.get(g).and_then(|r| r.get(s)).map_or((1.0, 1.0), |path| path_gain(&self.faders, path));
        let cued = self.cue.sounds.get(g).and_then(|r| r.get(s)).copied().unwrap_or(false);
        if record_sc && let Some(c) = chain {
            c.record_from(src, &mut self.sc_bufs);
        }
        let tapped = taps(self.tap.as_ref(), g, s);
        let spt = self.samples_per_tick();
        if let Some(sends) = self.sound_aux.get_mut(g).and_then(|row| row.get_mut(s)) {
            aux::mix(
                sends,
                kind as usize,
                src,
                out,
                ext,
                &mut self.buses,
                &mut self.bufs,
                at,
                self.inp_at,
                &self.faders,
                &self.timeline.faders,
                self.frame as f64 / spt,
                self.sequencing,
                glide,
            );
        }
        let ramp = &mut b.ramps[kind as usize];
        ramp.aim(mul(gain, fader), glide);
        // Onto the master's output, or another (none: dropped).
        let bus = self.routes.get(g).and_then(|r| r.get(s)).copied().flatten();
        let mut loud = false;
        if let Some(dst) = route_dst(bus, frames, at, self.inp_at, out, ext, &mut self.buses, &mut self.bufs) {
            let src = if kind == 0 { &b.smp[..frames * 2] } else { &b.inst[..frames * 2] };
            let ramp = &mut b.ramps[kind as usize];
            let mut peak = (0f32, 0f32);
            for f in 0..frames {
                let gain = ramp.next();
                let (mut a, mut z) = (src[f * 2] * gain.0, src[f * 2 + 1] * gain.1);
                if checked {
                    (a, z) = (if a.is_finite() { a } else { 0.0 }, if z.is_finite() { z } else { 0.0 });
                }
                dst[f * 2] += a;
                dst[f * 2 + 1] += z;
                if cued {
                    self.cue.mix[f * 2] += a;
                    self.cue.mix[f * 2 + 1] += z;
                }
                if tapped && let Some(t) = self.tap_block.get_mut(f * 2..f * 2 + 2) {
                    t[0] += a;
                    t[1] += z;
                }
                peak = (peak.0.max(a.abs()), peak.1.max(z.abs()));
            }
            if kind == 1 && peak.0.max(peak.1) > 1e-4 {
                loud = true;
            }
            if let Some(m) = self.meters.get_mut(g).and_then(|r| r.get_mut(s)) {
                m.0 = m.0.max(peak.0);
                m.1 = m.1.max(peak.1);
            }
        }
        self.bufs[g][s] = Some(b);
        loud
    }

    fn mix_sound_strips(&mut self, g: usize, s: usize, frames: usize, at: usize, glide: u32, out: &mut [f32], ext: &mut [Vec<f32>]) {
        if self.link_ahead(g, s).is_some() {
            return;
        }
        let connected = self.connected;
        let inp_at = self.inp_at;
        let (buses, bufs, meters) = (&mut self.buses, &mut self.bufs, &mut self.meters_extra);
        let inst = self.instruments.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut).map(|i| &mut i.extras);
        let chain = self.fx.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut).map(|c| &mut c.units);
        let units = chain.into_iter().flatten().map(|u| &mut u.extras);
        for x in inst.into_iter().chain(units).flatten() {
            mix_strip(x, frames, glide, connected, out, ext, at, inp_at, buses, bufs, meters);
        }
    }
}

/// Frames a span needs before its sounds are worth spreading over threads.
const MIN_PARALLEL_FRAMES: usize = 32;

/// The longest span rendered at once (longer blocks are split): the size of each
/// sound's buffers.
const JOB_FRAMES: usize = 2048;

/// The sounds a side-chain source stands for: (group, sound, after its fader). As
/// Maschine taps them (docs/fx-*.md, null-tested): a sound before its fader (whatever
/// its output); a group as the sum of the sounds going into it, each after its fader.
fn sidechain_sources(project: &Project, src: deckard_model::ScSource) -> Vec<(usize, usize, bool)> {
    use deckard_model::AudioDest as D;
    match src {
        deckard_model::ScSource::Sound(g, s) => vec![(g, s, false)],
        deckard_model::ScSource::Group(g) => project
            .groups
            .iter()
            .enumerate()
            .flat_map(|(gi, grp)| grp.sounds.iter().enumerate().map(move |(si, snd)| (gi, si, snd)))
            .filter(|(gi, _, snd)| snd.channel.dest == D::Group(g) || (*gi == g && snd.channel.dest == D::Parent))
            .map(|(gi, si, _)| (gi, si, true))
            .collect(),
    }
}

/// Which sounds feed side-chains, each with its buffer: keys (group, sound, after the
/// fader).
fn sidechain_index(project: &Project) -> HashMap<(usize, usize, bool), usize> {
    let mut sc_index = HashMap::new();
    let mut add = |src| {
        for k in sidechain_sources(project, src) {
            let n = sc_index.len();
            sc_index.entry(k).or_insert(n);
        }
    };
    for (gi, g) in project.groups.iter().enumerate() {
        for &(slot, src) in &g.channel.sidechain {
            if src != deckard_model::ScSource::Group(gi) && g.slots.get(slot).is_some_and(|x| x.active && !x.is_instrument()) {
                add(src);
            }
        }
        for s in &g.sounds {
            for &(slot, src) in &s.channel.sidechain {
                if s.slots.get(slot).is_some_and(|x| x.active && !x.is_instrument()) {
                    add(src);
                }
            }
        }
    }
    sc_index
}

/// For an edit (see [`Engine::new_after`]), a sound whose parts the playing engine has
/// already: which to keep as they are, small changes to make to them, and the gains it
/// plays at now.
#[derive(Default)]
struct Reuse {
    sampler: bool,
    /// New settings for the kept Sampler (its zones are the same).
    settings: Option<Box<sampler::Settings>>,
    instrument: bool,
    chain: bool,
    /// New settings of effects in the kept chain: (slot, parameters).
    params: Vec<(usize, Vec<(String, f64)>)>,
    chain_gain: (f32, f32),
    inst_gain: (f32, f32),
    inst_muted: bool,
    smp_gain: (f32, f32),
}

/// Whether `a` and `b` make the same timelines (the same patterns, scenes, sections,
/// grooves, bar length and modulated parameters' base values): an edit elsewhere keeps
/// the playing timeline.
fn same_timelines(a: &Project, b: &Project) -> bool {
    let same_slot = |sa: Option<&deckard_model::SlotInfo>, sb: Option<&deckard_model::SlotInfo>| match (sa, sb) {
        (Some(x), Some(y)) if x.plugin.is_none() && y.plugin.is_none() => x.module == y.module && x.params == y.params,
        (x, y) => x.map(|s| &s.plugin) == y.map(|s| &s.plugin),
    };
    a.scenes == b.scenes
        && a.sections == b.sections
        && a.swing == b.swing
        && (a.numerator, a.denominator) == (b.numerator, b.denominator)
        && a.groups.len() == b.groups.len()
        && a.groups.iter().zip(&b.groups).all(|(x, y)| {
            x.patterns == y.patterns
                && x.swing == y.swing
                && x.sounds.len() == y.sounds.len()
                && x.sounds.iter().zip(&y.sounds).all(|(m, n)| m.swing == n.swing)
                && x.patterns.iter().flat_map(|p| &p.mods).all(|m| match (m.sound, m.slot) {
                    (Some(s), Some(i)) => {
                        let (ma, mb) = (x.sounds.get(s), y.sounds.get(s));
                        same_slot(ma.and_then(|snd| snd.slots.get(i)), mb.and_then(|snd| snd.slots.get(i)))
                            && (i != 0
                                || ma.and_then(|snd| snd.sampler.as_ref()).map(|smp| &smp.params)
                                    == mb.and_then(|snd| snd.sampler.as_ref()).map(|smp| &smp.params))
                    }
                    (None, Some(i)) => same_slot(x.slots.get(i), y.slots.get(i)),
                    _ => true,
                })
        })
}

/// The slots of a sound's chain as its effect chain is built from them, without their
/// effects' settings: what makes two chains the same but for those.
fn chain_shape(s: &deckard_model::Sound) -> Vec<(usize, &str, bool, Option<(&str, &str)>)> {
    effect_slots(s)
        .into_iter()
        .map(|(i, x)| (i, x.module.as_str(), x.active, x.plugin.as_ref().map(|p| (p.id.as_str(), p.name.as_str()))))
        .collect()
}

/// Whether `slot`'s plug-in can be kept as it plays when `old` becomes `new`: either
/// the saved chunk is unchanged, or the live instance already holds `new`'s chunk (a
/// knob turn saved back into the document).
fn plugin_kept(
    plugins: &deckard_plugins::Host,
    slot: Slot,
    old: &Option<deckard_model::PluginRef>,
    new: &Option<deckard_model::PluginRef>,
) -> bool {
    if old == new {
        return true;
    }
    let (Some(a), Some(b)) = (old, new) else { return false };
    a.id == b.id
        && a.name == b.name
        && plugins.existing(slot).is_some_and(|s| s.lock().ok().is_some_and(|p| p.chunk() == b.chunk.as_slice()))
}

/// The synth a sound plays (its first slot's module, when on and one of
/// [`synth::CLASSES`]), and its parameters.
fn synth_of(s: &deckard_model::Sound) -> Option<(&str, &[(String, f64)])> {
    let slot = s.slots.first().filter(|x| x.active && x.plugin.is_none())?;
    synth::CLASSES.contains(&slot.module.as_str()).then_some((slot.module.as_str(), slot.params.as_slice()))
}

/// What a sound being edited can keep of what plays, comparing it as it was (`old`,
/// in `prev`) with what it is (`new`).
fn reuse_of(prev: &Project, project: &Project, g: usize, s: usize, sr: f64, plugins: &deckard_plugins::Host) -> Reuse {
    let (og, ng) = (&prev.groups[g], &project.groups[g]);
    let (old, new) = (&og.sounds[s], &ng.sounds[s]);
    let same_silence = muting(prev, g, s).silent == muting(project, g, s).silent;
    let mut r = Reuse::default();
    // The Sampler: the same, or the same zones (new settings).
    if same_silence {
        match (&old.sampler, &new.sampler, synth_of(old), synth_of(new)) {
            (_, _, Some((a, pa)), Some((b, pb))) if a == b => {
                r.sampler = true;
                if pa != pb {
                    r.settings = Some(Box::new(sampler::Settings::for_synth(pb, sr)));
                }
            }
            // (An Audio plug-in only when it's the same: its player is made anew, and
            // adopts the old one's place.)
            (a, b, None, None) if a == b && old.audio == new.audio && old.instrument == new.instrument => r.sampler = true,
            (Some(a), Some(b), None, None) if a.zones == b.zones => {
                r.sampler = true;
                r.settings = Some(Box::new(sampler::Settings::new(b, sr)));
            }
            _ => {}
        }
    }
    // Its plug-ins' strips (which pairs have one, and what each takes from its pair; not
    // their settings): a pair given a strip or losing one, or one split or joined, makes
    // the part anew, to listen to the pairs left and take each strip's side.
    fn strips(p: &Project, g: usize, s: usize, chain: bool) -> Vec<(Owner, usize, &str, Channels)> {
        let mut v: Vec<(Owner, usize, &str, Channels)> = p
            .extra_outs
            .iter()
            .filter(|x| match x.owner {
                Owner::Sound(a, b) => !chain && (a, b) == (g, s),
                Owner::Chain(a, b, _) => chain && (a, b) == (g, s),
                _ => false,
            })
            .map(|x| (x.owner, x.bus, x.plugin.as_str(), x.channels))
            .collect();
        v.sort_by_key(|&(o, bus, id, _)| (heard_rank(&Heard { owner: o, bus }), id));
        v
    }
    r.instrument = plugin_kept(plugins, Slot::Sound { group: g, sound: s }, &old.plugin, &new.plugin)
        && strips(prev, g, s, false) == strips(project, g, s, false);
    // The chain: the same slots and side-chains (its effects' settings may differ).
    let same_fx_plugins = effect_slots(old)
        .into_iter()
        .zip(effect_slots(new))
        .all(|((slot, a), (_, b))| plugin_kept(plugins, Slot::Chain { group: g, sound: s, slot }, &a.plugin, &b.plugin));
    if chain_shape(old) == chain_shape(new)
        && same_fx_plugins
        && old.channel.sidechain == new.channel.sidechain
        && strips(prev, g, s, true) == strips(project, g, s, true)
    {
        r.chain = true;
        for ((i, a), (_, b)) in effect_slots(old).into_iter().zip(effect_slots(new)) {
            if a.params != b.params {
                r.params.push((i, b.params.clone()));
            }
        }
    }
    r
}

/// One sound's output for a span, as [`render_sound`] leaves it: its Sampler's (after
/// its effects, before its level and pan; or, without effects, mixed) and its plug-in
/// instrument's (after its effects, before its level and pan), interleaved stereo.
struct SoundBufs {
    smp: Vec<f32>,
    inst: Vec<f32>,
    smp_out: bool,
    inst_out: bool,
    /// The level and pan of the Sampler's output when it has no effects (with
    /// effects, the chain's gain applies).
    smp_gain: (f32, f32),
    /// The gains applied to the Sampler's and the plug-in's outputs, gliding to new
    /// settings.
    ramps: [GainRamp; 2],
    /// The interface input the sound takes, for the whole span (`inp_on`: this span),
    /// played with its Sampler: through its effects, then its fader and route.
    inp: Vec<f32>,
    inp_on: bool,
}

impl SoundBufs {
    fn new(smp_gain: (f32, f32)) -> SoundBufs {
        SoundBufs {
            smp: vec![0.0; JOB_FRAMES * 2],
            inst: vec![0.0; JOB_FRAMES * 2],
            smp_out: false,
            inst_out: false,
            smp_gain,
            ramps: [GainRamp::default(); 2],
            inp: vec![0.0; JOB_FRAMES * 2],
            inp_on: false,
        }
    }
}

/// How long a sound's level, pan or mute takes to reach a new setting while playing
/// (rather than jumping, which clicks).
const GAIN_GLIDE_SECONDS: f64 = 0.01;

/// A stereo gain that glides to a new value in a straight line; once there it is the
/// value exactly (so steady playback is unchanged by it).
#[derive(Debug, Clone, Copy, Default)]
struct GainRamp {
    now: (f32, f32),
    target: (f32, f32),
    step: (f32, f32),
    left: u32,
    /// It has had a gain (the first one is taken at once).
    set: bool,
}

impl GainRamp {
    /// Aim at `target`, getting there over `frames` (at once the first time).
    fn aim(&mut self, target: (f32, f32), frames: u32) {
        if !self.set || frames == 0 {
            *self = GainRamp { now: target, target, step: (0.0, 0.0), left: 0, set: true };
        } else if target != self.target {
            self.target = target;
            self.left = frames;
            self.step = ((target.0 - self.now.0) / frames as f32, (target.1 - self.now.1) / frames as f32);
        }
    }

    fn next(&mut self) -> (f32, f32) {
        if self.left > 0 {
            self.now.0 += self.step.0;
            self.now.1 += self.step.1;
            self.left -= 1;
            if self.left == 0 {
                self.now = self.target;
            }
        }
        self.now
    }
}

/// Where one job's sound keeps its parts (each job touches only its own sound's).
#[derive(Clone, Copy)]
struct JobPtrs {
    smp: *mut SamplerRt,
    inst: *mut InstrumentRt,
    chain: *mut FxChain,
    bufs: *mut SoundBufs,
}

// A job's pointers are used by one thread at a time, while the engine waits.
unsafe impl Send for JobPtrs {}
unsafe impl Sync for JobPtrs {}

/// Render one sound for `frames` frames into `b`: its Sampler voices, then its plug-in
/// instrument, each through the sound's effects.
#[allow(clippy::too_many_arguments)]
fn render_sound(
    smp: Option<&mut SamplerRt>,
    mut inst: Option<&mut InstrumentRt>,
    mut chain: Option<&mut FxChain>,
    b: &mut SoundBufs,
    (frames, inp_at): (usize, usize),
    frame: u64,
    base: &Transport,
    spt: f64,
    sc: &[Vec<f32>],
) {
    let n2 = frames * 2;
    // Song positions within the span (computed as the engine always has: plug-ins from
    // the frame count, the Sampler's effects from the span's start).
    let at = |done: usize| Transport { quarters: base.quarters + done as f64 / spt / PPQ as f64, ..*base };
    let at_frame = |done: usize| Transport { quarters: (frame as f64 + done as f64) / spt / PPQ as f64, ..*base };
    b.smp_out = false;
    b.inst_out = false;
    // Its plug-ins' strips take what they play over the span (the chain may run twice).
    if let Some(i) = inst.as_deref_mut() {
        clear_strips(&mut i.extras, n2);
    }
    if let Some(c) = chain.as_deref_mut() {
        for u in &mut c.units {
            clear_strips(&mut u.extras, n2);
        }
    }
    if smp.is_some() || (b.inp_on && inst.is_none()) {
        let buf = &mut b.smp[..n2];
        buf.fill(0.0);
        let mut sounding = false;
        if let Some(smp) = smp {
            // The Audio plug-in follows the song.
            smp.set_song(base.quarters, base.tempo);
            if smp.runs() {
                smp.render(buf);
                sounding = true;
            }
        }
        // The interface or bus input it takes, with it.
        if b.inp_on {
            for (o, i) in buf.iter_mut().zip(&b.inp[inp_at * 2..]) {
                *o += *i;
            }
            sounding = true;
        }
        match chain.as_deref_mut() {
            // With effects: the dry sound through them (always, for the effects' tails).
            Some(c) => {
                let mut done = 0;
                while done < frames {
                    let n = (frames - done).min(MAX_BLOCK);
                    for f in 0..n {
                        c.l[f] = buf[(done + f) * 2];
                        c.r[f] = buf[(done + f) * 2 + 1];
                    }
                    c.run(n, &at(done), sc, done);
                    for f in 0..n {
                        buf[(done + f) * 2] = c.l[f];
                        buf[(done + f) * 2 + 1] = c.r[f];
                    }
                    done += n;
                }
                b.smp_out = true;
            }
            None => b.smp_out = sounding,
        }
    }
    if let Some(inst) = inst {
        let buf = &mut b.inst[..n2];
        buf.fill(0.0);
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(MAX_BLOCK);
            let t = at_frame(done);
            let started = std::time::Instant::now();
            let Ok(mut p) = inst.plugin.lock() else {
                skipped();
                done += n;
                continue;
            };
            let (l, r) = p.process(n, &t);
            timed(&inst.plugin, started);
            let m = l.len().min(r.len()).min(n);
            match chain.as_deref_mut() {
                Some(c) => {
                    c.l[..m].copy_from_slice(&l[..m]);
                    c.r[..m].copy_from_slice(&r[..m]);
                    c.l[m..n].fill(0.0);
                    c.r[m..n].fill(0.0);
                    if b.inp_on {
                        for f in 0..n {
                            c.l[f] += b.inp[(inp_at + done + f) * 2];
                            c.r[f] += b.inp[(inp_at + done + f) * 2 + 1];
                        }
                    }
                    take_strips(&p, &mut inst.extras, n, done, (1.0, 0.0, 0, 1.0));
                    if let (Some(owner), Some(board)) = (inst.owner, &inst.heard) {
                        hear(&p, owner, inst.scan, n, board);
                    }
                    drop(p);
                    c.run(n, &t, sc, done);
                    for f in 0..n {
                        buf[(done + f) * 2] = c.l[f];
                        buf[(done + f) * 2 + 1] = c.r[f];
                    }
                }
                None => {
                    for f in 0..m {
                        buf[(done + f) * 2] = l[f];
                        buf[(done + f) * 2 + 1] = r[f];
                    }
                    if b.inp_on {
                        for f in 0..n {
                            buf[(done + f) * 2] += b.inp[(inp_at + done + f) * 2];
                            buf[(done + f) * 2 + 1] += b.inp[(inp_at + done + f) * 2 + 1];
                        }
                    }
                    take_strips(&p, &mut inst.extras, n, done, (1.0, 0.0, 0, 1.0));
                    if let (Some(owner), Some(board)) = (inst.owner, &inst.heard) {
                        hear(&p, owner, inst.scan, n, board);
                    }
                }
            }
            done += n;
        }
        b.inst_out = true;
    }
}

/// Render offline: the whole of `mode` (plus ring-out), capped at `max_seconds`.
/// Interleaved stereo.
pub fn render(engine: &mut Engine, max_seconds: f64) -> Vec<f32> {
    let max_frames = (max_seconds * engine.sr) as usize;
    let mut out = Vec::new();
    let mut block = vec![0f32; 1024];
    while out.len() / 2 < max_frames && !engine.finished() {
        let n = (max_frames - out.len() / 2).min(512);
        engine.process(&mut block[..n * 2]);
        out.extend_from_slice(&block[..n * 2]);
    }
    out
}

#[cfg(test)]
mod tests {
    include!("additive_route_tests.rs");
    use super::*;
    use deckard_model::{Group, Note, Pattern, Sampler, Scene, Section, Sound, Zone};

    pub(crate) fn project() -> Project {
        let notes = (0..4)
            .map(|i| Note { expr: None, sound: 0, position: i * PPQ, length: 240, pitch: 60, velocity: 127, probability: 100 })
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

    /// The audio thread hands meters and the playhead to the UI: peaks merge until
    /// taken, and publishing doesn't reallocate the engine's own meters.
    #[test]
    fn telemetry_merges_peaks_without_reallocating() {
        let mut e = click_engine(48_000);
        let mut t = Telemetry::default();
        let mut buf = vec![0f32; 256];
        e.process(&mut buf);
        e.publish(&mut t);
        assert!(t.meters[0][0].0 > 0.5, "the click is metered: {:?}", t.meters);
        assert!(t.sequencing && t.mode == Some(Mode::Arrangement));
        let ptr = e.meters[0].as_ptr();
        e.process(&mut buf);
        e.publish(&mut t);
        assert_eq!(e.meters[0].as_ptr(), ptr, "the engine's meters were reset in place");
        assert!(t.meters[0][0].0 > 0.5, "the peak stays until the UI takes it");
        assert!(t.position > 0.0);
    }

    /// A clip plays its own pattern (repeated to fill it) in place of the group's
    /// pattern in the section; past the sections it lengthens the song.
    #[test]
    fn clips_play_in_place_of_patterns() {
        let mut p = project();
        let clip_note = Note { expr: None, sound: 0, position: 0, length: 240, pitch: 72, velocity: 100, probability: 100 };
        p.groups[0].clip_patterns = vec![Pattern { length: 2 * PPQ, notes: vec![clip_note], ..Default::default() }];
        p.groups[0].clips = vec![deckard_model::Clip { position: 4 * PPQ, length: 4 * PPQ, pattern: 0 }];
        let t = timeline(&p, Mode::Arrangement);
        let got: Vec<(i64, i64)> = t.events.iter().map(|e| (e.tick, e.pitch)).collect();
        let mut want: Vec<(i64, i64)> = (0..4).map(|i| (i * PPQ, 60)).collect();
        want.extend([(4 * PPQ, 72), (6 * PPQ, 72)]);
        assert_eq!(got, want);
        assert_eq!(t.length, 8 * PPQ);
        p.groups[0].clips[0].position = 10 * PPQ;
        assert_eq!(timeline(&p, Mode::Arrangement).length, 14 * PPQ, "a clip after the last section");
        assert!(timeline(&p, Mode::Scene(0)).events.iter().all(|e| e.pitch == 60), "scenes don't play clips");
    }

    #[test]
    fn pattern_loops_to_fill_section() {
        let t = timeline(&project(), Mode::Arrangement);
        assert_eq!(t.length, 8 * PPQ);
        assert_eq!(t.events.iter().map(|e| e.tick).collect::<Vec<_>>(), (0..8).map(|i| i * PPQ).collect::<Vec<_>>());
    }

    /// Note Repeat in the engine: a held pad plays on the rate's grid until released,
    /// and what it plays is recorded.
    #[test]
    fn note_repeat_plays_and_records_a_held_pad() {
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        e.looping = true;
        e.set_recording(true);
        e.set_repeat(Some((deckard_model::Arp { rate: 240, gate: 0.5, ..Default::default() }, repeat::RepeatMode::NoteRepeat)));
        let mut out = vec![0f32; 512];
        e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
        // 960 ticks (a quarter at 120 BPM: 24 000 frames).
        let blocks = (960.0 * e.samples_per_tick() / 256.0).ceil() as usize;
        for _ in 0..blocks {
            e.process(&mut out);
        }
        e.schedule(0, Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
        for _ in 0..blocks {
            e.process(&mut out);
        }
        let rec = e.take_recorded();
        assert!((4..=5).contains(&rec.len()), "{rec:?}");
        assert!(rec.iter().all(|r| r.length == 120 && r.velocity == 100));
        let gaps: Vec<f64> = rec.windows(2).map(|w| w[1].tick - w[0].tick).collect();
        assert!(gaps.iter().all(|g| (g - 240.0).abs() < 1.0), "{gaps:?}");
    }

    /// Pressure for Note Repeat only (a pad in the Velocity pad mode): its repeats play
    /// at it, nothing is recorded as expression; per-note pressure (the aftertouch modes)
    /// leaves the repeats' velocity alone.
    #[test]
    fn repeat_pressure_sets_repeat_velocity_only() {
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        e.looping = true;
        e.set_recording(true);
        e.set_repeat(Some((deckard_model::Arp { rate: 240, gate: 0.5, ..Default::default() }, repeat::RepeatMode::NoteRepeat)));
        let mut out = vec![0f32; 512];
        let blocks = (960.0 * e.samples_per_tick() / 256.0).ceil() as usize;
        e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
        e.schedule(0, Live::RepeatPressure { group: 0, sound: 0, pitch: 60, velocity: 30 });
        for _ in 0..blocks {
            e.process(&mut out);
        }
        e.schedule(0, Live::Expression { group: 0, sound: 0, pitch: 60, kind: ExprKind::Dim(Dim::Pressure), value: 1.0 });
        for _ in 0..blocks {
            e.process(&mut out);
        }
        e.schedule(0, Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
        e.process(&mut out);
        let rec = e.take_recorded();
        assert!(rec.len() >= 6, "{rec:?}");
        assert!(rec.iter().skip(1).all(|r| r.velocity == 30), "{:?}", rec.iter().map(|r| r.velocity).collect::<Vec<_>>());
        let exprs = e.take_recorded_expr();
        assert_eq!(exprs.len(), 1, "only the per-note pressure: {exprs:?}");
    }

    /// A count-in isn't the end of playback: past half a second of it (nothing sounding
    /// but the clicks) the engine hasn't finished.
    #[test]
    fn a_count_in_is_not_finished() {
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        let beat = (e.samples_per_tick() * PPQ as f64).round() as u64;
        e.count_in(beat * 8);
        let mut out = vec![0f32; 512];
        for _ in 0..(48_000 / 256) {
            e.process(&mut out);
        }
        assert!(!e.finished(), "a second into the count-in");
    }

    /// A count-in: the metronome clicks (even when it's off) and the song waits; then it
    /// plays from where it was.
    #[test]
    fn count_in_clicks_then_the_song_starts() {
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        let beat = (e.samples_per_tick() * PPQ as f64).round() as u64;
        e.count_in(beat * 4);
        let mut out = vec![0f32; 512];
        let mut clicked = false;
        let mut done = 0;
        while done + 256 <= beat * 4 {
            e.process(&mut out);
            clicked |= out.iter().any(|x| x.abs() > 0.01);
            assert_eq!(e.position(), 0.0, "the song waits");
            done += 256;
        }
        assert!(clicked, "the count-in clicks");
        for _ in 0..8 {
            e.process(&mut out);
        }
        assert!(e.position() > 0.0, "then it plays");
    }

    /// With the metronome off, AUTOENABLE has it click while recording, and only then.
    #[test]
    fn the_metronome_clicks_while_recording_if_set_to() {
        let clicks = |auto: bool, recording: bool| {
            let mut cache = SampleCache::default();
            // (A song that plays, its notes silent.)
            let mut p = project();
            p.groups[0].sounds[0].sampler = None;
            let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
            e.metronome_auto = auto;
            e.set_recording(recording);
            let mut out = vec![0f32; 512];
            (0..8).any(|_| {
                e.process(&mut out);
                out.iter().any(|x| x.abs() > 0.01)
            })
        };
        assert!(clicks(true, true));
        assert!(!clicks(true, false), "not recording");
        assert!(!clicks(false, true), "not set");
    }

    /// The metronome's TIMESIGNATURE: eighths click twice a quarter note.
    #[test]
    fn the_metronome_ticks_on_its_note_value() {
        let onsets = |ticks: u32| {
            let mut cache = SampleCache::default();
            let mut p = project();
            p.groups[0].sounds[0].sampler = None;
            let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
            (e.metronome, e.metronome_ticks) = (true, ticks);
            // (A second: two quarter notes at 120 BPM.)
            let mut out = vec![0f32; 96_000];
            for block in out.chunks_mut(512) {
                e.process(block);
            }
            (0..out.len() / 2)
                .filter(|&k| (k == 0 || out[k * 2 - 2] == 0.0) && out[k * 2] == 0.0 && out.get(k * 2 + 2).is_some_and(|v| *v != 0.0))
                .count()
        };
        assert_eq!((onsets(4), onsets(8)), (2, 4));
    }

    /// While recording and the song plays, live notes (pads, MIDI) come back with the
    /// song position they played at, and their releases; not otherwise.
    #[test]
    fn live_notes_are_recorded_with_their_position() {
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        e.looping = true;
        let mut out = vec![0f32; 512];
        e.process(&mut out);
        e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
        e.process(&mut out);
        assert!(e.take_recorded().is_empty(), "not recording");
        e.set_recording(true);
        let at = e.position();
        e.schedule(0, Live::NoteOn { group: 0, sound: 1, pitch: 62, velocity: 90, length: i64::MAX });
        e.process(&mut out);
        for _ in 0..10 {
            e.process(&mut out);
        }
        e.schedule(0, Live::NoteRelease { group: 0, sound: 1, pitch: 62 });
        e.process(&mut out);
        let rec = e.take_recorded();
        assert_eq!(rec.len(), 2, "{rec:?}");
        assert_eq!((rec[0].group, rec[0].sound, rec[0].pitch, rec[0].velocity), (0, 1, 62, 90));
        assert!((rec[0].tick - at).abs() < 1.0, "{} vs {at}", rec[0].tick);
        assert_eq!(rec[1].velocity, 0);
        let ticks_per_block = 256.0 / e.samples_per_tick();
        assert!((rec[1].tick - rec[0].tick - 11.0 * ticks_per_block).abs() < 1.0);
        assert!(e.take_recorded().is_empty(), "taken once");
    }

    /// A note held over the loop end: its release comes earlier in the song than its
    /// start, but later on the clock that never wraps.
    #[test]
    fn played_keeps_counting_over_the_loop() {
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        e.looping = true;
        e.set_recording(true);
        let mut out = vec![0f32; 512];
        e.process(&mut out);
        e.schedule(0, Live::NoteOn { group: 0, sound: 1, pitch: 62, velocity: 90, length: i64::MAX });
        e.process(&mut out);
        let mut blocks = 1;
        let mut last = e.position();
        while e.position() >= last {
            last = e.position();
            e.process(&mut out);
            blocks += 1;
        }
        e.schedule(0, Live::NoteRelease { group: 0, sound: 1, pitch: 62 });
        e.process(&mut out);
        let rec = e.take_recorded();
        assert_eq!(rec.len(), 2, "{rec:?}");
        assert!(rec[1].tick <= rec[0].tick, "the song wrapped: {rec:?}");
        let ticks_per_block = 256.0 / e.samples_per_tick();
        assert!((rec[1].played - rec[0].played - blocks as f64 * ticks_per_block).abs() < 1.0, "{rec:?}");
    }

    /// A count-in: a note in its last 1/16 is recorded where the song starts; one
    /// earlier is heard, not recorded.
    #[test]
    fn the_count_ins_last_sixteenth_is_recorded_at_the_start() {
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        let spt = e.samples_per_tick();
        let beat = (spt * PPQ as f64).round() as u64;
        e.count_in(beat * 4);
        e.set_recording(true);
        let mut out = vec![0f32; 512];
        let mut done = 0u64;
        let mut played_early = None;
        while done + 256 < beat * 4 {
            let left = (beat * 4 - done) as f64 / spt;
            if left > 600.0 && left < 700.0 && played_early.is_none() {
                e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
                played_early = Some(done);
            }
            if left < 200.0 {
                e.schedule(0, Live::NoteOn { group: 0, sound: 1, pitch: 62, velocity: 90, length: i64::MAX });
                e.process(&mut out);
                break;
            }
            e.process(&mut out);
            done += 256;
        }
        assert!(played_early.is_some());
        let rec = e.take_recorded();
        assert_eq!(rec.len(), 1, "only the late one: {rec:?}");
        assert_eq!((rec[0].sound, rec[0].tick), (1, 0.0), "at the start");
    }

    /// Per-note expression played live is recorded (with its song position) and not
    /// played: one note's bend doesn't bend the sound (part 2 sends it on).
    #[test]
    fn expression_is_recorded_not_played() {
        use deckard_model::{Dim, ExprKind};
        let mut cache = SampleCache::default();
        let mut render = |bend: bool| {
            let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
            e.looping = true;
            e.set_recording(true);
            let mut out = vec![0f32; 512];
            e.process(&mut out);
            e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
            if bend {
                let kind = ExprKind::Dim(Dim::Pitch);
                e.schedule(0, Live::Expression { group: 0, sound: 0, pitch: 60, kind, value: 12.0 });
            }
            let mut all = Vec::new();
            for _ in 0..8 {
                e.process(&mut out);
                all.extend_from_slice(&out);
            }
            (all, e.take_recorded_expr())
        };
        let (plain, none) = render(false);
        let (bent, rec) = render(true);
        assert!(none.is_empty());
        assert_eq!(rec.len(), 1, "{rec:?}");
        assert_eq!((rec[0].sound, rec[0].pitch, rec[0].value), (0, 60, 12.0));
        assert_eq!(plain, bent, "not played");
    }

    /// A MIDI controller is recorded only when the input asks for it (the sound that
    /// records it), and only while recording.
    #[test]
    fn control_in_records_only_when_asked() {
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        e.looping = true;
        let mut out = vec![0f32; 512];
        e.process(&mut out);
        let cc = MidiKind::Controller { number: 1, value: 64 };
        e.schedule(0, Live::ControlIn { group: 0, sound: 0, kind: cc, thru: false, record: true });
        e.process(&mut out);
        assert!(e.take_recorded_controls().is_empty(), "not recording");
        e.set_recording(true);
        e.schedule(0, Live::ControlIn { group: 0, sound: 0, kind: cc, thru: false, record: true });
        e.schedule(0, Live::ControlIn { group: 0, sound: 1, kind: cc, thru: false, record: false });
        e.process(&mut out);
        let rec = e.take_recorded_controls();
        assert_eq!(rec.iter().map(|r| (r.sound, r.kind)).collect::<Vec<_>>(), vec![(0, cc)]);
    }

    /// The browser's prehear sounds while nothing plays, once, and stops.
    #[test]
    fn prehear_sounds_with_the_song_stopped() {
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        assert!(e.prehear(Some(Arc::new(Sample { rate: 48_000, frames: vec![[0.5, 0.5]; 64] })), 1.0).is_none());
        let mut out = vec![0f32; 256];
        e.process(&mut out);
        assert!(out[..120].iter().all(|x| (*x - 0.5).abs() < 1e-3), "{:?}", &out[..8]);
        assert!(out[130..].iter().all(|x| *x == 0.0));
        assert!(e.prehear(None, 1.0).is_some());
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

    /// Pads played live (and MIDI notes): a Send links and chokes as in the timeline
    /// (`pads`): the linked sound plays too; a choked one fades out.
    #[test]
    fn live_pads_link_and_choke() {
        let engine = |choke: bool| {
            let mut p = project();
            let other = p.groups[0].sounds[0].clone();
            p.groups[0].sounds.push(other);
            for (i, c) in p.groups[0].sounds.iter_mut().map(|s| &mut s.channel).enumerate() {
                if choke {
                    (c.choke_group, c.choke_mode) = (1, i as i64);
                } else {
                    (c.link_group, c.link_mode) = (1, i as i64);
                }
            }
            let mut e = click_engine_for(&p, 1000);
            e.sequencing = false;
            set_sample(&mut e, vec![[0.5, 0.5]; 100], &[("envelope_type", 2.0)], true);
            e
        };
        let voices = |e: &Engine, s: usize| e.sounds[0][s].as_ref().map_or(0, |x| x.voices());
        let on = |sound| Live::NoteOn { group: 0, sound, pitch: 60, velocity: 100, length: i64::MAX };
        let mut out = vec![0f32; 2 * 50];
        let mut e = engine(false);
        e.play_live(on(0));
        assert_eq!((voices(&e, 0), voices(&e, 1)), (1, 1), "linked: both play");
        e.play_live(Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
        e.process(&mut out);
        let mut e = engine(true);
        e.play_live(on(1));
        e.process(&mut out);
        assert_eq!(voices(&e, 1), 1);
        e.play_live(on(0));
        e.process(&mut out);
        assert_eq!((voices(&e, 0), voices(&e, 1)), (1, 0), "the Send chokes the Receive");
    }

    /// Choke All Notes ends what every sound plays, whatever its choke group; the next note
    /// plays as usual.
    #[test]
    fn choke_all_ends_every_voice() {
        let mut p = project();
        let other = p.groups[0].sounds[0].clone();
        p.groups[0].sounds.push(other);
        let mut e = click_engine_for(&p, 1000);
        e.sequencing = false;
        set_sample(&mut e, vec![[0.5, 0.5]; 100], &[("envelope_type", 2.0)], true);
        let voices = |e: &Engine, s: usize| e.sounds[0][s].as_ref().map_or(0, |x| x.voices());
        let on = |sound| Live::NoteOn { group: 0, sound, pitch: 60, velocity: 100, length: i64::MAX };
        let mut out = vec![0f32; 2 * 50];
        e.play_live(on(0));
        e.play_live(on(1));
        e.process(&mut out);
        assert_eq!((voices(&e, 0), voices(&e, 1)), (1, 1));
        e.play_live(Live::ChokeAll);
        e.process(&mut out);
        assert_eq!((voices(&e, 0), voices(&e, 1)), (0, 0), "both end");
        e.play_live(on(0));
        e.process(&mut out);
        assert_eq!(voices(&e, 0), 1, "the next note plays");
    }

    /// #21: a sound that plays only by a link is silent while the sound with the notes is
    /// muted; muted alone, it's silent and the Send still plays.
    #[test]
    fn muting_the_send_silences_its_links() {
        let heard = |mute: usize| {
            let mut p = project();
            let mut other = p.groups[0].sounds[0].clone();
            other.channel.link_mode = 1;
            p.groups[0].sounds.push(other);
            p.groups[0].sounds[0].channel.link_group = 1;
            p.groups[0].sounds[1].channel.link_group = 1;
            p.groups[0].sounds[mute].muted = true;
            let mut e = click_engine_for(&p, 1000);
            render(&mut e, 2.0).iter().any(|v| v.abs() > 0.01)
        };
        assert!(!heard(0), "the Send muted: its link is silent too");
        assert!(heard(1), "the Receive muted: the Send still plays");
    }

    #[test]
    fn onsets_are_sample_accurate() {
        // 120 BPM: a quarter note is 0.5 s = 24000 frames at 48 kHz.
        let mut e = click_engine(48_000);
        let out = render(&mut e, 10.0);
        let onsets: Vec<usize> = (0..out.len() / 2).filter(|&i| out[i * 2] > 0.5).collect();
        assert_eq!(onsets, (0..8).map(|i| i * 24_000).collect::<Vec<_>>());
    }

    #[test]
    fn octave_up_plays_twice_as_fast() {
        let sr = 1000;
        let mut e = click_engine(sr);
        // a ramp sample, triggered an octave up: every other frame
        // A ramp, scaled to stay within the engine's output range (±4).
        let ramp: Vec<[f32; 2]> = (0..100).map(|i| [i as f32 / 100.0, i as f32 / 100.0]).collect();
        set_sample(&mut e, ramp, &[], false);
        e.timeline = Timeline {
            events: vec![NoteEvent {
                tick: 0,
                group: 0,
                sound: 0,
                pitch: 72,
                velocity: 127,
                length: 1,
                probability: 100,
                midi: None,
                cue: None,
                expr: None,
                from: None,
            }],
            length: 1,
            mods: Vec::new(),
            faders: Vec::new(),
            exprs: Vec::new(),
            prob_locked: Vec::new(),
            tempos: Vec::new(),
        };
        let out = render(&mut e, 1.0);
        let left: Vec<f32> = out.iter().step_by(2).map(|x| x * 100.0).collect();
        for (i, want) in [0.0, 2.0, 4.0, 6.0, 8.0].iter().enumerate() {
            assert!((left[i] - want).abs() < 1e-3, "frame {i}: {}", left[i]);
        }
        assert!((left[49] - 98.0).abs() < 1e-2 && left[50].abs() < 1e-2, "voice ends at the sample end: {:?}", &left[48..52]);
    }

    #[test]
    fn fifth_up_interpolates() {
        let sr = 1000;
        let mut e = click_engine(sr);
        let ramp: Vec<[f32; 2]> = (0..100).map(|i| [i as f32 / 100.0, i as f32 / 100.0]).collect();
        set_sample(&mut e, ramp, &[], false);
        e.timeline = Timeline {
            events: vec![NoteEvent {
                tick: 0,
                group: 0,
                sound: 0,
                pitch: 67,
                velocity: 127,
                length: 1,
                probability: 100,
                midi: None,
                cue: None,
                expr: None,
                from: None,
            }],
            length: 1,
            mods: Vec::new(),
            faders: Vec::new(),
            exprs: Vec::new(),
            prob_locked: Vec::new(),
            tempos: Vec::new(),
        };
        let out = render(&mut e, 1.0);
        // The 8-point interpolator follows a ramp once its history is filled.
        let ratio = 2f32.powf(7.0 / 12.0);
        for (i, &l) in out.iter().step_by(2).enumerate().skip(4).take(20) {
            assert!((l * 100.0 - i as f32 * ratio).abs() < 0.02, "frame {i}: {l}");
        }
    }

    #[test]
    fn a_note_played_live_is_reported_and_a_sequenced_one_isnt() {
        let mut e = click_engine(1000);
        let mut out = vec![0f32; 512];
        e.note_on(0, 1, 60, 100, PPQ);
        assert_eq!(e.live_hit.0, 0, "the sequence's way in");
        e.schedule(0, Live::NoteOn { group: 0, sound: 1, pitch: 72, velocity: 90, length: PPQ });
        e.process(&mut out);
        assert_eq!(e.live_hit, (1, 0, 1, 72, 90));
        e.schedule(0, Live::NoteIn { group: 2, sound: 3, pitch: 40, velocity: 30, thru: false });
        e.process(&mut out);
        assert_eq!(e.live_hit, (2, 2, 3, 40, 30));
    }

    /// What's recorded says whether it came in over MIDI.
    #[test]
    fn a_recorded_note_says_if_it_was_midi() {
        let mut e = click_engine(1000);
        e.set_recording(true);
        let mut out = vec![0f32; 512];
        e.schedule(0, Live::NoteOn { group: 0, sound: 1, pitch: 72, velocity: 90, length: PPQ });
        e.schedule(0, Live::NoteIn { group: 0, sound: 0, pitch: 40, velocity: 30, thru: false });
        e.process(&mut out);
        let by_pitch = |p: i64| e.recorded.iter().find(|r| r.pitch == p).expect("recorded");
        assert!(!by_pitch(72).midi, "a pad");
        assert!(by_pitch(40).midi);
    }

    #[test]
    fn note_on_plays_without_the_sequencer() {
        let mut e = click_engine(1000);
        e.sequencing = false;
        let mut out = vec![0f32; 200];
        e.process(&mut out[..100]);
        assert!(out.iter().all(|s| *s == 0.0));
        e.note_on(0, 0, 60, 127, PPQ);
        e.process(&mut out[100..]);
        assert_eq!(out[100], 1.0, "the click starts at the next block");
        assert_eq!(e.position(), 0.0, "the song position doesn't move");
    }

    #[test]
    fn tap_takes_a_sound_its_group_or_the_master() {
        for (source, hears) in [
            (TapSource::Sound(0, 0), true),
            (TapSource::Group(0), true),
            (TapSource::Master, true),
            (TapSource::Group(1), false),
            (TapSource::Sound(0, 1), false),
        ] {
            let mut e = click_engine(1000);
            e.sequencing = false;
            let buffer = Arc::new(Mutex::new(Vec::new()));
            e.set_tap(Some(Tap { source, buffer: buffer.clone() }));
            e.note_on(0, 0, 60, 127, PPQ);
            let mut out = vec![0f32; 200];
            e.process(&mut out);
            let got = buffer.lock().unwrap().clone();
            assert_eq!(got.len(), out.len(), "{source:?}: every frame is handed over");
            if hears {
                assert_eq!(got, out, "{source:?}: the tap hears what plays");
            } else {
                assert!(got.iter().all(|s| *s == 0.0), "{source:?}: silence");
            }
        }
    }

    #[test]
    fn a_scene_plays_at_its_own_tempo_from_when_it_starts() {
        let mut p = project();
        // Scene 1, the same quarters, at twice the song's tempo.
        p.scenes.push(Scene { patterns: vec![(0, 0)], tempo: Some(240.0), ..Default::default() });
        // 120 BPM at 1 kHz: a quarter is 500 frames, a bar 2000; at 240, 250 and 1000.
        let mut e = click_engine_for(&p, 1000);
        e.jump(&p, Mode::Scene(0));
        e.looping = true;
        let mut out = vec![0f32; 2 * 3500];
        e.process(&mut out[..2 * 700]);
        assert_eq!(e.tempo(), 120.0);
        e.queue(&p, Mode::Scene(1));
        e.process(&mut out[2 * 700..2 * 2100]);
        assert_eq!(e.tempo(), 240.0, "the scene's, once it starts");
        // Back: the song's again, on the bar.
        e.queue(&p, Mode::Scene(0));
        e.process(&mut out[2 * 2100..]);
        let onsets: Vec<usize> = (0..3500).filter(|&i| out[i * 2] > 0.5).collect();
        assert_eq!(onsets, [0, 500, 1000, 1500, 2000, 2250, 2500, 2750, 3000]);
        assert_eq!(e.tempo(), 120.0);
    }

    #[test]
    fn a_section_plays_at_its_scenes_tempo_and_the_timeline_knows_how_long_that_is() {
        let mut p = project();
        p.scenes.push(Scene { patterns: vec![(0, 0)], tempo: Some(240.0), ..Default::default() });
        let bar = 4 * PPQ;
        p.sections = vec![
            Section { scene: Some(0), length: bar, auto: false },
            Section { scene: Some(1), length: bar, auto: false },
            Section { scene: Some(0), length: bar, auto: false },
        ];
        let mut e = click_engine_for(&p, 1000);
        e.jump(&p, Mode::Arrangement);
        assert_eq!(e.timeline_frames().round() as usize, 2000 + 1000 + 2000);
        let mut out = vec![0f32; 2 * 3400];
        e.process(&mut out);
        let onsets: Vec<usize> = (0..3400).filter(|&i| out[i * 2] > 0.5).collect();
        assert_eq!(onsets, [0, 500, 1000, 1500, 2000, 2250, 2500, 2750, 3000]);
        assert_eq!(e.tempo(), 120.0);
    }

    /// Under a clock (Link, incoming MIDI clock) the clock's tempo is the tempo.
    #[test]
    fn a_followed_clock_overrides_a_scenes_tempo() {
        let mut p = project();
        p.scenes[0].tempo = Some(240.0);
        let mut e = click_engine_for(&p, 1000);
        e.jump(&p, Mode::Scene(0));
        e.set_followed(true);
        let mut out = vec![0f32; 2 * 100];
        e.process(&mut out);
        assert_eq!(e.tempo(), 120.0);
        e.set_followed(false);
        e.process(&mut out);
        assert_eq!(e.tempo(), 240.0);
    }

    /// A tempo edit moves what plays at the song's tempo, not a scene with its own.
    #[test]
    fn a_song_tempo_edit_leaves_a_scene_with_its_own_alone() {
        let mut p = project();
        p.scenes[0].tempo = Some(90.0);
        let mut e = click_engine_for(&p, 1000);
        e.jump(&p, Mode::Scene(0));
        let mut out = vec![0f32; 2 * 100];
        e.process(&mut out);
        assert_eq!(e.tempo(), 90.0);
        p.tempo = 200.0;
        let mut cache = SampleCache::default();
        e.update(&p, None, &mut cache);
        e.process(&mut out);
        assert_eq!(e.tempo(), 90.0);
    }

    #[test]
    fn launched_scene_starts_when_the_playing_one_ends() {
        // Scene 0: quarters for a bar; scene 1: half notes for a bar.
        let mut p = project();
        let halves = (0..2)
            .map(|i| Note { expr: None, sound: 0, position: i * 2 * PPQ, length: 240, pitch: 60, velocity: 127, probability: 100 })
            .collect();
        p.groups[0].patterns.push(Pattern { length: 4 * PPQ, notes: halves, ..Default::default() });
        p.scenes.push(Scene { patterns: vec![(0, 1)], ..Default::default() });
        // 120 BPM at 1 kHz: a quarter is 500 frames, a bar 2000.
        let mut e = click_engine_for(&p, 1000);
        e.jump(&p, Mode::Scene(0));
        e.looping = true;
        let mut out = vec![0f32; 2 * 6000];
        e.process(&mut out[..2 * 700]);
        e.queue(&p, Mode::Scene(1));
        assert_eq!((e.mode(), e.queued()), (Mode::Scene(0), Some(Mode::Scene(1))));
        e.process(&mut out[2 * 700..]);
        let onsets: Vec<usize> = (0..6000).filter(|&i| out[i * 2] > 0.5).collect();
        assert_eq!(onsets, [0, 500, 1000, 1500, 2000, 3000, 4000, 5000]);
        assert_eq!((e.mode(), e.queued()), (Mode::Scene(1), None));
    }

    #[test]
    fn update_keeps_position_and_follows_tempo() {
        // 120 BPM at 1 kHz: a quarter is 500 frames.
        let mut p = project();
        let mut e = click_engine_for(&p, 1000);
        let mut out = vec![0f32; 2 * 1250];
        e.process(&mut out);
        assert_eq!(e.position(), 1250.0 * 960.0 / 500.0, "2.5 beats in");
        // Twice the tempo: the next beats come every 250 frames from the same position.
        p.tempo = 240.0;
        let mut cache = SampleCache::default();
        let samples = e.samples.clone();
        let sound = e.sounds[0][0].take();
        e.update(&p, None, &mut cache);
        e.samples = samples; // the test sample (Engine::new found no files)
        e.sounds[0][0] = sound;
        assert_eq!(e.position(), 2.5 * 960.0);
        let mut out = vec![0f32; 2 * 1000];
        e.process(&mut out);
        let onsets: Vec<usize> = (0..1000).filter(|&i| out[i * 2] > 0.5).collect();
        assert_eq!(onsets, [125, 375, 625, 875], "beats 3..6 at 240 BPM");
    }

    /// #14: a tempo edit while playing, as the app sends it (built after the playing
    /// project, then adopted), plays at the new tempo from where it is.
    #[test]
    fn an_adopted_tempo_edit_follows_the_tempo() {
        let p = project();
        let mut e = click_engine_for(&p, 1000);
        let mut out = vec![0f32; 2 * 1250];
        e.process(&mut out);
        let mut q = p.clone();
        q.tempo = 240.0;
        let mut cache = SampleCache::default();
        let chains: Vec<Vec<bool>> = p.groups.iter().map(|g| vec![false; g.sounds.len()]).collect();
        let (fresh, _) = Engine::new_after(&q, Some((&p, &chains)), None, 1000, e.mode(), &mut cache);
        let mut timelines = if fresh.keeps_timelines() { Timelines::default() } else { Timelines::of(&q, &[e.mode()]) };
        let samples = e.samples.clone();
        let sound = e.sounds[0][0].take();
        let _old = e.adopt_with(fresh, &mut timelines);
        e.samples = samples;
        e.sounds[0][0] = sound;
        assert_eq!(e.tempo(), 240.0);
        assert_eq!(e.position(), 2.5 * 960.0);
        let mut out = vec![0f32; 2 * 1000];
        e.process(&mut out);
        let onsets: Vec<usize> = (0..1000).filter(|&i| out[i * 2] > 0.5).collect();
        assert_eq!(onsets, [125, 375, 625, 875], "beats 3..6 at 240 BPM");
    }

    #[test]
    fn midi_events_join_the_timeline_before_notes() {
        let mut p = project();
        let pedal = |position, value| deckard_model::MidiEvent { sound: 0, position, kind: MidiKind::Controller { number: 64, value } };
        p.groups[0].patterns[0].midi = vec![pedal(0, 127), pedal(2 * PPQ + 10, 0)];
        let t = timeline(&p, Mode::Pattern(0, 0));
        let first: Vec<Option<MidiKind>> = t.events.iter().take(2).map(|e| e.midi).collect();
        assert_eq!(
            first,
            [Some(MidiKind::Controller { number: 64, value: 127 }), None],
            "the pedal goes down before the note at the same tick"
        );
        assert!(t.events.iter().any(|e| e.tick == 2 * PPQ + 10 && e.midi == Some(MidiKind::Controller { number: 64, value: 0 })));
        assert_eq!(t.events.iter().filter(|e| e.midi.is_none()).count(), 4);
    }

    #[test]
    fn held_notes_sound_until_released() {
        let mut e = click_engine(1000);
        e.sequencing = false;
        // A looped, sustaining sound (ADSR) with a short release.
        set_sample(&mut e, vec![[0.5, 0.5]; 100], &[("envelope_type", 2.0), ("volume_envelope/release", 0.0)], true);
        e.note_hold(0, 0, 60, 127);
        let mut out = vec![0f32; 2 * 3000];
        e.process(&mut out);
        assert!(out[2 * 2999] > 0.4, "still sounding after 3 s (six beats)");
        e.note_release(0, 0, 60);
        let mut out = vec![0f32; 2 * 100];
        e.process(&mut out);
        assert!(out[2 * 99].abs() < 1e-6, "silent after the release");
        assert!(e.finished());
    }

    /// Play `notes` (start frame, pitch, velocity, length frames) of a 1 kHz sine at
    /// 44.1 kHz through a Sampler with `params`; returns the left channel.
    fn play_sine(params: &[(&str, f64)], notes: &[(usize, i64, i64, usize)], frames: usize) -> Vec<f32> {
        play_tone(1000.0, params, notes, frames)
    }

    fn play_tone(hz: f32, params: &[(&str, f64)], notes: &[(usize, i64, i64, usize)], frames: usize) -> Vec<f32> {
        let sr = 44_100.0;
        let mut e = click_engine(sr as u32);
        e.sequencing = false;
        let sine: Vec<[f32; 2]> =
            (0..44_100).map(|i| (i as f32 * hz * std::f32::consts::TAU / sr).sin()).map(|x| [x * 0.5, x * 0.5]).collect();
        set_sample(&mut e, sine, params, false);
        let mut out = vec![0f32; frames * 2];
        let mut at = 0;
        let mut events: Vec<(usize, bool, i64, i64)> =
            notes.iter().flat_map(|&(t, p, v, l)| [(t, true, p, v), (t + l, false, p, v)]).collect();
        events.sort_by_key(|e| (e.0, e.1));
        for (t, on, pitch, vel) in events {
            if t > at {
                e.process(&mut out[at * 2..t.min(frames) * 2]);
                at = t.min(frames);
            }
            if on {
                e.note_hold(0, 0, pitch, vel);
            } else {
                e.note_release(0, 0, pitch);
            }
        }
        if at < frames {
            e.process(&mut out[at * 2..]);
        }
        out.iter().step_by(2).copied().collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn sampler_velocity_scales_as_its_cube() {
        let full = play_sine(&[], &[(0, 60, 127, 4410)], 4410);
        let half = play_sine(&[], &[(0, 60, 64, 4410)], 4410);
        let ratio = rms(&half[100..]) / rms(&full[100..]);
        let want = (64.0f32 / 127.0).powi(3);
        assert!((ratio - want).abs() < 0.01, "velocity 64: {ratio} (want {want})");
        // Velocity → Volume at 0: velocity doesn't matter.
        let flat = play_sine(&[("velocity_volume", 0.0)], &[(0, 60, 64, 4410)], 4410);
        assert!((rms(&flat[100..]) / rms(&full[100..]) - 1.0).abs() < 0.01);
    }

    #[test]
    fn sampler_low_pass_cuts_highs() {
        let open = play_sine(&[], &[(0, 60, 127, 8820)], 8820);
        let lp = play_sine(&[("filter", 1.0), ("cutoff", 0.2)], &[(0, 60, 127, 8820)], 8820);
        let hp = play_sine(&[("filter", 3.0), ("cutoff", 0.2)], &[(0, 60, 127, 8820)], 8820);
        let (o, l, h) = (rms(&open[4410..]), rms(&lp[4410..]), rms(&hp[4410..]));
        assert!(crate::sampler::cutoff_hz(0.2, 1) < 400.0, "{}", crate::sampler::cutoff_hz(0.2, 1));
        assert!(l < 0.2 * o, "1 kHz through a low pass at {:.0} Hz: {l} vs {o}", crate::sampler::cutoff_hz(0.2, 1));
        assert!(h > 0.7 * o, "and through a high pass: {h} vs {o}");
    }

    /// Level (dB, relative to no filter) of tones at several frequencies through each
    /// filter mode, and how far each engine model's output is from Standard.
    #[test]
    fn sampler_filters_and_engine_models() {
        let freqs = [100.0, 300.0, 1000.0, 3000.0, 8000.0];
        let level = |hz: f32, params: &[(&str, f64)]| rms(&play_tone(hz, params, &[(0, 60, 127, 8820)], 8820)[4410..]);
        let db = |a: f32, b: f32| 20.0 * (a / b).log10();
        let mut table = Vec::new();
        for (name, params) in [
            ("LP2 cutoff .55 res 0", vec![("filter", 1.0), ("cutoff", 0.55)]),
            ("LP2 cutoff .55 res .8", vec![("filter", 1.0), ("cutoff", 0.55), ("resonance", 0.8)]),
            ("BP2 cutoff .55", vec![("filter", 2.0), ("cutoff", 0.55)]),
            ("HP2 cutoff .55", vec![("filter", 3.0), ("cutoff", 0.55)]),
            ("EQ 1 kHz gain +.5", vec![("filter", 4.0), ("eq_frequency", 0.5663), ("eq_gain", 0.5)]),
            ("EQ 1 kHz gain -1", vec![("filter", 4.0), ("eq_frequency", 0.5663), ("eq_gain", -1.0)]),
        ] {
            let row: Vec<f32> = freqs.iter().map(|&f| db(level(f, &params), level(f, &[]))).collect();
            eprintln!("{name:24} {}", row.iter().map(|d| format!("{d:7.1}")).collect::<String>());
            table.push(row);
        }
        eprintln!(
            "cutoff .55: LP {:.0} Hz, BP {:.0} Hz, HP {:.0} Hz",
            crate::sampler::cutoff_hz(0.55, 1),
            crate::sampler::cutoff_hz(0.55, 2),
            crate::sampler::cutoff_hz(0.55, 3)
        );
        // Low pass: passes lows, cuts highs; resonance boosts near the cutoff.
        assert!(table[0][0] > -1.0 && table[0][4] < -12.0, "LP {:?}", table[0]);
        // Band pass: peaks in the middle; high pass: the opposite of the low pass.
        assert!(table[2][2] > table[2][0] && table[2][2] > table[2][4], "BP {:?}", table[2]);
        assert!(table[3][0] < -12.0 && table[3][4] > -1.0, "HP {:?}", table[3]);
        // EQ: Maschine's peak band reaches A² at its centre (A = 2^(dB/6); see
        // docs/sampler-dsp.md §8.5): +18 dB at gain 0.5, -36 dB at -1; less away from it.
        assert!((table[4][2] - 18.0).abs() < 1.5 && table[4][0] < table[4][2] - 10.0, "EQ+ {:?}", table[4]);
        assert!((table[5][2] + 36.0).abs() < 1.5, "EQ- {:?}", table[5]);
        // Engine models: difference from Standard (12-bit quantizing, MP60's gain).
        let std_ = play_tone(1000.0, &[], &[(0, 60, 127, 8820)], 8820);
        for m in 1..=6 {
            let x = play_tone(1000.0, &[("mode", m as f64)], &[(0, 60, 127, 8820)], 8820);
            let diff: Vec<f32> = x.iter().zip(&std_).map(|(a, b)| a - b).collect();
            eprintln!(
                "mode {m}: level {:+.2} dB, difference {:.1} dB below the signal",
                db(rms(&x), rms(&std_)),
                -db(rms(&diff), rms(&std_))
            );
        }
    }

    /// Random Sampler settings (every parameter over its whole range, as knobs can set
    /// them) never panic, never produce NaN or infinity, and render in good time.
    #[test]
    fn sampler_survives_any_settings() {
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let ranges: Vec<(&str, f64, f64)> = deckard_model::SAMPLER_PARAMS
            .iter()
            .map(|(k, _)| {
                let (lo, hi) = match *k {
                    "polyphony" => (0.0, 7.0),
                    "mode" => (0.0, 6.0),
                    "envelope_type" => (0.0, 2.0),
                    "filter" => (0.0, 4.0),
                    "lfo/type" => (0.0, 4.0),
                    "lfo/sync_mode" => (0.0, 2.0),
                    "reverse" => (0.0, 1.0),
                    "lfo/phase" => (-0.5, 0.5),
                    k if k.starts_with("velocity_")
                        || k.starts_with("modwheel_")
                        || k.contains("amount")
                        || k == "tune"
                        || k == "eq_gain" =>
                    {
                        (-1.0, 1.0)
                    }
                    k if k.starts_with("mod_envelope/") && ["pitch", "cutoff", "drive", "pan"].iter().any(|d| k.ends_with(d)) => {
                        (-1.0, 1.0)
                    }
                    _ => (0.0, 1.0),
                };
                (*k, lo, hi)
            })
            .collect();
        let t = std::time::Instant::now();
        for round in 0..300 {
            let mut params: Vec<(&str, f64)> = Vec::new();
            for (k, lo, hi) in &ranges {
                let v = match round % 3 {
                    0 => lo + rnd() * (hi - lo),
                    1 => {
                        if rnd() < 0.5 {
                            *lo
                        } else {
                            *hi
                        }
                    }
                    _ => (lo + hi) / 2.0 + (rnd() - 0.5) * 0.1 * (hi - lo),
                };
                let v = if ["polyphony", "mode", "envelope_type", "filter", "lfo/type", "lfo/sync_mode", "reverse"].contains(k) {
                    v.round()
                } else {
                    v
                };
                params.push((k, v));
            }
            let notes = [(0, 36, 127, 300), (100, 60, 1, 2000), (150, 96, 64, 50), (400, 127, 127, 100), (500, 0, 100, 900)];
            let out = play_tone(440.0, &params, &notes, 2000);
            assert!(out.iter().all(|x| x.is_finite()), "round {round}: not finite with {params:?}");
            assert!(out.iter().all(|x| x.abs() < 100.0), "round {round}: blew up with {params:?}");
        }
        assert!(t.elapsed().as_secs_f64() < 60.0, "too slow: {:?}", t.elapsed());
    }

    /// Knob drags while notes sound: every block, the Sampler is rebuilt with other
    /// settings and takes over the sounding voices (as `Engine::adopt` does).
    #[test]
    fn sampler_survives_settings_changing_while_playing() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let sr = 44_100.0;
        let mut e = click_engine(sr as u32);
        e.sequencing = false;
        let sine: Vec<[f32; 2]> = (0..20_000).map(|i| (i as f32 * 0.05).sin()).map(|x| [x * 0.5, x * 0.5]).collect();
        set_sample(&mut e, sine, &[("envelope_type", 2.0)], true);
        let zone_sample = e.samples[0].clone();
        let mut out = vec![0f32; 2 * 64];
        let keys = [
            "lfo/cutoff_amount",
            "lfo/speed",
            "lfo/type",
            "lfo/sync_mode",
            "lfo/phase",
            "cutoff",
            "resonance",
            "filter",
            "mod_envelope/cutoff",
            "drive",
            "comp",
            "bits",
            "sample_rate",
            "polyphony",
            "mode",
            "envelope_type",
            "glide",
            "lfo/pitch_amount",
            "lfo/pan_amount",
            "tune",
            "start",
            "reverse",
            "eq_frequency",
            "eq_gain",
            "eq_width",
        ];
        let mut model = deckard_model::Sampler::default();
        for block in 0..20_000 {
            if block % 37 == 0 {
                e.note_hold(0, 0, 30 + (rnd() * 70.0) as i64, 1 + (rnd() * 126.0) as i64);
            }
            if block % 53 == 0 {
                e.note_release(0, 0, 30 + (rnd() * 70.0) as i64);
            }
            // A knob moves.
            let k = keys[(rnd() * keys.len() as f64) as usize];
            let v = match k {
                "filter" => (rnd() * 4.99).floor(),
                "lfo/type" => (rnd() * 4.99).floor(),
                "lfo/sync_mode" | "envelope_type" => (rnd() * 2.99).floor(),
                "polyphony" => (rnd() * 7.99).floor(),
                "mode" => (rnd() * 6.99).floor(),
                "reverse" => (rnd() * 1.99).floor(),
                "lfo/phase" => rnd() - 0.5,
                k if k.contains("amount") || k == "tune" || k == "eq_gain" || k == "mod_envelope/cutoff" => rnd() * 2.0 - 1.0,
                _ => rnd(),
            };
            model.set(k, v);
            let old = e.sounds[0][0].take().unwrap();
            let zone = sampler::ZoneRt {
                sample: zone_sample.clone(),
                root: 60,
                keys: (0, 127),
                vels: (0, 127),
                gain: 0.75,
                pan: 0.0,
                tune: 0.0,
                play: (0, 20_000),
                looped: Some((5_000, 15_000)),
                x_fade: (rnd() * 3000.0) as usize,
                fade_in: 0,
                fade_out: 0,
            };
            let mut fresh = SamplerRt::new(sampler::Settings::new(&model, sr), vec![zone], (1.0, 1.0), sr);
            fresh.adopt(old);
            e.sounds[0][0] = Some(fresh);
            e.process(&mut out);
            assert!(out.iter().all(|x| x.is_finite()), "block {block}: {k} = {v}: {:?} with {:?}", &out[..4], model.params);
            // Before the output's safety clamp: the voices themselves stay in bounds.
            let peak = e.sounds[0][0].as_ref().unwrap().last_peak();
            assert!(peak < 32.0, "block {block}: {k} = {v}: voice peak {peak} with {:?}", model.params);
        }
    }

    #[test]
    fn sampler_mono_chokes_the_previous_note() {
        // Polyphony "1": the second note fades the first out over 10 ms.
        let two = play_sine(&[("polyphony", 1.0)], &[(0, 60, 127, 8820), (2205, 72, 127, 8820)], 8820);
        let poly = play_sine(&[], &[(0, 60, 127, 8820), (2205, 72, 127, 8820)], 8820);
        let (mono, both) = (rms(&two[4000..]), rms(&poly[4000..]));
        assert!(mono < both * 0.8, "one voice left with polyphony 1: {mono} vs {both}");
    }

    #[test]
    fn sampler_envelopes() {
        // AHD (type 1): plays its hold and decay even after a short note.
        let ahd =
            play_sine(&[("envelope_type", 1.0), ("volume_envelope/hold", 0.4), ("volume_envelope/decay", 0.4)], &[(0, 60, 127, 100)], 8820);
        assert!(rms(&ahd[2000..2500]) > 0.1, "AHD ignores the note-off");
        // ADSR: sustains at sustain³ while held, then releases.
        let adsr = play_sine(
            &[("envelope_type", 2.0), ("volume_envelope/decay", 0.0), ("volume_envelope/sustain", 0.5), ("volume_envelope/release", 0.0)],
            &[(0, 60, 127, 4410)],
            8820,
        );
        let held = rms(&adsr[2000..4000]) / rms(&play_sine(&[], &[(0, 60, 127, 8820)], 8820)[2000..4000]);
        assert!((held - 0.125).abs() < 0.01, "sustain 0.5 → level 0.125: {held}");
        assert!(rms(&adsr[5000..]) < 1e-4, "released");
    }

    #[test]
    fn sampler_lofi_reduces_bits() {
        let crushed = play_sine(&[("bits", 0.9)], &[(0, 60, 127, 4410)], 4410);
        let mut levels: Vec<i64> = crushed[200..].iter().map(|v| (v * 1e4).round() as i64).collect();
        levels.sort_unstable();
        levels.dedup();
        assert!(levels.len() < 40, "a few levels only: {}", levels.len());
    }

    #[test]
    fn jump_goes_to_the_section_at_the_given_tick() {
        // One section of 8 quarters; jump from beat 2 back to beat 0 once.
        let mut e = click_engine(1000);
        e.jump_at(Some((2 * PPQ, 0)));
        let mut out = vec![0f32; 2 * 2000];
        e.process(&mut out);
        let onsets: Vec<usize> = (0..2000).filter(|&i| out[i * 2] > 0.5).collect();
        assert_eq!(onsets, [0, 500, 1000, 1500], "beats 0, 1, then 0, 1 again");
        assert_eq!(e.pending_jump(), None);
    }

    #[test]
    fn probability_drops_about_that_share_of_notes() {
        let mut e = click_engine(1000);
        let events: Vec<NoteEvent> = (0..400)
            .map(|i| NoteEvent {
                tick: i * 240,
                group: 0,
                sound: 0,
                pitch: 60,
                velocity: 127,
                length: 1,
                probability: 25,
                midi: None,
                cue: None,
                expr: None,
                from: None,
            })
            .collect();
        e.timeline = Timeline { events, length: 400 * 240, ..Default::default() };
        let out = render(&mut e, 120.0);
        let played = (0..out.len() / 2).filter(|&i| out[i * 2] > 0.5).count();
        assert!((70..=130).contains(&played), "about 100 of 400 notes at 25 %: {played}");
    }

    /// PROB LOCK: a locked sound's chances come out the same on every pass (the first
    /// pass's), an unlocked one's are rolled again each pass.
    #[test]
    fn a_locked_sounds_chances_repeat_every_pass() {
        let passes = |locked: bool| {
            let mut e = click_engine(1000);
            e.looping = true;
            let events: Vec<NoteEvent> = (0..32)
                .map(|i| NoteEvent {
                    tick: i * 240,
                    group: 0,
                    sound: 0,
                    pitch: 60,
                    velocity: 127,
                    length: 1,
                    probability: 50,
                    midi: None,
                    cue: None,
                    expr: None,
                    from: None,
                })
                .collect();
            let prob_locked = if locked { vec![(0, 0)] } else { Vec::new() };
            e.timeline = Timeline { events, length: 32 * 240, prob_locked, ..Default::default() };
            let mut out = vec![0f32; 2 * 512];
            while e.fired.len() < 32 * 4 {
                e.process(&mut out);
            }
            let plays: Vec<bool> = e.fired.iter().map(|f| f.4).collect();
            plays.chunks(32).take(4).map(|c| c.to_vec()).collect::<Vec<_>>()
        };
        let locked = passes(true);
        assert!(locked.windows(2).all(|w| w[0] == w[1]), "the same each pass");
        assert!(locked[0].contains(&true) && locked[0].contains(&false));
        let free = passes(false);
        assert!(free.windows(2).any(|w| w[0] != w[1]), "rolled again");
    }

    /// A sound sent to Out 2 plays there, not on the master's output; with the master
    /// on Out 3, what goes through it plays on Out 3 (`process`'s buffer).
    #[test]
    fn sounds_play_on_their_external_outputs() {
        use deckard_model::AudioDest as D;
        let mut p = project();
        p.groups[0].sounds[0].channel.dest = D::Ext(1);
        let mut e = click_engine_for(&p, 48_000);
        e.sequencing = false;
        let play = |e: &mut Engine| {
            e.note_on(0, 0, 60, 127, 960);
            let mut out = vec![0f32; 2 * 4800];
            let mut ext = vec![vec![0f32; 2 * 4800]; OUTPUTS];
            e.process_outputs(&mut out, &mut ext);
            let peak = |x: &[f32]| x.iter().fold(0f32, |m, v| m.max(v.abs()));
            (peak(&out), ext.iter().map(|b| peak(b)).collect::<Vec<f32>>())
        };
        let (master, ext) = play(&mut e);
        assert!(master < 1e-6 && ext[1] > 0.5, "on Out 2 only: {master} {ext:?}");
        assert_eq!(e.master_bus(), Some(0));
        let mut q = project();
        q.master_output = D::Ext(2);
        let e2 = click_engine_for(&q, 48_000);
        assert_eq!(e2.master_bus(), Some(2), "the master on Out 3");
    }

    /// Routing: a sound's Dest. decides which channels' levels it passes through.
    #[test]
    fn routing_gains() {
        use deckard_model::AudioDest as D;
        let mut p = project();
        p.groups[0].level = 0.5; // quieter than unity
        p.groups.push(Group { level: 0.75, pan: 1.0, sounds: vec![Sound { level: 0.75, ..Default::default() }], ..Default::default() });
        let unity = 1.0f32;
        let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4;
        let g05 = deckard_model::level_gain(0.5) as f32;
        assert!(close(sound_gain(&p, 0, 0), (g05, g05)), "through its group");
        p.groups[0].sounds[0].channel.dest = D::Master;
        assert!(close(sound_gain(&p, 0, 0), (unity, unity)), "past its group");
        p.groups[0].sounds[0].channel.dest = D::None;
        assert_eq!(sound_gain(&p, 0, 0), (0.0, 0.0));
        p.groups[0].sounds[0].channel.dest = D::Group(1);
        let g = sound_gain(&p, 0, 0);
        assert!(g.0 < 1e-4 && (g.1 - 2f32.sqrt()).abs() < 1e-3, "through group 2, panned right: {g:?}");
        p.groups[0].sounds[0].channel.dest = D::Ext(1);
        assert!(close(sound_gain(&p, 0, 0), (unity, unity)), "straight onto Out 2");
        assert_eq!(sound_bus(&p, 0, 0), Some(1));
        p.groups[0].sounds[0].channel.dest = D::Ext(OUTPUTS as i64);
        assert_eq!(sound_gain(&p, 0, 0), (0.0, 0.0), "an output there isn't");
        p.groups[0].sounds[0].channel.dest = D::Parent;
        p.groups[0].sounds[0].cue = true;
        assert!(close(sound_gain(&p, 0, 0), (unity, unity)), "the cue bus, on the master's output");
        assert_eq!(sound_bus(&p, 0, 0), None);
        p.cue_output = D::Ext(1);
        assert!(close(sound_gain(&p, 0, 0), (unity, unity)), "the cue bus on Out 2");
        assert_eq!(sound_bus(&p, 0, 0), Some(1));
    }

    /// A sound's chain runs Maschine's own effects: a limiter with 30 dB of drive and a
    /// low ceiling holds the sound down, and bypassed it doesn't.
    #[test]
    fn internal_effects_run_in_the_chain() {
        let mut p = project();
        let slot = |module: &str, active: bool, params: Vec<(String, f64)>| deckard_model::SlotInfo {
            module: module.into(),
            name: module.into(),
            active,
            plugin: None,
            params,
        };
        let lim = vec![("mode".into(), 1.0), ("threshold".into(), -30.0), ("release".into(), 0.0), ("ceiling".into(), 0.0)];
        let peak = |p: &Project| {
            let mut e = click_engine_for(p, 48_000);
            e.sequencing = false;
            e.note_on(0, 0, 60, 127, 960);
            let mut out = vec![0f32; 2 * 4800];
            e.process(&mut out);
            out.iter().fold(0f32, |m, v| m.max(v.abs()))
        };
        p.groups[0].sounds[0].slots = vec![slot("Sampler", true, Vec::new()), slot("Limiter", false, lim.clone())];
        let dry = peak(&p);
        p.groups[0].sounds[0].slots[1].active = true;
        let wet = peak(&p);
        let ceiling = 10f32.powf(-40.05 / 20.0);
        assert!(dry > 0.5, "the click plays: {dry}");
        assert!(wet <= ceiling * 1.01, "held at the limiter's ceiling: {wet} (dry {dry})");
    }

    /// A live note placed `offset` frames into a block starts exactly there, whatever
    /// the block size (live input keeps its timing to the sample).
    #[test]
    fn live_events_land_on_their_sample() {
        for (block, offset) in [(256usize, 0u64), (256, 100), (256, 255), (64, 37), (1024, 900)] {
            let mut e = click_engine(48_000);
            e.sequencing = false;
            let mut out = vec![0f32; block * 2];
            e.process(&mut out);
            e.schedule(offset, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 127, length: PPQ });
            e.process(&mut out);
            let first = out.chunks(2).position(|f| f[0].abs() > 0.1);
            assert_eq!(first, Some(offset as usize), "block {block}, offset {offset}");
        }
    }

    /// What `foresee` predicts from any moment is exactly what the sequencer then
    /// fires (when, which events, whether their probability lets them play): looping,
    /// a queued switch to another scene, a queued jump, uneven blocks.
    #[test]
    fn foresight_matches_the_sequencer() {
        let mut p = project();
        // Two scenes of different lengths and notes with probabilities.
        let notes = |k: i64| -> Vec<Note> {
            (0..12)
                .map(|i| Note {
                    expr: None,
                    sound: 0,
                    position: i * 160 + k,
                    length: 100,
                    pitch: 40 + i + k,
                    velocity: 100,
                    probability: 30 + (i * 7) % 70,
                })
                .collect()
        };
        p.groups[0].patterns = vec![
            Pattern { length: 2 * PPQ, notes: notes(0), ..Default::default() },
            Pattern { length: 3 * PPQ, notes: notes(5), ..Default::default() },
        ];
        p.scenes = vec![Scene { patterns: vec![(0, 0)], ..Default::default() }, Scene { patterns: vec![(0, 1)], ..Default::default() }];
        p.sections =
            vec![Section { scene: Some(0), length: 4 * PPQ, auto: false }, Section { scene: Some(1), length: 6 * PPQ, auto: false }];
        for (mode, setup) in [(Mode::Scene(0), 0), (Mode::Scene(0), 1), (Mode::Arrangement, 2)] {
            let mut e = click_engine_for(&p, 48_000);
            e.jump(&p, mode);
            e.looping = true;
            match setup {
                1 => e.queue(&p, Mode::Scene(1)),
                2 => e.jump_at(Some((3 * PPQ, PPQ))),
                _ => {}
            }
            let sizes = [256usize, 100, 512, 37, 1024, 300];
            let mut k = 0;
            let mut outcomes = (false, false);
            // From several moments, foresee what's next, then play it.
            for _ in 0..12 {
                let (start, skip, span) = (e.time, 700u64, 9_000u64);
                let mut want = Vec::new();
                e.foresee(skip, span, |seen| {
                    if let Seen::Event { t, e: ev, plays, .. } = seen {
                        want.push((start + t, ev.tick, ev.sound, ev.pitch, plays));
                    }
                });
                e.fired.clear();
                let mut done = 0u64;
                while done < skip + span {
                    let n = (sizes[k % sizes.len()] as u64).min(skip + span - done) as usize;
                    k += 1;
                    let mut out = vec![0f32; n * 2];
                    e.process(&mut out);
                    done += n as u64;
                }
                let got: Vec<_> = e.fired.iter().copied().filter(|f| f.0 >= start + skip).collect();
                assert_eq!(got, want, "mode {mode:?}, setup {setup}, from frame {start}");
                outcomes = (outcomes.0 || got.iter().any(|f| f.4), outcomes.1 || got.iter().any(|f| !f.4));
            }
            assert_eq!(outcomes, (true, true), "notes both kept and dropped by their probability");
        }
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
            let v = (t * 440.0 * std::f32::consts::TAU).sin() * 0.1
                + if i < 2400 { (t * 2000.0 * std::f32::consts::TAU).sin() * 0.8 } else { 0.0 };
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        path
    }

    /// A 120 BPM song whose sound 0 is an Audio plug-in playing a 4-beat 94 BPM loop
    /// (`engine`), on in its 8-beat pattern. With the folder holding the loop.
    fn audio_project(engine: deckard_model::AudioEngine) -> (tempfile::TempDir, Project) {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = project();
        let take = deckard_model::Take {
            sample: deckard_model::SampleRef {
                kind: 0,
                path: loop_wav(tmp.path(), 94.0, 4.0).to_string_lossy().into(),
                ..Default::default()
            },
            beats: 4.0,
            play_range: (0, 0),
        };
        let snd = &mut p.groups[0].sounds[0];
        snd.instrument = "AudioModule".into();
        snd.sampler = None;
        snd.audio = Some(deckard_model::AudioLoop { takes: vec![take], engine, pitchbend: 0.5, fade: 0.1222, ..Default::default() });
        p.groups[0].patterns[0] =
            Pattern { length: 8 * PPQ, audio: vec![deckard_model::AudioCue { sound: 0, take: 0 }], ..Default::default() };
        p.sections = vec![Section { scene: Some(0), length: 16 * PPQ, auto: false }];
        (tmp, p)
    }

    /// Where the loop's mark sounds: frames where the level (256-frame windows) rises past
    /// half its loudest.
    fn marks(out: &[f32]) -> Vec<usize> {
        let lv: Vec<f32> = out.chunks(512).map(|c| c.iter().map(|v| v * v).sum::<f32>()).collect();
        let top = lv.iter().cloned().fold(0.0, f32::max);
        (1..lv.len()).filter(|&i| lv[i] > top * 0.5 && lv[i - 1] <= top * 0.5).map(|i| i * 256).collect()
    }

    /// Tuned while it plays (an edit), the loop carries on where it was, transposed.
    #[test]
    fn an_edited_loop_plays_on_with_its_new_settings() {
        let (_tmp, p) = audio_project(deckard_model::AudioEngine::Stretch);
        let mut tuned = p.clone();
        tuned.groups[0].sounds[0].audio.as_mut().unwrap().tune = 12.0;
        let hz = |x: &[f32]| {
            let left: Vec<f32> = x.chunks_exact(2).map(|f| f[0]).collect();
            let n = left.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
            n as f64 * 48_000.0 / left.len() as f64
        };
        let chains = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
        let out = render_with(&p, None, 256, 2.0, |k, e, cache| {
            // Half a second in (between the marks): the tune changes, as an edit.
            if k == 94 {
                let (fresh, _) = Engine::new_after(&tuned, Some((&p, chains.as_slice())), None, 48_000, e.mode(), cache);
                drop(e.adopt(fresh, &tuned));
            }
        });
        // Away from the first beat's mark: before the edit 440 Hz, after it 880 Hz.
        let before = &out[2 * 6_000..2 * 23_000];
        let after = &out[2 * 40_000..2 * 90_000];
        assert!((hz(before) - 440.0).abs() < 20.0, "before: {}", hz(before));
        assert!((hz(after) - 880.0).abs() < 30.0, "after: {}", hz(after));
    }

    /// Started in the middle of its pattern, the loop plays at once, where the song is
    /// (its mark comes at the pattern's next four-beat boundary, not later).
    #[test]
    fn a_loop_started_mid_pattern_plays_from_there() {
        let (_tmp, p) = audio_project(deckard_model::AudioEngine::Stretch);
        let out = render_with(&p, None, 256, 3.0, |k, e, _| {
            if k == 0 {
                // Beat 2 of 8: the mark (on beat 4) two beats (48 000 frames) later.
                e.seek(2 * PPQ);
            }
        });
        let level: f32 = out[..48_000].iter().map(|v| v * v).sum();
        assert!(level > 1.0, "it plays from the start: {level}");
        let m = marks(&out);
        assert!(m.first().is_some_and(|&f| f.abs_diff(48_000) < 1024), "the next mark on beat 4: {m:?}");
    }

    /// The loop plays from the song's start, once every four beats at the song's 120
    /// BPM (not its own 94), with every engine; rendered ahead it sounds the same.
    #[test]
    fn audio_plugin_loops_play_in_time_with_the_song() {
        for engine in [deckard_model::AudioEngine::Stretch, deckard_model::AudioEngine::Repitch, deckard_model::AudioEngine::Formant] {
            let (_tmp, p) = audio_project(engine);
            let out = render_with(&p, None, 256, 7.0, |_, _, _| {});
            let m = marks(&out);
            assert!(m.first().is_some_and(|&f| f < 1024), "{engine:?}: starts at the song's start: {m:?}");
            let gaps: Vec<usize> = m.windows(2).map(|w| w[1] - w[0]).collect();
            assert!(gaps.len() >= 2 && gaps.iter().all(|&g| g.abs_diff(96_000) < 600), "{engine:?}: every 4 beats at 120: {gaps:?}");
            let ahead = render_with(&p, Some(700), 256, 7.0, |_, _, _| {});
            assert_eq!(marks(&ahead), m, "{engine:?}: rendered ahead, the same");
        }
    }

    fn render_eight(ahead: Option<u64>, block: usize, seconds: f64) -> Vec<f32> {
        let (_tmp, p) = eight_project();
        render_with(&p, ahead, block, seconds, |k, e, _| {
            if k == 0 && ahead.is_some() {
                assert_eq!(e.sounds_ahead(), 8, "every sound rendered ahead");
            }
        })
    }

    /// Sounds moving between live and ahead while they play (the focus switching to
    /// their group and away, every few blocks) sound exactly as if they never moved.
    #[test]
    fn moving_between_live_and_ahead_is_seamless() {
        let (_tmp, p) = eight_project();
        let inline = render_with(&p, None, 256, 3.0, |_, _, _| {});
        let mut moves = 0;
        let ahead = render_with(&p, Some(700), 256, 3.0, |k, e, _| {
            if k % 7 == 3 {
                let to = if e.sounds_ahead() > 0 { Some(0) } else { None };
                e.set_focus(to);
                moves += 1;
            }
        });
        assert!(moves > 20);
        let first = inline.iter().zip(&ahead).position(|(a, b)| a != b);
        assert_eq!(
            first,
            None,
            "bit for bit (first difference at frame {:?}, block {:?})",
            first.map(|i| i / 2),
            first.map(|i| i / 2 / 256)
        );
    }

    /// Edits while sounds are rendered ahead: a level change is applied live (exactly
    /// as without rendering ahead); an effect's change reaches the player (a lookahead
    /// later) and then sounds the same.
    #[test]
    fn edits_reach_sounds_rendered_ahead() {
        let (_tmp, p) = eight_project();
        let mut louder = p.clone();
        louder.groups[0].sounds[3].level = 0.3;
        let edit = |to: &Project| {
            let to = to.clone();
            move |k: usize, e: &mut Engine, cache: &mut SampleCache| {
                if k == 100 {
                    e.update(&to, None, cache);
                }
            }
        };
        let inline = render_with(&p, None, 256, 2.0, edit(&louder));
        let ahead = render_with(&p, Some(700), 256, 2.0, edit(&louder));
        assert_eq!(inline.iter().zip(&ahead).fold(0f32, |m, (a, b)| m.max((a - b).abs())), 0.0, "level: bit for bit");
        let mut driven = p.clone();
        let sat = driven.groups[0].sounds[5].slots.iter_mut().find(|x| x.module == "Saturator").unwrap();
        for (k, v) in sat.params.iter_mut() {
            if k.contains("drive") {
                *v = 1.0;
            }
        }
        let inline = render_with(&p, None, 256, 2.0, edit(&driven));
        let ahead = render_with(&p, Some(700), 256, 2.0, edit(&driven));
        let plain = render_with(&p, None, 256, 2.0, |_, _, _| {});
        let at = 100 * 256 * 2;
        let diff = |a: &[f32], b: &[f32]| a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
        assert_eq!(diff(&inline[..at], &ahead[..at]), 0.0, "the same before the edit");
        assert!(diff(&inline[at..], &plain[at..]) > 0.01, "the edit changes the sound");
        // A lookahead (700 frames) and the effect's glide later, it is the same.
        let settled = at + 2 * 48_000 / 4;
        assert!(diff(&inline[settled..], &ahead[settled..]) < 1e-4, "settled: {}", diff(&inline[settled..], &ahead[settled..]));
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

    /// A group with effects is a bus: its sounds' routes end at it, with only their own
    /// faders; the group's fader comes after its effects.
    #[test]
    fn a_group_with_effects_is_a_bus() {
        let (_tmp, mut p) = eight_project();
        p.groups[0].level = 0.5;
        let plain = sound_route(&p, 0, 0);
        assert_eq!(plain.0, None);
        p.groups[0].slots = vec![driven_saturator()];
        let (to, gain) = sound_route(&p, 0, 0);
        assert_eq!(to, Some(GROUP_BUS));
        let s = &p.groups[0].sounds[0];
        assert_eq!(gain, channel_gain(s.level, s.pan));
        assert_eq!(group_out_route(&p, 0), (None, channel_gain(0.5, p.groups[0].pan)));
        p.groups[0].muted = true;
        assert_eq!(sound_route(&p, 0, 0).1, (0.0, 0.0));
    }

    /// A group's effect changes what the group plays (and nothing when it isn't there).
    #[test]
    fn group_effects_play() {
        let (_tmp, p) = eight_project();
        let mut fx = p.clone();
        fx.groups[0].slots = vec![driven_saturator()];
        let plain = render_with(&p, None, 256, 1.0, |_, _, _| {});
        let driven = render_with(&fx, None, 256, 1.0, |_, _, _| {});
        assert!(driven.iter().all(|x| x.is_finite()));
        assert!(max_diff(&plain, &driven) > 0.05, "the group's saturator is heard");
        // Through the bus with its effect bypassed: as without a bus (the group's fader
        // and pan where they were).
        let mut bypassed = fx.clone();
        bypassed.groups[0].slots[0].active = false;
        bypassed.groups[0].level = 0.6;
        bypassed.groups[0].pan = 0.3;
        let mut folded = p.clone();
        folded.groups[0].level = 0.6;
        folded.groups[0].pan = 0.3;
        let (a, b) = (render_with(&bypassed, None, 256, 1.0, |_, _, _| {}), render_with(&folded, None, 256, 1.0, |_, _, _| {}));
        assert!(max_diff(&a, &b) < 1e-4, "bus = folded: {}", max_diff(&a, &b));
        // Its fader comes after its effects.
        let mut down = fx.clone();
        down.groups[0].level = 0.0;
        assert!(render_with(&down, None, 256, 0.5, |_, _, _| {}).iter().all(|x| x.abs() < 1e-6));
        let mut muted = fx.clone();
        muted.groups[0].muted = true;
        assert!(render_with(&muted, None, 256, 0.5, |_, _, _| {}).iter().all(|x| *x == 0.0));
    }

    /// Sounds rendered ahead go through their group's bus as the others do.
    #[test]
    fn rendered_ahead_sounds_go_through_the_group_bus() {
        let (_tmp, mut p) = eight_project();
        p.groups[0].slots = vec![driven_saturator()];
        let inline = render_with(&p, None, 256, 1.0, |_, _, _| {});
        let ahead = render_with(&p, Some(700), 256, 1.0, |_, _, _| {});
        assert!(max_diff(&inline, &ahead) < 1e-5, "{}", max_diff(&inline, &ahead));
    }

    /// An edit elsewhere (a sound's level) leaves a group's and the master's effects as
    /// they were: a delay's echoes carry on through it.
    #[test]
    fn edits_keep_group_and_master_effects() {
        let (_, d) = deckard_model::INTERNAL_EFFECTS.iter().find(|(c, _)| *c == "BeatDelay").unwrap();
        let delay = deckard_model::SlotInfo {
            module: "BeatDelay".into(),
            name: "BeatDelay".into(),
            active: true,
            plugin: None,
            params: d.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        };
        let (_tmp, mut p) = eight_project();
        p.groups[0].slots = vec![delay.clone()];
        p.master_slots = vec![delay];
        let same = p.clone();
        let plain = render_with(&p, None, 256, 1.5, |_, _, _| {});
        let edited = render_with(&p, None, 256, 1.5, move |k, e, cache| {
            if k == 100 {
                e.update(&same, None, cache);
            }
        });
        assert!(max_diff(&plain, &edited) < 1e-6, "{}", max_diff(&plain, &edited));
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

    /// A pattern's points become the parameter's stored values along the timeline:
    /// the knob's value moved by each deviation, clamped to the range.
    #[test]
    fn modulation_points_are_the_parameters_values_on_the_timeline() {
        let (_tmp, p, base) = modulated_project();
        let t = timeline(&p, Mode::Arrangement);
        let lane = t.mods.iter().find(|l| l.group == 0 && l.sound == Some(1)).expect("a lane");
        assert_eq!((lane.slot, lane.field.as_str()), (1, "feedback"));
        assert_eq!(lane.from, 0);
        assert!((lane.points[0].1 - (base - 0.5).clamp(0.0, 1.0)).abs() < 1e-9);
        assert!((lane.points[1].1 - (base + 0.5).clamp(0.0, 1.0)).abs() < 1e-9);
        assert!((lane.value_at(lane.points[0].0 as f64) - lane.points[0].1).abs() < 1e-9);
        let mid = (lane.points[0].0 + lane.points[1].0) as f64 / 2.0;
        assert!((lane.value_at(mid) - (lane.points[0].1 + lane.points[1].1) / 2.0).abs() < 1e-9, "joined by a line");
        assert!((lane.value_at(lane.to as f64 - 1.0) - lane.points[1].1).abs() < 1e-9, "the last holds");
    }

    /// While the pattern plays the effect follows the points; stopped, it has the
    /// knob's own value again. A modulated sound plays on the audio thread.
    #[test]
    fn modulation_moves_an_effects_parameter_while_playing() {
        let (_tmp, p, base) = modulated_project();
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        assert!(e.live_hint[0][1], "rendered live");
        e.looping = true;
        let mut buf = vec![0f32; 256 * 2];
        for _ in 0..20 {
            e.process(&mut buf);
        }
        // On the line between the points (within the 30 ms glide behind it).
        let lane = e.timeline.mods.iter().find(|l| l.sound == Some(1)).unwrap().clone();
        let want = lane.value_at(e.frame as f64 / e.samples_per_tick());
        assert!((feedback_now(&e) - want).abs() < 0.03, "{} vs {want}", feedback_now(&e));
        assert!(feedback_now(&e) < base - 0.3, "well under the knob's {base}");
        e.sequencing = false;
        for _ in 0..20 {
            e.process(&mut buf);
        }
        assert!((feedback_now(&e) - base).abs() < 1e-6, "back to {base}: {}", feedback_now(&e));
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

    /// Pressure moves an effect's parameter from Low to High; let go, it rests at Low;
    /// the route removed, it has the knob's value again.
    #[test]
    fn pressure_moves_an_effects_parameter() {
        use deckard_model::mod_matrix::ModSource;
        let (_tmp, mut p, base) = modulated_project();
        p.groups[0].patterns[0].mods.clear();
        let mut r = route(ModSource::Pressure, 1, &p.groups[0].sounds[1].slots[1].module.clone(), field("feedback"));
        r.low = 0.1;
        p.groups[0].sounds[1].mod_routes = vec![r];
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        press(&mut e, 1.0, 20);
        assert!((feedback_now(&e) - 1.0).abs() < 1e-3, "{}", feedback_now(&e));
        e.note_release(0, 1, 60);
        blocks(&mut e, 20);
        assert!((feedback_now(&e) - 0.1).abs() < 1e-3, "{}", feedback_now(&e));
        let mut plain = p.clone();
        plain.groups[0].sounds[1].mod_routes.clear();
        let (fresh, _) = Engine::new_after(&plain, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &plain));
        blocks(&mut e, 20);
        assert!((feedback_now(&e) - base).abs() < 1e-6, "back to {base}: {}", feedback_now(&e));
    }

    /// Pressure moves the Sampler's cutoff, while notes play; the route gone, the
    /// cutoff is the knob's again.
    #[test]
    fn pressure_moves_the_samplers_cutoff() {
        use deckard_model::mod_matrix::ModSource;
        let (_tmp, mut p) = eight_project();
        assert_eq!(p.groups[0].sounds[1].slots[0].module, "Sampler");
        let knob = p.groups[0].sounds[1].sampler.as_ref().unwrap().get("cutoff");
        let mut r = route(ModSource::Pressure, 0, "Sampler", field("cutoff"));
        (r.low, r.high) = (0.2, 0.8);
        p.groups[0].sounds[1].mod_routes = vec![r];
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        press(&mut e, 0.5, 2);
        let cutoff = |e: &Engine| e.sounds[0][1].as_ref().unwrap().settings_cutoff();
        assert!((cutoff(&e) - 0.5).abs() < 1e-4, "{}", cutoff(&e));
        let mut plain = p.clone();
        plain.groups[0].sounds[1].mod_routes.clear();
        let (fresh, _) = Engine::new_after(&plain, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &plain));
        blocks(&mut e, 1);
        assert!((f64::from(cutoff(&e)) - knob).abs() < 1e-4, "{} vs {knob}", cutoff(&e));
    }

    /// Pressure moves a drum synth's parameter.
    #[test]
    fn pressure_moves_a_drum_synths_parameter() {
        use deckard_model::mod_matrix::ModSource;
        let (_tmp, mut p) = eight_project();
        let snd = &mut p.groups[0].sounds[1];
        snd.sampler = None;
        snd.slots = vec![deckard_model::SlotInfo {
            module: "Kick".into(),
            name: "Kick".into(),
            active: true,
            plugin: None,
            params: vec![("decay".into(), 0.5)],
        }];
        snd.mod_routes = vec![route(ModSource::Pressure, 0, "Kick", field("decay"))];
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        press(&mut e, 0.25, 2);
        let v = e.sounds[0][1].as_ref().unwrap().synth_param("decay").unwrap();
        assert!((v - 0.25).abs() < 1e-6, "{v}");
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

    /// A Square LFO at 1/4: the decay is at High for the first half beat, at Low for
    /// the second.
    #[test]
    fn an_lfo_moves_a_routed_parameter() {
        use deckard_model::lfo::{Lfo, Shape};
        let mut l = Lfo::new(1);
        l.shape = Shape::Square;
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        blocks(&mut e, 1);
        assert!((decay_now(&e) - 1.0).abs() < 1e-6, "{}", decay_now(&e));
        let n = (e.samples_per_tick() * 480.0 / 256.0).ceil() as usize;
        blocks(&mut e, n);
        assert!((480.0..960.0).contains(&e.position()), "{}", e.position());
        assert!(decay_now(&e).abs() < 1e-6, "{}", decay_now(&e));
    }

    /// A Wander LFO moves a routed parameter along its four-sine sum, doesn't repeat on
    /// the next beat, and restarts from cycle 0 when triggered by a note.
    #[test]
    fn a_wander_lfo_moves_a_routed_parameter_and_retriggers() {
        use deckard_model::lfo::{Lfo, Shape};
        let l = Lfo { shape: Shape::Wander, ..Lfo::new(1) };
        let (_tmp, p) = lfo_project(1, vec![l.clone()]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        blocks(&mut e, 1);
        let at_start = decay_now(&e);
        assert!((at_start - f64::from(l.wave_at(0.0))).abs() < 1e-5, "{at_start}");
        let one_beat = (e.samples_per_tick() * 960.0 / 256.0).round() as usize;
        blocks(&mut e, one_beat - 1);
        let c1 = e.lfo_cycles(&e.lfos[0]);
        blocks(&mut e, 1);
        assert!((decay_now(&e) - f64::from(l.wave_at(c1))).abs() < 1e-5);
        assert!((decay_now(&e) - at_start).abs() > 1e-2, "a cycle later is a new point on the curve");

        let trig = Lfo { shape: Shape::Wander, trigger: Some((0, 1)), fade: 2.0, ..Lfo::new(1) };
        let (_tmp, p) = lfo_project(1, vec![trig.clone()]);
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        assert_eq!(e.lfo_value(1), Some(0.0), "0 before the trigger's first note");
        e.note_on(0, 1, 60, 100, 240);
        assert_eq!(e.lfo_cycles(&e.lfos[0]), 0.0, "restarted at the note-on");
        blocks(&mut e, one_beat);
        let c = e.lfo_cycles(&e.lfos[0]);
        let v = e.lfo_value(1).unwrap();
        assert!((v - trig.output(c, Some(c))).abs() < 1e-4, "{v} vs {}", trig.output(c, Some(c)));
        e.note_on(0, 1, 60, 100, 240);
        assert_eq!(e.lfo_cycles(&e.lfos[0]), 0.0, "retriggered back to cycle 0");
    }

    /// Each route's travel is published for its knob (by group, sound and its place in
    /// the sound's routes, read only by the same route): it follows the LFO; a route
    /// gone, or the engine let go, publishes none.
    #[test]
    fn route_travels_are_published() {
        use deckard_model::lfo::{Lfo, Shape};
        let mut l = Lfo::new(1);
        l.shape = Shape::Square;
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        let board = std::sync::Arc::new(meters::RouteTravels::default());
        e.set_route_travels(&board);
        let key = meters::route_key(&p.groups[0].sounds[1].mod_routes[0]);
        assert_eq!(board.get(0, 1, 0, key), None, "nothing yet");
        blocks(&mut e, 1);
        assert_eq!(board.get(0, 1, 0, key), Some(1.0), "High");
        // Another route read at its place (the document's list changed first): none.
        let other = route(deckard_model::mod_matrix::ModSource::Lfo(1), 0, "Kick", field("tone"));
        assert_eq!(board.get(0, 1, 0, meters::route_key(&other)), None, "not the engine's route");
        let n = (e.samples_per_tick() * 480.0 / 256.0).ceil() as usize;
        blocks(&mut e, n);
        assert_eq!(board.get(0, 1, 0, key), Some(0.0), "Low");
        // The route removed: none.
        let mut q = p.clone();
        q.groups[0].sounds[1].mod_routes.clear();
        let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &q));
        assert_eq!(board.get(0, 1, 0, key), None, "route gone");
        blocks(&mut e, 1);
        assert_eq!(board.get(0, 1, 0, key), None);
        // A muted LFO: none; the engine let go: none.
        let mut m = p.clone();
        m.lfos[0].muted = true;
        let (fresh, _) = Engine::new_after(&p, Some((&q, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &p));
        blocks(&mut e, 1);
        assert!(board.get(0, 1, 0, key).is_some(), "back");
        let (fresh, _) = Engine::new_after(&m, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &m));
        blocks(&mut e, 1);
        assert_eq!(board.get(0, 1, 0, key), None, "muted");
        drop(e.adopt(Engine::new_after(&p, Some((&m, &[])), None, 48_000, Mode::Arrangement, &mut cache).0, &p));
        blocks(&mut e, 1);
        e.release_routes();
        assert_eq!(board.get(0, 1, 0, key), None, "released");
    }

    /// A route from an LFO that isn't there leaves the knob at its own value.
    #[test]
    fn a_missing_lfo_leaves_the_knob_alone() {
        use deckard_model::lfo::Lfo;
        let (_tmp, p) = lfo_project(9, vec![Lfo::new(1)]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        assert_eq!(e.lfo_value(9), None);
        blocks(&mut e, 20);
        assert!((decay_now(&e) - 0.5).abs() < 1e-6, "{}", decay_now(&e));
    }

    /// A muted LFO leaves the knob at its own value; unmuted, it moves it again on its
    /// clock. Its trigger's notes still count while it's muted.
    #[test]
    fn a_muted_lfo_leaves_the_knob_alone() {
        use deckard_model::lfo::{Lfo, Shape};
        let l = Lfo { shape: Shape::Square, muted: true, ..Lfo::new(1) };
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        assert_eq!(e.lfo_value(1), None);
        blocks(&mut e, 1);
        assert!((decay_now(&e) - 0.5).abs() < 1e-6, "{}", decay_now(&e));
        let mut q = p.clone();
        q.lfos[0].muted = false;
        let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &q));
        blocks(&mut e, 1);
        assert!((decay_now(&e) - 1.0).abs() < 1e-6, "{}", decay_now(&e));
        let t = Lfo { trigger: Some((0, 1)), muted: true, ..Lfo::new(1) };
        let (_tmp, p) = lfo_project(1, vec![t]);
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        e.note_on(0, 1, 60, 100, 240);
        assert_eq!(e.lfos[0].trig_at.map(|t| t.1), Some(e.time), "a muted LFO's trigger still counts");
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

    /// An engine with Group 0's Sound 0 sent out to MIDI, so what it plays can be read, with
    /// [`OctaveUp`] on the group.
    fn midi_input_engine(later: f64) -> (Engine, std::sync::mpsc::Receiver<midi::MidiOut>, NotesIn, Arc<Mutex<bool>>) {
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        let (tx, rx) = std::sync::mpsc::sync_channel(1024);
        e.midi_tx = Some(tx);
        e.midi_routes = vec![vec![Some(midi::OutRoute::Port { port: "Out".into(), channel: 0, transpose: 0 })]];
        let (seen, stopped) = (NotesIn::default(), Arc::new(Mutex::new(false)));
        let plugin = OctaveUp { seen: seen.clone(), stopped: stopped.clone(), later, held: None };
        assert!(e.set_midi_input(0, Some(Box::new(plugin))).is_none());
        (e, rx, seen, stopped)
    }

    /// The (status, note) of what went out, and whether each is a note-on.
    fn midi_notes(rx: &std::sync::mpsc::Receiver<midi::MidiOut>) -> Vec<(bool, u8)> {
        rx.try_iter()
            .filter_map(|m| match m.bytes()[0] & 0xf0 {
                0x90 => Some((true, m.bytes()[1])),
                0x80 => Some((false, m.bytes()[1])),
                _ => None,
            })
            .collect()
    }

    /// A pattern's notes go in with their lengths; only what the plug-in plays is heard.
    #[test]
    fn a_midi_input_plugin_takes_a_patterns_notes() {
        let (mut e, rx, seen, _) = midi_input_engine(0.0);
        blocks(&mut e, 1);
        assert_eq!(seen.lock().unwrap()[0], midi_input::NoteIn::On { sound: 0, pitch: 60, velocity: 127, length: Some(240) });
        assert_eq!(midi_notes(&rx), [(true, 72)], "an octave up, and not as written");
    }

    /// A key is recorded as played, before the plug-in, and heard as it plays it.
    #[test]
    fn a_take_holds_what_was_played_before_a_midi_input_plugin() {
        let (mut e, rx, seen, _) = midi_input_engine(0.0);
        e.set_recording(true);
        blocks(&mut e, 1);
        seen.lock().unwrap().clear();
        let _ = midi_notes(&rx);
        e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 50, velocity: 90, length: HELD });
        blocks(&mut e, 1);
        assert_eq!(seen.lock().unwrap()[0], midi_input::NoteIn::On { sound: 0, pitch: 50, velocity: 90, length: None });
        assert_eq!(midi_notes(&rx), [(true, 62)]);
        e.schedule(0, Live::NoteRelease { group: 0, sound: 0, pitch: 50 });
        blocks(&mut e, 1);
        assert_eq!(seen.lock().unwrap()[1], midi_input::NoteIn::Off { sound: 0, pitch: 50 });
        assert_eq!(midi_notes(&rx), [(false, 62)]);
        let rec = e.take_recorded();
        assert_eq!(rec.iter().map(|r| r.pitch).collect::<Vec<_>>(), [50, 50], "{rec:?}");
    }

    /// What a plug-in plays later plays then, not before.
    #[test]
    fn a_midi_input_plugin_plays_at_the_time_it_says() {
        let (mut e, rx, _, _) = midi_input_engine(120.0);
        let frames_late = 120.0 * e.samples_per_tick();
        let early = (frames_late / 256.0).floor() as usize - 1;
        blocks(&mut e, early);
        assert_eq!(midi_notes(&rx), [], "120 ticks not gone yet");
        blocks(&mut e, 3);
        assert_eq!(midi_notes(&rx), [(true, 72)]);
    }

    /// Stopping drops what it was still to play and tells it.
    #[test]
    fn stopping_ends_what_a_midi_input_plugin_was_to_play() {
        let (mut e, rx, _, stopped) = midi_input_engine(480.0);
        blocks(&mut e, 2);
        e.stop();
        assert!(*stopped.lock().unwrap());
        let n = (960.0 * e.samples_per_tick() / 256.0).ceil() as usize;
        blocks(&mut e, n);
        assert_eq!(midi_notes(&rx), [], "nothing after the stop");
    }

    /// Note Echo's echoes come out at their times, and stopping ends those still to come.
    #[test]
    fn note_echo_plays_later_and_stops_with_the_song() {
        let (mut e, rx, _, _) = midi_input_engine(0.0);
        blocks(&mut e, 1);
        e.set_midi_input(0, Some(Box::new(note_echo::NoteEcho::new())));
        let _ = midi_notes(&rx);

        e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 50, velocity: 90, length: 60 });
        blocks(&mut e, 1);
        assert_eq!(midi_notes(&rx), [(true, 50)], "the note itself, at once");

        // At its defaults, the first echo is 360 ticks on.
        let block = 256.0 / e.samples_per_tick();
        blocks(&mut e, (340.0 / block).floor() as usize);
        assert!(!midi_notes(&rx).contains(&(true, 50)), "not yet");
        blocks(&mut e, (40.0 / block).ceil() as usize);
        assert!(midi_notes(&rx).contains(&(true, 50)), "the first echo");

        e.stop();
        let _ = midi_notes(&rx);
        blocks(&mut e, (1200.0 / block).ceil() as usize);
        assert!(!midi_notes(&rx).contains(&(true, 50)), "the rest are gone");
    }

    /// A parameter reaches the plug-in, and takes effect on the next note.
    #[test]
    fn a_midi_input_plugin_hears_its_parameters() {
        struct Transpose(i64);
        impl midi_input::MidiInput for Transpose {
            fn note(&mut self, note: midi_input::NoteIn, clock: midi_input::Clock, out: &mut midi_input::Outbox) {
                match note {
                    midi_input::NoteIn::On { sound, pitch, velocity, length } => {
                        out.on(clock.now, sound, pitch + self.0, velocity, length);
                    }
                    midi_input::NoteIn::Off { sound, pitch } => {
                        out.off(clock.now, sound, pitch + self.0);
                    }
                }
            }

            fn set_param(&mut self, id: u32, value: f64) {
                if id == 1 {
                    self.0 = value as i64;
                }
            }
        }

        let (mut e, rx, _, _) = midi_input_engine(0.0);
        blocks(&mut e, 1);
        e.set_midi_input(0, Some(Box::new(Transpose(0))));
        let _ = midi_notes(&rx);

        e.set_midi_input_param(0, 1, 5.0);
        // Nothing there to hear it, and no such group: nothing happens.
        e.set_midi_input_param(1, 1, 7.0);
        e.set_midi_input_param(midi_input::SLOTS, 1, 7.0);
        e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 50, velocity: 90, length: HELD });
        blocks(&mut e, 1);
        assert_eq!(midi_notes(&rx), [(true, 55)]);
    }

    /// Taken out, a plug-in ends what it holds, and notes go straight to the sound again.
    #[test]
    fn taking_a_midi_input_plugin_out_ends_its_notes() {
        let (mut e, rx, seen, _) = midi_input_engine(0.0);
        blocks(&mut e, 1);
        e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 50, velocity: 90, length: HELD });
        blocks(&mut e, 1);
        let _ = midi_notes(&rx);
        assert!(e.set_midi_input(0, None).is_some(), "it comes back, to be freed");
        assert_eq!(midi_notes(&rx), [(false, 62)]);
        let before = seen.lock().unwrap().len();
        e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 55, velocity: 90, length: HELD });
        blocks(&mut e, 1);
        assert_eq!(seen.lock().unwrap().len(), before);
        assert_eq!(midi_notes(&rx), [(true, 55)]);
    }

    /// A muted sound's notes don't go in, and a slot past the last refuses.
    #[test]
    fn a_midi_input_plugin_skips_muted_notes_and_slots_end() {
        let (mut e, _rx, seen, _) = midi_input_engine(0.0);
        e.sound_muted = vec![vec![true]];
        blocks(&mut e, 1);
        assert!(seen.lock().unwrap().is_empty());
        let plugin = OctaveUp { seen: NotesIn::default(), stopped: Default::default(), later: 0.0, held: None };
        assert!(e.set_midi_input(midi_input::SLOTS, Some(Box::new(plugin))).is_some());
    }

    /// Sounds rendered ahead have their pattern's notes sent in advance. With a plug-in on
    /// the group they aren't: a plug-in that plays nothing silences them, and one that plays
    /// its notes is heard.
    #[test]
    fn a_midi_input_plugin_takes_notes_from_sounds_rendered_ahead() {
        struct Silent;
        impl midi_input::MidiInput for Silent {
            fn note(&mut self, _: midi_input::NoteIn, _: midi_input::Clock, _: &mut midi_input::Outbox) {}
        }
        let (_tmp, p) = eight_project();
        let loudest = |plugin: Option<Box<dyn midi_input::MidiInput>>| {
            let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default());
            e.looping = true;
            assert!(e.set_midi_input(0, plugin).is_none());
            e.render_ahead(700, 2, 256, true);
            let mut buf = vec![0f32; 512];
            let mut most = 0f32;
            for _ in 0..188 {
                e.process(&mut buf);
                most = buf.iter().fold(most, |a, s| a.max(s.abs()));
            }
            assert_eq!(e.ahead_underruns(), 0);
            most
        };
        assert!(loudest(None) > 0.01, "the project plays");
        assert_eq!(loudest(Some(Box::new(Silent))), 0.0, "nothing got past it");
        let plugin = OctaveUp { seen: NotesIn::default(), stopped: Default::default(), later: 0.0, held: None };
        assert!(loudest(Some(Box::new(plugin))) > 0.01, "what it plays reaches them");
    }

    /// A plug-in that holds what comes in until the notes are settled, and says what it
    /// was called with.
    struct Settling {
        group: usize,
        log: Arc<Mutex<Vec<(&'static str, usize, f64)>>>,
        held: Vec<i64>,
    }

    impl midi_input::MidiInput for Settling {
        fn note(&mut self, note: midi_input::NoteIn, clock: midi_input::Clock, _: &mut midi_input::Outbox) {
            self.log.lock().unwrap().push(("note", self.group, clock.seconds));
            if let midi_input::NoteIn::On { pitch, .. } = note {
                self.held.push(pitch);
            }
        }

        fn settle(&mut self, clock: midi_input::Clock, out: &mut midi_input::Outbox) {
            self.log.lock().unwrap().push(("settle", self.group, clock.seconds));
            for pitch in self.held.drain(..) {
                out.on(clock.now, 0, pitch, 100, Some(120));
            }
        }
    }

    /// Notes due at one instant all go in before any plug-in settles, in whatever order
    /// the groups came; each plug-in settles once, and a later instant is another.
    #[test]
    fn midi_input_plugins_settle_once_every_note_is_in() {
        let (mut e, rx, _, _) = midi_input_engine(0.0);
        let log = Arc::new(Mutex::new(Vec::new()));
        for group in [0, 1] {
            let plugin = Settling { group, log: log.clone(), held: Vec::with_capacity(8) };
            e.set_midi_input(group, Some(Box::new(plugin)));
        }
        e.next = e.timeline.events.len();
        blocks(&mut e, 4);
        assert!(log.lock().unwrap().is_empty(), "no notes, nothing to settle");
        let _ = midi_notes(&rx);
        // Group 1's first: the later group's chord has to be in before group 0 settles.
        for (group, pitch) in [(1, 50), (0, 60), (0, 64)] {
            e.schedule(0, Live::NoteOn { group, sound: 0, pitch, velocity: 100, length: HELD });
        }
        blocks(&mut e, 1);
        let seen: Vec<(&str, usize)> = log.lock().unwrap().iter().map(|&(what, g, _)| (what, g)).collect();
        assert_eq!(seen, [("note", 1), ("note", 0), ("note", 0), ("settle", 0), ("settle", 1)]);
        assert_eq!(midi_notes(&rx), [(true, 60), (true, 64)], "what group 0 played, together, once settled");
        let now = log.lock().unwrap()[0].2;
        assert!((now - 4.0 * 256.0 / 48_000.0).abs() < 1e-9, "seconds run with the clock: {now}");
        log.lock().unwrap().clear();
        blocks(&mut e, 2);
        assert!(log.lock().unwrap().is_empty(), "settled once");
    }

    /// An engine that replaces this one has its plug-ins, told the song stopped.
    #[test]
    fn a_new_engine_keeps_the_midi_input_plugins() {
        let (mut old, _rx, seen, stopped) = midi_input_engine(0.0);
        let mut cache = SampleCache::default();
        let (mut new, _) = Engine::new(&project(), None, 48_000, Mode::Arrangement, &mut cache);
        new.take_midi_inputs(&mut old);
        assert!(*stopped.lock().unwrap());
        blocks(&mut new, 1);
        assert!(!seen.lock().unwrap().is_empty());
    }

    /// A CC route from Square LFO 1 (high 0.5) to CC 74 on channel 3 of "Synth", and what
    /// the engine sends.
    fn cc_engine(lfo: deckard_model::lfo::Lfo, port: &str) -> (tempfile::TempDir, Engine, std::sync::mpsc::Receiver<midi::MidiOut>) {
        use deckard_model::mod_matrix::{CcRoute, ModSource};
        let (tmp, mut p) = lfo_project(1, vec![lfo]);
        p.cc_routes = vec![CcRoute { high: 0.5, ..CcRoute::new(ModSource::Lfo(1), port, 3, 74) }];
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        e.midi_tx = Some(tx);
        (tmp, e, rx)
    }

    fn ccs(rx: &std::sync::mpsc::Receiver<midi::MidiOut>) -> Vec<(String, Vec<u8>)> {
        rx.try_iter().map(|m| (m.port.to_string(), m.bytes().to_vec())).collect()
    }

    /// An LFO sends its CC at 127 × its travel, only when the value changes: a Square at
    /// 1/4 sends 64 (half of the way at High 0.5), then 0 half a beat later.
    #[test]
    fn an_lfo_sends_a_cc() {
        use deckard_model::lfo::{Lfo, Shape};
        let (_tmp, mut e, rx) = cc_engine(Lfo { shape: Shape::Square, ..Lfo::new(1) }, "Synth");
        blocks(&mut e, 1);
        assert_eq!(ccs(&rx), vec![("Synth".to_string(), vec![0xb2, 74, 64])]);
        blocks(&mut e, 4);
        assert!(ccs(&rx).is_empty(), "the same value isn't sent again");
        let n = (e.samples_per_tick() * 480.0 / 256.0).ceil() as usize;
        blocks(&mut e, n);
        assert_eq!(ccs(&rx), vec![("Synth".to_string(), vec![0xb2, 74, 0])]);
    }

    /// An edit (a new engine adopted) doesn't send the same value again.
    #[test]
    fn an_edit_keeps_what_a_cc_sent() {
        use deckard_model::lfo::{Lfo, Shape};
        let (_tmp, mut e, rx) = cc_engine(Lfo { shape: Shape::Square, ..Lfo::new(1) }, "Synth");
        blocks(&mut e, 1);
        assert_eq!(ccs(&rx).len(), 1);
        let (_t, p) = lfo_project(1, vec![Lfo { shape: Shape::Square, ..Lfo::new(1) }]);
        let mut q = p.clone();
        q.cc_routes = vec![deckard_model::mod_matrix::CcRoute {
            high: 0.5,
            ..deckard_model::mod_matrix::CcRoute::new(deckard_model::mod_matrix::ModSource::Lfo(1), "Synth", 3, 74)
        }];
        let mut cache = SampleCache::default();
        let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &q));
        blocks(&mut e, 1);
        assert!(ccs(&rx).is_empty());
    }

    /// Depth turned back to 0 settles the CC once on its constant (it doesn't freeze at
    /// the last value sent).
    #[test]
    fn depth_back_to_0_settles_the_cc() {
        use deckard_model::lfo::{Lfo, Shape};
        let (_tmp, mut e, rx) = cc_engine(Lfo { shape: Shape::Square, ..Lfo::new(1) }, "Synth");
        blocks(&mut e, 1);
        assert_eq!(ccs(&rx), vec![("Synth".to_string(), vec![0xb2, 74, 64])]);
        e.cc_routes[0].route.high = 0.0;
        blocks(&mut e, 1);
        assert_eq!(ccs(&rx), vec![("Synth".to_string(), vec![0xb2, 74, 0])]);
        blocks(&mut e, 200);
        assert!(ccs(&rx).is_empty(), "once");
    }

    /// Frames in a CC's settle (0.25 s at 48 kHz) as blocks of 256, and a few more.
    const SETTLE_BLOCKS: usize = 48_000 / 4 / 256 + 2;

    /// The engine `e` adopting `q` (from `p`): an edit.
    fn edit_to(e: &mut Engine, p: &Project, q: &Project) {
        let mut cache = SampleCache::default();
        let (fresh, _) = Engine::new_after(q, Some((p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, q));
    }

    /// A live route given another port, channel or CC (an edit) sends its value there,
    /// once it has stayed there 0.25 s.
    #[test]
    fn a_moved_cc_sends_to_its_new_place() {
        use deckard_model::lfo::{Lfo, Shape};
        use deckard_model::mod_matrix::{CcRoute, ModSource};
        let l = Lfo { shape: Shape::Square, ..Lfo::new(1) };
        let (_tmp, mut e, rx) = cc_engine(l.clone(), "Synth");
        blocks(&mut e, 1);
        assert_eq!(ccs(&rx).len(), 1);
        let (_t, p) = lfo_project(1, vec![l]);
        let route = |port: &str, ch, cc| CcRoute { high: 0.5, ..CcRoute::new(ModSource::Lfo(1), port, ch, cc) };
        for (port, ch, cc, want) in [("Other", 3, 74, vec![0xb2, 74]), ("Other", 5, 74, vec![0xb4, 74]), ("Other", 5, 1, vec![0xb4, 1])] {
            let mut q = p.clone();
            q.cc_routes = vec![route(port, ch, cc)];
            edit_to(&mut e, &p, &q);
            blocks(&mut e, SETTLE_BLOCKS);
            let got = ccs(&rx);
            assert_eq!(got.len(), 1, "{port} {ch} {cc}: {got:?}");
            assert_eq!((got[0].0.as_str(), &got[0].1[..2]), (port, &want[..]), "{port} {ch} {cc}");
        }
    }

    /// A CC turned 1, 2, 3 in quick edits sends nothing to 1 or 2: only to 3, 0.25 s
    /// after the last change. A route left as it was keeps sending meanwhile.
    #[test]
    fn a_cc_settles_before_it_sends() {
        use deckard_model::lfo::{Lfo, Shape};
        use deckard_model::mod_matrix::{CcRoute, ModSource};
        let l = Lfo { shape: Shape::Square, ..Lfo::new(1) };
        let (_tmp, mut e, rx) = cc_engine(l.clone(), "Synth");
        let (_t, p) = lfo_project(1, vec![l]);
        let kept = CcRoute { high: 0.5, ..CcRoute::new(ModSource::Lfo(1), "Synth", 3, 74) };
        let turned = |cc| CcRoute { high: 0.5, ..CcRoute::new(ModSource::Lfo(1), "Synth", 1, cc) };
        let mut q = p.clone();
        q.cc_routes = vec![kept.clone()];
        edit_to(&mut e, &p, &q);
        blocks(&mut e, 1);
        assert_eq!(ccs(&rx), vec![("Synth".to_string(), vec![0xb2, 74, 64])], "an unchanged route sends at once");
        let mut was = q;
        for cc in [1, 2, 3] {
            let mut q = p.clone();
            q.cc_routes = vec![kept.clone(), turned(cc)];
            edit_to(&mut e, &was, &q);
            was = q;
            blocks(&mut e, 10);
            assert!(ccs(&rx).iter().all(|(_, b)| b[0] == 0xb2), "nothing to CC {cc} yet");
        }
        // Half a beat on (120 BPM): the square falls, the kept route sends 0.
        let n = (e.samples_per_tick() * 480.0 / 256.0).ceil() as usize;
        blocks(&mut e, n);
        let got = ccs(&rx);
        assert!(got.contains(&("Synth".to_string(), vec![0xb2, 74, 0])), "the kept route keeps sending: {got:?}");
        let to_new: Vec<_> = got.iter().filter(|(_, b)| b[0] == 0xb0).collect();
        assert_eq!(to_new.len(), 1, "{got:?}");
        assert_eq!(to_new[0].1[1], 3, "only to the last CC");
    }

    /// A muted LFO sends nothing more (no reset value); a CC to a port that isn't open is
    /// sent like any (the port's thread drops it) and nothing breaks.
    #[test]
    fn a_muted_lfo_sends_no_cc() {
        use deckard_model::lfo::{Lfo, Shape};
        let (_tmp, mut e, rx) = cc_engine(Lfo { shape: Shape::Square, muted: true, ..Lfo::new(1) }, "Synth");
        blocks(&mut e, 200);
        assert!(ccs(&rx).is_empty());
        // Depth 0 (a new route): nothing either.
        let (_tmp, mut e, rx) = cc_engine(Lfo { shape: Shape::Square, ..Lfo::new(1) }, "Synth");
        e.cc_routes[0].route.high = 0.0;
        blocks(&mut e, 200);
        assert!(ccs(&rx).is_empty(), "Depth 0");
        let (_tmp, mut e, _rx) = cc_engine(Lfo::new(1), "Nowhere");
        e.midi_tx = None;
        blocks(&mut e, 20);
    }

    /// A triggered LFO is silent until its sound's note, then fades in over its fade.
    #[test]
    fn a_note_restarts_a_triggered_lfo() {
        use deckard_model::lfo::{Lfo, Shape};
        let mut l = Lfo::new(1);
        // (A whole note's cycle: the wave is 1 for its first two beats.)
        (l.shape, l.rate, l.trigger, l.fade) = (Shape::Square, 6, Some((0, 1)), 1.0);
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        blocks(&mut e, 4);
        assert_eq!(e.lfo_value(1), Some(0.0));
        e.note_on(0, 1, 60, 100, 240);
        blocks(&mut e, 47);
        let since = (e.lfo_clock.ever - e.lfos[0].trig_at.unwrap().0) / PPQ as f64;
        assert!((since - 0.5).abs() < 0.01, "{since}");
        let v = e.lfo_value(1).unwrap();
        assert!((f64::from(v) - since).abs() < 1e-4, "{v} vs {since}");
    }

    /// A triggered LFO's wave starts at its phase point at each note, wherever in the
    /// bar the note falls.
    #[test]
    fn a_triggered_lfo_starts_at_its_phase_point() {
        use deckard_model::lfo::{Lfo, Shape};
        let mut l = Lfo::new(1);
        (l.shape, l.trigger, l.fade) = (Shape::Square, Some((0, 1)), 0.0);
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        // 0.62 of a beat in: on the song's clock the wave would be 0.
        blocks(&mut e, 58);
        e.note_on(0, 1, 60, 100, 240);
        assert_eq!(e.lfo_value(1), Some(1.0));
        // A quarter beat on: still the cycle's first half.
        blocks(&mut e, 23);
        assert_eq!(e.lfo_value(1), Some(1.0));
        assert!((decay_now(&e) - 1.0).abs() < 1e-6, "{}", decay_now(&e));
    }

    /// An LFO driving a plug-in's parameter is removed (the route stays): the plug-in
    /// plays its knob again.
    #[test]
    fn a_removed_lfo_lets_go_of_a_plugins_parameter() {
        use deckard_model::lfo::Lfo;
        use deckard_model::mod_matrix::{ModParam, ModSource};
        let class = deckard_plugins::native::CLASSES[0];
        let id = class.params.iter().find(|x| x.steps == 0).unwrap().id;
        let pr = deckard_model::PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
        let (_tmp, mut p) = eight_project();
        let snd = &mut p.groups[0].sounds[1];
        snd.sampler = None;
        snd.plugin = Some(pr.clone());
        snd.slots = vec![deckard_model::SlotInfo {
            module: "PluginHost".into(),
            name: class.name.into(),
            active: true,
            plugin: Some(pr.clone()),
            params: vec![],
        }];
        snd.mod_routes = vec![route(ModSource::Lfo(1), 0, &pr.id, ModParam::Plugin(id))];
        p.lfos = vec![Lfo::new(1)];
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        blocks(&mut e, 2);
        let plugin = e.instruments[0][1].as_ref().unwrap().plugin.clone();
        assert!(plugin.lock().unwrap().modulated(id).is_some(), "the LFO drives it");
        let mut gone = p.clone();
        gone.lfos.clear();
        let (fresh, _) = Engine::new_after(&gone, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &gone));
        blocks(&mut e, 1);
        assert_eq!(plugin.lock().unwrap().modulated(id), None, "back to its knob");
        // Busy when the span comes: let go at a later one.
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        blocks(&mut e, 2);
        let (fresh, _) = Engine::new_after(&gone, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &gone));
        {
            let _busy = plugin.lock().unwrap();
            e.apply_routes();
        }
        assert!(plugin.lock().unwrap().modulated(id).is_some(), "still held");
        blocks(&mut e, 1);
        assert_eq!(plugin.lock().unwrap().modulated(id), None, "let go once free");
        // Muted rather than removed: let go the same way.
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        blocks(&mut e, 2);
        assert!(plugin.lock().unwrap().modulated(id).is_some());
        let mut muted = p.clone();
        muted.lfos[0].muted = true;
        let (fresh, _) = Engine::new_after(&muted, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &muted));
        blocks(&mut e, 1);
        assert_eq!(plugin.lock().unwrap().modulated(id), None, "muted: back to its knob");
    }

    /// Muted notes, notes of sounds a solo left out, and chokes don't trigger.
    #[test]
    fn muted_soloed_and_choked_notes_dont_trigger() {
        use deckard_model::lfo::Lfo;
        let mut l = Lfo::new(1);
        l.trigger = Some((0, 1));
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        e.sound_muted[0][1] = true;
        e.note_on(0, 1, 60, 100, 240);
        assert_eq!(e.lfos[0].trig_at, None, "muted");
        e.sound_muted[0][1] = false;
        e.retain_sounds(|g, s| (g, s) != (0, 1));
        e.note_on(0, 1, 60, 100, 240);
        assert_eq!(e.lfos[0].trig_at, None, "soloed out");
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        let tps = 1.0 / e.samples_per_tick();
        let choke = NoteEvent {
            tick: 0,
            group: 0,
            sound: 1,
            pitch: 0,
            velocity: 0,
            length: 0,
            probability: 100,
            midi: Some(pads::CHOKE),
            cue: None,
            expr: None,
            from: None,
        };
        e.trigger(choke, tps);
        assert_eq!(e.lfos[0].trig_at, None, "choked");
        blocks(&mut e, 1);
        e.note_on(0, 1, 60, 100, 240);
        assert_eq!(e.lfos[0].trig_at.map(|t| t.1), Some(e.time), "a note that sounds");
    }

    /// An envelope on LFO 1 that sound 1's notes play: `a`, `d`, `r` seconds, sustain 0.5.
    fn env_engine(a: f32, d: f32, r: f32, retrigger: bool) -> (tempfile::TempDir, Project, Engine, SampleCache) {
        use deckard_model::lfo::Lfo;
        let l = Lfo { env: true, trigger: Some((0, 1)), attack: a, decay: d, sustain: 0.5, release: r, retrigger, ..Lfo::new(1) };
        let (tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        (tmp, p, e, cache)
    }

    fn env_now(e: &Engine) -> f32 {
        e.lfo_value(1).unwrap()
    }

    /// Silent before a note; a held pad rises, holds at the sustain, and falls to 0 when
    /// let go; its route follows.
    #[test]
    fn an_envelope_follows_a_held_pad() {
        let (_tmp, _p, mut e, _c) = env_engine(0.01, 0.01, 0.05, true);
        blocks(&mut e, 2);
        assert_eq!(e.lfo_value(1), Some(0.0), "silent before a note");
        e.note_hold(0, 1, 60, 100);
        blocks(&mut e, 1);
        let v = env_now(&e);
        assert!((v - 256.0 / 480.0).abs() < 1e-3, "rising: {v}");
        blocks(&mut e, 10);
        assert!((env_now(&e) - 0.5).abs() < 1e-6, "sustain: {}", env_now(&e));
        assert!((decay_now(&e) - 0.5).abs() < 1e-6, "the route follows: {}", decay_now(&e));
        e.note_release(0, 1, 60);
        blocks(&mut e, 1);
        let v = env_now(&e);
        assert!(v < 0.5 && v > 0.0, "releasing: {v}");
        blocks(&mut e, 15);
        assert_eq!(env_now(&e), 0.0);
    }

    /// Two notes held, one let go: still at the sustain; the last let go: released.
    #[test]
    fn overlapping_notes_release_after_the_last() {
        let (_tmp, _p, mut e, _c) = env_engine(0.01, 0.01, 0.01, true);
        e.note_hold(0, 1, 60, 100);
        blocks(&mut e, 2);
        e.note_hold(0, 1, 62, 100);
        blocks(&mut e, 10);
        e.note_release(0, 1, 60);
        blocks(&mut e, 3);
        assert!((env_now(&e) - 0.5).abs() < 1e-6, "one still held: {}", env_now(&e));
        e.note_release(0, 1, 62);
        blocks(&mut e, 1);
        assert!(env_now(&e) < 0.5);
        blocks(&mut e, 5);
        assert_eq!(env_now(&e), 0.0);
    }

    /// Retrigger off: a second note while one is held changes nothing, a note in the
    /// release starts it again (from where it was); on: every note restarts the attack.
    #[test]
    fn legato_off_ignores_a_second_note_and_retrigger_restarts_it() {
        for retrigger in [false, true] {
            let (_tmp, _p, mut e, _c) = env_engine(0.1, 0.1, 0.1, retrigger);
            e.note_hold(0, 1, 60, 100);
            blocks(&mut e, 40);
            assert!((env_now(&e) - 0.5).abs() < 1e-6, "{}", env_now(&e));
            e.note_hold(0, 1, 62, 100);
            blocks(&mut e, 1);
            let v = env_now(&e);
            if retrigger {
                assert!((v - (0.5 + 256.0 / 4800.0)).abs() < 1e-3, "restarted from 0.5: {v}");
            } else {
                assert!((v - 0.5).abs() < 1e-6, "legato: {v}");
            }
        }
        let (_tmp, _p, mut e, _c) = env_engine(0.1, 0.1, 0.1, false);
        e.note_hold(0, 1, 60, 100);
        blocks(&mut e, 40);
        e.note_release(0, 1, 60);
        blocks(&mut e, 2);
        let falling = env_now(&e);
        e.note_hold(0, 1, 64, 100);
        blocks(&mut e, 1);
        let v = env_now(&e);
        assert!((v - (falling + 256.0 / 4800.0)).abs() < 1e-3, "rising from {falling}: {v}");
    }

    /// A pattern note's end (its length) releases the envelope.
    #[test]
    fn a_pattern_notes_end_releases_the_envelope() {
        use deckard_model::lfo::Lfo;
        let l = Lfo { env: true, trigger: Some((0, 1)), attack: 0.001, decay: 0.001, release: 0.001, ..Lfo::new(1) };
        let (_tmp, mut p) = lfo_project(1, vec![l]);
        let mut n = p.groups[0].patterns[0].notes[0].clone();
        (n.sound, n.position, n.length, n.probability) = (1, 0, 240, 100);
        p.groups[0].patterns[0].notes.push(n);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        let end = (240.0 * e.samples_per_tick() / 256.0) as usize;
        blocks(&mut e, end - 2);
        assert!((env_now(&e) - 0.5).abs() < 1e-6, "held for its length: {}", env_now(&e));
        blocks(&mut e, 4);
        assert_eq!(env_now(&e), 0.0, "released at its end");
        assert_eq!(e.lfos[0].held, 0);
    }

    /// Stop lets go of every held note (the envelope releases), and so does a choke of
    /// the trigger sound.
    #[test]
    fn stop_releases_a_held_envelope() {
        let (_tmp, _p, mut e, _c) = env_engine(0.01, 0.01, 0.05, true);
        e.note_hold(0, 1, 60, 100);
        e.note_hold(0, 1, 62, 100);
        blocks(&mut e, 10);
        e.stop();
        assert_eq!(e.lfos[0].held, 0);
        blocks(&mut e, 1);
        assert!(env_now(&e) < 0.5 && env_now(&e) > 0.0, "releases: {}", env_now(&e));
        blocks(&mut e, 15);
        assert_eq!(env_now(&e), 0.0);
        e.note_hold(0, 1, 60, 100);
        blocks(&mut e, 10);
        let choke = NoteEvent {
            tick: 0,
            group: 0,
            sound: 1,
            pitch: 0,
            velocity: 0,
            length: 0,
            probability: 100,
            midi: Some(pads::CHOKE),
            cue: None,
            expr: None,
            from: None,
        };
        e.trigger(choke, 1.0 / e.samples_per_tick());
        assert_eq!(e.lfos[0].held, 0, "a choke ends its notes");
        blocks(&mut e, 15);
        assert_eq!(env_now(&e), 0.0);
    }

    /// An edit mid-note (the decay turned): the envelope carries on, held, at its level.
    #[test]
    fn an_edit_mid_note_keeps_the_envelope() {
        let (_tmp, p, mut e, mut cache) = env_engine(0.01, 0.01, 0.01, true);
        e.note_hold(0, 1, 60, 100);
        blocks(&mut e, 10);
        let mut q = p.clone();
        q.lfos[0].decay = 0.02;
        let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &q));
        assert_eq!(e.lfos[0].held, 1);
        assert!((env_now(&e) - 0.5).abs() < 1e-6, "no restart, no drop: {}", env_now(&e));
        blocks(&mut e, 1);
        assert!((env_now(&e) - 0.5).abs() < 1e-6, "{}", env_now(&e));
        e.note_release(0, 1, 60);
        blocks(&mut e, 6);
        assert_eq!(env_now(&e), 0.0);
    }

    /// Adopt `q` as an edit of `p`.
    fn adopt_edit(e: &mut Engine, p: &Project, q: &Project, cache: &mut SampleCache) {
        let (fresh, _) = Engine::new_after(q, Some((p, &[])), None, 48_000, Mode::Arrangement, cache);
        drop(e.adopt(fresh, q));
    }

    /// The trigger changed while its note is held: the envelope lets go of that note and
    /// releases from where it is (letting go of the old pad later changes nothing).
    #[test]
    fn a_trigger_changed_mid_note_releases() {
        let (_tmp, p, mut e, mut cache) = env_engine(0.01, 0.01, 0.05, true);
        e.note_hold(0, 1, 60, 100);
        blocks(&mut e, 10);
        let mut q = p.clone();
        q.lfos[0].trigger = Some((0, 2));
        // (Sound 2's own notes in the song would play the envelope again.)
        q.groups[0].patterns[0].notes.retain(|x| x.sound != 2);
        adopt_edit(&mut e, &p, &q, &mut cache);
        assert_eq!(e.lfos[0].held, 0);
        assert!((env_now(&e) - 0.5).abs() < 1e-6, "no jump: {}", env_now(&e));
        blocks(&mut e, 1);
        assert!(env_now(&e) < 0.5 && env_now(&e) > 0.0, "releasing: {}", env_now(&e));
        e.note_release(0, 1, 60);
        blocks(&mut e, 15);
        assert_eq!(env_now(&e), 0.0);
    }

    /// Envelope → LFO → Envelope with a note let go between: nothing stays held.
    #[test]
    fn a_mode_toggle_mid_note_never_sticks() {
        let (_tmp, p, mut e, mut cache) = env_engine(0.01, 0.01, 0.05, true);
        e.note_hold(0, 1, 60, 100);
        blocks(&mut e, 10);
        let mut q = p.clone();
        q.lfos[0].env = false;
        adopt_edit(&mut e, &p, &q, &mut cache);
        e.note_release(0, 1, 60);
        blocks(&mut e, 1);
        adopt_edit(&mut e, &q, &p, &mut cache);
        assert_eq!(e.lfos[0].held, 0);
        blocks(&mut e, 15);
        assert_eq!(env_now(&e), 0.0);
    }

    /// Turning Attack, Decay or Release mid-segment carries on from the level reached
    /// (moving no more than a block's worth), then follows the new time.
    #[test]
    fn a_time_edit_mid_segment_keeps_the_value() {
        let step = 256.0 / 48_000.0 / 0.5;
        // Attack 5 s, a second in: turned to 0.5 s.
        let (_tmp, p, mut e, mut cache) = env_engine(5.0, 0.3, 5.0, true);
        e.note_hold(0, 1, 60, 100);
        blocks(&mut e, 188);
        let v = env_now(&e);
        assert!((v - 0.2).abs() < 0.01, "{v}");
        let mut q = p.clone();
        q.lfos[0].attack = 0.5;
        adopt_edit(&mut e, &p, &q, &mut cache);
        assert!((env_now(&e) - v).abs() < 1e-4, "attack: {} vs {v}", env_now(&e));
        blocks(&mut e, 1);
        assert!(env_now(&e) - v > 0.0 && env_now(&e) - v <= step as f32 + 1e-4, "{}", env_now(&e));
        // Released, a second into a 5 s release: turned to 0.5 s.
        blocks(&mut e, 100);
        e.note_release(0, 1, 60);
        blocks(&mut e, 188);
        let v = env_now(&e);
        let mut r = q.clone();
        r.lfos[0].release = 0.5;
        adopt_edit(&mut e, &q, &r, &mut cache);
        assert!((env_now(&e) - v).abs() < 1e-4, "release: {} vs {v}", env_now(&e));
        blocks(&mut e, 1);
        assert!(v - env_now(&e) > 0.0 && v - env_now(&e) <= step as f32 + 1e-4, "{}", env_now(&e));
        // Decay 5 s to 0, a second in: turned to 0.5 s.
        let (_tmp, mut p, _e, mut cache) = env_engine(0.001, 5.0, 5.0, true);
        p.lfos[0].sustain = 0.0;
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        e.note_hold(0, 1, 60, 100);
        blocks(&mut e, 188);
        let v = env_now(&e);
        assert!((v - 0.8).abs() < 0.01, "{v}");
        let mut q = p.clone();
        q.lfos[0].decay = 0.5;
        adopt_edit(&mut e, &p, &q, &mut cache);
        assert!((env_now(&e) - v).abs() < 1e-4, "decay: {} vs {v}", env_now(&e));
        blocks(&mut e, 1);
        assert!(v - env_now(&e) > 0.0 && v - env_now(&e) <= step as f32 + 1e-4, "{}", env_now(&e));
    }

    /// A muted envelope, or one whose trigger is gone, gives nothing (its routes leave
    /// their targets alone).
    #[test]
    fn a_muted_or_untriggered_envelope_gives_nothing() {
        use deckard_model::lfo::Lfo;
        for l in [Lfo { env: true, trigger: Some((0, 1)), muted: true, ..Lfo::new(1) }, Lfo { env: true, trigger: None, ..Lfo::new(1) }] {
            let (_tmp, p) = lfo_project(1, vec![l]);
            let mut cache = SampleCache::default();
            let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
            e.note_hold(0, 1, 60, 100);
            blocks(&mut e, 4);
            assert_eq!(e.lfo_value(1), None);
            assert!((decay_now(&e) - 0.5).abs() < 1e-6, "the knob's own value: {}", decay_now(&e));
        }
    }

    /// An edit mid-fade: the fade carries on.
    #[test]
    fn an_edit_keeps_a_running_fade() {
        use deckard_model::lfo::Lfo;
        let mut l = Lfo::new(1);
        (l.trigger, l.fade) = (Some((0, 1)), 2.0);
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        e.note_on(0, 1, 60, 100, 240);
        blocks(&mut e, 24);
        let at = e.lfos[0].trig_at;
        assert!(at.is_some());
        let mut q = p.clone();
        q.lfos[0].level = 0.5;
        let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &q));
        assert_eq!(e.lfos[0].trig_at, at);
        assert_eq!(e.lfos[0].lfo.level, 0.5, "the edit's LFO");
    }

    /// An edit giving an LFO another trigger: it waits for that sound's first note.
    #[test]
    fn another_trigger_waits_for_its_note() {
        use deckard_model::lfo::Lfo;
        let l = Lfo { trigger: Some((0, 1)), ..Lfo::new(1) };
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        e.note_on(0, 1, 60, 100, 240);
        blocks(&mut e, 24);
        assert!(e.lfos[0].trig_at.is_some());
        let mut q = p.clone();
        q.lfos[0].trigger = Some((0, 2));
        let (fresh, _) = Engine::new_after(&q, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &q));
        assert_eq!(e.lfos[0].trig_at, None);
        assert_eq!(e.lfo_value(1), Some(0.0), "silent until the new sound's note");
    }

    /// Plays `frames` output frames in one go.
    fn play_frames(e: &mut Engine, frames: usize) {
        let mut buf = vec![0f32; frames * 2];
        e.process(&mut buf);
    }

    /// A synced LFO in Ideas counts the ticks played, each at the tempo then: a tempo
    /// change (an edit) mid-play doesn't move it, and it carries on at the new tempo.
    #[test]
    fn a_synced_lfo_follows_the_position_not_the_tempo_history() {
        use deckard_model::lfo::Lfo;
        let (_tmp, p) = lfo_project(1, vec![Lfo::new(1)]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
        play_frames(&mut e, 24_000);
        let c = e.lfo_cycles(&e.lfos[0]);
        assert!((c - 1.0).abs() < 1e-9, "a beat at 120: {c}");
        let mut slow = p.clone();
        slow.tempo = 90.0;
        let (fresh, _) = Engine::new_after(&slow, Some((&p, &[])), None, 48_000, Mode::Pattern(0, 0), &mut cache);
        drop(e.adopt(fresh, &slow));
        let c = e.lfo_cycles(&e.lfos[0]);
        assert!((c - 1.0).abs() < 1e-9, "the tempo changed, the LFO didn't move: {c}");
        play_frames(&mut e, 32_000);
        let c = e.lfo_cycles(&e.lfos[0]);
        assert!((c - 2.0).abs() < 1e-9, "a beat at 90: {c}");
    }

    /// After a bar's count-in, a bar played: a 1-bar synced LFO is exactly on the bar
    /// (the count-in doesn't count).
    #[test]
    fn a_synced_lfo_is_on_the_bar_after_a_count_in() {
        use deckard_model::lfo::Lfo;
        let l = Lfo { rate: 6, ..Lfo::new(1) };
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
        let bar = 96_000;
        e.count_in(bar as u64);
        play_frames(&mut e, bar);
        let c = e.lfo_cycles(&e.lfos[0]);
        assert!(c.abs() < 1e-9, "still at the start after the count-in: {c}");
        play_frames(&mut e, bar);
        let c = e.lfo_cycles(&e.lfos[0]);
        assert!((c - 1.0).abs() < 1e-9, "{c}");
    }

    /// A Hz LFO after a count-in is where it is in an export (from the song's start).
    #[test]
    fn a_hz_lfo_after_a_count_in_matches_an_export() {
        use deckard_model::lfo::Lfo;
        let l = Lfo { sync: false, hz: 0.7, ..Lfo::new(1) };
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
        e.count_in(96_000);
        play_frames(&mut e, 96_000 + 30_000);
        let (mut x, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
        play_frames(&mut x, 30_000);
        let (a, b) = (e.lfo_cycles(&e.lfos[0]), x.lfo_cycles(&x.lfos[0]));
        assert!((a - b).abs() < 1e-9, "{a} vs {b}");
        assert_eq!(e.lfo_value(1), x.lfo_value(1));
    }

    /// Stopped, an LFO keeps running at the tempo from where it was (in the
    /// arrangement too, from the position it stopped at).
    #[test]
    fn a_stopped_lfo_keeps_running_from_where_it_was() {
        use deckard_model::lfo::Lfo;
        let (_tmp, p) = lfo_project(1, vec![Lfo::new(1)]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        play_frames(&mut e, 24_000);
        e.stop();
        play_frames(&mut e, 24_000);
        let c = e.lfo_cycles(&e.lfos[0]);
        assert!((c - 2.0).abs() < 1e-9, "{c}");
    }

    /// A tempo change mid-fade: a triggered LFO carries on from where it was.
    #[test]
    fn a_tempo_change_keeps_a_triggered_lfo_in_place() {
        use deckard_model::lfo::Lfo;
        let l = Lfo { trigger: Some((0, 1)), fade: 4.0, ..Lfo::new(1) };
        let (_tmp, p) = lfo_project(1, vec![l]);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Pattern(0, 0), &mut cache);
        e.note_on(0, 1, 60, 100, 240);
        play_frames(&mut e, 24_000);
        let (c, v) = (e.lfo_cycles(&e.lfos[0]), e.lfo_value(1));
        assert!((c - 1.0).abs() < 1e-9, "{c}");
        let mut slow = p.clone();
        slow.tempo = 90.0;
        let (fresh, _) = Engine::new_after(&slow, Some((&p, &[])), None, 48_000, Mode::Pattern(0, 0), &mut cache);
        drop(e.adopt(fresh, &slow));
        assert!((e.lfo_cycles(&e.lfos[0]) - c).abs() < 1e-9);
        assert_eq!(e.lfo_value(1), v);
    }

    /// Pressure moves a plug-in instrument's parameter; the route gone, the
    /// modulation ends (the plug-in plays its knob again).
    #[test]
    fn pressure_moves_a_native_plugins_parameter() {
        use deckard_model::mod_matrix::{ModParam, ModSource};
        let class = deckard_plugins::native::CLASSES[0];
        let id = class.params.iter().find(|x| x.steps == 0).unwrap().id;
        let pr = deckard_model::PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
        let (_tmp, mut p) = eight_project();
        let snd = &mut p.groups[0].sounds[1];
        snd.sampler = None;
        snd.plugin = Some(pr.clone());
        snd.slots = vec![deckard_model::SlotInfo {
            module: "PluginHost".into(),
            name: class.name.into(),
            active: true,
            plugin: Some(pr.clone()),
            params: vec![],
        }];
        snd.mod_routes = vec![route(ModSource::Pressure, 0, &pr.id, ModParam::Plugin(id))];
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        press(&mut e, 0.75, 2);
        let plugin = e.instruments[0][1].as_ref().unwrap().plugin.clone();
        assert_eq!(plugin.lock().unwrap().modulated(id), Some(0.75));
        let mut plain = p.clone();
        plain.groups[0].sounds[1].mod_routes.clear();
        let (fresh, _) = Engine::new_after(&plain, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &plain));
        assert_eq!(plugin.lock().unwrap().modulated(id), None, "removing a route restores the plug-in");
        // Busy (the UI holding it) when the route goes: it's let go once free.
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        press(&mut e, 0.5, 2);
        let (fresh, _) = Engine::new_after(&plain, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        {
            let _busy = plugin.lock().unwrap();
            drop(e.adopt(fresh, &plain));
        }
        assert_eq!(plugin.lock().unwrap().modulated(id), Some(0.5), "still held");
        blocks(&mut e, 1);
        assert_eq!(plugin.lock().unwrap().modulated(id), None, "let go once free");
    }

    /// A Sampler route ended by its sound's instrument changing (a Kick loaded in its
    /// place) leaves the new instrument as it is.
    #[test]
    fn ending_a_route_leaves_the_next_instrument_alone() {
        use deckard_model::mod_matrix::ModSource;
        let (_tmp, mut p) = eight_project();
        p.groups[0].sounds[1].sampler.as_mut().unwrap().set("tune", 0.9);
        p.groups[0].sounds[1].mod_routes = vec![route(ModSource::Pressure, 0, "Sampler", field("tune"))];
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        press(&mut e, 0.5, 2);
        let mut kick = p.clone();
        let snd = &mut kick.groups[0].sounds[1];
        snd.sampler = None;
        snd.slots = vec![deckard_model::SlotInfo {
            module: "Kick".into(),
            name: "Kick".into(),
            active: true,
            plugin: None,
            params: vec![("tune".into(), 0.2)],
        }];
        let (fresh, _) = Engine::new_after(&kick, Some((&p, &[])), None, 48_000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &kick));
        let v = e.sounds[0][1].as_ref().unwrap().synth_param("tune").unwrap();
        assert!((v - 0.2).abs() < 1e-9, "the Kick's own tune: {v}");
    }

    /// A route whose slot holds another module does nothing.
    #[test]
    fn a_route_to_another_module_does_nothing() {
        use deckard_model::mod_matrix::ModSource;
        let (_tmp, mut p, base) = modulated_project();
        p.groups[0].patterns[0].mods.clear();
        p.groups[0].sounds[1].mod_routes = vec![route(ModSource::Pressure, 1, "Compressor", field("feedback"))];
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        assert!(e.mod_routes.is_empty());
        press(&mut e, 1.0, 20);
        assert!((feedback_now(&e) - base).abs() < 1e-6);
    }

    /// A group's lane moves its bus's effect.
    #[test]
    fn modulation_moves_a_groups_effect() {
        let (_tmp, mut p, _) = modulated_project();
        let delay = p.groups[0].sounds[1].slots[1].clone();
        p.groups[0].slots = vec![delay];
        let len = p.groups[0].patterns[0].length.max(p.bar_length());
        let point = |position, value| deckard_model::ModPoint { sound: None, slot: Some(0), field: "feedback".into(), position, value };
        p.groups[0].patterns[0].mods = vec![point(0, 0.4), point(len, 0.4)];
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        let mut buf = vec![0f32; 256 * 2];
        for _ in 0..10 {
            e.process(&mut buf);
        }
        let bus = e.buses[0].as_ref().unwrap();
        let FxUnit::Internal { glide, .. } = &bus.chain.units[0].fx else { unreachable!() };
        let v = glide.now().iter().find(|(k, _)| k == "feedback").unwrap().1;
        assert!((v - 0.9).abs() < 1e-6, "0.5 up 0.4: {v}");
    }

    /// Pattern modulation lanes move the Sampler's parameters and a plug-in's parameters
    /// while playing, and let go when stopped.
    #[test]
    fn modulation_moves_the_sampler_and_a_plugin_while_playing() {
        let (_tmp, mut p) = eight_project();
        let len = p.groups[0].patterns[0].length.max(p.bar_length());
        let base_cutoff = p.groups[0].sounds[1].sampler.as_ref().unwrap().get("cutoff");
        let class = deckard_plugins::native::CLASSES[0];
        let param = class.params.iter().find(|x| x.steps == 0).unwrap();
        let id = param.id;
        let pr = deckard_model::PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
        let snd2 = &mut p.groups[0].sounds[2];
        snd2.sampler = None;
        snd2.plugin = Some(pr.clone());
        snd2.slots = vec![deckard_model::SlotInfo {
            module: "PluginHost".into(),
            name: class.name.into(),
            active: true,
            plugin: Some(pr),
            params: vec![(id.to_string(), 0.25)],
        }];
        let pt = |sound, field: &str, position, value| deckard_model::ModPoint {
            sound: Some(sound),
            slot: Some(0),
            field: field.into(),
            position,
            value,
        };
        let id_field = id.to_string();
        p.groups[0].patterns[0].mods =
            vec![pt(1, "cutoff", 0, -0.5), pt(1, "cutoff", len, -0.5), pt(2, &id_field, 0, 0.5), pt(2, &id_field, len, 0.5)];
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        blocks(&mut e, 2);
        let cutoff = e.sounds[0][1].as_ref().unwrap().settings_cutoff();
        assert!((f64::from(cutoff) - (base_cutoff - 0.5).clamp(0.0, 1.0)).abs() < 1e-4, "{cutoff} vs {base_cutoff}");
        let plugin = e.instruments[0][2].as_ref().unwrap().plugin.clone();
        let want = (param.default + 0.5).clamp(0.0, 1.0);
        assert!((plugin.lock().unwrap().modulated(id).unwrap() - want).abs() < 1e-9);
        e.sequencing = false;
        blocks(&mut e, 2);
        let cutoff = e.sounds[0][1].as_ref().unwrap().settings_cutoff();
        assert!((f64::from(cutoff) - base_cutoff).abs() < 1e-4, "stopped: back to {base_cutoff} ({cutoff})");
        assert_eq!(plugin.lock().unwrap().modulated(id), None, "stopped: plug-in back to its knob");
    }

    /// A group's Perform FX: engaged (Reso Echo, the strip low) it's heard; not engaged,
    /// the group sounds as without it. (Reso Echo: released with nothing to ring out,
    /// Maschine's passes the input through exactly. Its Filter doesn't: it always runs
    /// through its oversampling, 16 samples late; docs/fx-perform-filter.md.)
    #[test]
    fn a_groups_perform_fx_plays_when_engaged() {
        let (_tmp, p) = eight_project();
        let slot = |engaged: f64| deckard_model::SlotInfo {
            module: deckard_model::PERFORM_FX.0.into(),
            name: "Perform FX".into(),
            active: true,
            plugin: None,
            params: deckard_model::PERFORM_FX
                .1
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        match *k {
                            "touchstrip_on_off" => engaged,
                            "touchstrip_pos" => 0.2,
                            "mode" => 3.0,
                            _ => *v,
                        },
                    )
                })
                .collect(),
        };
        let with = |engaged: f64| {
            let mut q = p.clone();
            q.groups[0].slots.insert(0, slot(engaged));
            render_with(&q, None, 256, 1.0, |_, _, _| {})
        };
        let plain = render_with(&p, None, 256, 1.0, |_, _, _| {});
        assert!(max_diff(&plain, &with(0.0)) < 1e-6, "disengaged: as without it");
        assert!(max_diff(&plain, &with(1.0)) > 0.05, "engaged: heard");
    }

    /// The master's effect runs on the whole mix, before the master's level.
    #[test]
    fn master_effects_play() {
        let (_tmp, p) = eight_project();
        let mut fx = p.clone();
        fx.master_slots = vec![driven_saturator()];
        let plain = render_with(&p, None, 256, 1.0, |_, _, _| {});
        let driven = render_with(&fx, None, 256, 1.0, |_, _, _| {});
        assert!(max_diff(&plain, &driven) > 0.05, "the master's saturator is heard");
        let mut silent = fx.clone();
        silent.master_level = 0.0;
        assert!(render_with(&silent, None, 256, 0.5, |_, _, _| {}).iter().all(|x| x.abs() < 1e-6));
    }

    /// An edit that takes sounds or groups away while sounds render ahead (an undo of
    /// loading a group, say), and one that brings them back: the players of sounds no
    /// longer there are let go, and it plays on.
    #[test]
    fn sounds_taken_away_while_rendering_ahead() {
        let (_tmp, p) = eight_project();
        let mut fewer = p.clone();
        fewer.groups[0].sounds.truncate(4);
        for pat in &mut fewer.groups[0].patterns {
            pat.notes.retain(|n| n.sound < 4);
        }
        let mut more = p.clone();
        more.groups.push(p.groups[0].clone());
        let (fewer, more, back) = (fewer, more, p.clone());
        let out = render_with(&p, Some(700), 256, 2.0, move |k, e, cache| match k {
            40 => e.update(&more, None, cache),
            80 => e.update(&fewer, None, cache),
            120 => e.update(&back, None, cache),
            _ => {}
        });
        assert!(out.iter().all(|x| x.is_finite()));
        assert!(out[130 * 512..].iter().any(|x| x.abs() > 1e-4), "it plays on");
    }

    /// Edits built incrementally (only what changed, the rest kept) sound exactly as
    /// edits built in full: a level, a Sampler knob, an effect knob, a bypass, a note
    /// added, a mute and unmute; with sounds rendered here and ahead.
    #[test]
    fn incremental_edits_sound_like_full_rebuilds() {
        let (_tmp, p0) = eight_project();
        let mut steps: Vec<(usize, Project)> = Vec::new();
        let mut p = p0.clone();
        p.groups[0].sounds[3].level = 0.3;
        steps.push((40, p.clone()));
        p.groups[0].sounds[2].sampler.as_mut().unwrap().set("tune", 7.0);
        steps.push((60, p.clone()));
        for (k, v) in p.groups[0].sounds[5].slots[1].params.iter_mut() {
            if k.contains("drive") {
                *v = 0.8;
            }
        }
        steps.push((80, p.clone()));
        p.groups[0].sounds[1].slots[1].active = false;
        steps.push((100, p.clone()));
        p.groups[0].patterns[0].notes.push(Note {
            expr: None,
            sound: 6,
            position: 1000,
            length: 400,
            pitch: 55,
            velocity: 120,
            probability: 100,
        });
        steps.push((120, p.clone()));
        p.groups[0].sounds[4].muted = true;
        p.groups[0].sounds[4].channel.audio_mute = true;
        steps.push((140, p.clone()));
        p.groups[0].sounds[4].muted = false;
        p.groups[0].sounds[6].pan = -0.5;
        steps.push((160, p.clone()));
        let chains0 = Engine::new(&p0, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
        let render = |incremental: bool, ahead: Option<u64>| {
            let (mut prev, mut chains) = (p0.clone(), chains0.clone());
            render_with(&p0, ahead, 256, 4.0, |k, e, cache| {
                let Some((_, to)) = steps.iter().find(|(at, _)| *at == k) else { return };
                let (fresh, report) =
                    Engine::new_after(to, incremental.then_some((&prev, chains.as_slice())), None, 48_000, e.mode(), cache);
                if incremental && k == 40 {
                    // A level: nothing built (the timeline aside).
                    assert!(
                        fresh.sounds[0].iter().all(Option::is_none) && fresh.fx[0].iter().all(Option::is_none),
                        "only what changed is built"
                    );
                }
                drop(e.adopt(fresh, to));
                (prev, chains) = (to.clone(), report.chains);
            })
        };
        for ahead in [None, Some(700)] {
            let (full, incr) = (render(false, ahead), render(true, ahead));
            assert!(full.iter().any(|x| x.abs() > 0.05), "it plays");
            let first = full.iter().zip(&incr).position(|(a, b)| a != b);
            assert_eq!(
                first,
                None,
                "ahead {ahead:?}: first difference at frame {:?} (block {:?})",
                first.map(|i| i / 2),
                first.map(|i| i / 512)
            );
        }
    }

    /// Stop reaches sounds rendered ahead: they fade within a lookahead (and the
    /// short fade), as they do rendered here.
    #[test]
    fn stop_reaches_sounds_rendered_ahead() {
        let (_tmp, p) = eight_project();
        let tail = |ahead: Option<u64>| {
            let out = render_with(&p, ahead, 256, 1.0, |k, e, _| {
                if k == 80 {
                    e.stop();
                }
            });
            // Last loud frame after the stop.
            out.chunks(2).rposition(|f| f[0].abs() > 1e-3).unwrap_or(0) as i64 - 80 * 256
        };
        // (Effects ring on after a stop: the delay's echoes last a while.)
        let (inline, ahead) = (tail(None), tail(Some(700)));
        assert!(inline > 0, "it played");
        assert!(ahead <= inline + 700 + 256, "ahead: within a lookahead more ({ahead} vs {inline} frames)");
    }

    /// A live note for a sound rendered ahead plays within a lookahead.
    #[test]
    fn live_notes_reach_sounds_rendered_ahead() {
        let (_tmp, mut p) = eight_project();
        p.groups[0].patterns[0].notes.clear();
        let onset = |ahead: Option<u64>| {
            let out = render_with(&p, ahead, 256, 0.5, |k, e, _| {
                e.sequencing = false;
                if k == 10 {
                    e.note_on(0, 2, 60, 127, PPQ);
                }
            });
            out.chunks(2).position(|f| f[0].abs() > 1e-3)
        };
        let (now, later) = (onset(None).unwrap(), onset(Some(700)).unwrap());
        assert_eq!(now, 10 * 256, "inline: at once");
        assert_eq!(later, now + 700 + 256, "ahead: exactly a lookahead and a block later");
    }

    /// Rendering ahead plays what the audio thread would have: the same notes at the
    /// same frames, through the same effects.
    #[test]
    fn rendering_ahead_sounds_the_same() {
        let inline = render_eight(None, 256, 3.0);
        assert!(inline.iter().any(|x| x.abs() > 0.05), "it plays");
        for (lookahead, block) in [(512u64, 256usize), (2048, 128), (300, 512)] {
            let ahead = render_eight(Some(lookahead), block, 3.0);
            let inline = render_eight(None, block, 3.0);
            let err = inline.iter().zip(&ahead).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
            let peak = inline.iter().fold(0f32, |m, a| m.max(a.abs()));
            eprintln!("lookahead {lookahead}, block {block}: max difference {err:e} (peak {peak})");
            assert!(err <= peak * 1e-4, "lookahead {lookahead}, block {block}: differs by {err} (peak {peak})");
        }
    }

    /// Sounds rendered on worker threads give exactly what one thread gives: eight
    /// sounds, half through effects (with state: delay, chorus, limiter), notes at
    /// different times.
    #[test]
    fn worker_threads_render_the_same() {
        let mut p = project();
        let base = p.groups[0].sounds[0].clone();
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
            vec![fx("Limiter", vec![("mode".into(), 1.0), ("threshold".into(), -20.0), ("release".into(), 0.0), ("ceiling".into(), 0.0)])],
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
        p.groups[0].patterns[0].notes = (0..32)
            .map(|i| Note {
                expr: None,
                sound: i % 8,
                position: i as i64 * 37,
                length: 3600,
                pitch: 48 + (i as i64 * 5) % 24,
                velocity: 100,
                probability: 100,
            })
            .collect();
        let render_with = |pool: Option<Arc<pool::RenderPool>>| {
            let mut e = click_engine_for(&p, 48_000);
            let noise: Vec<[f32; 2]> = (0..96_000u32)
                .map(|i| {
                    let x = (i.wrapping_mul(2_654_435_761) >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                    [x, -x * 0.7]
                })
                .collect();
            set_sample(&mut e, noise, &[], false);
            e.pool = pool;
            let mut out = Vec::new();
            // Large blocks: jobs long enough for sleeping workers to wake and join in.
            let mut block = vec![0f32; 2 * 2048];
            for _ in 0..(48_000 * 2 / 2048) {
                e.process(&mut block);
                out.extend_from_slice(&block);
            }
            out
        };
        let one = render_with(None);
        assert!(one.iter().any(|x| x.abs() > 0.01), "it plays");
        let pool = Arc::new(pool::RenderPool::new(3, 256, 48_000));
        // (On a busy machine the workers may not wake in time for a while: try more.)
        for k in 0..10 {
            let many = render_with(Some(pool.clone()));
            assert!(one.iter().zip(&many).all(|(a, b)| a.to_bits() == b.to_bits()), "bit for bit");
            if k >= 2 && pool.helped() > 0 {
                break;
            }
        }
        assert!(pool.helped() > 0, "the workers rendered sounds");
    }

    /// Turning a knob while playing: the running effect keeps its state and takes the
    /// new setting (a limiter's ceiling pulled down holds the next click down).
    #[test]
    fn effect_settings_change_while_playing() {
        let mut p = project();
        let lim = |ceiling: f64| deckard_model::SlotInfo {
            module: "Limiter".into(),
            name: "Limiter".into(),
            active: true,
            plugin: None,
            params: vec![("mode".into(), 1.0), ("threshold".into(), -30.0), ("release".into(), 0.0), ("ceiling".into(), ceiling)],
        };
        let sampler =
            deckard_model::SlotInfo { module: "Sampler".into(), name: "Sampler".into(), active: true, plugin: None, params: Vec::new() };
        p.groups[0].sounds[0].slots = vec![sampler, lim(1.0)];
        let mut e = click_engine_for(&p, 48_000);
        e.sequencing = false;
        let peak = |e: &mut Engine| {
            e.note_on(0, 0, 60, 127, 960);
            let mut out = vec![0f32; 2 * 4800];
            e.process(&mut out);
            out.iter().fold(0f32, |m, v| m.max(v.abs()))
        };
        let open = peak(&mut e);
        p.groups[0].sounds[0].slots[1] = lim(0.0);
        let samples = e.samples.clone();
        let sound = e.sounds[0][0].take();
        e.update(&p, None, &mut Default::default());
        e.samples = samples; // the test sample (Engine::new found no files)
        e.sounds[0][0] = sound;
        // The new ceiling glides in (as in Maschine) before the next click.
        e.process(&mut vec![0f32; 2 * 4800]);
        let held = peak(&mut e);
        assert!(open > 0.5, "the click passes at a high ceiling: {open}");
        assert!(held <= 10f32.powf(-40.05 / 20.0) * 1.01, "the new ceiling holds it down: {held}");
    }

    /// An effect that keeps the last value of its first setting (as its output).
    struct LastSet(Arc<std::sync::Mutex<Vec<f64>>>);
    impl fx::Effect for LastSet {
        fn set(&mut self, params: &[(String, f64)]) {
            self.0.lock().unwrap().push(params[0].1);
        }
        fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
            let v = *self.0.lock().unwrap().last().unwrap() as f32;
            l.fill(v);
            r.fill(v);
        }
    }

    /// A knob turned on an effect glides: the effect is taken there evenly over
    /// glide::GLIDE_MS, a step every glide::STEP frames; a choice changes at once.
    #[test]
    fn effect_settings_glide() {
        let sr = 48_000.0;
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let params = |v: f64, mode: f64| vec![("cutoff".to_string(), v), ("mode".to_string(), mode)];
        let mut fx: Box<dyn fx::Effect> = Box::new(LastSet(seen.clone()));
        fx.set(&params(0.2, 0.0));
        let unit = FxUnit::Internal {
            class: "Test".into(),
            params: params(0.2, 0.0),
            fx,
            glide: glide::Glide::new(&params(0.2, 0.0), sr),
            modded: params(0.2, 0.0),
            hit: false,
            modulated: false,
        };
        let mut chain = FxChain {
            units: vec![Unit::new(unit, 1, Vec::new(), true)],
            gain: (1.0, 1.0),
            l: vec![0.0; MAX_BLOCK],
            r: vec![0.0; MAX_BLOCK],
            dl: vec![0.0; MAX_BLOCK],
            dr: vec![0.0; MAX_BLOCK],
            fade: 0,
            meters: None,
            tap: Vec::new(),
            sl: vec![0.0; MAX_BLOCK],
            sr: vec![0.0; MAX_BLOCK],
            heard: None,
        };
        let transport = Transport { tempo: 120.0, playing: false, quarters: 0.0, numerator: 4, denominator: 4 };
        chain.set_params(&mut [(1, params(0.8, 2.0))]);
        let mut out = Vec::new();
        let n = 256.min(MAX_BLOCK);
        for _ in 0..(sr as usize / 20 / n) {
            chain.run(n, &transport, &[], 0);
            out.extend_from_slice(&chain.l[..n]);
        }
        let at = |ms: usize| out[ms * 48] as f64;
        assert!((at(0) - 0.2).abs() < 0.05, "starts where it was: {}", at(0));
        assert!(at(10) > 0.3 && at(10) < 0.6, "on its way at 10 ms: {}", at(10));
        assert!((at(35) - 0.8).abs() < 1e-6, "there by 35 ms: {}", at(35));
        let steps = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
        assert!(steps < 0.6 / 20.0, "in small steps: {steps}");
        assert!(seen.lock().unwrap().len() > 20, "set as it goes");
    }

    /// A Filter's cutoff turned down while a bright tone plays takes it out: the edit
    /// reaches the running effect (and glides there).
    #[test]
    fn filter_cutoff_turned_down_while_playing() {
        let filter = |cutoff: f64| deckard_model::SlotInfo {
            module: "Filter".into(),
            name: "Filter".into(),
            active: true,
            plugin: None,
            params: vec![("mode".into(), 0.0), ("cutoff".into(), cutoff), ("resonance".into(), 0.0), ("amount".into(), 0.0)],
        };
        let sampler =
            deckard_model::SlotInfo { module: "Sampler".into(), name: "Sampler".into(), active: true, plugin: None, params: Vec::new() };
        let mut p = project();
        p.groups[0].sounds[0].slots = vec![sampler, filter(1.0)];
        let mut e = click_engine_for(&p, 48_000);
        e.sequencing = false;
        // A bright tone: 6 kHz.
        let tone: Vec<[f32; 2]> = (0..48_000).map(|i| [0.5 * (i as f32 * std::f32::consts::TAU / 8.0).sin(); 2]).collect();
        set_sample(&mut e, tone, &[], true);
        e.note_hold(0, 0, 60, 127);
        let rms = |e: &mut Engine| {
            let mut out = vec![0f32; 2 * 4800];
            e.process(&mut out);
            (out.iter().skip(4800).map(|x| x * x).sum::<f32>() / 4800.0).sqrt()
        };
        rms(&mut e);
        let open = rms(&mut e);
        p.groups[0].sounds[0].slots[1] = filter(0.0);
        let samples = e.samples.clone();
        let sound = e.sounds[0][0].take();
        e.update(&p, None, &mut Default::default());
        e.samples = samples; // the test sample (Engine::new found no files)
        e.sounds[0][0] = sound;
        rms(&mut e);
        let closed = rms(&mut e);
        assert!(open > 0.05, "the tone plays: {open}");
        assert!(closed < open * 0.1, "the cutoff turned down takes it out: {open} -> {closed}");
    }

    /// Play a steady tone through sound 0 of `before`, then take up `after` while it
    /// sounds; the left channel from just before the edit to 50 ms after it.
    fn edit_while_playing(before: &Project, after: &Project) -> Vec<f32> {
        let mut e = click_engine_for(before, 48_000);
        e.sequencing = false;
        set_sample(&mut e, vec![[0.5, 0.5]; 48_000], &[], true);
        e.note_hold(0, 0, 60, 127);
        // Let effects settle.
        let mut block = vec![0f32; 2 * 4800];
        for _ in 0..4 {
            e.process(&mut block);
        }
        let mut out = block[block.len() - 2 * 256..].iter().step_by(2).copied().collect::<Vec<f32>>();
        let samples = e.samples.clone();
        let sound = e.sounds[0][0].take();
        e.update(after, None, &mut Default::default());
        e.samples = samples; // the test sample (Engine::new found no files)
        e.sounds[0][0] = sound;
        let mut after_edit = vec![0f32; 2 * 2400];
        e.process(&mut after_edit);
        out.extend(after_edit.iter().step_by(2));
        out
    }

    /// The largest step between neighbouring samples (a jump is a click).
    fn largest_step(x: &[f32]) -> f32 {
        x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()))
    }

    /// Level, pan and mute glide: turning a sound down while it holds a steady tone
    /// takes ~10 ms, with no step.
    #[test]
    fn level_changes_glide() {
        let before = project();
        let mut after = before.clone();
        after.groups[0].sounds[0].level = 0.25;
        let x = edit_while_playing(&before, &after);
        let (from, to) = (x[255], *x.last().unwrap());
        assert!(from > 0.1 && to < from * 0.5, "it was turned down: {from} -> {to}");
        assert!((x[256] - from).abs() < (from - to) * 0.01, "no jump at the edit: {} after {from}", x[256]);
        assert!((x[256 + 480] - to).abs() < 1e-6, "there after 10 ms");
        assert!(largest_step(&x) < (from - to) / 300.0, "no step: {}", largest_step(&x));
    }

    /// Bypassing an effect while it plays crossfades (20 ms) instead of switching, and
    /// switching it back on resumes it with its state (no fresh start, no overshoot).
    #[test]
    fn bypass_crossfades_and_keeps_state() {
        let slot = |module: &str, active: bool, params: Vec<(String, f64)>| deckard_model::SlotInfo {
            module: module.into(),
            name: module.into(),
            active,
            plugin: None,
            params,
        };
        let lim = vec![("mode".into(), 1.0), ("threshold".into(), -20.0), ("release".into(), 0.5), ("ceiling".into(), 0.0)];
        let mut on = project();
        on.groups[0].sounds[0].slots = vec![slot("Sampler", true, Vec::new()), slot("Limiter", true, lim.clone())];
        let mut off = on.clone();
        off.groups[0].sounds[0].slots[1].active = false;
        let x = edit_while_playing(&on, &off);
        let (wet, dry) = (x[255], *x.last().unwrap());
        assert!(dry > wet * 2.0, "the limiter was holding it down: {wet} vs {dry}");
        assert!((x[256] - wet).abs() < (dry - wet) * 0.01, "no jump at the edit");
        assert!(largest_step(&x) < (dry - wet) / 300.0, "a crossfade, not a switch: {}", largest_step(&x));
        // Back on: the limiter picks up where it was (already holding the level).
        let y = edit_while_playing(&off, &on);
        let back = *y.last().unwrap();
        assert!(largest_step(&y) < (dry - wet) / 300.0, "back on without a step: {}", largest_step(&y));
        assert!(y.iter().all(|v| *v <= dry * 1.001), "no overshoot");
        assert!(back < dry * 0.9, "the limiter is back: {back}");
    }

    /// Removing an effect fades it out where it was, and the effects after it keep
    /// their state (they are paired by order, not by slot number).
    #[test]
    fn removing_an_effect_fades_it_out() {
        let slot = |module: &str, params: Vec<(String, f64)>| deckard_model::SlotInfo {
            module: module.into(),
            name: module.into(),
            active: true,
            plugin: None,
            params,
        };
        let lim = vec![("mode".into(), 1.0), ("threshold".into(), -20.0), ("release".into(), 0.5), ("ceiling".into(), 0.0)];
        let sat = deckard_model::INTERNAL_EFFECTS
            .iter()
            .find(|(c, _)| *c == "Saturator")
            .unwrap()
            .1
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect();
        let mut both = project();
        both.groups[0].sounds[0].slots = vec![slot("Sampler", Vec::new()), slot("Saturator", sat), slot("Limiter", lim)];
        let mut one = both.clone();
        one.groups[0].sounds[0].slots.remove(1);
        let x = edit_while_playing(&both, &one);
        let swing = x.iter().fold(0f32, |m, v| m.max(*v)) - x.iter().fold(f32::MAX, |m, v| m.min(*v));
        assert!(largest_step(&x) <= swing / 100.0 + 1e-6, "no click: step {} of {swing}", largest_step(&x));
    }

    /// Side-chaining: sound 1 (routed to None, so unheard) drives a limiter on sound 0.
    /// When it plays, sound 0 comes out held down; when it doesn't, sound 0 passes.
    #[test]
    fn sidechain_ducks_a_sound() {
        let mut p = project();
        let click = p.groups[0].sounds[0].clone();
        p.groups[0].sounds.push(click);
        let slot = |module: &str, params: Vec<(String, f64)>| deckard_model::SlotInfo {
            module: module.into(),
            name: module.into(),
            active: true,
            plugin: None,
            params,
        };
        // Transparent limiter, no drive on the audio (side-chained), 30 dB on the
        // detector, the ceiling at its top.
        let lim = vec![("mode".into(), 1.0), ("threshold".into(), -30.0), ("release".into(), 0.5), ("ceiling".into(), 1.0)];
        p.groups[0].sounds[0].slots = vec![slot("Sampler", Vec::new()), slot("Limiter", lim)];
        p.groups[0].sounds[0].channel.sidechain = vec![(1, deckard_model::ScSource::Sound(0, 1))];
        p.groups[0].sounds[1].channel.dest = deckard_model::AudioDest::None;
        let peak = |with_source: bool| {
            let mut e = click_engine_for(&p, 48_000);
            e.sequencing = false;
            e.note_on(0, 0, 60, 127, 960);
            if with_source {
                e.note_on(0, 1, 60, 127, 960);
            }
            let mut out = vec![0f32; 2 * 4800];
            e.process(&mut out);
            out.iter().fold(0f32, |m, v| m.max(v.abs()))
        };
        let (alone, ducked) = (peak(false), peak(true));
        assert!(alone > 0.5, "sound 0 alone passes: {alone}");
        assert!(ducked < alone * 0.1, "held down by sound 1's side-chain: {ducked} vs {alone}");
    }

    /// Group routing: a Limiter on group 0 side-chained from sound 1 (routed to None)
    /// ducks group 0's output when sound 1 plays.
    #[test]
    fn sidechain_ducks_a_group() {
        let mut p = project();
        let click = p.groups[0].sounds[0].clone();
        p.groups[0].sounds.push(click);
        let lim = vec![("mode".into(), 1.0), ("threshold".into(), -30.0), ("release".into(), 0.5), ("ceiling".into(), 1.0)];
        p.groups[0].slots =
            vec![deckard_model::SlotInfo { module: "Limiter".into(), name: "Limiter".into(), active: true, plugin: None, params: lim }];
        p.groups[0].channel.sidechain = vec![(0, deckard_model::ScSource::Sound(0, 1))];
        p.groups[0].sounds[1].channel.dest = deckard_model::AudioDest::None;
        let peak = |with_source: bool| {
            let mut e = click_engine_for(&p, 48_000);
            e.sequencing = false;
            e.note_on(0, 0, 60, 127, 960);
            if with_source {
                e.note_on(0, 1, 60, 127, 960);
            }
            let mut out = vec![0f32; 2 * 4800];
            e.process(&mut out);
            out.iter().fold(0f32, |m, v| m.max(v.abs()))
        };
        let (alone, ducked) = (peak(false), peak(true));
        assert!(alone > 0.5, "group 0 alone passes: {alone}");
        assert!(ducked < alone * 0.1, "group 0 held down by sound 1's side-chain: {ducked} vs {alone}");
    }

    /// Plug-in side-chain routing: a plug-in effect with a side-chain input receives the
    /// routed side-chain audio (filtered through its `SidechainFilter`) on both Sound and
    /// Group chains.
    #[test]
    fn sidechain_ducks_a_plugin_effect() {
        struct Duck;
        impl deckard_plugins::native::Engine for Duck {
            fn set_param(&mut self, _: u32, _: f64) {}
            fn note_on(&mut self, _: u8, _: u8) {}
            fn note_off(&mut self, _: u8) {}
            fn reset(&mut self) {}
            fn has_sidechain(&self) -> bool {
                true
            }
            fn process(&mut self, _: &mut [f32], _: &mut [f32], _: &deckard_plugins::Transport) {}
            fn process_sc(
                &mut self,
                left: &mut [f32],
                right: &mut [f32],
                sidechain: Option<(&[f32], &[f32])>,
                _: &deckard_plugins::Transport,
            ) {
                if let Some((sl, sr)) = sidechain {
                    for i in 0..left.len() {
                        let sc = sl[i].abs().max(sr[i].abs());
                        let g = (1.0 - sc * 4.0).max(0.0);
                        left[i] *= g;
                        right[i] *= g;
                    }
                }
            }
        }
        static DUCK_CLASS: deckard_plugins::native::Class = deckard_plugins::native::Class {
            cid: *b"DKRDTESTDUCKFX01",
            name: "Duck",
            instrument: false,
            params: &[],
            pages: &[],
            make: |_| Box::new(Duck),
        };

        let run = |on_group: bool, with_source: bool, sc_gain: f64| {
            let mut p = project();
            let click = p.groups[0].sounds[0].clone();
            p.groups[0].sounds.push(click);
            p.groups[0].sounds[1].channel.dest = deckard_model::AudioDest::None;
            let plugin_id = DUCK_CLASS.plugin_id();
            let plug_slot = deckard_model::SlotInfo {
                module: "PluginHost".into(),
                name: "Duck".into(),
                active: true,
                plugin: Some(deckard_model::PluginRef { id: plugin_id.clone(), name: "Duck".into(), chunk: Vec::new(), pages: Vec::new() }),
                params: vec![
                    ("side_chain_filter/gain".into(), sc_gain),
                    ("side_chain_filter/center_freq".into(), 0.5),
                    ("side_chain_filter/width".into(), 0.5),
                ],
            };
            let host_slot = if on_group {
                p.groups[0].slots = vec![plug_slot];
                p.groups[0].channel.sidechain = vec![(0, deckard_model::ScSource::Sound(0, 1))];
                deckard_plugins::Slot::Group { group: 0, slot: 0 }
            } else {
                p.groups[0].sounds[0].slots = vec![
                    deckard_model::SlotInfo {
                        module: "Sampler".into(),
                        name: "Sampler".into(),
                        active: true,
                        plugin: None,
                        params: Vec::new(),
                    },
                    plug_slot,
                ];
                p.groups[0].sounds[0].channel.sidechain = vec![(1, deckard_model::ScSource::Sound(0, 1))];
                deckard_plugins::Slot::Chain { group: 0, sound: 0, slot: 1 }
            };
            let mut cache = SampleCache::default();
            let id = deckard_plugins::state::PluginId::parse(&plugin_id, "Duck").unwrap();
            cache.plugins.insert(host_slot, deckard_plugins::Plugin::native(&DUCK_CLASS, id, "Duck", 48_000.0));
            let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
            let mut frames = vec![[0f32; 2]; 100];
            frames[0] = [1.0, 1.0];
            set_sample(&mut e, frames, &[], false);
            e.sequencing = false;
            e.note_on(0, 0, 60, 127, 960);
            if with_source {
                e.note_on(0, 1, 60, 127, 960);
            }
            let mut out = vec![0f32; 2 * 480];
            e.process(&mut out);
            out.iter().fold(0f32, |m, v| m.max(v.abs()))
        };

        for on_group in [false, true] {
            let alone = run(on_group, false, 0.5);
            let ducked = run(on_group, true, 0.5);
            let attenuated_sc = run(on_group, true, 0.0);
            assert!(alone > 0.5, "on_group={on_group}: passes without side-chain: {alone}");
            assert!(ducked < 0.01, "on_group={on_group}: ducked by side-chain: {ducked}");
            assert!(
                (attenuated_sc - alone * 0.6).abs() < 1e-3,
                "on_group={on_group}: side_chain_filter/gain=0 (-20 dB) scales side-chain by 0.1: {attenuated_sc} vs {}",
                alone * 0.6
            );
        }
    }

    /// Playing patterns, rendering ahead, updated as the app updates (incrementally): a
    /// sound whose Compressor is side-chained from another goes silent when muted.
    #[test]
    fn a_side_chained_sound_mutes_while_playing() {
        let (_tmp, mut p) = eight_project();
        let comp: Vec<(String, f64)> = deckard_model::INTERNAL_EFFECTS
            .iter()
            .find(|(c, _)| *c == "Compressor")
            .unwrap()
            .1
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect();
        let slot =
            deckard_model::SlotInfo { module: "Compressor".into(), name: "Compressor".into(), active: true, plugin: None, params: comp };
        p.groups[0].sounds[1].slots.push(slot);
        let at = p.groups[0].sounds[1].slots.len() - 1;
        p.groups[0].sounds[1].channel.sidechain = vec![(at, deckard_model::ScSource::Sound(0, 2))];
        let chains0 = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
        // (Audio mute: muting only the notes lets what plays ring out, its delay's echoes
        // too, so it isn't the same as muted all along.)
        for (audio_mute, ahead) in [(true, None), (true, Some(4096))] {
            let mut muted = p.clone();
            muted.groups[0].sounds[1].muted = true;
            muted.groups[0].sounds[1].channel.audio_mute = audio_mute;
            let out = render_with(&p, ahead, 256, 3.0, |k, e, cache| {
                if k != 40 {
                    return;
                }
                let (fresh, _) = Engine::new_after(&muted, Some((&p, chains0.as_slice())), None, 48_000, e.mode(), cache);
                drop(e.adopt(fresh, &muted));
            });
            let want = render_with(&muted, ahead, 256, 3.0, |_, _, _| {});
            let tail = |x: &[f32]| x[2 * 96_000..].to_vec();
            assert!(
                max_diff(&tail(&out), &tail(&want)) < 1e-3,
                "audio mute {audio_mute}, ahead {ahead:?}: {}",
                max_diff(&tail(&out), &tail(&want))
            );
        }
    }

    /// Two groups (BUGS #3): group 1's sound 1 has a Compressor side-chained from group
    /// 0's sound 2. Muting group 1 silences it, built muted and muted while it plays,
    /// exactly as with no side-chain.
    #[test]
    fn a_group_with_a_side_chain_from_another_mutes() {
        let (_tmp, mut p) = eight_project();
        let comp = deckard_model::INTERNAL_EFFECTS.iter().find(|(c, _)| *c == "Compressor").unwrap().1;
        let slot = deckard_model::SlotInfo {
            module: "Compressor".into(),
            name: "Compressor".into(),
            active: true,
            plugin: None,
            params: comp.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        };
        let b = p.groups[0].clone();
        p.groups.push(b);
        p.scenes[0].patterns.push((1, 0));
        p.groups[1].sounds[1].slots.push(slot);
        let at = p.groups[1].sounds[1].slots.len() - 1;
        p.groups[1].sounds[1].channel.sidechain = vec![(at, deckard_model::ScSource::Sound(0, 2))];
        let mut muted = p.clone();
        muted.groups[1].muted = true;
        let mut alone = muted.clone();
        alone.groups[1].sounds[1].channel.sidechain.clear();
        let want = render_with(&alone, None, 256, 2.0, |_, _, _| {});
        let built = render_with(&muted, None, 256, 2.0, |_, _, _| {});
        assert!(max_diff(&built, &want) < 1e-4, "built muted: {}", max_diff(&built, &want));
        let chains0 = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
        for ahead in [None, Some(4096)] {
            let out = render_with(&p, ahead, 256, 3.0, |k, e, cache| {
                if k != 40 {
                    return;
                }
                let (fresh, _) = Engine::new_after(&muted, Some((&p, chains0.as_slice())), None, 48_000, e.mode(), cache);
                drop(e.adopt(fresh, &muted));
            });
            let want = render_with(&alone, ahead, 256, 3.0, |_, _, _| {});
            let tail = |x: &[f32]| x[2 * 96_000..].to_vec();
            assert!(
                max_diff(&tail(&out), &tail(&want)) < 1e-3,
                "muted while playing, ahead {ahead:?}: {}",
                max_diff(&tail(&out), &tail(&want))
            );
        }
    }

    /// A sound whose effect listens to a side-chain plays live after an edit too (an
    /// edit keeps its chain unbuilt): with another group focused it isn't rendered
    /// ahead, where its effect would hear nothing ("Effects Bug": the Limiter on a
    /// hi-hat, side-chained from the kick, stopped once group A was selected).
    #[test]
    fn a_listening_sound_stays_live_after_an_edit() {
        let (_tmp, mut p) = eight_project();
        let comp = deckard_model::INTERNAL_EFFECTS.iter().find(|(c, _)| *c == "Compressor").unwrap().1;
        let b = p.groups[0].clone();
        p.groups.push(b);
        p.scenes[0].patterns.push((1, 0));
        p.groups[1].sounds[1].slots.push(deckard_model::SlotInfo {
            module: "Compressor".into(),
            name: "Compressor".into(),
            active: true,
            plugin: None,
            params: comp.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        });
        let at = p.groups[1].sounds[1].slots.len() - 1;
        p.groups[1].sounds[1].channel.sidechain = vec![(at, deckard_model::ScSource::Sound(1, 2))];
        let chains = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
        let run = |ahead: Option<u64>| {
            render_with(&p, ahead, 256, 3.0, |k, e, cache| {
                if k == 0 {
                    e.set_focus(Some(0));
                }
                if k == 40 {
                    let (fresh, _) = Engine::new_after(&p, Some((&p, chains.as_slice())), None, 48_000, e.mode(), cache);
                    drop(e.adopt(fresh, &p));
                    assert!(e.live_hint[1][1], "the listening sound still plays live");
                }
                if k == 60 && ahead.is_some() {
                    assert!(e.link_ahead(1, 1).is_none(), "not rendered ahead");
                    assert!(e.link_ahead(1, 3).is_some(), "(the group's others are)");
                }
            })
        };
        let inline = run(None);
        let ahead = run(Some(700));
        assert!(max_diff(&inline, &ahead) < 1e-4, "as without rendering ahead: {}", max_diff(&inline, &ahead));
    }

    /// Lemondrop1's shape (BUGS #3, #4): group 0's sound 1 has a Compressor side-chained
    /// from all of group 1, a group with effects of its own whose sound 3 has a Limiter
    /// side-chained from its sound 2.
    fn group_side_chain_project() -> (tempfile::TempDir, Project) {
        let (tmp, mut p) = eight_project();
        let fx = |class: &str| {
            let d = deckard_model::INTERNAL_EFFECTS.iter().find(|(c, _)| *c == class).unwrap().1;
            deckard_model::SlotInfo {
                module: class.into(),
                name: class.into(),
                active: true,
                plugin: None,
                params: d.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            }
        };
        let b = p.groups[0].clone();
        p.groups.push(b);
        p.scenes[0].patterns.push((1, 0));
        p.groups[1].slots = vec![fx("Saturator"), fx("Limiter")];
        p.groups[0].sounds[1].slots.push(fx("Compressor"));
        let at = p.groups[0].sounds[1].slots.len() - 1;
        p.groups[0].sounds[1].channel.sidechain = vec![(at, deckard_model::ScSource::Group(1))];
        p.groups[1].sounds[3].slots.push(fx("Limiter"));
        let at = p.groups[1].sounds[3].slots.len() - 1;
        p.groups[1].sounds[3].channel.sidechain = vec![(at, deckard_model::ScSource::Sound(1, 2))];
        (tmp, p)
    }

    /// BUGS #3: muting the listening group silences it, built muted and while playing.
    #[test]
    fn a_group_side_chained_from_a_group_mutes() {
        let (_tmp, p) = group_side_chain_project();
        let mut muted = p.clone();
        muted.groups[0].muted = true;
        let mut alone = muted.clone();
        alone.groups[0].sounds[1].channel.sidechain.clear();
        for ahead in [None, Some(4096)] {
            let want = render_with(&alone, ahead, 256, 2.0, |_, _, _| {});
            let built = render_with(&muted, ahead, 256, 2.0, |_, _, _| {});
            assert!(max_diff(&built, &want) < 1e-4, "built muted, ahead {ahead:?}: {}", max_diff(&built, &want));
        }
        let chains0 = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut SampleCache::default()).1.chains;
        for ahead in [None, Some(4096)] {
            let out = render_with(&p, ahead, 256, 3.0, |k, e, cache| {
                if k != 40 {
                    return;
                }
                let (fresh, _) = Engine::new_after(&muted, Some((&p, chains0.as_slice())), None, 48_000, e.mode(), cache);
                drop(e.adopt(fresh, &muted));
            });
            let want = render_with(&alone, ahead, 256, 3.0, |_, _, _| {});
            let tail = |x: &[f32]| x[2 * 96_000..].to_vec();
            assert!(
                max_diff(&tail(&out), &tail(&want)) < 1e-3,
                "muted while playing, ahead {ahead:?}: {}",
                max_diff(&tail(&out), &tail(&want))
            );
        }
    }

    /// BUGS #4: which group has the focus doesn't change what's heard.
    #[test]
    fn the_focus_doesnt_change_a_group_side_chain() {
        let (_tmp, p) = group_side_chain_project();
        let at = |focus: Option<usize>| {
            render_with(&p, Some(4096), 256, 2.0, |k, e, _| {
                if k == 0 {
                    e.set_focus(focus);
                }
            })
        };
        let none = at(None);
        for f in [Some(0), Some(1)] {
            assert!(max_diff(&at(f), &none) < 1e-4, "focus {f:?}: {}", max_diff(&at(f), &none));
        }
    }

    /// A side-chain left on a sound's instrument slot (as moving or adding slots used to
    /// leave one) doesn't stop the sound muting.
    #[test]
    fn a_side_chain_on_the_instrument_slot_still_mutes() {
        let (_tmp, mut p) = eight_project();
        p.groups[0].sounds[1].channel.sidechain = vec![(0, deckard_model::ScSource::Sound(0, 2))];
        for audio_mute in [true, false] {
            let mut muted = p.clone();
            muted.groups[0].sounds[1].muted = true;
            muted.groups[0].sounds[1].channel.audio_mute = audio_mute;
            let mut alone = muted.clone();
            alone.groups[0].sounds[1].channel.sidechain.clear();
            let a = render_with(&muted, None, 256, 2.0, |_, _, _| {});
            let b = render_with(&alone, None, 256, 2.0, |_, _, _| {});
            assert!(max_diff(&a, &b) < 1e-4, "audio mute {audio_mute}: {}", max_diff(&a, &b));
        }
    }

    /// A sound whose effect is side-chained from another still mutes: built muted, and
    /// muted while it plays.
    #[test]
    fn a_side_chained_sound_still_mutes() {
        let mut p = project();
        let click = p.groups[0].sounds[0].clone();
        p.groups[0].sounds.push(click);
        let slot = |module: &str, params: Vec<(String, f64)>| deckard_model::SlotInfo {
            module: module.into(),
            name: module.into(),
            active: true,
            plugin: None,
            params,
        };
        let comp: Vec<(String, f64)> = deckard_model::INTERNAL_EFFECTS
            .iter()
            .find(|(c, _)| *c == "Compressor")
            .unwrap()
            .1
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect();
        p.groups[0].sounds[0].slots = vec![slot("Sampler", Vec::new()), slot("Compressor", comp)];
        p.groups[0].sounds[0].channel.sidechain = vec![(1, deckard_model::ScSource::Sound(0, 1))];
        let mut muted = p.clone();
        muted.groups[0].sounds[0].muted = true;
        muted.groups[0].sounds[0].channel.audio_mute = true;
        let peak = |e: &mut Engine| {
            e.note_on(0, 0, 60, 127, 960);
            let mut out = vec![0f32; 2 * 4800];
            e.process(&mut out);
            out.iter().fold(0f32, |m, v| m.max(v.abs()))
        };
        let mut e = click_engine_for(&p, 48_000);
        e.sequencing = false;
        assert!(peak(&mut e) > 0.1, "it plays");
        let mut e = click_engine_for(&muted, 48_000);
        e.sequencing = false;
        assert!(peak(&mut e) < 1e-6, "built muted: silent");
        // Muted while playing (the app's edit).
        let mut e = click_engine_for(&p, 48_000);
        e.sequencing = false;
        peak(&mut e);
        e.update(&muted, None, &mut SampleCache::default());
        let mut out = vec![0f32; 2 * 48_000];
        e.note_on(0, 0, 60, 127, 960);
        e.process(&mut out);
        let tail = out[2 * 24_000..].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(tail < 1e-6, "muted while playing: silent ({tail})");
    }

    /// A sound set to an external input plays it through its channel (gain, level).
    #[test]
    fn sounds_play_their_audio_input() {
        let mut p = project();
        p.groups[0].sounds[0].channel.audio_in = deckard_model::AudioSource::Ext(1); // In 2 L+R
        let mut e = click_engine_for(&p, 1000);
        e.sequencing = false;
        let live: LiveIn = Default::default();
        e.live_in = Some(live.clone());
        assert!(e.takes_input());
        let mut frame = [0f32; 32];
        (frame[2], frame[3]) = (0.5, -0.25);
        live.lock().unwrap().extend(std::iter::repeat_n(frame, 4));
        let mut out = vec![0f32; 8];
        e.process(&mut out);
        assert!((out[0] - 0.5).abs() < 1e-4 && (out[1] + 0.25).abs() < 1e-4, "{out:?}");
        assert!(live.lock().unwrap().is_empty());
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
    };

    static SPLIT_INSTRUMENT: deckard_plugins::native::Class = deckard_plugins::native::Class {
        cid: [8; 16],
        name: "Split Instrument",
        instrument: true,
        params: &[],
        pages: &[],
        make: |_| Box::new(Split { instrument: true, ..Default::default() }),
    };

    pub(crate) fn split_plugin(class: &'static deckard_plugins::native::Class) -> Shared {
        let id = deckard_plugins::state::PluginId::parse(&class.plugin_id(), class.name).expect("an id");
        Arc::new(Mutex::new(deckard_plugins::Plugin::native(class, id, class.name, 1000.0)))
    }

    /// [`Split`] as an effect in `owner`'s slot of `p`, listening as the engine sets it.
    fn split_unit(owner: deckard_model::extra_out::Owner, p: &Project) -> Unit {
        let plugin = split_plugin(&SPLIT);
        let mut u = Unit::new(plugin_unit(plugin.clone(), &[], 1000.0), 0, Vec::new(), true);
        u.listen(owner, &plugin, &SPLIT.plugin_id(), p);
        u
    }

    /// A project whose only sound takes In 1, stopped, and an engine for it, with
    /// [`Split`] on the master (its first slot).
    fn split_engine(edit: impl FnOnce(&mut Project)) -> Engine {
        let mut p = project();
        p.groups[0].patterns[0].notes.clear();
        p.groups[0].sounds[0].channel.audio_in = deckard_model::AudioSource::Ext(0);
        edit(&mut p);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 1000, Mode::Arrangement, &mut cache);
        e.sequencing = false;
        e.live_in = Some(Default::default());
        let mut chain = bare_chain(vec![split_unit(deckard_model::extra_out::Owner::Master(0), &p)], 1000.0);
        chain.heard = Some(e.heard_board.clone());
        e.master_fx = Some(chain);
        e
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

    fn heard_now(e: &mut Engine, t: &mut Telemetry) -> Vec<Heard> {
        e.publish(t);
        t.heard.clone()
    }

    fn master_heard(bus: usize) -> Heard {
        Heard { owner: deckard_model::extra_out::Owner::Master(0), bus }
    }

    /// A bus feeding another runs first; buses feeding each other in a loop (which a
    /// project can be made to do) still get an order, each worked out once.
    #[test]
    fn bus_order_runs_feeders_first_and_survives_loops() {
        let bus = |to: Option<usize>| Bus {
            chain: bare_chain(Vec::new(), 48_000.0),
            buf: Vec::new(),
            route: (to.map(|g| GROUP_BUS + g), (1.0, 1.0)),
            ramp: GainRamp::default(),
        };
        // 0 → 1 → 2, then a ring 3 → 4 → … → 40 → 3.
        let mut buses: Vec<Option<Bus>> = vec![Some(bus(Some(1))), Some(bus(Some(2))), Some(bus(None))];
        buses.extend((3..=40).map(|g| Some(bus(Some(if g == 40 { 3 } else { g + 1 })))));
        let order = bus_order(&buses);
        assert_eq!(order.len(), 41);
        let at = |g: usize| order.iter().position(|&x| x == g).unwrap();
        assert!(at(0) < at(1) && at(1) < at(2), "{order:?}");
    }

    /// Heard pairs sort as the mixer shows strips: a group's own slots, then its sounds
    /// (instrument, then chain), the master's last.
    #[test]
    fn heard_pairs_sort_in_mixer_order() {
        use deckard_model::extra_out::Owner;
        let h = |owner, bus| Heard { owner, bus };
        let mut v = [
            h(Owner::Master(0), 1),
            h(Owner::Chain(1, 0, 0), 1),
            h(Owner::Sound(1, 0), 2),
            h(Owner::Group(1, 1), 1),
            h(Owner::Sound(0, 3), 1),
            h(Owner::Group(1, 0), 3),
        ];
        v.sort_by_key(heard_rank);
        let owners: Vec<Owner> = v.iter().map(|h| h.owner).collect();
        assert_eq!(
            owners,
            [Owner::Sound(0, 3), Owner::Group(1, 0), Owner::Group(1, 1), Owner::Sound(1, 0), Owner::Chain(1, 0, 0), Owner::Master(0)]
        );
    }

    /// Silence on a plug-in's extra pairs (just under -90 dBFS, too) isn't sound.
    #[test]
    fn a_silent_extra_bus_is_not_heard() {
        let mut e = split_engine(|_| {});
        let mut t = Telemetry::default();
        split_block(&mut e, 0.0);
        assert!(heard_now(&mut e, &mut t).is_empty());
        // A quarter of it on the third pair stays under the floor; half of it on the
        // second doesn't.
        split_block(&mut e, 1.2e-4);
        assert_eq!(heard_now(&mut e, &mut t), vec![master_heard(1)]);
    }

    /// Sound on two extra pairs: each is heard once, in order, however many blocks it
    /// plays before the UI takes them; then not again until it plays again.
    #[test]
    fn sound_on_an_extra_bus_is_heard_once() {
        let mut e = split_engine(|_| {});
        let mut t = Telemetry::default();
        split_block(&mut e, 0.5);
        split_block(&mut e, 0.5);
        assert_eq!(heard_now(&mut e, &mut t), vec![master_heard(1), master_heard(2)]);
        split_block(&mut e, 0.5);
        assert_eq!(heard_now(&mut e, &mut t), vec![master_heard(1), master_heard(2)], "still once each");
        assert!(t.heard.capacity() >= HEARD_ROOM, "room made once: the audio thread keeps it");
        t.heard.clear();
        split_block(&mut e, 0.0);
        assert!(heard_now(&mut e, &mut t).is_empty(), "taken, and quiet since");
        // The main pair goes on as ever.
        let (master, _) = split_block(&mut e, 0.5);
        assert!(about(master.0, 0.5) && about(master.1, 0.5), "{master:?}");
    }

    /// A pair that has a strip already isn't listened to; one with a strip for another
    /// plug-in (the slot's has changed) is.
    #[test]
    fn a_bus_with_a_strip_is_not_scanned() {
        use deckard_model::extra_out::{ExtraOut, Owner};
        let strip = |bus: usize, plugin: &str| ExtraOut {
            owner: Owner::Master(0),
            bus,
            plugin: plugin.into(),
            name: String::new(),
            level: 1.0,
            pan: 0.0,
            muted: false,
            dest: deckard_model::AudioDest::Parent,
            channels: Default::default(),
        };
        let mut e = split_engine(|p| p.extra_outs = vec![strip(1, &SPLIT.plugin_id()), strip(2, "another")]);
        assert_eq!(e.master_fx.as_ref().unwrap().units[0].scan, 1 << 2);
        let mut t = Telemetry::default();
        split_block(&mut e, 0.5);
        assert_eq!(heard_now(&mut e, &mut t), vec![master_heard(2)]);
        let mut p = project();
        p.extra_outs = vec![strip(1, &SPLIT.plugin_id()), strip(2, &SPLIT.plugin_id())];
        assert_eq!(split_unit(Owner::Master(0), &p).scan, 0);
        assert_eq!(split_unit(Owner::Master(1), &p).scan, 0b110, "another slot's strips");
    }

    /// A strip added or removed for a sound's slot (not one moved) makes its parts anew
    /// on an edit, so they listen to the pairs left.
    #[test]
    fn a_new_strip_remakes_what_is_heard() {
        use deckard_model::extra_out::{ExtraOut, Owner};
        let a = project();
        let mut b = a.clone();
        let strip = ExtraOut {
            owner: Owner::Chain(0, 0, 1),
            bus: 1,
            plugin: "x".into(),
            name: String::new(),
            level: 1.0,
            pan: 0.0,
            muted: false,
            dest: deckard_model::AudioDest::Parent,
            channels: Default::default(),
        };
        b.extra_outs = vec![strip.clone()];
        let r = reuse_of(&a, &b, 0, 0, 1000.0, &deckard_plugins::Host::default());
        assert!(!r.chain && r.instrument);
        b.extra_outs[0].owner = Owner::Sound(0, 0);
        let r = reuse_of(&a, &b, 0, 0, 1000.0, &deckard_plugins::Host::default());
        assert!(r.chain && !r.instrument);
        let mut c = b.clone();
        c.extra_outs[0].level = 0.5;
        let r = reuse_of(&b, &c, 0, 0, 1000.0, &deckard_plugins::Host::default());
        assert!(r.chain && r.instrument, "a level is no new strip");
    }

    /// A bypassed plug-in makes no sound, so none is heard.
    #[test]
    fn a_bypassed_slot_is_not_heard() {
        let mut e = split_engine(|_| {});
        e.master_fx.as_mut().unwrap().units[0].mix = 0.0;
        let mut t = Telemetry::default();
        split_block(&mut e, 0.5);
        split_block(&mut e, 0.5);
        assert!(heard_now(&mut e, &mut t).is_empty());
    }

    /// An instrument's extra pair is heard as its sound's; its silent one isn't.
    #[test]
    fn an_instruments_extra_bus_is_heard() {
        use deckard_model::extra_out::Owner;
        let mut e = split_engine(|_| {});
        let p = project();
        let plugin = split_plugin(&SPLIT_INSTRUMENT);
        let mut inst = InstrumentRt::new(plugin.clone(), (1.0, 1.0), false);
        inst.listen(Owner::Sound(0, 0), &plugin, &SPLIT_INSTRUMENT.plugin_id(), &p);
        inst.heard = Some(e.heard_board.clone());
        e.instruments[0][0] = Some(inst);
        e.master_fx = None;
        let mut t = Telemetry::default();
        split_block(&mut e, 0.0);
        assert_eq!(heard_now(&mut e, &mut t), vec![Heard { owner: Owner::Sound(0, 0), bus: 2 }]);
    }

    /// A plug-in with one output pair has nothing to listen to, and nothing is heard.
    #[test]
    fn a_one_pair_plugin_reports_nothing() {
        use deckard_model::extra_out::Owner;
        let mut e = split_engine(|_| {});
        let class = deckard_plugins::native::CLASSES[0];
        let plugin = split_plugin(class);
        let p = project();
        let mut u = Unit::new(plugin_unit(plugin.clone(), &[], 1000.0), 0, Vec::new(), true);
        u.listen(Owner::Master(0), &plugin, &class.plugin_id(), &p);
        assert_eq!(u.scan, 0);
        e.master_fx.as_mut().unwrap().units[0] = u;
        let mut inst = InstrumentRt::new(plugin.clone(), (1.0, 1.0), false);
        inst.listen(Owner::Sound(0, 0), &plugin, &class.plugin_id(), &p);
        assert_eq!(inst.scan, 0);
        let mut t = Telemetry::default();
        split_block(&mut e, 0.5);
        assert!(heard_now(&mut e, &mut t).is_empty());
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

    fn near_pair(a: (f32, f32), b: (f32, f32)) -> bool {
        about(a.0, b.0) && about(a.1, b.1)
    }

    /// A strip onto Out 3: its pair plays there (a quarter of the input: half of it at
    /// unity), metered; the master has only the main pair.
    #[test]
    fn a_strip_on_ext_reaches_that_output() {
        let mut x = Strips::new(|_| {});
        let (master, out3) = x.block(0.5);
        assert!(near_pair(out3, (0.25, 0.25)), "Out 3: {out3:?}");
        assert!(near_pair(master, (0.5, 0.5)), "the master: {master:?}");
        let mut t = Telemetry::default();
        x.e.publish(&mut t);
        assert_eq!(t.extra_meters.len(), 1);
        assert!(near_pair(t.extra_meters[0], (0.25, 0.25)), "{:?}", t.extra_meters);
    }

    /// A strip's level and pan are its own: the owner's fader and the master's don't
    /// move it, and changing them doesn't make the chain anew.
    #[test]
    fn a_strips_level_and_pan_are_its_own() {
        let mut x = Strips::new(|p| (p.extra_outs[0].level, p.extra_outs[0].pan) = (0.5, -1.0));
        let left = 0.25 * 0.5 * std::f32::consts::SQRT_2;
        assert!(near_pair(x.block(0.5).1, (left, 0.0)), "{:?}", x.block(0.5));
        let buf = x.e.fx[0][0].as_ref().unwrap().units[0].extras[0].buf.as_ptr();
        x.edit(|p| (p.groups[0].sounds[0].level, p.groups[0].level, p.master_level) = (0.3, 0.4, 0.2));
        x.block(0.5);
        let (master, out3) = x.block(0.5);
        assert!(near_pair(out3, (left, 0.0)), "the faders above don't move it: {out3:?}");
        assert!(master.0 < 0.1, "they move the main pair: {master:?}");
        x.edit(|p| (p.extra_outs[0].level, p.extra_outs[0].pan) = (1.0, 0.0));
        x.block(0.5);
        assert!(near_pair(x.block(0.5).1, (0.25, 0.25)), "its own do");
        assert_eq!(x.e.fx[0][0].as_ref().unwrap().units[0].extras[0].buf.as_ptr(), buf, "the chain was kept");
    }

    /// A strip into another group goes through that group's fader (and on to where it
    /// goes: the master).
    #[test]
    fn a_strip_into_a_group_goes_through_that_group() {
        let mut x = Strips::new(|p| {
            p.groups.push(Group { level: 0.5, ..Default::default() });
            p.extra_outs[0].dest = deckard_model::AudioDest::Group(1);
        });
        let (master, out3) = x.block(0.5);
        let g = deckard_model::level_gain(0.5) as f32;
        assert!(near_pair(out3, (0.0, 0.0)));
        assert!(near_pair(master, (0.5 + 0.25 * g, 0.5 + 0.25 * g)), "{master:?}");
    }

    /// A strip into a sound goes through that sound's fader, then where it goes.
    #[test]
    fn a_strip_into_a_sound_goes_through_that_sound() {
        let mut x = Strips::new(|p| {
            p.groups[0].sounds.push(Sound { level: 0.5, ..Default::default() });
            p.extra_outs[0].dest = deckard_model::AudioDest::Sound(0, 1);
        });
        let (master, out3) = x.block(0.5);
        let s = deckard_model::level_gain(0.5) as f32;
        assert!(near_pair(out3, (0.0, 0.0)));
        assert!(near_pair(master, (0.5 + 0.25 * s, 0.5 + 0.25 * s)), "{master:?}");
    }

    /// A strip on an output the interface doesn't have connected plays through its
    /// parent (here its group, onto the master), until the output is connected.
    #[test]
    fn an_unconnected_ext_falls_back_to_the_parent() {
        let mut x = Strips::new(|_| {});
        x.e.set_connected_outputs(0b1);
        let (master, out3) = x.block(0.5);
        assert!(near_pair(out3, (0.0, 0.0)), "{out3:?}");
        assert!(near_pair(master, (0.75, 0.75)), "{master:?}");
        x.e.set_connected_outputs(0b101);
        let (master, out3) = x.block(0.5);
        assert!(near_pair(out3, (0.25, 0.25)) && near_pair(master, (0.5, 0.5)), "{master:?} {out3:?}");
    }

    /// A muted sound, or a muted group, keeps its plug-in running for an audible strip:
    /// only its main pair goes silent. The same for a group's own plug-in.
    #[test]
    fn a_muted_owner_keeps_its_plugin_running_for_its_strip() {
        use deckard_model::extra_out::Owner;
        for audio_mute in [false, true] {
            let mut x = Strips::new(|p| {
                p.groups[0].sounds[0].muted = true;
                p.groups[0].sounds[0].channel.audio_mute = audio_mute;
            });
            let (master, out3) = x.block(0.5);
            assert!(near_pair(master, (0.0, 0.0)) && near_pair(out3, (0.25, 0.25)), "sound (audio mute {audio_mute}): {master:?} {out3:?}");
        }
        let mut x = Strips::new(|p| p.groups[0].muted = true);
        let (master, out3) = x.block(0.5);
        assert!(near_pair(master, (0.0, 0.0)) && near_pair(out3, (0.25, 0.25)), "its group muted: {master:?} {out3:?}");
        // The group's own plug-in (its sounds go into it as ever).
        let mut x = Strips::new(|p| {
            p.groups[0].sounds[0].slots.clear();
            p.groups[0].slots = vec![plugin_slot(&SPLIT)];
            p.extra_outs = vec![split_strip(Owner::Group(0, 0), 1, deckard_model::AudioDest::Ext(2))];
            p.groups[0].muted = true;
        });
        let (master, out3) = x.block(0.5);
        assert!(near_pair(master, (0.0, 0.0)) && near_pair(out3, (0.25, 0.25)), "the group's plug-in: {master:?} {out3:?}");
        // Unmuted again: as before.
        x.edit(|p| p.groups[0].muted = false);
        x.block(0.5);
        let (master, out3) = x.block(0.5);
        assert!(near_pair(master, (0.5, 0.5)) && near_pair(out3, (0.25, 0.25)), "unmuted: {master:?} {out3:?}");
        // Without a strip that sounds, a muted owner is silent as ever.
        let mut x = Strips::new(|p| {
            p.groups[0].sounds[0].muted = true;
            p.groups[0].sounds[0].channel.audio_mute = true;
            p.extra_outs[0].muted = true;
        });
        assert!(x.e.in_routes.is_empty(), "not played");
        assert!(near_pair(x.block(0.5).0, (0.0, 0.0)));
    }

    /// [`split_block`] with `(l, r)` on In 1.
    fn stereo_block(e: &mut Engine, l: f32, r: f32) -> ((f32, f32), Vec<(f32, f32)>) {
        let mut frame = [0f32; 32];
        (frame[0], frame[1]) = (l, r);
        e.live_in.as_ref().unwrap().lock().unwrap().extend(std::iter::repeat_n(frame, 64));
        let mut out = vec![0f32; 128];
        let mut ext = vec![vec![0f32; 128]; OUTPUTS];
        e.process_outputs(&mut out, &mut ext);
        ((out[126], out[127]), ext.iter().map(|b| (b[126], b[127])).collect())
    }

    /// A mono strip plays (L+R)/2 on both sides, which its pan places as a channel's
    /// pan places a sound.
    #[test]
    fn a_mono_strip_sums_both_sides() {
        use deckard_model::extra_out::Channels;
        let mut x = Strips::new(|p| p.extra_outs[0].channels = Channels::Mono);
        // In 1 (0.8, 0.2): the pair is half of it, (0.4, 0.1); its sum halved, 0.25.
        let (master, ext) = stereo_block(&mut x.e, 0.8, 0.2);
        assert!(near_pair(ext[2], (0.25, 0.25)), "Out 3: {:?}", ext[2]);
        assert!(near_pair(master, (0.8, 0.2)), "the main pair as it is: {master:?}");
        x.edit(|p| p.extra_outs[0].pan = -1.0);
        stereo_block(&mut x.e, 0.8, 0.2);
        let (_, ext) = stereo_block(&mut x.e, 0.8, 0.2);
        assert!(near_pair(ext[2], (0.25 * std::f32::consts::SQRT_2, 0.0)), "panned left: {:?}", ext[2]);
        // Stereo again: each side its own.
        x.edit(|p| (p.extra_outs[0].pan, p.extra_outs[0].channels) = (0.0, Channels::Stereo));
        stereo_block(&mut x.e, 0.8, 0.2);
        let (_, ext) = stereo_block(&mut x.e, 0.8, 0.2);
        assert!(near_pair(ext[2], (0.4, 0.1)), "stereo: {:?}", ext[2]);
    }

    /// The halves of a split strip each play one side of the pair on both sides, with
    /// their own level and output, and meter apart; split while playing, too.
    #[test]
    fn split_strip_halves_carry_one_side_each() {
        use deckard_model::extra_out::{self, Channels};
        let mut x = Strips::new(|_| {});
        let (_, ext) = stereo_block(&mut x.e, 0.8, 0.2);
        assert!(near_pair(ext[2], (0.4, 0.1)), "stereo first: {:?}", ext[2]);
        x.edit(|p| {
            extra_out::split(&mut p.extra_outs, 0);
            p.extra_outs[1].level = 0.5;
            p.extra_outs[1].dest = deckard_model::AudioDest::Ext(3);
        });
        assert_eq!(x.p.extra_outs.iter().map(|e| e.channels).collect::<Vec<_>>(), vec![Channels::Left, Channels::Right]);
        stereo_block(&mut x.e, 0.8, 0.2);
        let (master, ext) = stereo_block(&mut x.e, 0.8, 0.2);
        assert!(near_pair(ext[2], (0.4, 0.4)), "L on Out 3: {:?}", ext[2]);
        assert!(near_pair(ext[3], (0.05, 0.05)), "R at half on Out 4: {:?}", ext[3]);
        assert!(near_pair(master, (0.8, 0.2)), "the main pair: {master:?}");
        let mut t = Telemetry::default();
        x.e.publish(&mut t);
        assert_eq!(t.extra_meters.len(), 2);
        assert!(near_pair(t.extra_meters[0], (0.4, 0.4)) && near_pair(t.extra_meters[1], (0.05, 0.05)), "{:?}", t.extra_meters);
        // Joined: stereo again, with the left half's settings.
        x.edit(|p| extra_out::join(&mut p.extra_outs, 1));
        stereo_block(&mut x.e, 0.8, 0.2);
        let (_, ext) = stereo_block(&mut x.e, 0.8, 0.2);
        assert!(near_pair(ext[2], (0.4, 0.1)) && near_pair(ext[3], (0.0, 0.0)), "{:?} {:?}", ext[2], ext[3]);
    }

    /// A muted strip is silent; the main pair plays on.
    #[test]
    fn a_muted_strip_is_silent() {
        let mut x = Strips::new(|p| p.extra_outs[0].muted = true);
        let (master, out3) = x.block(0.5);
        assert!(near_pair(out3, (0.0, 0.0)) && near_pair(master, (0.5, 0.5)), "{master:?} {out3:?}");
    }

    /// Taking the plug-in away (its strip with it, or left behind) while playing: no
    /// panic, nothing more on its output, and the meters follow.
    #[test]
    fn removing_the_plugin_drops_its_strip_without_panic() {
        for keep_strip in [false, true] {
            let mut x = Strips::new(|_| {});
            x.block(0.5);
            x.edit(|p| {
                p.groups[0].sounds[0].slots.clear();
                if !keep_strip {
                    p.extra_outs.clear();
                }
            });
            x.block(0.5);
            let (master, out3) = x.block(0.5);
            assert!(near_pair(out3, (0.0, 0.0)) && near_pair(master, (0.5, 0.5)), "{master:?} {out3:?}");
            let mut t = Telemetry::default();
            x.e.publish(&mut t);
            assert_eq!(t.extra_meters.len(), x.p.extra_outs.len());
            assert!(t.extra_meters.iter().all(|m| *m == (0.0, 0.0)), "{:?}", t.extra_meters);
        }
    }

    /// A plug-in with one output pair plays exactly as it did, with or without a strip
    /// claiming a pair it doesn't have.
    #[test]
    fn a_one_pair_plugin_sounds_unchanged() {
        use deckard_model::extra_out::Owner;
        let class = deckard_plugins::native::CLASSES[0];
        let render = |strip: bool| {
            let mut p = project();
            let snd = &mut p.groups[0].sounds[0];
            snd.sampler = None;
            snd.plugin = plugin_slot(class).plugin;
            snd.slots = vec![plugin_slot(class)];
            if strip {
                let mut x = split_strip(Owner::Sound(0, 0), 1, deckard_model::AudioDest::Ext(2));
                x.plugin = class.plugin_id();
                p.extra_outs = vec![x];
            }
            let mut cache = SampleCache::default();
            cache.test_plugins.push((Slot::Sound { group: 0, sound: 0 }, split_plugin(class)));
            let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
            let (mut all, mut others) = (Vec::new(), 0f32);
            for _ in 0..40 {
                let mut out = vec![0f32; 512];
                let mut ext = vec![vec![0f32; 512]; OUTPUTS];
                e.process_outputs(&mut out, &mut ext);
                all.extend_from_slice(&out);
                others = ext.iter().flatten().fold(others, |m, v| m.max(v.abs()));
            }
            (all, others)
        };
        let (plain, _) = render(false);
        let (with, others) = render(true);
        assert!(plain.iter().any(|v| v.abs() > 1e-3), "it plays");
        assert_eq!(plain, with);
        assert_eq!(others, 0.0);
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

    /// The cue bus's meter reads what's on it, at its level; a channel not on it, nothing.
    #[test]
    fn the_cue_meter_reads_what_is_on_the_bus() {
        let mut p = input_project();
        p.cue_output = deckard_model::AudioDest::Ext(1);
        p.cue_level = deckard_model::level_from_db(-6.0);
        let g = deckard_model::level_gain(p.cue_level) as f32;
        let mut e = input_project_engine(&p);
        e.sequencing = false;
        steady_outs(&mut e, 80);
        assert_eq!(e.cue.peak, (0.0, 0.0), "nothing on it");
        p.groups[0].sounds[0].cue = true;
        let mut e = input_project_engine(&p);
        e.sequencing = false;
        let (master, cue) = steady_outs(&mut e, 80);
        assert!(near(master, 0.0) && near(cue, 0.5 * g), "on Out 2: {master} {cue}");
        assert!(near(e.cue.peak.0, 0.5 * g) && near(e.cue.peak.1, 0.5 * g), "{:?}", e.cue.peak);
    }

    /// On the master's output, the meter shows the bus, not what's divided by the master's
    /// gain to get past it.
    #[test]
    fn the_cue_meter_is_past_the_masters_gain() {
        let mut p = input_project();
        p.master_level = deckard_model::level_from_db(-12.0);
        p.groups[0].sounds[0].cue = true;
        let mut e = input_project_engine(&p);
        e.sequencing = false;
        steady(&mut e, 80);
        assert!(near(e.cue.peak.0, 0.5), "{:?}", e.cue.peak);
    }

    /// A group is on the bus through its sounds, or with effects, its own bus; a sound
    /// through a group with effects on the bus isn't itself.
    #[test]
    fn what_is_on_the_cue_bus_follows_the_routes() {
        use deckard_model::AudioDest as D;
        let mut p = input_project();
        p.groups[0].cue = true;
        assert!(CueBus::of(&p).sounds[0][0], "through its group");
        assert!(CueBus::of(&p).groups[0]);
        p.groups[0].slots = vec![driven_saturator()];
        assert!(!CueBus::of(&p).sounds[0][0], "its group's bus is");
        assert!(CueBus::of(&p).groups[0]);
        p.groups[0].slots.clear();
        p.groups[0].cue = false;
        p.groups[0].sounds[0].channel.dest = D::Master;
        assert!(!CueBus::of(&p).sounds[0][0]);
        let mut e = input_project_engine(&p);
        e.sequencing = false;
        steady(&mut e, 80);
        assert_eq!(e.cue.peak, (0.0, 0.0));
        // With effects and on the bus: metered after them.
        p.groups[0].sounds[0].channel.dest = D::Parent;
        p.groups[0].slots = vec![driven_saturator()];
        p.groups[0].cue = true;
        let mut e = input_project_engine(&p);
        e.sequencing = false;
        steady(&mut e, 400);
        assert!(e.cue.peak.0 > 0.1, "{:?}", e.cue.peak);
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

    /// The metronome and the prehear are on the cue bus: its output, its level, its meter.
    #[test]
    fn the_prehear_is_on_the_cue_bus() {
        let mut p = input_project();
        p.cue_output = deckard_model::AudioDest::Ext(1);
        p.cue_level = deckard_model::level_from_db(-6.0);
        let g = deckard_model::level_gain(p.cue_level) as f32;
        let mut e = input_project_engine(&p);
        e.sequencing = false;
        let (master, cue) = hear(&mut e, 20);
        assert_eq!(master, 0.0);
        assert!(near(cue, 0.5 * g), "{cue}");
        assert!(near(e.cue.peak.0, 0.5 * g), "{:?}", e.cue.peak);
    }

    /// The metronome goes the same way.
    #[test]
    fn the_click_is_on_the_cue_bus() {
        let mut p = input_project();
        p.cue_output = deckard_model::AudioDest::Ext(1);
        // (At a rate a click can be heard at.)
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        e.metronome = true;
        let mut master = 0f32;
        let mut cue = 0f32;
        for _ in 0..40 {
            let mut out = vec![0f32; 16];
            let mut ext = vec![vec![0f32; 16]; 2];
            e.process_outputs(&mut out, &mut ext);
            master = out.iter().fold(master, |m, v| m.max(v.abs()));
            cue = ext[1].iter().fold(cue, |m, v| m.max(v.abs()));
        }
        assert_eq!(master, 0.0);
        assert!(cue > 0.01, "{cue}");
        assert!(e.cue.peak.0 > 0.01, "{:?}", e.cue.peak);
    }

    /// With the cue on the master's output (the default), they sound there, the master's
    /// fader not in the way.
    #[test]
    fn the_prehear_on_the_masters_output_is_past_its_fader() {
        let mut p = input_project();
        p.master_level = deckard_model::level_from_db(-12.0);
        let mut e = input_project_engine(&p);
        e.sequencing = false;
        let (master, _) = hear(&mut e, 20);
        assert!(near(master, 0.5), "{master}");
    }

    /// A cue output the interface doesn't have: they go on being heard, on the master's.
    #[test]
    fn the_prehear_falls_back_to_the_masters_output() {
        let mut p = input_project();
        p.cue_output = deckard_model::AudioDest::Ext(7);
        let mut e = input_project_engine(&p);
        e.sequencing = false;
        let (master, _) = hear(&mut e, 20);
        assert!(master > 0.1, "{master}");
        p.cue_output = deckard_model::AudioDest::None;
        let mut e = input_project_engine(&p);
        e.sequencing = false;
        let (master, _) = hear(&mut e, 20);
        assert!(near(master, 0.5), "no cue output at all: {master}");
    }

    /// A sound's level follows its lane while the pattern plays, from its fader's place;
    /// stopped, it's the fader's own again.
    #[test]
    fn a_lane_moves_a_sounds_level() {
        let mut p = fader_project();
        p.groups[0].sounds[0].modulators = vec![fader_lane("level")];
        p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "level", 0, -0.25)];
        assert_eq!(sound_gain(&p, 0, 0), (1.0, 1.0), "its fader isn't in the route");
        let mut e = fader_engine(&p);
        let want = 0.5 * deckard_model::level_gain(0.5) as f32;
        assert!(near(steady(&mut e, 100).0, want), "the lane: {want}");
        e.sequencing = false;
        assert!(near(steady(&mut e, 100).0, 0.5), "stopped: the fader's own");
    }

    /// Points are joined by lines, and the lane holds only where its pattern plays.
    #[test]
    fn a_lane_is_followed_along_the_pattern() {
        let mut p = fader_project();
        p.groups[0].sounds[0].modulators = vec![fader_lane("level")];
        p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "level", 0, 0.0), fader_point(Some(0), "level", 2 * PPQ, -0.5)];
        let t = timeline(&p, Mode::Arrangement);
        assert_eq!(t.faders.len(), 2, "a pass per bar of the section: {:?}", t.faders);
        assert_eq!((t.faders[1].from, t.faders[1].to), (4 * PPQ, 8 * PPQ));
        let mut e = fader_engine(&p);
        // A quarter is 500 frames at 120 BPM and 1000 Hz: there, half way (a little behind
        // it, gliding); past the last point, it holds.
        let halfway = steady(&mut e, 500).0;
        assert!((halfway - 0.5 * deckard_model::level_gain(0.5) as f32).abs() < 0.02, "{halfway}");
        let held = steady(&mut e, 600).0;
        assert!(near(held, 0.5 * deckard_model::level_gain(0.25) as f32), "{held}");
    }

    /// A fader at the bottom still rises with its lane (no gain to scale from).
    #[test]
    fn a_lane_lifts_a_silent_fader() {
        let mut p = fader_project();
        p.groups[0].sounds[0].level = 0.0;
        p.groups[0].sounds[0].modulators = vec![fader_lane("level")];
        p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "level", 0, 0.75)];
        let mut e = fader_engine(&p);
        assert!(near(steady(&mut e, 100).0, 0.5));
        e.sequencing = false;
        assert!(near(steady(&mut e, 100).0, 0.0));
    }

    /// Pan's lane: a deviation of its knob's travel, which covers −1..1.
    #[test]
    fn a_lane_moves_a_sounds_pan() {
        let mut p = fader_project();
        p.groups[0].sounds[0].modulators = vec![fader_lane("pan")];
        p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "pan", 0, 0.5)];
        let mut e = fader_engine(&p);
        let (l, r) = steady(&mut e, 100);
        assert!(l.abs() < 1e-4 && r > 0.5, "right: {l} {r}");
        e.sequencing = false;
        let (l, r) = steady(&mut e, 100);
        assert!(near(l, r), "the middle again: {l} {r}");
    }

    /// A group's lane moves the sounds that go through it, and with effects, its bus.
    #[test]
    fn a_lane_moves_a_groups_level() {
        let mut p = fader_project();
        p.groups[0].modulators = vec![fader_lane("level")];
        p.groups[0].patterns[0].mods = vec![fader_point(None, "level", 0, -0.25)];
        let mut e = fader_engine(&p);
        assert_eq!(e.sound_faders[0][0], [0], "no bus: on the sound's way out");
        let want = 0.5 * deckard_model::level_gain(0.5) as f32;
        assert!(near(steady(&mut e, 100).0, want));
        e.sequencing = false;
        assert!(near(steady(&mut e, 100).0, 0.5));
        // With effects, it's after them.
        p.groups[0].slots = vec![driven_saturator()];
        let mut e = fader_engine(&p);
        assert!(e.sound_faders[0][0].is_empty());
        assert_eq!(e.bus_faders[0], [0]);
        let moved = steady(&mut e, 500).0;
        e.sequencing = false;
        let own = steady(&mut e, 500).0;
        assert!((moved / own - deckard_model::level_gain(0.5) as f32).abs() < 1e-3, "{moved} / {own}");
    }

    /// Only the faders a sound's way out passes through move it.
    #[test]
    fn a_fader_off_the_route_leaves_a_sound_alone() {
        use deckard_model::AudioDest as D;
        let mut p = fader_project();
        p.groups[0].modulators = vec![fader_lane("level")];
        p.groups[0].sounds.push(Sound { modulators: vec![fader_lane("level")], ..Default::default() });
        // Past its group, to a bus sound with a lane.
        p.groups[0].sounds[0].channel.dest = D::Sound(0, 1);
        p.groups[0].sounds[1].channel.dest = D::Master;
        let (faders, sounds, _) = channel_faders(&p);
        assert_eq!(faders.iter().map(|f| (f.group, f.sound)).collect::<Vec<_>>(), [(0, None), (0, Some(1))]);
        assert_eq!(sounds[0][0], [1], "the bus sound's, not the group's");
        // On the cue bus: none of them.
        p.groups[0].sounds[0].cue = true;
        assert!(channel_faders(&p).1[0][0].is_empty());
    }

    /// With no lane, nothing changes: the fader's in the route as ever.
    #[test]
    fn a_fader_with_no_lane_stays_in_the_route() {
        let mut p = fader_project();
        p.groups[0].sounds[0].level = 0.5;
        let want = deckard_model::level_gain(0.5) as f32;
        assert!(near(sound_gain(&p, 0, 0).0, want));
        let e = fader_engine(&p);
        assert!(e.faders.is_empty());
        // Another of the channel's properties modulated isn't its fader.
        p.groups[0].sounds[0].modulators = vec![fader_lane("aux1_level")];
        assert!(near(sound_gain(&p, 0, 0).0, want));
    }

    /// A fader moved during play: the lane plays on from its new place, with the same
    /// timeline.
    #[test]
    fn a_moved_fader_keeps_its_lane() {
        let mut p = fader_project();
        p.groups[0].sounds[0].modulators = vec![fader_lane("level")];
        p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "level", 0, -0.25)];
        let mut cache = SampleCache::default();
        let (mut e, built) = Engine::new(&p, None, 1000, Mode::Arrangement, &mut cache);
        e.live_in = Some(Default::default());
        e.looping = true;
        steady(&mut e, 100);
        let mut moved = p.clone();
        moved.groups[0].sounds[0].level = 0.5;
        let (fresh, _) = Engine::new_after(&moved, Some((&p, built.chains.as_slice())), None, 1000, Mode::Arrangement, &mut cache);
        assert!(fresh.keeps_timelines(), "nothing that makes the timeline changed");
        drop(e.adopt(fresh, &moved));
        assert!(near(steady(&mut e, 100).0, 0.5 * deckard_model::level_gain(0.25) as f32));
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

    /// A sound taking audio in with effects (and nothing else in its chain): the input
    /// goes through them.
    #[test]
    fn an_input_sound_plays_through_its_effects() {
        let mut dry = vec![0f32; 128];
        input_engine(Vec::new(), deckard_model::AudioDest::Parent).process(&mut dry);
        assert!(dry[100].abs() > 0.1, "dry: {dry:?}");
        let mut wet = vec![0f32; 128];
        input_engine(vec![driven_saturator()], deckard_model::AudioDest::Parent).process(&mut wet);
        assert!(max_diff(&dry, &wet) > 0.05, "the Saturator changes it: {dry:?} / {wet:?}");
    }

    /// A sound whose chain starts with an instrument takes no audio in: it would play
    /// through the instrument's chain (and click).
    #[test]
    fn an_instrument_sound_takes_no_input() {
        let sampler =
            deckard_model::SlotInfo { module: "Sampler".into(), name: "Sampler".into(), active: true, plugin: None, params: Vec::new() };
        let mut e = input_engine(vec![sampler, driven_saturator()], deckard_model::AudioDest::Parent);
        assert!(!e.takes_input());
        let mut out = vec![0f32; 128];
        e.process(&mut out);
        assert!(out.iter().all(|x| x.abs() < 1e-6), "{out:?}");
    }

    /// A sound taking audio in plays where it's routed, not straight to the master.
    #[test]
    fn an_input_sound_plays_on_its_output() {
        let mut e = input_engine(Vec::new(), deckard_model::AudioDest::Ext(1));
        let (mut out, mut ext) = (vec![0f32; 128], vec![vec![0f32; 128]; 4]);
        e.process_outputs(&mut out, &mut ext);
        assert!(out.iter().all(|x| x.abs() < 1e-6), "not on the master: {out:?}");
        assert!(ext[1][100].abs() > 0.1, "on Out 2: {:?}", ext[1]);
    }

    /// A sound sends its notes to its MIDI output: its channel, transposed, with
    /// note-offs at the note's end; muted sounds send nothing.
    #[test]
    fn sounds_send_midi() {
        let mut p = project();
        p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
        p.groups[0].sounds[0].channel.midi_out_channel = 2;
        p.groups[0].sounds[0].channel.transpose = -12;
        let mut e = click_engine_for(&p, 1000);
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        e.midi_tx = Some(tx);
        let mut out = vec![0f32; 2 * 1000];
        e.process(&mut out);
        let msgs: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
        // 120 BPM: two beats a second; notes last a sixteenth (125 ms).
        assert_eq!(msgs, vec![vec![0x92, 48, 127], vec![0x82, 48, 0], vec![0x92, 48, 127], vec![0x82, 48, 0]]);
        // Live input with thru: out too; release ends it.
        e.note_in(0, 0, 64, 90, true);
        e.note_release(0, 0, 64);
        let msgs: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
        assert_eq!(msgs, vec![vec![0x92, 52, 90], vec![0x82, 52, 0]]);
        // Muted: nothing.
        p.groups[0].sounds[0].muted = true;
        let mut e = click_engine_for(&p, 1000);
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        e.midi_tx = Some(tx);
        e.process(&mut out);
        assert_eq!(rx.try_iter().count(), 0);
    }

    /// An engine at 1000 Hz (120 BPM: a beat is 500 frames) playing one note, C3 for
    /// `length` ticks with expression `e`, out to port "Synth" (channel 3), and what it
    /// sends.
    fn expr_engine(e: deckard_model::Expression, length: i64) -> (Engine, std::sync::mpsc::Receiver<midi::MidiOut>) {
        let mut p = project();
        p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
        p.groups[0].sounds[0].channel.midi_out_channel = 2;
        let n = Note { expr: Some(deckard_model::ExprId(0)), sound: 0, position: 0, length, pitch: 60, velocity: 100, probability: 100 };
        p.groups[0].patterns[0].notes = vec![n];
        p.expressions = vec![std::sync::Arc::new(e)];
        let mut engine = click_engine_for(&p, 1000);
        let (tx, rx) = std::sync::mpsc::sync_channel(4096);
        engine.midi_tx = Some(tx);
        (engine, rx)
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

    /// An engine like [`expr_engine`] playing `notes` (each with its expression in
    /// `exprs`, by index) out to "Synth" set to MPE (bend range 48).
    fn mpe_engine(notes: Vec<Note>, exprs: Vec<deckard_model::Expression>) -> (Engine, std::sync::mpsc::Receiver<midi::MidiOut>) {
        let (mut engine, rx) = expr_engine(deckard_model::Expression::default(), PPQ);
        let mut p = project();
        p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
        p.groups[0].sounds[0].channel.midi_out_channel = 2;
        p.groups[0].patterns[0].notes = notes;
        p.expressions = exprs.into_iter().map(std::sync::Arc::new).collect();
        let tx = engine.midi_tx.take();
        engine = click_engine_for(&p, 1000);
        engine.midi_tx = tx;
        engine.set_mpe_out([("Synth".to_string(), 48)].into());
        (engine, rx)
    }

    fn note_at(position: i64, length: i64, pitch: i64, expr: Option<u32>) -> Note {
        Note { expr: expr.map(deckard_model::ExprId), sound: 0, position, length, pitch, velocity: 100, probability: 100 }
    }

    fn sent(e: &mut Engine, rx: &std::sync::mpsc::Receiver<midi::MidiOut>, frames: usize) -> Vec<Vec<u8>> {
        sent_by_frame(e, rx, frames).into_iter().map(|x| x.1).collect()
    }

    /// On an MPE port each note gets a member channel of its own (2, 3, 4 …), and
    /// ends on it; the sound's channel setting doesn't count.
    #[test]
    fn mpe_notes_get_channels_of_their_own() {
        let notes = vec![note_at(0, PPQ, 60, None), note_at(10, PPQ, 64, None), note_at(20, PPQ, 67, None)];
        let (mut e, rx) = mpe_engine(notes, vec![]);
        let got = sent(&mut e, &rx, 700);
        let ons: Vec<&Vec<u8>> = got.iter().filter(|m| m[0] & 0xf0 == 0x90).collect();
        assert_eq!(ons, vec![&vec![0x91, 60, 100], &vec![0x92, 64, 100], &vec![0x93, 67, 100]]);
        for off in [[0x81, 60, 0], [0x82, 64, 0], [0x83, 67, 0]] {
            assert!(got.contains(&off.to_vec()), "{off:?} in {got:?}");
        }
    }

    /// Before its note-on, a note's channel gets the note's starting bend, slide
    /// (CC74) and pressure; a note without expression starts neutral.
    #[test]
    fn mpe_notes_start_with_their_values() {
        let x = deckard_model::Expression {
            pitch: curve(&[(0, 12.0)]),
            slide: curve(&[(0, 0.5)]),
            pressure: curve(&[(0, 0.25)]),
            ..Default::default()
        };
        let (mut e, rx) = mpe_engine(vec![note_at(0, PPQ, 60, Some(0)), note_at(10, PPQ, 62, None)], vec![x]);
        let got = sent(&mut e, &rx, 100);
        assert_eq!(&got[..4], &[vec![0xe1, 0, 80], vec![0xb1, 74, 64], vec![0xd1, 32], vec![0x91, 60, 100]]);
        assert_eq!(&got[4..8], &[vec![0xe2, 0, 64], vec![0xb2, 74, 64], vec![0xd2, 0], vec![0x92, 62, 100]]);
    }

    /// While a note plays, its curves go out on its channel: pressure as channel
    /// pressure, slide as CC74, pitch as pitch bend (clipped to the port's range).
    #[test]
    fn mpe_curves_play_on_the_member_channel() {
        let x = deckard_model::Expression {
            pitch: curve(&[(0, 0.0), (PPQ, 60.0)]),
            slide: curve(&[(0, 0.0), (PPQ, 1.0)]),
            pressure: curve(&[(0, 0.0), (PPQ, 1.0)]),
            ..Default::default()
        };
        let (mut e, rx) = mpe_engine(vec![note_at(0, PPQ, 60, Some(0))], vec![x]);
        let got = sent(&mut e, &rx, 600);
        let off = got.iter().position(|m| m[0] == 0x81).expect("note-off");
        let during = &got[4..off];
        assert!(during.iter().any(|m| m[0] == 0xd1 && m[1] > 100), "pressure");
        assert!(during.iter().any(|m| m[..2] == [0xb1, 74] && m[2] > 100), "slide");
        assert!(during.iter().any(|m| m[..] == [0xe1, 0x7f, 0x7f]), "pitch clipped at +48");
        assert!(during.iter().all(|m| m[0] & 0x0f == 1), "all on its channel: {during:?}");
    }

    /// A pattern's controller and pitch-bend lanes go on the manager channel (1).
    #[test]
    fn lanes_go_on_the_manager_channel() {
        let (mut e, rx) = mpe_engine(vec![], vec![]);
        let mut p = project();
        p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
        p.groups[0].sounds[0].channel.midi_out_channel = 2;
        p.groups[0].patterns[0].notes = vec![];
        p.groups[0].patterns[0].midi = vec![
            deckard_model::MidiEvent { sound: 0, position: 0, kind: MidiKind::Controller { number: 1, value: 50 } },
            deckard_model::MidiEvent { sound: 0, position: 10, kind: MidiKind::PitchBend(9000) },
        ];
        let tx = e.midi_tx.take();
        e = click_engine_for(&p, 1000);
        e.midi_tx = tx;
        e.set_mpe_out([("Synth".to_string(), 48)].into());
        let got = sent(&mut e, &rx, 50);
        assert_eq!(&got[..2], &[vec![0xb0, 1, 50], vec![0xe0, (9000 & 0x7f) as u8, (9000 >> 7) as u8]]);
    }

    /// With all 15 channels busy, a 16th note ends the oldest and takes its channel.
    #[test]
    fn the_sixteenth_note_steals_the_oldest() {
        let notes = (0..16).map(|i| note_at(i * 10, 3 * PPQ, 60 + i, None)).collect();
        let (mut e, rx) = mpe_engine(notes, vec![]);
        let got = sent(&mut e, &rx, 120);
        let off = got.iter().position(|m| m[..] == [0x81, 60, 0]).expect("the first note ended");
        let on = got.iter().position(|m| m[..] == [0x91, 75, 100]).expect("the 16th on its channel");
        assert!(off < on);
    }

    /// Stopping resets the member channels DECKARD used, and only those.
    #[test]
    fn stop_resets_used_member_channels() {
        let (mut e, rx) = mpe_engine(vec![note_at(0, 3 * PPQ, 60, None), note_at(10, 3 * PPQ, 64, None)], vec![]);
        sent(&mut e, &rx, 50);
        e.stop();
        let got: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
        for ch in [1u8, 2] {
            assert!(got.contains(&vec![0x80 | ch, if ch == 1 { 60 } else { 64 }, 0]), "{got:?}");
            for m in [vec![0xe0 | ch, 0, 64], vec![0xb0 | ch, 74, 64], vec![0xd0 | ch, 0]] {
                assert!(got.contains(&m), "{m:?} in {got:?}");
            }
        }
        assert!(got.iter().all(|m| (1..=2).contains(&(m[0] & 0x0f))), "only channels 2 and 3: {got:?}");
    }

    /// The port set back to normal while a note sounds: the note ends on its member
    /// channel, the channels reset, and later notes go on the sound's channel.
    #[test]
    fn changing_a_ports_format_resets_it() {
        let (mut e, rx) = mpe_engine(vec![note_at(0, PPQ, 60, None), note_at(2 * PPQ, PPQ, 62, None)], vec![]);
        sent(&mut e, &rx, 50);
        e.set_mpe_out(Default::default());
        let got = sent(&mut e, &rx, 1100);
        assert_eq!(&got[..4], &[vec![0x81, 60, 0], vec![0xe1, 0, 64], vec![0xb1, 74, 64], vec![0xd1, 0]]);
        assert!(got.contains(&vec![0x92, 62, 100]), "{got:?}");
    }

    fn live_expr(pitch: i64, kind: ExprKind, value: f32) -> Live {
        Live::Expression { group: 0, sound: 0, pitch, kind, value }
    }

    /// A sound with modulation routes: pressure of its first slot's cutoff.
    fn with_route(p: &mut Project) {
        use deckard_model::mod_matrix::{ModParam, ModRoute, ModSource};
        p.groups[0].sounds[0].mod_routes = vec![ModRoute::new(ModSource::Pressure, 0, "Sampler", ModParam::Field("cutoff".into()))];
    }

    fn live_engine() -> Engine {
        let mut p = project();
        with_route(&mut p);
        click_engine_for(&p, 1000)
    }

    /// Pressure: the hardest-pressed held note's.
    #[test]
    fn pressure_source_is_the_hardest_pressed_note() {
        let mut e = live_engine();
        e.note_hold(0, 0, 60, 100);
        e.play_live(live_expr(60, ExprKind::Dim(Dim::Pressure), 0.3));
        e.note_hold(0, 0, 64, 100);
        e.play_live(live_expr(64, ExprKind::Dim(Dim::Pressure), 0.7));
        assert_eq!(e.sources(0, 0)[2], Some(0.7));
        e.play_live(live_expr(64, ExprKind::Dim(Dim::Pressure), 0.1));
        assert_eq!(e.sources(0, 0)[2], Some(0.3));
    }

    /// Slide and pitch: the latest held note's; released, the one before's.
    #[test]
    fn slide_and_pitch_follow_the_latest_note() {
        let mut e = live_engine();
        e.note_hold(0, 0, 60, 100);
        e.play_live(live_expr(60, ExprKind::Dim(Dim::Slide), 0.2));
        e.play_live(live_expr(60, ExprKind::Dim(Dim::Pitch), -1.0));
        e.note_hold(0, 0, 64, 100);
        e.play_live(live_expr(64, ExprKind::Dim(Dim::Slide), 0.8));
        e.play_live(live_expr(64, ExprKind::Dim(Dim::Pitch), 2.0));
        assert_eq!(e.sources(0, 0)[..2], [Some(2.0), Some(0.8)]);
        e.note_release(0, 0, 64);
        assert_eq!(e.sources(0, 0)[..2], [Some(-1.0), Some(0.2)]);
    }

    /// A note without pressure (a plain hit) doesn't count for it.
    #[test]
    fn a_note_without_pressure_doesnt_count() {
        let mut e = live_engine();
        e.note_hold(0, 0, 60, 100);
        e.play_live(live_expr(60, ExprKind::Dim(Dim::Pressure), 0.5));
        e.note_hold(0, 0, 64, 100);
        assert_eq!(e.sources(0, 0), [None, None, Some(0.5)]);
    }

    /// A pad held with Note Repeat on: its pressure is the source the whole time it's
    /// held, between the repeats too, until it's let go.
    #[test]
    fn a_repeated_pads_pressure_holds_between_repeats() {
        let mut p = project();
        with_route(&mut p);
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
        e.looping = true;
        e.set_repeat(Some((deckard_model::Arp { rate: 240, gate: 0.5, ..Default::default() }, repeat::RepeatMode::NoteRepeat)));
        e.schedule(0, Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
        e.schedule(0, live_expr(60, ExprKind::Dim(Dim::Pressure), 0.6));
        let mut out = vec![0f32; 512];
        // Several repeats (a quarter: four at 240 ticks), checked every block.
        let n = (960.0 * e.samples_per_tick() / 256.0).ceil() as usize;
        for i in 0..n {
            e.process(&mut out);
            assert_eq!(e.sources(0, 0)[2], Some(0.6), "block {i}");
        }
        e.schedule(0, Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
        for _ in 0..n {
            e.process(&mut out);
        }
        assert_eq!(e.sources(0, 0)[2], None, "let go");
    }

    /// A pattern's note with a pressure ramp drives the source as it plays, and ends
    /// with it: on the Sampler and on a MIDI port alike.
    #[test]
    fn recorded_curves_drive_the_sources() {
        let ramp = deckard_model::Expression { pressure: curve(&[(0, 0.0), (PPQ, 1.0)]), ..Default::default() };
        let mut p = project();
        with_route(&mut p);
        p.groups[0].patterns[0].notes = vec![note_at(0, PPQ, 60, Some(0))];
        p.expressions = vec![std::sync::Arc::new(ramp.clone())];
        let mut on_sampler = click_engine_for(&p, 1000);
        let (mut on_port, _rx) = expr_engine(ramp, PPQ);
        for e in [&mut on_sampler, &mut on_port] {
            let mut out = vec![0f32; 2];
            // Half the note (120 BPM at 1 kHz: 500 frames a quarter).
            for _ in 0..250 {
                e.process(&mut out);
            }
            let x = e.sources(0, 0)[2].expect("pressure while the note plays");
            assert!((x - 0.5).abs() < 0.02, "{x}");
            for _ in 0..300 {
                e.process(&mut out);
            }
            assert_eq!(e.sources(0, 0), [None; 3], "the note ended");
        }
    }

    /// Played live into an MPE port: a held key's pressure, slide and pitch go out
    /// on its own channel.
    #[test]
    fn live_mpe_expression_goes_out_per_note() {
        let (mut e, rx) = mpe_engine(vec![], vec![]);
        e.note_in(0, 0, 60, 90, true);
        rx.try_iter().count();
        e.play_live(live_expr(60, ExprKind::Dim(Dim::Pressure), 0.5));
        e.play_live(live_expr(60, ExprKind::Dim(Dim::Slide), 0.25));
        e.play_live(live_expr(60, ExprKind::Dim(Dim::Pitch), 2.0));
        let got: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
        let b = midi::bend_14(2.0, 48);
        assert_eq!(got, vec![vec![0xd1, 64], vec![0xb1, 74, 32], vec![0xe1, (b & 0x7f) as u8, (b >> 7) as u8]]);
    }

    /// A key from an MPE controller comes with its starting bend and slide (after its
    /// note-on, as the input reads them): on an MPE port they still go out before the
    /// note-on, so the synth starts the note where it was played.
    #[test]
    fn live_mpe_notes_start_with_the_controllers_values() {
        let (mut e, rx) = mpe_engine(vec![], vec![]);
        e.schedule(0, Live::NoteIn { group: 0, sound: 0, pitch: 60, velocity: 90, thru: true });
        e.schedule(0, live_expr(60, ExprKind::Dim(Dim::Pitch), 2.0));
        e.schedule(0, live_expr(60, ExprKind::Dim(Dim::Slide), 0.25));
        let got = sent(&mut e, &rx, 1);
        let b = midi::bend_14(2.0, 48);
        assert_eq!(&got[..4], &[vec![0xe1, (b & 0x7f) as u8, (b >> 7) as u8], vec![0xb1, 74, 32], vec![0xd1, 0], vec![0x91, 60, 90]]);
        assert_eq!(got.len(), 4, "nothing again after: {got:?}");
    }

    /// Into a normal port only pressure goes, as poly aftertouch.
    #[test]
    fn live_pitch_and_slide_stay_off_normal_ports() {
        let (mut e, rx) = expr_engine(deckard_model::Expression::default(), PPQ);
        e.note_in(0, 0, 60, 90, true);
        rx.try_iter().count();
        e.play_live(live_expr(60, ExprKind::Dim(Dim::Slide), 0.25));
        e.play_live(live_expr(60, ExprKind::Dim(Dim::Pitch), 2.0));
        e.play_live(live_expr(60, ExprKind::Dim(Dim::Pressure), 0.5));
        let got: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
        assert_eq!(got, vec![vec![0xa2, 60, 64]]);
    }

    /// A key's release velocity, played live, goes on its note-off.
    #[test]
    fn live_release_velocity_reaches_the_note_off() {
        let (mut e, rx) = expr_engine(deckard_model::Expression::default(), PPQ);
        e.note_in(0, 0, 60, 90, true);
        rx.try_iter().count();
        e.play_live(live_expr(60, ExprKind::Release, 0.5));
        e.play_live(Live::NoteRelease { group: 0, sound: 0, pitch: 60 });
        let got: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
        assert_eq!(got, vec![vec![0x82, 60, 64]]);
    }

    /// A recorded note's pressure plays out with it as poly aftertouch on its key,
    /// rising with the curve, and stops with the note.
    #[test]
    fn recorded_pressure_plays_as_poly_aftertouch() {
        let e = deckard_model::Expression { pressure: curve(&[(0, 0.0), (PPQ, 1.0)]), ..Default::default() };
        let (mut engine, rx) = expr_engine(e, PPQ);
        let got = sent_by_frame(&mut engine, &rx, 600);
        let off = got.iter().position(|(_, m)| m[0] == 0x82).expect("note-off");
        assert_eq!(got[0].1, vec![0x92, 60, 100]);
        let at: Vec<u8> = got[..off].iter().filter(|(_, m)| m[0] == 0xa2 && m[1] == 60).map(|(_, m)| m[2]).collect();
        assert!(at.len() > 10, "{at:?}");
        assert!(at.windows(2).all(|w| w[1] > w[0]), "rising: {at:?}");
        assert!(at[0] < 5 && *at.last().unwrap() >= 120, "{at:?}");
        assert!(!got[off..].iter().any(|(_, m)| m[0] == 0xa2), "none after the note-off");
    }

    /// Only changes are sent, and no more often than every 5 ms (5 frames here).
    #[test]
    fn expression_sends_changes_at_most_every_5_ms() {
        let e = deckard_model::Expression { pressure: curve(&[(0, 0.5)]), ..Default::default() };
        let (mut engine, rx) = expr_engine(e, PPQ);
        let at: Vec<Vec<u8>> = sent_by_frame(&mut engine, &rx, 600).into_iter().map(|x| x.1).filter(|m| m[0] == 0xa2).collect();
        assert_eq!(at, vec![vec![0xa2, 60, 64]], "a steady pressure, once");
        let e = deckard_model::Expression { pressure: curve(&[(0, 0.0), (10, 1.0)]), ..Default::default() };
        let (mut engine, rx) = expr_engine(e, PPQ);
        let frames: Vec<usize> = sent_by_frame(&mut engine, &rx, 100).into_iter().filter(|(_, m)| m[0] == 0xa2).map(|x| x.0).collect();
        assert!(frames.len() >= 2, "{frames:?}");
        assert!(frames.windows(2).all(|w| w[1] - w[0] >= 5), "{frames:?}");
    }

    /// The note's release velocity goes on its note-off (0 without one, as before).
    #[test]
    fn release_velocity_on_the_note_off() {
        let e = deckard_model::Expression { release_velocity: Some(40), ..Default::default() };
        let (mut engine, rx) = expr_engine(e, PPQ);
        let got: Vec<Vec<u8>> = sent_by_frame(&mut engine, &rx, 600).into_iter().map(|x| x.1).collect();
        assert!(got.contains(&vec![0x82, 60, 40]), "{got:?}");
        let (mut engine, rx) = expr_engine(deckard_model::Expression::default(), PPQ);
        let got: Vec<Vec<u8>> = sent_by_frame(&mut engine, &rx, 600).into_iter().map(|x| x.1).collect();
        assert!(got.contains(&vec![0x82, 60, 0]), "{got:?}");
    }

    /// A normal port has no per-note pitch or slide: only the note goes out.
    #[test]
    fn pitch_and_slide_stay_off_normal_ports() {
        let e = deckard_model::Expression {
            pitch: curve(&[(0, 0.0), (PPQ, 2.0)]),
            slide: curve(&[(0, 0.2), (PPQ, 0.9)]),
            ..Default::default()
        };
        let (mut engine, rx) = expr_engine(e, PPQ);
        let got: Vec<Vec<u8>> = sent_by_frame(&mut engine, &rx, 600).into_iter().map(|x| x.1).collect();
        assert_eq!(got, vec![vec![0x92, 60, 100], vec![0x82, 60, 0]]);
    }

    /// A pattern note with expression keeps its id on the timeline, and the timeline
    /// carries the table it indexes; the engine finds it there (an unknown id: none).
    #[test]
    fn timeline_notes_carry_their_expression() {
        let mut p = project();
        p.groups[0].patterns[0].notes[0].expr = Some(deckard_model::ExprId(0));
        let e = deckard_model::Expression { release_velocity: Some(40), ..Default::default() };
        p.expressions = vec![std::sync::Arc::new(e.clone())];
        let t = timeline(&p, Mode::Arrangement);
        let with: Vec<_> = t.events.iter().filter(|n| n.expr.is_some()).collect();
        assert!(!with.is_empty());
        // The first note, at each pass of the pattern; the others have none.
        assert!(with.iter().all(|n| n.tick % (4 * PPQ) == 0 && n.expr == Some(deckard_model::ExprId(0))), "{with:?}");
        assert_eq!(t.exprs.len(), 1);
        let engine = click_engine_for(&p, 1000);
        assert_eq!(engine.expression(deckard_model::ExprId(0)).as_deref(), Some(&e));
        assert!(engine.expression(deckard_model::ExprId(9)).is_none());
    }

    /// Pressure on a held note sent out (a pad, or a key passed thru) goes out with it
    /// as poly aftertouch on its key; a note not sent out, or released, sends none.
    #[test]
    fn pressure_on_a_held_note_goes_out_as_poly_aftertouch() {
        let mut p = project();
        p.groups[0].sounds[0].channel.midi_out = deckard_model::MidiDest::Port("Synth".into());
        p.groups[0].sounds[0].channel.midi_out_channel = 2;
        p.groups[0].sounds[0].channel.transpose = -12;
        let mut e = click_engine_for(&p, 1000);
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        e.midi_tx = Some(tx);
        let pressure = |pitch, value| Live::Expression { group: 0, sound: 0, pitch, kind: ExprKind::Dim(Dim::Pressure), value };
        e.play_live(Live::NoteIn { group: 0, sound: 0, pitch: 64, velocity: 90, thru: true });
        e.play_live(pressure(64, 0.5));
        e.play_live(Live::NoteOn { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX });
        e.play_live(pressure(60, 1.0));
        e.play_live(pressure(61, 1.0));
        e.play_live(Live::NoteRelease { group: 0, sound: 0, pitch: 64 });
        e.play_live(pressure(64, 0.2));
        let msgs: Vec<Vec<u8>> = rx.try_iter().map(|m| m.bytes().to_vec()).collect();
        assert_eq!(msgs, vec![vec![0x92, 52, 90], vec![0xa2, 52, 64], vec![0x92, 48, 100], vec![0xa2, 48, 127], vec![0x82, 52, 0]]);
    }

    #[test]
    fn swing_delays_off_beat_sixteenths() {
        let mut p = project();
        // Sixteenths on beat 1: ticks 0, 240, 480, 720.
        p.groups[0].patterns[0].notes = (0..4)
            .map(|i| Note { expr: None, sound: 0, position: i * 240, length: 60, pitch: 60, velocity: 127, probability: 100 })
            .collect();
        p.groups[0].swing = deckard_model::Swing { amount: 100.0, ..Default::default() };
        let ticks: Vec<i64> = timeline(&p, Mode::Pattern(0, 0)).events.iter().take(4).map(|e| e.tick).collect();
        // The 1/8 cycle is 480 ticks; its midpoint (240) moves to 95 %.
        assert_eq!(ticks, [0, 456, 480, 936]);
    }

    #[test]
    fn meters_report_what_each_sound_played() {
        let mut e = click_engine(1000);
        let mut out = vec![0f32; 2 * 100];
        e.process(&mut out);
        let m = e.take_meters();
        assert_eq!(m[0][0], (1.0, 1.0), "the click's peak");
        e.process(&mut out[..2]);
        assert_eq!(e.take_meters()[0][0], (0.0, 0.0), "silence since");
    }

    #[test]
    fn pan_law_is_unity_at_center() {
        let (l, r) = pan_gains(0.0);
        assert!((l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        let (l, r) = pan_gains(-1.0);
        assert!(r.abs() < 1e-6 && l > 1.0);
    }

    #[test]
    fn remaining_aux_sound_pre_and_post_fader_reach_the_selected_output() {
        for pre in [false, true] {
            let mut p = fader_project();
            let s = &mut p.groups[0].sounds[0];
            s.channel.dest = deckard_model::AudioDest::None;
            s.channel.aux_dest[0] = deckard_model::AudioDest::Master;
            s.channel.aux_pre[0] = pre;
            s.aux[0] = 0.75;
            s.level = 0.5;
            let mut e = fader_engine(&p);
            let got = steady(&mut e, 100).0;
            let want = 0.5 * if pre { 1.0 } else { deckard_model::level_gain(0.5) as f32 };
            assert!(near(got, want), "pre={pre}: {got}, want {want}");
        }
    }

    #[test]
    fn remaining_aux_group_send_works_without_a_group_effect() {
        let mut p = fader_project();
        let g = &mut p.groups[0];
        g.channel.dest = deckard_model::AudioDest::None;
        g.channel.aux_dest[1] = deckard_model::AudioDest::Master;
        g.aux[1] = 0.75;
        let mut e = fader_engine(&p);
        assert!(e.buses[0].is_some());
        assert!(near(steady(&mut e, 100).0, 0.5));
    }

    #[test]
    fn remaining_aux_automation_plays_and_stopped_returns_to_base() {
        let mut p = fader_project();
        let s = &mut p.groups[0].sounds[0];
        s.channel.dest = deckard_model::AudioDest::None;
        s.channel.aux_dest[0] = deckard_model::AudioDest::Master;
        s.aux[0] = 0.75;
        s.modulators = vec![fader_lane("aux1_level")];
        p.groups[0].patterns[0].mods = vec![fader_point(Some(0), "aux1_level", 0, -0.25)];
        let mut e = fader_engine(&p);
        assert!(near(steady(&mut e, 100).0, 0.5 * deckard_model::level_gain(0.5) as f32));
        e.sequencing = false;
        assert!(near(steady(&mut e, 100).0, 0.5));
    }

    #[test]
    fn remaining_aux_cued_sound_does_not_leak_to_the_room() {
        let mut p = fader_project();
        let s = &mut p.groups[0].sounds[0];
        s.cue = true;
        s.channel.aux_dest[0] = deckard_model::AudioDest::Master;
        s.aux[0] = 0.75;
        p.cue_output = deckard_model::AudioDest::Ext(1);
        let mut e = fader_engine(&p);
        assert!(near(steady(&mut e, 100).0, 0.0));
    }

    #[test]
    fn remaining_aux_return_buses_run_after_their_senders() {
        let mut p = fader_project();
        let mut ret = p.groups[0].clone();
        ret.sounds.clear();
        ret.channel.dest = deckard_model::AudioDest::None;
        ret.channel.aux_dest[0] = deckard_model::AudioDest::Master;
        ret.aux[0] = 0.75;
        p.groups.push(ret);
        p.groups[0].channel.dest = deckard_model::AudioDest::None;
        p.groups[0].channel.aux_dest[0] = deckard_model::AudioDest::Group(1);
        p.groups[0].aux[0] = 0.75;
        let mut e = fader_engine(&p);
        assert_eq!(e.bus_order, vec![0, 1]);
        assert!(near(steady(&mut e, 100).0, 0.5));
        p.groups[1].channel.aux_dest[0] = deckard_model::AudioDest::Group(0);
        let mut e = fader_engine(&p);
        assert!(e.group_aux.iter().all(|s| s.is_empty()));
        assert!(near(steady(&mut e, 100).0, 0.0));
    }

    #[test]
    fn remaining_aux_group_cycles_are_silent_and_do_not_feed_back() {
        let mut p = fader_project();
        p.groups[0].channel.aux_dest[0] = deckard_model::AudioDest::Group(0);
        p.groups[0].aux[0] = 1.0;
        let e = fader_engine(&p);
        assert!(e.group_aux[0].is_empty());
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

    fn passive_fx_slot() -> deckard_model::SlotInfo {
        deckard_model::SlotInfo { module: "Compressor".into(), name: "Compressor".into(), active: false, plugin: None, params: Vec::new() }
    }

    fn bus_sound_with_fx() -> deckard_model::Sound {
        deckard_model::Sound {
            level: 0.75,
            slots: vec![passive_fx_slot()],
            channel: deckard_model::SoundChannel { dest: deckard_model::AudioDest::Master, ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn aux_review_dry_plus_both_aux_sends_to_group_sound_bus_and_ext_outputs() {
        let mut p = fader_project();
        p.groups[0].sounds.push(bus_sound_with_fx());

        let mut ret_grp = deckard_model::Group { level: 0.75, slots: vec![passive_fx_slot()], ..Default::default() };
        ret_grp.channel.dest = deckard_model::AudioDest::Ext(1);
        p.groups.push(ret_grp);

        let src = &mut p.groups[0].sounds[0];
        src.level = 0.5;
        src.channel.dest = deckard_model::AudioDest::Master;
        src.channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 1);
        src.channel.aux_pre[0] = false;
        src.aux[0] = 0.75;
        src.channel.aux_dest[1] = deckard_model::AudioDest::Group(1);
        src.channel.aux_pre[1] = true;
        src.aux[1] = 0.75;

        let mut e = fader_engine(&p);
        push_half_gain(e.fx[0][1].as_mut().unwrap());
        push_half_gain(&mut e.buses[1].as_mut().unwrap().chain);
        let (master, ext2) = steady_outs(&mut e, 120);
        let fader = deckard_model::level_gain(0.5) as f32;
        // Dry (0.5 * fader) + Aux 1 post-fader through Sound(0, 1) with HalfGainEffect (0.5 * fader * 0.5).
        let want_master = 0.5 * fader + 0.5 * fader * 0.5;
        // Aux 2 pre-fader through Group(1) with HalfGainEffect onto Ext(1) (0.5 * 0.5).
        let want_ext2 = 0.5 * 0.5;
        assert!(near(master, want_master), "master={master}, want={want_master}");
        assert!(near(ext2, want_ext2), "ext2={ext2}, want={want_ext2}");
    }

    #[test]
    fn aux_review_pre_and_post_source_level_and_pan_on_sound_and_group_sends() {
        let lvl = deckard_model::level_from_db(-6.0);
        let g_lvl = deckard_model::level_gain(lvl) as f32;
        let pan = -0.5;
        let (pl, pr) = pan_gains(pan);
        for pre in [false, true] {
            // Sound send with source level + pan
            let mut p = fader_project();
            let s = &mut p.groups[0].sounds[0];
            s.level = lvl;
            s.pan = pan;
            s.channel.dest = deckard_model::AudioDest::None;
            s.channel.aux_dest[1] = deckard_model::AudioDest::Master;
            s.channel.aux_pre[1] = pre;
            s.aux[1] = 0.75;
            let mut e = fader_engine(&p);
            let (l, r) = steady(&mut e, 100);
            let (wl, wr) = if pre { (0.5, 0.5) } else { (0.5 * g_lvl * pl, 0.5 * g_lvl * pr) };
            assert!(near(l, wl) && near(r, wr), "sound pre={pre}: got ({l}, {r}), want ({wl}, {wr})");

            // Group send with group level + pan
            let mut p = fader_project();
            let g = &mut p.groups[0];
            g.level = lvl;
            g.pan = pan;
            g.channel.dest = deckard_model::AudioDest::None;
            g.channel.aux_dest[0] = deckard_model::AudioDest::Master;
            g.channel.aux_pre[0] = pre;
            g.aux[0] = 0.75;
            let mut e = fader_engine(&p);
            let (l, r) = steady(&mut e, 100);
            assert!(near(l, wl) && near(r, wr), "group pre={pre}: got ({l}, {r}), want ({wl}, {wr})");
        }
    }

    #[test]
    fn aux_review_bus_sound_processes_incoming_main_and_aux_audio_and_onward_sends() {
        // Put the Bus Sound at (0, 0) and the sender at (0, 1) so index order is reversed:
        // topological bus_order must run sender (0, 1) in render_pass and Bus Sound (0, 0) in run_buses.
        let mut p = fader_project();
        let mut sender = p.groups[0].sounds[0].clone();
        sender.channel.dest = deckard_model::AudioDest::None;
        sender.channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 0);
        sender.aux[0] = 0.75;

        let mut bus_snd = bus_sound_with_fx();
        bus_snd.channel.aux_dest[1] = deckard_model::AudioDest::Ext(1);
        bus_snd.aux[1] = 0.75;
        p.groups[0].sounds = vec![bus_snd, sender];

        let mut e = fader_engine(&p);
        assert_eq!(e.bus_order, vec![sound_bus_id(0, 0)]);
        push_half_gain(e.fx[0][0].as_mut().unwrap());
        let (master, ext2) = steady_outs(&mut e, 120);
        assert!(near(master, 0.25), "master={master}");
        assert!(near(ext2, 0.25), "onward send ext2={ext2}");
        let meters = e.take_meters();
        assert!(near(meters[0][0].0, 0.25), "bus sound meter={:?}", meters[0][0]);

        // Group main output + Group Aux send feeding a Bus Sound in Group 1
        let mut p = fader_project();
        p.groups[0].slots = vec![passive_fx_slot()];
        p.groups[0].channel.dest = deckard_model::AudioDest::Sound(1, 0);
        p.groups[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(1, 0);
        p.groups[0].aux[0] = 0.75;

        let g1 = deckard_model::Group { level: 0.75, sounds: vec![bus_sound_with_fx()], ..Default::default() };
        p.groups.push(g1);

        let mut e = fader_engine(&p);
        assert_eq!(e.bus_order, vec![0, sound_bus_id(1, 0)]);
        push_half_gain(e.fx[1][0].as_mut().unwrap());
        let (master, _) = steady(&mut e, 120);
        // Group 0 sends 0.5 via main + 0.5 via Aux 1 = 1.0 into Sound(1, 0), halved by HalfGainEffect -> 0.5.
        assert!(near(master, 0.5), "group->sound bus master={master}");

        // Active native Maschine Compressor on Bus Sound attenuates incoming Aux audio
        let mut p_native = fader_project();
        let mut comp_bus = bus_sound_with_fx();
        comp_bus.slots = vec![deckard_model::SlotInfo {
            module: "Compressor".into(),
            name: "Compressor".into(),
            active: true,
            plugin: None,
            params: [("mode", 0.0), ("threshold", 0.0), ("amount", 0.0), ("knee", 0.5), ("attack", 0.2), ("release", 0.3), ("gain", 0.5)]
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
        }];
        p_native.groups[0].sounds.push(comp_bus);
        p_native.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::None;
        p_native.groups[0].sounds[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 1);
        p_native.groups[0].sounds[0].aux[0] = 0.75;
        let mut e_native = fader_engine(&p_native);
        let (native_out, _) = steady(&mut e_native, 120);
        // Compressor with amount=0.0, gain=0.5 (-18 dB = 10^(-18/20) ≈ 0.12589) scales 0.5 input to ~0.06295.
        assert!((native_out - 0.5 * 0.12589254).abs() < 1e-3, "native compressor on bus sound: got {native_out}");
    }

    #[test]
    fn aux_review_mute_solo_and_cue_semantics_across_bus_sounds_and_aux_sends() {
        let mut p = fader_project();
        p.groups[0].sounds.push(bus_sound_with_fx());
        p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::None;
        p.groups[0].sounds[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 1);
        p.groups[0].sounds[0].aux[0] = 0.75;

        // 1. Trigger mute (audio_mute = false) on Bus Sound still passes incoming audio.
        let mut trig_muted = p.clone();
        trig_muted.groups[0].sounds[1].muted = true;
        trig_muted.groups[0].sounds[1].channel.audio_mute = false;
        let mut e = fader_engine(&trig_muted);
        push_half_gain(e.fx[0][1].as_mut().unwrap());
        assert!(near(steady(&mut e, 100).0, 0.25));

        // 2. Audio Mute (audio_mute = true) on Bus Sound silences it.
        let mut audio_muted = trig_muted.clone();
        audio_muted.groups[0].sounds[1].channel.audio_mute = true;
        let mut e = fader_engine(&audio_muted);
        assert!(near(steady(&mut e, 100).0, 0.0));

        // 3. Soloing sender (0, 0) via retain_sounds keeps Bus Sound (0, 1)'s FX return alive.
        let mut e = fader_engine(&p);
        push_half_gain(e.fx[0][1].as_mut().unwrap());
        e.retain_sounds(|g, s| (g, s) == (0, 0));
        assert!(near(steady(&mut e, 100).0, 0.25));

        // 4. Cue on Bus Sound routes post-FX signal to cue output (Ext 2) without leaking to Master.
        let mut cued = p.clone();
        cued.cue_output = deckard_model::AudioDest::Ext(1);
        cued.groups[0].sounds[1].cue = true;
        let mut e = fader_engine(&cued);
        e.sequencing = false;
        push_half_gain(e.fx[0][1].as_mut().unwrap());
        let (master, cue_out) = steady_outs(&mut e, 120);
        assert!(near(master, 0.0), "cued bus sound leaked to master: {master}");
        assert!(near(cue_out, 0.25), "cued bus sound on cue out: {cue_out}");
        assert!(near(e.cue.peak.0, 0.25), "cue peak: {:?}", e.cue.peak);
    }

    #[test]
    fn aux_review_feedback_cycles_across_sounds_and_groups_are_broken_without_runaway() {
        // Sound(0, 0) -> Aux 1 -> Sound(0, 1) -> Aux 1 -> Sound(0, 0)
        let mut p = fader_project();
        let mut s1 = bus_sound_with_fx();
        s1.channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 0);
        s1.aux[0] = 1.0;
        p.groups[0].sounds.push(s1);
        p.groups[0].sounds[0].slots = vec![passive_fx_slot()];
        p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::None;
        p.groups[0].sounds[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 1);
        p.groups[0].sounds[0].aux[0] = 0.75;

        let mut e = fader_engine(&p);
        let (l, r) = steady(&mut e, 200);
        assert!(l.is_finite() && r.is_finite() && l <= 0.5 + 1e-4, "cycle output=({l}, {r})");

        // Self-routing on Sound main dest and Aux dest is silenced
        let mut self_p = fader_project();
        self_p.groups[0].sounds[0].slots = vec![passive_fx_slot()];
        self_p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::Sound(0, 0);
        self_p.groups[0].sounds[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 0);
        self_p.groups[0].sounds[0].aux[0] = 1.0;
        let mut e = fader_engine(&self_p);
        assert!(e.sound_aux[0][0].is_empty());
        assert!(near(steady(&mut e, 100).0, 0.0));
    }

    #[test]
    fn aux_review_live_adopt_glides_send_and_bus_edits_without_clicks_or_lost_fx_state() {
        let mut p0 = fader_project();
        p0.groups[0].sounds.push(bus_sound_with_fx());
        p0.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::None;

        let mut cache = SampleCache::default();
        let (mut e, rep0) = Engine::new(&p0, None, 1000, Mode::Arrangement, &mut cache);
        e.live_in = Some(Default::default());
        e.looping = true;
        push_half_gain(e.fx[0][1].as_mut().unwrap());
        assert!(near(steady(&mut e, 80).0, 0.0));

        // 1. Live-assign Aux 1 -> Sound(0, 1): must ramp smoothly from 0 to 0.25 (with HalfGainEffect preserved).
        let mut p1 = p0.clone();
        p1.groups[0].sounds[0].channel.aux_dest[0] = deckard_model::AudioDest::Sound(0, 1);
        p1.groups[0].sounds[0].aux[0] = 0.75;
        let (fresh, rep1) = Engine::new_after(&p1, Some((&p0, rep0.chains.as_slice())), None, 1000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh, &p1));

        let mut frame = [0f32; 32];
        (frame[0], frame[1]) = (0.5, 0.5);
        e.live_in.as_ref().unwrap().lock().unwrap().extend(std::iter::repeat_n(frame, 4));
        let mut first_block = [0f32; 8];
        e.process(&mut first_block);
        // At sr=1000, 10ms glide is 10 frames: sample 0 must be small (< 0.05), not a step jump to 0.25.
        assert!(first_block[0] > 0.0 && first_block[0] < 0.05, "step jump on send add: {:?}", first_block);
        assert!(near(steady(&mut e, 80).0, 0.25), "fx state lost or wrong steady level after adopt");

        // 2. Live-unassign Aux 1 -> None: must ramp smoothly down from 0.25 to 0.0 without a step drop to 0.
        let (fresh2, _) = Engine::new_after(&p0, Some((&p1, rep1.chains.as_slice())), None, 1000, Mode::Arrangement, &mut cache);
        drop(e.adopt(fresh2, &p0));
        e.live_in.as_ref().unwrap().lock().unwrap().extend(std::iter::repeat_n(frame, 4));
        let mut fade_block = [0f32; 8];
        e.process(&mut fade_block);
        assert!(fade_block[0] > 0.15 && fade_block[0] < 0.25, "abrupt drop on send remove: {:?}", fade_block);
        assert!(near(steady(&mut e, 80).0, 0.0), "did not settle to 0 after send remove");
    }

    #[test]
    fn aux_review_bus_sound_automation_and_extra_output_strips() {
        // 1. Automation on a Bus Sound's onward Aux 2 send
        let mut p = fader_project();
        let mut bus_snd = bus_sound_with_fx();
        bus_snd.channel.dest = deckard_model::AudioDest::None;
        bus_snd.channel.aux_dest[1] = deckard_model::AudioDest::Master;
        bus_snd.aux[1] = 0.75;
        bus_snd.modulators = vec![fader_lane("aux2_level")];
        p.groups[0].sounds.push(bus_snd);
        p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::Sound(0, 1);
        p.groups[0].patterns[0].mods = vec![fader_point(Some(1), "aux2_level", 0, -0.25)];

        let mut e = fader_engine(&p);
        let modded = 0.5 * deckard_model::level_gain(0.5) as f32;
        assert!(near(steady(&mut e, 100).0, modded));
        e.sequencing = false;
        assert!(near(steady(&mut e, 100).0, 0.5));

        // 2. Extra-output strip on Sound(0, 0) routed into Bus Sound(0, 1)
        let mut s = Strips::new(|p| {
            p.groups[0].sounds.push(bus_sound_with_fx());
            p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::None;
            p.extra_outs[0].dest = deckard_model::AudioDest::Sound(0, 1);
        });
        push_half_gain(s.e.fx[0][1].as_mut().unwrap());
        // Input 0.5 -> Split bus 1 (x0.5 = 0.25) -> strip (level 1.0) -> Sound(0, 1) (HalfGainEffect x0.5 = 0.125).
        let (master, _) = s.block(0.5);
        assert!(near(master.0, 0.125) && near(master.1, 0.125), "strip->sound bus got={master:?}, want 0.125");
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use deckard_model::{Group, Note, Pattern, PluginRef, Scene, Section, SlotInfo, Sound};

    /// A pattern played by DECKARD's own synth: no plug-in to install, and on any thread.
    #[test]
    fn native_synth_plays_a_pattern() {
        let class = deckard_plugins::native::CLASSES[0];
        let plugin = PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
        let notes = (0..4)
            .map(|i| Note { expr: None, sound: 0, position: i * PPQ, length: 240, pitch: 48 + i, velocity: 110, probability: 100 })
            .collect();
        let p = Project {
            tempo: 120.0,
            numerator: 4,
            denominator: 4,
            groups: vec![Group {
                level: 0.75,
                sounds: vec![Sound {
                    level: 0.75,
                    instrument: "PluginHost".into(),
                    plugin: Some(plugin.clone()),
                    slots: vec![SlotInfo {
                        module: "PluginHost".into(),
                        name: class.name.into(),
                        active: true,
                        plugin: Some(plugin),
                        params: Vec::new(),
                    }],
                    ..Default::default()
                }],
                patterns: vec![Pattern { length: 4 * PPQ, notes, ..Default::default() }],
                ..Default::default()
            }],
            scenes: vec![Scene { patterns: vec![(0, 0)], ..Default::default() }],
            sections: vec![Section { scene: Some(0), length: 4 * PPQ, auto: false }],
            ..Default::default()
        };
        let mut cache = SampleCache::default();
        let (mut e, report) = std::thread::spawn(move || {
            let r = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
            (r.0, r.1)
        })
        .join()
        .unwrap();
        assert!(report.plugin_errors.is_empty(), "{:?}", report.plugin_errors);
        assert!(report.no_sampler.is_empty(), "{:?}", report.no_sampler);
        let out = render(&mut e, 3.0);
        let peak = |a: usize, b: usize| out[a * 2..b * 2].iter().fold(0f32, |m, x| m.max(x.abs()));
        // Each beat (0.5 s at 120 BPM) starts a note.
        for beat in 0..4 {
            let at = beat * 24_000;
            assert!(peak(at, at + 4800) > 0.02, "beat {beat} sounds: {}", peak(at, at + 4800));
        }
        assert!(out.iter().all(|x| x.is_finite()));
    }

    /// A Compressor after the synth posts its input level, gain reduction and output
    /// level for its panel; a low threshold shows reduction.
    #[test]
    fn compressor_posts_its_meters() {
        let class = deckard_plugins::native::CLASSES[0];
        let plugin = PluginRef { id: class.plugin_id(), name: class.name.into(), ..Default::default() };
        let notes: Vec<Note> = (0..4)
            .map(|i| Note { expr: None, sound: 0, position: i * PPQ, length: 240, pitch: 48 + i, velocity: 127, probability: 100 })
            .collect();
        let comp = |threshold: f64| SlotInfo {
            module: "Compressor".into(),
            name: "Compressor".into(),
            active: true,
            plugin: None,
            params: [
                ("mode", 0.0),
                ("threshold", threshold),
                ("amount", 1.0),
                ("knee", 0.5),
                ("attack", 0.2),
                ("release", 0.3),
                ("gain", 1.0),
            ]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect(),
        };
        let take = |threshold: f64| {
            let p = Project {
                tempo: 120.0,
                numerator: 4,
                denominator: 4,
                groups: vec![Group {
                    level: 0.75,
                    sounds: vec![Sound {
                        level: 0.75,
                        instrument: "PluginHost".into(),
                        plugin: Some(plugin.clone()),
                        slots: vec![
                            SlotInfo {
                                module: "PluginHost".into(),
                                name: class.name.into(),
                                active: true,
                                plugin: Some(plugin.clone()),
                                params: Vec::new(),
                            },
                            comp(threshold),
                        ],
                        ..Default::default()
                    }],
                    patterns: vec![Pattern { length: 4 * PPQ, notes: notes.clone(), ..Default::default() }],
                    ..Default::default()
                }],
                scenes: vec![Scene { patterns: vec![(0, 0)], ..Default::default() }],
                sections: vec![Section { scene: Some(0), length: 4 * PPQ, auto: false }],
                ..Default::default()
            };
            std::thread::spawn(move || {
                let mut cache = SampleCache::default();
                let (mut e, _) = Engine::new(&p, None, 48_000, Mode::Arrangement, &mut cache);
                let m = Arc::new(meters::FxMeters::default());
                e.set_fx_meters(&m);
                render(&mut e, 0.4);
                m.take(0, 0, 1)
            })
            .join()
            .unwrap()
        };
        let (input, gr, output) = take(1.0); // −60 dB
        assert!(input > 0.01 && output > 0.0, "levels {input} {output}");
        assert!(gr > 6.0, "gain reduction {gr} dB");
        let (_, gr, _) = take(0.0); // 0 dB: nothing to reduce
        assert!(gr < 0.5, "gain reduction {gr} dB");
    }
}
