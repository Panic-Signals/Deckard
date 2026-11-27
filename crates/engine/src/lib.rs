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
pub mod eq_spectrum;
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
pub mod munge;
#[cfg(native_asio)]
pub(crate) mod native_asio;
pub mod note_echo;
#[cfg(feature = "output")]
pub mod output;
pub mod pads;
#[cfg(test)]
mod performance_regressions;
pub mod pool;
mod prehear;
pub mod queue;
pub mod render;
pub mod repeat;
pub mod spectrum;
pub use ahead::Junk;
pub mod sample;
pub mod sampler;
pub mod sync;
pub mod synth;
// The MP60 and S1200 filter tables wait for those engines (docs/sampler-dsp.md §10).
#[allow(dead_code)]
mod sampler_tables;
mod zhora_rt;
pub use zhora_rt::{ZhoraOut, project_takes_input, zhora_outs};

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
    /// Where in that expression's curves it starts (ticks): a ratchet's later hits start
    /// where they do in the note, so the curves run over the whole note.
    pub expr_at: i64,
    /// A note a link added ([`pads`]): the sound whose note played it. Muting that sound
    /// silences this too (#21).
    pub from: Option<usize>,
    /// A hit of a ratcheted note: what its chance is rolled by instead of the event's
    /// place in the timeline, the same for all the note's hits, so they play or stay
    /// silent together.
    pub roll: Option<u64>,
}

/// A plug-in's note-off due `offset` frames after `now`, moved to its note's attack when
/// that is still queued for later: the plug-in played the attack ahead (a quantized key
/// or an arp step) and then the key was let go before it came, so the off can't come
/// first, or the note would never end.
fn after_queued_on(live: &[(u64, Live)], now: u64, offset: u64, same_on: impl Fn(&Live) -> bool) -> u64 {
    let off = now + offset;
    live.iter().filter(|(at, e)| *at > off && same_on(e)).map(|(at, _)| at - now).max().unwrap_or(offset)
}

/// A MIDI input plug-in starts a note at frame `on`: an off it queued before for the same
/// note (`same_off`) that would come after `on` belonged to the note before, and goes. An
/// instrument knows a note only by its pitch, so that off would end the new note. A plug-in
/// plays the offs due within a block as the block starts, so a note's end can be queued
/// before a note-on that comes earlier in the same block.
fn drop_later_offs(live: &mut Vec<(u64, Live)>, on: u64, same_off: impl Fn(&Live) -> bool) {
    live.retain(|(at, e)| *at <= on || !same_off(e));
}

/// Whether `e` is muted: its sound is, or (a link's note) the sound whose note played it.
fn note_muted(muted: &[Vec<bool>], e: &NoteEvent) -> bool {
    let of = |s: usize| muted.get(e.group).and_then(|r| r.get(s)).copied().unwrap_or(false);
    of(e.sound) || e.from.is_some_and(of)
}

/// An Audio plug-in's loop starting (with this take) or stopping. The `i64` is the shift
/// in ticks from the cue's tick to the take's first frame (the offset less the pattern's Start).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    On(usize, i64),
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
    /// How many ratcheted notes have been put in: each takes the next roll
    /// ([`NoteEvent::roll`]).
    pub ratchets: u64,
    /// Where a queued switch can happen: every this many ticks (a scene's longest
    /// pattern, inside the longer cycle its patterns make looping by themselves). 0: only
    /// at the end.
    pub switch_every: i64,
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
    /// [`Live::ChokeAll`] for one Group's sounds only (e.g. track-level All Notes Off,
    /// which leaves other Groups playing).
    ChokeGroup {
        group: usize,
    },
    /// [`Live::ChokeAll`] for one sound only (the Sampler's STOP: what it auditions ends,
    /// the song plays on).
    ChokeSound {
        group: usize,
        sound: usize,
    },
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

/// Where the rolls of ratcheted notes ([`NoteEvent::roll`]) start: past any event's place
/// in a timeline.
const RATCHET_ROLLS: u64 = 1 << 40;

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
            t.length = scene_cycle(p, scene);
            t.switch_every = p.scene_length(scene);
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

/// The take a cue starts and the beat its first frame falls on (none: it stops).
fn cue_take(e: &NoteEvent) -> Option<(usize, f64)> {
    match e.cue? {
        Cue::On(t, shift) => Some((t, (e.tick + shift) as f64 / PPQ as f64)),
        Cue::Off => None,
    }
}

/// How many sounding sounds [`Engine::catch_beat_hits`] tracks at once.
const SOUNDING_ROOM: usize = 64;

/// Sound `s` of group `g`'s notes on timeline `tl`, as (tick, pitch, length, velocity).
fn sound_notes(tl: &Timeline, g: usize, s: usize) -> impl Iterator<Item = (i64, i64, i64, i64)> + '_ {
    tl.events.iter().filter(move |e| (e.group, e.sound) == (g, s) && e.velocity > 0).map(|e| (e.tick, e.pitch, e.length, e.velocity))
}

/// How long after a beat's tick a mute or pattern switch still catches that beat's hit
/// (compensating for output buffer latency plus the controller-to-adopt round-trip).
const BEAT_GRACE_SECONDS: f64 = 0.055;

/// Recent live notes tracked so [`Engine::catch_beat_hits`] never double-triggers one.
const RECENT_LIVE_ROOM: usize = 16;

/// Whether note `e` is already sounding at timeline position `at` (`0..len`).
fn sounding_in_pass(e: &NoteEvent, fired: bool, at: f64, len: f64, looping: bool) -> Option<f64> {
    if e.length == HELD || e.length <= 0 || e.midi.is_some() || e.cue.is_some() || e.velocity <= 0 {
        return None;
    }
    let (start, end) = (e.tick as f64, (e.tick + e.length) as f64);
    if fired && start <= at && at < end {
        return Some(at - start);
    }
    if looping && end > len && start <= at + len && at + len < end {
        return Some(at + len - start);
    }
    None
}

/// Frames since `ev_frame` passed on a timeline of `end` frames, if within `1..=window`.
/// Events at `cur` (`d == 0`) are still ahead in `self.next` and play normally.
fn frames_since_beat(cur: u64, end: u64, looping: bool, window: u64, ev_frame: u64) -> Option<u64> {
    if window == 0 {
        return None;
    }
    if ev_frame < cur {
        let d = cur - ev_frame;
        return (d <= window).then_some(d);
    }
    if looping && end > 0 && ev_frame > cur && ev_frame < end {
        let d = (end - ev_frame).saturating_add(cur);
        return (d > 0 && d <= window).then_some(d);
    }
    None
}

