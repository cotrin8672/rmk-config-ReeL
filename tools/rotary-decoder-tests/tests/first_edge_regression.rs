//! Synthetic counterexample: a cancelled A bounce is not click direction.
//! The full raw path has a net CCW/CW movement, independently of its first edge.
use rotary_decoder_tests::rotary_decoder::{ClockedDetentDecoder, DEBOUNCE_SAMPLES, Detent};

#[test]
fn cancelled_first_a_edge_does_not_override_the_complete_movement() {
    for start in 0_u8..4 {
        // A briefly departs and returns, then B and A complete the click.
        // The first two transitions cancel. Only the final two set direction.
        let raw = [start ^ 2, start, start ^ 1, start ^ 3];
        let expected = if start & 2 != (start & 1) << 1 {
            Detent::Clockwise
        } else {
            Detent::CounterClockwise
        };
        let mut decoder = ClockedDetentDecoder::new(start & 2 != 0, start & 1 != 0);
        let mut events = Vec::new();
        for state in raw
            .into_iter()
            .chain(std::iter::repeat_n(start ^ 3, DEBOUNCE_SAMPLES as usize))
        {
            if let Some(event) = decoder.update(state & 2 != 0, state & 1 != 0) {
                events.push(event);
            }
        }
        assert_eq!(events, [expected], "start={start:02b}");
    }
}
