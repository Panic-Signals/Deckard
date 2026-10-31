//! Newer releases (docs/superpowers/specs/2026-10-31-about-and-updates-design.md):
//! once per launch, when Preferences ▸ General allows and this build is a release,
//! GitHub's latest release is compared with [`crate::VERSION`]. A newer one shows as
//! a dot on the header's wordmark and a line in About, where **Update** installs it.

use crate::github::{self, Release};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

/// Where the check stands.
#[derive(Default)]
pub enum Check {
    /// Not checking: a build that isn't a release, or the preference is off.
    #[default]
    Off,
    Checking(Receiver<Result<Release, String>>),
    Latest,
    Available(Release),
    Failed(String),
}

/// The update state the app keeps.
#[derive(Default)]
pub struct Notice {
    pub check: Check,
    /// The install under way, once **Update** was pressed.
    pub job: Option<Job>,
}

impl Notice {
    /// Start the check, when `enabled` and this is a release build.
    pub fn start(enabled: bool) -> Notice {
        if !enabled || !crate::RELEASE_BUILD || github::parse_version(crate::VERSION).is_none() {
            return Notice::default();
        }
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let _ = tx.send(github::latest_release());
            })
            .ok();
        Notice { check: Check::Checking(rx), job: None }
    }

    /// Take up the check's answer and the job's progress (call every frame); true
    /// when something changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(job) = &mut self.job {
            while let Ok(p) = job.rx.try_recv() {
                job.progress = p;
                changed = true;
            }
        }
        if let Check::Checking(rx) = &self.check
            && let Ok(r) = rx.try_recv()
        {
            self.check = match r {
                Ok(r) if Some(r.version) > github::parse_version(crate::VERSION) => Check::Available(r),
                Ok(_) => Check::Latest,
                Err(e) => Check::Failed(e),
            };
            changed = true;
        }
        changed
    }

    /// The new release is in place and waits for this process to end.
    pub fn restarting(&self) -> bool {
        self.job.as_ref().is_some_and(|j| matches!(j.progress, Progress::Restarting))
    }

    /// Press **Update**: download and install the release on offer.
    pub fn start_update(&mut self) {
        let Some(release) = self.available().cloned() else { return };
        if self.job.as_ref().is_some_and(|j| !matches!(j.progress, Progress::Failed(_))) {
            return;
        }
        self.job = Some(Job::start(release, Install::detect()));
    }

    /// The newer release, when there is one.
    pub fn available(&self) -> Option<&Release> {
        match &self.check {
            Check::Available(r) => Some(r),
            _ => None,
        }
    }
}

/// The About card's update line.
pub fn line(notice: &mut Notice, ui: &mut egui::Ui) {
    let mut start = false;
    ui.horizontal(|ui| match &notice.check {
        Check::Off if !crate::RELEASE_BUILD => {
            ui.label(egui::RichText::new("A build from source: updates come from git.").weak());
        }
        Check::Off => {
            ui.label(egui::RichText::new("Updates are off (Preferences ▸ General).").weak());
        }
        Check::Checking(_) => {
            ui.label(egui::RichText::new("Checking for updates…").weak());
        }
        Check::Latest => {
            ui.label(egui::RichText::new("You have the latest release.").weak());
        }
        Check::Failed(e) => {
            ui.label(egui::RichText::new(format!("Couldn't check for updates: {e}")).weak());
        }
        Check::Available(r) => {
            ui.label(format!("Update available: {}", r.version_text()));
            match notice.job.as_ref().map(|j| &j.progress) {
                None | Some(Progress::Failed(_)) => {
                    if ui.button("Update").on_hover_text("Download and install it, then restart DECKARD").clicked() {
                        start = true;
                    }
                }
                Some(Progress::Downloading { done, total }) => {
                    let mb = |b: u64| format!("{:.0} MB", b as f64 / 1e6);
                    ui.label(egui::RichText::new(format!("downloading {} of {}", mb(*done), mb(*total))).weak());
                }
                Some(Progress::Installing) => {
                    ui.label(egui::RichText::new("installing…").weak());
                }
                Some(Progress::Restarting) => {
                    ui.label(egui::RichText::new("restarting…").weak());
                }
            }
            ui.hyperlink_to("Release notes", r.url.clone());
        }
    });
    if let Some(Progress::Failed(e)) = notice.job.as_ref().map(|j| &j.progress) {
        ui.label(egui::RichText::new(format!("The update failed: {e}")).color(egui::Color32::from_rgb(0xff, 0xa0, 0x80)));
    }
    if start {
        notice.start_update();
    }
}

