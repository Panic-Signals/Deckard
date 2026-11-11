//! Focused controller regressions and an opt-in measurement on the real USB gear.
use crate::App;
use std::time::{Duration, Instant};

fn app() -> App {
    let mut app = App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    while doc.borrow().project().groups.len() < 8 {
        doc.borrow_mut().add_group().unwrap();
    }
    app
}

#[test]
fn mk3_group_led_follows_the_press_in_the_same_pass() {
    let mut app = app();
    let out = app.mk3_events(vec![deckard_mk3::Event::Button("BUTTON_GROUP_B", true)], &mut Vec::new());
    assert_eq!(app.group, 1);
    assert_eq!(out.leds.get("LED_GROUP_B").unwrap() & 3, 3);
    assert_ne!(out.leds.get("LED_GROUP_A").unwrap() & 3, 3);
}

fn summary(values: &[f64]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |p: f64| sorted[((sorted.len() - 1) as f64 * p).round() as usize];
    serde_json::json!({"samples":sorted.len(),"median_ms":at(0.5),"p95_ms":at(0.95),"p99_ms":at(0.99),"max_ms":at(1.)})
}

fn await_write(mut applied: impl FnMut() -> Option<Instant>, after: Instant) -> Instant {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(at) = applied().filter(|at| *at >= after) {
            return at;
        }
        assert!(Instant::now() < until, "controller did not acknowledge its LED write");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
#[ignore = "opens real MK3/JAM; all performance applications must be closed"]
fn profile_real_mk3_jam_led_delivery() {
    let path = std::env::var_os("DECKARD_CONTROLLER_PROFILE").map(std::path::PathBuf::from);
    let mut app = app();
    app.surface = deckard_surface::Surface::with_user_dir(path.clone());
    let doc = app.doc.clone().unwrap();
    for group in 0..8 {
        for sound in 0..if group < 2 { 16 } else { 1 } {
            let module = if group < 2 {
                deckard_doc::NewModule::Instrument { class: "Kick".into(), params: Vec::new() }
            } else {
                deckard_doc::NewModule::Sampler
            };
            doc.borrow_mut().add_slot(group, sound, 0, &module).unwrap();
        }
    }
    let view = app.surface_view();
    let mut jam = deckard_surface::device::Device::new(crate::jam_io::DEVICE, path);
    jam.open(&view, Instant::now());
    app.jam_device = Some(jam);

    struct Hardware(crate::mk3_io::Mk3Io, crate::jam_io::JamIo);
    impl Drop for Hardware {
        fn drop(&mut self) {
            self.0.shut_down(Vec::new());
            self.1.shut_down();
        }
    }
    let hw = Hardware(crate::mk3_io::Mk3Io::start(), crate::jam_io::JamIo::start());
    hw.1.set_enabled(true);
    hw.0.set_display_brightness(80);
    let mut mk3_cpu = Vec::new();
    let mut mk3_wire = Vec::new();
    let mut jam_cpu = Vec::new();
    let mut jam_wire = Vec::new();
    let mut idle_cpu = Vec::new();
    for frame in 0..72 {
        let group = frame % 2;
        let name = if group == 0 { "BUTTON_GROUP_A" } else { "BUTTON_GROUP_B" };
        let start = Instant::now();
        let out = app.mk3_events(vec![deckard_mk3::Event::Button(name, true), deckard_mk3::Event::Button(name, false)], &mut Vec::new());
        assert_eq!(app.group, group);
        let led = if group == 0 { "LED_GROUP_A" } else { "LED_GROUP_B" };
        assert_eq!(out.leds.get(led).unwrap() & 3, 3, "same-pass MK3 LED");
        let cpu = start.elapsed().as_secs_f64() * 1000.;
        hw.0.set_leds(out.leds.clone());
        if let Some((n, frame)) = out.screens {
            hw.0.set_screens(n, frame);
        }
        let applied = await_write(|| hw.0.leds_applied_at(&out.leds), start);
        if frame >= 8 {
            mk3_cpu.push(cpu);
            mk3_wire.push(applied.duration_since(start).as_secs_f64() * 1000.);
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    for frame in 0..72 {
        let group = frame % 2;
        let name = if group == 0 { "GROUP_A" } else { "GROUP_B" };
        let start = Instant::now();
        let (leds, _) = app.jam_events(vec![deckard_jam::Event::Button(name, true), deckard_jam::Event::Button(name, false)], start);
        assert_eq!(app.group, group);
        let cpu = start.elapsed().as_secs_f64() * 1000.;
        hw.1.set_leds(leds.clone());
        let applied = await_write(|| hw.1.leds_applied_at(&leds), start);
        if frame >= 8 {
            jam_cpu.push(cpu);
            jam_wire.push(applied.duration_since(start).as_secs_f64() * 1000.);
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    for _ in 0..64 {
        let start = Instant::now();
        app.mk3_events(Vec::new(), &mut Vec::new());
        app.jam_events(Vec::new(), start);
        idle_cpu.push(start.elapsed().as_secs_f64() * 1000.);
    }
    assert_eq!(app.surface.error(), None);
    assert_eq!(app.jam_error, None);
    let report = serde_json::json!({
        "fixture":"8 groups, 32 native drum synths, 6 sampler slots; alternating group focus; MK3 screens update concurrently",
        "measurement":"synthetic input dispatch through production app/scripts/IO threads to completion of actual USB HID writes; excludes physical input scan and optical LED response",
        "mk3_cpu":summary(&mk3_cpu),"mk3_dispatch_to_usb":summary(&mk3_wire),
        "jam_cpu":summary(&jam_cpu),"jam_dispatch_to_usb":summary(&jam_wire),
        "combined_idle_cpu":summary(&idle_cpu),
        "raw_ms":{"mk3_cpu":mk3_cpu,"mk3_dispatch_to_usb":mk3_wire,"jam_cpu":jam_cpu,"jam_dispatch_to_usb":jam_wire,"combined_idle_cpu":idle_cpu}
    });
    let out = std::env::var_os("DECKARD_PROFILE_OUT").expect("profile output path");
    std::fs::write(out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("{report}");
}
