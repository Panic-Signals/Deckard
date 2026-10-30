//! Reader for boost's example `portable_binary_iarchive` encoding.
//!
//! - integers (and boost's class-id / version / object-id types): one signed byte `n`;
//!   `0` means zero, otherwise `|n|` little-endian magnitude bytes follow and a
//!   negative `n` negates the value
//! - `bool`, `char`, `unsigned char`: one raw byte
//! - `float`, `double`: raw little-endian IEEE bytes
//! - `std::string`: integer length, then bytes
//!
//! Header: signature string `"serialization::archive"`, library version (integer),
//! then one raw flags byte.
//!
//! Object framing ([`Session`]) follows boost's `basic_iarchive`:
//!
//! - the first time a class is loaded, its preamble is `tracking` then `version`
//!   (both integers; this archive does not write the optional class id)
//! - a tracked object is then followed by its object id. Ids count up from 0
//!   across all tracked objects; an id below the count is a back-reference and no
//!   data follows
//! - a pointer starts with a class id (`-1` = null). The declared class is
//!   registered before that read (unless abstract), so a class id that is new at
//!   that point is a derived class and is followed by its export name; then the
//!   preamble (first use), then the object id if tracked
//! - class ids are assigned in first-use order, by objects, pointers and explicit
//!   `register_type` calls alike

use crate::container::{Error, Result};
use std::collections::HashMap;

pub const SIGNATURE: &str = "serialization::archive";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub library_version: u64,
    pub flags: u8,
}

#[derive(Debug, Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    /// Added to positions in errors so they point into the containing file.
    base: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8], base: usize) -> Self {
        Reader { buf, pos: 0, base }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Position in the containing file.
    pub fn offset(&self) -> usize {
        self.base + self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn err(&self, what: &'static str) -> Error {
        Error { offset: self.base + self.pos, what }
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.buf.get(self.pos..self.pos.checked_add(n).ok_or(self.err("length overflow"))?);
        let s = s.ok_or(self.err("unexpected end of archive"))?;
        self.pos += n;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error { offset: self.base + self.pos - 1, what: "bool is not 0 or 1" }),
        }
    }

    pub fn int(&mut self) -> Result<i64> {
        let size = self.u8()? as i8;
        if size == 0 {
            return Ok(0);
        }
        let n = size.unsigned_abs() as usize;
        if n > 8 {
            return Err(Error { offset: self.base + self.pos - 1, what: "integer wider than 8 bytes" });
        }
        let mut b = [0u8; 8];
        b[..n].copy_from_slice(self.bytes(n)?);
        let v = u64::from_le_bytes(b) as i64;
        Ok(if size < 0 { v.wrapping_neg() } else { v })
    }

    pub fn uint(&mut self) -> Result<u64> {
        let at = self.base + self.pos;
        u64::try_from(self.int()?).map_err(|_| Error { offset: at, what: "negative value for unsigned field" })
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }

    pub fn string(&mut self) -> Result<String> {
        let at = self.base + self.pos;
        let len = usize::try_from(self.uint()?).map_err(|_| Error { offset: at, what: "string too long" })?;
        if len > self.remaining() {
            return Err(Error { offset: at, what: "string longer than archive" });
        }
        let raw = self.bytes(len)?;
        String::from_utf8(raw.to_vec()).map_err(|_| Error { offset: at, what: "string is not UTF-8" })
    }

    pub fn header(&mut self) -> Result<Header> {
        if self.string()? != SIGNATURE {
            return Err(Error { offset: self.base, what: "missing serialization::archive signature" });
        }
        Ok(Header { library_version: self.uint()?, flags: self.u8()? })
    }
}

#[derive(Debug, Clone, Copy)]
struct Class {
    id: i64,
    /// Preamble already read: (tracking, version).
    preamble: Option<(bool, u32)>,
}

/// What an object or pointer slot in the archive turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// A null pointer.
    Null,
    /// Back-reference to an object loaded earlier; no data follows.
    Ref(u64),
    /// A new object; its data follows. `class` is the export name for a
    /// polymorphic pointer seen for the first time.
    New { version: u32, oid: Option<u64>, class_id: i64, class: Option<String> },
}

/// Class and object bookkeeping for one archive. Class keys are whatever names the
/// caller uses consistently (we use the C++ type name).
#[derive(Debug, Default)]
pub struct Session {
    classes: HashMap<String, Class>,
    by_id: Vec<String>,
    next_oid: u64,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// The key a class id was registered under.
    pub fn class_name(&self, id: i64) -> &str {
        &self.by_id[id as usize]
    }

    /// `register_type<T>()`: assigns a class id without reading anything.
    pub fn register(&mut self, key: &str) -> i64 {
        if let Some(c) = self.classes.get(key) {
            return c.id;
        }
        let id = self.by_id.len() as i64;
        self.by_id.push(key.to_string());
        self.classes.insert(key.to_string(), Class { id, preamble: None });
        id
    }

