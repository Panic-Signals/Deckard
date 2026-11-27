//! PAD MODE's base key on the MK3.

use egui_kittest::Harness;

/// The app with a new project, as a test drives it: no project file is needed, so it
/// runs wherever the tests do.
fn fresh() -> Harness<'static, crate::App> {
    let mut app = crate::App::default();
    app.new_project();
    let mut h = Harness::builder()
        .with_size(eframe::egui::vec2(1500.0, 930.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut crate::App| app.frame(ui), app);
    h.run_steps(3);
    h
}

/// PAD MODE's octave and semitone buttons on the MK3 move the focused sound's base key
/// in the project: the note its pad plays.
#[test]
fn the_mk3s_pad_mode_octave_and_semitone_set_the_base_key() {
    use deckard_mk3::Event as Mk3;
    let press = |h: &mut Harness<'static, crate::App>, name: &'static str| {
        let mut reqs = Vec::new();
        h.state_mut().mk3_events(vec![Mk3::Button(name, true), Mk3::Button(name, false)], &mut reqs);
        h.run_steps(3);
    };
    let mut h = fresh();
    let key = |h: &Harness<'static, crate::App>| {
        let a = h.state();
        a.doc.as_ref().unwrap().borrow().project().groups[a.group].sounds[a.sound].channel.base_key
    };
    let was = key(&h);
    press(&mut h, "BUTTON_PAD_MODE");
    press(&mut h, "BUTTON_SCREEN_6");
    assert_eq!(key(&h), was + 12, "OCTAVE+");
    press(&mut h, "BUTTON_SCREEN_7");
    assert_eq!(key(&h), was + 11, "SEMITONE-");
}
