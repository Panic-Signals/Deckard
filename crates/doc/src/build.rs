//! Objects built from scratch: Maschine's own defaults, with no file to copy them from.
//!
//! A blank object comes from running a class's loader with nothing to read
//! (`mxprj::decode::blank`): every number 0, every text empty, collections empty,
//! pointers null except smart pointers' objects. [`DEFAULTS`] then gives the values that
//! are not 0 in a new object as Maschine 3 makes it (its plug-in's default state, an
//! empty project, and the module constructors), and the builders below add the parts a
//! new project, group or sound has: one song, group, scene, 16 sounds, a macro page and
//! velocity/probability lanes per channel, and so on.
//!
//! Class versions come from the project being edited where it has objects of a class
//! (an archive holds one version per class), else from Maschine 3's
//! ([`mxprj::types::classes`]).

use mxprj::decode::{BlankSpec, Obj, Value};
use mxprj::types::{Types, classes};
use std::collections::HashMap;

pub(crate) const PROJECT: &str = "NI::MASCHINE::DATA::Project";
pub(crate) const SONG: &str = "NI::MASCHINE::DATA::Song";
pub(crate) const GROUP: &str = "NI::MASCHINE::DATA::Group";
pub(crate) const SOUND: &str = "NI::MASCHINE::DATA::Sound";
const SOUND_CONFIG: &str = "NI::MASCHINE::DATA::SoundConfig";
const SCENE: &str = "NI::MASCHINE::DATA::Scene";
const PATTERN_REF: &str = "NI::MASCHINE::DATA::PatternRef";
const MIXING_LAYER: &str = "NI::MASCHINE::DATA::MixingLayer";
const MACRO_PAGE: &str = "NI::MASCHINE::DATA::MacroPage";
const MIDI_AUTOMATOR: &str = "NI::MASCHINE::DATA::MidiAutomator";
const REMOTE_PARAMETER: &str = "NI::MASCOMMON::DATA::RemoteParameter";
const QUICK_BROWSE: &str = "NI::DB3::QuickBrowseHandle";
const SOUND_INFO: &str = "NI::SOUND::SoundInfo2";
const ARP_PRESET: &str = "NI::MASCOMMON::DATA::ArpPreset";
const PATTERN: &str = "NI::MASCHINE::DATA::EventPattern";
const SECTION: &str = "NI::MASCHINE::DATA::Section";
const ZONE: &str = "NI::MASCOMMON::DATA::Zone";
const TIMED_EVENT: &str = "NI::MASCOMMON::DATA::TimedEvent";
const NOTE_EVENT: &str = "NI::MASCOMMON::DATA::NoteEvent";
const MIDI_EVENT: &str = "NI::MASCHINE::DATA::MidiEvent";
const AUDIO_EVENT: &str = "NI::MASCHINE::DATA::AudioEvent";
const EVENT_NODE: &str = "boost::multi_index::detail::index_node_base<NI::MASCOMMON::DATA::TimedEvent>";
const STRUM_PAIR: &str = "std::pair<NI::MASCOMMON::HW::StrumMode const, std::vector<std::vector<unsigned long>>>";

