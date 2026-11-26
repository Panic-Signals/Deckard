//! A New Sampler sound's layers (`deckard_model::layers`): what the LAYERS tab and the
//! MK3's LAYERS page do to them. A layer is a zone over the sound's keys whose meta
//! says so; the morph is the New Sampler's.

use crate::App;
use crate::layout::sampling::{LayerAct, MorphAct};
use deckard_doc::{Document, Target, ZoneFrom, ZoneSpec};
use deckard_model::Project;
use deckard_model::layers::{self, FREE, Layer, MAX_LAYERS, SYNCED, TIME_MAX};
use deckard_model::new_sampler::{Motion, NewSampler, PlayMode, SamplerMode, ZoneMeta};
use mxprj::decode::Value;
use std::path::PathBuf;

/// The longest part of a dropped sample its root is heard in, seconds.
const ROOT_SECONDS: usize = 20;

/// Sound (`g`, `s`)'s Sampler Preview mode (None: it isn't one). What the sound offers
/// is asked of this, never of what its zones hold.
pub(crate) fn mode_of(p: &Project, g: usize, s: usize) -> Option<SamplerMode> {
    let snd = p.groups.get(g)?.sounds.get(s)?;
    snd.sampler.as_ref()?;
    snd.new_sampler.map(|n| n.mode)
}

/// Sound (`g`, `s`) has what is Sampler Preview's own (Sync and Keys, what analysis
/// finds, slices that play to the sample's end): one not in Original, which does what a
/// classic Sampler does and no more.
pub(crate) fn own(p: &Project, g: usize, s: usize) -> bool {
    mode_of(p, g, s).is_some_and(|m| m != SamplerMode::Original)
}

/// Sound (`g`, `s`) is a Sampler Preview in Advanced: its zones have a play mode (Sync,
/// Keys), a speed, reverse, a tail and Motion.
pub(crate) fn advanced(p: &Project, g: usize, s: usize) -> bool {
    mode_of(p, g, s) == Some(SamplerMode::Advanced)
}

/// Sound (`g`, `s`) has layers: a Sampler Preview in Layers (with none yet, too).
pub(crate) fn can_layer(p: &Project, g: usize, s: usize) -> bool {
    mode_of(p, g, s) == Some(SamplerMode::Layers)
}

/// Its layers, a lane each: (the zone, the layer). In Layers every zone is one: a zone
/// not yet given its layer data is shown with a new layer's (the next edit writes it).
pub(crate) fn layers_in(p: &Project, g: usize, s: usize) -> Vec<(usize, Layer)> {
    if !can_layer(p, g, s) {
        return Vec::new();
    }
    let zones = &p.groups[g].sounds[s].sampler.as_ref().expect("can_layer").zones;
    zones.iter().enumerate().map(|(k, z)| (k, z.meta.as_ref().and_then(|m| m.layer).unwrap_or_default())).take(MAX_LAYERS).collect()
}

/// Whether sound (`g`, `s`) has a layer (so MORPH is a modulation destination).
pub(crate) fn layered(p: &Project, g: usize, s: usize) -> bool {
    !layers_in(p, g, s).is_empty()
}

/// RECORD's LAYER when the take is on no layer.
pub(crate) const NO_LAYER: i64 = -1;

/// The lane of sound (`g`, `s`) that plays `take` (the first, if several do). A take
/// and its edited copies (`name - N.wav`) are the same take, whichever the list or the
/// layer holds.
pub(crate) fn take_layer(p: &Project, g: usize, s: usize, take: &std::path::Path) -> Option<usize> {
    use deckard_engine::edit::edited_base;
    let base = edited_base(&take.file_name()?.to_string_lossy());
    let zones = &p.groups.get(g)?.sounds.get(s)?.sampler.as_ref()?.zones;
    layers_in(p, g, s).iter().position(|(k, _)| edited_base(&crate::analysis::file_name_of(&zones[*k].sample.path)) == base)
}

