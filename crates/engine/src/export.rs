//! Export Audio (docs/superpowers/specs/2026-10-23-phase10-export-design.md): the song,
//! a range of its sections, a scene or a pattern rendered offline, as the master or as
//! stems (a file per group or sound), written as WAV or AIFF with Maschine's names.
//!
//! Rendering runs an [`Engine`] block by block on the calling thread, without workers
//! or rendering ahead, so the same project always gives the same samples.

use crate::{Engine, Mode, SampleCache};
use deckard_model::{PPQ, Project};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// What part of the project plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    /// The whole song (its sections; every scene in turn if it has none).
    Song,
    /// Sections `.0` to `.1` (inclusive), as the song plays them.
    Sections(usize, usize),
    Scene(usize),
    /// (group, pattern)
    Pattern(usize, usize),
}

/// Whose output a file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stem {
    Master,
    Group(usize),
    Sound(usize, usize),
    /// The extra-output strip at this place in `project.extra_outs` (one on a hardware
    /// output): its pair alone, after its level and pan.
    Extra(usize),
}

/// The external output (Out 16, from 0) an extra-output strip's stem is rendered on:
/// [`stem_project`] sends the strip there and silences everything else that could.
pub const STEM_OUTPUT: usize = crate::OUTPUTS - 1;

