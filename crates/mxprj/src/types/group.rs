//! Groups and sounds: Group, Sound, MixingLayer, MacroPage, MidiAutomator, Modulator.

use super::Types;
use crate::container::Result;
use crate::decode::{Decoder, Value};

const MNP_BOOL: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>";
const MNP_FLOAT: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<float>";
const MNP_ULONG: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>";
const MNP_INT: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<int>";
const MNP_DOUBLE: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<double>";
const ENUM: &str = "NI::MASCOMMON::DATA::EnumParameter";
const WSTRING: &str = "NI::MASCOMMON::DATA::WStringParameter";
const AUDIO_IN: &str = "NI::MASCHINE::DATA::AudioInputParameter";
const AUDIO_OUT: &str = "NI::MASCHINE::DATA::AudioOutputParameter";
const EVENT_IN: &str = "NI::MASCHINE::DATA::EventInputParameter";
const EVENT_OUT: &str = "NI::MASCHINE::DATA::EventOutputParameter";
const COLOR: &str = "NI::MASCHINE::DATA::ColorParameter";
const SIDECHAIN_MAP: &str = "std::map<unsigned long, NI::MASCHINE::DATA::SideChainInputParameterValue>";

const GROUP: &str = "NI::MASCHINE::DATA::Group";
const SOUND: &str = "NI::MASCHINE::DATA::Sound";
const MIXING_LAYER: &str = "NI::MASCHINE::DATA::MixingLayer";
const MACRO_PAGE: &str = "NI::MASCHINE::DATA::MacroPage";
const MIDI_AUTOMATOR: &str = "NI::MASCHINE::DATA::MidiAutomator";
const MODULATOR: &str = "NI::MASCHINE::DATA::Modulator";
const SOUND_CONFIG: &str = "NI::MASCHINE::DATA::SoundConfig";
const SNAPSHOTS: &str = "NI::MASCHINE::DATA::ParameterSnapshotsManager";

pub(super) fn register(t: &mut Types) {
    t.object_vector(SOUND, false);
    t.object_vector(MACRO_PAGE, false);
    t.object_vector(MIDI_AUTOMATOR, false);
    t.object_vector(MODULATOR, false);
    t.weak_ptr(GROUP, false);
    t.weak_ptr(SOUND, false);

    t.add(SOUND, sound);
    t.add(GROUP, group);
    t.add(MIXING_LAYER, mixing_layer);
    t.add(MACRO_PAGE, |d, _| {
        d.base("NI::MASCOMMON::DATA::ParameterPage")?;
        d.object("parameters", "boost::array<std::shared_ptr<NI::MASCOMMON::DATA::ForwardParameter>, 8>").map(drop)
    });
    t.add(MIDI_AUTOMATOR, |d, _| {
        d.object("mode", ENUM)?;
        d.object("controller", MNP_ULONG).map(drop)
    });
    // A pointer to the modulated parameter (any MaschineParameter subclass).
    t.add(MODULATOR, |d, _| d.pointer("target", "NI::MASCOMMON::DATA::MaschineParameter", true).map(drop));

    // boost::array<T, N> is a native array: count, then N items (no item version).
    t.shared_ptr("NI::MASCOMMON::DATA::ForwardParameter", false);
    t.add_once("boost::array<std::shared_ptr<NI::MASCOMMON::DATA::ForwardParameter>, 8>", |d, _| {
        let at = d.offset();
        let n = d.int_or_blank("count", 8)?;
        if n != 8 {
            return Err(crate::container::Error { offset: at, what: "boost::array count is not 8" });
        }
        d.group("items", |d| {
            for i in 0..8 {
                d.object(&format!("item{i}"), "std::shared_ptr<NI::MASCOMMON::DATA::ForwardParameter>")?;
            }
            Ok(())
        })
    });

    t.add_once(format!("std::vector<{SOUND_CONFIG}>"), |d, _| d.collection("items", |d| d.object("item", SOUND_CONFIG).map(drop)));
    t.add_once(SOUND_CONFIG, sound_config);
    parameter_snapshots(t);
}

const SNAPSHOT: &str = "NI::MASCHINE::DATA::ParameterSnapshot";
const VALUES_MAP: &str = "NI::MASCHINE::DATA::ParameterSnapshotValuesMap";
const VALUES_ENTRY: &str = "NI::MASCHINE::DATA::ParameterSnapshotValuesMap::Entry";
const MASCHINE_PARAMETER: &str = "NI::MASCOMMON::DATA::MaschineParameter";

