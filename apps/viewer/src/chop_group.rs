//! Slices onto pads and into files (sampler roadmap, phase 3 part 2): APPLY to the pads
//! of new groups, a sound a slice, and each slice as a WAV file.

use crate::{App, chop};
use deckard_engine::sample::Sample;
use deckard_model::Note;
use deckard_model::new_sampler::{PlayMode, ZoneMeta};
use std::path::Path;

/// Pads in a group.
const PADS: usize = 16;

impl App {
    /// APPLY to pads: `slices` (of `s`, the last ending at `end`) each on a pad of new
    /// groups at the end of the group list, sixteen a group. A pad is a copy of the sliced
    /// sound without its notes, the slice its only zone, at its root on every key. The
    /// group sliced, the slicer and the focus stay as they are. One undo step.
    pub(crate) fn apply_slices_to_pads(&mut self, s: &Sample, slices: &[(usize, usize)], end: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, snd) = (self.group, self.sound);
        let p = doc.borrow().project().clone();
        let Some((src, sampler)) = p.groups.get(g).and_then(|x| x.sounds.get(snd)).and_then(|x| Some((x, x.sampler.as_ref()?))) else {
            self.status = "can't apply the slices: no sampler here".into();
            return;
        };
        // A slice that is a zone already (the sample was sliced): that zone's settings.
        let was = self.chops().map(|c| (c.1, c.2)).unwrap_or_default();
        let focused = self.zone;
        let zone_of = |a: usize| was.0.iter().position(|&x| x == a).map_or(focused, |k| was.1[k]);
        let original = src.new_sampler.is_some_and(|n| n.mode == deckard_model::new_sampler::SamplerMode::Original);
        let to_end = self.sampling.slices_to_end(src.new_sampler.is_some() && !original);
        let one_shot = sampler.envelope_type == 0;
        let settings = src.new_sampler.clone();
        let name = if src.name.trim().is_empty() { "Slice".to_string() } else { src.name.clone() };
        let groups = slices.len().div_ceil(PADS);
        let with_notes = self.sampling.pattern_mode < 2;
        let order = chop::note_order(slices.len(), self.sampling.shuffle, self.shuffle_seed());
        let ticks_per_frame = self.slice_ticks_per_frame(&p, s.rate);
        let scene = self.scene;
        let bar = p.bar_length().max(1);
        let label = "Apply Slices to Pads";
        let mut made = Vec::new();
        let done = doc.borrow_mut().transaction(label, |d| {
            let clip = d.copy_sound(g, snd)?.without_notes();
            for k in 0..groups {
                let ng = d.add_group()?;
                let group_name = if groups > 1 { format!("{name} {}", k + 1) } else { name.clone() };
                d.rename(deckard_doc::Target::Group(ng), &group_name)?;
                made.push(group_name);
                for (pad, (i, &(a, b))) in slices.iter().enumerate().skip(k * PADS).take(PADS).enumerate() {
                    let zi = zone_of(a);
                    let zone = sampler.zones.get(zi).ok_or("the slice's zone is gone")?;
                    let b = if to_end { end } else { b };
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
                    // Its tempo and mode come along. With none it plays as a sampler does:
                    // said here, or the analysis of a New Sampler zone would put a loop in Sync.
                    let meta = zone.meta.clone().unwrap_or_else(|| ZoneMeta {
                        file: zone.sample.path.clone(),
                        mode: PlayMode::Repitch,
                        ..ZoneMeta::default()
                    });
                    d.paste_sounds(ng, pad, std::slice::from_ref(&clip))?;
                    d.set_zones(ng, pad, &[spec], label)?;
                    d.set_new_sampler(ng, pad, settings.as_ref())?;
                    if !original {
                        d.set_zone_meta(ng, pad, 0, &meta, label)?;
                    }
                    // A one-shot would play on to the end whatever the pad does: the notes end it.
                    if to_end && one_shot {
                        let t = deckard_doc::Target::Sampler(ng, pad);
                        d.set(label, t, "envelope_type", mxprj::decode::Value::Float(2.0))?;
                        d.set(label, t, "volume_envelope/release", mxprj::decode::Value::Float(0.1))?;
                    }
                    d.rename(deckard_doc::Target::Sound(ng, pad), &format!("{name} {}", i + 1))?;
                }
                if !with_notes {
                    continue;
                }
                // A note a slice at the slice's time: the groups' patterns together play the
                // sample (shuffled: a note's time is its own, the slice it plays another's).
                let pi = d.add_pattern(ng, 0, "Slices")?;
                let a0 = slices[0].0;
                let length = (((end - a0) as f64 * ticks_per_frame).ceil() as i64).max(1);
                d.set_pattern_length(ng, pi, (length + bar - 1) / bar * bar)?;
                let start = d.project().groups[ng].patterns[pi].start;
                let add: Vec<Note> = slices
                    .iter()
                    .zip(&order)
                    .filter(|(_, o)| **o / PADS == k)
                    .map(|(&(a, _), &o)| Note {
                        ratchet: Default::default(),
                        expr: None,
                        sound: o % PADS,
                        position: start + ((a - a0) as f64 * ticks_per_frame).round() as i64,
                        length: (((slices[o].1 - slices[o].0) as f64 * ticks_per_frame).round() as i64).max(1),
                        pitch: 60,
                        velocity: 100,
                        probability: 100,
                    })
                    .collect();
                d.edit_notes("Slice Pattern", ng, pi, &[], &add)?;
                // In the focused scene, so it plays with the others.
                if scene < d.project().scenes.len() {
                    d.assign_pattern(scene, ng, Some(pi))?;
                }
            }
            Ok(())
        });
        self.status = match done {
            Ok(()) => format!("{} slices are on the pads of {}", slices.len(), made.join(", ")),
            Err(e) => format!("can't apply the slices: {e}"),
        };
    }

    /// Pattern ticks a frame of the focused zone takes when its notes play it: at the
    /// song's tempo, or (a Sync zone, stretched to the song) at its own.
    pub(crate) fn slice_ticks_per_frame(&self, p: &deckard_model::Project, rate: u32) -> f64 {
        let zone = p.groups.get(self.group).and_then(|g| g.sounds.get(self.sound)).and_then(|s| {
            s.new_sampler.as_ref()?;
            s.sampler.as_ref()?.zones.get(self.zone)?.meta.as_ref()
        });
        let own = zone.filter(|m| m.mode == PlayMode::Sync).and_then(|m| m.bpm).filter(|b| b.is_finite() && *b > 0.0);
        own.unwrap_or(p.tempo) / 60.0 * deckard_model::PPQ as f64 / f64::from(rate.max(1))
    }

    /// The seed of a shuffled APPLY's order: another each time.
    pub(crate) fn shuffle_seed(&mut self) -> u64 {
        self.sampling.shuffles = self.sampling.shuffles.wrapping_add(1);
        self.sampling.seed ^ self.sampling.shuffles.wrapping_mul(0x9E37_79B9)
    }

    /// The slices as APPLY cuts them (each to the next), with the sample they are of.
    fn cut_slices(&self) -> Option<(std::sync::Arc<Sample>, Vec<(usize, usize)>)> {
        let s = self.sampling.sample.clone()?;
        let r = self.slice_range().unwrap_or((0, s.frames.len()));
        let starts = if self.sampling.slices.is_empty() { vec![r.0] } else { self.sampling.slices.clone() };
        let slices = deckard_engine::edit::slices_from(&starts, r.1.min(s.frames.len()));
        Some((s, slices))
    }

    /// The files the slices would be exported to in `dir`, in order.
    pub(crate) fn slice_files(&self, dir: &Path) -> Vec<std::path::PathBuf> {
        let n = self.cut_slices().map_or(0, |c| c.1.len());
        // (The focused zone's file, whichever way its path's separators go.)
        let file = self.doc.as_ref().and_then(|d| {
            let d = d.borrow();
            let z = d.project().groups.get(self.group)?.sounds.get(self.sound)?.sampler.as_ref()?.zones.get(self.zone)?;
            Some(z.sample.path.rsplit(['/', '\\']).next().unwrap_or("").to_string())
        });
        let stem = Path::new(&file.unwrap_or_default()).file_stem().map_or(String::new(), |x| x.to_string_lossy().into_owned());
        chop::slice_file_names(&stem, n).into_iter().map(|f| dir.join(f)).collect()
    }

    /// Each slice as a WAV file in `dir`; how many were written. Files there already are
    /// replaced only with `replace`: without, the first one found is the error, and
    /// nothing is written.
    pub(crate) fn export_slices_to(&mut self, dir: &Path, replace: bool) -> Result<usize, String> {
        let (s, slices) = self.cut_slices().ok_or("no sample to export")?;
        let files = self.slice_files(dir);
        if let Some(f) = files.iter().find(|f| !replace && f.exists()) {
            return Err(format!("{} is there already", f.display()));
        }
        for (f, &(a, b)) in files.iter().zip(&slices) {
            let piece = Sample { rate: s.rate, frames: s.frames[a..b].to_vec() };
            deckard_engine::edit::write_wav(f, &piece)?;
        }
        Ok(files.len())
    }

    /// EXPORT: asks for a folder (and before replacing files in it), then writes them.
    pub(crate) fn export_slices(&mut self) {
        if self.cut_slices().is_none() {
            return;
        }
        let Some(dir) = rfd::FileDialog::new().set_title("Export slices to").pick_folder() else { return };
        let taken = self.slice_files(&dir).iter().filter(|f| f.exists()).count();
        if taken > 0 {
            let ask = rfd::MessageDialog::new()
                .set_title("Export slices")
                .set_description(format!("{taken} of the files are in {} already. Replace them?", dir.display()))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if ask != rfd::MessageDialogResult::Yes {
                return;
            }
        }
        self.status = match self.export_slices_to(&dir, true) {
            Ok(n) => format!("Exported {n} slices to {}", dir.display()),
            Err(e) => format!("can't export the slices: {e}"),
        };
    }
}
