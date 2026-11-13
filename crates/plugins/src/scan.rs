//! Finding installed plug-ins: VST3 classes (from a bundle's `moduleinfo.json`, or by
//! loading it, which callers should do in a separate process), VST2 plug-ins (by loading
//! them, likewise, or from an MPC plug-in folder's `plugin.json`), and Audio Units (from
//! the system's component registry).

use crate::module::{self, Module};
use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};
use vst3::ComPtr;
use vst3::Steinberg::*;

/// The plug-in formats Maschine hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Vst3,
    Vst2,
    AudioUnit,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Vst3 => "VST3",
            Kind::Vst2 => "VST",
            Kind::AudioUnit => "AU",
        }
    }
}

/// One installed plug-in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    pub kind: Kind,
    pub vendor: String,
    pub name: String,
    /// An instrument (else an effect).
    pub instrument: bool,
    /// The bundle (VST3/VST2) or "type subtype manufacturer" four-char codes (AU).
    pub location: String,
    /// The VST3 class id, upper-case hex (VST3), a VST2's unique id (8 hex digits; empty
    /// if not read yet), or an AU's type, subtype and manufacturer codes, in hex.
    pub cid: String,
}

impl Entry {
    /// Maschine's `plugin_id` for this plug-in (VST3: `"<vendor>:VST3:INSTR|FX:<cid>"`).
    pub fn maschine_id(&self) -> Option<String> {
        let role = if self.instrument { "INSTR" } else { "FX" };
        match self.kind {
            Kind::Vst3 => Some(format!("{}:VST3:{role}:{}", self.vendor, self.cid)),
            Kind::AudioUnit => {
                let (kind, sub) = self.au_codes()?;
                crate::state::au_id(&self.vendor, kind, sub)
            }
            Kind::Vst2 => Some(format!("{}:VST:{role}:{}", self.vendor, self.vst2_uid()?)),
        }
    }

    /// A VST2's unique id, if the scan read it.
    pub fn vst2_uid(&self) -> Option<u32> {
        (self.kind == Kind::Vst2 && self.cid.len() == 8).then(|| u32::from_str_radix(&self.cid, 16).ok()).flatten()
    }

    /// An AU's component type and subtype. (A cache from before AUs could be hosted has
    /// only the four-character codes of `location`.)
    fn au_codes(&self) -> Option<(u32, u32)> {
        let code = |s: Option<&str>| Some(u32::from_be_bytes(s?.as_bytes().try_into().ok()?));
        if self.cid.len() == 24 {
            let hex = |at: usize| u32::from_str_radix(self.cid.get(at..at + 8)?, 16).ok();
            return Some((hex(0)?, hex(8)?));
        }
        Some((code(self.location.get(..4))?, code(self.location.get(5..9))?))
    }

    /// Whether this build can host it: VST3, VST2 whose id the scan read, and Audio Units
    /// on macOS.
    pub fn hostable(&self) -> bool {
        match self.kind {
            Kind::Vst3 => true,
            Kind::AudioUnit => cfg!(target_os = "macos") && self.maschine_id().is_some(),
            Kind::Vst2 => self.vst2_uid().is_some(),
        }
    }

    /// One line of the scan cache (tab-separated).
    pub fn to_line(&self) -> String {
        format!("{}\t{}\t{}\t{}\t{}\t{}", self.kind.label(), self.vendor, self.name, self.instrument as u8, self.location, self.cid)
    }

    pub fn from_line(line: &str) -> Option<Entry> {
        let f: Vec<&str> = line.split('\t').collect();
        let kind = match *f.first()? {
            "VST3" => Kind::Vst3,
            "VST" => Kind::Vst2,
            "AU" => Kind::AudioUnit,
            _ => return None,
        };
        Some(Entry {
            kind,
            vendor: f.get(1)?.to_string(),
            name: f.get(2)?.to_string(),
            instrument: *f.get(3)? == "1",
            location: f.get(4)?.to_string(),
            cid: f.get(5).unwrap_or(&"").to_string(),
        })
    }
}

