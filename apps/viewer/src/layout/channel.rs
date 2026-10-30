//! Channel properties (docs/channel-software.md §1): the Input, Output, Groove and
//! Macro pages of the master, a group and a sound, as knobs, switches and choices.

use super::*;
use deckard_doc::SoundRoute;
use deckard_model::{AudioDest, AudioSource, MidiDest};

/// The four sets of properties, in the order of their tiles.
pub const SETS: [&str; 4] = ["Input", "Output", "Groove", "Macro"];

/// Which channel the Control area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Master,
    Group(usize),
    Sound(usize, usize),
}

/// A routing choice to make (what a pick in a routing list asks for).
#[derive(Debug, Clone, PartialEq)]
pub enum RouteSet {
    Sound(usize, usize, SoundRoute),
    /// A group's main output (`None`) or aux send.
    Group(usize, Option<usize>, AudioDest),
    GroupMidiSource(usize, String),
    /// The master's output (`false`) or the cue bus's.
    Master(bool, AudioDest),
    /// The side-chain source of the effect in slot `.2` of sound `.1` (group `.0`).
    SideChain(usize, usize, usize, Option<deckard_model::ScSource>),
}

/// The side-chain sources the effect in `slot` of chain `c` can listen to: none, any
/// group, any other sound. Only a sound's effects take one (`None` for a group's or the
/// master's).
pub fn sidechain_choices(p: &Project, c: deckard_doc::Chain, slot: usize) -> Option<(Vec<String>, Vec<RouteSet>, usize)> {
    use deckard_model::ScSource;
    let deckard_doc::Chain::Sound(g, s) = c else { return None };
    let mut options: Vec<(String, Option<ScSource>)> = vec![("None".into(), None)];
    for gi in 0..p.groups.len() {
        options.push((format!("{} - {}", group_id(gi), group_name(p, gi)), Some(ScSource::Group(gi))));
    }
    for (gi, grp) in p.groups.iter().enumerate() {
        for (si, snd) in grp.sounds.iter().enumerate() {
            if (gi, si) == (g, s) || (snd.slots.is_empty() && snd.name.is_empty()) {
                continue;
            }
            let name = if snd.name.is_empty() { format!("Sound {}", si + 1) } else { snd.name.clone() };
            options.push((format!("{}:S{} - {}: {name}", group_id(gi), si + 1, group_name(p, gi)), Some(ScSource::Sound(gi, si))));
        }
    }
    let cur_src = p.groups[g].sounds[s].channel.sidechain.iter().find(|(k, _)| *k == slot).map(|(_, x)| *x);
    let cur = options.iter().position(|(_, x)| *x == cur_src).unwrap_or(0);
    let (names, srcs): (Vec<String>, Vec<Option<ScSource>>) = options.into_iter().unzip();
    Some((names, srcs.into_iter().map(|x| RouteSet::SideChain(g, s, slot, x)).collect(), cur))
}

/// The action button at the end of a Groove page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrooveButton {
    ApplyVariation,
    Generate,
}

/// One page of channel properties.
#[derive(Default)]
pub struct Page {
    pub slots: Vec<Option<Knob>>,
    pub sections: Vec<(usize, Cow<'static, str>)>,
    /// The choices of routing selectors, by slot: names, and what each one sets.
    pub routes: Vec<(usize, Vec<String>, Vec<RouteSet>)>,
    pub button: Option<GrooveButton>,
    /// Shown instead of parameters.
    pub note: Option<String>,
}

impl Page {
    fn section(&mut self, name: &'static str) {
        self.sections.push((self.slots.len(), name.to_uppercase().into()));
    }

    fn knob(&mut self, (name, value, amount, edit): Knob) {
        self.slots.push(Some((name.to_uppercase().into(), value, amount, edit)));
    }

    fn empty(&mut self) {
        self.slots.push(None);
    }

