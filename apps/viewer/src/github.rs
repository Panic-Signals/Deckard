//! DECKARD's GitHub: the latest release (the update notice and the updater) and the
//! contributors (the About window), from the REST API. Network calls block; callers
//! run them on a thread. The parsing is separate from the fetching so it can be tested
//! with saved answers.

use std::io::Read;
use std::time::Duration;

/// The repository, `owner/name`.
pub const REPO: &str = "Panic-Signals/Deckard";
const API: &str = "https://api.github.com";

/// A release version, `major.minor.patch`.
pub type Version = (u32, u32, u32);

/// `1.2.3` or `v1.2.3` (a tag) as numbers; anything else (`0.1.0-Early-Alpha-Release`,
/// a crate version with a suffix, `main`) is none: not a release version.
pub fn parse_version(text: &str) -> Option<Version> {
    let t = text.trim().trim_start_matches(['v', 'V']);
    let mut parts = t.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// One file attached to a release.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    /// The download URL.
    pub url: String,
    /// `sha256:<hex>` when GitHub gives one.
    pub digest: Option<String>,
    pub size: u64,
}

/// A published release.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    /// The release page.
    pub url: String,
    pub assets: Vec<Asset>,
}

impl Release {
    /// From the API's JSON for one release. None when the tag isn't a release version
    /// or the release is a draft or pre-release.
    pub fn parse(json: &str) -> Option<Release> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
            return None;
        }
        let tag = v["tag_name"].as_str()?.to_string();
        let version = parse_version(&tag)?;
        let assets = v["assets"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|a| {
                        Some(Asset {
                            name: a["name"].as_str()?.to_string(),
                            url: a["browser_download_url"].as_str()?.to_string(),
                            digest: a["digest"].as_str().map(str::to_string),
                            size: a["size"].as_u64().unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Release { version, tag, url: v["html_url"].as_str().unwrap_or_default().to_string(), assets })
    }

    pub fn version_text(&self) -> String {
        let (a, b, c) = self.version;
        format!("{a}.{b}.{c}")
    }
}

/// Someone who committed to the repository.
#[derive(Debug, Clone, PartialEq)]
pub struct Contributor {
    pub login: String,
    pub url: String,
    pub avatar_url: String,
    pub contributions: u64,
}

/// From the API's JSON list of contributors, most contributions first.
pub fn parse_contributors(json: &str) -> Vec<Contributor> {
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let mut out: Vec<Contributor> = list
        .iter()
        .filter(|c| c["type"].as_str() != Some("Bot"))
        .filter_map(|c| {
            Some(Contributor {
                login: c["login"].as_str()?.to_string(),
                url: c["html_url"].as_str().unwrap_or_default().to_string(),
                avatar_url: c["avatar_url"].as_str().unwrap_or_default().to_string(),
                contributions: c["contributions"].as_u64().unwrap_or(0),
            })
        })
        .collect();
    out.sort_by(|a, b| b.contributions.cmp(&a.contributions).then_with(|| a.login.cmp(&b.login)));
    out
}

fn get(url: &str, accept: &str, timeout: Duration) -> Result<ureq::http::Response<ureq::Body>, String> {
    ureq::get(url)
        .config()
        .timeout_global(Some(timeout))
        .build()
        .header("User-Agent", format!("DECKARD/{}", crate::VERSION))
        .header("Accept", accept)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => "not found".to_string(),
            ureq::Error::StatusCode(403) | ureq::Error::StatusCode(429) => "GitHub is rate-limiting this computer; try later".to_string(),
            ureq::Error::StatusCode(n) => format!("GitHub answered {n}"),
            e => format!("{e}"),
        })
}

/// `GET` a JSON API page as text.
fn get_json(path: &str) -> Result<String, String> {
    let mut resp = get(&format!("{API}{path}"), "application/vnd.github+json", Duration::from_secs(20))?;
    resp.body_mut().read_to_string().map_err(|e| e.to_string())
}

/// The latest published release (GitHub's "latest": not a draft or pre-release).
pub fn latest_release() -> Result<Release, String> {
    let json = get_json(&format!("/repos/{REPO}/releases/latest"))?;
    Release::parse(&json).ok_or_else(|| "the latest release has no version number".to_string())
}

/// The contributors, most commits first.
pub fn contributors() -> Result<Vec<Contributor>, String> {
    Ok(parse_contributors(&get_json(&format!("/repos/{REPO}/contributors?per_page=100"))?))
}

