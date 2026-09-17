//! Bounded diagnostic storage. Samples are update() calls, not electrical edges.
use crate::rotary_decoder::Decision;

pub const DECISION_CAPACITY: usize = 32;
pub const SAMPLE_CAPACITY: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub number: u64,
    pub ticks: u64,
    pub a: bool,
    pub b: bool,
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
}
impl Trace {
    pub const fn new() -> Self {
        Self {
            samples: [None; SAMPLE_CAPACITY],
            records: [None; DECISION_CAPACITY],
            sample_count: 0,
            decision_count: 0,
        }
    }
    pub fn push(&mut self, ticks: u64, a: bool, b: bool, decision: Option<Decision>) {
        let slot = (self.sample_count % SAMPLE_CAPACITY as u64) as usize;
        self.sample_count += 1;
        let sample = Sample {
            number: self.sample_count,
            ticks,
            a,
            b,
        };
        // Samples are retrieved through a debugger, not a firmware reader.
        // Preserve these writes under release LTO. The slot is exclusively borrowed.
        unsafe { core::ptr::write_volatile(&mut self.samples[slot], Some(sample)) };
        if let Some(decision) = decision {
            let slot = (self.decision_count % DECISION_CAPACITY as u64) as usize;
            self.decision_count += 1;
            self.records[slot] = Some(Record {
                number: self.decision_count,
                sample,
                decision,
            });
        }
    }
    pub fn latest(&self, age: u64) -> Option<Record> {
        if self.decision_count <= age || age >= DECISION_CAPACITY as u64 {
            return None;
        }
        self.records[((self.decision_count - 1 - age) % DECISION_CAPACITY as u64) as usize]
    }
}