/// `ParameterSnapshotsManager` (MixingLayer v1+) and the types below it.
fn parameter_snapshots(t: &mut Types) {
    t.object_map(SNAPSHOT, false);
    t.vector_of_shared("NI::MASCOMMON::DATA::Slot", false);
    t.add_once(SNAPSHOTS, |d, _| {
        d.object("snapshots", &format!("NI::ML::TM::ObjectMap<unsigned long, {SNAPSHOT}>"))?;
        // The Lock settings (`ParameterSnapshotsManager`'s parameters): Morph, its Mode
        // (Travel, Target), Time (1/2 Bar … 16 Bars) and Grid (1/8 … Bar), the focused
        // bank and how many banks there are.
        d.object("morph", MNP_BOOL)?;
        d.object("morph_mode", ENUM)?;
        d.object("morph_time", ENUM)?;
        d.object("morph_grid", ENUM)?;
        d.object("focus_bank", MNP_ULONG)?;
        d.object("banks", MNP_ULONG).map(drop)
    });
    t.add_once(SNAPSHOT, |d, _| {
        d.object("slots", "std::vector<std::shared_ptr<NI::MASCOMMON::DATA::Slot>>")?;
        d.object("values", VALUES_MAP).map(drop)
    });
    let flat_map = format!("boost::container::flat_map<{MASCHINE_PARAMETER}*, {VALUES_ENTRY}>");
    let pair = format!("std::pair<{MASCHINE_PARAMETER}*, {VALUES_ENTRY}>");
    let fm = flat_map.clone();
    t.add_once(VALUES_MAP, move |d, v| {
        if v == 0 {
            // v0: parameter -> value, plus a bimap pairing parameters.
            d.object("values", LEGACY_VALUES)?;
            return d.object("pairs", BIMAP).map(drop);
        }
        d.object("entries", &fm).map(drop)
    });
    legacy_values(t);
    let p2 = pair.clone();
    t.add_once(flat_map, move |d, _| d.collection("items", |d| d.object("item", &p2).map(drop)));
    t.add_once(pair, |d, _| {
        d.pointer("parameter", MASCHINE_PARAMETER, true)?;
        d.object("entry", VALUES_ENTRY).map(drop)
    });
    t.add_once(VALUES_ENTRY, |d, _| {
        d.f32("value")?;
        if d.int("has_target")? != 0 {
            d.pointer("target", MASCHINE_PARAMETER, true)?;
            d.f32("target_value")?;
        }
        Ok(())
    });
}

const LEGACY_VALUES: &str = "std::map<NI::MASCOMMON::DATA::MaschineParameter*, float>";
const LEGACY_PAIR: &str = "std::pair<NI::MASCOMMON::DATA::MaschineParameter* const, float>";
const BIMAP: &str = "boost::bimaps::bimap<NI::MASCOMMON::DATA::MaschineParameter*, NI::MASCOMMON::DATA::MaschineParameter*>";
const BIMAP_CONTAINER: &str = "boost::multi_index::multi_index_container<boost::bimaps::relation<MaschineParameter*, MaschineParameter*>>";
const BIMAP_RELATION: &str = "boost::bimaps::relation::mutant_relation<MaschineParameter*, MaschineParameter*>";
const BIMAP_VALUE_VERSION: &str =
    "boost::multi_index::detail::serialization_version<boost::bimaps::relation::mutant_relation<MaschineParameter*, MaschineParameter*>>";
const BIMAP_NODE: &str =
    "boost::multi_index::detail::index_node_base<boost::bimaps::relation::mutant_relation<MaschineParameter*, MaschineParameter*>>";

