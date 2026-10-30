//! NI "hsin" item container.
//!
//! An item is
//!
//! ```text
//! u64 size            (whole item, including this field)
//! u32 version         (1)
//! "hsin"
//! u32, u32            (unknown; 1, 0 in every file seen)
//! [u8; 16] uuid
//! frame               (outermost DSIN frame, see below)
//! u32 version, u32 n  (child list)
//! n × { u32 tag, [u8;4] domain (LE fourcc), u32 item_id, item }
//! ```
//!
//! A frame is `u64 size, "DSIN", u32 id, u32 version`, then (unless `id == 1`, the
//! terminator) a nested frame for the base type, then the frame's own payload.
//! Frames therefore form a chain from the most-derived type down to id 1.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub what: &'static str,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at offset {:#x}", self.what, self.offset)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Frame<'a> {
    pub offset: usize,
    pub id: u32,
    pub version: u32,
    /// This frame's own payload (after the nested base frame).
    pub data: &'a [u8],
}

#[derive(Debug, Clone)]
pub struct Child<'a> {
    pub tag: u32,
    /// Four-character domain, e.g. `*b"NISD"`, `*b"MAS2"`.
    pub domain: [u8; 4],
    pub item_id: u32,
    pub item: Item<'a>,
}

#[derive(Debug, Clone)]
pub struct Item<'a> {
    pub offset: usize,
    /// Bytes 8..24 as read: version, `hsin`, and two unknown u32s.
    pub head: [u8; 16],
    pub uuid: [u8; 16],
    pub list_version: u32,
    /// Most-derived first; the terminator frame (id 1) is last.
    pub frames: Vec<Frame<'a>>,
    pub children: Vec<Child<'a>>,
}

impl<'a> Item<'a> {
    pub fn parse(buf: &'a [u8]) -> Result<Item<'a>> {
        let (item, end) = parse_item(buf, 0, 0)?;
        if end != buf.len() {
            return Err(Error { offset: end, what: "trailing bytes after root item" });
        }
        Ok(item)
    }

    /// The most-derived frame id, which identifies the item's type.
    pub fn type_id(&self) -> u32 {
        self.frames.first().map_or(0, |f| f.id)
    }

    pub fn frame(&self, id: u32) -> Option<&Frame<'a>> {
        self.frames.iter().find(|f| f.id == id)
    }

    /// The item as bytes, with each frame's own payload replaced by `payload(frame)`
    /// where that returns `Some`. Sizes are recomputed.
    pub fn to_bytes(&self, payload: &mut dyn FnMut(&Frame<'a>) -> Option<Vec<u8>>) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&self.head);
        out.extend_from_slice(&self.uuid);
        out.extend_from_slice(&frames_to_bytes(&self.frames, payload));
        out.extend_from_slice(&self.list_version.to_le_bytes());
        out.extend_from_slice(&(self.children.len() as u32).to_le_bytes());
        for c in &self.children {
            out.extend_from_slice(&c.tag.to_le_bytes());
            out.extend_from_slice(&u32::from_be_bytes(c.domain).to_le_bytes());
            out.extend_from_slice(&c.item_id.to_le_bytes());
            out.extend_from_slice(&c.item.to_bytes(payload));
        }
        let size = out.len() as u64;
        out[..8].copy_from_slice(&size.to_le_bytes());
        out
    }

    /// Depth-first search for the first item (including `self`) with a frame `id`.
    pub fn find_frame(&self, id: u32) -> Option<&Frame<'a>> {
        self.frame(id).or_else(|| self.children.iter().find_map(|c| c.item.find_frame(id)))
    }
}

/// A frame chain (most-derived first) as bytes.
fn frames_to_bytes<'a>(frames: &[Frame<'a>], payload: &mut dyn FnMut(&Frame<'a>) -> Option<Vec<u8>>) -> Vec<u8> {
    let Some(f) = frames.first() else { return Vec::new() };
    let mut body = if f.id == 1 { Vec::new() } else { frames_to_bytes(&frames[1..], payload) };
    match payload(f) {
        Some(p) => body.extend_from_slice(&p),
        None => body.extend_from_slice(f.data),
    }
    let mut out = Vec::with_capacity(body.len() + 20);
    out.extend_from_slice(&(body.len() as u64 + 20).to_le_bytes());
    out.extend_from_slice(b"DSIN");
    out.extend_from_slice(&f.id.to_le_bytes());
    out.extend_from_slice(&f.version.to_le_bytes());
    out.extend_from_slice(&body);
    out
}

