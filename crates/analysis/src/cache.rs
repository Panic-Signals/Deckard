//! What analysis found, kept on disk by the file's content, so a sample is analysed once
//! however often it's loaded, moved or renamed.

use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::{Root, Tempo, TempoSource};

/// What a sample is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Analysis {
    pub tempo: Option<Tempo>,
    pub root: Option<Root>,
    /// Its length.
    pub seconds: f64,
}

/// Analyse `frames`: the tempo (a tempo in `name` wins) and the root (a note in `name`
/// wins).
pub fn analyse(frames: &[[f32; 2]], rate: u32, name: &str) -> Analysis {
    Analysis {
        tempo: crate::tempo(frames, rate, deckard_library::names::tempo_in_name(name)),
        root: crate::root_with_name(frames, rate, name),
        seconds: if rate == 0 { 0.0 } else { frames.len() as f64 / f64::from(rate) },
    }
}

/// A folder of analyses, one small text file per content hash.
pub struct Cache {
    dir: PathBuf,
}

const SOURCES: [(TempoSource, &str); 3] = [(TempoSource::Hint, "hint"), (TempoSource::Loop, "loop"), (TempoSource::Audio, "audio")];

impl Cache {
    pub fn new(dir: impl Into<PathBuf>) -> Cache {
        Cache { dir: dir.into() }
    }

    /// The key of a file's bytes: their SHA-256, in hex.
    pub fn key(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.txt"))
    }

    /// The analysis kept for `key`, if one is and it reads.
    pub fn get(&self, key: &str) -> Option<Analysis> {
        let text = std::fs::read_to_string(self.path(key)).ok()?;
        let mut lines = text.lines();
        if lines.next()? != "v 1" {
            return None;
        }
        let mut a = Analysis { tempo: None, root: None, seconds: 0.0 };
        for line in lines {
            let mut w = line.split(' ');
            let mut num = || w.next().and_then(|x| x.parse::<f64>().ok());
            match line.split(' ').next()? {
                "seconds" => {
                    num();
                    a.seconds = num()?;
                }
                "tempo" => {
                    num();
                    let (bpm, beats, offset, confidence) = (num()?, num()?, num()?, num()?);
                    let source = SOURCES.iter().find(|s| Some(s.1) == line.split(' ').nth(5))?.0;
                    a.tempo = Some(Tempo {
                        bpm,
                        beats,
                        offset: offset as usize,
                        confidence: confidence as f32,
                        source,
                        candidates: [bpm * 2.0, bpm / 2.0],
                    });
                }
                "root" => {
                    num();
                    let (note, cents, confidence) = (num()?, num()?, num()?);
                    a.root = Some(Root { note: note as u8, cents: cents as f32, confidence: confidence as f32 });
                }
                _ => {}
            }
        }
        Some(a)
    }

    /// Keep `a` for `key` (written whole, then renamed into place).
    pub fn put(&self, key: &str, a: &Analysis) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let mut text = format!("v 1\nseconds {}\n", a.seconds);
        if let Some(t) = &a.tempo {
            let source = SOURCES.iter().find(|s| s.0 == t.source).map_or("audio", |s| s.1);
            text += &format!("tempo {} {} {} {} {source}\n", t.bpm, t.beats, t.offset, t.confidence);
        }
        if let Some(r) = &a.root {
            text += &format!("root {} {} {}\n", r.note, r.cents, r.confidence);
        }
        let tmp = self.dir.join(format!("{key}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, self.path(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_signals::{drums, tone};

    #[test]
    fn analysis_round_trips_through_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("analysis"));
        let mut f = drums(120.0, 4, 44_100, 0.5);
        f.extend(tone(110.0, 2.0, 44_100, 1));
        let a = Analysis { root: crate::root(&tone(110.0, 2.0, 44_100, 1), 44_100), ..analyse(&f, 44_100, "loop.wav") };
        assert!(a.tempo.is_some() && a.root.is_some());
        let key = Cache::key(b"some bytes");
        cache.put(&key, &a).unwrap();
        assert_eq!(cache.get(&key), Some(a));
        let none = Analysis { tempo: None, root: None, seconds: 0.5 };
        cache.put("other", &none).unwrap();
        assert_eq!(cache.get("other"), Some(none));
    }

    #[test]
    fn a_missing_or_bad_entry_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        assert_eq!(cache.get("nothing"), None);
        std::fs::write(dir.path().join("bad.txt"), "v 9\nseconds 1").unwrap();
        assert_eq!(cache.get("bad"), None);
        std::fs::write(dir.path().join("worse.txt"), "v 1\ntempo x").unwrap();
        assert_eq!(cache.get("worse"), None);
    }

    #[test]
    fn the_key_is_the_content() {
        assert_eq!(Cache::key(b"abc"), Cache::key(b"abc"));
        assert_ne!(Cache::key(b"abc"), Cache::key(b"abd"));
        assert_eq!(Cache::key(b"abc").len(), 64);
    }

    #[test]
    fn the_name_gives_the_hint() {
        let a = analyse(&drums(120.0, 4, 44_100, 0.5), 44_100, "x 100bpm.wav");
        let t = a.tempo.unwrap();
        assert_eq!((t.bpm, t.source), (100.0, TempoSource::Hint));
        assert!((a.seconds - 8.0).abs() < 0.001);
    }
}
