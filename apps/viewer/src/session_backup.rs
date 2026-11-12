//! The crash-recovery backup's worker ([`App::keep_session`]): encoding the project and
//! writing it happen here, off the UI thread, which only takes a [`Snapshot`].
//!
//! One thread takes the jobs in the order they were given, so an older backup never
//! lands over a newer one. A job still waiting when a newer comes is dropped (the
//! newer one says it all), so a slow encode never builds a queue.

use deckard_doc::Snapshot;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

pub(crate) enum Job {
    /// Keep the project: its file `session.mxprj`, and `info` as `session.txt`.
    Keep { dir: PathBuf, snapshot: Snapshot, info: String },
    /// Keep only `info` (`session.txt`): the project isn't backed up.
    Info { dir: PathBuf, info: String },
    /// Mark the backup there as ended, so the next start doesn't offer it.
    Retire { dir: PathBuf },
}

#[derive(Default)]
struct State {
    waiting: Option<Job>,
    busy: bool,
    quit: bool,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

#[derive(Default)]
pub(crate) struct Keeper {
    shared: Shared,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Keeper {
    /// Have `job` done, replacing the one waiting (if its turn hasn't come).
    pub(crate) fn submit(&mut self, job: Job) {
        if self.thread.is_none() {
            let shared = self.shared.clone();
            self.thread = std::thread::Builder::new().name("session-backup".into()).spawn(move || work(&shared)).ok();
        }
        let (lock, cv) = &*self.shared;
        lock.lock().unwrap().waiting = Some(job);
        cv.notify_all();
    }

    /// Wait until everything given is written.
    pub(crate) fn finish(&self) {
        let (lock, cv) = &*self.shared;
        let mut s = lock.lock().unwrap();
        while (s.waiting.is_some() || s.busy) && self.thread.is_some() {
            s = cv.wait(s).unwrap();
        }
    }
}

impl Drop for Keeper {
    fn drop(&mut self) {
        let (lock, cv) = &*self.shared;
        lock.lock().unwrap().quit = true;
        cv.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn work(shared: &Shared) {
    let types = mxprj::types::Types::new();
    let (lock, cv) = &**shared;
    loop {
        let job = {
            let mut s = lock.lock().unwrap();
            loop {
                if let Some(j) = s.waiting.take() {
                    s.busy = true;
                    break j;
                }
                if s.quit {
                    return;
                }
                s = cv.wait(s).unwrap();
            }
        };
        run(job, &types);
        lock.lock().unwrap().busy = false;
        cv.notify_all();
    }
}

fn run(job: Job, types: &mxprj::types::Types) {
    match job {
        Job::Keep { dir, snapshot, info } => {
            // A project that won't encode leaves the last backup as it is.
            let Ok(bytes) = snapshot.encode(types) else { return };
            let _ = std::fs::create_dir_all(&dir);
            write(&dir, "session.mxprj", bytes);
            write(&dir, "session.txt", info.into_bytes());
        }
        Job::Info { dir, info } => {
            let _ = std::fs::create_dir_all(&dir);
            write(&dir, "session.txt", info.into_bytes());
        }
        Job::Retire { dir } => {
            let Ok(info) = std::fs::read_to_string(dir.join("session.txt")) else { return };
            if let Some(ended) = retired(&info) {
                write(&dir, "session.txt", ended.into_bytes());
            }
        }
    }
}

/// `info` with the session marked ended, or `None` if it already is.
pub(crate) fn retired(info: &str) -> Option<String> {
    if info.lines().any(|l| l == "clean = true") {
        return None;
    }
    let mut out: String = info.lines().filter(|l| !l.starts_with("clean = ")).map(|l| format!("{l}\n")).collect();
    out += "clean = true\n";
    Some(out)
}

/// Through a temporary file, so a crash mid-way leaves the last.
fn write(dir: &Path, name: &str, data: Vec<u8>) {
    let tmp = dir.join(format!("{name}.saving"));
    if std::fs::write(&tmp, data).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(tempo: f64, info: &str, dir: &Path) -> Job {
        let mut d = deckard_doc::Document::new_project().unwrap();
        d.set_tempo(tempo).unwrap();
        Job::Keep { dir: dir.to_path_buf(), snapshot: d.snapshot(), info: info.into() }
    }

    /// Jobs land in the order given: the last one given is what's there at the end,
    /// however many came before (some may be dropped, none lands late).
    #[test]
    fn an_older_backup_never_lands_over_a_newer_one() {
        let tmp = tempfile::tempdir().unwrap();
        let mut keeper = Keeper::default();
        for i in 0..20 {
            keeper.submit(kept(80.0 + i as f64, &format!("n = {i}\n"), tmp.path()));
        }
        keeper.finish();
        assert_eq!(std::fs::read_to_string(tmp.path().join("session.txt")).unwrap(), "n = 19\n");
        let d = deckard_doc::Document::from_bytes(std::fs::read(tmp.path().join("session.mxprj")).unwrap(), None).unwrap();
        assert_eq!(d.project().tempo, 99.0);
        assert!(!tmp.path().join("session.mxprj.saving").exists(), "written through a temporary file, then renamed");
    }

    /// A job still waiting when the keeper is dropped is written first.
    #[test]
    fn dropping_the_keeper_finishes_the_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let mut keeper = Keeper::default();
        keeper.submit(kept(91.0, "n = 1\n", tmp.path()));
        drop(keeper);
        assert!(tmp.path().join("session.mxprj").exists());
    }

    #[test]
    fn retiring_marks_the_backup_ended_and_keeps_the_rest() {
        let info = "dirty = true\nclean = false\npath = C:/x.mxprj\n";
        assert_eq!(retired(info).unwrap(), "dirty = true\npath = C:/x.mxprj\nclean = true\n");
        assert_eq!(retired("dirty = true\nclean = true\n"), None);
        assert_eq!(retired("dirty = true\n").unwrap(), "dirty = true\nclean = true\n");
    }
}