/// A value as stored: a parameter's (its `value`, and `t0` which mirrors it) or a
/// plain field's.
#[derive(Debug, Clone, Copy)]
pub(crate) enum D {
    I(i64),
    F(f64),
    S(&'static str),
}

use D::{F, I, S};

/// Values that are not 0 in new objects, by class: a field path (`a/b` is field `b`
/// of object `a`, through base classes) and the value.
#[rustfmt::skip]
const DEFAULTS: &[(&str, &[(&str, D)])] = &[
    (PROJECT, &[("unknown_pair_lo", I(-1)), ("unknown_pair_hi", I(-1))]),
    ("NI::MASCHINE::DATA::WorkspaceConfig", &[
        ("b01", I(1)), ("b20", I(1)), ("i30", I(1)), ("b39", I(1)), ("b3b", I(1)), ("f3c", F(0.75)), ("b41", I(1)),
    ]),
    (SONG, &[
        ("tempo", F(120.0)), ("numerator", I(4)), ("denominator", I(2)),
        ("master_output/value/type", I(3)), ("level", F(0.75)),
        ("cue_output/value/type", I(3)), ("cue_level", F(0.75)),
        ("f170", I(1)), ("loop/value/length", I(16)), ("f180", I(2)), ("num_section_banks", I(1)), ("f1c8", I(2)),
        ("pattern_editor_zoom", F(1.0)), ("f200", I(1)), ("pattern_editor_snap", I(4)),
        ("pattern_editor_snap_enabled", I(1)), ("pattern_editor_nudge_snap", I(1)),
        ("clip_editor_zoom", F(1.0)), ("clip_editor_zoom_hw", F(1.0)), ("arranger_zoom", F(1.0)), ("arranger_zoom_hw", F(1.0)),
        ("f268", I(273)), ("focus_entity", I(1)), ("perform_grid", I(5)), ("arrange_grid_snap", I(1)),
        ("pattern_length_quick", I(1)), ("fixed_velocity", I(100)), ("default_velocity_jam", I(100)),
        ("accent_velocity_jam", I(127)), ("sounds_visible_mixer", I(1)), ("functional_blocks_visible_mixer", I(1)),
        ("f310", I(1)), ("f330_ul", I(1)), ("enum_a", I(4)),
        ("arpeggiator/pb0/t1", I(1)), ("slot_arpeggiator/pb0", I(1)),
        ("f_ul_a", I(-1)), ("f_ul_b/t1", I(-1)), ("f_ul_c", I(-1)), ("f_bool_tail", I(1)),
    ]),
    (MIXING_LAYER, &[
        ("macro_page/t1", I(-1)), ("swing_bounce", I(1)),
        ("parameter_snapshots/morph_time", I(1)), ("parameter_snapshots/morph_grid", I(3)),
        ("parameter_snapshots/banks", I(1)),
    ]),
    ("NI::MASCHINE::DATA::MidiRecall", &[("input", S("None")), ("midi_channel", I(1))]),
    (GROUP, &[
        ("event_input", S("None")), ("start_note", I(36)),
        ("audio_output/value/type", I(1)), ("audio_output/value/index", I(-1)), ("level", F(0.75)),
        ("aux1_output/value/index", I(-1)), ("aux2_output/value/index", I(-1)),
        ("color/color", I(9)), ("color/use_default_color", I(1)),
        ("unknown_ulong_d", I(60)), ("unknown_ulong_f/t1", I(1)), ("unknown_ulong_h", I(3)), ("unknown_ulong_i", I(3)),
        ("unknown_ulong_k", I(9)), ("unknown_enum_c", I(1)),
    ]),
    (SOUND, &[
        ("midi_thru", I(1)), ("input_gain", F(0.75)), ("level", F(0.75)), ("base_key", I(60)),
        ("color/color", I(1)), ("color/use_default_color", I(1)),
        ("pianoroll_offset_y", I(265)), ("pianoroll_offset_y_hw", I(265)), ("pianoroll_zoom_y", F(1.0)),
    ]),
    ("NI::MASCHINE::DATA::SequenceVariationModule", &[("length_scale", F(0.5)), ("distribution", I(1))]),
    ("NI::MASCHINE::DATA::SequenceGenerationModule", &[
        ("probability", F(55.0)), ("velocity_amount", F(10.0)), ("note_range/low", I(60)), ("note_range/high", I(72)),
        ("note_count", I(1)), ("distribution", I(1)), ("steps", I(8)), ("fills", I(5)),
    ]),
    (SOUND_CONFIG, &[
        ("midi_source", S("Default")), ("midi_thru", I(1)), ("midi_source_plugin", S("Default")),
        ("audio_input/index", I(-1)), ("input_gain", F(0.75)), ("audio_output/type", I(1)), ("audio_output/index", I(-1)),
        ("aux1_output/index", I(-1)), ("aux2_output/index", I(-1)), ("event_output/port", S("None")), ("swing_bounce", I(1)),
    ]),
    (SCENE, &[("name", S("Scene 1")), ("color/use_default_color", I(1)), ("tempo", F(120.0))]),
    ("NI::MASCHINE::DATA::Recorder", &[
        ("input/value/type", I(3)), ("input/value/index", I(-1)), ("recording_mode", I(1)), ("detect_threshold", F(-12.0)),
    ]),
    ("NI::MASCOMMON::DATA::ScaleEngine", &[
        ("scale/t1", I(1)), ("key_mode", I(1)), ("chord_type_chromatic", I(3)), ("pe0", I(3)), ("chord_position", I(9)),
    ]),
    ("NI::MASCOMMON::DATA::Arpeggiator", &[
        ("pb8", I(12)), ("dynamic", F(1.0)), ("pc0", I(240)), ("gate", F(1.0)), ("offset", I(8)), ("max_key", I(127)),
    ]),
    (ARP_PRESET, &[("f30", F(1.0)), ("f34", F(1.0)), ("i50", I(8)), ("i68", I(127))]),
    ("NI::MASCHINE::DATA::ArrangerState", &[("idea_space_focused", I(1))]),
    ("NI::MASCOMMON::DATA::VersionInfo", &[("major", I(3)), ("minor", I(7)), ("patch", I(1)), ("revision", S(" (R0)"))]),
    // An empty browser handle.
    (QUICK_BROWSE, &[("length", I(8))]),
    // New patterns are one 4/4 bar long; sections are a bar, auto length.
    (PATTERN, &[("length", I(3840)), ("color/use_default_color", I(1))]),
    ("NI::MASCHINE::DATA::Section", &[("length", I(3840)), ("auto_length", I(1))]),
    // A zone over every key and velocity, root C3, unity gain.
    (ZONE, &[
        ("key_range/high", I(127)), ("velocity_range/high", I(127)), ("root_key", I(60)), ("gain", F(0.75)), ("zoom_factor", F(1.0)),
    ]),
    ("NI::MASCOMMON::DATA::ProbabilityEventPolicy<NI::MASCOMMON::DATA::NoteEvent>", &[("probability", I(100))]),
];

/// The browser handle's data when it refers to nothing.
const QUICK_BROWSE_DATA: [u8; 8] = [10, 0, 0, 0, 0, 0, 0, 0];

// The four arpeggiator presets' first fields (`i00`, `i08`).
use deckard_model::ARP_PRESETS;

/// The group's strum map: per strum mode, the pads (16..39) each of 8 steps plays.
#[rustfmt::skip]
const STRUM_MAP: [(i64, &[&[i64]]); 3] = [
    (0, &[&[32, 36, 39, 25, 27, 30], &[33, 37, 24, 26, 28, 31], &[34, 38, 25, 27, 29, 16], &[35, 39, 26, 28, 30, 17],
          &[36, 24, 27, 29, 31, 18], &[37, 25, 28, 30, 16, 19], &[38, 26, 29, 31, 17, 20], &[39, 27, 30, 16, 18, 21]]),
    (1, &[&[32, 34, 36], &[33, 35, 37], &[34, 36, 38], &[35, 37, 39], &[36, 38, 24], &[37, 39, 25], &[38, 24, 26], &[39, 25, 27]]),
    (2, &[&[], &[], &[], &[], &[], &[], &[], &[]]),
];

/// The velocity and probability lanes every sound, group and song starts with.
const LANES: [i64; 2] = [2, 8];

/// Number of MIDI remote parameters a project has.
const REMOTE_PARAMETERS: usize = 128;

/// Set a plain field to `d`, keeping the kind stored (a float as the `f32` it is
/// written as, unless `double`).
fn set_leaf(v: &mut Value, d: D, double: bool) {
    let float = |f: f64| Value::Float(if double { f } else { f as f32 as f64 });
    *v = match (&*v, d) {
        (Value::Int(_), I(i)) => Value::Int(i),
        (Value::Int(_), F(f)) => Value::Int(f.round() as i64),
        (Value::Float(_), F(f)) => float(f),
        (Value::Float(_), I(i)) => float(i as f64),
        (Value::Str(_), S(s)) => Value::Str(s.to_string()),
        _ => return,
    };
}

/// Set the value at `path` in `o`: a parameter's `value` and `t0`, or a plain field.
/// False if there is nothing at `path`.
pub(crate) fn set(o: &mut Obj, path: &str, d: D) -> bool {
    let (first, rest) = match path.split_once('/') {
        Some((a, b)) => (a, Some(b)),
        None => (path, None),
    };
    let Some(v) = o.find_mut(first) else { return false };
    match (rest, v) {
        (Some(rest), Value::Obj(sub)) => set(sub, rest, d),
        (Some(_), _) => false,
        (None, Value::Obj(p)) => {
            let double = p.type_name.contains("<double>");
            let mut done = false;
            for k in ["value", "t0"] {
                if let Some(slot) = p.find_mut(k)
                    && !matches!(slot, Value::Obj(_))
                {
                    set_leaf(slot, d, double);
                    done = true;
                }
            }
            done
        }
        (None, leaf) => {
            set_leaf(leaf, d, false);
            true
        }
    }
}

/// Set the text parameter `field` of `o`.
pub(crate) fn set_text(o: &mut Obj, field: &str, text: &str) -> bool {
    match o.find_mut(field) {
        Some(Value::Obj(p)) => match p.find_mut("value") {
            Some(v @ Value::Str(_)) => {
                *v = Value::Str(text.to_string());
                true
            }
            _ => false,
        },
        _ => false,
    }
}

/// The base-class part `class` of `o` (searching its bases' bases too).
fn base_mut<'a>(o: &'a mut Obj, class: &str) -> Option<&'a mut Obj> {
    let name = format!("<{class}>");
    if let Some(i) = o.fields.iter().position(|(n, _)| *n == name) {
        return match &mut o.fields[i].1 {
            Value::Obj(b) => Some(b),
            _ => None,
        };
    }
    o.fields.iter_mut().filter(|(n, _)| n.starts_with('<')).find_map(|(_, v)| match v {
        Value::Obj(b) => base_mut(b, class),
        _ => None,
    })
}

