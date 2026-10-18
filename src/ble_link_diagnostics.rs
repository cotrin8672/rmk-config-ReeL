//! USB carries logs only, so plugging in cannot hide a BLE failure with USB HID.

use core::fmt::Write;

use embassy_nrf::usb::Driver;
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_time::{Instant, Timer};
use embassy_usb_logger::{DummyHandler, LoggerState, UsbLogger};
use rmk::core_traits::Runnable;

static LOGGER: UsbLogger<8192, DummyHandler> = UsbLogger::with_custom_style(|record, writer| {
    let _ = writeln!(
        writer,
        "BLD1,{},{},{}",
        Instant::now().as_millis(),
        record.level(),
        record.args()
    );
});

pub fn take_reset_reason() -> u32 {
    // Read before HAL/watchdog initialization; RESETREAS bits are write-one-to-clear.
    let reason = embassy_nrf::pac::POWER.resetreas().read();
    embassy_nrf::pac::POWER.resetreas().write_value(reason);
    reason.0
}

pub fn init(reset_reason: u32) {
    log::set_logger(&LOGGER).expect("diagnostic logger already registered");
    log::set_max_level(log::LevelFilter::Info);
    log::info!(
        "BOOT baseline=3037316 build={} resetreas=0x{:08X}",
        option_env!("GITHUB_SHA").unwrap_or("local"),
        reset_reason
    );
}

pub struct DiagnosticUsb {
    driver: Option<Driver<'static, HardwareVbusDetect>>,
    reset_reason: u32,
}

impl DiagnosticUsb {
    pub fn new(driver: Driver<'static, HardwareVbusDetect>, reset_reason: u32) -> Self {
        Self {
            driver: Some(driver),
            reset_reason,
        }
    }
}

impl Runnable for DiagnosticUsb {
    async fn run(&mut self) -> ! {
        let driver = self.driver.take().expect("diagnostic USB started twice");
        let mut state = LoggerState::new();
        let heartbeat = async {
            loop {
                // Repeat the boot flags so a late/reopened serial reader still gets them.
                log::info!("HEARTBEAT resetreas=0x{:08X} input=BLE", self.reset_reason);
                Timer::after_secs(2).await;
            }
        };
        rmk::futures::future::join(LOGGER.run(&mut state, driver), heartbeat).await;
        unreachable!()
    }
}
