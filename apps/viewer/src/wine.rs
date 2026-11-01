//! Windows plug-ins on Linux, through Wine and yabridge. NI's plug-ins have no Linux
//! versions; installed into Wine, yabridge presents them as VST3 bundles in
//! `~/.vst3/yabridge`, which DECKARD hosts like any other. This module tells what is
//! installed ([`Status`]), installs what isn't (Wine from the distribution's packages
//! with administrator rights through `pkexec`; yabridge from its own release archive,
//! `scripts/install-yabridge.sh`, or, for Wine 9.22 and later, built from its source,
//! `scripts/build-yabridge.sh`), and runs `yabridgectl sync`, which makes the bundles.
//!
//! Compiles everywhere (it is only process calls and paths) so its tests run on every
//! system; the app shows it on Linux.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The family of Linux distribution, for its package manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Debian, Ubuntu, Mint, Pop!_OS…: `apt-get`.
    Debian,
    /// Fedora, RHEL, Nobara…: `dnf`.
    Fedora,
    /// Arch, Manjaro, CachyOS…: `pacman`.
    Arch,
    Other,
}

impl Family {
    /// From `/etc/os-release`'s text: `ID` first, then `ID_LIKE` (Mint says
    /// `ID=linuxmint`, `ID_LIKE="ubuntu debian"`).
    pub fn parse(os_release: &str) -> Family {
        let mut id = None;
        let mut like = Vec::new();
        for line in os_release.lines() {
            let unquote = |v: &str| v.trim().trim_matches('"').to_lowercase();
            if let Some(v) = line.strip_prefix("ID=") {
                id = Some(unquote(v));
            } else if let Some(v) = line.strip_prefix("ID_LIKE=") {
                like.extend(unquote(v).split_whitespace().map(str::to_string));
            }
        }
        for name in id.into_iter().chain(like) {
            match name.as_str() {
                "debian" | "ubuntu" => return Family::Debian,
                "fedora" | "rhel" | "centos" => return Family::Fedora,
                "arch" => return Family::Arch,
                _ => {}
            }
        }
        Family::Other
    }

    /// This system's.
    pub fn detect() -> Family {
        std::fs::read_to_string("/etc/os-release").map_or(Family::Other, |t| Family::parse(&t))
    }

    /// The shell command that installs Wine, to run as root.
    pub fn wine_command(self) -> Option<&'static str> {
        match self {
            Family::Debian => Some("export DEBIAN_FRONTEND=noninteractive; apt-get update && apt-get install -y wine"),
            Family::Fedora => Some("dnf install -y wine"),
            Family::Arch => Some("pacman -S --noconfirm --needed wine"),
            Family::Other => None,
        }
    }
}

/// Wine's major and minor version from `wine --version` (`wine-9.21 (Staging)`,
/// `wine-11.0`).
pub fn parse_version(text: &str) -> Option<(u32, u32)> {
    let rest = text.trim().strip_prefix("wine-")?;
    let mut parts = rest.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0);
    Some((major, minor))
}

/// yabridge's 5.1.1 release (its last) predates Wine 9.22, whose window changes keep
/// mouse clicks and keys from reaching plug-in windows through it (yabridge #382,
/// #409); its master branch fixes that.
pub fn newer_than_yabridge_supports(version: (u32, u32)) -> bool {
    version >= (9, 22)
}

/// The Wine prefix: `$WINEPREFIX`, else `~/.wine`.
pub fn wine_prefix() -> PathBuf {
    if let Some(p) = std::env::var_os("WINEPREFIX") {
        return PathBuf::from(p);
    }
    home().join(".wine")
}

/// Where Windows installers put VST3 plug-ins inside the prefix.
pub fn windows_vst3_dir() -> PathBuf {
    wine_prefix().join("drive_c/Program Files/Common Files/VST3")
}

