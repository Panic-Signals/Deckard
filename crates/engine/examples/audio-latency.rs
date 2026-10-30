//! Open the output as the apps do and report the callback block size.
fn main() {
    let out = deckard_engine::output::Output::new().expect("output");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let n = out.block_frames();
    println!("{} Hz, {} frames per callback = {:.1} ms", out.rate, n, n as f64 * 1000.0 / out.rate as f64);
}
