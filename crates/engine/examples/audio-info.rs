//! Print the audio interfaces as Preferences ▸ Audio lists them, and how each (or the
//! one named on the command line) would be opened: `audio-info [device name]`.
use deckard_engine::devices;
fn main() {
    let name = std::env::args().nth(1);
    for (dir, list) in [("out", devices::output_devices()), ("in", devices::input_devices())] {
        println!("--- {dir}puts ---");
        for (n, ch) in &list {
            println!("{ch:3} ch  {n}");
        }
        let open = |n: Option<&str>| match if dir == "out" { devices::open_output(n) } else { devices::open_input(n) } {
            Ok((d, cfg)) => format!(
                "{}: rate {} channels {} format {:?} buffer {:?}",
                devices::device_name(&d),
                cfg.sample_rate(),
                cfg.channels(),
                cfg.sample_format(),
                cfg.buffer_size()
            ),
            Err(e) => format!("error: {e}"),
        };
        println!("default: {}", open(None));
        if let Some(n) = &name {
            println!("\"{n}\": {}", open(Some(n)));
        }
    }
}