/// Apply [`DEFAULTS`] to `o` and every object in it.
fn apply_defaults(o: &mut Obj) {
    let mut v = Value::Obj(std::mem::take(o));
    v.walk_mut(&mut |x| {
        if let Some((_, list)) = DEFAULTS.iter().find(|(c, _)| *c == x.type_name) {
            for (path, d) in list.iter() {
                set(x, path, *d);
            }
        }
        if x.type_name == SOUND_INFO {
            set_blob(x, &mxprj::container::new_sound_info());
        } else if x.type_name == QUICK_BROWSE {
            set_blob(x, &QUICK_BROWSE_DATA);
        }
    });
    let Value::Obj(x) = v else { unreachable!() };
    *o = x;
}

/// A sized blob (a length field, then the bytes).
fn set_blob(o: &mut Obj, bytes: &[u8]) {
    if let Some((_, v)) = o.fields.iter_mut().find(|(_, v)| matches!(v, Value::Int(_))) {
        *v = Value::Int(bytes.len() as i64);
    }
    if let Some((_, v)) = o.fields.iter_mut().find(|(_, v)| matches!(v, Value::Bytes(_))) {
        *v = Value::Bytes(bytes.to_vec());
    }
}

/// `o`'s field `name`, which must be an object.
pub(crate) fn obj_mut<'a>(o: &'a mut Obj, name: &str) -> Result<&'a mut Obj, String> {
    let class = o.type_name.clone();
    match o.find_mut(name) {
        Some(Value::Obj(x)) => Ok(x),
        _ => Err(format!("{class} has no {name}")),
    }
}

/// Append `items` to the list `field` of `holder` (a `std::vector`, `std::map`, ...);
/// `item_version` is the items' class version, stored before the list.
pub(crate) fn push_items(holder: &mut Obj, items: Vec<Value>, item_version: u32) -> Result<(), String> {
    let at = holder.fields.iter().position(|(n, _)| n == "items").ok_or("no items list")?;
    let Value::List(list) = &mut holder.fields[at].1 else { return Err("items is not a list".into()) };
    list.extend(items);
    let vf = "items#item_version";
    if item_version != 0 && !holder.fields.iter().any(|(n, _)| n == vf) {
        holder.fields.insert(at, (vf.into(), Value::Int(item_version as i64)));
    }
    Ok(())
}

