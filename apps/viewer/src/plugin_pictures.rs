//! DECKARD's own pictures of its plug-ins for the controller's displays (tiles in
//! the chain, logos on the right display), made by the `make_plugin_pictures` test.

/// (style sheet key, thumbnail PNG, logo PNG).
pub const PICTURES: &[(&str, &[u8], &[u8])] = &[
    ("audio", include_bytes!("../assets/plugins/audio_Thumb.png"), include_bytes!("../assets/plugins/audio_Logo.png")),
    ("basssynth", include_bytes!("../assets/plugins/basssynth_Thumb.png"), include_bytes!("../assets/plugins/basssynth_Logo.png")),
    ("beatdelay", include_bytes!("../assets/plugins/beatdelay_Thumb.png"), include_bytes!("../assets/plugins/beatdelay_Logo.png")),
    ("chorus", include_bytes!("../assets/plugins/chorus_Thumb.png"), include_bytes!("../assets/plugins/chorus_Logo.png")),
    ("compressor", include_bytes!("../assets/plugins/compressor_Thumb.png"), include_bytes!("../assets/plugins/compressor_Logo.png")),
    ("cymbal", include_bytes!("../assets/plugins/cymbal_Thumb.png"), include_bytes!("../assets/plugins/cymbal_Logo.png")),
    ("distortion", include_bytes!("../assets/plugins/distortion_Thumb.png"), include_bytes!("../assets/plugins/distortion_Logo.png")),
    ("eq", include_bytes!("../assets/plugins/eq_Thumb.png"), include_bytes!("../assets/plugins/eq_Logo.png")),
    ("filter", include_bytes!("../assets/plugins/filter_Thumb.png"), include_bytes!("../assets/plugins/filter_Logo.png")),
    ("flanger", include_bytes!("../assets/plugins/flanger_Thumb.png"), include_bytes!("../assets/plugins/flanger_Logo.png")),
    ("fm", include_bytes!("../assets/plugins/fm_Thumb.png"), include_bytes!("../assets/plugins/fm_Logo.png")),
    ("freqshifter", include_bytes!("../assets/plugins/freqshifter_Thumb.png"), include_bytes!("../assets/plugins/freqshifter_Logo.png")),
    ("gate", include_bytes!("../assets/plugins/gate_Thumb.png"), include_bytes!("../assets/plugins/gate_Logo.png")),
    ("graindelay", include_bytes!("../assets/plugins/graindelay_Thumb.png"), include_bytes!("../assets/plugins/graindelay_Logo.png")),
    ("grainstretch", include_bytes!("../assets/plugins/grainstretch_Thumb.png"), include_bytes!("../assets/plugins/grainstretch_Logo.png")),
    ("hi-hat", include_bytes!("../assets/plugins/hi-hat_Thumb.png"), include_bytes!("../assets/plugins/hi-hat_Logo.png")),
    ("ice", include_bytes!("../assets/plugins/ice_Thumb.png"), include_bytes!("../assets/plugins/ice_Logo.png")),
    ("kick", include_bytes!("../assets/plugins/kick_Thumb.png"), include_bytes!("../assets/plugins/kick_Logo.png")),
    ("legacyreverb", include_bytes!("../assets/plugins/legacyreverb_Thumb.png"), include_bytes!("../assets/plugins/legacyreverb_Logo.png")),
    ("limiter", include_bytes!("../assets/plugins/limiter_Thumb.png"), include_bytes!("../assets/plugins/limiter_Logo.png")),
    ("lo-fi", include_bytes!("../assets/plugins/lo-fi_Thumb.png"), include_bytes!("../assets/plugins/lo-fi_Logo.png")),
    ("maximizer", include_bytes!("../assets/plugins/maximizer_Thumb.png"), include_bytes!("../assets/plugins/maximizer_Logo.png")),
    ("metaverb", include_bytes!("../assets/plugins/metaverb_Thumb.png"), include_bytes!("../assets/plugins/metaverb_Logo.png")),
    ("percussion", include_bytes!("../assets/plugins/percussion_Thumb.png"), include_bytes!("../assets/plugins/percussion_Logo.png")),
    ("phaser", include_bytes!("../assets/plugins/phaser_Thumb.png"), include_bytes!("../assets/plugins/phaser_Logo.png")),
    ("plaits", include_bytes!("../assets/plugins/plaits_Thumb.png"), include_bytes!("../assets/plugins/plaits_Logo.png")),
    ("polysynth", include_bytes!("../assets/plugins/polysynth_Thumb.png"), include_bytes!("../assets/plugins/polysynth_Logo.png")),
    ("reflex", include_bytes!("../assets/plugins/reflex_Thumb.png"), include_bytes!("../assets/plugins/reflex_Logo.png")),
    ("replicant", include_bytes!("../assets/plugins/replicant_Thumb.png"), include_bytes!("../assets/plugins/replicant_Logo.png")),
    ("resochord", include_bytes!("../assets/plugins/resochord_Thumb.png"), include_bytes!("../assets/plugins/resochord_Logo.png")),
    ("reverb", include_bytes!("../assets/plugins/reverb_Thumb.png"), include_bytes!("../assets/plugins/reverb_Logo.png")),
    ("sampler", include_bytes!("../assets/plugins/sampler_Thumb.png"), include_bytes!("../assets/plugins/sampler_Logo.png")),
    ("saturator", include_bytes!("../assets/plugins/saturator_Thumb.png"), include_bytes!("../assets/plugins/saturator_Logo.png")),
    ("snare", include_bytes!("../assets/plugins/snare_Thumb.png"), include_bytes!("../assets/plugins/snare_Logo.png")),
    ("tom", include_bytes!("../assets/plugins/tom_Thumb.png"), include_bytes!("../assets/plugins/tom_Logo.png")),
    (
        "transientmaster",
        include_bytes!("../assets/plugins/transientmaster_Thumb.png"),
        include_bytes!("../assets/plugins/transientmaster_Logo.png"),
    ),
];

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Whether DECKARD has a picture of the plug-in with this key.
pub fn has(key: &str) -> bool {
    PICTURES.iter().any(|(k, ..)| *k == key)
}

