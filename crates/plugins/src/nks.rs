//! Native Kontrol Standard presets (`.nksf`, `.nksfx`), which most of Maschine's library
//! of instrument and effect presets are: a RIFF form `NIKS` holding, after a version
//! number each, the plug-in they are for (`PLID`, MessagePack) and its state (`PCHK`).
//!
//! `PLID` names the plug-in by its VST3 class (`VST3.uid`, four 32-bit numbers) or by its
//! VST2 unique id (`VST.magic`), the latter hosted through the VST3 version as a project's
//! are ([`crate::state`]). `PCHK` holds a `.vstpreset` or the plug-in's bare state.

use crate::module::{com_order, same_class};
use crate::scan::{Entry, Kind};
use crate::state::{Format, State, hex16, vst2_cid, wrap_chunk, wrap_typed};

/// One of the eight knob slots of an NKS page (`NICA` › `ni8`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Knob {
    /// The plug-in's parameter id, or `None` when the knob slot is empty.
    pub id: Option<u32>,
    pub name: String,
    pub section: String,
}

/// What a preset says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preset {
    /// The VST3 class, in the standard order.
    pub vst3: Option<[u8; 16]>,
    /// The VST2 unique id.
    pub vst2: Option<u32>,
    pub name: Option<String>,
    /// The plug-in's state.
    pub state: Vec<u8>,
    /// Controller pages (`NICA` › `ni8`), up to eight knobs each.
    pub pages: Vec<Vec<Knob>>,
}

/// A preset as a project stores a plug-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Maschine's `plugin_id`.
    pub id: String,
    pub name: String,
    pub chunk: Vec<u8>,
    pub pages: Vec<Vec<Knob>>,
}

/// Read a preset file.
pub fn parse(bytes: &[u8]) -> Result<Preset, String> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"NIKS") {
        return Err("not an NKS preset".into());
    }
    let (mut plid, mut pchk, mut nica) = (None, None, None);
    let mut at = 12usize;
    while let Some(head) = bytes.get(at..at + 8) {
        let n = u32::from_le_bytes(head[4..].try_into().unwrap()) as usize;
        let body = at.checked_add(8).and_then(|from| bytes.get(from..from.checked_add(n)?)).ok_or("the preset is truncated")?;
        match &head[..4] {
            b"PLID" => plid = Some(body),
            b"PCHK" => pchk = Some(body),
            b"NICA" => nica = Some(body),
            _ => {}
        }
        at += 8 + n + (n & 1);
    }
    // Each starts with a version number.
    let after_version = |body: &'static str, b: Option<&'_ [u8]>| -> Result<Vec<u8>, String> {
        Ok(b.ok_or_else(|| format!("the preset has no {body}"))?
            .get(4..)
            .ok_or_else(|| format!("the preset's {body} is too short"))?
            .to_vec())
    };
    let plid = after_version("plug-in id", plid)?;
    let state = after_version("state", pchk)?;
    let pages = nica.and_then(|b| b.get(4..)).map_or_else(Vec::new, parse_nica);
    let Mp::Map(fields) = (Reader { b: &plid, at: 0 }).value(0)? else { return Err("the preset's plug-in id isn't a map".into()) };
    let field = |name: &str| fields.iter().find(|(k, _)| k == name).map(|(_, v)| v);
    let vst3 = match field("VST3.uid") {
        Some(Mp::List(words)) if words.len() == 4 => {
            let mut cid = [0u8; 16];
            for (out, word) in cid.chunks_exact_mut(4).zip(words) {
                let Mp::Int(word) = word else { return Err("the preset's VST3 class isn't numbers".into()) };
                out.copy_from_slice(&(*word as u32).to_be_bytes());
            }
            Some(cid).filter(|c| *c != [0u8; 16])
        }
        _ => None,
    };
    let vst2 = match field("VST.magic") {
        Some(Mp::Int(magic)) => Some(*magic as u32).filter(|m| *m != 0),
        _ => None,
    };
    let name = match field("pluginName") {
        Some(Mp::Str(name)) if !name.is_empty() => Some(name.clone()),
        _ => None,
    };
    Ok(Preset { vst3, vst2, name, state, pages })
}