/// The class table for building objects into a project whose tree is `root`: its own
/// classes as they are, the others as Maschine 3 writes them.
pub(crate) fn class_table(root: Option<&Obj>) -> HashMap<String, (u32, bool)> {
    let mut t = classes::maschine3();
    if let Some(root) = root {
        root.walk_paths(&mut |o, _| {
            if !o.type_name.is_empty() {
                t.insert(o.type_name.clone(), (o.version, o.oid.is_some()));
            }
        });
    }
    t
}

/// Makes new objects with fresh object ids.
pub(crate) struct Builder<'t> {
    types: &'t Types,
    classes: HashMap<String, (u32, bool)>,
    pub(crate) next_oid: u64,
}

impl<'t> Builder<'t> {
    pub(crate) fn new(types: &'t Types, classes: HashMap<String, (u32, bool)>, next_oid: u64) -> Self {
        Builder { types, classes, next_oid }
    }

    fn oid(&mut self, class: &str) -> Option<u64> {
        if self.classes.get(class).is_some_and(|c| c.1) {
            self.next_oid += 1;
            Some(self.next_oid - 1)
        } else {
            None
        }
    }

    /// A new object of `class` at Maschine's defaults.
    pub(crate) fn blank(&mut self, class: &str) -> Result<Obj, String> {
        // A sound's routing parameters say their values aren't stored there (they are
        // in the group's sound configs).
        let ints = |path: &[String], field: &str| -> Option<i64> {
            (field == "skip_value" && path.len() >= 2 && path[path.len() - 2] == SOUND).then_some(1)
        };
        let spec = BlankSpec { classes: &self.classes, first_oid: self.next_oid, ints: &ints, project_level: true };
        let (mut o, next) = self.types.blank(class, &spec).map_err(|(e, at)| format!("can't build {class}: {e} in {at}"))?;
        self.next_oid = next;
        apply_defaults(&mut o);
        Ok(o)
    }

    /// `std::shared_ptr<class>` holding `o`.
    pub(crate) fn shared(&mut self, class: &str, o: Obj) -> Value {
        let t = format!("std::shared_ptr<{class}>");
        let version = self.classes.get(&t).map_or(1, |c| c.0);
        let oid = self.oid(&t);
        Value::Obj(Obj { type_name: t, version, oid, export: None, fields: vec![("px".into(), Value::Obj(o))] })
    }

    fn item_version(&self, class: &str) -> u32 {
        self.classes.get(class).map_or(0, |c| c.0)
    }

    /// Append `objs` (of `class`) to the `ObjectVector` `ov`; the first becomes its
    /// current one if `current`, and is selected if `select`.
    pub(crate) fn vector_push(&mut self, ov: &mut Obj, class: &str, objs: Vec<Obj>, current: bool, select: bool) -> Result<(), String> {
        let first = objs.first().and_then(|o| o.oid);
        let items: Vec<Value> = objs.into_iter().map(|o| self.shared(class, o)).collect();
        let v = self.item_version(&format!("std::shared_ptr<{class}>"));
        push_items(obj_mut(ov, "objects")?, items, v)?;
        if let Some(id) = first {
            if current {
                *ov.get_mut("current").ok_or("vector without current")? = Value::Ref(id);
            }
            if select {
                push_items(obj_mut(ov, "selection")?, vec![Value::Ref(id)], 0)?;
            }
        }
        Ok(())
    }

    /// Give a mixer channel (a sound, group or the song: `o` has a `MixingLayer` base)
    /// its first macro page and its velocity and probability lanes.
    fn channel(&mut self, o: &mut Obj) -> Result<(), String> {
        let page = self.blank(MACRO_PAGE)?;
        let mut lanes = Vec::new();
        for mode in LANES {
            let mut a = self.blank(MIDI_AUTOMATOR)?;
            set(&mut a, "mode", I(mode));
            lanes.push(a);
        }
        let class = o.type_name.clone();
        let ml = base_mut(o, MIXING_LAYER).ok_or_else(|| format!("{class} is not a mixer channel"))?;
        self.vector_push(obj_mut(ml, "macro_pages")?, MACRO_PAGE, vec![page], true, false)?;
        self.vector_push(obj_mut(ml, "midi_automators")?, MIDI_AUTOMATOR, lanes, true, false)
    }

    /// A new, empty sound for pad `index` (0-based) of a group.
    pub(crate) fn sound(&mut self, index: usize) -> Result<Obj, String> {
        let mut s = self.blank(SOUND)?;
        set(&mut s, "event_output_channel", I(index as i64));
        self.channel(&mut s)?;
        Ok(s)
    }

    /// `n` empty browser handles named `<prefix><i>`, as a group of fields (one is
    /// stored as itself).
    pub(crate) fn handles(&mut self, prefix: &str, n: usize) -> Result<Value, String> {
        let mut handles = Vec::new();
        for i in 0..n {
            handles.push((format!("{prefix}{i}"), Value::Obj(self.blank(QUICK_BROWSE)?)));
        }
        Ok(if handles.len() == 1 { handles.pop().unwrap().1 } else { Value::Obj(Obj { fields: handles, ..Obj::default() }) })
    }

    /// A sound's saved routing in its group, for pad `index`.
    pub(crate) fn sound_config(&mut self, index: usize) -> Result<Obj, String> {
        let mut c = self.blank(SOUND_CONFIG)?;
        set(&mut c, "midi_out_channel", I(index as i64));
        Ok(c)
    }

