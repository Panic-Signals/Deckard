//! MIDI ports: incoming notes and controllers play the channels set to take them
//! (sent to the playing engine from the port's thread, stamped with when they came,
//! so they keep their timing), incoming MIDI clock goes to the clock follower, and a
//! thread sends what sounds put out (and MIDI clock, when it's due).

use deckard_engine::Live;
use deckard_engine::midi::{InMsg, MidiOut, MidiTable};
use deckard_engine::output::Sender;
use deckard_engine::sync::{ClockFollower, SyncMsg};
use deckard_model::{Dim, ExprKind, MidiKind};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A note for the app to play because no engine was running: (group, sound, pitch,
/// velocity; 0 = release).
pub type Pending = (usize, usize, i64, i64);

/// MIDI Lock Change: the input and channel (1–16) Lock listens to, and how (1 notes, 2
/// program changes; 0 off).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockMidi {
    pub source: String,
    pub channel: u8,
    pub trigger: usize,
}

/// What a message means to Lock: `None` not Lock's; `Some(Some(slot))` recall the
/// snapshot in `slot`; `Some(None)` Lock's, nothing to do (a note-off, past slot 64).
pub fn lock_trigger(l: &LockMidi, port: &str, msg: &InMsg) -> Option<Option<usize>> {
    if l.trigger == 0 || l.source == "None" || (l.source != "All" && l.source != port) {
        return None;
    }
    let slot = |n: u8| (n < 64).then_some(n as usize);
    match (l.trigger, msg) {
        (1, InMsg::NoteOn { ch, note, .. }) if *ch == l.channel => Some(slot(*note)),
        (1, InMsg::NoteOff { ch, .. }) if *ch == l.channel => Some(None),
        (2, InMsg::Program { ch, program }) if *ch == l.channel => Some(slot(*program)),
        _ => None,
    }
}

/// What a message means to remote control (MIDI learn, mapped controls): its remote key
/// and value (0..1) when a parameter waits for a control (`learning`) or the key is
/// mapped to one; `None` for the rest, which play as usual.
pub fn remote_take(learning: bool, mapped: &std::collections::HashSet<u64>, bytes: &[u8]) -> Option<(u64, f64)> {
    let (key, v) = deckard_doc::remote::midi_key(bytes)?;
    (learning || mapped.contains(&key)).then_some((key, v))
}

/// Remote control's side of the inputs: whether a parameter waits for a control, the
/// keys mapped to parameters, and what came for them.
#[derive(Default)]
struct RemoteIn {
    learning: bool,
    mapped: std::collections::HashSet<u64>,
    came: Vec<(u64, f64)>,
}

pub struct MidiIo {
    table: Arc<Mutex<MidiTable>>,
    /// Where notes go: the audio output's command queue.
    output: Arc<Mutex<Option<Sender>>>,
    inputs: Vec<(String, midir::MidiInputConnection<()>)>,
    pending: Arc<Mutex<Vec<Pending>>>,
    /// MIDI Lock Change's setting, and the slots it was asked to recall.
    lock: Arc<Mutex<Option<LockMidi>>>,
    lock_pending: Arc<Mutex<Vec<usize>>>,
    remote: Arc<Mutex<RemoteIn>>,
    tx: SyncSender<MidiOut>,
    /// Messages in and out since start (for Preferences).
    count: Arc<std::sync::atomic::AtomicU64>,
    sent: Arc<std::sync::atomic::AtomicU64>,
    repaint: Arc<Mutex<Option<eframe::egui::Context>>>,
    /// Where incoming MIDI clock goes (Preferences > MIDI > Sync: Receive Clock).
    clock: Arc<Mutex<Option<Arc<ClockFollower>>>>,
    /// The MPE inputs and their member bend range (Preferences > MIDI).
    mpe: Arc<Mutex<HashMap<String, u8>>>,
    /// The MPE outputs (and their member bend range), and whether to set them up.
    mpe_out: Arc<Mutex<(bool, std::collections::BTreeMap<String, u8>)>>,
    /// The MIDI Monitor: what comes in and goes out (while its window is open).
    monitor: crate::midi_monitor::Monitor,
}

impl Default for MidiIo {
    fn default() -> MidiIo {
        let (tx, rx) = std::sync::mpsc::sync_channel::<MidiOut>(1024);
        let sent = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let s2 = sent.clone();
        let monitor = crate::midi_monitor::Monitor::default();
        let m2 = monitor.clone();
        let mpe_out: Arc<Mutex<(bool, std::collections::BTreeMap<String, u8>)>> = Arc::default();
        let o2 = mpe_out.clone();
        std::thread::Builder::new().name("DECKARD MIDI out".into()).spawn(move || send_loop(rx, s2, m2, o2)).ok();
        MidiIo {
            table: Arc::default(),
            output: Arc::default(),
            inputs: Vec::new(),
            pending: Arc::default(),
            lock: Arc::default(),
            lock_pending: Arc::default(),
            remote: Arc::default(),
            tx,
            count: Arc::default(),
            sent,
            repaint: Arc::default(),
            clock: Arc::default(),
            mpe: Arc::default(),
            mpe_out,
            monitor,
        }
    }
}