/// Give DECKARD's controller layer the pictures (thumbnails and logos).
pub fn give(s: &mut deckard_surface::Surface) {
    for (key, thumb, logo) in PICTURES {
        for (bytes, is_thumb) in [(*thumb, true), (*logo, false)] {
            if let Some((w, h, px)) = decode(bytes) {
                s.add_picture(key, is_thumb, w, h, px);
            }
        }
    }
}

/// The picture key for a chain slot (`s.name`): DECKARD's own built-in picture if it
/// has one, else the plug-in's thumbnail and logo from `NI Resources\image` (loaded on
/// demand into `s`).
pub fn slot_key(s: &deckard_surface::Surface, name: &str) -> Option<String> {
    let key = deckard_lua_host::plugin_picture_key(name);
    if key.is_empty() {
        return None;
    }
    if has(&key) || s.has_picture(&key, true) || s.has_picture(&key, false) {
        return Some(key);
    }
    with_ni_pictures(|ni| ni.load_slot(s, name, &key)).then_some(key)
}

/// Ensure the controller's surface has the browser tile (`thumb = true`) and right-screen
/// logo (`thumb = false`) for `product` under `"artwork:<lower>"`.
pub fn ensure_browser_product(s: &deckard_surface::Surface, product: &str) {
    let lower = product.trim().to_lowercase();
    if lower.is_empty() {
        return;
    }
    let key = format!("artwork:{lower}");
    if s.has_picture(&key, true) && s.has_picture(&key, false) {
        return;
    }
    with_ni_pictures(|ni| {
        ni.load_browser_product(s, &lower, &key);
    });
}

#[derive(Clone, Default)]
struct Entry {
    artwork: Option<PathBuf>,
    plugin: Option<PathBuf>,
    logo: Option<PathBuf>,
}

impl Entry {
    fn merge(&mut self, other: &Self) {
        if self.artwork.is_none() {
            self.artwork.clone_from(&other.artwork);
        }
        if self.plugin.is_none() {
            self.plugin.clone_from(&other.plugin);
        }
        if self.logo.is_none() {
            self.logo.clone_from(&other.logo);
        }
    }
}

type DecodedImage = Option<(usize, usize, Vec<[u8; 4]>)>;

#[derive(Default)]
struct NiPictures {
    root: Option<PathBuf>,
    by_name: HashMap<String, Entry>,
    by_key: HashMap<String, Entry>,
    decoded: HashMap<PathBuf, DecodedImage>,
}

fn ni_image_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("DECKARD_NI_IMAGE_DIR") {
        let p = PathBuf::from(dir);
        return p.is_dir().then_some(p);
    }
    let mut candidates =
        vec![PathBuf::from(r"C:\Users\Public\Documents\NI Resources\image"), PathBuf::from("/Users/Shared/NI Resources/image")];
    if let Some(pub_dir) = std::env::var_os("PUBLIC") {
        candidates.insert(0, PathBuf::from(pub_dir).join(r"Documents\NI Resources\image"));
    }
    candidates.into_iter().find(|p| p.is_dir())
}

fn strip_hash(name: &str) -> &str {
    if name.len() > 33 && name.is_char_boundary(name.len() - 33) {
        let (stem, suffix) = name.split_at(name.len() - 33);
        if suffix.starts_with('_') && suffix[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
            return stem;
        }
    }
    name
}

fn candidate_keys(name: &str) -> Vec<String> {
    let base = deckard_lua_host::plugin_picture_key(name);
    if base.is_empty() {
        return Vec::new();
    }
    let mut out = vec![base.clone()];
    let no_fx = base.strip_suffix("fx").filter(|s| !s.is_empty()).unwrap_or(&base);
    if no_fx != base {
        out.push(no_fx.to_string());
    }
    for s in [base.as_str(), no_fx] {
        let no_digits = s.trim_end_matches(|c: char| c.is_ascii_digit());
        if !no_digits.is_empty() && !out.iter().any(|x| x == no_digits) {
            out.push(no_digits.to_string());
        }
    }
    out
}

