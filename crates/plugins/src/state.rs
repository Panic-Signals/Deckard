//! What Maschine saves for a hosted plug-in, and how it maps onto a VST3 class.
//!
//! A `PluginHost` module stores:
//! - `plugin_id`: `"<vendor>:<format>:<kind>:<id>"`, e.g.
//!   `"Native Instruments:VST3:INSTR:5653544E694D616D6173736976650000"` (the class id
//!   in hex) or `"Native Instruments:VST:INSTR:1315523937"` (a VST2 unique id, decimal);
//! - the plug-in's name (`"Massive"`);
//! - `chunk`: `[u32 1][u32 1][u32 n][n bytes]`, little-endian. For VST3 the bytes are a
//!   `.vstpreset` (component state in its `Comp` entry, controller state in `Cont`); for
//!   VST2 they are what the plug-in returned for its state (`effGetChunk`).
//!
//! VST2 plug-ins are hosted through their VST3 versions, which by Steinberg's convention
//! have the class id `"VST" + <4-char unique id> + <name, lower case, 9 chars>` and read
//! their VST2 state as component state. (NI's VST2 and VST3 states are the same format.)

/// The plug-in format a Maschine plug-in id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Vst2,
    Vst3,
    AudioUnit,
}

/// A parsed `plugin_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginId {
    pub vendor: String,
    pub format: Format,
    /// "INSTR" or "FX" ("MF" for an AU's MIDI-controlled effect).
    pub kind: String,
    /// The VST3 class to host. An AU has none: see [`au_cid`].
    pub cid: [u8; 16],
}

impl PluginId {
    /// Parse Maschine's id for the plug-in `name`. `None` for formats we can't host.
    pub fn parse(id: &str, name: &str) -> Option<PluginId> {
        let mut parts = id.splitn(4, ':');
        let (vendor, format, kind, rest) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        let (format, cid) = match format {
            "VST3" => (Format::Vst3, hex16(rest)?),
            "VST" => (Format::Vst2, vst2_cid(rest.parse::<u32>().ok()?, name)),
            "AU" => (Format::AudioUnit, au_cid(au_kind(kind)?, rest.parse::<i32>().ok()? as u32, vendor)),
            _ => return None,
        };
        Some(PluginId { vendor: vendor.to_string(), format, kind: kind.to_string(), cid })
    }
}

/// The component type an AU's role in its id means.
fn au_kind(role: &str) -> Option<u32> {
    Some(u32::from_be_bytes(match role {
        "INSTR" => *b"aumu",
        "FX" => *b"aufx",
        "MF" => *b"aumf",
        _ => return None,
    }))
}

/// The role in an AU's id of a component type.
pub fn au_role(kind: u32) -> Option<&'static str> {
    match &kind.to_be_bytes() {
        b"aumu" => Some("INSTR"),
        b"aufx" => Some("FX"),
        b"aumf" => Some("MF"),
        _ => None,
    }
}

/// Maschine's id for an AU: `<vendor>:AU:<role>:<uid>`, the uid being the subtype's
/// four characters read as a signed integer.
pub fn au_id(vendor: &str, kind: u32, sub: u32) -> Option<String> {
    Some(format!("{vendor}:AU:{}:{}", au_role(kind)?, sub as i32))
}

/// What stands for an AU's class id: `"AU\0\0"`, type, subtype, and a hash of the vendor
/// (the id has no manufacturer code, and two vendors may use the same subtype).
fn au_cid(kind: u32, sub: u32, vendor: &str) -> [u8; 16] {
    let hash = vendor.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    let mut cid = [0u8; 16];
    cid[..2].copy_from_slice(b"AU");
    cid[4..8].copy_from_slice(&kind.to_be_bytes());
    cid[8..12].copy_from_slice(&sub.to_be_bytes());
    cid[12..].copy_from_slice(&hash.to_be_bytes());
    cid
}

/// The component type and subtype in what [`au_cid`] made.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn au_codes(cid: &[u8; 16]) -> (u32, u32) {
    let code = |at: usize| u32::from_be_bytes(cid[at..at + 4].try_into().unwrap());
    (code(4), code(8))
}

