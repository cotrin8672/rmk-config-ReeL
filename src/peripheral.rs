#![no_main]
#![no_std]

#[macro_use]
mod macros;
mod lcd_dirty_lines;
mod rotary_decoder;
mod sharp_lcd;
mod xiao_battery;

use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::mode::Async;
use embassy_nrf::peripherals::{RNG, SPI3, USBD};
use embassy_nrf::saadc::Input as _;
use embassy_nrf::{bind_interrupts, rng, saadc, spim, usb};
use embassy_sync::channel::Channel;
use embassy_time::Timer;
use nrf_mpsl::Flash;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use nrf_sdc::{self as sdc, mpsl};
use panic_probe as _;
use rmk::ble::build_ble_stack;
use rmk::config::StorageConfig;
use rmk::core_traits::Runnable;
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::event::{KeyboardEvent, publish_event_async};
use rmk::futures::future::join;
use rmk::input_device::battery::BatteryProcessor;
use rmk::input_device::rotary_encoder::Direction;
use rmk::matrix::Matrix;
use rmk::run_all;
use rmk::split::peripheral::run_rmk_split_peripheral;
use rmk::storage::new_storage_for_split_peripheral;
use rmk::watchdog::Nrf52Watchdog;
use rmk::{HostResources, RawMutex};
use static_cell::StaticCell;

use rotary_decoder::{ClockedDetentDecoder, Detent};
use sharp_lcd::new_status_lcd;
use xiao_battery::{
    DIVIDER_MEASURED, DIVIDER_TOTAL, PeripheralBatterySnapshot, XiaoBatteryMonitor,
    XiaoChargingStateReader,
};

bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<USBD>;
    RNG => rng::InterruptHandler<RNG>;
    EGU0_SWI0 => nrf_sdc::mpsl::LowPrioInterruptHandler;
    CLOCK_POWER => nrf_sdc::mpsl::ClockInterruptHandler, usb::vbus_detect::InterruptHandler;
    RADIO => nrf_sdc::mpsl::HighPrioInterruptHandler;
    TIMER0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
    RTC0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
    SPIM3 => spim::InterruptHandler<SPI3>;
    SAADC => saadc::InterruptHandler;
});

#[embassy_executor::task]
async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
    mpsl.run().await
}

const L2CAP_TXQ: u8 = 3;
const L2CAP_RXQ: u8 = 3;
const L2CAP_MTU: usize = 251;

fn build_sdc<'d, const N: usize>(
    peripherals: nrf_sdc::Peripherals<'d>,
    rng: &'d mut rng::Rng<Async>,
    mpsl: &'d MultiprotocolServiceLayer,
    memory: &'d mut sdc::Mem<N>,
) -> Result<nrf_sdc::SoftdeviceController<'d>, nrf_sdc::Error> {
    sdc::Builder::new()?
        .support_adv()
        .support_peripheral()
        .support_dle_peripheral()
        .support_phy_update_peripheral()
        .support_le_2m_phy()
        .peripheral_count(1)?
        .buffer_cfg(L2CAP_MTU as u16, L2CAP_MTU as u16, L2CAP_TXQ, L2CAP_RXQ)?
        .build(peripherals, rng, mpsl, memory)
}

fn ble_addr() -> [u8; 6] {
    let ficr = embassy_nrf::pac::FICR;
    let high = u64::from(ficr.deviceid(1).read());
    let addr = high << 32 | u64::from(ficr.deviceid(0).read());
    let addr = addr | 0x0000_c000_0000_0000;
    unwrap!(addr.to_le_bytes()[..6].try_into())
}

/// Rotary encoder reader for the left half.
///
/// Phase A is debounced as the one-edge-per-detent click clock. Direction
/// comes from signed raw Gray-code movement captured around that A edge, so
/// phase B is never sampled at a fragile fixed time relative to A.
struct LeftRotaryEncoder {
    pin_a: Input<'static>,
    pin_b: Input<'static>,
    decoder: ClockedDetentDecoder,
    last_a: bool,
    last_b: bool,
}

