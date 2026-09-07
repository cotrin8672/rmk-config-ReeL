//! Bounded state exploration, not a recording of the physical encoder.
//! Expected directions come from knob movement, independently of the decoder.
//! Hidden intermediate states model sampling loss; they are NOT evidence that
//! the device loses edges at exactly these positions. No private state injection.

use rotary_decoder_tests::rotary_decoder::{ClockedDetentDecoder, DEBOUNCE_SAMPLES, Detent};

const STABLE: usize = DEBOUNCE_SAMPLES as usize;
const CW_STATES: [u8; 4] = [3, 1, 0, 2];

#[derive(Clone, Copy, Debug)]
enum Capture {
    Visible,
    LatchedEvenBoundary,
    LatchedOddBoundary,
    HiddenEverywhere,
}

#[derive(Clone, Copy, Debug)]
struct Case {
    phase: usize,
    history: u8,
    capture: Capture,
    edge_gap: usize,
    rest: usize,
    bounce: bool,
    chatter: bool,
}

fn repeat(samples: &mut Vec<u8>, state: u8, count: usize) {
    samples.extend(std::iter::repeat_n(state, count));
}

fn transition(samples: &mut Vec<u8>, from: u8, to: u8, dwell: usize, bounce: bool) {
    if bounce {
        // A fully observed contact bounce: forward, back, then forward.
        repeat(samples, to, 1);
        repeat(samples, from, 1);
    }
    repeat(samples, to, dwell);
}

fn is_latched(case: Case, position: i32, cw: bool) -> bool {
    let next_position = position + if cw { 1 } else { -1 };
    let parity = position.min(next_position).rem_euclid(2);
    match case.capture {
        Capture::LatchedEvenBoundary => parity == 0,
        Capture::LatchedOddBoundary => parity == 1,
        _ => false,
    }
}

fn click_samples(case: Case, phase: usize, position: i32, cw: bool) -> Vec<u8> {
    let mut samples = Vec::new();
    let start = CW_STATES[phase];
    let latched = is_latched(case, position, cw);
    if case.chatter {
        // B-only chatter, including a level held beyond debounce, must not
        // create clicks or change the answer for a later visible movement.
        repeat(&mut samples, start ^ 1, 1);
        repeat(&mut samples, start, 1);
        repeat(&mut samples, start ^ 1, STABLE + 1);
        repeat(&mut samples, start, STABLE + 1);
    }
    let middle = CW_STATES[(phase + if cw { 1 } else { 3 }) % 4];
    let end = CW_STATES[(phase + 2) % 4];
    if matches!(case.capture, Capture::HiddenEverywhere) || latched {
        // Counterexample only: both edges occurred before acquisition ran.
        transition(&mut samples, start, end, STABLE, case.bounce);
    } else {
        transition(&mut samples, start, middle, case.edge_gap, case.bounce);
        transition(&mut samples, middle, end, STABLE, case.bounce);
    }
    repeat(&mut samples, end, case.rest);
    samples
}

fn feed(decoder: &mut ClockedDetentDecoder, samples: &[u8]) -> Vec<Detent> {
    samples
        .iter()
        .filter_map(|state| decoder.update(state & 2 != 0, state & 1 != 0))
        .collect()
}

fn feed_with_edge_b(
    decoder: &mut ClockedDetentDecoder,
    samples: &[u8],
    start_a: bool,
    edge_b: Option<bool>,
) -> Vec<Detent> {
    let mut edge_b = edge_b;
    samples
        .iter()
        .filter_map(|state| {
            let a = state & 2 != 0;
            let captured = (a != start_a).then(|| edge_b.take()).flatten();
            decoder.update_with_edge_b(a, state & 1 != 0, captured)
        })
        .collect()
}

#[derive(Default, Debug)]
struct Results {
    cases: usize,
    clicks: usize,
    failed_cases: usize,
    wrong_direction_clicks: usize,
    missing_clicks: usize,
    extra_events: usize,
    first_failure: Option<String>,
}

fn run_case(case: Case, directions: &[bool], results: &mut Results) {
    results.cases += 1;
    let mut phase = case.phase;
    let initial = CW_STATES[phase];
    let mut decoder = ClockedDetentDecoder::new(initial & 2 != 0, initial & 1 != 0);
    let mut position = 0;
    let mut failed = false;
    for (click, &cw) in directions.iter().enumerate() {
        let expected = if cw {
            Detent::Clockwise
        } else {
            Detent::CounterClockwise
        };
        let samples = click_samples(case, phase, position, cw);
        let middle = CW_STATES[(phase + if cw { 1 } else { 3 }) % 4];
        let edge_b = is_latched(case, position, cw).then_some(middle & 1 != 0);
        let actual = feed_with_edge_b(&mut decoder, &samples, CW_STATES[phase] & 2 != 0, edge_b);
        results.clicks += 1;
        if actual != [expected] {
            failed = true;
            results.missing_clicks += usize::from(actual.is_empty());
            results.extra_events += actual.len().saturating_sub(1);
            results.wrong_direction_clicks += usize::from(actual.iter().any(|d| *d != expected));
            if results.first_failure.is_none() {
                results.first_failure = Some(format!(
                    "{case:?}, history={:06b}, click={click}, position={position}, expected={expected:?}, actual={actual:?}",
                    case.history
                ));
            }
        }
        phase = (phase + 2) % 4;
        position += if cw { 1 } else { -1 };
    }
    results.failed_cases += usize::from(failed);
}

