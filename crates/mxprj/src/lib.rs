//! Maschine project files (`.mxprj`).
//!
//! Layout, outermost first:
//!
//! 1. an NI `hsin` item tree ([`container`]); the root has one `MAS2`/`PROJ` child
//! 2. somewhere below it, an item whose frame [`ARCHIVE_FRAME`] carries the document
//! 3. that frame's payload wraps a boost `portable_binary_iarchive` ([`archive`])
//!    holding the `NI::MASCHINE::DATA` object graph

pub mod archive;
pub mod container;
pub mod decode;
pub mod document;
pub mod sound_info;
pub mod types;

pub use container::{Error, Item, Result};

/// Frame id whose payload holds the serialized document.
pub const ARCHIVE_FRAME: u32 = 109;

/// Frame id of the `NISD/108` item: a u32 version, then the file's [`sound_info::SoundInfo`].
pub const SOUND_INFO_FRAME: u32 = 108;

pub struct Project<'a> {
    pub root: Item<'a>,
    /// The boost archive bytes, starting at the `serialization::archive` signature.
    pub archive: &'a [u8],
    /// File offset of `archive[0]`.
    pub archive_offset: usize,
    pub header: archive::Header,
}

impl<'a> Project<'a> {
    pub fn parse(file: &'a [u8]) -> Result<Project<'a>> {
        let root = Item::parse(file)?;
        let frame = root.find_frame(ARCHIVE_FRAME).ok_or(Error { offset: 0, what: "no archive frame (id 109)" })?;
        // Payload: u32 version, u32, u32, u64 blob size, then the blob = u32 length + archive.
        let d = frame.data;
        let at = |o: usize| Error { offset: frame_data_offset(file, d) + o, what: "malformed archive frame" };
        let blob_size = d.get(12..20).map(|b| u64::from_le_bytes(b.try_into().unwrap())).ok_or(at(12))?;
        let len = d.get(20..24).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or(at(20))? as usize;
        if blob_size != len as u64 + 4 || 24 + len > d.len() {
            return Err(at(12));
        }
        let archive = &d[24..24 + len];
        let archive_offset = frame_data_offset(file, d) + 24;
        let header = archive::Reader::new(archive, archive_offset).header()?;
        Ok(Project { root, archive, archive_offset, header })
    }

    /// Decode the start of the root `Project` object (content info and name).
    pub fn project_header(&self) -> Result<document::ProjectHeader> {
        document::project_header(&mut archive::Session::new(), &mut self.body())
    }

    /// The `MAS2` child's item id (`PROJ`, `GRP `, `SND `, `SLOT`), naming the document kind.
    pub fn kind(&self) -> Option<[u8; 4]> {
        self.root.children.iter().find(|c| &c.domain == b"MAS2").map(|c| c.item_id.to_be_bytes())
    }

    /// Decode the whole document with the registered type loaders. On failure,
    /// returns the error and the type path where decoding stopped.
    pub fn decode(&self, types: &types::Types) -> std::result::Result<decode::Obj, (Error, String)> {
        let root =
            self.kind().and_then(types::Types::root_for).ok_or((Error { offset: 0, what: "unknown document kind" }, String::new()))?;
        let mut d = decode::Decoder::new(self.body(), self.header.library_version, &types.loaders, &types.exports);
        let obj = d.root(root).map_err(|e| (e, d.path()))?;
        if d.r.remaining() != 0 {
            return Err((Error { offset: d.r.offset(), what: "bytes left after root object" }, root.to_string()));
        }
        Ok(obj)
    }

    /// The file with its document replaced by `root` (a tree as [`Project::decode`]
    /// returns it, possibly edited). Everything else in the file is kept as it was.
    /// Decoding the result gives `root` back, with object ids renumbered in order.
    pub fn encode(&self, types: &types::Types, root: decode::Obj) -> std::result::Result<Vec<u8>, (Error, String)> {
        let kind =
            self.kind().and_then(types::Types::root_for).ok_or((Error { offset: 0, what: "unknown document kind" }, String::new()))?;
        let (archive, _) = decode::encode(root, kind, self.header, &types.loaders, &types.exports)?;
        Ok(self.with_archive(&archive))
    }

    /// The file with the archive bytes replaced.
    pub fn with_archive(&self, archive: &[u8]) -> Vec<u8> {
        let old = self.archive.as_ptr();
        self.root.to_bytes(&mut |f| {
            if f.id != ARCHIVE_FRAME {
                return None;
            }
            let d = f.data;
            // Only the frame that holds this archive; keep its prefix and any tail.
            let start = old as usize - d.as_ptr() as usize;
            if !(d.as_ptr() as usize..d.as_ptr() as usize + d.len()).contains(&(old as usize)) {
                return None;
            }
            let tail = &d[start + self.archive.len()..];
            let mut p = d[..12].to_vec();
            p.extend_from_slice(&(archive.len() as u64 + 4).to_le_bytes());
            p.extend_from_slice(&(archive.len() as u32).to_le_bytes());
            p.extend_from_slice(archive);
            p.extend_from_slice(tail);
            Some(p)
        })
    }

    /// The file's tag block (the first `NISD/108` item's).
    pub fn sound_info(&self) -> Option<sound_info::SoundInfo> {
        let f = self.root.find_frame(SOUND_INFO_FRAME)?;
        sound_info::SoundInfo::parse(f.data.get(4..)?).ok()
    }

    /// The file with its tag block replaced by `info`; everything else as it was.
    pub fn with_sound_info(&self, info: &sound_info::SoundInfo) -> Vec<u8> {
        let Some(at) = self.root.find_frame(SOUND_INFO_FRAME).map(|f| f.data.as_ptr()) else {
            return self.root.to_bytes(&mut |_| None);
        };
        self.root.to_bytes(&mut |f| {
            (f.id == SOUND_INFO_FRAME && f.data.as_ptr() == at).then(|| {
                let mut p = f.data[..4].to_vec();
                p.extend_from_slice(&info.to_bytes());
                p
            })
        })
    }

    /// A reader positioned just after the archive header.
    pub fn body(&self) -> archive::Reader<'a> {
        let mut r = archive::Reader::new(self.archive, self.archive_offset);
        r.header().expect("header was validated in parse");
        r
    }
}

fn frame_data_offset(file: &[u8], data: &[u8]) -> usize {
    data.as_ptr() as usize - file.as_ptr() as usize
}
