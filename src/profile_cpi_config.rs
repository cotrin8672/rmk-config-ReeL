use embassy_time::{Duration, Timer};
use rmk::core_traits::Runnable;
use rmk::embassy_futures::select::{Either, select};
use rmk::event::{
    ConnectionStatusChangeEvent, EventSubscriber, PointingSetCpiEvent, SleepStateEvent,
    SubscribableEvent, publish_event_async,
};

const PROFILE_COUNT: usize = 5;
const PROFILE_CPI: [u16; PROFILE_COUNT] = [1600, 800, 1600, 1600, 1600];
const INITIAL_APPLY_DELAY: Duration = Duration::from_millis(50);

pub struct ProfileCpiConfigWatcher {
    active_profile: u8,
    pointing_device_id: u8,
}

impl ProfileCpiConfigWatcher {
    pub const fn new(pointing_device_id: u8) -> Self {
        Self {
            active_profile: 0,
            pointing_device_id,
        }
    }

    async fn apply_active_profile(&self) {
        publish_event_async(PointingSetCpiEvent {
            device_id: self.pointing_device_id,
            cpi: PROFILE_CPI[usize::from(self.active_profile)],
        })
        .await;
    }

    async fn update_profile(&mut self, profile: u8) {
        let profile = profile.min((PROFILE_COUNT - 1) as u8);
        if profile != self.active_profile {
            self.active_profile = profile;
            self.apply_active_profile().await;
        }
    }

    async fn restore_current_profile(&mut self) {
        self.active_profile = rmk::state::current_connection_status()
            .ble
            .profile
            .min((PROFILE_COUNT - 1) as u8);
        self.apply_active_profile().await;
    }
}

impl Runnable for ProfileCpiConfigWatcher {
    async fn run(&mut self) -> ! {
        let mut connection_subscriber = ConnectionStatusChangeEvent::subscriber();
        let mut sleep_subscriber = SleepStateEvent::subscriber();
        Timer::after(INITIAL_APPLY_DELAY).await;
        self.restore_current_profile().await;

        loop {
            match select(
                sleep_subscriber.next_event(),
                connection_subscriber.next_event(),
            )
            .await
            {
                Either::First(sleep) if sleep.0 => {
                    while sleep_subscriber.next_event().await.0 {}
                    self.restore_current_profile().await;
                }
                Either::First(_) => {}
                Either::Second(event) => self.update_profile(event.0.ble.profile).await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bt2_uses_lower_fixed_cpi() {
        assert_eq!(PROFILE_CPI, [1600, 800, 1600, 1600, 1600]);
    }
}
