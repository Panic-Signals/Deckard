//! Windows: gives the program its icon (Explorer and shortcuts read it from the file).

fn main() {
    println!("cargo:rerun-if-changed=assets/deckard.rc");
    println!("cargo:rerun-if-changed=assets/deckard.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/deckard.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
