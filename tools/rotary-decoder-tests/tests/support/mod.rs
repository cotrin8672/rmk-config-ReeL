//! Ideal sequential event model, not a GPIOTE/PPI timing simulation.
use rotary_decoder_tests::rotary_decoder::{ClockedDetentDecoder, Detent, Frame};

pub struct Capture {
    pub state: u8,
    equal: bool,
    counters: Frame,
    deadline: Option<u64>,
    pub frames: Vec<Frame>,
}
impl Capture {
    pub fn new(state: u8) -> Self {
        Self {
            state,
            equal: state >> 1 == state & 1,
            counters: Frame::default(),
            deadline: None,
            frames: Vec::new(),
        }
    }
    pub fn advance(&mut self, time: u64) {
        if self.deadline.is_some_and(|deadline| time >= deadline) {
            self.counters.sequence = self.counters.sequence.wrapping_add(1);
            self.frames.push(self.counters);
            self.deadline = None;
        }
    }
    pub fn edge(&mut self, time: u64, next: u8) {
        self.advance(time);
        let changed = self.state ^ next;
        assert!(
            changed == 1 || changed == 2,
            "model requires separately acquired A/B edges"
        );
        if changed == 2 {
            if self.equal {
                self.counters.positive = self.counters.positive.wrapping_add(1);
            } else {
                self.counters.negative = self.counters.negative.wrapping_add(1);
            }
            self.deadline = Some(time + 1000);
        }
        self.equal = !self.equal;
        self.state = next;
    }
    pub fn decode(&self) -> Vec<Detent> {
        let mut decoder = ClockedDetentDecoder::new();
        self.frames
            .iter()
            .filter_map(|frame| decoder.update(*frame).unwrap())
            .collect()
    }
}
