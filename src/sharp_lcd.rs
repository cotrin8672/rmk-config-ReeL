use core::convert::Infallible;
use defmt::unwrap;
use embassy_nrf::gpio::Output;
use embassy_nrf::spim::Spim;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};
use embedded_graphics::mono_font::{MonoTextStyle, ascii::FONT_6X10};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Baseline, Text};
use rmk::core_traits::Runnable;
use rmk::display::{DisplayDriver, DisplayProcessor, DisplayRenderer, RenderContext};
use rmk::types::battery::{BatteryStatus, ChargeState};
use static_cell::StaticCell;

use crate::lcd_dirty_lines::should_write_line;

const WIDTH: usize = 160;
const WIDTH_BYTES: usize = WIDTH / 8;
const HEIGHT: usize = 68;
const FRAMEBUFFER_SIZE: usize = WIDTH_BYTES * HEIGHT;
const LOGICAL_WIDTH: usize = 68;
const LOGICAL_HEIGHT: usize = 160;
const BASE_ROW_BYTES: usize = LOGICAL_WIDTH.div_ceil(8);
const BASE_UI: &[u8; BASE_ROW_BYTES * LOGICAL_HEIGHT] =
    include_bytes!("lcd_status_base_68x160.raw");
const WRITE_COMMAND: u8 = 0x01;
const VCOM_BIT: u8 = 0x02;
const CLEAR_COMMAND: u8 = 0x04;
const SCS_SETUP: Duration = Duration::from_micros(6);
const SCS_HOLD: Duration = Duration::from_micros(2);
const SCS_LOW: Duration = Duration::from_micros(6);
const VCOM_INTERVAL: Duration = Duration::from_millis(33);

static LCD_BUS: StaticCell<Mutex<ThreadModeRawMutex, SharpBus>> = StaticCell::new();

struct SharpBus {
    spi: Spim<'static>,
    cs: Output<'static>,
    vcom_high: bool,
}

impl SharpBus {
    fn new(spi: Spim<'static>, cs: Output<'static>) -> Self {
        Self {
            spi,
            cs,
            vcom_high: false,
        }
    }

    fn toggle_vcom(&mut self) -> u8 {
        self.vcom_high = !self.vcom_high;
        if self.vcom_high { VCOM_BIT } else { 0 }
    }

    async fn begin_transaction(&mut self) {
        Timer::after(SCS_LOW).await;
        self.cs.set_high();
        Timer::after(SCS_SETUP).await;
    }

    async fn end_transaction(&mut self) {
        Timer::after(SCS_HOLD).await;
        self.cs.set_low();
    }

    async fn clear(&mut self) {
        self.begin_transaction().await;
        let command = [CLEAR_COMMAND, 0];
        unwrap!(self.spi.write_from_ram(&command).await);
        self.end_transaction().await;
    }

    async fn write_frame(
        &mut self,
        framebuffer: &[u8; FRAMEBUFFER_SIZE],
        previous: Option<&[u8; FRAMEBUFFER_SIZE]>,
    ) {
        self.begin_transaction().await;

        let command = [WRITE_COMMAND | self.toggle_vcom()];
        unwrap!(self.spi.write_from_ram(&command).await);

        let mut line = [0_u8; WIDTH_BYTES + 2];
        let previous = previous.map(|framebuffer| framebuffer.as_slice());
        for y in 0..HEIGHT {
            if !should_write_line(framebuffer, previous, WIDTH_BYTES, y) {
                continue;
            }

            line[0] = (y + 1) as u8;
            line[1..=WIDTH_BYTES]
                .copy_from_slice(&framebuffer[y * WIDTH_BYTES..(y + 1) * WIDTH_BYTES]);
            unwrap!(self.spi.write_from_ram(&line).await);
        }

        let trailer = [0_u8];
        unwrap!(self.spi.write_from_ram(&trailer).await);
        self.end_transaction().await;
    }

    async fn invert_vcom(&mut self) {
        let command = [self.toggle_vcom(), 0];
        self.begin_transaction().await;
        unwrap!(self.spi.write_from_ram(&command).await);
        self.end_transaction().await;
    }
}

pub struct SharpDisplay {
    bus: &'static Mutex<ThreadModeRawMutex, SharpBus>,
    framebuffer: [u8; FRAMEBUFFER_SIZE],
    last_framebuffer: [u8; FRAMEBUFFER_SIZE],
    flushed_once: bool,
}

impl SharpDisplay {
    fn new(bus: &'static Mutex<ThreadModeRawMutex, SharpBus>) -> Self {
        Self {
            bus,
            // Sharp Memory LCD data is active-low: 1 is white and 0 is black.
            framebuffer: [0xff; FRAMEBUFFER_SIZE],
            last_framebuffer: [0xff; FRAMEBUFFER_SIZE],
            flushed_once: false,
        }
    }
}

