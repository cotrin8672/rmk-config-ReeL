//! Synthetic regressions; all raw single-bit edges reach the ideal capture model.
mod support;
use rotary_decoder_tests::rotary_decoder::Detent;
use support::Capture;

#[test]
fn cancelled_first_a_edge_does_not_override_complete_movement() {
    for start in 0..4 {
        let mut capture = Capture::new(start);
        for (time, state) in [
            (10, start ^ 2),
            (20, start),
            (30, start ^ 1),
            (40, start ^ 3),
        ] {
            capture.edge(time, state);
        }
        capture.advance(1040);
        let expected = if start >> 1 == start & 1 {
            Detent::CounterClockwise
        } else {
            Detent::Clockwise
        };
        assert_eq!(capture.decode(), [expected]);
    }
}

#[test]
fn cancelled_a_window_cannot_donate_movement_to_next_click() {
    // Two CW A edges return A to its original level while moving around
    // an entire quadrature cycle. Consume their even interval before CCW.
    let mut capture = Capture::new(0);
    for (time, state) in [(10, 2), (20, 3), (30, 1), (40, 0)] {
        capture.edge(time, state);
    }
    capture.advance(1030);
    assert!(capture.decode().is_empty());
    capture.edge(1040, 1);
    capture.edge(1050, 3);
    capture.advance(2050);
    assert_eq!(capture.decode(), [Detent::CounterClockwise]);
}
