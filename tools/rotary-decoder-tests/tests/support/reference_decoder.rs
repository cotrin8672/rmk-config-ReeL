// Frozen cb06d32 decoder for diagnostic non-interference checks.
// Decoder for the left rotary encoder (BM4.0A01, 9 pulse / 18 click).
//
// Device measurements established two independent facts:
//
// - A has one debounced level change per physical click, so it is the
//   reliable click clock.
// - The raw valid Gray-code transitions have a consistent sign for a
//   rotation direction. The first diagnostic build measured a net `-4`
//   over four clicks even though the old ±2 threshold emitted only twice.
//
// Reading B at a fixed time relative to A is not valid for this part.
// Contact hysteresis puts B just before A in one direction and just after A
// in the other; changing the delay merely swaps which direction is wrong.
//
// This decoder therefore keeps click and direction independent:
//
// - A debounces to exactly one event per click.
// - Every raw one-bit Gray transition contributes +1 or -1 to signed
//   movement. Bounce contributes opposite pairs and cancels.
// - When A confirms a click, the sign accumulated from A's first raw
//   departure through its debounced new level gives its direction. That
//   window belongs only to the current click, so post-click B chatter from
//   the previous direction cannot bias a reversal.
// - If A and B changed together and that local window has no signed
//   evidence, movement across the whole click interval is the fallback.
// - Once the contacts have been unchanged for [`EVIDENCE_IDLE_SAMPLES`],
//   movement left over from the previous click settling is discarded. It
//   must not bias the first click after a direction reversal.
//
// No instantaneous B read or fixed B-sampling delay is involved.
//
// A sampled two-bit transition contains no direction information and adds
// zero. If an entire click is such a transition (not observed in the
// four-click signed-position measurement), the previous direction is the
// only physically defensible fallback.

/// One physical detent click.
///
/// `Clockwise` corresponds to the datasheet's CW rotation, i.e. the raw
/// (A, B) state sequence 11 -> 01 -> 00 -> 10 -> 11. This matches the
/// direction previously reported as `Direction::Clockwise` by the original
/// GPIO decoder, so existing Vial encoder mappings keep their meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detent {
    Clockwise,
    CounterClockwise,
}

/// Consecutive identical A samples required before a click is accepted.
/// At the ~61 us sample period this is about 1 ms.
pub const DEBOUNCE_SAMPLES: u8 = 16;

/// Unchanged samples before direction evidence is considered old. This is
/// about 1 ms at the ~61 us sample period. A confirms at the same duration,
/// so a real A edge is consumed before its evidence can expire.
const EVIDENCE_IDLE_SAMPLES: u8 = DEBOUNCE_SAMPLES;

/// +1 follows the datasheet CW sequence
/// `11 -> 01 -> 00 -> 10 -> 11`; -1 follows the reverse sequence.
const TRANSITION_DELTA: [i8; 16] = [
    0, -1, 1, 0, // from 00
    1, 0, 0, -1, // from 01
    -1, 0, 0, 1, // from 10
    0, 1, -1, 0, // from 11
];

const fn encode(a_high: bool, b_high: bool) -> u8 {
    ((a_high as u8) << 1) | (b_high as u8)
}

pub struct ClockedDetentDecoder {
    stable_a: bool,
    candidate_a: bool,
    run_a: u8,
    previous_state: u8,
    unchanged_samples: u8,
    tracking_a_edge: bool,
    edge_movement: i32,
    interval_movement: i32,
    last_direction: Option<Detent>,
}

impl ClockedDetentDecoder {
    pub const fn new(a_high: bool, b_high: bool) -> Self {
        Self {
            stable_a: a_high,
            candidate_a: a_high,
            run_a: DEBOUNCE_SAMPLES,
            previous_state: encode(a_high, b_high),
            unchanged_samples: EVIDENCE_IDLE_SAMPLES,
            tracking_a_edge: false,
            edge_movement: 0,
            interval_movement: 0,
            last_direction: None,
        }
    }

    /// True once all pending edge and direction evidence has settled.
    pub fn is_idle(&self) -> bool {
        self.unchanged_samples >= EVIDENCE_IDLE_SAMPLES && !self.tracking_a_edge
    }

    /// Feed one raw sample. A debounced A transition emits one detent; its
    /// direction comes from signed Gray movement accumulated across the
    /// whole click interval rather than B at any selected instant.
    pub fn update(&mut self, a_high: bool, b_high: bool) -> Option<Detent> {
        let state = encode(a_high, b_high);
        if state == self.previous_state {
            self.unchanged_samples = self.unchanged_samples.saturating_add(1);
        } else {
            self.unchanged_samples = 0;
        }
        let delta = TRANSITION_DELTA[usize::from((self.previous_state << 2) | state)];
        self.previous_state = state;
        self.interval_movement += i32::from(delta);

        // Isolate evidence belonging to this A transition. Keep the window
        // open across bounce returns; close it only when either level has
        // remained stable for the full debounce duration.
        if !self.tracking_a_edge && a_high != self.stable_a {
            self.tracking_a_edge = true;
            self.edge_movement = i32::from(delta);
        } else if self.tracking_a_edge {
            self.edge_movement += i32::from(delta);
        }

        if a_high == self.candidate_a {
            self.run_a = self.run_a.saturating_add(1);
        } else {
            self.candidate_a = a_high;
            self.run_a = 1;
        }

        if self.run_a >= DEBOUNCE_SAMPLES && self.candidate_a != self.stable_a {
            self.stable_a = self.candidate_a;
            let movement = if self.edge_movement != 0 {
                self.edge_movement
            } else {
                self.interval_movement
            };
            let direction = if movement > 0 {
                Some(Detent::Clockwise)
            } else if movement < 0 {
                Some(Detent::CounterClockwise)
            } else {
                self.last_direction
            };
            self.tracking_a_edge = false;
            self.edge_movement = 0;
            self.interval_movement = 0;
            if let Some(direction) = direction {
                self.last_direction = Some(direction);
            }
            return direction;
        }

        // A glitch returned to its accepted level instead of becoming a
        // click. Close both evidence windows: otherwise a missed bounce
        // transition can leave an uncancelled interval sum that is reused
        // by the next click before the all-input-idle timeout expires.
        if self.tracking_a_edge
            && self.run_a >= DEBOUNCE_SAMPLES
            && self.candidate_a == self.stable_a
        {
            self.tracking_a_edge = false;
            self.edge_movement = 0;
            self.interval_movement = 0;
        }

        // A completed click can leave a trailing B transition in the next
        // interval. Once the contacts settle, that evidence belongs to the
        // old click and must not survive into a later reversal.
        if self.unchanged_samples >= EVIDENCE_IDLE_SAMPLES {
            self.interval_movement = 0;
        }
        None
    }
}
