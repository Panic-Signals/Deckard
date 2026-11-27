//! Chopping by playing, on the app: slices caught at the playhead, kept through
//! re-rolls, and one slice spread over a new sound's keys ([`crate::chop`] decides where).

use crate::{App, beat, chop};
use deckard_engine::munge::{Munge, munge};
use deckard_engine::sample::Sample;
use deckard_model::new_sampler::{BeatMap, PlayMode, ZoneMeta};

/// A munge ready to make: the sample, the range (from an eighth line on), its eighths,
/// an eighth's even length, the eighth of its bar the range starts on, the settings, the
/// zone's meta, and the beats the range runs from and to.
pub(crate) struct MungePlan {
    sample: std::sync::Arc<Sample>,
    range: (usize, usize),
    cuts: Vec<usize>,
    eighth: f64,
    phase: usize,
    settings: Munge,
    meta: ZoneMeta,
    beats: (f64, f64),
}

impl MungePlan {
    /// The munged sample, and where its munged part ends.
    pub(crate) fn make(&self) -> Result<(Sample, usize), String> {
        let out = self.of(&self.sample).ok_or("nothing would be left to play")?;
        let end = out.frames.len().saturating_sub(self.sample.frames.len() - self.range.1);
        Ok((out, end))
    }

    fn of(&self, s: &Sample) -> Option<Sample> {
        munge(s, self.range, &self.cuts, self.eighth, self.phase, &self.settings)
    }

    /// Where frame `x` of the sample (inclusive ends too) is in the result ending its
    /// munged part at `end`.
    fn moved(&self, x: i64, end: usize, delta: i64) -> i64 {
        if x >= self.range.1 as i64 { x + delta } else { x.min(end as i64 - 1) }.max(0)
    }

    /// The zone's meta for the result: its length in beats, and (a zone with warp
    /// markers) the markers before the range as they were, the range even between two
    /// of its own, those after moved with their audio.
    fn meta_after(&self, new_len: usize, end: usize) -> ZoneMeta {
        use deckard_model::new_sampler::Warp;
        let beat = self.eighth * 2.0;
        let (a, b) = self.range;
        let delta = new_len as i64 - self.sample.frames.len() as i64;
        let cut = self.beats.1 - (self.beats.0 + (end - a) as f64 / beat);
        let mut meta = self.meta.clone();
        if meta.warps.is_empty() {
            meta.beats = Some(new_len as f64 / beat);
        } else {
            let old = std::mem::take(&mut meta.warps);
            meta.warps = old.iter().filter(|w| (w.frame as usize) < a).copied().collect();
            meta.warps.push(Warp { frame: a as u64, beat: self.beats.0 });
            meta.warps.push(Warp { frame: end as u64, beat: self.beats.0 + (end - a) as f64 / beat });
            meta.warps.extend(
                old.iter()
                    .filter(|w| w.frame as usize > b)
                    .map(|w| Warp { frame: (w.frame as i64 + delta).max(0) as u64, beat: w.beat - cut }),
            );
            meta.beats = meta.beats.map(|x| (x - cut).max(0.0));
        }
        meta
    }
}

impl App {
    /// The focused zone's own beats: a New Sampler zone with a tempo.
    fn zone_beats(&self, rate: u32) -> Option<BeatMap> {
        let d = self.doc.as_ref()?.borrow();
        let m = d.project().groups.get(self.group)?.sounds.get(self.sound)?.sampler.as_ref()?.zones.get(self.zone)?.meta.clone()?;
        BeatMap::of(&m, rate)
    }

    /// The lines Find looks along in `r` for `count` starts: the sample's bars (or beats,
    /// of a short one) when the zone has a tempo; none otherwise (it goes by the hits).
    pub(crate) fn find_lines(&self, rate: u32, r: (usize, usize), count: usize) -> Vec<usize> {
        self.zone_beats(rate).map_or(Vec::new(), |map| {
            let by = |div| beat::grid_slices(&map, r, div);
            [1, 2, 4].into_iter().map(by).find(|l| l.len() >= 2 * count).unwrap_or_else(|| by(8))
        })
    }

    /// Every place in `r` Find would weigh for `count` starts: what Random picks among.
    pub(crate) fn slice_candidates(&self, s: &Sample, r: (usize, usize), count: usize) -> Vec<usize> {
        deckard_analysis::find_samples(&s.frames, s.rate, r, &self.find_lines(s.rate, r, count), usize::MAX)
    }

