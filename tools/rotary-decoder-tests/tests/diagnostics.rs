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

fn feed(
    trace: &mut rotary_decoder_tests::rotary_trace::Trace,
    decoder: &mut ClockedDetentDecoder,
    ticks: u64,
    a: bool,
    b: bool,
) {
    let before = decoder.state();
    decoder.update(a, b);
    trace.push(rotary_decoder_tests::rotary_trace::Sample {
        number: 0,
        ticks,
        a,
        b,
        before,
        after: decoder.state(),
        evidence: decoder.evidence(),
        decision: decoder.take_decision(),
    });
}
#[test]
fn capture_freezes_unknown_and_survives_later_input() {
    use rotary_decoder_tests::rotary_trace::{CaptureState, Trace, Trigger};
    let mut decoder = ClockedDetentDecoder::new(false, false);
    let mut trace = Trace::new();
    feed(&mut trace, &mut decoder, 0, false, false);
    assert_eq!(trace.len(), 0);
    trace.arm(Trigger::Ambiguous);
    for n in 0..16 {
        feed(&mut trace, &mut decoder, n, true, true);
    }
    assert_eq!(trace.state, CaptureState::Frozen);
    let frozen = trace.sample(15).unwrap();
    assert_eq!(frozen.decision.unwrap().source, DecisionSource::Unknown);
    for n in 16..2000 {
        feed(&mut trace, &mut decoder, n, false, false);
    }
    assert_eq!(trace.sample_count, 16);
    assert_eq!(trace.sample(15), Some(frozen));
    let before = decoder.state();
    trace.arm(Trigger::NextConfirmation);
    assert_eq!(decoder.state(), before);
    assert_eq!(trace.len(), 0);
    assert_eq!(trace.sample(0), None);
    assert_eq!(trace.latest(0), None);
}
#[test]
fn wrapped_prefix_has_complete_replay_state_and_clear_reasons() {
    use rotary_decoder_tests::rotary_trace::{CaptureState, SAMPLE_CAPACITY, Trace, Trigger};
    let mut decoder = ClockedDetentDecoder::new(false, false);
    let mut trace = Trace::new();
    trace.arm(Trigger::Ambiguous);
    for n in 0..600 {
        feed(&mut trace, &mut decoder, n, false, false);
    }
    // Valid CW followed by an ambiguous two-bit reversal: H must freeze.
    for n in 600..616 {
        feed(&mut trace, &mut decoder, n, true, false);
    }
    for n in 616..636 {
        feed(&mut trace, &mut decoder, n, true, true);
    }
    for n in 636..652 {
        feed(&mut trace, &mut decoder, n, false, false);
    }
    assert_eq!(trace.state, CaptureState::Frozen);
    assert_eq!(trace.len(), SAMPLE_CAPACITY);
    assert_eq!(trace.dropped(), 652 - SAMPLE_CAPACITY as u64);
    assert_eq!(
        trace
            .sample(trace.len() - 1)
            .unwrap()
            .decision
            .unwrap()
            .source,
        DecisionSource::History
    );
    let mut replay = ClockedDetentDecoder::from_state(trace.sample(0).unwrap().before);
    for index in 0..trace.len() {
        let sample = trace.sample(index).unwrap();
        assert_eq!(sample.number, trace.dropped() + index as u64 + 1);
        assert_eq!(sample.before, replay.state());
        replay.update(sample.a, sample.b);
        assert_eq!(sample.after, replay.state());
        assert_eq!(sample.evidence, replay.evidence());
        assert_eq!(sample.decision, replay.take_decision());
    }
    assert_eq!(trace.sample(trace.len()), None);
}
#[test]
fn logs_show_accumulation_and_cancel_clear_before_reset() {
    let mut decoder = ClockedDetentDecoder::new(false, false);
    decoder.update(true, false);
    assert_eq!(decoder.evidence().delta, 1);
    assert_eq!(decoder.evidence().edge_before_clear, 1);
    for _ in 0..16 {
        decoder.update(false, false);
    }
    assert_eq!(decoder.evidence().clears & 2, 2);
    assert_eq!(decoder.evidence().edge_before_clear, 0);
    assert_eq!(
        (
            decoder.state().edge_movement,
            decoder.state().interval_movement
        ),
        (0, 0)
    );
}