/// Which external output holds `stem`'s file: none for the master's output.
pub fn stem_output(stem: Stem) -> Option<usize> {
    match stem {
        Stem::Extra(_) => Some(STEM_OUTPUT),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileType {
    #[default]
    Wav,
    Aiff,
}

impl FileType {
    pub fn extension(self) -> &'static str {
        match self {
            FileType::Wav => "wav",
            FileType::Aiff => "aiff",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BitDepth {
    Int16,
    #[default]
    Int24,
    Float32,
}

/// The panel's sample rates.
pub const RATES: [u32; 5] = [44_100, 48_000, 88_200, 96_000, 192_000];

/// How files are written: the panel's Options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Format {
    pub file_type: FileType,
    pub rate: u32,
    pub depth: BitDepth,
    /// Bring each file's peak to 0 dBFS.
    pub normalize: bool,
    /// Exactly the range, the tail rendered into its start (and the tempo in the file);
    /// off, the file runs on for the tail, to a bar line.
    pub loop_optimize: bool,
}

impl Default for Format {
    fn default() -> Format {
        Format { file_type: FileType::Wav, rate: 44_100, depth: BitDepth::Int24, normalize: false, loop_optimize: false }
    }
}

/// The longest tail rendered after the range, in seconds.
const MAX_TAIL: f64 = 30.0;
/// Output below this (−100 dBFS) counts as silence when looking for the tail's end.
const QUIET: f32 = 1e-5;

/// The project and mode that play `range`, and where on the song's clock it starts
/// (ticks, for synced LFOs): a range of sections is the song cut to them (each
/// section's patterns start with it, as when the song plays).
pub fn playing(p: &Project, range: Range) -> (Project, Mode, f64) {
    match range {
        Range::Song => (p.clone(), Mode::Arrangement, 0.0),
        Range::Sections(a, b) => {
            let mut q = p.clone();
            let b = b.min(p.sections.len().saturating_sub(1));
            let a = a.min(b);
            q.sections = p.sections.get(a..=b).map(<[_]>::to_vec).unwrap_or_default();
            let offset: i64 = p.sections.iter().take(a).map(|s| s.length).sum();
            (q, Mode::Arrangement, offset as f64)
        }
        Range::Scene(s) => (p.clone(), Mode::Scene(s), 0.0),
        Range::Pattern(g, pi) => (p.clone(), Mode::Pattern(g, pi), 0.0),
    }
}

/// The project that plays only `stem`: the other groups (or sounds) muted. A group's
/// stem is its output, so the master's effects go and the master plays at 0 dB, centred;
/// a sound's is its own channel's output, so its group's effects and fader go too.
pub fn stem_project(p: &Project, stem: Stem) -> Project {
    let mut q = p.clone();
    let keep_group = match stem {
        Stem::Master => return q,
        Stem::Extra(i) => return extra_stem_project(q, i),
        Stem::Group(g) | Stem::Sound(g, _) => g,
    };
    q.master_slots.clear();
    // Other channels' strips too: a muted owner still plays for a strip that can be
    // heard, so one playing into Deckard would reach this stem.
    {
        use deckard_model::extra_out::Owner;
        let kept = |o: Owner| match (stem, o) {
            (Stem::Group(g), Owner::Sound(a, _) | Owner::Chain(a, _, _) | Owner::Group(a, _)) => a == g,
            (Stem::Sound(g, s), Owner::Sound(a, b) | Owner::Chain(a, b, _)) => (a, b) == (g, s),
            _ => false,
        };
        for x in q.extra_outs.iter_mut().filter(|x| !kept(x.owner)) {
            x.muted = true;
        }
    }
    let unity = deckard_model::level_from_db(0.0);
    q.master_level = unity;
    q.master_pan = 0.0;
    for (g, grp) in q.groups.iter_mut().enumerate() {
        if g != keep_group {
            grp.muted = true;
            continue;
        }
        grp.muted = false;
        if let Stem::Sound(_, s) = stem {
            grp.slots.clear();
            grp.level = unity;
            grp.pan = 0.0;
            crate::drop_fader_lanes(&mut grp.modulators);
            for (k, snd) in grp.sounds.iter_mut().enumerate() {
                snd.muted = k != s;
            }
        }
    }
    q
}

/// The project that plays only strip `i` of `q.extra_outs` onto [`STEM_OUTPUT`] (see
/// [`stem_output`]). The render keeps that output alone, so the rest of the project plays
/// as it does live: whatever feeds the strip's plug-in still does, and the owner's own
/// pair goes on to the master (not this file). What could reach the stem's output is
/// kept off it: the strip is unmuted and moved there, the other strips on an external
/// output are muted (those routed into Deckard play, as they may feed the plug-in), no
/// channel, send or the cue goes to an external output, and the master's output is moved
/// off it. What goes to the master's output (the engine mixes that into the master) still
/// does.
fn extra_stem_project(mut q: Project, i: usize) -> Project {
    use deckard_model::AudioDest as D;
    if i >= q.extra_outs.len() {
        // No such strip: nothing reaches the output.
        q.extra_outs.clear();
        return q;
    }
    let stem = D::Ext(STEM_OUTPUT as i64);
    let master = q.master_output;
    let moved = if master == stem { D::Ext(0) } else { master };
    // A way to an external output: the master's stays the master's; others go nowhere.
    let off = |d: &mut D| {
        if let D::Ext(_) = *d {
            *d = if *d == master { moved } else { D::None };
        }
    };
    for (k, x) in q.extra_outs.iter_mut().enumerate() {
        if k == i {
            x.muted = false;
            x.dest = stem;
        } else if matches!(x.dest, D::Ext(_)) {
            off(&mut x.dest);
            x.muted |= x.dest == D::None;
        }
    }
    for grp in &mut q.groups {
        off(&mut grp.channel.dest);
        grp.channel.aux_dest.iter_mut().for_each(off);
        for snd in &mut grp.sounds {
            off(&mut snd.channel.dest);
            snd.channel.aux_dest.iter_mut().for_each(off);
        }
    }
    off(&mut q.cue_output);
    if !matches!(q.cue_output, D::Ext(_)) {
        q.cue_output = D::None;
    }
    q.master_output = moved;
    q
}

/// A sound slot with nothing loaded.
pub fn sound_is_empty(snd: &deckard_model::Sound) -> bool {
    snd.slots.is_empty() && snd.sampler.is_none() && snd.audio.is_none() && snd.plugin.is_none()
}

/// The groups the panel lists: those with a sound loaded.
pub fn exportable_groups(p: &Project) -> Vec<usize> {
    (0..p.groups.len()).filter(|&g| !exportable_sounds(p, g).is_empty()).collect()
}

/// A group's sounds the panel lists: those with something loaded.
pub fn exportable_sounds(p: &Project, g: usize) -> Vec<usize> {
    p.groups.get(g).map_or(Vec::new(), |grp| (0..grp.sounds.len()).filter(|&s| !sound_is_empty(&grp.sounds[s])).collect())
}

/// Rendered audio: interleaved stereo at `rate`, and the range's length (in frames,
/// ticks and bars) for the file's loop metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub audio: Vec<f32>,
    pub rate: u32,
    pub range_frames: usize,
    pub ticks: i64,
    pub tempo: f64,
    pub numerator: i64,
    pub denominator: i64,
}

impl Rendered {
    pub fn frames(&self) -> usize {
        self.audio.len() / 2
    }

    /// The loop's length in beats (of the meter's denominator).
    pub fn beats(&self) -> f64 {
        self.ticks as f64 * self.denominator.max(1) as f64 / (4 * PPQ) as f64
    }
}

/// Render `mode` of `p` (opened from `project_path`) as `fmt` asks: the range, then its
/// tail until silence (see [`Format::loop_optimize`]). `offset`: where the range starts
/// on the song's clock ([`playing`]). `progress` hears the fraction done; `None` when
/// `cancel` was set.
#[allow(clippy::too_many_arguments)]
pub fn render(
    p: &Project,
    project_path: Option<&Path>,
    mode: Mode,
    offset: f64,
    fmt: &Format,
    cache: &mut SampleCache,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Option<Rendered> {
    render_output(p, project_path, mode, offset, fmt, cache, cancel, progress, None)
}

/// [`render`], keeping external output `output` (Out n, from 0; see [`stem_output`])
/// rather than the master's output (`None`). Every output counts as connected, as in a
/// render of the master: a strip sent to one plays there, not through its parent.
#[allow(clippy::too_many_arguments)]
pub fn render_output(
    p: &Project,
    project_path: Option<&Path>,
    mode: Mode,
    offset: f64,
    fmt: &Format,
    cache: &mut SampleCache,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
    output: Option<usize>,
) -> Option<Rendered> {
    let rate = fmt.rate.max(1);
    let (mut engine, _) = Engine::new(p, project_path, rate, mode, cache);
    let output = output.filter(|&n| n < crate::OUTPUTS);
    engine.set_connected_outputs(u16::MAX);
    let mut ext: Vec<Vec<f32>> = if output.is_some() { vec![vec![0f32; 1024]; crate::OUTPUTS] } else { Vec::new() };
    let mut main = vec![0f32; 1024];
    engine.looping = false;
    engine.metronome = false;
    engine.song_offset = offset;
    let ticks = engine.timeline().length;
    // (From the first scene's own tempo, if it has one; later ones may differ.)
    engine.follow_scene_tempo();
    let tempo = engine.tempo();
    let spt = rate as f64 * 60.0 / (tempo * PPQ as f64);
    let range = engine.timeline_frames().round() as usize;
    let max = range + (MAX_TAIL * rate as f64) as usize;
    let mut audio: Vec<f32> = Vec::with_capacity(range * 2 + rate as usize);
    // The last loud frame, and how long it has been quiet since.
    let mut last_loud = 0usize;
    let mut done = 0usize;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let n = if done < range { (range - done).min(512) } else { 512 };
        let block = match output {
            Some(o) => {
                ext.iter_mut().for_each(|b| b.fill(0.0));
                engine.process_outputs(&mut main[..n * 2], &mut ext);
                &ext[o]
            }
            None => {
                engine.process(&mut main[..n * 2]);
                &main
            }
        };
        for (i, f) in block[..n * 2].as_chunks::<2>().0.iter().enumerate() {
            if f[0].abs().max(f[1].abs()) > QUIET {
                last_loud = done + i + 1;
            }
        }
        audio.extend_from_slice(&block[..n * 2]);
        done += n;
        if done < range {
            if done.is_multiple_of(512 * 64) {
                progress(done as f64 / range.max(1) as f64);
            }
            continue;
        }
        let quiet = done - last_loud.max(range);
        if (quiet as f64 >= 0.5 * rate as f64 && engine.active_voices() == 0) || done >= max {
            break;
        }
    }
    let tail = last_loud.saturating_sub(range);
    let bar = (p.bar_length() as f64 * spt).round().max(1.0) as usize;
    let length = if fmt.loop_optimize {
        // The tail wraps round onto the start.
        let mut out = audio[..range * 2].to_vec();
        if range > 0 {
            for i in range..range + tail {
                let at = (i - range) % range;
                out[at * 2] += audio[i * 2];
                out[at * 2 + 1] += audio[i * 2 + 1];
            }
        }
        audio = out;
        range
    } else if tail == 0 {
        range
    } else {
        (range + tail).div_ceil(bar) * bar
    };
    audio.resize(length * 2, 0.0);
    progress(1.0);
    Some(Rendered { audio, rate, range_frames: range, ticks, tempo, numerator: p.numerator, denominator: p.denominator })
}

/// Scale `audio` so its peak is at 0 dBFS (silence stays silent).
pub fn normalize(audio: &mut [f32]) {
    let peak = audio.iter().fold(0f32, |m, s| m.max(s.abs()));
    if peak > 0.0 {
        let g = 1.0 / peak;
        audio.iter_mut().for_each(|s| *s *= g);
    }
}

/// A loop's metadata for the file: tempo, length in beats and the meter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoopInfo {
    pub tempo: f64,
    pub beats: f64,
    pub numerator: i64,
    pub denominator: i64,
}

/// Interleaved stereo `audio` as a file's bytes: WAV (PCM, or IEEE float for 32 bit;
/// an `acid` chunk with `info`) or AIFF (32 bit float as AIFF-C `fl32`).
pub fn encode(audio: &[f32], rate: u32, file_type: FileType, depth: BitDepth, info: Option<LoopInfo>) -> Vec<u8> {
    let frames = audio.len() / 2;
    let bits: u16 = match depth {
        BitDepth::Int16 => 16,
        BitDepth::Int24 => 24,
        BitDepth::Float32 => 32,
    };
    let big = file_type == FileType::Aiff;
    let mut data = Vec::with_capacity(audio.len() * bits as usize / 8);
    for &s in audio {
        let s = if s.is_finite() { s } else { 0.0 };
        match depth {
            BitDepth::Int16 => {
                let v = (s.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
                data.extend_from_slice(&if big { v.to_be_bytes() } else { v.to_le_bytes() });
            }
            BitDepth::Int24 => {
                let v = (s.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
                let b = v.to_le_bytes();
                data.extend_from_slice(&if big { [b[2], b[1], b[0]] } else { [b[0], b[1], b[2]] });
            }
            BitDepth::Float32 => data.extend_from_slice(&if big { s.to_be_bytes() } else { s.to_le_bytes() }),
        }
    }
    match file_type {
        FileType::Wav => wav(&data, rate, bits, depth == BitDepth::Float32, info),
        FileType::Aiff => aiff(&data, frames, rate, bits, depth == BitDepth::Float32),
    }
}

/// A RIFF chunk: id, little-endian length, body, a pad byte when odd.
fn riff_chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
}

fn wav(data: &[u8], rate: u32, bits: u16, float: bool, info: Option<LoopInfo>) -> Vec<u8> {
    let channels: u16 = 2;
    let align = channels * bits / 8;
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&(if float { 3u16 } else { 1u16 }).to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * align as u32).to_le_bytes());
    fmt.extend_from_slice(&align.to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    let mut body = b"WAVE".to_vec();
    riff_chunk(&mut body, b"fmt ", &fmt);
    if let Some(i) = info {
        // ACID loop info: a loop (flags 0), root note C3, its beats, meter and tempo.
        let mut acid = Vec::new();
        acid.extend_from_slice(&0u32.to_le_bytes());
        acid.extend_from_slice(&60u16.to_le_bytes());
        acid.extend_from_slice(&0x8000u16.to_le_bytes());
        acid.extend_from_slice(&0f32.to_le_bytes());
        acid.extend_from_slice(&(i.beats.round().max(0.0) as u32).to_le_bytes());
        acid.extend_from_slice(&(i.denominator.clamp(1, 64) as u16).to_le_bytes());
        acid.extend_from_slice(&(i.numerator.clamp(1, 64) as u16).to_le_bytes());
        acid.extend_from_slice(&(i.tempo as f32).to_le_bytes());
        riff_chunk(&mut body, b"acid", &acid);
    }
    riff_chunk(&mut body, b"data", data);
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// `v` as an 80-bit IEEE extended float (big-endian), AIFF's sample rate.
fn extended(v: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    if v <= 0.0 {
        return out;
    }
    let exp = v.log2().floor() as i32;
    let mant = (v / 2f64.powi(exp - 63)) as u64;
    out[..2].copy_from_slice(&((exp + 16_383) as u16).to_be_bytes());
    out[2..].copy_from_slice(&mant.to_be_bytes());
    out
}

/// An IFF chunk: id, big-endian length, body, a pad byte when odd.
fn iff_chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
}

fn aiff(data: &[u8], frames: usize, rate: u32, bits: u16, float: bool) -> Vec<u8> {
    let mut comm = Vec::new();
    comm.extend_from_slice(&2u16.to_be_bytes());
    comm.extend_from_slice(&(frames as u32).to_be_bytes());
    comm.extend_from_slice(&bits.to_be_bytes());
    comm.extend_from_slice(&extended(rate as f64));
    let mut body = Vec::new();
    if float {
        body.extend_from_slice(b"AIFC");
        iff_chunk(&mut body, b"FVER", &0xA280_5140u32.to_be_bytes());
        comm.extend_from_slice(b"fl32");
        let name = b"32-bit floating point";
        comm.push(name.len() as u8);
        comm.extend_from_slice(name);
        if (name.len() + 1) % 2 == 1 {
            comm.push(0);
        }
    } else {
        body.extend_from_slice(b"AIFF");
    }
    iff_chunk(&mut body, b"COMM", &comm);
    let mut ssnd = vec![0u8; 8];
    ssnd.extend_from_slice(data);
    iff_chunk(&mut body, b"SSND", &ssnd);
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// The tempo as file names show it: "120 BPM", "97.5 BPM".
pub fn bpm_text(tempo: f64) -> String {
    let t = (tempo * 100.0).round() / 100.0;
    format!("{t} BPM")
}

/// A group's index as Maschine shows it: A1 … H1, then A2 ….
pub fn group_index(g: usize) -> String {
    format!("{}{}", (b'A' + (g % 8) as u8) as char, g / 8 + 1)
}

/// What a file is called, before its extension: `[name]` then, for a stem, ` - [NN]
/// [A1] [Group]` or ` - [Sound]`, then ` - [BPM]` (Maschine's rules).
pub fn file_stem(p: &Project, name: &str, stem: Stem) -> String {
    let middle = match stem {
        Stem::Master => String::new(),
        Stem::Group(g) => format!(" - {:02} {} {}", g + 1, group_index(g), group_name(p, g)),
        Stem::Sound(g, s) => format!(" - {}", sound_name(p, g, s)),
        Stem::Extra(i) => extra_stem_name(p, i).map(|n| format!(" - {n}")).unwrap_or_default(),
    };
    safe_name(&format!("{name}{middle} - {}", bpm_text(p.tempo)))
}

/// Extra-output strip `i`'s part of its stem's name: its owner's channel, then the
/// plug-in's name and the strip's label ("Kick - Portal Out 3 L").
pub fn extra_stem_name(p: &Project, i: usize) -> Option<String> {
    use deckard_model::extra_out::{Owner, plugin_at, strip_label};
    let x = p.extra_outs.get(i)?;
    let owner = match x.owner {
        Owner::Sound(g, s) | Owner::Chain(g, s, _) => sound_name(p, g, s),
        Owner::Group(g, _) => group_name(p, g),
        Owner::Master(_) => "Master".into(),
    };
    let plugin = plugin_at(p, x.owner).map(|r| r.name.trim().to_string()).unwrap_or_default();
    let label = strip_label(x.bus, &x.name, x.channels);
    Some(format!("{owner} - {}", format!("{plugin} {label}").trim()))
}

/// A group's name, or "Group A1" when it has none.
fn group_name(p: &Project, g: usize) -> String {
    let grp = p.groups.get(g).map(|x| x.name.clone()).unwrap_or_default();
    if grp.is_empty() { format!("Group {}", group_index(g)) } else { grp }
}

/// A sound's name, or "Sound 1" when it has none.
fn sound_name(p: &Project, g: usize, s: usize) -> String {
    let snd = p.groups.get(g).and_then(|x| x.sounds.get(s)).map(|x| x.name.clone()).unwrap_or_default();
    if snd.is_empty() { format!("Sound {}", s + 1) } else { snd }
}

/// A pattern's audio file (the pattern editor's): `[Group] - [Pattern] - [BPM]`, or in
/// Keyboard view (`sound`) `[Group] - [Pattern] [Sound] - [BPM]`.
pub fn pattern_file_stem(p: &Project, g: usize, pi: usize, sound: Option<usize>) -> String {
    let named = |n: &str, or: String| if n.is_empty() { or } else { n.to_string() };
    let grp = p.groups.get(g);
    let group = named(grp.map_or("", |x| x.name.as_str()), format!("Group {}", group_index(g)));
    let pattern = named(grp.and_then(|x| x.patterns.get(pi)).map_or("", |x| x.name.as_str()), format!("Pattern {}", pi + 1));
    let sound =
        sound.map(|s| format!(" {}", named(grp.and_then(|x| x.sounds.get(s)).map_or("", |x| x.name.as_str()), format!("Sound {}", s + 1))));
    safe_name(&format!("{group} - {pattern}{} - {}", sound.unwrap_or_default(), bpm_text(p.tempo)))
}

/// `name` without the characters file systems refuse.
pub fn safe_name(name: &str) -> String {
    name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c }).collect()
}