    fn route(&mut self, name: &'static str, options: Vec<(String, RouteSet)>, cur: &RouteSet) {
        let i = options.iter().position(|(_, r)| r == cur);
        let value = i.map_or_else(|| "?".to_string(), |i| short(&options[i].0));
        let (names, sets): (Vec<String>, Vec<RouteSet>) = options.into_iter().unzip();
        self.routes.push((self.slots.len(), names, sets));
        self.knob((name.into(), value, 0.0, Some(Edit::Route(i.unwrap_or(usize::MAX)))));
    }
}

/// The name shown in a selector for a routing entry: the part before " - ",
/// [`abbrev`]iated.
fn short(name: &str) -> String {
    abbrev(name.split(" - ").next().unwrap_or(name))
}

/// A picked port or channel name as shown in its field: at most [`SHORT`] characters,
/// a long one its start and end with ".." between ("Focusrite USB" → "Foc..USB").
/// Menus, and the controller while its knob is touched, show the whole name.
pub fn abbrev(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= SHORT {
        return name.to_string();
    }
    let keep = SHORT - 2;
    let (head, tail) = (keep.div_ceil(2), keep / 2);
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{}..{}", start.trim_end(), end.trim_start())
}

/// The characters a picked name keeps ([`abbrev`]).
pub const SHORT: usize = 8;

/// Page names of a set for a channel (`macros`: the channel's macro page names).
pub fn page_names(level: Level, set: usize, macros: &[String]) -> Vec<String> {
    let names: &[&str] = match (level, set) {
        (Level::Sound(..), 0) => &["Audio", "MIDI"],
        (Level::Group(_), 0) => &["MIDI"],
        (Level::Master, 0) => &[],
        (Level::Sound(..), 1) => &["Audio", "Aux", "MIDI"],
        (Level::Group(_), 1) => &["Audio", "Aux"],
        (Level::Master, 1) => &["Audio"],
        (Level::Sound(..), 2) => &["Swing", "Variation", "Generate"],
        (_, 2) => &["Swing"],
        _ => return macros.to_vec(),
    };
    names.iter().map(|s| s.to_string()).collect()
}

pub const MIDI_CHANNELS_IN: [&str; 17] = ["All", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16"];
pub const MIDI_CHANNELS: [&str; 16] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16"];
pub const SWING_CYCLE_NAMES: [&str; 8] = ["1/8", "3/16", "1/4", "6/16", "1/2", "9/16", "1/1", "2/1"];
pub const ORDERS: [&str; 2] = ["Post", "Pre"];
pub const KEY_MODES: [&str; 3] = ["Off", "Drumkit", "Manual"];
pub const DISTRIBUTIONS: [&str; 3] = ["Even", "Natural", "Skewed"];
pub const GENERATE_MODES: [&str; 2] = ["Random", "Euclidean"];
pub const DIVISIONS: [&str; 11] = ["Grid", "1/4", "1/8", "1/16", "1/32", "1/8 T", "1/16 T", "1/4 Q", "1/8 Q", "1/16 Q", "Fit Bar"];

/// Maschine's external outputs and inputs (stereo pairs).
const EXT: usize = 16;

/// What the Control area needs to build a page.
pub struct Ctx<'a> {
    pub doc: &'a deckard_doc::Document,
    /// Sounds can send MIDI controllers (projects from Maschine 2.x on).
    pub has_midi_macro: bool,
    pub p: &'a Project,
    pub midi_ins: &'a [String],
    pub midi_outs: &'a [String],
    /// Notes in the piano roll (the Generate page's chords).
    pub polyphonic: bool,
    /// Variation and Generate settings of a sound (saved, or kept for the session).
    pub groove: &'a dyn Fn(usize, usize, &str) -> Option<f64>,
    /// The audio inputs and outputs: their aliases, and which are connected (the others
    /// aren't offered, unless picked).
    pub io: &'a crate::settings::Settings,
    /// The knob of a plug-in parameter (by object id), for a macro controlling it.
    pub plugin_knob: &'a dyn Fn(u64) -> Option<super::Knob>,
}

pub fn page(c: &Ctx, level: Level, set: usize, page: usize) -> Page {
    let mut pg = Page::default();
    match (level, set) {
        (Level::Master, 0) => pg.note = Some("The master has no input properties".into()),
        (Level::Sound(g, s), 0) if page == 0 => sound_audio_in(c, g, s, &mut pg),
        (Level::Sound(g, s), 0) => sound_midi_in(c, g, s, &mut pg),
        (Level::Group(g), 0) => group_midi_in(c, g, &mut pg),
        (Level::Master, 1) => master_out(c, &mut pg),
        (Level::Sound(g, s), 1) => match page {
            0 => audio_out(c, level, Target::Sound(g, s), &mut pg),
            1 => aux(c, level, Target::Sound(g, s), &mut pg),
            _ => sound_midi_out(c, g, s, &mut pg),
        },
        (Level::Group(g), 1) => match page {
            0 => audio_out(c, level, Target::GroupStrip(g), &mut pg),
            _ => aux(c, level, Target::GroupStrip(g), &mut pg),
        },
        (_, 2) => {
            let (t, swing) = match level {
                Level::Master => (Target::Song, c.p.swing),
                Level::Group(g) => (Target::Group(g), c.p.groups[g].swing),
                Level::Sound(g, s) => (Target::Sound(g, s), c.p.groups[g].sounds[s].swing),
            };
            match (level, page) {
                (Level::Sound(g, s), 1) => variation(c, g, s, &mut pg),
                (Level::Sound(g, s), 2) => generate(c, g, s, &mut pg),
                _ => {
                    // Amount, Cycle (a choice), Invert (a switch).
                    pg.section("Swing");
                    let mut knobs = swing_knobs(t, swing).into_iter();
                    let (_, value, amount, edit) = knobs.next().unwrap();
                    pg.knob(("Amount".into(), value, amount, edit));
                    let cycle = swing.cycle.clamp(0, 7) as usize;
                    pg.knob(choice("Cycle", t, "swing_cycle", &SWING_CYCLE_NAMES, cycle, 0.0));
                    pg.knob(toggle("Invert", t, "swing_invert", swing.invert));
                }
            }
        }
        _ => {}
    }
    pg
}

/// A level knob (stored 0..1, 0.75 = 0 dB).
fn level_knob(name: &'static str, t: Target, field: &'static str, v: f64) -> Knob {
    (name.into(), db(v), v as f32, Some(Edit::Param(t, field, 0.0, 1.0, 0.75)))
}

fn toggle(name: &'static str, t: Target, field: &'static str, on: bool) -> Knob {
    (name.into(), if on { "On" } else { "Off" }.into(), on as i32 as f32, Some(Edit::Toggle(t, field, on)))
}

fn choice(name: &'static str, t: Target, field: &'static str, options: &'static [&'static str], cur: usize, default: f64) -> Knob {
    let cur = cur.min(options.len() - 1);
    (name.into(), options[cur].to_string(), 0.0, Some(Edit::Choice(t, field, options, cur, default)))
}

