//! Observe RMK's existing smart bursts without extra SPI transactions or writes.
use embedded_hal_async::spi::{ErrorType, SpiBus};

use crate::trackball_diagnostic_data::decode_burst;

pub struct SensorTrace<S> {
    spi: S,
    burst_pending: bool,
    sequence: u32,
}

impl<S> SensorTrace<S> {
    pub fn new(spi: S) -> Self {
        Self {
            spi,
            burst_pending: false,
            sequence: 0,
        }
    }
}

impl<S: ErrorType> ErrorType for SensorTrace<S> {
    type Error = S::Error;
}

impl<S: SpiBus> SpiBus for SensorTrace<S> {
    async fn read(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        let burst = core::mem::take(&mut self.burst_pending) && words.len() == 7;
        if burst {
            self.sequence = self.sequence.wrapping_add(1);
        }
        self.spi.read(words).await?;
        if burst {
            if let Some(sample) = decode_burst(words) {
                if rmk::ble::diagnostics::mode() == 1 {
                    let packet = crate::trackball_diagnostic_data::motion_packet(
                        &sample,
                        self.sequence,
                        embassy_time::Instant::now().as_micros() as u32,
                        crate::trackball_camera::active_cpi(),
                    );
                    let _ = rmk::ble::diagnostics::PACKETS.try_send(packet);
                }
                // Failed reads and transport overflow leave sequence gaps; never wait for the host.
                log::info!(
                    "RTB1,{},{},{},{},{},{},{}",
                    self.sequence,
                    embassy_time::Instant::now().as_micros(),
                    sample.dx,
                    sample.dy,
                    sample.squal,
                    sample.shutter,
                    sample.motion,
                );
            }
        }
        Ok(())
    }

    async fn write(&mut self, words: &[u8]) -> Result<(), Self::Error> {
        self.burst_pending = false;
        self.spi.write(words).await?;
        self.burst_pending = words == [0x12];
        Ok(())
    }

    async fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        self.burst_pending = false;
        self.spi.transfer(read, write).await
    }

    async fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        self.burst_pending = false;
        self.spi.transfer_in_place(words).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.spi.flush().await
    }
}
