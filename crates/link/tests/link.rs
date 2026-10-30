//! Link instances: alone, and two in one process meeting on the network.

use deckard_link::Link;
use std::time::{Duration, Instant};

/// Alone, an instance keeps the tempo it was made with (and one set later), finds no
/// peers, and its beats advance at its tempo.
#[test]
fn alone() {
    let link = Link::new(120.0);
    assert!(!link.is_enabled());
    assert_eq!(link.peers(), 0);
    assert!((link.tempo() - 120.0).abs() < 1e-9);
    link.set_tempo(98.5);
    assert!((link.tempo() - 98.5).abs() < 1e-9);
    link.set_start_stop_sync(true);
    assert!(link.start_stop_sync());
    // Its beats at 98.5 BPM: about 0.164 of a beat in 100 ms; measured against the time
    // that actually passed, since a loaded machine oversleeps.
    let mut clock = link.audio_clock();
    let t = Instant::now();
    let a = clock.at(0, 4.0);
    std::thread::sleep(Duration::from_millis(100));
    let b = clock.at(0, 4.0);
    let slept = t.elapsed().as_secs_f64();
    assert!((b.tempo - 98.5).abs() < 1e-9);
    let advanced = b.beat - a.beat;
    let at_most = slept * 98.5 / 60.0;
    assert!(advanced > 0.12 && advanced <= at_most + 0.01, "advanced {advanced} beats in {slept} s");
    // Looking ahead: 1 s on is 98.5 / 60 beats further.
    let c = clock.at(1_000_000, 4.0);
    assert!((c.beat - clock.at(0, 4.0).beat - 98.5 / 60.0).abs() < 0.01);
    // The phase is the beat within the quantum.
    let p = link.phase(4.0);
    assert!((0.0..4.0).contains(&p));
    assert!(!link.is_playing());
    link.set_playing(true);
    assert!(link.is_playing());
    assert!(clock.at(0, 4.0).playing);
}

/// The session of one for tests: enabled, it still finds nobody.
#[test]
fn local_never_meets() {
    let a = Link::local(120.0);
    a.enable(true);
    assert!(a.is_enabled());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(a.peers(), 0);
}

/// Two instances in one process (two applications, as far as Link is concerned) find
/// each other once enabled, and a tempo set in one reaches the other. Only with
/// `DECKARD_LINK_NETWORK_TESTS=1`: enabled, they'd join any Link session on the network
/// (someone's music) and change its tempo. Skipped where the network doesn't let them
/// meet (no interface, a firewall).
#[test]
fn two_meet() {
    if !deckard_link::AVAILABLE || std::env::var_os("DECKARD_LINK_NETWORK_TESTS").is_none() {
        eprintln!("skipped: needs Link built and DECKARD_LINK_NETWORK_TESTS=1");
        return;
    }
    let a = Link::new(120.0);
    let b = Link::new(90.0);
    a.enable(true);
    b.enable(true);
    assert!(a.is_enabled());
    let deadline = Instant::now() + Duration::from_secs(5);
    while (a.peers() == 0 || b.peers() == 0) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    if a.peers() == 0 {
        eprintln!("skipped: the two instances didn't meet (no network?)");
        return;
    }
    assert_eq!(b.peers(), 1);
    // (Link carries a tempo as microseconds per beat.)
    a.set_tempo(133.0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while (b.tempo() - 133.0).abs() > 1e-3 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!((b.tempo() - 133.0).abs() < 1e-3, "b's tempo: {}", b.tempo());
    a.enable(false);
    b.enable(false);
}