impl OriginDimensions for SharpDisplay {
    fn size(&self) -> Size {
        Size::new(LOGICAL_WIDTH as u32, LOGICAL_HEIGHT as u32)
    }
}

impl DrawTarget for SharpDisplay {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x < 0
                || point.y < 0
                || point.x >= LOGICAL_WIDTH as i32
                || point.y >= LOGICAL_HEIGHT as i32
            {
                continue;
            }

            let physical_x = LOGICAL_HEIGHT - 1 - point.y as usize;
            let physical_y = point.x as usize;
            let index = physical_y * WIDTH_BYTES + physical_x / 8;
            let mask = 1 << (physical_x % 8);
            match color {
                BinaryColor::On => self.framebuffer[index] &= !mask,
                BinaryColor::Off => self.framebuffer[index] |= mask,
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        let byte = match color {
            BinaryColor::On => 0x00,
            BinaryColor::Off => 0xff,
        };
        self.framebuffer.fill(byte);
        Ok(())
    }
}

impl DisplayDriver for SharpDisplay {
    async fn init(&mut self) {
        self.bus.lock().await.clear().await;
    }

    async fn flush(&mut self) {
        if self.flushed_once && self.framebuffer == self.last_framebuffer {
            return;
        }

        let previous = self.flushed_once.then_some(&self.last_framebuffer);
        self.bus
            .lock()
            .await
            .write_frame(&self.framebuffer, previous)
            .await;
        self.last_framebuffer.copy_from_slice(&self.framebuffer);
        self.flushed_once = true;
    }
}

pub struct SharpVcomRunner {
    bus: &'static Mutex<ThreadModeRawMutex, SharpBus>,
}

impl Runnable for SharpVcomRunner {
    async fn run(&mut self) -> ! {
        loop {
            Timer::after(VCOM_INTERVAL).await;
            self.bus.lock().await.invert_vcom().await;
        }
    }
}

