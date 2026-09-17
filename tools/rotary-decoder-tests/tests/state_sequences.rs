//! Every six-click direction history, all initial states, bounce and B chatter.
//! The model assumes separately acquired edges; it cannot prove hardware capture.
mod support;
use rotary_decoder_tests::rotary_decoder::Detent;
use support::Capture;

#[test]
fn all_six_click_histories_and_eighteen_click_revolutions() {
    for initial in 0..4 {
        for history in 0u32..64 {
            for bounce in [false, true] {
                for chatter in [false, true] {
                    let mut capture = Capture::new(initial);
                    let mut time = 0;
                    let mut expected = Vec::new();
                    for click in 0..6 {
                        let cw = history & (1 << click) != 0;
                        if chatter {
                            for _ in 0..4 {
                                time += 1100;
                                capture.edge(time, capture.state ^ 1);
                            }
                        }
                        for _ in 0..2 {
                            let equal = capture.state >> 1 == capture.state & 1;
                            let next = capture.state ^ if equal == cw { 2 } else { 1 };
                            if bounce {
                                let old = capture.state;
                                time += 10;
                                capture.edge(time, next);
                                time += 10;
                                capture.edge(time, old);
                            }
                            time += 10;
                            capture.edge(time, next);
                        }
                        time += 1000;
                        capture.advance(time);
                        expected.push(if cw {
                            Detent::Clockwise
                        } else {
                            Detent::CounterClockwise
                        });
                    }
                    assert_eq!(
                        capture.decode(),
                        expected,
                        "initial={initial}, history={history}, bounce={bounce}, chatter={chatter}"
                    );
                }
            }
        }
        for cw in [false, true] {
            let mut capture = Capture::new(initial);
            for edge in 0..36 {
                let equal = capture.state >> 1 == capture.state & 1;
                capture.edge(edge * 5000, capture.state ^ if equal == cw { 2 } else { 1 });
            }
            capture.advance(200_000);
            assert_eq!(
                capture.decode(),
                vec![
                    if cw {
                        Detent::Clockwise
                    } else {
                        Detent::CounterClockwise
                    };
                    18
                ]
            );
        }
    }
}
