//! Zhora's screens: what the MK3 and the plug-in panel show of it (its tuner, buttons
//! and bar), the channels its outputs can take, and whether a project has one.

use deckard_doc::Chain;
use deckard_engine::zhora_outs;
use deckard_model::AudioSource;
use deckard_model::Project;
use deckard_plugins::native::{InputSource, Readout, TuneState};
use deckard_plugins::zhora::{
    ATTACK, CHECK, DECAY, HOLD, HOLD_NOTE, INPUT, LIVE, OUTS, RELEASE, SIGNAL_NAMES, SUSTAIN, TUNE, input_of, is_zhora,
};
use deckard_surface::{TunerView, ZhoraView};

use crate::App;
use crate::settings::Settings;

/// How often the Zhoras are looked at for a finished Tune.
const TUNE_WATCH: std::time::Duration = std::time::Duration::from_millis(250);

/// A note within this many cents is in tune.
const IN_TUNE: f32 = 5.0;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Whether a sound of `p` is played by Zhora.
pub fn zhora_in(p: &Project) -> bool {
    p.groups.iter().flat_map(|g| &g.sounds).any(|s| s.plugin.as_ref().is_some_and(|pr| is_zhora(&pr.id)))
}

/// A picker's step moved `step` places along the steps it offers (`list`, in order),
/// stopping at either end.
pub fn step_choice(current: usize, step: i64, list: &[usize]) -> usize {
    let Some(&first) = list.first() else { return current };
    let at = list.iter().position(|&v| v == current).map_or(0, |a| a as i64);
    list.get((at + step).clamp(0, list.len() as i64 - 1) as usize).copied().unwrap_or(first)
}

/// The steps of Zhora's Input picker in channel order: None, Sound's Input, then the
/// connected input channels (and `current`'s, connected or not).
pub fn input_steps(settings: &Settings, current: usize) -> Vec<usize> {
    let mut list = vec![0, 1];
    list.extend((0..32).map(|c| c + 2).filter(|&n| n == current || settings.input_on(n - 2)));
    list
}

/// What step `step` of Zhora's picker `id` (an output or its Input) is called, by the
/// names given in Audio Setup; None for other knobs.
pub fn choice_name(settings: &Settings, id: u32, step: usize) -> Option<String> {
    if OUTS.contains(&id) {
        return Some(step.checked_sub(1).map_or_else(|| "None".into(), |c| settings.output_name(c)));
    }
    (id == INPUT).then(|| match input_of(step as f64 / deckard_plugins::zhora::INPUT_STEPS as f64) {
        None => "None".into(),
        Some(InputSource::Sound) => "Sound's Input".into(),
        Some(InputSource::Channel(c)) => settings.input_name(c),
    })
}

/// What the Zhora on sound (`g`, `s`) listens on, as the tuner names it: a channel, or
/// its sound's input and what that is set to.
pub fn input_label(settings: &Settings, p: &Project, (g, s): (usize, usize), input: Option<InputSource>) -> String {
    match input {
        None => "No input".into(),
        Some(InputSource::Channel(c)) => settings.input_name(c),
        Some(InputSource::Sound) => {
            let audio_in = p.groups.get(g).and_then(|grp| grp.sounds.get(s)).map(|snd| snd.channel.audio_in);
            match audio_in {
                Some(AudioSource::Ext(n)) if (0..16).contains(&n) => settings.input_pair_name(n as usize),
                Some(AudioSource::Ext(n)) if (16..48).contains(&n) => settings.input_name(n as usize - 16),
                _ => "Sound's Input (none set)".into(),
            }
        }
    }
}

/// "+3¢", "−12¢", "0¢".
pub fn cents_text(cents: f32) -> String {
    match cents.abs().round() as i32 {
        0 => "0¢".into(),
        n if cents < 0.0 => format!("\u{2212}{n}¢"),
        n => format!("+{n}¢"),
    }
}

/// The channels the Zhoras' outputs have: a new CV route doesn't get them.
pub fn channels_held(p: &Project) -> Vec<u8> {
    zhora_outs(p).iter().map(|o| o.channel).collect()
}

/// The channels output `signal` of the Zhora on sound (`g`, `s`) can take: the CV only
/// ones that no CV route, other Zhora or other output of its own has. The one it's on
/// stays.
pub fn offered(p: &Project, settings: &Settings, (g, s): (usize, usize), signal: usize) -> Vec<usize> {
    let outs = zhora_outs(p);
    let me = |o: &&deckard_engine::ZhoraOut| (o.g, o.s, o.signal) == (g, s, signal);
    let mine = outs.iter().find(me).map(|o| usize::from(o.channel));
    let taken =
        |c: usize| p.cv_routes.iter().any(|r| usize::from(r.channel) == c) || outs.iter().any(|o| !me(&o) && usize::from(o.channel) == c);
    let mut list: Vec<usize> = settings.cv_channels().into_iter().filter(|&c| Some(c) == mine || !taken(c)).collect();
    if let Some(c) = mine.filter(|c| !list.contains(c)) {
        list.push(c);
        list.sort_unstable();
    }
    list
}

