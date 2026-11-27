use super::*;

/// A catalog of 200 plug-ins, longer than any screen: the Plug-in Manager's list.
fn many_plugins(app: &mut App) {
    app.catalog.entries = (0..200)
        .map(|i| deckard_plugins::scan::Entry {
            kind: deckard_plugins::scan::Kind::Vst3,
            vendor: "Acme".into(),
            name: format!("Plug-in {i:03}"),
            instrument: i % 3 == 0,
            location: String::new(),
            cid: format!("{i:032X}"),
        })
        .collect();
}

/// On a short screen (shorter than a 1920x515 monitor, so that the Hardware tab of a
/// new project doesn't fit), Preferences and the panels that open as dialogs stay
/// inside the window, and what doesn't fit scrolls into view: the Hardware tab's last
/// setting is reached with the wheel instead of being cut off.
#[test]
fn dialogs_fit_a_short_window_and_scroll() {
    let mut app = App { settings: settings::Settings::default(), model: "M3".into(), ..Default::default() };
    app.new_project();
    many_plugins(&mut app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1920.0, 380.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    layout::style(&h.ctx);
    h.run_steps(3);
    let screen = h.ctx.content_rect();
    let window = |h: &Harness<'static, App>, title: &str| {
        // (A window's id is its title's text, which egui keeps as an `Option`.)
        h.ctx.memory(|m| m.area_rect(egui::Id::new(Some(std::borrow::Cow::Borrowed(title))))).unwrap_or_else(|| panic!("{title} shown"))
    };
    for tab in [7, 2, 3, 4, 5, 6, 0, 1] {
        h.state_mut().prefs = Some(tab);
        h.run_steps(3);
        let r = window(&h, "Preferences");
        assert!(r.top() >= screen.top() - 0.5 && r.bottom() <= screen.bottom() + 0.5, "tab {tab}: {r:?} in {screen:?}");
    }
    // The Plug-in Manager: its settings at the top, inside the window, and the end of
    // its list out of sight (it scrolls, rather than the window growing). (On Linux the
    // Wine settings come first and are taller than this screen: the tab scrolls too.)
    h.state_mut().prefs = Some(0);
    h.run_steps(3);
    let prefs = window(&h, "Preferences");
    let r = h.get_by_label("Use VST3 Plug-ins").rect();
    assert!(r.top() >= prefs.top() && r.bottom() <= prefs.bottom(), "{r:?} in {prefs:?}");
    assert!(h.query_by_label("Acme — Plug-in 199").is_none_or(|n| n.rect().top() > prefs.bottom()), "the list scrolls");
    h.state_mut().prefs = Some(4);
    h.run_steps(3);
    let prefs = window(&h, "Preferences");
    let last = |h: &Harness<'static, App>| h.get_by_label("Desktop theme").rect();
    assert!(last(&h).bottom() > prefs.bottom(), "the Hardware tab is taller than the window: {:?} vs {prefs:?}", last(&h));
    h.hover_at(prefs.center());
    for _ in 0..20 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    h.run_steps(3);
    let prefs = window(&h, "Preferences");
    assert!(last(&h).bottom() <= prefs.bottom(), "scrolled into view: {:?} in {prefs:?}", last(&h));
    h.state_mut().prefs = None;
    for (title, open) in [("Note Repeat / Arp", 0), ("Lock", 1)] {
        h.state_mut().repeat_panel = open == 0;
        h.state_mut().lock_panel = open == 1;
        h.run_steps(3);
        let r = window(&h, title);
        assert!(r.top() >= screen.top() - 0.5 && r.bottom() <= screen.bottom() + 0.5, "{title}: {r:?} in {screen:?}");
    }
}

/// Renders every Preferences tab on a laptop screen into `DECKARD_RENDER_DIR`, to look
/// at (`cargo test -p deckard --bin deckard -- --ignored render_laptop_dialogs`).
#[test]
#[ignore]
fn render_laptop_dialogs() {
    let Some(dir) = std::env::var_os("DECKARD_RENDER_DIR").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = App { settings: settings::Settings::default(), model: "M3".into(), ..Default::default() };
    app.new_project();
    many_plugins(&mut app);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1366.0, 768.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    // (The app's own fonts, as at start-up: the harness has egui's.)
    layout::style(&h.ctx);
    h.run_steps(3);
    for tab in [7, 2, 3, 4, 5, 6, 0, 1] {
        h.state_mut().prefs = Some(tab);
        h.run_steps(4);
        h.render().unwrap().save(dir.join(format!("prefs-{tab}.png"))).unwrap();
    }
    // The Plug-in Manager on a short screen.
    h.set_size(egui::vec2(1366.0, 380.0));
    h.state_mut().prefs = Some(0);
    h.run_steps(4);
    h.render().unwrap().save(dir.join("prefs-0-short.png")).unwrap();
    h.set_size(egui::vec2(1366.0, 768.0));
    h.state_mut().prefs = None;
    // The main window's toolbars, in each view, at both laptop sizes.
    for (w, ht) in [(1366.0, 768.0), (1280.0, 720.0)] {
        h.set_size(egui::vec2(w, ht));
        for (name, song, mixer, lfos, browser) in [
            ("ideas", false, false, false, false),
            ("song", true, false, false, false),
            ("mixer", false, true, false, false),
            ("lfos", false, false, true, false),
            ("browser", false, false, false, true),
        ] {
            let app = h.state_mut();
            (app.song_view, app.lfos, app.browser.open) = (song, lfos, browser);
            app.set_mixer(mixer);
            h.run_steps(4);
            h.render().unwrap().save(dir.join(format!("main-{name}-{w}.png"))).unwrap();
        }
    }
    h.set_size(egui::vec2(1366.0, 768.0));
    let app = h.state_mut();
    (app.song_view, app.lfos, app.browser.open) = (false, false, false);
    app.set_mixer(false);
    for (name, set) in [("repeat", 0), ("lock", 1), ("monitor", 2)] {
        h.state_mut().repeat_panel = set == 0;
        h.state_mut().lock_panel = set == 1;
        h.state_mut().midi_monitor = set == 2;
        h.run_steps(4);
        h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
    }
}

