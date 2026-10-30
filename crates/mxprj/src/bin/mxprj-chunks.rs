//! mxprj-chunks: list the plug-ins a Maschine document hosts (PluginHost modules), and
//! optionally write each one's saved state (`chunk`) to a file.
//!
//! Usage: mxprj-chunks <file> [--out DIR]

use mxprj::Project;
use mxprj::decode::Value;
use mxprj::types::Types;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: mxprj-chunks <file> [--out DIR]");
        std::process::exit(2);
    };
    let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1));
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = Project::parse(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
    let root = p.decode(&Types::new()).unwrap_or_else(|(e, at)| panic!("{path}: {e} in {at}"));
    let mut n = 0;
    root.walk_paths(&mut |o, _| {
        if o.type_name != "NI::MASCHINE::DATA::PluginHost" {
            return;
        }
        let id = match o.find("plugin_id") {
            Some(Value::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let name = match o.find("plugin_name") {
            Some(Value::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let data = match o.find("chunk") {
            Some(Value::Obj(c)) => match c.find("size_data") {
                Some(Value::Bytes(b)) => b.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        let head: String = data.iter().take(16).map(|b| format!("{b:02x}")).collect();
        let ascii: String = data.iter().take(16).map(|&b| if b.is_ascii_graphic() { b as char } else { '.' }).collect();
        println!("#{n} {name:?} {id} {} bytes  {head} {ascii}", data.len());
        if let Some(dir) = out {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(format!("{dir}/{n}-{}.bin", name.replace(' ', "_")), &data).unwrap();
        }
        n += 1;
    });
}
