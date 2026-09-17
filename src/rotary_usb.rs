//! USB CDC transport for frozen traces; no USB writes on the acquisition path.
use crate::{
    rotary_diagnostics as diag, rotary_log,
    rotary_trace::{CaptureState, Trigger},
};
use core::fmt::{self, Write};
use embassy_nrf::usb::{Driver, vbus_detect::HardwareVbusDetect};
use embassy_time::Timer;
use embassy_usb::{
    Builder, Config,
    class::cdc_acm::{CdcAcmClass, State},
    driver::EndpointError,
};

type UsbDriver = Driver<'static, HardwareVbusDetect>;
struct Line {
    data: [u8; 512],
    len: usize,
}
impl Line {
    fn new() -> Self {
        Self {
            data: [0; 512],
            len: 0,
        }
    }
}
impl Write for Line {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.len + value.len() > self.data.len() {
            return Err(fmt::Error);
        }
        self.data[self.len..self.len + value.len()].copy_from_slice(value.as_bytes());
        self.len += value.len();
        Ok(())
    }
}
async fn send(
    class: &mut CdcAcmClass<'static, UsbDriver>,
    bytes: &[u8],
) -> Result<(), EndpointError> {
    for chunk in bytes.chunks(64) {
        class.write_packet(chunk).await?;
    }
    if bytes.len() % 64 == 0 {
        class.write_packet(&[]).await?;
    }
    Ok(())
}
async fn dump(class: &mut CdcAcmClass<'static, UsbDriver>) -> Result<(), EndpointError> {
    let (state, generation, count, dropped) = diag::status();
    if state != CaptureState::Frozen {
        return send(class, b"BUSY\n").await;
    }
    let mut line = Line::new();
    writeln!(&mut line, "BEGIN,1,{generation},{count},{dropped},32768").unwrap();
    send(class, &line.data[..line.len]).await?;
    line.len = 0;
    writeln!(&mut line, "{}", rotary_log::HEADER).unwrap();
    let mut hash = rotary_log::checksum(2166136261, &line.data[..line.len]);
    send(class, &line.data[..line.len]).await?;
    for index in 0..count {
        let sample = diag::sample(index).unwrap();
        line.len = 0;
        rotary_log::row(&mut line, sample).unwrap();
        hash = rotary_log::checksum(hash, &line.data[..line.len]);
        send(class, &line.data[..line.len]).await?;
    }
    line.len = 0;
    writeln!(&mut line, "END,{count},{hash:08x}").unwrap();
    send(class, &line.data[..line.len]).await
}
async fn session(class: &mut CdcAcmClass<'static, UsbDriver>) -> Result<(), EndpointError> {
    use rmk::embassy_futures::select::{Either, select};
    let mut packet = [0u8; 64];
    let mut command = [0u8; 32];
    let mut len = 0;
    let mut overflow = false;
    let mut pending = false;
    loop {
        if let Either::First(result) =
            select(class.read_packet(&mut packet), Timer::after_millis(50)).await
        {
            for &byte in &packet[..result?] {
                if byte == b'\r' {
                    continue;
                }
                if byte != b'\n' {
                    if len < command.len() {
                        command[len] = byte;
                        len += 1;
                    } else {
                        overflow = true;
                    }
                    continue;
                }
                if overflow {
                    send(class, b"ERR command\n").await?;
                } else {
                    match &command[..len] {
                        b"INFO" => {
                            let status = if diag::status().0 == CaptureState::Frozen {
                                "FROZEN"
                            } else {
                                "READY"
                            };
                            let mut line = Line::new();
                            writeln!(&mut line, "REEL_ROTARY_V1,{status}").unwrap();
                            send(class, &line.data[..line.len]).await?;
                        }
                        b"ARM" | b"NEXT" => {
                            let trigger = if &command[..len] == b"NEXT" {
                                Trigger::NextConfirmation
                            } else {
                                Trigger::Ambiguous
                            };
                            diag::arm(trigger);
                            send(class, b"ARMED\n").await?;
                            pending = true;
                        }
                        b"DUMP" => {
                            dump(class).await?;
                            pending = false;
                        }
                        _ => send(class, b"ERR command\n").await?,
                    }
                }
                len = 0;
                overflow = false;
            }
        }
        if pending && diag::status().0 == CaptureState::Frozen {
            dump(class).await?;
            pending = false;
        }
    }
}

#[embassy_executor::task]
pub async fn usb_task(driver: UsbDriver) {
    // Reuse this keyboard's existing identity, with a distinct diagnostic serial.
    let mut config = Config::new(0x4c4b, 0x524d);
    config.manufacturer = Some("ReeL");
    config.product = Some("ReeL Rotary Diagnostic");
    config.serial_number = Some("REEL-ROTARY-DIAG");
    static CONFIG: static_cell::StaticCell<[u8; 256]> = static_cell::StaticCell::new();
    static BOS: static_cell::StaticCell<[u8; 256]> = static_cell::StaticCell::new();
    static MSOS: static_cell::StaticCell<[u8; 64]> = static_cell::StaticCell::new();
    static CONTROL: static_cell::StaticCell<[u8; 64]> = static_cell::StaticCell::new();
    static STATE: static_cell::StaticCell<State<'static>> = static_cell::StaticCell::new();
    let mut builder = Builder::new(
        driver,
        config,
        CONFIG.init([0; 256]),
        BOS.init([0; 256]),
        MSOS.init([0; 64]),
        CONTROL.init([0; 64]),
    );
    let mut class = CdcAcmClass::new(&mut builder, STATE.init(State::new()), 64);
    let mut device = builder.build();
    rmk::futures::future::join(device.run(), async {
        loop {
            class.wait_connection().await;
            let _ = session(&mut class).await;
            Timer::after_millis(100).await;
        }
    })
    .await;
}
