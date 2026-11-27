use super::*;

#[test]
fn the_pattern_editor_shows_the_scene_pattern_and_follows_playback() {
    let Some(mut h) = harness(false, true) else { return };
    let (first, later) = {
        let d = h.state().doc.as_ref().unwrap().borrow();
        let p = d.project();
        let pat = |s: usize| p.scenes[s].patterns.iter().find(|(g, _)| *g == 0).map(|&(_, pi)| pi);
        // a later section whose scene plays a different pattern on group A
        let later = (0..p.sections.len()).find(|&i| p.sections[i].scene.and_then(pat) != pat(0) && p.sections[i].scene.is_some());
        (pat(0), later.map(|i| (i, p.section_start(i), p.sections[i].scene.unwrap(), p.sections[i].scene.and_then(pat))))
    };
    assert_eq!(Some(h.state().pattern), first, "opening focuses scene 1's pattern");
    let Some((_, start, scene, pattern)) = later else { return };
    if !play(&mut h) {
        return;
    }
    // Follow on: jump playback into the later section; the focus follows.
    if let Some(Ok(out)) = &h.state().output {
        out.seek(start + 10);
    }
    h.run_steps(3);
    assert_eq!(h.state().scene, scene);
    assert_eq!(Some(h.state().pattern), pattern);
    // Follow off: it stays.
    h.state_mut().follow = false;
    if let Some(Ok(out)) = &h.state().output {
        out.seek(10);
    }
    h.run_steps(3);
    assert_eq!(h.state().scene, scene);
    h.state_mut().stop();
}

#[test]
fn ideas_view_edits_scenes() {
    let Some(mut h) = harness(false, false) else { return };
    let p = project_of(&h);
    let scene = h.state().scene;
    // A pattern not in the focused scene (within the first 8 columns, visible).
    let Some((g, pi)) = (0..p.groups.len().min(8))
        .flat_map(|g| (0..p.groups[g].patterns.len().min(4)).map(move |pi| (g, pi)))
        .find(|gp| !p.scenes[scene].patterns.contains(gp))
    else {
        return;
    };
    click(&mut h, cell(g, pi));
    assert!(project_of(&h).scenes[scene].patterns.contains(&(g, pi)), "clicking a pattern puts it in the scene");
    assert_eq!((h.state().group, h.state().pattern), (g, pi));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);
    assert_eq!(project_of(&h).scenes[scene].patterns, p.scenes[scene].patterns, "⌘Z undoes it");

    // "+" after the last scene tab.
    let n = p.scenes.len();
    click(&mut h, Pos2::new(3.0 + 33.0 + n as f32 * 99.0 + 13.0, 33.0 + 3.0 + 16.0));
    assert_eq!(project_of(&h).scenes.len(), n + 1);
    assert_eq!(h.state().scene, n);

    // Right-click the first scene tab, Duplicate.
    right_click(&mut h, scene_tab(0));
    press(&mut h, "Duplicate");
    let after = project_of(&h);
    assert_eq!(after.scenes.len(), n + 2);
    assert_eq!(after.scenes[1].name, format!("{} (1)", p.scenes[0].name));
    assert_eq!(h.state().scene, 1);
}

/// A pattern put in a scene past the last (a project saved with no scenes: the Ideas
/// view's click asks for scene 0 then) makes that scene with it in, one undo step.
#[test]
fn a_pattern_into_a_scene_past_the_last_makes_one() {
    let Some(mut h) = harness(false, false) else { return };
    let doc = h.state().doc.clone().unwrap();
    let n = doc.borrow().project().scenes.len();
    let steps = doc.borrow().undo_labels().len();
    h.state_mut().arrange(layout::Act::AssignPattern(n, 1, Some(0)));
    let p = project_of(&h);
    assert_eq!(p.scenes.len(), n + 1, "a scene made");
    assert_eq!(p.scenes[n].patterns, vec![(1, 0)], "with the pattern in");
    assert_eq!(doc.borrow().undo_labels().len(), steps + 1, "one undo step");
    assert_eq!(h.state().scene, n, "and focused");
}

