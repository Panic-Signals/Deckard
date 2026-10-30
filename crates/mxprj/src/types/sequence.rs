//! Patterns and events: EventPattern, Sequence, events, SequenceGeneration/VariationModule, Arpeggiator, ArpPreset, ScaleEngine, StrummingMap.
//!
//! A pattern's notes live in `Sequence`, a boost `multi_index_container` of
//! `TimedEvent { position: int64 ticks, event: Event* }`. Events are polymorphic and
//! written under legacy export names (`NI::MASCHINE::DATA::NoteEvent`, ...), mapped
//! to the current classes in [`EXPORTS`]. Field names ending in a hex offset are
//! C++ struct offsets whose meaning is not established.

use super::{Scalar, Types};
use crate::container::Result;
use crate::decode::{Decoder, Value};

const EVENT: &str = "NI::MASCOMMON::DATA::Event";
const PATTERN: &str = "NI::MASCHINE::DATA::EventPattern";
const SEQUENCE: &str = "NI::MASCOMMON::DATA::Sequence";
const SEQ_REF: &str = "NI::MASCHINE::DATA::SequenceReference";
const TIMED_EVENT: &str = "NI::MASCOMMON::DATA::TimedEvent";
/// `boost::multi_index_container<TimedEvent, indexed_by<ordered_non_unique<TimeIndex, ...>>>`;
/// only `Sequence` refers to it, so a short key is used.
const EVENTS: &str = "NI::MASCOMMON::DATA::Sequence::Events";
const EVENT_NODE: &str = "boost::multi_index::detail::index_node_base<NI::MASCOMMON::DATA::TimedEvent>";
const EVENT_VALUE_VERSION: &str = "boost::multi_index::detail::serialization_version<NI::MASCOMMON::DATA::TimedEvent>";

const MNP: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter";
const ENUM: &str = "NI::MASCOMMON::DATA::EnumParameter";

/// Export names written for polymorphic `Event*` pointers -> registered type.
/// From the `BOOST_CLASS_EXPORT` key strings in the binary.
const EXPORTS: &[(&str, &str)] = &[
    ("NI::MASCHINE::DATA::NoteEvent", "NI::MASCOMMON::DATA::NoteEvent"),
    ("NI::MASCHINE::DATA::LengthEvent", "NI::MASCOMMON::DATA::LengthEvent"),
    ("NI::MASCHINE::DATA::ParameterEvent", "NI::MASCOMMON::DATA::ParameterEvent"),
    (
        "NI::MASCHINE::DATA::BoolModulationEvent",
        "NI::MASCOMMON::DATA::ModulationEvent<NI::MASCOMMON::DATA::MaschineNumericParameter<bool>>",
    ),
    ("NI::MASCHINE::DATA::IntModulationEvent", "NI::MASCOMMON::DATA::ModulationEvent<NI::MASCOMMON::DATA::MaschineNumericParameter<int>>"),
    (
        "NI::MASCHINE::DATA::SizeTModulationEvent",
        "NI::MASCOMMON::DATA::ModulationEvent<NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>>",
    ),
    (
        "NI::MASCHINE::DATA::TickModulationEvent",
        "NI::MASCOMMON::DATA::ModulationEvent<NI::MASCOMMON::DATA::MaschineNumericParameter<long long>>",
    ),
    (
        "NI::MASCHINE::DATA::FloatModulationEvent",
        "NI::MASCOMMON::DATA::ModulationEvent<NI::MASCOMMON::DATA::MaschineNumericParameter<float>>",
    ),
    (
        "NI::MASCHINE::DATA::DoubleModulationEvent",
        "NI::MASCOMMON::DATA::ModulationEvent<NI::MASCOMMON::DATA::MaschineNumericParameter<double>>",
    ),
    ("NI::MASCHINE::DATA::EnumModulationEvent", "NI::MASCOMMON::DATA::ModulationEvent<NI::MASCOMMON::DATA::EnumParameter>"),
];

/// Modulated parameter type -> how `ModulationEvent<T>` version 0 stored its constant value.
const MODULATED: &[(&str, Scalar)] = &[
    ("NI::MASCOMMON::DATA::MaschineNumericParameter<bool>", Scalar::Int),
    ("NI::MASCOMMON::DATA::MaschineNumericParameter<int>", Scalar::Int),
    ("NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>", Scalar::Int),
    ("NI::MASCOMMON::DATA::MaschineNumericParameter<long long>", Scalar::Int),
    ("NI::MASCOMMON::DATA::MaschineNumericParameter<float>", Scalar::F32),
    ("NI::MASCOMMON::DATA::MaschineNumericParameter<double>", Scalar::F64),
    ("NI::MASCOMMON::DATA::EnumParameter", Scalar::Int),
];

