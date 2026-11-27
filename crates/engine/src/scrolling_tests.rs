use super::scrolling::Scrolling;

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
fn an_unprepared_rate_does_not_allocate_on_push() {
    let scrolling = Scrolling::default();
    scrolling.sync(true, 1.0);

    let block = vec![0.25; 256 * 2];
    scrolling.push(&block, 256, 48_000);

    assert_eq!(scrolling.available_frames(), 0);
}