/// Per-group signature of which pattern slot `mode` plays (`0` when silent), so
/// [`Engine::catch_beat_hits`] distinguishes pattern switches from note edits/recording.
fn group_pattern_sigs(project: &Project, mode: Mode) -> Vec<u64> {
    let slot_of = |g: usize, pi: usize| project.groups.get(g).and_then(|grp| grp.patterns.get(pi)).map_or(pi, |p| p.slot) as u64 + 1;
    (0..project.groups.len())
        .map(|g| match mode {
            Mode::Pattern(pg, pi) => {
                if g == pg {
                    slot_of(g, pi)
                } else {
                    0
                }
            }
            Mode::Scene(s) => {
                project.scenes.get(s).and_then(|sc| sc.patterns.iter().find(|x| x.0 == g)).map_or(0, |&(_, pi)| slot_of(g, pi))
            }
            Mode::Arrangement => {
                let mut h = 0xcbf2_9ce4_8422_2325u64;
                let mut any = false;
                for (si, sec) in project.sections.iter().enumerate() {
                    let v = sec
                        .scene
                        .and_then(|s| project.scenes.get(s))
                        .and_then(|sc| sc.patterns.iter().find(|x| x.0 == g))
                        .map_or(0, |&(_, pi)| slot_of(g, pi));
                    if v != 0 {
                        any = true;
                    }
                    h ^= (si as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ v;
                    h = h.wrapping_mul(0x0100_0000_01b3);
                }
                if any { h | 1 } else { 0 }
            }
        })
        .collect()
}
fn pattern_length(p: &Project, g: usize, pi: usize) -> i64 {
    own_length(p, &p.groups[g].patterns[pi])
}

/// The longest a scene's cycle gets, in bars; past it, its patterns start over with the
/// longest one.
const MAX_CYCLE_BARS: i64 = 128;

/// How long scene `scene` plays before it repeats. Each pattern loops by itself, as in
/// Maschine's Ideas view (a 1-bar and a 1:0:2 pattern line up again after 9 bars), so
/// it's the shortest stretch every pattern fits a whole number of times into: the
/// longest pattern when that's over [`MAX_CYCLE_BARS`].
fn scene_cycle(p: &Project, scene: usize) -> i64 {
    fn gcd(a: i64, b: i64) -> i64 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let longest = p.scene_length(scene);
    let cap = MAX_CYCLE_BARS * p.bar_length().max(1);
    let mut cycle = longest.max(1);
    for &(g, pi) in &p.scenes[scene].patterns {
        let l = pattern_length(p, g, pi).max(1);
        cycle = cycle / gcd(cycle, l) * l;
        if cycle > cap {
            return longest;
        }
    }
    cycle
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

/// A channel's own properties the engine plays modulation on (a lane with no slot): its
/// gain, and its two aux send levels. The app offers only these as modulation targets.
pub fn channel_field(field: &str) -> bool {
    matches!(field, "level" | "pan" | "aux1_level" | "aux2_level")
}

/// A pattern's modulation of channels' faders and sends, as lanes: (sound, `None` for
/// the group; field; points from the pattern's start, at their deviations).
fn fader_lanes(p: &Project, pat: &deckard_model::Pattern) -> Vec<(Option<usize>, String, Vec<(i64, f64)>)> {
    let plen = own_length(p, pat);
    let mut out: Vec<(Option<usize>, String, Vec<(i64, f64)>)> = Vec::new();
    for m in pat.mods.iter().filter(|m| m.slot.is_none() && channel_field(&m.field)) {
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
    let ratchets = &mut t.ratchets;
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
        // Audio plug-ins on in the pattern: each instance starts its loop over (the take's first
        // frame falls at the cue's offset), and stops it at its end (a stop sorts before a start
        // at the same tick, a start before the notes).
        for c in &pat.audio {
            let at = start + rep;
            let end = start + rep + plen.min(length - rep);
            let cue = |tick: i64, cue: Cue| NoteEvent {
                expr_at: 0,
                roll: None,
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
            out.push(cue(at, Cue::On(c.take, c.offset - pat.start)));
            out.push(cue(end, Cue::Off));
        }
        for n in &pat.notes {
            let pos = n.position - pat.start;
            if pos < 0 || pos >= plen || rep + pos >= length {
                continue;
            }
            // A ratchet's hits: the swing moves the note's start, the hits keep their even
            // spacing after it; one that would start at or past the pattern's (or the
            // pass's) end doesn't play. An ordinary note is its one hit.
            let at = swung(p, g, n.sound, start + rep + pos);
            let roll = (!n.ratchet.is_plain()).then(|| {
                *ratchets += 1;
                RATCHET_ROLLS + *ratchets
            });
            for hit in n.ratchet.hits(n.length, n.velocity) {
                if pos + hit.offset >= plen || rep + pos + hit.offset >= length {
                    break;
                }
                out.push(NoteEvent {
                    tick: at + hit.offset,
                    group: g,
                    sound: n.sound,
                    pitch: n.pitch,
                    velocity: hit.velocity,
                    length: hit.length,
                    probability: n.probability,
                    midi: None,
                    cue: None,
                    expr: n.expr,
                    expr_at: hit.offset,
                    from: None,
                    roll,
                });
            }
        }
        for m in &pat.midi {
            let pos = m.position - pat.start;
            if pos < 0 || pos >= plen || rep + pos >= length {
                continue;
            }
            out.push(NoteEvent {
                expr_at: 0,
                roll: None,
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
    if !snd.cue && has_sound_bus(p, g, s) && reaches_main(p, snd.channel.dest, (g, Some(s)), Some(g), 0) {
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
                if let Some(bus) = p.groups.get(g).and_then(|x| x.sounds.get(s)).filter(|_| !has_sound_bus(p, g, s)) {
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
    /// [`cue_on_master`]: the output whose `ext` buffer the bus fills, to be added to
    /// the master's output after the master.
    post_master: Option<usize>,
    /// That buffer as the span found it: what the bus adds is the difference.
    before: Vec<f32>,
}

impl CueBus {
    fn of(p: &Project) -> CueBus {
        let route = cue_route(p);
        // The click and the prehear as before: onto the master's mix after its effects,
        // past its level.
        let clicks_route = match p.cue_output {
            deckard_model::AudioDest::Ext(n) if (0..OUTPUTS as i64).contains(&n) => ext_route(p, n, channel_gain(p.cue_level, p.cue_pan)),
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
            post_master: cue_on_master(p),
            before: vec![0.0; JOB_FRAMES * 2],
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
            .is_some_and(|bus| !has_sound_bus(p, g, s) && (bus.cue || dest_cued(p, bus.channel.dest, Some(g), depth + 1))),
        D::None | D::Master | D::Ext(_) => false,
    }
}

/// The cue bus, on its output. On the master's output it's still its own: the engine
/// adds it there after the master's effects and level ([`cue_on_master`]), not into
/// the master's mix, where the master's plug-ins would play it.
fn cue_route(p: &Project) -> Route {
    match p.cue_output {
        deckard_model::AudioDest::Ext(n) if cue_on_master(p).is_some() => (Some(n as usize), channel_gain(p.cue_level, p.cue_pan)),
        deckard_model::AudioDest::Ext(n) => ext_route(p, n, channel_gain(p.cue_level, p.cue_pan)),
        _ => SILENT,
    }
}

/// The master's output (Out n, from 0), when the cue bus is on it too.
fn cue_on_master(p: &Project) -> Option<usize> {
    match p.cue_output {
        deckard_model::AudioDest::Ext(n) if p.cue_output == p.master_output && (0..OUTPUTS as i64).contains(&n) => Some(n as usize),
        _ => None,
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

/// Whether sound (`g`, `s`) needs its own bus buffer when fed by another channel:
/// active effects in its chain, an Aux send, or a side-chain tap listening to it.
fn has_sound_bus(p: &Project, g: usize, s: usize) -> bool {
    let Some(snd) = p.groups.get(g).and_then(|grp| grp.sounds.get(s)) else { return false };
    !effect_slots(snd).is_empty()
        || snd.channel.aux_dest.iter().any(|d| *d != deckard_model::AudioDest::None)
        || sound_feeds_sidechain(p, g, s)
}

/// Whether sound (`g`, `s`) has a bus and another channel routes or sends audio into it.
fn is_sound_bus(p: &Project, g: usize, s: usize) -> bool {
    if !has_sound_bus(p, g, s) {
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
    if (grp.muted || (bus.muted && bus.channel.audio_mute)) && !(has_sound_bus(p, g, s) && sound_feeds_strip(p, g, s)) {
        return SILENT;
    }
    if has_sound_bus(p, g, s) {
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
        spectrum: None,
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
#[derive(Clone, Copy)]
enum SpectrumChain {
    Sound(usize, usize),
    Group(usize),
    Master,
}

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
    spectrum: Option<(Arc<eq_spectrum::EqSpectrum>, SpectrumChain)>,
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

fn inherit_unit(u: &mut Unit, o: &mut Unit, fade: u32) {
    match (&mut u.fx, &mut o.fx) {
        (FxUnit::Plugin { params, sc_filter, .. }, FxUnit::Plugin { params: op, sc_filter: osf, .. }) => {
            std::mem::swap(sc_filter, osf);
            if params != op {
                sc_filter.set(params);
            }
        }
        (FxUnit::Internal { params, fx, glide, modulated, .. }, FxUnit::Internal { params: op, fx: ofx, glide: og, modulated: om, .. }) => {
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
                    let Ok(mut p) = shared.try_audio() else {
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
                FxUnit::Internal { class, fx, glide, .. } => {
                    fx.set_transport(transport.playing.then_some(transport.quarters));
                    let mut run = |l: &mut [f32], r: &mut [f32]| {
                        // Gliding to new settings: a step of it every glide::STEP frames.
                        let mut at = 0;
                        while at < n {
                            let to = if glide.gliding() { (at + glide::STEP).min(n) } else { n };
                            if glide.tick() {
                                fx.set(glide.now());
                            }
                            let (l, r) = (&mut l[at..to], &mut r[at..to]);
                            if sources.is_empty() {
                                fx.process(l, r)
                            } else {
                                fx.process_sc(l, r, Some((&self.sl[at..to], &self.sr[at..to])))
                            }
                            at = to;
                        }
                    };
                    let (l, r) = (&mut self.l[..n], &mut self.r[..n]);
                    if class == "EQ"
                        && !unit.ghost
                        && unit.target > 0.0
                        && let Some((board, chain)) = &self.spectrum
                    {
                        let owner = match *chain {
                            SpectrumChain::Sound(g, s) => Owner::Chain(g, s, unit.slot),
                            SpectrumChain::Group(g) => Owner::Group(g, unit.slot),
                            SpectrumChain::Master => Owner::Master(unit.slot),
                        };
                        board.process(owner, l, r, run);
                    } else {
                        run(l, r);
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
        // An unchanged chain needs no bounded pairing table. Preserve every
        // unit's tail, even in long chains, on unrelated document edits.
        if self.units.len() == old.units.len() && self.units.iter().zip(&old.units).all(|(u, o)| !o.ghost && u.same(o)) {
            for (u, o) in self.units.iter_mut().zip(&mut old.units) {
                inherit_unit(u, o, self.fade);
            }
            return;
        }
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
        let n_new = self.units.len().min(N);
        // Include the empty suffix row/column even when either chain has N units.
        // Longest common subsequence, on the stack; no audio-thread allocation.
        let mut table = [[0u8; N + 1]; N + 1];
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
                inherit_unit(u, o, fade);
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
    /// It is Zhora: it makes CV only, and its sound plays its return (see `zhora_rt`).
    cv_only: bool,
}

impl InstrumentRt {
    fn new(plugin: Shared, gain: (f32, f32), muted: bool) -> InstrumentRt {
        InstrumentRt { plugin, gain, muted, owner: None, scan: 0, heard: None, extras: Vec::new(), cv_only: false }
    }

    /// As [`Unit::listen`].
    fn listen(&mut self, owner: Owner, plugin: &Shared, plugin_id: &str, project: &Project) {
        self.owner = Some(owner);
        self.scan = scan_of(plugin, owner, plugin_id, project);
        self.extras = strips_of(owner, plugin_id, project);
    }
}

/// What an internal recording takes its audio from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TapSource {
    /// The master output (after its level and pan).
    #[default]
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

/// Plug-in blocks skipped because the instance was busy or its mutex was poisoned.
/// For diagnostics.
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
    /// The sound (group, sound) whose layers' morph the UI wants, and where it is.
    pub morph_watch: Option<(usize, usize)>,
    pub morph: Option<f32>,
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
    /// Per-group bitmask (`1 << sound`, up to 64 sounds) of unmuted sounds currently
    /// playing a loop (`SamplerRt::loop_playing`).
    pub loop_playing: Vec<u64>,
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
    /// The arrangement loop range in ticks; an empty range means the whole timeline.
    loop_range: (i64, i64),
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
    /// The master's gains as played: they glide to a new level or pan instead of
    /// jumping (a master knob turned while playing would zip).
    master_ramp: GainRamp,
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
    /// Beats into the note being played: one joined late (see [`Engine::follow_timeline`]).
    chase_skip: f64,
    /// The group that plays live (pads, keys): its sounds aren't rendered ahead.
    focus: Option<usize>,
    /// An edit's engine: per sound, what to keep of the playing one's (see
    /// [`Engine::new_after`]); and whether to keep its timelines.
    reuse: Vec<Vec<Option<Reuse>>>,
    keep_timeline: bool,
    group_swaps: Vec<(usize, usize, Option<usize>)>,
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
    pattern_sigs: Vec<u64>,
    pattern_sigs_mode: Mode,
    recent_live: [(u64, usize, usize, i64); RECENT_LIVE_ROOM],
    recent_live_at: usize,
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
    /// The project's CV routes.
    cv_routes: Vec<deckard_model::mod_matrix::CvRoute>,
    /// The output channels that are CV only (bits: channel 0 is bit 0).
    cv_channels: u32,
    /// Each CV channel's level: the voltage it wrote last.
    cv_last: [f32; 32],
    /// Frames the CV outputs still glide for, after a jump in their LFOs or an edit.
    cv_smooth_left: u32,
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
    /// The project's Zhoras ([`zhora_rt`]), room for handing one its input (left, right;
    /// `JOB_FRAMES` each), and the CV only channels they wrote this span.
    zhoras: Vec<zhora_rt::ZhoraRt>,
    zhora_in: (Vec<f32>, Vec<f32>),
    zhora_cv_owned: u32,
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
    /// When it started (`time`), and ticks per sample then: where it is in its curves,
    /// which it entered `from` ticks in.
    started: u64,
    from: i64,
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

/// Frames the input queue may hold past two blocks while a Zhora listens, for an input
/// whose blocks arrive unevenly.
const ZHORA_INPUT_SLACK: usize = 256;

/// Frames of interface input, routed to Maschine's 32 input channels.
pub type LiveIn = Arc<Mutex<std::collections::VecDeque<[f32; 32]>>>;

/// Sounds taking an external input: (group, sound, left channel, right channel, input
/// gain). (The sound's fader applies after its effects, as its Sampler's does.)
fn input_routes(p: &Project) -> Vec<InRoute> {
    let mut out = Vec::new();
    for (g, grp) in p.groups.iter().enumerate() {
        for (s, snd) in grp.sounds.iter().enumerate() {
            let deckard_model::AudioSource::Ext(n) = snd.channel.audio_in else { continue };
            // A chain starting with an instrument plays that, not the input; Zhora's
            // return is its own (see `zhora_rt`). A muted sound takes no input: unlike
            // notes, nothing of it rings out (it still plays for strips it feeds).
            if muting(p, g, s).muted || snd.slots.first().is_some_and(|x| x.is_instrument()) || zhora_rt::is_zhora(snd) {
                continue;
            }
            let Some((l, r)) = ext_channels(n) else { continue };
            let gain = deckard_model::level_gain(snd.channel.input_gain) as f32;
            out.push((g, s, l, r, (gain, gain)));
        }
    }
    out
}

/// The interface channels (left, right) of a sound's audio input `Ext(n)`: pairs below 16,
/// then each channel alone (heard on both sides). None past the 32 channels.
pub(crate) fn ext_channels(n: i64) -> Option<(usize, usize)> {
    let n = n.max(0) as usize;
    let (l, r) = if n < 16 { (2 * n, 2 * n + 1) } else { (n - 16, n - 16) };
    (r < 32).then_some((l, r))
}

/// Keys mode asks for renders this many semitones either side of the root.
const KEYS_SPAN: i64 = 12;

/// A New Sampler zone in Sync or Keys (its `meta` says so, with a tempo): it plays the
/// render of `span` (its play range, or more of the file around it) at the song's
/// `tempo`, its Tune (zone's and sound's, semitones) in it, when `renders` has it, and asks for it when not, playing its own
/// sample in time (repitched) meanwhile. Keys asks for the notes' renders around the
/// root, nearest first, and plays those made. Other zones are left as they are.
fn synced_zone(
    zone: &mut sampler::ZoneRt,
    meta: Option<&deckard_model::new_sampler::ZoneMeta>,
    sound_tune: f64,
    tempo: f64,
    span: (usize, usize),
    renders: &mut render::Renders,
) {
    use deckard_model::new_sampler::PlayMode;
    let Some(meta) = meta.filter(|m| m.mode != PlayMode::Repitch) else { return };
    let Some(bpm) = meta.bpm.filter(|b| b.is_finite() && *b > 0.0) else { return };
    if !(tempo.is_finite() && tempo > 0.0) || zone.play.1 <= zone.play.0 {
        return;
    }
    let keys = meta.mode == PlayMode::Keys;
    let semitones = zone.tune as f64 + sound_tune;
    // Warp markers: the range plays through its beat map.
    let map = (!meta.warps.is_empty()).then(|| deckard_model::new_sampler::BeatMap::of(meta, zone.sample.rate).map(Arc::new)).flatten();
    // Stretched as it plays: the zone stays the file itself (its lanes too), each voice
    // stretches it to whatever the song's tempo is. (Warp markers still take a render.)
    if !renders.ahead && map.is_none() {
        // (Its own speed against the song's is a tempo that much lower.)
        let tempo = bpm / meta.speed_factor();
        zone.sync = Some(sampler::ZoneSync { tempo, keys, grid: meta.start.beats(), live: Some(semitones), ..Default::default() });
        return;
    }
    let job_of = |sample: &Arc<Sample>, semis: f64| {
        let j = render::RenderJob::new(sample, span, bpm, tempo, semis);
        match &map {
            Some(m) => j.warped(m.clone()),
            None => j,
        }
    };
    let job = |semis: f64| job_of(&zone.sample, semis);
    // The stem lanes' renders for a Tune (each lane stretched as the sample is): all
    // of them, or none yet.
    let lanes = zone.lanes.clone();
    let lane_renders = |renders: &mut render::Renders, semis: f64| -> Option<[Option<Arc<Sample>>; sampler::LANES]> {
        let l = lanes.as_ref()?;
        let made: [Option<Option<Arc<Sample>>>; sampler::LANES] = std::array::from_fn(|k| match &l.samples[k] {
            Some(s) => renders.get_or_want(job_of(s, semis)).map(Some),
            None => Some(None),
        });
        made.iter().all(Option::is_some).then(|| made.map(Option::flatten))
    };
    let base = job(semitones);
    let made = renders.get_or_want(base.clone());
    // Its lanes are asked for with it. With one muted, turned down or taken away the zone
    // waits for them all: until then it plays as it is (repitched, its lanes with it), so
    // a mute holds through a change of tempo.
    let lanes_made = lane_renders(renders, semitones);
    let off = lanes.as_ref().is_some_and(|l| l.only || (0..sampler::LANES).any(|k| l.samples[k].is_some() && l.levels[k] != 1.0));
    let Some(made) = made.filter(|_| !off || lanes_made.is_some()) else {
        zone.sync = Some(sampler::ZoneSync {
            tempo: bpm,
            keys,
            notes: Vec::new(),
            note_lanes: Vec::new(),
            grid: meta.start.beats(),
            shown: Vec::new(),
            live: None,
        });
        return;
    };
    let mut notes = Vec::new();
    let mut note_lanes = Vec::new();
    if keys {
        let mut offsets: Vec<i64> =
            (-KEYS_SPAN..=KEYS_SPAN).filter(|o| *o != 0 && (zone.keys.0..=zone.keys.1).contains(&(zone.root + o))).collect();
        offsets.sort_by_key(|o| (o.abs(), *o < 0));
        // A note's lanes are asked for once a lane is off unity (a mute then waits for
        // them; until they come the note plays whole).
        let muted = lanes.as_ref().is_some_and(|l| (0..sampler::LANES).any(|k| l.samples[k].is_some() && l.levels[k] != 1.0));
        for o in offsets {
            if let Some(r) = renders.get_or_want(job(semitones + o as f64)) {
                notes.push((o, r));
                if muted {
                    note_lanes.push(lane_renders(renders, semitones + o as f64).unwrap_or_default());
                }
            }
        }
    }
    // Lanes all at unity play as the sample does: until they're made it plays whole.
    match lanes_made {
        Some(samples) => zone.lanes.as_mut().unwrap().samples = samples,
        None => zone.lanes = None,
    }
    // The render holds `span` only (the play range, or the stretch of the file this zone
    // and its sound's other zones of it play: chops of one sample share one render),
    // its frames where `out_frame` puts them.
    let len = made.frames.len();
    let at = |f: usize| (base.out_frame(f as f64).round().max(0.0) as usize).min(len);
    let a = zone.play.0;
    let scale = |x: usize| at(a + x) - at(a);
    // Where the render's frames are in the sample, for the playhead (65 points: between
    // warp markers the stretch is even).
    let (from, to) = (zone.play.0 as f64, zone.play.1 as f64);
    let shown: Vec<(f64, f64)> = (0..=64)
        .map(|k| {
            let f = from + (to - from) * f64::from(k) / 64.0;
            (base.out_frame(f).clamp(0.0, len as f64), f)
        })
        .collect();
    zone.looped = zone.looped.map(|(l, r)| (at(l), at(r))).filter(|(l, r)| r > l);
    zone.x_fade = scale(zone.x_fade);
    zone.fade_in = scale(zone.fade_in);
    zone.fade_out = scale(zone.fade_out);
    zone.play = (at(zone.play.0), at(zone.play.1));
    zone.sample = made;
    zone.sync = Some(sampler::ZoneSync { tempo, keys, notes, note_lanes, grid: meta.start.beats(), shown, live: None });
}

type FoundKey = (i64, String, String, String, Option<std::path::PathBuf>);

/// Read-only snapshot of [`SampleCache`]'s known paths and already-cached files,
/// so background group preparation can skip files already in memory.
#[derive(Default, Clone)]
pub struct SampleCacheSnapshot {
    library_dirs: Vec<std::path::PathBuf>,
    found: HashMap<FoundKey, std::path::PathBuf>,
    cached: std::collections::HashSet<std::path::PathBuf>,
}

/// Samples resolved and decoded ahead of [`Engine::new_after`] (e.g. on a group-load
/// worker thread).
#[derive(Default)]
pub struct PreloadedSamples {
    found: Vec<(FoundKey, std::path::PathBuf)>,
    decoded: Vec<(std::path::PathBuf, Arc<Sample>)>,
}

fn collect_sound_sample_refs(s: &deckard_model::Sound, out: &mut Vec<deckard_model::SampleRef>) {
    if synth_of(s).is_some() {
        return;
    }
    if s.instrument == "AudioModule" {
        if let Some(a) = s.audio.as_ref() {
            out.extend(a.takes.iter().map(|t| t.sample.clone()));
        }
        return;
    }
    let Some(smp) = s.sampler.as_ref() else { return };
    for z in &smp.zones {
        out.push(z.sample.clone());
    }
}

fn parallel_preload_samples<F>(
    refs: Vec<deckard_model::SampleRef>,
    project_path: Option<&Path>,
    library_dirs: &[std::path::PathBuf],
    known_found: &HashMap<FoundKey, std::path::PathBuf>,
    is_cached: F,
) -> PreloadedSamples
where
    F: Fn(&Path) -> bool + Sync,
{
    enum Work {
        ResolveAndDecode(FoundKey, deckard_model::SampleRef),
        DecodeOnly(std::path::PathBuf),
    }

    let proj_buf = project_path.map(Path::to_path_buf);
    let mut seen_keys = std::collections::HashSet::new();
    let mut claimed = std::collections::HashSet::new();
    let mut work = Vec::new();
    for r in refs {
        if r.path.is_empty() {
            continue;
        }
        let key: FoundKey = (r.kind, r.path.clone(), r.library.clone(), r.library_root.clone(), proj_buf.clone());
        if !seen_keys.insert(key.clone()) {
            continue;
        }
        if let Some(p) = known_found.get(&key) {
            if !is_cached(p) && claimed.insert(p.clone()) {
                work.push(Work::DecodeOnly(p.clone()));
            }
        } else {
            work.push(Work::ResolveAndDecode(key, r));
        }
    }
    if work.is_empty() {
        return PreloadedSamples::default();
    }
    let decode_valid = |p: &Path| Sample::load(p).ok().filter(|s| !s.frames.is_empty() && s.rate > 0).map(Arc::new);
    if work.len() == 1 {
        let mut out = PreloadedSamples::default();
        match work.into_iter().next().unwrap() {
            Work::DecodeOnly(p) => {
                if let Some(s) = decode_valid(&p) {
                    out.decoded.push((p, s));
                }
            }
            Work::ResolveAndDecode(key, r) => {
                let mut resolver = library::Resolver::new(project_path);
                resolver.set_extra(library_dirs);
                if let Some(p) = resolver.resolve(&r) {
                    if !is_cached(&p)
                        && let Some(s) = decode_valid(&p)
                    {
                        out.decoded.push((p.clone(), s));
                    }
                    out.found.push((key, p));
                }
            }
        }
        return out;
    }

    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8).min(work.len());
    let cursor = std::sync::atomic::AtomicUsize::new(0);
    let claimed = Mutex::new(claimed);
    let mut out = PreloadedSamples::default();
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            handles.push(scope.spawn(|| {
                let mut resolver: Option<library::Resolver> = None;
                let mut local = PreloadedSamples::default();
                loop {
                    let i = cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(item) = work.get(i) else { break };
                    match item {
                        Work::DecodeOnly(p) => {
                            if let Some(s) = decode_valid(p) {
                                local.decoded.push((p.clone(), s));
                            }
                        }
                        Work::ResolveAndDecode(key, r) => {
                            let res = resolver.get_or_insert_with(|| {
                                let mut res = library::Resolver::new(project_path);
                                res.set_extra(library_dirs);
                                res
                            });
                            if let Some(p) = res.resolve(r) {
                                let should_decode = !is_cached(&p) && claimed.lock().unwrap().insert(p.clone());
                                if should_decode && let Some(s) = decode_valid(&p) {
                                    local.decoded.push((p.clone(), s));
                                }
                                local.found.push((key.clone(), p));
                            }
                        }
                    }
                }
                local
            }));
        }
        for h in handles {
            if let Ok(part) = h.join() {
                out.found.extend(part.found);
                out.decoded.extend(part.decoded);
            }
        }
    });
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
    found: HashMap<FoundKey, std::path::PathBuf>,
    /// Plug-in instances, kept across engines (they are slow to load and hold state).
    pub plugins: deckard_plugins::Host,
    /// Where the engines made with it note plug-ins' extra pairs making sound (one board,
    /// so parts an edit keeps and parts it makes anew note on the same).
    heard: Arc<meters::HeardBoard>,
    /// The New Sampler's stretched renders (Sync and Keys), and those asked for.
    renders: render::Renders,
    /// Tests: plug-ins (made by the test) that play these slots, whatever the project
    /// names.
    #[cfg(test)]
    test_plugins: Vec<(Slot, Shared)>,
}

impl SampleCache {
    /// The renders engines made with it asked for since last time: make them
    /// ([`render::RenderJob::run`], off the audio and UI threads), give them back with
    /// [`SampleCache::add_render`], and build the engine again to play them.
    pub fn wanted_renders(&mut self) -> Vec<render::RenderJob> {
        self.renders.take_wanted()
    }

    /// Sync and Keys zones play renders made ahead (asked for here, played once made)
    /// instead of being stretched as they play (a stretcher a voice: nothing to wait for
    /// or keep, at more work a note), which is how they play unless this is set.
    pub fn stretch_ahead(&mut self, on: bool) {
        self.renders.ahead = on;
    }

    /// A render made for `job`.
    pub fn add_render(&mut self, job: &render::RenderJob, render: Arc<Sample>) {
        self.renders.add(job, render);
    }

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

    /// Where the file `sample` (as a project refers to it) is, if it's found.
    pub fn path_of(&self, sample: &deckard_model::SampleRef, project_path: Option<&Path>) -> Option<std::path::PathBuf> {
        let mut resolver = library::Resolver::new(project_path);
        resolver.set_extra(&self.library_dirs);
        resolver.resolve(sample)
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

    /// Whether `path` has already been decoded and stored in this cache.
    pub fn has_cached(&self, path: &Path) -> bool {
        matches!(self.by_path.get(path), Some(Some(_)))
    }

    /// Snapshot known sample paths and cached entries so a background worker can resolve
    /// and decode uncached samples without holding [`SampleCache`].
    pub fn known_sample_state(&self) -> SampleCacheSnapshot {
        SampleCacheSnapshot {
            library_dirs: self.library_dirs.clone(),
            found: self.found.clone(),
            cached: self.by_path.keys().cloned().collect(),
        }
    }

    /// Resolve and decode every sample referenced by `group`'s sounds in parallel.
    pub fn preload_group_samples(
        group: &deckard_model::Group,
        project_path: Option<&Path>,
        snapshot: &SampleCacheSnapshot,
    ) -> PreloadedSamples {
        let mut refs = Vec::new();
        for s in &group.sounds {
            collect_sound_sample_refs(s, &mut refs);
        }
        parallel_preload_samples(refs, project_path, &snapshot.library_dirs, &snapshot.found, |p| snapshot.cached.contains(p))
    }

    /// Merge samples resolved and decoded off-thread into this cache.
    pub fn ingest_preloaded(&mut self, preloaded: PreloadedSamples) {
        for (key, path) in preloaded.found {
            self.found.entry(key).or_insert(path);
        }
        for (path, sample) in preloaded.decoded {
            self.by_path.entry(path).or_insert(Some(sample));
        }
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
                    spectrum: None,
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
        {
            let mut refs = Vec::new();
            for (gi, g) in project.groups.iter().enumerate() {
                for (si, s) in g.sounds.iter().enumerate() {
                    if !kept(gi, si, |r| r.sampler) && !muting(project, gi, si).silent {
                        collect_sound_sample_refs(s, &mut refs);
                    }
                }
            }
            if refs.len() >= 2 {
                let preloaded =
                    parallel_preload_samples(refs, project_path, &cache.library_dirs, &cache.found, |p| cache.by_path.contains_key(p));
                cache.ingest_preloaded(preloaded);
            }
        }
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
        let mut renders = std::mem::take(&mut cache.renders);
        renders.begin();
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
                                    // Without a length, the power-of-two beat count nearest the song's tempo.
                                    let beats = if t.beats > 0.0 {
                                        t.beats
                                    } else {
                                        let at_song = (range.1 - range.0) as f64 / sample.rate.max(1) as f64 * project.tempo / 60.0;
                                        2f64.powf(at_song.max(1.0).log2().round()).clamp(1.0, 128.0)
                                    };
                                    audio_loop::TakeRt { sample, range, beats }
                                })
                                .collect();
                            let settings = audio_loop::Settings {
                                mode: a.mode,
                                engine: a.engine,
                                focused: a.focused,
                                tune: a.tune,
                                bend_range: (a.pitchbend - 0.5) * 24.0,
                                fade: a.fade,
                            };
                            return Some(SamplerRt::audio(audio_loop::LoopRt::new(takes, settings, sr_f), sr_f));
                        }
                        let smp = s.sampler.as_ref()?;
                        let mut zones = Vec::new();
                        for (zi, z) in smp.zones.iter().enumerate() {
                            report.zones += 1;
                            let Some(sample) = load(&z.sample) else { continue };
                            let len = sample.frames.len();
                            // Ranges are inclusive in the file.
                            // What a zone plays of its file `n` frames long. Reversed, a
                            // start point (it plays to the file's end) plays from there
                            // back to the beginning.
                            let play_of = |z: &deckard_model::Zone, n: usize| {
                                let (lo, hi) = z.play_range;
                                let p = if hi > lo { ((lo.max(0) as usize).min(n), ((hi + 1) as usize).min(n)) } else { (0, n) };
                                let back = z.meta.as_ref().is_some_and(|m| m.reverse) && s.new_sampler.is_some();
                                // (One that loops stays where it is: its loop is what goes backwards.)
                                let looping = z.loop_active && z.loop_range.1 > z.loop_range.0;
                                if back && !looping && p.1 == n && p.0 > 0 { (0, p.0) } else { p }
                            };
                            let play = play_of(z, len);
                            let (ll, lh) = z.loop_range;
                            let looped = (z.loop_active && lh > ll).then(|| {
                                let a = (ll.max(0) as usize).clamp(play.0, play.1);
                                (a, ((lh + 1) as usize).clamp(a, play.1))
                            });
                            let mut zone = sampler::ZoneRt {
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
                                sync: None,
                                lanes: None,
                                reverse: false,
                                once: false,
                                tail_ms: 0.0,
                                layer: None,
                            };
                            if s.new_sampler.is_some() {
                                // A layer: its envelope and window, heard unless muted
                                // or left out by another's solo.
                                if let Some(l) = z.meta.as_ref().and_then(|m| m.layer.as_ref()) {
                                    let all: Vec<deckard_model::layers::Layer> =
                                        smp.zones.iter().filter_map(|z| z.meta.as_ref()?.layer).collect();
                                    let at = smp.zones[..zi].iter().filter(|z| z.meta.as_ref().is_some_and(|m| m.layer.is_some())).count();
                                    let on = deckard_model::layers::audible(&all).get(at).copied().unwrap_or(true);
                                    zone.layer = Some(sampler::LayerRt { on, env: sampler::layer_times(l, sr_f), window: l.window });
                                }
                                zone.reverse = z.meta.as_ref().is_some_and(|m| m.reverse);
                                zone.once = z.meta.as_ref().is_some_and(|m| m.once);
                                zone.tail_ms = z.meta.as_ref().map_or(0.0, |m| m.tail_ms() as f32);
                                // Its stem lanes: files at the sample's rate.
                                if let Some(m) = z.meta.as_ref().filter(|m| m.has_stems()) {
                                    let rate = zone.sample.rate;
                                    let samples: [Option<Arc<Sample>>; sampler::LANES] = std::array::from_fn(|k| {
                                        let l = m.stems[k].as_ref()?;
                                        load(&deckard_model::SampleRef { kind: 0, path: l.file.clone(), ..Default::default() })
                                            .filter(|x| x.rate == rate)
                                    });
                                    let mut levels = deckard_model::new_sampler::lane_levels(&m.stems);
                                    for k in 0..sampler::LANES {
                                        if samples[k].is_none() {
                                            levels[k] = 0.0;
                                        }
                                    }
                                    if samples.iter().any(Option::is_some) {
                                        zone.lanes = Some(sampler::Lanes { samples, levels, only: m.stems_only });
                                    }
                                }
                                // Zones of the same file stretched alike (chops of one sample)
                                // share one render, of all they play between them.
                                let alike = |o: &&deckard_model::Zone| {
                                    use deckard_model::new_sampler::PlayMode;
                                    let (Some(m), Some(om)) = (z.meta.as_ref(), o.meta.as_ref()) else { return false };
                                    o.sample == z.sample
                                        && o.tune == z.tune
                                        && om.mode != PlayMode::Repitch
                                        && (om.bpm, &om.warps, om.grid_offset) == (m.bpm, &m.warps, m.grid_offset)
                                };
                                let span = smp.zones.iter().filter(alike).fold(play, |a, o| {
                                    let p = play_of(o, len);
                                    (a.0.min(p.0), a.1.max(p.1))
                                });
                                synced_zone(&mut zone, z.meta.as_ref(), smp.tune, project.tempo, span, &mut renders);
                            }
                            zones.push(zone);
                        }
                        // At unity: the sound's level and pan are the mixer's (they glide).
                        let mut rt = SamplerRt::new(sampler::Settings::new(smp, sr_f), zones, (1.0, 1.0), sr_f);
                        rt.motion = s.new_sampler.map_or(Default::default(), |n| n.motion);
                        // Its layers' morph.
                        let layered = smp.zones.iter().any(|z| z.meta.as_ref().is_some_and(|m| m.layer.is_some()));
                        rt.set_morph(s.new_sampler.filter(|_| layered).map(|n| n.morph));
                        Some(rt)
                    })
                    .collect()
            })
            .collect();
        cache.renders = renders;
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
                        inst.cv_only = deckard_plugins::zhora::is_zhora(&pr.id);
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
        let zhoras = zhora_rt::zhoras_of(project, cache);
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
        let listeners = sidechain_listeners(project);
        let sound_buses = sound_buses_of(project, &listeners);
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
                            // A sound bus (or a sound listening to one) waits for the channels that feed it.
                            || sound_buses[g][s]
                            // Its CV leaves with the outputs.
                            || zhora_rt::is_zhora(snd)
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
                        let takes_in = in_routes.iter().any(|r| (r.0, r.1) == (g, s)) || zhoras.iter().any(|z| (z.g, z.s) == (g, s));
                        let has = sounds.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some)
                            || instruments.get(g).and_then(|r| r.get(s)).is_some_and(Option::is_some)
                            || takes_in
                            || sound_buses[g][s];
                        let smp_gain = if has_fx(g, s) {
                            (1.0, 1.0)
                        } else if muting(project, g, s).hushed {
                            (0.0, 0.0)
                        } else {
                            sound_gain(project, g, s)
                        };
                        has.then(|| {
                            let mut b = SoundBufs::new(smp_gain);
                            // (Room for the input, so taking it allocates nothing.)
                            if takes_in {
                                b.inp.reserve(JOB_FRAMES * 2);
                            }
                            Box::new(b)
                        })
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
        let strips: Vec<(Owner, usize, Channels, StripRoute)> =
            project.extra_outs.iter().map(|x| (x.owner, x.bus, x.channels, strip_route(project, x))).collect();
        let order = aux::order(&buses, &group_aux, &sound_buses, &sound_routes, &sound_aux, &strips, &listeners);
        let engine = Engine {
            sr: sr_f,
            tempo: project.tempo,
            song_tempo: project.tempo,
            followed: false,
            timeline,
            mode,
            queued: None,
            loop_range: (0, 0),
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
            master_ramp: GainRamp::default(),
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
            chase_skip: 0.0,
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
            pattern_sigs: group_pattern_sigs(project, mode),
            pattern_sigs_mode: mode,
            recent_live: [(u64::MAX, 0, 0, 0); RECENT_LIVE_ROOM],
            recent_live_at: 0,
            lfos: lfo_rt::lfos_of(project),
            cc_routes: lfo_rt::ccs_of(project),
            cv_routes: project.cv_routes.clone(),
            cv_channels: 0,
            cv_last: [0.0; 32],
            cv_smooth_left: 0,
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
            zhoras,
            zhora_in: (vec![0.0; JOB_FRAMES], vec![0.0; JOB_FRAMES]),
            zhora_cv_owned: 0,
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
        for (a, b, keep) in fresh.group_swaps.drain(..) {
            self.swap_group_runtime(a, b, keep);
        }
        self.forward_edits(&mut fresh);
        self.keep_parts(&mut fresh);
        std::mem::swap(&mut self.live_hint, &mut fresh.live_hint);
        let ticks = self.position();
        let old_frame = self.frame;
        let old_spt = self.samples_per_tick();
        let sigs_valid = self.pattern_sigs_mode == self.mode && fresh.mode == self.mode;
        // The notes the timeline sounds here, to set against the new one's (below).
        let (sounding, sounding_n) = self.timeline_sounding(ticks);
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
        self.adopt_cvs(&mut fresh);
        // Notes held by plug-ins that are gone or muted now end.
        let kept = |p: &Shared| fresh.instruments.iter().flatten().flatten().any(|i| !i.muted && Arc::ptr_eq(&i.plugin, p));
        for i in self.instruments.iter().flatten().flatten() {
            if !kept(&i.plugin) {
                i.plugin.audio_all_notes_off();
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
        std::mem::swap(&mut self.zhoras, &mut fresh.zhoras);
        std::mem::swap(&mut self.sound_muted, &mut fresh.sound_muted);
        std::mem::swap(&mut self.silent, &mut fresh.silent);
        std::mem::swap(&mut self.pads, &mut fresh.pads);
        for g in 0..self.sound_muted.len() {
            for s in 0..self.sound_muted[g].len() {
                if self.sound_muted[g][s] {
                    self.release_muted_sound(g, s);
                }
            }
        }
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
        self.retime_offs(self.tempo, fresh.tempo);
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
        let mut swapped_from_timelines = false;
        if fresh.keep_timeline {
            // The same timelines: this one's play on.
        } else if fresh.mode == self.mode {
            std::mem::swap(&mut self.timeline, &mut fresh.timeline);
        } else if timelines.swap_into(self.mode, &mut self.timeline) {
            swapped_from_timelines = true;
        } else {
            self.stale = true;
        }
        if !fresh.keep_timeline
            && let Some((mode, t)) = self.queued.as_mut()
            && !timelines.swap_into(*mode, t)
        {
            self.stale = true;
        }
        if !self.stale {
            std::mem::swap(&mut self.pattern_sigs, &mut fresh.pattern_sigs);
            self.pattern_sigs_mode = self.mode;
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
        // The edit's Audio sounds; while playing, a loop it switched on (or changed)
        // sounds from where the song is (stopped, it waits for the pattern to play).
        std::mem::swap(&mut self.audio_sounds, &mut fresh.audio_sounds);
        if self.sequencing {
            self.recue();
            let timeline_changed = !fresh.keep_timeline && !self.stale;
            let old_tl = if !timeline_changed {
                None
            } else if swapped_from_timelines {
                timelines.0.iter().find(|(m, _)| *m == self.mode).map(|(_, t)| t)
            } else {
                Some(&fresh.timeline)
            };
            // (Before the new pattern's long notes are joined: those play on.)
            self.stop_long_samples_on_pattern_off(old_tl, &fresh.pattern_sigs, sigs_valid && timeline_changed);
            if !fresh.keep_timeline {
                self.follow_timeline(&sounding[..sounding_n]);
            }
            self.catch_beat_hits(
                old_tl,
                &fresh.sound_muted,
                &fresh.pattern_sigs,
                sigs_valid && !self.stale,
                timeline_changed,
                old_frame,
                old_spt,
            );
        }
        fresh
    }

    /// A group's pattern turned off or switched for another while the song plays: its
    /// sounds' long sample voices that a pattern started (a zone longer than a beat, or
    /// looped) fade out now, leaving drum hits to ring out. (`active`: the timeline
    /// changed and the old pattern signatures `old_sigs` are comparable.)
    fn stop_long_samples_on_pattern_off(&mut self, old_tl: Option<&Timeline>, old_sigs: &[u64], active: bool) {
        let Some(old) = old_tl.filter(|_| active) else { return };
        let tempo = self.tempo;
        for g in 0..self.sounds.len() {
            if self.pattern_sigs.get(g) == old_sigs.get(g) {
                continue;
            }
            for s in 0..self.sounds[g].len() {
                // (A sound whose notes the switch left as they were plays on.)
                if sound_notes(old, g, s).next().is_none() || sound_notes(old, g, s).eq(sound_notes(&self.timeline, g, s)) {
                    continue;
                }
                if self.link_ahead(g, s).is_some() {
                    self.send_live(g, s, ahead::Ev::FadeLongPatternVoices);
                } else if let Some(smp) = self.sounds[g][s].as_mut() {
                    smp.fade_long_pattern_voices(tempo);
                }
            }
        }
    }

    /// The notes (group, sound, pitch) the timeline sounds at song position `ticks`,
    /// as many as there's room for. (No allocating: this runs on the audio thread.)
    fn timeline_sounding(&self, ticks: f64) -> ([(usize, usize, i64); SOUNDING_ROOM], usize) {
        let mut out = [(0, 0, 0); SOUNDING_ROOM];
        let mut n = 0;
        let len = self.timeline.length as f64;
        if !self.sequencing || len <= 0.0 {
            return (out, 0);
        }
        let at = if self.looping { ticks.rem_euclid(len) } else { ticks };
        for (i, e) in self.timeline.events.iter().enumerate() {
            let key = (e.group, e.sound, e.pitch);
            if n < SOUNDING_ROOM && sounding_in_pass(e, i < self.next, at, len, self.looping).is_some() && !out[..n].contains(&key) {
                out[n] = key;
                n += 1;
            }
        }
        (out, n)
    }

    /// The timeline changed under a playing song (a pattern taken out of the scene or
    /// put in, notes edited). A note the old one was sounding (`was`) that the new one
    /// doesn't sound here stops now, rather than at its end; a long Sampler note the new
    /// one sounds here that wasn't sounding starts now, as far in as it would be, rather
    /// than next time round. A loop recorded into a pattern comes and goes at once.
    /// Short notes (hits) wait for their next time, as do notes a group's MIDI input
    /// plug-in plays and notes that only just started ([`Engine::catch_beat_hits`]
    /// plays those from their start).
    fn follow_timeline(&mut self, was: &[(usize, usize, i64)]) {
        let len = self.timeline.length as f64;
        if len <= 0.0 {
            return;
        }
        let at = if self.looping { self.position().rem_euclid(len) } else { self.position() };
        let now = self.time;
        for &(g, s, pitch) in was {
            let still = self.timeline.events.iter().enumerate().any(|(i, e)| {
                (e.group, e.sound, e.pitch) == (g, s, pitch) && sounding_in_pass(e, i < self.next, at, len, self.looping).is_some()
            });
            if still {
                continue;
            }
            if self.link_ahead(g, s).is_some() {
                self.send_live(g, s, ahead::Ev::Release { pitch });
            } else {
                for o in self.note_offs.iter_mut().filter(|o| (o.1, o.2, o.3) == (g, s, pitch) && o.0 > now) {
                    o.0 = now;
                }
            }
        }
        let spt = self.samples_per_tick();
        if spt <= 0.0 {
            return;
        }
        // (As catch_beat_hits reckons it.)
        let step_half = ((PPQ as f64 / 8.0) * spt).round().max(1.0) as u64;
        let grace = ((BEAT_GRACE_SECONDS * self.sr).round().max(1.0) as u64).min(step_half) as f64;
        for i in 0..self.timeline.events.len() {
            let e = self.timeline.events[i];
            let Some(into) = sounding_in_pass(&e, i < self.next, at, len, self.looping) else { continue };
            if e.length < PPQ
                || e.probability < 100
                || into * spt <= grace
                || was.contains(&(e.group, e.sound, e.pitch))
                || note_muted(&self.sound_muted, &e)
                || self.has_midi_input(e.group)
            {
                continue;
            }
            let ahead = self.link_ahead(e.group, e.sound).is_some();
            let local_sampler =
                self.sounds.get(e.group).and_then(|r| r.get(e.sound)).and_then(Option::as_ref).is_some_and(sampler::SamplerRt::can_chase)
                    && self.instruments.get(e.group).and_then(|r| r.get(e.sound)).is_none_or(Option::is_none);
            if ahead {
                // Its player takes it up after its lookahead.
                let delay = self.ahead.as_ref().map_or(0, |a| a.lookahead + a.span) as f64 / spt;
                let into = into + delay;
                let left = e.length as f64 - into;
                if left <= 0.0 {
                    continue;
                }
                let id = self.ahead.as_mut().map_or(0, |a| {
                    a.next_id += 1;
                    a.next_id
                });
                let ev = ahead::Ev::NoteOn {
                    id,
                    pitch: e.pitch,
                    velocity: e.velocity,
                    gate: Some((left * spt).max(1.0) as u64),
                    beats: self.position() / PPQ as f64,
                    live: false,
                    skip: into / PPQ as f64,
                };
                self.send_live(e.group, e.sound, ev);
            } else if local_sampler {
                let left = (e.length as f64 - into).round() as i64;
                if left <= 0 {
                    continue;
                }
                self.chase_skip = into / PPQ as f64;
                self.from_timeline = true;
                self.trigger_with(NoteEvent { length: left, ..e }, 1.0 / spt, false);
                self.from_timeline = false;
                self.chase_skip = 0.0;
            }
        }
    }

    fn played_live_within(&self, group: usize, sound: usize, pitch: i64, window: u64) -> bool {
        self.recent_live
            .iter()
            .any(|&(t, g, s, p)| t != u64::MAX && (g, s, p) == (group, sound, pitch) && self.time.saturating_sub(t) <= window)
    }

    fn fade_beat_sound(&mut self, group: usize, sound: usize) {
        self.lfo_release(group, sound, None);
        if self.link_ahead(group, sound).is_some() {
            self.send_live(group, sound, ahead::Ev::Stop);
            return;
        }
        if let Some(Some(snd)) = self.sounds.get_mut(group).and_then(|r| r.get_mut(sound)) {
            snd.fade_all();
        }
        if let Some(Some(inst)) = self.instruments.get(group).and_then(|r| r.get(sound)) {
            inst.plugin.audio_all_notes_off();
        }
        let now = self.time;
        for o in self.note_offs.iter_mut().filter(|o| (o.1, o.2) == (group, sound) && o.0 > now) {
            o.0 = now;
        }
    }

    /// When a mute or pattern switch arrives just after a beat (within
    /// [`BEAT_GRACE_SECONDS`] and half a 16th note), catch that beat's hit: fade a sound
    /// that just triggered and is now muted or dropped, and trigger a hit that was muted
    /// or absent when the beat passed. No allocating: runs on the audio thread.
    #[allow(clippy::too_many_arguments)]
    fn catch_beat_hits(
        &mut self,
        old_timeline: Option<&Timeline>,
        old_muted: &[Vec<bool>],
        old_sigs: &[u64],
        sigs_valid: bool,
        timeline_changed: bool,
        old_frame: u64,
        old_spt: f64,
    ) {
        if !self.sequencing || self.time == 0 {
            return;
        }
        let spt = self.samples_per_tick();
        if spt <= 0.0 || old_spt <= 0.0 {
            return;
        }
        let step_half = ((PPQ as f64 / 8.0) * spt).round().max(1.0) as u64;
        let grace = ((BEAT_GRACE_SECONDS * self.sr).round().max(1.0) as u64).min(step_half);
        let window = grace.min(self.time);
        if window == 0 {
            return;
        }
        let any_mute_diff = self.sound_muted != old_muted;
        let any_pat_diff = sigs_valid && timeline_changed && self.pattern_sigs != old_sigs;
        if !any_mute_diff && !any_pat_diff {
            return;
        }
        let pat_changed = |new_sigs: &[u64], g: usize| sigs_valid && timeline_changed && new_sigs.get(g) != old_sigs.get(g);
        let old_step_half = ((PPQ as f64 / 8.0) * old_spt).round().max(1.0) as u64;
        let old_window = ((BEAT_GRACE_SECONDS * self.sr).round().max(1.0) as u64).min(old_step_half).min(self.time);
        let old_len = old_timeline.map_or(self.timeline.length, |t| t.length);
        let old_end = if old_len > 0 { (old_len as f64 * old_spt).round().max(0.0) as u64 } else { 0 };
        let end = self.frame_of(self.timeline.length);

        if old_len > 0 {
            let mut faded = [(0usize, 0usize); SOUNDING_ROOM];
            let mut faded_n = 0usize;
            let new_len = self.timeline.length;
            let new_at = if new_len > 0 { self.position().rem_euclid(new_len as f64) } else { 0.0 };
            let old_count = old_timeline.map_or(self.timeline.events.len(), |t| t.events.len());
            for i in 0..old_count {
                let oe = match old_timeline {
                    Some(ot) => ot.events[i],
                    None => self.timeline.events[i],
                };
                if oe.midi.is_some() || oe.cue.is_some() || oe.velocity <= 0 || note_muted(old_muted, &oe) {
                    continue;
                }
                let oef = (oe.tick as f64 * old_spt).round().max(0.0) as u64;
                if frames_since_beat(old_frame, old_end, self.looping, old_window, oef).is_none() {
                    continue;
                }
                let key = (oe.group, oe.sound);
                if faded[..faded_n].contains(&key) {
                    continue;
                }
                let muted_now = note_muted(&self.sound_muted, &oe);
                let dropped_by_pattern = !muted_now
                    && pat_changed(&self.pattern_sigs, oe.group)
                    && (new_len <= 0
                        || !self.timeline.events.iter().any(|e| {
                            (e.group, e.sound) == key
                                && e.midi.is_none()
                                && e.cue.is_none()
                                && e.velocity > 0
                                && !note_muted(&self.sound_muted, e)
                                && (sounding_in_pass(e, e.tick as f64 <= new_at, new_at, new_len as f64, self.looping).is_some()
                                    || frames_since_beat(self.frame, end, self.looping, window, self.frame_of(e.tick)).is_some())
                        }));
                if muted_now || dropped_by_pattern {
                    self.fade_beat_sound(oe.group, oe.sound);
                    if faded_n < SOUNDING_ROOM {
                        faded[faded_n] = key;
                        faded_n += 1;
                    }
                }
            }
        }

        if self.timeline.length <= 0 {
            return;
        }
        let tps = 1.0 / spt;
        for i in 0..self.timeline.events.len() {
            let e = self.timeline.events[i];
            if e.midi.is_some() || e.cue.is_some() || e.velocity <= 0 || e.probability < 100 || note_muted(&self.sound_muted, &e) {
                continue;
            }
            if self.repeat_replaces(&e) {
                continue;
            }
            let Some(d) = frames_since_beat(self.frame, end, self.looping, window, self.frame_of(e.tick)) else {
                continue;
            };
            let mute_unmuted = note_muted(old_muted, &e);
            if !mute_unmuted && !pat_changed(&self.pattern_sigs, e.group) {
                continue;
            }
            if self.played_live_within(e.group, e.sound, e.pitch, window) {
                continue;
            }
            let already_fired = match old_timeline {
                None => !mute_unmuted,
                Some(ot) => {
                    old_len > 0
                        && ot.events.iter().any(|oe| {
                            (oe.group, oe.sound, oe.pitch) == (e.group, e.sound, e.pitch)
                                && oe.midi.is_none()
                                && oe.cue.is_none()
                                && oe.velocity > 0
                                && !note_muted(old_muted, oe)
                                && frames_since_beat(
                                    old_frame,
                                    old_end,
                                    self.looping,
                                    old_window,
                                    (oe.tick as f64 * old_spt).round().max(0.0) as u64,
                                )
                                .is_some()
                        })
                }
            };
            if already_fired {
                continue;
            }
            let into_ticks = (d as f64 * tps).round() as i64;
            let rem_length = if e.length == HELD { HELD } else { (e.length - into_ticks).max(1) };
            if self.link_ahead(e.group, e.sound).is_some() {
                let gate = (rem_length != HELD).then(|| ((rem_length as f64) * spt).max(1.0) as u64);
                let id = self.ahead.as_mut().map_or(0, |a| {
                    a.next_id += 1;
                    a.next_id
                });
                let ev = ahead::Ev::NoteOn {
                    id,
                    pitch: e.pitch,
                    velocity: e.velocity,
                    gate,
                    beats: self.position() / PPQ as f64,
                    live: false,
                    skip: 0.0,
                };
                self.send_live(e.group, e.sound, ev);
            } else {
                self.from_timeline = true;
                self.trigger(NoteEvent { length: rem_length, ..e }, tps);
                self.from_timeline = false;
            }
        }
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
                parts.muted = fresh.sound_muted.get(g).and_then(|r| r.get(s)).copied().unwrap_or(false);
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

    pub fn set_loop_range(&mut self, start: i64, end: i64) {
        let start = start.max(0);
        self.loop_range = (start, end.max(start));
    }

    fn loop_range(&self) -> (i64, i64) {
        let end = if self.loop_range.1 > self.loop_range.0 { self.loop_range.1 } else { self.timeline.length };
        (self.loop_range.0.min(end), end.min(self.timeline.length))
    }

    /// The Audio plug-ins' loops as they'd be where the song now is (the last cue before
    /// it, for each): the song started or jumped into a pattern, past its start.
    fn recue(&mut self) {
        for k in 0..self.audio_sounds.len() {
            let (g, s) = self.audio_sounds[k];
            let last = self.timeline.events[..self.next].iter().rev().find(|e| e.cue.is_some() && e.group == g && e.sound == s);
            let cue = last.and_then(cue_take);
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
        self.pattern_sigs = group_pattern_sigs(project, mode);
        self.pattern_sigs_mode = mode;
        self.frame = 0;
        self.next = 0;
        self.sequencing = true;
        self.recue();
    }

    fn repeat_replaces(&self, e: &NoteEvent) -> bool {
        self.recording
            && e.cue.is_none()
            && e.midi.is_none()
            && self.repeater.as_ref().is_some_and(|r| r.replaces(e.group, e.sound, e.pitch))
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
            let switch = (self.queued.is_some() && !switched).then(|| self.switch_tick(tl, frame)).flatten();
            if sequencing {
                while let Some(e) = tl.events.get(next) {
                    if self.frame_of(e.tick) > frame || switch.is_some_and(|at| e.tick >= at) {
                        break;
                    }
                    if t >= skip {
                        fire(Seen::Event {
                            t,
                            e,
                            plays: chance(e.probability, e.roll.map_or(next, |k| k as usize), tl.chance_pass(e, pass), self.rng)
                                && !self.repeat_replaces(e),
                            frame,
                            pass,
                        });
                    }
                    next += 1;
                }
                let switch_now = switch.map(|at| self.frame_of(at)).filter(|&f| frame >= f);
                if switch_now.is_some() || (next >= tl.events.len() && frame >= end) {
                    if self.queued.is_some() && !switched {
                        frame -= switch_now.unwrap_or(end).min(frame);
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
                if let Some(at) = switch {
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

    /// Where a switch queued `frame` frames into timeline `tl` happens before its end, if
    /// it does: the next multiple of its `switch_every` (where the scene's longest
    /// pattern ends, inside the longer cycle of patterns looping by themselves).
    fn switch_tick(&self, tl: &Timeline, frame: u64) -> Option<i64> {
        let every = tl.switch_every;
        if every <= 0 || every >= tl.length {
            return None;
        }
        let k = (frame as f64 / self.samples_per_tick() / every as f64).floor() as i64;
        let at = if k > 0 && self.frame_of(k * every) >= frame { k * every } else { (k + 1) * every };
        (at < tl.length).then_some(at)
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
            i.plugin.audio_all_notes_off();
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
            // A Zhora is waited for (the screens hold it only for moments): a gate left
            // up on a modular drones.
            if i.cv_only {
                if let Ok(mut p) = i.plugin.lock() {
                    p.all_notes_off();
                }
            } else {
                i.plugin.audio_all_notes_off();
            }
        }
        if let Some(r) = &mut self.repeater {
            r.clear();
        }
        self.held_keys.clear();
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
            let tick = n.from + (now.saturating_sub(n.started) as f64 * n.tps) as i64;
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
            expr_at: 0,
            roll: None,
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
            expr_at: 0,
            roll: None,
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
            NoteEvent {
                expr_at: 0,
                roll: None,
                tick: 0,
                group,
                sound,
                pitch,
                velocity,
                length,
                probability: 100,
                midi: None,
                cue: None,
                expr: None,
                from: None,
            },
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
        if let Some(Some(inst)) = self.instruments.get(group).and_then(|g| g.get(sound)) {
            inst.plugin.audio_note_off(pitch);
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

    /// Route any internal EQs to the output's spectrum monitor.
    pub fn set_fx_spectrum(&mut self, board: &Arc<eq_spectrum::EqSpectrum>) {
        for (g, row) in self.fx.iter_mut().enumerate() {
            for (s, chain) in row.iter_mut().enumerate() {
                if let Some(chain) = chain {
                    chain.spectrum = Some((board.clone(), SpectrumChain::Sound(g, s)));
                }
            }
        }
        for (g, bus) in self.buses.iter_mut().enumerate() {
            if let Some(bus) = bus {
                bus.chain.spectrum = Some((board.clone(), SpectrumChain::Group(g)));
            }
        }
        if let Some(chain) = &mut self.master_fx {
            chain.spectrum = Some((board.clone(), SpectrumChain::Master));
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
                        parts.muted = self.sound_muted.get(g).and_then(|r| r.get(s)).copied().unwrap_or(false);
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
                    ahead::Ev::Cue(take, origin) => {
                        if let Some(Some(snd)) = self.sounds.get_mut(g).and_then(|r| r.get_mut(s)) {
                            snd.cue(take.map(|t| (t, origin)));
                        }
                    }
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
                    (Some(_), _) => {
                        let cue = cue_take(e);
                        ahead::Ev::Cue(cue.map(|c| c.0), cue.map_or(0.0, |c| c.1))
                    }
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
                            live: false,
                            skip: 0.0,
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

    fn mute_midi_input_sound(&mut self, group: usize, sound: usize) {
        self.live.retain(|(_, e)| {
            !matches!(
                e,
                Live::MidiInputOut {
                    group: g,
                    note: midi_input::NoteOut::On { sound: s, .. } | midi_input::NoteOut::Off { sound: s, .. },
                } if (*g, *s) == (group, sound)
            )
        });
        let Some(mut p) = self.midi_inputs.get_mut(group).and_then(Option::take) else { return };
        let clock = self.midi_input_clock(0.0);
        let mut out = std::mem::take(&mut self.midi_input_out);
        p.mute_sound(sound, clock, &mut midi_input::Outbox::new(&mut out));
        self.midi_inputs[group] = Some(p);
        self.play_midi_input_out(group, clock.now, &out);
        self.midi_input_out = out;
    }

    /// Release every held note of a sound that is now muted, leaving one-shot tails and
    /// effect echoes to ring out naturally. No allocating: runs on the audio thread.
    fn release_muted_sound(&mut self, group: usize, sound: usize) {
        if self.has_midi_input(group) {
            self.mute_midi_input_sound(group, sound);
        }
        let mut i = 0;
        while i < self.midi_offs.len() {
            let (g, s, _) = self.midi_offs[i].held;
            if (g, s) == (group, sound) {
                let n = self.midi_offs.remove(i);
                self.end_midi_note(n);
            } else {
                i += 1;
            }
        }
        self.lfo_release(group, sound, None);
        self.touch.retain(|t| (t.group, t.sound) != (group, sound));
        self.note_offs.retain(|&(_, g, s, ..)| (g, s) != (group, sound));
        if self.link_ahead(group, sound).is_none()
            && let Some(Some(smp)) = self.sounds.get_mut(group).and_then(|r| r.get_mut(sound))
        {
            smp.all_notes_off();
        }
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
            let mut offset = ((at - now) * spt).round().max(0.0) as u64;
            if let midi_input::NoteOut::Off { sound, pitch, .. } = note {
                offset = after_queued_on(
                    &self.live,
                    self.time,
                    offset,
                    |e| matches!(*e, Live::MidiInputOut { group: g, note: midi_input::NoteOut::On { sound: s, pitch: p, .. } } if (g, s, p) == (group, sound, pitch)),
                );
            }
            if let midi_input::NoteOut::On { sound, pitch, .. } = note {
                drop_later_offs(&mut self.live, self.time + offset, |e| match *e {
                    Live::MidiInputOut { group: g, note: midi_input::NoteOut::Off { sound: s, pitch: p, .. } } => {
                        (g, s, p) == (group, sound, pitch)
                    }
                    _ => false,
                });
            }
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
                // Only the keys it took: a keyboard's NoteIn never goes to it, so its
                // release must still end the note (else it hangs until the transport stops).
                Live::NoteRelease { group, sound, pitch } if r.release(group, sound, pitch) => {
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
                | Live::ChokeAll
                | Live::ChokeGroup { .. }
                | Live::ChokeSound { .. } => None,
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
            Live::ChokeGroup { group } => self.choke_group(group),
            Live::ChokeSound { group, sound } => self.choke_sound(group, sound),
        }
    }

    /// Every voice of every sound ends: each sound as its choke group would end it.
    fn choke_all(&mut self) {
        for group in 0..self.fx.len() {
            self.choke_group(group);
        }
    }

    /// Every voice of one Group's sounds ends, as [`Self::choke_all`] ends them.
    fn choke_group(&mut self, group: usize) {
        for sound in 0..self.fx.get(group).map_or(0, Vec::len) {
            self.choke_sound(group, sound);
        }
    }

    /// Every voice of one sound ends, as [`Self::choke_all`] ends them.
    fn choke_sound(&mut self, group: usize, sound: usize) {
        if self.fx.get(group).is_none_or(|g| sound >= g.len()) {
            return;
        }
        let tps = 1.0 / self.samples_per_tick();
        let e = NoteEvent {
            expr_at: 0,
            roll: None,
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

    /// A live note on `sound`: choke what it chokes, then `play` it and the sounds it
    /// links, each at its pitch (see [`pads`]).
    fn hit_pads(&mut self, group: usize, sound: usize, pitch: i64, mut play: impl FnMut(&mut Self, usize, i64)) {
        let (plays, chokes) = self.pads.hit(group, sound);
        let tps = 1.0 / self.samples_per_tick();
        for s in pads::bits(chokes) {
            let e = NoteEvent {
                expr_at: 0,
                roll: None,
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

    /// Whether any sound takes audio from the interface, or a Zhora listens to it.
    pub fn takes_input(&self) -> bool {
        !self.in_routes.is_empty() || !self.zhoras.is_empty()
    }

    /// The interface input for the next `frames`, into the buffers of the sounds that
    /// take it (they play it in [`Engine::render_pass`]), and to the Zhoras' tuners.
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
        // Keep up: never more than a few blocks behind. While a Zhora listens, what stands
        // here is the tuner's delay: no more than a block (and some slack) is kept.
        let cap = if self.zhoras.is_empty() { frames * 4 + 512 } else { frames * 2 + ZHORA_INPUT_SLACK };
        while q.len() > cap {
            q.pop_front();
        }
        zhora_rt::hear(&self.zhoras, &mut self.bufs, &mut self.zhora_in, &q, frames);
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

    /// The master's level as a gain (what the recorder's monitor is heard through).
    pub fn master_gain(&self) -> f32 {
        self.master
    }

    /// The external output the master plays on (Out n, from 0).
    pub fn master_bus(&self) -> Option<usize> {
        self.master_bus
    }

    fn process_span(&mut self, out: &mut [f32], ext: &mut [Vec<f32>], at: usize) {
        out.fill(0.0);
        if let Some(n) = self.cue.post_master
            && let Some(was) = ext.get(n).and_then(|b| b.get(at..at + out.len()))
        {
            self.cue.before[..was.len()].copy_from_slice(was);
        }
        let frames = out.len() / 2;
        self.take_input(frames);
        // Players' notes for the frames ahead of this span.
        self.send_ahead(frames as u64);
        self.repeat_ahead(frames);
        self.midi_inputs_ahead(frames);
        self.follow_scene_tempo();
        let mut tps = 1.0 / self.samples_per_tick();
        let (loop_start, loop_end) = self.loop_range();
        let start_frame = self.frame_of(loop_start);
        let mut end = self.frame_of(loop_end);
        let mut done = 0;
        while done < frames {
            if self.follow_scene_tempo() {
                tps = 1.0 / self.samples_per_tick();
                end = self.frame_of(self.loop_range().1);
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
                // A queued switch comes where the scene's longest pattern ends, which can
                // be before the end of the cycle its patterns make.
                let switch = self.queued.as_ref().and_then(|_| self.switch_tick(&self.timeline, self.frame));
                // Events due now.
                while let Some(&e) = self.timeline.events.get(self.next) {
                    if self.frame_of(e.tick) > self.frame || switch.is_some_and(|at| e.tick >= at) {
                        break;
                    }
                    let idx = self.next;
                    self.next += 1;
                    let plays =
                        chance(e.probability, e.roll.map_or(idx, |k| k as usize), self.timeline.chance_pass(&e, self.pass), self.rng)
                            && !self.repeat_replaces(&e);
                    #[cfg(test)]
                    self.fired.push((self.time, e.tick, e.sound, e.pitch, plays));
                    if plays {
                        self.from_timeline = true;
                        self.trigger(e, tps);
                        self.from_timeline = false;
                    }
                }
                let switch_now = switch.map(|at| self.frame_of(at)).filter(|&f| self.frame >= f);
                if switch_now.is_some() || (self.frame >= end && (self.looping || self.next >= self.timeline.events.len())) {
                    if let Some((mode, timeline)) = self.queued.take() {
                        self.frame -= switch_now.unwrap_or(end).min(self.frame);
                        self.next = 0;
                        self.pass += 1;
                        self.mode = mode;
                        self.timeline = timeline;
                        self.recue();
                        end = self.frame_of(self.loop_range().1);
                        continue;
                    }
                    if self.looping && end > 0 {
                        self.frame = start_frame + self.frame.saturating_sub(end);
                        self.next = self.timeline.events.iter().position(|e| e.tick >= loop_start).unwrap_or(self.timeline.events.len());
                        self.pass += 1;
                        self.recue();
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
                if let Some(at) = self.queued.as_ref().and_then(|_| self.switch_tick(&self.timeline, self.frame)) {
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
            if let Some(at) = self.midi_offs.iter().map(|o| o.at).min() {
                until = until.min(at.saturating_sub(self.time).max(1));
            }
            // CV follows its envelope by the frame.
            if self.cv_channels != 0
                && let Some(at) = self.lfo_note_end()
            {
                until = until.min(at - self.time);
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
            // A Zhora's CV is of its last block: one block per span.
            if !self.zhoras.is_empty() {
                until = until.min(MAX_BLOCK as u64);
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
            self.write_zhora_cv(ext, at + done * 2, part);
            self.write_cv(ext, at + done * 2, part);
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
        let glide = (MASTER_GLIDE_SECONDS * self.sr).round() as u32;
        self.master_ramp.aim((self.master * self.master_pan.0, self.master * self.master_pan.1), glide);
        for frame in out.chunks_exact_mut(2) {
            let gain = self.master_ramp.next();
            let (l, r) = (frame[0] * gain.0, frame[1] * gain.1);
            frame[0] = if l.is_finite() { l.clamp(-4.0, 4.0) } else { 0.0 };
            frame[1] = if r.is_finite() { r.clamp(-4.0, 4.0) } else { 0.0 };
            self.master_peak.0 = self.master_peak.0.max(frame[0].abs());
            self.master_peak.1 = self.master_peak.1.max(frame[1].abs());
        }
        if self.tap.as_ref().is_some_and(|t| t.source == TapSource::Master) {
            self.flush_tap(out);
        }
        // The cue bus on the master's output: added after the master (its effects, level,
        // meter and recording), as on an output of its own. (The interface takes the master
        // from `out` on those channels, so their `ext` is otherwise unheard; a channel that's
        // CV is the `ext` one, left as it is.)
        if let Some(n) = self.cue.post_master
            && let Some(cue) = ext.get_mut(n).and_then(|b| b.get_mut(at..at + out.len()))
        {
            for (i, ((o, c), was)) in out.iter_mut().zip(cue).zip(&self.cue.before).enumerate() {
                let channel = n * 2 + i % 2;
                if channel < 32 && self.cv_channels >> channel & 1 == 1 {
                    continue;
                }
                // What the bus added this span moves to the master's output; the
                // buffer is left as it was given.
                let v = *o + (*c - *was);
                *c = *was;
                *o = if v.is_finite() { v.clamp(-4.0, 4.0) } else { 0.0 };
            }
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
        if e.cue.is_some() {
            if self.link_ahead(e.group, e.sound).is_none()
                && let Some(Some(s)) = self.sounds.get_mut(e.group).and_then(|g| g.get_mut(e.sound))
            {
                s.cue(cue_take(&e));
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
        if e.midi.is_none() && e.velocity > 0 && !self.from_timeline {
            self.recent_live[self.recent_live_at] = (self.time, e.group, e.sound, e.pitch);
            self.recent_live_at = (self.recent_live_at + 1) % self.recent_live.len();
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
            self.touch_start(e.group, e.sound, e.pitch, frames, tps, e.expr, e.expr_at);
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
                                    let start = |d: Dim| expr.as_ref().and_then(|x| x.curve(d)).map(|c| c.value_at(e.expr_at));
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
                                from: e.expr_at,
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
                        ahead::Ev::NoteOn {
                            id,
                            pitch: e.pitch,
                            velocity: e.velocity,
                            gate,
                            beats: self.position() / PPQ as f64,
                            live: self.sequencing,
                            skip: 0.0,
                        }
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
            if let Some(Some(inst)) = self.instruments.get(e.group).and_then(|g| g.get(e.sound)) {
                plugin_control(&inst.plugin, kind);
            }
            return;
        }
        if let Some(Some(inst)) = self.instruments.get(e.group).and_then(|g| g.get(e.sound)) {
            if inst.muted {
                return;
            }
            // Drop missed attacks so they do not sound late, while leaving any
            // earlier note's release scheduled when the instance is busy.
            if let Ok(mut p) = inst.plugin.try_audio() {
                p.note_on(e.pitch, e.velocity);
                self.note_offs.retain(|o| !(o.1 == e.group && o.2 == e.sound && o.3 == e.pitch));
                if e.length != HELD {
                    let gate = ((e.length.max(1) as f64) / tps).max(1.0) as u64;
                    self.note_offs.push((self.time + gate, e.group, e.sound, e.pitch, 0));
                }
            }
            return;
        }
        let ctx = sampler::Context { tempo: self.tempo, beats: self.position() / PPQ as f64, live: self.sequencing && !self.from_timeline };
        let Some(Some(snd)) = self.sounds.get_mut(e.group).and_then(|g| g.get_mut(e.sound)) else { return };
        self.clock += 1;
        let id = self.clock;
        snd.from_timeline = self.from_timeline;
        snd.note_on_at(id, e.pitch, e.velocity.clamp(0, 127), &ctx, self.chase_skip);
        snd.from_timeline = false;
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
        if t.loop_playing.len() != self.sounds.len() {
            t.loop_playing.resize(self.sounds.len(), 0);
        }
        for (g, (mask, row)) in t.loop_playing.iter_mut().zip(&self.sounds).enumerate() {
            let mut bits = 0u64;
            let mrow = self.sound_muted.get(g);
            for (s, smp) in row.iter().enumerate().take(64) {
                let muted = mrow.and_then(|r| r.get(s)).copied().unwrap_or(false);
                if !muted && smp.as_ref().is_some_and(|smp| smp.loop_playing()) {
                    bits |= 1u64 << s;
                }
            }
            *mask = bits;
        }
        self.publish_heard(t);
        t.zones.resize(t.watch.len(), None);
        for (pos, &(g, s, z)) in t.zones.iter_mut().zip(&t.watch) {
            *pos = self.zone_position(g, s, z);
        }
        t.morph = t.morph_watch.and_then(|(g, s)| self.morph_position(g, s));
    }

    /// Where the morph of sound `sound`'s layers is (0 to 1) for its newest note; `None`
    /// with the morph off or nothing sounding.
    pub fn morph_position(&self, group: usize, sound: usize) -> Option<f32> {
        self.sounds.get(group)?.get(sound)?.as_ref()?.morph_position()
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
                    i.plugin.audio_note_off(pitch);
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

/// Side-chain links between buses, as `(source, listener)` bus ids (a group's index, or
/// [`sound_bus_id`]): a sound that feeds a side-chain, and each group or sound whose
/// active effects listen to it. Computed directly from `project` rather than `fresh.fx`,
/// because incremental [`Engine::new_after`] edits leave reused sound chains as `None`
/// in `fresh.fx` until [`Engine::keep_parts`] runs.
fn sidechain_listeners(project: &Project) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (lg, grp) in project.groups.iter().enumerate() {
        for &(slot, src) in &grp.channel.sidechain {
            if src != deckard_model::ScSource::Group(lg) && grp.slots.get(slot).is_some_and(|x| x.active && !x.is_instrument()) {
                for (sg, ss, _) in sidechain_sources(project, src) {
                    out.push((sound_bus_id(sg, ss), lg));
                }
            }
        }
        for (ls, snd) in grp.sounds.iter().enumerate() {
            for &(slot, src) in &snd.channel.sidechain {
                if snd.slots.get(slot).is_some_and(|x| x.active && !x.is_instrument()) {
                    for (sg, ss, _) in sidechain_sources(project, src) {
                        if (sg, ss) != (lg, ls) {
                            out.push((sound_bus_id(sg, ss), sound_bus_id(lg, ls)));
                        }
                    }
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Which sounds run in [`Engine::run_buses`] rather than [`Engine::render_pass`]: bus
/// Sounds ([`is_sound_bus`]), plus any non-silent sound whose side-chain listens
/// (directly or transitively) to one, so its source bus Sound fills `sc_bufs` first.
fn sound_buses_of(project: &Project, listeners: &[(usize, usize)]) -> Vec<Vec<bool>> {
    let mut buses: Vec<Vec<bool>> =
        project.groups.iter().enumerate().map(|(g, grp)| (0..grp.sounds.len()).map(|s| is_sound_bus(project, g, s)).collect()).collect();
    let cap = project.groups.iter().map(|g| g.sounds.len()).sum::<usize>();
    for _ in 0..cap {
        let mut changed = false;
        for &(src, dst) in listeners {
            let (Some((sg, ss)), Some((lg, ls))) = (decode_sound_bus(src), decode_sound_bus(dst)) else { continue };
            if buses.get(sg).and_then(|r| r.get(ss)).copied().unwrap_or(false)
                && let Some(cell) = buses.get_mut(lg).and_then(|r| r.get_mut(ls))
                && !*cell
                && !muting(project, lg, ls).silent
            {
                *cell = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    buses
}

/// Whether sound (`g`, `s`) feeds any active effect's side-chain.
fn sound_feeds_sidechain(project: &Project, g: usize, s: usize) -> bool {
    let feeds = |src| sidechain_sources(project, src).into_iter().any(|(sg, ss, _)| (sg, ss) == (g, s));
    project.groups.iter().enumerate().any(|(gi, grp)| {
        grp.channel.sidechain.iter().any(|&(slot, src)| {
            src != deckard_model::ScSource::Group(gi) && grp.slots.get(slot).is_some_and(|x| x.active && !x.is_instrument()) && feeds(src)
        }) || grp.sounds.iter().enumerate().any(|(si, snd)| {
            (gi, si) != (g, s)
                && snd
                    .channel
                    .sidechain
                    .iter()
                    .any(|&(slot, src)| snd.slots.get(slot).is_some_and(|x| x.active && !x.is_instrument()) && feeds(src))
        })
    })
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
        (x, y) => {
            x.and_then(|s| s.plugin.as_ref().map(|p| (p.id.as_str(), p.name.as_str())))
                == y.and_then(|s| s.plugin.as_ref().map(|p| (p.id.as_str(), p.name.as_str())))
        }
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
    // (A New Sampler's own settings, its Slice Motion, are given it when it's made.)
    if same_silence && old.new_sampler == new.new_sampler {
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

/// The master's level and pan glide longer: a knob or fader sends steps tens of
/// milliseconds apart, and each step of the whole mix would otherwise be heard.
const MASTER_GLIDE_SECONDS: f64 = 0.03;

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

fn plugin_control(plugin: &Shared, kind: MidiKind) {
    let (number, value) = match kind {
        MidiKind::Controller { number, value } => (usize::from(number.min(127)), u16::from(value)),
        MidiKind::PitchBend(value) => (129, value),
        MidiKind::ChannelPressure(value) => (128, u16::from(value)),
    };
    plugin.audio_control(number, value);
}

/// Render one sound for `frames` frames into `b`: its Sampler voices (or the input it
/// takes), then its plug-in instrument, each through the sound's effects. A Zhora makes
/// only CV: its return, the input, goes through the effects alone.
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
    let cv_only = inst.as_ref().is_some_and(|i| i.cv_only);
    // Zhora (CV only) plays its return here, as a sound with no instrument does.
    if smp.is_some() || (b.inp_on && inst.is_none()) || cv_only {
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
    if cv_only {
        // Its CV for the span; its audio is silence, and nothing of it is mixed.
        let mut done = 0;
        while let Some(inst) = inst.as_deref_mut()
            && done < frames
        {
            let n = (frames - done).min(MAX_BLOCK);
            let started = std::time::Instant::now();
            match inst.plugin.lock() {
                Ok(mut p) => {
                    p.process(n, &at_frame(done));
                    timed(&inst.plugin, started);
                }
                Err(_) => skipped(),
            }
            done += n;
        }
        return;
    }
    if let Some(inst) = inst {
        let buf = &mut b.inst[..n2];
        buf.fill(0.0);
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(MAX_BLOCK);
            let t = at_frame(done);
            let started = std::time::Instant::now();
            let mut p = inst.plugin.try_audio().ok();
            let (l, r) = if let Some(p) = p.as_mut() {
                let out = p.process(n, &t);
                timed(&inst.plugin, started);
                out
            } else {
                skipped();
                (&[][..], &[][..])
            };
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
                    if let Some(p) = p.as_ref() {
                        take_strips(p, &mut inst.extras, n, done, (1.0, 0.0, 0, 1.0));
                        if let (Some(owner), Some(board)) = (inst.owner, &inst.heard) {
                            hear(p, owner, inst.scan, n, board);
                        }
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
                    if let Some(p) = p.as_ref() {
                        take_strips(p, &mut inst.extras, n, done, (1.0, 0.0, 0, 1.0));
                        if let (Some(owner), Some(board)) = (inst.owner, &inst.heard) {
                            hear(p, owner, inst.scan, n, board);
                        }
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
mod tests;

#[cfg(test)]
mod native_tests;