/// Where `yabridgectl sync` puts the bridged VST3 bundles.
pub fn bridged_dir() -> PathBuf {
    home().join(".vst3/yabridge")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// Where yabridge's archive is unpacked (its README's place).
pub fn yabridge_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map_or_else(|| home().join(".local/share"), PathBuf::from).join("yabridge")
}

/// `yabridgectl`: in yabridge's folder, on the PATH, or the distribution's.
pub fn find_yabridgectl() -> Option<PathBuf> {
    let mut candidates = vec![yabridge_dir().join("yabridgectl")];
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join("yabridgectl")));
    }
    candidates.push(PathBuf::from("/usr/bin/yabridgectl"));
    candidates.into_iter().find(|p| p.is_file())
}

/// `.vst3` entries directly in `dir` (bundles are folders; an old Windows plug-in is
/// one file).
pub fn count_bundles(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map_or(0, |rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("vst3"))).count())
}

/// yabridge's build marker, `BUILD` in its folder, written by `scripts/build-yabridge.sh`
/// (`yabridge master <commit> built <date> for wine-11.0 by DECKARD`). None for a
/// release archive.
pub fn yabridge_build() -> Option<String> {
    std::fs::read_to_string(yabridge_dir().join("BUILD")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// What `build-yabridge.sh --check` found: the packages missing to build yabridge, and
/// the command (to run as root) that installs them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildCheck {
    pub missing: Vec<String>,
    pub root_command: Option<String>,
}

impl BuildCheck {
    /// From the script's output: `ready`, or `missing: a b c` and `root: <command>`.
    pub fn parse(text: &str) -> BuildCheck {
        let mut c = BuildCheck::default();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("missing:") {
                c.missing = rest.split_whitespace().map(str::to_string).collect();
            } else if let Some(rest) = line.strip_prefix("root:") {
                let cmd = rest.trim();
                c.root_command = (!cmd.is_empty()).then(|| cmd.to_string());
            }
        }
        c
    }

    /// Everything the build needs is there.
    pub fn ready(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Ask the build script what's missing.
pub fn build_check() -> BuildCheck {
    let Some(script) = crate::stems::script("build-yabridge.sh") else {
        return BuildCheck { missing: vec!["build-yabridge.sh (not with DECKARD)".into()], root_command: None };
    };
    match Command::new("bash").arg(&script).arg("--check").output() {
        Ok(o) => BuildCheck::parse(&String::from_utf8_lossy(&o.stdout)),
        Err(e) => BuildCheck { missing: vec![format!("{e}")], root_command: None },
    }
}

/// What is installed, as Preferences shows it.
#[derive(Debug, Clone)]
pub struct Status {
    pub family: Family,
    /// Wine's version text (`11.0`), if it runs.
    pub wine: Option<String>,
    pub yabridgectl: Option<PathBuf>,
    /// yabridge was built from source by DECKARD ([`yabridge_build`]).
    pub yabridge_build: Option<String>,
    /// What building yabridge would still need.
    pub build: BuildCheck,
    pub windows_vst3: PathBuf,
    /// Plug-ins installed into Wine.
    pub windows_plugins: usize,
    /// Bundles in `~/.vst3/yabridge`.
    pub bridged: usize,
    /// Administrator rights can be asked for (polkit).
    pub pkexec: bool,
}

impl Status {
    pub fn probe() -> Status {
        let wine = Command::new("wine")
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty());
        let windows_vst3 = windows_vst3_dir();
        let wine_version = wine.as_deref().and_then(parse_version);
        let yabridge_build = yabridge_build();
        // The build's prerequisites are only worth checking when a build is wanted.
        let build = if wine_version.is_some_and(newer_than_yabridge_supports) && yabridge_build.is_none() {
            build_check()
        } else {
            BuildCheck::default()
        };
        Status {
            family: Family::detect(),
            wine,
            yabridgectl: find_yabridgectl(),
            yabridge_build,
            build,
            windows_plugins: count_bundles(&windows_vst3),
            windows_vst3,
            bridged: count_bundles(&bridged_dir()),
            pkexec: Path::new("/usr/bin/pkexec").is_file(),
        }
    }

    pub fn wine_version(&self) -> Option<(u32, u32)> {
        self.wine.as_deref().and_then(parse_version)
    }

    /// Wine is 9.22 or later and yabridge is (or would be) the 5.1.1 release: plug-in
    /// windows would take no clicks or keys. yabridge has to be built from its source.
    pub fn yabridge_needs_build(&self) -> bool {
        self.wine_version().is_some_and(newer_than_yabridge_supports) && self.yabridge_build.is_none()
    }
}

fn last_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).lines().filter(|l| !l.trim().is_empty()).next_back().unwrap_or("").to_string()
}