fn sound_audio_in(c: &Ctx, g: usize, s: usize, pg: &mut Page) {
    let ch = &c.p.groups[g].sounds[s].channel;
    pg.section("Audio");
    let mut options = vec![("None".to_string(), AudioSource::None)];
    let io = c.io;
    let picked = |a: AudioSource| a == ch.audio_in;
    options.extend(
        (0..EXT)
            .filter(|&i| io.input_pair_on(i) || picked(AudioSource::Ext(i as i64)))
            .map(|i| (io.input_pair_name(i), AudioSource::Ext(i as i64))),
    );
    options.extend(
        (0..EXT * 2)
            .filter(|&i| io.input_on(i) || picked(AudioSource::Ext((EXT + i) as i64)))
            .map(|i| (io.input_name(i), AudioSource::Ext((EXT + i) as i64))),
    );
    let options = options.into_iter().map(|(n, a)| (n, RouteSet::Sound(g, s, SoundRoute::AudioIn(a)))).collect();
    pg.route("Source", options, &RouteSet::Sound(g, s, SoundRoute::AudioIn(ch.audio_in)));
    pg.knob(level_knob("Gain", Target::Sound(g, s), "input_gain", ch.input_gain));
}

/// MIDI input sources: the fixed ones, then the ports.
fn midi_sources(c: &Ctx, sound: bool) -> Vec<String> {
    let fixed: &[&str] = if sound { &["Default", "Focus", "None", "All"] } else { &["None", "All"] };
    let mut v: Vec<String> = fixed.iter().map(|s| s.to_string()).collect();
    v.extend(c.midi_ins.iter().cloned());
    v
}

fn sound_midi_in(c: &Ctx, g: usize, s: usize, pg: &mut Page) {
    let ch = &c.p.groups[g].sounds[s].channel;
    let t = Target::Sound(g, s);
    pg.section("MIDI");
    let mut names = midi_sources(c, true);
    if !names.contains(&ch.midi_source) {
        names.push(ch.midi_source.clone());
    }
    let options = names.into_iter().map(|n| (n.clone(), RouteSet::Sound(g, s, SoundRoute::MidiSource(n)))).collect();
    pg.route("Source", options, &RouteSet::Sound(g, s, SoundRoute::MidiSource(ch.midi_source.clone())));
    pg.knob(choice("Channel", t, "midi_channel", &MIDI_CHANNELS_IN, ch.midi_channel.max(0) as usize, 0.0));
    pg.knob(toggle("Thru", t, "midi_thru", ch.midi_thru));
}

fn group_midi_in(c: &Ctx, g: usize, pg: &mut Page) {
    let ch = &c.p.groups[g].channel;
    let t = Target::Group(g);
    pg.section("Routings");
    pg.knob(choice("Key Mode", t, "key_mode", &KEY_MODES, ch.key_mode.max(0) as usize, 0.0));
    if ch.key_mode != 2 {
        return;
    }
    let mut names = midi_sources(c, false);
    if !names.contains(&ch.midi_source) {
        names.push(ch.midi_source.clone());
    }
    let options = names.into_iter().map(|n| (n.clone(), RouteSet::GroupMidiSource(g, n))).collect();
    pg.route("Source", options, &RouteSet::GroupMidiSource(g, ch.midi_source.clone()));
    pg.knob(choice("Channel", t, "midi_channel", &MIDI_CHANNELS_IN, ch.midi_channel.max(0) as usize, 0.0));
    let n = ch.start_note;
    pg.knob(("Start Note".into(), note_name(n), n as f32 / 127.0, Some(Edit::Param(t, "start_note", 0.0, 127.0, 36.0))));
    pg.knob(toggle("Thru", t, "midi_thru", ch.midi_thru));
}

/// A sound that takes other channels' audio: an effect in its first slot.
fn is_bus(s: &deckard_model::Sound) -> bool {
    const SOURCES: [&str; 9] = ["Sampler", "Audio", "PluginHost", "Kick", "Snare", "HiHat", "Tom", "Percussion", "Cymbal"];
    s.slots.first().is_some_and(|x| !SOURCES.iter().any(|m| x.module.contains(m)))
}

