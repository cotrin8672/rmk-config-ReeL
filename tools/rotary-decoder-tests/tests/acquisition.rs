//! Frame timing and delayed consumers, not proof of peripheral timing.
mod support;
use rotary_decoder_tests::rotary_decoder::{ClockedDetentDecoder, Detent};
use support::Capture;

#[test]
fn b_chatter_does_not_postpone_a_deadline_or_add_frames() {
    let mut capture = Capture::new(0);
    capture.edge(10, 2);
    for time in (20..2000).step_by(10) {
        capture.edge(time, capture.state ^ 1);
    }
    capture.advance(100_000);
    assert_eq!(capture.frames.len(), 1);
    assert_eq!(capture.decode(), [Detent::Clockwise]);
}
#[test]
fn every_a_edge_restarts_quiet_time_and_bounce_returns_cancel() {
    let mut capture = Capture::new(0);
    capture.edge(10, 2);
    capture.edge(900, 0);
    capture.advance(1899);
    assert!(capture.frames.is_empty());
    capture.advance(1900);
    assert_eq!(capture.frames.len(), 1);
    assert!(capture.decode().is_empty());
    capture.advance(100_000);
    assert_eq!(capture.frames.len(), 1);
}
#[test]
fn cpu_delay_keeps_opposite_clicks_separate() {
    let mut capture = Capture::new(0);
    capture.edge(10, 2);
    capture.edge(20, 3);
    capture.edge(2010, 2);
    capture.edge(2020, 0);
    capture.advance(3020);
    assert_eq!(
        capture.decode(),
        [Detent::Clockwise, Detent::CounterClockwise]
    );
    // A capture overwritten before IRQ service must not merge the clicks.
    assert!(
        ClockedDetentDecoder::new()
            .update(capture.frames[1])
            .is_err()
    );
}