/// `ParameterSnapshotValuesMap` v0 storage: `map<MaschineParameter*, float>` and a
/// `bimap<MaschineParameter*, MaschineParameter*>` (a two-index multi_index_container).
fn legacy_values(t: &mut Types) {
    t.add_once(LEGACY_PAIR, |d, _| {
        d.pointer("parameter", MASCHINE_PARAMETER, true)?;
        d.f32("value").map(drop)
    });
    t.add_once(LEGACY_VALUES, |d, _| d.collection("items", |d| d.object("item", LEGACY_PAIR).map(drop)));
    t.add_once(BIMAP, |d, _| d.object("container", BIMAP_CONTAINER).map(drop));
    t.add_once(BIMAP_RELATION, |d, _| {
        d.pointer("left", MASCHINE_PARAMETER, true)?;
        d.pointer("right", MASCHINE_PARAMETER, true).map(drop)
    });
    // Empty marker objects (tracked so later pointers can refer to nodes).
    t.add_once(BIMAP_VALUE_VERSION, |_, _| Ok(()));
    t.add_once(BIMAP_NODE, |_, _| Ok(()));
    // multi_index_container::load: count, value version (v2+), per item the relation
    // and its node, then the header node. Both indices are unique, so no reorder lists.
    t.add_once(BIMAP_CONTAINER, |d, v| {
        let at = d.offset();
        let count = d.int("count")?;
        if count < 0 || count as u64 > d.remaining() as u64 {
            return Err(crate::container::Error { offset: at, what: "bimap count out of range" });
        }
        if v >= 2 {
            d.object("value_version", BIMAP_VALUE_VERSION)?;
        }
        d.group("items", |d| {
            for i in 0..count {
                d.object(&format!("r{i}"), BIMAP_RELATION)?;
                d.object(&format!("n{i}"), BIMAP_NODE)?;
            }
            Ok(())
        })?;
        d.object("header", BIMAP_NODE).map(drop)
    });
}

/// Number of items in a decoded `ObjectVector<T>` value.
fn object_vector_len(v: &Value) -> usize {
    let Value::Obj(ov) = v else { return 0 };
    let Some(Value::Obj(vec)) = ov.get("objects") else { return 0 };
    match vec.get("items") {
        Some(Value::List(items)) => items.len(),
        _ => 0,
    }
}

/// `MixingLayer::serialize`: the per-channel mixer strip shared by Sound and Group.
/// Field names follow the labels `MixingLayer::MixingLayer` gives its parameters, in
/// creation order (= serialization order): MacroPage, MacroFocus, Name, then the
/// groove: swing Amount, Cycle, Invert, Bounce.
fn mixing_layer(d: &mut Decoder, v: u32) -> Result<()> {
    d.object("macro_page", MNP_ULONG)?;
    d.object("macro_focus", MNP_ULONG)?;
    d.object("name", WSTRING)?;
    d.object("swing_amount", MNP_FLOAT)?;
    d.object("swing_cycle", ENUM)?;
    d.object("swing_invert", MNP_BOOL)?;
    d.object("swing_bounce", MNP_BOOL)?;
    d.object("chain", "NI::MASCOMMON::DATA::Chain")?;
    d.object("modulators", &format!("NI::ML::TM::ObjectVector<{MODULATOR}>"))?;
    d.object("macro_pages", &format!("NI::ML::TM::ObjectVector<{MACRO_PAGE}>"))?;
    d.object("midi_automators", &format!("NI::ML::TM::ObjectVector<{MIDI_AUTOMATOR}>"))?;
    if v > 0 {
        d.object("parameter_snapshots", SNAPSHOTS)?;
    }
    Ok(())
}

/// Output and aux routing: shared by `Sound::serialize` and
/// `Group::serializeTrackParametersLegacy` (which adds a color).
/// The group variant lacks `audio_mute`. Labels from Sound::Sound / Track::Track.
fn track_parameters(d: &mut Decoder, v: u32, sound: bool) -> Result<()> {
    d.object("audio_output", AUDIO_OUT)?;
    d.object("cue_enabled", MNP_BOOL)?;
    d.object("level", MNP_FLOAT)?;
    d.object("pan", MNP_FLOAT)?;
    d.object("mute", MNP_BOOL)?;
    if sound {
        d.object("audio_mute", MNP_BOOL)?;
    }
    d.object("aux1_output", AUDIO_OUT)?;
    d.object("aux1_level", MNP_FLOAT)?;
    if v == 0 {
        d.object("aux1_order_legacy", MNP_BOOL)?;
    } else {
        d.object("aux1_order", ENUM)?;
    }
    d.object("aux2_output", AUDIO_OUT)?;
    d.object("aux2_level", MNP_FLOAT)?;
    if v == 0 {
        d.object("aux2_order_legacy", MNP_BOOL)?;
    } else {
        d.object("aux2_order", ENUM)?;
    }
    Ok(())
}