/// Run a shell command as root, asking for an administrator's password through
/// `pkexec`; `done` is the message when it worked, `what` names the task in errors.
fn run_as_root(cmd: &str, what: &str, done: &str) -> Result<String, String> {
    let out = Command::new("pkexec").args(["sh", "-c", cmd]).output().map_err(|e| format!("pkexec: {e}"))?;
    match out.status.code() {
        Some(0) => Ok(done.into()),
        Some(126) => Err("cancelled".into()),
        Some(127) => Err("not allowed by the system's policy".into()),
        _ => Err(format!("{what} failed: {}", last_line(&out.stderr))),
    }
}

/// Install Wine with the distribution's package manager, asking for an administrator's
/// password through `pkexec`.
pub fn install_wine(family: Family) -> Result<String, String> {
    let cmd = family.wine_command().ok_or("no known package manager for this distribution")?;
    run_as_root(cmd, "installing Wine", "Wine installed")
}

/// Install what building yabridge needs (`cmd` from [`BuildCheck::root_command`]).
pub fn install_build_tools(cmd: &str) -> Result<String, String> {
    run_as_root(cmd, "installing the build tools", "build tools installed")
}

/// Build yabridge from its source and install it (`scripts/build-yabridge.sh`; minutes).
pub fn build_yabridge() -> Result<String, String> {
    let script = crate::stems::script("build-yabridge.sh").ok_or("yabridge's build script isn't with DECKARD")?;
    let out = Command::new("bash").arg(&script).output().map_err(|e| format!("{}: {e}", script.display()))?;
    if out.status.success() { Ok(last_line(&out.stdout)) } else { Err(format!("building yabridge failed: {}", last_line(&out.stderr))) }
}

/// Install yabridge into [`yabridge_dir`] (`scripts/install-yabridge.sh`: a download).
pub fn install_yabridge() -> Result<String, String> {
    let script = crate::stems::script("install-yabridge.sh").ok_or("yabridge's installer isn't with DECKARD")?;
    let out = Command::new("bash").arg(&script).output().map_err(|e| format!("{}: {e}", script.display()))?;
    if out.status.success() { Ok(last_line(&out.stdout)) } else { Err(format!("installing yabridge failed: {}", last_line(&out.stderr))) }
}

/// Make (or refresh) the bridged bundles: register the prefix's VST3 folder with
/// yabridgectl and `sync`.
pub fn sync() -> Result<String, String> {
    let ctl = find_yabridgectl().ok_or("yabridge isn't installed")?;
    let dir = windows_vst3_dir();
    if dir.is_dir() {
        // Fails when the folder is already registered: fine.
        let _ = Command::new(&ctl).arg("add").arg(&dir).output();
    }
    let out = Command::new(&ctl).arg("sync").output().map_err(|e| format!("{}: {e}", ctl.display()))?;
    if out.status.success() {
        let line = last_line(&out.stdout);
        Ok(if line.is_empty() { "synced".into() } else { line })
    } else {
        Err(format!("yabridgectl sync failed: {}", last_line(&out.stderr)))
    }
}

