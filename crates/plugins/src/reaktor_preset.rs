//! A Reaktor `.nrkt` snapshot names an ensemble; it isn't Reaktor's state, and Reaktor
//! given its bytes as state ignores them (nor does it open an `.nrkt` by path). So:
//! find the ensemble ([`reference`], [`resolve`]), have Reaktor open it (by path, as
//! `.ens` presets load: `nks::reaktor_ens`) and save its state, and put the snapshot in
//! that state where Reaktor's own snapshot of the ensemble is ([`Frame`]). The slot is
//! named after the ensemble ("Monark", not "Reaktor 6").
//!
//! Reaktor's state (Reaktor 6, `4RIN` 17 / `CSAR` 6): a header (the ensemble's path, its
//! name, its folder), then the host frame: a prefix (the host's settings, with the bank and
//! program it last recalled in a `BEER` block), the ensemble's snapshot as a NISound item
//! (`[u32 length]` then the item), and a tail (the ensemble's MIDI and parameter tables).
use std::path::{Path, PathBuf};

/// `raw` (an `.nrkt`'s bytes) as Reaktor's state, and the ensemble's name. `ensemble_state`
/// gives Reaktor's own state with the ensemble at a path opened ([`Frame`]).
pub(crate) fn wrap(raw: &[u8], ensemble_state: impl FnOnce(&Path) -> Result<Vec<u8>, String>) -> Result<(String, Vec<u8>), String> {
    let reference = reference(raw)?;
    let ensemble = resolve(&reference, &content_roots())?;
    let name = ensemble.file_stem().and_then(|s| s.to_str()).ok_or("invalid Reaktor ensemble name")?.to_owned();
    let frame = Frame::parse(&ensemble_state(&ensemble)?)?;
    let raw = instrument_snapshot_as_ensemble(raw, &frame.context)?;
    Ok((name, frame.with(&raw)))
}

/// Reaktor's state with an ensemble open, around its snapshot: the snapshot goes between
/// `prefix` and `tail`. `context` is the non-musical single-instrument context of its
/// snapshot's program, which an instrument-only snapshot is framed with.
#[derive(Debug)]
struct Frame {
    /// The header, up to the host frame's length.
    header: Vec<u8>,
    prefix: Vec<u8>,
    tail: Vec<u8>,
    context: Vec<u8>,
}

impl Frame {
    fn parse(state: &[u8]) -> Result<Frame, String> {
        let bad = || "Reaktor saved a state DECKARD doesn't know".to_string();
        let u32_at = |b: &[u8], at: usize| b.get(at..at + 4).map(|v| u32::from_le_bytes(v.try_into().unwrap()) as usize);
        let u64_at = |b: &[u8], at: usize| b.get(at..at + 8).map(|v| u64::from_le_bytes(v.try_into().unwrap()) as usize);
        if state.get(1..5) != Some(b"4RIN") || state.get(11..15) != Some(b"CSAR") {
            return Err(bad());
        }
        // The header's three strings (path, name, folder), each followed by fixed fields.
        let mut at = 24;
        for after in [4, 8, 4] {
            at = u32_at(state, at).and_then(|n| at.checked_add(4 + n + after)).ok_or_else(bad)?;
        }
        let len = u32_at(state, at).ok_or_else(bad)?;
        let payload = state.get(at + 4..).filter(|p| p.len() == len).ok_or_else(bad)?;
        // The snapshot: a NISound item whose length is the `u32` before it, after the
        // host's settings (which end in their `BEER` block).
        let beer = payload.windows(4).position(|w| w == b"BEER").ok_or_else(bad)?;
        let p = (beer..payload.len().saturating_sub(20))
            .find(|&p| &payload[p + 16..p + 20] == b"hsin" && u32_at(payload, p).is_some() && u32_at(payload, p) == u64_at(payload, p + 4))
            .ok_or_else(bad)?;
        let n = u32_at(payload, p).ok_or_else(bad)?;
        let snapshot = payload.get(p + 4..p + 4 + n).ok_or_else(bad)?;
        // Its program (mode 0): [633][0][0][1][1][1][map length], the instrument map, the
        // context, then [u32 length] and the musical snapshot to its end.
        let m = snapshot.windows(8).position(|w| w == b"DSINm\0\0\0").ok_or_else(bad)?;
        if u32_at(snapshot, m + 112) != Some(633) || u32_at(snapshot, m + 116) != Some(0) {
            return Err(bad());
        }
        let program = u32_at(snapshot, m + 120).and_then(|n| snapshot.get(m + 124..m + 124 + n)).ok_or_else(bad)?;
        let map = u32_at(program, 24).ok_or_else(bad)?;
        let from = 28 + map;
        let to = (from..program.len().saturating_sub(4))
            .find(|&c| u32_at(program, c).is_some_and(|n| c + 4 + n == program.len()))
            .ok_or_else(bad)?;
        let mut prefix = payload[..p].to_vec();
        // The bank and program it last recalled: left in, Reaktor recalls that stored
        // snapshot over the one given.
        prefix[beer + 8..beer + 16].fill(0);
        prefix[beer + 48..beer + 60].fill(0);
        Ok(Frame { header: state[..at].to_vec(), prefix, tail: payload[p + 4 + n..].to_vec(), context: program[from..to].to_vec() })
    }