/// On a laptop screen (1366x768, and 1280x720 to be safe) every Preferences tab and
/// the dialogs a laptop user opens most stay inside the window, both ways.
#[test]
fn dialogs_fit_a_laptop_screen() {
    for (w, ht) in [(1366.0, 768.0), (1280.0, 720.0)] {
        let mut app = App { settings: settings::Settings::default(), model: "M3".into(), ..Default::default() };
        app.new_project();
        many_plugins(&mut app);
        let mut h =
            Harness::builder().with_size(egui::vec2(w, ht)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app: &mut App| app.frame(ui), app);
        layout::style(&h.ctx);
        h.run_steps(3);
        let screen = h.ctx.content_rect();
        let fits = |h: &Harness<'static, App>, title: &str, what: &str| {
            let r = h
                .ctx
                .memory(|m| m.area_rect(egui::Id::new(Some(std::borrow::Cow::Borrowed(title)))))
                .unwrap_or_else(|| panic!("{what} shown"));
            assert!(screen.expand(0.5).contains_rect(r), "{w}x{ht} {what}: {r:?} in {screen:?}");
        };
        for tab in [7, 2, 3, 4, 5, 6, 0, 1] {
            h.state_mut().prefs = Some(tab);
            h.run_steps(3);
            fits(&h, "Preferences", &format!("Preferences tab {tab}"));
        }
        h.state_mut().prefs = None;
        h.state_mut().repeat_panel = true;
        h.run_steps(3);
        fits(&h, "Note Repeat / Arp", "Note Repeat / Arp");
        h.state_mut().repeat_panel = false;
        h.state_mut().lock_panel = true;
        h.run_steps(3);
        fits(&h, "Lock", "Lock");
        h.state_mut().lock_panel = false;
        h.state_mut().midi_monitor = true;
        h.run_steps(3);
        fits(&h, "MIDI Monitor", "MIDI Monitor");
        let opened = h.ctx.memory(|m| m.area_rect(egui::Id::new(Some(std::borrow::Cow::Borrowed("MIDI Monitor"))))).unwrap();
        h.run_steps(20);
        fits(&h, "MIDI Monitor", "MIDI Monitor, later");
        let later = h.ctx.memory(|m| m.area_rect(egui::Id::new(Some(std::borrow::Cow::Borrowed("MIDI Monitor"))))).unwrap();
        assert!((later.width() - opened.width()).abs() < 1.0, "{w}x{ht}: the MIDI Monitor doesn't keep widening: {opened:?} → {later:?}");
        h.state_mut().midi_monitor = false;
    }
}

/// Every character the app's own text uses (outside ASCII) has a glyph in its fonts:
/// none draws as a box ("Preferences ▸ General" did). Read from the sources, so a new
/// symbol is checked too.
#[test]
fn every_symbol_in_the_ui_has_a_glyph() {
    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if p.is_dir() {
                sources(&p, out);
            } else if name.ends_with(".rs") && !name.ends_with("tests.rs") {
                out.push(p);
            }
        }
    }
    // The non-ASCII characters inside string literals (not comments).
    let mut used = std::collections::BTreeMap::<char, String>::new();
    let mut files = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let (mut in_str, mut escaped) = (false, false);
        for line in text.lines() {
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                let c = chars[i];
                if !in_str && c == '/' && chars.get(i + 1) == Some(&'/') {
                    break;
                }
                if !in_str && c == '\'' {
                    // A char literal ('"', '\'') or a lifetime: skip it.
                    let close = chars[i + 1..].iter().take(4).position(|&x| x == '\'').map(|k| i + 1 + k);
                    i = close.filter(|&k| k > i + 1).map_or(i + 1, |k| k + 1);
                    continue;
                }
                if in_str && !escaped && !c.is_ascii() {
                    used.entry(c).or_insert_with(|| f.file_name().unwrap().to_string_lossy().to_string());
                }
                if c == '"' && !escaped {
                    in_str = !in_str;
                }
                escaped = in_str && c == '\\' && !escaped;
                i += 1;
            }
            escaped = false;
        }
    }
    assert!(used.contains_key(&'▸'), "the scan finds the UI's symbols");
    // Each font of the app's text, in order, read for its glyphs. (Not egui's
    // `has_glyph`: it reports "no" for whatever the font that also draws the box
    // supplies, here the emoji font, so every emoji would count as missing.)
    let defs = layout::font_definitions();
    let chain = &defs.families[&egui::FontFamily::Proportional];
    let fonts: Vec<fontdue::Font> = chain
        .iter()
        .map(|name| {
            let data = &defs.font_data[name];
            let settings = fontdue::FontSettings { collection_index: data.index, ..Default::default() };
            fontdue::Font::from_bytes(&data.font[..], settings).unwrap_or_else(|e| panic!("{name}: {e}"))
        })
        .collect();
    let missing: Vec<String> = used
        .iter()
        .filter(|(c, _)| !fonts.iter().any(|f| f.lookup_glyph_index(**c) != 0))
        .map(|(c, file)| format!("U+{:04X} {c} ({file})", *c as u32))
        .collect();
    assert!(missing.is_empty(), "no glyph for: {missing:?}");
}

