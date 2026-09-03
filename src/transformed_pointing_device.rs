use core::future::pending;

use embassy_time::{Duration, Instant, Timer};
use embedded_hal_async::digital::Wait;
use rmk::core_traits::Runnable;
use rmk::event::{
    Axis, AxisEvent, AxisValType, EventSubscriber, PointingEvent, PointingSetCpiEvent,
    SubscribableEvent, publish_event,
};
use rmk::input_device::pointing::{InitState, PointingDriver};

use crate::calibration_config::current_matrix;
use crate::motion_chunk::take_proportional_i8_chunk;
use crate::motion_gain::MotionGain;
use crate::trackball_transform::TrackballTransform;

pub struct TransformingPointingDevice<S: PointingDriver> {
    sensor: S,
    init_state: InitState,
    poll_interval: Duration,
    id: u8,
    report_interval: Duration,
    last_poll: Instant,
    last_report: Instant,
    accumulated_x: i32,
    accumulated_y: i32,
    pending_report_x: i32,
    pending_report_y: i32,
    requested_cpi: Option<u16>,
    transform: TrackballTransform,
    gain: MotionGain,
}

impl<S: PointingDriver> Runnable for TransformingPointingDevice<S> {
    async fn run(&mut self) -> ! {
        use rmk::embassy_futures::select::{Either, select};

        let mut cpi_subscriber = PointingSetCpiEvent::subscriber();

        loop {
            match select(self.read_pointing_event(), cpi_subscriber.next_event()).await {
                // Immediate publication evicts the oldest queued pointing
                // event under backpressure, keeping sensor polling responsive.
                Either::First(event) => publish_event(event),
                Either::Second(event) => self.on_pointing_set_cpi_event(event).await,
            }
        }
    }
}

impl<S: PointingDriver> TransformingPointingDevice<S> {
    const MAX_INIT_RETRIES: u8 = 3;
    const DEFAULT_POLL_INTERVAL_US: u64 = 500;

    pub fn with_report_hz(id: u8, sensor: S, report_hz: u16) -> Self {
        let report_interval = Duration::from_hz(report_hz as u64);

        Self {
            sensor,
            init_state: InitState::Pending,
            poll_interval: Duration::from_micros(Self::DEFAULT_POLL_INTERVAL_US)
                .min(report_interval),
            id,
            report_interval,
            last_poll: Instant::MIN,
            last_report: Instant::MIN,
            accumulated_x: 0,
            accumulated_y: 0,
            pending_report_x: 0,
            pending_report_y: 0,
            requested_cpi: None,
            transform: TrackballTransform::new(),
            gain: MotionGain::new(),
        }
    }

    async fn try_init(&mut self) -> bool {
        match self.init_state {
            InitState::Ready => return true,
            InitState::Failed => return false,
            InitState::Pending => {
                self.init_state = InitState::Initializing(0);
            }
            InitState::Initializing(_) => {}
        }

        if let InitState::Initializing(retry_count) = self.init_state {
            match self.sensor.init().await {
                Ok(()) => {
                    self.init_state = InitState::Ready;
                    if let Some(cpi) = self.requested_cpi {
                        let _ = self.sensor.set_resolution(cpi).await;
                    }
                    return true;
                }
                Err(_) => {
                    if retry_count + 1 >= Self::MAX_INIT_RETRIES {
                        self.init_state = InitState::Failed;
                        return false;
                    }
                    self.init_state = InitState::Initializing(retry_count + 1);
                    Timer::after(Duration::from_millis(100)).await;
                }
            }
        }

        false
    }

    async fn poll_once(&mut self) {
        if self.init_state != InitState::Ready && !self.try_init().await {
            return;
        }

        if !self.sensor.motion_pending() {
            return;
        }

        if let Ok(motion) = self.sensor.read_motion().await {
            self.accumulated_x = self.accumulated_x.saturating_add(i32::from(motion.dx));
            self.accumulated_y = self.accumulated_y.saturating_add(i32::from(motion.dy));
        }
    }

