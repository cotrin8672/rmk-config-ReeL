//! PMW3610 smart burst layout, shared by the trace and its host-side tests.
#[derive(Debug, PartialEq)]
pub struct SensorSample {
    pub dx: i16,
    pub dy: i16,
    pub squal: u8,
    pub shutter: u16,
    pub motion: u8,
}

pub fn decode_burst(bytes: &[u8]) -> Option<SensorSample> {
    if bytes.len() != 7 {
        return None;
    }
    let signed = |value: u16| ((value << 4) as i16) >> 4;
    // No-motion bursts can contain stale deltas; match RMK's motion-valid gate.
    let moving = bytes[0] & 0x80 != 0;
    Some(SensorSample {
        dx: if moving {
            signed((u16::from(bytes[3] & 0xf0) << 4) | u16::from(bytes[1]))
        } else {
            0
        },
        dy: if moving {
            signed((u16::from(bytes[3] & 0x0f) << 8) | u16::from(bytes[2]))
        } else {
            0
        },
        squal: bytes[4],
        shutter: u16::from_be_bytes([bytes[5], bytes[6]]),
        motion: bytes[0],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_signed_motion_and_optical_quality_without_stale_deltas() {
        assert_eq!(
            decode_burst(&[0x80, 0xff, 0x00, 0x78, 42, 0x12, 0x34]),
            Some(SensorSample {
                dx: 2047,
                dy: -2048,
                squal: 42,
                shutter: 0x1234,
                motion: 0x80,
            })
        );
        let negative = decode_burst(&[0x80, 0xff, 0xfe, 0xff, 9, 0, 45]).unwrap();
        assert_eq!((negative.dx, negative.dy), (-1, -2));
        let idle = decode_burst(&[0, 0xff, 0xfe, 0xff, 9, 0, 45]).unwrap();
        assert_eq!((idle.dx, idle.dy, idle.squal, idle.shutter), (0, 0, 9, 45));
        assert!(decode_burst(&[0; 4]).is_none());
        assert!(decode_burst(&[0; 8]).is_none());
    }
}

/// Twenty-byte notifications fit the minimum BLE ATT MTU without fragmentation.
pub fn motion_packet(sample: &SensorSample, seq: u32, micros: u32, cpi: u16) -> [u8; 20] {
    let mut packet = [0u8; 20];
    packet[..2].copy_from_slice(b"T\x01");
    packet[2..6].copy_from_slice(&seq.to_le_bytes());
    packet[6..10].copy_from_slice(&micros.to_le_bytes());
    packet[10..12].copy_from_slice(&sample.dx.to_le_bytes());
    packet[12..14].copy_from_slice(&sample.dy.to_le_bytes());
    packet[14] = sample.squal;
    packet[15..17].copy_from_slice(&sample.shutter.to_le_bytes());
    packet[17] = sample.motion;
    packet[18..20].copy_from_slice(&cpi.to_le_bytes());
    packet
}

pub fn frame_packet(id: u16, offset: usize, pixels: &[u8; 484]) -> Option<[u8; 20]> {
    if offset >= pixels.len() || offset % 14 != 0 || pixels.iter().any(|p| *p > 127) {
        return None;
    }
    let mut packet = [0u8; 20];
    packet[..2].copy_from_slice(b"F\x01");
    packet[2..4].copy_from_slice(&id.to_le_bytes());
    packet[4..6].copy_from_slice(&(offset as u16).to_le_bytes());
    let end = (offset + 14).min(pixels.len());
    packet[6..6 + end - offset].copy_from_slice(&pixels[offset..end]);
    Some(packet)
}

#[cfg(test)]
mod packet_tests {
    use super::*;
    #[test]
    fn preserves_signed_motion_and_all_484_pixels_in_ble_packets() {
        let sample = decode_burst(&[128, 255, 0, 248, 42, 0x12, 0x34]).unwrap();
        assert_eq!(
            motion_packet(&sample, 7, 4000, 1600),
            [
                84, 1, 7, 0, 0, 0, 160, 15, 0, 0, 255, 255, 0, 248, 42, 52, 18, 128, 64, 6
            ]
        );
        let pixels = core::array::from_fn(|i| (i % 128) as u8);
        let mut output = [0u8; 484];
        for offset in (0..484).step_by(14) {
            let packet = frame_packet(23, offset, &pixels).unwrap();
            let end = (offset + 14).min(484);
            output[offset..end].copy_from_slice(&packet[6..6 + end - offset]);
        }
        assert_eq!(pixels, output);
        assert!(frame_packet(1, 484, &pixels).is_none());
        assert!(frame_packet(1, 1, &pixels).is_none());
        assert!(frame_packet(1, 0, &[128; 484]).is_none());
    }
}
