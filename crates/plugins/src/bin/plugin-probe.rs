//! Hosts one plug-in in this process, with a time limit, and prints JSON.
//!
//! ```text
//! plugin-probe list [--out <file.json>] [--timeout <s>]
//! plugin-probe dump <name or plugin_id> [--state <file.nksf|.ens|.nrkt>] [--out <file.json>] [--timeout <s>]
//! plugin-probe measure <name or plugin_id> --recipe <recipe.json> [--state <file>] [--out <file.json>] [--timeout <s>]
//! ```
//!
//! `list` gives the installed plug-ins DECKARD can host, as a JSON array. `dump` gives a
//! plug-in's parameters: the ones the automatic pages show, and the text of each at 64
//! evenly spaced values. A plug-in that crashes takes only this process with it.
//!
//! `measure` plays an instrument and measures what its knobs do in real units, for a
//! mapping's tapers when the plug-in's own text gives none (a knob reading "0-10"). A
//! recipe (JSON) sets the knobs and says what to sweep:
//!
//! ```json
//! { "note": 81, "setup": { "18": 1.0, "28": 0.0 },
//!   "steps": [ { "name": "cutoff", "kind": "cutoff", "param": 33, "points": 21, "setup": { "34": 0.9 } },
//!              { "name": "attack", "kind": "attack", "param": 43, "points": 15 },
//!              { "name": "decay",  "kind": "decay",  "param": 44, "points": 15, "setup": { "45": 0.0 } } ] }
//! ```
//!
//! (`setup`: parameter id to normalised value, the recipe's for every step and then the
//! step's own.) Each step sweeps its parameter over `points` evenly spaced values, holds
//! `note` and measures:
//!
//! - `cutoff`: the frequency (Hz) where the output's spectrum peaks: set the recipe up so
//!   noise goes through the filter with the resonance high;
//! - `attack`: the time (ms) the level takes to reach 90% of the level the note holds
//!   with the parameter at 0 (40 000 meaning 40 s or more);
//! - `decay`: the time (ms) from the peak to 10% of it (the level as a linear picture
//!   shows it falling), up to 40 s;
//! - `bright_attack`, `bright_decay`: the same on the output's high frequencies (above
//!   about 2 kHz): a filter envelope's times, set up with noise through a low cutoff and
//!   the envelope opening it. A ballpark: brightness follows the cutoff on a log scale.
//!
//! The output lists each point (value, the plug-in's text, the measurement) and a taper
//! for a mapping: `{ "table", { 0, 20.0 }, ... }`.

use deckard_plugins::scan::{self, Entry};
use deckard_plugins::{Host, ParamInfo, Slot, module, nks};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// How many values of each parameter's range have their text read.
const POINTS: usize = 64;
const DEFAULT_TIMEOUT: u64 = 60;

#[derive(Debug, PartialEq)]
enum Command {
    List { out: Option<PathBuf> },
    Dump { plugin: String, state: Option<PathBuf>, out: Option<PathBuf> },
    Measure { plugin: String, recipe: PathBuf, state: Option<PathBuf>, out: Option<PathBuf> },
}

#[derive(Debug, PartialEq)]
struct Args {
    command: Command,
    timeout: u64,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut it = args.iter();
    let verb = it.next().ok_or("usage: plugin-probe list | dump <plugin> [--state <file>] [--out <file>] [--timeout <s>]")?;
    let (mut positional, mut state, mut out, mut timeout, mut recipe) = (None, None, None, DEFAULT_TIMEOUT, None);
    while let Some(a) = it.next() {
        let mut value = |what: &str| it.next().cloned().ok_or(format!("{what} needs a value"));
        match a.as_str() {
            "--state" => state = Some(PathBuf::from(value("--state")?)),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--recipe" => recipe = Some(PathBuf::from(value("--recipe")?)),
            "--timeout" => timeout = value("--timeout")?.parse().map_err(|_| "--timeout is a number of seconds".to_string())?,
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            s if positional.is_none() => positional = Some(s.to_string()),
            s => return Err(format!("unexpected argument {s}")),
        }
    }
    let command = match verb.as_str() {
        "list" if positional.is_none() && state.is_none() => Command::List { out },
        "list" => return Err("list takes only --out and --timeout".into()),
        "dump" if recipe.is_none() => Command::Dump { plugin: positional.ok_or("dump needs a plug-in name or id")?, state, out },
        "measure" => Command::Measure {
            plugin: positional.ok_or("measure needs a plug-in name or id")?,
            recipe: recipe.ok_or("measure needs --recipe <file.json>")?,
            state,
            out,
        },
        "dump" => return Err("--recipe is for measure".into()),
        v => return Err(format!("unknown command {v}")),
    };
    Ok(Args { command, timeout })
}