/// Where a channel's main output (`aux` false) or an aux send can go (`picked`: where
/// it goes now, offered whatever it is).
fn destinations(c: &Ctx, level: Level, aux: bool, picked: AudioDest) -> Vec<(String, AudioDest)> {
    let mut v = vec![("None".to_string(), AudioDest::None)];
    match level {
        Level::Sound(..) if !aux => {
            v.push(("Group".into(), AudioDest::Parent));
            v.push(("Master".into(), AudioDest::Master));
        }
        Level::Group(_) if !aux => v.push(("Master".into(), AudioDest::Parent)),
        _ => v.push(("Master".into(), AudioDest::Master)),
    }
    let own = match level {
        Level::Group(g) | Level::Sound(g, _) => Some(g),
        Level::Master => None,
    };
    for (gi, grp) in c.p.groups.iter().enumerate() {
        // A group can't feed itself (a sound reaches its own group as "Group").
        if Some(gi) != own {
            v.push((format!("{} - {}", group_id(gi), group_name(c.p, gi)), AudioDest::Group(gi)));
        }
        for (si, snd) in grp.sounds.iter().enumerate() {
            let me = level == Level::Sound(gi, si);
            let inside = matches!(level, Level::Group(x) if x == gi);
            if is_bus(snd) && !me && !inside {
                let name = if snd.name.is_empty() { format!("Sound {}", si + 1) } else { snd.name.clone() };
                v.push((format!("{}:S{}-1 - {}", group_id(gi), si + 1, name), AudioDest::Sound(gi, si)));
            }
        }
    }
    v.extend(ext_outputs(c, Some(picked)));
    v
}

/// The connected external outputs (and `picked`), by their aliases.
fn ext_outputs(c: &Ctx, picked: Option<AudioDest>) -> Vec<(String, AudioDest)> {
    (0..EXT)
        .filter(|&i| c.io.output_pair_on(i) || picked == Some(AudioDest::Ext(i as i64)))
        .map(|i| (c.io.output_pair_name(i), AudioDest::Ext(i as i64)))
        .collect()
}

fn audio_out(c: &Ctx, level: Level, t: Target, pg: &mut Page) {
    pg.section("Audio");
    let (dest, cue, lvl, pan_v) = match level {
        Level::Sound(g, s) => {
            let snd = &c.p.groups[g].sounds[s];
            (snd.channel.dest, snd.cue, snd.level, snd.pan)
        }
        Level::Group(g) => {
            let grp = &c.p.groups[g];
            (grp.channel.dest, grp.cue, grp.level, grp.pan)
        }
        Level::Master => unreachable!(),
    };
    let set = |d: AudioDest| match level {
        Level::Sound(g, s) => RouteSet::Sound(g, s, SoundRoute::Output(d)),
        Level::Group(g) => RouteSet::Group(g, None, d),
        Level::Master => RouteSet::Master(false, d),
    };
    let options = destinations(c, level, false, dest).into_iter().map(|(n, d)| (n, set(d))).collect();
    pg.route("Dest.", options, &set(dest));
    pg.knob(toggle("Cue", t, "cue_enabled", cue));
    pg.knob(("Level".into(), db(lvl), lvl as f32, Some(Edit::Level(t))));
    pg.knob(("Pan".into(), pan(pan_v), (pan_v as f32 + 1.0) / 2.0, Some(Edit::Pan(t))));
    if let Level::Sound(g, s) = level {
        pg.knob(toggle("Audio Mute", t, "audio_mute", c.p.groups[g].sounds[s].channel.audio_mute));
    }
}

fn aux(c: &Ctx, level: Level, t: Target, pg: &mut Page) {
    let (dests, levels, pre) = match level {
        Level::Sound(g, s) => {
            let snd = &c.p.groups[g].sounds[s];
            (snd.channel.aux_dest, snd.aux, snd.channel.aux_pre)
        }
        Level::Group(g) => {
            let grp = &c.p.groups[g];
            (grp.channel.aux_dest, grp.aux, grp.channel.aux_pre)
        }
        Level::Master => unreachable!(),
    };
    for k in 0..2 {
        pg.section(if k == 0 { "Aux 1" } else { "Aux 2" });
        let set = |d: AudioDest| match level {
            Level::Sound(g, s) => RouteSet::Sound(g, s, SoundRoute::Aux(k, d)),
            Level::Group(g) => RouteSet::Group(g, Some(k), d),
            Level::Master => RouteSet::Master(false, d),
        };
        let options = destinations(c, level, true, dests[k]).into_iter().map(|(n, d)| (n, set(d))).collect();
        pg.route("Dest.", options, &set(dests[k]));
        let (lf, of) = if k == 0 { ("aux1_level", "aux1_order") } else { ("aux2_level", "aux2_order") };
        pg.knob(level_knob("Level", t, lf, levels[k]));
        pg.knob(choice("Order", t, of, &ORDERS, pre[k] as usize, 0.0));
        pg.empty();
    }
}