fn text(b: &[c_char]) -> String {
    unsafe { CStr::from_ptr(b.as_ptr()) }.to_string_lossy().into_owned()
}

/// VST3 bundles in `dirs` and their subfolders (vendors nest them:
/// `VST3/MeldaProduction/EQ/MEqualizer.vst3`), but not inside a bundle.
pub fn vst3_bundles(dirs: &[PathBuf]) -> Vec<PathBuf> {
    bundles_with(dirs, "vst3", 6)
}

/// VST2 plug-ins in `dirs` and one level of subfolders: `.vst` bundles on macOS, `.so`
/// files on Linux, `.dll`s on Windows. (Deeper, any library a plug-in ships would count.)
pub fn vst2_bundles(dirs: &[PathBuf]) -> Vec<PathBuf> {
    bundles_with(dirs, VST2_EXTENSION, 1)
}

#[cfg(target_os = "linux")]
const VST2_EXTENSION: &str = "so";
#[cfg(windows)]
const VST2_EXTENSION: &str = "dll";
#[cfg(not(any(target_os = "linux", windows)))]
const VST2_EXTENSION: &str = "vst";

/// Files or bundles named `*.ext` in `dirs`, looking `depth` levels of subfolders down.
fn bundles_with(dirs: &[PathBuf], ext: &str, depth: usize) -> Vec<PathBuf> {
    fn walk(dir: &Path, ext: &str, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
                out.push(p);
            } else if depth > 0 && p.is_dir() {
                walk(&p, ext, depth - 1, out);
            }
        }
    }
    let mut out = Vec::new();
    for dir in dirs {
        walk(dir, ext, depth, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// A VST3 bundle's audio-processor classes from its `moduleinfo.json`, without loading
/// it. `None` if it has none.
pub fn vst3_from_moduleinfo(bundle: &Path) -> Option<Vec<Entry>> {
    let text = std::fs::read_to_string(bundle.join("Contents/Resources/moduleinfo.json")).ok()?;
    // moduleinfo.json allows trailing commas (JSON5-ish); drop them.
    let cleaned = text.replace(",\n}", "\n}").replace(",\n]", "\n]");
    let v: serde_json::Value = serde_json::from_str(&cleaned).ok()?;
    let factory_vendor = v.pointer("/Factory Info/Vendor").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let classes = v.get("Classes")?.as_array()?;
    Some(
        classes
            .iter()
            .filter(|c| c.get("Category").and_then(|x| x.as_str()) == Some("Audio Module Class"))
            .map(|c| {
                let subs: Vec<&str> = c
                    .get("Sub Categories")
                    .and_then(|x| x.as_array())
                    .map_or(Vec::new(), |a| a.iter().filter_map(|s| s.as_str()).collect());
                Entry {
                    kind: Kind::Vst3,
                    vendor: c
                        .get("Vendor")
                        .and_then(|x| x.as_str())
                        .filter(|s| !s.is_empty())
                        .map_or(factory_vendor.clone(), str::to_string),
                    name: c.get("Name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    instrument: subs.iter().any(|s| s.contains("Instrument")),
                    location: bundle.display().to_string(),
                    cid: c.get("CID").and_then(|x| x.as_str()).unwrap_or("").to_uppercase(),
                }
            })
            .collect(),
    )
}

/// A VST3 bundle's audio-processor classes, by loading it (do this in a helper process:
/// a plug-in can crash or hang while loading).
pub fn vst3_by_loading(bundle: &Path) -> Result<Vec<Entry>, String> {
    let m = Module::load(bundle)?;
    let mut out = Vec::new();
    let f2: Option<ComPtr<IPluginFactory2>> = m.factory.cast();
    let mut factory_info: PFactoryInfo = unsafe { std::mem::zeroed() };
    unsafe { m.factory.getFactoryInfo(&mut factory_info) };
    let factory_vendor = text(&factory_info.vendor);
    for i in 0..unsafe { m.factory.countClasses() } {
        let (cid, category, name, subs, vendor) = match &f2 {
            Some(f2) => {
                let mut info: PClassInfo2 = unsafe { std::mem::zeroed() };
                if unsafe { f2.getClassInfo2(i, &mut info) } != kResultOk {
                    continue;
                }
                (info.cid, text(&info.category), text(&info.name), text(&info.subCategories), text(&info.vendor))
            }
            None => {
                let mut info: PClassInfo = unsafe { std::mem::zeroed() };
                if unsafe { m.factory.getClassInfo(i, &mut info) } != kResultOk {
                    continue;
                }
                (info.cid, text(&info.category), text(&info.name), String::new(), String::new())
            }
        };
        if category != "Audio Module Class" {
            continue;
        }
        let cid: [u8; 16] = unsafe { std::mem::transmute::<TUID, [u8; 16]>(cid) };
        out.push(Entry {
            kind: Kind::Vst3,
            vendor: if vendor.is_empty() { factory_vendor.clone() } else { vendor },
            name,
            // Old factories have no sub-categories: count those as instruments.
            instrument: subs.contains("Instrument") || subs.is_empty(),
            location: bundle.display().to_string(),
            cid: cid.iter().map(|b| format!("{b:02X}")).collect(),
        });
    }
    Ok(out)
}

/// A VST2 plug-in's entry, from what it said about itself.
fn vst2_entry(info: crate::vst2::Info, location: &Path) -> Entry {
    let name =
        if info.name.is_empty() { location.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned()) } else { info.name };
    Entry {
        kind: Kind::Vst2,
        vendor: info.vendor,
        name,
        instrument: info.instrument,
        location: location.display().to_string(),
        cid: format!("{:08X}", info.unique_id),
    }
}

/// A VST2 plug-in's entry, by loading it (do this in a helper process: a plug-in can crash
/// or hang while loading).
pub fn vst2_by_loading(path: &Path) -> Result<Vec<Entry>, String> {
    crate::vst2::probe(path).map(|info| vec![vst2_entry(info, path)])
}

/// MPC plug-in folders (a `plugin.json` beside the binaries) in `dirs`, their subfolders
/// and theirs (`<drive>/VST2 Plugins/<folder>`), read without loading anything. Only
/// folders with a binary for this machine count.
pub fn mpc_entries(dirs: &[PathBuf]) -> Vec<Entry> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<Entry>) {
        if dir.join("plugin.json").is_file() {
            if let Some((info, file)) = crate::vst2::mpc_folder(dir) {
                out.push(vst2_entry(info, &file));
            }
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        if depth == 0 {
            return;
        }
        let mut subs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        subs.sort();
        for p in subs {
            walk(&p, depth - 1, out);
        }
    }
    let mut out = Vec::new();
    for dir in dirs {
        walk(dir, 2, &mut out);
    }
    out.dedup();
    out
}

/// The standard VST2 folders.
pub fn default_vst2_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut dirs = vec![PathBuf::from("/Library/Audio/Plug-Ins/VST")];
        if let Some(home) = std::env::var_os("HOME") {
            dirs.insert(0, PathBuf::from(home).join("Library/Audio/Plug-Ins/VST"));
        }
        dirs
    }
    #[cfg(target_os = "linux")]
    {
        // `$VST_PATH` (colon-separated) is the convention Linux hosts share.
        let mut dirs: Vec<PathBuf> = std::env::var_os("VST_PATH").map_or(Vec::new(), |p| std::env::split_paths(&p).collect());
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".vst"));
        }
        dirs.extend(["/usr/lib/vst", "/usr/local/lib/vst"].map(PathBuf::from));
        dirs.dedup();
        dirs
    }
    #[cfg(windows)]
    {
        let programs = std::env::var_os("ProgramFiles").map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from);
        let common = std::env::var_os("CommonProgramFiles").map_or_else(|| programs.join("Common Files"), PathBuf::from);
        // Native Instruments' installers put their VST plug-ins in a folder of their own.
        vec![
            programs.join("VstPlugins"),
            programs.join(r"Steinberg\VstPlugins"),
            programs.join(r"Native Instruments\VSTPlugins 64 bit"),
            common.join("VST2"),
        ]
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    Vec::new()
}

