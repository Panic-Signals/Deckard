use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::path::PathBuf;

fn test_wav(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("deckard-scrolling-drop-{}-{name}.wav", std::process::id()));
    let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for sample in [0i16, 1_000, -1_000, 0] {
        writer.write_sample(sample).unwrap();
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
    path
}

fn audio_preferences() -> Harness<'static, crate::App> {
    let app = crate::App { prefs: Some(2), ..Default::default() };
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut crate::App| {
            let ctx = ui.ctx().clone();
            app.preferences(&ctx);
        },
        app,
    );
    harness.run_steps(3);
    harness
}

fn enabled_audio_preferences() -> Harness<'static, crate::App> {
    let mut app = crate::App { prefs: Some(2), ..Default::default() };
    app.settings.scrolling_sampler = true;
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut crate::App| {
            let ctx = ui.ctx().clone();
            app.preferences(&ctx);
        },
        app,
    );
    harness.run_steps(3);
    harness
}

#[test]
fn audio_preferences_show_scrolling_sampler_controls() {
    let harness = audio_preferences();
    assert!(harness.query_by_label("Scrolling sampler").is_some());
    assert!(harness.query_by_label("Capture recent master output").is_some());
    assert!(harness.query_by_label("Dropped selection instrument").is_some());
}

#[test]
fn audio_preferences_toggle_capture_and_change_buffer() {
    let mut harness = audio_preferences();
    harness.get_by_label("Capture recent master output").click();
    harness.run_steps(2);
    assert!(harness.state().settings.scrolling_sampler);
    harness.get_by_value("120 seconds").click();
    harness.run_steps(1);
    harness.get_by_label("30 seconds").click();
    harness.run_steps(2);
    assert_eq!(harness.state().settings.scrolling_seconds, 30);
    harness.get_by_label("Capture recent master output").click();
    harness.run_steps(2);
    assert!(!harness.state().settings.scrolling_sampler);
}

#[test]
fn disabled_capture_hides_buffer_controls() {
    let harness = audio_preferences();
    assert!(harness.query_by_label("Buffer").is_none());
    assert!(harness.query_by_label("Waveform colour").is_none());
}

#[test]
fn enabled_capture_shows_selection_and_drop_controls() {
    let harness = enabled_audio_preferences();
    assert!(harness.query_by_label("Buffer").is_some());
    assert!(harness.query_by_label("Waveform colour").is_some());
    assert!(harness.query_by_label("Selection cleanup").is_some());
    assert!(harness.query_by_label("Fade to zero crossing").is_some());
    assert!(harness.query_by_label("Fade duration").is_some());
    assert!(harness.query_by_label("Dropped selection instrument").is_some());
}

#[test]
fn scrolling_sampler_preferences_update_saved_choices() {
    let mut harness = enabled_audio_preferences();
    harness.get_by_value("120 seconds").click();
    harness.run_steps(1);
    harness.get_by_label("300 seconds").click();
    harness.get_by_value("Theme").click();
    harness.run_steps(1);
    harness.get_by_label("Custom").click();
    harness.run_steps(1);
    harness.get_by_label("Remove leading and trailing silence").click();
    harness.run_steps(1);
    harness.run_steps(3);

    assert_eq!(harness.state().settings.scrolling_seconds, 300);
    assert_eq!(harness.state().settings.scrolling_palette, 1);
    assert!(harness.state().settings.scrolling_remove_silence);
    assert!(harness.state().settings.scrolling_fade_enabled);
    assert_eq!(harness.state().settings.scrolling_fade_ms, 5);
    assert!(harness.state().settings.scrolling_drop_audio);
}

#[test]
fn scrolling_sampler_preferences_toggle_fade() {
    let mut harness = enabled_audio_preferences();
    harness.get_by_label("Enable").click();
    harness.run_steps(3);
    assert!(!harness.state().settings.scrolling_fade_enabled);
}

#[test]
fn scrolling_sampler_preferences_choose_drop_mode() {
    let mut harness = enabled_audio_preferences();
    harness.get_by_value("Audio").click();
    harness.run_steps(1);
    harness.get_by_label("Sampler").click();
    harness.run_steps(3);
    assert!(!harness.state().settings.scrolling_drop_audio);
}

#[test]
fn scrolling_sampler_preferences_choose_fade_and_buffer_values() {
    let mut harness = enabled_audio_preferences();
    harness.get_by_value("5 ms").click();
    harness.run_steps(1);
    harness.get_by_label("10 ms").click();
    harness.get_by_value("256 samples (5.3 ms)").click();
    harness.run_steps(1);
    harness.get_by_label("512 samples (10.7 ms)").click();
    harness.run_steps(3);
    assert!(harness.state().settings.scrolling_fade_enabled);
    assert_eq!(harness.state().settings.scrolling_fade_ms, 10);
    assert_eq!(harness.state().settings.buffer_frames, 512);
}

#[test]
fn scrolling_sampler_preferences_render_custom_color_control() {
    let mut harness = enabled_audio_preferences();
    harness.get_by_value("Theme").click();
    harness.run_steps(1);
    harness.get_by_label("Custom").click();
    harness.run_steps(3);
    assert!(harness.query_by_label("Custom colour").is_some());
}

#[test]
fn dropping_scrolling_sample_loads_sampler_or_loop() {
    let path = test_wav("valid");

    let mut sampler = crate::App::default();
    sampler.new_project();
    sampler.settings.scrolling_drop_audio = false;
    sampler.drop_scrolling_sample(0, 0, &path);
    assert!(sampler.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].sampler.as_ref().is_some_and(|s| !s.zones.is_empty()));

    let mut audio = crate::App::default();
    audio.new_project();
    audio.settings.scrolling_drop_audio = true;
    audio.drop_scrolling_sample(0, 0, &path);
    assert!(audio.doc.as_ref().unwrap().borrow().project().groups[0].sounds[0].audio.as_ref().is_some_and(|audio| audio.takes.len() == 1));

    std::fs::remove_file(path).unwrap();
}

#[test]
fn dropping_invalid_scrolling_sample_reports_error() {
    let mut app = crate::App::default();
    app.new_project();
    let path = std::env::temp_dir().join(format!("deckard-missing-scroll-{}.wav", std::process::id()));
    app.drop_scrolling_sample(0, 0, &path);
    assert!(app.status.starts_with("can't read "));
}

#[test]
fn dropping_valid_sample_without_project_reports_load_error() {
    let path = test_wav("no-project");
    let mut app = crate::App::default();
    app.settings.scrolling_drop_audio = true;
    app.drop_scrolling_sample(0, 0, &path);
    assert!(app.status.starts_with("can't load "));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dropping_valid_sample_into_audio_sound_adds_a_take() {
    let path = test_wav("existing-audio");
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().audio_only(0, 0).unwrap();
    app.settings.scrolling_drop_audio = true;
    app.drop_scrolling_sample(0, 0, &path);
    assert_eq!(doc.borrow().project().groups[0].sounds[0].audio.as_ref().unwrap().takes.len(), 1);
    std::fs::remove_file(path).unwrap();
}