fn mnp(t: &str) -> String {
    format!("{MNP}<{t}>")
}

pub(super) fn register(t: &mut Types) {
    events(t);
    patterns(t);
    modules(t);
    strumming(t);
}

/// Register `f` under `name` and, when the type has a legacy export name, under that
/// too: a pointer whose class id was already seen is looked up by the name it was
/// first registered with, which for polymorphic pointers is the export name.
fn add_with_alias(t: &mut Types, name: &str, f: impl Fn(&mut Decoder, u32) -> Result<()> + Clone + 'static) {
    for (export, actual) in EXPORTS {
        if *actual == name {
            t.add(*export, f.clone());
        }
    }
    t.add(name, f);
}

// ---- events -------------------------------------------------------------

fn events(t: &mut Types) {
    for (export, actual) in EXPORTS {
        t.exports.insert(export.to_string(), actual.to_string());
    }

    // Event: a flag at +0x11 (set on construction; e.g. selection/mute state).
    t.add(EVENT, |d, _| d.int("flag_11").map(drop));
    add_with_alias(t, "NI::MASCOMMON::DATA::LengthEvent", |d, _| {
        d.base(EVENT)?;
        d.int("length").map(drop)
    });
    // v0 stored a mute flag (probability 0 or 100); v1+ a raw probability byte (0-100).
    t.add("NI::MASCOMMON::DATA::ProbabilityEventPolicy<NI::MASCOMMON::DATA::NoteEvent>", |d, v| {
        if v == 0 { d.int("muted").map(drop) } else { d.byte("probability").map(drop) }
    });
    add_with_alias(t, "NI::MASCOMMON::DATA::NoteEvent", |d, v| {
        d.base("NI::MASCOMMON::DATA::LengthEvent")?;
        if v != 0 {
            d.base("NI::MASCOMMON::DATA::ProbabilityEventPolicy<NI::MASCOMMON::DATA::NoteEvent>")?;
        }
        d.int("note")?;
        d.int("velocity").map(drop)
    });
    add_with_alias(t, "NI::MASCOMMON::DATA::ParameterEvent", |d, _| d.base(EVENT));
    for &(param, scalar) in MODULATED {
        let name = format!("NI::MASCOMMON::DATA::ModulationEvent<{param}>");
        add_with_alias(t, &name, move |d, v| {
            d.base("NI::MASCOMMON::DATA::ParameterEvent")?;
            d.pointer("parameter", param, false)?;
            if v == 0 { scalar.read(d, "constant") } else { d.object("modulation", "NI::MASCOMMON::Modulation").map(drop) }
        });
    }
    t.add("NI::MASCOMMON::Modulation", |d, _| {
        d.object("a", "std::array<float, 4>")?;
        d.object("b", "std::array<float, 4>").map(drop)
    });
    // boost C-array: count, then that many raw floats.
    t.add_once("std::array<float, 4>", |d, _| {
        let n = d.int("count")?;
        if !(0..=4).contains(&n) {
            return Err(d.err("std::array<float, 4> count out of range"));
        }
        for i in 0..n {
            d.f32(&format!("v{i}"))?;
        }
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::AudioEvent", |d, v| {
        d.base("NI::MASCOMMON::DATA::LengthEvent")?;
        if v != 0 {
            d.pointer("take", "NI::MASCOMMON::DATA::Take", false)?;
        }
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::ClipEvent", |d, _| {
        d.base("NI::MASCOMMON::DATA::LengthEvent")?;
        d.pointer("pattern", PATTERN, false).map(drop)
    });
    t.add("NI::MASCHINE::DATA::MidiEvent", |d, _| {
        d.base(EVENT)?;
        d.int("i14")?;
        d.int("i18")?;
        d.int("i20").map(drop)
    });

    t.add(TIMED_EVENT, |d, _| {
        d.int("position")?;
        d.pointer("event", EVENT, false).map(drop)
    });
    // The class version of serialization_version<T> is T's version; no data.
    t.add(EVENT_VALUE_VERSION, |_, _| Ok(()));
    // Node position marker: empty object, tracked so later pointers can refer to it.
    t.add(EVENT_NODE, |_, _| Ok(()));
    t.add(EVENTS, events_container);

    t.add(SEQUENCE, |d, _| {
        d.object("events", EVENTS)?;
        d.pointer("current", EVENT, false).map(drop)
    });
    t.object_vector(SEQUENCE, false);
}

