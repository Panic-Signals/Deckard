//! Song and its arrangement: Song, ArrangerState, Track, Recorder, MidiRecall, Scene,
//! Section, PatternRef, routing/loop/color parameters.
//!
//! Field names are struct offsets (`f1e8` = member at `this + 0x1e8`) unless the
//! decompile shows the meaning. `legacy_*` fields are read into temporaries by
//! the app (old versions converted on load).

use super::Types;
use crate::container::Result;
use crate::decode::{Decoder, Value};
use std::cell::Cell;

const MD: &str = "NI::MASCHINE::DATA::";
const B: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>";
const F: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<float>";
const D: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<double>";
const I: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<int>";
const LL: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<long long>";
const UL: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>";
const ENUM: &str = "NI::MASCOMMON::DATA::EnumParameter";
const AOP: &str = "NI::MASCHINE::DATA::AudioOutputParameter";
const LOOP: &str = "NI::MASCHINE::DATA::LoopParameter";
const COLOR: &str = "NI::MASCHINE::DATA::ColorParameter";
const MIXING_LAYER: &str = "NI::MASCHINE::DATA::MixingLayer";
const GROUP: &str = "NI::MASCHINE::DATA::Group";
const SOUND: &str = "NI::MASCHINE::DATA::Sound";
const SLOT: &str = "NI::MASCOMMON::DATA::Slot";

thread_local! {
    /// `MaschineRoutingParameter::m_skipValue` (constructor default false). Containers
    /// set it on specific members before loading them: Track on its three audio
    /// outputs, Group on its EventInputParameter. Only consulted for routing
    /// parameter class version 1.
    static ROUTING_SKIP: Cell<bool> = const { Cell::new(false) };
}

/// Load a routing parameter (`AudioOutputParameter`, `EventInputParameter`, ...) with
/// its "skip value" member preset to `skip`, as `Track`/`Group::serialize` do.
pub(super) fn with_routing_skip(d: &mut Decoder, skip: bool, name: &str, ty: &str) -> Result<()> {
    ROUTING_SKIP.with(|c| c.set(skip));
    let r = d.object(name, ty);
    ROUTING_SKIP.with(|c| c.set(false));
    r
}

/// `MaschineArchiveExtension::isProjectLevel`: true when loading a whole project.
pub(super) fn is_project_level(d: &Decoder) -> bool {
    d.path().starts_with("NI::MASCHINE::DATA::Project")
}

/// Number of items of a decoded `ObjectVector<T>` value.
fn object_vector_len(d: &Decoder, v: &Value) -> Result<usize> {
    let Value::Obj(ov) = v else { return Err(d.err("object vector was a back-reference")) };
    match ov.get("objects") {
        Some(Value::Obj(vec)) => match vec.get("items") {
            Some(Value::List(items)) => Ok(items.len()),
            _ => Err(d.err("object vector without items")),
        },
        _ => Err(d.err("object vector without objects")),
    }
}

pub(super) fn register(t: &mut Types) {
    values(t);
    routing(t);
    arrangement(t);
    track(t);
    song(t);
}

// ---- parameter values ------------------------------------------------------

/// `Audio{Input,Output}ParameterValue`: routing target (type, index) and the
/// group/sound/slot it points at.
fn audio_value(d: &mut Decoder, v: u32) -> Result<()> {
    d.int("type")?;
    d.int("index")?;
    match v {
        0 | 1 => {
            d.pointer("group", GROUP, false)?;
            d.pointer("sound", SOUND, false)?;
            d.pointer("slot", SLOT, false)?;
            d.int("legacy_i64")?;
        }
        2 => {
            d.object("group", "std::weak_ptr<NI::MASCHINE::DATA::Group>")?;
            d.object("sound", "std::weak_ptr<NI::MASCHINE::DATA::Sound>")?;
            d.object("slot", "std::weak_ptr<NI::MASCOMMON::DATA::Slot>")?;
        }
        _ => {
            d.pointer("group", GROUP, false)?;
            d.pointer("sound", SOUND, false)?;
            d.pointer("slot", SLOT, false)?;
        }
    }
    Ok(())
}

