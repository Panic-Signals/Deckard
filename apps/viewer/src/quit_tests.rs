//! Quitting: what the window does on its way out.

use super::*;

/// Quitting asks the MK3 thread for a black frame. The thread itself is not started.
#[test]
fn quitting_blanks_the_maschine_screens() {
    let mut app = App::default();
    app.mk3 = Some(mk3_io::Mk3Io::detached());
    eframe::App::on_exit(&mut app);
    let px = app.mk3.as_ref().unwrap().queued_frame().expect("a frame is queued");
    assert!(px.iter().all(|p| *p == [0, 0, 0, 255]));
}

#[test]
fn confirmed_quit_bypasses_periodic_work_and_keeps_close_requested() {
    let mut app = App::default();
    let ctx = eframe::egui::Context::default();
    ctx.begin_pass(Default::default());
    assert!(!app.finish_quit(&ctx));
    app.proceed(&ctx, Pending::Quit);
    assert!(app.finish_quit(&ctx));
    assert!(app.quitting);
    let mut output = ctx.end_pass();
    output.textures_delta.clear();
    assert!(
        output.viewport_output[&eframe::egui::ViewportId::ROOT].commands.iter().any(|c| matches!(c, eframe::egui::ViewportCommand::Close))
    );
}
