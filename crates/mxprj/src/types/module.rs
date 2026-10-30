//! Slots, chains and modules: Slot, Chain, Module and every exported Module subclass
//! (internal instruments, effects, plug-in host), plus the sampler's zones and takes.
//!
//! Module subclasses are reached through `std::shared_ptr<NI::MASCOMMON::DATA::Module>`;
//! the archive names each derived class by its boost export key, which is the C++
//! class name except for the two classes in [`EXPORTS`].
//!
//! Field names are struct offsets (`unknown_a8`) where the member is not known.
//! `unknown_tail` is the last member, whose load the compiler shared with other
//! functions (its offset is not visible in the decompile). The straight-line
//! loaders in `modules` were generated from the decompiled `serialize` bodies.

use super::Types;
use crate::container::Result;
use crate::decode::{Decoder, Value};

const MODULE: &str = "NI::MASCOMMON::DATA::Module";
const MNP_FLOAT: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<float>";
const MNP_BOOL: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>";
const MNP_ULONG: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>";
const MNP_DOUBLE: &str = "NI::MASCOMMON::DATA::MaschineNumericParameter<double>";
const ENUM: &str = "NI::MASCOMMON::DATA::EnumParameter";
const RANGE: &str = "NI::MASCOMMON::DATA::RangeParameter";
const QUICK_BROWSE: &str = "NI::DB3::QuickBrowseHandle";

/// Export keys that differ from the C++ class name.
const EXPORTS: &[(&str, &str)] = &[
    ("NI::MASCHINE::DATA::Sampler", "NI::MASCOMMON::DATA::Sampler"),
    ("NI::MASCHINE::DATA::AudioModule", "NI::MASCOMMON::DATA::AudioModule"),
];

pub(super) fn register(t: &mut Types) {
    for (export, ty) in EXPORTS {
        t.exports.insert(export.to_string(), ty.to_string());
    }
    slots(t);
    sampler(t);
    plugins(t);
    modules(t);
}

/// `int32 size` followed by `size` opaque bytes (a nested NI stream), kept as `<name>_data`.
fn blob(d: &mut Decoder, name: &str) -> Result<()> {
    let at = d.offset();
    let n = d.int(name)?;
    let n = usize::try_from(n).map_err(|_| crate::container::Error { offset: at, what: "negative blob size" })?;
    d.bytes(&format!("{name}_data"), n).map(drop)
}

fn slots(t: &mut Types) {
    t.object_vector("NI::MASCOMMON::DATA::Slot", false);
    // Module is abstract: boost does not pre-register it before reading the pointer,
    // so the first module in an archive has a new class id and its export name
    // (verified on .mxinst/.mxfx: class-id numbering only fits this way).
    t.shared_ptr(MODULE, true);
    t.scoped_ptr("NI::SOUND::SoundInfo2", false);
    t.scoped_ptr(QUICK_BROWSE, false);

    t.add(MODULE, |_, _| Ok(()));
    // SoundInfoContainer::serializeSoundInfo: scoped_ptr<SoundInfo2>; the SoundInfo2
    // itself is an NISound stream stored as a sized blob.
    // Also registered (identically) by song.rs; add_once keeps either order working.
    t.add_once("NI::SOUND::SoundInfo2", |d, _| blob(d, "nisound_size"));
    t.add("NI::MASCOMMON::DATA::Slot", |d, _| {
        d.object("active", MNP_BOOL)?;
        d.object("module", "std::shared_ptr<NI::MASCOMMON::DATA::Module>")?;
        d.object("sound_info", "boost::scoped_ptr<NI::SOUND::SoundInfo2>")?;
        Ok(())
    });
    // Chain: the slots, then one QuickBrowse handle per slot (written either directly
    // or via QuickBrowseable::loadQuickBrowseHandle, which is the same object load).
    t.add("NI::MASCOMMON::DATA::Chain", |d, _| {
        d.object("slots", "NI::ML::TM::ObjectVector<NI::MASCOMMON::DATA::Slot>")?;
        for _ in 0..d.field("slots").map_or(0, slot_count) {
            d.object("quick_browse", QUICK_BROWSE)?;
        }
        Ok(())
    });
}

