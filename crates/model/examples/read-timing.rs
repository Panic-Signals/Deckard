use std::time::Instant;
fn main() {
    let path = std::env::args().nth(1).expect("project");
    let bytes = std::fs::read(&path).unwrap();
    let types = mxprj::types::Types::new();
    let doc = mxprj::Project::parse(&bytes).unwrap();
    let root = doc.decode(&types).unwrap();
    for _ in 0..3 {
        let t = Instant::now();
        let ix = deckard_model::Index::of(&root);
        let t_ix = t.elapsed();
        drop(ix);
        let t = Instant::now();
        let mut n = 0;
        root.walk_paths(&mut |_, _| n += 1);
        let t_walk = t.elapsed();
        let t = Instant::now();
        let _ = deckard_model::Project::read(&root);
        let t_read = t.elapsed();
        let t = Instant::now();
        let c = root.clone();
        let t_clone = t.elapsed();
        drop(c);
        println!("objects {n}: index {t_ix:?}, walk {t_walk:?}, read {t_read:?}, clone {t_clone:?}");
    }
}
