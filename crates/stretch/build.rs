//! Compile the vendored Signalsmith Stretch (header-only C++) behind `src/shim.cpp`'s
//! C functions. Only a C++14 compiler is needed (no bindgen: `src/lib.rs` declares them).

fn main() {
    println!("cargo::rerun-if-changed=src/shim.cpp");
    println!("cargo::rerun-if-changed=vendor");
    cc::Build::new().cpp(true).std("c++14").file("src/shim.cpp").include("vendor").opt_level(3).warnings(false).compile("deckard_stretch");
}