/// Why an output of the Zhora on sound (`g`, `s`) isn't sent, naming it: its channel
/// isn't CV only, or a CV route or an earlier output has it (as the engine decides).
/// `None` when every output is sent.
pub fn output_problem(p: &Project, settings: &Settings, (g, s): (usize, usize)) -> Option<String> {
    let cv = settings.cv_channels();
    let outs = zhora_outs(p);
    for (k, o) in outs.iter().enumerate().filter(|(_, o)| (o.g, o.s) == (g, s)) {
        let c = usize::from(o.channel);
        let name = settings.output_name(c);
        let why = if !cv.contains(&c) {
            format!("{name} isn't CV only")
        } else if p.cv_routes.iter().any(|r| r.channel == o.channel) {
            format!("a CV route has {name}")
        } else if let Some(e) = outs[..k].iter().find(|e| e.channel == o.channel) {
            if (e.g, e.s) == (g, s) { format!("{} has {name}", SIGNAL_NAMES[e.signal]) } else { format!("another Zhora has {name}") }
        } else {
            continue;
        };
        return Some(format!("{} isn't sent: {why}", SIGNAL_NAMES[o.signal]));
    }
    None
}

/// The parameters a press of `what` sets, in order: Tune and Check pressed and let go,
/// Live switched, Hold switched (on from off, whatever value was saved).
pub fn press(what: &str, holding: bool, live: bool) -> Vec<(u32, f64)> {
    match what {
        "tune" => vec![(TUNE, 1.0), (TUNE, 0.0)],
        "check" => vec![(CHECK, 1.0), (CHECK, 0.0)],
        "live" => vec![(LIVE, if live { 0.0 } else { 1.0 })],
        "hold" if holding => vec![(HOLD, 0.0)],
        "hold" => vec![(HOLD, 0.0), (HOLD, 1.0)],
        _ => Vec::new(),
    }
}

/// `n` (MIDI) as DECKARD names it.
fn note(n: u8) -> String {
    crate::keys::note_name(i64::from(n))
}

/// The day (from 1970), hour and minute of `secs` (Unix) in local time.
fn local(secs: u64) -> (i64, i64, i64) {
    let t = secs as i64;
    let t = t + crate::recorder::local_offset(t);
    (t.div_euclid(86_400), t.rem_euclid(86_400) / 3600, t.rem_euclid(3600) / 60)
}

/// When `at` was, from `now`: "today 14:02", "yesterday 14:02", "7 Nov 14:02".
fn when(at: u64, now: u64) -> String {
    let ((day, h, m), (today, ..)) = (local(at), local(now));
    match today - day {
        0 => format!("today {h:02}:{m:02}"),
        1 => format!("yesterday {h:02}:{m:02}"),
        _ => {
            // The civil date (Howard Hinnant's algorithm, as recorder.rs).
            let z = day + 719_468;
            let doe = z - z.div_euclid(146_097) * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = doy - (153 * mp + 2) / 5 + 1;
            let month = if mp < 10 { mp + 3 } else { mp - 9 };
            format!("{d} {} {h:02}:{m:02}", MONTHS[month as usize - 1])
        }
    }
}

/// The bar: "Tuned today 14:02 · worst note 3¢ · C1–C7", with "· off: C7 D7" for the
/// notes over target; "Not tuned" before a Tune; what's running while it runs.
pub fn bar(r: &Readout, now: u64) -> String {
    match r.state {
        TuneState::Tuning => return "Tuning…".into(),
        TuneState::Checking => return "Checking…".into(),
        _ => {}
    }
    let Some(at) = r.tuned_at else { return "Not tuned".into() };
    let mut parts = vec![format!("Tuned {}", when(at, now))];
    if let Some(w) = r.worst_cents {
        parts.push(format!("worst note {:.0}¢", w.abs()));
    }
    if let Some((lo, hi)) = r.range {
        parts.push(format!("{}–{}", note(lo), note(hi)));
    }
    if !r.off.is_empty() {
        parts.push(format!("off: {}", r.off.iter().map(|&n| note(n)).collect::<Vec<_>>().join(" ")));
    }
    parts.join(" · ")
}

/// What the screens show of a Zhora: `r` its readout, `live` Live's knob, `hold_note`
/// its Hold Note, `input` what it listens on, `problem` an output not sent, `now` Unix
/// seconds.
pub fn view(r: &Readout, live: bool, hold_note: &str, input: String, problem: Option<String>, now: u64) -> ZhoraView {
    let cents = r.note.map(|n| n.1);
    let right = if r.holding {
        format!("Holding {hold_note}")
    } else if r.clamped {
        "Out of range".into()
    } else {
        match r.live_locked {
            Some(true) => "Live: locked on".into(),
            Some(false) => "Live: holding".into(),
            None => String::new(),
        }
    };
    ZhoraView {
        note: r.note.map(|(n, _)| note(n)),
        cents,
        in_tune: cents.is_some_and(|c| c.abs() <= IN_TUNE),
        status: bar(r, now),
        right,
        tune: r.state == TuneState::Tuning,
        check: r.state == TuneState::Checking,
        live,
        hold: r.holding,
        message: r.message.map(str::to_string).or(problem),
        input,
        env: Vec::new(),
        env_note_off: 0.0,
    }
}