/// Send each message as it comes (one with a time, when it's due), connecting to
/// ports by name when first used.
fn send_loop(
    rx: Receiver<MidiOut>,
    sent: Arc<std::sync::atomic::AtomicU64>,
    monitor: crate::midi_monitor::Monitor,
    mpe_out: Arc<Mutex<(bool, std::collections::BTreeMap<String, u8>)>>,
) {
    let mut ports: HashMap<String, Option<midir::MidiOutputConnection>> = HashMap::new();
    let mut send = |m: &MidiOut| {
        let conn = ports.entry(m.port.to_string()).or_insert_with(|| {
            let mut c = connect_out(&m.port);
            // An MPE synth is set up as soon as it's connected.
            let setup = mpe_out.lock().ok().and_then(|o| o.0.then(|| o.1.get(&*m.port).copied()).flatten());
            if let (Some(c), Some(range)) = (&mut c, setup) {
                for b in deckard_engine::midi::mpe_setup(range) {
                    if c.send(&b).is_ok() {
                        monitor.midi(crate::midi_monitor::Dir::Out, &m.port, &b, None);
                    }
                }
            }
            c
        });
        if let Some(c) = conn
            && c.send(m.bytes()).is_ok()
        {
            sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            monitor.midi(crate::midi_monitor::Dir::Out, &m.port, m.bytes(), None);
        }
    };
    // Messages waiting for their time, in the order they came (which is theirs).
    let mut waiting: std::collections::VecDeque<MidiOut> = std::collections::VecDeque::new();
    loop {
        let now = Instant::now();
        while waiting.front().is_some_and(|m| m.at.is_none_or(|t| t <= now)) {
            if let Some(m) = waiting.pop_front() {
                send(&m);
            }
        }
        let next = match waiting.front().and_then(|m| m.at) {
            Some(due) => rx.recv_timeout(due.saturating_duration_since(now)),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(m) if m.at.is_some_and(|t| t > Instant::now()) => waiting.push_back(m),
            Ok(m) => send(&m),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// MPE output ports to set up after a change: those newly MPE or with a new range.
fn setup_due(old: &std::collections::BTreeMap<String, u8>, new: &std::collections::BTreeMap<String, u8>) -> Vec<(String, u8)> {
    new.iter().filter(|(k, v)| old.get(*k) != Some(v)).map(|(k, v)| (k.clone(), *v)).collect()
}

fn connect_out(name: &str) -> Option<midir::MidiOutputConnection> {
    let out = midir::MidiOutput::new("DECKARD").ok()?;
    let port = out.ports().into_iter().find(|p| out.port_name(p).is_ok_and(|n| n == name))?;
    out.connect(&port, "DECKARD out").ok()
}

impl MidiIo {
    /// The MIDI Monitor's log (shared with the MK3's pads).
    pub fn monitor(&self) -> crate::midi_monitor::Monitor {
        self.monitor.clone()
    }

    /// Where engines send MIDI for output ports.
    pub fn sender(&self) -> SyncSender<MidiOut> {
        self.tx.clone()
    }

    /// The output notes go to.
    pub fn set_output(&self, output: Sender) {
        *self.output.lock().unwrap() = Some(output);
    }

    pub fn set_context(&self, ctx: &eframe::egui::Context) {
        let mut r = self.repaint.lock().unwrap();
        if r.is_none() {
            *r = Some(ctx.clone());
        }
    }

    /// These inputs are MPE controllers (with their member bend range).
    pub fn set_mpe(&self, ports: &std::collections::BTreeMap<String, u8>) {
        let mut m = self.mpe.lock().unwrap();
        if m.len() != ports.len() || ports.iter().any(|(k, v)| m.get(k) != Some(v)) {
            *m = ports.iter().map(|(k, v)| (k.clone(), *v)).collect();
        }
    }

    /// These outputs are MPE synths (with their member bend range); `send_setup`: set
    /// them up (on connecting, and here for those new or changed, or all when it's
    /// switched on).
    pub fn set_mpe_out(&self, ports: &std::collections::BTreeMap<String, u8>, send_setup: bool) {
        let mut o = self.mpe_out.lock().unwrap();
        if o.0 == send_setup && o.1 == *ports {
            return;
        }
        let due = match (send_setup, o.0) {
            (false, _) => Vec::new(),
            (true, false) => ports.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            (true, true) => setup_due(&o.1, ports),
        };
        *o = (send_setup, ports.clone());
        drop(o);
        for (port, range) in due {
            let port: Arc<str> = port.into();
            for bytes in deckard_engine::midi::mpe_setup(range) {
                let _ = self.tx.try_send(MidiOut { port: port.clone(), bytes, len: 3, at: None });
            }
        }
    }

    /// Use this routing (rebuilt when the project or the focus changes).
    pub fn set_table(&self, t: MidiTable) {
        *self.table.lock().unwrap() = t;
    }

    /// Listen to exactly these input ports.
    pub fn listen(&mut self, ports: &[String]) {
        self.inputs.retain(|(n, _)| ports.contains(n));
        for name in ports {
            if self.inputs.iter().any(|(n, _)| n == name) {
                continue;
            }
            let Ok(mut input) = midir::MidiInput::new("DECKARD") else { continue };
            // MIDI clock and the song position come through too.
            input.ignore(midir::Ignore::None);
            let Some(port) = input.ports().into_iter().find(|p| input.port_name(p).is_ok_and(|n| &n == name)) else { continue };
            let (table, output, pending, count, repaint) =
                (self.table.clone(), self.output.clone(), self.pending.clone(), self.count.clone(), self.repaint.clone());
            let (lock, lock_pending, clock, remote) =
                (self.lock.clone(), self.lock_pending.clone(), self.clock.clone(), self.remote.clone());
            let (mpe, monitor) = (self.mpe.clone(), self.monitor.clone());
            let mut st = PortState::default();
            let port_name: Arc<str> = name.as_str().into();
            let conn = input.connect(
                &port,
                "DECKARD in",
                move |_, bytes, _| {
                    let at = Instant::now();
                    count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if monitor.is_open() {
                        let range = mpe.lock().unwrap().get(&*port_name).copied();
                        monitor.midi(crate::midi_monitor::Dir::In, &port_name, bytes, range);
                    }
                    if let Some(sync) = SyncMsg::parse(bytes) {
                        if let Some(f) = clock.lock().unwrap().as_ref() {
                            f.message(sync, at);
                            if sync != SyncMsg::Clock
                                && let Some(ctx) = repaint.lock().unwrap().as_ref()
                            {
                                ctx.request_repaint();
                            }
                        }
                        return;
                    }
                    let Some(msg) = InMsg::parse(bytes) else { return };
                    // Lock's MIDI change first: its messages go no further.
                    let taken = lock.lock().unwrap().as_ref().and_then(|l| lock_trigger(l, &port_name, &msg));
                    if let Some(recall) = taken {
                        if let Some(slot) = recall {
                            lock_pending.lock().unwrap().push(slot);
                            if let Some(ctx) = repaint.lock().unwrap().as_ref() {
                                ctx.request_repaint();
                            }
                        }
                        return;
                    }
                    // Then remote control: a control being learned or mapped to a
                    // parameter goes no further.
                    {
                        let mut r = remote.lock().unwrap();
                        if let Some(came) = remote_take(r.learning, &r.mapped, bytes) {
                            r.came.push(came);
                            if let Some(ctx) = repaint.lock().unwrap().as_ref() {
                                ctx.request_repaint();
                            }
                            return;
                        }
                    }
                    // An MPE input or not, as Preferences says now.
                    let range = mpe.lock().unwrap().get(&*port_name).copied();
                    match (range, st.mpe.as_mut()) {
                        (Some(r), Some(z)) => z.bend_range = r as f32,
                        (Some(r), None) => st.mpe = Some(Mpe::new(r)),
                        (None, Some(_)) => st.mpe = None,
                        (None, None) => {}
                    }
                    handle(&table, &output, &pending, &repaint, &port_name, &mut st, msg);
                },
                (),
            );
            if let Ok(c) = conn {
                self.inputs.push((name.clone(), c));
            }
        }
    }

    /// Where incoming MIDI clock goes (`None`: it's let go).
    pub fn set_clock_follower(&self, f: Option<Arc<ClockFollower>>) {
        *self.clock.lock().unwrap() = f;
    }

    /// MIDI Lock Change's setting (`None`: off).
    pub fn set_lock(&self, l: Option<LockMidi>) {
        *self.lock.lock().unwrap() = l;
    }

    /// Remote control's setting: a parameter waits for a control (MIDI learn), and the
    /// keys mapped to parameters.
    pub fn set_remote(&self, learning: bool, mapped: std::collections::HashSet<u64>) {
        let mut r = self.remote.lock().unwrap();
        r.learning = learning;
        r.mapped = mapped;
    }

    /// What came for remote control since last asked, (key, value): each key's last
    /// value, in the order the keys came.
    pub fn take_remote(&self) -> Vec<(u64, f64)> {
        let came = std::mem::take(&mut self.remote.lock().unwrap().came);
        let mut out: Vec<(u64, f64)> = Vec::new();
        for (k, v) in came {
            match out.iter_mut().find(|x| x.0 == k) {
                Some(x) => x.1 = v,
                None => out.push((k, v)),
            }
        }
        out
    }

    /// The snapshot slots MIDI asked to recall.
    pub fn take_lock(&self) -> Vec<usize> {
        std::mem::take(&mut *self.lock_pending.lock().unwrap())
    }

    /// Notes that came while no engine ran, for the app to play.
    pub fn take_pending(&self) -> Vec<Pending> {
        std::mem::take(&mut *self.pending.lock().unwrap())
    }

    pub fn status(&self) -> String {
        let n = |a: &std::sync::atomic::AtomicU64| a.load(std::sync::atomic::Ordering::Relaxed);
        format!("Listening to {} input(s); {} message(s) in, {} out", self.inputs.len(), n(&self.count), n(&self.sent))
    }
}

/// An MPE input's lower zone: channel 1 the manager, 2–16 members, each member note
/// on a channel of its own (Preferences ▸ MIDI ▸ MPE).
#[derive(Debug, Clone)]
pub(crate) struct Mpe {
    /// A member's pitch bend range, ± semitones.
    bend_range: f32,
    /// What each member channel (index: channel − 1) plays now: its note and the notes
    /// it reached.
    members: [Option<(u8, Vec<deckard_engine::midi::Hit>)>; 16],
    /// Expression a member channel sent before its note: given to the note when it
    /// comes (Pitch, Slide, Pressure).
    initial: [[Option<f32>; 3]; 16],
}

impl Mpe {
    pub(crate) fn new(bend_range: u8) -> Mpe {
        Mpe { bend_range: bend_range as f32, members: Default::default(), initial: [[None; 3]; 16] }
    }
}

/// What an input keeps between messages: its MPE zone (if it's an MPE input) and, for
/// poly pressure, the notes held on it.
#[derive(Debug, Clone, Default)]
pub(crate) struct PortState {
    pub(crate) mpe: Option<Mpe>,
    held: Vec<(u8, u8, Vec<deckard_engine::midi::Hit>)>,
}

/// What an incoming message plays: notes, controllers (`record` on the sound that
/// records them: the focused one if it takes them, else those set to that channel), and
/// per-note expression for held notes. Expression for a note not held goes nowhere.
pub(crate) fn route(t: &MidiTable, st: &mut PortState, port: &str, msg: InMsg) -> Vec<Live> {
    let control = |ch: u8, kind: MidiKind| -> Vec<Live> {
        let hits = t.route_control(port, ch);
        let focused = hits.contains(&t.focus);
        hits.iter()
            .map(|&(g, s)| {
                let record = if focused { (g, s) == t.focus } else { t.own_input(g, s, port, ch) };
                Live::ControlIn { group: g, sound: s, kind, thru: t.thru(g, s), record }
            })
            .collect()
    };
    let express = |hits: &[deckard_engine::midi::Hit], kind: ExprKind, value: f32| -> Vec<Live> {
        hits.iter().map(|&(group, sound, pitch)| Live::Expression { group, sound, pitch, kind, value }).collect()
    };
    let on = |hits: &[deckard_engine::midi::Hit], velocity: u8| -> Vec<Live> {
        hits.iter()
            .map(|&(g, s, pitch)| Live::NoteIn { group: g, sound: s, pitch, velocity: velocity as i64, thru: t.thru(g, s) })
            .collect()
    };
    let off = |hits: &[deckard_engine::midi::Hit], velocity: u8| -> Vec<Live> {
        let mut out = if velocity > 0 { express(hits, ExprKind::Release, velocity as f32 / 127.0) } else { Vec::new() };
        out.extend(hits.iter().map(|&(group, sound, pitch)| Live::NoteRelease { group, sound, pitch }));
        out
    };
    let bend = |value: i16| MidiKind::PitchBend((value + 8192).clamp(0, 16383) as u16);
    if let Some(z) = st.mpe.as_mut() {
        // Members: per-note; the manager (and other CCs): the whole zone, as channel 1.
        let member = |ch: u8| (2..=16).contains(&ch).then(|| ch as usize - 1);
        let dim = |m: &mut Mpe, i: usize, d: usize, kind: Dim, v: f32| match &m.members[i] {
            Some((_, hits)) => express(hits, ExprKind::Dim(kind), v),
            None => {
                m.initial[i][d] = Some(v);
                Vec::new()
            }
        };
        return match msg {
            InMsg::NoteOn { ch, note, velocity } => {
                let hits = t.route_note(port, 1, note as i64);
                let mut out = on(&hits, velocity);
                if let Some(i) = member(ch) {
                    for (d, kind) in Dim::ALL.into_iter().enumerate() {
                        if let Some(v) = z.initial[i][d].take() {
                            out.extend(express(&hits, ExprKind::Dim(kind), v));
                        }
                    }
                    z.members[i] = Some((note, hits));
                }
                out
            }
            InMsg::NoteOff { ch, note, velocity } => {
                if let Some(i) = member(ch)
                    && z.members[i].as_ref().is_some_and(|(n, _)| *n == note)
                {
                    z.members[i] = None;
                    z.initial[i] = [None; 3];
                }
                off(&t.route_note(port, 1, note as i64), velocity)
            }
            InMsg::PitchBend { ch, value } => match member(ch) {
                Some(i) => {
                    let v = value as f32 / 8192.0 * z.bend_range;
                    dim(z, i, 0, Dim::Pitch, v)
                }
                None => control(1, bend(value)),
            },
            InMsg::Control { ch, number: 74, value } if member(ch).is_some() => {
                dim(z, member(ch).unwrap(), 1, Dim::Slide, value as f32 / 127.0)
            }
            InMsg::ChannelPressure { ch, value } => match member(ch) {
                Some(i) => dim(z, i, 2, Dim::Pressure, value as f32 / 127.0),
                None => control(1, MidiKind::ChannelPressure(value)),
            },
            InMsg::Control { number, value, .. } => control(1, MidiKind::Controller { number, value }),
            InMsg::PolyPressure { note, value, .. } => {
                let held: Vec<_> = z.members.iter().flatten().filter(|(n, _)| *n == note).flat_map(|(_, h)| h.clone()).collect();
                express(&held, ExprKind::Dim(Dim::Pressure), value as f32 / 127.0)
            }
            InMsg::Program { .. } => Vec::new(),
        };
    }
    match msg {
        InMsg::NoteOn { ch, note, velocity } => {
            let hits = t.route_note(port, ch, note as i64);
            st.held.retain(|(c, n, _)| (*c, *n) != (ch, note));
            st.held.push((ch, note, hits.clone()));
            on(&hits, velocity)
        }
        InMsg::NoteOff { ch, note, velocity } => {
            st.held.retain(|(c, n, _)| (*c, *n) != (ch, note));
            off(&t.route_note(port, ch, note as i64), velocity)
        }
        InMsg::PolyPressure { ch, note, value } => {
            let held: Vec<_> = st.held.iter().filter(|(c, n, _)| (*c, *n) == (ch, note)).flat_map(|(_, _, h)| h.clone()).collect();
            express(&held, ExprKind::Dim(Dim::Pressure), value as f32 / 127.0)
        }
        InMsg::ChannelPressure { ch, value } => control(ch, MidiKind::ChannelPressure(value)),
        InMsg::Control { ch, number, value } => control(ch, MidiKind::Controller { number, value }),
        InMsg::PitchBend { ch, value } => control(ch, bend(value)),
        InMsg::Program { .. } => Vec::new(),
    }
}

/// Play an incoming message on the channels it reaches.
fn handle(
    table: &Mutex<MidiTable>,
    output: &Mutex<Option<Sender>>,
    pending: &Mutex<Vec<Pending>>,
    repaint: &Mutex<Option<eframe::egui::Context>>,
    port: &str,
    st: &mut PortState,
    msg: InMsg,
) {
    let t = table.lock().unwrap();
    let out = output.lock().unwrap().clone().filter(Sender::has_engine);
    let mut later = Vec::new();
    for l in route(&t, st, port, msg) {
        match (&out, l) {
            (Some(o), l) => {
                o.live(l);
            }
            // No engine yet: the notes, for the app to play.
            (None, Live::NoteIn { group, sound, pitch, velocity, .. }) => later.push((group, sound, pitch, velocity)),
            (None, Live::NoteRelease { group, sound, pitch }) => later.push((group, sound, pitch, 0)),
            (None, _) => {}
        }
    }
    if !later.is_empty() {
        pending.lock().unwrap().extend(later);
        if let Some(ctx) = repaint.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The set-up goes to ports newly set to MPE and to those whose range changed.
    #[test]
    fn setup_is_due_for_new_and_changed_ports() {
        use std::collections::BTreeMap;
        let m = |v: &[(&str, u8)]| v.iter().map(|(k, r)| (k.to_string(), *r)).collect::<BTreeMap<_, _>>();
        assert_eq!(super::setup_due(&m(&[]), &m(&[("A", 48)])), vec![("A".to_string(), 48)]);
        assert_eq!(super::setup_due(&m(&[("A", 48)]), &m(&[("A", 48)])), vec![]);
        assert_eq!(super::setup_due(&m(&[("A", 48)]), &m(&[("A", 24)])), vec![("A".to_string(), 24)]);
        assert_eq!(super::setup_due(&m(&[("A", 48)]), &m(&[])), vec![]);
    }

    mod routing {
        use super::super::*;
        use deckard_engine::Live;
        use deckard_model::{Dim, ExprKind, Group, GroupChannel, Project, Sound, SoundChannel};

        /// Group 0: sound 0 takes "Hydra" (any channel), sound 1 plays nothing; group 1
        /// in Manual (keyboard) mode takes "Pads" channel 10 from C1 on its 16 pads.
        fn table() -> MidiTable {
            let sound = |src: &str| Sound {
                channel: SoundChannel { midi_source: src.into(), midi_channel: 0, ..Default::default() },
                ..Default::default()
            };
            let p = Project {
                groups: vec![
                    Group {
                        sounds: vec![sound("Hydra"), sound("Default")],
                        channel: GroupChannel { key_mode: 0, midi_source: "None".into(), ..Default::default() },
                        ..Default::default()
                    },
                    Group {
                        sounds: (0..16).map(|_| sound("Default")).collect(),
                        channel: GroupChannel {
                            key_mode: 2,
                            midi_source: "Pads".into(),
                            midi_channel: 10,
                            start_note: 36,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                ],
                ..Default::default()
            };
            let mut t = MidiTable::new(&p);
            t.focus = (0, 1);
            t.default_focus = false;
            t
        }

        fn mpe() -> PortState {
            PortState { mpe: Some(Mpe::new(48)), ..PortState::default() }
        }

        fn expr(l: &Live) -> Option<(i64, ExprKind, f32)> {
            match *l {
                Live::Expression { pitch, kind, value, .. } => Some((pitch, kind, value)),
                _ => None,
            }
        }

        fn run(t: &MidiTable, st: &mut PortState, port: &str, bytes: &[&[u8]]) -> Vec<Live> {
            bytes.iter().flat_map(|b| route(t, st, port, InMsg::parse(b).unwrap())).collect()
        }

        #[test]
        fn mpe_member_note_gets_its_expression() {
            let (t, mut st) = (table(), mpe());
            // Bend before the note-on (its initial value), then the note's life.
            let out = run(&t, &mut st, "Hydra", &[&[0xe1, 0, 0x50], &[0x91, 60, 100], &[0xb1, 74, 64], &[0xd1, 100], &[0x81, 60, 50]]);
            assert!(matches!(out[0], Live::NoteIn { group: 0, sound: 0, pitch: 60, velocity: 100, .. }), "{out:?}");
            let e: Vec<(i64, ExprKind, f32)> = out.iter().filter_map(expr).collect();
            let bend = ((0x50i32 << 7) - 8192) as f32 / 8192.0 * 48.0;
            assert_eq!(e[0], (60, ExprKind::Dim(Dim::Pitch), bend));
            assert_eq!((e[1].1, (e[1].2 * 1000.0).round()), (ExprKind::Dim(Dim::Slide), 504.0));
            assert_eq!((e[2].1, (e[2].2 * 1000.0).round()), (ExprKind::Dim(Dim::Pressure), 787.0));
            assert_eq!((e[3].1, (e[3].2 * 1000.0).round()), (ExprKind::Release, 394.0));
            assert!(matches!(out.last(), Some(Live::NoteRelease { group: 0, sound: 0, pitch: 60 })));
            assert_eq!(e.len(), 4);
        }

        #[test]
        fn two_member_notes_stay_apart() {
            let (t, mut st) = (table(), mpe());
            let out = run(&t, &mut st, "Hydra", &[&[0x91, 60, 100], &[0x92, 64, 100], &[0xe2, 0, 0x60]]);
            let e: Vec<_> = out.iter().filter_map(expr).collect();
            assert_eq!(e.len(), 1);
            assert_eq!(e[0].0, 64);
        }

        #[test]
        fn a_reused_member_channel_rebinds() {
            let (t, mut st) = (table(), mpe());
            let out = run(&t, &mut st, "Hydra", &[&[0x91, 60, 100], &[0x91, 67, 100], &[0xe1, 0, 0x60]]);
            let e: Vec<_> = out.iter().filter_map(expr).collect();
            assert_eq!(e.iter().map(|x| x.0).collect::<Vec<_>>(), vec![67]);
        }

        #[test]
        fn manager_channel_is_channel_wide() {
            let (t, mut st) = (table(), mpe());
            let out = run(&t, &mut st, "Hydra", &[&[0x91, 60, 100], &[0xb0, 1, 90]]);
            assert!(out.iter().any(|l| matches!(l, Live::ControlIn { sound: 0, record: true, .. })), "{out:?}");
            assert!(out.iter().all(|l| expr(l).is_none()));
        }

        #[test]
        fn poly_pressure_reaches_the_held_note() {
            let (t, mut st) = (table(), PortState::default());
            let out = run(&t, &mut st, "Hydra", &[&[0x90, 60, 100], &[0x90, 64, 100], &[0xa0, 64, 80]]);
            let e: Vec<_> = out.iter().filter_map(expr).collect();
            assert_eq!(e.len(), 1);
            assert_eq!((e[0].0, e[0].1), (64, ExprKind::Dim(Dim::Pressure)));
        }

        #[test]
        fn pressure_for_a_released_note_is_dropped() {
            let (t, mut st) = (table(), PortState::default());
            let out = run(&t, &mut st, "Hydra", &[&[0x90, 64, 100], &[0x80, 64, 0], &[0xa0, 64, 80]]);
            assert!(out.iter().all(|l| expr(l).is_none()), "{out:?}");
            // Nor in MPE: a member channel after its note-off.
            let mut st = mpe();
            let out = run(&t, &mut st, "Hydra", &[&[0x91, 60, 100], &[0x81, 60, 0], &[0xd1, 80]]);
            assert!(out.iter().all(|l| expr(l).is_none()), "{out:?}");
        }

        #[test]
        fn channel_pressure_is_a_lane() {
            let (t, mut st) = (table(), PortState::default());
            let out = run(&t, &mut st, "Hydra", &[&[0xd0, 40]]);
            assert!(
                out.iter().any(|l| matches!(l, Live::ControlIn { sound: 0, kind: MidiKind::ChannelPressure(40), record: true, .. })),
                "{out:?}"
            );
        }

        #[test]
        fn only_the_focused_pad_records() {
            let (mut t, mut st) = (table(), PortState::default());
            t.focus = (1, 3);
            let out = run(&t, &mut st, "Pads", &[&[0xb9, 1, 90]]);
            let recorded: Vec<(usize, usize)> = out
                .iter()
                .filter_map(|l| match *l {
                    Live::ControlIn { group, sound, record: true, .. } => Some((group, sound)),
                    _ => None,
                })
                .collect();
            assert_eq!(recorded, vec![(1, 3)]);
            assert!(out.len() >= 16, "every pad still hears it");
        }
    }

    /// MIDI Lock Change: on its source and channel, note n (0–63) or program n recalls
    /// slot n; the rest of that channel's notes are taken too; other ports pass.
    #[test]
    fn lock_triggers() {
        let notes = LockMidi { source: "Keys".into(), channel: 2, trigger: 1 };
        let on = |note| InMsg::NoteOn { ch: 2, note, velocity: 100 };
        assert_eq!(lock_trigger(&notes, "Keys", &on(5)), Some(Some(5)));
        assert_eq!(lock_trigger(&notes, "Keys", &on(70)), Some(None), "past slot 64: taken, nothing");
        assert_eq!(lock_trigger(&notes, "Keys", &InMsg::NoteOff { ch: 2, note: 5, velocity: 0 }), Some(None));
        assert_eq!(lock_trigger(&notes, "Pads", &on(5)), None, "another port");
        assert_eq!(lock_trigger(&notes, "Keys", &InMsg::NoteOn { ch: 1, note: 5, velocity: 100 }), None, "another channel");
        let pc = LockMidi { trigger: 2, ..notes.clone() };
        assert_eq!(lock_trigger(&pc, "Keys", &InMsg::Program { ch: 2, program: 0 }), Some(Some(0)));
        assert_eq!(lock_trigger(&pc, "Keys", &on(5)), None, "notes play");
        let off = LockMidi { trigger: 0, ..notes };
        assert_eq!(lock_trigger(&off, "Keys", &on(5)), None);
    }
    /// Remote control takes controllers, program changes, pressure and the pitch wheel
    /// that are mapped, or any of them while learning; notes never.
    #[test]
    fn remote_control_takes_mapped_controls() {
        let mapped: std::collections::HashSet<u64> = [7, deckard_doc::remote::MIDI_KEY_PITCH].into();
        assert_eq!(remote_take(false, &mapped, &[0xb2, 7, 127]), Some((7, 1.0)));
        assert_eq!(remote_take(false, &mapped, &[0xb2, 8, 127]), None, "not mapped: plays");
        assert_eq!(remote_take(true, &mapped, &[0xb2, 8, 0]), Some((8, 0.0)), "learning");
        assert_eq!(remote_take(true, &mapped, &[0x90, 60, 100]), None, "notes play");
        assert!(remote_take(false, &mapped, &[0xe0, 0, 0x40]).is_some());
    }

    /// A mapped control from a port reaches the app, each key's last value once.
    #[cfg(unix)]
    #[test]
    fn mapped_controls_come_to_the_app() {
        let knobs_name = "DECKARD Test Knobs";
        let Ok(knobs_out) = midir::MidiOutput::new("test knobs") else { return };
        let Ok(mut knobs) = knobs_out.create_virtual(knobs_name) else { return };
        let mut io = MidiIo::default();
        io.set_remote(false, [7].into());
        io.listen(&[knobs_name.to_string()]);
        if io.inputs.is_empty() {
            return;
        }
        for v in [10, 64, 127] {
            knobs.send(&[0xb0, 7, v]).unwrap();
        }
        knobs.send(&[0xb0, 8, 1]).unwrap();
        let mut got = Vec::new();
        for _ in 0..100 {
            got.extend(io.take_remote());
            if got.iter().any(|x: &(u64, f64)| x.1 == 1.0) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(got.last(), Some(&(7, 1.0)));
        assert!(got.iter().all(|x| x.0 == 7), "CC 8 isn't mapped: {got:?}");
    }
    #[cfg(unix)]
    use deckard_model::{Group, MidiDest, Project, Sound, SoundChannel};
    #[cfg(unix)]
    use midir::os::unix::{VirtualInput, VirtualOutput};

    /// A note from a keyboard port plays the sound that takes it, which sends it (thru,
    /// on its channel, transposed) to the synth on its MIDI output. (Virtual ports: not
    /// on Windows.)
    #[cfg(unix)]
    #[test]
    fn keyboard_to_sound_to_synth() {
        let synth_name = "DECKARD Test Synth";
        let keys_name = "DECKARD Test Keys";
        let got: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
        let g2 = got.clone();
        let Ok(synth_in) = midir::MidiInput::new("test synth") else { return };
        let Ok(_synth) = synth_in.create_virtual(synth_name, move |_, b, _| g2.lock().unwrap().push(b.to_vec()), ()) else { return };
        let Ok(keys_out) = midir::MidiOutput::new("test keys") else { return };
        let Ok(mut keys) = keys_out.create_virtual(keys_name) else { return };
        let p = Project {
            groups: vec![Group {
                sounds: vec![Sound {
                    channel: SoundChannel {
                        midi_source: keys_name.into(),
                        midi_thru: true,
                        midi_out: MidiDest::Port(synth_name.into()),
                        midi_out_channel: 4,
                        transpose: 7,
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut io = MidiIo::default();
        let Ok(out) = deckard_engine::output::Output::new() else { return };
        let (mut engine, _) = deckard_engine::Engine::new(&p, None, out.rate, deckard_engine::Mode::Arrangement, &mut Default::default());
        engine.sequencing = false;
        engine.master = 0.0;
        out.set_midi(io.sender());
        out.play(engine);
        io.set_output(out.sender());
        io.set_table(MidiTable::new(&p));
        io.listen(&[keys_name.to_string()]);
        assert_eq!(io.inputs.len(), 1, "listening to the keyboard");
        keys.send(&[0x90, 60, 100]).unwrap();
        keys.send(&[0x80, 60, 0]).unwrap();
        for _ in 0..100 {
            if got.lock().unwrap().len() >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(*got.lock().unwrap(), vec![vec![0x94, 67, 100], vec![0x84, 67, 0]]);
    }

    /// What a test port received, and when.
    #[cfg(unix)]
    type Received = Arc<Mutex<Vec<(Vec<u8>, std::time::Instant)>>>;

    /// A virtual port to listen on: what arrives, and when. (Not on Windows.)
    #[cfg(unix)]
    fn listener(name: &str) -> Option<(midir::MidiInputConnection<()>, Received)> {
        let got: Received = Arc::default();
        let g2 = got.clone();
        let input = midir::MidiInput::new("test listener").ok()?;
        let conn = input.create_virtual(name, move |_, b, _| g2.lock().unwrap().push((b.to_vec(), std::time::Instant::now())), ()).ok()?;
        Some((conn, got))
    }

    /// MIDI clock is sent when it's due (with the audio), notes at once.
    #[cfg(unix)]
    #[test]
    fn timed_messages_wait_until_due() {
        let name = "DECKARD Test Clock Out";
        let Some((_conn, got)) = listener(name) else { return };
        let io = MidiIo::default();
        let t0 = std::time::Instant::now();
        let port: Arc<str> = name.into();
        let tx = io.sender();
        tx.send(MidiOut { port: port.clone(), bytes: [0xf8, 0, 0], len: 1, at: Some(t0 + std::time::Duration::from_millis(80)) }).unwrap();
        tx.send(MidiOut { port: port.clone(), bytes: [0x90, 60, 100], len: 3, at: None }).unwrap();
        for _ in 0..100 {
            if got.lock().unwrap().len() >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let got = got.lock().unwrap();
        assert_eq!(got.iter().map(|(b, _)| b.clone()).collect::<Vec<_>>(), [vec![0x90, 60, 100], vec![0xf8]]);
        let late = got[1].1.duration_since(t0);
        assert!(late >= std::time::Duration::from_millis(78) && late < std::time::Duration::from_millis(150), "{late:?}");
    }

    /// Incoming MIDI clock, on an input listened to, goes to the clock follower (and
    /// plays nothing).
    #[cfg(unix)]
    #[test]
    fn clock_in_feeds_the_follower() {
        let name = "DECKARD Test Clock In";
        let Ok(out) = midir::MidiOutput::new("test clock") else { return };
        let Ok(mut clock) = out.create_virtual(name) else { return };
        let mut io = MidiIo::default();
        let follower = Arc::new(deckard_engine::sync::ClockFollower::default());
        io.set_clock_follower(Some(follower.clone()));
        io.listen(&[name.to_string()]);
        assert_eq!(io.inputs.len(), 1);
        clock.send(&[0xfa]).unwrap();
        for _ in 0..4 {
            clock.send(&[0xf8]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        for _ in 0..100 {
            if follower.tempo().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(follower.take_transport(), Some(deckard_engine::sync::Transport::Start(0.0)));
        assert!(follower.tempo().is_some());
        assert!(io.take_pending().is_empty());
    }
}