/// Sync is worth running: yabridge is there and Wine has plug-ins.
pub fn sync_wanted() -> bool {
    find_yabridgectl().is_some() && windows_vst3_dir().is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_family_comes_from_id_then_id_like() {
        assert_eq!(Family::parse("NAME=\"Linux Mint\"\nID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"), Family::Debian);
        assert_eq!(Family::parse("ID=ubuntu\nID_LIKE=debian\n"), Family::Debian);
        assert_eq!(Family::parse("ID=debian\n"), Family::Debian);
        assert_eq!(Family::parse("ID=fedora\n"), Family::Fedora);
        assert_eq!(Family::parse("ID=nobara\nID_LIKE=\"fedora\"\n"), Family::Fedora);
        assert_eq!(Family::parse("ID=rocky\nID_LIKE=\"rhel centos fedora\"\n"), Family::Fedora);
        assert_eq!(Family::parse("ID=manjaro\nID_LIKE=arch\n"), Family::Arch);
        assert_eq!(Family::parse("ID=nixos\n"), Family::Other);
        assert_eq!(Family::parse(""), Family::Other);
        assert!(Family::Debian.wine_command().unwrap().contains("apt-get install -y wine"));
        assert!(Family::Fedora.wine_command().unwrap().contains("dnf install -y wine"));
        assert_eq!(Family::Other.wine_command(), None);
    }

    #[test]
    fn wines_version_is_read_from_its_banner() {
        assert_eq!(parse_version("wine-11.0"), Some((11, 0)));
        assert_eq!(parse_version("wine-9.21 (Staging)\n"), Some((9, 21)));
        assert_eq!(parse_version("wine-10.0-rc1"), Some((10, 0)));
        assert_eq!(parse_version("wine-9"), Some((9, 0)));
        assert_eq!(parse_version("not wine"), None);
        assert!(!newer_than_yabridge_supports((9, 21)));
        assert!(newer_than_yabridge_supports((9, 22)));
        assert!(newer_than_yabridge_supports((11, 0)));
    }

    /// With Wine 9.22 or later, yabridge has to be built from source unless it was.
    #[test]
    fn a_build_is_wanted_with_new_wine_unless_already_built() {
        let st = |wine: &str, build: Option<&str>| Status {
            family: Family::Debian,
            wine: Some(wine.into()),
            yabridgectl: None,
            yabridge_build: build.map(str::to_string),
            build: BuildCheck::default(),
            windows_vst3: PathBuf::new(),
            windows_plugins: 0,
            bridged: 0,
            pkexec: true,
        };
        assert!(!st("wine-9.21", None).yabridge_needs_build());
        assert!(st("wine-11.0", None).yabridge_needs_build());
        assert!(!st("wine-11.0", Some("yabridge master b580a9f built 2026-11-01 for wine-11.0 by DECKARD")).yabridge_needs_build());
    }

    #[test]
    fn the_build_check_is_read_from_the_script() {
        let c = BuildCheck::parse(
            "missing: meson ninja-build wine-stable-dev\nroot: export DEBIAN_FRONTEND=noninteractive; apt-get update && apt-get install -y meson ninja-build wine-stable-dev\n",
        );
        assert_eq!(c.missing, ["meson", "ninja-build", "wine-stable-dev"]);
        assert!(c.root_command.as_deref().is_some_and(|r| r.ends_with("install -y meson ninja-build wine-stable-dev")));
        assert!(!c.ready());
        let ready = BuildCheck::parse("ready\n");
        assert!(ready.ready() && ready.root_command.is_none());
        let unknown = BuildCheck::parse("missing: meson\nroot:\n");
        assert_eq!((unknown.missing.len(), unknown.root_command), (1, None));
    }

    #[test]
    fn bundles_are_counted_whatever_their_case_and_shape() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Massive.vst3")).unwrap();
        std::fs::write(dir.path().join("Old.VST3"), b"").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"").unwrap();
        assert_eq!(count_bundles(dir.path()), 2);
        assert_eq!(count_bundles(&dir.path().join("missing")), 0);
    }
}