/// `Sound::serialize`.
fn sound(d: &mut Decoder, v: u32) -> Result<()> {
    d.object("content_info", "boost::scoped_ptr<NI::MASCHINE::DATA::ContentInfo>")?;
    d.object("event_input", EVENT_IN)?;
    if v <= 5 {
        // v < 5 may also rewrite the event input from runtime state; no reads.
        d.object("active_legacy", MNP_BOOL)?;
        d.object("midi_channel", ENUM)?;
        if v > 3 {
            d.object("midi_thru", MNP_BOOL)?;
        }
    } else {
        d.object("event_input_b", EVENT_IN)?;
        d.object("midi_channel", ENUM)?;
        d.object("midi_thru", MNP_BOOL)?;
    }
    d.object("audio_input", AUDIO_IN)?;
    d.object("input_gain", MNP_FLOAT)?;
    track_parameters(d, v, true)?;
    d.object("event_output", EVENT_OUT)?;
    d.object("event_output_channel", ENUM)?;
    d.object("transpose", MNP_INT)?;
    if v > 9 {
        d.object("sequence_variation", "NI::MASCHINE::DATA::SequenceVariationModule")?;
        d.object("sequence_generation", "NI::MASCHINE::DATA::SequenceGenerationModule")?;
    }
    d.object("base_key", MNP_ULONG)?;
    for n in ["choke_group", "choke_mode", "link_group", "link_mode"] {
        d.object(n, ENUM)?;
    }
    d.object("color", COLOR)?;
    d.object("pianoroll_offset_y", MNP_ULONG)?;
    d.object("pianoroll_offset_y_hw", MNP_ULONG)?;
    d.object("pianoroll_zoom_y", MNP_DOUBLE)?;
    if v > 6 {
        d.object("velocity", MNP_ULONG)?;
    }
    d.object("custom_name", MNP_BOOL)?;
    if v > 7 {
        d.object("midi_macro", "NI::MASCOMMON::DATA::MidiMacro")?;
    }
    d.base(MIXING_LAYER)?;
    d.object("sequences", "NI::ML::TM::ObjectVector<NI::MASCOMMON::DATA::Sequence>")?;
    // SoundInfoContainer::serializeSoundInfo
    d.object("sound_info", "boost::scoped_ptr<NI::SOUND::SoundInfo2>")?;
    Ok(())
}