const MAX_DEPTH: usize = 64;

fn u32_at(buf: &[u8], o: usize) -> Result<u32> {
    buf.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or(Error { offset: o, what: "truncated u32" })
}

fn u64_at(buf: &[u8], o: usize) -> Result<u64> {
    buf.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).ok_or(Error { offset: o, what: "truncated u64" })
}

/// Returns `start + size`, checked against `limit`.
fn span_end(start: usize, size: u64, min: usize, limit: usize, what: &'static str) -> Result<usize> {
    let end = usize::try_from(size).ok().and_then(|s| start.checked_add(s));
    match end {
        Some(e) if size as usize >= min && e <= limit => Ok(e),
        _ => Err(Error { offset: start, what }),
    }
}

fn parse_item(buf: &[u8], o: usize, depth: usize) -> Result<(Item<'_>, usize)> {
    if depth > MAX_DEPTH {
        return Err(Error { offset: o, what: "items nested too deeply" });
    }
    let end = span_end(o, u64_at(buf, o)?, 40, buf.len(), "bad item size")?;
    if buf.get(o + 12..o + 16) != Some(b"hsin") {
        return Err(Error { offset: o + 12, what: "missing hsin magic" });
    }
    let head: [u8; 16] = buf[o + 8..o + 24].try_into().unwrap();
    let uuid: [u8; 16] = buf[o + 24..o + 40].try_into().unwrap();
    let mut frames = Vec::new();
    let mut p = parse_frame(buf, o + 40, end, &mut frames, 0)?;

    let list_version = u32_at(buf, p)?;
    let n = u32_at(buf, p + 4)?;
    p += 8;
    let mut children = Vec::new();
    for _ in 0..n {
        let tag = u32_at(buf, p)?;
        // Stored as a little-endian u32, so the bytes on disk are the fourcc reversed.
        let domain = u32_at(buf, p + 4)?.to_be_bytes();
        let item_id = u32_at(buf, p + 8)?;
        let (item, next) = parse_item(&buf[..end], p + 12, depth + 1)?;
        children.push(Child { tag, domain, item_id, item });
        p = next;
    }
    if p != end {
        return Err(Error { offset: p, what: "item size does not match contents" });
    }
    Ok((Item { offset: o, head, uuid, list_version, frames, children }, end))
}

/// Appends the frame chain starting at `o` to `out`; returns the end of the outer frame.
fn parse_frame<'a>(buf: &'a [u8], o: usize, limit: usize, out: &mut Vec<Frame<'a>>, depth: usize) -> Result<usize> {
    if depth > MAX_DEPTH {
        return Err(Error { offset: o, what: "frames nested too deeply" });
    }
    let end = span_end(o, u64_at(buf, o)?, 20, limit, "bad frame size")?;
    if buf.get(o + 8..o + 12) != Some(b"DSIN") {
        return Err(Error { offset: o + 8, what: "missing DSIN magic" });
    }
    let id = u32_at(buf, o + 12)?;
    let version = u32_at(buf, o + 16)?;
    let idx = out.len();
    out.push(Frame { offset: o, id, version, data: &[] });
    let own_start = if id == 1 { o + 20 } else { parse_frame(buf, o + 20, end, out, depth + 1)? };
    out[idx].data = &buf[own_start..end];
    Ok(end)
}

// ---- writing a new file ------------------------------------------------------------

/// Builds the payloads of NI's item streams (little-endian; strings are a `u32`
/// character count and UTF-16LE).
#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn wstr(&mut self, s: &str) -> &mut Self {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.u32(units.len() as u32);
        for u in units {
            self.0.extend_from_slice(&u.to_le_bytes());
        }
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.0)
    }
}

/// A frame chain, most-derived first, ending with the terminator (id 1); each entry is
/// a frame id and its own payload. Every frame is version 1.
fn frame_chain(chain: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let Some(((id, own), rest)) = chain.split_first() else { return Vec::new() };
    let mut body = if *id == 1 { Vec::new() } else { frame_chain(rest) };
    body.extend_from_slice(own);
    let mut out = Out::default();
    out.u64(body.len() as u64 + 20).bytes(b"DSIN").u32(*id).u32(1).bytes(&body);
    out.take()
}

/// The terminator frame's payload.
fn terminator() -> (u32, Vec<u8>) {
    (1, 1u32.to_le_bytes().to_vec())
}