/// `dir/stem.ext`, or with ` - 1`, ` - 2` … added when that name is taken (nothing is
/// overwritten); `taken` are names already chosen in this export.
pub fn unique_path(dir: &Path, stem: &str, ext: &str, taken: &[PathBuf]) -> PathBuf {
    let free = |p: &PathBuf| !p.exists() && !taken.contains(p);
    let first = dir.join(format!("{stem}.{ext}"));
    if free(&first) {
        return first;
    }
    (1..).map(|n| dir.join(format!("{stem} - {n}.{ext}"))).find(free).unwrap()
}

/// The patterns of group `g` a bounce of sound `s` renders: those with its notes.
pub fn bounce_patterns(p: &Project, g: usize, s: usize) -> Vec<usize> {
    let Some(grp) = p.groups.get(g) else { return Vec::new() };
    (0..grp.patterns.len()).filter(|&pi| grp.patterns[pi].notes.iter().any(|n| n.sound == s)).collect()
}

/// The project a bounce of sound `s` renders: the sound alone, its effects in, before
/// its fader (its level and pan stay on the channel to play the bounce).
pub fn bounce_project(p: &Project, g: usize, s: usize) -> Project {
    let mut q = stem_project(p, Stem::Sound(g, s));
    if let Some(snd) = q.groups.get_mut(g).and_then(|x| x.sounds.get_mut(s)) {
        snd.level = deckard_model::level_from_db(0.0);
        snd.pan = 0.0;
        crate::drop_fader_lanes(&mut snd.modulators);
    }
    q
}