    /// Reaktor's state with `snapshot` (a NISound item, mode 0) in place of its own.
    fn with(&self, snapshot: &[u8]) -> Vec<u8> {
        let len = self.prefix.len() + 4 + snapshot.len() + self.tail.len();
        let mut out = self.header.clone();
        out.extend_from_slice(&(len as u32).to_le_bytes());
        out.extend_from_slice(&self.prefix);
        out.extend_from_slice(&(snapshot.len() as u32).to_le_bytes());
        out.extend_from_slice(snapshot);
        out.extend_from_slice(&self.tail);
        out
    }
}

/// An instrument-only snapshot (mode 5) omits the ensemble's instrument table, and Reaktor's
/// state takes mode 0: every map and value byte kept, the snapshot framed as a
/// single-instrument ensemble's program with `context` ([`Frame`]).
fn instrument_snapshot_as_ensemble(raw: &[u8], context: &[u8]) -> Result<Vec<u8>, String> {
    let u32_at = |at| raw.get(at..at + 4).map(|v| u32::from_le_bytes(v.try_into().unwrap()) as usize);
    let u64_at = |at| raw.get(at..at + 8).map(|v| u64::from_le_bytes(v.try_into().unwrap()) as usize);
    let Some(m) = raw.windows(8).position(|w| w == b"DSINm\0\0\0") else { return Err("Reaktor snapshot has no musical state".into()) };
    let mode = u32_at(m + 116).ok_or("truncated Reaktor musical state")?;
    let body_size = u32_at(m + 120).ok_or("truncated Reaktor musical state")?;
    raw.get(m + 124..m + 124 + body_size).ok_or("truncated Reaktor musical state")?;
    if mode == 0 {
        return Ok(raw.to_vec());
    }
    if mode != 5 {
        return Err(format!("unsupported Reaktor snapshot mode: {mode}"));
    }
    if u32_at(m + 112) != Some(633) || u32_at(m + 124) != Some(633) || u32_at(m + 128) != Some(5) {
        return Err("unsupported Reaktor instrument snapshot version".into());
    }
    let count = u32_at(m + 132).ok_or("truncated instrument snapshot")?;
    let map_len = count.checked_mul(8).and_then(|n| n.checked_add(16)).ok_or("invalid instrument map size")?;
    let body_len = u32_at(m + 120).ok_or("truncated instrument snapshot")?;
    let body = raw.get(m + 124..m + 124 + body_len).ok_or("truncated instrument snapshot body")?;
    let mapping = body.get(..map_len).ok_or("truncated instrument snapshot map")?;
    let snapshot = &body[map_len..];
    let mut program = Vec::new();
    for value in [633u32, 0, 0, 1, 1, 1, map_len as u32] {
        program.extend_from_slice(&value.to_le_bytes());
    }
    program.extend_from_slice(mapping);
    program.extend_from_slice(context);
    program.extend_from_slice(&(snapshot.len() as u32).to_le_bytes());
    program.extend_from_slice(snapshot);
    let mut payload = Vec::new();
    for value in [633u32, 0, program.len() as u32] {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    payload.extend_from_slice(&program);
    let old_len = u64_at(m + 104).ok_or("truncated Reaktor payload size")?;
    if old_len != 12 + body_len {
        return Err("inconsistent Reaktor payload lengths".into());
    }
    let end = m + 112 + old_len;
    let tail = raw.get(end..).ok_or("truncated Reaktor payload")?;
    let mut out = raw[..m + 104].to_vec();
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(tail);
    let delta = out.len() as i64 - raw.len() as i64;
    // Resource root lengths include their child resources; DSINe/t's local
    // metadata lengths do not. Only DSINm's local data length changes too.
    for tag in [b"DSINe\0\0\0", b"DSINt\0\0\0", b"DSINm\0\0\0"] {
        let at = raw.windows(8).position(|w| w == tag).ok_or("missing Reaktor state resource")? + 8;
        let old = u64_at(at).filter(|n| *n <= raw.len().saturating_sub(at)).ok_or("invalid Reaktor resource size")?;
        let len = old.checked_add_signed(delta as isize).ok_or("invalid Reaktor resource size")?;
        out[at..at + 8].copy_from_slice(&(len as u64).to_le_bytes());
        if tag == b"DSINm\0\0\0" {
            let old = u64_at(at + 40).filter(|n| *n <= raw.len().saturating_sub(at + 40)).ok_or("invalid Reaktor musical resource size")?;
            let len = old.checked_add_signed(delta as isize).ok_or("invalid Reaktor musical resource size")?;
            out[at + 40..at + 48].copy_from_slice(&(len as u64).to_le_bytes());
        }
    }
    let len = out.len() as u64;
    out[..8].copy_from_slice(&len.to_le_bytes());
    // The DSINs instrument-only flag is the last byte of its local metadata.
    let s = raw.windows(8).position(|w| w == b"DSINs\0\0\0").ok_or("missing Reaktor snapshot scope")?;
    let local_len = s.checked_sub(8).and_then(u64_at).ok_or("invalid Reaktor snapshot scope")?;
    if local_len != 121 {
        return Err("unsupported Reaktor snapshot scope descriptor".into());
    }
    *out.get_mut(s + 117).ok_or("truncated Reaktor snapshot scope")? = 0;
    Ok(out)
}

fn reference(raw: &[u8]) -> Result<String, String> {
    if raw.get(12..16) != Some(b"hsin") {
        return Err("not a Reaktor NISound snapshot".into());
    }
    let marker: Vec<u8> = "file://".encode_utf16().flat_map(u16::to_le_bytes).collect();
    // The locator precedes the sound metadata; never inspect the compressed snapshot.
    let head = &raw[..raw.len().min(4096)];
    let at = head.windows(marker.len()).position(|w| w == marker).ok_or("Reaktor snapshot has no ensemble reference")?;
    let len = at
        .checked_sub(4)
        .and_then(|i| head.get(i..at))
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
        .filter(|n| (7..=2048).contains(n))
        .ok_or("invalid Reaktor ensemble reference length")?;
    let bytes = head.get(at..at + len * 2).ok_or("truncated Reaktor ensemble reference")?;
    let units: Vec<_> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    let text = String::from_utf16(&units).map_err(|_| "invalid Reaktor ensemble reference")?;
    decode_uri(&text[7..])
}

fn decode_uri(s: &str) -> Result<String, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'%' {
            let pair = s.as_bytes().get(i + 1..i + 3).ok_or("invalid escape in ensemble reference")?;
            let hex = std::str::from_utf8(pair).map_err(|_| "invalid escape in ensemble reference")?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| "invalid escape in ensemble reference")?);
            i += 3;
        } else {
            out.push(s.as_bytes()[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "invalid UTF-8 ensemble path".into())
}

fn resolve(reference: &str, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let reference = reference.strip_prefix("localhost/").unwrap_or(reference);
    let path = PathBuf::from(reference.replace('\\', "/"));
    if path.is_absolute() && path.is_file() {
        return Ok(path);
    }
    let mut matches = Vec::new();
    for root in roots {
        let candidate = root.join(&path);
        if candidate.is_file() && !matches.contains(&candidate) {
            matches.push(candidate);
        }
    }
    // A moved user ensemble may retain its previous absolute locator. Relocate by
    // exact filename under registered content roots, refusing ambiguous matches.
    if matches.is_empty()
        && let Some(file) = path.file_name()
    {
        for root in roots {
            let candidate = root.join(file);
            if candidate.is_file() && !matches.contains(&candidate) {
                matches.push(candidate);
            }
        }
    }
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(format!("Reaktor ensemble not found: {reference}")),
        _ => Err(format!("ambiguous Reaktor ensemble reference: {reference}")),
    }
}