pub use module::default_search_paths as default_vst3_paths;

/// Installed Audio Units (instruments, effects and MIDI-controlled effects).
#[cfg(target_os = "macos")]
pub fn audio_units() -> Vec<Entry> {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Desc {
        kind: u32,
        sub: u32,
        manufacturer: u32,
        flags: u32,
        mask: u32,
    }
    #[link(name = "AudioToolbox", kind = "framework")]
    unsafe extern "C" {
        fn AudioComponentFindNext(after: *mut c_void, desc: *const Desc) -> *mut c_void;
        fn AudioComponentGetDescription(c: *mut c_void, desc: *mut Desc) -> i32;
        fn AudioComponentCopyName(c: *mut c_void, name: *mut *const c_void) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(s: *const c_void, buf: *mut c_char, len: isize, enc: u32) -> u8;
        fn CFRelease(cf: *const c_void);
    }
    let fourcc = |v: u32| String::from_utf8_lossy(&v.to_be_bytes()).into_owned();
    let mut out = Vec::new();
    for kind in [*b"aumu", *b"aufx", *b"aumf"] {
        let want = Desc { kind: u32::from_be_bytes(kind), ..Desc::default() };
        let mut c = std::ptr::null_mut();
        loop {
            c = unsafe { AudioComponentFindNext(c, &want) };
            if c.is_null() {
                break;
            }
            let mut d = Desc::default();
            unsafe { AudioComponentGetDescription(c, &mut d) };
            let mut name_ref = std::ptr::null();
            let mut full = String::new();
            if unsafe { AudioComponentCopyName(c, &mut name_ref) } == 0 && !name_ref.is_null() {
                let mut buf = [0 as c_char; 512];
                if unsafe { CFStringGetCString(name_ref, buf.as_mut_ptr(), buf.len() as isize, 0x0800_0100) } != 0 {
                    full = text(&buf);
                }
                unsafe { CFRelease(name_ref) };
            }
            // "Vendor: Name"
            let (vendor, name) = full.split_once(": ").map_or((String::new(), full.clone()), |(v, n)| (v.to_string(), n.to_string()));
            out.push(Entry {
                kind: Kind::AudioUnit,
                vendor,
                name,
                instrument: &kind == b"aumu",
                location: format!("{} {} {}", fourcc(d.kind), fourcc(d.sub), fourcc(d.manufacturer)),
                cid: format!("{:08X}{:08X}{:08X}", d.kind, d.sub, d.manufacturer),
            });
        }
    }
    out.sort();
    out
}

