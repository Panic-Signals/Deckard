//! Builds Mutable Instruments' DSP (Plaits) (the vendored C++, with `TEST` defined: the desktop
//! build, without the module's hardware) and the C bridge to it.

use std::path::Path;

fn sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("vendor dir").flatten() {
        let p = entry.path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|e| e == "cc") {
            out.push(p);
        }
    }
}

fn main() {
    let vendor = Path::new("vendor");
    let mut files = Vec::new();
    sources(&vendor.join("plaits"), &mut files);
    sources(&vendor.join("stmlib"), &mut files);
    files.sort();
    files.push("src/bridge.cc".into());
    let mut b = cc::Build::new();
    b.cpp(true)
        .std("c++14")
        .define("TEST", None)
        // M_PI and the like (Windows' math.h has them only when asked).
        .define("_USE_MATH_DEFINES", None)
        .include(vendor)
        .files(&files)
        .warnings(false)
        // Audio code: optimized even in debug builds.
        .opt_level(3)
        .debug(false);
    // Plaits' `TEST` mocks (user_data.h) call printf without including <cstdio>; macOS's
    // headers happen to pull it in transitively, Linux's and Windows' do not. Force-include it.
    if b.get_compiler().is_like_msvc() {
        b.flag("/FIcstdio");
    } else {
        b.flag("-include").flag("cstdio");
    }
    b.compile("mi");
    println!("cargo:rerun-if-changed=src/bridge.cc");
    println!("cargo:rerun-if-changed=vendor");
}