/// A small file (an avatar picture) in full, at most `limit` bytes.
pub fn fetch(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut resp = get(url, "*/*", Duration::from_secs(20))?;
    resp.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| e.to_string())
}

/// Download `url` to `into`, reporting bytes so far through `progress`, and give its
/// SHA-256 as `sha256:<hex>`. No overall timeout: a big file on a slow line takes what
/// it takes; the connection itself times out after 30 s of silence.
pub fn download(url: &str, into: &std::path::Path, mut progress: impl FnMut(u64)) -> Result<String, String> {
    use sha2::Digest;
    let mut resp = get(url, "application/octet-stream", Duration::from_secs(30 * 60))?;
    let mut reader = resp.body_mut().with_config().limit(2 * 1024 * 1024 * 1024).reader();
    let mut file = std::fs::File::create(into).map_err(|e| format!("{}: {e}", into.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("download: {e}"))?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n]).map_err(|e| format!("{}: {e}", into.display()))?;
        hasher.update(&buf[..n]);
        total += n as u64;
        progress(total);
    }
    Ok(format!("sha256:{}", hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_versions_are_numbers_only() {
        assert_eq!(parse_version("0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("v1.10.3"), Some((1, 10, 3)));
        assert_eq!(parse_version("0.1.0-Early-Alpha-Release"), None);
        assert_eq!(parse_version("0.1"), None);
        assert_eq!(parse_version("0.1.0.1"), None);
        assert_eq!(parse_version("main"), None);
        assert!(parse_version("0.2.0") > parse_version("0.1.9"));
        assert!(parse_version("1.0.0") > parse_version("0.99.99"));
    }

    const RELEASE: &str = r#"{
        "html_url": "https://github.com/Panic-Signals/Deckard/releases/tag/0.2.0",
        "tag_name": "0.2.0", "name": "0.2.0 Second Alpha", "draft": false, "prerelease": false,
        "assets": [
            {"name": "deckard_0.2.0-1_amd64.deb", "size": 12345678,
             "browser_download_url": "https://github.com/Panic-Signals/Deckard/releases/download/0.2.0/deckard_0.2.0-1_amd64.deb",
             "digest": "sha256:2b323c27db929fb75f29598c9ad49157b224d91cd724868ab726b583a3639245"},
            {"name": "DECKARD-0.2.0-macos-universal.zip", "size": 1,
             "browser_download_url": "https://x/DECKARD-0.2.0-macos-universal.zip", "digest": null}
        ]
    }"#;

    #[test]
    fn a_release_is_read_from_the_api() {
        let r = Release::parse(RELEASE).unwrap();
        assert_eq!(r.version, (0, 2, 0));
        assert_eq!(r.version_text(), "0.2.0");
        assert_eq!(r.tag, "0.2.0");
        assert_eq!(r.url, "https://github.com/Panic-Signals/Deckard/releases/tag/0.2.0");
        assert_eq!(r.assets.len(), 2);
        assert_eq!(r.assets[0].name, "deckard_0.2.0-1_amd64.deb");
        assert_eq!(r.assets[0].size, 12345678);
        assert_eq!(r.assets[0].digest.as_deref(), Some("sha256:2b323c27db929fb75f29598c9ad49157b224d91cd724868ab726b583a3639245"));
        assert_eq!(r.assets[1].digest, None);
        // A title isn't a version; a pre-release or a tag without numbers is no update.
        assert!(Release::parse(&RELEASE.replace("\"prerelease\": false", "\"prerelease\": true")).is_none());
        assert!(Release::parse(&RELEASE.replace("\"tag_name\": \"0.2.0\"", "\"tag_name\": \"nightly\"")).is_none());
        assert!(Release::parse("not json").is_none());
    }

    #[test]
    fn contributors_come_most_commits_first_without_bots() {
        let json = r#"[
            {"login": "helper", "html_url": "https://github.com/helper", "avatar_url": "https://a/helper", "contributions": 2, "type": "User"},
            {"login": "summitt", "html_url": "https://github.com/summitt", "avatar_url": "https://a/summitt", "contributions": 8, "type": "User"},
            {"login": "dependabot[bot]", "html_url": "", "avatar_url": "", "contributions": 50, "type": "Bot"}
        ]"#;
        let c = parse_contributors(json);
        assert_eq!(c.iter().map(|c| c.login.as_str()).collect::<Vec<_>>(), ["summitt", "helper"]);
        assert_eq!(c[0].avatar_url, "https://a/summitt");
        assert!(parse_contributors("{}").is_empty());
    }
}