/// How the update goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    Downloading {
        done: u64,
        total: u64,
    },
    Installing,
    /// Installed; a helper waits for this process to end and starts the new one.
    Restarting,
    Failed(String),
}

/// An update under way, on its own thread.
pub struct Job {
    rx: Receiver<Progress>,
    pub progress: Progress,
}

impl Job {
    fn start(release: Release, install: Install) -> Job {
        let (tx, rx) = channel();
        let total = pick_asset(&release, &install).map_or(0, |a| a.size);
        std::thread::Builder::new()
            .name("update".into())
            .spawn(move || {
                let progress = tx.clone();
                let result = run(&release, &install, &move |p| {
                    let _ = progress.send(p);
                });
                let _ = tx.send(match result {
                    Ok(()) => Progress::Restarting,
                    Err(e) => Progress::Failed(e),
                });
            })
            .ok();
        Job { rx, progress: Progress::Downloading { done: 0, total } }
    }
}

/// How this DECKARD was installed, which decides the asset and the way to put the new
/// one in place.
#[derive(Debug, Clone, PartialEq)]
pub enum Install {
    /// From the .deb: `/usr/bin/deckard`, replaced by apt.
    Deb {
        arch: &'static str,
    },
    /// From the .rpm, replaced by dnf.
    Rpm {
        arch: &'static str,
    },
    /// The tarball unpacked into `dir`, swapped for the new folder.
    Tarball {
        dir: PathBuf,
        arch: &'static str,
    },
    /// `DECKARD.app`, swapped for the new bundle.
    MacApp {
        app: PathBuf,
    },
    /// `DECKARD.exe` in `dir`; a script moves the new files in once this exits.
    Windows {
        dir: PathBuf,
    },
    Unknown,
}

/// The architecture as the asset names spell it (`x86_64`/`aarch64`; Debian's
/// `amd64`/`arm64`).
fn arch_names() -> (&'static str, &'static str) {
    if cfg!(target_arch = "aarch64") { ("aarch64", "arm64") } else { ("x86_64", "amd64") }
}

impl Install {
    pub fn detect() -> Install {
        let Ok(exe) = std::env::current_exe() else { return Install::Unknown };
        let os = if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(windows) {
            "windows"
        } else {
            "linux"
        };
        Install::detect_from(&exe, os, Path::new("/var/lib/dpkg/status").exists(), Path::new("/var/lib/rpm").exists())
    }

    /// From the program's path and system (separately, so it can be tested).
    pub fn detect_from(exe: &Path, os: &str, dpkg: bool, rpm: bool) -> Install {
        let (arch, deb_arch) = arch_names();
        match os {
            "macos" => exe
                .ancestors()
                .find(|p| p.extension().is_some_and(|e| e == "app"))
                .map_or(Install::Unknown, |app| Install::MacApp { app: app.to_path_buf() }),
            "windows" => exe.parent().map_or(Install::Unknown, |d| Install::Windows { dir: d.to_path_buf() }),
            _ => {
                if exe.starts_with("/usr") {
                    if dpkg {
                        Install::Deb { arch: deb_arch }
                    } else if rpm {
                        Install::Rpm { arch }
                    } else {
                        Install::Unknown
                    }
                } else {
                    exe.parent().map_or(Install::Unknown, |d| Install::Tarball { dir: d.to_path_buf(), arch })
                }
            }
        }
    }

    /// The asset for this install: what its name starts and ends with.
    fn asset_pattern(&self) -> Option<(&'static str, String)> {
        Some(match self {
            Install::Deb { arch } => ("deckard_", format!("_{arch}.deb")),
            Install::Rpm { arch } => ("deckard-", format!(".{arch}.rpm")),
            Install::Tarball { arch, .. } => ("deckard-", format!("-linux-{arch}.tar.gz")),
            Install::MacApp { .. } => ("DECKARD-", "-macos-universal.zip".to_string()),
            Install::Windows { .. } => ("DECKARD-", "-windows-x64.zip".to_string()),
            Install::Unknown => return None,
        })
    }
}