/// What RECORD's LAYER can be for the focused take on sound (`g`, `s`): each choice's
/// name and what it sets (`NO_LAYER`, a lane, or `MAX_LAYERS` for a new layer), and the
/// one the take is at. None: no take is focused, the sound can't have layers, or the
/// recorder's mode has a target of its own (Loop, Auto).
fn take_options(p: &Project, g: usize, s: usize, rec: &crate::layout::sampling::RecorderView) -> Option<(Vec<String>, Vec<i64>, i64)> {
    let take = rec.focused.and_then(|i| rec.takes.get(i))?;
    if rec.mode > 1 || !can_layer(p, g, s) {
        return None;
    }
    let have = layers_in(p, g, s).len();
    let mut options = vec!["None".to_string()];
    let mut values = vec![NO_LAYER];
    options.extend((1..=have).map(|k| k.to_string()));
    values.extend(0..have as i64);
    if have < MAX_LAYERS {
        options.push("New".into());
        values.push(MAX_LAYERS as i64);
    }
    let on = take_layer(p, g, s, take).map_or(NO_LAYER, |k| k as i64);
    Some((options, values, on))
}

/// RECORD's LAYER: the choices, the one shown (the one chosen, until it is applied; else
/// the focused take's layer), and what each sets.
pub(crate) fn take_choices(
    p: &Project,
    g: usize,
    s: usize,
    rec: &crate::layout::sampling::RecorderView,
) -> Option<(Vec<String>, usize, Vec<i64>)> {
    let (options, values, on) = take_options(p, g, s, rec)?;
    let shown = rec.layer_choice_for((g, s)).filter(|c| values.contains(c)).unwrap_or(on);
    let at = values.iter().position(|&v| v == shown).unwrap_or(0);
    Some((options, at, values))
}

/// The layer chosen for the focused take and not yet applied (APPLY puts it there).
pub(crate) fn layer_pending(p: &Project, g: usize, s: usize, rec: &crate::layout::sampling::RecorderView) -> Option<i64> {
    let (_, values, on) = take_options(p, g, s, rec)?;
    rec.layer_choice_for((g, s)).filter(|c| values.contains(c) && *c != on)
}

fn zones_of(d: &Document, g: usize, s: usize) -> Vec<deckard_model::Zone> {
    d.project().groups[g].sounds[s].sampler.as_ref().map(|x| x.zones.clone()).unwrap_or_default()
}

fn settings_of(d: &Document, g: usize, s: usize) -> NewSampler {
    d.project().groups[g].sounds[s].new_sampler.unwrap_or_default()
}

/// The sound's first use of layers: a lone plain zone becomes the first layer, over all
/// keys and playing repitched; a One-shot amp envelope becomes ADSR (so notes end, and a
/// layer's release has something to follow); Slice Motion goes off.
pub(crate) fn make_layered(d: &mut Document, g: usize, s: usize, label: &str) -> Result<(), String> {
    let zones = zones_of(d, g, s);
    for (k, z) in zones.iter().enumerate() {
        if z.meta.as_ref().is_some_and(|m| m.layer.is_some()) {
            continue;
        }
        let was = z.meta.clone().unwrap_or_else(|| ZoneMeta { file: z.sample.path.clone(), ..Default::default() });
        let meta = ZoneMeta { layer: Some(Layer::default()), mode: PlayMode::Repitch, ..was };
        d.set_zone_meta(g, s, k, &meta, label)?;
        for (field, v) in [("key_range/low", 0.0), ("key_range/high", 127.0), ("velocity_range/low", 0.0), ("velocity_range/high", 127.0)] {
            d.set(label, Target::Zone(g, s, k), field, Value::Float(v))?;
        }
    }
    if d.project().groups[g].sounds[s].sampler.as_ref().is_some_and(|x| x.envelope_type == 0) {
        d.set(label, Target::Sampler(g, s), "envelope_type", Value::Float(2.0))?;
    }
    let n = settings_of(d, g, s);
    if n.motion != Motion::Off {
        d.set_new_sampler_as(g, s, &NewSampler { motion: Motion::Off, ..n }, label)?;
    }
    Ok(())
}

/// Whether the sound isn't layered yet (its lone zone is plain, or its envelope
/// One-shot): the next edit makes it so.
fn plain(d: &Document, g: usize, s: usize) -> bool {
    zones_of(d, g, s).iter().any(|z| !z.meta.as_ref().is_some_and(|m| m.layer.is_some()))
}