    fn preamble(&mut self, r: &mut Reader, key: &str) -> Result<(bool, u32)> {
        if let Some(p) = self.classes[key].preamble {
            return Ok(p);
        }
        let at = r.base + r.pos;
        let tracking = match r.int()? {
            0 => false,
            1 => true,
            _ => return Err(Error { offset: at, what: "tracking flag is not 0 or 1" }),
        };
        let version = u32::try_from(r.uint()?).map_err(|_| Error { offset: at, what: "class version out of range" })?;
        self.classes.get_mut(key).unwrap().preamble = Some((tracking, version));
        Ok((tracking, version))
    }

    /// Reads the object id of a tracked object. `Err(oid)` inside `Ok` means a
    /// back-reference to an object already loaded.
    fn track(&mut self, r: &mut Reader, tracking: bool) -> Result<std::result::Result<Option<u64>, u64>> {
        if !tracking {
            return Ok(Ok(None));
        }
        let at = r.base + r.pos;
        let oid = r.uint()?;
        if oid < self.next_oid {
            return Ok(Err(oid));
        }
        if oid != self.next_oid {
            return Err(Error { offset: at, what: "object id out of sequence" });
        }
        self.next_oid += 1;
        Ok(Ok(Some(oid)))
    }

    /// A non-pointer object of class `key` (`ar >> obj`, including `base_object`).
    pub fn object(&mut self, r: &mut Reader, key: &str) -> Result<Slot> {
        let class_id = self.register(key);
        let (tracking, version) = self.preamble(r, key)?;
        Ok(match self.track(r, tracking)? {
            Err(oid) => Slot::Ref(oid),
            Ok(oid) => Slot::New { version, oid, class_id, class: None },
        })
    }

    /// A pointer whose declared class is `key`.
    ///
    /// Like boost's `load_pointer_type`, the declared class is registered first
    /// unless it is abstract. A class id that is still new after that belongs to a
    /// derived class, and is followed by that class's export name.
    /// `resolve` maps an export name read from the archive to the class key used
    /// elsewhere (the C++ type name), so a class is registered under one key no
    /// matter how it is first met.
    pub fn pointer(&mut self, r: &mut Reader, key: &str, abstract_: bool, resolve: &dyn Fn(&str) -> String) -> Result<Slot> {
        if !abstract_ {
            self.register(key);
        }
        let at = r.base + r.pos;
        let cid = r.int()?;
        if cid == -1 {
            return Ok(Slot::Null);
        }
        let known = self.by_id.len() as i64;
        if cid > known || cid < 0 {
            return Err(Error { offset: at, what: "class id out of sequence" });
        }
        let mut class = None;
        let class_key = if cid == known {
            let name = r.string()?;
            let k = resolve(&name);
            class = Some(name);
            if self.register(&k) != cid {
                return Err(Error { offset: at, what: "exported class already registered under another id" });
            }
            k
        } else {
            self.by_id[cid as usize].clone()
        };
        let (tracking, version) = self.preamble(r, &class_key)?;
        Ok(match self.track(r, tracking)? {
            Err(oid) => Slot::Ref(oid),
            Ok(oid) => Slot::New { version, oid, class_id: cid, class },
        })
    }
}

/// Writer for the same encoding; integers are written with the fewest magnitude
/// bytes, as boost's `portable_binary_oarchive` does.
#[derive(Debug, Default, Clone)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    pub fn int(&mut self, v: i64) {
        if v == 0 {
            self.buf.push(0);
            return;
        }
        let m = v.unsigned_abs();
        let n = 8 - m.leading_zeros() as usize / 8;
        self.buf.push(if v < 0 { (n as i8).wrapping_neg() as u8 } else { n as u8 });
        self.buf.extend_from_slice(&m.to_le_bytes()[..n]);
    }

    pub fn f32(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn f64(&mut self, v: f64) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn string(&mut self, s: &str) {
        self.int(s.len() as i64);
        self.bytes(s.as_bytes());
    }

    pub fn header(&mut self, h: Header) {
        self.string(SIGNATURE);
        self.int(h.library_version as i64);
        self.u8(h.flags);
    }
}

#[derive(Debug, Clone, Copy)]
struct WClass {
    id: i64,
    /// The version written in the class preamble, once written.
    preamble: Option<u32>,
}

/// The writing side of [`Session`]: assigns class ids in first-use order, writes
/// each class's preamble once, and numbers tracked objects afresh. Object ids from
/// the tree being written ("old" ids) are mapped to the new ones, so back-references
/// stay correct after edits.
#[derive(Debug, Default)]
pub struct WSession {
    classes: HashMap<String, WClass>,
    count: i64,
    next_oid: u64,
    /// old object id -> (new id, class key)
    oids: HashMap<u64, (u64, String)>,
}