fn sound_midi_out(c: &Ctx, g: usize, s: usize, pg: &mut Page) {
    let ch = &c.p.groups[g].sounds[s].channel;
    let t = Target::Sound(g, s);
    pg.section("MIDI");
    let mut options = vec![("None".to_string(), MidiDest::None)];
    options.extend(c.midi_outs.iter().map(|p| (p.clone(), MidiDest::Port(p.clone()))));
    if let MidiDest::Port(p) = &ch.midi_out
        && !c.midi_outs.contains(p)
    {
        options.push((p.clone(), ch.midi_out.clone()));
    }
    for (si, snd) in c.p.groups[g].sounds.iter().enumerate() {
        if si != s && snd.plugin.is_some() {
            let name = if snd.name.is_empty() { format!("Sound {}", si + 1) } else { snd.name.clone() };
            options.push((format!("S{} - {name}", si + 1), MidiDest::Sound(si)));
        }
    }
    let options = options.into_iter().map(|(n, d)| (n, RouteSet::Sound(g, s, SoundRoute::MidiOut(d)))).collect();
    pg.route("Dest.", options, &RouteSet::Sound(g, s, SoundRoute::MidiOut(ch.midi_out.clone())));
    pg.knob(choice("Channel", t, "event_output_channel", &MIDI_CHANNELS, ch.midi_out_channel.clamp(0, 15) as usize, s.min(15) as f64));
    let tr = ch.transpose;
    pg.knob((
        "Transpose".into(),
        if tr > 0 { format!("+{tr}") } else { tr.to_string() },
        (tr as f32 + 48.0) / 96.0,
        Some(Edit::Param(t, "transpose", -48.0, 48.0, 0.0)),
    ));
}

fn master_out(c: &Ctx, pg: &mut Page) {
    let ext = |cue: bool| -> Vec<(String, RouteSet)> {
        let picked = if cue { c.p.cue_output } else { c.p.master_output };
        ext_outputs(c, Some(picked)).into_iter().map(|(n, d)| (n, RouteSet::Master(cue, d))).collect()
    };
    pg.section("Audio");
    pg.route("Output", ext(false), &RouteSet::Master(false, c.p.master_output));
    let lvl = c.p.master_level;
    pg.knob(("Level".into(), db(lvl), lvl as f32, Some(Edit::Level(Target::Song))));
    let mp = c.p.master_pan;
    pg.knob(("Pan".into(), pan(mp), (mp as f32 + 1.0) / 2.0, Some(Edit::Pan(Target::Song))));
    pg.section("Cue");
    pg.route("Output", ext(true), &RouteSet::Master(true, c.p.cue_output));
    pg.knob(level_knob("Level", Target::Song, "cue_level", c.p.cue_level));
    let cp = c.p.cue_pan;
    pg.knob(("Pan".into(), pan(cp), (cp as f32 + 1.0) / 2.0, Some(Edit::Param(Target::Song, "cue_pan", -1.0, 1.0, 0.0))));
}

fn percent(name: &'static str, t: Target, field: &'static str, v: f64, lo: f64, default: f64) -> Knob {
    (name.into(), format!("{v:.0} %"), ((v - lo) / (100.0 - lo)) as f32, Some(Edit::Param(t, field, lo, 100.0, default)))
}

/// The Variation page's note length scale (0..1, 0.5 = as they are): ×¼ … ×4.
pub fn length_scale(v: f64) -> f64 {
    4f64.powf((v - 0.5) * 2.0)
}

fn variation(c: &Ctx, g: usize, s: usize, pg: &mut Page) {
    let t = Target::Sound(g, s);
    let f = |k: &str, d: f64| (c.groove)(g, s, &format!("sequence_variation/{k}")).unwrap_or(d);
    pg.section("Velocity");
    pg.knob(percent("Amount", t, "sequence_variation/velocity_amount", f("velocity_amount", 0.0), 0.0, 0.0));
    pg.section("Time Shift");
    pg.knob(percent("Step", t, "sequence_variation/time_shift", f("time_shift", 0.0), 0.0, 0.0));
    pg.section("Note Length");
    let ls = f("length_scale", 0.5);
    pg.knob((
        "Steps".into(),
        format!("x{:.2}", length_scale(ls)),
        ls as f32,
        Some(Edit::Param(t, "sequence_variation/length_scale", 0.0, 1.0, 0.5)),
    ));
    pg.knob(percent("Variation", t, "sequence_variation/length_variation", f("length_variation", 0.0), 0.0, 0.0));
    pg.empty();
    pg.empty();
    pg.section("Spread");
    pg.knob(choice("Distribution", t, "sequence_variation/distribution", &DISTRIBUTIONS, f("distribution", 1.0) as usize, 1.0));
    pg.knob(toggle("Live", t, "sequence_variation/live", f("live", 0.0) != 0.0));
    pg.button = Some(GrooveButton::ApplyVariation);
}

