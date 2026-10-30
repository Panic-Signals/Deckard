//! mxprj-dump: print the container tree and archive header of a Maschine project.
//!
//! Usage: mxprj-dump <file.mxprj> [--decode] [--notes] [--shape] [--hex N]   (N = archive body bytes to hex-dump)

use mxprj::{Item, Project};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: {} <file.mxprj> [--decode] [--notes] [--shape] [--hex N]", args[0]);
        std::process::exit(2);
    };
    let hex = args.iter().position(|a| a == "--hex").and_then(|i| args.get(i + 1)).and_then(|n| n.parse::<usize>().ok());
    let file = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    });
    let project = match Project::parse(&file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    };
    print_item(&project.root, "root", 0);
    println!(
        "archive: {} bytes at {:#x}, boost library version {}, flags {:#x}",
        project.archive.len(),
        project.archive_offset,
        project.header.library_version,
        project.header.flags
    );
    match project.project_header() {
        Ok(h) => println!("project: {h:#?}"),
        Err(e) => println!("project header: {e}"),
    }
    let want_tree = args.iter().any(|a| a == "--decode");
    let want_notes = args.iter().any(|a| a == "--notes");
    let want_shape = args.iter().any(|a| a == "--shape");
    if want_tree || want_notes || want_shape {
        match project.decode(&mxprj::types::Types::new()) {
            Ok(o) => {
                let root = mxprj::decode::Value::Obj(o);
                if want_tree {
                    let mut out = String::new();
                    root.write_tree(&mut out, "root", 0);
                    print!("{out}");
                }
                if want_notes {
                    print_notes(&root);
                }
                if want_shape {
                    let mut seen = std::collections::BTreeMap::new();
                    shape(&root, "", &mut seen);
                    for (path, (ty, n)) in seen {
                        println!("{path}  {ty}  x{n}");
                    }
                }
            }
            Err((e, path)) => println!("decode stopped: {e} in {path}"),
        }
    }
    if let Some(n) = hex {
        let r = project.body();
        let start = r.pos();
        let end = (start + n).min(project.archive.len());
        for (i, chunk) in project.archive[start..end].chunks(16).enumerate() {
            let hexs: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            let text: String = chunk.iter().map(|&b| if b.is_ascii_graphic() { b as char } else { '.' }).collect();
            println!("{:08x}  {:<48} {text}", project.archive_offset + start + i * 16, hexs.join(" "));
        }
    }
}

fn print_item(item: &Item, label: &str, depth: usize) {
    let pad = "  ".repeat(depth);
    let chain: Vec<String> = item.frames.iter().map(|f| format!("{}(v{}, {}B)", f.id, f.version, f.data.len())).collect();
    println!("{pad}{label} @{:#x} frames {}", item.offset, chain.join(" > "));
    for c in &item.children {
        let id = c.item_id.to_be_bytes();
        let id = if id.iter().all(u8::is_ascii_alphanumeric) { String::from_utf8_lossy(&id).into_owned() } else { c.item_id.to_string() };
        let label = format!("{}/{id} tag={}", String::from_utf8_lossy(&c.domain), c.tag);
        print_item(&c.item, &label, depth + 1);
    }
}

/// Every `TimedEvent` holding a `NoteEvent`: position and length in ticks
/// (960 per quarter note), MIDI note, velocity.
fn print_notes(root: &mxprj::decode::Value) {
    use mxprj::decode::Value;
    let mut n = 0;
    root.walk(&mut |o| {
        if o.type_name != "NI::MASCOMMON::DATA::TimedEvent" {
            return;
        }
        let Some(Value::Obj(ev)) = o.get("event") else { return };
        if !ev.type_name.ends_with("NoteEvent") {
            return;
        }
        let get = |k: &str| ev.find(k).and_then(Value::as_int).map_or("?".to_string(), |v| v.to_string());
        let pos = o.get("position").and_then(Value::as_int).unwrap_or(-1);
        println!("note pos={pos:>7} len={:>5} note={:>3} vel={:>3}", get("length"), get("note"), get("velocity"));
        n += 1;
    });
    println!("{n} notes");
}

/// Distinct field paths (list indices collapsed to `[]`) with their type and count.
fn shape(v: &mxprj::decode::Value, path: &str, seen: &mut std::collections::BTreeMap<String, (String, usize)>) {
    use mxprj::decode::Value;
    let ty = match v {
        Value::Obj(o) if !o.type_name.is_empty() => o.type_name.rsplit("::").next().unwrap_or("").to_string(),
        Value::Obj(_) => "{}".into(),
        Value::Int(_) => "int".into(),
        Value::Float(_) => "float".into(),
        Value::Str(_) => "str".into(),
        Value::Bytes(_) => "bytes".into(),
        Value::Null => "null".into(),
        Value::Ref(_) => "ref".into(),
        Value::List(_) => "list".into(),
    };
    let e = seen.entry(path.to_string()).or_insert((ty, 0));
    e.1 += 1;
    match v {
        Value::Obj(o) => {
            for (n, c) in &o.fields {
                // Parameters are leaves for this view.
                if path.matches('.').count() < 14 {
                    shape(c, &format!("{path}.{n}"), seen);
                }
            }
        }
        Value::List(items) => items.iter().for_each(|c| shape(c, &format!("{path}[]"), seen)),
        _ => {}
    }
}
