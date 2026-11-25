//! Kontakt library instruments (`.nki`) as a Kontakt VST3 state.
//!
//! An `.nki` is not a Kontakt plug-in state, and given its bytes as one Kontakt stops in
//! `setState` on a modal "One or more Kontakt instances cannot be recalled correctly,
//! perhaps due to missing content" box, which blocks the calling thread until OK is
//! pressed. Two things make it so (measured 2026-11-10 on Kontakt 7.10.9):
//!
//! - A plug-in state is a *multi*: the instrument's program sits in a slot of the multi's
//!   bank, inside a slot container, where an `.nki` holds it at the top of its program
//!   item. An instrument item where the multi belongs is refused, even Kontakt's own
//!   default instrument.
//! - An `.nki` names its samples relative to where the file is, and a state has no
//!   location, so they resolve to nothing: one missing file gives the same box. Every
//!   state Maschine or Kontakt saves names them by absolute path.
//!
//! [`instrument_state`] builds what Kontakt itself saves: its empty multi (what the
//! installed Kontakt saves when made with no state, [`empty_multi`]), with one slot holding the
//! instrument's program and settings, and the instrument's file table with every relative
//! path made absolute from the `.nki`'s folder. It checks every file the table names is
//! there first, since a missing one would bring the box back.
//!
//! Encrypted (Kontakt Player) library instruments can't be built that way: their whole
//! program item, file table included, is one encrypted block, and a state Kontakt saves
//! for one is the whole multi re-encrypted, not the `.nki`'s block. DECKARD never looks
//! inside it. [`state_opened_by_kontakt`] lets Kontakt open the file itself instead, with
//! the load-by-path command Maschine sends it, and takes the state Kontakt saves
//! (measured 2026-11-10 with Electric Mint, Prime Bass, Cuba and their snapshots: no
//! dialog, they play, and the state recalls like a Maschine project's).
//!
//! Snapshots (`.nksn`), parameter sets for an instrument Kontakt loads and applies
//! itself, go the same way. Refused with a reason before any Kontakt is made: files a
//! newer Kontakt saved ([`check_version`]), snapshots whose instrument path isn't there
//! ([`check_snapshot`]), and, for the states DECKARD builds, file tables other than
//! version 2 (Kontakt 5 and 6; Kontakt 7 writes version 3).
//!
//! The NI container format: an item is `[u64 size][u32 1]["hsin"][u32][u32][16-byte id]`,
//! a chain of data frames `[u64 size][domain][u32 id][u32 version][nested frame][own
//! data]` ending in frame `DSIN` 1, then `[u32][u32 count]` children, each `[u32][domain]
//! [u32 id]` and an item. Frame `DSIN` 115 holds a subtree, raw (`[u32][0][item]`) or
//! FastLZ-compressed (`[u32][1][u32 size][u32 packed size][data]`); frame `DSIN` 109 holds
//! the program's Kontakt chunks, `[u16 id][u32 size][data]` each.

use std::path::{Component, Path, PathBuf, Prefix};

/// Why an instrument isn't built when the installed Kontakt's empty state isn't the multi
/// DECKARD knows how to fill (a Kontakt that saves another layout): Kontakt opens it itself.
pub const UNKNOWN_MULTI: &str = "saved by a Kontakt whose empty state DECKARD doesn't know how to fill.";

/// The installed Kontakt's empty multi ("New (default)", no instrument): what it saves
/// (`getState`) when made with no state, asked once per Kontakt (`plugin_id`, `name`) and
/// kept. Making it takes a second or so; it must be on the main thread, where plug-ins are
/// made.
pub fn empty_multi(plugin_id: &str, name: &str) -> Result<Vec<u8>, String> {
    static MADE: std::sync::Mutex<Vec<(String, Vec<u8>)>> = std::sync::Mutex::new(Vec::new());
    if let Some((_, state)) = MADE.lock().unwrap().iter().find(|(id, _)| id == plugin_id) {
        return Ok(state.clone());
    }
    if !crate::on_main_thread() {
        return Err("Kontakt can only be made on the main thread".into());
    }
    let mut host = crate::Host::default();
    let plugin = host.instance(crate::Slot::Sound { group: 0, sound: 0 }, plugin_id, name, &[], 48_000.0)?;
    let state = plugin.lock().map_err(|_| "Kontakt is unavailable")?.state().component;
    drop(plugin);
    drop(host);
    check_multi(&state)?;
    MADE.lock().unwrap().push((plugin_id.to_string(), state.clone()));
    Ok(state)
}

/// Whether `state` is the multi [`instrument_state`] fills: its subtree holds the multi,
/// whose own subtree holds the program item with the bank (0x3), with its slot list, and
/// the file table (0x4b).
fn check_multi(state: &[u8]) -> Result<(), String> {
    let unknown = |_| UNKNOWN_MULTI.to_string();
    let multi = subtree(state).map_err(unknown)?.data;
    let bank_item = subtree(&multi).map_err(unknown)?.data;
    let (_, parts) = chunks(&bank_item).map_err(unknown)?;
    let bank = parts.iter().find(|(id, _)| *id == 0x3).ok_or(UNKNOWN_MULTI)?;
    parts.iter().find(|(id, _)| *id == 0x4b).ok_or(UNKNOWN_MULTI)?;
    bank_with_slot(&bank.1, &chunk(0x37, &[0; 8])).map_err(unknown)?;
    Ok(())
}

/// A multi slot's container (chunk `0x29`) up to its program chunk (`0x36`), as Kontakt 7
/// writes it for slot 0 with default settings. [`SLOT_LEN_AT`] holds the length of the rest
/// of the container; at [`SLOT_SETTINGS_AT`] is the instrument's settings chunk (`0x47`).
const SLOT_PREFIX: [u8; 97] = [
    0x01, 0x51, 0x00, 0x12, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x02, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x2b, 0x00, 0x16, 0x00, 0x00, 0x00, 0x00, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x40, 0x00, 0x00, 0x00, 0x0a,
    0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x47, 0x00, 0x12, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff,
    0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
];
const SLOT_LEN_AT: usize = 41;
const SLOT_SETTINGS_AT: usize = 73;

/// Why an encrypted library instrument isn't loaded.
pub const ENCRYPTED: &str =
    "an encrypted library instrument, which only Kontakt's own browser can open. Load it there, or load a Maschine sound saved with it.";

/// Why an instrument naming its files inside a registered library isn't built into a
/// state: only Kontakt knows where the library is ([`state_opened_by_kontakt`] lets it).
pub const LIBRARY: &str = "names its files inside a registered library, which only Kontakt can open.";

/// Why an instrument whose file list isn't version 2 (Kontakt 7 writes 3) isn't built
/// into a state: only Kontakt reads it ([`state_opened_by_kontakt`] lets it).
pub const NEW_TABLE: &str = "saved by a Kontakt whose file list only Kontakt itself can read (Kontakt 5 and 6 write version 2).";

/// Why a Kontakt snapshot isn't loaded.
pub const SNAPSHOT: &str = "Kontakt snapshots (.nksn) can't be loaded from DECKARD's browser: Kontakt applies them to their instrument itself. Load the instrument (.nki), or load it in Kontakt's own browser.";

const PROGRAM_ITEM: u32 = 109;
const SUBTREE: u32 = 115;
const DSIN: [u8; 4] = *b"DSIN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Node {
    Item,
    Frame([u8; 4], u32),
}