    /// A new group with `sounds` empty sounds.
    pub(crate) fn group(&mut self, sounds: usize) -> Result<Obj, String> {
        let mut g = self.blank(GROUP)?;
        let list = (0..sounds).map(|i| self.sound(i)).collect::<Result<Vec<_>, _>>()?;
        self.vector_push(obj_mut(&mut g, "sounds")?, SOUND, list, true, true)?;
        let configs = (0..sounds).map(|i| self.sound_config(i).map(Value::Obj)).collect::<Result<Vec<_>, _>>()?;
        let v = self.item_version(SOUND_CONFIG);
        push_items(obj_mut(&mut g, "sound_configs")?, configs, v)?;
        *g.get_mut("quick_browse").ok_or("group without quick_browse")? = self.handles("sound", sounds)?;
        // The strum map.
        let map = obj_mut(obj_mut(&mut g, "strumming_map")?, "px")?;
        let mut pairs = Vec::new();
        for (mode, steps) in STRUM_MAP {
            let mut p = self.blank(STRUM_PAIR)?;
            *p.get_mut("mode").ok_or("strum pair without mode")? = Value::Int(mode);
            let st = obj_mut(&mut p, "steps")?;
            let lists = steps
                .iter()
                .map(|pads| {
                    Value::Obj(Obj {
                        type_name: "std::vector<unsigned long>".into(),
                        fields: vec![("items".into(), Value::List(pads.iter().map(|&x| Value::Int(x)).collect()))],
                        ..Obj::default()
                    })
                })
                .collect();
            push_items(st, lists, 0)?;
            pairs.push(Value::Obj(p));
        }
        push_items(obj_mut(map, "map")?, pairs, 0)?;
        self.channel(&mut g)?;
        Ok(g)
    }

    /// A scene that plays nothing for each of `groups` (object ids), named `name`.
    pub(crate) fn scene(&mut self, name: &str, groups: &[u64]) -> Result<Obj, String> {
        let mut sc = self.blank(SCENE)?;
        set_text(&mut sc, "name", name);
        let map = obj_mut(&mut sc, "patterns")?;
        let pair = format!("std::pair<{GROUP}* const, std::shared_ptr<{PATTERN_REF}>>");
        let mut first_ref = None;
        let mut pairs = Vec::new();
        for &g in groups {
            let mut p = self.blank(&pair)?;
            *p.get_mut("first").ok_or("pair without first")? = Value::Ref(g);
            if let Ok(r) = obj_mut(obj_mut(&mut p, "second")?, "px") {
                first_ref = first_ref.or(r.oid);
            }
            pairs.push(Value::Obj(p));
        }
        let v = self.item_version(&pair);
        push_items(obj_mut(map, "objects")?, pairs, v)?;
        if let Some(id) = first_ref {
            *map.get_mut("current").ok_or("map without current")? = Value::Ref(id);
        }
        Ok(sc)
    }

    /// The item of an `ObjectMap<unsigned long, class>`: `key` and a new `class`
    /// object (made by `make`) behind a shared pointer.
    fn map_entry(&mut self, class: &str, key: i64, obj: Obj) -> Result<Value, String> {
        let mut pair = self.blank(&format!("std::pair<unsigned long const, std::shared_ptr<{class}>>"))?;
        *pair.get_mut("first").ok_or("pair without first")? = Value::Int(key);
        *obj_mut(&mut pair, "second")?.get_mut("px").ok_or("pair without px")? = Value::Obj(obj);
        Ok(Value::Obj(pair))
    }

    /// A new empty pattern `name`, `length` ticks long, with a slot for each of
    /// `sounds` (object ids), as `patterns_by_id` entry `id`.
    pub(crate) fn pattern_entry(&mut self, id: i64, name: &str, length: i64, sounds: &[u64]) -> Result<Value, String> {
        let mut p = self.blank(PATTERN)?;
        set_text(&mut p, "name", name);
        set(&mut p, "length", I(length));
        let pair_class = "std::pair<NI::MASCHINE::DATA::Sound const* const, std::shared_ptr<NI::MASCHINE::DATA::SequenceReference>>";
        let mut entries = Vec::new();
        for &snd in sounds {
            let mut e = self.blank(pair_class)?;
            *e.get_mut("sound").ok_or("entry without sound")? = Value::Ref(snd);
            entries.push(Value::Obj(e));
        }
        let v = self.item_version(pair_class);
        push_items(obj_mut(obj_mut(&mut p, "sound_sequences")?, "objects")?, entries, v)?;
        self.map_entry(PATTERN, id, p)
    }

    /// A `sections` entry `key` playing the scene with id `scene`.
    pub(crate) fn section_entry(&mut self, key: i64, scene: u64) -> Result<Value, String> {
        let mut sec = self.blank(SECTION)?;
        *sec.get_mut("scene").ok_or("section without scene")? = Value::Ref(scene);
        self.map_entry(SECTION, key, sec)
    }

    /// A scene's slot for the group with id `group` (playing nothing).
    pub(crate) fn scene_entry(&mut self, group: u64) -> Result<Value, String> {
        let mut p = self.blank(&format!("std::pair<{GROUP}* const, std::shared_ptr<{PATTERN_REF}>>"))?;
        *p.get_mut("first").ok_or("pair without first")? = Value::Ref(group);
        Ok(Value::Obj(p))
    }

    /// A list item (`shared_ptr`) holding a new `class` object.
    pub(crate) fn item(&mut self, class: &str) -> Result<Value, String> {
        let o = self.blank(class)?;
        Ok(self.shared(class, o))
    }

