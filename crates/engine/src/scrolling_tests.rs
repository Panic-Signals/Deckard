use super::scrolling::Scrolling;
use std::path::PathBuf;

fn wav_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("deckard-scrolling-test-{}-{name}.wav", std::process::id()))
}

#[test]
fn capture_is_prepared_before_push() {
    let scrolling = Scrolling::default();
    scrolling.sync(true, 1.0);
    scrolling.prepare(48_000);

    let block = vec![0.25; 256 * 2];
    scrolling.push(&block, 256, 48_000);

    assert_eq!(scrolling.available_frames(), 256);
    assert_eq!(scrolling.snapshot(1).2, 48_000);
}

#[test]
fn repeated_prepare_and_invalid_pushes_are_harmless() {
    let scrolling = Scrolling::default();
    scrolling.sync(true, 1.0);
    scrolling.prepare(48_000);
    scrolling.prepare(48_000);
    scrolling.push(&[], 0, 48_000);
    scrolling.push(&[0.25, 0.25], 1, 0);
    assert_eq!(scrolling.available_frames(), 0);
}

#[test]
fn changing_capture_length_before_prepare_keeps_zero_rate_empty() {
    let scrolling = Scrolling::default();
    scrolling.sync(true, 2.0);
    assert_eq!(scrolling.available_frames(), 0);
    scrolling.prepare(48_000);
    scrolling.sync(true, 3.0);
    assert_eq!(scrolling.available_frames(), 0);
}

#[test]
fn an_unprepared_rate_does_not_allocate_on_push() {
    let scrolling = Scrolling::default();
    scrolling.sync(true, 1.0);

    let block = vec![0.25; 256 * 2];
    scrolling.push(&block, 256, 48_000);

    assert_eq!(scrolling.available_frames(), 0);
}

#[test]
fn snapshots_report_peaks_and_clip_to_available_audio() {
    let scrolling = Scrolling::default();
    scrolling.sync(true, 1.0);
    scrolling.prepare(48_000);
    scrolling.push(&[0.25, -0.5, 0.75, -1.0], 2, 48_000);

    assert_eq!(scrolling.snapshot_range(0, 2, 2), Some(vec![(-0.5, 0.25), (-1.0, 0.75)]));
    assert_eq!(scrolling.snapshot_range(100, 2, 2), Some(vec![(0.0, 0.0); 2]));
    assert_eq!(scrolling.snapshot_range(0, 2, 0), Some(Vec::new()));
}

#[test]
fn pause_disable_and_clear_stop_capture() {
    let scrolling = Scrolling::default();
    scrolling.sync(true, 1.0);
    scrolling.prepare(48_000);
    scrolling.set_paused(true);
    scrolling.push(&[1.0, 1.0], 1, 48_000);
    assert_eq!(scrolling.available_frames(), 0);

    scrolling.set_paused(false);
    scrolling.push(&[1.0, 1.0], 1, 48_000);
    assert_eq!(scrolling.available_frames(), 1);
    scrolling.sync(false, 1.0);
    scrolling.push(&[1.0, 1.0], 1, 48_000);
    assert_eq!(scrolling.available_frames(), 0);
    scrolling.clear();
    assert_eq!(scrolling.available_frames(), 0);
}

#[test]
fn export_wav_applies_silence_cleanup_and_fade() {
    let scrolling = Scrolling::default();
    scrolling.sync(true, 1.0);
    scrolling.prepare(1_000);
    scrolling.push(&[0.0, 0.0, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0], 4, 1_000);
    let path = wav_path("cleanup");

    scrolling.export_wav(&path, 0, 4, true, 1).unwrap();
    let reader = hound::WavReader::open(&path).unwrap();
    assert_eq!(reader.spec().sample_rate, 1_000);
    assert_eq!(reader.duration(), 2);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn export_wav_rejects_empty_and_silent_selections() {
    let scrolling = Scrolling::default();
    scrolling.sync(true, 1.0);
    scrolling.prepare(1_000);
    let empty = wav_path("empty");
    assert_eq!(scrolling.export_wav(&empty, 0, 0, false, 0).unwrap_err(), "the selection is empty");

    scrolling.push(&[0.0, 0.0], 1, 1_000);
    let silent = wav_path("silent");
    assert_eq!(scrolling.export_wav(&silent, 0, 1, false, 0).unwrap_err(), "the selection is silent");
    assert!(!silent.exists());
}

#[test]
fn export_wav_rejects_selection_outside_capture() {
    let scrolling = Scrolling::default();
    scrolling.sync(true, 1.0);
    scrolling.prepare(1_000);
    scrolling.push(&[0.5, 0.5], 1, 1_000);
    let path = wav_path("outside");
    assert_eq!(scrolling.export_wav(&path, 10, 1, false, 0).unwrap_err(), "the selection is no longer in the capture buffer");
    assert!(!path.exists());
}
