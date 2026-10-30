//! Loaders for the C++ types in a project archive, keyed by C++ type name.
//!
//! Each loader mirrors one decompiled `serialize`/`load`/`load_object_data`
//! (`tools/ghidra/decompile.sh` + `tools/ghidra/condense.py`). Field names follow
//! the C++ member where known; `unknown_*` and positional names mark fields whose
//! meaning has not been established.
//!
//! Type families live in submodules, each with `pub(super) fn register(t: &mut Types)`.
//! Generic boost/std containers are registered through the helpers here.

use crate::container::Result;
use crate::decode::{Decoder, Loader};
use std::collections::HashMap;
use std::rc::Rc;

pub mod classes;
mod group;
mod module;
mod remote;
mod sequence;
mod song;

pub struct Types {
    pub loaders: HashMap<String, Loader>,
    pub exports: HashMap<String, String>,
}

/// How a `ValueParameter<T>` stores its value.
#[derive(Clone, Copy)]
pub(crate) enum Scalar {
    /// Raw 4-byte float.
    F32,
    /// Raw 8-byte double.
    F64,
    /// Portable integer (bool, int, long long, unsigned long).
    Int,
    /// Narrow string converted to wide on load.
    WString,
}

impl Scalar {
    pub(crate) fn read(self, d: &mut Decoder, name: &str) -> Result<()> {
        match self {
            Scalar::F32 => d.f32(name).map(drop),
            Scalar::F64 => d.f64(name).map(drop),
            Scalar::Int => d.int(name).map(drop),
            Scalar::WString => d.string(name).map(drop),
        }
    }
}

/// `T` spellings as they appear in the C++ template names, with their encoding.
pub(crate) const NUMERIC: &[(&str, Scalar)] = &[
    ("float", Scalar::F32),
    ("double", Scalar::F64),
    ("bool", Scalar::Int),
    ("int", Scalar::Int),
    ("long long", Scalar::Int),
    ("unsigned long", Scalar::Int),
];

impl Types {
    /// Register a loader. Panics on duplicates so two families can't silently
    /// disagree about a type.
    pub(crate) fn add(&mut self, name: impl Into<String>, f: impl Fn(&mut Decoder, u32) -> Result<()> + 'static) {
        let name = name.into();
        assert!(!self.loaders.contains_key(&name), "loader registered twice: {name}");
        self.loaders.insert(name, Rc::new(f));
    }

    /// Like `add`, but a no-op if the type is already registered (for generic
    /// containers that several families need).
    pub(crate) fn add_once(&mut self, name: impl Into<String>, f: impl Fn(&mut Decoder, u32) -> Result<()> + 'static) {
        self.loaders.entry(name.into()).or_insert_with(|| Rc::new(f));
    }

    pub fn new() -> Self {
        let mut t = Types { loaders: HashMap::new(), exports: HashMap::new() };
        t.parameters();
        t.project();
        song::register(&mut t);
        group::register(&mut t);
        module::register(&mut t);
        sequence::register(&mut t);
        remote::register(&mut t);
        t
    }

    /// A blank object of `type_name` (see [`crate::decode::blank`]). Returns it and
    /// the next free object id.
    pub fn blank(
        &self,
        type_name: &str,
        spec: &crate::decode::BlankSpec,
    ) -> std::result::Result<(crate::decode::Obj, u64), (crate::container::Error, String)> {
        crate::decode::blank(type_name, spec, classes::LIBRARY_VERSION, &self.loaders, &self.exports)
    }