/// An item or frame: where its `u64` size is, and the bytes it spans.
#[derive(Debug, Clone, Copy)]
struct Span {
    at: usize,
    end: usize,
    node: Node,
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(b.get(at..at + 4).ok_or("Kontakt file is truncated")?.try_into().unwrap()))
}
fn u64_at(b: &[u8], at: usize) -> Result<usize, String> {
    let v = u64::from_le_bytes(b.get(at..at + 8).ok_or("Kontakt file is truncated")?.try_into().unwrap());
    usize::try_from(v).map_err(|_| "Kontakt file is damaged".to_string())
}
fn damaged() -> String {
    "Kontakt file is damaged or not a Kontakt 5+ file".into()
}

/// Every item and frame of the item at `at`, in file order.
fn spans(b: &[u8]) -> Result<Vec<Span>, String> {
    let mut out = Vec::new();
    let end = item(b, 0, b.len(), &mut out, 0)?;
    if end != b.len() {
        return Err(damaged());
    }
    Ok(out)
}

fn item(b: &[u8], at: usize, limit: usize, out: &mut Vec<Span>, depth: usize) -> Result<usize, String> {
    let end = at.checked_add(u64_at(b, at)?).filter(|&e| e <= limit && depth < 64).ok_or_else(damaged)?;
    if b.get(at + 12..at + 16) != Some(b"hsin") {
        return Err(damaged());
    }
    out.push(Span { at, end, node: Node::Item });
    let mut c = frame(b, at + 40, end, out, depth)?;
    if c + 8 <= end {
        let n = u32_at(b, c + 4)?;
        c += 8;
        for _ in 0..n {
            c = item(b, c + 12, end, out, depth + 1)?;
        }
    }
    Ok(end)
}

fn frame(b: &[u8], at: usize, limit: usize, out: &mut Vec<Span>, depth: usize) -> Result<usize, String> {
    let end = at.checked_add(u64_at(b, at)?).filter(|&e| e <= limit && depth < 64 && e >= at + 20).ok_or_else(damaged)?;
    let domain: [u8; 4] = b[at + 8..at + 12].try_into().unwrap();
    let id = u32_at(b, at + 12)?;
    out.push(Span { at, end, node: Node::Frame(domain, id) });
    if !(domain == DSIN && id == 1) {
        frame(b, at + 20, end, out, depth + 1)?;
    }
    Ok(end)
}

/// Where a frame's own data starts: after its header and the frame it nests.
fn own_data(b: &[u8], frame_at: usize) -> Result<usize, String> {
    Ok(frame_at + 20 + u64_at(b, frame_at + 20)?)
}

/// A subtree (frame `DSIN` 115): where its stored form is, and the subtree itself.
struct Subtree {
    /// Start of the stored form (the raw/compressed flag) and its end.
    at: usize,
    end: usize,
    data: Vec<u8>,
}

/// The first subtree in `b`.
fn subtree(b: &[u8]) -> Result<Subtree, String> {
    let f = spans(b)?.into_iter().find(|s| s.node == Node::Frame(DSIN, SUBTREE)).ok_or_else(damaged)?;
    let at = own_data(b, f.at)? + 4;
    match b.get(at) {
        Some(0) => {
            let n = u64_at(b, at + 1)?;
            let end = (at + 1).checked_add(n).filter(|&e| e <= f.end).ok_or_else(damaged)?;
            Ok(Subtree { at, end, data: b[at + 1..end].to_vec() })
        }
        Some(1) => {
            let size = u32_at(b, at + 1)? as usize;
            let packed = u32_at(b, at + 5)? as usize;
            let end = (at + 9).checked_add(packed).filter(|&e| e <= f.end).ok_or_else(damaged)?;
            let data = fastlz(&b[at + 9..end], size).ok_or("ENCRYPTED")?;
            Ok(Subtree { at, end, data })
        }
        _ => Err(damaged()),
    }
}

/// `b` with `b[start..end]` replaced, every item and frame around it resized.
fn splice(b: &[u8], start: usize, end: usize, new: &[u8]) -> Result<Vec<u8>, String> {
    let grow = new.len() as i64 - (end - start) as i64;
    let mut out = Vec::with_capacity(b.len() + new.len());
    out.extend_from_slice(&b[..start]);
    out.extend_from_slice(new);
    out.extend_from_slice(&b[end..]);
    for s in spans(b)? {
        if s.at < start && s.end >= end {
            let size = (u64_at(b, s.at)? as i64 + grow) as u64;
            out[s.at..s.at + 8].copy_from_slice(&size.to_le_bytes());
        }
    }
    Ok(out)
}

/// `b` with its first subtree stored raw as `data`.
fn put_subtree(b: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    let s = subtree(b)?;
    let mut stored = Vec::with_capacity(1 + data.len());
    stored.push(0);
    stored.extend_from_slice(data);
    splice(b, s.at, s.end, &stored)
}

/// A program item's Kontakt chunks (`[u16 id][u32 size][data]`, whole), and the offset of
/// their `u64` total length.
fn chunks(b: &[u8]) -> Result<(usize, Vec<(u16, Vec<u8>)>), String> {
    let f = spans(b)?.into_iter().find(|s| s.node == Node::Frame(DSIN, PROGRAM_ITEM)).ok_or_else(damaged)?;
    let total_at = own_data(b, f.at)? + 12;
    let end = (total_at + 8).checked_add(u64_at(b, total_at)?).filter(|&e| e <= f.end).ok_or_else(damaged)?;
    let mut out = Vec::new();
    let mut o = total_at + 8;
    while o < end {
        let id = u16::from_le_bytes(b.get(o..o + 2).ok_or_else(damaged)?.try_into().unwrap());
        let next = (o + 6).checked_add(u32_at(b, o + 2)? as usize).filter(|&e| e <= end).ok_or_else(damaged)?;
        out.push((id, b[o..next].to_vec()));
        o = next;
    }
    Ok((total_at, out))
}

/// `b` with its program item's chunks replaced.
fn put_chunks(b: &[u8], total_at: usize, chunks: &[(u16, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let body: Vec<u8> = chunks.iter().flat_map(|(_, c)| c.iter().copied()).collect();
    let end = total_at + 8 + u64_at(b, total_at)?;
    let mut out = splice(b, total_at + 8, end, &body)?;
    out[total_at..total_at + 8].copy_from_slice(&(body.len() as u64).to_le_bytes());
    Ok(out)
}

fn chunk(id: u16, body: &[u8]) -> Vec<u8> {
    let mut c = Vec::with_capacity(6 + body.len());
    c.extend_from_slice(&id.to_le_bytes());
    c.extend_from_slice(&(body.len() as u32).to_le_bytes());
    c.extend_from_slice(body);
    c
}

/// One path segment of a file table.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    /// 1: drive, 2: folder, 4: file, 8: monolith (`.nkx`) file the rest is inside.
    Named(u8, String),
    /// 6: Kontakt's own folder (its factory presets).
    KontaktDir,
    /// 11: the folder of the library the file belongs to, which Kontakt finds from the
    /// library's registration.
    LibraryRoot,
}

fn read_path(b: &[u8], at: &mut usize) -> Result<Vec<Seg>, String> {
    let n = u32_at(b, *at)?;
    *at += 4;
    if n > 256 {
        return Err(damaged());
    }
    let mut segs = Vec::new();
    for _ in 0..n {
        let t = *b.get(*at).ok_or_else(damaged)?;
        *at += 1;
        match t {
            1 | 2 | 4 | 8 => {
                let len = u32_at(b, *at)? as usize;
                let units: Vec<u16> = b
                    .get(*at + 4..*at + 4 + 2 * len)
                    .ok_or_else(damaged)?
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                *at += 4 + 2 * len;
                segs.push(Seg::Named(t, String::from_utf16(&units).map_err(|_| damaged())?));
            }
            6 => segs.push(Seg::KontaktDir),
            11 => segs.push(Seg::LibraryRoot),
            _ => return Err(format!("names a file in a way DECKARD doesn't know (path element {t})")),
        }
    }
    Ok(segs)
}

fn write_path(out: &mut Vec<u8>, segs: &[Seg]) {
    out.extend_from_slice(&(segs.len() as u32).to_le_bytes());
    for s in segs {
        match s {
            Seg::Named(t, name) => {
                let units: Vec<u16> = name.encode_utf16().collect();
                out.push(*t);
                out.extend_from_slice(&(units.len() as u32).to_le_bytes());
                for u in units {
                    out.extend_from_slice(&u.to_le_bytes());
                }
            }
            Seg::KontaktDir => out.push(6),
            Seg::LibraryRoot => out.push(11),
        }
    }
}

/// The segments naming `dir`, an absolute Windows path with a drive letter.
fn dir_segments(dir: &Path) -> Result<Vec<Seg>, String> {
    let mut segs = Vec::new();
    let mut parts = dir.components();
    match parts.next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => segs.push(Seg::Named(1, (d as char).to_ascii_uppercase().to_string())),
            _ => return Err("on a network path, which DECKARD can't hand to Kontakt".into()),
        },
        _ => return Err("not on a drive Kontakt can name (Windows only)".into()),
    }
    for c in parts {
        match c {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(n) => segs.push(Seg::Named(2, n.to_string_lossy().into_owned())),
            Component::ParentDir => {
                segs.pop();
            }
            Component::Prefix(_) => return Err(damaged()),
        }
    }
    Ok(segs)
}

