//! MIDI remote control and project extras: MidiMapper, MidiMacro, RemoteParameter, RangeParameter, RangeHelperParameter, ParameterPage, VersionInfo, QuickBrowseHandle.

use super::Types;

/// `MidiMacro`'s constructor creates 128 per-CC parameters in a loop, then two
/// more; `serialize` writes each element of that vector. The count is runtime
/// state, not stored in the file.
const MIDI_MACRO_PARAMETERS: usize = 130;

pub(super) fn register(t: &mut Types) {
    // RemoteParameter: a pointer to the controlled parameter. MaschineParameter is
    // abstract (no pointer_iserializer exists for it), so it isn't pre-registered.
    t.add("NI::MASCOMMON::DATA::RemoteParameter", |d, _| d.pointer("parameter", "NI::MASCOMMON::DATA::MaschineParameter", true).map(drop));
    t.object_vector("NI::MASCOMMON::DATA::RemoteParameter", false);
    t.object_map("NI::MASCOMMON::DATA::RemoteParameter", false);

    t.add("NI::MASCOMMON::DATA::OperationId", |d, _| d.int("id").map(drop));
    t.add("NI::MASCOMMON::DATA::MidiOperation", |d, _| {
        d.object("operation", "NI::MASCOMMON::DATA::OperationId")?;
        d.int("unknown_flag").map(drop)
    });
    for v in ["NI::MASCOMMON::DATA::OperationId", "NI::MASCOMMON::DATA::MidiOperation"] {
        let pair = format!("std::pair<unsigned long const, {v}>");
        let pair2 = pair.clone();
        t.add(pair, move |d, _| {
            d.int("first")?;
            d.object("second", v).map(drop)
        });
        t.add(format!("std::map<unsigned long, {v}>"), move |d, _| d.collection("items", |d| d.object("item", &pair2).map(drop)));
    }

    // Version 1 stored OperationIds, later versions MidiOperations.
    t.add("NI::MASCOMMON::DATA::MidiMapper", |d, v| {
        d.object("remote_parameters", "NI::ML::TM::ObjectMap<unsigned long, NI::MASCOMMON::DATA::RemoteParameter>")?;
        if v == 1 {
            d.object("operations", "std::map<unsigned long, NI::MASCOMMON::DATA::OperationId>")?;
        }
        if v > 1 {
            d.object("operations", "std::map<unsigned long, NI::MASCOMMON::DATA::MidiOperation>")?;
        }
        Ok(())
    });

    t.add("NI::MASCOMMON::DATA::MidiMacro", |d, _| {
        d.group("parameters", |d| {
            for i in 0..MIDI_MACRO_PARAMETERS {
                d.object(&format!("p{i}"), "NI::MASCOMMON::DATA::MaschineNumericParameter<int>")?;
            }
            Ok(())
        })
    });

    // RangeHelperParameter derives from MaschineNumericParameter<unsigned long>
    // without its own serialize, so that type's load runs under this preamble.
    t.add("NI::MASCOMMON::DATA::RangeHelperParameter", |d, _| {
        d.base("NI::ML::TM::NumericParameter<unsigned long>")?;
        d.base("NI::MASCOMMON::DATA::MaschineParameter")?;
        d.int("t0")?;
        d.int("t1")?;
        d.int("flag")?;
        d.int("t2").map(drop)
    });
    t.add("NI::MASCOMMON::DATA::RangeParameter", |d, _| {
        d.base("NI::MASCOMMON::DATA::MaschineParameter")?;
        d.object("low", "NI::MASCOMMON::DATA::RangeHelperParameter")?;
        d.object("high", "NI::MASCOMMON::DATA::RangeHelperParameter").map(drop)
    });

    t.add("NI::MASCOMMON::DATA::ParameterPage", |d, _| d.object("name", "NI::MASCOMMON::DATA::WStringParameter").map(drop));

    // App version that wrote the project, e.g. 3, 5, 0, " (R0)".
    t.add("NI::MASCOMMON::DATA::VersionInfo", |d, _| {
        d.int("major")?;
        d.int("minor")?;
        d.int("patch")?;
        d.string("revision").map(drop)
    });

    // An int32 length and an opaque blob (NI::DB3 browser handle, read by
    // QuickBrowseHandle::read from a memory stream).
    t.add("NI::DB3::QuickBrowseHandle", |d, _| {
        let at = d.offset();
        let len = d.int("length")?;
        let len = usize::try_from(len).map_err(|_| crate::container::Error { offset: at, what: "negative blob length" })?;
        d.bytes("data", len).map(drop)
    });
    t.scoped_ptr("NI::DB3::QuickBrowseHandle", false);
}

#[cfg(test)]
mod tests {
    use crate::Project;
    use crate::archive::Reader;
    use crate::decode::{Decoder, Value};
    use crate::types::Types;

    /// Version-6 projects end with a VersionInfo; decode it from each candidate
    /// start near the end and require one that consumes the archive exactly.
    #[test]
    fn version_info_ends_v6_projects() {
        let dirs = ["/Users/Shared", "/Library/Application Support/Native Instruments/Maschine 3"];
        let mut files = Vec::new();
        for d in dirs {
            collect(std::path::Path::new(d), &mut files, 0);
        }
        let types = Types::new();
        let (mut tried, mut ok) = (0, 0);
        for f in &files {
            let bytes = std::fs::read(f).unwrap();
            let Ok(p) = Project::parse(&bytes) else { continue };
            if p.project_header().map(|h| h.version) != Ok(6) {
                continue;
            }
            tried += 1;
            let n = p.archive.len();
            let found = (n.saturating_sub(80)..n).any(|start| {
                let r = Reader::new(&p.archive[start..], p.archive_offset + start);
                let mut d = Decoder::new(r, p.header.library_version, &types.loaders, &types.exports);
                match d.root("NI::MASCOMMON::DATA::VersionInfo") {
                    Ok(o) if d.r.remaining() == 0 => {
                        matches!(o.get("major"), Some(Value::Int(2..=3)))
                            && matches!(o.get("revision"), Some(Value::Str(s)) if s.contains("(R"))
                    }
                    _ => false,
                }
            });
            if found {
                ok += 1;
            } else {
                eprintln!("no VersionInfo tail: {}", f.display());
            }
        }
        eprintln!("VersionInfo tail decoded in {ok} / {tried} v6 projects");
        assert_eq!(ok, tried);
    }

    fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 5 {
                collect(&p, out, depth + 1);
            } else if p.extension().is_some_and(|x| x == "mxprj") {
                out.push(p);
            }
        }
    }
}