/// On a laptop screen the header's groups don't overlap: the transport, centred, moves
/// right of Save rather than over it (at 1280 px it would cover it), and leaves the
/// master fader and the CPU meter their room. Narrower than its groups fit (900 px),
/// it stays centred, as before.
#[test]
fn the_header_fits_a_laptop_screen() {
    for w in [1280.0, 1366.0, 1920.0] {
        let mut app = App { settings: settings::Settings::default(), model: "M3".into(), ..Default::default() };
        app.new_project();
        let mut h = Harness::builder()
            .with_size(egui::vec2(w, 720.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
        layout::style(&h.ctx);
        h.run_steps(3);
        let at = |name: &str| h.ctx.data(|d| d.get_temp::<egui::Rect>(layout::header_rect(name))).unwrap_or_else(|| panic!("{name} drawn"));
        let (save, restart, keys, master, cpu) = (at("save"), at("restart"), at("keys"), at("master"), at("cpu"));
        assert!(save.right() <= restart.left(), "{w}: Save clear of the transport: {save:?} vs {restart:?}");
        assert!(keys.right() <= master.left(), "{w}: the transport clear of the master fader: {keys:?} vs {master:?}");
        assert!(cpu.right() <= w, "{w}: the CPU meter on the screen: {cpu:?}");
        if w >= 1920.0 {
            let mid = (restart.left() + keys.right()) / 2.0;
            assert!((mid - w / 2.0).abs() < 2.0, "{w}: still centred where there's room: {mid}");
        }
    }
    let mut app = App { settings: settings::Settings::default(), model: "M3".into(), ..Default::default() };
    app.new_project();
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 720.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    layout::style(&h.ctx);
    h.run_steps(3);
    let at = |name: &str| h.ctx.data(|d| d.get_temp::<egui::Rect>(layout::header_rect(name))).unwrap();
    let mid = (at("restart").left() + at("keys").right()) / 2.0;
    assert!((mid - 450.0).abs() < 2.0, "900: too narrow to fit between: centred, as before ({mid})");
}

/// The MASCHINE menu: the undo history undoes several steps at once; Open Recent
/// asks about unsaved changes first.
#[test]
fn menus_undo_history_and_open_recent() {
    let Some(mut h) = harness(false, true) else { return };
    let doc = h.state().doc.clone().unwrap();
    let tempo = doc.borrow().project().tempo;
    for t in [100.0, 101.0, 102.0] {
        doc.borrow_mut().set_tempo(t).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1600)); // not merged into one step
    }
    h.run_steps(2);
    let steps = doc.borrow().undo_labels().len();
    assert!(steps >= 3, "{:?}", doc.borrow().undo_labels());
    // MASCHINE ▸ Edit ▸ Undo History ▸ the third entry: three steps back.
    click(&mut h, Pos2::new(40.0, 16.0));
    h.get_by_label("Edit ⏵").hover();
    h.run_steps(3);
    h.get_by_label("Undo History ⏵").hover();
    h.run_steps(3);
    let third = doc.borrow().undo_labels()[2].to_string();
    h.get_all_by_label(&third).nth(2).expect("third history entry").click();
    h.run_steps(3);
    assert_eq!(doc.borrow().undo_labels().len(), steps - 3);
    assert_eq!(doc.borrow().project().tempo, tempo);
    assert_eq!(doc.borrow().redo_labels().len(), 3);
    // ⌘Y redoes one.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Y);
    h.run_steps(2);
    assert_eq!(doc.borrow().project().tempo, 100.0);

    // Open Recent with unsaved changes: asked first; Don't Save opens it.
    let tmp = tempfile::tempdir().unwrap();
    let other = tmp.path().join("menu.mxprj");
    std::fs::copy(h.state().path.clone().unwrap(), &other).unwrap();
    h.state_mut().recent = vec![other.clone()];
    h.state_mut().menu_commands.push(menu::Command::OpenRecent(other.clone()));
    h.run_steps(3);
    assert!(h.state().confirm.is_some(), "asks about the unsaved tempo change");
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert!(h.state().confirm.is_none());
    assert_eq!(h.state().path.as_deref(), Some(other.as_path()));
    assert!(!h.state().dirty());
}

/// File › New: an empty project built from scratch, untitled, that takes modules
/// (no other project or NI file needed) and asks about unsaved changes first.
#[test]
fn file_new_builds_an_empty_project() {
    let Some(mut h) = harness(false, true) else { return };
    h.state().doc.clone().unwrap().borrow_mut().set_tempo(99.0).unwrap();
    h.state_mut().menu_commands.push(menu::Command::New);
    h.run_steps(3);
    assert!(h.state().confirm.is_some(), "asks about the unsaved tempo change");
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert!(h.state().path.is_none());
    let doc = h.state().doc.clone().unwrap();
    assert_eq!(doc.borrow().project().groups[0].sounds.len(), 16);
    assert_eq!(doc.borrow().project().tempo, 120.0);
    h.state_mut().add_slot(layout::AddWhat::Sampler);
    h.state_mut().add_slot(layout::AddWhat::Effect("Reverb"));
    h.run_steps(2);
    let names: Vec<String> = doc.borrow().project().groups[0].sounds[0].slots.iter().map(|x| x.module.clone()).collect();
    assert_eq!(names, ["Sampler", "Reverb"], "{}", h.state().status);
}

/// "+" › Internal › each of Maschine's instruments in a new project: it goes in with
/// Maschine's defaults and plays a note.
#[test]
fn instruments_from_the_add_menu_play() {
    let Some(mut h) = harness(false, true) else { return };
    h.state_mut().menu_commands.push(menu::Command::New);
    h.run_steps(3);
    if h.state().confirm.is_some() {
        h.get_by_label("Don't Save").click();
        h.run_steps(3);
    }
    for (s, class) in deckard_model::INTERNAL_INSTRUMENTS.into_iter().enumerate() {
        (h.state_mut().group, h.state_mut().sound) = (0, s);
        h.state_mut().add_slot(layout::AddWhat::Instrument(class));
        h.run_steps(2);
    }
    let doc = h.state().doc.clone().unwrap();
    let project = doc.borrow().project().clone();
    for (s, class) in deckard_model::INTERNAL_INSTRUMENTS.into_iter().enumerate() {
        let slot = &project.groups[0].sounds[s].slots[0];
        assert_eq!(slot.module, class, "{}", h.state().status);
        let defaults = deckard_engine::synth::defaults(class).unwrap();
        for (k, v) in defaults {
            assert_eq!(slot.param(k), Some(v as f32 as f64), "{class} {k}");
        }
        let (mut e, _) = Engine::new(&project, None, 48_000, Mode::Arrangement, &mut SampleCache::default());
        e.note_on(0, s, 60, 100, deckard_model::PPQ);
        let out = deckard_engine::render(&mut e, 0.5);
        let peak = out.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-3, "{class} plays: peak {peak}");
    }
    assert_eq!(module_ui::pages("BassSynth").unwrap().len(), 2);
}

