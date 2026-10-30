//! Typed decoding of the document archive. Field order follows the
//! `serialize`/`load` outlines in `out/schema` (see `serialize-schema`); each
//! decoder notes the C++ type it mirrors. Names of fields whose meaning is not
//! yet known say so.

use crate::archive::{Reader, Session, Slot};
use crate::container::{Error, Result};

/// `NI::MASCHINE::DATA::ContentInfo`: which NI library/expansion a project ships with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentInfo {
    pub library: String,
    pub product: String,
    /// NI product id (SNPID), e.g. `"632"`.
    pub snpid: String,
    pub checksum: u32,
}

/// The fields at the start of `NI::MASCHINE::DATA::Project`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectHeader {
    pub version: u32,
    pub content: Option<ContentInfo>,
    pub name: String,
    /// Meaning unknown (bool at Project+0xd8).
    pub unknown_flag: bool,
    /// Meaning unknown (int64 at Project+0xe0).
    pub unknown_i64: i64,
    /// Meaning unknown (two int32 packed into Project+0xe8); present from version 1.
    pub unknown_pair: Option<(i64, i64)>,
}

fn new_object(s: &mut Session, r: &mut Reader, key: &str) -> Result<u32> {
    let at = r.offset();
    match s.object(r, key)? {
        Slot::New { version, .. } => Ok(version),
        _ => Err(Error { offset: at, what: "expected a new object" }),
    }
}

/// `NI::MASCOMMON::DATA::WStringParameter` → its string value.
pub fn wstring_parameter(s: &mut Session, r: &mut Reader) -> Result<String> {
    new_object(s, r, "NI::MASCOMMON::DATA::WStringParameter")?;
    // base ValueParameter<std::wstring>: base Parameter, then the value
    new_object(s, r, "NI::ML::TM::ValueParameter<std::wstring>")?;
    if new_object(s, r, "NI::ML::TM::Parameter")? > 0 {
        let _id = r.string()?;
    }
    let value = r.string()?;
    // base MaschineParameter: no data of its own
    new_object(s, r, "NI::MASCOMMON::DATA::MaschineParameter")?;
    Ok(value)
}

/// `ContentInfo::load`: a flag, then (if set) three strings and a checksum.
fn content_info(r: &mut Reader) -> Result<Option<ContentInfo>> {
    if r.int()? == 0 {
        return Ok(None);
    }
    let library = r.string()?;
    let product = r.string()?;
    let snpid = r.string()?;
    let checksum = r.int()? as u32;
    Ok(Some(ContentInfo { library, product, snpid, checksum }))
}

/// Decode the start of the root `Project` object.
pub fn project_header(s: &mut Session, r: &mut Reader) -> Result<ProjectHeader> {
    let version = new_object(s, r, "NI::MASCHINE::DATA::Project")?;
    // boost::scoped_ptr<ContentInfo> wraps a pointer.
    new_object(s, r, "boost::scoped_ptr<NI::MASCHINE::DATA::ContentInfo>")?;
    let content = match s.pointer(r, "NI::MASCHINE::DATA::ContentInfo", false, &|n| n.to_string())? {
        Slot::Null => None,
        Slot::New { .. } => content_info(r)?,
        Slot::Ref(_) => return Err(Error { offset: r.offset(), what: "ContentInfo back-reference" }),
    };
    let name = wstring_parameter(s, r)?;
    let unknown_flag = r.int()? != 0;
    let unknown_i64 = r.int()?;
    let unknown_pair = if version >= 1 { Some((r.int()?, r.int()?)) } else { None };
    Ok(ProjectHeader { version, content, name, unknown_flag, unknown_i64, unknown_pair })
}
