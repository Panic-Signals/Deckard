//! lua-host-run: boot the controller scripts and report what happened.
//!
//! Usage: lua-host-run <scripts-dir> [--mod DIR] [--controller M3] [--project F.mxprj]
//!                     [--ticks N] [--top N] [--press BUTTON_X]... [--hold BUTTON_X]... [--pad N]...
//!                     [--eval LUA]... [--screen ROOT]... [--encoder NAME:INC]... [--wheel STEPS]...
//!                     [--png FILE]... [--levels L,R]...
//!                     [--compare REF.png:OUT.png]...
//! Presses, pad hits and evals run after boot in order; presses and pads are each
//! followed by a tick. `--hold` presses a button (or touches a knob, BUTTON_CAP_N) and
//! keeps it down. `--eval` prints what the chunk returns.
//! `--project new` starts from a new, empty project, as Maschine does.
//! `--levels L,R` sets the recorder's input levels (linear), as the app does each frame.
//! `--browser A,B,C` gives BROWSE a library: Loops, two products, types and these
//! results.
//! `--pictures DIR` adds DECKARD's plug-in pictures from DIR (`<key>_Thumb.png`,
//! `<key>_Logo.png`; the app's `assets/plugins`).
//! `--plugins` gives the host a sample plug-in list and a sound with one plug-in, as the
//! app would (the Control page's chain and the plug-in browser).
//! `--compare` renders the screens like `--png` and writes OUT.png: the reference (e.g.
//! captured from NI's software, see `docs/mk3-usb.md`), ours, and where they differ in
//! red, stacked; it prints the share of pixels that differ.
//! `<scripts-dir>` is the `ni-res-extract` output from your own Maschine install.