/// The release's asset for `install`, by its name (not by version text, which the
/// 0.1.0 release spelt three ways).
pub fn pick_asset<'a>(release: &'a Release, install: &Install) -> Option<&'a github::Asset> {
    let (start, end) = install.asset_pattern()?;
    release.assets.iter().find(|a| a.name.starts_with(start) && a.name.ends_with(&end))
}

/// Where the download goes: beside the install (so the swap is a rename on one file
/// system), or the cache for a package the package manager installs.
fn work_dir(install: &Install) -> Result<PathBuf, String> {
    let base = match install {
        Install::Tarball { dir, .. } => dir.parent().map(Path::to_path_buf),
        Install::MacApp { app } => app.parent().map(Path::to_path_buf),
        Install::Windows { dir } => Some(dir.clone()),
        Install::Deb { .. } | Install::Rpm { .. } => std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))),
        Install::Unknown => None,
    }
    .ok_or("this DECKARD wasn't installed from a release; download the new one from GitHub")?;
    let dir = base.join(".deckard-update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't write beside DECKARD ({}): {e}", base.display()))?;
    Ok(dir)
}

fn sh(program: &str, args: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("");
        Err(match out.status.code() {
            Some(126) => "cancelled".to_string(),
            _ if last.is_empty() => format!("{program} failed ({})", out.status),
            _ => format!("{program}: {last}"),
        })
    }
}

/// Download, verify, install; then leave a helper that restarts DECKARD once this
/// process ends.
fn run(release: &Release, install: &Install, progress: &dyn Fn(Progress)) -> Result<(), String> {
    let asset = pick_asset(release, install).ok_or_else(|| format!("no build for this system in {}", release.version_text()))?;
    let dir = work_dir(install)?;
    let file = dir.join(&asset.name);
    let total = asset.size;
    let digest = github::download(&asset.url, &file, |done| progress(Progress::Downloading { done, total }))?;
    if let Some(expected) = &asset.digest
        && expected != &digest
    {
        return Err(format!("the download of {} is damaged (checksum mismatch); try again", asset.name));
    }
    progress(Progress::Installing);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    match install {
        Install::Deb { .. } => {
            sh("pkexec", &["sh", "-c", &format!("apt-get install -y '{}'", file.display())])?;
            relaunch_after_exit(&exe, None)
        }
        Install::Rpm { .. } => {
            sh("pkexec", &["sh", "-c", &format!("dnf install -y '{}'", file.display())])?;
            relaunch_after_exit(&exe, None)
        }
        Install::Tarball { dir: install_dir, .. } => {
            sh("tar", &["-xzf", &file.display().to_string(), "-C", &dir.display().to_string()])?;
            let unpacked = unpacked_folder(&dir, |p| p.join("deckard").is_file())?;
            let old = swap(install_dir, &unpacked)?;
            relaunch_after_exit(&exe, Some(&old))
        }
        Install::MacApp { app } => {
            sh("ditto", &["-x", "-k", &file.display().to_string(), &dir.display().to_string()])?;
            let unpacked = unpacked_folder(&dir, |p| p.extension().is_some_and(|e| e == "app"))?;
            let old = swap(app, &unpacked)?;
            relaunch_after_exit(&exe, Some(&old))
        }
        Install::Windows { dir: install_dir } => {
            let new = dir.join("new");
            std::fs::create_dir_all(&new).map_err(|e| e.to_string())?;
            sh("tar", &["-xf", &file.display().to_string(), "-C", &new.display().to_string()])?;
            if !new.join("DECKARD.exe").is_file() {
                return Err("the download holds no DECKARD.exe".into());
            }
            windows_helper(install_dir, &new, &dir)
        }
        Install::Unknown => Err("this DECKARD wasn't installed from a release".into()),
    }
}

/// The one folder `tar`/`ditto` left in `dir` that `looks_right`.
fn unpacked_folder(dir: &Path, looks_right: impl Fn(&Path) -> bool) -> Result<PathBuf, String> {
    std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir() && looks_right(p))
        .ok_or_else(|| "the download didn't unpack as expected".to_string())
}

/// Put `new` where `current` is; `current` is moved aside and its new place given
/// back (deleted by the helper once the new one runs).
fn swap(current: &Path, new: &Path) -> Result<PathBuf, String> {
    let old = current.with_extension("old");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(current, &old)
        .map_err(|e| format!("can't replace {} (is it in a folder you can write to?): {e}", current.display()))?;
    if let Err(e) = std::fs::rename(new, current) {
        let _ = std::fs::rename(&old, current);
        return Err(format!("can't move the new DECKARD into place: {e}"));
    }
    Ok(old)
}