    async fn on_pointing_set_cpi_event(&mut self, event: PointingSetCpiEvent) {
        if event.device_id == self.id {
            self.requested_cpi = Some(event.cpi);
            if self.init_state == InitState::Ready {
                let _ = self.sensor.set_resolution(event.cpi).await;
            }
        }
    }

    fn merge_accumulated_motion(&mut self) {
        if self.accumulated_x == 0 && self.accumulated_y == 0 {
            return;
        }

        // Keep any raw motion beyond i16 until a later report. The transform
        // is defined for one i16 motion vector at a time, so split only at
        // this boundary instead of discarding the excess.
        let raw_x = take_i16_chunk(&mut self.accumulated_x);
        let raw_y = take_i16_chunk(&mut self.accumulated_y);

        let (x, y) = self.transform.apply(raw_x, raw_y, current_matrix());
        let (x, y) = self.gain.apply(x, y);
        self.pending_report_x = self.pending_report_x.saturating_add(i32::from(x));
        self.pending_report_y = self.pending_report_y.saturating_add(i32::from(y));
    }

    fn take_pending_report(&mut self) -> Option<PointingEvent> {
        if self.pending_report_x == 0 && self.pending_report_y == 0 {
            return None;
        }

        // RMK's MouseReport uses i8 relative axes. Split both axes across the
        // same number of reports so every chunk preserves the vector direction.
        let (x, y) =
            take_proportional_i8_chunk(&mut self.pending_report_x, &mut self.pending_report_y);

        Some(PointingEvent {
            device_id: self.id,
            axes: [
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::X,
                    value: i16::from(x),
                },
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::Y,
                    value: i16::from(y),
                },
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::Z,
                    value: 0,
                },
            ],
        })
    }

    fn has_report_data(&self) -> bool {
        self.accumulated_x != 0
            || self.accumulated_y != 0
            || self.pending_report_x != 0
            || self.pending_report_y != 0
    }

    async fn read_pointing_event(&mut self) -> PointingEvent {
        use rmk::embassy_futures::select::{Either, select};

        if self.last_poll == Instant::MIN {
            self.last_poll = Instant::now();
        }
        if self.last_report == Instant::MIN {
            self.last_report = Instant::now();
        }

        loop {
            // Check the deadline before waiting for another sensor edge. This
            // is also the tie-breaker when MOTION remains asserted and both
            // futures are ready.
            if self.has_report_data() && self.last_report.elapsed() >= self.report_interval {
                self.last_report = Instant::now();
                self.merge_accumulated_motion();
                if let Some(event) = self.take_pending_report() {
                    return event;
                }
            }

            let has_report_data = self.has_report_data();
            let report_delay = self
                .report_interval
                .checked_sub(self.last_report.elapsed())
                .unwrap_or(Duration::MIN);
            let report_wait = async move {
                if has_report_data {
                    Timer::after(report_delay).await;
                } else {
                    pending::<()>().await;
                }
            };

            let poll_wait = async {
                if let Some(gpio) = self.sensor.motion_gpio() {
                    let _ = gpio.wait_for_low().await;
                } else {
                    Timer::after(
                        self.poll_interval
                            .checked_sub(self.last_poll.elapsed())
                            .unwrap_or(Duration::MIN),
                    )
                    .await;
                }
            };

            // embassy-futures::select polls its first future first, so the
            // report deadline must be the first future here.
            match select(report_wait, poll_wait).await {
                Either::First(_) => {
                    self.last_report = Instant::now();
                    self.merge_accumulated_motion();
                    if let Some(event) = self.take_pending_report() {
                        return event;
                    }
                }
                Either::Second(_) => {
                    self.poll_once().await;
                    self.last_poll = Instant::now();
                }
            }
        }
    }
}

fn take_i16_chunk(value: &mut i32) -> i16 {
    let chunk = (*value).clamp(i32::from(i16::MIN), i32::from(i16::MAX));
    *value -= chunk;
    chunk as i16
}