/// Number of elements in a decoded `ObjectVector`.
fn slot_count(ov: &Value) -> usize {
    let Value::Obj(ov) = ov else { return 0 };
    match ov.get("objects") {
        Some(Value::Obj(v)) => match v.get("items") {
            Some(Value::List(items)) => items.len(),
            _ => 0,
        },
        _ => 0,
    }
}

fn sampler(t: &mut Types) {
    t.object_vector("NI::MASCOMMON::DATA::Zone", false);
    t.object_vector("NI::MASCOMMON::DATA::Take", false);

    t.add("NI::MASCOMMON::AliasPath", |d, _| {
        d.string("root")?;
        d.string("library")?;
        Ok(())
    });
    // A file reference: kind, path, and for non-zero kinds an alias.
    t.add("NI::MASCOMMON::FileLocation", |d, _| {
        let kind = d.int("kind")?;
        d.string("path")?;
        if kind != 0 {
            d.object("alias", "NI::MASCOMMON::AliasPath")?;
        }
        Ok(())
    });
    t.add("NI::MASCOMMON::DATA::QuantizedFloatParameter", |d, _| d.base(MNP_FLOAT));
    // A take of an Audio module (docs/audio-plugin.md): its file; its length in beats
    // (the Source Length; its tempo follows from the file's frames); the play range;
    // the selection (as a Zone's); `unknown_98` has been the file's frame count.
    t.add("NI::MASCOMMON::DATA::Take", |d, v| {
        d.object("file", "NI::MASCOMMON::FileLocation")?;
        d.object("length", "NI::MASCOMMON::DATA::QuantizedFloatParameter")?;
        if v != 0 {
            d.object("play_range", RANGE)?;
            d.object("selection_range", RANGE)?;
            d.object("offset", MNP_ULONG)?;
            d.object("unknown_98", MNP_ULONG)?;
            d.object("zoom_factor", MNP_DOUBLE)?;
            if v > 1 {
                d.object("quick_browse", QUICK_BROWSE)?;
            }
        }
        Ok(())
    });
    t.add("NI::MASCOMMON::DATA::Zone", |d, v| {
        d.object("file", "NI::MASCOMMON::FileLocation")?;
        d.object("play_range", RANGE)?;
        d.object("loop_range", RANGE)?;
        d.object("loop_active", MNP_BOOL)?;
        // Names from the parameter labels in Zone::Zone.
        for n in ["x_fade", "attack", "decay", "root_key"] {
            d.object(n, MNP_ULONG)?;
        }
        d.object("key_range", RANGE)?;
        d.object("velocity_range", RANGE)?;
        // gain: linear amplitude (LevelScale: dB = 20*log10); pan -1..1; tune in semitones (±36)
        for n in ["gain", "pan", "tune"] {
            d.object(n, MNP_FLOAT)?;
        }
        d.object("offset", MNP_ULONG)?;
        d.object("zoom_factor", MNP_DOUBLE)?;
        d.object("selection_range", RANGE)?;
        if v != 0 {
            d.object("unknown_150", MNP_ULONG)?;
        }
        if v > 1 {
            d.object("quick_browse", QUICK_BROWSE)?;
        } else {
            d.object("quick_browse", "boost::scoped_ptr<NI::DB3::QuickBrowseHandle>")?;
        }
        Ok(())
    });
    t.add("NI::MASCOMMON::DATA::Sampler", |d, v| {
        d.base(MODULE)?;
        d.object("zones", "NI::ML::TM::ObjectVector<NI::MASCOMMON::DATA::Zone>")?;
        // Names from the parameter labels in Sampler::Sampler and what the processor
        // does with them (docs/sampler-dsp.md): `velocity_*` and `modwheel_*` are the
        // Velocity / Modwheel page.
        if v != 0 {
            d.object("editor_tab", ENUM)?;
            if v != 1 {
                d.object("takes", "NI::ML::TM::ObjectVector<NI::MASCOMMON::DATA::Take>")?;
            }
        }
        d.object("polyphony", ENUM)?;
        d.object("glide", MNP_FLOAT)?;
        d.object("pitchbend", MNP_FLOAT)?;
        d.object("mode", ENUM)?;
        d.object("envelope_type", ENUM)?;
        d.object("reverse", MNP_BOOL)?;
        for n in ["tune", "start", "comp", "drive", "sample_rate", "bits"] {
            d.object(n, MNP_FLOAT)?;
        }
        d.object("filter", ENUM)?;
        for n in [
            "cutoff",
            "resonance",
            "eq_frequency",
            "eq_width",
            "eq_gain",
            "velocity_start",
            "velocity_decay",
            "velocity_cutoff",
            "velocity_volume",
            "modwheel_start",
            "modwheel_cutoff",
            "modwheel_lfo",
            "modwheel_pan",
        ] {
            d.object(n, MNP_FLOAT)?;
        }
        d.object("volume_envelope", "NI::MASCOMMON::DATA::AHDSREnvelopeObject")?;
        d.object("mod_envelope", "NI::MASCOMMON::DATA::AHDSREnvelopeObject")?;
        d.object("lfo", "NI::MASCOMMON::DATA::LFOObject")?;
        Ok(())
    });
    // AudioModule (audio-loop module): takes, then version-gated parameters.
    t.add("NI::MASCOMMON::DATA::AudioModule", |d, v| {
        d.base(MODULE)?;
        // AudioModule::serializeTakes
        if v < 4 {
            if v < 2 {
                d.object("legacy_length", MNP_FLOAT)?;
            } else {
                d.object("legacy_length", "NI::MASCOMMON::DATA::QuantizedFloatParameter")?;
            }
            d.object("legacy_file", "NI::MASCOMMON::FileLocation")?;
        } else {
            d.object("takes", "NI::ML::TM::ObjectVector<NI::MASCOMMON::DATA::Take>")?;
        }
        // Mode 0 Loop, 1 Gate; Gate's fade; tune (semitones); pitchbend (0.5: none);
        // engine 0 Re-Pitch, 1 Stretch, 2 Formant (docs/audio-plugin.md).
        if v != 0 {
            d.object("mode", ENUM)?;
            d.object("fade", MNP_FLOAT)?;
            if v > 2 {
                d.object("tune", MNP_FLOAT)?;
                d.object("pitchbend", MNP_FLOAT)?;
                if v > 4 {
                    d.object("engine", ENUM)?;
                }
            }
        }
        Ok(())
    });
}