    /// Root type of a document, from the item id of the container's `MAS2` child.
    pub fn root_for(item_id: [u8; 4]) -> Option<&'static str> {
        Some(match &item_id {
            b"PROJ" => "NI::MASCHINE::DATA::Project",
            b"GRP " => "NI::MASCHINE::DATA::Group",
            b"SND " => "NI::MASCHINE::DATA::Sound",
            b"SLOT" => "NI::MASCOMMON::DATA::Slot",
            _ => return None,
        })
    }

    // ---- NI::ML::TM / NI::MASCOMMON parameters ------------------------------

    fn parameters(&mut self) {
        self.add("NI::ML::TM::Parameter", |d, v| {
            if v > 0 {
                d.string("id")?;
            }
            Ok(())
        });
        self.add("NI::MASCOMMON::DATA::MaschineParameter", |_, _| Ok(()));

        let mut scalars: Vec<(&str, Scalar)> = NUMERIC.to_vec();
        scalars.push(("std::wstring", Scalar::WString));
        for (t, kind) in scalars {
            self.add(format!("NI::ML::TM::ValueParameter<{t}>"), move |d, _| {
                d.base("NI::ML::TM::Parameter")?;
                kind.read(d, "value")
            });
        }
        for &(t, kind) in NUMERIC {
            let value_param = format!("NI::ML::TM::ValueParameter<{t}>");
            let numeric_param = format!("NI::ML::TM::NumericParameter<{t}>");
            self.add(numeric_param.clone(), move |d, _| d.base(&value_param));
            // Three values of type T and a flag; constructor arguments suggest a
            // default and a range, not yet confirmed which is which.
            self.add(format!("NI::MASCOMMON::DATA::MaschineNumericParameter<{t}>"), move |d, _| {
                d.base(&numeric_param)?;
                d.base("NI::MASCOMMON::DATA::MaschineParameter")?;
                kind.read(d, "t0")?;
                kind.read(d, "t1")?;
                d.int("flag")?;
                kind.read(d, "t2")
            });
        }
        self.add("NI::MASCOMMON::DATA::EnumParameter", |d, _| d.base("NI::MASCOMMON::DATA::MaschineNumericParameter<unsigned long>"));
        self.add("NI::MASCOMMON::DATA::WStringParameter", |d, _| {
            d.base("NI::ML::TM::ValueParameter<std::wstring>")?;
            d.base("NI::MASCOMMON::DATA::MaschineParameter")
        });
    }

    // ---- generic containers ------------------------------------------------
    //
    // boost conventions (boost/serialization/*.hpp), all verified on real files:
    // - vector / set / map: count, item version (library version > 3), items
    // - std::shared_ptr<T>: object with preamble, holding a T* pointer
    // - std::weak_ptr<T>: object holding a std::shared_ptr<T> object
    // - std::pair<K, V>: object with preamble; first, then second
    // - boost::scoped_ptr<T>: object with preamble, holding a T* pointer

    /// `std::shared_ptr<T>` (and `T*` pointer semantics).
    pub(crate) fn shared_ptr(&mut self, t: &'static str, abstract_: bool) {
        self.add_once(format!("std::shared_ptr<{t}>"), move |d, _| d.pointer("px", t, abstract_).map(drop));
    }

    pub(crate) fn weak_ptr(&mut self, t: &'static str, abstract_: bool) {
        self.shared_ptr(t, abstract_);
        let sp = format!("std::shared_ptr<{t}>");
        self.add_once(format!("std::weak_ptr<{t}>"), move |d, _| d.object("sp", &sp).map(drop));
    }

    pub(crate) fn scoped_ptr(&mut self, t: &'static str, abstract_: bool) {
        self.add_once(format!("boost::scoped_ptr<{t}>"), move |d, _| d.pointer("px", t, abstract_).map(drop));
    }

    /// `std::vector<std::shared_ptr<T>>`.
    pub(crate) fn vector_of_shared(&mut self, t: &'static str, abstract_: bool) {
        self.shared_ptr(t, abstract_);
        let sp = format!("std::shared_ptr<{t}>");
        self.add_once(format!("std::vector<std::shared_ptr<{t}>>"), move |d, _| d.collection("items", |d| d.object("item", &sp).map(drop)));
    }

    /// `NI::ML::TM::ObjectVector<T>`: vector<shared_ptr<T>>, then a T* (current), then set<T*>.
    pub(crate) fn object_vector(&mut self, t: &'static str, abstract_: bool) {
        self.vector_of_shared(t, abstract_);
        let vec = format!("std::vector<std::shared_ptr<{t}>>");
        let set = format!("std::set<{t}*>");
        self.add_once(set.clone(), move |d, _| d.collection("items", |d| d.pointer("item", t, abstract_).map(drop)));
        self.add_once(format!("NI::ML::TM::ObjectVector<{t}>"), move |d, _| {
            d.object("objects", &vec)?;
            d.pointer("current", t, abstract_)?;
            d.object("selection", &set)?;
            Ok(())
        });
    }

    /// `std::map<K, std::shared_ptr<V>>` with a portable-integer key `key_name`
    /// as spelled in the C++ type (e.g. "unsigned long").
    pub(crate) fn map_int_to_shared(&mut self, key: &'static str, v: &'static str, abstract_: bool) {
        self.shared_ptr(v, abstract_);
        let sp = format!("std::shared_ptr<{v}>");
        let pair = format!("std::pair<{key} const, std::shared_ptr<{v}>>");
        let sp2 = sp.clone();
        self.add_once(pair.clone(), move |d, _| {
            d.int("first")?;
            d.object("second", &sp2).map(drop)
        });
        self.add_once(format!("std::map<{key}, std::shared_ptr<{v}>>"), move |d, _| {
            d.collection("items", |d| d.object("item", &pair).map(drop))
        });
    }

    /// `NI::ML::TM::ObjectMap<unsigned long, V>`: map<ulong, shared_ptr<V>>, then a V*.
    pub(crate) fn object_map(&mut self, v: &'static str, abstract_: bool) {
        self.map_int_to_shared("unsigned long", v, abstract_);
        let map = format!("std::map<unsigned long, std::shared_ptr<{v}>>");
        self.add_once(format!("NI::ML::TM::ObjectMap<unsigned long, {v}>"), move |d, _| {
            d.object("objects", &map)?;
            d.pointer("current", v, abstract_)?;
            Ok(())
        });
    }

    /// `std::vector<unsigned long>` and similar vectors of portable integers.
    pub(crate) fn vector_of_int(&mut self, elem: &'static str) {
        self.add_once(format!("std::vector<{elem}>"), |d, _| d.collection("items", |d| d.int("item").map(drop)));
    }

    // ---- NI::MASCHINE::DATA::Project ----------------------------------------

    fn project(&mut self) {
        self.scoped_ptr("NI::MASCHINE::DATA::ContentInfo", false);
        self.add("NI::MASCHINE::DATA::ContentInfo", |d, _| {
            if d.int("has_content")? != 0 {
                d.string("library")?;
                d.string("product")?;
                d.string("snpid")?;
                d.int("checksum")?;
            }
            Ok(())
        });
        self.object_vector("NI::MASCHINE::DATA::Song", false);
        self.scoped_ptr("NI::MASCHINE::DATA::WorkspaceConfig", false);
        // Flags and sizes of the desktop workspace; names are struct offsets.
        self.add("NI::MASCHINE::DATA::WorkspaceConfig", |d, v| {
            d.int("b00")?;
            d.int("b01")?;
            if v < 2 {
                d.int("old_b")?;
            }
            for n in ["b02", "b03", "b04"] {
                d.int(n)?;
            }
            for n in ["i08", "i10", "i18", "b20", "i28", "i30"] {
                d.int(n)?;
            }
            if v == 0 {
                d.int("old_i")?;
            }
            for n in ["i34", "b38", "b39", "b3a", "b3b"] {
                d.int(n)?;
            }
            d.f32("f3c")?;
            d.int("b40")?;
            d.int("b41")?;
            Ok(())
        });

        self.add("NI::MASCHINE::DATA::Project", |d, v| {
            d.object("content_info", "boost::scoped_ptr<NI::MASCHINE::DATA::ContentInfo>")?;
            d.object("name", "NI::MASCOMMON::DATA::WStringParameter")?;
            d.int("unknown_flag")?;
            d.int("unknown_i64")?;
            if v == 0 {
                d.object("songs", "NI::ML::TM::ObjectVector<NI::MASCHINE::DATA::Song>")?;
                return project_remote_tail(d, v);
            }
            d.int("unknown_pair_lo")?;
            d.int("unknown_pair_hi")?;
            if v >= 2 {
                d.object("workspace", "boost::scoped_ptr<NI::MASCHINE::DATA::WorkspaceConfig>")?;
            }
            d.object("songs", "NI::ML::TM::ObjectVector<NI::MASCHINE::DATA::Song>")?;
            project_remote_tail(d, v)
        });
    }
}

