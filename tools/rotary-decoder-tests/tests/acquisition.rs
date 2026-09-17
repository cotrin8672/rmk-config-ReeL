//! Counterexamples for the acquisition paths, using the real decoder.
//! These are source-level failure demonstrations, NOT captured device traces.
//! A/B reads and wake scheduling are modelled explicitly; GPIO/IRQ hardware
//! itself is not executed by these host tests.

use rotary_decoder_tests::rotary_decoder::{ClockedDetentDecoder, DEBOUNCE_SAMPLES, Detent};

const SETTLE: usize = DEBOUNCE_SAMPLES as usize + 2;

fn feed(d: &mut ClockedDetentDecoder, state: u8, samples: usize) -> Vec<Detent> {
    (0..samples)
        .filter_map(|_| d.update(state & 2 != 0, state & 1 != 0))
        .collect()
}

fn after_cw() -> ClockedDetentDecoder {
    let mut d = ClockedDetentDecoder::new(true, true);
    assert_eq!(feed(&mut d, 1, SETTLE), [Detent::Clockwise]);
    assert!(feed(&mut d, 0, SETTLE).is_empty());
    assert!(d.is_idle());
    d
}

#[test]
fn separate_port_reads_can_invent_the_opposite_gray_path() {
    // Physical CW: 11 -> 01 -> 00. Current peripheral.rs reads A and B
    // through two independent GPIO IN loads. An interrupt between those
    // loads can make A come from 11 and B from 00, inventing state 10.
    let physical = [3_u8, 1, 0];
    let torn = (physical[0] & 2) | (physical[2] & 1);
    assert_eq!(torn, 2);
    assert!(!physical.contains(&torn));

    let mut current = ClockedDetentDecoder::new(true, true);
    let mut events = feed(&mut current, torn, 1);
    events.extend(feed(&mut current, 0, SETTLE));
    assert_eq!(events, [Detent::CounterClockwise]);

    // A single port load cannot invent 10. If both edges still fall between
    // coherent reads, however, it loses order; atomicity alone is not a fix.
    let mut atomic_but_late = ClockedDetentDecoder::new(true, true);
    assert!(feed(&mut atomic_but_late, 0, SETTLE).is_empty());
    let mut complete = ClockedDetentDecoder::new(true, true);
    assert!(feed(&mut complete, 1, 1).is_empty());
    assert_eq!(feed(&mut complete, 0, SETTLE), [Detent::Clockwise]);
}

#[test]
fn edge_wait_rearmed_from_current_level_can_hide_a_reversal_departure() {
    // Last decoded sample: 00. Then B rises before wait_for_any_edge is
    // first polled. embassy-nrf 0.11 now sees B=1 and arms SENSE=Low, so
    // that departure is not delivered to the decoder. A rises later.
    // The next decoded sample is 11, although physical CCW was 00->01->11.
    let mut current = after_cw();
    assert_eq!(feed(&mut current, 3, SETTLE), [Detent::Clockwise]);

    // A level wait relative to the LAST DECODED state would already be
    // ready at 01. Given service before the next edge, direction survives.
    let mut last_decoded_level_wait = after_cw();
    assert!(feed(&mut last_decoded_level_wait, 1, 1).is_empty());
    assert_eq!(
        feed(&mut last_decoded_level_wait, 3, SETTLE),
        [Detent::CounterClockwise]
    );

    // Even that wait cannot recover two edges that both precede service.
    let mut delayed_service = after_cw();
    assert_eq!(feed(&mut delayed_service, 3, SETTLE), [Detent::Clockwise]);
}

#[test]
fn first_or_last_edge_timestamps_are_not_a_complete_bounce_history() {
    // First-edge latch (615f82b): a harmless B pulse at rest comes first,
    // then the real CW click. Stored "first B before first A" is not the
    // order of the two net transitions of that click.
    let raw = [3_u8, 2, 3, 1, 0];
    let mut complete = ClockedDetentDecoder::new(true, true);
    for state in &raw[1..raw.len() - 1] {
        assert!(feed(&mut complete, *state, 1).is_empty());
    }
    assert_eq!(feed(&mut complete, 0, SETTLE), [Detent::Clockwise]);
    let first_a = raw.windows(2).position(|s| (s[0] ^ s[1]) & 2 != 0).unwrap();
    let first_b = raw.windows(2).position(|s| (s[0] ^ s[1]) & 1 != 0).unwrap();
    assert!(first_b < first_a);
    let mut restored_from_first = ClockedDetentDecoder::new(true, true);
    assert!(feed(&mut restored_from_first, 2, 1).is_empty());
    assert_eq!(
        feed(&mut restored_from_first, 0, SETTLE),
        [Detent::CounterClockwise]
    );

    // Last-edge latch (ee901d1): same CW click followed by an A bounce.
    // Latest B is before latest A, again falsely suggesting CCW.
    let raw = [3_u8, 1, 0, 2, 0];
    let last_a = raw
        .windows(2)
        .rposition(|s| (s[0] ^ s[1]) & 2 != 0)
        .unwrap();
    let last_b = raw
        .windows(2)
        .rposition(|s| (s[0] ^ s[1]) & 1 != 0)
        .unwrap();
    assert!(last_b < last_a);
    let mut complete = ClockedDetentDecoder::new(true, true);
    for state in &raw[1..raw.len() - 1] {
        assert!(feed(&mut complete, *state, 1).is_empty());
    }
    assert_eq!(feed(&mut complete, 0, SETTLE), [Detent::Clockwise]);
}