fn values(t: &mut Types) {
    t.weak_ptr(GROUP, false);
    t.weak_ptr(SOUND, false);
    t.weak_ptr(SLOT, false);

    t.add("NI::MASCHINE::DATA::AudioInputParameterValue", audio_value);
    t.add("NI::MASCHINE::DATA::AudioOutputParameterValue", audio_value);
    t.add("NI::MASCHINE::DATA::EventOutputParameterValue", |d, v| {
        d.int("type")?;
        d.string("port")?;
        match v {
            0 | 1 => {
                d.pointer("group", GROUP, false)?;
                d.pointer("sound", SOUND, false)?;
                d.pointer("slot", SLOT, false)?;
            }
            2 => {
                d.object("sound", "std::weak_ptr<NI::MASCHINE::DATA::Sound>")?;
                d.object("slot", "std::weak_ptr<NI::MASCOMMON::DATA::Slot>")?;
            }
            _ => {
                d.pointer("sound", SOUND, false)?;
                d.pointer("slot", SLOT, false)?;
            }
        }
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::SideChainInputParameterValue", |d, v| {
        d.int("type")?;
        if v == 1 {
            d.object("group", "std::weak_ptr<NI::MASCHINE::DATA::Group>")?;
            d.object("sound", "std::weak_ptr<NI::MASCHINE::DATA::Sound>")?;
        } else {
            d.pointer("group", GROUP, false)?;
            d.pointer("sound", SOUND, false)?;
        }
        Ok(())
    });
    // std::map<unsigned long, SideChainInputParameterValue>: items are pairs.
    t.add_once("std::pair<unsigned long const, NI::MASCHINE::DATA::SideChainInputParameterValue>", |d, _| {
        d.int("first")?;
        d.object("second", "NI::MASCHINE::DATA::SideChainInputParameterValue").map(drop)
    });
    t.add_once("std::map<unsigned long, NI::MASCHINE::DATA::SideChainInputParameterValue>", |d, _| {
        d.collection("items", |d| {
            d.object("item", "std::pair<unsigned long const, NI::MASCHINE::DATA::SideChainInputParameterValue>").map(drop)
        })
    });

    t.add("NI::MASCHINE::DATA::LoopParameterValue", |d, _| {
        d.f64("start")?;
        d.f64("length")?;
        Ok(())
    });
    t.add(COLOR, |d, _| {
        d.int("color")?;
        d.int("use_default_color")?;
        Ok(())
    });
}

// ---- routing parameters ------------------------------------------------------

/// `ValueParameter<V>` for struct values: base Parameter, then the value object.
fn value_parameter(t: &mut Types, v: &'static str) {
    t.add(format!("NI::ML::TM::ValueParameter<{MD}{v}>"), move |d, _| {
        d.base("NI::ML::TM::Parameter")?;
        d.object("value", &format!("{MD}{v}")).map(drop)
    });
}

/// `MaschineRoutingParameter<V>::load`: from version 2 a flag says whether the value
/// was stored; before that the member `m_skipValue` decides (see [`with_routing_skip`]).
fn routing_load(d: &mut Decoder, v: u32, value_param: &str) -> Result<()> {
    let skip = if v < 2 { ROUTING_SKIP.with(|c| c.get()) } else { d.int("skip_value")? != 0 };
    if v == 0 || !skip {
        d.base(value_param)?;
    }
    d.base("NI::MASCOMMON::DATA::MaschineParameter")
}

fn routing(t: &mut Types) {
    for v in [
        "AudioInputParameterValue",
        "AudioOutputParameterValue",
        "EventOutputParameterValue",
        "SideChainInputParameterValue",
        "LoopParameterValue",
    ] {
        value_parameter(t, v);
    }
    for (param, value) in [
        ("AudioInputParameter", "NI::ML::TM::ValueParameter<NI::MASCHINE::DATA::AudioInputParameterValue>"),
        ("AudioOutputParameter", "NI::ML::TM::ValueParameter<NI::MASCHINE::DATA::AudioOutputParameterValue>"),
        ("EventInputParameter", "NI::ML::TM::ValueParameter<std::wstring>"),
        ("EventOutputParameter", "NI::ML::TM::ValueParameter<NI::MASCHINE::DATA::EventOutputParameterValue>"),
        ("SideChainInputParameter", "NI::ML::TM::ValueParameter<NI::MASCHINE::DATA::SideChainInputParameterValue>"),
    ] {
        t.add(format!("{MD}{param}"), move |d, v| routing_load(d, v, value));
    }
    t.add(LOOP, |d, _| {
        d.base("NI::ML::TM::ValueParameter<NI::MASCHINE::DATA::LoopParameterValue>")?;
        d.base("NI::MASCOMMON::DATA::MaschineParameter")
    });
}