/// Ideas view: a click shows a pattern (and puts it in the scene), a double-click on
/// a pattern in the scene takes it out, and a double-click below a group's patterns
/// makes a new one there, in the scene.
#[test]
fn ideas_view_double_clicks() {
    let Some(mut h) = harness(false, false) else { return };
    let p = project_of(&h);
    let scene = h.state().scene;
    let Some(&(g, pi)) = p.scenes[scene].patterns.iter().find(|&&(g, pi)| g < 8 && pi < 4 && p.groups[g].patterns.len() < 6) else {
        return;
    };
    click(&mut h, cell(g, pi));
    assert!(project_of(&h).scenes[scene].patterns.contains(&(g, pi)), "a click leaves it in the scene");
    assert_eq!((h.state().group, h.state().pattern), (g, pi), "and shows it");
    double_click(&mut h, cell(g, pi));
    assert!(!project_of(&h).scenes[scene].patterns.iter().any(|x| x.0 == g), "a double-click takes it out");
    double_click(&mut h, cell(g, pi));
    assert!(project_of(&h).scenes[scene].patterns.contains(&(g, pi)), "double-clicking one that wasn't in leaves it in");
    let n = p.groups[g].patterns.len();
    double_click(&mut h, cell(g, n));
    let q = project_of(&h);
    assert_eq!(q.groups[g].patterns.len(), n + 1, "{}", h.state().status);
    assert_eq!(q.groups[g].patterns[n].name, format!("Pattern {}", n + 1));
    assert!(q.scenes[scene].patterns.contains(&(g, n)), "the new pattern is in the scene");
    assert_eq!((h.state().group, h.state().pattern), (g, n));
    assert!(q.groups[g].patterns[n].notes.is_empty());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);
    assert_eq!(project_of(&h).groups[g].patterns.len(), n, "one undo step");
}

#[test]
fn song_view_edits_sections() {
    let Some(mut h) = harness(false, true) else { return };
    let p = project_of(&h);
    if p.sections.len() < 2 {
        return;
    }
    let bar = p.bar_length();
    let pos = section_header(&h, 0);
    right_click(&mut h, pos);
    let length_menu = format!("Length ({} bars) ⏵", p.sections[0].length as f64 / bar as f64);
    press(&mut h, &length_menu);
    press(&mut h, "8 bars");
    assert_eq!(project_of(&h).sections[0].length, 8 * bar);

    let pos = section_header(&h, 1);
    right_click(&mut h, pos);
    press(&mut h, "Delete");
    assert_eq!(project_of(&h).sections.len(), p.sections.len() - 1);

    // "+" after the last section adds one playing the focused scene.
    let q = project_of(&h);
    let end = section_header(&h, q.sections.len() - 1);
    let d = h.state().doc.as_ref().unwrap().borrow();
    let p2 = d.project();
    let left = 3.0 + 33.0 + 163.0 + 1.0;
    let width = WIDTH - 3.0 - left;
    let total = p2.arrangement_length() as f32 / bar as f32;
    let ppb = ((width - 40.0) / total.max(16.0)).max(12.0);
    let x = left + total * ppb + 14.0;
    drop(d);
    click(&mut h, Pos2::new(x, end.y));
    let r = project_of(&h);
    assert_eq!(r.sections.len(), q.sections.len() + 1);
    assert_eq!(r.sections.last().unwrap().scene, Some(h.state().scene));
}

#[test]
fn controller_screen_buttons_edit_scenes() {
    let Some(mut h) = scripted_harness(false) else { return };
    // Switch the controller to the Ideas view too.
    h.state_mut().sync_controller();
    let n = project_of(&h).scenes.len();
    press(&mut h, "SCENE");
    press(&mut h, "DUPLICATE");
    assert_eq!(project_of(&h).scenes.len(), n + 1);
    press(&mut h, "SCENE");
}