/// An item: its id, frames (most-derived first, without the terminator) and children
/// (`tag`, domain, item id, item bytes).
fn item_bytes(uuid: [u8; 16], frames: Vec<(u32, Vec<u8>)>, children: &[(u32, [u8; 4], u32, Vec<u8>)]) -> Vec<u8> {
    let mut chain = frames;
    chain.push(terminator());
    let mut body = Out::default();
    body.u32(1).bytes(b"hsin").u32(1).u32(0).bytes(&uuid).bytes(&frame_chain(&chain)).u32(1).u32(children.len() as u32);
    for (tag, domain, id, item) in children {
        body.u32(*tag).u32(u32::from_be_bytes(*domain)).u32(*id).bytes(item);
    }
    let body = body.take();
    let mut out = Out::default();
    out.u64(body.len() as u64 + 8).bytes(&body);
    out.take()
}

// ---- items of our own at the root ----------------------------------------------------

/// The root's children: where its count is, and each child's span (header included)
/// with its domain and item id.
fn root_children(file: &[u8]) -> Result<(usize, Vec<(usize, usize, [u8; 4], u32)>)> {
    let end = span_end(0, u64_at(file, 0)?, 40, file.len(), "bad item size")?;
    let mut p = span_end(40, u64_at(file, 40)?, 20, end, "bad frame size")?;
    let count_at = p + 4;
    let n = u32_at(file, count_at)?;
    p += 8;
    let mut out = Vec::new();
    for _ in 0..n {
        let domain = u32_at(file, p + 4)?.to_be_bytes();
        let item_id = u32_at(file, p + 8)?;
        let next = span_end(p + 12, u64_at(file, p + 12)?, 40, end, "bad item size")?;
        out.push((p, next, domain, item_id));
        p = next;
    }
    Ok((count_at, out))
}

/// The payload of a root child's own frame (the DECKARD block, …), if the file has one
/// with that domain and item id.
pub fn root_child(file: &[u8], domain: [u8; 4], item: [u8; 4]) -> Option<Vec<u8>> {
    let (_, children) = root_children(file).ok()?;
    let &(start, end, ..) = children.iter().find(|c| c.2 == domain && c.3 == u32::from_be_bytes(item))?;
    let child = Item::parse(&file[start + 12..end]).ok()?;
    child.frames.first().filter(|f| f.id != 1).map(|f| f.data.to_vec())
}

/// The file with its root child of `domain` and `item` replaced by one holding
/// `payload` in a frame `frame_id` (appended as the root's last child), or removed if
/// `payload` is `None`. Maschine ignores a root child it doesn't know.
pub fn set_root_child(
    file: &[u8],
    domain: [u8; 4],
    item: [u8; 4],
    frame_id: u32,
    payload: Option<&[u8]>,
) -> std::result::Result<Vec<u8>, String> {
    let (count_at, children) = root_children(file).map_err(|e| e.to_string())?;
    let mut root_end = u64_at(file, 0).map_err(|e| e.to_string())? as usize;
    let mut out = file.to_vec();
    let mut n = children.len() as u32;
    for &(start, end, ..) in children.iter().rev().filter(|c| c.2 == domain && c.3 == u32::from_be_bytes(item)) {
        out.drain(start..end);
        root_end -= end - start;
        n -= 1;
    }
    if let Some(payload) = payload {
        let child = item_bytes(new_uuid(payload), vec![(frame_id, payload.to_vec())], &[]);
        let mut entry = Out::default();
        entry.u32(0).u32(u32::from_be_bytes(domain)).u32(u32::from_be_bytes(item)).bytes(&child);
        let entry = entry.take();
        out.splice(root_end..root_end, entry.iter().copied());
        root_end += entry.len();
        n += 1;
    }
    out[0..8].copy_from_slice(&(root_end as u64).to_le_bytes());
    out[count_at..count_at + 4].copy_from_slice(&n.to_le_bytes());
    Ok(out)
}

/// An item id that differs from save to save (Maschine's are random).
fn new_uuid(seed: &[u8]) -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let mut id = [0; 16];
    for half in id.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write(seed);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        half.copy_from_slice(&h.finish().to_le_bytes());
    }
    id
}

/// A constant that ends the root's item info and follows the archive.
const MARKER: u32 = 0x8565_620d;

/// The NISound format version Maschine 3 writes (1.7.14), packed and as text.
const NISOUND_VERSION: (u64, &str) = ((1 << 20) | (7 << 12) | 14, "1.7.14");

/// An NI sound-info stream (`NI::SOUND::SoundInfo2`) with nothing filled in
/// ([`crate::sound_info::SoundInfo`]'s default). Maschine stores one per project,
/// group and sound, and one (with a version before it) in every file's `NISD/108`.
pub fn new_sound_info() -> Vec<u8> {
    crate::sound_info::SoundInfo::default().to_bytes()
}