/// Project after the song list: MIDI remote mapping, then version-gated extras.
fn project_remote_tail(d: &mut Decoder, v: u32) -> Result<()> {
    if v < 3 {
        d.object("remote_map", "NI::ML::TM::ObjectMap<unsigned long, NI::MASCOMMON::DATA::RemoteParameter>")?;
    } else {
        d.object("midi_mapper", "NI::MASCOMMON::DATA::MidiMapper")?;
    }
    if v >= 4 {
        d.object("unknown_wstring_a", "NI::MASCOMMON::DATA::WStringParameter")?;
        d.object("unknown_wstring_b", "NI::MASCOMMON::DATA::WStringParameter")?;
    }
    d.object("remote_parameters", "NI::ML::TM::ObjectVector<NI::MASCOMMON::DATA::RemoteParameter>")?;
    if v >= 5 {
        d.object("unknown_bool_param", "NI::MASCOMMON::DATA::MaschineNumericParameter<bool>")?;
    }
    if v >= 6 {
        d.object("version_info", "NI::MASCOMMON::DATA::VersionInfo")?;
    }
    Ok(())
}

impl Default for Types {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every class Maschine 3 writes can be built blank, and a blank project writes
    /// and reads back unchanged.
    #[test]
    fn blank_objects() {
        let t = Types::new();
        let classes = classes::maschine3();
        let spec = crate::decode::BlankSpec { classes: &classes, first_oid: 1, ints: &|_, _| None, project_level: true };
        for (name, _, _) in classes::MASCHINE3 {
            if t.loaders.contains_key(*name) {
                t.blank(name, &spec).unwrap_or_else(|(e, at)| panic!("{name}: {e} in {at}"));
            }
        }
        let (project, _) = t.blank("NI::MASCHINE::DATA::Project", &spec).unwrap();
        let header = crate::archive::Header { library_version: classes::LIBRARY_VERSION, flags: 0 };
        let (bytes, written) = crate::decode::encode(project, "NI::MASCHINE::DATA::Project", header, &t.loaders, &t.exports).unwrap();
        let mut r = crate::archive::Reader::new(&bytes, 0);
        let h = r.header().unwrap();
        let mut d = crate::decode::Decoder::new(r, h.library_version, &t.loaders, &t.exports);
        let back = d.root("NI::MASCHINE::DATA::Project").unwrap();
        assert_eq!(back, written);
    }
}