#[cfg(not(target_os = "macos"))]
pub fn audio_units() -> Vec<Entry> {
    Vec::new()
}

#[cfg(test)]
mod au_tests {
    use super::*;

    fn entry(location: &str, cid: &str) -> Entry {
        Entry {
            kind: Kind::AudioUnit,
            vendor: "Output".into(),
            name: "Portal".into(),
            instrument: false,
            location: location.into(),
            cid: cid.into(),
        }
    }

    #[test]
    fn nested_vst3_bundles_are_found_but_not_inside_a_bundle() {
        let root = std::env::temp_dir().join(format!("deckard-scan-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let make = |path: &str| {
            let p = root.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"").unwrap();
        };
        make("Top.vst3");
        make("Vendor/One.VST3");
        make("Melda/EQ/Deep.vst3");
        // A bundle is a folder on macOS and Linux: what it holds isn't another plug-in.
        make("Bundle.vst3/Contents/Inner.vst3");
        // VST2 stays one level down, where its libraries aren't mistaken for plug-ins.
        make("Top.dll");
        make("Vendor/One.dll");
        make("Vendor/Support/Library.dll");
        let names = |found: Vec<PathBuf>| -> Vec<String> {
            found.iter().map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")).collect()
        };
        let dirs = [root.clone()];
        let vst3 = names(vst3_bundles(&dirs));
        let vst2 = names(bundles_with(&dirs, "dll", 1));
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(vst3, ["Bundle.vst3", "Melda/EQ/Deep.vst3", "Top.vst3", "Vendor/One.VST3"]);
        assert_eq!(vst2, ["Top.dll", "Vendor/One.dll"]);
    }