/// One file of an export: what plays, whose output, and its name (less the tempo and
/// extension, which [`file_stem`] adds).
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub range: Range,
    pub stem: Stem,
    pub name: String,
    /// The file's whole name (less its extension) when it has one of its own (a
    /// pattern's export); else [`file_stem`] makes it.
    pub file: Option<String>,
}

/// Write each of `jobs` into `dir` as `fmt` says, in order; `progress` hears (job,
/// fraction of it). Returns the files written; stops (removing nothing written whole)
/// when `cancel` is set.
pub fn export(
    p: &Project,
    project_path: Option<&Path>,
    jobs: &[Job],
    fmt: &Format,
    dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, f64),
) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut cache = SampleCache::default();
    let mut written: Vec<PathBuf> = Vec::new();
    for (k, job) in jobs.iter().enumerate() {
        let (q, mode, offset) = playing(&stem_project(p, job.stem), job.range);
        let out = stem_output(job.stem);
        let Some(mut r) = render_output(&q, project_path, mode, offset, fmt, &mut cache, cancel, &mut |f| progress(k, f), out) else {
            return Ok(written);
        };
        if fmt.normalize {
            normalize(&mut r.audio);
        }
        let info =
            fmt.loop_optimize.then(|| LoopInfo { tempo: r.tempo, beats: r.beats(), numerator: r.numerator, denominator: r.denominator });
        let bytes = encode(&r.audio, r.rate, fmt.file_type, fmt.depth, info);
        let stem = job.file.clone().unwrap_or_else(|| file_stem(p, &job.name, job.stem));
        let path = unique_path(dir, &stem, fmt.file_type.extension(), &written);
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{parse_aiff, parse_wav};
    use deckard_model::{Group, Note, Pattern, SampleRef, Sampler, Scene, Section, Sound, Zone};

    /// A WAV of a decaying 220 Hz tone, `seconds` long, in `dir`.
    fn tone(dir: &Path, seconds: f64) -> PathBuf {
        let path = dir.join(format!("tone-{seconds}.wav"));
        let n = (48_000.0 * seconds) as usize;
        let audio: Vec<f32> = (0..n)
            .flat_map(|i| {
                let t = i as f32 / 48_000.0;
                let v = (t * 220.0 * std::f32::consts::TAU).sin() * 0.5 * (1.0 - i as f32 / n as f32);
                [v, v]
            })
            .collect();
        std::fs::write(&path, encode(&audio, 48_000, FileType::Wav, BitDepth::Float32, None)).unwrap();
        path
    }

    fn sound(name: &str, sample: &Path) -> Sound {
        Sound {
            name: name.into(),
            level: deckard_model::level_from_db(0.0),
            instrument: "Sampler".into(),
            sampler: Some(Sampler {
                zones: vec![Zone {
                    sample: SampleRef { kind: 0, path: sample.to_string_lossy().into(), ..Default::default() },
                    root_key: 60,
                    key_range: (0, 127),
                    velocity_range: (0, 127),
                    gain: 1.0,
                    ..Default::default()
                }],
                velocity_to_volume: 0.0,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Two groups of a sound each (and an empty one), a bar's pattern each, two
    /// sections; group B's note on the last beat rings past the end. With the folder
    /// holding the samples.
    fn project() -> (tempfile::TempDir, Project) {
        let tmp = tempfile::tempdir().unwrap();
        let note = |position| Note { expr: None, sound: 0, position, length: PPQ / 2, pitch: 60, velocity: 127, probability: 100 };
        let short = tone(tmp.path(), 0.1);
        let long = tone(tmp.path(), 1.5);
        let group = |name: &str, snd: Sound, at: i64| Group {
            name: name.into(),
            level: deckard_model::level_from_db(0.0),
            sounds: vec![snd, Sound::default()],
            patterns: vec![Pattern { length: 4 * PPQ, notes: vec![note(at)], ..Default::default() }],
            ..Default::default()
        };
        let p = Project {
            name: "Song".into(),
            tempo: 120.0,
            numerator: 4,
            denominator: 4,
            master_level: deckard_model::level_from_db(0.0),
            groups: vec![
                group("Drums", sound("Kick", &short), 0),
                group("Keys", sound("Pad", &long), 3 * PPQ),
                Group { level: deckard_model::level_from_db(0.0), sounds: vec![Sound::default()], ..Default::default() },
            ],
            scenes: vec![
                Scene { name: "Intro".into(), patterns: vec![(0, 0)], ..Default::default() },
                Scene { name: "Verse".into(), patterns: vec![(0, 0), (1, 0)], ..Default::default() },
            ],
            sections: vec![
                Section { scene: Some(0), length: 4 * PPQ, auto: false },
                Section { scene: Some(1), length: 4 * PPQ, auto: false },
            ],
            ..Default::default()
        };
        (tmp, p)
    }

    fn fmt(loop_optimize: bool) -> Format {
        Format { rate: 48_000, loop_optimize, ..Format::default() }
    }

    fn go(p: &Project, range: Range, stem: Stem, f: &Format) -> Rendered {
        let (q, mode, offset) = playing(&stem_project(p, stem), range);
        render(&q, None, mode, offset, f, &mut SampleCache::default(), &AtomicBool::new(false), &mut |_| {}).unwrap()
    }

    fn energy(a: &[f32]) -> f32 {
        a.iter().map(|s| s * s).sum()
    }

    /// A bar at 120 BPM and 48 kHz.
    const BAR: usize = 96_000;

    #[test]
    fn the_file_runs_on_to_the_bar_after_the_tail() {
        let (_tmp, p) = project();
        let r = go(&p, Range::Song, Stem::Master, &fmt(false));
        assert_eq!(r.range_frames, 2 * BAR);
        assert_eq!(r.frames(), 3 * BAR, "the 1.5 s tone rings into a third bar, which is kept whole");
        assert!(energy(&r.audio[2 * BAR * 2..]) > 1.0, "the tail is there");
    }

    #[test]
    fn loop_optimize_keeps_the_range_and_wraps_the_tail_onto_the_start() {
        let (_tmp, p) = project();
        let plain = go(&p, Range::Song, Stem::Master, &fmt(false));
        let looped = go(&p, Range::Song, Stem::Master, &fmt(true));
        assert_eq!(looped.frames(), 2 * BAR);
        // The kick plays at the start either way; the start also carries the tone's tail.
        let start = |r: &Rendered| energy(&r.audio[..20_000]);
        assert!(start(&looped) > start(&plain) * 1.5, "{} vs {}", start(&looped), start(&plain));
        assert_eq!(&looped.audio[50_000 * 2..60_000 * 2], &plain.audio[50_000 * 2..60_000 * 2], "elsewhere the same");
    }

    #[test]
    fn renders_are_the_same_every_time() {
        let (_tmp, p) = project();
        assert_eq!(go(&p, Range::Song, Stem::Master, &fmt(false)), go(&p, Range::Song, Stem::Master, &fmt(false)));
    }

    #[test]
    fn group_stems_add_up_to_the_master() {
        let (_tmp, p) = project();
        let master = go(&p, Range::Song, Stem::Master, &fmt(true));
        let a = go(&p, Range::Song, Stem::Group(0), &fmt(true));
        let b = go(&p, Range::Song, Stem::Group(1), &fmt(true));
        assert!(energy(&a.audio) > 1.0 && energy(&b.audio) > 1.0);
        let worst = (0..master.audio.len()).map(|i| (master.audio[i] - a.audio[i] - b.audio[i]).abs()).fold(0f32, f32::max);
        assert!(worst < 1e-4, "off by {worst}");
        // A sound's stem is its group's here (one sound, the group at 0 dB).
        assert_eq!(go(&p, Range::Song, Stem::Sound(1, 0), &fmt(true)).audio, b.audio);
    }

    #[test]
    fn ranges_play_their_sections_scene_or_pattern() {
        let (_tmp, p) = project();
        let verse = go(&p, Range::Sections(1, 1), Stem::Master, &fmt(true));
        assert_eq!(verse.range_frames, BAR);
        assert_eq!(go(&p, Range::Scene(0), Stem::Master, &fmt(true)).range_frames, BAR);
        assert_eq!(go(&p, Range::Pattern(1, 0), Stem::Master, &fmt(true)).range_frames, BAR);
        assert_eq!(go(&p, Range::Sections(0, 1), Stem::Master, &fmt(true)).range_frames, 2 * BAR);
        // The intro alone has no tone.
        let intro = go(&p, Range::Sections(0, 0), Stem::Master, &fmt(false));
        assert_eq!(intro.frames(), BAR);
    }

    /// A range of sections starts the LFOs' song clock where the song has it there.
    #[test]
    fn a_section_export_carries_the_song_offset() {
        let (_tmp, mut p) = project();
        p.sections[0].length = 3 * PPQ;
        assert_eq!(playing(&p, Range::Sections(1, 1)).2, (3 * PPQ) as f64);
        assert_eq!(playing(&p, Range::Sections(0, 1)).2, 0.0);
        assert_eq!(playing(&p, Range::Song).2, 0.0);
    }

    /// Bounce renders the patterns that play the sound, before its fader: the sound
    /// alone at 0 dB, centred.
    #[test]
    fn a_bounce_is_the_sound_before_its_fader() {
        let (_tmp, mut p) = project();
        p.groups[1].sounds[0].level = deckard_model::level_from_db(-6.0);
        p.groups[1].patterns.push(Pattern { length: 4 * PPQ, ..Default::default() });
        assert_eq!(bounce_patterns(&p, 1, 0), [0], "only patterns with its notes");
        assert!(bounce_patterns(&p, 1, 1).is_empty());
        let stem = go(&p, Range::Pattern(1, 0), Stem::Sound(1, 0), &fmt(true));
        let (q, mode, offset) = playing(&bounce_project(&p, 1, 0), Range::Pattern(1, 0));
        let b = render(&q, None, mode, offset, &fmt(true), &mut SampleCache::default(), &AtomicBool::new(false), &mut |_| {}).unwrap();
        let worst = stem
            .audio
            .iter()
            .zip(&b.audio)
            .map(|(a, b)| (a * deckard_model::level_gain(deckard_model::level_from_db(6.0)) as f32 - b).abs())
            .fold(0f32, f32::max);
        assert!(energy(&b.audio) > 1.0 && worst < 1e-4, "off by {worst}");
    }

    #[test]
    fn the_export_rate_is_the_files() {
        let (_tmp, p) = project();
        let r = go(&p, Range::Scene(0), Stem::Master, &Format { rate: 96_000, ..fmt(true) });
        assert_eq!((r.rate, r.frames()), (96_000, 2 * BAR));
    }

    #[test]
    fn files_decode_back() {
        let audio: Vec<f32> = (0..2000).map(|i| ((i as f32) * 0.01).sin() * 0.8).collect();
        for t in [FileType::Wav, FileType::Aiff] {
            for (d, tol) in [(BitDepth::Int16, 1e-4), (BitDepth::Int24, 1e-6), (BitDepth::Float32, 0.0)] {
                let bytes = encode(&audio, 88_200, t, d, None);
                let s = if t == FileType::Wav { parse_wav(&bytes) } else { parse_aiff(&bytes) }.unwrap();
                assert_eq!((s.rate, s.frames.len()), (88_200, 1000), "{t:?} {d:?}");
                let worst = s.frames.iter().flatten().zip(&audio).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
                assert!(worst <= tol, "{t:?} {d:?}: off by {worst}");
            }
        }
    }

    #[test]
    fn a_loop_carries_its_tempo() {
        let info = LoopInfo { tempo: 97.5, beats: 8.0, numerator: 4, denominator: 4 };
        let bytes = encode(&[0.0; 8], 44_100, FileType::Wav, BitDepth::Int24, Some(info));
        let at = bytes.windows(4).position(|w| w == b"acid").expect("an acid chunk");
        let body = &bytes[at + 8..at + 32];
        assert_eq!(u32::from_le_bytes(body[12..16].try_into().unwrap()), 8);
        assert_eq!(f32::from_le_bytes(body[20..24].try_into().unwrap()), 97.5);
        assert!(parse_wav(&bytes).is_ok());
    }

    #[test]
    fn normalize_brings_the_peak_to_full_scale() {
        let mut a = vec![0.1, -0.25, 0.2, 0.0];
        normalize(&mut a);
        assert_eq!(a, vec![0.4, -1.0, 0.8, 0.0]);
        let mut silent = vec![0.0; 4];
        normalize(&mut silent);
        assert_eq!(silent, vec![0.0; 4]);
    }

    #[test]
    fn names_follow_maschine() {
        let (_tmp, p) = project();
        assert_eq!(file_stem(&p, "Song", Stem::Master), "Song - 120 BPM");
        assert_eq!(file_stem(&p, "Song", Stem::Group(1)), "Song - 02 B1 Keys - 120 BPM");
        assert_eq!(file_stem(&p, "Song", Stem::Sound(0, 0)), "Song - Kick - 120 BPM");
        assert_eq!(file_stem(&p, "Song", Stem::Sound(0, 1)), "Song - Sound 2 - 120 BPM");
        assert_eq!(group_index(9), "B2");
        assert_eq!(bpm_text(97.5), "97.5 BPM");
        assert_eq!(safe_name("a/b:c"), "a-b-c");
        // The pattern editor's: the group's, or (Keyboard view) the sound's.
        assert_eq!(pattern_file_stem(&p, 1, 0, None), "Keys - Pattern 1 - 120 BPM");
        assert_eq!(pattern_file_stem(&p, 1, 0, Some(0)), "Keys - Pattern 1 Pad - 120 BPM");
    }

    /// [`project`] with the test plug-in Split as the kick's effect (its chain's slot 0,
    /// passing its input on and half of it on its second pair), and a strip for that
    /// pair on Out 3.
    fn with_strip() -> (tempfile::TempDir, Project) {
        use deckard_model::extra_out::Owner;
        let (tmp, mut p) = project();
        p.groups[0].sounds[0].slots = vec![crate::tests::plugin_slot(&crate::tests::SPLIT)];
        p.extra_outs = vec![crate::tests::split_strip(Owner::Chain(0, 0, 0), 1, deckard_model::AudioDest::Ext(2))];
        (tmp, p)
    }

    /// [`go`], with Split playing the kick's chain's first slot, group B's first and the
    /// master's first, when the project puts a plug-in there.
    fn go_split(p: &Project, range: Range, stem: Stem, f: &Format) -> Rendered {
        use deckard_plugins::Slot;
        let mut cache = SampleCache::default();
        for slot in [Slot::Chain { group: 0, sound: 0, slot: 0 }, Slot::Group { group: 1, slot: 0 }, Slot::Master { slot: 0 }] {
            cache.test_plugins.push((slot, crate::tests::split_plugin(&crate::tests::SPLIT)));
        }
        let (q, mode, offset) = playing(&stem_project(p, stem), range);
        render_output(&q, None, mode, offset, f, &mut cache, &AtomicBool::new(false), &mut |_| {}, stem_output(stem)).unwrap()
    }

    /// A strip on a hardware output is a stem of its own: its pair alone (half the
    /// kick here), on both channels; the master's file is what comes out of the master,
    /// without it. However the project's own outputs are set.
    #[test]
    fn a_strip_on_an_output_exports_as_its_own_stem() {
        use deckard_model::extra_out::Channels;
        let (_tmp, p) = with_strip();
        let f = fmt(true);
        let kick = go_split(&p, Range::Song, Stem::Sound(0, 0), &f);
        let extra = go_split(&p, Range::Song, Stem::Extra(0), &f);
        assert!(energy(&kick.audio) > 1.0, "the kick plays");
        assert_eq!(extra.frames(), kick.frames());
        let worst = kick.audio.iter().zip(&extra.audio).map(|(k, x)| (k * 0.5 - x).abs()).fold(0f32, f32::max);
        assert!(worst < 1e-4, "the stem is the pair: off by {worst}");
        // The master's file doesn't hold it.
        let master = go_split(&p, Range::Song, Stem::Master, &f);
        let mut without = p.clone();
        without.extra_outs.clear();
        let plain = go_split(&without, Range::Song, Stem::Master, &f);
        let worst = master.audio.iter().zip(&plain.audio).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(worst < 1e-6, "the master holds the strip: off by {worst}");
        // The stem's private output is the master's, or another strip's, in the project:
        // still the strip alone.
        for edit in [
            |q: &mut Project| q.master_output = deckard_model::AudioDest::Ext(15),
            |q: &mut Project| q.cue_output = deckard_model::AudioDest::Ext(15),
            // (Group B plays: only its way to Out 16 is cut.)
            |q: &mut Project| q.groups[1].channel.dest = deckard_model::AudioDest::Ext(15),
            |q: &mut Project| q.extra_outs[0].dest = deckard_model::AudioDest::Ext(15),
            |q: &mut Project| {
                let mut other = q.extra_outs[0].clone();
                other.bus = 2;
                other.dest = deckard_model::AudioDest::Ext(15);
                q.extra_outs.push(other);
            },
        ] {
            let mut q = p.clone();
            edit(&mut q);
            let again = go_split(&q, Range::Song, Stem::Extra(0), &f);
            let worst = again.audio.iter().zip(&extra.audio).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            assert!(worst < 1e-6 && again.frames() == extra.frames(), "off by {worst}");
        }
        // A split half: its one side on both channels of its stem.
        let mut q = p.clone();
        q.extra_outs[0].channels = Channels::Left;
        let half = go_split(&q, Range::Song, Stem::Extra(0), &f);
        assert!(energy(&half.audio) > 0.1);
        assert!(half.audio.as_chunks::<2>().0.iter().all(|[l, r]| (l - r).abs() < 1e-6), "both channels");
    }

    /// Another channel's strip playing into Deckard stays out of a stem: the kick's
    /// strip into its group stays out of the other sound's stem (its owner is muted
    /// there, but plays on for a strip), and one into the master out of group B's.
    #[test]
    fn a_stem_holds_no_other_channels_strips() {
        use deckard_model::AudioDest;
        let (_tmp, mut p) = project();
        let f = fmt(true);
        for (dest, stem) in [(AudioDest::Parent, Stem::Sound(0, 1)), (AudioDest::Master, Stem::Group(1))] {
            let (_, q) = with_strip();
            p.groups[0].sounds[0].slots = q.groups[0].sounds[0].slots.clone();
            p.extra_outs = q.extra_outs.clone();
            p.extra_outs[0].dest = dest;
            let mut without = p.clone();
            without.extra_outs.clear();
            let off = worst(&go_split(&without, Range::Song, stem, &f), &go_split(&p, Range::Song, stem, &f), 1.0);
            assert!(off < 1e-6, "{stem:?} holds the kick's strip: off by {off}");
            assert!(stem_project(&p, stem).extra_outs[0].muted);
        }
        // The kick's own stem, and its group's, keep it.
        for stem in [Stem::Sound(0, 0), Stem::Group(0)] {
            assert!(!stem_project(&p, stem).extra_outs[0].muted, "{stem:?}");
        }
    }

    fn worst(a: &Rendered, b: &Rendered, k: f32) -> f32 {
        assert_eq!(a.frames(), b.frames());
        a.audio.iter().zip(&b.audio).map(|(a, b)| (a * k - b).abs()).fold(0f32, f32::max)
    }

    /// The first bar of a render (the kick alone plays there).
    fn first_bar(r: &Rendered) -> &[f32] {
        &r.audio[..BAR * 2]
    }

    /// A group's plug-in's strip: what goes into the group, from its own sounds or
    /// another group's (the kick, sent to group B), half of it on the strip's stem. The
    /// master's file is the group's main pair alone.
    #[test]
    fn a_group_plugins_strip_hears_what_feeds_the_group() {
        use deckard_model::extra_out::Owner;
        let (_tmp, mut p) = project();
        p.groups[1].slots = vec![crate::tests::plugin_slot(&crate::tests::SPLIT)];
        p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::Group(1);
        p.extra_outs = vec![crate::tests::split_strip(Owner::Group(1, 0), 1, deckard_model::AudioDest::Ext(2))];
        let f = fmt(true);
        let master = go_split(&p, Range::Song, Stem::Master, &f);
        let extra = go_split(&p, Range::Song, Stem::Extra(0), &f);
        assert!(energy(first_bar(&extra)) > 0.1, "the kick, from group A, reaches it");
        let off = worst(&master, &extra, 0.5);
        assert!(off < 1e-4, "the stem is half of what enters the group: off by {off}");
        let mut without = p.clone();
        without.extra_outs.clear();
        let off = worst(&master, &go_split(&without, Range::Song, Stem::Master, &f), 1.0);
        assert!(off < 1e-6, "the master holds the strip: off by {off}");
    }

    /// The master's plug-in's strip: the whole mix (with what goes straight to the
    /// master's output), half of it on the strip's stem; the master's file without it.
    #[test]
    fn a_master_plugins_strip_hears_the_mix() {
        use deckard_model::extra_out::Owner;
        let (_tmp, mut p) = project();
        p.master_slots = vec![crate::tests::plugin_slot(&crate::tests::SPLIT)];
        p.master_output = deckard_model::AudioDest::Ext(0);
        // The kick straight onto the master's output (the engine mixes it into the master).
        p.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::Ext(0);
        p.extra_outs = vec![crate::tests::split_strip(Owner::Master(0), 1, deckard_model::AudioDest::Ext(2))];
        let f = fmt(true);
        let master = go_split(&p, Range::Song, Stem::Master, &f);
        let extra = go_split(&p, Range::Song, Stem::Extra(0), &f);
        assert!(energy(first_bar(&extra)) > 0.1, "the kick, on the master's output, reaches it");
        let off = worst(&master, &extra, 0.5);
        assert!(off < 1e-4, "the stem is half the mix: off by {off}");
        let mut without = p.clone();
        without.extra_outs.clear();
        let off = worst(&master, &go_split(&without, Range::Song, Stem::Master, &f), 1.0);
        assert!(off < 1e-6, "the master holds the strip: off by {off}");
        // The master's output on Out 16 itself: the same.
        let mut q = p.clone();
        q.master_output = deckard_model::AudioDest::Ext(15);
        q.groups[0].sounds[0].channel.dest = deckard_model::AudioDest::Ext(15);
        let off = worst(&extra, &go_split(&q, Range::Song, Stem::Extra(0), &f), 1.0);
        assert!(off < 1e-6, "off by {off}");
    }

    #[test]
    fn the_extra_stem_is_named_after_its_strip() {
        use deckard_model::extra_out::{Channels, Owner};
        let (_tmp, mut p) = with_strip();
        assert_eq!(file_stem(&p, "Song", Stem::Extra(0)), "Song - Kick - Split Out 2 - 120 BPM");
        p.extra_outs[0].name = "Bass".into();
        p.extra_outs[0].channels = Channels::Left;
        assert_eq!(file_stem(&p, "Song", Stem::Extra(0)), "Song - Kick - Split Out 2 Bass L - 120 BPM");
        // A group's plug-in, the master's.
        p.groups[1].slots = vec![crate::tests::plugin_slot(&crate::tests::SPLIT)];
        p.extra_outs[0].owner = Owner::Group(1, 0);
        assert_eq!(file_stem(&p, "Song", Stem::Extra(0)), "Song - Keys - Split Out 2 Bass L - 120 BPM");
        p.master_slots = vec![crate::tests::plugin_slot(&crate::tests::SPLIT)];
        p.extra_outs[0].owner = Owner::Master(0);
        assert_eq!(file_stem(&p, "Song", Stem::Extra(0)), "Song - Master - Split Out 2 Bass L - 120 BPM");
    }

    #[test]
    fn empty_groups_and_sounds_are_not_listed() {
        let (_tmp, p) = project();
        assert_eq!(exportable_groups(&p), [0, 1]);
        assert_eq!(exportable_sounds(&p, 0), [0]);
    }

    #[test]
    fn export_writes_a_file_per_stem_without_overwriting() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("export");
        let (_tmp, p) = project();
        let jobs: Vec<Job> =
            [Stem::Group(0), Stem::Group(1)].map(|stem| Job { range: Range::Scene(1), stem, name: "Song".into(), file: None }).to_vec();
        let f = Format { normalize: true, ..fmt(true) };
        let mut seen = Vec::new();
        let files = export(&p, None, &jobs, &f, &dir, &AtomicBool::new(false), &mut |k, _| seen.push(k)).unwrap();
        let names: Vec<String> = files.iter().map(|f| f.file_name().unwrap().to_string_lossy().into()).collect();
        assert_eq!(names, ["Song - 01 A1 Drums - 120 BPM.wav", "Song - 02 B1 Keys - 120 BPM.wav"]);
        assert!(seen.contains(&0) && seen.contains(&1));
        let s = parse_wav(&std::fs::read(&files[1]).unwrap()).unwrap();
        let peak = s.frames.iter().flatten().fold(0f32, |m, v| m.max(v.abs()));
        assert!((peak - 1.0).abs() < 1e-3, "normalized: {peak}");
        // Again: new names beside the old.
        let again = export(&p, None, &jobs[..1], &f, &dir, &AtomicBool::new(false), &mut |_, _| {}).unwrap();
        assert_eq!(again[0].file_name().unwrap(), "Song - 01 A1 Drums - 120 BPM - 1.wav");
        // Cancelled: nothing more.
        let none = export(&p, None, &jobs, &f, &dir, &AtomicBool::new(true), &mut |_, _| {}).unwrap();
        assert!(none.is_empty());
    }

    /// What's rendered without a fader is without its modulation too.
    #[test]
    fn a_fader_left_out_takes_its_lane_with_it() {
        let lanes = || vec![deckard_model::ModTarget { slot: None, field: "level".into() }];
        let mut p = Project {
            groups: vec![deckard_model::Group {
                modulators: lanes(),
                sounds: vec![deckard_model::Sound { modulators: lanes(), ..Default::default() }],
                ..Default::default()
            }],
            ..Default::default()
        };
        p.groups[0].level = 0.5;
        let fader_of = |p: &Project| crate::channel_faders(p).0.iter().map(|f| f.sound).collect::<Vec<_>>();
        assert_eq!(fader_of(&p), [None, Some(0)]);
        assert_eq!(fader_of(&stem_project(&p, Stem::Group(0))), [None, Some(0)], "a group's stem is after its fader");
        assert_eq!(fader_of(&stem_project(&p, Stem::Sound(0, 0))), [Some(0)], "a sound's isn't through its group's");
        assert!(fader_of(&bounce_project(&p, 0, 0)).is_empty(), "a bounce is before its own");
    }
}
