//! Stable text protocol shared by firmware export and host replay tests.
use crate::rotary_decoder::{DecisionSource, DecoderState, Detent};
use crate::rotary_trace::Sample;
use core::fmt::{self, Write};

pub const HEADER: &str = "n,ticks,a,b,b_stable,b_candidate,b_run,b_ab,b_idle,b_tracking,b_e,b_i,b_h,a_stable,a_candidate,a_run,a_ab,a_idle,a_tracking,a_e,a_i,a_h,delta,preclear_e,preclear_i,clears,source,output";
pub fn direction(value: Option<Detent>) -> i8 {
    match value {
        Some(Detent::Clockwise) => 1,
        Some(Detent::CounterClockwise) => -1,
        None => 0,
    }
}
pub fn source(value: Option<DecisionSource>) -> &'static str {
    match value {
        Some(DecisionSource::Edge) => "E",
        Some(DecisionSource::Interval) => "I",
        Some(DecisionSource::History) => "H",
        Some(DecisionSource::Unknown) => "U",
        None => "-",
    }
}
fn state(w: &mut impl Write, s: DecoderState) -> fmt::Result {
    write!(
        w,
        ",{},{},{},{},{},{},{},{},{}",
        u8::from(s.stable_a),
        u8::from(s.candidate_a),
        s.run_a,
        s.previous_state,
        s.unchanged_samples,
        u8::from(s.tracking_a_edge),
        s.edge_movement,
        s.interval_movement,
        direction(s.last_direction)
    )
}
pub fn row(w: &mut impl Write, s: Sample) -> fmt::Result {
    write!(
        w,
        "{},{},{},{}",
        s.number,
        s.ticks,
        u8::from(s.a),
        u8::from(s.b)
    )?;
    state(w, s.before)?;
    state(w, s.after)?;
    writeln!(
        w,
        ",{},{},{},{},{},{}",
        s.evidence.delta,
        s.evidence.edge_before_clear,
        s.evidence.interval_before_clear,
        s.evidence.clears,
        source(s.decision.map(|d| d.source)),
        direction(s.decision.and_then(|d| d.output))
    )
}
pub fn checksum(mut hash: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        hash = (hash ^ u32::from(*byte)).wrapping_mul(16777619);
    }
    hash
}
