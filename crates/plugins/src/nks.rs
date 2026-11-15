//! Native Kontrol Standard presets (`.nksf`, `.nksfx`), which most of Maschine's library
//! of instrument and effect presets are: a RIFF form `NIKS` holding, after a version
//! number each, the plug-in they are for (`PLID`, MessagePack) and its state (`PCHK`).
//!
//! `PLID` names the plug-in by its VST3 class (`VST3.uid`, four 32-bit numbers) or by its
//! VST2 unique id (`VST.magic`), the latter hosted through the VST3 version as a project's
//! are ([`crate::state`]). `PCHK` holds a `.vstpreset` or the plug-in's bare state.
//!
//! [`write`] writes one as Maschine 3 does when it saves a VST3 plug-in's preset
//! (`ChainFileCommands::saveSlotAs` › `NKSRoundtrip::saveAs`): `NISI`, `NICA`, `PLID`,
//! `PCHK`, each `[u32 1]` then MessagePack (msgpack-c's shortest forms, maps' keys in
//! order) or, for `PCHK`, the state. NI's factory presets come out byte for byte.

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
    /// The plug-in's vendor (`pluginVendor`).
    pub vendor: Option<String>,
    /// The plug-in's state.
    pub state: Vec<u8>,
    /// Controller pages (`NICA` › `ni8`), up to eight knobs each.
    pub pages: Vec<Vec<Knob>>,
    /// What the preset says about itself (`NISI`).
    pub info: SoundInfo,
}

/// A preset's description (`NISI`): its name, where it's filed and its tags.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SoundInfo {
    pub author: String,
    /// Product, bank and sub-bank (three levels, empty where there's none).
    pub bankchain: Vec<String>,
    pub characters: Vec<String>,
    pub comment: String,
    /// `"INST"` or `"FX"`.
    pub device_type: String,
    /// Modes (a preset's playing styles, e.g. "Arp"; most have none).
    pub modes: Vec<String>,
    pub name: String,
    /// Each a type and its sub-type.
    pub types: Vec<Vec<String>>,
    /// Lower-case, hyphenated (Maschine gives each preset it saves a random one).
    pub uuid: String,
    pub vendor: String,
}