/// Controller assignments (`NICA` after its version number): `{"ni8": [[knob, ...], ...]}`.
fn parse_nica(b: &[u8]) -> Vec<Vec<Knob>> {
    let Ok(Mp::Map(fields)) = (Reader { b, at: 0 }).value(0) else { return Vec::new() };
    let Some((_, Mp::List(pages))) = fields.into_iter().find(|(k, _)| k == "ni8") else { return Vec::new() };
    pages
        .into_iter()
        .filter_map(|p| match p {
            Mp::List(knobs) => Some(
                knobs
                    .into_iter()
                    .take(8)
                    .map(|k| match k {
                        Mp::Map(m) => {
                            let mut knob = Knob::default();
                            for (key, val) in m {
                                match (key.as_str(), val) {
                                    ("id", Mp::Int(id)) if (0..u32::MAX as i64).contains(&id) => knob.id = Some(id as u32),
                                    ("name", Mp::Str(s)) => knob.name = s,
                                    ("section", Mp::Str(s)) => knob.section = s,
                                    _ => {}
                                }
                            }
                            knob
                        }
                        _ => Knob::default(),
                    })
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

impl Preset {
    /// This preset for one of the `installed` plug-ins.
    pub fn resolve(&self, installed: &[Entry]) -> Result<Resolved, String> {
        let classes = || installed.iter().filter(|e| e.kind == Kind::Vst3).filter_map(|e| Some((e, hex16(&e.cid)?)));
        if let Some(wanted) = self.vst3 {
            let found = classes().find(|(_, cid)| same_class(cid, &wanted));
            let (e, cid) = found.ok_or_else(|| format!("{} isn't installed as VST3", self.plugin_name()))?;
            let chunk = if self.state.starts_with(b"VST3") {
                wrap_typed(1, &self.state)
            } else {
                wrap_chunk(&bare_state(&self.state), Format::Vst3, &cid)
            };
            let id = e.maschine_id().ok_or("the plug-in has no id")?;
            return Ok(Resolved { id, name: e.name.clone(), chunk, pages: self.pages.clone() });
        }
        let magic = self.vst2.ok_or("the preset doesn't say which plug-in it is for")?;
        // The VST3 version: Steinberg's ("VST", the VST2 id, the name in lower case) or
        // Arturia's ("ArtuAVIS", the VST2 id, "Proc"), or by plug-in name.
        let code = magic.to_be_bytes();
        let versions: Vec<_> = classes()
            .filter(|(e, cid)| {
                let standard = if cid[..3] == *b"VST" { *cid } else { com_order(cid) };
                (standard[..3] == *b"VST" && standard[3..7] == code)
                    || (standard[8..12] == code && &standard[12..16] == b"Proc")
                    || self.name.as_ref().is_some_and(|n| n.eq_ignore_ascii_case(&e.name))
            })
            .collect();
        let named = versions.iter().find(|(e, _)| self.name.as_ref().is_some_and(|n| n.eq_ignore_ascii_case(&e.name)));
        let (e, cid) = named.or(versions.first()).ok_or_else(|| {
            format!("{} (VST2 id {}) has no VST3 version installed", self.plugin_name(), String::from_utf8_lossy(&code).escape_default())
        })?;
        let state = bare_state(&self.state);
        if same_class(&vst2_cid(magic, &e.name), cid) {
            let role = if e.instrument { "INSTR" } else { "FX" };
            return Ok(Resolved {
                id: format!("{}:VST:{role}:{magic}", e.vendor),
                name: e.name.clone(),
                chunk: wrap_chunk(&state, Format::Vst2, cid),
                pages: self.pages.clone(),
            });
        }
        let chunk = if self.state.starts_with(b"VST3") { wrap_typed(1, &self.state) } else { wrap_chunk(&state, Format::Vst3, cid) };
        let id = e.maschine_id().ok_or("the plug-in has no id")?;
        Ok(Resolved { id, name: e.name.clone(), chunk, pages: self.pages.clone() })
    }

    fn plugin_name(&self) -> &str {
        self.name.as_deref().unwrap_or("The preset's plug-in")
    }
}

/// Bare `PCHK` bytes as a [`State`]: either `[u64 component_len][u64 controller_len]`
/// followed by both streams (Arturia's NKS presets), or the component stream alone.
fn bare_state(bytes: &[u8]) -> State {
    if bytes.len() >= 16 {
        let comp = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
        let cont = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
        if comp > 0 && 16usize.checked_add(comp).and_then(|n| n.checked_add(cont)) == Some(bytes.len()) {
            return State { component: bytes[16..16 + comp].to_vec(), controller: bytes[16 + comp..].to_vec() };
        }
    }
    State { component: bytes.to_vec(), controller: Vec::new() }
}

/// A preset in a plug-in's own file format (`.nfm8`, `.nmsv`, `.nrkt`, `.rkplr`,
/// `.ens`), hosted through its VST3 plug-in.
pub fn native_preset(ext: &str, bytes: &[u8], installed: &[Entry]) -> Option<Result<Resolved, String>> {
    let plugin = match ext {
        "nfm8" => "FM8",
        "nmsv" => "Massive",
        "nrkt" | "rkplr" | "ens" => "Reaktor 6",
        _ => return None,
    };
    let resolve = || -> Result<Resolved, String> {
        let e = installed
            .iter()
            .find(|e| e.kind == Kind::Vst3 && e.name.eq_ignore_ascii_case(plugin))
            .ok_or_else(|| format!("{plugin} isn't installed as VST3"))?;
        let cid = hex16(&e.cid).ok_or("the plug-in has no class id")?;
        let id = e.maschine_id().ok_or("the plug-in has no id")?;
        let state = State { component: bytes.to_vec(), controller: Vec::new() };
        let chunk = wrap_chunk(&state, Format::Vst3, &cid);
        Ok(Resolved { id, name: e.name.clone(), chunk, pages: Vec::new() })
    };
    Some(resolve())
}

/// A MessagePack value, as far as `PLID` needs one.
enum Mp {
    Int(i64),
    Str(String),
    List(Vec<Mp>),
    Map(Vec<(String, Mp)>),
    Other,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|end| *end <= self.b.len()).ok_or("the preset's plug-in id is truncated")?;
        let out = &self.b[self.at..end];
        self.at = end;
        Ok(out)
    }

    /// An unsigned big-endian number of `n` bytes.
    fn number(&mut self, n: usize) -> Result<u64, String> {
        Ok(self.take(n)?.iter().fold(0u64, |v, b| (v << 8) | *b as u64))
    }

    fn text(&mut self, n: usize) -> Result<Mp, String> {
        Ok(Mp::Str(String::from_utf8_lossy(self.take(n)?).into_owned()))
    }

    fn skip(&mut self, n: usize) -> Result<Mp, String> {
        self.take(n)?;
        Ok(Mp::Other)
    }

    fn list(&mut self, n: usize, depth: u8) -> Result<Mp, String> {
        // (Not sized ahead: a truncated file can claim any length.)
        let mut out = Vec::new();
        for _ in 0..n {
            out.push(self.value(depth + 1)?);
        }
        Ok(Mp::List(out))
    }

    fn map(&mut self, n: usize, depth: u8) -> Result<Mp, String> {
        let mut out = Vec::new();
        for _ in 0..n {
            let key = match self.value(depth + 1)? {
                Mp::Str(key) => key,
                _ => String::new(),
            };
            out.push((key, self.value(depth + 1)?));
        }
        Ok(Mp::Map(out))
    }

    fn value(&mut self, depth: u8) -> Result<Mp, String> {
        if depth > 8 {
            return Err("the preset's plug-in id is nested too deeply".into());
        }
        let t = self.take(1)?[0];
        match t {
            0x00..=0x7f => Ok(Mp::Int(t as i64)),
            0xe0..=0xff => Ok(Mp::Int(t as i8 as i64)),
            0x80..=0x8f => self.map((t & 0x0f) as usize, depth),
            0x90..=0x9f => self.list((t & 0x0f) as usize, depth),
            0xa0..=0xbf => self.text((t & 0x1f) as usize),
            0xc0 | 0xc2 | 0xc3 => Ok(Mp::Other),
            0xc4..=0xc6 => {
                let n = self.number(1 << (t - 0xc4))? as usize;
                self.skip(n)
            }
            0xc7..=0xc9 => {
                let n = self.number(1 << (t - 0xc7))? as usize;
                self.skip(n.saturating_add(1))
            }
            0xca => self.skip(4),
            0xcb => self.skip(8),
            0xcc..=0xcf => Ok(Mp::Int(self.number(1 << (t - 0xcc))? as i64)),
            0xd0 => Ok(Mp::Int(self.number(1)? as u8 as i8 as i64)),
            0xd1 => Ok(Mp::Int(self.number(2)? as u16 as i16 as i64)),
            0xd2 => Ok(Mp::Int(self.number(4)? as u32 as i32 as i64)),
            0xd3 => Ok(Mp::Int(self.number(8)? as i64)),
            0xd4..=0xd8 => self.skip((1usize << (t - 0xd4)) + 1),
            0xd9..=0xdb => {
                let n = self.number(1 << (t - 0xd9))? as usize;
                self.text(n)
            }
            0xdc | 0xdd => {
                let n = self.number(2 << (t - 0xdc))? as usize;
                self.list(n, depth)
            }
            0xde | 0xdf => {
                let n = self.number(2 << (t - 0xde))? as usize;
                self.map(n, depth)
            }
            0xc1 => Err("the preset's plug-in id isn't valid".into()),
        }
    }
}

/// A preset file, for tests.
#[doc(hidden)]
pub fn fixture(uid: Option<[u32; 4]>, magic: u32, name: &str, state: &[u8]) -> Vec<u8> {
    fixture_with_pages(uid, magic, name, state, &[])
}

/// [`fixture`] with `NICA` controller pages (`pages`: each page's knobs).
#[doc(hidden)]
pub fn fixture_with_pages(uid: Option<[u32; 4]>, magic: u32, name: &str, state: &[u8], pages: &[Vec<Knob>]) -> Vec<u8> {
    let key = |out: &mut Vec<u8>, s: &str| {
        out.push(0xa0 | s.len() as u8);
        out.extend(s.as_bytes());
    };
    let mut plid = 1u32.to_le_bytes().to_vec();
    plid.push(0x80 | 2 + uid.is_some() as u8);
    key(&mut plid, "VST.magic");
    plid.push(0xce);
    plid.extend(magic.to_be_bytes());
    key(&mut plid, "pluginName");
    key(&mut plid, name);
    if let Some(uid) = uid {
        key(&mut plid, "VST3.uid");
        plid.push(0x94);
        for word in uid {
            plid.push(0xce);
            plid.extend(word.to_be_bytes());
        }
    }
    let mut pchk = 1u32.to_le_bytes().to_vec();
    pchk.extend(state);
    let mut nica = 1u32.to_le_bytes().to_vec();
    nica.push(0x81);
    key(&mut nica, "ni8");
    nica.push(0x90 | pages.len().min(15) as u8);
    for pg in pages.iter().take(15) {
        nica.push(0x90 | pg.len().min(8) as u8);
        for k in pg.iter().take(8) {
            let fields = k.id.is_some() as u8 + !k.name.is_empty() as u8 + !k.section.is_empty() as u8;
            nica.push(0x80 | fields);
            if let Some(id) = k.id {
                key(&mut nica, "id");
                nica.push(0xce);
                nica.extend(id.to_be_bytes());
            }
            if !k.name.is_empty() {
                key(&mut nica, "name");
                key(&mut nica, &k.name);
            }
            if !k.section.is_empty() {
                key(&mut nica, "section");
                key(&mut nica, &k.section);
            }
        }
    }
    let mut body = b"NIKS".to_vec();
    for (id, chunk) in [(b"NISI", &b"\x01\0\0\0\x80"[..]), (b"NICA", &nica[..]), (b"PLID", &plid[..]), (b"PCHK", &pchk[..])] {
        body.extend(id);
        body.extend((chunk.len() as u32).to_le_bytes());
        body.extend(chunk);
        if chunk.len() & 1 == 1 {
            body.push(0);
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{chunk_parts, parse_vstpreset};

    fn vst3(vendor: &str, name: &str, instrument: bool, cid: &str) -> Entry {
        Entry { kind: Kind::Vst3, vendor: vendor.into(), name: name.into(), instrument, location: String::new(), cid: cid.into() }
    }

    /// Massive X, as Windows reports it: its class in COM's order, its VST2 id `Ni$H`.
    const MASSIVE_X: &str = "4E54535624696D486173736976652078";
    const NI_H: u32 = u32::from_be_bytes(*b"Ni$H");

    fn massive_x() -> Entry {
        vst3("Native Instruments", "Massive X", true, MASSIVE_X)
    }

    #[test]
    fn it_reads_what_a_preset_says() {
        let uid = [0x56535446, 0x55434B69, 0x6E66696C, 0x74726174];
        let p = parse(&fixture(Some(uid), 0, "Infiltrator", b"odd")).unwrap();
        assert_eq!(p.vst3.as_ref().map(|c| &c[..8]), Some(&b"VSTFUCKi"[..]));
        assert_eq!(p.vst2, None, "0 is no id");
        assert_eq!((p.name.as_deref(), &p.state[..]), (Some("Infiltrator"), &b"odd"[..]));
        let p = parse(&fixture(None, NI_H, "x", b"state")).unwrap();
        assert_eq!((p.vst3, p.vst2), (None, Some(NI_H)));
    }

    #[test]
    fn a_vst2_id_finds_the_vst3_version() {
        let p = parse(&fixture(None, NI_H, "Other Name", b"state")).unwrap();
        let r = p.resolve(&[vst3("V", "Else", true, "000102030405060708090A0B0C0D0E0F"), massive_x()]).unwrap();
        assert_eq!(r.id, format!("Native Instruments:VST:INSTR:{NI_H}"));
        assert_eq!(r.name, "Massive X");
        assert_eq!(chunk_parts(&r.chunk).unwrap(), (1, &b"state"[..]));
        // As a project's VST2 plug-in, this id is hosted through that same class.
        let id = crate::state::PluginId::parse(&r.id, &r.name).unwrap();
        assert!(same_class(&id.cid, &hex16(MASSIVE_X).unwrap()));
    }

    #[test]
    fn a_vst3_class_matches_in_either_order() {
        let cid = hex16(MASSIVE_X).unwrap();
        let standard = com_order(&cid);
        let words: Vec<u32> = standard.chunks_exact(4).map(|w| u32::from_be_bytes(w.try_into().unwrap())).collect();
        let uid = [words[0], words[1], words[2], words[3]];
        let installed = [massive_x()];
        // A .vstpreset is kept as it is.
        let preset = crate::state::write_vstpreset(&State { component: b"c".to_vec(), controller: b"d".to_vec() }, &cid);
        let r = parse(&fixture(Some(uid), 0, "Massive X", &preset)).unwrap().resolve(&installed).unwrap();
        assert_eq!(r.id, format!("Native Instruments:VST3:INSTR:{}", MASSIVE_X));
        assert_eq!(chunk_parts(&r.chunk).unwrap().1, &preset[..]);
        // Bare state becomes one, as the plug-in's component state.
        let r = parse(&fixture(Some(uid), 0, "Massive X", b"bare")).unwrap().resolve(&installed).unwrap();
        let state = parse_vstpreset(chunk_parts(&r.chunk).unwrap().1).unwrap();
        assert_eq!((&state.component[..], state.controller.len()), (&b"bare"[..], 0));
    }

    #[test]
    fn what_cant_be_found_says_so() {
        let e = parse(&fixture(Some([1, 2, 3, 4]), 0, "Omnisphere", b"s")).unwrap().resolve(&[massive_x()]).unwrap_err();
        assert_eq!(e, "Omnisphere isn't installed as VST3");
        let e = parse(&fixture(None, u32::from_be_bytes(*b"KaK1"), "Pigments", b"s")).unwrap().resolve(&[massive_x()]).unwrap_err();
        assert!(e.starts_with("Pigments (VST2 id KaK1)"), "{e}");
        assert!(parse(&fixture(None, 0, "x", b"s")).unwrap().resolve(&[massive_x()]).is_err());
    }

    #[test]
    fn a_damaged_file_is_an_error() {
        assert!(parse(b"").is_err());
        assert!(parse(b"RIFF\0\0\0\0WAVE").is_err());
        let good = fixture(None, NI_H, "x", b"state");
        for n in 12..good.len() - 1 {
            // Cut anywhere: an error or a shorter state, never a panic.
            let _ = parse(&good[..n]);
        }
        let mut huge = good.clone();
        huge[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&huge).is_err());
        // A nested value that never ends.
        let mut plid = 1u32.to_le_bytes().to_vec();
        plid.extend([0x91u8; 64]);
        let mut riff = b"RIFF\0\0\0\0NIKSPLID".to_vec();
        riff.extend((plid.len() as u32).to_le_bytes());
        riff.extend(plid);
        riff.extend(b"PCHK\x04\0\0\0\x01\0\0\0");
        assert!(parse(&riff).is_err());
    }
}