/// The parameter dump: the plug-in, and every parameter with its text at [`POINTS`] values.
fn dump_json(
    name: &str,
    id: &str,
    version: Option<&str>,
    all: &[ParamInfo],
    shown: &[ParamInfo],
    text: &dyn Fn(u32, f64) -> String,
) -> Value {
    let params: Vec<Value> = all
        .iter()
        .map(|p| {
            let texts: Vec<String> = (0..POINTS).map(|i| text(p.id, i as f64 / (POINTS - 1) as f64)).collect();
            json!({
                "id": p.id,
                "title": p.title,
                "short": p.short,
                "units": p.units,
                "steps": p.steps,
                "default": p.default,
                "shown": shown.iter().any(|s| s.id == p.id),
                "text": texts,
            })
        })
        .collect();
    json!({ "name": name, "id": id, "version": version, "points": POINTS, "count": params.len(), "parameters": params })
}

fn entry_json(e: &Entry) -> Value {
    json!({
        "name": e.name,
        "vendor": e.vendor,
        "version": Value::Null,
        "format": e.kind.label(),
        "instrument": e.instrument,
        "identity": e.maschine_id(),
        "cid": e.cid,
        "location": e.location,
    })
}

/// The plug-ins DECKARD can host in the standard folders.
fn installed() -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    let mut all = Vec::new();
    for b in scan::vst3_bundles(&module::search_paths()) {
        if !seen.insert(b.display().to_string().to_lowercase()) {
            continue;
        }
        match scan::vst3_from_moduleinfo(&b).map(Ok).unwrap_or_else(|| scan::vst3_by_loading(&b)) {
            Ok(e) => all.extend(e),
            Err(e) => eprintln!("{}: {e}", b.display()),
        }
    }
    all.extend(scan::audio_units());
    all.retain(Entry::hostable);
    all.sort();
    all.dedup();
    all
}

fn find<'a>(installed: &'a [Entry], wanted: &str) -> Result<&'a Entry, String> {
    installed
        .iter()
        .find(|e| e.maschine_id().is_some_and(|id| id.eq_ignore_ascii_case(wanted)))
        .or_else(|| installed.iter().find(|e| e.name.eq_ignore_ascii_case(wanted)))
        .ok_or_else(|| format!("no installed plug-in {wanted}"))
}

/// A preset file's plug-in id, name and state chunk.
fn load_state(path: &Path, installed: &[Entry]) -> Result<nks::Resolved, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "nksf" {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        return nks::parse(&bytes)?.resolve(installed);
    }
    nks::native_preset_file(path, None, false, installed)
        .unwrap_or_else(|| Err(format!("{}: not a preset type the probe knows", path.display())))
}

/// The plug-in's id, name and state to load, from `--state` or the installed list.
fn identity(plugin: &str, state: Option<&Path>, installed: &[Entry]) -> Result<(String, String, Vec<u8>), String> {
    Ok(match state {
        Some(path) => {
            let r = load_state(path, installed)?;
            (r.id, r.name, r.chunk)
        }
        None => {
            let e = find(installed, plugin)?;
            (e.maschine_id().ok_or("the plug-in has no id")?, e.name.clone(), Vec::new())
        }
    })
}