/// The Sampler's six pages: knobs change its parameters (undoable), on/off knobs
/// switch with a click, slots follow the filter mode, and the waveform's markers move
/// the zone's play range.
#[test]
fn sampler_pages_edit_parameters() {
    let Some(mut h) = harness(false, true) else { return };
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (0, 0, layout::ChainTab::Sound);
    h.run_steps(4);
    assert_eq!(h.state().slot, Some(0), "the Sampler's slot is shown");
    h.state_mut().sampler_page = 1;
    h.run_steps(4);
    let smp = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].sampler.clone().unwrap();
    let knobs = |h: &Harness<'static, App>| -> Vec<(String, egui::Rect)> {
        h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).expect("knobs drawn")
    };
    let knob =
        |h: &Harness<'static, App>, name: &str| knobs(h).into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name} knob")).1;
    // Pitch / Envelope: TUNE up a little; REVERSE switches with a click.
    let r = knob(&h, "TUNE");
    drag(&mut h, r.center(), r.center() - egui::vec2(0.0, 20.0));
    assert!(smp(&h).get("tune") > 0.05, "{}", smp(&h).get("tune"));
    assert_eq!(h.state().doc.as_ref().unwrap().borrow().undo_label(), Some("Sampler"));
    let r = knob(&h, "REVERSE");
    click(&mut h, r.center());
    assert!(smp(&h).reverse);
    // FX / Filter: no cutoff while the filter is off; LP2 brings CUTOFF and RESONANCE.
    h.state_mut().sampler_page = 2;
    h.run_steps(4);
    assert!(!knobs(&h).iter().any(|(n, _)| n == "CUTOFF"));
    // FILTER is a selector: its down arrow picks the next choice, LP2.
    let r = knob(&h, "FILTER");
    click(&mut h, egui::pos2(r.center().x, r.top() + 57.0));
    assert_eq!(smp(&h).get("filter"), 1.0);
    click(&mut h, egui::pos2(r.center().x, r.top() + 57.0));
    assert_eq!(smp(&h).get("filter"), 2.0, "BP2");
    click(&mut h, egui::pos2(r.center().x, r.top() + 27.0));
    assert_eq!(smp(&h).get("filter"), 1.0, "the up arrow goes back");
    let names: Vec<String> = knobs(&h).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["COMP", "DRIVE", "SR", "BITS", "FILTER", "CUTOFF", "RESONANCE"]);
    let r = knob(&h, "CUTOFF");
    drag(&mut h, r.center(), r.center() + egui::vec2(0.0, 30.0));
    assert!(smp(&h).get("cutoff") < 0.5);
    // The waveform: drag the start marker right.
    let before = smp(&h).zones[0].play_range.0;
    let wf: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-waveform"))).expect("waveform drawn");
    let y = wf.center().y;
    drag(&mut h, egui::pos2(wf.left() + 1.0, y), egui::pos2(wf.left() + 100.0, y));
    assert!(smp(&h).zones[0].play_range.0 > before + 100, "{}", smp(&h).zones[0].play_range.0);
    // Undo everything.
    for _ in 0..8 {
        h.state_mut().undo();
    }
    assert_eq!(smp(&h).get("tune"), 0.0);
    assert!(!smp(&h).reverse);
    assert_eq!(smp(&h).get("filter"), 0.0);
}

/// "+" › Internal: each of Maschine's effects DECKARD renders goes in at its
/// defaults, survives saving, and runs in the engine.
#[test]
fn internal_effects_from_the_add_menu() {
    let Some(mut h) = harness(false, true) else { return };
    let p = project_of(&h);
    let Some((g, s)) = p.groups.iter().enumerate().find_map(|(g, gr)| gr.sounds.iter().position(|s| s.slots.is_empty()).map(|s| (g, s)))
    else {
        return;
    };
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
    h.state_mut().add_slot(layout::AddWhat::Sampler);
    for (class, _) in deckard_model::INTERNAL_EFFECTS {
        h.state_mut().add_slot(layout::AddWhat::Effect(class));
        h.run_steps(2);
        assert_eq!(
            project_of(&h).groups[g].sounds[s].slots.last().map(|x| x.module.clone()).as_deref(),
            Some(*class),
            "{}",
            h.state().status
        );
        assert_eq!(h.state().slot, Some(project_of(&h).groups[g].sounds[s].slots.len() - 1), "the new slot is shown");
    }
    let bytes = h.state().doc.as_ref().unwrap().borrow().to_bytes().unwrap();
    if let Some(dir) = render_dir() {
        std::fs::write(dir.join("internal-effects.mxprj"), &bytes).unwrap();
    }
    let back = deckard_doc::Document::from_bytes(bytes, None).expect("reads back");
    let slots = &back.project().groups[g].sounds[s].slots;
    assert_eq!(slots.len(), 1 + deckard_model::INTERNAL_EFFECTS.len());
    for (slot, (class, defaults)) in slots[1..].iter().zip(deckard_model::INTERNAL_EFFECTS) {
        assert_eq!(slot.module, *class);
        for (k, d) in defaults.iter() {
            if let Some(v) = slot.param(k) {
                assert!((v - d).abs() < 1e-6, "{class} {k} = {v}, not the default {d}");
            }
        }
        assert!(deckard_engine::fx::make(&slot.module, &slot.params, 48_000.0).is_some(), "{class} renders");
    }
}