use deckard_lua_host::state::ControlRequest;
use deckard_lua_host::{Host, Scripts, State};
use deckard_nwl::draw::Canvas;
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut dir = None;
    let mut mods = Vec::new();
    let mut controller = "M3".to_string();
    let mut project = None;
    let mut ticks = 10;
    let mut top = 40;
    let mut plugins = false;
    let mut pictures: Option<PathBuf> = None;
    let mut actions = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--mod" => mods.push(PathBuf::from(args.next().expect("--mod DIR"))),
            "--controller" => controller = args.next().expect("--controller NAME"),
            "--project" => project = Some(PathBuf::from(args.next().expect("--project FILE"))),
            "--ticks" => ticks = args.next().and_then(|s| s.parse().ok()).expect("--ticks N"),
            "--press" | "--hold" | "--pad" | "--eval" | "--screen" | "--encoder" | "--wheel" | "--png" | "--compare" | "--levels"
            | "--sample" | "--browser" => actions.push((a.clone(), args.next().expect("argument"))),
            "--top" => top = args.next().and_then(|s| s.parse().ok()).expect("--top N"),
            "--plugins" => plugins = true,
            "--pictures" => pictures = Some(PathBuf::from(args.next().expect("--pictures DIR"))),
            _ => dir = Some(PathBuf::from(a)),
        }
    }
    let Some(dir) = dir else {
        eprintln!("usage: lua-host-run <scripts-dir> [--mod DIR] [--controller M3] [--project F.mxprj] [--ticks N] [--sample FRAMES] …");
        std::process::exit(2);
    };
    let mut scripts = Scripts::from_dir(&dir).expect("read scripts");
    for m in &mods {
        let r = scripts.add_mod(m).expect("read mod");
        eprintln!("mod {}: {} replaced, applied {:?}", m.display(), r.replaced, r.applied);
        for s in &r.skipped {
            eprintln!("  skipped: {s}");
        }
        for n in &r.native {
            eprintln!("  not applicable to DECKARD: {n}");
        }
    }
    eprintln!("{} scripts", scripts.len());
    let project = project.map(|p| {
        let doc = if p.as_os_str() == "new" { deckard_doc::Document::new_project() } else { deckard_doc::Document::open(&p) };
        doc.unwrap_or_else(|e| panic!("{e}")).shared()
    });
    let mut host = Host::new(scripts, State::new(&controller, project)).expect("host");
    let look = std::rc::Rc::new(deckard_nwl::draw::Look::mk3(&dir).unwrap_or_else(|e| panic!("{e}")));
    if let Some(dir) = &pictures {
        for entry in std::fs::read_dir(dir).expect("--pictures DIR").flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let parts = name.strip_suffix("_Thumb.png").map(|k| (k, true)).or_else(|| name.strip_suffix("_Logo.png").map(|k| (k, false)));
            if let (Some((key, thumb)), Ok(png)) = (parts, std::fs::read(entry.path())) {
                look.add_picture(&deckard_lua_host::plugin_picture_path(key, thumb), &png);
            }
        }
    }
    host.set_look(look.clone());
    // The displays as the hardware shows them (both, 960 × 272): the page, then the overlay.
    let draw = |host: &Host| {
        let (page, overlay) = host.display()?;
        let mut canvas = look.render(&page, 960, 272);
        if let Some(o) = overlay {
            look.render_over(&mut canvas, &o);
        }
        Some(canvas)
    };
    if plugins {
        sample_plugins(&mut host);
    }
    if let Err(e) = host.boot() {
        eprintln!("boot failed: {e}");
        std::process::exit(1);
    }
    eprintln!("booted {controller}");
    for _ in 0..ticks {
        host.tick();
    }
    for (kind, arg) in &actions {
        if kind == "--screen" {
            // Outline of a display root by accessor name, e.g. `PageStack` or `OverlayRoot`.
            let st = host.state.borrow();
            match st.hw.display_roots.get(arg.as_str()) {
                Some(&root) => print!("{}", st.widgets.outline(root)),
                None => eprintln!("no display root {arg}; have {:?}", st.hw.display_roots.keys().collect::<Vec<_>>()),
            }
            continue;
        }
        // A made-up sample for the Sampling page's EDIT, SLICE and ZONE screens: a
        // decaying tone in four slices, played from a fifth in, mapped to two zones.
        if kind == "--browser" {
            host.set_browser(deckard_lua_host::state::BrowserView {
                kind: 5,
                products: vec!["Deep Matter".into(), "Lilac Glare".into()],
                attributes: [
                    (vec!["Bass".into(), "Drums".into(), "Synth".into()], Some(1)),
                    (vec!["Kick".into(), "Snare".into()], None),
                    (vec!["Clean".into()], None),
                ],
                results: arg.split(',').map(|n| (n.to_string(), false)).collect(),
                ..Default::default()
            });
            host.tick();
            continue;
        }
        if kind == "--sample" {
            let frames: usize = arg.parse().expect("--sample FRAMES");
            let peaks: Vec<(f32, f32)> = (0..480)
                .map(|i| {
                    let a = (-(i % 120) as f32 / 40.0).exp() * (0.4 + 0.5 * (i as f32 / 7.0).sin().abs());
                    (-a, a)
                })
                .collect();
            host.state.borrow_mut().sample = Some(deckard_lua_host::state::SampleView {
                name: "Made Up.wav".into(),
                frames,
                rate: 44_100,
                overview: peaks.clone(),
                detail: peaks,
                view: (0.0, frames as f64),
                play: (frames / 5, frames),
                slices: (0..4).map(|k| k * frames / 4).collect(),
                focus_slice: Some(1),
                zones: vec![((0, 59), (0, 127)), ((60, 72), (0, 100))],
                focus_zone: 1,
                color: Some(4),
                ..Default::default()
            });
            for _ in 0..2 {
                host.tick();
            }
            continue;
        }
        if kind == "--levels" {
            let (l, r) = arg.split_once(',').expect("--levels L,R");
            host.state.borrow_mut().record.levels = (l.parse().expect("level"), r.parse().expect("level"));
            for _ in 0..2 {
                host.tick();
            }
            continue;
        }
        if kind == "--png" {
            let t = std::time::Instant::now();
            match draw(&host) {
                Some(c) => {
                    eprintln!("render: {:?}", t.elapsed());
                    c.write_png(std::path::Path::new(arg)).unwrap_or_else(|e| panic!("{e}"));
                    eprintln!("wrote {arg}");
                }
                None => eprintln!("no active screen"),
            }
            continue;
        }
        if kind == "--compare" {
            let (reference, out) = arg.split_once(':').expect("--compare REF.png:OUT.png");
            let Some(ours) = draw(&host) else {
                eprintln!("no active screen");
                continue;
            };
            let theirs = Canvas::read_png(std::path::Path::new(reference)).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!((theirs.width, theirs.height), (960, 272), "reference size");
            let mut sheet = Canvas::new(960, 272 * 3, [0, 0, 0, 255]);
            let mut differ = 0;
            for (i, (a, b)) in theirs.px.iter().zip(&ours.px).enumerate() {
                sheet.px[i] = *a;
                sheet.px[960 * 272 + i] = *b;
                // The reference went through RGB565: allow its rounding.
                let off = (0..3).any(|c| a[c].abs_diff(b[c]) > 12);
                differ += off as usize;
                let grey = (b[0] as u16 + b[1] as u16 + b[2] as u16) / 9;
                sheet.px[960 * 272 * 2 + i] = if off { [255, 40, 40, 255] } else { [grey as u8, grey as u8, grey as u8, 255] };
            }
            sheet.write_png(std::path::Path::new(out)).unwrap_or_else(|e| panic!("{e}"));
            println!("{reference}: {:.2}% of pixels differ; wrote {out}", differ as f64 * 100.0 / (960.0 * 272.0));
            continue;
        }
        if kind == "--eval" {
            match host.lua.load(arg.as_str()).eval::<mlua::MultiValue>() {
                Ok(v) => println!("{}", v.iter().map(|x| format!("{x:?}")).collect::<Vec<_>>().join("\t")),
                Err(e) => eprintln!("eval: {e}"),
            }
            continue;
        }
        if kind == "--wheel" {
            // The 4-D encoder turned STEPS clicks (negative: left).
            host.wheel(arg.parse().expect("--wheel STEPS"));
            host.tick();
            continue;
        }
        if kind == "--encoder" {
            // NAME:INC, e.g. ENCODER_SCREEN_1:0.05
            let (name, inc) = arg.split_once(':').expect("--encoder NAME:INC");
            if let Err(e) = host.encoder(name, inc.parse().expect("increment")) {
                eprintln!("{arg}: {e}");
            }
        } else if kind == "--press" || kind == "--hold" {
            let done = if kind == "--press" { host.press(arg) } else { host.button(arg, true) };
            if let Err(e) = done {
                eprintln!("{arg}: {e}");
            }
        } else {
            host.pad(arg.parse().expect("pad index"), 1.0);
        }
        host.tick();
        // What the app's Control area would do for these (the rest needs the app).
        {
            let mut st = host.state.borrow_mut();
            for r in std::mem::take(&mut st.control_requests) {
                match r {
                    ControlRequest::Level(l) => st.control.level = l,
                    ControlRequest::ModulesVisible(v) => st.control.modules_visible = v,
                    ControlRequest::PageGroup(g) => st.control.page_group = g,
                    _ => {}
                }
            }
        }
        host.tick();
        let st = host.state.borrow();
        let lit: Vec<String> = st
            .hw
            .leds
            .iter()
            .filter(|(_, l)| l.is_lit())
            .map(|(id, _)| host.constant_name("NI.HW.LED_", *id).unwrap_or(id.to_string()).replace("NI.HW.LED_", ""))
            .collect();
        eprintln!(
            "after {kind} {arg}: playing={} focus={:?} pages={:?}\n  LEDs lit: {}",
            st.transport.playing,
            st.focus,
            st.hw.page_stack.iter().map(|p| host.constant_name("NI.HW.PAGE_", *p).unwrap_or(p.to_string())).collect::<Vec<_>>(),
            lit.join(" ")
        );
    }
    for e in host.errors.iter().take(10) {
        eprintln!("error: {e}");
    }
    let report = host.report();
    let calls = report.iter().filter(|r| r.1 != "const").count();
    eprintln!("{} errors, {} stubbed API paths, {} constants", host.errors.len(), calls, report.len() - calls);
    for (path, kind, n) in report.iter().filter(|r| r.1 != "const").take(top) {
        println!("{n:8} {kind:5} {path}");
    }
    let st = host.state.borrow();
    eprintln!(
        "LEDs lit: {}, page stack: {:?}, log lines: {}",
        st.hw.leds.values().filter(|l| l.is_lit()).count(),
        st.hw.page_stack,
        st.log.len()
    );
}