/// The Mix view's plug-in strip: panels for the focused sound's slots; dragging
/// the Compressor's Knee or Threshold edits them, its power button bypasses it.
#[test]
fn plugin_strip_edits_the_compressor() {
    let Some(mut h) = harness(false, false) else { return };
    h.state_mut().mixer = true;
    h.run_steps(3);
    let slots = project_of(&h).groups[0].sounds[0].slots.clone();
    let Some(ci) = slots.iter().position(|x| x.module == "Compressor" && x.plugin.is_none()) else { return };
    let panels: Vec<(usize, egui::Rect)> =
        h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).expect("the strip draws its panels");
    assert_eq!(panels.len(), slots.len(), "a panel per slot");
    let r = panels.iter().find(|p| p.0 == ci).unwrap().1;
    let param = |h: &Harness<'static, App>, f: &str| project_of(h).groups[0].sounds[0].slots[ci].param(f).unwrap_or(0.0);
    // Knee: the first knob, right of the meters.
    let (inner_l, inner_t) = (r.left() + 4.0, r.top() + 26.0);
    let knee = Pos2::new(inner_l + 152.0, inner_t + 50.0);
    let before = param(&h, "knee");
    drag(&mut h, knee, knee - egui::vec2(0.0, 40.0));
    assert!(param(&h, "knee") > before + 0.1, "knee {before} -> {}", param(&h, "knee"));
    // Threshold: its handle, dragged down (lower dB, a larger value).
    let thr = param(&h, "threshold") as f32;
    let bar_h = r.height() - 26.0 - 82.0;
    let handle = Pos2::new(inner_l + 10.0 + 30.0 + 13.0 + 6.0, inner_t + 46.0 + thr * bar_h);
    drag(&mut h, handle, handle + egui::vec2(0.0, 30.0));
    assert!(param(&h, "threshold") > thr as f64 + 0.05, "threshold {thr} -> {}", param(&h, "threshold"));
    // Bypass.
    click(&mut h, Pos2::new(r.left() + 12.0, r.top() + 11.0));
    assert!(!project_of(&h).groups[0].sounds[0].slots[ci].active);
    click(&mut h, Pos2::new(r.left() + 12.0, r.top() + 11.0));
    assert!(project_of(&h).groups[0].sounds[0].slots[ci].active);
}

/// The drum synths' strip panels: MAIN (Tune, the engine, Velocity, Decay) and
/// MODULATION (the engine's own controls) edit the module; `$DECKARD_RENDER_DIR` gets
/// a picture of both tabs of each.
#[test]
fn drum_strip_panels_edit() {
    let Some(mut h) = harness(false, false) else { return };
    layout::style(&h.ctx);
    let empty: Vec<usize> = (0..16).filter(|&s| project_of(&h).groups[0].sounds.get(s).is_some_and(|x| x.slots.is_empty())).collect();
    let dir = render_dir();
    for (n, drum) in crate::drum_ui::DRUMS.iter().enumerate() {
        let Some(&sound) = empty.get(n) else { return };
        h.state_mut().mixer = false;
        h.state_mut().sound = sound;
        h.run_steps(2);
        h.state_mut().add_slot(layout::AddWhat::Instrument(drum));
        h.state_mut().mixer = true;
        h.run_steps(5);
        let slots = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[sound].slots.clone();
        let si = slots(&h).iter().position(|x| x.module == *drum).expect("added");
        let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap();
        let r = panels.iter().find(|p| p.0 == si).unwrap().1;
        if let Some(d) = &dir {
            h.render().unwrap().save(d.join(format!("strip-{}-main.png", drum.to_lowercase()))).unwrap();
        }
        // The body under the header bar, in the QML's 632×216.
        let body = egui::Rect::from_min_max(Pos2::new(r.left() + 4.0, r.top() + 26.0), Pos2::new(r.right() - 4.0, r.bottom()));
        let k = body.height() / 216.0;
        let at = |x: f32, y: f32| body.min + egui::vec2(x, y) * k;
        let param = |h: &Harness<'static, App>, f: &str| slots(h)[si].param(f);
        if *drum == "Snare" {
            // The engine's right arrow: Volt → Bit.
            click(&mut h, at(233.0 + 82.0, 47.0) + egui::vec2(147.0 * k / 2.0 + 6.0, 0.0));
            assert_eq!(param(&h, "mode"), Some(1.0), "the engine steps");
            click(&mut h, at(233.0 + 82.0, 47.0) - egui::vec2(147.0 * k / 2.0 + 6.0, 0.0));
            // Tune: drag up.
            let tune = at(31.0 + 62.0, 43.0 + 40.0);
            drag(&mut h, tune, tune - egui::vec2(0.0, 60.0));
            assert!(param(&h, "tune").unwrap() > 0.55, "tune {:?}", param(&h, "tune"));
        }
        // MODULATION.
        let tabs = Pos2::new(body.right() - 40.0, body.top() + 18.0);
        click(&mut h, tabs);
        if let Some(d) = &dir {
            h.render().unwrap().save(d.join(format!("strip-{}-modulation.png", drum.to_lowercase()))).unwrap();
        }
        if *drum == "Snare" {
            // Volt's Color knob (NOISE).
            let spots = layout::strip::editor::knob_spots(&slots(&h)[si], (0, sound, si));
            let (at, field) = spots.iter().find(|(_, f)| f == "noise_color").expect("Color shows for Volt");
            let c = r.min + *at;
            let before = param(&h, field).unwrap_or(0.5);
            drag(&mut h, c, c - egui::vec2(0.0, 40.0));
            assert!(param(&h, field).unwrap() > before, "Color turns");
        }
    }
}