// ---- scenes, sections, routing configs, recorder ----------------------------

fn arrangement(t: &mut Types) {
    t.add_once("NI::MASCHINE::DATA::PatternRef", |d, _| d.pointer("pattern", "NI::MASCHINE::DATA::EventPattern", false).map(drop));
    // ObjectMap<Group*, PatternRef>: map<Group*, shared_ptr<PatternRef>>, then a PatternRef*.
    t.shared_ptr("NI::MASCHINE::DATA::PatternRef", false);
    let pair = "std::pair<NI::MASCHINE::DATA::Group* const, std::shared_ptr<NI::MASCHINE::DATA::PatternRef>>";
    let map = "std::map<NI::MASCHINE::DATA::Group*, std::shared_ptr<NI::MASCHINE::DATA::PatternRef>>";
    t.add_once(pair, |d, _| {
        d.pointer("first", GROUP, false)?;
        d.object("second", "std::shared_ptr<NI::MASCHINE::DATA::PatternRef>").map(drop)
    });
    t.add_once(map, move |d, _| d.collection("items", |d| d.object("item", pair).map(drop)));
    t.add_once("NI::ML::TM::ObjectMap<NI::MASCHINE::DATA::Group*, NI::MASCHINE::DATA::PatternRef>", move |d, _| {
        d.object("objects", map)?;
        d.pointer("current", "NI::MASCHINE::DATA::PatternRef", false).map(drop)
    });

    t.add_once("NI::MASCHINE::DATA::Scene", |d, v| {
        d.object("name", "NI::MASCOMMON::DATA::WStringParameter")?;
        d.object("color", COLOR)?;
        d.object("patterns", "NI::ML::TM::ObjectMap<NI::MASCHINE::DATA::Group*, NI::MASCHINE::DATA::PatternRef>")?;
        if v >= 2 {
            d.object("scene_tempo_active", B)?;
            d.object("tempo", F)?;
        }
        Ok(())
    });
    t.add_once("NI::MASCHINE::DATA::Section", |d, _| {
        d.pointer("scene", "NI::MASCHINE::DATA::Scene", false)?;
        d.object("length", LL)?;
        d.object("auto_length", B)?;
        Ok(())
    });
    t.object_vector("NI::MASCHINE::DATA::Scene", false);
    t.object_vector("NI::MASCHINE::DATA::Section", false);
    t.object_map("NI::MASCHINE::DATA::Scene", false);
    t.object_map("NI::MASCHINE::DATA::Section", false);

    t.add_once("NI::MASCHINE::DATA::TrackRoutingConfig", |d, _| {
        d.object("output", "NI::MASCHINE::DATA::AudioOutputParameterValue")?;
        d.int("b58")?;
        d.int("b59")?;
        d.object("aux1", "NI::MASCHINE::DATA::AudioOutputParameterValue")?;
        d.f32("aux1_level")?;
        d.int("aux1_mode")?;
        d.object("aux2", "NI::MASCHINE::DATA::AudioOutputParameterValue")?;
        d.f32("aux2_level")?;
        d.int("aux2_mode")?;
        d.object("sidechain", "std::map<unsigned long, NI::MASCHINE::DATA::SideChainInputParameterValue>").map(drop)
    });
    t.add_once("NI::MASCHINE::DATA::GroupRoutingConfig", |d, v| {
        d.string("name")?;
        d.int("i160")?;
        d.int("i168")?;
        if v >= 5 {
            d.int("b170")?;
        }
        if (3..=6).contains(&v) {
            d.int("legacy_i64")?;
        }
        if v > 5 {
            return d.object("track", "NI::MASCHINE::DATA::TrackRoutingConfig").map(drop);
        }
        d.object("output", "NI::MASCHINE::DATA::AudioOutputParameterValue")?;
        d.int("b58")?;
        if v == 0 {
            d.f32("legacy_f0")?;
            d.f32("legacy_f1")?;
        }
        d.int("b59")?;
        d.object("aux1", "NI::MASCHINE::DATA::AudioOutputParameterValue")?;
        d.f32("aux1_level")?;
        d.int("aux1_mode")?;
        d.object("aux2", "NI::MASCHINE::DATA::AudioOutputParameterValue")?;
        d.f32("aux2_level")?;
        d.int("aux2_mode")?;
        if v > 3 {
            d.object("sidechain", "std::map<unsigned long, NI::MASCHINE::DATA::SideChainInputParameterValue>")?;
        }
        Ok(())
    });
    t.add_once("std::vector<NI::MASCHINE::DATA::GroupRoutingConfig>", |d, _| {
        d.collection("items", |d| d.object("item", "NI::MASCHINE::DATA::GroupRoutingConfig").map(drop))
    });

    // SoundInfoContainer::serializeSoundInfo: scoped_ptr<NI::SOUND::SoundInfo2>, whose
    // load is a u32 length and an opaque NI sound-info blob.
    t.scoped_ptr("NI::SOUND::SoundInfo2", false);
    t.add_once("NI::SOUND::SoundInfo2", |d, _| {
        let len = d.int("blob_len")?;
        let len = usize::try_from(len).map_err(|_| d.err("negative SoundInfo2 length"))?;
        d.bytes("blob", len).map(drop)
    });

    t.add_once("NI::MASCHINE::DATA::Recorder", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        if v == 0 {
            d.object("legacy_bool", B)?;
        }
        d.object("recording_source_category", ENUM)?;
        d.object("ext_stereo_inputs", ENUM)?;
        d.object("ext_mono_inputs", ENUM)?;
        if v < 2 {
            d.object("legacy_enum", ENUM)?;
            if v == 0 {
                return Ok(());
            }
        } else {
            d.object("input", "NI::MASCHINE::DATA::AudioInputParameter")?;
        }
        d.object("recording_mode", ENUM)?;
        d.object("detect_threshold", F)?;
        d.object("sync_length", ENUM)?;
        d.object("direct_monitoring", B)?;
        if v > 2 {
            d.object("loop_target", ENUM)?;
        }
        Ok(())
    });

    t.add("NI::MASCHINE::DATA::MidiRecall", |d, v| {
        if v == 1 {
            d.object("legacy_bool", B)?;
            d.object("input", "NI::MASCHINE::DATA::EventInputParameter")?;
            d.object("legacy_enum", ENUM)?;
        } else {
            d.object("input", "NI::MASCHINE::DATA::EventInputParameter")?;
            d.object("trigger", ENUM)?;
        }
        d.object("midi_channel", UL).map(drop)
    });

    t.add("NI::MASCHINE::DATA::ArrangerState", |d, v| {
        d.int("idea_space_focused")?;
        match v {
            1 => {
                d.int("legacy_bool")?;
            }
            0 => {
                d.object("legacy_bool", B)?;
            }
            _ => {}
        }
        Ok(())
    });
}

