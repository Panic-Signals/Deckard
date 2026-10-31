//! Newer releases (docs/superpowers/specs/2026-10-31-about-and-updates-design.md):
//! once per launch, when Preferences ▸ General allows and this build is a release,
//! GitHub's latest release is compared with [`crate::VERSION`]. A newer one shows as
//! a dot on the header's wordmark and a line in About, where **Update** installs it.

use crate::github::{self, Release};
use eframe::egui;
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
        Notice { check: Check::Checking(rx) }
    }

    /// Take up the check's answer (call every frame); true when it just arrived.
    pub fn poll(&mut self) -> bool {
        let Check::Checking(rx) = &self.check else { return false };
        let Ok(r) = rx.try_recv() else { return false };
        self.check = match r {
            Ok(r) if Some(r.version) > github::parse_version(crate::VERSION) => Check::Available(r),
            Ok(_) => Check::Latest,
            Err(e) => Check::Failed(e),
        };
        true
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
            ui.hyperlink_to("Release notes", r.url.clone());
        }
    });
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
}