fn content_roots() -> Vec<PathBuf> {
    let mut roots = registered_roots();
    let public = PathBuf::from(std::env::var_os("PUBLIC").unwrap_or_else(|| "C:/Users/Public".into())).join("Documents");
    // Defaults supplement registry entries, including user ensembles with absolute locators.
    roots.push(public.join("Reaktor Factory Library"));
    if let Some(user) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let ni = PathBuf::from(user).join("Documents/Native Instruments");
        roots.push(ni.join("Reaktor 6/Ensembles"));
        roots.push(ni.join("User Content/Reaktor"));
    }
    roots.sort();
    roots.dedup();
    roots
}

/// NI's registered content folders (`ContentDir` under each product's key).
fn registered_roots() -> Vec<PathBuf> {
    use windows_sys::Win32::System::Registry::*;
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    let mut roots = Vec::new();
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let mut base = std::ptr::null_mut();
        if unsafe { RegOpenKeyExW(hive, wide("SOFTWARE\\Native Instruments").as_ptr(), 0, KEY_READ, &mut base) } != 0 {
            continue;
        }
        let mut index = 0;
        loop {
            let mut name = [0u16; 512];
            let mut len = name.len() as u32;
            let result = unsafe {
                RegEnumKeyExW(
                    base,
                    index,
                    name.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if result == 259 {
                break;
            }
            index += 1;
            if result != 0 {
                continue;
            }
            let mut key = std::ptr::null_mut();
            if unsafe { RegOpenKeyExW(base, name.as_ptr(), 0, KEY_READ, &mut key) } != 0 {
                continue;
            }
            let mut path = [0u16; 4096];
            let mut size = (path.len() * 2) as u32;
            let result = unsafe {
                RegGetValueW(
                    key,
                    std::ptr::null(),
                    wide("ContentDir").as_ptr(),
                    RRF_RT_REG_SZ,
                    std::ptr::null_mut(),
                    path.as_mut_ptr().cast(),
                    &mut size,
                )
            };
            unsafe {
                RegCloseKey(key);
            }
            if result == 0 {
                let len = path.iter().position(|c| *c == 0).unwrap_or(path.len());
                if len > 0 {
                    roots.push(PathBuf::from(String::from_utf16_lossy(&path[..len])));
                }
            }
        }
        unsafe {
            RegCloseKey(base);
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locators_decode_relative_and_absolute_paths() {
        for path in ["file://Prism.rkplr", "file://Synthesizers/Sound%20School%20Analog.ens", "file://localhost/C:/My%20Ensembles/Any.ens"]
        {
            let mut raw = vec![0; 24];
            raw[12..16].copy_from_slice(b"hsin");
            raw.extend_from_slice(&(path.encode_utf16().count() as u32).to_le_bytes());
            raw.extend(path.encode_utf16().flat_map(u16::to_le_bytes));
            assert_eq!(reference(&raw).unwrap(), path[7..].replace("%20", " "));
            raw.pop();
            assert!(reference(&raw).is_err());
        }
    }

    /// A Reaktor state in miniature: the header, the host's settings ending in their `BEER`
    /// block (bank and program set), a mode-0 snapshot item, and a tail.
    fn state(context: &[u8], musical: &[u8]) -> Vec<u8> {
        let mut program = Vec::new();
        for v in [633u32, 0, 0, 1, 1, 1, 24] {
            program.extend_from_slice(&v.to_le_bytes());
        }
        program.extend_from_slice(&[7; 24]);
        program.extend_from_slice(context);
        program.extend_from_slice(&(musical.len() as u32).to_le_bytes());
        program.extend_from_slice(musical);
        let mut item = vec![0; 8];
        item.extend_from_slice(&1u32.to_le_bytes());
        item.extend_from_slice(b"hsin");
        item.resize(40, 0);
        let m = item.len();
        item.extend_from_slice(b"DSINm\0\0\0");
        item.resize(m + 112, 0);
        for v in [633u32, 0, program.len() as u32] {
            item.extend_from_slice(&v.to_le_bytes());
        }
        item.extend_from_slice(&program);
        let n = item.len() as u64;
        item[..8].copy_from_slice(&n.to_le_bytes());
        let mut payload = vec![3; 40];
        payload.extend_from_slice(b"BEER");
        payload.extend_from_slice(&[9; 60]);
        payload.extend_from_slice(&(item.len() as u32).to_le_bytes());
        payload.extend_from_slice(&item);
        payload.extend_from_slice(b"TAIL");
        let mut out = vec![1];
        out.extend_from_slice(b"4RIN");
        out.extend_from_slice(&17u32.to_le_bytes());
        out.extend_from_slice(&[0, 1]);
        out.extend_from_slice(b"CSAR");
        out.extend_from_slice(&6u32.to_le_bytes());
        out.push(0);
        out.extend_from_slice(&0u32.to_le_bytes());
        for (text, after) in [("C:/Ens/Any.ens", 4), ("Any", 8), ("C:/Ens", 4)] {
            out.extend_from_slice(&(text.len() as u32).to_le_bytes());
            out.extend_from_slice(text.as_bytes());
            out.extend(std::iter::repeat_n(0, after));
        }
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&payload);
        out
    }

    #[test]
    fn a_snapshot_goes_where_reaktors_own_was() {
        let context = (0..220).map(|i| i as u8).collect::<Vec<_>>();
        let own = state(&context, b"OWN MUSICAL STATE");
        let frame = Frame::parse(&own).unwrap();
        assert_eq!(frame.context, context, "the context between the map and the snapshot");
        assert_eq!(frame.tail, b"TAIL");
        // The bank and program it last recalled are cleared, the rest of the prefix kept.
        let beer = frame.prefix.windows(4).position(|w| w == b"BEER").unwrap();
        assert_eq!(&frame.prefix[..beer], &[3; 40]);
        assert!(frame.prefix[beer + 8..beer + 16].iter().chain(&frame.prefix[beer + 48..beer + 60]).all(|&b| b == 0));
        assert_eq!(frame.prefix[beer + 20], 9);
        // Put back, Reaktor's own snapshot gives the state as it was, bar those fields.
        let n = u32::from_le_bytes(own[frame.header.len() + 4 + frame.prefix.len()..][..4].try_into().unwrap()) as usize;
        let at = frame.header.len() + 8 + frame.prefix.len();
        let rebuilt = frame.with(&own[at..at + n]);
        assert_eq!(rebuilt.len(), own.len());
        assert_eq!(Frame::parse(&rebuilt).unwrap().tail, b"TAIL");
        // Another snapshot: the lengths follow it.
        let other = frame.with(b"x");
        assert_eq!(&other[frame.header.len()..][..4], &((frame.prefix.len() + 4 + 1 + 4) as u32).to_le_bytes());
    }

    #[test]
    fn a_state_it_doesnt_know_is_refused() {
        let own = state(&[0; 220], b"M");
        for cut in [0, 1, 20, 60, own.len() - 10] {
            assert!(Frame::parse(&own[..cut]).is_err());
        }
        let mut no_beer = own.clone();
        let b = no_beer.windows(4).position(|w| w == b"BEER").unwrap();
        no_beer[b] = b'X';
        assert!(Frame::parse(&no_beer).is_err());
    }

    #[test]
    fn missing_ensemble_is_an_error_instead_of_a_silent_instrument() {
        assert!(resolve("Missing.ens", &[]).unwrap_err().contains("not found"));
        assert!(reference(b"not a snapshot").is_err());
        assert!(decode_uri("bad%QZ.ens").is_err());
    }

    #[test]
    #[ignore = "local Reaktor presets and live Reaktor 6 VST3"]
    fn local_presets_keep_their_musical_snapshot_and_cera_pages() {
        let base = PathBuf::from(std::env::var_os("USERPROFILE").unwrap()).join("Documents/Native Instruments/User Content");
        for rel in [
            "Prism/PLUCK-Cera.nrkt",
            "Monark/BASS-A Train.nrkt",
            "Kontour/SYNTH-Ground.nrkt",
            "Reaktor/SoundSchool Analog/SYNTH-Britz.nrkt",
        ] {
            let raw = std::fs::read(base.join(rel)).unwrap();
            let converted = instrument_snapshot_as_ensemble(&raw, &[0; 220]).unwrap();
            let m = raw.windows(8).position(|w| w == b"DSINm\0\0\0").unwrap();
            let mode = u32::from_le_bytes(raw[m + 116..m + 120].try_into().unwrap());
            if mode == 0 {
                assert_eq!(converted, raw);
            } else {
                let count = u32::from_le_bytes(raw[m + 132..m + 136].try_into().unwrap()) as usize;
                let len = u32::from_le_bytes(raw[m + 120..m + 124].try_into().unwrap()) as usize;
                let snapshot = &raw[m + 124 + 16 + 8 * count..m + 124 + len];
                assert!(converted.windows(snapshot.len()).any(|w| w == snapshot));
            }
            if rel.starts_with("Prism/") {
                let entry = crate::scan::Entry {
                    kind: crate::scan::Kind::Vst3,
                    vendor: "Native Instruments".into(),
                    name: "Reaktor 6".into(),
                    instrument: true,
                    location: String::new(),
                    cid: "4E5453565269723665616B746F722036".into(),
                };
                let resolved = crate::nks::native_preset("nrkt", &raw, &[entry]).unwrap().unwrap();
                assert_eq!(resolved.name, "Prism");
                assert_eq!(resolved.pages.len(), 19);
                assert_eq!(resolved.pages[0][0].name, "Cabinet");
            }
            for len in 0..raw.len() {
                let _ = instrument_snapshot_as_ensemble(&raw[..len], &[0; 220]);
            }
        }
    }

    /// Monark snapshots (`.nrkt`) load into Reaktor 6 with their ensemble: named "Monark",
    /// and Reaktor, given the state, has the snapshot (its name, which Monark's default
    /// snapshot doesn't have) and not its default. (Reaktor saves a snapshot it was given
    /// longer than it came, so its bytes aren't compared.)
    #[test]
    #[ignore = "local Monark snapshots and live Reaktor 6 VST3"]
    fn nrkt_snapshots_load_into_their_ensemble() {
        let dir = PathBuf::from(std::env::var_os("USERPROFILE").unwrap()).join("Documents/Native Instruments/User Content/Monark");
        let monark = Path::new("C:/Users/Public/Documents/Monark/Monark.ens");
        let reaktor = crate::scan::Entry {
            kind: crate::scan::Kind::Vst3,
            vendor: "Native Instruments".into(),
            name: "Reaktor 6".into(),
            instrument: true,
            location: String::new(),
            cid: "5653544E6952367265616B746F722036".into(),
        };
        let installed = std::slice::from_ref(&reaktor);
        let snapshots = [("BASS-A Train.nrkt", "A Train"), ("PULSE-16.nrkt", "Lil French")];
        if !monark.is_file() || snapshots.iter().any(|(file, _)| !dir.join(file).is_file()) {
            return;
        }
        let saved_state = |chunk: &[u8], id: &str| {
            let mut host = crate::Host::default();
            let plugin = host.instance(crate::Slot::Sound { group: 0, sound: 0 }, id, "Reaktor 6", chunk, 48_000.0).unwrap();
            plugin.lock().unwrap().state().component
        };
        let default = crate::nks::native_preset_file(monark, None, false, installed).unwrap().unwrap();
        let default = saved_state(&default.chunk, &default.id);
        let mut states = Vec::new();
        for (file, title) in snapshots {
            let resolved = crate::nks::native_preset_file(&dir.join(file), None, false, installed).unwrap().unwrap();
            assert_eq!(resolved.name, "Monark");
            let saved = saved_state(&resolved.chunk, &resolved.id);
            assert!(saved.windows(title.len()).any(|w| w == title.as_bytes()), "{file}: Reaktor has the snapshot {title:?}");
            assert!(!default.windows(title.len()).any(|w| w == title.as_bytes()), "not Monark's default");
            assert_ne!(saved, default);
            states.push(saved);
        }
        assert_ne!(states[0], states[1]);
    }
}