/// The Mix view's strip draws each of Maschine's effects as its editor lays it out,
/// and every knob shown turns its parameter. `$DECKARD_RENDER_DIR` gets pictures.
#[test]
fn effect_editors_turn_their_knobs() {
    let Some(mut h) = harness(false, true) else { return };
    layout::style(&h.ctx);
    let p = project_of(&h);
    let Some((g, s)) = p.groups.iter().enumerate().find_map(|(g, gr)| gr.sounds.iter().position(|s| s.slots.is_empty()).map(|s| (g, s)))
    else {
        return;
    };
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
    h.state_mut().song_view = false;
    h.state_mut().mixer = true;
    let with_editor: Vec<&str> =
        deckard_model::INTERNAL_EFFECTS.iter().map(|(c, _)| *c).filter(|c| layout::strip::editor::editor(c).is_some()).collect();
    for class in &with_editor {
        // One effect at a time, so its panel is in view.
        while !project_of(&h).groups[g].sounds[s].slots.is_empty() {
            layout::apply_acts(h.state_mut(), vec![layout::Act::RemoveSlot(0)]);
            h.run_steps(1);
        }
        h.state_mut().add_slot(layout::AddWhat::Effect(class));
        h.run_steps(4);
        let slot = project_of(&h).groups[g].sounds[s].slots[0].clone();
        assert_eq!(slot.module, *class);
        let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap();
        let r = panels.iter().find(|p| p.0 == 0).expect("its panel").1;
        if let Some(dir) = render_dir() {
            h.render().unwrap().save(dir.join(format!("editor-{class}.png"))).unwrap();
        }
        let spots = layout::strip::editor::knob_spots(&slot, (g, s, 0));
        assert!(!spots.is_empty(), "{class} shows knobs");
        for (at, field) in spots {
            let before = project_of(&h).groups[g].sounds[s].slots[0].param(&field).unwrap_or(0.0);
            let c = r.min + at;
            // Up, or down from the top of the range.
            let dy = if before >= 0.99 { 40.0 } else { -40.0 };
            drag(&mut h, c, c + egui::vec2(0.0, dy));
            let after = project_of(&h).groups[g].sounds[s].slots[0].param(&field).unwrap_or(0.0);
            assert!((after - before).abs() > 1e-3, "{class} {field}: {before} -> {after}");
        }
    }
}