/// The VST3 class id of the VST3 version of a VST2 plug-in.
pub fn vst2_cid(unique_id: u32, name: &str) -> [u8; 16] {
    // Reaktor Player products (Monark, Prism) share Reaktor 6's VST2 id ("NiR6") and
    // VST3 class ("VSTNiR6reaktor 6") while keeping their own product name.
    let class_name = if &unique_id.to_be_bytes() == b"NiR6" { "reaktor 6" } else { name };
    let mut cid = [0u8; 16];
    cid[..3].copy_from_slice(b"VST");
    cid[3..7].copy_from_slice(&unique_id.to_be_bytes());
    for (d, c) in cid[7..].iter_mut().zip(class_name.to_lowercase().bytes()) {
        *d = c;
    }
    cid
}

pub(crate) fn hex16(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// What to give the plug-in: component state, and controller state if any.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub component: Vec<u8>,
    pub controller: Vec<u8>,
}

/// A `PluginHost` chunk's type and data (nothing for an empty chunk).
pub fn chunk_parts(chunk: &[u8]) -> Result<(u32, &[u8]), String> {
    if chunk.is_empty() {
        return Ok((0, &[]));
    }
    let u32_at = |at: usize| chunk.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let kind = u32_at(4).ok_or("plug-in state too short")?;
    let n = u32_at(8).ok_or("plug-in state too short")? as usize;
    Ok((kind, chunk.get(12..12 + n).ok_or("plug-in state is truncated")?))
}

/// A `PluginHost` chunk of the given type (the inverse of [`chunk_parts`]).
pub fn wrap_typed(kind: u32, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + data.len());
    out.extend(1u32.to_le_bytes());
    out.extend(kind.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}

/// Unwrap a `PluginHost` chunk saved for a plug-in of `format`.
pub fn unwrap_chunk(chunk: &[u8], format: Format) -> Result<State, String> {
    if chunk.is_empty() {
        return Ok(State::default());
    }
    let u32_at = |at: usize| chunk.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let n = u32_at(8).ok_or("plug-in state too short")? as usize;
    let data = chunk.get(12..12 + n).ok_or("plug-in state is truncated")?;
    if data.is_empty() {
        // An empty state (Maschine writes one for a plug-in it never ran).
        return Ok(State::default());
    }
    match format {
        Format::Vst3 => parse_vstpreset(data),
        _ => Ok(State { component: data.to_vec(), controller: Vec::new() }),
    }
}