/// `Group::serialize`.
fn group(d: &mut Decoder, v: u32) -> Result<()> {
    d.object("content_info", "boost::scoped_ptr<NI::MASCHINE::DATA::ContentInfo>")?;
    // Group sets its EventInputParameter's skip flag to !isProjectLevel, where
    // isProjectLevel is only consulted from v15 (earlier: always skip).
    let skip = v < 15 || !super::song::is_project_level(d);
    super::song::with_routing_skip(d, skip, "event_input", EVENT_IN)?;
    if v < 0x13 {
        d.object("unknown_bool_legacy", MNP_BOOL)?;
    }
    d.object("midi_channel", ENUM)?;
    d.object("start_note", MNP_ULONG)?;
    if v >= 5 {
        d.object("midi_thru", MNP_BOOL)?;
    }
    let legacy_track = v < 0xd;
    if legacy_track {
        // Group::serializeTrackParametersLegacy
        d.group("track_parameters_legacy", |d| {
            track_parameters(d, v, false)?;
            d.object("color", COLOR).map(drop)
        })?;
    }
    d.object("unknown_bool_b", MNP_BOOL)?;
    d.object("unknown_bool_c", MNP_BOOL)?;
    d.object("unknown_bool_d", MNP_BOOL)?;
    if v & !1 == 10 {
        d.object("unknown_bool_v10", MNP_BOOL)?;
    }
    d.object("unknown_ulong_b", MNP_ULONG)?;
    if legacy_track {
        d.object("unknown_ulong_legacy", MNP_ULONG)?;
    }
    d.object("unknown_ulong_c", MNP_ULONG)?;
    if v < 4 {
        if v == 3 {
            d.object("type_legacy", MNP_ULONG)?;
        }
    } else {
        d.object("midi_batch_export", ENUM)?;
    }
    d.object("sounds", &format!("NI::ML::TM::ObjectVector<{SOUND}>"))?;
    if legacy_track {
        d.base(MIXING_LAYER)?;
    } else {
        d.base("NI::MASCHINE::DATA::Track")?;
    }
    d.object("patterns_by_id", "NI::ML::TM::ObjectMap<unsigned long, NI::MASCHINE::DATA::EventPattern>")?;
    // Patterns are stored only at project level (and on the clipboard).
    if v > 10 && super::song::is_project_level(d) {
        d.object("patterns", "NI::ML::TM::ObjectVector<NI::MASCHINE::DATA::EventPattern>")?;
        d.object("sequence", "NI::MASCOMMON::DATA::Sequence")?;
    }
    // Group::serializeSoundStates
    d.object("sound_configs", &format!("std::vector<{SOUND_CONFIG}>"))?;
    // Group::serializeQuickBrowse: one handle per sound.
    let n = d.field("sounds").map_or(0, object_vector_len);
    d.group("quick_browse", |d| {
        for i in 0..n {
            d.object(&format!("sound{i}"), "NI::DB3::QuickBrowseHandle")?;
        }
        Ok(())
    })?;
    if v > 1 {
        d.object("sidechain_inputs", SIDECHAIN_MAP)?;
    }
    d.object("sound_info", "boost::scoped_ptr<NI::SOUND::SoundInfo2>")?;
    if v < 6 {
        return Ok(());
    }
    d.object("unknown_ulong_d", MNP_ULONG)?;
    if v > 0x14 {
        d.object("unknown_ulong_e", MNP_ULONG)?;
    }
    d.object("unknown_ulong_f", MNP_ULONG)?;
    d.object("unknown_ulong_g", MNP_ULONG)?;
    d.object("unknown_ulong_h", MNP_ULONG)?;
    if v < 9 {
        if v > 6 {
            d.object("unknown_bool_e", MNP_BOOL)?;
            if v != 7 {
                d.object("unknown_enum_c", ENUM)?;
            }
        }
        return Ok(());
    }
    d.object("unknown_ulong_i", MNP_ULONG)?;
    d.object("unknown_ulong_j", MNP_ULONG)?;
    if v > 0x16 {
        d.object("unknown_ulong_k", MNP_ULONG)?;
    }
    d.object("unknown_bool_e", MNP_BOOL)?;
    d.object("unknown_enum_c", ENUM)?;
    if v < 0x10 {
        if v < 0xe {
            return Ok(());
        }
    } else {
        d.object("unknown_ulong_l", MNP_ULONG)?;
    }
    d.object("unknown_enum_d", ENUM)?;
    d.object("strumming_map", "boost::scoped_ptr<NI::MASCHINE::DATA::StrummingMap>")?;
    if v < 0x11 {
        return Ok(());
    }
    d.object("unknown_bool_f", MNP_BOOL)?;
    if v == 0x11 {
        return Ok(());
    }
    d.object("key_mode", ENUM)?;
    if v < 0x13 {
        return Ok(());
    }
    d.object("key_mode_plugin", ENUM)?;
    if v == 0x13 {
        return Ok(());
    }
    d.object("unknown_enum_g", ENUM)?;
    d.object("unknown_enum_h", ENUM)?;
    Ok(())
}

/// `SoundConfig::serialize`: a sound's saved routing/state inside a group file.
fn sound_config(d: &mut Decoder, v: u32) -> Result<()> {
    d.string("midi_source")?;
    d.int("midi_channel")?;
    if v >= 5 {
        d.int("midi_thru")?;
    }
    if v >= 7 {
        d.string("midi_source_plugin")?;
    } else if v >= 3 {
        d.int("unknown_flag_legacy")?;
    }
    d.object("audio_input", "NI::MASCHINE::DATA::AudioInputParameterValue")?;
    d.f32("input_gain")?;
    d.object("audio_output", "NI::MASCHINE::DATA::AudioOutputParameterValue")?;
    if v == 0 {
        d.f32("unknown_f_legacy_a")?;
        d.f32("unknown_f_legacy_b")?;
    }
    d.int("mute")?;
    d.int("audio_mute")?;
    d.object("aux1_output", "NI::MASCHINE::DATA::AudioOutputParameterValue")?;
    d.f32("aux1_level")?;
    d.int("aux1_order")?;
    d.object("aux2_output", "NI::MASCHINE::DATA::AudioOutputParameterValue")?;
    d.f32("aux2_level")?;
    d.int("aux2_order")?;
    d.object("event_output", "NI::MASCHINE::DATA::EventOutputParameterValue")?;
    d.int("midi_out_channel")?;
    d.int("transpose")?;
    if v == 0 {
        d.int("unknown_legacy")?;
    }
    for n in ["choke_group", "choke_mode", "link_group", "link_mode", "swing_bounce", "cue_enabled"] {
        d.int(n)?;
    }
    if v >= 4 {
        d.object("sidechain_inputs", SIDECHAIN_MAP)?;
    }
    Ok(())
}
