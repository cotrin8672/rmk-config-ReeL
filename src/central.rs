#![no_main]
#![no_std]

mod adaptive_heading_filter;
mod keymap;
#[macro_use]
mod macros;
mod calibration_config;
mod lcd_dirty_lines;
mod motion_chunk;
mod motion_gain;
mod mouse_layer_priority;
mod profile_cpi_config;
mod quick_mod_tap;
mod sharp_lcd;
mod smart_aml_trigger;
mod trackball_transform;
mod transformed_pointing_device;
mod vial;
mod xiao_battery;

use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_embedded_hal::flash::partition::Partition;
use embassy_executor::Spawner;
use embassy_nrf::gpio::{Flex, Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::mode::Async;
use embassy_nrf::peripherals::{RNG, SPI3, USBD};
use embassy_nrf::saadc::Input as _;
use embassy_nrf::usb::Driver;
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_nrf::{bind_interrupts, rng, saadc, spim, usb};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::mutex::Mutex;
use nrf_mpsl::Flash;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use nrf_sdc::{self as sdc, mpsl};
use panic_probe as _;
use rmk::ble::BleTransport;
use rmk::config::{
    AutoMouseLayerConfig, BehaviorConfig, BleBatteryConfig, DeviceConfig, PositionalConfig,
    RmkConfig, StorageConfig, VialConfig,
};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::host::HostService;
use rmk::input_device::battery::BatteryProcessor;
use rmk::input_device::pmw3610::{BitBangSpiBus, Pmw3610, Pmw3610Config};
use rmk::input_device::pointing::{PointingProcessor, PointingProcessorConfig};
use rmk::keyboard::Keyboard;
use rmk::matrix::Matrix;
use rmk::split::PeripheralMatrixConfig;
use rmk::types::action::Action;
use rmk::types::fork::{Fork, StateBits};
use rmk::types::keycode::{HidKeyCode, KeyCode};
use rmk::types::morse::MorseMode;
use rmk::usb::UsbTransport;
use rmk::watchdog::Nrf52Watchdog;
use rmk::{AutoMouseLayerRunner, KeymapData, initialize_keymap_and_storage, run_all};
use static_cell::StaticCell;

use mouse_layer_priority::{AUTO_MOUSE_LAYER, MouseLayerPriority};
use profile_cpi_config::ProfileCpiConfigWatcher;
use quick_mod_tap::QuickModTap;
use sharp_lcd::new_status_lcd;
use smart_aml_trigger::SmartAutoMouseTrigger;
use transformed_pointing_device::TransformingPointingDevice;
use vial::{VIAL_KEYBOARD_DEF, VIAL_KEYBOARD_ID};
use xiao_battery::{DIVIDER_MEASURED, DIVIDER_TOTAL, XiaoBatteryMonitor, XiaoChargingStateReader};

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
const BLE_DEFAULT_TX_POWER_DBM: i8 = 0;
const TRACKBALL_DEVICE_ID: u8 = 0;
const TRACKBALL_REPORT_HZ: u16 = 125;
const AML_TRIGGER_DEVICE_ID: u8 = 1;
const AML_TRIGGER_THRESHOLD: u16 = 3;
const NRF52840_FLASH_SIZE: u32 = 1024 * 1024;

static SHARED_FLASH: StaticCell<Mutex<ThreadModeRawMutex, Flash<'static>>> = StaticCell::new();

fn build_sdc<'d, const N: usize>(
    peripherals: nrf_sdc::Peripherals<'d>,
    rng: &'d mut rng::Rng<Async>,
    mpsl: &'d MultiprotocolServiceLayer,
    memory: &'d mut sdc::Mem<N>,
) -> Result<nrf_sdc::SoftdeviceController<'d>, nrf_sdc::Error> {
    sdc::Builder::new()?
        .default_tx_power(BLE_DEFAULT_TX_POWER_DBM)?
        .support_scan()
        .support_central()
        .support_adv()
        .support_peripheral()
        .support_dle_peripheral()
        .support_dle_central()
        .support_phy_update_central()
        .support_phy_update_peripheral()
        .support_le_2m_phy()
        .central_count(1)?
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

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Starting ReeL right (RMK central)");

    let mut nrf_config = embassy_nrf::config::Config::default();
    nrf_config.lfclk_source = embassy_nrf::config::LfclkSource::ExternalXtal;
    nrf_config.dcdc.reg0_voltage = Some(embassy_nrf::config::Reg0Voltage::_3V3);
    nrf_config.dcdc.reg0 = true;
    nrf_config.dcdc.reg1 = true;
    let p = embassy_nrf::init(nrf_config);
    let _charge_current = Output::new(p.P0_13, Level::Low, OutputDrive::Standard);

    let mut lcd_spi_config = spim::Config::default();
    lcd_spi_config.frequency = spim::Frequency::M1;
    lcd_spi_config.mode = spim::MODE_0;
    lcd_spi_config.bit_order = spim::BitOrder::LsbFirst;
    let lcd_spi = spim::Spim::new_txonly(p.SPI3, Irqs, p.P1_00, p.P0_16, lcd_spi_config);
    let lcd_cs = Output::new(p.P1_10, Level::Low, OutputDrive::Standard);
    let (mut lcd, mut lcd_vcom) = new_status_lcd(lcd_spi, lcd_cs, true);

    let mut battery_monitor =
        XiaoBatteryMonitor::new(p.P0_31.degrade_saadc(), p.SAADC, p.P0_14).await;
    let mut battery_processor = BatteryProcessor::new(DIVIDER_MEASURED, DIVIDER_TOTAL);
    let mut charging_state_reader = XiaoChargingStateReader::new(Input::new(p.P0_17, Pull::Up));

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
    let mut sdc_memory = sdc::Mem::<6080>::new();
    let sdc = unwrap!(build_sdc(sdc_peripherals, &mut rng, mpsl, &mut sdc_memory));

    let usb_driver = Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs));
    let shared_flash = SHARED_FLASH.init(Mutex::new(Flash::take(mpsl, p.NVMC)));
    let storage_flash = Partition::new(shared_flash, 0, NRF52840_FLASH_SIZE);

    let (row_pins, col_pins) = config_matrix_pins_nrf!(
        peripherals: p,
        input: [P0_02, P0_03, P0_28, P0_29],
        output: [P0_04, P0_05, P1_11, P0_09, P0_10]
    );

    let storage_config = StorageConfig {
        start_addr: 0xA0000,
        num_sectors: 6,
        clear_layout: false,
        ..Default::default()
    };
    let rmk_config = RmkConfig {
        device_config: DeviceConfig {
            vid: 0x4C4B,
            pid: 0x524D,
            manufacturer: "cotrin8672",
            product_name: "ReeL",
            serial_number: "vial:5265654c:000001",
        },
        vial_config: VialConfig::new(&VIAL_KEYBOARD_ID, VIAL_KEYBOARD_DEF, &[(0, 0), (0, 1)]),
        ble_battery_config: BleBatteryConfig::default(),
        storage_config,
    };

    let mut keymap_data = KeymapData::new_with_encoder(
        keymap::get_default_keymap(),
        keymap::get_default_encoder_map(),
    );
    let mut behavior_config = BehaviorConfig::default();
    behavior_config.morse.default_profile = behavior_config
        .morse
        .default_profile
        .with_mode(Some(MorseMode::HoldOnOtherPress))
        .with_hold_timeout_ms(Some(220));
    behavior_config
        .morse
        .profiles
        .push(keymap::HOLD_PREFERRED_PROFILE)
        .unwrap();
    behavior_config
        .morse
        .profiles
        .push(keymap::BALANCED_LAYER_TAP_PROFILE)
        .unwrap();
    let windows_modifier = StateBits {
        modifiers: rmk::types::modifier::ModifierCombination::LGUI,
        ..StateBits::default()
    };
    behavior_config
        .fork
        .forks
        .push(Fork::new(
            rmk::k!(MouseWheelUp),
            rmk::k!(MouseWheelUp),
            rmk::k!(KbVolumeUp),
            windows_modifier,
            StateBits::default(),
            rmk::types::modifier::ModifierCombination::default(),
            false,
        ))
        .unwrap();
    behavior_config
        .fork
        .forks
        .push(Fork::new(
            rmk::k!(MouseWheelDown),
            rmk::k!(MouseWheelDown),
            rmk::k!(KbVolumeDown),
            windows_modifier,
            StateBits::default(),
            rmk::types::modifier::ModifierCombination::default(),
            false,
        ))
        .unwrap();
    behavior_config
        .morse
        .morses
        .push(
            QuickModTap::new(
                Action::Key(KeyCode::Hid(HidKeyCode::Backspace)),
                Action::Modifier(rmk::types::modifier::ModifierCombination::RCTRL),
                Action::Key(KeyCode::Hid(HidKeyCode::Backspace)),
            )
            .into_morse(),
        )
        .unwrap();
    behavior_config
        .auto_mouse_layer
        .push(
            AutoMouseLayerConfig::new(
                Some(AML_TRIGGER_DEVICE_ID),
                AUTO_MOUSE_LAYER,
                embassy_time::Duration::from_secs(5),
                AML_TRIGGER_THRESHOLD,
            )
            .with_deactivate_on_key(&[]),
        )
        .unwrap();
    let positional_config = PositionalConfig::default();
    let (keymap, mut storage) = initialize_keymap_and_storage(
        &mut keymap_data,
        storage_flash,
        &storage_config,
        &mut behavior_config,
        &positional_config,
    )
    .await;

    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, 4, 5, true, 0, 6>::new(row_pins, col_pins, debouncer);
    let mut keyboard = Keyboard::new(&keymap);
    let mut profile_cpi_config = ProfileCpiConfigWatcher::new(TRACKBALL_DEVICE_ID);
    let host_service = HostService::new(&keymap, &rmk_config);

    let pmw_sck = Output::new(p.P1_13, Level::High, OutputDrive::Standard);
    let pmw_sdio = Flex::new(p.P1_15);
    let pmw_cs = Output::new(p.P1_12, Level::High, OutputDrive::Standard);
    let pmw_motion = Some(Input::new(p.P1_14, Pull::Up));
    let pmw_spi = BitBangSpiBus::new(pmw_sck, pmw_sdio);
    let pmw_config = Pmw3610Config {
        res_cpi: 1600,
        smart_mode: true,
        ..Default::default()
    };
    let pmw_sensor = Pmw3610::new(TRACKBALL_DEVICE_ID, pmw_spi, pmw_cs, pmw_motion, pmw_config);
    let mut trackball = TransformingPointingDevice::with_report_hz(
        TRACKBALL_DEVICE_ID,
        pmw_sensor,
        TRACKBALL_REPORT_HZ,
    );
    let mut smart_aml_trigger = SmartAutoMouseTrigger::new(
        TRACKBALL_DEVICE_ID,
        AML_TRIGGER_DEVICE_ID,
        AML_TRIGGER_THRESHOLD as i16,
    );
    let mut mouse_layer_priority = MouseLayerPriority::new();
    let mut pointing_processor = PointingProcessor::new(
        &keymap,
        PointingProcessorConfig {
            device_id: TRACKBALL_DEVICE_ID,
            ..Default::default()
        },
    );
    let mut auto_mouse_layer = AutoMouseLayerRunner::new(&keymap);

    let mut usb_transport =
        UsbTransport::new(usb_driver, rmk_config.device_config).with_host_service(&host_service);
    let mut ble_transport = BleTransport::new(
        sdc,
        ble_addr(),
        rmk_config,
        [PeripheralMatrixConfig {
            rows: 4,
            cols: 6,
            row_offset: 0,
            col_offset: 0,
        }],
    )
    .with_host_service(&host_service);
    let mut watchdog = Nrf52Watchdog::default_runner(p.WDT);

    run_all!(
        matrix,
        trackball,
        smart_aml_trigger,
        mouse_layer_priority,
        pointing_processor,
        auto_mouse_layer,
        profile_cpi_config,
        storage,
        usb_transport,
        ble_transport,
        keyboard,
        watchdog,
        battery_monitor,
        battery_processor,
        charging_state_reader,
        lcd,
        lcd_vcom
    )
    .await;
}