/// Samples in the envelope's picture.
const ENV_POINTS: usize = 240;

/// The picture of an envelope whose knobs are at `[attack, decay, sustain, release]`
/// (0..1), as the LFO page draws one, and where it shows the note off.
pub fn env_picture([a, d, s, r]: [f64; 4]) -> (Vec<f32>, f32) {
    let (a, d, s, r) = (a as f32, d as f32, s as f32, r as f32);
    (deckard_model::lfo::adsr_picture(ENV_POINTS, a, d, s, r), deckard_model::lfo::adsr_note_off(a, d, r))
}

/// What the screens show of the Tuner effect: `r` its readout, `reference` its
/// Reference knob (0..1).
pub fn tuner_view(r: &Readout, reference: f64) -> TunerView {
    let cents = r.note.map(|n| n.1);
    TunerView {
        note: r.note.map(|(n, _)| note(n)),
        cents,
        in_tune: cents.is_some_and(|c| c.abs() <= IN_TUNE),
        reference: format!("{:.0} Hz", deckard_plugins::tuner::reference_hz(reference)),
        muted: r.holding,
    }
}

impl App {
    /// The focused slot, when it is the Tuner effect, on any chain: (chain, slot).
    pub(crate) fn focused_tuner(&self) -> Option<(Chain, usize)> {
        let chain = self.chain();
        let i = self.slot.filter(|_| !self.channel)?;
        let d = self.doc.as_ref()?.borrow();
        let tuner = d.chain_slots(chain)?.get(i)?.plugin.as_ref().is_some_and(|pr| deckard_plugins::tuner::is_tuner(&pr.id));
        tuner.then_some((chain, i))
    }

    /// What the screens show of the focused Tuner. Read without waiting on the audio
    /// thread: while it has the instance, the last view stays.
    pub(crate) fn tuner_view(&self) -> Option<TunerView> {
        let key = self.focused_tuner()?;
        let inst = self.samples.plugins.existing(self.plugin_slot_key(key.0, key.1))?;
        let mut shown = self.tuner_shown.borrow_mut();
        let Ok(t) = inst.try_lock() else {
            return shown.as_ref().filter(|(k, _)| *k == key).map(|(_, v)| v.clone());
        };
        let (r, reference) = (t.readout()?, t.param_value(deckard_plugins::tuner::REFERENCE));
        drop(t);
        let v = tuner_view(&r, reference);
        *shown = Some((key, v.clone()));
        Some(v)
    }

    /// The focused slot, when it is a Zhora playing a sound: (group, sound, slot).
    pub(crate) fn focused_zhora(&self) -> Option<(usize, usize, usize)> {
        let Chain::Sound(g, s) = self.chain() else { return None };
        let i = self.slot.filter(|_| !self.channel)?;
        let d = self.doc.as_ref()?.borrow();
        let zhora = d.chain_slots(Chain::Sound(g, s))?.get(i)?.plugin.as_ref().is_some_and(|pr| is_zhora(&pr.id));
        zhora.then_some((g, s, i))
    }

    /// What the screens show of the focused Zhora. Read without waiting on the audio
    /// thread: while it has the instance, the last view stays.
    pub(crate) fn zhora_view(&self, p: &Project) -> Option<ZhoraView> {
        let key = self.focused_zhora()?;
        let (g, s, i) = key;
        let inst = self.samples.plugins.existing(self.plugin_slot_key(Chain::Sound(g, s), i))?;
        let mut shown = self.zhora_shown.borrow_mut();
        let Ok(z) = inst.try_lock() else {
            return shown.as_ref().filter(|(k, _)| *k == key).map(|(_, v)| v.clone());
        };
        // Only the raw values under the lock (the audio thread waits for it).
        let r = z.readout()?;
        let (live, hold_note, input) = (z.param_value(LIVE) > 0.5, z.param_value(HOLD_NOTE), z.input_source());
        // Its envelope, only while the ENV page is shown.
        let on_env = deckard_plugins::zhora::CLASS.pages.get(self.plugin_page).is_some_and(|pg| pg.name == "ENV");
        let env = on_env.then(|| [ATTACK, DECAY, SUSTAIN, RELEASE].map(|k| z.param_value(k)));
        drop(z);
        let hold_note = crate::keys::note_name((hold_note * 127.0).round() as i64);
        let input = input_label(&self.settings, p, (g, s), input);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut v = view(&r, live, &hold_note, input, output_problem(p, &self.settings, (g, s)), now);
        if let Some(knobs) = env {
            (v.env, v.env_note_off) = env_picture(knobs);
        }
        *shown = Some((key, v.clone()));
        Some(v)
    }