fn generate(c: &Ctx, g: usize, s: usize, pg: &mut Page) {
    let t = Target::Sound(g, s);
    let f = |k: &str, d: f64| (c.groove)(g, s, &format!("sequence_generation/{k}")).unwrap_or(d);
    let mode = f("mode", 0.0) as usize;
    pg.section("Engine");
    pg.knob(choice("Mode", t, "sequence_generation/mode", &GENERATE_MODES, mode, 0.0));
    if mode == 0 {
        pg.section("Probability");
        pg.knob(percent("Amount", t, "sequence_generation/probability", f("probability", 55.0), 10.0, 55.0));
        pg.section("Velocity");
        pg.knob(percent("Amount", t, "sequence_generation/velocity_amount", f("velocity_amount", 10.0), 0.0, 10.0));
        if c.polyphonic {
            pg.section("Note Range");
            for (name, field, d) in
                [("Note Lo", "sequence_generation/note_range/low", 60.0), ("Note Hi", "sequence_generation/note_range/high", 72.0)]
            {
                let n = (c.groove)(g, s, field).unwrap_or(d) as i64;
                pg.knob((name.into(), note_name(n), n as f32 / 127.0, Some(Edit::Param(t, field, 0.0, 127.0, d))));
            }
            pg.section("Chords");
            let n = f("note_count", 1.0);
            pg.knob((
                "Note Count".into(),
                format!("{n:.0}"),
                ((n - 1.0) / 5.0) as f32,
                Some(Edit::Param(t, "sequence_generation/note_count", 1.0, 6.0, 1.0)),
            ));
        } else {
            for _ in 0..3 {
                pg.empty();
            }
        }
        pg.section("Spread");
        pg.knob(choice("Distribution", t, "sequence_generation/distribution", &DISTRIBUTIONS, f("distribution", 1.0) as usize, 1.0));
        pg.empty();
    } else {
        let division = f("division", 0.0) as usize;
        let max = max_steps(division);
        pg.section("Steps");
        let n = f("steps", 8.0);
        pg.knob((
            "Amount".into(),
            format!("{n:.0}"),
            ((n - 2.0) / (max - 2.0)) as f32,
            Some(Edit::Param(t, "sequence_generation/steps", 2.0, max, 8.0)),
        ));
        pg.section("Fills");
        let k = f("fills", 5.0);
        pg.knob((
            "Amount".into(),
            format!("{k:.0}"),
            ((k - 1.0) / 7.0) as f32,
            Some(Edit::Param(t, "sequence_generation/fills", 1.0, 8.0, 5.0)),
        ));
        pg.section("Rotate");
        let r = f("rotate", 0.0);
        pg.knob(("Steps".into(), format!("{r:.0}"), (r / 7.0) as f32, Some(Edit::Param(t, "sequence_generation/rotate", 0.0, 7.0, 0.0))));
        pg.section("Division");
        pg.knob(choice("Division", t, "sequence_generation/division", &DIVISIONS, division, 0.0));
        pg.empty();
        pg.empty();
        pg.knob(toggle("Live", t, "sequence_generation/live_mode", f("live_mode", 0.0) != 0.0));
    }
    pg.button = Some(GrooveButton::Generate);
}

/// The most Euclidean steps for a division (`recalcMaxSteps`; Grid: up to a bar of
/// the grid's steps).
pub fn max_steps(division: usize) -> f64 {
    match division {
        1 => 4.0,
        2 => 8.0,
        3 => 16.0,
        5 => 12.0,
        6 => 24.0,
        7 => 5.0,
        8 => 10.0,
        9 => 20.0,
        _ => 32.0,
    }
}

// ---- macros ------------------------------------------------------------------------

/// A parameter a macro can control: where it is in the target picker, its knob as its
/// own page shows it, and the parameter.
pub struct MacroTarget {
    pub path: Vec<String>,
    pub knob: Knob,
    pub t: Target,
    pub field: &'static str,
}

/// The parameter a knob changes, if a macro can control it.
pub fn knob_param(edit: &Edit) -> Option<(Target, &'static str)> {
    match *edit {
        Edit::Level(t) => Some((t, "level")),
        Edit::Pan(t) => Some((t, "pan")),
        Edit::Swing(t, _, SwingField::Amount) => Some((t, "swing_amount")),
        Edit::Param(t, f, ..) | Edit::Log(t, f, ..) | Edit::Int(t, f, ..) | Edit::Choice(t, f, ..) | Edit::Toggle(t, f, _) => Some((t, f)),
        _ => None,
    }
}

/// The channel a parameter belongs to (whose macros, and those above, can control it).
pub fn param_level(t: Target) -> Option<Level> {
    match t {
        Target::Song => Some(Level::Master),
        Target::Group(g) | Target::GroupStrip(g) => Some(Level::Group(g)),
        Target::Sound(g, s) | Target::Sampler(g, s) | Target::Zone(g, s, _) => Some(Level::Sound(g, s)),
        _ => None,
    }
}

/// The document target of a channel's macros.
pub fn macro_channel(level: Level) -> Target {
    match level {
        Level::Master => Target::Song,
        Level::Group(g) => Target::Group(g),
        Level::Sound(g, s) => Target::Sound(g, s),
    }
}

/// The MIDI controller parameters of a sound's MIDI output (`CC 0` … `CC 127`).
fn cc_fields() -> &'static [&'static str] {
    static F: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    F.get_or_init(|| (0..128).map(|i| &*Box::leak(format!("midi_macro/parameters/p{i}").into_boxed_str())).collect())
}

