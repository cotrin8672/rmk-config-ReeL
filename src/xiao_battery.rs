use embassy_nrf::Peri;
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pin};
use embassy_nrf::interrupt::{self, InterruptExt};
use embassy_nrf::peripherals::SAADC;
use embassy_nrf::saadc::{self, AnyInput, Saadc};
use embassy_time::{Duration, Timer};
use rmk::core_traits::Runnable;
use rmk::embassy_futures::select::{Either, select};
use rmk::event::{
    BatteryStatusEvent, CentralConnectedEvent, ChargingStateEvent, EventSubscriber,
    SubscribableEvent, publish_event,
};
use rmk::input_device::adc::{AnalogEventType, NrfAdc};

pub const DIVIDER_MEASURED: u32 = 510;
pub const DIVIDER_TOTAL: u32 = 1510;
const BATTERY_SAMPLE_INTERVAL_SECS: u64 = 60;
const CHARGING_INITIAL_DELAY_SECS: u64 = 2;
const CHARGING_DEBOUNCE_MS: u64 = 20;

/// Battery monitor for the XIAO nRF52840 onboard divider.
pub struct XiaoBatteryMonitor {
    adc: NrfAdc<'static, 1, 1>,
    // The XIAO schematic requires P0.14 to remain a low-side sink while reading.
    _read_enable: Output<'static>,
}

impl XiaoBatteryMonitor {
    pub async fn new(
        adc_pin: AnyInput<'static>,
        adc: Peri<'static, SAADC>,
        read_enable_pin: Peri<'static, impl Pin>,
    ) -> Self {
        let read_enable = Output::new(
            read_enable_pin,
            Level::Low,
            OutputDrive::Standard0Disconnect1,
        );

        let config = saadc::Config::default();
        let mut channel = saadc::ChannelConfig::single_ended(adc_pin);
        // The 1 MOhm / 510 kOhm divider has a source impedance near 338 kOhm.
        channel.time = saadc::Time::_40US;
        interrupt::SAADC.set_priority(interrupt::Priority::P3);
        let saadc = Saadc::new(adc, crate::Irqs, config, [channel]);
        saadc.calibrate().await;

        Self {
            adc: NrfAdc::new(
                saadc,
                [AnalogEventType::Battery],
                [0],
                // Battery status does not need sub-minute updates. Fewer
                // SAADC wakeups reduce idle power without changing BLE timing.
                Duration::from_secs(BATTERY_SAMPLE_INTERVAL_SECS),
                None,
            ),
            _read_enable: read_enable,
        }
    }
}

impl Runnable for XiaoBatteryMonitor {
    async fn run(&mut self) -> ! {
        self.adc.run().await
    }
}

/// Edge-driven reader for the XIAO nRF52840 active-low charging signal.
pub struct XiaoChargingStateReader {
    input: Input<'static>,
}

impl XiaoChargingStateReader {
    pub fn new(input: Input<'static>) -> Self {
        Self { input }
    }
}

impl Runnable for XiaoChargingStateReader {
    async fn run(&mut self) -> ! {
        Timer::after_secs(CHARGING_INITIAL_DELAY_SECS).await;
        let mut charging = self.input.is_low();
        publish_event(ChargingStateEvent { charging });

        loop {
            let _ = self.input.wait_for_any_edge().await;
            Timer::after_millis(CHARGING_DEBOUNCE_MS).await;
            let next_charging = self.input.is_low();
            if next_charging != charging {
                charging = next_charging;
                publish_event(ChargingStateEvent { charging });
            }
        }
    }
}

/// Replays the left battery state so a newly reconnected central receives a snapshot.
#[allow(dead_code)] // This shared module is also compiled into the central binary.
pub struct PeripheralBatterySnapshot {
    status: Option<BatteryStatusEvent>,
}

#[allow(dead_code)]
impl PeripheralBatterySnapshot {
    pub fn new() -> Self {
        Self { status: None }
    }
}

impl Runnable for PeripheralBatterySnapshot {
    async fn run(&mut self) -> ! {
        let mut battery_subscriber = BatteryStatusEvent::subscriber();
        let mut connection_subscriber = CentralConnectedEvent::subscriber();

        loop {
            match select(
                battery_subscriber.next_event(),
                connection_subscriber.next_event(),
            )
            .await
            {
                Either::First(status) => self.status = Some(status),
                Either::Second(connection) => {
                    if connection.connected
                        && let Some(status) = self.status
                    {
                        publish_event(status);
                    }
                }
            }
        }
    }
}
