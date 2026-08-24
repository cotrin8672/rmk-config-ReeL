use embassy_time::{Duration, Instant};
use rmk::{
    core_traits::Runnable,
    event::{
        Axis, AxisEvent, AxisValType, EventSubscriber, PointingEvent, SubscribableEvent,
        publish_event,
    },
};

use crate::mouse_layer_priority::manual_layer_is_active;

// The trackball must move this far before the mouse layer is entered.  Keep
// this as the single tuning point for the user-visible entry threshold.
const MINIMUM_ENTRY_MOVEMENT: u32 = 12;
const IMMEDIATE_MOTION_THRESHOLD: i32 = MINIMUM_ENTRY_MOVEMENT as i32;
const CUMULATIVE_MOTION_THRESHOLD: u32 = MINIMUM_ENTRY_MOVEMENT;
const ACCUMULATION_WINDOW: Duration = Duration::from_millis(100);

#[derive(Default)]
struct MotionAccumulator {
    x: i32,
    y: i32,
}

impl MotionAccumulator {
    fn reset(&mut self) {
        self.x = 0;
        self.y = 0;
    }

    fn add(&mut self, x: i32, y: i32) -> bool {
        self.x = self.x.saturating_add(x);
        self.y = self.y.saturating_add(y);

        self.x.unsigned_abs().saturating_add(self.y.unsigned_abs()) >= CUMULATIVE_MOTION_THRESHOLD
    }
}

pub struct SmartAutoMouseTrigger {
    source_device_id: u8,
    trigger_device_id: u8,
    trigger_delta: i16,
    accumulated_motion: MotionAccumulator,
    last_motion_at: Option<Instant>,
}

impl SmartAutoMouseTrigger {
    pub fn new(source_device_id: u8, trigger_device_id: u8, trigger_delta: i16) -> Self {
        Self {
            source_device_id,
            trigger_device_id,
            trigger_delta,
            accumulated_motion: MotionAccumulator::default(),
            last_motion_at: None,
        }
    }

    fn handle_event(&mut self, event: PointingEvent) {
        if event.device_id != self.source_device_id {
            return;
        }

        if manual_layer_is_active() {
            self.accumulated_motion.reset();
            self.last_motion_at = None;
            return;
        }

        let (x, y) = relative_xy(event);
        if x == 0 && y == 0 {
            return;
        }

        let now = Instant::now();
        if self
            .last_motion_at
            .map(|last| now.duration_since(last) > ACCUMULATION_WINDOW)
            .unwrap_or(true)
        {
            self.accumulated_motion.reset();
        }
        self.last_motion_at = Some(now);

        let immediate_motion =
            x.abs() >= IMMEDIATE_MOTION_THRESHOLD || y.abs() >= IMMEDIATE_MOTION_THRESHOLD;
        let cumulative_motion = self.accumulated_motion.add(x, y);

        if immediate_motion || cumulative_motion {
            self.accumulated_motion.reset();
            publish_event(PointingEvent {
                device_id: self.trigger_device_id,
                axes: [
                    AxisEvent {
                        typ: AxisValType::Rel,
                        axis: Axis::X,
                        value: self.trigger_delta,
                    },
                    AxisEvent {
                        typ: AxisValType::Rel,
                        axis: Axis::Y,
                        value: 0,
                    },
                    AxisEvent {
                        typ: AxisValType::Rel,
                        axis: Axis::Z,
                        value: 0,
                    },
                ],
            });
        }
    }
}

impl Runnable for SmartAutoMouseTrigger {
    async fn run(&mut self) -> ! {
        let mut subscriber = PointingEvent::subscriber();
        loop {
            self.handle_event(subscriber.next_event().await);
        }
    }
}

fn relative_xy(event: PointingEvent) -> (i32, i32) {
    let mut x = 0_i32;
    let mut y = 0_i32;

    for axis in event.axes {
        if !matches!(axis.typ, AxisValType::Rel) {
            continue;
        }

        match axis.axis {
            Axis::X => x = x.saturating_add(i32::from(axis.value)),
            Axis::Y => y = y.saturating_add(i32::from(axis.value)),
            _ => {}
        }
    }

    (x, y)
}

#[cfg(test)]
mod tests {
    use super::MotionAccumulator;

    #[test]
    fn alternating_jitter_cancels_out() {
        let mut motion = MotionAccumulator::default();

        for _ in 0..20 {
            assert!(!motion.add(1, 0));
            assert!(!motion.add(-1, 0));
        }
    }

    #[test]
    fn slow_consistent_motion_eventually_triggers() {
        let mut motion = MotionAccumulator::default();

        for _ in 0..11 {
            assert!(!motion.add(1, 0));
        }
        assert!(motion.add(1, 0));
    }
}