/// "+" › Internal › Replicant: DECKARD's own synth goes in as a plug-in with its
/// own pages (choices as selectors), its knobs are saved into the project as they
/// turn (undoable), and it plays.
#[test]
fn native_synth_from_the_add_menu() {
    let Some(mut h) = harness(false, true) else { return };
    let p = project_of(&h);
    let Some((g, s)) = p.groups.iter().enumerate().find_map(|(g, gr)| gr.sounds.iter().position(|s| s.slots.is_empty()).map(|s| (g, s)))
    else {
        return;
    };
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
    let class = deckard_plugins::native::CLASSES[0];
    h.state_mut().add_slot(layout::AddWhat::Plugin(layout::native_entry(class)));
    h.run_steps(4);
    let slot = |h: &Harness<'static, App>| project_of(h).groups[g].sounds[s].slots[0].clone();
    assert_eq!(slot(&h).plugin.as_ref().map(|p| p.id.clone()), Some(class.plugin_id()), "{}", h.state().status);
    assert!(slot(&h).is_instrument());
    let knobs: Vec<(String, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap();
    assert_eq!(
        knobs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        ["WAVE 1", "WAVE 2", "PITCH 2", "DETUNE", "MIX", "SUB", "NOISE", "PW"],
        "the Oscillators page"
    );
    let view = h.state().plugin_view().expect("pages shown");
    assert_eq!(view.pages.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Oscillators", "Filter", "Amp", "Modulation", "Super"]);
    assert_eq!(view.pages[0].choices[0], ["Saw", "Pulse", "Triangle", "Sine"], "a wave is a choice");
    if let Some(dir) = render_dir() {
        h.render().unwrap().save(dir.join("replicant-sound-tab.png")).unwrap();
        h.state_mut().mixer = true;
        h.run_steps(4);
        h.render().unwrap().save(dir.join("replicant-mix.png")).unwrap();
        h.state_mut().mixer = false;
        h.run_steps(2);
    }
    // Turn the cutoff: stored in the project at once, and undoable.
    let before = slot(&h).plugin.unwrap().chunk;
    h.state_mut().plugin_page = 1;
    h.run_steps(2);
    let cutoff = view_cutoff(&h);
    layout::apply_acts(h.state_mut(), vec![layout::Act::PluginParam(0, cutoff, 0.2)]);
    h.run_steps(2);
    let after = slot(&h).plugin.unwrap().chunk;
    assert_ne!(after, before, "the knob went into the project");
    let inst = h.state_mut().slot_plugin(g, s, 0).unwrap();
    assert_eq!(inst.lock().unwrap().param_value(cutoff), 0.2);
    let doc = h.state().doc.clone().unwrap();
    if let Some(dir) = render_dir() {
        std::fs::write(dir.join("native-synth-edited.mxprj"), doc.borrow().to_bytes().unwrap()).unwrap();
    }
    assert!(doc.borrow_mut().undo());
    h.run_steps(2);
    assert_eq!(slot(&h).plugin.unwrap().chunk, before, "undo restores it");
    let inst = h.state_mut().slot_plugin(g, s, 0).unwrap();
    assert_eq!(inst.lock().unwrap().param_value(cutoff), 0.6, "and the synth follows");
    // Saved and read back: the same plug-in and state.
    let bytes = doc.borrow().to_bytes().unwrap();
    if let Some(dir) = render_dir() {
        std::fs::write(dir.join("native-synth.mxprj"), &bytes).unwrap();
    }
    let back = deckard_doc::Document::from_bytes(bytes, None).expect("reads back");
    assert_eq!(back.project().groups[g].sounds[s].slots[0].plugin.as_ref().map(|p| &p.chunk), Some(&before));
}

fn view_cutoff(h: &Harness<'static, App>) -> u32 {
    let view = h.state().plugin_view().unwrap();
    view.pages[1].knobs.iter().find(|(i, ..)| i.short == "CUTOFF").unwrap().0.id
}

/// BUGS #24: each slot starts with an instrument or effect icon; a click on it
/// bypasses the plug-in, another brings it back.
#[test]
fn a_slots_icon_bypasses_it() {
    let Some(mut h) = harness(false, true) else { return };
    let p = project_of(&h);
    let Some((g, s)) = p.groups.iter().enumerate().find_map(|(g, gr)| gr.sounds.iter().position(|s| s.slots.is_empty()).map(|s| (g, s)))
    else {
        return;
    };
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
    h.run_steps(4);
    h.state_mut().add_slot(layout::AddWhat::Sampler);
    h.run_steps(4);
    let icons = |h: &Harness<'static, App>| -> Vec<(egui::Rect, bool)> {
        h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-chain-icons"))).unwrap()
    };
    assert_eq!(icons(&h).iter().map(|(_, inst)| *inst).collect::<Vec<_>>(), [true], "the Sampler is an instrument");
    let active = |h: &Harness<'static, App>| project_of(h).groups[g].sounds[s].slots[0].active;
    let at = icons(&h)[0].0.center();
    click(&mut h, at);
    assert!(!active(&h), "bypassed");
    let at = icons(&h)[0].0.center();
    click(&mut h, at);
    assert!(active(&h), "back");
}

/// The SOUND tab's chain: add a Sampler to an empty sound, show its pages, bypass and
/// remove it; the sound's name shows its channel.
#[test]
fn chain_slots_in_the_sound_tab() {
    let Some(mut h) = harness(false, true) else { return };
    let p = project_of(&h);
    let Some((g, s)) = p.groups.iter().enumerate().find_map(|(g, gr)| gr.sounds.iter().position(|s| s.slots.is_empty()).map(|s| (g, s)))
    else {
        return;
    };
    (h.state_mut().group, h.state_mut().sound, h.state_mut().chain_tab) = (g, s, layout::ChainTab::Sound);
    h.run_steps(4);
    assert_eq!(h.state().slot, None, "an empty sound shows its channel");
    h.state_mut().add_slot(layout::AddWhat::Sampler);
    h.run_steps(4);
    let slots = |h: &Harness<'static, App>| project_of(h).groups[g].sounds[s].slots.clone();
    assert_eq!(slots(&h).iter().map(|x| x.module.as_str()).collect::<Vec<_>>(), ["Sampler"]);
    assert_eq!(h.state().slot, Some(0), "the new slot is shown");
    let knobs: Vec<(String, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-knobs"))).unwrap();
    assert_eq!(knobs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["POLYPHONY", "PITCHBEND", "MODE"], "Voice Settings / Engine");
    // The channel icon: its channel properties; the plug-in icon: the chain again.
    let icon = |h: &Harness<'static, App>, k: usize| -> egui::Rect {
        h.ctx.data(|d| d.get_temp(egui::Id::new(("deckard-chain-mode", k)))).unwrap()
    };
    let channel_icon = icon(&h, 0);
    click(&mut h, channel_icon.center());
    assert!(h.state().channel);
    let plug_icon = icon(&h, 1);
    click(&mut h, plug_icon.center());
    assert!(!h.state().channel);
    let rows: Vec<egui::Rect> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-chain-rows"))).unwrap();
    click(&mut h, rows[0].center());
    assert_eq!(h.state().slot, Some(0));
    h.state_mut().arrange_act(layout::Act::SlotActive(0, false));
    h.run_steps(2);
    assert!(!slots(&h)[0].active);
    h.state_mut().arrange_act(layout::Act::RemoveSlot(0));
    h.run_steps(4);
    assert!(slots(&h).is_empty());
    assert_eq!(h.state().slot, None);
}

/// The Ideas view shows a group's patterns in their places, with the gaps between, and
/// a double-click on a gap or below the last makes a pattern there.
#[test]
fn the_ideas_view_shows_and_fills_gaps() {
    let mut app = App { show_controller: false, follow: true, grid: PPQ / 4, prop_set: 1, ..Default::default() };
    app.new_project();
    {
        let mut d = app.doc.as_ref().unwrap().borrow_mut();
        d.add_pattern(0, 0, "One").unwrap();
        d.add_pattern_in_slot(0, 0, 3, "Four").unwrap();
    }
    (app.group, app.pattern, app.scene, app.sound, app.song_view) = (0, 0, 0, 0, false);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 930.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    h.run_steps(4);
    let place = |h: &Harness<'static, App>, slot: usize| -> egui::Rect {
        h.ctx.data(|d| d.get_temp(layout::header_rect(&format!("pattern place 0 {slot}")))).expect("drawn")
    };
    let slots = |h: &Harness<'static, App>| project_of(h).groups[0].patterns.iter().map(|p| p.slot).collect::<Vec<_>>();
    let (first, fourth) = (place(&h, 0), place(&h, 3));
    assert!((fourth.top() - first.top() - 3.0 * first.height()).abs() < 0.5, "its own row: {first:?} {fourth:?}");
    // Clicking the second of the list, in the fourth place, picks it.
    click(&mut h, fourth.center());
    assert_eq!(h.state().pattern, 1);
    // A gap.
    let gap = place(&h, 1).center();
    double_click(&mut h, gap);
    assert_eq!(slots(&h), [0, 1, 3], "{}", h.state().status);
    assert_eq!(project_of(&h).groups[0].patterns[2].name, "Four", "the rest stay");
    // Below the last, leaving a gap.
    let below = place(&h, 6).center();
    double_click(&mut h, below);
    assert_eq!(slots(&h), [0, 1, 3, 6]);
}

