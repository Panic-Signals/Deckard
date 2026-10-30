//! mxprj-set: change a project's tempo and/or name and write it to a new file.
//!
//! Usage: mxprj-set <in.mxprj> <out.mxprj> [--tempo BPM] [--name NAME]
//! The input is never modified.

use mxprj::Project;
use mxprj::decode::Value;
use mxprj::types::Types;

/// Set `field.value` on every object of type `owner`; returns how many were set.
fn set_param(root: &mut Value, owner: &str, field: &str, value: &Value) -> usize {
    let mut n = 0;
    root.walk_mut(&mut |o| {
        if o.type_name == owner
            && let Some(Value::Obj(p)) = o.find_mut(field)
            && let Some(v) = p.find_mut("value")
        {
            *v = value.clone();
            n += 1;
        }
    });
    n
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: mxprj-set <in.mxprj> <out.mxprj> [--tempo BPM] [--name NAME]");
        std::process::exit(2);
    }
    let (input, output) = (&args[0], &args[1]);
    if std::fs::canonicalize(input).ok() == std::fs::canonicalize(output).ok() {
        eprintln!("refusing to overwrite the input");
        std::process::exit(2);
    }
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let bytes = std::fs::read(input).expect("read input");
    let types = Types::new();
    let doc = Project::parse(&bytes).expect("parse");
    let mut tree = Value::Obj(doc.decode(&types).unwrap_or_else(|(e, at)| panic!("decode: {e} in {at}")));
    if let Some(t) = flag("--tempo") {
        let n = set_param(&mut tree, "NI::MASCHINE::DATA::Song", "tempo", &Value::Float(t.parse().expect("--tempo BPM")));
        eprintln!("tempo set on {n} song(s)");
    }
    if let Some(name) = flag("--name") {
        let n = set_param(&mut tree, "NI::MASCHINE::DATA::Project", "name", &Value::Str(name));
        eprintln!("name set on {n} project(s)");
    }
    let Value::Obj(root) = tree else { unreachable!() };
    let out = doc.encode(&types, root).unwrap_or_else(|(e, at)| panic!("encode: {e} in {at}"));
    std::fs::write(output, out).expect("write output");
}
