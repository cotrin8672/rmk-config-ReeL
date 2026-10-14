//! The pointing task owns capture and reset; BLE never touches the SPI bus.
use crate::trackball_diagnostic_data::frame_packet;
use core::sync::atomic::{AtomicU16, Ordering};
use embassy_time::{Duration, with_timeout};
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal_async::{digital::Wait, spi::SpiBus};
use rmk::ble::diagnostics;
use rmk::input_device::pmw3610::Pmw3610;
use rmk::input_device::pointing::{MotionData, PointingDriver, PointingDriverError};

static CPI: AtomicU16 = AtomicU16::new(1600);
pub fn active_cpi() -> u16 {
    CPI.load(Ordering::Relaxed)
}

pub struct CameraSensor<S: SpiBus, C: OutputPin, M: InputPin + Wait> {
    sensor: Pmw3610<S, C, M>,
    frame_id: u16,
    needs_init: bool,
}
impl<S: SpiBus, C: OutputPin, M: InputPin + Wait> CameraSensor<S, C, M> {
    pub fn new(sensor: Pmw3610<S, C, M>) -> Self {
        Self {
            sensor,
            frame_id: 0,
            needs_init: false,
        }
    }
}

async fn send(packet: [u8; 20]) -> bool {
    diagnostics::mode() != 0
        && with_timeout(
            Duration::from_millis(250),
            diagnostics::PACKETS.send(packet),
        )
        .await
        .is_ok()
}
fn status(id: u16, phase: u8) -> [u8; 20] {
    let mut packet = [0u8; 20];
    packet[..2].copy_from_slice(b"S\x01");
    packet[2..4].copy_from_slice(&id.to_le_bytes());
    packet[4] = phase;
    packet
}

impl<S: SpiBus, C: OutputPin, M: InputPin + Wait> PointingDriver for CameraSensor<S, C, M> {
    type MOTION = M;
    async fn init(&mut self) -> Result<(), PointingDriverError> {
        self.sensor.init().await
    }
    async fn read_motion(&mut self) -> Result<MotionData, PointingDriverError> {
        if self.needs_init {
            if let Err(error) = self.sensor.init().await {
                embassy_time::Timer::after_millis(100).await;
                return Err(error);
            }
            self.needs_init = false;
        }
        if diagnostics::mode() == 2 {
            self.frame_id = self.frame_id.wrapping_add(1);
            let id = self.frame_id;
            let mut pixels = [0u8; 484];
            let _ = send(status(id, 1)).await;
            // capture_frame always resets the sensor, including after SPI errors/disconnection.
            let result = self.sensor.capture_frame(&mut pixels).await;
            if result.is_ok() {
                for offset in (0..484).step_by(14) {
                    if !send(frame_packet(id, offset, &pixels).unwrap()).await {
                        break;
                    }
                }
                let _ = send(status(id, 2)).await;
            } else {
                self.needs_init = true;
                let _ = send(status(id, 3)).await;
            }
            diagnostics::finish_capture();
            return result.map(|_| MotionData::default());
        }
        self.sensor.read_motion().await
    }
    fn motion_pending(&mut self) -> bool {
        self.sensor.motion_pending()
    }
    fn motion_gpio(&mut self) -> Option<&mut M> {
        self.sensor.motion_gpio()
    }
    async fn set_resolution(&mut self, cpi: u16) -> Result<(), PointingDriverError> {
        self.sensor.set_resolution(cpi).await?;
        CPI.store(cpi, Ordering::Relaxed);
        Ok(())
    }
}