    /// What MUNGE would do now: the edit range cut at the sample's eighth notes (the
    /// first at the range's start).
    pub(crate) fn munge_plan(&self) -> Result<MungePlan, String> {
        let settings = self.sampling.munge.ok_or("MUNGE isn't open")?;
        let sample = self.sampling.sample.clone().ok_or("no sample")?;
        let (a, b) = self.edit_range().ok_or("no sample")?;
        let range = (a.min(sample.frames.len()), b.min(sample.frames.len()));
        let no_tempo = "MUNGE needs the sample's tempo (ZONE: BPM)";
        let meta = {
            let d = self.doc.as_ref().ok_or("no project")?.borrow();
            let z = d
                .project()
                .groups
                .get(self.group)
                .and_then(|g| g.sounds.get(self.sound))
                .and_then(|s| s.sampler.as_ref()?.zones.get(self.zone));
            z.and_then(|z| z.meta.clone()).ok_or(no_tempo)?
        };
        let map = BeatMap::of(&meta, sample.rate).ok_or(no_tempo)?;
        let beat = f64::from(sample.rate) * 60.0 / meta.bpm.ok_or(no_tempo)?;
        // From the range's start if that is an eighth line (within a millisecond), else
        // from the next: what comes before it stays as it is.
        let at = map.beat_at(range.0 as f64) * 2.0;
        let near = (at - at.round()).abs() * beat / 2.0 <= f64::from(sample.rate) / 1000.0;
        let first = if near { at.round() } else { at.ceil() };
        let from = if near { range.0 } else { map.frame_at(first / 2.0).round().max(range.0 as f64) as usize };
        if from >= range.1 {
            return Err("the range holds no eighth note".into());
        }
        let range = (from, range.1);
        let mut cuts = vec![from];
        for k in 1..100_000 {
            let f = map.frame_at((first + f64::from(k)) / 2.0).round();
            if f >= range.1 as f64 {
                break;
            }
            if f > *cuts.last().unwrap() as f64 {
                cuts.push(f as usize);
            }
        }
        let beats = (first / 2.0, map.beat_at(range.1 as f64));
        let phase = (first as i64).rem_euclid(8) as usize;
        Ok(MungePlan { sample, range, cuts, eighth: beat / 2.0, phase, settings, meta, beats })
    }

    /// MUNGE's APPLY: the range munged, as a new file (one undo step). The zone goes on
    /// playing what it played (the munged part, or its share of it), its loop likewise.
    pub(crate) fn munge_apply(&mut self) {
        if !self.sampling.munge.is_some_and(|m| m.changes()) {
            self.status = "MUNGE: nothing to change (set SWING, or switch an eighth off)".into();
            return;
        }
        let made = self.munge_plan().and_then(|p| Ok((p.make()?, p)));
        let ((out, end), p) = match made {
            Ok(x) => x,
            Err(e) => {
                self.status = format!("can't munge: {e}");
                return;
            }
        };
        if out.frames == p.sample.frames {
            self.status = "MUNGE: nothing changes in this range".into();
            return;
        }
        let old_len = p.sample.frames.len() as i64;
        let delta = out.frames.len() as i64 - old_len;
        let zone = self.doc.as_ref().and_then(|d| {
            let d = d.borrow();
            d.project().groups.get(self.group)?.sounds.get(self.sound)?.sampler.as_ref()?.zones.get(self.zone).cloned()
        });
        let Some(zone) = zone else { return };
        let (lo, hi) = if zone.play_range.1 > zone.play_range.0 { zone.play_range } else { (0, old_len - 1) };
        let last = out.frames.len() as i64 - 1;
        let fit = |x: i64| p.moved(x, end, delta).min(last);
        let (lo, hi) = (fit(lo).min(last - 1).max(0), fit(hi));
        let (ll, lh) = (fit(zone.loop_range.0), fit(zone.loop_range.1));
        let params = [
            ("play_range/low", lo),
            ("play_range/high", hi.max(lo + 1).min(last)),
            ("loop_range/low", ll),
            ("loop_range/high", lh.max(ll)),
        ];
        let meta = p.meta_after(out.frames.len(), end);
        let lane = |l: &Sample| p.of(l).unwrap_or_else(|| l.clone());
        self.store_edit(out, "Munge", Some((&lane, (0, false))), Some((meta, params)));
        // (Open still if the file couldn't be written.)
        if self.status.starts_with("Munge:") {
            self.sampling.munge = None;
        }
    }

    /// MUNGE's HEAR: the munged range played, nothing kept.
    pub(crate) fn munge_hear(&mut self) {
        let made = self.munge_plan().and_then(|p| Ok((p.make()?, p.range.0)));
        match made {
            Ok(((out, end), from)) => {
                let part = Sample { rate: out.rate, frames: out.frames[from.min(end)..end].to_vec() };
                self.ensure_engine();
                let gain = self.doc.as_ref().map_or(1.0, |d| {
                    let d = d.borrow();
                    let p = d.project();
                    let g = p.groups.get(self.group);
                    let s = g.and_then(|g| g.sounds.get(self.sound));
                    deckard_model::level_gain(p.master_level) * g.map_or(1.0, |g| g.level) * s.map_or(1.0, |s| s.level)
                });
                if let Some(Ok(out)) = &self.output {
                    out.prehear(Some(std::sync::Arc::new(part)), gain as f32);
                    self.prehear_from = None;
                }
                self.status = "MUNGE: playing it (APPLY keeps it)".into();
            }
            Err(e) => self.status = format!("can't munge: {e}"),
        }
    }

