//! Open an ASIO driver as the app does and let it run, printing what it did:
//! `asio-probe <driver name> [frames] [seconds]`. The same example runs on the SDK build
//! (`--features asio`) and the native one (`--features asio-native`), to compare them: the
//! block cadence against what the rate and buffer promise, the latency the driver tells, late
//! blocks, and what it reported. It plays silence (no engine), so it tests the driver path, not
//! the engine's load. It needs the driver free: close other programs using the interface.
use deckard_engine::devices::{self, AudioDriver};
use deckard_engine::output::Output;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(name) = args.first() else {
        eprintln!("usage: asio-probe <ASIO driver name> [frames] [seconds]");
        std::process::exit(2);
    };
    let frames: u32 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(256);
    let seconds: u64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(10);
    devices::set_driver(AudioDriver::Asio);
    println!("built for ASIO: {}", devices::ASIO_BUILT);
    let listed = Instant::now();
    for (n, ch) in devices::output_devices() {
        println!("  {ch:3} ch  {n}");
    }
    println!("listing took {:?}", listed.elapsed());
    let opened = Instant::now();
    let out = match Output::with_device(Some(name), frames, true) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("open failed: {e}");
            std::process::exit(1);
        }
    };
    println!("opened in {:?}: {} at {} Hz, {} channels, {:?}", opened.elapsed(), out.device, out.rate, out.channels(), out.format);
    match devices::input_format(Some(name)) {
        Ok(f) => println!("input: {} at {} Hz, {} channels", f.device, f.rate, f.channels),
        Err(e) => println!("input: {e}"),
    }
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(seconds));
    let elapsed = started.elapsed().as_secs_f64();
    let block = out.block_frames().max(1);
    let expected = elapsed * f64::from(out.rate) / block as f64;
    let blocks = out.stats.callbacks.load(std::sync::atomic::Ordering::Relaxed) as f64;
    println!("block {block} frames, latency {:.2} ms (as the driver says)", out.latency().as_secs_f64() * 1000.0);
    println!("{blocks} blocks in {elapsed:.2} s, {expected:.0} expected ({:.2} %)", blocks / expected * 100.0);
    println!("{}", out.stats.summary());
    println!("asked to reopen: {}", out.take_invalidated());
    drop(out);
    devices::shut_down();
    println!("closed");
}
