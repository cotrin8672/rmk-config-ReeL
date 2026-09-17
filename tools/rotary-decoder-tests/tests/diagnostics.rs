//! Diagnostic instrumentation must preserve every baseline output and idle state.
#[path = "support/reference_decoder.rs"]
mod reference;
use rotary_decoder_tests::rotary_decoder::{
    ClockedDetentDecoder, DEBOUNCE_SAMPLES, DecisionSource, Detent,
};

fn code(value: Option<Detent>) -> i8 {
    match value {
        Some(Detent::Clockwise) => 1,
        Some(Detent::CounterClockwise) => -1,
        None => 0,
    }
}
fn baseline_code(value: Option<reference::Detent>) -> i8 {
    match value {
        Some(reference::Detent::Clockwise) => 1,
        Some(reference::Detent::CounterClockwise) => -1,
        None => 0,
    }
}
#[test]
fn instrumentation_preserves_cb06d32_decisions_and_idle_state() {
    for initial in 0u8..4 {
        let mut actual = ClockedDetentDecoder::new(initial & 2 != 0, initial & 1 != 0);
        let mut baseline = reference::ClockedDetentDecoder::new(initial & 2 != 0, initial & 1 != 0);
        let mut random = 1234567u32;
        let mut sources = [0usize; 4];
        for _ in 0..20_000 {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            let state = (random >> 16) as u8 & 3;
            let hold = 1 + (random >> 24) as usize % 35;
            for _ in 0..hold {
                let output = actual.update(state & 2 != 0, state & 1 != 0);
                assert_eq!(
                    code(output),
                    baseline_code(baseline.update(state & 2 != 0, state & 1 != 0))
                );
                assert_eq!(actual.is_idle(), baseline.is_idle());
                if let Some(d) = actual.take_decision() {
                    assert_ne!(d.old_a, d.new_a);
                    assert_eq!(d.observed_a, state & 2 != 0);
                    assert_eq!(d.observed_b, state & 1 != 0);
                    assert_eq!(d.output, output);
                    match d.source {
                        DecisionSource::Edge => {
                            assert_ne!(d.edge_movement, 0);
                            assert_eq!(code(output), d.edge_movement.signum() as i8);
                            sources[0] += 1;
                        }
                        DecisionSource::Interval => {
                            assert_eq!(d.edge_movement, 0);
                            assert_ne!(d.interval_movement, 0);
                            assert_eq!(code(output), d.interval_movement.signum() as i8);
                            sources[1] += 1;
                        }
                        DecisionSource::History => {
                            assert_eq!((d.edge_movement, d.interval_movement), (0, 0));
                            assert_eq!(output, d.previous_direction);
                            assert!(output.is_some());
                            sources[2] += 1;
                        }
                        DecisionSource::Unknown => {
                            assert_eq!((d.edge_movement, d.interval_movement), (0, 0));
                            assert_eq!(output, None);
                            sources[3] += 1;
                        }
                    }
                    assert_eq!(actual.take_decision(), None);
                } else {
                    assert_eq!(output, None);
                }
            }
        }
        assert!(
            sources[..3].iter().all(|n| *n > 0),
            "E/I/H branches must be exercised: {sources:?}"
        );
    }
}

#[test]
fn unknown_a_confirmation_is_recorded_before_evidence_is_cleared() {
    let mut decoder = ClockedDetentDecoder::new(false, false);
    for _ in 1..DEBOUNCE_SAMPLES {
        assert_eq!(decoder.update(true, true), None);
        assert_eq!(decoder.take_decision(), None);
    }
    assert_eq!(decoder.update(true, true), None);
    let d = decoder.take_decision().unwrap();
    assert_eq!(d.source, DecisionSource::Unknown);
    assert_eq!(d.previous_direction, None);
    assert_eq!((d.edge_movement, d.interval_movement), (0, 0));
    assert!(!d.old_a && d.new_a);
}

#[test]
fn bounded_trace_keeps_repeated_samples_and_none_output_confirmations() {
    use rotary_decoder_tests::rotary_trace::{DECISION_CAPACITY, SAMPLE_CAPACITY, Trace};
    let mut decoder = ClockedDetentDecoder::new(false, false);
    let mut trace = Trace::new();
    for n in 0..80 {
        let state = n % 2 == 0;
        for repeat in 0..DEBOUNCE_SAMPLES {
            decoder.update(state, state);
            trace.push(
                n * 100 + u64::from(repeat),
                state,
                state,
                decoder.take_decision(),
            );
        }
    }
    assert_eq!(trace.decision_count, 80);
    assert_eq!(trace.sample_count, 80 * u64::from(DEBOUNCE_SAMPLES));
    assert_eq!(trace.latest(0).unwrap().number, 80);
    assert_eq!(trace.latest(31).unwrap().number, 49);
    assert_eq!(trace.latest(32), None);
    assert_eq!(trace.records.iter().flatten().count(), DECISION_CAPACITY);
    assert_eq!(trace.samples.iter().flatten().count(), SAMPLE_CAPACITY);
    assert!(
        trace
            .records
            .iter()
            .flatten()
            .all(|r| r.decision.output.is_none())
    );
    assert_eq!(
        trace.samples.iter().flatten().map(|s| s.number).min(),
        Some(trace.sample_count - SAMPLE_CAPACITY as u64 + 1)
    );
}
