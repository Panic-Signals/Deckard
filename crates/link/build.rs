//! Builds Ableton Link's C wrapper (`abl_link.cpp`, with Link and asio's headers, all
//! vendored) as a static library. Skipped when the target's OS isn't the host's (the
//! cross-platform type-check, `scripts/check-platforms.sh`, whose C++ toolchain can't
//! build asio's networking for another OS) or with `DECKARD_LINK=off`: the crate then
//! offers a stand-in without the network (`deckard_link::AVAILABLE` is false).

use std::path::Path;

/// The OS a target triple names, as `CARGO_CFG_TARGET_OS` spells it.
fn os_of(triple: &str) -> &str {
    if triple.contains("apple") {
        "macos"
    } else if triple.contains("windows") {
        "windows"
    } else if triple.contains("linux") {
        "linux"
    } else {
        triple
    }
}

fn main() {
    println!("cargo::rustc-check-cfg=cfg(link_native)");
    println!("cargo::rerun-if-changed=vendor");
    println!("cargo::rerun-if-env-changed=DECKARD_LINK");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let host = std::env::var("HOST").unwrap_or_default();
    if os_of(&host) != target_os || std::env::var("DECKARD_LINK").is_ok_and(|v| v == "off") {
        return;
    }
    let vendor = Path::new("vendor");
    let mut b = cc::Build::new();
    b.cpp(true)
        .std("c++17")
        .include(vendor.join("link/include"))
        .include(vendor.join("link/extensions/abl_link/include"))
        .include(vendor.join("asio/include"))
        .file(vendor.join("link/extensions/abl_link/src/abl_link.cpp"))
        .warnings(false);
    match target_os.as_str() {
        "macos" => {
            b.define("LINK_PLATFORM_UNIX", "1").define("LINK_PLATFORM_MACOSX", "1");
        }
        "windows" => {
            // Windows 7 and later (asio asks), without min/max macros.
            b.define("LINK_PLATFORM_WINDOWS", "1").define("_WIN32_WINNT", "0x0601").define("NOMINMAX", None);
            for lib in ["ws2_32", "iphlpapi", "winmm"] {
                println!("cargo::rustc-link-lib={lib}");
            }
        }
        _ => {
            b.define("LINK_PLATFORM_UNIX", "1").define("LINK_PLATFORM_LINUX", "1");
            println!("cargo::rustc-link-lib=pthread");
        }
    }
    b.compile("abl_link");
    println!("cargo::rustc-cfg=link_native");
}
