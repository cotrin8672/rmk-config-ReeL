//! Decoder for the left rotary encoder (BM4.0A01, 9 pulse / 18 click).
//!
//! Device measurements established two independent facts:
//!
//! - A has one debounced level change per physical click, so it is the
//!   reliable click clock.
//! - The raw valid Gray-code transitions have a consistent sign for a
//!   rotation direction. The first diagnostic build measured a net `-4`
//!   over four clicks even though the old ±2 threshold emitted only twice.
//!
//! Reading B at a fixed time relative to A is not valid for this part.
//! Contact hysteresis puts B just before A in one direction and just after A
//! in the other; changing the delay merely swaps which direction is wrong.
//!
//! This decoder therefore keeps click and direction independent:
//!
//! - A debounces to exactly one event per click.
//! - Every raw one-bit Gray transition contributes +1 or -1 to signed
//!   movement. Bounce contributes opposite pairs and cancels.
//! - When A confirms a click, the sign accumulated from A's first raw
//!   departure through its debounced new level gives its direction. That
//!   window belongs only to the current click, so post-click B chatter from
//!   the previous direction cannot bias a reversal.
//! - If A and B changed together and that local window has no signed
//!   evidence, movement across the whole click interval is the fallback.
//! - Once the contacts have been unchanged for [`EVIDENCE_IDLE_SAMPLES`],
//!   movement left over from the previous click settling is discarded. It
//!   must not bias the first click after a direction reversal.
//!
//! No instantaneous B read or fixed B-sampling delay is involved.
//!
//! A sampled two-bit transition contains no direction information and adds
//! zero. Such a click remains pending until a later valid Gray transition
//! supplies its direction; the decoder never substitutes the old direction.

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