/// Give the layers these windows, lane by lane.
fn set_windows(d: &mut Document, g: usize, s: usize, windows: &[[f64; 4]], label: &str) -> Result<(), String> {
    for (k, z) in zones_of(d, g, s).iter().enumerate() {
        let (Some(meta), Some(&window)) = (z.meta.as_ref(), windows.get(k)) else { continue };
        let Some(layer) = meta.layer else { continue };
        if layer.window != window {
            d.set_zone_meta(g, s, k, &ZoneMeta { layer: Some(Layer { window, ..layer }), ..meta.clone() }, label)?;
        }
    }
    Ok(())
}

/// The windows spread evenly again, if they are as SPREAD left them.
fn respread(d: &mut Document, g: usize, s: usize, label: &str) -> Result<(), String> {
    let m = settings_of(d, g, s).morph;
    if !m.spread {
        return Ok(());
    }
    let n = zones_of(d, g, s).len();
    set_windows(d, g, s, &layers::spread(n, m.shape), label)
}

/// A window with point `point` put at `v`, its neighbours moved along so A ≤ B ≤ C ≤ D.
pub(crate) fn window_moved(mut w: [f64; 4], point: usize, v: f64) -> [f64; 4] {
    let point = point.min(3);
    let v = if v.is_finite() { v.clamp(-1.0, 2.0) } else { w[point] };
    w[point] = v;
    for k in 0..point {
        w[k] = w[k].min(v);
    }
    for x in w.iter_mut().skip(point + 1) {
        *x = x.max(v);
    }
    w
}

impl App {
    /// In Layers, the take EDIT works on in place of a layer: the focused take, once it
    /// was picked ([`crate::layout::sampling::RecordAct::FocusTake`]) and while it is on
    /// no layer. (One that is on a layer is edited as that layer.) Its place in the take
    /// list, and its file.
    pub(crate) fn shown_take(&self) -> Option<(usize, PathBuf)> {
        let (g, s) = (self.group, self.sound);
        if self.take_shown != Some((g, s)) {
            return None;
        }
        let v = &self.recorder.view;
        let i = v.focused?;
        let take = v.takes.get(i)?;
        let d = self.doc.as_ref()?.borrow();
        (can_layer(d.project(), g, s) && take_layer(d.project(), g, s, take).is_none()).then(|| (i, take.clone()))
    }