// ---- Track -------------------------------------------------------------------

fn track(t: &mut Types) {
    t.add("NI::MASCHINE::DATA::Track", |d, v| {
        // Track::serialize marks its audio outputs "skip value" unless loading a project.
        let skip = v == 0 || !is_project_level(d);
        with_routing_skip(d, skip, "audio_output", AOP)?;
        d.object("cue_enabled", B)?;
        d.object("level", F)?;
        d.object("pan", F)?;
        d.object("mute", B)?;
        with_routing_skip(d, skip, "aux1_output", AOP)?;
        d.object("aux1_level", F)?;
        d.object("aux1_order", ENUM)?;
        with_routing_skip(d, skip, "aux2_output", AOP)?;
        d.object("aux2_level", F)?;
        d.object("aux2_order", ENUM)?;
        d.object("color", COLOR)?;
        d.base(MIXING_LAYER)
    });
    t.object_vector("NI::MASCHINE::DATA::Track", false);
    t.object_vector(GROUP, false);
}

// ---- Song --------------------------------------------------------------------

fn song(t: &mut Types) {
    t.add("NI::MASCHINE::DATA::Song", song_load);
}

fn song_load(d: &mut Decoder, v: u32) -> Result<()> {
    d.object("tempo", F)?;
    d.object("numerator", UL)?;
    d.object("denominator", UL)?;
    if v < 3 {
        d.object("legacy_bool_a", B)?;
    }
    d.object("master_output", AOP)?;
    d.object("level", F)?;
    // The master's pan (docs/mixer-layout.md §8).
    d.object("pan", F)?;
    d.object("cue_enabled", B)?;
    d.object("cue_mute", B)?;
    d.object("cue_output", AOP)?;
    d.object("cue_level", F)?;
    d.object("cue_pan", F)?;
    if v < 13 {
        d.object("loop", LOOP)?;
        if v == 0 {
            d.object("legacy_bool_b", B)?;
            d.object("f180", ENUM)?;
            d.object("num_section_banks", UL)?;
            d.object("focus_section_bank_index", UL)?;
        } else {
            song_after_loop(d, v)?;
        }
    } else {
        d.object("f168", B)?;
        d.object("f170", B)?;
        d.object("loop", LOOP)?;
        song_after_loop(d, v)?;
    }
    d.object("f1c8", UL)?;
    if v < 7 {
        // Old arpeggiator settings, one rate and one rate unit per (4) arp slots.
        for i in 0..4 {
            d.object(&format!("legacy_arp_rate_{i}"), ENUM)?;
        }
        for i in 0..4 {
            d.object(&format!("legacy_arp_rate_unit_{i}"), ENUM)?;
        }
        d.object("legacy_arp_gate", F)?;
    }
    d.object("pattern_editor_offset", LL)?;
    d.object("pattern_editor_zoom", D)?;
    d.object("f1f8", LL)?;
    d.object("f200", D)?;
    if v > 15 {
        d.object("pattern_editor_focus_offset_hw", LL)?;
    }
    d.object("pattern_editor_snap", ENUM)?;
    d.object("pattern_editor_snap_enabled", B)?;
    d.object("pattern_editor_nudge_snap", ENUM)?;
    if v > 29 {
        d.object("clip_editor_zoom", D)?;
        d.object("clip_editor_offset", LL)?;
        d.object("clip_editor_zoom_hw", D)?;
        d.object("clip_editor_offset_hw", LL)?;
    }
    d.object("arranger_zoom", D)?;
    d.object("arranger_offset", LL)?;
    d.object("arranger_zoom_hw", D)?;
    d.object("arranger_offset_hw", LL)?;
    d.object("f268", I)?;
    if (12..=28).contains(&v) {
        d.object("legacy_bool_c", B)?;
        if (18..=20).contains(&v) {
            d.object("legacy_bool_d", B)?;
        }
    } else if v >= 29 {
        d.object("focus_entity", ENUM)?;
    }
    d.object("perform_grid", ENUM)?;
    d.object("perform_retrig", B)?;
    if v < 15 {
        d.object("legacy_enum_a", ENUM)?;
    } else {
        d.object("arrange_grid", ENUM)?;
        d.object("arrange_grid_snap", B)?;
        d.object("pattern_length_quick", B)?;
    }
    if (4..=19).contains(&v) {
        d.object("legacy_midi_input", "NI::MASCHINE::DATA::EventInputParameter")?;
        if v == 4 {
            d.object("legacy_bool_e", B)?;
        }
        d.object("legacy_enum_b", ENUM)?;
        d.object("legacy_ul_a", UL)?;
    } else if v < 4 {
        d.object("legacy_enum_b", ENUM)?;
        d.object("legacy_ul_a", UL)?;
    } else {
        if v >= 24 {
            d.object("scene_recall", "NI::MASCHINE::DATA::MidiRecall")?;
        }
        d.object("section_recall", "NI::MASCHINE::DATA::MidiRecall")?;
        d.object("snapshot_recall", "NI::MASCHINE::DATA::MidiRecall")?;
    }
    d.object("fixed_velocity", UL)?;
    if (2..=18).contains(&v) {
        d.object("legacy_ul_b", UL)?;
    }
    if v > 17 {
        d.object("default_velocity_jam", UL)?;
        d.object("accent_velocity_jam", UL)?;
    }
    d.object("pad_velocity_mode", ENUM)?;
    for n in ["sounds_visible_mixer", "functional_blocks_visible_mixer", "f308", "f310", "f318"] {
        d.object(n, B)?;
    }
    if v < 14 {
        d.object("groups", "NI::ML::TM::ObjectVector<NI::MASCHINE::DATA::Group>")?;
    } else if v < 28 {
        // Groups were stored as tracks; the app converts each into a group.
        d.object("groups", "NI::ML::TM::ObjectVector<NI::MASCHINE::DATA::Track>")?;
        d.object("f330_ul", UL)?;
    } else {
        d.object("groups", "NI::ML::TM::ObjectVector<NI::MASCHINE::DATA::Group>")?;
        d.object("f330_ul", UL)?;
    }
    d.base(MIXING_LAYER)?;

    // Song::serializeScenes
    if v < 25 {
        d.object("scenes", "NI::ML::TM::ObjectMap<unsigned long, NI::MASCHINE::DATA::Scene>")?;
    } else {
        d.object("scenes", "NI::ML::TM::ObjectVector<NI::MASCHINE::DATA::Scene>")?;
    }
    // Song::serializeSections
    if v >= 17 {
        d.object("sections", "NI::ML::TM::ObjectMap<unsigned long, NI::MASCHINE::DATA::Section>")?;
    } else if v >= 11 {
        d.object("sections", "NI::ML::TM::ObjectVector<NI::MASCHINE::DATA::Section>")?;
    }
    // Song::serializeGroupsRoutingConfig
    if v < 21 {
        d.object("groups_routing", "std::vector<NI::MASCHINE::DATA::GroupRoutingConfig>")?;
    }
    // Song::serializeQuickBrowse: one QuickBrowseHandle per group.
    let n_groups = object_vector_len(d, d.field("groups").ok_or_else(|| d.err("song without groups"))?)?;
    d.group("quick_browse", |d| {
        for i in 0..n_groups {
            d.object(&format!("group_{i}"), "NI::DB3::QuickBrowseHandle")?;
        }
        Ok(())
    })?;
    // SoundInfoContainer::serializeSoundInfo
    d.object("sound_info", "boost::scoped_ptr<NI::SOUND::SoundInfo2>")?;
    d.object("recorder", "NI::MASCHINE::DATA::Recorder")?;

    if v < 6 {
        return Ok(());
    }
    d.object("scale_engine", "NI::MASCOMMON::DATA::ScaleEngine")?;
    d.int("fac8")?;
    d.object("arpeggiator", "NI::MASCOMMON::DATA::Arpeggiator")?;
    d.object("enum_a", ENUM)?;
    d.object("enum_b", ENUM)?;
    if v == 6 {
        return Ok(());
    }
    d.object("arp_presets", "std::vector<NI::MASCOMMON::DATA::ArpPreset>")?;
    if v < 31 {
        // Per-slot scale engine / arpeggiator (versions 8..30).
        if v <= 7 {
            return Ok(());
        }
        d.object("slot_scale_engine", "NI::MASCOMMON::DATA::ScaleEngine")?;
        if v == 8 {
            return Ok(());
        }
        d.object("slot_arpeggiator", "NI::MASCOMMON::DATA::Arpeggiator")?;
        if v <= 21 {
            return Ok(());
        }
        if v > 25 {
            d.object("f_ul_a", UL)?;
        }
    } else {
        d.object("slot_scale_engine", "NI::MASCOMMON::DATA::ScaleEngine")?;
        d.object("slot_arpeggiator", "NI::MASCOMMON::DATA::Arpeggiator")?;
        d.object("f_ul_a", UL)?;
    }
    d.object("f_ul_b", UL)?;
    d.object("f_ul_c", UL)?;
    if v < 23 {
        return Ok(());
    }
    d.object("arranger_state", "NI::MASCHINE::DATA::ArrangerState")?;
    if v > 26 {
        d.object("f_bool_tail", B)?;
    }
    Ok(())
}

/// Song fields after the loop parameter, versions >= 1.
fn song_after_loop(d: &mut Decoder, v: u32) -> Result<()> {
    d.object("f180", ENUM)?;
    d.object("num_section_banks", UL)?;
    d.object("focus_section_bank_index", UL)?;
    if v < 18 {
        d.object("legacy_bool_b", B)?;
        if v < 7 {
            d.object("legacy_ll", LL)?;
        } else if v > 9 {
            d.object("hw_note_repeat_locked", B)?;
        }
    } else {
        d.object("hw_note_repeat_locked", B)?;
    }
    Ok(())
}