/// A complete project file (`.mxprj`) holding `archive` (a boost archive, header
/// included, of a `NI::MASCHINE::DATA::Project`), laid out as Maschine 3 writes one:
///
/// ```text
/// root                 frames 118 (file info) > 106 (item info) > 1
///   MAS2/PROJ          frames 101 (app version) > 106 > 1
///     NISD/108         sound info: authoring fields empty, standard properties
///     NISD/121
///     NISD/116         frames 116 > 115 (an empty embedded item) > 1
///       NISD/109       the archive
/// ```
///
/// `app_version` is the Maschine version the file says wrote it; `uuid` gives each
/// item a new id.
pub fn new_project_file(archive: &[u8], app_version: &str, uuid: &mut dyn FnMut() -> [u8; 16]) -> Vec<u8> {
    new_file(*b"PROJ", archive, app_version, &crate::sound_info::SoundInfo::default(), uuid)
}

/// A Maschine file of any kind — `PROJ` (`.mxprj`), `GRP ` (`.mxgrp`), `SND `
/// (`.mxsnd`), `SLOT` (`.mxinst`, `.mxfx`) — laid out as [`new_project_file`]'s (all
/// four are, in Maschine's own files), with `info` as its tag block.
pub fn new_file(
    kind: [u8; 4],
    archive: &[u8],
    app_version: &str,
    info: &crate::sound_info::SoundInfo,
    uuid: &mut dyn FnMut() -> [u8; 16],
) -> Vec<u8> {
    let mut o = Out::default();
    // NISD/109: the archive, as a stream string after a small header, then a marker.
    o.u32(1).u32(1).u32(1).u64(archive.len() as u64 + 4).u32(archive.len() as u32).bytes(archive);
    o.u32(0).u32(MARKER);
    let doc = item_bytes(uuid(), vec![(109, o.take())], &[]);
    // NISD/116 › 115: an empty item embedded in the frame.
    let empty = item_bytes(uuid(), Vec::new(), &[]);
    let f115 = o.u32(1).u8(0).bytes(&empty).take();
    let f116 = o.u32(1).u8(0).take();
    let holder = item_bytes(uuid(), vec![(116, f116), (115, f115)], &[(0, *b"NISD", 109, doc)]);
    let f121 = o.u32(1).u32(1).u32(0).take();
    let n121 = item_bytes(uuid(), vec![(121, f121)], &[]);
    // NISD/108: sound info.
    o.u32(1).bytes(&info.to_bytes());
    let n108 = item_bytes(uuid(), vec![(108, o.take())], &[]);
    // MAS2/<kind>: the app version.
    let f101 = o.u32(1).u8(0).u32(5).u32(1).wstr(app_version).take();
    let f106 = o.u32(1).u32(2).take();
    let proj = item_bytes(
        uuid(),
        vec![(101, f101), (106, f106)],
        &[(0, *b"NISD", 108, n108), (2, *b"NISD", 121, n121), (1, *b"NISD", 116, holder)],
    );
    // Root: file info (format version, "000"), item info.
    let f118 = o.u32(1).u64(NISOUND_VERSION.0).u32(1).u32(1).u32(0).u32(0).u32(0).wstr("000").u64(0).u64(0).take();
    let f106 = o.u32(1).u32(1).u32(1).u32(0).u32(0).u32(0).u32(MARKER).take();
    item_bytes(uuid(), vec![(118, f118), (106, f106)], &[(0, *b"MAS2", u32::from_be_bytes(kind), proj)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_project_file_parses() {
        let mut n = 0u8;
        let mut uuid = || {
            n += 1;
            [n; 16]
        };
        let archive = b"\x01\x16serialization::archive\x01\x14\x00";
        let file = new_project_file(archive, "3.7.1.0", &mut uuid);
        let root = Item::parse(&file).unwrap();
        assert_eq!(root.type_id(), 118);
        let p = crate::Project::parse(&file).unwrap();
        assert_eq!(p.kind(), Some(*b"PROJ"));
        assert_eq!(p.archive, archive);
        assert_eq!(p.header.library_version, 20);
        // Rewriting it changes nothing.
        assert_eq!(root.to_bytes(&mut |_| None), file);
    }

    /// A sound, group or preset file: the same layout with its kind and tag block.
    #[test]
    fn new_file_has_its_kind_and_tags() {
        let mut n = 0u8;
        let mut uuid = || {
            n += 1;
            [n; 16]
        };
        let archive = b"\x01\x16serialization::archive\x01\x0a\x00";
        let mut info = crate::sound_info::SoundInfo::default();
        info.name = "Kit".into();
        info.set_tags(&[vec!["Kits".into()]], &[]);
        let file = new_file(*b"GRP ", archive, "3.0.0", &info, &mut uuid);
        let p = crate::Project::parse(&file).unwrap();
        assert_eq!(p.kind(), Some(*b"GRP "));
        assert_eq!(p.sound_info(), Some(info));
    }

    fn project() -> Vec<u8> {
        let mut n = 0u8;
        let mut uuid = || {
            n += 1;
            [n; 16]
        };
        new_project_file(b"\x01\x16serialization::archive\x01\x14\x00", "3.7.1.0", &mut uuid)
    }

    const DKRD: u32 = u32::from_be_bytes(*b"DKRD");

    #[test]
    fn set_root_child_adds_one_child() {
        let file = set_root_child(&project(), *b"DKRD", *b"EXPR", DKRD, Some(b"hello")).unwrap();
        let root = Item::parse(&file).unwrap();
        assert_eq!(root.children.len(), 2);
        let last = root.children.last().unwrap();
        assert_eq!((last.domain, last.item_id), (*b"DKRD", u32::from_be_bytes(*b"EXPR")));
        assert_eq!(root_child(&file, *b"DKRD", *b"EXPR").as_deref(), Some(&b"hello"[..]));
    }

    #[test]
    fn set_root_child_replaces() {
        let one = set_root_child(&project(), *b"DKRD", *b"EXPR", DKRD, Some(b"one")).unwrap();
        let two = set_root_child(&one, *b"DKRD", *b"EXPR", DKRD, Some(b"two")).unwrap();
        assert_eq!(Item::parse(&two).unwrap().children.len(), 2);
        assert_eq!(root_child(&two, *b"DKRD", *b"EXPR").as_deref(), Some(&b"two"[..]));
    }

    #[test]
    fn set_root_child_none_restores_the_file() {
        let file = project();
        let with = set_root_child(&file, *b"DKRD", *b"EXPR", DKRD, Some(b"hello")).unwrap();
        assert_eq!(set_root_child(&with, *b"DKRD", *b"EXPR", DKRD, None).unwrap(), file);
    }

    #[test]
    fn set_root_child_none_on_a_plain_file_is_identity() {
        let file = project();
        assert_eq!(set_root_child(&file, *b"DKRD", *b"EXPR", DKRD, None).unwrap(), file);
        assert_eq!(root_child(&file, *b"DKRD", *b"EXPR"), None);
    }

    #[test]
    fn set_root_child_project_still_reads() {
        let file = project();
        let with = set_root_child(&file, *b"DKRD", *b"EXPR", DKRD, Some(b"hello")).unwrap();
        let (a, b) = (crate::Project::parse(&file).unwrap(), crate::Project::parse(&with).unwrap());
        assert_eq!((a.kind(), a.archive), (b.kind(), b.archive));
        // Rewriting it (as a save does) keeps the child.
        assert_eq!(b.root.to_bytes(&mut |_| None), with);
    }

    /// Against a file Maschine 3 wrote (its plug-in's default state, which
    /// `maschine-ref state` writes): the same bytes, given its archive and item ids.
    #[test]
    fn new_project_file_matches_maschine() {
        let Ok(path) = std::env::var("MXPRJ_MASCHINE3_STATE") else { return };
        let theirs = std::fs::read(path).unwrap();
        let p = crate::Project::parse(&theirs).unwrap();
        // Item ids in the order new_project_file asks for them.
        let child = |i: &Item, k: usize| i.children[k].item.uuid;
        let proj = &p.root.children[0].item;
        let holder = &proj.children[2].item;
        let f115 = p.root.find_frame(115).unwrap();
        let mut embedded = [0; 16];
        embedded.copy_from_slice(&f115.data[5 + 24..5 + 40]);
        let ids = vec![child(holder, 0), embedded, holder.uuid, child(proj, 1), child(proj, 0), proj.uuid, p.root.uuid];
        let mut it = ids.into_iter();
        let ours = new_project_file(p.archive, "3.7.1.0", &mut || it.next().unwrap());
        let at = ours.iter().zip(&theirs).position(|(a, b)| a != b);
        assert!(at.is_none() && ours.len() == theirs.len(), "differ at {at:?} (lengths {} {})", ours.len(), theirs.len());
    }
}