/// boost `multi_index_container::load` with a single `ordered_non_unique` index:
/// count; value version (v >= 2); per item a TimedEvent and its node marker; the
/// header node; then the index's reorder list (node pointers, null-terminated runs).
fn events_container(d: &mut Decoder, v: u32) -> Result<()> {
    let at = d.offset();
    let count = d.int("count")?;
    if count < 0 || count as u64 > d.remaining() as u64 {
        return Err(crate::container::Error { offset: at, what: "event count out of range" });
    }
    if v >= 2 {
        d.object("value_version", EVENT_VALUE_VERSION)?;
    }
    d.group("items", |d| {
        for i in 0..count {
            d.object(&format!("e{i}"), TIMED_EVENT)?;
            d.object(&format!("n{i}"), EVENT_NODE)?;
        }
        Ok(())
    })?;
    d.object("header", EVENT_NODE)?;
    // index_loader::load for ordered_non_unique.
    d.group("reorder", |d| {
        let mut i = 0;
        let mut next = |d: &mut Decoder| -> Result<bool> {
            i += 1;
            let name = format!("p{i}");
            d.pointer(&name, EVENT_NODE, false)?;
            Ok(d.field(&name) != Some(&Value::Null))
        };
        if !next(d)? {
            return Ok(());
        }
        loop {
            while next(d)? {}
            if !next(d)? {
                return Ok(());
            }
        }
    })
}

// ---- patterns ---------------------------------------------------------------

fn patterns(t: &mut Types) {
    t.add(SEQ_REF, |d, _| d.pointer("sequence", SEQUENCE, false).map(drop));
    t.shared_ptr(SEQ_REF, false);
    let pair = "std::pair<NI::MASCHINE::DATA::Sound const* const, std::shared_ptr<NI::MASCHINE::DATA::SequenceReference>>";
    let map = "std::map<NI::MASCHINE::DATA::Sound const*, std::shared_ptr<NI::MASCHINE::DATA::SequenceReference>>";
    t.add(pair, |d, _| {
        d.pointer("sound", "NI::MASCHINE::DATA::Sound", false)?;
        d.object("reference", "std::shared_ptr<NI::MASCHINE::DATA::SequenceReference>").map(drop)
    });
    t.add(map, move |d, _| d.collection("items", |d| d.object("item", pair).map(drop)));
    t.add("NI::ML::TM::ObjectMap<NI::MASCHINE::DATA::Sound const*, NI::MASCHINE::DATA::SequenceReference>", move |d, _| {
        d.object("objects", map)?;
        d.pointer("current", SEQ_REF, false).map(drop)
    });

    t.add(PATTERN, |d, v| {
        d.object("name", "NI::MASCOMMON::DATA::WStringParameter")?;
        d.object("length", &mnp("long long"))?;
        d.object("color", "NI::MASCHINE::DATA::ColorParameter")?;
        d.object("sequence", SEQUENCE)?;
        d.object("sound_sequences", "NI::ML::TM::ObjectMap<NI::MASCHINE::DATA::Sound const*, NI::MASCHINE::DATA::SequenceReference>")?;
        if (1..=4).contains(&v) {
            // Parameters dropped after version 4; read into temporaries.
            d.object("old_ll_a", &mnp("long long"))?;
            d.object("old_double", &mnp("double"))?;
            if v > 1 {
                d.object("old_ll_b", &mnp("long long"))?;
            }
            d.object("old_ll_c", &mnp("long long"))?;
            d.object("old_ll_d", &mnp("long long"))?;
        }
        if v > 2 {
            d.object("p80", &mnp("long long"))?;
            if v != 3 {
                d.object("p68", &mnp("bool"))?;
                d.object("start", &mnp("long long"))?;
            }
        }
        Ok(())
    });
    t.object_vector(PATTERN, false);
    t.object_map(PATTERN, false);
}

// ---- modules ----------------------------------------------------------------