/// Every knob of a channel's own pages (not macros, not routing), with where it is.
fn own_targets(c: &Ctx, level: Level, prefix: &[String], out: &mut Vec<MacroTarget>) {
    for (set, set_name) in SETS.iter().enumerate().take(3) {
        for (pi, page_name) in page_names(level, set, &[]).iter().enumerate() {
            let pg = page(c, level, set, pi);
            for k in pg.slots.into_iter().flatten() {
                let Some(edit) = &k.3 else { continue };
                let Some((t, field)) = knob_param(edit) else { continue };
                let mut path = prefix.to_vec();
                path.extend([set_name.to_string(), page_name.clone(), title(&k.0)]);
                out.push(MacroTarget { path, knob: k, t, field });
            }
        }
    }
    // The internal effects in its chain ("Slots"), page by page.
    let chain = match level {
        Level::Master => deckard_doc::Chain::Master,
        Level::Group(g) => deckard_doc::Chain::Group(g),
        Level::Sound(g, s) => deckard_doc::Chain::Sound(g, s),
    };
    for (i, slot) in c.doc.chain_slots(chain).unwrap_or(&[]).iter().enumerate() {
        let Some(pages) = crate::module_ui::pages(&slot.module) else { continue };
        let t = match chain {
            deckard_doc::Chain::Sound(g, s) => Target::Module(g, s, i),
            ch => Target::ChainModule(ch, i),
        };
        for (pi, page_name) in pages.iter().enumerate() {
            for k in crate::module_ui::page_for(slot, t, pi).slots.into_iter().flatten() {
                let Some(edit) = &k.3 else { continue };
                let Some((t, field)) = knob_param(edit) else { continue };
                let mut path = prefix.to_vec();
                path.extend(["Slots".to_string(), format!("{} {}", i + 1, slot.name), page_name.to_string(), title(&k.0)]);
                out.push(MacroTarget { path, knob: k, t, field });
            }
        }
    }
    if let Level::Sound(g, s) = level {
        let snd = &c.p.groups[g].sounds[s];
        if let Some(smp) = &snd.sampler {
            for (pi, name) in sampler_ui::PAGES.iter().enumerate() {
                for k in sampler_ui::page(smp, g, s, pi).slots.into_iter().flatten() {
                    let Some(edit) = &k.3 else { continue };
                    let Some((t, field)) = knob_param(edit) else { continue };
                    let mut path = prefix.to_vec();
                    path.extend(["Sampler".to_string(), name.to_string(), title(&k.0)]);
                    out.push(MacroTarget { path, knob: k, t, field });
                }
            }
        }
        if prefix.is_empty() && c.has_midi_macro {
            let t = Target::Sound(g, s);
            for (i, f) in cc_fields().iter().enumerate() {
                let v = c.doc.num(t, f).unwrap_or(0.0);
                let bank = format!("CC {}-{}", i / 32 * 32, i / 32 * 32 + 31);
                let knob: Knob =
                    (format!("CC {i}").into(), format!("{v:.0}"), (v / 127.0) as f32, Some(Edit::Param(t, f, 0.0, 127.0, 0.0)));
                out.push(MacroTarget { path: vec!["MIDI".into(), bank, format!("CC {i}")], knob, t, field: f });
            }
        }
    }
}

/// A knob label in title case ("CUTOFF" → "Cutoff").
pub(crate) fn title(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::new();
    let mut up = true;
    for ch in lower.chars() {
        out.extend(if up { ch.to_uppercase().collect::<Vec<_>>() } else { vec![ch] });
        up = ch == ' ' || ch == '/' || ch == '(';
    }
    out
}

/// What the macros of a channel can control: its own parameters, and its sounds'
/// (a group's) or its groups' (the master's).
pub fn macro_targets(c: &Ctx, level: Level) -> Vec<MacroTarget> {
    let mut out = Vec::new();
    own_targets(c, level, &[], &mut out);
    match level {
        Level::Group(g) => {
            for (s, snd) in c.p.groups[g].sounds.iter().enumerate() {
                let name = if snd.name.is_empty() { format!("Sound {}", s + 1) } else { snd.name.clone() };
                own_targets(c, Level::Sound(g, s), &["Sounds".into(), format!("S{} - {name}", s + 1)], &mut out);
            }
        }
        Level::Master => {
            for g in 0..c.p.groups.len() {
                own_targets(c, Level::Group(g), &["Groups".into(), format!("{} - {}", group_id(g), group_name(c.p, g))], &mut out);
            }
        }
        Level::Sound(..) => {}
    }
    out
}

/// Every parameter a knob turns anywhere in the project (each channel's properties,
/// Sampler and internal effects), for MIDI learn and host automation.
pub fn every_target(c: &Ctx) -> Vec<MacroTarget> {
    let mut levels = vec![Level::Master];
    for (g, grp) in c.p.groups.iter().enumerate() {
        levels.push(Level::Group(g));
        levels.extend((0..grp.sounds.len()).map(|s| Level::Sound(g, s)));
    }
    let mut out = Vec::new();
    for level in levels {
        let has_midi_macro =
            matches!(level, Level::Sound(g, s) if c.doc.param_oid(Target::Sound(g, s), "midi_macro/parameters/p0").is_some());
        own_targets(&Ctx { has_midi_macro, ..*c }, level, &[], &mut out);
    }
    out
}

