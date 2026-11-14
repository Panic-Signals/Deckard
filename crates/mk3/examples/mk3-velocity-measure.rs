//! Measure how Maschine turns pad pressure into velocity: log the MK3's raw pad reports
//! (opened shared, next to Maschine) and the notes Maschine sends out on a MIDI port, on
//! one clock, into a CSV. Run once per Pad Sensitivity / Velocity Scaling setting, then fit
//! with `mk3-velocity-fit.py`. Only reads from the controller (pad reports, and a snapshot
//! of its feature reports at the start); nothing is written to it.
//!
//! `cargo run -p deckard-mk3 --release --example mk3-velocity-measure -- --label s50-lin
//!  --midi "<part of the input's name>" [--out pad-velocity]`   (`--list` lists the MIDI inputs)
//!
//! Enter (or `q` Enter) stops and writes the summary.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

enum Rec {
    Hid(f64, Vec<(u8, u16)>),
    Midi(f64, Vec<u8>),
    Stop,
}

/// A pad held down: when it went down and the pressures it has reported since.
struct Press {
    t: f64,
    pressures: Vec<u16>,
    matched: bool,
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let midi_in = midir::MidiInput::new("mk3-velocity-measure").expect("MIDI input");
    let ports = midi_in.ports();
    let names: Vec<String> = ports.iter().map(|p| midi_in.port_name(p).unwrap_or_default()).collect();
    if args.iter().any(|a| a == "--list") {
        names.iter().for_each(|n| println!("{n}"));
        return;
    }
    let Some(label) = arg(&args, "--label") else {
        eprintln!("--label <setting> is required, e.g. --label s50-lin");
        std::process::exit(2);
    };
    let Some(want) = arg(&args, "--midi").map(|m| m.to_lowercase()) else {
        eprintln!("--midi <part of the input's name> is required (the loopback port Maschine sends to); inputs:");
        names.iter().for_each(|n| eprintln!("  {n}"));
        std::process::exit(2);
    };
    let Some(port_index) = names.iter().position(|n| n.to_lowercase().contains(&want)) else {
        eprintln!("no MIDI input matching {want:?}; inputs:");
        names.iter().for_each(|n| eprintln!("  {n}"));
        std::process::exit(2);
    };

