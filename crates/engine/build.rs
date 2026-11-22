// `native_asio`: this build plays through ASIO with transit's backend (no SDK). It's the
// `asio-native` feature on x86_64 Windows, unless `asio` (cpal over the real SDK) is on too.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(native_asio)");
    println!("cargo::rerun-if-changed=build.rs");
    let feature = |name: &str| std::env::var_os(format!("CARGO_FEATURE_{name}")).is_some();
    let target = |name: &str| std::env::var(format!("CARGO_CFG_TARGET_{name}")).unwrap_or_default();
    if feature("ASIO_NATIVE") && !feature("ASIO") && target("OS") == "windows" && target("ARCH") == "x86_64" {
        println!("cargo::rustc-cfg=native_asio");
    }
}