    /// The focused sound's Slice Motion (a New Sampler's; one undo step).
    pub(crate) fn set_motion(&mut self, motion: deckard_model::new_sampler::Motion) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, s) = (self.group, self.sound);
        let now = doc.borrow().project().groups.get(g).and_then(|x| x.sounds.get(s)).and_then(|x| x.new_sampler);
        let Some(now) = now.filter(|n| n.motion != motion) else { return };
        let settings = deckard_model::new_sampler::NewSampler { motion, ..now };
        match doc.borrow_mut().transaction("Slice Motion", |d| d.set_new_sampler(g, s, Some(&settings))) {
            Ok(()) => self.status = format!("Slice Motion: {}", motion.label()),
            Err(e) => self.status = format!("can't set Slice Motion: {e}"),
        }
    }

    /// CUE, or an empty pad: a slice where the sample is playing (Manual mode only, so
    /// no pad drops one by surprise).
    pub(crate) fn catch_slice(&mut self, free: bool) {
        if self.sampling.slice_mode != 3 {
            return;
        }
        let (Some(s), Some(at)) = (self.sampling.sample.clone(), self.sampling.playhead) else { return };
        let r = self.slice_range().unwrap_or((0, s.frames.len()));
        let map = self.zone_beats(s.rate);
        let Some(f) = chop::catch_at(at, map.as_ref(), free, r) else { return };
        let slices = &mut self.sampling.slices;
        if slices.is_empty() {
            slices.push(r.0);
        }
        if !slices.contains(&f) {
            slices.push(f);
            slices.sort_unstable();
        }
    }

    /// Slice `i` kept through re-rolls, or no longer (Find and Random).
    pub(crate) fn keep_slice(&mut self, i: usize) {
        let st = &mut self.sampling;
        let (true, Some(&f)) = (matches!(st.slice_mode, 4 | 5), st.slices.get(i)) else { return };
        match st.kept.iter().position(|k| *k == f) {
            Some(k) => {
                st.kept.remove(k);
            }
            None => st.kept.push(f),
        }
    }

    /// Key Shift: slice `i` onto the group's next empty sound, a copy of this one whose
    /// only zone is the slice over every key from its root at C3 (a Sync zone in Keys
    /// there: its length holds whatever the key). One undo step; the focus stays.
    pub(crate) fn key_shift(&mut self, i: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let len = self.sampling.len();
        let r = self.slice_range().unwrap_or((0, len));
        let slices = if self.sampling.slices.is_empty() { vec![r.0] } else { self.sampling.slices.clone() };
        let Some(&a) = slices.get(i) else { return };
        let b = slices.get(i + 1).copied().unwrap_or(r.1).min(len);
        if b < a + 2 {
            return;
        }
        let (g, snd) = (self.group, self.sound);
        // A slice that is a zone already (the sample was sliced): that zone's settings.
        let zi = self.chops().and_then(|c| c.1.iter().position(|&s| s == a).map(|k| c.2[k])).unwrap_or(self.zone);
        let p = doc.borrow().project().clone();
        let Some(group) = p.groups.get(g) else { return };
        let Some(src) = group.sounds.get(snd) else { return };
        let Some(zone) = src.sampler.as_ref().and_then(|x| x.zones.get(zi)) else { return };
        let Some(target) = group.sounds.iter().position(|x| x.slots.is_empty()) else {
            self.status = "Key Shift needs an empty sound in the group".into();
            return;
        };
        // Its tempo comes along, in Keys where it was in Sync. With none (a Sampler's zone,
        // or one not yet analysed) it plays as a sampler does: said here, or the analysis of
        // a New Sampler zone would put a loop in Sync.
        let meta = match zone.meta.clone() {
            Some(m) => ZoneMeta { mode: if m.mode == PlayMode::Sync { PlayMode::Keys } else { m.mode }, ..m },
            None => ZoneMeta { file: zone.sample.path.clone(), mode: PlayMode::Repitch, ..ZoneMeta::default() },
        };
        let settings = src.new_sampler.clone().unwrap_or_default();
        let spec = deckard_doc::ZoneSpec {
            from: deckard_doc::ZoneFrom::Zone(zi),
            file: None,
            set: vec![
                ("play_range/low", a as f64),
                ("play_range/high", (b - 1) as f64),
                ("key_range/low", 0.0),
                ("key_range/high", 127.0),
                ("root_key", 60.0),
                ("velocity_range/low", 0.0),
                ("velocity_range/high", 127.0),
                ("loop_active", 0.0),
            ],
        };
        let name = format!("{} keys", src.name);
        let label = "Key Shift";
        let done = doc.borrow_mut().transaction(label, |d| {
            let clip = d.copy_sound(g, snd)?.without_notes();
            d.paste_sounds(g, target, &[clip])?;
            d.set_zones(g, target, &[spec], label)?;
            d.set_new_sampler(g, target, Some(&settings))?;
            // (In Original it knows of its zone what a Sampler does.)
            if settings.mode != deckard_model::new_sampler::SamplerMode::Original {
                d.set_zone_meta(g, target, 0, &meta, label)?;
            }
            d.rename(deckard_doc::Target::Sound(g, target), &name)
        });
        self.status = match done {
            Ok(()) => format!("Key Shift: slice {} is on pad {} (play it in keyboard mode)", i + 1, target + 1),
            Err(e) => format!("can't key shift the slice: {e}"),
        };
    }
}