pub struct ReelStatusRenderer {
    central: bool,
    last_snapshot: Option<StatusSnapshot>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct StatusSnapshot {
    battery_level: Option<u8>,
    battery_charging: bool,
    split_connected: bool,
    layer: u8,
    profile: u8,
}

impl StatusSnapshot {
    fn from_context(ctx: &RenderContext, central: bool) -> Self {
        Self {
            battery_level: battery_level(ctx.battery.0),
            battery_charging: battery_is_charging(ctx.battery.0),
            split_connected: if central {
                ctx.peripherals_connected.first().copied().unwrap_or(false)
            } else {
                ctx.central_connected
            },
            layer: ctx.layer.min(3),
            profile: ctx.ble_status.profile.min(4),
        }
    }
}

impl ReelStatusRenderer {
    fn new(central: bool) -> Self {
        Self {
            central,
            last_snapshot: None,
        }
    }
}

fn draw_pixel<D>(display: &mut D, x: i32, y: i32)
where
    D: DrawTarget<Color = BinaryColor>,
{
    draw_pixel_color(display, x, y, BinaryColor::On);
}

fn draw_pixel_color<D>(display: &mut D, x: i32, y: i32, color: BinaryColor)
where
    D: DrawTarget<Color = BinaryColor>,
{
    let _ = display.draw_iter(core::iter::once(Pixel(Point::new(x, y), color)));
}

fn draw_rectangle<D>(display: &mut D, x0: i32, y0: i32, x1: i32, y1: i32, filled: bool)
where
    D: DrawTarget<Color = BinaryColor>,
{
    for y in y0..=y1 {
        for x in x0..=x1 {
            if filled || x == x0 || x == x1 || y == y0 || y == y1 {
                draw_pixel(display, x, y);
            }
        }
    }
}

fn draw_base<D>(display: &mut D)
where
    D: DrawTarget<Color = BinaryColor>,
{
    for y in 0..LOGICAL_HEIGHT {
        for x in 0..LOGICAL_WIDTH {
            if BASE_UI[y * BASE_ROW_BYTES + x / 8] & (1 << (x % 8)) != 0 {
                draw_pixel(display, x as i32, y as i32);
            }
        }
    }
}

fn battery_level(status: BatteryStatus) -> Option<u8> {
    match status {
        BatteryStatus::Available {
            level: Some(level), ..
        } => Some(level.min(100)),
        _ => None,
    }
}

fn battery_is_charging(status: BatteryStatus) -> bool {
    matches!(
        status,
        BatteryStatus::Available {
            charge_state: ChargeState::Charging,
            ..
        }
    )
}

fn battery_fill_width(level: Option<u8>) -> i32 {
    level
        .map(|level| i32::from((13 * u16::from(level) + 50) / 100))
        .unwrap_or(0)
}

fn draw_charging_mark<D>(display: &mut D, fill_width: i32)
where
    D: DrawTarget<Color = BinaryColor>,
{
    // Keep the approved 5x5 lightning bolt inside the existing battery body.
    const MARK: [u8; 5] = [
        0b00100, // ..#..
        0b01100, // .##..
        0b11111, // #####
        0b00110, // ..##.
        0b01100, // .##..
    ];
    for (row, bits) in MARK.iter().copied().enumerate() {
        for column in 0..5 {
            if bits & (1 << (4 - column)) != 0 {
                let x = 8 + column;
                let y = 6 + row as i32;
                let filled = fill_width > 0 && (4..=3 + fill_width).contains(&x);
                draw_pixel_color(
                    display,
                    x,
                    y,
                    if filled {
                        BinaryColor::Off
                    } else {
                        BinaryColor::On
                    },
                );
            }
        }
    }
}

fn draw_percent<D>(display: &mut D, x: i32, y: i32)
where
    D: DrawTarget<Color = BinaryColor>,
{
    const PERCENT: [u8; 8] = [0x4e, 0x2a, 0x2a, 0x1e, 0xf0, 0xa8, 0xa4, 0xe4];

    for (row, bits) in PERCENT.iter().copied().enumerate() {
        for column in 0..8 {
            if bits & (1 << column) != 0 {
                draw_pixel(display, x + column, y + row as i32);
            }
        }
    }
}

fn draw_battery<D>(display: &mut D, level: Option<u8>, charging: bool)
where
    D: DrawTarget<Color = BinaryColor>,
{
    let fill_width = battery_fill_width(level);
    if fill_width > 0 {
        draw_rectangle(display, 4, 6, 3 + fill_width, 10, true);
    }

    if charging {
        draw_charging_mark(display, fill_width);
    }

    let mut text = [0_u8; 3];
    let text_length = match level {
        Some(100) => {
            text.copy_from_slice(b"100");
            3
        }
        Some(level @ 10..=99) => {
            text[..2].copy_from_slice(&[b'0' + level / 10, b'0' + level % 10]);
            2
        }
        Some(level) => {
            text[0] = b'0' + level;
            1
        }
        None => {
            text[..2].copy_from_slice(b"--");
            2
        }
    };
    const FONT_ADVANCE: i32 = 6;
    const TEXT_X: i32 = 24;

    if let Ok(text) = core::str::from_utf8(&text[..text_length]) {
        let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
        let _ =
            Text::with_baseline(text, Point::new(TEXT_X, 4), style, Baseline::Top).draw(display);
    }
    draw_percent(display, TEXT_X + text_length as i32 * FONT_ADVANCE, 4);
}

fn draw_split_connection<D>(display: &mut D, connected: bool)
where
    D: DrawTarget<Color = BinaryColor>,
{
    for (x0, y0, x1, y1) in [(52, 9, 55, 12), (57, 6, 60, 12), (62, 3, 65, 12)] {
        draw_rectangle(display, x0, y0, x1, y1, connected);
    }
}

fn draw_active_layer<D>(display: &mut D, layer: u8)
where
    D: DrawTarget<Color = BinaryColor>,
{
    let y = 65 + i32::from(layer.min(3)) * 13;
    for (x, dy) in [(4, 0), (5, 1), (6, 1), (7, 2), (6, 3), (5, 3), (4, 4)] {
        draw_pixel(display, x, y + dy);
    }
}

fn draw_profiles<D>(display: &mut D, active_profile: u8)
where
    D: DrawTarget<Color = BinaryColor>,
{
    let active_profile = active_profile.min(4);
    for profile in 0..5 {
        let x = 23 + profile * 8;
        draw_rectangle(
            display,
            x,
            148,
            x + 4,
            152,
            profile == i32::from(active_profile),
        );
    }
}

impl DisplayRenderer<BinaryColor> for ReelStatusRenderer {
    fn render<D: DrawTarget<Color = BinaryColor>>(&mut self, ctx: &RenderContext, display: &mut D) {
        let snapshot = StatusSnapshot::from_context(ctx, self.central);
        if self.last_snapshot == Some(snapshot) {
            return;
        }

        let _ = display.clear(BinaryColor::Off);
        draw_base(display);

        draw_battery(display, snapshot.battery_level, snapshot.battery_charging);
        draw_split_connection(display, snapshot.split_connected);
        draw_active_layer(display, snapshot.layer);
        draw_profiles(display, snapshot.profile);

        self.last_snapshot = Some(snapshot);
    }
}

pub fn new_status_lcd(
    spi: Spim<'static>,
    cs: Output<'static>,
    central: bool,
) -> (
    DisplayProcessor<SharpDisplay, ReelStatusRenderer>,
    SharpVcomRunner,
) {
    let bus = LCD_BUS.init(Mutex::new(SharpBus::new(spi, cs)));
    let display = SharpDisplay::new(bus);
    let processor = DisplayProcessor::with_renderer(display, ReelStatusRenderer::new(central))
        .with_min_render_interval(VCOM_INTERVAL);
    (processor, SharpVcomRunner { bus })
}
