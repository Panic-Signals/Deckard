//! The tag block of a Maschine file: an `NI::SOUND::SoundInfo2` stream, the payload of
//! the `NISD/108` item (after a u32 version), the same in every kind of file. It holds
//! what NI's browser shows and filters on: name, author, the bank chain (Library ›
//! Bank › Sub-bank) and the attributes, Types and Characters.
//!
//! All integers little-endian; `wstr` is a u32 count of UTF-16 units, then the units.
//!
//! ```text
//! u32 2, u32 1
//! wstr ×6                 "", name, author, vendor, comment, ""
//! u64 date                u64::MAX: none
//! u32 ×6                  0, 0, 0, 0, 0, 1 in every file seen
//! u32 n, wstr ×n          the bank chain: ["Expansions Selection", "Resonant Blaze"]
//! u32 n, wstr ×n          attributes: a Type is "\:" before each level, every level
//!                         listed ("\:Kits", "\:Kits\:Percussion"); a Character is
//!                         "\." and its name ("\.in progress")
//! u32 n, wstr ×n          none in every file seen
//! u32 n, (wstr, wstr) ×n  properties: "\@color", "\@tempo", "\@verl", …
//! ```
//!
//! What isn't understood is kept as read, so a file's block written back is
//! byte-identical.

use crate::container::{Error, Result};

/// The NISound format version Maschine 3 writes (1.7.14).
const NISOUND_VERSION: &str = "1.7.14";

const TYPE: &str = "\\:";
const CHARACTER: &str = "\\.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundInfo {
    pub name: String,
    pub author: String,
    pub vendor: String,
    pub comment: String,
    /// Library, Bank, Sub-bank.
    pub bank: Vec<String>,
    attributes: Vec<String>,
    pub props: Vec<(String, String)>,
    head: [u32; 2],
    texts: [String; 2],
    date: u64,
    fields: [u32; 6],
    others: Vec<String>,
}

impl Default for SoundInfo {
    /// An empty block with the standard properties, as Maschine 3 writes a new file's.
    fn default() -> SoundInfo {
        let props = [
            ("\\@color", "0"),
            ("\\@devicetypeflags", "0"),
            ("\\@soundtype", "0"),
            ("\\@tempo", "0"),
            ("\\@verl", NISOUND_VERSION),
            ("\\@verm", NISOUND_VERSION),
            ("\\@visib", "0"),
        ];
        SoundInfo {
            name: String::new(),
            author: String::new(),
            vendor: String::new(),
            comment: String::new(),
            bank: Vec::new(),
            attributes: Vec::new(),
            props: props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            head: [2, 1],
            texts: Default::default(),
            date: u64::MAX,
            fields: [0, 0, 0, 0, 0, 1],
            others: Vec::new(),
        }
    }
}

impl SoundInfo {
    pub fn parse(bytes: &[u8]) -> Result<SoundInfo> {
        let mut r = Reader { b: bytes, at: 0 };
        let head = [r.u32()?, r.u32()?];
        let mut texts: Vec<String> = Vec::with_capacity(6);
        for _ in 0..6 {
            texts.push(r.wstr()?);
        }
        let date = r.u64()?;
        let mut fields = [0; 6];
        for f in &mut fields {
            *f = r.u32()?;
        }
        let bank = r.list()?;
        let attributes = r.list()?;
        let others = r.list()?;
        let n = r.u32()? as usize;
        let mut props = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            props.push((r.wstr()?, r.wstr()?));
        }
        if r.at != bytes.len() {
            return Err(Error { offset: r.at, what: "bytes left after the sound info" });
        }
        let [t0, name, author, vendor, comment, t5]: [String; 6] = texts.try_into().expect("six texts");
        Ok(SoundInfo { name, author, vendor, comment, bank, attributes, props, head, texts: [t0, t5], date, fields, others })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::new();
        let u32 = |o: &mut Vec<u8>, v: u32| o.extend_from_slice(&v.to_le_bytes());
        let wstr = |o: &mut Vec<u8>, s: &str| {
            let units: Vec<u16> = s.encode_utf16().collect();
            o.extend_from_slice(&(units.len() as u32).to_le_bytes());
            for u in units {
                o.extend_from_slice(&u.to_le_bytes());
            }
        };
        let list = |o: &mut Vec<u8>, l: &[String]| {
            o.extend_from_slice(&(l.len() as u32).to_le_bytes());
            for s in l {
                wstr(o, s);
            }
        };
        u32(&mut o, self.head[0]);
        u32(&mut o, self.head[1]);
        for s in [&self.texts[0], &self.name, &self.author, &self.vendor, &self.comment, &self.texts[1]] {
            wstr(&mut o, s);
        }
        o.extend_from_slice(&self.date.to_le_bytes());
        for f in self.fields {
            u32(&mut o, f);
        }
        list(&mut o, &self.bank);
        list(&mut o, &self.attributes);
        list(&mut o, &self.others);
        u32(&mut o, self.props.len() as u32);
        for (k, v) in &self.props {
            wstr(&mut o, k);
            wstr(&mut o, v);
        }
        o
    }

    /// The attributes as stored ("\:Kits", "\.in progress", …).
    pub fn attributes(&self) -> &[String] {
        &self.attributes
    }

    /// The Types, one level list per type, only the deepest of each (the file lists
    /// every level: "Kits", then "Kits › Percussion").
    pub fn types(&self) -> Vec<Vec<String>> {
        let all: Vec<Vec<String>> =
            self.attributes.iter().filter_map(|a| a.strip_prefix(TYPE)).map(|a| a.split(TYPE).map(str::to_string).collect()).collect();
        all.iter().filter(|t| !all.iter().any(|u| u.len() > t.len() && u.starts_with(t))).cloned().collect()
    }

    pub fn characters(&self) -> Vec<String> {
        self.attributes.iter().filter_map(|a| a.strip_prefix(CHARACTER)).map(str::to_string).collect()
    }

    /// Replace the Types and Characters: the characters first, then each type with
    /// every level of it listed, as Maschine writes them.
    pub fn set_tags(&mut self, types: &[Vec<String>], characters: &[String]) {
        let mut out: Vec<String> = characters.iter().map(|c| format!("{CHARACTER}{c}")).collect();
        for t in types {
            for depth in 1..=t.len() {
                let a: String = t[..depth].iter().map(|l| format!("{TYPE}{l}")).collect();
                if !out.contains(&a) {
                    out.push(a);
                }
            }
        }
        self.attributes = out;
    }

    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn set_prop(&mut self, key: &str, value: &str) {
        match self.props.iter_mut().find(|(k, _)| k == key) {
            Some(p) => p.1 = value.to_string(),
            None => self.props.push((key.to_string(), value.to_string())),
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let s = self.b.get(self.at..self.at + n).ok_or(Error { offset: self.at, what: "sound info ends early" })?;
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn wstr(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        let at = self.at;
        let units: Vec<u16> = self
            .take(n.checked_mul(2).ok_or(Error { offset: at, what: "bad text length" })?)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16(&units).map_err(|_| Error { offset: at, what: "bad UTF-16 text" })
    }
    fn list(&mut self) -> Result<Vec<String>> {
        let n = self.u32()? as usize;
        let mut v = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            v.push(self.wstr()?);
        }
        Ok(v)
    }
}
