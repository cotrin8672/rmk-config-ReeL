//! Synthetic counterexample: a cancelled A bounce is not click direction.
//! The full raw path has a net CCW/CW movement, independently of its first edge.
use rotary_decoder_tests::rotary_decoder::{ClockedDetentDecoder, DEBOUNCE_SAMPLES, Detent};

#[test]
fn aborted_a_window_cannot_donate_direction_to_the_next_click() {
    // Synthetic arrival chatter: two valid old-direction steps followed
    // by an ambiguous return to the accepted A level. This is not a click.
    // Start the reversal exactly when A debounce cancels that window,
    // before the separate all-input-idle counter expires its residue.
    for start in 0_u8..4 {
        for prior in [None, Some(false), Some(true)] {
            for rest in [0, 1, 16, 200] {
                let old = if start & 2 == (start & 1) << 1 {
                    Detent::Clockwise
                } else {
                    Detent::CounterClockwise
                };
                let reverse = match old {
                    Detent::Clockwise => Detent::CounterClockwise,
                    Detent::CounterClockwise => Detent::Clockwise,
                };
                let mut decoder = ClockedDetentDecoder::new(start & 2 != 0, start & 1 != 0);
                if let Some(cw) = prior {
                    // Two completed clicks return to the starting phase with a
                    // known previous direction, using only public input samples.
                    for phase in [start, start ^ 3] {
                        let a_first = cw == (phase & 2 == (phase & 1) << 1);
                        let middle = phase ^ if a_first { 2 } else { 1 };
                        let expected = if cw {
                            Detent::Clockwise
                        } else {
                            Detent::CounterClockwise
                        };
                        let mut seed_events = Vec::new();
                        for state in [middle].into_iter().chain(std::iter::repeat_n(
                            phase ^ 3,
                            DEBOUNCE_SAMPLES as usize + 1,
                        )) {
                            if let Some(event) = decoder.update(state & 2 != 0, state & 1 != 0) {
                                seed_events.push(event);
                            }
                        }
                        assert_eq!(seed_events, [expected]);
                    }
                }
                let mut events = Vec::new();
                let samples = [start ^ 2, start ^ 3]
                    .into_iter()
                    .chain(std::iter::repeat_n(start, DEBOUNCE_SAMPLES as usize + rest))
                    // B departs in the reverse direction; the next sample skips
                    // the middle state of the A edge / B bounce pair.
                    .chain([start ^ 1])
                    .chain(std::iter::repeat_n(start ^ 2, DEBOUNCE_SAMPLES as usize));
                for state in samples {
                    if let Some(event) = decoder.update(state & 2 != 0, state & 1 != 0) {
                        events.push(event);
                    }
                }
                assert_eq!(
                    events,
                    [reverse],
                    "start={start:02b}, prior={prior:?}, rest={rest}"
                );
            }
        }
    }
}

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