/// Where the file a path names is on disk: the monolith, for a file inside one. Inside a
/// registered library, only Kontakt knows.
fn on_disk(segs: &[Seg], kontakt_dir: &Path) -> Option<PathBuf> {
    let mut p = PathBuf::new();
    for s in segs {
        match s {
            Seg::Named(1, d) => p.push(format!("{d}:\\")),
            Seg::KontaktDir => p.push(kontakt_dir),
            Seg::LibraryRoot => return None,
            Seg::Named(8, n) => {
                p.push(n);
                break;
            }
            Seg::Named(_, n) => p.push(n),
        }
    }
    Some(p)
}

/// A version 2 file table (chunk `0x4b`) with every relative path made absolute from
/// `dir`, after checking every file it names is there.
fn absolute_table(table: &[u8], dir: &[Seg], kontakt_dir: &Path) -> Result<Vec<u8>, String> {
    let body = &table[6..];
    let version = u16::from_le_bytes(body.get(..2).ok_or_else(damaged)?.try_into().unwrap());
    if version != 2 {
        return Err(NEW_TABLE.into());
    }
    let mut out = body[..2].to_vec();
    let mut at = 2;
    let mut missing = Vec::new();
    let mut in_library = false;
    let mut fix = |segs: Vec<Seg>, out: &mut Vec<u8>| {
        let segs = match segs.first() {
            Some(Seg::Named(2 | 4 | 8, _)) => dir.iter().cloned().chain(segs).collect(),
            _ => segs,
        };
        if !segs.is_empty() {
            match on_disk(&segs, kontakt_dir) {
                Some(p) if !p.exists() => missing.push(p),
                Some(_) => {}
                None => in_library = true,
            }
        }
        write_path(out, &segs);
    };
    // Special files (resource containers, the instrument's folder), then the samples and
    // their timestamps and flags (12 bytes each), then other files (the instrument, IRs).
    for list in 0..3 {
        let n = u32_at(body, at)? as usize;
        at += 4;
        out.extend_from_slice(&(n as u32).to_le_bytes());
        for _ in 0..n {
            let segs = read_path(body, &mut at)?;
            fix(segs, &mut out);
        }
        if list == 1 {
            let extra = body.get(at..at + 12 * n).ok_or_else(damaged)?;
            out.extend_from_slice(extra);
            at += 12 * n;
        }
    }
    out.extend_from_slice(body.get(at..).ok_or_else(damaged)?);
    if in_library {
        return Err(LIBRARY.into());
    }
    if let Some(first) = missing.first() {
        let more = if missing.len() > 1 { format!(" (and {} more)", missing.len() - 1) } else { String::new() };
        return Err(format!("needs {}{more}, which isn't there: Kontakt would stop on its \"missing content\" box", first.display()));
    }
    Ok(chunk(0x4b, &out))
}

/// Kontakt's own folder of factory presets (`presets` is inside), for the plug-in named
/// `kontakt` ("Kontakt 7").
fn kontakt_dir(kontakt: &str) -> PathBuf {
    let common =
        std::env::var_os("CommonProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files\Common Files"));
    common.join("Native Instruments").join(kontakt)
}

/// The version of Kontakt that saved `nki` ("8.4.0.0"), from its unencrypted header
/// (frame `DSIN` 101 of its instrument item: `[u32 2][u32 1][u32 length][UTF-16]`).
pub fn saved_by(nki: &[u8]) -> Option<String> {
    let f = spans(nki).ok()?.into_iter().find(|s| s.node == Node::Frame(DSIN, 101))?;
    let own = own_data(nki, f.at).ok()?;
    (own..f.end.saturating_sub(4)).find_map(|p| {
        let n = u32_at(nki, p).ok()? as usize;
        if !(3..=24).contains(&n) {
            return None;
        }
        let units: Vec<u16> = nki.get(p + 4..p + 4 + 2 * n)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let v = String::from_utf16(&units).ok()?;
        let numeric = v.split('.').count() >= 3 && v.split('.').all(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()));
        numeric.then_some(v)
    })
}

/// Refuse an instrument a newer Kontakt saved than the plug-in named `kontakt` ("Kontakt
/// 7"; "Kontakt" is 6): it would stop on a "your version of Kontakt is too old" box.
pub fn check_version(nki: &[u8], kontakt: &str) -> Result<(), String> {
    let lower = kontakt.to_ascii_lowercase();
    let have = if lower == "kontakt" { Some(6) } else { lower.strip_prefix("kontakt ").and_then(|v| v.parse::<u32>().ok()) };
    let saved = saved_by(nki);
    let need = saved.as_deref().and_then(|v| v.split('.').next()?.parse::<u32>().ok());
    match (have, need, saved) {
        (Some(have), Some(need), Some(v)) if need > have => {
            Err(format!("saved by Kontakt {v}, newer than {kontakt}, which would refuse it. Load it where Kontakt {need} is installed."))
        }
        _ => Ok(()),
    }
}

/// Refuse a snapshot (`.nksn`) Kontakt couldn't open without stopping on a dialog: one
/// from a newer Kontakt, or whose instrument, named by path, isn't there (Kontakt would
/// stop on "Content Missing"). One naming its instrument inside a registered library, or
/// encrypted, is Kontakt's to resolve.
pub fn check_snapshot(nksn: &[u8], kontakt: &str) -> Result<(), String> {
    if nksn.get(12..16) != Some(b"hsin") {
        return Err("not a Kontakt 5 or later snapshot".into());
    }
    check_version(nksn, kontakt)?;
    let Ok(program) = subtree(nksn) else { return Ok(()) };
    let Ok((_, parts)) = chunks(&program.data) else { return Ok(()) };
    let Some((_, table)) = parts.iter().find(|(id, _)| *id == 0x4b) else { return Ok(()) };
    let body = &table[6..];
    if body.get(..2) != Some(&2u16.to_le_bytes()[..]) {
        return Ok(());
    }
    let kontakt_dir = kontakt_dir(kontakt);
    let mut at = 2;
    for list in 0..3 {
        let n = u32_at(body, at)? as usize;
        at += 4;
        for _ in 0..n {
            let segs = read_path(body, &mut at)?;
            match segs.first() {
                None | Some(Seg::LibraryRoot) => {}
                Some(Seg::Named(1, _) | Seg::KontaktDir) => {
                    if let Some(p) = on_disk(&segs, &kontakt_dir).filter(|p| !p.exists()) {
                        return Err(format!("needs {}, which isn't there: Kontakt would stop on its \"Content Missing\" box", p.display()));
                    }
                }
                Some(_) => return Err("names its instrument relative to somewhere DECKARD doesn't know".into()),
            }
        }
        if list == 1 {
            at += 12 * n;
        }
    }
    Ok(())
}