fn modules(t: &mut Types) {
    t.add("NI::MASCHINE::DATA::SequenceGenerationModule", |d, _| {
        d.object("mode", ENUM)?;
        d.object("probability", &mnp("float"))?;
        d.object("velocity_amount", &mnp("float"))?;
        d.object("note_range", "NI::MASCOMMON::DATA::RangeParameter")?;
        d.object("note_count", &mnp("unsigned long"))?;
        d.object("distribution", ENUM)?;
        d.object("steps", &mnp("unsigned long"))?;
        d.object("fills", &mnp("unsigned long"))?;
        d.object("rotate", &mnp("unsigned long"))?;
        d.object("division", ENUM)?;
        d.object("live_mode", &mnp("bool")).map(drop)
    });
    t.add("NI::MASCHINE::DATA::SequenceVariationModule", |d, _| {
        for n in ["velocity_amount", "time_shift", "length_scale", "length_variation"] {
            d.object(n, &mnp("float"))?;
        }
        d.object("distribution", ENUM)?;
        d.object("live", &mnp("bool")).map(drop)
    });

    t.add("NI::MASCOMMON::DATA::Arpeggiator", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("pa8", &mnp("bool"))?;
        d.object("pb0", ENUM)?;
        d.object("type", ENUM)?;
        d.object("pb8", ENUM)?;
        d.object("sequence", ENUM)?;
        d.object("pd8", &mnp("float"))?;
        d.object("octaves", ENUM)?;
        d.object("dynamic", &mnp("float"))?;
        d.object("pc0", &mnp("long long"))?;
        d.object("gate", &mnp("float"))?;
        match v {
            0 => {}
            1 => {
                // Dropped enum parameter, read into a temporary.
                d.object("old_enum", ENUM)?;
            }
            _ => {
                d.object("hold", &mnp("bool"))?;
                if v >= 3 {
                    for n in ["retrigger", "repeat", "offset", "inversion"] {
                        d.object(n, ENUM)?;
                    }
                    d.object("min_key", &mnp("unsigned long"))?;
                    d.object("max_key", &mnp("unsigned long"))?;
                }
            }
        }
        Ok(())
    });

    t.add("NI::MASCOMMON::DATA::ArpPreset", |d, v| {
        for n in ["i00", "i08", "i18", "i20", "i28"] {
            d.int(n)?;
        }
        if v >= 1 {
            d.f32("f30")?;
            d.f32("f34")?;
            if v >= 2 {
                d.int("i10")?;
                d.f32("f38")?;
                for n in ["i40", "i48", "i50", "i58", "i60", "i68"] {
                    d.int(n)?;
                }
            }
        }
        Ok(())
    });
    t.add_once("std::vector<NI::MASCOMMON::DATA::ArpPreset>", |d, _| {
        d.collection("items", |d| d.object("item", "NI::MASCOMMON::DATA::ArpPreset").map(drop))
    });

    t.add("NI::MASCOMMON::DATA::ScaleEngine", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("active", &mnp("bool"))?;
        if v > 2 {
            d.object("scale_bank", ENUM)?;
        }
        for n in ["scale", "root_note", "key_mode", "chord_mode"] {
            d.object(n, ENUM)?;
        }
        if v == 0 {
            // Chord-set enum and three values converted on load.
            d.object("old_chord_set", ENUM)?;
            for n in ["old_a", "old_b", "old_c"] {
                d.int(n)?;
            }
            return Ok(());
        }
        for n in ["chord_type_chromatic", "pe0", "pe8"] {
            d.object(n, ENUM)?;
        }
        if v > 3 {
            d.object("chord_position", ENUM)?;
            if v > 4 {
                d.object("chord_screen_chord_mode", ENUM)?;
                if v > 5 {
                    d.string("s120")?;
                }
            }
        }
        Ok(())
    });
}

// ---- strumming --------------------------------------------------------------

fn strumming(t: &mut Types) {
    t.vector_of_int("unsigned long");
    t.add_once("std::vector<std::vector<unsigned long>>", |d, _| {
        // std::vector of a primitive is object_serializable (boost collection traits): no preamble.
        d.collection("items", |d| d.inline("item", "std::vector<unsigned long>").map(drop))
    });
    let pair = "std::pair<NI::MASCOMMON::HW::StrumMode const, std::vector<std::vector<unsigned long>>>";
    let map = "std::map<NI::MASCOMMON::HW::StrumMode, std::vector<std::vector<unsigned long>>>";
    t.add(pair, |d, _| {
        d.int("mode")?;
        d.object("steps", "std::vector<std::vector<unsigned long>>").map(drop)
    });
    t.add(map, move |d, _| d.collection("items", |d| d.object("item", pair).map(drop)));
    t.add("NI::MASCHINE::DATA::StrummingMap", move |d, _| d.object("map", map).map(drop));
    t.scoped_ptr("NI::MASCHINE::DATA::StrummingMap", false);
}