    /// A new note's `TimedEvent` (position 0, a `NoteEvent` of length 0, note 0,
    /// velocity 0, probability 100).
    pub(crate) fn note_event(&mut self) -> Result<Value, String> {
        let mut te = self.blank(TIMED_EVENT)?;
        *te.get_mut("event").ok_or("TimedEvent without event")? = Value::Obj(self.blank(NOTE_EVENT)?);
        Ok(Value::Obj(te))
    }

    /// A new MIDI event's `TimedEvent`.
    pub(crate) fn midi_event(&mut self) -> Result<Value, String> {
        let mut te = self.blank(TIMED_EVENT)?;
        *te.get_mut("event").ok_or("TimedEvent without event")? = Value::Obj(self.blank(MIDI_EVENT)?);
        Ok(Value::Obj(te))
    }

    /// A pattern's Audio plug-in event (`TimedEvent` at 0 holding an `AudioEvent` that
    /// lasts for ever and plays the take with object id `take`).
    pub(crate) fn audio_event(&mut self, take: u64) -> Result<Value, String> {
        let mut te = self.blank(TIMED_EVENT)?;
        let mut ev = self.blank(AUDIO_EVENT)?;
        if let Some(l) = ev.find_mut("length") {
            *l = Value::Int(i64::MAX);
        }
        *ev.get_mut("take").ok_or("AudioEvent without take")? = Value::Ref(take);
        *te.get_mut("event").ok_or("TimedEvent without event")? = Value::Obj(ev);
        Ok(Value::Obj(te))
    }

    /// A clip's `TimedEvent`: a `ClipEvent` at `position`, `length` long, playing the
    /// clip pattern with object id `pattern`.
    pub(crate) fn clip_event(&mut self, position: i64, length: i64, pattern: u64) -> Result<Value, String> {
        let mut te = self.blank(TIMED_EVENT)?;
        *te.get_mut("position").ok_or("TimedEvent without position")? = Value::Int(position);
        let mut ev = self.blank("NI::MASCHINE::DATA::ClipEvent")?;
        *ev.find_mut("length").ok_or("ClipEvent without length")? = Value::Int(length);
        *ev.get_mut("pattern").ok_or("ClipEvent without pattern")? = Value::Ref(pattern);
        *te.get_mut("event").ok_or("TimedEvent without event")? = Value::Obj(ev);
        Ok(Value::Obj(te))
    }

    /// A modulation point's `TimedEvent`: a `ModulationEvent` of the parameter (class
    /// `param_class`, object id `param`) at `position` with `value`. A v0 event holds
    /// the value; a v1 event a plain point's `Modulation` (`a = [0, 1, 0, 0]`,
    /// `b = [value, 0, 0, 0]`).
    pub(crate) fn mod_event(&mut self, param_class: &str, param: u64, position: i64, value: f64) -> Result<Value, String> {
        let mut te = self.blank(TIMED_EVENT)?;
        *te.get_mut("position").ok_or("TimedEvent without position")? = Value::Int(position);
        let mut ev = self.blank(&format!("NI::MASCOMMON::DATA::ModulationEvent<{param_class}>"))?;
        *ev.get_mut("parameter").ok_or("modulation event without parameter")? = Value::Ref(param);
        let f = |v: f64| Value::Float(if param_class.contains("<double>") { v } else { v as f32 as f64 });
        if let Some(c) = ev.get_mut("constant") {
            *c = match c {
                Value::Int(_) => Value::Int(value.round() as i64),
                _ => f(value),
            };
        } else if let Some(Value::Obj(m)) = ev.get_mut("modulation") {
            for (k, vals) in [("a", [0.0, 1.0, 0.0, 0.0]), ("b", [value, 0.0, 0.0, 0.0])] {
                if let Some(Value::Obj(arr)) = m.get_mut(k) {
                    arr.fields = std::iter::once(("count".to_string(), Value::Int(4)))
                        .chain(vals.iter().enumerate().map(|(i, v)| (format!("v{i}"), Value::Float(*v as f32 as f64))))
                        .collect();
                }
            }
        } else {
            return Err("modulation event without a value".into());
        }
        *te.get_mut("event").ok_or("TimedEvent without event")? = Value::Obj(ev);
        Ok(Value::Obj(te))
    }

    /// A sequence's event node marker.
    pub(crate) fn node(&mut self) -> Result<Value, String> {
        Ok(Value::Obj(self.blank(EVENT_NODE)?))
    }

    /// A new song: one group of 16 sounds and one scene.
    pub(crate) fn song(&mut self) -> Result<Obj, String> {
        let mut song = self.blank(SONG)?;
        let group = self.group(16)?;
        let gid = group.oid.ok_or("group without id")?;
        self.vector_push(obj_mut(&mut song, "groups")?, GROUP, vec![group], true, true)?;
        *song.get_mut("quick_browse").ok_or("song without quick_browse")? = self.handles("group_", 1)?;
        let scene = self.scene("Scene 1", &[gid])?;
        self.vector_push(obj_mut(&mut song, "scenes")?, SCENE, vec![scene], true, false)?;
        let mut presets = Vec::new();
        for (a, b) in ARP_PRESETS {
            let mut p = self.blank(ARP_PRESET)?;
            set(&mut p, "i00", I(a));
            set(&mut p, "i08", I(b));
            presets.push(Value::Obj(p));
        }
        let v = self.item_version(ARP_PRESET);
        push_items(obj_mut(&mut song, "arp_presets")?, presets, v)?;
        self.channel(&mut song)?;
        Ok(song)
    }

