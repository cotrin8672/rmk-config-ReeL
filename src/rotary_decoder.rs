//! Decode cumulative signed A-edge counters captured after 1 ms of A stability.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detent {
    Clockwise,
    CounterClockwise,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub positive: u32,
    pub negative: u32,
    pub sequence: u32,
}

pub struct ClockedDetentDecoder {
    previous: Frame,
    faulted: bool,
}
impl ClockedDetentDecoder {
    pub const fn new() -> Self {
        Self {
            previous: Frame {
                positive: 0,
                negative: 0,
                sequence: 0,
            },
            faulted: false,
        }
    }
    /// Missing frames are unrecoverable: never emit a combined interval.
    /// Wrap is valid provided fewer than 2^32 edges occur per frame.
    pub fn update(&mut self, frame: Frame) -> Result<Option<Detent>, ()> {
        if self.faulted || frame.sequence.wrapping_sub(self.previous.sequence) != 1 {
            self.faulted = true;
            return Err(());
        }
        let positive = frame.positive.wrapping_sub(self.previous.positive);
        let negative = frame.negative.wrapping_sub(self.previous.negative);
        self.previous = frame;
        if positive.wrapping_add(negative) & 1 == 0 {
            return Ok(None);
        }
        Ok(Some(if positive > negative {
            Detent::Clockwise
        } else {
            Detent::CounterClockwise
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const GRAY: [i32; 16] = [0, -1, 1, 0, 1, 0, 0, -1, -1, 0, 0, 1, 0, 1, -1, 0];
    #[test]
    fn all_single_bit_paths_through_length_fourteen() {
        let mut paths = 0;
        for initial in 0u8..4 {
            for length in 0..=14 {
                for bits in 0..(1u32 << length) {
                    let mut state = initial;
                    let mut gray = 0;
                    let mut frame = Frame {
                        sequence: 1,
                        ..Frame::default()
                    };
                    for edge in 0..length {
                        let next = state ^ if bits & (1 << edge) == 0 { 2 } else { 1 };
                        gray += GRAY[usize::from(state * 4 + next)];
                        if (state ^ next) & 2 != 0 {
                            if (state >> 1) == (state & 1) {
                                frame.positive += 1;
                            } else {
                                frame.negative += 1;
                            }
                        }
                        state = next;
                    }
                    let result = ClockedDetentDecoder::new().update(frame).unwrap();
                    if (state ^ initial) & 2 == 0 {
                        assert_eq!(result, None);
                    } else {
                        assert_ne!(frame.positive, frame.negative);
                        assert_eq!(
                            result,
                            Some(if gray > 0 {
                                Detent::Clockwise
                            } else {
                                Detent::CounterClockwise
                            })
                        );
                    }
                    paths += 1;
                }
            }
        }
        assert_eq!(paths, 131_068);
    }
    #[test]
    fn cancelled_first_a_and_immediate_reversal() {
        let mut decoder = ClockedDetentDecoder::new();
        // 00 -> 10 -> 00 -> 01 -> 11: +1 -1 -1.
        assert_eq!(
            decoder.update(Frame {
                positive: 1,
                negative: 2,
                sequence: 1
            }),
            Ok(Some(Detent::CounterClockwise))
        );
        assert_eq!(
            decoder.update(Frame {
                positive: 2,
                negative: 2,
                sequence: 2
            }),
            Ok(Some(Detent::Clockwise))
        );
        // Even intervals are consumed, including nonzero signed sums.
        assert_eq!(
            decoder.update(Frame {
                positive: 2,
                negative: 4,
                sequence: 3
            }),
            Ok(None)
        );
        assert_eq!(
            decoder.update(Frame {
                positive: 3,
                negative: 4,
                sequence: 4
            }),
            Ok(Some(Detent::Clockwise))
        );
    }
    #[test]
    fn counters_and_frame_number_wrap() {
        let mut decoder = ClockedDetentDecoder {
            previous: Frame {
                positive: u32::MAX,
                negative: u32::MAX,
                sequence: u32::MAX,
            },
            faulted: false,
        };
        assert_eq!(
            decoder.update(Frame {
                positive: 1,
                negative: 0,
                sequence: 0
            }),
            Ok(Some(Detent::Clockwise))
        );
    }
    #[test]
    fn lost_or_duplicate_frame_latches_fault() {
        for sequence in [0, 2, 100] {
            let mut decoder = ClockedDetentDecoder::new();
            assert_eq!(
                decoder.update(Frame {
                    positive: 1,
                    negative: 0,
                    sequence
                }),
                Err(())
            );
            assert_eq!(
                decoder.update(Frame {
                    positive: 1,
                    negative: 0,
                    sequence: 1
                }),
                Err(())
            );
        }
    }
}