fn plugins(t: &mut Types) {
    t.object_vector("NI::MASCOMMON::DATA::PluginPage", false);
    t.shared_ptr("NI::MASCOMMON::DATA::PluginParameter", false);
    t.add("NI::MASCOMMON::DATA::PluginChunk", |d, _| blob(d, "size"));
    t.add("NI::MASCHINE::DATA::PluginWindowHandler", |d, _| blob(d, "size"));
    t.add("NI::MASCOMMON::DATA::PluginParameter", |d, v| {
        d.base(MNP_FLOAT)?;
        d.int("index")?;
        if v == 0 {
            d.string("custom_name_nis_mapping")?;
        } else {
            d.string("custom_name")?;
            d.int("unknown_flag")?;
        }
        d.string("section_name")?;
        Ok(())
    });
    t.add("NI::MASCOMMON::DATA::ForwardParameter", |d, _| {
        d.base("NI::MASCOMMON::DATA::MaschineParameter")?;
        d.pointer("target", "NI::MASCOMMON::DATA::MaschineParameter", false)?;
        d.string("custom_name")?;
        d.string("section_name")?;
        Ok(())
    });
    // boost::array<T, 8>: element count, then the elements (no item version).
    t.add("boost::array<std::shared_ptr<NI::MASCOMMON::DATA::PluginParameter>, 8ul>", |d, _| {
        let n = d.int("count")?;
        if !(0..=8).contains(&n) {
            return Err(d.err("boost::array count out of range"));
        }
        for _ in 0..n {
            d.object("item", "std::shared_ptr<NI::MASCOMMON::DATA::PluginParameter>")?;
        }
        Ok(())
    });
    t.add("NI::MASCOMMON::DATA::PluginPage", |d, _| {
        d.base("NI::MASCOMMON::DATA::ParameterPage")?;
        d.object("parameters", "boost::array<std::shared_ptr<NI::MASCOMMON::DATA::PluginParameter>, 8ul>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::PluginHost", |d, v| {
        d.base(MODULE)?;
        for n in ["unknown_1b0", "unknown_1b8", "plugin_popup_open", "unknown_4c8"] {
            d.object(n, MNP_BOOL)?;
        }
        d.string("plugin_id")?;
        d.string("plugin_name")?;
        d.object("chunk", "NI::MASCOMMON::DATA::PluginChunk")?;
        if v != 0 {
            d.int("unknown_160")?;
        }
        d.object("parameters", "NI::MASCHINE::DATA::PluginParameters")?;
        d.object("window", "NI::MASCHINE::DATA::PluginWindowHandler")?;
        Ok(())
    });
}

/// Straight-line loaders generated from the decompiled `serialize` bodies.
#[rustfmt::skip]
fn modules(t: &mut Types) {
    t.add("NI::MASCHINE::DATA::AudioInputModule", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("unknown_a8", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    // docs/synth-bass-synth.md §1.
    t.add("NI::MASCHINE::DATA::BassSynth", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("shape", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("octave", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        if v > 1 {
            d.object("shape_2", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("octave_2", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
            d.object("detune", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("osc_mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        }
        d.object("cutoff", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("resonance", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mod_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("drive", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        if v == 0 {
            // Version 0 stored Glide as a switch; Maschine loads it into `glide`.
            d.object("legacy_glide", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        } else {
            d.object("glide", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        }
        d.object("glide_time", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    // docs/fx-beat-delay.md §1.
    t.add("NI::MASCHINE::DATA::BeatDelay", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("time", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("offset", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("feedback", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("crossover", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("color", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("split", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("stereo", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("unit", "NI::MASCOMMON::DATA::EnumParameter")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::CabinetEmulation", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("cabinet", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("microphone", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("distance", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Chorder", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("unknown_a8", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        d.object("unknown_b0", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        d.object("unknown_b8", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        d.object("unknown_c0", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        d.object("unknown_c8", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        d.object("unknown_d0", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        d.object("unknown_d8", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        d.object("unknown_tail", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Chorus", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("rate", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Compressor", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        if v != 0 {
            d.object("mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        }
        d.object("threshold", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("knee", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("attack", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("release", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("gain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("side_chain_filter", "NI::MASCHINE::DATA::SideChainFilter")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Cymbal", |d, _| {
        // docs/synth-cymbal.md §1.2 (Tune Stepped, 0xb8, is not stored).
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("engine", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("impact", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("width", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("velocity", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("density", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tone", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("crash_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("edge", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("bell", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("character", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tail", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::DSeqModule", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("unknown_a8", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("unknown_b0", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("unknown_b8", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_c0", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_c8", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("unknown_d0", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_110", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_150", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_d8", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_118", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_158", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_e0", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_120", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_160", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_e8", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_128", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_168", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_f0", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_130", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_170", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_f8", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_138", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_178", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_100", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_140", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_180", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_108", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_148", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_tail", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::EQ", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("low_freq", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("low_gain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("low_mid_freq", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("low_mid_gain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("high_mid_freq", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("high_mid_gain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("high_freq", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("high_gain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("low_mid_width", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("high_mid_width", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("output_gain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::EventMonitorModule", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Filter", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("cutoff", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("resonance", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("source", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("speed", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("lfo_shape", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("phase", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("smooth", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("env_shape", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("side_chain_filter", "NI::MASCHINE::DATA::SideChainFilter")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Flanger", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("frequency", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        if v != 0 {
            d.object("source", "NI::MASCOMMON::DATA::EnumParameter")?;
        }
        d.object("amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("speed", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("shape", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("stereo", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("feedback", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("invert", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::FM", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("rate", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("split", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("contour", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::FreqShifter", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("coarse", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("fine", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("feedback", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("stereo", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("invert", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Gate", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("threshold", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("attack", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hold", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("release", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("side_chain_filter", "NI::MASCHINE::DATA::SideChainFilter")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::GrainDelay", |d, _| {
        // docs/fx-grain-delay.md §1 (members 0xa8 … 0xe8)
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("pitch", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("size", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("jitter", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("reverse", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("space", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("density", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("modulation", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("stereo", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::GrainStretch", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("on", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("stretch", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("loop_length", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("pitch", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("link", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("size", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Hihat", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        // docs/synth-hihat.md §1 (names: Maschine's HihatModel properties).
        d.object("mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("color", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("gate", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("velocity_scaling", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hihat606_sat", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hihat606_noise", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hihat606_seed", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hihat606_random", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("cymbal_strike", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("cymbal_sample", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("cymbal_distress", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        if v > 1 {
            d.object("impact", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("hybrid_metallic", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("hybrid_character", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("hybrid_bend", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("hybrid_rattle", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        }
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Iceverb", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("color", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("ice", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("size", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Kick", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        // docs/synth-kick.md §1: the shared parameters, then each engine's own (named by
        // the KickModel QML property; offsets in the comments are the DATA::Kick members).
        d.object("engine", "NI::MASCOMMON::DATA::EnumParameter")?; // 0xa8
        d.object("tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0xb0
        d.object("decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0xc0
        d.object("punch", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0xc8
        d.object("impact", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0xd8
        d.object("velocity", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0xe0
        d.object("sub_kick_p_env_decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0xe8
        d.object("sub_kick_thin_attack", "NI::MASCOMMON::DATA::EnumParameter")?; // 0xf0
        d.object("sub_kick_attack_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0xf8
        d.object("sub_kick_color", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x100
        d.object("clean_bd_drive_gain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x108
        d.object("clean_bd_drive_tone", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x110
        d.object("clean_bd_bias", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x118
        d.object("clean_bd_drive_dw", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x120
        d.object("dusty_kick_filter", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x128
        d.object("dusty_kick_noise", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x130
        d.object("acoustic_supporter_tune_skin", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x150
        d.object("acoustic_supporter_room_size", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x158
        d.object("acoustic_supporter_room_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x160
        d.object("acoustic_supporter_room_character", "NI::MASCOMMON::DATA::EnumParameter")?; // 0x168
        d.object("bend", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0xd0
        d.object("medio_snap", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x170
        d.object("medio_punch_mode", "NI::MASCOMMON::DATA::EnumParameter")?; // 0x178
        d.object("medio_punch_decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x180
        d.object("medio_punch_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x188
        d.object("jungle_crispness_mode", "NI::MASCOMMON::DATA::EnumParameter")?; // 0x190
        d.object("jungle_crispness_decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x198
        d.object("jungle_crispness_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x1a0
        if v > 1 {
            d.object("grit_kick_gate", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?; // 0x138
            d.object("grit_kick_aero_grind", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x140
            d.object("grit_kick_aero_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?; // 0x148
        }
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Limiter", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("threshold", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("side_chain_filter", "NI::MASCHINE::DATA::SideChainFilter")?;
        if v == 0 {
            return Ok(());
        }
        d.object("mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("release", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("ceiling", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::LoFi", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("sample_rate", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("smooth", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("bits", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("stereo", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Maximizer", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("curve", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("turbo", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("side_chain_filter", "NI::MASCHINE::DATA::SideChainFilter")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Metaverb", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("size", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("low", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("high", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("pan", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::MullhollandDrive", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        // Class version 0 has no Mode (Maschine loads it as 0, Mulholland) and no
        // Analog parameters (docs/fx-distortion.md §1.2).
        if v == 0 {
        } else {
            d.object("mode", "NI::MASCOMMON::DATA::EnumParameter")?;
            d.object("analog_saturation", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("analog_bass", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("analog_tone", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("analog_focus", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
            d.object("analog_definition", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("analog_mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        }
        d.object("drive", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("color", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("feedback", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tone", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tone_mod", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("gate", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("release", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Perc", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("velocity", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("shaker_grain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("shaker_filter", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("shaker_env_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("shaker_attack", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("shaker_hold", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("shaker_release", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("fractal_freq_a", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("fractal_freq_b", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("fractal_freq_c", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("fractal_fm", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("fractal_am", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("fractal_tune_hold", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("fractal_keytrack", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("fractal_glide", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        if v != 0 {
            d.object("shaker_accent", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("shaker_fill", "NI::MASCOMMON::DATA::EnumParameter")?;
            d.object("shaker_rate", "NI::MASCOMMON::DATA::EnumParameter")?;
            d.object("shaker_swing", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("shaker_twist", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("shaker_length", "NI::MASCOMMON::DATA::EnumParameter")?;
            d.object("shaker_offset", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
            d.object("shaker_sync", "NI::MASCOMMON::DATA::EnumParameter")?;
        }
        d.object("decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("impact", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("kettle_puff", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("kettle_damp", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    // Field names: the parameters' labels by object offset, from PerformanceFX's
    // constructor (docs/superpowers/plans/2026-10-22-phase5-perform-fx.md).
    t.add("NI::MASCHINE::DATA::PerformanceFX", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("touchstrip_pos", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("touchstrip_on_off", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("filter_type", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("filter_min_freq", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_max_freq", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_resonance", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_saturation", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_out_level", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("flanger_decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("flanger_stereo", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("flanger_tone", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("flanger_saturation", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("flanger_wet_level", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("flanger_polarity", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("burst_time", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("burst_feedback", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("burst_time_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("burst_ts_assign", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("burst_filter", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("burst_feed", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("burst_mix_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("burst_wet_level", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("reso_resonance", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("reso_feedback", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("reso_time", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("reso_saturation", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("reso_pump", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("reso_wet_level", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("reso_mix_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("stutter_ts_assign", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("stutter_length", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("stutter_pitch", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("stutter_direction", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("stutter_quantize", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("stutter_gate", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("stutter_retrigger", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("ring_spread", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("ring_rev_mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("ring_mix_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("ring_rev_time", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("ring_dry_wet", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("ring_glide", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tremolo_trem_vibr", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tremolo_vibr_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("tremolo_attack_release", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tremolo_stereo", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tremolo_depth_min", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tremolo_depth_max", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tremolo_rate_min", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tremolo_rate_max", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("scratcher_delay_time", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("scratcher_feedback", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("scratcher_freq_shift", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("scratcher_wet_level", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Phaser", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("frequency", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("source", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("speed", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("shape", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("stereo", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("feedback", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("eight_pole", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::PlateReverb", |d, v| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("plate_pre_delay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("plate_decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("plate_low_shelf", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("plate_high_damp", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        if v == 0 {
            return Ok(());
        }
        d.object("mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("wet_gain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("room_rev_time", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("room_size", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("room_damp", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("room_mod", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("room_diff", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("room_pre_delay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("room_high_cut", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("room_low_shelf", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hall_rev_time", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hall_size", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hall_damp", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hall_mod", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hall_diff", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hall_pre_delay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hall_high_cut", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hall_low_shelf", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::PolySynth", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("pm_filter_env", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("pm_osc_b", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("pm_freq_a", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("pm_pw_a", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("pm_filter", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("osc_a_tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        d.object("osc_a_saw", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("osc_a_pulse", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("osc_a_pulse_width", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("osc_a_sync", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("osc_b_tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        d.object("osc_b_fine", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("osc_b_saw", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("osc_b_triangle", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("osc_b_pulse", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("osc_b_pulse_width", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("osc_b_low", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("osc_b_kbd", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("mixer_osc_a", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mixer_osc_b", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mixer_noise", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mixer_external", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_cutoff", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_resonance", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_env_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_kbd", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_invert", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("filter_attack", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_sustain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter_release", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("lfo_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("lfo_rate", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("lfo_saw", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("lfo_triangle", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("lfo_pulse", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("mod_lfo_noise", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mod_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mod_freq_a", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("mod_freq_b", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("mod_pw_a", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("mod_pw_b", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("mod_filter", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("glide", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("amp_attack", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("amp_decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("amp_sustain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("amp_release", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("release", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("velocity", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("lfo_voice_trig", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("hold", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("filter_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("detune", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("volume", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("pitchbend_range", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        d.object("voices", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("voice_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Reflex", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("color", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("smooth", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("size", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Resochord", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("style", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("chord", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("spread", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
        d.object("brightness", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("feedback", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Reverb", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("room", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("size", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("low", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("high", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("pan", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("stereo", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("freeze", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Saturator", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("classic_input", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("classic_drive", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("classic_contour", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tape_input", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tape_drive", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tape_contour", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tube_drive", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tube_charge", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tube_bass", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tube_treble", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tube_gain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tube_overload", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("tube_eq_bypass", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Snare", |d, _| {
        // docs/synth-snare.md §1 (engine-specific fields: Volt/Bit, Pow/Clap, Sharp, modal).
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("skin_tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("impact", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("body", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("spectra_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("spectra_tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("spectra_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("spectra_decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("velocity", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("osc_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("noise_color", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("noise_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("punch", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("osc_mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("grit", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("gate", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        d.object("bend", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("room", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("focus", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("noise", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::TestToneModule", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        d.object("unknown_a8", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("unknown_b0", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::Tom", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        // Labels from Tom's constructor (docs/synth-tom.md §1). Tronic uses Interval and
        // the FM fields; Fractal KTr. Mode … Color; Floor Flex, Skin and Air; Floor and
        // High Tension. +0xb8 ("Tune Stepped") is not stored; Skin (+0x160) is stored
        // before Air (+0x158).
        d.object("engine", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("tune", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("bend", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("impact", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("velocity", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("interval", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("fm_freq", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("fm_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("fm_decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("keytrack_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("mix", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("glide", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("transpose", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("freq_a", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("freq_b", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("fm", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("am", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("color", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("tension", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("flex", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("skin", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("air", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::TransientDesigner", |d, _| {
        d.base("NI::MASCOMMON::DATA::Module")?;
        // Serialized +0xb8, +0xa8, +0xb0, +0xc0 (docs/fx-transient-master.md §1.2).
        d.object("input_gain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("attack", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("sustain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("limit", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        Ok(())
    });
    t.add("NI::MASCOMMON::DATA::AHDSREnvelopeObject", |d, _| {
        d.object("attack", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("hold", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("decay", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("sustain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("release", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        // Modulation amounts (labels from AHDSREnvelopeObject's constructor).
        d.object("pitch", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("cutoff", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("drive", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("pan", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCOMMON::DATA::LFOObject", |d, _| {
        d.object("type", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("speed", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("phase", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("sync_mode", "NI::MASCOMMON::DATA::EnumParameter")?;
        d.object("pitch_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("cutoff_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("drive_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        // The fourth amount, as the processor's LFO→pan.
        d.object("pan_amount", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::PluginParameters", |d, _| {
        d.object("unknown_48", "NI::ML::TM::ObjectVector<NI::MASCOMMON::DATA::PluginPage>")?;
        d.object("unknown_68", "NI::MASCHINE::DATA::SideChainFilter")?;
        Ok(())
    });
    t.add("NI::MASCOMMON::DATA::SampleSlice", |d, _| {
        d.object("unknown_48", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_50", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        d.object("unknown_tail", "NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>")?;
        Ok(())
    });
    t.add("NI::MASCHINE::DATA::SideChainFilter", |d, _| {
        d.object("source", "NI::MASCHINE::DATA::SideChainInputParameter")?;
        d.object("gain", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("center_freq", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("width", "NI::MASCOMMON::DATA::MaschineNumericParameter<float>")?;
        d.object("filter", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
        Ok(())
    });
}