/// A plug-in list and a chain like the app's: Maschine's modules, DECKARD's, a few
/// external plug-ins; the focused sound has a Kick.
fn sample_plugins(host: &mut Host) {
    use deckard_lua_host::state::{ControlSlot, PluginEntry, PluginFormat};
    let e = |name: &str, instrument: bool, format: PluginFormat, vendor: &str| PluginEntry {
        name: name.into(),
        instrument,
        format,
        vendor: vendor.into(),
    };
    let mut list = vec![
        e("Sampler", true, PluginFormat::Internal, "Native Instruments"),
        e("Kick", true, PluginFormat::Internal, "Native Instruments"),
        e("Snare", true, PluginFormat::Internal, "Native Instruments"),
        e("Replicant", true, PluginFormat::Internal, "DECKARD"),
        e("Compressor", false, PluginFormat::Internal, "Native Instruments"),
        e("Reverb", false, PluginFormat::Internal, "Native Instruments"),
        e("Massive X", true, PluginFormat::Vst3, "Native Instruments"),
        e("Diva", true, PluginFormat::Vst3, "u-he"),
        e("Pro-Q 3", false, PluginFormat::Vst3, "FabFilter"),
        e("AUBandpass", false, PluginFormat::AudioUnit, "Apple"),
    ];
    // All of Maschine's effects, as the app lists them.
    for (class, _) in deckard_model::INTERNAL_EFFECTS {
        list.push(e(deckard_model::module_label(class), false, PluginFormat::Internal, "Native Instruments"));
    }
    host.set_plugins(list);
    let mut st = host.state.borrow_mut();
    st.control.level = 2;
    st.control.modules_visible = true;
    st.control.slots = vec![
        ControlSlot { name: "Kick".into(), plugin: false, active: true, instrument: true },
        ControlSlot { name: "Compressor".into(), plugin: false, active: true, instrument: false },
        ControlSlot { name: "Pro-Q 3".into(), plugin: true, active: false, instrument: false },
    ];
    st.control.focus_slot = if std::env::var_os("DECKARD_FOCUS_PLUS").is_some() { None } else { Some(0) };
}
