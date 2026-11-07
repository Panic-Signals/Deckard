//! Print the audio interfaces as Preferences ▸ Audio lists them, and how each (or the
//! one named on the command line) would be opened: `audio-info [--asio] [device name]`.
//! `--asio` (a build with the `asio` feature) lists the ASIO drivers. That loads each of
//! them in turn, though it opens no stream.
use deckard_engine::devices;
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--asio") {
        args.remove(0);
        devices::set_driver(devices::AudioDriver::Asio);
        println!("through {}", devices::driver().name());
    }
    let name = args.first().cloned();
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