/// Which plug-in a preset is for (`PLID`): its VST3 class (standard order) or VST2 id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginIds {
    pub vst3: Option<[u8; 16]>,
    pub vst2: Option<u32>,
    pub name: String,
    pub vendor: String,
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
    let (mut plid, mut pchk, mut nica, mut nisi) = (None, None, None, None);
    let mut at = 12usize;
    while let Some(head) = bytes.get(at..at + 8) {
        let n = u32::from_le_bytes(head[4..].try_into().unwrap()) as usize;
        let body = at.checked_add(8).and_then(|from| bytes.get(from..from.checked_add(n)?)).ok_or("the preset is truncated")?;
        match &head[..4] {
            b"PLID" => plid = Some(body),
            b"PCHK" => pchk = Some(body),
            b"NICA" => nica = Some(body),
            b"NISI" => nisi = Some(body),
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
    let text = |key: &str| match field(key) {
        Some(Mp::Str(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    };
    let (name, vendor) = (text("pluginName"), text("pluginVendor"));
    let info = nisi.and_then(|b| b.get(4..)).map_or_else(SoundInfo::default, parse_nisi);
    Ok(Preset { vst3, vst2, name, vendor, state, pages, info })
}

/// A preset's description (`NISI` after its version number).
fn parse_nisi(b: &[u8]) -> SoundInfo {
    let Ok(Mp::Map(fields)) = (Reader { b, at: 0 }).value(0) else { return SoundInfo::default() };
    let text = |v: &Mp| match v {
        Mp::Str(s) => s.clone(),
        _ => String::new(),
    };
    let texts = |v: &Mp| match v {
        Mp::List(l) => l.iter().map(text).collect(),
        _ => Vec::new(),
    };
    let mut info = SoundInfo::default();
    for (key, v) in &fields {
        match key.as_str() {
            "author" => info.author = text(v),
            "bankchain" => info.bankchain = texts(v),
            "characters" => info.characters = texts(v),
            "comment" => info.comment = text(v),
            "deviceType" => info.device_type = text(v),
            "modes" => info.modes = texts(v),
            "name" => info.name = text(v),
            "types" => info.types = if let Mp::List(l) = v { l.iter().map(texts).collect() } else { Vec::new() },
            "uuid" => info.uuid = text(v),
            "vendor" => info.vendor = text(v),
            _ => {}
        }
    }
    info
}

/// An NKS preset file as Maschine writes one when it saves a plug-in's preset
/// (`NKSRoundtrip::saveAs` › `nksf::NKSWriter::v1::write`): a RIFF form `NIKS` of
/// `NISI` (`info`), `NICA` (the knob `pages`), `PLID` (`plugin`) and `PCHK` (its
/// `state`), each a version number (1) and then its data, padded to an even length.
pub fn write(info: &SoundInfo, pages: &[Vec<Knob>], plugin: &PluginIds, state: &[u8]) -> Vec<u8> {
    // NISI: every key, in order (a `std::map`); `modes` only when there are some.
    let mut nisi = Pack::default();
    nisi.map(9 + !info.modes.is_empty() as usize);
    nisi.str("author").str(&info.author);
    nisi.str("bankchain").strs(&info.bankchain);
    nisi.str("characters").strs(&info.characters);
    nisi.str("comment").str(&info.comment);
    nisi.str("deviceType").str(&info.device_type);
    if !info.modes.is_empty() {
        nisi.str("modes").strs(&info.modes);
    }
    nisi.str("name").str(&info.name);
    nisi.str("types").list(info.types.len());
    for t in &info.types {
        nisi.strs(t);
    }
    nisi.str("uuid").str(&info.uuid);
    nisi.str("vendor").str(&info.vendor);

    // NICA: `{"ni8": [page, ...]}`, each knob's keys in order, `name` and `section` only
    // when set (an empty slot is `autoname` and `vflag` alone). Every knob NI's factory
    // presets have is `autoname` true and `vflag` false.
    let mut nica = Pack::default();
    nica.map(1).str("ni8").list(pages.len());
    for page in pages {
        nica.list(page.len());
        for k in page {
            nica.map(2 + k.id.is_some() as usize + !k.name.is_empty() as usize + !k.section.is_empty() as usize);
            nica.str("autoname").bool(true);
            if let Some(id) = k.id {
                nica.str("id").uint(id.into());
            }
            if !k.name.is_empty() {
                nica.str("name").str(&k.name);
            }
            if !k.section.is_empty() {
                nica.str("section").str(&k.section);
            }
            nica.str("vflag").bool(false);
        }
    }

    // PLID: those present of `VST.magic`, `VST3.uid`, `pluginName`, `pluginVendor`.
    let mut plid = Pack::default();
    let has = |s: &str| !s.is_empty();
    plid.map(plugin.vst2.is_some() as usize + plugin.vst3.is_some() as usize + has(&plugin.name) as usize + has(&plugin.vendor) as usize);
    if let Some(magic) = plugin.vst2 {
        plid.str("VST.magic").uint(magic.into());
    }
    if let Some(cid) = plugin.vst3 {
        plid.str("VST3.uid").list(4);
        for word in cid.as_chunks::<4>().0 {
            plid.uint(u32::from_be_bytes(*word).into());
        }
    }
    if has(&plugin.name) {
        plid.str("pluginName").str(&plugin.name);
    }
    if has(&plugin.vendor) {
        plid.str("pluginVendor").str(&plugin.vendor);
    }

    let mut body = b"NIKS".to_vec();
    for (id, data) in [(b"NISI", &nisi.0[..]), (b"NICA", &nica.0[..]), (b"PLID", &plid.0[..]), (b"PCHK", state)] {
        body.extend(id);
        body.extend((4 + data.len() as u32).to_le_bytes());
        body.extend(1u32.to_le_bytes());
        body.extend(data);
        if data.len() & 1 == 1 {
            body.push(0);
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

/// A new preset's `uuid`: random (version 4), lower case and hyphenated, as Maschine
/// gives each preset it saves. The randomness is the standard library's per-process
/// random hash keys (no extra dependency for an id that only has to be unique).
pub fn new_uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let half = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(salt);
        h.write_u64(n);
        h.write_u128(now);
        h.finish()
    };
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&half(1).to_le_bytes());
    b[8..].copy_from_slice(&half(2).to_le_bytes());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// MessagePack as msgpack-c packs it: every value in its shortest form.
#[derive(Default)]
struct Pack(Vec<u8>);

impl Pack {
    fn head(&mut self, n: usize, fix: u8, max_fix: usize, wide: u8) -> &mut Self {
        if n <= max_fix {
            self.0.push(fix | n as u8);
        } else if n <= 0xffff {
            self.0.push(wide);
            self.0.extend((n as u16).to_be_bytes());
        } else {
            self.0.push(wide + 1);
            self.0.extend((n as u32).to_be_bytes());
        }
        self
    }

    fn map(&mut self, n: usize) -> &mut Self {
        self.head(n, 0x80, 15, 0xde)
    }

    fn list(&mut self, n: usize) -> &mut Self {
        self.head(n, 0x90, 15, 0xdc)
    }

    fn str(&mut self, s: &str) -> &mut Self {
        let n = s.len();
        if n < 32 {
            self.0.push(0xa0 | n as u8);
        } else if n < 256 {
            self.0.extend([0xd9, n as u8]);
        } else if n <= 0xffff {
            self.0.push(0xda);
            self.0.extend((n as u16).to_be_bytes());
        } else {
            self.0.push(0xdb);
            self.0.extend((n as u32).to_be_bytes());
        }
        self.0.extend(s.as_bytes());
        self
    }

    fn strs(&mut self, l: &[String]) -> &mut Self {
        self.list(l.len());
        for s in l {
            self.str(s);
        }
        self
    }

    fn uint(&mut self, v: u64) -> &mut Self {
        match v {
            0..=0x7f => self.0.push(v as u8),
            0x80..=0xff => self.0.extend([0xcc, v as u8]),
            0x100..=0xffff => {
                self.0.push(0xcd);
                self.0.extend((v as u16).to_be_bytes());
            }
            0x1_0000..=0xffff_ffff => {
                self.0.push(0xce);
                self.0.extend((v as u32).to_be_bytes());
            }
            _ => {
                self.0.push(0xcf);
                self.0.extend(v.to_be_bytes());
            }
        }
        self
    }

    fn bool(&mut self, b: bool) -> &mut Self {
        self.0.push(if b { 0xc3 } else { 0xc2 });
        self
    }
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

/// Why a Kontakt library file (`.nksn` snapshot, `.nki` instrument) isn't loaded.
pub const KONTAKT_FILE: &str = "Kontakt library snapshots and instruments (.nksn, .nki) can't be loaded from DECKARD's browser: Kontakt only recalls them when it opens the file itself. Load it in Kontakt's own browser, or load a Maschine sound or group saved with it.";

/// A preset in a plug-in's own file format (`.nfm8`, `.nmsv`, `.nrkt`, `.rkplr`,
/// `.ens`), hosted through its VST3 plug-in.
///
/// Kontakt library files are refused ([`KONTAKT_FILE`]). Kontakt 7 resolves a library
/// instrument's content from where its file is; given the file's bytes as its state
/// (raw, or inside the wrapper of a state Maschine saved), it can't, and stops in
/// `setState` on a modal "cannot be recalled correctly, perhaps due to missing
/// content" box, which froze the thread loading it. (Measured 2026-11-09 with Session
/// Guitarist snapshots and instruments; a state Maschine saved still loads.)
pub fn native_preset(ext: &str, bytes: &[u8], installed: &[Entry]) -> Option<Result<Resolved, String>> {
    let ext = ext.to_ascii_lowercase();
    let plugin = match ext.as_str() {
        "nfm8" => "FM8",
        "nmsv" => "Massive",
        "nksn" | "nki" => return Some(Err(KONTAKT_FILE.into())),
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
        let component = wrap_nisound_preset(&ext, bytes).unwrap_or_else(|| bytes.to_vec());
        let pages = parse_dsiny(bytes);
        let state = State { component, controller: Vec::new() };
        let chunk = wrap_chunk(&state, Format::Vst3, &cid);
        Ok(Resolved { id, name: e.name.clone(), chunk, pages })
    };
    Some(resolve())
}

fn find_tag(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Wrap a bare `.nfm8` or `.nmsv` `NISound` (`hsin`/`DSIN`) preset file in the 5-subchunk
/// (`tsrp`/`knab`/`ctni`/`siug`/`gptc`) plug-in instance state that `FM8.vst3` and
/// `Massive.vst3` expect from `IComponent::setState`. Without this wrapper, `FM8.vst3`
/// stalls for ~8 s in `setState` and both plug-ins reject the preset.
fn wrap_nisound_preset(ext: &str, raw: &[u8]) -> Option<Vec<u8>> {
    if !matches!(ext, "nfm8" | "nmsv") || raw.len() < 120 {
        return None;
    }
    let root_size = u64::from_le_bytes(raw.get(..8)?.try_into().ok()?) as usize;
    let buf = raw.get(..root_size.min(raw.len()))?;
    if buf.get(12..16) != Some(b"hsin") {
        return None;
    }
    let e_idx = find_tag(buf, b"DSINe\0\0\0")?.checked_add(8)?;
    // Real files have an outer resource tag and an inner NISound chunk with
    // the same name. Only the inner one has the adjacent compressed DSINs.
    let t_idx = buf
        .windows(8)
        .enumerate()
        .filter(|(_, w)| *w == b"DSINt\0\0\0")
        .filter_map(|(i, _)| i.checked_sub(48))
        .find(|&i| buf.get(i + 68..i + 76) == Some(b"DSINs\0\0\0"))?;
    let t_part = buf.get(t_idx..)?;
    if t_part.len() < 117 || t_part.get(48..56) != Some(b"DSINt\0\0\0") || t_part.get(68..76) != Some(b"DSINs\0\0\0") {
        return None;
    }
    let uncomp_len = u32::from_le_bytes(t_part.get(109..113)?.try_into().ok()?) as usize;
    let comp_len = u32::from_le_bytes(t_part.get(113..117)?.try_into().ok()?) as usize;
    let comp_end = 117usize.checked_add(comp_len)?;
    let comp_stream = t_part.get(117..comp_end)?;
    let t_footer = t_part.get(comp_end..)?;
    let dec = fastlz_decompress(comp_stream, uncomp_len)?;
    if dec.len() < 204 || dec.get(200..204) == Some(b"tsrp") {
        return None;
    }
    let preset_len = u64::from_le_bytes(dec.get(180..188)?.try_into().ok()?) as usize;
    let preset_end = 188usize.checked_add(preset_len)?;
    if preset_end.checked_add(16)? != dec.len() {
        return None;
    }
    let preset = &dec[188..preset_end];
    let footer16 = &dec[preset_end..];

    let push_sub = |out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u64).to_le_bytes());
        out.extend_from_slice(tag);
        out.extend_from_slice(body);
    };

    let mut new_dec = Vec::with_capacity(dec.len() + 660);
    new_dec.extend_from_slice(&dec[..188]);
    new_dec.extend_from_slice(&5u32.to_le_bytes());
    push_sub(&mut new_dec, b"tsrp", preset);
    push_sub(&mut new_dec, b"knab", &[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0, 0, 0, 0]);
    if ext == "nfm8" {
        let mut ctni = Vec::with_capacity(537);
        ctni.extend_from_slice(b"E8MF");
        ctni.extend_from_slice(&[0x11, 0x27, 0, 0, 0, 0, 0, 0, 0x41, 0, 0, 0, 0, 0, 0, 0xa0, 0x42, 0, 0, 0xa0, 0x42]);
        ctni.extend_from_slice(&[0xff; 28]);
        ctni.extend_from_slice(&0x74u32.to_le_bytes());
        ctni.extend_from_slice(&[0xff; 8]);
        ctni.extend_from_slice(&0x0fu32.to_le_bytes());
        ctni.extend_from_slice(&[0xff; 468]);
        push_sub(&mut new_dec, b"ctni", &ctni);

        let mut siug = Vec::with_capacity(32);
        siug.extend_from_slice(b"E8MF");
        siug.extend_from_slice(&1000u64.to_le_bytes());
        siug.extend_from_slice(&0u32.to_le_bytes());
        siug.extend_from_slice(&u32::MAX.to_le_bytes());
        siug.extend_from_slice(&25u32.to_le_bytes());
        siug.extend_from_slice(&1u32.to_le_bytes());
        siug.extend_from_slice(&1u32.to_le_bytes());
        push_sub(&mut new_dec, b"siug", &siug);
    } else {
        let ctni = [8, 0, 0, 0, 3, 0, 0, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
        push_sub(&mut new_dec, b"ctni", &ctni);
        push_sub(&mut new_dec, b"siug", &[0, 0, 0, 0]);
    }
    push_sub(&mut new_dec, b"gptc", b"YTME");
    new_dec.extend_from_slice(footer16);

    let total_dec = new_dec.len() as u64;
    new_dec[0..8].copy_from_slice(&total_dec.to_le_bytes());
    new_dec[84..92].copy_from_slice(&(total_dec - 84).to_le_bytes());
    new_dec[124..132].copy_from_slice(&(total_dec - 132).to_le_bytes());
    new_dec[180..188].copy_from_slice(&(total_dec - 204).to_le_bytes());

    let new_comp = fastlz_compress(&new_dec);
    let mut new_t = Vec::with_capacity(117 + new_comp.len() + t_footer.len());
    new_t.extend_from_slice(&t_part[..117]);
    new_t.extend_from_slice(&new_comp);
    new_t.extend_from_slice(t_footer);
    let t_len = new_t.len() as u64;
    new_t[0..8].copy_from_slice(&t_len.to_le_bytes());
    new_t[40..48].copy_from_slice(&(t_len - 48).to_le_bytes());
    new_t[60..68].copy_from_slice(&(t_len - 73).to_le_bytes());
    new_t[109..113].copy_from_slice(&(total_dec as u32).to_le_bytes());
    new_t[113..117].copy_from_slice(&(new_comp.len() as u32).to_le_bytes());

    let mut out = Vec::with_capacity(t_idx + new_t.len());
    out.extend_from_slice(&buf[..t_idx]);
    out.extend_from_slice(&new_t);
    let out_len = out.len() as u64;
    out[0..8].copy_from_slice(&out_len.to_le_bytes());
    out[e_idx..e_idx + 8].copy_from_slice(&(out_len - e_idx as u64).to_le_bytes());
    Some(out)
}

/// FastLZ Level 1 decompression (`DSINs` payload).
fn fastlz_decompress(data: &[u8], expected_len: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(expected_len.min(1 << 20));
    let mut i = 0usize;
    while i < data.len() {
        let ctrl = data[i];
        i += 1;
        if ctrl < 32 {
            let len = (ctrl as usize) + 1;
            let s = data.get(i..i.checked_add(len)?)?;
            out.extend_from_slice(s);
            i += len;
        } else {
            let mut len = ((ctrl >> 5) as usize) - 1;
            let mut ofs = ((ctrl & 31) as usize) << 8;
            if len == 6 {
                len += *data.get(i)? as usize;
                i += 1;
            }
            ofs += (*data.get(i)? as usize) + 1;
            i += 1;
            if ofs > out.len() {
                return None;
            }
            let start = out.len() - ofs;
            for k in 0..(len + 3) {
                let b = out[start + k];
                out.push(b);
            }
        }
    }
    (out.len() == expected_len).then_some(out)
}

/// FastLZ Level 1 compression (`DSINs` payload).
fn fastlz_compress(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut lit_start = 0usize;
    let mut lit_len = 0usize;
    let flush_lit = |out: &mut Vec<u8>, input: &[u8], mut start: usize, mut len: usize| {
        while len > 0 {
            let chunk = len.min(32);
            out.push((chunk - 1) as u8);
            out.extend_from_slice(&input[start..start + chunk]);
            start += chunk;
            len -= chunk;
        }
    };
    let mut i = 0usize;
    while i < input.len() {
        let mut best_len = 0usize;
        let mut best_ofs = 0usize;
        if i + 3 <= input.len() {
            let min_j = i.saturating_sub(8192);
            for j in (min_j..i).rev() {
                if input[j] == input[i] && input[j + 1] == input[i + 1] && input[j + 2] == input[i + 2] {
                    let mut l = 3usize;
                    let max_l = (input.len() - i).min(264);
                    while l < max_l && input[j + l] == input[i + l] {
                        l += 1;
                    }
                    if l > best_len {
                        best_len = l;
                        best_ofs = i - j;
                        if best_len == 264 {
                            break;
                        }
                    }
                }
            }
        }
        if best_len >= 3 {
            flush_lit(&mut out, input, lit_start, lit_len);
            lit_len = 0;
            let len_code = best_len - 2;
            let ofs = best_ofs - 1;
            let ofs_hi = ((ofs >> 8) & 31) as u8;
            let ofs_lo = (ofs & 0xff) as u8;
            if len_code < 7 {
                out.push(((len_code as u8) << 5) | ofs_hi);
                out.push(ofs_lo);
            } else {
                out.push((7 << 5) | ofs_hi);
                out.push((len_code - 7) as u8);
                out.push(ofs_lo);
            }
            i += best_len;
            lit_start = i;
        } else {
            if lit_len == 0 {
                lit_start = i;
            }
            lit_len += 1;
            i += 1;
        }
    }
    flush_lit(&mut out, input, lit_start, lit_len);
    out
}

/// Controller pages from the `DSINy` chunk of an `NISound` (`.nfm8`, `.nmsv`) preset.
fn parse_dsiny(raw: &[u8]) -> Vec<Vec<Knob>> {
    let parse = || -> Option<Vec<Vec<Knob>>> {
        let root_size = u64::from_le_bytes(raw.get(..8)?.try_into().ok()?) as usize;
        let buf = raw.get(..root_size.min(raw.len()))?;
        // Both the outer resource and its NISound payload carry DSINy. Only
        // the inner tag has the hsin frame whose offsets are parsed below.
        let y_idx = buf
            .windows(8)
            .enumerate()
            .filter(|(_, w)| *w == b"DSINy\0\0\0")
            .filter_map(|(i, _)| i.checked_sub(48))
            .find(|&i| buf.get(i + 12..i + 16) == Some(b"hsin"))?;
        let y_part = buf.get(y_idx..)?;
        let y_size = u64::from_le_bytes(y_part.get(..8)?.try_into().ok()?) as usize;
        let y = y_part.get(..y_size.min(y_part.len()))?;
        if y.len() < 96 {
            return None;
        }
        let mut at = 92usize;
        let read_u32 = |at: &mut usize| -> Option<u32> {
            let v = u32::from_le_bytes(y.get(*at..at.checked_add(4)?)?.try_into().ok()?);
            *at += 4;
            Some(v)
        };
        let read_utf16 = |at: &mut usize| -> Option<String> {
            let n = read_u32(at)? as usize;
            if n > 256 {
                return None;
            }
            let bytes = y.get(*at..at.checked_add(n.checked_mul(2)?)?)?;
            *at += n * 2;
            let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            Some(String::from_utf16_lossy(&units))
        };
        let num_sections = (read_u32(&mut at)? as usize).min(64);
        let mut pages: Vec<Vec<Knob>> = Vec::new();
        let mut cur: Vec<Knob> = Vec::new();
        for _ in 0..num_sections {
            let _ver = read_u32(&mut at)?;
            let num_knobs = (read_u32(&mut at)? as usize).min(64);
            let section_name = read_utf16(&mut at)?;
            let mut raw_knobs = Vec::with_capacity(num_knobs);
            for _ in 0..num_knobs {
                let _kver = read_u32(&mut at)?;
                let kname = read_utf16(&mut at)?;
                let _lo = read_u32(&mut at)?;
                let _hi = read_u32(&mut at)?;
                let pid = read_u32(&mut at)?;
                let pos = read_u32(&mut at)?;
                raw_knobs.push((kname, pid, pos));
            }
            let slots = raw_knobs.iter().filter_map(|&(_, _, pos)| (1..=8).contains(&pos).then_some(pos as usize)).max().unwrap_or(0);
            if slots == 0 {
                continue;
            }
            let mut sec_knobs = vec![Knob::default(); slots];
            sec_knobs[0].section = section_name;
            for (kname, pid, pos) in raw_knobs {
                if (1..=slots).contains(&(pos as usize)) {
                    let slot = &mut sec_knobs[pos as usize - 1];
                    slot.id = (pid != u32::MAX).then_some(pid);
                    slot.name = if kname == "<{[autoname]}>" { String::new() } else { kname.trim().to_string() };
                }
            }
            if !cur.is_empty() && cur.len() + sec_knobs.len() > 8 {
                cur.resize(8, Knob::default());
                pages.push(std::mem::take(&mut cur));
            }
            cur.extend(sec_knobs);
            if cur.len() == 8 {
                pages.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            pages.push(cur);
        }
        Some(pages)
    };
    parse().unwrap_or_default()
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

    #[test]
    fn nfm8_and_nmsv_presets_wrap_tsrp_and_parse_dsiny_pages() {
        let utf16 = |s: &str| -> Vec<u8> {
            let units: Vec<u16> = s.encode_utf16().collect();
            let mut out = (units.len() as u32).to_le_bytes().to_vec();
            for u in units {
                out.extend_from_slice(&u.to_le_bytes());
            }
            out
        };
        // Build a synthetic NISound (.nfm8) container with DSINe, DSINy and DSINt/DSINs.
        let mut y_body = vec![0u8; 92];
        y_body[12..16].copy_from_slice(b"hsin");
        y_body[48..56].copy_from_slice(b"DSINy\0\0\0");
        y_body.extend_from_slice(&2u32.to_le_bytes()); // 2 sections
        for (sec_name, knobs) in [
            ("Timbre", &[("<{[autoname]}>", 5u32, 1u32), ("Brightness", 0u32, 2u32)][..]),
            ("FX", &[("Switch", 92u32, 1u32), ("Amount", 25u32, 2u32)][..]),
        ] {
            y_body.extend_from_slice(&100u32.to_le_bytes());
            y_body.extend_from_slice(&(knobs.len() as u32).to_le_bytes());
            y_body.extend(utf16(sec_name));
            for &(kname, pid, pos) in knobs {
                y_body.extend_from_slice(&100u32.to_le_bytes());
                y_body.extend(utf16(kname));
                y_body.extend_from_slice(&0.0f32.to_le_bytes());
                y_body.extend_from_slice(&1.0f32.to_le_bytes());
                y_body.extend_from_slice(&pid.to_le_bytes());
                y_body.extend_from_slice(&pos.to_le_bytes());
            }
        }
        let y_len = y_body.len() as u64;
        y_body[0..8].copy_from_slice(&y_len.to_le_bytes());

        let bare_preset = b"E8MFbare-fm8-preset-payload-with-repeats-repeats-repeats";
        let mut dec = vec![0u8; 188];
        dec[12..16].copy_from_slice(b"hsin");
        dec[180..188].copy_from_slice(&(bare_preset.len() as u64).to_le_bytes());
        dec.extend_from_slice(bare_preset);
        dec.extend_from_slice(&[0u8; 16]);
        let dec_len = dec.len() as u64;
        dec[0..8].copy_from_slice(&dec_len.to_le_bytes());

        let comp = fastlz_compress(&dec);
        assert_eq!(fastlz_decompress(&comp, dec.len()).as_deref(), Some(&dec[..]));

        let mut t_part = vec![0u8; 117];
        t_part[48..56].copy_from_slice(b"DSINt\0\0\0");
        t_part[68..76].copy_from_slice(b"DSINs\0\0\0");
        t_part[109..113].copy_from_slice(&(dec.len() as u32).to_le_bytes());
        t_part[113..117].copy_from_slice(&(comp.len() as u32).to_le_bytes());
        t_part.extend_from_slice(&comp);
        t_part.extend_from_slice(&[0u8; 16]);
        let t_len = t_part.len() as u64;
        t_part[0..8].copy_from_slice(&t_len.to_le_bytes());

        let mut raw = vec![0u8; 64];
        raw[12..16].copy_from_slice(b"hsin");
        raw[48..56].copy_from_slice(b"DSINe\0\0\0");
        // The real resource index repeats this tag before the inner NISound
        // header. It has no adjacent DSINs and must not defeat state wrapping.
        raw.extend_from_slice(b"DSINt\0\0\0");
        raw.extend_from_slice(&y_body);
        raw.extend_from_slice(&t_part);
        let raw_len = raw.len() as u64;
        raw[0..8].copy_from_slice(&raw_len.to_le_bytes());
        raw[56..64].copy_from_slice(&(raw_len - 56).to_le_bytes());

        let fm8 = vst3("Native Instruments", "FM8", true, "5653544E694638666D38000000000000");
        let resolved = native_preset("nfm8", &raw, &[fm8]).unwrap().unwrap();
        assert_eq!(resolved.pages.len(), 1);
        assert_eq!(
            resolved.pages[0],
            vec![
                Knob { id: Some(5), name: String::new(), section: "Timbre".into() },
                Knob { id: Some(0), name: "Brightness".into(), section: String::new() },
                Knob { id: Some(92), name: "Switch".into(), section: "FX".into() },
                Knob { id: Some(25), name: "Amount".into(), section: String::new() },
            ]
        );
        let state = parse_vstpreset(chunk_parts(&resolved.chunk).unwrap().1).unwrap();
        let t_idx = state
            .component
            .windows(8)
            .enumerate()
            .filter(|(_, w)| *w == b"DSINt\0\0\0")
            .filter_map(|(i, _)| i.checked_sub(48))
            .find(|&i| state.component.get(i + 68..i + 76) == Some(b"DSINs\0\0\0"))
            .unwrap();
        let tp = &state.component[t_idx..];
        let ulen = u32::from_le_bytes(tp[109..113].try_into().unwrap()) as usize;
        let clen = u32::from_le_bytes(tp[113..117].try_into().unwrap()) as usize;
        let unwrapped = fastlz_decompress(&tp[117..117 + clen], ulen).unwrap();
        assert_eq!(u32::from_le_bytes(unwrapped[188..192].try_into().unwrap()), 5);
        assert_eq!(u64::from_le_bytes(unwrapped[192..200].try_into().unwrap()) as usize, bare_preset.len());
        assert_eq!(&unwrapped[200..204], b"tsrp");
        assert_eq!(&unwrapped[204..204 + bare_preset.len()], bare_preset);
        assert_eq!(wrap_nisound_preset("nfm8", &state.component), None, "already wrapped");
    }

    /// Kontakt library files are refused before Kontakt is made: it would stop on a
    /// modal "missing content" box in setState.
    #[test]
    fn kontakt_library_files_are_refused_with_a_reason() {
        let mut raw = vec![0; 32];
        raw[12..16].copy_from_slice(b"hsin");
        let kontakt = vst3("Native Instruments", "Kontakt 7", true, "11111111111111111111111111111111");
        for ext in ["nksn", "NKSN", "nki", "NKI"] {
            assert_eq!(native_preset(ext, &raw, &[kontakt.clone()]), Some(Err(KONTAKT_FILE.into())), "{ext}");
        }
    }

    fn info() -> SoundInfo {
        SoundInfo {
            author: String::new(),
            bankchain: vec!["Infiltrator".into(), String::new(), String::new()],
            characters: vec!["Dark".into()],
            comment: String::new(),
            device_type: "INST".into(),
            modes: Vec::new(),
            name: "Night Lead".into(),
            types: vec![vec!["Synth Lead".into(), "Classic".into()]],
            uuid: "0b6a6a5e-3f0e-4c47-9d0e-2f1f4b5c6d7e".into(),
            vendor: "Steinberg".into(),
        }
    }

    fn ids(vst3: Option<[u8; 16]>, vst2: Option<u32>) -> PluginIds {
        PluginIds { vst3, vst2, name: "Infiltrator".into(), vendor: "Steinberg".into() }
    }

    const CID: [u8; 16] = *b"VSTFUCKinfiltrat";

    #[test]
    fn a_written_preset_reads_back() {
        let pages = vec![
            vec![
                Knob { id: Some(3), name: "Cutoff".into(), section: "Filter".into() },
                Knob { id: None, name: String::new(), section: String::new() },
            ],
            vec![Knob { id: Some(300), name: "Drive".into(), section: String::new() }],
        ];
        let bytes = write(&info(), &pages, &ids(Some(CID), None), b"VST3 state");
        let p = parse(&bytes).unwrap();
        assert_eq!(p.vst3, Some(CID));
        assert_eq!(p.vst2, None);
        assert_eq!(p.name.as_deref(), Some("Infiltrator"));
        assert_eq!(p.state, b"VST3 state");
        assert_eq!(p.pages, pages);
        assert_eq!(p.info, info());
    }

    /// RIFF form `NIKS`; `NISI`, `NICA`, `PLID`, `PCHK` in that order, each a version
    /// (1) then its data, padded to even (`nksf::NKSWriter::v1::write`, `riff::write`).
    #[test]
    fn chunks_go_in_maschines_order() {
        let bytes = write(&info(), &[], &ids(Some(CID), None), b"odd");
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize, bytes.len() - 8);
        assert_eq!(&bytes[8..12], b"NIKS");
        let mut at = 12;
        let mut seen = Vec::new();
        while at < bytes.len() {
            let n = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            assert_eq!(u32::from_le_bytes(bytes[at + 8..at + 12].try_into().unwrap()), 1, "version");
            seen.push(bytes[at..at + 4].to_vec());
            at += 8 + n + (n & 1);
        }
        assert_eq!(at, bytes.len());
        assert_eq!(seen, [b"NISI", b"NICA", b"PLID", b"PCHK"].map(|c| c.to_vec()));
        assert_eq!(&bytes[bytes.len() - 4..], b"odd\0", "PCHK last, padded to even");
    }

    /// `PLID`: `VST.magic`, `VST3.uid`, `pluginName`, `pluginVendor`, those present
    /// (`nksf::writePluginIdentifiers`); numbers in their shortest form.
    #[test]
    fn plid_names_the_plugin_as_maschine_does() {
        let bytes = write(&info(), &[], &ids(None, Some(u32::from_be_bytes(*b"NiR7"))), b"");
        let plid = chunk(&bytes, b"PLID");
        let mut want = vec![0x83, 0xa9];
        want.extend(b"VST.magic");
        want.push(0xce);
        want.extend(b"NiR7");
        want.push(0xaa);
        want.extend(b"pluginName");
        want.push(0xab);
        want.extend(b"Infiltrator");
        want.push(0xac);
        want.extend(b"pluginVendor");
        want.push(0xa9);
        want.extend(b"Steinberg");
        assert_eq!(plid, want);

        let plid = chunk(
            &write(&info(), &[], &ids(Some([0; 15].into_iter().chain([7]).collect::<Vec<_>>().try_into().unwrap()), None), b""),
            b"PLID",
        );
        let mut want = vec![0x83, 0xa8];
        want.extend(b"VST3.uid");
        want.extend([0x94, 0, 0, 0, 7]);
        assert_eq!(plid[..want.len()], want[..], "four words, each as short as it goes");
    }

    /// `NISI`'s keys in order (a `std::map`), every one written, and a knob's
    /// `autoname`, `id`, `name`, `section` (when it has one), `vflag`.
    #[test]
    fn nisi_and_nica_keys_are_sorted() {
        let bytes = write(&info(), &[vec![Knob { id: Some(5), name: "Mix".into(), section: String::new() }]], &ids(Some(CID), None), b"");
        let nisi = chunk(&bytes, b"NISI");
        assert_eq!(nisi[0], 0x89, "nine keys");
        let keys = ["author", "bankchain", "characters", "comment", "deviceType", "name", "types", "uuid", "vendor"];
        let mut at = 0;
        for k in keys {
            let found = nisi[at..].windows(k.len() + 1).position(|w| w[0] == 0xa0 | k.len() as u8 && &w[1..] == k.as_bytes());
            at += found.unwrap_or_else(|| panic!("{k} after the one before")) + k.len() + 1;
        }
        let nica = chunk(&bytes, b"NICA");
        let mut want = vec![0x81, 0xa3];
        want.extend(b"ni8");
        want.extend([0x91, 0x91, 0x84, 0xa8]);
        want.extend(b"autoname");
        want.extend([0xc3, 0xa2]);
        want.extend(b"id");
        want.extend([0x05, 0xa4]);
        want.extend(b"name");
        want.push(0xa3);
        want.extend(b"Mix");
        want.push(0xa5);
        want.extend(b"vflag");
        want.push(0xc2);
        assert_eq!(nica, want);
    }

    /// A new preset's `uuid`: random (version 4), lower case and hyphenated, as
    /// Maschine's (`NI::GP::UUID::generateRandom`, `toString`).
    #[test]
    fn new_uuids_are_random_version_4() {
        let (a, b) = (new_uuid(), new_uuid());
        assert_ne!(a, b);
        for u in [a, b] {
            let groups: Vec<_> = u.split('-').map(str::len).collect();
            assert_eq!(groups, [8, 4, 4, 4, 12], "{u}");
            assert!(u.chars().all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c)), "{u}");
            assert_eq!(&u[14..15], "4", "{u}");
            assert!("89ab".contains(&u[19..20]), "{u}");
        }
    }

    #[test]
    fn modes_are_written_when_there_are_some() {
        let mut i = info();
        i.modes = vec!["Arp".into()];
        let bytes = write(&i, &[], &ids(Some(CID), None), b"");
        assert_eq!(chunk(&bytes, b"NISI")[0], 0x8a, "ten keys");
        assert_eq!(parse(&bytes).unwrap().info.modes, ["Arp"]);
    }

    /// The data of chunk `id` (after its version).
    fn chunk(bytes: &[u8], id: &[u8; 4]) -> Vec<u8> {
        let mut at = 12;
        while at + 8 <= bytes.len() {
            let n = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            if &bytes[at..at + 4] == id {
                return bytes[at + 12..at + 8 + n].to_vec();
            }
            at += 8 + n + (n & 1);
        }
        panic!("no {}", String::from_utf8_lossy(id));
    }

    /// NI's factory presets, written again from what they say, come out the same,
    /// byte for byte (they're written by the same `nksf` library as Maschine's). Skipped
    /// without NI's Raum installed.
    #[test]
    fn factory_presets_are_written_byte_for_byte() {
        let raum = std::path::Path::new("/Library/Application Support/Native Instruments/Raum");
        let Ok(dirs) = std::fs::read_dir(raum) else { return };
        let files = dirs.flatten().filter_map(|d| std::fs::read_dir(d.path()).ok()).flatten().flatten().map(|f| f.path());
        let mut n = 0;
        for f in files.filter(|p| p.extension().is_some_and(|e| e == "nksfx")) {
            let bytes = std::fs::read(&f).unwrap();
            let p = parse(&bytes).unwrap();
            let plugin = PluginIds {
                vst3: p.vst3,
                vst2: p.vst2,
                name: p.name.clone().unwrap_or_default(),
                vendor: p.vendor.clone().unwrap_or_default(),
            };
            let ours = write(&p.info, &p.pages, &plugin, &p.state);
            for id in [b"NISI", b"NICA", b"PLID", b"PCHK"] {
                let (a, b) = (chunk(&ours, id), chunk(&bytes, id));
                let at = a.iter().zip(&b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
                assert_eq!(
                    a,
                    b,
                    "{} {}: from byte {at}: {:02x?} vs {:02x?}",
                    f.display(),
                    String::from_utf8_lossy(id),
                    &a[at..(at + 24).min(a.len())],
                    &b[at..(at + 24).min(b.len())]
                );
            }
            assert_eq!(ours, bytes, "{}", f.display());
            n += 1;
        }
        assert!(n > 0);
    }
}