/// A sound takes one instrument, in its first slot: a second is refused, effects
/// still go after it.
#[test]
fn one_instrument_per_sound() {
    let Some(mut h) = harness(false, false) else { return };
    let Some(sound) = (0..16).find(|&s| project_of(&h).groups[0].sounds.get(s).is_some_and(|x| x.slots.is_empty())) else { return };
    h.state_mut().sound = sound;
    h.state_mut().add_slot(layout::AddWhat::Instrument("Kick"));
    h.state_mut().add_slot(layout::AddWhat::Instrument("Snare"));
    h.state_mut().add_slot(layout::AddWhat::Sampler);
    h.state_mut().add_slot(layout::AddWhat::Effect("Chorus"));
    h.run_steps(2);
    let modules: Vec<String> = project_of(&h).groups[0].sounds[sound].slots.iter().map(|x| x.module.clone()).collect();
    assert_eq!(modules, ["Kick", "Chorus"]);
}

/// The Poly Synth's strip panel: four tabs (Osc A, Osc B, LFO, Mod) laid out as
/// Maschine's; its switches and knobs edit the synth. `$DECKARD_RENDER_DIR` gets a
/// picture of each tab.
/// A double-click on a knob types its value, as it's shown; ⌘-click sets its default.
#[test]
fn poly_synth_knob_takes_a_typed_value() {
    let Some(mut h) = harness(false, false) else { return };
    layout::style(&h.ctx);
    let Some(sound) = (0..16).find(|&s| project_of(&h).groups[0].sounds.get(s).is_some_and(|x| x.slots.is_empty())) else { return };
    h.state_mut().sound = sound;
    h.run_steps(2);
    h.state_mut().add_slot(layout::AddWhat::Instrument("PolySynth"));
    h.state_mut().mixer = true;
    h.run_steps(5);
    let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap();
    let r = panels.iter().find(|p| p.0 == 0).unwrap().1;
    let body = egui::Rect::from_min_max(Pos2::new(r.left() + 4.0, r.top() + 26.0), Pos2::new(r.right() - 4.0, r.bottom()));
    let k = body.height() / 216.0;
    let tabs = h.ctx.data(|d| d.get_temp::<Vec<egui::Rect>>(egui::Id::new("deckard-poly-tabs"))).unwrap();
    click(&mut h, tabs[1].center());
    h.run_steps(3);
    let c = body.min + egui::vec2(506.0 + 20.5, 76.0 + 20.5) * k;
    let cutoff = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[sound].slots[0].param("filter_cutoff").unwrap();
    let double = |h: &mut Harness<'static, App>, modifiers: egui::Modifiers, times: usize| {
        // (Long enough after the last click not to count with it.)
        h.run_steps(40);
        h.hover_at(c);
        h.step();
        for _ in 0..times {
            for pressed in [true, false] {
                h.event(Event::PointerButton { pos: c, button: PointerButton::Primary, pressed, modifiers });
                h.step();
            }
        }
        h.run_steps(2);
    };
    double(&mut h, Default::default(), 2);
    assert_eq!(cutoff(&h), 1.0, "opening the box changes nothing");
    // Esc leaves it.
    h.event(Event::Text("10".into()));
    h.step();
    h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    h.run_steps(3);
    assert_eq!(cutoff(&h), 1.0, "Esc cancels");
    // Enter keeps it, read as the knob shows it (a percentage).
    double(&mut h, Default::default(), 2);
    h.event(Event::Text("25".into()));
    h.step();
    h.event(Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    h.run_steps(3);
    assert!((cutoff(&h) - 0.25).abs() < 0.002, "typed 25 %: {}", cutoff(&h));
    // ⌘-click: its default.
    h.event(Event::ModifiersChanged(egui::Modifiers::COMMAND));
    double(&mut h, egui::Modifiers::COMMAND, 1);
    h.event(Event::ModifiersChanged(Default::default()));
    assert_eq!(cutoff(&h), 1.0, "its default");
}

