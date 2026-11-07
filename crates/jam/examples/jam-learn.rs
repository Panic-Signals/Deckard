//! Learn which bit each JAM control sets: it names a control, you press it, it records
//! what changed. Then the encoder, the strips and the footswitch, one at a time.
//! `cargo run -p deckard-jam --example jam-learn [from]`
//! Prompts are spoken too. There's no time limit. Enter skips a control; Ctrl-C stops, and
//! `from` (1..) picks up at that control. Close Maschine first. Writes to
//! $JAM_LEARN_OUT (default ./jam-map.txt), appending.

mod support;

use deckard_jam::{COLUMNS, Change, changes, open, rising, switches};
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::time::{Duration, Instant};
use support::{refuse_if_maschine_runs, say, typed};

/// The last report of each id, and the reads.
struct Watch {
    dev: hidapi::HidDevice,
    last: BTreeMap<u8, Vec<u8>>,
}

impl Watch {
    /// The next report's id and what changed in it, if one came within 20 ms.
    fn next(&mut self) -> Option<(u8, Vec<Change>)> {
        let mut buf = [0u8; 256];
        let n = self.dev.read_timeout(&mut buf, 20).expect("read");
        if n == 0 {
            return None;
        }
        let was = self.last.insert(buf[0], buf[..n].to_vec()).unwrap_or_default();
        Some((buf[0], changes(&was, &buf[..n])))
    }

    /// Everything that changes for `d`, in text.
    fn gather(&mut self, d: Duration) -> Vec<String> {
        let start = Instant::now();
        let mut seen: BTreeMap<(u8, usize), (u8, u8, u32)> = BTreeMap::new();
        while start.elapsed() < d {
            if let Some((id, c)) = self.next() {
                for x in c {
                    let e = seen.entry((id, x.byte)).or_insert((x.now, x.now, 0));
                    e.0 = e.0.min(x.now);
                    e.1 = e.1.max(x.now);
                    e.2 += 1;
                }
            }
        }
        seen.iter().map(|((id, b), (lo, hi, n))| format!("{id:#04x}[{b}] {lo:02x}..{hi:02x} x{n}")).collect()
    }
}

fn main() {
    refuse_if_maschine_runs();
    let from: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
    let path = std::env::var("JAM_LEARN_OUT").unwrap_or_else(|_| "jam-map.txt".into());
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path).expect("output file");
    let mut w = Watch { dev: open().expect("open the JAM"), last: BTreeMap::new() };
    // Bits already named, here or in a run this one picks up from: a control can't take
    // another's (pushing the encoder also touches it).
    let mut named: HashSet<(usize, u8)> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split('\t').nth(1))
        .flat_map(|bits| bits.split(' ').filter_map(|b| b.split_once('.')).map(|(b, n)| (b.parse(), n.parse())).collect::<Vec<_>>())
        .filter_map(|(b, n)| Some((b.ok()?, n.ok()?)))
        .collect();
    let input = typed();
    let list = switches();
    println!("{} controls. Press each as it's named; Enter skips. Writing to {path}.", list.len());
    // The first report sets the state to compare with: what's at rest isn't a press.
    println!("Press and let go of any button to start …");
    say("Press and let go of any button to start");
    while !w.next().is_some_and(|(id, _)| id == 0x01) {}
    let _ = w.gather(Duration::from_millis(800));
    say("Press each control as I name it");

    for (i, name) in list.iter().enumerate().skip(from - 1) {
        print!("{:3}/{}  {name} … ", i + 1, list.len());
        std::io::stdout().flush().ok();
        // Pads say their column and row: "A1" alone is hard to catch.
        let spoken = match name.chars().collect::<Vec<_>>().as_slice() {
            [c, r] if r.is_ascii_digit() => format!("column {c} row {r}"),
            _ => name.split(" (").next().unwrap_or(name).to_string(),
        };
        say(&spoken);
        let mut other = Vec::new();
        let found = loop {
            if input.try_recv().is_ok() {
                break None;
            }
            let Some((id, c)) = w.next() else { continue };
            if id == 0x01 {
                let bits: Vec<(usize, u8)> = rising(&c).into_iter().filter(|b| !named.contains(b)).collect();
                if !bits.is_empty() {
                    named.extend(bits.iter().copied());
                    break Some(bits);
                }
            } else if !c.is_empty() && id != 0x02 {
                // Something other than a strip moved: keep it, for a control that doesn't
                // report in 0x01.
                other.push(format!("{id:#04x}[{}]", c[0].byte));
            }
        };
        let line = match &found {
            Some(bits) => {
                let list: Vec<String> = bits.iter().map(|(b, n)| format!("{b}.{n}")).collect();
                format!("{name}\t{}", list.join(" "))
            }
            None => format!("{name}\tskipped\t{}", other.join(" ")),
        };
        println!("{}", line.split('\t').skip(1).collect::<Vec<_>>().join(" "));
        writeln!(file, "{line}").expect("write");
        file.flush().ok();
        // Let go before the next one.
        if found.is_some() {
            let _ = w.gather(Duration::from_millis(350));
            loop {
                let done = w.last.get(&0x01).is_none_or(|r| {
                    found.as_ref().is_some_and(|bits| bits.iter().all(|(b, n)| r.get(*b).is_none_or(|v| v & (1 << n) == 0)))
                });
                if done {
                    break;
                }
                let _ = w.next();
            }
        }
    }

    // What moves a value rather than a bit: each for six seconds, everything that changed.
    let mut timed = |w: &mut Watch, name: String, ask: String| {
        println!("{ask} (6 seconds) …");
        say(&ask);
        std::thread::sleep(Duration::from_millis(1800));
        let seen = w.gather(Duration::from_secs(6));
        println!("  {}", seen.join("  "));
        writeln!(file, "{name}\t{}", seen.join("  ")).expect("write");
        file.flush().ok();
    };
    timed(&mut w, "encoder clockwise".into(), "Turn the encoder slowly clockwise".into());
    timed(&mut w, "encoder anticlockwise".into(), "Turn the encoder slowly anticlockwise".into());
    for c in COLUMNS {
        timed(&mut w, format!("strip {c} slide"), format!("Slide one finger up and down strip {c}"));
    }
    timed(&mut w, "strip A two fingers".into(), "Hold one finger low on strip A and slide another above it".into());
    timed(&mut w, "strip A hold still".into(), "Hold one finger still on strip A".into());
    timed(&mut w, "footswitch".into(), "If you have a footswitch press it now otherwise wait".into());
    timed(&mut w, "nothing".into(), "Touch nothing".into());
    println!("Done: {path}");
    say("Done");
}