impl LeftRotaryEncoder {
    fn new(pin_a: Input<'static>, pin_b: Input<'static>) -> Self {
        let last_a = pin_a.is_high();
        let last_b = pin_b.is_high();
        let decoder = ClockedDetentDecoder::new(last_a, last_b);
        Self {
            pin_a,
            pin_b,
            decoder,
            last_a,
            last_b,
        }
    }

    async fn feed_sample(&mut self, a_high: bool, b_high: bool) {
        self.last_a = a_high;
        self.last_b = b_high;
        if let Some(detent) = self.decoder.update(a_high, b_high) {
            let direction = match detent {
                Detent::Clockwise => Direction::Clockwise,
                Detent::CounterClockwise => Direction::CounterClockwise,
            };
            ENCODER_DIRECTION_CHANNEL.send(direction).await;
        }
    }
}

const ENCODER_EVENT_QUEUE_SIZE: usize = 64;
static ENCODER_DIRECTION_CHANNEL: Channel<RawMutex, Direction, ENCODER_EVENT_QUEUE_SIZE> =
    Channel::new();

#[embassy_executor::task]
async fn encoder_event_task() -> ! {
    loop {
        let direction = ENCODER_DIRECTION_CHANNEL.receive().await;
        publish_event_async(KeyboardEvent::rotary_encoder(0, direction, true)).await;
        // Keep the tap visible to the split transport while edge capture keeps
        // running independently in LeftRotaryEncoder::run.
        Timer::after_millis(5).await;
        publish_event_async(KeyboardEvent::rotary_encoder(0, direction, false)).await;
    }
}