fn dump(plugin: &str, state: Option<&Path>, installed: &[Entry]) -> Result<Value, String> {
    let (id, name, chunk, version) = match state {
        Some(path) => {
            let r = load_state(path, installed)?;
            (r.id, r.name, r.chunk, None)
        }
        None => {
            let e = find(installed, plugin)?;
            (e.maschine_id().ok_or("the plug-in has no id")?, e.name.clone(), Vec::new(), None::<String>)
        }
    };
    let mut host = Host::default();
    let inst = host.instance(Slot::Sound { group: 0, sound: 0 }, &id, &name, &chunk, 44_100.0)?;
    let p = inst.lock().map_err(|_| "plug-in lock poisoned".to_string())?;
    let (all, shown) = (p.parameter_list(false), p.parameters());
    let params = p.params().ok_or("the plug-in has no parameter handle")?;
    Ok(dump_json(&name, &id, version.as_deref(), &all, &shown, &|id, v| params.text(id, v)))
}

fn run(args: Args) -> Result<(), String> {
    let (value, out) = match args.command {
        Command::List { out } => (Value::Array(installed().iter().map(entry_json).collect()), out),
        Command::Dump { plugin, state, out } => (dump(&plugin, state.as_deref(), &installed())?, out),
        Command::Measure { plugin, recipe, state, out } => {
            let text = std::fs::read_to_string(&recipe).map_err(|e| format!("{}: {e}", recipe.display()))?;
            let recipe = measure::Recipe::parse(&serde_json::from_str(&text).map_err(|e| format!("{}: {e}", recipe.display()))?)?;
            let (id, name, chunk) = identity(&plugin, state.as_deref(), &installed())?;
            let mut host = Host::default();
            let inst = host.instance(Slot::Sound { group: 0, sound: 0 }, &id, &name, &chunk, measure::RATE)?;
            let mut p = inst.lock().map_err(|_| "plug-in lock poisoned".to_string())?;
            (measure::run(&mut p, &name, &recipe)?, out)
        }
    };
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    match out {
        Some(path) => std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?,
        None => println!("{text}"),
    }
    Ok(())
}

fn main() {
    // Plug-ins load on this thread.
    deckard_plugins::mark_main_thread();
    let args = match parse_args(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("plugin-probe: {e}");
            deckard_plugins::exit(2)
        }
    };
    let limit = args.timeout;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(limit));
        eprintln!("plugin-probe: timed out after {limit} s");
        deckard_plugins::exit(3)
    });
    match run(args) {
        Ok(()) => deckard_plugins::exit(0),
        Err(e) => {
            eprintln!("plugin-probe: {e}");
            deckard_plugins::exit(1)
        }
    }
}

mod measure {
    use deckard_plugins::{Plugin, Transport};
    use serde_json::{Value, json};

    pub const RATE: f64 = 44_100.0;
    const BLOCK: usize = 512;
    /// The level is followed in windows this long.
    const WINDOW: usize = 220;
    const WINDOW_MS: f64 = WINDOW as f64 * 1000.0 / RATE;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Kind {
        Cutoff,
        Attack,
        Decay,
        BrightAttack,
        BrightDecay,
    }

    #[derive(Debug, PartialEq)]
    pub struct Step {
        pub name: String,
        pub kind: Kind,
        pub param: u32,
        pub points: usize,
        pub setup: Vec<(u32, f64)>,
    }

    #[derive(Debug, PartialEq)]
    pub struct Recipe {
        pub note: i64,
        pub setup: Vec<(u32, f64)>,
        pub steps: Vec<Step>,
    }

    fn setup(v: &Value) -> Result<Vec<(u32, f64)>, String> {
        let Some(m) = v.as_object() else { return Ok(Vec::new()) };
        let mut out: Vec<(u32, f64)> = m
            .iter()
            // (Keys starting with `_` are notes for whoever fills the recipe in.)
            .filter(|(k, _)| !k.starts_with('_'))
            .map(|(k, v)| {
                Ok((
                    k.parse().map_err(|_| format!("setup: \"{k}\" isn't a parameter id"))?,
                    v.as_f64().ok_or(format!("setup: {k} isn't a number"))?,
                ))
            })
            .collect::<Result<_, String>>()?;
        out.sort_by_key(|(k, _)| *k);
        Ok(out)
    }