    /// A new project as Maschine 3 starts one.
    pub(crate) fn project(&mut self) -> Result<Obj, String> {
        let mut p = self.blank(PROJECT)?;
        let song = self.song()?;
        self.vector_push(obj_mut(&mut p, "songs")?, SONG, vec![song], true, false)?;
        let remote = (0..REMOTE_PARAMETERS).map(|_| self.blank(REMOTE_PARAMETER)).collect::<Result<Vec<_>, _>>()?;
        self.vector_push(obj_mut(&mut p, "remote_parameters")?, REMOTE_PARAMETER, remote, true, false)?;
        Ok(p)
    }
}

/// New item ids for a file's container: random (from the process's hash seeds and the
/// time).
pub(crate) fn uuids() -> impl FnMut() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let state = std::collections::hash_map::RandomState::new();
    let mut n = 0u64;
    move || {
        let mut out = [0u8; 16];
        for half in out.chunks_mut(8) {
            let mut h = state.build_hasher();
            h.write_u64(n);
            h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
            n += 1;
            half.copy_from_slice(&h.finish().to_le_bytes());
        }
        out
    }
}

impl crate::Document {
    /// A new, empty project, as Maschine 3 starts one: one song with one group of 16
    /// empty sounds and one scene, at Maschine's defaults (120 BPM, 4/4). Built
    /// entirely from code; it has no path until saved.
    pub fn new_project() -> Result<crate::Document, String> {
        let types = Types::new();
        let mut b = Builder::new(&types, class_table(None), 1);
        let root = b.project()?;
        let header = mxprj::archive::Header { library_version: classes::LIBRARY_VERSION, flags: 0 };
        let (archive, _) =
            mxprj::decode::encode(root, PROJECT, header, &types.loaders, &types.exports).map_err(|(e, at)| format!("{e} in {at}"))?;
        let file = mxprj::container::new_project_file(&archive, classes::APP_VERSION, &mut uuids());
        crate::Document::from_bytes(file, None)
    }

    /// Run `f` with a [`Builder`] for new objects in this project (its class versions,
    /// ids above every id in use).
    pub(crate) fn build<T>(&mut self, f: impl FnOnce(&mut Builder) -> Result<T, String>) -> Result<T, String> {
        let classes = class_table(Some(self.root()));
        let mut b = Builder::new(&self.types, classes, self.next_oid);
        let r = f(&mut b);
        let next = b.next_oid;
        self.next_oid = next;
        r
    }
}

#[cfg(test)]
mod tests {
    use crate::Document;

    #[test]
    fn new_project_opens_saves_and_reopens() {
        let d = Document::new_project().unwrap();
        let p = d.project();
        assert_eq!(p.groups.len(), 1);
        assert_eq!(p.groups[0].sounds.len(), 16);
        assert_eq!(p.scenes.len(), 1);
        assert_eq!((p.tempo, p.numerator, p.denominator), (120.0, 4, 4));
        assert!(p.groups[0].sounds.iter().all(|s| s.slots.is_empty()));
        let bytes = d.to_bytes().unwrap();
        let again = Document::from_bytes(bytes.clone(), None).unwrap();
        assert_eq!(again.to_bytes().unwrap(), bytes);
        assert!(again.project() == d.project());
        if let Ok(dir) = std::env::var("DECKARD_WRITE_NEW_PROJECT") {
            std::fs::write(std::path::Path::new(&dir).join("new.mxprj"), &bytes).unwrap();
        }
    }

    /// One scene always stays: deleting the only one is refused and changes nothing.
    #[test]
    fn the_last_scene_cant_be_deleted() {
        let mut d = Document::new_project().unwrap();
        let steps = d.undo_labels().len();
        assert!(d.delete_scene(0).is_err());
        assert_eq!(d.project().scenes.len(), 1);
        assert_eq!(d.undo_labels().len(), steps);
        d.add_scene().unwrap();
        d.delete_scene(0).unwrap();
        assert_eq!(d.project().scenes.len(), 1, "with two, one can go");
    }

    /// Every kind of module, built from scratch.
    fn every_module() -> Vec<crate::NewModule> {
        use crate::NewModule;
        let mut all = vec![
            NewModule::Sampler,
            NewModule::Audio,
            NewModule::Plugin {
                id: "Native Instruments:VST3:INSTR:5653544E694D6D6173736976650000".into(),
                name: "Massive".into(),
                chunk: Vec::new(),
            },
        ];
        for c in ["Kick", "Snare", "Hihat", "Tom", "Perc", "Cymbal", "BassSynth", "PolySynth"] {
            all.push(NewModule::Instrument { class: c.into(), params: Vec::new() });
        }
        all.extend(deckard_model::INTERNAL_EFFECTS.iter().map(|(c, _)| NewModule::Effect(c)));
        all
    }

    /// Fill sounds with every kind of module; it saves and reads back with them.
    fn add_every_module(d: &mut Document) {
        let all = every_module();
        let n = d.project().groups[0].sounds.len();
        for (i, m) in all.iter().enumerate() {
            let s = i % n;
            let at = if m.is_instrument() { 0 } else { d.project().groups[0].sounds[s].slots.len() };
            d.add_slot(0, s, at, m).unwrap_or_else(|e| panic!("{m:?}: {e}"));
        }
        let back = Document::from_bytes(d.to_bytes().unwrap(), None).unwrap();
        assert!(back.project() == d.project());
        let classes: Vec<String> = back.project().groups[0].sounds.iter().flat_map(|s| s.slots.iter().map(|x| x.module.clone())).collect();
        for m in &all {
            let c = m.class();
            let short = c.rsplit("::").next().unwrap();
            assert!(classes.iter().any(|x| x == short), "{short} missing after reading back: {classes:?}");
        }
        let smp = back.project().groups[0].sounds.iter().find_map(|s| s.sampler.clone()).unwrap();
        assert!(smp.zones.is_empty());
        assert_eq!(smp.get("cutoff"), 0.5);
    }