/// One or more consecutive clicks whose direction was resolved together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetentBatch {
    pub detent: Detent,
    pub count: u8,
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
    pending_clicks: u8,
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
            pending_clicks: 0,
        }
    }

    /// True once all pending edge and direction evidence has settled.
    pub fn is_idle(&self) -> bool {
        self.unchanged_samples >= EVIDENCE_IDLE_SAMPLES && !self.tracking_a_edge
    }

    /// Feed one raw sample. A debounced A transition emits one detent; its
    /// direction comes from signed Gray movement accumulated across the
    /// whole click interval rather than B at any selected instant.
    pub fn update(&mut self, a_high: bool, b_high: bool) -> Option<DetentBatch> {
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
            self.pending_clicks = self.pending_clicks.saturating_add(1);
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
                None
            };
            self.tracking_a_edge = false;
            self.edge_movement = 0;
            if let Some(direction) = direction {
                let count = self.pending_clicks;
                self.pending_clicks = 0;
                self.interval_movement = 0;
                return Some(DetentBatch {
                    detent: direction,
                    count,
                });
            }
            return None;
        }

        // A glitch returned to its accepted level instead of becoming a
        // click. Discard only its local evidence.
        if self.tracking_a_edge
            && self.run_a >= DEBOUNCE_SAMPLES
            && self.candidate_a == self.stable_a
        {
            self.tracking_a_edge = false;
            self.edge_movement = 0;
        }

        // A directionless A click can be followed by the usable B transition
        // from that same physical click. Resolve only after the contacts have
        // settled so bounce pairs have first had a chance to cancel.
        if self.unchanged_samples >= EVIDENCE_IDLE_SAMPLES {
            if self.pending_clicks > 0 && self.interval_movement != 0 {
                let detent = if self.interval_movement > 0 {
                    Detent::Clockwise
                } else {
                    Detent::CounterClockwise
                };
                let count = self.pending_clicks;
                self.pending_clicks = 0;
                self.interval_movement = 0;
                return Some(DetentBatch { detent, count });
            }

            // With no unresolved click, settled movement is only residue from
            // the preceding click and must not bias a later reversal.
            self.interval_movement = 0;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{ClockedDetentDecoder, DEBOUNCE_SAMPLES, Detent, DetentBatch};

    const STABLE: usize = DEBOUNCE_SAMPLES as usize;

    struct Harness {
        decoder: ClockedDetentDecoder,
        clockwise: u32,
        counterclockwise: u32,
    }

    impl Harness {
        fn new(a: bool, b: bool) -> Self {
            Self {
                decoder: ClockedDetentDecoder::new(a, b),
                clockwise: 0,
                counterclockwise: 0,
            }
        }

        /// Feed the same raw sample `count` times.
        fn hold(&mut self, (a, b): (bool, bool), count: usize) {
            for _ in 0..count {
                match self.decoder.update(a, b) {
                    Some(DetentBatch {
                        detent: Detent::Clockwise,
                        count,
                    }) => self.clockwise += u32::from(count),
                    Some(DetentBatch {
                        detent: Detent::CounterClockwise,
                        count,
                    }) => self.counterclockwise += u32::from(count),
                    None => {}
                }
            }
        }

        /// Alternate between two raw samples, `period` samples each,
        /// `flips` times — bounce or chatter, always shorter than the
        /// debounce window per level.
        fn flap(&mut self, first: (bool, bool), second: (bool, bool), period: usize, flips: usize) {
            assert!(period < STABLE);
            for flip in 0..flips {
                let state = if flip % 2 == 0 { first } else { second };
                self.hold(state, period);
            }
        }

        fn assert_events(&mut self, clockwise: u32, counterclockwise: u32) {
            assert_eq!(
                (self.clockwise, self.counterclockwise),
                (clockwise, counterclockwise)
            );
            self.clockwise = 0;
            self.counterclockwise = 0;
        }
    }

    const S11: (bool, bool) = (true, true);
    const S01: (bool, bool) = (false, true);
    const S00: (bool, bool) = (false, false);
    const S10: (bool, bool) = (true, false);

    /// Measured CW anatomy (A-leading direction): bouncy A edge mid-travel,
    /// then the arrival snap where both phases jitter together and B comes
    /// out flipped.
    fn cw_click_from_11(harness: &mut Harness) {
        harness.flap(S01, S11, 3, 5); // A edge with bounce
        harness.hold(S01, 200); // mid-travel plateau; direction resolves here
        harness.flap(S00, S11, 2, 6); // snap: ambiguous doubles
        harness.hold(S00, 200); // settled at the next rest
    }

    fn cw_click_from_00(harness: &mut Harness) {
        harness.flap(S10, S00, 3, 5);
        harness.hold(S10, 200);
        harness.flap(S11, S00, 2, 6);
        harness.hold(S11, 200);
    }

    /// Measured CCW anatomy (B-leading direction): B toggles just before
    /// A's edge with its bounce tail crossing it, then A edges. This is the
    /// case that made the previous revision alternate up/down.
    fn ccw_click_from_11(harness: &mut Harness) {
        harness.hold(S10, 3); // B real toggle, still bouncing...
        harness.hold(S11, 2);
        harness.hold(S10, 4);
        harness.hold(S00, 200); // ...A edges while B's tail settles
    }

    fn ccw_click_from_00(harness: &mut Harness) {
        harness.hold(S01, 3);
        harness.hold(S00, 2);
        harness.hold(S01, 4);
        harness.hold(S11, 200);
    }

    /// B-leading click where the user crawls: B settles well before A edges.
    fn slow_ccw_click_from_00(harness: &mut Harness) {
        harness.hold(S01, STABLE + 5); // B settles high at departure
        harness.hold(S00, 3); // one late B bounce, too short to matter
        harness.hold(S01, 200);
        harness.flap(S11, S01, 3, 5); // A edge with bounce
        harness.hold(S11, 200);
    }

    #[test]
    fn cw_clicks_emit_exactly_once_per_click() {
        let mut harness = Harness::new(true, true);
        cw_click_from_11(&mut harness);
        harness.assert_events(1, 0);
        cw_click_from_00(&mut harness);
        harness.assert_events(1, 0);
    }

    #[test]
    fn ccw_clicks_emit_exactly_once_per_click_despite_b_leading_tightly() {
        let mut harness = Harness::new(true, true);
        ccw_click_from_11(&mut harness);
        harness.assert_events(0, 1);
        ccw_click_from_00(&mut harness);
        harness.assert_events(0, 1);
    }

    #[test]
    fn slow_b_leading_clicks_are_also_correct() {
        let mut harness = Harness::new(false, false);
        slow_ccw_click_from_00(&mut harness);
        harness.assert_events(0, 1);
    }

    #[test]
    fn first_completely_ambiguous_double_has_no_invented_direction() {
        // A two-bit transition has no direction information. With no prior
        // direction to use as a fallback, the decoder must not invent one.
        let mut harness = Harness::new(true, true);
        harness.hold(S00, STABLE + 200);
        harness.assert_events(0, 0);
    }

    #[test]
    fn ambiguous_reversal_waits_for_new_direction_instead_of_reusing_old_one() {
        let mut harness = Harness::new(true, true);

        cw_click_from_11(&mut harness);
        harness.assert_events(1, 0);

        // The first reverse click is sampled as a directionless two-bit
        // transition. It must not repeat the preceding CW direction.
        harness.hold(S11, STABLE + 20);
        harness.assert_events(0, 0);

        // The next valid CCW click resolves both the held click and itself.
        ccw_click_from_11(&mut harness);
        harness.assert_events(0, 2);
    }

    #[test]
    fn settled_trailing_transition_resolves_a_pending_click() {
        let mut harness = Harness::new(true, true);

        harness.hold(S00, STABLE + 20);
        harness.assert_events(0, 0);
        harness.hold(S01, STABLE + 20);
        harness.assert_events(0, 1);
    }

    #[test]
    fn consecutive_ambiguous_clicks_are_all_retained_until_direction_resolves() {
        let mut harness = Harness::new(true, true);

        harness.hold(S00, STABLE + 20);
        harness.hold(S11, STABLE + 20);
        harness.assert_events(0, 0);

        ccw_click_from_11(&mut harness);
        harness.assert_events(0, 3);
    }

    #[test]
    fn eighteen_clicks_per_revolution_in_each_direction() {
        let mut harness = Harness::new(true, true);
        for _ in 0..9 {
            cw_click_from_11(&mut harness);
            cw_click_from_00(&mut harness);
        }
        harness.assert_events(18, 0);
        for _ in 0..9 {
            ccw_click_from_11(&mut harness);
            ccw_click_from_00(&mut harness);
        }
        harness.assert_events(0, 18);
    }

    #[test]
    fn direction_reversal_is_immediate_and_correct() {
        let mut harness = Harness::new(true, true);
        cw_click_from_11(&mut harness);
        harness.assert_events(1, 0);
        ccw_click_from_00(&mut harness);
        harness.assert_events(0, 1);
        cw_click_from_11(&mut harness);
        harness.assert_events(1, 0);
    }

    #[test]
    fn reversal_discards_settled_residue_from_the_previous_click() {
        let mut harness = Harness::new(true, true);

        cw_click_from_11(&mut harness);
        harness.assert_events(1, 0);
        // Reproduce the device symptom: uncancelled trailing B movement has
        // the old CW sign. A quiet detent must expire it before reversal.
        harness.decoder.interval_movement = 2;
        harness.hold(S00, STABLE + 1);
        ccw_click_from_00(&mut harness);
        harness.assert_events(0, 1);

        harness.decoder.interval_movement = -2;
        harness.hold(S11, STABLE + 1);
        cw_click_from_11(&mut harness);
        harness.assert_events(1, 0);
    }

    #[test]
    fn immediate_reversal_uses_current_a_edge_over_old_direction_residue() {
        let mut harness = Harness::new(true, true);

        cw_click_from_11(&mut harness);
        harness.assert_events(1, 0);
        // No quiet period: model B chatter continually refreshing stale CW
        // evidence right up to an immediate reversal. Whole-interval
        // summation would report CW; the current A edge says CCW.
        harness.decoder.interval_movement = 3;
        ccw_click_from_00(&mut harness);
        harness.assert_events(0, 1);

        harness.decoder.interval_movement = -3;
        cw_click_from_11(&mut harness);
        harness.assert_events(1, 0);
    }

    #[test]
    fn rest_chatter_on_b_never_emits() {
        // B sits on its own switching threshold at every detent and may
        // chatter there indefinitely — fast or slow.
        let mut harness = Harness::new(false, false);
        harness.flap(S01, S00, 2, 100); // fast chatter
        for _ in 0..10 {
            harness.hold(S01, STABLE + 10); // slow chatter: each level settles
            harness.hold(S00, STABLE + 10);
        }
        harness.assert_events(0, 0);
    }

    #[test]
    fn short_glitches_on_a_are_ignored() {
        let mut harness = Harness::new(true, true);
        harness.flap(S01, S11, 4, 20);
        harness.hold(S11, 200);
        harness.assert_events(0, 0);
    }

    #[test]
    fn snap_doubles_alone_never_emit() {
        // Both phases jittering together (the arrival snap) with A settling
        // back where it was: no click may be reported.
        let mut harness = Harness::new(false, true);
        harness.flap(S00, S11, 2, 8);
        harness.hold(S00, 200);
        harness.assert_events(0, 0);
    }

    #[test]
    fn fast_rotation_still_counts_every_click() {
        // ~5 ms per quadrature state = ~50 clicks/s, faster than a hand
        // flick. Each direction decision must land inside its own plateau.
        let mut harness = Harness::new(true, true);
        for _ in 0..9 {
            harness.hold(S01, 80);
            harness.hold(S00, 80);
            harness.hold(S10, 80);
            harness.hold(S11, 80);
        }
        harness.assert_events(18, 0);
        for _ in 0..9 {
            harness.hold(S10, 80);
            harness.hold(S00, 80);
            harness.hold(S01, 80);
            harness.hold(S11, 80);
        }
        harness.assert_events(0, 18);
    }
}