/// Wrap a plug-in's state as Maschine saves it for a plug-in of `format` with class
/// `cid` (the inverse of [`unwrap_chunk`]).
pub fn wrap_chunk(state: &State, format: Format, cid: &[u8; 16]) -> Vec<u8> {
    let data = match format {
        Format::Vst3 => write_vstpreset(state, cid),
        _ => state.component.clone(),
    };
    let mut out = Vec::with_capacity(12 + data.len());
    out.extend(1u32.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}

/// A `.vstpreset` holding `state` for class `cid`: header, `Comp`, `Cont`, then the
/// chunk list, as Maschine writes them.
pub fn write_vstpreset(state: &State, cid: &[u8; 16]) -> Vec<u8> {
    let hex: String = cid.iter().map(|b| format!("{b:02X}")).collect();
    let comp_at = 48usize;
    let cont_at = comp_at + state.component.len();
    let list_at = cont_at + state.controller.len();
    let mut p = Vec::with_capacity(list_at + 48);
    p.extend(b"VST3");
    p.extend(1i32.to_le_bytes());
    p.extend(hex.as_bytes());
    p.extend((list_at as i64).to_le_bytes());
    p.extend(&state.component);
    p.extend(&state.controller);
    p.extend(b"List");
    p.extend(2i32.to_le_bytes());
    for (id, at, size) in [(b"Comp", comp_at, state.component.len()), (b"Cont", cont_at, state.controller.len())] {
        p.extend(id);
        p.extend((at as i64).to_le_bytes());
        p.extend((size as i64).to_le_bytes());
    }
    p
}

/// The `Comp` and `Cont` entries of a `.vstpreset`.
pub fn parse_vstpreset(data: &[u8]) -> Result<State, String> {
    if data.get(..4) != Some(b"VST3") {
        return Err("plug-in state is not a VST3 preset".into());
    }
    let i64_at = |at: usize| data.get(at..at + 8).map(|b| i64::from_le_bytes(b.try_into().unwrap()));
    let list = i64_at(40).ok_or("preset header too short")? as usize;
    if data.get(list..list + 4) != Some(b"List") {
        return Err("preset has no chunk list".into());
    }
    let count = data.get(list + 4..list + 8).map(|b| i32::from_le_bytes(b.try_into().unwrap())).ok_or("bad chunk list")?;
    let mut state = State::default();
    for i in 0..count.max(0) as usize {
        let e = list + 8 + i * 20;
        let id = data.get(e..e + 4).ok_or("bad chunk list")?;
        let (off, size) = (i64_at(e + 4).ok_or("bad chunk list")? as usize, i64_at(e + 12).ok_or("bad chunk list")? as usize);
        let bytes = data.get(off..off + size).ok_or("preset entry out of range")?.to_vec();
        match id {
            b"Comp" => state.component = bytes,
            b"Cont" => state.controller = bytes,
            _ => {}
        }
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_unit_ids() {
        let code = |c: &[u8; 4]| u32::from_be_bytes(*c);
        // The subtype's characters as a signed integer.
        let id = au_id("Output", code(b"aufx"), code(b"PRTL")).unwrap();
        assert_eq!(id, "Output:AU:FX:1347572812");
        let p = PluginId::parse(&id, "Portal").unwrap();
        assert_eq!((p.format, p.vendor.as_str(), p.kind.as_str()), (Format::AudioUnit, "Output", "FX"));
        assert_eq!(au_codes(&p.cid), (code(b"aufx"), code(b"PRTL")));
        // A subtype with the top bit set is negative.
        let high = au_id("V", code(b"aumu"), 0xA900_0001).unwrap();
        assert!(high.ends_with(":INSTR:-1459617791"), "{high}");
        assert_eq!(au_codes(&PluginId::parse(&high, "x").unwrap().cid).1, 0xA900_0001);
        // Two vendors with one subtype are different plug-ins.
        let other = PluginId::parse("Other:AU:FX:1347572812", "Portal").unwrap();
        assert_ne!(other.cid, p.cid);
        assert!(au_id("V", code(b"auou"), 1).is_none());
    }

    #[test]
    fn chunks_of_a_type() {
        let chunk = wrap_typed(3, b"<plist/>");
        assert_eq!(chunk_parts(&chunk).unwrap(), (3, &b"<plist/>"[..]));
        assert_eq!(chunk_parts(&[]).unwrap(), (0, &[][..]));
        assert!(chunk_parts(&chunk[..14]).is_err());
    }

    #[test]
    fn vst2_ids_map_to_vst3_classes() {
        // As in Maschine's own VST3 ids for the same plug-ins.
        let massive = PluginId::parse("Native Instruments:VST:INSTR:1315523937", "Massive").unwrap();
        assert_eq!(massive.cid, hex16("5653544E694D616D6173736976650000").unwrap());
        let reaktor = PluginId::parse("Native Instruments:VST:INSTR:1315525174", "Reaktor 6").unwrap();
        assert_eq!(reaktor.cid, hex16("5653544E6952367265616B746F722036").unwrap());
        let monark = PluginId::parse("Native Instruments:VST:INSTR:1315525174", "Monark").unwrap();
        assert_eq!(monark.cid, reaktor.cid);
        let v3 = PluginId::parse("Native Instruments:VST3:INSTR:5653544E694D616D6173736976650000", "Massive").unwrap();
        assert_eq!((v3.format, v3.cid), (Format::Vst3, massive.cid));
    }

    #[test]
    fn vstpreset_entries() {
        let mut p = b"VST3".to_vec();
        p.extend(1i32.to_le_bytes());
        p.extend(b"5653544E694D616D6173736976650000");
        let comp = b"component";
        let list = 48 + comp.len();
        p.extend((list as i64).to_le_bytes());
        p.extend(comp);
        p.extend(b"List");
        p.extend(2i32.to_le_bytes());
        p.extend(b"Comp");
        p.extend(48i64.to_le_bytes());
        p.extend((comp.len() as i64).to_le_bytes());
        p.extend(b"Cont");
        p.extend((list as i64).to_le_bytes());
        p.extend(0i64.to_le_bytes());
        let mut chunk = vec![1, 0, 0, 0, 1, 0, 0, 0];
        chunk.extend((p.len() as u32).to_le_bytes());
        chunk.extend(&p);
        let s = unwrap_chunk(&chunk, Format::Vst3).unwrap();
        assert_eq!(s.component, comp);
        assert!(s.controller.is_empty());
        let cid = hex16("5653544E694D616D6173736976650000").unwrap();
        assert_eq!(wrap_chunk(&s, Format::Vst3, &cid), chunk, "writing gives back the same bytes");
    }
}