#[test]
fn poly_synth_strip_panel_edits() {
    let Some(mut h) = harness(false, false) else { return };
    layout::style(&h.ctx);
    let Some(sound) = (0..16).find(|&s| project_of(&h).groups[0].sounds.get(s).is_some_and(|x| x.slots.is_empty())) else { return };
    h.state_mut().sound = sound;
    h.run_steps(2);
    h.state_mut().add_slot(layout::AddWhat::Instrument("PolySynth"));
    h.state_mut().mixer = true;
    h.run_steps(5);
    let slot = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[sound].slots[0].clone();
    assert_eq!(slot(&h).module, "PolySynth");
    let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap();
    let r = panels.iter().find(|p| p.0 == 0).unwrap().1;
    let body = egui::Rect::from_min_max(Pos2::new(r.left() + 4.0, r.top() + 26.0), Pos2::new(r.right() - 4.0, r.bottom()));
    let k = body.height() / 216.0;
    let at = |x: f32, y: f32| body.min + egui::vec2(x, y) * k;
    // Osc A: Saw (on) off; then Tune up.
    assert_eq!(slot(&h).param("osc_a_saw").unwrap_or(1.0), 1.0);
    click(&mut h, at(179.0 + 9.0, 126.0 + 19.0));
    assert_eq!(slot(&h).param("osc_a_saw"), Some(0.0), "Saw switches off");
    let tune = at(38.0 + 20.5, 118.0 + 20.5);
    drag(&mut h, tune, tune - egui::vec2(0.0, 40.0));
    assert!(slot(&h).param("osc_a_tune").unwrap_or(0.0) >= 1.0, "Tune goes up in semitones");
    let dir = render_dir();
    let tabs = h.ctx.data(|d| d.get_temp::<Vec<egui::Rect>>(egui::Id::new("deckard-poly-tabs")));
    for (n, name) in ["osc-a", "osc-b", "lfo", "mod"].iter().enumerate() {
        if let Some(t) = tabs.as_ref().and_then(|t| t.get(n)) {
            click(&mut h, t.center());
        }
        if let Some(d) = &dir {
            h.render().unwrap().save(d.join(format!("strip-polysynth-{name}.png"))).unwrap();
        }
    }
    assert!(tabs.is_some_and(|t| t.len() == 4), "four tabs");
}