/// The Kontakt VST3 component state that loads the instrument in `nki` (an `.nki` file's
/// bytes, from `path`) into the plug-in named `kontakt`, or why it can't. `empty` gives
/// that Kontakt's empty multi ([`empty_multi`]), asked only once the instrument can be built.
pub fn instrument_state(
    nki: &[u8],
    path: &Path,
    kontakt: &str,
    empty: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    if nki.get(12..16) != Some(b"hsin") {
        return Err("an old (Kontakt 4 or earlier) instrument, which only Kontakt's own browser can open".into());
    }
    check_version(nki, kontakt)?;
    let program = match subtree(nki) {
        Ok(s) => s.data,
        Err(e) if e == "ENCRYPTED" => return Err(ENCRYPTED.into()),
        Err(e) => return Err(e),
    };
    let (_, parts) = chunks(&program).map_err(|_| ENCRYPTED.to_string())?;
    let find = |id: u16| parts.iter().find(|(i, _)| *i == id).map(|(_, c)| c.as_slice());
    let prog = find(0x28).ok_or(ENCRYPTED)?;
    let table = find(0x4b).ok_or_else(damaged)?;
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    let dir = dir_segments(path.parent().ok_or_else(damaged)?)?;
    let table = absolute_table(table, &dir, &kontakt_dir(kontakt))?;

    // The slot: its container (0x29) holding the instrument's settings and program.
    let mut prefix = SLOT_PREFIX;
    if let Some(settings) = find(0x47).filter(|s| s.len() == 24) {
        prefix[SLOT_SETTINGS_AT..SLOT_SETTINGS_AT + 24].copy_from_slice(settings);
    }
    let mut program_body = 1u32.to_le_bytes().to_vec();
    program_body.extend_from_slice(&prog[6..]);
    let program_chunk = chunk(0x36, &program_body);
    let rest = (prefix.len() - SLOT_LEN_AT - 4 + program_chunk.len()) as u32;
    prefix[SLOT_LEN_AT..SLOT_LEN_AT + 4].copy_from_slice(&rest.to_le_bytes());
    let mut container = prefix.to_vec();
    container.extend_from_slice(&program_chunk);
    let mut slots = Vec::new();
    slots.extend_from_slice(&1u32.to_le_bytes());
    slots.extend_from_slice(&0u32.to_le_bytes());
    slots.extend_from_slice(&chunk(0x29, &container));
    let slots = chunk(0x37, &slots);

    // Kontakt's empty multi: its subtree holds the multi, whose own subtree holds the
    // program item: the bank (0x3), browser state (0xf02) and file table (0x4b).
    let empty = empty()?;
    check_multi(&empty)?;
    let multi = subtree(&empty)?.data;
    let bank_item = subtree(&multi)?.data;
    let (total_at, parts) = chunks(&bank_item)?;
    let mut out = Vec::new();
    for (id, c) in parts {
        out.push(match id {
            0x3 => (id, bank_with_slot(&c, &slots)?),
            0x4b => (id, table.clone()),
            _ => (id, c),
        });
    }
    let bank_item = put_chunks(&bank_item, total_at, &out)?;
    let multi = put_subtree(&multi, &bank_item)?;
    put_subtree(&empty, &multi)
}

/// The bank chunk `bank` with its slot list (0x37) replaced by `slots`.
fn bank_with_slot(bank: &[u8], slots: &[u8]) -> Result<Vec<u8>, String> {
    let body = &bank[6..];
    // The bank's header ends in the length of the chunks after it.
    let head = (0..body.len().min(0x200))
        .find(|&p| p + 4 <= body.len() && u32_at(body, p).is_ok_and(|n| p + 4 + n as usize == body.len()))
        .ok_or_else(damaged)?;
    let mut rest = Vec::new();
    let mut o = head + 4;
    let mut found = false;
    while o < body.len() {
        let id = u16::from_le_bytes(body[o..o + 2].try_into().unwrap());
        let next = o + 6 + u32_at(body, o + 2)? as usize;
        if id == 0x37 {
            rest.extend_from_slice(slots);
            found = true;
        } else {
            rest.extend_from_slice(body.get(o..next).ok_or_else(damaged)?);
        }
        o = next;
    }
    if !found {
        return Err(damaged());
    }
    let mut new = body[..head].to_vec();
    new.extend_from_slice(&(rest.len() as u32).to_le_bytes());
    new.extend_from_slice(&rest);
    Ok(chunk(0x3, &new))
}

/// FastLZ, level 1 or 2 (the level is in the first byte's top bits).
fn fastlz(data: &[u8], expected: usize) -> Option<Vec<u8>> {
    let level = data.first()? >> 5;
    if level > 1 {
        return None;
    }
    let mut out = Vec::with_capacity(expected.min(1 << 26));
    let mut i = 1usize;
    let mut ctrl = (data[0] & 31) as usize;
    loop {
        if ctrl < 32 {
            let n = ctrl + 1;
            out.extend_from_slice(data.get(i..i.checked_add(n)?)?);
            i += n;
        } else {
            let mut len = (ctrl >> 5) - 1;
            let ofs = (ctrl & 31) << 8;
            if len == 6 {
                if level == 0 {
                    len += *data.get(i)? as usize;
                    i += 1;
                } else {
                    loop {
                        let c = *data.get(i)? as usize;
                        i += 1;
                        len += c;
                        if c != 255 {
                            break;
                        }
                    }
                }
            }
            let code = *data.get(i)? as usize;
            i += 1;
            let mut dist = ofs + code + 1;
            if level == 1 && code == 255 && ofs == 31 << 8 {
                dist = ((*data.get(i)? as usize) << 8 | *data.get(i + 1)? as usize) + 8191 + 1;
                i += 2;
            }
            let start = out.len().checked_sub(dist)?;
            for k in 0..len + 3 {
                let b = out[start + k];
                out.push(b);
            }
        }
        if out.len() > expected {
            return None;
        }
        if i >= data.len() {
            break;
        }
        ctrl = data[i] as usize;
        i += 1;
    }
    (out.len() == expected).then_some(out)
}

/// Why an instrument Kontakt was given to open didn't load.
pub const NOT_OPENED: &str = "Kontakt didn't open it (nothing loaded).";

