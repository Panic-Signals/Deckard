//! Try the MK3's displays (bulk interface 5) with `deckard_mk3::display`: colour
//! gradients on both screens (every pixel sent as is), then for five seconds a white bar
//! moving down the left screen (only the changed rows sent).
//! NI's NIHostIntegrationAgent holds the interface while it runs; quit it first.
//! `cargo run -p deckard-mk3 --example mk3-display-test`

use deckard_mk3::display::{Displays, HEIGHT, WIDTH, rgb565};
use std::time::{Duration, Instant};

fn gradient(flip: bool) -> Vec<u16> {
    let mut px = vec![0u16; WIDTH * HEIGHT];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let r = (x * 255 / WIDTH) as u8;
            let g = (y * 255 / HEIGHT) as u8;
            px[y * WIDTH + x] = if flip { rgb565(g, r, 255 - r) } else { rgb565(r, g, 128) };
        }
    }
    px
}

fn main() {
    let mut d = match Displays::open() {
        Ok(d) => d,
        Err(e) => return println!("{e}"),
    };
    let left = gradient(false);
    d.show(0, &left).expect("left");
    d.show(1, &gradient(true)).expect("right");
    println!("gradients sent: left red across, green down; right different colours");
    std::thread::sleep(Duration::from_secs(4));
    let start = Instant::now();
    let mut frames = 0;
    while start.elapsed() < Duration::from_secs(5) {
        let y = (start.elapsed().as_millis() as usize / 10) % (HEIGHT - 20);
        let mut px = left.clone();
        for row in y..y + 20 {
            px[row * WIDTH..(row + 1) * WIDTH].fill(rgb565(255, 255, 255));
        }
        d.show(0, &px).expect("bar");
        frames += 1;
    }
    println!("moving bar: {frames} frames in 5 s");
}