impl Runnable for LeftRotaryEncoder {
    async fn run(&mut self) -> ! {
        use rmk::embassy_futures::select::{Either, select};

        // Two 32.768 kHz timer ticks (~61 us).
        const SAMPLE_PERIOD_TICKS: u64 = 2;

        loop {
            let edge = select(
                self.pin_a.wait_for_any_edge(),
                self.pin_b.wait_for_any_edge(),
            )
            .await;

            // Preserve the phase that woke the task. If both contacts have
            // changed by the time the GPIO levels are read, feeding this
            // intermediate one-bit state keeps their order visible to the
            // existing Gray-code decoder instead of collapsing it into an
            // ambiguous two-bit transition.
            let levels = (self.pin_a.is_high(), self.pin_b.is_high());
            let ordered_edge = match edge {
                Either::First(()) => (levels.0, self.last_b),
                Either::Second(()) => (self.last_a, levels.1),
            };
            self.feed_sample(ordered_edge.0, ordered_edge.1).await;
            if ordered_edge != levels {
                self.feed_sample(levels.0, levels.1).await;
            }

            while !self.decoder.is_idle() {
                Timer::after_ticks(SAMPLE_PERIOD_TICKS).await;
                let levels = (self.pin_a.is_high(), self.pin_b.is_high());
                self.feed_sample(levels.0, levels.1).await;
            }
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Starting ReeL left (RMK peripheral)");

    let mut nrf_config = embassy_nrf::config::Config::default();
    nrf_config.lfclk_source = embassy_nrf::config::LfclkSource::ExternalXtal;
    nrf_config.dcdc.reg0_voltage = Some(embassy_nrf::config::Reg0Voltage::_3V3);
    nrf_config.dcdc.reg0 = true;
    nrf_config.dcdc.reg1 = true;
    let p = embassy_nrf::init(nrf_config);

    let mut lcd_spi_config = spim::Config::default();
    lcd_spi_config.frequency = spim::Frequency::M1;
    lcd_spi_config.mode = spim::MODE_0;
    lcd_spi_config.bit_order = spim::BitOrder::LsbFirst;
    let lcd_spi = spim::Spim::new_txonly(p.SPI3, Irqs, p.P1_00, p.P0_16, lcd_spi_config);
    let lcd_cs = Output::new(p.P1_10, Level::Low, OutputDrive::Standard);
    let (mut lcd, mut lcd_vcom) = new_status_lcd(lcd_spi, lcd_cs, false);

    let mut battery_monitor =
        XiaoBatteryMonitor::new(p.P0_31.degrade_saadc(), p.SAADC, p.P0_14).await;
    let mut battery_processor = BatteryProcessor::new(DIVIDER_MEASURED, DIVIDER_TOTAL);
    let mut charging_state_reader = XiaoChargingStateReader::new(Input::new(p.P0_17, Pull::Up));
    let mut battery_snapshot = PeripheralBatterySnapshot::new();

    let mpsl_peripherals =
        mpsl::Peripherals::new(p.RTC0, p.TIMER0, p.TEMP, p.PPI_CH19, p.PPI_CH30, p.PPI_CH31);
    // XIAO nRF52840 has a 32.768 kHz crystal. Using it avoids LFRC
    // calibration work and its associated high-frequency-clock wakeups.
    // The RC-only fields must be zero when the XTAL source is selected.
    let lfclk_config = mpsl::raw::mpsl_clock_lfclk_cfg_t {
        source: mpsl::raw::MPSL_CLOCK_LF_SRC_XTAL as u8,
        rc_ctiv: 0,
        rc_temp_ctiv: 0,
        accuracy_ppm: mpsl::raw::MPSL_DEFAULT_CLOCK_ACCURACY_PPM as u16,
        skip_wait_lfclk_started: mpsl::raw::MPSL_DEFAULT_SKIP_WAIT_LFCLK_STARTED != 0,
    };
    static MPSL: StaticCell<MultiprotocolServiceLayer> = StaticCell::new();
    static SESSION_MEM: StaticCell<mpsl::SessionMem<1>> = StaticCell::new();
    let mpsl = MPSL.init(unwrap!(mpsl::MultiprotocolServiceLayer::with_timeslots(
        mpsl_peripherals,
        Irqs,
        lfclk_config,
        SESSION_MEM.init(mpsl::SessionMem::new()),
    )));
    spawner.spawn(mpsl_task(&*mpsl).unwrap());

    let sdc_peripherals = sdc::Peripherals::new(
        p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23, p.PPI_CH24,
        p.PPI_CH25, p.PPI_CH26, p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
    );
    let mut rng = rng::Rng::new(p.RNG, Irqs);
    let mut sdc_memory = sdc::Mem::<4696>::new();
    let sdc = unwrap!(build_sdc(sdc_peripherals, &mut rng, mpsl, &mut sdc_memory));
    let mut host_resources = HostResources::new();
    let stack = build_ble_stack(sdc, ble_addr(), &mut host_resources).await;

    let (row_pins, col_pins) = config_matrix_pins_nrf!(
        peripherals: p,
        input: [P0_02, P0_03, P0_28, P0_29],
        output: [P0_04, P0_05, P1_11, P1_12, P1_13, P0_09]
    );

    let storage_config = StorageConfig {
        start_addr: 0xA0000,
        num_sectors: 6,
        ..Default::default()
    };
    let flash = Flash::take(mpsl, p.NVMC);
    let mut storage = new_storage_for_split_peripheral(flash, storage_config).await;

    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, 4, 6, true>::new(row_pins, col_pins, debouncer);
    let encoder_a = Input::new(p.P1_14, Pull::Up);
    let encoder_b = Input::new(p.P1_15, Pull::Up);
    let mut encoder = LeftRotaryEncoder::new(encoder_a, encoder_b);
    spawner.spawn(encoder_event_task().unwrap());
    let mut watchdog = Nrf52Watchdog::default_runner(p.WDT);

    join(
        run_all!(
            matrix,
            encoder,
            storage,
            watchdog,
            battery_monitor,
            battery_processor,
            charging_state_reader,
            battery_snapshot,
            lcd,
            lcd_vcom
        ),
        run_rmk_split_peripheral(0, &stack),
    )
    .await;
}