fn explore(capture: Capture) -> Results {
    let mut results = Results::default();
    for phase in 0..4 {
        // Every six-click CW/CCW history, including initial unknown direction,
        // repeated reversals, continuous rotation, and both boundary parities.
        for history in 0..64 {
            let directions: Vec<_> = (0..6).map(|bit| history & (1 << bit) != 0).collect();
            for edge_gap in [1, STABLE - 1, STABLE, STABLE + 1] {
                for rest in [0, STABLE - 1, STABLE, STABLE + 1, 200] {
                    for bounce in [false, true] {
                        for chatter in [false, true] {
                            run_case(
                                Case {
                                    phase,
                                    history,
                                    capture,
                                    edge_gap,
                                    rest,
                                    bounce,
                                    chatter,
                                },
                                &directions,
                                &mut results,
                            );
                        }
                    }
                }
            }
        }
    }
    results
}

#[test]
fn all_visible_six_click_histories_preserve_count_and_order() {
    let results = explore(Capture::Visible);
    eprintln!("visible={results:#?}");
    assert_eq!(results.failed_cases, 0, "{results:#?}");
}

#[test]
fn alternating_boundary_edge_latches_preserve_count_and_order() {
    let even = explore(Capture::LatchedEvenBoundary);
    let odd = explore(Capture::LatchedOddBoundary);
    assert_eq!(
        even.failed_cases + odd.failed_cases,
        0,
        "even={even:#?}\nodd={odd:#?}"
    );
}

#[test]
fn moving_one_click_changes_boundary_but_must_not_lock_direction() {
    let mut results = Results::default();
    for phase in 0..4 {
        for seed_cw in [false, true] {
            for capture in [Capture::LatchedEvenBoundary, Capture::LatchedOddBoundary] {
                let case = Case {
                    phase,
                    history: u8::from(seed_cw),
                    capture,
                    edge_gap: 1,
                    rest: 200,
                    bounce: false,
                    chatter: false,
                };
                // Establish history, rock back/forth, move one extra click,
                // then rock back/forth over the neighbouring boundary.
                let s = seed_cw;
                run_case(case, &[s, !s, s, !s, s, s, !s, s, !s, s], &mut results);
            }
        }
    }
    assert_eq!(results.failed_cases, 0, "{results:#?}");
}

#[test]
fn missing_phase_order_cannot_be_recovered_from_direction_history() {
    for phase in 0..4 {
        let case = Case {
            phase,
            history: 0,
            capture: Capture::HiddenEverywhere,
            edge_gap: 1,
            rest: 200,
            bounce: false,
            chatter: false,
        };
        let cw = click_samples(case, phase, 0, true);
        let ccw = click_samples(case, phase, 0, false);
        // Opposite physical movements can yield IDENTICAL decoder input.
        // No state-machine-only change can guarantee both correct answers,
        // even if the entire preceding physical/sampled history is identical.
        assert_eq!(cw, ccw);
        assert_ne!(Detent::Clockwise, Detent::CounterClockwise);
    }
}

#[test]
fn a_only_clicks_do_not_wait_for_b_or_reuse_history() {
    // Preserve the established A-clock contract: a visible signed A edge
    // must emit even when no net B edge is captured at the arrival snap.
    for (phase, expected) in [
        (0, Detent::Clockwise),
        (1, Detent::CounterClockwise),
        (2, Detent::Clockwise),
        (3, Detent::CounterClockwise),
    ] {
        for prior in [None, Some(false), Some(true)] {
            let state = CW_STATES[phase];
            let mut decoder = ClockedDetentDecoder::new(state & 2 != 0, state & 1 != 0);
            let case = Case {
                phase,
                history: 0,
                capture: Capture::Visible,
                edge_gap: STABLE + 1,
                rest: 200,
                bounce: false,
                chatter: false,
            };
            if let Some(cw) = prior {
                let direction = if cw {
                    Detent::Clockwise
                } else {
                    Detent::CounterClockwise
                };
                // Two clicks return to the same raw phase, with known history.
                for offset in [0, 2] {
                    let samples = click_samples(case, (phase + offset) % 4, 0, cw);
                    assert_eq!(feed(&mut decoder, &samples), [direction]);
                }
            }
            assert_eq!(
                feed(&mut decoder, &vec![state ^ 2; STABLE + 200]),
                [expected],
                "phase={phase}, prior={prior:?}"
            );
        }
    }
}

#[test]
fn aborted_a_edges_and_rest_chatter_do_not_create_clicks() {
    for state in 0..4 {
        for duration in [1, STABLE - 1] {
            let mut decoder = ClockedDetentDecoder::new(state & 2 != 0, state & 1 != 0);
            let mut samples = Vec::new();
            for _ in 0..4 {
                repeat(&mut samples, state ^ 2, duration);
                repeat(&mut samples, state, STABLE + 1);
                repeat(&mut samples, state ^ 1, STABLE + 1);
                repeat(&mut samples, state, STABLE + 1);
            }
            assert!(
                feed(&mut decoder, &samples).is_empty(),
                "state={state}, duration={duration}"
            );
        }
    }
}
