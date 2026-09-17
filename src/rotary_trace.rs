//! Frozen capture of actual update() calls, including identical samples.
use crate::rotary_decoder::{Decision, DecisionSource, DecoderState, UpdateEvidence};

pub const DECISION_CAPACITY: usize = 32;
pub const SAMPLE_CAPACITY: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureState {
    Disarmed,
    Armed,
    Frozen,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Ambiguous,
    NextConfirmation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub number: u64,
    pub ticks: u64,
    pub a: bool,
    pub b: bool,
    pub before: DecoderState,
    pub after: DecoderState,
    pub evidence: UpdateEvidence,
    pub decision: Option<Decision>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub number: u64,
    pub sample: Sample,
    pub decision: Decision,
}

pub struct Trace {
    pub samples: [Option<Sample>; SAMPLE_CAPACITY],
    pub records: [Option<Record>; DECISION_CAPACITY],
    pub sample_count: u64,
    pub decision_count: u64,
    pub state: CaptureState,
    pub generation: u32,
    trigger: Trigger,
}
impl Trace {
    pub const fn new() -> Self {
        Self {
            samples: [None; SAMPLE_CAPACITY],
            records: [None; DECISION_CAPACITY],
            sample_count: 0,
            decision_count: 0,
            state: CaptureState::Disarmed,
            generation: 0,
            trigger: Trigger::Ambiguous,
        }
    }
    /// Only resets diagnostic indices. Never resets the live decoder.
    pub fn arm(&mut self, trigger: Trigger) {
        self.sample_count = 0;
        self.decision_count = 0;
        self.generation = self.generation.wrapping_add(1);
        self.trigger = trigger;
        self.state = CaptureState::Armed;
    }
    pub fn push(&mut self, mut sample: Sample) {
        if self.state != CaptureState::Armed {
            return;
        }
        let slot = (self.sample_count % SAMPLE_CAPACITY as u64) as usize;
        self.sample_count += 1;
        sample.number = self.sample_count;
        self.samples[slot] = Some(sample);
        if let Some(decision) = sample.decision {
            let slot = (self.decision_count % DECISION_CAPACITY as u64) as usize;
            self.decision_count += 1;
            self.records[slot] = Some(Record {
                number: self.decision_count,
                sample,
                decision,
            });
            if self.trigger == Trigger::NextConfirmation
                || matches!(
                    decision.source,
                    DecisionSource::History | DecisionSource::Unknown
                )
            {
                self.state = CaptureState::Frozen;
            }
        }
    }
    pub fn len(&self) -> usize {
        self.sample_count.min(SAMPLE_CAPACITY as u64) as usize
    }
    pub fn dropped(&self) -> u64 {
        self.sample_count.saturating_sub(SAMPLE_CAPACITY as u64)
    }
    /// Chronological order, anchored by a complete before-state even after wrap.
    pub fn sample(&self, index: usize) -> Option<Sample> {
        if index >= self.len() {
            return None;
        }
        self.samples[((self.dropped() + index as u64) % SAMPLE_CAPACITY as u64) as usize]
    }
    pub fn latest(&self, age: u64) -> Option<Record> {
        if self.decision_count <= age || age >= DECISION_CAPACITY as u64 {
            return None;
        }
        self.records[((self.decision_count - 1 - age) % DECISION_CAPACITY as u64) as usize]
    }
}