    /// A press of the focused Zhora's `what` (`tune`, `check`, `live`, `hold`). Live is a
    /// setting, saved in the project; the others are actions, sent to the instance only.
    pub(crate) fn zhora_press(&mut self, what: &str) {
        let Some((g, s, i)) = self.focused_zhora() else { return };
        let Some(inst) = self.samples.plugins.existing(self.plugin_slot_key(Chain::Sound(g, s), i)) else { return };
        let Ok(mut z) = inst.lock() else { return };
        let writes = press(what, z.readout().is_some_and(|r| r.holding), z.param_value(LIVE) > 0.5);
        if what == "live" {
            drop(z);
            let acts = writes.into_iter().map(|(id, v)| crate::layout::Act::PluginParam(i, id, v)).collect();
            crate::layout::apply_acts(self, acts);
            return;
        }
        for (id, v) in writes {
            z.set_param(id, v);
        }
    }

    /// A few times a second, stores a Tune that finished (see [`App::store_finished_tunes`]),
    /// and keeps frames coming while one runs.
    pub(crate) fn watch_tunes(&mut self, ctx: &eframe::egui::Context) {
        let now = std::time::Instant::now();
        if self.tunes_watched.is_some_and(|t| now.duration_since(t) < TUNE_WATCH) {
            return;
        }
        self.tunes_watched = Some(now);
        if self.store_finished_tunes(|_| true, false) {
            ctx.request_repaint_after(TUNE_WATCH);
        }
    }

    /// A Zhora that finished a Tune has a new table: its state goes into the project at
    /// once, as its own undo step "Tune", so the project counts as changed and a later
    /// edit's undo keeps it. The instances `which` picks are looked at; with `wait` false
    /// none is waited for (one the audio thread has is looked at next time). True while a
    /// Zhora is tuning.
    pub(crate) fn store_finished_tunes(&mut self, which: impl Fn(&deckard_plugins::Slot) -> bool, wait: bool) -> bool {
        let Some(doc) = self.doc.clone() else { return false };
        let (mut stored, mut tuning) = (Vec::new(), false);
        for (slot, p) in self.samples.plugins.instances().filter(|(s, _)| which(s)) {
            let p = if wait { p.lock().ok() } else { p.try_lock().ok() };
            let Some(mut p) = p else { continue };
            if p.id.cid != deckard_plugins::zhora::CLASS.cid {
                continue;
            }
            tuning |= p.readout().is_some_and(|r| r.state == TuneState::Tuning);
            if !p.changed_by_itself() {
                continue;
            }
            p.take_dirty();
            let chunk = p.save_chunk();
            p.mark_saved(chunk.clone());
            stored.push((*slot, chunk));
        }
        for (slot, chunk) in stored {
            let r = doc.borrow_mut().outside_take(|d| d.transaction("Tune", |d| crate::store_chunk(d, slot, chunk)));
            if let Err(e) = r {
                self.status = format!("tuning not stored: {e}");
            }
        }
        tuning
    }

