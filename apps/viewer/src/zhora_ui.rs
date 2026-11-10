//! Zhora's screens: what the MK3 and the plug-in panel show of it (its tuner, buttons
//! and bar), the channels its outputs can take, and whether a project has one.

use deckard_doc::Chain;
use deckard_engine::zhora_outs;
use deckard_model::Project;
use deckard_plugins::native::{Readout, TuneState};
use deckard_plugins::zhora::{CHECK, HOLD, HOLD_NOTE, LIVE, OUTS, SIGNAL_NAMES, TUNE, is_zhora};
use deckard_surface::ZhoraView;

use crate::App;
use crate::settings::Settings;

/// A note within this many cents is in tune.
const IN_TUNE: f32 = 5.0;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Whether a sound of `p` is played by Zhora.
pub fn zhora_in(p: &Project) -> bool {
    p.groups.iter().flat_map(|g| &g.sounds).any(|s| s.plugin.as_ref().is_some_and(|pr| is_zhora(&pr.id)))
}

/// An output's step (0 None, n channel n - 1) moved `step` places along None and then
/// `channels`, stopping at either end.
pub fn step_output(current: usize, step: i64, channels: &[usize]) -> usize {
    let list: Vec<usize> = std::iter::once(0).chain(channels.iter().map(|c| c + 1)).collect();
    let at = list.iter().position(|&v| v == current).unwrap_or(0) as i64;
    list[(at + step).clamp(0, list.len() as i64 - 1) as usize]
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
    }
}

impl App {
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
        let (live, hold_note, input) = (z.param_value(LIVE) > 0.5, z.param_value(HOLD_NOTE), z.input_pair());
        drop(z);
        let hold_note = crate::keys::note_name((hold_note * 127.0).round() as i64);
        let input = input.map_or_else(|| "No input".into(), |n| self.settings.input_pair_name(n));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let v = view(&r, live, &hold_note, input, output_problem(p, &self.settings, (g, s)), now);
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

    /// The channels output `id` of the focused Zhora can take, when knob `id` is one.
    pub(crate) fn zhora_offered(&self, p: &Project, id: u32) -> Option<Vec<usize>> {
        let (g, s, _) = self.focused_zhora()?;
        let signal = OUTS.iter().position(|&o| o == id)?;
        Some(offered(p, &self.settings, (g, s), signal))
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

    #[test]
    fn step_output_visits_only_cv_channels_and_none() {
        let channels = [2, 3];
        let mut at = 0;
        let mut seen = vec![at];
        for _ in 0..3 {
            at = step_output(at, 1, &channels);
            seen.push(at);
        }
        assert_eq!(seen, [0, 3, 4, 4], "None, channel 2, channel 3, then stops");
        assert_eq!(step_output(4, -5, &channels), 0, "back to None");
        assert_eq!(step_output(0, -1, &channels), 0);
        assert_eq!(step_output(0, 1, &[]), 0, "no channels: None");
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

    /// A Tune that finishes after a knob stored the state mid-Tune: the next save has
    /// the new table, and the project reopened plays it.
    #[test]
    fn a_finished_tune_reaches_the_saved_project() {
        use deckard_plugins::zhora::{HOLD_NOTE, INPUT, PITCH_OUT};
        use std::time::{Duration, Instant};
        let mut app = app_with_zhora();
        let rate = app.output_rate();
        let chain = Chain::Sound(0, 0);
        let set = |app: &mut App, id, v| crate::layout::apply_acts(app, vec![crate::layout::Act::PluginParamIn(chain, 0, id, v)]);
        set(&mut app, INPUT, 1.0 / 16.0);
        set(&mut app, PITCH_OUT, 1.0 / 32.0);
        app.store_plugin_states();
        let inst = app.samples.plugins.existing(app.plugin_slot_key(chain, 0)).unwrap();
        let t = deckard_plugins::Transport { tempo: 120.0, playing: false, quarters: 0.0, numerator: 4, denominator: 4 };
        inst.lock().unwrap().process(1024, &t);
        app.zhora_press("tune");
        let deadline = Instant::now() + Duration::from_secs(240);
        let (mut hops, mut phase) = (0u32, 0f64);
        loop {
            {
                let mut z = inst.lock().unwrap();
                z.process(1024, &t);
                if z.readout().unwrap().state != TuneState::Tuning {
                    break;
                }
                let v = z.cv(0, 1024).unwrap().1[1023] as f64;
                // A voice over two octaves either side of C3, silent past that.
                let hz = (v.abs() <= 0.2 + 1e-4).then(|| 261.63 * 2f64.powf(v * 10.0));
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
            while inst.lock().unwrap().readout().unwrap().analysed != hops {
                std::thread::sleep(Duration::from_millis(1));
                assert!(Instant::now() < deadline, "the analysis fell silent");
            }
            // A knob mid-Tune (one the voice doesn't hear) stores the state then.
            if hops == 20 {
                set(&mut app, HOLD_NOTE, 60.0 / 127.0);
            }
            assert!(Instant::now() < deadline, "the tune never finished");
        }
        let r = inst.lock().unwrap().readout().unwrap();
        assert_eq!((r.state, r.message.as_deref()), (TuneState::Idle, None));
        assert!(r.tuned_at.is_some() && hops > 20, "{hops}");
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
