//! Query an NI index: `query <komplete.db3> <Kind> [type …]`, e.g. `query copy.db3 Loop Drums`.
//! Point it at a copy of the index.

use deckard_library::{Kind, Query, ni::NiIndex};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let kind = Kind::ALL.into_iter().find(|k| k.table() == a[1]).expect("a kind: Project, Group, Sound, Instrument, Effect, Loop, Oneshot");
    let ni = NiIndex::open(std::path::Path::new(&a[0])).unwrap();
    let t = std::time::Instant::now();
    let r = ni.query(&Query { kind, types: a[2..].to_vec(), ..Query::default() }).unwrap();
    println!("{} items in {:?}", r.items.len(), t.elapsed());
    println!("types: {:?}", &r.types[..r.types.len().min(12)]);
    println!("characters: {:?}", &r.characters[..r.characters.len().min(12)]);
    println!("products: {:?}", r.products);
    for i in r.items.iter().take(4) {
        println!("  {} | {:?} | {:?} | {:?} | {}", i.name, i.product, i.types, i.tempo, i.path.display());
    }
}