/// Linux and macOS: a detached shell waits for this process to end, deletes `old`,
/// and starts the (new) program at `exe`'s path.
#[cfg(not(windows))]
fn relaunch_after_exit(exe: &Path, old: Option<&Path>) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let pid = std::process::id();
    let exe = exe.display().to_string();
    let rm = old.map_or(String::new(), |o| format!("rm -rf '{}'; ", o.display()));
    let launch = if cfg!(target_os = "macos") && exe.contains(".app/") {
        let app = exe.split(".app/").next().unwrap_or(&exe).to_string() + ".app";
        format!("open -n '{app}'")
    } else {
        format!("'{exe}' &")
    };
    let script = format!("while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; {rm}{launch}");
    std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("can't start the restart helper: {e}"))?;
    Ok(())
}

#[cfg(windows)]
fn relaunch_after_exit(_exe: &Path, _old: Option<&Path>) -> Result<(), String> {
    Err("not how Windows updates".into())
}

/// Windows: `update.cmd` waits for this process to end, moves the new files over the
/// old, starts DECKARD and cleans up.
fn windows_helper(install_dir: &Path, new: &Path, work: &Path) -> Result<(), String> {
    let pid = std::process::id();
    let script = format!(
        "@echo off\r\n:wait\r\ntasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul\r\nif not errorlevel 1 (timeout /t 1 /nobreak >nul & goto wait)\r\n\
         robocopy \"{new}\" \"{dir}\" /E /MOVE /NFL /NDL /NJH /NJS >nul\r\nstart \"\" \"{dir}\\DECKARD.exe\"\r\n\
         cd /d \"{dir}\"\r\nrmdir /s /q \"{work}\"\r\n",
        new = new.display(),
        dir = install_dir.display(),
        work = work.display()
    );
    let path = work.join("update.cmd");
    std::fs::write(&path, script).map_err(|e| e.to_string())?;
    if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", "start", "/min", "", &path.display().to_string()])
            .spawn()
            .map_err(|e| format!("can't start the update helper: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a newer release is an update; the same or an older one is "latest", and
    /// a build from source (the crate version carries no release) never checks.
    #[test]
    fn only_a_newer_release_is_an_update() {
        let mut n = Notice::default();
        let (tx, rx) = channel();
        n.check = Check::Checking(rx);
        let newer = Release { version: (999, 0, 0), tag: "999.0.0".into(), url: String::new(), assets: Vec::new() };
        tx.send(Ok(newer.clone())).unwrap();
        assert!(n.poll());
        assert_eq!(n.available().map(|r| r.version), Some((999, 0, 0)));

        let (tx, rx) = channel();
        n.check = Check::Checking(rx);
        tx.send(Ok(Release { version: (0, 0, 0), ..newer.clone() })).unwrap();
        n.poll();
        assert!(matches!(n.check, Check::Latest));
        assert!(n.available().is_none());

        let (tx, rx) = channel();
        n.check = Check::Checking(rx);
        tx.send(Err("offline".into())).unwrap();
        n.poll();
        assert!(matches!(&n.check, Check::Failed(e) if e == "offline"));

        // Nothing arrived yet: nothing changes.
        let (_tx, rx) = channel();
        n.check = Check::Checking(rx);
        assert!(!n.poll());

        // The test binary isn't a release build: no check starts, asked or not.
        assert!(matches!(Notice::start(false).check, Check::Off));
        assert!(matches!(Notice::start(true).check, Check::Off));
    }

    fn release_with(names: &[&str]) -> Release {
        Release {
            version: (0, 2, 0),
            tag: "0.2.0".into(),
            url: String::new(),
            assets: names
                .iter()
                .map(|n| github::Asset { name: n.to_string(), url: format!("https://x/{n}"), digest: None, size: 1 })
                .collect(),
        }
    }

    /// The install is told from the program's place, and each kind takes its own
    /// asset, by name pattern: the 0.1.0 release's sloppy version strings included.
    #[test]
    fn each_install_takes_its_own_asset() {
        let (arch, deb_arch) = arch_names();
        let deb = Install::detect_from(Path::new("/usr/bin/deckard"), "linux", true, false);
        let rpm = Install::detect_from(Path::new("/usr/bin/deckard"), "linux", false, true);
        let tar = Install::detect_from(Path::new("/home/me/deckard-0.1.0-linux-x86_64/deckard"), "linux", true, true);
        let mac = Install::detect_from(Path::new("/Applications/DECKARD.app/Contents/MacOS/deckard"), "macos", false, false);
        let win = Install::detect_from(Path::new("C:\\Users\\me\\DECKARD\\DECKARD.exe"), "windows", false, false);
        assert_eq!(deb, Install::Deb { arch: deb_arch });
        assert_eq!(rpm, Install::Rpm { arch });
        assert_eq!(tar, Install::Tarball { dir: PathBuf::from("/home/me/deckard-0.1.0-linux-x86_64"), arch });
        assert_eq!(mac, Install::MacApp { app: PathBuf::from("/Applications/DECKARD.app") });
        assert!(matches!(win, Install::Windows { .. }));
        assert_eq!(Install::detect_from(Path::new("/usr/local/bin/deckard"), "linux", false, false), Install::Unknown);
        assert_eq!(Install::detect_from(Path::new("/tmp/deckard"), "macos", false, false), Install::Unknown);

        let r = release_with(&[
            "deckard-0.1.0-Early-Alpha-Release-linux-x86_64.tar.gz",
            "DECKARD-0.1.0-Early-Alpha-Release-macos-universal.zip",
            "deckard-0.1.0.Early_Alpha_Release-1.x86_64.rpm",
            "deckard_0.1.0.Early-Alpha-Release-1_amd64.deb",
            "DECKARD-0.2.0-windows-x64.zip",
            "deckard-0.2.0-linux-aarch64.tar.gz",
        ]);
        let name = |i: &Install| pick_asset(&r, i).map(|a| a.name.as_str());
        if arch == "x86_64" {
            assert_eq!(name(&deb), Some("deckard_0.1.0.Early-Alpha-Release-1_amd64.deb"));
            assert_eq!(name(&rpm), Some("deckard-0.1.0.Early_Alpha_Release-1.x86_64.rpm"));
            assert_eq!(name(&tar), Some("deckard-0.1.0-Early-Alpha-Release-linux-x86_64.tar.gz"));
        } else {
            assert_eq!(name(&tar), Some("deckard-0.2.0-linux-aarch64.tar.gz"));
        }
        assert_eq!(name(&mac), Some("DECKARD-0.1.0-Early-Alpha-Release-macos-universal.zip"));
        assert_eq!(name(&win), Some("DECKARD-0.2.0-windows-x64.zip"));
        assert_eq!(name(&Install::Unknown), None);
        assert_eq!(pick_asset(&release_with(&["notes.txt"]), &mac), None);
    }

    /// The swap keeps the old install aside and puts it back when the new one can't
    /// be moved in.
    #[test]
    fn swapping_keeps_the_old_install_until_the_new_one_is_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("DECKARD.app");
        let new = tmp.path().join("work/DECKARD.app");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("v"), "1").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("v"), "2").unwrap();
        let old = swap(&current, &new).unwrap();
        assert_eq!(std::fs::read_to_string(current.join("v")).unwrap(), "2");
        assert_eq!(std::fs::read_to_string(old.join("v")).unwrap(), "1");
        assert_eq!(old, tmp.path().join("DECKARD.old"));
        // Nothing to move in: the old one is back.
        assert!(swap(&current, &tmp.path().join("missing")).is_err());
        assert_eq!(std::fs::read_to_string(current.join("v")).unwrap(), "2");
    }

    /// The Windows helper script waits for this process, moves the files, starts the
    /// new program and cleans up; it is only run on Windows.
    #[test]
    fn the_windows_helper_is_written_for_this_process() {
        let tmp = tempfile::tempdir().unwrap();
        let (install, new, work) = (tmp.path().join("DECKARD"), tmp.path().join("work/new"), tmp.path().join("work"));
        std::fs::create_dir_all(&new).unwrap();
        if cfg!(windows) {
            return;
        }
        windows_helper(&install, &new, &work).unwrap();
        let script = std::fs::read_to_string(work.join("update.cmd")).unwrap();
        assert!(script.contains(&format!("PID eq {}", std::process::id())));
        assert!(script.contains("robocopy"));
        assert!(script.contains("DECKARD.exe"));
        assert!(script.contains(&format!("rmdir /s /q \"{}\"", work.display())));
    }
}