/// The Sampler's strip panel: drag S to move the play start; the FX page's filter
/// handle sets cutoff and resonance, the envelope's attack handle the attack.
/// `$DECKARD_RENDER_DIR` gets a picture of each page.
#[test]
fn sampler_strip_panel_edits() {
    let Some(mut h) = harness(false, false) else { return };
    layout::style(&h.ctx);
    h.state_mut().mixer = true;
    h.run_steps(5);
    let sound = |h: &Harness<'static, App>| project_of(h).groups[0].sounds[0].clone();
    let Some(si) = sound(&h).slots.iter().position(|x| x.module == "Sampler" && x.plugin.is_none()) else { return };
    let panels: Vec<(usize, egui::Rect)> = h.ctx.data(|d| d.get_temp(egui::Id::new("deckard-strip-panels"))).unwrap();
    let r = panels.iter().find(|p| p.0 == si).unwrap().1;
    let dir = std::env::var_os("DECKARD_RENDER_DIR").map(PathBuf::from);
    let shot = |h: &mut Harness<'static, App>, name: &str| {
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join(name)).unwrap();
        }
    };
    let body_top = r.top() + 26.0;
    let area = egui::Rect::from_min_max(Pos2::new(r.left() + 16.0, body_top + 36.0), Pos2::new(r.right() - 16.0, r.bottom() - 6.0));
    // Main: drag S right.
    assert!(h.state().waveform.is_some(), "the zone's waveform loads");
    shot(&mut h, "strip-sampler-main.png");
    let lo = sound(&h).sampler.unwrap().zones[0].play_range.0;
    let wave_bottom = area.top() + (area.height() - 43.0);
    let s_marker = Pos2::new(area.left() + 1.0, wave_bottom - 9.0);
    drag(&mut h, s_marker, s_marker + egui::vec2(120.0, 0.0));
    let moved = sound(&h).sampler.unwrap().zones[0].play_range.0;
    assert!(moved > lo, "play start {lo} -> {moved}");
    // FX: the tab, then LP2 and ADSR.
    click(&mut h, Pos2::new(r.right() - 24.0, body_top + 17.0));
    let smp = |h: &Harness<'static, App>| sound(h).sampler.unwrap();
    let top = area.top();
    let fx = area.left() + 334.0;
    let right_arrow = |center_x: f32| Pos2::new(center_x + 34.0, top + 6.0);
    let left_arrow = |center_x: f32| Pos2::new(center_x - 34.0, top + 6.0);
    for _ in 0..4 {
        click(&mut h, left_arrow(fx + 180.0));
    }
    assert_eq!(smp(&h).get("filter"), 0.0, "the filter cycler steps down to Off");
    click(&mut h, right_arrow(fx + 180.0));
    assert_eq!(smp(&h).get("filter"), 1.0, "LP2");
    let disp = egui::Rect::from_min_size(Pos2::new(fx + 12.0, top + 20.0), egui::vec2(206.0, 70.0));
    let to = Pos2::new(disp.left() + disp.width() * 0.3, disp.bottom() - disp.height() * 0.6);
    drag(&mut h, disp.center(), to);
    let (c, q) = (smp(&h).get("cutoff"), smp(&h).get("resonance"));
    assert!((c - 0.3).abs() < 0.03 && (q - 0.6).abs() < 0.03, "cutoff {c} resonance {q}");
    let env_x = fx + 230.0;
    let w = area.right() - env_x;
    for _ in 0..2 {
        click(&mut h, right_arrow(env_x + w - 56.0));
    }
    assert_eq!(smp(&h).get("envelope_type"), 2.0, "ADSR");
    let inner = egui::Rect::from_min_size(Pos2::new(env_x + 12.0, top + 20.0), egui::vec2(w - 33.0, 70.0)).shrink(10.0);
    let a = smp(&h).get("volume_envelope/attack") as f32;
    let attack = Pos2::new(inner.left() + inner.width() * a / 4.0, inner.top());
    drag(&mut h, attack, Pos2::new(inner.left() + inner.width() * 0.125, inner.top()));
    let a2 = smp(&h).get("volume_envelope/attack");
    assert!((a2 - 0.5).abs() < 0.05, "attack {a} -> {a2}");
    shot(&mut h, "strip-sampler-fx.png");
}