    let dir = PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "pad-velocity".into()));
    fs::create_dir_all(&dir).expect("create the output folder");
    let mut stem = label.clone();
    let mut n = 2;
    while dir.join(format!("{stem}.csv")).exists() {
        stem = format!("{label}-{n}");
        n += 1;
    }
    let csv_path = dir.join(format!("{stem}.csv"));

    let api = hidapi::HidApi::new().expect("hidapi");
    #[cfg(target_os = "macos")]
    api.set_open_exclusive(false);
    let dev = api.open(deckard_mk3::VENDOR, deckard_mk3::PRODUCT).expect("open the MK3's HID interface");

    // The feature reports as they stand: if Maschine pushes the sensitivity to the
    // controller, one of these may change from setting to setting.
    let mut features = String::new();
    for id in [0xd0u8, 0xd8, 0xd9, 0xda, 0xdb, 0xf0, 0xf2, 0xf8, 0xf9] {
        let mut buf = [0u8; 256];
        buf[0] = id;
        match dev.get_feature_report(&mut buf) {
            Ok(n) => features += &format!("0x{id:02x}: {:02x?}\n", &buf[..n]),
            Err(e) => features += &format!("0x{id:02x}: {e}\n"),
        }
    }
    fs::write(dir.join(format!("{stem}.features.txt")), &features).expect("write the feature reports");

    let start = Instant::now();
    let (tx, rx) = mpsc::channel::<Rec>();

    let midi_tx = tx.clone();
    let _conn = midi_in
        .connect(
            &ports[port_index],
            "mk3-velocity-measure",
            move |_, bytes, _| {
                let _ = midi_tx.send(Rec::Midi(start.elapsed().as_secs_f64() * 1000.0, bytes.to_vec()));
            },
            (),
        )
        .expect("open the MIDI input");

    let hid_tx = tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        loop {
            let Ok(n) = dev.read_timeout(&mut buf, 50) else { break };
            if n == 0 || buf[0] != 2 {
                continue;
            }
            let t = start.elapsed().as_secs_f64() * 1000.0;
            let entries: Vec<(u8, u16)> = buf[1..n]
                .chunks_exact(3)
                .map(|e| (e[0], u16::from_be_bytes([e[1], e[2]])))
                .take_while(|&(i, w)| i != 0 || w != 0)
                .collect();
            if hid_tx.send(Rec::Hid(t, entries)).is_err() {
                break;
            }
        }
    });

    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        let _ = tx.send(Rec::Stop);
    });

    let mut out = BufWriter::new(File::create(&csv_path).expect("create the CSV"));
    writeln!(out, "# label={label} midi={}", names[port_index]).unwrap();
    writeln!(out, "t_ms,source,pad,event,pressure,status,note,velocity").unwrap();
    println!("logging {label} -> {}", csv_path.display());
    println!("MIDI from {:?}. Play the pads, one hit at a time; Enter stops.", names[port_index]);

    let mut held: HashMap<usize, Press> = HashMap::new();
    let mut recent: Vec<(usize, Press)> = Vec::new();
    let (mut presses, mut notes, mut matched) = (0usize, 0usize, 0usize);
    while let Ok(rec) = rx.recv() {
        match rec {
            Rec::Stop => break,
            Rec::Hid(t, entries) => {
                for (index, word) in entries {
                    let (event, pressure) = (word >> 12, word & 0x0fff);
                    let pad = if index < 16 { deckard_mk3::pad_number(index) } else { 100 + index as usize };
                    writeln!(out, "{t:.3},hid,{pad},{event},{pressure},,,").unwrap();
                    match event {
                        0 | 1 => {
                            presses += 1;
                            held.insert(pad, Press { t, pressures: vec![pressure], matched: false });
                        }
                        4 => {
                            if let Some(p) = held.get_mut(&pad) {
                                p.pressures.push(pressure);
                            }
                        }
                        2 | 3 => {
                            if let Some(p) = held.remove(&pad) {
                                if !p.matched {
                                    println!("{t:9.1} ms  pad {pad:2} pressed (peak {:4}) but no note", p.pressures.iter().max().unwrap());
                                }
                                recent.push((pad, p));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Rec::Midi(t, bytes) => {
                let fields = match bytes.as_slice() {
                    [s, n, v, ..] => format!("{s},{n},{v}"),
                    [s, n] => format!("{s},{n},"),
                    [s] => format!("{s},,"),
                    [] => ",,".into(),
                };
                writeln!(out, "{t:.3},midi,,,,{fields}").unwrap();
                if let [s, _, v, ..] = bytes.as_slice() {
                    if s & 0xf0 == 0x90 && *v > 0 {
                        notes += 1;
                        // The latest unmatched press within 100 ms.
                        let best = held
                            .iter_mut()
                            .map(|(pad, p)| (*pad, p))
                            .chain(recent.iter_mut().map(|(pad, p)| (*pad, p)))
                            .filter(|(_, p)| !p.matched && p.t <= t && t - p.t < 100.0)
                            .max_by(|a, b| a.1.t.total_cmp(&b.1.t));
                        match best {
                            Some((pad, p)) => {
                                matched += 1;
                                p.matched = true;
                                let first: Vec<String> = p.pressures.iter().take(4).map(|x| x.to_string()).collect();
                                println!(
                                    "{t:9.1} ms  vel {v:3}  <- pad {pad:2}, {:4.1} ms after press, pressures {}",
                                    t - p.t,
                                    first.join(" ")
                                );
                            }
                            None => println!("{t:9.1} ms  vel {v:3}  (no press within 100 ms)"),
                        }
                    }
                }
                recent.retain(|(_, p)| t - p.t < 200.0);
            }
        }
        out.flush().ok();
    }
    out.flush().unwrap();
    println!("{label}: {presses} presses, {notes} notes, {matched} matched -> {}", csv_path.display());
}