    #[test]
    fn modules_from_scratch_in_a_new_project() {
        let mut d = Document::new_project().unwrap();
        add_every_module(&mut d);
        if let Ok(dir) = std::env::var("DECKARD_WRITE_NEW_PROJECT") {
            std::fs::write(std::path::Path::new(&dir).join("new-modules.mxprj"), d.to_bytes().unwrap()).unwrap();
        }
        // Removing them all leaves an empty project again.
        for s in 0..16 {
            d.reset_sound(0, s).unwrap();
        }
        assert!(d.project().groups[0].sounds.iter().all(|s| s.slots.is_empty()));
    }

    #[test]
    fn modules_from_scratch_in_an_older_project() {
        let path = "/Users/Shared/Velvet Lounge Library/Projects/Cloud 9 Vibes.mxprj";
        let Ok(mut d) = Document::open(std::path::Path::new(path)) else { return };
        add_every_module(&mut d);
    }

    /// A 16-bit mono WAV of a decaying 220 Hz tone (`frames` long).
    fn tone(path: &std::path::Path, frames: usize) {
        let mut data = Vec::new();
        for i in 0..frames {
            let t = i as f64 / 44100.0;
            let v = (t * 220.0 * std::f64::consts::TAU).sin() * (-t * 4.0).exp() * 0.8;
            data.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes());
        }
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        for x in [16u32, 0x0001_0001, 44100, 88200, 0x0010_0002] {
            w.extend_from_slice(&x.to_le_bytes());
        }
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(&data);
        std::fs::write(path, w).unwrap();
    }

    /// Everything a project is made of, started from nothing: patterns, notes, MIDI
    /// events, lanes, macro pages, scenes, sections, a Sampler with a zone, modules.
    #[test]
    fn new_project_takes_every_edit() {
        use deckard_model::{LaneRef, MidiEvent, MidiKind, Note};
        let dir = std::env::var("DECKARD_WRITE_NEW_PROJECT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join(format!("deckard-new-{}", std::process::id())));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("tone.wav");
        tone(&wav, 44100);

        let mut d = Document::new_project().unwrap();
        d.set_tempo(133.0).unwrap();
        d.rename(crate::Target::Sound(0, 0), "Tone").unwrap();
        // A pattern, in the scene, with notes and a controller.
        assert_eq!(d.add_pattern(0, 0, "Pattern 1").unwrap(), 0);
        d.assign_pattern(0, 0, Some(0)).unwrap();
        let note = |sound, position, pitch| Note { expr: None, sound, position, length: 480, pitch, velocity: 100, probability: 100 };
        d.edit_notes("Add", 0, 0, &[], &[note(0, 0, 60), note(0, 960, 64), note(1, 1920, 60)]).unwrap();
        let cc = MidiEvent { sound: 0, position: 480, kind: MidiKind::Controller { number: 1, value: 64 } };
        d.edit_midi("Add", 0, 0, &[], &[cc]).unwrap();
        assert_eq!(d.add_pattern(0, 0, "Pattern 2").unwrap(), 1);
        d.add_lane(0, 0, LaneRef { kind: 4, number: 1 }).unwrap();
        d.add_macro_page(crate::Target::Sound(0, 0)).unwrap();
        // The arrangement.
        assert_eq!(d.add_scene().unwrap(), 1);
        d.assign_pattern(1, 0, Some(1)).unwrap();
        assert_eq!(d.add_section(0).unwrap(), 0);
        assert_eq!(d.add_section(1).unwrap(), 1);
        // A Sampler playing the tone, and an effect after it.
        d.add_slot(0, 0, 0, &crate::NewModule::Sampler).unwrap();
        let spec = crate::ZoneSpec {
            from: crate::ZoneFrom::Template,
            file: Some(wav.clone()),
            set: vec![("play_range/high", 44099.0), ("loop_range/high", 44099.0)],
        };
        d.set_zones(0, 0, &[spec], "Load Sample").unwrap();
        d.add_slot(0, 0, 1, &crate::NewModule::Effect("Reverb")).unwrap();

        let p = d.project().clone();
        assert_eq!(p.groups[0].patterns.len(), 2);
        assert_eq!(p.groups[0].patterns[0].notes.len(), 3);
        assert_eq!(p.groups[0].patterns[0].midi.len(), 1);
        assert_eq!(p.scenes.len(), 2);
        assert_eq!(p.sections.len(), 2);
        let smp = p.groups[0].sounds[0].sampler.clone().unwrap();
        assert_eq!(smp.zones.len(), 1);
        assert_eq!((smp.zones[0].key_range, smp.zones[0].root_key, smp.zones[0].play_range), ((0, 127), 60, (0, 44099)));

        let out = dir.join("new-full.mxprj");
        d.save_as(&out).unwrap();
        let again = Document::open(&out).unwrap();
        assert!(again.project() == &p, "reads back as saved");
        assert_eq!(again.to_bytes().unwrap(), std::fs::read(&out).unwrap(), "saves identically");
        if std::env::var("DECKARD_WRITE_NEW_PROJECT").is_err() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        // All of it undoes back to the empty project.
        while d.undo() {}
        assert!(d.project().groups[0].patterns.is_empty());
        assert_eq!(d.project().tempo, 120.0);
    }
}
