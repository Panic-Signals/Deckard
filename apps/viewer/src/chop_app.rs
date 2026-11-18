//! Chopping by playing, on the app: slices caught at the playhead, kept through
//! re-rolls, and one slice spread over a new sound's keys ([`crate::chop`] decides where).

use crate::{App, beat, chop};
use deckard_engine::sample::Sample;
use deckard_model::new_sampler::{BeatMap, PlayMode, ZoneMeta};

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
        let meta = zone.meta.clone().map(|m| ZoneMeta { mode: if m.mode == PlayMode::Sync { PlayMode::Keys } else { m.mode }, ..m });
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
            if let Some(m) = &meta {
                d.set_zone_meta(g, target, 0, m, label)?;
            }
            d.rename(deckard_doc::Target::Sound(g, target), &name)
        });
        self.status = match done {
            Ok(()) => format!("Key Shift: slice {} is on pad {} (play it in keyboard mode)", i + 1, target + 1),
            Err(e) => format!("can't key shift the slice: {e}"),
        };
    }
}