    /// SAMPLER: the focused sound's mode asked for. One with a sample asks first (the
    /// change keeps that sample and loses the rest): [`App::answer_sampler_mode`]. One
    /// without, and Original made Advanced (which loses nothing), change at once.
    /// `knob`: from the controller's knob, where nothing changes until confirmed (a
    /// knob is brushed, and passes Advanced on its way from Original to Layers).
    pub(crate) fn ask_sampler_mode(&mut self, mode: SamplerMode, knob: bool) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, s) = (self.group, self.sound);
        let found = {
            let d = doc.borrow();
            mode_of(d.project(), g, s).map(|now| (now, d.project().groups[g].sounds[s].sampler.as_ref().map_or(0, |x| x.zones.len())))
        };
        let Some((now, zones)) = found else { return };
        if self.recorder.busy() {
            self.status = "the mode can't be changed while recording".into();
            return;
        }
        self.sampler_ask = None;
        if now == mode {
            return;
        }
        let empty = zones == 0 && self.shown_take().is_none();
        if empty || (!knob && now == SamplerMode::Original && mode == SamplerMode::Advanced) {
            self.change_sampler_mode(mode, false);
        } else {
            self.sampler_ask = Some(((g, s), mode));
        }
    }

    /// The mode change asked for, answered: OK goes ahead (if the sound it was asked of
    /// is still the focused one, and nothing is being recorded).
    pub(crate) fn answer_sampler_mode(&mut self, ok: bool) {
        let Some((at, mode)) = self.sampler_ask.take() else { return };
        if ok && at == (self.group, self.sound) && !self.recorder.busy() {
            // (Original made Advanced keeps everything, asked or not.)
            let now = self.doc.as_ref().and_then(|d| mode_of(d.borrow().project(), at.0, at.1));
            self.change_sampler_mode(mode, !(now == Some(SamplerMode::Original) && mode == SamplerMode::Advanced));
        }
    }

    /// The focused sound in `mode`, as one undo step. `keep`: with the sample the
    /// Sampling view shows (a zone's, a layer's, or a take's on no layer) as its only
    /// one; else as it is.
    fn change_sampler_mode(&mut self, mode: SamplerMode, keep: bool) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, s) = (self.group, self.sound);
        let take = self.shown_take().filter(|_| keep);
        let done = if let Some((_, file)) = &take {
            match deckard_engine::sample::Sample::load(file) {
                Ok(smp) => {
                    let frames = smp.frames.len();
                    doc.borrow_mut().transaction("Sampler Mode", |d| {
                        let from = if zones_of(d, g, s).is_empty() { ZoneFrom::Template } else { ZoneFrom::Zone(0) };
                        d.set_zones(g, s, &[crate::new_zone(file, frames, from)], "Sampler Mode")?;
                        d.set_sampler_mode(g, s, mode, Some((0, frames)))
                    })
                }
                Err(e) => Err(format!("can't read {}: {e}", file.display())),
            }
        } else if keep {
            // The zone shown, and its file's length if that is the one loaded.
            let key = self.sampling.sample_key.as_ref().filter(|k| (k.0, k.1) == (g, s));
            let zone = key.map_or(self.zone, |k| k.2);
            let frames = if key.is_some() { self.sampling.len() } else { 0 };
            let zones = zones_of(&doc.borrow(), g, s).len();
            doc.borrow_mut().set_sampler_mode(g, s, mode, Some((zone.min(zones.saturating_sub(1)), frames)))
        } else {
            doc.borrow_mut().set_sampler_mode(g, s, mode, None)
        };
        match done {
            Ok(()) => {
                if keep {
                    self.zone = 0;
                    self.sampling.selection = None;
                    self.sampling.slices.clear();
                }
                self.take_shown = None;
                // A tab the mode doesn't have: EDIT (RECORD without a sample).
                let p = doc.borrow().project().clone();
                let zone = !zones_of(&doc.borrow(), g, s).is_empty();
                if !crate::layout::sampling::tab_shown(&p, g, s, self.sampling.tab, zone, false) {
                    self.sampling.tab = usize::from(zone);
                }
                self.status = format!("Sampler: {}", mode.name());
            }
            Err(e) => self.status = format!("can't change the mode: {e}"),
        }
    }

    /// The focused sound's layers ([`layers_in`]).
    pub(crate) fn layers_of(&self) -> Vec<(usize, Layer)> {
        self.doc.as_ref().map_or_else(Vec::new, |d| layers_in(d.borrow().project(), self.group, self.sound))
    }

    /// A take recorded for sound (`g`, `s`): its sample. In Layers the first take is the
    /// first layer, and a sound with layers keeps them: the take waits in the list, for
    /// RECORD's LAYER and APPLY.
    pub(crate) fn take_into(&mut self, g: usize, s: usize, path: &std::path::Path, frames: usize) {
        let layers = self.doc.as_ref().and_then(|d| {
            let d = d.borrow();
            can_layer(d.project(), g, s).then(|| layers_in(d.project(), g, s).len())
        });
        match layers {
            Some(0) => self.take_to_layer(g, s, path, 0),
            Some(_) => {}
            None => self.load_take(g, s, path, frames),
        }
    }

    /// APPLY: the focused take onto the layer RECORD's LAYER was turned to; None takes
    /// the layer it is on away.
    pub(crate) fn apply_take_layer(&mut self) {
        let (g, s) = (self.group, self.sound);
        let v = &self.recorder.view;
        let take = v.focused.and_then(|i| v.takes.get(i)).cloned();
        let found = self.doc.as_ref().and_then(|d| {
            let d = d.borrow();
            let to = layer_pending(d.project(), g, s, v)?;
            Some((to, take.as_ref().and_then(|t| take_layer(d.project(), g, s, t))))
        });
        self.recorder.view.layer_choice = None;
        match (found, take) {
            (Some((NO_LAYER, Some(lane))), _) => self.layer_act(lane, LayerAct::Remove),
            (Some((to, _)), Some(take)) if to >= 0 => self.take_to_layer(g, s, &take, to as usize),
            _ => {}
        }
    }

    /// `path` onto lane `lane` of sound (`g`, `s`) (past its layers: a new one).
    pub(crate) fn take_to_layer(&mut self, g: usize, s: usize, path: &std::path::Path, lane: usize) {
        let keep = (self.group, self.sound, self.zone);
        (self.group, self.sound) = (g, s);
        self.layer_files(lane, &[path.to_path_buf()]);
        if (g, s) != (keep.0, keep.1) {
            (self.group, self.sound, self.zone) = keep;
        }
    }

    /// `files` onto the focused sound's lanes from `lane` on: an empty lane gets a new
    /// layer, a full one has its sample replaced (its settings kept). Each sample's root
    /// is heard, so the layers play in tune. One undo step.
    pub(crate) fn layer_files(&mut self, lane: usize, files: &[PathBuf]) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, s) = (self.group, self.sound);
        let p = doc.borrow().project().clone();
        if !can_layer(&p, g, s) {
            self.status = "layers are for a Sampler Preview in Layers".into();
            return;
        }
        let have = layers_in(&p, g, s).len();
        let lane = lane.min(have);
        let fit = files.len().min(MAX_LAYERS.saturating_sub(lane));
        let mut loaded = Vec::new();
        for file in &files[..fit] {
            match deckard_engine::sample::Sample::load(file) {
                Ok(smp) => {
                    let part = &smp.frames[..smp.frames.len().min(ROOT_SECONDS * smp.rate as usize)];
                    let root = deckard_analysis::root(part, smp.rate);
                    loaded.push((file.clone(), smp.frames.len(), root));
                }
                Err(e) => {
                    self.status = format!("can't read {}: {e}", file.display());
                    return;
                }
            }
        }
        if loaded.is_empty() {
            if !files.is_empty() {
                self.status = format!("{} didn't fit: a sound has {MAX_LAYERS} layers", files.len());
            }
            return;
        }
        let added = lane + loaded.len() > have;
        let label = if added { "Add Layer" } else { "Replace Layer" };
        let done = doc.borrow_mut().transaction(label, |d| {
            make_layered(d, g, s, label)?;
            let zones = zones_of(d, g, s);
            let keys = zones.first().map_or((0, 127), |z| z.key_range);
            let last_window = zones.last().and_then(|z| z.meta.as_ref()?.layer).map(|l| l.window);
            let mut specs: Vec<ZoneSpec> =
                (0..zones.len()).map(|k| ZoneSpec { from: ZoneFrom::Zone(k), file: None, set: Vec::new() }).collect();
            for (i, (file, frames, root)) in loaded.iter().enumerate() {
                let k = lane + i;
                let end = frames.saturating_sub(1) as f64;
                let mut set = vec![
                    ("play_range/low", 0.0),
                    ("play_range/high", end),
                    ("loop_range/low", 0.0),
                    ("loop_range/high", end),
                    ("loop_active", 0.0),
                    ("root_key", root.map_or(60.0, |r| f64::from(r.note))),
                    // In tune: as far the other way as the sample sits from its root.
                    ("tune", root.map_or(0.0, |r| -f64::from(r.cents) / 100.0)),
                ];
                if k < zones.len() {
                    specs[k] = ZoneSpec { from: ZoneFrom::Zone(k), file: Some(file.clone()), set };
                } else {
                    set.extend([
                        ("key_range/low", keys.0 as f64),
                        ("key_range/high", keys.1 as f64),
                        ("velocity_range/low", 0.0),
                        ("velocity_range/high", 127.0),
                        ("gain", 0.75),
                        ("pan", 0.0),
                        ("attack", 0.0),
                        ("decay", 0.0),
                        ("x_fade", 0.0),
                    ]);
                    let from = if zones.is_empty() { ZoneFrom::Template } else { ZoneFrom::Zone(0) };
                    specs.push(ZoneSpec { from, file: Some(file.clone()), set });
                }
            }
            d.set_zones(g, s, &specs, label)?;
            for (i, (_, _, root)) in loaded.iter().enumerate() {
                let k = lane + i;
                let path = d.project().groups[g].sounds[s].sampler.as_ref().ok_or("no sampler")?.zones[k].sample.path.clone();
                // A replaced layer keeps its settings; a new one has the last one's window
                // (spread again below, if they are spread).
                let layer = zones
                    .get(k)
                    .and_then(|z| z.meta.as_ref()?.layer)
                    .unwrap_or(Layer { window: last_window.unwrap_or([0.0, 0.0, 1.0, 1.0]), ..Default::default() });
                let meta = ZoneMeta {
                    file: path,
                    mode: PlayMode::Repitch,
                    root: root.map(|r| r.note),
                    cents: root.map_or(0.0, |r| r.cents),
                    root_guess: root.is_some_and(|r| r.guess),
                    key_minor: root.and_then(|r| r.minor),
                    layer: Some(layer),
                    ..Default::default()
                };
                d.set_zone_meta(g, s, k, &meta, label)?;
            }
            respread(d, g, s, label)
        });
        match done {
            Ok(()) => {
                self.zone = lane + loaded.len() - 1;
                let left = files.len() - loaded.len();
                self.status = if left > 0 {
                    format!("{left} didn't fit: a sound has {MAX_LAYERS} layers")
                } else if loaded.len() == 1 {
                    format!("Layer {}: {}", lane + 1, crate::analysis::file_name_of(&loaded[0].0.to_string_lossy()))
                } else {
                    format!("{} layers, from {}", loaded.len(), lane + 1)
                };
            }
            Err(e) => self.status = format!("can't add the layer: {e}"),
        }
    }

    /// Change the layer on `lane` (its meta's), as the undo step `label`.
    fn edit_layer(&mut self, lane: usize, label: &str, f: impl FnOnce(&mut Layer)) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, s) = (self.group, self.sound);
        let mut d = doc.borrow_mut();
        if layers_in(d.project(), g, s).get(lane).is_none() {
            return;
        }
        let go = |d: &mut Document, f: Box<dyn FnOnce(&mut Layer) + '_>| -> Result<(), String> {
            let z = zones_of(d, g, s).get(lane).cloned().ok_or("no such layer")?;
            let meta = z.meta.ok_or("not a layer")?;
            let mut layer = meta.layer.ok_or("not a layer")?;
            f(&mut layer);
            d.set_zone_meta(g, s, lane, &ZoneMeta { layer: Some(layer), ..meta }, label)
        };
        let done = if plain(&d, g, s) {
            d.transaction(label, |d| {
                make_layered(d, g, s, label)?;
                go(d, Box::new(f))
            })
        } else {
            go(&mut d, Box::new(f))
        };
        drop(d);
        if let Err(e) = done {
            self.status = format!("can't change the layer: {e}");
        }
    }

    /// Set one of the layer zone's own fields (its level, pan, tune, root).
    fn set_layer_zone(&mut self, lane: usize, label: &str, field: &str, v: f64) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, s) = (self.group, self.sound);
        let mut d = doc.borrow_mut();
        if layers_in(d.project(), g, s).get(lane).is_none() || !v.is_finite() {
            return;
        }
        let done = if plain(&d, g, s) {
            d.transaction(label, |d| {
                make_layered(d, g, s, label)?;
                d.set(label, Target::Zone(g, s, lane), field, Value::Float(v))
            })
        } else {
            d.set(label, Target::Zone(g, s, lane), field, Value::Float(v))
        };
        drop(d);
        if let Err(e) = done {
            self.status = format!("can't change the layer: {e}");
        }
    }

    /// The layers in this order of their zones (a removal, a copy, a move), their windows
    /// staying with the lanes; one undo step.
    fn reorder_layers(&mut self, order: &[usize], label: &str) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, s) = (self.group, self.sound);
        let done = doc.borrow_mut().transaction(label, |d| {
            make_layered(d, g, s, label)?;
            let windows: Vec<[f64; 4]> = zones_of(d, g, s).iter().filter_map(|z| Some(z.meta.as_ref()?.layer?.window)).collect();
            let specs: Vec<ZoneSpec> = order.iter().map(|&k| ZoneSpec { from: ZoneFrom::Zone(k), file: None, set: Vec::new() }).collect();
            d.set_zones(g, s, &specs, label)?;
            // (Moved, the windows stay with the lanes; one more layer has the last's; one
            // fewer, the rest keep their own.)
            let kept: Vec<[f64; 4]> =
                (0..order.len()).map(|k| windows.get(k).or(windows.last()).copied().unwrap_or([0.0, 0.0, 1.0, 1.0])).collect();
            if order.len() >= windows.len() {
                set_windows(d, g, s, &kept, label)?;
            }
            respread(d, g, s, label)
        });
        if let Err(e) = done {
            self.status = format!("can't change the layers: {e}");
        }
    }

    /// What the LAYERS tab or the MK3 asks of the layer on `lane`.
    pub(crate) fn layer_act(&mut self, lane: usize, act: LayerAct) {
        let time = |v: f64| if v.is_finite() { v.clamp(0.0, TIME_MAX) } else { 0.0 };
        let n = self.layers_of().len();
        match act {
            LayerAct::Files(files) => self.layer_files(lane, &files),
            _ if lane >= n => {}
            LayerAct::Focus => {
                self.zone = lane;
                // (EDIT works on the layer picked, not on a take picked before.)
                self.take_shown = None;
            }
            LayerAct::Mute => self.edit_layer(lane, "Layer Mute", |l| l.mute = !l.mute),
            LayerAct::Solo => self.edit_layer(lane, "Layer Solo", |l| l.solo = !l.solo),
            LayerAct::Level(v) => self.set_layer_zone(lane, "Layer Level", "gain", v.clamp(0.0, 1.0)),
            LayerAct::Pan(v) => self.set_layer_zone(lane, "Layer Pan", "pan", v.clamp(-1.0, 1.0)),
            LayerAct::Tune(v) => self.set_layer_zone(lane, "Layer Tune", "tune", v.clamp(-36.0, 36.0)),
            LayerAct::Root(k) => self.set_layer_zone(lane, "Layer Root", "root_key", k.clamp(0, 127) as f64),
            LayerAct::Attack(v) => self.edit_layer(lane, "Layer Envelope", |l| l.attack = time(v)),
            LayerAct::Decay(v) => self.edit_layer(lane, "Layer Envelope", |l| l.decay = time(v)),
            LayerAct::Sustain(v) => {
                self.edit_layer(lane, "Layer Envelope", |l| l.sustain = if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 })
            }
            LayerAct::Release(v) => self.edit_layer(lane, "Layer Envelope", |l| l.release = time(v)),
            LayerAct::Window(point, v) => {
                // Moved by hand: the windows are no longer spread by themselves.
                self.set_morph("Layer Window", |m| m.spread = false);
                self.edit_layer(lane, "Layer Window", |l| l.window = window_moved(l.window, point, v));
            }
            LayerAct::Remove => {
                let order: Vec<usize> = (0..n).filter(|&k| k != lane).collect();
                self.reorder_layers(&order, "Remove Layer");
                self.zone = self.zone.min(n.saturating_sub(2));
            }
            LayerAct::Duplicate if n < MAX_LAYERS => {
                let mut order: Vec<usize> = (0..n).collect();
                order.insert(lane + 1, lane);
                self.reorder_layers(&order, "Duplicate Layer");
                self.zone = lane + 1;
            }
            LayerAct::Duplicate => self.status = format!("a sound has {MAX_LAYERS} layers"),
            LayerAct::MoveTo(to) => {
                let to = to.min(n - 1);
                if to != lane {
                    let mut order: Vec<usize> = (0..n).collect();
                    let k = order.remove(lane);
                    order.insert(to, k);
                    self.reorder_layers(&order, "Move Layer");
                    self.zone = to;
                }
            }
        }
    }

    /// Change the focused sound's morph, as the undo step `label`.
    fn set_morph(&mut self, label: &str, f: impl FnOnce(&mut layers::Morph)) {
        let Some(doc) = self.doc.clone() else { return };
        let (g, s) = (self.group, self.sound);
        let mut d = doc.borrow_mut();
        let Some(mut n) = d.project().groups.get(g).and_then(|x| x.sounds.get(s)).and_then(|x| x.new_sampler) else { return };
        f(&mut n.morph);
        let done = d.set_new_sampler_as(g, s, &n, label);
        drop(d);
        if let Err(e) = done {
            self.status = format!("can't change the morph: {e}");
        }
    }

    /// What the LAYERS tab or the MK3 asks of the focused sound's morph, or of all its
    /// layers.
    pub(crate) fn morph_act(&mut self, act: MorphAct) {
        let unit = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        let Some(doc) = self.doc.clone() else { return };
        let (g, s) = (self.group, self.sound);
        if !can_layer(doc.borrow().project(), g, s) {
            return;
        }
        match act {
            MorphAct::On(on) => self.set_morph("Morph", |m| m.on = on),
            MorphAct::Position(v) => self.set_morph("Morph Position", |m| m.position = unit(v)),
            MorphAct::Sync(on) => self.set_morph("Morph Rate", |m| m.sync = on),
            MorphAct::Bars(v) => self.set_morph("Morph Rate", |m| {
                // The nearest of the synced cycles.
                m.bars = SYNCED.iter().copied().min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs())).unwrap_or(8.0);
                m.sync = true;
            }),
            MorphAct::Seconds(v) => self.set_morph("Morph Rate", |m| {
                m.seconds = if v.is_finite() { v.clamp(FREE.0, FREE.1) } else { m.seconds };
                m.sync = false;
            }),
            MorphAct::Depth(v) => self.set_morph("Morph Depth", |m| m.depth = unit(v)),
            MorphAct::Reset(on) => self.set_morph("Morph", |m| m.reset = on),
            MorphAct::Fade(fade) => self.set_morph("Morph", |m| m.fade = fade),
            MorphAct::Shape(shape) => {
                let done = doc.borrow_mut().transaction("Morph Shape", |d| {
                    let mut n = settings_of(d, g, s);
                    n.morph.shape = shape;
                    d.set_new_sampler_as(g, s, &n, "Morph Shape")?;
                    respread(d, g, s, "Morph Shape")
                });
                if let Err(e) = done {
                    self.status = format!("can't change the morph: {e}");
                }
            }
            MorphAct::Spread => {
                let done = doc.borrow_mut().transaction("Spread Layers", |d| {
                    let mut n = settings_of(d, g, s);
                    n.morph.spread = true;
                    d.set_new_sampler_as(g, s, &n, "Spread Layers")?;
                    respread(d, g, s, "Spread Layers")
                });
                if let Err(e) = done {
                    self.status = format!("can't spread the layers: {e}");
                }
            }
            MorphAct::Keys(lo, hi) => {
                let (lo, hi) = (lo.clamp(0, 127), hi.clamp(0, 127));
                let (lo, hi) = (lo.min(hi), lo.max(hi));
                let done = doc.borrow_mut().transaction_joined("Layer Keys", |d| {
                    make_layered(d, g, s, "Layer Keys")?;
                    for k in 0..zones_of(d, g, s).len() {
                        // (Low first going down, high first going up: the range is never empty.)
                        let now = zones_of(d, g, s)[k].key_range;
                        let fields = if lo > now.1 {
                            [("key_range/high", hi), ("key_range/low", lo)]
                        } else {
                            [("key_range/low", lo), ("key_range/high", hi)]
                        };
                        for (field, v) in fields {
                            d.set("Layer Keys", Target::Zone(g, s, k), field, Value::Float(v as f64))?;
                        }
                    }
                    Ok(())
                });
                if let Err(e) = done {
                    self.status = format!("can't set the keys: {e}");
                }
            }
            MorphAct::Add => self.pick_layer_files(),
        }
    }

    /// + LAYER: choose files for the next empty lanes.
    fn pick_layer_files(&mut self) {
        let n = self.layers_of().len();
        if n >= MAX_LAYERS {
            self.status = format!("a sound has {MAX_LAYERS} layers");
            return;
        }
        let files = rfd::FileDialog::new().add_filter("Audio", &["wav", "aif", "aiff", "mp3", "flac", "ogg"]).pick_files();
        if let Some(files) = files.filter(|f| !f.is_empty()) {
            self.layer_files(n, &files);
        }
    }
}