/// Auto-write on while the song plays: dragging a mixer strip's fader or pan writes
/// modulation and leaves them where they are. Otherwise it moves them.
#[test]
fn dragging_a_mixer_fader_writes_with_auto_write_on() {
    let mut app = App { show_controller: false, follow: true, grid: PPQ / 4, prop_set: 1, ..Default::default() };
    app.new_project();
    {
        let mut d = app.doc.as_ref().unwrap().borrow_mut();
        d.add_pattern(0, 0, "Pattern 1").unwrap();
        d.assign_pattern(0, 0, Some(0)).unwrap();
    }
    (app.group, app.pattern, app.scene, app.sound) = (0, 0, 0, 0);
    (app.mixer, app.mixer_sounds) = (true, false);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 930.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    h.run_steps(4);
    let strips: Vec<(layout::Channel, egui::Rect)> =
        h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-mixer-strips"))).expect("mixer drawn");
    let r = strips.iter().find(|(c, _)| *c == layout::Channel::Group(0)).unwrap().1;
    let x = r.left() + 48.0;
    let zero_db = r.bottom() - 186.0 + 26.0 + 25.0;
    let pan = egui::pos2(r.left() + 5.0 + 30.5, r.bottom() - 186.0 + 8.0);
    let level = |h: &Harness<'static, App>| project_of(h).groups[0].level;
    let pan_now = |h: &Harness<'static, App>| project_of(h).groups[0].pan;
    // Stopped: it moves.
    let before = level(&h);
    drag(&mut h, egui::pos2(x, zero_db), egui::pos2(x, zero_db + 22.0));
    assert!(level(&h) < before * 0.8, "about -10 dB: {}", level(&h));
    // Playing with Auto-write on: written.
    let before = level(&h);
    h.state_mut().auto_write = true;
    h.state_mut().transport_play();
    if !matches!(h.state().output, Some(Ok(_))) {
        eprintln!("no audio output; skipping");
        return;
    }
    let wait = std::time::Instant::now();
    while h.state().playhead().is_none() && wait.elapsed() < std::time::Duration::from_secs(60) {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let at = zero_db + 22.0;
    drag(&mut h, egui::pos2(x, at), egui::pos2(x, at + 15.0));
    drag(&mut h, pan, pan + egui::vec2(10.0, 0.0));
    h.state_mut().stop();
    h.run_steps(2);
    let p = project_of(&h);
    assert_eq!(level(&h), before, "the fader stays");
    assert_eq!(pan_now(&h), 0.0, "and the pan");
    let points =
        |field: &str| p.groups[0].patterns[0].mods.iter().filter(|m| m.sound.is_none() && m.slot.is_none() && m.field == field).count();
    assert!(points("level") > 0, "the level written");
    assert!(points("pan") > 0, "the pan written");
    assert!(p.groups[0].patterns[0].mods.iter().filter(|m| m.field == "level").all(|m| m.value < 0.0), "down from the fader's place");
}

/// The Mixer: a strip per group; the fader sets the level, M mutes, right-click on
/// M solos (again: unsolo), the headphones cue; double-clicking a group header's
/// background shows its sounds.
#[test]
fn mixer_strips() {
    let Some(mut h) = harness(false, true) else { return };
    h.state_mut().mixer = true;
    h.state_mut().mixer_sounds = false;
    h.run_steps(4);
    let strips = |h: &Harness<'static, App>| -> Vec<(layout::Channel, egui::Rect)> {
        h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-mixer-strips"))).expect("mixer drawn")
    };
    let rect = |h: &Harness<'static, App>, ch| strips(h).into_iter().find(|(c, _)| *c == ch).unwrap().1;
    let p = |h: &Harness<'static, App>| project_of(h);
    assert!(strips(&h).iter().any(|(c, _)| *c == layout::Channel::Master), "the master on the right");
    // Fader: drag group 1's arrow down.
    let r = rect(&h, layout::Channel::Group(1));
    let level = p(&h).groups[1].level;
    let y = r.bottom() - 186.0 + 26.0 + 25.0; // 0 dB line
    let x = r.left() + 48.0;
    drag(&mut h, egui::pos2(x, y), egui::pos2(x, y + 22.0));
    assert!(p(&h).groups[1].level < level * 0.6, "about -10 dB: {}", p(&h).groups[1].level);
    // Mute, then solo group 2 (right-click M), then unsolo.
    let m = egui::pos2(r.right() - 23.0, r.bottom() - 186.0 + 153.0);
    click(&mut h, m);
    assert!(p(&h).groups[1].muted);
    click(&mut h, m);
    assert!(!p(&h).groups[1].muted);
    let r2 = rect(&h, layout::Channel::Group(2));
    let m2 = egui::pos2(r2.right() - 23.0, r2.bottom() - 186.0 + 153.0);
    h.hover_at(m2);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: m2, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.run_steps(3);
    let muted: Vec<bool> = p(&h).groups.iter().map(|g| g.muted).collect();
    assert!(!muted[2] && muted.iter().enumerate().all(|(i, m)| i == 2 || *m), "solo: {muted:?}");
    h.hover_at(m2);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: m2, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.run_steps(3);
    assert!(p(&h).groups.iter().all(|g| !g.muted), "unsolo");
    // Cue.
    click(&mut h, egui::pos2(r2.left() + 14.0, r2.bottom() - 186.0 + 153.0));
    assert!(p(&h).groups[2].cue);
    // Double-click group 2's header background: its sounds.
    double_click(&mut h, egui::pos2(r2.left() + 60.0, r2.top() + 10.0));
    assert!(h.state().mixer_sounds);
    assert_eq!(h.state().group, 2);
    assert!(strips(&h).iter().any(|(c, _)| *c == layout::Channel::Sound(2, 0)));
    assert!(strips(&h).iter().any(|(c, _)| *c == layout::Channel::Group(2)), "the group on the right");
}