/// The state Kontakt saves once it has opened the preset (`.nki`, `.nksn`) at `path`
/// itself, with a throwaway Kontakt (`plugin_id`, `name`). This is how an encrypted
/// library instrument ([`ENCRYPTED`]) loads: Kontakt opens, decrypts and licenses it
/// itself; DECKARD never reads inside it. The result is a `PluginHost` chunk, recalled
/// like any state Kontakt or Maschine saved.
///
/// Kontakt is asked the way Maschine asks it (traced with vst3_hook, 2026-11-10): it gets
/// its own empty state, then the load-by-path command of its NKS interface
/// ([`nks_load_payload`]). A Kontakt that doesn't answer that interface has the file
/// dropped on its never-shown window instead, as a user dragging it from Explorer does.
///
/// Must be called on the main thread, where Maschine sends the command. Kontakt loads
/// inside the call (Electric Mint 0.5 s, Prime Bass 1.4 s, more from a cold disk with
/// Kontakt's own Progress window up); the last few results are kept, so loading the same
/// file again (a rig preflight, then the load) doesn't open it twice.
#[cfg(windows)]
pub fn state_opened_by_kontakt(path: &Path, plugin_id: &str, name: &str) -> Result<Vec<u8>, String> {
    let file = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    check_version(&file, name)?;
    opened::state(path, &file, plugin_id, name)
}

/// The id of a preset file's instrument item (the root's first child), which Maschine
/// sends after the path. Kontakt also takes zeros.
pub fn preset_id(file: &[u8]) -> [u8; 16] {
    let mut id = [0; 16];
    if let Ok(spans) = spans(file) {
        if let Some(child) = spans.iter().filter(|s| s.node == Node::Item).nth(1) {
            if let Some(b) = file.get(child.at + 0x18..child.at + 0x28) {
                id.copy_from_slice(b);
            }
        }
    }
    id
}