/// The macro page: the parameters its macros control (as their own knobs, named as
/// the macros are), "Macro n" for empty ones. Also, per slot, the target it controls.
pub fn macro_page(c: &Ctx, level: Level, page: usize, targets: &[MacroTarget]) -> (Page, Vec<Option<usize>>) {
    let mut pg = Page::default();
    let pages = c.doc.macro_pages(macro_channel(level));
    let Some(mp) = pages.get(page) else { return (pg, Vec::new()) };
    let ids: Vec<Option<u64>> = targets.iter().map(|t| c.doc.param_oid(t.t, t.field)).collect();
    let mut which = Vec::new();
    for (i, m) in mp.macros.iter().enumerate() {
        if !m.section.is_empty() {
            pg.sections.push((i, m.section.to_uppercase().into()));
        }
        let found = m.target.and_then(|id| ids.iter().position(|x| *x == Some(id)));
        which.push(found);
        // (A plug-in's parameter isn't among the targets: its knob comes from the plug-in.)
        let plugin = m.target.filter(|_| found.is_none()).and_then(|id| (c.plugin_knob)(id));
        match (m.target, found) {
            (Some(_), None) if plugin.is_some() => {
                let (name, value, amount, edit) = plugin.unwrap();
                let name: Cow<'static, str> = if m.name.is_empty() { name } else { m.name.to_uppercase().into() };
                pg.slots.push(Some((name, value, amount, edit)));
            }
            (Some(_), Some(k)) => {
                let (name, value, amount, edit) = targets[k].knob.clone();
                let name: Cow<'static, str> = if m.name.is_empty() { name } else { m.name.to_uppercase().into() };
                pg.slots.push(Some((name, value, amount, edit)));
            }
            (Some(_), None) => {
                let name = if m.name.is_empty() { "(OTHER)".to_string() } else { m.name.to_uppercase() };
                pg.slots.push(Some((name.into(), "not here yet".into(), 0.0, None)));
            }
            (None, _) => pg.slots.push(Some((format!("MACRO {}", page * 8 + i + 1).into(), String::new(), 0.0, None))),
        }
    }
    (pg, which)
}

/// A macro page's tab name: its own, else its sections', else "Page n".
pub fn macro_page_name(p: &deckard_doc::MacroPage, i: usize) -> String {
    if !p.name.is_empty() {
        return p.name.clone();
    }
    let sections: Vec<&str> = p.macros.iter().map(|m| m.section.as_str()).filter(|s| !s.is_empty()).collect();
    if sections.is_empty() { format!("Page {}", i + 1) } else { sections.join(" / ") }
}

#[cfg(test)]
mod io_tests {
    use super::*;

    /// The output and input pickers show the connected channels by their aliases; one
    /// not connected isn't offered, unless it's the one picked.
    #[test]
    fn pickers_offer_connected_channels_by_alias() {
        let doc = deckard_doc::Document::new_project().unwrap();
        let mut io = crate::settings::Settings::default();
        io.output_routing[2..4].copy_from_slice(&[Some(2), Some(3)]);
        io.output_aliases[2] = "Monitors".into();
        io.output_aliases[3] = "Monitors".into();
        io.input_aliases[0] = "Mic".into();
        let groove = |_: usize, _: usize, _: &str| None;
        let c = Ctx {
            doc: &doc,
            has_midi_macro: false,
            p: doc.project(),
            midi_ins: &[],
            midi_outs: &[],
            polyphonic: false,
            groove: &groove,
            io: &io,
            plugin_knob: &|_| None,
        };
        let names = |picked: AudioDest| destinations(&c, Level::Sound(0, 0), false, picked).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        let shown = names(AudioDest::Parent);
        assert!(shown.contains(&"Ext. 1".into()) && shown.contains(&"Monitors".into()), "{shown:?}");
        assert!(!shown.contains(&"Ext. 3".into()), "not connected: {shown:?}");
        assert!(names(AudioDest::Ext(2)).contains(&"Ext. 3".into()), "kept while picked");
        assert_eq!((io.input_name(0), io.input_pair_name(0)), ("Mic".into(), "Mic / In 1 R".into()));
    }

    /// A picked name shows in 8 characters: a long one's start and end around "..".
    #[test]
    fn long_names_are_shortened_in_the_middle() {
        assert_eq!(abbrev("Ext. 1"), "Ext. 1");
        assert_eq!(abbrev("Monitors"), "Monitors");
        assert_eq!(abbrev("Focusrite USB"), "Foc..USB");
        assert_eq!(abbrev("Mic / In 1 R").chars().count(), 8);
        assert_eq!(short("A:S1-1 - Kick"), "A:S1-1");
    }
}
