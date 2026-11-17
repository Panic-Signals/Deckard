//! Hosts one plug-in in this process, with a time limit, and prints JSON.
//!
//! ```text
//! plugin-probe list [--out <file.json>] [--timeout <s>]
//! plugin-probe dump <name or plugin_id> [--state <file.nksf|.ens|.nrkt>] [--out <file.json>] [--timeout <s>]
//! ```
//!
//! `list` gives the installed plug-ins DECKARD can host, as a JSON array. `dump` gives a
//! plug-in's parameters: the ones the automatic pages show, and the text of each at 64
//! evenly spaced values. A plug-in that crashes takes only this process with it.

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
}

#[derive(Debug, PartialEq)]
struct Args {
    command: Command,
    timeout: u64,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut it = args.iter();
    let verb = it.next().ok_or("usage: plugin-probe list | dump <plugin> [--state <file>] [--out <file>] [--timeout <s>]")?;
    let (mut positional, mut state, mut out, mut timeout) = (None, None, None, DEFAULT_TIMEOUT);
    while let Some(a) = it.next() {
        let mut value = |what: &str| it.next().cloned().ok_or(format!("{what} needs a value"));
        match a.as_str() {
            "--state" => state = Some(PathBuf::from(value("--state")?)),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--timeout" => timeout = value("--timeout")?.parse().map_err(|_| "--timeout is a number of seconds".to_string())?,
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            s if positional.is_none() => positional = Some(s.to_string()),
            s => return Err(format!("unexpected argument {s}")),
        }
    }
    let command = match verb.as_str() {
        "list" if positional.is_none() && state.is_none() => Command::List { out },
        "list" => return Err("list takes only --out and --timeout".into()),
        "dump" => Command::Dump { plugin: positional.ok_or("dump needs a plug-in name or id")?, state, out },
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

#[cfg(test)]
mod tests {
    use super::*;

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