impl NiPictures {
    fn scan(root: Option<PathBuf>) -> Self {
        let mut ni = Self { root: root.clone(), ..Self::default() };
        if let Some(dir) = root {
            Self::walk(&dir, 3, &mut ni);
        }
        ni
    }

    fn walk(dir: &Path, depth: usize, ni: &mut Self) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for path in dirs {
            let artwork = ["MST_artwork.png", "VB_artwork.png"].into_iter().map(|f| path.join(f)).find(|p| p.is_file());
            let plugin = ["MST_plugin.png"].into_iter().map(|f| path.join(f)).find(|p| p.is_file());
            let logo = ["MST_logo.png", "VB_logo.png"].into_iter().map(|f| path.join(f)).find(|p| p.is_file());
            if artwork.is_some() || plugin.is_some() || logo.is_some() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    let entry = Entry { artwork, plugin, logo };
                    let lower = name.trim().to_lowercase();
                    let clean = strip_hash(&lower).trim().to_string();
                    ni.by_name.entry(lower).or_default().merge(&entry);
                    ni.by_name.entry(clean.clone()).or_default().merge(&entry);
                    for k in candidate_keys(&clean) {
                        ni.by_key.entry(k).or_default().merge(&entry);
                    }
                }
            }
            if depth > 1 {
                Self::walk(&path, depth - 1, ni);
            }
        }
    }

    fn lookup(&self, name: &str) -> Option<Entry> {
        let lower = strip_hash(&name.trim().to_lowercase()).trim().to_string();
        let mut found = self.by_name.get(&lower).cloned();
        for k in candidate_keys(&lower) {
            if let Some(e) = self.by_key.get(&k) {
                match &mut found {
                    Some(cur) => cur.merge(e),
                    None => found = Some(e.clone()),
                }
            }
        }
        found
    }

    fn decode_cached(&mut self, path: &Path) -> DecodedImage {
        if let Some(cached) = self.decoded.get(path) {
            return cached.clone();
        }
        let img = std::fs::read(path).ok().and_then(|b| decode(&b));
        self.decoded.insert(path.to_path_buf(), img.clone());
        img
    }

    fn load_slot(&mut self, s: &deckard_surface::Surface, name: &str, key: &str) -> bool {
        let Some(entry) = self.lookup(name) else { return false };
        let thumb_path = entry.plugin.as_ref().or(entry.artwork.as_ref()).or(entry.logo.as_ref()).cloned();
        let logo_path = entry.logo.as_ref().or(entry.plugin.as_ref()).or(entry.artwork.as_ref()).cloned();
        let mut any = false;
        if let Some(p) = thumb_path {
            if let Some((w, h, px)) = self.decode_cached(&p) {
                s.add_picture(key, true, w, h, px);
                any = true;
            }
        }
        if let Some(p) = logo_path {
            if let Some((w, h, px)) = self.decode_cached(&p) {
                s.add_picture(key, false, w, h, px);
                any = true;
            }
        }
        any
    }

    fn load_browser_product(&mut self, s: &deckard_surface::Surface, product_lower: &str, key: &str) -> bool {
        let Some(entry) = self.lookup(product_lower) else { return false };
        let thumb_path = entry.artwork.as_ref().or(entry.plugin.as_ref()).or(entry.logo.as_ref()).cloned();
        let logo_path = entry.logo.as_ref().or(entry.artwork.as_ref()).or(entry.plugin.as_ref()).cloned();
        let mut any = false;
        if let Some(p) = thumb_path {
            if let Some((w, h, px)) = self.decode_cached(&p) {
                s.add_picture(key, true, w, h, px);
                any = true;
            }
        }
        if let Some(p) = logo_path {
            if let Some((w, h, px)) = self.decode_cached(&p) {
                s.add_picture(key, false, w, h, px);
                any = true;
            }
        }
        any
    }
}

fn with_ni_pictures<R>(f: impl FnOnce(&mut NiPictures) -> R) -> R {
    static NI: OnceLock<Mutex<NiPictures>> = OnceLock::new();
    let root = ni_image_root();
    let mut guard = NI.get_or_init(|| Mutex::new(NiPictures::scan(root.clone()))).lock().expect("ni pictures lock");
    if guard.root != root {
        *guard = NiPictures::scan(root);
    }
    f(&mut guard)
}

/// A PNG as RGBA pixels (w, h, pixels).
fn decode(bytes: &[u8]) -> Option<(usize, usize, Vec<[u8; 4]>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let buf = &buf[..info.buffer_size()];
    let px = match info.color_type {
        png::ColorType::Rgba => buf.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
        png::ColorType::Rgb => buf.chunks(3).map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks(2).map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().map(|&g| [g, g, g, 255]).collect(),
        _ => return None,
    };
    Some((info.width as usize, info.height as usize, px))
}
