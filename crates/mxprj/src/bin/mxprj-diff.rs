//! mxprj-diff: what changed between two versions of a Maschine document, as tree
//! paths. Object ids are matched up as the trees are walked (an inserted object
//! renumbers every later one), and list items are aligned by content, so an insert
//! shows as one `+` line rather than a changed tail.
//!
//! Usage: mxprj-diff <before> <after> [--max N]

use mxprj::Project;
use mxprj::decode::{Obj, Value};
use mxprj::types::Types;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

struct Diff {
    /// before oid -> after oid, for objects matched so far.
    oids: HashMap<u64, u64>,
    out: Vec<String>,
}

/// Content hash ignoring object ids (references hash as "a reference").
fn fingerprint(v: &Value, h: &mut impl Hasher) {
    match v {
        Value::Int(i) => (0u8, i).hash(h),
        Value::Float(f) => (1u8, f.to_bits()).hash(h),
        Value::Str(s) => (2u8, s).hash(h),
        Value::Bytes(b) => (3u8, b).hash(h),
        Value::Null => 4u8.hash(h),
        Value::Ref(_) => 5u8.hash(h),
        Value::List(items) => {
            (6u8, items.len()).hash(h);
            items.iter().for_each(|i| fingerprint(i, h));
        }
        Value::Obj(o) => {
            (7u8, &o.type_name, o.version).hash(h);
            for (n, v) in &o.fields {
                n.hash(h);
                fingerprint(v, h);
            }
        }
    }
}

fn hash(v: &Value) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    fingerprint(v, &mut h);
    h.finish()
}

/// A short description of a value for `+`/`-` lines.
fn describe(v: &Value) -> String {
    match v {
        Value::Obj(o) => {
            let name = o.find("name").and_then(|n| match n {
                Value::Obj(p) => match p.find("value") {
                    Some(Value::Str(s)) => Some(s.clone()),
                    _ => None,
                },
                _ => None,
            });
            let ty = if o.type_name.is_empty() { "{..}".to_string() } else { short(&o.type_name) };
            let oid = o.oid.map_or(String::new(), |i| format!(" #{i}"));
            match name {
                Some(n) => format!("{ty}{oid} {n:?}"),
                None => format!("{ty}{oid}"),
            }
        }
        Value::List(l) => format!("[{} items]", l.len()),
        Value::Str(s) => format!("{s:?}"),
        Value::Ref(o) => format!("-> #{o}"),
        Value::Bytes(b) => format!("<{} bytes>", b.len()),
        v => format!("{v:?}"),
    }
}

fn short(t: &str) -> String {
    t.replace("NI::MASCHINE::DATA::", "").replace("NI::MASCOMMON::DATA::", "").replace("NI::ML::TM::", "")
}

impl Diff {
    fn value(&mut self, path: &str, a: &Value, b: &Value) {
        match (a, b) {
            (Value::Obj(x), Value::Obj(y)) if x.type_name == y.type_name => self.obj(path, x, y),
            (Value::List(x), Value::List(y)) => self.list(path, x, y),
            (Value::Ref(x), Value::Ref(y)) => {
                if self.oids.get(x) != Some(y) {
                    let was = self.oids.get(x).map_or(format!("#{x} (unmatched)"), |m| format!("#{m}"));
                    self.out.push(format!("~ {path}: reference -> {was} now -> #{y}"));
                }
            }
            (Value::Float(x), Value::Float(y)) if x.to_bits() == y.to_bits() => {}
            (x, y) if x == y => {}
            (x, y) => self.out.push(format!("~ {path}: {} => {}", describe(x), describe(y))),
        }
    }

    fn obj(&mut self, path: &str, a: &Obj, b: &Obj) {
        if let (Some(x), Some(y)) = (a.oid, b.oid) {
            self.oids.insert(x, y);
        }
        if a.version != b.version {
            self.out.push(format!("~ {path}: version {} => {}", a.version, b.version));
        }
        // Fields by name, in order (same type, so the same schema; counts can differ).
        let mut bi = 0;
        for (n, va) in &a.fields {
            match b.fields[bi..].iter().position(|(m, _)| m == n) {
                Some(k) => {
                    for (m, vb) in &b.fields[bi..bi + k] {
                        self.out.push(format!("+ {path}.{m}: {}", describe(vb)));
                    }
                    self.value(&format!("{path}.{n}"), va, &b.fields[bi + k].1);
                    bi += k + 1;
                }
                None => self.out.push(format!("- {path}.{n}: {}", describe(va))),
            }
        }
        for (m, vb) in &b.fields[bi..] {
            self.out.push(format!("+ {path}.{m}: {}", describe(vb)));
        }
    }

    fn list(&mut self, path: &str, a: &[Value], b: &[Value]) {
        if a.len() == b.len() {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                self.value(&format!("{path}[{i}]"), x, y);
            }
            return;
        }
        // Align by content: longest common subsequence of fingerprints, then pair up
        // the unmatched runs in order (changed items) and report the rest.
        let (ha, hb): (Vec<u64>, Vec<u64>) = (a.iter().map(hash).collect(), b.iter().map(hash).collect());
        let (n, m) = (a.len(), b.len());
        let mut lcs = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i][j] = if ha[i] == hb[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        let mut run_a = Vec::new();
        let mut run_b = Vec::new();
        let flush = |d: &mut Diff, run_a: &mut Vec<usize>, run_b: &mut Vec<usize>| {
            let k = run_a.len().min(run_b.len());
            for t in 0..k {
                d.value(&format!("{path}[{}->{}]", run_a[t], run_b[t]), &a[run_a[t]], &b[run_b[t]]);
            }
            for &x in &run_a[k..] {
                d.out.push(format!("- {path}[{x}]: {}", describe(&a[x])));
            }
            for &y in &run_b[k..] {
                d.out.push(format!("+ {path}[{y}]: {}", describe(&b[y])));
            }
            run_a.clear();
            run_b.clear();
        };
        while i < n || j < m {
            if i < n && j < m && ha[i] == hb[j] {
                flush(self, &mut run_a, &mut run_b);
                self.value(&format!("{path}[{i}->{j}]"), &a[i], &b[j]);
                i += 1;
                j += 1;
            } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
                run_b.push(j);
                j += 1;
            } else {
                run_a.push(i);
                i += 1;
            }
        }
        flush(self, &mut run_a, &mut run_b);
    }
}

fn load(path: &str, types: &Types) -> Obj {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = Project::parse(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
    p.decode(types).unwrap_or_else(|(e, at)| panic!("{path}: {e} in {at}"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: mxprj-diff <before> <after> [--max N]");
        std::process::exit(2);
    }
    let max = args.iter().position(|a| a == "--max").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(200);
    let types = Types::new();
    let (a, b) = (load(&args[0], &types), load(&args[1], &types));
    let mut d = Diff { oids: HashMap::new(), out: Vec::new() };
    d.obj("root", &a, &b);
    for l in d.out.iter().take(max) {
        println!("{l}");
    }
    if d.out.len() > max {
        println!("... {} more", d.out.len() - max);
    }
    eprintln!("{} differences", d.out.len());
}