/// Sampling: the strip button opens it on the focused sound's sample; dragging on
/// the waveform selects; NORM. writes an edited copy and points the zone at it
/// (undoable); SLICE ▸ APPLY makes a zone per slice on keys from C3.
#[test]
fn recordings_land_where_maschine_puts_them() {
    use layout::sampling::RecParam;
    let Some(mut h) = harness(false, true) else { return };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    h.state_mut().path = Some(dir.join("x.mxprj"));
    let rate = 48_000;
    let tone = |secs: f64| deckard_engine::sample::Sample {
        rate,
        frames: (0..(secs * rate as f64) as usize)
            .map(|i| {
                let v = (i as f32 / rate as f32 * 220.0 * std::f32::consts::TAU).sin() * 0.5;
                [v, v]
            })
            .collect(),
    };
    let p = project_of(&h);
    let bar = p.bar_length() as f64;
    let free = |p: &deckard_model::Project, g: usize| {
        (1..16).find(|&s| p.groups[g].sounds[s].slots.is_empty() && p.groups[g].sounds[s].sampler.is_none()).unwrap()
    };

    // Loop, Target Pattern: a new pattern in the scene, playing the take in the next
    // free sound, long enough for it (2 bars into a 1-bar pattern doubles it).
    (h.state_mut().group, h.state_mut().pattern, h.state_mut().sound, h.state_mut().scene) = (0, 0, 0, 0);
    h.state_mut().song_view = false;
    h.state_mut().recorder.view.set(RecParam::Target, 2);
    let s1 = free(&p, 0);
    let n = p.groups[0].patterns.len();
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone(1.0), channels: 2, mode: 2, ticks: 2.0 * bar, focus: (0, 0) });
    let q = project_of(&h);
    assert_eq!(q.groups[0].patterns.len(), n + 1, "{}", h.state().status);
    let pat = &q.groups[0].patterns[n];
    // An Audio plug-in playing the take (its length the recording's, in beats), on in
    // the new pattern without notes.
    assert!(pat.notes.iter().all(|x| x.sound != s1));
    assert_eq!(pat.audio, vec![deckard_model::AudioCue { sound: s1, take: 0, offset: 0 }]);
    assert!(pat.length >= 2 * bar as i64);
    assert_eq!(q.scenes[0].patterns.iter().find(|(g, _)| *g == 0).map(|x| x.1), Some(n), "the scene plays the new pattern");
    let a = q.groups[0].sounds[s1].audio.clone().expect("an Audio plug-in");
    assert_eq!(q.groups[0].sounds[s1].instrument, "AudioModule");
    assert_eq!(a.takes.len(), 1);
    assert!((a.takes[0].beats - 2.0 * bar / PPQ as f64).abs() < 1e-6, "{}", a.takes[0].beats);
    assert_eq!((h.state().pattern, h.state().sound), (n, s1));

    // Loop, Target Sound: the next free sound, in the current pattern.
    h.state_mut().recorder.view.set(RecParam::Target, 1);
    let s2 = free(&q, 0);
    assert_ne!(s1, s2);
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone(0.5), channels: 1, mode: 2, ticks: bar / 2.0, focus: (0, s1) });
    let q = project_of(&h);
    assert!(q.groups[0].patterns[n].audio.iter().any(|c| c.sound == s2));
    assert!(q.groups[0].sounds[s2].audio.is_some());
    // Loop, Target Take, on that sound: a second take, playing in the pattern.
    (h.state_mut().sound, h.state_mut().pattern) = (s2, n);
    h.state_mut().recorder.view.set(RecParam::Target, 0);
    h.state_mut().finish_recording(recorder::Finished::Take { sample: tone(0.5), channels: 1, mode: 2, ticks: bar / 2.0, focus: (0, s2) });
    let q = project_of(&h);
    assert_eq!(q.groups[0].sounds[s2].audio.as_ref().map(|a| (a.takes.len(), a.focused)), Some((2, 1)));
    assert!(q.groups[0].patterns[n].audio.contains(&deckard_model::AudioCue { sound: s2, take: 1, offset: 0 }));

    // Auto: zones with Maschine's key/velocity ranges and roots, in a free sound
    // when the focused one has no Sampler.
    let g = 7; // Pad: a Massive sound
    assert!(q.groups[g].sounds[0].sampler.is_none());
    (h.state_mut().group, h.state_mut().sound) = (g, 0);
    let s3 = free(&q, g);
    let mut done = Vec::new();
    for (note, keys) in [(48u8, (0u8, 53u8)), (60, (54, 127))] {
        let path = dir.join(format!("Pad {} 100.wav", recorder::note_name(note)));
        let s = tone(0.5);
        deckard_engine::edit::write_wav(&path, &s).unwrap();
        done.push((recorder::AutoItem { note, velocity: 100, keys, vels: (0, 127), path }, s.frames.len(), Some((4800, 20_000))));
    }
    h.state_mut().finish_recording(recorder::Finished::Auto { done, focus: (g, 0) });
    let q = project_of(&h);
    let zones = &q.groups[g].sounds[s3].sampler.as_ref().expect("a Sampler in the free sound").zones;
    let got: Vec<(i64, i64, i64)> = zones.iter().map(|z| (z.root_key, z.key_range.0, z.key_range.1)).collect();
    assert_eq!(got, vec![(48, 0, 53), (60, 54, 127)]);
    assert!(zones.iter().all(|z| z.loop_active && z.loop_range == (4800, 20_000)));
    assert_eq!((h.state().sound, h.state().sampling.tab), (s3, 3));
}