    #[test]
    fn an_au_has_maschines_id() {
        // Scanned now, and from a cache from before AUs could be hosted.
        let id = Some("Output:AU:FX:1347572812".to_string());
        assert_eq!(entry("aufx PRTL Outp", "").maschine_id(), id);
        assert_eq!(
            entry(
                "?",
                &format!("{:08X}{:08X}{:08X}", u32::from_be_bytes(*b"aufx"), u32::from_be_bytes(*b"PRTL"), u32::from_be_bytes(*b"Outp"))
            )
            .maschine_id(),
            id
        );
        assert_eq!(entry("aumf PRTL Outp", "").maschine_id().as_deref(), Some("Output:AU:MF:1347572812"));
        assert_eq!(entry("aumf PRTL Outp", "").hostable(), cfg!(target_os = "macos"));
    }

    #[test]
    fn vst2_entries_are_hostable_once_their_id_is_read() {
        let mut e = Entry {
            kind: Kind::Vst2,
            vendor: "Acme".into(),
            name: "Fake".into(),
            instrument: true,
            location: "/x/Fake.so".into(),
            cid: String::new(),
        };
        // From a cache made before VST2 could be hosted: listed, not offered.
        assert!(!e.hostable());
        assert_eq!(e.maschine_id(), None);
        e.cid = "41634662".into();
        assert!(e.hostable());
        assert_eq!(e.vst2_uid(), Some(0x4163_4662));
        // Maschine's id: the unique id in decimal, as it saves VST2 plug-ins.
        assert_eq!(e.maschine_id().as_deref(), Some("Acme:VST:INSTR:1097025122"));
        assert_eq!(Entry::from_line(&e.to_line()), Some(e.clone()));
        // A project naming it finds the id back.
        let id = crate::PluginId::parse(&e.maschine_id().unwrap(), "Fake").unwrap();
        assert_eq!(crate::state::vst2_uid(&id.cid), Some(0x4163_4662));
    }

    #[test]
    fn mpc_folders_are_found_in_a_vst2_plugins_folder() {
        let root = std::env::temp_dir().join(format!("deckard-mpc-scan-{}", std::process::id()));
        let dir = root.join("VST2 Plugins").join("Acme - VST - Fake");
        std::fs::create_dir_all(dir.join("VST")).unwrap();
        for f in ["fake-arm64.so", "fake-arm32.so"] {
            std::fs::write(dir.join("VST").join(f), b"").unwrap();
        }
        std::fs::write(
            dir.join("plugin.json"),
            r#"{"name":"Fake","manufacturer":"Acme","kind":"effect","uid":"AcFk","file32":"VST/fake-arm32.so","file64":"VST/fake-arm64.so"}"#,
        )
        .unwrap();
        let found = mpc_entries(std::slice::from_ref(&root));
        if cfg!(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm"))) {
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].name.as_str(), found[0].instrument, found[0].cid.as_str()), ("Fake", false, "4163466B"));
        } else {
            assert!(found.is_empty(), "MPC binaries don't run here");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