    impl Recipe {
        pub fn parse(v: &Value) -> Result<Recipe, String> {
            let mut steps = Vec::new();
            for s in v["steps"].as_array().ok_or("a recipe has steps")? {
                let kind = match s["kind"].as_str() {
                    Some("cutoff") => Kind::Cutoff,
                    Some("attack") => Kind::Attack,
                    Some("decay") => Kind::Decay,
                    Some("bright_attack") => Kind::BrightAttack,
                    Some("bright_decay") => Kind::BrightDecay,
                    k => return Err(format!("no measurement kind {k:?} (cutoff, attack, decay, bright_attack, bright_decay)")),
                };
                steps.push(Step {
                    name: s["name"].as_str().unwrap_or("").to_string(),
                    kind,
                    param: s["param"].as_u64().ok_or("a step has a param id")? as u32,
                    points: s["points"].as_u64().unwrap_or(11).clamp(2, 64) as usize,
                    setup: setup(&s["setup"])?,
                });
            }
            Ok(Recipe { note: v["note"].as_i64().unwrap_or(81), setup: setup(&v["setup"])?, steps })
        }
    }

    /// Runs the plug-in for `secs` (mono), stopping early when `stop` says so (it sees the
    /// level windows so far).
    fn render(p: &mut Plugin, secs: f64, bright: bool, mut stop: impl FnMut(&[f32]) -> bool) -> Vec<f32> {
        let t = Transport::default();
        let mut out = Vec::new();
        let mut env = Vec::new();
        while (out.len() as f64) < secs * RATE {
            let (l, r) = p.process(BLOCK, &t);
            out.extend(l.iter().zip(r).map(|(a, b)| (a + b) * 0.5));
            while (env.len() + 1) * WINDOW <= out.len() {
                let w = &out[env.len() * WINDOW..(env.len() + 1) * WINDOW];
                let w = if bright { highs(w) } else { w.to_vec() };
                env.push((w.iter().map(|x| x * x).sum::<f32>() / WINDOW as f32).sqrt());
            }
            if stop(&env) {
                break;
            }
        }
        out
    }

    /// The level (RMS) in windows of [`WINDOW`] samples.
    pub fn envelope(x: &[f32]) -> Vec<f32> {
        x.chunks_exact(WINDOW).map(|w| (w.iter().map(|v| v * v).sum::<f32>() / WINDOW as f32).sqrt()).collect()
    }

    /// `x`'s high frequencies: three first differences (+18 dB an octave, crossing
    /// unity gain near RATE / 6; below about 2 kHz it's 20 dB down and falling).
    pub fn highs(x: &[f32]) -> Vec<f32> {
        let mut y = x.to_vec();
        for _ in 0..3 {
            for i in (1..y.len()).rev() {
                y[i] -= y[i - 1];
            }
        }
        y
    }

    /// Windows at a note's start a click can fill (20 ms).
    const CLICK: usize = 4;

    /// The level to time: [`envelope`], skipping the note's first 20 ms (a click at its
    /// start mustn't count as its peak); for the high frequencies, also averaged over
    /// 50 ms, since noise's level wavers from window to window.
    fn level(x: &[f32], bright: bool) -> Vec<f32> {
        let skip = CLICK * WINDOW;
        let x = &x[skip.min(x.len())..];
        if !bright {
            return envelope(x);
        }
        let e = envelope(&highs(x));
        let n = (50.0 / WINDOW_MS) as usize;
        (0..e.len()).map(|i| e[i.saturating_sub(n / 2)..(i + n / 2 + 1).min(e.len())].iter().sum::<f32>() / (n + 1) as f32).collect()
    }