    /// The steps knob `id` of the focused Zhora offers, in order, when it is an output
    /// (None, then the channels it can take) or its Input (see [`input_steps`]); it is on
    /// step `current`.
    pub(crate) fn zhora_offered(&self, p: &Project, id: u32, current: usize) -> Option<Vec<usize>> {
        let (g, s, _) = self.focused_zhora()?;
        if id == INPUT {
            return Some(input_steps(&self.settings, current));
        }
        let signal = OUTS.iter().position(|&o| o == id)?;
        Some(std::iter::once(0).chain(offered(p, &self.settings, (g, s), signal).into_iter().map(|c| c + 1)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_model::mod_matrix::{CvRoute, ModSource};
    use deckard_model::{Group, PluginRef, SlotInfo, Sound};
    use deckard_plugins::native::TuneState;

    fn readout() -> Readout {
        Readout {
            note: None,
            state: TuneState::NotTuned,
            worst_cents: None,
            tuned_at: None,
            range: None,
            live_locked: None,
            clamped: false,
            message: None,
            off: Vec::new(),
            holding: false,
            analysed: 0,
        }
    }

    /// A sound played by Zhora, with `params` set.
    fn zhora(params: &[(u32, f64)]) -> Sound {
        let class = &deckard_plugins::zhora::CLASS;
        let id = deckard_plugins::state::PluginId::parse(&class.plugin_id(), class.name).unwrap();
        let mut p = deckard_plugins::Plugin::native(class, id, class.name, 48_000.0);
        for &(k, v) in params {
            p.set_param(k, v);
        }
        let pr = PluginRef { id: class.plugin_id(), name: class.name.into(), chunk: p.save_chunk(), ..Default::default() };
        Sound {
            plugin: Some(pr.clone()),
            slots: vec![SlotInfo { module: "PluginHost".into(), name: pr.name.clone(), active: true, plugin: Some(pr), params: vec![] }],
            ..Default::default()
        }
    }

    /// An output's value for channel `ch`.
    fn on(ch: usize) -> f64 {
        (ch as f64 + 1.0) / 32.0
    }

    fn project(sounds: Vec<Sound>) -> Project {
        Project { groups: vec![Group { sounds, ..Default::default() }], ..Default::default() }
    }

    fn cv_only(channels: &[usize]) -> Settings {
        let mut s = Settings::default();
        for &c in channels {
            s.output_cv_only[c] = true;
        }
        s
    }

    /// The Input picker offers None, Sound's Input and the connected channels, each by
    /// the name Audio Setup gives it; a channel no longer connected stays while it is on it.
    #[test]
    fn the_input_picker_names_connected_mono_channels() {
        let mut s = Settings::default();
        s.input_routing = vec![None; 32];
        (s.input_routing[0], s.input_routing[3]) = (Some(0), Some(3));
        s.input_aliases[3] = "Modular".into();
        assert_eq!(input_steps(&s, 0), [0, 1, 2, 5]);
        assert_eq!(input_steps(&s, 9), [0, 1, 2, 5, 9], "the one it is on");
        let names: Vec<String> = input_steps(&s, 0).into_iter().map(|n| choice_name(&s, INPUT, n).unwrap()).collect();
        assert_eq!(names, ["None", "Sound's Input", "In 1 L", "Modular"]);
        assert_eq!(choice_name(&s, OUTS[0], 3).as_deref(), Some("Out 2 L"));
        assert_eq!(choice_name(&s, HOLD_NOTE, 3), None);
    }

    /// The Tuner effect's screens: the note and cents (green within 5), A4's pitch from
    /// its Reference knob, and Muted while Mute is on.
    #[test]
    fn the_tuner_effect_shows_its_note_reference_and_mute() {
        let r = Readout { note: Some((40, -3.4)), state: TuneState::Idle, ..readout() };
        let v = tuner_view(&r, 0.5);
        assert_eq!((v.note.as_deref(), v.cents, v.in_tune), (Some(note(40).as_str()), Some(-3.4), true));
        assert_eq!((v.reference.as_str(), v.muted), ("440 Hz", false));
        let r = Readout { note: Some((40, 12.0)), holding: true, ..readout() };
        let v = tuner_view(&r, 12.0 / 20.0);
        assert_eq!((v.in_tune, v.reference.as_str(), v.muted), (false, "442 Hz", true));
        assert_eq!(tuner_view(&readout(), 0.5).note, None, "nothing heard");
        // Offered as an effect in "+", by its name.
        let fx: Vec<&str> = deckard_plugins::native::CLASSES.iter().filter(|c| !c.instrument).map(|c| c.name).collect();
        assert!(fx.contains(&"Tuner"), "{fx:?}");
    }

    /// The tuner names what it listens on; on Sound's Input, what the sound's input is.
    #[test]
    fn the_tuner_names_its_input() {
        let mut s = Settings::default();
        s.input_aliases[2] = "VCO".into();
        let mut p = project(vec![zhora(&[])]);
        let label = |p: &Project, i| input_label(&s, p, (0, 0), i);
        assert_eq!(label(&p, None), "No input");
        assert_eq!(label(&p, Some(InputSource::Channel(2))), "VCO");
        assert_eq!(label(&p, Some(InputSource::Sound)), "Sound's Input (none set)");
        p.groups[0].sounds[0].channel.audio_in = AudioSource::Ext(16 + 2);
        assert_eq!(label(&p, Some(InputSource::Sound)), "VCO");
        p.groups[0].sounds[0].channel.audio_in = AudioSource::Ext(0);
        assert_eq!(label(&p, Some(InputSource::Sound)), s.input_pair_name(0));
    }

    #[test]
    fn step_output_visits_only_cv_channels_and_none() {
        let channels = [0, 3, 4];
        let mut at = 0;
        let mut seen = vec![at];
        for _ in 0..3 {
            at = step_choice(at, 1, &channels);
            seen.push(at);
        }
        assert_eq!(seen, [0, 3, 4, 4], "None, channel 2, channel 3, then stops");
        assert_eq!(step_choice(4, -5, &channels), 0, "back to None");
        assert_eq!(step_choice(0, -1, &channels), 0);
        assert_eq!(step_choice(0, 1, &[0]), 0, "no channels: None");
    }

    #[test]
    fn the_tuner_view_goes_green_within_5_cents() {
        let mut r = readout();
        r.note = Some((69, 4.9));
        let v = view(&r, false, "A3", "In 1 L+R".into(), None, 0);
        assert_eq!((v.note.as_deref(), v.cents, v.in_tune), (Some("A3"), Some(4.9), true));
        r.note = Some((60, -5.5));
        let v = view(&r, false, "A3", "In 1 L+R".into(), None, 0);
        assert_eq!((v.note.as_deref(), v.in_tune), (Some("C3"), false), "60 is C3");
        r.note = None;
        assert!(!view(&r, false, "A3", String::new(), None, 0).in_tune, "nothing heard");
        assert_eq!(cents_text(2.4), "+2¢");
        assert_eq!(cents_text(-12.0), "\u{2212}12¢");
        assert_eq!(cents_text(0.2), "0¢");
    }

    #[test]
    fn the_bar_says_not_tuned_before_a_tune() {
        let mut r = readout();
        let now = 1_800_000_000;
        assert_eq!(bar(&r, now), "Not tuned");
        r.state = TuneState::Tuning;
        let v = view(&r, false, "A3", String::new(), None, now);
        assert_eq!((v.status.as_str(), v.tune, v.check), ("Tuning…", true, false));
        (r.state, r.tuned_at, r.worst_cents, r.range) = (TuneState::Idle, Some(now), Some(-3.2), Some((36, 108)));
        let b = bar(&r, now);
        assert!(b.starts_with("Tuned today ") && b.ends_with(" · worst note 3¢ · C1–C7"), "{b}");
        r.off = vec![108, 110];
        assert!(bar(&r, now).ends_with(" · off: C7 D7"), "{}", bar(&r, now));
        assert!(bar(&r, now + 86_400).starts_with("Tuned yesterday "));
        assert!(!bar(&r, now + 5 * 86_400).contains("day"), "a date: {}", bar(&r, now + 5 * 86_400));
        // A failed Tune's message, in place of the bar until the next press.
        r.message = Some("No signal on the input");
        let v = view(&r, false, "A3", String::new(), None, now);
        assert_eq!(v.message.as_deref(), Some("No signal on the input"));
        r.holding = true;
        assert_eq!(view(&r, false, "A3", String::new(), None, now).right, "Holding A3");
    }

    /// Display buttons 5 to 8 (and the panel's): Tune and Check pressed and let go,
    /// Live and Hold switched.
    #[test]
    fn zhora_display_buttons_5_to_8_are_tune_check_live_hold() {
        assert_eq!(press("tune", false, false), [(41, 1.0), (41, 0.0)]);
        assert_eq!(press("check", false, false), [(42, 1.0), (42, 0.0)]);
        assert_eq!(press("live", false, false), [(32, 1.0)]);
        assert_eq!(press("live", false, true), [(32, 0.0)]);
        assert_eq!(press("hold", false, false).last(), Some(&(40, 1.0)), "a press, whatever was saved");
        assert_eq!(press("hold", true, false), [(40, 0.0)]);
    }

    #[test]
    fn an_output_on_an_unmarked_channel_is_flagged() {
        let mut p = project(vec![zhora(&[(1, on(4)), (2, on(5))])]);
        let mut settings = cv_only(&[4]);
        assert_eq!(output_problem(&p, &settings, (0, 0)).as_deref(), Some("Gate isn't sent: Out 3 R isn't CV only"));
        settings.output_cv_only[5] = true;
        assert_eq!(output_problem(&p, &settings, (0, 0)), None, "marked: it's sent");
        // A CV route's channel, or an earlier Zhora's, isn't its.
        p.cv_routes = vec![CvRoute::new(ModSource::Lfo(1), 4)];
        assert_eq!(output_problem(&p, &settings, (0, 0)).as_deref(), Some("Pitch isn't sent: a CV route has Out 3 L"));
        p.cv_routes.clear();
        p.groups[0].sounds.push(zhora(&[(3, on(5))]));
        assert_eq!(output_problem(&p, &settings, (0, 1)).as_deref(), Some("Clock isn't sent: another Zhora has Out 3 R"));
        assert_eq!(output_problem(&p, &settings, (0, 0)), None, "the first has it");
    }

    /// An output offers the CV only channels no route, other Zhora or other output of its
    /// own has; its own stays.
    #[test]
    fn an_output_offers_only_free_cv_channels() {
        let mut p = project(vec![zhora(&[(1, on(4)), (2, on(5))]), zhora(&[(1, on(6))])]);
        p.cv_routes = vec![CvRoute::new(ModSource::Lfo(1), 7)];
        let settings = cv_only(&[4, 5, 6, 7, 8]);
        assert_eq!(offered(&p, &settings, (0, 0), 0), [4, 8]);
        assert_eq!(offered(&p, &settings, (0, 0), 3), [8], "a None output");
        assert_eq!(offered(&p, &settings, (0, 1), 0), [6, 8]);
        assert!(zhora_in(&p) && !zhora_in(&project(vec![Sound::default()])));
    }

    /// A new project with Zhora on sound 1, focused, its instance made.
    fn app_with_zhora() -> App {
        let mut app = App { settings: Settings::default(), ..App::default() };
        app.new_project();
        (app.group, app.sound, app.chain_tab, app.channel) = (0, 0, crate::layout::ChainTab::Sound, false);
        app.add_slot(crate::layout::AddWhat::Plugin(crate::layout::native_entry(&deckard_plugins::zhora::CLASS)));
        app.slot = Some(0);
        app.slot_plugin_in(Chain::Sound(0, 0), 0).unwrap();
        app
    }

    /// Its envelope's picture is sent on the ENV page only, and follows Attack.
    #[test]
    fn the_envelope_is_pictured_on_the_env_page_only() {
        let mut app = app_with_zhora();
        let env = |app: &App| app.surface_view().control.zhora.unwrap().env;
        assert!(env(&app).is_empty(), "OUTS");
        crate::layout::apply_acts(&mut app, vec![crate::layout::Act::PluginPage(1)]);
        let before = env(&app);
        assert_eq!(before.len(), ENV_POINTS);
        set(&mut app, ATTACK, 0.9);
        let after = env(&app);
        let peak = |p: &[f32]| p.iter().position(|&v| v > 0.99).unwrap();
        assert!(peak(&after) > peak(&before), "a longer attack peaks later");
        crate::layout::apply_acts(&mut app, vec![crate::layout::Act::PluginPage(2)]);
        assert!(env(&app).is_empty(), "LEVELS");
    }

    /// The tuner reads the instance for the Control page only.
    #[test]
    fn the_zhora_tuner_is_built_on_the_control_page_only() {
        let mut app = app_with_zhora();
        assert_eq!(app.surface.page(), "control");
        assert!(app.surface_view().control.zhora.is_some());
        let view = app.surface_view();
        for a in app.surface.restore("mixer", "keyboard", &view, std::time::Instant::now()) {
            app.surface_act(a);
        }
        assert_eq!(app.surface.page(), "mixer");
        assert!(app.surface_view().control.zhora.is_none());
    }

    /// The computer's output selector has the step (as the controller's knob does) and
    /// shows it at its place among None and the channels offered.
    #[test]
    fn an_output_selector_finds_its_step_in_the_list() {
        let values = [0.0, 5.0 / 32.0, 9.0 / 32.0];
        assert_eq!(crate::layout::listed_at(&values, 32, 9), 2);
        assert_eq!(crate::layout::listed_at(&values, 32, 5), 1);
        assert_eq!(crate::layout::listed_at(&values, 32, 0), 0);
        assert_eq!(crate::layout::listed_at(&values, 32, 7), 0, "not listed: the first");
    }

    /// Runs a Tune on sound 1's Zhora (input and pitch output set), the instance fed a
    /// simulated voice speaking within `reach` of the centre's output value, until it
    /// ends; `each_hop` runs after every hop. The readout when it ended.
    fn tune_in(app: &mut App, reach: f64, mut each_hop: impl FnMut(&mut App, u32)) -> Readout {
        use std::time::{Duration, Instant};
        let rate = app.output_rate();
        let chain = Chain::Sound(0, 0);
        let inst = app.samples.plugins.existing(app.plugin_slot_key(chain, 0)).unwrap();
        // Blocks to a detection: the detector reads 60 kHz or under.
        let (mut per, mut r) = (1u32, rate);
        while r > 60_000.0 {
            (per, r) = (per * 2, r / 2.0);
        }
        let t = deckard_plugins::Transport { tempo: 120.0, playing: false, quarters: 0.0, numerator: 4, denominator: 4 };
        inst.lock().unwrap().process(1024, &t);
        // Detections the instance made before this Tune.
        let before = inst.lock().unwrap().readout().unwrap().analysed;
        app.zhora_press("tune");
        let deadline = Instant::now() + Duration::from_secs(240);
        let (mut hops, mut phase) = (0u32, 0f64);
        loop {
            {
                let mut z = inst.lock().unwrap();
                z.process(1024, &t);
                if z.readout().unwrap().state != TuneState::Tuning {
                    return z.readout().unwrap();
                }
                let v = z.cv(0, 1024).unwrap().1[1023] as f64;
                let hz = (v.abs() <= reach + 1e-4).then(|| 261.63 * 2f64.powf(v * 10.0));
                let mut b = [0f32; 1024];
                if let Some(hz) = hz {
                    for s in &mut b {
                        *s = (0.5 * (phase * std::f64::consts::TAU).sin()) as f32;
                        phase = (phase + hz / rate).fract();
                    }
                }
                z.input(&b, &b);
            }
            hops += 1;
            // Each hop analysed before the next, as the voice would answer in time.
            while inst.lock().unwrap().readout().unwrap().analysed != before.wrapping_add(hops / per) {
                std::thread::sleep(Duration::from_millis(1));
                assert!(Instant::now() < deadline, "the analysis fell silent");
            }
            each_hop(app, hops);
            assert!(Instant::now() < deadline, "the tune never finished");
        }
    }

    /// Sound 1's Zhora, listening on In 1 L with its pitch on Out 1 L, stored.
    fn listening_zhora() -> App {
        use deckard_plugins::zhora::{INPUT, PITCH_OUT};
        let mut app = app_with_zhora();
        set(&mut app, INPUT, 2.0 / 33.0);
        set(&mut app, PITCH_OUT, 1.0 / 32.0);
        app.store_plugin_states();
        app
    }

    fn set(app: &mut App, id: u32, v: f64) {
        crate::layout::apply_acts(app, vec![crate::layout::Act::PluginParamIn(Chain::Sound(0, 0), 0, id, v)]);
    }

    /// What sound 1's Zhora plays from, as the project has it now (an undo reloads it).
    fn tuned_in_project(app: &mut App) -> Readout {
        let z = app.slot_plugin_in(Chain::Sound(0, 0), 0).unwrap();
        z.lock().unwrap().readout().unwrap()
    }

    /// A finished Tune is in the project as soon as the screens next look: quitting asks
    /// about it, and it is its own undo step.
    #[test]
    fn a_finished_tune_is_stored_at_once() {
        let mut app = listening_zhora();
        let file = std::env::temp_dir().join(format!("zhora-tune-{}.mxprj", std::process::id()));
        app.doc.as_ref().unwrap().borrow_mut().save_as(&file).unwrap();
        let _ = std::fs::remove_file(&file);
        assert!(!app.asks_on_quit(), "saved");
        let r = tune_in(&mut app, 0.2, |_, _| {});
        assert_eq!((r.state, r.message), (TuneState::Idle, None));
        app.watch_tunes(&eframe::egui::Context::default());
        assert!(app.asks_on_quit(), "quitting would lose the tuning");
        assert_eq!(app.doc.as_ref().unwrap().borrow().undo_label(), Some("Tune"));
        assert_eq!(tuned_in_project(&mut app).tuned_at, r.tuned_at);
    }

    /// A knob turned after a Tune, and undone, leaves the new table playing.
    #[test]
    fn undoing_a_knob_after_a_tune_keeps_the_new_table() {
        use deckard_plugins::zhora::HOLD_NOTE;
        let mut app = listening_zhora();
        let r = tune_in(&mut app, 0.2, |_, _| {});
        app.watch_tunes(&eframe::egui::Context::default());
        set(&mut app, HOLD_NOTE, 60.0 / 127.0);
        app.undo();
        let now = tuned_in_project(&mut app);
        assert!(now.tuned_at.is_some() && now.tuned_at == r.tuned_at, "{:?}", now.tuned_at);
        assert_eq!(now.range, r.range);
    }

    /// A knob turned the moment a Tune ends, before the screens look, and undone: the
    /// new table still plays, and the Tune is its own step.
    #[test]
    fn a_knob_right_after_a_tune_keeps_the_new_table() {
        use deckard_plugins::zhora::HOLD_NOTE;
        let mut app = listening_zhora();
        let r = tune_in(&mut app, 0.2, |_, _| {});
        set(&mut app, HOLD_NOTE, 60.0 / 127.0);
        app.undo();
        let now = tuned_in_project(&mut app);
        assert!(now.tuned_at.is_some() && now.tuned_at == r.tuned_at, "{:?}", now.tuned_at);
        assert_eq!(app.doc.as_ref().unwrap().borrow().undo_label(), Some("Tune"));
    }

    /// Undoing a Tune brings the table before it back.
    #[test]
    fn undoing_a_tune_brings_the_old_table_back() {
        let mut app = listening_zhora();
        // Stored directly: `watch_tunes` skips a look soon after the last, and a fast
        // machine finishes the second Tune inside that gap.
        let old = tune_in(&mut app, 0.2, |_, _| {});
        app.store_finished_tunes(|_| true, true);
        // A voice with a narrower range makes a table that tells apart.
        let new = tune_in(&mut app, 0.1, |_, _| {});
        app.store_finished_tunes(|_| true, true);
        assert_ne!(new.range, old.range);
        assert_eq!(tuned_in_project(&mut app).range, new.range);
        app.undo();
        assert_eq!(tuned_in_project(&mut app).range, old.range, "the old table is back");
    }

    /// A Tune that finishes after a knob stored the state mid-Tune: the next save has
    /// the new table, and the project reopened plays it.
    #[test]
    fn a_finished_tune_reaches_the_saved_project() {
        use deckard_plugins::zhora::HOLD_NOTE;
        let mut app = listening_zhora();
        let chain = Chain::Sound(0, 0);
        // A knob mid-Tune (one the voice doesn't hear) stores the state then.
        let r = tune_in(&mut app, 0.2, |app, hops| {
            if hops == 20 {
                set(app, HOLD_NOTE, 60.0 / 127.0);
            }
        });
        assert_eq!((r.state, r.message), (TuneState::Idle, None));
        assert!(r.tuned_at.is_some());
        app.store_plugin_states();
        let bytes = app.doc.as_ref().unwrap().borrow().to_bytes().unwrap();
        let mut again = App { settings: Settings::default(), ..App::default() };
        again.show_document(deckard_doc::Document::from_bytes(bytes, None));
        let z = again.slot_plugin_in(chain, 0).unwrap();
        let back = z.lock().unwrap().readout().unwrap();
        assert_eq!(back.tuned_at, r.tuned_at, "the table is in the saved project");
        assert_eq!(back.worst_cents, r.worst_cents);
    }
}