/// What Kontakt's load-by-path command (`0x6E690008`) points at, as Maschine builds it:
/// `[u32 1][u32 1][u32 2][u32 length]`, the absolute path in UTF-8 without a terminator,
/// then the preset's 16-byte id.
pub fn nks_load_payload(path: &str, id: [u8; 16]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(32 + path.len());
    for v in [1u32, 1, 2, path.len() as u32] {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    payload.extend_from_slice(path.as_bytes());
    payload.extend_from_slice(&id);
    payload
}

#[cfg(windows)]
mod opened {
    use super::NOT_OPENED;
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, Instant, SystemTime};
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, EnumChildWindows, GetPropW, GetWindowRect, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);
    const IID_ISHELLITEM: Guid = Guid(0x43826d1e, 0xe718, 0x42ee, [0xbc, 0x55, 0xa1, 0xe2, 0x61, 0xc3, 0x7b, 0xfe]);
    const BHID_DATAOBJECT: Guid = Guid(0xb8c0bd9f, 0xed24, 0x455c, [0x83, 0xe6, 0xd5, 0x39, 0x0c, 0x4f, 0xe8, 0xc4]);
    const IID_IDATAOBJECT: Guid = Guid(0x0000010e, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
    /// `MK_LBUTTON`: the left button is held while dragging.
    const LEFT_BUTTON: u32 = 1;
    /// `DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK`, as Explorer offers.
    const ANY_EFFECT: u32 = 7;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct PointL {
        x: i32,
        y: i32,
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn OleInitialize(reserved: *mut c_void) -> i32;
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHCreateItemFromParsingName(path: *const u16, bc: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> i32;
    }

    /// A COM object pointer, released when dropped.
    struct Com(*mut c_void);
    impl Com {
        unsafe fn method(&self, index: usize) -> usize {
            unsafe { *(*(self.0 as *const *const usize)).add(index) }
        }
    }
    impl Drop for Com {
        fn drop(&mut self) {
            type Release = unsafe extern "system" fn(*mut c_void) -> u32;
            if !self.0.is_null() {
                unsafe {
                    let release: Release = std::mem::transmute(self.method(2));
                    release(self.0);
                }
            }
        }
    }

    type Key = (PathBuf, u64, Option<SystemTime>);
    static RECENT: Mutex<Vec<(Key, Vec<u8>)>> = Mutex::new(Vec::new());
    const KEEP: usize = 4;

    pub fn state(path: &Path, file: &[u8], plugin_id: &str, name: &str) -> Result<Vec<u8>, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let key: Key = (path.to_path_buf(), meta.len(), meta.modified().ok());
        if let Some((_, chunk)) = RECENT.lock().unwrap().iter().find(|(k, _)| *k == key) {
            return Ok(chunk.clone());
        }
        if !crate::on_main_thread() {
            return Err("Kontakt can only open a file on the main thread".into());
        }
        let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
        let text = absolute.to_str().ok_or("the file's path isn't valid Unicode")?.replace('/', "\\");
        // As Maschine: a Kontakt in its own empty state (made with none), then the load
        // command.
        let mut host = crate::Host::default();
        let plugin = host.instance(crate::Slot::Sound { group: 0, sound: 0 }, plugin_id, name, &[], 48_000.0)?;
        let empty = plugin.lock().map_err(|_| "Kontakt is unavailable")?.save_chunk();
        let payload = super::nks_load_payload(&text, super::preset_id(file));
        let sent = {
            let p = plugin.lock().map_err(|_| "Kontakt is unavailable")?;
            let component = p.component().ok_or("not a VST3 Kontakt")?;
            unsafe { command_load(component.as_ptr() as *mut c_void, &payload) }
        };
        let chunk = if sent { plugin.lock().map_err(|_| "Kontakt is unavailable")?.save_chunk() } else { by_drop(path, &plugin, name)? };
        drop(plugin);
        drop(host);
        if chunk.len() <= empty.len() {
            return Err(NOT_OPENED.into());
        }
        let mut recent = RECENT.lock().unwrap();
        recent.retain(|(k, _)| k.0 != key.0);
        recent.push((key, chunk.clone()));
        if recent.len() > KEEP {
            recent.remove(0);
        }
        Ok(chunk)
    }

    /// Kontakt's NKS command interface (`45AF934E-90A4684C-BF41B535-443028AE`, memory
    /// order), which its component answers: three `FUnknown` methods, then one command
    /// dispatch, `(this, u32 command, u64 size, data, u64 0, result block)`.
    const NKS_COMMANDS: [u8; 16] = [0x45, 0xAF, 0x93, 0x4E, 0x90, 0xA4, 0x68, 0x4C, 0xBF, 0x41, 0xB5, 0x35, 0x44, 0x30, 0x28, 0xAE];
    /// Load the preset whose path the data points at.
    const LOAD_BY_PATH: u32 = 0x6E69_0008;

    /// Send Kontakt's component the load-by-path command, as Maschine does: the data is
    /// `[u64 payload address][u32 payload length][8 zero bytes]`. Whether Kontakt has the
    /// interface (it returns 1 whether or not anything loaded: the caller compares states).
    unsafe fn command_load(component: *mut c_void, payload: &[u8]) -> bool {
        type Query = unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32;
        type Command = unsafe extern "system" fn(*mut c_void, u32, u64, *const c_void, u64, *mut c_void) -> i64;
        let component = Com(component);
        let mut nks = std::ptr::null_mut();
        let comp_ptr = component.0;
        let query: Query = unsafe { std::mem::transmute(component.method(0)) };
        // Borrowed from the plug-in: not ours to release.
        std::mem::forget(component);
        let Ok(answered) =
            crate::seh::guard("NKS plug-in", "queryInterface(NKS_COMMANDS)", || unsafe { query(comp_ptr, &NKS_COMMANDS, &mut nks) })
        else {
            return false;
        };
        if answered != 0 || nks.is_null() {
            return false;
        }
        let nks = Com(nks);
        let mut data = Vec::with_capacity(20);
        data.extend_from_slice(&(payload.as_ptr() as u64).to_le_bytes());
        data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        data.extend_from_slice(&[0; 8]);
        let mut result = [0u8; 256];
        let nks_ptr = nks.0;
        let command: Command = unsafe { std::mem::transmute(nks.method(3)) };
        let data_ptr = data.as_mut_ptr() as *const c_void;
        let data_len = data.len() as u64;
        let res_ptr = result.as_mut_ptr() as *mut c_void;
        if crate::seh::guard("NKS plug-in", "LOAD_BY_PATH", || unsafe { command(nks_ptr, LOAD_BY_PATH, data_len, data_ptr, 0, res_ptr) })
            .is_err()
        {
            std::mem::forget(nks);
            return false;
        }
        true
    }

    /// The fallback: drop the file on Kontakt's never-shown window.
    fn by_drop(path: &Path, plugin: &crate::Shared, name: &str) -> Result<Vec<u8>, String> {
        // Already initialised on the app's thread (S_FALSE); a failure means the thread
        // is in the wrong COM apartment for drag and drop.
        let hr = unsafe { OleInitialize(std::ptr::null_mut()) };
        if hr < 0 {
            return Err(format!("drag and drop is unavailable ({hr:#x})"));
        }
        let data = data_object(path)?;
        let editor = crate::editor::Editor::open_hidden(plugin, name)?;
        let root = editor.hwnd();
        // Kontakt's view takes drops once it is set up: offer the file until it does.
        let start = Instant::now();
        let mut taken = false;
        while !taken && start.elapsed() < Duration::from_secs(10) {
            pump(root);
            if let Some((target, rect)) = drop_target(root) {
                taken = drop_on(&target, &data, rect);
            }
            if !taken {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        pump(root);
        let chunk = plugin.lock().map_err(|_| "Kontakt is unavailable")?.save_chunk();
        drop(editor);
        if !taken {
            return Err(NOT_OPENED.into());
        }
        Ok(chunk)
    }

    /// The shell's data object for the file: what Explorer hands a drop target.
    fn data_object(path: &Path) -> Result<Com, String> {
        let wide: Vec<u16> = path.as_os_str().to_string_lossy().replace('/', "\\").encode_utf16().chain(Some(0)).collect();
        let mut item = std::ptr::null_mut();
        let hr = unsafe { SHCreateItemFromParsingName(wide.as_ptr(), std::ptr::null_mut(), &IID_ISHELLITEM, &mut item) };
        if hr < 0 {
            return Err(format!("{}: the shell can't name it ({hr:#x})", path.display()));
        }
        let item = Com(item);
        type Bind = unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *const Guid, *mut *mut c_void) -> i32;
        let mut data = std::ptr::null_mut();
        let hr = unsafe {
            let bind: Bind = std::mem::transmute(item.method(3));
            bind(item.0, std::ptr::null_mut(), &BHID_DATAOBJECT, &IID_IDATAOBJECT, &mut data)
        };
        if hr < 0 {
            return Err(format!("{}: the shell has no data object for it ({hr:#x})", path.display()));
        }
        Ok(Com(data))
    }

    /// Dispatch the messages of the editor's window and the windows in it, and the
    /// thread's own, without touching the app's windows.
    fn pump(root: HWND) {
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        for hwnd in [root, -1isize as HWND] {
            for _ in 0..256 {
                if unsafe { PeekMessageW(&mut msg, hwnd, 0, 0, PM_REMOVE) } == 0 {
                    break;
                }
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        crate::editor::serve_timers();
    }

    unsafe extern "system" fn collect(h: HWND, l: LPARAM) -> BOOL {
        unsafe { (*(l as *mut Vec<HWND>)).push(h) };
        1
    }

    /// The largest window inside the editor with an OLE drop target: Kontakt's view.
    fn drop_target(root: HWND) -> Option<(Com, RECT)> {
        let mut all = vec![root];
        unsafe { EnumChildWindows(root, Some(collect), &mut all as *mut _ as LPARAM) };
        let prop: Vec<u16> = "OleDropTargetInterface\0".encode_utf16().collect();
        let mut best: Option<(usize, RECT, i64)> = None;
        for h in all {
            let target = unsafe { GetPropW(h, prop.as_ptr()) } as usize;
            if target == 0 {
                continue;
            }
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            unsafe { GetWindowRect(h, &mut r) };
            let area = (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
            if best.as_ref().is_none_or(|b| area > b.2) {
                best = Some((target, r, area));
            }
        }
        let (target, rect, _) = best?;
        // Borrowed from the window: hold a reference of our own while using it.
        type AddRef = unsafe extern "system" fn(*mut c_void) -> u32;
        let t = Com(target as *mut c_void);
        unsafe {
            let add_ref: AddRef = std::mem::transmute(t.method(1));
            add_ref(t.0);
        }
        Some((t, rect))
    }

    /// Drag the data object over the target's rack (low in its view, where an empty
    /// Kontakt shows its rack) and drop it there. Whether the target took it.
    fn drop_on(target: &Com, data: &Com, r: RECT) -> bool {
        type Enter = unsafe extern "system" fn(*mut c_void, *mut c_void, u32, PointL, *mut u32) -> i32;
        type Over = unsafe extern "system" fn(*mut c_void, u32, PointL, *mut u32) -> i32;
        type Leave = unsafe extern "system" fn(*mut c_void) -> i32;
        let at = PointL { x: (r.left + r.right) / 2, y: r.top + (r.bottom - r.top) * 2 / 3 };
        unsafe {
            let enter: Enter = std::mem::transmute(target.method(3));
            let over: Over = std::mem::transmute(target.method(4));
            let leave: Leave = std::mem::transmute(target.method(5));
            let drop: Enter = std::mem::transmute(target.method(6));
            let mut effect = ANY_EFFECT;
            if enter(target.0, data.0, LEFT_BUTTON, at, &mut effect) < 0 {
                return false;
            }
            effect = ANY_EFFECT;
            if over(target.0, LEFT_BUTTON, at, &mut effect) < 0 || effect == 0 {
                leave(target.0);
                return false;
            }
            effect = ANY_EFFECT;
            drop(target.0, data.0, LEFT_BUTTON, at, &mut effect) >= 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An item: header, a frame chain (outermost first, ending in `DSIN` 1), children.
    fn item(frames: &[([u8; 4], u32, Vec<u8>)], children: &[([u8; 4], u32, Vec<u8>)]) -> Vec<u8> {
        // Innermost first: DSIN 1 with a u32, then each frame wraps the previous.
        let mut f = frame(DSIN, 1, &[], &1u32.to_le_bytes());
        for (d, id, own) in frames.iter().rev() {
            f = frame(*d, *id, &f, own);
        }
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(b"hsin");
        body.extend_from_slice(&[0; 8]);
        body.extend_from_slice(&[7; 16]);
        body.extend_from_slice(&f);
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&(children.len() as u32).to_le_bytes());
        for (d, id, c) in children {
            body.extend_from_slice(&0u32.to_le_bytes());
            body.extend_from_slice(d);
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(c);
        }
        let mut out = ((body.len() + 8) as u64).to_le_bytes().to_vec();
        out.extend_from_slice(&body);
        out
    }

    fn frame(domain: [u8; 4], id: u32, nested: &[u8], own: &[u8]) -> Vec<u8> {
        let mut out = ((20 + nested.len() + own.len()) as u64).to_le_bytes().to_vec();
        out.extend_from_slice(&domain);
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(nested);
        out.extend_from_slice(own);
        out
    }

    fn raw_subtree_item(sub: &[u8]) -> Vec<u8> {
        let mut own = 1u32.to_le_bytes().to_vec();
        own.push(0);
        own.extend_from_slice(sub);
        item(&[(DSIN, 116, Vec::new()), (DSIN, SUBTREE, own)], &[])
    }

    fn program_item(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut own = [1u32, 1, 1].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        own.extend_from_slice(&(body.len() as u64).to_le_bytes());
        own.extend_from_slice(&body);
        own.extend_from_slice(&[0x0d, 0x62, 0x65, 0x85, 1, 0, 0, 0]);
        let program = item(&[(DSIN, PROGRAM_ITEM, own)], &[]);
        item(&[], &[(DSIN, PROGRAM_ITEM, program)])
    }

    fn path(segs: &[Seg]) -> Vec<u8> {
        let mut out = Vec::new();
        write_path(&mut out, segs);
        out
    }

    fn v2_table(specials: &[Vec<Seg>], samples: &[Vec<Seg>], others: &[Vec<Seg>]) -> Vec<u8> {
        let mut t = 2u16.to_le_bytes().to_vec();
        for (k, list) in [specials, samples, others].into_iter().enumerate() {
            t.extend_from_slice(&(list.len() as u32).to_le_bytes());
            for p in list {
                t.extend_from_slice(&path(p));
            }
            if k == 1 {
                for _ in list {
                    t.extend_from_slice(&[0x55; 8]);
                    t.extend_from_slice(&[0; 4]);
                }
            }
        }
        t.extend_from_slice(&[1, 0, 0, 0, 0, 0]);
        chunk(0x4b, &t)
    }

    /// A Kontakt 5/6 style `.nki`: root item, a 4KIN child with the program subtree.
    fn nki(program: &[u8]) -> Vec<u8> {
        let inst = item(&[([b'4', b'K', b'I', b'N'], 3, Vec::new())], &[(DSIN, 116, raw_subtree_item(program))]);
        item(&[(DSIN, 118, Vec::new())], &[([b'4', b'K', b'I', b'N'], 3, inst)])
    }

    /// A version 2 file table's three path lists.
    fn read_table(table: &[u8]) -> Vec<Vec<Vec<Seg>>> {
        let body = &table[6..];
        let mut at = 2;
        let mut lists = Vec::new();
        for list in 0..3 {
            let n = u32_at(body, at).unwrap() as usize;
            at += 4;
            lists.push((0..n).map(|_| read_path(body, &mut at).unwrap()).collect());
            if list == 1 {
                at += 12 * n;
            }
        }
        lists
    }

    fn named(t: u8, s: &str) -> Seg {
        Seg::Named(t, s.into())
    }

    /// An empty multi as Kontakt saves one, in miniature: its subtree holds the multi, whose
    /// subtree holds the program item with the bank (an empty slot list), the browser state
    /// and an empty file table.
    fn fixture_multi() -> Vec<u8> {
        let slots = chunk(0x37, &[0; 8]);
        let mut bank = vec![0xab; 6];
        bank.extend_from_slice(&(slots.len() as u32).to_le_bytes());
        bank.extend_from_slice(&slots);
        let bank_item = program_item(&[chunk(0x3, &bank), chunk(0xf02, &[0; 4]), v2_table(&[vec![], vec![]], &[], &[])]);
        raw_subtree_item(&raw_subtree_item(&bank_item))
    }

    #[test]
    fn an_empty_multi_is_one_instruments_go_in() {
        let empty = fixture_multi();
        check_multi(&empty).unwrap();
        let multi = subtree(&empty).unwrap().data;
        let bank_item = subtree(&multi).unwrap().data;
        let (_, parts) = chunks(&bank_item).unwrap();
        assert_eq!(parts.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0x3, 0xf02, 0x4b]);
        // Its slot list is empty, and the bank's header length is found.
        let bank = &parts[0].1;
        assert_eq!(&bank_with_slot(bank, &chunk(0x37, &[0; 8])).unwrap(), bank);
        // Anything else isn't filled: Kontakt opens the instrument itself.
        assert_eq!(check_multi(&raw_subtree_item(&program_item(&[chunk(0x3, &[0; 4])]))), Err(UNKNOWN_MULTI.into()));
        assert_eq!(check_multi(b"not a state"), Err(UNKNOWN_MULTI.into()));
    }

    /// The installed Kontakt 7, made with no state, saves the multi instruments are built
    /// into (Kontakt 7.10.9, 2026-11-24).
    #[test]
    #[ignore = "loads live Kontakt 7 VST3"]
    fn a_fresh_kontakt_saves_an_empty_multi() {
        if !Path::new(r"C:\Program Files\Common Files\VST3\Kontakt 7.vst3").exists() {
            return;
        }
        let kontakt = crate::scan::Entry {
            kind: crate::scan::Kind::Vst3,
            vendor: "Native Instruments".into(),
            name: "Kontakt 7".into(),
            instrument: true,
            location: String::new(),
            cid: "4E5453564B696B376F6E74616B742037".into(),
        };
        let id = kontakt.maschine_id().unwrap();
        let empty = empty_multi(&id, "Kontakt 7").unwrap();
        check_multi(&empty).unwrap();
        assert_eq!(empty_multi(&id, "Kontakt 7").unwrap(), empty, "kept");
    }

    #[test]
    fn the_slot_prefix_is_where_its_fields_say() {
        assert_eq!(SLOT_PREFIX[SLOT_SETTINGS_AT..SLOT_SETTINGS_AT + 6], [0x47, 0, 0x12, 0, 0, 0]);
        assert_eq!(SLOT_PREFIX[SLOT_LEN_AT..SLOT_LEN_AT + 4], [0; 4]);
    }

    #[test]
    fn fastlz_reads_both_levels() {
        // Level 1: 3 literals "abc", then a match of 3 at distance 3.
        assert_eq!(fastlz(&[2, b'a', b'b', b'c', 0x20, 2], 6).as_deref(), Some(&b"abcabc"[..]));
        // Level 2 marks the first byte with 1 << 5.
        assert_eq!(fastlz(&[0x22, b'a', b'b', b'c', 0x20, 2], 6).as_deref(), Some(&b"abcabc"[..]));
        // Anything else is not FastLZ (an encrypted instrument).
        assert_eq!(fastlz(&[0xab, 1, 2, 3], 6), None);
        assert_eq!(fastlz(&[2, b'a', b'b', b'c', 0x20, 9], 6), None);
    }

    #[cfg(windows)]
    #[test]
    fn an_instrument_becomes_a_one_slot_multi_with_absolute_paths() {
        let dir = std::env::temp_dir().join(format!("deckard-kontakt-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Samples")).unwrap();
        std::fs::write(dir.join("Samples").join("a.ncw"), b"x").unwrap();
        let samples = vec![vec![named(2, "Samples"), named(4, "a.ncw")]];
        let others = vec![vec![named(4, "Piano.nki")]];
        std::fs::write(dir.join("Piano.nki"), b"x").unwrap();
        let program = b"PROGRAM-BODY".to_vec();
        let settings = chunk(0x47, &[9; 0x12]);
        let raw = nki(&program_item(&[chunk(0x28, &program), settings.clone(), v2_table(&[vec![], vec![]], &samples, &others)]));

        let state = instrument_state(&raw, &dir.join("Piano.nki"), "Kontakt 7", || Ok(fixture_multi())).unwrap();
        // Still a well-formed container, as Kontakt's own is.
        spans(&state).unwrap();
        let multi = subtree(&state).unwrap().data;
        let bank_item = subtree(&multi).unwrap().data;
        let (_, parts) = chunks(&bank_item).unwrap();
        assert_eq!(parts.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0x3, 0xf02, 0x4b]);
        // The bank's one slot holds the settings and the program body.
        let bank = &parts[0].1;
        let at = bank.windows(program.len()).position(|w| w == program.as_slice()).unwrap();
        assert_eq!(&bank[at - 10..at - 4], &chunk(0x36, &[0; 4 + 12])[..6]);
        assert!(bank.windows(settings.len()).any(|w| w == settings.as_slice()));
        // The file table names the sample and the instrument from the drive down.
        let lists = read_table(&parts[2].1);
        assert_eq!(lists[0], [Vec::<Seg>::new(), Vec::new()]);
        assert!(matches!(lists[1][0][0], Seg::Named(1, _)));
        assert_eq!(on_disk(&lists[1][0], Path::new("")), Some(dir.join("Samples").join("a.ncw")));
        assert_eq!(on_disk(&lists[2][0], Path::new("")), Some(dir.join("Piano.nki")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn a_missing_sample_is_refused_before_kontakt_sees_it() {
        let dir = std::env::temp_dir().join(format!("deckard-kontakt-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let samples = vec![vec![named(2, "Samples"), named(4, "gone.ncw")]];
        let raw = nki(&program_item(&[chunk(0x28, b"P"), v2_table(&[], &samples, &[])]));
        let err = instrument_state(&raw, &dir.join("x.nki"), "Kontakt 7", || Ok(fixture_multi())).unwrap_err();
        assert!(err.contains("gone.ncw") && err.contains("missing content"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unreadable_instruments_are_refused_with_a_reason() {
        // An old (Kontakt 4) file.
        assert!(
            instrument_state(b"/\\ NI FC MTD  /\\", Path::new("C:\\x.nki"), "Kontakt 7", || Ok(fixture_multi()))
                .unwrap_err()
                .contains("Kontakt 4")
        );
        // Encrypted: the program subtree isn't FastLZ.
        let mut own = 1u32.to_le_bytes().to_vec();
        own.push(1);
        own.extend_from_slice(&64u32.to_le_bytes());
        own.extend_from_slice(&4u32.to_le_bytes());
        own.extend_from_slice(&[0xab, 0xe1, 0x69, 0xf3]);
        let sub = item(&[(DSIN, 116, Vec::new()), (DSIN, SUBTREE, own)], &[]);
        let raw = item(&[(DSIN, 118, Vec::new())], &[([b'4', b'K', b'I', b'N'], 3, sub)]);
        assert_eq!(instrument_state(&raw, Path::new("C:\\x.nki"), "Kontakt 7", || Ok(fixture_multi())).unwrap_err(), ENCRYPTED);
        // A Kontakt 7 (version 3) file table.
        let mut t3 = 3u16.to_le_bytes().to_vec();
        t3.extend_from_slice(&[0; 8]);
        let raw = nki(&program_item(&[chunk(0x28, b"P"), chunk(0x4b, &t3)]));
        // Past the checks above the file's folder is named by its drive, which only
        // Windows paths have: elsewhere that is the refusal instead.
        if cfg!(windows) {
            assert_eq!(instrument_state(&raw, Path::new("C:\\x.nki"), "Kontakt 7", || Ok(fixture_multi())).unwrap_err(), NEW_TABLE);
        }
    }

    #[test]
    fn an_instrument_from_a_newer_kontakt_is_refused() {
        let mut own = Vec::new();
        own.extend_from_slice(&[0; 4]);
        own.extend_from_slice(&2u32.to_le_bytes());
        own.extend_from_slice(&1u32.to_le_bytes());
        own.extend_from_slice(&7u32.to_le_bytes());
        own.extend(b"8.4.0.0".iter().flat_map(|&c| [c, 0]));
        let inst = item(&[([b'4', b'K', b'I', b'N'], 3, Vec::new()), (DSIN, 101, own)], &[]);
        let raw = item(&[(DSIN, 118, Vec::new())], &[([b'4', b'K', b'I', b'N'], 3, inst)]);
        assert_eq!(saved_by(&raw).as_deref(), Some("8.4.0.0"));
        assert!(check_version(&raw, "Kontakt 7").unwrap_err().contains("Kontakt 8.4.0.0"));
        assert!(check_version(&raw, "Kontakt 8").is_ok());
        assert!(check_version(&raw, "Kontakt").is_err());
        assert!(instrument_state(&raw, Path::new("C:\\x.nki"), "Kontakt 7", || Ok(fixture_multi())).unwrap_err().contains("newer"));
        // No version found: nothing to refuse.
        assert!(check_version(b"", "Kontakt 7").is_ok());
    }

    #[test]
    fn library_paths_are_left_to_kontakt() {
        let samples = vec![vec![Seg::LibraryRoot, named(2, "Samples"), named(4, "a.ncw")]];
        let raw = nki(&program_item(&[chunk(0x28, b"P"), v2_table(&[], &samples, &[])]));
        // Past the checks above the file's folder is named by its drive, which only
        // Windows paths have: elsewhere that is the refusal instead.
        if cfg!(windows) {
            assert_eq!(instrument_state(&raw, Path::new("C:\\x.nki"), "Kontakt 7", || Ok(fixture_multi())).unwrap_err(), LIBRARY);
        }
        // A snapshot naming its instrument inside its library is Kontakt's to resolve; one
        // naming a path that isn't there is refused.
        let others = vec![vec![Seg::LibraryRoot, named(2, "Instruments"), named(4, "Mint.nki")]];
        let snap = nki(&program_item(&[chunk(0x51, b"S"), v2_table(&others, &[], &[])]));
        assert!(check_snapshot(&snap, "Kontakt 7").is_ok());
        let gone = vec![vec![named(1, ""), named(2, "Users"), named(4, "Piano.nki")]];
        let snap = nki(&program_item(&[chunk(0x51, b"S"), v2_table(&gone, &[], &[])]));
        assert!(check_snapshot(&snap, "Kontakt 7").unwrap_err().contains("Content Missing"));
        assert!(check_snapshot(b"nope", "Kontakt 7").is_err());
    }

    #[test]
    fn the_load_command_carries_the_path_and_the_presets_id() {
        let path = r"C:\Users\Public\Documents\Session Bassist - Prime Bass Library\Snapshots\Prime Bass\Funk Machine.nksn";
        let id = [0x64, 0x65, 0x0a, 0xec, 0x22, 0x41, 0x4b, 0x64, 0x9a, 0x88, 0x9a, 0x42, 0x76, 0x3f, 0x40, 0xa0];
        let p = nks_load_payload(path, id);
        // As Maschine sent it for this preset: 133 bytes.
        assert_eq!(p.len(), 133);
        assert_eq!(&p[..16], &[1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 101, 0, 0, 0]);
        assert_eq!(&p[16..117], path.as_bytes());
        assert_eq!(&p[117..], &id);
        // The id is the instrument item's: the root's first child.
        let inst = item(&[([b'4', b'K', b'I', b'N'], 3, Vec::new())], &[]);
        let mut raw = item(&[(DSIN, 118, Vec::new())], &[([b'4', b'K', b'I', b'N'], 3, inst.clone())]);
        let child = raw.len() - inst.len();
        raw[child + 0x18..child + 0x28].copy_from_slice(&id);
        assert_eq!(preset_id(&raw), id);
        assert_eq!(preset_id(b"not a preset"), [0; 16]);
    }

    #[test]
    fn splice_resizes_everything_around_the_change() {
        let program = program_item(&[chunk(0x28, b"abc")]);
        let (total_at, mut parts) = chunks(&program).unwrap();
        parts[0].1 = chunk(0x28, b"a much longer program body");
        let grown = put_chunks(&program, total_at, &parts).unwrap();
        assert_eq!(grown.len(), program.len() + 23);
        spans(&grown).unwrap();
        assert_eq!(chunks(&grown).unwrap().1, parts);
    }
}