    /// The time (ms) the level takes to reach 95% of its peak.
    pub fn rise_ms(env: &[f32]) -> Option<f64> {
        let peak = env.iter().copied().fold(0.0, f32::max);
        if peak < 1e-4 {
            return None;
        }
        env.iter().position(|&e| e >= 0.95 * peak).map(|i| (i as f64 + 0.5) * WINDOW_MS)
    }

    /// The time (ms) from the level's peak to 10% of it.
    pub fn fall_ms(env: &[f32]) -> Option<f64> {
        let (at, peak) = env.iter().copied().enumerate().fold((0, 0.0f32), |a, (i, e)| if e > a.1 { (i, e) } else { a });
        if peak < 1e-4 {
            return None;
        }
        env[at..].iter().position(|&e| e <= 0.1 * peak).map(|i| i as f64 * WINDOW_MS)
    }

    /// The frequency (Hz) where `x`'s spectrum peaks, 20 Hz to 20 kHz (Goertzel at 1/24
    /// octave steps, smoothed over three).
    pub fn peak_hz(x: &[f32]) -> Option<f64> {
        let n = x.len() as f64;
        let freqs: Vec<f64> = (0..).map(|i| 20.0 * 2f64.powf(i as f64 / 24.0)).take_while(|f| *f <= 20_000.0).collect();
        let power: Vec<f64> = freqs
            .iter()
            .map(|f| {
                let w = 2.0 * std::f64::consts::PI * f / RATE;
                let c = 2.0 * w.cos();
                let (mut s1, mut s2) = (0.0, 0.0);
                for (i, v) in x.iter().enumerate() {
                    // (A Hann window, so a strong low peak doesn't leak over the rest.)
                    let h = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n).cos();
                    let s = *v as f64 * h + c * s1 - s2;
                    s2 = s1;
                    s1 = s;
                }
                s1 * s1 + s2 * s2 - c * s1 * s2
            })
            .collect();
        if power.iter().all(|p| *p < 1e-9) {
            return None;
        }
        let smooth = |i: usize| power[i.saturating_sub(1)..(i + 2).min(power.len())].iter().sum::<f64>();
        (0..power.len()).max_by(|a, b| smooth(*a).total_cmp(&smooth(*b))).map(|i| freqs[i])
    }

    /// One measurement at the parameter's value `v`. `full`: the level a note reaches
    /// (for the attacks: the level held with the parameter at 0).
    fn point(p: &mut Plugin, r: &Recipe, s: &Step, v: f64, full: Option<f32>) -> Option<f64> {
        p.all_notes_off();
        for (id, x) in r.setup.iter().chain(&s.setup) {
            p.set_param(*id, *x);
        }
        p.set_param(s.param, v);
        // (Settle: the old note's release, smoothed parameters.)
        render(p, 0.6, false, |_| false);
        p.note_on(r.note, 100);
        let out = match s.kind {
            Kind::Cutoff => {
                let x = render(p, 1.4, false, |_| false);
                let from = (0.4 * RATE) as usize;
                peak_hz(&x[from.min(x.len())..])
            }
            Kind::Attack | Kind::BrightAttack => {
                let bright = s.kind == Kind::BrightAttack;
                // Until it reaches 90% of the full level (or 40 s: then "at least 40 s"). (Not
                // 95%: a free-running synth's level differs a few percent from note to note.)
                let target = 0.9 * full?;
                let x = render(p, 40.0, bright, |env| env.len() > CLICK + 10 && env[env.len() - 10..].iter().sum::<f32>() / 10.0 >= target);
                let e = level(&x, bright);
                // (Averaged over 25 ms either side, against the level's ripple.)
                let avg = |i: usize| {
                    e[i.saturating_sub(5)..(i + 6).min(e.len())].iter().sum::<f32>()
                        / (i + 6).min(e.len()).saturating_sub(i.saturating_sub(5)) as f32
                };
                let at = (0..e.len()).find(|&i| avg(i) >= target);
                Some(at.map_or(40_000.0, |i| (i + CLICK) as f64 * WINDOW_MS))
            }
            Kind::Decay | Kind::BrightDecay => {
                let bright = s.kind == Kind::BrightDecay;
                let x = render(p, 40.0, bright, |env| {
                    let (at, peak) = env.iter().copied().enumerate().fold((0, 0.0f32), |a, (i, e)| if e > a.1 { (i, e) } else { a });
                    peak > 1e-4 && env[at..].iter().any(|&e| e <= 0.05 * peak)
                });
                fall_ms(&level(&x, bright))
            }
        };
        p.note_off(r.note);
        out
    }

    pub fn run(p: &mut Plugin, name: &str, r: &Recipe) -> Result<Value, String> {
        let params = p.params().ok_or("the plug-in has no parameter handle")?;
        let mut steps = Vec::new();
        for s in &r.steps {
            let mut points = Vec::new();
            let mut table = Vec::new();
            // The attacks' full level: the note held with the parameter at 0.
            let full = match s.kind {
                Kind::Attack | Kind::BrightAttack => {
                    let bright = s.kind == Kind::BrightAttack;
                    p.all_notes_off();
                    for (id, x) in r.setup.iter().chain(&s.setup) {
                        p.set_param(*id, *x);
                    }
                    p.set_param(s.param, 0.0);
                    render(p, 0.6, false, |_| false);
                    p.note_on(r.note, 100);
                    let e = level(&render(p, 1.5, bright, |_| false), bright);
                    p.note_off(r.note);
                    let tail = &e[e.len() / 2..];
                    Some(tail.iter().sum::<f32>() / tail.len().max(1) as f32).filter(|l| *l > 1e-4)
                }
                _ => None,
            };
            for i in 0..s.points {
                let v = i as f64 / (s.points - 1) as f64;
                let m = point(p, r, s, v, full);
                eprintln!("{} {:.3} {:>10} -> {}", s.name, v, params.text(s.param, v).trim(), m.map_or("-".into(), |m| format!("{m:.1}")));
                if let Some(m) = m {
                    table.push(format!("{{ {v:.3}, {m:.1} }}"));
                }
                points.push(json!({ "value": v, "text": params.text(s.param, v).trim(), "measured": m }));
            }
            let unit = if s.kind == Kind::Cutoff { "hz" } else { "ms" };
            steps.push(json!({
                "name": s.name, "param": s.param, "unit": unit, "points": points,
                "taper": format!("{{ \"table\", {} }}", table.join(", ")),
            }));
        }
        Ok(json!({ "name": name, "note": r.note, "steps": steps }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_signals() {
        use measure::*;
        // A 1 kHz sine: its peak.
        let sine: Vec<f32> = (0..44_100).map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / RATE).sin() as f32).collect();
        let hz = peak_hz(&sine).unwrap();
        assert!((hz / 1000.0 - 1.0).abs() < 0.04, "{hz}");
        // A linear rise over 200 ms, then held: about 190 ms to 95%.
        let rise: Vec<f32> = (0..44_100).map(|i| (i as f64 / 8820.0).min(1.0) as f32 * sine[i]).collect();
        let ms = rise_ms(&envelope(&rise)).unwrap();
        assert!((ms - 190.0).abs() < 12.0, "{ms}");
        // An exponential fall with a 100 ms time constant: 10% at about 230 ms.
        let fall: Vec<f32> = (0..44_100).map(|i| (-(i as f64) / 4410.0).exp() as f32 * sine[i]).collect();
        let ms = fall_ms(&envelope(&fall)).unwrap();
        assert!((ms - 230.0).abs() < 15.0, "{ms}");
        assert_eq!(rise_ms(&[0.0; 10]), None);
        // High frequencies pass, low ones are held back.
        let tone = |hz: f64| -> f32 {
            let x: Vec<f32> = (0..4410).map(|i| (2.0 * std::f64::consts::PI * hz * i as f64 / RATE).sin() as f32).collect();
            envelope(&highs(&x))[5]
        };
        assert!(tone(8000.0) > 50.0 * tone(200.0), "{} vs {}", tone(8000.0), tone(200.0));
    }

    #[test]
    fn reads_a_recipe() {
        let v: Value = serde_json::from_str(
            r#"{ "note": 60, "setup": { "18": 1 }, "steps": [ { "name": "cutoff", "kind": "cutoff", "param": 33, "points": 5, "setup": { "34": 0.9 } } ] }"#,
        )
        .unwrap();
        let r = measure::Recipe::parse(&v).unwrap();
        assert_eq!(r.note, 60);
        let noted: Value = serde_json::from_str(r#"{ "setup": { "_why": "", "18": 1 }, "steps": [] }"#).unwrap();
        assert_eq!(measure::Recipe::parse(&noted).unwrap().setup, [(18, 1.0)], "notes are skipped");
        assert_eq!(r.setup, [(18, 1.0)]);
        assert_eq!(r.steps[0].kind, measure::Kind::Cutoff);
        assert_eq!(r.steps[0].setup, [(34, 0.9)]);
        assert!(measure::Recipe::parse(&serde_json::json!({ "steps": [ { "kind": "volume", "param": 1 } ] })).is_err());
        assert!(parse_args(&s(&["measure", "Monark"])).is_err(), "no recipe");
        assert!(matches!(parse_args(&s(&["measure", "Monark", "--recipe", "r.json"])).unwrap().command, Command::Measure { .. }));
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse_args(&s(&["list"])).unwrap(), Args { command: Command::List { out: None }, timeout: 60 });
        let a = parse_args(&s(&["dump", "Monark", "--state", "a.ens", "--out", "o.json", "--timeout", "5"])).unwrap();
        assert_eq!(
            a,
            Args {
                command: Command::Dump { plugin: "Monark".into(), state: Some("a.ens".into()), out: Some("o.json".into()) },
                timeout: 5
            }
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        for bad in [
            &[][..],
            &["dump"],
            &["list", "x"],
            &["dump", "a", "b"],
            &["dump", "a", "--nope"],
            &["dump", "a", "--state"],
            &["fly"],
            &["list", "--timeout", "x"],
        ] {
            assert!(parse_args(&s(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn dump_shape() {
        let info = |id, title: &str| ParamInfo { id, title: title.into(), short: "S".into(), units: "Hz".into(), steps: 0, default: 0.25 };
        let all = [info(7, "Cutoff"), info(9, "Hidden")];
        let v = dump_json("Fake", "V:VST3:FX:AB", Some("1.2"), &all, &all[..1], &|id, x| format!("{id}:{x:.2}"));
        assert_eq!(v["name"], "Fake");
        assert_eq!(v["version"], "1.2");
        assert_eq!(v["count"], 2);
        let p = &v["parameters"][0];
        assert_eq!(
            (p["id"].as_u64(), p["title"].as_str(), p["units"].as_str(), p["default"].as_f64()),
            (Some(7), Some("Cutoff"), Some("Hz"), Some(0.25))
        );
        assert_eq!(p["shown"], true);
        assert_eq!(v["parameters"][1]["shown"], false);
        let t = p["text"].as_array().unwrap();
        assert_eq!(t.len(), 64);
        assert_eq!(t[0], "7:0.00");
        assert_eq!(t[63], "7:1.00");
        assert_eq!(t[63 / 2 + 1 - 1], "7:0.49");
    }

    #[test]
    fn finds_by_name_or_id() {
        let e = Entry {
            kind: scan::Kind::Vst3,
            vendor: "V".into(),
            name: "Mono".into(),
            instrument: true,
            location: String::new(),
            cid: "AB".into(),
        };
        let list = [e];
        assert!(find(&list, "mono").is_ok());
        assert!(find(&list, "V:VST3:INSTR:AB").is_ok());
        assert!(find(&list, "nope").is_err());
    }
}