impl WSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, key: &str) -> i64 {
        if let Some(c) = self.classes.get(key) {
            return c.id;
        }
        let id = self.count;
        self.count += 1;
        self.classes.insert(key.to_string(), WClass { id, preamble: None });
        id
    }

    /// Whether the object with this old id was written.
    pub fn written(&self, old: u64) -> bool {
        self.oids.contains_key(&old)
    }

    pub fn is_registered(&self, key: &str) -> bool {
        self.classes.contains_key(key)
    }

    /// A class's version is written once, for the whole archive: every object of
    /// the class must have it (readers load all of them with that version).
    fn preamble(&mut self, w: &mut Writer, key: &str, tracking: bool, version: u32) -> Result<()> {
        let c = self.classes.get_mut(key).expect("registered");
        match c.preamble {
            None => {
                c.preamble = Some(version);
                w.int(tracking as i64);
                w.int(version as i64);
                Ok(())
            }
            Some(v) if v == version => Ok(()),
            Some(_) => Err(Error { offset: w.len(), what: "objects of one class with different versions in one archive" }),
        }
    }

    /// Writes the new id of a tracked object; returns it.
    fn track(&mut self, w: &mut Writer, key: &str, old: Option<u64>) -> Option<u64> {
        let old = old?;
        let new = self.next_oid;
        self.next_oid += 1;
        self.oids.insert(old, (new, key.to_string()));
        w.int(new as i64);
        Some(new)
    }

    /// The new id for a back-reference to `old`.
    fn reference(&self, w: &mut Writer, old: u64) -> Result<(u64, &str)> {
        let (new, key) = self.oids.get(&old).ok_or(Error { offset: w.len(), what: "back-reference to an object not written" })?;
        w.int(*new as i64);
        Ok((*new, key))
    }

    /// A new non-pointer object of class `key`; `old` is its id in the tree (if tracked).
    pub fn object_new(&mut self, w: &mut Writer, key: &str, version: u32, old: Option<u64>) -> Result<Option<u64>> {
        self.register(key);
        self.preamble(w, key, old.is_some(), version)?;
        Ok(self.track(w, key, old))
    }

    pub fn object_ref(&mut self, w: &mut Writer, key: &str, old: u64) -> Result<u64> {
        self.register(key);
        if self.classes[key].preamble.is_none() {
            return Err(Error { offset: w.len(), what: "back-reference before the class was written" });
        }
        Ok(self.reference(w, old)?.0)
    }

    pub fn pointer_null(&mut self, w: &mut Writer, declared: &str, abstract_: bool) {
        if !abstract_ {
            self.register(declared);
        }
        w.int(-1);
    }

    /// A new object through a pointer declared as `declared`; `export` is written if
    /// the class is new to the archive.
    #[allow(clippy::too_many_arguments)]
    pub fn pointer_new(
        &mut self,
        w: &mut Writer,
        declared: &str,
        abstract_: bool,
        key: &str,
        export: &str,
        version: u32,
        old: Option<u64>,
    ) -> Result<Option<u64>> {
        if !abstract_ {
            self.register(declared);
        }
        match self.classes.get(key) {
            Some(c) => w.int(c.id),
            None => {
                let id = self.register(key);
                w.int(id);
                w.string(export);
            }
        }
        self.preamble(w, key, old.is_some(), version)?;
        Ok(self.track(w, key, old))
    }

    pub fn pointer_ref(&mut self, w: &mut Writer, declared: &str, abstract_: bool, old: u64) -> Result<u64> {
        if !abstract_ {
            self.register(declared);
        }
        let (_, key) = self.oids.get(&old).ok_or(Error { offset: w.len(), what: "back-reference to an object not written" })?;
        let id = self.classes[key.as_str()].id;
        w.int(id);
        Ok(self.reference(w, old)?.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ints_round_trip() {
        for v in [0i64, 1, -1, 22, 255, 256, -256, 0x1234, i64::MAX, i64::MIN, -0x7fff_ffff] {
            let mut w = Writer::new();
            w.int(v);
            let mut r = Reader::new(&w.buf, 0);
            assert_eq!(r.int().unwrap(), v);
            assert_eq!(r.remaining(), 0);
        }
        let mut w = Writer::new();
        w.int(0x1234);
        w.int(-256);
        assert_eq!(w.buf, [0x02, 0x34, 0x12, 0xfe, 0x00, 0x01]);
    }

    #[test]
    fn ints() {
        let mut r = Reader::new(&[0x00, 0x01, 0x16, 0xff, 0x01, 0x02, 0x34, 0x12, 0xfe, 0x00, 0x01], 0);
        assert_eq!(r.int().unwrap(), 0);
        assert_eq!(r.int().unwrap(), 22);
        assert_eq!(r.int().unwrap(), -1);
        assert_eq!(r.int().unwrap(), 0x1234);
        assert_eq!(r.int().unwrap(), -256);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn header() {
        let mut v = vec![0x01, 22];
        v.extend_from_slice(SIGNATURE.as_bytes());
        v.extend_from_slice(&[0x01, 0x11, 0x00]);
        assert_eq!(Reader::new(&v, 0).header().unwrap(), Header { library_version: 17, flags: 0 });
    }

    #[test]
    fn truncated_is_error_not_panic() {
        assert!(Reader::new(&[0x08, 1, 2], 0).int().is_err());
        assert!(Reader::new(&[0x01, 0xff], 0).string().is_err());
    }
}
